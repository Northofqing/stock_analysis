//! 数据提供者模块
//!
//! 提供多种数据源的统一接口

pub mod announcement;
pub mod chip_distribution;
pub mod consensus;
pub mod eastmoney_provider;
pub mod limit_status;
pub mod north_flow;
pub mod financials;
pub mod gtimg_provider;
pub mod industry;
pub mod intraday_kline;
pub mod money_flow;
pub mod rustdx_provider;
pub mod service;
pub mod valuation_history;
pub mod yahoo;

pub use chip_distribution::{
    compute_chip_distribution, format_for_prompt as format_chip_prompt, ChipDistribution,
};
pub use eastmoney_provider::HttpProvider;
pub use financials::{fetch_with_fallback_blocking as fetch_financials, assess_quality, FinancialPeriod, Financials, QualityReport};
pub use gtimg_provider::GtimgProvider;
pub use money_flow::{
    fetch_intraday_shape_blocking, fetch_money_flow_blocking, format_for_prompt as format_flow_prompt,
    IntradayShape, MoneyFlowSummary,
};
pub use rustdx_provider::RustdxProvider;
pub use valuation_history::{fetch_blocking as fetch_valuation_history, ValuationHistory};
pub use consensus::{fetch_blocking as fetch_consensus, ConsensusData, RecentReport};
pub use industry::{fetch_blocking as fetch_industry, IndustryBenchmark};

use anyhow::Result;
use chrono::NaiveDate;
use std::collections::HashMap;
use std::sync::RwLock;

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
    /// 多期财务历史序列（按报告期从新到旧），仅填充到 data[0]（最新一根 K 线）
    pub financials_history: Option<Vec<FinancialPeriod>>,
    /// PE/PB 历史分位（近 3 年），仅填充到 data[0]
    pub valuation_history: Option<ValuationHistory>,
    /// 卖方分析师一致预期（近 6 个月研报），仅填充到 data[0]
    pub consensus: Option<ConsensusData>,
    /// 行业横向对标（同业 PE/PB/ROE 中位数 + 个股百分位），仅填充到 data[0]
    pub industry: Option<IndustryBenchmark>,
    // NEW: 涨跌停标记
    pub is_limit_up: bool,            // 是否涨停
    pub is_limit_down: bool,          // 是否跌停
    pub is_suspended: bool,           // 是否停牌
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
    fn get_realtime_quote(&self, _code: &str) -> Result<Option<RealtimeQuote>> {
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
    financials_client: reqwest::Client,
    stock_name_cache: RwLock<HashMap<String, String>>,
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

        // 备用：东方财富HTTP数据源
        if let Ok(http_provider) = HttpProvider::new() {
            providers.push(Box::new(http_provider));
        }

        let financials_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .build()
            .unwrap_or_default();

        Ok(Self {
            providers,
            financials_client,
            stock_name_cache: RwLock::new(HashMap::new()),
        })
    }
    // 获取股票名称
    pub fn get_stock_name(&self, code: &str) -> Option<String> {
        // 先查缓存（名称在进程生命周期内不变）
        if let Some(name) = self.stock_name_cache.read().unwrap().get(code) {
            return Some(name.clone());
        }
        for provider in &self.providers {
            if let Some(name) = provider.get_stock_name(code) {
                self.stock_name_cache.write().unwrap().insert(code.to_string(), name.clone());
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
                Ok(mut data) if !data.is_empty() => {
                    log::info!("成功从 {} 获取到 {} 条数据", provider.name(), data.len());

                    // 四个补充数据源并行抓取（独立 HTTP 调用）
                    let (fin, vh, cs, ib) = std::thread::scope(|s| {
                        let client = &self.financials_client;
                        let fin_h = s.spawn(|| financials::fetch_with_fallback_blocking(client, code));
                        let vh_h = s.spawn(|| valuation_history::fetch_blocking(client, code));
                        let cs_h = s.spawn(|| consensus::fetch_blocking(client, code));
                        let ib_h = s.spawn(|| industry::fetch_blocking(client, code));
                        (
                            fin_h.join().unwrap_or_default(),
                            vh_h.join().unwrap_or_default(),
                            cs_h.join().unwrap_or_default(),
                            ib_h.join().unwrap_or_default(),
                        )
                    });

                    if fin.any() {
                        if let Some(latest) = data.first_mut() {
                            latest.eps = latest.eps.or(fin.eps);
                            latest.roe = latest.roe.or(fin.roe);
                            latest.revenue_yoy = latest.revenue_yoy.or(fin.revenue_yoy);
                            latest.net_profit_yoy = latest.net_profit_yoy.or(fin.net_profit_yoy);
                            latest.gross_margin = latest.gross_margin.or(fin.gross_margin);
                            latest.net_margin = latest.net_margin.or(fin.net_margin);
                            if !fin.history.is_empty() {
                                latest.financials_history = Some(fin.history.clone());
                            }
                        }
                    }

                    if let Some(vh) = vh {
                        if let Some(latest) = data.first_mut() {
                            latest.pe_ratio = latest.pe_ratio.or(vh.current_pe);
                            latest.pb_ratio = latest.pb_ratio.or(vh.current_pb);
                            latest.valuation_history = Some(vh);
                        }
                    }

                    if let Some(cs) = cs {
                        if let Some(latest) = data.first_mut() {
                            latest.consensus = Some(cs);
                        }
                    }

                    if let Some(ib) = ib {
                        if let Some(latest) = data.first_mut() {
                            latest.industry = Some(ib);
                        }
                    }

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
