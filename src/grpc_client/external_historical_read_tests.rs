use super::*;
use crate::grpc_client::client::external_query_wire_fixture::{
    ExternalQueryWireFixture, HistoricalCapabilityBehavior, HistoricalQueryReply,
};
use crate::grpc_client::external_pb::magic::market::v1::{QueryRequest, QueryResponse};
use crate::grpc_client::external_query_transport::ExternalWireMaterialV1;
use crate::grpc_client::external_v1::build_external_historical_bars_query_request;
use crate::market_domain::{AssetClass, Exchange, InstrumentId};
use chrono::NaiveDate;

fn test_query() -> ExternalHistoricalBarsQuery {
    build_external_historical_bars_query_request(
        &InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
        NaiveDate::from_ymd_opt(2026, 9, 11).unwrap(),
        NaiveDate::from_ymd_opt(2026, 9, 15).unwrap(),
        3,
    )
    .expect("TEST_CODE delivered HistoricalBars query")
}

#[tokio::test]
async fn wg06_exact_window_reader_uses_historical_external_route_and_fresh_controls() {
    let fixture = ExternalQueryWireFixture::bind_historical(
        HistoricalQueryReply::Success,
        HistoricalCapabilityBehavior::Ready,
    )
    .await
    .expect("TEST_CODE HistoricalBars fixture");
    let mut reader = ExternalHistoricalReadClient::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE HistoricalBars mTLS connection");
    for _ in 0..2 {
        fixture.release_capabilities();
        fixture.release();
        let observation = reader
            .query_once(test_query())
            .await
            .expect("TEST_CODE observation");
        assert!(observation.connection_identity.validate_recorded());
        assert_eq!(observation.wire.method, ExternalQueryMethod::HistoricalBars);
        observation
            .wire
            .validate_descriptor(
                ExternalQueryMethod::HistoricalBars,
                &observation.connection_identity.descriptor_sha256,
            )
            .expect("TEST_CODE raw wire bound to HistoricalBars descriptor");
        let request = QueryRequest::decode(observation.request_bytes.as_slice()).unwrap();
        let request_id = &request.context.as_ref().unwrap().request_id;
        let response = QueryResponse::decode(observation.wire.payload().unwrap()).unwrap();
        assert_eq!(response.request_id, *request_id);
        assert_eq!(response.operation, ExternalOperation::HistoricalBars as i32);
        assert!(observation.request_id_correlation.starts_with("sha256:"));
        assert!(!observation.request_id_correlation.contains(request_id));
        assert!(!observation.health_wire.is_empty());
        assert!(!observation.capabilities_wire.is_empty());
        assert!(observation.server_build_identity.identity_error.is_empty());
        let result = observation
            .result
            .expect("TEST_CODE observed admitted envelope");
        assert_eq!(result.selected_provider, "HithinkFinance");
        assert_eq!(result.source(), "grpc-mtls:macro.test.invalid");
        assert_eq!(result.records[0].data, b"TEST_CODE_RAW_HISTORICAL_RECORD");
        assert!(observation.status.is_none());
    }
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.health_calls, 2);
    assert_eq!(snapshot.capabilities_calls, 2);
    assert_eq!(snapshot.calls, 2);
    assert_eq!(snapshot.tcp_accepts, 1);
    assert_eq!(snapshot.methods, ["historical_bars", "historical_bars"]);
    assert_eq!(snapshot.authorized, [true, true]);
    assert!(snapshot.unexpected_methods.is_empty());
    drop(reader);
    fixture.finish().await.expect("TEST_CODE fixture cleanup");
}

#[tokio::test]
async fn wg06_exact_window_capability_conflicts_reject_before_data_rpc() {
    for (behavior, expected) in [
        (
            HistoricalCapabilityBehavior::Missing,
            "external_historical_capability_missing",
        ),
        (
            HistoricalCapabilityBehavior::Duplicate,
            "external_historical_capability_ambiguous",
        ),
        (
            HistoricalCapabilityBehavior::Unadmitted,
            "external_historical_capability_unadmitted",
        ),
        (
            HistoricalCapabilityBehavior::Unavailable,
            "external_historical_runtime_unavailable",
        ),
        (
            HistoricalCapabilityBehavior::Blocked,
            "external_historical_capability_conflict",
        ),
    ] {
        let fixture =
            ExternalQueryWireFixture::bind_historical(HistoricalQueryReply::Success, behavior)
                .await
                .expect("TEST_CODE HistoricalBars capability fixture");
        let mut reader = ExternalHistoricalReadClient::connect_client_bundle(fixture.bundle_path())
            .await
            .expect("TEST_CODE capability connection");
        fixture.release_capabilities();
        let error = reader
            .query_once(test_query())
            .await
            .expect_err("TEST_CODE capability rejected");
        assert_eq!(error.details().reason_code.as_deref(), Some(expected));
        assert_eq!(fixture.snapshot().calls, 0);
        drop(reader);
        fixture
            .finish()
            .await
            .expect("TEST_CODE capability fixture cleanup");
    }
}

#[tokio::test]
async fn wg06_exact_window_envelope_conflicts_retain_raw_response_without_retry() {
    for (reply, expected_reason) in [
        (
            HistoricalQueryReply::Incomplete,
            Some("external_historical_response_incomplete"),
        ),
        (
            HistoricalQueryReply::Unadmitted,
            Some("external_historical_response_unadmitted"),
        ),
        (
            HistoricalQueryReply::ProviderMismatch,
            Some("external_historical_provider_mismatch"),
        ),
        (HistoricalQueryReply::OperationMismatch, None),
        (HistoricalQueryReply::RequestIdMismatch, None),
    ] {
        let fixture =
            ExternalQueryWireFixture::bind_historical(reply, HistoricalCapabilityBehavior::Ready)
                .await
                .expect("TEST_CODE HistoricalBars conflict fixture");
        let mut reader = ExternalHistoricalReadClient::connect_client_bundle(fixture.bundle_path())
            .await
            .expect("TEST_CODE conflict connection");
        fixture.release_capabilities();
        fixture.release();
        let observation = reader
            .query_once(test_query())
            .await
            .expect("TEST_CODE rejected observation retained");
        assert!(matches!(
            observation.wire.evidence,
            ExternalWireMaterialV1::Payload { .. }
        ));
        assert!(observation.status.is_none());
        let error = observation
            .result
            .expect_err("TEST_CODE envelope conflict rejected");
        if let Some(reason) = expected_reason {
            assert_eq!(error.details().reason_code.as_deref(), Some(reason));
        } else {
            assert!(matches!(error, GrpcError::Unknown { .. }));
        }
        assert_eq!(fixture.snapshot().calls, 1);
        assert_eq!(fixture.snapshot().methods, ["historical_bars"]);
        drop(reader);
        fixture
            .finish()
            .await
            .expect("TEST_CODE conflict fixture cleanup");
    }
}

#[tokio::test]
async fn wg06_exact_window_remote_status_retains_details_and_error_detail_trailer() {
    let fixture = ExternalQueryWireFixture::bind_historical(
        HistoricalQueryReply::StatusWithTrailer,
        HistoricalCapabilityBehavior::Ready,
    )
    .await
    .expect("TEST_CODE HistoricalBars status fixture");
    let mut reader = ExternalHistoricalReadClient::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE status connection");
    fixture.release_capabilities();
    fixture.release();
    let observation = reader
        .query_once(test_query())
        .await
        .expect("TEST_CODE status observation");
    let status = observation.status.expect("TEST_CODE raw status retained");
    assert_eq!(status.code, tonic::Code::Unavailable as i32);
    assert!(!status.details.is_empty());
    assert_eq!(
        status.error_detail_trailer,
        ExternalHistoricalTrailerMaterial::Bytes(status.details.clone())
    );
    assert!(matches!(
        observation.wire.evidence,
        ExternalWireMaterialV1::Missing { .. }
    ));
    let error = observation.result.expect_err("TEST_CODE classified status");
    assert!(matches!(error, GrpcError::Unavailable { .. }));
    assert_eq!(error.details().provider.as_deref(), Some("HithinkFinance"));
    assert_eq!(error.details().reason_code.as_deref(), Some("unavailable"));
    assert_eq!(fixture.snapshot().calls, 1);
    drop(reader);
    fixture.finish().await.expect("TEST_CODE status cleanup");
}
