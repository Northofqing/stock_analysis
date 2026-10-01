use super::*;
use crate::data_gateway::historical_observed_store::{
    HistoricalObservedStore, StoredObservedEvidence,
};
use crate::data_gateway::historical_record_projection::project_observed_historical_records;
use crate::grpc_client::client::external_historical_read::ExternalHistoricalTrailerMaterial;
use crate::grpc_client::client::external_query_wire_fixture::{
    ExternalQueryWireFixture, HistoricalCapabilityBehavior, HistoricalQueryReply,
};

fn test_request() -> HistoricalWindowRequest {
    HistoricalWindowRequest::new(
        InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
        NaiveDate::from_ymd_opt(2026, 9, 11).unwrap(),
        NaiveDate::from_ymd_opt(2026, 9, 15).unwrap(),
        DateTime::parse_from_rfc3339("2026-09-16T15:31:00+08:00")
            .unwrap()
            .with_timezone(&Utc),
    )
    .unwrap()
}

#[tokio::test]
async fn wg06_observed_store_capture_v1_parts_keep_original_hash_and_failed_raw_status_trailer() {
    for reply in [
        HistoricalQueryReply::Success,
        HistoricalQueryReply::StatusWithTrailer,
        HistoricalQueryReply::StatusMalformedTrailer("TEST_CODE_MALFORMED_TRAILER_ONE"),
        HistoricalQueryReply::Incomplete,
    ] {
        let fixture =
            ExternalQueryWireFixture::bind_historical(reply, HistoricalCapabilityBehavior::Ready)
                .await
                .unwrap();
        let mut gateway =
            ExternalHistoricalBarsGateway::connect_client_bundle(fixture.bundle_path())
                .await
                .unwrap();
        fixture.release_capabilities();
        fixture.release();
        let capture = gateway.observe_once(test_request()).await.unwrap();
        if reply == HistoricalQueryReply::Success {
            // The existing live TEST_CODE envelope is opaque, not a fabricated
            // successful historical row projection from archived receipts.
            assert!(capture.observation().result.is_ok());
        } else {
            assert!(capture.observation().result.is_err());
        }
        assert_eq!(
            capture.capture_hash(),
            original_v1_hash(
                &capture.request,
                &capture.issued_request_bytes,
                &capture.observation
            )
            .unwrap()
        );
        let parts = capture.hash_parts_v1().unwrap();
        assert_eq!(parts.len(), 16);
        assert_eq!(parts[3], capture.observation().health_wire);
        assert_eq!(parts[6], capture.observation().capabilities_wire);
        assert_eq!(parts[10], capture.issued_request_bytes);
        assert_eq!(parts[11], capture.observation().request_bytes);
        assert_eq!(hash_capture_parts_v1(&parts), capture.capture_hash());
        let output = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(output.path()).unwrap();
        let store = HistoricalObservedStore::open_existing(&root, &[]).unwrap();
        let (artifact, _) = store.persist(&capture).unwrap();
        let recorded: StoredObservedEvidence = store.read_checked(&artifact).unwrap();
        for (name, index) in [
            ("query_wire_evidence", 12),
            ("raw_status_and_trailer", 13),
            ("typed_query_outcome", 14),
            ("request_binding_outcome", 15),
        ] {
            assert_eq!(recorded.raw_part(name).unwrap(), parts[index]);
        }
        if let Some(status) = capture.observation().status.as_ref() {
            let raw: serde_json::Value =
                serde_json::from_slice(recorded.raw_part("raw_status_and_trailer").unwrap())
                    .unwrap();
            assert_eq!(raw["details"], serde_json::json!(status.details));
            assert_eq!(
                raw["error_detail_trailer"],
                serde_json::to_value(&status.error_detail_trailer).unwrap()
            );
            assert_eq!(
                raw["error_detail_trailer_encoded"]["encoded_bytes"],
                serde_json::json!(status
                    .raw_status
                    .metadata()
                    .get_bin("magic-error-detail-bin")
                    .unwrap()
                    .as_encoded_bytes())
            );
        }
        assert!(project_observed_historical_records(&capture).is_err());
        assert_eq!(fixture.snapshot().calls, 1);
        drop(gateway);
        fixture.finish().await.unwrap();
    }
}

#[tokio::test]
async fn wg06_observed_store_negative_binding_and_non_utf8_status_preserve_bytes_without_admission()
{
    let fixture = ExternalQueryWireFixture::bind_historical(
        HistoricalQueryReply::StatusWithTrailer,
        HistoricalCapabilityBehavior::Ready,
    )
    .await
    .unwrap();
    let mut reader = ExternalHistoricalReadClient::connect_client_bundle(fixture.bundle_path())
        .await
        .unwrap();
    let request = test_request();
    let query = request.query().unwrap();
    let mut issued = query.wire_bytes();
    fixture.release_capabilities();
    fixture.release();
    let mut observation = reader.query_once(query).await.unwrap();
    // Explicit TEST_CODE alteration of an already negative live observation:
    // proves that storage preserves bytes instead of normalizing/truncating.
    let details = vec![0, 0xff, 0x80, 0x1f];
    let trailer = vec![0xfe, 0, 0x81];
    let status = observation.status.as_mut().unwrap();
    status.raw_status = tonic::Status::with_details(
        tonic::Code::Unavailable,
        "TEST_CODE_RAW_STATUS",
        details.clone().into(),
    );
    status.raw_status.metadata_mut().insert_bin(
        "magic-error-detail-bin",
        tonic::metadata::MetadataValue::from_bytes(&trailer),
    );
    status.details = details.clone();
    status.error_detail_trailer = ExternalHistoricalTrailerMaterial::Bytes(trailer.clone());
    issued.push(0);
    let capture =
        GatewayObservedHistoricalWindowCapture::from_observed(request, issued, observation)
            .unwrap();
    assert!(capture.request_binding_error().is_some());
    assert!(project_observed_historical_records(&capture).is_err());
    let output = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(output.path()).unwrap();
    let store = HistoricalObservedStore::open_existing(&root, &[]).unwrap();
    let (artifact, _) = store.persist(&capture).unwrap();
    let recorded: StoredObservedEvidence = store.read_checked(&artifact).unwrap();
    let raw: serde_json::Value =
        serde_json::from_slice(recorded.raw_part("raw_status_and_trailer").unwrap()).unwrap();
    assert_eq!(raw["details"], serde_json::json!(details));
    assert_eq!(
        raw["error_detail_trailer"],
        serde_json::json!({"Bytes":trailer})
    );
    assert_eq!(
        recorded.raw_part("observed_request_wire").unwrap(),
        capture.observation().request_bytes
    );
    assert_ne!(
        recorded.raw_part("issued_request_wire").unwrap(),
        recorded.raw_part("observed_request_wire").unwrap()
    );
    let binding: serde_json::Value =
        serde_json::from_slice(recorded.raw_part("request_binding_outcome").unwrap()).unwrap();
    assert_eq!(binding["outcome"], "QueryRejected");
    assert_eq!(
        artifact.capture_sha256(),
        original_v1_hash(
            &capture.request,
            &capture.issued_request_bytes,
            &capture.observation
        )
        .unwrap()
    );
    drop(reader);
    fixture.finish().await.unwrap();
}

// Frozen pre-refactor v1 algorithm; independent length-prefix reducer.
fn original_v1_hash(
    request: &HistoricalWindowRequest,
    issued_request_bytes: &[u8],
    observation: &ExternalHistoricalObservation,
) -> Result<String, GrpcError> {
    let serialize = |value: &serde_json::Value| {
        serde_json::to_vec(value)
            .map_err(|_| request_error("external_historical_capture_serialization", false))
    };
    let request_material = serialize(&serde_json::json!({
        "instrument":request.instrument(),
        "from":request.from(),
        "to":request.to(),
        "required_trading_dates":request.required_trading_dates(),
        "calendar_authority_sha256":request.calendar_authority_hash(),
        "invoked_at":request.invoked_at,
        "wire_limit":request.required_trading_dates().len()
    }))?;
    let connection_material = serde_json::to_vec(&observation.connection_identity)
        .map_err(|_| request_error("external_historical_capture_serialization", false))?;
    let wire_material = serde_json::to_vec(&observation.wire)
        .map_err(|_| request_error("external_historical_capture_serialization", false))?;
    let status_material = match &observation.status {
        None => serialize(&serde_json::json!({"status":"Absent"}))?,
        Some(status) => serialize(&serde_json::json!({
            "status":"Observed",
            "code":status.code,
            "message":status.raw_status.message(),
            "details":status.details,
            "error_detail_trailer":status.error_detail_trailer,
            "error_detail_trailer_encoded":match status.raw_status.metadata().get_bin("magic-error-detail-bin") {
                None => serde_json::json!({"presence":"Absent"}),
                Some(header) => serde_json::json!({
                    "presence":"Present",
                    "encoded_bytes":header.as_encoded_bytes()
                })
            }
        }))?,
    };
    let mut hasher = Sha256::new();
    for bytes in [
        CAPTURE_MATERIAL.as_bytes(),
        &request_material,
        &connection_material,
        &observation.health_wire,
        &observation.health.encode_to_vec(),
        &observation.server_build_identity.encode_to_vec(),
        &observation.capabilities_wire,
        &observation.capabilities_response.encode_to_vec(),
        &observation.capability.encode_to_vec(),
        observation.request_id_correlation.as_bytes(),
        issued_request_bytes,
        &observation.request_bytes,
        &wire_material,
        &status_material,
        &serialize(&observed_result_material(&observation.result))?,
        &serialize(
            &match request_binding_error(issued_request_bytes, observation) {
                None => serde_json::json!({"request_binding":"Matched"}),
                Some(error) => observed_result_material(&Err(error)),
            },
        )?,
    ] {
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    Ok(hex::encode(hasher.finalize()))
}
