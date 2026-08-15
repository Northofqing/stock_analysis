//! P4 M2: data_gateway → gRPC 桥 (双进程模式: monitor 连独立 grpc_market_server)。
//! env 契约:
//! - `DATA_GATEWAY_GRPC=1` 启用 (缺省 library 模式, 出声默认 v15.x);
//! - `GRPC_MARKET_ADDR` 服务端地址 (默认 http://127.0.0.1:18082);
//! - `DATA_GATEWAY_GRPC_DISABLED=RealtimeQuotes,HistoricalBars` 按 op 名 opt-out。
//! fail-closed: 服务端不可达 / 证据链缺失 → GatewayError::unavailable
//! (retryable=true) 或 invalid_evidence, 绝不静默回退 library。
//!
//! 同步方法 (realtime_quotes/daily_bars) 在 spawn_blocking 线程里调用 →
//! Handle::block_on; 纯同步线程 → 静态 BRIDGE_RUNTIME。
pub mod convert;

use crate::data_gateway::{
    board_ranking::BoardRankingFact, BlockTradeReview, BoardDirectoryFact, BoardFlowFact,
    BoardKind, BoardMembershipRecord, DragonTigerStockReview, EconomicReleaseFact,
    EventAnnouncement, ForeignExchangeFact, FuturesDeliveryFact, GatewayBatch, GatewayError,
    GlobalIndexFact, GlobalNewsRecord, InstrumentFundFlowFact, IntradayShapeFact,
    MagicTdxT0Batch, MarketMinutePoint, MarketMoneyFlow, MarketOrderBook,
    MarketSecurityMetadata, NorthboundDailyFact, ProviderTopNFact, RealtimeIndexQuote,
    RealtimeMarketQuote, ResearchReportFact, SinaInstrumentNewsRecord, UpperLimitRecord,
};
use crate::data_provider::{consensus::ConsensusData, KlineData};
use crate::grpc_client::client::GrpcMarketClient;
use crate::grpc_client::envelope::QueryResult;
use crate::grpc_client::pb::magic::market::v1::Operation;
use chrono::NaiveDate;
use magic_market_core::{FinancialStatement, MarketStatistics};
use magic_tdx_rs::protocol::types::SecurityBar;
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Mutex as AsyncMutex;

/// 桥单例缓存 (l6_sink OnceLock 模式): None = 未初始化 (首次调用时连接)。
static SOURCE: OnceLock<Mutex<Option<Arc<GrpcSource>>>> = OnceLock::new();

/// 纯同步线程 (无 tokio runtime) 的 block_on 载体。
static BRIDGE_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

const DEFAULT_ADDR: &str = "http://127.0.0.1:18082";

/// 网关钩子入口: DATA_GATEWAY_GRPC=1 且 op 未被 DISABLED → Some(Arc<GrpcSource>)
/// (惰性连接, 失败不缓存); 否则 Ok(None) (library 路径)。
/// 连接失败 → Err(unavailable retryable) (fail-closed)。
pub fn bridge_for(op: &str) -> Result<Option<Arc<GrpcSource>>, GatewayError> {
    if std::env::var("DATA_GATEWAY_GRPC").as_deref() != Ok("1") {
        return Ok(None);
    }
    let disabled = std::env::var("DATA_GATEWAY_GRPC_DISABLED").unwrap_or_default();
    if disabled.split(',').any(|name| name.trim() == op) {
        log::warn!(
            "[data_gateway] gRPC 桥: op {op} 被 DATA_GATEWAY_GRPC_DISABLED 排除, 走 library"
        );
        return Ok(None);
    }
    let cell = SOURCE.get_or_init(|| Mutex::new(None));
    if let Some(source) = cell.lock().unwrap().as_ref() {
        return Ok(Some(source.clone()));
    }
    // 连接是惰性的: 此处只注册桥实例 (连接在首个方法调用 ensure_connected 做,
    // 避免 block_on 在 async 上下文 panic; 失败不缓存语义在方法层保持)。
    let arc = Arc::new(GrpcSource {
        addr: std::env::var("GRPC_MARKET_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string()),
        client: AsyncMutex::new(None),
    });
    *cell.lock().unwrap() = Some(arc.clone());
    Ok(Some(arc))
}

/// 测试/重置用: 清空桥缓存 (重连)。
pub fn reset_bridge() {
    if let Some(cell) = SOURCE.get() {
        *cell.lock().unwrap() = None;
    }
}

/// block_on 判别器: 有 runtime 线程 (spawn_blocking) → Handle::block_on;
/// 纯同步线程 → 静态 BRIDGE_RUNTIME。
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(fut),
        Err(_) => BRIDGE_RUNTIME
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                    .expect("grpc bridge runtime 创建失败")
            })
            .block_on(fut),
    }
}

/// gRPC 客户端桥: 每 op 一个查询方法, 内部 client.query (§10 重试语义) + convert。
/// 连接是惰性 async (`ensure_connected`, 在方法层做) — 同步方法在 blocking 线程
/// 经 block_on 调用, async 方法在 runtime worker 调用; 首连放在方法层避免
/// bridge_for 里 block_on 在 async 上下文 panic (tokio 禁止)。
pub struct GrpcSource {
    addr: String,
    /// 连接态缓存: None = 尚未连接成功 (失败不缓存, 下次调用重试)。
    /// tokio Mutex: 跨 await 持有 (Send) — delegate JoinSet spawn 要求。
    client: AsyncMutex<Option<GrpcMarketClient>>,
}

impl GrpcSource {
    async fn ensure_connected(&self) -> Result<(), GatewayError> {
        let mut guard = self.client.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        let client = GrpcMarketClient::connect(&self.addr).await.map_err(|e| {
            GatewayError::unavailable(
                "GrpcBridge",
                None,
                true,
                format!("gRPC 服务端 {} 不可达: {e}", self.addr),
            )
        })?;
        log::info!("[data_gateway] gRPC 桥已连接: server={}", self.addr);
        *guard = Some(client);
        Ok(())
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    async fn query_op(&self, op: Operation, params: Value) -> Result<QueryResult, GatewayError> {
        self.ensure_connected().await?;
        let mut guard = self.client.lock().await;
        let client = guard.as_mut().expect("ensure_connected 后必有 client");
        client.query(op, params).await.map_err(|e| {
            GatewayError::unavailable(
                "GrpcBridge",
                None,
                true,
                format!(
                    "gRPC {:?} 查询失败: {e}",
                    crate::grpc_contract::ops::method_name(op)
                ),
            )
        })
    }

    // ---------- 6 个首批 op (M2) ----------

    pub async fn realtime_quotes_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        let q = self
            .query_op(Operation::RealtimeQuotes, serde_json::json!({ "codes": codes }))
            .await?;
        convert::realtime_quotes(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn realtime_quotes(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        block_on(self.realtime_quotes_async(codes))
    }

    pub async fn minute_data_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
        let q = self
            .query_op(Operation::MinuteData, serde_json::json!({ "codes": [code] }))
            .await?;
        convert::minute_data(&q)
    }

    pub fn minute_data(&self, code: &str) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
        block_on(self.minute_data_async(code))
    }

    pub async fn order_books_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
        let q = self
            .query_op(Operation::OrderBooks, serde_json::json!({ "codes": codes }))
            .await?;
        convert::order_books(&q)
    }

    pub fn order_books(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
        block_on(self.order_books_async(codes))
    }

    pub async fn money_flows_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
        let q = self
            .query_op(Operation::MoneyFlows, serde_json::json!({ "codes": codes }))
            .await?;
        convert::money_flows(&q)
    }

    pub fn money_flows(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
        block_on(self.money_flows_async(codes))
    }

    pub async fn security_metadata_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
        // 文档 §8 契约: instruments 对象数组 (exchange 由 code 前缀推导)。
        let params = crate::grpc_contract::params::instruments_for(codes);
        let q = self
            .query_op(Operation::SecurityMetadata, params)
            .await?;
        convert::security_metadata(&q)
    }

    pub fn security_metadata(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
        block_on(self.security_metadata_async(codes))
    }

    pub async fn daily_bars_async(
        &self,
        code: &str,
        days: usize,
    ) -> Result<GatewayBatch<KlineData>, GatewayError> {
        let q = self
            .query_op(
                Operation::HistoricalBars,
                serde_json::json!({ "codes": [code], "days": days }),
            )
            .await?;
        convert::historical_bars(code, &q)
    }

    /// 同步包装。
    pub fn daily_bars(
        &self,
        code: &str,
        days: usize,
    ) -> Result<GatewayBatch<KlineData>, GatewayError> {
        block_on(self.daily_bars_async(code, days))
    }

    // ---------- M3 批次 1: 全球市场/日历/公告/新闻/交割 ----------

    pub async fn global_indices_async(
        &self,
    ) -> Result<GatewayBatch<GlobalIndexFact>, GatewayError> {
        let q = self
            .query_op(Operation::GlobalIndices, serde_json::json!({}))
            .await?;
        convert::global_indices(&q)
    }

    pub async fn foreign_exchange_async(
        &self,
    ) -> Result<GatewayBatch<ForeignExchangeFact>, GatewayError> {
        let q = self
            .query_op(Operation::ForeignExchange, serde_json::json!({}))
            .await?;
        convert::foreign_exchange(&q)
    }

    pub async fn announcements_async(
        &self,
    ) -> Result<GatewayBatch<EventAnnouncement>, GatewayError> {
        let q = self
            .query_op(Operation::Announcements, serde_json::json!({}))
            .await?;
        convert::announcements(&q)
    }

    pub async fn global_news_async(
        &self,
    ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
        let q = self
            .query_op(Operation::GlobalNews, serde_json::json!({}))
            .await?;
        convert::global_news(&q)
    }

    pub async fn economic_calendar_async(
        &self,
    ) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
        let q = self
            .query_op(Operation::EconomicCalendar, serde_json::json!({}))
            .await?;
        convert::economic_calendar(&q)
    }

    pub async fn futures_delivery_async(
        &self,
    ) -> Result<GatewayBatch<FuturesDeliveryFact>, GatewayError> {
        let q = self
            .query_op(Operation::FuturesDelivery, serde_json::json!({}))
            .await?;
        convert::futures_delivery(&q)
    }

    // ---------- M3 批次 2: 龙虎榜/大宗/一致预期/板块/研报/北向/财务/技术/资金流/排行/指数/个股新闻/形态/涨停复盘/T0 ----------

    /// 龙虎榜: 参数与本地 DragonTigerGateway::market_review 对齐 (date +
    /// disclosure_limit + stock_limit)。
    pub async fn dragon_tiger_async(
        &self,
        trading_date: NaiveDate,
        disclosure_limit: u32,
        stock_limit: usize,
    ) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
        let q = self
            .query_op(
                Operation::DragonTiger,
                serde_json::json!({
                    "date": trading_date.format("%Y-%m-%d").to_string(),
                    "disclosure_limit": disclosure_limit,
                    "stock_limit": stock_limit,
                }),
            )
            .await?;
        convert::dragon_tiger(&q)
    }

    pub async fn market_dragon_tiger_async(
        &self,
    ) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
        let q = self
            .query_op(Operation::MarketDragonTiger, serde_json::json!({}))
            .await?;
        convert::market_dragon_tiger(&q)
    }

    /// 大宗交易: 参数与本地 BlockTradesGateway::market_review 对齐 (codes + date)。
    pub async fn block_trades_async(
        &self,
        codes: &[String],
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<BlockTradeReview>, GatewayError> {
        let q = self
            .query_op(
                Operation::BlockTrades,
                serde_json::json!({
                    "codes": codes,
                    "date": trading_date.format("%Y-%m-%d").to_string(),
                }),
            )
            .await?;
        convert::block_trades(&q)
    }

    /// 一致预期: 逐代码 (与本地 ConsensusDataGateway::fetch 对齐)。
    pub async fn consensus_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<ConsensusData>, GatewayError> {
        let q = self
            .query_op(Operation::Consensus, serde_json::json!({ "codes": [code] }))
            .await?;
        convert::consensus(&q)
    }

    /// 板块目录: kind + limit (与本地 BoardDataGateway::directory 对齐)。
    pub async fn board_directory_async(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardDirectoryFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::BoardDirectory,
                serde_json::json!({ "kind": format!("{kind:?}"), "limit": limit }),
            )
            .await?;
        convert::board_directory(&q)
    }

    /// 板块成分归属: 逐代码 (与本地 BoardDataGateway::memberships 对齐)。
    pub async fn board_constituents_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
        let q = self
            .query_op(Operation::BoardConstituents, serde_json::json!({ "codes": [code] }))
            .await?;
        convert::board_constituents(&q)
    }

    /// 板块资金流: kind + limit (与本地 BoardDataGateway::day1_flows 对齐)。
    pub async fn board_flows_async(
        &self,
        kind: BoardKind,
        limit: u32,
    ) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::BoardFlows,
                serde_json::json!({ "kind": format!("{kind:?}"), "limit": limit }),
            )
            .await?;
        convert::board_flows(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程), 与本地 day1_flows_blocking 对齐。
    pub fn board_flows(&self, kind: BoardKind, limit: u32) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
        block_on(self.board_flows_async(kind, limit))
    }

    /// 板块排行: fid 路由 (f3 → ConceptHits, f62 → MarketRankings) + top_n
    /// (与本地 BoardRankingGateway::fetch_top 对齐; 非法 fid fail-closed)。
    pub async fn board_ranking_async(
        &self,
        fid: &str,
        top_n: usize,
    ) -> Result<GatewayBatch<BoardRankingFact>, GatewayError> {
        let operation = match fid {
            "f3" => Operation::ConceptHits,
            "f62" => Operation::MarketRankings,
            _ => {
                return Err(GatewayError::invalid_request(
                    "GrpcBridge",
                    format!("板块排行 fid 非法: {fid:?} (允许 f3/f62)"),
                ))
            }
        };
        let q = self
            .query_op(operation, serde_json::json!({ "top_n": top_n }))
            .await?;
        convert::board_ranking(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn board_ranking(
        &self,
        fid: &str,
        top_n: usize,
    ) -> Result<GatewayBatch<BoardRankingFact>, GatewayError> {
        block_on(self.board_ranking_async(fid, top_n))
    }

    pub async fn research_reports_async(
        &self,
    ) -> Result<GatewayBatch<ResearchReportFact>, GatewayError> {
        let q = self
            .query_op(Operation::ResearchReports, serde_json::json!({}))
            .await?;
        convert::research_reports(&q)
    }

    pub async fn northbound_daily_async(
        &self,
    ) -> Result<GatewayBatch<NorthboundDailyFact>, GatewayError> {
        let q = self
            .query_op(Operation::NorthboundDaily, serde_json::json!({}))
            .await?;
        convert::northbound_daily(&q)
    }

    pub async fn financial_statements_async(
        &self,
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        let q = self
            .query_op(Operation::FinancialStatements, serde_json::json!({}))
            .await?;
        convert::financial_statements(&q)
    }

    /// 估值统计: codes (与本地 CompanyDataGateway::market_statistics 对齐)。
    pub async fn market_statistics_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketStatistics>, GatewayError> {
        let q = self
            .query_op(Operation::MarketStatistics, serde_json::json!({ "codes": codes }))
            .await?;
        convert::market_statistics(&q)
    }

    /// 15 分钟线: codes + count (与本地 HistoricalBarsGateway::fifteen_min_bars
    /// 对齐)。
    pub async fn technical_bars_async(
        &self,
        codes: &[String],
        count: u32,
    ) -> Result<GatewayBatch<SecurityBar>, GatewayError> {
        let q = self
            .query_op(
                Operation::TechnicalBars,
                serde_json::json!({ "codes": codes, "count": count }),
            )
            .await?;
        convert::technical_bars(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn technical_bars(
        &self,
        codes: &[String],
        count: u32,
    ) -> Result<GatewayBatch<SecurityBar>, GatewayError> {
        block_on(self.technical_bars_async(codes, count))
    }

    pub async fn fund_flow_series_async(
        &self,
        limit: u32,
    ) -> Result<GatewayBatch<InstrumentFundFlowFact>, GatewayError> {
        let q = self
            .query_op(Operation::FundFlowSeries, serde_json::json!({ "limit": limit }))
            .await?;
        convert::fund_flow_series(&q)
    }

    pub async fn provider_top_n_rankings_async(
        &self,
    ) -> Result<GatewayBatch<ProviderTopNFact>, GatewayError> {
        let q = self
            .query_op(Operation::ProviderTopNRankings, serde_json::json!({}))
            .await?;
        convert::provider_top_n_rankings(&q)
    }

    /// 指数实时行情: codes (与本地 IndexDataGateway::realtime_quotes 对齐)。
    pub async fn index_quotes_async(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeIndexQuote>, GatewayError> {
        let q = self
            .query_op(Operation::IndexQuotes, serde_json::json!({ "codes": codes }))
            .await?;
        convert::index_quotes(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn index_quotes(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeIndexQuote>, GatewayError> {
        block_on(self.index_quotes_async(codes))
    }

    /// 个股新闻: codes + from_days (与本地 SinaInstrumentNewsGateway
    /// instrument_news_in_range 对齐; 范围终点=服务端当前时刻, 同机等价)。
    pub async fn instrument_news_async(
        &self,
        codes: &[String],
        from_days: u32,
    ) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
        let q = self
            .query_op(
                Operation::InstrumentNews,
                serde_json::json!({ "codes": codes, "from_days": from_days }),
            )
            .await?;
        convert::instrument_news(&q)
    }

    pub async fn intraday_shape_async(
        &self,
    ) -> Result<GatewayBatch<IntradayShapeFact>, GatewayError> {
        let q = self
            .query_op(Operation::IntradayShape, serde_json::json!({}))
            .await?;
        convert::intraday_shape(&q)
    }

    /// 涨停复盘: date (与本地 ReviewGateway::r03_upper_limit_pool 对齐)。
    pub async fn upper_limit_pool_review_async(
        &self,
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<UpperLimitRecord>, GatewayError> {
        let q = self
            .query_op(
                Operation::UpperLimitPoolReview,
                serde_json::json!({ "date": trading_date.format("%Y-%m-%d").to_string() }),
            )
            .await?;
        convert::upper_limit_pool_review(&q)
    }

    /// T0 证据批: 返回 MagicTdxT0Batch (records + rejections 全量, 与本地
    /// MagicTdxGateway::get_t0_evidence_batch 对齐 — rejections 不能丢)。
    pub async fn t0_evidence_batch_async(
        &self,
        codes: &[String],
    ) -> Result<MagicTdxT0Batch, GatewayError> {
        let q = self
            .query_op(Operation::T0Evidence, serde_json::json!({ "codes": codes }))
            .await?;
        convert::t0_evidence_batch(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn t0_evidence_batch(
        &self,
        codes: &[String],
    ) -> Result<MagicTdxT0Batch, GatewayError> {
        block_on(self.t0_evidence_batch_async(codes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // env 是进程级: 这些测试并行时会互相看到对方的 env (race)。
    // 共享锁串行化 env 敏感的测试 (M3 全量并行跑时暴露)。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn bridge_disabled_without_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::remove_var("GRPC_MARKET_ADDR");
        assert!(bridge_for("RealtimeQuotes").unwrap().is_none());
    }

    #[test]
    fn bridge_disabled_by_op_name() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("DATA_GATEWAY_GRPC_DISABLED", "RealtimeQuotes");
        std::env::remove_var("GRPC_MARKET_ADDR");
        assert!(
            bridge_for("RealtimeQuotes").unwrap().is_none(),
            "DISABLED 命中 → library"
        );
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
    }

    #[test]
    fn bridge_enabled_but_unreachable_is_fail_closed() {
        // 连接是惰性的: bridge_for 只注册实例, fail-closed 在方法层
        // (首个查询 ensure_connected 失败 → unavailable retryable)。
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        reset_bridge();
        let bridge = bridge_for("RealtimeQuotes").unwrap().expect("bridge 实例存在");
        let err = bridge.realtime_quotes(&["600519".to_string()]).unwrap_err();
        assert!(err.retryable(), "服务端不可达必须 retryable");
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_ADDR");
        reset_bridge();
    }
}
