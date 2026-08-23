//! BR-248 经济仓位净收益归因。
//! Registered business rules: BR-248.
//!
//! 设计：docs/superpowers/specs/2026-08-20-attribution-research-loop-design.md §11。
//! 一个主样本是同一代码从空仓到再次空仓的完整生命周期；开放仓位右删失。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use chrono::{NaiveDate, NaiveDateTime};
use diesel::RunQueryDsl;

use super::attribution::{signal_family_of, SignalFamily};
use crate::trading::paper_lot_ledger::parse_paper_fill_timestamp;

const MIN_CLOSED_POSITIONS: usize = 200;
const MIN_COVERAGE_DAYS: i64 = 84;

#[derive(diesel::QueryableByName, Debug, Clone, PartialEq)]
pub struct EconomicFillRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub plan_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub name: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    pub fill_price: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub occurred_at: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub virtual_reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostBasisKind {
    Observed,
    Scenario,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FillCostEvidence {
    pub fill_id: i64,
    /// 该成交在已冻结口径下的总不利成本金额（人民币）。
    pub adverse_cost: f64,
    pub evidence_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FillCostLedger {
    pub basis_id: String,
    pub kind: CostBasisKind,
    pub costs: Vec<FillCostEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostBasisAudit {
    pub basis_id: String,
    pub kind: CostBasisKind,
    /// 与 `source_fill_ids` 同序的一对一费用证据 ID。
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetOutcomeClass {
    Profit,
    Loss,
    Breakeven,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetMetrics {
    Unavailable {
        reason: String,
    },
    Available {
        basis_id: String,
        kind: CostBasisKind,
        total_adverse_cost: f64,
        net_pnl: f64,
        return_on_buy_notional: f64,
        outcome: NetOutcomeClass,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryFamilyComposition {
    pub family: SignalFamily,
    pub quantity: u64,
    pub buy_notional: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosedEconomicPosition {
    pub cycle_open_fill_id: i64,
    pub code: String,
    pub names: Vec<String>,
    pub opened_at: NaiveDateTime,
    pub closed_at: NaiveDateTime,
    pub source_fill_ids: Vec<i64>,
    pub source_plan_ids: Vec<String>,
    pub buy_fill_ids: Vec<i64>,
    pub sell_fill_ids: Vec<i64>,
    pub exit_reasons: Vec<String>,
    pub entry_composition: Vec<EntryFamilyComposition>,
    pub gross_buy_notional: f64,
    pub gross_sell_notional: f64,
    pub gross_pnl: f64,
    pub net: NetMetrics,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenEconomicPosition {
    pub cycle_open_fill_id: i64,
    pub code: String,
    pub names: Vec<String>,
    pub opened_at: NaiveDateTime,
    pub source_fill_ids: Vec<i64>,
    pub source_plan_ids: Vec<String>,
    pub buy_fill_ids: Vec<i64>,
    pub sell_fill_ids: Vec<i64>,
    pub entry_composition: Vec<EntryFamilyComposition>,
    pub remaining_quantity: u64,
    pub gross_buy_notional: f64,
    pub gross_sell_notional: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetSummary {
    Unavailable {
        reason: String,
    },
    Available {
        basis_id: String,
        kind: CostBasisKind,
        wins: usize,
        losses: usize,
        breakeven: usize,
        win_rate: Option<f64>,
        total_adverse_cost: f64,
        total_net_pnl: f64,
        average_net_pnl: Option<f64>,
        median_net_pnl: Option<f64>,
        return_on_buy_notional: Option<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationStatus {
    NetUnavailable {
        reason: String,
    },
    InsufficientSample {
        closed_positions: usize,
        coverage_days: Option<i64>,
        reasons: Vec<String>,
    },
    ResearchOnly {
        missing_evidence: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EconomicPositionReport {
    pub as_of_date: NaiveDate,
    pub source_fill_ids: Vec<i64>,
    pub cost_basis: Option<CostBasisAudit>,
    pub closed_positions: Vec<ClosedEconomicPosition>,
    pub open_positions: Vec<OpenEconomicPosition>,
    pub coverage_days: Option<i64>,
    pub net_summary: NetSummary,
    pub validation_status: ValidationStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FillDirection {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
struct ValidatedFill {
    id: i64,
    plan_id: String,
    code: String,
    name: String,
    direction: FillDirection,
    price: f64,
    quantity: u32,
    occurred_at: NaiveDateTime,
    virtual_reason: String,
    entry_family: Option<SignalFamily>,
}

#[derive(Debug)]
struct OpenLot {
    bought_at: NaiveDateTime,
    remaining_quantity: u32,
}

#[derive(Debug, Default)]
struct FamilyAccumulator {
    quantity: u64,
    buy_notional: f64,
}

#[derive(Debug)]
struct PositionCycle {
    cycle_open_fill_id: i64,
    code: String,
    names: BTreeSet<String>,
    opened_at: NaiveDateTime,
    source_fill_ids: Vec<i64>,
    source_plan_ids: Vec<String>,
    buy_fill_ids: Vec<i64>,
    sell_fill_ids: Vec<i64>,
    exit_reasons: BTreeSet<String>,
    entry_composition: BTreeMap<SignalFamily, FamilyAccumulator>,
    lots: VecDeque<OpenLot>,
    remaining_quantity: u64,
    gross_buy_notional: f64,
    gross_sell_notional: f64,
}

impl PositionCycle {
    fn from_buy(fill: &ValidatedFill) -> Result<Self, String> {
        let mut cycle = Self {
            cycle_open_fill_id: fill.id,
            code: fill.code.clone(),
            names: BTreeSet::new(),
            opened_at: fill.occurred_at,
            source_fill_ids: Vec::new(),
            source_plan_ids: Vec::new(),
            buy_fill_ids: Vec::new(),
            sell_fill_ids: Vec::new(),
            exit_reasons: BTreeSet::new(),
            entry_composition: BTreeMap::new(),
            lots: VecDeque::new(),
            remaining_quantity: 0,
            gross_buy_notional: 0.0,
            gross_sell_notional: 0.0,
        };
        cycle.add_buy(fill)?;
        Ok(cycle)
    }

    fn add_buy(&mut self, fill: &ValidatedFill) -> Result<(), String> {
        let family = fill.entry_family.ok_or_else(|| {
            format!(
                "economic buy id={} entry strategy family unavailable",
                fill.id
            )
        })?;
        let notional = checked_notional(fill.price, fill.quantity, fill.id)?;
        self.names.insert(fill.name.clone());
        self.source_fill_ids.push(fill.id);
        self.source_plan_ids.push(fill.plan_id.clone());
        self.buy_fill_ids.push(fill.id);
        self.gross_buy_notional = checked_money_add(
            self.gross_buy_notional,
            notional,
            &format!("economic cycle {} buy notional", self.cycle_open_fill_id),
        )?;
        self.remaining_quantity = self
            .remaining_quantity
            .checked_add(u64::from(fill.quantity))
            .ok_or_else(|| {
                format!(
                    "economic cycle {} quantity overflow",
                    self.cycle_open_fill_id
                )
            })?;
        let composition = self.entry_composition.entry(family).or_default();
        composition.quantity = composition
            .quantity
            .checked_add(u64::from(fill.quantity))
            .ok_or_else(|| format!("economic fill id={} family quantity overflow", fill.id))?;
        composition.buy_notional = checked_money_add(
            composition.buy_notional,
            notional,
            &format!("economic fill id={} family buy notional", fill.id),
        )?;
        self.lots.push_back(OpenLot {
            bought_at: fill.occurred_at,
            remaining_quantity: fill.quantity,
        });
        Ok(())
    }

    fn add_sell(&mut self, fill: &ValidatedFill) -> Result<(), String> {
        let notional = checked_notional(fill.price, fill.quantity, fill.id)?;
        let mut remaining = fill.quantity;
        while remaining > 0 {
            let lot = self.lots.front_mut().ok_or_else(|| {
                format!(
                    "economic sell id={} oversells {} by {} shares",
                    fill.id, self.code, remaining
                )
            })?;
            if lot.bought_at.date() >= fill.occurred_at.date() {
                return Err(format!(
                    "economic sell id={} violates A-share T+1 for {}: buy_date={} sell_date={}",
                    fill.id,
                    self.code,
                    lot.bought_at.date(),
                    fill.occurred_at.date()
                ));
            }
            let consumed = remaining.min(lot.remaining_quantity);
            remaining -= consumed;
            lot.remaining_quantity -= consumed;
            if lot.remaining_quantity == 0 {
                self.lots.pop_front();
            }
        }
        self.remaining_quantity = self
            .remaining_quantity
            .checked_sub(u64::from(fill.quantity))
            .ok_or_else(|| {
                format!(
                    "economic sell id={} oversells {} by quantity underflow",
                    fill.id, self.code
                )
            })?;
        self.names.insert(fill.name.clone());
        self.source_fill_ids.push(fill.id);
        self.source_plan_ids.push(fill.plan_id.clone());
        self.sell_fill_ids.push(fill.id);
        self.exit_reasons.insert(fill.virtual_reason.clone());
        self.gross_sell_notional = checked_money_add(
            self.gross_sell_notional,
            notional,
            &format!("economic cycle {} sell notional", self.cycle_open_fill_id),
        )?;
        Ok(())
    }

    fn composition(&self) -> Vec<EntryFamilyComposition> {
        self.entry_composition
            .iter()
            .map(|(family, value)| EntryFamilyComposition {
                family: *family,
                quantity: value.quantity,
                buy_notional: value.buy_notional,
            })
            .collect()
    }
}

#[derive(Debug)]
enum ValidatedCostLedger {
    Unavailable,
    Available {
        basis_id: String,
        kind: CostBasisKind,
        costs: HashMap<i64, ValidatedFillCost>,
    },
}

#[derive(Debug)]
struct ValidatedFillCost {
    adverse_cost: f64,
    evidence_id: String,
}

fn checked_notional(price: f64, quantity: u32, fill_id: i64) -> Result<f64, String> {
    let notional = price * f64::from(quantity);
    if !notional.is_finite() || notional <= 0.0 {
        return Err(format!(
            "economic fill id={fill_id} notional invalid: {notional}"
        ));
    }
    Ok(notional)
}

fn checked_money_add(total: f64, amount: f64, label: &str) -> Result<f64, String> {
    let value = total + amount;
    if !value.is_finite() {
        return Err(format!("{label} is non-finite"));
    }
    Ok(value)
}

fn validate_fills(
    rows: &[EconomicFillRow],
    as_of_date: NaiveDate,
) -> Result<Vec<ValidatedFill>, String> {
    let mut seen_ids = HashSet::new();
    let mut seen_plan_ids = HashSet::new();
    let mut previous_order = None;
    let mut validated = Vec::with_capacity(rows.len());
    for row in rows {
        if row.id <= 0
            || row.plan_id.trim().is_empty()
            || row.code.trim().is_empty()
            || row.name.trim().is_empty()
            || row.virtual_reason.trim().is_empty()
        {
            return Err(format!(
                "economic fill identity invalid: id={} plan_id={:?} code={:?} name={:?}",
                row.id, row.plan_id, row.code, row.name
            ));
        }
        if !seen_ids.insert(row.id) {
            return Err(format!("economic fill duplicate identity: id={}", row.id));
        }
        if !seen_plan_ids.insert(row.plan_id.as_str()) {
            return Err(format!(
                "economic fill duplicate plan identity: plan_id={:?}",
                row.plan_id
            ));
        }
        let occurred_at = parse_paper_fill_timestamp(row.id, &row.occurred_at)?;
        let current_order = (occurred_at, row.id);
        if previous_order.is_some_and(|previous| previous >= current_order) {
            return Err(format!(
                "economic fills out of order at id={} occurred_at={}",
                row.id, occurred_at
            ));
        }
        previous_order = Some(current_order);
        if occurred_at.date() > as_of_date {
            return Err(format!(
                "economic fill id={} has future fill date {} after {}",
                row.id,
                occurred_at.date(),
                as_of_date
            ));
        }
        let price = row
            .fill_price
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| format!("economic fill id={} fill_price missing/invalid", row.id))?;
        let quantity = u32::try_from(row.quantity)
            .ok()
            .filter(|value| *value > 0 && value.is_multiple_of(100))
            .ok_or_else(|| {
                format!(
                    "economic fill id={} quantity invalid: {}",
                    row.id, row.quantity
                )
            })?;
        checked_notional(price, quantity, row.id)?;
        let (direction, entry_family) = match row.direction.as_str() {
            "buy" => {
                let family = signal_family_of(&row.virtual_reason);
                if matches!(family, SignalFamily::Unknown | SignalFamily::ExitByRule) {
                    return Err(format!(
                        "economic buy id={} entry strategy family unavailable: {:?}",
                        row.id, row.virtual_reason
                    ));
                }
                (FillDirection::Buy, Some(family))
            }
            "sell" => (FillDirection::Sell, None),
            other => {
                return Err(format!(
                    "economic fill id={} direction invalid: {other:?}",
                    row.id
                ));
            }
        };
        validated.push(ValidatedFill {
            id: row.id,
            plan_id: row.plan_id.clone(),
            code: row.code.clone(),
            name: row.name.clone(),
            direction,
            price,
            quantity,
            occurred_at,
            virtual_reason: row.virtual_reason.clone(),
            entry_family,
        });
    }
    Ok(validated)
}

fn validate_cost_ledger(
    fills: &[ValidatedFill],
    ledger: Option<&FillCostLedger>,
) -> Result<ValidatedCostLedger, String> {
    let Some(ledger) = ledger else {
        return Ok(ValidatedCostLedger::Unavailable);
    };
    if ledger.basis_id.trim().is_empty() {
        return Err("economic cost basis_id is empty".to_owned());
    }
    if ledger.kind == CostBasisKind::Observed {
        return Err(
            "economic observed cost source-backed capability is unavailable; use Scenario or no ledger"
                .to_owned(),
        );
    }
    let fill_ids = fills.iter().map(|fill| fill.id).collect::<HashSet<_>>();
    let mut costs = HashMap::with_capacity(ledger.costs.len());
    for cost in &ledger.costs {
        if !fill_ids.contains(&cost.fill_id) {
            return Err(format!(
                "economic cost ledger references unknown fill id={}",
                cost.fill_id
            ));
        }
        if cost.evidence_id.trim().is_empty()
            || !cost.adverse_cost.is_finite()
            || cost.adverse_cost < 0.0
        {
            return Err(format!(
                "economic cost evidence invalid for fill id={}",
                cost.fill_id
            ));
        }
        if costs
            .insert(
                cost.fill_id,
                ValidatedFillCost {
                    adverse_cost: cost.adverse_cost,
                    evidence_id: cost.evidence_id.clone(),
                },
            )
            .is_some()
        {
            return Err(format!(
                "economic cost ledger duplicate fill id={}",
                cost.fill_id
            ));
        }
    }
    for fill in fills {
        if !costs.contains_key(&fill.id) {
            return Err(format!("economic cost ledger missing fill id={}", fill.id));
        }
    }
    Ok(ValidatedCostLedger::Available {
        basis_id: ledger.basis_id.clone(),
        kind: ledger.kind,
        costs,
    })
}

fn net_metrics_for_cycle(
    cycle: &PositionCycle,
    gross_pnl: f64,
    ledger: &ValidatedCostLedger,
) -> Result<NetMetrics, String> {
    let ValidatedCostLedger::Available {
        basis_id,
        kind,
        costs,
    } = ledger
    else {
        return Ok(NetMetrics::Unavailable {
            reason: "fill cost ledger unavailable; net metrics are not computed".to_owned(),
        });
    };
    let mut total_adverse_cost = 0.0;
    for fill_id in &cycle.source_fill_ids {
        let cost = costs.get(fill_id).ok_or_else(|| {
            format!("economic cost ledger missing cycle fill id={fill_id} after validation")
        })?;
        total_adverse_cost = checked_money_add(
            total_adverse_cost,
            cost.adverse_cost,
            &format!("economic cycle {} total cost", cycle.cycle_open_fill_id),
        )?;
    }
    let net_pnl = gross_pnl - total_adverse_cost;
    let return_on_buy_notional = net_pnl / cycle.gross_buy_notional;
    if !net_pnl.is_finite() || !return_on_buy_notional.is_finite() {
        return Err(format!(
            "economic cycle {} net metrics are non-finite",
            cycle.cycle_open_fill_id
        ));
    }
    let outcome = if net_pnl > 0.0 {
        NetOutcomeClass::Profit
    } else if net_pnl < 0.0 {
        NetOutcomeClass::Loss
    } else {
        NetOutcomeClass::Breakeven
    };
    Ok(NetMetrics::Available {
        basis_id: basis_id.clone(),
        kind: *kind,
        total_adverse_cost,
        net_pnl,
        return_on_buy_notional,
        outcome,
    })
}

fn cost_basis_audit(
    fills: &[ValidatedFill],
    ledger: &ValidatedCostLedger,
) -> Result<Option<CostBasisAudit>, String> {
    let ValidatedCostLedger::Available {
        basis_id,
        kind,
        costs,
    } = ledger
    else {
        return Ok(None);
    };
    let mut evidence_ids = Vec::with_capacity(fills.len());
    for fill in fills {
        let cost = costs.get(&fill.id).ok_or_else(|| {
            format!(
                "economic cost ledger missing report fill id={} after validation",
                fill.id
            )
        })?;
        evidence_ids.push(cost.evidence_id.clone());
    }
    Ok(Some(CostBasisAudit {
        basis_id: basis_id.clone(),
        kind: *kind,
        evidence_ids,
    }))
}

fn close_cycle(
    cycle: PositionCycle,
    closed_at: NaiveDateTime,
    ledger: &ValidatedCostLedger,
) -> Result<ClosedEconomicPosition, String> {
    let gross_pnl = cycle.gross_sell_notional - cycle.gross_buy_notional;
    if !gross_pnl.is_finite() {
        return Err(format!(
            "economic cycle {} gross PnL is non-finite",
            cycle.cycle_open_fill_id
        ));
    }
    let net = net_metrics_for_cycle(&cycle, gross_pnl, ledger)?;
    let entry_composition = cycle.composition();
    Ok(ClosedEconomicPosition {
        cycle_open_fill_id: cycle.cycle_open_fill_id,
        code: cycle.code,
        names: cycle.names.into_iter().collect(),
        opened_at: cycle.opened_at,
        closed_at,
        source_fill_ids: cycle.source_fill_ids,
        source_plan_ids: cycle.source_plan_ids,
        buy_fill_ids: cycle.buy_fill_ids,
        sell_fill_ids: cycle.sell_fill_ids,
        exit_reasons: cycle.exit_reasons.into_iter().collect(),
        entry_composition,
        gross_buy_notional: cycle.gross_buy_notional,
        gross_sell_notional: cycle.gross_sell_notional,
        gross_pnl,
        net,
    })
}

fn summarize_net(
    closed: &[ClosedEconomicPosition],
    ledger: &ValidatedCostLedger,
) -> Result<NetSummary, String> {
    let ValidatedCostLedger::Available { basis_id, kind, .. } = ledger else {
        return Ok(NetSummary::Unavailable {
            reason: "fill cost ledger unavailable; net summary is not computed".to_owned(),
        });
    };
    let mut wins = 0;
    let mut losses = 0;
    let mut breakeven = 0;
    let mut total_adverse_cost = 0.0;
    let mut total_net_pnl = 0.0;
    let mut total_buy_notional = 0.0;
    let mut net_values = Vec::with_capacity(closed.len());
    for cycle in closed {
        let NetMetrics::Available {
            total_adverse_cost: cycle_cost,
            net_pnl,
            outcome,
            ..
        } = cycle.net
        else {
            return Err(format!(
                "economic cycle {} net metrics unavailable under a complete cost ledger",
                cycle.cycle_open_fill_id
            ));
        };
        match outcome {
            NetOutcomeClass::Profit => wins += 1,
            NetOutcomeClass::Loss => losses += 1,
            NetOutcomeClass::Breakeven => breakeven += 1,
        }
        total_adverse_cost = checked_money_add(
            total_adverse_cost,
            cycle_cost,
            "economic summary total adverse cost",
        )?;
        total_net_pnl = checked_money_add(total_net_pnl, net_pnl, "economic summary net PnL")?;
        total_buy_notional = checked_money_add(
            total_buy_notional,
            cycle.gross_buy_notional,
            "economic summary buy notional",
        )?;
        net_values.push(net_pnl);
    }
    net_values.sort_by(f64::total_cmp);
    let average_net_pnl = (!net_values.is_empty())
        .then_some(total_net_pnl / net_values.len() as f64)
        .filter(|value| value.is_finite());
    let median_net_pnl = if net_values.is_empty() {
        None
    } else if net_values.len().is_multiple_of(2) {
        let upper = net_values.len() / 2;
        Some(net_values[upper - 1] / 2.0 + net_values[upper] / 2.0)
    } else {
        Some(net_values[net_values.len() / 2])
    };
    let directional = wins + losses;
    let win_rate = (directional > 0).then_some(wins as f64 / directional as f64);
    let return_on_buy_notional = (total_buy_notional > 0.0)
        .then_some(total_net_pnl / total_buy_notional)
        .filter(|value| value.is_finite());
    Ok(NetSummary::Available {
        basis_id: basis_id.clone(),
        kind: *kind,
        wins,
        losses,
        breakeven,
        win_rate,
        total_adverse_cost,
        total_net_pnl,
        average_net_pnl,
        median_net_pnl,
        return_on_buy_notional,
    })
}

fn coverage_days(closed: &[ClosedEconomicPosition]) -> Result<Option<i64>, String> {
    let Some(first_opened) = closed.iter().map(|cycle| cycle.opened_at.date()).min() else {
        return Ok(None);
    };
    let last_closed = closed
        .iter()
        .map(|cycle| cycle.closed_at.date())
        .max()
        .ok_or_else(|| "economic closed-position coverage is unavailable".to_owned())?;
    let days = last_closed
        .signed_duration_since(first_opened)
        .num_days()
        .checked_add(1)
        .ok_or_else(|| "economic coverage days overflow".to_owned())?;
    if days <= 0 {
        return Err(format!("economic coverage days invalid: {days}"));
    }
    Ok(Some(days))
}

fn validation_status(
    net_summary: &NetSummary,
    closed_positions: usize,
    coverage_days: Option<i64>,
) -> ValidationStatus {
    if let NetSummary::Unavailable { reason } = net_summary {
        return ValidationStatus::NetUnavailable {
            reason: reason.clone(),
        };
    }
    let mut reasons = Vec::new();
    if closed_positions < MIN_CLOSED_POSITIONS {
        reasons.push(format!(
            "closed economic positions {closed_positions} < {MIN_CLOSED_POSITIONS}"
        ));
    }
    if coverage_days.is_none_or(|days| days < MIN_COVERAGE_DAYS) {
        reasons.push(format!(
            "coverage days {} < {MIN_COVERAGE_DAYS}",
            coverage_days.map_or_else(|| "unavailable".to_owned(), |days| days.to_string())
        ));
    }
    if !reasons.is_empty() {
        return ValidationStatus::InsufficientSample {
            closed_positions,
            coverage_days,
            reasons,
        };
    }
    ValidationStatus::ResearchOnly {
        missing_evidence: vec![
            "cycle-aligned benchmark alpha".to_owned(),
            "code/entry-date clustered uncertainty".to_owned(),
            "source-backed market-regime coverage".to_owned(),
        ],
    }
}

/// BR-248：从已排序的真实成交事实重建空仓到再次空仓的经济仓位。
pub fn rebuild_economic_positions(
    rows: &[EconomicFillRow],
    as_of_date: NaiveDate,
    cost_ledger: Option<&FillCostLedger>,
) -> Result<EconomicPositionReport, String> {
    let fills = validate_fills(rows, as_of_date)?;
    let ledger = validate_cost_ledger(&fills, cost_ledger)?;
    let source_fill_ids = fills.iter().map(|fill| fill.id).collect::<Vec<_>>();
    let cost_basis = cost_basis_audit(&fills, &ledger)?;
    let mut states = BTreeMap::<String, PositionCycle>::new();
    let mut closed_positions = Vec::new();

    for fill in &fills {
        match fill.direction {
            FillDirection::Buy => match states.get_mut(&fill.code) {
                Some(cycle) => cycle.add_buy(fill)?,
                None => {
                    states.insert(fill.code.clone(), PositionCycle::from_buy(fill)?);
                }
            },
            FillDirection::Sell => {
                let closes = {
                    let cycle = states.get_mut(&fill.code).ok_or_else(|| {
                        format!(
                            "economic sell id={} oversells {} without an open position",
                            fill.id, fill.code
                        )
                    })?;
                    cycle.add_sell(fill)?;
                    cycle.remaining_quantity == 0
                };
                if closes {
                    let cycle = states.remove(&fill.code).ok_or_else(|| {
                        format!(
                            "economic sell id={} closed position state disappeared for {}",
                            fill.id, fill.code
                        )
                    })?;
                    closed_positions.push(close_cycle(cycle, fill.occurred_at, &ledger)?);
                }
            }
        }
    }

    let open_positions = states
        .into_values()
        .map(|cycle| {
            let entry_composition = cycle.composition();
            OpenEconomicPosition {
                cycle_open_fill_id: cycle.cycle_open_fill_id,
                code: cycle.code,
                names: cycle.names.into_iter().collect(),
                opened_at: cycle.opened_at,
                source_fill_ids: cycle.source_fill_ids,
                source_plan_ids: cycle.source_plan_ids,
                buy_fill_ids: cycle.buy_fill_ids,
                sell_fill_ids: cycle.sell_fill_ids,
                entry_composition,
                remaining_quantity: cycle.remaining_quantity,
                gross_buy_notional: cycle.gross_buy_notional,
                gross_sell_notional: cycle.gross_sell_notional,
            }
        })
        .collect::<Vec<_>>();
    let coverage_days = coverage_days(&closed_positions)?;
    let net_summary = summarize_net(&closed_positions, &ledger)?;
    let validation_status = validation_status(&net_summary, closed_positions.len(), coverage_days);
    Ok(EconomicPositionReport {
        as_of_date,
        source_fill_ids,
        cost_basis,
        closed_positions,
        open_positions,
        coverage_days,
        net_summary,
        validation_status,
    })
}

const ECONOMIC_FILLS_SQL: &str = "SELECT id, plan_id, code, name, direction, fill_price, \
     quantity, CAST(ts AS TEXT) AS occurred_at, virtual_reason \
     FROM paper_trades WHERE status = 'Filled' ORDER BY ts ASC, id ASC";

/// 完整来源批次先严格验证，再由 Rust 截止到显式评估日。评估日后的坏行也不能
/// 被日期过滤静默隐藏。
pub fn select_economic_rows_through(
    rows: Vec<EconomicFillRow>,
    as_of_date: NaiveDate,
) -> Result<Vec<EconomicFillRow>, String> {
    let mut previous_order = None;
    let mut dates = Vec::with_capacity(rows.len());
    let mut max_source_date = as_of_date;
    for row in &rows {
        let occurred_at = parse_paper_fill_timestamp(row.id, &row.occurred_at)?;
        let order = (occurred_at, row.id);
        if previous_order.is_some_and(|previous| previous >= order) {
            return Err(format!(
                "economic fills out of order at id={} occurred_at={}",
                row.id, occurred_at
            ));
        }
        previous_order = Some(order);
        let source_date = occurred_at.date();
        max_source_date = max_source_date.max(source_date);
        dates.push(source_date);
    }
    // 先验证完整来源批次的身份、价格、数量、方向和买入族，再做历史截止；
    // 后来的坏行不能被日期过滤静默藏掉。
    validate_fills(&rows, max_source_date)?;
    Ok(rows
        .into_iter()
        .zip(dates)
        .filter_map(|(row, source_date)| (source_date <= as_of_date).then_some(row))
        .collect())
}

/// 只读薄壳：读取原始时间，不调用 SQLite 日期解释函数。
pub fn query_economic_fills_through(as_of_date: NaiveDate) -> Result<Vec<EconomicFillRow>, String> {
    let db = crate::database::DatabaseManager::try_get()
        .ok_or_else(|| "economic-position database is not initialized".to_owned())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("economic-position database: {error}"))?;
    let rows = diesel::sql_query(ECONOMIC_FILLS_SQL)
        .load::<EconomicFillRow>(&mut conn)
        .map_err(|error| format!("query economic-position fills: {error}"))?;
    select_economic_rows_through(rows, as_of_date)
}

/// 当前仅供显式只读研究调用；没有费用证据时净指标保持 Unavailable。
pub fn compute_economic_position_report(
    as_of_date: NaiveDate,
    cost_ledger: Option<&FillCostLedger>,
) -> Result<EconomicPositionReport, String> {
    let rows = query_economic_fills_through(as_of_date)?;
    rebuild_economic_positions(&rows, as_of_date, cost_ledger)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn date(raw: &str) -> NaiveDate {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d").unwrap()
    }

    fn fill(
        id: i64,
        code: &str,
        direction: &str,
        price: f64,
        quantity: i64,
        occurred_at: &str,
        reason: &str,
    ) -> EconomicFillRow {
        EconomicFillRow {
            id,
            plan_id: format!("TEST_CODE_PLAN_{id}"),
            code: code.to_owned(),
            name: format!("TEST_CODE_{code}"),
            direction: direction.to_owned(),
            fill_price: Some(price),
            quantity,
            occurred_at: occurred_at.to_owned(),
            virtual_reason: reason.to_owned(),
        }
    }

    fn complete_costs(
        kind: CostBasisKind,
        rows: &[EconomicFillRow],
        adverse_cost: f64,
    ) -> FillCostLedger {
        FillCostLedger {
            basis_id: format!("TEST_CODE_{kind:?}_COST_V1"),
            kind,
            costs: rows
                .iter()
                .map(|row| FillCostEvidence {
                    fill_id: row.id,
                    adverse_cost,
                    evidence_id: format!("TEST_CODE_COST_EVIDENCE_{}", row.id),
                })
                .collect(),
        }
    }

    #[test]
    fn br248_multi_lot_and_partial_sells_form_one_closed_position() {
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-01-05 10:00:00",
                "NewsCatalyst",
            ),
            fill(
                2,
                "TEST_CODE_600001",
                "buy",
                12.0,
                100,
                "2026-01-06 10:00:00",
                "Momentum",
            ),
            fill(
                3,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-01-07 10:00:00",
                "BR-234四大铁律卖出:减仓",
            ),
            fill(
                4,
                "TEST_CODE_600001",
                "sell",
                13.0,
                100,
                "2026-01-08 10:00:00",
                "BR-234四大铁律卖出:清仓",
            ),
        ];

        let report = rebuild_economic_positions(&rows, date("2026-01-08"), None).unwrap();

        assert_eq!(report.closed_positions.len(), 1);
        assert!(report.open_positions.is_empty());
        let closed = &report.closed_positions[0];
        assert_eq!(closed.buy_fill_ids, vec![1, 2]);
        assert_eq!(closed.sell_fill_ids, vec![3, 4]);
        assert_eq!(closed.gross_buy_notional, 2_200.0);
        assert_eq!(closed.gross_sell_notional, 2_400.0);
        assert_eq!(closed.gross_pnl, 200.0);
        assert_eq!(closed.entry_composition.len(), 2);
        assert!(matches!(closed.net, NetMetrics::Unavailable { .. }));
        assert!(matches!(report.net_summary, NetSummary::Unavailable { .. }));
    }

    #[test]
    fn br248_flat_reentry_is_a_new_cycle_and_codes_are_isolated() {
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-01-05 10:00:00",
                "Momentum",
            ),
            fill(
                2,
                "TEST_CODE_000002",
                "buy",
                20.0,
                100,
                "2026-01-05 10:01:00",
                "Breakout",
            ),
            fill(
                3,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-01-06 10:00:00",
                "BR-234四大铁律卖出",
            ),
            fill(
                4,
                "TEST_CODE_600001",
                "buy",
                12.0,
                100,
                "2026-01-07 10:00:00",
                "Momentum",
            ),
            fill(
                5,
                "TEST_CODE_600001",
                "sell",
                10.0,
                100,
                "2026-01-08 10:00:00",
                "BR-234四大铁律卖出",
            ),
        ];

        let report = rebuild_economic_positions(&rows, date("2026-01-08"), None).unwrap();

        assert_eq!(report.closed_positions.len(), 2);
        assert_eq!(report.closed_positions[0].cycle_open_fill_id, 1);
        assert_eq!(report.closed_positions[1].cycle_open_fill_id, 4);
        assert_eq!(report.open_positions.len(), 1);
        assert_eq!(report.open_positions[0].code, "TEST_CODE_000002");
        assert_eq!(report.open_positions[0].remaining_quantity, 100);
    }

    #[test]
    fn br248_invalid_trade_facts_fail_the_whole_batch() {
        let base = vec![
            fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-01-05 10:00:00",
                "Momentum",
            ),
            fill(
                2,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-01-06 10:00:00",
                "BR-234四大铁律卖出",
            ),
        ];

        let mut unknown = base.clone();
        unknown[0].virtual_reason = "mystery".to_owned();
        assert!(
            rebuild_economic_positions(&unknown, date("2026-01-06"), None)
                .unwrap_err()
                .contains("entry strategy family unavailable")
        );

        let same_day = vec![
            fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-01-05 10:00:00",
                "Momentum",
            ),
            fill(
                2,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-01-05 14:00:00",
                "BR-234四大铁律卖出",
            ),
        ];
        assert!(
            rebuild_economic_positions(&same_day, date("2026-01-05"), None)
                .unwrap_err()
                .contains("T+1")
        );

        let mut duplicate = base.clone();
        duplicate[1].id = 1;
        assert!(
            rebuild_economic_positions(&duplicate, date("2026-01-06"), None)
                .unwrap_err()
                .contains("duplicate")
        );

        let mut oversell = base.clone();
        oversell[1].quantity = 200;
        assert!(
            rebuild_economic_positions(&oversell, date("2026-01-06"), None)
                .unwrap_err()
                .contains("oversell")
        );

        let mut future = base.clone();
        future[1].occurred_at = "2026-01-07 10:00:00".to_owned();
        assert!(
            rebuild_economic_positions(&future, date("2026-01-06"), None)
                .unwrap_err()
                .contains("future fill")
        );

        let mut bad_price = base.clone();
        bad_price[0].fill_price = None;
        assert!(
            rebuild_economic_positions(&bad_price, date("2026-01-06"), None)
                .unwrap_err()
                .contains("fill_price missing/invalid")
        );

        let mut bad_quantity = base.clone();
        bad_quantity[0].quantity = 150;
        assert!(
            rebuild_economic_positions(&bad_quantity, date("2026-01-06"), None)
                .unwrap_err()
                .contains("quantity invalid")
        );

        let mut bad_direction = base;
        bad_direction[0].direction = "hold".to_owned();
        assert!(
            rebuild_economic_positions(&bad_direction, date("2026-01-06"), None)
                .unwrap_err()
                .contains("direction invalid")
        );

        let mut bad_time = vec![
            fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-01-05 10:00:00",
                "Momentum",
            ),
            fill(
                2,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-01-06 10:00:00",
                "BR-234四大铁律卖出",
            ),
        ];
        bad_time[0].occurred_at = "now".to_owned();
        assert!(
            rebuild_economic_positions(&bad_time, date("2026-01-06"), None)
                .unwrap_err()
                .contains("timestamp invalid")
        );

        let mut unordered = bad_time;
        unordered[0].occurred_at = "2026-01-06 10:00:00".to_owned();
        unordered[1].occurred_at = "2026-01-05 10:00:00".to_owned();
        assert!(
            rebuild_economic_positions(&unordered, date("2026-01-06"), None)
                .unwrap_err()
                .contains("out of order")
        );
    }

    #[test]
    fn br248_historical_cutoff_validates_later_source_rows_before_filtering() {
        let mut rows = vec![
            fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-01-05 10:00:00",
                "Momentum",
            ),
            fill(
                2,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-01-06 10:00:00",
                "BR-234四大铁律卖出",
            ),
            fill(
                3,
                "TEST_CODE_600002",
                "buy",
                12.0,
                100,
                "2026-01-07 10:00:00",
                "Momentum",
            ),
        ];
        let through = select_economic_rows_through(rows.clone(), date("2026-01-06")).unwrap();
        assert_eq!(through.len(), 2);

        rows[2].virtual_reason = "mystery".to_owned();
        assert!(select_economic_rows_through(rows, date("2026-01-06"))
            .unwrap_err()
            .contains("entry strategy family unavailable"));
    }

    #[test]
    fn br248_unverified_observed_cost_ledger_fails_closed() {
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-01-05 10:00:00",
                "Momentum",
            ),
            fill(
                2,
                "TEST_CODE_600001",
                "sell",
                12.0,
                100,
                "2026-01-06 10:00:00",
                "BR-234四大铁律卖出",
            ),
        ];
        let ledger = complete_costs(CostBasisKind::Observed, &rows, 5.0);
        let error =
            rebuild_economic_positions(&rows, date("2026-01-06"), Some(&ledger)).unwrap_err();
        assert!(
            error.contains("source-backed capability is unavailable"),
            "{error}"
        );
    }

    #[test]
    fn br248_cost_ledger_must_bind_every_fill_exactly_once() {
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-01-05 10:00:00",
                "Momentum",
            ),
            fill(
                2,
                "TEST_CODE_600001",
                "sell",
                12.0,
                100,
                "2026-01-06 10:00:00",
                "BR-234四大铁律卖出",
            ),
        ];

        let mut missing = complete_costs(CostBasisKind::Scenario, &rows, 0.0);
        missing.costs.pop();
        assert!(
            rebuild_economic_positions(&rows, date("2026-01-06"), Some(&missing))
                .unwrap_err()
                .contains("missing fill id=2")
        );

        let mut duplicate = complete_costs(CostBasisKind::Scenario, &rows, 0.0);
        duplicate.costs.push(duplicate.costs[0].clone());
        assert!(
            rebuild_economic_positions(&rows, date("2026-01-06"), Some(&duplicate))
                .unwrap_err()
                .contains("duplicate fill id=1")
        );

        let mut unknown = complete_costs(CostBasisKind::Scenario, &rows, 0.0);
        unknown.costs.push(FillCostEvidence {
            fill_id: 99,
            adverse_cost: 0.0,
            evidence_id: "TEST_CODE_UNKNOWN_COST".to_owned(),
        });
        assert!(
            rebuild_economic_positions(&rows, date("2026-01-06"), Some(&unknown))
                .unwrap_err()
                .contains("unknown fill id=99")
        );

        let mut invalid_cost = complete_costs(CostBasisKind::Scenario, &rows, 0.0);
        invalid_cost.costs[0].adverse_cost = -0.01;
        assert!(
            rebuild_economic_positions(&rows, date("2026-01-06"), Some(&invalid_cost))
                .unwrap_err()
                .contains("cost evidence invalid")
        );

        let mut empty_basis = complete_costs(CostBasisKind::Scenario, &rows, 0.0);
        empty_basis.basis_id.clear();
        assert!(
            rebuild_economic_positions(&rows, date("2026-01-06"), Some(&empty_basis))
                .unwrap_err()
                .contains("basis_id is empty")
        );
    }

    #[test]
    fn br248_profit_loss_and_breakeven_have_separate_denominators() {
        let rows = vec![
            fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-01-01 10:00:00",
                "Momentum",
            ),
            fill(
                2,
                "TEST_CODE_600002",
                "buy",
                10.0,
                100,
                "2026-01-01 10:00:01",
                "Momentum",
            ),
            fill(
                3,
                "TEST_CODE_600003",
                "buy",
                10.0,
                100,
                "2026-01-01 10:00:02",
                "Momentum",
            ),
            fill(
                4,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-01-02 10:00:00",
                "BR-234四大铁律卖出",
            ),
            fill(
                5,
                "TEST_CODE_600002",
                "sell",
                9.0,
                100,
                "2026-01-02 10:00:01",
                "BR-234四大铁律卖出",
            ),
            fill(
                6,
                "TEST_CODE_600003",
                "sell",
                10.0,
                100,
                "2026-01-02 10:00:02",
                "BR-234四大铁律卖出",
            ),
        ];
        let ledger = complete_costs(CostBasisKind::Scenario, &rows, 0.0);

        let report = rebuild_economic_positions(&rows, date("2026-01-02"), Some(&ledger)).unwrap();

        assert!(matches!(
            report.net_summary,
            NetSummary::Available {
                kind: CostBasisKind::Scenario,
                wins: 1,
                losses: 1,
                breakeven: 1,
                win_rate: Some(rate),
                ..
            } if rate == 0.5
        ));
    }

    fn threshold_rows(cycles: usize) -> Vec<EconomicFillRow> {
        let mut rows = Vec::with_capacity(cycles * 2);
        for index in 0..cycles {
            rows.push(fill(
                i64::try_from(index + 1).unwrap(),
                &format!("TEST_CODE_{index:06}"),
                "buy",
                10.0,
                100,
                "2026-01-01 10:00:00",
                "Momentum",
            ));
        }
        for index in 0..cycles {
            rows.push(fill(
                i64::try_from(cycles + index + 1).unwrap(),
                &format!("TEST_CODE_{index:06}"),
                "sell",
                11.0,
                100,
                "2026-03-25 10:00:00",
                "BR-234四大铁律卖出",
            ));
        }
        rows
    }

    #[test]
    fn br248_requires_two_hundred_closed_positions_and_eighty_four_days() {
        let rows = threshold_rows(200);
        let ledger = complete_costs(CostBasisKind::Scenario, &rows, 0.0);
        let report = rebuild_economic_positions(&rows, date("2026-03-25"), Some(&ledger)).unwrap();
        assert_eq!(report.coverage_days, Some(84));
        assert!(matches!(
            report.validation_status,
            ValidationStatus::ResearchOnly { .. }
        ));

        let rows = threshold_rows(199);
        let ledger = complete_costs(CostBasisKind::Scenario, &rows, 0.0);
        let report = rebuild_economic_positions(&rows, date("2026-03-25"), Some(&ledger)).unwrap();
        assert!(matches!(
            report.validation_status,
            ValidationStatus::InsufficientSample {
                closed_positions: 199,
                coverage_days: Some(84),
                ..
            }
        ));
    }
}
