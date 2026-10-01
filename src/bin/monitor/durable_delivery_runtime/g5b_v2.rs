//! Short local v2 operations use the existing runtime owner and blocking pool.
use super::*;
use stock_analysis::monitor::g5b_analysis_v2::{
    self, G5bAnalysisClaimV2, G5bCompletedAnalysisV2, G5bModelDispatchViewV2,
};

pub(crate) async fn inspect_g5b_cohort_v2(date: NaiveDate) -> Result<Option<usize>, String> {
    ensure_startup_reconciled().await?;
    let state = runtime_state()?;
    tokio::task::spawn_blocking(move || {
        g5b_analysis_v2::inspect_analysis_cohort_v2(Arc::clone(&state.coordinator), date)
            .map_err(|e| format!("inspect G5b v2 cohort: {e}"))
    })
    .await
    .map_err(|e| format!("join G5b v2 cohort inspection: {e}"))?
}

pub(crate) async fn claim_g5b_analysis_v2(
    date: NaiveDate,
    index: usize,
) -> Result<G5bAnalysisClaimV2, String> {
    ensure_startup_reconciled().await?;
    let state = runtime_state()?;
    tokio::task::spawn_blocking(move || {
        g5b_analysis_v2::claim_analysis_v2(Arc::clone(&state.coordinator), date, index)
            .map_err(|e| format!("claim G5b v2 member {index}: {e}"))
    })
    .await
    .map_err(|e| format!("join G5b v2 claim: {e}"))?
}

pub(crate) async fn freeze_g5b_analysis_v2(
    date: NaiveDate,
    completed: G5bCompletedAnalysisV2,
) -> Result<(), String> {
    let state = runtime_state()?;
    tokio::task::spawn_blocking(move || {
        completed
            .freeze()
            .map_err(|e| format!("freeze G5b v2 result: {e}"))?;
        g5b_analysis_v2::archive_model_observations_v2(Arc::clone(&state.coordinator), date)
            .map_err(|e| format!("archive G5b v2 result: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("join G5b v2 freeze/archive: {e}"))?
}

pub(crate) async fn archive_g5b_v2(date: NaiveDate) -> Result<(), String> {
    let state = runtime_state()?;
    tokio::task::spawn_blocking(move || {
        g5b_analysis_v2::archive_model_observations_v2(Arc::clone(&state.coordinator), date)
            .map(|_| ())
            .map_err(|e| format!("recover G5b v2 archive: {e}"))
    })
    .await
    .map_err(|e| format!("join G5b v2 archive recovery: {e}"))?
}

pub(crate) async fn inspect_g5b_dispatch_v2(
    date: NaiveDate,
    index: usize,
) -> Result<Option<G5bModelDispatchViewV2>, String> {
    let state = runtime_state()?;
    tokio::task::spawn_blocking(move || {
        g5b_analysis_v2::inspect_model_dispatch_v2(Arc::clone(&state.coordinator), date, index)
            .map_err(|e| format!("inspect G5b v2 dispatch: {e}"))
    })
    .await
    .map_err(|e| format!("join G5b v2 dispatch inspection: {e}"))?
}

pub(crate) async fn deliver_g5b_model_dispatch_v2(view: G5bModelDispatchViewV2) -> PushOutcome {
    if let Err(e) = ensure_startup_reconciled().await {
        return PushOutcome::Denied(format!("durable delivery admission frozen: {e}"));
    }
    let state = match runtime_state() {
        Ok(state) => state,
        Err(e) => return PushOutcome::SinkError(e),
    };
    match tokio::task::spawn_blocking(move || deliver_model_blocking(state.as_ref(), view)).await {
        Ok(Ok(evidence)) => outcome_from_state(evidence.state),
        Ok(Err(e)) => PushOutcome::SinkError(e),
        Err(e) => PushOutcome::SinkError(format!("join G5b v2 delivery: {e}")),
    }
}

fn deliver_model_blocking(
    state: &RuntimeState,
    view: G5bModelDispatchViewV2,
) -> Result<DurableDispatchEvidence, String> {
    let _critical_section = state
        .counted_delivery_critical_section
        .lock()
        .map_err(|_| "BR-239 counted delivery critical-section mutex poisoned".to_owned())?;
    let prepared = g5b_analysis_v2::prepare_model_owner_v2(
        Arc::clone(&state.coordinator),
        view.business_date(),
        view.selection_index(),
        1,
    )
    .map_err(|e| format!("prepare actual G5b v2 owner: {e}"))?;
    if prepared.envelope() != view.envelope() {
        return Err("G5b v2 original dispatch changed before admission".to_owned());
    }
    // The local owner has released its date fence. Reuse the sole physical
    // consumer without a generic second prepare or a second delivery ledger.
    advance_prepared_envelope(state, prepared.envelope().clone())
}
