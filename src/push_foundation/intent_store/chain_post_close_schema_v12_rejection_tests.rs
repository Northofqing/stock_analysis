use super::*;
use rusqlite::params;
use rusqlite::types::Value;
use std::time::Duration;

const FUTURE_14_SHA256: &str =
    "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const FUTURE_SHA256: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const QUERY_TERMINALS: &str = "chain_post_close_macro_query_terminals";

#[derive(Clone, Copy)]
enum RejectionScenario {
    HalfInstalled,
    OwnedCatalogDrift,
    HeaderDrift,
    RegistryDrift,
    FutureLayout,
    FrozenV11Reader,
}

impl RejectionScenario {
    fn run_id(self) -> &'static str {
        match self {
            Self::HalfInstalled => "TEST_CODE_V12_HALF_INSTALLED_REJECTION",
            Self::OwnedCatalogDrift => "TEST_CODE_V12_OWNED_CATALOG_DRIFT_REJECTION",
            Self::HeaderDrift => "TEST_CODE_V12_HEADER_DRIFT_REJECTION",
            Self::RegistryDrift => "TEST_CODE_V12_REGISTRY_DRIFT_REJECTION",
            Self::FutureLayout => "TEST_CODE_V12_FUTURE_LAYOUT_REJECTION",
            Self::FrozenV11Reader => "TEST_CODE_V12_FROZEN_V11_READER_REJECTION",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::HalfInstalled => "v12 half-installed rejection",
            Self::OwnedCatalogDrift => "v12 owned-catalog drift rejection",
            Self::HeaderDrift => "v12 header drift rejection",
            Self::RegistryDrift => "v12 registry drift rejection",
            Self::FutureLayout => "v12 future-layout rejection",
            Self::FrozenV11Reader => "v12 frozen-v11-reader rejection",
        }
    }

    fn installs_v12(self) -> bool {
        matches!(
            self,
            Self::HeaderDrift | Self::RegistryDrift | Self::FutureLayout | Self::FrozenV11Reader
        )
    }
}

fn assert_confirmed_v11_baseline(
    business: &mut V2BusinessFixture,
    config: &crate::monitor::push_job::LocalChainPostCloseConfig,
    intent: &crate::monitor::push_job::IntentId,
) {
    assert_real_v11_macro_facts(business.connection());
    let mut local = business
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(config)
        .unwrap();
    let recovery = local.inspect_macro(intent).unwrap();
    assert!(!recovery.is_complete());
    assert!(!recovery.has_unconfirmed_effect());
    assert!(recovery
        .global_news(GlobalNewsProvider::Eastmoney)
        .unwrap()
        .is_complete());
    assert_eq!(
        recovery.plan().deadline_at().get() - recovery.plan().started_at().get(),
        15_000_000
    );
    drop(local);
}

fn assert_same_connection_state(
    before: &v11_migration_tests::DatabaseState,
    after: &v11_migration_tests::DatabaseState,
) {
    assert_eq!(after.application_id, before.application_id);
    assert_eq!(after.user_version, before.user_version);
    assert_eq!(after.query_only, before.query_only);
    assert_eq!(after.autocommit, before.autocommit);
}

fn assert_half_installed_delta(
    clean: &v11_migration_tests::DatabaseState,
    damaged: &v11_migration_tests::DatabaseState,
) {
    assert_same_connection_state(clean, damaged);
    assert_eq!(damaged.tables.len(), clean.tables.len() + 1);
    for (name, rows) in &clean.tables {
        assert_eq!(
            &damaged.tables[name], rows,
            "TEST_CODE half-install changed {name}"
        );
    }
    assert!(damaged.tables[QUERY_TERMINALS].is_empty());

    for old in &clean.catalog {
        assert_eq!(
            catalog_row(damaged, text(old, 1)),
            old,
            "TEST_CODE half-install changed old catalog object {}",
            text(old, 1)
        );
    }
    let mut additions = damaged
        .catalog
        .iter()
        .filter(|row| !clean.catalog.iter().any(|old| old[1] == row[1]))
        .map(|row| {
            (
                text(row, 0).to_owned(),
                text(row, 1).to_owned(),
                text(row, 2).to_owned(),
                row[4].clone(),
            )
        })
        .collect::<Vec<_>>();
    additions.sort_by(|left, right| left.1.cmp(&right.1));
    assert_eq!(additions.len(), 4);
    assert_eq!(additions[0].0, "table");
    assert_eq!(additions[0].1, QUERY_TERMINALS);
    assert_eq!(additions[0].2, QUERY_TERMINALS);
    assert!(matches!(&additions[0].3, Value::Blob(_)));
    assert_eq!(
        additions[1..]
            .iter()
            .map(|(kind, name, table, sql)| {
                assert_eq!(kind, "index");
                assert_eq!(table, QUERY_TERMINALS);
                assert_eq!(sql, &Value::Null);
                name.as_str()
            })
            .collect::<Vec<_>>(),
        vec![
            "sqlite_autoindex_chain_post_close_macro_query_terminals_1",
            "sqlite_autoindex_chain_post_close_macro_query_terminals_2",
            "sqlite_autoindex_chain_post_close_macro_query_terminals_3",
        ]
    );
    assert_eq!(damaged.catalog.len(), clean.catalog.len() + 4);
}

fn assert_only_catalog_definition_changed(
    clean: &v11_migration_tests::DatabaseState,
    damaged: &v11_migration_tests::DatabaseState,
    target: &str,
) {
    assert_same_connection_state(clean, damaged);
    assert_eq!(damaged.tables, clean.tables);
    assert_eq!(damaged.catalog.len(), clean.catalog.len());
    let mut changed = 0;
    for old in &clean.catalog {
        let name = text(old, 1);
        let current = catalog_row(damaged, name);
        if name == target {
            assert_eq!(&current[..4], &old[..4]);
            assert_ne!(definition(current), definition(old));
            let original = std::str::from_utf8(definition(old)).unwrap();
            assert_eq!(original.matches("'chain v11 macro immutable'").count(), 1);
            assert_eq!(
                definition(current),
                original
                    .replace(
                        "'chain v11 macro immutable'",
                        "'TEST_CODE chain v11 macro immutable drift'",
                    )
                    .as_bytes()
            );
            changed += 1;
        } else {
            assert_eq!(current, old, "TEST_CODE unexpected catalog drift: {name}");
        }
    }
    assert_eq!(changed, 1);
}

fn assert_only_v12_metadata_cell_changed(
    clean: &v11_migration_tests::DatabaseState,
    damaged: &v11_migration_tests::DatabaseState,
    scenario: RejectionScenario,
) {
    assert_same_connection_state(clean, damaged);
    assert_eq!(damaged.catalog, clean.catalog);
    for (name, rows) in &clean.tables {
        if !matches!(
            (scenario, name.as_str()),
            (RejectionScenario::HeaderDrift, "chain_post_close_layouts")
                | (
                    RejectionScenario::RegistryDrift,
                    "chain_post_close_layout_objects"
                )
        ) {
            assert_eq!(
                &damaged.tables[name], rows,
                "TEST_CODE metadata drift changed {name}"
            );
        }
    }

    match scenario {
        RejectionScenario::HeaderDrift => {
            let mut expected = clean.tables["chain_post_close_layouts"].clone();
            let row = expected
                .iter_mut()
                .find(|row| row[0] == Value::Integer(12))
                .unwrap();
            assert_eq!(
                row[6],
                Value::Text("chain-post-close-layout-v12".to_owned())
            );
            row[6] = Value::Text("TEST_CODE_DAMAGED_V12_HEADER".to_owned());
            assert_eq!(damaged.tables["chain_post_close_layouts"], expected);
            let old = clean.tables["chain_post_close_layouts"]
                .iter()
                .find(|row| row[0] == Value::Integer(12))
                .unwrap();
            let current = damaged.tables["chain_post_close_layouts"]
                .iter()
                .find(|row| row[0] == Value::Integer(12))
                .unwrap();
            assert_eq!(
                clean.tables["chain_post_close_layouts"].len(),
                damaged.tables["chain_post_close_layouts"].len()
            );
            for column in 0..old.len() {
                if column == 6 {
                    assert_eq!(
                        old[column],
                        Value::Text("chain-post-close-layout-v12".to_owned())
                    );
                    assert_eq!(
                        current[column],
                        Value::Text("TEST_CODE_DAMAGED_V12_HEADER".to_owned())
                    );
                } else {
                    assert_eq!(current[column], old[column]);
                }
            }
        }
        RejectionScenario::RegistryDrift => {
            let is_target = |row: &&Vec<Value>| {
                row[0] == Value::Integer(12) && row[1] == Value::Text(QUERY_TERMINALS.to_owned())
            };
            let mut expected = clean.tables["chain_post_close_layout_objects"].clone();
            let expected_row = expected
                .iter_mut()
                .find(|row| {
                    row[0] == Value::Integer(12)
                        && row[1] == Value::Text(QUERY_TERMINALS.to_owned())
                })
                .unwrap();
            let expected_definition = match &expected_row[3] {
                Value::Text(value) => format!("{value} "),
                other => panic!("TEST_CODE v12 registry definition: {other:?}"),
            };
            expected_row[3] = Value::Text(expected_definition);
            assert_eq!(damaged.tables["chain_post_close_layout_objects"], expected);
            let old = clean.tables["chain_post_close_layout_objects"]
                .iter()
                .find(is_target)
                .unwrap();
            let current = damaged.tables["chain_post_close_layout_objects"]
                .iter()
                .find(is_target)
                .unwrap();
            assert_eq!(
                clean.tables["chain_post_close_layout_objects"].len(),
                damaged.tables["chain_post_close_layout_objects"].len()
            );
            assert_eq!(&current[..3], &old[..3]);
            assert_eq!(
                current[3],
                Value::Text(format!(
                    "{} ",
                    match &old[3] {
                        Value::Text(value) => value,
                        other => panic!("TEST_CODE v12 registry definition: {other:?}"),
                    }
                ))
            );
        }
        _ => panic!("TEST_CODE metadata assertion used for non-metadata scenario"),
    }
}

fn assert_future_layout_delta(
    clean: &v11_migration_tests::DatabaseState,
    damaged: &v11_migration_tests::DatabaseState,
) {
    assert_same_connection_state(clean, damaged);
    assert_eq!(damaged.catalog, clean.catalog);
    for (name, rows) in &clean.tables {
        if name != "chain_post_close_layouts" && name != "chain_post_close_layout_objects" {
            assert_eq!(
                &damaged.tables[name], rows,
                "TEST_CODE future layout changed {name}"
            );
        }
    }
    let forged = [Value::Integer(13), Value::Integer(14)];
    let old_headers = damaged.tables["chain_post_close_layouts"]
        .iter()
        .filter(|row| !forged.contains(&row[0]))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(old_headers, clean.tables["chain_post_close_layouts"]);
    assert_eq!(
        damaged.tables["chain_post_close_layouts"].len(),
        clean.tables["chain_post_close_layouts"].len() + 2
    );
    let header = damaged.tables["chain_post_close_layouts"]
        .iter()
        .find(|row| row[0] == Value::Integer(14))
        .unwrap();
    assert_eq!(
        header,
        &vec![
            Value::Integer(14),
            Value::Integer(13),
            Value::Text(FUTURE_SHA256.to_owned()),
            Value::Integer(1),
            Value::Integer(1),
            Value::Integer(1),
            Value::Text("TEST_CODE_chain-post-close-layout-v14".to_owned()),
            Value::Text(FUTURE_14_SHA256.to_owned()),
        ]
    );

    let old_registry = damaged.tables["chain_post_close_layout_objects"]
        .iter()
        .filter(|row| !forged.contains(&row[0]))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        old_registry,
        clean.tables["chain_post_close_layout_objects"]
    );
    let future_registry = damaged.tables["chain_post_close_layout_objects"]
        .iter()
        .filter(|row| forged.contains(&row[0]))
        .collect::<Vec<_>>();
    assert_eq!(future_registry.len(), 482);
    for future in future_registry {
        let source = clean.tables["chain_post_close_layout_objects"]
            .iter()
            .find(|row| row[0] == Value::Integer(12) && row[1..] == future[1..])
            .unwrap();
        assert_eq!(&future[1..], &source[1..]);
    }
}

fn install_first_v12_table(business: &V2BusinessFixture) {
    let (first_table, _) = include_str!("chain_post_close.v12.sql")
        .split_once("CREATE TABLE chain_post_close_macro_dimension_terminals")
        .expect("TEST_CODE frozen v12 second table marker");
    business.connection().execute_batch(first_table).unwrap();
}

fn drift_v11_owned_catalog(business: &V2BusinessFixture) {
    const TRIGGER: &str = "chain_post_close_macro_plans_update";
    let original: String = business
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            [TRIGGER],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original.matches("'chain v11 macro immutable'").count(), 1);
    let drifted = original.replace(
        "'chain v11 macro immutable'",
        "'TEST_CODE chain v11 macro immutable drift'",
    );
    let transaction = business.connection().unchecked_transaction().unwrap();
    transaction
        .execute_batch("DROP TRIGGER chain_post_close_macro_plans_update")
        .unwrap();
    transaction.execute_batch(&drifted).unwrap();
    transaction.commit().unwrap();
}

fn drift_v12_metadata(business: &V2BusinessFixture, scenario: RejectionScenario) {
    let (trigger_name, update, parameters): (&str, &str, Vec<String>) = match scenario {
        RejectionScenario::HeaderDrift => (
            "chain_post_close_layouts_update",
            "UPDATE chain_post_close_layouts SET description=?1 WHERE layout_version=12",
            vec!["TEST_CODE_DAMAGED_V12_HEADER".to_owned()],
        ),
        RejectionScenario::RegistryDrift => {
            let original: String = business
                .connection()
                .query_row(
                    "SELECT definition FROM chain_post_close_layout_objects WHERE layout_version=12 AND name=?1",
                    [QUERY_TERMINALS],
                    |row| row.get(0),
                )
                .unwrap();
            (
                "chain_post_close_layout_objects_update",
                "UPDATE chain_post_close_layout_objects SET definition=?1 WHERE layout_version=12 AND name=?2",
                vec![format!("{original} "), QUERY_TERMINALS.to_owned()],
            )
        }
        _ => panic!("TEST_CODE metadata injection used for non-metadata scenario"),
    };
    let trigger: String = business
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            [trigger_name],
            |row| row.get(0),
        )
        .unwrap();
    let transaction = business.connection().unchecked_transaction().unwrap();
    transaction
        .execute_batch(&format!("DROP TRIGGER {trigger_name}"))
        .unwrap();
    let changed = match parameters.as_slice() {
        [value] => transaction.execute(update, [value]).unwrap(),
        [value, name] => transaction.execute(update, params![value, name]).unwrap(),
        _ => unreachable!(),
    };
    assert_eq!(changed, 1);
    transaction.execute_batch(&trigger).unwrap();
    transaction.commit().unwrap();
}

fn install_future_layout(business: &V2BusinessFixture) {
    let transaction = business.connection().unchecked_transaction().unwrap();
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
                 SELECT 13,name,object_type,definition FROM chain_post_close_layout_objects \
                 WHERE layout_version=12",
                [],
            )
            .unwrap(),
        241
    );
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_layouts( \
                 layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                 artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256) \
                 VALUES(13,12,?1,1,1,1,'TEST_CODE_chain-post-close-layout-v13',?2)",
                params![V12_SHA256, FUTURE_SHA256],
            )
            .unwrap(),
        1
    );
    // Layout 14 is the first version this build does not know; 13 only exists
    // so the predecessor chain is well-formed.
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
                 SELECT 14,name,object_type,definition FROM chain_post_close_layout_objects \
                 WHERE layout_version=13",
                [],
            )
            .unwrap(),
        241
    );
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO chain_post_close_layouts( \
                 layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
                 artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256) \
                 VALUES(14,13,?1,1,1,1,'TEST_CODE_chain-post-close-layout-v14',?2)",
                params![FUTURE_SHA256, FUTURE_14_SHA256],
            )
            .unwrap(),
        1
    );
    transaction.commit().unwrap();
}

async fn run_rejection_scenario(scenario: RejectionScenario) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), async {
            let control_tests::ConfirmedExternalBaseline { config, intent, .. } =
                control_tests::establish_confirmed_external_first_source(
                    &mut business,
                    &mut parent_server,
                    &mut external_server,
                    scenario.run_id(),
                )
                .await;
            let external_network = external_server.as_ref().unwrap().snapshot();
            let parent_network = parent_server.as_ref().unwrap().snapshot();
            let memberships = parent_server.as_ref().unwrap().membership_snapshot();
            macro_rules! assert_no_rpc {
                () => {
                    assert_eq!(
                        external_server.as_ref().unwrap().snapshot(),
                        external_network
                    );
                    assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_network);
                    assert_eq!(
                        parent_server.as_ref().unwrap().membership_snapshot(),
                        memberships
                    );
                };
            }

            business.reopen();
            let v11 = business.chain_post_close().verify_schema().unwrap();
            assert_eq!(v11.schema_version(), 11);
            assert_eq!(v11.ddl_sha256().as_str(), V11_SHA256);
            assert_confirmed_v11_baseline(&mut business, &config, &intent);
            assert_no_rpc!();

            if scenario.installs_v12() {
                let before_v12 = v11_migration_tests::DatabaseState::capture(business.connection());
                let migrated = business
                    .chain_post_close()
                    .migrate_schema_v11_to_v12()
                    .unwrap();
                assert_eq!(migrated.schema_version(), 12);
                assert_eq!(migrated.ddl_sha256().as_str(), V12_SHA256);
                assert_eq!(
                    business.chain_post_close().verify_schema().unwrap(),
                    migrated
                );
                let clean_v12 = v11_migration_tests::DatabaseState::capture(business.connection());
                assert_old_rows_preserved(&before_v12, &clean_v12);
                assert_v12_metadata(business.connection());
                assert_no_rpc!();
            }

            let clean = v11_migration_tests::DatabaseState::capture(business.connection());
            match scenario {
                RejectionScenario::HalfInstalled => install_first_v12_table(&business),
                RejectionScenario::OwnedCatalogDrift => drift_v11_owned_catalog(&business),
                RejectionScenario::HeaderDrift | RejectionScenario::RegistryDrift => {
                    drift_v12_metadata(&business, scenario)
                }
                RejectionScenario::FutureLayout => install_future_layout(&business),
                RejectionScenario::FrozenV11Reader => {}
            }
            let damaged = v11_migration_tests::DatabaseState::capture(business.connection());
            match scenario {
                RejectionScenario::HalfInstalled => assert_half_installed_delta(&clean, &damaged),
                RejectionScenario::OwnedCatalogDrift => assert_only_catalog_definition_changed(
                    &clean,
                    &damaged,
                    "chain_post_close_macro_plans_update",
                ),
                RejectionScenario::HeaderDrift | RejectionScenario::RegistryDrift => {
                    assert_only_v12_metadata_cell_changed(&clean, &damaged, scenario)
                }
                RejectionScenario::FutureLayout => assert_future_layout_delta(&clean, &damaged),
                RejectionScenario::FrozenV11Reader => assert_eq!(damaged, clean),
            }
            assert_no_rpc!();

            match scenario {
                RejectionScenario::HalfInstalled | RejectionScenario::OwnedCatalogDrift => {
                    assert_eq!(
                        business.chain_post_close().verify_schema(),
                        Err(ChainPostCloseError::SchemaRejected)
                    );
                    assert_eq!(
                        v11_migration_tests::DatabaseState::capture(business.connection()),
                        damaged
                    );
                    assert_no_rpc!();
                    assert_eq!(
                        business.chain_post_close().migrate_schema_v11_to_v12(),
                        Err(ChainPostCloseError::SchemaRejected)
                    );
                }
                RejectionScenario::HeaderDrift | RejectionScenario::RegistryDrift => {
                    assert_eq!(
                        business.chain_post_close().verify_schema(),
                        Err(ChainPostCloseError::SchemaRejected)
                    );
                }
                RejectionScenario::FutureLayout => {
                    assert_eq!(
                        business.chain_post_close().verify_schema(),
                        Err(ChainPostCloseError::UnsupportedVersion)
                    );
                }
                RejectionScenario::FrozenV11Reader => {
                    let current = business.chain_post_close().verify_schema().unwrap();
                    assert_eq!(current.schema_version(), 12);
                    assert_eq!(current.ddl_sha256().as_str(), V12_SHA256);
                    assert_eq!(
                        v11_migration_tests::DatabaseState::capture(business.connection()),
                        damaged
                    );
                    assert_no_rpc!();
                    assert_eq!(
                        business.chain_post_close().verify_schema_v11_reader(),
                        Err(ChainPostCloseError::UnsupportedVersion)
                    );
                }
            }
            assert_eq!(
                v11_migration_tests::DatabaseState::capture(business.connection()),
                damaged
            );
            assert_no_rpc!();

            business.reopen();
            assert_eq!(
                v11_migration_tests::DatabaseState::capture(business.connection()),
                damaged
            );
            assert_no_rpc!();
            if matches!(scenario, RejectionScenario::FrozenV11Reader) {
                let current = business.chain_post_close().verify_schema().unwrap();
                assert_eq!(current.schema_version(), 12);
                assert_eq!(current.ddl_sha256().as_str(), V12_SHA256);
                assert_eq!(
                    business.chain_post_close().verify_schema_v11_reader(),
                    Err(ChainPostCloseError::UnsupportedVersion)
                );
                assert_eq!(
                    v11_migration_tests::DatabaseState::capture(business.connection()),
                    damaged
                );
                assert_no_rpc!();
            }
        }))
        .catch_unwind()
        .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        scenario.label(),
    )
    .await;
    drop(business);
    match body {
        Ok(Ok(())) => {}
        Ok(Err(_)) => panic!("TEST_CODE {} body deadline", scenario.label()),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn v11_to_v12_half_installed_first_query_terminal_table_is_rejected_without_repair_or_rpc() {
    run_rejection_scenario(RejectionScenario::HalfInstalled).await;
}

#[tokio::test]
async fn v11_to_v12_owned_catalog_definition_drift_is_rejected_without_repair_or_rpc() {
    run_rejection_scenario(RejectionScenario::OwnedCatalogDrift).await;
}

#[tokio::test]
async fn real_v12_metadata_drift_is_rejected_without_repair_or_rpc() {
    run_rejection_scenario(RejectionScenario::HeaderDrift).await;
    run_rejection_scenario(RejectionScenario::RegistryDrift).await;
}

#[tokio::test]
async fn real_v12_with_unknown_future_layout_is_unsupported_without_repair_or_rpc() {
    run_rejection_scenario(RejectionScenario::FutureLayout).await;
}

#[tokio::test]
async fn frozen_v11_reader_rejects_real_v12_layout_without_rpc() {
    run_rejection_scenario(RejectionScenario::FrozenV11Reader).await;
}
