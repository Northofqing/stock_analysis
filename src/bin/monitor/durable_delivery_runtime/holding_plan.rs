//! T03 original-owner adapter. The counted runtime remains the sole consumer.
use super::*;
use crate::holding_plan::{HoldingPlanCandidate, HoldingPlanPreparation};
use stock_analysis::durable_delivery::{
    HoldingPlanOccurrenceObservation, HoldingPlanOwnedOccurrence, HoldingPlanPrepareOutcome,
    HoldingPlanReceiptKind,
};

pub(crate) enum HoldingPlanLocalProgress {
    PhysicalAccepted,
    NotSendable(String),
    Candidate(HoldingPlanCandidate),
}
pub(crate) enum HoldingPlanDispatchResult {
    PhysicalAccepted,
    NotCompleted(String),
    AlreadyOwned(HoldingPlanCandidate),
}

fn ready_holding_runtime() -> Result<Arc<RuntimeState>, String> {
    let state = runtime_state()?;
    if !state.producer_ready.load(Ordering::Acquire) {
        return Err("holding_plan_startup_barrier_required".into());
    }
    Ok(state)
}

pub(crate) async fn prepare_holding_plan_tick(
    banner: Option<crate::push_templates::BannerCtx>,
) -> Result<HoldingPlanPreparation, String> {
    let state = ready_holding_runtime()?;
    tokio::task::spawn_blocking(move || {
        crate::holding_plan::prepare_tick_with(
            banner.as_ref(),
            stock_analysis::database::user_position_snapshot::latest_user_position_snapshot,
            |date, instrument| {
                state
                    .coordinator
                    .inspect_holding_plan_occurrence(date, instrument)
                    .map_err(|_| "holding_plan_owner_read_failed".to_owned())
            },
            crate::holding_plan::read_legacy_markers,
            crate::market_data::fetch_realtime_quote_batch,
            Utc::now()
                .with_timezone(
                    &chrono::FixedOffset::east_opt(8 * 60 * 60).expect("valid Shanghai offset"),
                )
                .fixed_offset(),
        )
    })
    .await
    .map_err(|_| "holding_plan_preparation_worker_failed".to_owned())?
}

fn inspect_original(
    state: &RuntimeState,
    instrument: &InstrumentId,
    expected: &HoldingPlanOwnedOccurrence,
) -> Result<HoldingPlanOwnedOccurrence, String> {
    let date = NaiveDate::parse_from_str(&expected.envelope().business_date, "%Y-%m-%d")
        .map_err(|_| "holding_plan_original_date_invalid".to_owned())?;
    match state
        .coordinator
        .inspect_holding_plan_occurrence(date, instrument)
        .map_err(|_| "holding_plan_owner_read_failed".to_owned())?
    {
        HoldingPlanOccurrenceObservation::Owned(actual)
            if actual.envelope() == expected.envelope() =>
        {
            Ok(actual)
        }
        _ => Err("holding_plan_original_owner_changed".into()),
    }
}

pub(crate) async fn reconcile_holding_plan_candidate(
    candidate: HoldingPlanCandidate,
) -> Result<HoldingPlanLocalProgress, String> {
    let state = ready_holding_runtime()?;
    tokio::task::spawn_blocking(move || reconcile_candidate_blocking(state.as_ref(), candidate))
        .await
        .map_err(|_| "holding_plan_reconcile_worker_failed".to_owned())?
}

fn reconcile_candidate_blocking(
    state: &RuntimeState,
    candidate: HoldingPlanCandidate,
) -> Result<HoldingPlanLocalProgress, String> {
    let _critical = state
        .counted_delivery_critical_section
        .lock()
        .map_err(|_| "BR-239 counted delivery critical-section mutex poisoned".to_owned())?;
    let candidate = match candidate {
        HoldingPlanCandidate::Fresh(prepared) => {
            let instrument = match prepared.binding.scope() {
                CountedDeliveryScope::Ticket { instrument } => instrument.clone(),
                _ => return Err("holding_plan_ticket_scope_required".into()),
            };
            match state
                .coordinator
                .inspect_holding_plan_occurrence(prepared.binding.business_date(), &instrument)
                .map_err(|_| "holding_plan_owner_read_failed".to_owned())?
            {
                HoldingPlanOccurrenceObservation::Missing => {
                    return Ok(HoldingPlanLocalProgress::Candidate(
                        HoldingPlanCandidate::Fresh(prepared),
                    ))
                }
                HoldingPlanOccurrenceObservation::Owned(owned) => {
                    HoldingPlanCandidate::Original { instrument, owned }
                }
            }
        }
        original => original,
    };
    let HoldingPlanCandidate::Original { instrument, owned } = candidate else {
        unreachable!()
    };
    let original = inspect_original(state, &instrument, &owned)?;
    // Append/ack/finalizer recovery does not require a new notification gate.
    // No sink, prepare or retry authorization is performed here.
    let hydrations = reconcile_current_decision(state, &original.envelope().decision_identity)?;
    queue_hydrations(state, &unique_hydrations(hydrations))?;
    let actual = inspect_original(state, &instrument, &original)?;
    if actual.receipt_kind() == HoldingPlanReceiptKind::PhysicalAccepted && actual.local_drained() {
        return Ok(HoldingPlanLocalProgress::PhysicalAccepted);
    }
    if actual.state() == DecisionState::Reserved
        || (actual.state() == DecisionState::RejectedDurable && actual.retry_authorized())
    {
        return Ok(HoldingPlanLocalProgress::Candidate(
            HoldingPlanCandidate::Original {
                instrument,
                owned: actual,
            },
        ));
    }
    Ok(HoldingPlanLocalProgress::NotSendable(format!(
        "holding_plan_noncompletion_{:?}",
        actual.receipt_kind()
    )))
}

pub(crate) async fn deliver_holding_plan_candidate(
    governed: crate::notify::GovernedHoldingPlanCandidate,
) -> Result<HoldingPlanDispatchResult, String> {
    let state = ready_holding_runtime()?;
    tokio::task::spawn_blocking(move || {
        deliver_candidate_blocking(state.as_ref(), governed.into_candidate())
    })
    .await
    .map_err(|_| "holding_plan_delivery_worker_failed".to_owned())?
}

fn deliver_candidate_blocking(
    state: &RuntimeState,
    candidate: HoldingPlanCandidate,
) -> Result<HoldingPlanDispatchResult, String> {
    let _critical = state
        .counted_delivery_critical_section
        .lock()
        .map_err(|_| "BR-239 counted delivery critical-section mutex poisoned".to_owned())?;
    let (instrument, envelope, retry_authorized) = match candidate {
        HoldingPlanCandidate::Original { instrument, owned } => {
            let actual = inspect_original(state, &instrument, &owned)?;
            (
                instrument,
                actual.envelope().clone(),
                actual.retry_authorized(),
            )
        }
        HoldingPlanCandidate::Fresh(prepared) => {
            let instrument = match prepared.binding.scope() {
                CountedDeliveryScope::Ticket { instrument } => instrument.clone(),
                _ => return Err("holding_plan_ticket_scope_required".into()),
            };
            let envelope = envelope_from_binding(
                prepared.binding,
                PushKind::HoldingPlan,
                &prepared.text,
                None,
            )?;
            match state
                .coordinator
                .prepare_holding_plan_occurrence(&envelope, 1, Utc::now())
                .map_err(|_| "holding_plan_prepare_failed".to_owned())?
            {
                HoldingPlanPrepareOutcome::AlreadyOwned(owned) => {
                    // This return drops the counted guard. The winner must go
                    // through its own governance outside this critical section.
                    return Ok(HoldingPlanDispatchResult::AlreadyOwned(
                        HoldingPlanCandidate::Original { instrument, owned },
                    ));
                }
                HoldingPlanPrepareOutcome::Prepared(_) => {}
            }
            let actual = match state
                .coordinator
                .inspect_holding_plan_occurrence(
                    NaiveDate::parse_from_str(&envelope.business_date, "%Y-%m-%d")
                        .map_err(|_| "holding_plan_original_date_invalid".to_owned())?,
                    &instrument,
                )
                .map_err(|_| "holding_plan_owner_read_failed".to_owned())?
            {
                HoldingPlanOccurrenceObservation::Owned(actual)
                    if actual.envelope() == &envelope =>
                {
                    actual
                }
                _ => return Err("holding_plan_prepared_owner_mismatch".into()),
            };
            (instrument, envelope, actual.retry_authorized())
        }
    };
    let date = NaiveDate::parse_from_str(&envelope.business_date, "%Y-%m-%d")
        .map_err(|_| "holding_plan_original_date_invalid".to_owned())?;
    let identity = envelope.decision_identity.clone();
    advance_prepared_envelope_with_resume(state, envelope, |current| {
        if current == DecisionState::Reserved
            || (current == DecisionState::RejectedDurable && retry_authorized)
        {
            state
                .coordinator
                .resume_deliverable(&identity, std::slice::from_ref(&state.sink), Utc::now())
                .map(Some)
                .map_err(|_| "holding_plan_resume_failed".to_owned())
        } else {
            Ok(None)
        }
    })?;
    match state
        .coordinator
        .inspect_holding_plan_occurrence(date, &instrument)
        .map_err(|_| "holding_plan_final_owner_read_failed".to_owned())?
    {
        HoldingPlanOccurrenceObservation::Owned(actual)
            if actual.envelope().decision_identity == identity =>
        {
            if actual.receipt_kind() == HoldingPlanReceiptKind::PhysicalAccepted
                && actual.local_drained()
            {
                Ok(HoldingPlanDispatchResult::PhysicalAccepted)
            } else {
                Ok(HoldingPlanDispatchResult::NotCompleted(format!(
                    "holding_plan_noncompletion_{:?}",
                    actual.receipt_kind()
                )))
            }
        }
        _ => Err("holding_plan_final_owner_mismatch".into()),
    }
}

#[cfg(test)]
#[path = "holding_plan_tests.rs"]
mod tests;
