# Task 11 Report — poll_news_loop

## Status
DONE (commit `d9b082f`)

## Changes
File: `src/bin/monitor/main.rs` (only file touched, +46 lines, 0 deletions)

### Step 1 — Added `poll_news_loop()` async fn
Inserted between `push_market_fund_top10` (line 5138) and `push` (was 5144, now 5190).

Key design:
- Independent of `news_monitor_loop` (only persists to DB, no signal/AI analysis)
- Uses `SinaNewsProvider::fetch_top_news(20)` (already provided by Task 10)
- Uses `DatabaseManager::with_db("poll_news", |db| ...)` (review #15 helper)
  - Avoids 13+ places of `let Some(db) = DatabaseManager::try_get() else { return; };` boilerplate
  - Handles DB uninitialized state with one-shot warn (no per-tick spam)
- `tokio::time::interval(90s)` + `MissedTickBehavior::Skip` (project convention)
- Logs `[新闻] Sina 拉取 N 条, DB 写 M 条` per cycle
- Failure logs `[新闻] Sina 拉取失败: <err>` (warn, not error)

### Step 2 — Spawned in main
Added `tokio::spawn(poll_news_loop());` after `main_loops` block (line ~1109).

Rationale: `tokio::spawn` (detached task) chosen over `tokio::join!` so `poll_news_loop`
lifetime is decoupled from `monitor_loop`/`news_monitor_loop`. If either of those
returns early (e.g., main_loops select! resolves), poll_news continues independently.

### Step 3 — cargo build
PASS. `Finished dev profile [unoptimized + debuginfo] target(s) in 10.77s`.
92 pre-existing warnings (unrelated to this change, mostly `push_templates.rs` dead code).

### Step 4 — Commit
```
d9b082f feat(news): add poll_news_loop (Sina 财经要闻, 90s interval, 双写 DB)
 1 file changed, 46 insertions(+)
```

## Key implementation choices
1. **Used `DatabaseManager::with_db` over `try_get` + unwrap**: follows review #15 helper
   which already encapsulates the unwrap-or-warn-once pattern.
2. **90s interval hardcoded**: brief specified this; matches Sina top_news refresh cadence.
   (No env override; `news_monitor_loop` already has `NEWS_POLL_INTERVAL` env knob — kept
   the two loops independent by design.)
3. **No test file added**: brief explicitly noted `/tests` in `.gitignore`; loop body is
   glue between `SinaNewsProvider::fetch_top_news` (already covered by Task 10) and
   `DatabaseManager::insert_news_item` (already covered by Task 9).
4. **Used `Some(())` / `None` as closure return**: required because `with_db` closure
   signature is `FnOnce(&DatabaseManager) -> Option<T>` and we want to differentiate
   "insert OK" vs "insert failed".

## Code snippet
```rust
/// v13.11 (Task 11): 独立轮询 Sina 财经要闻, 每 90s 拉一次 top 20,
/// 双写 DatabaseManager::news_items 表 (供后续链路/回溯使用).
/// 与 news_monitor_loop 解耦: 本 loop 不做信号/AI 分析, 仅入库.
async fn poll_news_loop() {
    use stock_analysis::data_provider::sina_news_provider::SinaNewsProvider;
    use stock_analysis::database::DatabaseManager;
    use std::time::Duration;

    let provider = SinaNewsProvider::new();
    let mut interval = tokio::time::interval(Duration::from_secs(90));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    log::info!("[PollNews] 启动 (Sina 财经要闻, 90s 间隔, 双写 DB)");

    loop {
        interval.tick().await;
        match provider.fetch_top_news(20).await {
            Ok(items) => {
                let count = items.len();
                let mut written = 0usize;
                for item in &items {
                    let ok = DatabaseManager::with_db("poll_news", |db| {
                        if db.insert_news_item(item).is_ok() { Some(()) } else { None }
                    });
                    if ok.is_some() { written += 1; }
                }
                log::info!("[新闻] Sina 拉取 {} 条, DB 写 {} 条", count, written);
            }
            Err(e) => log::warn!("[新闻] Sina 拉取失败: {e}"),
        }
    }
}
```

## Open follow-up (not in Task 11 scope)
- Decoupling note: `poll_news_loop` and `news_monitor_loop` now both pull Sina top news.
  Consider letting `news_monitor_loop` read from DB (news_items table) instead of pulling
  from network itself, to avoid double-fetch. Defer to Task 12+ (Phase 2 review).