//! BR-165/BR-199 evidence-preserving CFFEX futures-delivery acquisition.

use super::review::{acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result};
use super::{BatchEvidence, GatewayBatch, GatewayError};
use chrono::{Datelike, NaiveDate};
use magic_exchange_rs::{CffexClient, ExchangeError};
use magic_market_core::{
    DataBatch, FuturesDeliveryCalendar, FuturesDeliveryEvent, FuturesDeliveryMethod,
    FuturesDeliveryRequest, FuturesProduct, PositiveU32, ProviderId,
};
use std::collections::HashSet;

const CAPABILITY: &str = "R-08-cffex-delivery";
const SOURCE: &str = "cffex-official-notice";

/// Truthful production capability advertised by the pinned CFFEX provider.
///
/// This is deliberately a contract read, not a network probe: startup may
/// report availability without creating a provider or fabricating readiness.
pub const fn cffex_futures_delivery_live_supported() -> bool {
    CffexClient::calendar_capabilities().futures_delivery
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
        let request_hash = acquisition_request_hash(CAPABILITY, &format!("{year:04}-{month:02}"));
        // P4 M3 钩子: DATA_GATEWAY_GRPC=1 → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("FuturesDelivery") {
            Ok(Some(bridge)) => {
                let result = bridge.futures_delivery_async().await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Cffex);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(CAPABILITY, ProviderId::Cffex, &request_hash, Err(error));
            }
        }
        let worker_request_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = build_request(year, month).and_then(fetch_and_admit_cffex_batch);
            audit_gateway_result(CAPABILITY, ProviderId::Cffex, &worker_request_hash, result)
        })
        .await;

        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    CAPABILITY,
                    ProviderId::Cffex,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }
}

fn build_request(year: u32, month: u32) -> Result<FuturesDeliveryRequest, GatewayError> {
    let year = PositiveU32::new(year).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid CFFEX year: {error}"))
    })?;
    let month = PositiveU32::new(month).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid CFFEX month: {error}"))
    })?;
    FuturesDeliveryRequest::new(year, month).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid CFFEX contract month: {error}"))
    })
}

fn fetch_and_admit_cffex_batch(
    request: FuturesDeliveryRequest,
) -> Result<GatewayBatch<FuturesDeliveryFact>, GatewayError> {
    let client = CffexClient::new().map_err(cffex_gateway_error)?;
    let batch = client
        .futures_delivery_calendar(&request)
        .map_err(cffex_gateway_error)?;
    admit_cffex_batch(batch, &request)
}

fn admit_cffex_batch(
    batch: DataBatch<FuturesDeliveryEvent>,
    request: &FuturesDeliveryRequest,
) -> Result<GatewayBatch<FuturesDeliveryFact>, GatewayError> {
    if batch.provenance().source() != SOURCE || !batch.quality().is_complete() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cffex),
            "CFFEX delivery batch is not a complete official-notice batch",
        ));
    }
    let evidence = BatchEvidence::from_provenance(ProviderId::Cffex, batch.provenance())?;
    if batch.records().len() != 4 {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cffex),
            format!(
                "CFFEX delivery batch must contain IF/IH/IC/IM exactly once, got {} records",
                batch.records().len()
            ),
        ));
    }

    let expected_suffix = format!(
        "{:02}{:02}",
        request.year().get() % 100,
        request.month().get()
    );
    let expected_year = i32::try_from(request.year().get()).map_err(|error| {
        GatewayError::invalid_request(
            CAPABILITY,
            format!("invalid CFFEX year conversion: {error}"),
        )
    })?;
    let mut seen_products = HashSet::with_capacity(4);
    let mut common_delivery_date: Option<NaiveDate> = None;
    let mut records = Vec::with_capacity(4);

    for record in batch.into_records() {
        let product_code = product_code(record.product);
        if !seen_products.insert(product_code) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cffex),
                format!("duplicate CFFEX product {product_code}"),
            ));
        }
        let expected_contract = format!("{product_code}{expected_suffix}");
        if record.contract_code.as_str() != expected_contract {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cffex),
                format!(
                    "CFFEX contract {} differs from requested {expected_contract}",
                    record.contract_code.as_str()
                ),
            ));
        }
        if record.method != FuturesDeliveryMethod::NotProvided {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cffex),
                format!(
                    "CFFEX contract {expected_contract} claims a settlement method that the official notice does not prove"
                ),
            ));
        }

        let delivery_date = parse_date(record.delivery_date.as_str(), "delivery")?;
        if delivery_date.year() != expected_year || delivery_date.month() != request.month().get() {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cffex),
                format!(
                    "CFFEX contract {expected_contract} date is inconsistent with the requested month"
                ),
            ));
        }
        let last_trading_date = record
            .last_trading_date
            .as_ref()
            .map(|date| parse_date(date.as_str(), "last trading"))
            .transpose()?;
        if last_trading_date.is_some_and(|date| {
            date.year() != delivery_date.year()
                || date.month() != delivery_date.month()
                || date > delivery_date
        }) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cffex),
                format!(
                    "CFFEX contract {expected_contract} last-trading date is outside the requested month or after delivery"
                ),
            ));
        }
        if common_delivery_date.is_some_and(|date| date != delivery_date) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cffex),
                "CFFEX contracts disagree on the official delivery date",
            ));
        }
        common_delivery_date = Some(delivery_date);

        if record.evidence.provider() != ProviderId::Cffex
            || record.evidence.batch_id() != evidence.batch_id
            || record.evidence.observed_at() != evidence.observed_at
            || record.evidence.source_at() != evidence.source_at.as_deref()
        {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cffex),
                format!("CFFEX contract {expected_contract} evidence differs from its batch"),
            ));
        }
        records.push(FuturesDeliveryFact {
            contract_code: expected_contract,
            product_code: product_code.to_string(),
            last_trading_date,
            delivery_date,
            notice_url: record.notice_url.as_str().to_string(),
        });
    }

    common_delivery_date.ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cffex),
            "CFFEX delivery batch has no official delivery date",
        )
    })?;
    let publication_date = evidence.source_at.as_deref().ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cffex),
            "CFFEX batch has no official notice publication date",
        )
    })?;
    parse_date(publication_date, "notice publication")?;

    records.sort_by(|left, right| left.contract_code.cmp(&right.contract_code));
    Ok(GatewayBatch::Available { records, evidence })
}

fn product_code(product: FuturesProduct) -> &'static str {
    match product {
        FuturesProduct::If => "IF",
        FuturesProduct::Ih => "IH",
        FuturesProduct::Ic => "IC",
        FuturesProduct::Im => "IM",
    }
}

fn parse_date(value: &str, field: &str) -> Result<NaiveDate, GatewayError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cffex),
            format!("invalid CFFEX {field} date {value:?}: {error}"),
        )
    })
}

fn cffex_gateway_error(error: ExchangeError) -> GatewayError {
    let message = error.to_string();
    match error {
        ExchangeError::InvalidRequest(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Cffex),
            "invalid_request",
            "provider_invalid_request",
            false,
            message,
        ),
        ExchangeError::Unsupported(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Cffex),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        ExchangeError::Authentication(_)
        | ExchangeError::RateLimited
        | ExchangeError::Transport(_)
        | ExchangeError::Tls { .. }
        | ExchangeError::HttpStatus(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Cffex),
            "unavailable",
            "provider_transport",
            true,
            message,
        ),
        ExchangeError::Incomplete(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Cffex),
            "unavailable",
            "official_notice_unavailable",
            true,
            message,
        ),
        ExchangeError::Decode(_) | ExchangeError::Schema(_) | ExchangeError::Core(_) => {
            GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Cffex),
                "partial",
                "provider_batch_rejected",
                false,
                message,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magic_exchange_rs::TlsBackend;
    use magic_market_core::{HttpsUrl, IsoDate, NonEmptyText, Provenance, SourceEvidence};

    fn request() -> FuturesDeliveryRequest {
        build_request(2026, 7).expect("TEST_CODE request")
    }

    fn batch() -> DataBatch<FuturesDeliveryEvent> {
        let observed_at = "1784822400.000000000";
        let delivery_date = "2026-07-17";
        let publication_date = "2026-07-16";
        let batch_id = "TEST_CODE_cffex_2607";
        let records = [
            (FuturesProduct::If, "IF2607"),
            (FuturesProduct::Ih, "IH2607"),
            (FuturesProduct::Ic, "IC2607"),
            (FuturesProduct::Im, "IM2607"),
        ]
        .into_iter()
        .map(|(product, contract_code)| FuturesDeliveryEvent {
            product,
            contract_code: NonEmptyText::new(contract_code).expect("contract"),
            last_trading_date: None,
            delivery_date: IsoDate::new(delivery_date).expect("delivery"),
            method: FuturesDeliveryMethod::NotProvided,
            notice_url: HttpsUrl::new("https://www.cffex.com.cn/jystz/20260717/45321.html")
                .expect("notice URL"),
            evidence: SourceEvidence::new(ProviderId::Cffex, observed_at, batch_id)
                .expect("evidence")
                .with_source_at(publication_date)
                .expect("source date"),
        })
        .collect();
        let provenance = Provenance::new(SOURCE, observed_at)
            .expect("provenance")
            .with_source_at(publication_date)
            .expect("source date")
            .with_batch_id(batch_id)
            .expect("batch id");
        DataBatch::strict(records, provenance)
    }

    #[test]
    fn br165_admits_and_stably_orders_complete_cffex_batch() {
        let admitted = admit_cffex_batch(batch(), &request()).expect("admitted CFFEX batch");
        let contracts: Vec<_> = admitted
            .records()
            .iter()
            .map(|record| record.contract_code.as_str())
            .collect();
        assert_eq!(contracts, ["IC2607", "IF2607", "IH2607", "IM2607"]);
        assert_eq!(
            admitted.records()[0].delivery_date.to_string(),
            "2026-07-17"
        );
        assert_eq!(admitted.evidence().provider, ProviderId::Cffex);
        assert_eq!(admitted.evidence().source_at.as_deref(), Some("2026-07-16"));
        assert_eq!(admitted.records()[0].last_trading_date, None);
    }

    #[test]
    fn br165_rejects_missing_required_product() {
        let original = batch();
        let records = original.into_records().into_iter().take(3).collect();
        let provenance = Provenance::new(SOURCE, "1784822400.000000000")
            .unwrap()
            .with_source_at("2026-07-16")
            .unwrap()
            .with_batch_id("TEST_CODE_cffex_2607")
            .unwrap();
        let error = admit_cffex_batch(DataBatch::strict(records, provenance), &request())
            .expect_err("partial CFFEX batch must be rejected");
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br165_rejects_contract_month_mismatch() {
        let error = admit_cffex_batch(batch(), &build_request(2026, 8).unwrap())
            .expect_err("wrong request month must be rejected");
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br165_classifies_missing_notice_as_retryable_unavailable() {
        let error = cffex_gateway_error(ExchangeError::Incomplete(
            "TEST_CODE official notice absent".to_string(),
        ));
        assert_eq!(error.audit_outcome(), "unavailable");
        assert_eq!(error.reason_code(), "official_notice_unavailable");
        assert!(error.retryable());
    }

    #[test]
    fn br165_formal_provider_contract_owns_live_admission() {
        assert!(!cffex_futures_delivery_live_supported());
        let error = fetch_and_admit_cffex_batch(request())
            .expect_err("unadmitted production capability must fail before network I/O");
        assert_eq!(error.audit_outcome(), "unsupported");
        assert_eq!(error.reason_code(), "provider_unsupported");
        assert!(!error.retryable());
    }

    fn rebuilt_batch(
        records: Vec<FuturesDeliveryEvent>,
        source: &str,
        source_at: Option<&str>,
        issues: Vec<String>,
    ) -> DataBatch<FuturesDeliveryEvent> {
        let mut provenance = Provenance::new(source, "1784822400.000000000").unwrap();
        if let Some(source_at) = source_at {
            provenance = provenance.with_source_at(source_at).unwrap();
        }
        provenance = provenance.with_batch_id("TEST_CODE_cffex_2607").unwrap();
        if issues.is_empty() {
            DataBatch::strict(records, provenance)
        } else {
            DataBatch::best_effort(records, provenance, issues).unwrap()
        }
    }

    #[test]
    fn br165_request_and_batch_envelope_validation_fail_closed() {
        assert!(build_request(0, 7).is_err());
        assert!(build_request(2026, 0).is_err());
        assert!(build_request(2026, 13).is_err());

        let records = batch().into_records();
        assert!(admit_cffex_batch(
            rebuilt_batch(
                records.clone(),
                "TEST_CODE_wrong_source",
                Some("2026-07-16"),
                Vec::new(),
            ),
            &request(),
        )
        .is_err());
        assert!(admit_cffex_batch(
            rebuilt_batch(
                records,
                SOURCE,
                Some("2026-07-16"),
                vec!["TEST_CODE incomplete official notice".to_string()],
            ),
            &request(),
        )
        .is_err());
    }

    #[test]
    fn br165_product_settlement_date_and_evidence_must_match_notice() {
        let mut duplicate = batch().into_records();
        duplicate[1].product = duplicate[0].product;
        duplicate[1].contract_code = NonEmptyText::new("IF2607").unwrap();
        assert!(admit_cffex_batch(
            rebuilt_batch(duplicate, SOURCE, Some("2026-07-16"), Vec::new()),
            &request(),
        )
        .is_err());

        let mut fabricated_method = batch().into_records();
        fabricated_method[0].method = FuturesDeliveryMethod::Cash;
        assert!(admit_cffex_batch(
            rebuilt_batch(fabricated_method, SOURCE, Some("2026-07-16"), Vec::new(),),
            &request(),
        )
        .is_err());

        let mut date_mismatch = batch().into_records();
        date_mismatch[0].last_trading_date = Some(IsoDate::new("2026-08-01").unwrap());
        assert!(admit_cffex_batch(
            rebuilt_batch(date_mismatch, SOURCE, Some("2026-07-16"), Vec::new()),
            &request(),
        )
        .is_err());

        let mut disagreement = batch().into_records();
        disagreement[3].delivery_date = IsoDate::new("2026-07-18").unwrap();
        disagreement[3].evidence = SourceEvidence::new(
            ProviderId::Cffex,
            "1784822400.000000000",
            "TEST_CODE_cffex_2607",
        )
        .unwrap()
        .with_source_at("2026-07-16")
        .unwrap();
        assert!(admit_cffex_batch(
            rebuilt_batch(disagreement, SOURCE, Some("2026-07-16"), Vec::new()),
            &request(),
        )
        .is_err());

        let mut wrong_evidence = batch().into_records();
        wrong_evidence[0].evidence = SourceEvidence::new(
            ProviderId::Cffex,
            "1784822400.000000000",
            "TEST_CODE_other_batch",
        )
        .unwrap()
        .with_source_at("2026-07-16")
        .unwrap();
        assert!(admit_cffex_batch(
            rebuilt_batch(wrong_evidence, SOURCE, Some("2026-07-16"), Vec::new()),
            &request(),
        )
        .is_err());

        assert!(admit_cffex_batch(
            rebuilt_batch(batch().into_records(), SOURCE, None, Vec::new()),
            &request(),
        )
        .is_err());
        assert!(parse_date("TEST_CODE_bad_date", "TEST_CODE").is_err());
        assert_eq!(product_code(FuturesProduct::If), "IF");
        assert_eq!(product_code(FuturesProduct::Ih), "IH");
        assert_eq!(product_code(FuturesProduct::Ic), "IC");
        assert_eq!(product_code(FuturesProduct::Im), "IM");
    }

    #[test]
    fn br165_exchange_error_mapping_is_stable_for_operations() {
        let cases = [
            cffex_gateway_error(ExchangeError::InvalidRequest("TEST_CODE".into())),
            cffex_gateway_error(ExchangeError::Unsupported("TEST_CODE".into())),
            cffex_gateway_error(ExchangeError::Authentication(403)),
            cffex_gateway_error(ExchangeError::RateLimited),
            cffex_gateway_error(ExchangeError::Transport("TEST_CODE".into())),
            cffex_gateway_error(ExchangeError::Tls {
                backend: TlsBackend::Rustls,
                message: "TEST_CODE".into(),
            }),
            cffex_gateway_error(ExchangeError::HttpStatus(503)),
            cffex_gateway_error(ExchangeError::Decode("TEST_CODE".into())),
            cffex_gateway_error(ExchangeError::Schema("TEST_CODE".into())),
        ];
        assert_eq!(cases[0].audit_outcome(), "invalid_request");
        assert_eq!(cases[1].audit_outcome(), "unsupported");
        for error in &cases[2..7] {
            assert_eq!(error.audit_outcome(), "unavailable");
            assert!(error.retryable());
        }
        for error in &cases[7..] {
            assert_eq!(error.audit_outcome(), "partial");
            assert!(!error.retryable());
        }
    }
}
