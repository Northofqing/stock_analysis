//! 2026-09-21 评估 #1+#2: 模拟盘逐笔成本证据 (派生, 不落库).
//!
//! 费率口径与 `src/strategy/lot.rs` 一致: 佣金万三 (最低 5 元, 双边) +
//! 印花税千一 (仅卖出). 评估 §九 V11 实测口径: 100 股 ¥10 往返 =
//! 买 5 + 卖 5 + 印 1 = ¥11 = 1.1%; ¥322 仓位 = 3.2%.
//!
//! 派生值不写回 `paper_trades` (原「加四列」处方已作废, 见
//! memory/no-broker-integration-simulation-only): 卡片/闸门/引擎在
//! 消费点即时计算, 历史口径保持可追溯.

use crate::performance::economic_position::{CostBasisKind, FillCostEvidence, FillCostLedger};
use crate::strategy::lot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillSide {
    Buy,
    Sell,
}

/// 单边成交不利成本: 佣金 (万三, 最低 ¥5) + 印花税 (仅卖出, 千一).
pub fn fill_adverse_cost(side: FillSide, notional: f64) -> f64 {
    let mut cost = lot::min_commission(notional);
    if matches!(side, FillSide::Sell) {
        cost += lot::stamp_tax(notional);
    }
    cost
}

/// 往返 (买 + 卖) 不利成本金额.
pub fn round_trip_adverse_cost(buy_notional: f64, sell_notional: f64) -> f64 {
    fill_adverse_cost(FillSide::Buy, buy_notional)
        + fill_adverse_cost(FillSide::Sell, sell_notional)
}

/// 净收益率 (百分比): (卖额 − 买额 − 往返成本) / 买额 × 100.
///
/// 与评估 §8.1 一致: ¥1,000 仓位平进平出 = −1.1% (不再是 +0.00%).
pub fn net_return_pct(buy_price: f64, sell_price: f64, quantity: u64) -> f64 {
    let buy_notional = buy_price * quantity as f64;
    let sell_notional = sell_price * quantity as f64;
    (sell_notional - buy_notional - round_trip_adverse_cost(buy_notional, sell_notional))
        / buy_notional
        * 100.0
}

/// Net return for a FIFO sale whose buy-side fee was allocated from its
/// original buy fills. The sell-side fee is charged once for this sell fill.
pub fn net_return_pct_with_allocated_buy_fee(
    buy_notional: f64,
    sell_notional: f64,
    allocated_buy_fee: f64,
) -> Result<f64, String> {
    if !buy_notional.is_finite()
        || buy_notional <= 0.0
        || !sell_notional.is_finite()
        || sell_notional <= 0.0
        || !allocated_buy_fee.is_finite()
        || allocated_buy_fee < 0.0
    {
        return Err("FIFO fee inputs must be finite and positive".to_owned());
    }
    let sell_fee = fill_adverse_cost(FillSide::Sell, sell_notional);
    let result =
        (sell_notional - buy_notional - allocated_buy_fee - sell_fee) / buy_notional * 100.0;
    if !result.is_finite() {
        return Err("FIFO net return is not finite".to_owned());
    }
    Ok(result)
}

/// 按成交行构建费率口径 FillCostLedger (喂 economic_position 引擎).
///
/// `fills` = (fill_id, side, notional). basis_id 冻结口径版本:
/// "lot-rates-v1" (佣金万三最低5 + 印花税千一).
///
/// ⚠️ kind = Scenario 而非 Observed: 引擎公开路径拒绝 Observed (BR-251
/// 私有 replay seam 专属); 费率推导成本属模型口径, Scenario 语义准确.
pub fn lot_rate_fill_cost_ledger(fills: &[(i64, FillSide, f64)]) -> Result<FillCostLedger, String> {
    let basis_id = "lot-rates-v1";
    let costs = fills
        .iter()
        .map(|(fill_id, side, notional)| FillCostEvidence {
            fill_id: *fill_id,
            adverse_cost: fill_adverse_cost(*side, *notional),
            evidence_id: format!("fee-lot-rates-v1:{fill_id}"),
        })
        .collect();
    Ok(FillCostLedger {
        basis_id: basis_id.to_owned(),
        kind: CostBasisKind::Scenario,
        costs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 评估 V11: 100 股 ¥10 往返 = 买 5 + 卖 5 + 印 1 = ¥11 = 1.1%.
    #[test]
    fn v11_small_lot_round_trip_is_1_1_percent() {
        let cost = round_trip_adverse_cost(1000.0, 1000.0);
        assert!((cost - 11.0).abs() < 1e-9, "cost={cost}");
        let net = net_return_pct(10.0, 10.0, 100);
        assert!((net + 1.1).abs() < 1e-9, "net={net}");
    }

    /// 评估 V11: ¥322 仓位 = (5 + 5 + 0.322) / 322 = 3.205%.
    #[test]
    fn v11_322_yuan_position_cost_is_3_2_percent() {
        let cost = round_trip_adverse_cost(322.0, 322.0);
        assert!((cost - 10.322).abs() < 1e-9, "cost={cost}");
        let net = net_return_pct(3.22, 3.22, 100);
        assert!((net + 3.20559).abs() < 1e-3, "net={net}");
    }

    /// 大额成交佣金超过保底: 双边 0.03% + 印花 0.1% = 0.16%.
    #[test]
    fn large_notional_uses_rate_without_floor() {
        let cost = round_trip_adverse_cost(100_000.0, 100_000.0);
        assert!((cost - 160.0).abs() < 1e-9, "cost={cost}");
    }

    /// 印花税只在卖出侧.
    #[test]
    fn stamp_tax_applies_to_sell_side_only() {
        let buy = fill_adverse_cost(FillSide::Buy, 10_000.0);
        let sell = fill_adverse_cost(FillSide::Sell, 10_000.0);
        assert!((buy - 5.0).abs() < 1e-9, "buy={buy}");
        assert!((sell - 15.0).abs() < 1e-9, "sell={sell}");
    }

    #[test]
    fn ledger_covers_every_fill_with_stable_basis() {
        let ledger = lot_rate_fill_cost_ledger(&[
            (1, FillSide::Buy, 1000.0),
            (2, FillSide::Sell, 1000.0),
        ])
        .expect("ledger");
        assert_eq!(ledger.basis_id, "lot-rates-v1");
        assert_eq!(ledger.kind, CostBasisKind::Scenario);
        assert_eq!(ledger.costs.len(), 2);
        assert!((ledger.costs[0].adverse_cost - 5.0).abs() < 1e-9);
        assert!((ledger.costs[1].adverse_cost - 6.0).abs() < 1e-9);
    }

    #[test]
    fn three_buy_fills_charge_three_buy_commissions_on_one_sell() {
        let ledger = lot_rate_fill_cost_ledger(&[
            (1, FillSide::Buy, 1000.0),
            (2, FillSide::Buy, 1000.0),
            (3, FillSide::Buy, 1000.0),
            (4, FillSide::Sell, 3000.0),
        ])
        .expect("four fill cost facts");
        let buy_fee: f64 = ledger.costs[..3].iter().map(|cost| cost.adverse_cost).sum();
        let total_fee: f64 = ledger.costs.iter().map(|cost| cost.adverse_cost).sum();
        let net = net_return_pct_with_allocated_buy_fee(3000.0, 3000.0, buy_fee)
            .expect("valid FIFO fee evidence");
        assert!((buy_fee - 15.0).abs() < 1e-9);
        assert!((total_fee - 23.0).abs() < 1e-9);
        assert!((net + 23.0 / 3000.0 * 100.0).abs() < 1e-9, "net={net}");
    }
}
