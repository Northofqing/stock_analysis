wrote /Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-11-brief.md: 63 lines
main.rs`

- [ ] **Step 1: Add poll_news_loop function**

```rust
// src/bin/monitor/main.rs

async fn poll_news_loop() {
    use stock_analysis::data_provider::sina_news_provider::SinaNewsProvider;
    use stock_analysis::database::DatabaseManager;
    
    let provider = SinaNewsProvider::new();
    let mut interval = tokio::time::interval(Duration::from_secs(90));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    
    loop {
        interval.tick().await;
        match provider.fetch_top_news(20).await {
            Ok(items) => {
                let mut db_count = 0;
                for item in &items {
                    // 双写
                    if let Some(db) = DatabaseManager::try_get() {
                        if db.insert_news_item(item).is_ok() {
                            db_count += 1;
                        }
                    }
                }
                log::info!("[新闻] Sina 拉取 {} 条, DB 写 {} 条", items.len(), db_count);
            }
            Err(e) => log::warn!("[新闻] Sina 拉取失败: {e}"),
        }
    }
}
```

- [ ] **Step 2: Spawn poll_news_loop in main**

```rust
// src/bin/monitor/main.rs (在 main 启动其他 task 时)
tokio::spawn(poll_news_loop());
```

- [ ] **Step 3: Run cargo build, verify compiles**

```bash
cargo build
```

Expected: pass.

- [ ] **Step 4: Commit**

```bash
git add src/bin/monitor/main.rs
git commit -m "feat(news): add poll_news_loop (Sina 财经要闻, 90s interval, 双写 DB)"
```

---

