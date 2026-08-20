//! 2026-08-20 Attribution Research Loop — 交付物 A 核心模块.
//!
//! 设计: docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md §4.
//! 数据来源: paper_trades (plan_id + virtual_reason), 证据 E3-E7.
//! 归因口径: 已实现 (FIFO 带 lot 归属) + 未实现浮盈 (未平仓 lot × 收盘价).

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 入场信号族 (归因维度). spec §4.1.
/// Ord 派生供 Task 3 的 BTreeMap 聚合排序使用.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SignalFamily {
    NewsCatalyst,
    VolumeSurge,
    MainNetInflow,
    Breakout,
    PostCloseFundInflow,
    ExitByRule,
    Unknown,
}

impl SignalFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            SignalFamily::NewsCatalyst => "NewsCatalyst",
            SignalFamily::VolumeSurge => "VolumeSurge",
            SignalFamily::MainNetInflow => "MainNetInflow",
            SignalFamily::Breakout => "Breakout",
            SignalFamily::PostCloseFundInflow => "PostCloseFundInflow",
            SignalFamily::ExitByRule => "ExitByRule",
            SignalFamily::Unknown => "Unknown",
        }
    }
}

/// virtual_reason → 信号族. 规则表见 spec §4.1; 未命中 → Unknown (报告明示, 不静默).
pub fn signal_family_of(reason: &str) -> SignalFamily {
    let r = reason.trim();
    if r.starts_with("NewsCatalyst") {
        return SignalFamily::NewsCatalyst;
    }
    if r.starts_with("VolumeSurge") {
        return SignalFamily::VolumeSurge;
    }
    if r.starts_with("MainNetInflow") {
        return SignalFamily::MainNetInflow;
    }
    if r.starts_with("Breakout") {
        return SignalFamily::Breakout;
    }
    if r.starts_with("盘后资金净流入") || r.contains("收盘价买入") {
        return SignalFamily::PostCloseFundInflow;
    }
    if r.starts_with("BR-") {
        return SignalFamily::ExitByRule;
    }
    SignalFamily::Unknown
}

/// 提取 `涨幅+X.X%` 数值; 无 → None.
pub fn parse_change_pct(reason: &str) -> Option<f64> {
    let (_, rest) = reason.split_once("涨幅")?;
    let value = rest.split('%').next()?.trim();
    value.parse::<f64>().ok()
}

/// 提取 `量比X.X` 数值; 无 → None.
pub fn parse_volume_ratio(reason: &str) -> Option<f64> {
    let (_, rest) = reason.split_once("量比")?;
    let value: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    value.parse::<f64>().ok()
}

/// 可疑数据: |涨幅| > 25 或 量比 ≤ 0 (spec §4.1; 证据 E6: 涨幅+858.9% ×27、量比0.0).
/// 可疑 lot 仍计入所属族 PnL, 由报告「数据质量」节单独标注 — 不删除, 不静默.
pub fn is_suspicious_reason(reason: &str) -> bool {
    if let Some(pct) = parse_change_pct(reason) {
        if pct.abs() > 25.0 {
            return true;
        }
    }
    if let Some(ratio) = parse_volume_ratio(reason) {
        if ratio <= 0.0 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_from_reason_prefixes() {
        assert_eq!(signal_family_of("NewsCatalyst"), SignalFamily::NewsCatalyst);
        assert_eq!(signal_family_of("VolumeSurge"), SignalFamily::VolumeSurge);
        assert_eq!(signal_family_of("MainNetInflow"), SignalFamily::MainNetInflow);
        assert_eq!(signal_family_of("Breakout"), SignalFamily::Breakout);
        assert_eq!(signal_family_of("BR-234四大铁律卖出:结构止损（破中期趋势）"), SignalFamily::ExitByRule);
        assert_eq!(signal_family_of("盘后资金净流入Top10 收盘价买入: 主力+9.96亿 量比1.5 涨幅-2.9%"), SignalFamily::PostCloseFundInflow);
        assert_eq!(signal_family_of("未知原因"), SignalFamily::Unknown);
    }

    #[test]
    fn suspicious_rules_capture_garbage_but_keep_sane() {
        assert!(is_suspicious_reason("盘后资金净流入Top10 收盘价买入: 主力+25.32亿 量比0.0 涨幅+858.9%"));
        assert!(is_suspicious_reason("... 涨幅+999.0%"));
        assert!(!is_suspicious_reason("... 涨幅+10.0% 量比1.5"));
        assert!(!is_suspicious_reason("NewsCatalyst"));
    }

    #[test]
    fn parse_helpers_extract_structured_fields() {
        let reason = "盘后资金净流入Top10 收盘价买入: 主力+9.96亿 量比1.5 涨幅-2.9%";
        assert_eq!(parse_change_pct(reason), Some(-2.9));
        assert_eq!(parse_volume_ratio(reason), Some(1.5));
        assert_eq!(parse_change_pct("NewsCatalyst"), None);
        assert_eq!(parse_volume_ratio("NewsCatalyst"), None);
    }

    #[test]
    fn family_names_are_stable_snake_case() {
        assert_eq!(SignalFamily::PostCloseFundInflow.as_str(), "PostCloseFundInflow");
        assert_eq!(SignalFamily::ExitByRule.as_str(), "ExitByRule");
    }
}
