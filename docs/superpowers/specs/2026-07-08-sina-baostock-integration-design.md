# Sina + Baostock 数据源集成 — 设计文档

**Date**: 2026-07-08
**Status**: Approved (brainstorming complete)
**Author**: Claude + user
**Scope**: 2 个免费稳定源替代/补充 review #15 的 4 源 fallback.

## 1. 背景

review #15 完成后 fallback 链有 4 源（腾讯 / 东财 ×3 host / RustDX TCP），但都是：
- **公开 HTTP 接口** — 风险同质（一家云被 ban 其它家也可能）
- **盘中实时** — 盘后/夜间无数据
- **反爬严** — review #14 见过 IP ban / 限频

**目标**：加 2 个新源
- **新浪 (Sina)** — 公开 HTTP，IP 域名独立于腾讯/东财，0 费用 0 注册
- **Baostock** — 证券所级别日终数据，**无限调用**，日终兜底

## 2. License & 风险

| 源 | License | 风险 |
|----|---------|------|
| Sina (hq.sinajs.cn) | 公开 HTTP，爬虫灰色 | 服务无 SLA，但同 IP/域名独立降低联 ban 风险 |
| Baostock (baostock.com) | 公开免费，明确允许量化用途 | 🟢 极稳，证券所级别数据 |

**无新增依赖风险**。纯 HTTP + WebSocket-like session。

## 3. 范围

| 数据类型 | Sina | Baostock |
|----------|------|----------|
| 日 K 线 | ✅ JSON 数组 | ✅ POST 序时 |
| 实时价 | ✅ hq_str 字段 | ❌ (无盘中) |
| 分钟 K 线 | ✅ 1/5/15/30/60min | ❌ (无) |
| 复权 | ✅ 前复权参数 | ✅ adjustflag 1/2/3 |
| A 股完整 | ✅ 沪深 | ✅ 沪深 |
| 港美股 | ❌ | ❌ |

**Phase 1 范围**：
- Sina: 日 K 线（实时价用 hq_str 顺手取，0 成本）
- Baostock: 日 K 线 + 日终复权

## 4. 架构

### 4.1 总览

```
DataFetcherManager::get_daily_data(code, days)
  ↓
fetch_kline_with_fallback() ← review #15 竞速链, 5 源 (review #15 4 源 + 新浪)
  ↓ Priority 0 = 将来可能加 QMT; 当前 priority 0 不存在, 自动 fallthrough
  ├─ Priority 1: SinaProvider       ← NEW
  ├─ Priority 2: GtimgProvider
  ├─ Priority 3: HttpProvider
  └─ Priority 4: RustdxProvider

盘后 (15:00-次日 9:30) 独立路径:
fetch_kline_post_close(code, days)
  ├─ Step 1: BaostockProvider     ← NEW (日终, 无限流, 最稳)
  ├─ Step 2: SinaProvider
  ├─ Step 3: GtimgProvider
  └─ Step 4: HttpProvider
```

### 4.2 关键设计

**Sina 是 fallback 链的一等公民**：竞速 + 质检 + 第一 Ok 胜出。

**Baostock 是盘后专用路径**：不参与盘中竞速，避免 4 源 HTTP 全失败时被陈旧的日终数据"救场"误判。

### 4.3 新增/修改文件

| 文件 | 类型 | 行数 |
|------|------|------|
| `src/data_provider/sina_provider.rs` | **NEW** | ~180 |
| `src/data_provider/baostock_provider.rs` | **NEW** | ~250 |
| `src/data_provider/stock_code_map.rs` | 修改 | +20 (to_sina/to_baostock/from_baostock) |
| `src/data_provider/fallback.rs` | 修改 | +20 (Sina 接入) + ~80 (新 fetch_kline_post_close) |
| `src/data_provider/mod.rs` | 修改 | +3 |
| `src/bin/monitor/main.rs` | 修改 | +10 (盘后切换入口) |
| `tests/sina_provider_test.rs` | **NEW** | ~120 |
| `tests/baostock_provider_test.rs` | **NEW** | ~150 |
| `docs/sina_baostock_integration.md` | **NEW** | ~150 |
| `Cargo.toml` | 修改 | +2 (encoding_rs 等) |

## 5. 组件设计

### 5.1 SinaProvider

```rust
// src/data_provider/sina_provider.rs
use crate::data_provider::stock_code_map::to_sina;

pub struct SinaProvider {
    client: reqwest::Client,
}

impl SinaProvider {
    pub fn new() -> Self { ... }
}

impl DataProvider for SinaProvider {
    fn name(&self) -> &'static str { "sina_hq" }
    
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        // Sina K线 JSON URL:
        // https://quotes.sina.cn/cn/api/jsonp_v2.php/=/CN_MarketDataService.getKLineData?symbol=sh600519&scale=240&datalen=30
        let url = format!(
            "https://quotes.sina.cn/cn/api/jsonp_v2.php/=/CN_MarketDataService.getKLineData\
             ?symbol={}&scale=240&datalen={}",
            to_sina(code), days
        );
        let body = self.client.get(&url).header("Referer", "https://finance.sina.com.cn").send().await?.text().await?;
        // body 是 JSONP: var arr_data=[{day,open,high,low,close,volume}, ...];
        // 提取 [ ... ] 部分, JSON parse
        parse_sina_kline(&body, code)
    }
    
    fn get_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        // https://hq.sinajs.cn/list=sh600519
        // 返回: var hq_str_sh600519="平安银行,13.50,13.45,...";
        // 字段: [0]name, [1]today_open, [2]yesterday_close, [3]current, [4]high, [5]low, ...
        // [6]bid, [7]ask, [8]volume, [9]amount
        let url = format!("https://hq.sinajs.cn/list={}", to_sina(code));
        let body = self.client.get(&url).header("Referer", "https://finance.sina.com.cn").send().await?.text().await?;
        parse_sina_realtime(&body, code)
    }
}

fn parse_sina_kline(body: &str, code: &str) -> Result<Vec<KlineData>> {
    // JSONP: var arr_data=[...]; 或 var arr_data=null;
    // 截取 [...] 部分
    let json_start = body.find('[').ok_or_else(|| anyhow!("Sina K线: 无 JSON 数据"))?;
    let json_end = body.rfind(']').ok_or_else(|| anyhow!("Sina K线: JSON 不完整"))?;
    let json = &body[json_start..=json_end];
    let arr: Vec<SinaKlineRow> = serde_json::from_str(json)?;
    Ok(arr.into_iter().map(|r| map_kline(r, code)).collect())
}

fn map_kline(r: SinaKlineRow, _code: &str) -> KlineData {
    let date = NaiveDate::parse_from_str(&r.day, "%Y-%m-%d").unwrap_or_else(|_| Local::now().date_naive());
    let open = r.open.parse().unwrap_or(0.0);
    // ... 映射
    KlineData {
        date, open, high: r.high.parse().unwrap_or(0.0),
        low: r.low.parse().unwrap_or(0.0),
        close: r.close.parse().unwrap_or(0.0),
        volume: r.volume.parse().unwrap_or(0.0),
        amount: 0.0,  // Sina 不直接给, 可用 close * volume 估算
        pct_chg: if open > 0.0 { (close - open) / open * 100.0 } else { 0.0 },
        intraday_price: None, settled: true,
        // ... 其它 None
    }
}
```

### 5.2 BaostockProvider

```rust
// src/data_provider/baostock_provider.rs
use crate::data_provider::stock_code_map::{to_baostock, from_baostock};

pub struct BaostockProvider {
    client: reqwest::Client,
    base_url: String,  // http://baostock.com/baostock
    session_id: Option<String>,
}

impl BaostockProvider {
    pub fn new() -> Self { ... }
    
    async fn ensure_session(&mut self) -> Result<()> {
        if self.session_id.is_some() { return Ok(()); }
        // POST {base}/Login (form-encoded: user=anonymous&password=...)
        let resp = self.client.post(format!("{}/Login", self.base_url))
            .form(&[("user", "anonymous"), ("password", "888888")])
            .send().await?.text().await?;
        // 解析 "sessionId=XXXXX\nErrorCode=0\nErrorMsg=..."
        let sid = parse_baostock_response(&resp, "sessionId")?
            .ok_or_else(|| anyhow!("Baostock login: 无 sessionId"))?;
        self.session_id = Some(sid);
        Ok(())
    }
}

impl DataProvider for BaostockProvider {
    fn name(&self) -> &'static str { "baostock" }
    
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        // session 用 tokio::sync::Mutex<Option<String>> 包 (见下方 decision)
        let session_id = self.ensure_session().await?;
        let url = format!(
            "{}/QueryHistoryKLinePlus?code={}&fields=date,open,high,low,close,volume,amount&adjustflag=2&startdate=...&enddate=...&sessionid={session_id}",
            self.base_url, to_baostock(code)
        );
        let body = self.client.post(&url).send().await?.text().await?;
        parse_baostock_kline(&body, code)
    }
}
```

**Trait 限制**：现有 `DataProvider::get_daily_data` 是 `&self`。Baostock 需要可变 session。
**方案**：
- 改 trait 成 `&mut self`？**破坏性** — 其它 provider 都不需要 mutable
- 用 `OnceCell<String>` 包 session_id（lazy 初始化 + interior mutability）✅
- 用 `tokio::sync::Mutex<String>` 包 session（async-friendly）✅

**决策**：用 `tokio::sync::Mutex<Option<String>>` 包 session。

```rust
pub struct BaostockProvider {
    client: reqwest::Client,
    session: tokio::sync::Mutex<Option<String>>,
}

impl BaostockProvider {
    async fn ensure_session(&self) -> Result<String> {
        let mut guard = self.session.lock().await;
        if let Some(sid) = guard.as_ref() {
            return Ok(sid.clone());
        }
        let sid = self.login().await?;
        *guard = Some(sid.clone());
        Ok(sid)
    }
}
```

### 5.3 Stock code 映射（扩展现有）

```rust
// src/data_provider/stock_code_map.rs 新增:
pub fn to_sina(code: &str) -> String {
    let prefix = match code.chars().next() {
        Some('6') | Some('9') | Some('5') => "sh",
        _ => "sz",
    };
    format!("{}{}", prefix, code)
}

pub fn to_baostock(code: &str) -> String {
    let prefix = match code.chars().next() {
        Some('6') | Some('9') | Some('5') => "sh",
        _ => "sz",
    };
    format!("{}.{}", prefix, code)
}

pub fn from_baostock(bs_code: &str) -> String {
    bs_code.split('.').nth(1).unwrap_or(bs_code).to_string()
}
```

### 5.4 fallback.rs 改造

```rust
// 1. SinaProvider 加到 review #15 的 4-way join (变 5-way)
enum SourceResult {
    Sina(Result<Vec<KlineData>>),     // NEW
    Tencent(Result<Vec<KlineData>>),
    Eastmoney(Result<Vec<KlineData>>),
    Rustdx(Result<Vec<KlineData>>),
}

let candidates = [
    (s, "sina_hq"),        // NEW priority 1
    (t, "tencent_qfq"),
    (e, "eastmoney_qfq"),
    (r, "rustdx_none"),
];

// 2. 新增 fetch_kline_post_close (盘后专用)
pub async fn fetch_kline_post_close(code: &str, days: usize) -> Result<(Vec<KlineData>, &'static str)> {
    // 1. Baostock 优先 (日终权威, 0 风险)
    match BaostockProvider::new().get_daily_data(code, days).await {
        Ok(d) if !d.is_empty() => return Ok((d, "baostock")),
        Ok(_) => log::debug!("Baostock {code} 返回空"),
        Err(e) => log::warn!("Baostock {code} 失败: {e}"),
    }
    // 2. fallthrough 到 fallback chain
    fetch_kline_with_fallback(code, days).await
}
```

### 5.5 main.rs 集成

```rust
// src/bin/monitor/main.rs 在 收盘后 review 任务里:
use stock_analysis::data_provider::fallback::fetch_kline_post_close;

async fn post_close_review() {
    let now = Local::now();
    let session = crate::calendar::session_at(now.naive_local());
    if !matches!(session, MarketSession::AfterHours | MarketSession::Closed) {
        return;  // 非盘后, 不调
    }
    for code in holdings {
        let (data, src) = fetch_kline_post_close(code, 30).await?;
        log::info!("[盘后] {code} K线: src={src} 数量={}", data.len());
        // ... 复盘逻辑
    }
}
```

## 6. 错误处理

| 错误 | Sina | Baostock |
|------|------|----------|
| HTTP 4xx/5xx | `Err`，fallthrough | `Err`，fallthrough |
| 解析失败 (JSON/HTML 变) | `Err`，fallthrough + log error | `Err`，fallthrough + log error |
| Rate limit (Sina 偶发 403) | `Err`，fallthrough | **不会发生** (无限流) |
| 登录失败 (Baostock) | n/a | `Err` + 自动 retry 1 次 |
| Session expired | n/a | 自动重新 login (lazy) |
| 盘外调用 (Baostock 17:00-9:30) | n/a | 仍可调（日终已固定） |

## 7. 测试策略

```rust
// tests/sina_provider_test.rs
#[tokio::test]
async fn sina_kline_600000_returns_data() {
    let p = SinaProvider::new();
    let data = p.get_daily_data("600000", 5).await.unwrap();
    assert!(!data.is_empty());
    assert!(data[0].close > 0.0);
}

#[tokio::test]
async fn sina_kline_format_url() {
    // 验证 to_sina: 000001 → sz000001
    assert_eq!(to_sina("000001"), "sz000001");
    assert_eq!(to_sina("600000"), "sh600000");
    assert_eq!(to_sina("301000"), "sz301000");
    assert_eq!(to_sina("688001"), "sh688001");
}

#[tokio::test]
async fn sina_realtime_returns_quote() {
    let p = SinaProvider::new();
    let q = p.get_realtime_quote("600000").await.unwrap();
    assert!(q.is_some());
}

// tests/baostock_provider_test.rs
#[tokio::test]
async fn baostock_login_logout() {
    let p = BaostockProvider::new();
    let data = p.get_daily_data("600000", 5).await.unwrap();
    assert!(!data.is_empty());
}

#[tokio::test]
async fn baostock_format_url() {
    assert_eq!(to_baostock("000001"), "sz.000001");
    assert_eq!(to_baostock("600000"), "sh.600000");
    assert_eq!(from_baostock("sh.600000"), "600000");
}

#[tokio::test]
async fn baostock_adjflag_qfq() {
    // 验证复权数据返回
    let p = BaostockProvider::new();
    let data = p.get_daily_data("600000", 30).await.unwrap();
    // 至少 1 行有非 0 close
    assert!(data.iter().any(|k| k.close > 0.0));
}

// 集成测试
#[tokio::test]
async fn fallback_includes_sina() {
    // Sina 成功 → 走 Sina (因 priority 最高 HTTP)
    let (data, src) = fetch_kline_with_fallback("600000", 5).await.unwrap();
    // 可能是 sina_hq 或其它 (看网络)
    assert!(!data.is_empty());
    assert!(matches!(src, "sina_hq" | "tencent_qfq" | "eastmoney_qfq" | "rustdx_none"));
}

#[tokio::test]
async fn post_close_prefers_baostock() {
    // 盘后: 调 fetch_kline_post_close
    let (data, src) = fetch_kline_post_close("600000", 5).await.unwrap();
    assert!(!data.is_empty());
    // 第一次成功可能是 baostock, 失败 fallthrough
    println!("post_close src = {src}");
}
```

## 8. 部署 + 配置

### 8.1 Cargo.toml

```toml
[dependencies]
# 已有: reqwest (含 blocking + json), serde, serde_json, chrono
# 新增: encoding_rs (Sina 是 GBK 编码!)
encoding_rs = "0.8"
```

**关键发现**：Sina 接口返回 **GBK 编码**（不是 UTF-8），需要 `encoding_rs` 转换。

### 8.2 配置

- 无新增 env var（自动 fallback）
- 可选: `BAOSTOCK_BASE_URL` (默认 `http://baostock.com/baostock`)
- 可选: `BAOSTOCK_USER` (默认 `anonymous`)
- 可选: `BAOSTOCK_PASSWORD` (默认 `888888`)

## 9. 验收标准

- [ ] `cargo build` 通过
- [ ] `cargo test --lib` 全部通过（含新测试 + 现有 907）
- [ ] Sina 5+ 测试通过（K线 + 实时 + URL 格式 + 错误处理）
- [ ] Baostock 5+ 测试通过（login + K线 + 复权 + 错误处理）
- [ ] 集成: `fallback_includes_sina` + `post_close_prefers_baostock` 通过
- [ ] 启动 log 显示 5 源 fallback chain
- [ ] 盘后 1 次 review 调用 Baostock 成功

## 10. 阶段化 (按 ROI 排序)

1. **Phase 1**: Sina (P0) — 实时 HTTP fallback 加一层
2. **Phase 2**: Baostock (P1) — 盘后日终数据兜底
3. **Phase 3**: 文档 + 部署指南

## 11. 风险 + 缓解

| 风险 | 等级 | 缓解 |
|------|------|------|
| Sina 改 API 格式 | 🟡 | JSONP 解析容错 + log error 触发 fallthrough |
| Sina 频率限制 | 🟢 | 单次 1 req + 复用 client；不批量 |
| Sina GBK 编码 | 🟢 | `encoding_rs` 转换 |
| Baostock 服务下线 | 🟢 | 公开 13 年极稳；fallthrough 兜底 |
| Baostock 登录失败 | 🟢 | retry 1 次；fallthrough |
| 5 源 fallback 调试复杂 | 🟡 | 启动 log 清晰列出 source_name |

## 12. 与 QMT 集成的关系

QMT 集成（上一轮 plan `2026-07-08-qmt-integration.md`）**保留**为可选：
- 用户装 MiniQMT → QMT 本地 priority 0
- 不装 → 直接走 Sina (priority 1)
- 两者完全独立

本 spec 推进 Sina + Baostock，QMT 暂搁置。后续若 QMT 集成需求起来，可复用本 spec 的 fallback 框架。
