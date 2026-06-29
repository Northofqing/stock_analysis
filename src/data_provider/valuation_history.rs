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

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn percentile_of(sorted: &[f64], current: f64) -> f64 {
    // 当前值在升序序列中的排名 / 总数 * 100；相同则取中点
    let n = sorted.len() as f64;
    let less = sorted.iter().filter(|v| **v < current).count() as f64;
    let equal = sorted.iter().filter(|v| (**v - current).abs() < 1e-9).count() as f64;
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
    let median = if sorted.len() % 2 == 0 {
        Some((sorted[mid - 1] + sorted[mid]) / 2.0)
    } else {
        Some(sorted[mid])
    };
    (min, max, median)
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

    let arr = json
        .get("result")
        .and_then(|r| r.get("data"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("估值历史无 result.data"))?;
    if arr.is_empty() {
        return Err(anyhow!("估值历史 data 为空"));
    }

    let mut pes: Vec<f64> = Vec::with_capacity(arr.len());
    let mut pbs: Vec<f64> = Vec::with_capacity(arr.len());
    let mut newest_date: Option<String> = None;
    let mut oldest_date: Option<String> = None;

    // arr 按 TRADE_DATE 降序，索引 0 = 最新
    for (i, item) in arr.iter().enumerate() {
        if let Some(pe) = item.get("PE_TTM").and_then(as_f64) {
            if pe.is_finite() && pe > 0.0 {
                pes.push(pe);
            }
        }
        if let Some(pb) = item.get("PB_MRQ").and_then(as_f64) {
            if pb.is_finite() && pb > 0.0 {
                pbs.push(pb);
            }
        }
        let date = item
            .get("TRADE_DATE")
            .and_then(|v| v.as_str())
            .map(|s| s.split_whitespace().next().unwrap_or(s).to_string());
        if i == 0 {
            newest_date = date.clone();
        }
        oldest_date = date;
    }

    let current_pe = pes.first().copied();
    let current_pb = pbs.first().copied();

    let mut sorted_pe = pes.clone();
    sorted_pe.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut sorted_pb = pbs.clone();
    sorted_pb.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let pe_percentile = current_pe
        .filter(|_| sorted_pe.len() >= 30)
        .map(|c| percentile_of(&sorted_pe, c));
    let pb_percentile = current_pb
        .filter(|_| sorted_pb.len() >= 30)
        .map(|c| percentile_of(&sorted_pb, c));

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
        oldest_date,
        newest_date,
    })
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
