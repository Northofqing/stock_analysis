//! RustDX 数据提供者
//!
//! 使用 rustdx-complete 库从通达信服务器获取股票数据
//! 优点：直连通达信公共服务器，速度快，数据准确，无需额外配置

use super::gtimg_provider::GtimgProvider;
use super::{DataProvider, KlineData, RealtimeQuote};
use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use log::{debug, error, info, warn};
use rustdx_complete::tcp::stock::Kline;
use rustdx_complete::tcp::{Tcp, Tdx};

/// RustDX 数据提供者
///
/// 使用通达信TCP协议直接获取数据，特点：
/// - 速度快：直连服务器，低延迟
/// - 准确：通达信官方数据源
/// - 免费：无需API密钥
/// - 稳定：公共服务器 115.238.56.198:7709
///
/// 注意：盈利指标（PE、PB等）通过腾讯财经API补充
pub struct RustdxProvider {
    gtimg_provider: GtimgProvider,
}

#[derive(Debug, Clone)]
struct RustdxBarInput {
    year: i32,
    month: u32,
    day: u32,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    amount: f64,
}

impl RustdxProvider {
    /// 创建新的 RustDxProvider 实例
    pub fn new() -> Result<Self> {
        info!("[通达信] 初始化 RustDX 数据提供者");
        let gtimg_provider = GtimgProvider::new()?;
        Ok(Self { gtimg_provider })
    }

    /// 创建新的 TCP 连接
    fn new_connection() -> Result<Tcp> {
        debug!("[通达信] 创建新的TCP连接");
        Tcp::new().context("无法连接到通达信服务器")
    }

    /// 转换股票代码格式
    /// 600000 -> 1 (上海)
    /// 000001 -> 0 (深圳)
    fn parse_market(code: &str) -> u8 {
        if code.starts_with('6') {
            1 // 上海
        } else {
            0 // 深圳/创业板
        }
    }

    /// 规范化股票代码（补全为6位）
    fn normalize_code(code: &str) -> Result<String> {
        // 移除空格和特殊字符
        let code = code.trim();

        // 检查是否为纯数字
        if !code.chars().all(|c| c.is_ascii_digit()) {
            return Err(anyhow!("股票代码 {} 包含非数字字符", code));
        }

        // 检查长度
        if code.len() > 6 {
            return Err(anyhow!("股票代码 {} 长度超过6位", code));
        }

        if code.is_empty() {
            return Err(anyhow!("股票代码为空"));
        }

        // 补全为6位（前面补0）
        let normalized = format!("{:0>6}", code);

        debug!("[通达信] 代码规范化: {} -> {}", code, normalized);

        Ok(normalized)
    }

    /// 获取K线数据（内部方法）
    fn fetch_kline_internal(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        // 规范化股票代码
        let code = Self::normalize_code(code)?;

        let market = Self::parse_market(&code) as u16;

        // 通达信每次请求最多返回约 800 条K线，需要分页获取
        const BATCH_SIZE: u16 = 800;
        let mut all_bars = Vec::new();
        let mut offset: u16 = 0;
        let remaining = days;

        loop {
            let count = BATCH_SIZE.min((remaining - all_bars.len()) as u16);
            if count == 0 {
                break;
            }

            let mut tcp = Self::new_connection()?;
            let recv_result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Vec<_>> {
                    let mut kline = Kline::new(market, &code, 9, offset, count);
                    kline.recv_parsed(&mut tcp).map_err(|e| anyhow!("{}", e))?;
                    Ok(kline.result().to_vec())
                }));

            match recv_result {
                Ok(Ok(data)) => {
                    let fetched = data.len();
                    if fetched == 0 {
                        break; // 服务器无更多数据
                    }
                    all_bars.extend(data);
                    offset += fetched as u16;
                    debug!(
                        "[通达信] {} 分页获取: offset={}, 本次={}, 累计={}",
                        code,
                        offset,
                        fetched,
                        all_bars.len()
                    );
                    if fetched < count as usize {
                        break; // 已获取全部可用数据
                    }
                    if all_bars.len() >= remaining {
                        break;
                    }
                }
                Ok(Err(e)) => {
                    return Err(anyhow!(
                        "获取股票 {} K线第 {} 页失败（已取 {} 条，整批拒绝）: {}",
                        code,
                        u32::from(offset) / u32::from(BATCH_SIZE) + 1,
                        all_bars.len(),
                        e
                    ));
                }
                Err(_) => {
                    return Err(anyhow!(
                        "获取股票 {} K线第 {} 页时底层库 panic（已取 {} 条，整批拒绝）",
                        code,
                        u32::from(offset) / u32::from(BATCH_SIZE) + 1,
                        all_bars.len()
                    ));
                }
            }
        }

        if all_bars.is_empty() {
            return Err(anyhow!("股票 {} 没有返回K线数据", code));
        }

        let raw_bars: Vec<RustdxBarInput> = all_bars
            .iter()
            .map(|bar| RustdxBarInput {
                year: bar.dt.year as i32,
                month: bar.dt.month as u32,
                day: bar.dt.day as u32,
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                volume: bar.vol,
                amount: bar.amount,
            })
            .collect();
        let kline_data = Self::parse_kline_batch(&code, &raw_bars)?;

        info!("[通达信] {} 成功获取 {} 条K线数据", code, kline_data.len());

        Ok(kline_data)
    }

    /// BR-092: decode a complete RustDX batch, calculate real adjacent returns,
    /// then apply the shared OHLCV/date/jump validation before any computation.
    fn parse_kline_batch(code: &str, bars: &[RustdxBarInput]) -> Result<Vec<KlineData>> {
        if bars.is_empty() {
            return Err(anyhow!("股票 {} 没有返回K线数据", code));
        }
        let mut kline_data: Vec<KlineData> = bars
            .iter()
            .enumerate()
            .map(|(index, bar)| {
                let date =
                    NaiveDate::from_ymd_opt(bar.year, bar.month, bar.day).ok_or_else(|| {
                        anyhow!(
                            "通达信 {} 第 {} 行日期非法: year={} month={} day={}（整批拒绝）",
                            code,
                            index + 1,
                            bar.year,
                            bar.month,
                            bar.day
                        )
                    })?;
                Ok(KlineData {
                    date,
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    volume: bar.volume,
                    amount: bar.amount,
                    pct_chg: 0.0,
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
                    adjust: crate::data_provider::AdjustType::None,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        kline_data.sort_by_key(|item| item.date);
        for i in 1..kline_data.len() {
            let prev_close = kline_data[i - 1].close;
            if prev_close > 0.0 {
                kline_data[i].pct_chg = ((kline_data[i].close - prev_close) / prev_close) * 100.0;
            }
        }

        super::validate_kline_series_strict(&mut kline_data, code)?;
        Ok(kline_data)
    }

    /// 获取实时行情（内部方法）
    fn fetch_realtime_internal(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        // rustdx-complete 1.0.0 decodes the realtime timestamp as zero. It
        // therefore cannot satisfy BR-097, so use the real Tencent source that
        // carries an upstream-observed timestamp.
        self.gtimg_provider.fetch_realtime_quote(code)
    }
}

impl DataProvider for RustdxProvider {
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        info!("[通达信] 获取股票 {} 最近 {} 天数据", code, days);

        let mut kline_data = self.fetch_kline_internal(code, days)?;

        // v11-P0-3 commit 2: K 线缺口推断 → 喂入 HALTED_PERIODS, 供 is_halted_period 查询
        super::halt_status::infer_halt_from_kline_gaps(code, &kline_data);

        // 填涨跌停 / 停牌标记（在夏普和实时报价前先填，因为夏普的 close 改写可能影响）
        super::limit_status::apply_limit_flags_inplace(code, None, &mut kline_data);

        // 计算夏普比率（使用60天滚动窗口）
        if !kline_data.is_empty() {
            use crate::sharpe_calculator;

            // 数据是降序的（最新在前），反转来计算，原地反转避免 clone
            kline_data.reverse();
            sharpe_calculator::update_sharpe_ratios(&mut kline_data, Some(60), Some(0.03));
            kline_data.reverse();

            if let Some(latest) = kline_data.first() {
                if let Some(sharpe) = latest.sharpe_ratio {
                    debug!("[通达信] {} 夏普比率: {:.4}", code, sharpe);
                }
            }
        }

        // 尝试从腾讯财经获取实时行情补充盈利指标
        // 因为通达信不提供PE、PB、换手率、市值等财务指标
        if !kline_data.is_empty() {
            info!("[通达信] 尝试从腾讯财经补充盈利指标");
            match self.gtimg_provider.fetch_realtime_quote(code) {
                Ok(Some(quote)) => {
                    if let Some(latest) = kline_data.first_mut() {
                        // 修复 P1.8: 不再覆盖 latest.close (会污染 Sharpe 计算)
                        // 之前: latest.close = quote.price (盘中价覆盖 settled close)
                        //       → sharpe_calculator 用盘中价当 settled close 计算
                        //       → 60 日 Sharpe 实际是盘中波动率, 不是日线收益
                        // 现在: 盘中价存在 intraday_price, close 保持日线 settled
                        // 标记 settled=false (盘中期)
                        let old_close = latest.close;
                        latest.intraday_price = Some(quote.price);
                        latest.settled = false; // 盘中期, 不是 settled close

                        // 补充盈利指标
                        latest.pe_ratio = latest.pe_ratio.or(quote.pe_ratio);
                        latest.pb_ratio = latest.pb_ratio.or(quote.pb_ratio);
                        latest.turnover_rate = latest.turnover_rate.or(quote.turnover_rate);
                        latest.market_cap = latest.market_cap.or(quote.market_cap);
                        latest.circulating_cap = latest.circulating_cap.or(quote.circulating_cap);

                        info!("[通达信+腾讯] {} 价格: close={:.2}元 (盘中={:.2}元), PE={:?}, PB={:?}, 换手率={:?}%, 总市值={:?}亿, 流通市值={:?}亿",
                            code, old_close, quote.price, quote.pe_ratio, quote.pb_ratio,
                            quote.turnover_rate, quote.market_cap, quote.circulating_cap);
                    }
                }
                Err(e) => {
                    warn!("[通达信] 无法从腾讯财经获取 {} 的盈利指标: {}", code, e);
                }
                Ok(None) => {
                    warn!("[通达信] 腾讯财经未返回 {} 的盈利指标", code);
                }
            }
        }

        Ok(kline_data)
    }

    fn get_stock_name(&self, _code: &str) -> Option<String> {
        // 通达信返回的股票名称经常为空，所以这里返回None
        // 让系统使用腾讯财经等其他数据源获取名称
        debug!("[通达信] 股票名称功能已禁用，请使用其他数据源");
        None
    }

    fn get_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        self.fetch_realtime_internal(code)
    }

    fn name(&self) -> &'static str {
        "通达信"
    }
}

impl Default for RustdxProvider {
    fn default() -> Self {
        Self::new().unwrap_or_else(|e| {
            error!("[通达信] 初始化失败: {}", e);
            panic!("无法初始化通达信数据提供者");
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_code_and_market_without_network() {
        assert_eq!(RustdxProvider::normalize_code("1").unwrap(), "000001");
        assert_eq!(
            RustdxProvider::normalize_code(" 600000 ").unwrap(),
            "600000"
        );
        assert_eq!(RustdxProvider::parse_market("600000"), 1);
        assert_eq!(RustdxProvider::parse_market("000001"), 0);
        assert!(RustdxProvider::normalize_code("").is_err());
        assert!(RustdxProvider::normalize_code("600000.SH").is_err());
        assert!(RustdxProvider::normalize_code("1234567").is_err());
    }

    fn raw(year: i32, month: u32, day: u32, close: f64) -> RustdxBarInput {
        RustdxBarInput {
            year,
            month,
            day,
            open: close,
            high: close + 0.2,
            low: close - 0.2,
            close,
            volume: 1_000.0,
            amount: close * 1_000.0,
        }
    }

    #[test]
    fn br092_complete_rustdx_batch_is_strict_and_newest_first() {
        let batch = vec![raw(2026, 7, 17, 11.0), raw(2026, 7, 16, 10.0)];
        let parsed = RustdxProvider::parse_kline_batch("TEST_CODE_000001", &batch)
            .expect("complete RustDX batch");
        assert_eq!(
            parsed.iter().map(|bar| bar.date).collect::<Vec<_>>(),
            [
                NaiveDate::from_ymd_opt(2026, 7, 17).unwrap(),
                NaiveDate::from_ymd_opt(2026, 7, 16).unwrap(),
            ]
        );
        assert!((parsed[0].pct_chg - 10.0).abs() < 1e-9);
        assert_eq!(parsed[0].adjust, crate::data_provider::AdjustType::None);
        assert!(parsed.iter().all(|bar| bar.settled));
    }

    #[test]
    fn br092_rustdx_parser_rejects_incomplete_or_bad_batches() {
        assert!(RustdxProvider::parse_kline_batch("TEST_CODE_000001", &[]).is_err());

        let mut invalid_date = raw(2026, 2, 30, 10.0);
        assert!(
            RustdxProvider::parse_kline_batch("TEST_CODE_000001", &[invalid_date.clone()]).is_err()
        );

        invalid_date.year = 2026;
        invalid_date.month = 7;
        invalid_date.day = 16;
        invalid_date.high = 9.0;
        assert!(
            RustdxProvider::parse_kline_batch("TEST_CODE_000001", &[invalid_date.clone()]).is_err()
        );

        let duplicate = raw(2026, 7, 16, 10.1);
        assert!(RustdxProvider::parse_kline_batch(
            "TEST_CODE_000001",
            &[raw(2026, 7, 16, 10.0), duplicate],
        )
        .is_err());
        assert!(RustdxProvider::parse_kline_batch(
            "TEST_CODE_000001",
            &[raw(2026, 7, 16, 10.0), raw(2026, 7, 20, 10.1)],
        )
        .is_err());
        assert!(RustdxProvider::parse_kline_batch(
            "TEST_CODE_000001",
            &[raw(2026, 7, 16, 10.0), raw(2026, 7, 17, 13.0)],
        )
        .is_err());

        let mut bad_amount = raw(2026, 7, 16, 10.0);
        bad_amount.amount = f64::NAN;
        assert!(RustdxProvider::parse_kline_batch("TEST_CODE_000001", &[bad_amount]).is_err());
    }

    #[test]
    fn zero_day_request_fails_before_opening_a_transport() {
        let provider = RustdxProvider::new().expect("provider construction has no network IO");
        let error = provider
            // Native code is a transport-protocol input only; no order or persistence occurs.
            .fetch_kline_internal("000001", 0)
            .expect_err("zero-day batch is unavailable");
        assert!(error.to_string().contains("没有返回K线数据"));
        assert_eq!(provider.get_stock_name("TEST_CODE_000001"), None);
        assert_eq!(provider.name(), "通达信");
    }

    #[test]
    #[ignore = "live RustDX TCP integration test; run explicitly with --ignored"]
    fn test_rustdx_connection() {
        let _provider = RustdxProvider::new().unwrap();
        assert!(RustdxProvider::new_connection().is_ok());
    }

    #[test]
    #[ignore = "live RustDX and Tencent integration test; run explicitly with --ignored"]
    fn test_fetch_kline() {
        let provider = RustdxProvider::new().unwrap();
        let result = provider.get_daily_data("600000", 10);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(!data.is_empty());
        println!("获取到 {} 条K线数据", data.len());
    }

    #[test]
    #[ignore = "live RustDX TCP integration test; run explicitly with --ignored"]
    fn test_fetch_realtime() {
        let provider = RustdxProvider::new().unwrap();
        let result = provider.get_realtime_quote("600000");
        assert!(result.is_ok());
        if let Some(quote) = result.unwrap() {
            println!("实时行情: {} {:.2}元", quote.name, quote.price);
        }
    }
}
