//! Registered business rules: BR-119, BR-159, BR-164.
//! Typed seller-consensus acquisition and normalization.

use crate::data_gateway::review::{
    acquisition_request_hash, audit_gateway_result, GatewayBatch, GatewayError,
};

use crate::data_provider::consensus::ConsensusData;

use crate::market_domain::ProviderId;

const CAPABILITY: &str = "consensus";
const REPORT_LIMIT: u32 = 50;
const REPORT_WINDOW_DAYS: i64 = 180;

/// Production seller-consensus seam.
///
/// The blocking typed provider is constructed, used and dropped inside a
/// `spawn_blocking` worker so it cannot drop its HTTP runtime in Tokio's async
/// execution context.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsensusDataGateway;

impl ConsensusDataGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn fetch(&self, code: &str) -> Result<GatewayBatch<ConsensusData>, GatewayError> {
        let code = code.to_owned();
        let request_hash = acquisition_request_hash(
            CAPABILITY,
            format!("{code}:{REPORT_WINDOW_DAYS}:{REPORT_LIMIT}"),
        );
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("Consensus") {
            Ok(bridge) => {
                let result = bridge.consensus_async(&code).await;
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
