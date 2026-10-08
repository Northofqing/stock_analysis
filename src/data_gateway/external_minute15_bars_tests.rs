use super::*;
use crate::grpc_client::envelope::CanonicalRecord;

fn instrument() -> InstrumentId {
    InstrumentId::new(Exchange::Shenzhen, "300274", AssetClass::Equity).unwrap()
}
fn batch() -> QueryResult {
    let observed = chrono::DateTime::parse_from_rfc3339("2026-08-10T15:01:00+08:00")
        .unwrap()
        .timestamp()
        .to_string();
    let mut records = Vec::new();
    for (hour, minute) in [
        (9, 45),
        (10, 0),
        (10, 15),
        (10, 30),
        (10, 45),
        (11, 0),
        (11, 15),
        (11, 30),
        (13, 15),
        (13, 30),
        (13, 45),
        (14, 0),
        (14, 15),
        (14, 30),
        (14, 45),
        (15, 0),
    ] {
        let boundary = format!("2026-08-10 {hour:02}:{minute:02}:00");
        let data = serde_json::to_vec(&serde_json::json!({
            "instrument":instrument(),"interval":"Minute15","bar_start":boundary,"bar_end":boundary,
            "open":10.0,"high":11.0,"low":9.0,"close":10.5,"volume":123.0,"amount":4567.0,
            "adjustment":"Unadjusted","source_at":&boundary[..16],"observed_at":observed,"provider":"Tdx","batch_id":"TEST_CODE_TDX_BATCH"
        })).unwrap();
        records.push(CanonicalRecord {
            schema: "magic.market.bar".into(),
            schema_version: 1,
            content_type: "application/json; charset=utf-8".into(),
            data,
        });
    }
    QueryResult {
        admission: QueryAdmission::Admitted,
        selected_provider: "Tdx".into(),
        batch_id: "TEST_CODE_TDX_BATCH".into(),
        complete: true,
        observed_at: observed,
        source_at: "2026-08-10 15:00".into(),
        records,
        provenance: AcquisitionProvenance::ExternalMtlsAuthority("TEST_CODE_AUTHORITY".into()),
        diagnostic_blocker: String::new(),
    }
}
fn mutate(q: &mut QueryResult, key: &str, value: serde_json::Value) {
    let mut raw: serde_json::Value = serde_json::from_slice(&q.records[0].data).unwrap();
    raw[key] = value;
    q.records[0].data = serde_json::to_vec(&raw).unwrap();
}

#[test]
fn r12_minute15_closed_request_keeps_tail_null_selectors_and_day_contract() {
    use crate::grpc_client::external_pb::magic::market::v1::QueryRequest;
    let query = build_external_minute15_tail_request(&instrument(), 48).unwrap();
    let request = QueryRequest::decode(query.wire_bytes().as_slice()).unwrap();
    assert_eq!(request.preferred_provider, "Tdx");
    assert!(!request.allow_unadmitted);
    let payload = request.payload.unwrap();
    assert_eq!(payload.content_type, "application/json; charset=utf-8");
    let raw: serde_json::Value = serde_json::from_slice(&payload.data).unwrap();
    assert_eq!(raw["interval"], "Minute15");
    assert!(raw["start"].is_null() && raw["end"].is_null());
    assert_eq!(raw["limit"], 48);
    for limit in [0, 801] {
        assert!(build_external_minute15_tail_request(&instrument(), limit).is_err());
    }
    let d = chrono::NaiveDate::from_ymd_opt(2026, 8, 10).unwrap();
    let day = crate::grpc_client::external_v1::build_external_historical_bars_query_request(
        &instrument(),
        d,
        d,
        1,
    )
    .unwrap();
    let day = QueryRequest::decode(day.wire_bytes().as_slice()).unwrap();
    assert_eq!(day.preferred_provider, "HithinkFinance");
    let value: serde_json::Value = serde_json::from_slice(&day.payload.unwrap().data).unwrap();
    assert_eq!(value["interval"], "Day");
    assert_eq!(value["start"], "2026-08-10");
}
#[test]
fn r12_minute15_native_endpoints_fill_real_date_and_preserve_quantities() {
    let q = batch();
    let bars = project_records(&instrument(), 16, &q).unwrap();
    assert_eq!(
        (
            bars[0].year,
            bars[0].month,
            bars[0].day,
            bars[0].hour,
            bars[0].minute
        ),
        (2026, 8, 10, 9, 45)
    );
    assert_eq!(bars[0].datetime, "2026-08-10 09:45:00");
    assert_eq!(bars[0].vol, 123.0);
    assert_eq!(bars[0].amount, 4567.0);
    let exact = chrono::NaiveDate::from_ymd_opt(2026, 8, 10)
        .unwrap()
        .and_hms_opt(1, 45, 0)
        .unwrap();
    assert_eq!(
        crate::review::backtest::locate_signal_bar(&bars, exact).unwrap(),
        Some(0)
    );
    assert_eq!(
        crate::review::backtest::locate_signal_bar(&bars, exact + chrono::Duration::minutes(1))
            .unwrap(),
        None
    );
}
#[test]
fn r12_minute15_wrong_native_identity_evidence_and_source_are_rejected() {
    for (key, value) in [
        (
            "instrument",
            serde_json::json!({"exchange":"Shenzhen","code":"000001","asset_class":"Equity"}),
        ),
        ("interval", serde_json::json!("Day")),
        ("adjustment", serde_json::json!("ForwardAdjusted")),
        ("provider", serde_json::json!("Sina")),
        ("batch_id", serde_json::json!("TEST_CODE_OTHER")),
        ("observed_at", serde_json::json!("1799999999")),
        ("source_at", serde_json::json!("2026-08-10 10:00")),
        ("bar_start", serde_json::json!("2026-08-10 09:30:00")),
        ("volume", serde_json::json!(-1.0)),
        ("high", serde_json::json!(1.0)),
    ] {
        let mut q = batch();
        mutate(&mut q, key, value);
        assert!(project_records(&instrument(), 16, &q).is_err(), "{key}");
    }
    for which in [
        "incomplete",
        "diagnostic",
        "unadmitted",
        "authority",
        "schema",
        "empty",
        "count",
        "source",
    ] {
        let mut q = batch();
        match which {
            "incomplete" => q.complete = false,
            "diagnostic" => q.diagnostic_blocker = "TEST_CODE_DIAGNOSTIC".into(),
            "unadmitted" => q.admission = QueryAdmission::Unadmitted,
            "authority" => {
                q.provenance = AcquisitionProvenance::LocalWireSource("TEST_CODE_LOCAL".into())
            }
            "schema" => q.records[0].schema_version = 2,
            "empty" => q.records.clear(),
            "count" => q.records.push(q.records[0].clone()),
            "source" => q.source_at = "2026-08-11 15:00".into(),
            _ => unreachable!(),
        }
        assert!(project_records(&instrument(), 16, &q).is_err(), "{which}");
    }
}
#[test]
fn r12_minute15_grid_gap_duplicate_reverse_lunch_and_future_reject() {
    for which in ["gap", "duplicate", "reverse", "lunch", "future"] {
        let mut q = batch();
        match which {
            "gap" => {
                q.records.remove(4);
            }
            "duplicate" => q.records[2] = q.records[1].clone(),
            "reverse" => q.records.swap(2, 3),
            "lunch" => {
                mutate(
                    &mut q,
                    "bar_start",
                    serde_json::json!("2026-08-10 12:00:00"),
                );
                mutate(&mut q, "bar_end", serde_json::json!("2026-08-10 12:00:00"));
                mutate(&mut q, "source_at", serde_json::json!("2026-08-10 12:00"));
            }
            "future" => {
                q.observed_at = "1".into();
                for r in &mut q.records {
                    let mut v: serde_json::Value = serde_json::from_slice(&r.data).unwrap();
                    v["observed_at"] = serde_json::json!("1");
                    r.data = serde_json::to_vec(&v).unwrap();
                }
            }
            _ => unreachable!(),
        }
        assert!(project_records(&instrument(), 16, &q).is_err(), "{which}");
    }
}
#[test]
fn r12_minute15_source_error_retains_native_unknown_reason_and_retry_flag() {
    let error = GrpcError::Internal {
        details: Box::new(crate::grpc_client::errors::ErrorDetail {
            reason_code: Some("TEST_CODE_NATIVE_REASON".into()),
            retryable: Some(false),
            ..Default::default()
        }),
    };
    let failure = Minute15Error::source(error.clone());
    assert_eq!(failure.reason_code(), "TEST_CODE_NATIVE_REASON");
    assert!(!failure.retryable());
    assert_eq!(failure.source_error, Some(error));
}

#[test]
fn r12_minute15_client_clock_rejects_future_observation_without_aging_history() {
    let q = batch();
    let now = chrono::DateTime::from_timestamp(q.observed_at.parse::<i64>().unwrap(), 0).unwrap();
    assert!(project_records_at(&instrument(), 16, &q, now).is_ok());
    assert!(
        project_records_at(&instrument(), 16, &q, now + chrono::Duration::days(90)).is_ok(),
        "old historical bars are not live quotes"
    );
    assert_eq!(
        project_records_at(&instrument(), 16, &q, now - chrono::Duration::seconds(3))
            .unwrap_err()
            .reason_code(),
        "minute15_observed_at_future"
    );
}
#[test]
fn r12_minute15_private_store_has_distinct_recorded_domain_and_hash_binding() {
    use super::super::historical_observed_store::{HistoricalObservedStore, Publication};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = std::fs::canonicalize(dir.path()).unwrap();
    let store = HistoricalObservedStore::open_existing(&path, &[]).unwrap();
    let issued = build_external_minute15_tail_request(&instrument(), 48)
        .unwrap()
        .wire_bytes();
    let retained = WindowTransportEvidence {
        stage: "TEST_CODE_CAPABILITY_FAILURE".into(),
        ..Default::default()
    };
    let raw = capture("300274", 48, &issued, &retained, None, None).unwrap();
    let (artifact, pubstate) = store.persist_minute15(&raw).unwrap();
    assert_eq!(pubstate, Publication::Published);
    let read = store.read_checked(&artifact).unwrap();
    assert_eq!(read.raw_part("capture_domain").unwrap(), CAPTURE_DOMAIN);
    assert_eq!(read.raw_part("issued_request_wire").unwrap(), issued);
    assert!(read.raw_part("minute15_tail_request").is_some());
    assert!(read.raw_part("request_calendar").is_none());
    assert_eq!(
        store.persist_minute15(&raw).unwrap().1,
        Publication::ExistingExact
    );
    assert_eq!(
        sha(&std::fs::read(path.join(artifact.filename())).unwrap()),
        artifact.file_sha256()
    );
    assert!(HistoricalObservedStore::open_existing(&path, &[path.clone()]).is_err());
}

#[tokio::test]
#[ignore = "explicit single readonly Windows841 canary; private candidate evidence only"]
async fn r12_minute15_live_readonly_receiver_canary() {
    let bundle =
        PathBuf::from(std::env::var_os("R12_TEST_CODE_LIVE_BUNDLE").expect("explicit bundle"));
    let output = PathBuf::from(
        std::env::var_os("R12_TEST_CODE_LIVE_EVIDENCE")
            .expect("explicit isolated private directory"),
    );
    let batch = receive_at(&bundle, &output, &instrument(), 48)
        .await
        .expect("real production receiver gates");
    assert_eq!(batch.receipt.requested_limit, 48);
    assert!(batch.bars.len() <= 48);
    println!(
        "R12_MINUTE15_RECEIPT={}",
        serde_json::to_string(batch.receipt()).unwrap()
    );
}
