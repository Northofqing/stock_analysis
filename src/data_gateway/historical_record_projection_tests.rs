use super::*;
use crate::calendar::resolve_verified_replay_range;
use crate::grpc_client::client::external_query_wire_fixture::{
    ExternalQueryWireFixture, HistoricalCapabilityBehavior, HistoricalQueryReply,
};
use crate::grpc_client::external_pb::magic::market::v1::{Operation, QueryResponse};
use prost::Message as _;
use serde_json::{json, Value};

struct RecordedFixture {
    saved: Value,
    response: QueryResult,
    instrument: InstrumentId,
    from: NaiveDate,
    to: NaiveDate,
    trading_dates: Vec<NaiveDate>,
    original_limit: usize,
}

impl RecordedFixture {
    fn bounds(&self) -> ProjectionBounds<'_> {
        ProjectionBounds {
            instrument: &self.instrument,
            from: self.from,
            to: self.to,
            required_trading_dates: &self.trading_dates,
            wire_limit: self.original_limit,
        }
    }

    fn parse(&self) -> Result<ParsedHistoricalRecords, HistoricalProjectionError> {
        parse_record_set(&self.bounds(), &self.response)
    }
}

fn recorded(case: &str) -> RecordedFixture {
    let (source, expected_receipt) = match case {
        "688277" => (
            include_str!("fixtures/historical_records_20261002/bars-688277.json"),
            "ea8da734a8967ff70d942f2ca96e9252f2fb0a189b72c06cc91beb8aaa3a35d2",
        ),
        "688561" => (
            include_str!("fixtures/historical_records_20261002/bars-688561.json"),
            "60a262b44f3fc3b9f80ed981fa3d8d0706a154a037cebee5ca9d03b4fd0e4670",
        ),
        "688561-limit1" => (
            include_str!("fixtures/historical_records_20261002/bars-688561-limit1.json"),
            "86d19e8debfbb938c85a6517aa914e781593da7093e7408108bba5daf964850c",
        ),
        _ => panic!("unknown TEST_CODE offline receipt fixture"),
    };
    let saved: Value = serde_json::from_str(source).unwrap();
    assert_eq!(saved["receipt_sha256"], expected_receipt);
    assert_eq!(
        saved["fixture_kind"],
        "TEST_CODE_PUBLIC_RECORDED_HISTORICAL_ROWS"
    );
    assert_eq!(
        saved["production_build_identity"]["source_revision"],
        "67c832e43f36f188e4d769f409691c0b1d9a2ea2"
    );
    let wire = &saved["query_response_protobuf_reencoded"];
    let bytes = hex::decode(wire["bytes_hex"].as_str().unwrap()).unwrap();
    assert_eq!(bytes.len() as u64, wire["byte_count"].as_u64().unwrap());
    assert_eq!(
        hex::encode(Sha256::digest(&bytes)),
        wire["sha256"].as_str().unwrap()
    );
    let response = crate::grpc_client::envelope::parse_external_native_query_response(
        saved["request_id"].as_str().unwrap(),
        Operation::HistoricalBars,
        "TEST_CODE_OFFLINE_RECEIPT_PARSER_NOT_LIVE_AUTHORITY",
        QueryResponse::decode(bytes.as_slice()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        response.records.len(),
        saved["records"].as_array().unwrap().len()
    );
    for (record, original) in response
        .records
        .iter()
        .zip(saved["records"].as_array().unwrap())
    {
        assert_eq!(record.schema, original["schema"].as_str().unwrap());
        assert_eq!(
            record.schema_version as u64,
            original["schema_version"].as_u64().unwrap()
        );
        assert_eq!(
            record.content_type,
            original["content_type"].as_str().unwrap()
        );
        assert_eq!(
            record.data,
            original["data_utf8"].as_str().unwrap().as_bytes()
        );
        assert_eq!(
            hex::encode(Sha256::digest(&record.data)),
            original["data_sha256"].as_str().unwrap()
        );
    }
    let request = &saved["request_json"];
    let instrument: InstrumentId = serde_json::from_value(request["instrument"].clone()).unwrap();
    let from = canonical_date(request["start"].as_str().unwrap()).unwrap();
    let to = canonical_date(request["end"].as_str().unwrap()).unwrap();
    let trading_dates = resolve_verified_replay_range(from, to)
        .unwrap()
        .required_trading_dates()
        .to_vec();
    let original_limit = request["limit"].as_u64().unwrap() as usize;
    RecordedFixture {
        saved,
        response,
        instrument,
        from,
        to,
        trading_dates,
        original_limit,
    }
}

fn mutate_record(fixture: &mut RecordedFixture, index: usize, change: impl FnOnce(&mut Value)) {
    let mut value = serde_json::from_slice(&fixture.response.records[index].data).unwrap();
    change(&mut value);
    fixture.response.records[index].data = serde_json::to_vec(&value).unwrap();
}

#[test]
fn wg06_historical_projection_real_receipts_keep_exact_bytes_units_and_provider_distinction() {
    for (case, limit, row_count, missing_count) in [
        ("688277", 15, 1, 10),
        ("688561", 15, 11, 0),
        ("688561-limit1", 1, 1, 10),
    ] {
        let fixture = recorded(case);
        assert_eq!(fixture.original_limit, limit);
        assert_eq!(fixture.trading_dates.len(), 11);
        let parsed = fixture.parse().unwrap();
        assert_eq!(parsed.rows.len(), row_count);
        assert_eq!(parsed.missing_observed_dates.len(), missing_count);
        assert!(parsed.observed_complete_flag);
        for (row, record) in parsed.rows.iter().zip(&fixture.response.records) {
            assert_eq!(row.instrument(), &fixture.instrument);
            assert_eq!(row.raw_record(), record);
            assert_eq!(
                row.raw_data_sha256(),
                hex::encode(Sha256::digest(&record.data))
            );
            assert_eq!(row.record_provider(), ProviderId::Tonghuashun);
            assert_eq!(row.selected_provider(), ProviderId::HithinkFinance);
            assert_ne!(row.record_provider(), row.selected_provider());
            assert_eq!(
                row.response_observed_at(),
                observed_instant(&fixture.response.observed_at).unwrap()
            );
        }
        if case == "688277" {
            let row = &parsed.rows[0];
            assert_eq!(
                row.source_date(),
                NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()
            );
            assert_eq!(
                (
                    row.open().get(),
                    row.high().get(),
                    row.low().get(),
                    row.close().get()
                ),
                (22.5, 22.5, 17.2, 17.4)
            );
            assert_eq!(row.volume_lots().get(), 302553.5);
            assert_eq!(row.amount_cny().get(), 603447516.21);
            assert!(row.response_observed_at().date_naive() > row.source_date());
        }
    }
}

#[test]
fn wg06_historical_projection_complete_true_limit1_preserves_unknown_missing_dates() {
    let full = recorded("688561");
    let limited = recorded("688561-limit1");
    let parsed_full = full.parse().unwrap();
    let parsed_limited = limited.parse().unwrap();
    assert!(parsed_full.missing_observed_dates.is_empty());
    assert!(parsed_full.observed_complete_flag && parsed_limited.observed_complete_flag);
    assert_eq!(
        parsed_limited.missing_observed_dates,
        full.trading_dates[..10]
    );
    assert_eq!(parsed_limited.rows[0].source_date(), full.trading_dates[10]);
    // Two acquisitions have different batch/observation identities, even for
    // the same latest prices. A comparison must not merge their provenance.
    assert_ne!(full.response.batch_id, limited.response.batch_id);
    assert_ne!(full.response.observed_at, limited.response.observed_at);
    let mut mixed = recorded("688561");
    mixed.response.records[10] = limited.response.records[0].clone();
    assert_eq!(
        mixed.parse().unwrap_err(),
        HistoricalProjectionError::InvalidRecord {
            index: 10,
            reason: "record adjustment or flattened evidence mismatch",
        }
    );
}

#[test]
fn wg06_historical_projection_rejects_wrong_schema_identity_dates_and_flattened_evidence() {
    let mutations: Vec<Box<dyn Fn(&mut Value)>> = vec![
        Box::new(|row| row["instrument"]["code"] = json!("688277")),
        Box::new(|row| row["instrument"]["code"] = json!(" 688561")),
        Box::new(|row| row["instrument"]["exchange"] = json!("Shenzhen")),
        Box::new(|row| row["instrument"]["asset_class"] = json!("Index")),
        Box::new(|row| row["instrument"]["native_identity"] = json!("invented")),
        Box::new(|row| row["interval"] = json!("Week")),
        Box::new(|row| row["bar_start"] = json!("2026-7-16")),
        Box::new(|row| row["bar_end"] = json!("2026-07-17")),
        Box::new(|row| row["source_at"] = json!("2026-07-16T00:00:00+08:00")),
        Box::new(|row| {
            for key in ["bar_start", "bar_end", "source_at"] {
                row[key] = json!("2026-07-15");
            }
        }),
        Box::new(|row| {
            for key in ["bar_start", "bar_end", "source_at"] {
                row[key] = json!("2026-07-18");
            }
        }),
        Box::new(|row| row["adjustment"] = json!("Forward")),
        Box::new(|row| row["provider"] = json!("HithinkFinance")),
        Box::new(|row| row["batch_id"] = json!("TEST_CODE_OTHER_ACQUISITION")),
        Box::new(|row| row["observed_at"] = json!("1790875706.639175101")),
        Box::new(|row| row["evidence"] = json!({"provider":"Tonghuashun"})),
    ];
    for (case, change) in mutations.into_iter().enumerate() {
        let mut fixture = recorded("688561");
        mutate_record(&mut fixture, 0, change);
        assert!(
            matches!(
                fixture.parse(),
                Err(HistoricalProjectionError::InvalidRecord { index: 0, .. })
            ),
            "mutation={case}"
        );
    }
    for header in 0..3 {
        let mut fixture = recorded("688561");
        match header {
            0 => fixture.response.records[0].schema = "TEST_CODE_UNKNOWN_BAR".into(),
            1 => fixture.response.records[0].schema_version = 2,
            _ => fixture.response.records[0].content_type = "application/octet-stream".into(),
        }
        assert_eq!(
            fixture.parse().unwrap_err(),
            HistoricalProjectionError::InvalidRecord {
                index: 0,
                reason: "unsupported record schema"
            }
        );
    }
}

#[test]
fn wg06_historical_projection_rejects_bad_numeric_values_and_any_bad_tail_row() {
    for (key, value) in [
        ("open", json!(0)),
        ("open", json!(-1)),
        ("high", json!(25.0)),
        ("low", json!(25.7)),
        ("volume", json!(-1)),
        ("volume", json!("NaN")),
        ("amount", json!(-1)),
        ("amount", Value::Null),
        ("close", json!("Infinity")),
    ] {
        let mut fixture = recorded("688561");
        mutate_record(&mut fixture, 0, |row| row[key] = value);
        assert!(
            matches!(
                fixture.parse(),
                Err(HistoricalProjectionError::InvalidRecord { index: 0, .. })
            ),
            "field={key}"
        );
    }
    let mut absent = recorded("688561");
    mutate_record(&mut absent, 10, |row| {
        row.as_object_mut().unwrap().remove("amount");
    });
    assert_eq!(
        absent.parse().unwrap_err(),
        HistoricalProjectionError::InvalidRecord {
            index: 10,
            reason: "invalid strict daily record JSON"
        }
    );
    let mut overflow = recorded("688561");
    let original = std::str::from_utf8(&overflow.response.records[0].data).unwrap();
    let changed = original.replace("\"volume\":100362.13", "\"volume\":1e309");
    assert_ne!(changed, original);
    overflow.response.records[0].data = changed.into_bytes();
    assert!(matches!(
        overflow.parse(),
        Err(HistoricalProjectionError::InvalidRecord { index: 0, .. })
    ));
}

#[test]
fn wg06_historical_projection_rejects_duplicate_json_fields_dates_reverse_order_and_limit_overrun()
{
    let mut duplicate_field = recorded("688561");
    let original = std::str::from_utf8(&duplicate_field.response.records[0].data).unwrap();
    duplicate_field.response.records[0].data = original
        .replacen("{", "{\"provider\":\"Tonghuashun\",", 1)
        .into_bytes();
    assert_eq!(
        duplicate_field.parse().unwrap_err(),
        HistoricalProjectionError::InvalidRecord {
            index: 0,
            reason: "invalid strict daily record JSON"
        }
    );
    let mut duplicate_identity = recorded("688561");
    let original = std::str::from_utf8(&duplicate_identity.response.records[0].data).unwrap();
    duplicate_identity.response.records[0].data = original
        .replacen(
            "\"instrument\":{",
            "\"instrument\":{\"code\":\"688561\",",
            1,
        )
        .into_bytes();
    assert!(matches!(
        duplicate_identity.parse(),
        Err(HistoricalProjectionError::InvalidRecord { index: 0, .. })
    ));
    let mut duplicate_date = recorded("688561");
    duplicate_date
        .response
        .records
        .insert(1, duplicate_date.response.records[0].clone());
    assert_eq!(
        duplicate_date.parse().unwrap_err(),
        HistoricalProjectionError::InvalidRecord {
            index: 1,
            reason: "record dates are not unique ascending dates"
        }
    );
    let mut reversed = recorded("688561");
    reversed.response.records.reverse();
    assert_eq!(
        reversed.parse().unwrap_err(),
        HistoricalProjectionError::InvalidRecord {
            index: 1,
            reason: "record dates are not unique ascending dates"
        }
    );
    let mut over_limit = recorded("688561-limit1");
    over_limit
        .response
        .records
        .push(over_limit.response.records[0].clone());
    assert_eq!(
        over_limit.parse().unwrap_err(),
        HistoricalProjectionError::InvalidEnvelope("record count exceeds issued limit")
    );
}

#[test]
fn wg06_historical_projection_checks_batch_timestamps_without_promoting_dates_to_publication() {
    for value in [
        "2026-07-30",
        "unix-ms:1785254400000",
        "unix-ms:-1",
        "unix-ms:9223372036854775807",
    ] {
        let mut fixture = recorded("688561");
        fixture.response.source_at = value.into();
        assert!(
            matches!(
                fixture.parse(),
                Err(HistoricalProjectionError::InvalidEnvelope(_))
            ),
            "source_at={value}"
        );
    }
    for value in [
        "2026-10-01T17:28:26Z",
        "2026-07-30",
        "1790875706.",
        "1790875706.1234567890",
        "-1.123",
        "9223372036854775807.123",
    ] {
        let mut fixture = recorded("688561");
        fixture.response.observed_at = value.into();
        assert_eq!(
            fixture.parse().unwrap_err(),
            HistoricalProjectionError::InvalidEnvelope("invalid response observation instant")
        );
    }
    let mut empty = recorded("688561");
    empty.response.records.clear();
    let parsed = empty.parse().unwrap();
    assert!(parsed.rows.is_empty());
    assert_eq!(parsed.missing_observed_dates, empty.trading_dates);
    assert!(parsed.observed_complete_flag); // Synthetic row-set boundary, not a live authority-empty receipt.
}

#[test]
fn wg06_historical_projection_uses_the_requested_calendar_vector_and_rejects_bad_outer_evidence() {
    let mut excluded_date = recorded("688561");
    excluded_date.trading_dates.remove(0);
    assert_eq!(
        excluded_date.parse().unwrap_err(),
        HistoricalProjectionError::InvalidRecord {
            index: 0,
            reason: "record date is outside the exact trading-date request",
        }
    );
    let mut bad_calendar = recorded("688561");
    bad_calendar.trading_dates.swap(0, 1);
    assert_eq!(
        bad_calendar.parse().unwrap_err(),
        HistoricalProjectionError::InvalidRequest("invalid exact-date bounds")
    );
    for mutation in 0..4 {
        let mut fixture = recorded("688561");
        match mutation {
            0 => fixture.response.admission = QueryAdmission::Unadmitted,
            1 => fixture.response.selected_provider = "Tonghuashun".into(),
            2 => fixture.response.diagnostic_blocker = "TEST_CODE diagnostic".into(),
            _ => fixture.response.batch_id.clear(),
        }
        assert_eq!(
            fixture.parse().unwrap_err(),
            HistoricalProjectionError::InvalidEnvelope("invalid observed provider or batch")
        );
    }
}

#[test]
fn wg06_historical_projection_raw_hash_does_not_reencode_equivalent_json() {
    let original = recorded("688277");
    let original_row = original.parse().unwrap().rows.remove(0);
    let mut whitespace = recorded("688277");
    whitespace.response.records[0].data.insert(0, b' ');
    let changed_row = whitespace.parse().unwrap().rows.remove(0);
    assert_eq!(original_row.volume_lots(), changed_row.volume_lots());
    assert_eq!(original_row.amount_cny(), changed_row.amount_cny());
    assert_ne!(
        original_row.raw_data_sha256(),
        changed_row.raw_data_sha256()
    );
    assert_eq!(
        changed_row.raw_record().data,
        whitespace.response.records[0].data
    );
    assert_eq!(
        original.saved["records"][0]["data_sha256"],
        original_row.raw_data_sha256()
    );
}

#[tokio::test]
async fn wg06_historical_projection_rejection_preserves_capture_hash_raw_rpc_error_and_trailer() {
    for reply in [
        HistoricalQueryReply::Success,
        HistoricalQueryReply::StatusWithTrailer,
    ] {
        let fixture =
            ExternalQueryWireFixture::bind_historical(reply, HistoricalCapabilityBehavior::Ready)
                .await
                .unwrap();
        let mut gateway = super::super::external_historical_bars::ExternalHistoricalBarsGateway::connect_client_bundle(fixture.bundle_path()).await.unwrap();
        fixture.release_capabilities();
        fixture.release();
        let request = HistoricalWindowRequest::new(
            InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap(),
            NaiveDate::from_ymd_opt(2026, 9, 11).unwrap(),
            NaiveDate::from_ymd_opt(2026, 9, 15).unwrap(),
            DateTime::parse_from_rfc3339("2026-09-16T15:31:00+08:00")
                .unwrap()
                .with_timezone(&Utc),
        )
        .unwrap();
        let capture = gateway.observe_once(request).await.unwrap();
        let before_hash = capture.capture_hash().to_owned();
        let before_request = capture.observation().request_bytes.clone();
        let before_error = capture
            .observation()
            .result
            .as_ref()
            .err()
            .map(|error| format!("{error:?}"));
        let before_status = capture.observation().status.as_ref().map(|status| {
            (
                status.raw_status.code(),
                status.raw_status.message().to_owned(),
                status.details.clone(),
                status
                    .raw_status
                    .metadata()
                    .get_bin("magic-error-detail-bin")
                    .unwrap()
                    .as_encoded_bytes()
                    .to_vec(),
                status.error_detail_trailer.clone(),
            )
        });
        let error = project_observed_historical_records(&capture).unwrap_err();
        if reply == HistoricalQueryReply::StatusWithTrailer {
            assert_eq!(error, HistoricalProjectionError::QueryRejected);
            assert!(before_status.is_some());
        } else {
            assert_eq!(
                error,
                HistoricalProjectionError::InvalidEnvelope("invalid response observation instant")
            );
        }
        assert_eq!(capture.capture_hash(), before_hash);
        assert_eq!(capture.observation().request_bytes, before_request);
        assert_eq!(
            capture
                .observation()
                .result
                .as_ref()
                .err()
                .map(|error| format!("{error:?}")),
            before_error
        );
        assert_eq!(
            capture.observation().status.as_ref().map(|status| (
                status.raw_status.code(),
                status.raw_status.message().to_owned(),
                status.details.clone(),
                status
                    .raw_status
                    .metadata()
                    .get_bin("magic-error-detail-bin")
                    .unwrap()
                    .as_encoded_bytes()
                    .to_vec(),
                status.error_detail_trailer.clone(),
            )),
            before_status
        );
        assert_eq!(fixture.snapshot().calls, 1);
    }
}
