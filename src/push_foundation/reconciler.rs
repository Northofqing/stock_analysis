//! W11 business-side startup recovery. This module has no sink or dispatch capability.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;

use crate::monitor::push_job::{CompletionPolicy, ReasonCode, TerminalDisposition, UtcMicros};

use super::business_finalizer::{
    commit_accepted_finalization, prepare_accepted_finalization, AcceptedFinalizationOutcome,
    AcceptedPreparationOutcome, AcceptedPreparationRequest, BusinessFinalizerError, FinalizerFence,
};

use super::intent_store::{
    AttestedReadyIntent, BusinessIntentStore, IntentSnapshot, IntentState, IntentStoreError,
    LeaseOwnerId, RecoveryCursor, TransitionActor, TransitionOutcome,
};
use super::terminal_authority::{verify_terminal, TerminalAuthorityPort, TerminalTemplateBinding};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum RecoveryBindingError {
    #[error("recovery bindings are unavailable for the attested intent")]
    Unavailable,
}

pub(crate) struct RecoveryBindings<'a> {
    pub(crate) template: &'a TerminalTemplateBinding,
    pub(crate) policy: &'a CompletionPolicy,
    pub(crate) authority: &'a dyn TerminalAuthorityPort,
}

impl<'a> RecoveryBindings<'a> {
    pub(crate) fn new(
        template: &'a TerminalTemplateBinding,
        policy: &'a CompletionPolicy,
        authority: &'a dyn TerminalAuthorityPort,
    ) -> Self {
        Self {
            template,
            policy,
            authority,
        }
    }
}

pub(crate) trait RecoveryBindingsPort {
    fn resolve<'a>(
        &'a self,
        intent: &AttestedReadyIntent,
    ) -> Result<RecoveryBindings<'a>, RecoveryBindingError>;
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RecoveryConfig {
    owner: LeaseOwnerId,
    actor: TransitionActor,
    now: UtcMicros,
    lease_until: UtcMicros,
    page_size: usize,
    max_iterations: usize,
}

impl RecoveryConfig {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new(
        owner: LeaseOwnerId,
        actor: TransitionActor,
        now: UtcMicros,
        lease_until: UtcMicros,
        page_size: usize,
        max_iterations: usize,
    ) -> Result<Self, RecoveryError> {
        if lease_until <= now {
            return Err(RecoveryError::InvalidConfig {
                check: "lease_until",
            });
        }
        if !(1..=1_000).contains(&page_size) {
            return Err(RecoveryError::InvalidConfig { check: "page_size" });
        }
        if !(1..=100).contains(&max_iterations) {
            return Err(RecoveryError::InvalidConfig {
                check: "max_iterations",
            });
        }
        Ok(Self {
            owner,
            actor,
            now,
            lease_until,
            page_size,
            max_iterations,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryBoundary {
    DispatchPending,
    LiveForeignLease,
    AuthorityBlocked,
    RejectedAuthorizationRequired,
    OperatorAuditRequired,
    ManualResolutionRequired,
    Finalized,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryEntry {
    business_date: String,
    intent_id: String,
    state: IntentState,
    version: u64,
    lease_generation: u64,
    boundary: RecoveryBoundary,
}

impl RecoveryEntry {
    pub(crate) fn business_date(&self) -> &str {
        &self.business_date
    }

    pub(crate) fn intent_id(&self) -> &str {
        &self.intent_id
    }

    pub(crate) fn state(&self) -> IntentState {
        self.state
    }

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn lease_generation(&self) -> u64 {
        self.lease_generation
    }

    pub(crate) fn boundary(&self) -> RecoveryBoundary {
        self.boundary
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct StartupRecoveryReport {
    iterations: usize,
    transition_count: usize,
    entries: Vec<RecoveryEntry>,
}

impl StartupRecoveryReport {
    pub(crate) fn iterations(&self) -> usize {
        self.iterations
    }

    pub(crate) fn transition_count(&self) -> usize {
        self.transition_count
    }

    pub(crate) fn entries(&self) -> &[RecoveryEntry] {
        &self.entries
    }

    pub(crate) fn entry(&self, intent_id: &str) -> Option<&RecoveryEntry> {
        self.entries
            .iter()
            .find(|entry| entry.intent_id == intent_id)
    }
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum RecoveryError {
    #[error(transparent)]
    Store(#[from] IntentStoreError),
    #[error(transparent)]
    Binding(#[from] RecoveryBindingError),
    #[error(transparent)]
    Finalizer(#[from] BusinessFinalizerError),
    #[error("invalid recovery configuration: {check}")]
    InvalidConfig { check: &'static str },
    #[error("startup recovery exceeded its fixed-point iteration limit")]
    IterationLimitExceeded,
}

pub(crate) fn reconcile_startup(
    store: &mut BusinessIntentStore,
    config: &RecoveryConfig,
    bindings: &dyn RecoveryBindingsPort,
) -> Result<StartupRecoveryReport, RecoveryError> {
    let mut entries = BTreeMap::new();
    let mut transition_count = 0;

    for iteration in 1..=config.max_iterations {
        let mut cursor = None;
        let mut made_progress = false;
        loop {
            let page = store.scan_recovery_page(cursor.as_ref(), config.page_size)?;
            if page.is_empty() {
                break;
            }
            cursor = page.last().map(RecoveryCursor::after);
            for candidate in page {
                let before_version = candidate.version();
                let (current, boundary, applied) =
                    reconcile_one(store, candidate, config, bindings)?;
                transition_count += applied;
                made_progress |= current.version() > before_version;
                entries.insert(
                    (
                        current.business_date().to_owned(),
                        current.intent_id().to_owned(),
                    ),
                    RecoveryEntry {
                        business_date: current.business_date().to_owned(),
                        intent_id: current.intent_id().to_owned(),
                        state: current.state(),
                        version: current.version(),
                        lease_generation: current.lease_generation(),
                        boundary,
                    },
                );
            }
        }
        if !made_progress {
            return Ok(StartupRecoveryReport {
                iterations: iteration,
                transition_count,
                entries: entries.into_values().collect(),
            });
        }
    }
    Err(RecoveryError::IterationLimitExceeded)
}

fn reconcile_one(
    store: &mut BusinessIntentStore,
    candidate: IntentSnapshot,
    config: &RecoveryConfig,
    bindings: &dyn RecoveryBindingsPort,
) -> Result<(IntentSnapshot, RecoveryBoundary, usize), RecoveryError> {
    if candidate.state() == IntentState::ResolutionRequired {
        return Ok((candidate, RecoveryBoundary::ManualResolutionRequired, 0));
    }
    let (current, applied) = match ensure_recovery_lease(store, candidate, config)? {
        LeaseRecovery::Ready { current, applied } => (current, applied),
        LeaseRecovery::LiveForeign { current } => {
            return Ok((current, RecoveryBoundary::LiveForeignLease, 0));
        }
    };
    match current.state() {
        IntentState::PendingDispatch => Ok((current, RecoveryBoundary::DispatchPending, applied)),
        IntentState::AwaitingAuthority | IntentState::AwaitingFinalizer => {
            let intent = current.attested_ready_binding()?;
            let resolved = bindings.resolve(&intent)?;
            reconcile_authority(store, current, config, resolved, applied)
        }
        IntentState::ResolutionRequired => {
            Ok((current, RecoveryBoundary::ManualResolutionRequired, applied))
        }
        _ => Err(IntentStoreError::IntegrityFailed {
            check: "recovery_candidate_terminal_state",
        }
        .into()),
    }
}

fn reconcile_authority(
    store: &mut BusinessIntentStore,
    current: IntentSnapshot,
    config: &RecoveryConfig,
    bindings: RecoveryBindings<'_>,
    applied_before_authority: usize,
) -> Result<(IntentSnapshot, RecoveryBoundary, usize), RecoveryError> {
    let intent = current.attested_ready_binding()?;
    let lease_until = current
        .lease_until()
        .ok_or(IntentStoreError::IntegrityFailed {
            check: "recovery_authority_lease_until",
        })?;
    if current.lease_owner() != Some(config.owner.as_str())
        || current.lease_generation() == 0
        || lease_until <= config.now
    {
        return Err(IntentStoreError::IntegrityFailed {
            check: "recovery_authority_fence",
        }
        .into());
    }

    // W10 records an invalid terminal reference while already AwaitingFinalizer. Re-querying a
    // still-invalid reference must remain read-only or every fixed-point pass would append again.
    if current.reason() == ReasonCode::FinalizerTerminalRefInvalid
        && verify_terminal(
            &current,
            bindings.template,
            bindings.policy,
            bindings.authority,
            config.now,
        )
        .is_err()
    {
        return Ok((
            current,
            RecoveryBoundary::AuthorityBlocked,
            applied_before_authority,
        ));
    }

    let request = AcceptedPreparationRequest::new(
        intent.intent_id.clone(),
        current.version(),
        config.actor.clone(),
        FinalizerFence::new(
            config.owner.clone(),
            current.lease_generation(),
            lease_until,
        ),
        config.now,
        config.now,
    )?;
    match prepare_accepted_finalization(
        store,
        request,
        bindings.template,
        bindings.policy,
        bindings.authority,
    ) {
        Ok(AcceptedPreparationOutcome::Pending(pending)) => {
            let outcome = commit_accepted_finalization(
                store,
                pending,
                bindings.template,
                bindings.policy,
                bindings.authority,
                config.now,
                config.now,
            )?;
            let persisted = store
                .inspect(&intent.intent_id)?
                .ok_or(IntentStoreError::IntentMissing)?;
            let boundary = match outcome {
                AcceptedFinalizationOutcome::Applied { .. }
                | AcceptedFinalizationOutcome::AlreadyCommitted { .. } => {
                    RecoveryBoundary::Finalized
                }
                AcceptedFinalizationOutcome::ResolutionRequired { .. } => {
                    RecoveryBoundary::ManualResolutionRequired
                }
            };
            let applied = applied_before_authority + version_delta(&current, &persisted)?;
            Ok((persisted, boundary, applied))
        }
        Ok(AcceptedPreparationOutcome::AlreadyFinalized(_)) => {
            let persisted = store
                .inspect(&intent.intent_id)?
                .ok_or(IntentStoreError::IntentMissing)?;
            let applied = applied_before_authority + version_delta(&current, &persisted)?;
            Ok((persisted, RecoveryBoundary::Finalized, applied))
        }
        Err(BusinessFinalizerError::DispositionNotCompletable { actual }) => {
            reconcile_nonaccepted_disposition(
                store,
                current,
                actual,
                config,
                applied_before_authority,
            )
        }
        Err(BusinessFinalizerError::Terminal(_)) => {
            record_authority_blocker(store, current, config, applied_before_authority)
        }
        Err(BusinessFinalizerError::TerminalInvalid { .. }) => {
            let persisted = store
                .inspect(&intent.intent_id)?
                .ok_or(IntentStoreError::IntentMissing)?;
            let applied = applied_before_authority + version_delta(&current, &persisted)?;
            Ok((persisted, RecoveryBoundary::AuthorityBlocked, applied))
        }
        Err(error) => Err(error.into()),
    }
}

fn reconcile_nonaccepted_disposition(
    store: &mut BusinessIntentStore,
    current: IntentSnapshot,
    disposition: TerminalDisposition,
    config: &RecoveryConfig,
    applied_before_authority: usize,
) -> Result<(IntentSnapshot, RecoveryBoundary, usize), RecoveryError> {
    match (current.state(), disposition) {
        (IntentState::AwaitingAuthority, TerminalDisposition::Rejected) => {
            if current.reason() == ReasonCode::TransportRejected {
                return Ok((
                    current,
                    RecoveryBoundary::RejectedAuthorizationRequired,
                    applied_before_authority,
                ));
            }
            apply_recovery_observation(
                store,
                current,
                IntentState::AwaitingAuthority,
                ReasonCode::TransportRejected,
                RecoveryBoundary::RejectedAuthorizationRequired,
                config,
                applied_before_authority,
            )
        }
        (
            IntentState::AwaitingAuthority | IntentState::AwaitingFinalizer,
            TerminalDisposition::Uncertain,
        ) => apply_recovery_observation(
            store,
            current,
            IntentState::ResolutionRequired,
            ReasonCode::TransportUncertain,
            RecoveryBoundary::ManualResolutionRequired,
            config,
            applied_before_authority,
        ),
        (IntentState::AwaitingAuthority, TerminalDisposition::ManualConfirmedNotDelivered) => Ok((
            current,
            RecoveryBoundary::OperatorAuditRequired,
            applied_before_authority,
        )),
        (IntentState::AwaitingFinalizer, TerminalDisposition::Rejected)
        | (IntentState::AwaitingFinalizer, TerminalDisposition::ManualConfirmedNotDelivered) => {
            apply_recovery_observation(
                store,
                current,
                IntentState::ResolutionRequired,
                ReasonCode::OperatorResolutionConflict,
                RecoveryBoundary::ManualResolutionRequired,
                config,
                applied_before_authority,
            )
        }
        (_, TerminalDisposition::Accepted | TerminalDisposition::ManualConfirmedAccepted) => {
            Err(IntentStoreError::IntegrityFailed {
                check: "accepted_disposition_rejected_by_finalizer",
            }
            .into())
        }
        _ => Err(IntentStoreError::IntegrityFailed {
            check: "nonaccepted_recovery_source_state",
        }
        .into()),
    }
}

fn record_authority_blocker(
    store: &mut BusinessIntentStore,
    current: IntentSnapshot,
    config: &RecoveryConfig,
    applied_before_authority: usize,
) -> Result<(IntentSnapshot, RecoveryBoundary, usize), RecoveryError> {
    if current.reason() == ReasonCode::FinalizerTerminalRefInvalid {
        return Ok((
            current,
            RecoveryBoundary::AuthorityBlocked,
            applied_before_authority,
        ));
    }
    apply_recovery_observation(
        store,
        current.clone(),
        current.state(),
        ReasonCode::FinalizerTerminalRefInvalid,
        RecoveryBoundary::AuthorityBlocked,
        config,
        applied_before_authority,
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_recovery_observation(
    store: &mut BusinessIntentStore,
    current: IntentSnapshot,
    to_state: IntentState,
    reason: ReasonCode,
    boundary: RecoveryBoundary,
    config: &RecoveryConfig,
    applied_before_observation: usize,
) -> Result<(IntentSnapshot, RecoveryBoundary, usize), RecoveryError> {
    let intent = current.attested_ready_binding()?;
    let fence_until = current
        .lease_until()
        .ok_or(IntentStoreError::IntegrityFailed {
            check: "recovery_observation_lease_until",
        })?;
    let transition = store.apply_recovery_observation(
        &current,
        to_state,
        reason,
        &config.actor,
        config.now,
        &config.owner,
        current.lease_generation(),
        fence_until,
    )?;
    match transition {
        TransitionOutcome::Applied(_) | TransitionOutcome::AlreadyCommitted(_) => {
            let persisted = store
                .inspect(&intent.intent_id)?
                .ok_or(IntentStoreError::IntentMissing)?;
            let applied = applied_before_observation + version_delta(&current, &persisted)?;
            Ok((persisted, boundary, applied))
        }
        TransitionOutcome::Conflict { current } => {
            Ok((*current, boundary, applied_before_observation))
        }
    }
}

fn version_delta(before: &IntentSnapshot, after: &IntentSnapshot) -> Result<usize, RecoveryError> {
    let delta =
        after
            .version()
            .checked_sub(before.version())
            .ok_or(IntentStoreError::IntegrityFailed {
                check: "recovery_version_regression",
            })?;
    usize::try_from(delta).map_err(|_| {
        IntentStoreError::IntegrityFailed {
            check: "recovery_transition_count_overflow",
        }
        .into()
    })
}

enum LeaseRecovery {
    Ready {
        current: IntentSnapshot,
        applied: usize,
    },
    LiveForeign {
        current: IntentSnapshot,
    },
}

fn ensure_recovery_lease(
    store: &mut BusinessIntentStore,
    candidate: IntentSnapshot,
    config: &RecoveryConfig,
) -> Result<LeaseRecovery, RecoveryError> {
    if let (Some(owner), Some(until)) = (candidate.lease_owner(), candidate.lease_until()) {
        if until > config.now {
            return if owner == config.owner.as_str() {
                Ok(LeaseRecovery::Ready {
                    current: candidate,
                    applied: 0,
                })
            } else {
                Ok(LeaseRecovery::LiveForeign { current: candidate })
            };
        }
    }

    let intent_id = candidate.attested_ready_binding()?.intent_id;
    match store.claim_recovery_lease(
        &candidate,
        &config.owner,
        config.lease_until,
        &config.actor,
        config.now,
    )? {
        TransitionOutcome::Applied(_) => {
            let current = store
                .inspect(&intent_id)?
                .ok_or(IntentStoreError::IntentMissing)?;
            Ok(LeaseRecovery::Ready {
                current,
                applied: 1,
            })
        }
        TransitionOutcome::AlreadyCommitted(_) => {
            let current = store
                .inspect(&intent_id)?
                .ok_or(IntentStoreError::IntentMissing)?;
            Ok(LeaseRecovery::Ready {
                current,
                applied: 0,
            })
        }
        TransitionOutcome::Conflict { current } => {
            if current
                .lease_until()
                .is_some_and(|until| until > config.now)
                && current.lease_owner() != Some(config.owner.as_str())
            {
                Ok(LeaseRecovery::LiveForeign { current: *current })
            } else {
                Ok(LeaseRecovery::Ready {
                    current: *current,
                    applied: 0,
                })
            }
        }
    }
}
