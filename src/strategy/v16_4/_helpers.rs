//! v16.5 #2 完整化 helper: 策略共享结构化指标读取
//!
// 从 StrategyInput.metric_json 解析 vol_ratio / price_chg_pct / sector / push_subkind.
// 8 strategy 都用这个, 避免每 impl 重复 serde 解析.
// Realtime quotes are fetched only by strategies that actually require one.

use serde_json::Value;

pub struct MetricFields {
    pub vol_ratio: Option<f64>,
    pub price_chg_pct: Option<f64>,
    pub main_net_yi: Option<f64>,
    pub sector: Option<String>,
    pub push_subkind: Option<String>,
}

fn optional_finite_number(value: &Value, field: &str) -> Result<Option<f64>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => {
            let number = raw
                .as_f64()
                .ok_or_else(|| format!("metric_json.{field} 必须是数字"))?;
            if !number.is_finite() {
                return Err(format!("metric_json.{field} 非有限"));
            }
            Ok(Some(number))
        }
    }
}

fn optional_nonempty_string(value: &Value, field: &str) -> Result<Option<String>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => {
            let text = raw
                .as_str()
                .ok_or_else(|| format!("metric_json.{field} 必须是字符串"))?
                .trim();
            if text.is_empty() {
                return Err(format!("metric_json.{field} 不能为空"));
            }
            Ok(Some(text.to_string()))
        }
    }
}

pub fn parse(metric_json: &str, code: &str, push_price: f64) -> Result<MetricFields, String> {
    if !push_price.is_finite() || push_price <= 0.0 {
        return Err(format!("[{code}] push_price 非正或非有限: {push_price}"));
    }
    let value: Value = serde_json::from_str(metric_json)
        .map_err(|error| format!("[{code}] metric_json 解析失败: {error}"))?;
    if !value.is_object() {
        return Err(format!("[{code}] metric_json 必须是对象"));
    }
    Ok(MetricFields {
        vol_ratio: optional_finite_number(&value, "vol_ratio")?,
        price_chg_pct: optional_finite_number(&value, "price_chg_pct")?,
        main_net_yi: optional_finite_number(&value, "main_net_yi")?,
        sector: optional_nonempty_string(&value, "sector")?,
        push_subkind: optional_nonempty_string(&value, "push_subkind")?,
    })
}
