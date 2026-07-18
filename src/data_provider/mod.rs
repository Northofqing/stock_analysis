//! 数据提供者模块
//!
//! 提供多种数据源的统一接口

pub mod announcement;
pub mod baostock_provider;
pub mod chain_registry;
pub mod chip_distribution;
pub mod consensus;
pub mod eastmoney_provider;
pub mod fallback;
pub mod financials;
pub mod gtimg_provider;
pub mod halt_status;
pub mod industry;
pub mod intraday_kline;
pub mod ipo_date;
pub mod limit_status;
pub mod money_flow;
pub mod north_flow;
pub mod rustdx_provider;
pub mod service;
pub mod sina_news_provider;
pub mod sina_provider;
pub mod stock_code_map;
pub mod valuation_history;
pub mod yahoo;
// review #16: 新闻条目结构 + content_hash
pub mod news_item;

pub use chip_distribution::{
    compute_chip_distribution, format_for_prompt as format_chip_prompt, ChipDistribution,
};
pub use consensus::{fetch_blocking as fetch_consensus, ConsensusData, RecentReport};
pub use eastmoney_provider::HttpProvider;
pub use financials::{
    assess_quality, fetch_with_fallback_blocking as fetch_financials, FinancialPeriod, Financials,
    QualityReport,
};
pub use gtimg_provider::GtimgProvider;
pub use industry::{fetch_blocking as fetch_industry, IndustryBenchmark};
pub use money_flow::{
    fetch_intraday_shape_blocking, fetch_money_flow_blocking,
    format_for_prompt as format_flow_prompt, IntradayShape, MoneyFlowSummary,
};
pub use rustdx_provider::RustdxProvider;
pub use sina_provider::SinaProvider;
pub use valuation_history::{fetch_blocking as fetch_valuation_history, ValuationHistory};

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::HashMap;
use std::sync::RwLock;

use crate::block_on_async_with_timeout;

/// 复权方式标注 — v11 P0-2 引入
///
/// 每条 K 线标注其价格口径,便于切源时下游比对。
/// - `Qfq`: 前复权 (腾讯/东财 HTTP 直出, 或 RustDX 经 gbbq 计算)
/// - `None`: 不复权 (历史默认值; DB 反序列化路径也用此值, 语义为"上游假定 Qfq, 字段值不可知")
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AdjustType {
    Qfq,
    None,
}

impl AdjustType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdjustType::Qfq => "qfq",
            AdjustType::None => "none",
        }
    }
}

/// Codex review P1 #4 修复: 公共 ban 检测 helper
///
/// HTTP/HTTPS 错误信息如果包含 empty reply / 4xx / 持续超时, 标记为该源被 ban / 不可用.
/// fallback.rs 用这个分类日志: ban → "ban suspected" (建议切代理), non-ban → "non-ban error" (临时错误).
///
/// 原本只在 service.rs 私有, commit 2 抽 fallback 时丢失诊断信号.
pub fn is_ban_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("empty reply")
        || m.contains("connection reset")
        || m.contains("connection refused")
        || m.contains("timeout")
        || m.contains("429")
        || m.contains("403")
        || m.contains("502")
        || m.contains("503")
        || m.contains("504")
        || m.contains("peer closed connection")
}

/// Codex review P1 #4 修复: 公共 brief 截断 helper
///
/// 截断超长错误信息 (避免日志刷屏). fallback.rs 的 `?` 链会内嵌完整 URL, 必须截断.
pub fn brief(s: &str) -> String {
    const MAX: usize = 120;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let head: String = s.chars().take(MAX).collect();
        format!("{head}…(截断)")
    }
}

/// 实时行情数据（包含盈利指标）
#[derive(Debug, Clone)]
pub struct RealtimeQuote {
    pub code: String,
    pub name: String,
    pub price: f64,   // 当前价
    pub pct_chg: f64, // 涨跌幅(%)
    pub pe_ratio: Option<f64>,
    pub pb_ratio: Option<f64>,
    pub turnover_rate: Option<f64>,
    pub market_cap: Option<f64>,
    pub circulating_cap: Option<f64>,
    pub volume: Option<f64>,
    pub amount: Option<f64>,
    pub limit_up_price: Option<f64>,
    pub limit_down_price: Option<f64>,
    /// Exchange/source-observed time used by the BR-097 five-second gate.
    pub source_time: DateTime<Utc>,
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
    /// 修复 P1.8: 盘中实时价 (与 close 分离)
    /// 之前: rustdx_provider 用 quote.price 覆盖 latest.close, 导致 Sharpe 用盘中价
    ///       60 日滚动计算实际变成了盘中波动, 不是日线 settled close
    /// 现在: intraday_price 单独存盘中价, close 保持日线 settled close
    /// Sharpe 计算只用 close, 避免 look-ahead
    pub intraday_price: Option<f64>,
    /// 是否已收盘 (true: 收盘后 close 是最终价; false: 盘中 intraday 才是当前价)
    /// 用于 Sharpe 计算时区分历史 vs 盘中
    pub settled: bool,
    // 盈利水平相关字段
    pub pe_ratio: Option<f64>,        // 市盈率（动态）
    pub pb_ratio: Option<f64>,        // 市净率
    pub turnover_rate: Option<f64>,   // 换手率(%)
    pub market_cap: Option<f64>,      // 总市值（亿元）
    pub circulating_cap: Option<f64>, // 流通市值（亿元）
    // 新增财务指标
    pub eps: Option<f64>,            // 每股收益（元）
    pub roe: Option<f64>,            // 净资产收益率(%)
    pub revenue_yoy: Option<f64>,    // 营业收入同比增长率(%)
    pub net_profit_yoy: Option<f64>, // 净利润同比增长率(%)
    pub gross_margin: Option<f64>,   // 毛利率(%)
    pub net_margin: Option<f64>,     // 净利率(%)
    pub sharpe_ratio: Option<f64>,   // 夏普比率（风险调整后收益）
    /// 多期财务历史序列（按报告期从新到旧），仅填充到 data[0]（最新一根 K 线）
    pub financials_history: Option<Vec<FinancialPeriod>>,
    /// PE/PB 历史分位（近 3 年），仅填充到 data[0]
    pub valuation_history: Option<ValuationHistory>,
    /// 卖方分析师一致预期（近 6 个月研报），仅填充到 data[0]
    pub consensus: Option<ConsensusData>,
    /// 行业横向对标（同业 PE/PB/ROE 中位数 + 个股百分位），仅填充到 data[0]
    pub industry: Option<IndustryBenchmark>,
    // NEW: 涨跌停标记
    pub is_limit_up: bool,   // 是否涨停
    pub is_limit_down: bool, // 是否跌停
    pub is_suspended: bool,  // 是否停牌
    // v11 P0-2: 复权方式标注
    pub adjust: AdjustType, // 该 K 线价格是前复权 (Qfq) 还是不复权 (None)
}

/// BR-092 / AGENTS 2.3: 日 K 进入计算前的严格批量边界。
///
/// 成功后统一为日期降序（最新在前）；任何坏行、重复/缺失交易日或相邻收盘
/// 变化绝对值超过 20% 都拒绝整批，不做 skip/dedup/default。
pub fn validate_kline_series_strict(data: &mut [KlineData], code: &str) -> Result<()> {
    if data.is_empty() {
        return Err(anyhow!("[{code}] 日 K 数据为空"));
    }
    for bar in data.iter() {
        let prices = [bar.open, bar.high, bar.low, bar.close];
        if prices
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(anyhow!(
                "[{code}] {} OHLC 非正或非有限: {:?}",
                bar.date,
                prices
            ));
        }
        if bar.high + 1e-9 < bar.open.max(bar.close)
            || bar.low - 1e-9 > bar.open.min(bar.close)
            || bar.high + 1e-9 < bar.low
        {
            return Err(anyhow!(
                "[{code}] {} OHLC 关系错误 open={} high={} low={} close={}",
                bar.date,
                bar.open,
                bar.high,
                bar.low,
                bar.close
            ));
        }
        if !bar.volume.is_finite()
            || bar.volume < 0.0
            || !bar.amount.is_finite()
            || bar.amount < 0.0
            || !bar.pct_chg.is_finite()
        {
            return Err(anyhow!(
                "[{code}] {} 成交量/额/涨跌幅非法 volume={} amount={} pct_chg={}",
                bar.date,
                bar.volume,
                bar.amount,
                bar.pct_chg
            ));
        }
        if bar.pct_chg.abs() > 20.0 {
            return Err(anyhow!(
                "[{code}] {} 涨跌幅 {:.4}% 超过 20%，需要人工确认",
                bar.date,
                bar.pct_chg
            ));
        }
    }

    data.sort_by_key(|bar| bar.date);
    for pair in data.windows(2) {
        let previous = &pair[0];
        let current = &pair[1];
        if current.date <= previous.date {
            return Err(anyhow!(
                "[{code}] 日 K 日期重复或无序: {} -> {}",
                previous.date,
                current.date
            ));
        }
        let mut missing = previous
            .date
            .succ_opt()
            .ok_or_else(|| anyhow!("[{code}] 日 K 日期溢出"))?;
        while missing < current.date {
            if crate::calendar::is_trading_day(missing) {
                return Err(anyhow!("[{code}] 日 K 缺少交易日 {missing}"));
            }
            missing = missing
                .succ_opt()
                .ok_or_else(|| anyhow!("[{code}] 日 K 日期溢出"))?;
        }
        let change_pct = (current.close - previous.close) / previous.close * 100.0;
        if change_pct.abs() > 20.0 {
            return Err(anyhow!(
                "[{code}] {} -> {} 相邻收盘变化 {:.4}% 超过 20%，需要人工确认",
                previous.date,
                current.date,
                change_pct
            ));
        }
    }
    data.sort_by_key(|bar| std::cmp::Reverse(bar.date));
    Ok(())
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

        Ok(Self {
            providers,
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
                self.stock_name_cache
                    .write()
                    .unwrap()
                    .insert(code.to_string(), name.clone());
                return Some(name);
            }
        }
        None
    }

    /// 获取股票数据（自动切换数据源）
    ///
    /// v11 P0-2 commit 2: 改用共享 fallback 函数 (`fallback::fetch_kline_with_fallback`),
    /// 内部用 `block_on_async_with_timeout` 调共享 async 函数 (sync 入口保留, 13 个下游调用方零改动).
    ///
    /// ⚠️ 必须在 multi_thread runtime 中调用, 否则 `block_on_async` 会 panic (lib.rs:143).
    /// 所有当前调用方 (`bin/*`, `pipeline/*` 等) 已在 multi_thread runtime 中。
    pub fn get_daily_data(
        &self,
        code: &str,
        days: usize,
    ) -> Result<(Vec<KlineData>, &'static str)> {
        // 1. 走共享 fallback (腾讯 → 东财 → RustDX)
        // block_on_async_with_timeout 把 future 输出包装成 Result<_, String>,
        // future 本身又是 anyhow::Result, 故嵌套为 Result<Result<_>, String>. 两个 ? 解嵌套.
        let timeout_result =
            block_on_async_with_timeout(fallback::fetch_kline_with_fallback(code, days), 30)
                .map_err(|e| anyhow::anyhow!("fallback 超时: {}", e))?;
        let (data, source_name) =
            timeout_result.map_err(|e| anyhow::anyhow!("fallback 失败: {}", e))?;

        log::info!("成功从 {} 获取到 {} 条数据", source_name, data.len());

        // 补充财务/估值/一致预期/行业数据不再从同步日 K 接口隐式抓取。
        // 这些来源走各自的 Result API，由实际需要它们的调用方显式处理 unavailable，
        // 避免后台线程无 runtime 时把失败静默改写为默认对象（BR-115）。
        Ok((data, source_name))
    }
}

impl Default for DataFetcherManager {
    fn default() -> Self {
        Self::new().expect("创建DataFetcherManager失败")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v11 P0-2: AdjustType as_str 应返回稳定的小写字符串,用于 data_source 复合命名 (rustdx_qfq / tencent_qfq / eastmoney_qfq)
    #[test]
    fn adjust_type_as_str_stable() {
        assert_eq!(AdjustType::Qfq.as_str(), "qfq");
        assert_eq!(AdjustType::None.as_str(), "none");
    }

    /// v11 P0-2: AdjustType 必须是 Copy (赋值零成本),且支持 PartialEq 比较
    #[test]
    fn adjust_type_is_copy_and_eq() {
        let a = AdjustType::Qfq;
        let b = a; // Copy: a 仍然可用
        assert_eq!(a, b);
        assert_ne!(AdjustType::Qfq, AdjustType::None);
    }

    /// v11 P0-2 commit 2: `DataFetcherManager::get_daily_data` (sync 入口) 在
    /// `spawn_blocking` 上下文里能调, 不应触发 `block_on_async` panic (lib.rs:143).
    ///
    /// ⚠️ 网络依赖 — `#[ignore]` 跳过 CI, 手动跑: `cargo test --lib sync_get_daily_data_no_panic_in_spawn_blocking -- --ignored`
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn sync_get_daily_data_no_panic_in_spawn_blocking() {
        let dm = DataFetcherManager::new().expect("create DataFetcherManager");
        let result = tokio::task::spawn_blocking(move || dm.get_daily_data("600519", 5)).await;

        // 关键断言: spawn_blocking 不应 panic. 网络错误是预期的 (Err(_)), 但
        // spawn_blocking 自身的 JoinError 表示 panic, 必须 fail.
        match result {
            Ok(_outcome) => {
                // Ok(Ok((data, src))) 或 Ok(Err(network)), 都接受.
                println!("spawn_blocking did not panic (outcome: data fetched or network err)");
            }
            Err(join_err) => {
                panic!(
                    "spawn_blocking panic'd: {} — sync 入口在 spawn_blocking 上下文不安全",
                    join_err
                );
            }
        }
    }
}
