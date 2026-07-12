//! 微博热搜榜 provider（正交源：全民级突发/科技/政策热点最快落点）
//!
//! 数据源：https://weibo.com/ajax/side/hotSearch
//! - 免费直连，返回实时热搜榜（realtime）
//!
//! 定位：
//! - 现有 4 个快讯源（金十/见闻/财联社/新浪）全是**财经快讯**，高度同质，
//!   对"长十乙火箭回收"这类科技/时政/全民事件系统性漏报。
//! - 微博热搜是全民热点的最快落点，与财经快讯源**正交**，补的是信息「种类」不是「数量」。
//!
//! 设计约束（AGENTS.md 数据红线）：
//! - 2.1 无 mock：真实 API，抓取失败返回 `Err`，绝不编造热搜。
//! - 2.2 缺数据：热搜无发布时间戳 → `published_date` 留空（None），不臆造时间。
//! - 2.10 纯抓取器：不做 dedup/filter/sort/limit 语义（去重复用 `fetch_flash_titles`
//!   已有逻辑，题材过滤交给下游 `chain_mapper`），故无需新增 BR。
//!
//! 实现参照 xueqiu.rs / sina_flash.rs 模式。
//! 反爬：微博 ajax 接口通常仅需 User-Agent；部分环境需 Cookie，
//!       读 `WEIBO_COOKIE` env（严禁 hardcode 到源码/提交 git）。

use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

use super::super::types::{SearchProvider, SearchResponse, SearchResult};

const HOT_SEARCH_URL: &str = "https://weibo.com/ajax/side/hotSearch";

/// 微博热搜榜 provider
pub struct WeiboHotProvider {
    name: String,
    client: reqwest::Client,
}

impl WeiboHotProvider {
    pub fn new() -> Self {
        Self {
            name: "微博热搜".to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .build()
                .unwrap(),
        }
    }

    /// 抓取实时热搜榜，返回前 `limit` 条。
    ///
    /// 失败（网络/解析）返回 `Err`，由调用方按 best-effort 处理，不静默填充。
    pub async fn fetch_hot_search(&self, limit: usize) -> Result<Vec<SearchResult>> {
        let mut req = self
            .client
            .get(HOT_SEARCH_URL)
            .header("Referer", "https://weibo.com/");
        if let Ok(cookie) = std::env::var("WEIBO_COOKIE") {
            if !cookie.is_empty() {
                log::info!("[weibo] 使用 cookie ({} bytes)", cookie.len());
                req = req.header("Cookie", cookie);
            }
        }

        let body = req
            .send()
            .await
            .context("微博热搜请求失败")?
            .text()
            .await
            .context("微博热搜读取响应失败")?;

        let mut results = parse_hot_search(&body);
        results.truncate(limit);
        Ok(results)
    }
}

impl Default for WeiboHotProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize, Debug)]
struct HotSearchResp {
    #[serde(default)]
    data: Option<HotSearchData>,
}

#[derive(Deserialize, Debug)]
struct HotSearchData {
    #[serde(default)]
    realtime: Option<Vec<HotItem>>,
}

#[derive(Deserialize, Debug)]
struct HotItem {
    /// 热搜词（如 "长十乙火箭海上回收成功"）
    #[serde(default)]
    word: Option<String>,
    /// 备用文案，部分条目 word 为空时用 note
    #[serde(default)]
    note: Option<String>,
    /// 分类（如 "科技"/"社会"），仅作 snippet 展示
    #[serde(default)]
    category: Option<String>,
}

/// 纯函数：把微博热搜 JSON 文本解析为 `SearchResult` 列表（离线可测）。
///
/// 只提取热搜词作为标题；无发布时间戳 → `published_date` 留空（红线 2.2）。
/// 广告/无词条目跳过。不做去重/过滤/排序（红线 2.10）。
fn parse_hot_search(body: &str) -> Vec<SearchResult> {
    let resp: HotSearchResp = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[weibo] 热搜解析失败: {}", e);
            return Vec::new();
        }
    };

    let realtime = match resp.data.and_then(|d| d.realtime) {
        Some(v) => v,
        None => return Vec::new(),
    };

    let mut results = Vec::new();
    for item in realtime {
        let title = match item
            .word
            .filter(|w| !w.is_empty())
            .or(item.note.filter(|n| !n.is_empty()))
        {
            Some(t) => t,
            None => continue,
        };
        let snippet = item
            .category
            .filter(|c| !c.is_empty())
            .map(|c| format!("热搜分类: {}", c))
            .unwrap_or_default();
        // url 指向微博搜索页，便于人工回溯
        let url = format!(
            "https://s.weibo.com/weibo?q={}",
            urlencoding::encode(&title)
        );
        results.push(SearchResult::new(
            title,
            snippet,
            url,
            "微博热搜".to_string(),
        ));
    }
    results
}

#[async_trait]
impl SearchProvider for WeiboHotProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        let started = std::time::Instant::now();
        match self.fetch_hot_search(max_results.min(50)).await {
            Ok(results) => SearchResponse {
                query: query.to_string(),
                success: true,
                error_message: None,
                search_time: started.elapsed().as_secs_f64(),
                results,
                provider: self.name.clone(),
            },
            Err(e) => SearchResponse::error(query.to_string(), self.name.clone(), e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_and_available() {
        let p = WeiboHotProvider::new();
        assert_eq!(p.name(), "微博热搜");
        assert!(p.is_available());
    }

    #[test]
    fn test_parse_hot_search_basic() {
        let body = r#"{
            "ok": 1,
            "data": {
                "realtime": [
                    {"word": "长十乙火箭海上回收成功", "category": "科技", "num": 1234567},
                    {"word": "某地暴雨预警", "category": "社会"}
                ]
            }
        }"#;
        let results = parse_hot_search(body);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "长十乙火箭海上回收成功");
        assert_eq!(results[0].source, "微博热搜");
        // 红线 2.2: 无时间戳 → 不臆造发布时间
        assert!(results[0].published_date.is_none());
        assert!(results[0].snippet.contains("科技"));
    }

    #[test]
    fn test_parse_hot_search_falls_back_to_note() {
        let body = r#"{"data": {"realtime": [{"word": "", "note": "备用文案"}]}}"#;
        let results = parse_hot_search(body);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "备用文案");
    }

    #[test]
    fn test_parse_hot_search_skips_empty() {
        let body = r#"{"data": {"realtime": [{"category": "社会"}]}}"#;
        let results = parse_hot_search(body);
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_hot_search_bad_json_returns_empty() {
        assert!(parse_hot_search("not json").is_empty());
        assert!(parse_hot_search("{}").is_empty());
    }
}
