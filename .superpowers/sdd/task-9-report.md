# Task 9 Report: NewsItem struct + news_items migration

**Date**: 2026-07-09
**Status**: ✅ Completed (4 commits → 1 squashed)
**Branch**: master
**Commit**: `902f704 feat(news): add NewsItem struct + news_items table + insert helper`

---

## Files changed

| File | Change | Lines |
|------|--------|-------|
| `src/data_provider/news_item.rs` | NEW | +55 |
| `src/data_provider/mod.rs` | MOD (register module) | +2 |
| `src/database/mod.rs` | MOD (NEWS_ITEMS_SCHEMA const + execute + insert helper) | +52 |
| `tests/news_item_test.rs` | NEW (TDD, git add -f) | +44 |

Total: 152 insertions.

---

## Interfaces produced

### `NewsItem` struct (`src/data_provider/news_item.rs`)
```rust
pub struct NewsItem {
    pub source: String,         // "sina_financial" | "sina_stock"
    pub external_id: String,    // url as ID for now
    pub category: String,       // "财经要闻" | "个股新闻"
    pub code: Option<String>,   // 6-digit code (个股新闻 only)
    pub title: String,
    pub summary: String,
    pub url: String,
    pub source_name: String,
    pub published_at: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
    pub content_hash: String,   // SHA256 hex of title+summary
}
```

### `content_hash` helper
```rust
pub fn content_hash(title: &str, summary: &str) -> String
// SHA256 hex (64 字符), 字节拼接 (无 separator).
```

### `news_items` table (idempotent CREATE TABLE IF NOT EXISTS)
```sql
CREATE TABLE news_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    category TEXT NOT NULL,
    code TEXT,                         -- nullable for 财经要闻
    title TEXT NOT NULL,
    summary TEXT,                      -- nullable in schema (实际都填)
    url TEXT NOT NULL,
    source_name TEXT,                  -- nullable
    published_at INTEGER NOT NULL,
    fetched_at INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    UNIQUE(source, external_id)        -- dedup key
);
CREATE INDEX idx_news_items_code_time ON news_items(code, published_at);
CREATE INDEX idx_news_items_published ON news_items(published_at);
```

### `DatabaseManager::insert_news_item`
```rust
pub fn insert_news_item(&self, item: &NewsItem) -> Result<(), String>
// INSERT OR IGNORE → 同 (source, external_id) 已存在静默跳过 (UNIQUE 约束).
// code: None 时写空串.
// 时间戳: unix seconds (i64) → BigInt bind.
```

---

## TDD 流程 (3 tests)

1. **Step 3** 写 failing tests:
   - `content_hash_deterministic` — 同输入同 hash, 64 字符
   - `content_hash_differs_for_diff_input` — 改 summary hash 变
   - `news_item_serializes` — serde_json 含 "sina_financial"

2. **Step 5** 跑测试 → **FAIL** (module `news_item` 不存在, E0599; const `NEWS_ITEMS_SCHEMA` 不在 scope, E0425).

3. **Step 6** 注册 `pub mod news_item;` → 修 `Self::NEWS_ITEMS_SCHEMA` 引用 (impl 内 associated const 必须 Self:: 显式)。

4. **Step 7** 跑测试 → **PASS** (3/3 ok, finished in 0.00s).

5. **Step 8** 加 `insert_news_item` (impl DatabaseManager) → 编译 + 测试仍 PASS.

---

## 实施偏差记录

### 1. `batch_execute` → `diesel::sql_query().execute()`
Brief 假设用 `conn.batch_execute(NEWS_ITEMS_SCHEMA)`, 但 `diesel::SqliteConnection` 不暴露
`batch_execute` (那是 `diesel::Connection` trait method, `r2d2::PooledConnection` 上不一定有).
改用项目内已用一致的模式: `diesel::sql_query(NEWS_ITEMS_SCHEMA).execute(&mut *conn)?`,
SQLite 一次连接支持 multi-statement 字符串, 行为一致.

### 2. `NEWS_ITEMS_SCHEMA` 引用需 `Self::` 前缀
Brief 把 const 放在 `impl DatabaseManager` 内 (`pub const ...`), 我先放到 module 顶层.
`run_migrations` 也在 `impl` 内, 顶层 const 不可见 → E0425.
移回 impl 内后, Rust 仍要求 `Self::NEWS_ITEMS_SCHEMA` (impl 内 item 引用 item 必须 Self::
显式). 编译错误信息直接提示此修复.

### 3. `code` 字段 nullable 处理
Brief 写 `bind::<Text, _>(item.code.as_deref().unwrap_or(""))` — 实际是 OK 的, 但下游查
询时 `code = ''` 与 `code IS NULL` 是两个状态. 当前 schema 列允许 NULL, 业务代码统一
写空串, 后续读取需注意 (默认按 `code != ''` 过滤个股新闻).

---

## 验证

| 检查项 | 结果 |
|--------|------|
| `cargo test --test news_item_test` | ✅ 3/3 passed |
| `cargo build --lib` | ✅ 0 errors (87 warnings 全是 pre-existing dead code) |
| `cargo test --lib test_database_init` | ✅ ok (DB 初始化后 `news_items` 表实际创建) |
| `sqlite3 .schema news_items` | ✅ schema + 2 indexes 全部正确 |
| `cargo test --lib` 全量 | ⚠️ 924 pass / 1 fail (`test_backfill_st_type_prefix_anchored` 预存 flake, 见下) |

### Pre-existing flake 调查
`test_backfill_st_type_prefix_anchored` 在并行 `cargo test --lib` 时偶发 fail (race with
`DatabaseManager::OnceCell` 全局单例 — `init(Some(path))` 第一次成功, 后续同 path 走
"数据库已经初始化" 错误). 该 test 单独跑 (`cargo test --lib test_backfill_st_type_prefix_anchored`)
100% 通过. **与本 task 改动无关**, 是 v14.1 commit `2fb123a` 引入的已知 flake.
修复需用 `serial_test` 或重构 `DatabaseManager` 单例 (超出本 task 范围, 不动).

### Schema 在 DB 中实际创建
```sql
sqlite> .schema news_items
CREATE TABLE news_items (
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
```

---

## 与其他 task 的边界

- **不实现** fetch 逻辑 (sina_financial / sina_stock RSS 抓取) — 后续 task.
- **不写** insert_news_item 的集成测试 (DB round-trip) — 超出 brief 范围, 后续 task 接入 fetch 时一并加.
- **不动** `news_dedup` 表 (5min 滑窗, 已存在) — 与 news_items 互补不冲突.
- **不动** docs/ (Task 9 不动 docs).

---

## Commit

```
902f704 feat(news): add NewsItem struct + news_items table + insert helper
4 files changed, 152 insertions(+)
create mode 100644 src/data_provider/news_item.rs
create mode 100644 tests/news_item_test.rs
```

(`tests/` 在 `.gitignore` → `git add -f tests/news_item_test.rs` 已用.)