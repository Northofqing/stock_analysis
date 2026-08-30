//! data_gateway 委托层 (方案 A): 服务端进程内调用 data_gateway 取真实数据,
//! 序列化为 canonical JSON。fixture_mode 下不经过这里。
//! 每个 op 一个 fetch_xxx(params: &Value) -> Result<Fetched, DelegateError> (M1 起;
//! 既有 24 个 fetch 保持无参签名 → dispatch 里包 DelegateError::Fetch)。
//!
//! 签名说明 (实测, 非计划假设): data_gateway 全部是 async fn (内部自行
//! spawn_blocking), 所以 fetch 本身是 async, handler 直接 await, 不套 spawn_blocking。
//! 记录结构体没有 derive Serialize → 逐字段 json! 映射 (字段名 = 结构体字段名);
//! M1 扩展的 14 个 fetch 中，已 derive Serialize 的 record
//! (FinancialStatement/GeneralWebResearchRecord) 用 serde_json::to_value 直出；
//! T0 v2 批次由 t0_wire 的显式 DTO 序列化，固定投影中国时段 civil label。
use crate::data_gateway::outcome_daily_bars::{
    fetch_magic_tdx_outcome_adaptive, OutcomeTransportFailure, RawOutcomeFetch,
};
use crate::data_gateway::{
    company::CompanyDataGateway,
    general_web_research::{GeneralWebResearchGateway, GeneralWebResearchProvider},
};
use crate::data_gateway::{
    BlockTradesGateway, BoardDataGateway, BoardKind, CapitalDataGateway, ChainIntelligenceGateway,
    ConsensusDataGateway, DragonTigerGateway, EconomicCalendarGateway, EventCalendarGateway,
    FuturesDeliveryGateway, GlobalMarketGateway, GlobalNewsGateway, GlobalNewsProvider,
    HistoricalBarsGateway, IndexDataGateway, IntradayShapeGateway, MagicTdxGateway,
    MarketCapabilitiesGateway, NorthboundQuotaFact, ResearchDataGateway, ReviewDataGateway,
    SinaInstrumentNewsGateway,
};
use crate::grpc_client::pb::magic::market::v1::Operation;
use crate::market_domain::{FlowInterval, NorthboundChannel, StatementKind};
use crate::market_domain::{InstrumentId, LimitPoolEntry, LimitPoolKind, ProviderId};
use chrono::{Datelike, Local, NaiveDate};
use serde_json::{json, Value};

/// 委托取数结果。所有字段均来自实际 Gateway 批次；handler 只校验并透传，
/// 不得生成 provider/source/batch/time 来掩盖缺失证据。
pub struct Fetched {
    pub data: Vec<u8>,
    pub source_at: String,
    pub observed_at: String,
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

    /// 覆盖 message 保留调用方上下文 (分类字段不动 — from_gateway 后调用)。
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
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
    BenchmarkFetch {
        failure: FetchFailure,
        audit_outcome: &'static str,
        audit_state: crate::data_gateway::grpc_source::BenchmarkServerAuditState,
    },
}

impl std::fmt::Display for DelegateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DelegateError::Params(e) => write!(f, "{e}"),
            DelegateError::Fetch(failure) => write!(f, "取数失败: {}", failure.message),
            DelegateError::BenchmarkFetch { failure, .. } => {
                write!(f, "历史基准取数失败: {}", failure.message)
            }
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

/// GatewayBatch 打包: evidence 证据链 5 字段全回填 (M1 新 fetch 用)。
fn pack_ev(
    records: Vec<Value>,
    batch: &crate::data_gateway::GatewayBatch<impl Sized>,
) -> Result<Fetched, FetchFailure> {
    let ev = batch.evidence();
    pack_ev_from(
        records,
        ev.source_at.clone().unwrap_or_default(),
        ev.observed_at.clone(),
        format!("{:?}", ev.provider),
        ev.source.clone(),
        ev.batch_id.clone(),
    )
}

/// P4 M2: 通用证据回填打包 (非 GatewayBatch 载体如 AdmittedDailyBars 手动调用)。
fn pack_ev_from(
    records: Vec<Value>,
    source_at: String,
    observed_at: String,
    provider: String,
    source: String,
    batch_id: String,
) -> Result<Fetched, FetchFailure> {
    Ok(Fetched {
        data: serde_json::to_vec(&records).map_err(|e| FetchFailure::unknown(e.to_string()))?,
        source_at,
        observed_at,
        provider,
        source,
        batch_id,
    })
}

fn retain_single_batch_evidence(
    retained: &mut Option<crate::data_gateway::BatchEvidence>,
    batch: &crate::data_gateway::GatewayBatch<impl Sized>,
) -> Result<(), FetchFailure> {
    retain_batch_evidence(retained, batch.evidence())
}

fn retain_batch_evidence(
    retained: &mut Option<crate::data_gateway::BatchEvidence>,
    evidence: &crate::data_gateway::BatchEvidence,
) -> Result<(), FetchFailure> {
    if let Some(existing) = retained.as_ref() {
        if existing != evidence {
            return Err(FetchFailure {
                message: "one QueryResponse cannot merge distinct provider batch identities"
                    .to_string(),
                provider: None,
                reason_code: "invalid_evidence",
                retryable: false,
            });
        }
    } else {
        *retained = Some(evidence.clone());
    }
    Ok(())
}

fn pack_retained_evidence(
    records: Vec<Value>,
    evidence: Option<crate::data_gateway::BatchEvidence>,
) -> Result<Fetched, FetchFailure> {
    let evidence = evidence.ok_or_else(|| FetchFailure {
        message: "delegate returned no source-backed batch evidence".to_string(),
        provider: None,
        reason_code: "no_verified_batch",
        retryable: true,
    })?;
    pack_ev_from(
        records,
        evidence.source_at.unwrap_or_default(),
        evidence.observed_at,
        format!("{:?}", evidence.provider),
        evidence.source,
        evidence.batch_id,
    )
}

fn pack_benchmark_audited(
    request: &crate::data_gateway::BenchmarkRequest,
    audited: &crate::data_gateway::review::AuditedBenchmarkBatch,
) -> Result<Fetched, FetchFailure> {
    let wire =
        crate::data_gateway::grpc_source::BenchmarkGrpcResponseWire::from_audited(request, audited)
            .map_err(FetchFailure::from_gateway)?;
    let evidence = audited.batch.evidence();
    Ok(Fetched {
        data: serde_json::to_vec(&wire).map_err(|error| {
            FetchFailure::unknown(format!("BenchmarkBars wire serialize: {error}"))
        })?,
        // Proto outer evidence has no presence bit: empty is the sole absence encoding.
        // The canonical JSON wire retains explicit null and the client cross-check maps
        // this empty scalar back to None; it is never substituted with a bar/observed time.
        source_at: evidence.source_at.clone().unwrap_or_default(),
        observed_at: evidence.observed_at.clone(),
        provider: format!("{:?}", evidence.provider),
        source: evidence.source.clone(),
        batch_id: evidence.batch_id.clone(),
    })
}

fn resolve_benchmark_request(
    params: &Value,
) -> Result<crate::data_gateway::BenchmarkRequest, DelegateError> {
    let wire: crate::data_gateway::grpc_source::BenchmarkRequestWire =
        serde_json::from_value(params.clone()).map_err(|error| {
            crate::grpc_contract::params::ParamsError::InvalidArgument(format!(
                "BenchmarkBars request wire 非法: {error}"
            ))
        })?;
    wire.to_request().map_err(|error| {
        crate::grpc_contract::params::ParamsError::InvalidArgument(format!(
            "BenchmarkBars request identity/range 非法: {}",
            error.message()
        ))
        .into()
    })
}

async fn fetch_benchmark_bars(params: &Value) -> Result<Fetched, DelegateError> {
    let request = resolve_benchmark_request(params)?;
    let audited = ReviewDataGateway::new()
        .benchmark_bars_library_for_grpc(request.clone())
        .await
        .map_err(|library_failure| {
            let (error, audit_state) = library_failure.into_parts();
            let message = format!("历史基准 Gateway 不可用: {error}");
            let audit_outcome = error.audit_outcome();
            DelegateError::BenchmarkFetch {
                failure: FetchFailure::from_gateway(error).with_message(message),
                audit_outcome,
                audit_state,
            }
        })?;
    pack_benchmark_audited(&request, &audited).map_err(DelegateError::Fetch)
}

fn not_yet(op: Operation) -> Result<Fetched, FetchFailure> {
    Err(FetchFailure::unknown(format!(
        "{}: delegate 尚未实现 (Task 10 补全)",
        crate::grpc_contract::ops::method_name(op)
    )))
}

/// 统一取数入口 (M1: 新 14 个 fetch 收 params; 既有 24 个签名不变 → 显式 codes
/// 对它们尚不生效, M2 客户端桥接入时逐个升级)。
pub async fn fetch(op: Operation, _schema: &str, params: &Value) -> Result<Fetched, DelegateError> {
    match op {
        // P4 M2: 首批 6 op 升级收 params (客户端桥按 codes/days 精确请求)。
        Operation::RealtimeQuotes => fetch_realtime_quotes(params).await,
        Operation::HistoricalBars => fetch_historical_bars(params).await,
        Operation::MinuteData => fetch_minute_data(params).await,
        Operation::OrderBooks => fetch_order_books(params).await,
        Operation::MoneyFlows => fetch_money_flows(params).await,
        Operation::SecurityMetadata => fetch_security_metadata(params).await,
        Operation::GlobalIndices => fetch_global_indices().await.map_err(DelegateError::Fetch),
        Operation::Announcements => fetch_market_announcements()
            .await
            .map_err(DelegateError::Fetch),
        Operation::GlobalNews => fetch_global_news(params).await,
        Operation::EconomicCalendar => fetch_economic_calendar()
            .await
            .map_err(DelegateError::Fetch),
        Operation::FuturesDelivery => fetch_futures_delivery().await.map_err(DelegateError::Fetch),
        Operation::DragonTiger => fetch_dragon_tiger(params)
            .await
            .map_err(DelegateError::Fetch),
        Operation::BlockTrades => fetch_block_trades(params)
            .await
            .map_err(DelegateError::Fetch),
        Operation::Consensus => fetch_consensus(params).await.map_err(DelegateError::Fetch),
        Operation::BoardDirectory => fetch_board_directory(params)
            .await
            .map_err(DelegateError::Fetch),
        Operation::BoardConstituents => fetch_board_constituents(params).await,
        Operation::BoardFlows => fetch_board_flows(params)
            .await
            .map_err(DelegateError::Fetch),
        Operation::LimitPools => fetch_limit_pools(params).await,
        Operation::StrongStockReasons => fetch_strong_stock_reasons(params)
            .await
            .map_err(DelegateError::Fetch),
        Operation::MarketDragonTiger => fetch_market_dragon_tiger(params)
            .await
            .map_err(DelegateError::Fetch),
        Operation::MarketRankings => fetch_market_rankings(params)
            .await
            .map_err(DelegateError::Fetch),
        Operation::ConceptHits => fetch_concept_hits(params)
            .await
            .map_err(DelegateError::Fetch),
        Operation::ResearchReports => fetch_research_reports(params)
            .await
            .map_err(DelegateError::Fetch),
        Operation::NorthboundDaily => fetch_northbound_daily(params)
            .await
            .map_err(DelegateError::Fetch),
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
        // BR-251: server 必须调用 library seam，避免回到自身 gRPC 桥递归。
        Operation::BenchmarkBars => fetch_benchmark_bars(params).await,
        // M4c 扩展 (P4): A-10 完整 batch (本地扩展 61, monitor 复盘消费)。
        Operation::ChainBatch => fetch_chain_batch_full(params)
            .await
            .map_err(DelegateError::Fetch),
        _ => not_yet(op).map_err(DelegateError::Fetch),
    }
}

// ---------- 统一实时行情 (Task 8 已落地, spawn_blocking 路径) ----------

/// 字段映射以实际 struct 为准: RealtimeMarketQuote 有
/// code/name/price/previous_close/change_percent (无 volume/amount)。
/// P4 M2: 升级收 params (codes 缺省 watchlist)。
///
/// 同步 Gateway 调用必须跑在 spawn_blocking 上: TDX 连接卡顿时整条
/// provider chain 可耗时 20s+, 若直接执行会阻塞 tokio worker, 导致
/// monitor 桥请求排队超时 (BR-243 盘中卡窗根因)。
pub async fn fetch_realtime_quotes(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let batch = tokio::task::spawn_blocking(move || {
        crate::data_gateway::MarketDataGateway::new().realtime_quotes(&codes)
    })
    .await
    .map_err(|error| {
        DelegateError::Fetch(FetchFailure::unknown(format!(
            "统一实时行情 task 失败: {error}"
        )))
    })?
    .map_err(|e| {
        let message = format!("统一实时行情 Gateway 不可用: {e}");
        DelegateError::Fetch(FetchFailure::from_gateway(e).with_message(message))
    })?;
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
        set.spawn(async move {
            gateway.minute_data(&code, None).await.map_err(|e| {
                DelegateError::Fetch(FetchFailure::unknown(format!(
                    "分钟线 Gateway 不可用 ({code}): {e}"
                )))
            })
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut evidence_first: Option<crate::data_gateway::BatchEvidence> = None;
    while let Some(joined) = set.join_next().await {
        let batch = joined.map_err(|e| {
            DelegateError::Fetch(FetchFailure::unknown(format!("分钟线 task 失败: {e}")))
        })?;
        let batch = batch?;
        retain_batch_evidence(&mut evidence_first, batch.evidence())
            .map_err(DelegateError::Fetch)?;
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
    // P4 M2: 一个 QueryResponse 只能绑定一个真实批次身份；跨批聚合显式拒绝。
    let ev = evidence_first.ok_or_else(|| {
        DelegateError::Fetch(FetchFailure::unknown(
            "分钟线: 无任何 batch 成功".to_string(),
        ))
    })?;
    pack_ev_from(
        records,
        ev.source_at.clone().unwrap_or_default(),
        ev.observed_at.clone(),
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
    let batch = gateway.order_books(&codes).await.map_err(|e| {
        let message = format!("盘口 Gateway 不可用: {e}");
        DelegateError::Fetch(FetchFailure::from_gateway(e).with_message(message))
    })?;
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
    let batch = gateway.money_flows(&codes).await.map_err(|e| {
        DelegateError::Fetch(FetchFailure::unknown(format!("资金流 Gateway 不可用: {e}")))
    })?;
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
    let batch = gateway.security_metadata(&codes).await.map_err(|e| {
        DelegateError::Fetch(FetchFailure::unknown(format!(
            "证券元数据 Gateway 不可用: {e}"
        )))
    })?;
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
    let batch = gateway.us_indices().await.map_err(|e| {
        let message = format!("全球指数 Gateway 不可用: {e}");
        FetchFailure::from_gateway(e).with_message(message)
    })?;
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
    pack_ev(records, &batch)
}

async fn fetch_market_announcements() -> Result<Fetched, FetchFailure> {
    let gateway = EventCalendarGateway::new();
    let batch = gateway
        .market_announcements(today(), 100)
        .await
        .map_err(|e| {
            let message = format!("公告 Gateway 不可用: {e}");
            FetchFailure::from_gateway(e).with_message(message)
        })?;
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
    pack_ev(records, &batch)
}

fn resolve_global_news_request(
    params: &Value,
) -> Result<(GlobalNewsProvider, u32), crate::grpc_contract::params::ParamsError> {
    use crate::data_gateway::global_news::MAX_GLOBAL_NEWS_LIMIT;
    use crate::grpc_contract::params::{resolve_required_string, resolve_u32, ParamsError};

    let provider_name = resolve_required_string(params, "provider")?;
    let provider = GlobalNewsProvider::from_wire_name(&provider_name).ok_or_else(|| {
        ParamsError::InvalidArgument(format!(
            "provider 非法值 {provider_name:?} (允许: Eastmoney/Cailianpress/Jin10/ThePaper)"
        ))
    })?;
    if params.get("limit").is_none() {
        return Err(ParamsError::InvalidArgument("limit 必填".to_string()));
    }
    let limit = resolve_u32(params, "limit", 0)?;
    if !(1..=MAX_GLOBAL_NEWS_LIMIT).contains(&limit) {
        return Err(ParamsError::InvalidArgument(format!(
            "limit 必须在 1..={MAX_GLOBAL_NEWS_LIMIT}，收到 {limit}"
        )));
    }
    Ok((provider, limit))
}

async fn fetch_global_news(params: &Value) -> Result<Fetched, DelegateError> {
    let (provider, limit) = resolve_global_news_request(params)?;
    let gateway = GlobalNewsGateway::new();
    let batch = gateway.global_news(provider, limit).await.map_err(|e| {
        let message = format!("全球新闻 Gateway 不可用: {e}");
        DelegateError::Fetch(FetchFailure::from_gateway(e).with_message(message))
    })?;
    if batch.evidence().provider != provider.provider_id()
        || batch.evidence().source != provider.source()
        || batch.records().len() > limit as usize
    {
        return Err(DelegateError::Fetch(FetchFailure {
            message: format!(
                "global-news response violates request provider={} source={} limit={limit}",
                provider.wire_name(),
                provider.source()
            ),
            provider: Some(provider.provider_id()),
            reason_code: "invalid_evidence",
            retryable: false,
        }));
    }
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
    pack_ev(records, &batch).map_err(DelegateError::Fetch)
}

async fn fetch_economic_calendar() -> Result<Fetched, FetchFailure> {
    let gateway = EconomicCalendarGateway::new();
    let batch = gateway.latest_releases(20, None).await.map_err(|e| {
        let message = format!("财经日历 Gateway 不可用: {e}");
        FetchFailure::from_gateway(e).with_message(message)
    })?;
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
    pack_ev(records, &batch)
}

async fn fetch_futures_delivery() -> Result<Fetched, FetchFailure> {
    let gateway = FuturesDeliveryGateway::new();
    let now = Local::now();
    let batch = gateway
        .cffex_contract_month(now.year() as u32, now.month())
        .await
        .map_err(|e| {
            let message = format!("交割日历 Gateway 不可用: {e}");
            FetchFailure::from_gateway(e).with_message(message)
        })?;
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
    pack_ev(records, &batch)
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
    let stock_limit = crate::grpc_contract::params::resolve_u32(params, "stock_limit", 20)
        .map_err(|e| format!("params 无效: {e}"))?;
    let gateway = DragonTigerGateway::new();
    let batch = gateway
        .market_review(trading_date, disclosure_limit, stock_limit as usize)
        .await
        .map_err(|e| {
            let message = format!("龙虎榜 Gateway 不可用: {e}");
            FetchFailure::from_gateway(e).with_message(message)
        })?;
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
    pack_ev(records, &batch)
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
        .map_err(|e| {
            let message = format!("大宗交易 Gateway 不可用: {e}");
            FetchFailure::from_gateway(e).with_message(message)
        })?;
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
    pack_ev(records, &batch)
}

/// 一致预期: 生产调用方逐代码 fetch(code) (v17_sources) — codes 缺省 watchlist。
async fn fetch_consensus(params: &Value) -> Result<Fetched, FetchFailure> {
    let gateway = ConsensusDataGateway::new();
    let codes = crate::grpc_contract::params::resolve_codes(params)
        .map_err(|e| format!("params 无效: {e}"))?;
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        // ConsensusData 记录本身没有 code 字段 (逐代码查询) → 带 code 回传, JSON 里补上。
        // 保真传播 GatewayError 分类 (no_current_reports 等业务态必须过 detail 还原,
        // 不能 format! 折叠成 unknown → no_verified_batch/retryable=true 无界重试)。
        set.spawn(async move {
            let batch = gateway.fetch(&code).await.map_err(|e| (code.clone(), e))?;
            Ok::<_, (String, crate::data_gateway::GatewayError)>((code, batch))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut evidence_first: Option<crate::data_gateway::BatchEvidence> = None;
    while let Some(joined) = set.join_next().await {
        let (code, batch) = joined
            .map_err(|e| FetchFailure::unknown(format!("一致预期 task 失败: {e}")))?
            .map_err(|(code, e)| FetchFailure {
                message: format!("一致预期 Gateway 不可用 ({code}): {e}"),
                provider: e.provider(),
                reason_code: e.reason_code(),
                retryable: e.retryable(),
            })?;
        retain_single_batch_evidence(&mut evidence_first, &batch)?;
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
    pack_retained_evidence(records, evidence_first)
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
        set.spawn(async move {
            gateway.daily_bars_async(&code, days).await.map_err(|e| {
                DelegateError::Fetch(FetchFailure::unknown(format!(
                    "日线 Gateway 不可用 ({code}): {e}"
                )))
            })
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut evidence_first: Option<crate::data_gateway::BatchEvidence> = None;
    while let Some(joined) = set.join_next().await {
        let batch = joined.map_err(|e| {
            DelegateError::Fetch(FetchFailure::unknown(format!("日线 task 失败: {e}")))
        })?;
        let batch = batch?;
        retain_batch_evidence(&mut evidence_first, batch.evidence())
            .map_err(DelegateError::Fetch)?;
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
    let ev = evidence_first.ok_or_else(|| {
        DelegateError::Fetch(FetchFailure::unknown("日线: 无任何 batch 成功".to_string()))
    })?;
    pack_ev_from(
        records,
        ev.source_at.clone().unwrap_or_default(),
        ev.observed_at.clone(),
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
    let batch = gateway.directory(kind, limit).await.map_err(|e| {
        let message = format!("板块目录 Gateway 不可用: {e}");
        FetchFailure::from_gateway(e).with_message(message)
    })?;
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
    pack_ev(records, &batch)
}

/// 板块成分: board 模块无公开「板块→成分」生产入口 (board_constituents_raw
/// 需内部 BoardConstituentRequest, 未导出) → 用 memberships(code) 查
/// 「个股→所属板块」, 输出成分归属视图。
///
/// BR-238: 每次请求只允许一个 canonical code。多个代码会产生多个独立
/// GatewayBatch，禁止在这里合并后伪造一个跨批次 identity。
async fn fetch_board_constituents(params: &Value) -> Result<Fetched, DelegateError> {
    let gateway = BoardDataGateway::new();
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let [code] = codes.as_slice() else {
        return Err(DelegateError::Params(
            crate::grpc_contract::params::ParamsError::InvalidArgument(
                "board_constituents: BR-238 要求每次恰好一个 canonical code".to_string(),
            ),
        ));
    };
    let batch = gateway.memberships(code).await.map_err(|error| {
        let message = format!("板块归属 Gateway 不可用 ({code}): {error}");
        DelegateError::Fetch(FetchFailure::from_gateway(error).with_message(message))
    })?;
    pack_board_membership_batch(&batch).map_err(DelegateError::Fetch)
}

fn pack_board_membership_batch(
    batch: &crate::data_gateway::GatewayBatch<crate::data_gateway::BoardMembershipRecord>,
) -> Result<Fetched, FetchFailure> {
    let records = batch
        .records()
        .iter()
        .map(|record| {
            json!({
                "instrument_code": record.instrument_code,
                "board_code": record.board_code,
                "board_name": record.board_name,
                "kind": format!("{:?}", record.kind),
            })
        })
        .collect();
    pack_ev(records, batch)
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
    let batch = gateway.day1_flows(kind, limit).await.map_err(|e| {
        let message = format!("板块资金流 Gateway 不可用: {e}");
        FetchFailure::from_gateway(e).with_message(message)
    })?;
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
    pack_ev(records, &batch)
}

fn resolve_limit_pools_request(
    params: &Value,
) -> Result<NaiveDate, crate::grpc_contract::params::ParamsError> {
    use crate::grpc_contract::params::{
        resolve_required_date, resolve_required_string, resolve_u32, ParamsError,
    };

    const REQUIRED_KEYS: [&str; 3] = ["kind", "trading_date", "limit"];
    let object = params
        .as_object()
        .ok_or_else(|| ParamsError::InvalidArgument("LimitPools params 必须是对象".to_string()))?;
    if object.len() != REQUIRED_KEYS.len()
        || object
            .keys()
            .any(|key| !REQUIRED_KEYS.contains(&key.as_str()))
    {
        return Err(ParamsError::InvalidArgument(
            "LimitPools params 必须且只能包含 kind/trading_date/limit".to_string(),
        ));
    }
    let kind = resolve_required_string(params, "kind")?;
    if kind != "Upper" {
        return Err(ParamsError::InvalidArgument(format!(
            "kind 非法值 {kind:?} (仅允许 Upper)"
        )));
    }
    let trading_date = resolve_required_date(params, "trading_date")?;
    let limit = resolve_u32(params, "limit", 0)?;
    if limit != 200 {
        return Err(ParamsError::InvalidArgument(format!(
            "limit 必须为 200，收到 {limit}"
        )));
    }
    Ok(trading_date)
}

/// StrongStockReasons/ChainBatch 共用 A-10 题材链 batch (唯一生产入口)。
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
        observed_at: Local::now().to_rfc3339(),
        provider: format!("{:?}", ProviderId::Custom),
        source: format!("chain-intelligence: {}", batch.calculation_version),
        batch_id: batch.batch_id,
    })
}

fn invalid_limit_pool_evidence(provider: ProviderId, message: impl Into<String>) -> FetchFailure {
    FetchFailure {
        message: message.into(),
        provider: Some(provider),
        reason_code: "invalid_evidence",
        retryable: false,
    }
}

fn pack_limit_pool_batch(
    batch: &crate::data_gateway::GatewayBatch<LimitPoolEntry>,
    trading_date: NaiveDate,
    requested_limit: u32,
) -> Result<Fetched, FetchFailure> {
    let evidence = batch.evidence();
    let provider = evidence.provider;
    if !matches!(provider, ProviderId::Eastmoney | ProviderId::Tonghuashun) {
        return Err(invalid_limit_pool_evidence(
            provider,
            "LimitPools batch provider is not registered for exact-date upper-limit data",
        ));
    }
    let expected_date = trading_date.format("%Y-%m-%d").to_string();
    if evidence.source_at.as_deref() != Some(expected_date.as_str()) {
        return Err(invalid_limit_pool_evidence(
            provider,
            format!(
                "LimitPools batch source_at {:?} differs from requested {expected_date}",
                evidence.source_at
            ),
        ));
    }
    let maximum_count = usize::try_from(requested_limit).map_err(|_| {
        invalid_limit_pool_evidence(provider, "LimitPools requested limit exceeds usize")
    })?;
    if batch.records().len() > maximum_count {
        return Err(invalid_limit_pool_evidence(
            provider,
            format!(
                "LimitPools batch count {} exceeds requested limit {requested_limit}",
                batch.records().len()
            ),
        ));
    }
    for record in batch.records() {
        if record.kind != LimitPoolKind::Upper {
            return Err(invalid_limit_pool_evidence(
                provider,
                "LimitPools record kind differs from requested Upper",
            ));
        }
        if record.trading_date.as_str() != expected_date.as_str() {
            return Err(invalid_limit_pool_evidence(
                provider,
                format!(
                    "LimitPools record trading_date {} differs from requested {expected_date}",
                    record.trading_date.as_str()
                ),
            ));
        }
        if record.evidence.provider() != provider
            || record.evidence.source_at() != evidence.source_at.as_deref()
            || record.evidence.observed_at() != evidence.observed_at.as_str()
            || record.evidence.batch_id() != evidence.batch_id.as_str()
        {
            return Err(invalid_limit_pool_evidence(
                provider,
                format!(
                    "LimitPools record {} evidence differs from its batch envelope",
                    record.instrument.code()
                ),
            ));
        }
    }
    let records = batch
        .records()
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| FetchFailure::unknown(format!("LimitPoolEntry 序列化失败: {error}")))?;
    pack_ev(records, batch)
}

/// Exact-date full upper-limit membership batch for the LocalBridge LimitPools contract.
async fn fetch_limit_pools(params: &Value) -> Result<Fetched, DelegateError> {
    const FULL_LIMIT: u32 = 200;
    let trading_date = resolve_limit_pools_request(params)?;
    let batch = tokio::task::spawn_blocking(move || {
        ReviewDataGateway::new().current_upper_limit_pool_library(trading_date)
    })
    .await
    .map_err(|error| {
        DelegateError::Fetch(FetchFailure::unknown(format!(
            "LimitPools worker join 失败: {error}"
        )))
    })?
    .map_err(|error| {
        let message = format!("LimitPools Gateway 不可用: {error}");
        DelegateError::Fetch(FetchFailure::from_gateway(error).with_message(message))
    })?;
    pack_limit_pool_batch(&batch, trading_date, FULL_LIMIT).map_err(DelegateError::Fetch)
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
    pack_ev_from(
        records,
        batch.trading_date.to_string(),
        Local::now().to_rfc3339(),
        format!("{:?}", ProviderId::Custom),
        format!("chain-intelligence: {}", batch.calculation_version),
        batch.batch_id,
    )
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
    let stock_limit = crate::grpc_contract::params::resolve_u32(params, "stock_limit", 20)
        .map_err(|e| format!("params 无效: {e}"))?;
    let gateway = DragonTigerGateway::new();
    let batch = gateway
        .market_review(trading_date, disclosure_limit, stock_limit as usize)
        .await
        .map_err(|e| {
            let message = format!("龙虎榜 Gateway 不可用: {e}");
            FetchFailure::from_gateway(e).with_message(message)
        })?;
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
    pack_ev(records, &batch)
}

/// 板块排行: fetch_top 是同步 reqwest 阻塞调用 → spawn_blocking 隔离,
/// 不卡 tokio worker。
async fn fetch_board_ranking(fid: &str, top_n: usize) -> Result<Fetched, FetchFailure> {
    let _ = (fid, top_n);
    Err(FetchFailure {
        message: "板块排行 provider 未返回可保真的 batch evidence".to_string(),
        provider: None,
        reason_code: "no_verified_batch",
        retryable: false,
    })
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
        set.spawn(async move {
            let batch = gateway
                .instrument_reports(&code, page_size)
                .await
                .map_err(|e| {
                    let message = format!("研报 Gateway 不可用 ({code}): {e}");
                    FetchFailure::from_gateway(e).with_message(message)
                })?;
            Ok::<_, FetchFailure>((code, batch))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut evidence_first: Option<crate::data_gateway::BatchEvidence> = None;
    while let Some(joined) = set.join_next().await {
        let (code, batch) = joined.map_err(|e| format!("研报 task 失败: {e}"))??;
        retain_single_batch_evidence(&mut evidence_first, &batch)?;
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
    pack_retained_evidence(records, evidence_first)
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
    let batch = gateway.northbound_daily(date, channel).await.map_err(|e| {
        let message = format!("北向资金 Gateway 不可用: {e}");
        FetchFailure::from_gateway(e).with_message(message)
    })?;
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
    pack_ev(records, &batch)
}

// ---------- M1 扩展 (P4): 8 个 proto 已有 op ----------

/// 外汇 (USD/CNY): GlobalMarketGateway::usd_cny 是唯一外汇入口 (上游无参数化
/// forex 方法, P4 探索确认) → params 仅留兼容位, 固定 USD/CNY。
async fn fetch_foreign_exchange(params: &Value) -> Result<Fetched, DelegateError> {
    let _ = params;
    let batch = GlobalMarketGateway::new().usd_cny().await.map_err(|e| {
        DelegateError::Fetch(FetchFailure::unknown(format!("外汇 Gateway 不可用: {e}")))
    })?;
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
            return Err(DelegateError::Params(
                crate::grpc_contract::params::ParamsError::InvalidArgument(format!(
                    "kind 非法值 {other:?} (允许: balance/income/cash_flow)"
                )),
            ))
        }
    };
    let batch = CompanyDataGateway::new()
        .financial_statements(&codes, kind)
        .await
        .map_err(|e| {
            DelegateError::Fetch(FetchFailure::unknown(format!(
                "财务报告 Gateway 不可用: {e}"
            )))
        })?;
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            serde_json::to_value(r)
                .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string())))
        })
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
        .map_err(|e| {
            DelegateError::Fetch(FetchFailure::unknown(format!(
                "市场统计 Gateway 不可用: {e}"
            )))
        })?;
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
    let _ = params;
    Err(DelegateError::Fetch(FetchFailure {
        message: "15 分钟线 provider 未返回可保真的 batch evidence".to_string(),
        provider: None,
        reason_code: "no_verified_batch",
        retryable: false,
    }))
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
        .map_err(|e| {
            DelegateError::Fetch(FetchFailure::unknown(format!(
                "公司行动 Gateway 不可用: {e}"
            )))
        })?;
    let state = ctx.corporate_actions;
    let mut records: Vec<Value> = Vec::new();
    for r in state.records() {
        let terms = serde_json::to_value(&r.terms).map_err(|e| {
            DelegateError::Fetch(FetchFailure::unknown(format!("terms 序列化失败: {e}")))
        })?;
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
        data: serde_json::to_vec(&records)
            .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string())))?,
        source_at: evidence
            .map(|e| e.source_at.clone().unwrap_or_default())
            .unwrap_or_default(),
        observed_at: evidence.map(|e| e.observed_at.clone()).unwrap_or_default(),
        provider: evidence
            .map(|e| format!("{:?}", e.provider))
            .unwrap_or_default(),
        source: evidence.map(|e| e.source.clone()).unwrap_or_default(),
        batch_id: evidence.map(|e| e.batch_id.clone()).unwrap_or_default(),
    })
}

/// BR-242 语义检索: data_gateway 无向量检索 (P4 探索) → 联网检索 GeneralWebResearchGateway
/// (Bocha/Tavily/SerpApi, 需 API key; 无 key 显式失败, 不静默回退)。
fn resolve_semantic_search_request(
    params: &Value,
) -> Result<(GeneralWebResearchProvider, String, usize), crate::grpc_contract::params::ParamsError>
{
    use crate::grpc_contract::params::{resolve_required_string, resolve_u32, ParamsError};

    const REQUIRED_KEYS: [&str; 3] = ["provider", "query", "limit"];
    let object = params.as_object().ok_or_else(|| {
        ParamsError::InvalidArgument("SemanticSearch params 必须是对象".to_string())
    })?;
    if object.len() != REQUIRED_KEYS.len()
        || object
            .keys()
            .any(|key| !REQUIRED_KEYS.contains(&key.as_str()))
    {
        return Err(ParamsError::InvalidArgument(
            "SemanticSearch params 必须且只能包含 provider/query/limit".to_string(),
        ));
    }
    let provider_name = resolve_required_string(params, "provider")?;
    let provider = GeneralWebResearchProvider::from_wire_name(&provider_name).ok_or_else(|| {
        ParamsError::InvalidArgument(format!(
            "provider 非法值 {provider_name:?} (允许: Bocha/Tavily/SerpApi)"
        ))
    })?;
    let query = resolve_required_string(params, "query")?;
    if query.trim() != query {
        return Err(ParamsError::InvalidArgument(
            "query 不得包含首尾空白".to_string(),
        ));
    }
    let limit = resolve_u32(params, "limit", 0)?;
    if !(1..=50).contains(&limit) {
        return Err(ParamsError::InvalidArgument(format!(
            "limit 必须在 1..=50，收到 {limit}"
        )));
    }
    Ok((provider, query, limit as usize))
}

async fn fetch_semantic_search(params: &Value) -> Result<Fetched, DelegateError> {
    let (provider, query, limit) = resolve_semantic_search_request(params)?;
    let batch = GeneralWebResearchGateway::from_environment(provider)
        .search(&query, limit)
        .await
        .map_err(|error| {
            DelegateError::Fetch(FetchFailure {
                message: format!("{} 联网检索不可用: {error}", provider.label()),
                provider: None,
                reason_code: error.reason_code(),
                retryable: error.retryable(),
            })
        })?;
    let evidence = batch.evidence();
    let batch_records = match &batch {
        crate::data_gateway::GeneralWebResearchBatch::Available { records, .. } => {
            records.as_slice()
        }
        crate::data_gateway::GeneralWebResearchBatch::VerifiedEmpty(_) => &[],
    };
    if evidence.provider != provider
        || evidence.source != provider.source()
        || evidence.query != query
        || batch_records.len() > limit
        || batch_records.iter().any(|record| {
            record.evidence.provider != provider
                || record.evidence.batch_id != evidence.batch_id
                || record.evidence.observed_at != evidence.observed_at
        })
    {
        return Err(DelegateError::Fetch(FetchFailure {
            message: format!(
                "SemanticSearch response violates request provider={} source={} limit={limit}",
                provider.wire_name(),
                provider.source()
            ),
            provider: None,
            reason_code: "invalid_evidence",
            retryable: false,
        }));
    }
    // GeneralWebResearchBatch 只有 evidence() 访问器; records 在 enum 变体字段。
    let records: Vec<Value> = batch_records
        .iter()
        .map(|record| {
            serde_json::to_value(record)
                .map_err(|error| DelegateError::Fetch(FetchFailure::unknown(error.to_string())))
        })
        .collect::<Result<_, _>>()?;
    Ok(Fetched {
        data: serde_json::to_vec(&records)
            .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(e.to_string())))?,
        source_at: String::new(),
        observed_at: evidence.observed_at.to_rfc3339(),
        provider: format!("{:?}", evidence.provider),
        source: evidence.source.clone(),
        batch_id: evidence.batch_id.clone(),
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
        set.spawn(async move {
            gateway
                .instrument_fund_flow(&code, interval, limit)
                .await
                .map_err(|e| FetchFailure::unknown(format!("资金流 Gateway 不可用 ({code}): {e}")))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut evidence_first: Option<crate::data_gateway::BatchEvidence> = None;
    while let Some(joined) = set.join_next().await {
        let batch = joined
            .map_err(|e| FetchFailure::unknown(format!("资金流 task 失败: {e}")))?
            .map_err(DelegateError::Fetch)?;
        retain_single_batch_evidence(&mut evidence_first, &batch).map_err(DelegateError::Fetch)?;
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
    pack_retained_evidence(records, evidence_first).map_err(DelegateError::Fetch)
}

/// 头部排行: CapitalDataGateway::provider_top_n_pair (VolumeRatio + MainNetInflow
/// 原子对, limit 固定 20)。BR-198: 同日请求须 15:35 后合格 (上游约束, 原样传递)。
async fn fetch_provider_top_n_rankings(params: &Value) -> Result<Fetched, DelegateError> {
    let date = crate::grpc_contract::params::resolve_date(params)?;
    let pair = CapitalDataGateway::new()
        .provider_top_n_pair(date)
        .await
        .map_err(provider_top_n_gateway_failure)?;
    pack_provider_top_n_pair(&pair).map_err(DelegateError::Fetch)
}

pub(crate) fn provider_top_n_gateway_failure(
    error: crate::data_gateway::GatewayError,
) -> DelegateError {
    let message = format!("头部排行 Gateway 不可用: {error}");
    DelegateError::Fetch(FetchFailure::from_gateway(error).with_message(message))
}

fn pack_provider_top_n_pair(
    pair: &crate::data_gateway::capital::ProviderTopNPair,
) -> Result<Fetched, FetchFailure> {
    let volume_evidence = pair.volume_ratio.evidence();
    let inflow_evidence = pair.main_net_inflow.evidence();
    if pair.volume_ratio.records().is_empty()
        || pair.main_net_inflow.records().is_empty()
        || volume_evidence.source.trim().is_empty()
        || volume_evidence.observed_at.trim().is_empty()
        || volume_evidence.batch_id.trim().is_empty()
        || inflow_evidence.source.trim().is_empty()
        || inflow_evidence.observed_at.trim().is_empty()
        || inflow_evidence.batch_id.trim().is_empty()
        || volume_evidence.batch_id == inflow_evidence.batch_id
    {
        return Err(FetchFailure {
            message: "BR-240 provider Top-N requires two non-empty, distinct real batch evidences"
                .to_string(),
            provider: Some(volume_evidence.provider),
            reason_code: "invalid_evidence",
            retryable: false,
        });
    }

    let mut records: Vec<Value> = Vec::new();
    for batch in [&pair.volume_ratio, &pair.main_net_inflow] {
        let evidence = batch.evidence();
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
                "evidence": {
                    "provider": format!("{:?}", evidence.provider),
                    "source": evidence.source,
                    "source_at": evidence.source_at,
                    "observed_at": evidence.observed_at,
                    "batch_id": evidence.batch_id,
                },
            }));
        }
    }
    // QueryResponse 的公共 evidence 只能承载一个真实批次；保留 volume 批次作为
    // 传输兼容证据，双路权威 evidence 逐行保存在上面的 evidence 对象中。
    pack_ev(records, &pair.volume_ratio)
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
        .map_err(|e| {
            DelegateError::Fetch(FetchFailure::unknown(format!(
                "指数行情 Gateway 不可用: {e}"
            )))
        })?;
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
        set.spawn(async move {
            gateway
                .instrument_news_in_range(&code, from, now)
                .await
                .map_err(|e| {
                    FetchFailure::unknown(format!("个股新闻 Gateway 不可用 ({code}): {e}"))
                })
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut evidence_first: Option<crate::data_gateway::BatchEvidence> = None;
    while let Some(joined) = set.join_next().await {
        let batch = joined
            .map_err(|e| FetchFailure::unknown(format!("个股新闻 task 失败: {e}")))?
            .map_err(DelegateError::Fetch)?;
        retain_single_batch_evidence(&mut evidence_first, &batch).map_err(DelegateError::Fetch)?;
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
    pack_retained_evidence(records, evidence_first).map_err(DelegateError::Fetch)
}

/// 日内形态: IntradayShapeGateway::current_shape (逐代码, async)。
async fn fetch_intraday_shape(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let gateway = IntradayShapeGateway::new();
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        set.spawn(async move {
            gateway.current_shape(&code).await.map_err(|e| {
                FetchFailure::unknown(format!("日内形态 Gateway 不可用 ({code}): {e}"))
            })
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut evidence_first: Option<crate::data_gateway::BatchEvidence> = None;
    while let Some(joined) = set.join_next().await {
        let batch = joined
            .map_err(|e| FetchFailure::unknown(format!("日内形态 task 失败: {e}")))?
            .map_err(DelegateError::Fetch)?;
        retain_single_batch_evidence(&mut evidence_first, &batch).map_err(DelegateError::Fetch)?;
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
    pack_retained_evidence(records, evidence_first).map_err(DelegateError::Fetch)
}

/// T0 证据: MagicTdxGateway::get_t0_evidence_batch (同步阻塞, 直接调用)。
/// 通过显式 wire DTO 输出，已验证的五分钟 civil label 固定标注为 +08:00。
async fn fetch_t0_evidence(params: &Value) -> Result<Fetched, DelegateError> {
    let codes = crate::grpc_contract::params::resolve_codes(params)?;
    let batch = MagicTdxGateway::new()
        .get_t0_evidence_batch(&codes, chrono::Utc::now())
        .map_err(|e| DelegateError::Fetch(FetchFailure::unknown(format!("T0 证据不可用: {e}"))))?;
    let data = crate::grpc_server::t0_wire::encode_t0_batch_v2(&batch).map_err(|error| {
        DelegateError::Fetch(FetchFailure::unknown(format!(
            "T0 v2 batch serialization failed: {error}"
        )))
    })?;
    Ok(Fetched {
        data,
        source_at: batch.source_at.to_rfc3339(),
        observed_at: batch.observed_at.to_rfc3339(),
        provider: format!("{:?}", batch.provider),
        source: batch.source,
        batch_id: batch.batch_id,
    })
}

/// 复盘 outcome 日线 (P4 M3 transport seam): 服务端在 spawn_blocking 内执行
/// 自适应抓取 (fetch_magic_tdx_outcome_adaptive — claim 台账/audit 始终留客户端,
/// 这里只迁移 provider transport)。成功视图保留 batch 与 attempts；失败时没有
/// 可验证批次身份，直接返回 typed FetchFailure，不伪造响应 evidence。
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
    let instrument: InstrumentId =
        serde_json::from_value(params.get("instrument").cloned().ok_or_else(|| {
            DelegateError::Params(crate::grpc_contract::params::ParamsError::InvalidArgument(
                "outcome_daily_bars: instrument 必填".into(),
            ))
        })?)
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
    .map_err(|e| {
        DelegateError::Fetch(FetchFailure::unknown(format!(
            "outcome_daily_bars worker join 失败: {e}"
        )))
    })?;

    match result {
        Ok(RawOutcomeFetch { batch, attempts }) => {
            let batch_json = serde_json::to_value(&batch).map_err(|e| {
                DelegateError::Fetch(FetchFailure::unknown(format!(
                    "outcome batch 序列化失败: {e}"
                )))
            })?;
            let attempts_json = serde_json::to_value(&attempts).map_err(|e| {
                DelegateError::Fetch(FetchFailure::unknown(format!(
                    "outcome attempts 序列化失败: {e}"
                )))
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
                observed_at: provenance.fetched_at().to_string(),
                provider: format!("{:?}", ProviderId::Tdx),
                source: provenance.source().to_string(),
                batch_id: provenance.batch_id().unwrap_or_default().to_string(),
            })
        }
        Err(OutcomeTransportFailure { error, attempts }) => Err(DelegateError::Fetch(
            FetchFailure::from_gateway(error).with_message(format!(
                "outcome transport failed after {} attempts",
                attempts.len()
            )),
        )),
    }
}

/// 涨停池复盘: ReviewDataGateway::r03_upper_limit_pool (date 默认今天)。
async fn fetch_upper_limit_pool_review(params: &Value) -> Result<Fetched, DelegateError> {
    let date = crate::grpc_contract::params::resolve_date(params)?;
    let batch = ReviewDataGateway::new()
        .r03_upper_limit_pool(date)
        .await
        .map_err(|e| {
            let message = format!("涨停池复盘 Gateway 不可用: {e}");
            DelegateError::Fetch(FetchFailure::from_gateway(e).with_message(message))
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_delegate_and_client_converter_roundtrip_one_audited_batch() {
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: "TEST_CODE_000300".to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let audited = crate::data_gateway::review::AuditedBenchmarkBatch {
            batch: crate::data_gateway::GatewayBatch::Available {
                records: vec![crate::data_gateway::BenchmarkBar {
                    at: crate::data_gateway::BenchmarkBarTime::Daily(day),
                    open: 3_500.0,
                    high: 3_510.0,
                    low: 3_490.0,
                    close: 3_505.0,
                    volume: None,
                    amount: Some(8_000.0),
                }],
                evidence: crate::data_gateway::BatchEvidence {
                    provider: ProviderId::Tdx,
                    source: "TEST_CODE_magic-tdx-index-bars".to_owned(),
                    source_at: None,
                    observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
                    batch_id: "TEST_CODE_benchmark_batch".to_owned(),
                },
            },
            receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt {
                audit_id: 17,
                record_hash: "a".repeat(64),
                previous_outcome: None,
                current_outcome: "available".to_owned(),
            },
            request_hash: "c".repeat(64),
        };

        let fetched = pack_benchmark_audited(&request, &audited)
            .expect("server serializes the exact audited TEST_CODE batch");
        assert!(
            fetched.source_at.is_empty(),
            "missing provider source time uses only the proto absence encoding"
        );
        let query = crate::grpc_client::envelope::QueryResult {
            admission: crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted,
            selected_provider: fetched.provider,
            batch_id: fetched.batch_id,
            complete: true,
            observed_at: fetched.observed_at,
            source_at: fetched.source_at,
            records: vec![
                crate::grpc_client::pb::magic::market::v1::CanonicalPayload {
                    schema: "market.benchmark_bars".to_owned(),
                    schema_version: 1,
                    content_type: "application/json; charset=utf-8".to_owned(),
                    data: fetched.data,
                },
            ],
            source: fetched.source,
            diagnostic_blocker: String::new(),
        };
        let admitted = crate::data_gateway::grpc_source::convert::benchmark_bars_for_test(
            &request,
            &query,
            crate::data_gateway::BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
            chrono::DateTime::parse_from_rfc3339("2026-08-21T15:01:01+08:00")
                .expect("TEST_CODE consumer time")
                .with_timezone(&chrono::Utc),
        )
        .expect("client re-admits the server wire without another audit append");

        assert_eq!(admitted.batch.records(), audited.batch.records());
        assert_eq!(admitted.batch.evidence(), audited.batch.evidence());
        assert_eq!(admitted.receipt, audited.receipt);
        assert_eq!(admitted.batch.evidence().source_at, None);
    }

    #[test]
    fn benchmark_delegate_requires_an_exact_numeric_request_wire() {
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: "TEST_CODE_000300".to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let params = serde_json::to_value(
            crate::data_gateway::grpc_source::BenchmarkRequestWire::try_from_request(&request)
                .expect("TEST_CODE valid request"),
        )
        .expect("TEST_CODE request JSON");
        assert_eq!(
            resolve_benchmark_request(&params).expect("exact request wire"),
            request
        );

        for invalid in [
            serde_json::json!({
                "granularity": "Daily",
                "from": {"kind":"daily","year":2026,"month":8,"day":21},
                "to": {"kind":"daily","year":2026,"month":8,"day":21}
            }),
            serde_json::json!({
                "instrument": "TEST_CODE_000300",
                "granularity": "Minute1",
                "from": {"kind":"daily","year":2026,"month":8,"day":21},
                "to": {"kind":"daily","year":2026,"month":8,"day":21}
            }),
            serde_json::json!({
                "instrument": "TEST_CODE_000300",
                "granularity": "Daily",
                "from": {"kind":"daily","year":2026,"month":8,"day":21},
                "to": {"kind":"daily","year":2026,"month":8,"day":21},
                "unexpected": true
            }),
        ] {
            assert!(
                resolve_benchmark_request(&invalid).is_err(),
                "invalid BenchmarkBars params must fail closed: {invalid}"
            );
        }
    }

    #[tokio::test]
    async fn grpc_env_guard_benchmark_delegate_uses_library_without_recursion() {
        let _env = crate::data_gateway::grpc_source::test_grpc_env_guard();
        crate::database::DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        crate::data_gateway::grpc_source::reset_bridge();

        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let request = crate::data_gateway::BenchmarkRequest {
            instrument: "TEST_CODE_000300".to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        };
        let params = serde_json::to_value(
            crate::data_gateway::grpc_source::BenchmarkRequestWire::try_from_request(&request)
                .expect("TEST_CODE valid request"),
        )
        .expect("TEST_CODE request JSON");
        let result = fetch_benchmark_bars(&params).await;
        std::env::remove_var("GRPC_MARKET_ADDR");
        crate::data_gateway::grpc_source::reset_bridge();

        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("TEST_CODE identity must be rejected by the library registry"),
        };
        match error {
            DelegateError::BenchmarkFetch { failure, .. } => {
                assert_eq!(failure.reason_code, "benchmark_test_identity_rejected");
                assert!(!failure.retryable);
                assert!(
                    !failure.message.contains("gRPC server"),
                    "server delegate must not recurse through its configured bridge"
                );
            }
            DelegateError::Params(error) => panic!("valid TEST_CODE wire was misparsed: {error}"),
            DelegateError::Fetch(failure) => {
                panic!("benchmark provider failure lost explicit audit ownership: {failure:?}")
            }
        }
    }

    #[test]
    fn p01_limit_pools_request_requires_exact_upper_date_and_full_limit() {
        let trading_date = NaiveDate::from_ymd_opt(2026, 8, 18).expect("TEST_CODE date");
        assert_eq!(
            resolve_limit_pools_request(&json!({
                "kind": "Upper",
                "trading_date": "2026-08-18",
                "limit": 200,
            }))
            .expect("exact P-01 LimitPools request"),
            trading_date
        );

        for invalid in [
            json!({}),
            json!({"kind": "Upper", "trading_date": "2026-08-18"}),
            json!({"kind": "upper", "trading_date": "2026-08-18", "limit": 200}),
            json!({"kind": "Upper", "trading_date": "2026-08-18", "limit": 199}),
            json!({"kind": "Upper", "trading_date": "2026-08-18", "limit": 1000}),
            json!({"kind": "Upper", "trading_date": "2026-02-30", "limit": 200}),
            json!({"kind": "Upper", "date": "2026-08-18", "limit": 200}),
            json!({
                "kind": "Upper",
                "trading_date": "2026-08-18",
                "limit": 200,
                "unexpected": true,
            }),
        ] {
            assert!(
                resolve_limit_pools_request(&invalid).is_err(),
                "invalid LimitPools request must fail closed: {invalid}"
            );
        }
    }

    #[test]
    fn br238_global_news_request_requires_exact_provider_and_limit() {
        let cases = [
            ("Eastmoney", GlobalNewsProvider::Eastmoney),
            ("Cailianpress", GlobalNewsProvider::Cailianpress),
            ("Jin10", GlobalNewsProvider::Jin10),
            ("ThePaper", GlobalNewsProvider::ThePaper),
        ];
        for (wire_name, expected) in cases {
            assert_eq!(
                resolve_global_news_request(&json!({"provider": wire_name, "limit": 7}))
                    .expect("registered global-news request"),
                (expected, 7)
            );
        }

        for invalid in [
            json!({}),
            json!({"provider": "Eastmoney"}),
            json!({"limit": 7}),
            json!({"provider": "eastmoney", "limit": 7}),
            json!({"provider": "Eastmoney", "limit": 0}),
            json!({"provider": "Eastmoney", "limit": 21}),
        ] {
            assert!(
                resolve_global_news_request(&invalid).is_err(),
                "invalid request must fail closed: {invalid}"
            );
        }
    }

    #[test]
    fn br242_semantic_search_request_binds_exact_provider_query_and_limit() {
        for (wire_name, expected) in [
            ("Bocha", GeneralWebResearchProvider::Bocha),
            ("Tavily", GeneralWebResearchProvider::Tavily),
            ("SerpApi", GeneralWebResearchProvider::SerpApi),
        ] {
            assert_eq!(
                resolve_semantic_search_request(&json!({
                    "provider": wire_name,
                    "query": "TEST_CODE 语义检索",
                    "limit": 50,
                }))
                .expect("exact SemanticSearch request"),
                (expected, "TEST_CODE 语义检索".to_string(), 50)
            );
        }

        for invalid in [
            json!({}),
            json!({"query": "TEST_CODE", "limit": 1}),
            json!({"provider": "Bocha", "limit": 1}),
            json!({"provider": "Bocha", "query": "TEST_CODE"}),
            json!({"provider": "bocha", "query": "TEST_CODE", "limit": 1}),
            json!({"provider": "Bocha", "query": "", "limit": 1}),
            json!({"provider": "Bocha", "query": "TEST_CODE", "limit": 0}),
            json!({"provider": "Bocha", "query": "TEST_CODE", "limit": 51}),
            json!({
                "provider": "Bocha",
                "query": "TEST_CODE",
                "limit": 1,
                "unexpected": true,
            }),
        ] {
            assert!(
                resolve_semantic_search_request(&invalid).is_err(),
                "invalid SemanticSearch request must fail closed: {invalid}"
            );
        }
    }

    #[test]
    fn br238_gateway_failure_classification_survives_delegate_context() {
        let failure = FetchFailure::from_gateway(crate::data_gateway::GatewayError::unavailable(
            "TEST_CODE_capability",
            Some(ProviderId::Sina),
            false,
            "TEST_CODE_original",
        ))
        .with_message("TEST_CODE_context");
        assert_eq!(failure.provider, Some(ProviderId::Sina));
        assert_eq!(failure.reason_code, "no_verified_batch");
        assert!(!failure.retryable);
        assert_eq!(failure.message, "TEST_CODE_context");
    }
    use crate::data_gateway::{
        capital::{ProviderTopNFact, ProviderTopNPair, ProviderTopNRequestEvidence},
        BatchEvidence, BoardMembershipRecord, GatewayBatch,
    };
    use crate::market_domain::{
        AssetClass, Exchange, FiniteNumber, IsoDate, MarketRankingKind, MarketRankingUnit, Money,
        NonEmptyText, PositiveU32, Price, Quantity, Ratio, RatioUnit, SourceEvidence,
    };

    fn p01_limit_pool_entry(
        kind: LimitPoolKind,
        trading_date: &str,
        provider: ProviderId,
        batch_id: &str,
    ) -> LimitPoolEntry {
        LimitPoolEntry {
            kind,
            instrument: InstrumentId::new(
                Exchange::Shanghai,
                "TEST_CODE_600001",
                AssetClass::Equity,
            )
            .expect("TEST_CODE instrument"),
            trading_date: IsoDate::new(trading_date).expect("TEST_CODE trading date"),
            price: Price::new(10.25).expect("positive price"),
            change: Ratio::new(10.0, RatioUnit::Percent).expect("finite change"),
            volume: Some(Quantity::new(12_300.0).expect("non-negative volume")),
            turnover: Some(Ratio::new(3.5, RatioUnit::Percent).expect("finite turnover")),
            sealed_amount: Some(Money::new(8_800_000.0).expect("finite amount")),
            first_seal_at: Some(
                NonEmptyText::new("2026-08-18T09:31:00+08:00").expect("first seal"),
            ),
            last_seal_at: Some(NonEmptyText::new("2026-08-18T10:02:00+08:00").expect("last seal")),
            break_count: Some(1),
            streak: Some(PositiveU32::new(2).expect("positive streak")),
            industry: Some(NonEmptyText::new("TEST_CODE industry").expect("industry")),
            board_name: Some(NonEmptyText::new("TEST_CODE board").expect("board")),
            seal_state: Some(NonEmptyText::new("TEST_CODE sealed").expect("seal state")),
            reseal_count: Some(1),
            reason: Some(NonEmptyText::new("TEST_CODE reason").expect("reason")),
            evidence: SourceEvidence::new(provider, "2026-08-18T10:03:00+08:00", batch_id)
                .expect("record evidence")
                .with_source_at(trading_date)
                .expect("record source date"),
        }
    }

    #[test]
    fn p01_limit_pools_pack_retains_full_entry_and_original_batch_envelope() {
        let trading_date = NaiveDate::from_ymd_opt(2026, 8, 18).expect("TEST_CODE date");
        let batch_id = "TEST_CODE_LIMIT_POOL_BATCH";
        let entry = p01_limit_pool_entry(
            LimitPoolKind::Upper,
            "2026-08-18",
            ProviderId::Eastmoney,
            batch_id,
        );
        let batch = GatewayBatch::Available {
            records: vec![entry.clone()],
            evidence: BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "TEST_CODE_eastmoney_limit_pool".to_string(),
                source_at: Some("2026-08-18".to_string()),
                observed_at: "2026-08-18T10:03:00+08:00".to_string(),
                batch_id: batch_id.to_string(),
            },
        };

        let packed = pack_limit_pool_batch(&batch, trading_date, 200)
            .expect("valid full LimitPoolEntry batch");
        let decoded: Vec<LimitPoolEntry> =
            serde_json::from_slice(&packed.data).expect("full LimitPoolEntry wire batch");

        assert_eq!(decoded, vec![entry]);
        assert_eq!(packed.provider, "Eastmoney");
        assert_eq!(packed.source, "TEST_CODE_eastmoney_limit_pool");
        assert_eq!(packed.source_at, "2026-08-18");
        assert_eq!(packed.observed_at, "2026-08-18T10:03:00+08:00");
        assert_eq!(packed.batch_id, batch_id);
    }

    #[test]
    fn p01_limit_pools_pack_rejects_count_date_kind_and_evidence_conflicts() {
        let trading_date = NaiveDate::from_ymd_opt(2026, 8, 18).expect("TEST_CODE date");
        let batch_id = "TEST_CODE_LIMIT_POOL_BATCH";
        let evidence = BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: "TEST_CODE_eastmoney_limit_pool".to_string(),
            source_at: Some("2026-08-18".to_string()),
            observed_at: "2026-08-18T10:03:00+08:00".to_string(),
            batch_id: batch_id.to_string(),
        };
        let valid = p01_limit_pool_entry(
            LimitPoolKind::Upper,
            "2026-08-18",
            ProviderId::Eastmoney,
            batch_id,
        );

        let mut wrong_date = valid.clone();
        wrong_date.trading_date = IsoDate::new("2026-08-17").expect("TEST_CODE prior date");
        let mut wrong_kind = valid.clone();
        wrong_kind.kind = LimitPoolKind::Broken;
        let mut wrong_evidence = valid.clone();
        wrong_evidence.evidence = SourceEvidence::new(
            ProviderId::Eastmoney,
            "2026-08-18T10:03:00+08:00",
            "TEST_CODE_OTHER_BATCH",
        )
        .expect("conflicting evidence")
        .with_source_at("2026-08-18")
        .expect("conflicting source date");
        let mut wrong_envelope_date = evidence.clone();
        wrong_envelope_date.source_at = Some("2026-08-17".to_string());

        let conflicts = [
            (
                "count",
                GatewayBatch::Available {
                    records: vec![valid.clone(); 201],
                    evidence: evidence.clone(),
                },
            ),
            (
                "envelope-date",
                GatewayBatch::Available {
                    records: vec![valid.clone()],
                    evidence: wrong_envelope_date,
                },
            ),
            (
                "date",
                GatewayBatch::Available {
                    records: vec![wrong_date],
                    evidence: evidence.clone(),
                },
            ),
            (
                "kind",
                GatewayBatch::Available {
                    records: vec![wrong_kind],
                    evidence: evidence.clone(),
                },
            ),
            (
                "evidence",
                GatewayBatch::Available {
                    records: vec![wrong_evidence],
                    evidence,
                },
            ),
        ];

        for (conflict, batch) in conflicts {
            let Err(failure) = pack_limit_pool_batch(&batch, trading_date, 200) else {
                panic!("request-bound LimitPools {conflict} conflict must fail closed");
            };
            assert_eq!(failure.reason_code, "invalid_evidence", "{conflict}");
            assert!(!failure.retryable, "{conflict}");
        }
    }

    fn provider_top_n_side(
        metric: MarketRankingKind,
        unit: MarketRankingUnit,
        code: &str,
        batch_id: &str,
    ) -> GatewayBatch<ProviderTopNFact> {
        GatewayBatch::Available {
            records: vec![ProviderTopNFact {
                metric,
                source_order_ordinal: PositiveU32::new(1).expect("positive ordinal"),
                instrument: InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity)
                    .expect("test instrument"),
                label: NonEmptyText::new(format!("TEST_CODE_{code}")).expect("non-empty label"),
                value: FiniteNumber::new(1.0).expect("finite value"),
                unit,
                trading_date: IsoDate::new("2026-08-17").expect("ISO date"),
                filter_identity: NonEmptyText::new("TEST_CODE_A_SHARE_FILTER")
                    .expect("non-empty filter"),
                provider_declared_total: PositiveU32::new(2).expect("positive total"),
                inspected_row_count: PositiveU32::new(1).expect("positive row count"),
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Eastmoney,
                source: "eastmoney-web".to_string(),
                source_at: None,
                observed_at: "2026-08-17T15:36:00+08:00".to_string(),
                batch_id: batch_id.to_string(),
            },
        }
    }

    fn provider_top_n_request(metric: MarketRankingKind) -> ProviderTopNRequestEvidence {
        ProviderTopNRequestEvidence {
            metric,
            trading_date: IsoDate::new("2026-08-17").expect("ISO date"),
            limit: PositiveU32::new(20).expect("positive limit"),
            filter_identity: NonEmptyText::new("TEST_CODE_A_SHARE_FILTER")
                .expect("non-empty filter"),
            request_hash: "TEST_CODE_REQUEST_HASH".to_string(),
        }
    }

    #[test]
    fn br240_provider_top_n_pack_retains_both_real_batch_evidences() {
        let volume_batch_id = "TEST_CODE_REAL_VOLUME_BATCH";
        let inflow_batch_id = "TEST_CODE_REAL_INFLOW_BATCH";
        let pair = ProviderTopNPair {
            volume_ratio_request: provider_top_n_request(MarketRankingKind::VolumeRatio),
            volume_ratio: provider_top_n_side(
                MarketRankingKind::VolumeRatio,
                MarketRankingUnit::Multiple,
                "TEST_CODE_600001",
                volume_batch_id,
            ),
            main_net_inflow_request: provider_top_n_request(MarketRankingKind::MainNetInflow),
            main_net_inflow: provider_top_n_side(
                MarketRankingKind::MainNetInflow,
                MarketRankingUnit::Yuan,
                "TEST_CODE_600002",
                inflow_batch_id,
            ),
        };

        let packed = pack_provider_top_n_pair(&pair).expect("two real evidence batches");
        let rows: Vec<Value> = serde_json::from_slice(&packed.data).expect("canonical rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["evidence"]["batch_id"], volume_batch_id);
        assert_eq!(rows[1]["evidence"]["batch_id"], inflow_batch_id);
        assert_ne!(
            rows[0]["evidence"]["batch_id"],
            rows[1]["evidence"]["batch_id"]
        );
        for row in rows {
            assert_eq!(row["evidence"]["provider"], "Eastmoney");
            assert_eq!(row["evidence"]["source"], "eastmoney-web");
            assert_eq!(row["evidence"]["source_at"], Value::Null);
            assert_eq!(row["evidence"]["observed_at"], "2026-08-17T15:36:00+08:00");
        }
    }

    #[test]
    fn br240_provider_top_n_pack_rejects_identical_batch_ids() {
        let duplicate_batch_id = "TEST_CODE_DUPLICATE_REAL_BATCH";
        let pair = ProviderTopNPair {
            volume_ratio_request: provider_top_n_request(MarketRankingKind::VolumeRatio),
            volume_ratio: provider_top_n_side(
                MarketRankingKind::VolumeRatio,
                MarketRankingUnit::Multiple,
                "TEST_CODE_600001",
                duplicate_batch_id,
            ),
            main_net_inflow_request: provider_top_n_request(MarketRankingKind::MainNetInflow),
            main_net_inflow: provider_top_n_side(
                MarketRankingKind::MainNetInflow,
                MarketRankingUnit::Yuan,
                "TEST_CODE_600002",
                duplicate_batch_id,
            ),
        };

        let Err(failure) = pack_provider_top_n_pair(&pair) else {
            panic!("duplicate IDs must fail closed");
        };
        assert_eq!(failure.reason_code, "invalid_evidence");
        assert!(!failure.retryable);
    }

    #[test]
    fn single_code_membership_pack_preserves_batch_identity() {
        let batch = GatewayBatch::Available {
            records: vec![BoardMembershipRecord {
                instrument_code: "TEST_CODE_600519".to_string(),
                board_code: "TEST_CODE_BK0475".to_string(),
                board_name: "TEST_CODE_BOARD".to_string(),
                kind: BoardKind::Concept,
            }],
            evidence: BatchEvidence {
                provider: ProviderId::Tdx,
                source: "tdx".to_string(),
                source_at: Some("2026-08-17T09:20:00+08:00".to_string()),
                observed_at: "2026-08-17T09:20:01+08:00".to_string(),
                batch_id: "TEST_CODE_MEMBERSHIP_BATCH_1".to_string(),
            },
        };

        let packed = pack_board_membership_batch(&batch).expect("single-code membership pack");

        assert_eq!(packed.provider, "Tdx");
        assert_eq!(packed.source, "tdx");
        assert_eq!(packed.batch_id, "TEST_CODE_MEMBERSHIP_BATCH_1");
        assert_eq!(packed.observed_at, "2026-08-17T09:20:01+08:00");
    }

    #[test]
    fn production_delegates_do_not_use_identity_erasing_pack() {
        let source = include_str!("delegate.rs");
        let forbidden_definition = ["fn pack", "(records"].concat();
        let forbidden_call = ["pack", "(records,"].concat();
        assert!(!source.contains(&forbidden_definition));
        assert!(!source.contains(&forbidden_call));
    }

    #[tokio::test]
    async fn membership_request_rejects_cross_batch_aggregation() {
        let params = json!({
            "codes": ["TEST_CODE_600519", "TEST_CODE_000001"],
        });

        let result = fetch_board_constituents(&params).await;

        let Err(DelegateError::Params(error)) = result else {
            panic!("multi-code membership request must fail before acquisition");
        };
        assert!(error.to_string().contains("BR-238"));
    }
}
