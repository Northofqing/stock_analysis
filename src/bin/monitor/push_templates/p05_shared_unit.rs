//! One acquisition for the legacy auction candidate children.
//! These are dispatch observations, never a durable shared-Unit completion.

use super::*;
use crate::durable_delivery_runtime::CountedDeliveryBinding;
use crate::notify::{PushKind, PushOutcome};
use std::collections::BTreeSet;
use std::future::Future;
use stock_analysis::monitor::prediction::{
    prepare_candidate_board, CandidateBoardPreparation, CandidateBoardPreparationRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CandidateChildObservation {
    Observed(PushOutcome),
    NotPrepared(String),
}

impl CandidateChildObservation {
    pub(crate) fn was_pushed(&self) -> bool {
        matches!(self, Self::Observed(outcome) if outcome.is_pushed())
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Observed(PushOutcome::Pushed) => "pushed",
            Self::Observed(PushOutcome::Deduped) => "deduped",
            Self::Observed(PushOutcome::Denied(_)) => "denied",
            Self::Observed(PushOutcome::SinkError(_)) => "sink_error",
            Self::NotPrepared(_) => "not_prepared",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InvalidatedObservation {
    NoPreviousSnapshot,
    EmptyObservedDifference,
    Items(Vec<(String, CandidateChildObservation)>),
    NotPrepared(String),
}

impl InvalidatedObservation {
    pub(crate) fn observed_count(&self) -> usize {
        match self {
            Self::Items(items) => items.len(),
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateUnitDispatchObservation {
    pub(crate) auction_repush: CandidateChildObservation,
    pub(crate) candidate_board: CandidateChildObservation,
    pub(crate) invalidated: InvalidatedObservation,
}

impl CandidateUnitDispatchObservation {
    fn unavailable(reason: String) -> Self {
        Self {
            auction_repush: CandidateChildObservation::NotPrepared(reason.clone()),
            candidate_board: CandidateChildObservation::NotPrepared(reason.clone()),
            invalidated: InvalidatedObservation::NotPrepared(reason),
        }
    }
}

struct CapturedCandidateSlot {
    business_date: chrono::NaiveDate,
    date: String,
    hhmm: String,
    hhmmss: String,
}

impl CapturedCandidateSlot {
    fn new(date: &str, captured_at: chrono::DateTime<chrono::FixedOffset>) -> Result<Self, String> {
        let (business_date, hhmm) = candidate_board_slot_at(date, captured_at)?;
        Ok(Self {
            business_date,
            date: date.to_owned(),
            hhmm,
            hhmmss: captured_at.format("%H:%M:%S").to_string(),
        })
    }
}

pub(super) struct PreparedCandidateChildDelivery {
    pub(super) token: crate::presentation_registry::ProductionPresentationToken,
    pub(super) text: String,
    pub(super) binding: CountedDeliveryBinding,
}

fn prepare_registered_child(
    kind: PushKind,
    text: String,
    binding: Result<CountedDeliveryBinding, String>,
) -> Result<PreparedCandidateChildDelivery, String> {
    let (family, producer, renderer) = match kind {
        PushKind::AuctionRepush => (
            "A-02-auction-repush",
            "auction_repush_dispatcher",
            "render_auction_repush",
        ),
        PushKind::CandidateBoard => (
            "P-05-candidate-board",
            "candidate_board_dispatcher",
            "format_candidate_board",
        ),
        PushKind::CandidateInvalidated => (
            "T-08-candidate-invalidated",
            "candidate_dispatcher",
            "render_candidate_invalidated",
        ),
        _ => return Err("p05_child_kind_invalid".to_owned()),
    };
    let binding = binding?;
    let token = crate::presentation_registry::acquire_token(family, kind, producer, renderer)?;
    Ok(PreparedCandidateChildDelivery {
        token,
        text,
        binding,
    })
}

async fn send_registered_child(child: PreparedCandidateChildDelivery) -> PushOutcome {
    crate::notify::push_counted_with_binding(child.token, &child.text, None, child.binding).await
}

pub(super) async fn dispatch(
    date: &str,
    captured_at: chrono::DateTime<chrono::FixedOffset>,
) -> CandidateUnitDispatchObservation {
    dispatch_with(
        date,
        captured_at,
        load_real_candidate_batch,
        send_registered_child,
        || candidate_snapshot_previous(date),
        |codes| candidate_snapshot_persist(date, codes),
    )
    .await
}

/// The acquisition seam and local effects are shared by production and tests.
/// Test replacements do not grant source or physical receipt authority.
pub(super) async fn dispatch_with<A, AF, D, DF, R, W>(
    date: &str,
    captured_at: chrono::DateTime<chrono::FixedOffset>,
    acquire: A,
    mut deliver: D,
    previous_snapshot: R,
    persist_snapshot: W,
) -> CandidateUnitDispatchObservation
where
    A: FnOnce() -> AF,
    AF: Future<Output = Result<RealCandidateBatch, String>>,
    D: FnMut(PreparedCandidateChildDelivery) -> DF,
    DF: Future<Output = PushOutcome>,
    R: FnOnce() -> Option<BTreeSet<String>>,
    W: FnOnce(&BTreeSet<String>),
{
    let slot = match CapturedCandidateSlot::new(date, captured_at) {
        Ok(slot) => slot,
        Err(reason) => return CandidateUnitDispatchObservation::unavailable(reason),
    };
    let batch = match acquire().await {
        Ok(batch) => batch,
        Err(error) => {
            log::warn!("[MU-auction-candidates] candidate source unavailable: {error}");
            return CandidateUnitDispatchObservation::unavailable(
                "p05_source_unavailable".to_owned(),
            );
        }
    };
    let auction_repush =
        dispatch_auction_from_batch(slot.business_date, &slot.hhmmss, &batch, &mut deliver).await;
    let board = dispatch_board_from_batch(
        &slot,
        &batch,
        &mut deliver,
        previous_snapshot,
        persist_snapshot,
    )
    .await;
    CandidateUnitDispatchObservation {
        auction_repush,
        candidate_board: board.board,
        invalidated: board.invalidated,
    }
}

fn auction_top5(
    batch: &RealCandidateBatch,
) -> Vec<stock_analysis::opportunity::candidate_panel::CandidateEntry> {
    use stock_analysis::opportunity::candidate_panel::EvidenceTier;
    let mut ranked: Vec<_> = batch
        .entries
        .iter()
        .filter(|entry| {
            entry
                .current_price
                .is_some_and(|price| price.is_finite() && price > 0.0)
                && entry.heat_score.is_some_and(|heat| heat.is_finite())
        })
        .cloned()
        .collect();
    ranked.sort_by(|a, b| {
        let tier_a = a.tier == EvidenceTier::Strong;
        let tier_b = b.tier == EvidenceTier::Strong;
        tier_b
            .cmp(&tier_a)
            .then_with(|| match (b.heat_score, a.heat_score) {
                (Some(heat_b), Some(heat_a)) => heat_b.total_cmp(&heat_a),
                _ => std::cmp::Ordering::Equal,
            })
    });
    ranked.into_iter().take(5).collect()
}

async fn dispatch_auction_from_batch<D, DF>(
    business_date: chrono::NaiveDate,
    hhmmss: &str,
    batch: &RealCandidateBatch,
    deliver: &mut D,
) -> CandidateChildObservation
where
    D: FnMut(PreparedCandidateChildDelivery) -> DF,
    DF: Future<Output = PushOutcome>,
{
    if batch.entries.is_empty() {
        log_dispatcher_attempt("A-02", false, 0, "no candidates at auction");
        return CandidateChildObservation::NotPrepared("p05_no_candidates".to_owned());
    }
    let eligible = batch
        .entries
        .iter()
        .filter(|entry| {
            entry
                .current_price
                .is_some_and(|price| price.is_finite() && price > 0.0)
                && entry.heat_score.is_some_and(|heat| heat.is_finite())
        })
        .count();
    let excluded = batch.entries.len().saturating_sub(eligible);
    if excluded > 0 {
        log::warn!(
            "[A-02][BR-223] excluded {excluded} candidate(s) with missing/invalid price or heat"
        );
    }
    let top5 = auction_top5(batch);
    if top5.is_empty() {
        log_dispatcher_attempt("A-02", false, 0, "no priced candidates at auction");
        return CandidateChildObservation::NotPrepared("p05_no_priced_candidates".to_owned());
    }
    let text = render_auction_repush(hhmmss, &top5);
    let binding = build_auction_repush_counted_binding(business_date, hhmmss, &text);
    let observation = match prepare_registered_child(PushKind::AuctionRepush, text, binding) {
        Ok(child) => CandidateChildObservation::Observed(deliver(child).await),
        Err(reason) => {
            log::error!("[A-02][BR-196] counted 准备失败: {reason}");
            CandidateChildObservation::NotPrepared(reason)
        }
    };
    log_dispatcher_attempt("A-02", observation.was_pushed(), top5.len(), "");
    observation
}

struct BoardDispatchObservation {
    board: CandidateChildObservation,
    invalidated: InvalidatedObservation,
}

async fn dispatch_board_from_batch<D, DF, R, W>(
    slot: &CapturedCandidateSlot,
    batch: &RealCandidateBatch,
    deliver: &mut D,
    previous_snapshot: R,
    persist_snapshot: W,
) -> BoardDispatchObservation
where
    D: FnMut(PreparedCandidateChildDelivery) -> DF,
    DF: Future<Output = PushOutcome>,
    R: FnOnce() -> Option<BTreeSet<String>>,
    W: FnOnce(&BTreeSet<String>),
{
    use stock_analysis::opportunity::candidate_panel::EvidenceTier;
    if batch.entries.is_empty() {
        log_dispatcher_attempt("P-05", false, 0, "no candidates");
        return BoardDispatchObservation {
            board: CandidateChildObservation::NotPrepared("p05_no_candidates".to_owned()),
            invalidated: InvalidatedObservation::NotPrepared("p05_no_candidates".to_owned()),
        };
    }
    let codes_now = batch
        .entries
        .iter()
        .map(|entry| entry.code.clone())
        .collect::<BTreeSet<_>>();
    let invalidated = match previous_snapshot() {
        None => InvalidatedObservation::NoPreviousSnapshot,
        Some(previous) => {
            let mut outcomes = Vec::new();
            for code in previous.difference(&codes_now) {
                let name = batch
                    .entries
                    .iter()
                    .find(|entry| &entry.code == code)
                    .map(|entry| entry.name.as_str())
                    .unwrap_or(code.as_str());
                let observation = dispatch_invalidated_with(
                    slot.business_date,
                    code,
                    &slot.hhmmss,
                    name,
                    "候选",
                    "从候选台消失",
                    deliver,
                )
                .await;
                outcomes.push((code.clone(), observation));
            }
            if outcomes.is_empty() {
                InvalidatedObservation::EmptyObservedDifference
            } else {
                InvalidatedObservation::Items(outcomes)
            }
        }
    };
    let samples = batch
        .entries
        .iter()
        .filter(|entry| entry.tier == EvidenceTier::Strong && entry.current_price.is_some())
        .map(|entry| (entry.code.clone(), entry.heat_score.unwrap_or(50.0)))
        .collect();
    let text = stock_analysis::opportunity::candidate_panel::format_candidate_board(&batch.entries);
    let preparation = match CandidateBoardPreparationRequest::new(
        &slot.date,
        &slot.hhmm,
        text.as_bytes().to_vec(),
        samples,
    ) {
        Ok(request) => prepare_candidate_board(request).await,
        Err(error) => Err(error),
    };
    let (text, binding) = match preparation {
        Err(error) => {
            if let Some(report) = error.save_report() {
                report.log();
            }
            log::warn!("[P-05] preparation blocked reason={}", error.reason());
            return BoardDispatchObservation {
                board: CandidateChildObservation::NotPrepared(error.reason().to_owned()),
                invalidated,
            };
        }
        Ok(CandidateBoardPreparation::Frozen {
            record,
            save_report,
            ..
        }) => {
            if let Some(report) = save_report {
                report.log();
            }
            let binding = build_candidate_board_counted_binding_v2(&record);
            let text = match String::from_utf8(record.rendered_bytes().to_vec()) {
                Ok(text) => text,
                Err(_) => {
                    return BoardDispatchObservation {
                        board: CandidateChildObservation::NotPrepared(
                            "p05_frozen_utf8_invalid".to_owned(),
                        ),
                        invalidated,
                    }
                }
            };
            (text, binding)
        }
        Ok(CandidateBoardPreparation::UnlinkedNoStrong) => {
            log::info!("[P-05] no sampled Strong rows; card remains UnlinkedV1");
            let binding =
                build_candidate_board_counted_binding(slot.business_date, &slot.hhmm, &text);
            (text, binding)
        }
    };
    // Preserve the legacy position: after sampling, before board counted send.
    persist_snapshot(&codes_now);
    let board = match prepare_registered_child(PushKind::CandidateBoard, text, binding) {
        Ok(child) => CandidateChildObservation::Observed(deliver(child).await),
        Err(reason) => {
            log::error!("[P-05][BR-196] counted 准备失败: {reason}");
            CandidateChildObservation::NotPrepared(reason)
        }
    };
    log_dispatcher_attempt("P-05", board.was_pushed(), batch.entries.len(), "");
    BoardDispatchObservation { board, invalidated }
}

async fn dispatch_invalidated_with<D, DF>(
    business_date: chrono::NaiveDate,
    code: &str,
    hhmmss: &str,
    name: &str,
    prev: &str,
    reason: &str,
    deliver: &mut D,
) -> CandidateChildObservation
where
    D: FnMut(PreparedCandidateChildDelivery) -> DF,
    DF: Future<Output = PushOutcome>,
{
    let text = render_candidate_invalidated(hhmmss, name, code, prev, reason);
    let binding =
        build_candidate_invalidated_counted_binding(business_date, code, prev, reason, &text);
    match prepare_registered_child(PushKind::CandidateInvalidated, text, binding) {
        Ok(child) => CandidateChildObservation::Observed(deliver(child).await),
        Err(reason) => {
            log::error!("[T-08][BR-196] counted 准备失败: {reason}");
            CandidateChildObservation::NotPrepared(reason)
        }
    }
}

pub(super) async fn dispatch_auction_repush_compat(hhmm: &str) -> CandidateChildObservation {
    match load_real_candidate_batch().await {
        Ok(batch) => {
            dispatch_auction_from_batch(
                chrono::Local::now().date_naive(),
                hhmm,
                &batch,
                &mut send_registered_child,
            )
            .await
        }
        Err(error) => {
            log::warn!("[A-02][BR-223] 候选源不可用: {error}");
            CandidateChildObservation::NotPrepared("p05_source_unavailable".to_owned())
        }
    }
}

pub(super) async fn dispatch_candidate_board_compat(date: &str) -> CandidateChildObservation {
    let slot =
        match CapturedCandidateSlot::new(date, stock_analysis::monitor::prediction::shanghai_now())
        {
            Ok(slot) => slot,
            Err(reason) => {
                log::warn!("[P-05] preparation blocked reason={reason}");
                return CandidateChildObservation::NotPrepared(reason);
            }
        };
    let batch = match load_real_candidate_batch().await {
        Ok(batch) => batch,
        Err(error) => {
            log::warn!("[P-05][BR-223] 候选源不可用: {error}");
            return CandidateChildObservation::NotPrepared("p05_source_unavailable".to_owned());
        }
    };
    dispatch_board_from_batch(
        &slot,
        &batch,
        &mut send_registered_child,
        || candidate_snapshot_previous(date),
        |codes| candidate_snapshot_persist(date, codes),
    )
    .await
    .board
}

pub(super) async fn dispatch_invalidated_compat(
    business_date: chrono::NaiveDate,
    code: &str,
    hhmm: &str,
    name: &str,
    prev: &str,
    reason: &str,
) -> CandidateChildObservation {
    dispatch_invalidated_with(
        business_date,
        code,
        hhmm,
        name,
        prev,
        reason,
        &mut send_registered_child,
    )
    .await
}

#[cfg(test)]
#[path = "p05_shared_unit_tests.rs"]
pub(super) mod tests;
