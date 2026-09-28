//! BR-165/BR-199 evidence-preserving CFFEX futures-delivery acquisition.

use super::{BatchEvidence, GatewayBatch, GatewayError};

use chrono::NaiveDate;
use serde::Deserialize;

use crate::grpc_client::envelope::{QueryAdmission, QueryResult};
use crate::market_domain::{ProviderId, SourceEvidence};

const CAPABILITY: &str = "R-08-cffex-planned-calendar";
const CONFIRMED_CAPABILITY: &str = "R-08-cffex-delivery";
const RECORD_SCHEMA: &str = "magic.market.futures_delivery_event";
const PLANNED_BATCH_PREFIX: &str = "cffex-equity-index-planned-delivery-2026-v2:";
const HOLIDAY_CALENDAR_URL: &str =
    "https://www.gov.cn/gongbao/2025/issue_12406/material/gwygb202532.pdf";
const PLANNED_2026_DAYS: [u32; 12] = [16, 24, 20, 17, 15, 22, 17, 21, 18, 16, 20, 18];
pub const FUTURES_DELIVERY_CONTRACT_UNAVAILABLE_V1: &str =
    "futures_delivery_contract_unavailable_v1";
pub const CONFIRMED_DELIVERY_AUTHORITY_UNAVAILABLE_V2: &str =
    "cffex_confirmed_delivery_authority_unavailable_v2";

/// The upstream v2 calendar is planned; the confirmed EventCalendar sink still
/// lacks month-specific exchange proof and must not send a factual reminder.
pub const fn cffex_futures_delivery_live_supported() -> bool {
    false
}

/// One admitted contract fact from an official CFFEX delivery notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuturesDeliveryFact {
    pub contract_code: String,
    pub product_code: String,
    pub last_trading_date: Option<NaiveDate>,
    pub delivery_date: NaiveDate,
    pub notice_url: String,
}

/// A conditional rule-derived date, never proof that delivery occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuturesDeliveryPlannedFact {
    pub contract_code: String,
    pub product_code: String,
    pub last_trading_date: NaiveDate,
    pub delivery_date: NaiveDate,
    pub rule_url: String,
    pub holiday_calendar_url: String,
}

/// The admitted upstream product is the revisioned 2026 monthly schedule.
/// Other years cannot inherit the 2026 source's complete coverage claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FuturesDeliveryRequest {
    year: u32,
    month: u32,
}

impl FuturesDeliveryRequest {
    pub fn new(year: u32, month: u32) -> Result<Self, GatewayError> {
        if !(2000..=9999).contains(&year) || !(1..=12).contains(&month) {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                format!("invalid requested CFFEX contract month {year:04}-{month:02}"),
            ));
        }
        if year != 2026 {
            return Err(GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Cffex),
                "unsupported",
                "cffex_delivery_year_not_covered_v2",
                false,
                format!("planned CFFEX delivery calendar covers 2026, requested {year}"),
            ));
        }
        Ok(Self { year, month })
    }

    pub(crate) fn params(self) -> serde_json::Value {
        serde_json::json!({"year": self.year, "month": self.month})
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlannedDeliveryWire {
    product: String,
    contract_code: String,
    last_trading_date: Option<NaiveDate>,
    delivery_date: NaiveDate,
    method: String,
    schedule_status: String,
    date_basis: String,
    rule_url: String,
    holiday_calendar_url: String,
    evidence: SourceEvidence,
}

fn expected_rule_url(product: &str) -> Option<(&'static str, &'static str)> {
    match product {
        "If" => Some(("IF", "https://www.cffex.com.cn/cn/hs300.html")),
        "Ih" => Some(("IH", "https://www.cffex.com.cn/cn/sz50gzqh.html")),
        "Ic" => Some(("IC", "https://www.cffex.com.cn/cn/zz500.html")),
        "Im" => Some(("IM", "https://www.cffex.com.cn/zz1000/")),
        _ => None,
    }
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::Cffex), message)
}

/// Validate a v2 planned calendar without interpreting zero rows as a verified
/// absence or upgrading a schedule to a confirmed delivery fact.
pub(crate) fn convert_response(
    request: FuturesDeliveryRequest,
    response: &QueryResult,
) -> Result<GatewayBatch<FuturesDeliveryPlannedFact>, GatewayError> {
    if response.admission != QueryAdmission::Admitted
        || !response.complete
        || !response.diagnostic_blocker.is_empty()
        || response.selected_provider != "Cffex"
        || !response.source().starts_with("grpc-mtls:")
        || !response.source_at.is_empty()
        || response.batch_id != format!("{PLANNED_BATCH_PREFIX}{:02}", request.month)
        || response.records.len() != 4
    {
        return Err(invalid(
            "CFFEX monthly batch is not admitted, complete and four-product",
        ));
    }
    let batch_at = super::evidence_time::parse_evidence_instant(
        CAPABILITY,
        ProviderId::Cffex,
        "batch observed_at",
        &response.observed_at,
    )?;
    let evidence = BatchEvidence {
        provider: ProviderId::Cffex,
        source: response.source().to_owned(),
        source_at: None,
        observed_at: response.observed_at.clone(),
        batch_id: response.batch_id.clone(),
    };
    let mut products = std::collections::HashSet::new();
    let expected_date = NaiveDate::from_ymd_opt(
        2026,
        request.month,
        PLANNED_2026_DAYS[request.month as usize - 1],
    )
    .expect("reviewed 2026 calendar date");
    let suffix = format!("{:02}{:02}", request.year % 100, request.month);
    let mut records = Vec::with_capacity(4);
    for payload in &response.records {
        if payload.schema != RECORD_SCHEMA
            || payload.schema_version != 2
            || payload.content_type != "application/json; charset=utf-8"
        {
            return Err(invalid(
                "CFFEX delivery record schema/version/content type mismatch",
            ));
        }
        let wire: PlannedDeliveryWire = serde_json::from_slice(&payload.data)
            .map_err(|error| invalid(format!("CFFEX delivery record invalid: {error}")))?;
        let (product_code, rule_url) = expected_rule_url(&wire.product)
            .ok_or_else(|| invalid("CFFEX delivery product is outside IF/IH/IC/IM"))?;
        if !products.insert(product_code)
            || wire.contract_code != format!("{product_code}{suffix}")
            || wire.delivery_date != expected_date
            || wire.last_trading_date != Some(wire.delivery_date)
            || wire.method != "Cash"
            || wire.schedule_status != "Planned"
            || wire.date_basis != "CffexRuleAndPublishedHolidays"
            || wire.rule_url != rule_url
            || wire.holiday_calendar_url != HOLIDAY_CALENDAR_URL
            || wire.evidence.provider() != ProviderId::Cffex
            || wire.evidence.source_at().is_some()
            || wire.evidence.batch_id() != response.batch_id
        {
            return Err(invalid(
                "CFFEX delivery scope, product or source evidence conflicts",
            ));
        }
        let record_at = super::evidence_time::parse_evidence_instant(
            CAPABILITY,
            ProviderId::Cffex,
            "record observed_at",
            wire.evidence.observed_at(),
        )?;
        if record_at != batch_at {
            return Err(invalid(
                "CFFEX delivery record observation differs from its batch",
            ));
        }
        records.push(FuturesDeliveryPlannedFact {
            contract_code: wire.contract_code,
            product_code: product_code.to_owned(),
            last_trading_date: wire.last_trading_date.expect("validated date"),
            delivery_date: wire.delivery_date,
            rule_url: wire.rule_url,
            holiday_calendar_url: wire.holiday_calendar_url,
        });
    }
    Ok(GatewayBatch::Available { records, evidence })
}

/// Separates the planned calendar from the unqualified confirmed-event sink.
#[derive(Debug, Clone, Copy, Default)]
pub struct FuturesDeliveryGateway;

impl FuturesDeliveryGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn cffex_contract_month(
        &self,
        year: u32,
        month: u32,
    ) -> Result<GatewayBatch<FuturesDeliveryFact>, GatewayError> {
        if !(2000..=9999).contains(&year) || !(1..=12).contains(&month) {
            return Err(GatewayError::invalid_request(
                CONFIRMED_CAPABILITY,
                format!("invalid requested CFFEX contract month {year:04}-{month:02}"),
            ));
        }
        Err(GatewayError::classified(
            CONFIRMED_CAPABILITY,
            Some(ProviderId::Cffex),
            "unavailable",
            CONFIRMED_DELIVERY_AUTHORITY_UNAVAILABLE_V2,
            false,
            "CFFEX v2 supplies a planned calendar, not a month-specific confirmed delivery notice",
        ))
    }

    /// Read-only planned calendar acquisition. Callers must label these dates
    /// as conditional and must not treat a row as a settlement receipt.
    pub async fn cffex_planned_contract_month(
        &self,
        year: u32,
        month: u32,
    ) -> Result<GatewayBatch<FuturesDeliveryPlannedFact>, GatewayError> {
        let request = FuturesDeliveryRequest::new(year, month)?;
        self.fetch_planned_source(request).await
    }

    async fn fetch_planned_source(
        &self,
        request: FuturesDeliveryRequest,
    ) -> Result<GatewayBatch<FuturesDeliveryPlannedFact>, GatewayError> {
        let request_hash = super::review::acquisition_request_hash(
            CAPABILITY,
            format!("ExternalV1/FuturesDelivery/{}", request.params()),
        );
        let result = match super::grpc_source::bridge_for("FuturesDelivery") {
            Ok(bridge) => bridge.futures_delivery_planned_2026_async(request).await,
            Err(error) => Err(error),
        };
        super::review::audit_routed_gateway_result(CAPABILITY, &request_hash, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseManager;
    use crate::grpc_client::client::external_query_wire_fixture::ExternalQueryWireFixture;
    use crate::grpc_client::envelope::{AcquisitionProvenance, CanonicalRecord};
    use diesel::{sql_types::BigInt, RunQueryDsl};
    use serial_test::serial;
    use std::time::Duration;

    #[derive(diesel::QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    fn audit_count() -> i64 {
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = 'R-08-cffex-planned-calendar'",
        )
        .get_result::<CountRow>(&mut *connection)
        .unwrap()
        .count
    }

    fn delivery_response() -> QueryResult {
        let records = ["If", "Ih", "Ic", "Im"]
            .into_iter()
            .map(|product| {
                let code = product.to_ascii_uppercase();
                let (_, rule_url) = expected_rule_url(product).unwrap();
                CanonicalRecord {
                    schema: RECORD_SCHEMA.to_owned(),
                    schema_version: 2,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: serde_json::to_vec(&serde_json::json!({
                        "product": product,
                        "contract_code": format!("{code}2609"),
                        "last_trading_date": "2026-09-18",
                        "delivery_date": "2026-09-18",
                        "method": "Cash",
                        "schedule_status": "Planned",
                        "date_basis": "CffexRuleAndPublishedHolidays",
                        "rule_url": rule_url,
                        "holiday_calendar_url": HOLIDAY_CALENDAR_URL,
                        "evidence": {
                            "provider": "Cffex",
                            "source_at": null,
                            "observed_at": "1790478510.469882800",
                            "batch_id": "cffex-equity-index-planned-delivery-2026-v2:09"
                        }
                    }))
                    .unwrap(),
                }
            })
            .collect();
        QueryResult {
            admission: QueryAdmission::Admitted,
            selected_provider: "Cffex".to_owned(),
            batch_id: "cffex-equity-index-planned-delivery-2026-v2:09".to_owned(),
            complete: true,
            observed_at: "1790478510.469882800".to_owned(),
            source_at: String::new(),
            records,
            provenance: AcquisitionProvenance::ExternalMtlsAuthority(
                "grpc-mtls:magic-market-data".to_owned(),
            ),
            diagnostic_blocker: String::new(),
        }
    }

    #[test]
    fn r08_v2_planned_monthly_fixture_requires_four_exact_cffex_products() {
        let request = FuturesDeliveryRequest::new(2026, 9).unwrap();
        let response = delivery_response();
        let batch = convert_response(request, &response).unwrap();
        assert_eq!(batch.records().len(), 4);
        assert_eq!(batch.evidence().provider, ProviderId::Cffex);
        assert_eq!(batch.records()[0].contract_code, "IF2609");
        assert_eq!(batch.records()[0].delivery_date.to_string(), "2026-09-18");
        assert_eq!(
            batch.records()[0].rule_url,
            "https://www.cffex.com.cn/cn/hs300.html"
        );

        let mut wrong_month = delivery_response();
        let mut record: serde_json::Value =
            serde_json::from_slice(&wrong_month.records[0].data).unwrap();
        record["delivery_date"] = serde_json::json!("2026-10-16");
        wrong_month.records[0].data = serde_json::to_vec(&record).unwrap();
        assert!(convert_response(request, &wrong_month).is_err());

        let mut duplicate = delivery_response();
        duplicate.records[3].data = duplicate.records[0].data.clone();
        assert!(convert_response(request, &duplicate).is_err());

        let mut legacy_v1 = delivery_response();
        legacy_v1.records[0].schema_version = 1;
        assert!(convert_response(request, &legacy_v1).is_err());

        let mut false_confirmation = delivery_response();
        let mut record: serde_json::Value =
            serde_json::from_slice(&false_confirmation.records[0].data).unwrap();
        record["schedule_status"] = serde_json::json!("Confirmed");
        false_confirmation.records[0].data = serde_json::to_vec(&record).unwrap();
        assert!(convert_response(request, &false_confirmation).is_err());

        let mut older_observation = delivery_response();
        let mut record: serde_json::Value =
            serde_json::from_slice(&older_observation.records[0].data).unwrap();
        record["evidence"]["observed_at"] = serde_json::json!("1790478510.469882799");
        older_observation.records[0].data = serde_json::to_vec(&record).unwrap();
        assert!(convert_response(request, &older_observation).is_err());
    }

    #[test]
    fn r08_2026_monthly_fixture_rejects_empty_partial_and_foreign_provider() {
        let request = FuturesDeliveryRequest::new(2026, 9).unwrap();
        let mut empty = delivery_response();
        empty.records.clear();
        assert!(convert_response(request, &empty).is_err());
        let mut partial = delivery_response();
        partial.complete = false;
        assert!(convert_response(request, &partial).is_err());
        let mut foreign = delivery_response();
        foreign.selected_provider = "Sina".to_owned();
        assert!(convert_response(request, &foreign).is_err());
        assert_eq!(
            FuturesDeliveryRequest::new(2027, 9)
                .unwrap_err()
                .reason_code(),
            "cffex_delivery_year_not_covered_v2"
        );
    }

    #[tokio::test]
    async fn futures_delivery_gateway_keeps_confirmed_sink_blocked_before_rpc() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).unwrap();
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();
        super::super::grpc_source::set_test_query_responses(vec![]);
        let before = audit_count();

        assert!(!cffex_futures_delivery_live_supported());
        let error = FuturesDeliveryGateway::new()
            .cffex_contract_month(2026, 9)
            .await
            .expect_err("planned v2 rows cannot prove confirmed delivery");
        assert_eq!(
            error.reason_code(),
            CONFIRMED_DELIVERY_AUTHORITY_UNAVAILABLE_V2
        );
        assert_eq!(error.capability(), CONFIRMED_CAPABILITY);
        assert_eq!(audit_count(), before);

        for month in [0, 13] {
            let error = FuturesDeliveryGateway::new()
                .cffex_contract_month(2026, month)
                .await
                .expect_err("invalid month must fail before RPC");
            assert_eq!(error.reason_code(), "invalid_request");
            assert_eq!(error.capability(), CONFIRMED_CAPABILITY);
        }
        let error = FuturesDeliveryGateway::new()
            .cffex_contract_month(2027, 9)
            .await
            .expect_err("no valid month has confirmed delivery authority");
        assert_eq!(
            error.reason_code(),
            CONFIRMED_DELIVERY_AUTHORITY_UNAVAILABLE_V2
        );
        assert_eq!(error.capability(), CONFIRMED_CAPABILITY);
        assert_eq!(audit_count(), before);

        super::super::grpc_source::set_test_query_responses(vec![Ok(delivery_response())]);
        let batch = FuturesDeliveryGateway::new()
            .cffex_planned_contract_month(2026, 9)
            .await
            .expect("planned route accepts its v2 wire fixture");
        assert_eq!(batch.records().len(), 4);
        assert_eq!(audit_count(), before + 1);
    }

    #[tokio::test]
    #[serial]
    async fn futures_delivery_gateway_uses_qualified_external_v2_wire_once() {
        let fixture = ExternalQueryWireFixture::bind_qualified_futures_delivery()
            .await
            .expect("qualified delivery fixture");
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("audit database init");
        std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", fixture.bundle_path());
        super::super::grpc_source::reset_bridge();
        fixture.release_capabilities();
        fixture.release();

        let batch = tokio::time::timeout(
            Duration::from_secs(20),
            FuturesDeliveryGateway::new().cffex_planned_contract_month(2026, 9),
        )
        .await
        .expect("delivery gateway deadline")
        .expect("qualified delivery batch");
        assert_eq!(batch.records().len(), 4);
        assert_eq!(batch.evidence().provider, ProviderId::Cffex);
        let observed = fixture.snapshot();
        assert_eq!(observed.capabilities_calls, 1);
        assert_eq!(observed.calls, 1);
        assert_eq!(observed.methods, ["futures_delivery"]);
        assert!(observed.unexpected_methods.is_empty());

        super::super::grpc_source::reset_bridge();
        fixture.finish().await.expect("delivery fixture cleanup");
    }
}
