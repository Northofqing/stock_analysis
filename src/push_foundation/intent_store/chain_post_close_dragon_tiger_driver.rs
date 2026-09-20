use std::cell::Cell;
use std::rc::Rc;

use anyhow::Result as AnyResult;

use crate::data_gateway::grpc_source::{BoardContinuation, GrpcSource};
use crate::pipeline::chain_analysis::preparation::DragonTigerObservationClock;

use super::dragon_tiger::{Recovery, Terminal};
use super::*;

fn stopped(error: ChainPostCloseError, intent_id: &str) -> anyhow::Error {
    preparation_stop(error, intent_id)
}

async fn execute(
    local: &mut LocalChainPostClose<'_>,
    mut lease: RunLease,
    clock: &dyn DragonTigerObservationClock,
    cancelled: Rc<Cell<bool>>,
    mut occurrence_version: Option<u64>,
    mut observation: Option<chrono::DateTime<chrono::FixedOffset>>,
    mut session: crate::grpc_client::client::board_attempt::BoardQuerySession,
) -> AnyResult<(
    RunLease,
    u64,
    Terminal,
    crate::data_gateway::grpc_source::BoardAttemptCompletion,
)> {
    let intent_id = lease.intent_id.as_str().to_owned();
    loop {
        let authorized = session.authorize_next().map_err(|error| {
            anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                intent_id: intent_id.clone(),
            })
        })?;
        let (next, call) = local
            .begin_dragon_tiger_attempt(
                lease,
                observation.take(),
                occurrence_version,
                &authorized,
                clock.now(),
            )
            .map_err(|error| stopped(error, &intent_id))?;
        lease = next;
        occurrence_version = Some(call.occurrence_version);
        let mut guard = EffectGuard::new(Rc::clone(&cancelled));
        let completion = authorized.execute().await;
        let continuation = completion.continuation;
        let (next, terminal) = local
            .record_dragon_tiger_result(lease, call, &completion, clock.now())
            .map_err(|error| result_unconfirmed(error, intent_id.clone()))?;
        lease = next;
        guard.disarm();
        if let BoardContinuation::Retry { .. } = continuation {
            let backoff = terminal
                .retry_backoff()
                .map_err(|error| stopped(error, &intent_id))?
                .ok_or_else(|| stopped(ChainPostCloseError::SchemaRejected, &intent_id))?;
            tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
            continue;
        }
        return Ok((
            lease,
            occurrence_version.ok_or(ChainPostCloseError::SchemaRejected)?,
            terminal,
            completion,
        ));
    }
}

pub(super) async fn drive(
    local: &mut LocalChainPostClose<'_>,
    mut lease: RunLease,
    source: &GrpcSource,
    clock: &dyn DragonTigerObservationClock,
    cancelled: Rc<Cell<bool>>,
) -> AnyResult<(RunLease, dragon_tiger_codec::Projection)> {
    let intent_id = lease.intent_id.as_str().to_owned();
    let recovery = local
        .load_dragon_tiger(&lease, clock.now())
        .map_err(|error| stopped(error, &intent_id))?;
    let (occurrence_version, terminal, gateway, material) = match recovery {
        Recovery::Complete(projection) => return Ok((lease, projection)),
        Recovery::BegunUnconfirmed => {
            return Err(stopped(
                ChainPostCloseError::IncompleteEffect {
                    intent_id: intent_id.clone(),
                },
                &intent_id,
            ));
        }
        Recovery::Terminal {
            occurrence_version,
            terminal,
            projected,
            material,
        } => (occurrence_version, terminal, projected, material),
        Recovery::NeverStarted => {
            let observation = clock.dragon_tiger_request_observation();
            let session = source
                .dragon_tiger_query_session(observation.date_naive(), 100, 5_000)
                .await
                .map_err(|error| {
                    anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                        intent_id: intent_id.clone(),
                    })
                })?;
            let (next, occurrence, terminal, completion) = execute(
                local,
                lease,
                clock,
                Rc::clone(&cancelled),
                None,
                Some(observation),
                session,
            )
            .await?;
            lease = next;
            (
                occurrence,
                terminal,
                GrpcSource::dragon_tiger_completion(completion),
                None,
            )
        }
        Recovery::Planned {
            request,
            occurrence_version,
        } => {
            let session = source
                .resume_dragon_tiger_query_session(request)
                .await
                .map_err(|error| {
                    anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                        intent_id: intent_id.clone(),
                    })
                })?;
            let (next, occurrence, terminal, completion) = execute(
                local,
                lease,
                clock,
                Rc::clone(&cancelled),
                Some(occurrence_version),
                None,
                session,
            )
            .await?;
            lease = next;
            (
                occurrence,
                terminal,
                GrpcSource::dragon_tiger_completion(completion),
                None,
            )
        }
        Recovery::Retry {
            request,
            occurrence_version,
            backoff_ms,
        } => {
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            let session = source
                .resume_dragon_tiger_query_session(request)
                .await
                .map_err(|error| {
                    anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                        intent_id: intent_id.clone(),
                    })
                })?;
            let (next, occurrence, terminal, completion) = execute(
                local,
                lease,
                clock,
                Rc::clone(&cancelled),
                Some(occurrence_version),
                None,
                session,
            )
            .await?;
            lease = next;
            (
                occurrence,
                terminal,
                GrpcSource::dragon_tiger_completion(completion),
                None,
            )
        }
    };
    let material = match (&gateway, material) {
        (Err(_), Some(material)) => Some(material),
        (Err(error), None) => {
            let (next, material) = local
                .confirm_dragon_tiger_error(lease, &terminal, error, clock.now())
                .map_err(|error| result_unconfirmed(error, intent_id.clone()))?;
            lease = next;
            Some(material)
        }
        (Ok(_), None) => None,
        _ => return Err(stopped(ChainPostCloseError::SchemaRejected, &intent_id)),
    };
    let (lease, projection) = local
        .finalize_dragon_tiger(
            lease,
            occurrence_version,
            &terminal,
            &gateway,
            material.as_ref(),
            clock.now(),
        )
        .map_err(|error| result_unconfirmed(error, intent_id))?;
    Ok((lease, projection))
}
