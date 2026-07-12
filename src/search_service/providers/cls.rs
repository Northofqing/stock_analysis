//! 财联社（CLS）直连 provider。

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

use super::super::types::{SearchProvider, SearchResponse, SearchResult};
use super::cls_sign::build_signed_params;

#[derive(Debug, Deserialize)]
struct ClsTelegraphResp {
    errno: Option<i64>,
    data: Option<ClsTelegraphData>,
}

#[derive(Debug, Deserialize)]
struct ClsTelegraphData {
    l: Option<std::collections::HashMap<String, ClsTelegraphItem>>,
}

#[derive(Debug, Deserialize)]
struct ClsTelegraphItem {
    id: Option<i64>,
    title: Option<String>,
    content: Option<String>,
    ctime: Option<i64>,
}

pub struct ClsProvider {
    name: String,
    client: reqwest::Client,
}

impl ClsProvider {
    pub fn new() -> Self {
        Self {
            name: "财联社".to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent(
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
                )
                .build()
                .unwrap_or_default(),
        }
    }

    /// 拉取 CLS 最新电报（接口由前端页面请求链路逆向得到）。
    /// BR-037: 统一走 cls_sign 构造签名参数，禁止裸 query 请求。
    pub async fn fetch_live_news(&self, limit: usize) -> Result<Vec<SearchResult>> {
        let params = build_signed_params(&[("name", "telegraph".to_string())]);

        let resp: ClsTelegraphResp = self
            .client
            .get("https://www.cls.cn/api/cache")
            .query(&params)
            .header("Origin", "https://www.cls.cn")
            .header("Referer", "https://www.cls.cn/telegraph")
            .send()
            .await
            .context("财联社快讯请求失败")?
            .json()
            .await
            .context("财联社快讯解析失败")?;

        if resp.errno != Some(0) {
            return Err(anyhow::anyhow!("财联社API返回错误码: {:?}", resp.errno));
        }

        let mut items: Vec<ClsTelegraphItem> = resp
            .data
            .and_then(|d| d.l)
            .map(|m| m.into_values().collect())
            .unwrap_or_default();

        items.sort_by(|a, b| b.ctime.unwrap_or(0).cmp(&a.ctime.unwrap_or(0)));

        let now_ts = chrono::Local::now().timestamp();
        let mut out = Vec::new();
        for item in items.into_iter().take(limit.saturating_mul(3).max(30)) {
            let title = item
                .title
                .or_else(|| {
                    item.content
                        .as_ref()
                        .map(|c| c.chars().take(60).collect::<String>())
                })
                .unwrap_or_default();
            if title.trim().is_empty() {
                continue;
            }
            if let Some(ts) = item.ctime {
                // 仅保留最近 12 小时内容，避免老缓存混入。
                if now_ts - ts > 12 * 3600 {
                    continue;
                }
            }

            let snippet = item.content.unwrap_or_default();
            let date_tag = item.ctime.map(|ts| {
                chrono::DateTime::from_timestamp(ts, 0)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Local)
                    .format("%H:%M")
                    .to_string()
            });
            let detail_id = item.id.unwrap_or_default();
            let url = if detail_id > 0 {
                format!("https://www.cls.cn/detail/{}", detail_id)
            } else {
                "https://www.cls.cn/telegraph".to_string()
            };

            out.push(
                SearchResult::new(title, snippet, url, "财联社".to_string())
                    .with_date(date_tag.unwrap_or_default()),
            );
            if out.len() >= limit {
                break;
            }
        }

        Ok(out)
    }
}

#[async_trait]
impl SearchProvider for ClsProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        let start = Instant::now();
        let keywords: Vec<String> = query
            .split_whitespace()
            .filter(|w| w.chars().count() >= 2)
            .take(6)
            .map(|s| s.to_lowercase())
            .collect();

        let candidates = match self.fetch_live_news(max_results.max(20)).await {
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

        let results: Vec<SearchResult> = if keywords.is_empty() {
            candidates.into_iter().take(max_results).collect()
        } else {
            candidates
                .into_iter()
                .filter(|r| {
                    let text = format!("{} {}", r.title, r.snippet).to_lowercase();
                    keywords.iter().any(|kw| text.contains(kw))
                })
                .take(max_results)
                .collect()
        };

        let success = !results.is_empty();
        SearchResponse {
            query: query.to_string(),
            results,
            provider: self.name.clone(),
            success,
            error_message: if success {
                None
            } else {
                Some("无相关结果".to_string())
            },
            search_time: start.elapsed().as_secs_f64(),
        }
    }
}
