//! v21: 雪球 (xueqiu.com) 实时新闻数据源
//!
//! BR-036: 无 XUEQIU_COOKIE 时降级为不可用，避免主题池噪声与伪失败。
//! 数据源: https://xueqiu.com/
//! - 公共时间线 API: /v4/statuses/public_timeline_by_category.json
//! - category=6 A股 / category=12 港股 / category=0 全部
//!
//! 特点:
//! - 完全免费, 无需 API Key
//! - 雪球用户/机构观点聚合, A股市场情绪丰富
//! - 与 sina/cls/jin10 互为冗余, 雪球偏情绪+个股讨论
//!
//! 实现参照 sina_flash.rs / wallstreetcn.rs 模式
//!
//! 风险: 雪球 API 可能反爬 (限流 429), 自动降级
//! 备选: 若 429 错误率高, 切换到 mika.sina.cn 或新浪华语

use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use log::warn;
use serde::Deserialize;

use super::super::types::{SearchProvider, SearchResponse, SearchResult};

/// 雪球 (xueqiu) 公共时间线 provider
pub struct XueqiuProvider {
    name: String,
    client: reqwest::Client,
    /// category: 6=A股 / 12=港股 / 0=全部
    categories: Vec<u32>,
    /// 失败退避时间 (避免反复触发 429)
    cooldown_until: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
}

impl XueqiuProvider {
    pub fn new() -> Self {
        Self::with_categories(vec![6, 0]) // 默认 A股 + 全部
    }

    /// 自定义 categories (测试或单类目部署用)
    pub fn with_categories(categories: Vec<u32>) -> Self {
        Self {
            name: "雪球".to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .build()
                .unwrap(),
            categories,
            cooldown_until: std::sync::Mutex::new(None),
        }
    }

    /// 拉取单 category 的快讯
    async fn fetch_category(&self, category: u32, count: usize) -> Result<Vec<SearchResult>> {
        // v25.2: 雪球 API 参数是 count (不是 num) - 修正
        let url = format!(
            "https://xueqiu.com/v4/statuses/public_timeline_by_category.json?category={}&count={}&page=1",
            category, count
        );

        #[derive(Deserialize, Debug)]
        struct StatusItem {
            #[serde(rename = "text")]
            text: Option<String>, // 雪球 API 实际用 text 字段 (不是 title)
            #[serde(rename = "target")]
            target: Option<String>,
            #[serde(rename = "created_at")]
            created_at: Option<i64>,
        }

        #[derive(Deserialize, Debug)]
        struct Resp {
            #[serde(rename = "list")] // 雪球 API 用 list 不是 statuses
            list: Option<Vec<StatusItem>>,
        }

        // v21.1: 读 XUEQIU_COOKIE env var, 设 Cookie 头 (雪球公共 timeline 实际需 auth)
        // 部署: export XUEQIU_COOKIE="xq_a_token=...; xq_r_token=..."
        // 安全: 严禁 hardcode 在源码/提交到 git
        // v25.1: 移除 Referer/Origin (curl 测试表明这些 header 触发雪球反爬),
        //        只用最简 User-Agent + Cookie
        let mut req = self.client.get(&url).header("User-Agent", "Mozilla/5.0");
        if let Ok(cookie) = std::env::var("XUEQIU_COOKIE") {
            if !cookie.is_empty() {
                log::info!("[xueqiu] 使用 cookie ({} bytes)", cookie.len());
                req = req.header("Cookie", cookie);
            } else {
                log::warn!("[xueqiu] XUEQIU_COOKIE env 已设置但为空");
            }
        } else {
            log::warn!("[xueqiu] XUEQIU_COOKIE 未设置, 可能被反爬");
        }
        let resp: Resp = req
            .send()
            .await
            .with_context(|| format!("雪球 category={} 请求失败", category))?
            .json()
            .await
            .with_context(|| format!("雪球 category={} 解析失败", category))?;

        let now = chrono::Local::now().timestamp_millis();
        let mut results: Vec<SearchResult> = Vec::new();
        for item in resp.list.unwrap_or_default() {
            // 雪球 API: text 字段直接是快讯内容, 不在 data 里
            let title = match item.text.filter(|t| !t.is_empty()) {
                Some(t) => t,
                None => continue,
            };
            // 过滤 6 小时以外的旧新闻 (雪球时间戳是毫秒)
            if let Some(ts_ms) = item.created_at {
                let ts_s = ts_ms / 1000;
                if now / 1000 - ts_s > 6 * 3600 {
                    continue;
                }
            }
            let date_tag = item.created_at.and_then(|ts_ms| {
                chrono::DateTime::from_timestamp(ts_ms / 1000, 0)
                    .map(|dt| dt.with_timezone(&chrono::Local).format("%H:%M").to_string())
            });
            // v21.1: 雪球 API text 即快讯内容, 不需要再清洗 HTML
            let snippet: String = title.chars().take(140).collect();
            // v21.1: 必须先取 target (避免 move 问题) - 后续 url/user 都用
            let target_str = item.target.as_deref().unwrap_or("").to_string();
            let url = if !target_str.is_empty() {
                target_str.clone()
            } else {
                format!("https://xueqiu.com/snowman/category/{}/detail", category)
            };
            let user = target_str.rsplit('/').next().unwrap_or("").to_string();
            let source_label = if user.is_empty() {
                format!("雪球(cat={})", category)
            } else {
                format!("雪球({}@cat={})", user, category)
            };

            results.push(
                SearchResult::new(
                    title.chars().take(80).collect(),
                    snippet.chars().take(240).collect(),
                    url,
                    source_label,
                )
                .with_date(date_tag.unwrap_or_default()),
            );
            if results.len() >= count {
                break;
            }
        }
        Ok(results)
    }

    /// 抓取所有 categories 并合并去重
    pub async fn fetch_flash_news(&self, count_per_category: usize) -> Vec<SearchResult> {
        // v21: 冷却期检查 (避免 429 后反复触发)
        {
            let cd = self.cooldown_until.lock().unwrap();
            if let Some(until) = *cd {
                if chrono::Utc::now() < until {
                    log::debug!("[xueqiu] 冷却中, 跳过拉取");
                    return Vec::new();
                }
            }
        }

        let join = {
            let mut handles = Vec::new();
            for &cat in &self.categories {
                handles.push(self.fetch_category(cat, count_per_category));
            }
            futures::future::join_all(handles).await
        };

        let mut all: Vec<SearchResult> = Vec::new();
        let mut had_error = false;
        for res in join {
            match res {
                Ok(v) => all.extend(v),
                Err(e) => {
                    warn!("[xueqiu] 拉取失败: {}", e);
                    had_error = true;
                }
            }
        }

        // v21: 错误时设冷却 (避免反复触发 429)
        if had_error && all.is_empty() {
            let mut cd = self.cooldown_until.lock().unwrap();
            *cd = Some(chrono::Utc::now() + chrono::Duration::minutes(5));
            warn!("[xueqiu] 全失败, 冷却 5 分钟");
        }

        // 按 title 前 30 字去重
        let mut seen = std::collections::HashSet::new();
        all.retain(|r| {
            let key: String = r.title.chars().take(30).collect();
            seen.insert(key)
        });
        all
    }
}

/// 简单 HTML 标签清理 (用于 description)
#[cfg(test)]
fn strip_html_tags(s: &str) -> String {
    crate::util::strip_html_tags(s)
}

#[async_trait]
impl SearchProvider for XueqiuProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        // 雪球公共 timeline 实际需 cookie, 未设置时直接不可用 (避免反爬空结果噪声).
        std::env::var("XUEQIU_COOKIE")
            .map(|c| !c.trim().is_empty())
            .unwrap_or(false)
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        let started = std::time::Instant::now();
        let items = self.fetch_flash_news(max_results.min(50)).await;
        SearchResponse {
            query: query.to_string(),
            success: true,
            error_message: None,
            search_time: started.elapsed().as_secs_f64(),
            results: items,
            provider: self.name.clone(),
        }
    }
}

impl Default for XueqiuProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_categories() {
        let p = XueqiuProvider::new();
        assert_eq!(p.categories, vec![6, 0]); // A股 + 全部
        assert_eq!(
            p.is_available(),
            std::env::var("XUEQIU_COOKIE")
                .map(|c| !c.trim().is_empty())
                .unwrap_or(false)
        );
        assert_eq!(p.name, "雪球");
    }

    #[test]
    fn test_custom_categories() {
        let p = XueqiuProvider::with_categories(vec![6, 12, 0]);
        assert_eq!(p.categories, vec![6, 12, 0]);
    }

    #[test]
    fn test_strip_html_tags() {
        // strip_html_tags 仅去 HTML 标签, 不动文本内容 (含空格/换行)
        assert_eq!(strip_html_tags("hello <b>world</b>"), "hello world");
        assert_eq!(strip_html_tags("a<br/>b"), "ab");
        assert_eq!(strip_html_tags("no tags"), "no tags");
        assert_eq!(strip_html_tags("<em>重点</em>关注"), "重点关注");
    }
}
