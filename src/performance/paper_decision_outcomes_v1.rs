//! Read-only decision/order/fill lineage from the existing modeled execution
//! owner. References do not attest a positive investment decision or strategy
//! version. No live approval, new ledger, or persisted result is created here.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::trading::paper_book_v2_execution::{
    self as execution, ParentStatus, RecordedExecutionView,
};
use crate::trading::paper_book_v2_fill_model::Side;
use crate::trading::paper_ledger::LedgerError;

const MAX_PARENTS: usize = 1_024;
const MAX_FILLS: usize = 4_096;
const MAX_TEXT: usize = 256;
const MAX_REPORT_ALLOCATION: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub enum RecordedDecisionLinkageV1 {
    ReferenceOnly,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub enum StrategyVersionEvidenceV1 {
    NotRecorded,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PaperDecisionOrderOutcomeV1 {
    pub parent_id: String,
    pub investment_decision_reference: String,
    pub family_id: String,
    pub chain_id: String,
    pub instrument_code: String,
    pub side: String,
    pub status: String,
    pub requested_quantity: u32,
    pub filled_quantity: u32,
    pub remaining_quantity: u32,
    pub cancelled_quantity: u32,
    pub fill_count: usize,
    pub filled_notional_micro_cny: i64,
    pub modeled_fee_micro_cny: i64,
    pub inherited_buy_fee_micro_cny: i64,
    /// Existing modeled execution result; buys are not closed strategy cycles.
    pub realized_pnl_micro_cny: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PaperDecisionFillLineageV1 {
    pub fill_id: String,
    pub parent_id: String,
    pub observation_id: String,
    pub executed_at: DateTime<Utc>,
    pub quantity: u32,
    pub price_micro_cny: i64,
    pub notional_micro_cny: i64,
    pub modeled_fee_micro_cny: i64,
    pub inherited_buy_fee_micro_cny: i64,
    pub realized_pnl_micro_cny: i64,
}

/// An ordinary report. Serialization preserves the explicit reference-only
/// boundary; this type has no conversion to trading or promotion authority.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RecordedPaperDecisionOutcomesV1 {
    pub schema: &'static str,
    pub account_id: String,
    pub epoch_id: String,
    pub execution_manifest_hash: String,
    pub observed_head_version: i64,
    pub observed_head_hash: String,
    pub observed_financial_at: DateTime<Utc>,
    pub fill_model_version: String,
    pub fee_policy_instance_id: String,
    pub decision_linkage: RecordedDecisionLinkageV1,
    pub strategy_version_evidence: StrategyVersionEvidenceV1,
    pub orders: Vec<PaperDecisionOrderOutcomeV1>,
    pub fills: Vec<PaperDecisionFillLineageV1>,
}

/// Full original financial replay, namespace and read-tail checks precede the
/// report. Missing funding/catalog/source qualification remains a reader error.
pub fn read_paper_decision_outcomes_v1(
    account: &str,
) -> Result<RecordedPaperDecisionOutcomesV1, LedgerError> {
    summarize_recorded_execution(&execution::read_actual(account)?)
}

fn invalid() -> LedgerError {
    LedgerError::InvalidInput(
        "recorded paper decision lineage differs or exceeds report bounds".into(),
    )
}

fn add(value: &mut i64, delta: i64) -> Result<(), LedgerError> {
    *value = value.checked_add(delta).ok_or(LedgerError::Overflow)?;
    Ok(())
}

fn text_cost(total: &mut usize, value: &str) -> Result<(), LedgerError> {
    if value.is_empty() || value.len() > MAX_TEXT || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    *total = total
        .checked_add(value.len())
        .ok_or(LedgerError::Overflow)?;
    if *total > MAX_REPORT_ALLOCATION {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) fn summarize_recorded_execution(
    view: &RecordedExecutionView,
) -> Result<RecordedPaperDecisionOutcomesV1, LedgerError> {
    let projection = &view.projection;
    if projection.parents.len() > MAX_PARENTS || projection.fills.len() > MAX_FILLS {
        return Err(invalid());
    }
    // Fixed cardinality and all copied text are checked before report allocation.
    // Include temporary join/set nodes with a conservative 256-byte allowance.
    let mut cost = projection
        .parents
        .len()
        .checked_mul(std::mem::size_of::<PaperDecisionOrderOutcomeV1>() + 256)
        .and_then(|n| {
            projection
                .fills
                .len()
                .checked_mul(std::mem::size_of::<PaperDecisionFillLineageV1>() + 256)
                .and_then(|m| n.checked_add(m))
        })
        .ok_or(LedgerError::Overflow)?;
    for value in [
        &view.manifest.account_id,
        &view.manifest.epoch_id,
        &view.manifest_hash,
        &view.head.event_hash,
        &view.manifest.fill_model_version,
        &view.manifest.fee_policy_instance_id,
    ] {
        text_cost(&mut cost, value)?;
    }
    for (parent_id, parent) in &projection.parents {
        let intent = &parent.intent;
        if parent_id != &intent.parent_id
            || intent.account_id != view.manifest.account_id
            || intent.epoch_id != view.manifest.epoch_id
            || intent.execution_manifest_hash != view.manifest_hash
            || intent.family_id != view.manifest.budget.family_id
            || parent
                .filled
                .checked_add(parent.remaining)
                .and_then(|n| n.checked_add(parent.cancelled))
                != Some(intent.quantity)
        {
            return Err(invalid());
        }
        for value in [
            parent_id,
            &intent.investment_decision_id,
            &intent.family_id,
            &intent.chain_id,
            &intent.instrument_code,
        ] {
            text_cost(&mut cost, value)?;
        }
    }
    for fill in &projection.fills {
        for value in [&fill.fill_id, &fill.parent_id, &fill.observation_id] {
            text_cost(&mut cost, value)?;
        }
    }
    if cost > MAX_REPORT_ALLOCATION {
        return Err(invalid());
    }

    let mut orders = Vec::with_capacity(projection.parents.len());
    let mut joins = BTreeMap::new();
    let mut decision_ids = BTreeSet::new();
    for (parent_id, parent) in &projection.parents {
        let intent = &parent.intent;
        if !decision_ids.insert(intent.investment_decision_id.as_str()) {
            return Err(invalid());
        }
        joins.insert(parent_id.as_str(), (orders.len(), 0_u32));
        orders.push(PaperDecisionOrderOutcomeV1 {
            parent_id: parent_id.clone(),
            investment_decision_reference: intent.investment_decision_id.clone(),
            family_id: intent.family_id.clone(),
            chain_id: intent.chain_id.clone(),
            instrument_code: intent.instrument_code.clone(),
            side: intent.side.text().into(),
            status: match parent.status {
                ParentStatus::Working => "Working",
                ParentStatus::PartiallyFilled => "PartiallyFilled",
                ParentStatus::Filled => "Filled",
                ParentStatus::Cancelled => "Cancelled",
                ParentStatus::Expired => "Expired",
            }
            .into(),
            requested_quantity: intent.quantity,
            filled_quantity: parent.filled,
            remaining_quantity: parent.remaining,
            cancelled_quantity: parent.cancelled,
            fill_count: 0,
            filled_notional_micro_cny: 0,
            modeled_fee_micro_cny: 0,
            inherited_buy_fee_micro_cny: 0,
            realized_pnl_micro_cny: 0,
        });
    }
    let mut fills = Vec::with_capacity(projection.fills.len());
    let mut fill_ids = BTreeSet::new();
    for fill in &projection.fills {
        let Some((index, quantity)) = joins.get_mut(fill.parent_id.as_str()) else {
            return Err(invalid());
        };
        let parent = &projection.parents[&fill.parent_id];
        let model = &fill.model;
        if !fill_ids.insert(fill.fill_id.as_str())
            || fill.side != parent.intent.side
            || model.quantity == 0
            || model.quantity % 100 != 0
            || model.price_micro_cny <= 0
            || i128::from(model.notional_micro_cny)
                != i128::from(model.price_micro_cny) * i128::from(model.quantity)
            || model.commission_micro_cny < 0
            || model.stamp_tax_micro_cny < 0
            || model
                .commission_micro_cny
                .checked_add(model.stamp_tax_micro_cny)
                != Some(model.total_fee_micro_cny)
            || fill.inherited_buy_fee_micro_cny < 0
            || (fill.side == Side::Buy
                && (fill.realized_pnl_micro_cny != 0 || fill.inherited_buy_fee_micro_cny != 0))
            || model.fee_policy_instance_id != view.manifest.fee_policy_instance_id
        {
            return Err(invalid());
        }
        *quantity = quantity
            .checked_add(model.quantity)
            .ok_or(LedgerError::Overflow)?;
        let order = &mut orders[*index];
        order.fill_count += 1;
        add(
            &mut order.filled_notional_micro_cny,
            model.notional_micro_cny,
        )?;
        add(&mut order.modeled_fee_micro_cny, model.total_fee_micro_cny)?;
        add(
            &mut order.inherited_buy_fee_micro_cny,
            fill.inherited_buy_fee_micro_cny,
        )?;
        add(
            &mut order.realized_pnl_micro_cny,
            fill.realized_pnl_micro_cny,
        )?;
        fills.push(PaperDecisionFillLineageV1 {
            fill_id: fill.fill_id.clone(),
            parent_id: fill.parent_id.clone(),
            observation_id: fill.observation_id.clone(),
            executed_at: fill.executed_at,
            quantity: model.quantity,
            price_micro_cny: model.price_micro_cny,
            notional_micro_cny: model.notional_micro_cny,
            modeled_fee_micro_cny: model.total_fee_micro_cny,
            inherited_buy_fee_micro_cny: fill.inherited_buy_fee_micro_cny,
            realized_pnl_micro_cny: fill.realized_pnl_micro_cny,
        });
    }
    if joins
        .iter()
        .any(|(_, (index, quantity))| orders[*index].filled_quantity != *quantity)
    {
        return Err(invalid());
    }
    Ok(RecordedPaperDecisionOutcomesV1 {
        schema: "recorded-paper-decision-outcomes-v1",
        account_id: view.manifest.account_id.clone(),
        epoch_id: view.manifest.epoch_id.clone(),
        execution_manifest_hash: view.manifest_hash.clone(),
        observed_head_version: view.head.version,
        observed_head_hash: view.head.event_hash.clone(),
        observed_financial_at: projection.account.as_of,
        fill_model_version: view.manifest.fill_model_version.clone(),
        fee_policy_instance_id: view.manifest.fee_policy_instance_id.clone(),
        decision_linkage: RecordedDecisionLinkageV1::ReferenceOnly,
        strategy_version_evidence: StrategyVersionEvidenceV1::NotRecorded,
        orders,
        fills,
    })
}
