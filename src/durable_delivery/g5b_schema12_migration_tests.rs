//! Independent additive-migration regressions through the attested coordinator.
use super::*;

#[derive(Debug, PartialEq, Eq)]
struct LegacySnapshot {
    catalog: Vec<(String, String, String, Option<String>)>,
    rows: BTreeMap<String, Vec<(i64, Vec<String>)>>,
}

fn rowid_bound_values(connection: &Connection, table: &str) -> Vec<(i64, Vec<String>)> {
    assert!(table
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'));
    let mut query = connection
        .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
        .unwrap();
    let count = query.column_count();
    let values = query
        .query_map([], |row| {
            let values = (1..count)
                .map(|index| {
                    Ok(match row.get_ref(index)? {
                        rusqlite::types::ValueRef::Null => "null".to_owned(),
                        rusqlite::types::ValueRef::Integer(value) => format!("integer:{value}"),
                        rusqlite::types::ValueRef::Real(value) => {
                            format!("real:{:016x}", value.to_bits())
                        }
                        rusqlite::types::ValueRef::Text(value) => {
                            format!("text:{}", hex::encode(value))
                        }
                        rusqlite::types::ValueRef::Blob(value) => {
                            format!("blob:{}", hex::encode(value))
                        }
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((row.get(0)?, values))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    values
}

fn legacy_snapshot(connection: &Connection) -> LegacySnapshot {
    let mut query = connection
        .prepare(
            "SELECT type,name,tbl_name,sql FROM main.sqlite_master
         WHERE lower(name) NOT GLOB 'g5b_*' AND lower(tbl_name) NOT GLOB 'g5b_*'
         ORDER BY type,name,tbl_name",
        )
        .unwrap();
    let catalog = query
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let tables = connection
        .prepare(
            "SELECT name FROM main.sqlite_master WHERE type='table'
         AND lower(name) NOT GLOB 'g5b_*' ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let rows = tables
        .into_iter()
        .map(|table| {
            let values = rowid_bound_values(connection, &table);
            (table, values)
        })
        .collect();
    LegacySnapshot { catalog, rows }
}

fn release_and_shape_v11(fixture: &mut Fixture) -> (CoordinatorConfig, LegacySnapshot) {
    let coordinator = fixture.coordinator.take().unwrap();
    assert_eq!(Arc::strong_count(&coordinator), 1);
    drop(coordinator);
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::schema::register_sha256_function(&connection).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    // The extension must be empty. This removes only newly added test schema,
    // never any old table or actual cohort/intent/member evidence.
    super::super::schema_g5b_cohort::remove_empty_extension_for_legacy_test(&connection);
    connection
        .pragma_update(None, "user_version", 11_i64)
        .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    let before = legacy_snapshot(&connection);
    let code = fixture
        .database_path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let config = CoordinatorConfig::test(
        &fixture.database_path,
        code,
        "TEST_CODE_SCHEMA12_REOPEN_OWNER_0123456789abcdef",
    );
    (config, before)
}

fn version(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

#[test]
fn schema12_migration_fresh_attested_open_has_six_empty_tables_and_exact_version() {
    let fixture = Fixture::new("SCHEMA12_FRESH_ATTESTED");
    assert_eq!(version(&fixture.database_path), 12);
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::schema_g5b_cohort::verify_catalog(&connection).unwrap();
    for table in super::super::schema_g5b_cohort::TABLES {
        assert_eq!(
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_policy_catalog"),
        49
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(DISTINCT push_kind) FROM delivery_policy_catalog"),
        46
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM pragma_foreign_key_check"),
        0
    );
}

#[test]
fn schema12_migration_populated_v11_preserves_every_legacy_typed_value_and_catalog() {
    let mut fixture = Fixture::new("SCHEMA12_POPULATED_V11");
    let append = MemoryAppendPort::default();
    let delivered =
        establish_authoritative_delivered_projection(&fixture, "SCHEMA12_ACCEPTED", &append, false);
    let uncertain = envelope(
        "SCHEMA12_UNCERTAIN",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &uncertain, &append);
    let uncertain_sink = StaticSink::new(AuthoritativeSinkResult::Uncertain(uncertainty(now())));
    let uncertain_sinks: [AuthoritativeSink; 1] = [uncertain_sink];
    fixture
        .coordinator
        .resume_deliverable(&uncertain.decision_identity, &uncertain_sinks, now())
        .unwrap();
    reconcile_terminal(
        &fixture,
        &append,
        DecisionState::UncertainManualReview,
        &uncertain.decision_identity,
    );

    // An actual v1 G5b decision, expired attempt and non-authoritative raw late
    // receipt must remain legacy facts, never be adopted into a v2 cohort.
    let legacy_g5b = g5b_frozen_envelope("SCHEMA12_LEGACY_RAW_LATE", false);
    prepare_reserved(&fixture, &legacy_g5b, &append);
    let attempt = fixture
        .coordinator
        .begin_attempt(&legacy_g5b.decision_identity, 1, now())
        .unwrap()
        .unwrap();
    let recovered_at = now() + chrono::Duration::seconds(121);
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .unwrap();
    fixture
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            AuthoritativeSinkResult::Accepted(receipt(recovered_at)),
            recovered_at,
        )
        .unwrap();
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovered_at)
        .unwrap();
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results WHERE authoritative_for_state=0 AND late_after_fence=1"),1);
    assert_eq!(
        fixture
            .coordinator
            .decision_state(&delivered.decision_identity)
            .unwrap(),
        DecisionState::Delivered
    );

    let (config, before) = release_and_shape_v11(&mut fixture);
    let reopened = DurableDeliveryCoordinator::open(config).unwrap();
    assert_eq!(version(&fixture.database_path), 12);
    let connection = Connection::open(&fixture.database_path).unwrap();
    assert_eq!(legacy_snapshot(&connection),before,"additive migration must preserve DDL, TEXT/BLOB bytes, numeric types, and row ordering facts");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM g5b_cohorts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(connection);
    assert_eq!(
        reopened
            .decision_state(&legacy_g5b.decision_identity)
            .unwrap(),
        DecisionState::UncertainManualReview
    );
    assert_eq!(
        reopened
            .decision_state(&uncertain.decision_identity)
            .unwrap(),
        DecisionState::UncertainManualReview
    );
    let forbidden = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(recovered_at)));
    let forbidden_sinks: [AuthoritativeSink; 1] = [forbidden.clone()];
    let observed = reopened
        .resume_deliverable(&delivered.decision_identity, &forbidden_sinks, recovered_at)
        .unwrap();
    assert_eq!(observed.sink_calls, 0);
    assert_eq!(forbidden.calls.load(Ordering::SeqCst), 0);
    drop(reopened);
}

#[test]
fn schema12_migration_unknown_reserved_half_schema_is_not_adopted() {
    let mut fixture = Fixture::new("SCHEMA12_RESERVED_HALF_SCHEMA");
    let (config, before) = release_and_shape_v11(&mut fixture);
    let connection = Connection::open(&fixture.database_path).unwrap();
    connection.execute_batch("CREATE TABLE g5b_day_heads(business_date TEXT PRIMARY KEY,foreign_payload BLOB); INSERT INTO g5b_day_heads VALUES('2026-07-30',X'00FF');").unwrap();
    let unknown: Vec<u8> = connection
        .query_row("SELECT foreign_payload FROM g5b_day_heads", [], |row| {
            row.get(0)
        })
        .unwrap();
    drop(connection);
    assert!(DurableDeliveryCoordinator::open(config).is_err());
    let after = Connection::open(&fixture.database_path).unwrap();
    assert_eq!(version(&fixture.database_path), 11);
    assert_eq!(legacy_snapshot(&after), before);
    assert_eq!(
        after
            .query_row("SELECT foreign_payload FROM g5b_day_heads", [], |row| row
                .get::<_, Vec<
                u8,
            >>(
                0
            ))
            .unwrap(),
        unknown
    );
    assert_eq!(
        after
            .query_row(
                "SELECT COUNT(*) FROM main.sqlite_master WHERE type='table' AND name GLOB 'g5b_*'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn schema12_migration_bootstrap_failure_rolls_back_extension_and_old_bytes() {
    let mut fixture = Fixture::new("SCHEMA12_BOOTSTRAP_ROLLBACK");
    let append = MemoryAppendPort::default();
    let _delivered = establish_authoritative_delivered_projection(
        &fixture,
        "SCHEMA12_ROLLBACK_ACCEPTED",
        &append,
        false,
    );
    let (config, before) = release_and_shape_v11(&mut fixture);
    let _hook = install_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::AfterSchemaSqlBeforeCommitValidation,
        || {
            Err(DurableDeliveryError::IsolationViolation(
                "TEST_CODE schema12 interrupt after migration SQL".to_owned(),
            ))
        },
    )
    .unwrap();
    let result = DurableDeliveryCoordinator::open(config);
    assert!(
        matches!(result,Err(DurableDeliveryError::IsolationViolation(reason)) if reason.contains("schema12 interrupt after migration SQL"))
    );
    let after = Connection::open(&fixture.database_path).unwrap();
    assert_eq!(version(&fixture.database_path), 11);
    assert_eq!(legacy_snapshot(&after), before);
    assert_eq!(after.query_row("SELECT COUNT(*) FROM main.sqlite_master WHERE name GLOB 'g5b_*' OR tbl_name GLOB 'g5b_*'",[], |row|row.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn schema12_migration_current_version_missing_guard_is_never_healed() {
    let mut fixture = Fixture::new("SCHEMA12_NO_HEAL_CURRENT");
    let coordinator = fixture.coordinator.take().unwrap();
    assert_eq!(Arc::strong_count(&coordinator), 1);
    drop(coordinator);
    let connection = Connection::open(&fixture.database_path).unwrap();
    let trigger: String = connection.query_row("SELECT name FROM main.sqlite_master WHERE type='trigger' AND name GLOB 'g5b_*' ORDER BY name LIMIT 1",[], |row|row.get(0)).unwrap();
    connection
        .execute_batch(&format!("DROP TRIGGER {trigger}"))
        .unwrap();
    let before = legacy_snapshot(&connection);
    drop(connection);
    let code = fixture
        .database_path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let config = CoordinatorConfig::test(
        &fixture.database_path,
        code,
        "TEST_CODE_SCHEMA12_DRIFT_OWNER_0123456789abcdef",
    );
    assert!(DurableDeliveryCoordinator::open(config).is_err());
    let after = Connection::open(&fixture.database_path).unwrap();
    assert_eq!(version(&fixture.database_path), 12);
    assert_eq!(legacy_snapshot(&after), before);
    assert_eq!(
        after
            .query_row(
                "SELECT COUNT(*) FROM main.sqlite_master WHERE name=?1",
                [trigger],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
