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
use std::sync::Arc;

const OUTCOME_FEEDBACK_DISPLAY_LIMIT: usize = 20;

/// Read-only classification; it cannot construct a receipt or row association.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeItemDeliveryStatus {
    Unlinked,
    UnlinkedV1,
    FrozenOnly,
    Pending,
    PhysicallyAccepted,
    AcceptedAwaitingDrain,
    AcceptedNotCounted,
    ManualAccepted,
    Rejected,
    Uncertain,
    ManualNotDelivered,
    Unavailable { reason: &'static str },
}

#[derive(PartialEq, Eq)]
struct LinkedOutcomeContext {
    members: BTreeMap<i64, LinkedMember>,
    cards: BTreeMap<String, Vec<CandidateBoardCardObservationV2>>,
    units: Vec<crate::p05_auction_unit::P05OutcomeUnitRead>,
}

#[derive(PartialEq, Eq)]
struct OutcomeFeedbackContext {
    snapshot: OutcomePredictionSnapshot,
    linked: Result<LinkedOutcomeContext, &'static str>,
}

/// Private indices into one complete, verified read. No caller ID/receipt factory.
#[derive(Clone, PartialEq, Eq)]
pub struct OutcomeItemFeedback {
    context: Arc<OutcomeFeedbackContext>,
    indices: Vec<usize>,
    total: usize,
}

impl std::fmt::Debug for OutcomeItemFeedback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutcomeItemFeedback")
            .field("displayed", &self.indices.len())
            .field("total", &self.total)
            .finish_non_exhaustive()
    }
}

impl OutcomeItemFeedback {
    fn new(context: Arc<OutcomeFeedbackContext>, target: Option<&str>) -> Self {
        let selected = |row: &RecordedOutcomeRow| target.is_none_or(|date| row.target_date == date);
        let total = context.snapshot.rows.iter().filter(|row| selected(row)).count();
        let indices = context.snapshot.rows.iter().enumerate()
            .filter(|(_, row)| selected(row))
            .take(OUTCOME_FEEDBACK_DISPLAY_LIMIT).map(|(index, _)| index).collect();
        Self { context, indices, total }
    }

    pub fn total(&self) -> usize { self.total }
    pub fn displayed(&self) -> usize { self.indices.len() }

    pub fn items(&self) -> impl ExactSizeIterator<Item = OutcomeItemFeedbackRow<'_>> + '_ {
        self.indices.iter().map(move |&index| OutcomeItemFeedbackRow {
            row: &self.context.snapshot.rows[index], context: &self.context,
        })
    }

    /// Historical card fact only: no v1 receipt is attached to a row.
    pub fn unlinked_v1_cards(&self) -> Option<usize> {
        self.context.linked.as_ref().ok().map(|linked| linked.cards.values().flatten()
            .filter(|card| matches!(card.source_link(), crate::durable_delivery::CandidateBoardSourceLinkV1::UnlinkedV1)).count())
    }

    /// Original NoStrong units have no target membership; no later row is adopted.
    pub fn no_strong_units(&self) -> Option<usize> {
        self.context.linked.as_ref().ok().map(|linked| linked.units.iter()
            .filter(|unit| unit.board.as_ref().is_some_and(|board| board.target_date.is_none())).count())
    }
}

/// Borrowed actual row and original opaque card. Not Clone/Deserialize/constructible.
pub struct OutcomeItemFeedbackRow<'a> {
    row: &'a RecordedOutcomeRow,
    context: &'a OutcomeFeedbackContext,
}

impl<'a> OutcomeItemFeedbackRow<'a> {
    pub fn prediction_row_id(&self) -> i64 { self.row.id }
    pub fn code(&self) -> Option<&'a str> { self.row.code.as_deref() }
    pub fn target_date(&self) -> &'a str { &self.row.target_date }
    pub fn recorded_hit(&self) -> Option<bool> { self.row.hit.map(|hit| hit == 1) }

    fn freeze(&self) -> Option<&'a crate::database::p05_prediction_freeze::FrozenCandidateBoardV2> {
        self.context.snapshot.freezes.iter().find(|freeze| freeze.ordered_rows().iter()
            .any(|member| member.prediction_row_id() == self.row.id))
    }

    pub fn frozen_occurrence_identity(&self) -> Option<&'a str> {
        self.freeze().map(|freeze| freeze.occurrence_identity())
    }

    pub fn frozen_source_sha256(&self) -> Option<&'a str> {
        self.freeze().map(|freeze| freeze.source_sha256())
    }

    pub fn original_card(&self) -> Option<&'a crate::durable_delivery::CandidateBoardCardObservationV1> {
        let linked = self.context.linked.as_ref().ok()?;
        let freeze = self.freeze()?;
        linked.cards.values().flatten().find(|observation| {
            observation.card().occurrence_identity() == freeze.occurrence_identity()
                && matches!(observation.source_link(),
                    crate::durable_delivery::CandidateBoardSourceLinkV1::DeclaredV2 { ordered_rows }
                    if ordered_rows.iter().any(|member| member.prediction_row_id() == self.row.id))
        }).map(|observation| observation.card())
    }

    pub fn delivery_status(&self) -> OutcomeItemDeliveryStatus {
        use crate::durable_delivery::CandidateBoardCardTerminalV1 as Terminal;
        let linked = match &self.context.linked {
            Ok(linked) => linked,
            Err(reason) => return OutcomeItemDeliveryStatus::Unavailable { reason },
        };
        if let Some(member) = linked.members.get(&self.row.id) {
            return if member.awaiting { OutcomeItemDeliveryStatus::AcceptedAwaitingDrain }
                else { OutcomeItemDeliveryStatus::PhysicallyAccepted };
        }
        if let Some(card) = self.original_card() {
            return match card.terminal() {
                Terminal::Pending => OutcomeItemDeliveryStatus::Pending,
                Terminal::Accepted => OutcomeItemDeliveryStatus::AcceptedNotCounted,
                Terminal::ManualAccepted => OutcomeItemDeliveryStatus::ManualAccepted,
                Terminal::Rejected => OutcomeItemDeliveryStatus::Rejected,
                Terminal::Uncertain => OutcomeItemDeliveryStatus::Uncertain,
                Terminal::ManualNotDelivered => OutcomeItemDeliveryStatus::ManualNotDelivered,
            };
        }
        if let Some(freeze) = self.freeze() {
            if linked.cards.values().flatten().any(|card| {
                card.card().occurrence_identity() == freeze.occurrence_identity()
                    && matches!(card.source_link(), crate::durable_delivery::CandidateBoardSourceLinkV1::UnlinkedV1)
            }) {
                // Same occurrence is not row membership. original_card() stays None.
                return OutcomeItemDeliveryStatus::UnlinkedV1;
            }
            return OutcomeItemDeliveryStatus::FrozenOnly;
        }
        OutcomeItemDeliveryStatus::Unlinked
    }
}

/// Conservative escaped-render extent before Arc/indices/feedback allocation.
/// Every due row is charged, including rows hidden by the display limit.
/// The original SQL 4096/16MiB preflight and full semantic/tail checks are separate.
fn feedback_extent_preflight(context: &OutcomeFeedbackContext) -> Result<(), &'static str> {
    let rows = &context.snapshot.rows;
    if rows.len() > OUTCOME_REPORT_MAX_ROWS as usize
        || rows.windows(2).any(|pair| pair[0].id >= pair[1].id)
    {
        return Err("outcome_feedback_row_extent_invalid");
    }
    let limit = crate::database::p05_prediction_freeze::OUTCOME_REPORT_MAX_BYTES as usize;
    let mut bytes = 1024usize;
    for row in rows {
        bytes = bytes.checked_add(1024).ok_or("outcome_feedback_extent_exceeded")?;
        let item = OutcomeItemFeedbackRow { row, context };
        let card = item.original_card();
        for field in [
            item.code(), Some(item.target_date()),
            item.frozen_occurrence_identity(), item.frozen_source_sha256(),
            card.map(|value| value.occurrence_identity()),
            card.map(|value| value.decision_identity()),
            card.map(|value| value.envelope_sha256()),
            card.map(|value| value.source_binding_sha256()),
            card.and_then(|value| value.terminal_attempt_identity()),
            card.and_then(|value| value.disposition_identity()),
            card.and_then(|value| value.terminal_evidence_sha256()),
            card.and_then(|value| value.accepted_channel()),
        ].into_iter().flatten() {
            // Rust Debug escaping needs at most six ASCII bytes per input byte.
            bytes = bytes.checked_add(field.len().checked_mul(6)
                .ok_or("outcome_feedback_extent_exceeded")?)
                .ok_or("outcome_feedback_extent_exceeded")?;
        }
        if bytes > limit { return Err("outcome_feedback_extent_exceeded"); }
    }
    Ok(())
}

impl OutcomePeriodObservation {
    pub fn item_feedback(&self) -> &OutcomeItemFeedback { &self.item_feedback }
}

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
    item_feedback: OutcomeItemFeedback,
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
        let label = "周窗";
        let feedback = self.weekly.item_feedback();
        let card_context = match (feedback.unlinked_v1_cards(), feedback.no_strong_units()) {
            (Some(v1), Some(no_strong)) =>
                format!("UnlinkedV1卡{v1}不关联行 / NoStrong无目标成员单元{no_strong}"),
            _ => "原卡/Unit上下文不可用；UnlinkedV1/NoStrong数量未知".into(),
        };
        lines.push(format!("[OutcomeTracker][{label}逐项] 显示{}/总{}（原row ID顺序）；{}",
            feedback.displayed(), feedback.total(), card_context));
        for item in feedback.items() {
            let result = match item.recorded_hit() {
                Some(true) => "hit",
                Some(false) => "miss",
                None => "pending",
            };
            let card = item.original_card();
            lines.push(format!("[OutcomeTracker][{label}项] row_id={} code={:?} target={} result={} link={:?} frozen_occurrence={:?} frozen_source={:?} card_occurrence={:?} decision={:?} terminal={:?} attempt={:?} disposition={:?} terminal_evidence_sha256={:?} envelope_sha256={:?} source_sha256={:?} channel={:?}",
                item.prediction_row_id(), item.code(), item.target_date(), result, item.delivery_status(),
                item.frozen_occurrence_identity(), item.frozen_source_sha256(),
                card.map(|c| c.occurrence_identity()), card.map(|c| c.decision_identity()),
                card.map(|c| c.terminal()), card.and_then(|c| c.terminal_attempt_identity()),
                card.and_then(|c| c.disposition_identity()), card.and_then(|c| c.terminal_evidence_sha256()),
                card.map(|c| c.envelope_sha256()), card.map(|c| c.source_binding_sha256()),
                card.and_then(|c| c.accepted_channel())));
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
            Ok(context) => PhysicalLinkedOutcomeObservation::Observed(linked_counts(
                &snapshot.rows,
                &context.members,
                |r| !daily || r.target_date == as_of.to_string(),
            )),
            Err(reason) => PhysicalLinkedOutcomeObservation::Unavailable { reason },
        };
        let physical_day = component(true);
        let physical_week = component(false);
        // Pure field-only projection after ALL original tails. No new callback/query.
        let feedback_context = OutcomeFeedbackContext { snapshot, linked };
        feedback_extent_preflight(&feedback_context)?;
        let feedback_context = Arc::new(feedback_context);
        let day = as_of.to_string();
        let daily_items = OutcomeItemFeedback::new(feedback_context.clone(), Some(&day));
        let weekly_items = OutcomeItemFeedback::new(feedback_context, None);
        Ok(OutcomeDailyWeeklyObservation {
            daily: OutcomePeriodObservation {
                as_of,
                window_start: as_of,
                trading_sessions: 1,
                observed: observed_day,
                physical_linked: physical_day,
                item_feedback: daily_items,
            },
            weekly: OutcomePeriodObservation {
                as_of,
                window_start: cursor,
                trading_sessions: 5,
                observed: observed_week,
                physical_linked: physical_week,
                item_feedback: weekly_items,
            },
        })
    }
    fn read_linked(
        &self,
        snapshot: &OutcomePredictionSnapshot,
        days: &[String],
    ) -> Result<LinkedOutcomeContext, &'static str> {
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
        drop(flat);
        Ok(LinkedOutcomeContext { members, cards, units })
    }
}

#[derive(Debug, PartialEq, Eq)]
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
