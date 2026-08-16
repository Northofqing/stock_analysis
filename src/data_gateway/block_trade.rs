//! BR-223 evidence-preserving 盘后大宗交易 Gateway (BlockTrades)。
//!
//! 数据源: 上游 magic-eastmoney-rs `BlockTrades` trait (东财 RPT_DATA_BLOCKTRADE,
//! 按代码查询)。聚合批次证据 = 首个成功真实批次的 provenance (逐代码真实证据
//! 保留在上游记录内, 不合成 batch_id, 遵守 BR-164)。

use super::instrument_identity::resolve_production_equity;
use super::review::{acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result};
use super::{BatchEvidence, GatewayBatch, GatewayError};
use chrono::NaiveDate;
#[cfg(feature = "magic-gateway")]
use magic_eastmoney_rs::EastmoneyClient;
use crate::magic_compat::{IsoDate, PositiveU32, ProviderId};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{BlockTrades, InstrumentDateRangeRequest};

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
        let request_hash = acquisition_request_hash(CAPABILITY, &codes.join(","));
        // P4 M3: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("BlockTrades") {
            Ok(Some(bridge)) => {
                let result = bridge.block_trades_async(codes, trading_date).await;
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
        let codes_owned = codes.to_vec();
        let result: Result<GatewayBatch<BlockTradeReview>, GatewayError> =
            match tokio::task::spawn_blocking(move || {
                fetch_block_trades(&codes_owned, trading_date)
            })
            .await
            {
                Ok(result) => result,
                Err(error) => audit_blocking_join_failure(
                    CAPABILITY,
                    ProviderId::Eastmoney,
                    request_hash.clone(),
                    error.to_string(),
                )
                .await,
            };
        audit_gateway_result(CAPABILITY, ProviderId::Eastmoney, &request_hash, result)
    }
}

fn fetch_block_trades(
    codes: &[String],
    trading_date: NaiveDate,
) -> Result<GatewayBatch<BlockTradeReview>, GatewayError> {
    let client = EastmoneyClient::new().map_err(|error| {
        GatewayError::unavailable(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            true,
            format!("EastmoneyClient::new failed: {error}"),
        )
    })?;
    let day = IsoDate::new(trading_date.format("%Y-%m-%d").to_string()).map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!("invalid trading date {}: {error}", trading_date),
        )
    })?;
    let limit = PositiveU32::new(100).map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!("limit: {error}"),
        )
    })?;

    let mut reviews: Vec<BlockTradeReview> = Vec::new();
    let mut issues: Vec<String> = Vec::new();
    let mut first_provenance: Option<crate::magic_compat::Provenance> = None;

    for code in codes {
        let identity = resolve_production_equity(code, None).map_err(|error| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Eastmoney),
                format!("resolve {code}: {error}"),
            )
        })?;
        let request = InstrumentDateRangeRequest::new(identity.instrument().clone(), limit)
            .and_then(|request| request.with_range(day.clone(), day.clone()))
            .map_err(|error| {
                GatewayError::unavailable(
                    CAPABILITY,
                    Some(ProviderId::Eastmoney),
                    true,
                    format!("request {code}: {error}"),
                )
            })?;
        match client.block_trades(&request) {
            Ok(batch) => {
                if first_provenance.is_none() {
                    first_provenance = Some(batch.provenance().clone());
                }
                for record in batch.records() {
                    let code = record.instrument.code().to_string();
                    reviews.push(BlockTradeReview {
                        code,
                        traded_at: record
                            .traded_at
                            .as_ref()
                            .map(|value| value.as_str().to_string()),
                        price: record.price.get(),
                        close_price: record.close_price.as_ref().map(|value| value.get()),
                        premium_ratio: record.premium_ratio.as_ref().map(|value| value.get()),
                        volume: record.volume.get(),
                        amount: record.amount.as_ref().map(|value| value.get()),
                        buyer: record
                            .buyer
                            .as_ref()
                            .map(|value| value.as_str().to_string()),
                        seller: record
                            .seller
                            .as_ref()
                            .map(|value| value.as_str().to_string()),
                    });
                }
            }
            Err(error) => issues.push(format!("{code}: {error}")),
        }
    }

    if reviews.is_empty() && !issues.is_empty() {
        return Err(GatewayError::unavailable(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            true,
            format!("all block-trade queries failed: {}", issues.join("; ")),
        ));
    }
    let provenance = first_provenance.as_ref().ok_or_else(|| {
        GatewayError::unavailable(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            true,
            "no block-trade batch provenance".to_string(),
        )
    })?;
    let evidence = BatchEvidence::from_provenance(ProviderId::Eastmoney, provenance).map_err(
        |error| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Eastmoney),
                format!("provenance: {error}"),
            )
        },
    )?;
    if reviews.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }
    Ok(GatewayBatch::Available {
        records: reviews,
        evidence,
    })
}
