//! Sina 财经数据提供者 (骨架版, Task 2)
//!
//! 通过 Sina JSONP 接口抓取日 K 线数据.
//! Sina 接口返回 GBK 编码, 用 `encoding_rs` 转 UTF-8.
//!
//! 实时行情 (hq_str) 与股票名解析在后续 Task 中实现.

use anyhow::{anyhow, Result};
use encoding_rs::GBK;
use serde::Deserialize;

use super::{DataProvider, KlineData, RealtimeQuote};
use crate::data_provider::stock_code_map::to_sina;

/// Sina 数据提供者
pub struct SinaProvider {
    client: reqwest::Client,
}

/// 构造 Sina K线 URL (JSONP).
///
/// Sina 接口格式:
/// `https://quotes.sina.cn/cn/api/jsonp_v2.php/=/CN_MarketDataService.getKLineData?symbol=sh600519&scale=240&datalen=30`
///
/// - `scale=240` → 日 K 线 (240 分钟对应一个交易日)
/// - `datalen` → 返回最近 N 条
pub fn build_kline_url(code: &str, days: usize) -> String {
    let sina_code = to_sina(code);
    format!(
        "https://quotes.sina.cn/cn/api/jsonp_v2.php/=/CN_MarketDataService.getKLineData\
         ?symbol={sina_code}&scale=240&datalen={days}"
    )
}

/// Sina K线 JSON 数组中的一条 (JSONP body 内的 `[ ... ]` 元素).
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
    /// 创建新的 SinaProvider, 10s 超时, 简单 UA.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    /// 抓取 Sina K线 (GBK → UTF-8 decode).
    pub async fn fetch_kline_raw(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        let url = build_kline_url(code, days);
        let bytes = self.client
            .get(&url)
            .header("Referer", "https://finance.sina.com.cn")
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        // Sina 返回 GBK 编码 (实测), 用 encoding_rs 转 UTF-8
        let (utf8, _, had_errors) = GBK.decode(&bytes);
        if had_errors {
            log::warn!("[Sina] {code} GBK decode 错误, 部分字符可能异常");
        }
        let body = utf8.into_owned();
        parse_kline_body(&body, code)
    }
}

/// 从 JSONP body 提取 `[ ... ]` 数组, 解析为 `Vec<KlineData>`.
pub fn parse_kline_body(body: &str, code: &str) -> Result<Vec<KlineData>> {
    let start = body
        .find('[')
        .ok_or_else(|| anyhow!("Sina K线: 无 JSON 数组"))?;
    let end = body
        .rfind(']')
        .ok_or_else(|| anyhow!("Sina K线: JSON 不完整"))?;
    let json = &body[start..=end];
    let rows: Vec<SinaKlineRow> = serde_json::from_str(json)
        .map_err(|e| anyhow!("Sina K线 JSON parse 失败: {e}"))?;
    Ok(rows.into_iter().map(|r| map_kline_row(r, code)).collect())
}

/// 将单条 Sina K线行映射到标准 `KlineData` 结构.
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
        adjust: super::AdjustType::None,
    }
}

impl DataProvider for SinaProvider {
    fn name(&self) -> &'static str {
        "sina_hq"
    }

    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        // sync DataProvider trait 内部跑 async — 用 crate 共享 helper
        // (避免 Handle::current() 在 current_thread runtime 里 panic)
        crate::block_on_async(self.fetch_kline_raw(code, days))
    }

    fn get_stock_name(&self, _code: &str) -> Option<String> {
        // 暂未实现, Phase 2 从 hq_str 解析
        None
    }

    fn get_realtime_quote(&self, _code: &str) -> Result<Option<RealtimeQuote>> {
        // Task 3 实现
        Ok(None)
    }
}

impl Default for SinaProvider {
    fn default() -> Self {
        Self::new()
    }
}
