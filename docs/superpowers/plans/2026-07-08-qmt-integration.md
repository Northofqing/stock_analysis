# QMT 本地缓存集成 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 MiniQMT 本地缓存 (`qmt-parser` 0.2.1) 接入 `fallback.rs` 竞速链作为 priority 0, 解决 review #14/15 中 HTTP K线源被 ban 时的零网络 fallback.

**Architecture:** 实现 `QmtProvider` 满足现有 `DataProvider` trait (与 `GtimgProvider`/`HttpProvider`/`RustdxProvider` 平级), 在 `fetch_kline_with_fallback` 中用 `tokio::join!` 与三源 HTTP 竞速, 第一个 Ok+质检+未 stale 胜出. Stock code 格式映射 ("000001" ↔ "000001.SZ") 抽到独立 helper.

**Tech Stack:** Rust + tokio + qmt-parser 0.2.1 (GPL-3.0) + 现有 DataProvider trait.

## Global Constraints

- qmt-parser 版本必须 pin: `qmt-parser = "=0.2.1"` (exact version, 不接受 minor/patch 改动)
- 严禁 GPL-3.0 相关的 license 误标: `Cargo.toml` 注释明确说明依赖 GPL 库
- 所有新增代码必须 `cargo clippy -- -D warnings` 干净通过
- Stale 数据 = Err 走 fallthrough, 严禁静默使用
- 每次 commit 前必须 `cargo build` 通过

---

### Task 1: stock_code_map helper 模块 + 单元测试

**Files:**
- Create: `src/data_provider/stock_code_map.rs`
- Modify: `src/data_provider/mod.rs` (添加 `pub mod stock_code_map;`)
- Create: `tests/stock_code_map_test.rs`

**Interfaces:**
- Consumes: 无 (全新模块)
- Produces:
  - `pub fn to_qmt_symbol(code: &str) -> String`
  - `pub fn from_qmt_symbol(qmt_code: &str) -> String`
  - `pub fn market_of(code: &str) -> qmt_parser::Market`

- [ ] **Step 1: Write failing tests**

```rust
// tests/stock_code_map_test.rs
use stock_analysis::data_provider::stock_code_map::{to_qmt_symbol, from_qmt_symbol, market_of};

#[test]
fn to_qmt_symbol_sz_main_board() {
    assert_eq!(to_qmt_symbol("000001"), "000001.SZ");
    assert_eq!(to_qmt_symbol("301000"), "301000.SZ");  // 创业板
    assert_eq!(to_qmt_symbol("002415"), "002415.SZ");  // 中小板
}

#[test]
fn to_qmt_symbol_sh_main_board() {
    assert_eq!(to_qmt_symbol("600000"), "600000.SH");
    assert_eq!(to_qmt_symbol("688001"), "688001.SH");  // 科创板
    assert_eq!(to_qmt_symbol("900900"), "900900.SH");  // B 股
}

#[test]
fn from_qmt_symbol_strips_suffix() {
    assert_eq!(from_qmt_symbol("000001.SZ"), "000001");
    assert_eq!(from_qmt_symbol("600000.SH"), "600000");
    assert_eq!(from_qmt_symbol("999999"), "999999");  // 无后缀也 OK
}

#[test]
fn market_of_six_prefix_is_sh() {
    use qmt_parser::Market;
    assert!(matches!(market_of("600000"), Market::Sh));
    assert!(matches!(market_of("688001"), Market::Sh));
    assert!(matches!(market_of("900900"), Market::Sh));
}

#[test]
fn market_of_other_prefix_is_sz() {
    use qmt_parser::Market;
    assert!(matches!(market_of("000001"), Market::Sz));
    assert!(matches!(market_of("002415"), Market::Sz));
    assert!(matches!(market_of("300750"), Market::Sz));
}
```

- [ ] **Step 2: Run tests, verify FAIL**

```bash
cargo test --test stock_code_map_test
```

Expected: FAIL — `stock_code_map` module 不存在.

- [ ] **Step 3: Add Cargo.toml dep**

```toml
# Cargo.toml, [dependencies]
# qmt-parser 是 GPL-3.0, 项目自用 (不分发) — OK
qmt-parser = "=0.2.1"
```

- [ ] **Step 4: Implement helper module**

```rust
// src/data_provider/stock_code_map.rs
use qmt_parser::Market;

/// 我们的 6 位 code → QMT 的 `code.market` 格式.
/// 6/9/5 开头 = 沪市 (主板/科创板/B 股), 其它 = 深市.
pub fn to_qmt_symbol(code: &str) -> String {
    let suffix = match code.chars().next() {
        Some('6') | Some('9') | Some('5') => ".SH",
        _ => ".SZ",
    };
    format!("{}{}", code, suffix)
}

/// QMT 的 `code.market` → 我们的 6 位 code.
pub fn from_qmt_symbol(qmt_code: &str) -> String {
    qmt_code.split('.').next().unwrap_or(qmt_code).to_string()
}

/// 我们的 code → QMT Market enum.
pub fn market_of(code: &str) -> Market {
    match code.chars().next() {
        Some('6') | Some('9') | Some('5') => Market::Sh,
        _ => Market::Sz,
    }
}
```

- [ ] **Step 5: Register module**

```rust
// src/data_provider/mod.rs, 在现有 `pub mod eastmoney_provider;` 附近
pub mod stock_code_map;
```

- [ ] **Step 6: Run tests, verify PASS**

```bash
cargo test --test stock_code_map_test
```

Expected: 5 tests passed.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/data_provider/stock_code_map.rs src/data_provider/mod.rs tests/stock_code_map_test.rs
git commit -m "feat(qmt): add stock_code_map helper for QMT symbol format"
```

---

### Task 2: staleness check helper

**Files:**
- Create: `src/data_provider/staleness.rs`
- Modify: `src/data_provider/mod.rs` (添加 `pub mod staleness;`)
- Create: `tests/staleness_test.rs`

**Interfaces:**
- Consumes: `&[qmt_parser::DailyKlineData]`
- Produces: `pub fn check_staleness(code: &str, raw: &[DailyKlineData]) -> Result<(), anyhow::Error>`

- [ ] **Step 1: Write failing tests**

```rust
// tests/staleness_test.rs
use chrono::{Duration, Utc};
use qmt_parser::DailyKlineData;
use stock_analysis::data_provider::staleness::check_staleness;

fn daily_at(days_ago: i64) -> DailyKlineData {
    let ts = (Utc::now() - Duration::days(days_ago)).timestamp_millis_opt().unwrap();
    DailyKlineData {
        timestamp_ms: ts,
        open: 10.0, high: 11.0, low: 9.5, close: 10.0,
        volume: 1000, amount: 10000.0, open_interest: 0,
        file_pre_close: 10.0,
    }
}

#[test]
fn accepts_today() {
    let daily = daily_at(0);
    assert!(check_staleness("000001", &[daily]).is_ok());
}

#[test]
fn accepts_yesterday() {
    let daily = daily_at(1);
    assert!(check_staleness("000001", &[daily]).is_ok());
}

#[test]
fn rejects_three_days_ago() {
    let daily = daily_at(3);
    let result = check_staleness("000001", &[daily]);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("stale"), "expected 'stale' in err, got: {err}");
}

#[test]
fn empty_data_does_not_flag_stale() {
    // 空数据应该让上层 fallthrough (Ok(empty) → 质检 reject), 不是 stale.
    assert!(check_staleness("000001", &[]).is_ok());
}
```

- [ ] **Step 2: Run tests, verify FAIL**

```bash
cargo test --test staleness_test
```

Expected: FAIL — `staleness` module 不存在.

- [ ] **Step 3: Implement staleness module**

```rust
// src/data_provider/staleness.rs
use anyhow::{anyhow, Result};
use chrono::Local;
use qmt_parser::DailyKlineData;

/// 检查 QMT 缓存数据是否 stale.
/// 超过 1 trading day (即 stale ≥ 2 自然日 且 今天不是交易日) → 返 Err.
/// 调用方 (fallback.rs) 收到 Err 自然 fallthrough HTTP.
///
/// 空数组返 Ok(()) — 让上层质检 reject 走 fallthrough, 不在此层提前 block.
pub fn check_staleness(code: &str, raw: &[DailyKlineData]) -> Result<()> {
    let Some(latest) = raw.last() else { return Ok(()); };
    let latest_date = chrono::DateTime::from_timestamp_millis(latest.timestamp_ms)
        .ok_or_else(|| anyhow!("invalid timestamp_ms: {}", latest.timestamp_ms))?
        .with_timezone(&Local)
        .date_naive();
    let today = Local::now().date_naive();
    let staleness_days = (today - latest_date).num_days();
    
    // 阈值: 1 trading day = 大约 2 自然日 (考虑周末).
    // 若今天或昨天是交易日, 数据是今天/昨天 → 接受.
    // 否则视为 stale (数据太老).
    let yesterday = today - chrono::Duration::days(1);
    let trading_recent = crate::calendar::is_trading_day(today) 
        || crate::calendar::is_trading_day(yesterday);
    
    if staleness_days > 1 && !trading_recent {
        log::warn!(
            "QMT {code} 数据滞后 {n} 天 (最新 {date}), fallthrough HTTP",
            code = code, n = staleness_days, date = latest_date
        );
        return Err(anyhow!("stale: latest={latest_date}, today={today}"));
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests, verify PASS**

```bash
cargo test --test staleness_test
```

Expected: 4 tests passed.

- [ ] **Step 5: Commit**

```bash
git add src/data_provider/staleness.rs src/data_provider/mod.rs tests/staleness_test.rs
git commit -m "feat(qmt): add staleness check helper (>1 trading day → Err)"
```

---

### Task 3: QmtProvider skeleton + auto_detect

**Files:**
- Create: `src/data_provider/qmt_provider.rs`
- Modify: `src/data_provider/mod.rs` (添加 `pub mod qmt_provider;`)
- Create: `tests/qmt_provider_test.rs`

**Interfaces:**
- Consumes: `qmt_parser::QmtDataDir`
- Produces:
  - `pub struct QmtProvider { dir: QmtDataDir }`
  - `impl QmtProvider { pub fn auto_detect() -> Option<Self> }`
  - `impl DataProvider for QmtProvider` (skeleton — get_daily_data placeholder)

- [ ] **Step 1: Write failing test for auto_detect**

```rust
// tests/qmt_provider_test.rs
use std::env;

#[test]
fn auto_detect_returns_none_without_env() {
    env::remove_var("QMT_DATA_DIR");
    // 路径探测: 候选路径若都不存在 → None
    let result = stock_analysis::data_provider::qmt_provider::QmtProvider::auto_detect();
    // 不强制 None (依赖环境), 但必须不 panic
    let _ = result;
}

#[test]
fn auto_detect_with_env_var() {
    let tmp = tempdir::TempDir::new("qmt_test").unwrap();
    let datadir = tmp.path().join("datadir");
    std::fs::create_dir_all(&datadir).unwrap();
    // qmt-parser 需要 datadir 存在但内容可空 (0.2.1 行为: 空目录可解析为 0 数据)
    env::set_var("QMT_DATA_DIR", datadir.parent().unwrap());
    let result = stock_analysis::data_provider::qmt_provider::QmtProvider::auto_detect();
    env::remove_var("QMT_DATA_DIR");
    assert!(result.is_some(), "QMT_DATA_DIR set but auto_detect returned None");
}
```

Add to `Cargo.toml [dev-dependencies]`:
```toml
tempdir = "0.3"
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cargo test --test qmt_provider_test
```

Expected: FAIL — `qmt_provider` module 不存在.

- [ ] **Step 3: Implement QmtProvider skeleton**

```rust
// src/data_provider/qmt_provider.rs
use std::path::PathBuf;
use anyhow::Result;
use qmt_parser::{QmtDataDir, Market};
use crate::data_provider::stock_code_map::{to_qmt_symbol, market_of};
use super::{DataProvider, KlineData, RealtimeQuote};

const QMT_AUTO_DETECT_PATHS: &[&str] = &[
    "~/qmtdata",
    "~/.qmtdata",
    "/mnt/data/trade/qmtdata",
    "./qmtdata",
];

pub struct QmtProvider {
    dir: QmtDataDir,
}

impl QmtProvider {
    /// auto-detect: env var QMT_DATA_DIR 覆盖, 否则按候选路径探测.
    /// 候选路径存在 + 可作为 QmtDataDir 打开 → 返 Some.
    /// 都没找到 → 返 None (上层 fallback 跳过 QMT).
    pub fn auto_detect() -> Option<Self> {
        let path: PathBuf = std::env::var("QMT_DATA_DIR").ok()
            .map(PathBuf::from)
            .or_else(|| {
                QMT_AUTO_DETECT_PATHS.iter()
                    .map(|p| shellexpand(p).into_owned())
                    .map(PathBuf::from)
                    .find(|p| p.join("datadir").exists())
            })?;
        
        let datadir = path.join("datadir");
        if !datadir.exists() {
            log::info!("QMT 路径 {datadir:?} 存在但无 datadir 子目录, 跳过");
            return None;
        }
        
        match QmtDataDir::new(datadir.to_str()?) {
            Ok(dir) => {
                log::info!("QMT 启用: 路径={datadir:?}");
                Some(Self { dir })
            }
            Err(e) => {
                log::warn!("QMT 路径 {datadir:?} 打开失败: {e}");
                None
            }
        }
    }
}

impl DataProvider for QmtProvider {
    fn name(&self) -> &'static str { "qmt_local" }
    
    fn get_daily_data(&self, _code: &str, _days: usize) -> Result<Vec<KlineData>> {
        // Task 4 实现
        Err(anyhow::anyhow!("not yet implemented"))
    }
    
    fn get_stock_name(&self, _code: &str) -> Option<String> {
        // Task 5 实现
        None
    }
    
    fn get_realtime_quote(&self, _code: &str) -> Result<Option<RealtimeQuote>> {
        Ok(None)
    }
}
```

Add to `Cargo.toml [dependencies]`:
```toml
shellexpand = "3.1"
```

- [ ] **Step 4: Register module**

```rust
// src/data_provider/mod.rs
pub mod qmt_provider;
```

- [ ] **Step 5: Run test, verify PASS**

```bash
cargo test --test qmt_provider_test
```

Expected: 2 tests passed (one may skip if env var path is empty).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/data_provider/qmt_provider.rs src/data_provider/mod.rs tests/qmt_provider_test.rs
git commit -m "feat(qmt): add QmtProvider skeleton + auto_detect"
```

---

### Task 4: QmtProvider::get_daily_data 字段映射

**Files:**
- Modify: `src/data_provider/qmt_provider.rs` (impl get_daily_data)
- Modify: `tests/qmt_provider_test.rs` (加 map_kline 测试)

**Interfaces:**
- Consumes: `&self`, `code: &str`, `days: usize`
- Produces: `Result<Vec<KlineData>>`

- [ ] **Step 1: Write failing test for map_kline**

```rust
// tests/qmt_provider_test.rs (add)
use qmt_parser::DailyKlineData;

fn sample_daily() -> DailyKlineData {
    DailyKlineData {
        timestamp_ms: 1700000000000,  // 2023-11-14 22:13:20 UTC
        open: 10.0, high: 11.0, low: 9.5, close: 10.8,
        volume: 1000, amount: 10800.0, open_interest: 0,
        file_pre_close: 10.0,
    }
}

#[test]
fn map_kline_basic_fields() {
    let k = stock_analysis::data_provider::qmt_provider::map_kline_for_test(sample_daily(), "000001");
    assert_eq!(k.open, 10.0);
    assert_eq!(k.close, 10.8);
    assert_eq!(k.volume, 1000.0);  // u32 → f64
    assert_eq!(k.amount, 10800.0);
    assert!((k.pct_chg - 8.0).abs() < 0.001);  // (10.8-10)/10*100
    assert!(k.settled);
    assert!(k.intraday_price.is_none());
}
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cargo test --test qmt_provider_test map_kline
```

Expected: FAIL — `map_kline_for_test` not found.

- [ ] **Step 3: Implement get_daily_data + map_kline**

```rust
// src/data_provider/qmt_provider.rs (replace get_daily_data + add map_kline)
use chrono::{DateTime, Local, Utc};
use super::staleness::check_staleness;

impl DataProvider for QmtProvider {
    fn name(&self) -> &'static str { "qmt_local" }
    
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        let qmt_code = to_qmt_symbol(code);
        let market = market_of(code);
        // qmt-parser 0.2.1 暴露: parse_daily_to_structs(market, &qmt_code, days)
        let raw = self.dir.parse_daily_to_structs(market, &qmt_code, days)?;
        
        check_staleness(code, &raw)?;
        Ok(raw.into_iter().map(|d| map_kline(d, code)).collect())
    }
    
    fn get_stock_name(&self, _code: &str) -> Option<String> { None }
    fn get_realtime_quote(&self, _code: &str) -> Result<Option<RealtimeQuote>> { Ok(None) }
}

/// DailyKlineData → KlineData 字段映射.
/// 注: amount 的 daily ratio scale 待 Phase 1 实测核对 (qmt-parser README 提到).
pub fn map_kline(d: DailyKlineData, _our_code: &str) -> KlineData {
    use chrono::TimeZone;
    let date = Local.timestamp_opt(d.timestamp_ms / 1000, 0).single()
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| Local::now().date_naive());
    
    let pct_chg = if d.file_pre_close > 0.0 {
        (d.close - d.file_pre_close) / d.file_pre_close * 100.0
    } else { 0.0 };
    
    KlineData {
        date,
        open: d.open,
        high: d.high,
        low: d.low,
        close: d.close,
        volume: d.volume as f64,
        amount: d.amount,
        pct_chg,
        intraday_price: None,  // QMT 日线无盘中价
        settled: true,          // QMT 缓存 = 已收盘
        pe_ratio: None, pb_ratio: None,
        turnover_rate: None, market_cap: None, circulating_cap: None,
        // ... 其它字段 KlineData 默认
    }
}

/// 测试专用: 暴露 map_kline 给 tests/qmt_provider_test.rs
#[doc(hidden)]
pub fn map_kline_for_test(d: DailyKlineData, code: &str) -> KlineData {
    map_kline(d, code)
}
```

- [ ] **Step 4: Run test, verify PASS**

```bash
cargo test --test qmt_provider_test map_kline
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/data_provider/qmt_provider.rs tests/qmt_provider_test.rs
git commit -m "feat(qmt): implement get_daily_data + DailyKlineData→KlineData mapping"
```

---

### Task 5: get_stock_name (查 qmt-parser 0.2.1 实际 API)

**Files:**
- Modify: `src/data_provider/qmt_provider.rs` (impl get_stock_name)
- Modify: `tests/qmt_provider_test.rs` (加 stock_name 测试)

- [ ] **Step 1: 查 qmt-parser 0.2.1 暴露的 stock name 方法**

```bash
cargo doc --no-deps --open  # 浏览 qmt_parser 文档
# 或: grep -r "fn.*name" ~/.cargo/registry/src/*/qmt-parser-0.2.1/src/
```

记录: 方法名 + 签名 (e.g. `dir.get_short_name(market, code) -> Option<String>`).

- [ ] **Step 2: Write failing test**

```rust
// tests/qmt_provider_test.rs (add, 替换 stock_name 函数依赖实际 API)
#[test]
fn get_stock_name_returns_some_for_cached_stock() {
    // 假设 Task 1 fixture 已经创建 000001.SZ 的元数据
    // (此测试在 Task 6 fixture 完成后才能真正跑)
    // Phase 1: 只验证函数不 panic + 返回 Option<String>
    let tmp = tempdir::TempDir::new("qmt_test").unwrap();
    let datadir = tmp.path().join("datadir");
    std::fs::create_dir_all(&datadir).unwrap();
    env::set_var("QMT_DATA_DIR", datadir.parent().unwrap());
    if let Some(provider) = stock_analysis::data_provider::qmt_provider::QmtProvider::auto_detect() {
        let name = provider.get_stock_name("000001");
        // 不强制 Some (空目录时 None 是合法), 只要不 panic
        let _ = name;
    }
    env::remove_var("QMT_DATA_DIR");
}
```

- [ ] **Step 3: Implement get_stock_name**

按 Step 1 查到的 API 实现:

```rust
fn get_stock_name(&self, code: &str) -> Option<String> {
    let qmt_code = to_qmt_symbol(code);
    let market = market_of(code);
    // 替换为实际 qmt-parser 0.2.1 API (例如):
    //   self.dir.get_short_name(market, &qmt_code)
    //   或 self.dir.lookup_name(market, &qmt_code)
    //   或 None (qmt-parser 0.2.1 不支持, Phase 2)
    todo!("按 qmt-parser 0.2.1 实际 API 实现")
}
```

如果 0.2.1 不支持股票名查, 简单返 `None` 即可, 不需要 todo!().

- [ ] **Step 4: Run test, verify PASS**

```bash
cargo test --test qmt_provider_test get_stock_name
```

Expected: PASS (不 panic).

- [ ] **Step 5: Commit**

```bash
git add src/data_provider/qmt_provider.rs tests/qmt_provider_test.rs
git commit -m "feat(qmt): wire get_stock_name via qmt-parser 0.2.1 API"
```

---

### Task 6: 测试 fixture + 集成测试

**Files:**
- Create: `tests/fixtures/qmt/datadir/day/000001.SZ.day` (binary fixture)
- Create: `tests/fixtures/qmt/datadir/day/600000.SH.day` (binary fixture)
- Modify: `tests/qmt_provider_test.rs` (加 integration tests)

- [ ] **Step 1: 用 qmt-parser 0.2.1 反向生成 fixture**

如果手头有 MiniQMT 安装:
```bash
# 从 ~/qmtdata/datadir/day/000001.SZ.day 复制 5 行 (truncate)
cp ~/qmtdata/datadir/day/000001.SZ.day tests/fixtures/qmt/datadir/day/000001.SZ.day
```

如无 MiniQMT: 写一个一次性 Python/Rust 脚本调用 qmt-parser 生成 fixture, 或在 PR 中用空白 fixture (Task 9 集成测试会 skip).

- [ ] **Step 2: Write integration test (uses real fixture)**

```rust
// tests/qmt_provider_test.rs (add)
#[test]
fn parse_real_fixture_000001_sz() {
    let tmp = tempdir::TempDir::new("qmt_test_real").unwrap();
    // 复制 fixture 到临时目录
    let src = std::path::Path::new("tests/fixtures/qmt");
    copy_dir_recursive(src, &tmp.path()).unwrap();
    
    env::set_var("QMT_DATA_DIR", tmp.path());
    let provider = stock_analysis::data_provider::qmt_provider::QmtProvider::auto_detect()
        .expect("fixture should make auto_detect succeed");
    let klines = provider.get_daily_data("000001", 5).expect("parse should succeed");
    env::remove_var("QMT_DATA_DIR");
    
    assert!(!klines.is_empty(), "fixture must contain at least 1 row");
    let k = &klines[0];
    assert!(k.open > 0.0);
    assert!(k.close > 0.0);
    assert!(k.settled);
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    // 简化: 用 std::fs::copy 逐个文件, 或加 fs_extra crate
    // Phase 1: 用 std::process::Command + cp (Unix only)
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Run integration test, verify PASS**

```bash
cargo test --test qmt_provider_test parse_real_fixture
```

Expected: PASS (用真实 fixture).

- [ ] **Step 4: Commit**

```bash
git add tests/fixtures/qmt/ tests/qmt_provider_test.rs
git commit -m "test(qmt): add real .day fixture + parse integration test"
```

---

### Task 7: Fallback 集成 (QmtProvider 作为 priority 0)

**Files:**
- Modify: `src/data_provider/fallback.rs` (加 QmtProvider 进 `SourceResult` + 4-way join)
- Modify: `tests/fallback_qmt_test.rs` (新文件, 集成测试)

- [ ] **Step 1: Write failing integration test**

```rust
// tests/fallback_qmt_test.rs
use stock_analysis::data_provider::fallback::fetch_kline_with_fallback;

#[tokio::test]
async fn fallback_prefers_qmt_when_available() {
    // 假设 QMT_DATA_DIR 已 set 且有 fixture
    // (测试 fixtures 在 Task 6)
    let (data, src) = fetch_kline_with_fallback("000001", 5).await.unwrap();
    // QMT 是 priority 0, 成功 → qmt_local
    assert_eq!(src, "qmt_local");
    assert!(!data.is_empty());
}

#[tokio::test]
async fn fallback_uses_http_when_qmt_stale() {
    // 把 fixture 改成 5 天前 (Task 6 fixture 创建后)
    // → QMT stale → Err → fallthrough HTTP
    let (_data, src) = fetch_kline_with_fallback("000001", 5).await.unwrap();
    // HTTP (mock 或真实) → tencent_qfq 或 eastmoney_qfq
    assert!(matches!(src, "tencent_qfq" | "eastmoney_qfq"));
}
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cargo test --test fallback_qmt_test
```

Expected: FAIL — `SourceResult::Qmt` 不存在, fallback 不接 QMT.

- [ ] **Step 3: Modify fallback.rs**

```rust
// src/data_provider/fallback.rs (modify)
use crate::data_provider::qmt_provider::QmtProvider;

// 1. 加 SourceResult::Qmt 变体
enum SourceResult {
    Qmt(Result<Vec<KlineData>>),
    Tencent(Result<Vec<KlineData>>),
    Eastmoney(Result<Vec<KlineData>>),
    Rustdx(Result<Vec<KlineData>>),
}

// 2. 在 fetch_kline_with_fallback 内, 加 qmt_fut + 4-way join
pub async fn fetch_kline_with_fallback(
    code: &str,
    days: usize,
) -> Result<(Vec<KlineData>, &'static str)> {
    let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
    let qc_threshold = max_gap_for(code);
    
    // QMT (priority 0): 本地缓存, 零网络
    let qmt_fut = match QmtProvider::auto_detect() {
        Some(provider) => {
            let code = code.to_string();
            Either::Left(async move {
                let r = tokio::task::spawn_blocking(move || provider.get_daily_data(&code, days))
                    .await
                    .map_err(|e| anyhow!("QMT task: {e}"))
                    .and_then(|inner| inner);
                SourceResult::Qmt(r)
            })
        }
        None => Either::Right(async { 
            SourceResult::Qmt(Err(anyhow!("QMT not installed (auto_detect returned None)")))
        }),
    };
    // ... tencent_fut / eastmoney_fut / rustdx_fut (现有)
    
    let (q, t, e, r) = tokio::join!(qmt_fut, tencent_fut, eastmoney_fut, rustdx_fut);
    
    let candidates: [(SourceResult, &'static str); 4] = [
        (q, "qmt_local"),       // NEW: priority 0
        (t, "tencent_qfq"),
        (e, "eastmoney_qfq"),
        (r, "rustdx_none"),
    ];
    // ... 后续循环不变 (QMT 失败 → 跳过 → 下一候选)
}
```

如果 0.2.1 没有 `Either` 风格, 用 `BoxFuture` 或 `Pin<Box<dyn Future>>` 替代:
```rust
use futures::future::BoxFuture;
// ...
let qmt_fut: BoxFuture<'_, SourceResult> = match QmtProvider::auto_detect() {
    Some(p) => Box::pin(async move { /* ... */ }),
    None => Box::pin(async { SourceResult::Qmt(Err(anyhow!("..."))) }),
};
```

- [ ] **Step 4: Run test, verify PASS**

```bash
cargo test --test fallback_qmt_test
```

Expected: 2 tests passed.

- [ ] **Step 5: Run all tests, verify no regression**

```bash
cargo test --lib
```

Expected: 909+ passed (含 8 新测试).

- [ ] **Step 6: Commit**

```bash
git add src/data_provider/fallback.rs tests/fallback_qmt_test.rs
git commit -m "feat(qmt): integrate QmtProvider as fallback priority 0 (4-way join)"
```

---

### Task 8: 启动日志 + BR 文档 + README

**Files:**
- Modify: `src/bin/monitor/main.rs` (启动时打印 QMT 状态)
- Modify: `docs/business_rules.md` (加 BR-013)
- Create: `docs/qmt_integration.md` (用户文档)
- Modify: `README.md` (加 QMT 段)

- [ ] **Step 1: Modify main.rs to log QMT status at startup**

```rust
// src/bin/monitor/main.rs (在 main 开头, after config 加载)
match stock_analysis::data_provider::qmt_provider::QmtProvider::auto_detect() {
    Some(p) => log::info!("[启动] QMT 本地缓存已启用: name={}", p.name()),
    None => log::info!("[启动] QMT 未安装, 跳过本地缓存 (HTTP 兜底)"),
}
```

- [ ] **Step 2: Add BR-013 to business_rules.md**

```markdown
| BR-013 | ✅ registered | QMT 本地缓存接入 fallback priority 0 — 零网络 fallback, stale 数据 fallthrough HTTP | `src/data_provider/qmt_provider.rs`, `src/data_provider/fallback.rs` |
```

- [ ] **Step 3: Write user-facing docs**

```markdown
<!-- docs/qmt_integration.md -->
# QMT 本地缓存集成

## 背景
[...]

## License
qmt-parser 是 GPL-3.0. 项目自用, 不分发, OK.

## 安装
1. 装 MiniQMT (xtquant)
2. 启动一次缓存数据到 ~/qmtdata/datadir/day/
3. cargo build  (qmt-parser 已 default-on)

## 配置
- `QMT_DATA_DIR` env var 覆盖 auto-detect
- 默认探测: `~/qmtdata`, `~/.qmtdata`, `/mnt/data/trade/qmtdata`, `./qmtdata`

## Fallback 链
[流程图: QMT (0) → 腾讯 (1) → 东财 (2) → RustDX (3)]

## 故障排查
- QMT 没启用 → 看 `[启动] QMT 未安装` 日志
- Stale fallthrough → 看 `QMT {code} 数据滞后` warn
- 解析失败 → 看 `QMT 路径 {path} 打开失败` error
```

- [ ] **Step 4: Update README**

```markdown
<!-- README.md, 在 ## Architecture 段加 -->
### 数据源
- K线 fallback: QMT (本地) → 腾讯 HTTP → 东财 HTTP → RustDX TCP
- 详见 `docs/qmt_integration.md`
```

- [ ] **Step 5: Run all tests, verify no regression**

```bash
cargo test --lib
```

Expected: 909+ passed.

- [ ] **Step 6: Commit**

```bash
git add src/bin/monitor/main.rs docs/business_rules.md docs/qmt_integration.md README.md
git commit -m "docs(qmt): add integration docs, BR-013, README update, startup log"
```

---

## Final verification

```bash
cargo build --release
cargo test --lib
cargo clippy -- -D warnings
```

All must pass. Then push to master.
