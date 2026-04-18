//! 反向择时信号：sentiment_score < 40 且技术面企稳
//!
//! # 背景
//!
//! 历史回测数据（2026-02-07 ~ 2026-04-18，6508条推荐）表明本系统 AI 评分呈现
//! 显著的**反向信号**特征：
//!
//! | 评分段 | T1胜率 | T5胜率 | T5均涨 |
//! |--------|--------|--------|--------|
//! | 80+    | 43.77% | 45.45% | +0.20% |
//! | 70-79  | 43.28% | 45.53% | +0.19% |
//! | <40    | **56.91%** | **55.62%** | **+2.40%** |
//! | 市场基准 | 48.86% | 49.14% | +1.12% |
//!
//! 评分 <40 的超跌股在 T1/T5 胜率与均涨均显著跑赢市场，说明 AI "看空" 的品种
//! 往往恰好处于短期超跌反弹起点。本模块将其形式化为独立的反向择时信号。
//!
//! # 信号触发条件（必须全部满足）
//!
//! 1. **AI 评分 < 40**（已识别为弱势）
//! 2. **超跌**：MA20 乖离率 < -8% 或 52 周位置 < 20%
//! 3. **企稳迹象**：最近 3 日有至少 1 根阳线，且 3 日累计跌幅 > -5%，今日跌幅 > -2%
//! 4. **无加速下跌**：最近 3 日内无跌停
//! 5. **非恐慌抛售**：今日未出现"放量 >2 倍 + 跌幅 >3%"
//!
//! # 风险提示
//!
//! - 仅基于历史统计，未来市场结构变化可能令信号失效
//! - 应与仓位管理（如 5% 单只、8% 硬止损）配合使用
//! - 不适用于基本面恶化（退市、爆雷）标的，此类 AI 评分低但不应抄底

use crate::data_provider::KlineData;

/// 反向择时信号检测结果
#[derive(Debug, Clone)]
pub struct ContrarianSignal {
    /// 是否触发买入信号
    pub triggered: bool,
    /// 触发理由（人类可读）
    pub reason: String,
}

/// 检测反向择时信号
///
/// # 参数
/// - `data`: 日K线数据（按时间倒序，data[0] 为最新）
/// - `sentiment_score`: AI 综合评分 0-100
///
/// # 返回
/// `ContrarianSignal`，其中 `triggered=true` 表示建议反向买入
pub fn detect_contrarian_signal(data: &[KlineData], sentiment_score: i32) -> ContrarianSignal {
    // 基本门槛：评分 < 40 且数据充足
    if sentiment_score >= 40 || data.len() < 20 {
        return ContrarianSignal { triggered: false, reason: String::new() };
    }

    let latest = &data[0];

    // ---- 条件 1：超跌 ----
    let ma20: f64 = data[..20].iter().map(|k| k.close).sum::<f64>() / 20.0;
    let bias_ma20 = if ma20 > 0.0 { (latest.close - ma20) / ma20 * 100.0 } else { 0.0 };

    let week52_len = data.len().min(250);
    let low_52w = data[..week52_len].iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
    let high_52w = data[..week52_len].iter().map(|k| k.high).fold(f64::NEG_INFINITY, f64::max);
    let pos_52w = if (high_52w - low_52w).abs() > 0.001 {
        (latest.close - low_52w) / (high_52w - low_52w) * 100.0
    } else { 50.0 };

    let is_oversold = bias_ma20 < -8.0 || pos_52w < 20.0;
    if !is_oversold {
        return ContrarianSignal { triggered: false, reason: String::new() };
    }

    // ---- 条件 2：最近 3 日企稳 ----
    let recent_len = 3.min(data.len());
    let recent3 = &data[..recent_len];
    let has_green = recent3.iter().any(|k| k.pct_chg > 0.0);
    let recent3_chg: f64 = recent3.iter().map(|k| k.pct_chg).sum();
    let is_stabilizing = (has_green && recent3_chg > -5.0) && latest.pct_chg > -2.0;
    if !is_stabilizing {
        return ContrarianSignal { triggered: false, reason: String::new() };
    }

    // ---- 条件 3：近 3 日无跌停（-9.5% 阈值）----
    let no_limit_down = !recent3.iter().any(|k| k.pct_chg <= -9.5);
    if !no_limit_down {
        return ContrarianSignal { triggered: false, reason: String::new() };
    }

    // ---- 条件 4：非恐慌放量下跌 ----
    let vol_5d_avg = if data.len() >= 5 {
        data[..5].iter().map(|k| k.volume).sum::<f64>() / 5.0
    } else { latest.volume };
    let vol_ratio = if vol_5d_avg > 0.0 { latest.volume / vol_5d_avg } else { 1.0 };
    let no_panic = !(vol_ratio > 2.0 && latest.pct_chg < -3.0);
    if !no_panic {
        return ContrarianSignal { triggered: false, reason: String::new() };
    }

    let reason = format!(
        "🔄反向信号：评分{} | MA20乖离{:+.1}% | 52周位{:.0}% | 近3日累计{:+.2}% | 量比{:.2} | 今日{:+.2}%",
        sentiment_score, bias_ma20, pos_52w, recent3_chg, vol_ratio, latest.pct_chg
    );
    ContrarianSignal { triggered: true, reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn mk(close: f64, pct_chg: f64, volume: f64) -> KlineData {
        KlineData {
            date: NaiveDate::from_ymd_opt(2026, 4, 18).unwrap(),
            open: close, high: close * 1.01, low: close * 0.99, close,
            volume, amount: volume * close, pct_chg,
            pe_ratio: None, pb_ratio: None, turnover_rate: None,
            market_cap: None, circulating_cap: None,
            eps: None, roe: None, revenue_yoy: None, net_profit_yoy: None,
            gross_margin: None, net_margin: None, sharpe_ratio: None,
        }
    }

    #[test]
    fn high_score_does_not_trigger() {
        let data: Vec<KlineData> = (0..30).map(|_| mk(10.0, 0.0, 1e6)).collect();
        let sig = detect_contrarian_signal(&data, 75);
        assert!(!sig.triggered);
    }

    #[test]
    fn oversold_and_stabilizing_triggers() {
        // 最新价远低于 20 日均线，今日企稳
        let mut data: Vec<KlineData> = Vec::new();
        data.push(mk(8.5, 0.5, 1e6)); // latest 企稳
        data.push(mk(8.45, -0.3, 9e5));
        data.push(mk(8.48, 0.8, 1.1e6));
        for _ in 0..30 { data.push(mk(10.0, 0.0, 1e6)); } // 20日均价≈10
        let sig = detect_contrarian_signal(&data, 35);
        assert!(sig.triggered, "should trigger: {}", sig.reason);
    }

    #[test]
    fn panic_volume_blocks() {
        let mut data: Vec<KlineData> = Vec::new();
        data.push(mk(8.0, -4.0, 3e6)); // 今日放量暴跌
        for _ in 0..30 { data.push(mk(10.0, 0.0, 1e6)); }
        let sig = detect_contrarian_signal(&data, 30);
        assert!(!sig.triggered);
    }
}
