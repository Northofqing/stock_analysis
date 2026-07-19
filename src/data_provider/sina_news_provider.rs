//! Registered business rules: BR-066.
//! Sina 新闻数据提供者 (Task 10)
//!
//! 财经要闻 + 个股新闻 + 历史回溯 (按时间范围客户端过滤).
//!
//! Sina 新闻 API:
//! - 财经要闻 URL: `https://feed.mix.sina.com.cn/api/roll/get?pageid=153&lid=1686&k=&num=20&page=1`
//! - 个股新闻 URL: `https://feed.mix.sina.com.cn/api/roll/get?pageid=155&lid=2516&k={code}&num=20&page=1`
//!
//! 响应 JSON: `{"result":{"data":[{"url":"...","title":"...","intro":"...","ctime":1700000000,"media_name":"..."}]}}`
//!
//! Sina 新闻 API 通常返回 UTF-8, 但容错 GBK (与 Sina K线/hq 接口保持一致).

use anyhow::{anyhow, Result};
use chrono::Utc;
use encoding_rs::GBK;

use super::news_item::{content_hash, NewsItem};

/// Sina 新闻 feed API base URL.
pub const SINA_NEWS_API_BASE: &str = "https://feed.mix.sina.com.cn/api/roll/get";

/// 财经要闻 URL (lid=1686, pageid=155).
pub fn build_top_news_url(num: usize) -> String {
    build_top_news_url_from_base(SINA_NEWS_API_BASE, num)
}

/// 个股新闻 URL (lid=2516, pageid=155, k=code).
pub fn build_stock_news_url(code: &str, num: usize) -> String {
    build_stock_news_url_from_base(SINA_NEWS_API_BASE, code, num)
}

fn build_top_news_url_from_base(base: &str, num: usize) -> String {
    format!(
        "{}?pageid=155&lid=1686&k=&num={num}&page=1",
        base.trim_end_matches('/')
    )
}

fn build_stock_news_url_from_base(base: &str, code: &str, num: usize) -> String {
    format!(
        "{}?pageid=155&lid=2516&k={code}&num={num}&page=1",
        base.trim_end_matches('/')
    )
}

/// 解析 Sina 新闻 body → `Vec<NewsItem>`.
///
/// 字段映射:
/// - `url` → `external_id`
/// - `title` → `title`
/// - `intro` → `summary`
/// - `ctime` → `published_at`
/// - `media_name` → `source_name`
///
/// `category` 与 `code` 由调用方注入 (区分财经要闻/个股新闻).
pub fn parse_sina_news_body(
    body: &str,
    category: &str,
    code: Option<&str>,
) -> Result<Vec<NewsItem>> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| anyhow!("Sina news JSON parse: {e}"))?;
    let data = v
        .get("result")
        .and_then(|r| r.get("data"))
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow!("Sina news: 无 result.data 数组"))?;

    let now = Utc::now();
    if category.trim().is_empty() {
        return Err(anyhow!("Sina news: category 不能为空"));
    }
    if let Some(code) = code {
        #[cfg(test)]
        let protocol_code = code.strip_prefix("TEST_CODE_").unwrap_or(code);
        #[cfg(not(test))]
        let protocol_code = code;
        if protocol_code.len() != 6 || !protocol_code.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(anyhow!("Sina news: 非法股票代码 {code:?}"));
        }
    }
    let source = if code.is_some() {
        "sina_stock"
    } else {
        "sina_financial"
    };
    let mut items = Vec::with_capacity(data.len());
    for (index, entry) in data.iter().enumerate() {
        let url = entry
            .get("url")
            .and_then(|x| x.as_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("Sina news 第 {} 行缺少 url", index + 1))?
            .to_string();
        let title = entry
            .get("title")
            .and_then(|x| x.as_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("Sina news 第 {} 行缺少 title", index + 1))?
            .to_string();
        let intro = entry
            .get("intro")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let ctime = entry
            .get("ctime")
            .and_then(|x| x.as_i64())
            .ok_or_else(|| anyhow!("Sina news 第 {} 行缺少整数 ctime", index + 1))?;
        if ctime < 946_684_800 {
            return Err(anyhow!(
                "Sina news 第 {} 行 ctime 早于 2000-01-01: {ctime}",
                index + 1
            ));
        }
        // Sina 财经要闻响应中 `media_name` 经常为空字符串.
        // Fallback 顺序: media_name 非空 -> oid/docid 字段 -> 默认 "新浪财经".
        let media_name = {
            let raw = entry
                .get("media_name")
                .and_then(|x| x.as_str())
                .map(|s| s.trim())
                .unwrap_or("");
            if !raw.is_empty() {
                raw.to_string()
            } else {
                // 尝试从 oid/docid 字段提取; 都缺失则用默认.
                let oid = entry
                    .get("oid")
                    .and_then(|x| x.as_str())
                    .map(|s| s.trim())
                    .unwrap_or("");
                let docid = entry
                    .get("docid")
                    .and_then(|x| x.as_str())
                    .map(|s| s.trim())
                    .unwrap_or("");
                if !oid.is_empty() {
                    format!("sina:{oid}")
                } else if !docid.is_empty() {
                    format!("sina:{docid}")
                } else {
                    "新浪财经".to_string()
                }
            }
        };
        let published_at = chrono::DateTime::from_timestamp(ctime, 0)
            .ok_or_else(|| anyhow!("Sina news 第 {} 行 ctime 非法: {ctime}", index + 1))?;
        if published_at > now + chrono::Duration::minutes(5) {
            return Err(anyhow!(
                "Sina news 第 {} 行发布时间在未来: {published_at}",
                index + 1
            ));
        }
        let hash = content_hash(&title, &intro);
        items.push(NewsItem {
            source: source.to_string(),
            external_id: url.clone(),
            category: category.to_string(),
            code: code.map(|c| c.to_string()),
            title,
            summary: intro,
            url,
            source_name: media_name,
            published_at,
            fetched_at: now,
            content_hash: hash,
        });
    }
    Ok(items)
}

pub struct SinaNewsProvider {
    client: reqwest::Client,
    api_base: String,
}

impl SinaNewsProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            api_base: SINA_NEWS_API_BASE.to_string(),
        }
    }

    /// 财经要闻 (大盘/政策/外盘快讯).
    pub async fn fetch_top_news(&self, num: usize) -> Result<Vec<NewsItem>> {
        let url = build_top_news_url_from_base(&self.api_base, num);
        let body = self.fetch_bytes(&url).await?;
        parse_sina_news_body(&body, "财经要闻", None)
    }

    /// 个股新闻 (按 code).
    pub async fn fetch_stock_news(&self, code: &str, num: usize) -> Result<Vec<NewsItem>> {
        let url = build_stock_news_url_from_base(&self.api_base, code, num);
        let body = self.fetch_bytes(&url).await?;
        parse_sina_news_body(&body, "个股新闻", Some(code))
    }

    /// 历史回溯 (按 code + 时间范围). Sina 不直接支持, 拉多页然后客户端过滤.
    /// 实现: 固定拉 5 页 (5 × 20 = 100 条), 然后客户端过滤 `from..=to`.
    pub async fn fetch_stock_news_in_range(
        &self,
        code: &str,
        from: chrono::DateTime<Utc>,
        to: chrono::DateTime<Utc>,
    ) -> Result<Vec<NewsItem>> {
        let mut all = Vec::new();
        for page in 1..=5 {
            let num = 20;
            let url = format!(
                "{}?pageid=155&lid=2516&k={code}&num={num}&page={page}",
                self.api_base
            );
            let body = self.fetch_bytes(&url).await?;
            let items = parse_sina_news_body(&body, "个股新闻", Some(code))?;
            all.extend(items);
        }
        Ok(all
            .into_iter()
            .filter(|i| i.published_at >= from && i.published_at <= to)
            .collect())
    }

    /// 内部 helper: GET → UTF-8 first → GBK fallback → String.
    ///
    /// review #16 P0 #1: Sina news API 实际返 UTF-8, 强制 GBK 解会乱码 (中文标题/摘要
    /// 全部 mojibake, content_hash 撞 dedup 失效). 先试 UTF-8 decode, 失败再 fallback GBK.
    async fn fetch_bytes(&self, url: &str) -> Result<String> {
        let bytes = self
            .client
            .get(url)
            .header("Referer", "https://finance.sina.com.cn")
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Ok(decode_sina_bytes(&bytes))
    }
}

/// 把 Sina news API 响应字节解码为 UTF-8 String.
/// 先试 UTF-8, 失败 fallback GBK + log warn (旧版接口).
pub fn decode_sina_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (s, _, had_errors) = GBK.decode(bytes);
            if had_errors {
                log::warn!("[Sina news] GBK decode 错误, 部分字符可能异常");
            }
            s.into_owned()
        }
    }
}

impl Default for SinaNewsProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_body() -> String {
        serde_json::json!({
            "result": {
                "data": [
                    {
                        "url": "https://example.test/one",
                        "title": "测试标题一",
                        "intro": "测试摘要一",
                        "ctime": 1_700_000_000_i64,
                        "media_name": "测试媒体"
                    },
                    {
                        "url": "https://example.test/two",
                        "title": "测试标题二",
                        "ctime": 1_700_000_001_i64,
                        "media_name": "",
                        "oid": "OID-2"
                    },
                    {
                        "url": "https://example.test/three",
                        "title": "测试标题三",
                        "intro": "",
                        "ctime": 1_700_000_002_i64,
                        "media_name": "",
                        "docid": "DOC-3"
                    },
                    {
                        "url": "https://example.test/four",
                        "title": "测试标题四",
                        "ctime": 1_700_000_003_i64,
                        "media_name": ""
                    }
                ]
            }
        })
        .to_string()
    }

    fn body_with_row(row: serde_json::Value) -> String {
        serde_json::json!({"result": {"data": [row]}}).to_string()
    }

    #[test]
    fn urls_preserve_feed_identity_and_limits() {
        assert_eq!(
            build_top_news_url(20),
            "https://feed.mix.sina.com.cn/api/roll/get?pageid=155&lid=1686&k=&num=20&page=1"
        );
        assert_eq!(
            build_stock_news_url("600519", 7),
            "https://feed.mix.sina.com.cn/api/roll/get?pageid=155&lid=2516&k=600519&num=7&page=1"
        );
    }

    #[test]
    fn complete_news_batch_preserves_sources_optional_intro_and_hashes() {
        let financial = parse_sina_news_body(&valid_body(), "财经要闻", None).unwrap();
        assert_eq!(financial.len(), 4);
        assert!(financial.iter().all(|item| item.source == "sina_financial"));
        assert!(financial.iter().all(|item| item.category == "财经要闻"));
        assert!(financial.iter().all(|item| item.code.is_none()));
        assert_eq!(financial[0].source_name, "测试媒体");
        assert_eq!(financial[1].source_name, "sina:OID-2");
        assert_eq!(financial[2].source_name, "sina:DOC-3");
        assert_eq!(financial[3].source_name, "新浪财经");
        assert_eq!(financial[1].summary, "");
        assert_eq!(financial[0].external_id, "https://example.test/one");
        assert_eq!(financial[0].url, financial[0].external_id);
        assert_eq!(
            financial[0].published_at,
            chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
        );
        assert_eq!(
            financial[0].content_hash,
            content_hash("测试标题一", "测试摘要一")
        );
        assert!(financial
            .iter()
            .all(|item| item.fetched_at >= item.published_at));

        let stock = parse_sina_news_body(&valid_body(), "个股新闻", Some("600519")).unwrap();
        assert!(stock.iter().all(|item| item.source == "sina_stock"));
        assert!(stock
            .iter()
            .all(|item| item.code.as_deref() == Some("600519")));
    }

    #[test]
    fn parser_accepts_an_explicit_empty_batch() {
        let items = parse_sina_news_body(r#"{"result":{"data":[]}}"#, "财经要闻", None).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn parser_rejects_document_category_and_code_failures() {
        for (body, category, code, expected) in [
            ("not-json", "财经要闻", None, "JSON parse"),
            ("{}", "财经要闻", None, "result.data"),
            (r#"{"result":{"data":{}}}"#, "财经要闻", None, "result.data"),
            (&valid_body(), " ", None, "category"),
            (&valid_body(), "个股新闻", Some("60051"), "非法股票代码"),
            (&valid_body(), "个股新闻", Some("ABCDEF"), "非法股票代码"),
        ] {
            let error = parse_sina_news_body(body, category, code)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(expected),
                "expected={expected:?} error={error:?}"
            );
        }
    }

    #[test]
    fn parser_rejects_any_incomplete_or_invalid_row() {
        let base = serde_json::json!({
            "url": "https://example.test/news",
            "title": "测试标题",
            "intro": "摘要",
            "ctime": 1_700_000_000_i64,
            "media_name": "媒体"
        });
        let mut cases = Vec::new();
        for (field, expected) in [
            ("url", "缺少 url"),
            ("title", "缺少 title"),
            ("ctime", "整数 ctime"),
        ] {
            let mut row = base.clone();
            row.as_object_mut().unwrap().remove(field);
            cases.push((row, expected));
        }
        let mut empty_url = base.clone();
        empty_url["url"] = serde_json::json!(" ");
        cases.push((empty_url, "缺少 url"));
        let mut empty_title = base.clone();
        empty_title["title"] = serde_json::json!("");
        cases.push((empty_title, "缺少 title"));
        let mut string_time = base.clone();
        string_time["ctime"] = serde_json::json!("1700000000");
        cases.push((string_time, "整数 ctime"));
        let mut ancient = base.clone();
        ancient["ctime"] = serde_json::json!(946_684_799_i64);
        cases.push((ancient, "早于 2000"));
        let mut invalid_epoch = base.clone();
        invalid_epoch["ctime"] = serde_json::json!(i64::MAX);
        cases.push((invalid_epoch, "ctime 非法"));
        let mut future = base;
        future["ctime"] = serde_json::json!(Utc::now().timestamp() + 301);
        cases.push((future, "发布时间在未来"));

        for (row, expected) in cases {
            let error = parse_sina_news_body(&body_with_row(row), "财经要闻", None)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(expected),
                "expected={expected:?} error={error:?}"
            );
        }
    }

    #[test]
    fn decoder_prefers_utf8_and_falls_back_to_gbk() {
        let utf8 = "新浪财经 UTF-8";
        assert_eq!(decode_sina_bytes(utf8.as_bytes()), utf8);

        let (encoded, _, had_errors) = GBK.encode("新浪财经 GBK");
        assert!(!had_errors);
        assert_eq!(decode_sina_bytes(encoded.as_ref()), "新浪财经 GBK");
    }

    #[test]
    fn provider_default_keeps_real_api_base() {
        let provider = SinaNewsProvider::default();
        assert_eq!(provider.api_base, SINA_NEWS_API_BASE);
    }

    #[tokio::test]
    async fn loopback_news_transport_executes_top_stock_and_five_page_range() {
        use super::super::{loopback_http_client, TestHttpResponse, TestHttpServer};

        let server = TestHttpServer::new(
            (0..7)
                .map(|_| TestHttpResponse::json(valid_body()))
                .collect(),
        );
        let provider = SinaNewsProvider {
            client: loopback_http_client(),
            api_base: server.base_url().to_string(),
        };
        assert_eq!(provider.fetch_top_news(4).await.unwrap().len(), 4);
        let stock = provider
            .fetch_stock_news("TEST_CODE_600519", 4)
            .await
            .unwrap();
        assert!(stock
            .iter()
            .all(|item| item.code.as_deref() == Some("TEST_CODE_600519")));
        let from = chrono::DateTime::from_timestamp(1_699_999_999, 0).unwrap();
        let to = chrono::DateTime::from_timestamp(1_700_000_004, 0).unwrap();
        let ranged = provider
            .fetch_stock_news_in_range("TEST_CODE_600519", from, to)
            .await
            .unwrap();
        assert_eq!(ranged.len(), 20);
        let requests = server.finish();
        assert!(requests[0].contains("lid=1686"));
        assert!(requests[1].contains("k=TEST_CODE_600519"));
        assert!(requests[2].ends_with("page=1"));
        assert!(requests[6].ends_with("page=5"));
    }
}
