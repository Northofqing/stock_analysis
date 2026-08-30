//! BR-161 evidence-preserving global-index and foreign-exchange acquisition.

use super::review::{acquisition_request_hash, audit_gateway_result};

use super::{GatewayBatch, GatewayError};

use crate::market_domain::{FxPair, GlobalIndexCode, ProviderId};
use chrono::{DateTime, Utc};

const INDEX_CAPABILITY: &str = "R-08-global-indices";
const FX_CAPABILITY: &str = "R-08-global-fx";
const SOURCE: &str = "sina-web";
const REALTIME_MAX_AGE_MILLIS: i64 = 5_000;

/// One admitted global-index quote with exact provider timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalIndexFact {
    pub code: GlobalIndexCode,
    pub name: String,
    pub value: f64,
    pub change: f64,
    pub change_percent: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// One admitted foreign-exchange quote with exact provider timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignExchangeFact {
    pub pair: FxPair,
    pub name: String,
    pub rate: f64,
    pub change: Option<f64>,
    pub change_percent: Option<f64>,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Production seam for the typed Sina global-market providers.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalMarketGateway;

impl GlobalMarketGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Acquires exactly Dow Jones, Nasdaq Composite and S&P 500.
    pub async fn us_indices(&self) -> Result<GatewayBatch<GlobalIndexFact>, GatewayError> {
        let request_hash =
            acquisition_request_hash(INDEX_CAPABILITY, "DowJones,NasdaqComposite,Sp500");
        // P4 M3 钩子 (2026-08-17 补): remote gRPC → gRPC 通道 (fail-closed,
        // audit 对等)。此前 GlobalIndices 服务端 delegate/桥方法已存在但网关钩子缺失
        // → 零 magic 生产构建 push_templates.rs us_indices 每天 fail-closed 报错。
        match super::grpc_source::bridge_for("GlobalIndices") {
            Ok(bridge) => {
                let result = bridge.global_indices_async().await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Sina);
                return audit_gateway_result(
                    INDEX_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    INDEX_CAPABILITY,
                    ProviderId::Sina,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }

    /// Acquires exactly the USD/CNY quote.
    pub async fn usd_cny(&self) -> Result<GatewayBatch<ForeignExchangeFact>, GatewayError> {
        let request_hash = acquisition_request_hash(FX_CAPABILITY, "UsdCny");
        // P4 M3 钩子: remote gRPC → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("ForeignExchange") {
            Ok(bridge) => {
                let result = bridge.foreign_exchange_async().await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Sina);
                return audit_gateway_result(FX_CAPABILITY, audit_provider, &request_hash, result);
            }
            Err(error) => {
                return audit_gateway_result(
                    FX_CAPABILITY,
                    ProviderId::Sina,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }
}
