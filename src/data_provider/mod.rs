//! 数据提供者模块
//!
//! 提供多种数据源的统一接口

pub mod eastmoney_provider;
pub mod gtimg_provider;
pub mod rustdx_provider;
pub mod tushare_provider;

pub use eastmoney_provider::HttpProvider;
pub use gtimg_provider::GtimgProvider;
pub use rustdx_provider::RustdxProvider;
pub use tushare_provider::TushareProvider;

use anyhow::Result;
use chrono::NaiveDate;

/// 实时行情数据（包含盈利指标）
#[derive(Debug, Clone)]
pub struct RealtimeQuote {
    pub code: String,
    pub name: String,
    pub price: f64,           // 当前价
    pub pct_chg: f64,         // 涨跌幅(%)
    pub pe_ratio: f64,        // 市盈率（动态）
    pub pb_ratio: f64,        // 市净率
    pub turnover_rate: f64,   // 换手率(%)
    pub market_cap: f64,      // 总市值（亿元）
    pub circulating_cap: f64, // 流通市值（亿元）
    pub volume: f64,          // 成交量
    pub amount: f64,          // 成交额
}

/// 标准化的K线数据
#[derive(Debug, Clone)]
pub struct KlineData {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub pct_chg: f64,
    // 盈利水平相关字段
    pub pe_ratio: Option<f64>,        // 市盈率（动态）
    pub pb_ratio: Option<f64>,        // 市净率
    pub turnover_rate: Option<f64>,   // 换手率(%)
    pub market_cap: Option<f64>,      // 总市值（亿元）
    pub circulating_cap: Option<f64>, // 流通市值（亿元）
    // 新增财务指标
    pub eps: Option<f64>,             // 每股收益（元）
    pub roe: Option<f64>,             // 净资产收益率(%)
    pub revenue_yoy: Option<f64>,     // 营业收入同比增长率(%)
    pub net_profit_yoy: Option<f64>,  // 净利润同比增长率(%)
    pub gross_margin: Option<f64>,    // 毛利率(%)
    pub net_margin: Option<f64>,      // 净利率(%)
    pub sharpe_ratio: Option<f64>,    // 夏普比率（风险调整后收益）
}

/// 数据提供者接口
pub trait DataProvider: Send + Sync {
    /// 获取股票日线数据
    ///
    /// # 参数
    /// - code: 股票代码
    /// - days: 获取天数
    ///
    /// # 返回
    /// - Result<Vec<KlineData>>
    fn get_daily_data(&self, code: &str, days: usize) -> Result<Vec<KlineData>>;
    
    /// 获取股票名称
    fn get_stock_name(&self, code: &str) -> Option<String>;
    
    /// 获取实时行情（包含盈利指标）
    fn get_realtime_quote(&self, code: &str) -> Result<Option<RealtimeQuote>> {
        // 默认实现返回 None
        Ok(None)
    }
    
    /// 获取数据源名称
    fn name(&self) -> &'static str;
}

/// 数据获取管理器
///
/// 支持多数据源自动切换
pub struct DataFetcherManager {
    providers: Vec<Box<dyn DataProvider>>,
}

impl DataFetcherManager {
    /// 创建新的管理器
    pub fn new() -> Result<Self> {
        let mut providers: Vec<Box<dyn DataProvider>> = Vec::new();

        // 优先使用 RustDX 通达信（速度快、稳定、免费）
        if let Ok(rustdx_provider) = RustdxProvider::new() {
            providers.push(Box::new(rustdx_provider));
        }

        // 备用：腾讯财经（稳定可靠）
        if let Ok(gtimg_provider) = GtimgProvider::new() {
            providers.push(Box::new(gtimg_provider));
        }

        // 备用：Tushare Pro（专业数据源，需要积分）
        if let Ok(tushare_provider) = TushareProvider::new() {
            providers.push(Box::new(tushare_provider));
        }

        // 备用：东方财富HTTP数据源
        if let Ok(http_provider) = HttpProvider::new() {
            providers.push(Box::new(http_provider));
        }
        
        Ok(Self { providers })
    }
    // 获取股票名称
    pub fn get_stock_name(&self, code: &str) -> Option<String> {
        for provider in &self.providers {
            if let Some(name) = provider.get_stock_name(code) {
                return Some(name);
            }
        }
        None
    }

    /// 获取股票数据（自动切换数据源）
    pub fn get_daily_data(
        &self,
        code: &str,
        days: usize,
    ) -> Result<(Vec<KlineData>, &'static str)> {
        for provider in &self.providers {
            log::info!("尝试使用数据源: {}", provider.name());

            match provider.get_daily_data(code, days) {
                Ok(data) if !data.is_empty() => {
                    log::info!("成功从 {} 获取到 {} 条数据", provider.name(), data.len());
                    return Ok((data, provider.name()));
                }
                Ok(_) => {
                    log::warn!("数据源 {} 返回空数据", provider.name());
                }
                Err(e) => {
                    log::warn!("数据源 {} 获取失败: {}", provider.name(), e);
                }
            }
        }

        anyhow::bail!("所有数据源均获取失败")
    }
}

impl Default for DataFetcherManager {
    fn default() -> Self {
        Self::new().expect("创建DataFetcherManager失败")
    }
}
