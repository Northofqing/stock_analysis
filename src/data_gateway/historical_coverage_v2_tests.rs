use super::*;
use serde_json::{json, Value};

struct SyntheticRpcShape {
    request: wire::QueryRequest,
    response: wire::QueryResponse,
}

impl SyntheticRpcShape {
    fn from_actual_normal_provider(limit: usize) -> Self {
        let (raw, expected_sha) = match limit {
            1 => (NORMAL_PROVIDER_LIMIT_1, NORMAL_PROVIDER_LIMIT_1_SHA256),
            15 => (NORMAL_PROVIDER_LIMIT_15, NORMAL_PROVIDER_LIMIT_15_SHA256),
            _ => panic!("unsupported TEST_CODE original provider receipt"),
        };
        assert_eq!(sha(raw.as_bytes()), expected_sha);
        let original: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(
            original["scope"],
            "NormalProviderObservationNotGrpcAcceptance"
        );
        assert_eq!(original["request"]["limit"], limit);
        let payload = serde_json::to_vec(&original["request"]).unwrap();
        let request_id = format!("TEST_CODE_SYNTHETIC_V2_RPC_LIMIT_{limit}");
        let request = wire::QueryRequest {
            context: Some(wire::RequestContext {
                protocol_version: 1,
                request_id: request_id.clone(),
            }),
            preferred_provider: "HithinkFinance".into(),
            payload: Some(wire::CanonicalPayload {
                schema: REQUEST_SCHEMA.into(),
                schema_version: 2,
                content_type: CONTENT_TYPE.into(),
                data: payload.clone(),
            }),
            allow_unadmitted: false,
        };
        let envelope = json!({
            "request_id": request_id,
            "request_payload_sha256": sha(&payload),
            "request": original["request"],
            "coverage_scope": SCOPE,
            "result": original["result"],
        });
        let batch = &envelope["result"]["batch"];
        let response = wire::QueryResponse {
            request_id,
            operation: wire::Operation::HistoricalBars as i32,
            admission: wire::AdmissionState::Admitted as i32,
            selected_provider: "HithinkFinance".into(),
            batch_id: batch["provenance"]["batch_id"].as_str().unwrap().into(),
            complete: batch["quality"]["complete"].as_bool().unwrap(),
            observed_at: batch["provenance"]["fetched_at"].as_str().unwrap().into(),
            source_at: batch["provenance"]["source_at"].as_str().unwrap().into(),
            records: vec![wire::CanonicalPayload {
                schema: COVERAGE_SCHEMA.into(),
                schema_version: 2,
                content_type: CONTENT_TYPE.into(),
                data: serde_json::to_vec_pretty(&envelope).unwrap(),
            }],
            diagnostic_blocker: String::new(),
        };
        Self { request, response }
    }

    fn parse(
        &self,
    ) -> Result<RecordedHistoricalCoverageV2Observation, RecordedHistoricalCoverageV2Error> {
        parse_recorded_historical_coverage_v2(
            &self.request.encode_to_vec(),
            &self.response.encode_to_vec(),
        )
    }

    fn mutate_envelope(&mut self, change: impl FnOnce(&mut Value)) {
        let mut envelope: Value = serde_json::from_slice(&self.response.records[0].data).unwrap();
        change(&mut envelope);
        self.response.records[0].data = serde_json::to_vec_pretty(&envelope).unwrap();
    }
}

#[test]
fn wg06_historical_coverage_v2_actual_normal_provider_limits_remain_recorded_unknown() {
    for (limit, rows, complete, truncated) in [(15, 11, true, false), (1, 1, false, true)] {
        let fixture = SyntheticRpcShape::from_actual_normal_provider(limit);
        let observation = fixture.parse().unwrap();
        assert_eq!(observation.request_wire(), fixture.request.encode_to_vec());
        assert_eq!(
            observation.response_wire(),
            fixture.response.encode_to_vec()
        );
        assert_eq!(
            observation.coverage_json(),
            fixture.response.records[0].data
        );
        assert_eq!(
            observation.request_payload_sha256(),
            sha(&fixture.request.payload.as_ref().unwrap().data)
        );
        assert_eq!(observation.caller_limit(), limit);
        assert_eq!(observation.native_row_dates().len(), rows);
        assert_eq!(observation.validated_source_rows_claim(), 11);
        assert_eq!(observation.observed_complete_flag(), complete);
        assert_eq!(observation.caller_limit_truncated(), truncated);
        assert_eq!(
            observation.native_row_dates().last().unwrap().to_string(),
            "2026-07-30"
        );
        for unknown in [
            observation.source_exhaustion(),
            observation.authority_calendar_coverage(),
            observation.missing_date_reasons(),
        ] {
            assert_eq!(unknown, "Unknown");
        }
        assert_eq!(observation.source_revision(), "NotProvided");
        assert_eq!(observation.historical_publication_time(), "NotProvided");
        assert!(!observation.pit_guarantee());
        assert!(!observation.upstream_body_digest_independently_verified());
        assert!(observation.upstream_repository_admitted_claim());
        assert_eq!(observation.scope(), "RecordedOnlyNotGrpcAcceptance");
        assert_eq!(observation.outer_provider_claim(), "HithinkFinance");
        assert_eq!(observation.record_provider_claim(), ProviderId::Tonghuashun);
        assert_eq!(observation.volume_unit(), "LotsOf100Shares");
        assert_eq!(observation.amount_unit(), "CNY");
        let exact_json: Value = serde_json::from_slice(observation.coverage_json()).unwrap();
        let row = exact_json["result"]["batch"]["records"]
            .as_array()
            .unwrap()
            .last()
            .unwrap();
        assert_eq!(row["volume"], 83915.9);
        assert_eq!(row["amount"], 212389898.25);
        assert_eq!(row["source_at"], "2026-07-30");
        assert_eq!(row["observed_at"], fixture.response.observed_at);
    }
}

#[test]
fn wg06_historical_coverage_v2_binds_actual_payload_bytes_not_semantic_reencoding() {
    let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
    fixture
        .request
        .payload
        .as_mut()
        .unwrap()
        .data
        .insert(0, b' ');
    assert_eq!(
        fixture.parse().unwrap_err(),
        RecordedHistoricalCoverageV2Error::RequestBinding
    );
    let actual_sha = sha(&fixture.request.payload.as_ref().unwrap().data);
    fixture.mutate_envelope(|envelope| envelope["request_payload_sha256"] = json!(actual_sha));
    let observation = fixture.parse().unwrap();
    assert_eq!(observation.request_payload_sha256(), actual_sha);
    assert_eq!(observation.request_wire(), fixture.request.encode_to_vec());
    assert_eq!(fixture.request.payload.as_ref().unwrap().data[0], b' ');
}

#[test]
fn wg06_historical_coverage_v2_request_id_window_and_limit_conflicts_reject() {
    for (path, replacement) in [
        ("/request_id", json!("TEST_CODE_OTHER_REQUEST")),
        ("/request/start", json!("2026-07-17")),
        ("/request/end", json!("2026-07-31")),
        ("/request/limit", json!(15)),
        ("/request/instrument/code", json!("688277")),
        ("/request_payload_sha256", json!("0".repeat(64))),
    ] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        fixture.mutate_envelope(|envelope| *envelope.pointer_mut(path).unwrap() = replacement);
        assert_eq!(
            fixture.parse().unwrap_err(),
            RecordedHistoricalCoverageV2Error::RequestBinding,
            "{path}"
        );
    }
    let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
    fixture.response.request_id.push_str("_OTHER");
    assert_eq!(
        fixture.parse().unwrap_err(),
        RecordedHistoricalCoverageV2Error::Envelope
    );
}

#[test]
fn wg06_historical_coverage_v2_unknown_and_duplicate_fields_never_upgrade_scope() {
    for path in [
        "",
        "/request",
        "/request/instrument",
        "/result",
        "/result/batch",
        "/result/batch/quality",
        "/result/batch/provenance",
        "/result/coverage",
        "/result/coverage/native_response",
        "/result/coverage/native_response/adjust",
        "/result/coverage/response_receipt",
        "/result/batch/records/0",
        "/result/batch/records/0/instrument",
    ] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        fixture.mutate_envelope(|envelope| {
            envelope
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(
                    "TEST_CODE_SECRET_LIKE_FIELD".into(),
                    json!("TEST_CODE_SECRET_NEVER_ECHO"),
                );
        });
        let error = fixture.parse().unwrap_err();
        assert_eq!(error, RecordedHistoricalCoverageV2Error::Envelope, "{path}");
        assert!(!error.to_string().contains("SECRET"));
    }
    for (field, replacement) in [
        (
            "\"request_id\":",
            "\"request_id\":\"TEST_CODE_DUPLICATE\",\"request_id\":",
        ),
        (
            "\"returned_rows\":",
            "\"returned_rows\":1,\"returned_rows\":",
        ),
        ("\"volume\":", "\"volume\":83915.9,\"volume\":"),
    ] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        let raw = String::from_utf8(fixture.response.records[0].data.clone()).unwrap();
        assert!(raw.contains(field));
        fixture.response.records[0].data = raw.replacen(field, replacement, 1).into_bytes();
        assert_eq!(
            fixture.parse().unwrap_err(),
            RecordedHistoricalCoverageV2Error::Envelope
        );
    }
}

#[test]
fn wg06_historical_coverage_v2_claims_cannot_be_promoted_or_contradict_limit() {
    for (path, replacement, expected) in [
        (
            "/result/coverage/pit_guarantee",
            json!(true),
            RecordedHistoricalCoverageV2Error::Coverage,
        ),
        (
            "/result/coverage/response_validated",
            json!(false),
            RecordedHistoricalCoverageV2Error::Coverage,
        ),
        (
            "/result/coverage/source_exhaustion",
            json!("Exhausted"),
            RecordedHistoricalCoverageV2Error::Envelope,
        ),
        (
            "/result/coverage/source_exhaustion",
            json!({"Unknown": null}),
            RecordedHistoricalCoverageV2Error::Envelope,
        ),
        (
            "/result/coverage/authority_calendar_coverage",
            json!("Complete"),
            RecordedHistoricalCoverageV2Error::Envelope,
        ),
        (
            "/result/coverage/missing_date_reasons",
            json!([]),
            RecordedHistoricalCoverageV2Error::Envelope,
        ),
        (
            "/result/coverage/source_revision",
            json!("Known"),
            RecordedHistoricalCoverageV2Error::Envelope,
        ),
        (
            "/result/coverage/historical_publication_time",
            json!("2026-07-30"),
            RecordedHistoricalCoverageV2Error::Envelope,
        ),
        (
            "/result/coverage/returned_rows",
            json!(11),
            RecordedHistoricalCoverageV2Error::Coverage,
        ),
        (
            "/result/coverage/validated_source_rows",
            json!(3001),
            RecordedHistoricalCoverageV2Error::Coverage,
        ),
        (
            "/result/coverage/caller_limit_truncated",
            json!(false),
            RecordedHistoricalCoverageV2Error::Coverage,
        ),
        (
            "/result/batch/quality/complete",
            json!(true),
            RecordedHistoricalCoverageV2Error::Coverage,
        ),
        (
            "/result/batch/quality/issues",
            json!([]),
            RecordedHistoricalCoverageV2Error::Coverage,
        ),
        (
            "/coverage_scope",
            json!("HistoricalCoverageComplete"),
            RecordedHistoricalCoverageV2Error::Coverage,
        ),
    ] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        fixture.mutate_envelope(|envelope| *envelope.pointer_mut(path).unwrap() = replacement);
        assert_eq!(fixture.parse().unwrap_err(), expected, "{path}");
    }
}

#[test]
fn wg06_historical_coverage_v2_native_echo_is_not_inferred_from_requested_dates() {
    for (path, replacement) in [
        (
            "/result/coverage/native_response/request_id",
            json!("TEST_CODE_OTHER_NATIVE_RESPONSE"),
        ),
        (
            "/result/coverage/native_response/thscode",
            json!("688561.SZ"),
        ),
        ("/result/coverage/native_response/interval", json!("1m")),
        (
            "/result/coverage/native_response/timestamp_ms",
            json!(1785254400000_i64),
        ),
        (
            "/result/coverage/native_response/adjust",
            json!({"state":"Absent"}),
        ),
        (
            "/result/coverage/native_response/adjust",
            json!({"state":"Null"}),
        ),
        (
            "/result/coverage/native_response/adjust",
            json!({"state":"Value","value":"qfq"}),
        ),
    ] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        // Keep timestamp/source_at internally coherent so this exercises the
        // actual native latest-date binding, rather than an earlier mismatch.
        let source_at = if path.ends_with("timestamp_ms") {
            Some(format!("unix-ms:{}", replacement.as_i64().unwrap()))
        } else {
            None
        };
        if let Some(source_at) = &source_at {
            fixture.response.source_at = source_at.clone();
        }
        fixture.mutate_envelope(|envelope| {
            *envelope.pointer_mut(path).unwrap() = replacement;
            if let Some(source_at) = source_at {
                envelope["result"]["batch"]["provenance"]["source_at"] = json!(source_at);
            }
        });
        assert_eq!(
            fixture.parse().unwrap_err(),
            RecordedHistoricalCoverageV2Error::NativeContext,
            "{path}"
        );
    }
    let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
    fixture.mutate_envelope(|envelope| {
        envelope["result"]["coverage"]
            .as_object_mut()
            .unwrap()
            .remove("native_response");
    });
    assert_eq!(
        fixture.parse().unwrap_err(),
        RecordedHistoricalCoverageV2Error::Envelope
    );
}

#[test]
fn wg06_historical_coverage_v2_receipt_context_and_digest_claim_are_distinct() {
    let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
    fixture.mutate_envelope(|envelope| {
        envelope["result"]["coverage"]["response_receipt"]["body_sha256"] = json!("a".repeat(64))
    });
    let observation = fixture.parse().unwrap();
    assert_eq!(observation.claimed_upstream_body_sha256(), "a".repeat(64));
    assert_eq!(observation.claimed_upstream_body_byte_length(), 1752);
    assert!(!observation.upstream_body_digest_independently_verified());
    for suffix in [
        "&thscode=688561.SH",
        "&TEST_CODE_SECRET=query",
        "#TEST_CODE_SECRET",
    ] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        fixture.mutate_envelope(|envelope| {
            let url = envelope["result"]["coverage"]["response_receipt"]["final_url"]
                .as_str()
                .unwrap()
                .to_owned();
            envelope["result"]["coverage"]["response_receipt"]["final_url"] =
                json!(format!("{url}{suffix}"));
        });
        assert_eq!(
            fixture.parse().unwrap_err(),
            RecordedHistoricalCoverageV2Error::Receipt
        );
    }
    for (from, to) in [
        ("688561.SH", "688277.SH"),
        ("1784131200000", "1784217600000"),
        ("adjust=none", "adjust=qfq"),
        ("fuyao.aicubes.cn", "TEST_CODE_SECRET.invalid"),
    ] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        fixture.mutate_envelope(|envelope| {
            let url = envelope["result"]["coverage"]["response_receipt"]["final_url"]
                .as_str()
                .unwrap();
            envelope["result"]["coverage"]["response_receipt"]["final_url"] =
                json!(url.replace(from, to));
        });
        let error = fixture.parse().unwrap_err();
        assert_eq!(error, RecordedHistoricalCoverageV2Error::Receipt);
        assert!(!error.to_string().contains("SECRET"));
    }
}

#[test]
fn wg06_historical_coverage_v2_bad_record_rejects_whole_observation() {
    for (path, replacement, expected) in [
        (
            "/provider",
            json!("HithinkFinance"),
            RecordedHistoricalCoverageV2Error::Row { index: 0 },
        ),
        (
            "/adjustment",
            json!("Forward"),
            RecordedHistoricalCoverageV2Error::Row { index: 0 },
        ),
        (
            "/batch_id",
            json!("TEST_CODE_OTHER_BATCH"),
            RecordedHistoricalCoverageV2Error::Row { index: 0 },
        ),
        (
            "/source_at",
            json!("2026-07-29"),
            RecordedHistoricalCoverageV2Error::Row { index: 0 },
        ),
        (
            "/observed_at",
            json!("1790902926.657342800"),
            RecordedHistoricalCoverageV2Error::Row { index: 0 },
        ),
        (
            "/bar_end",
            json!("2026-07-31"),
            RecordedHistoricalCoverageV2Error::Row { index: 0 },
        ),
        (
            "/instrument/exchange",
            json!("Shenzhen"),
            RecordedHistoricalCoverageV2Error::Row { index: 0 },
        ),
        (
            "/high",
            json!(1.0),
            RecordedHistoricalCoverageV2Error::Row { index: 0 },
        ),
        (
            "/amount",
            json!(-1.0),
            RecordedHistoricalCoverageV2Error::Row { index: 0 },
        ),
        (
            "/volume",
            json!(-1.0),
            RecordedHistoricalCoverageV2Error::Envelope,
        ),
        (
            "/open",
            json!(0.0),
            RecordedHistoricalCoverageV2Error::Envelope,
        ),
    ] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        fixture.mutate_envelope(|envelope| {
            *envelope["result"]["batch"]["records"][0]
                .pointer_mut(path)
                .unwrap() = replacement
        });
        assert_eq!(fixture.parse().unwrap_err(), expected, "{path}");
    }
    let mut outside = SyntheticRpcShape::from_actual_normal_provider(1);
    outside.mutate_envelope(|envelope| {
        for field in ["bar_start", "bar_end", "source_at"] {
            envelope["result"]["batch"]["records"][0][field] = json!("2026-07-31");
        }
    });
    assert_eq!(
        outside.parse().unwrap_err(),
        RecordedHistoricalCoverageV2Error::Row { index: 0 }
    );
    for duplicate in [true, false] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(15);
        fixture.mutate_envelope(|envelope| {
            let rows = envelope["result"]["batch"]["records"]
                .as_array_mut()
                .unwrap();
            if duplicate {
                rows[1] = rows[0].clone();
            } else {
                rows.swap(0, 1);
            }
        });
        assert_eq!(
            fixture.parse().unwrap_err(),
            RecordedHistoricalCoverageV2Error::Row { index: 1 }
        );
    }
}

#[test]
fn wg06_historical_coverage_v2_synthetic_empty_is_unknown_not_no_data_authority() {
    let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
    fixture.response.complete = true;
    fixture.mutate_envelope(|envelope| {
        envelope["result"]["batch"]["records"] = json!([]);
        envelope["result"]["batch"]["quality"] = json!({"complete":true,"issues":[]});
        envelope["result"]["coverage"]["validated_source_rows"] = json!(0);
        envelope["result"]["coverage"]["returned_rows"] = json!(0);
        envelope["result"]["coverage"]["caller_limit_truncated"] = json!(false);
    });
    let observation = fixture.parse().unwrap();
    assert!(observation.native_row_dates().is_empty());
    assert_eq!(observation.validated_source_rows_claim(), 0);
    assert!(observation.observed_complete_flag());
    assert_eq!(observation.source_exhaustion(), "Unknown");
    assert_eq!(observation.authority_calendar_coverage(), "Unknown");
    assert!(!observation.pit_guarantee());
}

#[test]
fn wg06_historical_coverage_v2_wire_and_single_envelope_fail_closed() {
    for mode in 0..6 {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        match mode {
            0 => fixture.response.admission = wire::AdmissionState::Unadmitted as i32,
            1 => fixture.response.selected_provider = "Tonghuashun".into(),
            2 => fixture.response.operation = wire::Operation::MinuteData as i32,
            3 => fixture
                .response
                .records
                .push(fixture.response.records[0].clone()),
            4 => fixture.response.records[0].schema_version = 1,
            5 => fixture.response.diagnostic_blocker = "TEST_CODE_DIAGNOSTIC_ONLY".into(),
            _ => unreachable!(),
        }
        assert_eq!(
            fixture.parse().unwrap_err(),
            RecordedHistoricalCoverageV2Error::Envelope
        );
    }
    let fixture = SyntheticRpcShape::from_actual_normal_provider(1);
    let request = fixture.request.encode_to_vec();
    let response = fixture.response.encode_to_vec();
    let mut duplicate_id = response.clone();
    duplicate_id.push(0x0a); // A real extra occurrence of QueryResponse.request_id.
    duplicate_id.push(fixture.response.request_id.len() as u8);
    duplicate_id.extend_from_slice(fixture.response.request_id.as_bytes());
    assert_eq!(
        parse_recorded_historical_coverage_v2(&request, &duplicate_id).unwrap_err(),
        RecordedHistoricalCoverageV2Error::ResponseWire
    );
    assert_eq!(
        parse_recorded_historical_coverage_v2(&request, &[]).unwrap_err(),
        RecordedHistoricalCoverageV2Error::Envelope
    );
    assert_eq!(
        parse_recorded_historical_coverage_v2(&[0xff], &response).unwrap_err(),
        RecordedHistoricalCoverageV2Error::RequestWire
    );
    assert_eq!(
        parse_recorded_historical_coverage_v2(&vec![0; MAX_REQUEST_BYTES + 1], &response)
            .unwrap_err(),
        RecordedHistoricalCoverageV2Error::InputBound
    );
    assert_eq!(
        parse_recorded_historical_coverage_v2(&request, &vec![0; MAX_RESPONSE_BYTES + 1])
            .unwrap_err(),
        RecordedHistoricalCoverageV2Error::InputBound
    );
}

#[test]
fn wg06_historical_coverage_v2_invalid_request_is_rejected_before_response() {
    for (path, value) in [
        ("/limit", json!(0)),
        ("/limit", json!(65536)),
        ("/start", json!("1899-01-01")),
        ("/start", json!("2026-07-31")),
        ("/interval", json!("Minute1")),
        ("/instrument/code", json!("688561 ")),
        ("/instrument/asset_class", json!("Index")),
        ("/instrument/exchange", json!("Beijing")),
    ] {
        let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
        let mut request: Value =
            serde_json::from_slice(&fixture.request.payload.as_ref().unwrap().data).unwrap();
        *request.pointer_mut(path).unwrap() = value;
        fixture.request.payload.as_mut().unwrap().data = serde_json::to_vec(&request).unwrap();
        assert_eq!(
            fixture.parse().unwrap_err(),
            RecordedHistoricalCoverageV2Error::Request,
            "{path}"
        );
    }
    let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
    fixture.request.allow_unadmitted = true;
    assert_eq!(
        fixture.parse().unwrap_err(),
        RecordedHistoricalCoverageV2Error::Request
    );
}

#[test]
fn wg06_historical_coverage_v2_synthetic_shenzhen_binds_actual_context_fields() {
    // This deliberately changed instrument is a SYNTHETIC boundary fixture,
    // not a second observed source receipt for Shenzhen.
    let mut fixture = SyntheticRpcShape::from_actual_normal_provider(1);
    let mut requested: Value =
        serde_json::from_slice(&fixture.request.payload.as_ref().unwrap().data).unwrap();
    requested["instrument"] = json!({"exchange":"Shenzhen","asset_class":"Equity","code":"000001"});
    let payload = serde_json::to_vec(&requested).unwrap();
    fixture.request.payload.as_mut().unwrap().data = payload.clone();
    fixture.mutate_envelope(|envelope| {
        envelope["request"] = requested.clone();
        envelope["request_payload_sha256"] = json!(sha(&payload));
        envelope["result"]["batch"]["records"][0]["instrument"] = requested["instrument"].clone();
        envelope["result"]["coverage"]["native_response"]["thscode"] = json!("000001.SZ");
        let url = envelope["result"]["coverage"]["response_receipt"]["final_url"]
            .as_str()
            .unwrap();
        envelope["result"]["coverage"]["response_receipt"]["final_url"] =
            json!(url.replace("688561.SH", "000001.SZ"));
    });
    let observation = fixture.parse().unwrap();
    assert_eq!(observation.instrument().exchange(), Exchange::Shenzhen);
    assert_eq!(observation.instrument().code(), "000001");
    assert_eq!(observation.authority_calendar_coverage(), "Unknown");
    fixture.mutate_envelope(|envelope| {
        envelope["result"]["coverage"]["native_response"]["thscode"] = json!("000001.SH")
    });
    assert_eq!(
        fixture.parse().unwrap_err(),
        RecordedHistoricalCoverageV2Error::NativeContext
    );
}

// Exact public Windows .10 NormalProvider receipts, not gRPC receipts. These
// inputs retain their original scope. All QueryRequest/QueryResponse envelopes
// made below are deliberately SYNTHETIC v2 composition shapes.

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

const NORMAL_PROVIDER_LIMIT_1_SHA256: &str =
    "21257a4d4346b881cf02966f0aefa7cda24e81ef41d475e07939e5c959d4fac9";

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

const NORMAL_PROVIDER_LIMIT_15_SHA256: &str =
    "24135564cd28079c5bebc9807eb38c548104e21a4dbd4ccbca3b1780f5a7b1f1";
