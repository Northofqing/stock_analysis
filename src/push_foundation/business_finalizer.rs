//! W10 business-intent finalization. Concrete Unit cursor mutations remain outside this slice.

#![cfg_attr(not(test), allow(dead_code))]

use crate::monitor::push_job::{
    evaluate_completion, CompletionDirective, CompletionFact, CompletionPolicy, IntentId,
    TerminalDisposition, UtcMicros, VerifiedTerminalRef,
};

use super::intent_store::{
    BusinessIntentStore, IntentSnapshot, IntentState, IntentStoreError, LeaseOwnerId,
    TransitionActor, TransitionOutcome, TransitionReceipt,
};
use super::terminal_authority::{
    reverify_for_finalization, verify_terminal, TerminalAuthorityError, TerminalAuthorityPort,
    TerminalTemplateBinding,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FinalizerFence {
    owner: LeaseOwnerId,
    generation: u64,
    until: UtcMicros,
}

impl FinalizerFence {
    pub(crate) fn new(owner: LeaseOwnerId, generation: u64, until: UtcMicros) -> Self {
        Self {
            owner,
            generation,
            until,
        }
    }

    fn matches(&self, snapshot: &IntentSnapshot, occurred_at: UtcMicros) -> bool {
        snapshot.lease_owner() == Some(self.owner.as_str())
            && snapshot.lease_generation() == self.generation
            && snapshot.lease_until() == Some(self.until)
            && self.until > occurred_at
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct AcceptedPreparationRequest {
    intent_id: IntentId,
    expected_version: u64,
    actor: TransitionActor,
    fence: FinalizerFence,
    verified_at: UtcMicros,
    occurred_at: UtcMicros,
}

impl AcceptedPreparationRequest {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        intent_id: IntentId,
        expected_version: u64,
        actor: TransitionActor,
        fence: FinalizerFence,
        verified_at: UtcMicros,
        occurred_at: UtcMicros,
    ) -> Result<Self, BusinessFinalizerError> {
        if expected_version >= i64::MAX as u64 {
            return Err(BusinessFinalizerError::InvalidRequest {
                check: "version_overflow",
            });
        }
        if verified_at > occurred_at {
            return Err(BusinessFinalizerError::InvalidRequest {
                check: "verification_after_transition",
            });
        }
        if fence.generation == 0 || fence.until <= occurred_at {
            return Err(BusinessFinalizerError::InvalidRequest {
                check: "inactive_finalizer_fence",
            });
        }
        Ok(Self {
            intent_id,
            expected_version,
            actor,
            fence,
            verified_at,
            occurred_at,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingAcceptedFinalization {
    prior: VerifiedTerminalRef,
    intent_id: IntentId,
    qualified_version: u64,
    actor: TransitionActor,
    fence: FinalizerFence,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AcceptedPreparationOutcome {
    Pending(PendingAcceptedFinalization),
    AlreadyFinalized(TransitionReceipt),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AcceptedFinalizationOutcome {
    Applied {
        receipt: TransitionReceipt,
        directive: CompletionDirective,
    },
    AlreadyCommitted {
        receipt: TransitionReceipt,
        directive: CompletionDirective,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum BusinessFinalizerError {
    #[error(transparent)]
    Store(#[from] IntentStoreError),
    #[error(transparent)]
    Terminal(#[from] TerminalAuthorityError),
    #[error("invalid finalizer request: {check}")]
    InvalidRequest { check: &'static str },
    #[error("business intent is not in an accepted-finalizer source state")]
    InvalidSourceState,
    #[error("business finalizer lease fence does not match the current intent")]
    FenceMismatch,
    #[error("business intent version changed before finalization")]
    BusinessCasConflict,
    #[error("terminal disposition cannot enter the accepted finalizer")]
    DispositionNotCompletable { actual: TerminalDisposition },
    #[error("completion policy rejected the verified terminal")]
    PolicyRejected,
}

pub(crate) fn prepare_accepted_finalization(
    store: &mut BusinessIntentStore,
    request: AcceptedPreparationRequest,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
) -> Result<AcceptedPreparationOutcome, BusinessFinalizerError> {
    let current = store
        .inspect(&request.intent_id)?
        .ok_or(IntentStoreError::IntentMissing)?;
    let chain = store.inspect_transition_chain(&request.intent_id)?;
    if current.state() == IntentState::Completed {
        let receipt = chain
            .last()
            .filter(|event| event.to_state() == IntentState::Completed)
            .cloned()
            .ok_or(IntentStoreError::IntegrityFailed {
                check: "completed_head_event",
            })?;
        return Ok(AcceptedPreparationOutcome::AlreadyFinalized(receipt));
    }
    if current.version() != request.expected_version {
        return Err(BusinessFinalizerError::BusinessCasConflict);
    }
    if !matches!(
        current.state(),
        IntentState::AwaitingAuthority | IntentState::AwaitingFinalizer
    ) {
        return Err(BusinessFinalizerError::InvalidSourceState);
    }
    if !request.fence.matches(&current, request.occurred_at) {
        return Err(BusinessFinalizerError::FenceMismatch);
    }

    let prior = verify_terminal(&current, template, policy, authority, request.verified_at)?;
    require_accepted_disposition(prior.terminal_disposition())?;
    let _ = accepted_directive(policy, &prior)?;

    let qualified_version = if current.state() == IntentState::AwaitingFinalizer {
        current.version()
    } else {
        match store.apply_authority_qualification(
            &prior,
            current.version(),
            &request.actor,
            request.occurred_at,
            &request.fence.owner,
            request.fence.generation,
            request.fence.until,
        )? {
            TransitionOutcome::Applied(receipt) | TransitionOutcome::AlreadyCommitted(receipt) => {
                receipt.result_version()
            }
            TransitionOutcome::Conflict { .. } => {
                return Err(BusinessFinalizerError::BusinessCasConflict)
            }
        }
    };

    Ok(AcceptedPreparationOutcome::Pending(
        PendingAcceptedFinalization {
            prior,
            intent_id: request.intent_id,
            qualified_version,
            actor: request.actor,
            fence: request.fence,
        },
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_accepted_finalization(
    store: &mut BusinessIntentStore,
    pending: PendingAcceptedFinalization,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
    verified_at: UtcMicros,
    occurred_at: UtcMicros,
) -> Result<AcceptedFinalizationOutcome, BusinessFinalizerError> {
    if verified_at > occurred_at {
        return Err(BusinessFinalizerError::InvalidRequest {
            check: "verification_after_transition",
        });
    }
    let current = store
        .inspect(&pending.intent_id)?
        .ok_or(IntentStoreError::IntentMissing)?;
    store.inspect_transition_chain(&pending.intent_id)?;
    if current.state() != IntentState::AwaitingFinalizer {
        return Err(BusinessFinalizerError::InvalidSourceState);
    }
    if current.version() != pending.qualified_version {
        return Err(BusinessFinalizerError::BusinessCasConflict);
    }
    if !pending.fence.matches(&current, occurred_at) {
        return Err(BusinessFinalizerError::FenceMismatch);
    }

    let fresh = reverify_for_finalization(
        &pending.prior,
        &current,
        template,
        policy,
        authority,
        verified_at,
    )?;
    require_accepted_disposition(fresh.verified_terminal().terminal_disposition())?;
    let directive = accepted_directive(policy, fresh.verified_terminal())?;
    match store.apply_accepted_finalization(
        fresh,
        current.version(),
        &pending.actor,
        occurred_at,
        &pending.fence.owner,
        pending.fence.generation,
        pending.fence.until,
    )? {
        TransitionOutcome::Applied(receipt) => {
            Ok(AcceptedFinalizationOutcome::Applied { receipt, directive })
        }
        TransitionOutcome::AlreadyCommitted(receipt) => {
            Ok(AcceptedFinalizationOutcome::AlreadyCommitted { receipt, directive })
        }
        TransitionOutcome::Conflict { .. } => Err(BusinessFinalizerError::BusinessCasConflict),
    }
}

fn require_accepted_disposition(
    disposition: TerminalDisposition,
) -> Result<(), BusinessFinalizerError> {
    if matches!(
        disposition,
        TerminalDisposition::Accepted | TerminalDisposition::ManualConfirmedAccepted
    ) {
        return Ok(());
    }
    Err(BusinessFinalizerError::DispositionNotCompletable {
        actual: disposition,
    })
}

fn accepted_directive(
    policy: &CompletionPolicy,
    terminal: &VerifiedTerminalRef,
) -> Result<CompletionDirective, BusinessFinalizerError> {
    let result = terminal.clone().into_delivery_result();
    evaluate_completion(policy, CompletionFact::Delivery(&result))
        .map_err(|_| BusinessFinalizerError::PolicyRejected)
}
