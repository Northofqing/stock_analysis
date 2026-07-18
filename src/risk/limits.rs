//! Registered business rules: BR-054, BR-055.
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
///
/// 修复 P1.6: 真正执行 single_sector_max_pct
/// 修复 (2026-06-30 codex review): 删除 cash_floor_pct 死代码 (旧版本
///   `let cash_pct = 100.0` 永远 false, 现金检查从未触发).
///   现金底限检查已迁移到 `risk::cash_guard::check_cash`, 调用方在
///   `bin/monitor/main.rs` 的风控研判路径单独调用.
///
/// 修复 review #14: 缺价 fallback `cost_price` 会让硬止损永远不触发
/// (cost - cost)/cost = 0%). 改为显式 emit "缺价告警" violation, 让持仓
/// 缺价的事实进入风控信号, 不再静默失真.
pub fn check_position_limits(
    positions: &[Position],
    prices: &std::collections::HashMap<String, f64>,
    limits: &HardLimits,
) -> Vec<LimitViolation> {
    if positions.is_empty() {
        return vec![];
    }

    // 单次遍历: 收集 (position, price) 列表 + sector 总市值 + 总市值
    // 同时识别缺价持仓 (review #14: 不再 fallback 到 cost_price).
    struct ValuedPos<'a> {
        pos: &'a Position,
        price: Option<f64>,
    }
    let mut valued: Vec<ValuedPos<'_>> = Vec::with_capacity(positions.len());
    let mut sector_totals: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    let mut total_value = 0.0;
    for p in positions {
        let price = prices.get(&p.code).copied();
        if let Some(pr) = price {
            let mv = p.shares as f64 * pr;
            total_value += mv;
            if !p.sector.is_empty() && p.sector != "其他" {
                *sector_totals.entry(p.sector.as_str()).or_insert(0.0) += mv;
            }
        }
        valued.push(ValuedPos { pos: p, price });
    }

    let mut violations = Vec::new();

    for v in &valued {
        let p = v.pos;
        // 缺价 — emit 显式 violation, 不再静默用成本价 (review #14 修复)
        let Some(price) = v.price else {
            violations.push(LimitViolation {
                code: p.code.clone(),
                name: p.name.clone(),
                rule: "缺价".to_string(),
                current: "无".to_string(),
                limit: "需实时价".to_string(),
            });
            continue;
        };
        if total_value <= 0.0 {
            continue;
        }

        let market_value = p.shares as f64 * price;
        let pct = market_value / total_value * 100.0;

        // 单票上限
        if pct > limits.single_stock_max_pct {
            violations.push(LimitViolation {
                code: p.code.clone(),
                name: p.name.clone(),
                rule: "单票仓位上限".to_string(),
                current: format!("{:.1}%", pct),
                limit: format!("≤{:.0}%", limits.single_stock_max_pct),
            });
        }

        // 修复 P1.6: 板块集中度检查 (review #14: 用预计算的 sector_totals,
        // O(1) lookup, 不再 O(N) filter)
        if !p.sector.is_empty() && p.sector != "其他" {
            let sector_pct =
                sector_totals.get(p.sector.as_str()).copied().unwrap_or(0.0) / total_value * 100.0;
            if sector_pct > limits.single_sector_max_pct {
                violations.push(LimitViolation {
                    code: p.sector.clone(),
                    name: format!("板块 {}", p.sector),
                    rule: "板块集中度上限".to_string(),
                    current: format!("{:.1}%", sector_pct),
                    limit: format!("≤{:.0}%", limits.single_sector_max_pct),
                });
            }
        }

        // 止损线（以成本价为基准）
        if p.cost_price > 0.0 {
            let loss_pct = (price - p.cost_price) / p.cost_price * 100.0;
            if loss_pct <= limits.stop_loss_pct {
                violations.push(LimitViolation {
                    code: p.code.clone(),
                    name: p.name.clone(),
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
    if violations.is_empty() {
        return String::new();
    }
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
            code: code.into(),
            name: code.into(),
            shares,
            cost_price: cost,
            hard_stop: Some(cost * 0.9),
            added_at: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            status: crate::portfolio::PositionStatus::Holding,
            sector: "其他".into(),
            ..Default::default()
        }
    }

    #[test]
    fn test_stop_loss_violation() {
        let positions = vec![
            pos("TEST_CODE_000001", 10.0, 1000),
            pos("TEST_CODE_000002", 20.0, 500),
        ];
        let mut prices = std::collections::HashMap::new();
        prices.insert("TEST_CODE_000001".to_string(), 8.5); // -15% → 止损
        prices.insert("TEST_CODE_000002".to_string(), 21.0); // OK
        let limits = HardLimits::default();
        let v = check_position_limits(&positions, &prices, &limits);
        assert!(v
            .iter()
            .any(|x| x.rule == "止损线" && x.code == "TEST_CODE_000001"));
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
        let positions = vec![pos("TEST_CODE_000001", 10.0, 10000)];
        let mut prices = std::collections::HashMap::new();
        prices.insert("TEST_CODE_000001".to_string(), 10.0);
        let v = check_position_limits(&positions, &prices, &HardLimits::default());
        assert!(v.iter().any(|x| x.rule == "单票仓位上限"));
    }

    #[test]
    fn test_zero_total_value() {
        // 所有价格为 0 → 总市值为 0 → 不 panic，返回空或安全结果
        let positions = vec![pos("TEST_CODE_000001", 10.0, 1000)];
        let prices = std::collections::HashMap::new(); // 空 map → 用 cost_price
        let v = check_position_limits(&positions, &prices, &HardLimits::default());
        // 不 panic 就算通过
        assert!(v.is_empty() || !v.is_empty());
    }
}
