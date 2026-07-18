//! 巨潮资讯网（cninfo）直连 provider。
//!
//! 巨潮资讯网是中国证监会指定的 A 股法定信息披露平台，覆盖沪深两市
//! 全部上市公司公告。本 provider 使用其公开全文检索接口拉取真实公告，
//! 失败时显式报错（不返回占位/假数据，符合 AGENTS.md 数据红线 2.1）。

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use log::{error, info};
use serde::Deserialize;

use super::super::types::{NewsType, SearchProvider, SearchResponse, SearchResult, Sentiment};

/// 公告 PDF / 网页正文静态资源前缀。
const STATIC_BASE: &str = "http://static.cninfo.com.cn/";

#[derive(Debug, Deserialize)]
struct CninfoResponse {
    announcements: Option<Vec<CninfoAnnouncement>>,
}

#[derive(Debug, Deserialize)]
struct CninfoAnnouncement {
    #[serde(rename = "announcementTitle")]
    announcement_title: Option<String>,
    /// 发布时间，epoch 毫秒。
    #[serde(rename = "announcementTime")]
    announcement_time: Option<i64>,
    #[serde(rename = "adjunctUrl")]
    adjunct_url: Option<String>,
    #[serde(rename = "secCode")]
    sec_code: Option<String>,
    #[serde(rename = "secName")]
    sec_name: Option<String>,
}

/// 巨潮资讯网公告检索 provider。
pub struct CninfoProvider {
    name: String,
    client: reqwest::Client,
}

impl Default for CninfoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CninfoProvider {
    pub fn new() -> Self {
        Self {
            name: "巨潮资讯".to_string(),
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

    /// 调用巨潮全文检索接口拉取真实公告。
    async fn do_search(&self, query: &str, max_results: usize) -> Result<SearchResponse> {
        let keyword = Self::extract_keyword(query);
        if keyword.is_empty() {
            return Ok(SearchResponse::error(
                query.to_string(),
                self.name.clone(),
                "无法从查询中提取检索关键词".to_string(),
            ));
        }

        let start = Instant::now();
        let url = "http://www.cninfo.com.cn/new/fulltextSearch/full";

        let params = [
            ("searchkey", keyword.as_str()),
            ("sdate", ""),
            ("edate", ""),
            ("isfulltext", "false"),
            ("sortName", "pubdate"),
            ("sortType", "desc"),
            ("pageNum", "1"),
        ];

        let resp: CninfoResponse = self
            .client
            .post(url)
            .header("Origin", "http://www.cninfo.com.cn")
            .header("Referer", "http://www.cninfo.com.cn/new/fulltextSearch")
            .header("X-Requested-With", "XMLHttpRequest")
            .form(&params)
            .send()
            .await
            .context("巨潮资讯检索请求失败")?
            .json()
            .await
            .context("巨潮资讯响应解析失败")?;

        let announcements = resp.announcements.unwrap_or_default();
        info!(
            "[巨潮资讯] 检索完成，query='{}', 返回 {} 条公告",
            query,
            announcements.len()
        );

        let results: Vec<SearchResult> = announcements
            .into_iter()
            .filter_map(Self::to_result)
            .take(max_results)
            .collect();

        let success = !results.is_empty();
        Ok(SearchResponse {
            query: query.to_string(),
            results,
            provider: self.name.clone(),
            success,
            error_message: if success {
                None
            } else {
                Some("无相关公告".to_string())
            },
            search_time: start.elapsed().as_secs_f64(),
        })
    }

    /// 将单条巨潮公告转换为标准 `SearchResult`。
    fn to_result(a: CninfoAnnouncement) -> Option<SearchResult> {
        let raw_title = a.announcement_title?;
        let title = Self::strip_highlight(&raw_title);
        if title.trim().is_empty() {
            return None;
        }

        let sec_code = a.sec_code.unwrap_or_default();
        let sec_name = a.sec_name.unwrap_or_default();
        let source = if sec_name.is_empty() {
            "巨潮资讯".to_string()
        } else {
            format!("巨潮资讯·{}({})", sec_name, sec_code)
        };

        let url = a
            .adjunct_url
            .filter(|u| !u.is_empty())
            .map(|u| format!("{}{}", STATIC_BASE, u))
            .unwrap_or_else(|| "http://www.cninfo.com.cn/".to_string());

        // epoch 毫秒 → 本地日期。
        let published_date = a.announcement_time.and_then(|ms| {
            chrono::DateTime::from_timestamp_millis(ms).map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string()
            })
        });

        let snippet = if sec_name.is_empty() {
            title.clone()
        } else {
            format!("{} {}", sec_name, title)
        };

        let mut result = SearchResult {
            title,
            snippet,
            url,
            source,
            published_date,
            // 巨潮是法定信披平台，默认归类为公告。
            news_type: NewsType::Announcement,
            sentiment: Sentiment::Unknown,
            importance: 7, // 官方信披，权威性高
            relevance: 0.85,
            keywords: Vec::new(),
        };
        result.analyze_type();
        result.analyze_sentiment();
        result.calculate_importance();
        Some(result)
    }

    /// 去除巨潮返回标题中的高亮标签（如 `<em>关键词</em>`）。
    fn strip_highlight(title: &str) -> String {
        title
            .replace("<em>", "")
            .replace("</em>", "")
            .replace("<EM>", "")
            .replace("</EM>", "")
            .trim()
            .to_string()
    }

    /// 从查询串提取检索关键词。
    ///
    /// 优先使用公司名 / 关键词，去掉「股票/最新/消息/新闻」等停用词；
    /// 若仅有 6 位代码也可直接检索。
    fn extract_keyword(query: &str) -> String {
        let stop_words = ["股票", "最新", "消息", "新闻", "公告", "的"];
        let mut code = String::new();
        let mut name_parts: Vec<String> = Vec::new();

        for part in query.split_whitespace() {
            if part.len() == 6 && part.chars().all(|c| c.is_ascii_digit()) {
                code = part.to_string();
            } else if !stop_words.iter().any(|w| part.contains(w)) {
                name_parts.push(part.to_string());
            }
        }

        if !name_parts.is_empty() {
            name_parts.join(" ")
        } else {
            code
        }
    }
}

#[async_trait]
impl SearchProvider for CninfoProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        true // 巨潮资讯公开接口，免费可用
    }

    fn supports_topic_search(&self) -> bool {
        false // 法定公告全文检索, 宽泛主题词几乎无匹配, 保留给按代码查公告
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        match self.do_search(query, max_results).await {
            Ok(response) => {
                info!(
                    "[巨潮资讯] 搜索 '{}' 完成，返回 {} 条，耗时 {:.2}s",
                    query,
                    response.results.len(),
                    response.search_time
                );
                response
            }
            Err(e) => {
                error!("[巨潮资讯] 搜索失败: {}", e);
                SearchResponse::error(query.to_string(), self.name.clone(), e.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_highlight() {
        assert_eq!(
            CninfoProvider::strip_highlight("关于<em>重大资产重组</em>的公告"),
            "关于重大资产重组的公告"
        );
        assert_eq!(CninfoProvider::strip_highlight("普通标题"), "普通标题");
    }

    #[test]
    fn test_extract_keyword_with_name_and_code() {
        assert_eq!(
            CninfoProvider::extract_keyword("贵州茅台 600519 股票 最新消息"),
            "贵州茅台"
        );
    }

    #[test]
    fn test_extract_keyword_code_only() {
        assert_eq!(CninfoProvider::extract_keyword("600519 公告"), "600519");
    }

    #[test]
    fn test_extract_keyword_empty() {
        assert_eq!(CninfoProvider::extract_keyword("股票 最新 消息"), "");
    }

    #[test]
    fn test_to_result_skips_empty_title() {
        let a = CninfoAnnouncement {
            announcement_title: Some("  ".to_string()),
            announcement_time: Some(1_719_331_200_000),
            adjunct_url: Some("finalpage/2024-06-25/x.PDF".to_string()),
            sec_code: Some("000001".to_string()),
            sec_name: Some("平安银行".to_string()),
        };
        assert!(CninfoProvider::to_result(a).is_none());
    }

    #[test]
    fn test_to_result_builds_url_and_date() {
        let a = CninfoAnnouncement {
            announcement_title: Some("关于<em>分红</em>的公告".to_string()),
            announcement_time: Some(1_719_331_200_000),
            adjunct_url: Some("finalpage/2024-06-25/x.PDF".to_string()),
            sec_code: Some("000001".to_string()),
            sec_name: Some("平安银行".to_string()),
        };
        let r = CninfoProvider::to_result(a).expect("应生成结果");
        assert_eq!(r.title, "关于分红的公告");
        assert!(r.url.starts_with("http://static.cninfo.com.cn/"));
        assert!(r.published_date.is_some());
        assert_eq!(r.news_type, NewsType::Announcement);
    }
}
