wrote /Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-7-brief.md: 85 lines
fallback.rs`
- Create: `tests/fallback_post_close_test.rs`

- [ ] **Step 1: Write failing test**

```rust
// tests/fallback_post_close_test.rs
use stock_analysis::data_provider::fallback::fetch_kline_post_close;

#[tokio::test]
async fn post_close_prefers_baostock() {
    let (data, src) = fetch_kline_post_close("600000", 30).await.unwrap();
    assert!(!data.is_empty());
    println!("post_close src = {src}");
    // 期望 baostock 胜出, 但也可能是 fallthrough 到其它 (网络问题)
    assert!(matches!(src, "baostock" | "sina_hq" | "tencent_qfq" | "eastmoney_qfq" | "rustdx_none"));
}
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cargo test --test fallback_post_close_test
```

Expected: FAIL — `fetch_kline_post_close` not found.

- [ ] **Step 3: Implement fetch_kline_post_close**

```rust
// src/data_provider/fallback.rs (追加)
use crate::data_provider::baostock_provider::BaostockProvider;

/// 盘后专用 K线拉取 (15:00-次日 9:30).
/// 1. Baostock (日终权威, 无限流) 优先
/// 2. fallthrough 到 review #15 4-way join
pub async fn fetch_kline_post_close(
    code: &str,
    days: usize,
) -> Result<(Vec<KlineData>, &'static str)> {
    // 1. Baostock (证券所级别日终数据, 0 风险)
    let baostock = BaostockProvider::new();
    match baostock.get_daily_data(code, days).await {
        Ok(data) if !data.is_empty() => {
            log::info!("[盘后] {code} Baostock 命中, {} 条", data.len());
            return Ok((data, "baostock"));
        }
        Ok(_) => log::debug!("[盘后] {code} Baostock 返回空"),
        Err(e) => log::warn!("[盘后] {code} Baostock 失败: {e}"),
    }
    
    // 2. fallthrough 到 fallback chain
    log::info!("[盘后] {code} Baostock 失败, fallthrough 4-way join");
    fetch_kline_with_fallback(code, days).await
}
```

- [ ] **Step 4: Run test, verify PASS**

```bash
cargo test --test fallback_post_close_test
```

Expected: 1 test passed.

- [ ] **Step 5: Run all tests, verify no regression**

```bash
cargo test --lib
```

Expected: 912+ passed.

- [ ] **Step 6: Commit**

```bash
git add src/data_provider/fallback.rs tests/fallback_post_close_test.rs
git commit -m "feat(baostock): add fetch_kline_post_close (盘后专用, Baostock priority)"
```

---

