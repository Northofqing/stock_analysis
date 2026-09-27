//! V12 transport Adapter. Scheduling, fallback and rendering live in one runner.
use super::schema;
use super::{
    macro_codec as codec,
    macro_live::{Live, Ticket},
    macro_plan_v3, ChainPostCloseError, LocalChainPostClose, RunLease,
};
use crate::data_gateway::grpc_source::{
    macro_queries::{ConnectedMacroQueries, PreparedMacroQueries},
    GrpcSource,
};
use crate::grpc_client::client::{
    external_control_attempt::{
        AuthorizedCapabilitiesAttempt, AuthorizedHealthAttempt, ExternalControlCompletion,
    },
    macro_attempt::{AuthorizedMacroAttempt, ExternalMacroAttemptCompletion},
    GrpcMarketClient, PreparedExternalEndpoint,
};
use crate::grpc_client::external_pb::magic::market::v1::{CapabilitiesResponse, HealthResponse};
use crate::pipeline::chain_analysis::preparation::{MacroObservationClock, PreparationStop};
use crate::search_service::{
    macro_news::runner::{self, MacroStepIo, QueryKey, Returned, RunEnd, Snapshot, Step},
    SearchService,
};
use futures::{future::LocalBoxFuture, FutureExt};
use std::{cell::Cell, rc::Rc, time::Duration};

pub(super) enum Outcome {
    Full(String),
    LegacySourceConfirmed,
}

pub(super) async fn drive(
    local: &mut LocalChainPostClose<'_>,
    lease: RunLease,
    source: &GrpcSource,
    clock: &dyn MacroObservationClock,
    cancelled: Rc<Cell<bool>>,
    search: &SearchService,
) -> anyhow::Result<(RunLease, Outcome)> {
    drive_prepared(local, lease, source, clock, cancelled, search, None).await
}

#[cfg(test)]
pub(super) async fn drive_test_prepared(
    local: &mut LocalChainPostClose<'_>,
    lease: RunLease,
    source: &GrpcSource,
    clock: &dyn MacroObservationClock,
    cancelled: Rc<Cell<bool>>,
    search: &SearchService,
    prepared: PreparedExternalEndpoint,
) -> anyhow::Result<(RunLease, Outcome)> {
    drive_prepared(
        local,
        lease,
        source,
        clock,
        cancelled,
        search,
        Some(prepared),
    )
    .await
}

async fn drive_prepared(
    local: &mut LocalChainPostClose<'_>,
    lease: RunLease,
    source: &GrpcSource,
    clock: &dyn MacroObservationClock,
    cancelled: Rc<Cell<bool>>,
    search: &SearchService,
    prepared_override: Option<PreparedExternalEndpoint>,
) -> anyhow::Result<(RunLease, Outcome)> {
    let intent = lease.intent_id.as_str().to_owned();
    let result = async {
        let retired_layout = schema::runtime_layout_version(&local.store.connection)? >= 14;
        let mut live = Live::open(local, lease, clock, cancelled)?;
        let legacy = live.current().is_some_and(|current| current.full.is_none());
        if legacy && live.snapshot()?.terminal(QueryKey::Gateway(1)).is_some() {
            return Ok((live.into_lease(), Outcome::LegacySourceConfirmed));
        }
        if let Some(output) = live
            .current()
            .and_then(|recovery| recovery.stage_final())
            .map(|final_| final_.output_bytes().to_vec())
        {
            return Ok((
                live.into_lease(),
                Outcome::Full(
                    String::from_utf8(output).map_err(|_| ChainPostCloseError::SchemaRejected)?,
                ),
            ));
        }
        // This is deliberately after the durable Unknown/final checks.
        let route = match prepared_override {
            Some(prepared) => PreparedMacroQueries::External(prepared),
            None => source.prepare_macro_queries()?,
        };
        let local_queries = match &route {
            PreparedMacroQueries::Local(local) => Some(local.clone()),
            PreparedMacroQueries::External(_) if legacy => None,
            PreparedMacroQueries::External(_) => {
                let local_route = match live.current().and_then(|recovery| recovery.full.as_ref()) {
                    Some(full) => full.local.clone(),
                    None => macro_plan_v3::LocalRoute::observe(source)?,
                };
                if local_route.state == macro_plan_v3::LocalRouteState::ObservedConnected {
                    let connected = source.connected_local_macro_queries()?;
                    codec::require(local_route.endpoint.as_deref() == Some(connected.endpoint()))?;
                    Some(connected)
                } else {
                    None
                }
            }
        };
        if let Some(current) = live.current() {
            codec::require(
                current.plan.endpoint() == route.endpoint()
                    && current.plan.profile() == route.profile(),
            )?;
            if let Some(local) = local_queries.as_ref().filter(|_| !retired_layout) {
                codec::require(
                    current
                        .full
                        .as_ref()
                        .and_then(|full| full.local.endpoint.as_deref())
                        == Some(local.endpoint()),
                )?;
            }
        } else {
            let web = search.macro_web_snapshot(source)?;
            let mut requests = Vec::new();
            for ordinal in 1..=4 {
                let identity =
                    crate::grpc_client::client::macro_attempt::MacroQueryIdentity::GlobalNews {
                        provider: [
                            crate::data_gateway::GlobalNewsProvider::Eastmoney,
                            crate::data_gateway::GlobalNewsProvider::Cailianpress,
                            crate::data_gateway::GlobalNewsProvider::Jin10,
                            crate::data_gateway::GlobalNewsProvider::ThePaper,
                        ][usize::from(ordinal - 1)],
                        limit: 20,
                    };
                let request = match &route {
                    PreparedMacroQueries::Local(local) => codec::Request::capture_for(
                        &identity,
                        &local.session(identity.clone())?.authorize_next()?,
                    )?,
                    PreparedMacroQueries::External(prepared) => {
                        codec::Request::capture_prepared_for(
                            &identity,
                            &prepared.prepare_macro_query(identity.clone())?,
                        )?
                    }
                };
                requests.push((
                    QueryKey::Gateway(ordinal),
                    request,
                    route.endpoint().to_owned(),
                ));
            }
            if let Some(local) = local_queries.as_ref().filter(|_| !retired_layout) {
                let identity =
                    crate::grpc_client::client::macro_attempt::MacroQueryIdentity::EconomicCalendar;
                requests.push((
                    QueryKey::Gateway(5),
                    codec::Request::capture_for(
                        &identity,
                        &local.session(identity.clone())?.authorize_next()?,
                    )?,
                    local.endpoint().to_owned(),
                ));
            }
            let episode = match &route {
                PreparedMacroQueries::External(prepared) => Some(codec::ReadinessEpisodePlan::new(
                    prepared.prepare_health_attempt()?.request_material(),
                    prepared.prepare_capabilities_attempt()?.request_material(),
                )?),
                PreparedMacroQueries::Local(_) => None,
            };
            live.initialize(
                clock.macro_request_observation(),
                route.endpoint(),
                requests,
                episode,
                &web,
            )?;
        }
        let external = match route {
            PreparedMacroQueries::External(prepared) => Some(prepared),
            PreparedMacroQueries::Local(_) => None,
        };
        let mut adapter = Durable {
            live,
            clock,
            intent: intent.clone(),
            local: local_queries,
            external,
            connected_external: None,
            current_capabilities_confirmed: false,
            retired_layout,
        };
        let output = if legacy {
            runner::continue_legacy_source(&mut adapter).await?;
            Outcome::LegacySourceConfirmed
        } else {
            Outcome::Full(runner::run(&mut adapter).await?)
        };
        Ok((adapter.live.into_lease(), output))
    }
    .await;
    result.map_err(|error: anyhow::Error| {
        if error.downcast_ref::<PreparationStop>().is_some() {
            error
        } else {
            error.context(PreparationStop::AuthorityRejected { intent_id: intent })
        }
    })
}

struct Durable<'local, 'store, 'clock> {
    live: Live<'local, 'store, 'clock>,
    clock: &'clock dyn MacroObservationClock,
    intent: String,
    local: Option<ConnectedMacroQueries>,
    external: Option<PreparedExternalEndpoint>,
    connected_external: Option<GrpcMarketClient>,
    current_capabilities_confirmed: bool,
    retired_layout: bool,
}

enum AdapterTicket {
    Effect(Ticket),
    Qualification(Ticket),
    LocalDecision(QueryKey),
}
enum Material {
    Data(ExternalMacroAttemptCompletion),
    Health(ExternalControlCompletion<HealthResponse>),
    Capabilities(ExternalControlCompletion<CapabilitiesResponse>),
    LocalDecision,
}
enum OwnedAttempt {
    Connected(AuthorizedMacroAttempt),
    Health(AuthorizedHealthAttempt),
    Capabilities(AuthorizedCapabilitiesAttempt),
}

impl Durable<'_, '_, '_> {
    fn snapshot(&self) -> anyhow::Result<Snapshot> {
        let mut snapshot = self.live.snapshot()?;
        if snapshot.definition.external_news
            && matches!(
                snapshot.external,
                runner::RouteState::NeedsCapabilities | runner::RouteState::Ready
            )
            && (1..=4).any(|ordinal| snapshot.terminal(QueryKey::Gateway(ordinal)).is_none())
        {
            if self.connected_external.is_none() {
                snapshot.external = runner::RouteState::NeedsHealth;
            } else if snapshot.external == runner::RouteState::Ready
                && !self.current_capabilities_confirmed
            {
                snapshot.external = runner::RouteState::NeedsCapabilities;
            }
        }
        Ok(snapshot)
    }
}

impl<'local, 'store, 'clock> MacroStepIo for Durable<'local, 'store, 'clock> {
    type Ticket = AdapterTicket;
    type Material = Material;
    type Call = LocalBoxFuture<'clock, anyhow::Result<Returned<AdapterTicket, Material>>>;
    type Wait = LocalBoxFuture<'clock, anyhow::Result<()>>;
    fn open(&mut self) -> anyhow::Result<Snapshot> {
        self.live.settle_legacy_local_unavailable()?;
        self.live.settle_operation_retired()?;
        if self.live.needs_historical_rejection() {
            self.live.checkpoint()?;
            let definition = self.live.snapshot()?.definition;
            let prepared = self
                .external
                .as_ref()
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            let mut requests = Vec::with_capacity(3);
            for ordinal in 2..=4 {
                let identity = definition.identity(QueryKey::Gateway(ordinal))?;
                let authorized = prepared.prepare_macro_query(identity.clone())?;
                requests.push(codec::Request::capture_prepared_for(
                    &identity,
                    &authorized,
                )?);
            }
            self.live.settle_historical_rejection(
                requests
                    .try_into()
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
                prepared.endpoint_uri(),
            )?;
        }
        self.snapshot()
    }
    fn checkpoint(&mut self) -> anyhow::Result<()> {
        self.live.checkpoint()
    }
    fn now(&self) -> i64 {
        self.live.now()
    }
    fn candidate_eligible(&mut self, ordinal: u32) -> anyhow::Result<bool> {
        Ok(self
            .live
            .snapshot()?
            .definition
            .candidates
            .iter()
            .find(|candidate| candidate.ordinal == ordinal)
            .ok_or(ChainPostCloseError::SchemaRejected)?
            .eligible)
    }
    fn admit(
        &mut self,
        step: Step,
    ) -> anyhow::Result<LocalBoxFuture<'clock, anyhow::Result<Returned<AdapterTicket, Material>>>>
    {
        let qualification = matches!(step, Step::Health | Step::Capabilities)
            && self
                .live
                .current()
                .and_then(|recovery| recovery.readiness_episodes.first())
                .and_then(|episode| {
                    episode
                        .controls
                        .get(if step == Step::Health { 0 } else { 1 })
                })
                .is_some_and(|control| {
                    control.outcome == Some(super::macro_stage::MacroControlOutcome::Ready)
                });
        let (ticket, attempt) = match step {
            Step::Data { query, attempt } => {
                if self.retired_layout && query == QueryKey::Gateway(5) {
                    return Err(ChainPostCloseError::RetiredEconomicCalendarAttemptBlocked.into());
                }
                let snapshot = self.live.snapshot()?;
                let identity = snapshot.definition.identity(query)?;
                if let crate::grpc_client::client::macro_attempt::MacroQueryIdentity::SemanticSearch { provider,query:ref text,limit } = identity {
                    if crate::data_gateway::general_web_research::validate_request(provider,text,limit).is_err() {
                        self.live.reject_request(query)?;
                        return Ok(async move { Ok(Returned { ticket:AdapterTicket::LocalDecision(query),material:Material::LocalDecision }) }.boxed_local());
                    }
                }
                let original = self
                    .live
                    .current()
                    .and_then(|recovery| recovery.full.as_ref())
                    .and_then(|full| full.requests.get(&query));
                let external =
                    matches!(query, QueryKey::Gateway(1..=4)) && snapshot.definition.external_news;
                if external {
                    let prepared = self
                        .external
                        .as_ref()
                        .ok_or(ChainPostCloseError::SchemaRejected)?;
                    let authorized = if let Some(original) = original {
                        prepared.resume_macro_query(
                            identity.clone(),
                            original
                                .request
                                .restored_external(&original.endpoint, attempt),
                        )?
                    } else if let Some(legacy) =
                        self.live.current().filter(|current| current.full.is_none())
                    {
                        codec::require(query == QueryKey::Gateway(1))?;
                        prepared.resume_macro_query(
                            identity.clone(),
                            legacy
                                .plan
                                .request
                                .restored_external(legacy.plan.endpoint(), attempt),
                        )?
                    } else {
                        codec::require(attempt == 1)?;
                        prepared.prepare_macro_query(identity.clone())?
                    };
                    let request = codec::Request::capture_prepared_for(&identity, &authorized)?;
                    let endpoint = prepared.endpoint_uri().to_owned();
                    let client = self
                        .connected_external
                        .as_ref()
                        .ok_or(ChainPostCloseError::SchemaRejected)?;
                    let connection = client.external_connection_identity()?;
                    let attempt =
                        OwnedAttempt::Connected(authorized.bind_connected(client.clone())?);
                    (
                        self.live.begin_data_connection(
                            query,
                            match step {
                                Step::Data { attempt, .. } => attempt,
                                _ => unreachable!(),
                            },
                            request,
                            &endpoint,
                            Some(connection),
                        )?,
                        attempt,
                    )
                } else {
                    let local = self
                        .local
                        .as_ref()
                        .ok_or(ChainPostCloseError::SchemaRejected)?;
                    let session = if let Some(original) = original {
                        local.resume(identity.clone(), original.request.restored(attempt))?
                    } else {
                        codec::require(attempt == 1)?;
                        local.session(identity.clone())?
                    };
                    let authorized = session.authorize_next()?;
                    let request = codec::Request::capture_for(&identity, &authorized)?;
                    let endpoint = local.endpoint().to_owned();
                    (
                        self.live.begin_data(query, attempt, request, &endpoint)?,
                        OwnedAttempt::Connected(authorized),
                    )
                }
            }
            Step::Health | Step::Capabilities => {
                let index = if step == Step::Health { 0 } else { 1 };
                let prepared = self
                    .external
                    .as_ref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let fresh_health = if qualification && step == Step::Health {
                    Some(prepared.prepare_health_attempt()?)
                } else {
                    None
                };
                let fresh_capabilities = if qualification && step == Step::Capabilities {
                    Some(prepared.prepare_capabilities_attempt()?)
                } else {
                    None
                };
                let material = if let Some(health) = &fresh_health {
                    health.request_material()
                } else if let Some(capabilities) = &fresh_capabilities {
                    capabilities.request_material()
                } else {
                    self.live
                        .current()
                        .and_then(|recovery| recovery.readiness_episodes.first())
                        .and_then(|episode| episode.controls.get(index))
                        .ok_or(ChainPostCloseError::SchemaRejected)?
                        .request_material()
                };
                let prepared = self
                    .external
                    .as_ref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let attempt = if step == Step::Health {
                    OwnedAttempt::Health(match fresh_health {
                        Some(health) => health,
                        None => prepared.resume_health_attempt(material.clone())?,
                    })
                } else {
                    let authorized = match fresh_capabilities {
                        Some(capabilities) => capabilities,
                        None => prepared.resume_capabilities_attempt(material.clone())?,
                    };
                    let client = self
                        .connected_external
                        .as_ref()
                        .ok_or(ChainPostCloseError::SchemaRejected)?;
                    OwnedAttempt::Capabilities(authorized.bind_connected(client.clone())?)
                };
                let connection = match &attempt {
                    OwnedAttempt::Health(health) => Some(health.connection_identity()),
                    OwnedAttempt::Capabilities(_) => Some(
                        self.connected_external
                            .as_ref()
                            .ok_or(ChainPostCloseError::SchemaRejected)?
                            .external_connection_identity()?,
                    ),
                    _ => None,
                };
                (
                    if qualification {
                        self.live.begin_qualification_control(
                            material,
                            connection.ok_or(ChainPostCloseError::SchemaRejected)?,
                        )?
                    } else {
                        self.live.begin_control_connection(material, connection)?
                    },
                    attempt,
                )
            }
            Step::Prepare(_) => return Err(ChainPostCloseError::SchemaRejected.into()),
        };
        let clock = self.clock;
        let limit = self.live.limit();
        let deadline = self.live.deadline();
        let lease_until = self.live.lease_until();
        let intent = self.intent.clone();
        Ok(async move {
            let check = || {
                let now = clock.now().get();
                if now >= deadline || now >= lease_until || tokio::time::Instant::now() >= limit {
                    Err(anyhow::Error::new(PreparationStop::ResultUnconfirmed {
                        intent_id: intent.clone(),
                    }))
                } else {
                    Ok(())
                }
            };
            check()?;
            let call = async move {
                Ok::<Material, anyhow::Error>(match attempt {
                    OwnedAttempt::Connected(attempt) => Material::Data(
                        ExternalMacroAttemptCompletion::Unary(attempt.execute().await),
                    ),
                    OwnedAttempt::Health(attempt) => Material::Health(attempt.execute().await),
                    OwnedAttempt::Capabilities(attempt) => {
                        Material::Capabilities(attempt.execute().await?)
                    }
                })
            };
            let material = tokio::time::timeout_at(limit, call).await.map_err(|_| {
                PreparationStop::ResultUnconfirmed {
                    intent_id: intent.clone(),
                }
            })??;
            check()?;
            Ok(Returned {
                ticket: if qualification {
                    AdapterTicket::Qualification(ticket)
                } else {
                    AdapterTicket::Effect(ticket)
                },
                material,
            })
        }
        .boxed_local())
    }
    fn record(
        &mut self,
        step: Step,
        returned: Returned<AdapterTicket, Material>,
    ) -> anyhow::Result<Snapshot> {
        match (step, returned.ticket, returned.material) {
            (Step::Health, AdapterTicket::Qualification(ticket), Material::Health(completion)) => {
                let identity = completion
                    .connection_identity()
                    .cloned()
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let qualified = self.live.record_qualification_control(
                    ticket,
                    codec::ControlRawResult::capture_health(&completion),
                    identity,
                )?;
                codec::require(qualified)?;
                self.connected_external = completion.into_connected_client();
                self.snapshot()
            }
            (
                Step::Capabilities,
                AdapterTicket::Qualification(ticket),
                Material::Capabilities(completion),
            ) => {
                self.connected_external
                    .as_ref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?
                    .external_connection_identity()?;
                let identity = completion
                    .connection_identity()
                    .cloned()
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let qualified = self.live.record_qualification_control(
                    ticket,
                    codec::ControlRawResult::capture_capabilities(&completion),
                    identity,
                )?;
                codec::require(qualified)?;
                self.connected_external = completion.into_connected_client();
                self.current_capabilities_confirmed = true;
                self.snapshot()
            }
            (Step::Data { .. }, AdapterTicket::Effect(ticket), Material::Data(completion)) => {
                if matches!(
                    step,
                    Step::Data {
                        query: QueryKey::Gateway(1..=4),
                        ..
                    }
                ) && self.external.is_some()
                {
                    self.connected_external
                        .as_ref()
                        .ok_or(ChainPostCloseError::SchemaRejected)?
                        .external_connection_identity()?;
                }
                self.live.record_data(ticket, &completion)
            }
            (Step::Health, AdapterTicket::Effect(ticket), Material::Health(completion)) => {
                self.live.record_control_connection(
                    ticket,
                    codec::ControlRawResult::capture_health(&completion),
                    completion.connection_identity().cloned(),
                )?;
                self.connected_external = completion.into_connected_client();
                self.snapshot()
            }
            (
                Step::Capabilities,
                AdapterTicket::Effect(ticket),
                Material::Capabilities(completion),
            ) => {
                self.connected_external
                    .as_ref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?
                    .external_connection_identity()?;
                self.live.record_control_connection(
                    ticket,
                    codec::ControlRawResult::capture_capabilities(&completion),
                    completion.connection_identity().cloned(),
                )?;
                self.current_capabilities_confirmed = completion.processed().is_ok();
                self.connected_external = completion.into_connected_client();
                self.snapshot()
            }
            (
                Step::Data { query, .. },
                AdapterTicket::LocalDecision(actual),
                Material::LocalDecision,
            ) if query == actual => self.live.snapshot(),
            _ => Err(ChainPostCloseError::SchemaRejected.into()),
        }
    }
    fn settle(&mut self, _query: QueryKey) -> anyhow::Result<Snapshot> {
        self.snapshot()
    }
    fn wait(&self, due: i64) -> LocalBoxFuture<'clock, anyhow::Result<()>> {
        let clock = self.clock;
        let limit = self.live.limit();
        // The owner checkpoint immediately after this wake classifies expiry
        // against its exact live U/A, including the other four retry lanes.
        async move {
            // Wall observations may advance independently of this executor's
            // monotonic clock. Never turn a timer wake into an early due proof.
            loop {
                let now = tokio::time::Instant::now();
                let remaining = due.saturating_sub(clock.now().get());
                if remaining <= 0 || now >= limit {
                    break;
                }
                let remaining = u64::try_from(remaining).unwrap_or(u64::MAX);
                let wake = std::cmp::min(now + Duration::from_micros(remaining), limit);
                tokio::time::sleep_until(wake).await;
            }
            Ok(())
        }
        .boxed_local()
    }
    fn finish_dimension(
        &mut self,
        dimension: u8,
        selected: Option<u32>,
    ) -> anyhow::Result<Snapshot> {
        self.live.finish_dimension(dimension, selected)
    }
    fn close(&mut self, end: RunEnd) -> anyhow::Result<String> {
        self.live.close(end)
    }
}
