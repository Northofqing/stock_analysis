use super::*;
use crate::grpc_client::client::external_control_loopback_fixture::{
    write_test_code_bundle, ExternalMtlsSwitch, TEST_CODE_MTLS_CA_CERT, TEST_CODE_MTLS_SERVER_CERT,
    TEST_CODE_MTLS_SERVER_KEY,
};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tonic::codegen::{http, Body, BoxFuture, Service, StdError};
use tonic::server::{NamedService, UnaryService};
use tonic::transport::{Certificate, Identity, ServerTlsConfig};

const TEST_AUTH: &str = "Bearer TEST_CODE_EXTERNAL_CONTROL_TOKEN";
const TEST_AUTHORITY: &str = "grpc-mtls:macro.test.invalid";

fn candidate_health(id: &str) -> pb::HealthResponse {
    pb::HealthResponse {
        request_id: id.into(),
        live: true,
        ready: true,
        build_identity: Some(pb::BuildIdentity {
            service_version: "0.2.0".into(),
            source_revision: "b7d206668753dc762e776b0ff893c63d53166afe".into(),
            binary_sha256: "f23a7bf05f68b7f4fabc7feee27b405f1ae6a69ec281272203e5f3437114479a"
                .into(),
            contract_sha256: "abf28a3e0028488a7579da4d961e1a7c1408482bdc0500122c1956d225e480cf"
                .into(),
            identity_error: String::new(),
        }),
        ..Default::default()
    }
}

#[test]
fn candidate_b7_probe_compiled_policy_public_bytes_null_and_both_descriptor_scopes_are_exact() {
    let inputs = compiled_candidate_b7_inputs().unwrap();
    assert_eq!(
        inputs.policy_sha256(),
        "002567fb6006202984f6597a141db21f84de74d46adad7678d990e706e241597"
    );
    assert_eq!(
        inputs.client_descriptor_sha256(),
        EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256
    );
    let json = serde_json::to_value(inputs).unwrap();
    assert_eq!(json["raw_metadata_bytes"], 779);
    assert_eq!(json["raw_proto_bytes"], 12707);
    assert_eq!(json["formal_deployment_identity_available"], false);
    assert_eq!(
        json["raw_metadata_sha256"],
        "605e856735047f68a7cc5c1bbc3f7e65472da59c41f0c7cd395d0bf3eb0338a8"
    );
    assert_eq!(
        json["raw_proto_sha256"],
        "801c2033e72c520b34ca110eda18029699c619b740b71e50f99308a3088689c1"
    );
    assert_ne!(
        json["expected_server_descriptor_sha256"],
        json["compiled_client_descriptor_sha256"]
    );
    let formal: serde_json::Value = serde_json::from_slice(include_bytes!(
        "probe_profiles/windows-b7-20261002.17/bundle-metadata.json"
    ))
    .unwrap();
    assert!(formal["deployment_build_identity"].is_null());
    let current = crate::grpc_client::build_identity::compiled_public_inputs().unwrap();
    assert_eq!(current.bundle_version, "2026-10-01.3");
    assert_eq!(
        current.policy_sha256,
        "a7bf22f2693e768349f8e53ec5180bf88128f88992966264377c69ec288c2fa8"
    );
    assert_eq!(
        current.raw_metadata_sha256,
        "010cfd26409b486ffa655111f27e7847a4b7ae4d844231ca79997d6308d2d36b"
    );
}

#[test]
fn candidate_b7_probe_never_qualifies_default67_or_archived_sep28_as_candidate() {
    let candidate = BuildIdentityTrust::candidate_b7_probe().unwrap();
    let bundled = BuildIdentityTrust::bundled().unwrap();
    let health = candidate_health("TEST_CODE_b7_identity");
    assert!(candidate.current_health(&health).is_ok());
    assert!(bundled.current_health(&health).is_err());
    assert!(crate::grpc_client::build_identity::qualify_public_health(&health).is_err());
    let current = pb::HealthResponse {
        build_identity: Some(crate::grpc_client::build_identity::test_public_build_identity()),
        ..health.clone()
    };
    assert!(candidate.current_health(&current).is_err());
    let recorded67 = ConnectionIdentity {
        version: 1,
        epoch: "TEST_CODE_67_recorded_epoch".into(),
        policy_sha256: bundled.current_policy_sha256(),
        descriptor_sha256: EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256.into(),
    };
    assert!(recorded67.validate_recorded());
    assert!(bundled
        .recorded_health(
            &recorded67.policy_sha256,
            &recorded67.descriptor_sha256,
            &current
        )
        .is_ok());
    let recorded_candidate = ConnectionIdentity {
        policy_sha256: candidate.current_policy_sha256(),
        ..recorded67
    };
    assert!(!recorded_candidate.validate_recorded());
    let meta: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../contracts/external_v1_history/20260928.2/bundle-metadata.json"
    ))
    .unwrap();
    let old = &meta["deployment_build_identity"];
    let old_health = pb::HealthResponse {
        request_id: "TEST_CODE_SEP28_ORIGINAL_POLICY".into(),
        live: true,
        ready: true,
        build_identity: Some(pb::BuildIdentity {
            service_version: old["service_version"].as_str().unwrap().into(),
            source_revision: old["source_revision"].as_str().unwrap().into(),
            contract_sha256: old["contract_sha256"].as_str().unwrap().into(),
            binary_sha256: old["binary_sha256"].as_str().unwrap().into(),
            identity_error: String::new(),
        }),
        ..Default::default()
    };
    let descriptor = crate::grpc_client::archived_external_20260928::DESCRIPTOR_SHA256;
    let raw = old_health.encode_to_vec();
    let decoded = ExternalDecoder::for_descriptor(descriptor)
        .unwrap()
        .health(&raw)
        .unwrap();
    assert_eq!(decoded, old_health);
    assert!(bundled
        .recorded_health(
            "de9a897d7e35f35bdd475b4133f002b27e9dfb40cb029aeb81d590ac3e70f54a",
            descriptor,
            &decoded
        )
        .is_ok());
    assert!(
        bundled.historical_health(&decoded).is_err(),
        "real Sep28 V4 must not use Sep17 v3"
    );
    assert!(candidate.current_health(&decoded).is_err());
}

#[test]
fn candidate_b7_probe_full_identity_and_ready_cannot_be_replaced_by_health_claims() {
    let trust = BuildIdentityTrust::candidate_b7_probe().unwrap();
    let original = candidate_health("TEST_CODE_exact_identity");
    for field in 0..6 {
        let mut changed = original.clone();
        match field {
            0 => changed
                .build_identity
                .as_mut()
                .unwrap()
                .service_version
                .push('x'),
            1 => changed
                .build_identity
                .as_mut()
                .unwrap()
                .source_revision
                .push('x'),
            2 => changed
                .build_identity
                .as_mut()
                .unwrap()
                .binary_sha256
                .push('x'),
            3 => {
                changed.build_identity.as_mut().unwrap().contract_sha256 =
                    EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256.into()
            }
            4 => {
                changed.build_identity.as_mut().unwrap().identity_error =
                    "TEST_CODE_remote_identity_error".into()
            }
            _ => changed.ready = false,
        }
        assert!(trust.current_health(&changed).is_err());
    }
    let mut missing = original;
    missing.build_identity = None;
    assert!(trust.current_health(&missing).is_err());
}

#[test]
fn candidate_b7_probe_plan_freezes_every_schema_limit_request_wire_and_has_no_runtime_expected_factory(
) {
    let plan = CandidateProbePlan::windows_b7().unwrap();
    let bytes = plan.canonical_bytes().unwrap();
    assert_eq!(
        plan.dto.steps.0[..8]
            .iter()
            .map(|s| s.kind)
            .collect::<Vec<_>>(),
        vec![
            StepKind::Health,
            StepKind::Capabilities,
            StepKind::UnauthenticatedHealth,
            StepKind::UnsupportedSchemaNegative,
            StepKind::Health,
            StepKind::Capabilities,
            StepKind::Business,
            StepKind::Health,
        ]
    );
    assert_eq!(
        CandidateProbePlan::read_checked(&bytes)
            .unwrap()
            .canonical_bytes()
            .unwrap(),
        bytes
    );
    let mut queries = Vec::new();
    for step in &plan.dto.steps.0 {
        if matches!(
            step.kind,
            StepKind::Business | StepKind::UnsupportedSchemaNegative
        ) {
            let request =
                pb::QueryRequest::decode(hex::decode(&step.request_wire_hex).unwrap().as_slice())
                    .unwrap();
            assert!(!request.allow_unadmitted);
            assert_eq!(
                request.context.as_ref().unwrap().request_id,
                step.request_id
            );
            let payload = request.payload.as_ref().unwrap();
            let data: serde_json::Value = serde_json::from_slice(&payload.data).unwrap();
            queries.push((
                payload.schema_version,
                data["limit"].as_u64().unwrap(),
                request.preferred_provider,
            ));
        }
    }
    assert_eq!(
        queries.iter().map(|q| (q.0, q.1)).collect::<Vec<_>>(),
        vec![
            (3, 1),
            (1, 1),
            (1, 15),
            (2, 1),
            (2, 15),
            (1, 1),
            (1, 300),
            (2, 1),
            (2, 300)
        ]
    );
    for mutation in 0..7 {
        let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        match mutation {
            0 => json["endpoint"] = "https://TEST_CODE_other.invalid:50051".into(),
            1 => json["automatic_retries"] = 1.into(),
            2 => json["provider_fallback"] = true.into(),
            3 => json["steps"][4]["request_wire_sha256"] = "0".repeat(64).into(),
            4 => json["steps"][4]["request_wire_hex"] = "00".into(),
            5 => json["steps"][1]["request_id"] = json["steps"][0]["request_id"].clone(),
            _ => {
                json["expected_identity"] =
                    serde_json::json!({"source_revision":"TEST_CODE_learned"})
            }
        }
        assert!(CandidateProbePlan::read_checked(&serde_json::to_vec(&json).unwrap()).is_err());
    }
    let duplicated = String::from_utf8(bytes.clone())
        .unwrap()
        .replacen("{", "{\"version\":1,", 1);
    assert!(CandidateProbePlan::read_checked(duplicated.as_bytes()).is_err());
    assert!(CandidateProbePlan::read_checked(&vec![b' '; MAX_CANDIDATE_PLAN_BYTES + 1]).is_err());
}

#[test]
fn candidate_b7_probe_schema3_requires_original_typed_invalid_request_without_provider_or_trace() {
    let plan = CandidateProbePlan::windows_b7().unwrap();
    let step = &plan.dto.steps.0[3];
    let state = Arc::new(Mutex::new(State::default()));
    let original = schema_rejection(&step.request_id);
    let valid = typed_status(&state, tonic::Code::InvalidArgument, original.clone());
    assert!(fixed_schema_rejection(step, &valid));
    for mutation in 0..12 {
        let mut detail = original.clone();
        match mutation {
            0 => detail.reason_code = "provider_unavailable".into(),
            1 => detail.request_id = "TEST_CODE_OTHER_REQUEST".into(),
            2 => detail.operation = pb::Operation::HistoricalBars as i32,
            3 => detail.retryable = true,
            4 => detail.admission = pb::AdmissionState::Admitted as i32,
            5 => detail.evidence_code = "TEST_CODE_EVIDENCE".into(),
            6 => detail.evidence_field = "TEST_CODE_FIELD".into(),
            7 => detail.record_index = 1,
            8 => detail.has_record_index = true,
            9 => detail
                .provider_attempts
                .push(pb::ProviderAttemptDetail::default()),
            10 => detail.provider = "Cninfo".into(),
            _ => detail.provider = "TEST_CODE_UNRECOGNIZED_PROVIDER".into(),
        }
        assert!(!fixed_schema_rejection(
            step,
            &typed_status(&state, tonic::Code::InvalidArgument, detail)
        ));
    }
    assert!(!fixed_schema_rejection(
        step,
        &tonic::Status::invalid_argument("invalid_request")
    ));
    let mut mismatch = valid.clone();
    let other = schema_rejection("TEST_CODE_OTHER_REQUEST").encode_to_vec();
    mismatch.metadata_mut().insert_bin(
        "magic-error-detail-bin",
        tonic::metadata::MetadataValue::from_bytes(&other),
    );
    assert!(!fixed_schema_rejection(step, &mismatch));
    let mut unknown = original.encode_to_vec();
    unknown.extend_from_slice(&[0xf8, 0x07, 1]);
    let noncanonical = tonic::Status::with_details(
        tonic::Code::InvalidArgument,
        "TEST_CODE_UNKNOWN_FIELD",
        prost::bytes::Bytes::from(unknown),
    );
    assert!(!fixed_schema_rejection(step, &noncanonical));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Case {
    #[default]
    Success,
    Old67,
    WrongHealthId,
    SecondHealthStatus,
    WrongCapsId,
    UnadmittedCaps,
    BusinessFailure,
    WrongQueryId,
    UnexpectedUnauthenticatedSuccess,
    UnexpectedSchemaSuccess,
    SchemaProviderFailure,
    AuthenticatedHealthDenied,
    LastHealthStatus,
    WrongInnerId,
    WrongPayloadHash,
    BadPrefix,
    UnknownHistoricalV2,
    ExhaustedExtraPages,
    NonemptyWithZeroUnique,
    VerifiedSourceEmpty,
    HistoricalUnderLimitFalse,
    HistoricalEmptyFalse,
    VerifiedHistoricalEmpty,
    HistoricalFullWindowFalse,
    HistoricalSourceBeyondWindow,
    HistoricalStartDayPartial,
    HistoricalV2StartDayPartial,
    HistoricalStartDayStrict,
}
#[derive(Clone, Debug, Default)]
struct Observation {
    tcp: usize,
    methods: Vec<String>,
    requests: Vec<Vec<u8>>,
    auth: Vec<bool>,
    responses: Vec<Vec<u8>>,
    statuses: Vec<(i32, Vec<u8>, Vec<u8>)>,
}
#[derive(Default)]
struct State {
    case: Case,
    observation: Observation,
    health_calls: usize,
}
#[derive(Clone)]
struct FixtureService {
    state: Arc<Mutex<State>>,
}

fn capture_request<M: Message>(
    state: &Arc<Mutex<State>>,
    method: &str,
    request: &tonic::Request<M>,
) -> bool {
    let auth = request
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        == Some(TEST_AUTH);
    let mut state = state.lock().unwrap();
    state.observation.methods.push(method.into());
    state
        .observation
        .requests
        .push(request.get_ref().encode_to_vec());
    state.observation.auth.push(auth);
    auth
}
fn reply(
    state: &Arc<Mutex<State>>,
    bytes: Vec<u8>,
) -> Result<tonic::Response<Vec<u8>>, tonic::Status> {
    state
        .lock()
        .unwrap()
        .observation
        .responses
        .push(bytes.clone());
    Ok(tonic::Response::new(bytes))
}
fn failed(state: &Arc<Mutex<State>>, code: tonic::Code, id: &str, op: i32) -> tonic::Status {
    let detail = pb::ErrorDetail {
        request_id: id.into(),
        operation: op,
        provider: "HithinkFinance".into(),
        reason_code: "provider_unavailable".into(),
        retryable: false,
        ..Default::default()
    };
    typed_status(state, code, detail)
}
fn schema_rejection(id: &str) -> pb::ErrorDetail {
    pb::ErrorDetail {
        request_id: id.into(),
        operation: pb::Operation::MarketAnnouncements as i32,
        reason_code: "invalid_request".into(),
        admission: pb::AdmissionState::Unadmitted as i32,
        ..Default::default()
    }
}
fn typed_status(
    state: &Arc<Mutex<State>>,
    code: tonic::Code,
    detail: pb::ErrorDetail,
) -> tonic::Status {
    let detail = detail.encode_to_vec();
    let mut status = tonic::Status::with_details(
        code,
        "TEST_CODE_REMOTE_SENSITIVE_DETAIL_DO_NOT_PRINT",
        prost::bytes::Bytes::from(detail.clone()),
    );
    status.metadata_mut().insert_bin(
        "magic-error-detail-bin",
        tonic::metadata::MetadataValue::from_bytes(&detail),
    );
    state
        .lock()
        .unwrap()
        .observation
        .statuses
        .push((code as i32, detail.clone(), detail));
    status
}

struct HealthUnary(FixtureService);
impl UnaryService<pb::HealthRequest> for HealthUnary {
    type Response = Vec<u8>;
    type Future = BoxFuture<tonic::Response<Vec<u8>>, tonic::Status>;
    fn call(&mut self, req: tonic::Request<pb::HealthRequest>) -> Self::Future {
        let service = self.0.clone();
        Box::pin(async move {
            let auth = capture_request(&service.state, "Health", &req);
            let id = req.into_inner().context.unwrap().request_id;
            let (case, calls) = {
                let mut s = service.state.lock().unwrap();
                if auth {
                    s.health_calls += 1;
                }
                (s.case, s.health_calls)
            };
            if !auth && case != Case::UnexpectedUnauthenticatedSuccess {
                return Err(tonic::Status::unauthenticated("TEST_CODE_AUTH_REQUIRED"));
            }
            if auth && case == Case::AuthenticatedHealthDenied {
                return Err(tonic::Status::unauthenticated("TEST_CODE_AUTH_REQUIRED"));
            }
            if (case == Case::SecondHealthStatus && calls == 2)
                || (case == Case::LastHealthStatus && calls == 17)
            {
                return Err(failed(
                    &service.state,
                    tonic::Code::DeadlineExceeded,
                    &id,
                    0,
                ));
            }
            let mut response = candidate_health(&id);
            if case == Case::Old67 {
                response.build_identity =
                    Some(crate::grpc_client::build_identity::test_public_build_identity());
            }
            if case == Case::WrongHealthId {
                response.request_id = "TEST_CODE_FOREIGN_HEALTH_ID".into();
            }
            reply(&service.state, response.encode_to_vec())
        })
    }
}
struct CapsUnary(FixtureService);
impl UnaryService<pb::CapabilitiesRequest> for CapsUnary {
    type Response = Vec<u8>;
    type Future = BoxFuture<tonic::Response<Vec<u8>>, tonic::Status>;
    fn call(&mut self, req: tonic::Request<pb::CapabilitiesRequest>) -> Self::Future {
        let service = self.0.clone();
        Box::pin(async move {
            if !capture_request(&service.state, "Capabilities", &req) {
                return Err(tonic::Status::unauthenticated("TEST_CODE_AUTH_REQUIRED"));
            }
            let id = req.into_inner().context.unwrap().request_id;
            let case = service.state.lock().unwrap().case;
            let response = pb::CapabilitiesResponse {
                request_id: if case == Case::WrongCapsId {
                    "TEST_CODE_FOREIGN_CAP_ID".into()
                } else {
                    id
                },
                capabilities: [0, 4]
                    .iter()
                    .map(|i| {
                        let (op, _, provider, _, _) = business_recipe(*i);
                        pb::Capability {
                            operation: op as i32,
                            provider: provider.into(),
                            repository_admission: if case == Case::UnadmittedCaps {
                                pb::AdmissionState::Unadmitted as i32
                            } else {
                                pb::AdmissionState::Admitted as i32
                            },
                            runtime_available: true,
                            exact_scope: "TEST_CODE_SYNTHETIC_TRANSPORT_ONLY".into(),
                            ..Default::default()
                        }
                    })
                    .collect(),
            };
            reply(&service.state, response.encode_to_vec())
        })
    }
}
// Original NormalProvider records below are source-shape evidence only.
// QueryRequest/QueryResponse envelopes are synthetic RPC fixture construction,
// never a live observation or an admitted Gateway/coverage/PIT capability.
fn original_historical_result(limit: usize) -> serde_json::Value {
    let (raw, expected) = if limit == 1 {
        (
            NORMAL_PROVIDER_LIMIT_1,
            "21257a4d4346b881cf02966f0aefa7cda24e81ef41d475e07939e5c959d4fac9",
        )
    } else {
        (
            NORMAL_PROVIDER_LIMIT_15,
            "24135564cd28079c5bebc9807eb38c548104e21a4dbd4ccbca3b1780f5a7b1f1",
        )
    };
    assert_eq!(digest(raw.as_bytes()), expected);
    let original: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert_eq!(
        original["scope"],
        "NormalProviderObservationNotGrpcAcceptance"
    );
    original["result"].clone()
}
fn synthetic_query_response(request: &pb::QueryRequest, op: pb::Operation) -> pb::QueryResponse {
    use serde_json::{json, Value};
    let id = &request.context.as_ref().unwrap().request_id;
    let payload = request.payload.as_ref().unwrap();
    let requested: Value = serde_json::from_slice(&payload.data).unwrap();
    let limit = requested["limit"].as_u64().unwrap() as usize;
    let result = if op == pb::Operation::HistoricalBars {
        original_historical_result(limit)
    } else {
        let request_shape: ProbeAnnouncementRequest =
            serde_json::from_value(requested.clone()).unwrap();
        let observed = "1790902884.244366600";
        let (total, raw, unique, returned) = if limit == 1 {
            (31_u64, 30_u64, 30_u64, 1_u64)
        } else {
            (2, 2, 2, 2)
        };
        let pages = vec![ProbeAnnouncementPage {
            requested_page: 1,
            source_total: total,
            source_total_pages: total / 30,
            has_more: raw < total,
            row_count: raw,
            request_body_sha256: probe_page_request_sha(&request_shape, 1),
            response_body_sha256: digest(
                b"TEST_CODE_SYNTHETIC_CNINFO_ORIGINAL_PAGE_BODY_NOT_PRESENT",
            ),
            response_bytes: 300,
        }];
        let pages_sha = digest(&serde_json::to_vec(&pages).unwrap());
        let batch_id = format!("cninfo:{observed}:market-announcements:2026-07-24:2026-07-24:pages=1:total={total}:limit={limit}:raw={raw}:unique={unique}:returned={returned}:pages-sha256={pages_sha}");
        let mut rows = Vec::new();
        for i in 0..returned {
            let published = format!("2026-07-24T15:{:02}:00+08:00", 30 - i);
            let announcement_id = format!("TEST_CODE_SYNTHETIC_{i}");
            let url = format!("https://www.cninfo.com.cn/new/disclosure/detail?stockCode=688561&announcementId={announcement_id}&orgId=TEST_CODE_SYNTHETIC_ORG&announcementTime=2026-07-24");
            rows.push(json!({
                "announcement_id":announcement_id,
                "instrument":{"exchange":"Shanghai","code":"688561","asset_class":"Equity"},
                "instrument_name":"TEST_CODE_SYNTHETIC_ONLY", "category":null,
                "title":"TEST_CODE_SYNTHETIC_TRANSPORT_CONTRACT_NOT_QUALIFIED", "published_at":published,
                "canonical_url":url, "pdf_url":null,
                "evidence":{"provider":"Cninfo","source_at":published,"observed_at":observed,"batch_id":batch_id},
            }));
        }
        let mut issues = Vec::new();
        if raw < total {
            issues.push(format!(
                "source pagination incomplete: inspected {raw} of {total} declared rows"
            ));
        }
        if returned < unique {
            issues.push(format!(
                "caller limit truncates {unique} inspected unique records to {returned}"
            ));
        }
        json!({
            "batch":{"records":rows,"provenance":{"source":"cninfo-market","source_at":rows[0]["published_at"],"fetched_at":observed,"batch_id":batch_id},
                "quality":{"complete":issues.is_empty(),"issues":issues}},
            "coverage":{"source_total":total,"expected_request_pages":total.div_ceil(30),"pages_read":1,
                "inspected_raw_rows":raw,"unique_rows":unique,"returned_rows":returned,"equivalent_duplicate_rows":0,
                "terminal_has_more":raw<total,"source_exhausted":raw==total,"caller_limit_truncated":returned<unique,
                "verified_empty":false,"pages":pages},
        })
    };
    let batch = &result["batch"];
    let records = if payload.schema_version == 1 {
        batch["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| pb::CanonicalPayload {
                schema: if op == pb::Operation::HistoricalBars {
                    "magic.market.bar"
                } else {
                    "magic.market.announcement"
                }
                .into(),
                schema_version: 1,
                content_type: JSON_CONTENT_TYPE.into(),
                data: serde_json::to_vec_pretty(row).unwrap(),
            })
            .collect()
    } else {
        let mut envelope = json!({"request_id":id,"request_payload_sha256":digest(&payload.data),"request":requested,
            "coverage_scope":if op == pb::Operation::HistoricalBars {"HithinkNativeDateRangeResponseObservationOnly"} else {"CninfoNativeDateRangeQuery"},
            "result":result});
        if op == pb::Operation::MarketAnnouncements {
            envelope["pit_guarantee"] = json!(false);
            envelope["exchange_event_universe_complete"] = json!(false);
        }
        vec![pb::CanonicalPayload {
            schema: if op == pb::Operation::HistoricalBars {
                "magic.market.historical_bars.coverage"
            } else {
                "magic.market.market_announcements.coverage"
            }
            .into(),
            schema_version: payload.schema_version,
            content_type: JSON_CONTENT_TYPE.into(),
            data: serde_json::to_vec_pretty(&envelope).unwrap(),
        }]
    };
    pb::QueryResponse {
        request_id: id.clone(),
        operation: op as i32,
        admission: pb::AdmissionState::Admitted as i32,
        selected_provider: request.preferred_provider.clone(),
        batch_id: batch["provenance"]["batch_id"].as_str().unwrap().into(),
        complete: batch["quality"]["complete"].as_bool().unwrap(),
        records,
        observed_at: batch["provenance"]["fetched_at"].as_str().unwrap().into(),
        source_at: batch["provenance"]["source_at"].as_str().unwrap().into(),
        diagnostic_blocker: String::new(),
    }
}
fn forge_impossible_historical_window_truncation(
    request: &pb::QueryRequest,
    response: &mut pb::QueryResponse,
) {
    use serde_json::{json, Value};
    let requested: ProbeHistoricalRequest =
        serde_json::from_slice(&request.payload.as_ref().unwrap().data).unwrap();
    let start = probe_date(&requested.start).unwrap();
    let end = probe_date(&requested.end).unwrap();
    assert_eq!(requested.limit, 15);
    assert_eq!(end.signed_duration_since(start).num_days() + 1, 15);
    let version = request.payload.as_ref().unwrap().schema_version;
    let original: Value = serde_json::from_slice(&response.records[0].data).unwrap();
    let template = if version == 1 {
        original.clone()
    } else {
        original["result"]["batch"]["records"][0].clone()
    };
    let rows: Vec<Value> = (0..15)
        .map(|offset| {
            let mut row = template.clone();
            let date = (start + chrono::Duration::days(offset)).to_string();
            row["bar_start"] = json!(date);
            row["bar_end"] = json!(date);
            row["source_at"] = json!(date);
            row
        })
        .collect();
    if version == 1 {
        let record = response.records[0].clone();
        response.records = rows
            .iter()
            .map(|row| pb::CanonicalPayload {
                data: serde_json::to_vec_pretty(row).unwrap(),
                ..record.clone()
            })
            .collect();
    } else {
        let mut envelope = original;
        envelope["result"]["batch"]["records"] = json!(rows);
        envelope["result"]["batch"]["quality"] = json!({
            "complete":false,
            "issues":["caller limit 15 retained 15 of 16 validated historical rows"]
        });
        envelope["result"]["coverage"]["validated_source_rows"] = json!(16);
        envelope["result"]["coverage"]["returned_rows"] = json!(15);
        envelope["result"]["coverage"]["caller_limit_truncated"] = json!(true);
        response.records[0].data = serde_json::to_vec_pretty(&envelope).unwrap();
    }
    response.complete = false;
}
fn synthetic_historical_start_day_observation(
    request: &pb::QueryRequest,
    response: &mut pb::QueryResponse,
    complete: bool,
) {
    use serde_json::{json, Value};
    let payload = request.payload.as_ref().unwrap();
    let requested: ProbeHistoricalRequest = serde_json::from_slice(&payload.data).unwrap();
    assert_eq!(requested.limit, 1);
    let start = probe_date(&requested.start).unwrap();
    let offset = chrono::FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let millis = start
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(offset)
        .single()
        .unwrap()
        .timestamp_millis();
    response.source_at = format!("unix-ms:{millis}");
    let mut original: Value = serde_json::from_slice(&response.records[0].data).unwrap();
    let row = if payload.schema_version == 1 {
        &mut original
    } else {
        &mut original["result"]["batch"]["records"][0]
    };
    row["bar_start"] = json!(requested.start);
    row["bar_end"] = json!(requested.start);
    row["source_at"] = json!(requested.start);
    if payload.schema_version == 2 {
        original["result"]["batch"]["provenance"]["source_at"] = json!(response.source_at);
        original["result"]["batch"]["quality"] = json!({
            "complete":complete,
            "issues": if complete {Vec::<String>::new()} else {vec!["caller limit 1 retained 1 of 2 validated historical rows".to_string()]}
        });
        original["result"]["coverage"]["validated_source_rows"] =
            json!(if complete { 1 } else { 2 });
        original["result"]["coverage"]["caller_limit_truncated"] = json!(!complete);
        original["result"]["coverage"]["native_response"]["timestamp_ms"] = json!(millis);
    }
    response.records[0].data = serde_json::to_vec_pretty(&original).unwrap();
    response.complete = complete;
}
fn forge_impossible_limit1_two_page_prefix(
    request: &pb::QueryRequest,
    response: &mut pb::QueryResponse,
) {
    use serde_json::json;
    let payload = request.payload.as_ref().unwrap();
    let requested: ProbeAnnouncementRequest = serde_json::from_slice(&payload.data).unwrap();
    assert_eq!(requested.limit, 1);
    let pages: Vec<_> = (1..=2)
        .map(|page| ProbeAnnouncementPage {
            requested_page: page,
            source_total: 61,
            source_total_pages: 2,
            has_more: true,
            row_count: 30,
            request_body_sha256: probe_page_request_sha(&requested, page),
            response_body_sha256: digest(format!("TEST_CODE_SYNTHETIC_PAGE_{page}").as_bytes()),
            response_bytes: 300,
        })
        .collect();
    let pages_sha = digest(&serde_json::to_vec(&pages).unwrap());
    let batch_id = format!("cninfo:{}:market-announcements:{}:{}:pages=2:total=61:limit=1:raw=60:unique=60:returned=1:pages-sha256={pages_sha}", response.observed_at, requested.start, requested.end);
    response.batch_id = batch_id.clone();
    response.complete = false;
    if payload.schema_version == 1 {
        mutate_query_json(response, |row| {
            row["evidence"]["batch_id"] = json!(batch_id)
        });
    } else {
        mutate_query_json(response, |envelope| {
            envelope["result"]["batch"]["provenance"]["batch_id"] = json!(batch_id);
            envelope["result"]["batch"]["records"][0]["evidence"]["batch_id"] = json!(batch_id);
            envelope["result"]["batch"]["quality"] = json!({"complete":false,"issues":[
                "source pagination incomplete: inspected 60 of 61 declared rows",
                "caller limit truncates 60 inspected unique records to 1"]});
            envelope["result"]["coverage"] = json!({"source_total":61,"expected_request_pages":3,"pages_read":2,
                "inspected_raw_rows":60,"unique_rows":60,"returned_rows":1,"equivalent_duplicate_rows":0,
                "terminal_has_more":true,"source_exhausted":false,"caller_limit_truncated":true,"verified_empty":false,"pages":pages});
        });
    }
}
fn forge_exhausted31_with_impossible_extra_pages(
    request: &pb::QueryRequest,
    response: &mut pb::QueryResponse,
) {
    use serde_json::{json, Value};
    let payload = request.payload.as_ref().unwrap();
    let requested: ProbeAnnouncementRequest = serde_json::from_slice(&payload.data).unwrap();
    assert_eq!(requested.limit, 300);
    let observed = response.observed_at.clone();
    let mut inspected = 0_u64;
    let pages: Vec<_> = (1..=10)
        .map(|page| {
            let count = (31 - inspected).min(30);
            inspected += count;
            ProbeAnnouncementPage {
                requested_page: page,
                source_total: 31,
                source_total_pages: 1,
                has_more: inspected < 31,
                row_count: count,
                request_body_sha256: probe_page_request_sha(&requested, page),
                response_body_sha256: digest(
                    format!("TEST_CODE_SYNTHETIC_EXHAUSTED_PAGE_{page}").as_bytes(),
                ),
                response_bytes: 300,
            }
        })
        .collect();
    assert_eq!(inspected, 31);
    let pages_sha = digest(&serde_json::to_vec(&pages).unwrap());
    let batch_id = format!("cninfo:{observed}:market-announcements:{}:{}:pages=10:total=31:limit=300:raw=31:unique=31:returned=31:pages-sha256={pages_sha}",requested.start,requested.end);
    let template: Value = if payload.schema_version == 1 {
        serde_json::from_slice(&response.records[0].data).unwrap()
    } else {
        serde_json::from_slice::<Value>(&response.records[0].data).unwrap()["result"]["batch"]
            ["records"][0]
            .clone()
    };
    let mut rows = Vec::new();
    for i in 0..31 {
        let mut row = template.clone();
        let published = format!("2026-07-24T15:{:02}:00+08:00", 30 - i);
        let id = format!("TEST_CODE_EXHAUSTED_ORIGINAL_SHAPE_{i}");
        row["announcement_id"] = json!(id);
        row["published_at"] = json!(published);
        row["canonical_url"]=json!(format!("https://www.cninfo.com.cn/new/disclosure/detail?stockCode=688561&announcementId={id}&orgId=TEST_CODE_SYNTHETIC_ORG&announcementTime=2026-07-24"));
        row["evidence"]["source_at"] = json!(published);
        row["evidence"]["batch_id"] = json!(batch_id);
        rows.push(row);
    }
    response.batch_id = batch_id.clone();
    response.complete = true;
    if payload.schema_version == 1 {
        response.records = rows
            .iter()
            .map(|row| pb::CanonicalPayload {
                schema: "magic.market.announcement".into(),
                schema_version: 1,
                content_type: JSON_CONTENT_TYPE.into(),
                data: serde_json::to_vec_pretty(row).unwrap(),
            })
            .collect();
    } else {
        mutate_query_json(response, |envelope| {
            envelope["result"]["batch"]["records"] = json!(rows);
            envelope["result"]["batch"]["provenance"]["batch_id"] = json!(batch_id);
            envelope["result"]["batch"]["quality"] = json!({"complete":true,"issues":[]});
            envelope["result"]["coverage"] = json!({"source_total":31,"expected_request_pages":2,"pages_read":10,
                "inspected_raw_rows":31,"unique_rows":31,"returned_rows":31,"equivalent_duplicate_rows":0,
                "terminal_has_more":false,"source_exhausted":true,"caller_limit_truncated":false,"verified_empty":false,"pages":pages});
        });
    }
}
fn synthetic_zero_returned_announcement_observation(
    request: &pb::QueryRequest,
    response: &mut pb::QueryResponse,
    actual_empty: bool,
) {
    use serde_json::json;
    let payload = request.payload.as_ref().unwrap();
    let requested: ProbeAnnouncementRequest = serde_json::from_slice(&payload.data).unwrap();
    let total = if actual_empty { 0_u64 } else { 31_u64 };
    let count = if actual_empty { 1 } else { 2 };
    let mut inspected = 0_u64;
    let pages: Vec<_> = (1..=count)
        .map(|page| {
            let rows = (total - inspected).min(30);
            inspected += rows;
            ProbeAnnouncementPage {
                requested_page: page,
                source_total: total,
                source_total_pages: total / 30,
                has_more: inspected < total,
                row_count: rows,
                request_body_sha256: probe_page_request_sha(&requested, page),
                response_body_sha256: digest(
                    format!("TEST_CODE_SYNTHETIC_ZERO_RETURNED_PAGE_{page}").as_bytes(),
                ),
                response_bytes: 300,
            }
        })
        .collect();
    let pages_sha = digest(&serde_json::to_vec(&pages).unwrap());
    let batch_id=format!("cninfo:{}:market-announcements:{}:{}:pages={count}:total={total}:limit={}:raw={total}:unique=0:returned=0:pages-sha256={pages_sha}",response.observed_at,requested.start,requested.end,requested.limit);
    response.batch_id = batch_id.clone();
    response.complete = actual_empty;
    response.source_at.clear();
    if payload.schema_version == 1 {
        response.records.clear();
    } else {
        mutate_query_json(response, |envelope| {
            envelope["result"]["batch"]["records"] = json!([]);
            envelope["result"]["batch"]["provenance"]["batch_id"] = json!(batch_id);
            envelope["result"]["batch"]["provenance"]["source_at"] = json!(null);
            let issues: Vec<_> = if actual_empty {
                Vec::new()
            } else {
                vec!["source identity overlap: 31 equivalent duplicate rows cannot prove complete unique coverage"]
            };
            envelope["result"]["batch"]["quality"] =
                json!({"complete":actual_empty,"issues":issues});
            envelope["result"]["coverage"] = json!({"source_total":total,"expected_request_pages":if actual_empty {1} else {2},"pages_read":count,
                "inspected_raw_rows":total,"unique_rows":0,"returned_rows":0,"equivalent_duplicate_rows":total,
                "terminal_has_more":false,"source_exhausted":true,"caller_limit_truncated":false,"verified_empty":actual_empty,"pages":pages});
        });
    }
}
fn mutate_query_json(
    response: &mut pb::QueryResponse,
    change: impl FnOnce(&mut serde_json::Value),
) {
    let mut value = serde_json::from_slice(&response.records[0].data).unwrap();
    change(&mut value);
    response.records[0].data = serde_json::to_vec_pretty(&value).unwrap();
}

struct QueryUnary {
    service: FixtureService,
    op: pb::Operation,
}
impl UnaryService<pb::QueryRequest> for QueryUnary {
    type Response = Vec<u8>;
    type Future = BoxFuture<tonic::Response<Vec<u8>>, tonic::Status>;
    fn call(&mut self, req: tonic::Request<pb::QueryRequest>) -> Self::Future {
        let service = self.service.clone();
        let op = self.op;
        Box::pin(async move {
            if !capture_request(
                &service.state,
                if op == pb::Operation::HistoricalBars {
                    "HistoricalBars"
                } else {
                    "MarketAnnouncements"
                },
                &req,
            ) {
                return Err(tonic::Status::unauthenticated("TEST_CODE_AUTH_REQUIRED"));
            }
            let request = req.into_inner();
            let id = request.context.as_ref().unwrap().request_id.clone();
            let payload = request.payload.as_ref().unwrap();
            let case = service.state.lock().unwrap().case;
            if payload.schema_version == 3 && case != Case::UnexpectedSchemaSuccess {
                return Err(if case == Case::SchemaProviderFailure {
                    failed(&service.state, tonic::Code::InvalidArgument, &id, op as i32)
                } else {
                    typed_status(
                        &service.state,
                        tonic::Code::InvalidArgument,
                        schema_rejection(&id),
                    )
                });
            }
            if case == Case::BusinessFailure {
                return Err(failed(
                    &service.state,
                    tonic::Code::Unavailable,
                    &id,
                    op as i32,
                ));
            }
            let mut response = synthetic_query_response(&request, op);
            if op == pb::Operation::HistoricalBars
                && serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap()["limit"] == 1
                && (case == Case::HistoricalStartDayPartial && payload.schema_version == 1
                    || case == Case::HistoricalV2StartDayPartial && payload.schema_version == 2
                    || case == Case::HistoricalStartDayStrict)
            {
                synthetic_historical_start_day_observation(
                    &request,
                    &mut response,
                    case == Case::HistoricalStartDayStrict,
                );
            }
            if op == pb::Operation::HistoricalBars
                && serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap()["limit"]
                    == 15
                && (case == Case::HistoricalFullWindowFalse && payload.schema_version == 1
                    || case == Case::HistoricalSourceBeyondWindow && payload.schema_version == 2)
            {
                forge_impossible_historical_window_truncation(&request, &mut response);
            }
            if op == pb::Operation::HistoricalBars && payload.schema_version == 1 {
                if case == Case::HistoricalUnderLimitFalse
                    && serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap()["limit"]
                        == 15
                {
                    assert_eq!(response.records.len(), 11);
                    assert!(response.complete);
                    response.complete = false;
                }
                if case == Case::HistoricalEmptyFalse {
                    response.records.clear();
                    response.complete = false;
                }
                if case == Case::VerifiedHistoricalEmpty {
                    response.records.clear();
                    response.complete = true;
                }
            }
            if case == Case::WrongQueryId {
                response.request_id = "TEST_CODE_FOREIGN_QUERY_ID".into();
            }
            if op == pb::Operation::MarketAnnouncements {
                if case == Case::VerifiedSourceEmpty {
                    synthetic_zero_returned_announcement_observation(&request, &mut response, true);
                }
                if case == Case::NonemptyWithZeroUnique
                    && serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap()["limit"]
                        == 300
                {
                    synthetic_zero_returned_announcement_observation(
                        &request,
                        &mut response,
                        false,
                    );
                }
            }
            if case == Case::ExhaustedExtraPages
                && op == pb::Operation::MarketAnnouncements
                && serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap()["limit"]
                    == 300
            {
                forge_exhausted31_with_impossible_extra_pages(&request, &mut response);
            }
            if payload.schema_version == 2 {
                if case == Case::WrongInnerId {
                    mutate_query_json(&mut response, |json| {
                        json["request_id"] = "TEST_CODE_OTHER_INNER_ID".into()
                    });
                }
                if case == Case::WrongPayloadHash {
                    mutate_query_json(&mut response, |json| {
                        json["request_payload_sha256"] = "0".repeat(64).into()
                    });
                }
                if case == Case::BadPrefix && op == pb::Operation::MarketAnnouncements {
                    forge_impossible_limit1_two_page_prefix(&request, &mut response);
                }
            }
            let mut raw = response.encode_to_vec();
            if payload.schema_version == 1
                || case == Case::UnknownHistoricalV2 && op == pb::Operation::HistoricalBars
            {
                raw.extend_from_slice(&[0xfa, 0x07, 4, b'T', b'E', b'S', b'T']);
                // v1 legal unknown field127 remains original evidence
            }
            reply(&service.state, raw)
        })
    }
}

#[derive(Default)]
struct RawCodec<Q>(PhantomData<Q>);
struct RawEncoder;
impl tonic::codec::Encoder for RawEncoder {
    type Item = Vec<u8>;
    type Error = tonic::Status;
    fn encode(
        &mut self,
        item: Vec<u8>,
        dest: &mut tonic::codec::EncodeBuf<'_>,
    ) -> Result<(), Self::Error> {
        use prost::bytes::BufMut;
        dest.put_slice(&item);
        Ok(())
    }
}
impl<Q: Message + Default + Send + 'static> tonic::codec::Codec for RawCodec<Q> {
    type Encode = Vec<u8>;
    type Decode = Q;
    type Encoder = RawEncoder;
    type Decoder = tonic_prost::ProstDecoder<Q>;
    fn encoder(&mut self) -> Self::Encoder {
        RawEncoder
    }
    fn decoder(&mut self) -> Self::Decoder {
        tonic_prost::ProstDecoder::new(tonic::codec::BufferSettings::default())
    }
}
#[derive(Clone)]
struct SystemServer(FixtureService);
impl NamedService for SystemServer {
    const NAME: &'static str = "magic.market.v1.SystemService";
}
impl<B: Body + Send + 'static> Service<http::Request<B>> for SystemServer
where
    B::Error: Into<StdError> + Send + 'static,
{
    type Response = http::Response<tonic::body::Body>;
    type Error = Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        let service = self.0.clone();
        let health = request.uri().path().ends_with("/GetHealth");
        Box::pin(async move {
            let response = if health {
                tonic::server::Grpc::new(RawCodec::<pb::HealthRequest>::default())
                    .unary(HealthUnary(service), request)
                    .await
            } else {
                tonic::server::Grpc::new(RawCodec::<pb::CapabilitiesRequest>::default())
                    .unary(CapsUnary(service), request)
                    .await
            };
            Ok(response)
        })
    }
}
#[derive(Clone)]
struct DataServer(FixtureService);
impl NamedService for DataServer {
    const NAME: &'static str = "magic.market.v1.MarketDataService";
}
impl<B: Body + Send + 'static> Service<http::Request<B>> for DataServer
where
    B::Error: Into<StdError> + Send + 'static,
{
    type Response = http::Response<tonic::body::Body>;
    type Error = Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        let service = self.0.clone();
        let op = if request.uri().path().ends_with("/HistoricalBars") {
            pb::Operation::HistoricalBars
        } else {
            pb::Operation::MarketAnnouncements
        };
        Box::pin(async move {
            Ok(
                tonic::server::Grpc::new(RawCodec::<pb::QueryRequest>::default())
                    .unary(QueryUnary { service, op }, request)
                    .await,
            )
        })
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    bundle: std::path::PathBuf,
    endpoint: String,
    state: Arc<Mutex<State>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), tonic::transport::Error>>>,
}
impl Fixture {
    async fn bind(case: Case) -> Self {
        use tokio_stream::StreamExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("https://{}", listener.local_addr().unwrap());
        let temp = tempfile::tempdir().unwrap();
        let bundle = write_test_code_bundle(
            temp.path(),
            "TEST_CODE_candidate",
            &endpoint,
            "macro.test.invalid",
        )
        .unwrap();
        let state = Arc::new(Mutex::new(State {
            case,
            ..Default::default()
        }));
        let accept_state = state.clone();
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener).map(move |c| {
            if c.is_ok() {
                accept_state.lock().unwrap().observation.tcp += 1;
            }
            c
        });
        let tls = ServerTlsConfig::new()
            .identity(Identity::from_pem(
                TEST_CODE_MTLS_SERVER_CERT,
                TEST_CODE_MTLS_SERVER_KEY,
            ))
            .client_ca_root(Certificate::from_pem(TEST_CODE_MTLS_CA_CERT))
            .client_auth_optional(false);
        let service = FixtureService {
            state: state.clone(),
        };
        let (shutdown, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .tls_config(tls)
                .unwrap()
                .add_service(SystemServer(service.clone()))
                .add_service(DataServer(service))
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = receive.await;
                })
                .await
        });
        Self {
            _temp: temp,
            bundle,
            endpoint,
            state,
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }
    fn prepared(&self) -> PreparedExternalEndpoint {
        let config = GrpcMarketClient::load_bundle_config(&self.bundle).unwrap();
        GrpcMarketClient::prepare_loaded_bundle(
            config,
            BuildIdentityTrust::candidate_b7_probe().unwrap(),
        )
        .unwrap()
    }
    fn snapshot(&self) -> Observation {
        self.state.lock().unwrap().observation.clone()
    }
    async fn finish(mut self) {
        if let Some(s) = self.shutdown.take() {
            let _ = s.send(());
        }
        if let Some(mut t) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(5), &mut t).await {
                Ok(result) => result.unwrap().unwrap(),
                Err(_) => {
                    t.abort();
                    let _ = t.await;
                    panic!("TEST_CODE fixture shutdown timed out");
                }
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(s) = self.shutdown.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

async fn run_case(case: Case) -> (CandidateProbeRunReceipt, Observation, String) {
    let fixture = Fixture::bind(case).await;
    let plan = CandidateProbePlan::windows_b7().unwrap();
    let receipt = tokio::time::timeout(
        Duration::from_secs(15),
        plan.execute_prepared(fixture.prepared()),
    )
    .await
    .unwrap()
    .unwrap();
    let observed = fixture.snapshot();
    let endpoint = fixture.endpoint.clone();
    fixture.finish().await;
    (receipt, observed, endpoint)
}

#[tokio::test]
async fn candidate_b7_probe_real_mtls_all36_fixed_requests_one_dial_and_original_bytes_remain_recorded_only(
) {
    let (receipt, observed, endpoint) = run_case(Case::Success).await;
    assert_eq!(receipt.outcome_name(), "CompletedRecordedObservation");
    assert_eq!(receipt.rpc_count(), 36);
    assert_eq!(observed.tcp, 1);
    assert_eq!(observed.methods.len(), 36);
    for (step, request) in receipt.dto.plan.steps.0.iter().zip(&observed.requests) {
        assert_eq!(request, &hex::decode(&step.request_wire_hex).unwrap());
    }
    assert!(observed
        .auth
        .iter()
        .enumerate()
        .all(|(i, a)| *a == (i != 2)));
    assert_eq!(observed.methods[2], "Health");
    assert_eq!(observed.methods[3], "MarketAnnouncements");
    assert!(
        !receipt.dto.connection_identity.validate_recorded(),
        "candidate receipt cannot enter production replay"
    );
    let bytes = receipt.canonical_bytes().unwrap();
    let evidence =
        read_receipt_on_target(&bytes, &digest(&bytes), &endpoint, TEST_AUTHORITY).unwrap();
    assert_eq!(evidence.rpc_count(), 36);
    assert!(
        read_candidate_b7_receipt(&bytes, &digest(&bytes)).is_err(),
        "production reader rejects loopback target"
    );
    let mut raw = RawRead::new(&receipt.arena, &receipt.dto.parts.0).unwrap();
    let mut server_responses = observed.responses.iter();
    for step in &receipt.dto.steps.0 {
        match &step.material {
            Material::ControlResponse { part } => {
                assert_eq!(raw.part(*part).unwrap(), server_responses.next().unwrap())
            }
            Material::QueryResponse { wire } => {
                let w = raw.wire(wire, wire.method).unwrap();
                assert_eq!(w.payload().unwrap(), server_responses.next().unwrap());
                let original_request = pb::QueryRequest::decode(
                    hex::decode(&receipt.dto.plan.steps.0[step.ordinal].request_wire_hex)
                        .unwrap()
                        .as_slice(),
                )
                .unwrap();
                let has_original_extension = original_request.payload.unwrap().schema_version == 1;
                assert_eq!(
                    w.payload()
                        .unwrap()
                        .ends_with(&[0xfa, 0x07, 4, b'T', b'E', b'S', b'T']),
                    has_original_extension
                );
            }
            Material::TonicStatus {
                wire,
                message,
                details,
                trailer,
                ..
            } => {
                if let Some(w) = wire {
                    raw.wire(w, w.method).unwrap();
                }
                raw.part(*message).unwrap();
                raw.part(*details).unwrap();
                if let Trailer::Bytes { part } = trailer {
                    raw.part(*part).unwrap();
                }
            }
            _ => panic!("unexpected positive fixture material"),
        }
    }
    assert_eq!(raw.next, receipt.dto.parts.0.len());
}

#[tokio::test]
async fn candidate_b7_probe_real_identity_caps_and_query_conflicts_stop_first_without_health_after()
{
    for (case, count) in [
        (Case::Old67, 1),
        (Case::WrongHealthId, 1),
        (Case::AuthenticatedHealthDenied, 1),
        (Case::WrongCapsId, 2),
        (Case::UnadmittedCaps, 2),
        (Case::SchemaProviderFailure, 4),
        (Case::WrongQueryId, 7),
        (Case::LastHealthStatus, 36),
    ] {
        let (receipt, observed, endpoint) = run_case(case).await;
        assert_eq!(receipt.outcome_name(), "StoppedRecordedObservation");
        assert_eq!(receipt.rpc_count(), count);
        assert_eq!(observed.methods.len(), count);
        assert_eq!(observed.tcp, 1);
        assert!(!receipt.dto.steps.0.last().unwrap().outcome.continues());
        let bytes = receipt.canonical_bytes().unwrap();
        assert_eq!(
            read_receipt_on_target(&bytes, &digest(&bytes), &endpoint, TEST_AUTHORITY)
                .unwrap()
                .rpc_count(),
            count
        );
    }
}

#[tokio::test]
async fn candidate_b7_probe_real_typed_failure_keeps_exact_status_trailer_and_stops_before_health_after(
) {
    let (receipt, observed, endpoint) = run_case(Case::BusinessFailure).await;
    assert_eq!(receipt.rpc_count(), 7);
    assert_eq!(
        observed.methods,
        vec![
            "Health",
            "Capabilities",
            "Health",
            "MarketAnnouncements",
            "Health",
            "Capabilities",
            "HistoricalBars"
        ]
    );
    let last = receipt.dto.steps.0.last().unwrap();
    assert_eq!(last.outcome, StepOutcome::StoppedStatus);
    let bytes = receipt.canonical_bytes().unwrap();
    assert_eq!(
        read_receipt_on_target(&bytes, &digest(&bytes), &endpoint, TEST_AUTHORITY)
            .unwrap()
            .outcome_name(),
        "StoppedRecordedObservation"
    );
    let mut raw = RawRead::new(&receipt.arena, &receipt.dto.parts.0).unwrap();
    for s in &receipt.dto.steps.0[..6] {
        assert_eq!(
            read_step_material(
                &receipt.dto.plan.steps.0[s.ordinal],
                &s.material,
                &mut raw,
                TEST_AUTHORITY,
            )
            .unwrap(),
            s.outcome
        );
    }
    if let Material::TonicStatus {
        wire,
        code,
        message,
        details,
        trailer,
        matched_request_id_correlation,
        ..
    } = &last.material
    {
        assert_eq!(*code, 14);
        raw.wire(wire.as_ref().unwrap(), ExternalQueryMethod::HistoricalBars)
            .unwrap();
        assert_eq!(
            raw.part(*message).unwrap(),
            b"TEST_CODE_REMOTE_SENSITIVE_DETAIL_DO_NOT_PRINT"
        );
        assert_eq!(
            raw.part(*details).unwrap(),
            observed.statuses.last().unwrap().1
        );
        if let Trailer::Bytes { part } = trailer {
            assert_eq!(
                raw.part(*part).unwrap(),
                observed.statuses.last().unwrap().2
            );
        } else {
            panic!("exact trailer missing");
        }
        assert_eq!(
            *matched_request_id_correlation,
            crate::grpc_client::errors::request_id_correlation(
                &receipt.dto.plan.steps.0[6].request_id
            )
        );
    } else {
        panic!("raw typed status missing");
    }
}

#[tokio::test]
async fn candidate_b7_probe_real_health_status_clears_old_qualification_and_cold_caps_make_zero_rpc(
) {
    let fixture = Fixture::bind(Case::SecondHealthStatus).await;
    let prepared = fixture.prepared();
    let generation = prepared.plan_connection_generation();
    let mut client = prepared
        .connect_generation(generation.clone())
        .await
        .unwrap();
    let plan = CandidateProbePlan::windows_b7().unwrap();
    let mut raw = RawBuilder::new();
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[0], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::ObservedSuccess
    );
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[1], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::ObservedSuccess
    );
    assert!(client.require_external_qualification().is_ok());
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[4], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::StoppedStatus
    );
    assert!(client.require_external_qualification().is_err());
    assert!(generation
        .observe_health(
            &plan.dto.steps.0[0].request_id,
            &candidate_health(&plan.dto.steps.0[0].request_id),
        )
        .is_err());
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[5], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::StoppedLocalGate
    );
    assert_eq!(
        fixture.snapshot().methods,
        vec!["Health", "Capabilities", "Health"]
    );
    drop(client);
    fixture.finish().await;
    let (receipt, observed, _) = run_case(Case::SecondHealthStatus).await;
    assert_eq!(receipt.rpc_count(), 5);
    assert_eq!(observed.methods.len(), 5);
}

#[tokio::test]
async fn candidate_b7_probe_real_negative_unexpected_ok_stops_and_auth_status_never_restores_qualification(
) {
    for (case, count) in [
        (Case::UnexpectedUnauthenticatedSuccess, 3),
        (Case::UnexpectedSchemaSuccess, 4),
    ] {
        let (receipt, observed, endpoint) = run_case(case).await;
        assert_eq!(receipt.outcome_name(), "StoppedRecordedObservation");
        assert_eq!(receipt.rpc_count(), count);
        assert_eq!(observed.methods.len(), count);
        assert_eq!(
            receipt.dto.steps.0.last().unwrap().outcome,
            StepOutcome::StoppedUnexpectedNegativeResponse
        );
        let bytes = receipt.canonical_bytes().unwrap();
        assert_eq!(
            read_receipt_on_target(&bytes, &digest(&bytes), &endpoint, TEST_AUTHORITY)
                .unwrap()
                .outcome_name(),
            "StoppedRecordedObservation"
        );
    }
    let fixture = Fixture::bind(Case::Success).await;
    let prepared = fixture.prepared();
    let mut client = prepared.connect_once().await.unwrap();
    let plan = CandidateProbePlan::windows_b7().unwrap();
    let mut raw = RawBuilder::new();
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[0], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::ObservedSuccess
    );
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[2], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::ExpectedUnauthenticatedStatus
    );
    assert!(client.require_external_qualification().is_err());
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[6], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::StoppedLocalGate
    );
    assert_eq!(fixture.snapshot().methods, vec!["Health", "Health"]);
    drop(client);
    fixture.finish().await;
}

#[tokio::test]
async fn candidate_b7_probe_real_disconnect_cannot_redial_or_requalify_same_generation() {
    let a = Fixture::bind(Case::Success).await;
    let b = Fixture::bind(Case::Success).await;
    let switch = ExternalMtlsSwitch::bind(&a.endpoint).await;
    let config = GrpcMarketClient::load_bundle_config(switch.bundle_path()).unwrap();
    let prepared = GrpcMarketClient::prepare_loaded_bundle(
        config,
        BuildIdentityTrust::candidate_b7_probe().unwrap(),
    )
    .unwrap();
    let generation = prepared.plan_connection_generation();
    let mut client = prepared
        .connect_generation(generation.clone())
        .await
        .unwrap();
    let plan = CandidateProbePlan::windows_b7().unwrap();
    let mut raw = RawBuilder::new();
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[0], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::ObservedSuccess
    );
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[1], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::ObservedSuccess
    );
    switch.switch_to(&b.endpoint).await;
    let failed = tokio::time::timeout(
        Duration::from_secs(5),
        execute_step(&mut client, &plan.dto.steps.0[4], &mut raw),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!failed.outcome.continues());
    assert!(client.require_external_qualification().is_err());
    assert!(generation
        .observe_health(
            &plan.dto.steps.0[0].request_id,
            &candidate_health(&plan.dto.steps.0[0].request_id)
        )
        .is_err());
    assert_eq!(
        execute_step(&mut client, &plan.dto.steps.0[6], &mut raw)
            .await
            .unwrap()
            .outcome,
        StepOutcome::StoppedLocalGate
    );
    assert_eq!(a.snapshot().tcp, 1);
    assert_eq!(b.snapshot().tcp, 0);
    assert!(b.snapshot().methods.is_empty());
    drop(client);
    drop(prepared);
    drop(switch);
    a.finish().await;
    b.finish().await;
}

#[tokio::test]
async fn candidate_b7_probe_fixed_production_bundle_target_is_checked_before_any_dial() {
    let fixture = Fixture::bind(Case::Success).await;
    let plan = CandidateProbePlan::windows_b7().unwrap();
    assert_eq!(
        plan.execute_from_bundle(&fixture.bundle).await.err(),
        Some(CandidateProbeError::BundleInput)
    );
    assert_eq!(fixture.snapshot().tcp, 0);
    assert!(fixture.snapshot().methods.is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn candidate_b7_probe_real_tls_failure_records_one_connect_without_health_or_fallback() {
    let fixture = Fixture::bind(Case::Success).await;
    let plan = CandidateProbePlan::windows_b7().unwrap();
    let mut config = GrpcMarketClient::load_bundle_config(&fixture.bundle).unwrap();
    config.tls_server_name = "wrong.test.invalid".into();
    let prepared = GrpcMarketClient::prepare_loaded_bundle(
        config,
        BuildIdentityTrust::candidate_b7_probe().unwrap(),
    )
    .unwrap();
    let receipt = plan.execute_prepared(prepared).await.unwrap();
    assert_eq!(receipt.outcome_name(), "StoppedRecordedObservation");
    assert_eq!(receipt.dto.connect_invocations, 1);
    assert!(receipt.dto.connect_error_class.is_some());
    assert_eq!(receipt.rpc_count(), 0);
    assert_eq!(fixture.snapshot().tcp, 1);
    assert!(fixture.snapshot().methods.is_empty());
    let bytes = receipt.canonical_bytes().unwrap();
    assert_eq!(
        read_receipt_on_target(
            &bytes,
            &digest(&bytes),
            &fixture.endpoint,
            "grpc-mtls:wrong.test.invalid"
        )
        .unwrap()
        .rpc_count(),
        0
    );
    fixture.finish().await;
}

fn pack(dto: &ReceiptDto, arena: &[u8]) -> Vec<u8> {
    let json = serde_json::to_vec(dto).unwrap();
    let mut b = MAGIC.to_vec();
    b.extend_from_slice(&(json.len() as u32).to_be_bytes());
    b.extend_from_slice(&json);
    b.extend_from_slice(arena);
    b
}

#[tokio::test]
async fn candidate_b7_probe_recorded_reader_rejects_raw_tamper_alias_missing_membership_and_claimed_completion(
) {
    let (receipt, _, endpoint) = run_case(Case::Success).await;
    let original = receipt.canonical_bytes().unwrap();
    let mut raw_tamper = original.clone();
    *raw_tamper.last_mut().unwrap() ^= 1;
    assert!(
        read_receipt_on_target(&raw_tamper, &digest(&original), &endpoint, TEST_AUTHORITY).is_err()
    );
    assert!(
        read_receipt_on_target(&raw_tamper, &digest(&raw_tamper), &endpoint, TEST_AUTHORITY)
            .is_err()
    );
    for mutation in 0..8 {
        let mut dto = receipt.dto.clone();
        let arena = receipt.arena.clone();
        match mutation {
            0 => {
                dto.connection_identity.policy_sha256 = BuildIdentityTrust::bundled()
                    .unwrap()
                    .current_policy_sha256()
            }
            1 => dto.connection_identity.descriptor_sha256 = "0".repeat(64),
            2 => dto.connection_identity.epoch = "".into(),
            3 => dto.steps.0.swap(0, 1),
            4 => {
                if let Material::ControlResponse { part } = &mut dto.steps.0[4].material {
                    *part = 0
                }
            }
            5 => {
                dto.parts.0.push(PartMeta {
                    byte_length: 0,
                    sha256: digest(b""),
                });
            }
            6 => {
                dto.steps.0.pop();
            }
            _ => dto.connect_error_class = Some(ErrorClass::Unavailable),
        }
        let b = pack(&dto, &arena);
        assert!(read_receipt_on_target(&b, &digest(&b), &endpoint, TEST_AUTHORITY).is_err());
    }
    let (failed, _, failed_target) = run_case(Case::BusinessFailure).await;
    let mut dto = failed.dto.clone();
    dto.outcome = RunOutcome::CompletedRecordedObservation;
    let b = pack(&dto, &failed.arena);
    assert!(read_receipt_on_target(&b, &digest(&b), &failed_target, TEST_AUTHORITY).is_err());
    let mut unknown = serde_json::to_value(&receipt.dto).unwrap();
    unknown["LiveCapture"] = true.into();
    let json = serde_json::to_vec(&unknown).unwrap();
    let mut b = MAGIC.to_vec();
    b.extend_from_slice(&(json.len() as u32).to_be_bytes());
    b.extend_from_slice(&json);
    b.extend_from_slice(&receipt.arena);
    assert!(read_receipt_on_target(&b, &digest(&b), &endpoint, TEST_AUTHORITY).is_err());
    let mut duplicate = serde_json::to_vec(&receipt.dto).unwrap();
    duplicate.splice(1..1, b"\"version\":1,".iter().copied());
    let mut b = MAGIC.to_vec();
    b.extend_from_slice(&(duplicate.len() as u32).to_be_bytes());
    b.extend_from_slice(&duplicate);
    b.extend_from_slice(&receipt.arena);
    assert!(read_receipt_on_target(&b, &digest(&b), &endpoint, TEST_AUTHORITY).is_err());
    let mut b = MAGIC.to_vec();
    b.extend_from_slice(&((MAX_MANIFEST_BYTES + 1) as u32).to_be_bytes());
    assert_eq!(
        read_receipt_on_target(&b, &digest(&b), &endpoint, TEST_AUTHORITY).err(),
        Some(CandidateProbeError::MaterialBound)
    );
}

fn typed_response_outcome(step: &PlannedStep, raw: Vec<u8>) -> StepOutcome {
    let (_, method, _, _, _) = business_recipe(step_spec(step.ordinal).business.unwrap());
    let wire = ExternalWireEvidenceV1 {
        material: "external-unary-response-evidence-v1".into(),
        profile: "ExternalV1".into(),
        method,
        client_descriptor_sha256: EXTERNAL_V1_CLIENT_DESCRIPTOR_SHA256.into(),
        evidence: ExternalWireMaterialV1::Payload {
            payload_sha256: digest(&raw),
            protobuf_payload: raw,
            decode_limit_bytes: EXTERNAL_QUERY_DECODE_LIMIT_BYTES,
        },
    };
    query_outcome(step, &wire, TEST_AUTHORITY)
}
#[test]
fn candidate_b7_probe_typed_partial_observations_preserve_false_and_reject_binding_prefix_and_row_conflicts(
) {
    use serde_json::json;
    let plan = CandidateProbePlan::windows_b7().unwrap();
    for index in 0..8 {
        let step = &plan.dto.steps.0[6 + index * 4];
        let request =
            pb::QueryRequest::decode(hex::decode(&step.request_wire_hex).unwrap().as_slice())
                .unwrap();
        let (op, _, _, version, limit) = business_recipe(index);
        let response = synthetic_query_response(&request, op);
        assert_eq!(
            response.complete,
            limit != 1,
            "original batch quality remains partial for both providers' limit1"
        );
        assert_eq!(
            typed_response_outcome(step, response.encode_to_vec()),
            StepOutcome::ObservedSuccess
        );
        if op == pb::Operation::HistoricalBars && limit == 1 {
            let mut impossible = response.clone();
            synthetic_historical_start_day_observation(&request, &mut impossible, false);
            if version == 2 {
                assert!(crate::data_gateway::parse_recorded_historical_coverage_v2(
                    &request.encode_to_vec(), &impossible.encode_to_vec(),
                ).is_ok(), "the unchanged general recorded reader accepts the otherwise coherent start-day shape");
            }
            assert_eq!(
                typed_response_outcome(step, impossible.encode_to_vec()),
                StepOutcome::StoppedEnvelope,
                "a retained starting day leaves no earlier in-range unique day to truncate"
            );
            let mut strict = response.clone();
            synthetic_historical_start_day_observation(&request, &mut strict, true);
            assert_eq!(
                typed_response_outcome(step, strict.encode_to_vec()),
                StepOutcome::ObservedSuccess,
                "the same start-day record and native timestamp are valid when the source has one row"
            );
        }
        if op == pb::Operation::HistoricalBars && limit == 15 {
            let mut impossible = response.clone();
            forge_impossible_historical_window_truncation(&request, &mut impossible);
            if version == 1 {
                assert_eq!(impossible.records.len(), 15);
                let mut strict = impossible.clone();
                strict.complete = true;
                assert_eq!(
                    typed_response_outcome(step, strict.encode_to_vec()),
                    StepOutcome::ObservedSuccess,
                    "fifteen unique in-range calendar days have otherwise valid original row bindings"
                );
            } else {
                assert!(crate::data_gateway::parse_recorded_historical_coverage_v2(
                    &request.encode_to_vec(),
                    &impossible.encode_to_vec(),
                )
                .is_ok(), "the unchanged general recorded reader does not enforce the candidate's native unique-day source bound");
            }
            assert_eq!(
                typed_response_outcome(step, impossible.encode_to_vec()),
                StepOutcome::StoppedEnvelope,
                "a fifteen-day window cannot contain a sixteenth validated unique native source day"
            );
        }
        for mutation in 0..5 {
            let mut bad = response.clone();
            match mutation {
                0 => bad.records[0].schema = "TEST_CODE_UNKNOWN_RECORD_SCHEMA".into(),
                1 => bad.records[0].schema_version = 999,
                2 => bad.records[0].content_type = "text/plain".into(),
                3 => bad.selected_provider = "TEST_CODE_OTHER_PROVIDER".into(),
                _ => bad.request_id = "TEST_CODE_OTHER_OUTER_REQUEST".into(),
            }
            assert_eq!(
                typed_response_outcome(step, bad.encode_to_vec()),
                StepOutcome::StoppedEnvelope,
                "index={index} mutation={mutation}"
            );
        }
        if version == 1 {
            if op == pb::Operation::HistoricalBars {
                let mut empty = response.clone();
                empty.records.clear();
                empty.complete = true;
                assert_eq!(
                    typed_response_outcome(step, empty.encode_to_vec()),
                    StepOutcome::ObservedSuccess,
                    "native empty source returns a strict batch, not caller-limit truncation"
                );
                empty.complete = false;
                assert_eq!(
                    typed_response_outcome(step, empty.encode_to_vec()),
                    StepOutcome::StoppedEnvelope,
                    "an empty returned batch cannot be native caller-limit truncation"
                );
                empty.complete = true;
                empty.source_at.clear();
                assert_eq!(
                    typed_response_outcome(step, empty.encode_to_vec()),
                    StepOutcome::StoppedEnvelope,
                    "native batch provenance still requires the positive upstream timestamp when empty"
                );
                if limit == 15 {
                    let mut impossible = response.clone();
                    assert!(impossible.complete);
                    assert_eq!(impossible.records.len(), 11);
                    impossible.complete = false;
                    assert_eq!(
                        typed_response_outcome(step, impossible.encode_to_vec()),
                        StepOutcome::StoppedEnvelope,
                        "the same eleven valid original rows are not a truncated fifteen-row result"
                    );
                }
            }
            let mut bad = response.clone();
            mutate_query_json(&mut bad, |row| {
                if op == pb::Operation::HistoricalBars {
                    row["high"] = json!(0.01);
                } else {
                    row["evidence"]["batch_id"] = json!("TEST_CODE_OTHER_BATCH");
                }
            });
            assert_eq!(
                typed_response_outcome(step, bad.encode_to_vec()),
                StepOutcome::StoppedEnvelope
            );
            let mut excess = response.clone();
            while excess.records.len() <= limit as usize {
                excess.records.push(response.records[0].clone());
            }
            assert_eq!(
                typed_response_outcome(step, excess.encode_to_vec()),
                StepOutcome::StoppedEnvelope
            );
        } else {
            for (path, value) in [
                ("/request_id", json!("TEST_CODE_OTHER_INNER_REQUEST")),
                ("/request_payload_sha256", json!("0".repeat(64))),
                ("/request/limit", json!(999)),
                ("/request/start", json!("2026-07-25")),
                ("/result/batch/quality/complete", json!(!response.complete)),
            ] {
                let mut bad = response.clone();
                mutate_query_json(&mut bad, |envelope| {
                    *envelope.pointer_mut(path).unwrap() = value
                });
                assert_eq!(
                    typed_response_outcome(step, bad.encode_to_vec()),
                    StepOutcome::StoppedEnvelope,
                    "index={index} path={path}"
                );
            }
            let mut bad = response.clone();
            mutate_query_json(&mut bad, |envelope| {
                envelope["request"].as_object_mut().unwrap().remove("limit");
            });
            assert_eq!(
                typed_response_outcome(step, bad.encode_to_vec()),
                StepOutcome::StoppedEnvelope
            );
            let mut bad = response.clone();
            let text = String::from_utf8(bad.records[0].data.clone()).unwrap();
            bad.records[0].data = text
                .replacen('{', "{\"request_id\":\"TEST_CODE_DUPLICATE\",", 1)
                .into_bytes();
            assert_eq!(
                typed_response_outcome(step, bad.encode_to_vec()),
                StepOutcome::StoppedEnvelope
            );
        }
        if op == pb::Operation::MarketAnnouncements {
            // Original provider's safe relative PDF path may contain a query
            // or fragment; this recorded-only parser never fetches the URL.
            let mut allowed_pdf = response.clone();
            mutate_query_json(&mut allowed_pdf, |value| {
                let row = if version == 1 {
                    value
                } else {
                    &mut value["result"]["batch"]["records"][0]
                };
                row["pdf_url"] = json!("https://static.cninfo.com.cn/TEST_CODE.pdf?view=1#page=2");
            });
            assert_eq!(
                typed_response_outcome(step, allowed_pdf.encode_to_vec()),
                StepOutcome::ObservedSuccess
            );
            if limit == 1 {
                let mut impossible = response.clone();
                forge_impossible_limit1_two_page_prefix(&request, &mut impossible);
                assert_eq!(typed_response_outcome(step, impossible.encode_to_vec()), StepOutcome::StoppedEnvelope,
                    "even mutually matching two-page counters/SHA/quality cannot represent the original limit1 loop");
            }
            if limit == 300 {
                let mut impossible = response.clone();
                forge_exhausted31_with_impossible_extra_pages(&request, &mut impossible);
                let rows: Vec<ProbeAnnouncementRow> = if version == 1 {
                    impossible
                        .records
                        .iter()
                        .map(|record| serde_json::from_slice(&record.data).unwrap())
                        .collect()
                } else {
                    serde_json::from_value(
                        serde_json::from_slice::<serde_json::Value>(&impossible.records[0].data)
                            .unwrap()["result"]["batch"]["records"]
                            .clone(),
                    )
                    .unwrap()
                };
                let requested: ProbeAnnouncementRequest =
                    serde_json::from_slice(&request.payload.as_ref().unwrap().data).unwrap();
                assert_eq!(rows.len(), 31);
                assert!(impossible.complete);
                assert!(
                    probe_announcement_rows(&requested, &impossible, &rows),
                    "31 row/date/url/evidence shapes are otherwise valid"
                );
                assert_eq!(typed_response_outcome(step,impossible.encode_to_vec()),StepOutcome::StoppedEnvelope,
                    "source exhaustion after page2 forbids coherent extra page3..10 claims in both versions");
            }
            let mut actual_empty = response.clone();
            synthetic_zero_returned_announcement_observation(&request, &mut actual_empty, true);
            assert!(actual_empty.complete);
            assert_eq!(typed_response_outcome(step,actual_empty.encode_to_vec()),StepOutcome::ObservedSuccess,
                "native source-total-zero and its single empty page remain a legitimate recorded observation");
            if limit == 300 {
                let mut impossible = response.clone();
                synthetic_zero_returned_announcement_observation(&request, &mut impossible, false);
                assert!(!impossible.complete);
                assert_eq!(typed_response_outcome(step,impossible.encode_to_vec()),StepOutcome::StoppedEnvelope,
                    "31 raw records cannot all be duplicates without the first distinct identity in either schema");
            }
            let mut bad = response.clone();
            bad.complete = !response.complete;
            assert_eq!(
                typed_response_outcome(step, bad.encode_to_vec()),
                StepOutcome::StoppedEnvelope
            );
            if version == 2 {
                for (path, value) in [
                    ("/pit_guarantee", json!(true)),
                    ("/exchange_event_universe_complete", json!(true)),
                    ("/result/coverage/inspected_raw_rows", json!(1)),
                    (
                        "/result/coverage/source_exhausted",
                        json!(!response.complete),
                    ),
                    (
                        "/result/coverage/pages/0/has_more",
                        json!(response.complete),
                    ),
                    (
                        "/result/coverage/pages/0/request_body_sha256",
                        json!("0".repeat(64)),
                    ),
                    (
                        "/result/coverage/pages/0/response_body_sha256",
                        json!("0".repeat(64)),
                    ),
                    (
                        "/result/batch/records/0/published_at",
                        json!("2026-07-25T15:30:00+08:00"),
                    ),
                ] {
                    let mut bad = response.clone();
                    mutate_query_json(&mut bad, |envelope| {
                        *envelope.pointer_mut(path).unwrap() = value
                    });
                    assert_eq!(
                        typed_response_outcome(step, bad.encode_to_vec()),
                        StepOutcome::StoppedEnvelope,
                        "index={index} path={path}"
                    );
                }
            }
        }
    }
}
#[tokio::test]
async fn candidate_b7_probe_real_partial_success_has_health_after_but_bad_inner_hash_prefix_and_unknown_v2_stop(
) {
    let (receipt, observed, endpoint) = run_case(Case::Success).await;
    assert_eq!(receipt.rpc_count(), 36);
    for index in [0, 2, 4, 6] {
        let ordinal = 6 + index * 4;
        let Material::QueryResponse { wire } = &receipt.dto.steps.0[ordinal].material else {
            panic!("original query evidence absent");
        };
        let raw = RawRead::new(&receipt.arena, &receipt.dto.parts.0).unwrap();
        // Read this original referenced part directly without inventing live authority.
        let WireBody::Payload { part, .. } = &wire.body else {
            panic!("payload absent");
        };
        let response =
            pb::QueryResponse::decode(&receipt.arena[raw.ranges[*part].clone()]).unwrap();
        assert!(!response.complete);
        assert_eq!(
            receipt.dto.steps.0[ordinal].outcome,
            StepOutcome::ObservedSuccess
        );
        assert_eq!(observed.methods[ordinal + 1], "Health");
    }
    let bytes = receipt.canonical_bytes().unwrap();
    assert_eq!(
        read_receipt_on_target(&bytes, &digest(&bytes), &endpoint, TEST_AUTHORITY)
            .unwrap()
            .rpc_count(),
        36
    );
    let (empty_receipt, empty_observed, empty_endpoint) = run_case(Case::VerifiedSourceEmpty).await;
    assert_eq!(empty_receipt.rpc_count(), 36);
    assert_eq!(empty_receipt.outcome_name(), "CompletedRecordedObservation");
    assert_eq!(empty_observed.tcp, 1);
    let empty_bytes = empty_receipt.canonical_bytes().unwrap();
    assert_eq!(
        read_receipt_on_target(
            &empty_bytes,
            &digest(&empty_bytes),
            &empty_endpoint,
            TEST_AUTHORITY
        )
        .unwrap()
        .rpc_count(),
        36
    );
    let (historical_empty, historical_observed, historical_endpoint) =
        run_case(Case::VerifiedHistoricalEmpty).await;
    assert_eq!(historical_empty.rpc_count(), 36);
    assert_eq!(
        historical_empty.outcome_name(),
        "CompletedRecordedObservation"
    );
    assert_eq!(historical_observed.tcp, 1);
    let raw = RawRead::new(&historical_empty.arena, &historical_empty.dto.parts.0).unwrap();
    for ordinal in [6, 10] {
        let Material::QueryResponse { wire } = &historical_empty.dto.steps.0[ordinal].material
        else {
            panic!("empty historical wire absent");
        };
        let WireBody::Payload { part, .. } = &wire.body else {
            panic!("empty historical payload absent");
        };
        let response =
            pb::QueryResponse::decode(&historical_empty.arena[raw.ranges[*part].clone()]).unwrap();
        assert!(response.records.is_empty());
        assert!(response.complete);
        assert!(response.source_at.starts_with("unix-ms:"));
        assert_eq!(historical_observed.methods[ordinal + 1], "Health");
    }
    let historical_empty_bytes = historical_empty.canonical_bytes().unwrap();
    assert_eq!(
        read_receipt_on_target(
            &historical_empty_bytes,
            &digest(&historical_empty_bytes),
            &historical_endpoint,
            TEST_AUTHORITY
        )
        .unwrap()
        .rpc_count(),
        36
    );
    let (start_strict, start_observed, start_endpoint) =
        run_case(Case::HistoricalStartDayStrict).await;
    assert_eq!(start_strict.rpc_count(), 36);
    assert_eq!(start_observed.tcp, 1);
    let raw = RawRead::new(&start_strict.arena, &start_strict.dto.parts.0).unwrap();
    for ordinal in [6, 14] {
        let Material::QueryResponse { wire } = &start_strict.dto.steps.0[ordinal].material else {
            panic!("start-day wire absent");
        };
        let WireBody::Payload { part, .. } = &wire.body else {
            panic!("start-day payload absent");
        };
        let response =
            pb::QueryResponse::decode(&start_strict.arena[raw.ranges[*part].clone()]).unwrap();
        assert!(response.complete);
        let body: serde_json::Value = serde_json::from_slice(&response.records[0].data).unwrap();
        let date = if ordinal == 6 {
            &body["bar_start"]
        } else {
            assert_eq!(body["result"]["coverage"]["validated_source_rows"], 1);
            &body["result"]["batch"]["records"][0]["bar_start"]
        };
        assert_eq!(date, "2026-07-16");
        assert_eq!(start_observed.methods[ordinal + 1], "Health");
    }
    let start_bytes = start_strict.canonical_bytes().unwrap();
    assert_eq!(
        read_receipt_on_target(
            &start_bytes,
            &digest(&start_bytes),
            &start_endpoint,
            TEST_AUTHORITY
        )
        .unwrap()
        .rpc_count(),
        36
    );
    for (case, count) in [
        (Case::WrongInnerId, 15),
        (Case::WrongPayloadHash, 15),
        (Case::BadPrefix, 31),
        (Case::UnknownHistoricalV2, 15),
        (Case::ExhaustedExtraPages, 27),
        (Case::NonemptyWithZeroUnique, 27),
        (Case::HistoricalUnderLimitFalse, 11),
        (Case::HistoricalEmptyFalse, 7),
        (Case::HistoricalFullWindowFalse, 11),
        (Case::HistoricalSourceBeyondWindow, 19),
        (Case::HistoricalStartDayPartial, 7),
        (Case::HistoricalV2StartDayPartial, 15),
    ] {
        let (receipt, observed, endpoint) = run_case(case).await;
        assert_eq!(receipt.rpc_count(), count);
        assert_eq!(observed.tcp, 1);
        assert_ne!(observed.methods.last().unwrap(), "Health");
        assert_eq!(
            receipt.dto.steps.0.last().unwrap().outcome,
            StepOutcome::StoppedEnvelope
        );
        let bytes = receipt.canonical_bytes().unwrap();
        assert_eq!(
            read_receipt_on_target(&bytes, &digest(&bytes), &endpoint, TEST_AUTHORITY)
                .unwrap()
                .outcome_name(),
            "StoppedRecordedObservation"
        );
        if case == Case::UnknownHistoricalV2 {
            assert!(observed
                .responses
                .last()
                .unwrap()
                .ends_with(&[0xfa, 0x07, 4, b'T', b'E', b'S', b'T']));
        }
    }
}

// Original public NormalProvider shape, not a gRPC acceptance record.
const NORMAL_PROVIDER_LIMIT_1: &str = r####"{
  "load_probe": {
    "active_requests": 0,
    "maximum_concurrency": 1,
    "minimum_start_gap_seconds": null,
    "request_starts": 1
  },
  "request": {
    "end": "2026-07-30",
    "instrument": {
      "asset_class": "Equity",
      "code": "688561",
      "exchange": "Shanghai"
    },
    "interval": "Day",
    "limit": 1,
    "start": "2026-07-16"
  },
  "result": {
    "batch": {
      "provenance": {
        "batch_id": "8c8cd66d3ddb4b6bb0419e2315cd050c",
        "fetched_at": "1790902884.244366600",
        "source": "HithinkFinance",
        "source_at": "unix-ms:1785340800000"
      },
      "quality": {
        "complete": false,
        "issues": [
          "caller limit 1 retained 1 of 11 validated historical rows"
        ]
      },
      "records": [
        {
          "adjustment": "Unadjusted",
          "amount": 212389898.25,
          "bar_end": "2026-07-30",
          "bar_start": "2026-07-30",
          "batch_id": "8c8cd66d3ddb4b6bb0419e2315cd050c",
          "close": 24.78,
          "high": 25.88,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 24.73,
          "observed_at": "1790902884.244366600",
          "open": 25.0,
          "provider": "Tonghuashun",
          "source_at": "2026-07-30",
          "volume": 83915.9
        }
      ]
    },
    "coverage": {
      "authority_calendar_coverage": "Unknown",
      "caller_limit_truncated": true,
      "historical_publication_time": "NotProvided",
      "missing_date_reasons": "Unknown",
      "native_response": {
        "adjust": {
          "state": "Value",
          "value": "none"
        },
        "interval": "1d",
        "request_id": "8c8cd66d3ddb4b6bb0419e2315cd050c",
        "thscode": "688561.SH",
        "timestamp_ms": 1785340800000
      },
      "pit_guarantee": false,
      "response_receipt": {
        "body_byte_length": 1752,
        "body_sha256": "11c1526a210ddba925c369d6d66a91b73f2e05e0055a1e7766cc90f9ab698309",
        "final_url": "https://fuyao.aicubes.cn/api/a-share/prices/historical?thscode=688561.SH&interval=1d&start=1784131200000&end=1785427199999&adjust=none&offset=0"
      },
      "response_validated": true,
      "returned_rows": 1,
      "source_exhaustion": "Unknown",
      "source_revision": "NotProvided",
      "validated_source_rows": 11
    }
  },
  "scope": "NormalProviderObservationNotGrpcAcceptance"
}
"####;

// Original public NormalProvider shape, not a gRPC acceptance record.
const NORMAL_PROVIDER_LIMIT_15: &str = r####"{
  "load_probe": {
    "active_requests": 0,
    "maximum_concurrency": 1,
    "minimum_start_gap_seconds": null,
    "request_starts": 1
  },
  "request": {
    "end": "2026-07-30",
    "instrument": {
      "asset_class": "Equity",
      "code": "688561",
      "exchange": "Shanghai"
    },
    "interval": "Day",
    "limit": 15,
    "start": "2026-07-16"
  },
  "result": {
    "batch": {
      "provenance": {
        "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
        "fetched_at": "1790902926.657342800",
        "source": "HithinkFinance",
        "source_at": "unix-ms:1785340800000"
      },
      "quality": {
        "complete": true,
        "issues": []
      },
      "records": [
        {
          "adjustment": "Unadjusted",
          "amount": 256412900.24,
          "bar_end": "2026-07-16",
          "bar_start": "2026-07-16",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 25.64,
          "high": 26.19,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 25.01,
          "observed_at": "1790902926.657342800",
          "open": 25.43,
          "provider": "Tonghuashun",
          "source_at": "2026-07-16",
          "volume": 100362.13
        },
        {
          "adjustment": "Unadjusted",
          "amount": 315009019.02,
          "bar_end": "2026-07-17",
          "bar_start": "2026-07-17",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 25.41,
          "high": 26.1,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 24.63,
          "observed_at": "1790902926.657342800",
          "open": 25.88,
          "provider": "Tonghuashun",
          "source_at": "2026-07-17",
          "volume": 123948.92
        },
        {
          "adjustment": "Unadjusted",
          "amount": 276357221.51,
          "bar_end": "2026-07-20",
          "bar_start": "2026-07-20",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 25.23,
          "high": 25.9,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 24.48,
          "observed_at": "1790902926.657342800",
          "open": 25.41,
          "provider": "Tonghuashun",
          "source_at": "2026-07-20",
          "volume": 109733.99
        },
        {
          "adjustment": "Unadjusted",
          "amount": 263856220.3,
          "bar_end": "2026-07-21",
          "bar_start": "2026-07-21",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 25.65,
          "high": 25.87,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 24.68,
          "observed_at": "1790902926.657342800",
          "open": 24.98,
          "provider": "Tonghuashun",
          "source_at": "2026-07-21",
          "volume": 103833.31
        },
        {
          "adjustment": "Unadjusted",
          "amount": 428548191.55,
          "bar_end": "2026-07-22",
          "bar_start": "2026-07-22",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 26.79,
          "high": 27.14,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 24.98,
          "observed_at": "1790902926.657342800",
          "open": 25.55,
          "provider": "Tonghuashun",
          "source_at": "2026-07-22",
          "volume": 164303.17
        },
        {
          "adjustment": "Unadjusted",
          "amount": 286599186.39,
          "bar_end": "2026-07-23",
          "bar_start": "2026-07-23",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 25.94,
          "high": 26.6,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 25.75,
          "observed_at": "1790902926.657342800",
          "open": 26.2,
          "provider": "Tonghuashun",
          "source_at": "2026-07-23",
          "volume": 109864.86
        },
        {
          "adjustment": "Unadjusted",
          "amount": 264830358.51,
          "bar_end": "2026-07-24",
          "bar_start": "2026-07-24",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 24.0,
          "high": 25.53,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 24.0,
          "observed_at": "1790902926.657342800",
          "open": 25.53,
          "provider": "Tonghuashun",
          "source_at": "2026-07-24",
          "volume": 107351.34
        },
        {
          "adjustment": "Unadjusted",
          "amount": 159124281.3,
          "bar_end": "2026-07-27",
          "bar_start": "2026-07-27",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 24.62,
          "high": 24.76,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 23.61,
          "observed_at": "1790902926.657342800",
          "open": 23.97,
          "provider": "Tonghuashun",
          "source_at": "2026-07-27",
          "volume": 65183.41
        },
        {
          "adjustment": "Unadjusted",
          "amount": 220569919.59,
          "bar_end": "2026-07-28",
          "bar_start": "2026-07-28",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 24.72,
          "high": 25.28,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 24.28,
          "observed_at": "1790902926.657342800",
          "open": 24.4,
          "provider": "Tonghuashun",
          "source_at": "2026-07-28",
          "volume": 88637.73
        },
        {
          "adjustment": "Unadjusted",
          "amount": 226730954.72,
          "bar_end": "2026-07-29",
          "bar_start": "2026-07-29",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 25.2,
          "high": 25.68,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 24.52,
          "observed_at": "1790902926.657342800",
          "open": 24.9,
          "provider": "Tonghuashun",
          "source_at": "2026-07-29",
          "volume": 90154.22
        },
        {
          "adjustment": "Unadjusted",
          "amount": 212389898.25,
          "bar_end": "2026-07-30",
          "bar_start": "2026-07-30",
          "batch_id": "73c9f71ffd5347c490c3c401a59a2f5d",
          "close": 24.78,
          "high": 25.88,
          "instrument": {
            "asset_class": "Equity",
            "code": "688561",
            "exchange": "Shanghai"
          },
          "interval": "Day",
          "low": 24.73,
          "observed_at": "1790902926.657342800",
          "open": 25.0,
          "provider": "Tonghuashun",
          "source_at": "2026-07-30",
          "volume": 83915.9
        }
      ]
    },
    "coverage": {
      "authority_calendar_coverage": "Unknown",
      "caller_limit_truncated": false,
      "historical_publication_time": "NotProvided",
      "missing_date_reasons": "Unknown",
      "native_response": {
        "adjust": {
          "state": "Value",
          "value": "none"
        },
        "interval": "1d",
        "request_id": "73c9f71ffd5347c490c3c401a59a2f5d",
        "thscode": "688561.SH",
        "timestamp_ms": 1785340800000
      },
      "pit_guarantee": false,
      "response_receipt": {
        "body_byte_length": 1752,
        "body_sha256": "404217b93a1d0f8df8fa19dabcaac2e6f1d098e04824ba4086ccf5c6fdbcc537",
        "final_url": "https://fuyao.aicubes.cn/api/a-share/prices/historical?thscode=688561.SH&interval=1d&start=1784131200000&end=1785427199999&adjust=none&offset=0"
      },
      "response_validated": true,
      "returned_rows": 11,
      "source_exhaustion": "Unknown",
      "source_revision": "NotProvided",
      "validated_source_rows": 11
    }
  },
  "scope": "NormalProviderObservationNotGrpcAcceptance"
}
"####;
