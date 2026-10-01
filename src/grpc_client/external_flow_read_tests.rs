use super::*;
use crate::grpc_client::client::external_query_wire_fixture::ExternalQueryWireFixture;
use crate::grpc_client::external_pb::magic::market::v1::{QueryRequest, QueryResponse};
use crate::grpc_client::external_query_transport::ExternalWireMaterialV1;
use std::time::Duration;

fn flow_params(operation: ExternalOperation) -> serde_json::Value {
    match operation {
        ExternalOperation::MoneyFlows => serde_json::json!({
            "instruments": [{"exchange":"Shanghai","code":"600519","asset_class":"Equity"}]
        }),
        ExternalOperation::BoardFlows => serde_json::json!({
            "category":"Industry","interval":"Day1","limit":2
        }),
        _ => unreachable!("TEST_CODE flow-only operation"),
    }
}

#[tokio::test]
async fn flow_reader_uses_two_external_methods_and_preserves_one_shot_evidence() {
    let fixture = ExternalQueryWireFixture::bind_qualified_flows()
        .await
        .expect("TEST_CODE flow fixture");
    let mut reader = ExternalFlowReadClient::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE flow mTLS connection");
    for (operation, expected_method, expected_schema) in [
        (
            ExternalOperation::MoneyFlows,
            "money_flows",
            "magic.market.money_flow",
        ),
        (
            ExternalOperation::BoardFlows,
            "board_flows",
            "magic.market.board_flow",
        ),
    ] {
        fixture.release_capabilities();
        fixture.release();
        let observation = tokio::time::timeout(
            Duration::from_secs(5),
            reader.query_once(operation, flow_params(operation)),
        )
        .await
        .expect("TEST_CODE flow query deadline")
        .expect("TEST_CODE flow qualification");
        assert_eq!(observation.operation, operation);
        assert!(observation.server_build_identity.identity_error.is_empty());
        assert!(observation.connection_identity.validate_recorded());
        assert_eq!(observation.capability.operation, operation as i32);
        assert_eq!(observation.capability.provider, "Eastmoney");
        let request = QueryRequest::decode(observation.request_bytes.as_slice())
            .expect("TEST_CODE External request wire");
        assert!(!request.allow_unadmitted);
        let request_id = &request.context.as_ref().unwrap().request_id;
        assert!(observation.request_id_correlation.starts_with("sha256:"));
        assert!(!observation.request_id_correlation.contains(request_id));
        observation
            .wire
            .validate_descriptor(
                ExternalQueryMethod::from_external_operation(operation).unwrap(),
                &observation.connection_identity.descriptor_sha256,
            )
            .expect("TEST_CODE response wire bound to descriptor and method");
        let ExternalWireMaterialV1::Payload {
            protobuf_payload, ..
        } = &observation.wire.evidence
        else {
            panic!("TEST_CODE complete response body expected");
        };
        let response = QueryResponse::decode(protobuf_payload.as_slice())
            .expect("TEST_CODE External response wire");
        assert_eq!(response.request_id, *request_id);
        assert_eq!(response.operation, operation as i32);
        let result = observation
            .result
            .expect("TEST_CODE admitted flow envelope");
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].schema, expected_schema);
        assert_eq!(result.source(), "grpc-mtls:macro.test.invalid");
        assert!(observation.status.is_none());
        assert_eq!(
            fixture.snapshot().methods.last().map(String::as_str),
            Some(expected_method)
        );
    }
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.health_calls, 2);
    assert_eq!(snapshot.capabilities_calls, 2);
    assert_eq!(snapshot.calls, 2);
    assert_eq!(snapshot.authorized, [true, true]);
    assert!(snapshot.unexpected_methods.is_empty());
    drop(reader);
    fixture
        .finish()
        .await
        .expect("TEST_CODE flow fixture cleanup");
}

#[tokio::test]
async fn flow_reader_rejects_absent_capability_before_data_rpc() {
    let fixture = ExternalQueryWireFixture::bind_generated_routes()
        .await
        .expect("TEST_CODE unadmitted flow fixture");
    let mut reader = ExternalFlowReadClient::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE unadmitted flow mTLS connection");
    fixture.release_capabilities();
    let error = reader
        .query_once(
            ExternalOperation::MoneyFlows,
            flow_params(ExternalOperation::MoneyFlows),
        )
        .await
        .err()
        .expect("TEST_CODE missing flow capability rejected");
    assert_eq!(
        error.details().reason_code.as_deref(),
        Some("external_flow_capability_missing")
    );
    assert_eq!(fixture.snapshot().calls, 0);
    drop(reader);
    fixture
        .finish()
        .await
        .expect("TEST_CODE flow gate fixture cleanup");
}

#[tokio::test]
async fn flow_reader_preserves_status_and_classifies_without_retry() {
    let fixture = ExternalQueryWireFixture::bind_flow_status()
        .await
        .expect("TEST_CODE flow status fixture");
    let mut reader = ExternalFlowReadClient::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE flow status mTLS connection");
    fixture.release_capabilities();
    fixture.release();
    let observation = reader
        .query_once(
            ExternalOperation::BoardFlows,
            flow_params(ExternalOperation::BoardFlows),
        )
        .await
        .expect("TEST_CODE status observation");
    let status = observation.status.expect("TEST_CODE raw status retained");
    assert_eq!(status.code, tonic::Code::Unavailable as i32);
    assert!(!status.details.is_empty());
    assert_eq!(
        status.error_detail_trailer,
        super::super::unary_attempt::UnaryTrailerMaterial::Absent
    );
    assert!(matches!(
        observation.wire.evidence,
        ExternalWireMaterialV1::Missing { .. }
    ));
    let error = observation
        .result
        .expect_err("TEST_CODE classified remote failure");
    assert!(matches!(error, GrpcError::Unavailable { .. }));
    assert_eq!(error.details().provider.as_deref(), Some("Eastmoney"));
    assert_eq!(error.details().reason_code.as_deref(), Some("unavailable"));
    assert_eq!(fixture.snapshot().calls, 1);
    drop(reader);
    fixture
        .finish()
        .await
        .expect("TEST_CODE flow status fixture cleanup");
}

#[tokio::test]
async fn flow_reader_rejects_incomplete_response_without_retry_or_local_fallback() {
    let fixture = ExternalQueryWireFixture::bind_flow_incomplete()
        .await
        .expect("TEST_CODE incomplete flow fixture");
    let mut reader = ExternalFlowReadClient::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE incomplete flow mTLS connection");
    fixture.release_capabilities();
    fixture.release();
    let observation = reader
        .query_once(
            ExternalOperation::MoneyFlows,
            flow_params(ExternalOperation::MoneyFlows),
        )
        .await
        .expect("TEST_CODE incomplete flow observation");
    assert!(matches!(
        observation.wire.evidence,
        ExternalWireMaterialV1::Payload { .. }
    ));
    assert!(observation.status.is_none());
    let error = observation
        .result
        .expect_err("TEST_CODE incomplete flow rejected");
    assert_eq!(
        error.details().reason_code.as_deref(),
        Some("external_flow_response_incomplete")
    );
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.calls, 1);
    assert_eq!(snapshot.methods, ["money_flows"]);
    assert!(snapshot.unexpected_methods.is_empty());
    drop(reader);
    fixture
        .finish()
        .await
        .expect("TEST_CODE incomplete flow cleanup");
}

#[test]
fn flow_capability_rejects_duplicate_and_contradictory_eastmoney_rows() {
    let ready = Capability {
        operation: ExternalOperation::MoneyFlows as i32,
        repository_admission: AdmissionState::Admitted as i32,
        runtime_available: true,
        provider: "Eastmoney".to_owned(),
        exact_scope: "TEST_CODE one scope".to_owned(),
        blocker: String::new(),
        diagnostic_available: false,
    };
    let mut denied = ready.clone();
    denied.repository_admission = AdmissionState::Unadmitted as i32;
    denied.runtime_available = false;
    denied.blocker = "TEST_CODE denied".to_owned();
    let error = require_flow_capability(&[ready.clone(), denied], ExternalOperation::MoneyFlows)
        .expect_err("TEST_CODE conflicting duplicate rejected");
    assert_eq!(
        error.details().reason_code.as_deref(),
        Some("external_flow_capability_ambiguous")
    );
    let mut blocked = ready;
    blocked.blocker = "TEST_CODE contradictory ready".to_owned();
    let error = require_flow_capability(&[blocked], ExternalOperation::MoneyFlows)
        .expect_err("TEST_CODE blocked ready rejected");
    assert_eq!(
        error.details().reason_code.as_deref(),
        Some("external_flow_capability_conflict")
    );
}
