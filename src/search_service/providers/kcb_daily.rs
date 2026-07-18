//! 科创板日报 (STAR Market Daily) 直连 provider。
//!
//! 修复 B-002: 科创板日报是财联社旗下独立垂直媒体 (聚焦半导体/新能源/AI),
//! 之前 CLS provider 只抓 `refreshTenTelegraph` (主站电报), 科创板日报不在其列.
//!
//! 实现: 用 CLS 的缓存接口, 但用 `channel=kcb` 参数指定科创板日报频道.
//! 失败时显式报错, 不返回占位/假数据 (AGENTS.md §2.1 数据红线).

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use log::{info, warn};
use serde::Deserialize;

use super::super::types::{SearchProvider, SearchResponse, SearchResult};
use super::cls_sign::build_signed_params;

#[derive(Debug, Deserialize)]
struct ClsArticleResp {
    errno: Option<i64>,
    data: Option<ClsArticleData>,
}

#[derive(Debug, Deserialize)]
struct ClsArticleData {
    roll_data: Option<Vec<ClsArticleItem>>,
}

#[derive(Debug, Deserialize)]
struct ClsArticleItem {
    title: Option<String>,
    content: Option<String>,
    ctime: Option<i64>,
    id: Option<i64>,
}

pub struct KcbDailyProvider {
    name: String,
    client: reqwest::Client,
}

impl Default for KcbDailyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl KcbDailyProvider {
    pub fn new() -> Self {
        Self {
            name: "科创板日报".to_string(),
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

    /// 修复 B-002: 拉取科创板日报最新文章
    /// BR-037: 与 CLS 共用签名算法，但仅在快讯池抓取，不进入主题池。
    pub async fn fetch_latest(&self, limit: usize) -> Result<Vec<SearchResult>> {
        let params = build_signed_params(&[("category", "kcb".to_string())]);

        let http_resp = self
            .client
            .get("https://www.cls.cn/v1/roll/get_roll_list")
            .query(&params)
            .header("Origin", "https://www.cls.cn")
            .header("Referer", "https://www.cls.cn/kcb")
            .send()
            .await
            .context("科创板日报请求失败")?;

        let status = http_resp.status();
        let body = http_resp.text().await.context("科创板日报响应读取失败")?;

        if !status.is_success() {
            let preview = body.chars().take(200).collect::<String>();
            return Err(anyhow::anyhow!(
                "科创板日报HTTP失败 status={} body={}...",
                status,
                preview
            ));
        }

        let resp: ClsArticleResp = serde_json::from_str(&body).map_err(|e| {
            let preview = body.chars().take(200).collect::<String>();
            warn!("[科创板日报] JSON 解析失败: {} | body={}...", e, preview);
            anyhow::anyhow!("科创板日报解析失败: {}", e)
        })?;

        if resp.errno != Some(0) {
            return Err(anyhow::anyhow!("科创板日报API返回错误码: {:?}", resp.errno));
        }

        let items = resp.data.and_then(|d| d.roll_data).unwrap_or_default();

        let now = chrono::Local::now().timestamp();
        let mut results = Vec::new();
        for item in items.iter().take(limit) {
            let title = item.title.clone().unwrap_or_default();
            let content = item.content.clone().unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let age_secs = now - item.ctime.unwrap_or(0);
            let hours_ago = age_secs / 3600;

            results.push(SearchResult {
                title,
                snippet: content.chars().take(100).collect(),
                url: format!("https://www.cls.cn/detail/{}", item.id.unwrap_or(0)),
                source: "科创板日报".to_string(),
                published_date: item.ctime.map(|t| {
                    chrono::DateTime::from_timestamp(t, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_default()
                }),
                news_type: super::super::types::NewsType::Industry,
                sentiment: super::super::types::Sentiment::Neutral,
                importance: if hours_ago < 4 { 7 } else { 5 },
                relevance: 0.8,
                keywords: vec![],
            });
        }
        info!("[科创板日报] 拉取 {} 条", results.len());
        Ok(results)
    }
}

#[async_trait]
impl SearchProvider for KcbDailyProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        true
    }

    fn supports_topic_search(&self) -> bool {
        // CLS kcb 频道端点当前需签名 (sign/app/os), 裸请求失败; 且本 provider 忽略 query 只拉最新.
        // 暂排除出主题搜索, 待 RSSHub 签名方案修复后再启用.
        false
    }

    async fn search(&self, _query: &str, max_results: usize) -> SearchResponse {
        let start = Instant::now();
        match self.fetch_latest(max_results).await {
            Ok(results) => SearchResponse {
                success: true,
                results,
                provider: self.name.clone(),
                query: String::new(),
                search_time: start.elapsed().as_secs_f64(),
                error_message: None,
            },
            Err(e) => SearchResponse {
                success: false,
                results: vec![],
                provider: self.name.clone(),
                query: String::new(),
                search_time: start.elapsed().as_secs_f64(),
                error_message: Some(format!("科创板日报: {}", e)),
            },
        }
    }
}
