//! v16.5 #2 完整化 helper: 7 stub strategy 共享真数据读取
//!
// 从 StrategyInput.metric_json 解析 vol_ratio / price_chg_pct / sector / push_subkind.
// 8 strategy 都用这个, 避免每 impl 重复 serde 解析.
// v16.5 #2: 内部调 quote_provider().get_quote_price(code) 拿真市价, 0.0 fallback push_price.

use serde_json::Value;

pub struct MetricFields {
    pub vol_ratio: f64,
    pub price_chg_pct: f64,
    pub sector: String,
    pub push_subkind: String,
    pub code: String,
    /// v16.5 #2: 真市价 (quote_provider) 优先 push_price fallback
    pub quote_price: f64,
    pub push_price: f64,
}

pub fn parse(metric_json: &str, code: &str, push_price: f64) -> MetricFields {
    let m: Value = serde_json::from_str(metric_json).unwrap_or_default();
    // v16.5 #2: 真市价 优先 push_price fallback
    let quote_price = crate::broker::quote_provider()
        .map(|p| p.get_quote_price(code))
        .filter(|&p| p > 0.0)
        .unwrap_or(push_price);
    MetricFields {
        vol_ratio: m.get("vol_ratio").and_then(|v| v.as_f64()).unwrap_or(0.0),
        price_chg_pct: m.get("price_chg_pct").and_then(|v| v.as_f64()).unwrap_or(0.0),
        sector: m.get("sector").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        push_subkind: m.get("push_subkind").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        code: code.to_string(),
        quote_price,
        push_price,
    }
}
