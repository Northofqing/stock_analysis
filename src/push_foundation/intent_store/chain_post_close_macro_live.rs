//! One live owner for the full Macro journal. No connection escapes an operation.
use super::{
    inspect_run_and_macro_scoped_with_catalog, inspect_run_and_macro_with_catalog,
    macro_codec as codec, macro_native as native, macro_plan_v3 as plan3,
    macro_recovery as recovery, macro_stage as old, preparation_stop, result_unconfirmed, schema,
    storage, ChainPostCloseError, EffectGuard, LocalChainPostClose, RunLease, RunRecovery,
};
use crate::database::data_acquisition_audit::{
    append_acquisition_in_transaction, DataAcquisitionAuditReceipt,
};
use crate::grpc_client::client::{
    external_control_attempt::{ExternalControlKind, ExternalControlRequestMaterial},
    macro_attempt::MacroContinuation,
};
use crate::monitor::push_job::{raw_digest, UtcMicros};
use crate::pipeline::chain_analysis::preparation::{
    MacroConfirmedCommit, MacroObservationClock, PreparationStop,
};
use crate::search_service::{
    macro_news::{
        runner::{BudgetExpired, QueryKey, RunEnd, Snapshot, Step},
        NativeOutcome,
    },
    service::MacroWebSnapshot,
};
use chrono::{DateTime, FixedOffset};
use codec::{require, Result};
use rusqlite::{params_from_iter, types::Value, Transaction, TransactionBehavior};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::Duration,
};

/// Not Clone and not constructible outside this module. Dropping an armed
/// ticket marks this preparation owner unusable; reopen relies on journal U.
pub(super) struct Ticket {
    step: Step,
    begin: u64,
    guard: EffectGuard,
}

pub(super) struct Live<'local, 'store, 'clock> {
    local: &'local mut LocalChainPostClose<'store>,
    lease: RunLease,
    clock: &'clock dyn MacroObservationClock,
    cancelled: Rc<Cell<bool>>,
    active: BTreeMap<Step, u64>,
    finalizing: Option<u64>,
    started: i64,
    deadline: i64,
    opened_wall_at: i64,
    opened_monotonic: tokio::time::Instant,
    limit: tokio::time::Instant,
    current: Option<old::MacroRecovery>,
    #[cfg(test)]
    gateway_a_begin_wall_expiry: Option<(GatewayABeginExpiryPhase, &'clock Cell<UtcMicros>)>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum GatewayABeginExpiryPhase {
    BeforeCommit,
    AfterCommit,
}

struct Gate<'clock> {
    clock: &'clock dyn MacroObservationClock,
    intent: String,
    deadline: i64,
    limit: tokio::time::Instant,
    lease_until: i64,
    unresolved: bool,
}
impl Gate<'_> {
    fn sample(&self, allow_expired: bool) -> anyhow::Result<UtcMicros> {
        let now = self.clock.now();
        if now.get() >= self.lease_until {
            return Err(preparation_stop(
                ChainPostCloseError::StaleLease {
                    intent_id: self.intent.clone(),
                },
                &self.intent,
            ));
        }
        if !allow_expired
            && (now.get() >= self.deadline || tokio::time::Instant::now() >= self.limit)
        {
            return Err(if self.unresolved {
                PreparationStop::ResultUnconfirmed {
                    intent_id: self.intent.clone(),
                }
                .into()
            } else {
                BudgetExpired.into()
            });
        }
        Ok(now)
    }
    fn after_commit(
        &self,
        receipt: MacroConfirmedCommit,
        allow_expired: bool,
    ) -> anyhow::Result<()> {
        let now = self.clock.now();
        if !allow_expired
            && (now.get() >= self.deadline || tokio::time::Instant::now() >= self.limit)
        {
            return Err(PreparationStop::DeadlineAfterConfirmedCommit { receipt }.into());
        }
        if now.get() >= self.lease_until {
            return Err(preparation_stop(
                ChainPostCloseError::StaleLease {
                    intent_id: self.intent.clone(),
                },
                &self.intent,
            ));
        }
        Ok(())
    }
}

impl<'local, 'store, 'clock> Live<'local, 'store, 'clock> {
    pub(super) fn open(
        local: &'local mut LocalChainPostClose<'store>,
        lease: RunLease,
        clock: &'clock dyn MacroObservationClock,
        cancelled: Rc<Cell<bool>>,
    ) -> anyhow::Result<Self> {
        let entered = clock.now();
        let monotonic = tokio::time::Instant::now();
        let transaction = local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("full Macro open"))?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) =
            inspect_run_and_macro_with_catalog(&transaction, &lease.intent_id, &catalog)?;
        old::validate_admission(&transaction, &lease, entered, &run)?;
        // Unknown wins over unsupported old plan, date, registry or route access.
        if current
            .as_ref()
            .is_some_and(old::MacroRecovery::has_unconfirmed_effect)
        {
            // Route through the canonical mapping rather than raising the stop directly,
            // matching the v11 driver for this same predicate
            // (chain_post_close_macro_driver_v11.rs). preparation_stop attaches the domain
            // error as the root cause, so the source chain carries
            // ChainPostCloseError::IncompleteEffect. Not every IncompleteOnReopen site is
            // shaped this way -- e.g. LocalConceptBatchPreparationIo::concepts raises a
            // bare stop -- so this is consistency with the v11 driver, not a universal
            // invariant.
            return Err(preparation_stop(
                ChainPostCloseError::IncompleteEffect {
                    intent_id: lease.intent_id.as_str().to_owned(),
                },
                lease.intent_id.as_str(),
            ));
        }
        transaction.commit()
            .map_err(|_| storage("full Macro open commit"))?;
        if let Some(recovery) = &current {
            if recovery.full.is_none() {
                plan3::legacy_local_route(&recovery.plan)?;
            }
        }
        let started = current
            .as_ref()
            .map_or(entered.get(), |recovery| recovery.plan.started);
        let deadline = current
            .as_ref()
            .map(|recovery| recovery.plan.deadline)
            .unwrap_or(
                started
                    .checked_add(15_000_000)
                    .ok_or(ChainPostCloseError::SchemaRejected)?,
            );
        let remaining = u64::try_from(deadline.checked_sub(entered.get())
            .ok_or(ChainPostCloseError::SchemaRejected)?.max(0))
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let limit = monotonic.checked_add(Duration::from_micros(remaining))
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        Ok(Self {
            local,
            lease,
            clock,
            cancelled,
            active: BTreeMap::new(),
            finalizing: None,
            started,
            deadline,
            opened_wall_at: entered.get(),
            opened_monotonic: monotonic,
            limit,
            current,
            #[cfg(test)]
            gateway_a_begin_wall_expiry: None,
        })
    }

    #[cfg(test)]
    pub(super) fn arm_gateway_a_begin_wall_expiry_for_test(
        &mut self,
        clock: &'clock Cell<UtcMicros>,
    ) {
        assert!(self.gateway_a_begin_wall_expiry.is_none(), "TEST_CODE expiry already armed");
        self.gateway_a_begin_wall_expiry = Some((GatewayABeginExpiryPhase::BeforeCommit, clock));
    }

    #[cfg(test)]
    pub(super) fn arm_gateway_a_begin_late_wall_expiry_for_test(
        &mut self,
        clock: &'clock Cell<UtcMicros>,
    ) {
        assert!(self.gateway_a_begin_wall_expiry.is_none(), "TEST_CODE expiry already armed");
        self.gateway_a_begin_wall_expiry = Some((GatewayABeginExpiryPhase::AfterCommit, clock));
    }

    fn gate(&self) -> Gate<'clock> {
        Gate {
            clock: self.clock,
            intent: self.lease.intent_id.as_str().to_owned(),
            deadline: self.deadline,
            limit: self.limit,
            lease_until: self.lease.until.get(),
            unresolved: !self.active.is_empty() || self.finalizing.is_some(),
        }
    }
    pub(super) fn current(&self) -> Option<&old::MacroRecovery> {
        self.current.as_ref()
    }
    pub(super) fn snapshot(&self) -> anyhow::Result<Snapshot> {
        Ok(self
            .current
            .as_ref()
            .and_then(|recovery| recovery.full.as_ref())
            .ok_or(ChainPostCloseError::MacroNotStarted)?
            .snapshot
            .clone())
    }
    pub(super) fn now(&self) -> i64 {
        self.clock.now().get()
    }
    pub(super) fn limit(&self) -> tokio::time::Instant {
        self.limit
    }
    pub(super) fn deadline(&self) -> i64 {
        self.deadline
    }
    pub(super) fn lease_until(&self) -> i64 {
        self.lease.until.get()
    }
    pub(super) fn into_lease(self) -> RunLease {
        self.lease
    }

    pub(super) fn checkpoint(&mut self) -> anyhow::Result<()> {
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: self.lease.intent_id.as_str().to_owned(),
            }
            .into());
        }
        let gate = self.gate();
        gate.sample(false)?;
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|_| storage("Macro checkpoint"))?;
        let now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (_, current) = admitted(
            &transaction,
            &catalog,
            &self.lease,
            now,
            &self.active,
            self.finalizing,
        )?;
        transaction.commit()
            .map_err(|_| storage("Macro checkpoint commit"))?;
        self.current = current;
        gate.sample(false)?;
        Ok(())
    }

    pub(super) fn initialize(
        &mut self,
        observation: DateTime<FixedOffset>,
        endpoint: &str,
        requests: Vec<(QueryKey, codec::Request, String)>,
        episode: Option<codec::ReadinessEpisodePlan>,
        web: &MacroWebSnapshot,
    ) -> anyhow::Result<()> {
        let gate = self.gate();
        gate.sample(false)?;
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("full Macro plan begin"))?;
        let now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current, parent) = inspect_run_and_macro_scoped_with_catalog(
            &transaction,
            &self.lease.intent_id,
            &catalog,
            |run, current, validated| {
                old::validate_admission(&transaction, &self.lease, now, run)?;
                require(current.is_none())?;
                super::dragon_tiger::macro_parent_from_validated(
                    &transaction,
                    &self.lease.intent_id,
                    run,
                    &validated,
                )
            },
        )?;
        require(current.is_none() && self.active.is_empty())?;
        let parent = parent.ok_or(ChainPostCloseError::SchemaRejected)?;
        let first = requests
            .iter()
            .find(|(key, _, _)| *key == QueryKey::Gateway(1))
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let plan = plan3::PlanV3::new(
            &parent,
            UtcMicros::try_new(self.started).map_err(|_| ChainPostCloseError::SchemaRejected)?,
            observation,
            endpoint,
            first.1.clone(),
            web,
        )?;
        let bytes = codec::encode(&plan)?;
        let plan_sha = raw_digest(&bytes).as_str().to_owned();
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        let plan_version = writer.append(Body::Plan {
            bytes: &bytes,
            plan: &plan,
        })?;
        let definition = plan3::definition(&plan.core)?;
        let mut request_versions = BTreeMap::new();
        for (query, request, route_endpoint) in &requests {
            let value = plan3::RequestPlan {
                version: 2,
                query: *query,
                request: request.clone(),
            };
            value.validate(&definition, &plan.local_route, endpoint, route_endpoint)?;
            let version = writer.append(Body::Request {
                value: &value,
                plan_version,
                endpoint: route_endpoint,
            })?;
            require(request_versions.insert(*query, version).is_none())?;
        }
        if let Some(episode) = &episode {
            episode.validate()?;
            require(
                definition.external_news
                    && (1..=4)
                        .all(|ordinal| request_versions.contains_key(&QueryKey::Gateway(ordinal))),
            )?;
            writer.append(Body::Episode {
                value: episode,
                plan_version,
                request_version: *request_versions
                    .get(&QueryKey::Gateway(1))
                    .ok_or(ChainPostCloseError::SchemaRejected)?,
                endpoint,
                authority: plan
                    .core
                    .acquisition_authority()
                    .ok_or(ChainPostCloseError::SchemaRejected)?,
            })?;
        } else {
            require(!definition.external_news)?;
        }
        if plan.local_route.state == plan3::LocalRouteState::ObservedUnavailable {
            let outcome = native::local_unavailable_outcome(&plan.local_route)?;
            let terminal = native::QueryTerminal {
                version: 2,
                query: QueryKey::Gateway(5),
                plan_version,
                plan_sha256: plan_sha,
                request_plan_version: None,
                request_sha256: None,
                cause: native::TerminalCause::LocalRouteUnavailable,
                native_sha256: raw_digest(&native::native_bytes(&outcome)?)
                    .as_str()
                    .to_owned(),
            };
            writer.terminal(&definition, &terminal, &outcome)?;
        }
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &self.active, None)?;
        gate.sample(false)?;
        transaction.commit()
            .map_err(|_| storage("full Macro plan commit"))?;
        self.lease.head = candidate.head;
        self.current = current;
        gate.after_commit(receipt, false)
    }

    pub(super) fn begin_data(
        &mut self,
        query: QueryKey,
        attempt: u32,
        request: codec::Request,
        endpoint: &str,
    ) -> anyhow::Result<Ticket> {
        if self.cancelled.get() {
            return Err(PreparationStop::ResultUnconfirmed {
                intent_id: self.lease.intent_id.as_str().to_owned(),
            }
            .into());
        }
        let gate = self.gate();
        gate.sample(false)?;
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("full Macro data begin"))?;
        let checkpoint_now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) = admitted(
            &transaction,
            &catalog,
            &self.lease,
            checkpoint_now,
            &self.active,
            self.finalizing,
        )?;
        self.current = current;
        gate.sample(false)?;
        let step = Step::Data { query, attempt };
        require(!self.active.contains_key(&step) && self.active.len() < 5)?;
        let now = gate.sample(false)?;
        validate_admitted(
            &transaction,
            &self.lease,
            now,
            &run,
            &self.current,
            &self.active,
            None,
        )?;
        let current = self
            .current
            .as_ref()
            .ok_or(ChainPostCloseError::MacroNotStarted)?;
        let full = current
            .full
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let identity = full.snapshot.definition.identity(query)?;
        request.validate_for(&identity)?;
        let state = full
            .snapshot
            .queries
            .get(&query)
            .cloned()
            .unwrap_or_default();
        require(
            state.terminal.is_none()
                && state.next_attempt == attempt
                && state.retry_due.is_none_or(|due| due <= now.get()),
        )?;
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        let request_version = if let Some(original) = full.requests.get(&query) {
            require(
                original.endpoint == endpoint
                    && codec::encode(&original.request)? == codec::encode(&request)?,
            )?;
            original.fact.version
        } else {
            let value = plan3::RequestPlan {
                version: 2,
                query,
                request: request.clone(),
            };
            value.validate(
                &full.snapshot.definition,
                &full.local,
                current.plan.endpoint(),
                endpoint,
            )?;
            writer.append(Body::Request {
                value: &value,
                plan_version: current.plan_version,
                endpoint,
            })?
        };
        let ready = if request.contract_profile()
            == crate::grpc_client::client::ContractProfile::ExternalV1
        {
            Some(
                current
                    .readiness_episodes
                    .first()
                    .and_then(|episode| episode.ready_result_version())
                    .ok_or(ChainPostCloseError::SchemaRejected)?,
            )
        } else {
            None
        };
        let value = native::DataBegin {
            version: 2,
            query,
            attempt,
            request_plan_version: request_version,
            request_sha256: raw_digest(&request.bytes).as_str().to_owned(),
            readiness_result_version: ready,
            previous_result_version: current
                .attempts
                .iter()
                .rev()
                .find(|previous| previous.query == query)
                .and_then(|previous| previous.result),
        };
        let begin = writer.append(Body::DataBegin(&value))?;
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let mut expected = self.active.clone();
        require(expected.insert(step, begin).is_none())?;
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &expected, None)?;
        #[cfg(test)]
        if query == QueryKey::Gateway(1) && attempt == 1
            && matches!(self.gateway_a_begin_wall_expiry, Some((GatewayABeginExpiryPhase::BeforeCommit, _)))
        {
            if let Some((_, clock)) = self.gateway_a_begin_wall_expiry.take() {
                clock.set(UtcMicros::try_new(self.deadline).expect("TEST_CODE original deadline"));
            }
        }
        gate.sample(false)?;
        transaction.commit()
            .map_err(|_| storage("full Macro data begin commit"))?;
        self.lease.head = candidate.head;
        self.active = expected;
        self.current = current;
        // Mint only after confirmation. A late commit still leaves a real U and
        // must hard-stop with this receipt, never authorize an effect poll.
        let ticket = Ticket {
            step,
            begin,
            guard: EffectGuard::new(Rc::clone(&self.cancelled)),
        };
        #[cfg(test)]
        if query == QueryKey::Gateway(1) && attempt == 1
            && matches!(self.gateway_a_begin_wall_expiry, Some((GatewayABeginExpiryPhase::AfterCommit, _)))
        {
            if let Some((_, clock)) = self.gateway_a_begin_wall_expiry.take() {
                clock.set(UtcMicros::try_new(self.deadline).expect("TEST_CODE original deadline"));
            }
        }
        gate.after_commit(receipt, false)?;
        Ok(ticket)
    }

    pub(super) fn begin_control(
        &mut self,
        material: ExternalControlRequestMaterial,
    ) -> anyhow::Result<Ticket> {
        self.checkpoint()?;
        let (step, ordinal, kind) = match material.kind {
            ExternalControlKind::Health => (Step::Health, 1, "Health"),
            ExternalControlKind::Capabilities => (Step::Capabilities, 2, "Capabilities"),
        };
        require(!self.active.contains_key(&step) && self.active.len() < 5)?;
        let gate = self.gate();
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("full Macro control begin"))?;
        let now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) =
            admitted(&transaction, &catalog, &self.lease, now, &self.active, None)?;
        let current = current.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let episode = current
            .readiness_episodes
            .first()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let control = episode
            .controls
            .get(ordinal as usize - 1)
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(
            control.begin.is_none()
                && codec::encode(&codec::ControlRequest::capture(material)?)?
                    == codec::encode(&control.request)?,
        )?;
        let health_result = if ordinal == 2 {
            require(episode.controls[0].outcome == Some(old::MacroControlOutcome::Ready))?;
            Some(
                episode.controls[0]
                    .result
                    .ok_or(ChainPostCloseError::SchemaRejected)?,
            )
        } else {
            None
        };
        let value = old::ControlBegin {
            version: 1,
            episode_ordinal: 1,
            control_ordinal: ordinal,
            kind: kind.to_owned(),
            episode_sha256: raw_digest(&codec::encode(&episode.plan)?)
                .as_str()
                .to_owned(),
            request_sha256: raw_digest(control.request.request_bytes())
                .as_str()
                .to_owned(),
            health_result_version: health_result,
        };
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        let begin = writer.append(Body::ControlBegin {
            value: &value,
            episode_version: episode.plan_version,
        })?;
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let mut expected = self.active.clone();
        require(expected.insert(step, begin).is_none())?;
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &expected, None)?;
        gate.sample(false)?;
        transaction
            .commit()
            .map_err(|_| storage("full Macro control begin commit"))?;
        self.lease.head = candidate.head;
        self.active = expected;
        self.current = current;
        let ticket = Ticket {
            step,
            begin,
            guard: EffectGuard::new(Rc::clone(&self.cancelled)),
        };
        gate.after_commit(receipt, false)?;
        Ok(ticket)
    }

    pub(super) fn record_data(
        &mut self,
        mut ticket: Ticket,
        completion: &crate::grpc_client::client::macro_attempt::ExternalMacroAttemptCompletion,
    ) -> anyhow::Result<Snapshot> {
        let Step::Data { query, attempt } = ticket.step else {
            return Err(ChainPostCloseError::SchemaRejected.into());
        };
        require(self.active.get(&ticket.step) == Some(&ticket.begin))?;
        let gate = self.gate();
        gate.sample(false)?;
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("full Macro result begin"))?;
        let now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) =
            admitted(&transaction, &catalog, &self.lease, now, &self.active, None)?;
        let current = current.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let full = current
            .full
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let request = &full
            .requests
            .get(&query)
            .ok_or(ChainPostCloseError::SchemaRejected)?
            .request;
        let original = current
            .attempts
            .iter()
            .find(|original| {
                original.query == query
                    && original.ordinal == attempt
                    && original.begin == ticket.begin
                    && original.result.is_none()
            })
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let provider_catalog = old::historical_provider_catalog(
            &current.readiness_episodes,
            original.readiness_result_version(),
        );
        let identity = full.snapshot.definition.identity(query)?;
        let data =
            native::DataResult::capture_external(
                query,
                &identity,
                request,
                attempt,
                completion,
                provider_catalog,
            )?;
        let (outcome, material) = data.project(&identity, request, provider_catalog)?;
        let retry_due = match material.continuation {
            MacroContinuation::Terminal => None,
            MacroContinuation::Retry { backoff_ms } => Some(
                now.get()
                    .checked_add(
                        i64::try_from(backoff_ms)
                            .ok()
                            .and_then(|ms| ms.checked_mul(1000))
                            .ok_or(ChainPostCloseError::SchemaRejected)?,
                    )
                    .ok_or(ChainPostCloseError::SchemaRejected)?,
            ),
        };
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        let result = writer.append(Body::DataResult {
            value: &data,
            begin: ticket.begin,
            request_sha: raw_digest(&request.bytes).as_str(),
            retry_due,
        })?;
        if retry_due.is_none() {
            let terminal = native::QueryTerminal {
                version: 2,
                query,
                plan_version: current.plan_version,
                plan_sha256: raw_digest(&current.plan_bytes).as_str().to_owned(),
                request_plan_version: Some(
                    full.requests
                        .get(&query)
                        .ok_or(ChainPostCloseError::SchemaRejected)?
                        .fact
                        .version,
                ),
                request_sha256: Some(raw_digest(&request.bytes).as_str().to_owned()),
                cause: native::TerminalCause::DataResult { version: result },
                native_sha256: data.native_sha256.clone(),
            };
            writer.terminal(&full.snapshot.definition, &terminal, &outcome)?;
        }
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let mut expected = self.active.clone();
        require(expected.remove(&ticket.step) == Some(ticket.begin))?;
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &expected, None)?;
        gate.sample(false)?;
        transaction.commit()
            .map_err(|_| PreparationStop::ResultUnconfirmed {
                intent_id: gate.intent.clone(),
            })?;
        self.lease.head = candidate.head;
        self.active = expected;
        self.current = current;
        ticket.guard.disarm();
        gate.after_commit(receipt, false)?;
        self.snapshot()
    }

    pub(super) fn record_control(
        &mut self,
        mut ticket: Ticket,
        raw: codec::ControlRawResult,
    ) -> anyhow::Result<Snapshot> {
        let (ordinal, kind) = match ticket.step {
            Step::Health => (1, "Health"),
            Step::Capabilities => (2, "Capabilities"),
            _ => return Err(ChainPostCloseError::SchemaRejected.into()),
        };
        require(self.active.get(&ticket.step) == Some(&ticket.begin))?;
        let gate = self.gate();
        gate.sample(false)?;
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("full Macro control result begin"))?;
        let now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) =
            admitted(&transaction, &catalog, &self.lease, now, &self.active, None)?;
        let current = current.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let full = current
            .full
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let control = current
            .readiness_episodes
            .first()
            .and_then(|episode| episode.controls.get(ordinal as usize - 1))
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(control.begin == Some(ticket.begin) && control.result.is_none())?;
        let projected = raw.project(&control.request)?;
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        let result = writer.append(Body::ControlResult {
            value: &raw,
            control_ordinal: ordinal,
            kind,
            begin: ticket.begin,
            request_sha: raw_digest(control.request.request_bytes()).as_str(),
            ready: projected.is_ok(),
        })?;
        if let Err(error) = projected {
            for ordinal in 1..=4 {
                let query = QueryKey::Gateway(ordinal);
                require(
                    !full.terminals.contains_key(&query)
                        && !current
                            .attempts
                            .iter()
                            .any(|attempt| attempt.query == query),
                )?;
                let request = full
                    .requests
                    .get(&query)
                    .ok_or(ChainPostCloseError::SchemaRejected)?;
                let outcome = NativeOutcome::News(Err(error.clone()));
                let terminal = native::QueryTerminal {
                    version: 2,
                    query,
                    plan_version: current.plan_version,
                    plan_sha256: raw_digest(&current.plan_bytes).as_str().to_owned(),
                    request_plan_version: Some(request.fact.version),
                    request_sha256: Some(raw_digest(&request.request.bytes).as_str().to_owned()),
                    cause: native::TerminalCause::SharedControlRejected { version: result },
                    native_sha256: raw_digest(&native::native_bytes(&outcome)?)
                        .as_str()
                        .to_owned(),
                };
                writer.terminal(&full.snapshot.definition, &terminal, &outcome)?;
            }
        }
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let mut expected = self.active.clone();
        require(expected.remove(&ticket.step) == Some(ticket.begin))?;
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &expected, None)?;
        gate.sample(false)?;
        transaction
            .commit()
            .map_err(|_| PreparationStop::ResultUnconfirmed {
                intent_id: gate.intent.clone(),
            })?;
        self.lease.head = candidate.head;
        self.active = expected;
        self.current = current;
        ticket.guard.disarm();
        gate.after_commit(receipt, false)?;
        self.snapshot()
    }

    pub(super) fn settle_legacy_local_unavailable(&mut self) -> anyhow::Result<()> {
        let needed = self
            .current
            .as_ref()
            .and_then(|recovery| recovery.full.as_ref())
            .is_some_and(|full| {
                full.local.state == plan3::LocalRouteState::ObservedUnavailable
                    && !full.terminals.contains_key(&QueryKey::Gateway(5))
            });
        if !needed {
            return Ok(());
        }
        self.checkpoint()?;
        let gate = self.gate();
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("legacy Macro Local observation terminal"))?;
        let now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) =
            admitted(&transaction, &catalog, &self.lease, now, &self.active, None)?;
        let current = current.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let full = current
            .full
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(!full.terminals.contains_key(&QueryKey::Gateway(5)))?;
        let outcome = native::local_unavailable_outcome(&full.local)?;
        let value = native::QueryTerminal {
            version: 2,
            query: QueryKey::Gateway(5),
            plan_version: current.plan_version,
            plan_sha256: raw_digest(&current.plan_bytes).as_str().to_owned(),
            request_plan_version: None,
            request_sha256: None,
            cause: native::TerminalCause::LocalRouteUnavailable,
            native_sha256: raw_digest(&native::native_bytes(&outcome)?)
                .as_str()
                .to_owned(),
        };
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        writer.terminal(&full.snapshot.definition, &value, &outcome)?;
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &self.active, None)?;
        gate.sample(false)?;
        transaction
            .commit()
            .map_err(|_| storage("legacy Macro Local observation terminal commit"))?;
        self.lease.head = candidate.head;
        self.current = current;
        gate.after_commit(receipt, false)
    }

    pub(super) fn needs_historical_rejection(&self) -> bool {
        self.current.as_ref().and_then(|current| current.full.as_ref())
            .is_some_and(|full| full.historical_rejection.is_some()
                && !full.terminals.contains_key(&QueryKey::Gateway(2)))
    }

    pub(super) fn settle_historical_rejection(
        &mut self,
        requests: [codec::Request; 3],
        endpoint: &str,
    ) -> anyhow::Result<()> {
        require(self.active.is_empty() && self.finalizing.is_none() && !self.cancelled.get())?;
        self.checkpoint()?;
        let gate = self.gate();
        let transaction = self.local.store.connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("historical Macro rejection begin"))?;
        let now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) = admitted(&transaction, &catalog, &self.lease, now, &self.active, None)?;
        let current = current.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let full = current.full.as_ref().ok_or(ChainPostCloseError::SchemaRejected)?;
        let origin = full.historical_rejection.as_ref().ok_or(ChainPostCloseError::SchemaRejected)?;
        require(full.begin.is_none() && full.final_.is_none()
            && (2..=4).all(|ordinal| !full.requests.contains_key(&QueryKey::Gateway(ordinal))
                && !full.terminals.contains_key(&QueryKey::Gateway(ordinal))))?;
        let outcome = NativeOutcome::News(Err(origin.error.clone()));
        let plan_sha = raw_digest(&current.plan_bytes).as_str().to_owned();
        let native_sha = raw_digest(&native::native_bytes(&outcome)?).as_str().to_owned();
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        let mut request_versions = [0_u64; 3];
        for (index, request) in requests.iter().enumerate() {
            require(request.profile == current.plan.request.profile
                && request.authority == current.plan.request.authority
                && request.policy == current.plan.request.policy)?;
            let value = plan3::RequestPlan {
                version: 2,
                query: QueryKey::Gateway(u8::try_from(index + 2)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?),
                request: request.clone(),
            };
            value.validate(&full.snapshot.definition, &full.local, current.plan.endpoint(), endpoint)?;
            request_versions[index] = writer.append(Body::Request {
                value: &value, plan_version: current.plan_version, endpoint,
            })?;
        }
        for (index, request) in requests.iter().enumerate() {
            let value = native::QueryTerminal {
                version: 2,
                query: QueryKey::Gateway(u8::try_from(index + 2)
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?),
                plan_version: current.plan_version,
                plan_sha256: plan_sha.clone(),
                request_plan_version: Some(request_versions[index]),
                request_sha256: Some(raw_digest(&request.bytes).as_str().to_owned()),
                cause: origin.cause(),
                native_sha256: native_sha.clone(),
            };
            writer.terminal(&full.snapshot.definition, &value, &outcome)?;
        }
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &self.active, None)?;
        gate.sample(false)?;
        transaction.commit().map_err(|_| PreparationStop::ResultUnconfirmed {
            intent_id: gate.intent.clone(),
        })?;
        self.lease.head = candidate.head;
        self.current = current;
        gate.after_commit(receipt, false)
    }

    pub(super) fn reject_request(&mut self, query: QueryKey) -> anyhow::Result<()> {
        self.checkpoint()?;
        let gate = self.gate();
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("Macro request rejection begin"))?;
        let now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) =
            admitted(&transaction, &catalog, &self.lease, now, &self.active, None)?;
        let current = current.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let full = current
            .full
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(!full.requests.contains_key(&query) && !full.terminals.contains_key(&query))?;
        let outcome = native::request_rejected_outcome(&full.snapshot.definition, query)?;
        let value = native::QueryTerminal {
            version: 2,
            query,
            plan_version: current.plan_version,
            plan_sha256: raw_digest(&current.plan_bytes).as_str().to_owned(),
            request_plan_version: None,
            request_sha256: None,
            cause: native::TerminalCause::RequestRejected,
            native_sha256: raw_digest(&native::native_bytes(&outcome)?)
                .as_str()
                .to_owned(),
        };
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        writer.terminal(&full.snapshot.definition, &value, &outcome)?;
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &self.active, None)?;
        gate.sample(false)?;
        transaction
            .commit()
            .map_err(|_| storage("Macro request rejection commit"))?;
        self.lease.head = candidate.head;
        self.current = current;
        gate.after_commit(receipt, false)
    }

    pub(super) fn finish_dimension(
        &mut self,
        dimension: u8,
        selected: Option<u32>,
    ) -> anyhow::Result<Snapshot> {
        self.checkpoint()?;
        require(self.active.is_empty())?;
        let gate = self.gate();
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("Macro dimension begin"))?;
        let now = gate.sample(false)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) =
            admitted(&transaction, &catalog, &self.lease, now, &self.active, None)?;
        let current = current.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let full = current
            .full
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let eligible = full
            .snapshot
            .definition
            .candidates
            .iter()
            .filter(|candidate| candidate.eligible)
            .map(|candidate| candidate.ordinal)
            .collect::<Vec<_>>();
        let last = eligible
            .iter()
            .filter_map(|candidate| {
                full.terminals.get(&QueryKey::Web {
                    dimension,
                    candidate: *candidate,
                })
            })
            .max_by_key(|terminal| terminal.version)
            .map(|terminal| terminal.version);
        let value = native::DimensionTerminal {
            version: 2,
            dimension,
            plan_version: current.plan_version,
            plan_sha256: raw_digest(&current.plan_bytes).as_str().to_owned(),
            selected_candidate: selected,
            last_query_terminal_version: last,
            eligible_candidates: eligible,
            pace_due: now
                .get()
                .checked_add(300_000)
                .ok_or(ChainPostCloseError::SchemaRejected)?,
        };
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        writer.append(Body::Dimension(&value))?;
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &self.active, None)?;
        gate.sample(false)?;
        transaction.commit()
            .map_err(|_| storage("Macro dimension commit"))?;
        self.lease.head = candidate.head;
        self.current = current;
        gate.after_commit(receipt, false)?;
        self.snapshot()
    }

    pub(super) fn close(&mut self, end: RunEnd) -> anyhow::Result<String> {
        require(self.active.is_empty() && self.finalizing.is_none() && !self.cancelled.get())?;
        let (kind, output) = match end {
            RunEnd::Complete(output) => (native::FinalKind::Complete, output),
            RunEnd::BudgetExpired => (native::FinalKind::BudgetExpired, String::new()),
        };
        let expired = kind == native::FinalKind::BudgetExpired;
        let gate = self.gate();
        gate.sample(expired)?;
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("Macro finalize begin"))?;
        let now = gate.sample(expired)?;
        let expiry = if !expired {
            native::ExpiryBasis::None
        } else if now.get() >= self.deadline {
            native::ExpiryBasis::WallDeadline
        } else {
            let elapsed = tokio::time::Instant::now().checked_duration_since(self.opened_monotonic)
                .ok_or(ChainPostCloseError::SchemaRejected)?;
            native::ExpiryBasis::MonotonicRemaining {
                opened_wall_at: self.opened_wall_at,
                elapsed_us: i64::try_from(elapsed.as_micros())
                    .map_err(|_| ChainPostCloseError::SchemaRejected)?,
            }
        };
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) =
            admitted(&transaction, &catalog, &self.lease, now, &self.active, None)?;
        let current = current.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let full = current
            .full
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(full.begin.is_none() && full.final_.is_none())?;
        let value = native::FinalizeBegin {
            version: 3,
            plan_version: current.plan_version,
            plan_sha256: raw_digest(&current.plan_bytes).as_str().to_owned(),
            kind,
            started_at: self.started,
            deadline_at: self.deadline,
            facts_sha256: full.facts_sha256.clone(),
            output: output.into_bytes(),
            output_sha256: String::new(),
            pending: if expired {
                recovery::pending(&full.snapshot)
            } else {
                Vec::new()
            },
            expiry,
        };
        let value = native::FinalizeBegin {
            output_sha256: raw_digest(&value.output).as_str().to_owned(),
            ..value
        };
        value.validate_recorded_at(now.get())?;
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        let begin_version = writer.append(Body::Finalize(&value))?;
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let (_, current) = admitted(
            &transaction,
            &catalog,
            &candidate,
            now,
            &self.active,
            Some(begin_version),
        )?;
        gate.sample(expired)?;
        transaction.commit()
            .map_err(|_| storage("Macro finalize begin commit"))?;
        self.lease.head = candidate.head;
        self.finalizing = Some(begin_version);
        self.current = current;
        gate.after_commit(receipt, expired)?;
        // F has its own lock, fresh CAS and clock observations. No retry or
        // synthetic BudgetExpired is allowed if this second transaction fails.
        self.finish_final(expired)
    }

    fn finish_final(&mut self, expired: bool) -> anyhow::Result<String> {
        let gate = self.gate();
        gate.sample(expired)?;
        let transaction = self
            .local
            .store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| storage("Macro StageFinal begin"))?;
        let now = gate.sample(expired)?;
        let catalog = schema::verify_v12_transaction(&transaction)?;
        let (run, current) = admitted(
            &transaction,
            &catalog,
            &self.lease,
            now,
            &self.active,
            self.finalizing,
        )?;
        let current = current.ok_or(ChainPostCloseError::MacroNotStarted)?;
        let full = current
            .full
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        let (begin_fact, begin) = full
            .begin
            .as_ref()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
        require(
            full.final_.is_none()
                && Some(begin_fact.version) == self.finalizing
                && self.active.is_empty()
                && (begin.kind == native::FinalKind::BudgetExpired) == expired,
        )?;
        let value = native::StageFinal {
            version: 2,
            finalize_begin_version: begin_fact.version,
            finalize_begin_sha256: begin_fact.digest.clone(),
            kind: begin.kind,
            output_sha256: begin.output_sha256.clone(),
        };
        let initial_head = self.lease.head;
        let mut candidate = transaction_lease_candidate(&self.lease);
        let mut writer = Writer::new(&transaction, &mut candidate, &run, now);
        writer.append(Body::Final {
            value: &value,
            begin,
        })?;
        let receipt = writer.receipt(initial_head)?;
        drop(writer);
        let (_, current) = admitted(&transaction, &catalog, &candidate, now, &self.active, None)?;
        gate.sample(expired)?;
        transaction.commit()
            .map_err(|_| {
                result_unconfirmed(storage("Macro StageFinal commit"), gate.intent.clone())
            })?;
        self.lease.head = candidate.head;
        self.finalizing = None;
        self.current = current;
        gate.after_commit(receipt, expired)?;
        self.snapshot()?
            .final_output
            .ok_or_else(|| ChainPostCloseError::SchemaRejected.into())
    }
}

// Only synchronous write transactions use this private staging copy. It never
// authorizes an effect or leaves this file; only its head is published after a
// confirmed COMMIT. Dropping it makes no claim about SQLite rollback success.
fn transaction_lease_candidate(lease: &RunLease) -> RunLease {
    RunLease {
        intent_id: lease.intent_id.clone(),
        run_id: lease.run_id.clone(),
        owner: lease.owner.clone(),
        generation: lease.generation,
        head: lease.head,
        until: lease.until,
        input: lease.input.clone(),
    }
}

fn unmatched(current: &Option<old::MacroRecovery>) -> BTreeSet<u64> {
    current
        .iter()
        .flat_map(|recovery| {
            recovery
                .attempts
                .iter()
                .filter(|attempt| attempt.result.is_none())
                .map(|attempt| attempt.begin)
                .chain(recovery.readiness_episodes.iter().flat_map(|episode| {
                    episode
                        .controls
                        .iter()
                        .filter(|control| control.begin.is_some() && control.result.is_none())
                        .filter_map(|control| control.begin)
                }))
        })
        .collect()
}

fn admitted(
    transaction: &Transaction<'_>,
    catalog: &schema::V12CatalogProof<'_, '_>,
    lease: &RunLease,
    now: UtcMicros,
    active: &BTreeMap<Step, u64>,
    finalizing: Option<u64>,
) -> Result<(RunRecovery, Option<old::MacroRecovery>)> {
    let (run, current) =
        inspect_run_and_macro_with_catalog(transaction, &lease.intent_id, catalog)?;
    validate_admitted(transaction, lease, now, &run, &current, active, finalizing)?;
    Ok((run, current))
}

fn validate_admitted(
    transaction: &Transaction<'_>,
    lease: &RunLease,
    now: UtcMicros,
    run: &RunRecovery,
    current: &Option<old::MacroRecovery>,
    active: &BTreeMap<Step, u64>,
    finalizing: Option<u64>,
) -> Result<()> {
    old::validate_admission(transaction, lease, now, run)?;
    require(active.len() <= 5 && unmatched(current) == active.values().copied().collect())?;
    let unresolved_final = current
        .as_ref()
        .and_then(|recovery| recovery.full.as_ref())
        .and_then(|full| {
            full.begin
                .as_ref()
                .filter(|_| full.final_.is_none())
                .map(|(fact, _)| fact.version)
        });
    require(unresolved_final == finalizing)?;
    Ok(())
}

enum Body<'a> {
    Plan {
        bytes: &'a [u8],
        plan: &'a plan3::PlanV3,
    },
    Request {
        value: &'a plan3::RequestPlan,
        plan_version: u64,
        endpoint: &'a str,
    },
    Episode {
        value: &'a codec::ReadinessEpisodePlan,
        plan_version: u64,
        request_version: u64,
        endpoint: &'a str,
        authority: &'a str,
    },
    DataBegin(&'a native::DataBegin),
    DataResult {
        value: &'a native::DataResult,
        begin: u64,
        request_sha: &'a str,
        retry_due: Option<i64>,
    },
    ControlBegin {
        value: &'a old::ControlBegin,
        episode_version: u64,
    },
    ControlResult {
        value: &'a codec::ControlRawResult,
        control_ordinal: u32,
        kind: &'a str,
        begin: u64,
        request_sha: &'a str,
        ready: bool,
    },
    Terminal {
        value: &'a native::QueryTerminal,
        native: &'a [u8],
        audit: Option<(
            &'a str,
            &'a crate::data_gateway::review::OwnedGatewayAuditRecord,
            &'a DataAcquisitionAuditReceipt,
        )>,
    },
    Dimension(&'a native::DimensionTerminal),
    Finalize(&'a native::FinalizeBegin),
    Final {
        value: &'a native::StageFinal,
        begin: &'a native::FinalizeBegin,
    },
}

struct Writer<'transaction, 'connection, 'state> {
    transaction: &'transaction Transaction<'connection>,
    lease: &'state mut RunLease,
    run: &'state RunRecovery,
    now: UtcMicros,
    last_sha: String,
    audits: Vec<DataAcquisitionAuditReceipt>,
}

impl<'transaction, 'connection, 'state> Writer<'transaction, 'connection, 'state> {
    fn new(
        transaction: &'transaction Transaction<'connection>,
        lease: &'state mut RunLease,
        run: &'state RunRecovery,
        now: UtcMicros,
    ) -> Self {
        Self {
            transaction,
            lease,
            run,
            now,
            last_sha: String::new(),
            audits: Vec::new(),
        }
    }
    fn receipt(&self, previous: u64) -> Result<MacroConfirmedCommit> {
        require(self.lease.head > previous && !self.last_sha.is_empty())?;
        Ok(MacroConfirmedCommit {
            intent_id: self.lease.intent_id.as_str().to_owned(),
            first_version: previous + 1,
            last_version: self.lease.head,
            last_fact_sha256: self.last_sha.clone(),
            audit_receipts: self.audits.clone(),
        })
    }
    fn terminal(
        &mut self,
        definition: &crate::search_service::macro_news::runner::Definition,
        value: &native::QueryTerminal,
        outcome: &NativeOutcome,
    ) -> Result<u64> {
        let bytes = native::native_bytes(outcome)?;
        if matches!(value.query, QueryKey::Gateway(_)) {
            let identity = definition
                .identity(value.query)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?;
            let (capability, audit) = native::gateway_audit(&identity, outcome, self.now.get())?;
            let receipt =
                append_acquisition_in_transaction(self.transaction, &audit.borrowed(capability))
                    .map_err(|_| storage("full Macro terminal audit"))?;
            let version = self.append(Body::Terminal {
                value,
                native: &bytes,
                audit: Some((capability, &audit, &receipt)),
            })?;
            self.audits.push(receipt);
            Ok(version)
        } else {
            self.append(Body::Terminal {
                value,
                native: &bytes,
                audit: None,
            })
        }
    }

    // The enum is the complete write vocabulary. No caller can supply a table,
    // SQL fragment, parameter list, callback or connection through this seam.
    fn append(&mut self, body: Body<'_>) -> Result<u64> {
        let (table,columns,bytes,extra):(&str,&str,Vec<u8>,Vec<Value>) = match body {
            Body::Plan { bytes,plan } => (old::TABLES[0],"parent_version,parent_sha256,started_at,deadline_at,request_sha256",bytes.to_vec(),
                vec![number(plan.core.parent_version)?,text(&plan.core.parent_digest),Value::Integer(plan.core.started),Value::Integer(plan.core.deadline),text(raw_digest(&plan.core.request.bytes).as_str())]),
            Body::Request { value,plan_version,endpoint } => {
                let (phase,item,candidate) = native::query_columns(value.query)?;
                (old::TABLES[1],"phase,item_ordinal,candidate_ordinal,plan_version,request_sha256,profile,acquisition_authority,endpoint",codec::encode(value)?,
                    vec![text(phase),Value::Integer(item.into()),Value::Integer(candidate.into()),number(plan_version)?,text(raw_digest(&value.request.bytes).as_str()),
                        text(&value.request.profile),optional_text(value.request.authority.as_deref()),text(endpoint)])
            }
            Body::Episode { value,plan_version,request_version,endpoint,authority } => {
                let controls = value.controls();
                require(controls.iter().all(|control| control.endpoint() == endpoint && control.authority() == authority))?;
                (old::TABLES[2],"episode_ordinal,plan_version,request_plan_version,phase,item_ordinal,candidate_ordinal,required_operation,endpoint,acquisition_authority,health_request_sha256,capabilities_request_sha256",codec::encode(value)?,
                    vec![Value::Integer(1),number(plan_version)?,number(request_version)?,text("Gateway"),Value::Integer(1),Value::Integer(1),
                        Value::Integer(crate::grpc_client::pb::magic::market::v1::Operation::GlobalNews as i64),text(endpoint),text(authority),
                        text(raw_digest(controls[0].request_bytes()).as_str()),text(raw_digest(controls[1].request_bytes()).as_str())])
            }
            Body::DataBegin(value) => {
                let (phase,item,candidate) = native::query_columns(value.query)?;
                (old::TABLES[5],"phase,item_ordinal,candidate_ordinal,attempt_ordinal,request_plan_version,request_sha256,readiness_result_version,previous_result_version",codec::encode(value)?,
                    vec![text(phase),Value::Integer(item.into()),Value::Integer(candidate.into()),Value::Integer(value.attempt.into()),number(value.request_plan_version)?,
                        text(&value.request_sha256),optional_number(value.readiness_result_version)?,optional_number(value.previous_result_version)?])
            }
            Body::DataResult { value,begin,request_sha,retry_due } => {
                let (phase,item,candidate) = native::query_columns(value.query)?;
                (old::TABLES[6],"phase,item_ordinal,candidate_ordinal,attempt_ordinal,begin_version,request_sha256,continuation,retry_not_before",codec::encode(value)?,
                    vec![text(phase),Value::Integer(item.into()),Value::Integer(candidate.into()),Value::Integer(value.attempt.into()),number(begin)?,text(request_sha),
                        text(if retry_due.is_some() { "Retry" } else { "Terminal" }),retry_due.map(Value::Integer).unwrap_or(Value::Null)])
            }
            Body::ControlBegin { value,episode_version } => (old::TABLES[3],"episode_ordinal,control_ordinal,kind,episode_plan_version,request_sha256,health_result_version",codec::encode(value)?,
                vec![Value::Integer(value.episode_ordinal.into()),Value::Integer(value.control_ordinal.into()),text(&value.kind),number(episode_version)?,text(&value.request_sha256),optional_number(value.health_result_version)?]),
            Body::ControlResult { value,control_ordinal,kind,begin,request_sha,ready } => (old::TABLES[4],"episode_ordinal,control_ordinal,kind,begin_version,request_sha256,outcome",codec::encode(value)?,
                vec![Value::Integer(1),Value::Integer(control_ordinal.into()),text(kind),number(begin)?,text(request_sha),text(if ready { "Ready" } else { "Rejected" })]),
            Body::Terminal { value,native,audit } => {
                let (phase,item,candidate) = native::query_columns(value.query)?;
                let (cause,data,control,called) = match value.cause {
                    native::TerminalCause::DataResult { version } => ("DataResult",Some(version),None,"Returned"),
                    native::TerminalCause::SharedControlRejected { version } => ("SharedControlRejected",None,Some(version),"NotCalled"),
                    native::TerminalCause::HistoricalControlRejected { control_result_version, .. } => ("HistoricalControlRejected",None,Some(control_result_version),"NotCalled"),
                    native::TerminalCause::RequestRejected => ("RequestRejected",None,None,"NotCalled"),
                    native::TerminalCause::LocalRouteUnavailable => ("LocalRouteUnavailable",None,None,"NotCalled"),
                };
                let mut values = vec![text(phase),Value::Integer(item.into()),Value::Integer(candidate.into()),number(value.plan_version)?,text(&value.plan_sha256),
                    optional_number(value.request_plan_version)?,optional_text(value.request_sha256.as_deref()),text(cause),optional_number(data)?,optional_number(control)?,
                    text(called),Value::Blob(native.to_vec()),number(native.len() as u64)?,text(&value.native_sha256)];
                values.extend(match audit {
                    Some((capability,audit,receipt)) => vec![Value::Integer(receipt.audit_id),text(&receipt.record_hash),text(capability),text(&audit.provider),text(&audit.request_hash),
                        optional_text(receipt.previous_outcome.as_deref()),text(&receipt.current_outcome)],
                    None => vec![Value::Null;7],
                });
                (recovery::TABLES[0],"phase,item_ordinal,candidate_ordinal,plan_version,plan_sha256,request_plan_version,request_sha256,cause_kind,data_result_version,control_result_version,call_state,native_bytes,native_length,native_sha256,audit_id,audit_record_hash,audit_capability,audit_provider,audit_request_hash,previous_outcome,current_outcome",codec::encode(value)?,values)
            }
            Body::Dimension(value) => (recovery::TABLES[1],"dimension,plan_version,plan_sha256,outcome,last_query_terminal_version,selected_candidate_ordinal,pace_due",codec::encode(value)?,
                vec![Value::Integer(value.dimension.into()),number(value.plan_version)?,text(&value.plan_sha256),text(if value.selected_candidate.is_some() { "SelectedResearchOnly" }
                    else if value.eligible_candidates.is_empty() { "NoEligibleProviders" } else { "Exhausted" }),optional_number(value.last_query_terminal_version)?,
                    value.selected_candidate.map(|v| Value::Integer(v.into())).unwrap_or(Value::Null),Value::Integer(value.pace_due)]),
            Body::Finalize(value) => {
                let mut values = final_values(value)?;
                let (opened, elapsed) = value.expiry_columns();
                values.extend([opened.map(Value::Integer).unwrap_or(Value::Null),
                    elapsed.map(Value::Integer).unwrap_or(Value::Null)]);
                (recovery::TABLES[2],"plan_version,plan_sha256,kind,started_at,deadline_at,facts_sha256,output_bytes,output_length,output_sha256,expiry_opened_at,expiry_elapsed_us",codec::encode(value)?,values)
            }
            Body::Final { value,begin } => {
                let mut values = vec![number(value.finalize_begin_version)?];
                values.extend(final_values(begin)?);
                (recovery::TABLES[3],"finalize_begin_version,plan_version,plan_sha256,kind,started_at,deadline_at,facts_sha256,output_bytes,output_length,output_sha256",codec::encode(value)?,values)
            }
        };
        let prior = old::advance(self.transaction, self.lease, self.now)?;
        let digest = raw_digest(&bytes).as_str().to_owned();
        let mut values = vec![
            text(self.lease.intent_id.as_str()),
            text(self.lease.run_id.as_str()),
            text(self.run.context.canonical_sha256().as_str()),
            text(raw_digest(&self.run.input.encode()?).as_str()),
            text(self.lease.owner.as_str()),
            number(self.lease.generation)?,
            number(prior)?,
            number(self.lease.head)?,
            Value::Integer(self.now.get()),
            Value::Blob(bytes.clone()),
            number(bytes.len() as u64)?,
            text(&digest),
        ];
        values.extend(extra);
        let placeholders = (1..=values.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("INSERT INTO {table}(intent_id,run_id,run_context_sha256,input_sha256,lease_owner,lease_generation,prior_head_version,run_version,recorded_at,bytes,byte_length,sha256,{columns}) VALUES({placeholders})");
        self.transaction
            .execute(&sql, params_from_iter(values))
            .map_err(|_| storage("full Macro fact insert"))?;
        self.last_sha = digest;
        Ok(self.lease.head)
    }
}

fn number(value: u64) -> Result<Value> {
    Ok(Value::Integer(
        i64::try_from(value).map_err(|_| ChainPostCloseError::SchemaRejected)?,
    ))
}
fn optional_number(value: Option<u64>) -> Result<Value> {
    value
        .map(number)
        .transpose()
        .map(|value| value.unwrap_or(Value::Null))
}
fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}
fn optional_text(value: Option<&str>) -> Value {
    value.map(text).unwrap_or(Value::Null)
}
fn final_values(value: &native::FinalizeBegin) -> Result<Vec<Value>> {
    Ok(vec![
        number(value.plan_version)?,
        text(&value.plan_sha256),
        text(match value.kind {
            native::FinalKind::Complete => "Complete",
            native::FinalKind::BudgetExpired => "BudgetExpired",
        }),
        Value::Integer(value.started_at),
        Value::Integer(value.deadline_at),
        text(&value.facts_sha256),
        Value::Blob(value.output.clone()),
        number(value.output.len() as u64)?,
        text(&value.output_sha256),
    ])
}
