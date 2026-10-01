//! Explicit cash and inventory owner for modeled v2 research fills.
//! This is not an exchange execution, source qualification, or a legacy trade ledger.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};

use crate::market_domain::{AssetClass, Exchange, InstrumentId};
use crate::performance::fee_evidence::{
    AShareFeePolicyV2, AShareFeeV2Error, FeeCoverageRequirement, FillSide, QualifiedInstrument,
};

use super::research_fill_v2::{
    prepare_research_fill_with_policy_v2, PreparedResearchFillV2, ResearchFillInputV2,
    ResearchFillPrestateV2, ResearchFillV2Error,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchHoldingV2 {
    pub instrument_id: InstrumentId,
    pub quantity: u64,
}

/// An exact post-rebalance quantity at one caller-specified modeled execution
/// instant. Every existing holding must appear, including targets of zero.
/// A fill ID is supplied only when the quantity changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchRebalanceTargetV2 {
    pub instrument_id: InstrumentId,
    pub assumed_instrument: QualifiedInstrument,
    pub executed_price_micro_cny: i64,
    pub target_quantity: u64,
    pub fill_id: Option<i64>,
}

/// Applies caller-specified assumed fills in their supplied order. Cash and
/// inventory use integers; each successful fill retains its complete effect.
/// Holdings are exposed in ascending security-code order. A fresh owner starts
/// with cash and no positions, and does not reuse legacy `Trade` or its fees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchPortfolioV2 {
    policy: AShareFeePolicyV2,
    required_coverage: FeeCoverageRequirement,
    initial_cash_micro_cny: i64,
    cash_micro_cny: i64,
    holdings_by_code: BTreeMap<String, ResearchHoldingV2>,
    seen_fill_ids: BTreeSet<i64>,
    effects: Vec<PreparedResearchFillV2>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResearchPortfolioV2Error {
    #[error("research initial cash cannot be negative")]
    InvalidInitialCash,
    #[error("research fill ID is duplicated: {0}")]
    DuplicateFillId(i64),
    #[error("research rebalance target is duplicated: {0}")]
    DuplicateTarget(String),
    #[error("research rebalance omits an existing holding: {0}")]
    MissingHoldingTarget(String),
    #[error("research rebalance target changes an existing instrument identity: {0}")]
    TargetIdentityMismatch(String),
    #[error("research rebalance changed target needs a fill ID: {0}")]
    MissingRebalanceFillId(String),
    #[error("research rebalance unchanged target must not have a fill ID: {0}")]
    UnneededRebalanceFillId(String),
    #[error("research fill cannot be applied: {0}")]
    Fill(#[from] ResearchFillV2Error),
}

impl ResearchPortfolioV2 {
    pub fn new(
        policy: AShareFeePolicyV2,
        initial_cash_micro_cny: i64,
        required_coverage: FeeCoverageRequirement,
    ) -> Result<Self, ResearchPortfolioV2Error> {
        if initial_cash_micro_cny < 0 {
            return Err(ResearchPortfolioV2Error::InvalidInitialCash);
        }
        Ok(Self {
            policy,
            required_coverage,
            initial_cash_micro_cny,
            cash_micro_cny: initial_cash_micro_cny,
            holdings_by_code: BTreeMap::new(),
            seen_fill_ids: BTreeSet::new(),
            effects: Vec::new(),
        })
    }

    pub fn policy(&self) -> &AShareFeePolicyV2 {
        &self.policy
    }

    pub fn required_coverage(&self) -> FeeCoverageRequirement {
        self.required_coverage
    }

    pub fn initial_cash_micro_cny(&self) -> i64 {
        self.initial_cash_micro_cny
    }

    pub fn cash_micro_cny(&self) -> i64 {
        self.cash_micro_cny
    }

    pub fn holdings(&self) -> impl Iterator<Item = &ResearchHoldingV2> {
        self.holdings_by_code.values()
    }

    pub fn effects(&self) -> &[PreparedResearchFillV2] {
        &self.effects
    }

    /// Prepare the complete effect before mutating any owner state. Rejected
    /// fills consume neither an ID nor cash/inventory and leave no effect.
    pub fn apply_fill(
        &mut self,
        input: ResearchFillInputV2,
    ) -> Result<&PreparedResearchFillV2, ResearchPortfolioV2Error> {
        if self.seen_fill_ids.contains(&input.fill_id) {
            return Err(ResearchPortfolioV2Error::DuplicateFillId(input.fill_id));
        }
        let code = input.instrument_id.code().to_owned();
        let prestate = ResearchFillPrestateV2 {
            cash_micro_cny: self.cash_micro_cny,
            held_quantity: self
                .holdings_by_code
                .get(&code)
                .map_or(0, |holding| holding.quantity),
        };
        let effect = prepare_research_fill_with_policy_v2(
            &self.policy,
            &input,
            prestate,
            self.required_coverage,
        )?;

        self.cash_micro_cny = effect.poststate.cash_micro_cny;
        if effect.poststate.held_quantity == 0 {
            self.holdings_by_code.remove(&code);
        } else {
            self.holdings_by_code.insert(
                code,
                ResearchHoldingV2 {
                    instrument_id: input.instrument_id,
                    quantity: effect.poststate.held_quantity,
                },
            );
        }
        self.seen_fill_ids.insert(input.fill_id);
        self.effects.push(effect);
        Ok(self.effects.last().expect("effect was just pushed"))
    }

    /// Apply one exact-quantity research rebalance as a transaction. Targets
    /// are sorted by security code, sells are prepared before buys, and every
    /// rejected fill leaves the original owner unchanged. This does not infer
    /// target weights, exchange eligibility, liquidity, T+1, or a real fill.
    pub fn rebalance_exact_targets(
        &mut self,
        assumed_executed_at_utc: DateTime<Utc>,
        targets: Vec<ResearchRebalanceTargetV2>,
    ) -> Result<&[PreparedResearchFillV2], ResearchPortfolioV2Error> {
        let mut by_code = BTreeMap::new();
        for target in targets {
            let code = target.instrument_id.code().to_owned();
            if by_code.insert(code.clone(), target).is_some() {
                return Err(ResearchPortfolioV2Error::DuplicateTarget(code));
            }
        }
        for code in self.holdings_by_code.keys() {
            if !by_code.contains_key(code) {
                return Err(ResearchPortfolioV2Error::MissingHoldingTarget(code.clone()));
            }
        }

        let mut sells = Vec::new();
        let mut buys = Vec::new();
        for (code, target) in by_code {
            if target.instrument_id.exchange() != Exchange::Shanghai
                || target.instrument_id.asset_class() != AssetClass::Equity
            {
                return Err(ResearchFillV2Error::UnsupportedSecurityIdentity.into());
            }
            if target.assumed_instrument != self.policy.scope() {
                return Err(ResearchFillV2Error::Fee(AShareFeeV2Error::ScopeMismatch).into());
            }
            if target.executed_price_micro_cny <= 0 {
                return Err(ResearchFillV2Error::InvalidPrice.into());
            }
            let current = self.holdings_by_code.get(&code);
            if current.is_some_and(|holding| holding.instrument_id != target.instrument_id) {
                return Err(ResearchPortfolioV2Error::TargetIdentityMismatch(code));
            }
            let held = current.map_or(0, |holding| holding.quantity);
            let (side, quantity) = if target.target_quantity < held {
                (FillSide::Sell, held - target.target_quantity)
            } else {
                (FillSide::Buy, target.target_quantity - held)
            };
            if quantity == 0 {
                if target.fill_id.is_some() {
                    return Err(ResearchPortfolioV2Error::UnneededRebalanceFillId(code));
                }
                continue;
            }
            let fill_id = target
                .fill_id
                .ok_or(ResearchPortfolioV2Error::MissingRebalanceFillId(code))?;
            let fill = ResearchFillInputV2 {
                fill_id,
                instrument_id: target.instrument_id,
                assumed_instrument: target.assumed_instrument,
                side,
                assumed_executed_at_utc,
                executed_price_micro_cny: target.executed_price_micro_cny,
                quantity,
            };
            match side {
                FillSide::Sell => sells.push(fill),
                FillSide::Buy => buys.push(fill),
            }
        }

        let previous_effect_count = self.effects.len();
        let mut staged = self.clone();
        for fill in sells.into_iter().chain(buys) {
            staged.apply_fill(fill)?;
        }
        *self = staged;
        Ok(&self.effects[previous_effect_count..])
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::market_domain::{AssetClass, Exchange};
    use crate::performance::fee_evidence::{
        AShareFeeV2Error, FeeCoverage, FeeListingSegment, FeeMarket, FeeRate, FeeSecurityKind,
        FillSide, QualifiedInstrument,
    };

    fn policy(commission_numerator: i64) -> AShareFeePolicyV2 {
        AShareFeePolicyV2::new(
            QualifiedInstrument::new(
                FeeMarket::Shanghai,
                FeeSecurityKind::AShareStock,
                FeeListingSegment::ShanghaiMainA,
            )
            .unwrap(),
            FeeRate::new(commission_numerator, 10_000).unwrap(),
            5_000_000,
            FeeCoverage::initial_model(),
            "TEST_CODE_m4_research_portfolio_v2",
        )
        .unwrap()
    }

    fn portfolio(cash: i64) -> ResearchPortfolioV2 {
        ResearchPortfolioV2::new(
            policy(3),
            cash,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap()
    }

    fn fill(id: i64, code: &str, side: FillSide) -> ResearchFillInputV2 {
        ResearchFillInputV2 {
            fill_id: id,
            instrument_id: InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap(),
            assumed_instrument: policy(3).scope(),
            side,
            assumed_executed_at_utc: DateTime::parse_from_rfc3339("2023-08-28T02:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            executed_price_micro_cny: 10_000_000,
            quantity: 100,
        }
    }

    fn target(
        fill_id: Option<i64>,
        code: &str,
        target_quantity: u64,
        executed_price_micro_cny: i64,
    ) -> ResearchRebalanceTargetV2 {
        ResearchRebalanceTargetV2 {
            instrument_id: InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap(),
            assumed_instrument: policy(3).scope(),
            executed_price_micro_cny,
            target_quantity,
            fill_id,
        }
    }

    fn execution_time() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2023-08-28T02:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn applied_fills_keep_complete_fees_and_stable_effect_and_holding_order() {
        let mut owner = portfolio(3_000_000_000);
        let policy_instance_id = owner.policy().instance_id();
        let first = owner
            .apply_fill(fill(2, "TEST_CODE_Z", FillSide::Buy))
            .unwrap();
        assert_eq!(first.notional_micro_cny, 1_000_000_000);
        assert_eq!(first.fee.policy_instance_id, policy_instance_id);
        assert_eq!(first.fee.commission_micro_cny, 5_000_000);
        assert_eq!(first.fee.stamp_tax_micro_cny, 0);
        owner
            .apply_fill(fill(1, "TEST_CODE_A", FillSide::Buy))
            .unwrap();
        assert_eq!(owner.cash_micro_cny(), 990_000_000);
        assert_eq!(
            owner
                .effects()
                .iter()
                .map(|effect| effect.input.fill_id)
                .collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(
            owner
                .holdings()
                .map(|holding| holding.instrument_id.code())
                .collect::<Vec<_>>(),
            ["TEST_CODE_A", "TEST_CODE_Z"]
        );

        let sold = owner
            .apply_fill(fill(3, "TEST_CODE_Z", FillSide::Sell))
            .unwrap();
        assert_eq!(sold.fee.commission_micro_cny, 5_000_000);
        assert_eq!(sold.fee.stamp_tax_micro_cny, 500_000);
        assert_eq!(sold.poststate.held_quantity, 0);
        assert_eq!(owner.cash_micro_cny(), 1_984_500_000);
        assert_eq!(owner.holdings().count(), 1);
        assert_eq!(owner.effects().len(), 3);
    }

    #[test]
    fn rejected_fill_is_atomic_and_does_not_consume_its_id() {
        let mut owner = portfolio(1_005_000_000);
        let buy = fill(1, "TEST_CODE_A", FillSide::Buy);
        owner.apply_fill(buy.clone()).unwrap();
        let unchanged = owner.clone();

        assert_eq!(
            owner.apply_fill(buy),
            Err(ResearchPortfolioV2Error::DuplicateFillId(1))
        );
        assert_eq!(owner, unchanged);

        let mut second = fill(2, "TEST_CODE_B", FillSide::Buy);
        assert_eq!(
            owner.apply_fill(second.clone()),
            Err(ResearchPortfolioV2Error::Fill(
                ResearchFillV2Error::InsufficientCash {
                    available: 0,
                    required: 1_005_000_000,
                }
            ))
        );
        assert_eq!(owner, unchanged);
        second.side = FillSide::Sell;
        assert_eq!(
            owner.apply_fill(second.clone()),
            Err(ResearchPortfolioV2Error::Fill(
                ResearchFillV2Error::InsufficientPosition {
                    held: 0,
                    requested: 100,
                }
            ))
        );
        assert_eq!(owner, unchanged);

        let sell = fill(3, "TEST_CODE_A", FillSide::Sell);
        owner.apply_fill(sell).unwrap();
        second.side = FillSide::Buy;
        second.executed_price_micro_cny = 9_000_000;
        owner.apply_fill(second).unwrap();
        assert_eq!(
            owner
                .effects()
                .iter()
                .map(|effect| effect.input.fill_id)
                .collect::<Vec<_>>(),
            [1, 3, 2]
        );
    }

    #[test]
    fn unsupported_and_overflow_fills_leave_owner_unchanged() {
        let mut owner = portfolio(i64::MAX);
        let initial = owner.clone();
        let mut input = fill(1, "TEST_CODE_A", FillSide::Buy);

        input.instrument_id =
            InstrumentId::new(Exchange::Shenzhen, "TEST_CODE_A", AssetClass::Equity).unwrap();
        assert_eq!(
            owner.apply_fill(input.clone()),
            Err(ResearchPortfolioV2Error::Fill(
                ResearchFillV2Error::UnsupportedSecurityIdentity
            ))
        );
        assert_eq!(owner, initial);

        input.instrument_id =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_A", AssetClass::Equity).unwrap();
        input.assumed_executed_at_utc = DateTime::parse_from_rfc3339("2008-09-18T02:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            owner.apply_fill(input.clone()),
            Err(ResearchPortfolioV2Error::Fill(ResearchFillV2Error::Fee(
                AShareFeeV2Error::UnsupportedTradeDate
            )))
        );
        assert_eq!(owner, initial);

        input.assumed_executed_at_utc = DateTime::parse_from_rfc3339("2023-08-28T02:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        input.executed_price_micro_cny = i64::MAX;
        assert_eq!(
            owner.apply_fill(input.clone()),
            Err(ResearchPortfolioV2Error::Fill(
                ResearchFillV2Error::Overflow
            ))
        );
        assert_eq!(owner, initial);

        input.executed_price_micro_cny = 1;
        input.quantity = 1;
        owner.apply_fill(input).unwrap();
        let after_buy = owner.clone();
        let mut sell = fill(2, "TEST_CODE_A", FillSide::Sell);
        sell.quantity = 1;
        sell.executed_price_micro_cny = 100_000_000;
        assert_eq!(
            owner.apply_fill(sell),
            Err(ResearchPortfolioV2Error::Fill(
                ResearchFillV2Error::Overflow
            ))
        );
        assert_eq!(owner, after_buy);

        let mut unavailable = ResearchPortfolioV2::new(
            policy(3),
            2_000_000_000,
            FeeCoverageRequirement::CompleteTradingCost,
        )
        .unwrap();
        let unchanged = unavailable.clone();
        assert_eq!(
            unavailable.apply_fill(fill(1, "TEST_CODE_A", FillSide::Buy)),
            Err(ResearchPortfolioV2Error::Fill(ResearchFillV2Error::Fee(
                AShareFeeV2Error::UnsupportedCoverage
            )))
        );
        assert_eq!(unavailable, unchanged);
    }

    #[test]
    fn explicit_policy_changes_retained_fee_and_cash() {
        let mut lower = ResearchPortfolioV2::new(
            policy(3),
            200_000_000_000,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        let mut higher = ResearchPortfolioV2::new(
            policy(4),
            200_000_000_000,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        let mut input = fill(1, "TEST_CODE_A", FillSide::Buy);
        input.executed_price_micro_cny = 1_000_000_000;
        lower.apply_fill(input.clone()).unwrap();
        higher.apply_fill(input).unwrap();
        assert_ne!(
            lower.effects()[0].fee.policy_instance_id,
            higher.effects()[0].fee.policy_instance_id
        );
        assert_eq!(lower.cash_micro_cny() - higher.cash_micro_cny(), 10_000_000);
    }

    #[test]
    fn exact_rebalance_sells_before_buys_in_stable_order() {
        let mut owner = portfolio(2_010_000_000);
        owner
            .apply_fill(fill(1, "TEST_CODE_A", FillSide::Buy))
            .unwrap();
        owner
            .apply_fill(fill(2, "TEST_CODE_B", FillSide::Buy))
            .unwrap();
        assert_eq!(owner.cash_micro_cny(), 0);
        let mut other_order = owner.clone();

        let a = target(Some(3), "TEST_CODE_A", 0, 10_000_000);
        let b = target(Some(4), "TEST_CODE_B", 200, 9_000_000);
        let effects = owner
            .rebalance_exact_targets(execution_time(), vec![b.clone(), a.clone()])
            .unwrap();
        assert_eq!(
            effects
                .iter()
                .map(|effect| (effect.input.fill_id, effect.input.side))
                .collect::<Vec<_>>(),
            [(3, FillSide::Sell), (4, FillSide::Buy)]
        );
        other_order
            .rebalance_exact_targets(execution_time(), vec![a, b])
            .unwrap();
        assert_eq!(owner, other_order);
        assert_eq!(owner.cash_micro_cny(), 89_500_000);
        assert_eq!(
            owner
                .holdings()
                .map(|holding| (holding.instrument_id.code(), holding.quantity))
                .collect::<Vec<_>>(),
            [("TEST_CODE_B", 200)]
        );
    }

    #[test]
    fn failed_rebalance_rolls_back_sells_and_preserves_fill_ids_for_retry() {
        let mut owner = portfolio(1_005_000_000);
        owner
            .apply_fill(fill(1, "TEST_CODE_A", FillSide::Buy))
            .unwrap();
        let original = owner.clone();
        let sell_a = target(Some(3), "TEST_CODE_A", 0, 10_000_000);
        let unaffordable_b = target(Some(4), "TEST_CODE_B", 100, 10_000_000);
        assert_eq!(
            owner.rebalance_exact_targets(execution_time(), vec![unaffordable_b, sell_a.clone()]),
            Err(ResearchPortfolioV2Error::Fill(
                ResearchFillV2Error::InsufficientCash {
                    available: 994_500_000,
                    required: 1_005_000_000,
                }
            ))
        );
        assert_eq!(owner, original);

        let affordable_b = target(Some(4), "TEST_CODE_B", 100, 9_000_000);
        let effects = owner
            .rebalance_exact_targets(execution_time(), vec![affordable_b, sell_a])
            .unwrap();
        assert_eq!(effects.len(), 2);
        assert_eq!(owner.cash_micro_cny(), 89_500_000);
    }

    #[test]
    fn rebalance_requires_complete_targets_and_no_fill_for_unchanged_holding() {
        let mut owner = portfolio(1_005_000_000);
        owner
            .apply_fill(fill(1, "TEST_CODE_A", FillSide::Buy))
            .unwrap();
        let original = owner.clone();
        assert_eq!(
            owner.rebalance_exact_targets(execution_time(), Vec::new()),
            Err(ResearchPortfolioV2Error::MissingHoldingTarget(
                "TEST_CODE_A".into()
            ))
        );
        assert_eq!(
            owner.rebalance_exact_targets(
                execution_time(),
                vec![target(Some(2), "TEST_CODE_A", 100, 10_000_000)]
            ),
            Err(ResearchPortfolioV2Error::UnneededRebalanceFillId(
                "TEST_CODE_A".into()
            ))
        );
        assert_eq!(
            owner.rebalance_exact_targets(
                execution_time(),
                vec![target(None, "TEST_CODE_A", 0, 10_000_000)]
            ),
            Err(ResearchPortfolioV2Error::MissingRebalanceFillId(
                "TEST_CODE_A".into()
            ))
        );
        assert_eq!(owner, original);
        assert!(owner
            .rebalance_exact_targets(
                execution_time(),
                vec![target(None, "TEST_CODE_A", 100, 10_000_000)]
            )
            .unwrap()
            .is_empty());
        assert_eq!(owner, original);
    }
}
