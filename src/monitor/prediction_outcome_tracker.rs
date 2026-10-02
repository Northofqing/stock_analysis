//! Read-only maturity-session reports. Recorded outcomes and original physical
//! card acceptance remain distinct; this module never settles or sends a row.
use super::{completed_session_as_of_at, shanghai_now, verifier::recorded_direction_hit};
use crate::database::p05_prediction_freeze::{
    OutcomePredictionSnapshot, RecordedOutcomeRow, OUTCOME_REPORT_MAX_ROWS,
};
use crate::database::DatabaseManager;
use crate::durable_delivery::{
    CandidateBoardCardObservationV2, DurableDeliveryCoordinator, P05ChildReceiptObservation,
};
use crate::p05_auction_unit::read_p05_outcome_unit;
use crate::p05_candidate_board_link::link_candidate_board_snapshot;
use chrono::{DateTime, FixedOffset, NaiveDate};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeSampleCounts {
    pub due_samples: usize,
    pub recorded_samples: usize,
    pub hits: usize,
    pub pending_samples: usize,
    pub rate: Option<f64>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalLinkedOutcomeCounts {
    /// One original CandidateBoard receipt can cover several samples.
    pub physically_accepted_cards: usize,
    pub covered_samples: usize,
    pub awaiting_drain_samples: usize,
    pub recorded_samples: usize,
    pub hits: usize,
    pub pending_samples: usize,
    pub rate: Option<f64>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum PhysicalLinkedOutcomeObservation {
    Observed(PhysicalLinkedOutcomeCounts),
    Unavailable { reason: &'static str },
}
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomePeriodObservation {
    pub as_of: NaiveDate,
    pub window_start: NaiveDate,
    pub trading_sessions: usize,
    pub observed: OutcomeSampleCounts,
    pub physical_linked: PhysicalLinkedOutcomeObservation,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeDailyWeeklyObservation {
    pub daily: OutcomePeriodObservation,
    pub weekly: OutcomePeriodObservation,
}
impl OutcomeDailyWeeklyObservation {
    pub fn render(&self) -> String {
        fn rate(value: Option<f64>) -> String {
            value
                .map(|v| format!("{:.1}%", v * 100.))
                .unwrap_or_else(|| "样本不足".into())
        }
        let mut lines = Vec::new();
        for (label, period) in [("日", &self.daily), ("周(5交易日)", &self.weekly)] {
            let linked = match &period.physical_linked {
                PhysicalLinkedOutcomeObservation::Unavailable { reason } => {
                    format!("原物理关联不可用 reason={reason}")
                }
                PhysicalLinkedOutcomeObservation::Observed(c) => format!(
                    "原Accepted卡{} / 原成员{} / 待drain{} / 已记录结果{} / 待结果{} / 命中率{}",
                    c.physically_accepted_cards,
                    c.covered_samples,
                    c.awaiting_drain_samples,
                    c.recorded_samples,
                    c.pending_samples,
                    rate(c.rate)
                ),
            };
            lines.push(format!("[OutcomeTracker][{label}] 冻结到期交易日 {}..={} 观察样本{} / 已记录结果{} / 待结果{} / 命中率{} | {}；非执行结算/ReviewTask完成",period.window_start,period.as_of,period.observed.due_samples,period.observed.recorded_samples,period.observed.pending_samples,rate(period.observed.rate),linked));
        }
        lines.join("\n")
    }
}

/// Dependencies are the existing owners, never a caller-supplied receipt.
pub struct OutcomeTracker<'a> {
    prediction: &'a DatabaseManager,
    counted: Option<&'a DurableDeliveryCoordinator>,
}
impl<'a> OutcomeTracker<'a> {
    pub fn new(
        prediction: &'a DatabaseManager,
        counted: Option<&'a DurableDeliveryCoordinator>,
    ) -> Self {
        Self {
            prediction,
            counted,
        }
    }
    /// Production owns its clock; this read grants no completion or future proof.
    pub fn read_daily_weekly(&self) -> Result<OutcomeDailyWeeklyObservation, &'static str> {
        self.read_at(shanghai_now())
    }
    #[cfg(test)]
    pub(crate) fn read_at_for_test(
        &self,
        now: DateTime<FixedOffset>,
    ) -> Result<OutcomeDailyWeeklyObservation, &'static str> {
        if !self.prediction.has_isolated_p05_consumer_origin() {
            return Err("outcome_test_origin_required");
        }
        self.read_at(now)
    }
    fn read_at(
        &self,
        now: DateTime<FixedOffset>,
    ) -> Result<OutcomeDailyWeeklyObservation, &'static str> {
        let as_of = completed_session_as_of_at(now).map_err(|_| "outcome_calendar_unavailable")?;
        let mut days = vec![as_of.to_string()];
        let mut cursor = as_of;
        for _ in 1..5 {
            cursor = crate::calendar::verified_prev_a_share_trading_day(cursor)
                .map_err(|_| "outcome_calendar_unavailable")?;
            days.push(cursor.to_string());
        }
        days.reverse();
        let snapshot = self
            .prediction
            .read_outcome_prediction_window(&days)
            .map_err(|_| "outcome_prediction_snapshot_unavailable")?;
        // Validate observed results even if no counted runtime exists.
        let observed_week = sample_counts(&snapshot.rows, |_| true)?;
        let observed_day = sample_counts(&snapshot.rows, |r| r.target_date == as_of.to_string())?;
        let linked = self.read_linked(&snapshot, &days);
        // ALL durable/card hooks have ended. The actual operational tail is not
        // an immutable JSON replay, and cannot run another durable callback.
        let tail = self
            .prediction
            .read_outcome_prediction_window(&days)
            .map_err(|_| "outcome_prediction_tail_unavailable")?;
        if tail != snapshot {
            return Err("outcome_prediction_changed_at_tail");
        }
        let component = |daily: bool| match &linked {
            Ok(members) => PhysicalLinkedOutcomeObservation::Observed(linked_counts(
                &snapshot.rows,
                members,
                |r| !daily || r.target_date == as_of.to_string(),
            )),
            Err(reason) => PhysicalLinkedOutcomeObservation::Unavailable { reason },
        };
        Ok(OutcomeDailyWeeklyObservation {
            daily: OutcomePeriodObservation {
                as_of,
                window_start: as_of,
                trading_sessions: 1,
                observed: observed_day,
                physical_linked: component(true),
            },
            weekly: OutcomePeriodObservation {
                as_of,
                window_start: cursor,
                trading_sessions: 5,
                observed: observed_week,
                physical_linked: component(false),
            },
        })
    }
    fn read_linked(
        &self,
        snapshot: &OutcomePredictionSnapshot,
        days: &[String],
    ) -> Result<BTreeMap<i64, LinkedMember>, &'static str> {
        let counted = self.counted.ok_or("outcome_counted_cache_absent")?;
        // The five inverse T+5 dates route actual Unit reads, including completed
        // and incomplete Units. No route date creates a target or a sample.
        let mut dates = BTreeSet::new();
        for day in days {
            let mut origin = NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .map_err(|_| "outcome_calendar_unavailable")?;
            for _ in 0..5 {
                origin = crate::calendar::verified_prev_a_share_trading_day(origin)
                    .map_err(|_| "outcome_calendar_unavailable")?;
            }
            dates.insert(origin.to_string());
        }
        for freeze in &snapshot.freezes {
            dates.insert(freeze.business_date().to_owned());
        }
        if dates.len() > 10 {
            return Err("outcome_routing_budget_exceeded");
        }
        let mut units = Vec::new();
        let mut cards = BTreeMap::new();
        for date in dates {
            let unit = read_p05_outcome_unit(counted, self.prediction, &date)
                .map_err(|_| "outcome_unit_context_unavailable")?;
            if unit.preparing {
                return Err("outcome_unit_preparation_pending");
            }
            let observed = counted
                .candidate_board_card_observations_with_source_for_date(&date)
                .map_err(|_| "outcome_card_snapshot_unavailable")?;
            if cards.values().map(Vec::len).sum::<usize>() + observed.len()
                > OUTCOME_REPORT_MAX_ROWS as usize
            {
                return Err("outcome_card_budget_exceeded");
            }
            cards.insert(date, observed);
            units.push(unit);
        }
        let flat: Vec<_> = cards.values().flatten().collect();
        let drains = counted
            .read_outcome_card_drains(&flat)
            .map_err(|_| "outcome_drain_snapshot_unavailable")?;
        let mut members = BTreeMap::new();
        for unit in &units {
            let observed = &cards[&unit.date];
            if let Some(board) = &unit.board {
                // Only the exact original board, never AuctionRepush/T08.
                let card = observed
                    .iter()
                    .find(|c| c.card().decision_identity() == board.decision_identity);
                if let Some(card) = card {
                    if card.card().envelope_sha256() != board.envelope_sha256
                        || card.card().source_binding_sha256() != board.source_sha256
                    {
                        return Err("outcome_unit_board_binding_mismatch");
                    }
                }
                if observed
                    .iter()
                    .any(|c| c.card().decision_identity() != board.decision_identity)
                {
                    return Err("outcome_unit_extra_card");
                }
                let physical=board.receipts.iter().find(|r|matches!(r,P05ChildReceiptObservation::PhysicallyAccepted{child_identity,decision_identity,..} if child_identity==&board.child_identity&&decision_identity==&board.decision_identity));
                if let Some(target) = &board.target_date {
                    if !days.contains(target) {
                        return Err("outcome_unit_target_mismatch");
                    }
                    // The actual DB snapshot contains every original member.
                    for (id, code, score) in &board.rows {
                        let row = snapshot
                            .rows
                            .iter()
                            .find(|r| r.id == *id)
                            .ok_or("outcome_unit_row_missing")?;
                        if row.code.as_deref() != Some(code)
                            || row.target_date != *target
                            || row.pred_date != unit.date
                            || row.score_bits != Some(*score)
                        {
                            return Err("outcome_unit_row_binding_mismatch");
                        }
                    }
                    if physical.is_some() {
                        let card = card.ok_or("outcome_unit_physical_card_missing")?;
                        if !card.card().is_authoritative_accepted_card() {
                            return Err("outcome_unit_terminal_mismatch");
                        }
                        let awaiting = board.awaiting_drain
                            || *drains
                                .get(&board.decision_identity)
                                .ok_or("outcome_drain_member_missing")?;
                        for (id, _, _) in &board.rows {
                            insert_member(&mut members, *id, &board.decision_identity, awaiting)?;
                        }
                    }
                } // NoStrong keeps its original absence; never gains later rows.
            } else if unit.draft_identity.is_some() {
                return Err("outcome_unit_unknown_membership");
            } else {
                for card in observed {
                    let freeze = snapshot
                        .freezes
                        .iter()
                        .find(|f| f.occurrence_identity() == card.card().occurrence_identity());
                    let link = link_candidate_board_snapshot(
                        Some(card),
                        freeze,
                        &unit.date,
                        card.card().occurrence_identity(),
                    )
                    .map_err(|_| "outcome_legacy_link_unavailable")?;
                    if let Some(rows) = link.accepted_rows() {
                        let awaiting = *drains
                            .get(card.card().decision_identity())
                            .ok_or("outcome_drain_member_missing")?;
                        for row in rows {
                            insert_member(
                                &mut members,
                                row.prediction_row_id(),
                                card.card().decision_identity(),
                                awaiting,
                            )?;
                        }
                    }
                }
            }
        }
        // Every later card core can have a hook. Compare complete sets, then
        // validate all Unit None/Some + original card/receipt/drain in one
        // no-hook routing snapshot. No SQL core follows that tail.
        for (date, expected) in &cards {
            let actual = counted
                .candidate_board_card_observations_with_source_for_date(date)
                .map_err(|_| "outcome_card_tail_unavailable")?;
            if actual != *expected {
                return Err("outcome_card_changed_at_tail");
            }
        }
        counted
            .validate_outcome_report_tail(&units, &flat, &drains)
            .map_err(|_| "outcome_sql_binding_changed_at_tail")?;
        Ok(members)
    }
}

#[derive(Debug)]
struct LinkedMember {
    decision: String,
    awaiting: bool,
}
fn insert_member(
    members: &mut BTreeMap<i64, LinkedMember>,
    id: i64,
    decision: &str,
    awaiting: bool,
) -> Result<(), &'static str> {
    if let Some(existing) = members.get(&id) {
        if existing.decision != decision || existing.awaiting != awaiting {
            return Err("outcome_row_has_conflicting_card_owner");
        }
    } else {
        members.insert(
            id,
            LinkedMember {
                decision: decision.into(),
                awaiting,
            },
        );
    }
    Ok(())
}
fn recorded_hit(row: &RecordedOutcomeRow) -> Result<Option<bool>, &'static str> {
    match (row.actual_change_bits, row.hit) {
        (_, None) => Ok(None),
        (Some(bits), Some(hit)) if matches!(hit, 0 | 1) => {
            let expected = recorded_direction_hit(&row.direction, f64::from_bits(bits))
                .map_err(|_| "outcome_recorded_direction_invalid")?;
            if expected != (hit == 1) {
                return Err("outcome_recorded_hit_mismatch");
            }
            Ok(Some(expected))
        }
        _ => Err("outcome_recorded_result_invalid"),
    }
}
fn sample_counts(
    rows: &[RecordedOutcomeRow],
    select: impl Fn(&RecordedOutcomeRow) -> bool,
) -> Result<OutcomeSampleCounts, &'static str> {
    let mut c = OutcomeSampleCounts {
        due_samples: 0,
        recorded_samples: 0,
        hits: 0,
        pending_samples: 0,
        rate: None,
    };
    for row in rows.iter().filter(|r| select(r)) {
        c.due_samples += 1;
        match recorded_hit(row)? {
            Some(hit) => {
                c.recorded_samples += 1;
                c.hits += usize::from(hit)
            }
            None => c.pending_samples += 1,
        }
    }
    if c.recorded_samples > 0 {
        c.rate = Some(c.hits as f64 / c.recorded_samples as f64);
    }
    Ok(c)
}
fn linked_counts(
    rows: &[RecordedOutcomeRow],
    members: &BTreeMap<i64, LinkedMember>,
    select: impl Fn(&RecordedOutcomeRow) -> bool,
) -> PhysicalLinkedOutcomeCounts {
    let mut c = PhysicalLinkedOutcomeCounts {
        physically_accepted_cards: 0,
        covered_samples: 0,
        awaiting_drain_samples: 0,
        recorded_samples: 0,
        hits: 0,
        pending_samples: 0,
        rate: None,
    };
    let mut cards = BTreeSet::new();
    for row in rows.iter().filter(|r| select(r)) {
        if let Some(member) = members.get(&row.id) {
            cards.insert(&member.decision);
            c.covered_samples += 1;
            if member.awaiting {
                c.awaiting_drain_samples += 1;
            } else {
                match recorded_hit(row)
                    .expect("same immutable rows validated in observed component")
                {
                    Some(hit) => {
                        c.recorded_samples += 1;
                        c.hits += usize::from(hit)
                    }
                    None => c.pending_samples += 1,
                }
            }
        }
    }
    c.physically_accepted_cards = cards.len();
    if c.recorded_samples > 0 {
        c.rate = Some(c.hits as f64 / c.recorded_samples as f64);
    }
    c
}
