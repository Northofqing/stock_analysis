//! BR-119/BR-164 evidence-preserving research-report acquisition Gateway.

use super::review::{acquisition_request_hash, audit_gateway_result};

use super::{GatewayBatch, GatewayError};
use crate::market_domain::ProviderId;

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
        let request_hash = acquisition_request_hash(CAPABILITY, format!("{code}:1:{page_size}"));
        // P4 M4b: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("ResearchReports") {
            Ok(bridge) => {
                let result = bridge.research_reports_async(&code, page_size).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Eastmoney);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
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
    }
}

fn validate_code(code: &str) -> Result<&str, GatewayError> {
    a_share_instrument(code)?;
    Ok(code)
}

fn a_share_instrument(code: &str) -> Result<crate::market_domain::InstrumentId, GatewayError> {
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
