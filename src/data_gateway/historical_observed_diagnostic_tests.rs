use super::*;

#[test]
fn wg06_observed_diagnostic_exact_request_rejects_normalized_or_unfinished_inputs() {
    let request = diagnostic_request("Shanghai", "600519", "2026-09-11", "2026-09-15").unwrap();
    assert_eq!(request.required_trading_dates().len(), 3);
    for (exchange, code, from, to) in [
        ("Beijing", "920001", "2026-09-11", "2026-09-15"),
        ("Shanghai", " 600519", "2026-09-11", "2026-09-15"),
        ("Shanghai", "600519 ", "2026-09-11", "2026-09-15"),
        ("Shanghai", "600519", "2026-9-11", "2026-09-15"),
        ("Shanghai", "600519", "2026-09-12", "2026-09-13"),
        ("Shanghai", "600519", "2099-01-01", "2099-01-02"),
    ] {
        assert!(diagnostic_request(exchange, code, from, to).is_err());
    }
}

#[test]
fn wg06_observed_diagnostic_never_echoes_remote_error_detail_text() {
    let mut details = crate::grpc_client::errors::ErrorDetail::default();
    details.code = "TEST_CODE_PRIVATE_CREDENTIAL_TEXT".to_owned();
    details.reason_code = Some("TEST_CODE_PRIVATE_REASON_TEXT".to_owned());
    let error = crate::grpc_client::errors::GrpcError::Unknown {
        details: Box::new(details),
    };
    assert_eq!(local_error_kind(&error), "Unknown");
    let output = serde_json::json!({"kind":local_error_kind(&error)}).to_string();
    assert!(!output.contains("TEST_CODE_PRIVATE"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn wg06_observed_diagnostic_persists_actual_gateway_negative_status_without_admission() {
    use crate::grpc_client::client::external_query_wire_fixture::{
        ExternalQueryWireFixture, HistoricalCapabilityBehavior, HistoricalQueryReply,
    };
    let fixture = ExternalQueryWireFixture::bind_historical(
        HistoricalQueryReply::StatusWithTrailer,
        HistoricalCapabilityBehavior::Ready,
    )
    .await
    .unwrap();
    let output = tempfile::tempdir().unwrap();
    let output_root = std::fs::canonicalize(output.path()).unwrap();
    fixture.release_capabilities();
    fixture.release();
    let summary: String = observed_history_diagnostic(
        fixture.bundle_path(),
        &output_root,
        "Shanghai",
        "600519",
        "2026-09-11",
        "2026-09-15",
    )
    .await
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(value["scope"], "ObservedOnly");
    assert_eq!(value["authority"], "NotAdmitted");
    assert_eq!(value["coverage"], "Unknown");
    assert_eq!(value["pit"], "NotCertified");
    assert_eq!(value["diagnostic_result"], "StoredNegativeObservation");
    assert_eq!(value["query_outcome"]["state"], "Rejected");
    assert_eq!(value["query_outcome"]["kind"], "Unavailable");
    assert_eq!(value["projection"]["state"], "Rejected");
    assert_eq!(value["request"]["wire_limit"], 3);
    assert_eq!(
        value["request"]["required_trading_dates"],
        serde_json::json!(["2026-09-11", "2026-09-14", "2026-09-15"])
    );
    assert!(!summary.contains("TEST_CODE historical provider unavailable"));
    assert!(!summary.contains("raw_status_and_trailer"));
    assert_eq!(std::fs::read_dir(&output_root).unwrap().count(), 1);
    let path = output_root.join(value["artifact"]["filename"].as_str().unwrap());
    let bytes = std::fs::read(path).unwrap();
    use sha2::Digest as _;
    assert_eq!(
        hex::encode(sha2::Sha256::digest(&bytes)),
        value["artifact"]["file_sha256"].as_str().unwrap()
    );
    assert_eq!(fixture.snapshot().calls, 1);
    fixture.finish().await.unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn wg06_observed_diagnostic_pre_call_failure_has_no_artifact_and_forbidden_outputs_never_dial(
) {
    use crate::grpc_client::client::external_query_wire_fixture::{
        ExternalQueryWireFixture, HistoricalCapabilityBehavior, HistoricalQueryReply,
    };
    let fixture = ExternalQueryWireFixture::bind_historical(
        HistoricalQueryReply::Success,
        HistoricalCapabilityBehavior::Missing,
    )
    .await
    .unwrap();
    let output = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(output.path()).unwrap();
    fixture.release_capabilities();
    fixture.release();
    assert!(observed_history_diagnostic(
        fixture.bundle_path(),
        &root,
        "Shanghai",
        "600519",
        "2026-09-11",
        "2026-09-15"
    )
    .await
    .is_err());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    assert_eq!(fixture.snapshot().calls, 0);
    fixture.finish().await.unwrap();

    let bundle = tempfile::tempdir().unwrap();
    let bundle_root = std::fs::canonicalize(bundle.path()).unwrap();
    let alias = root.join("TEST_CODE_ALIAS");
    std::os::unix::fs::symlink(&bundle_root, &alias).unwrap();
    for output in [&bundle_root, &alias] {
        let error = observed_history_diagnostic(
            &bundle_root,
            output,
            "Shanghai",
            "600519",
            "2026-09-11",
            "2026-09-15",
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("isolated anchored"));
    }
    assert!(observed_history_diagnostic(
        &bundle_root,
        crate::production_root::production_root(),
        "Shanghai",
        "600519",
        "2026-09-11",
        "2026-09-15"
    )
    .await
    .unwrap_err()
    .to_string()
    .contains("isolated anchored"));
    assert_eq!(std::fs::read_dir(&bundle_root).unwrap().count(), 0);
}
