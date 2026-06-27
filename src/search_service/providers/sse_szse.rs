//! 上交所 / 深交所 公告直连 provider。
//!
//! 按股票代码路由到对应交易所的官方公告接口拉取真实公告：
//! - 上交所（6xx/68x/9xx）：`query.sse.com.cn` 公司公告查询（JSONP）
//! - 深交所（0xx/2xx/3xx）：`www.szse.cn` 公告列表（JSON POST）
//!
//! 失败时显式报错，不返回占位/假数据（AGENTS.md 数据红线 2.1）。
//! 交易所接口需具体股票代码，无代码时显式报错而非编造结果。

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use async_trait::async_trait;
use log::{error, info};
use serde_json::json;

use super::super::types::{NewsType, SearchProvider, SearchResponse, SearchResult, Sentiment};

/// 交易所归属。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Market {
    /// 上海证券交易所
    Sse,
    /// 深圳证券交易所
    Szse,
}

/// 上交所 / 深交所 公告 provider。
pub struct SseSzseProvider {
    name: String,
    client: reqwest::Client,
}

impl Default for SseSzseProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SseSzseProvider {
    pub fn new() -> Self {
        Self {
            name: "沪深交易所".to_string(),
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

    /// 根据 6 位股票代码判断交易所归属。
    fn classify_market(code: &str) -> Option<Market> {
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        match &code[..1] {
            // 上交所：主板 6xx，科创板 688/689，B 股 900
            "6" | "9" => Some(Market::Sse),
            // 深交所：主板 0xx，中小板 002，创业板 3xx，B 股 200
            "0" | "2" | "3" => Some(Market::Szse),
            _ => None,
        }
    }

    /// 从查询串提取 6 位股票代码。
    fn extract_code(query: &str) -> Option<String> {
        query
            .split_whitespace()
            .find(|part| part.len() == 6 && part.chars().all(|c| c.is_ascii_digit()))
            .map(|s| s.to_string())
    }

    async fn do_search(&self, query: &str, max_results: usize) -> Result<SearchResponse> {
        let code = match Self::extract_code(query) {
            Some(c) => c,
            None => {
                return Ok(SearchResponse::error(
                    query.to_string(),
                    self.name.clone(),
                    "交易所公告查询需提供 6 位股票代码".to_string(),
                ));
            }
        };

        let market = match Self::classify_market(&code) {
            Some(m) => m,
            None => {
                return Ok(SearchResponse::error(
                    query.to_string(),
                    self.name.clone(),
                    format!("无法识别股票代码 {} 的交易所归属", code),
                ));
            }
        };

        let start = Instant::now();
        let results = match market {
            Market::Sse => self.fetch_sse(&code, max_results).await?,
            Market::Szse => self.fetch_szse(&code, max_results).await?,
        };

        info!(
            "[沪深交易所] 公告查询完成，code={}, market={:?}, 返回 {} 条",
            code,
            market,
            results.len()
        );

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

    /// 拉取上交所公司公告（JSONP 接口）。
    async fn fetch_sse(&self, code: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let ts = chrono::Local::now().timestamp_millis();
        let url = format!(
            "http://query.sse.com.cn/security/stock/queryCompanyBulletinNew.do\
             ?jsonCallBack=jsonpCallback&isPagination=true&SECURITY_CODE={code}\
             &START_DATE=&END_DATE=&BULLETIN_TYPE=\
             &pageHelp.pageSize={size}&pageHelp.pageNo=1&pageHelp.beginPage=1\
             &pageHelp.cacheSize=1&pageHelp.endPage=1&_={ts}",
            code = code,
            size = max_results.max(10),
            ts = ts,
        );

        let text = self
            .client
            .get(&url)
            .header("Referer", "http://www.sse.com.cn/")
            .send()
            .await
            .context("上交所公告请求失败")?
            .text()
            .await
            .context("上交所公告读取失败")?;

        // 去除 JSONP 包装：jsonpCallback({...}) -> {...}
        let json_text = text
            .trim()
            .strip_prefix("jsonpCallback(")
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(&text);

        let value: serde_json::Value =
            serde_json::from_str(json_text).context("上交所公告 JSON 解析失败")?;

        // 结构：{ "pageHelp": { "data": [ [ {..}, {..} ] ] } }
        let data = value
            .get("pageHelp")
            .and_then(|p| p.get("data"))
            .and_then(|d| d.as_array());

        let mut results = Vec::new();
        if let Some(rows) = data {
            for row in rows {
                let items = match row.as_array() {
                    Some(arr) => arr.clone(),
                    None => vec![row.clone()],
                };
                for item in items {
                    let title = item
                        .get("TITLE")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if title.is_empty() {
                        continue;
                    }
                    let date = item
                        .get("SSEDATE")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let rel_url = item.get("URL").and_then(|v| v.as_str()).unwrap_or("");
                    let url = if rel_url.is_empty() {
                        "http://www.sse.com.cn/".to_string()
                    } else if rel_url.starts_with("http") {
                        rel_url.to_string()
                    } else {
                        format!("http://www.sse.com.cn{}", rel_url)
                    };

                    results.push(Self::build_result(title, date, url, "上交所", code));
                    if results.len() >= max_results {
                        return Ok(results);
                    }
                }
            }
        }
        Ok(results)
    }

    /// 拉取深交所公司公告（JSON POST 接口）。
    async fn fetch_szse(&self, code: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let url = "http://www.szse.cn/api/disc/announcement/annList";
        let body = json!({
            "seDate": ["", ""],
            "stock": [code],
            "channelCode": ["listedNotice_disc"],
            "pageSize": max_results.max(10),
            "pageNum": 1,
        });

        let value: serde_json::Value = self
            .client
            .post(url)
            .header("Referer", "http://www.szse.cn/")
            .header("X-Request-Type", "ajax")
            .json(&body)
            .send()
            .await
            .context("深交所公告请求失败")?
            .json()
            .await
            .context("深交所公告解析失败")?;

        let mut results = Vec::new();
        if let Some(rows) = value.get("data").and_then(|d| d.as_array()) {
            for item in rows {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if title.is_empty() {
                    continue;
                }
                let date = item
                    .get("publishTime")
                    .and_then(|v| v.as_str())
                    .map(|s| s.split(' ').next().unwrap_or(s).to_string())
                    .unwrap_or_default();
                let url = item
                    .get("attachPath")
                    .and_then(|v| v.as_str())
                    .filter(|p| !p.is_empty())
                    .map(|p| {
                        if p.starts_with("http") {
                            p.to_string()
                        } else {
                            format!("http://disc.static.szse.cn/download{}", p)
                        }
                    })
                    .unwrap_or_else(|| "http://www.szse.cn/".to_string());

                results.push(Self::build_result(title, date, url, "深交所", code));
                if results.len() >= max_results {
                    break;
                }
            }
        }
        Ok(results)
    }

    /// 构造统一的公告 `SearchResult`。
    fn build_result(
        title: String,
        date: String,
        url: String,
        exchange: &str,
        code: &str,
    ) -> SearchResult {
        let mut result = SearchResult {
            title,
            snippet: String::new(),
            url,
            source: format!("{}({})", exchange, code),
            published_date: if date.is_empty() { None } else { Some(date) },
            news_type: NewsType::Announcement,
            sentiment: Sentiment::Unknown,
            importance: 7, // 交易所官方公告，权威性高
            relevance: 0.85,
            keywords: Vec::new(),
        };
        result.snippet = result.title.clone();
        result.analyze_type();
        result.analyze_sentiment();
        result.calculate_importance();
        result
    }
}

#[async_trait]
impl SearchProvider for SseSzseProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        true // 交易所公开接口，免费可用
    }

    async fn search(&self, query: &str, max_results: usize) -> SearchResponse {
        match self.do_search(query, max_results).await {
            Ok(response) => {
                info!(
                    "[沪深交易所] 搜索 '{}' 完成，返回 {} 条，耗时 {:.2}s",
                    query,
                    response.results.len(),
                    response.search_time
                );
                response
            }
            Err(e) => {
                error!("[沪深交易所] 搜索失败: {}", e);
                SearchResponse::error(query.to_string(), self.name.clone(), e.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_market_sse() {
        assert_eq!(SseSzseProvider::classify_market("600519"), Some(Market::Sse));
        assert_eq!(SseSzseProvider::classify_market("688981"), Some(Market::Sse));
        assert_eq!(SseSzseProvider::classify_market("900957"), Some(Market::Sse));
    }

    #[test]
    fn test_classify_market_szse() {
        assert_eq!(SseSzseProvider::classify_market("000001"), Some(Market::Szse));
        assert_eq!(SseSzseProvider::classify_market("002594"), Some(Market::Szse));
        assert_eq!(SseSzseProvider::classify_market("300750"), Some(Market::Szse));
    }

    #[test]
    fn test_classify_market_invalid() {
        assert_eq!(SseSzseProvider::classify_market("12345"), None);
        assert_eq!(SseSzseProvider::classify_market("ABCDEF"), None);
        assert_eq!(SseSzseProvider::classify_market("700000"), None);
    }

    #[test]
    fn test_extract_code() {
        assert_eq!(
            SseSzseProvider::extract_code("贵州茅台 600519 最新公告"),
            Some("600519".to_string())
        );
        assert_eq!(SseSzseProvider::extract_code("贵州茅台 最新公告"), None);
    }

    #[test]
    fn test_build_result() {
        let r = SseSzseProvider::build_result(
            "关于召开股东大会的公告".to_string(),
            "2024-06-25".to_string(),
            "http://www.sse.com.cn/x.PDF".to_string(),
            "上交所",
            "600519",
        );
        assert_eq!(r.news_type, NewsType::Announcement);
        assert_eq!(r.source, "上交所(600519)");
        assert_eq!(r.published_date, Some("2024-06-25".to_string()));
        assert_eq!(r.snippet, "关于召开股东大会的公告");
    }
}
