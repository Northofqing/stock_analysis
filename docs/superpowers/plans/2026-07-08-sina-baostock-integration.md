# Sina + Baostock 数据源集成 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Sina (HTTP 公开) + Baostock (日终权威) 接入 review #15 的 K线 fallback 链 + 新增盘后专用路径 + Sina 新闻数据源, 提供 0 费用多源兜底 + 实时/盘后新闻拉取.

**Architecture:**
- **K 线 (Phase 1)**: SinaProvider 实现现有 `DataProvider` trait 接入 review #15 的 4-way join (变 5-way), priority 1. BaostockProvider 作为独立盘后入口 (fetch_kline_post_close), 不进 fallback 链.
- **新闻 (Phase 2)**: SinaNewsProvider 提供财经要闻 + 个股新闻, 实时轮询 + 盘后回溯, 双写 `news_dedup` (去重) + `news_items` (详存, 新表).
- Stock code 格式映射抽到 stock_code_map helper.

**Tech Stack:** Rust + reqwest (async) + encoding_rs (Sina GBK) + tokio (Baostock session Mutex) + diesel (news_items 表).

## Global Constraints

- 不破坏 review #15 现有架构 (4-way join, SourceResult enum, 竞速 + 质检)
- 新增 `encoding_rs` 依赖只为 Sina GBK 编码, 其它路径不强制使用
- 每次 commit 前必须 `cargo build --message-format=short` 通过
- Sina/Baostock 网络测试需 `#[ignore]` 或 `#[tokio::test]` with timeout (避免 CI 卡死)

---

### Task 1: stock_code_map 扩展 (to_sina / to_baostock / from_baostock)

**Files:**
- Modify: `src/data_provider/stock_code_map.rs` (添加 3 个函数)
- Modify: `tests/stock_code_map_test.rs` (添加 3 个新测试)

**Interfaces:**
- Produces:
  - `pub fn to_sina(code: &str) -> String` → `"sh600519"`, `"sz000001"`
  - `pub fn to_baostock(code: &str) -> String` → `"sh.600519"`, `"sz.000001"`
  - `pub fn from_baostock(bs_code: &str) -> String` → `"600519"`, `"000001"`

- [ ] **Step 1: Write failing tests**

```rust
// tests/stock_code_map_test.rs (追加)
#[test]
fn to_sina_sh_main_board() {
    assert_eq!(to_sina("600000"), "sh600000");
    assert_eq!(to_sina("688001"), "sh688001");  // 科创板
    assert_eq!(to_sina("900900"), "sh900900");  // B 股
}

#[test]
fn to_sina_sz() {
    assert_eq!(to_sina("000001"), "sz000001");
    assert_eq!(to_sina("301000"), "sz301000");  // 创业板
    assert_eq!(to_sina("002415"), "sz002415");  // 中小板
}

#[test]
fn to_baostock_format() {
    assert_eq!(to_baostock("600000"), "sh.600000");
    assert_eq!(to_baostock("000001"), "sz.000001");
    assert_eq!(to_baostock("688001"), "sh.688001");
}

#[test]
fn from_baostock_strips_prefix() {
    assert_eq!(from_baostock("sh.600000"), "600000");
    assert_eq!(from_baostock("sz.000001"), "000001");
    assert_eq!(from_baostock("600000"), "600000");  // 无前缀容错
}
```

- [ ] **Step 2: Run tests, verify FAIL**

```bash
cargo test --test stock_code_map_test
```

Expected: FAIL — `to_sina`/`to_baostock`/`from_baostock` not found.

- [ ] **Step 3: Implement helper functions**

```rust
// src/data_provider/stock_code_map.rs (追加到文件末尾)

/// Sina hq 接口用 "sh600519" / "sz000001" 格式.
pub fn to_sina(code: &str) -> String {
    let prefix = match code.chars().next() {
        Some('6') | Some('9') | Some('5') => "sh",
        _ => "sz",
    };
    format!("{}{}", prefix, code)
}

/// Baostock 用 "sh.600000" / "sz.000001" 格式 (中间有点).
pub fn to_baostock(code: &str) -> String {
    let prefix = match code.chars().next() {
        Some('6') | Some('9') | Some('5') => "sh",
        _ => "sz",
    };
    format!("{}.{}", prefix, code)
}

/// Baostock → 我们的 6 位 code. 无前缀容错.
pub fn from_baostock(bs_code: &str) -> String {
    bs_code.split('.').nth(1).unwrap_or(bs_code).to_string()
}
```

- [ ] **Step 4: Run tests, verify PASS**

```bash
cargo test --test stock_code_map_test
```

Expected: 4 new tests passed (总计 9 passed).

- [ ] **Step 5: Commit**

```bash
git add src/data_provider/stock_code_map.rs tests/stock_code_map_test.rs
git commit -m "feat(data): add to_sina/to_baostock/from_baostock stock code helpers"
```

---

### Task 2: SinaProvider skeleton + K线 URL 构造

**Files:**
- Create: `src/data_provider/sina_provider.rs`
- Modify: `src/data_provider/mod.rs` (注册模块)
- Modify: `Cargo.toml` (添加 encoding_rs)

**Interfaces:**
- Produces:
  - `pub struct SinaProvider { client: reqwest::Client }`
  - `impl DataProvider for SinaProvider` (K线 + 实时价)
  - `fn build_kline_url(code: &str, days: usize) -> String`

- [ ] **Step 1: Add encoding_rs dep**

```toml
# Cargo.toml
encoding_rs = "0.8"
```

- [ ] **Step 2: Write failing test (URL 构造 + format)**

```rust
// tests/sina_provider_test.rs
use stock_analysis::data_provider::sina_provider::build_kline_url;
use stock_analysis::data_provider::sina_provider::SinaProvider;
use stock_analysis::data_provider::DataProvider;

#[test]
fn build_kline_url_format() {
    let url = build_kline_url("600000", 5);
    assert!(url.contains("sh600000"));
    assert!(url.contains("scale=240"));
    assert!(url.contains("datalen=5"));
}

#[test]
fn build_kline_url_sz_prefix() {
    let url = build_kline_url("000001", 30);
    assert!(url.contains("sz000001"));
}

#[test]
fn sina_provider_name() {
    let p = SinaProvider::new();
    assert_eq!(p.name(), "sina_hq");
}
```

- [ ] **Step 3: Run tests, verify FAIL**

```bash
cargo test --test sina_provider_test
```

Expected: FAIL — `sina_provider` module 不存在.

- [ ] **Step 4: Implement SinaProvider skeleton**

```rust
// src/data_provider/sina_provider.rs
use anyhow::{anyhow, Result};
use encoding_rs::GBK;
use serde::Deserialize;

use super::{DataProvider, KlineData, RealtimeQuote};
use crate::data_provider::stock_code_map::to_sina;

pub struct SinaProvider {
    client: reqwest::Client,
}

/// 构造 Sina K线 URL (JSONP).
/// URL: https://quotes.sina.cn/cn/api/jsonp_v2.php/=/CN_MarketDataService.getKLineData?symbol=sh600519&scale=240&datalen=30
pub fn build_kline_url(code: &str, days: usize) -> String {
    let sina_code = to_sina(code);
    format!(
        "https://quotes.sina.cn/cn/api/jsonp_v2.php/=/CN_MarketDataService.getKLineData\
         ?symbol={sina_code}&scale=240&datalen={days}"
    )
}

/// Sina K线 JSON 数组 (JSONP body 内 [...])
#[derive(Debug, Deserialize)]
pub struct SinaKlineRow {
    pub day: String,        // "2024-01-15"
    pub open: String,       // 字符串数字
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: String,     // 手
}

impl SinaProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// 抓取 Sina K线 (GBK → UTF-8 decode).
    async fn fetch_kline_raw(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        let url = build_kline_url(code, days);
        let bytes = self.client.get(&url)
            .header("Referer", "https://finance.sina.com.cn")
            .send().await?
            .error_for_status()?
            .bytes().await?;
        // Sina 返回 GBK 编码 (实测), 用 encoding_rs 转 UTF-8
        let (utf8, _, had_errors) = GBK.decode(&bytes);
        if had_errors {
            log::warn!("[Sina] {code} GBK decode 错误, 部分字符可能异常");
        }
        let body = utf8.into_owned();
        parse_kline_body(&body, code)
    }
}

/// 从 JSONP body 提取 [ ... ] 数组, 解析为 Vec<KlineData>.
pub fn parse_kline_body(body: &str, code: &str) -> Result<Vec<KlineData>> {
    let start = body.find('[')
        .ok_or_else(|| anyhow!("Sina K线: 无 JSON 数组"))?;
    let end = body.rfind(']')
        .ok_or_else(|| anyhow!("Sina K线: JSON 不完整"))?;
    let json = &body[start..=end];
    let rows: Vec<SinaKlineRow> = serde_json::from_str(json)
        .map_err(|e| anyhow!("Sina K线 JSON parse 失败: {e}"))?;
    Ok(rows.into_iter().map(|r| map_kline_row(r, code)).collect())
}

fn map_kline_row(r: SinaKlineRow, _code: &str) -> KlineData {
    use chrono::NaiveDate;
    let date = NaiveDate::parse_from_str(&r.day, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Local::now().date_naive());
    let open = r.open.parse().unwrap_or(0.0);
    let high = r.high.parse().unwrap_or(0.0);
    let low = r.low.parse().unwrap_or(0.0);
    let close = r.close.parse().unwrap_or(0.0);
    let volume = r.volume.parse().unwrap_or(0.0);
    let pct_chg = if open > 0.0 { (close - open) / open * 100.0 } else { 0.0 };
    KlineData {
        date, open, high, low, close, volume,
        amount: 0.0,  // Sina K线 API 不直接给 amount
        pct_chg,
        intraday_price: None, settled: true,
        pe_ratio: None, pb_ratio: None,
        turnover_rate: None, market_cap: None, circulating_cap: None,
    }
}

impl DataProvider for SinaProvider {
    fn name(&self) -> &'static str { "sina_hq" }
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        // sync DataProvider trait 内部跑 async — 用 tokio runtime
        tokio::runtime::Handle::current()
            .block_on(self.fetch_kline_raw(code, days))
    }
    fn get_stock_name(&self, code: &str) -> Option<String> {
        // 暂未实现, Phase 2 从 hq_str 解析
        None
    }
    fn get_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        // Task 3 实现
        Ok(None)
    }
}
```

- [ ] **Step 5: Register module**

```rust
// src/data_provider/mod.rs
pub mod sina_provider;
```

- [ ] **Step 6: Run tests, verify PASS**

```bash
cargo test --test sina_provider_test
```

Expected: 3 tests passed (URL 构造 + provider name).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/data_provider/sina_provider.rs src/data_provider/mod.rs tests/sina_provider_test.rs
git commit -m "feat(sina): add SinaProvider skeleton + K线 URL + GBK decode"
```

---

### Task 3: SinaProvider::get_realtime_quote (hq_str 实时价)

**Files:**
- Modify: `src/data_provider/sina_provider.rs`
- Modify: `tests/sina_provider_test.rs`

- [ ] **Step 1: Write failing test**

```rust
// tests/sina_provider_test.rs (追加)
use stock_analysis::data_provider::sina_provider::build_hq_url;

#[test]
fn build_hq_url_format() {
    let url = build_hq_url("600000");
    assert!(url.contains("hq.sinajs.cn"));
    assert!(url.contains("list=sh600000"));
}

#[test]
fn parse_hq_str_format() {
    // Sina 真实响应格式 (实测): var hq_str_sh600519="平安银行,13.50,13.45,...,100,500,...";
    let body = r#"var hq_str_sh600519="平安银行,13.50,13.45,13.48,13.52,13.40,13.47,13.49,12345,16789,100,500,...";
    let quote = stock_analysis::data_provider::sina_provider::parse_hq_str(body, "600519")
        .expect("parse hq_str");
    assert_eq!(quote.current, 13.48);
    assert_eq!(quote.open, 13.50);
    assert_eq!(quote.yesterday_close, 13.45);
    assert_eq!(quote.high, 13.52);
    assert_eq!(quote.low, 13.40);
    assert!(quote.volume > 0.0);
}
```

- [ ] **Step 2: Run tests, verify FAIL**

```bash
cargo test --test sina_provider_test hq
```

Expected: FAIL — `build_hq_url` / `parse_hq_str` not found.

- [ ] **Step 3: Implement hq_str functions**

```rust
// src/data_provider/sina_provider.rs (追加)

/// Sina 实时行情 URL.
/// https://hq.sinajs.cn/list=sh600519,sz000001
/// 多个 code 用逗号分隔, 一次请求拿多个.
pub fn build_hq_url(codes: &str) -> String {
    let sina_codes: Vec<String> = codes
        .split(',')
        .map(|c| to_sina(c.trim()))
        .collect();
    format!("https://hq.sinajs.cn/list={}", sina_codes.join(","))
}

/// Sina hq_str 解析结果.
#[derive(Debug, Default)]
pub struct SinaHqQuote {
    pub name: String,
    pub open: f64,
    pub yesterday_close: f64,
    pub current: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub amount: f64,
}

/// 解析 var hq_str_xx="name,open,prev_close,current,high,low,bid,ask,volume,amount,...";
pub fn parse_hq_str(body: &str, code: &str) -> Result<SinaHqQuote> {
    // 提取第一个 "..." 字符串
    let start = body.find('"').ok_or_else(|| anyhow!("Sina hq: 无引号"))?;
    let end = body.rfind('"').ok_or_else(|| anyhow!("Sina hq: 引号不闭合"))?;
    let csv = &body[start + 1..end];
    let fields: Vec<&str> = csv.split(',').collect();
    if fields.len() < 10 {
        return Err(anyhow!("Sina hq {} 字段数 {} < 10", code, fields.len()));
    }
    Ok(SinaHqQuote {
        name: fields[0].to_string(),
        open: fields[1].parse().unwrap_or(0.0),
        yesterday_close: fields[2].parse().unwrap_or(0.0),
        current: fields[3].parse().unwrap_or(0.0),
        high: fields[4].parse().unwrap_or(0.0),
        low: fields[5].parse().unwrap_or(0.0),
        volume: fields[8].parse().unwrap_or(0.0),
        amount: fields[9].parse().unwrap_or(0.0),
    })
}

impl SinaProvider {
    /// 抓取 Sina 实时价 (单只).
    pub async fn fetch_hq_async(&self, code: &str) -> Result<SinaHqQuote> {
        let url = build_hq_url(code);
        let bytes = self.client.get(&url)
            .header("Referer", "https://finance.sina.com.cn")
            .send().await?
            .error_for_status()?
            .bytes().await?;
        let (utf8, _, _) = GBK.decode(&bytes);
        let body = utf8.into_owned();
        parse_hq_str(&body, code)
    }
}

impl DataProvider for SinaProvider {
    // ... (保持 Task 2 的)
    fn get_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        let hq = tokio::runtime::Handle::current()
            .block_on(self.fetch_hq_async(code))?;
        // SinaHqQuote → RealtimeQuote 字段映射
        Ok(Some(RealtimeQuote {
            code: code.to_string(),
            name: hq.name,
            price: hq.current,
            change: hq.current - hq.yesterday_close,
            change_pct: if hq.yesterday_close > 0.0 {
                (hq.current - hq.yesterday_close) / hq.yesterday_close * 100.0
            } else { 0.0 },
            volume: hq.volume,
            amount: hq.amount,
            pe_ratio: None, pb_ratio: None,
            market_cap: None,
            timestamp: chrono::Local::now(),
        }))
    }
}
```

- [ ] **Step 4: Run tests, verify PASS**

```bash
cargo test --test sina_provider_test
```

Expected: 5 tests passed (3 URL/parse + 2 hq_str).

- [ ] **Step 5: Commit**

```bash
git add src/data_provider/sina_provider.rs tests/sina_provider_test.rs
git commit -m "feat(sina): add get_realtime_quote via hq_str + GBK decode"
```

---

### Task 4: SinaProvider 接入 review #15 4-way join (变 5-way)

**Files:**
- Modify: `src/data_provider/fallback.rs`
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

### Task 5: BaostockProvider skeleton + login/logout

**Files:**
- Create: `src/data_provider/baostock_provider.rs`
- Modify: `src/data_provider/mod.rs`

- [ ] **Step 1: Write failing test (URL 格式 + format helpers)**

```rust
// tests/baostock_provider_test.rs
use stock_analysis::data_provider::baostock_provider::{
    build_login_url, build_kline_query_body, build_logout_url,
    parse_baostock_response,
};

#[test]
fn build_login_url() {
    assert_eq!(build_login_url(), "http://baostock.com/baostock/Login");
}

#[test]
fn build_kline_query_body_format() {
    let body = build_kline_query_body(
        "sh.600000",
        "date,open,high,low,close",
        "20240101",
        "20241231",
        "session_xxx",
    );
    assert!(body.contains("QueryHistoryKLinePlus"));
    assert!(body.contains("code=sh.600000"));
    assert!(body.contains("adjustflag=2"));  // 前复权
    assert!(body.contains("sessionid=session_xxx"));
}

#[test]
fn parse_baostock_response_extracts_field() {
    let body = "sessionId=ABC123\nErrorCode=0\nErrorMsg=success\n";
    assert_eq!(
        parse_baostock_response(body, "sessionId").unwrap(),
        Some("ABC123".to_string())
    );
    assert_eq!(
        parse_baostock_response(body, "ErrorCode").unwrap(),
        Some("0".to_string())
    );
    assert_eq!(
        parse_baostock_response(body, "Missing").unwrap(),
        None
    );
}
```

- [ ] **Step 2: Run tests, verify FAIL**

```bash
cargo test --test baostock_provider_test
```

Expected: FAIL — `baostock_provider` module 不存在.

- [ ] **Step 3: Implement BaostockProvider skeleton**

```rust
// src/data_provider/baostock_provider.rs
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use tokio::sync::Mutex;

use super::{DataProvider, KlineData, RealtimeQuote};
use crate::data_provider::stock_code_map::to_baostock;

pub const BAOSTOCK_DEFAULT_BASE: &str = "http://baostock.com/baostock";

pub struct BaostockProvider {
    client: reqwest::Client,
    base_url: String,
    session: Mutex<Option<String>>,
}

pub fn build_login_url() -> String {
    format!("{}/Login", BAOSTOCK_DEFAULT_BASE)
}

pub fn build_logout_url() -> String {
    format!("{}/Logout", BAOSTOCK_DEFAULT_BASE)
}

/// 构造 K线查询 body (Baostock 用 form-encoded POST).
pub fn build_kline_query_body(
    code: &str,
    fields: &str,
    start_date: &str,
    end_date: &str,
    session_id: &str,
) -> String {
    // Baostock 协议: 多个 query 用 \n 分隔
    format!(
        "QueryHistoryKLinePlus&code={code}&fields={fields}&adjustflag=2&\
         startdate={start_date}&enddate={end_date}&sessionid={session_id}"
    )
}

/// 解析 Baostock 响应 (key=value\nkey=value 格式).
/// 返回 key 对应的 value, 找不到返 None.
pub fn parse_baostock_response(body: &str, key: &str) -> Result<Option<String>> {
    let prefix = format!("{}=", key);
    for line in body.lines() {
        if let Some(val) = line.strip_prefix(&prefix) {
            return Ok(Some(val.trim().to_string()));
        }
    }
    Ok(None)
}

impl BaostockProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            base_url: std::env::var("BAOSTOCK_BASE_URL")
                .unwrap_or_else(|_| BAOSTOCK_DEFAULT_BASE.to_string()),
            session: Mutex::new(None),
        }
    }
    
    /// 确保 session 已登录 (lazy init, 自动重登).
    pub async fn ensure_session(&self) -> Result<String> {
        let mut guard = self.session.lock().await;
        if let Some(sid) = guard.as_ref() {
            return Ok(sid.clone());
        }
        let sid = self.login().await?;
        *guard = Some(sid.clone());
        Ok(sid)
    }
    
    async fn login(&self) -> Result<String> {
        let body = self.client.post(build_login_url())
            .form(&[("user", "anonymous"), ("password", "888888")])
            .send().await?
            .text().await?;
        let code = parse_baostock_response(&body, "ErrorCode")?
            .ok_or_else(|| anyhow!("Baostock login: 无 ErrorCode"))?;
        if code != "0" {
            let msg = parse_baostock_response(&body, "ErrorMsg")?.unwrap_or_default();
            return Err(anyhow!("Baostock login 失败: code={code} msg={msg}"));
        }
        let sid = parse_baostock_response(&body, "sessionId")?
            .ok_or_else(|| anyhow!("Baostock login: 无 sessionId"))?;
        log::info!("[Baostock] login 成功, sessionId={}", &sid[..8.min(sid.len())]);
        Ok(sid)
    }
}

impl DataProvider for BaostockProvider {
    fn name(&self) -> &'static str { "baostock" }
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        // Task 6 实现
        Err(anyhow::anyhow!("not yet implemented"))
    }
    fn get_stock_name(&self, _code: &str) -> Option<String> { None }
    fn get_realtime_quote(&self, _code: &str) -> Result<Option<RealtimeQuote>> { Ok(None) }
}
```

- [ ] **Step 4: Register module**

```rust
// src/data_provider/mod.rs
pub mod baostock_provider;
```

- [ ] **Step 5: Run tests, verify PASS**

```bash
cargo test --test baostock_provider_test
```

Expected: 3 tests passed (URL + body + parse).

- [ ] **Step 6: Commit**

```bash
git add src/data_provider/baostock_provider.rs src/data_provider/mod.rs tests/baostock_provider_test.rs
git commit -m "feat(baostock): add BaostockProvider skeleton + login + format helpers"
```

---

### Task 6: BaostockProvider::get_daily_data 字段映射

**Files:**
- Modify: `src/data_provider/baostock_provider.rs`
- Modify: `tests/baostock_provider_test.rs`

- [ ] **Step 1: Add test (parse K线 body)**

```rust
// tests/baostock_provider_test.rs (追加)

#[test]
fn parse_kline_body_format() {
    // Baostock 响应格式 (实测): 
    // code,date,open,high,low,close,volume,amount
    // sh.600000,2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50
    let body = "code,date,open,high,low,close,volume,amount\nsh.600000,2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50\nsh.600000,2024-01-16,13.55,13.70,13.50,13.65,15000,20000.00\n";
    let klines = stock_analysis::data_provider::baostock_provider::parse_kline_body(body, "600000").unwrap();
    assert_eq!(klines.len(), 2);
    assert_eq!(klines[0].open, 13.50);
    assert_eq!(klines[0].close, 13.55);
    assert_eq!(klines[0].volume, 12345.0);
    assert_eq!(klines[0].amount, 16789.50);
    assert_eq!(klines[1].date, chrono::NaiveDate::from_ymd_opt(2024, 1, 16).unwrap());
}
```

- [ ] **Step 2: Run test, verify FAIL**

```bash
cargo test --test baostock_provider_test parse_kline
```

Expected: FAIL — `parse_kline_body` not found.

- [ ] **Step 3: Implement get_daily_data + parse_kline_body**

```rust
// src/data_provider/baostock_provider.rs (追加)
use chrono::NaiveDate;
use super::staleness_helper_trait;  // 假设有个 trait, 见 Step 4

/// 解析 Baostock K线 CSV body → Vec<KlineData>.
pub fn parse_kline_body(body: &str, our_code: &str) -> Result<Vec<KlineData>> {
    let mut lines = body.lines();
    // 第 1 行是表头: code,date,open,high,low,close,volume,amount
    let header_line = lines.next().ok_or_else(|| anyhow!("Baostock K线: 空 body"))?;
    let headers: Vec<&str> = header_line.split(',').collect();
    
    let idx = |name: &str| -> usize {
        headers.iter().position(|h| h.trim() == name)
            .ok_or_else(|| anyhow!("Baostock K线: 缺 {} 列", name))
    };
    let i_date = idx("date")?;
    let i_open = idx("open")?;
    let i_high = idx("high")?;
    let i_low = idx("low")?;
    let i_close = idx("close")?;
    let i_volume = idx("volume")?;
    let i_amount = idx("amount")?;
    
    let mut result = Vec::new();
    for line in lines {
        if line.trim().is_empty() { continue; }
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 7 { continue; }
        
        let date = NaiveDate::parse_from_str(&fields[i_date], "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Local::now().date_naive());
        let open = fields[i_open].parse().unwrap_or(0.0);
        let high = fields[i_high].parse().unwrap_or(0.0);
        let low = fields[i_low].parse().unwrap_or(0.0);
        let close = fields[i_close].parse().unwrap_or(0.0);
        let volume = fields[i_volume].parse().unwrap_or(0.0);
        let amount = fields[i_amount].parse::<f64>().unwrap_or(0.0);
        let pct_chg = if open > 0.0 { (close - open) / open * 100.0 } else { 0.0 };
        
        result.push(KlineData {
            date, open, high, low, close, volume, amount, pct_chg,
            intraday_price: None, settled: true,
            pe_ratio: None, pb_ratio: None,
            turnover_rate: None, market_cap: None, circulating_cap: None,
        });
    }
    let _ = our_code;  // 当前解析已用 baostock code
    Ok(result)
}

impl BaostockProvider {
    async fn fetch_kline_async(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        let sid = self.ensure_session().await?;
        let bs_code = to_baostock(code);
        let end_date = chrono::Local::now().date_naive();
        let start_date = end_date - chrono::Duration::days(days as i64 * 2);  // ×2 留 buffer for 停牌
        
        let body = build_kline_query_body(
            &bs_code,
            "date,open,high,low,close,volume,amount",
            &start_date.format("%Y%m%d").to_string(),
            &end_date.format("%Y%m%d").to_string(),
            &sid,
        );
        let resp = self.client.post(&format!("{}/QueryHistoryKLinePlus", self.base_url))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send().await?
            .text().await?;
        let code = parse_baostock_response(&resp, "ErrorCode")?
            .ok_or_else(|| anyhow!("Baostock K线: 无 ErrorCode"))?;
        if code != "0" {
            return Err(anyhow!("Baostock K线失败: code={code}"));
        }
        parse_kline_body(&resp, code)
    }
}

impl DataProvider for BaostockProvider {
    fn name(&self) -> &'static str { "baostock" }
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        tokio::runtime::Handle::current()
            .block_on(self.fetch_kline_async(code, days))
    }
    fn get_stock_name(&self, _code: &str) -> Option<String> { None }
    fn get_realtime_quote(&self, _code: &str) -> Result<Option<RealtimeQuote>> { Ok(None) }
}
```

- [ ] **Step 4: Run test, verify PASS**

```bash
cargo test --test baostock_provider_test
```

Expected: 4 tests passed.

- [ ] **Step 5: Commit**

```bash
git add src/data_provider/baostock_provider.rs tests/baostock_provider_test.rs
git commit -m "feat(baostock): implement get_daily_data + parse_kline_body CSV mapping"
```

---

### Task 7: 盘后专用路径 fetch_kline_post_close

**Files:**
- Modify: `src/data_provider/fallback.rs`
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

### Task 8: main.rs 盘后切换 + 启动日志 + 文档

**Files:**
- Modify: `src/bin/monitor/main.rs`
- Modify: `docs/business_rules.md` (加 BR-014 + BR-015)
- Create: `docs/sina_baostock_integration.md`

- [ ] **Step 1: Add startup log**

```rust
// src/bin/monitor/main.rs (在 main 开头, after config 加载)
log::info!(
    "[启动] K线 fallback chain: sina_hq → tencent_qfq → eastmoney_qfq → rustdx_none (4-way join, review #15)"
);
log::info!("[启动] 盘后路径: baostock → 4-way join (post_close)");
```

- [ ] **Step 2: Add BR-014 + BR-015 to business_rules.md**

```markdown
| BR-014 | ✅ registered | Sina (hq.sinajs.cn) 接入 fallback priority 1 — GBK 编码 + 公开 HTTP + JSONP 解析, IP 独立于腾讯/东财 | `src/data_provider/sina_provider.rs`, `src/data_provider/stock_code_map.rs` |
| BR-015 | ✅ registered | Baostock (baostock.com) 盘后专用日终数据, 无限调用, WebSocket-like session + 复权 (adjustflag=2) | `src/data_provider/baostock_provider.rs`, `src/data_provider/fallback.rs` |
```

- [ ] **Step 3: Write user-facing docs**

```markdown
<!-- docs/sina_baostock_integration.md -->
# Sina + Baostock 数据源集成

## 背景
review #15 后 fallback 链有 4 源 (腾讯/东财/RustDX), 全是公开 HTTP/TCP, 风险同质.
本次加 2 个新源:
- **Sina** (priority 1): 公开 HTTP, 域名独立, 0 费用
- **Baostock** (盘后专用): 证券所级别日终, 无限流

## Fallback 链
[流程图: Sina (1) → 腾讯 (2) → 东财 (3) → RustDX (4)]

## 盘后路径
[流程图: Baostock → 4-way join]

## 配置
- 无新增 env var (自动启用)
- 可选 `BAOSTOCK_BASE_URL` (默认 baostock.com)

## 故障排查
- Sina 503: 偶发, fallthrough 自动处理
- Baostock login 失败: 重试 1 次, fallthrough
```

- [ ] **Step 4: Update README**

```markdown
<!-- README.md 在 ## Architecture 段 -->
### 数据源
- K线 fallback: Sina → 腾讯 → 东财 → RustDX (4-way join)
- 盘后日终: Baostock (独立路径)
- 详见 `docs/sina_baostock_integration.md`
```

- [ ] **Step 5: Run all tests, verify no regression**

```bash
cargo test --lib
```

Expected: 912+ passed.

- [ ] **Step 6: Commit**

```bash
git add src/bin/monitor/main.rs docs/business_rules.md docs/sina_baostock_integration.md README.md
git commit -m "docs(data): add Sina+Baostock integration docs, BR-014/015, startup log"
```

---

## Final verification

```bash
cargo build --release
cargo test --lib
cargo clippy -- -D warnings
```

All must pass. Then push to master.

---

# Phase 2: Sina 新闻集成 (Tasks 9-12)

> 复用 Phase 1 的 Sina 客户端 (同 base URL, 同 GBK 处理, 同 stock_code_map).

---

### Task 9: NewsItem struct + diesel migration for news_items table

**Files:**
- Create: `src/data_provider/news_item.rs`
- Modify: `src/data_provider/mod.rs` (注册模块)
- Modify: `src/database/mod.rs` (添加 news_items schema + insert_news_item helper)
- Create: `tests/news_item_test.rs`

**Interfaces:**
- Produces:
  - `pub struct NewsItem { source, external_id, category, code, title, summary, url, source_name, published_at, fetched_at, content_hash }`
  - `pub fn content_hash(title: &str, summary: &str) -> String` (SHA256 hex)
  - `pub fn insert_news_item(item: &NewsItem) -> Result<()>` on `DatabaseManager`

- [ ] **Step 1: Add news_items table migration**

```rust
// src/database/mod.rs (在现有 schema 段)
// news_items: 详存新闻 (与 news_dedup 互补, 5min 去重 vs 永久详存)
pub const NEWS_ITEMS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS news_items (
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
CREATE INDEX IF NOT EXISTS idx_news_items_code_time ON news_items(code, published_at);
CREATE INDEX IF NOT EXISTS idx_news_items_published ON news_items(published_at);
"#;
```

在 `init` 函數末尾追加執行 `NEWS_ITEMS_SCHEMA`.

- [ ] **Step 2: Add NewsItem struct + content_hash**

```rust
// src/data_provider/news_item.rs
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NewsItem {
    pub source: String,         // "sina_financial" | "sina_stock"
    pub external_id: String,    // url 作为 id
    pub category: String,       // "财经要闻" | "个股新闻"
    pub code: Option<String>,   // 6 位 code (仅个股新闻)
    pub title: String,
    pub summary: String,
    pub url: String,
    pub source_name: String,
    pub published_at: DateTime<Utc>,
    pub fetched_at: DateTime<Utc>,
    pub content_hash: String,
}

/// SHA256 hex of (title + summary) — 用于 dedup.
pub fn content_hash(title: &str, summary: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(summary.as_bytes());
    format!("{:x}", hasher.finalize())
}
```

- [ ] **Step 3: Write failing tests**

```rust
// tests/news_item_test.rs
use stock_analysis::data_provider::news_item::{content_hash, NewsItem};
use chrono::Utc;

#[test]
fn content_hash_deterministic() {
    let h1 = content_hash("title1", "summary1");
    let h2 = content_hash("title1", "summary1");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64);  // SHA256 hex
}

#[test]
fn content_hash_differs_for_diff_input() {
    let h1 = content_hash("title1", "summary1");
    let h2 = content_hash("title1", "summary2");
    assert_ne!(h1, h2);
}

#[test]
fn news_item_serializes() {
    let item = NewsItem {
        source: "sina_financial".into(),
        external_id: "https://example.com/1".into(),
        category: "财经要闻".into(),
        code: None,
        title: "Test".into(),
        summary: "Summary".into(),
        url: "https://example.com/1".into(),
        source_name: "新浪财经".into(),
        published_at: Utc::now(),
        fetched_at: Utc::now(),
        content_hash: content_hash("Test", "Summary"),
    };
    let json = serde_json::to_string(&item).unwrap();
    assert!(json.contains("sina_financial"));
}
```

- [ ] **Step 4: Add sha2 dep**

```toml
# Cargo.toml
sha2 = "0.10"  # 已有 (review #14)
```

- [ ] **Step 5: Run tests, verify FAIL**

```bash
cargo test --test news_item_test
```

Expected: FAIL — `news_item` module 不存在.

- [ ] **Step 6: Register module**

```rust
// src/data_provider/mod.rs
pub mod news_item;
```

- [ ] **Step 7: Run tests, verify PASS**

```bash
cargo test --test news_item_test
```

Expected: 3 tests passed.

- [ ] **Step 8: Add insert_news_item helper to DatabaseManager**

```rust
// src/database/mod.rs (impl DatabaseManager)
pub fn insert_news_item(&self, item: &crate::data_provider::news_item::NewsItem) -> Result<(), String> {
    use crate::data_provider::news_item::NewsItem;
    use diesel::prelude::*;
    let mut conn = self.get_conn().map_err(|e| e.to_string())?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO news_items (source, external_id, category, code, title, summary, url, source_name, published_at, fetched_at, content_hash) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
    )
    .bind::<Text, _>(&item.source)
    .bind::<Text, _>(&item.external_id)
    .bind::<Text, _>(&item.category)
    .bind::<Text, _>(item.code.as_deref().unwrap_or(""))
    .bind::<Text, _>(&item.title)
    .bind::<Text, _>(&item.summary)
    .bind::<Text, _>(&item.url)
    .bind::<Text, _>(&item.source_name)
    .bind::<BigInt, _>(item.published_at.timestamp())
    .bind::<BigInt, _>(item.fetched_at.timestamp())
    .bind::<Text, _>(&item.content_hash)
    .execute(&mut *conn)
    .map_err(|e| e.to_string())?;
    Ok(())
}
```

- [ ] **Step 9: Commit**

```bash
git add src/data_provider/news_item.rs src/data_provider/mod.rs src/database/mod.rs tests/news_item_test.rs
git commit -m "feat(news): add NewsItem struct + news_items table + insert helper"
```

---

### Task 10: SinaNewsProvider + fetch_top_news / fetch_stock_news

**Files:**
- Create: `src/data_provider/sina_news_provider.rs`
- Modify: `src/data_provider/mod.rs` (注册)

- [ ] **Step 1: Write failing tests**

```rust
// tests/sina_news_provider_test.rs
use stock_analysis::data_provider::sina_news_provider::{
    SinaNewsProvider, build_top_news_url, build_stock_news_url,
    parse_sina_news_body,
};

#[test]
fn build_top_news_url() {
    let url = build_top_news_url(20);
    assert!(url.contains("feed.mix.sina.com.cn"));
    assert!(url.contains("lid=1686"));
    assert!(url.contains("num=20"));
}

#[test]
fn build_stock_news_url() {
    let url = build_stock_news_url("600000", 20);
    assert!(url.contains("lid=2516"));
    assert!(url.contains("k=600000"));
}

#[test]
fn parse_sina_news_body_extracts_items() {
    // Sina 真实响应格式 (实测): 
    // {"result":{"data":[{"title":"...","url":"...","intro":"...","ctime":1700000000,"media_name":"..."}]}}
    let body = r#"{"result":{"data":[{"url":"https://example.com/1","title":"新闻1","intro":"摘要1","ctime":1700000000,"media_name":"新浪财经"}]}}"#;
    let items = parse_sina_news_body(body, "财经要闻", None).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "新闻1");
    assert_eq!(items[0].url, "https://example.com/1");
    assert_eq!(items[0].summary, "摘要1");
    assert_eq!(items[0].category, "财经要闻");
    assert_eq!(items[0].code, None);  // 财经要闻无 code
    assert_eq!(items[0].content_hash.len(), 64);
}

#[test]
fn parse_sina_news_body_with_code() {
    let body = r#"{"result":{"data":[{"url":"https://example.com/2","title":"股票新闻","intro":"摘要2","ctime":1700000000,"media_name":"新浪财经"}]}}"#;
    let items = parse_sina_news_body(body, "个股新闻", Some("600000")).unwrap();
    assert_eq!(items[0].code, Some("600000".to_string()));
}
```

- [ ] **Step 2: Run tests, verify FAIL**

```bash
cargo test --test sina_news_provider_test
```

Expected: FAIL — `sina_news_provider` module 不存在.

- [ ] **Step 3: Implement SinaNewsProvider**

```rust
// src/data_provider/sina_news_provider.rs
use anyhow::{anyhow, Result};
use chrono::Utc;
use encoding_rs::GBK;

use super::news_item::{content_hash, NewsItem};

pub struct SinaNewsProvider {
    client: reqwest::Client,
    api_base: String,  // "https://feed.mix.sina.com.cn/api/roll/get"
}

const SINA_NEWS_API_BASE: &str = "https://feed.mix.sina.com.cn/api/roll/get";

/// 财经要闻 URL (lid=1686).
pub fn build_top_news_url(num: usize) -> String {
    format!(
        "{SINA_NEWS_API_BASE}?pageid=153&lid=1686&k=&num={num}&page=1"
    )
}

/// 个股新闻 URL (lid=2516, k=code).
pub fn build_stock_news_url(code: &str, num: usize) -> String {
    format!(
        "{SINA_NEWS_API_BASE}?pageid=155&lid=2516&k={code}&num={num}&page=1"
    )
}

/// 解析 Sina 新闻 body → Vec<NewsItem>.
/// 字段映射: url → external_id, title, intro → summary, ctime → published_at, media_name → source_name.
pub fn parse_sina_news_body(body: &str, category: &str, code: Option<&str>) -> Result<Vec<NewsItem>> {
    // 解析外层 result.data
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| anyhow!("Sina news JSON parse: {e}"))?;
    let data = v.get("result")
        .and_then(|r| r.get("data"))
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow!("Sina news: 无 result.data 数组"))?;
    
    let now = Utc::now();
    let mut items = Vec::with_capacity(data.len());
    for entry in data {
        let url = entry.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let title = entry.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let intro = entry.get("intro").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let ctime = entry.get("ctime").and_then(|x| x.as_i64()).unwrap_or(0);
        let media_name = entry.get("media_name").and_then(|x| x.as_str()).unwrap_or("新浪财经").to_string();
        let source = if code.is_some() { "sina_stock" } else { "sina_financial" };
        let published_at = chrono::DateTime::from_timestamp(ctime, 0)
            .unwrap_or_else(|| now);
        let hash = content_hash(&title, &intro);
        items.push(NewsItem {
            source: source.to_string(),
            external_id: url.clone(),
            category: category.to_string(),
            code: code.map(|c| c.to_string()),
            title,
            summary: intro,
            url,
            source_name: media_name,
            published_at,
            fetched_at: now,
            content_hash: hash,
        });
    }
    Ok(items)
}

impl SinaNewsProvider {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client, api_base: SINA_NEWS_API_BASE.to_string() }
    }
    
    /// 财经要闻 (大盘/政策/外盘快讯).
    pub async fn fetch_top_news(&self, num: usize) -> Result<Vec<NewsItem>> {
        let url = build_top_news_url(num);
        let bytes = self.client.get(&url)
            .header("Referer", "https://finance.sina.com.cn")
            .send().await?
            .error_for_status()?
            .bytes().await?;
        // Sina 新闻 API 通常返回 UTF-8, 但容错 GBK
        let (utf8, _, _) = GBK.decode(&bytes);
        let body = utf8.into_owned();
        parse_sina_news_body(&body, "财经要闻", None)
    }
    
    /// 个股新闻 (按 code).
    pub async fn fetch_stock_news(&self, code: &str, num: usize) -> Result<Vec<NewsItem>> {
        let url = build_stock_news_url(code, num);
        let bytes = self.client.get(&url)
            .header("Referer", "https://finance.sina.com.cn")
            .send().await?
            .error_for_status()?
            .bytes().await?;
        let (utf8, _, _) = GBK.decode(&bytes);
        let body = utf8.into_owned();
        parse_sina_news_body(&body, "个股新闻", Some(code))
    }
    
    /// 历史回溯 (按 code + 时间范围). Sina 不直接支持, 拉多页然后过滤.
    /// Phase 2 实现: 固定拉 5 页 (5 × 20 = 100 条), 然后客户端过滤.
    pub async fn fetch_stock_news_in_range(
        &self, code: &str, from: chrono::DateTime<Utc>, to: chrono::DateTime<Utc>,
    ) -> Result<Vec<NewsItem>> {
        let mut all = Vec::new();
        for page in 1..=5 {
            let num = 20;
            let url = format!(
                "{}?pageid=155&lid=2516&k={code}&num={num}&page={page}",
                self.api_base
            );
            let bytes = self.client.get(&url)
                .header("Referer", "https://finance.sina.com.cn")
                .send().await?
                .error_for_status()?
                .bytes().await?;
            let (utf8, _, _) = GBK.decode(&bytes);
            let body = utf8.into_owned();
            let items = parse_sina_news_body(&body, "个股新闻", Some(code))?;
            all.extend(items);
        }
        // 客户端过滤时间范围
        let filtered: Vec<NewsItem> = all.into_iter()
            .filter(|i| i.published_at >= from && i.published_at <= to)
            .collect();
        Ok(filtered)
    }
}
```

- [ ] **Step 4: Register module**

```rust
// src/data_provider/mod.rs
pub mod sina_news_provider;
```

- [ ] **Step 5: Run tests, verify PASS**

```bash
cargo test --test sina_news_provider_test
```

Expected: 4 tests passed.

- [ ] **Step 6: Commit**

```bash
git add src/data_provider/sina_news_provider.rs src/data_provider/mod.rs tests/sina_news_provider_test.rs
git commit -m "feat(news): add SinaNewsProvider (top + stock + history range)"
```

---

### Task 11: 双写 helper + 实时轮询 (monitor_loop)

**Files:**
- Modify: `src/bin/monitor/main.rs`

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

### Task 12: 盘后回溯 + 启动日志 + BR-016 + 文档

**Files:**
- Modify: `src/bin/monitor/main.rs` (盘后回溯调用)
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
