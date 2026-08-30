//! BR-164 evidence-preserving A-share index quote Gateway.
//!
//! Tencent's Magic provider is the only provider at the pinned upstream
//! revision that proves the normalized realtime quote contract for the six
//! domestic indices consumed by `MarketAnalyzer`.  The consumer therefore
//! receives one complete, ordered Tencent batch or an explicit failure; it no
//! longer owns a Tencent URL, HTTP client, retry loop, or wire parser.

use crate::market_domain::ProviderId;
use chrono::{DateTime, Utc};

use super::review::{acquisition_request_hash, audit_gateway_result, GatewayBatch, GatewayError};

const CAPABILITY: &str = "RealtimeIndexQuotes";

/// One admitted domestic-index quote and its record-level evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeIndexQuote {
    pub code: String,
    pub name: String,
    pub current: f64,
    pub change: f64,
    pub change_percent: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub previous_close: f64,
    pub volume: f64,
    pub amount: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Single-owner boundary for domestic-index quotes.
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexDataGateway;

impl IndexDataGateway {
    pub const fn new() -> Self {
        Self
    }

    pub fn realtime_quotes(
        &self,
        storage_codes: &[String],
    ) -> Result<GatewayBatch<RealtimeIndexQuote>, GatewayError> {
        let request_hash = acquisition_request_hash(CAPABILITY, storage_codes.join(","));
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("IndexQuotes") {
            Ok(bridge) => {
                if storage_codes.is_empty() {
                    return Err(GatewayError::invalid_request(
                        CAPABILITY,
                        "index quote request must contain at least one code",
                    ));
                }
                let result = bridge.index_quotes(storage_codes);
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tencent);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Err(error) => {
                return audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Tencent,
                    &request_hash,
                    Err(error),
                );
            }
        }
    }
}
