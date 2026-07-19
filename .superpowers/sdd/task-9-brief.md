wrote /Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-9-brief.md: 183 lines
ata_provider/news_item.rs`
- Modify: `src/data_provider/mod.rs` (注册模块)
- Modify: `src/database/mod.rs` (添加 news_items schema + insert_news_item helper)
- Create: `tests/news_item_test.rs`

**Interfaces:**
- Produces:
  - `pub struct NewsItem { source, external_id, category, code, title, summary, url, source_name, published_at, fetched_at, content_hash }`
  - `pub fn content_hash(title: &str, summary: &str) -> String` (SHA256 hex)
  - `pub fn insert_news_item(item: &NewsItem) -> Result<()>` on `DatabaseManager`

- [ ] **Step 1: Add news_items table migration**

```rust
// src/database/mod.rs (在现有 schema 段)
// news_items: 详存新闻 (与 news_dedup 互补, 5min 去重 vs 永久详存)
pub const NEWS_ITEMS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS news_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    category TEXT NOT NULL,
    code TEXT,
    title TEXT NOT NULL,
    summary TEXT,
    url TEXT NOT NULL,
    source_name TEXT,
    published_at INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    UNIQUE(source, external_id)
);
CREATE INDEX IF NOT EXISTS idx_news_items_code_time ON news_items(code, published_at);
CREATE INDEX IF NOT EXISTS idx_news_items_published ON news_items(published_at);
"#;
```

在 `init` 函數末尾追加執行 `NEWS_ITEMS_SCHEMA`.

- [ ] **Step 2: Add NewsItem struct + content_hash**

```rust
// src/data_provider/news_item.rs
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NewsItem {
    pub source: String,         // "sina_financial" | "sina_stock"
    pub external_id: String,    // url 作为 id
    pub category: String,       // "财经要闻" | "个股新闻"
    pub code: Option<String>,   // 6 位 code (仅个股新闻)
    pub title: String,
    pub summary: String,
    pub url: String,
    pub source_name: String,
    pub published_at: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
    pub content_hash: String,
}

/// SHA256 hex of (title + summary) — 用于 dedup.
pub fn content_hash(title: &str, summary: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(summary.as_bytes());
    format!("{:x}", hasher.finalize())
}
```

- [ ] **Step 3: Write failing tests**

```rust
// tests/news_item_test.rs
use stock_analysis::data_provider::news_item::{content_hash, NewsItem};
use chrono::Utc;

#[test]
fn content_hash_deterministic() {
    let h1 = content_hash("title1", "summary1");
    let h2 = content_hash("title1", "summary1");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64);  // SHA256 hex
}

#[test]
fn content_hash_differs_for_diff_input() {
    let h1 = content_hash("title1", "summary1");
    let h2 = content_hash("title1", "summary2");
    assert_ne!(h1, h2);
}

#[test]
fn news_item_serializes() {
    let item = NewsItem {
        source: "sina_financial".into(),
        external_id: "https://example.com/1".into(),
        category: "财经要闻".into(),
        code: None,
        title: "Test".into(),
        summary: "Summary".into(),
        url: "https://example.com/1".into(),
        source_name: "新浪财经".into(),
        published_at: Utc::now(),
        fetched_at: Utc::now(),
        content_hash: content_hash("Test", "Summary"),
    };
    let json = serde_json::to_string(&item).unwrap();
    assert!(json.contains("sina_financial"));
}
```

- [ ] **Step 4: Add sha2 dep**

```toml
# Cargo.toml
sha2 = "0.10"  # 已有 (review #14)
```

- [ ] **Step 5: Run tests, verify FAIL**

```bash
cargo test --test news_item_test
```

Expected: FAIL — `news_item` module 不存在.

- [ ] **Step 6: Register module**

```rust
// src/data_provider/mod.rs
pub mod news_item;
```

- [ ] **Step 7: Run tests, verify PASS**

```bash
cargo test --test news_item_test
```

Expected: 3 tests passed.

- [ ] **Step 8: Add insert_news_item helper to DatabaseManager**

```rust
// src/database/mod.rs (impl DatabaseManager)
pub fn insert_news_item(&self, item: &crate::data_provider::news_item::NewsItem) -> Result<(), String> {
    use crate::data_provider::news_item::NewsItem;
    use diesel::prelude::*;
    let mut conn = self.get_conn().map_err(|e| e.to_string())?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO news_items (source, external_id, category, code, title, summary, url, source_name, published_at, fetched_at, content_hash) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
    )
    .bind::<Text, _>(&item.source)
    .bind::<Text, _>(&item.external_id)
    .bind::<Text, _>(&item.category)
    .bind::<Text, _>(item.code.as_deref().unwrap_or(""))
    .bind::<Text, _>(&item.title)
    .bind::<Text, _>(&item.summary)
    .bind::<Text, _>(&item.url)
    .bind::<Text, _>(&item.source_name)
    .bind::<BigInt, _>(item.published_at.timestamp())
    .bind::<BigInt, _>(item.fetched_at.timestamp())
    .bind::<Text, _>(&item.content_hash)
    .execute(&mut *conn)
    .map_err(|e| e.to_string())?;
    Ok(())
}
```

- [ ] **Step 9: Commit**

```bash
git add src/data_provider/news_item.rs src/data_provider/mod.rs src/database/mod.rs tests/news_item_test.rs
git commit -m "feat(news): add NewsItem struct + news_items table + insert helper"
```

---

