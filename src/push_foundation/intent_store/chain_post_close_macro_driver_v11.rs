use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use super::{
    macro_codec, macro_stage::MacroControlOutcome, preparation_stop, result_unconfirmed,
    ChainPostCloseError, EffectGuard, LocalChainPostClose, PreparationStop, RunLease,
    UnmigratedStage,
};
use crate::data_gateway::grpc_source::macro_queries::PreparedMacroQueries;
use crate::data_gateway::grpc_source::GrpcSource;
use crate::data_gateway::GlobalNewsProvider;
use crate::grpc_client::client::macro_attempt::{
    ExternalMacroAttemptCompletion, MacroContinuation,
};
use crate::pipeline::chain_analysis::preparation::MacroObservationClock;
use crate::search_service::SearchService;

fn pending() -> anyhow::Error {
    PreparationStop::StageNotMigrated {
        next: UnmigratedStage::Macro,
    }
    .into()
}

fn unconfirmed(intent: &str) -> anyhow::Error {
    PreparationStop::ResultUnconfirmed {
        intent_id: intent.to_owned(),
    }
    .into()
}

fn authority(error: crate::grpc_client::errors::GrpcError, intent: &str) -> anyhow::Error {
    anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
        intent_id: intent.to_owned(),
    })
}

fn before_deadline(
    clock: &dyn MacroObservationClock,
    deadline: i64,
    limit: tokio::time::Instant,
) -> bool {
    clock.now().get() < deadline && tokio::time::Instant::now() < limit
}

pub(super) async fn drive(
    local: &mut LocalChainPostClose<'_>,
    mut lease: RunLease,
    source: &GrpcSource,
    clock: &dyn MacroObservationClock,
    cancelled: Rc<Cell<bool>>,
    search_service: &SearchService,
) -> anyhow::Result<RunLease> {
    let intent = lease.intent_id.as_str().to_owned();
    let entered = clock.now();
    let entered_monotonic = tokio::time::Instant::now();
    let recovery = local
        .load_macro(&lease, entered)
        .map_err(|error| preparation_stop(error, &intent))?;
    if recovery
        .as_ref()
        .is_some_and(|recovery| recovery.has_unconfirmed_effect())
    {
        return Err(preparation_stop(
            ChainPostCloseError::IncompleteEffect {
                intent_id: intent.clone(),
            },
            &intent,
        ));
    }
    if recovery.as_ref().is_some_and(|recovery| {
        recovery
            .global_news(GlobalNewsProvider::Eastmoney)
            .is_some()
    }) {
        return Ok(lease);
    }
    let deadline = match &recovery {
        Some(recovery) => recovery.plan().deadline_at().get(),
        None => entered
            .get()
            .checked_add(15_000_000)
            .ok_or_else(|| preparation_stop(ChainPostCloseError::SchemaRejected, &intent))?,
    };
    let remaining = deadline
        .checked_sub(entered.get())
        .filter(|remaining| *remaining > 0)
        .ok_or_else(pending)?;
    let limit =
        entered_monotonic + Duration::from_micros(u64::try_from(remaining).map_err(|_| pending())?);

    // The durable state and any unknown effect are checked before reading a
    // bundle, authorizing a request, observing the clock, or connecting.
    let route = source.prepare_macro_queries().map_err(|error| {
        anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
            intent_id: intent.clone(),
        })
    })?;
    if let Some(recovery) = &recovery {
        if recovery.plan().endpoint() != route.endpoint()
            || recovery.plan().profile() != route.profile()
        {
            return Err(preparation_stop(
                ChainPostCloseError::SchemaRejected,
                &intent,
            ));
        }
    } else {
        let web = search_service.macro_web_snapshot(source).map_err(|error| {
            anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                intent_id: intent.clone(),
            })
        })?;
        let observation = clock.macro_request_observation();
        let (request, episode) = match &route {
            PreparedMacroQueries::Local(connected) => {
                let authorized = connected
                    .session(macro_codec::first_identity())
                    .and_then(|session| session.authorize_next())
                    .map_err(|error| authority(error, &intent))?;
                (macro_codec::Request::capture(&authorized)?, None)
            }
            PreparedMacroQueries::External(prepared) => {
                let data = prepared
                    .prepare_macro_query(macro_codec::first_identity())
                    .map_err(|error| authority(error, &intent))?;
                let health = prepared
                    .prepare_health_attempt()
                    .map_err(|error| authority(error, &intent))?;
                let capabilities = prepared
                    .prepare_capabilities_attempt()
                    .map_err(|error| authority(error, &intent))?;
                (
                    macro_codec::Request::capture_prepared(&data)?,
                    Some(macro_codec::ReadinessEpisodePlan::new(
                        health.request_material(),
                        capabilities.request_material(),
                    )?),
                )
            }
        };
        lease = local
            .plan_macro_request(
                lease,
                entered,
                observation,
                route.endpoint(),
                request,
                episode,
                &web,
                clock.now(),
            )
            .map_err(|error| preparation_stop(error, &intent))?;
    }

    match route {
        PreparedMacroQueries::Local(connected) => {
            drive_local(
                local, lease, connected, clock, cancelled, deadline, limit, &intent,
            )
            .await
        }
        PreparedMacroQueries::External(prepared) => {
            drive_external(
                local, lease, prepared, clock, cancelled, deadline, limit, &intent,
            )
            .await
        }
    }
}

async fn drive_local(
    local: &mut LocalChainPostClose<'_>,
    mut lease: RunLease,
    connected: crate::data_gateway::grpc_source::macro_queries::ConnectedMacroQueries,
    clock: &dyn MacroObservationClock,
    cancelled: Rc<Cell<bool>>,
    deadline: i64,
    limit: tokio::time::Instant,
    intent: &str,
) -> anyhow::Result<RunLease> {
    loop {
        let recovery = local
            .load_macro(&lease, clock.now())
            .map_err(|error| preparation_stop(error, intent))?
            .ok_or_else(|| preparation_stop(ChainPostCloseError::MacroNotStarted, intent))?;
        if recovery.has_unconfirmed_effect() {
            return Err(preparation_stop(
                ChainPostCloseError::IncompleteEffect {
                    intent_id: intent.to_owned(),
                },
                intent,
            ));
        }
        if recovery
            .global_news(GlobalNewsProvider::Eastmoney)
            .is_some()
        {
            return Ok(lease);
        }
        if !before_deadline(clock, deadline, limit) {
            return Err(pending());
        }
        wait_retry(&recovery, clock, limit).await?;
        let next = u32::try_from(recovery.attempts().len() + 1)
            .map_err(|_| preparation_stop(ChainPostCloseError::SchemaRejected, intent))?;
        let session = connected
            .resume(
                macro_codec::first_identity(),
                recovery.plan().first_source_request().restored(next),
            )
            .map_err(|error| authority(error, intent))?;
        let authorized = session
            .authorize_next()
            .map_err(|error| authority(error, intent))?;
        if !before_deadline(clock, deadline, limit) {
            return Err(pending());
        }
        let (next_lease, call) = local
            .begin_macro_attempt(lease, &authorized, clock.now())
            .map_err(|error| preparation_stop(error, intent))?;
        lease = next_lease;
        let mut guard = EffectGuard::new(Rc::clone(&cancelled));
        recheck_begun(local, &lease, clock, intent)?;
        if !before_deadline(clock, deadline, limit) {
            return Err(unconfirmed(intent));
        }
        let completion = tokio::time::timeout_at(limit, authorized.execute())
            .await
            .map_err(|_| unconfirmed(intent))?;
        let returned_at = clock.now();
        if returned_at.get() >= deadline || tokio::time::Instant::now() >= limit {
            return Err(unconfirmed(intent));
        }
        lease = local
            .record_macro_result(lease, call, &completion, returned_at)
            .map_err(|error| result_unconfirmed(error, intent.to_owned()))?;
        guard.disarm();
    }
}

async fn drive_external(
    local: &mut LocalChainPostClose<'_>,
    mut lease: RunLease,
    prepared: crate::grpc_client::client::PreparedExternalEndpoint,
    clock: &dyn MacroObservationClock,
    cancelled: Rc<Cell<bool>>,
    deadline: i64,
    limit: tokio::time::Instant,
    intent: &str,
) -> anyhow::Result<RunLease> {
    let mut connected = None;
    loop {
        let recovery = local
            .load_macro(&lease, clock.now())
            .map_err(|error| preparation_stop(error, intent))?
            .ok_or_else(|| preparation_stop(ChainPostCloseError::MacroNotStarted, intent))?;
        if recovery.has_unconfirmed_effect() {
            return Err(preparation_stop(
                ChainPostCloseError::IncompleteEffect {
                    intent_id: intent.to_owned(),
                },
                intent,
            ));
        }
        if recovery
            .global_news(GlobalNewsProvider::Eastmoney)
            .is_some()
        {
            return Ok(lease);
        }
        if !before_deadline(clock, deadline, limit) {
            return Err(pending());
        }
        let episode = recovery
            .readiness_episodes()
            .first()
            .ok_or_else(|| preparation_stop(ChainPostCloseError::SchemaRejected, intent))?;
        let controls = episode.controls();
        if controls[0].begin_version().is_none() {
            let attempt = prepared
                .resume_health_attempt(controls[0].request_material())
                .map_err(|error| authority(error, intent))?;
            let (next_lease, call) = local
                .begin_health_control(lease, &attempt, clock.now())
                .map_err(|error| preparation_stop(error, intent))?;
            lease = next_lease;
            let mut guard = EffectGuard::new(Rc::clone(&cancelled));
            recheck_begun(local, &lease, clock, intent)?;
            if !before_deadline(clock, deadline, limit) {
                return Err(unconfirmed(intent));
            }
            let completion = tokio::time::timeout_at(limit, attempt.execute())
                .await
                .map_err(|_| unconfirmed(intent))?;
            let returned_at = clock.now();
            if returned_at.get() >= deadline || tokio::time::Instant::now() >= limit {
                return Err(unconfirmed(intent));
            }
            let (next_lease, outcome) = local
                .record_health_control_result(lease, call, &completion, returned_at)
                .map_err(|error| result_unconfirmed(error, intent.to_owned()))?;
            lease = next_lease;
            guard.disarm();
            if outcome == MacroControlOutcome::Ready {
                // Deterministic cooperative cancellation point after the Ready
                // result commit and before any Capabilities restore/begin.
                tokio::task::yield_now().await;
                connected = completion.into_connected_client();
            }
            continue;
        }
        if controls[0].outcome() != Some(MacroControlOutcome::Ready) {
            return Err(preparation_stop(
                ChainPostCloseError::SchemaRejected,
                intent,
            ));
        }
        if controls[1].begin_version().is_none() {
            let mut attempt = prepared
                .resume_capabilities_attempt(controls[1].request_material())
                .map_err(|error| authority(error, intent))?;
            if let Some(client) = connected.take() {
                attempt = attempt
                    .bind_connected(client)
                    .map_err(|error| authority(error, intent))?;
            }
            let (next_lease, call) = local
                .begin_capabilities_control(lease, &attempt, clock.now())
                .map_err(|error| preparation_stop(error, intent))?;
            lease = next_lease;
            let mut guard = EffectGuard::new(Rc::clone(&cancelled));
            recheck_begun(local, &lease, clock, intent)?;
            if !before_deadline(clock, deadline, limit) {
                return Err(unconfirmed(intent));
            }
            let completion = tokio::time::timeout_at(limit, attempt.execute())
                .await
                .map_err(|_| unconfirmed(intent))?;
            let returned_at = clock.now();
            if returned_at.get() >= deadline || tokio::time::Instant::now() >= limit {
                return Err(unconfirmed(intent));
            }
            let (next_lease, outcome) = local
                .record_capabilities_control_result(lease, call, &completion, returned_at)
                .map_err(|error| result_unconfirmed(error, intent.to_owned()))?;
            lease = next_lease;
            guard.disarm();
            if outcome == MacroControlOutcome::Ready {
                // Deterministic cooperative cancellation point after the Ready
                // result commit and before any data restore/begin.
                tokio::task::yield_now().await;
                connected = completion.into_connected_client();
            }
            continue;
        }
        if controls[1].outcome() != Some(MacroControlOutcome::Ready)
            || episode.ready_result_version().is_none()
        {
            return Err(preparation_stop(
                ChainPostCloseError::SchemaRejected,
                intent,
            ));
        }

        wait_retry(&recovery, clock, limit).await?;
        let next = u32::try_from(recovery.attempts().len() + 1)
            .map_err(|_| preparation_stop(ChainPostCloseError::SchemaRejected, intent))?;
        let authorized = prepared
            .resume_macro_query(
                macro_codec::first_identity(),
                recovery
                    .plan()
                    .first_source_request()
                    .restored_external(recovery.plan().endpoint(), next),
            )
            .map_err(|error| authority(error, intent))?;
        if !before_deadline(clock, deadline, limit) {
            return Err(pending());
        }
        let (next_lease, call) = local
            .begin_prepared_macro_attempt(lease, &authorized, clock.now())
            .map_err(|error| preparation_stop(error, intent))?;
        lease = next_lease;
        let mut guard = EffectGuard::new(Rc::clone(&cancelled));
        recheck_begun(local, &lease, clock, intent)?;
        if !before_deadline(clock, deadline, limit) {
            return Err(unconfirmed(intent));
        }
        let completion = if let Some(client) = connected.take() {
            let attempt = authorized
                .bind_connected(client)
                .map_err(|_| unconfirmed(intent))?;
            ExternalMacroAttemptCompletion::Unary(
                tokio::time::timeout_at(limit, attempt.execute())
                    .await
                    .map_err(|_| unconfirmed(intent))?,
            )
        } else {
            tokio::time::timeout_at(limit, authorized.execute())
                .await
                .map_err(|_| unconfirmed(intent))?
                .map_err(|_| unconfirmed(intent))?
        };
        let returned_at = clock.now();
        if returned_at.get() >= deadline || tokio::time::Instant::now() >= limit {
            return Err(unconfirmed(intent));
        }
        lease = local
            .record_external_macro_result(lease, call, &completion, returned_at)
            .map_err(|error| result_unconfirmed(error, intent.to_owned()))?;
        guard.disarm();
    }
}

fn recheck_begun(
    local: &mut LocalChainPostClose<'_>,
    lease: &RunLease,
    clock: &dyn MacroObservationClock,
    intent: &str,
) -> anyhow::Result<()> {
    let recovery = local
        .load_macro(lease, clock.now())
        .map_err(|error| result_unconfirmed(error, intent.to_owned()))?
        .ok_or_else(|| unconfirmed(intent))?;
    if !recovery.has_unconfirmed_effect() {
        return Err(unconfirmed(intent));
    }
    Ok(())
}

async fn wait_retry(
    recovery: &super::macro_stage::MacroRecovery,
    clock: &dyn MacroObservationClock,
    limit: tokio::time::Instant,
) -> anyhow::Result<()> {
    if let Some(last) = recovery.attempts().last() {
        if !matches!(last.continuation(), Some(MacroContinuation::Retry { .. })) {
            return Err(ChainPostCloseError::SchemaRejected.into());
        }
        let due = last
            .retry_not_before
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        if due > clock.now().get() {
            let micros = u64::try_from(due - clock.now().get()).map_err(|_| pending())?;
            tokio::time::timeout_at(limit, tokio::time::sleep(Duration::from_micros(micros)))
                .await
                .map_err(|_| pending())?;
        }
    }
    Ok(())
}
