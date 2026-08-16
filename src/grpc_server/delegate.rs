//! data_gateway 委托层 (方案 A): 服务端进程内调用 data_gateway 取真实数据,
//! 序列化为 canonical JSON。fixture_mode 下不经过这里。
//! 每个 op 一个 fetch_xxx(params: &Value) -> Result<Fetched, DelegateError> (M1 起;
//! 既有 24 个 fetch 保持无参签名 → dispatch 里包 DelegateError::Fetch)。
//!
//! 签名说明 (实测, 非计划假设): data_gateway 全部是 async fn (内部自行
//! spawn_blocking), 所以 fetch 本身是 async, handler 直接 await, 不套 spawn_blocking。
//! 记录结构体没有 derive Serialize → 逐字段 json! 映射 (字段名 = 结构体字段名);
//! M1 扩展的 14 个 fetch 中已 derive Serialize 的 record (FinancialStatement/
//! MagicTdxT0Evidence/GeneralWebResearchRecord) 用 serde_json::to_value 直出。
use crate::data_gateway::{
    board_ranking::BoardRankingGateway, BlockTradesGateway, BoardDataGateway, BoardKind,
    CapitalDataGateway, ChainIntelligenceGateway, ConsensusDataGateway, DragonTigerGateway,
    EconomicCalendarGateway, EventCalendarGateway, FuturesDeliveryGateway, GlobalMarketGateway,
    GlobalNewsGateway, GlobalNewsProvider, HistoricalBarsGateway, IndexDataGateway,
    IntradayShapeGateway, MagicTdxGateway, MarketCapabilitiesGateway, NorthboundQuotaFact,
    ResearchDataGateway, ReviewDataGateway, SinaInstrumentNewsGateway,
};
use crate::data_gateway::{
    company::CompanyDataGateway,
    general_web_research::{GeneralWebResearchGateway, GeneralWebResearchProvider},
};
use crate::data_gateway::outcome_daily_bars::{
    fetch_magic_tdx_outcome_adaptive, OutcomeTransportFailure, RawOutcomeFetch,
};
use crate::magic_compat::{InstrumentId, ProviderId};
use magic_market_core::{FlowInterval, NorthboundChannel, StatementKind};
use crate::grpc_client::pb::magic::market::v1::Operation;
use chrono::{Datelike, Local, NaiveDate};
use serde_json::{json, Value};

/// 委托取数结果 (M1 起证据链回填 provider/source/batch_id; 合同 §6 缺则不填充)。
/// 既有 24 op 的 fetch 尚走 pack() (证据留空) → handlers 对空 provider 保留
/// "tdx-dev" 兼容值, M2 全量升级后移除。
pub struct Fetched {
    pub data: Vec<u8>,
    pub source_at: String,
    pub provider: String,
    pub source: String,
    pub batch_id: String,
}

/// 取数失败 (携带服务端侧分类, 客户端桥据此重建 GatewayError 保真;
/// proto ErrorDetail 字段: provider/reason_code/retryable)。
#[derive(Debug)]
pub struct FetchFailure {
    pub message: String,
    pub provider: Option<ProviderId>,
    pub reason_code: &'static str,
    pub retryable: bool,
}

impl FetchFailure {
    /// 从 data_gateway 网关错误提取分类 (provider/reason_code/retryable 保真)。
    pub fn from_gateway(e: crate::data_gateway::GatewayError) -> Self {
        Self {
            message: e.to_string(),
            provider: e.provider(),
            reason_code: e.reason_code(),
            retryable: e.retryable(),
        }
    }

    /// 非网关错误 (serde/IO 等) → 默认 unavailable 语义 (fail-closed, 可重试)。
    pub fn unknown(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            provider: None,
            reason_code: "no_verified_batch",
            retryable: true,
        }
    }
}

impl From<String> for FetchFailure {
    fn from(e: String) -> Self {
        FetchFailure::unknown(e)
    }
}

impl From<&str> for FetchFailure {
    fn from(e: &str) -> Self {
        FetchFailure::unknown(e)
    }
}

impl From<serde_json::Error> for FetchFailure {
    fn from(e: serde_json::Error) -> Self {
        FetchFailure::unknown(e.to_string())
    }
}

impl From<crate::data_gateway::GatewayError> for FetchFailure {
    fn from(e: crate::data_gateway::GatewayError) -> Self {
        FetchFailure::from_gateway(e)
    }
}

/// 委托层错误: Params = 请求方参数 (→ Status::invalid_argument);
/// Fetch = 取数失败 (→ Status::internal + ErrorDetail 分类, fail-closed 不静默回退)。
#[derive(Debug)]
pub enum DelegateError {
    Params(crate::grpc_contract::params::ParamsError),
    Fetch(FetchFailure),
}

impl std::fmt::Display for DelegateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DelegateError::Params(e) => write!(f, "{e}"),
            DelegateError::Fetch(failure) => write!(f, "取数失败: {}", failure.message),
        }
    }
}

impl From<crate::grpc_contract::params::ParamsError> for DelegateError {
    fn from(e: crate::grpc_contract::params::ParamsError) -> Self {
        DelegateError::Params(e)
    }
}

impl From<FetchFailure> for DelegateError {
    fn from(f: FetchFailure) -> Self {
        DelegateError::Fetch(f)
    }
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// batch 的 evidence 可信源时间 (合同 §6: 缺则不填充)。
fn source_at_of(batch: &crate::data_gateway::GatewayBatch<impl Sized>) -> String {
    batch.evidence().source_at.clone().unwrap_or_default()
}

/// 无 evidence 的视图打包 (provider/source/batch_id 留空, 合同 §6 缺则不填充)。
fn pack(records: Vec<Value>, source_at: String) -> Result<Fetched, FetchFailure> {
    Ok(Fetched {
        data: serde_json::to_vec(&records).map_err(|e| FetchFailure::unknown(e.to_string()))?,
        source_at,
        provider: String::new(),
        source: String::new(),
        batch_id: String::new(),
    })
}

/// GatewayBatch 打包: evidence 证据链 5 字段全回填 (M1 新 fetch 用)。
fn pack_ev(
    records: Vec<Value>,
    batch: &crate::data_gateway::GatewayBatch<impl Sized>,
) -> Result<Fetched, FetchFailure> {
    let ev = batch.evidence();
    pack_ev_from(
        records,
        ev.source_at.clone().unwrap_or_default(),
        format!("{:?}", ev.provider),
        ev.source.clone(),
        ev.batch_id.clone(),
    )
}

/// P4 M2: 通用证据回填打包 (非 GatewayBatch 载体如 AdmittedDailyBars 手动调用)。
fn pack_ev_from(
    records: Vec<Value>,
    source_at: String,
    provider: String,
    source: String,
    batch_id: String,
) -> Result<Fetched, FetchFailure> {
    Ok(Fetched {
        data: serde_json::to_vec(&records).map_err(|e| FetchFailure::unknown(e.to_string()))?,
        source_at,
        provider,
        source,
        batch_id,
    })
}

fn not_yet(op: Operation) -> Result<Fetched, FetchFailure> {
    Err(FetchFailure::unknown(format!(
        "{}: delegate 尚未实现 (Task 10 补全)",
        crate::grpc_contract::ops::method_name(op)
    )))
}

/// 统一取数入口 (M1: 新 14 个 fetch 收 params; 既有 24 个签名不变 → 显式 codes
/// 对它们尚不生效, M2 客户端桥接入时逐个升级)。
pub async fn fetch(
    op: Operation,
    _schema: &str,
    params: &Value,
) -> Result<Fetched, DelegateError> {
    match op {
        // P4 M2: 首批 6 op 升级收 params (客户端桥按 codes/days 精确请求)。
        Operation::RealtimeQuotes => fetch_realtime_quotes(params),
        Operation::HistoricalBars => fetch_historical_bars(params).await,
        Operation::MinuteData => fetch_minute_data(params).await,
        Operation::OrderBooks => fetch_order_books(params).await,
        Operation::MoneyFlows => fetch_money_flows(params).await,
        Operation::SecurityMetadata => fetch_security_metadata(params).await,
        Operation::GlobalIndices => fetch_global_indices().await.map_err(DelegateError::Fetch),
        Operation::Announcements => fetch_announcements().await.map_err(DelegateError::Fetch),
        Operation::GlobalNews => fetch_global_news().await.map_err(DelegateError::Fetch),
        Operation::EconomicCalendar => fetch_economic_calendar().await.map_err(DelegateError::Fetch),
        Operation::FuturesDelivery => fetch_futures_delivery().await.map_err(DelegateError::Fetch),
        Operation::DragonTiger => fetch_dragon_tiger(params).await.map_err(DelegateError::Fetch),
        Operation::BlockTrades => fetch_block_trades(params).await.map_err(DelegateError::Fetch),
        Operation::Consensus => fetch_consensus(params).await.map_err(DelegateError::Fetch),
        Operation::BoardDirectory => fetch_board_directory(params).await.map_err(DelegateError::Fetch),
        Operation::BoardConstituents => fetch_board_constituents(params).await.map_err(DelegateError::Fetch),
        Operation::BoardFlows => fetch_board_flows(params).await.map_err(DelegateError::Fetch),
        Operation::LimitPools => fetch_limit_pools(params).await.map_err(DelegateError::Fetch),
        Operation::StrongStockReasons => fetch_strong_stock_reasons(params).await.map_err(DelegateError::Fetch),
        Operation::MarketDragonTiger => fetch_market_dragon_tiger(params).await.map_err(DelegateError::Fetch),
        Operation::MarketRankings => fetch_market_rankings(params).await.map_err(DelegateError::Fetch),
        Operation::ConceptHits => fetch_concept_hits(params).await.map_err(DelegateError::Fetch),
        Operation::ResearchReports => fetch_research_reports(params).await.map_err(DelegateError::Fetch),
        Operation::NorthboundDaily => fetch_northbound_daily(params).await.map_err(DelegateError::Fetch),
        // M1 扩展 (P4): 8 个 proto 已有 op (直接返回 DelegateError, Params 可映射 400)。
        Operation::ForeignExchange => fetch_foreign_exchange(params).await,
        Operation::FinancialStatements => fetch_financial_statements(params).await,
        Operation::MarketStatistics => fetch_market_statistics(params).await,
        Operation::TechnicalBars => fetch_technical_bars(params).await,
        Operation::CorporateActions => fetch_corporate_actions(params).await,
        Operation::SemanticSearch => fetch_semantic_search(params).await,
        Operation::FundFlowSeries => fetch_fund_flow_series(params).await,
        Operation::ProviderTopNRankings => fetch_provider_top_n_rankings(params).await,
        // M1 扩展 (P4): 6 个新 op (proto 编号 55-60)。
        Operation::IndexQuotes => fetch_index_quotes(params).await,
        Operation::InstrumentNews => fetch_instrument_news(params).await,
        Operation::IntradayShape => fetch_intraday_shape(params).await,
        Operation::T0Evidence => fetch_t0_evidence(params).await,
        Operation::OutcomeDailyBars => fetch_outcome_daily_bars(params).await,
        Operation::UpperLimitPoolReview => fetch_upper_limit_pool_review(params).await,
        // M4c 扩展 (P4): A-10 完整 batch (本地扩展 61, monitor 复盘消费)。
        Operation::ChainBatch => fetch_chain_batch_full(params).await.map_err(DelegateError::Fetch),
        _ => not_yet(op).map_err(DelegateError::Fetch),
    }
}

// ---------- 统一实时行情 (Task 8 已落地, 同步路径) ----------

/// 字段映射以实际 struct 为准: RealtimeMarketQuote 有
/// code/name/price/previous_close/change_percent (无 volume/amount)。
/// P4 M2: 升级收 params (codes 缺省 watchlist)。
pub fn fetch_realtime_quotes(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let batch = crate::data_gateway::MarketDataGateway::new()
        .realtime_quotes(&codes)
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("统一实时行情 Gateway 不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|s| {
            json!({
                "code": s.code,
                "name": s.name,
                "price": s.price,
                "change_pct": s.change_percent,
                "previous_close": s.previous_close,
            })
        })
        .collect();
    // P4 M2: 证据链回填 (客户端桥需要真实 provider/source/batch_id)。
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}

// ---------- 核心 12 op (Task 9) ----------

/// P4 M2: 升级收 params (codes 缺省 watchlist)。
async fn fetch_minute_data(params: &Value) -> Result<Fetched, DelegateError> {
    let gateway = MarketCapabilitiesGateway::new();
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        let gateway = gateway;
        set.spawn(async move {
            gateway
                .minute_data(&code, None)
                .await
                .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("分钟线 Gateway 不可用 ({code}): {e}"))))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut evidence_first: Option<crate::data_gateway::BatchEvidence> = None;
    while let Some(joined) = set.join_next().await {
        let batch = joined.map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("分钟线 task 失败: {e}"))))?;
        let batch = batch?;
        if evidence_first.is_none() {
            evidence_first = Some(batch.evidence().clone());
        }
        records.extend(batch.records().iter().map(|r| {
            json!({
                "code": r.code,
                "minute_at": r.minute_at.to_rfc3339(),
                "price": r.price,
                "cumulative_quantity": r.cumulative_quantity,
                "cumulative_amount": r.cumulative_amount,
                "source_at": r.source_at.to_rfc3339(),
            })
        }));
    }
    // P4 M2: 证据链回填 (取首个 batch 的 evidence; 全部失败时 evidence_first 为 None
    // → 显式错误, 不静默填默认证据)。
    let ev = evidence_first
        .ok_or_else(|| DelegateError::Fetch(FetchFailure::unknown("分钟线: 无任何 batch 成功".to_string())))?;
    pack_ev_from(
        records,
        ev.source_at.clone().unwrap_or_default(),
        format!("{:?}", ev.provider),
        ev.source.clone(),
        ev.batch_id.clone(),
    )
    .map_err(DelegateError::Fetch)
}

/// P4 M2: 升级收 params (codes 缺省 watchlist)。
async fn fetch_order_books(params: &Value) -> Result<Fetched, DelegateError> {
    let gateway = MarketCapabilitiesGateway::new();
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let batch = gateway
        .order_books(&codes)
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("盘口 Gateway 不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            let level = |l: &crate::data_gateway::MarketBookLevel| {
                json!({"price": l.price, "quantity": l.quantity})
            };
            json!({
                "code": r.code,
                "bids": r.bids.iter().map(level).collect::<Vec<_>>(),
                "asks": r.asks.iter().map(level).collect::<Vec<_>>(),
                "total_bid_quantity": r.total_bid_quantity,
                "total_ask_quantity": r.total_ask_quantity,
                "source_at": r.source_at.to_rfc3339(),
            })
        })
        .collect();
    // P4 M2: 证据链回填。
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}

/// P4 M2: 升级收 params (codes 缺省 watchlist)。
async fn fetch_money_flows(params: &Value) -> Result<Fetched, DelegateError> {
    let gateway = MarketCapabilitiesGateway::new();
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let batch = gateway
        .money_flows(&codes)
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("资金流 Gateway 不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "code": r.code,
                "main_net": r.main_net,
                "super_large_net": r.super_large_net,
                "large_net": r.large_net,
                "medium_net": r.medium_net,
                "small_net": r.small_net,
                "source_at": r.source_at.to_rfc3339(),
            })
        })
        .collect();
    // P4 M2: 证据链回填。
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}

/// P4 M2: 升级收 params (文档 §8 instruments 格式, 缺省 watchlist)。
async fn fetch_security_metadata(params: &Value) -> Result<Fetched, DelegateError> {
    let gateway = MarketCapabilitiesGateway::new();
    let codes = crate::grpc_contract::params::resolve_instruments(params)?;
    let batch = gateway
        .security_metadata(&codes)
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("证券元数据 Gateway 不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "code": r.code,
                "name": r.name,
                "board": format!("{:?}", r.board),
                "is_st": r.is_st,
                "listed_on": r.listed_on.to_string(),
                "price_limit_percent": r.price_limit_percent,
                "source_at": r.source_at.to_rfc3339(),
            })
        })
        .collect();
    // P4 M2: 证据链回填。
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}

async fn fetch_global_indices() -> Result<Fetched, FetchFailure> {
    let gateway = GlobalMarketGateway::new();
    let batch = gateway
        .us_indices()
        .await
        .map_err(|e| format!("全球指数 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "code": format!("{:?}", r.code),
                "name": r.name,
                "value": r.value,
                "change": r.change,
                "change_percent": r.change_percent,
                "source_at": r.source_at.to_rfc3339(),
            })
        })
        .collect();
    pack(records, source_at)
}

async fn fetch_announcements() -> Result<Fetched, FetchFailure> {
    let gateway = EventCalendarGateway::new();
    let batch = gateway
        .market_announcements(today(), 100)
        .await
        .map_err(|e| format!("公告 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "announcement_id": r.announcement_id,
                "code": r.code,
                "category": r.category,
                "title": r.title,
                "published_at": r.published_at,
                "url": r.canonical_url,
            })
        })
        .collect();
    pack(records, source_at)
}

async fn fetch_global_news() -> Result<Fetched, FetchFailure> {
    let gateway = GlobalNewsGateway::new();
    let batch = gateway
        .global_news(GlobalNewsProvider::Eastmoney, 20)
        .await
        .map_err(|e| format!("全球新闻 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "item_id": r.item_id,
                "title": r.title,
                "summary": r.summary,
                "content": r.content,
                "publisher": r.publisher,
                "url": r.canonical_url,
                "published_at": r.published_at.to_rfc3339(),
                "instruments": r.instruments,
                "topics": r.topics,
                "language": r.language,
            })
        })
        .collect();
    pack(records, source_at)
}

async fn fetch_economic_calendar() -> Result<Fetched, FetchFailure> {
    let gateway = EconomicCalendarGateway::new();
    let batch = gateway
        .latest_releases(20, None)
        .await
        .map_err(|e| format!("财经日历 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "event_id": r.event_id,
                "indicator_id": r.indicator_id,
                "country": r.country,
                "name": r.name,
                "period": r.period,
                "scheduled_at": r.scheduled_at.to_rfc3339(),
                "released_at": r.released_at.to_rfc3339(),
                "previous": r.previous,
                "consensus": r.consensus,
                "actual": r.actual,
                "revised": r.revised,
                "unit": r.unit,
                "importance": r.importance,
                "impact": r.impact,
            })
        })
        .collect();
    pack(records, source_at)
}

async fn fetch_futures_delivery() -> Result<Fetched, FetchFailure> {
    let gateway = FuturesDeliveryGateway::new();
    let now = Local::now();
    let batch = gateway
        .cffex_contract_month(now.year() as u32, now.month())
        .await
        .map_err(|e| format!("交割日历 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "contract_code": r.contract_code,
                "product_code": r.product_code,
                "last_trading_date": r.last_trading_date.map(|d| d.to_string()),
                "delivery_date": r.delivery_date.to_string(),
                "notice_url": r.notice_url,
            })
        })
        .collect();
    pack(records, source_at)
}

/// 龙虎榜: 生产调用方传 trading_date + disclosure_limit + stock_limit
/// (push_templates r04, main.rs 12411) — 参数默认值保持 M1 行为 (今天/100/20),
/// 显式字段才改变行为 (v15.x)。
async fn fetch_dragon_tiger(params: &Value) -> Result<Fetched, FetchFailure> {
    let trading_date = crate::grpc_contract::params::resolve_date(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    let disclosure_limit =
        crate::grpc_contract::params::resolve_u32(params, "disclosure_limit", 100)
            .map_err(|e| format!("params 无效: {e}"))?;
    let stock_limit =
        crate::grpc_contract::params::resolve_u32(params, "stock_limit", 20)
            .map_err(|e| format!("params 无效: {e}"))?;
    let gateway = DragonTigerGateway::new();
    let batch = gateway
        .market_review(trading_date, disclosure_limit, stock_limit as usize)
        .await
        .map_err(|e| format!("龙虎榜 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "exchange": format!("{:?}", r.exchange),
                "code": r.code,
                "ranking_net_amount_yuan": r.ranking_net_amount_yuan,
                "disclosures": r.disclosures.iter().map(|d| {
                    json!({
                        "entry_id": d.entry_id,
                        "trade_id": d.trade_id,
                        "reason": d.reason,
                        "buy_amount_yuan": d.buy_amount_yuan,
                        "sell_amount_yuan": d.sell_amount_yuan,
                        "net_amount_yuan": d.net_amount_yuan,
                        "turnover_rate_pct": d.turnover_rate_pct,
                        "seats": d.seats.iter().map(|s| {
                            json!({
                                "side": format!("{:?}", s.side),
                                "rank": s.rank,
                                "seat_name": s.seat_name,
                                "amount_yuan": s.amount_yuan,
                                "buy_amount_yuan": s.buy_amount_yuan,
                                "sell_amount_yuan": s.sell_amount_yuan,
                                "net_amount_yuan": s.net_amount_yuan,
                            })
                        }).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    pack(records, source_at)
}

/// 大宗交易: 生产调用方传 codes + trading_date (push_templates r05) —
/// codes 缺省 watchlist, date 缺省今天。
async fn fetch_block_trades(params: &Value) -> Result<Fetched, FetchFailure> {
    let codes = crate::grpc_contract::params::resolve_codes(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    let trading_date = crate::grpc_contract::params::resolve_date(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    let gateway = BlockTradesGateway::new();
    let batch = gateway
        .market_review(&codes, trading_date)
        .await
        .map_err(|e| format!("大宗交易 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "code": r.code,
                "traded_at": r.traded_at,
                "price": r.price,
                "close_price": r.close_price,
                "premium_ratio": r.premium_ratio,
                "volume": r.volume,
                "amount": r.amount,
                "buyer": r.buyer,
                "seller": r.seller,
            })
        })
        .collect();
    pack(records, source_at)
}

/// 一致预期: 生产调用方逐代码 fetch(code) (v17_sources) — codes 缺省 watchlist。
async fn fetch_consensus(params: &Value) -> Result<Fetched, FetchFailure> {
    let gateway = ConsensusDataGateway::new();
    let codes = crate::grpc_contract::params::resolve_codes(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        let gateway = gateway;
        // ConsensusData 记录本身没有 code 字段 (逐代码查询) → 带 code 回传, JSON 里补上。
        set.spawn(async move {
            let batch = gateway
                .fetch(&code)
                .await
                .map_err(|e| format!("一致预期 Gateway 不可用 ({code}): {e}"))?;
            Ok::<_, String>((code, batch))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut source_at = String::new();
    while let Some(joined) = set.join_next().await {
        let (code, batch) = joined.map_err(|e| format!("一致预期 task 失败: {e}"))??;
        if source_at.is_empty() {
            source_at = source_at_of(&batch);
        }
        records.extend(batch.records().iter().map(|r| {
            json!({
                "code": code,
                "report_count": r.report_count,
                "broker_count": r.broker_count,
                "eps_this_year_avg": r.eps_this_year_avg,
                "eps_next_year_avg": r.eps_next_year_avg,
                "eps_next2_year_avg": r.eps_next2_year_avg,
                "rating_distribution": r.rating_distribution,
            })
        }));
    }
    pack(records, source_at)
}

// ---------- Task 10 补全的 11 op (delegate 24 个生产 op 全量覆盖) ----------

/// 日线: 逐代码 daily_bars_async (AdmittedDailyBars, 非 GatewayBatch →
/// source_at 从 evidence 取, 不能走 source_at_of)。
/// P4 M2: 升级收 params (codes 缺省 watchlist; days 缺省 120)。
async fn fetch_historical_bars(params: &Value) -> Result<Fetched, DelegateError> {
    let gateway = HistoricalBarsGateway::new();
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let days = crate::grpc_contract::params::resolve_u32(params, "days", 120)? as usize;
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        let gateway = gateway;
        set.spawn(async move {
            gateway
                .daily_bars_async(&code, days)
                .await
                .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("日线 Gateway 不可用 ({code}): {e}"))))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut evidence_first: Option<crate::data_gateway::BatchEvidence> = None;
    while let Some(joined) = set.join_next().await {
        let batch = joined.map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("日线 task 失败: {e}"))))?;
        let batch = batch?;
        if evidence_first.is_none() {
            evidence_first = Some(batch.evidence().clone());
        }
        let code = batch.target_code().to_string();
        records.extend(batch.records().iter().map(|k| {
            json!({
                "code": code,
                "date": k.date.to_string(),
                "open": k.open,
                "high": k.high,
                "low": k.low,
                "close": k.close,
                "volume": k.volume,
                "amount": k.amount,
                "pct_chg": k.pct_chg,
                "settled": k.settled,
            })
        }));
    }
    // P4 M2: 证据链回填 (AdmittedDailyBars 载体, 手动 pack_ev_from)。
    let ev = evidence_first
        .ok_or_else(|| DelegateError::Fetch(FetchFailure::unknown("日线: 无任何 batch 成功".to_string())))?;
    pack_ev_from(
        records,
        ev.source_at.clone().unwrap_or_default(),
        format!("{:?}", ev.provider),
        ev.source.clone(),
        ev.batch_id.clone(),
    )
    .map_err(DelegateError::Fetch)
}

/// 板块目录: 生产调用方传 kind + limit (push_templates A-11, 200 只) —
/// kind 缺省 Concept, limit 缺省 50 (M1 行为)。
async fn fetch_board_directory(params: &Value) -> Result<Fetched, FetchFailure> {
    let kind_str = crate::grpc_contract::params::resolve_enum_str(
        params,
        "kind",
        &["Industry", "Concept", "Region"],
        "Concept",
    )
    .map_err(|e| format!("params 无效: {e}"))?;
    let kind = match kind_str {
        "Industry" => BoardKind::Industry,
        "Concept" => BoardKind::Concept,
        _ => BoardKind::Region,
    };
    let limit = crate::grpc_contract::params::resolve_u32(params, "limit", 50)
        .map_err(|e| format!("params 无效: {e}"))?;
    let gateway = BoardDataGateway::new();
    let batch = gateway
        .directory(kind, limit)
        .await
        .map_err(|e| format!("板块目录 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "code": r.code,
                "name": r.name,
                "kind": format!("{:?}", r.kind),
                "member_count": r.member_count,
            })
        })
        .collect();
    pack(records, source_at)
}

/// 板块成分: board 模块无公开「板块→成分」生产入口 (board_constituents_raw
/// 需内部 BoardConstituentRequest, 未导出) → 用 memberships(code) 对 watchlist
/// 逐代码查「个股→所属板块」, 输出成分归属视图。
/// 板块成分归属: 生产调用方传板块 code (push_templates A-11 memberships) —
/// codes 缺省 watchlist。
async fn fetch_board_constituents(params: &Value) -> Result<Fetched, FetchFailure> {
    let gateway = BoardDataGateway::new();
    let codes = crate::grpc_contract::params::resolve_codes(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        let gateway = gateway;
        set.spawn(async move {
            gateway
                .memberships(&code)
                .await
                .map_err(|e| format!("板块归属 Gateway 不可用 ({code}): {e}"))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut source_at = String::new();
    while let Some(joined) = set.join_next().await {
        let batch = joined.map_err(|e| format!("板块归属 task 失败: {e}"))??;
        if source_at.is_empty() {
            source_at = source_at_of(&batch);
        }
        records.extend(batch.records().iter().map(|r| {
            json!({
                "instrument_code": r.instrument_code,
                "board_code": r.board_code,
                "board_name": r.board_name,
                "kind": format!("{:?}", r.kind),
            })
        }));
    }
    pack(records, source_at)
}

/// 板块资金流: 生产调用方传 kind + limit (statistics.rs / main.rs
/// day1_flows_blocking, Industry/20) — kind 缺省 Concept, limit 缺省 20。
async fn fetch_board_flows(params: &Value) -> Result<Fetched, FetchFailure> {
    let kind_str = crate::grpc_contract::params::resolve_enum_str(
        params,
        "kind",
        &["Industry", "Concept", "Region"],
        "Concept",
    )
    .map_err(|e| format!("params 无效: {e}"))?;
    let kind = match kind_str {
        "Industry" => BoardKind::Industry,
        "Concept" => BoardKind::Concept,
        _ => BoardKind::Region,
    };
    let limit = crate::grpc_contract::params::resolve_u32(params, "limit", 20)
        .map_err(|e| format!("params 无效: {e}"))?;
    let gateway = BoardDataGateway::new();
    let batch = gateway
        .day1_flows(kind, limit)
        .await
        .map_err(|e| format!("板块资金流 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "code": r.code,
                "name": r.name,
                "kind": format!("{:?}", r.kind),
                "rank": r.rank,
                "return_pct": r.return_pct,
                "main_net_yuan": r.main_net_yuan,
                "leader_code": r.leader_code,
                "leader_name": r.leader_name,
            })
        })
        .collect();
    pack(records, source_at)
}

/// LimitPools/StrongStockReasons/ChainBatch 共用 A-10 题材链 batch (唯一生产入口)。
/// M4c: 加 date 参数 (resolve_date, 默认 today) — monitor 复盘按指定交易日重算。
async fn fetch_chain_batch(
    params: &Value,
) -> Result<crate::database::chain_intelligence::VisibleChainBatch, String> {
    let trading_date = crate::grpc_contract::params::resolve_date(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    ChainIntelligenceGateway::new()
        .build_for_date(trading_date)
        .await
        .map_err(|e| format!("题材链 Gateway 不可用: {e}"))
}

/// M4c: A-10 完整 batch (本地扩展 op 61, schema market.chain_batch v1)。
/// 44/45 视图扁平化后 inputs/版本/rejections 不可重建 — monitor 复盘经此 op
/// 拿完整 VisibleChainBatch (计算+stage+publish 副作用在服务端进程执行,
/// 与 build_for_date 语义一致; 切桥后 A-10 单写方 = 服务端)。
async fn fetch_chain_batch_full(params: &Value) -> Result<Fetched, FetchFailure> {
    let batch = fetch_chain_batch(params).await?;
    let data = serde_json::to_vec(&batch)
        .map_err(|e| FetchFailure::unknown(format!("VisibleChainBatch 序列化失败: {e}")))?;
    Ok(Fetched {
        data,
        source_at: batch.trading_date.to_string(),
        provider: String::new(),
        source: String::new(),
        batch_id: String::new(),
    })
}

/// 涨停池: 全部涨停链成员扁平视图 (含连板 streak)。
async fn fetch_limit_pools(params: &Value) -> Result<Fetched, FetchFailure> {
    let batch = fetch_chain_batch(params).await?;
    let mut records: Vec<Value> = Vec::new();
    for chain in &batch.chains {
        for m in &chain.members {
            records.push(json!({
                "chain_id": chain.chain_id,
                "board_name": chain.board_name,
                "code": m.instrument_id,
                "name": m.security_name,
                "streak": m.streak,
            }));
        }
    }
    pack(records, batch.trading_date.to_string())
}

/// 强势股原因: 涨停链维度 (板块催化 + 涨停数 + 连续板成员)。
async fn fetch_strong_stock_reasons(params: &Value) -> Result<Fetched, FetchFailure> {
    let batch = fetch_chain_batch(params).await?;
    let records: Vec<Value> = batch
        .chains
        .iter()
        .map(|c| {
            json!({
                "chain_id": c.chain_id,
                "board_name": c.board_name,
                "upper_limit_count": c.upper_limit_count,
                "continuous_count": c.continuous_count,
                "members": c
                    .members
                    .iter()
                    .map(|m| {
                        json!({
                            "code": m.instrument_id,
                            "name": m.security_name,
                            "streak": m.streak,
                        })
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    pack(records, batch.trading_date.to_string())
}

/// 全市场龙虎榜: 与 DragonTiger op 共用 market_review (唯一生产入口),
/// 区别仅在 schema 视图。
/// 全市场龙虎榜 (R-04 视图别名): 参数语义与 fetch_dragon_tiger 一致。
async fn fetch_market_dragon_tiger(params: &Value) -> Result<Fetched, FetchFailure> {
    let trading_date = crate::grpc_contract::params::resolve_date(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    let disclosure_limit =
        crate::grpc_contract::params::resolve_u32(params, "disclosure_limit", 100)
            .map_err(|e| format!("params 无效: {e}"))?;
    let stock_limit =
        crate::grpc_contract::params::resolve_u32(params, "stock_limit", 20)
            .map_err(|e| format!("params 无效: {e}"))?;
    let gateway = DragonTigerGateway::new();
    let batch = gateway
        .market_review(trading_date, disclosure_limit, stock_limit as usize)
        .await
        .map_err(|e| format!("龙虎榜 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "exchange": format!("{:?}", r.exchange),
                "code": r.code,
                "ranking_net_amount_yuan": r.ranking_net_amount_yuan,
                "disclosures": r.disclosures.iter().map(|d| {
                    json!({
                        "entry_id": d.entry_id,
                        "trade_id": d.trade_id,
                        "reason": d.reason,
                        "buy_amount_yuan": d.buy_amount_yuan,
                        "sell_amount_yuan": d.sell_amount_yuan,
                        "net_amount_yuan": d.net_amount_yuan,
                        "turnover_rate_pct": d.turnover_rate_pct,
                        "seats": d.seats.iter().map(|s| {
                            json!({
                                "side": format!("{:?}", s.side),
                                "rank": s.rank,
                                "seat_name": s.seat_name,
                                "amount_yuan": s.amount_yuan,
                                "buy_amount_yuan": s.buy_amount_yuan,
                                "sell_amount_yuan": s.sell_amount_yuan,
                                "net_amount_yuan": s.net_amount_yuan,
                            })
                        }).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    pack(records, source_at)
}

/// 板块排行: fetch_top 是同步 reqwest 阻塞调用 → spawn_blocking 隔离,
/// 不卡 tokio worker。
async fn fetch_board_ranking(fid: &str, top_n: usize) -> Result<Fetched, FetchFailure> {
    let fid = fid.to_string();
    let joined = tokio::task::spawn_blocking(move || BoardRankingGateway::new().fetch_top(&fid, top_n))
        .await
        .map_err(|e| format!("板块排行 task 失败: {e}"))?;
    let facts = joined.map_err(|e| format!("板块排行 Gateway 不可用: {e}"))?;
    let records: Vec<Value> = facts
        .iter()
        .map(|r| {
            json!({
                "code": r.code,
                "name": r.name,
                "change_pct": r.change_pct,
                "main_inflow": r.main_inflow,
                "leader_name": r.leader_name,
                "vol_ratio": r.vol_ratio,
                "turnover": r.turnover,
                "day1_ratio": r.day1_ratio,
                "day5_ratio": r.day5_ratio,
            })
        })
        .collect();
    pack(records, String::new())
}

/// 主力净流入排行 (fid=f62): 生产调用方传 top_n (sector_monitor rank_top) —
/// 缺省 20 (M1 行为)。
async fn fetch_market_rankings(params: &Value) -> Result<Fetched, FetchFailure> {
    let top_n = crate::grpc_contract::params::resolve_u32(params, "top_n", 20)
        .map_err(|e| format!("params 无效: {e}"))?;
    fetch_board_ranking("f62", top_n as usize).await
}

/// 概念涨幅榜 (东财概念板块排行 fid=f3): 生产调用方传 top_n
/// (sector_monitor rank_top, push_templates 5/10/30) — 缺省 30 (M1 行为)。
async fn fetch_concept_hits(params: &Value) -> Result<Fetched, FetchFailure> {
    let top_n = crate::grpc_contract::params::resolve_u32(params, "top_n", 30)
        .map_err(|e| format!("params 无效: {e}"))?;
    fetch_board_ranking("f3", top_n as usize).await
}

/// 研报: 逐代码 instrument_reports (记录无 code 字段 → 带 code 回传)。
/// 研报: codes 缺省 watchlist, page_size 缺省 5 (M1 行为; agent 工具传 20)。
async fn fetch_research_reports(params: &Value) -> Result<Fetched, FetchFailure> {
    let codes = crate::grpc_contract::params::resolve_codes(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    let page_size = crate::grpc_contract::params::resolve_u32(params, "page_size", 5)
        .map_err(|e| format!("params 无效: {e}"))?;
    let gateway = ResearchDataGateway::new();
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        let gateway = gateway;
        set.spawn(async move {
            let batch = gateway
                .instrument_reports(&code, page_size)
                .await
                .map_err(|e| format!("研报 Gateway 不可用 ({code}): {e}"))?;
            Ok::<_, String>((code, batch))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut source_at = String::new();
    while let Some(joined) = set.join_next().await {
        let (code, batch) = joined.map_err(|e| format!("研报 task 失败: {e}"))??;
        if source_at.is_empty() {
            source_at = source_at_of(&batch);
        }
        records.extend(batch.records().iter().map(|r| {
            json!({
                "code": code,
                "report_id": r.report_id,
                "title": r.title,
                "organization": r.organization,
                "rating": r.rating,
                "published_at": r.published_at,
                "canonical_url": r.canonical_url,
                "target_price_upper": r.source_target_price_upper,
                "target_price_lower": r.source_target_price_lower,
            })
        }));
    }
    pack(records, source_at)
}

/// 北向资金: 沪股通 + 深股通 两 channel 并发 (逐 channel 查询)。
/// 北向日数据: P4 M4b 升级 — 收 date + channel params (默认今天/Shanghai),
/// 与客户端桥 CapitalDataGateway::northbound_daily(trading_date, channel) 对齐
/// (此前固定 today()+双 channel 合流, 无法满足指定日期/单通道请求)。
async fn fetch_northbound_daily(params: &Value) -> Result<Fetched, FetchFailure> {
    let date = crate::grpc_contract::params::resolve_date(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    let channel = match crate::grpc_contract::params::resolve_enum_str(
        params,
        "channel",
        &["Shanghai", "Shenzhen"],
        "Shanghai",
    )
    .map_err(|e| format!("params 无效: {e}"))?
    {
        "Shenzhen" => NorthboundChannel::Shenzhen,
        _ => NorthboundChannel::Shanghai,
    };
    let gateway = CapitalDataGateway::new();
    let batch = gateway
        .northbound_daily(date, channel)
        .await
        .map_err(|e| format!("北向资金 Gateway 不可用: {e}"))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "trading_date": r.trading_date.to_string(),
                "channel": format!("{:?}", r.channel),
                "total_turnover": r.total_turnover,
                "total_trade_count": r.total_trade_count,
                "quota_balance": match r.quota_balance {
                    NorthboundQuotaFact::Amount(v) => json!(v),
                    NorthboundQuotaFact::Unavailable => json!("unavailable"),
                },
                "etf_turnover": r.etf_turnover,
                "top_turnover": r
                    .top_turnover
                    .iter()
                    .map(|t| {
                        json!({
                            "rank": t.rank,
                            "code": t.code,
                            "name": t.name,
                            "total_turnover": t.total_turnover,
                        })
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    pack(records, source_at_of(&batch))
}

// ---------- M1 扩展 (P4): 8 个 proto 已有 op ----------

/// 外汇 (USD/CNY): GlobalMarketGateway::usd_cny 是唯一外汇入口 (上游无参数化
/// forex 方法, P4 探索确认) → params 仅留兼容位, 固定 USD/CNY。
async fn fetch_foreign_exchange(params: &Value) -> Result<Fetched, DelegateError> {
    let _ = params;
    let batch = GlobalMarketGateway::new()
        .usd_cny()
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("外汇 Gateway 不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "pair": format!("{:?}", r.pair),
                "name": r.name,
                "rate": r.rate,
                "change": r.change,
                "change_percent": r.change_percent,
                "source_at": r.source_at.to_rfc3339(),
            })
        })
        .collect();
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}

/// 财务报告: CompanyDataGateway::financial_statements (kind 必填
/// balance/income/cash_flow)。FinancialStatement 已 derive Serialize
/// (magic_market_core) → serde_json::to_value 直出 (P4 hybrid wire 策略)。
async fn fetch_financial_statements(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let kind_str = crate::grpc_contract::params::resolve_required_string(params, "kind")?;
    let kind = match kind_str.as_str() {
        "balance" => StatementKind::Balance,
        "income" => StatementKind::Income,
        "cash_flow" => StatementKind::CashFlow,
        other => {
            return Err(DelegateError::Params(crate::grpc_contract::params::ParamsError::InvalidArgument(
                format!("kind 非法值 {other:?} (允许: balance/income/cash_flow)"),
            )))
        }
    };
    let batch = CompanyDataGateway::new()
        .financial_statements(&codes, kind)
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("财务报告 Gateway 不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| serde_json::to_value(r).map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string()))))
        .collect::<Result<_, _>>()?;
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}

/// 市场统计: CompanyDataGateway::market_statistics (访问器-only record →
/// 逐字段; 缺失字段保持 null, 不填零)。
async fn fetch_market_statistics(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let batch = CompanyDataGateway::new()
        .market_statistics(&codes)
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("市场统计 Gateway 不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "code": r.instrument().code(),
                "turnover_rate": r.turnover_rate().map(|v| v.get()),
                "trailing_pe": r.trailing_pe().map(|v| v.get()),
                "static_pe": r.static_pe().map(|v| v.get()),
                "pb": r.pb().map(|v| v.get()),
                "volume_ratio": r.volume_ratio().map(|v| v.get()),
                "total_market_cap": r.total_market_cap().map(|v| v.get()),
                "floating_market_cap": r.floating_market_cap().map(|v| v.get()),
                "upper_limit": r.upper_limit().map(|v| v.get()),
                "lower_limit": r.lower_limit().map(|v| v.get()),
            })
        })
        .collect();
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}

/// 技术 K 线: data_gateway 无技术指标引擎 (P4 探索) → 15 分钟 K 线
/// (HistoricalBarsGateway::fifteen_min_bars, 同步阻塞) 作为 TechnicalBars 视图;
/// count 默认 48 (3 个交易日)。fifteen_min_bars 返回 Vec 非 GatewayBatch →
/// source_at/证据留空 (合同 §6 缺则不填充)。
async fn fetch_technical_bars(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let count = crate::grpc_contract::params::resolve_u32(params, "count", 48)? as usize;
    let gateway = HistoricalBarsGateway::new();
    let mut records: Vec<Value> = Vec::new();
    for code in codes {
        let bars = gateway
            .fifteen_min_bars(&code, count)
            .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("15 分钟线 Gateway 不可用 ({code}): {e}"))))?;
        records.extend(bars.iter().map(|b| {
            json!({
                "code": code,
                "open": b.open,
                "close": b.close,
                "high": b.high,
                "low": b.low,
                "vol": b.vol,
                "amount": b.amount,
                "at": b.datetime,
            })
        }));
    }
    pack(records, String::new()).map_err(DelegateError::Fetch)
}

/// 公司行动: SecurityLifecycleGateway::acquire 返回生命周期上下文, 取
/// corporate_actions 部分 (无独立公开入口, P4 探索确认)。
/// window_start/window_end 必填 (与生命周期合同一致)。
async fn fetch_corporate_actions(params: &Value) -> Result<Fetched, DelegateError> {
    let code = crate::grpc_contract::params::resolve_required_string(params, "code")?;
    let window_start = crate::grpc_contract::params::resolve_required_date(params, "window_start")?;
    let window_end = crate::grpc_contract::params::resolve_required_date(params, "window_end")?;
    let ctx = crate::data_gateway::security_lifecycle::SecurityLifecycleGateway::new()
        .acquire(&code, window_start, window_end)
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("公司行动 Gateway 不可用: {e}"))))?;
    let state = ctx.corporate_actions;
    let mut records: Vec<Value> = Vec::new();
    for r in state.records() {
        let terms = serde_json::to_value(&r.terms)
            .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("terms 序列化失败: {e}"))))?;
        records.push(json!({
            "code": r.code,
            "category": format!("{:?}", r.category),
            "effective_on": r.effective_on.to_string(),
            "record_on": r.record_on.map(|d| d.to_string()),
            "ex_on": r.ex_on.map(|d| d.to_string()),
            "payable_on": r.payable_on.map(|d| d.to_string()),
            "terms": terms,
        }));
    }
    // SecurityLifecycleContext 非 GatewayBatch → evidence 从 CorporateActionState 取。
    let evidence = state.evidence();
    Ok(Fetched {
        data: serde_json::to_vec(&records).map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string())))?,
        source_at: evidence
            .map(|e| e.source_at.clone().unwrap_or_default())
            .unwrap_or_default(),
        provider: evidence.map(|e| format!("{:?}", e.provider)).unwrap_or_default(),
        source: evidence.map(|e| e.source.clone()).unwrap_or_default(),
        batch_id: evidence.map(|e| e.batch_id.clone()).unwrap_or_default(),
    })
}

/// 语义检索: data_gateway 无向量检索 (P4 探索) → 联网检索 GeneralWebResearchGateway
/// (Bocha/Tavily/SerpApi, 需 API key; 无 key 显式失败, 不静默回退)。
async fn fetch_semantic_search(params: &Value) -> Result<Fetched, DelegateError> {
    let query = crate::grpc_contract::params::resolve_required_string(params, "query")?;
    let limit = crate::grpc_contract::params::resolve_u32(params, "limit", 10)? as usize;
    let batch = GeneralWebResearchGateway::from_environment(GeneralWebResearchProvider::Bocha)
        .search(&query, limit)
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("联网检索不可用: {e}"))))?;
    // GeneralWebResearchBatch 只有 evidence() 访问器; records 在 enum 变体字段。
    let records: Vec<Value> = match &batch {
        crate::data_gateway::GeneralWebResearchBatch::Available { records, .. } => records
            .iter()
            .map(|r| serde_json::to_value(r).map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string()))))
            .collect::<Result<_, _>>()?,
        crate::data_gateway::GeneralWebResearchBatch::VerifiedEmpty(_) => Vec::new(),
    };
    let ev = batch.evidence();
    Ok(Fetched {
        data: serde_json::to_vec(&records).map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string())))?,
        source_at: String::new(),
        provider: format!("{:?}", ev.provider),
        source: ev.source.clone(),
        batch_id: ev.batch_id.clone(),
    })
}

/// 资金流序列: CapitalDataGateway::instrument_fund_flow (interval 仅
/// minute1/day1 受网关支持, P4 探索确认; limit 默认 20)。
async fn fetch_fund_flow_series(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let interval = match crate::grpc_contract::params::resolve_enum_str(
        params,
        "interval",
        &["minute1", "day1"],
        "day1",
    )? {
        "minute1" => FlowInterval::Minute1,
        _ => FlowInterval::Day1,
    };
    let limit = crate::grpc_contract::params::resolve_u32(params, "limit", 20)?;
    let gateway = CapitalDataGateway::new();
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        let gateway = gateway;
        set.spawn(async move {
            gateway
                .instrument_fund_flow(&code, interval, limit)
                .await
                .map_err(|e| FetchFailure::unknown(format!("资金流 Gateway 不可用 ({code}): {e}")))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut source_at = String::new();
    while let Some(joined) = set.join_next().await {
        let batch = joined
            .map_err(|e| FetchFailure::unknown(format!("资金流 task 失败: {e}")))?
            .map_err(DelegateError::Fetch)?;
        if source_at.is_empty() {
            source_at = source_at_of(&batch);
        }
        records.extend(batch.records().iter().map(|r| {
            json!({
                "code": r.code,
                "interval": format!("{:?}", r.interval),
                "period_at": r.period_at,
                "main_net": r.main_net,
                "main_ratio_percent": r.main_ratio_percent,
                "super_large_net": r.super_large_net,
                "large_net": r.large_net,
                "medium_net": r.medium_net,
                "small_net": r.small_net,
            })
        }));
    }
    pack(records, source_at).map_err(DelegateError::Fetch)
}

/// 头部排行: CapitalDataGateway::provider_top_n_pair (VolumeRatio + MainNetInflow
/// 原子对, limit 固定 20)。BR-198: 同日请求须 15:35 后合格 (上游约束, 原样传递)。
async fn fetch_provider_top_n_rankings(params: &Value) -> Result<Fetched, DelegateError> {
    let date = crate::grpc_contract::params::resolve_date(params)?;
    let pair = CapitalDataGateway::new()
        .provider_top_n_pair(date)
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("头部排行 Gateway 不可用: {e}"))))?;
    let mut records: Vec<Value> = Vec::new();
    for batch in [&pair.volume_ratio, &pair.main_net_inflow] {
        for r in batch.records() {
            records.push(json!({
                "metric": format!("{:?}", r.metric),
                "ordinal": r.source_order_ordinal.get(),
                "code": r.instrument.code(),
                "label": r.label.as_str(),
                "value": r.value.get(),
                "unit": format!("{:?}", r.unit),
                "trading_date": r.trading_date.as_str(),
                "filter_identity": r.filter_identity.as_str(),
                "provider_declared_total": r.provider_declared_total.get(),
                "inspected_row_count": r.inspected_row_count.get(),
            }));
        }
    }
    // ProviderTopNPair 非 GatewayBatch → evidence 取第一路 (volume_ratio)。
    pack_ev(records, &pair.volume_ratio).map_err(DelegateError::Fetch)
}

// ---------- M1 扩展 (P4): 6 个新 op (proto 编号 55-60) ----------

/// 指数实时行情: IndexDataGateway::realtime_quotes (同步阻塞, 直接调用)。
/// 缺省 codes = 6 大指数 (params::MAIN_INDICES, 来源 market_analyzer 私有常量)。
async fn fetch_index_quotes(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let codes = if codes.is_empty() {
        crate::grpc_contract::params::MAIN_INDICES
            .iter()
            .map(|(c, _)| c.to_string())
            .collect()
    } else {
        codes
    };
    let batch = IndexDataGateway::new()
        .realtime_quotes(&codes)
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("指数行情 Gateway 不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "code": r.code,
                "name": r.name,
                "current": r.current,
                "change": r.change,
                "change_percent": r.change_percent,
                "open": r.open,
                "high": r.high,
                "low": r.low,
                "previous_close": r.previous_close,
                "volume": r.volume,
                "amount": r.amount,
                "source_at": r.source_at.to_rfc3339(),
            })
        })
        .collect();
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}

/// 个股新闻: SinaInstrumentNewsGateway::instrument_news_in_range (from = now -
/// from_days, 默认 30, 与 monitor post_close_news_review 惯例一致)。
async fn fetch_instrument_news(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let from_days = crate::grpc_contract::params::resolve_u32(params, "from_days", 30)?;
    let now = chrono::Utc::now();
    let from = now - chrono::Duration::days(from_days as i64);
    let gateway = SinaInstrumentNewsGateway::new();
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        let gateway = gateway;
        set.spawn(async move {
            gateway
                .instrument_news_in_range(&code, from, now)
                .await
                .map_err(|e| FetchFailure::unknown(format!("个股新闻 Gateway 不可用 ({code}): {e}")))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut source_at = String::new();
    while let Some(joined) = set.join_next().await {
        let batch = joined
            .map_err(|e| FetchFailure::unknown(format!("个股新闻 task 失败: {e}")))?
            .map_err(DelegateError::Fetch)?;
        if source_at.is_empty() {
            source_at = source_at_of(&batch);
        }
        records.extend(batch.records().iter().map(|r| {
            let item = r.persistence_item();
            json!({
                "code": item.code,
                "title": item.title,
                "summary": item.summary,
                "url": item.url,
                "source_name": item.source_name,
                "published_at": item.published_at.to_rfc3339(),
                "source": item.source,
                "external_id": item.external_id,
                "category": item.category,
                "fetched_at": item.fetched_at.to_rfc3339(),
                "content_hash": item.content_hash,
            })
        }));
    }
    pack(records, source_at).map_err(DelegateError::Fetch)
}

/// 日内形态: IntradayShapeGateway::current_shape (逐代码, async)。
async fn fetch_intraday_shape(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let gateway = IntradayShapeGateway::new();
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        let gateway = gateway;
        set.spawn(async move {
            gateway
                .current_shape(&code)
                .await
                .map_err(|e| FetchFailure::unknown(format!("日内形态 Gateway 不可用 ({code}): {e}")))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut source_at = String::new();
    while let Some(joined) = set.join_next().await {
        let batch = joined
            .map_err(|e| FetchFailure::unknown(format!("日内形态 task 失败: {e}")))?
            .map_err(DelegateError::Fetch)?;
        if source_at.is_empty() {
            source_at = source_at_of(&batch);
        }
        records.extend(batch.records().iter().map(|r| {
            json!({
                "date": r.date,
                "pre_close": r.pre_close,
                "open_pct": r.open_pct,
                "high_pct": r.high_pct,
                "low_pct": r.low_pct,
                "close_pct": r.close_pct,
                "amplitude": r.amplitude,
                "tail_30m_pct": r.tail_30m_pct,
                "shape_label": r.shape_label,
            })
        }));
    }
    pack(records, source_at).map_err(DelegateError::Fetch)
}

/// T0 证据: MagicTdxGateway::get_t0_evidence_batch (同步阻塞, 直接调用)。
/// MagicTdxT0Evidence/Rejection 已 derive Serialize → to_value 直出。
async fn fetch_t0_evidence(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let batch = MagicTdxGateway::new()
        .get_t0_evidence_batch(&codes, chrono::Utc::now())
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("T0 证据不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records
        .iter()
        .map(|r| serde_json::to_value(r).map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string()))))
        .collect::<Result<_, _>>()?;
    let rejections: Vec<Value> = batch
        .rejections
        .iter()
        .map(|r| serde_json::to_value(r).map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string()))))
        .collect::<Result<_, _>>()?;
    let view = json!({ "records": records, "rejections": rejections });
    Ok(Fetched {
        data: serde_json::to_vec(&view).map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string())))?,
        source_at: batch.source_at.to_rfc3339(),
        provider: String::new(),
        source: String::new(),
        batch_id: batch.batch_id,
    })
}

/// 复盘 outcome 日线 (P4 M3 transport seam): 服务端在 spawn_blocking 内执行
/// 自适应抓取 (fetch_magic_tdx_outcome_adaptive — claim 台账/audit 始终留客户端,
/// 这里只迁移 provider transport)。视图 = {"batch": to_value(DataBatch<Bar>),
/// "attempts": to_value(Preimage), "error": {...}|null}; error 分支保留完整
/// attempts, 客户端 convert::outcome_daily_bars 重建 OutcomeTransportFailure。
/// 参数: code/market/expected_bar_count/maximum_latest_n/window_start/instrument
/// (InstrumentId 对象 round-trip)。
async fn fetch_outcome_daily_bars(params: &Value) -> Result<Fetched, DelegateError> {
    let code = crate::grpc_contract::params::resolve_required_string(params, "code")?;
    let market = crate::grpc_contract::params::resolve_required_string(params, "market")?;
    let expected_bar_count = params
        .get("expected_bar_count")
        .and_then(Value::as_u64)
        .map(|v| v as u16)
        .ok_or_else(|| {
            DelegateError::Params(crate::grpc_contract::params::ParamsError::InvalidArgument(
                "outcome_daily_bars: expected_bar_count 必填".into(),
            ))
        })?;
    let maximum_latest_n = params
        .get("maximum_latest_n")
        .and_then(Value::as_u64)
        .map(|v| v as u16)
        .ok_or_else(|| {
            DelegateError::Params(crate::grpc_contract::params::ParamsError::InvalidArgument(
                "outcome_daily_bars: maximum_latest_n 必填".into(),
            ))
        })?;
    let window_start = crate::grpc_contract::params::resolve_required_date(params, "window_start")?;
    let instrument: InstrumentId = serde_json::from_value(
        params
            .get("instrument")
            .cloned()
            .ok_or_else(|| {
                DelegateError::Params(
                    crate::grpc_contract::params::ParamsError::InvalidArgument(
                        "outcome_daily_bars: instrument 必填".into(),
                    ),
                )
            })?,
    )
    .map_err(|e| {
        DelegateError::Params(crate::grpc_contract::params::ParamsError::InvalidArgument(
            format!("outcome_daily_bars: instrument 非法: {e}"),
        ))
    })?;

    let result = tokio::task::spawn_blocking(move || {
        fetch_magic_tdx_outcome_adaptive(
            instrument,
            market,
            code,
            expected_bar_count,
            maximum_latest_n,
            window_start,
        )
    })
    .await
    .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("outcome_daily_bars worker join 失败: {e}"))))?;

    match result {
        Ok(RawOutcomeFetch { batch, attempts }) => {
            let batch_json = serde_json::to_value(&batch)
                .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("outcome batch 序列化失败: {e}"))))?;
            let attempts_json = serde_json::to_value(&attempts).map_err(|e| {
                DelegateError::Fetch(FetchFailure::unknown(format!("outcome attempts 序列化失败: {e}")))
            })?;
            let provenance = batch.provenance();
            Ok(Fetched {
                data: serde_json::to_vec(&json!({
                    "batch": batch_json,
                    "attempts": attempts_json,
                    "error": Value::Null,
                }))
                .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string())))?,
                source_at: provenance.source_at().unwrap_or_default().to_string(),
                provider: format!("{:?}", ProviderId::Tdx),
                source: provenance.source().to_string(),
                batch_id: provenance.batch_id().unwrap_or_default().to_string(),
            })
        }
        Err(OutcomeTransportFailure { error, attempts }) => {
            let attempts_json = serde_json::to_value(&attempts).map_err(|e| {
                DelegateError::Fetch(FetchFailure::unknown(format!("outcome attempts 序列化失败: {e}")))
            })?;
            Ok(Fetched {
                data: serde_json::to_vec(&json!({
                    "batch": Value::Null,
                    "attempts": attempts_json,
                    "error": {
                        "capability": error.capability(),
                        "provider": error.provider().map(|p| format!("{:?}", p)),
                        "audit_outcome": error.audit_outcome(),
                        "reason_code": error.reason_code(),
                        "retryable": error.retryable(),
                        "message": error.message(),
                    },
                }))
                .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string())))?,
                source_at: String::new(),
                provider: format!("{:?}", ProviderId::Tdx),
                source: String::new(),
                batch_id: String::new(),
            })
        }
    }
}

/// 涨停池复盘: ReviewDataGateway::r03_upper_limit_pool (date 默认今天)。
async fn fetch_upper_limit_pool_review(params: &Value) -> Result<Fetched, DelegateError> {
    let date = crate::grpc_contract::params::resolve_date(params)?;
    let batch = ReviewDataGateway::new()
        .r03_upper_limit_pool(date)
        .await
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("涨停池复盘 Gateway 不可用: {e}"))))?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "code": r.code,
                "trading_date": r.trading_date.to_string(),
                "theme": r.theme,
                "streak": r.streak,
            })
        })
        .collect();
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}
