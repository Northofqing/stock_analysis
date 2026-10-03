//! Integer accounting rules, not an approval factory. These closed persisted
//! records are observations; only the private issuer may approve their use.

use crate::trading::paper_replay_financial_work_v1::{self as fw, FinancialWork, FinancialFailure, ClosedFinancialText as Txt};
use super::paper_ledger::{LedgerError, Projection};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const POLICY_VERSION: &str = "paper-parent-budget/v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ProfitPolicy {
    ReinvestWithinFixedAuthorizedBudget,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum LotDisposition {
    AllocatedToStrategy,
    UnassignedReadOnly,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InitialLotAllocation {
    pub(crate) lot_id: String,
    pub(crate) original_quantity: u32,
    pub(crate) disposition: LotDisposition,
    pub(crate) chain_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BudgetRecord {
    pub(crate) version: String,
    pub(crate) family_id: String,
    pub(crate) effective_from: NaiveDate,
    pub(crate) effective_through: NaiveDate,
    pub(crate) authorized_budget_micro_cny: i64,
    pub(crate) initial_strategy_cash_micro_cny: i64,
    pub(crate) concentration_bps: u32,
    pub(crate) chain_exposure_bps: u32,
    pub(crate) cash_floor_bps: u32,
    pub(crate) max_order_exposure_micro_cny: i64,
    pub(crate) original_seed_reference: String,
    pub(crate) review_reference: String,
    pub(crate) profit_policy: ProfitPolicy,
    pub(crate) initial_lots: Vec<InitialLotAllocation>,
}

fn invalid(reason: &str) -> LedgerError {
    LedgerError::InvalidInput(format!("parent budget {reason}"))
}

pub(crate) fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

pub(crate) fn checked(value: i128) -> Result<i64, LedgerError> {
    i64::try_from(value).map_err(|_| LedgerError::Overflow)
}

fn add(total: &mut i128, value: i128) -> Result<(), LedgerError> {
    *total = total.checked_add(value).ok_or(LedgerError::Overflow)?;
    Ok(())
}

pub(crate) fn notional(price: i64, quantity: u32) -> Result<i64, LedgerError> {
    fw::historical(notional_with_work(price, quantity, &mut FinancialWork::Historical))
}
pub(crate) fn notional_with_work(price: i64, quantity: u32, w: &mut FinancialWork<'_, '_>) -> fw::Result<i64> {
    w.finish()?;
    if price <= 0 || quantity == 0 {
        return Err(w.error(Txt::Budget(fw::BudgetText::NonpositivePriceOrQuantity))?);
    }
    Ok(checked(i128::from(price) * i128::from(quantity))?)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CashPartitions {
    pub(crate) account_cash: i64,
    pub(crate) strategy_cash: i64,
    pub(crate) unassigned_cash: i64,
}

impl CashPartitions {
    pub(crate) fn validate(&self) -> Result<(), LedgerError> {
        fw::historical(self.validate_with_work(&mut FinancialWork::Historical))
    }
    pub(crate) fn validate_with_work(&self, w: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
        w.finish()?;
        if self.account_cash < 0 || self.strategy_cash < 0 || self.unassigned_cash < 0 || checked(i128::from(self.strategy_cash) + i128::from(self.unassigned_cash))? != self.account_cash {
            return Err(w.error(Txt::Budget(fw::BudgetText::ExecutionCashPartitionsDiffer))?);
        }
        Ok(())
    }

    /// A single economic delta updates both views. Unassigned cash is never
    /// borrowed, released into this family, or used to cover a strategy loss.
    pub(crate) fn apply_strategy_delta(&mut self, delta: i64) -> Result<(), LedgerError> {
        fw::historical(self.apply_strategy_delta_with_work(delta, &mut FinancialWork::Historical))
    }
    pub(crate) fn apply_strategy_delta_with_work(&mut self, delta: i64, w: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
        w.finish()?;
        self.validate_with_work(w)?;
        let next = Self {
            account_cash: checked(i128::from(self.account_cash) + i128::from(delta))?,
            strategy_cash: checked(i128::from(self.strategy_cash) + i128::from(delta))?,
            unassigned_cash: self.unassigned_cash,
        };
        next.validate_with_work(w)?;
        *self = next;
        Ok(())
    }
}

impl BudgetRecord {
    pub(crate) fn validate_shape(&self) -> Result<(), LedgerError> {
        fw::historical(self.validate_shape_with_work(&mut FinancialWork::Historical))
    }
    pub(crate) fn validate_shape_with_work(&self, w: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
        w.finish()?;
        if self.version != POLICY_VERSION || !token(&self.family_id) || self.effective_from > self.effective_through || !token(&self.original_seed_reference) || !token(&self.review_reference) || self.authorized_budget_micro_cny <= 0 || self.initial_strategy_cash_micro_cny < 0 || !(1..=10_000).contains(&self.concentration_bps) || !(1..=10_000).contains(&self.chain_exposure_bps) || self.cash_floor_bps > 10_000 || self.max_order_exposure_micro_cny <= 0 || self.max_order_exposure_micro_cny > self.authorized_budget_micro_cny {
            return Err(w.error(Txt::Budget(fw::BudgetText::DescriptorIsInvalid))?);
        }
        let mut previous: Option<&str> = None;
        for allocation in &self.initial_lots {
            if !token(&allocation.lot_id) || allocation.original_quantity == 0 || previous.is_some_and(|p| p >= allocation.lot_id.as_str()) || match allocation.disposition {
                LotDisposition::AllocatedToStrategy => {
                    !allocation.chain_id.as_deref().is_some_and(token)
                }
                LotDisposition::UnassignedReadOnly => allocation.chain_id.is_some(),
            }
            {
                return Err(w.error(Txt::Budget(fw::BudgetText::CompleteOrderedLotAllocationIsInvalid))?);
            }
            previous = Some(&allocation.lot_id);
        }
        Ok(())
    }

    /// Caller supplies only the original, fully verified V2 genesis snapshot.
    /// This pure comparison does not attest a database or approve a manifest.
    pub(crate) fn initial_cash(&self, genesis: &Projection) -> Result<CashPartitions, LedgerError> {
        fw::historical(self.initial_cash_with_work(genesis, &mut FinancialWork::Historical))
    }
    pub(crate) fn initial_cash_with_work(&self, genesis: &Projection, w: &mut FinancialWork<'_, '_>) -> fw::Result<CashPartitions> {
        w.finish()?;
        self.validate_shape_with_work(w)?;
        if genesis.cash.micros() < self.initial_strategy_cash_micro_cny || self.initial_lots.len() != genesis.lots.len() {
            return Err(w.error(Txt::Budget(fw::BudgetText::AllocationDoesNotMatchGenesis))?);
        }
        let mut original = BTreeMap::new();
        for lot in &genesis.lots {
            w.lot_ref(&mut original, lot.lot_id.as_str(), lot)?;
        }
        if original.len() != genesis.lots.len() {
            return Err(w.error(Txt::Budget(fw::BudgetText::DuplicateGenesisLot))?);
        }
        let mut c0 = i128::from(self.initial_strategy_cash_micro_cny);
        for allocation in &self.initial_lots {
            let lot = w.option(original.get(allocation.lot_id.as_str()), Txt::Budget(fw::BudgetText::UnknownGenesisLot))?;
            if lot.quantity != allocation.original_quantity {
                return Err(w.error(Txt::Budget(fw::BudgetText::GenesisQuantityChanged))?);
            }
            if allocation.disposition == LotDisposition::AllocatedToStrategy {
                let mark = w.option(genesis.marks.get(&lot.code), Txt::Budget(fw::BudgetText::GenesisMarkAbsent))?;
                c0 = c0 .checked_add(i128::from(notional_with_work(mark.price.micros(), lot.quantity, w)?)) .ok_or(LedgerError::Overflow)?;
            }
        }
        if c0 < 0 || checked(c0)? > self.authorized_budget_micro_cny {
            return Err(w.error(Txt::Budget(fw::BudgetText::InitialAllocatedCapitalExceedsFixedBudget))?);
        }
        let partitions = CashPartitions {
            account_cash: genesis.cash.micros(),
            strategy_cash: self.initial_strategy_cash_micro_cny,
            unassigned_cash: checked( i128::from(genesis.cash.micros()) - i128::from(self.initial_strategy_cash_micro_cny), )?,
        };
        partitions.validate_with_work(w)?;
        Ok(partitions)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkingReservation {
    pub(crate) parent_id: String,
    pub(crate) code: String,
    pub(crate) chain_id: String,
    pub(crate) buy_max_notional: i64,
    pub(crate) fee_reserve: i64,
    pub(crate) cash_reserve: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MarkedAllocation {
    pub(crate) code: String,
    pub(crate) chain_id: String,
    pub(crate) marked_value: i64,
}

/// Checks a new buy only. Actual valuation, recovery, sell, cancellation and
/// existing reservations remain facts when market appreciation exceeds B.
pub(crate) fn require_new_buy( policy: &BudgetRecord, cash: &CashPartitions, marked: &[MarkedAllocation], working: &[WorkingReservation], new: &WorkingReservation, ) -> Result<(), LedgerError> {
    fw::historical(require_new_buy_with_work(policy, cash, marked, working, new, &mut FinancialWork::Historical))
}
pub(crate) fn require_new_buy_with_work( policy: &BudgetRecord, cash: &CashPartitions, marked: &[MarkedAllocation], working: &[WorkingReservation], new: &WorkingReservation, w: &mut FinancialWork<'_, '_>) -> fw::Result<()> {
    w.finish()?;
    policy.validate_shape_with_work(w)?;
    cash.validate_with_work(w)?;
    let mut aggregate = 0_i128;
    let mut per_code: BTreeMap<&str,
    i128> = BTreeMap::new();
    let mut per_chain: BTreeMap<&str,
    i128> = BTreeMap::new();
    let mut reserved_cash = 0_i128;
    let mut parents = BTreeSet::new();
    for holding in marked {
        if holding.marked_value < 0 || !token(&holding.code) || !token(&holding.chain_id) {
            return Err(w.error(Txt::Budget(fw::BudgetText::MarkedAllocationIsInvalid))?);
        }
        add(&mut aggregate, i128::from(holding.marked_value))?;
        add( w.exposure(&mut per_code, &holding.code)?, i128::from(holding.marked_value), )?;
        add( w.exposure(&mut per_chain, &holding.chain_id)?, i128::from(holding.marked_value), )?;
    }
    for reservation in working.iter().chain(std::iter::once(new)) {
        if reservation.buy_max_notional < 0 || reservation.fee_reserve < 0 || reservation.cash_reserve < 0 || checked( i128::from(reservation.buy_max_notional) + i128::from(reservation.fee_reserve), )? != reservation.cash_reserve || !token(&reservation.parent_id) || !token(&reservation.code) || !token(&reservation.chain_id) || !w.set(&mut parents, reservation.parent_id.as_str())? {
            return Err(w.error(Txt::Budget(fw::BudgetText::WorkingReservationIsInvalid))?);
        }
        let all_in = i128::from(reservation.buy_max_notional) + i128::from(reservation.fee_reserve);
        add(&mut aggregate, all_in)?;
        add(w.exposure(&mut per_code, &reservation.code)?, all_in)?;
        add(w.exposure(&mut per_chain, &reservation.chain_id)?, all_in)?;
        add(&mut reserved_cash, i128::from(reservation.cash_reserve))?;
    }
    let b = i128::from(policy.authorized_budget_micro_cny);
    let single = per_code.get(new.code.as_str()).copied().unwrap_or_default();
    let chain = per_chain .get(new.chain_id.as_str()) .copied() .unwrap_or_default();
    let floor = (b * i128::from(policy.cash_floor_bps) + 9_999) / 10_000;
    if aggregate > b || new.buy_max_notional <= 0 || i128::from(new.buy_max_notional) + i128::from(new.fee_reserve) > i128::from(policy.max_order_exposure_micro_cny) || single.checked_mul(10_000).ok_or(LedgerError::Overflow)? > b * i128::from(policy.concentration_bps) || chain.checked_mul(10_000).ok_or(LedgerError::Overflow)? > b * i128::from(policy.chain_exposure_bps) || i128::from(cash.strategy_cash) - reserved_cash < floor {
        return Err(w.error(Txt::Budget(fw::BudgetText::NewBuyExceedsFixedAllInOrCashLimits))?);
    }
    // Validate the representability of every aggregate before returning it as
    // a checked integer accounting proof; no f64 conversion is involved.
    checked(aggregate)?;
    checked(reserved_cash)?;
    Ok(())
}
