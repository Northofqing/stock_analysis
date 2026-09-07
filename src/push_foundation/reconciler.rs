//! W11 business-side startup recovery. This module has no sink or dispatch capability.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;

use crate::monitor::push_job::{CompletionPolicy, UtcMicros};

use super::intent_store::{
    AttestedReadyIntent, BusinessIntentStore, IntentSnapshot, IntentState, IntentStoreError,
    LeaseOwnerId, RecoveryCursor, TransitionActor, TransitionOutcome,
};
use super::terminal_authority::{TerminalAuthorityPort, TerminalTemplateBinding};

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
            let _ = bindings.resolve(&intent)?;
            Ok((current, RecoveryBoundary::AuthorityBlocked, applied))
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
