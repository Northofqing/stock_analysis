# Task 2 Report: SinaProvider skeleton + K线 URL + GBK decode

**Date**: 2026-07-08
**Branch**: master
**Commit**: `4bace9b feat(sina): add SinaProvider skeleton + K线 URL + GBK decode`

## TDD Flow (严格按 brief)

### Step 1: Cargo.toml 加 encoding_rs
- 添加 `encoding_rs = "0.8"`

### Step 2: 写 failing test
- `tests/sina_provider_test.rs` (3 tests)
- 用 `git add -f` (B-001: /tests 在 gitignore)

### Step 3: 跑测试 → FAIL
```
error[E0432]: unresolved import `stock_analysis::data_provider::sina_provider`
error: could not compile `stock_analysis` (test "sina_provider_test") due to 2 previous errors
```

### Step 5: 注册模块
- `src/data_provider/mod.rs`: 加 `pub mod sina_provider;`

### Step 4: 实现 SinaProvider
- `src/data_provider/sina_provider.rs` (190 行)
- 包含:
  - `pub fn build_kline_url(code: &str, days: usize) -> String`
  - `pub struct SinaProvider { client: reqwest::Client }`
  - `impl DataProvider for SinaProvider` (K线 OK, name/quote 留待后续)
  - `pub fn parse_kline_body(body: &str, code: &str) -> Result<Vec<KlineData>>` (JSONP `[ ... ]` 提取)
  - `pub struct SinaKlineRow` (day/open/high/low/close/volume)
  - `fn map_kline_row(...)` (string → f64 解析, 缺值 fallback 0.0)
  - `impl Default for SinaProvider`

### Step 6: 跑测试 → PASS
```
running 3 tests
test build_kline_url_format ... ok
test build_kline_url_sz_prefix ... ok
test sina_provider_name ... ok

test result: ok. 3 passed; 0 failed; 0 ignored
```

### cargo build
```
Finished `dev` profile [unoptimized + debuginfo] target(s) in 23.37s
```

## 关键设计选择

1. **`crate::block_on_async` 而非 `Handle::current().block_on`**
   - brief 警告 + gtimg_provider.rs:85-103 已用此 pattern
   - 避免 `current_thread` runtime panic
   - `get_daily_data` sync 入口走此 helper

2. **GBK decode**
   - `encoding_rs::GBK.decode(&bytes)` 返回 `(Cow<str>, encoding, had_errors)`
   - `had_errors=true` 时 `log::warn!` 记录 (best-effort 不阻断)

3. **KlineData 字段补全**
   - `KlineData` struct 在 review 期间扩展 (新增 eps/roe/.../adjust 等)
   - `map_kline_row` 必须填全所有字段, 包含新增的
   - `adjust: AdjustType::None` (Sina 不带复权参数)

4. **`unwrap_or(0.0)` 容错**
   - 字符串数字 parse 失败 → 0.0 (而非 Err)
   - 与 gtimg_provider.rs 行为一致, 避免脏数据中断

5. **`build_kline_url` 接受 raw 6-digit code**
   - 内部调 `to_sina()` 加前缀
   - `600000` → `sh600000`, `000001` → `sz000001`

## 暂未实现 (留给后续 Task)

- `get_stock_name` → `None` (Phase 2: 解析 hq_str 提取 name)
- `get_realtime_quote` → `Ok(None)` (Task 3: 实现 hq_str 解析)

## 文件变更
- `Cargo.toml` (+1 line: encoding_rs)
- `Cargo.lock` (auto)
- `src/data_provider/mod.rs` (+1 line: pub mod sina_provider)
- `src/data_provider/sina_provider.rs` (NEW, 190 lines)
- `tests/sina_provider_test.rs` (NEW, 30 lines, --force)
