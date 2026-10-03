//! Deterministic modeled fills. Recorded inputs can be replayed, but cannot
//! reconstruct a source-issued live execution window or an approved intent.

use super::paper_book_v2_budget_v1::{checked, notional, token};
use super::paper_ledger::LedgerError;
use crate::performance::fee_evidence::FillSide;
use crate::performance::fee_policy::{
    a_share_stock_fill_fee_with_policy_v2, AShareFeePolicyV2, FeeCoverageRequirement,
    StampTaxBracketV2,
};
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use serde::{Deserialize, Serialize};

pub(crate) const MODEL_VERSION: &str = "paper-parent-whole-lot-fill/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum Side {
    Buy,
    Sell,
}
impl Side {
    pub(crate) fn text(self) -> &'static str {
        match self {
            Self::Buy => "Buy",
            Self::Sell => "Sell",
        }
    }
    fn fee_side(self) -> FillSide {
        match self {
            Self::Buy => FillSide::Buy,
            Self::Sell => FillSide::Sell,
        }
    }
}

/// A persisted observation, not live source authority. Issuers retain the
/// original admitted facts separately; strict readers expose only this DTO.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowRecord {
    pub(crate) version: String,
    pub(crate) observation_id: String,
    pub(crate) instrument_code: String,
    pub(crate) session_date: NaiveDate,
    pub(crate) source_at: DateTime<Utc>,
    pub(crate) observed_at: DateTime<Utc>,
    pub(crate) fresh_through: DateTime<Utc>,
    pub(crate) source_reference: String,
    pub(crate) facts_contract: String,
    pub(crate) facts_batch_id: String,
    pub(crate) facts_source: String,
    pub(crate) facts_source_at: String,
    pub(crate) facts_observed_at: String,
    pub(crate) fee_segment: String,
    pub(crate) listed: bool,
    pub(crate) suspended: bool,
    pub(crate) tick_micro_cny: i64,
    pub(crate) lower_micro_cny: i64,
    pub(crate) upper_micro_cny: i64,
    pub(crate) regime_version: String,
    pub(crate) price_micro_cny: i64,
    pub(crate) modeled_available_quantity: u32,
}
impl WindowRecord {
    pub(crate) fn validate(&self) -> Result<(), LedgerError> {
        let local = self
            .observed_at
            .checked_add_signed(chrono::Duration::hours(8))
            .ok_or_else(|| {
                LedgerError::EvidenceUnavailable(
                    "execution window Shanghai clock exceeds supported range".into(),
                )
            })?;
        let day = local.date_naive();
        let seconds = local.time().num_seconds_from_midnight();
        let session = (9 * 3600 + 30 * 60..=11 * 3600 + 30 * 60).contains(&seconds)
            || (13 * 3600..=15 * 3600).contains(&seconds);
        if self.version != MODEL_VERSION
            || !token(&self.observation_id)
            || !token(&self.instrument_code)
            || !token(&self.source_reference)
            || !token(&self.facts_contract)
            || !token(&self.facts_batch_id)
            || !token(&self.facts_source)
            || !token(&self.regime_version)
            || !matches!(self.fee_segment.as_str(), "ShanghaiMainA" | "ShanghaiStarA")
            || chrono::DateTime::parse_from_rfc3339(&self.facts_source_at)
                .ok()
                .zip(chrono::DateTime::parse_from_rfc3339(&self.facts_observed_at).ok())
                .is_none_or(|(a, b)| a > b || b.with_timezone(&Utc) > self.observed_at)
            || self.source_at > self.observed_at
            || self.observed_at > self.fresh_through
            || day != self.session_date
            || !session
            || !crate::calendar::verified_a_share_trading_day(day)
                .map_err(LedgerError::EvidenceUnavailable)?
            || !self.listed
            || self.tick_micro_cny <= 0
            || self.lower_micro_cny <= 0
            || self.upper_micro_cny < self.lower_micro_cny
            || self.lower_micro_cny % self.tick_micro_cny != 0
            || self.upper_micro_cny % self.tick_micro_cny != 0
            || self.price_micro_cny < self.lower_micro_cny
            || self.price_micro_cny > self.upper_micro_cny
            || self.price_micro_cny % self.tick_micro_cny != 0
        {
            return Err(LedgerError::EvidenceUnavailable(
                "execution window is not admissible".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum NoFillReason {
    Suspended,
    OutsideLimit,
    LessThanWholeLot,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModeledFill {
    pub(crate) quantity: u32,
    pub(crate) price_micro_cny: i64,
    pub(crate) notional_micro_cny: i64,
    pub(crate) commission_micro_cny: i64,
    pub(crate) stamp_tax_micro_cny: i64,
    pub(crate) total_fee_micro_cny: i64,
    pub(crate) fee_policy_instance_id: String,
    pub(crate) stamp_tax_bracket: String,
    pub(crate) sellable_from: NaiveDate,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ModelOutcome {
    NoFill(NoFillReason),
    Fill(ModeledFill),
}

pub(crate) fn model(
    side: Side,
    remaining: u32,
    limit: i64,
    fee_price_cap: i64,
    window: &WindowRecord,
    fee_policy: &AShareFeePolicyV2,
) -> Result<ModelOutcome, LedgerError> {
    window.validate()?;
    if remaining == 0
        || remaining % 100 != 0
        || limit <= 0
        || fee_price_cap < limit
        || fee_price_cap > window.upper_micro_cny
        || limit < window.lower_micro_cny
        || limit % window.tick_micro_cny != 0
        || fee_price_cap % window.tick_micro_cny != 0
    {
        return Err(LedgerError::InvalidInput(
            "parent whole-lot bounds invalid".into(),
        ));
    }
    if window.suspended {
        return Ok(ModelOutcome::NoFill(NoFillReason::Suspended));
    }
    if (side == Side::Buy && window.price_micro_cny > limit)
        || (side == Side::Sell && window.price_micro_cny < limit)
    {
        return Ok(ModelOutcome::NoFill(NoFillReason::OutsideLimit));
    }
    if window.price_micro_cny > fee_price_cap {
        return Err(LedgerError::EvidenceUnavailable(
            "execution price exceeds frozen fee cap".into(),
        ));
    }
    let quantity = remaining.min(window.modeled_available_quantity / 100 * 100);
    if quantity == 0 {
        return Ok(ModelOutcome::NoFill(NoFillReason::LessThanWholeLot));
    }
    let value = notional(window.price_micro_cny, quantity)?;
    let fee = a_share_stock_fill_fee_with_policy_v2(
        fee_policy,
        fee_policy.scope(),
        side.fee_side(),
        value,
        window.session_date,
        FeeCoverageRequirement::ModeledComponentsOnly,
    )
    .map_err(|e| LedgerError::EvidenceUnavailable(e.to_string()))?;
    let sellable_from = crate::calendar::verified_next_a_share_trading_day(window.session_date)
        .map_err(LedgerError::EvidenceUnavailable)?;
    Ok(ModelOutcome::Fill(ModeledFill {
        quantity,
        price_micro_cny: window.price_micro_cny,
        notional_micro_cny: value,
        commission_micro_cny: fee.commission_micro_cny,
        stamp_tax_micro_cny: fee.stamp_tax_micro_cny,
        total_fee_micro_cny: fee.total_micro_cny,
        fee_policy_instance_id: fee.policy_instance_id,
        stamp_tax_bracket: fee.stamp_tax_bracket.id().into(),
        sellable_from,
    }))
}

/// Bound every partition into whole lots, including component rounding.
/// For k >= 1, half_up(k*x) <= ceil(k*x) <= k*ceil(x); the per-fill
/// commission minimum is also <= k times the original one-lot commission.
/// Thus n times the one-lot component ceilings covers every partition and
/// every price <= max_price. Actual fills still use the unchanged fee model.
pub(crate) fn worst_case_fee(
    side: Side,
    remaining: u32,
    max_price: i64,
    day: NaiveDate,
    policy: &AShareFeePolicyV2,
) -> Result<i64, LedgerError> {
    if remaining % 100 != 0 {
        return Err(LedgerError::EvidenceUnavailable(
            "odd lot model unavailable".into(),
        ));
    }
    if remaining == 0 {
        return Ok(0);
    }
    let fee = a_share_stock_fill_fee_with_policy_v2(
        policy,
        policy.scope(),
        side.fee_side(),
        notional(max_price, 100)?,
        day,
        FeeCoverageRequirement::ModeledComponentsOnly,
    )
    .map_err(|e| LedgerError::EvidenceUnavailable(e.to_string()))?;
    // This reads only the immutable canonical descriptor of the actual
    // policy value; it neither reconstructs a policy nor issues authority.
    let descriptor = policy.canonical_bytes();
    let text = std::str::from_utf8(&descriptor)
        .map_err(|_| LedgerError::EvidenceUnavailable("fee descriptor UTF8 unavailable".into()))?;
    let integer = |key: &str| -> Result<i128, LedgerError> {
        let prefix = format!("{key}=");
        let mut values = text.lines().filter_map(|line| line.strip_prefix(&prefix));
        let value = values
            .next()
            .and_then(|v| v.parse::<i128>().ok())
            .ok_or_else(|| {
                LedgerError::EvidenceUnavailable("fee descriptor rate unavailable".into())
            })?;
        if values.next().is_some() {
            return Err(LedgerError::EvidenceUnavailable(
                "fee descriptor rate duplicated".into(),
            ));
        }
        Ok(value)
    };
    let numerator = integer("commission_rate_num")?;
    let denominator = integer("commission_rate_den")?;
    if numerator < 0 || denominator <= 0 {
        return Err(LedgerError::EvidenceUnavailable(
            "fee descriptor rate invalid".into(),
        ));
    }
    let ceiling = |numerator: i128, denominator: i128| -> Result<i128, LedgerError> {
        let quotient = numerator / denominator;
        if numerator % denominator == 0 {
            Ok(quotient)
        } else {
            quotient.checked_add(1).ok_or(LedgerError::Overflow)
        }
    };
    let value = i128::from(fee.notional_micro_cny);
    let commission = ceiling(
        value.checked_mul(numerator).ok_or(LedgerError::Overflow)?,
        denominator,
    )?
    .max(i128::from(fee.commission_micro_cny));
    let stamp = if side == Side::Buy {
        0
    } else {
        let denominator = match fee.stamp_tax_bracket {
            StampTaxBracketV2::SellerOnePerThousand => 1_000,
            StampTaxBracketV2::SellerHalfPerThousand => 2_000,
        };
        ceiling(value, denominator)?
    };
    let per_lot = commission.checked_add(stamp).ok_or(LedgerError::Overflow)?;
    checked(
        per_lot
            .checked_mul(i128::from(remaining / 100))
            .ok_or(LedgerError::Overflow)?,
    )
}
