//! RustDX 数据提供者
//!
//! 使用 rustdx-complete 库从通达信服务器获取股票数据
//! 优点：直连通达信公共服务器，速度快，数据准确，无需额外配置

use super::{DataProvider, KlineData, RealtimeQuote};
use super::gtimg_provider::GtimgProvider;
use anyhow::{anyhow, Context, Result};
use chrono::NaiveDate;
use log::{debug, error, info, warn};
use rustdx_complete::tcp::stock::{Kline, SecurityQuotes};
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

impl RustdxProvider {
    /// 创建新的 RustDxProvider 实例
    pub fn new() -> Result<Self> {
        info!("[通达信] 初始化 RustDX 数据提供者");
        let gtimg_provider = GtimgProvider::new()?;
        Ok(Self {
            gtimg_provider,
        })
    }

    /// 创建新的 TCP 连接
    fn new_connection() -> Result<Tcp> {
        debug!("[通达信] 创建新的TCP连接");
        Tcp::new().context("无法连接到通达信服务器")
    }

    /// 转换股票代码格式
    /// 600000 -> 1 (上海)
    /// 000001 -> 0 (深圳)
    fn parse_market(&self, code: &str) -> u8 {
        if code.starts_with('6') {
            1 // 上海
        } else {
            0 // 深圳/创业板
        }
    }

    /// 规范化股票代码（补全为6位）
    fn normalize_code(&self, code: &str) -> Result<String> {
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
        let code = self.normalize_code(code)?;
        
        let mut tcp = Self::new_connection()?;
        
        let market = self.parse_market(&code) as u16;
        
        // rustdx-complete 内部在返回数量与请求数量不一致时会 panic（assert），
        // 用 catch_unwind 兜底，避免因单只股票数据异常导致整个程序崩溃
        let recv_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Vec<_>> {
            let mut kline = Kline::new(market, &code, 9, 0, days as u16);
            kline.recv_parsed(&mut tcp)
                .map_err(|e| anyhow!("{}", e))?;
            Ok(kline.result().to_vec())
        }));
        let result = match recv_result {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => return Err(anyhow!("获取股票 {} K线数据失败: {}", code, e)),
            Err(_) => return Err(anyhow!(
                "获取股票 {} K线数据时底层库 panic（可能该股票已停牌/退市或代码无效）", code
            )),
        };
        
        if result.is_empty() {
            return Err(anyhow!("股票 {} 没有返回K线数据", code));
        }
        
        // 转换为标准化的KlineData格式
        let mut kline_data: Vec<KlineData> = result.iter().map(|bar| {
            // DateTime 转换为 NaiveDate
            let date = NaiveDate::from_ymd_opt(
                bar.dt.year as i32,
                bar.dt.month as u32,
                bar.dt.day as u32
            ).unwrap_or_else(|| chrono::Local::now().date_naive());
            
            KlineData {
                date,
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                volume: bar.vol,
                amount: bar.amount,
                pct_chg: 0.0, // 稍后计算
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
            }
        }).collect();
        
        // 计算涨跌幅
        for i in 1..kline_data.len() {
            let prev_close = kline_data[i - 1].close;
            if prev_close > 0.0 {
                kline_data[i].pct_chg = ((kline_data[i].close - prev_close) / prev_close) * 100.0;
            }
        }
        
        // 按日期降序排序（最新在前）
        kline_data.sort_by(|a, b| b.date.cmp(&a.date));
        
        info!("[通达信] {} 成功获取 {} 条K线数据", code, kline_data.len());
        
        Ok(kline_data)
    }

    /// 获取实时行情（内部方法）
    fn fetch_realtime_internal(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        // 规范化股票代码
        let code = self.normalize_code(code)?;
        
        let mut tcp = Self::new_connection()?;
        
        let market = self.parse_market(&code) as u16;
        
        // 创建行情查询请求
        let mut quotes = SecurityQuotes::new(vec![(market, &code as &str)]);
        
        quotes.recv_parsed(&mut tcp)
            .context(format!("获取股票 {} 实时行情失败", code))?;
        
        let result = quotes.result();
        
        if result.is_empty() {
            warn!("[通达信] {} 没有返回实时行情数据", code);
            return Ok(None);
        }
        
        let quote = &result[0];
        
        // 通达信返回的数据中已经有涨跌幅字段
        let pct_chg = quote.change_percent;
        
        // 市值计算（通达信没有直接返回市值，这里暂时不计算）
        // 需要额外的接口获取股本数据
        let market_cap = 0.0;
        let circulating_cap = 0.0;
        
        // 换手率计算（通达信没有直接返回，需要成交量/流通股本）
        let turnover_rate = 0.0;
        
        let realtime_quote = RealtimeQuote {
            code: code.to_string(),
            name: quote.name.clone(),
            price: quote.price,
            pct_chg,
            pe_ratio: 0.0,  // 通达信基础行情不包含PE
            pb_ratio: 0.0,  // 通达信基础行情不包含PB
            turnover_rate,
            market_cap,
            circulating_cap,
            volume: quote.vol,
            amount: quote.amount,
        };
        
        debug!("[通达信] {} 实时行情: {:.2}元, 涨跌幅: {:.2}%, 成交量: {:.0}, 名称: {}", 
            code, realtime_quote.price, pct_chg, quote.vol, quote.name);
        
        Ok(Some(realtime_quote))
    }

    /// 获取股票名称（内部方法）
    fn fetch_stock_name_internal(&self, code: &str) -> Option<String> {
        match self.fetch_realtime_internal(code) {
            Ok(Some(quote)) => Some(quote.name),
            Ok(None) => {
                debug!("[通达信] {} 无法获取股票名称", code);
                None
            }
            Err(e) => {
                debug!("[通达信] {} 获取股票名称失败: {}", code, e);
                None
            }
        }
    }
}

impl DataProvider for RustdxProvider {
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>> {
        info!("[通达信] 获取股票 {} 最近 {} 天数据", code, days);
        
        let mut kline_data = self.fetch_kline_internal(code, days)?;
        
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
                        // 更新最新K线的收盘价为实时价格
                        let old_close = latest.close;
                        latest.close = quote.price;
                        
                        // 补充盈利指标
                        latest.pe_ratio = Some(quote.pe_ratio);
                        latest.pb_ratio = Some(quote.pb_ratio);
                        latest.turnover_rate = Some(quote.turnover_rate);
                        latest.market_cap = Some(quote.market_cap);
                        latest.circulating_cap = Some(quote.circulating_cap);
                        
                        info!("[通达信+腾讯] {} 价格: {:.2}元 -> {:.2}元, PE={:.2}, PB={:.2}, 换手率={:.2}%, 总市值={:.2}亿, 流通市值={:.2}亿", 
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

    fn get_stock_name(&self, code: &str) -> Option<String> {
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
    fn test_rustdx_connection() {
        let _provider = RustdxProvider::new().unwrap();
        assert!(RustdxProvider::new_connection().is_ok());
    }

    #[test]
    fn test_fetch_kline() {
        let provider = RustdxProvider::new().unwrap();
        let result = provider.get_daily_data("600000", 10);
        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(!data.is_empty());
        println!("获取到 {} 条K线数据", data.len());
    }

    #[test]
    fn test_fetch_realtime() {
        let provider = RustdxProvider::new().unwrap();
        let result = provider.get_realtime_quote("600000");
        assert!(result.is_ok());
        if let Some(quote) = result.unwrap() {
            println!("实时行情: {} {:.2}元", quote.name, quote.price);
        }
    }
}
