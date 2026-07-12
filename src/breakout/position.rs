//! 位置判断 — 距 60 日均线的偏离程度。

use super::signal::PricePosition;

/// 距 60 日均线偏离百分比 → 位置
pub fn classify_position(distance_from_ma60_pct: f64) -> PricePosition {
    if distance_from_ma60_pct > 15.0 {
        PricePosition::High
    } else if distance_from_ma60_pct < -15.0 {
        PricePosition::Low
    } else {
        PricePosition::Mid
    }
}

/// 计算距均线偏离百分比
pub fn distance_from_ma(price: f64, ma: f64) -> f64 {
    if ma <= 0.0 {
        return 0.0;
    }
    (price - ma) / ma * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_high_position() {
        assert_eq!(classify_position(20.0), PricePosition::High);
    }

    #[test]
    fn test_low_position() {
        assert_eq!(classify_position(-20.0), PricePosition::Low);
    }

    #[test]
    fn test_mid_position() {
        assert_eq!(classify_position(5.0), PricePosition::Mid);
    }
}
