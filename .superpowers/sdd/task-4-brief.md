wrote /Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-4-brief.md: 99 lines
ta_provider/fallback.rs`
- Modify: `tests/fallback_sina_test.rs` (新文件)

- [ ] **Step 1: Write failing integration test**

```rust
// tests/fallback_sina_test.rs
use stock_analysis::data_provider::fallback::fetch_kline_with_fallback;

#[tokio::test]
async fn fallback_returns_data_with_sina_in_chain() {
    // Sina/腾讯/东财/RustDX 中任一成功即可
    let (data, src) = fetch_kline_with_fallback("600000", 5).await.unwrap();
    assert!(!data.is_empty(), "所有 4 源都不该失败");
    // src 可能是 sina_hq 或其它 (看哪个最快)
    assert!(matches!(src, "sina_hq" | "tencent_qfq" | "eastmoney_qfq" | "rustdx_none"));
}
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cargo test --test fallback_sina_test
```

Expected: FAIL — `fetch_kline_with_fallback` 当前不支持 Sina (只能从 3 源选).

- [ ] **Step 3: Modify fallback.rs (加 Sina 进竞速链)**

```rust
// src/data_provider/fallback.rs (修改)
use crate::data_provider::sina_provider::SinaProvider;

// 1. 加 SourceResult::Sina
enum SourceResult {
    Sina(Result<Vec<KlineData>>),       // NEW
    Tencent(Result<Vec<KlineData>>),
    Eastmoney(Result<Vec<KlineData>>),
    Rustdx(Result<Vec<KlineData>>),
}

// 2. 在 fetch_kline_with_fallback 内加 sina_fut
pub async fn fetch_kline_with_fallback(
    code: &str,
    days: usize,
) -> Result<(Vec<KlineData>, &'static str)> {
    let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
    let qc_threshold = max_gap_for(code);
    
    // NEW: Sina (priority 1)
    let sina_fut = {
        let code = code.to_string();
        async move {
            let r = SinaProvider::new().fetch_kline_raw(&code, days).await;
            SourceResult::Sina(r)
        }
    };
    // ... tencent_fut / eastmoney_fut / rustdx_fut (现有)
    
    let (s, t, e, r) = tokio::join!(sina_fut, tencent_fut, eastmoney_fut, rustdx_fut);
    
    let candidates: [(SourceResult, &'static str); 4] = [
        (s, "sina_hq"),        // NEW priority 1
        (t, "tencent_qfq"),
        (e, "eastmoney_qfq"),
        (r, "rustdx_none"),
    ];
    // ... 后续循环不变
}
```

- [ ] **Step 4: Run test, verify PASS**

```bash
cargo test --test fallback_sina_test
```

Expected: 1 test passed.

- [ ] **Step 5: Run all tests, verify no regression**

```bash
cargo test --lib
```

Expected: 908+ passed.

- [ ] **Step 6: Commit**

```bash
git add src/data_provider/fallback.rs tests/fallback_sina_test.rs
git commit -m "feat(sina): integrate SinaProvider as fallback priority 1 (4-way join)"
```

---

