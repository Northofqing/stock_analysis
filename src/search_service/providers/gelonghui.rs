//! 格隆汇快讯 provider（P4）。
//!
//! 数据源：`https://www.gelonghui.com/live` 页面内嵌 SSR 数据。
//! 说明：页面脚本中包含 `title/createTimestamp/route` 字段，解析后转成 SearchResult。

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use regex::Regex;

use super::super::types::{SearchProvider, SearchResponse, SearchResult};

pub struct GelonghuiProvider {
    name: String,
    client: reqwest::Client,
}

impl GelonghuiProvider {
    pub fn new() -> Self {
        Self {
            name: "格隆汇".to_string(),
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

    pub async fn fetch_live(&self, limit: usize) -> Result<Vec<SearchResult>> {
        let html = self
            .client
            .get("https://www.gelonghui.com/live")
            .send()
            .await
            .context("格隆汇页面请求失败")?
            .text()
            .await
            .context("格隆汇页面读取失败")?;

        // 页面脚本对象中抓取 title + route 二元组。
        // createTimestamp 在 SSR 压缩后有时是变量占位（如 k/l/m），不稳定，故不作为必需字段。
        let re = Regex::new(r#"title:"([^"]+)"[\s\S]*?route:"([^"]+)""#)
            .context("格隆汇解析规则构建失败")?;

        let mut out = Vec::new();
        for cap in re
            .captures_iter(&html)
            .take(limit.saturating_mul(4).max(40))
        {
            let title = decode_js_escaped(cap.get(1).map(|m| m.as_str()).unwrap_or_default());
            let route_raw = cap.get(2).map(|m| m.as_str()).unwrap_or_default();
            let route = decode_js_escaped(route_raw);

            if title.trim().is_empty() || !route.contains("gelonghui.com/live/") {
                continue;
            }

            out.push(
                SearchResult::new(title, "格隆汇快讯".to_string(), route, "格隆汇".to_string())
                    .with_date(String::new()),
            );

            if out.len() >= limit {
                break;
            }
        }

        if out.is_empty() {
            return Err(anyhow::anyhow!("格隆汇解析结果为空"));
        }

        Ok(out)
    }
}

#[async_trait]
impl SearchProvider for GelonghuiProvider {
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
        let results = match self.fetch_live(max_results).await {
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

fn decode_js_escaped(input: &str) -> String {
    input
        .replace("\\u002F", "/")
        .replace("\\u003A", ":")
        .replace("\\u003D", "=")
        .replace("\\u0026", "&")
        .replace("\\u0027", "'")
        .replace("\\u0026quot;", "\"")
}
