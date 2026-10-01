//! Pure modeled research-fill effect for the explicit A-share fee policy.
//! No exchange fill, portfolio ledger, or source qualification is attested here.

use chrono::{DateTime, Utc};

use crate::market_domain::{AssetClass, Exchange, InstrumentId};
use crate::performance::fee_evidence::{
    a_share_stock_fill_fee_with_policy_v2, shanghai_execution_date, AShareFeePolicyV2,
    AShareFeeV2Error, AShareFillFeeV2, FeeCoverageRequirement, FillSide, QualifiedInstrument,
};

/// A caller-specified research fill. Price is already post-slippage and in
/// micro-CNY; the caller must separately prove instrument classification and
/// source data. Neither the code nor a ticker prefix establishes A-share status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchFillInputV2 {
    pub fill_id: i64,
    pub instrument_id: InstrumentId,
    pub assumed_instrument: QualifiedInstrument,
    pub side: FillSide,
    pub assumed_executed_at_utc: DateTime<Utc>,
    pub executed_price_micro_cny: i64,
    pub quantity: u64,
}

/// Only this security's quantity is represented. A future run owner must
/// supply and bind the rest of the portfolio and check duplicate fill IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResearchFillPrestateV2 {
    pub cash_micro_cny: i64,
    pub held_quantity: u64,
}

/// Complete modeled fee components and the exact effect of one assumed fill.
/// This value is not a durable receipt or an authoritative fill-cost ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedResearchFillV2 {
    pub input: ResearchFillInputV2,
    pub notional_micro_cny: i64,
    pub fee: AShareFillFeeV2,
    pub cash_delta_micro_cny: i64,
    pub poststate: ResearchFillPrestateV2,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResearchFillV2Error {
    #[error("research fill ID must be positive")]
    InvalidFillId,
    #[error("research security ID must be a Shanghai equity")]
    UnsupportedSecurityIdentity,
    #[error("research execution price must be positive micro-CNY")]
    InvalidPrice,
    #[error("research fill quantity must be positive")]
    InvalidQuantity,
    #[error("research prestate cash cannot be negative")]
    InvalidCash,
    #[error("research fill arithmetic overflow")]
    Overflow,
    #[error("research fill has insufficient cash: available={available}, required={required}")]
    InsufficientCash { available: i64, required: i64 },
    #[error("research fill has insufficient position: held={held}, requested={requested}")]
    InsufficientPosition { held: u64, requested: u64 },
    #[error("research fill fee unavailable: {0}")]
    Fee(#[from] AShareFeeV2Error),
}

/// Prepare one modeled effect without mutating the supplied prestate. The
/// executed date is derived from the specified UTC instant in Asia/Shanghai;
/// it is never taken from the signal date or the computer's local time.
pub fn prepare_research_fill_with_policy_v2(
    policy: &AShareFeePolicyV2,
    input: &ResearchFillInputV2,
    prestate: ResearchFillPrestateV2,
    required_coverage: FeeCoverageRequirement,
) -> Result<PreparedResearchFillV2, ResearchFillV2Error> {
    if input.fill_id <= 0 {
        return Err(ResearchFillV2Error::InvalidFillId);
    }
    if input.instrument_id.exchange() != Exchange::Shanghai
        || input.instrument_id.asset_class() != AssetClass::Equity
    {
        return Err(ResearchFillV2Error::UnsupportedSecurityIdentity);
    }
    if input.executed_price_micro_cny <= 0 {
        return Err(ResearchFillV2Error::InvalidPrice);
    }
    if input.quantity == 0 {
        return Err(ResearchFillV2Error::InvalidQuantity);
    }
    if prestate.cash_micro_cny < 0 {
        return Err(ResearchFillV2Error::InvalidCash);
    }
    let notional_micro_cny = i128::from(input.executed_price_micro_cny)
        .checked_mul(i128::from(input.quantity))
        .and_then(|amount| i64::try_from(amount).ok())
        .ok_or(ResearchFillV2Error::Overflow)?;
    let fee = a_share_stock_fill_fee_with_policy_v2(
        policy,
        input.assumed_instrument,
        input.side,
        notional_micro_cny,
        shanghai_execution_date(input.assumed_executed_at_utc),
        required_coverage,
    )?;
    let (cash_delta_micro_cny, held_quantity) = match input.side {
        FillSide::Buy => {
            let required = notional_micro_cny
                .checked_add(fee.total_micro_cny)
                .ok_or(ResearchFillV2Error::Overflow)?;
            if prestate.cash_micro_cny < required {
                return Err(ResearchFillV2Error::InsufficientCash {
                    available: prestate.cash_micro_cny,
                    required,
                });
            }
            let held_quantity = prestate
                .held_quantity
                .checked_add(input.quantity)
                .ok_or(ResearchFillV2Error::Overflow)?;
            (-required, held_quantity)
        }
        FillSide::Sell => {
            if prestate.held_quantity < input.quantity {
                return Err(ResearchFillV2Error::InsufficientPosition {
                    held: prestate.held_quantity,
                    requested: input.quantity,
                });
            }
            let cash_delta = notional_micro_cny
                .checked_sub(fee.total_micro_cny)
                .ok_or(ResearchFillV2Error::Overflow)?;
            (cash_delta, prestate.held_quantity - input.quantity)
        }
    };
    let cash_micro_cny = prestate
        .cash_micro_cny
        .checked_add(cash_delta_micro_cny)
        .ok_or(ResearchFillV2Error::Overflow)?;
    if cash_micro_cny < 0 {
        return Err(ResearchFillV2Error::InsufficientCash {
            available: prestate.cash_micro_cny,
            required: -cash_delta_micro_cny,
        });
    }
    Ok(PreparedResearchFillV2 {
        input: input.clone(),
        notional_micro_cny,
        fee,
        cash_delta_micro_cny,
        poststate: ResearchFillPrestateV2 {
            cash_micro_cny,
            held_quantity,
        },
    })
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::performance::fee_evidence::{
        FeeCoverage, FeeListingSegment, FeeMarket, FeeRate, FeeSecurityKind, StampTaxBracketV2,
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
            "TEST_CODE_m4_research_fill_v2",
        )
        .unwrap()
    }

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fill(side: FillSide, when: &str) -> ResearchFillInputV2 {
        ResearchFillInputV2 {
            fill_id: 1,
            instrument_id: InstrumentId::new(
                Exchange::Shanghai,
                "TEST_CODE_600000",
                AssetClass::Equity,
            )
            .unwrap(),
            assumed_instrument: policy(3).scope(),
            side,
            assumed_executed_at_utc: at(when),
            executed_price_micro_cny: 10_000_000,
            quantity: 100,
        }
    }

    #[test]
    fn flat_price_round_trip_uses_full_modeled_fees_and_conserves_cash() {
        let policy = policy(3);
        let starting = ResearchFillPrestateV2 {
            cash_micro_cny: 2_000_000_000,
            held_quantity: 0,
        };
        let buy = fill(FillSide::Buy, "2023-08-28T02:00:00Z");
        let bought = prepare_research_fill_with_policy_v2(
            &policy,
            &buy,
            starting,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        assert_eq!(bought.notional_micro_cny, 1_000_000_000);
        assert_eq!(bought.fee.policy_instance_id, policy.instance_id());
        assert_eq!(bought.fee.commission_micro_cny, 5_000_000);
        assert_eq!(bought.fee.stamp_tax_micro_cny, 0);
        assert_eq!(bought.cash_delta_micro_cny, -1_005_000_000);
        assert_eq!(bought.poststate.held_quantity, 100);

        let mut sell = fill(FillSide::Sell, "2023-08-29T02:00:00Z");
        sell.fill_id = 2;
        let sold = prepare_research_fill_with_policy_v2(
            &policy,
            &sell,
            bought.poststate,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        assert_eq!(sold.fee.commission_micro_cny, 5_000_000);
        assert_eq!(sold.fee.stamp_tax_micro_cny, 500_000);
        assert_eq!(sold.cash_delta_micro_cny, 994_500_000);
        assert_eq!(sold.poststate.held_quantity, 0);
        assert_eq!(
            starting.cash_micro_cny - sold.poststate.cash_micro_cny,
            10_500_000
        );
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &sell,
                bought.poststate,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Ok(sold)
        );
    }

    #[test]
    fn modeled_execution_instant_selects_shanghai_tax_bracket() {
        let policy = policy(3);
        let state = ResearchFillPrestateV2 {
            cash_micro_cny: 10_000_000,
            held_quantity: 100,
        };
        let before = prepare_research_fill_with_policy_v2(
            &policy,
            &fill(FillSide::Sell, "2023-08-27T15:59:00Z"),
            state,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        let after = prepare_research_fill_with_policy_v2(
            &policy,
            &fill(FillSide::Sell, "2023-08-27T16:01:00Z"),
            state,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        assert_eq!(before.fee.trade_date.to_string(), "2023-08-27");
        assert_eq!(before.fee.stamp_tax_micro_cny, 1_000_000);
        assert_eq!(
            before.fee.stamp_tax_bracket,
            StampTaxBracketV2::SellerOnePerThousand
        );
        assert_eq!(after.fee.trade_date.to_string(), "2023-08-28");
        assert_eq!(after.fee.stamp_tax_micro_cny, 500_000);
        assert_eq!(
            after.fee.stamp_tax_bracket,
            StampTaxBracketV2::SellerHalfPerThousand
        );
        // A Sunday signal is not an input: the assumed Monday execution sets the fee date.
        let monday = fill(FillSide::Sell, "2023-08-28T02:00:00Z");
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &monday,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            )
            .unwrap()
            .fee
            .stamp_tax_micro_cny,
            500_000
        );
    }

    #[test]
    fn policy_change_changes_fee_identity_and_cash_effect() {
        let state = ResearchFillPrestateV2 {
            cash_micro_cny: 200_000_000_000,
            held_quantity: 100,
        };
        let mut input = fill(FillSide::Sell, "2026-09-28T02:00:00Z");
        input.executed_price_micro_cny = 1_000_000_000;
        let lower = prepare_research_fill_with_policy_v2(
            &policy(3),
            &input,
            state,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        let higher = prepare_research_fill_with_policy_v2(
            &policy(4),
            &input,
            state,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        assert_eq!(lower.fee.commission_micro_cny, 30_000_000);
        assert_eq!(higher.fee.commission_micro_cny, 40_000_000);
        assert_ne!(lower.fee.policy_instance_id, higher.fee.policy_instance_id);
        assert_eq!(
            lower.cash_delta_micro_cny - higher.cash_delta_micro_cny,
            10_000_000
        );
    }

    #[test]
    fn invalid_scope_date_and_coverage_are_typed_fee_failures() {
        let policy = policy(3);
        let state = ResearchFillPrestateV2 {
            cash_micro_cny: 2_000_000_000,
            held_quantity: 100,
        };
        let input = fill(FillSide::Sell, "2008-09-18T02:00:00Z");
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &input,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::Fee(
                AShareFeeV2Error::UnsupportedTradeDate
            ))
        );
        let valid = fill(FillSide::Sell, "2026-09-28T02:00:00Z");
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &valid,
                state,
                FeeCoverageRequirement::CompleteTradingCost
            ),
            Err(ResearchFillV2Error::Fee(
                AShareFeeV2Error::UnsupportedCoverage
            ))
        );
        let mut star = valid.clone();
        star.assumed_instrument = QualifiedInstrument::new(
            FeeMarket::Shanghai,
            FeeSecurityKind::AShareStock,
            FeeListingSegment::ShanghaiStarA,
        )
        .unwrap();
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &star,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::Fee(AShareFeeV2Error::ScopeMismatch))
        );
        let mut shenzhen = valid.clone();
        shenzhen.instrument_id =
            InstrumentId::new(Exchange::Shenzhen, "TEST_CODE_000001", AssetClass::Equity).unwrap();
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &shenzhen,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::UnsupportedSecurityIdentity)
        );
        let mut fund = valid;
        fund.instrument_id =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_FUND", AssetClass::Fund).unwrap();
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &fund,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::UnsupportedSecurityIdentity)
        );
    }

    #[test]
    fn invalid_amount_balance_and_position_fail_without_mutation() {
        let policy = policy(3);
        let state = ResearchFillPrestateV2 {
            cash_micro_cny: 1_000_000_000,
            held_quantity: 100,
        };
        let input = fill(FillSide::Buy, "2026-09-28T02:00:00Z");
        let original = input.clone();
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &input,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::InsufficientCash {
                available: 1_000_000_000,
                required: 1_005_000_000,
            })
        );
        assert_eq!(input, original);
        assert_eq!(state.cash_micro_cny, 1_000_000_000);

        let mut bad = input.clone();
        bad.fill_id = 0;
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &bad,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::InvalidFillId)
        );
        bad = input.clone();
        bad.executed_price_micro_cny = 0;
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &bad,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::InvalidPrice)
        );
        bad.executed_price_micro_cny = -1;
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &bad,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::InvalidPrice)
        );
        bad = input.clone();
        bad.quantity = 0;
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &bad,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::InvalidQuantity)
        );
        bad = input.clone();
        bad.quantity = u64::MAX;
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &bad,
                state,
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::Overflow)
        );
        bad = input.clone();
        bad.side = FillSide::Sell;
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &bad,
                ResearchFillPrestateV2 {
                    held_quantity: 0,
                    ..state
                },
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::InsufficientPosition {
                held: 0,
                requested: 100,
            })
        );
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &input,
                ResearchFillPrestateV2 {
                    cash_micro_cny: -1,
                    ..state
                },
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::InvalidCash)
        );
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &input,
                ResearchFillPrestateV2 {
                    cash_micro_cny: i64::MAX,
                    held_quantity: u64::MAX
                },
                FeeCoverageRequirement::ModeledComponentsOnly
            ),
            Err(ResearchFillV2Error::Overflow)
        );
        let mut too_large_to_charge = input.clone();
        too_large_to_charge.executed_price_micro_cny = i64::MAX;
        too_large_to_charge.quantity = 1;
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &too_large_to_charge,
                ResearchFillPrestateV2 {
                    cash_micro_cny: i64::MAX,
                    held_quantity: 0,
                },
                FeeCoverageRequirement::ModeledComponentsOnly,
            ),
            Err(ResearchFillV2Error::Overflow)
        );
        let mut tiny_sell = input.clone();
        tiny_sell.side = FillSide::Sell;
        tiny_sell.executed_price_micro_cny = 1;
        tiny_sell.quantity = 1;
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &tiny_sell,
                ResearchFillPrestateV2 {
                    cash_micro_cny: 0,
                    held_quantity: 1,
                },
                FeeCoverageRequirement::ModeledComponentsOnly,
            ),
            Err(ResearchFillV2Error::InsufficientCash {
                available: 0,
                required: 4_999_999,
            })
        );
        let mut ordinary_sell = input;
        ordinary_sell.side = FillSide::Sell;
        assert_eq!(
            prepare_research_fill_with_policy_v2(
                &policy,
                &ordinary_sell,
                ResearchFillPrestateV2 {
                    cash_micro_cny: i64::MAX,
                    held_quantity: 100,
                },
                FeeCoverageRequirement::ModeledComponentsOnly,
            ),
            Err(ResearchFillV2Error::Overflow)
        );
    }
}
