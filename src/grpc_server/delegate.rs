//! data_gateway 委托层 (方案 A): 服务端进程内调用 data_gateway 取真实数据,
//! 序列化为 canonical JSON。fixture_mode 下不经过这里。
//! 每个 op 一个 fetch_xxx(schema: &str) -> Result<Fetched, String>。
use crate::grpc_client::pb::magic::market::v1::Operation;

pub struct Fetched {
    pub data: Vec<u8>,
    pub source_at: String,
}

fn not_yet(op: Operation) -> Result<Fetched, String> {
    Err(format!(
        "{}: delegate 尚未实现 (Task 9/10 补全)",
        crate::grpc_contract::ops::method_name(op)
    ))
}

pub fn fetch(op: Operation, schema: &str) -> Result<Fetched, String> {
    let _ = schema;
    match op {
        Operation::RealtimeQuotes => fetch_realtime_quotes(),
        Operation::HistoricalBars => fetch_historical_bars(),
        Operation::MinuteData => fetch_minute_data(),
        Operation::Announcements => fetch_announcements(),
        Operation::GlobalNews => fetch_global_news(),
        Operation::SecurityMetadata => fetch_security_metadata(),
        _ => not_yet(op),
    }
}

/// 真实路径: 统一实时行情 Gateway。
/// 字段映射以实际 struct 为准: RealtimeMarketQuote 有
/// code/name/price/previous_close/change_percent (无 volume/amount)。
pub fn fetch_realtime_quotes() -> Result<Fetched, String> {
    let codes = std::env::var("STOCK_LIST")
        .map(|s| {
            s.split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let batch = crate::data_gateway::MarketDataGateway::new()
        .realtime_quotes(&codes)
        .map_err(|e| format!("统一实时行情 Gateway 不可用: {e}"))?;
    let records: Vec<serde_json::Value> = batch
        .records()
        .iter()
        .map(|s| {
            serde_json::json!({
                "code": s.code,
                "name": s.name,
                "price": s.price,
                "change_pct": s.change_percent,
                "previous_close": s.previous_close,
            })
        })
        .collect();
    Ok(Fetched {
        data: serde_json::to_vec(&records).map_err(|e| e.to_string())?,
        source_at: chrono::Local::now().to_rfc3339(),
    })
}

// Task 9/10: 其余 5 个代表 op + 全部生产 op 的 fetch_xxx 逐个落地;
// 每个 op 落地时先 grep data_gateway 对应 Gateway 的返回类型字段名再写 JSON 映射。
fn fetch_historical_bars() -> Result<Fetched, String> { not_yet(Operation::HistoricalBars) }
fn fetch_minute_data() -> Result<Fetched, String> { not_yet(Operation::MinuteData) }
fn fetch_announcements() -> Result<Fetched, String> { not_yet(Operation::Announcements) }
fn fetch_global_news() -> Result<Fetched, String> { not_yet(Operation::GlobalNews) }
fn fetch_security_metadata() -> Result<Fetched, String> { not_yet(Operation::SecurityMetadata) }
