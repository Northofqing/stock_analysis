//! RSI 通用超买超卖策略（增强版：Wilder + 过滤器 + 冷却期 + 分档加仓）
//!
//! 详见 `super` 模块文档。

use anyhow::Result;
use chrono::{Local, NaiveDate, TimeZone};
use log::{info, warn};
use polars::prelude::*;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use super::super::core::{BacktestState, BacktestSummary, Trade, TradeAction};
use super::super::{KlineStrategy, Signal, StrategyResult};
use super::common::{compute_ema_vec, compute_macd_vec, compute_rsi_wilder, compute_sma_vec, compute_vwap_monthly_vec};
use crate::data_provider::KlineData;

// ────────────────────────────── 策略参数 ──────────────────────────────

/// RSI 超买超卖策略配置（增强版：附加 VWAP/MACD/EMA/SMA 过滤器）
#[derive(Debug, Clone)]
pub struct RsiConfig {
    /// RSI 计算周期（默认 14）
    pub rsi_period: usize,
    /// 超卖阈值：RSI 低于此值视为超卖，触发买入（默认 30）
    pub oversold: f64,
    /// 超买阈值：RSI 高于此值视为超买，触发卖出（默认 70）
    pub overbought: f64,
    /// 均衡平仓阈值：RSI 回到此值且持仓盈利时平仓（默认 50）
    pub exit_level: f64,
    /// 是否启用均线趋势过滤（开启后只在价格 > `trend_ma_period` 日均线时买入）
    pub use_trend_filter: bool,
    /// 趋势过滤使用的均线周期（默认 60，可设 120/200 与深度超卖并存）
    pub trend_ma_period: usize,
    /// 是否启用月度 VWAP 过滤（买入时价格须 > VWAP）
    pub use_vwap_filter: bool,
    /// 是否启用 MACD 过滤（买入要求 MACD 柱 > 0，持仓中柱转负卖出）
    pub use_macd_filter: bool,
    /// 是否启用 EMA20 + SMA20 过滤（买入要求价格 > 两者，跌破两者卖出）
    pub use_ema_sma_filter: bool,
    /// EMA 周期（默认 20）
    pub ema_period: usize,
    /// SMA 周期（默认 20）
    pub sma_period: usize,
    /// MACD 快线周期（默认 12）
    pub macd_fast: usize,
    /// MACD 慢线周期（默认 26）
    pub macd_slow: usize,
    /// MACD 信号线周期（默认 9）
    pub macd_signal: usize,
    /// 增强过滤器最少通过数量（评分制：启用的过滤器中至少通过几个才允许买入，默认 1）
    /// 设为 0 表示不需要任何增强过滤器通过，仅依赖 RSI 超卖信号
    pub min_buy_filters: usize,
    /// 若为 true 则要求所有启用的过滤器全部通过（覆盖 `min_buy_filters`）
    pub require_all_filters: bool,
    /// 卖出后冷却期（至少间隔多少根 K 线才允许再次买入，默认 10）
    pub cooldown_bars: usize,
    /// 持仓最少持有 K 线数（低于此值不允许非止损/止盈的卖出，0=关闭）
    pub min_hold_bars: usize,
    /// 固定止损百分比（例如 0.05 表示 -5% 止损；0 表示不启用）
    pub stop_loss_pct: f64,
    /// 固定止盈百分比（例如 0.08 表示 +8% 止盈；0 表示不启用）
    pub take_profit_pct: f64,
    /// 持仓中若 RSI 继续下探 `add_on_rsi_delta`（如 5.0）且仓位未满，触发加仓（0=关闭）
    pub add_on_rsi_delta: f64,
    /// 单次加仓最大仓位比例（相对初始资金）
    pub add_on_position_pct: f64,
    /// 买入前要求 RSI 连续 N 根 K 线回升（0=不要求）
    pub rsi_rising_confirm_bars: usize,
    /// 时间周期标注（如 "1h"、"1d"，不影响计算逻辑，仅用于报告标注）
    pub timeframe: String,
    /// 回测起始日期（None 表示不限制）
    pub start_date: Option<NaiveDate>,
    /// 回测结束日期（None 表示不限制）
    pub end_date: Option<NaiveDate>,
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
            rsi_period: 21,
            oversold: 30.0,
            overbought: 70.0,
            exit_level: 65.0,
            use_trend_filter: true,
            trend_ma_period: 60,
            use_vwap_filter: true,
            use_macd_filter: true,
            use_ema_sma_filter: true,
            ema_period: 20,
            sma_period: 20,
            macd_fast: 12,
            macd_slow: 26,
            macd_signal: 9,
            min_buy_filters: 1,
            require_all_filters: false,
            cooldown_bars: 10,
            min_hold_bars: 0,
            stop_loss_pct: 0.0,
            take_profit_pct: 0.0,
            add_on_rsi_delta: 0.0,
            add_on_position_pct: 0.0,
            rsi_rising_confirm_bars: 0,
            timeframe: "1h".to_string(),
            start_date: Some(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()),
            end_date: None,
            initial_capital: 100_000.0,
            max_position_pct: 0.50,
            commission_rate: 0.0003,
            slippage_rate: 0.001,
        }
    }
}

impl RsiConfig {
    /// 基线（现行默认 1h 配置）——作为对照组
    pub fn preset_baseline() -> Self {
        Self::default()
    }

    /// v1: 修正为日K + 取消 MA60 趋势过滤冲突 + 冷却期缩短
    pub fn preset_daily_v1() -> Self {
        Self {
            rsi_period: 14,
            oversold: 30.0,
            overbought: 70.0,
            exit_level: 55.0,
            use_trend_filter: false, // 解除 RSI<30 与 close>MA60 的互斥
            trend_ma_period: 60,
            cooldown_bars: 3,
            min_buy_filters: 1,
            require_all_filters: false,
            min_hold_bars: 0,
            stop_loss_pct: 0.0,
            take_profit_pct: 0.0,
            add_on_rsi_delta: 0.0,
            add_on_position_pct: 0.0,
            timeframe: "1d".to_string(),
            ..Self::default()
        }
    }

    /// v2: 趋势过滤换到 MA200（长期过滤，不会夹死超卖信号）
    pub fn preset_daily_v2() -> Self {
        Self {
            use_trend_filter: true,
            trend_ma_period: 200,
            ..Self::preset_daily_v1()
        }
    }

    /// v3: 所有过滤器全部通过 + 超卖收紧到 25（精选信号）
    pub fn preset_daily_v3_strict() -> Self {
        Self {
            oversold: 25.0,
            require_all_filters: true,
            ..Self::preset_daily_v2()
        }
    }

    /// v4: 加入止盈 +8% / 止损 -5%（不对称：期望胜率上升）
    pub fn preset_daily_v4_stop_take() -> Self {
        Self {
            stop_loss_pct: 0.05,
            take_profit_pct: 0.08,
            min_hold_bars: 2,
            ..Self::preset_daily_v3_strict()
        }
    }

    /// v5: 宽止盈 +12% / 紧止损 -3%（高胜率偏向）
    pub fn preset_daily_v5_high_winrate() -> Self {
        Self {
            oversold: 22.0,
            exit_level: 60.0,
            stop_loss_pct: 0.03,
            take_profit_pct: 0.12,
            // 禁用 MACD/EMA-SMA 的"趋势破位"卖出，避免过早止损
            use_macd_filter: false,
            use_ema_sma_filter: false,
            min_hold_bars: 3,
            ..Self::preset_daily_v4_stop_take()
        }
    }

    /// v6: 干净的"RSI+MA200 趋势+止盈止损"，不使用额外买入过滤器
    pub fn preset_daily_v6_clean() -> Self {
        Self {
            rsi_period: 14,
            oversold: 30.0,
            overbought: 70.0,
            exit_level: 55.0,
            use_trend_filter: true,
            trend_ma_period: 200,
            use_vwap_filter: false,
            use_macd_filter: false,
            use_ema_sma_filter: false,
            min_buy_filters: 0,
            require_all_filters: false,
            cooldown_bars: 3,
            min_hold_bars: 2,
            stop_loss_pct: 0.03,
            take_profit_pct: 0.05,
            add_on_rsi_delta: 0.0,
            add_on_position_pct: 0.0,
            timeframe: "1d".to_string(),
            ..Self::default()
        }
    }

    /// v7: v6 + 更深超卖 + 更宽止盈（更稀有但更高盈亏比）
    pub fn preset_daily_v7_deeper() -> Self {
        Self {
            oversold: 25.0,
            stop_loss_pct: 0.04,
            take_profit_pct: 0.08,
            ..Self::preset_daily_v6_clean()
        }
    }

    /// v8: v6 + VWAP/MACD/EMA 三过滤（评分≥2）进一步精选
    pub fn preset_daily_v8_score2() -> Self {
        Self {
            use_vwap_filter: true,
            use_macd_filter: true,
            use_ema_sma_filter: true,
            min_buy_filters: 2,
            require_all_filters: false,
            stop_loss_pct: 0.04,
            take_profit_pct: 0.06,
            ..Self::preset_daily_v6_clean()
        }
    }

    /// v9: v7 + 仅在"RSI 连续 2 日回升"时买入（通过在 run_single 中判定，实际以 rsi_rising_required 开关启用）
    /// 暂沿用 v7，改为在胜率验证环节用筛选过滤器实现
    pub fn preset_daily_v9_reversal() -> Self {
        Self {
            oversold: 28.0,
            stop_loss_pct: 0.03,
            take_profit_pct: 0.06,
            min_hold_bars: 3,
            // 要求 RSI 回升确认：在 buy 分支加判定
            rsi_rising_confirm_bars: 1,
            ..Self::preset_daily_v7_deeper()
        }
    }

    /// v10: v7 但取消止损（"拿到盈利才卖"），胜率为王
    pub fn preset_daily_v10_no_stop() -> Self {
        Self {
            stop_loss_pct: 0.0, // 关闭硬止损
            take_profit_pct: 0.08,
            oversold: 25.0,
            exit_level: 55.0,
            ..Self::preset_daily_v7_deeper()
        }
    }

    /// v11: v10 + 回升 1 根确认（过滤"接飞刀"）
    pub fn preset_daily_v11_no_stop_rising() -> Self {
        Self {
            rsi_rising_confirm_bars: 1,
            ..Self::preset_daily_v10_no_stop()
        }
    }

    /// v12: v10 + 超卖更深（22）+ 仅 MA200 上方
    pub fn preset_daily_v12_deep_no_stop() -> Self {
        Self {
            oversold: 22.0,
            take_profit_pct: 0.10,
            ..Self::preset_daily_v10_no_stop()
        }
    }

    /// v13: v11 + 超卖 22 + 回升 2 根（最严格）
    pub fn preset_daily_v13_strict_rising() -> Self {
        Self {
            oversold: 22.0,
            rsi_rising_confirm_bars: 2,
            take_profit_pct: 0.10,
            ..Self::preset_daily_v11_no_stop_rising()
        }
    }
}

// ────────────────────────────── Polars 指标计算 ──────────────────────────────

/// 使用 Wilder 指数平滑计算 RSI 及可选趋势过滤指标（通用策略用）
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

    // 使用 Wilder 指数平滑计算 RSI（替代 Polars rolling_mean SMA 版本）
    let rsi_wilder = compute_rsi_wilder(&close, config.rsi_period);
    // 前 rsi_period 个点为预热期，设为 null
    let rsi_series: Vec<Option<f64>> = rsi_wilder.iter().enumerate().map(|(i, &v)| {
        if i < config.rsi_period { None } else { Some(v) }
    }).collect();

    let df = df![
        "date"  => &dates,
        "close" => &close,
        "rsi"   => &rsi_series,
    ]?;

    // 可选：追加趋势过滤列
    if config.use_trend_filter {
        let trend_window = config.trend_ma_period.max(2);
        let df = df
            .lazy()
            .with_columns([col("close")
                .rolling_mean(RollingOptionsFixedWindow {
                    window_size: trend_window,
                    min_periods: trend_window,
                    ..Default::default()
                })
                .alias("trend_ma")])
            .with_columns([col("close").gt(col("trend_ma")).alias("is_uptrend")])
            .collect()?;
        return Ok(df);
    }

    Ok(df)
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
        report.push_str("# 📊 RSI 增强策略 v2 回测报告（Wilder RSI + 跌势减缓过滤 + 冷却期 + 分档加仓）\n\n");
        report.push_str(&format!("**生成时间**: {}\n\n", now));
        report.push_str("---\n\n");

        // 策略参数
        report.push_str("## ⚙️ 策略参数\n\n");
        report.push_str("| 参数 | 值 |\n|------|----|\n");
        report.push_str(&format!("| 回测区间 | {} ~ {} |\n",
            self.config.start_date.map_or("不限".to_string(), |d| d.format("%Y-%m-%d").to_string()),
            self.config.end_date.map_or("不限".to_string(), |d| d.format("%Y-%m-%d").to_string()),
        ));
        report.push_str(&format!("| 时间周期 | {} |\n", self.config.timeframe));
        report.push_str(&format!("| RSI 周期 | {} |\n", self.config.rsi_period));
        report.push_str(&format!("| 超卖阈值 | {:.0} |\n", self.config.oversold));
        report.push_str(&format!("| 超买阈值 | {:.0} |\n", self.config.overbought));
        report.push_str(&format!("| 均衡平仓阈值 | {:.0} |\n", self.config.exit_level));
        report.push_str(&format!(
            "| 趋势过滤(MA60) | {} |\n",
            if self.config.use_trend_filter { "开启" } else { "关闭" }
        ));
        report.push_str(&format!(
            "| VWAP 月度过滤 | {} |\n",
            if self.config.use_vwap_filter { "开启" } else { "关闭" }
        ));
        report.push_str(&format!(
            "| MACD 过滤 ({}/{}/{}) | {} |\n",
            self.config.macd_fast, self.config.macd_slow, self.config.macd_signal,
            if self.config.use_macd_filter { "开启" } else { "关闭" }
        ));
        report.push_str(&format!(
            "| EMA{}/SMA{} 过滤 | {} |\n",
            self.config.ema_period, self.config.sma_period,
            if self.config.use_ema_sma_filter { "开启" } else { "关闭" }
        ));
        report.push_str(&format!(
            "| 过滤器模式 | 评分制（≥{} 个通过即可买入） |\n",
            self.config.min_buy_filters
        ));
        report.push_str(&format!(
            "| 卖出后冷却期 | {} 根K线 |\n",
            self.config.cooldown_bars
        ));
        report.push_str(&format!(
            "| 单股基础仓位 | {:.0}%（RSI<20: {:.0}%, RSI<15: {:.0}%） |\n",
            self.config.max_position_pct * 100.0,
            self.config.max_position_pct * 140.0,
            (0.70_f64.min(self.config.max_position_pct * 2.0)) * 100.0
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
        report.push_str("**RSI 增强策略 v2**：Wilder EMA + 跌势减缓过滤 + 冷却期 + 分档加仓\n\n");
        report.push_str("### 核心改进\n");
        report.push_str("1. **Wilder 指数平滑 RSI**：替代 SMA，信号更灵敏精准\n");
        report.push_str("2. **跌势减缓过滤**：买入过滤器从「趋势向上」改为「跌势放缓」，解决超卖与趋势矛盾\n");
        report.push_str("3. **交易冷却期**：卖出后间隔 N 根K线才允许买入，减少无效来回交易\n");
        report.push_str("4. **RSI 分档加仓**：RSI 越低仓位越重，提高有效信号收益率\n");
        report.push_str(&format!("5. **均衡平仓阈值提高至 {}**：让利润充分奔跑\n\n", self.config.exit_level));
        report.push_str("### 增强过滤器（评分制 — 跌势减缓确认）\n");
        report.push_str(&format!(
            "启用的过滤器中至少 **{}** 个通过即允许买入\n\n",
            self.config.min_buy_filters
        ));
        report.push_str("| 过滤器 | 条件 | 作用 |\n|--------|------|------|\n");
        report.push_str("| VWAP 月度 | 价格在 VWAP ±3% 范围内 | 接近机构成本线，有支撑 |\n");
        report.push_str("| MACD (12/26/9) | 柱状线负值收窄 或 已转正 | 跌势放缓/动能恢复 |\n");
        report.push_str("| EMA20 / SMA20 | 价格在均线下方 3% 以内 | 接近支撑位，非深度破位 |\n");
        report.push_str("| MA60 趋势 | 价格 > MA60 | 中期趋势过滤 |\n\n");
        report.push_str("### 卖出条件\n");
        report.push_str("| 条件 | 说明 |\n|------|------|\n");
        report.push_str(&format!("| RSI > {} | 超买区域平仓 |\n", self.config.overbought));
        report.push_str(&format!("| RSI ≥ {} 且盈利 | 均衡平仓锁利 |\n", self.config.exit_level));
        report.push_str("| MACD 柱状线连续 2 根为负 | 持续动能衰竭 |\n");
        report.push_str("| 价格同时跌破 EMA20 和 SMA20 | 短期趋势破位 |\n\n");
        report.push_str(&format!("> 💡 时间周期: {} | RSI(Wilder {}) | 冷却期 {} 根K线\n\n", self.config.timeframe, self.config.rsi_period, self.config.cooldown_bars));

        report
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

        // ── 预计算增强指标 ──
        let close_vec: Vec<f64> = klines.iter().map(|k| k.close).collect();
        let ema20 = if self.config.use_ema_sma_filter {
            compute_ema_vec(&close_vec, self.config.ema_period)
        } else {
            vec![None; n]
        };
        let sma20 = if self.config.use_ema_sma_filter {
            compute_sma_vec(&close_vec, self.config.sma_period)
        } else {
            vec![None; n]
        };
        let (_macd_line, _signal_line, macd_hist) = if self.config.use_macd_filter {
            compute_macd_vec(&close_vec, self.config.macd_fast, self.config.macd_slow, self.config.macd_signal)
        } else {
            (vec![None; n], vec![None; n], vec![None; n])
        };
        let vwap_monthly = if self.config.use_vwap_filter {
            compute_vwap_monthly_vec(klines)
        } else {
            vec![None; n]
        };

        let mut cash = self.config.initial_capital;
        let mut shares: f64 = 0.0;
        let mut avg_cost: f64 = 0.0;
        let mut trades: Vec<Trade> = Vec::new();
        let mut daily_values: Vec<(chrono::DateTime<Local>, f64)> = Vec::new();
        let mut signals: Vec<Signal> = Vec::with_capacity(n);
        let mut rsi_values: Vec<Option<f64>> = Vec::with_capacity(n);
        let mut bars_since_sell: usize = usize::MAX; // 卖出后冷却计数
        let mut bars_held: usize = 0; // 当前持仓 K 线数
        let mut min_rsi_in_pos: f64 = f64::MAX; // 持仓期间的最低 RSI（用于分档加仓判定）

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

            // 日期范围过滤：仅在范围内记录净值和交易
            let in_date_range = self.config.start_date.map_or(true, |s| naive >= s)
                && self.config.end_date.map_or(true, |e| naive <= e);

            if in_date_range {
                daily_values.push((dt, cash + shares * close));
            }

            // 日期范围外不交易，仅计算指标
            if !in_date_range {
                signals.push(Signal::Hold);
                rsi_values.push(rsi_col.get(i));
                continue;
            }

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

            // 冷却计数递增
            if bars_since_sell < usize::MAX {
                bars_since_sell += 1;
            }

            // ── 增强过滤器（评分制：改为"跌势减缓"确认） ──
            let mut buy_filter_score: usize = 0;
            let mut buy_filter_total: usize = 0;

            // VWAP monthly: 价格接近 VWAP（在 VWAP ±3% 范围内，说明在机构成本区附近）
            if self.config.use_vwap_filter {
                buy_filter_total += 1;
                if let Some(vwap_val) = vwap_monthly.get(i).copied().flatten() {
                    if close >= vwap_val * 0.97 && close <= vwap_val * 1.03 {
                        buy_filter_score += 1;
                    }
                }
            }

            // MACD: 柱状线收窄（负值绝对值缩小，跌势减缓）
            if self.config.use_macd_filter && i > 0 {
                buy_filter_total += 1;
                if let (Some(hist_now), Some(hist_prev)) =
                    (macd_hist.get(i).copied().flatten(), macd_hist.get(i - 1).copied().flatten())
                {
                    // 柱状线为负但绝对值在缩小（跌势放缓）
                    if hist_now < 0.0 && hist_prev < 0.0 && hist_now.abs() < hist_prev.abs() {
                        buy_filter_score += 1;
                    }
                    // 或柱状线已转正（动能恢复）
                    if hist_now > 0.0 {
                        buy_filter_score += 1;
                    }
                }
            }

            // EMA20 + SMA20: 价格接近或触及均线（跌势支撑位确认，而非必须在上方）
            if self.config.use_ema_sma_filter {
                buy_filter_total += 1;
                let near_ema = ema20.get(i).copied().flatten().map_or(false, |v| {
                    close >= v * 0.97 // 在 EMA 下方 3% 以内即可
                });
                let near_sma = sma20.get(i).copied().flatten().map_or(false, |v| {
                    close >= v * 0.97
                });
                if near_ema || near_sma {
                    buy_filter_score += 1;
                }
            }

            // 评分通过条件：
            //   - require_all_filters=true：所有启用的过滤器都必须通过
            //   - 否则：至少满足 min_buy_filters 个（若无启用过滤器则自动通过）
            let filters_ok = if buy_filter_total == 0 {
                true
            } else if self.config.require_all_filters {
                buy_filter_score >= buy_filter_total
            } else {
                buy_filter_score >= self.config.min_buy_filters
            };

            // 冷却期检查
            let cooldown_ok = bars_since_sell >= self.config.cooldown_bars;

            // RSI 回升确认：要求最近 N 根 K 线 RSI 递增（过滤"接飞刀"）
            let rsi_rising_ok = if self.config.rsi_rising_confirm_bars == 0 {
                true
            } else {
                let bars = self.config.rsi_rising_confirm_bars;
                if i < bars {
                    false
                } else {
                    let mut rising = true;
                    for k in 0..bars {
                        let cur = rsi_col.get(i - k);
                        let prev = rsi_col.get(i - k - 1);
                        match (cur, prev) {
                            (Some(c), Some(p)) if c > p => continue,
                            _ => {
                                rising = false;
                                break;
                            }
                        }
                    }
                    rising
                }
            };

            // 更新持仓追踪字段
            if shares > 0.0 {
                bars_held += 1;
                if rsi < min_rsi_in_pos {
                    min_rsi_in_pos = rsi;
                }
            }

            // ──── 止损 / 止盈（最高优先级，绕过 min_hold 与其他过滤） ────
            if shares > 0.0 && avg_cost > 0.0 {
                let pnl_pct = (close - avg_cost) / avg_cost;
                let hit_stop = self.config.stop_loss_pct > 0.0
                    && pnl_pct <= -self.config.stop_loss_pct;
                let hit_tp = self.config.take_profit_pct > 0.0
                    && pnl_pct >= self.config.take_profit_pct;
                if hit_stop || hit_tp {
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
                    bars_since_sell = 0;
                    bars_held = 0;
                    min_rsi_in_pos = f64::MAX;
                    signals.push(Signal::Sell);
                    continue;
                }
            }

            // ──── 买入：RSI 超卖 + 趋势 + 过滤器 + 冷却期（空仓首次建仓） ────
            if rsi < self.config.oversold && shares < 1.0 && uptrend && filters_ok && cooldown_ok && rsi_rising_ok {
                // P2: RSI 分档决定仓位比例 — RSI 越低仓位越重
                let position_pct = if rsi < 15.0 {
                    0.70_f64.min(self.config.max_position_pct * 2.0)
                } else if rsi < 20.0 {
                    self.config.max_position_pct * 1.4
                } else {
                    self.config.max_position_pct
                };
                let buy_price = close * (1.0 + self.config.slippage_rate);
                let invest = cash.min(self.config.initial_capital * position_pct);
                let buy_shares = (invest / buy_price).floor();
                if buy_shares > 0.0 {
                    let amount = buy_shares * buy_price;
                    let comm = amount * self.config.commission_rate;
                    cash -= amount + comm;
                    avg_cost = (avg_cost * shares + amount) / (shares + buy_shares);
                    shares += buy_shares;
                    bars_held = 0;
                    min_rsi_in_pos = rsi;
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

            // ──── 分档加仓：持仓中 RSI 继续下探到新低 `add_on_rsi_delta` 以上 ────
            if self.config.add_on_rsi_delta > 0.0
                && self.config.add_on_position_pct > 0.0
                && shares > 0.0
                && rsi < min_rsi_in_pos - self.config.add_on_rsi_delta
                && cash > self.config.initial_capital * 0.05
            {
                let buy_price = close * (1.0 + self.config.slippage_rate);
                let invest = cash.min(self.config.initial_capital * self.config.add_on_position_pct);
                let add_shares = (invest / buy_price).floor();
                if add_shares > 0.0 {
                    let amount = add_shares * buy_price;
                    let comm = amount * self.config.commission_rate;
                    cash -= amount + comm;
                    avg_cost = (avg_cost * shares + amount) / (shares + add_shares);
                    shares += add_shares;
                    min_rsi_in_pos = rsi;
                    trades.push(Trade {
                        date: dt,
                        code: code.to_string(),
                        name: name.to_string(),
                        action: TradeAction::Buy,
                        shares: add_shares,
                        price: buy_price,
                        amount,
                        commission: comm,
                    });
                    signals.push(Signal::Buy);
                    continue;
                }
            }

            // ──── 卖出：超买 / 均衡平仓 / MACD连续转空 / 跌破EMA+SMA ────
            if shares > 0.0 && bars_held >= self.config.min_hold_bars {
                let mut should_sell = rsi > self.config.overbought
                    || (rsi >= self.config.exit_level && avg_cost > 0.0 && close > avg_cost);

                // MACD 柱状线连续 2 根为负 → 动能持续衰竭（过滤单根震荡噪音）
                if self.config.use_macd_filter && !should_sell && i > 1 {
                    if let (Some(hist_now), Some(hist_prev)) =
                        (macd_hist.get(i).copied().flatten(), macd_hist.get(i - 1).copied().flatten())
                    {
                        if hist_now < 0.0 && hist_prev < 0.0 {
                            should_sell = true;
                        }
                    }
                }

                // 价格同时跌破 EMA20 和 SMA20 → 短期趋势破位
                if self.config.use_ema_sma_filter && !should_sell {
                    let below_ema = ema20.get(i).copied().flatten().map_or(false, |v| close < v);
                    let below_sma = sma20.get(i).copied().flatten().map_or(false, |v| close < v);
                    if below_ema && below_sma {
                        should_sell = true;
                    }
                }

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
                    bars_since_sell = 0; // 重置冷却计数
                    bars_held = 0;
                    min_rsi_in_pos = f64::MAX;
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
        "RSI增强策略v2(Wilder+跌势减缓+冷却期+分档加仓)"
    }

    fn description(&self) -> &'static str {
        "Wilder RSI + 跌势减缓过滤 + 冷却期 + RSI分档加仓，1h级别"
    }

    fn run_portfolio_boxed(
        &self,
        stocks: &[(String, String, Vec<KlineData>)],
    ) -> Result<Box<dyn StrategyResult>> {
        let result = self.backtest.run_portfolio(stocks)?;
        Ok(Box::new(result))
    }
}
