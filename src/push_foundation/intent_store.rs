//! Attested business-intent storage. This module selects no default database and has no sink.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::monitor::push_job::{
    canonical_preimage, derive_decision_id, derive_intent_id, derive_occurrence_id, raw_digest,
    AudienceId, BusinessDate, CanonicalValue, CompletionOwnerId, DecisionId, IntentId,
    IntentIdentityMaterial, Namespace, OccurrenceFamily, OccurrenceId, OccurrenceIdentityMaterial,
    OccurrenceKey, PreparedPush, ReasonCode, RunId, Sha256Digest, SourceContractId, SubjectId,
    UnitId, UtcMicros,
};

use super::migration::{attest_connection, validate_database_path};
use super::{FoundationMigrationError, FoundationSchemaMigration};

const BUSY_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IntentStoreError {
    #[error(transparent)]
    Foundation(#[from] FoundationMigrationError),
    #[error("business database must already exist before opening intent storage")]
    DatabaseMissing,
    #[error("cannot open business intent database")]
    DatabaseOpenFailed,
    #[error("required SQLite connection safeguards are unavailable")]
    ConnectionSafeguardFailed,
    #[error("invalid initial intent: {check}")]
    InvalidInitialIntent { check: &'static str },
    #[error("invalid intent transition: {check}")]
    InvalidTransition { check: &'static str },
    #[error("business intent does not exist")]
    IntentMissing,
    #[error("immutable material conflicts with an existing intent")]
    ImmutableConflict { intent_id: String },
    #[error("business intent storage operation failed: {operation}")]
    StorageFailed { operation: &'static str },
    #[error("business intent persisted fact failed integrity check: {check}")]
    IntegrityFailed { check: &'static str },
    #[cfg(test)]
    #[error("injected W08 fault: {point}")]
    InjectedFault { point: &'static str },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitialDecisionKind {
    Ready,
    NoData,
    Disabled,
}

impl InitialDecisionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::NoData => "NoData",
            Self::Disabled => "Disabled",
        }
    }

    fn parse(value: &str) -> Result<Self, IntentStoreError> {
        match value {
            "Ready" => Ok(Self::Ready),
            "NoData" => Ok(Self::NoData),
            "Disabled" => Ok(Self::Disabled),
            _ => Err(IntentStoreError::IntegrityFailed {
                check: "job_decision_kind",
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntentState {
    PendingDispatch,
    AwaitingAuthority,
    AwaitingFinalizer,
    Completed,
    NotDelivered,
    NoData,
    Disabled,
    ResolutionRequired,
}

impl IntentState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PendingDispatch => "PendingDispatch",
            Self::AwaitingAuthority => "AwaitingAuthority",
            Self::AwaitingFinalizer => "AwaitingFinalizer",
            Self::Completed => "Completed",
            Self::NotDelivered => "NotDelivered",
            Self::NoData => "NoData",
            Self::Disabled => "Disabled",
            Self::ResolutionRequired => "ResolutionRequired",
        }
    }

    fn parse(value: &str) -> Result<Self, IntentStoreError> {
        match value {
            "PendingDispatch" => Ok(Self::PendingDispatch),
            "AwaitingAuthority" => Ok(Self::AwaitingAuthority),
            "AwaitingFinalizer" => Ok(Self::AwaitingFinalizer),
            "Completed" => Ok(Self::Completed),
            "NotDelivered" => Ok(Self::NotDelivered),
            "NoData" => Ok(Self::NoData),
            "Disabled" => Ok(Self::Disabled),
            "ResolutionRequired" => Ok(Self::ResolutionRequired),
            _ => Err(IntentStoreError::IntegrityFailed { check: "state" }),
        }
    }
}

fn validate_transition_text(
    field: &'static str,
    value: String,
) -> Result<String, IntentStoreError> {
    if value.is_empty() || value.len() > 512 || value.contains('\0') || value.trim() != value {
        return Err(IntentStoreError::InvalidTransition { check: field });
    }
    Ok(value)
}

macro_rules! transition_text {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name(String);

        impl $name {
            pub fn try_new(value: String) -> Result<Self, IntentStoreError> {
                validate_transition_text($field, value).map(Self)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

transition_text!(TransitionActor, "transition_actor");
transition_text!(LeaseOwnerId, "lease_owner");

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaseAction {
    Preserve,
    Acquire {
        owner: LeaseOwnerId,
        until: UtcMicros,
    },
    Release {
        owner: LeaseOwnerId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntentTransitionCommand {
    intent_id: IntentId,
    from_state: IntentState,
    to_state: IntentState,
    expected_version: u64,
    actor: TransitionActor,
    reason: ReasonCode,
    occurred_at: UtcMicros,
    lease_action: LeaseAction,
}

impl IntentTransitionCommand {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        intent_id: IntentId,
        from_state: IntentState,
        to_state: IntentState,
        expected_version: u64,
        actor: TransitionActor,
        reason: ReasonCode,
        occurred_at: UtcMicros,
        lease_action: LeaseAction,
    ) -> Result<Self, IntentStoreError> {
        if expected_version >= i64::MAX as u64 {
            return Err(IntentStoreError::InvalidTransition {
                check: "version_overflow",
            });
        }
        if !nonterminal_edge_allowed(from_state, to_state, reason) {
            return Err(IntentStoreError::InvalidTransition {
                check: "edge_reason_not_allowed",
            });
        }
        if let LeaseAction::Acquire { until, .. } = &lease_action {
            if *until <= occurred_at {
                return Err(IntentStoreError::InvalidTransition {
                    check: "lease_until_not_future",
                });
            }
        }
        Ok(Self {
            intent_id,
            from_state,
            to_state,
            expected_version,
            actor,
            reason,
            occurred_at,
            lease_action,
        })
    }
}

fn nonterminal_edge_allowed(from: IntentState, to: IntentState, reason: ReasonCode) -> bool {
    if from == to {
        return matches!(
            (from, reason),
            (
                IntentState::PendingDispatch
                    | IntentState::AwaitingAuthority
                    | IntentState::AwaitingFinalizer
                    | IntentState::ResolutionRequired,
                ReasonCode::IntentLeaseHeld | ReasonCode::IntentDispatchClaimed
            ) | (
                IntentState::AwaitingAuthority,
                ReasonCode::TransportRejected | ReasonCode::FinalizerTerminalRefInvalid
            ) | (
                IntentState::AwaitingFinalizer,
                ReasonCode::FinalizerTerminalRefInvalid
            )
        );
    }
    matches!(
        (from, to, reason),
        (
            IntentState::PendingDispatch,
            IntentState::AwaitingAuthority,
            ReasonCode::IntentDispatchClaimed
        ) | (
            IntentState::PendingDispatch,
            IntentState::NoData,
            ReasonCode::IntentNoData
        ) | (
            IntentState::PendingDispatch,
            IntentState::Disabled,
            ReasonCode::PolicyDisabled
        ) | (
            IntentState::PendingDispatch
                | IntentState::AwaitingAuthority
                | IntentState::AwaitingFinalizer
                | IntentState::Completed
                | IntentState::NoData
                | IntentState::Disabled,
            IntentState::ResolutionRequired,
            ReasonCode::IntentPayloadConflict
                | ReasonCode::IntentExpectedVersionConflict
                | ReasonCode::FinalizerCasConflict
        ) | (
            IntentState::AwaitingAuthority | IntentState::AwaitingFinalizer,
            IntentState::ResolutionRequired,
            ReasonCode::TransportUncertain | ReasonCode::OperatorResolutionConflict
        )
    )
}

fn persisted_edge_reason_allowed(from: IntentState, to: IntentState, reason: ReasonCode) -> bool {
    nonterminal_edge_allowed(from, to, reason)
        || matches!(
            (from, to, reason),
            (
                IntentState::AwaitingAuthority | IntentState::ResolutionRequired,
                IntentState::AwaitingFinalizer,
                ReasonCode::IntentAuthorityVerified
            ) | (
                IntentState::AwaitingFinalizer,
                IntentState::Completed,
                ReasonCode::FinalizerCompleted
            ) | (
                IntentState::AwaitingAuthority | IntentState::ResolutionRequired,
                IntentState::NotDelivered,
                ReasonCode::OperatorNotDelivered
            )
        )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialIntentIdentity {
    namespace: Namespace,
    unit_id: UnitId,
    occurrence: OccurrenceIdentityMaterial,
    completion_owner: CompletionOwnerId,
    source_contract_id: SourceContractId,
    subject: SubjectId,
    audience: AudienceId,
}

impl InitialIntentIdentity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        namespace: Namespace,
        unit_id: UnitId,
        occurrence: OccurrenceIdentityMaterial,
        completion_owner: CompletionOwnerId,
        source_contract_id: SourceContractId,
        subject: SubjectId,
        audience: AudienceId,
    ) -> Self {
        Self {
            namespace,
            unit_id,
            occurrence,
            completion_owner,
            source_contract_id,
            subject,
            audience,
        }
    }

    fn intent_id(&self) -> IntentId {
        derive_intent_id(&IntentIdentityMaterial::new(
            self.namespace.clone(),
            self.unit_id.clone(),
            self.completion_owner.clone(),
            self.source_contract_id.clone(),
            derive_occurrence_id(&self.occurrence),
            self.subject.clone(),
            self.audience.clone(),
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialIntentDraft {
    intent_id: IntentId,
    decision_kind: InitialDecisionKind,
    namespace: String,
    unit_id: String,
    occurrence_family: String,
    occurrence_key: String,
    completion_owner: String,
    source_contract_id: String,
    subject: String,
    audience: String,
    durable_decision_id: String,
    business_date: String,
    prepared_push_bytes: Option<Vec<u8>>,
    rendered_bytes: Option<Vec<u8>>,
    payload_sha256: Option<Sha256Digest>,
    rendered_sha256: Option<Sha256Digest>,
    evidence_sha256: Sha256Digest,
    template_sha256: Sha256Digest,
    source_contract_sha256: Sha256Digest,
    state: IntentState,
    reason: ReasonCode,
    created_at: UtcMicros,
}

impl InitialIntentDraft {
    pub fn ready(
        identity: InitialIntentIdentity,
        prepared: &PreparedPush,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        created_at: UtcMicros,
    ) -> Result<Self, IntentStoreError> {
        let intent_id = identity.intent_id();
        let occurrence = derive_occurrence_id(&identity.occurrence);
        if prepared.intent_id() != &intent_id
            || prepared.decision_id() != &derive_decision_id(&intent_id)
            || prepared.unit_id() != &identity.unit_id
            || prepared.occurrence() != &occurrence
            || prepared.subject() != &identity.subject
            || prepared.source_binding().source_contract_id() != &identity.source_contract_id
        {
            return Err(IntentStoreError::InvalidInitialIntent {
                check: "prepared_push_identity_binding",
            });
        }
        if prepared.rendered_bytes().is_empty() {
            return Err(IntentStoreError::InvalidInitialIntent {
                check: "rendered_bytes_non_empty",
            });
        }
        let prepared_snapshot = prepared.canonical_snapshot_bytes();
        let rendered_bytes = prepared.rendered_bytes().as_bytes().to_vec();
        Ok(Self::from_identity(
            identity,
            InitialDecisionKind::Ready,
            Some(prepared_snapshot.as_bytes().to_vec()),
            Some(rendered_bytes),
            Some(prepared_snapshot.sha256().clone()),
            Some(prepared.rendered_sha256().clone()),
            prepared.source_binding().evidence_fingerprint().clone(),
            template_sha256,
            source_contract_sha256,
            IntentState::PendingDispatch,
            ReasonCode::IntentCreated,
            created_at,
        ))
    }

    pub fn no_data(
        identity: InitialIntentIdentity,
        evidence_sha256: Sha256Digest,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        created_at: UtcMicros,
    ) -> Self {
        Self::from_identity(
            identity,
            InitialDecisionKind::NoData,
            None,
            None,
            None,
            None,
            evidence_sha256,
            template_sha256,
            source_contract_sha256,
            IntentState::NoData,
            ReasonCode::IntentNoData,
            created_at,
        )
    }

    pub fn disabled(
        identity: InitialIntentIdentity,
        evidence_sha256: Sha256Digest,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        created_at: UtcMicros,
    ) -> Self {
        Self::from_identity(
            identity,
            InitialDecisionKind::Disabled,
            None,
            None,
            None,
            None,
            evidence_sha256,
            template_sha256,
            source_contract_sha256,
            IntentState::Disabled,
            ReasonCode::PolicyDisabled,
            created_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_identity(
        identity: InitialIntentIdentity,
        decision_kind: InitialDecisionKind,
        prepared_push_bytes: Option<Vec<u8>>,
        rendered_bytes: Option<Vec<u8>>,
        payload_sha256: Option<Sha256Digest>,
        rendered_sha256: Option<Sha256Digest>,
        evidence_sha256: Sha256Digest,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        state: IntentState,
        reason: ReasonCode,
        created_at: UtcMicros,
    ) -> Self {
        let intent_id = identity.intent_id();
        let durable_decision_id = derive_decision_id(&intent_id).as_str().to_owned();
        Self {
            intent_id,
            decision_kind,
            namespace: namespace_storage(&identity.namespace),
            unit_id: identity.unit_id.as_str().to_owned(),
            occurrence_family: identity.occurrence.occurrence_family().as_str().to_owned(),
            occurrence_key: identity.occurrence.occurrence_key().as_str().to_owned(),
            completion_owner: identity.completion_owner.as_str().to_owned(),
            source_contract_id: identity.source_contract_id.as_str().to_owned(),
            subject: subject_storage(&identity.subject),
            audience: identity.audience.as_str().to_owned(),
            durable_decision_id,
            business_date: identity.occurrence.business_date().as_str().to_owned(),
            prepared_push_bytes,
            rendered_bytes,
            payload_sha256,
            rendered_sha256,
            evidence_sha256,
            template_sha256,
            source_contract_sha256,
            state,
            reason,
            created_at,
        }
    }

    pub fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntentSnapshot {
    intent_id: String,
    decision_kind: InitialDecisionKind,
    namespace: String,
    unit_id: String,
    occurrence_family: String,
    occurrence_key: String,
    completion_owner: String,
    source_contract_id: String,
    subject: String,
    audience: String,
    durable_decision_id: String,
    business_date: String,
    prepared_push_bytes: Option<Vec<u8>>,
    rendered_bytes: Option<Vec<u8>>,
    payload_sha256: Option<Sha256Digest>,
    rendered_sha256: Option<Sha256Digest>,
    evidence_sha256: Sha256Digest,
    template_sha256: Sha256Digest,
    source_contract_sha256: Sha256Digest,
    state: IntentState,
    previous_state: Option<IntentState>,
    reason: ReasonCode,
    lease_owner: Option<String>,
    lease_until: Option<UtcMicros>,
    lease_generation: u64,
    version: u64,
    created_at: UtcMicros,
    updated_at: UtcMicros,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttestedReadyIntent {
    pub(crate) namespace: Namespace,
    pub(crate) decision_id: DecisionId,
    pub(crate) intent_id: IntentId,
    pub(crate) unit_id: UnitId,
    pub(crate) occurrence: OccurrenceId,
    pub(crate) business_date: BusinessDate,
    pub(crate) completion_owner: CompletionOwnerId,
    pub(crate) subject: SubjectId,
    pub(crate) audience: AudienceId,
    pub(crate) template_sha256: Sha256Digest,
    pub(crate) rendered_sha256: Sha256Digest,
}

impl IntentSnapshot {
    pub fn intent_id(&self) -> &str {
        &self.intent_id
    }
    pub fn decision_kind(&self) -> InitialDecisionKind {
        self.decision_kind
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    pub fn subject(&self) -> &str {
        &self.subject
    }
    pub fn prepared_push_bytes(&self) -> Option<&[u8]> {
        self.prepared_push_bytes.as_deref()
    }
    pub fn rendered_bytes(&self) -> Option<&[u8]> {
        self.rendered_bytes.as_deref()
    }
    pub fn payload_sha256(&self) -> Option<&Sha256Digest> {
        self.payload_sha256.as_ref()
    }
    pub fn rendered_sha256(&self) -> Option<&Sha256Digest> {
        self.rendered_sha256.as_ref()
    }
    pub fn state(&self) -> IntentState {
        self.state
    }
    pub fn reason(&self) -> ReasonCode {
        self.reason
    }
    pub fn lease_generation(&self) -> u64 {
        self.lease_generation
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn previous_state(&self) -> Option<IntentState> {
        self.previous_state
    }
    pub fn lease_owner(&self) -> Option<&str> {
        self.lease_owner.as_deref()
    }
    pub fn lease_until(&self) -> Option<UtcMicros> {
        self.lease_until
    }

    pub(crate) fn attested_ready_binding(&self) -> Result<AttestedReadyIntent, IntentStoreError> {
        verify_snapshot(self)?;
        if self.decision_kind != InitialDecisionKind::Ready {
            return Err(IntentStoreError::IntegrityFailed {
                check: "terminal_binding_requires_ready",
            });
        }
        let namespace = parse_namespace(&self.namespace)?;
        let subject = parse_subject(&self.subject)?;
        let business_date =
            BusinessDate::parse(&self.business_date).map_err(|_| integrity("business_date"))?;
        let occurrence_material = OccurrenceIdentityMaterial::new(
            business_date.clone(),
            OccurrenceFamily::try_new(self.occurrence_family.clone())
                .map_err(|_| integrity("occurrence_family"))?,
            OccurrenceKey::try_new(self.occurrence_key.clone())
                .map_err(|_| integrity("occurrence_key"))?,
        );
        let unit_id = UnitId::try_new(self.unit_id.clone()).map_err(|_| integrity("unit_id"))?;
        let completion_owner = CompletionOwnerId::try_new(self.completion_owner.clone())
            .map_err(|_| integrity("completion_owner"))?;
        let source_contract_id = SourceContractId::try_new(self.source_contract_id.clone())
            .map_err(|_| integrity("source_contract_id"))?;
        let occurrence = derive_occurrence_id(&occurrence_material);
        let audience =
            AudienceId::try_new(self.audience.clone()).map_err(|_| integrity("audience"))?;
        let intent_id = derive_intent_id(&IntentIdentityMaterial::new(
            namespace.clone(),
            unit_id.clone(),
            completion_owner.clone(),
            source_contract_id,
            occurrence.clone(),
            subject.clone(),
            audience.clone(),
        ));
        let rendered_sha256 = self
            .rendered_sha256
            .clone()
            .ok_or_else(|| integrity("ready_rendered_sha256"))?;
        Ok(AttestedReadyIntent {
            namespace,
            decision_id: derive_decision_id(&intent_id),
            intent_id,
            unit_id,
            occurrence,
            business_date,
            completion_owner,
            subject,
            audience,
            template_sha256: self.template_sha256.clone(),
            rendered_sha256,
        })
    }

    fn immutable_matches(&self, draft: &InitialIntentDraft) -> bool {
        self.intent_id == draft.intent_id.as_str()
            && self.decision_kind == draft.decision_kind
            && self.namespace == draft.namespace
            && self.unit_id == draft.unit_id
            && self.occurrence_family == draft.occurrence_family
            && self.occurrence_key == draft.occurrence_key
            && self.completion_owner == draft.completion_owner
            && self.source_contract_id == draft.source_contract_id
            && self.subject == draft.subject
            && self.audience == draft.audience
            && self.durable_decision_id == draft.durable_decision_id
            && self.business_date == draft.business_date
            && self.prepared_push_bytes == draft.prepared_push_bytes
            && self.rendered_bytes == draft.rendered_bytes
            && self.payload_sha256 == draft.payload_sha256
            && self.rendered_sha256 == draft.rendered_sha256
            && self.evidence_sha256 == draft.evidence_sha256
            && self.template_sha256 == draft.template_sha256
            && self.source_contract_sha256 == draft.source_contract_sha256
            && self.created_at == draft.created_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InitialIntentOutcome {
    Inserted(IntentSnapshot),
    ExistingIdentical(IntentSnapshot),
}

impl InitialIntentOutcome {
    pub fn snapshot(&self) -> &IntentSnapshot {
        match self {
            Self::Inserted(snapshot) | Self::ExistingIdentical(snapshot) => snapshot,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionReceipt {
    event_id: Sha256Digest,
    intent_id: String,
    from_state: IntentState,
    to_state: IntentState,
    expected_version: u64,
    result_version: u64,
    previous_sha256: Option<Sha256Digest>,
    canonical_sha256: Sha256Digest,
    actor: String,
    reason: ReasonCode,
    terminal_disposition: Option<String>,
    terminal_decision_id: Option<String>,
    operator_audit_ref: Option<String>,
    operator_audit_sha256: Option<Sha256Digest>,
    terminal_ref_id: Option<String>,
    terminal_binding_sha256: Option<Sha256Digest>,
    occurred_at: UtcMicros,
}

impl TransitionReceipt {
    pub fn event_id(&self) -> &Sha256Digest {
        &self.event_id
    }
    pub fn intent_id(&self) -> &str {
        &self.intent_id
    }
    pub fn from_state(&self) -> IntentState {
        self.from_state
    }
    pub fn to_state(&self) -> IntentState {
        self.to_state
    }
    pub fn expected_version(&self) -> u64 {
        self.expected_version
    }
    pub fn result_version(&self) -> u64 {
        self.result_version
    }
    pub fn previous_sha256(&self) -> Option<&Sha256Digest> {
        self.previous_sha256.as_ref()
    }
    pub fn canonical_sha256(&self) -> &Sha256Digest {
        &self.canonical_sha256
    }
    pub fn actor(&self) -> &str {
        &self.actor
    }
    pub fn reason(&self) -> ReasonCode {
        self.reason
    }
    pub fn occurred_at(&self) -> UtcMicros {
        self.occurred_at
    }

    fn from_command(
        command: &IntentTransitionCommand,
        previous_sha256: Option<Sha256Digest>,
    ) -> Self {
        let result_version = command.expected_version + 1;
        let event_id = transition_event_id(
            command.intent_id.as_str(),
            command.expected_version,
            result_version,
        );
        let mut receipt = Self {
            event_id,
            intent_id: command.intent_id.as_str().to_owned(),
            from_state: command.from_state,
            to_state: command.to_state,
            expected_version: command.expected_version,
            result_version,
            previous_sha256,
            canonical_sha256: Sha256Digest::from_bytes([0; 32]),
            actor: command.actor.as_str().to_owned(),
            reason: command.reason,
            terminal_disposition: None,
            terminal_decision_id: None,
            operator_audit_ref: None,
            operator_audit_sha256: None,
            terminal_ref_id: None,
            terminal_binding_sha256: None,
            occurred_at: command.occurred_at,
        };
        receipt.canonical_sha256 = transition_canonical_sha256(&receipt);
        receipt
    }

    fn matches_command(&self, command: &IntentTransitionCommand, current: &IntentSnapshot) -> bool {
        let event_matches = self.intent_id == command.intent_id.as_str()
            && self.from_state == command.from_state
            && self.to_state == command.to_state
            && self.expected_version == command.expected_version
            && self.result_version == command.expected_version + 1
            && self.actor == command.actor.as_str()
            && self.reason == command.reason
            && self.occurred_at == command.occurred_at
            && self.terminal_disposition.is_none()
            && self.terminal_decision_id.is_none()
            && self.operator_audit_ref.is_none()
            && self.operator_audit_sha256.is_none()
            && self.terminal_ref_id.is_none()
            && self.terminal_binding_sha256.is_none();
        if !event_matches || current.version != self.result_version {
            return event_matches;
        }
        match &command.lease_action {
            LeaseAction::Preserve => true,
            LeaseAction::Acquire { owner, until } => {
                current.lease_owner.as_deref() == Some(owner.as_str())
                    && current.lease_until == Some(*until)
            }
            LeaseAction::Release { .. } => {
                current.lease_owner.is_none() && current.lease_until.is_none()
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransitionOutcome {
    Applied(TransitionReceipt),
    AlreadyCommitted(TransitionReceipt),
    Conflict { current: Box<IntentSnapshot> },
}

impl TransitionOutcome {
    pub fn receipt(&self) -> Option<&TransitionReceipt> {
        match self {
            Self::Applied(receipt) | Self::AlreadyCommitted(receipt) => Some(receipt),
            Self::Conflict { .. } => None,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InitialCommitFault {
    BeforeCommit,
    AfterCommitAckLost,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransitionFault {
    AfterCas,
    AfterAppend,
    AfterCommitAckLost,
}

pub struct BusinessIntentStore {
    connection: Connection,
}

impl BusinessIntentStore {
    pub fn open(database: &Path) -> Result<Self, IntentStoreError> {
        validate_database_path(database)?;
        match fs::symlink_metadata(database) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(IntentStoreError::DatabaseOpenFailed),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(IntentStoreError::DatabaseMissing)
            }
            Err(_) => return Err(IntentStoreError::DatabaseOpenFailed),
        }
        let canonical_database =
            fs::canonicalize(database).map_err(|_| IntentStoreError::DatabaseOpenFailed)?;
        let connection = Connection::open_with_flags(
            canonical_database,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| IntentStoreError::DatabaseOpenFailed)?;
        let migration = FoundationSchemaMigration::bundled()?;
        attest_connection(&connection, migration.ddl_sha256())?;
        connection
            .execute_batch(
                "PRAGMA query_only=OFF; PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;",
            )
            .map_err(|_| IntentStoreError::ConnectionSafeguardFailed)?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|_| IntentStoreError::ConnectionSafeguardFailed)?;
        for (pragma, expected) in [
            ("PRAGMA query_only", 0_i64),
            ("PRAGMA foreign_keys", 1_i64),
            ("PRAGMA recursive_triggers", 1_i64),
        ] {
            let actual: i64 = connection
                .query_row(pragma, [], |row| row.get(0))
                .map_err(|_| IntentStoreError::ConnectionSafeguardFailed)?;
            if actual != expected {
                return Err(IntentStoreError::ConnectionSafeguardFailed);
            }
        }
        Ok(Self { connection })
    }

    pub fn record_initial(
        &mut self,
        draft: &InitialIntentDraft,
    ) -> Result<InitialIntentOutcome, IntentStoreError> {
        self.record_initial_inner(draft, None)
    }

    #[cfg(test)]
    pub(crate) fn record_initial_with_fault(
        &mut self,
        draft: &InitialIntentDraft,
        fault: InitialCommitFault,
    ) -> Result<InitialIntentOutcome, IntentStoreError> {
        let point = match fault {
            InitialCommitFault::BeforeCommit => "before_initial_commit",
            InitialCommitFault::AfterCommitAckLost => "after_initial_commit_ack_lost",
        };
        self.record_initial_inner(draft, Some(point))
    }

    fn record_initial_inner(
        &mut self,
        draft: &InitialIntentDraft,
        fault: Option<&'static str>,
    ) -> Result<InitialIntentOutcome, IntentStoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "begin_initial",
            })?;
        if let Some(existing) = query_intent(&transaction, draft.intent_id.as_str())? {
            let chain_result = query_transition_chain(&transaction, &existing);
            transaction
                .rollback()
                .map_err(|_| IntentStoreError::StorageFailed {
                    operation: "rollback_initial_read",
                })?;
            chain_result?;
            return if existing.immutable_matches(draft) {
                Ok(InitialIntentOutcome::ExistingIdentical(existing))
            } else {
                Err(IntentStoreError::ImmutableConflict {
                    intent_id: draft.intent_id.as_str().to_owned(),
                })
            };
        }

        transaction
            .execute(
                "INSERT INTO push_intents(\
                    intent_id,job_decision_kind,namespace,unit_id,occurrence_family,occurrence_key,\
                    completion_owner,source_contract_id,subject,audience,durable_decision_id,\
                    business_date,prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256,\
                    evidence_sha256,template_sha256,source_contract_sha256,state,previous_state,reason,\
                    lease_owner,lease_until,lease_generation,version,created_at,updated_at\
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    draft.intent_id.as_str(),
                    draft.decision_kind.as_str(),
                    draft.namespace,
                    draft.unit_id,
                    draft.occurrence_family,
                    draft.occurrence_key,
                    draft.completion_owner,
                    draft.source_contract_id,
                    draft.subject,
                    draft.audience,
                    draft.durable_decision_id,
                    draft.business_date,
                    draft.prepared_push_bytes,
                    draft.rendered_bytes,
                    draft.payload_sha256.as_ref().map(Sha256Digest::as_str),
                    draft.rendered_sha256.as_ref().map(Sha256Digest::as_str),
                    draft.evidence_sha256.as_str(),
                    draft.template_sha256.as_str(),
                    draft.source_contract_sha256.as_str(),
                    draft.state.as_str(),
                    Option::<&str>::None,
                    draft.reason.as_str(),
                    Option::<&str>::None,
                    Option::<i64>::None,
                    0_i64,
                    0_i64,
                    draft.created_at.get(),
                    draft.created_at.get(),
                ],
            )
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "insert_initial",
            })?;
        if fault == Some("before_initial_commit") {
            transaction
                .rollback()
                .map_err(|_| IntentStoreError::StorageFailed {
                    operation: "rollback_initial_fault",
                })?;
            return Err(injected_fault("before_initial_commit"));
        }
        if transaction.commit().is_err() {
            return match self.inspect(&draft.intent_id)? {
                Some(existing) if existing.immutable_matches(draft) => {
                    query_transition_chain(&self.connection, &existing)?;
                    Ok(InitialIntentOutcome::ExistingIdentical(existing))
                }
                Some(_) => Err(IntentStoreError::ImmutableConflict {
                    intent_id: draft.intent_id.as_str().to_owned(),
                }),
                None => Err(IntentStoreError::StorageFailed {
                    operation: "commit_initial",
                }),
            };
        }
        if fault == Some("after_initial_commit_ack_lost") {
            return Err(injected_fault("after_initial_commit_ack_lost"));
        }

        let persisted =
            self.inspect(&draft.intent_id)?
                .ok_or(IntentStoreError::IntegrityFailed {
                    check: "initial_post_commit_missing",
                })?;
        if !persisted.immutable_matches(draft) {
            return Err(IntentStoreError::IntegrityFailed {
                check: "initial_post_commit_mismatch",
            });
        }
        query_transition_chain(&self.connection, &persisted)?;
        Ok(InitialIntentOutcome::Inserted(persisted))
    }

    pub fn apply_nonterminal_transition(
        &mut self,
        command: &IntentTransitionCommand,
    ) -> Result<TransitionOutcome, IntentStoreError> {
        self.apply_nonterminal_transition_inner(command, None)
    }

    #[cfg(test)]
    pub(crate) fn apply_nonterminal_transition_with_fault(
        &mut self,
        command: &IntentTransitionCommand,
        fault: TransitionFault,
    ) -> Result<TransitionOutcome, IntentStoreError> {
        let point = match fault {
            TransitionFault::AfterCas => "after_transition_cas",
            TransitionFault::AfterAppend => "after_transition_append",
            TransitionFault::AfterCommitAckLost => "after_transition_commit_ack_lost",
        };
        self.apply_nonterminal_transition_inner(command, Some(point))
    }

    fn apply_nonterminal_transition_inner(
        &mut self,
        command: &IntentTransitionCommand,
        fault: Option<&'static str>,
    ) -> Result<TransitionOutcome, IntentStoreError> {
        let result_version = command.expected_version + 1;
        let event_id = transition_event_id(
            command.intent_id.as_str(),
            command.expected_version,
            result_version,
        );
        if let Some(existing) = query_transition(&self.connection, event_id.as_str())? {
            let current = self
                .inspect(&command.intent_id)?
                .ok_or(IntentStoreError::IntentMissing)?;
            query_transition_chain(&self.connection, &current)?;
            return if existing.matches_command(command, &current) {
                Ok(TransitionOutcome::AlreadyCommitted(existing))
            } else {
                Ok(TransitionOutcome::Conflict {
                    current: Box::new(current),
                })
            };
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "begin_transition",
            })?;
        let current = query_intent(&transaction, command.intent_id.as_str())?
            .ok_or(IntentStoreError::IntentMissing)?;
        let chain = query_transition_chain(&transaction, &current)?;
        if current.state != command.from_state || current.version != command.expected_version {
            transaction
                .rollback()
                .map_err(|_| IntentStoreError::StorageFailed {
                    operation: "rollback_transition_conflict",
                })?;
            return Ok(TransitionOutcome::Conflict {
                current: Box::new(current),
            });
        }
        if command.occurred_at < current.updated_at {
            transaction
                .rollback()
                .map_err(|_| IntentStoreError::StorageFailed {
                    operation: "rollback_transition_time",
                })?;
            return Err(IntentStoreError::InvalidTransition {
                check: "occurred_at_before_current_update",
            });
        }
        let previous_sha256 = chain.last().map(|event| event.canonical_sha256.clone());
        let proposed = TransitionReceipt::from_command(command, previous_sha256);
        let (lease_owner, lease_until, lease_generation) = apply_lease_action(&current, command)?;
        if command.to_state == IntentState::AwaitingAuthority
            && command.reason == ReasonCode::IntentDispatchClaimed
            && (lease_owner.is_none()
                || lease_until.is_none_or(|until| until <= command.occurred_at.get()))
        {
            transaction
                .rollback()
                .map_err(|_| IntentStoreError::StorageFailed {
                    operation: "rollback_missing_dispatch_lease",
                })?;
            return Err(IntentStoreError::InvalidTransition {
                check: "dispatch_requires_active_lease",
            });
        }
        let affected = transaction
            .execute(
                "UPDATE push_intents SET \
                    state=?,previous_state=state,reason=?,lease_owner=?,lease_until=?,\
                    lease_generation=?,version=?,updated_at=? \
                 WHERE intent_id=? AND state=? AND version=? AND lease_generation=? \
                   AND lease_owner IS ? AND lease_until IS ?",
                params![
                    command.to_state.as_str(),
                    command.reason.as_str(),
                    lease_owner,
                    lease_until,
                    as_i64("lease_generation", lease_generation)?,
                    as_i64("result_version", result_version)?,
                    command.occurred_at.get(),
                    command.intent_id.as_str(),
                    command.from_state.as_str(),
                    as_i64("expected_version", command.expected_version)?,
                    as_i64("current_lease_generation", current.lease_generation)?,
                    current.lease_owner,
                    current.lease_until.map(UtcMicros::get),
                ],
            )
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "cas_transition",
            })?;
        if affected != 1 {
            transaction
                .rollback()
                .map_err(|_| IntentStoreError::StorageFailed {
                    operation: "rollback_zero_row_cas",
                })?;
            let current = self
                .inspect(&command.intent_id)?
                .ok_or(IntentStoreError::IntentMissing)?;
            return Ok(TransitionOutcome::Conflict {
                current: Box::new(current),
            });
        }
        if fault == Some("after_transition_cas") {
            transaction
                .rollback()
                .map_err(|_| IntentStoreError::StorageFailed {
                    operation: "rollback_after_cas_fault",
                })?;
            return Err(injected_fault("after_transition_cas"));
        }

        insert_transition(&transaction, &proposed)?;
        if fault == Some("after_transition_append") {
            transaction
                .rollback()
                .map_err(|_| IntentStoreError::StorageFailed {
                    operation: "rollback_after_append_fault",
                })?;
            return Err(injected_fault("after_transition_append"));
        }
        if transaction.commit().is_err() {
            if let Some(existing) = query_transition(&self.connection, event_id.as_str())? {
                let current = self
                    .inspect(&command.intent_id)?
                    .ok_or(IntentStoreError::IntentMissing)?;
                if existing.matches_command(command, &current) {
                    query_transition_chain(&self.connection, &current)?;
                    return Ok(TransitionOutcome::AlreadyCommitted(existing));
                }
            }
            return Err(IntentStoreError::StorageFailed {
                operation: "commit_transition",
            });
        }
        if fault == Some("after_transition_commit_ack_lost") {
            return Err(injected_fault("after_transition_commit_ack_lost"));
        }

        let persisted = query_transition(&self.connection, event_id.as_str())?
            .ok_or_else(|| integrity("transition_post_commit_missing"))?;
        if persisted != proposed {
            return Err(integrity("transition_post_commit_mismatch"));
        }
        let current = self
            .inspect(&command.intent_id)?
            .ok_or(IntentStoreError::IntentMissing)?;
        query_transition_chain(&self.connection, &current)?;
        Ok(TransitionOutcome::Applied(persisted))
    }

    pub fn inspect(
        &self,
        intent_id: &IntentId,
    ) -> Result<Option<IntentSnapshot>, IntentStoreError> {
        query_intent(&self.connection, intent_id.as_str())
    }

    pub fn inspect_transition_chain(
        &self,
        intent_id: &IntentId,
    ) -> Result<Vec<TransitionReceipt>, IntentStoreError> {
        let current = self
            .inspect(intent_id)?
            .ok_or(IntentStoreError::IntentMissing)?;
        query_transition_chain(&self.connection, &current)
    }

    #[cfg(test)]
    pub(crate) fn intent_count(&self) -> Result<u64, IntentStoreError> {
        let count: i64 = self
            .connection
            .query_row("SELECT count(*) FROM push_intents", [], |row| row.get(0))
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "count_intents",
            })?;
        u64::try_from(count).map_err(|_| IntentStoreError::IntegrityFailed {
            check: "intent_count",
        })
    }

    #[cfg(test)]
    pub(crate) fn transition_count(&self, intent_id: &IntentId) -> Result<u64, IntentStoreError> {
        let count: i64 = self
            .connection
            .query_row(
                "SELECT count(*) FROM push_intent_transitions WHERE intent_id=?",
                [intent_id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "count_transitions",
            })?;
        u64::try_from(count).map_err(|_| integrity("transition_count"))
    }
}

fn apply_lease_action(
    current: &IntentSnapshot,
    command: &IntentTransitionCommand,
) -> Result<(Option<String>, Option<i64>, u64), IntentStoreError> {
    match &command.lease_action {
        LeaseAction::Preserve => Ok((
            current.lease_owner.clone(),
            current.lease_until.map(UtcMicros::get),
            current.lease_generation,
        )),
        LeaseAction::Acquire { owner, until } => {
            if current.lease_owner.as_deref() != Some(owner.as_str())
                && current.lease_owner.is_some()
                && current
                    .lease_until
                    .is_some_and(|existing_until| existing_until > command.occurred_at)
            {
                return Err(IntentStoreError::InvalidTransition {
                    check: "foreign_lease_not_expired",
                });
            }
            let generation = current.lease_generation.checked_add(1).ok_or(
                IntentStoreError::InvalidTransition {
                    check: "lease_generation_overflow",
                },
            )?;
            Ok((
                Some(owner.as_str().to_owned()),
                Some(until.get()),
                generation,
            ))
        }
        LeaseAction::Release { owner } => {
            if current.lease_owner.as_deref() != Some(owner.as_str()) {
                return Err(IntentStoreError::InvalidTransition {
                    check: "lease_release_owner_mismatch",
                });
            }
            Ok((None, None, current.lease_generation))
        }
    }
}

fn insert_transition(
    connection: &Connection,
    receipt: &TransitionReceipt,
) -> Result<(), IntentStoreError> {
    connection
        .execute(
            "INSERT INTO push_intent_transitions(\
                event_id,intent_id,from_state,to_state,expected_version,result_version,\
                previous_sha256,canonical_sha256,actor,reason,terminal_disposition,\
                terminal_decision_id,operator_audit_ref,operator_audit_sha256,terminal_ref_id,\
                terminal_binding_sha256,occurred_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            params![
                receipt.event_id.as_str(),
                receipt.intent_id,
                receipt.from_state.as_str(),
                receipt.to_state.as_str(),
                as_i64("expected_version", receipt.expected_version)?,
                as_i64("result_version", receipt.result_version)?,
                receipt.previous_sha256.as_ref().map(Sha256Digest::as_str),
                receipt.canonical_sha256.as_str(),
                receipt.actor,
                receipt.reason.as_str(),
                receipt.terminal_disposition,
                receipt.terminal_decision_id,
                receipt.operator_audit_ref,
                receipt
                    .operator_audit_sha256
                    .as_ref()
                    .map(Sha256Digest::as_str),
                receipt.terminal_ref_id,
                receipt
                    .terminal_binding_sha256
                    .as_ref()
                    .map(Sha256Digest::as_str),
                receipt.occurred_at.get(),
            ],
        )
        .map_err(|_| IntentStoreError::StorageFailed {
            operation: "append_transition",
        })?;
    Ok(())
}

fn transition_event_id(
    intent_id: &str,
    expected_version: u64,
    result_version: u64,
) -> Sha256Digest {
    raw_digest(&canonical_preimage(
        "IntentTransitionV1",
        &BTreeMap::from([
            (
                "expected_version",
                CanonicalValue::Unsigned(expected_version),
            ),
            ("intent_id", CanonicalValue::String(intent_id.to_owned())),
            ("result_version", CanonicalValue::Unsigned(result_version)),
        ]),
    ))
}

fn transition_canonical_sha256(receipt: &TransitionReceipt) -> Sha256Digest {
    raw_digest(&canonical_preimage(
        "IntentTransitionV1",
        &BTreeMap::from([
            ("actor", CanonicalValue::String(receipt.actor.clone())),
            (
                "event_id",
                CanonicalValue::String(receipt.event_id.as_str().to_owned()),
            ),
            (
                "expected_version",
                CanonicalValue::Unsigned(receipt.expected_version),
            ),
            (
                "from_state",
                CanonicalValue::String(receipt.from_state.as_str().to_owned()),
            ),
            (
                "intent_id",
                CanonicalValue::String(receipt.intent_id.clone()),
            ),
            (
                "occurred_at",
                CanonicalValue::Unsigned(receipt.occurred_at.get() as u64),
            ),
            (
                "operator_audit_ref",
                optional_string_value(receipt.operator_audit_ref.as_deref()),
            ),
            (
                "operator_audit_sha256",
                optional_digest_value(receipt.operator_audit_sha256.as_ref()),
            ),
            (
                "previous_sha256",
                optional_digest_value(receipt.previous_sha256.as_ref()),
            ),
            (
                "reason",
                CanonicalValue::String(receipt.reason.as_str().to_owned()),
            ),
            (
                "result_version",
                CanonicalValue::Unsigned(receipt.result_version),
            ),
            (
                "terminal_binding_sha256",
                optional_digest_value(receipt.terminal_binding_sha256.as_ref()),
            ),
            (
                "terminal_decision_id",
                optional_string_value(receipt.terminal_decision_id.as_deref()),
            ),
            (
                "terminal_disposition",
                optional_string_value(receipt.terminal_disposition.as_deref()),
            ),
            (
                "terminal_ref_id",
                optional_string_value(receipt.terminal_ref_id.as_deref()),
            ),
            (
                "to_state",
                CanonicalValue::String(receipt.to_state.as_str().to_owned()),
            ),
        ]),
    ))
}

fn optional_string_value(value: Option<&str>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |value| {
        CanonicalValue::String(value.to_owned())
    })
}

fn optional_digest_value(value: Option<&Sha256Digest>) -> CanonicalValue {
    optional_string_value(value.map(Sha256Digest::as_str))
}

fn query_transition(
    connection: &Connection,
    event_id: &str,
) -> Result<Option<TransitionReceipt>, IntentStoreError> {
    let raw = connection
        .query_row(
            "SELECT event_id,intent_id,from_state,to_state,expected_version,result_version,\
                    previous_sha256,canonical_sha256,actor,reason,terminal_disposition,\
                    terminal_decision_id,operator_audit_ref,operator_audit_sha256,terminal_ref_id,\
                    terminal_binding_sha256,occurred_at \
             FROM push_intent_transitions WHERE event_id=?",
            [event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, i64>(16)?,
                ))
            },
        )
        .optional()
        .map_err(|_| IntentStoreError::StorageFailed {
            operation: "read_transition",
        })?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    if validate_transition_text("persisted_transition_actor", raw.8.clone()).is_err() {
        return Err(integrity("persisted_transition_actor"));
    }
    let receipt = TransitionReceipt {
        event_id: parse_digest("event_id", &raw.0)?,
        intent_id: raw.1,
        from_state: IntentState::parse(&raw.2)?,
        to_state: IntentState::parse(&raw.3)?,
        expected_version: parse_u64("transition_expected_version", raw.4)?,
        result_version: parse_u64("transition_result_version", raw.5)?,
        previous_sha256: parse_optional_digest("transition_previous_sha256", raw.6)?,
        canonical_sha256: parse_digest("transition_canonical_sha256", &raw.7)?,
        actor: raw.8,
        reason: ReasonCode::try_from(raw.9.as_str()).map_err(|_| integrity("transition_reason"))?,
        terminal_disposition: raw.10,
        terminal_decision_id: raw.11,
        operator_audit_ref: raw.12,
        operator_audit_sha256: parse_optional_digest("operator_audit_sha256", raw.13)?,
        terminal_ref_id: raw.14,
        terminal_binding_sha256: parse_optional_digest("terminal_binding_sha256", raw.15)?,
        occurred_at: parse_micros(raw.16)?,
    };
    verify_transition(&receipt)?;
    Ok(Some(receipt))
}

fn verify_transition(receipt: &TransitionReceipt) -> Result<(), IntentStoreError> {
    parse_digest("transition_intent_id", &receipt.intent_id)?;
    if receipt.result_version != receipt.expected_version + 1 {
        return Err(integrity("transition_version_step"));
    }
    if (receipt.result_version == 1) != receipt.previous_sha256.is_none() {
        return Err(integrity("transition_first_predecessor"));
    }
    if transition_event_id(
        &receipt.intent_id,
        receipt.expected_version,
        receipt.result_version,
    ) != receipt.event_id
    {
        return Err(integrity("transition_event_id"));
    }
    if transition_canonical_sha256(receipt) != receipt.canonical_sha256 {
        return Err(integrity("transition_canonical_sha256"));
    }
    if !persisted_edge_reason_allowed(receipt.from_state, receipt.to_state, receipt.reason) {
        return Err(integrity("transition_edge_reason"));
    }
    let terminal = matches!(
        receipt.to_state,
        IntentState::Completed | IntentState::NotDelivered
    );
    if terminal != receipt.terminal_ref_id.is_some()
        || terminal != receipt.terminal_binding_sha256.is_some()
        || terminal != receipt.terminal_disposition.is_some()
    {
        return Err(integrity("transition_terminal_group"));
    }
    if receipt.to_state == IntentState::Completed {
        if !matches!(
            receipt.terminal_disposition.as_deref(),
            Some("Accepted" | "ManualConfirmedAccepted")
        ) || receipt.terminal_decision_id.is_some()
            || receipt.operator_audit_ref.is_some()
            || receipt.operator_audit_sha256.is_some()
        {
            return Err(integrity("completed_terminal_group"));
        }
    } else if receipt.to_state == IntentState::NotDelivered {
        if receipt.terminal_disposition.as_deref() != Some("ManualConfirmedNotDelivered")
            || receipt.reason != ReasonCode::OperatorNotDelivered
            || receipt.terminal_decision_id.is_none()
            || receipt.operator_audit_ref.is_none()
            || receipt.operator_audit_sha256.is_none()
        {
            return Err(integrity("not_delivered_terminal_group"));
        }
    } else if receipt.terminal_decision_id.is_some()
        || receipt.operator_audit_ref.is_some()
        || receipt.operator_audit_sha256.is_some()
    {
        return Err(integrity("nonterminal_terminal_group"));
    }
    Ok(())
}

fn query_transition_chain(
    connection: &Connection,
    current: &IntentSnapshot,
) -> Result<Vec<TransitionReceipt>, IntentStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT event_id FROM push_intent_transitions \
             WHERE intent_id=? ORDER BY result_version",
        )
        .map_err(|_| IntentStoreError::StorageFailed {
            operation: "prepare_transition_chain",
        })?;
    let event_ids = statement
        .query_map([current.intent_id.as_str()], |row| row.get::<_, String>(0))
        .map_err(|_| IntentStoreError::StorageFailed {
            operation: "query_transition_chain",
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| IntentStoreError::StorageFailed {
            operation: "read_transition_chain",
        })?;
    drop(statement);
    let mut chain = Vec::with_capacity(event_ids.len());
    for event_id in event_ids {
        let event = query_transition(connection, &event_id)?
            .ok_or_else(|| integrity("transition_chain_member_missing"))?;
        let expected_result_version = chain.len() as u64 + 1;
        let expected_previous = chain
            .last()
            .map(|previous: &TransitionReceipt| &previous.canonical_sha256);
        if event.result_version != expected_result_version
            || event.expected_version + 1 != event.result_version
            || event.previous_sha256.as_ref() != expected_previous
            || chain
                .last()
                .is_some_and(|previous| event.occurred_at < previous.occurred_at)
            || chain
                .last()
                .is_some_and(|previous| previous.to_state != event.from_state)
        {
            return Err(integrity("transition_chain_continuity"));
        }
        if event.to_state == IntentState::NotDelivered {
            let no_authority_acceptance = !chain.iter().any(|previous| {
                matches!(
                    previous.to_state,
                    IntentState::AwaitingFinalizer | IntentState::Completed
                )
            });
            let eligible_origin = event.from_state == IntentState::AwaitingAuthority
                || (event.from_state == IntentState::ResolutionRequired
                    && chain
                        .iter()
                        .rev()
                        .find(|previous| {
                            previous.to_state == IntentState::ResolutionRequired
                                && previous.from_state != IntentState::ResolutionRequired
                        })
                        .is_some_and(|previous| {
                            previous.from_state == IntentState::AwaitingAuthority
                                && previous.reason == ReasonCode::TransportUncertain
                        }));
            if !no_authority_acceptance || !eligible_origin {
                return Err(integrity("not_delivered_history"));
            }
        }
        chain.push(event);
    }
    if chain.len() as u64 != current.version {
        return Err(integrity("intent_version_transition_count"));
    }
    let initial_state = match current.decision_kind {
        InitialDecisionKind::Ready => IntentState::PendingDispatch,
        InitialDecisionKind::NoData => IntentState::NoData,
        InitialDecisionKind::Disabled => IntentState::Disabled,
    };
    if chain
        .first()
        .is_some_and(|first| first.from_state != initial_state)
    {
        return Err(integrity("transition_chain_initial_state"));
    }
    if let Some(last) = chain.last() {
        if last.to_state != current.state
            || Some(last.from_state) != current.previous_state
            || last.reason != current.reason
            || last.occurred_at != current.updated_at
        {
            return Err(integrity("intent_transition_head_binding"));
        }
    } else if current.previous_state.is_some() {
        return Err(integrity("initial_previous_state"));
    }
    Ok(chain)
}

fn as_i64(check: &'static str, value: u64) -> Result<i64, IntentStoreError> {
    i64::try_from(value).map_err(|_| IntentStoreError::InvalidTransition { check })
}

fn injected_fault(point: &'static str) -> IntentStoreError {
    #[cfg(test)]
    {
        IntentStoreError::InjectedFault { point }
    }
    #[cfg(not(test))]
    {
        let _ = point;
        IntentStoreError::StorageFailed {
            operation: "test_fault_unavailable",
        }
    }
}

fn query_intent(
    connection: &Connection,
    intent_id: &str,
) -> Result<Option<IntentSnapshot>, IntentStoreError> {
    let raw = connection
        .query_row(
            "SELECT intent_id,job_decision_kind,namespace,unit_id,occurrence_family,occurrence_key,\
                    completion_owner,source_contract_id,subject,audience,durable_decision_id,\
                    business_date,prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256,\
                    evidence_sha256,template_sha256,source_contract_sha256,state,previous_state,reason,\
                    lease_owner,lease_until,lease_generation,version,created_at,updated_at \
             FROM push_intents WHERE intent_id=?",
            [intent_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, Option<Vec<u8>>>(12)?,
                    row.get::<_, Option<Vec<u8>>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, String>(17)?,
                    row.get::<_, String>(18)?,
                    row.get::<_, String>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, String>(21)?,
                    row.get::<_, Option<String>>(22)?,
                    row.get::<_, Option<i64>>(23)?,
                    row.get::<_, i64>(24)?,
                    row.get::<_, i64>(25)?,
                    row.get::<_, i64>(26)?,
                    row.get::<_, i64>(27)?,
                ))
            },
        )
        .optional()
        .map_err(|_| IntentStoreError::StorageFailed {
            operation: "read_intent",
        })?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let snapshot = IntentSnapshot {
        intent_id: raw.0,
        decision_kind: InitialDecisionKind::parse(&raw.1)?,
        namespace: raw.2,
        unit_id: raw.3,
        occurrence_family: raw.4,
        occurrence_key: raw.5,
        completion_owner: raw.6,
        source_contract_id: raw.7,
        subject: raw.8,
        audience: raw.9,
        durable_decision_id: raw.10,
        business_date: raw.11,
        prepared_push_bytes: raw.12,
        rendered_bytes: raw.13,
        payload_sha256: parse_optional_digest("payload_sha256", raw.14)?,
        rendered_sha256: parse_optional_digest("rendered_sha256", raw.15)?,
        evidence_sha256: parse_digest("evidence_sha256", &raw.16)?,
        template_sha256: parse_digest("template_sha256", &raw.17)?,
        source_contract_sha256: parse_digest("source_contract_sha256", &raw.18)?,
        state: IntentState::parse(&raw.19)?,
        previous_state: raw.20.as_deref().map(IntentState::parse).transpose()?,
        reason: ReasonCode::try_from(raw.21.as_str()).map_err(|_| {
            IntentStoreError::IntegrityFailed {
                check: "reason_code",
            }
        })?,
        lease_owner: raw.22,
        lease_until: raw.23.map(parse_micros).transpose()?,
        lease_generation: parse_u64("lease_generation", raw.24)?,
        version: parse_u64("version", raw.25)?,
        created_at: parse_micros(raw.26)?,
        updated_at: parse_micros(raw.27)?,
    };
    verify_snapshot(&snapshot)?;
    Ok(Some(snapshot))
}

fn verify_snapshot(snapshot: &IntentSnapshot) -> Result<(), IntentStoreError> {
    let namespace = parse_namespace(&snapshot.namespace)?;
    let subject = parse_subject(&snapshot.subject)?;
    let occurrence = OccurrenceIdentityMaterial::new(
        BusinessDate::parse(&snapshot.business_date).map_err(|_| integrity("business_date"))?,
        OccurrenceFamily::try_new(snapshot.occurrence_family.clone())
            .map_err(|_| integrity("occurrence_family"))?,
        OccurrenceKey::try_new(snapshot.occurrence_key.clone())
            .map_err(|_| integrity("occurrence_key"))?,
    );
    let derived = derive_intent_id(&IntentIdentityMaterial::new(
        namespace,
        UnitId::try_new(snapshot.unit_id.clone()).map_err(|_| integrity("unit_id"))?,
        CompletionOwnerId::try_new(snapshot.completion_owner.clone())
            .map_err(|_| integrity("completion_owner"))?,
        SourceContractId::try_new(snapshot.source_contract_id.clone())
            .map_err(|_| integrity("source_contract_id"))?,
        derive_occurrence_id(&occurrence),
        subject,
        AudienceId::try_new(snapshot.audience.clone()).map_err(|_| integrity("audience"))?,
    ));
    if derived.as_str() != snapshot.intent_id
        || derive_decision_id(&derived).as_str() != snapshot.durable_decision_id
    {
        return Err(integrity("intent_identity_binding"));
    }
    match snapshot.decision_kind {
        InitialDecisionKind::Ready => {
            let prepared = snapshot
                .prepared_push_bytes
                .as_deref()
                .ok_or_else(|| integrity("ready_payload_group"))?;
            let rendered = snapshot
                .rendered_bytes
                .as_deref()
                .ok_or_else(|| integrity("ready_payload_group"))?;
            if prepared.is_empty()
                || rendered.is_empty()
                || snapshot.payload_sha256.as_ref() != Some(&digest(prepared))
                || snapshot.rendered_sha256.as_ref() != Some(&digest(rendered))
            {
                return Err(integrity("ready_payload_binding"));
            }
        }
        InitialDecisionKind::NoData | InitialDecisionKind::Disabled => {
            if snapshot.prepared_push_bytes.is_some()
                || snapshot.rendered_bytes.is_some()
                || snapshot.payload_sha256.is_some()
                || snapshot.rendered_sha256.is_some()
            {
                return Err(integrity("non_send_payload_group"));
            }
        }
    }
    if snapshot.updated_at < snapshot.created_at {
        return Err(integrity("intent_time_order"));
    }
    if snapshot.lease_owner.is_some() != snapshot.lease_until.is_some() {
        return Err(integrity("intent_lease_pair"));
    }
    if snapshot.lease_generation > snapshot.version {
        return Err(integrity("intent_lease_generation"));
    }
    if snapshot.version == 0 {
        let (initial_state, initial_reason) = match snapshot.decision_kind {
            InitialDecisionKind::Ready => (IntentState::PendingDispatch, ReasonCode::IntentCreated),
            InitialDecisionKind::NoData => (IntentState::NoData, ReasonCode::IntentNoData),
            InitialDecisionKind::Disabled => (IntentState::Disabled, ReasonCode::PolicyDisabled),
        };
        if snapshot.state != initial_state
            || snapshot.reason != initial_reason
            || snapshot.previous_state.is_some()
            || snapshot.lease_owner.is_some()
            || snapshot.lease_generation != 0
            || snapshot.updated_at != snapshot.created_at
        {
            return Err(integrity("initial_intent_state"));
        }
    } else if snapshot.previous_state.is_none() {
        return Err(integrity("transitioned_previous_state"));
    }
    Ok(())
}

fn namespace_storage(namespace: &Namespace) -> String {
    match namespace {
        Namespace::Production => "Production".to_owned(),
        Namespace::Test { run_id } => format!("Test:{}", run_id.as_str()),
    }
}

fn subject_storage(subject: &SubjectId) -> String {
    match subject {
        SubjectId::Global => "Global".to_owned(),
        SubjectId::Entity(value) => format!("Entity:{}", value.as_str()),
    }
}

fn parse_namespace(value: &str) -> Result<Namespace, IntentStoreError> {
    if value == "Production" {
        return Ok(Namespace::Production);
    }
    value
        .strip_prefix("Test:")
        .ok_or_else(|| integrity("namespace"))
        .and_then(|run_id| {
            RunId::try_new(run_id.to_owned())
                .map(Namespace::test)
                .map_err(|_| integrity("namespace"))
        })
}

fn parse_subject(value: &str) -> Result<SubjectId, IntentStoreError> {
    if value == "Global" {
        return Ok(SubjectId::Global);
    }
    value
        .strip_prefix("Entity:")
        .ok_or_else(|| integrity("subject"))
        .and_then(|subject| SubjectId::entity(subject.to_owned()).map_err(|_| integrity("subject")))
}

fn parse_digest(field: &'static str, value: &str) -> Result<Sha256Digest, IntentStoreError> {
    Sha256Digest::parse(field, value).map_err(|_| integrity(field))
}

fn parse_optional_digest(
    field: &'static str,
    value: Option<String>,
) -> Result<Option<Sha256Digest>, IntentStoreError> {
    value
        .as_deref()
        .map(|value| parse_digest(field, value))
        .transpose()
}

fn parse_micros(value: i64) -> Result<UtcMicros, IntentStoreError> {
    UtcMicros::try_new(value).map_err(|_| integrity("utc_micros"))
}

fn parse_u64(check: &'static str, value: i64) -> Result<u64, IntentStoreError> {
    u64::try_from(value).map_err(|_| integrity(check))
}

fn digest(bytes: &[u8]) -> Sha256Digest {
    use sha2::{Digest, Sha256};

    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn integrity(check: &'static str) -> IntentStoreError {
    IntentStoreError::IntegrityFailed { check }
}
