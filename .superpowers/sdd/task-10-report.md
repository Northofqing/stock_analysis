# Task 10 Report: SinaNewsProvider (财经要闻 + 个股新闻 + 历史回溯)

## Status: COMPLETED

## TDD Result
- **Step 1**: 4 tests written (`tests/sina_news_provider_test.rs`)
- **Step 2**: FAIL confirmed (compile error: module `sina_news_provider` 不存在)
- **Step 3**: `src/data_provider/sina_news_provider.rs` 创建 (~155 行)
- **Step 4**: `src/data_provider/mod.rs` 注册 `pub mod sina_news_provider`
- **Step 5**: 4 tests PASS — `test result: ok. 4 passed; 0 failed`
- **Step 6**: Committed as `fe50cf1`

## Implementation

### Public API
```rust
pub const SINA_NEWS_API_BASE: &str = "https://feed.mix.sina.com.cn/api/roll/get";
pub fn build_top_news_url(num: usize) -> String;       // lid=1686, pageid=153
pub fn build_stock_news_url(code: &str, num: usize) -> String; // lid=2516, pageid=155, k=code
pub fn parse_sina_news_body(body: &str, category: &str, code: Option<&str>) -> Result<Vec<NewsItem>>;

pub struct SinaNewsProvider { client: reqwest::Client, api_base: String }
impl SinaNewsProvider {
    pub fn new() -> Self;
    pub async fn fetch_top_news(&self, num: usize) -> Result<Vec<NewsItem>>;
    pub async fn fetch_stock_news(&self, code: &str, num: usize) -> Result<Vec<NewsItem>>;
    pub async fn fetch_stock_news_in_range(
        &self,
        code: &str,
        from: chrono::DateTime<Utc>,
        to: chrono::DateTime<Utc>,
    ) -> Result<Vec<NewsItem>>;
}
```

### Key decisions
- **Refactor**: extracted common HTTP fetch logic into private `fetch_bytes(&self, url)` helper (saves duplication × 3 fetch methods). Same brief behavior, cleaner code.
- **SINA_NEWS_API_BASE visibility**: brief had it as `const` (private) in the struct snippet but also exposed as `pub` in task spec. Used `pub const` so downstream tests can reference it without a builder indirection.
- **Source field**: `sina_financial` (财经要闻, code=None) vs `sina_stock` (个股新闻, code=Some). Matches Task 9 schema.
- **content_hash**: `SHA256(title || summary)` per `news_item::content_hash` (64 hex chars). Verified in test.
- **GBK 容错**: Sina news API mainly returns UTF-8 (different endpoint from K线 GBK), but pass through `GBK.decode()` for safety — UTF-8 is a perfect subset, no corruption.
- **历史回溯**: Sina doesn't support direct date filtering — fetch 5 pages (100 items) and client-side filter `published_at ∈ [from, to]`.

## Test results
```
running 4 tests
test build_stock_news_url_format ... ok
test build_top_news_url_format ... ok
test parse_sina_news_body_extracts_items ... ok
test parse_sina_news_body_with_code ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### Bug fix during TDD
- Initial test file used `fn build_stock_news_url()` (same name as imported function). Rust shadowing caused recursive self-call → E0061 "function takes 2 arguments but 0 supplied". Fixed by renaming test function to `build_stock_news_url_format`.

## Files
- New: `src/data_provider/sina_news_provider.rs` (155 行)
- New: `tests/sina_news_provider_test.rs` (37 行, ⚠️ `git add -f`)
- Modified: `src/data_provider/mod.rs` (+1 line `pub mod sina_news_provider;`)

## Commit
`fe50cf1 feat(news): add SinaNewsProvider (top + stock + history range)`

3 files changed, 220 insertions(+)
