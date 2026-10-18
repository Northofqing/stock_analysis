//! 仓位建议（2026-10-18）。
//!
//! ## 不是什么
//!
//! **不是风控闸门。** 用户明确说过"仓位大不代表不能加仓""不代表没有好机会"，
//! 也要求过移除 Frozen 对交易建议的拦截。本模块只输出建议值，不阻断任何交易、
//! 不写任何状态、不设任何熔断。
//!
//! ## 是什么
//!
//! 回答一个具体问题：**这只票该放多少钱。**
//!
//! 用风险预算法（risk-based sizing）：
//!
//! ```text
//! 止损距离 = (买点 - 放弃价) / 买点
//! 建议仓位 = 单笔风险预算 / 止损距离
//! ```
//!
//! ## 三个必须的修正（都是从实际数据里发现的）
//!
//! 1. **止损距离下限 3%**
//!    600396 实测买 10.97 / 放弃 10.85，距离只有 1.1%。按纯公式算，
//!    1.1% 的距离会给出 180% 的仓位 —— 因为"止损很近所以可以重仓"。
//!    但 1.1% 的止损在 A 股是**假止损**：一个正常波动就打掉，止损位毫无意义。
//!    所以距离下限锁到 3%。
//!
//! 2. **止损距离上限 10%**
//!    距离过远说明买点与放弃价脱节（规则没找到有效支撑），此时不应该按
//!    "距离大 → 仓位小"机械缩水，而是直接提示这个票的买点质量存疑。
//!
//! 3. **单票上限 30%**
//!    即使风险预算允许，也不建议单票超过 30%。这不是风控，是常识：
//!    单票 50% 时任何一次误判都伤及本金，影响后续机会的参与能力。
//!
//! ## 不给建议的情况
//!
//! - 超买/异动（`overheated`）—— 追高不是仓位问题，是不该买
//! - 已在 20 日低点附近（`at_support`）—— 支撑未确认，给了买点就是猜
//! - 均线数据不足 —— 没有放弃价算不出止损距离

use crate::monitor::buy_zone::{BuyZone, Trend};

/// 单笔风险预算（占总资产比例）。2% 是行业惯例：连错 10 次也还剩 80% 本金。
const RISK_PER_TRADE: f64 = 0.02;
/// 单票上限。超过这个比例，一次误判就伤及本金。
const MAX_SINGLE_POSITION: f64 = 0.30;
/// 止损距离下限。低于 3% 的止损在 A 股是假止损，正常波动就打掉。
const MIN_STOP_DISTANCE: f64 = 0.03;
/// 止损距离上限。超过 10% 说明买点与放弃价脱节，买点质量存疑。
const MAX_STOP_DISTANCE: f64 = 0.10;

#[derive(Debug, Clone)]
pub struct SizingAdvice {
    pub code: String,
    /// 建议仓位（占总资产），0.0 表示不建议建仓
    pub suggested_pct: f64,
    /// 单票上限（无论建议值多少都不建议超过）
    pub max_pct: f64,
    /// 买点（用于计算止损距离）
    pub entry: Option<f64>,
    /// 放弃价
    pub abandon: Option<f64>,
    /// 实际使用的止损距离（已 clamp）
    pub stop_distance_pct: Option<f64>,
    /// 原始止损距离（未 clamp，用于识别假止损）
    pub raw_stop_distance_pct: Option<f64>,
    /// 建议依据，一句话
    pub basis: String,
}

impl SizingAdvice {
    pub fn render(&self) -> String {
        if self.suggested_pct <= 0.0 {
            return format!("{} 不建议建仓：{}", self.code, self.basis);
        }
        let stop = self
            .stop_distance_pct
            .map(|value| format!("{:.1}%", value * 100.0))
            .unwrap_or_else(|| "—".to_string());
        format!(
            "{} 建议 {:.0}%（上限 {:.0}%），止损距离 {}{}",
            self.code,
            self.suggested_pct * 100.0,
            self.max_pct * 100.0,
            stop,
            if self.basis.is_empty() {
                String::new()
            } else {
                format!("\n     依据：{}", self.basis)
            }
        )
    }
}

/// 根据可买价分析结果给出仓位建议。
pub fn advise(code: &str, zone: &BuyZone) -> SizingAdvice {
    let base = SizingAdvice {
        code: code.to_string(),
        suggested_pct: 0.0,
        max_pct: MAX_SINGLE_POSITION,
        entry: zone.entry_low.zip(zone.entry_high).map(|(low, high)| (low + high) / 2.0),
        abandon: zone.abandon,
        stop_distance_pct: None,
        raw_stop_distance_pct: None,
        basis: String::new(),
    };

    // 超买：追高不是仓位问题
    if zone.overheated {
        return SizingAdvice {
            basis: "超买/异动，不在可建仓状态".to_string(),
            ..base
        };
    }
    // 贴支撑：支撑未确认
    if zone.at_support {
        return SizingAdvice {
            basis: "已在 20 日低点附近，等支撑确认再谈仓位".to_string(),
            ..base
        };
    }
    // 没有买点：均线数据不足或规则未给
    let (Some(entry), Some(abandon)) = (base.entry, base.abandon) else {
        return SizingAdvice {
            basis: "缺少买点或放弃价，无法计算止损距离".to_string(),
            ..base
        };
    };
    if entry <= 0.0 || abandon >= entry {
        return SizingAdvice {
            basis: format!("买点 {entry:.2} 与放弃价 {abandon:.2} 关系异常"),
            ..base
        };
    }

    let raw_stop = (entry - abandon) / entry;

    // 上限 10%：距离过远说明买点质量存疑，不给重仓建议
    if raw_stop > MAX_STOP_DISTANCE {
        return SizingAdvice {
            raw_stop_distance_pct: Some(raw_stop),
            basis: format!(
                "止损距离 {:.1}% 过宽（上限 {:.0}%），买点与放弃价脱节，建议等价格结构清晰",
                raw_stop * 100.0,
                MAX_STOP_DISTANCE * 100.0
            ),
            ..base
        };
    }

    let effective_stop = raw_stop.max(MIN_STOP_DISTANCE);
    let mut size = RISK_PER_TRADE / effective_stop;
    let mut notes: Vec<String> = Vec::new();

    if raw_stop < MIN_STOP_DISTANCE {
        notes.push(format!(
            "实际止损距离仅 {:.1}%（<{:.0}%，按假止损处理，仓位按 {:.0}% 距离计算）",
            raw_stop * 100.0,
            MIN_STOP_DISTANCE * 100.0,
            MIN_STOP_DISTANCE * 100.0
        ));
    }

    if size > MAX_SINGLE_POSITION {
        notes.push(format!(
            "风险预算算出 {:.0}%，受单票上限 {:.0}% 约束",
            size * 100.0,
            MAX_SINGLE_POSITION * 100.0
        ));
        size = MAX_SINGLE_POSITION;
    }

    let trend_label = match zone.trend {
        Trend::Bullish => "多头排列",
        Trend::Bearish => "空头排列",
        Trend::Tangled => "均线纠缠",
        Trend::Unknown => "趋势不明",
    };
    notes.insert(0, trend_label.to_string());

    SizingAdvice {
        suggested_pct: size,
        stop_distance_pct: Some(effective_stop),
        raw_stop_distance_pct: Some(raw_stop),
        basis: notes.join("；"),
        ..base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone_with(entry_low: f64, entry_high: f64, abandon: f64) -> BuyZone {
        BuyZone {
            code: "TEST".to_string(),
            as_of: "2026-10-16".to_string(),
            close: 10.0,
            ma5: Some(10.0),
            ma10: Some(10.0),
            ma20: Some(10.0),
            volume_ratio: Some(1.0),
            low_20d: Some(9.0),
            high_20d: Some(11.0),
            limit_up: None,
            limit_down: None,
            trend: Trend::Bearish,
            entry_low: Some(entry_low),
            entry_high: Some(entry_high),
            abandon: Some(abandon),
            trigger: String::new(),
            at_support: false,
            overheated: false,
        }
    }

    #[test]
    fn tight_stop_is_treated_as_fake_stop() {
        // 600396 实测：买 10.97-11.30 放弃 10.85 → 实际距离约 1.3%
        let advice = advise("600396", &zone_with(10.97, 11.30, 10.85));
        let raw = advice.raw_stop_distance_pct.unwrap();
        assert!(raw < MIN_STOP_DISTANCE, "原始距离应小于 3%，实际 {raw}");
        let effective = advice.stop_distance_pct.unwrap();
        assert!((effective - MIN_STOP_DISTANCE).abs() < 1e-9);
        assert!(advice.suggested_pct <= MAX_SINGLE_POSITION + 1e-9);
        assert!(advice.suggested_pct > 0.0);
    }

    #[test]
    fn wide_stop_yields_no_advice() {
        // 买 10.00 放弃 8.50 → 15%，超过上限
        let advice = advise("TEST", &zone_with(10.0, 10.0, 8.5));
        assert_eq!(advice.suggested_pct, 0.0);
        assert!(advice.basis.contains("过宽"));
    }

    #[test]
    fn normal_stop_sizes_by_risk_budget() {
        // 买 10.00 放弃 9.50 → 5%；2% / 5% = 40%，受 30% 上限约束
        let advice = advise("TEST", &zone_with(10.0, 10.0, 9.5));
        assert!((advice.stop_distance_pct.unwrap() - 0.05).abs() < 1e-9);
        assert!((advice.suggested_pct - MAX_SINGLE_POSITION).abs() < 1e-9);
    }

    #[test]
    fn overheated_gets_zero() {
        let mut zone = zone_with(10.0, 10.5, 9.5);
        zone.overheated = true;
        let advice = advise("TEST", &zone);
        assert_eq!(advice.suggested_pct, 0.0);
        assert!(advice.basis.contains("超买"));
    }
}
