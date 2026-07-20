//! Registered business rules: BR-137.
//! 政府/监管公告 provider（P6）。
//!
//! 当前实现优先使用发改委通知公告 RSS（公开、稳定、结构化）：
//! - https://www.ndrc.gov.cn/xwdt/tzgg/rss.xml

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use regex::Regex;

use super::super::types::{SearchProvider, SearchResponse, SearchResult};

const NDRC_TZGG_RSS: &str = "https://www.ndrc.gov.cn/xwdt/tzgg/rss.xml";
const NDRC_TZGG_HTML: &str = "https://www.ndrc.gov.cn/xwdt/tzgg/";

pub struct GovPolicyProvider {
    name: String,
    client: reqwest::Client,
}

impl Default for GovPolicyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GovPolicyProvider {
    pub fn new() -> Self {
        Self {
            name: "政府监管".to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(12))
                .user_agent(
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
                )
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn fetch_latest(&self, limit: usize) -> Result<Vec<SearchResult>> {
        if let Ok(v) = self.fetch_from_ndrc_html(limit).await {
            if !v.is_empty() {
                return Ok(v);
            }
        }

        let xml = self
            .client
            .get(NDRC_TZGG_RSS)
            .send()
            .await
            .context("发改委 RSS 请求失败")?
            .text()
            .await
            .context("发改委 RSS 读取失败")?;

        let item_re = Regex::new(r#"(?s)<item>(.*?)</item>"#).context("item regex 构建失败")?;
        let title_re = Regex::new(r#"(?s)<title>(.*?)</title>"#).context("title regex 构建失败")?;
        let link_re = Regex::new(r#"(?s)<link>(.*?)</link>"#).context("link regex 构建失败")?;
        let date_re =
            Regex::new(r#"(?s)<pubDate>(.*?)</pubDate>"#).context("pubDate regex 构建失败")?;

        let mut out = Vec::new();
        for cap in item_re
            .captures_iter(&xml)
            .take(limit.saturating_mul(2).max(30))
        {
            let item = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
            let title = title_re
                .captures(item)
                .and_then(|m| m.get(1))
                .map(|m| strip_cdata(m.as_str()).trim().to_string())
                .unwrap_or_default();
            let link = link_re
                .captures(item)
                .and_then(|m| m.get(1))
                .map(|m| strip_cdata(m.as_str()).trim().to_string())
                .unwrap_or_default();
            let pub_date = date_re
                .captures(item)
                .and_then(|m| m.get(1))
                .map(|m| strip_cdata(m.as_str()).trim().to_string())
                .unwrap_or_default();

            if title.is_empty() || link.is_empty() {
                continue;
            }

            let mut result = SearchResult::new(
                title,
                "发改委通知公告".to_string(),
                link,
                "政府监管".to_string(),
            );
            if !pub_date.is_empty() {
                result.published_date = Some(pub_date);
            } else {
                log::warn!("[GovPolicyProvider][BR-137] RSS item missing pubDate");
            }
            out.push(result);

            if out.len() >= limit {
                break;
            }
        }

        if out.is_empty() {
            return Err(anyhow::anyhow!("政府监管 RSS 解析为空"));
        }

        Ok(out)
    }

    async fn fetch_from_ndrc_html(&self, limit: usize) -> Result<Vec<SearchResult>> {
        let html = self
            .client
            .get(NDRC_TZGG_HTML)
            .send()
            .await
            .context("发改委公告页请求失败")?
            .text()
            .await
            .context("发改委公告页读取失败")?;

        let re = Regex::new(r#"href="(\./[0-9]{6}/t[0-9]{8}_[0-9]+\.html)"[^>]*title="([^"]+)""#)
            .context("发改委公告页解析规则构建失败")?;

        let mut out = Vec::new();
        for cap in re
            .captures_iter(&html)
            .take(limit.saturating_mul(3).max(30))
        {
            let rel = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
            let title = cap
                .get(2)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }

            let link = format!(
                "https://www.ndrc.gov.cn/xwdt/tzgg/{}",
                rel.trim_start_matches("./")
            );

            let published_date = extract_date_from_ndrc_href(rel)
                .ok_or_else(|| anyhow::anyhow!("发改委公告链接缺少合法发布日期: {rel}"))?;
            out.push(
                SearchResult::new(
                    title,
                    "发改委通知公告".to_string(),
                    link,
                    "政府监管".to_string(),
                )
                .with_date(published_date),
            );
            if out.len() >= limit {
                break;
            }
        }

        if out.is_empty() {
            return Err(anyhow::anyhow!("发改委公告页解析为空"));
        }

        Ok(out)
    }
}

#[async_trait]
impl SearchProvider for GovPolicyProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        true
    }

    fn supports_topic_search(&self) -> bool {
        false
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        let start = Instant::now();
        let results = match self.fetch_latest(max_results).await {
            Ok(v) => v,
            Err(e) => {
                return SearchResponse {
                    query: query.to_string(),
                    results: Vec::new(),
                    provider: self.name.clone(),
                    success: false,
                    error_message: Some(e.to_string()),
                    search_time: start.elapsed().as_secs_f64(),
                };
            }
        };

        let keywords: Vec<String> = query
            .split_whitespace()
            .filter(|w| w.chars().count() >= 2)
            .map(|s| s.to_lowercase())
            .collect();

        let filtered = if keywords.is_empty() {
            results
        } else {
            results
                .into_iter()
                .filter(|r| {
                    let text = format!("{} {}", r.title, r.snippet).to_lowercase();
                    keywords.iter().any(|kw| text.contains(kw))
                })
                .take(max_results)
                .collect()
        };

        SearchResponse {
            query: query.to_string(),
            success: !filtered.is_empty(),
            error_message: None,
            results: filtered,
            provider: self.name.clone(),
            search_time: start.elapsed().as_secs_f64(),
        }
    }
}

fn strip_cdata(s: &str) -> &str {
    s.trim()
        .trim_start_matches("<![CDATA[")
        .trim_end_matches("]]>")
}

fn extract_date_from_ndrc_href(href: &str) -> Option<String> {
    // 例如: ./202607/t20260710_1406433.html
    let re = Regex::new(r#"t(\d{4})(\d{2})(\d{2})_"#).ok()?;
    let cap = re.captures(href)?;
    let y = cap.get(1)?.as_str();
    let m = cap.get(2)?.as_str();
    let d = cap.get(3)?.as_str();
    Some(format!("{}-{}-{}", y, m, d))
}
