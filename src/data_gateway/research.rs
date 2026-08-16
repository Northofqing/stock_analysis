//! BR-119/BR-164 evidence-preserving research-report acquisition Gateway.

use super::review::{acquisition_request_hash, audit_gateway_result};
#[cfg(feature = "magic-gateway")]
use super::review::audit_blocking_join_failure;
use super::{GatewayBatch, GatewayError};
#[cfg(feature = "magic-gateway")]
use super::BatchEvidence;
#[cfg(feature = "magic-gateway")]
use magic_eastmoney_rs::{EastmoneyClient, EastmoneyError};
use crate::magic_compat::ProviderId;
#[cfg(feature = "magic-gateway")]
use crate::magic_compat::{DataBatch, PositiveU32};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{ReportScope, ResearchReport, ResearchReports, ResearchRequest};

const CAPABILITY: &str = "research-reports";

#[derive(Debug, Clone, PartialEq)]
pub struct ResearchReportFact {
    pub report_id: String,
    pub title: String,
    pub organization: String,
    pub organization_id: Option<String>,
    pub author: Option<String>,
    pub rating: Option<String>,
    pub industry_code: Option<String>,
    pub industry_name: Option<String>,
    pub published_at: String,
    pub canonical_url: String,
    pub pdf_url: Option<String>,
    pub source_target_price_upper: Option<f64>,
    pub source_target_price_lower: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ResearchDataGateway;

impl ResearchDataGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn instrument_reports(
        &self,
        code: &str,
        page_size: u32,
    ) -> Result<GatewayBatch<ResearchReportFact>, GatewayError> {
        let code = validate_code(code)?.to_owned();
        let request_hash = acquisition_request_hash(CAPABILITY, &format!("{code}:1:{page_size}"));
        // P4 M4b: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("ResearchReports") {
            Ok(Some(bridge)) => {
                let result = bridge.research_reports_async(&code, page_size).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Eastmoney);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Eastmoney,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Eastmoney),
                "unavailable",
                "provider_transport",
                true,
                "library transport disabled: DATA_GATEWAY_GRPC=1 required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let worker_request_hash = request_hash.clone();
            let joined = tokio::task::spawn_blocking(move || {
                let result = build_request(&code, page_size).and_then(fetch_reports);
                audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Eastmoney,
                    &worker_request_hash,
                    result,
                )
            })
            .await;

            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_blocking_join_failure(
                        CAPABILITY,
                        ProviderId::Eastmoney,
                        request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
    }
}

#[cfg(feature = "magic-gateway")]
fn build_request(code: &str, page_size: u32) -> Result<ResearchRequest, GatewayError> {
    let instrument = a_share_instrument(code)?;
    let page = PositiveU32::new(1)
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    let page_size = PositiveU32::new(page_size)
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    ResearchRequest::new(ReportScope::Instrument(instrument), page, page_size)
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))
}

#[cfg(feature = "magic-gateway")]
fn fetch_reports(
    request: ResearchRequest,
) -> Result<GatewayBatch<ResearchReportFact>, GatewayError> {
    let provider = EastmoneyClient::new().map_err(eastmoney_gateway_error)?;
    let batch = provider
        .research_reports(&request)
        .map_err(eastmoney_gateway_error)?;
    normalize_reports_batch(batch)
}

#[cfg(feature = "magic-gateway")]
fn normalize_reports_batch(
    batch: DataBatch<ResearchReport>,
) -> Result<GatewayBatch<ResearchReportFact>, GatewayError> {
    if !batch.quality().is_complete() {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "partial",
            "provider_partial_batch",
            false,
            format!("quality issues: {:?}", batch.quality().issues()),
        ));
    }
    let evidence = BatchEvidence::from_provenance(ProviderId::Eastmoney, batch.provenance())?;
    if batch.records().is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }
    let records = batch
        .records()
        .iter()
        .map(|record| {
            validate_record_evidence(&record.evidence, &evidence)?;
            Ok(ResearchReportFact {
                report_id: record.report_id.as_str().to_owned(),
                title: record.title.as_str().to_owned(),
                organization: record.organization.as_str().to_owned(),
                organization_id: record
                    .organization_id
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                author: record
                    .author
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                rating: record
                    .rating
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                industry_code: record
                    .industry_code
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                industry_name: record
                    .industry_name
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                published_at: record.published_at.as_str().to_owned(),
                canonical_url: record.canonical_url.as_str().to_owned(),
                pdf_url: record
                    .pdf_url
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                source_target_price_upper: record.source_indv_aim_price_t.map(|value| value.get()),
                source_target_price_lower: record.source_indv_aim_price_l.map(|value| value.get()),
            })
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    Ok(GatewayBatch::Available { records, evidence })
}

#[cfg(feature = "magic-gateway")]
fn validate_record_evidence(
    record: &crate::magic_compat::SourceEvidence,
    batch: &BatchEvidence,
) -> Result<(), GatewayError> {
    if record.provider() != batch.provider
        || record.batch_id() != batch.batch_id
        || record.observed_at() != batch.observed_at
    {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "research record evidence differs from batch evidence",
        ));
    }
    Ok(())
}

fn validate_code(code: &str) -> Result<&str, GatewayError> {
    a_share_instrument(code)?;
    Ok(code)
}

fn a_share_instrument(code: &str) -> Result<crate::magic_compat::InstrumentId, GatewayError> {
    #[cfg(test)]
    let resolved = super::instrument_identity::resolve_test_equity(code, None);
    #[cfg(not(test))]
    let resolved = super::instrument_identity::resolve_production_equity(code, None);
    let identity =
        resolved.map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    identity
        .require_a_share()
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    Ok(identity.instrument().clone())
}

#[cfg(feature = "magic-gateway")]
fn eastmoney_gateway_error(error: EastmoneyError) -> GatewayError {
    let message = error.to_string();
    match error {
        EastmoneyError::InvalidRequest(_) => GatewayError::invalid_request(CAPABILITY, message),
        EastmoneyError::Unsupported(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        EastmoneyError::Transport(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unavailable",
            "provider_transport",
            true,
            message,
        ),
        EastmoneyError::VerifiedEmpty(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "verified_empty",
            "verified_empty",
            false,
            message,
        ),
        EastmoneyError::ResponseTooLarge { .. }
        | EastmoneyError::Decode(_)
        | EastmoneyError::Protocol(_)
        | EastmoneyError::Core(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unavailable",
            "provider_invalid_batch",
            false,
            message,
        ),
    }
}

#[cfg(test)]
#[cfg(feature = "magic-gateway")]
mod tests {
    use super::{build_request, eastmoney_gateway_error, normalize_reports_batch};
    use magic_eastmoney_rs::EastmoneyError;
    use crate::magic_compat::{AssetClass, DataBatch, Exchange, InstrumentId, NonEmptyText, Price, Provenance, ProviderId, SourceEvidence};
use magic_market_core::{HttpsUrl, ReportScope, ResearchReport};

    const OBSERVED_AT: &str = "1784965800.000000000";
    const BATCH_ID: &str = "TEST_CODE_research_batch";

    fn provenance(batch_id: &str) -> Provenance {
        Provenance::new("TEST_CODE_eastmoney-research", OBSERVED_AT)
            .unwrap()
            .with_source_at("2026-07-25T09:30:00+08:00")
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap()
    }

    fn report(record_batch_id: &str) -> ResearchReport {
        let instrument =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600000", AssetClass::Equity).unwrap();
        ResearchReport {
            report_id: NonEmptyText::new("TEST_CODE_REPORT_1").unwrap(),
            scope: ReportScope::Instrument(instrument),
            title: NonEmptyText::new("TEST_CODE 盈利预测上调").unwrap(),
            organization: NonEmptyText::new("TEST_CODE 研究所").unwrap(),
            organization_id: Some(NonEmptyText::new("TEST_CODE_ORG_1").unwrap()),
            author: Some(NonEmptyText::new("测试分析师").unwrap()),
            rating: Some(NonEmptyText::new("增持").unwrap()),
            industry_code: Some(NonEmptyText::new("TEST_CODE_I01").unwrap()),
            industry_name: Some(NonEmptyText::new("测试行业").unwrap()),
            published_at: NonEmptyText::new("2026-07-25T09:30:00+08:00").unwrap(),
            canonical_url: HttpsUrl::new("https://example.com/TEST_CODE/report").unwrap(),
            pdf_url: Some(HttpsUrl::new("https://example.com/TEST_CODE/report.pdf").unwrap()),
            estimates: Vec::new(),
            source_indv_aim_price_t: Some(Price::new(14.0).unwrap()),
            source_indv_aim_price_l: Some(Price::new(12.0).unwrap()),
            evidence: SourceEvidence::new(ProviderId::Eastmoney, OBSERVED_AT, record_batch_id)
                .unwrap(),
        }
    }

    #[test]
    fn validates_supported_a_share_identity() {
        for (code, exchange) in [
            ("TEST_CODE_600000", Exchange::Shanghai),
            ("TEST_CODE_000001", Exchange::Shenzhen),
            ("TEST_CODE_920001", Exchange::Beijing),
        ] {
            let request = build_request(code, 100).unwrap();
            let ReportScope::Instrument(instrument) = request.scope() else {
                panic!("instrument scope required");
            };
            assert_eq!(instrument.exchange(), exchange);
            assert_eq!(instrument.code(), code);
        }
        for code in [
            "TEST_CODE_430047",
            "TEST_CODE_830001",
            "TEST_CODE_200001",
            "TEST_CODE_900901",
            "TEST_CODE_100001",
            "TEST_CODE_60000A",
        ] {
            assert!(build_request(code, 100).is_err(), "{code}");
        }
    }

    #[test]
    fn request_bounds_are_explicit() {
        let request = build_request("TEST_CODE_600000", 100).unwrap();
        assert_eq!(request.page().get(), 1);
        assert_eq!(request.page_size().get(), 100);
        assert!(build_request("TEST_CODE_600000", 0).is_err());
        assert!(build_request("TEST_CODE_600000", 101).is_err());
    }

    #[test]
    fn complete_report_batch_preserves_optional_fields_and_evidence() {
        let batch = DataBatch::strict(vec![report(BATCH_ID)], provenance(BATCH_ID));
        let admitted = normalize_reports_batch(batch).unwrap();
        let facts = admitted.records();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].report_id, "TEST_CODE_REPORT_1");
        assert_eq!(facts[0].title, "TEST_CODE 盈利预测上调");
        assert_eq!(facts[0].organization_id.as_deref(), Some("TEST_CODE_ORG_1"));
        assert_eq!(facts[0].author.as_deref(), Some("测试分析师"));
        assert_eq!(facts[0].rating.as_deref(), Some("增持"));
        assert_eq!(facts[0].industry_code.as_deref(), Some("TEST_CODE_I01"));
        assert_eq!(facts[0].industry_name.as_deref(), Some("测试行业"));
        assert_eq!(
            facts[0].pdf_url.as_deref(),
            Some("https://example.com/TEST_CODE/report.pdf")
        );
        assert_eq!(facts[0].source_target_price_upper, Some(14.0));
        assert_eq!(facts[0].source_target_price_lower, Some(12.0));
        assert_eq!(admitted.evidence().batch_id, BATCH_ID);
    }

    #[test]
    fn empty_partial_and_mismatched_batches_remain_distinct() {
        let empty =
            normalize_reports_batch(DataBatch::strict(Vec::new(), provenance(BATCH_ID))).unwrap();
        assert!(empty.is_verified_empty());

        let partial = DataBatch::best_effort(
            vec![report(BATCH_ID)],
            provenance(BATCH_ID),
            vec!["TEST_CODE missing page".to_owned()],
        )
        .unwrap();
        assert_eq!(
            normalize_reports_batch(partial).unwrap_err().reason_code(),
            "provider_partial_batch"
        );

        let mismatched =
            DataBatch::strict(vec![report("TEST_CODE_wrong_batch")], provenance(BATCH_ID));
        assert_eq!(
            normalize_reports_batch(mismatched)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );
    }

    #[test]
    fn provider_failures_keep_retry_semantics() {
        let invalid = eastmoney_gateway_error(EastmoneyError::InvalidRequest("bad".to_owned()));
        assert_eq!(invalid.reason_code(), "invalid_request");
        assert!(!invalid.retryable());

        let unsupported =
            eastmoney_gateway_error(EastmoneyError::Unsupported("missing".to_owned()));
        assert_eq!(unsupported.reason_code(), "provider_unsupported");
        assert!(!unsupported.retryable());

        let transport = eastmoney_gateway_error(EastmoneyError::Transport("offline".to_owned()));
        assert_eq!(transport.reason_code(), "provider_transport");
        assert!(transport.retryable());

        for error in [
            EastmoneyError::ResponseTooLarge { limit: 1 },
            EastmoneyError::Decode("bad json".to_owned()),
            EastmoneyError::Protocol("bad schema".to_owned()),
        ] {
            let mapped = eastmoney_gateway_error(error);
            assert_eq!(mapped.reason_code(), "provider_invalid_batch");
            assert!(!mapped.retryable());
        }
    }
}
