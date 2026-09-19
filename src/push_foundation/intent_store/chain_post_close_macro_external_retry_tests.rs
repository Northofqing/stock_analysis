use super::*;
use crate::grpc_client::client::external_control_attempt::ExternalControlKind;
use crate::grpc_client::client::external_control_loopback_fixture::{
    ExternalControlObservation, ExternalMtlsMacroFixture, ObservedHealthTrailer,
};
use crate::grpc_client::client::macro_attempt::{
    ExternalMacroAttemptCompletion, MacroContinuation, MacroQueryIdentity,
};
use crate::grpc_client::client::ContractProfile;
use crate::grpc_client::external_pb::magic::market::v1::{
    AdmissionState, CanonicalPayload, CapabilitiesResponse, Capability, ErrorDetail, Operation,
    ProviderAttemptDetail, QueryResponse,
};
use crate::grpc_client::provider_attempts::ProviderAttempts;
use crate::grpc_client::retry::RetryDecision;
use crate::pipeline::chain_analysis::preparation::ChainPreparationIo;
use crate::push_foundation::intent_store::chain_post_close::macro_stage::{
    MacroControlOutcome, MacroRecoveredTrailer, MacroRecoveredWire,
};
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use std::collections::BTreeMap;

const AUTHORITY: &str = "grpc-mtls:macro.test.invalid";
const STARTED_LOCAL: &str = "2026-09-14T15:31:00+08:00";
const BATCH_ID: &str = "TEST_CODE_EXTERNAL_DATA_BATCH";
const OBSERVED_AT: &str = "2026-09-14T15:31:00+08:00";
const SOURCE_AT: &str = "2026-09-14 15:30";
const REQUEST_HASH: &str = "fb86badeeebfca14c04928026fe295416a2a47c6534250291c066f3a74b67b9c";
const RECORD_DATA: &[u8] = br#"{"item_id":"TEST_CODE_EXTERNAL_NEWS_001","title":"TEST_CODE external data title","summary":"TEST_CODE external data summary","content":"TEST_CODE external data content","publisher":"TEST_CODE Eastmoney publisher","url":"https://example.com/TEST_CODE_EXTERNAL_NEWS_001","published_at":"2026-09-14T15:30:00+08:00","instruments":[{"exchange":"Shanghai","code":"TEST_CODE_600001","asset_class":"Equity"}],"topics":["TEST_CODE_external_topic"],"language":"zh-CN","evidence":{"provider":"Eastmoney","source_at":"2026-09-14 15:30","observed_at":"2026-09-14T15:31:00+08:00","batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH"}}"#;
const EXPECTED_NATIVE: &[u8] = br#"{"evidence":{"batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH","observed_at":"2026-09-14T15:31:00+08:00","provider":"Eastmoney","source":"eastmoney-web","source_at":"2026-09-14 15:30"},"kind":"Available","records":[{"canonical_url":"https://example.com/TEST_CODE_EXTERNAL_NEWS_001","content":"TEST_CODE external data content","evidence":{"batch_id":"TEST_CODE_EXTERNAL_DATA_BATCH","observed_at":"2026-09-14T15:31:00+08:00","provider":"Eastmoney","source_at":"2026-09-14 15:30"},"instruments":["TEST_CODE_600001"],"item_id":"TEST_CODE_EXTERNAL_NEWS_001","language":"zh-CN","observed_at":"2026-09-14T07:31:00+00:00","published_at":"2026-09-14T07:30:00+00:00","publisher":"TEST_CODE Eastmoney publisher","summary":"TEST_CODE external data summary","title":"TEST_CODE external data title","topics":["TEST_CODE_external_topic"]}],"version":1}"#;

fn expected_capabilities(request_id: &str) -> Vec<u8> {
    CapabilitiesResponse {
        request_id: request_id.to_owned(),
        capabilities: vec![
            Capability {
                operation: Operation::GlobalNews as i32,
                repository_admission: AdmissionState::Admitted as i32,
                runtime_available: true,
                provider: "Eastmoney".to_owned(),
                exact_scope: "TEST_CODE_GLOBAL_NEWS_EASTMONEY_20".to_owned(),
                blocker: String::new(),
                diagnostic_available: true,
            },
            Capability {
                operation: Operation::SemanticSearch as i32,
                repository_admission: AdmissionState::Unadmitted as i32,
                runtime_available: false,
                provider: "Bocha".to_owned(),
                exact_scope: "TEST_CODE_UNDELIVERED_SEMANTIC_SEARCH".to_owned(),
                blocker: "TEST_CODE_CAPABILITY_BLOCKED".to_owned(),
                diagnostic_available: false,
            },
        ],
    }
    .encode_to_vec()
}

fn expected_data_response(request_id: &str) -> Vec<u8> {
    let response = QueryResponse {
        request_id: request_id.to_owned(),
        operation: Operation::GlobalNews as i32,
        admission: AdmissionState::Admitted as i32,
        selected_provider: "Eastmoney".to_owned(),
        batch_id: BATCH_ID.to_owned(),
        complete: true,
        observed_at: OBSERVED_AT.to_owned(),
        source_at: SOURCE_AT.to_owned(),
        records: vec![CanonicalPayload {
            schema: "magic.market.news_item".to_owned(),
            schema_version: 2,
            content_type: "application/json; charset=utf-8".to_owned(),
            data: RECORD_DATA.to_vec(),
        }],
        diagnostic_blocker: String::new(),
    };
    let mut bytes = response.encode_to_vec();
    bytes.extend_from_slice(&[0x5a, 0x00]);
    bytes
}

fn data_result_bytes(
    database: &std::path::Path,
    intent: &IntentId,
    ordinal: i64,
) -> Vec<u8> {
    let reader = BusinessIntentStore::open(database).unwrap();
    let bytes = reader
        .connection
        .query_row(
            "SELECT bytes FROM chain_post_close_macro_attempt_results \
             WHERE intent_id=?1 AND attempt_ordinal=?2",
            rusqlite::params![intent.as_str(), ordinal],
            |row| row.get(0),
        )
        .unwrap();
    reader.connection.close().unwrap();
    bytes
}

fn data_result_count(database: &std::path::Path, intent: &IntentId) -> i64 {
    let reader = BusinessIntentStore::open(database).unwrap();
    let count = reader
        .connection
        .query_row(
            "SELECT COUNT(*) FROM chain_post_close_macro_attempt_results WHERE intent_id=?1",
            [intent.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    reader.connection.close().unwrap();
    count
}

fn fact_snapshot_at(
    database: &std::path::Path,
    tables: &[String],
) -> BTreeMap<String, Vec<Vec<rusqlite::types::Value>>> {
    let reader = BusinessIntentStore::open(database).unwrap();
    let facts = old_fact_rows(&reader.connection, tables);
    reader.connection.close().unwrap();
    facts
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RealV2Tamper {
    OuterFactDigest,
    MaterialTag,
    Profile,
    Method,
    Descriptor,
    PayloadHash,
    PayloadBinding,
    DecodeLimit,
    FailureMaterial,
}

impl RealV2Tamper {
    fn label(self) -> &'static str {
        match self {
            Self::OuterFactDigest => "outer fact digest",
            Self::MaterialTag => "material tag",
            Self::Profile => "profile",
            Self::Method => "method",
            Self::Descriptor => "descriptor",
            Self::PayloadHash => "payload hash",
            Self::PayloadBinding => "payload binding",
            Self::DecodeLimit => "decode limit",
            Self::FailureMaterial => "failure material",
        }
    }

    fn is_outer_only(self) -> bool {
        self == Self::OuterFactDigest
    }
}

#[derive(Debug, Eq, PartialEq)]
struct StoredDataResult {
    bytes: Vec<u8>,
    byte_length: i64,
    sha256: String,
}

#[derive(Debug, Eq, PartialEq)]
struct DurableAttemptLinks {
    plan_bytes: Vec<u8>,
    begins: Vec<(i64, i64, String, Option<i64>, Option<i64>)>,
    results: Vec<(i64, i64, i64, String, String, Option<i64>)>,
}

fn durable_attempt_links(
    connection: &rusqlite::Connection,
    intent: &IntentId,
) -> DurableAttemptLinks {
    let plan_bytes = connection
        .query_row(
            "SELECT bytes FROM chain_post_close_macro_plans WHERE intent_id=?1",
            [intent.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    let mut begins_statement = connection
        .prepare(
            "SELECT attempt_ordinal,run_version,request_sha256,\
                    readiness_result_version,previous_result_version \
             FROM chain_post_close_macro_attempt_begins \
             WHERE intent_id=?1 ORDER BY attempt_ordinal",
        )
        .unwrap();
    let begins = begins_statement
        .query_map([intent.as_str()], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    drop(begins_statement);
    let mut results_statement = connection
        .prepare(
            "SELECT attempt_ordinal,run_version,begin_version,request_sha256,\
                    continuation,retry_not_before \
             FROM chain_post_close_macro_attempt_results \
             WHERE intent_id=?1 ORDER BY attempt_ordinal",
        )
        .unwrap();
    let results = results_statement
        .query_map([intent.as_str()], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    DurableAttemptLinks {
        plan_bytes,
        begins,
        results,
    }
}

fn stored_data_result(
    connection: &rusqlite::Connection,
    intent: &IntentId,
) -> StoredDataResult {
    connection
        .query_row(
            "SELECT bytes,byte_length,sha256 \
             FROM chain_post_close_macro_attempt_results \
             WHERE intent_id=?1 AND attempt_ordinal=1",
            [intent.as_str()],
            |row| {
                Ok(StoredDataResult {
                    bytes: row.get(0)?,
                    byte_length: row.get(1)?,
                    sha256: row.get(2)?,
                })
            },
        )
        .unwrap()
}

fn replace_once(bytes: &[u8], from: &str, to: &str, label: &str) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).unwrap();
    assert_eq!(
        text.matches(from).count(),
        1,
        "TEST_CODE {label} mutation source must be unique"
    );
    text.replacen(from, to, 1).into_bytes()
}

fn mutate_payload_binding(bytes: &[u8]) -> Vec<u8> {
    let marker = b"\"protobuf_payload\":[";
    let start = bytes
        .windows(marker.len())
        .position(|candidate| candidate == marker)
        .expect("TEST_CODE V2 protobuf payload marker")
        + marker.len();
    let end = start
        + bytes[start..]
            .iter()
            .position(|byte| *byte == b']')
            .expect("TEST_CODE V2 protobuf payload end");
    let mut mutated = bytes.to_vec();
    mutated.splice(end..end, [b',', b'0']);
    let value: serde_json::Value = serde_json::from_slice(&mutated).unwrap();
    let payload = value["external_wire"]["evidence"]["Payload"]["protobuf_payload"]
        .as_array()
        .unwrap()
        .iter()
        .map(|byte| u8::try_from(byte.as_u64().unwrap()).unwrap())
        .collect::<Vec<_>>();
    let old_hash = value["external_wire"]["evidence"]["Payload"]["payload_sha256"]
        .as_str()
        .unwrap();
    let new_hash = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&payload));
    replace_once(
        &mutated,
        &format!("\"payload_sha256\":\"{old_hash}\""),
        &format!("\"payload_sha256\":\"{new_hash}\""),
        "payload digest rebinding",
    )
}

fn mutate_failure_material(bytes: &[u8]) -> Vec<u8> {
    let response = b"\"response\":";
    let response_start = bytes
        .windows(response.len())
        .position(|candidate| candidate == response)
        .expect("TEST_CODE V2 response marker")
        + response.len();
    assert_eq!(bytes[response_start], b'[');
    let response_end = response_start
        + bytes[response_start..]
            .iter()
            .position(|byte| *byte == b']')
            .expect("TEST_CODE V2 response end")
        + 1;
    let mut mutated = bytes.to_vec();
    mutated.splice(response_start..response_end, *b"null");
    mutated = replace_once(
        &mutated,
        "\"diagnostic\":null",
        "\"diagnostic\":\"external_response_wire_invalid\"",
        "failure diagnostic",
    );
    let evidence = b"\"evidence\":";
    let evidence_start = mutated
        .windows(evidence.len())
        .rposition(|candidate| candidate == evidence)
        .expect("TEST_CODE V2 external evidence marker")
        + evidence.len();
    assert!(mutated.ends_with(b"}}}}"));
    let evidence_end = mutated.len() - 2;
    mutated.splice(
        evidence_start..evidence_end,
        *b"{\"Missing\":{\"framed_body_limit_bytes\":4194308}}",
    );
    mutated
}

fn mutated_v2_bytes(bytes: &[u8], case: RealV2Tamper) -> Vec<u8> {
    match case {
        RealV2Tamper::OuterFactDigest => replace_once(
            bytes,
            "\"profile\":\"ExternalV1\"",
            "\"profile\":\"ExternalW1\"",
            case.label(),
        ),
        RealV2Tamper::MaterialTag => replace_once(
            bytes,
            "external-unary-response-evidence-v1",
            "external-unary-response-evidence-v0",
            case.label(),
        ),
        RealV2Tamper::Profile => replace_once(
            bytes,
            "\"profile\":\"ExternalV1\"",
            "\"profile\":\"LocalBridgeV1\"",
            case.label(),
        ),
        RealV2Tamper::Method => replace_once(
            bytes,
            "\"method\":\"OPERATION_GLOBAL_NEWS\"",
            "\"method\":\"OPERATION_SECURITY_METADATA\"",
            case.label(),
        ),
        RealV2Tamper::Descriptor => replace_once(
            bytes,
            crate::grpc_client::external_query_transport::EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
            "0ba0fa3b2fa450e74bdcc8cb5f163348a6ca90df3f3626d1d8f2ec27137f5edb",
            case.label(),
        ),
        RealV2Tamper::PayloadHash => {
            let value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            let hash = value["external_wire"]["evidence"]["Payload"]["payload_sha256"]
                .as_str()
                .unwrap();
            replace_once(bytes, hash, &"0".repeat(64), case.label())
        }
        RealV2Tamper::PayloadBinding => mutate_payload_binding(bytes),
        RealV2Tamper::DecodeLimit => replace_once(
            bytes,
            "\"decode_limit_bytes\":4194304",
            "\"decode_limit_bytes\":4194303",
            case.label(),
        ),
        RealV2Tamper::FailureMaterial => mutate_failure_material(bytes),
    }
}

fn mutate_copied_v2_result(
    database: &std::path::Path,
    intent: &IntentId,
    case: RealV2Tamper,
) -> (StoredDataResult, StoredDataResult) {
    let connection = rusqlite::Connection::open(database).unwrap();
    let before = stored_data_result(&connection, intent);
    let bytes = mutated_v2_bytes(&before.bytes, case);
    assert_ne!(bytes, before.bytes, "TEST_CODE {} mutation", case.label());
    let transaction = connection.unchecked_transaction().unwrap();
    let trigger_sql: String = transaction
        .query_row(
            "SELECT sql FROM sqlite_schema \
             WHERE type='trigger' AND name='chain_post_close_macro_attempt_results_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    transaction
        .execute_batch("DROP TRIGGER chain_post_close_macro_attempt_results_update")
        .unwrap();
    let (byte_length, sha256) = if case.is_outer_only() {
        (before.byte_length, before.sha256.clone())
    } else {
        (
            i64::try_from(bytes.len()).unwrap(),
            raw_digest(&bytes).as_str().to_owned(),
        )
    };
    assert_eq!(
        transaction
            .execute(
                "UPDATE chain_post_close_macro_attempt_results \
                 SET bytes=?1,byte_length=?2,sha256=?3 \
                 WHERE intent_id=?4 AND attempt_ordinal=1",
                rusqlite::params![bytes, byte_length, sha256, intent.as_str()],
            )
            .unwrap(),
        1
    );
    transaction.execute_batch(&trigger_sql).unwrap();
    transaction.commit().unwrap();
    let after = stored_data_result(&connection, intent);
    connection.close().unwrap();
    if case.is_outer_only() {
        assert_ne!(raw_digest(&after.bytes).as_str(), after.sha256);
    } else {
        assert_eq!(after.byte_length, i64::try_from(after.bytes.len()).unwrap());
        assert_eq!(after.sha256, raw_digest(&after.bytes).as_str());
    }
    (before, after)
}

fn assert_controls(
    recovery: &crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery,
    checkpoint: &control_recovery_tests::ConfirmedHealthCheckpoint,
    capabilities_response: &[u8],
    ready_result_version: u64,
) {
    assert_eq!(recovery.readiness_episodes().len(), 1);
    let episode = &recovery.readiness_episodes()[0];
    assert_eq!(episode.episode_ordinal(), 1);
    assert_eq!(episode.ready_result_version(), Some(ready_result_version));
    assert_eq!(
        episode.initiating_source(),
        &MacroQueryIdentity::GlobalNews {
            provider: GlobalNewsProvider::Eastmoney,
            limit: 20,
        }
    );
    let controls = episode.controls();
    assert_eq!(controls.len(), 2);
    assert_eq!(controls[0].kind(), ExternalControlKind::Health);
    assert_eq!(controls[0].begin_version(), Some(checkpoint.health.begin_version));
    assert_eq!(controls[0].result_version(), Some(checkpoint.health.result_version));
    assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Ready));
    assert_eq!(controls[0].request_id(), checkpoint.health.request.id);
    assert_eq!(controls[0].request_bytes(), checkpoint.health.request.bytes);
    assert_eq!(
        controls[0].response_bytes(),
        Some(checkpoint.health.response_bytes.as_slice())
    );
    assert_eq!(controls[1].kind(), ExternalControlKind::Capabilities);
    assert!(controls[1].begin_version().unwrap() > checkpoint.health.result_version);
    assert_eq!(controls[1].result_version(), Some(ready_result_version));
    assert_eq!(controls[1].outcome(), Some(MacroControlOutcome::Ready));
    assert_eq!(controls[1].request_id(), checkpoint.capabilities.id);
    assert_eq!(controls[1].request_bytes(), checkpoint.capabilities.bytes);
    assert_eq!(controls[1].response_bytes(), Some(capabilities_response));
}

fn assert_plan(
    recovery: &crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery,
    checkpoint: &control_recovery_tests::ConfirmedHealthCheckpoint,
    endpoint: &str,
    started_at: i64,
) {
    assert_eq!(recovery.plan_bytes(), checkpoint.plan_bytes);
    let plan = recovery.plan();
    assert_eq!(plan.profile(), ContractProfile::ExternalV1);
    assert_eq!(plan.acquisition_authority(), Some(AUTHORITY));
    assert_eq!(plan.endpoint(), endpoint);
    assert_eq!(plan.started_at().get(), started_at);
    assert_eq!(plan.deadline_at().get(), started_at + 15_000_000);
    assert_eq!(plan.observed_local(), STARTED_LOCAL);
    assert_eq!(plan.first_source_request().request_id(), checkpoint.data.id);
    assert_eq!(
        plan.first_source_request().request_bytes(),
        checkpoint.data.bytes
    );
    assert_eq!(
        plan.first_source_request().retry_policy(),
        (4, 1000, 60_000, 200)
    );
}

fn assert_wire_prefix(
    wire: &ExternalControlObservation,
    checkpoint: &control_recovery_tests::ConfirmedHealthCheckpoint,
    capabilities_response: &[u8],
) {
    assert_eq!(wire.health_requests, vec![checkpoint.health.request.bytes.clone()]);
    assert_eq!(wire.health_authorized, vec![true]);
    assert_eq!(wire.health_responses, vec![checkpoint.health.response_bytes.clone()]);
    assert!(wire.health_statuses.is_empty());
    assert_eq!(wire.capabilities_calls, 1);
    assert_eq!(
        wire.capabilities_requests,
        vec![checkpoint.capabilities.bytes.clone()]
    );
    assert_eq!(wire.capabilities_authorized, vec![true]);
    assert_eq!(wire.capabilities_responses, vec![capabilities_response.to_vec()]);
    assert!(wire.capabilities_statuses.is_empty());
}


fn assert_historical_provider_attempts(
    provider_attempts: &ProviderAttempts,
    checkpoint: &str,
) {
    let attempts = provider_attempts
        .accepted()
        .unwrap_or_else(|| panic!("{checkpoint}: expected an accepted historical attempt trace"));
    let expected = [
        (1, "Eastmoney", "rejected", "query_rejected", false, false),
        (2, "Eastmoney", "failed", "unavailable", true, false),
        (3, "Eastmoney", "selected", "selected", false, false),
    ];
    assert_eq!(attempts.len(), expected.len(), "{checkpoint}");
    for (actual, expected) in attempts.iter().zip(expected) {
        assert_eq!(
            (
                actual.ordinal,
                actual.provider.as_str(),
                actual.outcome.as_str(),
                actual.reason_code.as_str(),
                actual.retryable,
                actual.terminal,
            ),
            expected,
            "{checkpoint}",
        );
        assert!(actual.provider.is_supported(), "{checkpoint}: provider");
        assert!(actual.outcome.is_supported(), "{checkpoint}: outcome");
        assert!(
            actual.reason_code.is_supported(),
            "{checkpoint}: reason code",
        );
    }
}

#[tokio::test]
async fn single_user_external_macro_confirmed_data_retry_reopens_after_remaining_backoff() {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let mut time_paused = false;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                "TEST_CODE_RUN_EXTERNAL_MACRO_DATA_CONFIRMED_RETRY",
            )
            .await;
            external_server = Some(
                ExternalMtlsMacroFixture::bind_data_retry_then_zero_length_success_for_test()
                    .await
                    .expect("TEST_CODE External data Retry mTLS fixture"),
            );
            let external = external_server
                .as_ref()
                .expect("TEST_CODE External data Retry mTLS owner");
            let checkpoint = control_recovery_tests::reach_confirmed_health_checkpoint(
                &mut business,
                &baseline,
                external,
                "TEST_CODE_EXTERNAL_DATA_RETRY_HEALTH_OWNER",
            )
            .await;
            let control_tests::ExternalParentBaseline {
                endpoint: parent_endpoint,
                source: old_parent_source,
                queries: old_queries,
                stocks,
                config,
                intent,
                final_bytes: parent_final,
                receipt: parent_receipt,
                context: parent_context,
                head: _,
                tables: earlier_tables,
                facts: earlier_facts,
                audit: baseline_audit,
                network: old_network,
                memberships: old_memberships,
            } = baseline;
            let database = business.database();
            let started_at = micros(STARTED_LOCAL);
            let registered = control_unknown_commit_tests::registered();
            let search_service = macro_search_service(&registered);
            let capabilities_response = expected_capabilities(&checkpoint.capabilities.id);

            drop(old_queries);
            drop(old_parent_source);
            business.reopen();
            let first_macro_source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let first_parent_source = GrpcSource::from_board_loopback_test_client(
                connect_parent_instance(&parent_endpoint).await,
            );
            let first_queries = first_parent_source.connected_board_queries().await.unwrap();
            let first_clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at + 3_000_000).unwrap()),
                observation: DateTime::parse_from_rfc3339("2026-09-14T15:32:00+08:00")
                    .unwrap(),
                observation_calls: Cell::new(0),
            };
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let lease = local
                .resume_run(
                    &intent,
                    macro_lease(
                        "TEST_CODE_EXTERNAL_DATA_RETRY_OWNER_A",
                        started_at + 3_000_000,
                        started_at + 3_250_000,
                        checkpoint.head_version,
                    ),
                )
                .unwrap();
            let first_owner_head = local.inspect_run(&intent).unwrap().head_version();
            assert!(first_owner_head > checkpoint.head_version);
            let mut io = local
                .macro_preparation_io_v11(
                    lease,
                    &first_queries,
                    &first_clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &first_parent_source,
                    &first_macro_source,
                    &search_service,
                )
                .unwrap();
            let mut prepared = Box::pin(prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks.clone(),
                None,
                &mut io,
            ));

            let capabilities_deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                match futures::poll!(&mut prepared) {
                    std::task::Poll::Pending => {}
                    std::task::Poll::Ready(result) => panic!(
                        "TEST_CODE data Retry returned before Capabilities receipt: {result:?}"
                    ),
                }
                let wire = external.snapshot();
                if wire.capabilities_calls == 1 {
                    assert_eq!(wire.data_calls, 0);
                    break;
                }
                assert!(
                    std::time::Instant::now() < capabilities_deadline,
                    "TEST_CODE data Retry Capabilities receipt watchdog"
                );
                tokio::task::yield_now().await;
            }
            let (capabilities_pending, pending_run) =
                control_unknown_commit_tests::inspect_at(&database, &config, &intent);
            assert_eq!(pending_run.owner, "TEST_CODE_EXTERNAL_DATA_RETRY_OWNER_A");
            assert_eq!(pending_run.generation, 4);
            assert_eq!(pending_run.context, parent_context);
            let pending_controls = capabilities_pending.readiness_episodes()[0].controls();
            assert_eq!(pending_controls[0].outcome(), Some(MacroControlOutcome::Ready));
            assert_eq!(pending_controls[1].result_version(), None);
            assert_eq!(pending_controls[1].outcome(), None);
            assert_eq!(pending_controls[1].response_bytes(), None);
            assert!(capabilities_pending.has_unconfirmed_effect());
            assert!(capabilities_pending.attempts().is_empty());
            assert_eq!(control_tests::audit_snapshot_at(&database), baseline_audit);

            external.release_capabilities();
            let data_receipt_deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                match futures::poll!(&mut prepared) {
                    std::task::Poll::Pending => {}
                    std::task::Poll::Ready(result) => {
                        panic!("TEST_CODE data Retry returned before first data receipt: {result:?}")
                    }
                }
                if external.snapshot().data_calls == 1 {
                    break;
                }
                assert!(
                    std::time::Instant::now() < data_receipt_deadline,
                    "TEST_CODE data Retry first data receipt watchdog"
                );
                tokio::task::yield_now().await;
            }
            let (data_pending, data_pending_run) =
                control_unknown_commit_tests::inspect_at(&database, &config, &intent);
            let ready_result_version = data_pending.readiness_episodes()[0]
                .ready_result_version()
                .unwrap();
            assert_controls(
                &data_pending,
                &checkpoint,
                &capabilities_response,
                ready_result_version,
            );
            assert_plan(&data_pending, &checkpoint, external.endpoint(), started_at);
            assert_eq!(data_pending.parent_final_bytes(), parent_final);
            assert_eq!(data_pending.attempts().len(), 1);
            let first_pending = &data_pending.attempts()[0];
            assert_eq!(first_pending.attempt_ordinal(), 1);
            assert_eq!(first_pending.request_id(), checkpoint.data.id);
            assert_eq!(first_pending.request_bytes(), checkpoint.data.bytes);
            assert_eq!(
                first_pending.readiness_result_version(),
                Some(ready_result_version)
            );
            assert_eq!(first_pending.result_version(), None);
            assert_eq!(first_pending.response_bytes(), None);
            assert_eq!(first_pending.continuation(), None);
            assert!(data_pending.has_unconfirmed_effect());
            assert_eq!(data_pending_run.head, first_pending.begin_version());
            assert_eq!(data_pending_run.owner, "TEST_CODE_EXTERNAL_DATA_RETRY_OWNER_A");
            assert_eq!(data_pending_run.generation, 4);
            assert_eq!(data_pending_run.context, parent_context);
            assert_eq!(control_tests::audit_snapshot_at(&database), baseline_audit);
            let first_begin_version = first_pending.begin_version();
            let first_receipt_wire = external.snapshot();
            assert_wire_prefix(&first_receipt_wire, &checkpoint, &capabilities_response);
            assert_eq!(first_receipt_wire.data_calls, 1);
            assert_eq!(first_receipt_wire.data_methods, vec!["global_news"]);
            assert_eq!(first_receipt_wire.data_requests, vec![checkpoint.data.bytes.clone()]);
            assert_eq!(first_receipt_wire.data_authorized, vec![true]);
            assert!(first_receipt_wire.data_responses.is_empty());
            assert!(first_receipt_wire.data_statuses.is_empty());

            external.release_data();
            let retry_deadline = std::time::Instant::now() + Duration::from_secs(5);
            let (first_retry, first_retry_run) = loop {
                match futures::poll!(&mut prepared) {
                    std::task::Poll::Pending => {}
                    std::task::Poll::Ready(result) => panic!(
                        "TEST_CODE data Retry prepare returned before cancellation: {result:?}"
                    ),
                }
                let (recovery, run) =
                    control_unknown_commit_tests::inspect_at(&database, &config, &intent);
                if recovery.attempts().len() == 1
                    && recovery.attempts()[0].result_version().is_some()
                {
                    break (recovery, run);
                }
                assert!(
                    std::time::Instant::now() < retry_deadline,
                    "TEST_CODE data Retry result commit watchdog"
                );
                tokio::task::yield_now().await;
            };
            assert!(!first_retry.has_unconfirmed_effect());
            assert!(!first_retry.is_complete());
            assert!(first_retry
                .global_news(GlobalNewsProvider::Eastmoney)
                .is_none());
            assert_controls(
                &first_retry,
                &checkpoint,
                &capabilities_response,
                ready_result_version,
            );
            assert_plan(&first_retry, &checkpoint, external.endpoint(), started_at);
            assert_eq!(first_retry.parent_final_bytes(), parent_final);
            let first = &first_retry.attempts()[0];
            assert_eq!(first.attempt_ordinal(), 1);
            assert_eq!(first.begin_version(), first_begin_version);
            let first_result_version = first.result_version().unwrap();
            assert!(first_result_version > first_begin_version);
            assert_eq!(first_retry_run.head, first_result_version);
            assert_eq!(first_retry_run.owner, "TEST_CODE_EXTERNAL_DATA_RETRY_OWNER_A");
            assert_eq!(first_retry_run.generation, 4);
            assert_eq!(first_retry_run.context, parent_context);
            assert_eq!(first.request_id(), checkpoint.data.id);
            assert_eq!(first.request_bytes(), checkpoint.data.bytes);
            assert_eq!(first.readiness_result_version(), Some(ready_result_version));
            assert_eq!(first.response_bytes(), None);
            assert_eq!(
                first.continuation(),
                Some(MacroContinuation::Retry { backoff_ms: 1000 })
            );
            assert_eq!(first.retry_not_before, Some(started_at + 4_000_000));
            let first_material = first.result_material().unwrap();
            assert_eq!(
                first_material.diagnostic,
                Some("[redacted-unclassified-status]")
            );
            assert_eq!(first_material.retry_decision, RetryDecision::RetryBackoff);
            assert_eq!(
                first_material.continuation,
                MacroContinuation::Retry { backoff_ms: 1000 }
            );
            let (status_code, status_details) = match first_material.wire {
                MacroRecoveredWire::Status {
                    code,
                    details,
                    trailer: MacroRecoveredTrailer::Absent,
                } => (code, details.to_vec()),
                _ => panic!("TEST_CODE External first data result must retain Status material"),
            };
            assert_eq!(status_code, tonic::Code::Unavailable as i32);
            let expected_detail = ErrorDetail {
                request_id: checkpoint.data.id.clone(),
                operation: Operation::GlobalNews as i32,
                provider: "Eastmoney".to_owned(),
                reason_code: "no_verified_batch".to_owned(),
                retryable: true,
                ..ErrorDetail::default()
            };
            assert_eq!(status_details, expected_detail.encode_to_vec());
            assert_eq!(ErrorDetail::decode(status_details.as_slice()).unwrap(), expected_detail);
            let first_raw = data_result_bytes(&database, &intent, 1);
            let first_raw_json: serde_json::Value = serde_json::from_slice(&first_raw).unwrap();
            // An External status carries the captured (empty) body material, so
            // the raw result is v2 and binds the client descriptor.
            assert_eq!(first_raw_json.as_object().unwrap().len(), 10);
            assert_eq!(
                first_raw_json,
                serde_json::json!({
                    "version": 2,
                    "connect_unavailable": false,
                    "response": null,
                    "code": tonic::Code::Unavailable as i32,
                    "details": status_details,
                    "trailer": "Absent",
                    "diagnostic": "[redacted-unclassified-status]",
                    "decision": "RetryBackoff",
                    "backoff_ms": 1000,
                    "external_wire": {
                        "material": "external-unary-response-evidence-v1",
                        "profile": "ExternalV1",
                        "method": "OPERATION_GLOBAL_NEWS",
                        "client_descriptor_sha256":
                            crate::grpc_client::external_query_transport::EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
                        "evidence": {
                            "Missing": {
                                "framed_body_limit_bytes":
                                    crate::grpc_client::external_query_transport::EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
                            }
                        }
                    },
                })
            );
            assert_eq!(data_result_count(&database, &intent), 1);
            assert_eq!(control_tests::audit_snapshot_at(&database), baseline_audit);
            let first_wire = external.snapshot();
            assert_wire_prefix(&first_wire, &checkpoint, &capabilities_response);
            assert_eq!(first_wire.data_calls, 1);
            assert_eq!(first_wire.data_requests, vec![checkpoint.data.bytes.clone()]);
            assert_eq!(first_wire.data_authorized, vec![true]);
            assert!(first_wire.data_responses.is_empty());
            assert_eq!(first_wire.data_statuses.len(), 1);
            assert_eq!(first_wire.data_statuses[0].code, status_code);
            assert_eq!(first_wire.data_statuses[0].details, expected_detail.encode_to_vec());
            assert_eq!(first_wire.data_statuses[0].trailer, ObservedHealthTrailer::Absent);
            assert_eq!(first_clock.observation_calls.get(), 0);

            drop(prepared);
            drop(io);
            drop(local);
            drop(first_queries);
            drop(first_parent_source);
            drop(first_macro_source);
            business.reopen();

            let resumed_macro_source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let resumed_parent_source = GrpcSource::from_board_loopback_test_client(
                connect_parent_instance(&parent_endpoint).await,
            );
            let resumed_queries = resumed_parent_source.connected_board_queries().await.unwrap();
            let resumed_clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at + 3_500_000).unwrap()),
                observation: DateTime::parse_from_rfc3339("2026-09-14T15:45:00+08:00")
                    .unwrap(),
                observation_calls: Cell::new(0),
            };
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let lease = local
                .resume_run(
                    &intent,
                    macro_lease(
                        "TEST_CODE_EXTERNAL_DATA_RETRY_OWNER_B",
                        started_at + 3_500_000,
                        started_at + 4_250_000,
                        first_result_version,
                    ),
                )
                .unwrap();
            let resumed_head = local.inspect_run(&intent).unwrap().head_version();
            assert_eq!(resumed_head, first_result_version + 1);
            assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 5);
            let mut io = local
                .macro_preparation_io_v11(
                    lease,
                    &resumed_queries,
                    &resumed_clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &resumed_parent_source,
                    &resumed_macro_source,
                    &search_service,
                )
                .unwrap();

            tokio::time::pause();
            time_paused = true;
            let mut prepared = Box::pin(prepare_chain_analysis_with_io(
                NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
                stocks,
                None,
                &mut io,
            ));
            match futures::poll!(&mut prepared) {
                std::task::Poll::Pending => {}
                std::task::Poll::Ready(result) => {
                    panic!("TEST_CODE recovered data Retry returned before remaining wait: {result:?}")
                }
            }
            assert_eq!(external.snapshot(), first_wire);
            let (before_due, before_due_run) =
                control_unknown_commit_tests::inspect_at(&database, &config, &intent);
            assert_plan(&before_due, &checkpoint, external.endpoint(), started_at);
            assert_controls(
                &before_due,
                &checkpoint,
                &capabilities_response,
                ready_result_version,
            );
            assert_eq!(before_due.attempts().len(), 1);
            assert_eq!(before_due.attempts()[0].retry_not_before, Some(started_at + 4_000_000));
            assert!(!before_due.has_unconfirmed_effect());
            assert_eq!(before_due_run.head, resumed_head);
            assert_eq!(before_due_run.owner, "TEST_CODE_EXTERNAL_DATA_RETRY_OWNER_B");
            assert_eq!(before_due_run.generation, 5);
            assert_eq!(before_due_run.context, parent_context);
            assert_eq!(control_tests::audit_snapshot_at(&database), baseline_audit);

            resumed_clock
                .now
                .set(UtcMicros::try_new(started_at + 3_999_000).unwrap());
            tokio::time::advance(Duration::from_millis(499)).await;
            let early_deadline = std::time::Instant::now() + Duration::from_millis(100);
            while std::time::Instant::now() < early_deadline {
                match futures::poll!(&mut prepared) {
                    std::task::Poll::Pending => {}
                    std::task::Poll::Ready(result) => panic!(
                        "TEST_CODE External data Retry resumed before persisted due: {result:?}"
                    ),
                }
                tokio::task::yield_now().await;
            }
            assert_eq!(external.snapshot(), first_wire);
            assert_eq!(data_result_count(&database, &intent), 1);
            let (still_waiting, still_waiting_run) =
                control_unknown_commit_tests::inspect_at(&database, &config, &intent);
            assert_eq!(still_waiting.attempts().len(), 1);
            assert!(!still_waiting.has_unconfirmed_effect());
            assert_eq!(still_waiting_run.head, resumed_head);
            assert_eq!(still_waiting_run.generation, 5);
            assert_eq!(still_waiting_run.context, parent_context);

            resumed_clock
                .now
                .set(UtcMicros::try_new(started_at + 4_000_000).unwrap());
            tokio::time::advance(Duration::from_millis(1)).await;
            match futures::poll!(&mut prepared) {
                std::task::Poll::Pending => {}
                std::task::Poll::Ready(result) => panic!(
                    "TEST_CODE recovered data Retry returned at persisted due: {result:?}"
                ),
            }
            let (mut due_begin, mut due_begin_run) =
                control_unknown_commit_tests::inspect_at(&database, &config, &intent);
            if due_begin.attempts().len() == 1 {
                resumed_clock
                    .now
                    .set(UtcMicros::try_new(started_at + 4_001_000).unwrap());
                tokio::time::advance(Duration::from_millis(1)).await;
                match futures::poll!(&mut prepared) {
                    std::task::Poll::Pending => {}
                    std::task::Poll::Ready(result) => panic!(
                        "TEST_CODE recovered data Retry returned at bounded due tick: {result:?}"
                    ),
                }
                (due_begin, due_begin_run) =
                    control_unknown_commit_tests::inspect_at(&database, &config, &intent);
            }
            assert_eq!(due_begin.attempts().len(), 2);
            assert!(due_begin.has_unconfirmed_effect());
            let due_attempt = &due_begin.attempts()[1];
            assert_eq!(due_attempt.attempt_ordinal(), 2);
            assert_eq!(due_attempt.request_id(), checkpoint.data.id);
            assert_eq!(due_attempt.request_bytes(), checkpoint.data.bytes);
            assert_eq!(
                due_attempt.readiness_result_version(),
                Some(ready_result_version)
            );
            assert_eq!(due_attempt.result_version(), None);
            assert_eq!(due_attempt.response_bytes(), None);
            assert_eq!(due_attempt.continuation(), None);
            assert_eq!(due_begin_run.head, due_attempt.begin_version());
            assert!(due_begin_run.head > resumed_head);
            assert_eq!(due_begin_run.owner, "TEST_CODE_EXTERNAL_DATA_RETRY_OWNER_B");
            assert_eq!(due_begin_run.generation, 5);
            assert_eq!(due_begin_run.context, parent_context);
            assert_eq!(data_result_count(&database, &intent), 1);
            assert_eq!(control_tests::audit_snapshot_at(&database), baseline_audit);
            let due_wire = external.snapshot();
            assert_wire_prefix(&due_wire, &checkpoint, &capabilities_response);
            assert!((1..=2).contains(&due_wire.data_calls));
            assert_eq!(
                due_wire.data_methods,
                vec!["global_news"; due_wire.data_calls]
            );
            assert_eq!(
                due_wire.data_requests,
                vec![checkpoint.data.bytes.clone(); due_wire.data_calls]
            );
            assert_eq!(due_wire.data_authorized, vec![true; due_wire.data_calls]);
            assert!(due_wire.data_responses.is_empty());
            assert_eq!(due_wire.data_statuses, first_wire.data_statuses);
            tokio::time::resume();
            time_paused = false;
            let second_receipt_deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                match futures::poll!(&mut prepared) {
                    std::task::Poll::Pending => {}
                    std::task::Poll::Ready(result) => panic!(
                        "TEST_CODE recovered data Retry returned before second receipt: {result:?}"
                    ),
                }
                if external.snapshot().data_calls == 2 {
                    break;
                }
                assert!(
                    std::time::Instant::now() < second_receipt_deadline,
                    "TEST_CODE recovered data Retry second receipt watchdog"
                );
                tokio::task::yield_now().await;
            }
            let (second_pending, second_pending_run) =
                control_unknown_commit_tests::inspect_at(&database, &config, &intent);
            assert_eq!(second_pending.attempts().len(), 2);
            assert!(second_pending.has_unconfirmed_effect());
            let second_pending_attempt = &second_pending.attempts()[1];
            assert_eq!(second_pending_attempt.attempt_ordinal(), 2);
            assert_eq!(second_pending_attempt.request_id(), checkpoint.data.id);
            assert_eq!(second_pending_attempt.request_bytes(), checkpoint.data.bytes);
            assert_eq!(
                second_pending_attempt.readiness_result_version(),
                Some(ready_result_version)
            );
            assert!(second_pending_attempt.begin_version() > resumed_head);
            assert_eq!(second_pending_attempt.result_version(), None);
            assert_eq!(second_pending_run.head, second_pending_attempt.begin_version());
            assert_eq!(second_pending_run.generation, 5);
            assert_eq!(second_pending_run.context, parent_context);
            assert_eq!(data_result_count(&database, &intent), 1);
            assert_eq!(control_tests::audit_snapshot_at(&database), baseline_audit);
            let second_receipt_wire = external.snapshot();
            assert_wire_prefix(&second_receipt_wire, &checkpoint, &capabilities_response);
            assert_eq!(second_receipt_wire.data_calls, 2);
            assert_eq!(
                second_receipt_wire.data_requests,
                vec![checkpoint.data.bytes.clone(), checkpoint.data.bytes.clone()]
            );
            assert_eq!(second_receipt_wire.data_authorized, vec![true, true]);
            assert!(second_receipt_wire.data_responses.is_empty());
            assert_eq!(second_receipt_wire.data_statuses, first_wire.data_statuses);

            external.release_data();
            let stopped = tokio::time::timeout(Duration::from_secs(5), &mut prepared)
                .await
                .expect("TEST_CODE recovered External data completion watchdog")
                .expect_err("TEST_CODE first External source must leave Macro pending");
            assert_partial_macro_stop(&stopped);
            drop(prepared);
            drop(io);

            let completed = local.inspect_macro(&intent).unwrap();
            assert!(!completed.is_complete());
            assert!(!completed.has_unconfirmed_effect());
            assert_plan(&completed, &checkpoint, external.endpoint(), started_at);
            assert_controls(
                &completed,
                &checkpoint,
                &capabilities_response,
                ready_result_version,
            );
            assert_eq!(completed.parent_final_bytes(), parent_final);
            assert_eq!(completed.attempts().len(), 2);
            let restored_first = &completed.attempts()[0];
            assert_eq!(restored_first.begin_version(), first_begin_version);
            assert_eq!(restored_first.result_version(), Some(first_result_version));
            assert_eq!(restored_first.request_bytes(), checkpoint.data.bytes);
            assert_eq!(
                restored_first.continuation(),
                Some(MacroContinuation::Retry { backoff_ms: 1000 })
            );
            assert_eq!(restored_first.retry_not_before, Some(started_at + 4_000_000));
            let second = &completed.attempts()[1];
            assert_eq!(second.attempt_ordinal(), 2);
            assert_eq!(second.request_id(), checkpoint.data.id);
            assert_eq!(second.request_bytes(), checkpoint.data.bytes);
            assert_eq!(second.readiness_result_version(), Some(ready_result_version));
            assert!(second.result_version().unwrap() > second.begin_version());
            assert_eq!(second.retry_not_before, None);
            assert_eq!(second.continuation(), Some(MacroContinuation::Terminal));
            let second_material = second.result_material().unwrap();
            assert_eq!(second_material.diagnostic, None);
            assert_eq!(second_material.retry_decision, RetryDecision::NoRetry);
            assert_eq!(second_material.continuation, MacroContinuation::Terminal);
            let response_bytes = match second_material.wire {
                MacroRecoveredWire::Response(bytes) => bytes.to_vec(),
                _ => panic!("TEST_CODE External second data result must retain Response material"),
            };
            assert_eq!(response_bytes, expected_data_response(&checkpoint.data.id));
            assert!(response_bytes.ends_with(&[0x5a, 0x00]));
            let generated_response = QueryResponse::decode(response_bytes.as_slice()).unwrap();
            assert_ne!(generated_response.encode_to_vec(), response_bytes);
            assert_eq!(second.response_bytes(), Some(response_bytes.as_slice()));
            let second_raw = data_result_bytes(&database, &intent, 2);
            let second_raw_json: serde_json::Value = serde_json::from_slice(&second_raw).unwrap();
            assert_eq!(second_raw_json["version"], 2);
            assert_eq!(second_raw_json["connect_unavailable"], false);
            assert_eq!(
                second_raw_json["response"],
                serde_json::json!(response_bytes.clone())
            );
            assert!(second_raw_json["code"].is_null());
            assert!(second_raw_json["details"].is_null());
            assert_eq!(second_raw_json["trailer"], "Absent");
            assert!(second_raw_json["diagnostic"].is_null());
            assert_eq!(second_raw_json["decision"], "NoRetry");
            assert!(second_raw_json["backoff_ms"].is_null());
            let external_wire = &second_raw_json["external_wire"];
            assert_eq!(
                external_wire["material"],
                "external-unary-response-evidence-v1"
            );
            assert_eq!(external_wire["profile"], "ExternalV1");
            assert_eq!(external_wire["method"], "OPERATION_GLOBAL_NEWS");
            assert_eq!(
                external_wire["client_descriptor_sha256"],
                crate::grpc_client::external_query_transport::EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
            );
            let payload_material = &external_wire["evidence"]["Payload"];
            assert_eq!(
                payload_material["protobuf_payload"],
                serde_json::json!(response_bytes.clone())
            );
            assert_eq!(payload_material["decode_limit_bytes"], 4 * 1024 * 1024);
            assert_eq!(
                payload_material["payload_sha256"],
                serde_json::json!(hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&response_bytes)))
            );
            let source = completed
                .global_news(GlobalNewsProvider::Eastmoney)
                .expect("TEST_CODE recovered External first source");
            assert!(source.is_complete());
            assert_eq!(source.profile(), "ExternalV1");
            assert_eq!(source.acquisition_authority(), Some(AUTHORITY));
            assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
            assert!(source.error().is_none());
            let batch = source.batch().unwrap();
            assert!(!batch.is_verified_empty());
            assert_eq!(batch.evidence().provider, ProviderId::Eastmoney);
            assert_eq!(batch.evidence().source, "eastmoney-web");
            assert_eq!(batch.evidence().source_at.as_deref(), Some(SOURCE_AT));
            assert_eq!(batch.evidence().observed_at, OBSERVED_AT);
            assert_eq!(batch.evidence().batch_id, BATCH_ID);
            assert_eq!(batch.records().len(), 1);
            assert_eq!(batch.records()[0].item_id, "TEST_CODE_EXTERNAL_NEWS_001");
            assert_eq!(batch.records()[0].canonical_url, "https://example.com/TEST_CODE_EXTERNAL_NEWS_001");
            assert_eq!(source.final_bytes(), Some(EXPECTED_NATIVE));
            let receipt = source.audit_receipt().unwrap().clone();
            assert_eq!(receipt.previous_outcome, None);
            assert_eq!(receipt.current_outcome, "available");
            assert_eq!(completed.pending_source_identities().len(), 4);
            assert_eq!(completed.pending_research_queries().len(), 6);
            assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 5);
            assert_eq!(
                local
                    .inspect_run(&intent)
                    .unwrap()
                    .context()
                    .canonical_bytes(),
                parent_context
            );
            assert_eq!(resumed_clock.observation_calls.get(), 0);
            drop(local);

            assert_eq!(data_result_count(&database, &intent), 2);
            assert_eq!(data_result_bytes(&database, &intent, 1), first_raw);
            let final_wire = external.snapshot();
            assert_wire_prefix(&final_wire, &checkpoint, &capabilities_response);
            assert_eq!(final_wire.tcp_accepts, 3);
            assert_eq!(final_wire.data_calls, 2);
            assert_eq!(final_wire.data_methods, vec!["global_news", "global_news"]);
            assert_eq!(
                final_wire.data_requests,
                vec![checkpoint.data.bytes.clone(), checkpoint.data.bytes.clone()]
            );
            assert_eq!(final_wire.data_authorized, vec![true, true]);
            assert_eq!(final_wire.data_responses, vec![response_bytes]);
            assert_eq!(final_wire.data_statuses, first_wire.data_statuses);
            assert_eq!(
                control_tests::raw_control_bytes(&database, &intent, 1),
                checkpoint.raw
            );
            control_tests::assert_response_raw(
                &control_tests::raw_control_bytes(&database, &intent, 2),
                &capabilities_response,
            );
            let final_audit = control_tests::audit_snapshot_at(&database);
            assert_eq!(final_audit.len(), baseline_audit.len() + 1);
            assert_eq!(&final_audit[..baseline_audit.len()], baseline_audit.as_slice());
            let transaction = business.connection().unchecked_transaction().unwrap();
            let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
            assert_eq!(verified.receipt(), &receipt);
            let audit = verified.record();
            assert_eq!(audit.capability, "GlobalNews-Eastmoney");
            assert_eq!(audit.provider, "Eastmoney");
            assert_eq!(audit.source, "eastmoney-web");
            assert_eq!(audit.request_hash, REQUEST_HASH);
            assert_eq!(audit.source_at, Some(SOURCE_AT));
            assert_eq!(audit.observed_at, OBSERVED_AT);
            assert_eq!(audit.batch_id, Some(BATCH_ID));
            assert_eq!(audit.outcome, "available");
            assert_eq!(
                (
                    audit.request_count,
                    audit.accepted_count,
                    audit.rejected_count,
                ),
                (1, 1, 0)
            );
            assert_eq!(audit.reason_code, "accepted");
            assert!(!audit.retryable);
            read_acquisition_in_transaction(&transaction, &parent_receipt).unwrap();
            transaction.commit().unwrap();
            assert_eq!(
                parent_server.as_ref().unwrap().snapshot(),
                old_network
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                old_memberships
            );
            assert_eq!(
                old_fact_rows(business.connection(), &earlier_tables),
                earlier_facts
            );

            drop(resumed_queries);
            drop(resumed_parent_source);
            drop(resumed_macro_source);
            external.set_reject_new_connections_for_test(true);
            let before_final_wire = external.snapshot();
            business.reopen();
            let final_macro_source = GrpcSource::from_external_macro_bundle_for_test(
                external.bundle_path().to_path_buf(),
            );
            let final_parent_source = GrpcSource::from_board_loopback_test_client(
                connect_parent_instance(&parent_endpoint).await,
            );
            let final_queries = final_parent_source.connected_board_queries().await.unwrap();
            let final_clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(started_at + 4_500_000).unwrap()),
                observation: DateTime::parse_from_rfc3339("2026-09-14T15:46:00+08:00")
                    .unwrap(),
                observation_calls: Cell::new(0),
            };
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let completed_head = local.inspect_run(&intent).unwrap().head_version();
            let lease = local
                .resume_run(
                    &intent,
                    macro_lease(
                        "TEST_CODE_EXTERNAL_DATA_RETRY_OWNER_C",
                        started_at + 4_500_000,
                        started_at + 10_000_000,
                        completed_head,
                    ),
                )
                .unwrap();
            let final_owner_head = local.inspect_run(&intent).unwrap().head_version();
            assert_eq!(final_owner_head, completed_head + 1);
            assert_eq!(local.inspect_run(&intent).unwrap().lease_generation(), 6);
            let before_final_native = local
                .inspect_macro(&intent)
                .unwrap()
                .global_news(GlobalNewsProvider::Eastmoney)
                .unwrap()
                .final_bytes()
                .unwrap()
                .to_vec();
            assert_eq!(before_final_native, EXPECTED_NATIVE);
            let mut final_io = local
                .macro_preparation_io_v11(
                    lease,
                    &final_queries,
                    &final_clock,
                    FixedClusterConfiguration::resolve(Some("2")),
                    &final_parent_source,
                    &final_macro_source,
                    &search_service,
                )
                .unwrap();
            let before_final_audit = control_tests::audit_snapshot_at(&database);
            let before_final_health_raw =
                control_tests::raw_control_bytes(&database, &intent, 1);
            let before_final_capabilities_raw =
                control_tests::raw_control_bytes(&database, &intent, 2);
            let before_final_first_raw = data_result_bytes(&database, &intent, 1);
            let before_final_second_raw = data_result_bytes(&database, &intent, 2);
            let before_final_parent_network = parent_server.as_ref().unwrap().snapshot();
            let before_final_memberships =
                parent_server.as_ref().unwrap().membership_snapshot();
            let before_final_facts = fact_snapshot_at(&database, &earlier_tables);
            let stopped = final_io
                .macro_search_with_budget()
                .await
                .expect("TEST_CODE final External recovery has no elapsed timeout")
                .expect_err("TEST_CODE final External recovery still leaves Macro pending");
            assert!(matches!(
                stopped.downcast_ref::<PreparationStop>(),
                Some(PreparationStop::StageNotMigrated {
                    next: UnmigratedStage::Macro
                })
            ));
            drop(final_io);
            drop(local);
            let (final_recovery, final_run) =
                control_unknown_commit_tests::inspect_at(&database, &config, &intent);
            assert!(!final_recovery.is_complete());
            assert!(!final_recovery.has_unconfirmed_effect());
            assert_plan(&final_recovery, &checkpoint, external.endpoint(), started_at);
            assert_controls(
                &final_recovery,
                &checkpoint,
                &capabilities_response,
                ready_result_version,
            );
            assert_eq!(final_recovery.attempts().len(), 2);
            assert_eq!(
                final_recovery.attempts()[0].continuation(),
                Some(MacroContinuation::Retry { backoff_ms: 1000 })
            );
            assert_eq!(
                final_recovery.attempts()[1].continuation(),
                Some(MacroContinuation::Terminal)
            );
            assert_eq!(
                final_recovery
                    .global_news(GlobalNewsProvider::Eastmoney)
                    .unwrap()
                    .final_bytes(),
                Some(before_final_native.as_slice())
            );
            assert_eq!(final_run.head, final_owner_head);
            assert_eq!(final_run.owner, "TEST_CODE_EXTERNAL_DATA_RETRY_OWNER_C");
            assert_eq!(final_run.generation, 6);
            assert_eq!(final_run.context, parent_context);
            assert_eq!(external.snapshot(), before_final_wire);
            assert_eq!(control_tests::audit_snapshot_at(&database), before_final_audit);
            assert_eq!(
                control_tests::raw_control_bytes(&database, &intent, 1),
                before_final_health_raw
            );
            assert_eq!(
                control_tests::raw_control_bytes(&database, &intent, 2),
                before_final_capabilities_raw
            );
            assert_eq!(
                data_result_bytes(&database, &intent, 1),
                before_final_first_raw
            );
            assert_eq!(
                data_result_bytes(&database, &intent, 2),
                before_final_second_raw
            );
            assert_eq!(before_final_second_raw, second_raw);
            assert_eq!(
                parent_server.as_ref().unwrap().snapshot(),
                before_final_parent_network
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                before_final_memberships
            );
            assert_eq!(
                fact_snapshot_at(&database, &earlier_tables),
                before_final_facts
            );
            assert_eq!(final_clock.observation_calls.get(), 0);
            assert_eq!(before_final_wire, final_wire);
            drop(final_queries);
            drop(final_parent_source);
            drop(final_macro_source);
        },
    ))
    .catch_unwind()
    .await;

    let time_cleanup = if time_paused {
        std::panic::catch_unwind(tokio::time::resume)
    } else {
        Ok(())
    };
    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "External data confirmed Retry",
    )
    .await;
    drop(business);
    time_cleanup.expect("TEST_CODE resume paused time after External data Retry body");
    match body {
        Ok(result) => result.expect("TEST_CODE External data Retry body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[derive(Clone, Copy, Debug)]
enum ValidFailureWriterCase {
    Missing,
    Overflow,
    InvalidFrame,
}

impl ValidFailureWriterCase {
    fn label(self) -> &'static str {
        match self {
            Self::Missing => "Missing",
            Self::Overflow => "Overflow",
            Self::InvalidFrame => "InvalidFrame",
        }
    }

    fn run_id(self) -> &'static str {
        match self {
            Self::Missing => "TEST_CODE_EXTERNAL_VALID_MISSING_WRITER_REOPEN",
            Self::Overflow => "TEST_CODE_EXTERNAL_VALID_OVERFLOW_WRITER_REOPEN",
            Self::InvalidFrame => "TEST_CODE_EXTERNAL_VALID_INVALID_FRAME_WRITER_REOPEN",
        }
    }

    fn owner(self) -> &'static str {
        match self {
            Self::Missing => "TEST_CODE_EXTERNAL_VALID_MISSING_OWNER",
            Self::Overflow => "TEST_CODE_EXTERNAL_VALID_OVERFLOW_OWNER",
            Self::InvalidFrame => "TEST_CODE_EXTERNAL_VALID_INVALID_FRAME_OWNER",
        }
    }

    fn evidence(self) -> crate::grpc_client::external_query_transport::ExternalWireMaterialV1 {
        use crate::grpc_client::external_query_transport::{
            ExternalFrameFailureV1, ExternalWireMaterialV1,
            EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
        };

        match self {
            Self::Missing => ExternalWireMaterialV1::Missing {
                framed_body_limit_bytes: EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
            },
            Self::Overflow => ExternalWireMaterialV1::Overflow {
                framed_body_limit_bytes: EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
                observed_framed_body_bytes_at_least:
                    EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES + 1,
            },
            Self::InvalidFrame => ExternalWireMaterialV1::InvalidFrame {
                failure: ExternalFrameFailureV1::CompressionUnsupported,
                grpc_body_bytes: vec![1, 0, 0, 0, 0],
                body_sha256:
                    "957b88b12730e646e0f33d3618b77dfa579e8231e3c59c7104be7165611c8027"
                        .to_owned(),
                framed_body_limit_bytes: EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
            },
        }
    }

    fn evidence_json(self) -> serde_json::Value {
        use crate::grpc_client::external_query_transport::EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES;

        match self {
            Self::Missing => serde_json::json!({
                "Missing": {
                    "framed_body_limit_bytes": EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
                }
            }),
            Self::Overflow => serde_json::json!({
                "Overflow": {
                    "framed_body_limit_bytes": EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
                    "observed_framed_body_bytes_at_least":
                        EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES + 1,
                }
            }),
            Self::InvalidFrame => serde_json::json!({
                "InvalidFrame": {
                    "failure": "CompressionUnsupported",
                    "grpc_body_bytes": [1, 0, 0, 0, 0],
                    "body_sha256":
                        "957b88b12730e646e0f33d3618b77dfa579e8231e3c59c7104be7165611c8027",
                    "framed_body_limit_bytes": EXTERNAL_QUERY_FRAMED_BODY_LIMIT_BYTES,
                }
            }),
        }
    }
}

async fn run_valid_failure_writer_reopen_case(case: ValidFailureWriterCase) {
    use crate::grpc_client::client::macro_attempt::{
        ExternalMacroAttemptCompletion, MacroAttemptCompletion, MacroTrailerMaterial,
    };
    use crate::grpc_client::external_query_transport::{
        wire_error, ExternalQueryMethod, ExternalWireEvidenceV1,
        EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
    };

    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                case.run_id(),
            )
            .await;
            external_server = Some(
                ExternalMtlsMacroFixture::bind_data_success_for_test()
                    .await
                    .unwrap_or_else(|error| {
                        panic!("TEST_CODE {} mTLS fixture: {error:?}", case.label())
                    }),
            );
            let external = external_server.as_ref().unwrap();
            let checkpoint = control_recovery_tests::reach_confirmed_health_checkpoint(
                &mut business,
                &baseline,
                external,
                case.owner(),
            )
            .await;
            let database = business.database();
            let started_at = micros(STARTED_LOCAL);
            let prepared = crate::grpc_client::client::GrpcMarketClient::prepare_client_bundle(
                external.bundle_path(),
            )
            .unwrap_or_else(|error| {
                panic!("TEST_CODE {} prepare client bundle: {error:?}", case.label())
            });

            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let lease = local
                .resume_run(
                    &baseline.intent,
                    macro_lease(
                        case.owner(),
                        started_at + 3_000_000,
                        started_at + 12_000_000,
                        checkpoint.head_version,
                    ),
                )
                .unwrap();
            let capabilities_material = {
                let recovery = local.inspect_macro(&baseline.intent).unwrap();
                let controls = recovery.readiness_episodes()[0].controls();
                assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Ready));
                assert_eq!(controls[1].begin_version(), None);
                controls[1].request_material()
            };
            let capabilities_attempt = prepared
                .resume_capabilities_attempt(capabilities_material)
                .unwrap();
            assert_eq!(
                capabilities_attempt.request_id(),
                checkpoint.capabilities.id
            );
            assert_eq!(
                capabilities_attempt.request_bytes(),
                checkpoint.capabilities.bytes
            );
            let (lease, capabilities_call) = local
                .begin_capabilities_control(
                    lease,
                    &capabilities_attempt,
                    UtcMicros::try_new(started_at + 4_000_000).unwrap(),
                )
                .unwrap();
            external.release_capabilities();
            let capabilities_completion = tokio::time::timeout(
                Duration::from_secs(5),
                capabilities_attempt.execute(),
            )
            .await
            .unwrap_or_else(|_| panic!("TEST_CODE {} Capabilities deadline", case.label()));
            assert!(capabilities_completion.processed().is_ok());
            let (lease, outcome) = local
                .record_capabilities_control_result(
                    lease,
                    capabilities_call,
                    &capabilities_completion,
                    UtcMicros::try_new(started_at + 5_000_000).unwrap(),
                )
                .unwrap();
            assert_eq!(outcome, MacroControlOutcome::Ready);
            drop(capabilities_completion);

            let ready = local.inspect_macro(&baseline.intent).unwrap();
            let ready_result_version = ready.readiness_episodes()[0]
                .ready_result_version()
                .unwrap();
            let authorized = prepared
                .resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    ready
                        .plan()
                        .first_source_request()
                        .restored_external(ready.plan().endpoint(), 1),
                )
                .unwrap();
            drop(ready);
            let (lease, call) = local
                .begin_prepared_macro_attempt(
                    lease,
                    &authorized,
                    UtcMicros::try_new(started_at + 6_000_000).unwrap(),
                )
                .unwrap();
            let begun = local.inspect_macro(&baseline.intent).unwrap();
            assert!(begun.has_unconfirmed_effect());
            assert_eq!(begun.attempts().len(), 1);
            let begun_attempt = &begun.attempts()[0];
            assert_eq!(begun_attempt.attempt_ordinal(), 1);
            assert_eq!(begun_attempt.request_id(), checkpoint.data.id);
            assert_eq!(begun_attempt.request_bytes(), checkpoint.data.bytes);
            assert_eq!(
                begun_attempt.readiness_result_version(),
                Some(ready_result_version)
            );
            assert_eq!(begun_attempt.result_version(), None);
            assert!(begun_attempt.result_material().is_none());
            assert!(begun.global_news(GlobalNewsProvider::Eastmoney).is_none());
            drop(begun);
            assert_eq!(data_result_count(&database, &baseline.intent), 0);
            let before_result_wire = external.snapshot();
            assert_wire_prefix(
                &before_result_wire,
                &checkpoint,
                &expected_capabilities(&checkpoint.capabilities.id),
            );
            assert_eq!(before_result_wire.tcp_accepts, 2);
            assert_eq!(before_result_wire.data_calls, 0);
            assert!(before_result_wire.data_requests.is_empty());
            assert!(before_result_wire.data_responses.is_empty());
            assert!(before_result_wire.data_statuses.is_empty());

            let evidence = ExternalWireEvidenceV1 {
                material: "external-unary-response-evidence-v1".to_owned(),
                profile: "ExternalV1".to_owned(),
                method: ExternalQueryMethod::GlobalNews,
                client_descriptor_sha256: EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256.to_owned(),
                evidence: case.evidence(),
            };
            let completion = ExternalMacroAttemptCompletion::Unary(MacroAttemptCompletion {
                response_bytes: None,
                status_code: None,
                status_details: None,
                status_error_detail_trailer: MacroTrailerMaterial::Absent,
                processed: Err(wire_error("external_response_wire_invalid")),
                retry_decision: RetryDecision::NoRetry,
                continuation: MacroContinuation::Terminal,
                external_wire: Some(evidence),
            });
            let ExternalMacroAttemptCompletion::Unary(unary) = &completion else {
                panic!("TEST_CODE valid failure must use unary completion");
            };
            let processed_error = unary
                .processed
                .as_ref()
                .expect_err("TEST_CODE valid failure must remain a typed local wire error");
            assert_eq!(processed_error.details().code, "external_response_wire_invalid");
            assert_eq!(
                processed_error.details().reason_code.as_deref(),
                Some("external_response_wire_invalid")
            );
            assert_eq!(processed_error.details().retryable, Some(false));
            assert_eq!(processed_error.safe_diagnostic(), None);
            let _lease = local
                .record_external_macro_result(
                    lease,
                    call,
                    &completion,
                    UtcMicros::try_new(started_at + 7_000_000).unwrap(),
                )
                .unwrap();

            let raw = data_result_bytes(&database, &baseline.intent, 1);
            assert_eq!(data_result_count(&database, &baseline.intent), 1);
            let raw_json: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            assert_eq!(
                raw_json,
                serde_json::json!({
                    "version": 2,
                    "connect_unavailable": false,
                    "response": null,
                    "code": null,
                    "details": null,
                    "trailer": "Absent",
                    "diagnostic": null,
                    "decision": "NoRetry",
                    "backoff_ms": null,
                    "external_wire": {
                        "material": "external-unary-response-evidence-v1",
                        "profile": "ExternalV1",
                        "method": "OPERATION_GLOBAL_NEWS",
                        "client_descriptor_sha256": EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256,
                        "evidence": case.evidence_json(),
                    }
                }),
                "{} exact committed RawResult V2",
                case.label()
            );
            let recovered = local.inspect_macro(&baseline.intent).unwrap();
            assert!(!recovered.has_unconfirmed_effect());
            assert_eq!(recovered.attempts().len(), 1);
            let attempt = &recovered.attempts()[0];
            assert_eq!(attempt.result_version().is_some(), true);
            assert_eq!(attempt.continuation(), Some(MacroContinuation::Terminal));
            let material = attempt.result_material().unwrap();
            assert!(matches!(material.wire, MacroRecoveredWire::LocalWireFailure));
            assert_eq!(material.diagnostic, None);
            assert_eq!(material.retry_decision, RetryDecision::NoRetry);
            assert_eq!(material.continuation, MacroContinuation::Terminal);
            let source = recovered
                .global_news(GlobalNewsProvider::Eastmoney)
                .unwrap();
            assert!(source.is_complete());
            assert_eq!(source.profile(), "ExternalV1");
            assert_eq!(source.acquisition_authority(), Some(AUTHORITY));
            assert_eq!(source.retry_policy(), (4, 1000, 60_000, 200));
            assert!(source.batch().is_none());
            let error = source.error().unwrap();
            assert_eq!(error.capability(), "GrpcExternalV1");
            assert_eq!(error.provider(), None);
            assert_eq!(error.audit_outcome(), "partial");
            assert_eq!(error.reason_code(), "internal");
            assert!(!error.retryable());
            assert_eq!(error.message(), "ExternalV1 GlobalNews 查询失败");
            let native = source.final_bytes().unwrap().to_vec();
            assert_eq!(
                native,
                r#"{"error":{"audit_outcome":"partial","capability":"GrpcExternalV1","message":"ExternalV1 GlobalNews 查询失败","provider":null,"reason_code":"internal","retryable":false},"kind":"Error","version":1}"#.as_bytes()
            );
            let receipt = source.audit_receipt().unwrap().clone();
            let plan_bytes = recovered.plan_bytes().to_vec();
            let parent_final = recovered.parent_final_bytes().to_vec();
            let pending_sources = recovered.pending_source_identities().to_vec();
            let pending_research = recovered.pending_research_queries().to_vec();
            drop(recovered);
            drop(local);

            let transaction = business.connection().unchecked_transaction().unwrap();
            let verified = read_acquisition_in_transaction(&transaction, &receipt).unwrap();
            assert_eq!(verified.receipt(), &receipt);
            let audit = verified.record();
            assert_eq!(audit.capability, "GlobalNews-Eastmoney");
            assert_eq!(audit.provider, "Eastmoney");
            assert_eq!(audit.source, "review-data-gateway");
            assert_eq!(audit.request_hash, REQUEST_HASH);
            assert_eq!(audit.source_at, None);
            assert_eq!(audit.batch_id, None);
            assert_eq!(audit.outcome, "partial");
            assert_eq!(
                (audit.request_count, audit.accepted_count, audit.rejected_count),
                (1, 0, 1)
            );
            assert_eq!(audit.reason_code, "internal");
            assert!(!audit.retryable);
            read_acquisition_in_transaction(&transaction, &baseline.receipt).unwrap();
            transaction.commit().unwrap();

            let audit_snapshot = control_tests::audit_snapshot_at(&database);
            let facts = fact_snapshot_at(&database, &baseline.tables);
            let parent_network_before = parent_server
                .as_ref()
                .unwrap()
                .snapshot_with_tcp_for_test();
            let parent_membership_before = parent_server
                .as_ref()
                .unwrap()
                .membership_snapshot();
            external.set_reject_new_connections_for_test(true);
            let wire_before = external.snapshot();
            assert_eq!(wire_before, before_result_wire);
            business.reopen();

            let mut reopened_local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let reopened = reopened_local.inspect_macro(&baseline.intent).unwrap();
            assert!(!reopened.has_unconfirmed_effect());
            assert_eq!(reopened.plan_bytes(), plan_bytes);
            assert_eq!(reopened.parent_final_bytes(), parent_final);
            assert_eq!(reopened.pending_source_identities(), pending_sources);
            assert_eq!(reopened.pending_research_queries(), pending_research);
            assert_eq!(reopened.attempts().len(), 1);
            let reopened_attempt = &reopened.attempts()[0];
            assert_eq!(reopened_attempt.request_id(), checkpoint.data.id);
            assert_eq!(reopened_attempt.request_bytes(), checkpoint.data.bytes);
            assert_eq!(
                reopened_attempt.readiness_result_version(),
                Some(ready_result_version)
            );
            assert_eq!(
                reopened_attempt.continuation(),
                Some(MacroContinuation::Terminal)
            );
            let reopened_material = reopened_attempt.result_material().unwrap();
            assert!(matches!(
                reopened_material.wire,
                MacroRecoveredWire::LocalWireFailure
            ));
            assert_eq!(reopened_material.diagnostic, None);
            assert_eq!(reopened_material.retry_decision, RetryDecision::NoRetry);
            assert_eq!(
                reopened_material.continuation,
                MacroContinuation::Terminal
            );
            let reopened_source = reopened
                .global_news(GlobalNewsProvider::Eastmoney)
                .unwrap();
            assert_eq!(reopened_source.final_bytes(), Some(native.as_slice()));
            assert_eq!(reopened_source.audit_receipt(), Some(&receipt));
            let reopened_error = reopened_source.error().unwrap();
            assert_eq!(reopened_error.capability(), "GrpcExternalV1");
            assert_eq!(reopened_error.provider(), None);
            assert_eq!(reopened_error.audit_outcome(), "partial");
            assert_eq!(reopened_error.reason_code(), "internal");
            assert!(!reopened_error.retryable());
            assert_eq!(reopened_error.message(), "ExternalV1 GlobalNews 查询失败");
            drop(reopened);
            drop(reopened_local);

            assert_eq!(data_result_bytes(&database, &baseline.intent, 1), raw);
            assert_eq!(control_tests::audit_snapshot_at(&database), audit_snapshot);
            assert_eq!(fact_snapshot_at(&database, &baseline.tables), facts);
            assert_eq!(external.snapshot(), wire_before);
            assert_eq!(
                parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test(),
                parent_network_before
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                parent_membership_before
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
        Ok(result) => result
            .unwrap_or_else(|_| panic!("TEST_CODE {} writer/reopen body deadline", case.label())),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn single_user_external_macro_valid_missing_writer_reopens_without_data_rpc() {
    run_valid_failure_writer_reopen_case(ValidFailureWriterCase::Missing).await;
}

#[tokio::test]
async fn single_user_external_macro_valid_overflow_writer_reopens_without_data_rpc() {
    run_valid_failure_writer_reopen_case(ValidFailureWriterCase::Overflow).await;
}

#[tokio::test]
async fn single_user_external_macro_valid_invalid_frame_writer_reopens_without_data_rpc() {
    run_valid_failure_writer_reopen_case(ValidFailureWriterCase::InvalidFrame).await;
}

#[tokio::test]
async fn single_user_external_macro_real_v2_tamper_rejects_before_any_resend() {
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
                "TEST_CODE_EXTERNAL_REAL_V2_TAMPER",
            )
            .await;
            let database = business.database();
            let legal = data_result_bytes(&database, &confirmed.intent, 1);
            let legal_json: serde_json::Value = serde_json::from_slice(&legal).unwrap();
            assert_eq!(legal_json["version"], 2);
            assert_eq!(
                legal_json["external_wire"]["material"],
                "external-unary-response-evidence-v1"
            );
            assert_eq!(legal_json["external_wire"]["profile"], "ExternalV1");
            assert_eq!(
                legal_json["external_wire"]["method"],
                "OPERATION_GLOBAL_NEWS"
            );
            assert_eq!(
                legal_json["external_wire"]["client_descriptor_sha256"],
                crate::grpc_client::external_query_transport::EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
            );
            assert_eq!(
                legal_json["external_wire"]["evidence"]["Payload"]["protobuf_payload"],
                legal_json["response"]
            );
            assert_eq!(
                legal_json["external_wire"]["evidence"]["Payload"]["decode_limit_bytes"],
                crate::grpc_client::external_query_transport::EXTERNAL_QUERY_DECODE_LIMIT_BYTES
            );

            let pristine_directory = tempfile::tempdir().unwrap();
            let pristine_database = pristine_directory.path().join("business.sqlite");
            let store = business.store.take().unwrap();
            store.connection.close().unwrap();
            std::fs::copy(&database, &pristine_database).unwrap();
            business.store = Some(BusinessIntentStore::open(&database).unwrap());

            let external = external_server.as_ref().unwrap();
            external.set_reject_new_connections_for_test(true);
            let external_before = external.snapshot();
            let parent_before = parent_server
                .as_ref()
                .unwrap()
                .snapshot_with_tcp_for_test();
            let cases = [
                RealV2Tamper::OuterFactDigest,
                RealV2Tamper::MaterialTag,
                RealV2Tamper::Profile,
                RealV2Tamper::Method,
                RealV2Tamper::Descriptor,
                RealV2Tamper::PayloadHash,
                RealV2Tamper::PayloadBinding,
                RealV2Tamper::DecodeLimit,
                RealV2Tamper::FailureMaterial,
            ];
            for case in cases {
                let candidate_directory = tempfile::tempdir().unwrap();
                let candidate_database = candidate_directory.path().join("business.sqlite");
                std::fs::copy(&pristine_database, &candidate_database).unwrap();

                let mut candidate_store =
                    BusinessIntentStore::open(&candidate_database).unwrap();
                let mut bound_local = candidate_store
                    .single_user_local_chain_post_close(&confirmed.config)
                    .unwrap_or_else(|error| {
                        panic!("{} independent positive bind: {error:?}", case.label())
                    });
                let positive = bound_local
                    .inspect_macro(&confirmed.intent)
                    .unwrap_or_else(|error| {
                        panic!("{} independent positive control: {error:?}", case.label())
                    });
                assert!(!positive.has_unconfirmed_effect(), "{}", case.label());
                assert_eq!(positive.attempts().len(), 1, "{}", case.label());
                assert_eq!(
                    positive.plan().deadline_at().get() - positive.plan().started_at().get(),
                    15_000_000,
                    "{}",
                    case.label()
                );
                assert_eq!(
                    positive.attempts()[0].continuation(),
                    Some(MacroContinuation::Terminal),
                    "{}",
                    case.label()
                );
                assert!(
                    positive
                        .global_news(GlobalNewsProvider::Eastmoney)
                        .unwrap()
                        .is_complete(),
                    "{}",
                    case.label()
                );
                drop(positive);
                let candidate_reader = rusqlite::Connection::open(&candidate_database).unwrap();
                let links_before =
                    durable_attempt_links(&candidate_reader, &confirmed.intent);
                candidate_reader.close().unwrap();

                let (before, after) = mutate_copied_v2_result(
                    &candidate_database,
                    &confirmed.intent,
                    case,
                );
                assert_eq!(before.bytes, legal, "{} clean copied row", case.label());
                assert!(matches!(
                    bound_local.inspect_macro(&confirmed.intent),
                    Err(ChainPostCloseError::SchemaRejected)
                ), "{} damaged row accepted by held reader", case.label());
                drop(bound_local);
                assert!(candidate_store.connection.is_autocommit());
                assert_eq!(
                    {
                        let reader =
                            rusqlite::Connection::open(&candidate_database).unwrap();
                        let stored = stored_data_result(&reader, &confirmed.intent);
                        assert_eq!(
                            durable_attempt_links(&reader, &confirmed.intent),
                            links_before,
                            "{} deadline/begin/result links changed",
                            case.label()
                        );
                        reader.close().unwrap();
                        stored
                    },
                    after,
                    "{} recovery mutated damaged row",
                    case.label()
                );
                candidate_store.connection.close().unwrap();
                let mut fresh_store =
                    BusinessIntentStore::open(&candidate_database).unwrap();
                assert!(matches!(
                    fresh_store.single_user_local_chain_post_close(&confirmed.config),
                    Err(ChainPostCloseError::SchemaRejected)
                ), "{} damaged row accepted by fresh full validation", case.label());
                assert!(fresh_store.connection.is_autocommit());
                assert_eq!(
                    stored_data_result(&fresh_store.connection, &confirmed.intent),
                    after,
                    "{} fresh bind repaired damaged row",
                    case.label()
                );
                assert_eq!(
                    durable_attempt_links(&fresh_store.connection, &confirmed.intent),
                    links_before,
                    "{} fresh bind changed deadline/begin/result links",
                    case.label()
                );
                fresh_store.connection.close().unwrap();
                assert_eq!(external.snapshot(), external_before, "{}", case.label());
                assert_eq!(
                    parent_server
                        .as_ref()
                        .unwrap()
                        .snapshot_with_tcp_for_test(),
                    parent_before,
                    "{}",
                    case.label()
                );
            }
        },
    ))
    .catch_unwind()
    .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        "External data real V2 tamper",
    )
    .await;
    drop(business);
    match body {
        Ok(result) => result.expect("TEST_CODE External data real V2 tamper body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoricalProviderAttemptsCase {
    Published,
    Unpublished,
    CapabilitiesRequestIdTamper,
    ReadinessLinkTamper,
}

impl HistoricalProviderAttemptsCase {
    fn label(self) -> &'static str {
        match self {
            Self::Published => "historical provider attempts",
            Self::Unpublished => "unpublished historical provider attempts",
            Self::CapabilitiesRequestIdTamper => "historical Capabilities request-id tamper",
            Self::ReadinessLinkTamper => "historical readiness-link tamper",
        }
    }

    fn run_id(self) -> &'static str {
        match self {
            Self::Published => "TEST_CODE_EXTERNAL_HISTORICAL_ATTEMPTS_RUN",
            Self::Unpublished => "TEST_CODE_EXTERNAL_UNPUBLISHED_ATTEMPTS_RUN",
            Self::CapabilitiesRequestIdTamper => {
                "TEST_CODE_EXTERNAL_HISTORICAL_CAPABILITIES_ID_TAMPER_RUN"
            }
            Self::ReadinessLinkTamper => {
                "TEST_CODE_EXTERNAL_HISTORICAL_READINESS_LINK_TAMPER_RUN"
            }
        }
    }

    fn owner(self) -> &'static str {
        match self {
            Self::Published => "TEST_CODE_EXTERNAL_HISTORICAL_ATTEMPTS_OWNER",
            Self::Unpublished => "TEST_CODE_EXTERNAL_UNPUBLISHED_ATTEMPTS_OWNER",
            Self::CapabilitiesRequestIdTamper => {
                "TEST_CODE_EXTERNAL_HISTORICAL_CAPABILITIES_ID_TAMPER_OWNER"
            }
            Self::ReadinessLinkTamper => {
                "TEST_CODE_EXTERNAL_HISTORICAL_READINESS_LINK_TAMPER_OWNER"
            }
        }
    }

    fn unpublished_provider(self) -> bool {
        self == Self::Unpublished
    }

    fn damages_history(self) -> bool {
        matches!(
            self,
            Self::CapabilitiesRequestIdTamper | Self::ReadinessLinkTamper
        )
    }
}

fn historical_wire_provider_attempts(
    case: HistoricalProviderAttemptsCase,
) -> Vec<ProviderAttemptDetail> {
    vec![
        ProviderAttemptDetail {
            ordinal: 1,
            provider: "Eastmoney".to_owned(),
            outcome: "rejected".to_owned(),
            reason_code: "query_rejected".to_owned(),
            retryable: false,
            terminal: false,
        },
        ProviderAttemptDetail {
            ordinal: 2,
            provider: "Eastmoney".to_owned(),
            outcome: "failed".to_owned(),
            reason_code: "unavailable".to_owned(),
            retryable: true,
            terminal: false,
        },
        ProviderAttemptDetail {
            ordinal: 3,
            provider: if case.unpublished_provider() {
                "Cailianpress"
            } else {
                "Eastmoney"
            }
            .to_owned(),
            outcome: "selected".to_owned(),
            reason_code: "selected".to_owned(),
            retryable: false,
            terminal: false,
        },
    ]
}

fn assert_historical_provider_attempts_for_case(
    provider_attempts: &ProviderAttempts,
    case: HistoricalProviderAttemptsCase,
    checkpoint: &str,
) {
    if case.unpublished_provider() {
        assert!(
            matches!(
                provider_attempts,
                ProviderAttempts::Rejected { observed_count: 3 }
            ),
            "{checkpoint}: unpublished provider must reject the complete trace",
        );
        assert!(provider_attempts.accepted().is_none(), "{checkpoint}");
    } else {
        assert_historical_provider_attempts(provider_attempts, checkpoint);
    }
}

#[derive(Clone, Debug, PartialEq)]
struct HistoricalDatabaseSnapshot {
    catalog: Vec<Vec<rusqlite::types::Value>>,
    tables: BTreeMap<String, Vec<Vec<rusqlite::types::Value>>>,
}

fn historical_rows(
    connection: &Connection,
    sql: &str,
) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = connection.prepare(sql).unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn historical_database_snapshot(connection: &Connection) -> HistoricalDatabaseSnapshot {
    let catalog = historical_rows(
        connection,
        "SELECT type,name,tbl_name,rootpage,CAST(sql AS BLOB) \
         FROM sqlite_schema ORDER BY type,name",
    );
    let table_names = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let tables = table_names
        .into_iter()
        .map(|name| {
            let quoted = name.replace('"', "\"\"");
            let query = format!("SELECT * FROM \"{quoted}\"");
            let width = connection.prepare(&query).unwrap().column_count();
            let order = (1..=width)
                .map(|column| column.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let rows = historical_rows(connection, &format!("{query} ORDER BY {order}"));
            (name, rows)
        })
        .collect();
    HistoricalDatabaseSnapshot { catalog, tables }
}

fn historical_snapshot_at(database: &std::path::Path) -> HistoricalDatabaseSnapshot {
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection.busy_timeout(Duration::from_millis(250)).unwrap();
    let snapshot = historical_database_snapshot(&connection);
    connection.close().unwrap();
    snapshot
}

fn historical_value_text(value: &rusqlite::types::Value) -> &str {
    match value {
        rusqlite::types::Value::Text(value) => value,
        _ => panic!("TEST_CODE historical tamper expected text"),
    }
}

fn historical_value_integer(value: &rusqlite::types::Value) -> i64 {
    match value {
        rusqlite::types::Value::Integer(value) => *value,
        _ => panic!("TEST_CODE historical tamper expected integer"),
    }
}

fn historical_target_row(
    row: &[rusqlite::types::Value],
    intent: &IntentId,
    case: HistoricalProviderAttemptsCase,
) -> bool {
    if historical_value_text(&row[0]) != intent.as_str() {
        return false;
    }
    match case {
        HistoricalProviderAttemptsCase::CapabilitiesRequestIdTamper => {
            historical_value_integer(&row[12]) == 1
                && historical_value_integer(&row[13]) == 2
        }
        HistoricalProviderAttemptsCase::ReadinessLinkTamper => {
            historical_value_text(&row[12]) == "Gateway"
                && historical_value_integer(&row[13]) == 1
                && historical_value_integer(&row[14]) == 1
                && historical_value_integer(&row[15]) == 1
        }
        _ => false,
    }
}

fn assert_historical_database_delta(
    before: &HistoricalDatabaseSnapshot,
    after: &HistoricalDatabaseSnapshot,
    intent: &IntentId,
    case: HistoricalProviderAttemptsCase,
) {
    assert_eq!(after.catalog, before.catalog, "{}: catalog", case.label());
    assert_eq!(
        after.tables.keys().collect::<Vec<_>>(),
        before.tables.keys().collect::<Vec<_>>(),
        "{}: table set",
        case.label(),
    );
    let target_table = match case {
        HistoricalProviderAttemptsCase::CapabilitiesRequestIdTamper => {
            "chain_post_close_macro_control_attempt_results"
        }
        HistoricalProviderAttemptsCase::ReadinessLinkTamper => {
            "chain_post_close_macro_attempt_begins"
        }
        _ => panic!("TEST_CODE historical delta requires a damage case"),
    };
    let changed_tables = before
        .tables
        .iter()
        .filter_map(|(name, rows)| {
            (after.tables.get(name).unwrap() != rows).then_some(name.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(changed_tables, vec![target_table], "{}: tables", case.label());

    let before_rows = &before.tables[target_table];
    let after_rows = &after.tables[target_table];
    assert_eq!(after_rows.len(), before_rows.len(), "{}: row count", case.label());
    let changed_rows = before_rows
        .iter()
        .zip(after_rows)
        .enumerate()
        .filter_map(|(index, (old, new))| (old != new).then_some(index))
        .collect::<Vec<_>>();
    assert_eq!(changed_rows.len(), 1, "{}: changed rows", case.label());
    let index = changed_rows[0];
    assert!(
        historical_target_row(&before_rows[index], intent, case),
        "{}: target row",
        case.label(),
    );
    let changed_columns = before_rows[index]
        .iter()
        .zip(&after_rows[index])
        .enumerate()
        .filter_map(|(column, (old, new))| (old != new).then_some(column))
        .collect::<Vec<_>>();
    match case {
        HistoricalProviderAttemptsCase::CapabilitiesRequestIdTamper => {
            assert!(
                changed_columns == vec![9, 11] || changed_columns == vec![9, 10, 11],
                "{}: control raw columns {changed_columns:?}",
                case.label(),
            );
        }
        HistoricalProviderAttemptsCase::ReadinessLinkTamper => {
            assert_eq!(changed_columns, vec![18], "{}: readiness column", case.label());
        }
        _ => unreachable!(),
    }
}

fn replace_historical_segment_once(
    bytes: &[u8],
    needle: &[u8],
    replacement: &[u8],
) -> Vec<u8> {
    assert!(!needle.is_empty());
    let offsets = bytes
        .windows(needle.len())
        .enumerate()
        .filter_map(|(offset, value)| (value == needle).then_some(offset))
        .collect::<Vec<_>>();
    assert_eq!(offsets.len(), 1);
    let offset = offsets[0];
    let mut changed =
        Vec::with_capacity(bytes.len() - needle.len() + replacement.len());
    changed.extend_from_slice(&bytes[..offset]);
    changed.extend_from_slice(replacement);
    changed.extend_from_slice(&bytes[offset + needle.len()..]);
    assert_ne!(changed, bytes);
    changed
}

fn historical_trigger_sql(
    transaction: &rusqlite::Transaction<'_>,
    name: &str,
) -> String {
    transaction
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

fn inject_historical_capabilities_request_id_tamper(
    database: &std::path::Path,
    intent: &IntentId,
    original_request_id: &str,
) -> (HistoricalDatabaseSnapshot, HistoricalDatabaseSnapshot) {
    let mut connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection.busy_timeout(Duration::from_millis(250)).unwrap();
    let before = historical_database_snapshot(&connection);
    let (original, original_length, original_digest): (Vec<u8>, i64, String) = connection
        .query_row(
            "SELECT bytes,byte_length,sha256 \
             FROM chain_post_close_macro_control_attempt_results \
             WHERE intent_id=?1 AND episode_ordinal=1 AND control_ordinal=2",
            [intent.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(original_length, i64::try_from(original.len()).unwrap());
    assert_eq!(original_digest, raw_digest(&original).as_str());

    let original_outer: macro_codec::ControlRawResult =
        macro_codec::decode(&original).unwrap();
    let original_response = original_outer.response_bytes().unwrap().to_vec();
    assert_eq!(original_response, expected_capabilities(original_request_id));
    let mut response = CapabilitiesResponse::decode(original_response.as_slice()).unwrap();
    assert_eq!(response.encode_to_vec(), original_response);
    assert_eq!(response.request_id, original_request_id);
    let replacement_request_id = "X".repeat(original_request_id.len());
    assert_ne!(replacement_request_id, original_request_id);
    response.request_id = replacement_request_id;
    let replacement_response = response.encode_to_vec();
    assert_eq!(
        CapabilitiesResponse::decode(replacement_response.as_slice())
            .unwrap()
            .encode_to_vec(),
        replacement_response,
    );

    let original_segment = serde_json::to_vec(&original_response).unwrap();
    let replacement_segment = serde_json::to_vec(&replacement_response).unwrap();
    let damaged_bytes = replace_historical_segment_once(
        &original,
        &original_segment,
        &replacement_segment,
    );
    let damaged_outer: macro_codec::ControlRawResult =
        macro_codec::decode(&damaged_bytes).unwrap();
    assert_eq!(
        damaged_outer.response_bytes(),
        Some(replacement_response.as_slice()),
    );
    assert_eq!(macro_codec::encode(&damaged_outer).unwrap(), damaged_bytes);
    let damaged_length = i64::try_from(damaged_bytes.len()).unwrap();
    let damaged_digest = raw_digest(&damaged_bytes).as_str().to_owned();

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let trigger_sql = historical_trigger_sql(
        &transaction,
        "chain_post_close_macro_control_attempt_results_update",
    );
    transaction
        .execute_batch("DROP TRIGGER chain_post_close_macro_control_attempt_results_update")
        .unwrap();
    assert_eq!(
        transaction
            .execute(
                "UPDATE chain_post_close_macro_control_attempt_results \
                 SET bytes=?1,byte_length=?2,sha256=?3 \
                 WHERE intent_id=?4 AND episode_ordinal=1 AND control_ordinal=2",
                params![
                    damaged_bytes,
                    damaged_length,
                    damaged_digest,
                    intent.as_str()
                ],
            )
            .unwrap(),
        1,
    );
    transaction.execute_batch(&trigger_sql).unwrap();
    transaction.commit().unwrap();

    let damaged = historical_database_snapshot(&connection);
    assert_historical_database_delta(
        &before,
        &damaged,
        intent,
        HistoricalProviderAttemptsCase::CapabilitiesRequestIdTamper,
    );
    connection.close().unwrap();
    (before, damaged)
}

fn inject_historical_readiness_link_tamper(
    database: &std::path::Path,
    intent: &IntentId,
) -> (HistoricalDatabaseSnapshot, HistoricalDatabaseSnapshot) {
    let mut connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    connection.busy_timeout(Duration::from_millis(250)).unwrap();
    let before = historical_database_snapshot(&connection);

    let controls = connection
        .prepare(
            "SELECT control_ordinal,kind,outcome,run_version \
             FROM chain_post_close_macro_control_attempt_results \
             WHERE intent_id=?1 ORDER BY control_ordinal",
        )
        .unwrap()
        .query_map([intent.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(controls.len(), 2);
    assert_eq!(
        (&controls[0].0, controls[0].1.as_str(), controls[0].2.as_str()),
        (&1, "Health", "Ready"),
    );
    assert_eq!(
        (&controls[1].0, controls[1].1.as_str(), controls[1].2.as_str()),
        (&2, "Capabilities", "Ready"),
    );
    let health_result_version = controls[0].3;
    let capabilities_result_version = controls[1].3;
    assert_ne!(health_result_version, capabilities_result_version);

    let (begin_bytes, begin_length, begin_digest, readiness_result_version):
        (Vec<u8>, i64, String, Option<i64>) = connection
        .query_row(
            "SELECT bytes,byte_length,sha256,readiness_result_version \
             FROM chain_post_close_macro_attempt_begins \
             WHERE intent_id=?1 AND phase='Gateway' AND item_ordinal=1 \
             AND candidate_ordinal=1 AND attempt_ordinal=1",
            [intent.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(begin_length, i64::try_from(begin_bytes.len()).unwrap());
    assert_eq!(begin_digest, raw_digest(&begin_bytes).as_str());
    assert_eq!(
        readiness_result_version,
        Some(capabilities_result_version),
    );
    let begin: crate::push_foundation::intent_store::chain_post_close::macro_stage::Begin =
        macro_codec::decode(&begin_bytes).unwrap();
    assert_eq!(macro_codec::encode(&begin).unwrap(), begin_bytes);
    let begin_json: serde_json::Value = serde_json::from_slice(&begin_bytes).unwrap();
    assert!(
        begin_json.get("readiness_result_version").is_none(),
        "legacy Begin JSON must not grow a synthetic readiness link",
    );

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let trigger_sql = historical_trigger_sql(
        &transaction,
        "chain_post_close_macro_attempt_begins_update",
    );
    transaction
        .execute_batch("DROP TRIGGER chain_post_close_macro_attempt_begins_update")
        .unwrap();
    assert_eq!(
        transaction
            .execute(
                "UPDATE chain_post_close_macro_attempt_begins \
                 SET readiness_result_version=?1 \
                 WHERE intent_id=?2 AND phase='Gateway' AND item_ordinal=1 \
                 AND candidate_ordinal=1 AND attempt_ordinal=1",
                params![health_result_version, intent.as_str()],
            )
            .unwrap(),
        1,
    );
    transaction.execute_batch(&trigger_sql).unwrap();
    transaction.commit().unwrap();

    let damaged = historical_database_snapshot(&connection);
    assert_historical_database_delta(
        &before,
        &damaged,
        intent,
        HistoricalProviderAttemptsCase::ReadinessLinkTamper,
    );
    let changed_begin = damaged.tables["chain_post_close_macro_attempt_begins"]
        .iter()
        .find(|row| {
            historical_target_row(
                row,
                intent,
                HistoricalProviderAttemptsCase::ReadinessLinkTamper,
            )
        })
        .unwrap();
    assert_eq!(
        historical_value_integer(&changed_begin[18]),
        health_result_version,
    );
    assert_eq!(
        &changed_begin[9..12],
        &before.tables["chain_post_close_macro_attempt_begins"]
            .iter()
            .find(|row| {
                historical_target_row(
                    row,
                    intent,
                    HistoricalProviderAttemptsCase::ReadinessLinkTamper,
                )
            })
            .unwrap()[9..12],
    );
    connection.close().unwrap();
    (before, damaged)
}

async fn run_historical_provider_attempts_case(case: HistoricalProviderAttemptsCase) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let baseline = control_tests::setup_external_parent(
                &mut business,
                &mut parent_server,
                case.run_id(),
            )
            .await;
            external_server = Some(
                ExternalMtlsMacroFixture::bind_data_provider_attempts_for_test(case.unpublished_provider())
                    .await
                    .expect("TEST_CODE historical attempts mTLS fixture"),
            );
            let external = external_server
                .as_ref()
                .expect("TEST_CODE historical attempts fixture owner");
            let checkpoint = control_recovery_tests::reach_confirmed_health_checkpoint(
                &mut business,
                &baseline,
                external,
                case.owner(),
            )
            .await;
            let database = business.database();
            let started_at = micros(STARTED_LOCAL);
            let prepared = crate::grpc_client::client::GrpcMarketClient::prepare_client_bundle(
                external.bundle_path(),
            )
            .expect("TEST_CODE historical attempts prepare bundle");

            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let lease = local
                .resume_run(
                    &baseline.intent,
                    macro_lease(
                        case.owner(),
                        started_at + 3_000_000,
                        started_at + 12_000_000,
                        checkpoint.head_version,
                    ),
                )
                .unwrap();
            let capabilities_material = {
                let recovery = local.inspect_macro(&baseline.intent).unwrap();
                let controls = recovery.readiness_episodes()[0].controls();
                assert_eq!(controls[0].outcome(), Some(MacroControlOutcome::Ready));
                assert_eq!(controls[1].begin_version(), None);
                controls[1].request_material()
            };
            let capabilities_attempt = prepared
                .resume_capabilities_attempt(capabilities_material)
                .unwrap();
            let (lease, capabilities_call) = local
                .begin_capabilities_control(
                    lease,
                    &capabilities_attempt,
                    UtcMicros::try_new(started_at + 4_000_000).unwrap(),
                )
                .unwrap();
            external.release_capabilities();
            let capabilities_completion =
                tokio::time::timeout(Duration::from_secs(5), capabilities_attempt.execute())
                    .await
                    .expect("TEST_CODE historical attempts Capabilities deadline");
            assert!(capabilities_completion.processed().is_ok());
            let (lease, outcome) = local
                .record_capabilities_control_result(
                    lease,
                    capabilities_call,
                    &capabilities_completion,
                    UtcMicros::try_new(started_at + 5_000_000).unwrap(),
                )
                .unwrap();
            assert_eq!(outcome, MacroControlOutcome::Ready);
            let connected = capabilities_completion
                .into_connected_client()
                .expect("TEST_CODE historical attempts Capabilities connected client");

            let ready = local.inspect_macro(&baseline.intent).unwrap();
            let ready_result_version = ready.readiness_episodes()[0]
                .ready_result_version()
                .unwrap();
            let authorized = prepared
                .resume_macro_query(
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Eastmoney,
                        limit: 20,
                    },
                    ready
                        .plan()
                        .first_source_request()
                        .restored_external(ready.plan().endpoint(), 1),
                )
                .unwrap();
            drop(ready);
            let (lease, call) = local
                .begin_prepared_macro_attempt(
                    lease,
                    &authorized,
                    UtcMicros::try_new(started_at + 6_000_000).unwrap(),
                )
                .unwrap();
            let connected_attempt = authorized
                .bind_connected(connected)
                .expect("TEST_CODE historical attempts bind Capabilities client");
            external.release_data();
            let completion = ExternalMacroAttemptCompletion::Unary(
                tokio::time::timeout(Duration::from_secs(5), connected_attempt.execute())
                    .await
                    .expect("TEST_CODE historical attempts data deadline"),
            );
            let ExternalMacroAttemptCompletion::Unary(unary) = &completion else {
                panic!("TEST_CODE historical attempts require unary status");
            };
            assert_eq!(
                unary.status_code,
                Some(tonic::Code::FailedPrecondition as i32)
            );
            assert!(unary.response_bytes.is_none());
            assert_eq!(
                unary.status_error_detail_trailer,
                crate::grpc_client::client::macro_attempt::MacroTrailerMaterial::Absent
            );
            let online_error = unary
                .processed
                .as_ref()
                .expect_err("TEST_CODE provider attempts status must reject the query");
            assert!(matches!(
                online_error,
                crate::grpc_client::errors::GrpcError::FailedPrecondition { .. }
            ));
            assert_eq!(
                online_error.details().reason_code.as_deref(),
                Some("invalid_evidence")
            );
            assert_eq!(online_error.details().retryable, Some(false));
            assert_eq!(unary.retry_decision, RetryDecision::NoRetry);
            assert_eq!(unary.continuation, MacroContinuation::Terminal);
            assert_historical_provider_attempts_for_case(
                &online_error.details().provider_attempts,
                case,
                "online same-endpoint catalog",
            );

            let status_details = unary
                .status_details
                .as_ref()
                .expect("TEST_CODE historical attempts raw status detail")
                .clone();
            let raw_detail = ErrorDetail::decode(status_details.as_slice()).unwrap();
            assert_eq!(raw_detail.encode_to_vec(), status_details);
            assert_eq!(raw_detail.request_id, checkpoint.data.id);
            assert_eq!(raw_detail.operation, Operation::GlobalNews as i32);
            assert_eq!(raw_detail.provider, "Eastmoney");
            assert_eq!(raw_detail.reason_code, "invalid_evidence");
            assert!(!raw_detail.retryable);
            assert_eq!(raw_detail.admission, AdmissionState::Admitted as i32);
            assert_eq!(
                raw_detail.provider_attempts,
                historical_wire_provider_attempts(case),
            );

            let _lease = local
                .record_external_macro_result(
                    lease,
                    call,
                    &completion,
                    UtcMicros::try_new(started_at + 7_000_000).unwrap(),
                )
                .unwrap();
            let raw = data_result_bytes(&database, &baseline.intent, 1);
            let recovered = local.inspect_macro(&baseline.intent).unwrap();
            assert!(!recovered.has_unconfirmed_effect());
            assert_eq!(recovered.attempts().len(), 1);
            let attempt = &recovered.attempts()[0];
            assert_eq!(attempt.readiness_result_version(), Some(ready_result_version));
            assert_eq!(attempt.continuation(), Some(MacroContinuation::Terminal));
            let material = attempt.result_material().unwrap();
            let recovered_diagnostic = material.diagnostic.map(str::to_owned);
            assert_eq!(
                material.diagnostic,
                Some("[redacted-unclassified-status]")
            );
            assert_eq!(material.retry_decision, RetryDecision::NoRetry);
            assert_eq!(material.continuation, MacroContinuation::Terminal);
            match material.wire {
                MacroRecoveredWire::Status {
                    code,
                    details,
                    trailer: MacroRecoveredTrailer::Absent,
                } => {
                    assert_eq!(code, tonic::Code::FailedPrecondition as i32);
                    assert_eq!(details, status_details);
                }
                _ => panic!("TEST_CODE historical attempts must retain raw status material"),
            }
            assert_historical_provider_attempts_for_case(
                material
                    .provider_attempts()
                    .expect("TEST_CODE writer recovery attempts observation"),
                case,
                "writer recovery",
            );

            let source = recovered
                .global_news(GlobalNewsProvider::Eastmoney)
                .unwrap();
            let final_bytes = source.final_bytes().unwrap().to_vec();
            let receipt = source.audit_receipt().unwrap().clone();
            let plan_bytes = recovered.plan_bytes().to_vec();
            let parent_final = recovered.parent_final_bytes().to_vec();
            drop(recovered);
            drop(local);

            let audit_snapshot = control_tests::audit_snapshot_at(&database);
            let facts = fact_snapshot_at(&database, &baseline.tables);
            let parent_network_before = parent_server
                .as_ref()
                .unwrap()
                .snapshot_with_tcp_for_test();
            let parent_membership_before = parent_server
                .as_ref()
                .unwrap()
                .membership_snapshot();
            let wire_before = external.snapshot();
            assert_wire_prefix(
                &wire_before,
                &checkpoint,
                &expected_capabilities(&checkpoint.capabilities.id),
            );
            assert_eq!(wire_before.tcp_accepts, 2);
            assert_eq!(wire_before.data_calls, 1);
            assert_eq!(wire_before.data_requests, vec![checkpoint.data.bytes.clone()]);
            assert_eq!(wire_before.data_authorized, vec![true]);
            assert!(wire_before.data_responses.is_empty());
            assert_eq!(wire_before.data_statuses.len(), 1);
            assert_eq!(
                wire_before.data_statuses[0].code,
                tonic::Code::FailedPrecondition as i32
            );
            assert_eq!(wire_before.data_statuses[0].details, status_details);
            assert_eq!(
                wire_before.data_statuses[0].trailer,
                ObservedHealthTrailer::Absent
            );

            if case.damages_history() {
                let mut held_local = business
                    .store
                    .as_mut()
                    .unwrap()
                    .single_user_local_chain_post_close(&baseline.config)
                    .unwrap();
                let held_legal = held_local.inspect_macro(&baseline.intent).unwrap();
                assert_historical_provider_attempts(
                    held_legal.attempts()[0]
                        .result_material()
                        .unwrap()
                        .provider_attempts()
                        .expect("TEST_CODE held legal historical attempts"),
                    "held legal history before damage",
                );
                drop(held_legal);

                external.set_reject_new_connections_for_test(true);
                let (before_damage, damaged) = match case {
                    HistoricalProviderAttemptsCase::CapabilitiesRequestIdTamper => {
                        inject_historical_capabilities_request_id_tamper(
                            &database,
                            &baseline.intent,
                            &checkpoint.capabilities.id,
                        )
                    }
                    HistoricalProviderAttemptsCase::ReadinessLinkTamper => {
                        inject_historical_readiness_link_tamper(
                            &database,
                            &baseline.intent,
                        )
                    }
                    _ => unreachable!(),
                };
                assert_ne!(damaged, before_damage);
                assert!(matches!(
                    held_local.inspect_macro(&baseline.intent),
                    Err(ChainPostCloseError::SchemaRejected)
                ));
                assert_eq!(historical_snapshot_at(&database), damaged);
                assert_eq!(external.snapshot(), wire_before);
                assert_eq!(
                    parent_server
                        .as_ref()
                        .unwrap()
                        .snapshot_with_tcp_for_test(),
                    parent_network_before
                );
                assert_eq!(
                    parent_server.as_ref().unwrap().membership_snapshot(),
                    parent_membership_before
                );
                drop(held_local);

                business.reopen();
                assert!(matches!(
                    business
                        .store
                        .as_mut()
                        .unwrap()
                        .single_user_local_chain_post_close(&baseline.config),
                    Err(ChainPostCloseError::SchemaRejected)
                ));
                assert_eq!(historical_snapshot_at(&database), damaged);
                assert_eq!(external.snapshot(), wire_before);
                assert_eq!(
                    parent_server
                        .as_ref()
                        .unwrap()
                        .snapshot_with_tcp_for_test(),
                    parent_network_before
                );
                assert_eq!(
                    parent_server.as_ref().unwrap().membership_snapshot(),
                    parent_membership_before
                );
                return;
            }

            external.set_reject_new_connections_for_test(true);
            business.reopen();
            let mut reopened_local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&baseline.config)
                .unwrap();
            let reopened = reopened_local.inspect_macro(&baseline.intent).unwrap();
            assert_eq!(reopened.plan_bytes(), plan_bytes);
            assert_eq!(reopened.parent_final_bytes(), parent_final);
            assert_eq!(reopened.attempts().len(), 1);
            let reopened_attempt = &reopened.attempts()[0];
            assert_eq!(
                reopened_attempt.readiness_result_version(),
                Some(ready_result_version)
            );
            assert_eq!(
                reopened_attempt.continuation(),
                Some(MacroContinuation::Terminal)
            );
            let reopened_material = reopened_attempt.result_material().unwrap();
            assert_eq!(
                reopened_material.diagnostic,
                recovered_diagnostic.as_deref()
            );
            assert_eq!(
                reopened_material.retry_decision,
                RetryDecision::NoRetry
            );
            assert_eq!(
                reopened_material.continuation,
                MacroContinuation::Terminal
            );
            match reopened_material.wire {
                MacroRecoveredWire::Status {
                    code,
                    details,
                    trailer: MacroRecoveredTrailer::Absent,
                } => {
                    assert_eq!(code, tonic::Code::FailedPrecondition as i32);
                    assert_eq!(details, status_details);
                }
                _ => panic!("TEST_CODE reopened attempts must retain raw status material"),
            }
            assert_historical_provider_attempts_for_case(
                reopened_material
                    .provider_attempts()
                    .expect("TEST_CODE reopened attempts observation"),
                case,
                "true reopen recovery",
            );
            let reopened_source = reopened
                .global_news(GlobalNewsProvider::Eastmoney)
                .unwrap();
            assert_eq!(reopened_source.final_bytes(), Some(final_bytes.as_slice()));
            assert_eq!(reopened_source.audit_receipt(), Some(&receipt));
            drop(reopened);
            drop(reopened_local);

            assert_eq!(data_result_bytes(&database, &baseline.intent, 1), raw);
            assert_eq!(control_tests::audit_snapshot_at(&database), audit_snapshot);
            assert_eq!(fact_snapshot_at(&database, &baseline.tables), facts);
            assert_eq!(external.snapshot(), wire_before);
            assert_eq!(
                parent_server
                    .as_ref()
                    .unwrap()
                    .snapshot_with_tcp_for_test(),
                parent_network_before
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                parent_membership_before
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
        Ok(result) => result.unwrap_or_else(|_| {
            panic!("TEST_CODE {} body timeout", case.label())
        }),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}


#[tokio::test]
async fn single_user_external_macro_historical_provider_attempts_survive_true_reopen() {
    run_historical_provider_attempts_case(HistoricalProviderAttemptsCase::Published).await;
}

#[tokio::test]
async fn single_user_external_macro_unpublished_provider_attempts_reopen_as_rejected() {
    run_historical_provider_attempts_case(HistoricalProviderAttemptsCase::Unpublished).await;
}

#[tokio::test]
async fn single_user_external_macro_historical_capabilities_request_id_tamper_rejects_without_rpc() {
    run_historical_provider_attempts_case(
        HistoricalProviderAttemptsCase::CapabilitiesRequestIdTamper,
    )
    .await;
}

#[tokio::test]
async fn single_user_external_macro_historical_readiness_link_tamper_rejects_without_rpc() {
    run_historical_provider_attempts_case(HistoricalProviderAttemptsCase::ReadinessLinkTamper)
        .await;
}

#[path = "chain_post_close_macro_external_v12_tests.rs"]
mod v12_tests;
