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

