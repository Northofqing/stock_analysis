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

fn optional_finite_f64(item: &Value, field: &str) -> Result<Option<f64>> {
    let Some(v) = item.get(field) else {
        return Ok(None);
    };
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_f64()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| anyhow!("一致预期字段 {field} 不是有限数字: {v}")),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| anyhow!("一致预期字段 {field} 非法: {s:?}")),
        _ => Err(anyhow!("一致预期字段 {field} 类型非法: {v}")),
    }
}

fn required_text(item: &Value, field: &str) -> Result<String> {
    item.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("一致预期记录缺少非空字段 {field}"))
}

fn required_publish_date(item: &Value) -> Result<(String, chrono::NaiveDate)> {
    let raw = required_text(item, "publishDate")?;
    let date_text = raw.split_whitespace().next().unwrap_or(&raw);
    let date = chrono::NaiveDate::parse_from_str(date_text, "%Y-%m-%d")
        .map_err(|error| anyhow!("一致预期 publishDate 非法 {date_text:?}: {error}"))?;
    Ok((date_text.to_string(), date))
}

fn avg(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

/// BR-119: validate and aggregate one complete consensus protocol batch.
fn parse_consensus_data(
    json: &Value,
    begin: chrono::NaiveDate,
    today: chrono::NaiveDate,
) -> Result<ConsensusData> {
    let arr = json
        .get("data")
        .and_then(Value::as_array)
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
    let mut previous_publish_date: Option<chrono::NaiveDate> = None;

    for (index, item) in arr.iter().enumerate() {
        let (publish_date, parsed_publish_date) = required_publish_date(item)?;
        if parsed_publish_date < begin || parsed_publish_date > today {
            return Err(anyhow!(
                "一致预期 publishDate 超出请求窗口: {parsed_publish_date} not in {begin}..={today}"
            ));
        }
        if let Some(previous) = previous_publish_date {
            if parsed_publish_date > previous {
                return Err(anyhow!(
                    "一致预期记录时间顺序错误: {parsed_publish_date} 晚于前一条 {previous}"
                ));
            }
        }
        previous_publish_date = Some(parsed_publish_date);

        if let Some(value) = optional_finite_f64(item, "predictThisYearEps")? {
            eps_this.push(value);
        }
        if let Some(value) = optional_finite_f64(item, "predictNextYearEps")? {
            eps_next.push(value);
        }
        if let Some(value) = optional_finite_f64(item, "predictNextTwoYearEps")? {
            eps_next2.push(value);
        }
        let high = optional_finite_f64(item, "indvAimPriceT")?;
        let low = optional_finite_f64(item, "indvAimPriceL")?;
        for (field, value) in [("indvAimPriceT", high), ("indvAimPriceL", low)] {
            if value.is_some_and(|price| price <= 0.0) {
                return Err(anyhow!("一致预期字段 {field} 必须 > 0"));
            }
        }
        if high.zip(low).is_some_and(|(high, low)| low > high) {
            return Err(anyhow!(
                "一致预期目标价上下限颠倒: low={low:?} high={high:?}"
            ));
        }
        if let Some(value) = high {
            tp_high.push(value);
        }
        if let Some(value) = low {
            tp_low.push(value);
        }

        let rating = required_text(item, "emRatingName")?;
        *rating_dist.entry(rating.clone()).or_insert(0) += 1;
        let org_name = required_text(item, "orgSName")?;
        brokers.insert(org_name.clone());
        let title = required_text(item, "title")?;
        if index == 0 {
            latest_report_date = Some(publish_date.clone());
        }
        if recent_reports.len() < 3 {
            recent_reports.push(RecentReport {
                title,
                org_name,
                publish_date,
                rating,
            });
        }
    }

    if eps_this.is_empty() && eps_next.is_empty() && eps_next2.is_empty() {
        return Err(anyhow!("一致预期批次不含任何 EPS 预测"));
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

    parse_consensus_data(&json, begin, today)
}

/// 同步包装：在已有 tokio runtime 上下文内调用
pub fn fetch_blocking(client: &reqwest::Client, code: &str) -> Result<ConsensusData> {
    // 修复 Top10#5 (2026-06-29 audit): 用 crate::block_on_async 统一替代
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(anyhow!("[一致预期] 无 tokio runtime，无法抓取 {code}"));
    }
    let client = client.clone();
    let code_s = code.to_string();
    crate::block_on_async(async move { fetch_async(&client, &code_s).await })
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

#[cfg(test)]
mod strict_contract_tests {
    use super::*;

    #[test]
    fn malformed_present_consensus_number_is_an_error() {
        let item = serde_json::json!({"predictThisYearEps": "not-a-number"});
        assert!(optional_finite_f64(&item, "predictThisYearEps").is_err());
        let missing = serde_json::json!({"predictThisYearEps": ""});
        assert_eq!(
            optional_finite_f64(&missing, "predictThisYearEps").unwrap(),
            None
        );
        assert_eq!(
            optional_finite_f64(&serde_json::json!({}), "missing").unwrap(),
            None
        );
        assert!(optional_finite_f64(&serde_json::json!({"x": true}), "x").is_err());
        assert_eq!(avg(&[]), None);
    }

    #[test]
    fn consensus_report_requires_valid_date_and_nonempty_identity_fields() {
        let valid = serde_json::json!({
            "publishDate": "2026-07-18 00:00:00",
            "title": "测试研报",
            "orgSName": "测试券商",
            "emRatingName": "买入"
        });
        assert_eq!(required_publish_date(&valid).unwrap().0, "2026-07-18");
        assert!(required_text(&valid, "title").is_ok());

        let bad_date = serde_json::json!({"publishDate": "2026-99-99"});
        assert!(required_publish_date(&bad_date).is_err());
        assert!(required_text(&serde_json::json!({"title": ""}), "title").is_err());
    }

    fn report(date: &str, broker: &str, rating: &str, eps: Value) -> Value {
        serde_json::json!({
            "publishDate": date,
            "title": format!("{broker}研报"),
            "orgSName": broker,
            "emRatingName": rating,
            "predictThisYearEps": eps,
            "predictNextYearEps": 2.0,
            "predictNextTwoYearEps": 3.0,
            "indvAimPriceT": 15.0,
            "indvAimPriceL": 12.0
        })
    }

    #[test]
    fn local_consensus_batch_aggregates_only_complete_real_reports() {
        let begin = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        let json = serde_json::json!({"data": [
            report("2026-07-18", "甲券商", "买入", serde_json::json!(1.0)),
            report("2026-07-17", "乙券商", "增持", serde_json::json!(1.5)),
            report("2026-07-16", "甲券商", "中性", serde_json::json!(2.0)),
            report("2026-07-15", "丙券商", "推荐", serde_json::json!(2.5))
        ]});

        let data = parse_consensus_data(&json, begin, today).expect("valid consensus batch");

        assert_eq!(data.report_count, 4);
        assert_eq!(data.broker_count, 3);
        assert_eq!(data.eps_this_year_avg, Some(1.75));
        assert_eq!(data.target_price_high_avg, Some(15.0));
        assert_eq!(data.latest_report_date.as_deref(), Some("2026-07-18"));
        assert_eq!(data.recent_reports.len(), 3);
        assert_eq!(data.bullish_ratio(), Some(75.0));
        assert_eq!(data.upside_pct(10.0), Some(50.0));
        assert_eq!(data.upside_pct(0.0), None);
    }

    #[test]
    fn local_consensus_batch_rejects_protocol_and_time_failures() {
        let begin = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        assert!(parse_consensus_data(&serde_json::json!({}), begin, today).is_err());
        assert!(parse_consensus_data(&serde_json::json!({"data": []}), begin, today).is_err());

        let outside = serde_json::json!({"data": [report(
            "2025-12-31", "甲券商", "买入", serde_json::json!(1.0)
        )]});
        assert!(parse_consensus_data(&outside, begin, today).is_err());

        let out_of_order = serde_json::json!({"data": [
            report("2026-07-17", "甲券商", "买入", serde_json::json!(1.0)),
            report("2026-07-18", "乙券商", "买入", serde_json::json!(1.0))
        ]});
        assert!(parse_consensus_data(&out_of_order, begin, today).is_err());
    }

    #[test]
    fn local_consensus_batch_rejects_bad_prices_and_missing_eps_evidence() {
        let begin = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();

        let mut nonpositive = report("2026-07-18", "甲券商", "买入", serde_json::json!(1.0));
        nonpositive["indvAimPriceT"] = serde_json::json!(0.0);
        assert!(
            parse_consensus_data(&serde_json::json!({"data": [nonpositive]}), begin, today)
                .is_err()
        );

        let mut reversed = report("2026-07-18", "甲券商", "买入", serde_json::json!(1.0));
        reversed["indvAimPriceT"] = serde_json::json!(10.0);
        reversed["indvAimPriceL"] = serde_json::json!(12.0);
        assert!(
            parse_consensus_data(&serde_json::json!({"data": [reversed]}), begin, today).is_err()
        );

        let no_eps = report("2026-07-18", "甲券商", "买入", Value::Null);
        let mut no_eps = no_eps;
        no_eps["predictNextYearEps"] = Value::Null;
        no_eps["predictNextTwoYearEps"] = Value::Null;
        assert!(
            parse_consensus_data(&serde_json::json!({"data": [no_eps]}), begin, today).is_err()
        );
    }

    #[test]
    fn empty_ratings_have_no_bullish_ratio_and_blocking_requires_runtime() {
        assert_eq!(ConsensusData::default().bullish_ratio(), None);
        assert!(fetch_blocking(&reqwest::Client::new(), "TEST_CODE_000001").is_err());
    }

    #[tokio::test]
    async fn real_consensus_transport_failure_is_not_an_empty_consensus() {
        let client = super::super::unreachable_http_client();
        assert!(fetch_async(&client, "TEST_CODE_000001").await.is_err());
    }
}
