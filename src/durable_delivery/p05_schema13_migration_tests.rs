//! Attested additive migration and no-heal regressions, not SQL-only success fixtures.
use super::*;

#[derive(Debug, Eq, PartialEq)]
struct OldSnapshot {
    catalog: Vec<(String, String, String, Option<String>)>,
    rows: BTreeMap<String, Vec<Vec<String>>>,
}
fn old_snapshot(connection: &Connection) -> OldSnapshot {
    let catalog=connection.prepare("SELECT type,name,tbl_name,sql FROM main.sqlite_master WHERE lower(name) NOT GLOB 'p05_*' AND lower(tbl_name) NOT GLOB 'p05_*' ORDER BY type,name,tbl_name").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let tables=connection.prepare("SELECT name FROM main.sqlite_master WHERE type='table' AND lower(name) NOT GLOB 'p05_*' ORDER BY name").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let rows = tables
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |r| {
                    (0..columns)
                        .map(|column| {
                            Ok(match r.get_ref(column)? {
                                rusqlite::types::ValueRef::Null => "null".into(),
                                rusqlite::types::ValueRef::Integer(value) => {
                                    format!("integer:{value}")
                                }
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
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (table, rows)
        })
        .collect();
    OldSnapshot { catalog, rows }
}
fn release_shape(fixture: &mut Fixture, version: i64) -> CoordinatorConfig {
    drop(fixture.coordinator.take().unwrap());
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::schema_p05_unit::remove_empty_extension_for_legacy_test(&connection);
    if version < 12 {
        super::super::schema_g5b_cohort::remove_empty_extension_for_legacy_test(&connection);
    }
    connection
        .pragma_update(None, "user_version", version)
        .unwrap();
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
        "TEST_CODE_P05_SCHEMA13_REOPEN_0123456789abcdef",
    )
}
fn version(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

#[test]
fn p05_unit_store_schema13_fresh_has_seven_empty_tables_and_unmodified_schema12_manifest() {
    let fixture = Fixture::new("P05_SCHEMA13_FRESH");
    let connection = Connection::open(&fixture.database_path).unwrap();
    assert_eq!(version(&fixture.database_path), 13);
    super::super::schema_p05_unit::verify_catalog(&connection).unwrap();
    super::super::schema_g5b_cohort::verify_catalog(&connection).unwrap();
    for table in super::super::schema_p05_unit::TABLES {
        assert_eq!(
            fixture.query_i64(&format!("SELECT COUNT(*) FROM {table}")),
            0
        );
    }
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_policy_catalog"),
        49
    );
}

#[test]
fn p05_unit_store_schema13_v12_additive_preserves_all_old_rows_raw_policies_sequence_and_g5b_ddl() {
    let mut fixture = Fixture::new("P05_SCHEMA13_FROM12");
    let append = MemoryAppendPort::default();
    let delivered = establish_authoritative_delivered_projection(
        &fixture,
        "P05_SCHEMA13_ACCEPTED",
        &append,
        false,
    );
    let unknown = envelope(
        "P05_SCHEMA13_UNKNOWN",
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "2026-07-30",
        false,
    );
    prepare_reserved(&fixture, &unknown, &append);
    let attempt = fixture
        .coordinator
        .begin_attempt(&unknown.decision_identity, 1, now())
        .unwrap()
        .unwrap();
    let recovered = now() + chrono::Duration::seconds(121);
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovered)
        .unwrap();
    fixture
        .coordinator
        .record_sink_result(
            &attempt.attempt_identity,
            attempt.fence_token,
            AuthoritativeSinkResult::Accepted(receipt(recovered)),
            recovered,
        )
        .unwrap();
    fixture
        .coordinator
        .reconcile_all_pending(&append, recovered)
        .unwrap();
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results WHERE authoritative_for_state=0 AND late_after_fence=1"),1);
    let config = release_shape(&mut fixture, 12);
    let connection = Connection::open(&fixture.database_path).unwrap();
    let before = old_snapshot(&connection);
    drop(connection);
    let reopened = DurableDeliveryCoordinator::open(config).unwrap();
    assert_eq!(version(&fixture.database_path), 13);
    let connection = Connection::open(&fixture.database_path).unwrap();
    assert_eq!(old_snapshot(&connection), before);
    super::super::schema_g5b_cohort::verify_catalog(&connection).unwrap();
    assert_eq!(
        reopened
            .decision_state(&delivered.decision_identity)
            .unwrap(),
        DecisionState::Delivered
    );
    assert_eq!(
        reopened.decision_state(&unknown.decision_identity).unwrap(),
        DecisionState::UncertainManualReview
    );
}

#[test]
fn p05_unit_store_schema13_v11_chain_adds_schema12_then13_without_changing_legacy_bytes() {
    let mut fixture = Fixture::new("P05_SCHEMA13_FROM11");
    let append = MemoryAppendPort::default();
    establish_authoritative_delivered_projection(
        &fixture,
        "P05_SCHEMA13_CHAIN_ACCEPTED",
        &append,
        false,
    );
    let config = release_shape(&mut fixture, 11);
    let connection = Connection::open(&fixture.database_path).unwrap();
    let before = old_snapshot(&connection);
    drop(connection);
    let reopened = DurableDeliveryCoordinator::open(config).unwrap();
    assert_eq!(version(&fixture.database_path), 13);
    let connection = Connection::open(&fixture.database_path).unwrap();
    // Only the exact schema12 objects may appear in the old-object snapshot.
    let after = old_snapshot(&connection);
    let legacy_catalog = after
        .catalog
        .into_iter()
        .filter(|(_, name, table, _)| !name.starts_with("g5b_") && !table.starts_with("g5b_"))
        .collect();
    let legacy_rows = after
        .rows
        .into_iter()
        .filter(|(table, _)| !table.starts_with("g5b_"))
        .collect();
    assert_eq!(
        OldSnapshot {
            catalog: legacy_catalog,
            rows: legacy_rows
        },
        before
    );
    super::super::schema_g5b_cohort::verify_catalog(&connection).unwrap();
    super::super::schema_p05_unit::verify_catalog(&connection).unwrap();
    drop(reopened);
}

#[test]
fn p05_unit_store_schema13_pre13_reserved_case_shadow_unknown_objects_are_never_adopted() {
    for legacy in [11, 12] {
        let mut fixture = Fixture::new(&format!("P05_SCHEMA13_RESERVED_{legacy}"));
        let config = release_shape(&mut fixture, legacy);
        let connection = Connection::open(&fixture.database_path).unwrap();
        let before = old_snapshot(&connection);
        connection.execute_batch("CREATE TABLE P05_foreign_unknown(payload BLOB); INSERT INTO P05_foreign_unknown VALUES(X'00FF');").unwrap();
        drop(connection);
        assert!(DurableDeliveryCoordinator::open(config).is_err());
        let connection = Connection::open(&fixture.database_path).unwrap();
        assert_eq!(version(&fixture.database_path), legacy);
        assert_eq!(old_snapshot(&connection), before);
        assert_eq!(
            connection
                .query_row("SELECT payload FROM P05_foreign_unknown", [], |r| r
                    .get::<_, Vec<u8>>(0))
                .unwrap(),
            vec![0, 255]
        );
    }
}

#[test]
fn p05_unit_store_schema13_current_missing_table_or_trigger_is_not_healed() {
    for object in [
        "TABLE p05_unit_children",
        "TRIGGER p05_unit_drafts_no_update",
    ] {
        let mut fixture = Fixture::new("P05_SCHEMA13_NO_HEAL");
        drop(fixture.coordinator.take().unwrap());
        let connection = Connection::open(&fixture.database_path).unwrap();
        connection.execute_batch(&format!("DROP {object}")).unwrap();
        let before = old_snapshot(&connection);
        drop(connection);
        let code = fixture
            .database_path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        assert!(DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            &fixture.database_path,
            code,
            "TEST_CODE_P05_SCHEMA13_DRIFT_0123456789abcdef"
        ))
        .is_err());
        let connection = Connection::open(&fixture.database_path).unwrap();
        assert_eq!(old_snapshot(&connection), before);
        assert_eq!(version(&fixture.database_path), 13);
        let name = object.split_whitespace().nth(1).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM main.sqlite_master WHERE name=?1",
                    [name],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}

#[test]
fn p05_unit_store_schema13_bootstrap_post_sql_failure_rolls_back_both_extensions_and_version() {
    let mut fixture = Fixture::new("P05_SCHEMA13_ROLLBACK");
    let config = release_shape(&mut fixture, 11);
    let connection = Connection::open(&fixture.database_path).unwrap();
    let before = old_snapshot(&connection);
    drop(connection);
    let _hook = install_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::AfterSchemaSqlBeforeCommitValidation,
        || {
            Err(DurableDeliveryError::IsolationViolation(
                "TEST_CODE P05 migration interrupt".into(),
            ))
        },
    )
    .unwrap();
    assert!(DurableDeliveryCoordinator::open(config).is_err());
    let connection = Connection::open(&fixture.database_path).unwrap();
    assert_eq!(version(&fixture.database_path), 11);
    assert_eq!(old_snapshot(&connection), before);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM main.sqlite_master WHERE lower(name) GLOB 'p05_*' OR lower(name) GLOB 'g5b_*'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}
