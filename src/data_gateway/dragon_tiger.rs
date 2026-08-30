//! BR-162 evidence-preserving R-04 whole-market dragon-tiger Gateway.

use super::review::{acquisition_request_hash, audit_gateway_result};

use super::{GatewayBatch, GatewayError};

use crate::market_domain::{DragonTigerSide, Exchange, ProviderId};
use chrono::NaiveDate;

const CAPABILITY: &str = "R-04";

/// One exact source seat from a complete buy-five/sell-five disclosure.
#[derive(Debug, Clone, PartialEq)]
pub struct DragonTigerSeatReview {
    pub side: DragonTigerSide,
    pub rank: u32,
    pub seat_name: String,
    pub amount_yuan: f64,
    pub buy_amount_yuan: Option<f64>,
    pub sell_amount_yuan: Option<f64>,
    pub net_amount_yuan: Option<f64>,
}

/// One source `TRADE_ID` disclosure. Distinct reasons remain distinct records.
#[derive(Debug, Clone, PartialEq)]
pub struct DragonTigerSourceDisclosure {
    pub entry_id: String,
    pub trade_id: String,
    pub reason: Option<String>,
    pub buy_amount_yuan: Option<f64>,
    pub sell_amount_yuan: Option<f64>,
    pub net_amount_yuan: Option<f64>,
    pub turnover_rate_pct: Option<f64>,
    pub seats: Vec<DragonTigerSeatReview>,
}

/// Report-ready aggregation for one stock, retaining all source disclosures.
#[derive(Debug, Clone, PartialEq)]
pub struct DragonTigerStockReview {
    pub exchange: Exchange,
    pub code: String,
    pub ranking_net_amount_yuan: f64,
    pub disclosures: Vec<DragonTigerSourceDisclosure>,
}

/// Evidence-preserving R-04 acquisition seam.
#[derive(Debug, Clone, Copy, Default)]
pub struct DragonTigerGateway;

impl DragonTigerGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn market_review(
        &self,
        trading_date: NaiveDate,
        disclosure_limit: u32,
        stock_limit: usize,
    ) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
        let request_hash = acquisition_request_hash(
            CAPABILITY,
            format!("{trading_date}:{disclosure_limit}:{stock_limit}"),
        );
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("DragonTiger") {
            Ok(bridge) => {
                let result = bridge
                    .dragon_tiger_async(trading_date, disclosure_limit, stock_limit)
                    .await;
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
