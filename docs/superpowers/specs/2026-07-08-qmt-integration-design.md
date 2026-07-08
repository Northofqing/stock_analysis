# QMT 本地缓存集成 — 设计文档

**Date**: 2026-07-08
**Status**: Approved (brainstorming complete)
**Author**: Claude + user

## 1. 背景

A 股实时交易监控依赖 HTTP K 线源（腾讯 → 东财 → RustDX TCP 兜底）。
review #14/15 修复了三源竞速 + 质检门禁，但 HTTP 仍可能：

- IP ban（东财 429、腾讯黑名单）— review #14 见过
- DNS 抽风、机房临时断网
- 海外出差无国内网

**MiniQMT/QMT** 是券商本地客户端，会把行情数据缓存为 `.dat` 二进制文件到 `~/qmtdata/datadir/`。
[sunnysab/qmt-parser](https://github.com/sunnysab/qmt-parser) 是 Rust 写的解析库，零网络依赖。

**目标**：QMT 作为 priority 0 接入 fallback 链，HTTP 全失败时本地兜底。

## 2. License 风险

qmt-parser 是 **GPL-3.0**（强制传染 — 静态/动态链接它的 binary 分发时需 GPL-3.0 兼容）。

**项目状态**：自用 + 不分发，README 写明「仅供个人量化研究与学习」。

**接受 GPL-3.0 传染**：项目本就仅自用，实际不受影响。
集成方式：default-on（`qmt-parser` 加入 `dependencies`，不设 feature gate）。

## 3. 范围

集成 **全量** 数据类型（qmt-parser 0.2.1 全部支持）：

| 数据 | qmt-parser 路径 | 我们的暴露 |
|------|----------------|------------|
| 日线 K 线 | `{qmt}/datadir/day/{code}.day` | `get_daily_data()` (已有) |
| 1-min K 线 | `{qmt}/datadir/{market}/{code}-1m.dat` | 新增 `get_minute_kline()` |
| Tick 数据 | `{qmt}/datadir/{market}/{code}-{date}-tick.dat` | 新增 `get_tick_data()` |
| 财务数据 | `{qmt}/datadir/finance/{code}.DAT` | 新增 `get_finance()` |
| 股票名 | sector/industry CSV/DAT | `get_stock_name()` (已有) |

**简化**：先做日线 + 股票名（核心 fallback 价值），1-min / tick / 财务留作 Phase 2。

## 4. 架构

### 4.1 总览

```
DataFetcherManager::get_daily_data(code, days)
  ↓
fetch_kline_with_fallback()  ← review #15 竞速链
  ↓
┌─────────────────────────────────────────────────────┐
│ Priority 0: QmtProvider   (本地 .dat, 零网络)  NEW │
│ Priority 1: GtimgProvider (HTTP 腾讯)              │
│ Priority 2: HttpProvider  (HTTP 东财)              │
│ Priority 3: RustdxProvider(TCP 兜底)              │
└─────────────────────────────────────────────────────┘
  ↓ 第一个 Ok + 质检通过 胜出
```

### 4.2 修改/新增文件 (Phase 1 — 日线 + 股票名)

| 文件 | 类型 | 行数 |
|------|------|------|
| `src/data_provider/qmt_provider.rs` | **NEW** | ~220 |
| `src/data_provider/stock_code_map.rs` | **NEW** | ~50 |
| `src/data_provider/fallback.rs` | 修改 | +35 |
| `src/data_provider/mod.rs` | 修改 | +10 |
| `src/bin/monitor/main.rs` | 修改 | +5 (启动日志) |
| `tests/qmt_provider_test.rs` | **NEW** | ~180 |
| `tests/fixtures/qmt/000001.day` | **NEW** | binary fixture |
| `docs/qmt_integration.md` | **NEW** | ~200 |
| `Cargo.toml` | 修改 | +2 |

> **Phase 2 (1-min / tick / 财务) 留到 Section 10**，本 spec 只覆盖日线 + 股票名。

## 5. 组件设计

### 5.1 QmtProvider

```rust
// src/data_provider/qmt_provider.rs
use std::path::PathBuf;
use qmt_parser::{QmtDataDir, Market};

pub struct QmtProvider {
    dir: QmtDataDir,
    market: Market,  // Sz / Sh — 根据 code 前缀推断
}

impl QmtProvider {
    /// auto-detect 默认路径 + env var 覆盖
    /// 默认: ~/qmtdata, ~/.qmtdata, /mnt/data/trade/qmtdata
    pub fn auto_detect() -> Option<Self> {
        let path = std::env::var("QMT_DATA_DIR").ok()
            .map(PathBuf::from)
            .or_else(|| default_paths().into_iter().find(|p| p.exists()))?;
        QmtDataDir::new(path.to_str()?).ok().map(|dir| Self { dir, market: Market::Sz })
    }
}

impl DataProvider for QmtProvider {
    fn name(&self) -> &'static str { "qmt_local" }
    
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        let qmt_code = to_qmt_symbol(code);
        let market = market_of(code);
        let raw = self.dir.parse_daily_to_structs(market, &qmt_code, ...)?;
        
        // Stale 检查 (Section 6)
        check_staleness(code, &raw)?;
        
        // 字段映射
        Ok(raw.into_iter().map(|d| map_kline(d, code)).collect())
    }
    
    fn get_stock_name(&self, code: &str) -> Option<String> {
        // Phase 1: 查 qmt-parser 暴露的股票名方法 (具体 API 在实现时核对).
        // 0.2.1 API: `dir.get_short_name(market, qmt_code)` 或类似.
        // 不存在时返 None (HTTP provider 兜底).
        todo!("在实现 Phase 1 时核对 qmt-parser 0.2.1 暴露的 stock name API")
    }
}
```

### 5.2 Stock code 映射

```rust
// src/data_provider/stock_code_map.rs

/// 我们的 "000001" → QMT 的 "000001.SZ"
pub fn to_qmt_symbol(code: &str) -> String {
    let suffix = match code.chars().next() {
        Some('6') | Some('9') | Some('5') => ".SH",  // 沪市
        _ => ".SZ",                                   // 深市
    };
    format!("{}{}", code, suffix)
}

pub fn from_qmt_symbol(qmt_code: &str) -> String {
    qmt_code.split('.').next().unwrap_or(qmt_code).to_string()
}

pub fn market_of(code: &str) -> Market {
    match code.chars().next() {
        Some('6') | Some('9') | Some('5') => Market::Sh,
        _ => Market::Sz,
    }
}
```

### 5.3 QmtDataDir 字段映射

```rust
fn map_kline(d: DailyKlineData, our_code: &str) -> KlineData {
    use chrono::TimeZone;
    let date = chrono::Utc
        .timestamp_millis_opt(d.timestamp_ms)
        .unwrap()
        .with_timezone(&chrono::Local)
        .date_naive();
    
    // qmt 的 amount 是按 daily ratio scale 过，需查文档验证
    // Phase 1: 直接用，标注 possible scale issue
    let amount = d.amount;
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
        amount,
        pct_chg,
        intraday_price: None,        // QMT 日线无盘中价
        settled: true,                // QMT 缓存是已收盘数据
        pe_ratio: None,
        pb_ratio: None,
        turnover_rate: None,
        market_cap: None,
        circulating_cap: None,
        // ... 其它字段 None
    }
}
```

## 6. 错误处理 + Stale Fallthrough

### 6.1 错误分类

| 错误类型 | 行为 | 日志 |
|---------|------|------|
| Provider 未启用 (auto-detect 失败) | `None`，fallback 跳过 | `info!` 一次性 |
| 股票代码无缓存 (`Ok(empty)`) | 质检 reject，fallthrough | `debug!` |
| **Stale 数据** (最新日期 < 1 trading day) | `Err`，fallthrough HTTP | `warn!` |
| 文件 IO 错误 (权限/损坏) | `Err`，fallback 终止 | `error!` |
| 格式不识别 (QMT 升级) | `Err` | `error!` |

### 6.2 Stale 检查

```rust
fn check_staleness(code: &str, raw: &[DailyKlineData]) -> Result<()> {
    let Some(latest) = raw.last() else { return Ok(()); };  // 空数据不 stale
    let today = chrono::Local::now().date_naive();
    let staleness_days = (today - latest.timestamp_date()).num_days();
    
    // 超过 1 trading day 才算 stale (允许盘后场景)
    if staleness_days > 1 && !is_recent_trading_day(today) {
        log::warn!(
            "QMT {code} 数据滞后 {n} 天 (最新 {date}), fallthrough HTTP",
            code = code, n = staleness_days, date = latest.timestamp_date()
        );
        return Err(anyhow!("stale: latest={}, today={}", latest.timestamp_date(), today));
    }
    Ok(())
}

fn is_recent_trading_day(d: NaiveDate) -> bool {
    // 复用 calendar.rs::is_trading_day() + 前看 1 天
    let d1 = d - chrono::Duration::days(1);
    calendar::is_trading_day(d) || calendar::is_trading_day(d1)
}
```

### 6.3 Fallback 集成

```rust
// fallback.rs 修改
enum SourceResult {
    Qmt(Result<Vec<KlineData>>),       // NEW
    Tencent(Result<Vec<KlineData>>),
    Eastmoney(Result<Vec<KlineData>>),
    Rustdx(Result<Vec<KlineData>>),
}

let qmt_fut = match QmtProvider::auto_detect() {
    Some(p) => Either::Left(async move {
        let r = tokio::task::spawn_blocking(move || p.get_daily_data(code, days)).await
            .map_err(|e| anyhow!("QMT task: {}", e))
            .and_then(|inner| inner);
        SourceResult::Qmt(r)
    }),
    None => Either::Right(async { SourceResult::Qmt(Err(anyhow!("QMT not available"))) }),
};
// ... tencent_fut, eastmoney_fut, rustdx_fut

let (q, t, e, r) = tokio::join!(qmt_fut, tencent_fut, eastmoney_fut, rustdx_fut);

let candidates = [
    (q, "qmt_local"),       // Priority 0 — 最先
    (t, "tencent_qfq"),
    (e, "eastmoney_qfq"),
    (r, "rustdx_none"),
];
// ... 后续循环不变
```

## 7. 测试策略

### 7.1 单元测试

```rust
// tests/qmt_provider_test.rs

#[test]
fn test_auto_detect_no_path() {
    std::env::remove_var("QMT_DATA_DIR");
    // 默认探测路径都不存在 → None
    assert!(QmtProvider::auto_detect().is_none());
}

#[test]
fn test_auto_detect_env_var() {
    let tmp = tempdir();
    std::env::set_var("QMT_DATA_DIR", tmp.path());
    assert!(QmtProvider::auto_detect().is_some());
    std::env::remove_var("QMT_DATA_DIR");
}

#[test]
fn test_to_qmt_symbol_sz() {
    assert_eq!(to_qmt_symbol("000001"), "000001.SZ");
    assert_eq!(to_qmt_symbol("301000"), "301000.SZ");
}

#[test]
fn test_to_qmt_symbol_sh() {
    assert_eq!(to_qmt_symbol("600000"), "600000.SH");
    assert_eq!(to_qmt_symbol("900900"), "900900.SH");
}

#[test]
fn test_market_of() {
    assert!(matches!(market_of("000001"), Market::Sz));
    assert!(matches!(market_of("600000"), Market::Sh));
}

#[test]
fn test_map_kline_basic() {
    let raw = DailyKlineData {
        timestamp_ms: 1700000000000,  // 2023-11-14
        open: 10.0, high: 11.0, low: 9.5, close: 10.8,
        volume: 1000, amount: 10800.0, open_interest: 0,
        file_pre_close: 10.0,
    };
    let k = map_kline(raw, "000001");
    assert_eq!(k.open, 10.0);
    assert_eq!(k.pct_chg, 8.0);  // (10.8-10)/10*100
    assert!(k.settled);
    assert!(k.intraday_price.is_none());
}

#[test]
fn test_staleness_rejects_2_days_old() {
    let old = make_daily_with_date(NaiveDate::from_ymd(2026, 7, 6));  // 2 days ago
    let result = check_staleness("000001", &[old]);
    assert!(result.is_err());
}

#[test]
fn test_staleness_accepts_today() {
    let today_daily = make_daily_with_date(chrono::Local::now().date_naive());
    let result = check_staleness("000001", &[today_daily]);
    assert!(result.is_ok());
}
```

### 7.2 Fixture 策略

从 qmt-parser 0.2.1 解析过的真实 binary dump 5 行 fixture：

```rust
// tests/fixtures/qmt/000001.day
// 5 个交易日的真实 MiniQMT 格式 K 线
// 用 Python 脚本从 MiniQMT 安装目录 dump 出来 (build-time only)
```

不依赖完整 MiniQMT 工具链。

### 7.3 集成测试

```rust
#[tokio::test]
async fn test_fallback_qmt_wins() {
    // QmtProvider 存在 + 有数据 → 走 QMT
    setup_qmt_fixture();
    let (data, src) = fetch_kline_with_fallback("000001", 5).await.unwrap();
    assert_eq!(src, "qmt_local");
    assert_eq!(data.len(), 5);
}

#[tokio::test]
async fn test_fallback_qmt_stale_to_http() {
    // QMT 数据是 3 天前 → fallthrough 到 HTTP (mock)
    setup_qmt_stale_fixture();
    let (data, src) = fetch_kline_with_fallback("000001", 5).await.unwrap();
    assert_ne!(src, "qmt_local");
    assert!(matches!(src, "tencent_qfq" | "eastmoney_qfq"));
}

#[tokio::test]
async fn test_fallback_qmt_missing_to_http() {
    // QMT 没有此股 → fallthrough
    setup_empty_qmt_for("999999");
    let (_, src) = fetch_kline_with_fallback("999999", 5).await.unwrap();
    assert_ne!(src, "qmt_local");
}
```

## 8. 部署 + 配置

### 8.1 Cargo.toml

```toml
[dependencies]
qmt-parser = "0.2"
```

### 8.2 安装步骤

1. 装 MiniQMT (xtquant) 客户端
2. 启动一次让它缓存数据到 `~/qmtdata/datadir/`
3. `cargo build`（qmt-parser 已 default-on）
4. 启动监控，日志显示 `QMT 启用: 路径=~/qmtdata`

### 8.3 配置

- `QMT_DATA_DIR` env var 覆盖 auto-detect
- auto-detect 默认路径（按顺序探测）：
  1. `~/qmtdata` (MiniQMT 默认)
  2. `~/.qmtdata` (xtquant 自定义)
  3. `/mnt/data/trade/qmtdata` (生产服务器)
  4. `./qmtdata` (开发时)

### 8.4 不启用 QMT

删 MiniQMT 或 `QMT_DATA_DIR=/nonexistent` → auto-detect 失败 → log info 一次 → 走纯 HTTP 路径。无破坏性。

## 9. 验收标准

- [ ] `cargo build` 通过
- [ ] `cargo test` 全部通过（含 8 个新测试 + 现有 907 个）
- [ ] 装 MiniQMT 后启动：log 显示 `QMT 启用`，K线源竞速日志出现 `qmt_local`
- [ ] 离线场景：拔网线 + 装 MiniQMT → 仍能拿日 K 线（HTTP 三个全 fail，QMT 胜出）
- [ ] Stale 场景：人为把 fixture 改 3 天前 → log warn + fallthrough HTTP
- [ ] 无 QMT 场景：删 MiniQMT → log info 一次后走 HTTP，行为与现在一致
- [ ] 文档 `docs/qmt_integration.md` 完整可读

## 10. Phase 2 (后续)

集成 1-min K线 / tick / 财务数据：

```rust
impl DataProvider for QmtProvider {
    // Phase 1
    fn get_daily_data(...) -> ...;
    fn get_stock_name(...) -> ...;
    
    // Phase 2
    fn get_minute_kline(&self, code: &str, count: usize) -> Result<Vec<MinKline>> { ... }
    fn get_tick_data(&self, code: &str, date: NaiveDate) -> Result<Vec<TickData>> { ... }
    fn get_finance(&self, code: &str) -> Result<Vec<FinanceRecord>> { ... }
}
```

需要扩展 `DataProvider` trait 加默认实现 + 新增 `MinKline` / `TickData` / `FinanceRecord` 类型映射。

## 11. 风险 + 缓解

| 风险 | 缓解 |
|------|------|
| qmt-parser 0.2.1 API 改动 | Pin `qmt-parser = "=0.2.1"` exact version |
| QMT 数据格式升级 (xtquant 升级) | `parse_daily_to_structs` 失败时 fallthrough HTTP + log error |
| Auto-detect 误判 (路径碰巧存在但不是 QMT) | 启动时检查 `datadir/day/{至少一个}.day` 存在再启用 |
| Stale 数据被误接受 | 严格 1 trading day 阈值 + log warn 提醒 |
| GPL 传染 | 项目自用不分发，影响可控 |
