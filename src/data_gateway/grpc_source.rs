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
    GeneralWebResearchBatch, GlobalIndexFact, GlobalNewsRecord, ImplementedCorporateAction,
    InstrumentFundFlowFact, IntradayShapeFact, MagicTdxT0Batch, MarketMinutePoint,
    MarketMoneyFlow, MarketOrderBook, MarketSecurityMetadata, NorthboundDailyFact,
    ProviderTopNFact, RealtimeIndexQuote, RealtimeMarketQuote, ResearchReportFact,
    SinaInstrumentNewsRecord, UpperLimitRecord,
};
use crate::data_gateway::outcome_daily_bars::{OutcomeTransportFailure, RawOutcomeFetch};
use crate::data_provider::{consensus::ConsensusData, KlineData};
use crate::grpc_client::client::GrpcMarketClient;
use crate::grpc_client::envelope::QueryResult;
use crate::grpc_client::pb::magic::market::v1::Operation;
use chrono::NaiveDate;
use magic_market_core::{
    FinancialStatement, FlowInterval, InstrumentId, MarketStatistics, NorthboundChannel,
    StatementKind,
};
use magic_tdx_rs::protocol::types::SecurityBar;
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Mutex as AsyncMutex;

/// 桥单例缓存 (l6_sink OnceLock 模式): None = 未初始化 (首次调用时连接)。
static SOURCE: OnceLock<Mutex<Option<Arc<GrpcSource>>>> = OnceLock::new();

/// 纯同步线程 (无 tokio runtime) 的 block_on 载体。
static BRIDGE_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

const DEFAULT_ADDR: &str = "http://127.0.0.1:18082";

/// 已挂桥的 op 清单 (与各网关文件内 `super::grpc_source::bridge_for("X")` 调用
/// 一一对应)。变更时必须同步 — hooked_ops_match_bridge_for_call_sites 单测
/// 直接扫 src/data_gateway 源码断言集合相等, 防 rot (Spec Evidence Rule)。
pub const HOOKED_OPS: &[&str] = &[
    "Announcements",
    "BlockTrades",
    "BoardConstituents",
    "BoardDirectory",
    "BoardFlows",
    "BoardRanking",
    "Consensus",
    "CorporateActions",
    "DragonTiger",
    "EconomicCalendar",
    "FinancialStatements",
    "ForeignExchange",
    "FundFlowSeries",
    "FuturesDelivery",
    "GlobalNews",
    "HistoricalBars",
    "IndexQuotes",
    "InstrumentNews",
    "IntradayShape",
    "MarketStatistics",
    "MinuteData",
    "MoneyFlows",
    "NorthboundDaily",
    "OrderBooks",
    "OutcomeDailyBars",
    "ProviderTopNRankings",
    "RealtimeQuotes",
    "ResearchReports",
    "SecurityMetadata",
    "SemanticSearch",
    "T0Evidence",
    "TechnicalBars",
    "UpperLimitPoolReview",
];

/// 保持本地 (library 模式) 的网关能力 — P4 M3 风险条款: 服务端 op 已实现或
/// 半实现, 但桥保真未经验证 → 不静默切换, 出声 banner 列 follow-up。
/// 接桥时从本表删除并移入 HOOKED_OPS。
pub const KEEP_LOCAL_OPS: &[&str] = &["limit_pools", "strong_stock_reasons"];

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

/// M4 启动 banner (v15.x 出声原则): 数据源模式必须打印, 默认 library。
/// 语义与 bridge_for 完全一致 (DATA_GATEWAY_GRPC=1 才走 gRPC)。
/// main.rs [broker] 启动完成后调用。
pub fn startup_banner() -> String {
    let mode = if std::env::var("DATA_GATEWAY_GRPC").as_deref() == Ok("1") {
        "grpc"
    } else {
        "library"
    };
    let server = std::env::var("GRPC_MARKET_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    let disabled = std::env::var("DATA_GATEWAY_GRPC_DISABLED").unwrap_or_default();
    let disabled = if disabled.is_empty() {
        "无".to_string()
    } else {
        disabled
    };
    format!(
        "[data_gateway] 数据源模式 = {mode} | server = {server} | 桥接 {} ops | \
         禁用 = {disabled} | 保持本地 {} ops: {} \
         (limit_pools/strong_stock_reasons 是 A-10 chain 计算+DB 发布, 非数据请求 \
          — 服务端仅为同一共享表投影, 接桥会搬迁计算副作用, 违背 P4 只迁移 transport)",
        HOOKED_OPS.len(),
        KEEP_LOCAL_OPS.len(),
        KEEP_LOCAL_OPS.join(",")
    )
}

/// block_on 判别器 (两路, 任何线程上下文都安全 — 生产 monitor 的同步网关调用
/// 直接在 runtime 线程上发生, library 模式就是阻塞该线程, 桥必须保持同语义
/// 而不是 panic):
/// - runtime 上下文 (Handle 命中): 一律走独立 std 线程 + BRIDGE_RUNTIME 并
///   join。不做 Handle::block_on 是因为这些线程里它要么必 panic — worker task
///   (tokio 红线 "Cannot start a runtime from within a runtime", 2026-08-15
///   生产事故根因) 或 block_on 驱动主线程 (#[tokio::main]/#[tokio::test],
///   try_id 无法区分) — 要么只是碰巧合法 (spawn_blocking)。统一独立线程无
///   上下文误判风险; 本线程 join 阻塞到网络完成 = library 模式同步方法行为。
/// - 纯同步线程 (Handle 未命中): 静态 BRIDGE_RUNTIME 直接 block_on。
fn block_on<F: std::future::Future + Send>(fut: F) -> F::Output
where
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(_) => std::thread::scope(|s| {
            s.spawn(move || {
                BRIDGE_RUNTIME
                    .get_or_init(|| {
                        tokio::runtime::Builder::new_multi_thread()
                            .enable_all()
                            .worker_threads(2)
                            .build()
                            .expect("grpc bridge runtime 创建失败")
                    })
                    .block_on(fut)
            })
            .join()
            .unwrap_or_else(|_| panic!("grpc 桥 blocking 线程 panic (runtime 上下文路径)"))
        }),
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

    /// 研报: 逐代码 + page_size (与本地 ResearchDataGateway::instrument_reports
    /// 对齐; 服务端 fetch_research_reports 收 codes+page_size)。
    pub async fn research_reports_async(
        &self,
        code: &str,
        page_size: u32,
    ) -> Result<GatewayBatch<ResearchReportFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::ResearchReports,
                serde_json::json!({ "codes": [code], "page_size": page_size }),
            )
            .await?;
        convert::research_reports(&q)
    }

    /// 北向日数据: date + channel (与本地 CapitalDataGateway::northbound_daily
    /// 对齐; 服务端 fetch_northbound_daily 收 date+channel)。
    pub async fn northbound_daily_async(
        &self,
        trading_date: NaiveDate,
        channel: NorthboundChannel,
    ) -> Result<GatewayBatch<NorthboundDailyFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::NorthboundDaily,
                serde_json::json!({
                    "date": trading_date.format("%Y-%m-%d").to_string(),
                    "channel": format!("{channel:?}"),
                }),
            )
            .await?;
        convert::northbound_daily(&q)
    }

    /// 财务报告: codes + kind (与本地 CompanyDataGateway::financial_statements
    /// 对齐; 服务端 fetch_financial_statements 的 kind 是 snake_case 字面量)。
    pub async fn financial_statements_async(
        &self,
        codes: &[String],
        kind: StatementKind,
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        let kind = match kind {
            StatementKind::Balance => "balance",
            StatementKind::Income => "income",
            StatementKind::CashFlow => "cash_flow",
            other => {
                return Err(GatewayError::invalid_request(
                    "GrpcBridge",
                    format!("财务报告 kind 不支持走桥: {other:?}"),
                ))
            }
        };
        let q = self
            .query_op(
                Operation::FinancialStatements,
                serde_json::json!({ "codes": codes, "kind": kind }),
            )
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

    /// 资金流序列: 逐代码 + interval + limit (与本地
    /// CapitalDataGateway::instrument_fund_flow 对齐; 服务端
    /// fetch_fund_flow_series 收 codes+interval+limit)。
    pub async fn fund_flow_series_async(
        &self,
        code: &str,
        interval: FlowInterval,
        limit: u32,
    ) -> Result<GatewayBatch<InstrumentFundFlowFact>, GatewayError> {
        let q = self
            .query_op(
                Operation::FundFlowSeries,
                serde_json::json!({
                    "codes": [code],
                    "interval": format!("{interval:?}"),
                    "limit": limit,
                }),
            )
            .await?;
        convert::fund_flow_series(&q)
    }

    /// 头部排行双路 (volume_ratio + main_net_inflow): 与本地
    /// CapitalDataGateway::provider_top_n_pair 对齐 — 客户端 convert 按 metric
    /// 分组重建两个 GatewayBatch (request evidence 由本地方法构造, 桥只换
    /// transport 数据)。
    pub async fn provider_top_n_pair_async(
        &self,
        trading_date: NaiveDate,
    ) -> Result<
        (
            GatewayBatch<ProviderTopNFact>,
            GatewayBatch<ProviderTopNFact>,
        ),
        GatewayError,
    > {
        let q = self
            .query_op(
                Operation::ProviderTopNRankings,
                serde_json::json!({ "date": trading_date.format("%Y-%m-%d").to_string() }),
            )
            .await?;
        convert::provider_top_n_pair(&q)
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

    /// 日内形态: 逐代码 (与本地 IntradayShapeGateway::current_shape 对齐;
    /// 服务端 fetch_intraday_shape 收 codes)。
    pub async fn intraday_shape_async(
        &self,
        code: &str,
    ) -> Result<GatewayBatch<IntradayShapeFact>, GatewayError> {
        let q = self
            .query_op(Operation::IntradayShape, serde_json::json!({ "codes": [code] }))
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

    /// 联网检索: query + limit (与本地 GeneralWebResearchGateway::search
    /// 对齐; 服务端 fetch_semantic_search 收 query+limit, API key 在服务端持有)。
    pub async fn semantic_search_async(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<GeneralWebResearchBatch, GatewayError> {
        let q = self
            .query_op(
                Operation::SemanticSearch,
                serde_json::json!({ "query": query, "limit": limit }),
            )
            .await?;
        convert::semantic_search(&q, query)
    }

    /// 公司行动: code + window (与本地 SecurityLifecycleGateway::acquire 的
    /// corporate_actions 部分对齐; 服务端 fetch_corporate_actions 收
    /// code+window_start+window_end, 已服务端侧完成 Implemented 投影)。
    pub async fn corporate_actions_async(
        &self,
        code: &str,
        window_start: NaiveDate,
        window_end: NaiveDate,
    ) -> Result<GatewayBatch<ImplementedCorporateAction>, GatewayError> {
        let q = self
            .query_op(
                Operation::CorporateActions,
                serde_json::json!({
                    "code": code,
                    "window_start": window_start.format("%Y-%m-%d").to_string(),
                    "window_end": window_end.format("%Y-%m-%d").to_string(),
                }),
            )
            .await?;
        convert::corporate_actions(&q)
    }

    /// outcome 复盘日线 (P4 M3): 服务端执行 adaptive 抓取 (claim 台账留客户端),
    /// 视图重建 RawOutcomeFetch / OutcomeTransportFailure (error+attempts 保真)。
    /// 参数与 fetch_magic_tdx_outcome_adaptive 对齐 (instrument 对象 round-trip)。
    pub async fn outcome_daily_bars_async(
        &self,
        instrument: InstrumentId,
        market: String,
        code: String,
        expected_bar_count: u16,
        maximum_latest_n: u16,
        window_start: NaiveDate,
    ) -> Result<RawOutcomeFetch, OutcomeTransportFailure> {
        let q = self
            .query_op(
                Operation::OutcomeDailyBars,
                serde_json::json!({
                    "instrument": instrument,
                    "market": market,
                    "code": code,
                    "expected_bar_count": expected_bar_count,
                    "maximum_latest_n": maximum_latest_n,
                    "window_start": window_start.format("%Y-%m-%d").to_string(),
                }),
            )
            .await
            .map_err(|e| OutcomeTransportFailure::new(e, Vec::new()))?;
        convert::outcome_daily_bars(&q)
    }

    /// 同步包装 (spawn_blocking / 纯同步线程)。
    pub fn outcome_daily_bars_adaptive(
        &self,
        instrument: InstrumentId,
        market: String,
        code: String,
        expected_bar_count: u16,
        maximum_latest_n: u16,
        window_start: NaiveDate,
    ) -> Result<RawOutcomeFetch, OutcomeTransportFailure> {
        block_on(self.outcome_daily_bars_async(
            instrument,
            market,
            code,
            expected_bar_count,
            maximum_latest_n,
            window_start,
        ))
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

    #[tokio::test]
    async fn sync_method_from_async_worker_does_not_panic() {
        // 生产事故回归 (2026-08-15 21:07 主线程 panic 杀进程): monitor 同步
        // 网关调用直接在 async worker 上发生, 旧判别器 Handle::block_on 触发
        // tokio "Cannot start a runtime from within a runtime"。修复后 async
        // worker 走独立 std 线程路径 → 服务端不可达返回 Err 而非 panic。
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        reset_bridge();
        let bridge = bridge_for("RealtimeQuotes").unwrap().expect("bridge 实例存在");
        let err = bridge.realtime_quotes(&["600519".to_string()]).unwrap_err();
        assert!(err.retryable(), "async worker 路径也必须 fail-closed retryable");
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("GRPC_MARKET_ADDR");
        reset_bridge();
    }

    #[test]
    fn startup_banner_defaults_to_library() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::remove_var("GRPC_MARKET_ADDR");
        let b = startup_banner();
        assert!(b.contains("数据源模式 = library"), "默认必须 library (v15.x 出声): {b}");
        assert!(b.contains("server = http://127.0.0.1:18082"), "默认地址: {b}");
        assert!(b.contains("禁用 = 无"), "无禁用: {b}");
        assert!(b.contains("保持本地 2 ops"), "keep-local 计数: {b}");
    }

    #[test]
    fn startup_banner_grpc_mode_and_disabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("DATA_GATEWAY_GRPC", "1");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:19001");
        std::env::set_var("DATA_GATEWAY_GRPC_DISABLED", "T0Evidence,InstrumentNews");
        let b = startup_banner();
        assert!(b.contains("数据源模式 = grpc"), "grpc 模式: {b}");
        assert!(b.contains("server = http://127.0.0.1:19001"), "显式地址: {b}");
        assert!(b.contains("禁用 = T0Evidence,InstrumentNews"), "禁用列表: {b}");
        assert!(b.contains("保持本地 2 ops"), "keep-local 计数: {b}");
        assert!(
            b.contains("A-10 chain 计算+DB 发布"),
            "keep-local 原因出声: {b}"
        );
        std::env::remove_var("DATA_GATEWAY_GRPC");
        std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
        std::env::remove_var("GRPC_MARKET_ADDR");
    }

    #[test]
    fn hooked_ops_disjoint_from_keep_local() {
        for op in HOOKED_OPS {
            assert!(
                !KEEP_LOCAL_OPS.contains(&op),
                "{op} 同时出现在 HOOKED_OPS 和 KEEP_LOCAL_OPS — 必须只在一处"
            );
        }
    }

    #[test]
    fn hooked_ops_match_bridge_for_call_sites() {
        // Spec Evidence Rule: banner 的 HOOKED_OPS 必须与真实钩子一致, 防 rot。
        // 扫 src/data_gateway/ 下除 grpc_source.rs 外各文件的 bridge_for 调用
        // (grpc_source.rs 自身的 bridge_for 是单测不是钩子), 去重后集合断言相等。
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/data_gateway");
        let mut found: Vec<String> = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .expect("src/data_gateway 可读")
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("grpc_source.rs") {
                continue; // 定义模块: 钩子调用在其他网关文件
            }
            let text = std::fs::read_to_string(&path).expect("读网关源文件");
            for (idx, _) in text.match_indices("bridge_for(\"") {
                let rest = &text[idx + "bridge_for(\"".len()..];
                let end = rest.find('"').unwrap_or_else(|| panic!("bridge_for 名称未闭合: {path:?}"));
                found.push(rest[..end].to_string());
            }
        }
        // 一个 op 可有多个钩子调用点 (day1_flows/day5_flows 等) → 去重。
        found.sort();
        found.dedup();
        let mut expected: Vec<&str> = HOOKED_OPS.to_vec();
        expected.sort();
        assert_eq!(
            found, expected,
            "HOOKED_OPS 与真实 bridge_for 调用不一致 (banner 会撒谎)。\
             改钩子时同步 const, 改 const 时同步钩子。"
        );
    }
}
