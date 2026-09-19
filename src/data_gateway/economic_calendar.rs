//! BR-133/BR-167 evidence-preserving macroeconomic release acquisition.

use super::review::{acquisition_request_hash, audit_gateway_result};

use super::{GatewayBatch, GatewayError};

use crate::market_domain::{ProviderId, SourceEvidence};
use chrono::{DateTime, Utc};

pub(crate) const CAPABILITY: &str = "EconomicCalendar-Jin10";
const SOURCE: &str = "jin10-flash-v1";
const MAX_LIMIT: u32 = 20;

/// One admitted public macroeconomic release with immutable upstream evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomicReleaseFact {
    pub event_id: String,
    pub indicator_id: u32,
    pub country: String,
    pub name: String,
    pub period: Option<String>,
    pub scheduled_at: DateTime<Utc>,
    pub released_at: DateTime<Utc>,
    pub previous: Option<String>,
    pub consensus: Option<String>,
    pub actual: Option<String>,
    pub revised: Option<String>,
    pub unit: Option<String>,
    pub importance: u32,
    pub impact: Option<String>,
    pub evidence: SourceEvidence,
}

/// Production seam for the released Jin10 economic-release provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct EconomicCalendarGateway;

impl EconomicCalendarGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn latest_releases(
        &self,
        limit: u32,
        country: Option<&str>,
    ) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
        let country = country.map(str::to_owned);
        // P4 M3 钩子: remote gRPC → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("EconomicCalendar") {
            Ok(bridge) => {
                let result = bridge.economic_calendar_async().await;
                return audit_macro_query(limit, country.as_deref(), result);
            }
            Err(error) => {
                return audit_macro_query(limit, country.as_deref(), Err(error));
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }
}

pub(crate) fn macro_request_hash(limit: u32, country: Option<&str>) -> String {
    acquisition_request_hash(CAPABILITY, format!("limit={limit}:country={}", country.unwrap_or("*")))
}

pub(crate) fn audit_macro_query(
    limit: u32,
    country: Option<&str>,
    result: Result<GatewayBatch<EconomicReleaseFact>, GatewayError>,
) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
    let provider = result.as_ref().map(|batch| batch.evidence().provider)
        .unwrap_or(ProviderId::Jin10);
    audit_gateway_result(CAPABILITY, provider, &macro_request_hash(limit, country), result)
}
