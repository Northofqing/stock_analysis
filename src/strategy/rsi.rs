//! RSI（相对强弱指数）策略集
//!
//! # 策略一：通用 RSI 超买超卖（`RsiBacktest`）
//!
//! 1. 用 Polars 计算 N 日 RSI：`RSI = 100 - 100 / (1 + RS)`，`RS = 平均涨幅 / 平均跌幅`
//! 2. RSI < 超卖阈值（默认 30）且无持仓时 → **买入**
//! 3. RSI > 超买阈值（默认 70）时 → **卖出**（止盈）
//! 4. RSI 回升至均衡区（默认 50）且持仓盈利 → **平仓**（锁利）
//! 5. 可选 60 日均线趋势过滤，只在上升趋势中做多
//!
//! # 策略二：精准 RSI 深度超卖均值回归（`PrecisionRsiBacktest`）
//!
//! **买入条件（全部满足）**
//! 1. 5 日 RSI < 30 — 深度超卖
//! 2. 5 日 RSI 连续第三天走低（`rsi[i] < rsi[i-1] < rsi[i-2]`）— 超卖动能充分释放
//! 3. 三个交易日前 5 日 RSI < 60（`rsi[i-3] < 60`）— 前期压力已消化，排除假性超卖
//! 4. 收盘价 > 200 日均线 — 长期上升趋势，避免下跌通道诱多
//!
//! **卖出条件**
//! - 5 日 RSI 向上突破 50（前一日 < 50，当日 >= 50）— 超卖格局修复，均值回归完成

use anyhow::Result;
use chrono::{Local, NaiveDate, TimeZone};
use log::{info, warn};
use polars::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use super::core::{BacktestState, BacktestSummary, Trade, TradeAction};
use crate::data_provider::KlineData;
use super::{KlineStrategy, Signal, StrategyResult};

// ────────────────────────────── 策略参数 ──────────────────────────────

/// RSI 超买超卖策略配置
#[derive(Debug, Clone)]
pub struct RsiConfig {
    /// RSI 计算周期（默认 14 日）
    pub rsi_period: usize,
    /// 超卖阈值：RSI 低于此值视为超卖，触发买入（默认 30）
    pub oversold: f64,
    /// 超买阈值：RSI 高于此值视为超买，触发卖出（默认 70）
    pub overbought: f64,
    /// 均衡平仓阈值：RSI 回到此值且持仓盈利时平仓（默认 50）
    pub exit_level: f64,
    /// 是否启用 60 日均线趋势过滤（开启后只在价格 > MA60 时买入）
    pub use_trend_filter: bool,
    /// 初始资金（元）
    pub initial_capital: f64,
    /// 单只股票最大仓位比例（0–1）
    pub max_position_pct: f64,
    /// 手续费率
    pub commission_rate: f64,
    /// 滑点率
    pub slippage_rate: f64,
}

impl Default for RsiConfig {
    fn default() -> Self {
        Self {
            rsi_period: 14,
            oversold: 30.0,
            overbought: 70.0,
            exit_level: 50.0,
            use_trend_filter: true,
            initial_capital: 100_000.0,
            max_position_pct: 0.25,
            commission_rate: 0.0003,
            slippage_rate: 0.001,
        }
    }
}

// ────────────────────────────── Polars 指标计算 ──────────────────────────────

/// 计算指定周期的 RSI 列（返回的列名为 `rsi_{period}`）
fn build_rsi_lazy(lf: LazyFrame, period: usize, close_col: &str) -> LazyFrame {
    let alias = format!("rsi_{}", period);
    lf.with_columns([
        (col(close_col) - col(close_col).shift(lit(1))).alias("_delta")
    ])
    .with_columns([
        when(col("_delta").gt(lit(0.0f64)))
            .then(col("_delta"))
            .otherwise(lit(0.0f64))
            .alias("_gain"),
        when(col("_delta").lt(lit(0.0f64)))
            .then(lit(0.0f64) - col("_delta"))
            .otherwise(lit(0.0f64))
            .alias("_loss"),
    ])
    .with_columns([
        col("_gain")
            .rolling_mean(RollingOptionsFixedWindow {
                window_size: period,
                min_periods: period,
                ..Default::default()
            })
            .alias("_avg_gain"),
        col("_loss")
            .rolling_mean(RollingOptionsFixedWindow {
                window_size: period,
                min_periods: period,
                ..Default::default()
            })
            .alias("_avg_loss"),
    ])
    .with_columns([
        when(col("_avg_loss").eq(lit(0.0f64)))
            .then(lit(100.0f64))
            .otherwise(
                lit(100.0f64)
                    - lit(100.0f64) / (lit(1.0f64) + col("_avg_gain") / col("_avg_loss")),
            )
            .alias(alias),
    ])
    .drop(["_delta", "_gain", "_loss", "_avg_gain", "_avg_loss"])
}

/// 使用 Polars 计算 RSI 及可选趋势过滤指标（通用策略用）
pub fn compute_rsi_indicators(klines: &[KlineData], config: &RsiConfig) -> Result<DataFrame> {
    let n = klines.len();
    if n < config.rsi_period + 5 {
        anyhow::bail!(
            "K线数据不足 {} 条，无法计算 RSI({})",
            config.rsi_period + 5,
            config.rsi_period
        );
    }

    let dates: Vec<String> = klines
        .iter()
        .map(|k| k.date.format("%Y-%m-%d").to_string())
        .collect();
    let close: Vec<f64> = klines.iter().map(|k| k.close).collect();

    let df = df![
        "date"  => &dates,
        "close" => &close,
    ]?;

    let lf = build_rsi_lazy(df.lazy(), config.rsi_period, "close");
    // rename rsi_{period} → rsi for backward compat
    let period_alias = format!("rsi_{}", config.rsi_period);
    let df = lf
        .rename([period_alias.as_str()], ["rsi"], true)
        .collect()?;

    // 可选：追加趋势过滤列
    if config.use_trend_filter {
        let df = df
            .lazy()
            .with_columns([col("close")
                .rolling_mean(RollingOptionsFixedWindow {
                    window_size: 60,
                    min_periods: 60,
                    ..Default::default()
                })
                .alias("trend_ma")])
            .with_columns([col("close").gt(col("trend_ma")).alias("is_uptrend")])
            .collect()?;
        return Ok(df);
    }

    Ok(df)
}

/// 计算精准策略所需全部指标：5日RSI + 200日均线
pub fn compute_precision_indicators(klines: &[KlineData]) -> Result<DataFrame> {
    let n = klines.len();
    if n < 205 {
        anyhow::bail!("K线数据不足 205 条，无法计算 200 日均线（当前 {} 条）", n);
    }

    let dates: Vec<String> = klines
        .iter()
        .map(|k| k.date.format("%Y-%m-%d").to_string())
        .collect();
    let close: Vec<f64> = klines.iter().map(|k| k.close).collect();

    let df = df![
        "date"  => &dates,
        "close" => &close,
    ]?;

    // 5日RSI
    let lf = build_rsi_lazy(df.lazy(), 5, "close");
    // 200日均线
    let df = lf
        .with_columns([col("close")
            .rolling_mean(RollingOptionsFixedWindow {
                window_size: 200,
                min_periods: 200,
                ..Default::default()
            })
            .alias("ma200")])
        .collect()?;

    Ok(df)
}

// ────────────────────────────── 精准RSI配置 ──────────────────────────────

/// 精准 RSI 深度超卖均值回归策略配置
#[derive(Debug, Clone)]
pub struct PrecisionRsiConfig {
    /// 初始资金（元）
    pub initial_capital: f64,
    /// 单只股票最大仓位比例（0–1）
    pub max_position_pct: f64,
    /// 手续费率
    pub commission_rate: f64,
    /// 滑点率
    pub slippage_rate: f64,
}

impl Default for PrecisionRsiConfig {
    fn default() -> Self {
        Self {
            initial_capital: 100_000.0,
            max_position_pct: 0.25,
            commission_rate: 0.0003,
            slippage_rate: 0.001,
        }
    }
}

// ────────────────────────────── 单股回测结果 ──────────────────────────────

/// 单只股票的 RSI 回测结果
pub struct SingleRsiResult {
    pub code: String,
    pub name: String,
    pub initial_capital: f64,
    pub final_value: f64,
    pub trades: Vec<Trade>,
    pub daily_values: Vec<(chrono::DateTime<Local>, f64)>,
    pub signals: Vec<Signal>,
    /// 每日 RSI 值（部分日期为 None，指标预热期）
    pub rsi_values: Vec<Option<f64>>,
}

// ────────────────────────────── 组合回测结果 ──────────────────────────────

/// RSI 策略组合回测结果
pub struct RsiResult {
    pub config: RsiConfig,
    pub single_results: Vec<SingleRsiResult>,
    pub portfolio_daily_values: Vec<(chrono::DateTime<Local>, f64)>,
    pub portfolio_trades: Vec<Trade>,
}

impl RsiResult {
    /// 转换为通用 BacktestSummary
    pub fn to_summary(&self) -> BacktestSummary {
        let total_initial = self.config.initial_capital * self.single_results.len() as f64;
        let mut state = BacktestState::new(total_initial);
        state.daily_values = self.portfolio_daily_values.clone();
        state.trades = self.portfolio_trades.clone();
        BacktestSummary::from_state(&state, total_initial)
    }

    /// 生成净值曲线图
    pub fn generate_chart(&self, output_path: &str) -> Result<PathBuf> {
        let total_initial = self.config.initial_capital * self.single_results.len() as f64;
        let mut state = BacktestState::new(total_initial);
        state.daily_values = self.portfolio_daily_values.clone();
        state.trades = self.portfolio_trades.clone();
        let summary = BacktestSummary::from_state(&state, total_initial);
        summary.generate_chart(&state, output_path)
    }

    /// 生成 Markdown 回测报告
    pub fn generate_report(&self) -> String {
        let summary = self.to_summary();
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let mut report = String::new();
        report.push_str("# 📊 RSI 超买超卖策略回测报告\n\n");
        report.push_str(&format!("**生成时间**: {}\n\n", now));
        report.push_str("---\n\n");

        // 策略参数
        report.push_str("## ⚙️ 策略参数\n\n");
        report.push_str("| 参数 | 值 |\n|------|----|\n");
        report.push_str(&format!("| RSI 周期 | {} 日 |\n", self.config.rsi_period));
        report.push_str(&format!("| 超卖阈值 | {:.0} |\n", self.config.oversold));
        report.push_str(&format!("| 超买阈值 | {:.0} |\n", self.config.overbought));
        report.push_str(&format!("| 均衡平仓阈值 | {:.0} |\n", self.config.exit_level));
        report.push_str(&format!(
            "| 趋势过滤(MA60) | {} |\n",
            if self.config.use_trend_filter { "开启" } else { "关闭" }
        ));
        report.push_str(&format!(
            "| 单股最大仓位 | {:.0}% |\n",
            self.config.max_position_pct * 100.0
        ));
        report.push_str(&format!(
            "| 手续费率 | {:.2}‰ |\n",
            self.config.commission_rate * 1000.0
        ));
        report.push_str(&format!(
            "| 滑点率 | {:.1}‰ |\n\n",
            self.config.slippage_rate * 1000.0
        ));

        // 组合汇总
        report.push_str("## 📈 组合回测结果\n\n");
        report.push_str("| 指标 | 数值 | 说明 |\n|------|------|------|\n");
        report.push_str(&format!(
            "| 初始资金 | ¥{:.2}万 | {} 只股票 × {:.0}万/只 |\n",
            summary.initial_capital / 10000.0,
            self.single_results.len(),
            self.config.initial_capital / 10000.0
        ));
        report.push_str(&format!(
            "| 期末资产 | ¥{:.2}万 | - |\n",
            summary.final_value / 10000.0
        ));
        let ret_emoji = if summary.total_return > 0.0 { "📈" } else { "📉" };
        report.push_str(&format!(
            "| 总收益率 | {:.2}% | {} |\n",
            summary.total_return * 100.0,
            ret_emoji
        ));
        report.push_str(&format!(
            "| 年化收益率 | {:.2}% | - |\n",
            summary.annual_return * 100.0
        ));
        let dd_label = if summary.max_drawdown < 0.1 {
            "🛡️ 风险较低"
        } else if summary.max_drawdown < 0.2 {
            "⚠️ 风险适中"
        } else {
            "🚨 风险较高"
        };
        report.push_str(&format!(
            "| 最大回撤 | {:.2}% | {} |\n",
            summary.max_drawdown * 100.0,
            dd_label
        ));
        let sr_label = if summary.sharpe_ratio > 1.0 {
            "⭐ 优秀"
        } else if summary.sharpe_ratio > 0.5 {
            "✅ 良好"
        } else {
            "⚠️ 一般"
        };
        report.push_str(&format!(
            "| 夏普比率 | {:.2} | {} |\n",
            summary.sharpe_ratio, sr_label
        ));
        report.push_str(&format!(
            "| 总交易次数 | {} 次 | - |\n",
            summary.total_trades
        ));
        report.push_str(&format!(
            "| 胜率 | {:.1}% | - |\n\n",
            summary.win_rate * 100.0
        ));

        // 个股明细
        report.push_str("## 📋 个股回测明细\n\n");
        report.push_str("| 股票 | 代码 | 初始资金 | 期末市值 | 收益率 | 交易次数 |\n");
        report.push_str("|------|------|---------|---------|--------|----------|\n");
        for r in &self.single_results {
            let ret = (r.final_value / r.initial_capital - 1.0) * 100.0;
            let emoji = if ret > 0.0 { "🟢" } else { "🔴" };
            report.push_str(&format!(
                "| {} {} | {} | {:.0} | {:.0} | {} {:.2}% | {} |\n",
                emoji,
                r.name,
                r.code,
                r.initial_capital,
                r.final_value,
                emoji,
                ret,
                r.trades.len()
            ));
        }
        report.push_str("\n");

        // 策略说明
        report.push_str("## 📝 策略说明\n\n");
        report.push_str("**RSI 超买超卖策略**基于相对强弱指数量化市场情绪极端状态：\n\n");
        report.push_str("1. **RSI 计算**：`RSI = 100 - 100 / (1 + RS)`，`RS = N日平均涨幅 / N日平均跌幅`，默认周期 14 日\n");
        report.push_str("2. **买入信号**：RSI < 超卖阈值（默认 30） → 市场恐慌性抛售，寻找反弹机会\n");
        report.push_str("3. **卖出信号**：RSI > 超买阈值（默认 70） → 过度追涨，止盈离场\n");
        report.push_str("4. **均衡平仓**：RSI 回升至均衡区（默认 50）且持仓盈利 → 落袋为安\n");
        report.push_str("5. **趋势过滤**：开启时仅在收盘价 > MA60 时买入，规避下跌趋势中的超卖陷阱\n\n");
        report.push_str("> ⚠️ 本策略适合震荡市和阶段性行情，强趋势市中可能频繁触发假信号。\n\n");

        report
    }
}

// ────────────────────────────── 精准RSI单股结果 ──────────────────────────────

/// 精准RSI策略单只股票回测结果
pub struct SinglePrecisionRsiResult {
    pub code: String,
    pub name: String,
    pub initial_capital: f64,
    pub final_value: f64,
    pub trades: Vec<Trade>,
    pub daily_values: Vec<(chrono::DateTime<Local>, f64)>,
    pub signals: Vec<Signal>,
    /// 每日 5日RSI 值
    pub rsi5_values: Vec<Option<f64>>,
}

// ────────────────────────────── 精准RSI组合结果 ──────────────────────────────

/// 精准 RSI 策略组合回测结果
pub struct PrecisionRsiResult {
    pub config: PrecisionRsiConfig,
    pub single_results: Vec<SinglePrecisionRsiResult>,
    pub portfolio_daily_values: Vec<(chrono::DateTime<Local>, f64)>,
    pub portfolio_trades: Vec<Trade>,
}

impl PrecisionRsiResult {
    pub fn to_summary(&self) -> BacktestSummary {
        let total_initial = self.config.initial_capital * self.single_results.len() as f64;
        let mut state = BacktestState::new(total_initial);
        state.daily_values = self.portfolio_daily_values.clone();
        state.trades = self.portfolio_trades.clone();
        BacktestSummary::from_state(&state, total_initial)
    }

    pub fn generate_chart(&self, output_path: &str) -> Result<PathBuf> {
        let total_initial = self.config.initial_capital * self.single_results.len() as f64;
        let mut state = BacktestState::new(total_initial);
        state.daily_values = self.portfolio_daily_values.clone();
        state.trades = self.portfolio_trades.clone();
        let summary = BacktestSummary::from_state(&state, total_initial);
        summary.generate_chart(&state, output_path)
    }

    pub fn generate_report(&self) -> String {
        let summary = self.to_summary();
        let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let mut report = String::new();
        report.push_str("# 📊 精准RSI深度超卖均值回归策略回测报告\n\n");
        report.push_str(&format!("**生成时间**: {}\n\n", now));
        report.push_str("---\n\n");

        report.push_str("## ⚙️ 策略规则\n\n");
        report.push_str("### 买入条件（需同时满足全部）\n\n");
        report.push_str("| # | 条件 | 说明 |\n|---|------|------|\n");
        report.push_str("| 1 | 5日RSI < 30 | 深度超卖区域 |\n");
        report.push_str("| 2 | 5日RSI连续第三天走低 | 超卖动能充分释放，蓄积反弹力量 |\n");
        report.push_str("| 3 | 三交易日前 5日RSI < 60 | 前期压力已消化，排除假性超卖 |\n");
        report.push_str("| 4 | 收盘价 > 200日均线 | 长期上升趋势，规避下跌通道诱多 |\n\n");
        report.push_str("### 卖出条件\n\n");
        report.push_str("| 条件 | 说明 |\n|------|------|\n");
        report.push_str("| 5日RSI 向上突破 50（前日 < 50，当日 ≥ 50） | 超卖格局彻底修复，均值回归第一阶段完成 |\n\n");

        report.push_str("## ⚙️ 策略参数\n\n");
        report.push_str("| 参数 | 值 |\n|------|----|\n");
        report.push_str("| RSI 周期 | 5 日 |\n");
        report.push_str("| 超卖阈值 | 30 |\n");
        report.push_str("| 离场阈值（RSI突破） | 50 |\n");
        report.push_str("| 趋势均线 | MA200 |\n");
        report.push_str(&format!("| 单股最大仓位 | {:.0}% |\n", self.config.max_position_pct * 100.0));
        report.push_str(&format!("| 手续费率 | {:.2}‰ |\n", self.config.commission_rate * 1000.0));
        report.push_str(&format!("| 滑点率 | {:.1}‰ |\n\n", self.config.slippage_rate * 1000.0));

        report.push_str("## 📈 组合回测结果\n\n");
        report.push_str("| 指标 | 数值 | 说明 |\n|------|------|------|\n");
        report.push_str(&format!(
            "| 初始资金 | ¥{:.2}万 | {} 只股票 × {:.0}万/只 |\n",
            summary.initial_capital / 10000.0,
            self.single_results.len(),
            self.config.initial_capital / 10000.0
        ));
        report.push_str(&format!("| 期末资产 | ¥{:.2}万 | - |\n", summary.final_value / 10000.0));
        let ret_emoji = if summary.total_return > 0.0 { "📈" } else { "📉" };
        report.push_str(&format!("| 总收益率 | {:.2}% | {} |\n", summary.total_return * 100.0, ret_emoji));
        report.push_str(&format!("| 年化收益率 | {:.2}% | - |\n", summary.annual_return * 100.0));
        let dd_label = if summary.max_drawdown < 0.1 { "🛡️ 风险较低" } else if summary.max_drawdown < 0.2 { "⚠️ 风险适中" } else { "🚨 风险较高" };
        report.push_str(&format!("| 最大回撤 | {:.2}% | {} |\n", summary.max_drawdown * 100.0, dd_label));
        let sr_label = if summary.sharpe_ratio > 1.0 { "⭐ 优秀" } else if summary.sharpe_ratio > 0.5 { "✅ 良好" } else { "⚠️ 一般" };
        report.push_str(&format!("| 夏普比率 | {:.2} | {} |\n", summary.sharpe_ratio, sr_label));
        report.push_str(&format!("| 总交易次数 | {} 次 | - |\n", summary.total_trades));
        report.push_str(&format!("| 胜率 | {:.1}% | - |\n\n", summary.win_rate * 100.0));

        report.push_str("## 📋 个股回测明细\n\n");
        report.push_str("| 股票 | 代码 | 初始资金 | 期末市值 | 收益率 | 交易次数 |\n");
        report.push_str("|------|------|---------|---------|--------|----------|\n");
        for r in &self.single_results {
            let ret = (r.final_value / r.initial_capital - 1.0) * 100.0;
            let emoji = if ret > 0.0 { "🟢" } else { "🔴" };
            report.push_str(&format!(
                "| {} {} | {} | {:.0} | {:.0} | {} {:.2}% | {} |\n",
                emoji, r.name, r.code, r.initial_capital, r.final_value, emoji, ret, r.trades.len()
            ));
        }
        report.push_str("\n");

        report.push_str("> ⚠️ 本策略要求至少 205 日K线（200日均线预热），适合中长期持仓的均值回归波段操作。\n\n");
        report
    }
}

impl StrategyResult for PrecisionRsiResult {
    fn to_summary(&self) -> BacktestSummary { self.to_summary() }
    fn generate_report(&self) -> String { self.generate_report() }
    fn generate_chart(&self, path: &str) -> Result<PathBuf> { self.generate_chart(path) }
}

// ────────────────────────────── 精准RSI回测引擎 ──────────────────────────────

/// 精准 RSI 深度超卖均值回归回测引擎
pub struct PrecisionRsiBacktest {
    config: PrecisionRsiConfig,
}

impl PrecisionRsiBacktest {
    pub fn new(config: PrecisionRsiConfig) -> Self {
        Self { config }
    }

    /// 对单只股票运行历史回测（klines 须按日期升序排列）
    pub fn run_single(
        &self,
        code: &str,
        name: &str,
        klines: &[KlineData],
    ) -> Result<SinglePrecisionRsiResult> {
        let df = compute_precision_indicators(klines)?;
        let n = df.height();

        let dates = df.column("date")?.str()?.clone();
        let close_col = df.column("close")?.f64()?.clone();
        let rsi5_col = df.column("rsi_5")?.f64()?.clone();
        let ma200_col = df.column("ma200")?.f64()?.clone();

        // 预先收集 rsi_5 为 Vec<Option<f64>> 方便回望
        let rsi5_vec: Vec<Option<f64>> = (0..n).map(|i| rsi5_col.get(i)).collect();

        let mut cash = self.config.initial_capital;
        let mut shares: f64 = 0.0;
        let mut avg_cost: f64 = 0.0;
        let mut trades: Vec<Trade> = Vec::new();
        let mut daily_values: Vec<(chrono::DateTime<Local>, f64)> = Vec::new();
        let mut signals: Vec<Signal> = Vec::with_capacity(n);

        for i in 0..n {
            let close = match close_col.get(i) {
                Some(v) => v,
                None => { signals.push(Signal::Hold); continue; }
            };

            let date_str = dates.get(i).unwrap_or("1970-01-01");
            let naive = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
            let dt = Local
                .from_local_datetime(&naive.and_hms_opt(15, 0, 0).unwrap())
                .single()
                .unwrap_or_else(|| Local::now());

            daily_values.push((dt, cash + shares * close));

            // 需要足够的历史才能判断
            if i < 3 {
                signals.push(Signal::Hold);
                continue;
            }

            let rsi_now = match rsi5_vec[i] { Some(v) => v, None => { signals.push(Signal::Hold); continue; } };
            let rsi_1  = match rsi5_vec[i - 1] { Some(v) => v, None => { signals.push(Signal::Hold); continue; } };
            let rsi_2  = match rsi5_vec[i - 2] { Some(v) => v, None => { signals.push(Signal::Hold); continue; } };
            let rsi_3  = match rsi5_vec[i - 3] { Some(v) => v, None => { signals.push(Signal::Hold); continue; } };
            let ma200  = match ma200_col.get(i) { Some(v) => v, None => { signals.push(Signal::Hold); continue; } };

            // ──── 买入：四条件全部满足 ────
            if shares < 1.0 {
                let cond1 = rsi_now < 30.0;                          // 深度超卖
                let cond2 = rsi_now < rsi_1 && rsi_1 < rsi_2;       // 连续第三天走低
                let cond3 = rsi_3 < 60.0;                            // 三日前RSI < 60
                let cond4 = close > ma200;                            // 站稳200日均线

                if cond1 && cond2 && cond3 && cond4 {
                    let buy_price = close * (1.0 + self.config.slippage_rate);
                    let invest = cash.min(self.config.initial_capital * self.config.max_position_pct);
                    let buy_shares = (invest / buy_price).floor();
                    if buy_shares > 0.0 {
                        let amount = buy_shares * buy_price;
                        let comm = amount * self.config.commission_rate;
                        cash -= amount + comm;
                        avg_cost = (avg_cost * shares + amount) / (shares + buy_shares);
                        shares += buy_shares;
                        trades.push(Trade {
                            date: dt, code: code.to_string(), name: name.to_string(),
                            action: TradeAction::Buy, shares: buy_shares,
                            price: buy_price, amount, commission: comm,
                        });
                        signals.push(Signal::Buy);
                        continue;
                    }
                }
            }

            // ──── 卖出：5日RSI向上突破50 ────
            if shares > 0.0 {
                // 突破：前一日 < 50，当日 >= 50
                let cross_above_50 = rsi_1 < 50.0 && rsi_now >= 50.0;
                if cross_above_50 {
                    let sell_price = close * (1.0 - self.config.slippage_rate);
                    let amount = shares * sell_price;
                    let comm = amount * self.config.commission_rate;
                    cash += amount - comm;
                    trades.push(Trade {
                        date: dt, code: code.to_string(), name: name.to_string(),
                        action: TradeAction::Sell, shares,
                        price: sell_price, amount, commission: comm,
                    });
                    shares = 0.0;
                    avg_cost = 0.0;
                    signals.push(Signal::Sell);
                    continue;
                }
            }

            signals.push(Signal::Hold);
        }

        let final_close = close_col.get(n.saturating_sub(1)).unwrap_or(0.0);
        let final_value = cash + shares * final_close;

        Ok(SinglePrecisionRsiResult {
            code: code.to_string(),
            name: name.to_string(),
            initial_capital: self.config.initial_capital,
            final_value,
            trades,
            daily_values,
            signals,
            rsi5_values: rsi5_vec,
        })
    }

    /// 对多只股票批量回测，汇总为组合结果
    pub fn run_portfolio(
        &self,
        stocks: &[(String, String, Vec<KlineData>)],
    ) -> Result<PrecisionRsiResult> {
        if stocks.is_empty() {
            anyhow::bail!("股票列表为空");
        }

        let mut all_single: Vec<SinglePrecisionRsiResult> = Vec::new();
        for (code, name, klines) in stocks {
            let mut sorted = klines.clone();
            sorted.sort_by(|a, b| a.date.cmp(&b.date));

            if sorted.len() < 205 {
                warn!("[{}] K线不足205条，跳过精准RSI回测（当前{}条）", code, sorted.len());
                continue;
            }

            match self.run_single(code, name, &sorted) {
                Ok(r) => {
                    info!(
                        "[{}] 精准RSI回测完成: 收益 {:.2}%, 交易 {} 次",
                        code,
                        (r.final_value / r.initial_capital - 1.0) * 100.0,
                        r.trades.len()
                    );
                    all_single.push(r);
                }
                Err(e) => warn!("[{}] 精准RSI回测失败: {}", code, e),
            }
        }

        if all_single.is_empty() {
            anyhow::bail!("无有效精准RSI回测结果");
        }

        let (portfolio_daily_values, portfolio_trades) = {
            let mut date_set: BTreeSet<String> = BTreeSet::new();
            for r in &all_single {
                for (dt, _) in &r.daily_values {
                    date_set.insert(dt.format("%Y-%m-%d").to_string());
                }
            }
            let maps: Vec<HashMap<String, f64>> = all_single.iter()
                .map(|r| r.daily_values.iter().map(|(dt, v)| (dt.format("%Y-%m-%d").to_string(), *v)).collect())
                .collect();
            let total_initial = self.config.initial_capital * all_single.len() as f64;
            let mut pv: Vec<(chrono::DateTime<Local>, f64)> = date_set.iter().map(|ds| {
                let naive = NaiveDate::parse_from_str(ds, "%Y-%m-%d")
                    .unwrap_or_else(|_| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
                let dt = Local.from_local_datetime(&naive.and_hms_opt(15, 0, 0).unwrap())
                    .single().unwrap_or_else(|| Local::now());
                let sum: f64 = maps.iter().map(|m| m.get(ds).copied().unwrap_or(self.config.initial_capital)).sum();
                (dt, sum)
            }).collect();
            if let Some(&(_, first_val)) = pv.first() {
                if first_val > 0.0 {
                    let scale = total_initial / first_val;
                    for (_, v) in pv.iter_mut() { *v *= scale; }
                }
            }
            let mut all_trades: Vec<Trade> = all_single.iter().flat_map(|r| r.trades.clone()).collect();
            all_trades.sort_by_key(|t| t.date);
            (pv, all_trades)
        };

        Ok(PrecisionRsiResult {
            config: self.config.clone(),
            single_results: all_single,
            portfolio_daily_values,
            portfolio_trades,
        })
    }
}

/// `KlineStrategy` 包装，使精准RSI策略可注入 `HybridStrategy`
pub struct PrecisionRsiStrategy {
    backtest: PrecisionRsiBacktest,
}

impl PrecisionRsiStrategy {
    pub fn new(config: PrecisionRsiConfig) -> Self {
        Self { backtest: PrecisionRsiBacktest::new(config) }
    }
}

impl Default for PrecisionRsiStrategy {
    fn default() -> Self { Self::new(PrecisionRsiConfig::default()) }
}

impl KlineStrategy for PrecisionRsiStrategy {
    fn name(&self) -> &'static str { "精准RSI深度超卖均值回归" }

    fn description(&self) -> &'static str {
        "5日RSI<30+连续三日走低+三日前RSI<60+价格>MA200 买入；5日RSI上穿50 卖出"
    }

    fn run_portfolio_boxed(
        &self,
        stocks: &[(String, String, Vec<KlineData>)],
    ) -> Result<Box<dyn StrategyResult>> {
        let result = self.backtest.run_portfolio(stocks)?;
        Ok(Box::new(result))
    }
}

// ────────────────────────────── 回测引擎 ──────────────────────────────

/// RSI 超买超卖回测引擎
pub struct RsiBacktest {
    config: RsiConfig,
}

impl RsiBacktest {
    pub fn new(config: RsiConfig) -> Self {
        Self { config }
    }

    /// 对单只股票运行历史回测（klines 须按日期升序排列）
    pub fn run_single(
        &self,
        code: &str,
        name: &str,
        klines: &[KlineData],
    ) -> Result<SingleRsiResult> {
        let df = compute_rsi_indicators(klines, &self.config)?;
        let n = df.height();

        let dates = df.column("date")?.str()?.clone();
        let close_col = df.column("close")?.f64()?.clone();
        let rsi_col = df.column("rsi")?.f64()?.clone();
        let uptrend_col: Option<ChunkedArray<BooleanType>> = if self.config.use_trend_filter {
            df.column("is_uptrend")
                .ok()
                .and_then(|c| c.bool().ok().cloned())
        } else {
            None
        };

        let mut cash = self.config.initial_capital;
        let mut shares: f64 = 0.0;
        let mut avg_cost: f64 = 0.0;
        let mut trades: Vec<Trade> = Vec::new();
        let mut daily_values: Vec<(chrono::DateTime<Local>, f64)> = Vec::new();
        let mut signals: Vec<Signal> = Vec::with_capacity(n);
        let mut rsi_values: Vec<Option<f64>> = Vec::with_capacity(n);

        for i in 0..n {
            let close = match close_col.get(i) {
                Some(v) => v,
                None => {
                    signals.push(Signal::Hold);
                    rsi_values.push(None);
                    continue;
                }
            };

            let date_str = dates.get(i).unwrap_or("1970-01-01");
            let naive = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
            let dt = Local
                .from_local_datetime(&naive.and_hms_opt(15, 0, 0).unwrap())
                .single()
                .unwrap_or_else(|| Local::now());

            daily_values.push((dt, cash + shares * close));

            let rsi = rsi_col.get(i);
            rsi_values.push(rsi);

            let rsi = match rsi {
                Some(v) => v,
                None => {
                    signals.push(Signal::Hold);
                    continue;
                }
            };

            // 趋势过滤：未开启时默认允许交易
            let uptrend = uptrend_col
                .as_ref()
                .and_then(|c| c.get(i))
                .unwrap_or(true);

            // ──── 买入：RSI 超卖 ────
            if rsi < self.config.oversold && shares < 1.0 && uptrend {
                let buy_price = close * (1.0 + self.config.slippage_rate);
                let invest = cash.min(self.config.initial_capital * self.config.max_position_pct);
                let buy_shares = (invest / buy_price).floor();
                if buy_shares > 0.0 {
                    let amount = buy_shares * buy_price;
                    let comm = amount * self.config.commission_rate;
                    cash -= amount + comm;
                    avg_cost = (avg_cost * shares + amount) / (shares + buy_shares);
                    shares += buy_shares;
                    trades.push(Trade {
                        date: dt,
                        code: code.to_string(),
                        name: name.to_string(),
                        action: TradeAction::Buy,
                        shares: buy_shares,
                        price: buy_price,
                        amount,
                        commission: comm,
                    });
                    signals.push(Signal::Buy);
                    continue;
                }
            }

            // ──── 卖出：超买 或 回归均衡区且盈利 ────
            if shares > 0.0 {
                let should_sell = rsi > self.config.overbought
                    || (rsi >= self.config.exit_level && avg_cost > 0.0 && close > avg_cost);
                if should_sell {
                    let sell_price = close * (1.0 - self.config.slippage_rate);
                    let amount = shares * sell_price;
                    let comm = amount * self.config.commission_rate;
                    cash += amount - comm;
                    trades.push(Trade {
                        date: dt,
                        code: code.to_string(),
                        name: name.to_string(),
                        action: TradeAction::Sell,
                        shares,
                        price: sell_price,
                        amount,
                        commission: comm,
                    });
                    shares = 0.0;
                    avg_cost = 0.0;
                    signals.push(Signal::Sell);
                    continue;
                }
            }

            signals.push(Signal::Hold);
        }

        let final_close = close_col.get(n.saturating_sub(1)).unwrap_or(0.0);
        let final_value = cash + shares * final_close;

        Ok(SingleRsiResult {
            code: code.to_string(),
            name: name.to_string(),
            initial_capital: self.config.initial_capital,
            final_value,
            trades,
            daily_values,
            signals,
            rsi_values,
        })
    }

    /// 对多只股票批量回测，汇总为组合结果
    pub fn run_portfolio(
        &self,
        stocks: &[(String, String, Vec<KlineData>)],
    ) -> Result<RsiResult> {
        if stocks.is_empty() {
            anyhow::bail!("股票列表为空");
        }

        let mut all_single: Vec<SingleRsiResult> = Vec::new();
        for (code, name, klines) in stocks {
            let mut sorted = klines.clone();
            sorted.sort_by(|a, b| a.date.cmp(&b.date));

            if sorted.len() < self.config.rsi_period + 5 {
                warn!("[{}] K线不足，跳过 RSI 回测", code);
                continue;
            }

            match self.run_single(code, name, &sorted) {
                Ok(r) => {
                    info!(
                        "[{}] RSI 回测完成: 收益 {:.2}%, 交易 {} 次",
                        code,
                        (r.final_value / r.initial_capital - 1.0) * 100.0,
                        r.trades.len()
                    );
                    all_single.push(r);
                }
                Err(e) => warn!("[{}] RSI 回测失败: {}", code, e),
            }
        }

        if all_single.is_empty() {
            anyhow::bail!("无有效 RSI 回测结果");
        }

        let (portfolio_daily_values, portfolio_trades) = self.aggregate_portfolio(&all_single);
        Ok(RsiResult {
            config: self.config.clone(),
            single_results: all_single,
            portfolio_daily_values,
            portfolio_trades,
        })
    }

    /// 按日期等权合成组合净值（与布林带策略相同的聚合方式）
    fn aggregate_portfolio(
        &self,
        results: &[SingleRsiResult],
    ) -> (Vec<(chrono::DateTime<Local>, f64)>, Vec<Trade>) {
        let mut date_set: BTreeSet<String> = BTreeSet::new();
        for r in results {
            for (dt, _) in &r.daily_values {
                date_set.insert(dt.format("%Y-%m-%d").to_string());
            }
        }

        let maps: Vec<HashMap<String, f64>> = results
            .iter()
            .map(|r| {
                r.daily_values
                    .iter()
                    .map(|(dt, v)| (dt.format("%Y-%m-%d").to_string(), *v))
                    .collect()
            })
            .collect();

        let total_initial = self.config.initial_capital * results.len() as f64;

        let mut portfolio_values: Vec<(chrono::DateTime<Local>, f64)> = date_set
            .iter()
            .map(|date_str| {
                let naive = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                    .unwrap_or_else(|_| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
                let dt = Local
                    .from_local_datetime(&naive.and_hms_opt(15, 0, 0).unwrap())
                    .single()
                    .unwrap_or_else(|| Local::now());
                let sum: f64 = maps
                    .iter()
                    .map(|m| m.get(date_str).copied().unwrap_or(self.config.initial_capital))
                    .sum();
                (dt, sum)
            })
            .collect();

        // 归一化至初始资金
        if let Some(&(_, first_val)) = portfolio_values.first() {
            if first_val > 0.0 {
                let scale = total_initial / first_val;
                for (_, v) in portfolio_values.iter_mut() {
                    *v *= scale;
                }
            }
        }

        let mut all_trades: Vec<Trade> = results.iter().flat_map(|r| r.trades.clone()).collect();
        all_trades.sort_by_key(|t| t.date);

        (portfolio_values, all_trades)
    }
}

// ────────────────────────────── KlineStrategy / StrategyResult 绑定 ──────────────────────────────

/// `RsiResult` 实现 `StrategyResult`，可注册到 `HybridStrategy`
impl StrategyResult for RsiResult {
    fn to_summary(&self) -> BacktestSummary {
        self.to_summary()
    }

    fn generate_report(&self) -> String {
        self.generate_report()
    }

    fn generate_chart(&self, path: &str) -> Result<PathBuf> {
        self.generate_chart(path)
    }
}

/// `KlineStrategy` 包装，使 `RsiBacktest` 可直接注入 `HybridStrategy`
pub struct RsiStrategy {
    backtest: RsiBacktest,
}

impl RsiStrategy {
    pub fn new(config: RsiConfig) -> Self {
        Self {
            backtest: RsiBacktest::new(config),
        }
    }
}

impl Default for RsiStrategy {
    fn default() -> Self {
        Self::new(RsiConfig::default())
    }
}

impl KlineStrategy for RsiStrategy {
    fn name(&self) -> &'static str {
        "RSI超买超卖"
    }

    fn description(&self) -> &'static str {
        "基于RSI(14)，RSI<30超卖买入，RSI>70超买卖出，可选60日均线趋势过滤"
    }

    fn run_portfolio_boxed(
        &self,
        stocks: &[(String, String, Vec<KlineData>)],
    ) -> Result<Box<dyn StrategyResult>> {
        let result = self.backtest.run_portfolio(stocks)?;
        Ok(Box::new(result))
    }
}
