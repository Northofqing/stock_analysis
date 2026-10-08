//! One original observed batch becomes a complete immutable P05 Unit before
//! presentation or counted admission. Saved Units recover without new facts.

use super::*;
#[cfg(test)]
use crate::durable_delivery_runtime::CountedDeliveryBinding;
use crate::notify::PushKind;
#[cfg(test)]
use crate::notify::PushOutcome;
use std::collections::BTreeSet;
#[cfg(test)]
use std::future::Future;
use std::sync::Arc;
#[cfg(test)]
use stock_analysis::monitor::prediction::{
    prepare_candidate_board, CandidateBoardPreparation, CandidateBoardPreparationRequest,
};
use stock_analysis::p05_auction_unit::{
    P05AuctionUnit, P05ObservedBatchSourceBytes, P05UnitObservation,
};

pub(super) async fn initialize_before_window() {
    if !crate::durable_delivery_runtime::frozen_platform_features_enabled() {
        log::info!("[P05 Unit] disabled reason=user_scope_frozen; provider/sql/file operations=0");
        return;
    }
    if let Err(e) = crate::durable_delivery_runtime::initialize_p05_family_before_window().await {
        log::warn!("[P05 Unit] prospective initialization unavailable: {e}");
    }
}

/// Saved recovery is deliberately independent of the current auction window,
/// source availability and the market-active waiting loop.
pub(super) async fn tick(allow_fresh: bool) -> Result<(), String> {
    if !crate::durable_delivery_runtime::frozen_platform_features_enabled() {
        return Ok(());
    }
    let runtime = crate::durable_delivery_runtime::inspect_unfinished_p05_units().await?;
    let restored_dates = recover_saved_with(
        runtime
            .into_iter()
            .map(|stored| stored.business_date().to_owned()),
        |date| async move {
            let unit = crate::durable_delivery_runtime::restore_p05_unit(date.clone())
                .await?
                .ok_or_else(|| format!("original stored P05 Unit disappeared date={date}"))?;
            advance_unit(unit).await.map(|_| ())
        },
    )
    .await?;
    let restore_clock = stock_analysis::monitor::prediction::shanghai_now();
    let date = restore_clock.date_naive().to_string();
    if restored_dates.contains(&date) {
        return Ok(());
    }
    if let Some(unit) = crate::durable_delivery_runtime::restore_p05_unit(date.clone()).await? {
        advance_unit(unit).await?;
        return Ok(());
    }
    let captured = stock_analysis::monitor::prediction::shanghai_now();
    if captured.date_naive().to_string() != date {
        return Err("P05 date changed after original Unit absence read".to_owned());
    }
    if !allow_fresh || !fresh_auction_window(captured)? {
        return Ok(());
    }
    let unit = prepare_once_with(&date, captured, load_real_candidate_batch, |prepared| {
        crate::durable_delivery_runtime::observe_p05_batch(
            captured,
            prepared.entries,
            prepared.source,
            prepared.auction_rendered,
            prepared.board_rendered,
        )
    })
    .await?;
    advance_unit(unit).await?;
    Ok(())
}

// These seams carry scheduling/ordinary observation only. They cannot create
// a stored Unit, qualification or completion without the real owner callback.
async fn recover_saved_with<Dates, Recover, RecoverFuture>(
    dates: Dates,
    mut recover: Recover,
) -> Result<BTreeSet<String>, String>
where
    Dates: IntoIterator<Item = String>,
    Recover: FnMut(String) -> RecoverFuture,
    RecoverFuture: std::future::Future<Output = Result<(), String>>,
{
    let mut restored = BTreeSet::new();
    let mut errors = Vec::new();
    for date in dates {
        restored.insert(date.clone());
        if let Err(e) = recover(date.clone()).await {
            errors.push(format!("original P05 Unit date={date}: {e}"));
        }
    }
    if errors.is_empty() {
        Ok(restored)
    } else {
        Err(errors.join("; "))
    }
}

async fn prepare_once_with<Acquire, AcquireFuture, Persist, PersistFuture, Stored>(
    date: &str,
    captured: chrono::DateTime<chrono::FixedOffset>,
    acquire: Acquire,
    persist: Persist,
) -> Result<Stored, String>
where
    Acquire: FnOnce() -> AcquireFuture,
    AcquireFuture: std::future::Future<Output = Result<RealCandidateBatch, String>>,
    Persist: FnOnce(PreparedObservedBatch) -> PersistFuture,
    PersistFuture: std::future::Future<Output = Result<Stored, String>>,
{
    // Validate the captured date before source work. Fresh production-window
    // authority remains with the actual immutable owner, never this callback.
    CapturedCandidateSlot::new(date, captured)?;
    let batch = acquire().await?;
    persist(prepare_observed_batch(date, captured, batch)?).await
}

fn fresh_auction_window(now: chrono::DateTime<chrono::FixedOffset>) -> Result<bool, String> {
    let local = now.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
    Ok(
        stock_analysis::calendar::verified_a_share_trading_day(local.date_naive())?
            && local.time() >= chrono::NaiveTime::from_hms_opt(9, 20, 0).unwrap()
            && local.time() < chrono::NaiveTime::from_hms_opt(9, 25, 0).unwrap(),
    )
}

async fn advance_unit(unit: Arc<P05AuctionUnit>) -> Result<P05UnitObservation, String> {
    use stock_analysis::durable_delivery::P05ChildReceiptObservation as Child;
    let observed =
        crate::durable_delivery_runtime::reconcile_p05_unit_observation(Arc::clone(&unit)).await?;
    if observed.child_observations().len() != unit.child_count() {
        return Err("P05 original receipt set does not match required children".to_owned());
    }
    for (index, child) in observed.child_observations().iter().enumerate() {
        if matches!(child, Child::PhysicallyAccepted { .. }) {
            // Original Accepted facts are passively reconciled above, never
            // retried through presentation or a new counted decision.
            continue;
        }
        let view =
            crate::durable_delivery_runtime::inspect_p05_child(Arc::clone(&unit), index).await?;
        let token = presentation_for_kind(view.envelope().push_kind)?;
        let outcome = crate::notify::push_p05_unit_child(token, view).await;
        log::info!("[P05 Unit] original child ordinal={index} dispatch observation={outcome:?}");
    }
    let current =
        crate::durable_delivery_runtime::reconcile_p05_unit_observation(Arc::clone(&unit)).await?;
    let current =
        if current.completion_identity().is_none() && all_children_physically_accepted(&current) {
            crate::durable_delivery_runtime::finalize_p05_unit(unit, current).await?
        } else {
            current
        };
    log::info!(
        "[P05 Unit] receipt observation unit={} revision={} current_completion={}",
        current.unit_identity(),
        current.mutation_revision(),
        current.completion_identity().unwrap_or("unproven")
    );
    Ok(current)
}

fn all_children_physically_accepted(observed: &P05UnitObservation) -> bool {
    !observed.child_observations().is_empty() && observed.child_observations().iter().all(|child| {
        matches!(
            child,
            stock_analysis::durable_delivery::P05ChildReceiptObservation::PhysicallyAccepted { .. }
        )
    })
}

pub(super) fn presentation_for_kind(
    kind: stock_analysis::durable_delivery::PushKind,
) -> Result<crate::presentation_registry::ProductionPresentationToken, String> {
    use stock_analysis::durable_delivery::PushKind as DurableKind;
    let (family, kind, producer, renderer) = match kind {
        DurableKind::AuctionRepush => (
            "A-02-auction-repush",
            PushKind::AuctionRepush,
            "auction_repush_dispatcher",
            "render_auction_repush",
        ),
        DurableKind::CandidateBoard => (
            "P-05-candidate-board",
            PushKind::CandidateBoard,
            "candidate_board_dispatcher",
            "format_candidate_board",
        ),
        DurableKind::CandidateInvalidated => (
            "T-08-candidate-invalidated",
            PushKind::CandidateInvalidated,
            "candidate_dispatcher",
            "render_candidate_invalidated",
        ),
        _ => return Err("p05_child_kind_invalid".to_owned()),
    };
    crate::presentation_registry::acquire_token(family, kind, producer, renderer)
}

struct PreparedObservedBatch {
    entries: Vec<stock_analysis::opportunity::candidate_panel::CandidateEntry>,
    source: P05ObservedBatchSourceBytes,
    auction_rendered: Vec<u8>,
    board_rendered: Vec<u8>,
}

fn prepare_observed_batch(
    date: &str,
    captured: chrono::DateTime<chrono::FixedOffset>,
    batch: RealCandidateBatch,
) -> Result<PreparedObservedBatch, String> {
    let slot = CapturedCandidateSlot::new(date, captured)?;
    if batch.entries.is_empty() {
        return Err("p05_no_candidates".to_owned());
    }
    let top5 = auction_top5(&batch);
    if top5.is_empty() {
        return Err("p05_no_priced_candidates".to_owned());
    }
    let source = observed_source_bytes(&batch)?;
    Ok(PreparedObservedBatch {
        auction_rendered: render_auction_repush(&slot.hhmmss, &top5).into_bytes(),
        board_rendered: stock_analysis::opportunity::candidate_panel::format_candidate_board(
            &batch.entries,
        )
        .into_bytes(),
        entries: batch.entries,
        source,
    })
}

/// These are exact encodings of the loader's retained decoded observations,
/// not raw SQLite/file bodies or new source qualification.
fn observed_source_bytes(
    batch: &RealCandidateBatch,
) -> Result<P05ObservedBatchSourceBytes, String> {
    use serde_json::{json, Value};
    let evidence = |e: &stock_analysis::data_gateway::BatchEvidence| json!({"provider":e.provider,"source":e.source,"source_at":e.source_at,"observed_at":e.observed_at,"batch_id":e.batch_id});
    let bytes = |v: &Value| {
        serde_json::to_vec(v)
            .map_err(|e| format!("P05 retained source observation encoding failed: {e}"))
    };
    let files:Vec<_>=batch.p5_files.iter().map(|f|json!({"source":f.source.label(),"file_identity":f.file_identity,"raw_file_sha256":f.raw_file_sha256,"generated_at":f.generated_at,"selection_version":f.selection_version,"metadata_state":format!("{:?}",f.metadata_state),"rows":f.rows.iter().map(|r|json!({"physical_line":r.physical_line,"raw_line_sha256":r.raw_line_sha256,"code":r.code,"name":r.name,"generated_at":r.generated_at,"selection_version":r.selection_version,"metadata_state":format!("{:?}",r.metadata_state)})).collect::<Vec<_>>()})).collect();
    let file_refs:Vec<_>=batch.p5_candidate_refs.iter().map(|r|json!({"code":r.code,"source":r.source.label(),"file_identity":r.file_identity,"raw_file_sha256":r.raw_file_sha256,"physical_line":r.physical_line,"raw_line_sha256":r.raw_line_sha256})).collect();
    let query = &batch.chain_query;
    let row = |r: &stock_analysis::database::concepts::ChainDailyRow| json!({"date":r.date,"concept":r.concept,"stocks":r.stocks,"continuation_count":r.continuation_count});
    let chain = json!({"schema":query.schema,"latest_date":query.latest_date,"total_rows":query.total_rows,"ordered_rows":query.ordered_rows.iter().map(row).collect::<Vec<_>>(),"ordered_rows_sha256":query.ordered_rows_sha256,"selected_rows":query.selected_rows.iter().map(|r|json!({"ordinal":r.ordinal,"row":row(&r.row),"row_sha256":r.row_sha256})).collect::<Vec<_>>(),"generation":format!("{:?}",query.generation)});
    let refs:Vec<_>=batch.chain_candidate_refs.iter().map(|r|json!({"code":r.code,"source":r.source.label(),"date":r.date,"concept":r.concept,"ordinal":r.ordinal,"row_sha256":r.row_sha256,"ordered_rows_sha256":r.ordered_rows_sha256})).collect();
    Ok(P05ObservedBatchSourceBytes {
        quote_evidence: batch
            .quote_evidence
            .as_ref()
            .map(|e| bytes(&evidence(e)))
            .transpose()?,
        statistics_evidence: batch
            .statistics_evidence
            .as_ref()
            .map(|e| bytes(&evidence(e)))
            .transpose()?,
        p5_file_witnesses: bytes(&json!(files))?,
        p5_candidate_refs: bytes(&json!(file_refs))?,
        chain_query: bytes(&chain)?,
        chain_candidate_refs: bytes(&json!(refs))?,
    })
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CandidateChildObservation {
    Observed(PushOutcome),
    NotPrepared(String),
}

#[cfg(test)]
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

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InvalidatedObservation {
    NoPreviousSnapshot,
    EmptyObservedDifference,
    Items(Vec<(String, CandidateChildObservation)>),
    NotPrepared(String),
}

#[cfg(test)]
impl InvalidatedObservation {
    pub(crate) fn observed_count(&self) -> usize {
        match self {
            Self::Items(items) => items.len(),
            _ => 0,
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateUnitDispatchObservation {
    pub(crate) auction_repush: CandidateChildObservation,
    pub(crate) candidate_board: CandidateChildObservation,
    pub(crate) invalidated: InvalidatedObservation,
}

#[cfg(test)]
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
    #[cfg(test)]
    business_date: chrono::NaiveDate,
    #[cfg(test)]
    date: String,
    #[cfg(test)]
    hhmm: String,
    hhmmss: String,
}

impl CapturedCandidateSlot {
    fn new(date: &str, captured_at: chrono::DateTime<chrono::FixedOffset>) -> Result<Self, String> {
        let (_business_date, _hhmm) = candidate_board_slot_at(date, captured_at)?;
        Ok(Self {
            #[cfg(test)]
            business_date: _business_date,
            #[cfg(test)]
            date: date.to_owned(),
            #[cfg(test)]
            hhmm: _hhmm,
            hhmmss: captured_at.format("%H:%M:%S").to_string(),
        })
    }
}

#[cfg(test)]
pub(super) struct PreparedCandidateChildDelivery {
    pub(super) token: crate::presentation_registry::ProductionPresentationToken,
    pub(super) text: String,
    pub(super) binding: CountedDeliveryBinding,
}

#[cfg(test)]
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

#[cfg(test)]
async fn send_registered_child(child: PreparedCandidateChildDelivery) -> PushOutcome {
    crate::notify::push_counted_with_binding(child.token, &child.text, None, child.binding).await
}

#[cfg(test)]
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
#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
struct BoardDispatchObservation {
    board: CandidateChildObservation,
    invalidated: InvalidatedObservation,
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
#[path = "p05_unit_runtime_tests.rs"]
mod runtime_tests;
