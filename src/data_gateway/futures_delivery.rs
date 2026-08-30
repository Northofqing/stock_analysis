//! BR-165/BR-199 evidence-preserving CFFEX futures-delivery acquisition.

use super::review::{acquisition_request_hash, audit_gateway_result};

use super::{GatewayBatch, GatewayError};
use crate::market_domain::ProviderId;

use chrono::NaiveDate;

const CAPABILITY: &str = "R-08-cffex-delivery";
const SOURCE: &str = "cffex-official-notice";

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
        let request_hash = acquisition_request_hash(CAPABILITY, format!("{year:04}-{month:02}"));
        // P4 M3 钩子: remote gRPC → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("FuturesDelivery") {
            Ok(bridge) => {
                let result = bridge.futures_delivery_async().await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Cffex);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Err(error) => {
                return audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Cffex,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }
}
