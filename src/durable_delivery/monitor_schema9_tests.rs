use super::*;

fn retained_fixture(label: &str) -> Fixture {
    let mut fixture = Fixture::new(label);
    drop(fixture.coordinator.take().unwrap());
    let connection = Connection::open(&fixture.database_path).unwrap();
    let objects = connection.prepare(
        "SELECT type,name FROM sqlite_master WHERE type IN ('trigger','table') AND name NOT LIKE 'sqlite_%' ORDER BY type DESC",
    ).unwrap().query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    for (kind, name) in objects {
        connection
            .execute_batch(&format!("DROP {kind} \"{name}\""))
            .unwrap();
    }
    connection
        .execute_batch(super::super::monitor_schema9::DDL)
        .unwrap();
    for row in compiled_policy_catalog() {
        connection
            .execute(
                "INSERT INTO delivery_policy_catalog VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    row.push_kind.as_str(),
                    row.sub_kind.as_str(),
                    row.cooldown_scope.as_str(),
                    row.base_cooldown_secs,
                    row.override_cooldown_secs,
                    row.window_mode.as_str(),
                    i64::from(row.counts_against_daily_budget),
                    row.policy_version
                ],
            )
            .unwrap();
    }
    connection.pragma_update(None, "user_version", 9).unwrap();
    drop(connection);
    fixture.coordinator = FixtureCoordinator(Some(Arc::new(
        DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).unwrap(),
    )));
    fixture
}

fn config(fixture: &Fixture) -> CoordinatorConfig {
    let code = fixture
        .database_path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    CoordinatorConfig::test(
        &fixture.database_path,
        code,
        "TEST_CODE_SCHEMA9_OWNER_0123456789abcdef",
    )
}

fn catalog(fixture: &Fixture) -> Vec<(String, String, Option<String>)> {
    Connection::open(&fixture.database_path)
        .unwrap()
        .prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn monitor_schema9_delivery_reopen_and_exact_retry_keep_catalog_and_send_once() {
    let mut fixture = retained_fixture("SCHEMA9_DELIVERY");
    let before = catalog(&fixture);
    let item = envelope(
        "SCHEMA9",
        PushKind::DataMode,
        DeliverySubKind::None,
        "2026-07-15",
        false,
    );
    fixture.coordinator.prepare(&item, 1, now()).unwrap();
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    let append = MemoryAppendPort::default();
    fixture
        .coordinator
        .resume_deliverable(&item.decision_identity, &sinks, &append, now())
        .unwrap();
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&item.decision_identity)
            .unwrap(),
        DecisionState::Delivered
    );
    drop(fixture.coordinator.take().unwrap());
    fixture.coordinator = FixtureCoordinator(Some(Arc::new(
        DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).unwrap(),
    )));
    fixture.coordinator.prepare(&item, 1, now()).unwrap();
    fixture
        .coordinator
        .resume_deliverable(&item.decision_identity, &sinks, &append, now())
        .unwrap();
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalog(&fixture), before);
    assert_eq!(fixture.query_i64("PRAGMA user_version"), 9);
}

#[test]
fn monitor_schema9_missing_trigger_and_changed_policy_fail_without_healing() {
    for policy in [false, true] {
        let mut fixture = retained_fixture("SCHEMA9_NO_HEAL");
        drop(fixture.coordinator.take().unwrap());
        let c = Connection::open(&fixture.database_path).unwrap();
        if policy {
            c.execute("UPDATE delivery_policy_catalog SET base_cooldown_secs=123 WHERE push_kind='DataMode'", []).unwrap();
        } else {
            c.execute_batch("DROP TRIGGER immutable_state_event_update")
                .unwrap();
        }
        drop(c);
        let before = catalog(&fixture);
        assert!(
            DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).is_err()
        );
        assert_eq!(catalog(&fixture), before);
        assert_eq!(fixture.query_i64("PRAGMA user_version"), 9);
        if policy {
            assert_eq!(fixture.query_i64("SELECT base_cooldown_secs FROM delivery_policy_catalog WHERE push_kind='DataMode'"), 123);
        }
    }
}

#[test]
fn monitor_schema9_rejects_other_headers_and_platform_extensions() {
    for version in [0, 8, 14] {
        let mut fixture = retained_fixture("SCHEMA9_HEADER");
        drop(fixture.coordinator.take().unwrap());
        Connection::open(&fixture.database_path)
            .unwrap()
            .pragma_update(None, "user_version", version)
            .unwrap();
        let before = catalog(&fixture);
        assert!(
            DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).is_err()
        );
        assert_eq!(catalog(&fixture), before);
        assert_eq!(fixture.query_i64("PRAGMA user_version"), version);
    }
    let mut fixture = retained_fixture("SCHEMA9_EXTENSION");
    drop(fixture.coordinator.take().unwrap());
    Connection::open(&fixture.database_path)
        .unwrap()
        .execute_batch("CREATE TABLE g5b_day_heads(TEST_CODE INTEGER)")
        .unwrap();
    let before = catalog(&fixture);
    assert!(DurableDeliveryCoordinator::open_existing_monitor_schema9(config(&fixture)).is_err());
    assert_eq!(catalog(&fixture), before);
}

#[test]
fn monitor_schema9_runtime_drift_blocks_new_delivery_without_schema_repair() {
    let fixture = retained_fixture("SCHEMA9_DRIFT");
    Connection::open(&fixture.database_path)
        .unwrap()
        .execute_batch("DROP TRIGGER immutable_attempt_event_update")
        .unwrap();
    let item = envelope(
        "SCHEMA9_DRIFT",
        PushKind::DataMode,
        DeliverySubKind::None,
        "2026-07-15",
        false,
    );
    assert!(fixture.coordinator.prepare(&item, 1, now()).is_err());
    assert_eq!(
        fixture.query_i64("SELECT count(*) FROM delivery_decisions"),
        0
    );
    assert_eq!(fixture.query_i64("PRAGMA user_version"), 9);
}
