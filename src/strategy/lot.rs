//! A 股整百股 / 最小佣金工具
//!
//! 修复：QUANT_ANALYST_REVIEW §2.4, §2.5
//! 原 bug：
//!   - BacktestEngine 不按 100 股取整，回测出现 "17 股" 伪持仓
//!   - 佣金不保底 5 元（A 股标准），小额交易回测低估成本

/// A 股 1 手 = 100 股
pub const LOT_SIZE: u64 = 100;

/// 券商佣金率（万三）
pub const COMMISSION_RATE: f64 = 0.0003;

/// 最低佣金 5 元（A 股监管要求）
pub const MIN_COMMISSION: f64 = 5.0;

/// 印花税（仅卖出，千一）
pub const STAMP_TAX_RATE: f64 = 0.001;

/// 向下取整到 100 的倍数。负数 / NaN 返回 0。
pub fn round_lot(shares: f64) -> u64 {
    if !shares.is_finite() || shares < 0.0 {
        return 0;
    }
    let n = (shares / LOT_SIZE as f64).floor() as u64;
    n * LOT_SIZE
}

/// 计算佣金，含最低 5 元保底。
/// `amount` 是成交金额（元）。返回佣金（元）。
/// amount <= 0 时返回 0。
pub fn min_commission(amount: f64) -> f64 {
    if !amount.is_finite() || amount <= 0.0 {
        return 0.0;
    }
    (amount * COMMISSION_RATE).max(MIN_COMMISSION)
}

/// 计算印花税（仅卖出方收取）。
/// `amount` 是成交金额（元）。返回印花税（元）。
pub fn stamp_tax(amount: f64) -> f64 {
    if !amount.is_finite() || amount <= 0.0 {
        return 0.0;
    }
    amount * STAMP_TAX_RATE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lot_size_constant() {
        assert_eq!(LOT_SIZE, 100);
    }
    #[test]
    fn min_commission_constant() {
        assert_eq!(MIN_COMMISSION, 5.0);
    }

    #[test]
    fn round_lot_basic() {
        assert_eq!(round_lot(150.0), 100);
        assert_eq!(round_lot(199.0), 100);
        assert_eq!(round_lot(200.0), 200);
        assert_eq!(round_lot(201.0), 200);
        assert_eq!(round_lot(0.0), 0);
        assert_eq!(round_lot(50.0), 0); // < 1 手
        assert_eq!(round_lot(99.0), 0);
        assert_eq!(round_lot(100.0), 100);
        assert_eq!(round_lot(101.0), 100);
        assert_eq!(round_lot(17.0), 0); // 17 股伪持仓 -> 0
    }

    #[test]
    fn round_lot_negative_or_nan() {
        assert_eq!(round_lot(-1.0), 0);
        assert_eq!(round_lot(f64::NAN), 0);
    }

    #[test]
    fn min_commission_floor() {
        // 1 万元: 0.0003*10000 = 3 元, 拉满到 5
        assert!((min_commission(10_000.0) - 5.0).abs() < 1e-6);
        // 5 万元: 0.0003*50000 = 15 元
        assert!((min_commission(50_000.0) - 15.0).abs() < 1e-6);
        // 2 万元: 0.0003*20000 = 6 元, 已超过 5
        assert!((min_commission(20_000.0) - 6.0).abs() < 1e-6);
        // 1.5 万元: 0.0003*15000 = 4.5, 拉满到 5
        assert!((min_commission(15_000.0) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn min_commission_zero_or_negative() {
        assert_eq!(min_commission(0.0), 0.0);
        assert_eq!(min_commission(-1.0), 0.0);
    }

    #[test]
    fn stamp_tax_basic() {
        assert!((stamp_tax(10_000.0) - 10.0).abs() < 1e-6);
        assert_eq!(stamp_tax(0.0), 0.0);
    }
}
