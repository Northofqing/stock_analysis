//! cross（从 indicators.rs 拆分）

use serde::{Deserialize, Serialize};
use std::fmt;

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
pub fn detect_cross(fast: &[f64], slow: &[f64], lookback: usize) -> CrossType {
    let len = fast.len();
    if len < 2 {
        return CrossType::None;
    }
    let start = len.saturating_sub(lookback);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_crossing_above_slow_is_golden() {
        let fast = vec![1.0, 1.2, 1.6, 2.2];
        let slow = vec![1.0, 1.1, 1.4, 1.8];
        assert_eq!(detect_cross(&fast, &slow, 4), CrossType::GoldenCross);
    }

    #[test]
    fn fast_crossing_below_slow_is_death() {
        let fast = vec![3.0, 2.6, 2.0, 1.4];
        let slow = vec![3.0, 2.8, 2.4, 2.0];
        assert_eq!(detect_cross(&fast, &slow, 4), CrossType::DeathCross);
    }

    #[test]
    fn parallel_series_have_no_cross() {
        let fast = vec![2.0, 2.5, 3.0, 3.5];
        let slow = vec![1.0, 1.5, 2.0, 2.5];
        assert_eq!(detect_cross(&fast, &slow, 4), CrossType::None);
    }
}
