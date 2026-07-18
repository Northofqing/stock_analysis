wrote /Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-10-brief.md: 224 lines
_provider/sina_news_provider.rs`
- Modify: `src/data_provider/mod.rs` (注册)

- [ ] **Step 1: Write failing tests**

```rust
// tests/sina_news_provider_test.rs
use stock_analysis::data_provider::sina_news_provider::{
    SinaNewsProvider, build_top_news_url, build_stock_news_url,
    parse_sina_news_body,
};

#[test]
fn build_top_news_url() {
    let url = build_top_news_url(20);
    assert!(url.contains("feed.mix.sina.com.cn"));
    assert!(url.contains("lid=1686"));
    assert!(url.contains("num=20"));
}

#[test]
fn build_stock_news_url() {
    let url = build_stock_news_url("600000", 20);
    assert!(url.contains("lid=2516"));
    assert!(url.contains("k=600000"));
}

#[test]
fn parse_sina_news_body_extracts_items() {
    // Sina 真实响应格式 (实测): 
    // {"result":{"data":[{"title":"...","url":"...","intro":"...","ctime":1700000000,"media_name":"..."}]}}
    let body = r#"{"result":{"data":[{"url":"https://example.com/1","title":"新闻1","intro":"摘要1","ctime":1700000000,"media_name":"新浪财经"}]}}"#;
    let items = parse_sina_news_body(body, "财经要闻", None).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "新闻1");
    assert_eq!(items[0].url, "https://example.com/1");
    assert_eq!(items[0].summary, "摘要1");
    assert_eq!(items[0].category, "财经要闻");
    assert_eq!(items[0].code, None);  // 财经要闻无 code
    assert_eq!(items[0].content_hash.len(), 64);
}

#[test]
fn parse_sina_news_body_with_code() {
    let body = r#"{"result":{"data":[{"url":"https://example.com/2","title":"股票新闻","intro":"摘要2","ctime":1700000000,"media_name":"新浪财经"}]}}"#;
    let items = parse_sina_news_body(body, "个股新闻", Some("600000")).unwrap();
    assert_eq!(items[0].code, Some("600000".to_string()));
}
```

- [ ] **Step 2: Run tests, verify FAIL**

```bash
cargo test --test sina_news_provider_test
```

Expected: FAIL — `sina_news_provider` module 不存在.

- [ ] **Step 3: Implement SinaNewsProvider**

```rust
// src/data_provider/sina_news_provider.rs
use anyhow::{anyhow, Result};
use chrono::Utc;
use encoding_rs::GBK;

use super::news_item::{content_hash, NewsItem};

pub struct SinaNewsProvider {
    client: reqwest::Client,
    api_base: String,  // "https://feed.mix.sina.com.cn/api/roll/get"
}

const SINA_NEWS_API_BASE: &str = "https://feed.mix.sina.com.cn/api/roll/get";

/// 财经要闻 URL (lid=1686).
pub fn build_top_news_url(num: usize) -> String {
    format!(
        "{SINA_NEWS_API_BASE}?pageid=153&lid=1686&k=&num={num}&page=1"
    )
}

/// 个股新闻 URL (lid=2516, k=code).
pub fn build_stock_news_url(code: &str, num: usize) -> String {
    format!(
        "{SINA_NEWS_API_BASE}?pageid=155&lid=2516&k={code}&num={num}&page=1"
    )
}

/// 解析 Sina 新闻 body → Vec<NewsItem>.
/// 字段映射: url → external_id, title, intro → summary, ctime → published_at, media_name → source_name.
pub fn parse_sina_news_body(body: &str, category: &str, code: Option<&str>) -> Result<Vec<NewsItem>> {
    // 解析外层 result.data
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| anyhow!("Sina news JSON parse: {e}"))?;
    let data = v.get("result")
        .and_then(|r| r.get("data"))
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow!("Sina news: 无 result.data 数组"))?;
    
    let now = Utc::now();
    let mut items = Vec::with_capacity(data.len());
    for entry in data {
        let url = entry.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let title = entry.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let intro = entry.get("intro").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let ctime = entry.get("ctime").and_then(|x| x.as_i64()).unwrap_or(0);
        let media_name = entry.get("media_name").and_then(|x| x.as_str()).unwrap_or("新浪财经").to_string();
        let source = if code.is_some() { "sina_stock" } else { "sina_financial" };
        let published_at = chrono::DateTime::from_timestamp(ctime, 0)
            .unwrap_or_else(|| now);
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

impl SinaNewsProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client, api_base: SINA_NEWS_API_BASE.to_string() }
    }
    
    /// 财经要闻 (大盘/政策/外盘快讯).
    pub async fn fetch_top_news(&self, num: usize) -> Result<Vec<NewsItem>> {
        let url = build_top_news_url(num);
        let bytes = self.client.get(&url)
            .header("Referer", "https://finance.sina.com.cn")
            .send().await?
            .error_for_status()?
            .bytes().await?;
        // Sina 新闻 API 通常返回 UTF-8, 但容错 GBK
        let (utf8, _, _) = GBK.decode(&bytes);
        let body = utf8.into_owned();
        parse_sina_news_body(&body, "财经要闻", None)
    }
    
    /// 个股新闻 (按 code).
    pub async fn fetch_stock_news(&self, code: &str, num: usize) -> Result<Vec<NewsItem>> {
        let url = build_stock_news_url(code, num);
        let bytes = self.client.get(&url)
            .header("Referer", "https://finance.sina.com.cn")
            .send().await?
            .error_for_status()?
            .bytes().await?;
        let (utf8, _, _) = GBK.decode(&bytes);
        let body = utf8.into_owned();
        parse_sina_news_body(&body, "个股新闻", Some(code))
    }
    
    /// 历史回溯 (按 code + 时间范围). Sina 不直接支持, 拉多页然后过滤.
    /// Phase 2 实现: 固定拉 5 页 (5 × 20 = 100 条), 然后客户端过滤.
    pub async fn fetch_stock_news_in_range(
        &self, code: &str, from: chrono::DateTime<Utc>, to: chrono::DateTime<Utc>,
    ) -> Result<Vec<NewsItem>> {
        let mut all = Vec::new();
        for page in 1..=5 {
            let num = 20;
            let url = format!(
                "{}?pageid=155&lid=2516&k={code}&num={num}&page={page}",
                self.api_base
            );
            let bytes = self.client.get(&url)
                .header("Referer", "https://finance.sina.com.cn")
                .send().await?
                .error_for_status()?
                .bytes().await?;
            let (utf8, _, _) = GBK.decode(&bytes);
            let body = utf8.into_owned();
            let items = parse_sina_news_body(&body, "个股新闻", Some(code))?;
            all.extend(items);
        }
        // 客户端过滤时间范围
        let filtered: Vec<NewsItem> = all.into_iter()
            .filter(|i| i.published_at >= from && i.published_at <= to)
            .collect();
        Ok(filtered)
    }
}
```

- [ ] **Step 4: Register module**

```rust
// src/data_provider/mod.rs
pub mod sina_news_provider;
```

- [ ] **Step 5: Run tests, verify PASS**

```bash
cargo test --test sina_news_provider_test
```

Expected: 4 tests passed.

- [ ] **Step 6: Commit**

```bash
git add src/data_provider/sina_news_provider.rs src/data_provider/mod.rs tests/sina_news_provider_test.rs
git commit -m "feat(news): add SinaNewsProvider (top + stock + history range)"
```

---

