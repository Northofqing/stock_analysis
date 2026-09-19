//! V12 transport Adapter. Scheduling, fallback and rendering live in one runner.
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
    macro_attempt::{
        AuthorizedMacroAttempt, AuthorizedPreparedMacroRequest, ExternalMacroAttemptCompletion,
    },
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


pub(super) async fn drive(
    local: &mut LocalChainPostClose<'_>,
    lease: RunLease,
    source: &GrpcSource,
    clock: &dyn MacroObservationClock,
    cancelled: Rc<Cell<bool>>,
    search: &SearchService,
) -> anyhow::Result<(RunLease, String)> {
    let intent = lease.intent_id.as_str().to_owned();
    let result = async {
        let mut live = Live::open(local, lease, clock, cancelled)?;
        if let Some(output) = live
            .current()
            .and_then(|recovery| recovery.stage_final())
            .map(|final_| final_.output_bytes().to_vec())
        {
            return Ok((
                live.into_lease(),
                String::from_utf8(output).map_err(|_| ChainPostCloseError::SchemaRejected)?,
            ));
        }
        // This is deliberately after the durable Unknown/final checks.
        let route = source.prepare_macro_queries()?;
        let local_queries = match &route {
            PreparedMacroQueries::Local(local) => Some(local.clone()),
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
            if let Some(local) = &local_queries {
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
            if let Some(local) = &local_queries {
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
        };
        let output = runner::run(&mut adapter).await?;
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
}

enum AdapterTicket {
    Effect(Ticket),
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
    Prepared(AuthorizedPreparedMacroRequest),
    Health(AuthorizedHealthAttempt),
    Capabilities(AuthorizedCapabilitiesAttempt),
}

impl<'local, 'store, 'clock> MacroStepIo for Durable<'local, 'store, 'clock> {
    type Ticket = AdapterTicket;
    type Material = Material;
    type Call = LocalBoxFuture<'clock, anyhow::Result<Returned<AdapterTicket, Material>>>;
    type Wait = LocalBoxFuture<'clock, anyhow::Result<()>>;
    fn open(&mut self) -> anyhow::Result<Snapshot> {
        self.live.settle_legacy_local_unavailable()?;
        if self.live.needs_historical_rejection() {
            self.live.checkpoint()?;
            let definition = self.live.snapshot()?.definition;
            let prepared = self.external.as_ref().ok_or(ChainPostCloseError::SchemaRejected)?;
            let mut requests = Vec::with_capacity(3);
            for ordinal in 2..=4 {
                let identity = definition.identity(QueryKey::Gateway(ordinal))?;
                let authorized = prepared.prepare_macro_query(identity.clone())?;
                requests.push(codec::Request::capture_prepared_for(&identity, &authorized)?);
            }
            self.live.settle_historical_rejection(
                requests.try_into().map_err(|_| ChainPostCloseError::SchemaRejected)?,
                prepared.endpoint_uri(),
            )?;
        }
        self.live.snapshot()
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
        let (ticket, attempt) = match step {
            Step::Data { query, attempt } => {
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
                    } else {
                        codec::require(attempt == 1)?;
                        prepared.prepare_macro_query(identity.clone())?
                    };
                    let request = codec::Request::capture_prepared_for(&identity, &authorized)?;
                    let endpoint = prepared.endpoint_uri().to_owned();
                    let attempt = if let Some(client) = &self.connected_external {
                        OwnedAttempt::Connected(authorized.bind_connected(client.clone())?)
                    } else {
                        OwnedAttempt::Prepared(authorized)
                    };
                    (
                        self.live.begin_data(
                            query,
                            match step {
                                Step::Data { attempt, .. } => attempt,
                                _ => unreachable!(),
                            },
                            request,
                            &endpoint,
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
                let material = self
                    .live
                    .current()
                    .and_then(|recovery| recovery.readiness_episodes.first())
                    .and_then(|episode| episode.controls.get(index))
                    .ok_or(ChainPostCloseError::SchemaRejected)?
                    .request_material();
                let prepared = self
                    .external
                    .as_ref()
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let attempt = if step == Step::Health {
                    OwnedAttempt::Health(prepared.resume_health_attempt(material.clone())?)
                } else {
                    let authorized = prepared.resume_capabilities_attempt(material.clone())?;
                    OwnedAttempt::Capabilities(if let Some(client) = &self.connected_external {
                        authorized.bind_connected(client.clone())?
                    } else {
                        authorized
                    })
                };
                (self.live.begin_control(material)?, attempt)
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
                    OwnedAttempt::Prepared(attempt) => Material::Data(attempt.execute().await?),
                    OwnedAttempt::Health(attempt) => Material::Health(attempt.execute().await),
                    OwnedAttempt::Capabilities(attempt) => {
                        Material::Capabilities(attempt.execute().await)
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
                ticket: AdapterTicket::Effect(ticket),
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
            (Step::Data { .. }, AdapterTicket::Effect(ticket), Material::Data(completion)) => {
                self.live.record_data(ticket, &completion)
            }
            (Step::Health, AdapterTicket::Effect(ticket), Material::Health(completion)) => {
                let snapshot = self
                    .live
                    .record_control(ticket, codec::ControlRawResult::capture_health(&completion))?;
                self.connected_external = completion.into_connected_client();
                Ok(snapshot)
            }
            (
                Step::Capabilities,
                AdapterTicket::Effect(ticket),
                Material::Capabilities(completion),
            ) => {
                let snapshot = self.live.record_control(
                    ticket,
                    codec::ControlRawResult::capture_capabilities(&completion),
                )?;
                self.connected_external = completion.into_connected_client();
                Ok(snapshot)
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
        self.live.snapshot()
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
