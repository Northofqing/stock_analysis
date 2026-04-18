//! 策略接口层
//!
//! # 策略体系架构
//!
//! ```text
//! ──────────────────────────────────────────────────────────────────
//!  KlineStrategy (trait)           FundamentalStrategy (trait)
//!  ├── BollingerZScoreStrategy      └── MultiFactorStrategy
//!  ├── RsiStrategy
//!  └── HybridStrategy (信号聚合器)
//! ──────────────────────────────────────────────────────────────────
//!
//! 扩展新策略的步骤：
//!   1. 新建 src/strategy/<my_strategy>.rs
//!   2. 实现 KlineStrategy 或 FundamentalStrategy trait
//!   3. 在 strategy/mod.rs 中 `pub mod <my_strategy>` 并 re-export
//!   4. 可直接加入 HybridStrategy
//! ──────────────────────────────────────────────────────────────────
//! ```

use anyhow::Result;
use std::path::PathBuf;

use crate::data_provider::KlineData;

pub mod core;
pub mod bollinger_zscore;
pub mod multi_factor;
pub mod rsi;
pub mod contrarian;

pub use rsi::{RsiStrategy, RsiConfig, RsiBacktest, RsiResult, SingleRsiResult};
pub use rsi::{PrecisionRsiStrategy, PrecisionRsiConfig, PrecisionRsiBacktest, PrecisionRsiResult, SinglePrecisionRsiResult};
pub use bollinger_zscore::{BollingerZScoreStrategy, BollingerZScoreConfig, BollingerZScoreBacktest, BollingerZScoreResult, SingleBacktestResult};
pub use multi_factor::{MultiFactorStrategy, MultiFactorEngine, MultiFactorConfig, StockFactors, StockScore, Factor, FactorDirection};
pub use core::{BacktestSummary, BacktestState, BacktestEngine, BacktestConfig, Trade, TradeAction, Position};
pub use contrarian::{detect_contrarian_signal, ContrarianSignal};

// ────────────────────────────── 通用信号类型 ──────────────────────────────

/// 交易信号（各策略共用，位于此处避免重复定义）
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Signal {
    Buy,
    Sell,
    Hold,
}

// ────────────────────────────── 通用结果 trait ──────────────────────────────

/// 策略运行结果的通用接口
///
/// 所有策略的结果类型实现此 trait，可统一输出报告、摘要和图表。
pub trait StrategyResult: Send + Sync {
    /// 转换为通用 BacktestSummary 摘要
    fn to_summary(&self) -> BacktestSummary;

    /// 生成 Markdown 格式回测报告
    fn generate_report(&self) -> String;

    /// 生成净值曲线图（可选支持，默认返回错误）
    fn generate_chart(&self, path: &str) -> Result<PathBuf> {
        let _ = path;
        Err(anyhow::anyhow!("此策略不支持图表生成"))
    }
}

// ────────────────────────────── K 线策略 trait ──────────────────────────────

/// K 线策略接口（object-safe）
///
/// 适用于基于 OHLCV K线时序数据回测的策略，例如：
/// - 布林带 + Z-Score 均值回归
/// - RSI 超买超卖
/// - 动量策略 / 双均线 / 海龟交易 …（可扩展）
///
/// # 扩展方式
/// ```rust
/// struct MyStrategy { /* config */ }
///
/// impl KlineStrategy for MyStrategy {
///     fn name(&self) -> &'static str { "我的策略" }
///     fn description(&self) -> &'static str { "策略描述" }
///     fn run_portfolio_boxed(
///         &self,
///         stocks: &[(String, String, Vec<KlineData>)],
///     ) -> Result<Box<dyn StrategyResult>> {
///         let result = MyResult { /* ... */ };
///         Ok(Box::new(result))
///     }
/// }
/// ```
pub trait KlineStrategy: Send + Sync {
    /// 策略名称（用于日志与报告标题）
    fn name(&self) -> &'static str;

    /// 策略简介
    fn description(&self) -> &'static str;

    /// 对股票组合运行回测，返回装箱的策略结果
    fn run_portfolio_boxed(
        &self,
        stocks: &[(String, String, Vec<KlineData>)],
    ) -> Result<Box<dyn StrategyResult>>;
}

// ────────────────────────────── 基本面策略 trait ──────────────────────────────

/// 基本面选股策略接口（object-safe）
///
/// 适用于基于财务指标、因子排名进行选股的策略，例如多因子策略。
///
/// # 扩展方式
/// ```rust
/// struct MyFundamentalStrategy;
///
/// impl FundamentalStrategy for MyFundamentalStrategy {
///     fn name(&self) -> &'static str { "我的基本面策略" }
///     fn description(&self) -> &'static str { "..." }
///     fn select_stocks(&self, stocks: &[StockFactors]) -> Result<Vec<StockScore>> {
///         // 排名逻辑
///     }
/// }
/// ```
pub trait FundamentalStrategy: Send + Sync {
    fn name(&self) -> &'static str;

    fn description(&self) -> &'static str;

    /// 对股票池进行因子评分与排名，返回评分结果（按分数排序）
    fn select_stocks(&self, stocks: &[StockFactors]) -> Result<Vec<StockScore>>;
}

// ────────────────────────────── 混合策略（信号聚合器） ──────────────────────────────

/// 混合策略：将多个 KlineStrategy 组合，独立运行后汇总各策略结果
///
/// # 使用示例
/// ```rust
/// use stock_analysis::strategy::{HybridStrategy, BollingerZScoreStrategy, RsiStrategy};
///
/// let hybrid = HybridStrategy::builder()
///     .add(Box::new(BollingerZScoreStrategy::default()), 0.5)
///     .add(Box::new(RsiStrategy::default()), 0.5)
///     .build();
///
/// for (name, weight, result) in hybrid.run_all(&stocks_data) {
///     match result {
///         Ok(r) => println!("{} (权重{:.0}%)\n{}", name, weight * 100.0, r.generate_report()),
///         Err(e) => eprintln!("{} 运行失败: {}", name, e),
///     }
/// }
/// ```
///
/// # 扩展说明
/// - 添加新策略：实现 `KlineStrategy` 后调用 `.add()`
/// - 高级信号融合：在 `run_all()` 返回的结果上可进一步实现：
///   - 多数投票（每日信号 Buy/Sell/Hold 的加权投票）
///   - Kelly 仓位调整
///   - 组合层面的净值加权平均
pub struct HybridStrategy {
    strategies: Vec<(Box<dyn KlineStrategy>, f64)>,
}

/// HybridStrategy 构建器
#[derive(Default)]
pub struct HybridStrategyBuilder {
    strategies: Vec<(Box<dyn KlineStrategy>, f64)>,
}

impl HybridStrategy {
    pub fn builder() -> HybridStrategyBuilder {
        HybridStrategyBuilder::default()
    }

    /// 已注册的子策略名称列表
    pub fn strategy_names(&self) -> Vec<&'static str> {
        self.strategies.iter().map(|(s, _)| s.name()).collect()
    }

    /// 运行所有子策略，返回 `(策略名, 权重, 结果)` 列表
    ///
    /// **当前行为**：各策略独立运行，互不干扰，分别返回结果  
    /// **扩展点**：可在此对 `StrategyResult::to_summary()` 进行加权聚合
    pub fn run_all(
        &self,
        stocks: &[(String, String, Vec<KlineData>)],
    ) -> Vec<(&'static str, f64, Result<Box<dyn StrategyResult>>)> {
        self.strategies
            .iter()
            .map(|(strategy, weight)| {
                let result = strategy.run_portfolio_boxed(stocks);
                (strategy.name(), *weight, result)
            })
            .collect()
    }
}

impl HybridStrategyBuilder {
    /// 添加子策略及其权重（建议各权重之和 = 1.0）
    pub fn add(mut self, strategy: Box<dyn KlineStrategy>, weight: f64) -> Self {
        self.strategies.push((strategy, weight));
        self
    }

    pub fn build(self) -> HybridStrategy {
        HybridStrategy {
            strategies: self.strategies,
        }
    }
}
