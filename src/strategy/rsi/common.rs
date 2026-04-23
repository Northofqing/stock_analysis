//! RSI 策略共享的指标计算辅助函数
//!
//! 从原 `src/strategy/rsi.rs` 拆出，供 `standard` 与 `precision` 子模块复用。

use chrono::Datelike;
use polars::prelude::*;

use crate::data_provider::KlineData;

// ────────────────────────────── Polars 指标计算 ──────────────────────────────

/// 使用 Wilder 指数平滑计算 RSI（比 SMA 更灵敏精准）
/// 返回与 closes 等长的 Vec，前 period 个点为 50.0（预热期）
pub(super) fn compute_rsi_wilder(closes: &[f64], period: usize) -> Vec<f64> {
    let len = closes.len();
    let mut result = vec![50.0; len];
    if len < 2 || period == 0 {
        return result;
    }

    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;

    // 第一个窗口：简单平均作为种子
    let first_window = period.min(len - 1);
    for i in 1..=first_window {
        let change = closes[i] - closes[i - 1];
        if change > 0.0 {
            avg_gain += change;
        } else {
            avg_loss += change.abs();
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    if avg_gain + avg_loss > 1e-10 {
        result[first_window] = avg_gain / (avg_gain + avg_loss) * 100.0;
    }

    // 后续使用 Wilder 指数平滑
    for i in (first_window + 1)..len {
        let change = closes[i] - closes[i - 1];
        let gain = if change > 0.0 { change } else { 0.0 };
        let loss = if change < 0.0 { change.abs() } else { 0.0 };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
        if avg_gain + avg_loss > 1e-10 {
            result[i] = avg_gain / (avg_gain + avg_loss) * 100.0;
        }
    }
    result
}

/// 计算指定周期的 RSI 列（Polars LazyFrame 版本，用于精准策略的 5 日 RSI）
/// 注意：此函数使用 rolling_mean（SMA 版），仅供 PrecisionRsiBacktest 使用
pub(super) fn build_rsi_lazy(lf: LazyFrame, period: usize, close_col: &str) -> LazyFrame {
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

// ────────────────────────────── 辅助指标计算（Vec 版） ──────────────────────────────

/// 计算指数移动平均线 (EMA)
pub(super) fn compute_ema_vec(close: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = close.len();
    let mut ema: Vec<Option<f64>> = vec![None; n];
    if n < period || period == 0 {
        return ema;
    }
    let initial: f64 = close[..period].iter().sum::<f64>() / period as f64;
    ema[period - 1] = Some(initial);
    let k = 2.0 / (period as f64 + 1.0);
    for i in period..n {
        if let Some(prev) = ema[i - 1] {
            ema[i] = Some(close[i] * k + prev * (1.0 - k));
        }
    }
    ema
}

/// 计算简单移动平均线 (SMA)
pub(super) fn compute_sma_vec(close: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = close.len();
    let mut sma: Vec<Option<f64>> = vec![None; n];
    if n < period || period == 0 {
        return sma;
    }
    let mut sum: f64 = close[..period].iter().sum();
    sma[period - 1] = Some(sum / period as f64);
    for i in period..n {
        sum += close[i] - close[i - period];
        sma[i] = Some(sum / period as f64);
    }
    sma
}

/// 计算 MACD (返回 macd_line, signal_line, histogram)
pub(super) fn compute_macd_vec(
    close: &[f64],
    fast: usize,
    slow: usize,
    signal_period: usize,
) -> (Vec<Option<f64>>, Vec<Option<f64>>, Vec<Option<f64>>) {
    let n = close.len();
    let ema_fast = compute_ema_vec(close, fast);
    let ema_slow = compute_ema_vec(close, slow);

    // MACD line = EMA_fast - EMA_slow
    let mut macd_line: Vec<Option<f64>> = vec![None; n];
    for i in 0..n {
        if let (Some(f), Some(s)) = (ema_fast[i], ema_slow[i]) {
            macd_line[i] = Some(f - s);
        }
    }

    // Signal line = EMA(signal_period) of MACD line (仅对有值的部分计算)
    let macd_values: Vec<f64> = macd_line.iter().filter_map(|&v| v).collect();
    let signal_raw = compute_ema_vec(&macd_values, signal_period);

    // 将信号线映射回原始索引
    let mut signal_line: Vec<Option<f64>> = vec![None; n];
    let mut j = 0;
    for i in 0..n {
        if macd_line[i].is_some() {
            if j < signal_raw.len() {
                signal_line[i] = signal_raw[j];
            }
            j += 1;
        }
    }

    // Histogram = MACD line - Signal line
    let mut histogram: Vec<Option<f64>> = vec![None; n];
    for i in 0..n {
        if let (Some(m), Some(s)) = (macd_line[i], signal_line[i]) {
            histogram[i] = Some(m - s);
        }
    }

    (macd_line, signal_line, histogram)
}

/// 计算月度 VWAP (Volume Weighted Average Price)，每月重置
pub(super) fn compute_vwap_monthly_vec(klines: &[KlineData]) -> Vec<Option<f64>> {
    let n = klines.len();
    let mut vwap: Vec<Option<f64>> = vec![None; n];
    if n == 0 {
        return vwap;
    }

    let mut cum_pv: f64 = 0.0;
    let mut cum_vol: f64 = 0.0;
    let mut current_month = (klines[0].date.year(), klines[0].date.month());

    for i in 0..n {
        let month = (klines[i].date.year(), klines[i].date.month());

        // 月份切换时重置累计值
        if month != current_month {
            cum_pv = 0.0;
            cum_vol = 0.0;
            current_month = month;
        }

        let typical_price = (klines[i].high + klines[i].low + klines[i].close) / 3.0;
        cum_pv += typical_price * klines[i].volume;
        cum_vol += klines[i].volume;

        if cum_vol > 0.0 {
            vwap[i] = Some(cum_pv / cum_vol);
        }
    }

    vwap
}
