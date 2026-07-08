//! Baostock 数据源实现 (v13 计划).
//!
//! Baostock 是一个 A 股免费的 K 线 + 财务数据接口 (HTTP 文本协议).
//! 协议: 文本响应 (key=value\nkey=value 格式).
//!
//! 登录: `anonymous` / `888888` 默认 (公开访问, 无需注册).
//! K 线查询: `QueryHistoryKLinePlus`, 字段含 `adjustflag=2` (前复权).
//!
//! Task 5: 骨架 + login/logout + format helpers.
//! Task 6+: 真实 `get_daily_data` 实现 + 集成到 fallback chain.

use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use tokio::sync::Mutex;

use super::stock_code_map::to_baostock;
use super::{AdjustType, DataProvider, KlineData, RealtimeQuote};

/// Baostock 默认 base URL (公开 endpoint).
pub const BAOSTOCK_DEFAULT_BASE: &str = "http://baostock.com/baostock";

/// Baostock 数据源.
///
/// 内部用 `Mutex<Option<String>>` 缓存 session id, 首次 `ensure_session`
/// 触发懒登录; 后续调用复用同一 session.
///
/// `pub(crate)` 字段允许测试和同 crate 其他模块在必要时访问.
pub struct BaostockProvider {
    pub(crate) client: reqwest::Client,
    /// Skeleton 阶段暂未使用 (Task 6 实际查询时用此 URL). 保留供后续扩展.
    #[allow(dead_code)]
    pub(crate) base_url: String,
    pub(crate) session: Mutex<Option<String>>,
}

/// 构造登录 URL.
pub fn build_login_url() -> String {
    format!("{}/Login", BAOSTOCK_DEFAULT_BASE)
}

/// 构造登出 URL.
pub fn build_logout_url() -> String {
    format!("{}/Logout", BAOSTOCK_DEFAULT_BASE)
}

/// 构造 K线查询 body (Baostock 用 form-encoded POST).
///
/// 协议: `QueryHistoryKLinePlus&code=...&fields=...&adjustflag=2&...`
/// `adjustflag=2` 表示前复权 (qfq).
pub fn build_kline_query_body(
    code: &str,
    fields: &str,
    start_date: &str,
    end_date: &str,
    session_id: &str,
) -> String {
    format!(
        "QueryHistoryKLinePlus&code={code}&fields={fields}&adjustflag=2&\
         startdate={start_date}&enddate={end_date}&sessionid={session_id}"
    )
}

/// 解析 Baostock 响应 (key=value\nkey=value 格式).
///
/// 返回 `key` 对应的 value (首匹配, trim). 找不到返 `None`.
/// 不会因为某个 key 不存在而返回 Err.
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
    /// 创建新实例. 优先用 `BAOSTOCK_BASE_URL` env 覆盖默认 base.
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

    /// 确保 session 已登录 (懒初始化, 命中缓存直接返回).
    ///
    /// 第一次调用触发 `login`; 之后复用 session id 直到进程退出.
    /// ⚠️ Baostock session 有时效 (实测 ~1h), 后续 Task 会加过期重登.
    pub async fn ensure_session(&self) -> Result<String> {
        let mut guard = self.session.lock().await;
        if let Some(sid) = guard.as_ref() {
            return Ok(sid.clone());
        }
        let sid = self.login().await?;
        *guard = Some(sid.clone());
        Ok(sid)
    }

    /// 真实登录请求 (POST `{base}/Login` form-encoded).
    ///
    /// 默认凭据: `anonymous` / `888888` (公开访问).
    /// 期望响应: `sessionId=XXX\nErrorCode=0\nErrorMsg=...`
    pub(crate) async fn login(&self) -> Result<String> {
        let body = self
            .client
            .post(build_login_url())
            .form(&[("user", "anonymous"), ("password", "888888")])
            .send()
            .await?
            .text()
            .await?;
        let code = parse_baostock_response(&body, "ErrorCode")?
            .ok_or_else(|| anyhow!("Baostock login: 无 ErrorCode"))?;
        if code != "0" {
            let msg = parse_baostock_response(&body, "ErrorMsg")?.unwrap_or_default();
            return Err(anyhow!("Baostock login 失败: code={code} msg={msg}"));
        }
        let sid = parse_baostock_response(&body, "sessionId")?
            .ok_or_else(|| anyhow!("Baostock login: 无 sessionId"))?;
        log::info!(
            "[Baostock] login 成功, sessionId={}",
            &sid[..8.min(sid.len())]
        );
        Ok(sid)
    }

    /// 登出 (POST `{base}/Logout` 含 sessionid).
    ///
    /// 错误日志但不抛 Err — 进程退出时调用, 不希望被登出失败阻塞.
    pub async fn logout(&self, session_id: &str) {
        let body = format!("sessionid={session_id}");
        match self
            .client
            .post(build_logout_url())
            .body(body)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(t) = resp.text().await {
                    log::info!("[Baostock] logout 响应: {t}");
                }
            }
            Err(e) => {
                log::warn!("[Baostock] logout 失败: {e}");
            }
        }
    }

    /// 异步拉取 K 线 (内部 helper, 由 `get_daily_data` sync 入口包装).
    ///
    /// 协议: `POST {base}/QueryHistoryKLinePlus` (form-encoded).
    /// `adjustflag=2` (前复权) 已在 `build_kline_query_body` 写死.
    /// 起始日期用 `days * 2` 留 buffer (含停牌日).
    pub async fn fetch_kline_async(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        let sid = self.ensure_session().await?;
        let bs_code = to_baostock(code);
        let end_date = chrono::Local::now().date_naive();
        let start_date = end_date - chrono::Duration::days(days as i64 * 2);

        let body = build_kline_query_body(
            &bs_code,
            "date,open,high,low,close,volume,amount",
            &start_date.format("%Y%m%d").to_string(),
            &end_date.format("%Y%m%d").to_string(),
            &sid,
        );
        let resp = self
            .client
            .post(&format!("{}/QueryHistoryKLinePlus", self.base_url))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await?
            .text()
            .await?;
        let error_code = parse_baostock_response(&resp, "ErrorCode")?
            .ok_or_else(|| anyhow!("Baostock K线: 无 ErrorCode"))?;
        if error_code != "0" {
            let msg = parse_baostock_response(&resp, "ErrorMsg")?.unwrap_or_default();
            return Err(anyhow!("Baostock K线失败: code={error_code} msg={msg}"));
        }
        parse_kline_body(&resp, code)
    }
}

/// 解析 Baostock K线 CSV body → `Vec<KlineData>`.
///
/// 输入格式 (实测):
/// ```text
/// code,date,open,high,low,close,volume,amount
/// sh.600000,2024-01-15,13.50,13.60,13.45,13.55,12345,16789.50
/// ```
///
/// - 第 1 行是表头, 后续每行 1 条 K线.
/// - `date` 格式 `"YYYY-MM-DD"`.
/// - 解析失败的字段回退为 0.0 / 今天 (不抛 Err, 保证解析容错).
pub fn parse_kline_body(body: &str, _our_code: &str) -> Result<Vec<KlineData>> {
    let mut lines = body.lines();
    let header_line = lines
        .next()
        .ok_or_else(|| anyhow!("Baostock K线: 空 body"))?;
    let headers: Vec<&str> = header_line.split(',').collect();

    let idx = |name: &str| -> Result<usize> {
        headers
            .iter()
            .position(|h| h.trim() == name)
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
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 7 {
            continue;
        }

        let date = NaiveDate::parse_from_str(fields[i_date], "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Local::now().date_naive());
        let open: f64 = fields[i_open].parse().unwrap_or(0.0);
        let high: f64 = fields[i_high].parse().unwrap_or(0.0);
        let low: f64 = fields[i_low].parse().unwrap_or(0.0);
        let close: f64 = fields[i_close].parse().unwrap_or(0.0);
        let volume: f64 = fields[i_volume].parse().unwrap_or(0.0);
        let amount: f64 = fields[i_amount].parse().unwrap_or(0.0);
        let pct_chg = if open > 0.0 {
            (close - open) / open * 100.0
        } else {
            0.0
        };

        result.push(KlineData {
            date,
            open,
            high,
            low,
            close,
            volume,
            amount,
            pct_chg,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            // Baostock QueryHistoryKLinePlus 默认 adjustflag=2 (前复权), 在 build_kline_query_body 已固定.
            adjust: AdjustType::Qfq,
        });
    }
    let _ = _our_code; // 当前解析已用 baostock code, 保留参数供未来扩展 (回填本地 code).
    Ok(result)
}

impl Default for BaostockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl DataProvider for BaostockProvider {
    fn name(&self) -> &'static str {
        "baostock"
    }
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        crate::block_on_async(self.fetch_kline_async(code, days))
    }
    fn get_stock_name(&self, _code: &str) -> Option<String> {
        None
    }
    fn get_realtime_quote(&self, _code: &str) -> Result<Option<RealtimeQuote>> {
        Ok(None)
    }
}

#[cfg(test)]
mod inline_tests {
    use super::*;

    /// `BAOSTOCK_DEFAULT_BASE` 必须是稳定的公开 endpoint — 不可变.
    /// 多个测试 (URL 拼接 / 集成测试) 依赖此字符串.
    #[test]
    fn default_base_is_stable() {
        assert_eq!(BAOSTOCK_DEFAULT_BASE, "http://baostock.com/baostock");
    }

    /// `parse_baostock_response` 容忍尾部空白和 `\r\n` 行结束符 (Windows 风格响应).
    #[test]
    fn parse_handles_crlf_and_trailing_whitespace() {
        let body = "sessionId=XYZ \r\nErrorCode=0\r\n";
        assert_eq!(
            parse_baostock_response(body, "sessionId").unwrap(),
            Some("XYZ".to_string())
        );
    }

    /// `parse_baostock_response` 前缀匹配是贪婪的, "sessionIdPrefix" 不会误匹配 "sessionId".
    /// 这是一个防护性测试: Baostock 响应只应有标准 key, 不应该有前缀冲突.
    #[test]
    fn parse_prefix_is_exact() {
        let body = "sessionIdPrefix=wrong\nsessionId=right";
        assert_eq!(
            parse_baostock_response(body, "sessionId").unwrap(),
            Some("right".to_string())
        );
    }
}
