//! W10 business-intent finalization. Concrete Unit cursor mutations remain outside this slice.

#![cfg_attr(not(test), allow(dead_code))]

use crate::monitor::push_job::{
    evaluate_completion, CompletionDirective, CompletionFact, CompletionPolicy, DecisionId,
    IntentId, Sha256Digest, TerminalDisposition, UtcMicros, VerifiedTerminalRef,
};

#[cfg(test)]
pub(crate) use super::intent_store::TransitionFault as FinalizerFault;
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

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct VerifiedResolutionClearance {
    intent_id: IntentId,
    resolution_version: u64,
    conflict_event_sha256: Sha256Digest,
}

impl VerifiedResolutionClearance {
    #[cfg(test)]
    pub(crate) fn for_test(
        intent_id: IntentId,
        resolution_version: u64,
        conflict_event_sha256: Sha256Digest,
    ) -> Self {
        Self {
            intent_id,
            resolution_version,
            conflict_event_sha256,
        }
    }

    fn matches(&self, current: &IntentSnapshot, chain: &[TransitionReceipt]) -> bool {
        current.state() == IntentState::ResolutionRequired
            && self.intent_id.as_str() == current.intent_id()
            && self.resolution_version == current.version()
            && chain.last().map(TransitionReceipt::canonical_sha256)
                == Some(&self.conflict_event_sha256)
    }
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
    resolution_clearance: Option<VerifiedResolutionClearance>,
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
            resolution_clearance: None,
        })
    }

    pub(crate) fn with_resolution_clearance(
        mut self,
        clearance: VerifiedResolutionClearance,
    ) -> Self {
        self.resolution_clearance = Some(clearance);
        self
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingAcceptedFinalization {
    activation_operation: Option<String>,
    prior: Box<VerifiedTerminalRef>,
    intent_id: IntentId,
    qualified_version: u64,
    actor: TransitionActor,
    fence: FinalizerFence,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct VerifiedOperatorAuditRef {
    intent_id: IntentId,
    decision_id: DecisionId,
    expected_version: u64,
    audit_ref: String,
    audit_sha256: Sha256Digest,
}

impl VerifiedOperatorAuditRef {
    #[cfg(test)]
    pub(crate) fn for_test(
        intent_id: IntentId,
        decision_id: DecisionId,
        expected_version: u64,
        audit_ref: String,
        audit_sha256: Sha256Digest,
    ) -> Result<Self, BusinessFinalizerError> {
        if audit_ref.is_empty()
            || audit_ref.len() > 512
            || audit_ref.contains('\0')
            || audit_ref.trim() != audit_ref
        {
            return Err(BusinessFinalizerError::InvalidRequest {
                check: "operator_audit_ref",
            });
        }
        Ok(Self {
            intent_id,
            decision_id,
            expected_version,
            audit_ref,
            audit_sha256,
        })
    }

    fn matches_snapshot(&self, current: &IntentSnapshot) -> Result<bool, IntentStoreError> {
        let attested = current.attested_ready_binding()?;
        Ok(self.intent_id.as_str() == current.intent_id()
            && self.intent_id == attested.intent_id
            && self.decision_id == attested.decision_id
            && self.expected_version == current.version())
    }

    fn matches_terminal(&self, terminal: &VerifiedTerminalRef) -> bool {
        self.intent_id == *terminal.intent_id() && self.decision_id == *terminal.decision_id()
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct NotDeliveredPreparationRequest {
    intent_id: IntentId,
    expected_version: u64,
    actor: TransitionActor,
    fence: FinalizerFence,
    verified_at: UtcMicros,
    audit: Option<VerifiedOperatorAuditRef>,
}

impl NotDeliveredPreparationRequest {
    pub(crate) fn new(
        intent_id: IntentId,
        expected_version: u64,
        actor: TransitionActor,
        fence: FinalizerFence,
        verified_at: UtcMicros,
        audit: VerifiedOperatorAuditRef,
    ) -> Result<Self, BusinessFinalizerError> {
        Self::build(
            intent_id,
            expected_version,
            actor,
            fence,
            verified_at,
            Some(audit),
        )
    }

    #[cfg(test)]
    pub(crate) fn without_audit_for_test(
        intent_id: IntentId,
        expected_version: u64,
        actor: TransitionActor,
        fence: FinalizerFence,
        verified_at: UtcMicros,
    ) -> Result<Self, BusinessFinalizerError> {
        Self::build(intent_id, expected_version, actor, fence, verified_at, None)
    }

    fn build(
        intent_id: IntentId,
        expected_version: u64,
        actor: TransitionActor,
        fence: FinalizerFence,
        verified_at: UtcMicros,
        audit: Option<VerifiedOperatorAuditRef>,
    ) -> Result<Self, BusinessFinalizerError> {
        if expected_version >= i64::MAX as u64 {
            return Err(BusinessFinalizerError::InvalidRequest {
                check: "version_overflow",
            });
        }
        if fence.generation == 0 || fence.until <= verified_at {
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
            audit,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingNotDeliveredFinalization {
    activation_operation: Option<String>,
    prior: VerifiedTerminalRef,
    intent_id: IntentId,
    source_state: IntentState,
    expected_version: u64,
    actor: TransitionActor,
    fence: FinalizerFence,
    audit: VerifiedOperatorAuditRef,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AcceptedPreparationOutcome {
    Pending(PendingAcceptedFinalization),
    AlreadyFinalized(TransitionReceipt),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum NotDeliveredPreparationOutcome {
    Pending(Box<PendingNotDeliveredFinalization>),
    AlreadyFinalized(Box<TransitionReceipt>),
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
    ResolutionRequired {
        receipt: TransitionReceipt,
    },
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum NotDeliveredFinalizationOutcome {
    Applied {
        receipt: TransitionReceipt,
        directive: CompletionDirective,
    },
    AlreadyCommitted {
        receipt: TransitionReceipt,
        directive: CompletionDirective,
    },
    ResolutionRequired {
        receipt: TransitionReceipt,
    },
}

enum FinalizerStoreMode {
    Normal,
    #[cfg(test)]
    Fault(FinalizerFault),
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum BusinessFinalizerError {
    #[error(transparent)]
    Store(#[from] IntentStoreError),
    #[error(transparent)]
    Terminal(#[from] TerminalAuthorityError),
    #[error("terminal authority became invalid during finalization")]
    TerminalInvalid {
        source: TerminalAuthorityError,
        receipt: Box<TransitionReceipt>,
    },
    #[error("invalid finalizer request: {check}")]
    InvalidRequest { check: &'static str },
    #[error("business intent is not in an accepted-finalizer source state")]
    InvalidSourceState,
    #[error("business finalizer lease fence does not match the current intent")]
    FenceMismatch,
    #[error("business finalizer conflict could not be isolated in one CAS attempt")]
    ConflictUnresolved { current: Box<IntentSnapshot> },
    #[error("terminal disposition cannot enter the accepted finalizer")]
    DispositionNotCompletable { actual: TerminalDisposition },
    #[error("completion policy rejected the verified terminal")]
    PolicyRejected,
    #[error("resolution recovery requires an authenticated clearance capability")]
    ResolutionClearanceRequired,
    #[error("resolution clearance does not bind the current conflict")]
    ResolutionClearanceMismatch,
    #[error("manual not-delivered finalization requires an authenticated operator audit")]
    OperatorAuditRequired,
    #[error("operator audit does not bind the current intent decision and version")]
    OperatorAuditMismatch,
    #[error("intent history is not eligible for manual not-delivered finalization")]
    NotDeliveredHistoryIneligible,
}

/// Borrowed from one running worker; pending values contain no reusable permit.
pub(super) enum FinalizerExecution<'a> {
    #[cfg(unix)]
    Current(&'a super::activation_business_effect::BusinessExecution<'a>),
    #[cfg(test)]
    Legacy,
    #[cfg(not(unix))]
    Denied(std::marker::PhantomData<&'a ()>),
}

impl FinalizerExecution<'_> {
    fn check(
        &self,
        store: &BusinessIntentStore,
        intent: &IntentId,
    ) -> Result<Option<String>, BusinessFinalizerError> {
        match self {
            #[cfg(unix)]
            Self::Current(execution) => {
                execution
                    .check(store, intent.as_str())
                    .map_err(|_| BusinessFinalizerError::FenceMismatch)?;
                Ok(Some(execution.operation_binding()))
            }
            #[cfg(test)]
            Self::Legacy => Ok(None),
            #[cfg(not(unix))]
            Self::Denied(_) => Err(BusinessFinalizerError::FenceMismatch),
        }
    }

    pub(super) fn prepare_accepted(
        &self,
        store: &mut BusinessIntentStore,
        request: AcceptedPreparationRequest,
        template: &TerminalTemplateBinding,
        policy: &CompletionPolicy,
        authority: &dyn TerminalAuthorityPort,
    ) -> Result<AcceptedPreparationOutcome, BusinessFinalizerError> {
        let operation = self.check(store, &request.intent_id)?;
        let mut outcome =
            prepare_accepted_finalization_inner(store, request, template, policy, authority)?;
        if let AcceptedPreparationOutcome::Pending(pending) = &mut outcome {
            pending.activation_operation = operation;
            #[cfg(all(test, unix))]
            if let Self::Current(execution) = self {
                if let Some(source) = execution
                    .pending_source_binding()
                    .map_err(|_| BusinessFinalizerError::FenceMismatch)?
                {
                    pending.activation_operation = Some(source);
                }
            }
        }
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_accepted(
        &self,
        store: &mut BusinessIntentStore,
        pending: PendingAcceptedFinalization,
        template: &TerminalTemplateBinding,
        policy: &CompletionPolicy,
        authority: &dyn TerminalAuthorityPort,
        verified_at: UtcMicros,
        occurred_at: UtcMicros,
    ) -> Result<AcceptedFinalizationOutcome, BusinessFinalizerError> {
        let operation = self.check(store, &pending.intent_id)?;
        if operation != pending.activation_operation {
            return Err(BusinessFinalizerError::FenceMismatch);
        }
        let mode = FinalizerStoreMode::Normal;
        #[cfg(all(test, unix))]
        let mode = if matches!(self, Self::Current(execution) if execution.business_commit_ack_lost())
        {
            FinalizerStoreMode::Fault(FinalizerFault::AfterCommitAckLost)
        } else {
            mode
        };
        commit_accepted_finalization_inner(
            store,
            pending,
            template,
            policy,
            authority,
            verified_at,
            occurred_at,
            mode,
        )
    }

    pub(super) fn prepare_not_delivered(
        &self,
        store: &mut BusinessIntentStore,
        request: NotDeliveredPreparationRequest,
        template: &TerminalTemplateBinding,
        policy: &CompletionPolicy,
        authority: &dyn TerminalAuthorityPort,
    ) -> Result<NotDeliveredPreparationOutcome, BusinessFinalizerError> {
        let operation = self.check(store, &request.intent_id)?;
        let mut outcome =
            prepare_not_delivered_finalization_inner(store, request, template, policy, authority)?;
        if let NotDeliveredPreparationOutcome::Pending(pending) = &mut outcome {
            pending.activation_operation = operation;
        }
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_not_delivered(
        &self,
        store: &mut BusinessIntentStore,
        pending: Box<PendingNotDeliveredFinalization>,
        template: &TerminalTemplateBinding,
        policy: &CompletionPolicy,
        authority: &dyn TerminalAuthorityPort,
        verified_at: UtcMicros,
        occurred_at: UtcMicros,
    ) -> Result<NotDeliveredFinalizationOutcome, BusinessFinalizerError> {
        let operation = self.check(store, &pending.intent_id)?;
        if operation != pending.activation_operation {
            return Err(BusinessFinalizerError::FenceMismatch);
        }
        commit_not_delivered_finalization_inner(
            store,
            pending,
            template,
            policy,
            authority,
            verified_at,
            occurred_at,
        )
    }
}

#[cfg(test)]
pub(crate) fn prepare_accepted_finalization(
    store: &mut BusinessIntentStore,
    request: AcceptedPreparationRequest,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
) -> Result<AcceptedPreparationOutcome, BusinessFinalizerError> {
    prepare_accepted_finalization_inner(store, request, template, policy, authority)
}

#[cfg(test)]
pub(crate) fn prepare_not_delivered_finalization(
    store: &mut BusinessIntentStore,
    request: NotDeliveredPreparationRequest,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
) -> Result<NotDeliveredPreparationOutcome, BusinessFinalizerError> {
    FinalizerExecution::Legacy.prepare_not_delivered(store, request, template, policy, authority)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_not_delivered_finalization(
    store: &mut BusinessIntentStore,
    pending: Box<PendingNotDeliveredFinalization>,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
    verified_at: UtcMicros,
    occurred_at: UtcMicros,
) -> Result<NotDeliveredFinalizationOutcome, BusinessFinalizerError> {
    FinalizerExecution::Legacy.commit_not_delivered(
        store,
        pending,
        template,
        policy,
        authority,
        verified_at,
        occurred_at,
    )
}

fn prepare_accepted_finalization_inner(
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
        return Err(BusinessFinalizerError::ConflictUnresolved {
            current: Box::new(current),
        });
    }
    match current.state() {
        IntentState::AwaitingAuthority | IntentState::AwaitingFinalizer => {
            if request.resolution_clearance.is_some() {
                return Err(BusinessFinalizerError::ResolutionClearanceMismatch);
            }
        }
        IntentState::ResolutionRequired => match request.resolution_clearance.as_ref() {
            None => return Err(BusinessFinalizerError::ResolutionClearanceRequired),
            Some(clearance) if !clearance.matches(&current, &chain) => {
                return Err(BusinessFinalizerError::ResolutionClearanceMismatch)
            }
            Some(_) => {}
        },
        _ => return Err(BusinessFinalizerError::InvalidSourceState),
    }
    if !request.fence.matches(&current, request.occurred_at) {
        return Err(BusinessFinalizerError::FenceMismatch);
    }

    let prior = match verify_terminal(&current, template, policy, authority, request.verified_at) {
        Ok(prior) => prior,
        Err(source) if current.state() == IntentState::AwaitingFinalizer => {
            return Err(record_terminal_invalid(
                store,
                &request.intent_id,
                current.version(),
                &request.actor,
                request.occurred_at,
                &request.fence,
                source,
            ));
        }
        Err(source) => return Err(source.into()),
    };
    require_accepted_disposition(prior.terminal_disposition())?;
    let _ = accepted_directive(policy, &prior)?;

    let qualified_version = if current.state() == IntentState::AwaitingFinalizer {
        current.version()
    } else {
        match store.apply_authority_qualification(
            &prior,
            current.state(),
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
            TransitionOutcome::Conflict { current } => {
                return Err(BusinessFinalizerError::ConflictUnresolved { current })
            }
        }
    };

    Ok(AcceptedPreparationOutcome::Pending(
        PendingAcceptedFinalization {
            activation_operation: None,
            prior: Box::new(prior),
            intent_id: request.intent_id,
            qualified_version,
            actor: request.actor,
            fence: request.fence,
        },
    ))
}

fn prepare_not_delivered_finalization_inner(
    store: &mut BusinessIntentStore,
    request: NotDeliveredPreparationRequest,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
) -> Result<NotDeliveredPreparationOutcome, BusinessFinalizerError> {
    let current = store
        .inspect(&request.intent_id)?
        .ok_or(IntentStoreError::IntentMissing)?;
    let chain = store.inspect_transition_chain(&request.intent_id)?;
    if current.state() == IntentState::NotDelivered {
        let receipt = chain
            .last()
            .filter(|event| event.to_state() == IntentState::NotDelivered)
            .cloned()
            .ok_or(IntentStoreError::IntegrityFailed {
                check: "not_delivered_head_event",
            })?;
        return Ok(NotDeliveredPreparationOutcome::AlreadyFinalized(Box::new(
            receipt,
        )));
    }
    if current.version() != request.expected_version {
        return Err(BusinessFinalizerError::ConflictUnresolved {
            current: Box::new(current),
        });
    }
    if !not_delivered_history_eligible(&current, &chain) {
        return Err(BusinessFinalizerError::NotDeliveredHistoryIneligible);
    }
    if !request.fence.matches(&current, request.verified_at) {
        return Err(BusinessFinalizerError::FenceMismatch);
    }
    let audit = request
        .audit
        .ok_or(BusinessFinalizerError::OperatorAuditRequired)?;
    if !audit.matches_snapshot(&current)? {
        return Err(BusinessFinalizerError::OperatorAuditMismatch);
    }

    let prior = verify_terminal(&current, template, policy, authority, request.verified_at)?;
    require_not_delivered_disposition(prior.terminal_disposition())?;
    if !audit.matches_terminal(&prior) {
        return Err(BusinessFinalizerError::OperatorAuditMismatch);
    }
    let _ = not_delivered_directive(policy, &prior)?;

    Ok(NotDeliveredPreparationOutcome::Pending(Box::new(
        PendingNotDeliveredFinalization {
            activation_operation: None,
            prior,
            intent_id: request.intent_id,
            source_state: current.state(),
            expected_version: current.version(),
            actor: request.actor,
            fence: request.fence,
            audit,
        },
    )))
}

#[allow(clippy::too_many_arguments)]
fn commit_not_delivered_finalization_inner(
    store: &mut BusinessIntentStore,
    pending: Box<PendingNotDeliveredFinalization>,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
    verified_at: UtcMicros,
    occurred_at: UtcMicros,
) -> Result<NotDeliveredFinalizationOutcome, BusinessFinalizerError> {
    if verified_at > occurred_at {
        return Err(BusinessFinalizerError::InvalidRequest {
            check: "verification_after_transition",
        });
    }
    let current = store
        .inspect(&pending.intent_id)?
        .ok_or(IntentStoreError::IntentMissing)?;
    let chain = store.inspect_transition_chain(&pending.intent_id)?;
    if current.state() == IntentState::NotDelivered {
        let receipt = chain
            .last()
            .filter(|event| event.to_state() == IntentState::NotDelivered)
            .cloned()
            .ok_or(IntentStoreError::IntegrityFailed {
                check: "not_delivered_head_event",
            })?;
        if receipt.matches_not_delivered_completion(
            &pending.prior,
            pending.expected_version,
            &pending.actor,
            occurred_at,
            &pending.audit.audit_ref,
            &pending.audit.audit_sha256,
        ) {
            let directive = not_delivered_directive(policy, &pending.prior)?;
            return Ok(NotDeliveredFinalizationOutcome::AlreadyCommitted { receipt, directive });
        }
        return Err(BusinessFinalizerError::ConflictUnresolved {
            current: Box::new(current),
        });
    }
    if !not_delivered_history_eligible(&current, &chain) {
        return Err(BusinessFinalizerError::NotDeliveredHistoryIneligible);
    }
    if current.state() != pending.source_state || current.version() != pending.expected_version {
        return isolate_not_delivered_conflict(store, &current, &pending, occurred_at);
    }
    if !pending.fence.matches(&current, occurred_at) {
        return Err(BusinessFinalizerError::FenceMismatch);
    }
    if !pending.audit.matches_snapshot(&current)? {
        return Err(BusinessFinalizerError::OperatorAuditMismatch);
    }

    let fresh = reverify_for_finalization(
        &pending.prior,
        &current,
        template,
        policy,
        authority,
        verified_at,
    )?;
    require_not_delivered_disposition(fresh.verified_terminal().terminal_disposition())?;
    if !pending.audit.matches_terminal(fresh.verified_terminal()) {
        return Err(BusinessFinalizerError::OperatorAuditMismatch);
    }
    let directive = not_delivered_directive(policy, fresh.verified_terminal())?;
    let transition = store.apply_not_delivered_finalization(
        fresh,
        current.state(),
        current.version(),
        &pending.actor,
        occurred_at,
        &pending.fence.owner,
        pending.fence.generation,
        pending.fence.until,
        &pending.audit.audit_ref,
        &pending.audit.audit_sha256,
    )?;
    match transition {
        TransitionOutcome::Applied(receipt) => {
            Ok(NotDeliveredFinalizationOutcome::Applied { receipt, directive })
        }
        TransitionOutcome::AlreadyCommitted(receipt) => {
            Ok(NotDeliveredFinalizationOutcome::AlreadyCommitted { receipt, directive })
        }
        TransitionOutcome::Conflict { current } => {
            isolate_not_delivered_conflict(store, &current, &pending, occurred_at)
        }
    }
}

fn isolate_not_delivered_conflict(
    store: &mut BusinessIntentStore,
    current: &IntentSnapshot,
    pending: &PendingNotDeliveredFinalization,
    occurred_at: UtcMicros,
) -> Result<NotDeliveredFinalizationOutcome, BusinessFinalizerError> {
    if !matches!(current.state(), IntentState::AwaitingAuthority) {
        return Err(BusinessFinalizerError::ConflictUnresolved {
            current: Box::new(current.clone()),
        });
    }
    match store.apply_finalizer_conflict(
        &pending.intent_id,
        current,
        &pending.actor,
        occurred_at,
    )? {
        TransitionOutcome::Applied(receipt) | TransitionOutcome::AlreadyCommitted(receipt) => {
            Ok(NotDeliveredFinalizationOutcome::ResolutionRequired { receipt })
        }
        TransitionOutcome::Conflict { current } => {
            Err(BusinessFinalizerError::ConflictUnresolved { current })
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn commit_accepted_finalization(
    store: &mut BusinessIntentStore,
    pending: PendingAcceptedFinalization,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
    verified_at: UtcMicros,
    occurred_at: UtcMicros,
) -> Result<AcceptedFinalizationOutcome, BusinessFinalizerError> {
    commit_accepted_finalization_inner(
        store,
        pending,
        template,
        policy,
        authority,
        verified_at,
        occurred_at,
        FinalizerStoreMode::Normal,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_accepted_finalization_with_fault(
    store: &mut BusinessIntentStore,
    pending: PendingAcceptedFinalization,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
    verified_at: UtcMicros,
    occurred_at: UtcMicros,
    fault: FinalizerFault,
) -> Result<AcceptedFinalizationOutcome, BusinessFinalizerError> {
    commit_accepted_finalization_inner(
        store,
        pending,
        template,
        policy,
        authority,
        verified_at,
        occurred_at,
        FinalizerStoreMode::Fault(fault),
    )
}

#[allow(clippy::too_many_arguments)]
fn commit_accepted_finalization_inner(
    store: &mut BusinessIntentStore,
    pending: PendingAcceptedFinalization,
    template: &TerminalTemplateBinding,
    policy: &CompletionPolicy,
    authority: &dyn TerminalAuthorityPort,
    verified_at: UtcMicros,
    occurred_at: UtcMicros,
    mode: FinalizerStoreMode,
) -> Result<AcceptedFinalizationOutcome, BusinessFinalizerError> {
    if verified_at > occurred_at {
        return Err(BusinessFinalizerError::InvalidRequest {
            check: "verification_after_transition",
        });
    }
    let current = store
        .inspect(&pending.intent_id)?
        .ok_or(IntentStoreError::IntentMissing)?;
    let chain = store.inspect_transition_chain(&pending.intent_id)?;
    if current.state() == IntentState::Completed {
        let receipt = chain
            .last()
            .filter(|event| event.to_state() == IntentState::Completed)
            .cloned()
            .ok_or(IntentStoreError::IntegrityFailed {
                check: "completed_head_event",
            })?;
        if receipt.matches_accepted_completion(
            &pending.prior,
            pending.qualified_version,
            &pending.actor,
            occurred_at,
        ) {
            let directive = accepted_directive(policy, &pending.prior)?;
            return Ok(AcceptedFinalizationOutcome::AlreadyCommitted { receipt, directive });
        }
        return isolate_finalizer_conflict(
            store,
            &pending.intent_id,
            &current,
            &pending.actor,
            occurred_at,
        );
    }
    if current.state() == IntentState::ResolutionRequired {
        return Err(BusinessFinalizerError::ConflictUnresolved {
            current: Box::new(current),
        });
    }
    if current.state() != IntentState::AwaitingFinalizer {
        return Err(BusinessFinalizerError::InvalidSourceState);
    }
    if current.version() != pending.qualified_version {
        return isolate_finalizer_conflict(
            store,
            &pending.intent_id,
            &current,
            &pending.actor,
            occurred_at,
        );
    }
    if !pending.fence.matches(&current, occurred_at) {
        return Err(BusinessFinalizerError::FenceMismatch);
    }

    let fresh = match reverify_for_finalization(
        &pending.prior,
        &current,
        template,
        policy,
        authority,
        verified_at,
    ) {
        Ok(fresh) => fresh,
        Err(source) => {
            return Err(record_terminal_invalid(
                store,
                &pending.intent_id,
                current.version(),
                &pending.actor,
                occurred_at,
                &pending.fence,
                source,
            ));
        }
    };
    require_accepted_disposition(fresh.verified_terminal().terminal_disposition())?;
    let directive = accepted_directive(policy, fresh.verified_terminal())?;
    let transition = match mode {
        FinalizerStoreMode::Normal => store.apply_accepted_finalization(
            fresh,
            current.version(),
            &pending.actor,
            occurred_at,
            &pending.fence.owner,
            pending.fence.generation,
            pending.fence.until,
        )?,
        #[cfg(test)]
        FinalizerStoreMode::Fault(fault) => store.apply_accepted_finalization_with_fault(
            fresh,
            current.version(),
            &pending.actor,
            occurred_at,
            &pending.fence.owner,
            pending.fence.generation,
            pending.fence.until,
            fault,
        )?,
    };
    match transition {
        TransitionOutcome::Applied(receipt) => {
            Ok(AcceptedFinalizationOutcome::Applied { receipt, directive })
        }
        TransitionOutcome::AlreadyCommitted(receipt) => {
            Ok(AcceptedFinalizationOutcome::AlreadyCommitted { receipt, directive })
        }
        TransitionOutcome::Conflict { current } => isolate_finalizer_conflict(
            store,
            &pending.intent_id,
            &current,
            &pending.actor,
            occurred_at,
        ),
    }
}

fn isolate_finalizer_conflict(
    store: &mut BusinessIntentStore,
    intent_id: &IntentId,
    current: &IntentSnapshot,
    actor: &TransitionActor,
    occurred_at: UtcMicros,
) -> Result<AcceptedFinalizationOutcome, BusinessFinalizerError> {
    match store.apply_finalizer_conflict(intent_id, current, actor, occurred_at)? {
        TransitionOutcome::Applied(receipt) | TransitionOutcome::AlreadyCommitted(receipt) => {
            Ok(AcceptedFinalizationOutcome::ResolutionRequired { receipt })
        }
        TransitionOutcome::Conflict { current } => {
            Err(BusinessFinalizerError::ConflictUnresolved { current })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_terminal_invalid(
    store: &mut BusinessIntentStore,
    intent_id: &IntentId,
    expected_version: u64,
    actor: &TransitionActor,
    occurred_at: UtcMicros,
    fence: &FinalizerFence,
    source: TerminalAuthorityError,
) -> BusinessFinalizerError {
    match store.apply_terminal_ref_invalid(
        intent_id,
        expected_version,
        actor,
        occurred_at,
        &fence.owner,
        fence.generation,
        fence.until,
    ) {
        Ok(TransitionOutcome::Applied(receipt))
        | Ok(TransitionOutcome::AlreadyCommitted(receipt)) => {
            BusinessFinalizerError::TerminalInvalid {
                source,
                receipt: Box::new(receipt),
            }
        }
        Ok(TransitionOutcome::Conflict { current }) => {
            BusinessFinalizerError::ConflictUnresolved { current }
        }
        Err(error) => BusinessFinalizerError::Store(error),
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

fn require_not_delivered_disposition(
    disposition: TerminalDisposition,
) -> Result<(), BusinessFinalizerError> {
    if disposition == TerminalDisposition::ManualConfirmedNotDelivered {
        return Ok(());
    }
    Err(BusinessFinalizerError::DispositionNotCompletable {
        actual: disposition,
    })
}

fn not_delivered_history_eligible(current: &IntentSnapshot, chain: &[TransitionReceipt]) -> bool {
    if chain.iter().any(|event| {
        matches!(
            event.to_state(),
            IntentState::AwaitingFinalizer | IntentState::Completed
        )
    }) {
        return false;
    }
    match current.state() {
        IntentState::AwaitingAuthority => true,
        IntentState::ResolutionRequired => chain
            .iter()
            .rev()
            .find(|event| {
                event.to_state() == IntentState::ResolutionRequired
                    && event.from_state() != IntentState::ResolutionRequired
            })
            .is_some_and(|event| {
                event.from_state() == IntentState::AwaitingAuthority
                    && event.reason() == crate::monitor::push_job::ReasonCode::TransportUncertain
            }),
        _ => false,
    }
}

fn accepted_directive(
    policy: &CompletionPolicy,
    terminal: &VerifiedTerminalRef,
) -> Result<CompletionDirective, BusinessFinalizerError> {
    let result = terminal.clone().into_delivery_result();
    evaluate_completion(policy, CompletionFact::Delivery(&result))
        .map_err(|_| BusinessFinalizerError::PolicyRejected)
}

fn not_delivered_directive(
    policy: &CompletionPolicy,
    terminal: &VerifiedTerminalRef,
) -> Result<CompletionDirective, BusinessFinalizerError> {
    let result = terminal.clone().into_delivery_result();
    evaluate_completion(policy, CompletionFact::Delivery(&result))
        .map_err(|_| BusinessFinalizerError::PolicyRejected)
}
