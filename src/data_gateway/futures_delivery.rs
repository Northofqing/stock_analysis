//! BR-165/BR-199 evidence-preserving CFFEX futures-delivery acquisition.

use super::{BatchEvidence, GatewayBatch, GatewayError};

use chrono::{Datelike, NaiveDate};
use serde::Deserialize;

use crate::grpc_client::envelope::{QueryAdmission, QueryResult};
use crate::market_domain::{ProviderId, SourceEvidence};

const CAPABILITY: &str = "R-08-cffex-delivery";
const RECORD_SCHEMA: &str = "magic.market.futures_delivery_event";
const NOTICE_URL: &str = "https://www.cffex.com.cn/jystz/20251217/46425.html";
pub const FUTURES_DELIVERY_CONTRACT_UNAVAILABLE_V1: &str =
    "futures_delivery_contract_unavailable_v1";

/// no-feature (monitor 零 magic): 进程内无 CffexClient, 契约无从读取。
/// 诚实声明 = false → 启动 banner 走 warn 分支 (出声, 与 remote gRPC
/// 下 gRPC 通道独立承载 R-08 交付不冲突)。

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
                "cffex_delivery_year_not_covered_v1",
                false,
                format!("formal CFFEX delivery schedule covers 2026, requested {year}"),
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
struct DeliveryWire {
    product: String,
    contract_code: String,
    last_trading_date: Option<NaiveDate>,
    delivery_date: NaiveDate,
    method: String,
    notice_url: String,
    evidence: SourceEvidence,
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::Cffex), message)
}

/// Validate one ExternalV1 monthly batch without interpreting zero rows as a
/// verified absence. The current admitted 2026 source always has four products.
pub(crate) fn convert_response(
    request: FuturesDeliveryRequest,
    response: &QueryResult,
) -> Result<GatewayBatch<FuturesDeliveryFact>, GatewayError> {
    if response.admission != QueryAdmission::Admitted
        || !response.complete
        || !response.diagnostic_blocker.is_empty()
        || response.selected_provider != "Cffex"
        || !response.source().starts_with("grpc-mtls:")
        || !response.source_at.is_empty()
        || response.batch_id.trim().is_empty()
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
    let mut delivery_date = None;
    let suffix = format!("{:02}{:02}", request.year % 100, request.month);
    let mut records = Vec::with_capacity(4);
    for payload in &response.records {
        if payload.schema != RECORD_SCHEMA
            || payload.schema_version != 1
            || payload.content_type != "application/json; charset=utf-8"
        {
            return Err(invalid(
                "CFFEX delivery record schema/version/content type mismatch",
            ));
        }
        let wire: DeliveryWire = serde_json::from_slice(&payload.data)
            .map_err(|error| invalid(format!("CFFEX delivery record invalid: {error}")))?;
        let product_code = match wire.product.as_str() {
            "If" => "IF",
            "Ih" => "IH",
            "Ic" => "IC",
            "Im" => "IM",
            _ => return Err(invalid("CFFEX delivery product is outside IF/IH/IC/IM")),
        };
        if !products.insert(product_code)
            || wire.contract_code != format!("{product_code}{suffix}")
            || wire.delivery_date.year() != request.year as i32
            || wire.delivery_date.month() != request.month
            || delivery_date.is_some_and(|date| date != wire.delivery_date)
            || wire.last_trading_date != Some(wire.delivery_date)
            || wire.method != "Cash"
            || wire.notice_url != NOTICE_URL
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
        if record_at > batch_at {
            return Err(invalid("CFFEX delivery record observed after its batch"));
        }
        delivery_date = Some(wire.delivery_date);
        records.push(FuturesDeliveryFact {
            contract_code: wire.contract_code,
            product_code: product_code.to_owned(),
            last_trading_date: wire.last_trading_date,
            delivery_date: wire.delivery_date,
            notice_url: wire.notice_url,
        });
    }
    Ok(GatewayBatch::Available { records, evidence })
}

/// Production seam for the unified CFFEX official-notice provider.
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
                CAPABILITY,
                format!("invalid requested CFFEX contract month {year:04}-{month:02}"),
            ));
        }
        Err(GatewayError::classified(
            CAPABILITY,
            None,
            "unavailable",
            FUTURES_DELIVERY_CONTRACT_UNAVAILABLE_V1,
            false,
            format!(
                "CFFEX requested month {year:04}-{month:02} is blocked before business RPC: \
                 FuturesDeliveryRequest v1 scope, coverage and verified-empty semantics are not delivered"
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseManager;
    use crate::grpc_client::envelope::{AcquisitionProvenance, CanonicalRecord};
    use diesel::{sql_types::BigInt, RunQueryDsl};

    #[derive(diesel::QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    fn audit_count() -> i64 {
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = 'R-08-cffex-delivery'",
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
                CanonicalRecord {
                    schema: RECORD_SCHEMA.to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: serde_json::to_vec(&serde_json::json!({
                        "product": product,
                        "contract_code": format!("{code}2609"),
                        "last_trading_date": "2026-09-18",
                        "delivery_date": "2026-09-18",
                        "method": "Cash",
                        "notice_url": NOTICE_URL,
                        "evidence": {
                            "provider": "Cffex",
                            "source_at": null,
                            "observed_at": "2026-09-27T08:00:00+08:00",
                            "batch_id": "cffex-equity-index-delivery-2026-v1:09"
                        }
                    }))
                    .unwrap(),
                }
            })
            .collect();
        QueryResult {
            admission: QueryAdmission::Admitted,
            selected_provider: "Cffex".to_owned(),
            batch_id: "cffex-equity-index-delivery-2026-v1:09".to_owned(),
            complete: true,
            observed_at: "2026-09-27T08:00:00+08:00".to_owned(),
            source_at: String::new(),
            records,
            provenance: AcquisitionProvenance::ExternalMtlsAuthority(
                "grpc-mtls:magic-market-data".to_owned(),
            ),
            diagnostic_blocker: String::new(),
        }
    }

    #[test]
    fn r08_2026_monthly_fixture_requires_four_exact_cffex_products() {
        let request = FuturesDeliveryRequest::new(2026, 9).unwrap();
        let response = delivery_response();
        let batch = convert_response(request, &response).unwrap();
        assert_eq!(batch.records().len(), 4);
        assert_eq!(batch.evidence().provider, ProviderId::Cffex);
        assert_eq!(batch.records()[0].contract_code, "IF2609");
        assert_eq!(batch.records()[0].delivery_date.to_string(), "2026-09-18");

        let mut wrong_month = delivery_response();
        let mut record: serde_json::Value =
            serde_json::from_slice(&wrong_month.records[0].data).unwrap();
        record["delivery_date"] = serde_json::json!("2026-10-16");
        wrong_month.records[0].data = serde_json::to_vec(&record).unwrap();
        assert!(convert_response(request, &wrong_month).is_err());

        let mut duplicate = delivery_response();
        duplicate.records[3].data = duplicate.records[0].data.clone();
        assert!(convert_response(request, &duplicate).is_err());
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
            "cffex_delivery_year_not_covered_v1"
        );
    }

    #[tokio::test]
    async fn task9_futures_delivery_contract_unavailable_has_zero_rpc_and_zero_success_audit() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).unwrap();
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();
        // Any attempted physical query panics because the queue is empty.
        super::super::grpc_source::set_test_query_responses(vec![]);
        let before = audit_count();

        let error = FuturesDeliveryGateway::new()
            .cffex_contract_month(2026, 9)
            .await
            .expect_err("missing request/coverage contract must fail closed");
        assert_eq!(
            error.reason_code(),
            "futures_delivery_contract_unavailable_v1"
        );
        assert!(error.message().contains("2026-09"));
        assert_eq!(
            audit_count(),
            before,
            "no success/failure RPC audit is legal"
        );

        for month in [0, 13] {
            let error = FuturesDeliveryGateway::new()
                .cffex_contract_month(2026, month)
                .await
                .expect_err("invalid month must fail before RPC");
            assert_eq!(error.reason_code(), "invalid_request");
        }
        assert_eq!(audit_count(), before);
    }
}
