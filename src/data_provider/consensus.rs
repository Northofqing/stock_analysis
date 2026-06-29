//! 卖方研报一致预期（EPS / 评级分布 / 目标价）
//!
//! 数据源：东方财富 `reportapi.eastmoney.com/report/list`，按个股聚合近 6 个月研报：
//! - 当年/下一年/再下一年的 EPS 预测均值
//! - 评级分布（买入 / 增持 / 中性 / 减持 / 卖出）
//! - 目标价区间（若研报披露）
//!
//! URL 示例：
//! https://reportapi.eastmoney.com/report/list?pageSize=50&beginTime=2025-11-01&
//! endTime=2026-05-23&pageNo=1&qType=0&code=600519

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::collections::HashMap;

/// 卖方一致预期汇总
#[derive(Debug, Clone, Default)]
pub struct ConsensusData {
    /// 覆盖研报数量（去重前）
    pub report_count: usize,
    /// 覆盖券商数量（去重后）
    pub broker_count: usize,
    /// 当年 EPS 预测均值
    pub eps_this_year_avg: Option<f64>,
    /// 下一年 EPS 预测均值
    pub eps_next_year_avg: Option<f64>,
    /// 再下一年 EPS 预测均值
    pub eps_next2_year_avg: Option<f64>,
    /// 评级分布（中文标签 -> 计数），如 "买入" / "增持" / "中性" / "减持" / "卖出"
    pub rating_distribution: HashMap<String, u32>,
    /// 目标价上限均值（仅统计有披露的研报）
    pub target_price_high_avg: Option<f64>,
    /// 目标价下限均值
    pub target_price_low_avg: Option<f64>,
    /// 最近一份研报日期（YYYY-MM-DD）
    pub latest_report_date: Option<String>,
    /// 最近 3 份研报摘要（标题、机构、日期、评级）
    pub recent_reports: Vec<RecentReport>,
}

#[derive(Debug, Clone)]
pub struct RecentReport {
    pub title: String,
    pub org_name: String,
    pub publish_date: String,
    pub rating: String,
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                s.parse::<f64>().ok()
            }
        }
        _ => None,
    }
}

fn avg(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

/// 异步拉取一致预期
pub async fn fetch_async(client: &reqwest::Client, code: &str) -> Result<ConsensusData> {
    // 取最近 180 天研报
    let today = chrono::Local::now().date_naive();
    let begin = today - chrono::Duration::days(180);
    let url = format!(
        "https://reportapi.eastmoney.com/report/list?\
         pageSize=50&pageNo=1&qType=0&\
         beginTime={}&endTime={}&code={}",
        begin.format("%Y-%m-%d"),
        today.format("%Y-%m-%d"),
        code
    );
    log::debug!("[一致预期] {}", url);

    let resp = client
        .get(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Referer", "https://data.eastmoney.com/")
        .send()
        .await
        .context("一致预期请求失败")?;
    if !resp.status().is_success() {
        return Err(anyhow!("一致预期状态码 {}", resp.status()));
    }
    let text = resp.text().await.context("一致预期读取响应失败")?;
    let json: Value = serde_json::from_str(&text).context("一致预期 JSON 解析失败")?;

    let arr = json
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("一致预期无 data 数组"))?;
    if arr.is_empty() {
        return Err(anyhow!("一致预期 data 为空"));
    }

    let mut eps_this: Vec<f64> = Vec::new();
    let mut eps_next: Vec<f64> = Vec::new();
    let mut eps_next2: Vec<f64> = Vec::new();
    let mut tp_high: Vec<f64> = Vec::new();
    let mut tp_low: Vec<f64> = Vec::new();
    let mut rating_dist: HashMap<String, u32> = HashMap::new();
    let mut brokers: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut latest_report_date: Option<String> = None;
    let mut recent_reports: Vec<RecentReport> = Vec::new();

    for (i, item) in arr.iter().enumerate() {
        if let Some(v) = item.get("predictThisYearEps").and_then(as_f64) {
            if v.is_finite() && v.abs() > 1e-9 {
                eps_this.push(v);
            }
        }
        if let Some(v) = item.get("predictNextYearEps").and_then(as_f64) {
            if v.is_finite() && v.abs() > 1e-9 {
                eps_next.push(v);
            }
        }
        if let Some(v) = item.get("predictNextTwoYearEps").and_then(as_f64) {
            if v.is_finite() && v.abs() > 1e-9 {
                eps_next2.push(v);
            }
        }
        if let Some(v) = item.get("indvAimPriceT").and_then(as_f64) {
            if v.is_finite() && v > 0.0 {
                tp_high.push(v);
            }
        }
        if let Some(v) = item.get("indvAimPriceL").and_then(as_f64) {
            if v.is_finite() && v > 0.0 {
                tp_low.push(v);
            }
        }
        if let Some(r) = item.get("emRatingName").and_then(|v| v.as_str()) {
            let r = r.trim();
            if !r.is_empty() {
                *rating_dist.entry(r.to_string()).or_insert(0) += 1;
            }
        }
        if let Some(org) = item.get("orgSName").and_then(|v| v.as_str()) {
            let org = org.trim();
            if !org.is_empty() {
                brokers.insert(org.to_string());
            }
        }
        let pub_date = item
            .get("publishDate")
            .and_then(|v| v.as_str())
            .map(|s| s.split_whitespace().next().unwrap_or(s).to_string());
        if i == 0 {
            latest_report_date = pub_date.clone();
        }
        if recent_reports.len() < 3 {
            recent_reports.push(RecentReport {
                title: item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                org_name: item
                    .get("orgSName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                publish_date: pub_date.unwrap_or_default(),
                rating: item
                    .get("emRatingName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }

    Ok(ConsensusData {
        report_count: arr.len(),
        broker_count: brokers.len(),
        eps_this_year_avg: avg(&eps_this),
        eps_next_year_avg: avg(&eps_next),
        eps_next2_year_avg: avg(&eps_next2),
        rating_distribution: rating_dist,
        target_price_high_avg: avg(&tp_high),
        target_price_low_avg: avg(&tp_low),
        latest_report_date,
        recent_reports,
    })
}

/// 同步包装：在已有 tokio runtime 上下文内调用
pub fn fetch_blocking(client: &reqwest::Client, code: &str) -> Option<ConsensusData> {
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
                log::warn!("[一致预期] {} 拉取失败: {}", code_s, e);
                None
            }
        }
    })
}

impl ConsensusData {
    /// 计算"买入+增持"占比（看多比例）
    pub fn bullish_ratio(&self) -> Option<f64> {
        let total: u32 = self.rating_distribution.values().sum();
        if total == 0 {
            return None;
        }
        let bull: u32 = self
            .rating_distribution
            .iter()
            .filter(|(k, _)| k.contains("买入") || k.contains("增持") || k.contains("推荐"))
            .map(|(_, v)| *v)
            .sum();
        Some(bull as f64 / total as f64 * 100.0)
    }

    /// 基于目标价均值与当前价的相对空间（%）
    pub fn upside_pct(&self, current_price: f64) -> Option<f64> {
        if current_price <= 0.0 {
            return None;
        }
        let high = self.target_price_high_avg?;
        Some((high - current_price) / current_price * 100.0)
    }
}
