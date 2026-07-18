//! PE / PB 历史估值序列与分位计算
//!
//! 数据源：东方财富 datacenter `RPT_VALUEANALYSIS_DET`，按 TRADE_DATE 降序拉取约 3 年数据，
//! 计算当前 PE / PB 在历史区间内的分位百分位（0~100，越低越便宜）。
//!
//! URL 示例：
//! https://datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPT_VALUEANALYSIS_DET&
//! columns=SECURITY_CODE,TRADE_DATE,PE_TTM,PB_MRQ&filter=(SECURITY_CODE="600519")&
//! sortColumns=TRADE_DATE&sortTypes=-1&pageSize=750&pageNumber=1

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

/// 估值历史统计结果
#[derive(Debug, Clone)]
pub struct ValuationHistory {
    pub current_pe: Option<f64>,
    pub current_pb: Option<f64>,
    /// 当前 PE 在近 N 日历史中的分位（0~100，越低越便宜）
    pub pe_percentile: Option<f64>,
    pub pb_percentile: Option<f64>,
    pub pe_min: Option<f64>,
    pub pe_max: Option<f64>,
    pub pe_median: Option<f64>,
    pub pb_min: Option<f64>,
    pub pb_max: Option<f64>,
    pub pb_median: Option<f64>,
    pub sample_days: usize,
    pub oldest_date: Option<String>,
    pub newest_date: Option<String>,
}

fn optional_finite_f64(v: &Value, field: &str) -> Result<Option<f64>> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_f64()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| anyhow!("估值历史字段 {field} 不是有限数字: {v}")),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| anyhow!("估值历史字段 {field} 非法: {s:?}")),
        _ => Err(anyhow!("估值历史字段 {field} 类型非法: {v}")),
    }
}

fn percentile_of(sorted: &[f64], current: f64) -> f64 {
    // 当前值在升序序列中的排名 / 总数 * 100；相同则取中点
    let n = sorted.len() as f64;
    let less = sorted.iter().filter(|v| **v < current).count() as f64;
    let equal = sorted
        .iter()
        .filter(|v| (**v - current).abs() < 1e-9)
        .count() as f64;
    ((less + equal / 2.0) / n) * 100.0
}

fn min_max_median(values: &[f64]) -> (Option<f64>, Option<f64>, Option<f64>) {
    if values.is_empty() {
        return (None, None, None);
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = sorted.first().copied();
    let max = sorted.last().copied();
    let mid = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        Some((sorted[mid - 1] + sorted[mid]) / 2.0)
    } else {
        Some(sorted[mid])
    };
    (min, max, median)
}

/// BR-119: validate and aggregate one complete valuation-history protocol batch.
fn parse_valuation_history(json: &Value) -> Result<ValuationHistory> {
    let arr = json
        .get("result")
        .and_then(|result| result.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("估值历史无 result.data"))?;
    if arr.is_empty() {
        return Err(anyhow!("估值历史 data 为空"));
    }

    let mut pes: Vec<f64> = Vec::with_capacity(arr.len());
    let mut pbs: Vec<f64> = Vec::with_capacity(arr.len());
    let mut dates = Vec::with_capacity(arr.len());
    let mut current_pe = None;
    let mut current_pb = None;

    for (index, item) in arr.iter().enumerate() {
        let raw_date = item
            .get("TRADE_DATE")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("估值历史第 {} 行缺少 TRADE_DATE", index + 1))?;
        let date_text = raw_date.split_whitespace().next().unwrap_or(raw_date);
        let date = chrono::NaiveDate::parse_from_str(date_text, "%Y-%m-%d")
            .map_err(|error| anyhow!("估值历史第 {} 行日期非法: {error}", index + 1))?;
        dates.push(date);

        let pe = match item.get("PE_TTM") {
            Some(value) => optional_finite_f64(value, "PE_TTM")?,
            None => None,
        };
        let pb = match item.get("PB_MRQ") {
            Some(value) => optional_finite_f64(value, "PB_MRQ")?,
            None => None,
        };
        if index == 0 {
            current_pe = pe.filter(|value| *value > 0.0);
            current_pb = pb.filter(|value| *value > 0.0);
        }
        if let Some(value) = pe.filter(|value| *value > 0.0) {
            pes.push(value);
        }
        if let Some(value) = pb.filter(|value| *value > 0.0) {
            pbs.push(value);
        }
    }

    for pair in dates.windows(2) {
        let newer = pair[0];
        let older = pair[1];
        if newer <= older {
            return Err(anyhow!("估值历史日期重复或非降序: {newer} -> {older}"));
        }
        let expected = crate::calendar::next_trading_day(older);
        if newer != expected {
            return Err(anyhow!(
                "估值历史交易日断档: {older} 后应为 {expected}, 实际为 {newer}"
            ));
        }
    }

    let mut sorted_pe = pes.clone();
    sorted_pe.sort_by(|left, right| left.total_cmp(right));
    let mut sorted_pb = pbs.clone();
    sorted_pb.sort_by(|left, right| left.total_cmp(right));
    let pe_percentile = current_pe
        .filter(|_| sorted_pe.len() >= 30)
        .map(|current| percentile_of(&sorted_pe, current));
    let pb_percentile = current_pb
        .filter(|_| sorted_pb.len() >= 30)
        .map(|current| percentile_of(&sorted_pb, current));
    let (pe_min, pe_max, pe_median) = min_max_median(&pes);
    let (pb_min, pb_max, pb_median) = min_max_median(&pbs);

    Ok(ValuationHistory {
        current_pe,
        current_pb,
        pe_percentile,
        pb_percentile,
        pe_min,
        pe_max,
        pe_median,
        pb_min,
        pb_max,
        pb_median,
        sample_days: pes.len().max(pbs.len()),
        newest_date: dates.first().map(ToString::to_string),
        oldest_date: dates.last().map(ToString::to_string),
    })
}

/// 异步拉取并计算分位
pub async fn fetch_async(client: &reqwest::Client, code: &str) -> Result<ValuationHistory> {
    let filter = format!("(SECURITY_CODE=\"{}\")", code);
    let url = format!(
        "https://datacenter-web.eastmoney.com/api/data/v1/get?\
         reportName=RPT_VALUEANALYSIS_DET&\
         columns=SECURITY_CODE,TRADE_DATE,PE_TTM,PB_MRQ&\
         filter={}&sortColumns=TRADE_DATE&sortTypes=-1&pageSize=750&pageNumber=1",
        urlencoding::encode(&filter)
    );
    log::debug!("[估值历史] {}", url);

    let resp = client
        .get(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Referer", "https://data.eastmoney.com/")
        .send()
        .await
        .context("估值历史请求失败")?;
    if !resp.status().is_success() {
        return Err(anyhow!("估值历史状态码 {}", resp.status()));
    }
    let text = resp.text().await.context("估值历史读取响应失败")?;
    let json: Value = serde_json::from_str(&text).context("估值历史 JSON 解析失败")?;

    parse_valuation_history(&json)
}

/// 同步包装：在已有 tokio runtime 上下文内调用；无 runtime 时返回 None。
pub fn fetch_blocking(client: &reqwest::Client, code: &str) -> Option<ValuationHistory> {
    // 修复 Top10#5 (2026-06-29 audit): 用 crate::block_on_async 统一替代
    if tokio::runtime::Handle::try_current().is_err() {
        return None;
    }
    let client = client.clone();
    let code_s = code.to_string();
    crate::block_on_async(async move {
        match fetch_async(&client, &code_s).await {
            Ok(v) => Some(v),
            Err(e) => {
                log::warn!("[估值历史] {} 拉取失败: {}", code_s, e);
                None
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_statistics_cover_missing_odd_even_and_equal_values() {
        assert_eq!(
            optional_finite_f64(&serde_json::json!(1.5), "x").unwrap(),
            Some(1.5)
        );
        assert_eq!(
            optional_finite_f64(&serde_json::json!(" 2.5 "), "x").unwrap(),
            Some(2.5)
        );
        assert!(optional_finite_f64(&Value::Bool(true), "x").is_err());
        assert_eq!(percentile_of(&[1.0, 2.0, 2.0, 4.0], 2.0), 50.0);
        assert_eq!(min_max_median(&[]), (None, None, None));
        assert_eq!(
            min_max_median(&[3.0, 1.0, 2.0]),
            (Some(1.0), Some(3.0), Some(2.0))
        );
        assert_eq!(
            min_max_median(&[4.0, 1.0, 3.0, 2.0]),
            (Some(1.0), Some(4.0), Some(2.5))
        );
    }

    #[test]
    fn local_valuation_batch_computes_real_history_statistics() {
        let mut dates = Vec::new();
        let mut date = chrono::NaiveDate::from_ymd_opt(2026, 5, 4).unwrap();
        for _ in 0..30 {
            dates.push(date);
            date = crate::calendar::next_trading_day(date);
        }
        dates.reverse();
        let newest = dates.first().unwrap().to_string();
        let oldest = dates.last().unwrap().to_string();
        let rows: Vec<Value> = dates
            .iter()
            .enumerate()
            .map(|(index, date)| {
                serde_json::json!({
                    "TRADE_DATE": format!("{date} 00:00:00"),
                    "PE_TTM": if index == 0 { Value::String("10".into()) } else { serde_json::json!(10 + index) },
                    "PB_MRQ": 2.0 + index as f64 / 10.0
                })
            })
            .collect();
        let json = serde_json::json!({"result": {"data": rows}});

        let history = parse_valuation_history(&json).expect("complete local valuation batch");

        assert_eq!(history.sample_days, 30);
        assert_eq!(history.current_pe, Some(10.0));
        assert_eq!(history.current_pb, Some(2.0));
        assert_eq!(history.pe_percentile, Some(100.0 / 60.0));
        assert_eq!(history.pe_min, Some(10.0));
        assert_eq!(history.pe_max, Some(39.0));
        assert_eq!(history.pe_median, Some(24.5));
        assert_eq!(history.newest_date.as_deref(), Some(newest.as_str()));
        assert_eq!(history.oldest_date.as_deref(), Some(oldest.as_str()));
    }

    #[test]
    fn local_valuation_batch_rejects_missing_or_empty_protocol_arrays() {
        assert!(parse_valuation_history(&serde_json::json!({})).is_err());
        assert!(parse_valuation_history(&serde_json::json!({"result": {"data": []}})).is_err());
    }

    #[test]
    fn short_or_nonpositive_series_remains_explicitly_unranked() {
        let json = serde_json::json!({"result": {"data": [
            {"TRADE_DATE": "2026-07-17", "PE_TTM": -1, "PB_MRQ": 2},
            {"TRADE_DATE": "2026-07-16", "PE_TTM": null, "PB_MRQ": 2.1}
        ]}});
        let history = parse_valuation_history(&json).expect("negative values are unranked");
        assert_eq!(history.current_pe, None);
        assert_eq!(history.pe_percentile, None);
        assert_eq!(history.pb_percentile, None);
        assert_eq!(history.sample_days, 2);

        let malformed = serde_json::json!({"result": {"data": [
            {"TRADE_DATE": "2026-07-17", "PE_TTM": "bad", "PB_MRQ": 2}
        ]}});
        assert!(parse_valuation_history(&malformed).is_err());
    }

    #[test]
    fn blocking_wrapper_without_runtime_returns_missing() {
        assert!(fetch_blocking(&reqwest::Client::new(), "TEST_CODE_000001").is_none());
    }
}
