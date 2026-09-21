//! rsi（从 indicators.rs 拆分）

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
        return vec![
            RsiPoint {
                rsi6: 50.0,
                rsi12: 50.0,
                rsi24: 50.0
            };
            len
        ];
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
pub(super) fn rsi_single(closes: &[f64], period: usize) -> Vec<f64> {
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
    // 2026-09-21 (系统评估 §6.3): 短序列时循环仅跑 first_window 次却除以
    // period → RSI 被系统性低估。以实际窗口长度做简单平均 (Wilder 平滑
    // 只应在首窗之后; 首窗为简单平均)。
    let divisor = first_window.max(1) as f64;
    avg_gain /= divisor;
    avg_loss /= divisor;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_sequence_first_window_uses_actual_window_length() {
        // 序列长度 3 < period 6: first_window=2, 首窗应为简单平均而非除 period
        let closes = vec![10.0, 11.0, 12.0];
        let out = rsi_single(&closes, 6);
        // 全涨序列: avg_gain=2, avg_loss=0 → RSI=100 (不再被 period 低估)
        assert_eq!(out[2], 100.0);
    }

    #[test]
    fn full_window_still_wilder_shaped() {
        let closes = vec![10.0, 11.0, 10.5, 11.5, 11.0, 11.8, 12.2];
        let out = rsi_single(&closes, 6);
        // 首窗 = period=6, 全涨为主 → RSI 接近 100 但不等于 (存在回调)
        assert!(out[6] > 50.0);
    }
}
