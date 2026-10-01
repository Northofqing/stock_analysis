use super::*;
use crate::grpc_client::client::external_query_wire_fixture::ExternalQueryWireFixture;
use crate::grpc_client::external_pb::magic::market::v1::{CapabilitiesResponse, HealthResponse};
use crate::grpc_client::external_query_transport::ExternalWireMaterialV1;
use crate::market_domain::{AssetClass, Exchange};
use prost::Message as _;
use std::time::Duration;

fn money_instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity)
        .expect("TEST_CODE canonical money instrument")
}

fn observed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-14T15:31:01+08:00")
        .expect("TEST_CODE observation timestamp")
        .to_utc()
}

#[tokio::test]
async fn isolated_gateway_admits_both_flow_shapes_with_original_evidence() {
    let fixture = ExternalQueryWireFixture::bind_qualified_flows()
        .await
        .expect("TEST_CODE flow fixture");
    let mut gateway = ExternalFlowGateway::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE flow mTLS connection");
    fixture.release_capabilities();
    fixture.release();
    let money = tokio::time::timeout(
        Duration::from_secs(5),
        gateway.money_flow_once(&money_instrument(), observed_now()),
    )
    .await
    .expect("TEST_CODE money deadline")
    .expect("TEST_CODE money observation");
    let ExternalFlowAdmission::Admitted(money_batch) = &money.admission else {
        panic!("TEST_CODE MoneyFlows must be admitted");
    };
    assert_eq!(money_batch.records().len(), 1);
    assert_eq!(
        money_batch.records()[0].source_date.to_string(),
        "2026-09-14"
    );
    assert_eq!(money_batch.records()[0].main_net, 1.0);
    assert_eq!(money_batch.records()[0].small_net, 5.0);
    assert_eq!(
        money_batch.evidence().source_at.as_deref(),
        Some("2026-09-14")
    );
    assert!(money.observation.connection_identity.validate_recorded());
    assert!(money.observation.health.live && money.observation.health.ready);
    assert_eq!(
        money.observation.capabilities_response.capabilities.len(),
        2
    );
    let health_wire = HealthResponse::decode(money.observation.health_wire.as_slice())
        .expect("TEST_CODE full Health wire");
    assert_eq!(health_wire.request_id, money.observation.health.request_id);
    let capabilities_wire =
        CapabilitiesResponse::decode(money.observation.capabilities_wire.as_slice())
            .expect("TEST_CODE full Capabilities wire");
    assert_eq!(
        capabilities_wire.request_id,
        money.observation.capabilities_response.request_id
    );
    assert!(money
        .observation
        .server_build_identity
        .identity_error
        .is_empty());
    assert_eq!(money.observation.capability.provider, "Eastmoney");
    assert_eq!(money.observation.operation, ExternalOperation::MoneyFlows);
    assert!(money.observation.result.is_ok());
    assert!(money.observation.status.is_none());
    assert!(matches!(
        money.observation.wire.evidence,
        ExternalWireMaterialV1::Payload { .. }
    ));

    fixture.release_capabilities();
    fixture.release();
    let board = tokio::time::timeout(
        Duration::from_secs(5),
        gateway.board_flows_once(BoardKind::Industry, FlowInterval::Day1, 2, observed_now()),
    )
    .await
    .expect("TEST_CODE board deadline")
    .expect("TEST_CODE board observation");
    let ExternalFlowAdmission::Admitted(board_batch) = &board.admission else {
        panic!("TEST_CODE BoardFlows must be admitted");
    };
    assert_eq!(board_batch.records().len(), 1);
    assert_eq!(board_batch.records()[0].board_code, "BK0001");
    assert_eq!(board_batch.records()[0].interval, FlowInterval::Day1);
    assert_eq!(board_batch.records()[0].main_net, Some(1.0));
    assert_eq!(board_batch.records()[0].small_net, Some(5.0));
    assert_eq!(
        board_batch.evidence().source_at.as_deref(),
        Some("1789371000")
    );
    assert_eq!(board.observation.operation, ExternalOperation::BoardFlows);
    assert!(board.observation.result.is_ok());
    assert!(matches!(
        board.observation.wire.evidence,
        ExternalWireMaterialV1::Payload { .. }
    ));
    let snapshot = fixture.snapshot();
    assert_eq!(snapshot.methods, ["money_flows", "board_flows"]);
    assert_eq!(snapshot.calls, 2);
    assert!(snapshot.unexpected_methods.is_empty());
    drop(gateway);
    fixture
        .finish()
        .await
        .expect("TEST_CODE flow fixture cleanup");
}

#[tokio::test]
async fn isolated_gateway_keeps_unavailable_record_wire_without_admission() {
    let fixture = ExternalQueryWireFixture::bind_flow_record_unavailable()
        .await
        .expect("TEST_CODE unavailable record fixture");
    let mut gateway = ExternalFlowGateway::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE flow mTLS connection");
    fixture.release_capabilities();
    fixture.release();
    let read = gateway
        .money_flow_once(&money_instrument(), observed_now())
        .await
        .expect("TEST_CODE unavailable record observation");
    let ExternalFlowAdmission::EvidenceRejected(error) = &read.admission else {
        panic!("TEST_CODE unavailable record may not be admitted");
    };
    assert_eq!(error.reason_code(), "flow_record_unavailable");
    assert!(error.retryable());
    assert!(read.observation.result.is_ok());
    assert!(matches!(
        read.observation.wire.evidence,
        ExternalWireMaterialV1::Payload { .. }
    ));
    assert!(read.observation.connection_identity.validate_recorded());
    assert!(read.observation.health.live && read.observation.health.ready);
    assert_eq!(read.observation.capabilities_response.capabilities.len(), 2);
    assert_eq!(read.observation.capability.provider, "Eastmoney");
    assert_eq!(fixture.snapshot().calls, 1);
    drop(gateway);
    fixture
        .finish()
        .await
        .expect("TEST_CODE unavailable cleanup");
}

#[tokio::test]
async fn isolated_gateway_retains_transport_status_and_partial_response_evidence() {
    for status_reply in [true, false] {
        let fixture = if status_reply {
            ExternalQueryWireFixture::bind_flow_status().await
        } else {
            ExternalQueryWireFixture::bind_flow_incomplete().await
        }
        .expect("TEST_CODE rejected flow fixture");
        let mut gateway = ExternalFlowGateway::connect_client_bundle(fixture.bundle_path())
            .await
            .expect("TEST_CODE flow mTLS connection");
        fixture.release_capabilities();
        fixture.release();
        let read = gateway
            .board_flows_once(BoardKind::Industry, FlowInterval::Day1, 2, observed_now())
            .await
            .expect("TEST_CODE rejected flow observation");
        assert!(matches!(
            read.admission,
            ExternalFlowAdmission::QueryRejected
        ));
        let error = read
            .observation
            .result
            .as_ref()
            .expect_err("TEST_CODE query failure");
        assert_eq!(
            error.details().reason_code.as_deref(),
            Some(if status_reply {
                "unavailable"
            } else {
                "external_flow_response_incomplete"
            })
        );
        assert!(read.observation.connection_identity.validate_recorded());
        assert!(read.observation.health.live && read.observation.health.ready);
        assert_eq!(read.observation.capabilities_response.capabilities.len(), 2);
        assert_eq!(read.observation.capability.provider, "Eastmoney");
        if status_reply {
            let status = read
                .observation
                .status
                .as_ref()
                .expect("TEST_CODE raw status");
            assert_eq!(status.code, tonic::Code::Unavailable as i32);
            assert!(!status.details.is_empty());
            assert_eq!(
                status.raw_status.message(),
                "TEST_CODE flow provider unavailable"
            );
            assert_eq!(status.raw_status.details(), status.details.as_slice());
            assert!(matches!(
                read.observation.wire.evidence,
                ExternalWireMaterialV1::Missing { .. }
            ));
        } else {
            assert!(read.observation.status.is_none());
            assert!(matches!(
                read.observation.wire.evidence,
                ExternalWireMaterialV1::Payload { .. }
            ));
        }
        let snapshot = fixture.snapshot();
        assert_eq!(snapshot.methods, ["board_flows"]);
        assert_eq!(snapshot.calls, 1);
        assert!(snapshot.unexpected_methods.is_empty());
        drop(gateway);
        fixture.finish().await.expect("TEST_CODE rejected cleanup");
    }
}
