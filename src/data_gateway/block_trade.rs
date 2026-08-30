//! BR-223 evidence-preserving 盘后大宗交易 Gateway (BlockTrades)。
//!
//! 数据源: 上游 magic-eastmoney-rs `BlockTrades` trait (东财 RPT_DATA_BLOCKTRADE,
//! 按代码查询)。聚合批次证据 = 首个成功真实批次的 provenance (逐代码真实证据
//! 保留在上游记录内, 不合成 batch_id, 遵守 BR-164)。

use super::review::{acquisition_request_hash, audit_gateway_result};

use super::{GatewayBatch, GatewayError};
use crate::market_domain::ProviderId;

use chrono::NaiveDate;

const CAPABILITY: &str = "BlockTrades";

/// 报告就绪的大宗交易行 (保留来源字段)。
#[derive(Debug, Clone, PartialEq)]
pub struct BlockTradeReview {
    pub code: String,
    pub traded_at: Option<String>,
    pub price: f64,
    pub close_price: Option<f64>,
    pub premium_ratio: Option<f64>,
    pub volume: f64,
    pub amount: Option<f64>,
    pub buyer: Option<String>,
    pub seller: Option<String>,
}

/// Evidence-preserving 盘后大宗交易 acquisition seam。
#[derive(Debug, Clone, Copy, Default)]
pub struct BlockTradesGateway;

impl BlockTradesGateway {
    pub const fn new() -> Self {
        Self
    }

    /// 对给定代码集获取 trading_date 当日大宗交易 (逐代码查询)。
    pub async fn market_review(
        &self,
        codes: &[String],
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<BlockTradeReview>, GatewayError> {
        let request_hash = acquisition_request_hash(CAPABILITY, codes.join(","));
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("BlockTrades") {
            Ok(bridge) => {
                let result = bridge.block_trades_async(codes, trading_date).await;
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
