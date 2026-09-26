//! Acquisition-boundary regression tests. All audit storage is test-only SQLite.
use super::*;
use crate::database::DatabaseManager;
use crate::grpc_client::{
    envelope::{AcquisitionProvenance, CanonicalRecord, QueryAdmission, QueryResult},
    errors::GrpcError,
};
use crate::market_domain::ProviderId;
use crate::market_domain::{AssetClass, Exchange, FlowInterval, InstrumentId, NorthboundChannel};
use chrono::{NaiveDate, TimeZone, Utc};
use diesel::{sql_types::Text, RunQueryDsl};
use prost::Message;

fn wire_error(provider: Option<&str>, invalid: bool) -> GrpcError {
    let detail = crate::grpc_client::pb::magic::market::v1::ErrorDetail {
        request_id: "TEST_CODE_d15_request".into(),
        operation: crate::grpc_client::pb::magic::market::v1::Operation::Consensus as i32,
        provider: provider.unwrap_or_default().into(),
        reason_code: "provider_transport".into(),
        retryable: true,
        ..Default::default()
    };
    GrpcError::from(tonic::Status::with_details(
        if invalid {
            tonic::Code::InvalidArgument
        } else {
            tonic::Code::Unavailable
        },
        "TEST_CODE upstream failure",
        detail.encode_to_vec().into(),
    ))
}

fn wire_batch(provider: &str, records: serde_json::Value) -> QueryResult {
    QueryResult {
        admission: QueryAdmission::Admitted,
        selected_provider: provider.into(),
        batch_id: "TEST_CODE_d15_batch".into(),
        complete: true,
        observed_at: "2026-09-25T16:00:00+08:00".into(),
        source_at: String::new(),
        records: vec![CanonicalRecord {
            schema: "TEST_CODE_query_boundary".into(),
            schema_version: 1,
            content_type: "application/json; charset=utf-8".into(),
            data: serde_json::to_vec(&records).unwrap(),
        }],
        provenance: AcquisitionProvenance::LocalWireSource("TEST_CODE_provider_source".into()),
        diagnostic_blocker: String::new(),
    }
}

fn init() -> grpc_source::TestGrpcEnvGuard {
    let guard = grpc_source::test_grpc_env_guard();
    DatabaseManager::init(None).unwrap();
    std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
    std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
    guard
}

// The dispatch only erases the record type AFTER the real public Gateway call.
async fn fetch(entry: usize) -> Result<BatchEvidence, GatewayError> {
    let date = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
    let codes = vec!["600519".to_owned()];
    macro_rules! evidence {
        ($call:expr) => {
            $call.await.map(|batch| batch.evidence().clone())
        };
    }
    match entry {
        0 => evidence!(ConsensusDataGateway::new().fetch(&codes[0])),
        1 => evidence!(EventCalendarGateway::new().market_announcements(date, 20)),
        2 => evidence!(FuturesDeliveryGateway::new().cffex_contract_month(2026, 9)),
        3 => evidence!(DragonTigerGateway::new().market_review(date, 20, 20)),
        4 => evidence!(BlockTradesGateway::new().market_review(&codes, date)),
        5 => evidence!(SinaInstrumentNewsGateway::new().instrument_news_in_range(
            &codes[0],
            Utc.with_ymd_and_hms(2026, 9, 24, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 25, 0, 0, 0).unwrap()
        )),
        6 => evidence!(ResearchDataGateway::new().instrument_reports("TEST_CODE_600519", 20)),
        7 => evidence!(GlobalMarketGateway::new().us_indices()),
        8 => evidence!(GlobalMarketGateway::new().usd_cny()),
        9 => evidence!(CompanyDataGateway::new().balance_sheets(&codes)),
        10 => evidence!(CompanyDataGateway::new().income_statements(&codes)),
        11 => evidence!(CompanyDataGateway::new().cash_flow_statements(&codes)),
        12 => evidence!(CompanyDataGateway::new().market_statistics(&codes)),
        13 => evidence!(CapitalDataGateway::new().instrument_fund_flow(
            &codes[0],
            FlowInterval::Day1,
            20
        )),
        14 => {
            evidence!(CapitalDataGateway::new().northbound_daily(date, NorthboundChannel::Shanghai))
        }
        15 => {
            let request = CurrentAuctionRequest::new(
                vec![InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap()],
                AuctionStage::Live,
            )
            .unwrap();
            evidence!(CurrentAuctionObservationsGateway::new().fetch(&request))
        }
        16 => evidence!(EconomicReleaseObservationsGateway::new()
            .fetch(&EconomicReleaseObservationsRequest::new(20, None).unwrap())),
        17 => evidence!(EconomicReleaseScheduleGateway::new()
            .fetch(&EconomicReleaseScheduleRequest::new(date, date, 20).unwrap())),
        _ => unreachable!(),
    }
}

fn all_audits() -> Vec<Audit> {
    let mut conn = DatabaseManager::get().get_conn().unwrap();
    diesel::sql_query("SELECT a.id,a.capability,a.provider,a.outcome,a.reason_code,a.request_hash,c.record_hash FROM data_acquisition_audit a JOIN data_acquisition_audit_chain c ON c.acquisition_audit_id=a.id ORDER BY a.id").load(&mut *conn).unwrap()
}

#[tokio::test]
async fn d15_attribution_public_gateways_wire_failures_and_history() {
    let _env = init();
    let mut failures = Vec::new();
    for entry in 0..18 {
        for (provider, expected) in [
            (Some("Sina"), "Sina"),
            (None, "Custom"),
            (Some("FutureUnknownProvider"), "Custom"),
        ] {
            let before = all_audits();
            grpc_source::set_test_query_responses(vec![Err(wire_error(provider, false))]);
            let returned = fetch(entry).await.unwrap_err();
            let after = all_audits();
            assert_eq!(
                &after[..before.len()],
                before.as_slice(),
                "historical bytes/hash must not change"
            );
            assert_eq!(after.len(), before.len() + 1, "entry {entry}");
            let audit = after.last().unwrap();
            if audit.provider != expected {
                failures.push(format!(
                    "entry={entry} wire={provider:?}: {} != {expected}",
                    audit.provider
                ));
            }
            assert_eq!(
                returned.provider(),
                if expected == "Sina" {
                    Some(ProviderId::Sina)
                } else {
                    None
                }
            );
            assert_eq!(audit.reason_code, returned.reason_code());
            assert_eq!(audit.outcome, returned.audit_outcome());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let rows = all_audits();
    let mut conn = DatabaseManager::get().get_conn().unwrap();
    for (index, row) in rows.iter().enumerate() {
        let receipt = crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt {
            audit_id: row.id,
            record_hash: row.record_hash.clone(),
            current_outcome: row.outcome.clone(),
            previous_outcome: rows[..index]
                .iter()
                .rev()
                .find(|old| old.capability == row.capability && old.provider == row.provider)
                .map(|old| old.outcome.clone()),
        };
        crate::database::data_acquisition_audit::read_verified_acquisition_audit(
            &mut conn, &receipt,
        )
        .expect("full historical chain/receipt stays verifiable");
    }
}

#[tokio::test]
async fn d15_attribution_primary_fallback_verified_empty_and_available() {
    let _env = init();
    for provider in ["Eastmoney", "Sina"] {
        for rows in [
            serde_json::json!([]),
            serde_json::json!([{"report_count":2,"broker_count":1,"rating_distribution":{"buy":2},"eps_this_year_avg":null,"eps_next_year_avg":null,"eps_next2_year_avg":null}]),
        ] {
            let before = all_audits().len();
            grpc_source::set_test_query_responses(vec![Ok(wire_batch(provider, rows.clone()))]);
            let batch = ConsensusDataGateway::new().fetch("600519").await.unwrap();
            assert_eq!(batch.records().len(), rows.as_array().unwrap().len());
            assert_eq!(batch.evidence().batch_id, "TEST_CODE_d15_batch");
            let after = all_audits();
            assert_eq!(after.len(), before + 1);
            assert_eq!(after.last().unwrap().provider, provider);
            assert_eq!(
                after.last().unwrap().outcome,
                if rows.as_array().unwrap().is_empty() {
                    "verified_empty"
                } else {
                    "available"
                }
            );
        }
    }
}

#[tokio::test]
async fn d15_attribution_wire_invalid_argument_keeps_known_provider() {
    let _env = init();
    grpc_source::set_test_query_responses(vec![Err(wire_error(Some("Sina"), true))]);
    let error = ConsensusDataGateway::new()
        .fetch("600519")
        .await
        .unwrap_err();
    assert_eq!(error.provider(), Some(ProviderId::Sina));
    assert!(!error.retryable());
    assert_eq!(error.audit_outcome(), "invalid_request");
    assert_eq!(all_audits().last().unwrap().provider, "Sina");
}

#[tokio::test]
async fn d15_attribution_fixed_contract_mismatch_never_becomes_accepted() {
    let _env = init();
    for entry in 15..18 {
        for provider in ["Sina", "FutureUnknownProvider"] {
            grpc_source::set_test_query_responses(vec![Ok(wire_batch(
                provider,
                serde_json::json!([]),
            ))]);
            let error = fetch(entry).await.unwrap_err();
            assert_eq!(error.reason_code(), "invalid_evidence");
            assert_eq!(
                error.provider(),
                if provider == "Sina" {
                    Some(ProviderId::Sina)
                } else {
                    None
                },
                "entry={entry}"
            );
            assert_eq!(
                all_audits().last().unwrap().provider,
                if provider == "Sina" { "Sina" } else { "Custom" }
            );
            assert_ne!(all_audits().last().unwrap().outcome, "available");
        }
    }
    let evidence = BatchEvidence {
        provider: ProviderId::Sina,
        source: "eastmoney-global-news".into(),
        source_at: Some("2026-09-25 15:00:00".into()),
        observed_at: "2026-09-25T16:00:00+08:00".into(),
        batch_id: "TEST_CODE_mismatch".into(),
    };
    let error =
        global_news::validate_global_news_batch_evidence(GlobalNewsProvider::Eastmoney, &evidence)
            .unwrap_err();
    assert_eq!(error.provider(), Some(ProviderId::Sina));
}

#[tokio::test]
async fn d15_attribution_top_n_admission_precedes_success_audit() {
    let _env = init();
    for provider in ["Eastmoney", "Sina"] {
        let rows = serde_json::json!([
            {"metric":"VolumeRatio","ordinal":1,"code":"600001","label":"TEST_CODE_volume","value":1.0,"unit":"Multiple","trading_date":"2026-09-25","filter_identity":"TEST_CODE_FILTER","provider_declared_total":2,"inspected_row_count":2,"evidence":{"provider":provider,"source":"eastmoney-web","source_at":null,"observed_at":"2026-09-25T16:00:00+08:00","batch_id":"TEST_CODE_d15_batch"}},
            {"metric":"MainNetInflow","ordinal":1,"code":"600002","label":"TEST_CODE_inflow","value":1.0,"unit":"Yuan","trading_date":"2026-09-25","filter_identity":"TEST_CODE_FILTER","provider_declared_total":2,"inspected_row_count":2,"evidence":{"provider":provider,"source":"eastmoney-web","source_at":null,"observed_at":"2026-09-25T16:00:00+08:00","batch_id":"TEST_CODE_d15_inflow"}}
        ]);
        let mut query = wire_batch(provider, rows);
        query.provenance = AcquisitionProvenance::LocalWireSource("eastmoney-web".into());
        grpc_source::set_test_query_responses(vec![Ok(query)]);
        let before = all_audits().len();
        let result = CapitalDataGateway::new()
            .provider_top_n_pair(NaiveDate::from_ymd_opt(2026, 9, 25).unwrap())
            .await;
        assert_eq!(result.is_ok(), provider == "Eastmoney", "{result:?}");
        let audits = all_audits();
        assert_eq!(audits.len(), before + 2);
        for row in &audits[before..] {
            assert_eq!(row.provider, provider);
            assert_eq!(
                row.outcome,
                if provider == "Eastmoney" {
                    "available"
                } else {
                    "partial"
                }
            );
        }
    }
}

#[tokio::test]
async fn d15_attribution_top_n_failure_has_two_independent_metric_audits() {
    let _env = init();
    for provider in [Some("Sina"), None] {
        let before = all_audits().len();
        grpc_source::set_test_query_responses(vec![Err(wire_error(provider, false))]);
        let error = CapitalDataGateway::new()
            .provider_top_n_pair(NaiveDate::from_ymd_opt(2026, 9, 25).unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.provider(), provider.map(|_| ProviderId::Sina));
        let after = all_audits();
        assert_eq!(after.len(), before + 2);
        let rows = &after[before..];
        assert_ne!(rows[0].request_hash, rows[1].request_hash);
        for audit in rows {
            assert_eq!(
                audit.provider,
                if provider.is_some() { "Sina" } else { "Custom" }
            );
        }
    }
}

#[tokio::test]
async fn d15_attribution_security_lifecycle_components_keep_independent_routes() {
    let _env = init();
    let date = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
    for fail_listing in [false, true] {
        for provider in [None, Some("Sina")] {
            let before = all_audits().len();
            let success = Ok(wire_batch("Tdx", serde_json::json!([])));
            let failure = Err(wire_error(provider, false));
            grpc_source::set_test_query_responses(if fail_listing {
                vec![failure, success]
            } else {
                vec![success, failure]
            });
            let context = SecurityLifecycleGateway::new()
                .acquire("600519", date, date)
                .await
                .unwrap();
            let after = all_audits();
            assert_eq!(after.len(), before + 2);
            let expected = if fail_listing {
                [if provider.is_some() { "Sina" } else { "Custom" }, "Tdx"]
            } else {
                ["Tdx", if provider.is_some() { "Sina" } else { "Custom" }]
            };
            for (audit, provider) in after[before..].iter().zip(expected) {
                assert_eq!(audit.provider, provider);
            }
            assert_eq!(context.corporate_actions.is_verified_empty(), fail_listing);
            if !fail_listing {
                assert!(matches!(
                    context.corporate_actions,
                    security_lifecycle::CorporateActionState::Unavailable(_)
                ));
            }
        }
    }
}

#[tokio::test]
async fn d15_attribution_lifecycle_fixed_tdx_contract_rejects_other_success() {
    let _env = init();
    let date = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
    grpc_source::set_test_query_responses(vec![
        Ok(wire_batch("Sina", serde_json::json!([]))),
        Ok(wire_batch("Sina", serde_json::json!([]))),
    ]);
    let before = all_audits().len();
    let context = SecurityLifecycleGateway::new()
        .acquire("600519", date, date)
        .await
        .unwrap();
    assert!(
        matches!(
            context.corporate_actions,
            security_lifecycle::CorporateActionState::Unavailable(_)
        ),
        "fixed TDX product cannot admit Sina success"
    );
    for audit in &all_audits()[before..] {
        assert_eq!(audit.provider, "Sina");
        assert_eq!(audit.outcome, "partial");
        assert_eq!(audit.reason_code, "invalid_evidence");
    }
}

#[tokio::test]
async fn d15_attribution_retired_economic_remains_local_and_jin10() {
    let _env = init();
    // Any network query would exhaust this queue and fail the test.
    grpc_source::set_test_query_responses(vec![]);
    let error = EconomicCalendarGateway::new()
        .latest_releases(20, None)
        .await
        .unwrap_err();
    assert_eq!(error.provider(), Some(ProviderId::Jin10));
    assert_eq!(error.reason_code(), "operation_retired");
    assert!(!error.retryable());
    let audit = all_audits().pop().unwrap();
    assert_eq!(audit.provider, "Jin10");
    assert_eq!(audit.reason_code, "operation_retired");
}

#[test]
fn d15_attribution_index_sync_failure_uses_actual_route() {
    let _env = init();
    for provider in [Some("Sina"), None] {
        grpc_source::set_test_query_responses(vec![Err(wire_error(provider, false))]);
        let error = IndexDataGateway::new()
            .realtime_quotes(&["000001".into()])
            .unwrap_err();
        assert_eq!(error.provider(), provider.map(|_| ProviderId::Sina));
        assert_eq!(
            all_audits().last().unwrap().provider,
            if provider.is_some() { "Sina" } else { "Custom" }
        );
    }
}

#[derive(Debug, PartialEq, Eq, diesel::QueryableByName)]
struct Audit {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    capability: String,
    #[diesel(sql_type = Text)]
    provider: String,
    #[diesel(sql_type = Text)]
    outcome: String,
    #[diesel(sql_type = Text)]
    reason_code: String,
    #[diesel(sql_type = Text)]
    request_hash: String,
    #[diesel(sql_type = Text)]
    record_hash: String,
}

fn audits(hash: &str) -> Vec<Audit> {
    let mut conn = DatabaseManager::get().get_conn().unwrap();
    diesel::sql_query("SELECT a.id,a.capability,a.provider,a.outcome,a.reason_code,a.request_hash,c.record_hash FROM data_acquisition_audit a JOIN data_acquisition_audit_chain c ON c.acquisition_audit_id=a.id WHERE a.request_hash=? ORDER BY a.id")
        .bind::<Text, _>(hash).load(&mut *conn).unwrap()
}

#[test]
fn d15_attribution_macro_settle_preserves_terminal_provider_and_history() {
    let _env = grpc_source::test_grpc_env_guard();
    DatabaseManager::init(None).unwrap();
    for (offset, provider) in [Some(ProviderId::Sina), None].into_iter().enumerate() {
        let limit = 17301 + offset as u32;
        let error = GatewayError::classified(
            "TEST_CODE_route",
            provider,
            "unavailable",
            "provider_timeout",
            true,
            "TEST_CODE terminal failure",
        );
        let news_hash = global_news::macro_request_hash(GlobalNewsProvider::Eastmoney, limit);
        let economic_hash = economic_calendar::macro_request_hash(limit, None);
        for (hash, returned) in [
            (
                news_hash,
                global_news::audit_macro_query(
                    GlobalNewsProvider::Eastmoney,
                    limit,
                    Err(error.clone()),
                )
                .unwrap_err(),
            ),
            (
                economic_hash,
                economic_calendar::audit_macro_query(limit, None, Err(error.clone())).unwrap_err(),
            ),
        ] {
            assert_eq!(returned.provider(), error.provider());
            assert_eq!(returned.reason_code(), error.reason_code());
            assert_eq!(returned.retryable(), error.retryable());
            assert_eq!(returned.message(), error.message());
            let rows = audits(&hash);
            assert_eq!(
                rows.last().unwrap().provider,
                if provider.is_some() { "Sina" } else { "Custom" }
            );
            assert_eq!(rows.last().unwrap().reason_code, "provider_timeout");
            assert_eq!(rows.last().unwrap().outcome, "unavailable");
        }
    }
}

#[test]
fn d15_attribution_macro_wire_settle_uses_current_not_frozen_projection() {
    let _env = init();
    let wire = Err(wire_error(Some("Sina"), true));
    for provider in [
        GlobalNewsProvider::Eastmoney,
        GlobalNewsProvider::Cailianpress,
        GlobalNewsProvider::Jin10,
        GlobalNewsProvider::ThePaper,
    ] {
        let projected = grpc_source::macro_queries::news_outcome(
            provider,
            20,
            crate::grpc_contract::methods::ContractProfile::LocalBridgeV1,
            &wire,
        );
        let error = global_news::audit_macro_query(provider, 20, projected).unwrap_err();
        assert_eq!(error.provider(), Some(ProviderId::Sina));
        assert_eq!(error.audit_outcome(), "invalid_request");
        assert_eq!(all_audits().last().unwrap().provider, "Sina");
    }
    let projected = grpc_source::macro_queries::economic_outcome(&wire);
    let error = economic_calendar::audit_macro_query(20, None, projected).unwrap_err();
    assert_eq!(error.provider(), Some(ProviderId::Sina));
    assert_eq!(all_audits().last().unwrap().provider, "Sina");
    let frozen = grpc_source::macro_queries::map_macro_error(wire.as_ref().unwrap_err());
    assert_eq!(frozen.provider(), None);
    review::restore_gateway_error(&review::store_gateway_error(&frozen)).unwrap();
}
