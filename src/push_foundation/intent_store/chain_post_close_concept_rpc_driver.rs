//! One six-slot transport loop, with closed dispatch to the two journals.
use super::position_concept_rpc as position;
use super::positions::PositionBatch;
use super::*;

pub(super) enum Journal<'a> {
    Initial,
    Positions(&'a PositionBatch),
}
enum Call {
    Initial(concept_rpc::ConceptRpcCall),
    Positions(position::Call),
}
impl Call {
    fn occurrence_version(&self) -> u64 {
        match self {
            Self::Initial(value) => value.occurrence_version(),
            Self::Positions(value) => value.occurrence_version(),
        }
    }
}
enum Terminal {
    Initial(concept_rpc::StoredAttemptResult),
    Positions(position::Terminal),
}
impl Terminal {
    fn retry_backoff(&self) -> Result<Option<u64>, ChainPostCloseError> {
        match self {
            Self::Initial(value) => value.retry_backoff(),
            Self::Positions(value) => value.retry_backoff(),
        }
    }
}
enum Material {
    Initial(concept_rpc::StoredErrorMaterial),
    Positions(position::Material),
}
impl Material {
    fn gateway_error(&self) -> crate::data_gateway::GatewayError {
        match self {
            Self::Initial(value) => value.gateway_error(),
            Self::Positions(value) => value.gateway_error(),
        }
    }
}
enum Capability {
    Initial(concept_rpc::LiveErrorCapability),
    Positions(position::LiveErrorCapability),
}
pub(super) enum Projection {
    Initial(StoredConceptProviderResult),
    Positions(position::Projection),
}
impl Projection {
    pub(super) fn into_initial(self) -> Result<StoredConceptProviderResult, ChainPostCloseError> {
        match self {
            Self::Initial(value) => Ok(value),
            _ => Err(ChainPostCloseError::SchemaRejected),
        }
    }
    pub(super) fn into_positions(self) -> Result<position::Projection, ChainPostCloseError> {
        match self {
            Self::Positions(value) => Ok(value),
            _ => Err(ChainPostCloseError::SchemaRejected),
        }
    }
}
enum Recovery {
    NeverStarted,
    Planned {
        request: crate::data_gateway::grpc_source::RestoredMembershipRequest,
        occurrence_version: u64,
    },
    Retry {
        request: crate::data_gateway::grpc_source::RestoredMembershipRequest,
        occurrence_version: u64,
        backoff_ms: u64,
    },
    BegunUnconfirmed,
    Response {
        terminal: Terminal,
        request: concept_rpc_codec::RestoredRequest,
        response: crate::grpc_client::pb::magic::market::v1::QueryResponse,
    },
    Error {
        terminal: Terminal,
        material: Material,
    },
    TerminalUnconfirmed,
    Complete(Projection),
}

impl From<concept_rpc::Recovery> for Recovery {
    fn from(value: concept_rpc::Recovery) -> Self {
        match value {
            concept_rpc::Recovery::NeverStarted => Self::NeverStarted,
            concept_rpc::Recovery::Planned {
                request,
                occurrence_version,
            } => Self::Planned {
                request,
                occurrence_version,
            },
            concept_rpc::Recovery::Retry {
                request,
                occurrence_version,
                backoff_ms,
            } => Self::Retry {
                request,
                occurrence_version,
                backoff_ms,
            },
            concept_rpc::Recovery::BegunUnconfirmed => Self::BegunUnconfirmed,
            concept_rpc::Recovery::Response {
                terminal,
                request,
                response,
            } => Self::Response {
                terminal: Terminal::Initial(terminal),
                request,
                response,
            },
            concept_rpc::Recovery::Error { terminal, material } => Self::Error {
                terminal: Terminal::Initial(terminal),
                material: Material::Initial(material),
            },
            concept_rpc::Recovery::TerminalUnconfirmed => Self::TerminalUnconfirmed,
            concept_rpc::Recovery::Complete(value) => Self::Complete(Projection::Initial(value)),
        }
    }
}

impl From<position::Recovery> for Recovery {
    fn from(value: position::Recovery) -> Self {
        match value {
            position::Recovery::NeverStarted => Self::NeverStarted,
            position::Recovery::Planned {
                request,
                occurrence_version,
            } => Self::Planned {
                request,
                occurrence_version,
            },
            position::Recovery::Retry {
                request,
                occurrence_version,
                backoff_ms,
            } => Self::Retry {
                request,
                occurrence_version,
                backoff_ms,
            },
            position::Recovery::BegunUnconfirmed => Self::BegunUnconfirmed,
            position::Recovery::Response {
                terminal,
                request,
                response,
            } => Self::Response {
                terminal: Terminal::Positions(terminal),
                request,
                response,
            },
            position::Recovery::Error { terminal, material } => Self::Error {
                terminal: Terminal::Positions(terminal),
                material: Material::Positions(material),
            },
            position::Recovery::TerminalUnconfirmed => Self::TerminalUnconfirmed,
            position::Recovery::Complete(value) => Self::Complete(Projection::Positions(value)),
        }
    }
}

impl Journal<'_> {
    fn load(
        &self,
        local: &mut LocalChainPostClose<'_>,
        lease: &RunLease,
        ordinal: u64,
        code: &str,
        now: UtcMicros,
    ) -> Result<Recovery, ChainPostCloseError> {
        match self {
            Self::Initial => local
                .load_concept_rpc(lease, ordinal, code, now)
                .map(Into::into),
            Self::Positions(batch) => local
                .load_position_rpc(lease, batch, ordinal, code, now)
                .map(Into::into),
        }
    }
    fn begin(
        &self,
        local: &mut LocalChainPostClose<'_>,
        lease: RunLease,
        ordinal: u64,
        code: &str,
        occurrence: Option<u64>,
        authorized: &crate::grpc_client::client::board_attempt::AuthorizedBoardAttempt,
        now: UtcMicros,
    ) -> Result<(RunLease, Call), ChainPostCloseError> {
        match self {
            Self::Initial => local
                .begin_concept_rpc_attempt(lease, ordinal, code, occurrence, authorized, now)
                .map(|(lease, call)| (lease, Call::Initial(call))),
            Self::Positions(batch) => local
                .begin_position_rpc(lease, batch, ordinal, code, occurrence, authorized, now)
                .map(|(lease, call)| (lease, Call::Positions(call))),
        }
    }
    fn record(
        &self,
        local: &mut LocalChainPostClose<'_>,
        lease: RunLease,
        call: Call,
        completion: &crate::data_gateway::grpc_source::BoardAttemptCompletion,
        now: UtcMicros,
    ) -> Result<(RunLease, Terminal, Option<Capability>), ChainPostCloseError> {
        match (self, call) {
            (Self::Initial, Call::Initial(call)) => local
                .record_concept_rpc_result(lease, call, completion, now)
                .map(|(lease, terminal, capability)| {
                    (
                        lease,
                        Terminal::Initial(terminal),
                        capability.map(Capability::Initial),
                    )
                }),
            (Self::Positions(batch), Call::Positions(call)) => local
                .record_position_rpc(lease, batch, call, completion, now)
                .map(|(lease, terminal, capability)| {
                    (
                        lease,
                        Terminal::Positions(terminal),
                        capability.map(Capability::Positions),
                    )
                }),
            _ => Err(ChainPostCloseError::SchemaRejected),
        }
    }
    fn confirm_error(
        &self,
        local: &mut LocalChainPostClose<'_>,
        lease: RunLease,
        capability: Capability,
        error: &crate::data_gateway::GatewayError,
        now: UtcMicros,
    ) -> Result<(RunLease, Material), ChainPostCloseError> {
        match (self, capability) {
            (Self::Initial, Capability::Initial(value)) => local
                .confirm_concept_rpc_error(lease, value, error, now)
                .map(|(lease, value)| (lease, Material::Initial(value))),
            (Self::Positions(batch), Capability::Positions(value)) => local
                .confirm_position_rpc_error(lease, batch, value, error, now)
                .map(|(lease, value)| (lease, Material::Positions(value))),
            _ => Err(ChainPostCloseError::SchemaRejected),
        }
    }
    fn finalize(
        &self,
        local: &mut LocalChainPostClose<'_>,
        lease: RunLease,
        terminal: &Terminal,
        projected: &Result<
            crate::data_gateway::GatewayBatch<crate::data_gateway::BoardMembershipRecord>,
            crate::data_gateway::GatewayError,
        >,
        material: Option<&Material>,
        now: UtcMicros,
    ) -> Result<(RunLease, Projection), ChainPostCloseError> {
        match (self, terminal) {
            (Self::Initial, Terminal::Initial(terminal)) => {
                let material = match material {
                    Some(Material::Initial(value)) => Some(value),
                    None => None,
                    _ => return Err(ChainPostCloseError::SchemaRejected),
                };
                local
                    .finalize_concept_rpc(lease, terminal, projected, material, now)
                    .map(|(lease, value)| (lease, Projection::Initial(value)))
            }
            (Self::Positions(batch), Terminal::Positions(terminal)) => {
                let material = match material {
                    Some(Material::Positions(value)) => Some(value),
                    None => None,
                    _ => return Err(ChainPostCloseError::SchemaRejected),
                };
                local
                    .finalize_position_rpc(lease, batch, terminal, projected, material, now)
                    .map(|(lease, value)| (lease, Projection::Positions(value)))
            }
            _ => Err(ChainPostCloseError::SchemaRejected),
        }
    }
}

enum RpcReadyWork {
    Fresh {
        ordinal: u64,
        code: String,
    },
    Planned {
        ordinal: u64,
        code: String,
        request: crate::data_gateway::grpc_source::RestoredMembershipRequest,
        occurrence_version: u64,
    },
}

enum RpcBackoffTarget {
    Live(crate::grpc_client::client::board_attempt::BoardQuerySession),
    Restored(crate::data_gateway::grpc_source::RestoredMembershipRequest),
}

enum RpcEvent {
    BackoffReady {
        ordinal: u64,
        code: String,
        occurrence_version: u64,
        target: RpcBackoffTarget,
    },
    AttemptFinished {
        ordinal: u64,
        code: String,
        occurrence_version: u64,
        session: crate::grpc_client::client::board_attempt::BoardQuerySession,
        call: Call,
        completion: crate::data_gateway::grpc_source::BoardAttemptCompletion,
        guard: EffectGuard,
    },
}

type RpcEventFuture<'provider> = Pin<Box<dyn Future<Output = RpcEvent> + 'provider>>;

pub(super) async fn drive<C>(
    local: &mut LocalChainPostClose<'_>,
    mut lease: RunLease,
    queries: &ConnectedBoardQueries,
    clock: &C,
    journal: Journal<'_>,
    work: &[(u64, String)],
    cancelled: Rc<Cell<bool>>,
) -> AnyResult<(RunLease, Vec<Projection>)>
where
    C: ConceptEffectClock,
{
    let intent_id = lease.intent_id.as_str().to_owned();
    let mut recoveries = Vec::with_capacity(work.len());
    for (ordinal, code) in work {
        let ordinal = *ordinal;
        let recovery = journal
            .load(local, &lease, ordinal, code, clock.now())
            .map_err(|error| preparation_stop(error, &intent_id))?;
        if matches!(
            recovery,
            Recovery::BegunUnconfirmed | Recovery::TerminalUnconfirmed
        ) {
            return Err(preparation_stop(
                ChainPostCloseError::IncompleteEffect {
                    intent_id: intent_id.clone(),
                },
                &intent_id,
            ));
        }
        recoveries.push((ordinal, code.clone(), recovery));
    }
    if recoveries
        .iter()
        .filter(|(_, _, recovery)| matches!(recovery, Recovery::Retry { .. }))
        .count()
        > 6
    {
        return Err(preparation_stop(
            ChainPostCloseError::SchemaRejected,
            &intent_id,
        ));
    }
    let mut ready = VecDeque::new();
    let mut recovered_retry = Vec::new();
    let mut stored = Vec::with_capacity(work.len());
    for (ordinal, code, recovery) in recoveries {
        match recovery {
            Recovery::NeverStarted => ready.push_back(RpcReadyWork::Fresh { ordinal, code }),
            Recovery::Planned {
                request,
                occurrence_version,
            } => ready.push_back(RpcReadyWork::Planned {
                ordinal,
                code,
                request,
                occurrence_version,
            }),
            Recovery::Retry {
                request,
                occurrence_version,
                backoff_ms,
            } => recovered_retry.push((ordinal, code, occurrence_version, request, backoff_ms)),
            Recovery::BegunUnconfirmed | Recovery::TerminalUnconfirmed => {
                unreachable!("preflight rejected")
            }
            Recovery::Response {
                terminal,
                request,
                response,
            } => {
                let projected = ConnectedBoardQueries::restore_memberships_response(
                    request.profile,
                    request.acquisition_authority.as_deref(),
                    &request.request_id,
                    response,
                );
                if projected.is_err() {
                    return Err(preparation_stop(
                        ChainPostCloseError::IncompleteEffect {
                            intent_id: intent_id.clone(),
                        },
                        &intent_id,
                    ));
                }
                let (next, result) = journal
                    .finalize(local, lease, &terminal, &projected, None, clock.now())
                    .map_err(|error| result_unconfirmed(error, intent_id.clone()))?;
                lease = next;
                stored.push(result);
            }
            Recovery::Error { terminal, material } => {
                let projected = Err(material.gateway_error());
                let (next, result) = journal
                    .finalize(
                        local,
                        lease,
                        &terminal,
                        &projected,
                        Some(&material),
                        clock.now(),
                    )
                    .map_err(|error| result_unconfirmed(error, intent_id.clone()))?;
                lease = next;
                stored.push(result);
            }
            Recovery::Complete(result) => stored.push(result),
        }
    }
    let mut active = HashSet::new();
    let mut events = FuturesUnordered::<RpcEventFuture<'_>>::new();
    for (ordinal, code, occurrence_version, request, backoff_ms) in recovered_retry {
        active.insert(ordinal);
        events.push(Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            RpcEvent::BackoffReady {
                ordinal,
                code,
                occurrence_version,
                target: RpcBackoffTarget::Restored(request),
            }
        }));
    }
    while !ready.is_empty() || !events.is_empty() {
        while active.len() < 6 {
            let Some(work) = ready.pop_front() else {
                break;
            };
            let (ordinal, code, occurrence_version, mut session) = match work {
                RpcReadyWork::Fresh { ordinal, code } => {
                    let session = queries.memberships_session(&code).map_err(|error| {
                        anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                            intent_id: intent_id.clone(),
                        })
                    })?;
                    (ordinal, code, None, session)
                }
                RpcReadyWork::Planned {
                    ordinal,
                    code,
                    request,
                    occurrence_version,
                } => {
                    let session = queries
                        .resume_memberships_session(request)
                        .map_err(|error| {
                            anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                                intent_id: intent_id.clone(),
                            })
                        })?;
                    (ordinal, code, Some(occurrence_version), session)
                }
            };
            let authorized = session.authorize_next().map_err(|error| {
                anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                    intent_id: intent_id.clone(),
                })
            })?;
            let (next, call) = journal
                .begin(
                    local,
                    lease,
                    ordinal,
                    &code,
                    occurrence_version,
                    &authorized,
                    clock.now(),
                )
                .map_err(|error| preparation_stop(error, &intent_id))?;
            lease = next;
            active.insert(ordinal);
            let guard = EffectGuard::new(Rc::clone(&cancelled));
            events.push(Box::pin(async move {
                let completion = authorized.execute().await;
                RpcEvent::AttemptFinished {
                    ordinal,
                    code,
                    occurrence_version: occurrence_version.unwrap_or(call.occurrence_version()),
                    session,
                    call,
                    completion,
                    guard,
                }
            }));
        }
        let Some(event) = events.next().await else {
            continue;
        };
        match event {
            RpcEvent::BackoffReady {
                ordinal,
                code,
                occurrence_version,
                target,
            } => {
                let mut session = match target {
                    RpcBackoffTarget::Live(session) => session,
                    RpcBackoffTarget::Restored(request) => queries
                        .resume_memberships_session(request)
                        .map_err(|error| {
                            cancelled.set(true);
                            anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                                intent_id: intent_id.clone(),
                            })
                        })?,
                };
                let authorized = session.authorize_next().map_err(|error| {
                    cancelled.set(true);
                    anyhow::Error::new(error).context(PreparationStop::AuthorityRejected {
                        intent_id: intent_id.clone(),
                    })
                })?;
                let (next, call) = journal
                    .begin(
                        local,
                        lease,
                        ordinal,
                        &code,
                        Some(occurrence_version),
                        &authorized,
                        clock.now(),
                    )
                    .map_err(|error| {
                        cancelled.set(true);
                        preparation_stop(error, &intent_id)
                    })?;
                lease = next;
                let guard = EffectGuard::new(Rc::clone(&cancelled));
                events.push(Box::pin(async move {
                    let completion = authorized.execute().await;
                    RpcEvent::AttemptFinished {
                        ordinal,
                        code,
                        occurrence_version,
                        session,
                        call,
                        completion,
                        guard,
                    }
                }));
            }
            RpcEvent::AttemptFinished {
                ordinal,
                code,
                occurrence_version,
                session,
                call,
                completion,
                mut guard,
            } => {
                let continuation = completion.continuation;
                let recorded = journal.record(local, lease, call, &completion, clock.now());
                let (next, terminal, capability) = match recorded {
                    Ok(value) => value,
                    Err(error) => {
                        cancelled.set(true);
                        drop(events);
                        return Err(result_unconfirmed(error, intent_id));
                    }
                };
                lease = next;
                guard.disarm();
                if matches!(
                    continuation,
                    crate::data_gateway::grpc_source::BoardContinuation::Retry { .. }
                ) {
                    let backoff_ms = terminal
                        .retry_backoff()
                        .map_err(|error| preparation_stop(error, &intent_id))?
                        .ok_or_else(|| {
                            preparation_stop(ChainPostCloseError::SchemaRejected, &intent_id)
                        })?;
                    events.push(Box::pin(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        RpcEvent::BackoffReady {
                            ordinal,
                            code,
                            occurrence_version,
                            target: RpcBackoffTarget::Live(session),
                        }
                    }));
                    continue;
                }
                let projected = ConnectedBoardQueries::memberships_completion(completion);
                let material = if let Err(error) = &projected {
                    let capability = capability.ok_or_else(|| {
                        cancelled.set(true);
                        result_unconfirmed(ChainPostCloseError::SchemaRejected, intent_id.clone())
                    })?;
                    let (next, material) = journal
                        .confirm_error(local, lease, capability, error, clock.now())
                        .map_err(|error| {
                            cancelled.set(true);
                            result_unconfirmed(error, intent_id.clone())
                        })?;
                    lease = next;
                    Some(material)
                } else {
                    None
                };
                let (next, result) = journal
                    .finalize(
                        local,
                        lease,
                        &terminal,
                        &projected,
                        material.as_ref(),
                        clock.now(),
                    )
                    .map_err(|error| {
                        cancelled.set(true);
                        result_unconfirmed(error, intent_id.clone())
                    })?;
                lease = next;
                stored.push(result);
                active.remove(&ordinal);
            }
        }
    }
    Ok((lease, stored))
}
