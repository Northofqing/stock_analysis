use super::*;
use crate::grpc_client::client::external_query_wire_fixture::{
    ExternalQueryWireFixture, HistoricalCapabilityBehavior, HistoricalQueryReply,
};
use crate::grpc_client::external_pb::magic::market::v1::QueryRequest;
use crate::grpc_client::external_query_transport::ExternalWireMaterialV1;

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
}

fn invoked_at(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap()
}

fn test_window() -> HistoricalWindowRequest {
    HistoricalWindowRequest::new(
        instrument(),
        date(11),
        date(15),
        invoked_at("2026-09-16T15:31:00+08:00"),
    )
    .expect("TEST_CODE verified closed trading-date range")
}

#[test]
fn wg06_exact_window_binds_delivered_dates_and_calendar_count_to_request_wire() {
    let request = test_window();
    assert_eq!(
        request.required_trading_dates(),
        [date(11), date(14), date(15)]
    );
    assert_eq!(request.calendar_authority_hash().len(), 64);
    let query = request.query().unwrap().into_request();
    assert_eq!(query.preferred_provider, "HithinkFinance");
    assert!(!query.allow_unadmitted);
    let payload = query.payload.unwrap();
    assert_eq!(payload.schema, "magic.market.historical_bars.request");
    assert_eq!(payload.schema_version, 1);
    assert_eq!(payload.content_type, "application/json; charset=utf-8");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap(),
        serde_json::json!({
            "instrument":{"exchange":"Shanghai","code":"600519","asset_class":"Equity"},
            "interval":"Day","start":"2026-09-11","end":"2026-09-15","limit":3
        })
    );
}

#[test]
fn wg06_exact_window_rejects_future_unclosed_empty_invalid_and_unavailable_calendar() {
    for (from, to, now, expected) in [
        (
            date(11),
            date(15),
            "2026-09-15T14:59:59+08:00",
            "external_historical_window_incomplete",
        ),
        (
            date(11),
            date(16),
            "2026-09-15T15:31:00+08:00",
            "external_historical_window_incomplete",
        ),
        (
            date(12),
            date(13),
            "2026-09-15T15:31:00+08:00",
            "trading_calendar_empty",
        ),
        (
            date(15),
            date(11),
            "2026-09-15T15:31:00+08:00",
            "invalid_trading_calendar_range",
        ),
        (
            NaiveDate::from_ymd_opt(2024, 9, 11).unwrap(),
            NaiveDate::from_ymd_opt(2024, 9, 15).unwrap(),
            "2026-09-15T15:31:00+08:00",
            "trading_calendar_unavailable",
        ),
    ] {
        let error =
            HistoricalWindowRequest::new(instrument(), from, to, invoked_at(now)).unwrap_err();
        assert_eq!(error.details().reason_code.as_deref(), Some(expected));
    }
    let at_close = HistoricalWindowRequest::new(
        instrument(),
        date(11),
        date(15),
        invoked_at("2026-09-15T15:00:00+08:00"),
    );
    assert!(at_close.is_ok());
    // An already completed Friday remains requestable on a Saturday morning.
    assert!(HistoricalWindowRequest::new(
        instrument(),
        date(11),
        date(12),
        invoked_at("2026-09-12T09:00:00+08:00"),
    )
    .is_ok());
    for instrument in [
        InstrumentId::new(Exchange::Beijing, "920001", AssetClass::Equity).unwrap(),
        InstrumentId::new(Exchange::Shanghai, "000001", AssetClass::Index).unwrap(),
        InstrumentId::new(Exchange::Shanghai, "TEST_CODE", AssetClass::Equity).unwrap(),
    ] {
        assert_eq!(
            HistoricalWindowRequest::new(
                instrument,
                date(11),
                date(15),
                invoked_at("2026-09-16T15:31:00+08:00")
            )
            .unwrap_err()
            .details()
            .reason_code
            .as_deref(),
            Some("external_historical_instrument_unsupported")
        );
    }
}

#[tokio::test]
async fn wg06_exact_window_gateway_keeps_opaque_observation_and_sealed_capture_identity() {
    let fixture = ExternalQueryWireFixture::bind_historical(
        HistoricalQueryReply::Success,
        HistoricalCapabilityBehavior::Ready,
    )
    .await
    .expect("TEST_CODE Gateway historical fixture");
    let mut gateway = ExternalHistoricalBarsGateway::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE Gateway historical connection");
    fixture.release_capabilities();
    fixture.release();
    let capture = gateway
        .observe_once(test_window())
        .await
        .expect("TEST_CODE sealed observation");
    assert_eq!(capture.capture_hash().len(), 64);
    assert_eq!(
        capture.request().required_trading_dates(),
        [date(11), date(14), date(15)]
    );
    let wire = QueryRequest::decode(capture.observation().request_bytes.as_slice()).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&wire.payload.unwrap().data).unwrap();
    assert_eq!(body["limit"], 3);
    let result = capture
        .observation()
        .result
        .as_ref()
        .expect("TEST_CODE envelope observed");
    // The opaque transport fixture proves this boundary does not guess rows or
    // accidentally construct a coverage, OHLCV, or verified replay capability.
    assert_eq!(
        result.records[0].schema,
        "TEST_CODE_OPAQUE_HISTORICAL_RECORD"
    );
    assert_eq!(result.records[0].data, b"TEST_CODE_RAW_HISTORICAL_RECORD");
    assert_eq!(fixture.snapshot().calls, 1);

    let GatewayObservedHistoricalWindowCapture {
        request,
        mut observation,
        capture_hash,
    } = capture;
    assert_eq!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    let different_window = HistoricalWindowRequest::new(
        instrument(),
        date(10),
        date(15),
        invoked_at("2026-09-16T15:31:00+08:00"),
    )
    .unwrap();
    assert_ne!(
        observed_capture_hash(&different_window, &observation).unwrap(),
        capture_hash
    );
    let mut different_instrument = request.clone();
    different_instrument.instrument =
        InstrumentId::new(Exchange::Shenzhen, "300005", AssetClass::Equity).unwrap();
    assert_ne!(
        observed_capture_hash(&different_instrument, &observation).unwrap(),
        capture_hash
    );
    observation.request_bytes.push(0);
    assert_ne!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    observation.request_bytes.pop();
    observation.health_wire.push(0);
    assert_ne!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    observation.health_wire.pop();
    observation.capabilities_wire.push(0);
    assert_ne!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    observation.capabilities_wire.pop();
    let original_epoch = observation.connection_identity.epoch.clone();
    observation
        .connection_identity
        .epoch
        .push_str("TEST_CODE_CHANGED");
    assert_ne!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    observation.connection_identity.epoch = original_epoch;
    if let ExternalWireMaterialV1::Payload {
        protobuf_payload, ..
    } = &mut observation.wire.evidence
    {
        protobuf_payload.push(0);
    } else {
        panic!("TEST_CODE query raw body expected");
    }
    assert_ne!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    if let ExternalWireMaterialV1::Payload {
        protobuf_payload, ..
    } = &mut observation.wire.evidence
    {
        protobuf_payload.pop();
    }
    assert_eq!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    observation
        .result
        .as_mut()
        .unwrap()
        .selected_provider
        .push_str("TEST_CODE_CHANGED");
    assert_ne!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    drop(gateway);
    fixture
        .finish()
        .await
        .expect("TEST_CODE Gateway fixture cleanup");
}

#[tokio::test]
async fn wg06_exact_window_gateway_seals_failed_status_and_trailer_bytes() {
    let fixture = ExternalQueryWireFixture::bind_historical(
        HistoricalQueryReply::StatusWithTrailer,
        HistoricalCapabilityBehavior::Ready,
    )
    .await
    .expect("TEST_CODE historical failed capture fixture");
    let mut gateway = ExternalHistoricalBarsGateway::connect_client_bundle(fixture.bundle_path())
        .await
        .expect("TEST_CODE failed capture connection");
    fixture.release_capabilities();
    fixture.release();
    let capture = gateway
        .observe_once(test_window())
        .await
        .expect("TEST_CODE failed observation retained");
    assert!(capture.observation().result.is_err());
    let GatewayObservedHistoricalWindowCapture {
        request,
        mut observation,
        capture_hash,
    } = capture;
    observation.status.as_mut().unwrap().details.push(0);
    assert_ne!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    observation.status.as_mut().unwrap().details.pop();
    if let crate::grpc_client::client::external_historical_read::ExternalHistoricalTrailerMaterial::Bytes(bytes) =
        &mut observation.status.as_mut().unwrap().error_detail_trailer
    {
        bytes.push(0);
    } else {
        panic!("TEST_CODE retained error-detail trailer expected");
    }
    assert_ne!(
        observed_capture_hash(&request, &observation).unwrap(),
        capture_hash
    );
    drop(gateway);
    fixture
        .finish()
        .await
        .expect("TEST_CODE failed capture cleanup");
}
