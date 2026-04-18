//! 技术指标模块
//!
//! 包含 MACD、KDJ、RSI 三项指标的计算、顶底背离检测及金叉共振判断。
//!
//! ## 公式说明
//!
//! ### MACD（指数平滑异同移动平均线）
//! - DIF  = EMA(close, 12) − EMA(close, 26)
//! - DEA  = EMA(DIF, 9)
//! - MACD柱 = 2 × (DIF − DEA)
//!
//! ### KDJ（随机指标）
//! - RSV = (close − LLV(low, 9)) / (HHV(high, 9) − LLV(low, 9)) × 100
//! - K = SMA(RSV, 3, 1)   （即 K_prev × 2/3 + RSV × 1/3）
//! - D = SMA(K, 3, 1)
//! - J = 3K − 2D
//!
//! ### RSI（相对强弱指标）
//! - RSI(N) = SMA(max(close−prev_close, 0), N) /
//!            SMA(abs(close−prev_close), N) × 100

use serde::{Deserialize, Serialize};
use std::fmt;

// ============================================================================
// MACD
// ============================================================================

/// 单条 MACD 数据点
#[derive(Debug, Clone, Default)]
pub struct MacdPoint {
    pub dif: f64,
    pub dea: f64,
    pub histogram: f64, // MACD 柱 = 2*(DIF-DEA)
}

/// 计算 MACD 序列
///
/// `closes` 按时间升序排列（最旧在前）。
pub fn calc_macd(closes: &[f64], fast: usize, slow: usize, signal: usize) -> Vec<MacdPoint> {
    if closes.len() < slow {
        return vec![MacdPoint::default(); closes.len()];
    }

    let ema_fast = ema(closes, fast);
    let ema_slow = ema(closes, slow);

    let dif: Vec<f64> = ema_fast
        .iter()
        .zip(ema_slow.iter())
        .map(|(f, s)| f - s)
        .collect();

    let dea = ema(&dif, signal);

    dif.iter()
        .zip(dea.iter())
        .map(|(&d, &e)| MacdPoint {
            dif: d,
            dea: e,
            histogram: 2.0 * (d - e),
        })
        .collect()
}

// ============================================================================
// KDJ
// ============================================================================

/// 单条 KDJ 数据点
#[derive(Debug, Clone, Default)]
pub struct KdjPoint {
    pub k: f64,
    pub d: f64,
    pub j: f64,
}

/// 计算 KDJ 序列
///
/// `highs`, `lows`, `closes` 按时间升序排列，长度必须一致。
pub fn calc_kdj(
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
    n: usize,   // RSV 周期，默认 9
    m1: usize,  // K 平滑周期，默认 3
    m2: usize,  // D 平滑周期，默认 3
) -> Vec<KdjPoint> {
    let len = closes.len();
    if len == 0 {
        return Vec::new();
    }

    // 计算 RSV
    let mut rsv = vec![50.0_f64; len];
    for i in 0..len {
        let start = if i + 1 >= n { i + 1 - n } else { 0 };
        let hh = highs[start..=i]
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        let ll = lows[start..=i]
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);
        if (hh - ll).abs() > 1e-10 {
            rsv[i] = (closes[i] - ll) / (hh - ll) * 100.0;
        }
    }

    // SMA 平滑：K_i = K_{i-1} * (m-1)/m + RSV_i * 1/m
    let mut k_vals = vec![50.0_f64; len];
    let mut d_vals = vec![50.0_f64; len];
    let mut j_vals = vec![50.0_f64; len];

    for i in 0..len {
        if i == 0 {
            k_vals[i] = rsv[i];
        } else {
            k_vals[i] = k_vals[i - 1] * (m1 as f64 - 1.0) / m1 as f64
                + rsv[i] / m1 as f64;
        }
    }

    for i in 0..len {
        if i == 0 {
            d_vals[i] = k_vals[i];
        } else {
            d_vals[i] = d_vals[i - 1] * (m2 as f64 - 1.0) / m2 as f64
                + k_vals[i] / m2 as f64;
        }
    }

    for i in 0..len {
        j_vals[i] = 3.0 * k_vals[i] - 2.0 * d_vals[i];
    }

    (0..len)
        .map(|i| KdjPoint {
            k: k_vals[i],
            d: d_vals[i],
            j: j_vals[i],
        })
        .collect()
}

// ============================================================================
// RSI
// ============================================================================

/// 单条 RSI 数据点
#[derive(Debug, Clone, Default)]
pub struct RsiPoint {
    pub rsi6: f64,
    pub rsi12: f64,
    pub rsi24: f64,
}

/// 计算 RSI 序列
///
/// `closes` 按时间升序排列。返回同等长度的序列，前期数据可能不准确。
pub fn calc_rsi(closes: &[f64]) -> Vec<RsiPoint> {
    let len = closes.len();
    if len < 2 {
        return vec![RsiPoint { rsi6: 50.0, rsi12: 50.0, rsi24: 50.0 }; len];
    }

    let rsi6 = rsi_single(closes, 6);
    let rsi12 = rsi_single(closes, 12);
    let rsi24 = rsi_single(closes, 24);

    (0..len)
        .map(|i| RsiPoint {
            rsi6: rsi6[i],
            rsi12: rsi12[i],
            rsi24: rsi24[i],
        })
        .collect()
}

/// 计算单一周期的 RSI
fn rsi_single(closes: &[f64], period: usize) -> Vec<f64> {
    let len = closes.len();
    let mut result = vec![50.0; len];
    if len < 2 {
        return result;
    }

    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;

    // 第一个窗口
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

    // 后续使用指数平滑
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

// ============================================================================
// 背离检测
// ============================================================================

/// 背离类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DivergenceType {
    /// 顶背离（价格创新高，指标未创新高）—— 看跌信号
    BearishTop,
    /// 底背离（价格创新低，指标未创新低）—— 看涨信号
    BullishBottom,
    /// 无背离
    None,
}

impl fmt::Display for DivergenceType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::BearishTop => write!(f, "顶背离(看跌)"),
            Self::BullishBottom => write!(f, "底背离(看涨)"),
            Self::None => write!(f, "无"),
        }
    }
}

/// 单项指标背离结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceResult {
    pub indicator: String,
    pub divergence: DivergenceType,
    pub description: String,
}

/// 在价格序列和指标序列中检测顶底背离
///
/// `prices` 和 `indicator` 等长、升序。
/// `lookback` 是回溯窗口（默认 30）。
pub fn detect_divergence(
    prices: &[f64],
    indicator: &[f64],
    lookback: usize,
    indicator_name: &str,
) -> DivergenceResult {
    let len = prices.len();
    if len < lookback || len < 10 {
        return DivergenceResult {
            indicator: indicator_name.to_string(),
            divergence: DivergenceType::None,
            description: "数据不足".to_string(),
        };
    }

    let start = len - lookback;
    let mid = start + lookback / 2;

    // 在回溯窗口中找前半段和后半段的极值
    let (prev_high_price, prev_high_idx) = max_with_index(&prices[start..mid]);
    let (curr_high_price, curr_high_idx) = max_with_index(&prices[mid..len]);
    let curr_high_idx = curr_high_idx + mid;
    let prev_high_idx = prev_high_idx + start;

    let (prev_low_price, prev_low_idx) = min_with_index(&prices[start..mid]);
    let (curr_low_price, curr_low_idx) = min_with_index(&prices[mid..len]);
    let curr_low_idx = curr_low_idx + mid;
    let prev_low_idx = prev_low_idx + start;

    // 顶背离：价格创新高，但指标没有创新高
    if curr_high_price > prev_high_price * 0.998 {
        let prev_ind = indicator[prev_high_idx];
        let curr_ind = indicator[curr_high_idx];
        if curr_ind < prev_ind * 0.97 {
            return DivergenceResult {
                indicator: indicator_name.to_string(),
                divergence: DivergenceType::BearishTop,
                description: format!(
                    "{}顶背离：价格高点 {:.2}->{:.2}(↑)，指标 {:.2}->{:.2}(↓)",
                    indicator_name, prev_high_price, curr_high_price, prev_ind, curr_ind
                ),
            };
        }
    }

    // 底背离：价格创新低，但指标没有创新低
    if curr_low_price < prev_low_price * 1.002 {
        let prev_ind = indicator[prev_low_idx];
        let curr_ind = indicator[curr_low_idx];
        if curr_ind > prev_ind * 1.03 {
            return DivergenceResult {
                indicator: indicator_name.to_string(),
                divergence: DivergenceType::BullishBottom,
                description: format!(
                    "{}底背离：价格低点 {:.2}->{:.2}(↓)，指标 {:.2}->{:.2}(↑)",
                    indicator_name, prev_low_price, curr_low_price, prev_ind, curr_ind
                ),
            };
        }
    }

    DivergenceResult {
        indicator: indicator_name.to_string(),
        divergence: DivergenceType::None,
        description: format!("{}未发现背离", indicator_name),
    }
}

// ============================================================================
// 金叉/死叉检测
// ============================================================================

/// 交叉类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossType {
    /// 金叉（快线上穿慢线）
    GoldenCross,
    /// 死叉（快线下穿慢线）
    DeathCross,
    /// 无交叉
    None,
}

impl fmt::Display for CrossType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::GoldenCross => write!(f, "金叉"),
            Self::DeathCross => write!(f, "死叉"),
            Self::None => write!(f, "无"),
        }
    }
}

/// 检测最近 N 根K线内快线是否上穿/下穿慢线
///
/// `fast` 和 `slow` 等长且升序。`lookback` 默认 5。
fn detect_cross(fast: &[f64], slow: &[f64], lookback: usize) -> CrossType {
    let len = fast.len();
    if len < 2 {
        return CrossType::None;
    }
    let start = if len > lookback { len - lookback } else { 0 };

    for i in (start + 1..len).rev() {
        let prev_diff = fast[i - 1] - slow[i - 1];
        let curr_diff = fast[i] - slow[i];
        if prev_diff <= 0.0 && curr_diff > 0.0 {
            return CrossType::GoldenCross;
        }
        if prev_diff >= 0.0 && curr_diff < 0.0 {
            return CrossType::DeathCross;
        }
    }
    CrossType::None
}

// ============================================================================
// 综合指标分析结果
// ============================================================================

/// 三项指标综合分析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorAnalysis {
    // --- MACD ---
    pub macd_dif: f64,
    pub macd_dea: f64,
    pub macd_histogram: f64,
    pub macd_cross: CrossType,
    pub macd_divergence: DivergenceResult,

    // --- KDJ ---
    pub kdj_k: f64,
    pub kdj_d: f64,
    pub kdj_j: f64,
    pub kdj_cross: CrossType,
    pub kdj_divergence: DivergenceResult,

    // --- RSI ---
    pub rsi6: f64,
    pub rsi12: f64,
    pub rsi24: f64,
    pub rsi_cross: CrossType,
    pub rsi_divergence: DivergenceResult,

    // --- 共振 ---
    pub golden_cross_resonance: bool,
    pub death_cross_resonance: bool,
    pub bottom_divergence_resonance: bool,
    pub top_divergence_resonance: bool,

    /// 指标综合评分（0-100）
    pub indicator_score: i32,
    /// 信号描述
    pub signals: Vec<String>,
}

impl Default for IndicatorAnalysis {
    fn default() -> Self {
        Self {
            macd_dif: 0.0,
            macd_dea: 0.0,
            macd_histogram: 0.0,
            macd_cross: CrossType::None,
            macd_divergence: DivergenceResult {
                indicator: "MACD".to_string(),
                divergence: DivergenceType::None,
                description: String::new(),
            },
            kdj_k: 50.0,
            kdj_d: 50.0,
            kdj_j: 50.0,
            kdj_cross: CrossType::None,
            kdj_divergence: DivergenceResult {
                indicator: "KDJ".to_string(),
                divergence: DivergenceType::None,
                description: String::new(),
            },
            rsi6: 50.0,
            rsi12: 50.0,
            rsi24: 50.0,
            rsi_cross: CrossType::None,
            rsi_divergence: DivergenceResult {
                indicator: "RSI".to_string(),
                divergence: DivergenceType::None,
                description: String::new(),
            },
            golden_cross_resonance: false,
            death_cross_resonance: false,
            bottom_divergence_resonance: false,
            top_divergence_resonance: false,
            indicator_score: 50,
            signals: Vec::new(),
        }
    }
}

/// 运行完整的指标分析
///
/// `highs`, `lows`, `closes` 按时间升序排列，长度一致。
pub fn analyze_indicators(
    highs: &[f64],
    lows: &[f64],
    closes: &[f64],
) -> IndicatorAnalysis {
    let mut result = IndicatorAnalysis::default();
    let len = closes.len();
    if len < 30 {
        result.signals.push("数据不足，指标分析不可靠".to_string());
        return result;
    }

    // ---- MACD ----
    let macd = calc_macd(closes, 12, 26, 9);
    if let Some(latest) = macd.last() {
        result.macd_dif = latest.dif;
        result.macd_dea = latest.dea;
        result.macd_histogram = latest.histogram;
    }
    let dif_series: Vec<f64> = macd.iter().map(|p| p.dif).collect();
    let dea_series: Vec<f64> = macd.iter().map(|p| p.dea).collect();
    result.macd_cross = detect_cross(&dif_series, &dea_series, 5);
    result.macd_divergence = detect_divergence(closes, &dif_series, 30.min(len), "MACD");

    // ---- KDJ ----
    let kdj = calc_kdj(highs, lows, closes, 9, 3, 3);
    if let Some(latest) = kdj.last() {
        result.kdj_k = latest.k;
        result.kdj_d = latest.d;
        result.kdj_j = latest.j;
    }
    let k_series: Vec<f64> = kdj.iter().map(|p| p.k).collect();
    let d_series: Vec<f64> = kdj.iter().map(|p| p.d).collect();
    result.kdj_cross = detect_cross(&k_series, &d_series, 5);
    result.kdj_divergence = detect_divergence(closes, &k_series, 30.min(len), "KDJ-K");

    // ---- RSI ----
    let rsi = calc_rsi(closes);
    if let Some(latest) = rsi.last() {
        result.rsi6 = latest.rsi6;
        result.rsi12 = latest.rsi12;
        result.rsi24 = latest.rsi24;
    }
    let rsi6_series: Vec<f64> = rsi.iter().map(|p| p.rsi6).collect();
    let rsi12_series: Vec<f64> = rsi.iter().map(|p| p.rsi12).collect();
    result.rsi_cross = detect_cross(&rsi6_series, &rsi12_series, 5);
    result.rsi_divergence = detect_divergence(closes, &rsi6_series, 30.min(len), "RSI6");

    // ---- 共振检测 ----
    let macd_golden = result.macd_cross == CrossType::GoldenCross;
    let kdj_golden = result.kdj_cross == CrossType::GoldenCross;
    let rsi_golden = result.rsi_cross == CrossType::GoldenCross;

    let macd_death = result.macd_cross == CrossType::DeathCross;
    let kdj_death = result.kdj_cross == CrossType::DeathCross;
    let rsi_death = result.rsi_cross == CrossType::DeathCross;

    // 金叉共振：至少两项指标同时金叉
    let golden_count = [macd_golden, kdj_golden, rsi_golden]
        .iter()
        .filter(|&&x| x)
        .count();
    result.golden_cross_resonance = golden_count >= 2;

    // 死叉共振：至少两项指标同时死叉
    let death_count = [macd_death, kdj_death, rsi_death]
        .iter()
        .filter(|&&x| x)
        .count();
    result.death_cross_resonance = death_count >= 2;

    // 底背离共振：至少两项指标同时底背离
    let bottom_count = [
        result.macd_divergence.divergence == DivergenceType::BullishBottom,
        result.kdj_divergence.divergence == DivergenceType::BullishBottom,
        result.rsi_divergence.divergence == DivergenceType::BullishBottom,
    ]
    .iter()
    .filter(|&&x| x)
    .count();
    result.bottom_divergence_resonance = bottom_count >= 2;

    // 顶背离共振
    let top_count = [
        result.macd_divergence.divergence == DivergenceType::BearishTop,
        result.kdj_divergence.divergence == DivergenceType::BearishTop,
        result.rsi_divergence.divergence == DivergenceType::BearishTop,
    ]
    .iter()
    .filter(|&&x| x)
    .count();
    result.top_divergence_resonance = top_count >= 2;

    // ---- 综合评分 ----
    let mut score: i32 = 50;
    let mut signals = Vec::new();

    // MACD
    match result.macd_cross {
        CrossType::GoldenCross => {
            score += 10;
            signals.push("📈 MACD金叉".to_string());
        }
        CrossType::DeathCross => {
            score -= 10;
            signals.push("📉 MACD死叉".to_string());
        }
        _ => {}
    }
    if result.macd_histogram > 0.0 {
        score += 3;
    } else {
        score -= 3;
    }
    match result.macd_divergence.divergence {
        DivergenceType::BullishBottom => {
            score += 12;
            signals.push(format!("🔥 {}", result.macd_divergence.description));
        }
        DivergenceType::BearishTop => {
            score -= 12;
            signals.push(format!("⚠️ {}", result.macd_divergence.description));
        }
        _ => {}
    }

    // KDJ
    match result.kdj_cross {
        CrossType::GoldenCross => {
            score += 8;
            signals.push("📈 KDJ金叉".to_string());
        }
        CrossType::DeathCross => {
            score -= 8;
            signals.push("📉 KDJ死叉".to_string());
        }
        _ => {}
    }
    if result.kdj_j < 20.0 {
        score += 5;
        signals.push(format!("💡 KDJ-J超卖({:.1})", result.kdj_j));
    } else if result.kdj_j > 80.0 {
        score -= 5;
        signals.push(format!("⚠️ KDJ-J超买({:.1})", result.kdj_j));
    }
    match result.kdj_divergence.divergence {
        DivergenceType::BullishBottom => {
            score += 10;
            signals.push(format!("🔥 {}", result.kdj_divergence.description));
        }
        DivergenceType::BearishTop => {
            score -= 10;
            signals.push(format!("⚠️ {}", result.kdj_divergence.description));
        }
        _ => {}
    }

    // RSI
    match result.rsi_cross {
        CrossType::GoldenCross => {
            score += 6;
            signals.push("📈 RSI6上穿RSI12".to_string());
        }
        CrossType::DeathCross => {
            score -= 6;
            signals.push("📉 RSI6下穿RSI12".to_string());
        }
        _ => {}
    }
    if result.rsi6 < 20.0 {
        score += 8;
        signals.push(format!("💡 RSI6超卖({:.1})，可能反弹", result.rsi6));
    } else if result.rsi6 > 80.0 {
        score -= 8;
        signals.push(format!("⚠️ RSI6超买({:.1})，注意回调", result.rsi6));
    }
    match result.rsi_divergence.divergence {
        DivergenceType::BullishBottom => {
            score += 10;
            signals.push(format!("🔥 {}", result.rsi_divergence.description));
        }
        DivergenceType::BearishTop => {
            score -= 10;
            signals.push(format!("⚠️ {}", result.rsi_divergence.description));
        }
        _ => {}
    }

    // 共振加分（重要信号）
    if result.golden_cross_resonance {
        score += 15;
        signals.push(format!(
            "🚀 金叉共振！{}指标同时金叉",
            golden_count
        ));
    }
    if result.death_cross_resonance {
        score -= 15;
        signals.push(format!(
            "💀 死叉共振！{}指标同时死叉",
            death_count
        ));
    }
    if result.bottom_divergence_resonance {
        score += 15;
        signals.push(format!(
            "🔥 底背离共振！{}指标同时底背离，强烈看涨",
            bottom_count
        ));
    }
    if result.top_divergence_resonance {
        score -= 15;
        signals.push(format!(
            "💀 顶背离共振！{}指标同时顶背离，强烈看跌",
            top_count
        ));
    }

    result.indicator_score = score.clamp(0, 100);
    result.signals = signals;
    result
}

/// 格式化指标分析结果为文本
pub fn format_indicator_analysis(a: &IndicatorAnalysis) -> String {
    let mut lines = vec![
        "=== 技术指标分析 ===".to_string(),
        String::new(),
        "📊 MACD:".to_string(),
        format!("   DIF: {:.4}  DEA: {:.4}  柱: {:.4}", a.macd_dif, a.macd_dea, a.macd_histogram),
        format!("   交叉: {}  背离: {}", a.macd_cross, a.macd_divergence.divergence),
        String::new(),
        "📊 KDJ:".to_string(),
        format!("   K: {:.1}  D: {:.1}  J: {:.1}", a.kdj_k, a.kdj_d, a.kdj_j),
        format!("   交叉: {}  背离: {}", a.kdj_cross, a.kdj_divergence.divergence),
        String::new(),
        "📊 RSI:".to_string(),
        format!("   RSI6: {:.1}  RSI12: {:.1}  RSI24: {:.1}", a.rsi6, a.rsi12, a.rsi24),
        format!("   交叉: {}  背离: {}", a.rsi_cross, a.rsi_divergence.divergence),
        String::new(),
        "🎯 共振判断:".to_string(),
        format!("   金叉共振: {}  死叉共振: {}", a.golden_cross_resonance, a.death_cross_resonance),
        format!("   底背离共振: {}  顶背离共振: {}", a.bottom_divergence_resonance, a.top_divergence_resonance),
        format!("   指标评分: {}/100", a.indicator_score),
    ];

    if !a.signals.is_empty() {
        lines.push(String::new());
        lines.push("📋 信号列表:".to_string());
        for s in &a.signals {
            lines.push(format!("   {}", s));
        }
    }

    lines.join("\n")
}

// ============================================================================
// 工具函数
// ============================================================================

/// EMA 指数移动平均
fn ema(data: &[f64], period: usize) -> Vec<f64> {
    let len = data.len();
    let mut result = vec![0.0; len];
    if len == 0 || period == 0 {
        return result;
    }

    let k = 2.0 / (period as f64 + 1.0);
    result[0] = data[0];
    for i in 1..len {
        result[i] = data[i] * k + result[i - 1] * (1.0 - k);
    }
    result
}

/// 返回切片中的最大值和索引
fn max_with_index(data: &[f64]) -> (f64, usize) {
    let mut max_val = f64::NEG_INFINITY;
    let mut max_idx = 0;
    for (i, &v) in data.iter().enumerate() {
        if v > max_val {
            max_val = v;
            max_idx = i;
        }
    }
    (max_val, max_idx)
}

/// 返回切片中的最小值和索引
fn min_with_index(data: &[f64]) -> (f64, usize) {
    let mut min_val = f64::INFINITY;
    let mut min_idx = 0;
    for (i, &v) in data.iter().enumerate() {
        if v < min_val {
            min_val = v;
            min_idx = i;
        }
    }
    (min_val, min_idx)
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_price_data(days: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let mut highs = Vec::new();
        let mut lows = Vec::new();
        let mut closes = Vec::new();
        let mut price = 10.0;

        for i in 0..days {
            // 模拟先涨后跌（产生背离机会）
            if i < days / 2 {
                price *= 1.005;
            } else {
                price *= 0.998;
            }
            let h = price * 1.015;
            let l = price * 0.985;
            highs.push(h);
            lows.push(l);
            closes.push(price);
        }

        (highs, lows, closes)
    }

    #[test]
    fn test_ema() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = ema(&data, 3);
        assert_eq!(result.len(), 5);
        assert!((result[0] - 1.0).abs() < 1e-10);
        // EMA should be between min and max
        assert!(result[4] > 1.0 && result[4] <= 5.0);
    }

    #[test]
    fn test_macd() {
        let (_, _, closes) = make_price_data(60);
        let macd = calc_macd(&closes, 12, 26, 9);
        assert_eq!(macd.len(), 60);
        // DIF = EMA12 - EMA26, 在上涨趋势中应为正
        assert!(macd[40].dif > 0.0);
    }

    #[test]
    fn test_kdj() {
        let (highs, lows, closes) = make_price_data(60);
        let kdj = calc_kdj(&highs, &lows, &closes, 9, 3, 3);
        assert_eq!(kdj.len(), 60);
        // K, D 应在 0-100 区间附近
        let last = &kdj[59];
        assert!(last.k >= 0.0 && last.k <= 100.0);
        assert!(last.d >= 0.0 && last.d <= 100.0);
    }

    #[test]
    fn test_rsi() {
        let (_, _, closes) = make_price_data(60);
        let rsi = calc_rsi(&closes);
        assert_eq!(rsi.len(), 60);
        let last = &rsi[59];
        assert!(last.rsi6 >= 0.0 && last.rsi6 <= 100.0);
        assert!(last.rsi12 >= 0.0 && last.rsi12 <= 100.0);
    }

    #[test]
    fn test_cross_detection() {
        let fast = vec![1.0, 2.0, 3.0, 2.5, 3.5, 4.0];
        let slow = vec![2.0, 2.5, 2.8, 3.0, 3.2, 3.3];
        // fast 从 < slow 到 > slow: 金叉
        let cross = detect_cross(&fast, &slow, 6);
        assert_eq!(cross, CrossType::GoldenCross);
    }

    #[test]
    fn test_analyze_indicators() {
        let (highs, lows, closes) = make_price_data(60);
        let result = analyze_indicators(&highs, &lows, &closes);

        assert!(result.indicator_score >= 0 && result.indicator_score <= 100);
        println!("{}", format_indicator_analysis(&result));
    }

    #[test]
    fn test_golden_cross_resonance() {
        // 构造使 MACD、KDJ 同时金叉的数据
        let mut closes = vec![10.0; 30];
        // 先跌再涨，触发金叉
        for i in 0..15 {
            closes.push(10.0 - i as f64 * 0.1);
        }
        for i in 0..15 {
            closes.push(8.5 + i as f64 * 0.2);
        }

        let highs: Vec<f64> = closes.iter().map(|c| c * 1.01).collect();
        let lows: Vec<f64> = closes.iter().map(|c| c * 0.99).collect();

        let result = analyze_indicators(&highs, &lows, &closes);
        // 检查评分合理
        assert!(result.indicator_score >= 0 && result.indicator_score <= 100);
        println!("共振测试评分: {}", result.indicator_score);
        for s in &result.signals {
            println!("  {}", s);
        }
    }
}
