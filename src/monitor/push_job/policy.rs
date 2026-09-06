//! W03 closed reason vocabulary and pure completion policy.

use std::collections::BTreeSet;
use std::num::NonZeroU32;

use super::delivery::{AuthorityClass, DeliveryResult, DeliveryResultView, TerminalDisposition};
use super::identity::validate_text;
use super::{
    CompletionOwnerId, OccurrenceId, PushJobError, Result, Sha256Digest, SourceContractId, UnitId,
    UtcMicros,
};

macro_rules! reason_codes {
    ($($variant:ident => $value:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
        pub enum ReasonCode {
            $($variant),+
        }

        impl ReasonCode {
            pub const ALL: [Self; 52] = [$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
        }

        impl TryFrom<&str> for ReasonCode {
            type Error = PushJobError;

            fn try_from(value: &str) -> Result<Self> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    other => Err(PushJobError::InvalidReasonCode(other.to_owned())),
                }
            }
        }
    };
}

reason_codes! {
    ScheduleNotTradingDay => "schedule.not_trading_day",
    ScheduleWindowNotOpen => "schedule.window_not_open",
    ScheduleWindowExpired => "schedule.window_expired",
    ScheduleOccurrenceClosed => "schedule.occurrence_closed",
    ScheduleOccurrenceConflict => "schedule.occurrence_conflict",
    ScheduleWindowOpen => "schedule.window_open",
    ScheduleDeferred => "schedule.deferred",
    InputSourceRecovered => "input.source_recovered",
    ActivationReady => "activation.ready",
    InputSourceUnavailable => "input.source_unavailable",
    InputSourceUnready => "input.source_unready",
    InputEvidenceInvalid => "input.evidence_invalid",
    InputNoVerifiedBatch => "input.no_verified_batch",
    InputAccountSnapshotMissing => "input.account_snapshot_missing",
    InputNamespaceViolation => "input.namespace_violation",
    PolicyDisabled => "policy.disabled",
    PolicyStarved => "policy.starved",
    PolicyOptInDisabled => "policy.opt_in_disabled",
    PolicyCooldownActive => "policy.cooldown_active",
    PolicyDailyBudgetFull => "policy.daily_budget_full",
    PolicySuppressed => "policy.suppressed",
    IntentPayloadConflict => "intent.payload_conflict",
    IntentExpectedVersionConflict => "intent.expected_version_conflict",
    IntentLeaseHeld => "intent.lease_held",
    IntentTransitionConflict => "intent.transition_conflict",
    TransportRejected => "transport.rejected",
    TransportUncertain => "transport.uncertain",
    TransportNoChannelConfigured => "transport.no_channel_configured",
    TransportAllChannelsFailed => "transport.all_channels_failed",
    TransportPartiallyAccepted => "transport.partially_accepted",
    FinalizerTerminalRefInvalid => "finalizer.terminal_ref_invalid",
    FinalizerBindingMismatch => "finalizer.binding_mismatch",
    FinalizerCasConflict => "finalizer.cas_conflict",
    FinalizerDeadlineExceeded => "finalizer.deadline_exceeded",
    FinalizerTransitionAppendFailed => "finalizer.transition_append_failed",
    ActivationManifestMismatch => "activation.manifest_mismatch",
    ActivationGenerationConflict => "activation.generation_conflict",
    ActivationOwnerConflict => "activation.owner_conflict",
    ActivationCoreUnready => "activation.core_unready",
    ActivationProducerUnready => "activation.producer_unready",
    ShadowSemanticDiff => "shadow.semantic_diff",
    ShadowSideEffectAttempted => "shadow.side_effect_attempted",
    OperatorNotDelivered => "operator.not_delivered",
    OperatorUnauthorized => "operator.unauthorized",
    OperatorEvidenceInvalid => "operator.evidence_invalid",
    OperatorResolutionConflict => "operator.resolution_conflict",
    IntentCreated => "intent.created",
    IntentNoData => "intent.no_data",
    IntentDispatchClaimed => "intent.dispatch_claimed",
    IntentAuthorityVerified => "intent.authority_verified",
    FinalizerCompleted => "finalizer.completed",
    ActivationApplied => "activation.applied",
}

impl ReasonCode {
    pub(super) const fn allows_input_backoff(self) -> bool {
        matches!(
            self,
            Self::InputSourceUnavailable
                | Self::InputSourceUnready
                | Self::InputEvidenceInvalid
                | Self::InputNoVerifiedBatch
                | Self::InputAccountSnapshotMissing
        )
    }

    pub(super) const fn is_input_blocker(self) -> bool {
        matches!(
            self,
            Self::InputSourceUnavailable
                | Self::InputSourceUnready
                | Self::InputEvidenceInvalid
                | Self::InputNoVerifiedBatch
                | Self::InputAccountSnapshotMissing
                | Self::InputNamespaceViolation
        )
    }

    pub(super) const fn is_suppression_reason(self) -> bool {
        matches!(
            self,
            Self::PolicyCooldownActive | Self::PolicyDailyBudgetFull | Self::PolicySuppressed
        )
    }

    pub(super) const fn is_permanent_preparation_failure(self) -> bool {
        matches!(
            self,
            Self::InputEvidenceInvalid | Self::InputNamespaceViolation
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RetryPolicy {
    Never,
    InputBackoff {
        not_before: UtcMicros,
    },
    AuthorizedRejected {
        not_before: UtcMicros,
        max_attempts: NonZeroU32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryEligibility {
    Never,
    NotBefore(UtcMicros),
    EligibleInputRetry,
    EligibleAuthorizedRejected,
    RejectedAuthorizationRequired,
    AttemptsExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryDirective {
    reason: ReasonCode,
    eligibility: RetryEligibility,
}

impl RetryDirective {
    pub fn reason(self) -> ReasonCode {
        self.reason
    }

    pub fn eligibility(self) -> RetryEligibility {
        self.eligibility
    }
}

impl RetryPolicy {
    pub fn evaluate(
        &self,
        now: UtcMicros,
        attempts: u32,
        rejected_authorized: bool,
        reason: ReasonCode,
    ) -> RetryDirective {
        let eligibility = if reason == ReasonCode::TransportUncertain {
            RetryEligibility::Never
        } else {
            match self {
                Self::Never => RetryEligibility::Never,
                Self::InputBackoff { not_before } => {
                    if !reason.allows_input_backoff() {
                        RetryEligibility::Never
                    } else if now < *not_before {
                        RetryEligibility::NotBefore(*not_before)
                    } else {
                        RetryEligibility::EligibleInputRetry
                    }
                }
                Self::AuthorizedRejected {
                    not_before,
                    max_attempts,
                } => {
                    if reason != ReasonCode::TransportRejected {
                        RetryEligibility::Never
                    } else if !rejected_authorized {
                        RetryEligibility::RejectedAuthorizationRequired
                    } else if attempts >= max_attempts.get() {
                        RetryEligibility::AttemptsExhausted
                    } else if now < *not_before {
                        RetryEligibility::NotBefore(*not_before)
                    } else {
                        RetryEligibility::EligibleAuthorizedRejected
                    }
                }
            }
        };
        RetryDirective {
            reason,
            eligibility,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct CompletionPolicyId(String);

impl CompletionPolicyId {
    pub fn try_new(value: String) -> Result<Self> {
        validate_text("completion_policy_id", value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct CompletionPolicyVersion(String);

impl CompletionPolicyVersion {
    pub fn try_new(value: String) -> Result<Self> {
        validate_text("completion_policy_version", value).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdvanceEvent {
    AcceptedBound,
    AcceptedOrManualBound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ScheduleCloseBranch {
    OnAccepted,
    VerifiedNoData,
    ExplicitDisabled,
    SuppressedOccurrence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleClosePolicy {
    branches: BTreeSet<ScheduleCloseBranch>,
}

impl ScheduleClosePolicy {
    // W06 catalog registration is the first production caller of policy builders.
    #[allow(dead_code)]
    pub(crate) fn selected(branches: impl IntoIterator<Item = ScheduleCloseBranch>) -> Self {
        Self {
            branches: branches.into_iter().collect(),
        }
    }

    // W06 catalog registration is the first production caller of policy builders.
    #[allow(dead_code)]
    pub(crate) fn none() -> Self {
        Self {
            branches: BTreeSet::new(),
        }
    }

    pub fn allows(&self, branch: ScheduleCloseBranch) -> bool {
        self.branches.contains(&branch)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorPolicy {
    AcceptedBoundOnly,
    Never,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoDataPolicy {
    KeepOpen,
    CloseVerifiedOccurrence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisabledPolicy {
    KeepOpen,
    CloseDisabledOccurrence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UncertainPolicy {
    QuarantineThenVerifiedManual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlreadyTerminalPolicy {
    RequeryExactBinding,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinalizerKind {
    BoundCursor,
    ScheduleOnly,
    CompatibilityObservation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum RetentionClass {
    Migration,
    Regulatory,
    Model,
    Trading,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogOwnerRef {
    unit_id: UnitId,
    completion_owner: CompletionOwnerId,
    catalog_sha256: Sha256Digest,
}

impl CatalogOwnerRef {
    // W06 owns catalog SHA and completion-owner binding in production.
    #[allow(dead_code)]
    pub(crate) fn new(
        unit_id: UnitId,
        completion_owner: CompletionOwnerId,
        catalog_sha256: Sha256Digest,
    ) -> Self {
        Self {
            unit_id,
            completion_owner,
            catalog_sha256,
        }
    }

    pub fn unit_id(&self) -> &UnitId {
        &self.unit_id
    }

    pub fn completion_owner(&self) -> &CompletionOwnerId {
        &self.completion_owner
    }

    pub fn catalog_sha256(&self) -> &Sha256Digest {
        &self.catalog_sha256
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
// W06 is the first production module allowed to assemble registrations.
#[allow(dead_code)]
pub(crate) struct CompletionPolicyRegistration {
    id: CompletionPolicyId,
    version: CompletionPolicyVersion,
    declared_completion_owner: CompletionOwnerId,
    completion_owner: CatalogOwnerRef,
    advance_event: AdvanceEvent,
    schedule_close_policy: ScheduleClosePolicy,
    notification_cursor_policy: CursorPolicy,
    no_data_policy: NoDataPolicy,
    disabled_policy: DisabledPolicy,
    retry_policy: RetryPolicy,
    uncertain_manual_policy: UncertainPolicy,
    already_terminal_policy: AlreadyTerminalPolicy,
    allowed_authority: Vec<AuthorityClass>,
    finalizer_kind: FinalizerKind,
    retention_class: RetentionClass,
}

impl CompletionPolicyRegistration {
    // W06 catalog registration supplies these already-typed fields.
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: CompletionPolicyId,
        version: CompletionPolicyVersion,
        declared_completion_owner: CompletionOwnerId,
        completion_owner: CatalogOwnerRef,
        advance_event: AdvanceEvent,
        schedule_close_policy: ScheduleClosePolicy,
        notification_cursor_policy: CursorPolicy,
        no_data_policy: NoDataPolicy,
        disabled_policy: DisabledPolicy,
        retry_policy: RetryPolicy,
        uncertain_manual_policy: UncertainPolicy,
        already_terminal_policy: AlreadyTerminalPolicy,
        allowed_authority: Vec<AuthorityClass>,
        finalizer_kind: FinalizerKind,
        retention_class: RetentionClass,
    ) -> Self {
        Self {
            id,
            version,
            declared_completion_owner,
            completion_owner,
            advance_event,
            schedule_close_policy,
            notification_cursor_policy,
            no_data_policy,
            disabled_policy,
            retry_policy,
            uncertain_manual_policy,
            already_terminal_policy,
            allowed_authority,
            finalizer_kind,
            retention_class,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionPolicy {
    id: CompletionPolicyId,
    version: CompletionPolicyVersion,
    completion_owner: CatalogOwnerRef,
    advance_event: AdvanceEvent,
    schedule_close_policy: ScheduleClosePolicy,
    notification_cursor_policy: CursorPolicy,
    no_data_policy: NoDataPolicy,
    disabled_policy: DisabledPolicy,
    retry_policy: RetryPolicy,
    uncertain_manual_policy: UncertainPolicy,
    already_terminal_policy: AlreadyTerminalPolicy,
    allowed_authority: Vec<AuthorityClass>,
    finalizer_kind: FinalizerKind,
    retention_class: RetentionClass,
}

impl CompletionPolicy {
    // W06 catalog registration is the first production caller.
    #[allow(dead_code)]
    pub(crate) fn register(registration: CompletionPolicyRegistration) -> Result<Self> {
        if registration.declared_completion_owner
            != *registration.completion_owner.completion_owner()
        {
            return Err(PushJobError::InvalidCompletionPolicy(
                "completion owner must match the catalog owner reference",
            ));
        }
        if has_duplicate_authority(&registration.allowed_authority) {
            return Err(PushJobError::InvalidCompletionPolicy(
                "allowed authority classes must be unique",
            ));
        }
        match registration.finalizer_kind {
            FinalizerKind::BoundCursor => {
                if registration.notification_cursor_policy != CursorPolicy::AcceptedBoundOnly {
                    return Err(PushJobError::InvalidCompletionPolicy(
                        "bound cursor requires AcceptedBoundOnly cursor policy",
                    ));
                }
                if registration.allowed_authority.is_empty() {
                    return Err(PushJobError::InvalidCompletionPolicy(
                        "bound cursor requires at least one strong authority",
                    ));
                }
            }
            FinalizerKind::ScheduleOnly => {
                if registration.notification_cursor_policy != CursorPolicy::Never {
                    return Err(PushJobError::InvalidCompletionPolicy(
                        "schedule-only finalizer cannot advance a notification cursor",
                    ));
                }
                if registration
                    .schedule_close_policy
                    .allows(ScheduleCloseBranch::OnAccepted)
                    && registration.allowed_authority.is_empty()
                {
                    return Err(PushJobError::InvalidCompletionPolicy(
                        "accepted schedule closure requires a strong authority",
                    ));
                }
            }
            FinalizerKind::CompatibilityObservation => {
                if registration.notification_cursor_policy != CursorPolicy::Never {
                    return Err(PushJobError::InvalidCompletionPolicy(
                        "compatibility observation cannot advance a notification cursor",
                    ));
                }
                if !registration.allowed_authority.is_empty() {
                    return Err(PushJobError::InvalidCompletionPolicy(
                        "compatibility observation cannot list a strong authority",
                    ));
                }
            }
        }

        Ok(Self {
            id: registration.id,
            version: registration.version,
            completion_owner: registration.completion_owner,
            advance_event: registration.advance_event,
            schedule_close_policy: registration.schedule_close_policy,
            notification_cursor_policy: registration.notification_cursor_policy,
            no_data_policy: registration.no_data_policy,
            disabled_policy: registration.disabled_policy,
            retry_policy: registration.retry_policy,
            uncertain_manual_policy: registration.uncertain_manual_policy,
            already_terminal_policy: registration.already_terminal_policy,
            allowed_authority: registration.allowed_authority,
            finalizer_kind: registration.finalizer_kind,
            retention_class: registration.retention_class,
        })
    }

    pub fn id(&self) -> &CompletionPolicyId {
        &self.id
    }

    pub fn version(&self) -> &CompletionPolicyVersion {
        &self.version
    }

    pub fn completion_owner(&self) -> &CatalogOwnerRef {
        &self.completion_owner
    }

    pub fn retention_class(&self) -> RetentionClass {
        self.retention_class
    }
}

// Called by the W06-gated registration path and directly exercised by W03 tests.
#[allow(dead_code)]
fn has_duplicate_authority(authorities: &[AuthorityClass]) -> bool {
    authorities
        .iter()
        .enumerate()
        .any(|(index, authority)| authorities[index + 1..].contains(authority))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedEmptyEvidenceRef {
    occurrence: OccurrenceId,
    source_contract_id: SourceContractId,
    evidence_sha256: Sha256Digest,
    verified_at: UtcMicros,
}

impl VerifiedEmptyEvidenceRef {
    pub fn new(
        occurrence: OccurrenceId,
        source_contract_id: SourceContractId,
        evidence_sha256: Sha256Digest,
        verified_at: UtcMicros,
    ) -> Self {
        Self {
            occurrence,
            source_contract_id,
            evidence_sha256,
            verified_at,
        }
    }

    pub fn occurrence(&self) -> &OccurrenceId {
        &self.occurrence
    }

    pub fn source_contract_id(&self) -> &SourceContractId {
        &self.source_contract_id
    }

    pub fn evidence_sha256(&self) -> &Sha256Digest {
        &self.evidence_sha256
    }

    pub fn verified_at(&self) -> UtcMicros {
        self.verified_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisabledEvidenceRef {
    occurrence: OccurrenceId,
    reason: ReasonCode,
    evidence_sha256: Sha256Digest,
    observed_at: UtcMicros,
}

impl DisabledEvidenceRef {
    pub fn occurrence(&self) -> &OccurrenceId {
        &self.occurrence
    }

    pub fn reason(&self) -> ReasonCode {
        self.reason
    }

    pub fn evidence_sha256(&self) -> &Sha256Digest {
        &self.evidence_sha256
    }

    pub fn observed_at(&self) -> UtcMicros {
        self.observed_at
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CompletionFact<'a> {
    VerifiedNoData(&'a VerifiedEmptyEvidenceRef),
    ExplicitDisabled(&'a DisabledEvidenceRef),
    BlockedOnInput {
        reason: ReasonCode,
        retry_after: Option<UtcMicros>,
    },
    Suppressed {
        reason: ReasonCode,
        eligible_after: Option<UtcMicros>,
    },
    RetryableFailure {
        reason: ReasonCode,
        retry_after: Option<UtcMicros>,
    },
    PermanentFailure {
        reason: ReasonCode,
    },
    Delivery(&'a DeliveryResult),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleDirective {
    KeepOpen,
    CloseVerifiedNoData,
    CloseExplicitDisabled,
    CloseSuppressedOccurrence,
    CloseOnAccepted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorDirective {
    Never,
    AdvanceAccepted,
    AdvanceManualAccepted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManualDirective {
    None,
    QuarantineThenVerifiedManual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionDirective {
    schedule: ScheduleDirective,
    cursor: CursorDirective,
    retry: RetryDirective,
    manual: ManualDirective,
}

impl CompletionDirective {
    pub fn schedule(self) -> ScheduleDirective {
        self.schedule
    }

    pub fn cursor(self) -> CursorDirective {
        self.cursor
    }

    pub fn retry(self) -> RetryDirective {
        self.retry
    }

    pub fn manual(self) -> ManualDirective {
        self.manual
    }
}

pub fn evaluate_completion(
    policy: &CompletionPolicy,
    fact: CompletionFact<'_>,
) -> Result<CompletionDirective> {
    match fact {
        CompletionFact::VerifiedNoData(_) => Ok(directive(
            if policy.no_data_policy == NoDataPolicy::CloseVerifiedOccurrence
                && policy
                    .schedule_close_policy
                    .allows(ScheduleCloseBranch::VerifiedNoData)
            {
                ScheduleDirective::CloseVerifiedNoData
            } else {
                ScheduleDirective::KeepOpen
            },
            CursorDirective::Never,
            never_retry(ReasonCode::IntentNoData),
            ManualDirective::None,
        )),
        CompletionFact::ExplicitDisabled(evidence) => Ok(directive(
            if policy.disabled_policy == DisabledPolicy::CloseDisabledOccurrence
                && policy
                    .schedule_close_policy
                    .allows(ScheduleCloseBranch::ExplicitDisabled)
            {
                ScheduleDirective::CloseExplicitDisabled
            } else {
                ScheduleDirective::KeepOpen
            },
            CursorDirective::Never,
            never_retry(evidence.reason()),
            ManualDirective::None,
        )),
        CompletionFact::BlockedOnInput {
            reason,
            retry_after,
        }
        | CompletionFact::RetryableFailure {
            reason,
            retry_after,
        } => Ok(directive(
            ScheduleDirective::KeepOpen,
            CursorDirective::Never,
            preparation_retry(&policy.retry_policy, reason, retry_after),
            ManualDirective::None,
        )),
        CompletionFact::Suppressed {
            reason,
            eligible_after,
        } => Ok(directive(
            if policy
                .schedule_close_policy
                .allows(ScheduleCloseBranch::SuppressedOccurrence)
            {
                ScheduleDirective::CloseSuppressedOccurrence
            } else {
                ScheduleDirective::KeepOpen
            },
            CursorDirective::Never,
            eligible_after.map_or_else(
                || never_retry(reason),
                |not_before| RetryDirective {
                    reason,
                    eligibility: RetryEligibility::NotBefore(not_before),
                },
            ),
            ManualDirective::None,
        )),
        CompletionFact::PermanentFailure { reason } => Ok(directive(
            ScheduleDirective::KeepOpen,
            CursorDirective::Never,
            never_retry(reason),
            ManualDirective::None,
        )),
        CompletionFact::Delivery(result) => evaluate_delivery(policy, result),
    }
}

fn evaluate_delivery(
    policy: &CompletionPolicy,
    result: &DeliveryResult,
) -> Result<CompletionDirective> {
    match result.view() {
        DeliveryResultView::TransportAccepted(terminal) => {
            require_allowed_strong_policy(policy, terminal.authority_class())?;
            Ok(accepted_directive(policy, false))
        }
        DeliveryResultView::TransportRejected(terminal) => {
            require_allowed_strong_policy(policy, terminal.authority_class())?;
            let retry = match policy.retry_policy {
                RetryPolicy::AuthorizedRejected { .. } => RetryDirective {
                    reason: ReasonCode::TransportRejected,
                    eligibility: RetryEligibility::RejectedAuthorizationRequired,
                },
                RetryPolicy::Never | RetryPolicy::InputBackoff { .. } => {
                    never_retry(ReasonCode::TransportRejected)
                }
            };
            Ok(directive(
                ScheduleDirective::KeepOpen,
                CursorDirective::Never,
                retry,
                ManualDirective::None,
            ))
        }
        DeliveryResultView::TransportUncertain(terminal) => {
            require_allowed_strong_policy(policy, terminal.authority_class())?;
            debug_assert_eq!(
                policy.uncertain_manual_policy,
                UncertainPolicy::QuarantineThenVerifiedManual
            );
            Ok(directive(
                ScheduleDirective::KeepOpen,
                CursorDirective::Never,
                never_retry(ReasonCode::TransportUncertain),
                ManualDirective::QuarantineThenVerifiedManual,
            ))
        }
        DeliveryResultView::AlreadyTerminal(terminal) => {
            require_allowed_strong_policy(policy, terminal.authority_class())?;
            debug_assert_eq!(
                policy.already_terminal_policy,
                AlreadyTerminalPolicy::RequeryExactBinding
            );
            match terminal.terminal_disposition() {
                TerminalDisposition::ManualConfirmedAccepted => {
                    Ok(accepted_directive(policy, true))
                }
                TerminalDisposition::ManualConfirmedNotDelivered => Ok(directive(
                    ScheduleDirective::KeepOpen,
                    CursorDirective::Never,
                    never_retry(ReasonCode::OperatorNotDelivered),
                    ManualDirective::None,
                )),
                TerminalDisposition::Accepted
                | TerminalDisposition::Rejected
                | TerminalDisposition::Uncertain => Err(PushJobError::PolicyViolation(
                    "AlreadyTerminal contained a non-manual disposition",
                )),
            }
        }
        DeliveryResultView::BestEffortAccepted(_)
        | DeliveryResultView::PartiallyAccepted(_)
        | DeliveryResultView::NoChannelConfigured(_)
        | DeliveryResultView::AllChannelsFailed(_) => {
            require_compatibility_policy(policy)?;
            Ok(directive(
                ScheduleDirective::KeepOpen,
                CursorDirective::Never,
                never_retry(
                    result
                        .reason_code()
                        .unwrap_or(ReasonCode::IntentDispatchClaimed),
                ),
                ManualDirective::None,
            ))
        }
        DeliveryResultView::Blocked(reason) => Ok(directive(
            ScheduleDirective::KeepOpen,
            CursorDirective::Never,
            match policy.retry_policy {
                RetryPolicy::InputBackoff { not_before } if reason.allows_input_backoff() => {
                    RetryDirective {
                        reason,
                        eligibility: RetryEligibility::NotBefore(not_before),
                    }
                }
                RetryPolicy::Never
                | RetryPolicy::InputBackoff { .. }
                | RetryPolicy::AuthorizedRejected { .. } => never_retry(reason),
            },
            ManualDirective::None,
        )),
    }
}

fn accepted_directive(policy: &CompletionPolicy, manual: bool) -> CompletionDirective {
    let close = policy
        .schedule_close_policy
        .allows(ScheduleCloseBranch::OnAccepted);
    let cursor = if policy.finalizer_kind == FinalizerKind::BoundCursor
        && policy.notification_cursor_policy == CursorPolicy::AcceptedBoundOnly
    {
        match (policy.advance_event, manual) {
            (AdvanceEvent::AcceptedBound, false) | (AdvanceEvent::AcceptedOrManualBound, false) => {
                CursorDirective::AdvanceAccepted
            }
            (AdvanceEvent::AcceptedOrManualBound, true) => CursorDirective::AdvanceManualAccepted,
            (AdvanceEvent::AcceptedBound, true) => CursorDirective::Never,
        }
    } else {
        CursorDirective::Never
    };
    directive(
        if close {
            ScheduleDirective::CloseOnAccepted
        } else {
            ScheduleDirective::KeepOpen
        },
        cursor,
        never_retry(ReasonCode::IntentAuthorityVerified),
        ManualDirective::None,
    )
}

fn require_allowed_strong_policy(
    policy: &CompletionPolicy,
    authority: AuthorityClass,
) -> Result<()> {
    if policy.finalizer_kind == FinalizerKind::CompatibilityObservation {
        return Err(PushJobError::PolicyViolation(
            "compatibility policy cannot consume a strong terminal",
        ));
    }
    if !policy.allowed_authority.contains(&authority) {
        return Err(PushJobError::PolicyViolation(
            "terminal authority is not allowed by completion policy",
        ));
    }
    Ok(())
}

fn require_compatibility_policy(policy: &CompletionPolicy) -> Result<()> {
    if policy.finalizer_kind != FinalizerKind::CompatibilityObservation {
        return Err(PushJobError::PolicyViolation(
            "compatibility result requires CompatibilityObservation finalizer",
        ));
    }
    Ok(())
}

fn preparation_retry(
    policy: &RetryPolicy,
    reason: ReasonCode,
    fact_retry_after: Option<UtcMicros>,
) -> RetryDirective {
    match policy {
        RetryPolicy::InputBackoff { not_before } if reason.allows_input_backoff() => {
            RetryDirective {
                reason,
                eligibility: RetryEligibility::NotBefore(
                    fact_retry_after.map_or(*not_before, |fact| fact.max(*not_before)),
                ),
            }
        }
        RetryPolicy::Never
        | RetryPolicy::InputBackoff { .. }
        | RetryPolicy::AuthorizedRejected { .. } => never_retry(reason),
    }
}

fn never_retry(reason: ReasonCode) -> RetryDirective {
    RetryDirective {
        reason,
        eligibility: RetryEligibility::Never,
    }
}

fn directive(
    schedule: ScheduleDirective,
    cursor: CursorDirective,
    retry: RetryDirective,
    manual: ManualDirective,
) -> CompletionDirective {
    CompletionDirective {
        schedule,
        cursor,
        retry,
        manual,
    }
}

#[cfg(test)]
pub(super) struct PolicyFixtureOptions {
    pub(super) completion_owner: &'static str,
    pub(super) catalog_completion_owner: &'static str,
    pub(super) advance_event: AdvanceEvent,
    pub(super) cursor_policy: CursorPolicy,
    pub(super) retry_policy: RetryPolicy,
    pub(super) allowed_authority: Vec<AuthorityClass>,
    pub(super) finalizer_kind: FinalizerKind,
    pub(super) close_all_schedule_branches: bool,
    pub(super) no_data_policy: NoDataPolicy,
    pub(super) disabled_policy: DisabledPolicy,
}

#[cfg(test)]
pub(super) fn fixture_policy_options() -> PolicyFixtureOptions {
    PolicyFixtureOptions {
        completion_owner: "fixture-owner",
        catalog_completion_owner: "fixture-owner",
        advance_event: AdvanceEvent::AcceptedOrManualBound,
        cursor_policy: CursorPolicy::AcceptedBoundOnly,
        retry_policy: RetryPolicy::Never,
        allowed_authority: vec![AuthorityClass::GenericCounted],
        finalizer_kind: FinalizerKind::BoundCursor,
        close_all_schedule_branches: true,
        no_data_policy: NoDataPolicy::CloseVerifiedOccurrence,
        disabled_policy: DisabledPolicy::CloseDisabledOccurrence,
    }
}

#[cfg(test)]
pub(super) fn try_policy_fixture(options: PolicyFixtureOptions) -> Result<CompletionPolicy> {
    let owner = CompletionOwnerId::try_new(options.completion_owner.to_owned())?;
    let catalog_owner = CatalogOwnerRef::new(
        UnitId::try_new("MU-fixture".to_owned())?,
        CompletionOwnerId::try_new(options.catalog_completion_owner.to_owned())?,
        Sha256Digest::parse("fixture catalog", &"d".repeat(64))?,
    );
    CompletionPolicy::register(CompletionPolicyRegistration::new(
        CompletionPolicyId::try_new("fixture-policy".to_owned())?,
        CompletionPolicyVersion::try_new("v1".to_owned())?,
        owner,
        catalog_owner,
        options.advance_event,
        if options.close_all_schedule_branches {
            ScheduleClosePolicy::selected([
                ScheduleCloseBranch::OnAccepted,
                ScheduleCloseBranch::VerifiedNoData,
                ScheduleCloseBranch::ExplicitDisabled,
                ScheduleCloseBranch::SuppressedOccurrence,
            ])
        } else {
            ScheduleClosePolicy::none()
        },
        options.cursor_policy,
        options.no_data_policy,
        options.disabled_policy,
        options.retry_policy,
        UncertainPolicy::QuarantineThenVerifiedManual,
        AlreadyTerminalPolicy::RequeryExactBinding,
        options.allowed_authority,
        options.finalizer_kind,
        RetentionClass::Trading,
    ))
}

#[cfg(test)]
pub(super) fn policy_fixture(
    no_data_policy: NoDataPolicy,
    disabled_policy: DisabledPolicy,
    cursor_policy: CursorPolicy,
    retry_policy: RetryPolicy,
) -> CompletionPolicy {
    let mut options = fixture_policy_options();
    options.no_data_policy = no_data_policy;
    options.disabled_policy = disabled_policy;
    options.cursor_policy = cursor_policy;
    options.retry_policy = retry_policy;
    if cursor_policy == CursorPolicy::Never {
        options.finalizer_kind = FinalizerKind::ScheduleOnly;
    }
    try_policy_fixture(options).expect("valid policy fixture")
}

#[cfg(test)]
fn fixture_occurrence() -> OccurrenceId {
    use super::{
        derive_occurrence_id, BusinessDate, OccurrenceFamily, OccurrenceIdentityMaterial,
        OccurrenceKey,
    };

    derive_occurrence_id(&OccurrenceIdentityMaterial::new(
        BusinessDate::parse("2026-09-06").expect("fixture date"),
        OccurrenceFamily::try_new("daily".to_owned()).expect("fixture family"),
        OccurrenceKey::try_new("close".to_owned()).expect("fixture key"),
    ))
}

#[cfg(test)]
pub(super) fn verified_empty_fixture() -> &'static VerifiedEmptyEvidenceRef {
    use std::sync::OnceLock;

    static FIXTURE: OnceLock<VerifiedEmptyEvidenceRef> = OnceLock::new();
    FIXTURE.get_or_init(|| VerifiedEmptyEvidenceRef {
        occurrence: fixture_occurrence(),
        source_contract_id: SourceContractId::try_new("fixture-source-v1".to_owned())
            .expect("fixture source contract"),
        evidence_sha256: Sha256Digest::parse("fixture empty evidence", &"e".repeat(64))
            .expect("fixture empty digest"),
        verified_at: UtcMicros::try_new(1_788_705_600_000_000).expect("fixture verified time"),
    })
}

#[cfg(test)]
pub(super) fn disabled_fixture(reason: ReasonCode) -> &'static DisabledEvidenceRef {
    Box::leak(Box::new(DisabledEvidenceRef {
        occurrence: fixture_occurrence(),
        reason,
        evidence_sha256: Sha256Digest::parse("fixture disabled evidence", &"f".repeat(64))
            .expect("fixture disabled digest"),
        observed_at: UtcMicros::try_new(1_788_705_600_000_000).expect("fixture observed time"),
    }))
}
