//! macd（从 indicators.rs 拆分）

use super::ema;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rising_series_gives_positive_dif_and_histogram() {
        let closes: Vec<f64> = (1..=40).map(|i| 10.0 + i as f64 * 0.5).collect();
        let out = calc_macd(&closes, 12, 26, 9);
        assert_eq!(out.len(), closes.len());
        let last = out.last().expect("nonempty");
        // 持续上涨 → 快线在慢线上方, 柱为正
        assert!(last.dif > last.dea);
        assert!(last.histogram > 0.0);
    }

    #[test]
    fn falling_series_gives_negative_histogram() {
        let closes: Vec<f64> = (1..=40).map(|i| 30.0 - i as f64 * 0.3).collect();
        let out = calc_macd(&closes, 12, 26, 9);
        let last = out.last().expect("nonempty");
        assert!(last.histogram < 0.0);
    }
}
