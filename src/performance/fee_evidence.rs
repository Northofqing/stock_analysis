//! 2026-09-21 评估 #1+#2: 模拟盘逐笔成本证据 (派生, 不落库).
//!
//! 冻结的 `lot-rates-v1` 口径与 `src/strategy/lot.rs` 一致: 佣金万三
//! (最低 5 元, 双边) + 印花税千一 (仅卖出). 评估 §九 V11 旧模型口径: 100 股 ¥10 往返 =
//! 买 5 + 卖 5 + 印 1 = ¥11 = 1.1%; ¥322 仓位 = 3.2%.
//!
//! 派生值不写回 `paper_trades` (原「加四列」处方已作废, 见
//! memory/no-broker-integration-simulation-only): 卡片/闸门/引擎在
//! 消费点即时计算, 历史口径保持可追溯.

use crate::performance::economic_position::{CostBasisKind, FillCostEvidence, FillCostLedger};
use crate::strategy::lot;
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub use super::fee_policy::{
    a_share_stock_fill_fee_with_policy_v2, shanghai_execution_date, AShareFeePolicyV2,
    AShareFeeV2Error, AShareFillFeeV2, ExcludedFeeReason, FeeCoverage, FeeCoverageRequirement,
    FeeListingSegment, FeeMarket, FeeRate, FeeSecurityKind, QualifiedInstrument, StampTaxBracketV2,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillSide {
    Buy,
    Sell,
}

/// New research fee schedule. PaperLedgerV1 remains permanently bound to
/// `lot-rates-v1`; adopting this schedule requires a new ledger generation.
pub const A_SHARE_FEE_SCHEDULE_V2: &str = "a-share-policy-by-trade-date-v2";

/// Fixed-assumption compatibility wrapper for old research callers. New work
/// must pass an explicit policy and qualified instrument through
/// `a_share_stock_fill_fee_with_policy_v2`.
pub fn a_share_stock_fill_fee_v2(
    side: FillSide,
    notional_micro_cny: i64,
    trade_date: NaiveDate,
) -> Result<AShareFillFeeV2, AShareFeeV2Error> {
    let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
    a_share_stock_fill_fee_with_policy_v2(
        &policy,
        policy.scope(),
        side,
        notional_micro_cny,
        trade_date,
        FeeCoverageRequirement::ModeledComponentsOnly,
    )
}

/// Legacy Scenario adapter: its aggregate `f64` costs and schedule-only ID are
/// not per-fill authority. Future v2 consumers need the explicit policy result.
/// The v1 ledger constructor remains unchanged for historical replay.
pub fn a_share_fill_cost_ledger_v2(
    fills: &[(i64, FillSide, i64, NaiveDate)],
) -> Result<FillCostLedger, String> {
    let costs = fills
        .iter()
        .map(|(fill_id, side, notional_micro_cny, trade_date)| {
            let fee = a_share_stock_fill_fee_v2(*side, *notional_micro_cny, *trade_date)
                .map_err(|error| format!("fill {fill_id}: {error}"))?;
            Ok(FillCostEvidence {
                fill_id: *fill_id,
                adverse_cost: fee.total_micro_cny as f64 / 1_000_000.0,
                evidence_id: format!("fee-{A_SHARE_FEE_SCHEDULE_V2}:{fill_id}:{}", fee.trade_date),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(FillCostLedger {
        basis_id: A_SHARE_FEE_SCHEDULE_V2.to_owned(),
        kind: CostBasisKind::Scenario,
        costs,
    })
}

/// One qualified executed fill for the explicit fee-policy scenario adapter.
/// The caller must supply instrument classification from a verified source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualifiedFillV2 {
    pub fill_id: i64,
    pub instrument: QualifiedInstrument,
    pub side: FillSide,
    pub notional_micro_cny: i64,
    pub executed_at_utc: DateTime<Utc>,
}

/// Project a single immutable policy instance over dated fills for research.
/// The resulting basis and each evidence ID bind the policy descriptor; this
/// does not write PaperLedgerV1 or attest a broker receipt.
pub fn a_share_fill_cost_ledger_with_policy_v2(
    policy: &AShareFeePolicyV2,
    fills: &[QualifiedFillV2],
    required_coverage: FeeCoverageRequirement,
) -> Result<FillCostLedger, String> {
    let basis_id = policy.instance_id();
    let mut seen = HashSet::with_capacity(fills.len());
    let mut costs = Vec::with_capacity(fills.len());
    for fill in fills {
        if fill.fill_id <= 0 || !seen.insert(fill.fill_id) {
            return Err("fee-policy v2 fill identities must be positive and unique".to_owned());
        }
        let trade_date = shanghai_execution_date(fill.executed_at_utc);
        let fee = a_share_stock_fill_fee_with_policy_v2(
            policy,
            fill.instrument,
            fill.side,
            fill.notional_micro_cny,
            trade_date,
            required_coverage,
        )
        .map_err(|error| format!("fill {}: {error}", fill.fill_id))?;
        let side = match fill.side {
            FillSide::Buy => "Buy",
            FillSide::Sell => "Sell",
        };
        let material = format!(
            "fee-policy-v2-fill-evidence/v1\npolicy={basis_id}\nfill_id={}\nside={side}\nnotional_micro_cny={}\nexecuted_at_utc={}\nfee_micro_cny={}\n",
            fill.fill_id,
            fill.notional_micro_cny,
            fill.executed_at_utc.to_rfc3339_opts(SecondsFormat::Nanos, true),
            fee.total_micro_cny,
        );
        costs.push(FillCostEvidence {
            fill_id: fill.fill_id,
            adverse_cost: fee.total_micro_cny as f64 / 1_000_000.0,
            evidence_id: format!(
                "fee-policy-v2:sha256:{}",
                hex::encode(Sha256::digest(material))
            ),
        });
    }
    Ok(FillCostLedger {
        basis_id,
        kind: CostBasisKind::Scenario,
        costs,
    })
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

    fn qualified_main_a() -> QualifiedInstrument {
        QualifiedInstrument::new(
            FeeMarket::Shanghai,
            FeeSecurityKind::AShareStock,
            FeeListingSegment::ShanghaiMainA,
        )
        .expect("supported modeled instrument")
    }

    fn executed_at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("test timestamp")
            .with_timezone(&Utc)
    }

    #[test]
    fn explicit_v2_ledger_binds_policy_and_shanghai_execution_cutover() {
        let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
        let instrument = qualified_main_a();
        let before = QualifiedFillV2 {
            fill_id: 1,
            instrument,
            side: FillSide::Sell,
            notional_micro_cny: 1_000_000_000,
            executed_at_utc: executed_at("2023-08-27T15:59:59Z"),
        };
        let after = QualifiedFillV2 {
            fill_id: 2,
            executed_at_utc: executed_at("2023-08-27T16:00:00Z"),
            ..before
        };
        let ledger = a_share_fill_cost_ledger_with_policy_v2(
            &policy,
            &[before, after],
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .expect("dated modeled fees");
        assert_eq!(ledger.basis_id, policy.instance_id());
        assert_eq!(ledger.kind, CostBasisKind::Scenario);
        assert_eq!(ledger.costs[0].adverse_cost, 6.0);
        assert_eq!(ledger.costs[1].adverse_cost, 5.5);
        assert_ne!(ledger.costs[0].evidence_id, ledger.costs[1].evidence_id);

        let revised = AShareFeePolicyV2::new(
            instrument,
            FeeRate::new(3, 10_000).unwrap(),
            5_000_000,
            FeeCoverage::initial_model(),
            "TEST_CODE_revised_source",
        )
        .unwrap();
        let revised_ledger = a_share_fill_cost_ledger_with_policy_v2(
            &revised,
            &[before, after],
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        assert_ne!(ledger.basis_id, revised_ledger.basis_id);
        assert_ne!(
            ledger.costs[0].evidence_id,
            revised_ledger.costs[0].evidence_id
        );
    }

    #[test]
    fn explicit_v2_ledger_rejects_duplicates_scope_and_complete_cost_claim() {
        let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
        let fill = QualifiedFillV2 {
            fill_id: 10,
            instrument: qualified_main_a(),
            side: FillSide::Buy,
            notional_micro_cny: 1_000_000_000,
            executed_at_utc: executed_at("2026-09-28T03:00:00Z"),
        };
        assert!(a_share_fill_cost_ledger_with_policy_v2(
            &policy,
            &[fill, fill],
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap_err()
        .contains("unique"));
        let star = QualifiedInstrument::new(
            FeeMarket::Shanghai,
            FeeSecurityKind::AShareStock,
            FeeListingSegment::ShanghaiStarA,
        )
        .unwrap();
        assert!(a_share_fill_cost_ledger_with_policy_v2(
            &policy,
            &[QualifiedFillV2 {
                instrument: star,
                ..fill
            }],
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap_err()
        .contains("scope"));
        assert!(a_share_fill_cost_ledger_with_policy_v2(
            &policy,
            &[fill],
            FeeCoverageRequirement::CompleteTradingCost,
        )
        .unwrap_err()
        .contains("complete trading cost"));
    }

    #[test]
    fn v2_trade_date_cutovers_preserve_legacy_fee_model() {
        let before_single_side = NaiveDate::from_ymd_opt(2008, 9, 18).unwrap();
        assert_eq!(
            a_share_stock_fill_fee_v2(FillSide::Sell, 1_000_000_000, before_single_side),
            Err(AShareFeeV2Error::UnsupportedTradeDate)
        );
        for (date, expected_stamp) in [
            (NaiveDate::from_ymd_opt(2008, 9, 19).unwrap(), 1_000_000),
            (NaiveDate::from_ymd_opt(2023, 8, 27).unwrap(), 1_000_000),
            (NaiveDate::from_ymd_opt(2023, 8, 28).unwrap(), 500_000),
        ] {
            let sell = a_share_stock_fill_fee_v2(FillSide::Sell, 1_000_000_000, date).unwrap();
            let buy = a_share_stock_fill_fee_v2(FillSide::Buy, 1_000_000_000, date).unwrap();
            assert_eq!(sell.commission_micro_cny, 5_000_000);
            assert_eq!(sell.stamp_tax_micro_cny, expected_stamp);
            assert_eq!(buy.stamp_tax_micro_cny, 0);
        }
        assert_eq!(fill_adverse_cost(FillSide::Sell, 1_000.0), 6.0);
    }

    #[test]
    fn v2_round_trip_and_large_fill_use_micro_cny_half_up() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let buy = a_share_stock_fill_fee_v2(FillSide::Buy, 1_000_000_000, date).unwrap();
        let sell = a_share_stock_fill_fee_v2(FillSide::Sell, 1_000_000_000, date).unwrap();
        assert_eq!(sell.basis_id, A_SHARE_FEE_SCHEDULE_V2);
        assert_eq!(buy.total_micro_cny + sell.total_micro_cny, 10_500_000);
        let large = a_share_stock_fill_fee_v2(FillSide::Sell, 100_000_000_000, date).unwrap();
        assert_eq!(large.commission_micro_cny, 30_000_000);
        assert_eq!(large.stamp_tax_micro_cny, 50_000_000);
        assert_eq!(
            a_share_stock_fill_fee_v2(FillSide::Buy, 0, date),
            Err(AShareFeeV2Error::InvalidNotional)
        );
        let ledger = a_share_fill_cost_ledger_v2(&[
            (1, FillSide::Buy, 1_000_000_000, date),
            (2, FillSide::Sell, 1_000_000_000, date),
        ])
        .unwrap();
        assert_eq!(ledger.basis_id, A_SHARE_FEE_SCHEDULE_V2);
        assert_eq!(ledger.kind, CostBasisKind::Scenario);
        assert_eq!(
            ledger.costs[0].adverse_cost + ledger.costs[1].adverse_cost,
            10.5
        );
        assert!(ledger.costs[1].evidence_id.contains("2026-09-28"));
    }

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
        let ledger =
            lot_rate_fill_cost_ledger(&[(1, FillSide::Buy, 1000.0), (2, FillSide::Sell, 1000.0)])
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
