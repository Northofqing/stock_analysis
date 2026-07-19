//! 现金警戒 — 底仓现金不得低于阈值。
//! BR-015: 产业链集中度检查的下游消费者 (上游 position_tracker.rs:314-340, 当前禁用待 stock_position 加 chain_name 列后启用)

pub struct CashGuard {
    pub floor_pct: f64, // 现金占比下限，默认 15%
}

impl Default for CashGuard {
    fn default() -> Self {
        Self { floor_pct: 15.0 }
    }
}

#[derive(Debug, Clone)]
pub struct CashAlert {
    pub total_value: f64,
    pub market_value: f64,
    pub cash_est: f64,
    pub cash_pct: f64,
    pub below_floor: bool,
}

/// 检查现金占比是否低于底线。cash 由用户手动输入或从 ledger 读取。
pub fn check_cash(cash: f64, total_value: f64, guard: &CashGuard) -> Option<CashAlert> {
    if total_value <= 0.0 {
        return None;
    }
    let cash_pct = cash / total_value * 100.0;
    Some(CashAlert {
        total_value,
        market_value: total_value - cash,
        cash_est: cash,
        cash_pct,
        below_floor: cash_pct < guard.floor_pct,
    })
}

/// 格式化现金告警
pub fn format_cash_alert(alert: &CashAlert) -> String {
    if alert.below_floor {
        format!(
            "💰 现金预警: 现金占比 {:.0}%（底线 {:.0}%），预留弹药不足",
            alert.cash_pct, 15.0
        )
    } else {
        format!(
            "💰 现金: {:.0}% (总资产 ¥{:.0})",
            alert.cash_pct, alert.total_value
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cash_below_floor() {
        let alert = check_cash(5000.0, 100000.0, &CashGuard::default()).unwrap();
        assert!(alert.below_floor); // 5% < 15%
    }

    #[test]
    fn test_cash_ok() {
        let alert = check_cash(30000.0, 100000.0, &CashGuard::default()).unwrap();
        assert!(!alert.below_floor); // 30% > 15%
    }

    #[test]
    fn test_non_positive_total_is_rejected() {
        assert!(check_cash(0.0, 0.0, &CashGuard::default()).is_none());
        assert!(check_cash(0.0, -1.0, &CashGuard::default()).is_none());
    }

    #[test]
    fn test_alert_formatting_covers_both_states() {
        let below = check_cash(5_000.0, 100_000.0, &CashGuard::default()).unwrap();
        assert!(format_cash_alert(&below).contains("现金预警"));
        let healthy = check_cash(30_000.0, 100_000.0, &CashGuard::default()).unwrap();
        let text = format_cash_alert(&healthy);
        assert!(text.contains("现金: 30%"));
        assert!(text.contains("总资产 ¥100000"));
    }
}
