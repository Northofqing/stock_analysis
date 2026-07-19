wrote /Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-12-brief.md: 112 lines
/main.rs` (盘后回溯调用)
- Modify: `docs/business_rules.md`
- Modify: `docs/sina_baostock_integration.md` (加 Phase 2 段)

- [ ] **Step 1: Add post_close_news_review**

```rust
// src/bin/monitor/main.rs

async fn post_close_news_review() {
    use stock_analysis::data_provider::sina_news_provider::SinaNewsProvider;
    use stock_analysis::database::DatabaseManager;
    use chrono::{Duration, Utc};
    
    let now = Utc::now();
    let from = now - Duration::days(30);
    let provider = SinaNewsProvider::new();
    let holdings: Vec<String> = stock_analysis::portfolio::get_positions()
        .unwrap_or_default()
        .iter()
        .map(|p| p.code.clone())
        .collect();
    
    log::info!("[盘后] 拉 {} 只持仓近 30 天个股新闻", holdings.len());
    for code in &holdings {
        match provider.fetch_stock_news_in_range(code, from, now).await {
            Ok(items) => {
                let mut db_count = 0;
                if let Some(db) = DatabaseManager::try_get() {
                    for item in &items {
                        if db.insert_news_item(item).is_ok() {
                            db_count += 1;
                        }
                    }
                }
                log::info!("[盘后] {code} Sina 个股新闻: {} 条, DB 写 {} 条", items.len(), db_count);
            }
            Err(e) => log::warn!("[盘后] {code} Sina 拉取失败: {e}"),
        }
    }
}
```

- [ ] **Step 2: Spawn post_close_news_review in main (盘后时段)**

```rust
// src/bin/monitor/main.rs (在 post_close_review 旁边)
tokio::spawn(async {
    // 等待到盘后 15:30 再启动
    // 简化: 每 30 分钟检查一次
    let mut interval = tokio::time::interval(Duration::from_secs(1800));
    loop {
        interval.tick().await;
        let now = chrono::Local::now();
        if now.time() >= chrono::NaiveTime::from_hms_opt(15, 30, 0).unwrap() {
            post_close_news_review().await;
        }
    }
});
```

- [ ] **Step 3: Update startup log**

```rust
// src/bin/monitor/main.rs (在 startup log 段)
log::info!("[启动] 新闻轮询: Sina 财经要闻 (90s 间隔, 双写 news_dedup + news_items)");
log::info!("[启动] 盘后回溯: Sina 个股新闻 (15:30 后, 30 天)");
```

- [ ] **Step 4: Add BR-016**

```markdown
| BR-016 | ✅ registered | Sina 新闻 API (feed.mix.sina.com.cn) — 实时轮询财经要闻 (90s) + 盘后回溯个股新闻 (15:30), 双写 news_dedup (5min 去重) + news_items (详存, 新表) | `src/data_provider/sina_news_provider.rs`, `src/data_provider/news_item.rs`, `src/database/mod.rs` |
```

- [ ] **Step 5: Update docs/sina_baostock_integration.md (加 Phase 2 段)**

```markdown
## Phase 2: Sina 新闻数据集成 (2026-07-08)

[摘要 spec §P2 内容]
```

- [ ] **Step 6: Run all tests**

```bash
cargo test --lib
```

Expected: 922+ passed.

- [ ] **Step 7: Commit**

```bash
git add src/bin/monitor/main.rs docs/business_rules.md docs/sina_baostock_integration.md
git commit -m "feat(news): add post_close_news_review + BR-016 + Phase 2 docs"
```

---

## Final verification (Phase 1 + Phase 2)

```bash
cargo build --release
cargo test --lib
cargo clippy -- -D warnings
```

All must pass. Then push to master.
