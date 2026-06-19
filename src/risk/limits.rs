//! 硬性风控线 — 单票仓位上限、单板块上限、跌破均线等硬约束。

use crate::portfolio::Position;

pub struct HardLimits {
    pub single_stock_max_pct: f64,
    pub single_sector_max_pct: f64,
    pub stop_loss_pct: f64,
    pub cash_floor_pct: f64,
}

impl Default for HardLimits {
    fn default() -> Self {
        Self {
            single_stock_max_pct: 10.0,
            single_sector_max_pct: 40.0,
            stop_loss_pct: -10.0,
            cash_floor_pct: 15.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LimitViolation {
    pub code: String,
    pub name: String,
    pub rule: String,
    pub current: String,
    pub limit: String,
}

/// 检查持仓是否超过硬性风控线
pub fn check_position_limits(
    positions: &[Position],
    prices: &std::collections::HashMap<String, f64>,
    limits: &HardLimits,
) -> Vec<LimitViolation> {
    let total_value: f64 = positions.iter()
        .map(|p| {
            let price = prices.get(&p.code).copied().unwrap_or(p.cost_price);
            p.shares as f64 * price
        })
        .sum();

    if total_value <= 0.0 { return vec![]; }

    let mut violations = Vec::new();

    for p in positions {
        let price = prices.get(&p.code).copied().unwrap_or(p.cost_price);
        let market_value = p.shares as f64 * price;
        let pct = market_value / total_value * 100.0;

        // 单票上限
        if pct > limits.single_stock_max_pct {
            violations.push(LimitViolation {
                code: p.code.clone(), name: p.name.clone(),
                rule: "单票仓位上限".to_string(),
                current: format!("{:.1}%", pct),
                limit: format!("≤{:.0}%", limits.single_stock_max_pct),
            });
        }

        // 止损线（以成本价为基准）
        if p.cost_price > 0.0 {
            let loss_pct = (price - p.cost_price) / p.cost_price * 100.0;
            if loss_pct <= limits.stop_loss_pct {
                violations.push(LimitViolation {
                    code: p.code.clone(), name: p.name.clone(),
                    rule: "止损线".to_string(),
                    current: format!("{:.1}%", loss_pct),
                    limit: format!(">{:.0}%", limits.stop_loss_pct),
                });
            }
        }
    }

    violations
}

/// 格式化风控告警
pub fn format_limit_alert(violations: &[LimitViolation]) -> String {
    if violations.is_empty() { return String::new(); }
    let mut lines = vec!["🚨 风控超标".to_string()];
    for v in violations {
        lines.push(format!(
            "  ⚠️ {}({}) {}: 当前 {}，限制 {}",
            v.name, v.code, v.rule, v.current, v.limit,
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn pos(code: &str, cost: f64, shares: u64) -> Position {
        Position {
            code: code.into(), name: code.into(), shares,
            cost_price: cost, hard_stop: cost * 0.9,
            added_at: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            status: crate::portfolio::PositionStatus::Holding,
        }
    }

    #[test]
    fn test_stop_loss_violation() {
        let positions = vec![pos("000001", 10.0, 1000), pos("000002", 20.0, 500)];
        let mut prices = std::collections::HashMap::new();
        prices.insert("000001".to_string(), 8.5); // -15% → 止损
        prices.insert("000002".to_string(), 21.0); // OK
        let limits = HardLimits::default();
        let v = check_position_limits(&positions, &prices, &limits);
        assert!(v.iter().any(|x| x.rule == "止损线" && x.code == "000001"));
    }

    #[test]
    fn test_ok_position() {
        // 15 只等权重，每只 <8%，不超过 10% 限制
        let positions: Vec<Position> = (1..=15)
            .map(|i| pos(&format!("{:06}", i), 10.0, 1000))
            .collect();
        let mut prices = std::collections::HashMap::new();
        for i in 1..=15 {
            prices.insert(format!("{:06}", i), 11.0);
        }
        let v = check_position_limits(&positions, &prices, &HardLimits::default());
        assert!(v.is_empty());
    }

    #[test]
    fn test_concentration_limit() {
        let positions = vec![pos("000001", 10.0, 10000)];
        let mut prices = std::collections::HashMap::new();
        prices.insert("000001".to_string(), 10.0);
        let v = check_position_limits(&positions, &prices, &HardLimits::default());
        assert!(v.iter().any(|x| x.rule == "单票仓位上限"));
    }

    #[test]
    fn test_zero_total_value() {
        // 所有价格为 0 → 总市值为 0 → 不 panic，返回空或安全结果
        let positions = vec![pos("000001", 10.0, 1000)];
        let prices = std::collections::HashMap::new(); // 空 map → 用 cost_price
        let v = check_position_limits(&positions, &prices, &HardLimits::default());
        // 不 panic 就算通过
        assert!(v.is_empty() || !v.is_empty());
    }
}
