wrote /Users/zhangzhen/Desktop/Quant/stock_analysis/.superpowers/sdd/task-5-brief.md: 193 lines
ostock_provider.rs`
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

