use async_trait::async_trait;
use chrono::{Duration, Local};
use futures::future::join_all;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use crate::agent::tool::Tool;

/// 详情页最多并发拉取条数（接口比较慢，控制一下）
const DETAIL_FETCH_TOP_N: usize = 5;
/// 摘要正文截断长度（字符）
const SUMMARY_MAX_CHARS: usize = 600;

pub struct FetchResearchTool {
    client: reqwest::Client,
}

impl FetchResearchTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

/// 东财研报列表接口返回的单条记录（仅取需要的字段）
#[derive(Debug, Deserialize)]
struct EmReportItem {
    #[serde(default)]
    title: String,
    #[serde(default, rename = "orgSName")]
    org_short_name: String,
    #[serde(default, rename = "orgName")]
    org_name: String,
    #[serde(default, rename = "emRatingName")]
    em_rating_name: String,
    #[serde(default, rename = "ratingChange")]
    rating_change: String,
    #[serde(default, rename = "publishDate")]
    publish_date: String,
    #[serde(default)]
    researcher: String,
    #[serde(default, rename = "indvInduName")]
    indv_indu_name: String,
    #[serde(default, rename = "infoCode")]
    info_code: String,
}

#[derive(Debug, Deserialize)]
struct EmReportResponse {
    #[serde(default)]
    data: Vec<EmReportItem>,
}

#[async_trait]
impl Tool for FetchResearchTool {
    fn name(&self) -> &str {
        "fetch_research"
    }

    fn description(&self) -> &str {
        "获取指定 A 股的最新机构研报摘要和评级（如买入、增持），用于辅助基本面未来基本面的预期判断。"
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "股票代码，如 '600519' 或 '000001'"
                }
            },
            "required": ["code"]
        })
    }

    async fn call(&self, input: serde_json::Value) -> anyhow::Result<String> {
        let code = input.get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' parameter"))?;

        // 默认取近 365 天（1年）的研报，最多 20 条
        let end = Local::now().date_naive();
        let begin = end - Duration::days(365);
        let url = format!(
            "https://reportapi.eastmoney.com/report/list\
             ?industryCode=*&pageSize=20&industry=*&rating=*&ratingChange=*\
             &beginTime={}&endTime={}&pageNo=1&fields=&qType=0&orgCode=\
             &code={}&rcode=&_={}",
            begin,
            end,
            code,
            end.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis()
        );
        log::debug!("[研报] {}", url);

        let resp = self
            .client
            .get(&url)
            .header("Referer", "https://data.eastmoney.com/")
            .send()
            .await;

        let body: EmReportResponse = match resp {
            Ok(r) => match r.json().await {
                Ok(v) => v,
                Err(e) => return Ok(json!({"error": format!("研报接口 JSON 解析失败: {}", e)}).to_string()),
            },
            Err(e) => return Ok(json!({"error": format!("研报接口请求失败: {}", e)}).to_string()),
        };

        if body.data.is_empty() {
            return Ok(json!({"error": "近 180 天未查询到机构研报", "code": code}).to_string());
        }

        // 评级分布统计
        let mut rating_counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for r in &body.data {
            if !r.em_rating_name.is_empty() {
                *rating_counts.entry(r.em_rating_name.clone()).or_insert(0) += 1;
            }
        }

        // 并发拉取前 N 条研报详情（摘要正文）
        let detail_targets: Vec<&EmReportItem> = body
            .data
            .iter()
            .filter(|r| !r.info_code.is_empty())
            .take(DETAIL_FETCH_TOP_N)
            .collect();
        let detail_futs = detail_targets
            .iter()
            .map(|r| fetch_report_summary(&self.client, &r.info_code));
        let summaries: Vec<Option<String>> = join_all(detail_futs).await;
        let mut summary_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (r, s) in detail_targets.iter().zip(summaries.into_iter()) {
            if let Some(text) = s {
                summary_map.insert(r.info_code.clone(), text);
            }
        }

        let reports: Vec<Value> = body
            .data
            .iter()
            .map(|r| {
                let date = r.publish_date.split(' ').next().unwrap_or(&r.publish_date).to_string();
                let institution = if !r.org_short_name.is_empty() {
                    r.org_short_name.clone()
                } else {
                    r.org_name.clone()
                };
                let summary = summary_map.get(&r.info_code).cloned().unwrap_or_default();
                json!({
                    "title": r.title,
                    "institution": institution,
                    "rating": r.em_rating_name,
                    "rating_change": r.rating_change,
                    "date": date,
                    "researcher": r.researcher,
                    "industry": r.indv_indu_name,
                    "info_code": r.info_code,
                    "summary": summary,
                })
            })
            .collect();

        let result = json!({
            "fetched": true,
            "code": code,
            "report_count": reports.len(),
            "summary_fetched": summary_map.len(),
            "rating_distribution": rating_counts,
            "reports": reports,
            "note": "数据源：东方财富研报中心 (reportapi.eastmoney.com/report/list, qType=0 个股研报)；近 180 天，最多 20 条；前 5 条已并发拉取详情页摘要 (report/detail)，正文已剥离 HTML 并截断。"
        });

        Ok(result.to_string())
    }
}

/// 拉取单篇研报详情页并提取纯文本摘要。
/// 失败时返回 None（让上层降级为空字符串，不影响其它字段）。
async fn fetch_report_summary(client: &reqwest::Client, info_code: &str) -> Option<String> {
    let url = format!(
        "https://reportapi.eastmoney.com/report/detail?infoCode={}",
        info_code
    );
    let resp = client
        .get(&url)
        .header("Referer", "https://data.eastmoney.com/")
        .send()
        .await
        .ok()?;
    let body: Value = resp.json().await.ok()?;

    // 兼容两种返回结构：data 为对象，或 data 为数组
    let content_html = body
        .pointer("/data/content")
        .and_then(|v| v.as_str())
        .or_else(|| {
            body.pointer("/data/0/content").and_then(|v| v.as_str())
        })
        .unwrap_or("");
    if content_html.is_empty() {
        return None;
    }

    Some(html_to_plain_text(content_html, SUMMARY_MAX_CHARS))
}

/// 简单地将 HTML 转为压缩后的纯文本，并按字符数截断。
fn html_to_plain_text(html: &str, max_chars: usize) -> String {
    // 1) 去掉 <script>/<style> 整段
    let re_block = Regex::new(r"(?is)<(script|style)[^>]*>.*?</\1>").unwrap();
    let stripped = re_block.replace_all(html, "");
    // 2) 去掉所有标签
    let re_tag = Regex::new(r"<[^>]+>").unwrap();
    let no_tag = re_tag.replace_all(&stripped, " ");
    // 3) 解码常见 HTML 实体
    let decoded = no_tag
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    // 4) 压缩空白
    let re_ws = Regex::new(r"\s+").unwrap();
    let compact = re_ws.replace_all(decoded.trim(), " ").to_string();

    if compact.chars().count() <= max_chars {
        compact
    } else {
        let truncated: String = compact.chars().take(max_chars).collect();
        format!("{}…", truncated)
    }
}
