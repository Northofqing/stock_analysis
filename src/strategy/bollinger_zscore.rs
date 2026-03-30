//! 布林带 + Z-Score 均值回归策略
//!
//! 核心逻辑：
//! 1. 用 Polars 计算布林带（BB）和 Z-Score
//! 2. 当价格跌破下轨且 Z-Score < -2 时买入（超卖回归）
//! 3. 当价格突破上轨且 Z-Score > 2 时卖出（超买回归）
//! 4. 在历史K线上逐日回测，生成净值曲线和回测指标

use anyhow::Result;
use chrono::{Local, TimeZone, NaiveDate};
use log::{info, warn};
use polars::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

use super::core::{BacktestSummary, BacktestState, Trade, TradeAction};
use crate::data_provider::KlineData;
use super::{KlineStrategy, StrategyResult};

// ────────────────────────────── 策略参数 ──────────────────────────────

/// 布林带 + Z-Score 策略配置
#[derive(Debug, Clone)]
pub struct BollingerZScoreConfig {
    /// 布林带窗口期（默认 20）
    pub bb_window: usize,
    /// 布林带倍数（默认 2.0）
    pub bb_std_mult: f64,
    /// Z-Score 买入阈值（默认 -2.0，低于此值视为超卖）
    pub zscore_buy: f64,
    /// Z-Score 卖出阈值（默认 2.0，高于此值视为超买）
    pub zscore_sell: f64,
    /// Z-Score 平仓阈值（默认 0.0，回归均值时平仓）
    pub zscore_exit: f64,
    /// 初始资金
    pub initial_capital: f64,
    /// 单只股票最大仓位比例（0-1）
    pub max_position_pct: f64,
    /// 手续费率
    pub commission_rate: f64,
    /// 滑点率
    pub slippage_rate: f64,
}

impl Default for BollingerZScoreConfig {
    fn default() -> Self {
        Self {
            bb_window: 20,
            bb_std_mult: 2.0,
            zscore_buy: -2.0,
            zscore_sell: 2.0,
            zscore_exit: 0.0,
            initial_capital: 100_000.0,
            max_position_pct: 0.25,
            commission_rate: 0.0003,
            slippage_rate: 0.001,
        }
    }
}

// ────────────────────────────── Polars 指标计算 ──────────────────────────────

/// 用 Polars 一次性计算所有技术指标列
pub fn compute_indicators(klines: &[KlineData], config: &BollingerZScoreConfig) -> Result<DataFrame> {
    let n = klines.len();
    if n < config.bb_window {
        anyhow::bail!("K线数据不足 {} 条，无法计算 {} 日布林带", n, config.bb_window);
    }

    // 构建基础列
    let dates: Vec<String> = klines.iter().map(|k| k.date.format("%Y-%m-%d").to_string()).collect();
    let close: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let open: Vec<f64> = klines.iter().map(|k| k.open).collect();
    let high: Vec<f64> = klines.iter().map(|k| k.high).collect();
    let low: Vec<f64> = klines.iter().map(|k| k.low).collect();
    let volume: Vec<f64> = klines.iter().map(|k| k.volume).collect();

    let w = config.bb_window as u32;
    let mult = config.bb_std_mult;

    let df = df![
        "date"   => &dates,
        "open"   => &open,
        "high"   => &high,
        "low"    => &low,
        "close"  => &close,
        "volume" => &volume,
    ]?;

    // 用 Lazy API 一次计算布林带和 Z-Score
    let df = df.lazy()
        .with_columns([
            // 布林带中轨 = SMA(close, window)
            col("close")
                .rolling_mean(RollingOptionsFixedWindow {
                    window_size: w as usize,
                    min_periods: w as usize,
                    ..Default::default()
                })
                .alias("bb_mid"),
            // 布林带标准差
            col("close")
                .rolling_std(RollingOptionsFixedWindow {
                    window_size: w as usize,
                    min_periods: w as usize,
                    ..Default::default()
                })
                .alias("bb_std"),
        ])
        .with_columns([
            // 上轨 = mid + mult * std
            (col("bb_mid") + col("bb_std") * lit(mult)).alias("bb_upper"),
            // 下轨 = mid - mult * std
            (col("bb_mid") - col("bb_std") * lit(mult)).alias("bb_lower"),
            // Z-Score = (close - mid) / std
            ((col("close") - col("bb_mid")) / col("bb_std")).alias("zscore"),
        ])
        .collect()?;

    Ok(df)
}

// ────────────────────────────── 趋势过滤 ──────────────────────────────

/// 添加趋势过滤：60日均线判断趋势方向，只在上升趋势中做均值回归买入
fn add_trend_filter(df: &DataFrame) -> Result<DataFrame> {
    let result = df
        .clone()
        .lazy()
        .with_columns([
            // 60日均线作为趋势判断
            col("close")
                .rolling_mean(RollingOptionsFixedWindow {
                    window_size: 60,
                    min_periods: 60,
                    ..Default::default()
                })
                .alias("trend_ma"),
        ])
        .with_columns([
            // 只在上升趋势中做均值回归
            col("close").gt(col("trend_ma")).alias("is_uptrend"),
        ])
        .collect()?;
    Ok(result)
}

// ────────────────────────────── 信号定义 ──────────────────────────────

/// 策略交易信号（重新导出自 strategy 模块，此处保留以向后兼容）
pub use super::Signal;


// ────────────────────────────── 回测引擎 ──────────────────────────────

/// 布林带 + Z-Score 均值回归回测引擎
pub struct BollingerZScoreBacktest {
    config: BollingerZScoreConfig,
}

impl BollingerZScoreBacktest {
    pub fn new(config: BollingerZScoreConfig) -> Self {
        Self { config }
    }

    /// 对单只股票运行历史回测
    ///
    /// `klines` 应按**日期升序**排列（最早在前）
    pub fn run_single(&self, code: &str, name: &str, klines: &[KlineData]) -> Result<SingleBacktestResult> {
        let df = compute_indicators(klines, &self.config)?;
        let df = add_trend_filter(&df)?;
        let n = df.height();

        // 提取列
        let dates = df.column("date")?.str()?;
        let close_col = df.column("close")?.f64()?;
        let bb_upper = df.column("bb_upper")?.f64()?;
        let bb_lower = df.column("bb_lower")?.f64()?;
        let bb_mid = df.column("bb_mid")?.f64()?;
        let zscore_col = df.column("zscore")?.f64()?;
        let is_uptrend = df.column("is_uptrend")?.bool()?;

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
            let z = zscore_col.get(i);
            let upper = bb_upper.get(i);
            let lower = bb_lower.get(i);
            let mid = bb_mid.get(i);

            // 计算当天总资产
            let total_value = cash + shares * close;
            let date_str = dates.get(i).unwrap_or("1970-01-01");
            let naive = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .unwrap_or(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
            let dt = Local.from_local_datetime(&naive.and_hms_opt(15, 0, 0).unwrap())
                .single()
                .unwrap_or_else(|| Local::now());
            daily_values.push((dt, total_value));

            // 指标还没生效（NaN），跳过
            if z.is_none() || upper.is_none() || lower.is_none() || mid.is_none() {
                signals.push(Signal::Hold);
                continue;
            }
            let z = z.unwrap();
            let upper = upper.unwrap();
            let lower = lower.unwrap();
            let _mid = mid.unwrap();

            // ──── 信号判定 ────
            let uptrend = is_uptrend.get(i).unwrap_or(false);

            // 买入条件：价格 <= 下轨 且 Z-Score <= 买入阈值，且当前无满仓，且处于上升趋势
            if close <= lower && z <= self.config.zscore_buy && shares < 1.0 && uptrend {
                let buy_price = close * (1.0 + self.config.slippage_rate);
                let max_invest = self.config.initial_capital * self.config.max_position_pct;
                let invest = cash.min(max_invest);
                let buy_shares = (invest / buy_price).floor();
                if buy_shares > 0.0 {
                    let amount = buy_shares * buy_price;
                    let comm = amount * self.config.commission_rate;
                    cash -= amount + comm;
                    let old_val = avg_cost * shares;
                    shares += buy_shares;
                    avg_cost = (old_val + amount) / shares;
                    trades.push(Trade {
                        date: dt, code: code.to_string(), name: name.to_string(),
                        action: TradeAction::Buy, shares: buy_shares,
                        price: buy_price, amount, commission: comm,
                    });
                    signals.push(Signal::Buy);
                    continue;
                }
            }

            // 卖出条件1：价格 >= 上轨 且 Z-Score >= 卖出阈值（超买止盈）
            // 卖出条件2：Z-Score 回归到 exit 阈值附近（回归均值平仓）
            if shares > 0.0 {
                let should_sell = (close >= upper && z >= self.config.zscore_sell)
                    || (z >= self.config.zscore_exit && avg_cost > 0.0 && close > avg_cost);
                if should_sell {
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

        // 如果最后还持仓，按最后收盘价计算市值
        let final_close = close_col.get(n - 1).unwrap_or(0.0);
        let final_value = cash + shares * final_close;

        Ok(SingleBacktestResult {
            code: code.to_string(),
            name: name.to_string(),
            initial_capital: self.config.initial_capital,
            final_value,
            trades,
            daily_values,
            signals,
            indicator_df: df,
        })
    }

    /// 对多只股票批量回测，汇总为组合回测结果
    pub fn run_portfolio(
        &self,
        stocks: &[(String, String, Vec<KlineData>)], // (code, name, klines)
    ) -> Result<BollingerZScoreResult> {
        if stocks.is_empty() {
            anyhow::bail!("股票列表为空");
        }

        let mut all_single: Vec<SingleBacktestResult> = Vec::new();

        for (code, name, klines) in stocks {
            // klines 从数据源拿到通常是降序（最新在前），需要反转为升序
            let mut sorted = klines.clone();
            sorted.sort_by(|a, b| a.date.cmp(&b.date));

            if sorted.len() < self.config.bb_window + 5 {
                warn!("[{}] K线不足，跳过", code);
                continue;
            }

            match self.run_single(code, name, &sorted) {
                Ok(result) => {
                    info!(
                        "[{}] 回测完成: 收益 {:.2}%, 交易 {} 次",
                        code,
                        (result.final_value / result.initial_capital - 1.0) * 100.0,
                        result.trades.len()
                    );
                    all_single.push(result);
                }
                Err(e) => {
                    warn!("[{}] 回测失败: {}", code, e);
                }
            }
        }

        if all_single.is_empty() {
            anyhow::bail!("无有效回测结果");
        }

        // 汇总组合级别的净值曲线（等权平均）
        let portfolio = self.aggregate_portfolio(&all_single);

        Ok(BollingerZScoreResult {
            config: self.config.clone(),
            single_results: all_single,
            portfolio_daily_values: portfolio.0,
            portfolio_trades: portfolio.1,
        })
    }

    /// 按日期对齐，等权合成组合净值
    fn aggregate_portfolio(
        &self,
        results: &[SingleBacktestResult],
    ) -> (Vec<(chrono::DateTime<Local>, f64)>, Vec<Trade>) {
        // 收集所有日期并排序去重
        let mut date_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for r in results {
            for (dt, _) in &r.daily_values {
                date_set.insert(dt.format("%Y-%m-%d").to_string());
            }
        }

        // 每个股票建立 date→value 的映射
        let maps: Vec<HashMap<String, f64>> = results
            .iter()
            .map(|r| {
                r.daily_values
                    .iter()
                    .map(|(dt, v)| (dt.format("%Y-%m-%d").to_string(), *v))
                    .collect()
            })
            .collect();

        let stock_count = results.len() as f64;
        let total_initial = self.config.initial_capital * stock_count;

        let mut portfolio_values: Vec<(chrono::DateTime<Local>, f64)> = Vec::new();
        for date_str in &date_set {
            let naive = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .unwrap_or(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
            let dt = Local
                .from_local_datetime(&naive.and_hms_opt(15, 0, 0).unwrap())
                .single()
                .unwrap_or_else(|| Local::now());

            let sum: f64 = maps
                .iter()
                .map(|m| m.get(date_str).copied().unwrap_or(self.config.initial_capital))
                .sum();
            portfolio_values.push((dt, sum));
        }

        // 归一化为初始资金
        if let Some(&(_, first_val)) = portfolio_values.first() {
            if first_val > 0.0 {
                let scale = total_initial / first_val;
                for (_, v) in portfolio_values.iter_mut() {
                    *v *= scale;
                }
            }
        }

        // 合并所有交易
        let mut all_trades: Vec<Trade> = results.iter().flat_map(|r| r.trades.clone()).collect();
        all_trades.sort_by_key(|t| t.date);

        (portfolio_values, all_trades)
    }
}

// ────────────────────────────── 结果结构 ──────────────────────────────

/// 单只股票回测结果
pub struct SingleBacktestResult {
    pub code: String,
    pub name: String,
    pub initial_capital: f64,
    pub final_value: f64,
    pub trades: Vec<Trade>,
    pub daily_values: Vec<(chrono::DateTime<Local>, f64)>,
    pub signals: Vec<Signal>,
    pub indicator_df: DataFrame,
}

/// 组合回测汇总结果
pub struct BollingerZScoreResult {
    pub config: BollingerZScoreConfig,
    pub single_results: Vec<SingleBacktestResult>,
    pub portfolio_daily_values: Vec<(chrono::DateTime<Local>, f64)>,
    pub portfolio_trades: Vec<Trade>,
}

impl BollingerZScoreResult {
    /// 转化为通用 BacktestSummary（复用现有回测报告和图表生成）
    pub fn to_summary(&self) -> BacktestSummary {
        let total_initial = self.config.initial_capital * self.single_results.len() as f64;

        // 构造 BacktestState 复用其指标计算
        let mut state = BacktestState::new(total_initial);
        state.daily_values = self.portfolio_daily_values.clone();
        state.trades = self.portfolio_trades.clone();

        BacktestSummary::from_state(&state, total_initial)
    }

    /// 生成图表（复用 BacktestSummary 的图表逻辑）
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
        report.push_str("# 📊 布林带+Z-Score 均值回归策略回测报告\n\n");
        report.push_str(&format!("**生成时间**: {}\n\n", now));
        report.push_str("---\n\n");

        // 策略参数
        report.push_str("## ⚙️ 策略参数\n\n");
        report.push_str("| 参数 | 值 |\n");
        report.push_str("|------|----|\n");
        report.push_str(&format!("| 布林带窗口 | {} 日 |\n", self.config.bb_window));
        report.push_str(&format!("| 布林带倍数 | {:.1}σ |\n", self.config.bb_std_mult));
        report.push_str(&format!("| Z-Score 买入阈值 | {:.1} |\n", self.config.zscore_buy));
        report.push_str(&format!("| Z-Score 卖出阈值 | {:.1} |\n", self.config.zscore_sell));
        report.push_str(&format!("| Z-Score 平仓阈值 | {:.1} |\n", self.config.zscore_exit));
        report.push_str(&format!("| 单股最大仓位 | {:.0}% |\n", self.config.max_position_pct * 100.0));
        report.push_str(&format!("| 手续费率 | {:.2}‰ |\n", self.config.commission_rate * 1000.0));
        report.push_str(&format!("| 滑点率 | {:.1}‰ |\n", self.config.slippage_rate * 1000.0));
        report.push_str("\n");

        // 组合汇总
        report.push_str("## 📈 组合回测结果\n\n");
        report.push_str("| 指标 | 数值 | 说明 |\n");
        report.push_str("|------|------|------|\n");
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
            summary.total_return * 100.0, ret_emoji
        ));
        report.push_str(&format!(
            "| 年化收益率 | {:.2}% | - |\n",
            summary.annual_return * 100.0
        ));
        let dd_emoji = if summary.max_drawdown < 0.1 {
            "🛡️ 风险较低"
        } else if summary.max_drawdown < 0.2 {
            "⚠️ 风险适中"
        } else {
            "🚨 风险较高"
        };
        report.push_str(&format!(
            "| 最大回撤 | {:.2}% | {} |\n",
            summary.max_drawdown * 100.0, dd_emoji
        ));
        let sr_emoji = if summary.sharpe_ratio > 1.0 {
            "⭐ 优秀"
        } else if summary.sharpe_ratio > 0.5 {
            "✅ 良好"
        } else {
            "⚠️ 一般"
        };
        report.push_str(&format!(
            "| 夏普比率 | {:.2} | {} |\n",
            summary.sharpe_ratio, sr_emoji
        ));
        report.push_str(&format!(
            "| 总交易次数 | {} 次 | - |\n",
            summary.total_trades
        ));
        report.push_str(&format!(
            "| 胜率 | {:.1}% | - |\n",
            summary.win_rate * 100.0
        ));
        report.push_str("\n");

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
        report.push_str("**布林带 + Z-Score 均值回归策略**基于统计学均值回归原理：\n\n");
        report.push_str("1. **布林带(Bollinger Bands)**：以 N 日移动平均线为中轨，上下各加减 K 倍标准差，量化价格波动区间\n");
        report.push_str("2. **Z-Score**：标准化衡量当前价格偏离均值的程度，`Z = (Price - Mean) / Std`\n");
        report.push_str("3. **买入信号**：价格触及/跌破下轨 **且** Z-Score ≤ 买入阈值 → 超卖，看多回归\n");
        report.push_str("4. **卖出信号**：价格触及/突破上轨 **且** Z-Score ≥ 卖出阈值 → 超买止盈；或 Z-Score 回归至 0 附近且盈利 → 均值回归平仓\n\n");
        report.push_str("> ⚠️ 本策略适合震荡市，趋势行情中可能频繁止损。\n\n");

        report
    }
}

// ────────────────────────────── KlineStrategy / StrategyResult 绑定 ──────────────────────────────

/// `BollingerZScoreResult` 实现 `StrategyResult`，可注册到 `HybridStrategy`
impl StrategyResult for BollingerZScoreResult {
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

/// `KlineStrategy` 包装，使布林带策略可直接注入 `HybridStrategy`
pub struct BollingerZScoreStrategy {
    backtest: BollingerZScoreBacktest,
}

impl BollingerZScoreStrategy {
    pub fn new(config: BollingerZScoreConfig) -> Self {
        Self {
            backtest: BollingerZScoreBacktest::new(config),
        }
    }
}

impl Default for BollingerZScoreStrategy {
    fn default() -> Self {
        Self::new(BollingerZScoreConfig::default())
    }
}

impl KlineStrategy for BollingerZScoreStrategy {
    fn name(&self) -> &'static str {
        "布林带+Z-Score均值回归"
    }

    fn description(&self) -> &'static str {
        "价格跌破布林下轨且Z-Score<-2时买入，突破上轨且Z-Score>2时卖出，含60日均线趋势过滤"
    }

    fn run_portfolio_boxed(
        &self,
        stocks: &[(String, String, Vec<KlineData>)],
    ) -> Result<Box<dyn StrategyResult>> {
        let result = self.backtest.run_portfolio(stocks)?;
        Ok(Box::new(result))
    }
}
