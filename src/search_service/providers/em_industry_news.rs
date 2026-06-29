//! 东财行业新闻流 provider（基于 cmsArticle 关键词搜索）
//!
//! 数据源：`https://search-api-web.eastmoney.com/search/jsonp`
//! - type=cmsArticle (全量文章搜索)
//! - 关键词=行业名（半导体/光伏/医药 等）
//! - 返回: date/code/title(HTML高亮)/mediaName/content
//!
//! 特点：
//! - 完全免费，无需 API Key
//! - 10 个 BOM 行业链 × 关键词并发 = 「行业新闻流」
//! - hitsTotal=10000+（取最新 30 条/行业）
//! - 媒体来源覆盖: 界面新闻/第一财经/东财证券/21财经/证券时报 等
//!
//! 与 EmAnnouncementProvider 的区别：
//! - 公告流 = 上市公司一手披露（信披合规，权威但慢）
//! - 行业新闻流 = 全网新闻聚合（快，但需交叉验证）
//!
//! 用法：
//! - 直接对接 BOM 链路: chain_mapper 拿到 event.subjects 后，
//!   用这些关键词查 em_industry_news 拿到最新 6h 新闻标题
//! - 下游可喂 event_extractor 做事件扩展

use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use log::warn;
use serde::Deserialize;

use super::super::types::{SearchProvider, SearchResponse, SearchResult};

/// BOM 行业关键词列表（与 bom_kb.rs 的 10 个 chain 对齐）
pub const INDUSTRY_KEYWORDS: &[(&str, &str)] = &[
    ("半导体", "半导体/芯片/晶圆/封测/HBM/先进封装"),
    ("新能源车", "新能源车/锂电池/正极材料/隔膜/电解液"),
    ("光伏", "光伏/硅料/硅片/电池片/组件"),
    ("医药", "医药/创新药/CRO/原料药"),
    ("消费电子", "消费电子/手机/果链/面板"),
    ("军工", "军工/航空/导弹/船舶"),
    ("化工", "化工/化纤/磷化工/纯碱"),
    ("计算机", "计算机/AI/软件/SaaS"),
    ("通信", "通信/光模块/运营商/卫星"),
    ("银行", "银行/保险/券商/金融"),
];

/// 东财行业新闻流 provider
pub struct EmIndustryNewsProvider {
    name: String,
    client: reqwest::Client,
}

impl EmIndustryNewsProvider {
    pub fn new() -> Self {
        Self {
            name: "东财行业新闻".to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .build()
                .unwrap(),
        }
    }

    /// 拉取单个关键词的最新文章
    ///
    /// - `keyword`: 关键词（如 "半导体"）
    /// - `per_keyword_limit`: 每个关键词返回的最新文章数
    pub async fn fetch_by_keyword(
        &self,
        keyword: &str,
        per_keyword_limit: usize,
    ) -> Result<Vec<SearchResult>> {
        #[derive(Deserialize, Debug)]
        struct Article {
            date: Option<String>,
            code: Option<String>,
            title: Option<String>,
            #[serde(default)]
            media_name: Option<String>,
            #[serde(default)]
            content: Option<String>,
        }

        #[derive(Deserialize, Debug)]
        struct CmsResult {
            #[serde(default)]
            cms_article: Vec<Article>,
        }

        #[derive(Deserialize, Debug)]
        struct Resp {
            #[serde(default)]
            code: i32,
            #[serde(default)]
            #[allow(dead_code)]
            hits_total: i64,
            #[serde(default)]
            result: Option<CmsResult>,
        }

        // 内层 param 用单层 JSON, 不要嵌套
        let param = serde_json::json!({
            "uid": "",
            "keyword": keyword,
            "type": ["cmsArticle"],
            "client": "web",
            "clientType": "web",
            "clientVersion": "curr",
            "pageIndex": 1,
            "pageSize": per_keyword_limit.min(30),
            "sortType": "1",  // 按时间倒序
        });

        let url = format!(
            "https://search-api-web.eastmoney.com/search/jsonp?cb=cb&param={}",
            urlencode(&param.to_string())
        );

        let body = self
            .client
            .get(&url)
            .header("Referer", "https://so.eastmoney.com/")
            .send()
            .await
            .with_context(|| format!("东财行业新闻 kw={} 请求失败", keyword))?
            .text()
            .await
            .with_context(|| format!("东财行业新闻 kw={} 读取 body 失败", keyword))?;

        // JSONP 包裹: cb({...}) → 剥掉 cb(...) 拿到 JSON
        let json_str = if let Some(start) = body.find('(') {
            if let Some(end) = body.rfind(')') {
                &body[start + 1..end]
            } else {
                &body
            }
        } else {
            &body
        };

        let resp: Resp = serde_json::from_str(json_str)
            .with_context(|| format!("东财行业新闻 kw={} 解析失败", keyword))?;

        if resp.code != 0 {
            return Ok(Vec::new());
        }

        let articles = resp.result.map(|r| r.cms_article).unwrap_or_default();
        let now = chrono::Local::now().timestamp();
        let mut results: Vec<SearchResult> = Vec::new();
        for a in articles {
            let raw_title = match a.title.filter(|t| !t.is_empty()) {
                Some(t) => t,
                None => continue,
            };
            let title = strip_html_tags(&raw_title);
            if title.is_empty() {
                continue;
            }

            // 6 小时过滤
            let date_tag = a.date.as_deref().and_then(|d| {
                chrono::NaiveDateTime::parse_from_str(d, "%Y-%m-%d %H:%M:%S")
                    .ok()
                    .and_then(|ndt| ndt.and_local_timezone(chrono::Local).single())
            });
            if let Some(dt) = date_tag {
                if now - dt.timestamp() > 6 * 3600 {
                    continue;
                }
            }

            let art_code = a.code.unwrap_or_default();
            let url = if art_code.is_empty() {
                format!("https://so.eastmoney.com/web/s?keyword={}", urlencode(keyword))
            } else {
                format!("https://so.eastmoney.com/news/detail/{}", art_code)
            };

            let media = a
                .media_name
                .filter(|s| !s.is_empty())
                .map(|s| format!("[{}] ", s))
                .unwrap_or_default();
            let snippet = a
                .content
                .map(|s| strip_html_tags(&s))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| title.chars().take(160).collect());

            let date_str = date_tag
                .map(|dt| dt.format("%H:%M").to_string())
                .unwrap_or_default();

            // 标题前缀: [半导体] [媒体名] + 标题
            let enriched_title = format!("[{}]{}{}", keyword, media, title);

            results.push(
                SearchResult::new(
                    enriched_title.chars().take(140).collect(),
                    snippet.chars().take(240).collect(),
                    url,
                    "东财行业新闻".to_string(),
                )
                .with_date(date_str),
            );
            if results.len() >= per_keyword_limit {
                break;
            }
        }
        Ok(results)
    }

    /// 一次性拉取所有 BOM 行业关键词的新闻（10 个行业并发）
    ///
    /// - `per_keyword_limit`: 每个行业取最新 N 条
    /// - 返回合并后按时间排序（最新在前）
    pub async fn fetch_all_industries(
        &self,
        per_keyword_limit: usize,
    ) -> Vec<SearchResult> {
        // 10 个行业并发
        let futures: Vec<_> = INDUSTRY_KEYWORDS
            .iter()
            .map(|(kw, _)| async move { (kw.to_string(), self.fetch_by_keyword(kw, per_keyword_limit).await) })
            .collect();

        let results = futures::future::join_all(futures).await;

        let mut all: Vec<SearchResult> = Vec::new();
        for (kw, res) in results {
            match res {
                Ok(mut v) => {
                    info_ind(&kw, v.len());
                    all.append(&mut v);
                }
                Err(e) => warn!("[em_industry] kw={} 失败: {}", kw, e),
            }
        }

        // 按 published_date 字符串倒序（虽然字符串排序不严格，但够用）
        // 实际数据时间格式都是 HH:MM，同日有效
        all.sort_by(|a, b| b.published_date.cmp(&a.published_date));
        all
    }
}

fn info_ind(kw: &str, n: usize) {
    log::info!("[em_industry][{}] 命中 {} 条", kw, n);
}

/// 简单 URL 编码（只处理必要字符）
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 简单 HTML 标签剥离（处理 <em>关键词</em> 这种高亮）
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

#[async_trait]
impl SearchProvider for EmIndustryNewsProvider {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        true
    }

    /// 默认 fetch_all_industries(10) 作为 SearchProvider 接口
    async fn search(&self, _query: &str, max_results: usize) -> SearchResponse {
        let started = std::time::Instant::now();
        let items = self.fetch_all_industries(max_results.min(30)).await;
        SearchResponse {
            query: _query.to_string(),
            success: true,
            error_message: None,
            search_time: started.elapsed().as_secs_f64(),
            results: items,
            provider: self.name.clone(),
        }
    }
}

impl Default for EmIndustryNewsProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(
            strip_html_tags("<em>半导体</em>ETF鹏华涨超4%"),
            "半导体ETF鹏华涨超4%"
        );
        assert_eq!(strip_html_tags("央行加息 25bp"), "央行加息 25bp");
        assert_eq!(strip_html_tags("<a href='x'>link</a>"), "link");
        assert_eq!(strip_html_tags(""), "");
        assert_eq!(strip_html_tags("<em>"), "");
    }

    #[test]
    fn test_urlencode() {
        assert_eq!(urlencode("半导体"), "%E5%8D%8A%E5%AF%BC%E4%BD%93");
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn test_industry_keywords_count() {
        // 与 bom_kb.rs 10 个 chain 对齐
        assert_eq!(INDUSTRY_KEYWORDS.len(), 10);
    }

    #[test]
    fn test_search_provider_trait_name() {
        let p = EmIndustryNewsProvider::new();
        assert_eq!(p.name(), "东财行业新闻");
        assert!(p.is_available());
    }

    #[test]
    fn test_industry_keywords_have_unique_names() {
        let mut seen = std::collections::HashSet::new();
        for (name, _) in INDUSTRY_KEYWORDS {
            assert!(seen.insert(*name), "重复 industry name: {}", name);
        }
    }
}