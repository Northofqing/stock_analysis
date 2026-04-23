//! divergence（从 indicators.rs 拆分）

use serde::{Deserialize, Serialize};
use std::fmt;
use super::{max_with_index, min_with_index};



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

