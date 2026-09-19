use super::board_loopback_fixture::{
    clone_with_invalid_instance_bearer, clone_with_retry_policy,
    spawn_macro_envelope_failure_loopback, spawn_macro_external_shape_loopback,
    spawn_macro_fixed_status_loopback, spawn_macro_local_shape_loopback, spawn_macro_loopback,
    spawn_macro_raw_status_loopback, spawn_macro_repeat_retry_loopback, MacroEnvelopeCase,
    MacroExternalSourceCase, MacroRawStatusCase,
};
use super::macro_attempt::{
    MacroContinuation, MacroQueryIdentity, MacroTrailerMaterial, RestoredMacroRequest,
};
use super::ContractProfile;
use crate::data_gateway::GlobalNewsProvider;
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::external_pb::magic::market::v1::{
    QueryRequest as ExternalQueryRequest, QueryResponse as ExternalQueryResponse,
};
use crate::grpc_client::pb::magic::market::v1::{
    AdmissionState, ErrorDetail, Operation, QueryRequest, QueryResponse,
};
use crate::grpc_client::retry::RetryDecision;
use futures::FutureExt as _;
use prost::Message as _;
use std::time::Duration;

fn clone_external_with_authority(
    base: &super::GrpcMarketClient,
    authority: Option<&str>,
) -> super::GrpcMarketClient {
    assert_eq!(base.profile, ContractProfile::ExternalV1);
    assert!(matches!(&base.data, super::DataTransport::External(_)));
    let mut client = base.clone();
    assert!(matches!(
        &client.authorization,
        super::ClientAuthorization::InstanceBearer(_)
    ));
    client.acquisition_authority = authority.map(str::to_owned);
    client
}

fn clone_with_restore_profile_mismatch_only(
    base: &super::GrpcMarketClient,
) -> super::GrpcMarketClient {
    assert_eq!(base.profile, ContractProfile::ExternalV1);
    assert!(matches!(&base.data, super::DataTransport::External(_)));
    let mut client = base.clone();
    // Deliberately inconsistent only to prove resume rejects before authorize/execute.
    client.profile = ContractProfile::LocalBridgeV1;
    client
}

fn assert_external_macro_request_shape(
    attempt: &super::macro_attempt::AuthorizedMacroAttempt,
    provider: &str,
) {
    assert_eq!(attempt.profile(), "ExternalV1");
    assert_eq!(attempt.retry_policy(), (4, 1000, 60_000, 200));
    let bytes = attempt.request_bytes();
    let synthetic_bearer = b"TEST_CODE_BOARD_LOOPBACK_TOKEN";
    assert!(!bytes
        .windows(synthetic_bearer.len())
        .any(|part| part == synthetic_bearer));
    let request = ExternalQueryRequest::decode(bytes.as_slice())
        .expect("TEST_CODE native External wire request");
    let context = request.context.expect("TEST_CODE External context");
    assert_eq!(context.protocol_version, 1);
    assert_eq!(context.request_id, attempt.request_id());
    assert!(!context.request_id.is_empty());
    assert_eq!(request.preferred_provider, provider);
    assert!(!request.allow_unadmitted);
    let payload = request.payload.expect("TEST_CODE External payload");
    assert_eq!(payload.schema, "magic.market.global_news.request");
    assert_eq!(payload.schema_version, 2);
    assert_eq!(payload.content_type, "application/json; charset=utf-8");
    assert_eq!(payload.data, br#"{"limit":20}"#);
}

fn assert_external_macro_transport_response(
    completion: &super::macro_attempt::MacroAttemptCompletion,
    sent_bytes: &[u8],
    request_id: &str,
    provider: &str,
    source: &str,
) {
    assert_eq!(completion.response_bytes.as_deref(), Some(sent_bytes));
    assert_eq!(completion.status_code, None);
    assert_eq!(completion.status_details, None);
    assert_eq!(
        completion.status_error_detail_trailer,
        MacroTrailerMaterial::Absent
    );
    assert_eq!(completion.retry_decision, RetryDecision::NoRetry);
    assert_eq!(completion.continuation, MacroContinuation::Terminal);
    let evidence = completion
        .external_wire
        .as_ref()
        .expect("TEST_CODE External raw response evidence");
    evidence
        .validate(crate::grpc_client::external_query_transport::ExternalQueryMethod::GlobalNews)
        .expect("TEST_CODE valid External GlobalNews evidence");
    assert_eq!(evidence.payload(), Some(sent_bytes));
    let raw = ExternalQueryResponse::decode(completion.response_bytes.as_deref().unwrap())
        .expect("TEST_CODE generated External raw response");
    assert_eq!(raw.request_id, request_id);
    assert_eq!(raw.operation, Operation::GlobalNews as i32);
    assert_eq!(raw.selected_provider, provider);
    assert_eq!(raw.batch_id, "TEST_CODE_EXTERNAL_MACRO_BATCH");
    assert_eq!(raw.observed_at, "2026-09-14T15:31:00+08:00");
    assert_eq!(raw.source_at, "2026-09-14T15:30:00+08:00");
    assert_eq!(raw.admission, AdmissionState::Admitted as i32);
    assert!(raw.complete);
    assert!(raw.records.is_empty());
    assert!(raw.diagnostic_blocker.is_empty());
    let auditable_wire =
        QueryResponse::decode(sent_bytes).expect("TEST_CODE fixture field11 audit decode");
    assert_eq!(auditable_wire.source, source);
}

#[tokio::test]
async fn macro_external_global_news_schema_v2_preserves_route_authority_and_restore_identity() {
    use crate::data_gateway::GeneralWebResearchProvider as Web;
    const AUTHORITY: &str = "grpc-mtls:TEST_CODE-macro.external";
    const OTHER_AUTHORITY: &str = "grpc-mtls:TEST_CODE-other.external";

    for source_case in [
        MacroExternalSourceCase::Empty,
        MacroExternalSourceCase::Conflict,
    ] {
        let (base, server) = spawn_macro_external_shape_loopback(source_case, AUTHORITY).await;
        let body_limit = if source_case == MacroExternalSourceCase::Empty {
            30
        } else {
            15
        };
        let outcome = std::panic::AssertUnwindSafe(tokio::time::timeout(
            Duration::from_secs(body_limit),
            async {
                assert_eq!(base.profile, ContractProfile::ExternalV1);
                assert!(matches!(&base.data, super::DataTransport::External(_)));
                assert_eq!(base.acquisition_authority.as_deref(), Some(AUTHORITY));
                let client = base.clone();
                if source_case == MacroExternalSourceCase::Empty {
                    for limit in [0, 21] {
                        let error = client
                            .macro_query(MacroQueryIdentity::GlobalNews {
                                provider: GlobalNewsProvider::Jin10,
                                limit,
                            })
                            .err()
                            .expect("TEST_CODE External rejects limit");
                        assert!(matches!(error, GrpcError::InvalidArgument { .. }));
                    }
                    for identity in [
                        MacroQueryIdentity::EconomicCalendar,
                        MacroQueryIdentity::SemanticSearch {
                            provider: Web::Bocha,
                            query: "TEST_CODE external unsupported".into(),
                            limit: 3,
                        },
                    ] {
                        let error = client
                            .macro_query(identity)
                            .err()
                            .expect("TEST_CODE reject undelivered External operation");
                        assert!(matches!(error, GrpcError::Unimplemented { .. }));
                    }
                    assert!(server.macro_snapshot().requests.is_empty());
                }
                let (provider, provider_wire) = if source_case == MacroExternalSourceCase::Empty {
                    (GlobalNewsProvider::Eastmoney, "Eastmoney")
                } else {
                    (GlobalNewsProvider::Jin10, "Jin10")
                };
                let identity = MacroQueryIdentity::GlobalNews {
                    provider,
                    limit: 20,
                };
                let first = client
                    .macro_query(identity.clone())
                    .expect("TEST_CODE External session")
                    .authorize_next()
                    .expect("TEST_CODE External authorization");
                assert_external_macro_request_shape(&first, provider_wire);
                assert_eq!(first.acquisition_authority(), Some(AUTHORITY));
                let request_id = first.request_id().to_owned();
                let request_bytes = first.request_bytes();
                let material = || RestoredMacroRequest {
                    request_bytes: request_bytes.clone(),
                    request_id: request_id.clone(),
                    profile: ContractProfile::ExternalV1,
                    acquisition_authority: Some(AUTHORITY.into()),
                    retry_policy: (4, 1000, 60_000, 200),
                    next_attempt: 2,
                };
                let first = tokio::time::timeout(Duration::from_secs(5), first.execute())
                    .await
                    .expect("TEST_CODE External first RPC timeout");
                assert_eq!(first.response_bytes, None);
                assert_eq!(first.status_code, Some(tonic::Code::Unavailable as i32));
                assert_eq!(first.retry_decision, RetryDecision::RetryBackoff);
                assert_eq!(
                    first.continuation,
                    MacroContinuation::Retry { backoff_ms: 1000 }
                );
                let observed = server.macro_snapshot();
                assert_eq!(observed.requests, vec![(request_bytes.clone(), true)]);
                assert_eq!(observed.routed_operations, vec![Operation::GlobalNews]);
                assert_eq!(
                    first.status_details.as_deref(),
                    Some(observed.retry_error_details[0].as_slice())
                );
                assert_eq!(
                    first.status_error_detail_trailer,
                    MacroTrailerMaterial::Bytes(observed.retry_error_details[0].clone())
                );
                let error = first.processed.expect_err("TEST_CODE External first retry");
                assert!(matches!(&error, GrpcError::Unavailable { .. }));
                assert_eq!(error.details().provider.as_deref(), Some(provider_wire));
                assert_eq!(
                    error.details().reason_code.as_deref(),
                    Some("no_verified_batch")
                );
                assert_eq!(error.details().retryable, Some(true));

                for (profile, authority) in [
                    (ContractProfile::LocalBridgeV1, Some(AUTHORITY)),
                    (ContractProfile::ExternalV1, Some(OTHER_AUTHORITY)),
                    (ContractProfile::ExternalV1, None),
                ] {
                    let mut saved = material();
                    saved.profile = profile;
                    saved.acquisition_authority = authority.map(str::to_owned);
                    let error = client
                        .resume_macro_query(identity.clone(), saved)
                        .err()
                        .expect("TEST_CODE reject saved External profile/authority drift");
                    assert!(matches!(error, GrpcError::FailedPrecondition { .. }));
                    assert_eq!(server.macro_snapshot().requests.len(), 1);
                }
                for changed_identity in [
                    MacroQueryIdentity::GlobalNews {
                        provider: if provider == GlobalNewsProvider::Eastmoney {
                            GlobalNewsProvider::Jin10
                        } else {
                            GlobalNewsProvider::Eastmoney
                        },
                        limit: 20,
                    },
                    MacroQueryIdentity::GlobalNews {
                        provider,
                        limit: 19,
                    },
                ] {
                    let error = client
                        .resume_macro_query(changed_identity, material())
                        .err()
                        .expect("TEST_CODE reject External route/limit drift");
                    assert!(matches!(error, GrpcError::FailedPrecondition { .. }));
                    assert_eq!(server.macro_snapshot().requests.len(), 1);
                }
                let requests_before_changed_current = server.macro_snapshot().requests.len();
                for changed_current in [
                    clone_with_restore_profile_mismatch_only(&base),
                    clone_external_with_authority(&base, Some(OTHER_AUTHORITY)),
                    clone_external_with_authority(&base, None),
                ] {
                    let error = changed_current
                        .resume_macro_query(identity.clone(), material())
                        .err()
                        .expect("TEST_CODE reject changed current profile/authority");
                    assert!(matches!(error, GrpcError::FailedPrecondition { .. }));
                    assert_eq!(
                        server.macro_snapshot().requests.len(),
                        requests_before_changed_current,
                        "restore mismatch must reject before authorize/execute",
                    );
                }
                let second = client
                    .resume_macro_query(identity, material())
                    .expect("TEST_CODE exact External restore")
                    .authorize_next()
                    .expect("TEST_CODE External reauthorization");
                assert_external_macro_request_shape(&second, provider_wire);
                assert_eq!(second.attempt_ordinal(), 2);
                assert_eq!(second.request_bytes(), request_bytes);
                assert_eq!(second.request_id(), request_id);
                assert_eq!(second.acquisition_authority(), Some(AUTHORITY));
                let completed = tokio::time::timeout(Duration::from_secs(5), second.execute())
                    .await
                    .expect("TEST_CODE External restored RPC timeout");
                let observation = server.macro_snapshot();
                assert_eq!(
                    observation.requests,
                    vec![(request_bytes.clone(), true), (request_bytes, true)]
                );
                assert_eq!(
                    observation.routed_operations,
                    vec![Operation::GlobalNews, Operation::GlobalNews]
                );
                let raw_source = if source_case == MacroExternalSourceCase::Conflict {
                    "TEST_CODE_FORBIDDEN_REMOTE_SOURCE"
                } else {
                    ""
                };
                assert_external_macro_transport_response(
                    &completed,
                    &observation.response_bytes[0],
                    &request_id,
                    provider_wire,
                    raw_source,
                );
                if source_case == MacroExternalSourceCase::Conflict {
                    let error = completed
                        .processed
                        .expect_err("TEST_CODE reject remote source conflict");
                    assert!(matches!(&error, GrpcError::FailedPrecondition { .. }));
                    assert_eq!(error.details().code, "external_source_field_conflict");
                    assert_eq!(
                        error.details().reason_code.as_deref(),
                        Some("external_source_field_conflict")
                    );
                    assert_eq!(error.details().retryable, Some(false));
                } else {
                    let result = completed
                        .processed
                        .expect("TEST_CODE inject current authority");
                    assert_eq!(result.source(), AUTHORITY);
                    assert!(matches!(
                        &result.provenance,
                        crate::grpc_client::envelope::AcquisitionProvenance::ExternalMtlsAuthority(
                            authority
                        ) if authority.as_str() == AUTHORITY
                    ));
                    assert_eq!(result.admission, AdmissionState::Admitted);
                    assert!(result.complete);
                    for (provider, wire) in [
                        (GlobalNewsProvider::Cailianpress, "Cailianpress"),
                        (GlobalNewsProvider::Jin10, "Jin10"),
                        (GlobalNewsProvider::ThePaper, "ThePaper"),
                    ] {
                        let attempt = client
                            .macro_query(MacroQueryIdentity::GlobalNews {
                                provider,
                                limit: 20,
                            })
                            .expect("TEST_CODE other External provider")
                            .authorize_next()
                            .expect("TEST_CODE other provider auth");
                        assert_external_macro_request_shape(&attempt, wire);
                        assert_eq!(attempt.acquisition_authority(), Some(AUTHORITY));
                        let bytes = attempt.request_bytes();
                        let id = attempt.request_id().to_owned();
                        let completion =
                            tokio::time::timeout(Duration::from_secs(5), attempt.execute())
                                .await
                                .expect("TEST_CODE other provider RPC timeout");
                        let observed = server.macro_snapshot();
                        assert_eq!(observed.requests.last(), Some(&(bytes, true)));
                        assert_external_macro_transport_response(
                            &completion,
                            observed.response_bytes.last().unwrap(),
                            &id,
                            wire,
                            "",
                        );
                        let result = completion
                            .processed
                            .expect("TEST_CODE other provider authority");
                        assert_eq!(result.source(), AUTHORITY);
                        assert!(matches!(
                            &result.provenance,
                            crate::grpc_client::envelope::AcquisitionProvenance::ExternalMtlsAuthority(
                                authority
                            ) if authority.as_str() == AUTHORITY
                        ));
                        assert_eq!(result.selected_provider, wire);
                    }
                    let missing = clone_external_with_authority(&base, None);
                    let attempt = missing
                        .macro_query(MacroQueryIdentity::GlobalNews {
                            provider: GlobalNewsProvider::Jin10,
                            limit: 20,
                        })
                        .expect("TEST_CODE missing authority session")
                        .authorize_next()
                        .expect("TEST_CODE missing authority auth");
                    assert_external_macro_request_shape(&attempt, "Jin10");
                    assert_eq!(attempt.acquisition_authority(), None);
                    let id = attempt.request_id().to_owned();
                    let completion =
                        tokio::time::timeout(Duration::from_secs(5), attempt.execute())
                            .await
                            .expect("TEST_CODE missing authority RPC timeout");
                    let observation = server.macro_snapshot();
                    assert_external_macro_transport_response(
                        &completion,
                        observation.response_bytes.last().unwrap(),
                        &id,
                        "Jin10",
                        "",
                    );
                    let error = completion
                        .processed
                        .expect_err("TEST_CODE reject missing authority");
                    assert!(matches!(&error, GrpcError::FailedPrecondition { .. }));
                    assert_eq!(
                        error.details().code,
                        "external_acquisition_authority_missing"
                    );
                    assert_eq!(
                        error.details().reason_code.as_deref(),
                        Some("external_acquisition_authority_missing")
                    );
                    assert_eq!(error.details().retryable, Some(false));
                    assert_eq!(observation.requests.len(), 6);
                    assert!(observation
                        .requests
                        .iter()
                        .all(|(_, authorized)| *authorized));
                    assert_eq!(
                        observation.routed_operations,
                        vec![Operation::GlobalNews; 6]
                    );
                    drop(missing);
                }
                assert_eq!(server.snapshot().non_board_requests, 0);
                drop(client);
            },
        ))
        .catch_unwind()
        .await;
        drop(base);
        server.finish().await;
        match outcome {
            Ok(result) => result.expect("TEST_CODE External profile body timeout"),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

#[tokio::test]
async fn macro_attempt_resume_preserves_request_and_original_response() {
    let (client, server) = spawn_macro_loopback().await;
    // Catch every body panic, including request construction and assertions;
    // join the owned server before reporting body failure or timeout.
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(15), async {
            let identity = MacroQueryIdentity::GlobalNews {
                provider: GlobalNewsProvider::Jin10,
                limit: 20,
            };
            let attempt = client
                .macro_query(identity.clone())
                .expect("TEST_CODE macro session")
                .authorize_next()
                .expect("TEST_CODE macro authorize");
            let request_bytes = attempt.request_bytes();
            let request_id = attempt.request_id().to_owned();
            assert_eq!(attempt.attempt_ordinal(), 1);
            assert_eq!(attempt.profile(), "LocalBridgeV1");
            assert_eq!(attempt.acquisition_authority(), None);
            assert_eq!(attempt.retry_policy(), (4, 1000, 60_000, 200));
            let restored = RestoredMacroRequest {
                request_bytes: request_bytes.clone(),
                request_id: request_id.clone(),
                profile: ContractProfile::LocalBridgeV1,
                acquisition_authority: None,
                retry_policy: attempt.retry_policy(),
                next_attempt: 2,
            };
            let request =
                QueryRequest::decode(request_bytes.as_slice()).expect("TEST_CODE decode request");
            let context = request.context.expect("TEST_CODE request context");
            assert_eq!(context.protocol_version, 1);
            assert_eq!(context.request_id, request_id);
            assert!(!request_id.is_empty());
            assert!(request.preferred_provider.is_empty());
            assert!(!request.allow_unadmitted);
            let payload = request.payload.expect("TEST_CODE request payload");
            assert_eq!(payload.schema, "news.global_news");
            assert_eq!(payload.schema_version, 1);
            assert_eq!(payload.content_type, "application/json; charset=utf-8");
            assert_eq!(payload.data, br#"{"limit":20,"provider":"Jin10"}"#);
            let first = tokio::time::timeout(Duration::from_secs(5), attempt.execute())
                .await
                .expect("TEST_CODE first RPC timeout");
            let first_observation = server.macro_snapshot();
            assert_eq!(first_observation.requests.len(), 1);
            assert_eq!(first.response_bytes, None);
            assert_eq!(first.status_code, Some(tonic::Code::Unavailable as i32));
            assert_eq!(
                first.status_details.as_deref(),
                Some(first_observation.retry_error_details[0].as_slice())
            );
            assert_eq!(
                first.status_error_detail_trailer,
                MacroTrailerMaterial::Bytes(first_observation.retry_error_details[0].clone())
            );
            let detail = ErrorDetail::decode(first_observation.retry_error_details[0].as_slice())
                .expect("TEST_CODE raw status detail");
            assert_eq!(detail.request_id, request_id);
            assert_eq!(detail.operation, Operation::GlobalNews as i32);
            assert_eq!(detail.provider, "Jin10");
            assert_eq!(detail.reason_code, "no_verified_batch");
            assert!(detail.retryable);
            let error = first.processed.expect_err("TEST_CODE retryable status");
            assert_eq!(error.details().provider.as_deref(), Some("Jin10"));
            assert_eq!(
                error.details().reason_code.as_deref(),
                Some("no_verified_batch")
            );
            assert_eq!(error.details().retryable, Some(true));
            assert_eq!(first.retry_decision, RetryDecision::RetryBackoff);
            assert_eq!(
                first.continuation,
                MacroContinuation::Retry { backoff_ms: 1000 }
            );

            // The durable caller will confirm result before this explicit restore.
            let second_attempt = client
                .resume_macro_query(identity, restored)
                .expect("TEST_CODE restore authorized retry")
                .authorize_next()
                .expect("TEST_CODE reattach current authorization");
            assert_eq!(second_attempt.attempt_ordinal(), 2);
            assert_eq!(second_attempt.request_id(), request_id);
            assert_eq!(second_attempt.request_bytes(), request_bytes);
            let second = tokio::time::timeout(Duration::from_secs(5), second_attempt.execute())
                .await
                .expect("TEST_CODE second RPC timeout");
            let observation = server.macro_snapshot();
            assert_eq!(observation.requests.len(), 2);
            assert!(observation
                .requests
                .iter()
                .all(|(_, authorized)| *authorized));
            assert_eq!(observation.requests[0].0, request_bytes);
            assert_eq!(observation.requests[1].0, request_bytes);
            assert_eq!(
                second.response_bytes.as_deref(),
                Some(observation.response_bytes[0].as_slice())
            );
            assert_eq!(second.status_code, None);
            assert_eq!(second.status_details, None);
            assert_eq!(
                second.status_error_detail_trailer,
                MacroTrailerMaterial::Absent
            );
            assert_eq!(second.retry_decision, RetryDecision::NoRetry);
            assert_eq!(second.continuation, MacroContinuation::Terminal);
            let raw = QueryResponse::decode(second.response_bytes.as_deref().unwrap())
                .expect("TEST_CODE original response");
            assert_eq!(raw.request_id, request_id);
            assert_eq!(raw.operation, Operation::GlobalNews as i32);
            assert_eq!(
                raw.records[0].data,
                br#"[{"title":"TEST_CODE_MACRO_NEWS"}]"#
            );
            let processed = second.processed.expect("TEST_CODE admitted response");
            assert_eq!(processed.admission, AdmissionState::Admitted);
            assert_eq!(processed.batch_id, "TEST_CODE_MACRO_BATCH");
            assert_eq!(processed.selected_provider, "Jin10");
            assert_eq!(processed.source(), "jin10-flash-v1");
            assert_eq!(processed.records, raw.records);
            assert!(processed.complete);
            assert_eq!(server.snapshot().non_board_requests, 0);
        }))
        .catch_unwind()
        .await;
    drop(client);
    server.finish().await;
    match outcome {
        Ok(result) => result.expect("TEST_CODE macro body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn macro_restore_rejects_changed_identity_and_invalid_current_auth_before_rpc() {
    let (client, server) = spawn_macro_loopback().await;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(15), async {
            let identity = MacroQueryIdentity::GlobalNews {
                provider: GlobalNewsProvider::Jin10,
                limit: 20,
            };
            let first_attempt = client
                .macro_query(identity.clone())
                .expect("TEST_CODE macro session")
                .authorize_next()
                .expect("TEST_CODE first authorization");
            let request_bytes = first_attempt.request_bytes();
            let request_id = first_attempt.request_id().to_owned();
            let retry_policy = first_attempt.retry_policy();
            let original =
                QueryRequest::decode(request_bytes.as_slice()).expect("TEST_CODE original request");
            let material = || RestoredMacroRequest {
                request_bytes: request_bytes.clone(),
                request_id: request_id.clone(),
                profile: ContractProfile::LocalBridgeV1,
                acquisition_authority: None,
                retry_policy,
                next_attempt: 2,
            };
            let first = tokio::time::timeout(Duration::from_secs(5), first_attempt.execute())
                .await
                .expect("TEST_CODE first RPC timeout");
            assert_eq!(
                first.continuation,
                MacroContinuation::Retry { backoff_ms: 1000 }
            );
            assert_eq!(first.retry_decision, RetryDecision::RetryBackoff);
            assert!(matches!(
                first.processed,
                Err(GrpcError::Unavailable { .. })
            ));
            assert_eq!(server.macro_snapshot().requests.len(), 1);

            let mut cases = vec![
                (
                    "identity provider",
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Cailianpress,
                        limit: 20,
                    },
                    material(),
                ),
                (
                    "identity operation",
                    MacroQueryIdentity::EconomicCalendar,
                    material(),
                ),
                (
                    "identity limit",
                    MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Jin10,
                        limit: 19,
                    },
                    material(),
                ),
            ];
            // Corrupt decoded public wire fields, then encode canonical protobuf:
            // these cases must reach the exact request-contract checks, not merely
            // fail protobuf decoding.
            let mutations: &[(&str, fn(&mut QueryRequest))] = &[
                ("wire request ID", |r| {
                    r.context.as_mut().unwrap().request_id = "TEST_CODE_DIFFERENT_REQUEST".into()
                }),
                ("protocol", |r| {
                    r.context.as_mut().unwrap().protocol_version = 2
                }),
                ("schema", |r| {
                    r.payload.as_mut().unwrap().schema = "market.economic_calendar".into()
                }),
                ("schema version", |r| {
                    r.payload.as_mut().unwrap().schema_version = 2
                }),
                ("content type", |r| {
                    r.payload.as_mut().unwrap().content_type = "application/json".into()
                }),
                ("preferred provider", |r| {
                    r.preferred_provider = "Jin10".into()
                }),
                ("allow unadmitted", |r| r.allow_unadmitted = true),
                ("missing context", |r| r.context = None),
                ("missing payload", |r| r.payload = None),
                ("payload provider", |r| {
                    r.payload.as_mut().unwrap().data =
                        br#"{"limit":20,"provider":"Cailianpress"}"#.to_vec()
                }),
                ("payload limit", |r| {
                    r.payload.as_mut().unwrap().data =
                        br#"{"limit":19,"provider":"Jin10"}"#.to_vec()
                }),
                ("noncanonical JSON", |r| {
                    r.payload.as_mut().unwrap().data =
                        br#"{ "limit":20,"provider":"Jin10"}"#.to_vec()
                }),
                ("invalid JSON", |r| {
                    r.payload.as_mut().unwrap().data = b"{".to_vec()
                }),
            ];
            for (label, mutate) in mutations {
                let mut request = original.clone();
                mutate(&mut request);
                let mut restored = material();
                restored.request_bytes = request.encode_to_vec();
                cases.push((*label, identity.clone(), restored));
            }
            let saved_mutations: &[(&str, fn(&mut RestoredMacroRequest))] = &[
                ("saved request ID", |r| {
                    r.request_id = "TEST_CODE_DIFFERENT_SAVED_ID".into()
                }),
                ("empty saved request ID", |r| r.request_id.clear()),
                ("profile", |r| r.profile = ContractProfile::ExternalV1),
                ("authority", |r| {
                    r.acquisition_authority = Some("grpc-mtls:TEST_CODE_OTHER_AUTHORITY".into())
                }),
                ("ordinal zero", |r| r.next_attempt = 0),
                ("ordinal exceeds budget", |r| r.next_attempt = 5),
                ("truncated protobuf", |r| {
                    r.request_bytes.pop();
                }),
                ("unknown protobuf field", |r| {
                    r.request_bytes.extend_from_slice(&[0xf8, 0x07, 0x01])
                }),
                ("duplicate protobuf fields", |r| {
                    let duplicate = r.request_bytes.clone();
                    r.request_bytes.extend_from_slice(&duplicate);
                }),
            ];
            for (label, mutate) in saved_mutations {
                let mut restored = material();
                mutate(&mut restored);
                cases.push((*label, identity.clone(), restored));
            }
            for (label, changed_identity, restored) in cases {
                let error = client
                    .resume_macro_query(changed_identity, restored)
                    .err()
                    .unwrap_or_else(|| panic!("TEST_CODE accepted corrupt restore: {label}"));
                assert!(
                    matches!(error, GrpcError::FailedPrecondition { .. }),
                    "{label}: {error:?}"
                );
                assert_eq!(
                    server.macro_snapshot().requests.len(),
                    1,
                    "{label} sent an RPC"
                );
            }

            // The persisted material carries no credential. Valid original bytes
            // must be authorized with this current instance, which is now invalid.
            let invalid_client = clone_with_invalid_instance_bearer(&client);
            let restored = invalid_client
                .resume_macro_query(identity.clone(), material())
                .expect("TEST_CODE valid identity restores before authorization");
            let error = restored
                .authorize_next()
                .err()
                .expect("TEST_CODE reject current invalid bearer");
            assert!(matches!(error, GrpcError::Unauthenticated { .. }));
            assert_eq!(server.macro_snapshot().requests.len(), 1);
            drop(invalid_client);

            let valid_retry = client
                .resume_macro_query(identity, material())
                .expect("TEST_CODE original material remains resumable")
                .authorize_next()
                .expect("TEST_CODE original current credential remains valid");
            assert_eq!(valid_retry.attempt_ordinal(), 2);
            assert_eq!(valid_retry.request_id(), request_id);
            assert_eq!(valid_retry.request_bytes(), request_bytes);
            let completed = tokio::time::timeout(Duration::from_secs(5), valid_retry.execute())
                .await
                .expect("TEST_CODE valid retry RPC timeout");
            assert!(completed.processed.is_ok());
            assert_eq!(completed.continuation, MacroContinuation::Terminal);
            let observation = server.macro_snapshot();
            assert_eq!(observation.requests.len(), 2);
            assert!(observation
                .requests
                .iter()
                .all(|(bytes, authorized)| *authorized && *bytes == request_bytes));
            assert_eq!(server.snapshot().non_board_requests, 0);
        }))
        .catch_unwind()
        .await;
    drop(client);
    server.finish().await;
    match outcome {
        Ok(result) => result.expect("TEST_CODE macro rejection body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn macro_explicit_retries_preserve_original_policy_until_exhausted() {
    let (client, server) = spawn_macro_repeat_retry_loopback().await;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(25), async {
            let identity = MacroQueryIdentity::GlobalNews {
                provider: GlobalNewsProvider::Jin10,
                limit: 20,
            };
            let first = client
                .macro_query(identity.clone())
                .expect("TEST_CODE original policy session")
                .authorize_next()
                .expect("TEST_CODE first authorization");
            let request_id = first.request_id().to_owned();
            let request_bytes = first.request_bytes();
            let retry_policy = first.retry_policy();
            assert_eq!(retry_policy, (4, 1000, 60_000, 200));
            let material = |next_attempt| RestoredMacroRequest {
                request_bytes: request_bytes.clone(),
                request_id: request_id.clone(),
                profile: ContractProfile::LocalBridgeV1,
                acquisition_authority: None,
                retry_policy,
                next_attempt,
            };
            let changed_client = clone_with_retry_policy(&client, (1, 7, 9, 0));
            let current_policy_probe = changed_client
                .macro_query(identity.clone())
                .expect("TEST_CODE current policy session")
                .authorize_next()
                .expect("TEST_CODE current policy authorization");
            assert_eq!(current_policy_probe.retry_policy(), (1, 7, 9, 0));
            drop(current_policy_probe);
            assert!(server.macro_snapshot().requests.is_empty());

            let mut completion = tokio::time::timeout(Duration::from_secs(5), first.execute())
                .await
                .expect("TEST_CODE first retryable RPC timeout");
            for (next_ordinal, expected_backoff) in [(2, 1000), (3, 2000), (4, 4000)] {
                assert_eq!(completion.retry_decision, RetryDecision::RetryBackoff);
                assert_eq!(
                    completion.continuation,
                    MacroContinuation::Retry {
                        backoff_ms: expected_backoff
                    }
                );
                assert!(matches!(
                    completion.processed,
                    Err(GrpcError::Unavailable { .. })
                ));
                assert_eq!(
                    server.macro_snapshot().requests.len(),
                    (next_ordinal - 1) as usize
                );
                // Only this explicitly observed Retry admits the next invocation.
                let next = changed_client
                    .resume_macro_query(identity.clone(), material(next_ordinal))
                    .expect("TEST_CODE restore original policy")
                    .authorize_next()
                    .expect("TEST_CODE reauthorize original request");
                assert_eq!(next.attempt_ordinal(), next_ordinal);
                assert_eq!(next.request_id(), request_id);
                assert_eq!(next.request_bytes(), request_bytes);
                assert_eq!(next.retry_policy(), (4, 1000, 60_000, 200));
                completion = tokio::time::timeout(Duration::from_secs(5), next.execute())
                    .await
                    .expect("TEST_CODE restored RPC timeout");
            }
            assert_eq!(
                completion.status_code,
                Some(tonic::Code::Unavailable as i32)
            );
            assert_eq!(completion.retry_decision, RetryDecision::RetryBackoff);
            assert_eq!(completion.continuation, MacroContinuation::Terminal);
            assert!(matches!(
                completion.processed,
                Err(GrpcError::Unavailable { .. })
            ));
            let error = changed_client
                .resume_macro_query(identity, material(5))
                .err()
                .expect("TEST_CODE fifth attempt must exceed original budget");
            assert!(matches!(error, GrpcError::FailedPrecondition { .. }));
            let observation = server.macro_snapshot();
            assert_eq!(observation.requests.len(), 4);
            assert_eq!(observation.retry_error_details.len(), 4);
            assert!(observation.response_bytes.is_empty());
            assert!(observation
                .requests
                .iter()
                .all(|(bytes, authorized)| *authorized && *bytes == request_bytes));
            assert_eq!(server.snapshot().non_board_requests, 0);
            drop(changed_client);
        }))
        .catch_unwind()
        .await;
    drop(client);
    server.finish().await;
    match outcome {
        Ok(result) => result.expect("TEST_CODE macro exhaustion body timeout"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn macro_nonretryable_status_preserves_raw_material_and_stops_after_one_rpc() {
    for (code, retryable, reason) in [
        (tonic::Code::Unavailable, false, "no_verified_batch"),
        (tonic::Code::InvalidArgument, true, "invalid_request"),
    ] {
        let (client, server) = spawn_macro_fixed_status_loopback(code, retryable).await;
        let outcome =
            std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(10), async {
                let attempt = client
                    .macro_query(MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Jin10,
                        limit: 20,
                    })
                    .expect("TEST_CODE terminal status session")
                    .authorize_next()
                    .expect("TEST_CODE terminal status authorization");
                let request_id = attempt.request_id().to_owned();
                let request_bytes = attempt.request_bytes();
                let completion = tokio::time::timeout(Duration::from_secs(5), attempt.execute())
                    .await
                    .expect("TEST_CODE terminal status RPC timeout");
                assert_eq!(completion.response_bytes, None);
                assert_eq!(completion.status_code, Some(code as i32));
                assert_eq!(completion.retry_decision, RetryDecision::NoRetry);
                assert_eq!(completion.continuation, MacroContinuation::Terminal);
                let observation = server.macro_snapshot();
                assert_eq!(observation.requests.len(), 1);
                assert_eq!(observation.requests[0], (request_bytes, true));
                assert_eq!(observation.retry_error_details.len(), 1);
                assert!(observation.response_bytes.is_empty());
                let raw_detail = &observation.retry_error_details[0];
                assert_eq!(
                    completion.status_details.as_deref(),
                    Some(raw_detail.as_slice())
                );
                assert_eq!(
                    completion.status_error_detail_trailer,
                    MacroTrailerMaterial::Bytes(raw_detail.clone())
                );
                let detail = ErrorDetail::decode(raw_detail.as_slice())
                    .expect("TEST_CODE complete raw ErrorDetail");
                assert_eq!(detail.request_id, request_id);
                assert_eq!(detail.operation, Operation::GlobalNews as i32);
                assert_eq!(detail.provider, "Jin10");
                assert_eq!(detail.reason_code, reason);
                assert_eq!(detail.retryable, retryable);
                let error = completion
                    .processed
                    .expect_err("TEST_CODE terminal processed error");
                match code {
                    tonic::Code::Unavailable => {
                        assert!(matches!(&error, GrpcError::Unavailable { .. }))
                    }
                    tonic::Code::InvalidArgument => {
                        assert!(matches!(&error, GrpcError::InvalidArgument { .. }))
                    }
                    _ => unreachable!("TEST_CODE closed terminal status table"),
                }
                assert_eq!(
                    error.details().method.map(|method| method.as_str_name()),
                    Some("OPERATION_GLOBAL_NEWS")
                );
                assert_eq!(error.details().provider.as_deref(), Some("Jin10"));
                assert_eq!(error.details().reason_code.as_deref(), Some(reason));
                assert_eq!(error.details().retryable, Some(retryable));
                assert_eq!(server.macro_snapshot().requests.len(), 1);
                assert_eq!(server.snapshot().non_board_requests, 0);
            }))
            .catch_unwind()
            .await;
        drop(client);
        server.finish().await;
        match outcome {
            Ok(result) => result.expect("TEST_CODE terminal status body timeout"),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

#[tokio::test]
async fn macro_raw_status_preserves_trailer_states_and_transport_retry_decisions() {
    for case in [
        MacroRawStatusCase::Absent,
        MacroRawStatusCase::TrailerOnly,
        MacroRawStatusCase::DistinctDetailsAndTrailer,
        MacroRawStatusCase::MalformedBase64,
        MacroRawStatusCase::InvalidProtobuf,
        MacroRawStatusCase::DeadlineExceeded,
    ] {
        let (client, server) = spawn_macro_raw_status_loopback(case).await;
        let outcome = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(10), async {
            let attempt = client.macro_query(MacroQueryIdentity::GlobalNews {
                provider: GlobalNewsProvider::Jin10, limit: 20,
            }).expect("TEST_CODE raw status session").authorize_next()
                .expect("TEST_CODE raw status authorization");
            let request_id = attempt.request_id().to_owned();
            let request_bytes = attempt.request_bytes();
            let completion = tokio::time::timeout(Duration::from_secs(5), attempt.execute())
                .await.expect("TEST_CODE raw status RPC timeout");
            let observation = server.macro_snapshot();
            assert_eq!(observation.requests, vec![(request_bytes, true)], "{case:?}");
            assert_eq!(observation.raw_statuses.len(), 1);
            assert!(observation.response_bytes.is_empty());
            let sent = &observation.raw_statuses[0];
            let expected_code = if case == MacroRawStatusCase::DeadlineExceeded {
                tonic::Code::DeadlineExceeded
            } else { tonic::Code::Unavailable };
            assert_eq!(sent.code, expected_code as i32);
            assert_eq!(completion.status_code, Some(expected_code as i32));
            assert_eq!(completion.status_details.as_deref(), Some(sent.details.as_slice()));
            assert_eq!(completion.response_bytes, None);
            match case {
                MacroRawStatusCase::Absent | MacroRawStatusCase::DeadlineExceeded => {
                    assert!(sent.details.is_empty());
                    assert_eq!(sent.trailer_header, None);
                    assert_eq!(completion.status_error_detail_trailer, MacroTrailerMaterial::Absent);
                }
                MacroRawStatusCase::TrailerOnly | MacroRawStatusCase::DistinctDetailsAndTrailer => {
                    let MacroTrailerMaterial::Bytes(bytes) = &completion.status_error_detail_trailer
                    else { panic!("TEST_CODE expected intact binary ErrorDetail: {case:?}"); };
                    let detail = ErrorDetail::decode(bytes.as_slice()).expect("TEST_CODE trailer ErrorDetail");
                    assert_eq!(detail.request_id, request_id);
                    assert_eq!(detail.operation, Operation::GlobalNews as i32);
                    assert_eq!(detail.provider, "Jin10");
                    assert_eq!(detail.reason_code, "no_verified_batch");
                    assert!(detail.retryable);
                    assert_eq!(sent.trailer_header.as_deref(), Some(
                        tonic::metadata::MetadataValue::<tonic::metadata::Binary>::from_bytes(bytes).as_encoded_bytes()));
                    if case == MacroRawStatusCase::DistinctDetailsAndTrailer {
                        let standard = ErrorDetail::decode(sent.details.as_slice()).expect("TEST_CODE distinct standard detail");
                        assert_eq!(standard.request_id, request_id);
                        assert_eq!(standard.operation, Operation::GlobalNews as i32);
                        assert_eq!(standard.provider, "Jin10");
                        assert_eq!(standard.reason_code, "no_verified_batch");
                        assert!(!standard.retryable);
                        assert_ne!(&sent.details, bytes);
                    } else { assert!(sent.details.is_empty()); }
                }
                MacroRawStatusCase::MalformedBase64 => {
                    assert!(sent.details.is_empty());
                    assert_eq!(sent.trailer_header.as_deref(), Some(b"%%%".as_slice()));
                    assert_eq!(completion.status_error_detail_trailer, MacroTrailerMaterial::Malformed);
                }
                MacroRawStatusCase::InvalidProtobuf => {
                    assert!(sent.details.is_empty());
                    assert_eq!(sent.trailer_header.as_deref(), Some(b"/w".as_slice()));
                    assert_eq!(completion.status_error_detail_trailer, MacroTrailerMaterial::Bytes(vec![0xff]));
                }
            }
            let error = completion.processed.expect_err("TEST_CODE raw status processed error");
            if case == MacroRawStatusCase::DeadlineExceeded {
                assert!(matches!(&error, GrpcError::DeadlineExceeded { .. }));
                assert_eq!(completion.retry_decision, RetryDecision::RetryBounded);
            } else {
                assert!(matches!(&error, GrpcError::Unavailable { .. }));
                assert_eq!(completion.retry_decision, RetryDecision::RetryBackoff);
            }
            if case == MacroRawStatusCase::TrailerOnly {
                assert_eq!(error.details().provider.as_deref(), Some("Jin10"));
                assert_eq!(error.details().reason_code.as_deref(), Some("no_verified_batch"));
                assert_eq!(error.details().retryable, Some(true));
            } else {
                // Conflicting or malformed detail channels retain transport
                // classification; neither channel silently overrides the other.
                assert_eq!(error.details().provider, None);
                assert_eq!(error.details().reason_code, None);
                assert_eq!(error.details().retryable, None);
            }
            assert_eq!(completion.continuation, MacroContinuation::Retry { backoff_ms: 1000 });
            assert_eq!(server.macro_snapshot().requests.len(), 1);
            assert_eq!(server.snapshot().non_board_requests, 0);
        })).catch_unwind().await;
        drop(client);
        server.finish().await;
        match outcome {
            Ok(result) => result.expect("TEST_CODE raw status body timeout"),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

#[tokio::test]
async fn macro_envelope_failures_preserve_original_response_and_do_not_retry() {
    for case in [
        MacroEnvelopeCase::WrongRequestId,
        MacroEnvelopeCase::MissingRequestId,
        MacroEnvelopeCase::WrongOperation,
    ] {
        let (client, server) = spawn_macro_envelope_failure_loopback(case).await;
        let outcome =
            std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(10), async {
                let attempt = client
                    .macro_query(MacroQueryIdentity::GlobalNews {
                        provider: GlobalNewsProvider::Jin10,
                        limit: 20,
                    })
                    .expect("TEST_CODE envelope session")
                    .authorize_next()
                    .expect("TEST_CODE envelope authorization");
                let request_id = attempt.request_id().to_owned();
                let request_bytes = attempt.request_bytes();
                let completion = tokio::time::timeout(Duration::from_secs(5), attempt.execute())
                    .await
                    .expect("TEST_CODE envelope RPC timeout");
                let observation = server.macro_snapshot();
                assert_eq!(observation.requests, vec![(request_bytes, true)]);
                assert_eq!(observation.response_bytes.len(), 1);
                assert!(observation.retry_error_details.is_empty());
                assert!(observation.raw_statuses.is_empty());
                assert_eq!(
                    completion.response_bytes.as_deref(),
                    Some(observation.response_bytes[0].as_slice())
                );
                assert_eq!(completion.status_code, None);
                assert_eq!(completion.status_details, None);
                assert_eq!(
                    completion.status_error_detail_trailer,
                    MacroTrailerMaterial::Absent
                );
                assert_eq!(completion.retry_decision, RetryDecision::NoRetry);
                assert_eq!(completion.continuation, MacroContinuation::Terminal);
                let raw = QueryResponse::decode(completion.response_bytes.as_deref().unwrap())
                    .expect("TEST_CODE preserved invalid envelope bytes");
                match case {
                    MacroEnvelopeCase::WrongRequestId => {
                        assert_eq!(raw.request_id, "TEST_CODE_WRONG_RESPONSE_ID");
                        assert_ne!(raw.request_id, request_id);
                        assert_eq!(raw.operation, Operation::GlobalNews as i32);
                    }
                    MacroEnvelopeCase::MissingRequestId => {
                        assert!(raw.request_id.is_empty());
                        assert_eq!(raw.operation, Operation::GlobalNews as i32);
                    }
                    MacroEnvelopeCase::WrongOperation => {
                        assert_eq!(raw.request_id, request_id);
                        assert_eq!(raw.operation, Operation::EconomicCalendar as i32);
                    }
                }
                assert_eq!(raw.admission, AdmissionState::Admitted as i32);
                assert_eq!(raw.selected_provider, "Jin10");
                assert_eq!(raw.batch_id, "TEST_CODE_MACRO_BATCH");
                assert!(raw.complete);
                assert_eq!(raw.observed_at, "2026-09-14T15:31:00+08:00");
                assert_eq!(raw.source_at, "2026-09-14T15:30:00+08:00");
                assert_eq!(raw.source, "jin10-flash-v1");
                assert!(raw.diagnostic_blocker.is_empty());
                assert_eq!(raw.records.len(), 1);
                assert_eq!(raw.records[0].schema, "news.global_news");
                assert_eq!(raw.records[0].schema_version, 1);
                assert_eq!(
                    raw.records[0].content_type,
                    "application/json; charset=utf-8"
                );
                assert_eq!(
                    raw.records[0].data,
                    br#"[{"title":"TEST_CODE_MACRO_NEWS"}]"#
                );
                let error = completion
                    .processed
                    .expect_err("TEST_CODE reject invalid response envelope");
                assert!(matches!(&error, GrpcError::Unknown { .. }));
                assert_eq!(error.details().code, "envelope");
                assert_eq!(error.details().provider, None);
                assert_eq!(error.details().retryable, None);
                assert_eq!(server.macro_snapshot().requests.len(), 1);
                assert_eq!(server.snapshot().non_board_requests, 0);
            }))
            .catch_unwind()
            .await;
        drop(client);
        server.finish().await;
        match outcome {
            Ok(result) => result.expect("TEST_CODE envelope body timeout"),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

#[tokio::test]
async fn macro_local_wire_shapes_restore_exact_provider_query_and_limit() {
    use crate::data_gateway::GeneralWebResearchProvider as Web;
    use GlobalNewsProvider as News;
    use MacroQueryIdentity::{EconomicCalendar, GlobalNews, SemanticSearch};

    let cases = [
        (
            GlobalNews {
                provider: News::Eastmoney,
                limit: 20,
            },
            Operation::GlobalNews,
            "Eastmoney",
            "news.global_news",
            r#"{"limit":20,"provider":"Eastmoney"}"#,
        ),
        (
            GlobalNews {
                provider: News::Cailianpress,
                limit: 20,
            },
            Operation::GlobalNews,
            "Cailianpress",
            "news.global_news",
            r#"{"limit":20,"provider":"Cailianpress"}"#,
        ),
        (
            GlobalNews {
                provider: News::Jin10,
                limit: 21,
            },
            Operation::GlobalNews,
            "Jin10",
            "news.global_news",
            r#"{"limit":21,"provider":"Jin10"}"#,
        ),
        (
            GlobalNews {
                provider: News::ThePaper,
                limit: 20,
            },
            Operation::GlobalNews,
            "ThePaper",
            "news.global_news",
            r#"{"limit":20,"provider":"ThePaper"}"#,
        ),
        (
            EconomicCalendar,
            Operation::EconomicCalendar,
            "Jin10",
            "market.economic_calendar",
            "{}",
        ),
        (
            SemanticSearch {
                provider: Web::Bocha,
                query: " TEST_CODE macro α ".into(),
                limit: 51,
            },
            Operation::SemanticSearch,
            "Bocha",
            "market.semantic_search",
            r#"{"limit":51,"provider":"Bocha","query":" TEST_CODE macro α "}"#,
        ),
        (
            SemanticSearch {
                provider: Web::Tavily,
                query: "TEST_CODE_macro_query".into(),
                limit: 3,
            },
            Operation::SemanticSearch,
            "Tavily",
            "market.semantic_search",
            r#"{"limit":3,"provider":"Tavily","query":"TEST_CODE_macro_query"}"#,
        ),
        (
            SemanticSearch {
                provider: Web::SerpApi,
                query: "TEST_CODE_macro_query".into(),
                limit: 3,
            },
            Operation::SemanticSearch,
            "SerpApi",
            "market.semantic_search",
            r#"{"limit":3,"provider":"SerpApi","query":"TEST_CODE_macro_query"}"#,
        ),
    ];
    for (identity, operation, provider, schema, expected_payload) in cases {
        let (base_client, server) = spawn_macro_local_shape_loopback().await;
        let outcome =
            std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(15), async {
                let policy = if provider == "Bocha" {
                    (3, 125, 1000, 0)
                } else {
                    (4, 1000, 60_000, 200)
                };
                let client = clone_with_retry_policy(&base_client, policy);
                let first = client
                    .macro_query(identity.clone())
                    .expect("TEST_CODE shape session")
                    .authorize_next()
                    .expect("TEST_CODE shape authorization");
                let request_id = first.request_id().to_owned();
                let request_bytes = first.request_bytes();
                assert_eq!(first.profile(), "LocalBridgeV1");
                assert_eq!(first.acquisition_authority(), None);
                assert_eq!(first.retry_policy(), policy);
                let material = || RestoredMacroRequest {
                    request_bytes: request_bytes.clone(),
                    request_id: request_id.clone(),
                    profile: ContractProfile::LocalBridgeV1,
                    acquisition_authority: None,
                    retry_policy: policy,
                    next_attempt: 2,
                };
                let request = QueryRequest::decode(request_bytes.as_slice())
                    .expect("TEST_CODE shape request");
                let context = request.context.expect("TEST_CODE shape context");
                assert_eq!(context.protocol_version, 1);
                assert_eq!(context.request_id, request_id);
                assert!(!request_id.is_empty());
                assert!(request.preferred_provider.is_empty());
                assert!(!request.allow_unadmitted);
                let payload = request.payload.expect("TEST_CODE shape payload");
                assert_eq!(payload.schema, schema);
                assert_eq!(payload.schema_version, 1);
                assert_eq!(payload.content_type, "application/json; charset=utf-8");
                assert_eq!(payload.data, expected_payload.as_bytes());
                let first = tokio::time::timeout(Duration::from_secs(5), first.execute())
                    .await
                    .expect("TEST_CODE shape first RPC timeout");
                assert_eq!(first.retry_decision, RetryDecision::RetryBackoff);
                assert_eq!(
                    first.continuation,
                    MacroContinuation::Retry {
                        backoff_ms: policy.1
                    }
                );
                assert!(matches!(
                    first.processed,
                    Err(GrpcError::Unavailable { .. })
                ));
                let first_observation = server.macro_snapshot();
                assert_eq!(
                    first_observation.requests,
                    vec![(request_bytes.clone(), true)]
                );
                assert_eq!(first_observation.routed_operations, vec![operation]);
                let first_detail =
                    ErrorDetail::decode(first_observation.retry_error_details[0].as_slice())
                        .expect("TEST_CODE matching route detail");
                assert_eq!(first_detail.operation, operation as i32);
                assert_eq!(first_detail.provider, provider);

                let drifted = match &identity {
                    GlobalNews { provider, limit } => vec![
                        GlobalNews {
                            provider: if *provider == News::Eastmoney {
                                News::Jin10
                            } else {
                                News::Eastmoney
                            },
                            limit: *limit,
                        },
                        GlobalNews {
                            provider: *provider,
                            limit: limit + 1,
                        },
                    ],
                    EconomicCalendar => vec![GlobalNews {
                        provider: News::Jin10,
                        limit: 20,
                    }],
                    SemanticSearch {
                        provider,
                        query,
                        limit,
                    } => {
                        let mut drifted = vec![
                            SemanticSearch {
                                provider: if *provider == Web::Bocha {
                                    Web::Tavily
                                } else {
                                    Web::Bocha
                                },
                                query: query.clone(),
                                limit: *limit,
                            },
                            SemanticSearch {
                                provider: *provider,
                                query: format!("{query}_CHANGED"),
                                limit: *limit,
                            },
                            SemanticSearch {
                                provider: *provider,
                                query: query.clone(),
                                limit: limit + 1,
                            },
                        ];
                        if query.trim() != query {
                            drifted.push(SemanticSearch {
                                provider: *provider,
                                query: query.trim().to_owned(),
                                limit: *limit,
                            });
                        }
                        drifted
                    }
                };
                for changed_identity in drifted {
                    let error = client
                        .resume_macro_query(changed_identity, material())
                        .err()
                        .expect("TEST_CODE reject shape identity drift");
                    assert!(matches!(error, GrpcError::FailedPrecondition { .. }));
                    assert_eq!(server.macro_snapshot().requests.len(), 1);
                }
                let second = client
                    .resume_macro_query(identity, material())
                    .expect("TEST_CODE restore exact shape")
                    .authorize_next()
                    .expect("TEST_CODE authorize restored shape");
                assert_eq!(second.attempt_ordinal(), 2);
                assert_eq!(second.request_bytes(), request_bytes);
                assert_eq!(second.request_id(), request_id);
                assert_eq!(second.profile(), "LocalBridgeV1");
                assert_eq!(second.acquisition_authority(), None);
                assert_eq!(second.retry_policy(), policy);
                let completed = tokio::time::timeout(Duration::from_secs(5), second.execute())
                    .await
                    .expect("TEST_CODE shape restored RPC timeout");
                let observation = server.macro_snapshot();
                assert_eq!(
                    observation.requests,
                    vec![(request_bytes.clone(), true), (request_bytes, true)]
                );
                assert_eq!(observation.routed_operations, vec![operation, operation]);
                assert_eq!(observation.response_bytes.len(), 1);
                assert_eq!(
                    completed.response_bytes.as_deref(),
                    Some(observation.response_bytes[0].as_slice())
                );
                assert_eq!(completed.retry_decision, RetryDecision::NoRetry);
                assert_eq!(completed.continuation, MacroContinuation::Terminal);
                let response = QueryResponse::decode(completed.response_bytes.as_deref().unwrap())
                    .expect("TEST_CODE raw shape response");
                assert_eq!(response.request_id, request_id);
                assert_eq!(response.operation, operation as i32);
                assert_eq!(response.selected_provider, provider);
                assert_eq!(response.records[0].schema, schema);
                assert_eq!(
                    response.records[0].data,
                    br#"[{"value":"TEST_CODE_MACRO_SHAPE"}]"#
                );
                let processed = completed
                    .processed
                    .expect("TEST_CODE admitted shape response");
                assert_eq!(processed.admission, AdmissionState::Admitted);
                assert!(processed.complete);
                assert_eq!(processed.selected_provider, provider);
                assert_eq!(processed.records, response.records);
                assert_eq!(server.snapshot().non_board_requests, 0);
                drop(client);
            }))
            .catch_unwind()
            .await;
        drop(base_client);
        server.finish().await;
        match outcome {
            Ok(result) => result.expect("TEST_CODE Local shape body timeout"),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}
