//! Caller-owned Legacy Adapter for the same complete Macro schedule.
use super::{
    runner::{
        self, BudgetMode, Candidate, Definition, Dimension, MacroStepIo, QueryKey, QueryOutcome,
        Returned, Route, RouteState, RunEnd, Snapshot, Step,
    },
    NativeOutcome,
};
use crate::data_gateway::grpc_source::{
    self,
    macro_legacy::{self, LegacyExternalRoute},
    macro_queries::ConnectedMacroQueries,
    GrpcSource,
};
use crate::data_gateway::{GatewayError, GlobalNewsProvider};
use crate::grpc_client::client::{
    external_control_attempt::ExternalControlCompletion,
    macro_attempt::{
        ExternalMacroAttemptCompletion, MacroContinuation, MacroQueryIdentity,
        RestoredExternalMacroRequest, RestoredMacroRequest,
    },
    ContractProfile,
};
use crate::grpc_client::external_pb::magic::market::v1::{CapabilitiesResponse, HealthResponse};
use crate::search_service::{service::RegisteredProvider, SearchResponse, SearchService};
use futures::{future::BoxFuture, FutureExt};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

pub(in crate::search_service) async fn search(
    service: &SearchService,
    max_results: usize,
) -> String {
    let source = grpc_source::bridge_for("GlobalNews");
    let external = source
        .as_ref()
        .is_ok_and(|source| source.legacy_macro_external_news());
    let registrations = service.macro_registrations();
    let candidates = registrations
        .iter()
        .enumerate()
        .map(|(index, entry)| Candidate {
            ordinal: u32::try_from(index + 1).unwrap_or(u32::MAX),
            provider: entry.general_web_identity,
            eligible: true,
        })
        .collect();
    let mut adapter = Legacy {
        registrations,
        source: source.clone().ok(),
        local: None,
        external: None,
        requests: BTreeMap::new(),
        audited: BTreeSet::new(),
        started: tokio::time::Instant::now(),
        version: 0,
        state: Snapshot {
            budget: BudgetMode::CallerOwned,
            definition: Definition {
                observed_date: chrono::Local::now().format("%Y年%m月%d日").to_string(),
                research_limit: max_results.min(3),
                candidates,
                external_news: external,
            },
            local: RouteState::Unprepared,
            external: if external {
                RouteState::Unprepared
            } else {
                RouteState::Ready
            },
            queries: BTreeMap::new(),
            dimensions: BTreeMap::new(),
            final_output: None,
        },
    };
    if let Err(error) = source {
        adapter.reject_route(Route::Local, error);
    }
    match runner::run(&mut adapter).await {
        Ok(output) => output,
        Err(error) => {
            log::warn!("[宏观新闻] shared Legacy schedule stopped: {error}");
            String::new()
        }
    }
}

struct Legacy<'providers> {
    registrations: &'providers [RegisteredProvider],
    source: Option<Arc<GrpcSource>>,
    local: Option<ConnectedMacroQueries>,
    external: Option<LegacyExternalRoute>,
    requests: BTreeMap<QueryKey, Request>,
    audited: BTreeSet<QueryKey>,
    started: tokio::time::Instant,
    version: u64,
    state: Snapshot,
}

struct Request {
    bytes: Vec<u8>,
    id: String,
    profile: ContractProfile,
    authority: Option<String>,
    policy: (u32, u64, u64, u64),
}
impl Request {
    fn restore(&self, attempt: u32) -> RestoredMacroRequest {
        RestoredMacroRequest {
            request_bytes: self.bytes.clone(),
            request_id: self.id.clone(),
            profile: self.profile,
            acquisition_authority: self.authority.clone(),
            retry_policy: self.policy,
            next_attempt: attempt,
        }
    }
    fn connected(
        attempt: &crate::grpc_client::client::macro_attempt::AuthorizedMacroAttempt,
    ) -> Self {
        Self {
            bytes: attempt.request_bytes(),
            id: attempt.request_id().to_owned(),
            profile: if attempt.profile() == "ExternalV1" {
                ContractProfile::ExternalV1
            } else {
                ContractProfile::LocalBridgeV1
            },
            authority: attempt.acquisition_authority().map(str::to_owned),
            policy: attempt.retry_policy(),
        }
    }
    fn prepared(
        attempt: &crate::grpc_client::client::macro_attempt::AuthorizedPreparedMacroRequest,
    ) -> Self {
        Self {
            bytes: attempt.request_bytes(),
            id: attempt.request_id().to_owned(),
            profile: attempt.profile(),
            authority: Some(attempt.acquisition_authority().to_owned()),
            policy: attempt.retry_policy(),
        }
    }
}

enum Material {
    Local(Result<ConnectedMacroQueries, GatewayError>),
    External(Result<LegacyExternalRoute, GatewayError>),
    Health(ExternalControlCompletion<HealthResponse>),
    Capabilities(ExternalControlCompletion<CapabilitiesResponse>),
    Data {
        query: QueryKey,
        outcome: NativeOutcome,
        continuation: MacroContinuation,
    },
    Compat {
        query: QueryKey,
        response: SearchResponse,
    },
}

impl Legacy<'_> {
    fn terminal(&mut self, query: QueryKey, outcome: QueryOutcome) {
        self.version += 1;
        let at = self.now();
        let state = self.state.queries.entry(query).or_default();
        state.terminal = Some(outcome);
        state.terminal_at = Some(at);
        state.terminal_version = Some(self.version);
        state.retry_due = None;
    }
    fn reject_route(&mut self, route: Route, error: GatewayError) {
        if route == Route::External {
            self.state.external = RouteState::Rejected;
            self.external = None;
        } else {
            self.state.local = RouteState::Rejected;
        }
        if route == Route::External || !self.state.definition.external_news {
            for ordinal in 1..=4 {
                self.terminal(
                    QueryKey::Gateway(ordinal),
                    QueryOutcome::Native(NativeOutcome::News(Err(error.clone()))),
                );
            }
        }
        if route == Route::Local {
            self.terminal(
                QueryKey::Gateway(5),
                QueryOutcome::Native(NativeOutcome::Economic(Err(error.clone()))),
            );
            let candidates = self.state.definition.candidates.clone();
            for dimension in 1..=6 {
                for candidate in &candidates {
                    if let Some(provider) = candidate.provider {
                        let error = crate::data_gateway::general_web_research::transport_error(
                            provider,
                            error.clone(),
                        );
                        self.terminal(
                            QueryKey::Web {
                                dimension,
                                candidate: candidate.ordinal,
                            },
                            QueryOutcome::Native(NativeOutcome::Web(Err(error))),
                        );
                    }
                }
            }
        }
    }
}

impl<'providers> MacroStepIo for Legacy<'providers> {
    type Ticket = Step;
    type Material = Material;
    type Call = BoxFuture<'providers, anyhow::Result<Returned<Step, Material>>>;
    type Wait = BoxFuture<'providers, anyhow::Result<()>>;
    fn open(&mut self) -> anyhow::Result<Snapshot> {
        // Initialization can fail before the first scheduling step; settle the
        // five logical Gateway acquisitions just as the old wrappers did.
        for ordinal in 1..=5 {
            let key = QueryKey::Gateway(ordinal);
            if self.state.terminal(key).is_some() {
                self.settle(key)?;
            }
        }
        Ok(self.state.clone())
    }
    fn checkpoint(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn now(&self) -> i64 {
        i64::try_from(self.started.elapsed().as_micros()).unwrap_or(i64::MAX)
    }
    fn candidate_eligible(&mut self, ordinal: u32) -> anyhow::Result<bool> {
        let entry = self
            .registrations
            .get(usize::try_from(
                ordinal
                    .checked_sub(1)
                    .ok_or_else(|| anyhow::anyhow!("Legacy Macro ordinal"))?,
            )?)
            .ok_or_else(|| anyhow::anyhow!("Legacy Macro registration missing"))?;
        Ok(entry.supports_general_web_search() && entry.is_available())
    }
    fn admit(
        &mut self,
        step: Step,
    ) -> anyhow::Result<BoxFuture<'providers, anyhow::Result<Returned<Step, Material>>>> {
        let call: BoxFuture<'providers, anyhow::Result<Material>> = match step {
            Step::Prepare(Route::Local) => {
                let source = self
                    .source
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Legacy Macro bridge missing"))?;
                async move { Ok(Material::Local(source.legacy_macro_local().await)) }.boxed()
            }
            Step::Prepare(Route::External) => {
                let source = self
                    .source
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("Legacy External Macro bridge missing"))?;
                async move { Ok(Material::External(source.legacy_macro_external().await)) }.boxed()
            }
            Step::Health => {
                let route = self
                    .external
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Legacy External route missing"))?;
                match route.prepared.prepare_health_attempt() {
                    Ok(attempt) => {
                        async move { Ok(Material::Health(attempt.execute().await)) }.boxed()
                    }
                    Err(error) => {
                        let error = grpc_source::map_external_connection_error(error);
                        async move { Ok(Material::External(Err(error))) }.boxed()
                    }
                }
            }
            Step::Capabilities => {
                let route = self
                    .external
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Legacy External route missing"))?;
                let attempt = route
                    .prepared
                    .prepare_capabilities_attempt()
                    .and_then(|attempt| {
                        if let Some(client) = &route.client {
                            attempt.bind_connected(client.clone())
                        } else {
                            Ok(attempt)
                        }
                    });
                match attempt {
                    Ok(attempt) => {
                        async move { Ok(Material::Capabilities(attempt.execute().await)) }.boxed()
                    }
                    Err(error) => {
                        let error = grpc_source::map_external_connection_error(error);
                        async move { Ok(Material::External(Err(error))) }.boxed()
                    }
                }
            }
            Step::Data { query, attempt } => {
                if let QueryKey::Web {
                    dimension,
                    candidate,
                } = query
                {
                    let entry = &self.registrations[usize::try_from(candidate - 1)?];
                    if entry.general_web_identity.is_none() {
                        let text = self.state.definition.query(dimension);
                        let limit = self.state.definition.research_limit;
                        return Ok(async move {
                            Ok(Returned {
                                ticket: step,
                                material: Material::Compat {
                                    query,
                                    response: entry.search(&text, limit).await,
                                },
                            })
                        }
                        .boxed());
                    }
                }
                let identity = self.state.definition.identity(query)?;
                if let MacroQueryIdentity::SemanticSearch {
                    provider,
                    query: ref text,
                    limit,
                } = identity
                {
                    if let Err(error) = crate::data_gateway::general_web_research::validate_request(
                        provider, text, limit,
                    ) {
                        return Ok(async move {
                            Ok(Returned {
                                ticket: step,
                                material: Material::Data {
                                    query,
                                    outcome: NativeOutcome::Web(Err(error)),
                                    continuation: MacroContinuation::Terminal,
                                },
                            })
                        }
                        .boxed());
                    }
                }
                let external = matches!(query, QueryKey::Gateway(1..=4))
                    && self.state.definition.external_news;
                if external {
                    let route = self
                        .external
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Legacy External route missing"))?;
                    let authorized = if let Some(request) = self.requests.get(&query) {
                        route.prepared.resume_macro_query(
                            identity.clone(),
                            RestoredExternalMacroRequest {
                                endpoint_uri: route.prepared.endpoint_uri().to_owned(),
                                request: request.restore(attempt),
                            },
                        )
                    } else {
                        route.prepared.prepare_macro_query(identity.clone())
                    };
                    match authorized {
                        Ok(authorized) => {
                            self.requests.insert(query, Request::prepared(&authorized));
                            let connected = route.client.clone();
                            async move {
                                let completion = if let Some(client) = connected {
                                    ExternalMacroAttemptCompletion::Unary(
                                        authorized.bind_connected(client)?.execute().await,
                                    )
                                } else {
                                    authorized.execute().await?
                                };
                                let (outcome, continuation) = match completion {
                                    ExternalMacroAttemptCompletion::Unary(completion) => (
                                        NativeOutcome::project(
                                            &identity,
                                            ContractProfile::ExternalV1,
                                            &completion.processed,
                                        ),
                                        completion.continuation,
                                    ),
                                    ExternalMacroAttemptCompletion::ConnectUnavailable {
                                        error,
                                        continuation,
                                        ..
                                    } => (
                                        NativeOutcome::project(
                                            &identity,
                                            ContractProfile::ExternalV1,
                                            &Err(error),
                                        ),
                                        continuation,
                                    ),
                                };
                                Ok(Material::Data {
                                    query,
                                    outcome,
                                    continuation,
                                })
                            }
                            .boxed()
                        }
                        Err(error) => async move {
                            Ok(Material::Data {
                                query,
                                outcome: NativeOutcome::project(
                                    &identity,
                                    ContractProfile::ExternalV1,
                                    &Err(error),
                                ),
                                continuation: MacroContinuation::Terminal,
                            })
                        }
                        .boxed(),
                    }
                } else {
                    let local = self
                        .local
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Legacy Local route missing"))?;
                    let session = if let Some(request) = self.requests.get(&query) {
                        local.resume(identity.clone(), request.restore(attempt))
                    } else {
                        local.session(identity.clone())
                    };
                    match session.and_then(|session| session.authorize_next()) {
                        Ok(authorized) => {
                            self.requests.insert(query, Request::connected(&authorized));
                            async move {
                                let completion = authorized.execute().await;
                                Ok(Material::Data {
                                    query,
                                    outcome: NativeOutcome::project(
                                        &identity,
                                        ContractProfile::LocalBridgeV1,
                                        &completion.processed,
                                    ),
                                    continuation: completion.continuation,
                                })
                            }
                            .boxed()
                        }
                        Err(error) => async move {
                            Ok(Material::Data {
                                query,
                                outcome: NativeOutcome::project(
                                    &identity,
                                    ContractProfile::LocalBridgeV1,
                                    &Err(error),
                                ),
                                continuation: MacroContinuation::Terminal,
                            })
                        }
                        .boxed(),
                    }
                }
            }
        };
        Ok(async move {
            Ok(Returned {
                ticket: step,
                material: call.await?,
            })
        }
        .boxed())
    }
    fn record(
        &mut self,
        step: Step,
        returned: Returned<Step, Material>,
    ) -> anyhow::Result<Snapshot> {
        if step != returned.ticket {
            anyhow::bail!("Legacy Macro returned another step");
        }
        match returned.material {
            Material::Local(Ok(local)) => {
                self.local = Some(local);
                self.state.local = RouteState::Ready;
            }
            Material::Local(Err(error)) => self.reject_route(Route::Local, error),
            Material::External(Ok(route)) => {
                self.state.external = if route.macro_ready {
                    RouteState::Ready
                } else if route.health_ready {
                    RouteState::NeedsCapabilities
                } else {
                    RouteState::NeedsHealth
                };
                self.external = Some(route);
            }
            Material::External(Err(error)) => self.reject_route(Route::External, error),
            Material::Health(completion) => match macro_legacy::health_outcome(&completion) {
                Ok(()) => {
                    self.external
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("Legacy External route missing"))?
                        .client = completion.into_connected_client();
                    self.state.external = RouteState::NeedsCapabilities;
                }
                Err(error) => self.reject_route(Route::External, error),
            },
            Material::Capabilities(completion) => {
                match macro_legacy::capabilities_outcome(&completion) {
                    Ok(()) => {
                        let route = self
                            .external
                            .as_mut()
                            .ok_or_else(|| anyhow::anyhow!("Legacy External route missing"))?;
                        let client = completion.into_connected_client().ok_or_else(|| {
                            anyhow::anyhow!("Legacy ready capability has no client")
                        })?;
                        let published = route
                            .qualification
                            .take()
                            .map(|qualification| qualification.publish(client.clone()))
                            .transpose();
                        route.client = Some(client);
                        if let Err(error) = published {
                            self.reject_route(Route::External, error);
                        } else {
                            self.state.external = RouteState::Ready;
                        }
                    }
                    Err(error) => self.reject_route(Route::External, error),
                }
            }
            Material::Data {
                query,
                outcome,
                continuation,
            } => match continuation {
                MacroContinuation::Terminal => self.terminal(query, QueryOutcome::Native(outcome)),
                MacroContinuation::Retry { backoff_ms } => {
                    let now = self.now();
                    let state = self.state.queries.entry(query).or_default();
                    state.next_attempt += 1;
                    state.retry_due = Some(
                        now.checked_add(
                            i64::try_from(backoff_ms)?
                                .checked_mul(1000)
                                .ok_or_else(|| anyhow::anyhow!("Legacy retry overflow"))?,
                        )
                        .ok_or_else(|| anyhow::anyhow!("Legacy retry overflow"))?,
                    );
                }
            },
            Material::Compat { query, response } => {
                self.terminal(query, QueryOutcome::LegacyCompat(response))
            }
        }
        Ok(self.state.clone())
    }
    fn settle(&mut self, query: QueryKey) -> anyhow::Result<Snapshot> {
        if !matches!(query, QueryKey::Gateway(_)) || self.audited.contains(&query) {
            return Ok(self.state.clone());
        }
        let outcome = self
            .state
            .queries
            .get_mut(&query)
            .and_then(|state| state.terminal.take())
            .ok_or_else(|| anyhow::anyhow!("Legacy terminal missing"))?;
        let outcome = match (query, outcome) {
            (
                QueryKey::Gateway(ordinal @ 1..=4),
                QueryOutcome::Native(NativeOutcome::News(result)),
            ) => {
                let provider = [
                    GlobalNewsProvider::Eastmoney,
                    GlobalNewsProvider::Cailianpress,
                    GlobalNewsProvider::Jin10,
                    GlobalNewsProvider::ThePaper,
                ][usize::from(ordinal - 1)];
                NativeOutcome::News(crate::data_gateway::global_news::audit_macro_query(
                    provider, 20, result,
                ))
            }
            (QueryKey::Gateway(5), QueryOutcome::Native(NativeOutcome::Economic(result))) => {
                NativeOutcome::Economic(crate::data_gateway::economic_calendar::audit_macro_query(
                    20, None, result,
                ))
            }
            _ => anyhow::bail!("Legacy Gateway terminal identity mismatch"),
        };
        self.state.queries.get_mut(&query).unwrap().terminal = Some(QueryOutcome::Native(outcome));
        self.audited.insert(query);
        Ok(self.state.clone())
    }
    fn wait(&self, due: i64) -> BoxFuture<'providers, anyhow::Result<()>> {
        let delay = u64::try_from(due.saturating_sub(self.now()).max(0)).unwrap_or(0);
        async move {
            tokio::time::sleep(Duration::from_micros(delay)).await;
            Ok(())
        }
        .boxed()
    }
    fn finish_dimension(
        &mut self,
        dimension: u8,
        selected: Option<u32>,
    ) -> anyhow::Result<Snapshot> {
        self.state.dimensions.insert(
            dimension,
            Dimension {
                selected,
                pace_due: self
                    .now()
                    .checked_add(300_000)
                    .ok_or_else(|| anyhow::anyhow!("Legacy dimension pace overflow"))?,
            },
        );
        Ok(self.state.clone())
    }
    fn close(&mut self, end: RunEnd) -> anyhow::Result<String> {
        match end {
            RunEnd::Complete(output) => Ok(output),
            RunEnd::BudgetExpired => {
                anyhow::bail!("Legacy Macro cannot manufacture a durable budget terminal")
            }
        }
    }
}
