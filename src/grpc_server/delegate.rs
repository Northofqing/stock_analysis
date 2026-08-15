//! data_gateway 委托层 (方案 A): 服务端进程内调用 data_gateway 取真实数据,
//! 序列化为 canonical JSON。fixture_mode 下不经过这里。
//! 每个 op 一个 fetch_xxx(schema: &str) -> Result<Fetched, String>。
//!
//! 签名说明 (实测, 非计划假设): data_gateway 全部是 async fn (内部自行
//! spawn_blocking), 所以 fetch 本身是 async, handler 直接 await, 不套 spawn_blocking。
//! 记录结构体没有 derive Serialize → 逐字段 json! 映射 (字段名 = 结构体字段名)。
use crate::data_gateway::{
    BlockTradesGateway, ConsensusDataGateway, DragonTigerGateway, EconomicCalendarGateway,
    EventCalendarGateway, FuturesDeliveryGateway, GlobalMarketGateway, GlobalNewsGateway,
    GlobalNewsProvider, MarketCapabilitiesGateway,
};
use crate::grpc_client::pb::magic::market::v1::Operation;
use chrono::{Datelike, Local, NaiveDate};
use serde_json::{json, Value};

pub struct Fetched {
    pub data: Vec<u8>,
    pub source_at: String,
}

fn watchlist_codes() -> Vec<String> {
    std::env::var("STOCK_LIST")
        .map(|s| {
            s.split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// batch 的 evidence 可信源时间 (合同 §6: 缺则不填充)。
fn source_at_of(batch: &crate::data_gateway::GatewayBatch<impl Sized>) -> String {
    batch.evidence().source_at.clone().unwrap_or_default()
}

fn pack(records: Vec<Value>, source_at: String) -> Result<Fetched, String> {
    Ok(Fetched {
        data: serde_json::to_vec(&records).map_err(|e| e.to_string())?,
        source_at,
    })
}

fn not_yet(op: Operation) -> Result<Fetched, String> {
    Err(format!(
        "{}: delegate 尚未实现 (Task 10 补全)",
        crate::grpc_contract::ops::method_name(op)
    ))
}

pub async fn fetch(op: Operation, schema: &str) -> Result<Fetched, String> {
    let _ = schema;
    match op {
        Operation::RealtimeQuotes => fetch_realtime_quotes(),
        Operation::HistoricalBars => not_yet(op),
        Operation::MinuteData => fetch_minute_data().await,
        Operation::OrderBooks => fetch_order_books().await,
        Operation::MoneyFlows => fetch_money_flows().await,
        Operation::SecurityMetadata => fetch_security_metadata().await,
        Operation::GlobalIndices => fetch_global_indices().await,
        Operation::Announcements => fetch_announcements().await,
        Operation::GlobalNews => fetch_global_news().await,
        Operation::EconomicCalendar => fetch_economic_calendar().await,
        Operation::FuturesDelivery => fetch_futures_delivery().await,
        Operation::DragonTiger => fetch_dragon_tiger().await,
        Operation::BlockTrades => fetch_block_trades().await,
        Operation::Consensus => fetch_consensus().await,
        _ => not_yet(op),
    }
}

// ---------- 统一实时行情 (Task 8 已落地, 同步路径) ----------

/// 字段映射以实际 struct 为准: RealtimeMarketQuote 有
/// code/name/price/previous_close/change_percent (无 volume/amount)。
pub fn fetch_realtime_quotes() -> Result<Fetched, String> {
    let codes = watchlist_codes();
    let batch = crate::data_gateway::MarketDataGateway::new()
        .realtime_quotes(&codes)
        .map_err(|e| format!("统一实时行情 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
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
    pack(records, source_at)
}

// ---------- 核心 12 op (Task 9) ----------

async fn fetch_minute_data() -> Result<Fetched, String> {
    let gateway = MarketCapabilitiesGateway::new();
    let codes = watchlist_codes();
    let mut set = tokio::task::JoinSet::new();
    for code in codes {
        let gateway = gateway;
        set.spawn(async move {
            gateway
                .minute_data(&code, None)
                .await
                .map_err(|e| format!("分钟线 Gateway 不可用 ({code}): {e}"))
        });
    }
    let mut records: Vec<Value> = Vec::new();
    let mut source_at = String::new();
    while let Some(joined) = set.join_next().await {
        let batch = joined.map_err(|e| format!("分钟线 task 失败: {e}"))??;
        if source_at.is_empty() {
            source_at = source_at_of(&batch);
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
    pack(records, source_at)
}

async fn fetch_order_books() -> Result<Fetched, String> {
    let gateway = MarketCapabilitiesGateway::new();
    let batch = gateway
        .order_books(&watchlist_codes())
        .await
        .map_err(|e| format!("盘口 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
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
    pack(records, source_at)
}

async fn fetch_money_flows() -> Result<Fetched, String> {
    let gateway = MarketCapabilitiesGateway::new();
    let batch = gateway
        .money_flows(&watchlist_codes())
        .await
        .map_err(|e| format!("资金流 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
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
    pack(records, source_at)
}

async fn fetch_security_metadata() -> Result<Fetched, String> {
    let gateway = MarketCapabilitiesGateway::new();
    let batch = gateway
        .security_metadata(&watchlist_codes())
        .await
        .map_err(|e| format!("证券元数据 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
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
    pack(records, source_at)
}

async fn fetch_global_indices() -> Result<Fetched, String> {
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

async fn fetch_announcements() -> Result<Fetched, String> {
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

async fn fetch_global_news() -> Result<Fetched, String> {
    let gateway = GlobalNewsGateway::new();
    let batch = gateway
        .global_news(GlobalNewsProvider::Eastmoney, 30)
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
                "publisher": r.publisher,
                "url": r.canonical_url,
                "published_at": r.published_at.to_rfc3339(),
                "instruments": r.instruments,
                "topics": r.topics,
            })
        })
        .collect();
    pack(records, source_at)
}

async fn fetch_economic_calendar() -> Result<Fetched, String> {
    let gateway = EconomicCalendarGateway::new();
    let batch = gateway
        .latest_releases(50, None)
        .await
        .map_err(|e| format!("财经日历 Gateway 不可用: {e}"))?;
    let source_at = source_at_of(&batch);
    let records: Vec<Value> = batch
        .records()
        .iter()
        .map(|r| {
            json!({
                "event_id": r.event_id,
                "country": r.country,
                "name": r.name,
                "period": r.period,
                "scheduled_at": r.scheduled_at.to_rfc3339(),
                "previous": r.previous,
                "consensus": r.consensus,
                "actual": r.actual,
                "unit": r.unit,
                "importance": r.importance,
            })
        })
        .collect();
    pack(records, source_at)
}

async fn fetch_futures_delivery() -> Result<Fetched, String> {
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

async fn fetch_dragon_tiger() -> Result<Fetched, String> {
    let gateway = DragonTigerGateway::new();
    let batch = gateway
        .market_review(today(), 100, 20)
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
                "disclosures": r.disclosures.len(),
            })
        })
        .collect();
    pack(records, source_at)
}

async fn fetch_block_trades() -> Result<Fetched, String> {
    let gateway = BlockTradesGateway::new();
    let batch = gateway
        .market_review(&watchlist_codes(), today())
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

async fn fetch_consensus() -> Result<Fetched, String> {
    let gateway = ConsensusDataGateway::new();
    let codes = watchlist_codes();
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
