//! ExternalV1（Windows gRPC）日线尾部拉取。
//!
//! 为什么有这条：2026-10-19 起本机桥的 `HistoricalBars` 四路 provider
//! （Tdx/Tencent/Sina/Baidu）全部取不到日线，唯一稳定的替代是 Windows VM 侧的
//! ExternalV1。它的 Tencent 臂只接受 `limit` 语义（拒绝 `start`/`end`），
//! 所以这里发的是"尾部 N 根"，由调用方按 `bar_start` 过滤目标交易日。
//!
//! 边界：只支持沪深股票（`AssetClass::Equity`）。指数、北交所在 ExternalV1
//! 上没有已验证的单位语义（`asset class Index has unverified field units`），
//! 本模块不吞掉这个事实，直接返回错误。

use std::path::Path;

use crate::grpc_client::client::external_historical_read::{
    ExternalHistoricalObservation, ExternalHistoricalReadClient,
};
use crate::grpc_client::external_v1::build_external_historical_bars_query_request;
use crate::market_domain::{AssetClass, Exchange, InstrumentId};

/// ExternalV1 的日线记录 schema。
const BAR_SCHEMA: &str = "magic.market.bar";

/// 一根已归一化的日线。
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalDailyBar {
    pub code: String,
    /// `YYYY-MM-DD`
    pub date: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// 手（100 股），与 `stock_daily.volume` 同口径。
    pub volume: f64,
    /// Tencent 臂不给成交额，为 `None`。
    pub amount: Option<f64>,
    pub provider: String,
}

fn f64_field(value: &serde_json::Value, key: &str) -> Result<f64, String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| format!("缺少数值字段 {key}"))
}

fn string_field(value: &serde_json::Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("缺少字符串字段 {key}"))
}

/// 从一次观察里解出日线；`schema` 不符的记录被跳过。
fn decode_bars(code: &str, observation: &ExternalHistoricalObservation) -> Result<Vec<ExternalDailyBar>, String> {
    let mut bars = Vec::new();
    let result = observation
        .result
        .as_ref()
        .map_err(|error| format!("远端拒绝: {error}"))?;
    for record in &result.records {
        if record.schema != BAR_SCHEMA {
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(&record.data)
            .map_err(|error| format!("记录解码失败: {error}"))?;
        let bar_end = string_field(&value, "bar_end")?;
        let bar_start = string_field(&value, "bar_start")?;
        if bar_start != bar_end {
            return Err(format!("非单日记录: {bar_start}..{bar_end}"));
        }
        bars.push(ExternalDailyBar {
            code: code.to_owned(),
            date: bar_end,
            open: f64_field(&value, "open")?,
            high: f64_field(&value, "high")?,
            low: f64_field(&value, "low")?,
            close: f64_field(&value, "close")?,
            volume: f64_field(&value, "volume")?,
            amount: value.get("amount").and_then(serde_json::Value::as_f64),
            provider: value
                .get("provider")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("ExternalV1")
                .to_owned(),
        });
    }
    if !result.complete {
        return Err("响应不完整（complete=false）".to_owned());
    }
    Ok(bars)
}

/// 拉取某只票 `[start, end]` 窗口内的日线。
///
/// provider 固定为 HithinkFinance：ExternalV1 上只有它同时满足
/// "接受显式日期区间" 与 "返回成交额" 两个条件。Tencent 臂拒绝区间，
/// Tdx 臂要求省略区间，Baidu 臂缺连续性证据——都会被服务端以 Unimplemented 拒绝。
///
/// 失败原因分三类：连接/健康（`connect`）、远端拒绝（`query`）、记录解码。
/// 调用方应当把失败码写进审计，而不是静默重试。
pub async fn fetch_daily_bars_window(
    bundle: &Path,
    exchange: Exchange,
    code: &str,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
    limit: u32,
) -> Result<Vec<ExternalDailyBar>, String> {
    let instrument = InstrumentId::new(exchange, code, AssetClass::Equity)
        .map_err(|error| format!("非法标的: {error}"))?;
    let query = build_external_historical_bars_query_request(&instrument, start, end, limit)
        .map_err(|error| format!("构造请求失败: {error:?}"))?;
    let mut reader = ExternalHistoricalReadClient::connect_client_bundle(bundle)
        .await
        .map_err(|error| format!("连接失败: {error}"))?;
    let observation = reader
        .query_once(query)
        .await
        .map_err(|error| format!("查询失败: {error}"))?;
    decode_bars(code, &observation)
}

/// 6 位代码 → 交易所；北交所（8/4/9 开头）不被 ExternalV1 覆盖，返回 `None`。
pub fn exchange_for_code(code: &str) -> Option<Exchange> {
    match code.as_bytes().first() {
        Some(b'6') => Some(Exchange::Shanghai),
        Some(b'0') | Some(b'3') => Some(Exchange::Shenzhen),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_inference_covers_sh_sz_and_rejects_bse() {
        assert_eq!(exchange_for_code("600396"), Some(Exchange::Shanghai));
        assert_eq!(exchange_for_code("002131"), Some(Exchange::Shenzhen));
        assert_eq!(exchange_for_code("300570"), Some(Exchange::Shenzhen));
        assert_eq!(exchange_for_code("920083"), None);
        assert_eq!(exchange_for_code("830799"), None);
    }
}
