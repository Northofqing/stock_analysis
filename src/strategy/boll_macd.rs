//! 布林带 + MACD 共振信号
//!
//! ## 4 条核心规则（用户指定）
//! 1. 布林带收口 + MACD 在 0 轴附近 → 准备变盘（仅提示，不触发买卖）
//! 2. 股价碰下轨 + MACD 底背离 → 买入
//! 3. 股价碰上轨 + MACD 顶背离 → 卖出
//! 4. 布林带张口 + MACD 在 0 轴上方金叉 → 主升浪启动（买入）
//!
//! ## 反误区（关键）
//! - **碰下轨 ≠ 买点**：单边下跌行情中，股价可"撑下轨一路跌"。
//!   必须配合 MACD 底背离，或 MACD 在 0 轴下方且**绿柱缩短**才考虑买入。
//! - **碰上轨 ≠ 卖点**：强势上涨中，股价可"顶上轨一路涨"。
//!   必须配合 MACD 顶背离，或 MACD **红柱缩短**才考虑卖出。

use serde::{Deserialize, Serialize};

use crate::data_provider::KlineData;
use crate::indicators::{calc_macd, detect_divergence, DivergenceType};

/// 布林+MACD 综合信号建议动作
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BollMacdAction {
    /// 不操作
    None,
    /// 准备变盘：布林收口 + MACD 在 0 轴附近（中性，仅提示）
    PreReversal,
    /// 主升浪启动：布林张口 + 0 轴上方金叉（强买）
    UptrendStart,
    /// 底部买入：碰下轨 + MACD 底背离/绿柱缩短
    BottomBuy,
    /// 顶部减仓：碰上轨 + MACD 顶背离/红柱缩短/死叉
    TopSell,
}

impl BollMacdAction {
    pub fn is_buy(self) -> bool {
        matches!(self, Self::UptrendStart | Self::BottomBuy)
    }
    pub fn is_sell(self) -> bool {
        matches!(self, Self::TopSell)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "无信号",
            Self::PreReversal => "准备变盘",
            Self::UptrendStart => "主升浪启动",
            Self::BottomBuy => "下轨抄底",
            Self::TopSell => "上轨减仓",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BollMacdSignal {
    pub action: BollMacdAction,
    pub reason: String,
    pub close: f64,
    pub upper: f64,
    pub middle: f64,
    pub lower: f64,
    /// 当前带宽 (UP-DN) / MID × 100
    pub band_width_pct: f64,
    /// 5 日前到今日的带宽变化率（>0 张口，<0 收口）
    pub band_change_pct: f64,
    pub macd_dif: f64,
    pub macd_dea: f64,
    pub macd_hist: f64,
    pub macd_div: DivergenceType,
}

impl BollMacdSignal {
    fn empty() -> Self {
        Self {
            action: BollMacdAction::None,
            reason: String::new(),
            close: 0.0,
            upper: 0.0,
            middle: 0.0,
            lower: 0.0,
            band_width_pct: 0.0,
            band_change_pct: 0.0,
            macd_dif: 0.0,
            macd_dea: 0.0,
            macd_hist: 0.0,
            macd_div: DivergenceType::None,
        }
    }
}

/// 检测布林+MACD 共振信号
///
/// `data` 倒序：`data[0]` 最新。
pub fn detect_boll_macd_signal(data: &[KlineData]) -> BollMacdSignal {
    if data.len() < 35 {
        return BollMacdSignal::empty();
    }

    // 转为时间正序（最旧 → 最新）
    let closes: Vec<f64> = data.iter().rev().map(|k| k.close).collect();
    let n = closes.len();

    // ============ 布林带 (20, 2σ) ============
    const PERIOD: usize = 20;
    const STDEV_MUL: f64 = 2.0;

    let recent20 = &closes[n - PERIOD..];
    let mid: f64 = recent20.iter().sum::<f64>() / PERIOD as f64;
    let var: f64 = recent20.iter().map(|c| (c - mid).powi(2)).sum::<f64>() / PERIOD as f64;
    let stdev = var.sqrt();
    let upper = mid + STDEV_MUL * stdev;
    let lower = mid - STDEV_MUL * stdev;
    let band_width = upper - lower;
    let band_width_pct = if mid > 0.0 { band_width / mid * 100.0 } else { 0.0 };

    // 5 日前的带宽（用来判断张口/收口趋势）
    let band_change_pct = if n >= PERIOD + 5 {
        let past = &closes[n - PERIOD - 5..n - 5];
        let past_mid = past.iter().sum::<f64>() / PERIOD as f64;
        let past_var = past.iter().map(|c| (c - past_mid).powi(2)).sum::<f64>() / PERIOD as f64;
        let past_band = 2.0 * STDEV_MUL * past_var.sqrt();
        if past_band > 0.0 {
            (band_width / past_band - 1.0) * 100.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    // ============ MACD (12, 26, 9) ============
    let macd = calc_macd(&closes, 12, 26, 9);
    if macd.len() < 3 {
        return BollMacdSignal::empty();
    }
    let m = macd.last().unwrap();
    let m_prev = &macd[macd.len() - 2];
    let m_pp = &macd[macd.len() - 3];

    // MACD 背离（30 日窗口，DIF 为指标）
    let dif_series: Vec<f64> = macd.iter().map(|p| p.dif).collect();
    let div = detect_divergence(&closes, &dif_series, 30, "MACD").divergence;

    let close = closes[n - 1];

    // 0 轴附近：|DIF| / 价 < 0.5%
    let zero_axis_near = (m.dif.abs() / close.max(0.001)) < 0.005;
    // 0 轴上方
    let above_zero = m.dif > 0.0 && m.dea > 0.0;
    // 金叉/死叉
    let golden_cross = m_prev.dif < m_prev.dea && m.dif > m.dea;
    let death_cross = m_prev.dif > m_prev.dea && m.dif < m.dea;

    // 布林收口/张口（5 日带宽变化）
    let squeezing = band_change_pct < -10.0;
    let expanding = band_change_pct > 15.0;

    // 触轨（容忍 1%）
    let touch_lower = close <= lower * 1.01;
    let touch_upper = close >= upper * 0.99;

    // 红柱缩短（连续 2 根减小，hist > 0）→ 顶部动能衰竭
    let hist_shrinking_red = m_pp.histogram > m_prev.histogram
        && m_prev.histogram > m.histogram
        && m.histogram > 0.0;
    // 绿柱缩短（连续 2 根负值绝对值减小，即 hist < 0 但越来越接近 0）→ 底部动能衰竭
    let hist_shrinking_green = m_pp.histogram < m_prev.histogram
        && m_prev.histogram < m.histogram
        && m.histogram < 0.0;

    // ============ 应用规则（优先级：强买 > 抄底 > 卖出 > 变盘提示）============

    // 规则 4：主升浪启动（最强买点）
    if expanding && above_zero && golden_cross {
        return BollMacdSignal {
            action: BollMacdAction::UptrendStart,
            reason: format!(
                "主升浪：布林张口({:+.1}%) + MACD 0 轴上方金叉 (DIF={:.3} DEA={:.3})",
                band_change_pct, m.dif, m.dea
            ),
            close, upper, middle: mid, lower, band_width_pct, band_change_pct,
            macd_dif: m.dif, macd_dea: m.dea, macd_hist: m.histogram, macd_div: div,
        };
    }

    // 规则 2：下轨抄底（必须配合 MACD 底背离 或 0 轴下方绿柱缩短）
    if touch_lower
        && (div == DivergenceType::BullishBottom || (m.dif < 0.0 && hist_shrinking_green))
    {
        let reason = if div == DivergenceType::BullishBottom {
            format!("下轨抄底：触下轨 ({:.2} ≤ {:.2}) + MACD 底背离", close, lower)
        } else {
            format!(
                "下轨抄底：触下轨 ({:.2} ≤ {:.2}) + MACD 绿柱缩短 (hist {:+.3})",
                close, lower, m.histogram
            )
        };
        return BollMacdSignal {
            action: BollMacdAction::BottomBuy,
            reason,
            close, upper, middle: mid, lower, band_width_pct, band_change_pct,
            macd_dif: m.dif, macd_dea: m.dea, macd_hist: m.histogram, macd_div: div,
        };
    }

    // 规则 3：上轨减仓（必须配合 顶背离 / 红柱缩短 / 死叉）
    if touch_upper
        && (div == DivergenceType::BearishTop || hist_shrinking_red || death_cross)
    {
        let reason = if div == DivergenceType::BearishTop {
            format!("上轨减仓：触上轨 ({:.2} ≥ {:.2}) + MACD 顶背离", close, upper)
        } else if death_cross {
            format!(
                "上轨减仓：触上轨 + MACD 死叉 (DIF {:.3} < DEA {:.3})",
                m.dif, m.dea
            )
        } else {
            format!(
                "上轨减仓：触上轨 ({:.2} ≥ {:.2}) + MACD 红柱缩短 (hist {:+.3})",
                close, upper, m.histogram
            )
        };
        return BollMacdSignal {
            action: BollMacdAction::TopSell,
            reason,
            close, upper, middle: mid, lower, band_width_pct, band_change_pct,
            macd_dif: m.dif, macd_dea: m.dea, macd_hist: m.histogram, macd_div: div,
        };
    }

    // 规则 1：变盘前夕（仅提示）
    if squeezing && zero_axis_near {
        return BollMacdSignal {
            action: BollMacdAction::PreReversal,
            reason: format!(
                "准备变盘：布林收口({:+.1}%) + MACD 0 轴附近 (DIF={:.3})",
                band_change_pct, m.dif
            ),
            close, upper, middle: mid, lower, band_width_pct, band_change_pct,
            macd_dif: m.dif, macd_dea: m.dea, macd_hist: m.histogram, macd_div: div,
        };
    }

    BollMacdSignal {
        action: BollMacdAction::None,
        reason: String::new(),
        close, upper, middle: mid, lower, band_width_pct, band_change_pct,
        macd_dif: m.dif, macd_dea: m.dea, macd_hist: m.histogram, macd_div: div,
    }
}
