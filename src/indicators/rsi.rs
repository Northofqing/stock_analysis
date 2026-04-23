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

