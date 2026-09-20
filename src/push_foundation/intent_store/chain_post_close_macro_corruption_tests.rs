use super::*;
use crate::grpc_client::client::board_loopback_fixture::BoardLoopbackServer;
use crate::grpc_client::client::macro_full_loopback_fixture::MacroFullLoopbackServer;
use crate::pipeline::chain_analysis::preparation::ChainPreparationIo;
use crate::search_service::macro_news::runner::QueryKey;
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use std::collections::{BTreeMap, BTreeSet};

const STARTED_LOCAL: &str = "2026-09-14T15:31:00+08:00";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CorruptionCase {
    RequestWrapperIdentity,
    ControlRawVersion,
    DataRawVersion,
    SourceNativeContent,
    Br159ReceiptHash,
    PlanParentOwner,
}

impl CorruptionCase {
    fn label(self) -> &'static str {
        match self {
            Self::RequestWrapperIdentity => "request-wrapper-identity",
            Self::ControlRawVersion => "control-raw-version",
            Self::DataRawVersion => "data-raw-version",
            Self::SourceNativeContent => "source-native-content",
            Self::Br159ReceiptHash => "br159-receipt-hash",
            Self::PlanParentOwner => "plan-parent-owner",
        }
    }

    fn run_id(self) -> &'static str {
        match self {
            Self::RequestWrapperIdentity => "TEST_CODE_RUN_MACRO_CORRUPT_REQUEST_IDENTITY",
            Self::ControlRawVersion => "TEST_CODE_RUN_MACRO_CORRUPT_CONTROL_RAW",
            Self::DataRawVersion => "TEST_CODE_RUN_MACRO_CORRUPT_DATA_RAW",
            Self::SourceNativeContent => "TEST_CODE_RUN_MACRO_CORRUPT_SOURCE_NATIVE",
            Self::Br159ReceiptHash => "TEST_CODE_RUN_MACRO_CORRUPT_BR159_RECEIPT",
            Self::PlanParentOwner => "TEST_CODE_RUN_MACRO_CORRUPT_PARENT_OWNER",
        }
    }

    fn table(self) -> &'static str {
        match self {
            Self::RequestWrapperIdentity => "chain_post_close_macro_request_plans",
            Self::ControlRawVersion => "chain_post_close_macro_control_attempt_results",
            Self::DataRawVersion => "chain_post_close_macro_attempt_results",
            Self::SourceNativeContent | Self::Br159ReceiptHash => {
                "chain_post_close_macro_source_finals"
            }
            Self::PlanParentOwner => "chain_post_close_macro_plans",
        }
    }

    fn trigger(self) -> &'static str {
        match self {
            Self::RequestWrapperIdentity => "chain_post_close_macro_request_plans_update",
            Self::ControlRawVersion => {
                "chain_post_close_macro_control_attempt_results_update"
            }
            Self::DataRawVersion => "chain_post_close_macro_attempt_results_update",
            Self::SourceNativeContent | Self::Br159ReceiptHash => {
                "chain_post_close_macro_source_finals_update"
            }
            Self::PlanParentOwner => "chain_post_close_macro_plans_update",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct DatabaseSnapshot {
    catalog: Vec<Vec<rusqlite::types::Value>>,
    rows: BTreeMap<String, Vec<Vec<rusqlite::types::Value>>>,
}

enum DamageValues {
    FactBytes {
        bytes: Vec<u8>,
        length: i64,
        digest: String,
    },
    SourceNative {
        wrapper: Vec<u8>,
        wrapper_length: i64,
        wrapper_digest: String,
        native: Vec<u8>,
        native_length: i64,
        native_digest: String,
    },
    ReceiptHash(String),
}

struct PreparedDamage {
    expected: DatabaseSnapshot,
    values: DamageValues,
}

fn database_snapshot(connection: &Connection) -> DatabaseSnapshot {
    let catalog = all_rows(
        connection,
        "SELECT type,name,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema ORDER BY name",
    );
    let names = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let rows = names
        .into_iter()
        .map(|name| {
            let rows = all_rows(connection, &format!("SELECT * FROM \"{name}\""));
            (name, rows)
        })
        .collect();
    DatabaseSnapshot { catalog, rows }
}

fn snapshot_path(database: &std::path::Path) -> DatabaseSnapshot {
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection.busy_timeout(Duration::from_millis(250)).unwrap();
    let snapshot = database_snapshot(&connection);
    connection.close().unwrap();
    snapshot
}

fn changed_tables<'a>(
    before: &'a DatabaseSnapshot,
    after: &'a DatabaseSnapshot,
) -> Vec<&'a str> {
    assert_eq!(after.catalog, before.catalog);
    assert_eq!(
        after.rows.keys().collect::<Vec<_>>(),
        before.rows.keys().collect::<Vec<_>>()
    );
    before
        .rows
        .iter()
        .filter_map(|(name, rows)| {
            (after.rows.get(name).expect("TEST_CODE snapshot table") != rows)
                .then_some(name.as_str())
        })
        .collect()
}

fn replace_once(bytes: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    assert!(!needle.is_empty());
    assert_eq!(needle.len(), replacement.len());
    let offsets = bytes
        .windows(needle.len())
        .enumerate()
        .filter_map(|(offset, value)| (value == needle).then_some(offset))
        .collect::<Vec<_>>();
    assert_eq!(offsets.len(), 1);
    let mut changed = bytes.to_vec();
    changed[offsets[0]..offsets[0] + needle.len()].copy_from_slice(replacement);
    assert_ne!(changed, bytes);
    changed
}

fn blob_digest(bytes: &[u8]) -> (i64, String) {
    (
        i64::try_from(bytes.len()).unwrap(),
        raw_digest(bytes).as_str().to_owned(),
    )
}

fn text(row: &[rusqlite::types::Value], column: usize) -> &str {
    match &row[column] {
        rusqlite::types::Value::Text(value) => value,
        _ => panic!("TEST_CODE expected text column {column}"),
    }
}

fn blob(row: &[rusqlite::types::Value], column: usize) -> Vec<u8> {
    match &row[column] {
        rusqlite::types::Value::Blob(value) => value.clone(),
        _ => panic!("TEST_CODE expected blob column {column}"),
    }
}

fn integer(row: &[rusqlite::types::Value], column: usize) -> i64 {
    match &row[column] {
        rusqlite::types::Value::Integer(value) => *value,
        _ => panic!("TEST_CODE expected integer column {column}"),
    }
}

fn is_target_row(
    row: &[rusqlite::types::Value],
    intent: &IntentId,
    case: CorruptionCase,
) -> bool {
    if text(row, 0) != intent.as_str() {
        return false;
    }
    match case {
        CorruptionCase::RequestWrapperIdentity => {
            text(row, 12) == "Gateway" && integer(row, 13) == 1 && integer(row, 14) == 1
        }
        CorruptionCase::ControlRawVersion => integer(row, 12) == 1 && integer(row, 13) == 2,
        CorruptionCase::DataRawVersion => {
            text(row, 12) == "Gateway"
                && integer(row, 13) == 1
                && integer(row, 14) == 1
                && integer(row, 15) == 1
        }
        CorruptionCase::SourceNativeContent | CorruptionCase::Br159ReceiptHash => {
            text(row, 12) == "Gateway" && integer(row, 13) == 1
        }
        CorruptionCase::PlanParentOwner => true,
    }
}

fn expected_changed_columns(case: CorruptionCase) -> &'static [usize] {
    match case {
        CorruptionCase::RequestWrapperIdentity
        | CorruptionCase::ControlRawVersion
        | CorruptionCase::DataRawVersion
        | CorruptionCase::PlanParentOwner => &[9, 11],
        CorruptionCase::SourceNativeContent => &[9, 11, 16, 18],
        CorruptionCase::Br159ReceiptHash => &[20],
    }
}

fn prepare_damage(
    before: &DatabaseSnapshot,
    intent: &IntentId,
    case: CorruptionCase,
) -> PreparedDamage {
    let mut expected = before.clone();
    let rows = expected
        .rows
        .get_mut(case.table())
        .expect("TEST_CODE damage target table");
    let targets = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| is_target_row(row, intent, case).then_some(index))
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 1);
    let target = targets[0];
    let original_row = rows[target].clone();
    let row = &mut rows[target];

    let values = match case {
        CorruptionCase::RequestWrapperIdentity => {
            let original = blob(row, 9);
            let value: serde_json::Value = serde_json::from_slice(&original).unwrap();
            let id = value.get("id").and_then(serde_json::Value::as_str).unwrap();
            let replacement_id = "X".repeat(id.len());
            assert_ne!(replacement_id, id);
            let needle = format!("\"id\":\"{id}\"");
            let replacement = format!("\"id\":\"{replacement_id}\"");
            let damaged = replace_once(&original, needle.as_bytes(), replacement.as_bytes());
            let (length, digest) = blob_digest(&damaged);
            row[9] = rusqlite::types::Value::Blob(damaged.clone());
            row[10] = rusqlite::types::Value::Integer(length);
            row[11] = rusqlite::types::Value::Text(digest.clone());
            DamageValues::FactBytes {
                bytes: damaged,
                length,
                digest,
            }
        }
        CorruptionCase::ControlRawVersion => {
            let original = blob(row, 9);
            let damaged = replace_once(&original, br#""version":1"#, br#""version":2"#);
            let (length, digest) = blob_digest(&damaged);
            row[9] = rusqlite::types::Value::Blob(damaged.clone());
            row[10] = rusqlite::types::Value::Integer(length);
            row[11] = rusqlite::types::Value::Text(digest.clone());
            DamageValues::FactBytes {
                bytes: damaged,
                length,
                digest,
            }
        }
        CorruptionCase::DataRawVersion => {
            let original = blob(row, 9);
            let damaged = replace_once(&original, br#""version":2"#, br#""version":3"#);
            let (length, digest) = blob_digest(&damaged);
            row[9] = rusqlite::types::Value::Blob(damaged.clone());
            row[10] = rusqlite::types::Value::Integer(length);
            row[11] = rusqlite::types::Value::Text(digest.clone());
            DamageValues::FactBytes {
                bytes: damaged,
                length,
                digest,
            }
        }
        CorruptionCase::SourceNativeContent => {
            let wrapper = blob(row, 9);
            let native = blob(row, 16);
            let native_digest = text(row, 18).to_owned();
            assert_eq!(raw_digest(&native).as_str(), native_digest);
            let damaged_native = replace_once(
                &native,
                b"TEST_CODE external data title",
                b"TEST_CODE EXTERNAL data title",
            );
            let (native_length, damaged_native_digest) = blob_digest(&damaged_native);
            let damaged_wrapper = replace_once(
                &wrapper,
                native_digest.as_bytes(),
                damaged_native_digest.as_bytes(),
            );
            let (wrapper_length, wrapper_digest) = blob_digest(&damaged_wrapper);
            row[9] = rusqlite::types::Value::Blob(damaged_wrapper.clone());
            row[10] = rusqlite::types::Value::Integer(wrapper_length);
            row[11] = rusqlite::types::Value::Text(wrapper_digest.clone());
            row[16] = rusqlite::types::Value::Blob(damaged_native.clone());
            row[17] = rusqlite::types::Value::Integer(native_length);
            row[18] = rusqlite::types::Value::Text(damaged_native_digest.clone());
            DamageValues::SourceNative {
                wrapper: damaged_wrapper,
                wrapper_length,
                wrapper_digest,
                native: damaged_native,
                native_length,
                native_digest: damaged_native_digest,
            }
        }
        CorruptionCase::Br159ReceiptHash => {
            let original = text(row, 20);
            let damaged = "f".repeat(64);
            assert_ne!(damaged, original);
            row[20] = rusqlite::types::Value::Text(damaged.clone());
            DamageValues::ReceiptHash(damaged)
        }
        CorruptionCase::PlanParentOwner => {
            let original = blob(row, 9);
            let value: serde_json::Value = serde_json::from_slice(&original).unwrap();
            let owner = value
                .get("parent_owner")
                .and_then(serde_json::Value::as_str)
                .unwrap();
            let replacement_owner = "P".repeat(owner.len());
            assert_ne!(replacement_owner, owner);
            let needle = format!("\"parent_owner\":\"{owner}\"");
            let replacement = format!("\"parent_owner\":\"{replacement_owner}\"");
            let damaged = replace_once(&original, needle.as_bytes(), replacement.as_bytes());
            let (length, digest) = blob_digest(&damaged);
            row[9] = rusqlite::types::Value::Blob(damaged.clone());
            row[10] = rusqlite::types::Value::Integer(length);
            row[11] = rusqlite::types::Value::Text(digest.clone());
            DamageValues::FactBytes {
                bytes: damaged,
                length,
                digest,
            }
        }
    };
    let changed_columns = original_row
        .iter()
        .zip(row.iter())
        .enumerate()
        .filter_map(|(column, (before, after))| (before != after).then_some(column))
        .collect::<Vec<_>>();
    assert_eq!(changed_columns.as_slice(), expected_changed_columns(case));
    assert_eq!(changed_tables(before, &expected), vec![case.table()]);
    PreparedDamage { expected, values }
}

fn drop_fixed_trigger(
    transaction: &rusqlite::Transaction<'_>,
    case: CorruptionCase,
) {
    match case {
        CorruptionCase::RequestWrapperIdentity => transaction
            .execute_batch("DROP TRIGGER chain_post_close_macro_request_plans_update")
            .unwrap(),
        CorruptionCase::ControlRawVersion => transaction
            .execute_batch("DROP TRIGGER chain_post_close_macro_control_attempt_results_update")
            .unwrap(),
        CorruptionCase::DataRawVersion => transaction
            .execute_batch("DROP TRIGGER chain_post_close_macro_attempt_results_update")
            .unwrap(),
        CorruptionCase::SourceNativeContent | CorruptionCase::Br159ReceiptHash => transaction
            .execute_batch("DROP TRIGGER chain_post_close_macro_source_finals_update")
            .unwrap(),
        CorruptionCase::PlanParentOwner => transaction
            .execute_batch("DROP TRIGGER chain_post_close_macro_plans_update")
            .unwrap(),
    }
}

fn inject_corruption(
    connection: &mut Connection,
    intent: &IntentId,
    case: CorruptionCase,
    values: &DamageValues,
) {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let trigger_sql = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            [case.trigger()],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    drop_fixed_trigger(&transaction, case);
    let changed = match (case, values) {
        (
            CorruptionCase::RequestWrapperIdentity,
            DamageValues::FactBytes {
                bytes,
                length,
                digest,
            },
        ) => transaction
            .execute(
                "UPDATE chain_post_close_macro_request_plans                  SET bytes=?1,byte_length=?2,sha256=?3                  WHERE intent_id=?4 AND phase='Gateway' AND item_ordinal=1                  AND candidate_ordinal=1",
                params![bytes, length, digest, intent.as_str()],
            )
            .unwrap(),
        (
            CorruptionCase::ControlRawVersion,
            DamageValues::FactBytes {
                bytes,
                length,
                digest,
            },
        ) => transaction
            .execute(
                "UPDATE chain_post_close_macro_control_attempt_results                  SET bytes=?1,byte_length=?2,sha256=?3                  WHERE intent_id=?4 AND episode_ordinal=1 AND control_ordinal=2",
                params![bytes, length, digest, intent.as_str()],
            )
            .unwrap(),
        (
            CorruptionCase::DataRawVersion,
            DamageValues::FactBytes {
                bytes,
                length,
                digest,
            },
        ) => transaction
            .execute(
                "UPDATE chain_post_close_macro_attempt_results                  SET bytes=?1,byte_length=?2,sha256=?3                  WHERE intent_id=?4 AND phase='Gateway' AND item_ordinal=1                  AND candidate_ordinal=1 AND attempt_ordinal=1",
                params![bytes, length, digest, intent.as_str()],
            )
            .unwrap(),
        (
            CorruptionCase::SourceNativeContent,
            DamageValues::SourceNative {
                wrapper,
                wrapper_length,
                wrapper_digest,
                native,
                native_length,
                native_digest,
            },
        ) => transaction
            .execute(
                "UPDATE chain_post_close_macro_source_finals                  SET bytes=?1,byte_length=?2,sha256=?3,native_bytes=?4,                      native_length=?5,native_sha256=?6                  WHERE intent_id=?7 AND phase='Gateway' AND item_ordinal=1",
                params![
                    wrapper,
                    wrapper_length,
                    wrapper_digest,
                    native,
                    native_length,
                    native_digest,
                    intent.as_str()
                ],
            )
            .unwrap(),
        (CorruptionCase::Br159ReceiptHash, DamageValues::ReceiptHash(hash)) => transaction
            .execute(
                "UPDATE chain_post_close_macro_source_finals                  SET audit_record_hash=?1                  WHERE intent_id=?2 AND phase='Gateway' AND item_ordinal=1",
                params![hash, intent.as_str()],
            )
            .unwrap(),
        (
            CorruptionCase::PlanParentOwner,
            DamageValues::FactBytes {
                bytes,
                length,
                digest,
            },
        ) => transaction
            .execute(
                "UPDATE chain_post_close_macro_plans                  SET bytes=?1,byte_length=?2,sha256=?3 WHERE intent_id=?4",
                params![bytes, length, digest, intent.as_str()],
            )
            .unwrap(),
        _ => panic!("TEST_CODE damage values do not match case"),
    };
    assert_eq!(changed, 1);
    transaction.execute_batch(&trigger_sql).unwrap();
    transaction.commit().unwrap();
    assert!(connection.is_autocommit());
}
fn assert_held_schema_rejected(error: &anyhow::Error, intent: &IntentId) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { intent_id })
            if intent_id == intent.as_str()
    ));
    assert_eq!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(&ChainPostCloseError::SchemaRejected)
    );
}

async fn run_corruption_case(case: CorruptionCase) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let confirmed = control_tests::establish_confirmed_external_first_source(
                &mut business,
                &mut parent_server,
                &mut external_server,
                case.run_id(),
            )
            .await;
            assert_eq!(
                business
                    .chain_post_close()
                    .verify_schema()
                    .unwrap()
                    .schema_version(),
                11
            );
            {
                let mut local = business
                    .store
                    .as_mut()
                    .unwrap()
                    .single_user_local_chain_post_close(&confirmed.config)
                    .unwrap();
                let recovery = local.inspect_macro(&confirmed.intent).unwrap();
                assert!(!recovery.has_unconfirmed_effect());
                let source = recovery
                    .global_news(GlobalNewsProvider::Eastmoney)
                    .unwrap();
                assert!(source.is_complete());
                assert!(source.batch().is_some());
                assert!(source.error().is_none());
            }
            let successful = database_snapshot(business.connection());
            let external = external_server.as_ref().unwrap();
            let parent_source = GrpcSource::from_board_loopback_test_client(
                connect_parent_instance(&confirmed.parent_endpoint).await,
            );
            let queries = parent_source.connected_board_queries().await.unwrap();
            let macro_source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let started_at = micros(STARTED_LOCAL);
            let clock = MacroClock {
                now: Cell::new(
                    UtcMicros::try_new(started_at + 11_000_000).unwrap(),
                ),
                observation: DateTime::parse_from_rfc3339(
                    "2026-09-14T15:33:00+08:00",
                )
                .unwrap(),
                observation_calls: Cell::new(0),
            };
            let registered = [
                GeneralWebResearchProvider::SerpApi,
                GeneralWebResearchProvider::Bocha,
                GeneralWebResearchProvider::Tavily,
            ];
            let search_service = macro_search_service(&registered);
            let database = business.database();
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&confirmed.config)
                .unwrap();
            let previous_head = local.inspect_run(&confirmed.intent).unwrap().head_version();
            let lease = local
                .resume_run(
                    &confirmed.intent,
                    macro_lease(
                        "TEST_CODE_MACRO_CORRUPTION_HELD_OWNER",
                        started_at + 11_000_000,
                        started_at + 12_500_000,
                        previous_head,
                    ),
                )
                .unwrap();
            let mut held_io = local
                .macro_preparation_io_v11(
                    lease,
                    &queries,
                    &clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &parent_source,
                    &macro_source,
                    &search_service,
                )
                .unwrap();
            assert_eq!(clock.observation_calls.get(), 0);
            let before_damage = snapshot_path(&database);
            assert_eq!(
                changed_tables(&successful, &before_damage),
                vec!["chain_post_close_runs"]
            );
            let parent_before_damage = parent_server
                .as_ref()
                .unwrap()
                .snapshot_with_tcp_for_test();
            assert!(parent_before_damage.0 > 0);
            let memberships_before_damage =
                parent_server.as_ref().unwrap().membership_snapshot();
            let external_before_damage = external.snapshot();

            let mut injector = Connection::open_with_flags(
                &database,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .unwrap();
            injector.busy_timeout(Duration::ZERO).unwrap();
            let prepared_damage = prepare_damage(&before_damage, &confirmed.intent, case);
            inject_corruption(
                &mut injector,
                &confirmed.intent,
                case,
                &prepared_damage.values,
            );
            let damaged = database_snapshot(&injector);
            assert_eq!(damaged, prepared_damage.expected);
            injector.close().unwrap();

            let held = held_io
                .macro_search_with_budget()
                .await
                .expect("TEST_CODE corruption held Macro budget")
                .expect_err("TEST_CODE corrupted held Macro accepted facts");
            assert_held_schema_rejected(&held, &confirmed.intent);
            assert_eq!(snapshot_path(&database), damaged);
            assert_eq!(external.snapshot(), external_before_damage);
            assert_eq!(
                parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test(),
                parent_before_damage
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                memberships_before_damage
            );
            assert_eq!(clock.observation_calls.get(), 0);
            drop(held_io);
            let inspected = local.inspect_macro(&confirmed.intent);
            assert!(matches!(
                inspected,
                Err(ChainPostCloseError::SchemaRejected)
            ));
            assert_eq!(snapshot_path(&database), damaged);
            assert_eq!(external.snapshot(), external_before_damage);
            assert_eq!(
                parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test(),
                parent_before_damage
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                memberships_before_damage
            );
            drop(local);
            drop(queries);
            drop(parent_source);
            drop(macro_source);

            business.reopen();
            assert!(matches!(
                business
                    .store
                    .as_mut()
                    .unwrap()
                    .single_user_local_chain_post_close(&confirmed.config),
                Err(ChainPostCloseError::SchemaRejected)
            ));
            assert_eq!(database_snapshot(business.connection()), damaged);
            assert_eq!(external.snapshot(), external_before_damage);
            assert_eq!(
                parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test(),
                parent_before_damage
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                memberships_before_damage
            );
        },
    ))
    .catch_unwind()
    .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        case.label(),
    )
    .await;
    drop(business);
    match body {
        Ok(result) => result.expect("TEST_CODE Macro corruption body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum V12FactCorruptionCase {
    QueryCauseLink,
    QueryRecordedOrder,
    QueryNativeBinding,
    QueryAuditLink,
    DimensionChoice,
    DimensionPaceChain,
    FinalizeBeginBytesVersion,
    FinalizeBeginFactsParent,
    StageFinalBytesVersion,
    StageFinalBeginParent,
}

impl V12FactCorruptionCase {
    const ALL: [Self; 10] = [
        Self::QueryCauseLink,
        Self::QueryRecordedOrder,
        Self::QueryNativeBinding,
        Self::QueryAuditLink,
        Self::DimensionChoice,
        Self::DimensionPaceChain,
        Self::FinalizeBeginBytesVersion,
        Self::FinalizeBeginFactsParent,
        Self::StageFinalBytesVersion,
        Self::StageFinalBeginParent,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::QueryCauseLink => "query-cause-link",
            Self::QueryRecordedOrder => "query-recorded-order",
            Self::QueryNativeBinding => "query-native-binding",
            Self::QueryAuditLink => "query-audit-link",
            Self::DimensionChoice => "dimension-choice",
            Self::DimensionPaceChain => "dimension-pace-chain",
            Self::FinalizeBeginBytesVersion => "finalize-begin-bytes-version",
            Self::FinalizeBeginFactsParent => "finalize-begin-facts-parent",
            Self::StageFinalBytesVersion => "stage-final-bytes-version",
            Self::StageFinalBeginParent => "stage-final-begin-parent",
        }
    }

    fn table(self) -> &'static str {
        match self {
            Self::QueryCauseLink
            | Self::QueryRecordedOrder
            | Self::QueryNativeBinding
            | Self::QueryAuditLink => "chain_post_close_macro_query_terminals",
            Self::DimensionChoice | Self::DimensionPaceChain => {
                "chain_post_close_macro_dimension_terminals"
            }
            Self::FinalizeBeginBytesVersion | Self::FinalizeBeginFactsParent => {
                "chain_post_close_macro_finalize_begins"
            }
            Self::StageFinalBytesVersion | Self::StageFinalBeginParent => {
                "chain_post_close_macro_stage_finals"
            }
        }
    }

    fn trigger(self) -> &'static str {
        match self {
            Self::QueryCauseLink
            | Self::QueryRecordedOrder
            | Self::QueryNativeBinding
            | Self::QueryAuditLink => "chain_post_close_macro_query_terminals_update",
            Self::DimensionChoice | Self::DimensionPaceChain => {
                "chain_post_close_macro_dimension_terminals_update"
            }
            Self::FinalizeBeginBytesVersion | Self::FinalizeBeginFactsParent => {
                "chain_post_close_macro_finalize_begins_update"
            }
            Self::StageFinalBytesVersion | Self::StageFinalBeginParent => {
                "chain_post_close_macro_stage_finals_update"
            }
        }
    }

    fn expected_changed_columns(self) -> &'static [usize] {
        match self {
            Self::QueryCauseLink => &[20],
            Self::QueryRecordedOrder => &[8],
            Self::QueryNativeBinding => &[23, 24, 25],
            Self::QueryAuditLink => &[27],
            Self::DimensionChoice => &[9, 11, 17],
            Self::DimensionPaceChain => &[8, 9, 11, 18],
            Self::FinalizeBeginBytesVersion
            | Self::StageFinalBytesVersion
            | Self::StageFinalBeginParent => &[9, 11],
            Self::FinalizeBeginFactsParent => &[9, 11, 17],
        }
    }
}

fn v12_target_row(
    snapshot: &DatabaseSnapshot,
    intent: &IntentId,
    case: V12FactCorruptionCase,
) -> usize {
    let targets = snapshot.rows[case.table()]
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            if text(row, 0) != intent.as_str() {
                return None;
            }
            let selected = match case {
                V12FactCorruptionCase::QueryCauseLink
                | V12FactCorruptionCase::QueryRecordedOrder
                | V12FactCorruptionCase::QueryNativeBinding
                | V12FactCorruptionCase::QueryAuditLink => {
                    text(row, 12) == "Gateway" && integer(row, 13) == 1
                }
                V12FactCorruptionCase::DimensionChoice
                | V12FactCorruptionCase::DimensionPaceChain => integer(row, 12) == 1,
                V12FactCorruptionCase::FinalizeBeginBytesVersion
                | V12FactCorruptionCase::FinalizeBeginFactsParent
                | V12FactCorruptionCase::StageFinalBytesVersion
                | V12FactCorruptionCase::StageFinalBeginParent => true,
            };
            selected.then_some(index)
        })
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 1, "TEST_CODE {} target", case.label());
    targets[0]
}

fn v12_gateway_row<'a>(
    snapshot: &'a DatabaseSnapshot,
    intent: &IntentId,
    ordinal: i64,
) -> &'a [rusqlite::types::Value] {
    let rows = snapshot.rows["chain_post_close_macro_query_terminals"]
        .iter()
        .filter(|row| {
            text(row, 0) == intent.as_str()
                && text(row, 12) == "Gateway"
                && integer(row, 13) == ordinal
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    rows[0]
}

fn replace_v12_fact_bytes(
    row: &mut [rusqlite::types::Value],
    needle: &[u8],
    replacement: &[u8],
) {
    let original = blob(row, 9);
    let damaged = replace_once(&original, needle, replacement);
    let (length, digest) = blob_digest(&damaged);
    row[9] = rusqlite::types::Value::Blob(damaged);
    row[10] = rusqlite::types::Value::Integer(length);
    row[11] = rusqlite::types::Value::Text(digest);
}

fn prepare_v12_fact_damage(
    before: &DatabaseSnapshot,
    intent: &IntentId,
    case: V12FactCorruptionCase,
) -> DatabaseSnapshot {
    let mut expected = before.clone();
    let target = v12_target_row(before, intent, case);
    let original_row = before.rows[case.table()][target].clone();

    match case {
        V12FactCorruptionCase::QueryCauseLink => {
            let donor = integer(v12_gateway_row(before, intent, 2), 20);
            assert_ne!(donor, integer(&original_row, 20));
            expected.rows.get_mut(case.table()).unwrap()[target][20] =
                rusqlite::types::Value::Integer(donor);
        }
        V12FactCorruptionCase::QueryRecordedOrder => {
            let cause_version = integer(&original_row, 20);
            let causes = before.rows["chain_post_close_macro_attempt_results"]
                .iter()
                .filter(|row| {
                    text(row, 0) == intent.as_str() && integer(row, 7) == cause_version
                })
                .collect::<Vec<_>>();
            assert_eq!(causes.len(), 1);
            let cause_time = integer(causes[0], 8);
            assert_eq!(integer(&original_row, 8), cause_time);
            let damaged = cause_time.checked_add(1).unwrap();
            let run = before.rows["chain_post_close_runs"]
                .iter()
                .find(|row| text(row, 0) == intent.as_str())
                .unwrap();
            assert!(damaged <= integer(run, 29));
            expected.rows.get_mut(case.table()).unwrap()[target][8] =
                rusqlite::types::Value::Integer(damaged);
        }
        V12FactCorruptionCase::QueryNativeBinding => {
            let row = &mut expected.rows.get_mut(case.table()).unwrap()[target];
            let mut damaged = blob(row, 23);
            damaged.push(b'!');
            let (length, digest) = blob_digest(&damaged);
            assert_ne!(digest, text(row, 25));
            row[23] = rusqlite::types::Value::Blob(damaged);
            row[24] = rusqlite::types::Value::Integer(length);
            row[25] = rusqlite::types::Value::Text(digest);
        }
        V12FactCorruptionCase::QueryAuditLink => {
            let donor = text(v12_gateway_row(before, intent, 2), 27).to_owned();
            assert_eq!(donor.len(), 64);
            assert_ne!(donor, text(&original_row, 27));
            expected.rows.get_mut(case.table()).unwrap()[target][27] =
                rusqlite::types::Value::Text(donor);
        }
        V12FactCorruptionCase::DimensionChoice => {
            let row = &mut expected.rows.get_mut(case.table()).unwrap()[target];
            assert_eq!(integer(row, 17), 2);
            replace_v12_fact_bytes(row, br#""selected_candidate":2"#, br#""selected_candidate":3"#);
            row[17] = rusqlite::types::Value::Integer(3);
        }
        V12FactCorruptionCase::DimensionPaceChain => {
            let dimension_two = before.rows[case.table()]
                .iter()
                .find(|row| {
                    text(row, 0) == intent.as_str() && integer(row, 12) == 2
                })
                .unwrap();
            let new_due = integer(dimension_two, 8).checked_add(1).unwrap();
            let new_recorded = new_due.checked_sub(300_000).unwrap();
            let last_terminal_version = integer(&original_row, 16);
            let last_terminal = before.rows["chain_post_close_macro_query_terminals"]
                .iter()
                .find(|row| {
                    text(row, 0) == intent.as_str()
                        && integer(row, 7) == last_terminal_version
                })
                .unwrap();
            assert!(new_recorded >= integer(last_terminal, 8));
            assert!(new_recorded < integer(dimension_two, 8));
            let row = &mut expected.rows.get_mut(case.table()).unwrap()[target];
            let old_due = integer(row, 18);
            let needle = format!("\"pace_due\":{old_due}");
            let replacement = format!("\"pace_due\":{new_due}");
            assert_eq!(needle.len(), replacement.len());
            replace_v12_fact_bytes(row, needle.as_bytes(), replacement.as_bytes());
            row[8] = rusqlite::types::Value::Integer(new_recorded);
            row[18] = rusqlite::types::Value::Integer(new_due);
        }
        V12FactCorruptionCase::FinalizeBeginBytesVersion => {
            replace_v12_fact_bytes(
                &mut expected.rows.get_mut(case.table()).unwrap()[target],
                br#""version":3"#,
                br#""version":4"#,
            );
        }
        V12FactCorruptionCase::FinalizeBeginFactsParent => {
            let row = &mut expected.rows.get_mut(case.table()).unwrap()[target];
            let original = text(row, 17).to_owned();
            let damaged = raw_digest(b"TEST_CODE_WRONG_FACT_SET").as_str().to_owned();
            assert_ne!(damaged, original);
            replace_v12_fact_bytes(row, original.as_bytes(), damaged.as_bytes());
            row[17] = rusqlite::types::Value::Text(damaged);
        }
        V12FactCorruptionCase::StageFinalBytesVersion => {
            replace_v12_fact_bytes(
                &mut expected.rows.get_mut(case.table()).unwrap()[target],
                br#""version":2"#,
                br#""version":3"#,
            );
        }
        V12FactCorruptionCase::StageFinalBeginParent => {
            let begin = before.rows["chain_post_close_macro_finalize_begins"]
                .iter()
                .find(|row| text(row, 0) == intent.as_str())
                .unwrap();
            let original = raw_digest(&blob(begin, 9)).as_str().to_owned();
            let damaged = raw_digest(b"TEST_CODE_WRONG_FINALIZE_BEGIN")
                .as_str()
                .to_owned();
            assert_ne!(damaged, original);
            replace_v12_fact_bytes(
                &mut expected.rows.get_mut(case.table()).unwrap()[target],
                original.as_bytes(),
                damaged.as_bytes(),
            );
        }
    }

    let row = &expected.rows[case.table()][target];
    let changed_columns = original_row
        .iter()
        .zip(row.iter())
        .enumerate()
        .filter_map(|(column, (before, after))| (before != after).then_some(column))
        .collect::<Vec<_>>();
    assert_eq!(
        changed_columns.as_slice(),
        case.expected_changed_columns(),
        "TEST_CODE {} exact changed columns",
        case.label()
    );
    assert_eq!(changed_tables(before, &expected), vec![case.table()]);
    if matches!(case, V12FactCorruptionCase::QueryNativeBinding) {
        assert_eq!(integer(row, 24), i64::try_from(blob(row, 23).len()).unwrap());
        assert_eq!(text(row, 25), raw_digest(&blob(row, 23)).as_str());
    } else if changed_columns.contains(&9) {
        assert_eq!(integer(row, 10), i64::try_from(blob(row, 9).len()).unwrap());
        assert_eq!(text(row, 11), raw_digest(&blob(row, 9)).as_str());
    }
    expected
}

fn drop_v12_fact_trigger(
    transaction: &rusqlite::Transaction<'_>,
    case: V12FactCorruptionCase,
) {
    match case {
        V12FactCorruptionCase::QueryCauseLink
        | V12FactCorruptionCase::QueryRecordedOrder
        | V12FactCorruptionCase::QueryNativeBinding
        | V12FactCorruptionCase::QueryAuditLink => transaction
            .execute_batch("DROP TRIGGER chain_post_close_macro_query_terminals_update")
            .unwrap(),
        V12FactCorruptionCase::DimensionChoice
        | V12FactCorruptionCase::DimensionPaceChain => transaction
            .execute_batch("DROP TRIGGER chain_post_close_macro_dimension_terminals_update")
            .unwrap(),
        V12FactCorruptionCase::FinalizeBeginBytesVersion
        | V12FactCorruptionCase::FinalizeBeginFactsParent => transaction
            .execute_batch("DROP TRIGGER chain_post_close_macro_finalize_begins_update")
            .unwrap(),
        V12FactCorruptionCase::StageFinalBytesVersion
        | V12FactCorruptionCase::StageFinalBeginParent => transaction
            .execute_batch("DROP TRIGGER chain_post_close_macro_stage_finals_update")
            .unwrap(),
    }
}

fn inject_v12_fact_damage(
    connection: &mut Connection,
    intent: &IntentId,
    case: V12FactCorruptionCase,
    expected: &DatabaseSnapshot,
) {
    let target = v12_target_row(expected, intent, case);
    let row = &expected.rows[case.table()][target];
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let trigger_sql = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            [case.trigger()],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    drop_v12_fact_trigger(&transaction, case);
    let changed = match case {
        V12FactCorruptionCase::QueryCauseLink => transaction
            .execute(
                "UPDATE chain_post_close_macro_query_terminals SET data_result_version=?1 \
                 WHERE intent_id=?2 AND phase='Gateway' AND item_ordinal=1",
                params![integer(row, 20), intent.as_str()],
            )
            .unwrap(),
        V12FactCorruptionCase::QueryRecordedOrder => transaction
            .execute(
                "UPDATE chain_post_close_macro_query_terminals SET recorded_at=?1 \
                 WHERE intent_id=?2 AND phase='Gateway' AND item_ordinal=1",
                params![integer(row, 8), intent.as_str()],
            )
            .unwrap(),
        V12FactCorruptionCase::QueryNativeBinding => transaction
            .execute(
                "UPDATE chain_post_close_macro_query_terminals \
                 SET native_bytes=?1,native_length=?2,native_sha256=?3 \
                 WHERE intent_id=?4 AND phase='Gateway' AND item_ordinal=1",
                params![blob(row, 23), integer(row, 24), text(row, 25), intent.as_str()],
            )
            .unwrap(),
        V12FactCorruptionCase::QueryAuditLink => transaction
            .execute(
                "UPDATE chain_post_close_macro_query_terminals SET audit_record_hash=?1 \
                 WHERE intent_id=?2 AND phase='Gateway' AND item_ordinal=1",
                params![text(row, 27), intent.as_str()],
            )
            .unwrap(),
        V12FactCorruptionCase::DimensionChoice => transaction
            .execute(
                "UPDATE chain_post_close_macro_dimension_terminals \
                 SET bytes=?1,byte_length=?2,sha256=?3,selected_candidate_ordinal=?4 \
                 WHERE intent_id=?5 AND dimension=1",
                params![blob(row, 9), integer(row, 10), text(row, 11), integer(row, 17), intent.as_str()],
            )
            .unwrap(),
        V12FactCorruptionCase::DimensionPaceChain => transaction
            .execute(
                "UPDATE chain_post_close_macro_dimension_terminals \
                 SET recorded_at=?1,bytes=?2,byte_length=?3,sha256=?4,pace_due=?5 \
                 WHERE intent_id=?6 AND dimension=1",
                params![integer(row, 8), blob(row, 9), integer(row, 10), text(row, 11), integer(row, 18), intent.as_str()],
            )
            .unwrap(),
        V12FactCorruptionCase::FinalizeBeginBytesVersion => transaction
            .execute(
                "UPDATE chain_post_close_macro_finalize_begins \
                 SET bytes=?1,byte_length=?2,sha256=?3 WHERE intent_id=?4",
                params![blob(row, 9), integer(row, 10), text(row, 11), intent.as_str()],
            )
            .unwrap(),
        V12FactCorruptionCase::FinalizeBeginFactsParent => transaction
            .execute(
                "UPDATE chain_post_close_macro_finalize_begins \
                 SET bytes=?1,byte_length=?2,sha256=?3,facts_sha256=?4 WHERE intent_id=?5",
                params![blob(row, 9), integer(row, 10), text(row, 11), text(row, 17), intent.as_str()],
            )
            .unwrap(),
        V12FactCorruptionCase::StageFinalBytesVersion
        | V12FactCorruptionCase::StageFinalBeginParent => transaction
            .execute(
                "UPDATE chain_post_close_macro_stage_finals \
                 SET bytes=?1,byte_length=?2,sha256=?3 WHERE intent_id=?4",
                params![blob(row, 9), integer(row, 10), text(row, 11), intent.as_str()],
            )
            .unwrap(),
    };
    assert_eq!(changed, 1, "TEST_CODE {} injected row", case.label());
    transaction.execute_batch(&trigger_sql).unwrap();
    transaction.commit().unwrap();
    assert!(connection.is_autocommit());
}

fn assert_v12_full_positive(
    snapshot: &DatabaseSnapshot,
    recovery: &crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery,
    intent: &IntentId,
    expected_macro: &[u8],
) {
    let query_rows = snapshot.rows["chain_post_close_macro_query_terminals"]
        .iter()
        .filter(|row| text(row, 0) == intent.as_str())
        .collect::<Vec<_>>();
    assert_eq!(query_rows.len(), 18);
    assert_eq!(
        query_rows.iter().filter(|row| text(row, 12) == "Gateway").count(),
        5
    );
    assert_eq!(
        query_rows
            .iter()
            .filter(|row| text(row, 12) == "WebDimension")
            .count(),
        13
    );
    assert!(query_rows.iter().all(|row| {
        text(row, 19) == "DataResult"
            && text(row, 22) == "Returned"
            && integer(row, 24) == i64::try_from(blob(row, 23).len()).unwrap()
            && text(row, 25) == raw_digest(&blob(row, 23)).as_str()
    }));

    let mut audit_ids = BTreeSet::new();
    for ordinal in 1..=5 {
        let row = v12_gateway_row(snapshot, intent, ordinal);
        let rusqlite::types::Value::Integer(audit_id) = &row[26] else {
            panic!("TEST_CODE Gateway {ordinal} missing audit id");
        };
        assert!(audit_ids.insert(*audit_id));
        assert_eq!(text(row, 27).len(), 64);
        let terminal = recovery
            .query_terminal(QueryKey::Gateway(u8::try_from(ordinal).unwrap()))
            .unwrap();
        assert!(terminal.was_called());
        assert!(terminal.audit_receipt().is_some());
        let native = blob(row, 23);
        assert_eq!(terminal.native_bytes(), native.as_slice());
    }
    for dimension in 1..=6 {
        let last = if dimension < 6 { 2 } else { 3 };
        for candidate in 1..=last {
            let terminal = recovery
                .query_terminal(QueryKey::Web {
                    dimension,
                    candidate,
                })
                .unwrap();
            assert!(terminal.was_called());
            assert!(terminal.audit_receipt().is_none());
        }
    }

    let mut dimensions = snapshot.rows["chain_post_close_macro_dimension_terminals"]
        .iter()
        .filter(|row| text(row, 0) == intent.as_str())
        .collect::<Vec<_>>();
    dimensions.sort_by_key(|row| integer(row, 12));
    assert_eq!(dimensions.len(), 6);
    for (index, row) in dimensions.iter().enumerate() {
        let dimension = i64::try_from(index + 1).unwrap();
        assert_eq!(integer(row, 12), dimension);
        assert_eq!(integer(row, 18) - integer(row, 8), 300_000);
        if dimension < 6 {
            assert_eq!(text(row, 15), "SelectedResearchOnly");
            assert_eq!(integer(row, 17), 2);
        } else {
            assert_eq!(text(row, 15), "Exhausted");
            assert_eq!(row[17], rusqlite::types::Value::Null);
        }
        if index > 0 {
            assert!(integer(row, 8) >= integer(dimensions[index - 1], 18));
        }
    }

    assert!(recovery.is_complete());
    assert_eq!(recovery.attempts().len(), 18);
    assert!(!recovery.has_unconfirmed_effect());
    assert!(recovery.pending_source_identities().is_empty());
    assert!(recovery.pending_research_queries().is_empty());
    let begin = recovery.finalize_begin().unwrap();
    let final_ = recovery.stage_final().unwrap();
    assert_eq!(begin.kind(), final_.kind());
    assert_eq!(begin.bytes(), final_.finalize_begin_bytes());
    assert!(begin.pending().is_empty());
    assert_eq!(final_.version(), begin.version() + 1);
    assert_eq!(final_.output_bytes(), expected_macro);
    assert_eq!(final_.deadline_at() - final_.started_at(), 15_000_000);
    assert_eq!(final_.owner(), begin.owner());
    assert_eq!(final_.generation(), begin.generation());
}

fn sidecar(database: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(suffix);
    path.into()
}

pub(super) async fn assert_v12_full_fact_corruption_matrix(
    seed_database: &std::path::Path,
    config: &crate::monitor::push_job::LocalChainPostCloseConfig,
    intent: &IntentId,
    macro_server: &MacroFullLoopbackServer,
    parent_server: &BoardLoopbackServer,
    expected_macro: &[u8],
) {
    for suffix in ["-journal", "-wal", "-shm"] {
        assert!(
            !sidecar(seed_database, suffix).exists(),
            "TEST_CODE FullSuccess seed has live SQLite sidecar {suffix}"
        );
    }
    let clean = snapshot_path(seed_database);
    let macro_network = macro_server.snapshot();
    let parent_network = parent_server.snapshot_with_tcp_for_test();
    let memberships = parent_server.membership_snapshot();

    for case in V12FactCorruptionCase::ALL {
        tokio::task::yield_now().await;
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join(format!("{}.sqlite", case.label()));
        assert_eq!(
            std::fs::copy(seed_database, &database).unwrap(),
            std::fs::metadata(seed_database).unwrap().len(),
            "TEST_CODE {} copies the clean committed seed",
            case.label()
        );
        let copied = snapshot_path(&database);
        assert_eq!(copied, clean, "TEST_CODE {} clean copy", case.label());

        let mut store = BusinessIntentStore::open(&database).unwrap();
        let mut local = store.single_user_local_chain_post_close(config).unwrap();
        {
            let recovery = local.inspect_macro(intent).unwrap();
            assert_v12_full_positive(&copied, &recovery, intent, expected_macro);
        }
        assert_eq!(macro_server.snapshot(), macro_network, "TEST_CODE {} positive Macro RPC", case.label());
        assert_eq!(parent_server.snapshot_with_tcp_for_test(), parent_network, "TEST_CODE {} positive parent RPC", case.label());
        assert_eq!(parent_server.membership_snapshot(), memberships, "TEST_CODE {} positive membership", case.label());

        let mut injector = Connection::open_with_flags(
            &database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        injector.busy_timeout(Duration::from_millis(250)).unwrap();
        let expected = prepare_v12_fact_damage(&copied, intent, case);
        inject_v12_fact_damage(&mut injector, intent, case, &expected);
        let damaged = database_snapshot(&injector);
        assert_eq!(damaged, expected, "TEST_CODE {} exact damaged snapshot", case.label());
        assert_eq!(macro_server.snapshot(), macro_network, "TEST_CODE {} injection Macro RPC", case.label());
        assert_eq!(parent_server.snapshot_with_tcp_for_test(), parent_network, "TEST_CODE {} injection parent RPC", case.label());
        assert_eq!(parent_server.membership_snapshot(), memberships, "TEST_CODE {} injection membership", case.label());

        assert!(matches!(
            local.inspect_macro(intent),
            Err(ChainPostCloseError::SchemaRejected)
        ));
        assert_eq!(database_snapshot(&injector), damaged, "TEST_CODE {} held reader repair", case.label());
        assert_eq!(macro_server.snapshot(), macro_network, "TEST_CODE {} held Macro RPC", case.label());
        assert_eq!(parent_server.snapshot_with_tcp_for_test(), parent_network, "TEST_CODE {} held parent RPC", case.label());
        assert_eq!(parent_server.membership_snapshot(), memberships, "TEST_CODE {} held membership", case.label());

        drop(local);
        assert!(store.connection.is_autocommit());
        store.connection.close().unwrap();
        injector.close().unwrap();
        let mut reopened = BusinessIntentStore::open(&database).unwrap();
        assert!(matches!(
            reopened.single_user_local_chain_post_close(config),
            Err(ChainPostCloseError::SchemaRejected)
        ));
        assert!(reopened.connection.is_autocommit());
        assert_eq!(snapshot_path(&database), damaged, "TEST_CODE {} fresh bind repair", case.label());
        assert_eq!(macro_server.snapshot(), macro_network, "TEST_CODE {} fresh Macro RPC", case.label());
        assert_eq!(parent_server.snapshot_with_tcp_for_test(), parent_network, "TEST_CODE {} fresh parent RPC", case.label());
        assert_eq!(parent_server.membership_snapshot(), memberships, "TEST_CODE {} fresh membership", case.label());
        reopened.connection.close().unwrap();
        for suffix in ["-journal", "-wal", "-shm"] {
            assert!(!sidecar(&database, suffix).exists(), "TEST_CODE {} leaves SQLite sidecar {suffix}", case.label());
        }
        drop(directory);
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn single_user_external_macro_corrupted_request_wrapper_identity_is_rejected_everywhere() {
    run_corruption_case(CorruptionCase::RequestWrapperIdentity).await;
}

#[tokio::test]
async fn single_user_external_macro_corrupted_control_raw_version_is_rejected_everywhere() {
    run_corruption_case(CorruptionCase::ControlRawVersion).await;
}

#[tokio::test]
async fn single_user_external_macro_corrupted_data_raw_version_is_rejected_everywhere() {
    run_corruption_case(CorruptionCase::DataRawVersion).await;
}

#[tokio::test]
async fn single_user_external_macro_corrupted_source_native_content_is_rejected_everywhere() {
    run_corruption_case(CorruptionCase::SourceNativeContent).await;
}

#[tokio::test]
async fn single_user_external_macro_corrupted_br159_receipt_hash_is_rejected_everywhere() {
    run_corruption_case(CorruptionCase::Br159ReceiptHash).await;
}

#[tokio::test]
async fn single_user_external_macro_corrupted_plan_parent_owner_is_rejected_everywhere() {
    run_corruption_case(CorruptionCase::PlanParentOwner).await;
}
