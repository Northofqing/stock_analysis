//! P4 M2: gRPC 响应 → 客户端类型化 GatewayBatch 转换。
//! 与服务端 delegate.rs fetch_xxx 的 JSON 视图逐字段镜像 (每条转换注明对应
//! fetch 行号, 视图字段名以 delegate 的 json! 键名为准 — 例如 change_pct 对应
//! 结构体字段 change_percent)。
//!
//! 缺字段/缺证据 → GatewayError::invalid_evidence (fail-closed, 绝不静默填充)。
//! 空 records → GatewayBatch::VerifiedEmpty (服务端 proven empty, 不 collapse
//! 成 unavailable)。
use crate::data_gateway::{
    board_ranking::BoardRankingFact, BatchEvidence, BlockTradeReview, BoardDirectoryFact,
    BoardDirectoryRecordEvidence, BoardFlowFact, BoardKind, BoardMembershipRecord,
    DragonTigerSeatReview, DragonTigerSourceDisclosure, DragonTigerStockReview,
    EconomicReleaseFact, EventAnnouncement, ForeignExchangeFact, FuturesDeliveryFact,
    GatewayBatch, GatewayError, GeneralWebResearchBatch, GeneralWebResearchBatchEvidence,
    GeneralWebResearchProvider, GeneralWebResearchRecord, GlobalIndexFact, GlobalNewsRecord,
    ImplementedCorporateAction, InstrumentFundFlowFact, IntradayShapeFact, MagicTdxT0Batch,
    MagicTdxT0DailyBar, MagicTdxT0Evidence, MagicTdxT0FiveMinuteBar, MagicTdxT0Quote,
    MagicTdxT0Rejection, MarketBookLevel, MarketMinutePoint, MarketMoneyFlow, MarketOrderBook,
    MarketSecurityMetadata, NorthboundDailyFact, NorthboundQuotaFact,
    NorthboundTopTurnoverFact, ProviderTopNFact, RealtimeIndexQuote, RealtimeMarketQuote,
    ResearchReportFact, ResearchUseScope, SecurityBoard, SinaInstrumentNewsRecord, T0BookLevel,
    UpperLimitRecord,
};
use crate::data_provider::{consensus::ConsensusData, news_item::NewsItem, AdjustType, KlineData};
use crate::data_gateway::outcome_daily_bars::{OutcomeTransportFailure, RawOutcomeFetch};
use crate::selection::schema_v2::OutcomeTransportAttemptPreimage;
use crate::grpc_client::envelope::QueryResult;
use chrono::{DateTime, NaiveDate, Utc};
use magic_market_core::{
    AssetClass, Bar, CorporateActionCategory, CorporateActionTerms, DataBatch, DragonTigerSide,
    Exchange, FinancialStatement, FiniteNumber, FlowInterval, FxPair, GlobalIndexCode,
    InstrumentId, IsoDate, MarketRankingKind, MarketRankingUnit, MarketStatistics, Money,
    NorthboundChannel, NonEmptyText, PositiveU32, Price, ProviderId, Ratio, SourceEvidence,
};
use magic_tdx_rs::protocol::types::SecurityBar;
use serde_json::Value;

/// bridge 缺证据时的 capability 标记 (audit_outcome=invalid_evidence)。
const BRIDGE_CAPABILITY: &str = "GrpcBridge";

fn err(capability: &'static str, msg: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(capability, None, msg)
}

/// Debug 名 → ProviderId (服务端 pack_ev 用 format!("{:?}", provider) 写 JSON)。
/// 未知/空 → Err (fail-closed: 不静默猜 Tdx)。
pub fn parse_provider(s: &str) -> Result<ProviderId, GatewayError> {
    Ok(match s {
        "Tdx" => ProviderId::Tdx,
        "Tencent" => ProviderId::Tencent,
        "Eastmoney" => ProviderId::Eastmoney,
        "Sina" => ProviderId::Sina,
        "Baostock" => ProviderId::Baostock,
        "Baidu" => ProviderId::Baidu,
        "Tonghuashun" => ProviderId::Tonghuashun,
        "Iwencai" => ProviderId::Iwencai,
        "Cninfo" => ProviderId::Cninfo,
        "Cailianpress" => ProviderId::Cailianpress,
        "Jin10" => ProviderId::Jin10,
        "ThePaper" => ProviderId::ThePaper,
        "Yonhap" => ProviderId::Yonhap,
        "WallstreetCn" => ProviderId::WallstreetCn,
        "Sse" => ProviderId::Sse,
        "Szse" => ProviderId::Szse,
        "Hkex" => ProviderId::Hkex,
        "Cffex" => ProviderId::Cffex,
        "StateCouncil" => ProviderId::StateCouncil,
        "Nbs" => ProviderId::Nbs,
        "Pbc" => ProviderId::Pbc,
        "Cfets" => ProviderId::Cfets,
        "Fred" => ProviderId::Fred,
        "Imf" => ProviderId::Imf,
        "WorldBank" => ProviderId::WorldBank,
        "SecEdgar" => ProviderId::SecEdgar,
        "XinhuaFinance" => ProviderId::XinhuaFinance,
        "Yicai" => ProviderId::Yicai,
        "SecuritiesTimes" => ProviderId::SecuritiesTimes,
        "LocalAnalysis" => ProviderId::LocalAnalysis,
        "LocalTerminal" => ProviderId::LocalTerminal,
        "Custom" => ProviderId::Custom,
        "" => {
            return Err(err(
                BRIDGE_CAPABILITY,
                "provider 空 (服务端未回填证据链, 旧 op 尚在升级中)",
            ))
        }
        other => {
            return Err(err(
                BRIDGE_CAPABILITY,
                format!("未知 provider Debug 名: {other}"),
            ))
        }
    })
}

/// 服务端 Debug 名 → SecurityBoard (fetch_security_metadata 视图)。
fn parse_board(s: &str) -> Result<SecurityBoard, GatewayError> {
    Ok(match s {
        "Main" => SecurityBoard::Main,
        "Star" => SecurityBoard::Star,
        "ChiNext" => SecurityBoard::ChiNext,
        "Beijing" => SecurityBoard::Beijing,
        other => {
            return Err(err(
                "SecurityMetadata",
                format!("未知 board Debug 名: {other}"),
            ))
        }
    })
}

/// 从 QueryResult 信封构造 BatchEvidence。
/// source_at 空 → None (合同 §6 缺则不填充); provider/source/batch_id 空 → Err。
fn evidence_of(q: &QueryResult, capability: &'static str) -> Result<BatchEvidence, GatewayError> {
    let provider = parse_provider(&q.selected_provider)
        .map_err(|e| err(capability, format!("selected_provider 无法解析: {e}")))?;
    if q.source.is_empty() {
        return Err(err(capability, "source 空 (服务端未回填证据链)"));
    }
    if q.batch_id.is_empty() {
        return Err(err(capability, "batch_id 空 (服务端未回填证据链)"));
    }
    Ok(BatchEvidence {
        provider,
        source: q.source.clone(),
        source_at: if q.source_at.is_empty() {
            None
        } else {
            Some(q.source_at.clone())
        },
        observed_at: q.observed_at.clone(),
        batch_id: q.batch_id.clone(),
    })
}

/// records data 字节 → Value 数组。
fn parse_records(q: &QueryResult, capability: &'static str) -> Result<Vec<Value>, GatewayError> {
    let Some(payload) = q.records.first() else {
        return Err(err(
            capability,
            "records 空 (服务端无 canonical payload)",
        ));
    };
    serde_json::from_slice(&payload.data)
        .map_err(|e| err(capability, format!("records 非 JSON 数组: {e}")))
}

fn as_str(v: &Value, key: &str, capability: &'static str) -> Result<String, GatewayError> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| err(capability, format!("record 缺字符串字段 {key}")))
}

fn as_f64(v: &Value, key: &str, capability: &'static str) -> Result<f64, GatewayError> {
    v.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| err(capability, format!("record 缺数值字段 {key}")))
}

fn as_bool(v: &Value, key: &str, capability: &'static str) -> Result<bool, GatewayError> {
    v.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| err(capability, format!("record 缺布尔字段 {key}")))
}

fn as_rfc3339(v: &Value, key: &str, capability: &'static str) -> Result<DateTime<Utc>, GatewayError> {
    let s = as_str(v, key, capability)?;
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| err(capability, format!("字段 {key} 非 RFC3339: {s} ({e})")))
}

fn as_date(v: &Value, key: &str, capability: &'static str) -> Result<NaiveDate, GatewayError> {
    let s = as_str(v, key, capability)?;
    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
        .map_err(|e| err(capability, format!("字段 {key} 非 YYYY-MM-DD: {s} ({e})")))
}

/// 逐代码 op 共用: 记录必须全部属于目标 code (服务端按 watchlist 拼接时,
/// params 未生效的中间态会被这里 fail-closed 拒绝, 不把错误代码当数据)。
fn per_code_records<'a>(
    q: &'a QueryResult,
    capability: &'static str,
    code: &'a str,
) -> Result<(Vec<Value>, BatchEvidence), GatewayError> {
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok((Vec::new(), ev));
    }
    if parsed
        .iter()
        .any(|v| v.get("code").and_then(Value::as_str) != Some(code))
    {
        return Err(err(
            capability,
            format!("服务端返回了非目标代码 {code} 的记录 (params 未生效?)"),
        ));
    }
    Ok((parsed, ev))
}

/// record 时间戳: 视图无逐条 source_at 的 op → 用证据链 source_at (fail-closed: 空 = Err)。
fn record_source_at(q: &QueryResult, capability: &'static str) -> Result<DateTime<Utc>, GatewayError> {
    if q.source_at.is_empty() {
        return Err(err(capability, "source_at 空 (服务端未回填证据链)"));
    }
    DateTime::parse_from_rfc3339(&q.source_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| err(capability, format!("source_at 非 RFC3339: {} ({e})", q.source_at)))
}

fn record_observed_at(q: &QueryResult, capability: &'static str) -> Result<DateTime<Utc>, GatewayError> {
    DateTime::parse_from_rfc3339(&q.observed_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| err(capability, format!("observed_at 非 RFC3339: {} ({e})", q.observed_at)))
}

// ---------- 6 个首批 op (M2) ----------

/// 统一实时行情。视图: delegate.rs fetch_realtime_quotes (:176-184)
/// {"code","name","price","change_pct","previous_close"}。
pub fn realtime_quotes(q: &QueryResult) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
    let capability = "RealtimeMarketQuotes";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<RealtimeMarketQuote> = parsed
        .iter()
        .map(|v| {
            Ok(RealtimeMarketQuote {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                price: as_f64(v, "price", capability)?,
                change_percent: as_f64(v, "change_pct", capability)?,
                previous_close: as_f64(v, "previous_close", capability)?,
                source_at: record_source_at(q, capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 分钟线。视图: delegate.rs fetch_minute_data (:211-219)
/// {"code","minute_at","price","cumulative_quantity","cumulative_amount","source_at"}。
pub fn minute_data(q: &QueryResult) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
    let capability = "MarketMinuteData";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketMinutePoint> = parsed
        .iter()
        .map(|v| {
            Ok(MarketMinutePoint {
                code: as_str(v, "code", capability)?,
                minute_at: as_rfc3339(v, "minute_at", capability)?,
                price: as_f64(v, "price", capability)?,
                cumulative_quantity: as_f64(v, "cumulative_quantity", capability)?,
                cumulative_amount: v.get("cumulative_amount").and_then(Value::as_f64),
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 盘口 5 档。视图: delegate.rs fetch_order_books (:235-244)
/// {"code","bids":[{"price","quantity"}],"asks","total_bid_quantity",
/// "total_ask_quantity","source_at"}。档位不足 5 → 补 0.0 (空档 = 无挂单, 语义
/// 正确); 超过 5 → 报错 (服务端视图违约)。
fn book_levels(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<[MarketBookLevel; 5], GatewayError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| err(capability, format!("record 缺数组字段 {key}")))?;
    if arr.len() > 5 {
        return Err(err(
            capability,
            format!("{key} 档位数 {} 超过 5 (服务端视图违约)", arr.len()),
        ));
    }
    let mut levels = [MarketBookLevel { price: 0.0, quantity: 0.0 }; 5];
    for (i, item) in arr.iter().enumerate() {
        levels[i] = MarketBookLevel {
            price: as_f64(item, "price", capability)?,
            quantity: as_f64(item, "quantity", capability)?,
        };
    }
    Ok(levels)
}

pub fn order_books(q: &QueryResult) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
    let capability = "MarketOrderBooks";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketOrderBook> = parsed
        .iter()
        .map(|v| {
            Ok(MarketOrderBook {
                code: as_str(v, "code", capability)?,
                bids: book_levels(v, "bids", capability)?,
                asks: book_levels(v, "asks", capability)?,
                total_bid_quantity: as_f64(v, "total_bid_quantity", capability)?,
                total_ask_quantity: as_f64(v, "total_ask_quantity", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 资金流。视图: delegate.rs fetch_money_flows (:262-270)
/// {"code","main_net","super_large_net","large_net","medium_net","small_net","source_at"}。
pub fn money_flows(q: &QueryResult) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
    let capability = "MarketMoneyFlows";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketMoneyFlow> = parsed
        .iter()
        .map(|v| {
            Ok(MarketMoneyFlow {
                code: as_str(v, "code", capability)?,
                main_net: as_f64(v, "main_net", capability)?,
                super_large_net: as_f64(v, "super_large_net", capability)?,
                large_net: as_f64(v, "large_net", capability)?,
                medium_net: as_f64(v, "medium_net", capability)?,
                small_net: as_f64(v, "small_net", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 证券元数据。视图: delegate.rs fetch_security_metadata (:282-291)
/// {"code","name","board"(Debug),"is_st","listed_on","price_limit_percent","source_at"}。
pub fn security_metadata(q: &QueryResult) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
    let capability = "SecurityMetadata";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketSecurityMetadata> = parsed
        .iter()
        .map(|v| {
            let board = as_str(v, "board", capability)?;
            let listed_on = as_date(v, "listed_on", capability)?;
            Ok(MarketSecurityMetadata {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                board: parse_board(&board)?,
                is_st: as_bool(v, "is_st", capability)?,
                listed_on,
                price_limit_percent: as_f64(v, "price_limit_percent", capability)?,
                // 视图无 price_limit_version (M1 冻结) → 留空, 消费方按缺证据感知。
                price_limit_version: String::new(),
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 日线 K 线。视图: delegate.rs fetch_historical_bars (:538-550)
/// {"code","date","open","high","low","close","volume","amount","pct_chg","settled"}。
/// 视图只含 KlineData 的 10 个字段子集 → 其余 Option 字段 = None、bool = false、
/// adjust = None (视图冻结, 消费者需要的字段由 M3+ 扩展服务端视图, 不在客户端
/// 发明数据)。
pub fn historical_bars(code: &str, q: &QueryResult) -> Result<GatewayBatch<KlineData>, GatewayError> {
    let capability = "HistoricalDailyBars";
    let (parsed, ev) = per_code_records(q, capability, code)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<KlineData> = parsed
        .iter()
        .map(|v| {
            Ok(KlineData {
                date: as_date(v, "date", capability)?,
                open: as_f64(v, "open", capability)?,
                high: as_f64(v, "high", capability)?,
                low: as_f64(v, "low", capability)?,
                close: as_f64(v, "close", capability)?,
                volume: as_f64(v, "volume", capability)?,
                amount: as_f64(v, "amount", capability)?,
                pct_chg: as_f64(v, "pct_chg", capability)?,
                settled: as_bool(v, "settled", capability)?,
                intraday_price: None,
                pe_ratio: None,
                pb_ratio: None,
                turnover_rate: None,
                market_cap: None,
                circulating_cap: None,
                eps: None,
                roe: None,
                revenue_yoy: None,
                net_profit_yoy: None,
                gross_margin: None,
                net_margin: None,
                sharpe_ratio: None,
                financials_history: None,
                valuation_history: None,
                consensus: None,
                industry: None,
                is_limit_up: false,
                is_limit_down: false,
                is_suspended: false,
                adjust: AdjustType::None,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

// ---------- M3 批次 1: 全球市场/日历/公告/新闻/交割 ----------

/// 全球指数。视图: delegate.rs fetch_global_indices (:339-361)
/// {"code"(Debug),"name","value","change","change_percent","source_at"}。
pub fn global_indices(q: &QueryResult) -> Result<GatewayBatch<GlobalIndexFact>, GatewayError> {
    let capability = "GlobalIndices";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(GlobalIndexFact {
                code: parse_global_index_code(&as_str(v, "code", capability)?)?,
                name: as_str(v, "name", capability)?,
                value: as_f64(v, "value", capability)?,
                change: as_f64(v, "change", capability)?,
                change_percent: as_f64(v, "change_percent", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 外汇。视图: delegate.rs fetch_foreign_exchange (:900-921)
/// {"pair"(Debug),"name","rate","change","change_percent","source_at"}。
/// change/change_percent 是可空数值 (JSON null → None, 不补零)。
pub fn foreign_exchange(q: &QueryResult) -> Result<GatewayBatch<ForeignExchangeFact>, GatewayError> {
    let capability = "ForeignExchange";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(ForeignExchangeFact {
                pair: parse_fx_pair(&as_str(v, "pair", capability)?)?,
                name: as_str(v, "name", capability)?,
                rate: as_f64(v, "rate", capability)?,
                change: as_optional_f64(v, "change", capability)?,
                change_percent: as_optional_f64(v, "change_percent", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 公告。视图: delegate.rs fetch_announcements (:363-385)
/// {"announcement_id","code","category","title","published_at","url"}。
/// category 可空 (JSON null → None)。
pub fn announcements(q: &QueryResult) -> Result<GatewayBatch<EventAnnouncement>, GatewayError> {
    let capability = "Announcements";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(EventAnnouncement {
                announcement_id: as_str(v, "announcement_id", capability)?,
                code: as_str(v, "code", capability)?,
                category: as_optional_str(v, "category", capability)?,
                title: as_str(v, "title", capability)?,
                published_at: as_str(v, "published_at", capability)?,
                canonical_url: as_str(v, "url", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 全球新闻。视图: delegate.rs fetch_global_news (:387-411)
/// {"item_id","title","summary","publisher","url","published_at",
///  "instruments","topics","content","language"}。
pub fn global_news(q: &QueryResult) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
    let capability = "GlobalNews";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(GlobalNewsRecord {
                item_id: as_str(v, "item_id", capability)?,
                title: as_str(v, "title", capability)?,
                summary: as_optional_str(v, "summary", capability)?,
                content: as_optional_str(v, "content", capability)?,
                publisher: as_str(v, "publisher", capability)?,
                canonical_url: as_str(v, "url", capability)?,
                published_at: as_rfc3339(v, "published_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                instruments: as_str_array(v, "instruments", capability)?,
                topics: as_str_array(v, "topics", capability)?,
                language: as_str(v, "language", capability)?,
                evidence: record_evidence(&ev, q)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 财经日历。视图: delegate.rs fetch_economic_calendar (:413-439)
/// {"event_id","country","name","period","scheduled_at","previous","consensus",
///  "actual","unit","importance","released_at","revised","impact","indicator_id"}。
pub fn economic_calendar(q: &QueryResult) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
    let capability = "EconomicCalendar";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(EconomicReleaseFact {
                event_id: as_str(v, "event_id", capability)?,
                indicator_id: as_u64(v, "indicator_id", capability)? as u32,
                country: as_str(v, "country", capability)?,
                name: as_str(v, "name", capability)?,
                period: as_optional_str(v, "period", capability)?,
                scheduled_at: as_rfc3339(v, "scheduled_at", capability)?,
                released_at: as_rfc3339(v, "released_at", capability)?,
                previous: as_optional_str(v, "previous", capability)?,
                consensus: as_optional_str(v, "consensus", capability)?,
                actual: as_optional_str(v, "actual", capability)?,
                revised: as_optional_str(v, "revised", capability)?,
                unit: as_optional_str(v, "unit", capability)?,
                importance: as_u64(v, "importance", capability)? as u32,
                impact: as_optional_str(v, "impact", capability)?,
                evidence: record_evidence(&ev, q)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 交割日历。视图: delegate.rs fetch_futures_delivery (:441-463)
/// {"contract_code","product_code","last_trading_date","delivery_date","notice_url"}。
/// last_trading_date 可空 (JSON null → None)。
pub fn futures_delivery(q: &QueryResult) -> Result<GatewayBatch<FuturesDeliveryFact>, GatewayError> {
    let capability = "FuturesDelivery";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(FuturesDeliveryFact {
                contract_code: as_str(v, "contract_code", capability)?,
                product_code: as_str(v, "product_code", capability)?,
                last_trading_date: as_optional_date(v, "last_trading_date", capability)?,
                delivery_date: as_date(v, "delivery_date", capability)?,
                notice_url: as_str(v, "notice_url", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// Debug 名 → GlobalIndexCode (服务端 format!("{:?}", code))。未知 → Err。
fn parse_global_index_code(s: &str) -> Result<GlobalIndexCode, GatewayError> {
    Ok(match s {
        "DowJones" => GlobalIndexCode::DowJones,
        "NasdaqComposite" => GlobalIndexCode::NasdaqComposite,
        "Sp500" => GlobalIndexCode::Sp500,
        "Nikkei225" => GlobalIndexCode::Nikkei225,
        "HangSeng" => GlobalIndexCode::HangSeng,
        "Ftse100" => GlobalIndexCode::Ftse100,
        _ => return Err(err("GlobalIndices", format!("未知 GlobalIndexCode: {s}"))),
    })
}

/// Debug 名 → FxPair。未知 → Err。
fn parse_fx_pair(s: &str) -> Result<FxPair, GatewayError> {
    Ok(match s {
        "UsdCny" => FxPair::UsdCny,
        "EurUsd" => FxPair::EurUsd,
        "UsdJpy" => FxPair::UsdJpy,
        "GbpUsd" => FxPair::GbpUsd,
        "AudUsd" => FxPair::AudUsd,
        "UsdChf" => FxPair::UsdChf,
        "UsdCad" => FxPair::UsdCad,
        "NzdUsd" => FxPair::NzdUsd,
        _ => return Err(err("ForeignExchange", format!("未知 FxPair: {s}"))),
    })
}

/// record 级证据: 用批级 evidence 构造 (视图无逐条证据字段)。
fn record_evidence(ev: &BatchEvidence, q: &QueryResult) -> Result<SourceEvidence, GatewayError> {
    let mut evidence = SourceEvidence::new(
        ev.provider,
        ev.observed_at.clone(),
        ev.batch_id.clone(),
    )
    .map_err(|e| err(BRIDGE_CAPABILITY, format!("record evidence 构造失败: {e}")))?;
    if let Some(source_at) = &ev.source_at {
        evidence = evidence
            .with_source_at(source_at.clone())
            .map_err(|e| err(BRIDGE_CAPABILITY, format!("record evidence source_at 失败: {e}")))?;
    }
    let _ = q;
    Ok(evidence)
}

/// 视图可空数值字段 (JSON null → None; 缺失 → Err fail-closed)。
fn as_optional_f64(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<Option<f64>, GatewayError> {
    let value = v.get(key).ok_or_else(|| err(capability, format!("record 缺数值字段 {key}")))?;
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_f64()
        .map(Some)
        .ok_or_else(|| err(capability, format!("字段 {key} 非数值")))
}

/// 视图可空字符串字段 (JSON null → None; 缺失 → Err fail-closed)。
fn as_optional_str(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<Option<String>, GatewayError> {
    let value = v.get(key).ok_or_else(|| err(capability, format!("record 缺字符串字段 {key}")))?;
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(|s| s.to_string())
        .map(Some)
        .ok_or_else(|| err(capability, format!("字段 {key} 非字符串")))
}

/// 视图可空日期字段 ("YYYY-MM-DD" 或 null)。
fn as_optional_date(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<Option<NaiveDate>, GatewayError> {
    let value = v.get(key).ok_or_else(|| err(capability, format!("record 缺日期字段 {key}")))?;
    if value.is_null() {
        return Ok(None);
    }
    let s = value
        .as_str()
        .ok_or_else(|| err(capability, format!("字段 {key} 非字符串")))?;
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(Some)
        .map_err(|e| err(capability, format!("字段 {key} 非 YYYY-MM-DD: {s} ({e})")))
}

/// 视图整数字段。
fn as_u64(v: &Value, key: &str, capability: &'static str) -> Result<u64, GatewayError> {
    let value = v.get(key).ok_or_else(|| err(capability, format!("record 缺数值字段 {key}")))?;
    value
        .as_u64()
        .ok_or_else(|| err(capability, format!("字段 {key} 非整数")))
}

/// 视图字符串数组字段。
fn as_str_array(v: &Value, key: &str, capability: &'static str) -> Result<Vec<String>, GatewayError> {
    let value = v.get(key).ok_or_else(|| err(capability, format!("record 缺数组字段 {key}")))?;
    let arr = value
        .as_array()
        .ok_or_else(|| err(capability, format!("字段 {key} 非数组")))?;
    arr.iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| err(capability, format!("字段 {key} 元素非字符串")))
        })
        .collect()
}

// ---------- M3 批次 2: 龙虎榜/大宗/一致预期/板块/研报/北向/财务/技术/资金流/排行/指数/个股新闻/形态/涨停复盘/T0 ----------

/// core 验证类型构造错误 → GatewayError (fail-closed)。
fn core_err(capability: &'static str, e: impl std::fmt::Display) -> GatewayError {
    err(capability, format!("core 验证失败: {e}"))
}

/// 枚举 Debug 名反解析 (视图 format!("{:?}", ...) 契约)。
fn parse_exchange(s: &str, capability: &'static str) -> Result<Exchange, GatewayError> {
    match s {
        "Shanghai" => Ok(Exchange::Shanghai),
        "Shenzhen" => Ok(Exchange::Shenzhen),
        "Beijing" => Ok(Exchange::Beijing),
        _ => Err(err(capability, format!("未知 Exchange: {s}"))),
    }
}

fn parse_board_kind(s: &str, capability: &'static str) -> Result<BoardKind, GatewayError> {
    match s {
        "Industry" => Ok(BoardKind::Industry),
        "Concept" => Ok(BoardKind::Concept),
        "Region" => Ok(BoardKind::Region),
        _ => Err(err(capability, format!("未知 BoardKind: {s}"))),
    }
}

fn parse_flow_interval(s: &str, capability: &'static str) -> Result<FlowInterval, GatewayError> {
    match s {
        "Minute1" => Ok(FlowInterval::Minute1),
        "Day1" => Ok(FlowInterval::Day1),
        "Day5" => Ok(FlowInterval::Day5),
        "Day10" => Ok(FlowInterval::Day10),
        "Day120" => Ok(FlowInterval::Day120),
        _ => Err(err(capability, format!("未知 FlowInterval: {s}"))),
    }
}

fn parse_northbound_channel(
    s: &str,
    capability: &'static str,
) -> Result<NorthboundChannel, GatewayError> {
    match s {
        "Shanghai" => Ok(NorthboundChannel::Shanghai),
        "Shenzhen" => Ok(NorthboundChannel::Shenzhen),
        _ => Err(err(capability, format!("未知 NorthboundChannel: {s}"))),
    }
}

fn parse_dragon_tiger_side(
    s: &str,
    capability: &'static str,
) -> Result<DragonTigerSide, GatewayError> {
    match s {
        "Buy" => Ok(DragonTigerSide::Buy),
        "Sell" => Ok(DragonTigerSide::Sell),
        _ => Err(err(capability, format!("未知 DragonTigerSide: {s}"))),
    }
}

/// Custom("xxx") → 内嵌 NonEmptyText (视图 Debug 名契约)。
fn parse_custom_string(s: &str, capability: &'static str) -> Result<NonEmptyText, GatewayError> {
    let inner = s
        .strip_prefix("Custom(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| err(capability, format!("非 Custom Debug 名: {s}")))?;
    let value: String = serde_json::from_str(inner)
        .map_err(|e| err(capability, format!("Custom 内嵌值解析失败 ({e}): {s}")))?;
    NonEmptyText::new(value).map_err(|e| core_err(capability, e))
}

fn parse_market_ranking_kind(
    s: &str,
    capability: &'static str,
) -> Result<MarketRankingKind, GatewayError> {
    match s {
        "VolumeRatio" => Ok(MarketRankingKind::VolumeRatio),
        "MainNetInflow" => Ok(MarketRankingKind::MainNetInflow),
        "Industry" => Ok(MarketRankingKind::Industry),
        "Concept" => Ok(MarketRankingKind::Concept),
        "Region" => Ok(MarketRankingKind::Region),
        "Popularity" => Ok(MarketRankingKind::Popularity),
        _ if s.starts_with("Custom(") => {
            Ok(MarketRankingKind::Custom(parse_custom_string(s, capability)?))
        }
        _ => Err(err(capability, format!("未知 MarketRankingKind: {s}"))),
    }
}

fn parse_market_ranking_unit(
    s: &str,
    capability: &'static str,
) -> Result<MarketRankingUnit, GatewayError> {
    match s {
        "Multiple" => Ok(MarketRankingUnit::Multiple),
        "Yuan" => Ok(MarketRankingUnit::Yuan),
        "Percent" => Ok(MarketRankingUnit::Percent),
        "Score" => Ok(MarketRankingUnit::Score),
        _ if s.starts_with("Custom(") => {
            Ok(MarketRankingUnit::Custom(parse_custom_string(s, capability)?))
        }
        _ => Err(err(capability, format!("未知 MarketRankingUnit: {s}"))),
    }
}

fn instrument_for(code: &str, capability: &'static str) -> Result<InstrumentId, GatewayError> {
    let exchange = match crate::grpc_contract::params::exchange_of(code) {
        "Shanghai" => Exchange::Shanghai,
        "Shenzhen" => Exchange::Shenzhen,
        "Beijing" => Exchange::Beijing,
        other => return Err(err(capability, format!("未知 exchange 前缀: {other}"))),
    };
    InstrumentId::new(exchange, code, AssetClass::Equity).map_err(|e| core_err(capability, e))
}

fn parse_disclosures(
    v: &Value,
    capability: &'static str,
) -> Result<Vec<DragonTigerSourceDisclosure>, GatewayError> {
    let arr = v
        .get("disclosures")
        .and_then(Value::as_array)
        .ok_or_else(|| err(capability, "字段 disclosures 非数组"))?;
    arr.iter()
        .map(|d| {
            let seats = d
                .get("seats")
                .and_then(Value::as_array)
                .ok_or_else(|| err(capability, "字段 seats 非数组"))?
                .iter()
                .map(|s| {
                    Ok(DragonTigerSeatReview {
                        side: parse_dragon_tiger_side(&as_str(s, "side", capability)?, capability)?,
                        rank: as_u64(s, "rank", capability)? as u32,
                        seat_name: as_str(s, "seat_name", capability)?,
                        amount_yuan: as_f64(s, "amount_yuan", capability)?,
                        buy_amount_yuan: as_optional_f64(s, "buy_amount_yuan", capability)?,
                        sell_amount_yuan: as_optional_f64(s, "sell_amount_yuan", capability)?,
                        net_amount_yuan: as_optional_f64(s, "net_amount_yuan", capability)?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DragonTigerSourceDisclosure {
                entry_id: as_str(d, "entry_id", capability)?,
                trade_id: as_str(d, "trade_id", capability)?,
                reason: as_optional_str(d, "reason", capability)?,
                buy_amount_yuan: as_optional_f64(d, "buy_amount_yuan", capability)?,
                sell_amount_yuan: as_optional_f64(d, "sell_amount_yuan", capability)?,
                net_amount_yuan: as_optional_f64(d, "net_amount_yuan", capability)?,
                turnover_rate_pct: as_optional_f64(d, "turnover_rate_pct", capability)?,
                seats,
            })
        })
        .collect()
}

pub fn dragon_tiger(q: &QueryResult) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
    let capability = "DragonTiger";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<DragonTigerStockReview> = parsed
        .iter()
        .map(|v| {
            Ok(DragonTigerStockReview {
                exchange: parse_exchange(&as_str(v, "exchange", capability)?, capability)?,
                code: as_str(v, "code", capability)?,
                ranking_net_amount_yuan: as_f64(v, "ranking_net_amount_yuan", capability)?,
                disclosures: parse_disclosures(v, capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn market_dragon_tiger(
    q: &QueryResult,
) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
    let capability = "MarketDragonTiger";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<DragonTigerStockReview> = parsed
        .iter()
        .map(|v| {
            Ok(DragonTigerStockReview {
                exchange: parse_exchange(&as_str(v, "exchange", capability)?, capability)?,
                code: as_str(v, "code", capability)?,
                ranking_net_amount_yuan: as_f64(v, "ranking_net_amount_yuan", capability)?,
                disclosures: parse_disclosures(v, capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn block_trades(q: &QueryResult) -> Result<GatewayBatch<BlockTradeReview>, GatewayError> {
    let capability = "BlockTrades";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BlockTradeReview> = parsed
        .iter()
        .map(|v| {
            Ok(BlockTradeReview {
                code: as_str(v, "code", capability)?,
                traded_at: as_optional_str(v, "traded_at", capability)?,
                price: as_f64(v, "price", capability)?,
                close_price: as_optional_f64(v, "close_price", capability)?,
                premium_ratio: as_optional_f64(v, "premium_ratio", capability)?,
                volume: as_f64(v, "volume", capability)?,
                amount: as_optional_f64(v, "amount", capability)?,
                buyer: as_optional_str(v, "buyer", capability)?,
                seller: as_optional_str(v, "seller", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn consensus(q: &QueryResult) -> Result<GatewayBatch<ConsensusData>, GatewayError> {
    let capability = "Consensus";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<ConsensusData> = parsed
        .iter()
        .map(|v| {
            let rating_distribution = v
                .get("rating_distribution")
                .and_then(Value::as_object)
                .ok_or_else(|| err(capability, "字段 rating_distribution 非对象"))?
                .iter()
                .map(|(k, val)| {
                    Ok((
                        k.clone(),
                        val.as_u64().ok_or_else(|| {
                            err(capability, format!("rating_distribution[{k}] 非整数"))
                        })? as u32,
                    ))
                })
                .collect::<Result<std::collections::HashMap<String, u32>, _>>()?;
            Ok(ConsensusData {
                report_count: as_u64(v, "report_count", capability)? as usize,
                broker_count: as_u64(v, "broker_count", capability)? as usize,
                eps_this_year_avg: as_optional_f64(v, "eps_this_year_avg", capability)?,
                eps_next_year_avg: as_optional_f64(v, "eps_next_year_avg", capability)?,
                eps_next2_year_avg: as_optional_f64(v, "eps_next2_year_avg", capability)?,
                rating_distribution,
                target_price_high_avg: None,
                target_price_low_avg: None,
                latest_report_date: None,
                recent_reports: Vec::new(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn board_directory(q: &QueryResult) -> Result<GatewayBatch<BoardDirectoryFact>, GatewayError> {
    let capability = "BoardDirectory";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BoardDirectoryFact> = parsed
        .iter()
        .map(|v| {
            Ok(BoardDirectoryFact {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                kind: parse_board_kind(&as_str(v, "kind", capability)?, capability)?,
                member_count: as_u64(v, "member_count", capability)? as u32,
                evidence: directory_evidence(&ev, capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn board_constituents(
    q: &QueryResult,
) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
    let capability = "BoardConstituents";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BoardMembershipRecord> = parsed
        .iter()
        .map(|v| {
            Ok(BoardMembershipRecord {
                instrument_code: as_str(v, "instrument_code", capability)?,
                board_code: as_str(v, "board_code", capability)?,
                board_name: as_str(v, "board_name", capability)?,
                kind: parse_board_kind(&as_str(v, "kind", capability)?, capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn board_flows(q: &QueryResult) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
    let capability = "BoardFlows";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BoardFlowFact> = parsed
        .iter()
        .map(|v| {
            Ok(BoardFlowFact {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                kind: parse_board_kind(&as_str(v, "kind", capability)?, capability)?,
                rank: as_u64(v, "rank", capability)? as u32,
                return_pct: as_optional_f64(v, "return_pct", capability)?,
                main_net_yuan: as_optional_f64(v, "main_net_yuan", capability)?,
                leader_code: as_optional_str(v, "leader_code", capability)?,
                leader_name: as_optional_str(v, "leader_name", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn board_ranking(q: &QueryResult) -> Result<GatewayBatch<BoardRankingFact>, GatewayError> {
    let capability = "MarketRankings";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BoardRankingFact> = parsed
        .iter()
        .map(|v| {
            Ok(BoardRankingFact {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                change_pct: as_f64(v, "change_pct", capability)?,
                main_inflow: as_f64(v, "main_inflow", capability)?,
                leader_name: as_str(v, "leader_name", capability)?,
                vol_ratio: as_f64(v, "vol_ratio", capability)?,
                turnover: as_f64(v, "turnover", capability)?,
                day1_ratio: as_f64(v, "day1_ratio", capability)?,
                day5_ratio: as_f64(v, "day5_ratio", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn research_reports(q: &QueryResult) -> Result<GatewayBatch<ResearchReportFact>, GatewayError> {
    let capability = "ResearchReports";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<ResearchReportFact> = parsed
        .iter()
        .map(|v| {
            Ok(ResearchReportFact {
                report_id: as_str(v, "report_id", capability)?,
                title: as_str(v, "title", capability)?,
                organization: as_str(v, "organization", capability)?,
                organization_id: None,
                author: None,
                rating: as_optional_str(v, "rating", capability)?,
                industry_code: None,
                industry_name: None,
                published_at: as_str(v, "published_at", capability)?,
                canonical_url: as_str(v, "canonical_url", capability)?,
                pdf_url: None,
                source_target_price_upper: as_optional_f64(v, "target_price_upper", capability)?,
                source_target_price_lower: as_optional_f64(v, "target_price_lower", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// BoardDirectoryRecordEvidence 构造 (批级 evidence 映射, 视图无逐条证据)。
fn directory_evidence(
    ev: &BatchEvidence,
    capability: &'static str,
) -> Result<BoardDirectoryRecordEvidence, GatewayError> {
    let _ = capability;
    Ok(BoardDirectoryRecordEvidence {
        provider: ev.provider,
        source: ev.source.clone(),
        source_at: ev.source_at.clone(),
        observed_at: ev.observed_at.clone(),
        batch_id: ev.batch_id.clone(),
    })
}

/// 视图级证据 parse (evidence_of + parse_records 合并)。
fn parse_records_parts(
    q: &QueryResult,
    capability: &'static str,
) -> Result<(Vec<Value>, BatchEvidence), GatewayError> {
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    Ok((parsed, ev))
}

pub fn northbound_daily(q: &QueryResult) -> Result<GatewayBatch<NorthboundDailyFact>, GatewayError> {
    let capability = "NorthboundDaily";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<NorthboundDailyFact> = parsed
        .iter()
        .map(|v| {
            let quota_balance = match v.get("quota_balance") {
                Some(Value::Number(n)) => {
                    NorthboundQuotaFact::Amount(n.as_f64().ok_or_else(|| {
                        err(capability, "quota_balance 非有限数字")
                    })?)
                }
                Some(Value::String(s)) if s == "unavailable" => NorthboundQuotaFact::Unavailable,
                _ => return Err(err(capability, "quota_balance 必须是数字或 unavailable")),
            };
            let top_turnover = v
                .get("top_turnover")
                .and_then(Value::as_array)
                .ok_or_else(|| err(capability, "字段 top_turnover 非数组"))?
                .iter()
                .map(|t| {
                    Ok(NorthboundTopTurnoverFact {
                        rank: as_u64(t, "rank", capability)? as u32,
                        code: as_str(t, "code", capability)?,
                        name: as_str(t, "name", capability)?,
                        total_turnover: as_f64(t, "total_turnover", capability)?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(NorthboundDailyFact {
                trading_date: as_date(v, "trading_date", capability)?,
                channel: parse_northbound_channel(&as_str(v, "channel", capability)?, capability)?,
                total_turnover: as_f64(v, "total_turnover", capability)?,
                total_trade_count: as_f64(v, "total_trade_count", capability)?,
                quota_balance,
                etf_turnover: as_f64(v, "etf_turnover", capability)?,
                top_turnover,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn financial_statements(
    q: &QueryResult,
) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
    let capability = "FinancialStatements";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<FinancialStatement> = parsed
        .iter()
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|e| err(capability, format!("FinancialStatement 反序列化失败: {e}")))
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn market_statistics(q: &QueryResult) -> Result<GatewayBatch<MarketStatistics>, GatewayError> {
    let capability = "MarketStatistics";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketStatistics> = parsed
        .iter()
        .map(|v| {
            let code = as_str(v, "code", capability)?;
            let instrument = instrument_for(&code, capability)?;
            MarketStatistics::new(
                instrument,
                as_optional_f64(v, "turnover_rate", capability)?
                    .map(|x| Ratio::decimal(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "trailing_pe", capability)?
                    .map(|x| FiniteNumber::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "static_pe", capability)?
                    .map(|x| FiniteNumber::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "pb", capability)?
                    .map(|x| FiniteNumber::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "total_market_cap", capability)?
                    .map(|x| Money::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "floating_market_cap", capability)?
                    .map(|x| Money::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "upper_limit", capability)?
                    .map(|x| Price::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "lower_limit", capability)?
                    .map(|x| Price::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "volume_ratio", capability)?
                    .map(|x| FiniteNumber::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                record_evidence(&ev, q)?,
            )
            .map_err(|e| core_err(capability, e))
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn technical_bars(q: &QueryResult) -> Result<GatewayBatch<SecurityBar>, GatewayError> {
    let capability = "TechnicalBars";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<SecurityBar> = parsed
        .iter()
        .map(|v| {
            Ok(SecurityBar {
                open: as_f64(v, "open", capability)?,
                close: as_f64(v, "close", capability)?,
                high: as_f64(v, "high", capability)?,
                low: as_f64(v, "low", capability)?,
                vol: as_f64(v, "vol", capability)?,
                amount: as_f64(v, "amount", capability)?,
                year: 0,
                month: 0,
                day: 0,
                hour: 0,
                minute: 0,
                datetime: as_str(v, "at", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn fund_flow_series(
    q: &QueryResult,
) -> Result<GatewayBatch<InstrumentFundFlowFact>, GatewayError> {
    let capability = "FundFlowSeries";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<InstrumentFundFlowFact> = parsed
        .iter()
        .map(|v| {
            Ok(InstrumentFundFlowFact {
                code: as_str(v, "code", capability)?,
                interval: parse_flow_interval(&as_str(v, "interval", capability)?, capability)?,
                period_at: as_str(v, "period_at", capability)?,
                main_net: as_f64(v, "main_net", capability)?,
                main_ratio_percent: as_f64(v, "main_ratio_percent", capability)?,
                super_large_net: as_f64(v, "super_large_net", capability)?,
                large_net: as_f64(v, "large_net", capability)?,
                medium_net: as_f64(v, "medium_net", capability)?,
                small_net: as_f64(v, "small_net", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 单条头部排行记录解析 (双路 provider_top_n_pair 与全量视图共用)。
fn parse_provider_top_n_record(
    v: &Value,
    capability: &'static str,
) -> Result<ProviderTopNFact, GatewayError> {
    Ok(ProviderTopNFact {
        metric: parse_market_ranking_kind(&as_str(v, "metric", capability)?, capability)?,
        source_order_ordinal: PositiveU32::new(as_u64(v, "ordinal", capability)? as u32)
            .map_err(|e| core_err(capability, e))?,
        instrument: instrument_for(&as_str(v, "code", capability)?, capability)?,
        label: NonEmptyText::new(as_str(v, "label", capability)?)
            .map_err(|e| core_err(capability, e))?,
        value: FiniteNumber::new(as_f64(v, "value", capability)?)
            .map_err(|e| core_err(capability, e))?,
        unit: parse_market_ranking_unit(&as_str(v, "unit", capability)?, capability)?,
        trading_date: IsoDate::new(as_str(v, "trading_date", capability)?)
            .map_err(|e| core_err(capability, e))?,
        filter_identity: NonEmptyText::new(as_str(v, "filter_identity", capability)?)
            .map_err(|e| core_err(capability, e))?,
        provider_declared_total: PositiveU32::new(
            as_u64(v, "provider_declared_total", capability)? as u32,
        )
        .map_err(|e| core_err(capability, e))?,
        // 服务端视图含真实 inspected_row_count (delegate 原样传递, 本地路径语义对等)。
        inspected_row_count: PositiveU32::new(
            as_u64(v, "inspected_row_count", capability)? as u32,
        )
        .map_err(|e| core_err(capability, e))?,
    })
}

pub fn provider_top_n_rankings(
    q: &QueryResult,
) -> Result<GatewayBatch<ProviderTopNFact>, GatewayError> {
    let capability = "ProviderTopNRankings";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<ProviderTopNFact> = parsed
        .iter()
        .map(|v| parse_provider_top_n_record(v, capability))
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 头部排行双路 (ProviderTopNPair 视角): 服务端视图合流输出 (无分路顺序
/// 保证), 客户端按 metric 分组重建 volume_ratio / main_net_inflow 两个
/// GatewayBatch — 与本地 CapitalDataGateway::provider_top_n_pair 的
/// `GatewayBatch<ProviderTopNFact> × 2` 结构对齐。
pub fn provider_top_n_pair(
    q: &QueryResult,
) -> Result<
    (
        GatewayBatch<ProviderTopNFact>,
        GatewayBatch<ProviderTopNFact>,
    ),
    GatewayError,
> {
    let capability = "ProviderTopNRankings";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok((
            GatewayBatch::VerifiedEmpty(ev.clone()),
            GatewayBatch::VerifiedEmpty(ev),
        ));
    }
    let mut volume: Vec<ProviderTopNFact> = Vec::new();
    let mut inflow: Vec<ProviderTopNFact> = Vec::new();
    for v in &parsed {
        let record = parse_provider_top_n_record(v, capability)?;
        match record.metric {
            MarketRankingKind::VolumeRatio => volume.push(record),
            MarketRankingKind::MainNetInflow => inflow.push(record),
            other => return Err(err(capability, format!("头部排行未知 metric: {other:?}"))),
        }
    }
    let partition = |records: Vec<ProviderTopNFact>| {
        if records.is_empty() {
            GatewayBatch::VerifiedEmpty(ev.clone())
        } else {
            GatewayBatch::Available {
                records,
                evidence: ev.clone(),
            }
        }
    };
    Ok((partition(volume), partition(inflow)))
}

pub fn index_quotes(q: &QueryResult) -> Result<GatewayBatch<RealtimeIndexQuote>, GatewayError> {
    let capability = "IndexQuotes";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<RealtimeIndexQuote> = parsed
        .iter()
        .map(|v| {
            Ok(RealtimeIndexQuote {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                current: as_f64(v, "current", capability)?,
                change: as_f64(v, "change", capability)?,
                change_percent: as_f64(v, "change_percent", capability)?,
                open: as_f64(v, "open", capability)?,
                high: as_f64(v, "high", capability)?,
                low: as_f64(v, "low", capability)?,
                previous_close: as_f64(v, "previous_close", capability)?,
                volume: as_f64(v, "volume", capability)?,
                amount: as_f64(v, "amount", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn instrument_news(
    q: &QueryResult,
) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
    let capability = "InstrumentNews";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<SinaInstrumentNewsRecord> = parsed
        .iter()
        .map(|v| {
            let item = NewsItem {
                source: as_str(v, "source", capability)?,
                external_id: as_str(v, "external_id", capability)?,
                category: as_str(v, "category", capability)?,
                code: as_optional_str(v, "code", capability)?,
                title: as_str(v, "title", capability)?,
                summary: as_str(v, "summary", capability)?,
                url: as_str(v, "url", capability)?,
                source_name: as_str(v, "source_name", capability)?,
                published_at: as_rfc3339(v, "published_at", capability)?,
                fetched_at: as_rfc3339(v, "fetched_at", capability)?,
                content_hash: as_str(v, "content_hash", capability)?,
            };
            Ok(SinaInstrumentNewsRecord::new(item, record_evidence(&ev, q)?))
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn intraday_shape(q: &QueryResult) -> Result<GatewayBatch<IntradayShapeFact>, GatewayError> {
    let capability = "IntradayShape";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<IntradayShapeFact> = parsed
        .iter()
        .map(|v| {
            // shape_label 是 &'static str (视图字符串) → Box::leak 保生命周期
            // (每批 record 数有限, 形状标签来自服务端已验证值, 非用户输入)。
            let label: &'static str = Box::leak(as_str(v, "shape_label", capability)?.into_boxed_str());
            Ok(IntradayShapeFact {
                date: as_str(v, "date", capability)?,
                pre_close: as_f64(v, "pre_close", capability)?,
                open_pct: as_f64(v, "open_pct", capability)?,
                high_pct: as_f64(v, "high_pct", capability)?,
                low_pct: as_f64(v, "low_pct", capability)?,
                close_pct: as_f64(v, "close_pct", capability)?,
                amplitude: as_f64(v, "amplitude", capability)?,
                tail_30m_pct: as_optional_f64(v, "tail_30m_pct", capability)?,
                shape_label: label,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

pub fn upper_limit_pool_review(
    q: &QueryResult,
) -> Result<GatewayBatch<UpperLimitRecord>, GatewayError> {
    let capability = "UpperLimitPoolReview";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<UpperLimitRecord> = parsed
        .iter()
        .map(|v| {
            Ok(UpperLimitRecord {
                code: as_str(v, "code", capability)?,
                trading_date: as_date(v, "trading_date", capability)?,
                theme: as_optional_str(v, "theme", capability)?,
                streak: as_optional_u32(v, "streak", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// T0 证据批: 视图是 {"records": [...], "rejections": [...]} 对象 (delegate
/// fetch_t0_evidence 契约; record 字段 serde 直出)。
/// 返回 MagicTdxT0Batch (records + rejections 全量) — 与本地
/// MagicTdxGateway::get_t0_evidence_batch 对齐, rejections 绝不丢弃。
/// 空 records → 空批 (本地语义: get_t0_evidence_batch 返回空批而非错误)。
pub fn t0_evidence_batch(q: &QueryResult) -> Result<MagicTdxT0Batch, GatewayError> {
    let capability = "T0Evidence";
    let ev = evidence_of(q, capability)?;
    let Some(payload) = q.records.first() else {
        return Err(err(
            capability,
            "records 空 (服务端无 canonical payload)",
        ));
    };
    // 合同 (M1): 视图是对象 {"records","rejections"} (非数组);
    // 防御性兼容数组包对象 (parse_records 的数组路径)。
    let value: Value = serde_json::from_slice(&payload.data)
        .map_err(|e| err(capability, format!("T0Evidence 视图非 JSON: {e}")))?;
    let view = value.as_array().and_then(|arr| arr.first()).unwrap_or(&value);
    let records = view
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| err(capability, "T0Evidence records 非数组"))?;
    let mut out = Vec::new();
    for v in records {
        let quote_obj = v
            .get("quote")
            .and_then(Value::as_object)
            .ok_or_else(|| err(capability, "T0Evidence quote 非对象"))?;
        let book = |key: &str| -> Result<[T0BookLevel; 5], GatewayError> {
            let arr = quote_obj
                .get(key)
                .and_then(Value::as_array)
                .ok_or_else(|| err(capability, format!("T0 quote.{key} 非数组")))?;
            let mut levels: Vec<T0BookLevel> = Vec::new();
            for item in arr {
                let obj = item
                    .as_object()
                    .ok_or_else(|| err(capability, format!("T0 quote.{key} 元素非对象")))?;
                levels.push(T0BookLevel {
                    price: obj
                        .get("price")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| err(capability, format!("T0 quote.{key}[].price 非法")))?,
                    volume: obj
                        .get("volume")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| err(capability, format!("T0 quote.{key}[].volume 非法")))?,
                });
            }
            <[T0BookLevel; 5]>::try_from(levels)
                .map_err(|_| err(capability, format!("T0 quote.{key} 长度必须为 5")))
        };
        let settled_daily = v
            .get("settled_daily")
            .and_then(Value::as_array)
            .ok_or_else(|| err(capability, "T0Evidence settled_daily 非数组"))?
            .iter()
            .map(|b| {
                Ok(MagicTdxT0DailyBar {
                    date: as_date(b, "date", capability)?,
                    open: as_f64(b, "open", capability)?,
                    high: as_f64(b, "high", capability)?,
                    low: as_f64(b, "low", capability)?,
                    close: as_f64(b, "close", capability)?,
                    volume: as_f64(b, "volume", capability)?,
                    amount: as_f64(b, "amount", capability)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let completed_five_minute = v
            .get("completed_five_minute")
            .and_then(Value::as_array)
            .ok_or_else(|| err(capability, "T0Evidence completed_five_minute 非数组"))?
            .iter()
            .map(|b| {
                Ok(MagicTdxT0FiveMinuteBar {
                    at: as_rfc3339(b, "at", capability)?.naive_utc(),
                    open: as_f64(b, "open", capability)?,
                    high: as_f64(b, "high", capability)?,
                    low: as_f64(b, "low", capability)?,
                    close: as_f64(b, "close", capability)?,
                    volume: as_f64(b, "volume", capability)?,
                    amount: as_f64(b, "amount", capability)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let code = as_str(v, "code", capability)?;
        out.push(MagicTdxT0Evidence {
            instrument: instrument_for(&code, capability)?,
            code,
            requested_at: as_rfc3339(v, "requested_at", capability)?,
            source_at: as_rfc3339(v, "source_at", capability)?,
            observed_at: as_rfc3339(v, "observed_at", capability)?,
            batch_id: as_str(v, "batch_id", capability)?,
            quote: MagicTdxT0Quote {
                price: quote_obj
                    .get("price")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.price 非法"))?,
                last_close: quote_obj
                    .get("last_close")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.last_close 非法"))?,
                open: quote_obj
                    .get("open")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.open 非法"))?,
                high: quote_obj
                    .get("high")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.high 非法"))?,
                low: quote_obj
                    .get("low")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.low 非法"))?,
                volume: quote_obj
                    .get("volume")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.volume 非法"))?,
                amount: quote_obj
                    .get("amount")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.amount 非法"))?,
                bids: book("bids")?,
                asks: book("asks")?,
            },
            settled_daily,
            completed_five_minute,
            intraday_average_price: as_f64(v, "intraday_average_price", capability)?,
        });
    }
    let rejections = view
        .get("rejections")
        .and_then(Value::as_array)
        .ok_or_else(|| err(capability, "T0Evidence rejections 非数组"))?
        .iter()
        .map(|r| {
            let code = as_str(r, "code", capability)?;
            Ok(MagicTdxT0Rejection {
                code,
                reason_code: Box::leak(
                    as_str(r, "reason_code", capability)?.into_boxed_str(),
                ),
                detail: as_str(r, "detail", capability)?,
                retryable: as_bool(r, "retryable", capability)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    // 批级时间戳: source_at/observed_at 从批级证据取 (服务端 pack_ev 来源);
    // requested_at 记录级有 (同批一致), 空批时以 observed_at 兜底 (同机取数时刻)。
    let requested_at = out
        .first()
        .map(|r| r.requested_at)
        .unwrap_or_else(|| ev.observed_at.parse().unwrap_or_else(|_| Utc::now()));
    Ok(MagicTdxT0Batch {
        requested_at,
        source_at: ev
            .source_at
            .as_deref()
            .ok_or_else(|| err(capability, "T0Evidence source_at 缺失"))?
            .parse()
            .map_err(|e| err(capability, format!("T0Evidence source_at 非法 ({e})")))?,
        observed_at: ev
            .observed_at
            .parse()
            .map_err(|e| err(capability, format!("T0Evidence observed_at 非法 ({e})")))?,
        batch_id: ev.batch_id.clone(),
        records: out,
        rejections,
    })
}

/// as_optional_u32: JSON null → None; 缺失 → Err fail-closed。
fn as_optional_u32(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<Option<u32>, GatewayError> {
    match v.get(key) {
        None => Err(err(capability, format!("字段 {key} 缺失"))),
        Some(Value::Null) => Ok(None),
        Some(x) => Ok(Some(
            x.as_u64()
                .ok_or_else(|| err(capability, format!("字段 {key} 非整数")))? as u32,
        )),
    }
}

/// Debug 名 → GeneralWebResearchProvider。服务端 pack_ev 用 format!("{:?}", provider)
/// 写 JSON ("Bocha"/"Tavily"/"SerpApi" — 不在 ProviderId 解析表内, 不能复用 parse_provider)。
/// 未知/空 → Err (fail-closed)。
fn parse_general_web_provider(s: &str) -> Result<GeneralWebResearchProvider, GatewayError> {
    Ok(match s {
        "Bocha" => GeneralWebResearchProvider::Bocha,
        "Tavily" => GeneralWebResearchProvider::Tavily,
        "SerpApi" => GeneralWebResearchProvider::SerpApi,
        _ => {
            return Err(err(
                "SemanticSearch",
                format!("selected_provider 无法解析为 GeneralWebResearchProvider: {s}"),
            ))
        }
    })
}

/// 语义检索桥。视图: delegate.rs fetch_semantic_search (:1170) — 服务端
/// records = serde_json::to_value(GeneralWebResearchRecord) 直出 (snake_case serde,
/// 含 record 级 evidence 子对象), selected_provider = Debug 名。批级 evidence
/// 客户端重建 (query 客户端已知; use_scope 恒 ResearchOnly — 本地 admit_records 语义)。
/// record 级 wire 完整性: serde round-trip + evidence 归属 (batch_id/provider) 与批级一致。
pub fn semantic_search(
    q: &QueryResult,
    query: &str,
) -> Result<GeneralWebResearchBatch, GatewayError> {
    let capability = "SemanticSearch";
    let provider = parse_general_web_provider(&q.selected_provider)?;
    if q.source.is_empty() {
        return Err(err(capability, "source 空 (服务端未回填证据链)"));
    }
    if q.batch_id.is_empty() {
        return Err(err(capability, "batch_id 空 (服务端未回填证据链)"));
    }
    let evidence = GeneralWebResearchBatchEvidence {
        provider,
        source: q.source.clone(),
        query: query.to_string(),
        observed_at: record_observed_at(q, capability)?,
        batch_id: q.batch_id.clone(),
        use_scope: ResearchUseScope::ResearchOnly,
    };
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GeneralWebResearchBatch::VerifiedEmpty(evidence));
    }
    let records: Vec<GeneralWebResearchRecord> = parsed
        .iter()
        .map(|v| {
            let record: GeneralWebResearchRecord = serde_json::from_value(v.clone())
                .map_err(|e| err(capability, format!("记录非 GeneralWebResearchRecord: {e}")))?;
            if record.evidence.batch_id != evidence.batch_id {
                return Err(err(capability, "记录 evidence.batch_id 与批级不一致"));
            }
            if record.evidence.provider != evidence.provider {
                return Err(err(capability, "记录 evidence.provider 与批级不一致"));
            }
            Ok(record)
        })
        .collect::<Result<_, _>>()?;
    Ok(GeneralWebResearchBatch::Available { records, evidence })
}

/// 公司行动桥。视图: delegate.rs fetch_corporate_actions (:1132)
/// {"code","category"(Debug 名),"effective_on","record_on","ex_on","payable_on",
/// "terms"(serde_json::to_value(CorporateActionTerms))}。
/// 注意: 视图无法区分 Available-空 与 VerifiedEmpty (服务端只回 state.records());
/// 消费方 (historical_bars.rs:1156-1173) 对两者同等对待 — 仅 Unavailable 失败,
/// 与 evidence_of 的 evidence 存在性语义一致 → 空批统一映射 VerifiedEmpty。
pub fn corporate_actions(
    q: &QueryResult,
) -> Result<GatewayBatch<ImplementedCorporateAction>, GatewayError> {
    let capability = "CorporateActions";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<ImplementedCorporateAction> = parsed
        .iter()
        .map(|v| {
            let category: CorporateActionCategory = serde_json::from_value(
                v.get("category")
                    .cloned()
                    .ok_or_else(|| err(capability, "字段 category 缺失"))?,
            )
            .map_err(|e| err(capability, format!("category 非合法行动类别: {e}")))?;
            let terms: CorporateActionTerms = serde_json::from_value(
                v.get("terms")
                    .cloned()
                    .ok_or_else(|| err(capability, "字段 terms 缺失"))?,
            )
            .map_err(|e| err(capability, format!("terms 非合法条款: {e}")))?;
            Ok(ImplementedCorporateAction {
                code: as_str(v, "code", capability)?,
                category,
                effective_on: as_date(v, "effective_on", capability)?,
                record_on: as_optional_date(v, "record_on", capability)?,
                ex_on: as_optional_date(v, "ex_on", capability)?,
                payable_on: as_optional_date(v, "payable_on", capability)?,
                terms,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// outcome 错误 wire 分类 → 静态常量对。服务端只发 review.rs GatewayError 构造器
/// 集合 (unavailable/partial/invalid_request/audit_failure + map_tdx_error 的
/// provider_* 对); 未知对 = 版本偏移 → 兜底 ("unavailable", "no_verified_batch")
/// 保持 fail-closed 重试语义 (retryable 由 wire 原样重建, 不受影响)。
fn rebuild_outcome_classification(
    audit_outcome: &str,
    reason_code: &str,
) -> (&'static str, &'static str) {
    match (audit_outcome, reason_code) {
        ("partial", "invalid_evidence") => ("partial", "invalid_evidence"),
        ("invalid_request", "invalid_request") => ("invalid_request", "invalid_request"),
        ("invalid_request", "unsupported_window") => ("invalid_request", "unsupported_window"),
        ("partial", "provider_invalid_data") => ("partial", "provider_invalid_data"),
        ("unavailable", "provider_unavailable") => ("unavailable", "provider_unavailable"),
        ("unavailable", "acquisition_audit_unavailable") => {
            ("unavailable", "acquisition_audit_unavailable")
        }
        _ => ("unavailable", "no_verified_batch"),
    }
}

/// outcome 复盘日线 (P4 M3): 服务端 adaptive 抓取完整视图
/// {"batch": DataBatch<Bar> 直出, "attempts": [Preimage], "error": {...}|null}。
/// batch/attempts 双向 serde 保真重建 (Bar 有 manual Deserialize, provider.rs:1469);
/// error 视图 → GatewayError::classified 重建 (capability 恒为本网关静态
/// "OutcomeDailyBarsV2", provider 经 parse_provider 回映, 分类经
/// rebuild_outcome_classification)。
pub fn outcome_daily_bars(
    q: &QueryResult,
) -> Result<RawOutcomeFetch, OutcomeTransportFailure> {
    let capability = "OutcomeDailyBarsV2";
    let payload = q.records.first().ok_or_else(|| {
        OutcomeTransportFailure::new(
            err(capability, "records 空 (服务端无 canonical payload)"),
            Vec::new(),
        )
    })?;
    let parsed: Value = serde_json::from_slice(&payload.data)
        .map_err(|e| OutcomeTransportFailure::new(err(capability, format!("视图非 JSON: {e}")), Vec::new()))?;
    let view = parsed
        .as_object()
        .ok_or_else(|| OutcomeTransportFailure::new(err(capability, "outcome 视图不是对象"), Vec::new()))?;
    let attempts = serde_json::from_value::<Vec<OutcomeTransportAttemptPreimage>>(
        view.get("attempts").cloned().unwrap_or(Value::Null),
    )
    .map_err(|e| {
        OutcomeTransportFailure::new(
            err(capability, format!("attempts 重建失败: {e}")),
            Vec::new(),
        )
    })?;
    if let Some(error_view) = view.get("error") {
        if !error_view.is_null() {
            let provider = error_view
                .get("provider")
                .and_then(Value::as_str)
                .and_then(|s| parse_provider(s).ok());
            let (audit_outcome, reason_code) = rebuild_outcome_classification(
                error_view.get("audit_outcome").and_then(Value::as_str).unwrap_or("unavailable"),
                error_view.get("reason_code").and_then(Value::as_str).unwrap_or("no_verified_batch"),
            );
            let retryable = error_view.get("retryable").and_then(Value::as_bool).unwrap_or(true);
            let message = error_view
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("(no message)")
                .to_string();
            return Err(OutcomeTransportFailure::new(
                GatewayError::classified(capability, provider, audit_outcome, reason_code, retryable, message),
                attempts,
            ));
        }
    }
    let batch = serde_json::from_value::<DataBatch<Bar>>(
        view.get("batch")
            .cloned()
            .ok_or_else(|| OutcomeTransportFailure::new(err(capability, "outcome 视图缺 batch"), Vec::new()))?,
    )
    .map_err(|e| {
        OutcomeTransportFailure::new(
            err(capability, format!("batch 重建失败: {e}")),
            Vec::new(),
        )
    })?;
    Ok(RawOutcomeFetch { batch, attempts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::{AdmissionState, CanonicalPayload};

    fn mk_q(data: &str, provider: &str, source: &str) -> QueryResult {
        QueryResult {
            admission: AdmissionState::Admitted,
            selected_provider: provider.to_string(),
            batch_id: "b-1".to_string(),
            complete: true,
            observed_at: "2026-08-15T10:00:00+08:00".to_string(),
            source_at: "2026-08-15T09:35:00+08:00".to_string(),
            records: vec![CanonicalPayload {
                schema: "x".to_string(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_string(),
                data: data.as_bytes().to_vec(),
            }],
            source: source.to_string(),
        }
    }

    #[test]
    fn provider_debug_names_roundtrip() {
        assert_eq!(parse_provider("Tdx").unwrap(), ProviderId::Tdx);
        assert_eq!(parse_provider("Eastmoney").unwrap(), ProviderId::Eastmoney);
        assert!(parse_provider("").is_err());
        assert!(parse_provider("Mystery").is_err());
    }

    #[test]
    fn realtime_quotes_canned_roundtrip() {
        // 与 delegate.rs fetch_realtime_quotes 视图字段一致 (交叉引用)。
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","price":1500.0,"change_pct":2.34,"previous_close":1465.7}]"#,
            "Tdx",
            "tdx",
        );
        let batch = realtime_quotes(&q).unwrap();
        let records = batch.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].code, "600519");
        assert_eq!(records[0].change_percent, 2.34);
        assert_eq!(records[0].provider, ProviderId::Tdx);
        assert_eq!(batch.evidence().source, "tdx");
        assert_eq!(
            batch.evidence().source_at.as_deref(),
            Some("2026-08-15T09:35:00+08:00")
        );
    }

    #[test]
    fn realtime_quotes_rejects_missing_change_pct() {
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","price":1500.0,"previous_close":1465.7}]"#,
            "Tdx",
            "tdx",
        );
        assert!(realtime_quotes(&q).is_err());
    }

    #[test]
    fn empty_records_is_verified_empty() {
        let q = mk_q("[]", "Tdx", "tdx");
        let batch = realtime_quotes(&q).unwrap();
        assert!(batch.is_verified_empty());
    }

    #[test]
    fn missing_evidence_is_fail_closed() {
        // provider 空 → Err, 不静默猜 Tdx。
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","price":1.0,"change_pct":0.0,"previous_close":1.0}]"#,
            "",
            "tdx",
        );
        assert!(realtime_quotes(&q).is_err());
        // source 空 → Err。
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","price":1.0,"change_pct":0.0,"previous_close":1.0}]"#,
            "Tdx",
            "",
        );
        assert!(realtime_quotes(&q).is_err());
    }

    #[test]
    fn order_books_five_levels_padded() {
        // 视图 bids/asks 各 2 档 → 补足 5 档 (空档 = 无挂单)。
        let q = mk_q(
            r#"[{"code":"600519","bids":[{"price":1500.0,"quantity":100.0},{"price":1499.0,"quantity":200.0}],"asks":[{"price":1501.0,"quantity":50.0}],"total_bid_quantity":300.0,"total_ask_quantity":50.0,"source_at":"2026-08-15T09:35:00+08:00"}]"#,
            "Tdx",
            "tdx",
        );
        let batch = order_books(&q).unwrap();
        let records = batch.records();
        assert_eq!(records[0].bids[0].price, 1500.0);
        assert_eq!(records[0].bids[2].price, 0.0, "缺档补 0.0");
        assert_eq!(records[0].bids[2].quantity, 0.0);
        assert_eq!(records[0].asks[0].price, 1501.0);
        assert_eq!(records[0].asks[4].price, 0.0);
    }

    #[test]
    fn security_metadata_board_and_date_parsed() {
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","board":"Main","is_st":false,"listed_on":"2001-08-27","price_limit_percent":10.0,"source_at":"2026-08-15T09:35:00+08:00"}]"#,
            "Tdx",
            "tdx",
        );
        let batch = security_metadata(&q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.board, SecurityBoard::Main);
        assert_eq!(r.listed_on, NaiveDate::from_ymd_opt(2001, 8, 27).unwrap());
        assert!(!r.is_st);
        // 视图无 price_limit_version → 留空 (消费方按缺证据感知)。
        assert!(r.price_limit_version.is_empty());
    }

    #[test]
    fn historical_bars_filters_to_target_code() {
        let q = mk_q(
            r#"[{"code":"600519","date":"2026-08-14","open":1480.0,"high":1510.0,"low":1475.0,"close":1500.0,"volume":123456.0,"amount":1.85e9,"pct_chg":1.3,"settled":true}]"#,
            "Tdx",
            "tdx",
        );
        let batch = historical_bars("600519", &q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.date, NaiveDate::from_ymd_opt(2026, 8, 14).unwrap());
        assert_eq!(r.close, 1500.0);
        assert!(r.settled);
        assert_eq!(r.adjust, AdjustType::None);
        assert!(r.pe_ratio.is_none(), "视图子集字段 = None (不发明数据)");
    }

    #[test]
    fn historical_bars_rejects_foreign_code() {
        let q = mk_q(
            r#"[{"code":"000001","date":"2026-08-14","open":10.0,"high":11.0,"low":9.0,"close":10.5,"volume":1.0,"amount":2.0,"pct_chg":0.5,"settled":true}]"#,
            "Tdx",
            "tdx",
        );
        assert!(historical_bars("600519", &q).is_err());
    }

    #[test]
    fn minute_data_optional_cumulative_amount() {
        let q = mk_q(
            r#"[{"code":"600519","minute_at":"2026-08-15T09:35:00+08:00","price":1500.0,"cumulative_quantity":100.0,"cumulative_amount":null,"source_at":"2026-08-15T09:35:00+08:00"}]"#,
            "Tdx",
            "tdx",
        );
        let batch = minute_data(&q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.code, "600519");
        assert!(r.cumulative_amount.is_none());
    }

    #[test]
    fn money_flows_all_five_nets() {
        let q = mk_q(
            r#"[{"code":"600519","main_net":1.0,"super_large_net":2.0,"large_net":3.0,"medium_net":4.0,"small_net":5.0,"source_at":"2026-08-15T09:35:00+08:00"}]"#,
            "Tdx",
            "tdx",
        );
        let batch = money_flows(&q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.main_net, 1.0);
        assert_eq!(r.small_net, 5.0);
        assert_eq!(r.provider, ProviderId::Tdx);
    }

    #[test]
    fn semantic_search_canned_roundtrip() {
        // 视图: delegate.rs fetch_semantic_search — to_value(GeneralWebResearchRecord)
        // 直出 (snake_case serde, 含 record 级 evidence 子对象)。
        let q = mk_q(
            r#"[{"title":"白酒行业景气度跟踪","snippet":"2026年中报白酒板块营收同比增长 8.2%","url":"https://example.com/ws1","publisher":"国泰君安证券","published_at_raw":"2026-08-15T09:00:00+08:00","published_at":"2026-08-15T09:00:00+08:00","evidence":{"provider":"bocha","observed_at":"2026-08-15T09:00:00+08:00","batch_id":"b-1","item_id":"ws-1","publication_quality":"exact_provider_time","use_scope":"research_only"}}]"#,
            "Bocha",
            "bocha-general-web",
        );
        let batch = semantic_search(&q, "白酒 景气").unwrap();
        let (records, evidence) = match batch {
            GeneralWebResearchBatch::Available { records, evidence } => (records, evidence),
            GeneralWebResearchBatch::VerifiedEmpty(_) => panic!("fixture 不应为空"),
        };
        assert_eq!(evidence.provider, GeneralWebResearchProvider::Bocha);
        assert_eq!(evidence.query, "白酒 景气");
        assert_eq!(evidence.use_scope, ResearchUseScope::ResearchOnly);
        assert_eq!(records[0].title, "白酒行业景气度跟踪");
        assert_eq!(records[0].evidence.batch_id, "b-1");
    }

    #[test]
    fn corporate_actions_canned_roundtrip() {
        // 视图: delegate.rs fetch_corporate_actions — category = Debug 名,
        // terms = to_value(CorporateActionTerms) (externally-tagged)。
        let q = mk_q(
            r#"[{"code":"600519","category":"Distribution","effective_on":"2026-08-20","record_on":"2026-08-13","ex_on":"2026-08-19","payable_on":"2026-08-21","terms":{"Distribution":{"cash_per_share":0.15,"bonus_per_share":null,"rights_per_share":null,"rights_price":null}}}]"#,
            "Tdx",
            "tdx",
        );
        let batch = corporate_actions(&q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.code, "600519");
        assert_eq!(r.category, CorporateActionCategory::Distribution);
        assert_eq!(r.effective_on, NaiveDate::from_ymd_opt(2026, 8, 20).unwrap());
        assert_eq!(r.ex_on, Some(NaiveDate::from_ymd_opt(2026, 8, 19).unwrap()));
        assert_eq!(r.payable_on, Some(NaiveDate::from_ymd_opt(2026, 8, 21).unwrap()));
        match &r.terms {
            CorporateActionTerms::Distribution { cash_per_share, .. } => {
                assert_eq!(cash_per_share.map(|c| c.get()), Some(0.15));
            }
            _ => panic!("terms 变体不符"),
        }
    }

    #[test]
    fn outcome_daily_bars_canned_roundtrip() {
        // 视图: delegate.rs fetch_outcome_daily_bars — batch = to_value(DataBatch<Bar>)
        // (Bar serde 字段 = provider.rs Repr), attempts = to_value(Preimage), error null。
        let q = mk_q(
            r#"{"batch":{"records":[{"instrument":{"exchange":"Shanghai","code":"600519","asset_class":"Equity"},"interval":"Day","bar_start":"2026-08-14","bar_end":"2026-08-14","open":1480.0,"high":1510.0,"low":1475.0,"close":1500.0,"volume":123456.0,"amount":1.85e9,"adjustment":"Unadjusted","source_at":null,"provider":"Tdx","batch_id":"fixture-ob"}],"provenance":{"source":"tdx","source_at":null,"fetched_at":"2026-08-15T10:00:00+08:00","batch_id":"fixture-ob"},"quality":{"complete":true,"issues":[]}},"attempts":[],"error":null}"#,
            "Tdx",
            "tdx",
        );
        let raw = outcome_daily_bars(&q).unwrap();
        assert_eq!(raw.batch.records().len(), 1, "batch 重建保真");
        let bar = &raw.batch.records()[0];
        assert_eq!(bar.instrument().code(), "600519", "bar instrument 保真");
        assert_eq!(bar.bar_start(), "2026-08-14", "bar_start 保真");
        assert_eq!(bar.close().get(), 1500.0, "bar close 保真");
        assert!(raw.batch.quality().is_complete(), "quality 保真");
        assert!(raw.attempts.is_empty(), "attempts 保真");
    }

    #[test]
    fn outcome_daily_bars_error_view_rebuild() {
        // error 视图 → OutcomeTransportFailure { error: classified 重建, attempts }。
        let q = mk_q(
            r#"{"batch":null,"attempts":[],"error":{"capability":"OutcomeDailyBarsV2","provider":"Tdx","audit_outcome":"unavailable","reason_code":"provider_unavailable","retryable":true,"message":"Magic TDX outcome bars failed: boom"}}"#,
            "Tdx",
            "tdx",
        );
        let failure = outcome_daily_bars(&q).unwrap_err();
        assert_eq!(failure.error.reason_code(), "provider_unavailable", "reason_code 重建");
        assert_eq!(failure.error.audit_outcome(), "unavailable", "audit_outcome 重建");
        assert!(failure.error.retryable(), "retryable 重建");
        assert_eq!(failure.error.provider(), Some(ProviderId::Tdx), "provider 回映");
        assert_eq!(failure.error.capability(), "OutcomeDailyBarsV2", "capability 静态");
        assert!(failure.attempts.is_empty(), "attempts 保真");
    }

    #[test]
    fn outcome_daily_bars_unknown_classification_falls_back_fail_closed() {
        // 未知 audit_outcome/reason_code 对 (版本偏移) → 兜底 no_verified_batch,
        // retryable 仍按 wire 原样 (fail-closed 语义不因版本偏移丢失)。
        let q = mk_q(
            r#"{"batch":null,"attempts":[],"error":{"capability":"OutcomeDailyBarsV9","provider":"Tdx","audit_outcome":"mystery","reason_code":"mystery","retryable":true,"message":"skew"}}"#,
            "Tdx",
            "tdx",
        );
        let failure = outcome_daily_bars(&q).unwrap_err();
        assert_eq!(failure.error.reason_code(), "no_verified_batch");
        assert!(failure.error.retryable());
    }
}
