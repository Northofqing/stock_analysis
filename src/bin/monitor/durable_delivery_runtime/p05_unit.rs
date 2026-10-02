//! P05 local preparation/recovery and the contextual route into the sole
//! counted consumer. Every operation uses the existing runtime namespace.
use super::*;
use stock_analysis::p05_auction_unit::{
    P05AuctionUnit, P05ChildDispatchView, P05ObservedBatchSourceBytes, P05StoredUnitSnapshot,
    P05UnitObservation,
};

fn ready_p05_runtime() -> Result<Arc<RuntimeState>, String> {
    let state = runtime_state()?;
    if !state.producer_ready.load(Ordering::Acquire) {
        return Err("P05 requires the existing startup reconciliation barrier".into());
    }
    Ok(state)
}

pub(crate) async fn initialize_p05_family_before_window() -> Result<(), String> {
    let state = ready_p05_runtime()?;
    tokio::task::spawn_blocking(move || {
        let local = Utc::now().with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
        if local.time() >= chrono::NaiveTime::from_hms_opt(9, 20, 0).unwrap()
            || !stock_analysis::calendar::verified_a_share_trading_day(local.date_naive())?
        {
            return Ok(());
        }
        // The real owner owns its independent fresh production clock and the
        // actual operational/legacy/durable absence checks. This is scheduling.
        P05AuctionUnit::initialize_prospective_family(state.coordinator.as_ref())
            .map(|_| ())
            .map_err(|e| format!("initialize actual P05 prospective family: {e}"))
    })
    .await
    .map_err(|e| format!("join P05 prospective initialization: {e}"))?
}

pub(crate) async fn inspect_unfinished_p05_units() -> Result<Vec<P05StoredUnitSnapshot>, String> {
    let state = ready_p05_runtime()?;
    tokio::task::spawn_blocking(move || {
        P05AuctionUnit::inspect_unfinished(state.coordinator.as_ref())
            .map_err(|e| format!("read actual unfinished P05 Units: {e}"))
    })
    .await
    .map_err(|e| format!("join P05 unfinished reader: {e}"))?
}

pub(crate) async fn restore_p05_unit(date: String) -> Result<Option<Arc<P05AuctionUnit>>, String> {
    let state = ready_p05_runtime()?;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        handle
            .block_on(P05AuctionUnit::restore(
                Arc::clone(&state.coordinator),
                &date,
            ))
            .map(|unit| unit.map(Arc::new))
            .map_err(|e| format!("restore original P05 Unit date={date}: {e}"))
    })
    .await
    .map_err(|e| format!("join P05 original Unit restore: {e}"))?
}

pub(crate) async fn observe_p05_batch(
    captured_at: DateTime<chrono::FixedOffset>,
    entries: Vec<stock_analysis::opportunity::candidate_panel::CandidateEntry>,
    source: P05ObservedBatchSourceBytes,
    auction_rendered: Vec<u8>,
    board_rendered: Vec<u8>,
) -> Result<Arc<P05AuctionUnit>, String> {
    let state = ready_p05_runtime()?;
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let mut render = crate::push_templates::render_p05_original_invalidation;
        handle
            .block_on(P05AuctionUnit::observe_batch(
                Arc::clone(&state.coordinator),
                captured_at,
                &entries,
                source,
                auction_rendered,
                board_rendered,
                &mut render,
            ))
            .map(Arc::new)
            .map_err(|e| format!("persist complete original P05 Unit: {e}"))
    })
    .await
    .map_err(|e| format!("join P05 observed Unit preparation: {e}"))?
}

pub(crate) async fn inspect_p05_child(
    unit: Arc<P05AuctionUnit>,
    index: usize,
) -> Result<P05ChildDispatchView, String> {
    let state = ready_p05_runtime()?;
    tokio::task::spawn_blocking(move || {
        unit.require_counted_owner(&state.coordinator)
            .map_err(|e| format!("P05 child reader runtime owner mismatch: {e}"))?;
        unit.inspect_child(index)
            .map_err(|e| format!("inspect original P05 child ordinal={index}: {e}"))
    })
    .await
    .map_err(|e| format!("join P05 original child reader: {e}"))?
}

/// Reconcile already existing evidence before testing physical authority. This
/// cannot prepare, authorize retry or send, and does not map PushOutcome to a
/// completion. It lets Accepted+late/audit-only Units reach reclosure.
pub(crate) async fn reconcile_p05_unit_observation(
    unit: Arc<P05AuctionUnit>,
) -> Result<P05UnitObservation, String> {
    let state = ready_p05_runtime()?;
    tokio::task::spawn_blocking(move || {
        unit.require_counted_owner(&state.coordinator)
            .map_err(|e| format!("P05 receipt runtime owner mismatch: {e}"))?;
        let _critical = state
            .counted_delivery_critical_section
            .lock()
            .map_err(|_| "BR-239 counted delivery critical-section mutex poisoned".to_owned())?;
        let observation = unit
            .observe()
            .map_err(|e| format!("read P05 receipts: {e}"))?;
        let mut hydrations = Vec::new();
        for child in observation.child_observations() {
            use stock_analysis::durable_delivery::P05ChildReceiptObservation as Child;
            let decision = match child {
                Child::NotPrepared { .. } => continue,
                Child::Pending {
                    decision_identity, ..
                }
                | Child::PhysicallyAccepted {
                    decision_identity, ..
                }
                | Child::NonAccepted {
                    decision_identity, ..
                } => decision_identity,
            };
            hydrations.extend(reconcile_current_decision(state.as_ref(), decision)?);
        }
        queue_hydrations(state.as_ref(), &unique_hydrations(hydrations))?;
        unit.observe()
            .map_err(|e| format!("read current P05 receipts after local reconcile: {e}"))
    })
    .await
    .map_err(|e| format!("join P05 local receipt reconcile: {e}"))?
}

pub(crate) async fn finalize_p05_unit(
    unit: Arc<P05AuctionUnit>,
    observed: P05UnitObservation,
) -> Result<P05UnitObservation, String> {
    let state = ready_p05_runtime()?;
    tokio::task::spawn_blocking(move || {
        unit.require_counted_owner(&state.coordinator)
            .map_err(|e| format!("P05 finalizer runtime owner mismatch: {e}"))?;
        let _critical = state
            .counted_delivery_critical_section
            .lock()
            .map_err(|_| "BR-239 counted delivery critical-section mutex poisoned".to_owned())?;
        unit.finalize(&observed)
            .map_err(|e| format!("finalize actual P05 Unit revision: {e}"))
    })
    .await
    .map_err(|e| format!("join P05 finalizer: {e}"))?
}

pub(crate) async fn deliver_p05_unit_child(
    governed: crate::notify::GovernedP05UnitChild,
) -> PushOutcome {
    let state = match ready_p05_runtime() {
        Ok(state) => state,
        Err(e) => return PushOutcome::Denied(e),
    };
    match tokio::task::spawn_blocking(move || deliver_child_blocking(state.as_ref(), governed))
        .await
    {
        Ok(Ok((evidence, _))) => outcome_from_state(evidence.state),
        Ok(Err(e)) => PushOutcome::SinkError(e),
        Err(e) => PushOutcome::SinkError(format!("join P05 contextual child consumer: {e}")),
    }
}

fn deliver_child_blocking(
    state: &RuntimeState,
    governed: crate::notify::GovernedP05UnitChild,
) -> Result<(DurableDispatchEvidence, usize), String> {
    let view = governed.into_view();
    view.require_counted_owner(&state.coordinator)
        .map_err(|e| format!("P05 dispatch runtime owner mismatch: {e}"))?;
    let _critical = state
        .counted_delivery_critical_section
        .lock()
        .map_err(|_| "BR-239 counted delivery critical-section mutex poisoned".to_owned())?;
    let prepared = view
        .prepare_child(1)
        .map_err(|e| format!("prepare actual P05 child: {e}"))?;
    if prepared.envelope() != view.envelope() {
        return Err("P05 original child changed before contextual admission".into());
    }
    prepared
        .require_counted_owner(&state.coordinator)
        .map_err(|e| format!("P05 prepared runtime owner mismatch: {e}"))?;
    let envelope = prepared.envelope().clone();
    let retry =
        envelope.push_kind == DurablePushKind::CandidateInvalidated && envelope.retry_authorized;
    advance_prepared_envelope_with_resume(state, envelope, |status| {
        if status == DecisionState::Reserved || (retry && status == DecisionState::RejectedDurable)
        {
            prepared
                .resume(std::slice::from_ref(&state.sink))
                .map(Some)
                .map_err(|e| format!("resume original P05 child: {e}"))
        } else {
            Ok(None)
        }
    })
}

/// Called inside the original startup blocking worker, where ready=false is
/// expected. No recursive startup wait and no generic P05 physical opening.
pub(super) fn resume_owned_p05_at_startup(
    state: &RuntimeState,
    decision_identity: &str,
) -> Result<Option<usize>, String> {
    let Some(view) =
        P05AuctionUnit::inspect_owned_child(Arc::clone(&state.coordinator), decision_identity)
            .map_err(|e| format!("inspect startup P05 contextual owner: {e}"))?
    else {
        return Ok(None);
    };
    let token = crate::push_templates::p05_unit_presentation_token(view.envelope().push_kind)?;
    let governed = crate::notify::preflight_p05_unit_child(token, view)
        .map_err(|outcome| format!("startup P05 governance blocker: {outcome:?}"))?;
    deliver_child_blocking(state, governed).map(|(_, calls)| Some(calls))
}
