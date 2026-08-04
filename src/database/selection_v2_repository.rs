//! BR-174/BR-182 schema-v2 durable staging repository.
//!
//! This module deliberately owns only persistence and recovery.  Acquisition,
//! selection, notification, and monitor scheduling remain outside this seam.

use super::selection_v2::{
    initialize_selection_v2_schema, initialize_selection_v2_schema_with_audit_session,
    verify_selection_v2_connection, SelectionV2SchemaError, SelectionV2StoreMode,
};
#[cfg(test)]
use super::selection_v2::{
    install_selection_v2_final_database_half_for_test,
    verify_selection_v2_final_database_half_for_test,
};
use crate::selection::audit::{
    AuditExactLookup, LockedSelectionAuditSession, SelectionAuditPhase, SelectionAuditRecord,
};
use crate::selection::config_activation_v2::PreparedConfigActivation;
use crate::selection::ingress_v2::PreparedSourceIngress;
use crate::selection::schema_v2::{
    canonical_json, sha256_json, CommitReceiptContentPreimage, CommittedAuditContentPreimage,
    ConfigActivationStageInputPreimage, FeedStatusKind, GenerationStageInputPreimage,
    IngressDecision, OutcomeClaimStageInputPreimage, OutcomeMarketRequestParametersPreimage,
    OutcomePhase, OutcomeStageInputPreimage, OutcomeTradingDateVectorPreimage,
    PreparedAuditContentPreimage, RequestKind, RunManifestContentPreimage, RunPayloadPreimage,
    RunRowHashPreimage, RunRowLogicalPrimaryKeyPreimage, RunStatus, SampleKeyPreimage,
    SelectionEvaluationAttemptRowContentPreimage, SelectionOutcomeAttemptRowContentPreimage,
    SelectionRecoveryEnvelopeRowContentPreimage, SelectionRejectionRowContentPreimage,
    SelectionRelationAttemptRowContentPreimage, SelectionSampleOutcomeRowContentPreimage,
    SelectionSampleRowContentPreimage, SelectionSourceBatchAttemptRowContentPreimage,
    SelectionSourceFactAttemptRowContentPreimage, SelectionSourceFactRowContentPreimage,
    SourceFactAttemptResult, SourceIngressStageInputPreimage, StagedDbPreimage, SubjectKind,
    DOMAIN_COMMITTED_AUDIT, DOMAIN_COMMIT_RECEIPT, DOMAIN_CONFIG_ACTIVATION_PAYLOAD,
    DOMAIN_CONFIG_ACTIVATION_STAGE, DOMAIN_GENERATION_PAYLOAD, DOMAIN_INGRESS_PAYLOAD,
    DOMAIN_OUTCOME_CLAIM_PAYLOAD, DOMAIN_OUTCOME_PAYLOAD, DOMAIN_PREPARED_AUDIT,
    DOMAIN_RECOVERY_ENVELOPE_ROW, DOMAIN_RUN_MANIFEST, DOMAIN_RUN_ROW, DOMAIN_RUN_ROW_LOGICAL_PK,
    DOMAIN_STAGED_DB, GENERATION_STAGE_PAYLOAD_SCHEMA, OUTCOME_CLAIM_STAGE_PAYLOAD_SCHEMA,
    OUTCOME_STAGE_PAYLOAD_SCHEMA, TABLE_EVALUATION_ATTEMPT, TABLE_OUTCOME_ATTEMPT,
    TABLE_RECOVERY_ENVELOPE, TABLE_REJECTION, TABLE_RELATION_ATTEMPT, TABLE_SAMPLE,
    TABLE_SAMPLE_OUTCOME, TABLE_SOURCE_BATCH_ATTEMPT, TABLE_SOURCE_FACT, TABLE_SOURCE_FACT_ATTEMPT,
};
use chrono::{DateTime, SecondsFormat, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Nullable, Text};
use thiserror::Error;

const CONFIG_ACTIVATION_PAYLOAD_SCHEMA: &str = "config-activation-stage-v1";
const SOURCE_INGRESS_PAYLOAD_SCHEMA: &str = "source-ingress-stage-v2";
const GENERATION_PAYLOAD_SCHEMA: &str = GENERATION_STAGE_PAYLOAD_SCHEMA;
const OUTCOME_CLAIM_PAYLOAD_SCHEMA: &str = OUTCOME_CLAIM_STAGE_PAYLOAD_SCHEMA;
const OUTCOME_PAYLOAD_SCHEMA: &str = OUTCOME_STAGE_PAYLOAD_SCHEMA;

#[derive(Debug, Error)]
pub enum SelectionV2RepositoryError {
    #[error(transparent)]
    Schema(#[from] SelectionV2SchemaError),
    #[error("selection-v2 repository database error: {0}")]
    Database(#[from] diesel::result::Error),
    #[error("selection-v2 exact snapshot database error: {0}")]
    ExactSnapshotDatabase(#[from] rusqlite::Error),
    #[error("selection-v2 repository canonicalization failed: {0}")]
    Canonical(String),
    #[error("selection-v2 repository audit verification failed: {0}")]
    Audit(String),
    #[error("selection-v2 repository audit I/O failed: {0}")]
    AuditIo(#[from] std::io::Error),
    #[error("selection-v2 repository audit JSON failed: {0}")]
    AuditJson(#[from] serde_json::Error),
    #[error("selection-v2 repository invariant {code}: {detail}")]
    Invariant { code: &'static str, detail: String },
    #[error(
        "selection-v2 config hash conflict: config_hash={config_hash} existing_run={existing_run_id} requested_run={requested_run_id}"
    )]
    ConfigHashConflict {
        config_hash: String,
        existing_run_id: String,
        requested_run_id: String,
    },
    #[error("selection-v2 replay conflict: subject_kind={subject_kind} subject_id={subject_id}")]
    ReplayConflict {
        subject_kind: String,
        subject_id: String,
    },
    #[error("selection-v2 transaction rollback failed: primary={primary}; rollback={rollback}")]
    RollbackFailed { primary: String, rollback: String },
}

pub(crate) type RepositoryResult<T> = Result<T, SelectionV2RepositoryError>;

fn invariant(code: &'static str, detail: impl Into<String>) -> SelectionV2RepositoryError {
    SelectionV2RepositoryError::Invariant {
        code,
        detail: detail.into(),
    }
}

fn canonical<T: serde::Serialize>(value: &T) -> RepositoryResult<String> {
    canonical_json(value).map_err(|error| SelectionV2RepositoryError::Canonical(error.to_string()))
}

fn hash<T: serde::Serialize>(value: &T) -> RepositoryResult<String> {
    sha256_json(value).map_err(|error| SelectionV2RepositoryError::Canonical(error.to_string()))
}

fn utc_nanos(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

/// A typed config activation stage request.  Strings and hashes emitted by the
/// preparation layer are intentionally excluded: the repository recomputes
/// them from these typed preimages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigActivationStageRequest {
    stage_input: ConfigActivationStageInputPreimage,
    run_payload: RunPayloadPreimage,
    recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
}

impl ConfigActivationStageRequest {
    #[allow(
        dead_code,
        reason = "BR-183 keeps config-activation persistence dormant until selection-v2 activation"
    )]
    pub(crate) fn try_from_prepared(prepared: &PreparedConfigActivation) -> RepositoryResult<Self> {
        Self::validated(
            prepared.stage_input().clone(),
            prepared.run_payload().clone(),
            prepared.recovery_envelope().clone(),
        )
    }

    pub(crate) fn validated(
        stage_input: ConfigActivationStageInputPreimage,
        run_payload: RunPayloadPreimage,
        recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
    ) -> RepositoryResult<Self> {
        let request = Self {
            stage_input,
            run_payload,
            recovery_envelope,
        };
        canonical_config_activation_envelope(&request)?;
        Ok(request)
    }

    pub(crate) fn with_owner_enveloped_at(
        mut self,
        enveloped_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        self.recovery_envelope.enveloped_at = utc_nanos(enveloped_at);
        canonical_config_activation_envelope(&self)?;
        Ok(self)
    }

    pub(crate) fn stage_run_id(&self) -> &str {
        &self.stage_input.stage_run_id
    }
}

/// A typed source-ingress stage request.  The complete domain rows are carried
/// in `stage_input`; no caller-provided content hash is trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIngressStageRequest {
    stage_input: SourceIngressStageInputPreimage,
    run_payload: RunPayloadPreimage,
    recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
}

impl SourceIngressStageRequest {
    pub(crate) fn try_from_prepared(prepared: &PreparedSourceIngress) -> RepositoryResult<Self> {
        Self::validated(
            prepared.stage_input().clone(),
            prepared.run_payload().clone(),
            prepared.recovery_envelope().clone(),
        )
    }

    pub(crate) fn validated(
        stage_input: SourceIngressStageInputPreimage,
        run_payload: RunPayloadPreimage,
        recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
    ) -> RepositoryResult<Self> {
        let request = Self {
            stage_input,
            run_payload,
            recovery_envelope,
        };
        canonical_source_ingress_envelope(&request)?;
        Ok(request)
    }

    pub(crate) fn with_owner_enveloped_at(
        mut self,
        enveloped_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        self.recovery_envelope.enveloped_at = utc_nanos(enveloped_at);
        canonical_source_ingress_envelope(&self)?;
        Ok(self)
    }

    pub(crate) fn stage_run_id(&self) -> &str {
        &self.stage_input.stage_run_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationStageRequest {
    stage_input: GenerationStageInputPreimage,
    run_payload: RunPayloadPreimage,
    recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
}

impl GenerationStageRequest {
    pub(crate) fn validated(
        stage_input: GenerationStageInputPreimage,
        run_payload: RunPayloadPreimage,
        recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
    ) -> RepositoryResult<Self> {
        let request = Self {
            stage_input,
            run_payload,
            recovery_envelope,
        };
        canonical_generation_envelope(&request)?;
        Ok(request)
    }

    pub(crate) fn with_owner_enveloped_at(
        mut self,
        enveloped_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        self.recovery_envelope.enveloped_at = utc_nanos(enveloped_at);
        canonical_generation_envelope(&self)?;
        Ok(self)
    }

    pub(crate) fn stage_run_id(&self) -> &str {
        &self.stage_input.stage_run_id
    }
}

/// Exact non-outcome stage capability reconstructed from one durable recovery
/// envelope. Outcome claim/run variants deliberately cannot be represented:
/// those stages remain exclusively owned by `OutcomeSettlementOwner`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "BR-183 retains by-value typed recovery capabilities while selection-v2 is disabled"
)]
pub(crate) enum NonOutcomeRecoveryStageRequest {
    ConfigActivation(ConfigActivationStageRequest),
    SourceIngress(SourceIngressStageRequest),
    Generation(GenerationStageRequest),
}

impl NonOutcomeRecoveryStageRequest {
    pub(crate) fn try_from_envelope(
        recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
    ) -> RepositoryResult<Option<Self>> {
        match recovery_envelope.subject_kind {
            SubjectKind::ConfigActivation => {
                let stage_input: ConfigActivationStageInputPreimage =
                    parse_canonical_payload(&recovery_envelope.payload_json)?;
                let run_payload = config_activation_run_payload(&stage_input);
                Ok(Some(Self::ConfigActivation(
                    ConfigActivationStageRequest::validated(
                        stage_input,
                        run_payload,
                        recovery_envelope,
                    )?,
                )))
            }
            SubjectKind::IngressRun => {
                let stage_input: SourceIngressStageInputPreimage =
                    parse_canonical_payload(&recovery_envelope.payload_json)?;
                let run_payload = source_ingress_run_payload(&stage_input)?;
                Ok(Some(Self::SourceIngress(
                    SourceIngressStageRequest::validated(
                        stage_input,
                        run_payload,
                        recovery_envelope,
                    )?,
                )))
            }
            SubjectKind::GenerationRun => {
                let stage_input: GenerationStageInputPreimage =
                    parse_canonical_payload(&recovery_envelope.payload_json)?;
                let run_payload = generation_run_payload(&stage_input)?;
                Ok(Some(Self::Generation(GenerationStageRequest::validated(
                    stage_input,
                    run_payload,
                    recovery_envelope,
                )?)))
            }
            SubjectKind::OutcomeClaim | SubjectKind::OutcomeRun => Ok(None),
        }
    }

    pub(crate) fn stage_run_id(&self) -> &str {
        match self {
            Self::ConfigActivation(request) => request.stage_run_id(),
            Self::SourceIngress(request) => request.stage_run_id(),
            Self::Generation(request) => request.stage_run_id(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeClaimStageRequest {
    stage_input: OutcomeClaimStageInputPreimage,
    run_payload: RunPayloadPreimage,
    recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
}

impl OutcomeClaimStageRequest {
    /// Internal recovery/owner seam. The public settlement API must only
    /// receive the opaque receipted claim minted after this request commits.
    pub(crate) fn validated(
        stage_input: OutcomeClaimStageInputPreimage,
        run_payload: RunPayloadPreimage,
        recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
    ) -> RepositoryResult<Self> {
        let request = Self {
            stage_input,
            run_payload,
            recovery_envelope,
        };
        canonical_outcome_claim_envelope(&request)?;
        Ok(request)
    }

    pub(crate) fn from_stage_input(
        stage_input: OutcomeClaimStageInputPreimage,
        enveloped_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        let run_payload = outcome_claim_run_payload(&stage_input);
        let payload_json = canonical(&stage_input)?;
        let recovery_envelope = SelectionRecoveryEnvelopeRowContentPreimage {
            domain: DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
            stage_run_id: stage_input.stage_run_id.clone(),
            subject_kind: SubjectKind::OutcomeClaim,
            logical_subject_key: stage_input.logical_subject_key.clone(),
            payload_schema: OUTCOME_CLAIM_PAYLOAD_SCHEMA.into(),
            payload_json_hash: crate::selection::schema_v2::sha256_bytes(payload_json.as_bytes()),
            payload_json,
            in_memory_payload_hash: hash(&run_payload)?,
            config_activation_run_id: stage_input.config_activation_run_id.clone(),
            config_hash: stage_input.config_hash.clone(),
            enveloped_at: utc_nanos(enveloped_at),
        };
        Self::validated(stage_input, run_payload, recovery_envelope)
    }

    pub(crate) fn with_owner_enveloped_at(
        mut self,
        enveloped_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        self.recovery_envelope.enveloped_at = utc_nanos(enveloped_at);
        canonical_outcome_claim_envelope(&self)?;
        Ok(self)
    }

    pub(crate) fn stage_run_id(&self) -> &str {
        &self.stage_input.stage_run_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeStageRequest {
    stage_input: OutcomeStageInputPreimage,
    run_payload: RunPayloadPreimage,
    recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
}

impl OutcomeStageRequest {
    /// Internal recovery seam only. Production callers must enter through
    /// `SelectionV2PersistenceOwner::commit_outcome(PreparedOutcomeStage)`.
    pub(crate) fn validated(
        stage_input: OutcomeStageInputPreimage,
        run_payload: RunPayloadPreimage,
        recovery_envelope: SelectionRecoveryEnvelopeRowContentPreimage,
    ) -> RepositoryResult<Self> {
        let request = Self {
            stage_input,
            run_payload,
            recovery_envelope,
        };
        canonical_outcome_envelope(&request)?;
        Ok(request)
    }

    pub(crate) fn from_prepared_outcome(
        prepared: crate::selection::outcome_v2::PreparedOutcomeStage,
        enveloped_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        let stage_input = prepared.into_stage_input();
        let rows = outcome_run_row_hashes(&stage_input)?;
        let run_payload = RunPayloadPreimage {
            domain: DOMAIN_OUTCOME_PAYLOAD.into(),
            subject_kind: SubjectKind::OutcomeRun,
            subject_id: stage_input.stage_run_id.clone(),
            logical_subject_key: stage_input.logical_subject_key.clone(),
            source_fact_key: None,
            config_activation_run_id: stage_input.config_activation_run_id.clone(),
            config_hash: stage_input.config_hash.clone(),
            config_snapshot_json_hash: None,
            config_activation_content_hash: None,
            config_activation_file_content_hash: None,
            config_effective_from_rfc3339_nanos_utc: None,
            artifact_valid_from: None,
            artifact_expires_at: None,
            executable_revision: None,
            legacy_cutover_snapshot_hash: None,
            generation_market_date: None,
            aggregator_observed_at_rfc3339_nanos_utc: None,
            ingress_source_batch_content_hash: None,
            outcome_phase: Some(stage_input.outcome_phase),
            stored_due_date: Some(stage_input.stored_due_date.clone()),
            outcome_claim_id: Some(stage_input.outcome_claim_id.clone()),
            planned_outcome_run_id: None,
            outcome_claim_receipt_content_hash: Some(
                stage_input.outcome_claim_receipt_content_hash.clone(),
            ),
            outcome_claim_due_binding_hash: Some(
                stage_input.outcome_claim_due_binding_hash.clone(),
            ),
            outcome_claim_provider_request_hash: Some(
                stage_input.outcome_claim_provider_request_hash.clone(),
            ),
            rows,
        };
        let payload_json = canonical(&stage_input)?;
        let in_memory_payload_hash = hash(&run_payload)?;
        let recovery_envelope = SelectionRecoveryEnvelopeRowContentPreimage {
            domain: DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
            stage_run_id: stage_input.stage_run_id.clone(),
            subject_kind: SubjectKind::OutcomeRun,
            logical_subject_key: stage_input.logical_subject_key.clone(),
            payload_schema: OUTCOME_PAYLOAD_SCHEMA.into(),
            payload_json_hash: crate::selection::schema_v2::sha256_bytes(payload_json.as_bytes()),
            payload_json,
            in_memory_payload_hash,
            config_activation_run_id: stage_input.config_activation_run_id.clone(),
            config_hash: stage_input.config_hash.clone(),
            enveloped_at: utc_nanos(enveloped_at),
        };
        Self::validated(stage_input, run_payload, recovery_envelope)
    }

    pub(crate) fn with_owner_enveloped_at(
        mut self,
        enveloped_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        self.recovery_envelope.enveloped_at = utc_nanos(enveloped_at);
        canonical_outcome_envelope(&self)?;
        Ok(self)
    }

    pub(crate) fn stage_run_id(&self) -> &str {
        &self.stage_input.stage_run_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageDisposition {
    Inserted,
    ExactReplay,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRunReceipt {
    pub disposition: StageDisposition,
    pub subject_kind: SubjectKind,
    pub subject_id: String,
    pub logical_subject_key: String,
    pub in_memory_payload_hash: String,
    pub recovery_envelope_content_hash: String,
    pub staged_db_content_hash: String,
    pub run_manifest_content_hash: String,
    pub expected_staged_row_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedRecoveryEnvelope {
    envelope: SelectionRecoveryEnvelopeRowContentPreimage,
    content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedRecoveryEnvelope {
    pub disposition: StageDisposition,
    envelope: SelectionRecoveryEnvelopeRowContentPreimage,
    content_hash: String,
    logical_subject_lock: Option<LogicalSubjectLockReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestReceiptedLogicalSubject {
    pub subject_kind: SubjectKind,
    pub subject_id: String,
    pub logical_subject_key: String,
    pub run_status: RunStatus,
    pub committed_at_rfc3339_nanos_utc: String,
    pub manifest_content_hash: String,
    pub receipt_content_hash: String,
    pub prepared_audit_hash: String,
    pub committed_audit_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicalSubjectLockState {
    Vacant,
    Recovering {
        subject_id: String,
        manifest_present: bool,
    },
    Receipted(LatestReceiptedLogicalSubject),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutcomeClaimLifecycleClass {
    ClaimPartial,
    ClaimActive,
    OutcomeRecovery,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutcomeClaimArtifactMatrix {
    claim_manifest: bool,
    claim_receipt: bool,
    outcome_envelope: bool,
    outcome_manifest: bool,
    outcome_receipt: bool,
    exact_claim_and_planned_run_binding: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutcomeClaimRecoveryMaterial {
    pub(crate) claim_stage: OutcomeClaimStageInputPreimage,
    pub(crate) claim_receipt_content_hash: Option<String>,
    pub(crate) outcome_stage: Option<OutcomeStageInputPreimage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutcomeClaimLifecycle {
    class: OutcomeClaimLifecycleClass,
    claim_stage: OutcomeClaimStageInputPreimage,
    claim_enveloped_at: String,
    claim_receipt_content_hash: Option<String>,
    outcome_stage: Option<OutcomeStageInputPreimage>,
}

impl OutcomeClaimLifecycle {
    pub(crate) fn class(&self) -> OutcomeClaimLifecycleClass {
        self.class
    }

    pub(crate) fn claim_id(&self) -> &str {
        &self.claim_stage.stage_run_id
    }

    pub(crate) fn planned_outcome_run_id(&self) -> &str {
        &self.claim_stage.planned_outcome_run_id
    }

    #[cfg(test)]
    pub(crate) fn claim_stage(&self) -> &OutcomeClaimStageInputPreimage {
        &self.claim_stage
    }

    pub(crate) fn blocks_new_due(&self) -> bool {
        self.class != OutcomeClaimLifecycleClass::Closed
    }

    #[cfg(test)]
    pub(crate) fn requires_provider_refetch(&self) -> bool {
        matches!(
            self.class,
            OutcomeClaimLifecycleClass::ClaimPartial | OutcomeClaimLifecycleClass::ClaimActive
        )
    }

    pub(crate) fn into_recovery_material(self) -> Option<OutcomeClaimRecoveryMaterial> {
        (self.class != OutcomeClaimLifecycleClass::Closed).then_some(OutcomeClaimRecoveryMaterial {
            claim_stage: self.claim_stage,
            claim_receipt_content_hash: self.claim_receipt_content_hash,
            outcome_stage: self.outcome_stage,
        })
    }
}

fn classify_outcome_claim_artifact_matrix(
    matrix: &OutcomeClaimArtifactMatrix,
) -> RepositoryResult<OutcomeClaimLifecycleClass> {
    if !matrix.exact_claim_and_planned_run_binding {
        return Err(invariant(
            "outcome_claim_lifecycle_cross_binding",
            "outcome artifacts do not bind the exact claim and planned outcome run",
        ));
    }
    match (
        matrix.claim_manifest,
        matrix.claim_receipt,
        matrix.outcome_envelope,
        matrix.outcome_manifest,
        matrix.outcome_receipt,
    ) {
        (false, false, false, false, false) | (true, false, false, false, false) => {
            Ok(OutcomeClaimLifecycleClass::ClaimPartial)
        }
        (true, true, false, false, false) => Ok(OutcomeClaimLifecycleClass::ClaimActive),
        (true, true, true, false, false) | (true, true, true, true, false) => {
            Ok(OutcomeClaimLifecycleClass::OutcomeRecovery)
        }
        (true, true, true, true, true) => Ok(OutcomeClaimLifecycleClass::Closed),
        _ => Err(invariant(
            "outcome_claim_lifecycle_mixed_artifacts",
            "claim/outcome artifact presence is not one closed recovery state",
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreparedStateAtLock {
    Missing,
    Exact { record_hash: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogicalSubjectLockReceipt {
    subject_kind: SubjectKind,
    subject_id: String,
    logical_subject_key: String,
    audit_tail_hash: Option<String>,
    prepared_state: PreparedStateAtLock,
    state: LogicalSubjectLockState,
    latest_receipted: Option<LatestReceiptedLogicalSubject>,
}

impl PersistedRecoveryEnvelope {
    pub fn subject_kind(&self) -> SubjectKind {
        self.envelope.subject_kind
    }

    pub fn stage_run_id(&self) -> &str {
        &self.envelope.stage_run_id
    }

    pub fn logical_subject_key(&self) -> &str {
        &self.envelope.logical_subject_key
    }

    pub fn in_memory_payload_hash(&self) -> &str {
        &self.envelope.in_memory_payload_hash
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn logical_subject_state(&self) -> Option<&LogicalSubjectLockState> {
        self.logical_subject_lock
            .as_ref()
            .map(|receipt| &receipt.state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReceipt {
    disposition: StageDisposition,
    subject_kind: SubjectKind,
    subject_id: String,
    content_hash: String,
    committed_at_rfc3339_nanos_utc: String,
}

impl CommitReceipt {
    pub fn disposition(&self) -> StageDisposition {
        self.disposition
    }

    pub fn subject_kind(&self) -> SubjectKind {
        self.subject_kind
    }

    pub fn subject_id(&self) -> &str {
        &self.subject_id
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub fn committed_at_rfc3339_nanos_utc(&self) -> &str {
        &self.committed_at_rfc3339_nanos_utc
    }
}

/// Capability proving that the exact typed Prepared content exists in a fully
/// validated on-disk audit chain.  Fields are private so callers cannot turn a
/// hash string into persistence authority.
#[derive(Debug, Clone)]
pub struct PreparedAuditProof {
    content: PreparedAuditContentPreimage,
    content_hash: String,
    record_hash: String,
    logical_subject_lock: LogicalSubjectLockReceipt,
}

/// Capability proving that the exact typed Committed content exists in a fully
/// validated on-disk audit chain.
#[derive(Debug, Clone)]
pub struct CommittedAuditProof {
    content: CommittedAuditContentPreimage,
    content_hash: String,
    record_hash: String,
    recorded_at: DateTime<Utc>,
}

impl PreparedAuditProof {
    fn load_locked(
        session: &mut LockedSelectionAuditSession<'_>,
        content: PreparedAuditContentPreimage,
        logical_subject_lock: &LogicalSubjectLockReceipt,
    ) -> RepositoryResult<Self> {
        validate_prepared_content(&content)?;
        require_lock_identity(logical_subject_lock, &content)?;
        let content_hash = hash(&content)?;
        let record = load_persisted_audit_record(
            session,
            prepared_phase(content.subject_kind),
            &content.subject_id,
            &content_hash,
        )?;
        match &logical_subject_lock.prepared_state {
            PreparedStateAtLock::Missing
                if record.previous_hash != logical_subject_lock.audit_tail_hash =>
            {
                return Err(invariant(
                    "logical_subject_lock_audit_tail_changed",
                    "Prepared is not the immediate audit successor of the locked logical-subject resolution",
                ));
            }
            PreparedStateAtLock::Exact { record_hash } if record.record_hash != *record_hash => {
                return Err(invariant(
                    "logical_subject_prepared_recovery_changed",
                    "recovered Prepared record differs from the record resolved under the lock",
                ));
            }
            PreparedStateAtLock::Missing | PreparedStateAtLock::Exact { .. } => {}
        }
        Ok(Self {
            content,
            content_hash,
            record_hash: record.record_hash,
            logical_subject_lock: logical_subject_lock.clone(),
        })
    }

    pub fn record_hash(&self) -> &str {
        &self.record_hash
    }
}

impl CommittedAuditProof {
    pub fn load(
        session: &mut LockedSelectionAuditSession<'_>,
        content: CommittedAuditContentPreimage,
    ) -> RepositoryResult<Self> {
        validate_committed_content(&content)?;
        let content_hash = hash(&content)?;
        let record = load_persisted_audit_record(
            session,
            committed_phase(content.subject_kind),
            &content.subject_id,
            &content_hash,
        )?;
        Ok(Self {
            content,
            content_hash,
            record_hash: record.record_hash,
            recorded_at: record.recorded_at.with_timezone(&Utc),
        })
    }

    pub fn record_hash(&self) -> &str {
        &self.record_hash
    }
}

fn load_persisted_audit_record(
    session: &mut LockedSelectionAuditSession<'_>,
    phase: SelectionAuditPhase,
    subject_id: &str,
    content_hash: &str,
) -> RepositoryResult<SelectionAuditRecord> {
    match session
        .lookup_exact(phase, subject_id, content_hash)
        .map_err(|error| SelectionV2RepositoryError::Audit(error.to_string()))?
    {
        AuditExactLookup::Exact(record) => Ok(record),
        AuditExactLookup::Missing => Err(invariant(
            "typed_audit_proof_missing",
            format!("phase={phase:?} subject_id={subject_id} content_hash={content_hash}"),
        )),
        AuditExactLookup::ContentConflict { existing_record } => Err(invariant(
            "typed_audit_proof_content_conflict",
            format!(
                "phase={phase:?} subject_id={subject_id} expected={content_hash} existing={}",
                existing_record.content_hash
            ),
        )),
    }
}

fn prepared_phase(subject_kind: SubjectKind) -> SelectionAuditPhase {
    match subject_kind {
        SubjectKind::ConfigActivation => SelectionAuditPhase::V2ConfigActivationPrepared,
        SubjectKind::IngressRun => SelectionAuditPhase::V2IngressPrepared,
        SubjectKind::GenerationRun => SelectionAuditPhase::V2GenerationPrepared,
        SubjectKind::OutcomeClaim => SelectionAuditPhase::V2OutcomeClaimPrepared,
        SubjectKind::OutcomeRun => SelectionAuditPhase::V2OutcomePrepared,
    }
}

fn committed_phase(subject_kind: SubjectKind) -> SelectionAuditPhase {
    match subject_kind {
        SubjectKind::ConfigActivation => SelectionAuditPhase::V2ConfigActivationCommitted,
        SubjectKind::IngressRun => SelectionAuditPhase::V2IngressCommitted,
        SubjectKind::GenerationRun => SelectionAuditPhase::V2GenerationCommitted,
        SubjectKind::OutcomeClaim => SelectionAuditPhase::V2OutcomeClaimCommitted,
        SubjectKind::OutcomeRun => SelectionAuditPhase::V2OutcomeCommitted,
    }
}

fn revalidate_prepared_proof(
    session: &mut LockedSelectionAuditSession<'_>,
    proof: &PreparedAuditProof,
) -> RepositoryResult<()> {
    let record = load_persisted_audit_record(
        session,
        prepared_phase(proof.content.subject_kind),
        &proof.content.subject_id,
        &proof.content_hash,
    )?;
    if record.record_hash != proof.record_hash {
        return Err(invariant(
            "prepared_audit_record_changed",
            "Prepared proof is no longer bound to the same locked audit record",
        ));
    }
    Ok(())
}

fn revalidate_committed_proof(
    session: &mut LockedSelectionAuditSession<'_>,
    proof: &CommittedAuditProof,
) -> RepositoryResult<()> {
    let record = load_persisted_audit_record(
        session,
        committed_phase(proof.content.subject_kind),
        &proof.content.subject_id,
        &proof.content_hash,
    )?;
    if record.record_hash != proof.record_hash
        || record.recorded_at.with_timezone(&Utc) != proof.recorded_at
    {
        return Err(invariant(
            "committed_audit_record_changed",
            "Committed proof is no longer bound to the same locked audit record",
        ));
    }
    Ok(())
}

fn validate_prepared_content(content: &PreparedAuditContentPreimage) -> RepositoryResult<()> {
    if content.domain != DOMAIN_PREPARED_AUDIT {
        return Err(invariant(
            "prepared_audit_domain_mismatch",
            "Prepared proof domain is not canonical",
        ));
    }
    require_subject_text(
        content.subject_kind,
        &content.subject_id,
        &content.logical_subject_key,
    )?;
    Ok(())
}

fn require_lock_identity(
    receipt: &LogicalSubjectLockReceipt,
    content: &PreparedAuditContentPreimage,
) -> RepositoryResult<()> {
    if receipt.subject_kind != content.subject_kind
        || receipt.subject_id != content.subject_id
        || receipt.logical_subject_key != content.logical_subject_key
    {
        return Err(invariant(
            "logical_subject_lock_identity_mismatch",
            "Prepared content is not bound to the logical subject resolved under the audit lock",
        ));
    }
    Ok(())
}

fn require_matching_lock_capabilities(
    persisted: &PersistedRecoveryEnvelope,
    proof: &PreparedAuditProof,
) -> RepositoryResult<()> {
    match &persisted.logical_subject_lock {
        Some(receipt) if receipt == &proof.logical_subject_lock => Ok(()),
        Some(_) => Err(invariant(
            "logical_subject_lock_capability_mismatch",
            "Prepared proof was issued for a different logical-subject resolution",
        )),
        None => Err(invariant(
            "logical_subject_lock_capability_missing",
            "stage requires an envelope resolved under the caller-held audit lock",
        )),
    }
}

fn validate_committed_content(content: &CommittedAuditContentPreimage) -> RepositoryResult<()> {
    if content.domain != DOMAIN_COMMITTED_AUDIT {
        return Err(invariant(
            "committed_audit_domain_mismatch",
            "Committed proof domain is not canonical",
        ));
    }
    require_subject_text(
        content.subject_kind,
        &content.subject_id,
        &content.logical_subject_key,
    )?;
    Ok(())
}

fn require_subject_text(
    _kind: SubjectKind,
    subject_id: &str,
    logical_subject_key: &str,
) -> RepositoryResult<()> {
    if subject_id.is_empty() || logical_subject_key.is_empty() {
        return Err(invariant(
            "subject_identity_empty",
            "subject id and logical subject key must be non-empty",
        ));
    }
    Ok(())
}

fn resolve_logical_subject_lock(
    conn: &mut SqliteConnection,
    session: &mut LockedSelectionAuditSession<'_>,
    envelope: &SelectionRecoveryEnvelopeRowContentPreimage,
) -> RepositoryResult<LogicalSubjectLockReceipt> {
    session
        .validate()
        .map_err(|error| SelectionV2RepositoryError::Audit(error.to_string()))?;
    if envelope.subject_kind == SubjectKind::OutcomeClaim {
        if let Some(lifecycle) =
            classify_outcome_claim_lifecycle(conn, &envelope.logical_subject_key)?
        {
            if lifecycle.blocks_new_due() && lifecycle.claim_id() != envelope.stage_run_id {
                return Err(invariant(
                    "outcome_claim_unclosed_conflict",
                    format!(
                        "logical_subject_key={} has unclosed claim {}; requested claim {} must recover the exact lifecycle",
                        envelope.logical_subject_key,
                        lifecycle.claim_id(),
                        envelope.stage_run_id
                    ),
                ));
            }
        }
    }
    let unreceipted = find_unreceipted_logical_subject(
        conn,
        envelope.subject_kind,
        &envelope.logical_subject_key,
    )?;
    if let Some(existing_run_id) = &unreceipted {
        if existing_run_id != &envelope.stage_run_id {
            return Err(invariant(
                "logical_subject_unreceipted_conflict",
                format!(
                    "logical_subject_key={} has recoverable run {}; requested run {} must not cross Prepared",
                    envelope.logical_subject_key, existing_run_id, envelope.stage_run_id
                ),
            ));
        }
    }
    let latest_receipted = load_latest_receipted_logical_subject(
        conn,
        session,
        envelope.subject_kind,
        &envelope.logical_subject_key,
    )?;
    let state = if let Some(subject_id) = unreceipted {
        LogicalSubjectLockState::Recovering {
            manifest_present: find_manifest(conn, &subject_id)?.is_some(),
            subject_id,
        }
    } else if let Some(latest) = latest_receipted.clone() {
        LogicalSubjectLockState::Receipted(latest)
    } else {
        LogicalSubjectLockState::Vacant
    };

    let prepared_content = PreparedAuditContentPreimage {
        domain: DOMAIN_PREPARED_AUDIT.into(),
        subject_kind: envelope.subject_kind,
        subject_id: envelope.stage_run_id.clone(),
        logical_subject_key: envelope.logical_subject_key.clone(),
        recovery_envelope_content_hash: hash(envelope)?,
        in_memory_payload_hash: envelope.in_memory_payload_hash.clone(),
    };
    let prepared_content_hash = hash(&prepared_content)?;
    let prepared_state = match session
        .lookup_exact(
            prepared_phase(envelope.subject_kind),
            &envelope.stage_run_id,
            &prepared_content_hash,
        )
        .map_err(|error| SelectionV2RepositoryError::Audit(error.to_string()))?
    {
        AuditExactLookup::Missing => PreparedStateAtLock::Missing,
        AuditExactLookup::Exact(record) => PreparedStateAtLock::Exact {
            record_hash: record.record_hash,
        },
        AuditExactLookup::ContentConflict { existing_record } => {
            return Err(invariant(
                "logical_subject_prepared_content_conflict",
                format!(
                    "subject_id={} expected={} existing={}",
                    envelope.stage_run_id, prepared_content_hash, existing_record.content_hash
                ),
            ));
        }
    };
    let audit_tail_hash = session
        .validate()
        .map_err(|error| SelectionV2RepositoryError::Audit(error.to_string()))?
        .tail_hash;
    Ok(LogicalSubjectLockReceipt {
        subject_kind: envelope.subject_kind,
        subject_id: envelope.stage_run_id.clone(),
        logical_subject_key: envelope.logical_subject_key.clone(),
        audit_tail_hash,
        prepared_state,
        state,
        latest_receipted,
    })
}

fn revalidate_logical_subject_lock(
    conn: &mut SqliteConnection,
    session: &mut LockedSelectionAuditSession<'_>,
    persisted: &PersistedRecoveryEnvelope,
    receipt: &LogicalSubjectLockReceipt,
) -> RepositoryResult<()> {
    if receipt.subject_kind != persisted.subject_kind()
        || receipt.subject_id != persisted.stage_run_id()
        || receipt.logical_subject_key != persisted.logical_subject_key()
    {
        return Err(invariant(
            "logical_subject_lock_identity_mismatch",
            "logical-subject lock receipt does not bind the persisted envelope",
        ));
    }
    if receipt.subject_kind == SubjectKind::OutcomeClaim {
        if let Some(lifecycle) =
            classify_outcome_claim_lifecycle(conn, &receipt.logical_subject_key)?
        {
            if lifecycle.blocks_new_due() && lifecycle.claim_id() != receipt.subject_id {
                return Err(invariant(
                    "outcome_claim_unclosed_conflict",
                    format!(
                        "logical_subject_key={} changed to unclosed claim {} while {} held the lock capability",
                        receipt.logical_subject_key,
                        lifecycle.claim_id(),
                        receipt.subject_id
                    ),
                ));
            }
        }
    }
    if let Some(existing_run_id) =
        find_unreceipted_logical_subject(conn, receipt.subject_kind, &receipt.logical_subject_key)?
    {
        if existing_run_id != receipt.subject_id {
            return Err(invariant(
                "logical_subject_unreceipted_conflict",
                format!(
                    "logical_subject_key={} changed to recoverable run {} while {} held the lock capability",
                    receipt.logical_subject_key, existing_run_id, receipt.subject_id
                ),
            ));
        }
    }
    let latest = load_latest_receipted_logical_subject(
        conn,
        session,
        receipt.subject_kind,
        &receipt.logical_subject_key,
    )?;
    if latest != receipt.latest_receipted {
        return Err(invariant(
            "logical_subject_latest_receipt_changed",
            "latest receipted logical subject changed after lock resolution",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigHashReuse {
    Absent,
    StagedUnreceipted {
        activation_run_id: String,
        manifest_content_hash: String,
        prepared_audit_hash: String,
    },
    ReceiptedExact {
        activation_run_id: String,
        manifest_content_hash: String,
        receipt_content_hash: String,
        prepared_audit_hash: String,
        committed_audit_hash: String,
    },
    Conflict {
        activation_run_id: String,
        manifest_content_hash: String,
    },
}

pub struct SelectionV2Repository {
    mode: SelectionV2StoreMode,
    #[cfg(test)]
    allow_final_database_half_for_test: bool,
}

impl SelectionV2Repository {
    pub(crate) fn initialize(
        conn: &mut SqliteConnection,
        mode: SelectionV2StoreMode,
    ) -> RepositoryResult<Self> {
        #[cfg(test)]
        if mode == SelectionV2StoreMode::Test {
            return Self::initialize_for_final_database_half_test(conn, mode);
        }
        initialize_selection_v2_schema(conn, mode)?;
        verify_selection_v2_connection(conn)?;
        Ok(Self {
            mode,
            #[cfg(test)]
            allow_final_database_half_for_test: false,
        })
    }

    #[cfg(test)]
    fn initialize_for_final_database_half_test(
        conn: &mut SqliteConnection,
        mode: SelectionV2StoreMode,
    ) -> RepositoryResult<Self> {
        install_selection_v2_final_database_half_for_test(conn, mode)?;
        Ok(Self {
            mode,
            allow_final_database_half_for_test: true,
        })
    }

    /// Initialize a persistence-owner repository from the exact audit-chain
    /// snapshot captured while the owner still holds the exclusive lock.
    ///
    /// This is intentionally separate from [`Self::initialize`]: production
    /// Production must never guess that the audit chain has no V2 evidence;
    /// tests use the same recovery reconciliation in a physically isolated
    /// namespace.
    pub(crate) fn initialize_with_audit_session(
        conn: &mut SqliteConnection,
        mode: SelectionV2StoreMode,
        audit_session: &mut LockedSelectionAuditSession<'_>,
    ) -> RepositoryResult<Self> {
        if mode == SelectionV2StoreMode::Production {
            initialize_selection_v2_schema_with_audit_session(conn, mode, audit_session)?;
        } else {
            initialize_selection_v2_schema(conn, mode)?;
        }
        verify_selection_v2_connection(conn)?;
        reconcile_database_and_audit(
            conn,
            audit_session,
            ReconciliationPurpose::PersistenceRecovery,
        )?;
        Ok(Self {
            mode,
            #[cfg(test)]
            allow_final_database_half_for_test: false,
        })
    }

    pub(crate) fn verify(&self, conn: &mut SqliteConnection) -> RepositoryResult<()> {
        #[cfg(test)]
        if self.allow_final_database_half_for_test {
            verify_selection_v2_final_database_half_for_test(conn, self.mode)?;
            return Ok(());
        }
        let _ = self.mode;
        verify_selection_v2_connection(conn)?;
        Ok(())
    }

    fn persist_validated_envelope(
        &self,
        conn: &mut SqliteConnection,
        validated: &ValidatedRecoveryEnvelope,
    ) -> RepositoryResult<PersistedRecoveryEnvelope> {
        self.verify(conn)?;
        if hash(&validated.envelope)? != validated.content_hash {
            return Err(invariant(
                "validated_envelope_hash_mismatch",
                "validated envelope capability does not bind its typed content",
            ));
        }

        let disposition = run_immediate_transaction(conn, |conn| {
            if let Some(existing_run_id) = find_unreceipted_logical_subject(
                conn,
                validated.envelope.subject_kind,
                &validated.envelope.logical_subject_key,
            )? {
                if existing_run_id != validated.envelope.stage_run_id {
                    return Err(invariant(
                        "logical_subject_unreceipted_conflict",
                        format!(
                            "logical_subject_key={} has recoverable run {}; requested run {} must not cross Prepared",
                            validated.envelope.logical_subject_key,
                            existing_run_id,
                            validated.envelope.stage_run_id
                        ),
                    ));
                }
            }
            if let Some(existing) = find_envelope(conn, &validated.envelope.stage_run_id)? {
                if existing.content_hash == validated.content_hash {
                    return Ok(StageDisposition::ExactReplay);
                }
                return Err(SelectionV2RepositoryError::ReplayConflict {
                    subject_kind: validated.envelope.subject_kind.as_str().into(),
                    subject_id: validated.envelope.stage_run_id.clone(),
                });
            }
            insert_envelope(conn, &validated.envelope, &validated.content_hash)?;
            Ok(StageDisposition::Inserted)
        })?;
        let persisted = verify_persisted_envelope(conn, validated)?;
        Ok(PersistedRecoveryEnvelope {
            disposition,
            envelope: persisted,
            content_hash: validated.content_hash.clone(),
            logical_subject_lock: None,
        })
    }

    fn require_exact_persisted_envelope(
        &self,
        conn: &mut SqliteConnection,
        validated: &ValidatedRecoveryEnvelope,
    ) -> RepositoryResult<PersistedRecoveryEnvelope> {
        self.verify(conn)?;
        if find_envelope(conn, &validated.envelope.stage_run_id)?.is_none() {
            return Err(invariant(
                "stage_envelope_missing",
                format!(
                    "domain stage requires a durable recovery envelope for {}",
                    validated.envelope.stage_run_id
                ),
            ));
        }
        let persisted = verify_persisted_envelope(conn, validated)?;
        Ok(PersistedRecoveryEnvelope {
            disposition: StageDisposition::ExactReplay,
            envelope: persisted,
            content_hash: validated.content_hash.clone(),
            logical_subject_lock: None,
        })
    }

    fn persist_locked_validated_envelope(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        validated: &ValidatedRecoveryEnvelope,
    ) -> RepositoryResult<PersistedRecoveryEnvelope> {
        let logical_subject_lock =
            resolve_logical_subject_lock(conn, audit_session, &validated.envelope)?;
        let mut persisted = self.persist_validated_envelope(conn, validated)?;
        persisted.logical_subject_lock = Some(logical_subject_lock);
        Ok(persisted)
    }

    pub(crate) fn persist_config_activation_envelope(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        request: &ConfigActivationStageRequest,
    ) -> RepositoryResult<PersistedRecoveryEnvelope> {
        let canonical = canonical_config_activation_envelope(request)?;
        self.persist_locked_validated_envelope(conn, audit_session, &canonical.validated_envelope)
    }

    pub(crate) fn persist_source_ingress_envelope(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        request: &SourceIngressStageRequest,
    ) -> RepositoryResult<PersistedRecoveryEnvelope> {
        let canonical = canonical_source_ingress_envelope(request)?;
        self.persist_locked_validated_envelope(conn, audit_session, &canonical.validated_envelope)
    }

    pub(crate) fn persist_generation_envelope(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        request: &GenerationStageRequest,
    ) -> RepositoryResult<PersistedRecoveryEnvelope> {
        let canonical = canonical_generation_envelope(request)?;
        self.persist_locked_validated_envelope(conn, audit_session, &canonical.validated_envelope)
    }

    pub(crate) fn persist_outcome_claim_envelope(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        request: &OutcomeClaimStageRequest,
    ) -> RepositoryResult<PersistedRecoveryEnvelope> {
        let canonical = canonical_outcome_claim_envelope(request)?;
        self.persist_locked_validated_envelope(conn, audit_session, &canonical.validated_envelope)
    }

    pub(crate) fn persist_outcome_envelope(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        request: &OutcomeStageRequest,
    ) -> RepositoryResult<PersistedRecoveryEnvelope> {
        self.verify(conn)?;
        let canonical = canonical_outcome_envelope(request)?;
        let logical_subject_lock = resolve_logical_subject_lock(
            conn,
            audit_session,
            &canonical.validated_envelope.envelope,
        )?;
        let disposition = run_immediate_transaction(conn, |conn| {
            load_outcome_authority(conn, &request.stage_input)?;
            if let Some(existing_run_id) = find_unreceipted_logical_subject(
                conn,
                canonical.validated_envelope.envelope.subject_kind,
                &canonical.validated_envelope.envelope.logical_subject_key,
            )? {
                if existing_run_id != canonical.validated_envelope.envelope.stage_run_id {
                    return Err(invariant(
                        "logical_subject_unreceipted_conflict",
                        format!(
                            "logical_subject_key={} has recoverable run {}; requested run {} must not cross Prepared",
                            canonical
                                .validated_envelope
                                .envelope
                                .logical_subject_key,
                            existing_run_id,
                            canonical.validated_envelope.envelope.stage_run_id
                        ),
                    ));
                }
            }
            if let Some(existing) =
                find_envelope(conn, &canonical.validated_envelope.envelope.stage_run_id)?
            {
                if existing.content_hash == canonical.validated_envelope.content_hash {
                    verify_persisted_envelope(conn, &canonical.validated_envelope)?;
                    return Ok(StageDisposition::ExactReplay);
                }
                return Err(SelectionV2RepositoryError::ReplayConflict {
                    subject_kind: SubjectKind::OutcomeRun.as_str().into(),
                    subject_id: canonical.validated_envelope.envelope.stage_run_id.clone(),
                });
            }
            insert_envelope(
                conn,
                &canonical.validated_envelope.envelope,
                &canonical.validated_envelope.content_hash,
            )?;
            verify_persisted_envelope(conn, &canonical.validated_envelope)?;
            Ok(StageDisposition::Inserted)
        })?;
        Ok(PersistedRecoveryEnvelope {
            disposition,
            envelope: canonical.validated_envelope.envelope,
            content_hash: canonical.validated_envelope.content_hash,
            logical_subject_lock: Some(logical_subject_lock),
        })
    }

    pub(crate) fn load_prepared_proof(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        persisted_envelope: &PersistedRecoveryEnvelope,
        content: PreparedAuditContentPreimage,
    ) -> RepositoryResult<PreparedAuditProof> {
        self.verify(conn)?;
        let logical_subject_lock = persisted_envelope
            .logical_subject_lock
            .as_ref()
            .ok_or_else(|| {
                invariant(
                    "logical_subject_lock_capability_missing",
                    "Prepared proof requires an envelope resolved under the caller-held audit lock",
                )
            })?;
        revalidate_logical_subject_lock(
            conn,
            audit_session,
            persisted_envelope,
            logical_subject_lock,
        )?;
        PreparedAuditProof::load_locked(audit_session, content, logical_subject_lock)
    }

    pub(crate) fn stage_config_activation(
        &self,
        conn: &mut SqliteConnection,
        request: &ConfigActivationStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<StagedRunReceipt> {
        self.verify(conn)?;
        revalidate_prepared_proof(audit_session, prepared_proof)?;
        require_matching_lock_capabilities(persisted_envelope, prepared_proof)?;
        revalidate_logical_subject_lock(
            conn,
            audit_session,
            persisted_envelope,
            &prepared_proof.logical_subject_lock,
        )?;
        let canonical = CanonicalStage::config_activation(
            request,
            persisted_envelope,
            prepared_proof,
            staged_at,
        )?;
        self.require_exact_persisted_envelope(conn, &canonical.validated_envelope)?;
        self.stage(conn, canonical)
    }

    pub(crate) fn stage_source_ingress(
        &self,
        conn: &mut SqliteConnection,
        request: &SourceIngressStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<StagedRunReceipt> {
        self.verify(conn)?;
        revalidate_prepared_proof(audit_session, prepared_proof)?;
        require_matching_lock_capabilities(persisted_envelope, prepared_proof)?;
        revalidate_logical_subject_lock(
            conn,
            audit_session,
            persisted_envelope,
            &prepared_proof.logical_subject_lock,
        )?;
        let canonical =
            CanonicalStage::source_ingress(request, persisted_envelope, prepared_proof, staged_at)?;
        self.require_exact_persisted_envelope(conn, &canonical.validated_envelope)?;
        self.stage(conn, canonical)
    }

    pub(crate) fn stage_generation(
        &self,
        conn: &mut SqliteConnection,
        request: &GenerationStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<StagedRunReceipt> {
        self.verify(conn)?;
        revalidate_prepared_proof(audit_session, prepared_proof)?;
        require_matching_lock_capabilities(persisted_envelope, prepared_proof)?;
        revalidate_logical_subject_lock(
            conn,
            audit_session,
            persisted_envelope,
            &prepared_proof.logical_subject_lock,
        )?;
        let canonical =
            CanonicalStage::generation(request, persisted_envelope, prepared_proof, staged_at)?;
        self.require_exact_persisted_envelope(conn, &canonical.validated_envelope)?;
        self.stage(conn, canonical)
    }

    pub(crate) fn stage_outcome_claim(
        &self,
        conn: &mut SqliteConnection,
        request: &OutcomeClaimStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<StagedRunReceipt> {
        self.verify(conn)?;
        revalidate_prepared_proof(audit_session, prepared_proof)?;
        require_matching_lock_capabilities(persisted_envelope, prepared_proof)?;
        revalidate_logical_subject_lock(
            conn,
            audit_session,
            persisted_envelope,
            &prepared_proof.logical_subject_lock,
        )?;
        let canonical =
            CanonicalStage::outcome_claim(request, persisted_envelope, prepared_proof, staged_at)?;
        self.require_exact_persisted_envelope(conn, &canonical.validated_envelope)?;
        self.stage(conn, canonical)
    }

    pub(crate) fn stage_outcome(
        &self,
        conn: &mut SqliteConnection,
        request: &OutcomeStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<StagedRunReceipt> {
        self.verify(conn)?;
        revalidate_prepared_proof(audit_session, prepared_proof)?;
        require_matching_lock_capabilities(persisted_envelope, prepared_proof)?;
        revalidate_logical_subject_lock(
            conn,
            audit_session,
            persisted_envelope,
            &prepared_proof.logical_subject_lock,
        )?;
        let canonical =
            CanonicalStage::outcome(request, persisted_envelope, prepared_proof, staged_at)?;
        self.require_exact_persisted_envelope(conn, &canonical.validated_envelope)?;
        self.stage(conn, canonical)
    }

    #[allow(
        dead_code,
        reason = "BR-183 keeps config reuse verification dormant until selection-v2 activation"
    )]
    pub(crate) fn config_hash_reuse(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        config_hash: &str,
        expected_manifest_content_hash: &str,
    ) -> RepositoryResult<ConfigHashReuse> {
        self.verify(conn)?;
        audit_session
            .validate()
            .map_err(|error| SelectionV2RepositoryError::Audit(error.to_string()))?;
        let Some(row) = find_config_hash(conn, config_hash)? else {
            return Ok(ConfigHashReuse::Absent);
        };
        let evidence = load_config_activation_evidence(conn, audit_session, row)?;
        if evidence.manifest_content_hash != expected_manifest_content_hash {
            return Ok(ConfigHashReuse::Conflict {
                activation_run_id: evidence.activation_run_id,
                manifest_content_hash: evidence.manifest_content_hash,
            });
        }
        Ok(match evidence.receipt {
            None => ConfigHashReuse::StagedUnreceipted {
                activation_run_id: evidence.activation_run_id,
                manifest_content_hash: evidence.manifest_content_hash,
                prepared_audit_hash: evidence.prepared_audit_hash,
            },
            Some(receipt) => ConfigHashReuse::ReceiptedExact {
                activation_run_id: evidence.activation_run_id,
                manifest_content_hash: evidence.manifest_content_hash,
                receipt_content_hash: receipt.content_hash,
                prepared_audit_hash: evidence.prepared_audit_hash,
                committed_audit_hash: receipt.committed_audit_hash,
            },
        })
    }

    pub(crate) fn verify_staged_readback(
        &self,
        conn: &mut SqliteConnection,
        subject_id: &str,
    ) -> RepositoryResult<StagedRunReceipt> {
        self.verify(conn)?;
        run_immediate_transaction(conn, |conn| {
            verify_staged_readback(conn, subject_id, StageDisposition::ExactReplay)
        })
    }

    pub(crate) fn insert_commit_receipt(
        &self,
        conn: &mut SqliteConnection,
        audit_session: &mut LockedSelectionAuditSession<'_>,
        prepared_proof: &PreparedAuditProof,
        committed_proof: &CommittedAuditProof,
    ) -> RepositoryResult<CommitReceipt> {
        self.verify(conn)?;
        revalidate_prepared_proof(audit_session, prepared_proof)?;
        revalidate_committed_proof(audit_session, committed_proof)?;
        let committed_at = utc_nanos(committed_proof.recorded_at);
        let (disposition, staged, content_hash) = run_immediate_transaction(conn, |conn| {
            let staged = verify_staged_readback(
                conn,
                &committed_proof.content.subject_id,
                StageDisposition::ExactReplay,
            )?;
            validate_receipt_proofs(&staged, prepared_proof, committed_proof)?;
            let content = CommitReceiptContentPreimage {
                domain: DOMAIN_COMMIT_RECEIPT.into(),
                subject_kind: staged.subject_kind,
                subject_id: staged.subject_id.clone(),
                logical_subject_key: staged.logical_subject_key.clone(),
                in_memory_payload_hash: staged.in_memory_payload_hash.clone(),
                recovery_envelope_content_hash: staged.recovery_envelope_content_hash.clone(),
                prepared_audit_hash: prepared_proof.record_hash.clone(),
                run_manifest_content_hash: staged.run_manifest_content_hash.clone(),
                staged_db_content_hash: staged.staged_db_content_hash.clone(),
                committed_audit_hash: committed_proof.record_hash.clone(),
                committed_at_rfc3339_nanos_utc: committed_at.clone(),
            };
            let content_hash = hash(&content)?;
            if let Some(existing) = find_commit_receipt(conn, &staged.subject_id)? {
                if existing.content_hash == content_hash {
                    let rebuilt = rebuild_commit_receipt(&existing)?;
                    if hash(&rebuilt)? != content_hash || rebuilt != content {
                        return Err(invariant(
                            "receipt_readback_rehash_mismatch",
                            "existing receipt columns do not reproduce the exact typed receipt",
                        ));
                    }
                    return Ok((StageDisposition::ExactReplay, staged, content_hash));
                }
                return Err(SelectionV2RepositoryError::ReplayConflict {
                    subject_kind: staged.subject_kind.as_str().into(),
                    subject_id: staged.subject_id.clone(),
                });
            }
            insert_commit_receipt_row(conn, &content, &content_hash)?;
            let readback = find_commit_receipt(conn, &staged.subject_id)?.ok_or_else(|| {
                invariant(
                    "receipt_readback_missing",
                    "receipt disappeared immediately after commit",
                )
            })?;
            let rebuilt = rebuild_commit_receipt(&readback)?;
            if readback.content_hash != content_hash
                || hash(&rebuilt)? != content_hash
                || rebuilt != content
            {
                return Err(invariant(
                    "receipt_readback_rehash_mismatch",
                    "stored receipt columns do not reproduce the exact typed receipt",
                ));
            }
            Ok((StageDisposition::Inserted, staged, content_hash))
        })?;
        revalidate_committed_proof(audit_session, committed_proof)?;
        Ok(CommitReceipt {
            disposition,
            subject_kind: staged.subject_kind,
            subject_id: staged.subject_id,
            content_hash,
            committed_at_rfc3339_nanos_utc: committed_at,
        })
    }

    /// Returns the owner-generated envelope time for a new run, or the exact
    /// already-durable value for same-run recovery.
    pub(crate) fn owner_enveloped_at(
        &self,
        conn: &mut SqliteConnection,
        subject_id: &str,
        proposed: DateTime<Utc>,
    ) -> RepositoryResult<DateTime<Utc>> {
        self.verify(conn)?;
        match find_envelope(conn, subject_id)? {
            Some(row) => parse_canonical_owner_time(&row.enveloped_at, "enveloped_at"),
            None => Ok(proposed),
        }
    }

    /// Returns the owner-generated manifest time for a new run, or the exact
    /// already-durable value for same-run recovery.
    pub(crate) fn owner_staged_at(
        &self,
        conn: &mut SqliteConnection,
        subject_id: &str,
        proposed: DateTime<Utc>,
    ) -> RepositoryResult<DateTime<Utc>> {
        self.verify(conn)?;
        match find_manifest(conn, subject_id)? {
            Some(row) => parse_canonical_owner_time(&row.staged_at, "staged_at"),
            None => Ok(proposed),
        }
    }

    fn stage(
        &self,
        conn: &mut SqliteConnection,
        canonical: CanonicalStage,
    ) -> RepositoryResult<StagedRunReceipt> {
        let disposition = run_immediate_transaction(conn, |conn| {
            if let CanonicalDomainRows::Outcome(stage) = &canonical.domain {
                load_outcome_authority(conn, stage)?;
            }
            if canonical.manifest.subject_kind == SubjectKind::ConfigActivation {
                if let Some(existing) = find_config_hash(
                    conn,
                    canonical
                        .manifest
                        .config_hash
                        .as_deref()
                        .unwrap_or_default(),
                )? {
                    if existing.subject_id != canonical.manifest.subject_id {
                        return Err(SelectionV2RepositoryError::ConfigHashConflict {
                            config_hash: canonical.manifest.config_hash.clone().unwrap_or_default(),
                            existing_run_id: existing.subject_id,
                            requested_run_id: canonical.manifest.subject_id.clone(),
                        });
                    }
                }
            }
            if let Some(existing) = find_manifest(conn, &canonical.manifest.subject_id)? {
                if existing.manifest_content_hash == canonical.manifest_content_hash {
                    return Ok(StageDisposition::ExactReplay);
                }
                return Err(SelectionV2RepositoryError::ReplayConflict {
                    subject_kind: canonical.manifest.subject_kind.as_str().into(),
                    subject_id: canonical.manifest.subject_id.clone(),
                });
            }
            match &canonical.domain {
                CanonicalDomainRows::ConfigActivation => {}
                CanonicalDomainRows::SourceIngress(stage) => insert_ingress_rows(conn, stage)?,
                CanonicalDomainRows::Generation(stage) => insert_generation_rows(conn, stage)?,
                CanonicalDomainRows::OutcomeClaim => {}
                CanonicalDomainRows::Outcome(stage) => insert_outcome_rows(conn, stage)?,
            }
            insert_manifest(conn, &canonical.manifest, &canonical.manifest_content_hash)?;
            Ok(StageDisposition::Inserted)
        })?;
        let mut verified = run_immediate_transaction(conn, |conn| {
            verify_staged_readback(conn, &canonical.manifest.subject_id, disposition)
        })?;
        if verified.run_manifest_content_hash != canonical.manifest_content_hash
            || verified.staged_db_content_hash != canonical.staged_db_content_hash
            || verified.in_memory_payload_hash != canonical.in_memory_payload_hash
        {
            return Err(invariant(
                "stage_readback_differs_from_request",
                "persisted stage does not match canonical typed request",
            ));
        }
        verified.disposition = disposition;
        Ok(verified)
    }
}

fn parse_canonical_owner_time(value: &str, field: &'static str) -> RepositoryResult<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|error| invariant("owner_time_invalid", format!("{field}: {error}")))?
        .with_timezone(&Utc);
    if utc_nanos(parsed) != value {
        return Err(invariant(
            "owner_time_noncanonical",
            format!("{field} must use exact RFC3339 nanoseconds in UTC"),
        ));
    }
    Ok(parsed)
}

fn reconcile_database_and_audit(
    conn: &mut SqliteConnection,
    audit_session: &mut LockedSelectionAuditSession<'_>,
    purpose: ReconciliationPurpose,
) -> RepositoryResult<()> {
    run_immediate_transaction(conn, |conn| {
        let mut reader = DieselExactSelectionSnapshotReader { conn };
        verify_database_and_audit_with_reader(&mut reader, audit_session, purpose).map(|_| ())
    })
}

/// Reconcile every manifest and receipt visible in the caller's already-pinned
/// SQLite snapshot against one complete audit-chain snapshot.
///
/// The caller must begin and pin a read transaction before invoking this
/// helper. It intentionally does not open a nested transaction, which lets the
/// verified read model retain the exact receipt high-water through its query.
pub(super) fn verify_database_and_audit_in_current_snapshot(
    conn: &mut SqliteConnection,
    audit_session: &mut LockedSelectionAuditSession<'_>,
) -> RepositoryResult<crate::selection::audit::ValidatedAuditChainSnapshot> {
    let mut reader = DieselExactSelectionSnapshotReader { conn };
    verify_database_and_audit_with_reader(
        &mut reader,
        audit_session,
        ReconciliationPurpose::AuthoritativeRead,
    )
}

pub(super) fn verify_database_and_audit_for_recovery_in_current_snapshot(
    conn: &mut SqliteConnection,
    audit_session: &mut LockedSelectionAuditSession<'_>,
) -> RepositoryResult<crate::selection::audit::ValidatedAuditChainSnapshot> {
    let mut reader = DieselExactSelectionSnapshotReader { conn };
    verify_database_and_audit_with_reader(
        &mut reader,
        audit_session,
        ReconciliationPurpose::PersistenceRecovery,
    )
}

/// Reconcile a database/audit snapshot using the exact rusqlite transaction
/// already retained by the global schema owner.
///
/// This API intentionally accepts neither a path nor a connection factory. It
/// cannot open a second database and therefore cannot escape the caller's
/// pinned SQLite snapshot.
pub(super) fn verify_database_and_audit_in_rusqlite_snapshot(
    transaction: &rusqlite::Transaction<'_>,
    audit_session: &mut LockedSelectionAuditSession<'_>,
) -> RepositoryResult<crate::selection::audit::ValidatedAuditChainSnapshot> {
    let mut reader = RusqliteExactSelectionSnapshotReader { transaction };
    verify_database_and_audit_with_reader(
        &mut reader,
        audit_session,
        ReconciliationPurpose::AuthoritativeRead,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconciliationPurpose {
    AuthoritativeRead,
    PersistenceRecovery,
}

#[derive(Debug, Clone, Copy)]
enum SubjectIdCollection {
    Manifests,
    Receipts,
}

trait ExactSelectionSnapshotReader {
    fn subject_ids(&mut self, collection: SubjectIdCollection) -> RepositoryResult<Vec<String>>;
    fn find_envelope(&mut self, subject_id: &str) -> RepositoryResult<Option<EnvelopeRow>>;
    fn find_manifest(&mut self, subject_id: &str) -> RepositoryResult<Option<ManifestRow>>;
    fn find_commit_receipt(
        &mut self,
        subject_id: &str,
    ) -> RepositoryResult<Option<CommitReceiptRow>>;
    fn source_batch_attempts(
        &mut self,
        ingress_run_id: &str,
    ) -> RepositoryResult<Vec<SelectionSourceBatchAttemptRowContentPreimage>>;
    fn source_facts(
        &mut self,
        ingress_run_id: &str,
    ) -> RepositoryResult<Vec<SelectionSourceFactRowContentPreimage>>;
    fn source_fact_attempts(
        &mut self,
        ingress_run_id: &str,
    ) -> RepositoryResult<Vec<SelectionSourceFactAttemptRowContentPreimage>>;
    fn generation_rows(
        &mut self,
        generation_run_id: &str,
    ) -> RepositoryResult<GenerationRowsReadback>;
    fn outcome_authority_row(
        &mut self,
        sample_key: &str,
    ) -> RepositoryResult<Option<OutcomeAuthorityRow>>;
    fn outcome_rows(&mut self, outcome_run_id: &str) -> RepositoryResult<OutcomeRowsReadback>;
}

struct DieselExactSelectionSnapshotReader<'connection> {
    conn: &'connection mut SqliteConnection,
}

impl ExactSelectionSnapshotReader for DieselExactSelectionSnapshotReader<'_> {
    fn subject_ids(&mut self, collection: SubjectIdCollection) -> RepositoryResult<Vec<String>> {
        let query = match collection {
            SubjectIdCollection::Manifests => {
                "SELECT subject_id FROM selection_v2_run_stages ORDER BY subject_id ASC"
            }
            SubjectIdCollection::Receipts => {
                "SELECT subject_id FROM selection_v2_commit_receipts ORDER BY subject_id ASC"
            }
        };
        load_subject_ids(self.conn, query)
    }

    fn find_envelope(&mut self, subject_id: &str) -> RepositoryResult<Option<EnvelopeRow>> {
        find_envelope(self.conn, subject_id)
    }

    fn find_manifest(&mut self, subject_id: &str) -> RepositoryResult<Option<ManifestRow>> {
        find_manifest(self.conn, subject_id)
    }

    fn find_commit_receipt(
        &mut self,
        subject_id: &str,
    ) -> RepositoryResult<Option<CommitReceiptRow>> {
        find_commit_receipt(self.conn, subject_id)
    }

    fn source_batch_attempts(
        &mut self,
        ingress_run_id: &str,
    ) -> RepositoryResult<Vec<SelectionSourceBatchAttemptRowContentPreimage>> {
        load_source_batch_attempts(self.conn, ingress_run_id)
    }

    fn source_facts(
        &mut self,
        ingress_run_id: &str,
    ) -> RepositoryResult<Vec<SelectionSourceFactRowContentPreimage>> {
        load_source_facts(self.conn, ingress_run_id)
    }

    fn source_fact_attempts(
        &mut self,
        ingress_run_id: &str,
    ) -> RepositoryResult<Vec<SelectionSourceFactAttemptRowContentPreimage>> {
        load_source_fact_attempts(self.conn, ingress_run_id)
    }

    fn generation_rows(
        &mut self,
        generation_run_id: &str,
    ) -> RepositoryResult<GenerationRowsReadback> {
        load_generation_rows(self.conn, generation_run_id)
    }

    fn outcome_authority_row(
        &mut self,
        sample_key: &str,
    ) -> RepositoryResult<Option<OutcomeAuthorityRow>> {
        query_outcome_authority_row(self.conn, sample_key)
    }

    fn outcome_rows(&mut self, outcome_run_id: &str) -> RepositoryResult<OutcomeRowsReadback> {
        load_outcome_rows(self.conn, outcome_run_id)
    }
}

struct RusqliteExactSelectionSnapshotReader<'borrow, 'connection> {
    transaction: &'borrow rusqlite::Transaction<'connection>,
}

impl ExactSelectionSnapshotReader for RusqliteExactSelectionSnapshotReader<'_, '_> {
    fn subject_ids(&mut self, collection: SubjectIdCollection) -> RepositoryResult<Vec<String>> {
        let query = match collection {
            SubjectIdCollection::Manifests => {
                "SELECT subject_id FROM selection_v2_run_stages ORDER BY subject_id ASC"
            }
            SubjectIdCollection::Receipts => {
                "SELECT subject_id FROM selection_v2_commit_receipts ORDER BY subject_id ASC"
            }
        };
        let mut statement = self.transaction.prepare(query)?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(SelectionV2RepositoryError::from)
    }

    fn find_envelope(&mut self, subject_id: &str) -> RepositoryResult<Option<EnvelopeRow>> {
        use rusqlite::OptionalExtension as _;
        self.transaction
            .query_row(
                "SELECT stage_run_id, subject_kind, logical_subject_key, payload_schema,
                        payload_json, payload_json_hash, in_memory_payload_hash,
                        config_activation_run_id, config_hash, enveloped_at, content_hash
                 FROM selection_v2_recovery_envelopes WHERE stage_run_id=?1",
                [subject_id],
                |row| {
                    Ok(EnvelopeRow {
                        stage_run_id: row.get(0)?,
                        subject_kind: row.get(1)?,
                        logical_subject_key: row.get(2)?,
                        payload_schema: row.get(3)?,
                        payload_json: row.get(4)?,
                        payload_json_hash: row.get(5)?,
                        in_memory_payload_hash: row.get(6)?,
                        config_activation_run_id: row.get(7)?,
                        config_hash: row.get(8)?,
                        enveloped_at: row.get(9)?,
                        content_hash: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(SelectionV2RepositoryError::from)
    }

    fn find_manifest(&mut self, subject_id: &str) -> RepositoryResult<Option<ManifestRow>> {
        use rusqlite::OptionalExtension as _;
        self.transaction
            .query_row(
                "SELECT subject_kind, subject_id, in_memory_payload_hash,
                        prepared_record_hash, expected_staged_row_count,
                        staged_db_content_hash, recovery_envelope_content_hash,
                        logical_subject_key, run_status, source_fact_key,
                        config_activation_run_id, config_hash, config_snapshot_json_hash,
                        config_activation_content_hash, config_activation_file_content_hash,
                        config_effective_from, artifact_valid_from, artifact_expires_at,
                        executable_revision, legacy_cutover_snapshot_hash,
                        generation_market_date, aggregator_observed_at,
                        ingress_source_batch_content_hash, outcome_phase, stored_due_date,
                        outcome_claim_id, planned_outcome_run_id,
                        outcome_claim_receipt_content_hash, outcome_claim_due_binding_hash,
                        outcome_claim_provider_request_hash,
                        staged_at, manifest_content_hash
                 FROM selection_v2_run_stages WHERE subject_id=?1",
                [subject_id],
                |row| {
                    Ok(ManifestRow {
                        subject_kind: row.get(0)?,
                        subject_id: row.get(1)?,
                        in_memory_payload_hash: row.get(2)?,
                        prepared_record_hash: row.get(3)?,
                        expected_staged_row_count: row.get(4)?,
                        staged_db_content_hash: row.get(5)?,
                        recovery_envelope_content_hash: row.get(6)?,
                        logical_subject_key: row.get(7)?,
                        run_status: row.get(8)?,
                        source_fact_key: row.get(9)?,
                        config_activation_run_id: row.get(10)?,
                        config_hash: row.get(11)?,
                        config_snapshot_json_hash: row.get(12)?,
                        config_activation_content_hash: row.get(13)?,
                        config_activation_file_content_hash: row.get(14)?,
                        config_effective_from: row.get(15)?,
                        artifact_valid_from: row.get(16)?,
                        artifact_expires_at: row.get(17)?,
                        executable_revision: row.get(18)?,
                        legacy_cutover_snapshot_hash: row.get(19)?,
                        generation_market_date: row.get(20)?,
                        aggregator_observed_at: row.get(21)?,
                        ingress_source_batch_content_hash: row.get(22)?,
                        outcome_phase: row.get(23)?,
                        stored_due_date: row.get(24)?,
                        outcome_claim_id: row.get(25)?,
                        planned_outcome_run_id: row.get(26)?,
                        outcome_claim_receipt_content_hash: row.get(27)?,
                        outcome_claim_due_binding_hash: row.get(28)?,
                        outcome_claim_provider_request_hash: row.get(29)?,
                        staged_at: row.get(30)?,
                        manifest_content_hash: row.get(31)?,
                    })
                },
            )
            .optional()
            .map_err(SelectionV2RepositoryError::from)
    }

    fn find_commit_receipt(
        &mut self,
        subject_id: &str,
    ) -> RepositoryResult<Option<CommitReceiptRow>> {
        use rusqlite::OptionalExtension as _;
        self.transaction
            .query_row(
                "SELECT subject_kind, subject_id, logical_subject_key,
                        in_memory_payload_hash, recovery_envelope_content_hash,
                        prepared_audit_hash, run_manifest_content_hash,
                        staged_db_content_hash, committed_audit_hash, committed_at,
                        content_hash
                 FROM selection_v2_commit_receipts WHERE subject_id=?1",
                [subject_id],
                |row| {
                    Ok(CommitReceiptRow {
                        subject_kind: row.get(0)?,
                        subject_id: row.get(1)?,
                        logical_subject_key: row.get(2)?,
                        in_memory_payload_hash: row.get(3)?,
                        recovery_envelope_content_hash: row.get(4)?,
                        prepared_audit_hash: row.get(5)?,
                        run_manifest_content_hash: row.get(6)?,
                        staged_db_content_hash: row.get(7)?,
                        committed_audit_hash: row.get(8)?,
                        committed_at: row.get(9)?,
                        content_hash: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(SelectionV2RepositoryError::from)
    }

    fn source_batch_attempts(
        &mut self,
        ingress_run_id: &str,
    ) -> RepositoryResult<Vec<SelectionSourceBatchAttemptRowContentPreimage>> {
        rusqlite_load_source_batch_attempts(self.transaction, ingress_run_id)
    }

    fn source_facts(
        &mut self,
        ingress_run_id: &str,
    ) -> RepositoryResult<Vec<SelectionSourceFactRowContentPreimage>> {
        rusqlite_load_source_facts(self.transaction, ingress_run_id)
    }

    fn source_fact_attempts(
        &mut self,
        ingress_run_id: &str,
    ) -> RepositoryResult<Vec<SelectionSourceFactAttemptRowContentPreimage>> {
        rusqlite_load_source_fact_attempts(self.transaction, ingress_run_id)
    }

    fn generation_rows(
        &mut self,
        generation_run_id: &str,
    ) -> RepositoryResult<GenerationRowsReadback> {
        rusqlite_load_generation_rows(self.transaction, generation_run_id)
    }

    fn outcome_authority_row(
        &mut self,
        sample_key: &str,
    ) -> RepositoryResult<Option<OutcomeAuthorityRow>> {
        rusqlite_query_outcome_authority_row(self.transaction, sample_key)
    }

    fn outcome_rows(&mut self, outcome_run_id: &str) -> RepositoryResult<OutcomeRowsReadback> {
        rusqlite_load_outcome_rows(self.transaction, outcome_run_id)
    }
}

fn verify_database_and_audit_with_reader<R: ExactSelectionSnapshotReader>(
    reader: &mut R,
    audit_session: &mut LockedSelectionAuditSession<'_>,
    purpose: ReconciliationPurpose,
) -> RepositoryResult<crate::selection::audit::ValidatedAuditChainSnapshot> {
    let snapshot = audit_session
        .validated_records()
        .map_err(|error| SelectionV2RepositoryError::Audit(error.to_string()))?;
    let manifest_ids = reader.subject_ids(SubjectIdCollection::Manifests)?;
    let receipt_ids = reader.subject_ids(SubjectIdCollection::Receipts)?;

    for subject_id in &receipt_ids {
        if reader.find_manifest(subject_id)?.is_none() {
            return Err(invariant(
                "authoritative_receipt_manifest_missing",
                format!("receipt {subject_id} has no run manifest"),
            ));
        }
    }
    for subject_id in &manifest_ids {
        reconcile_manifest_and_audit(reader, audit_session, subject_id, purpose)?;
    }
    for record in snapshot
        .records()
        .iter()
        .filter(|record| is_selection_v2_audit_phase(record.phase))
    {
        reconcile_audit_record_and_database(reader, record, purpose)?;
    }
    Ok(snapshot)
}

/// Validate an envelope-only recovery row before exposing it to a recovery
/// executor. Envelope-only rows are intentionally absent from manifest
/// reconciliation, so their strict typed payload must be checked here.
pub(super) fn verify_envelope_only_recovery_row(
    conn: &mut SqliteConnection,
    subject_id: &str,
) -> RepositoryResult<()> {
    let row = find_envelope(conn, subject_id)?.ok_or_else(|| {
        invariant(
            "recovery_envelope_missing",
            format!("recovery envelope {subject_id} disappeared from the pinned snapshot"),
        )
    })?;
    let envelope = rebuild_envelope(&row)?;
    if envelope.domain != DOMAIN_RECOVERY_ENVELOPE_ROW {
        return Err(invariant(
            "recovery_envelope_domain_mismatch",
            format!("recovery envelope {subject_id} has an unknown domain"),
        ));
    }
    let enveloped_at = DateTime::parse_from_rfc3339(&envelope.enveloped_at)
        .map_err(|error| {
            invariant(
                "recovery_envelope_time_invalid",
                format!("recovery envelope {subject_id} time is invalid: {error}"),
            )
        })?
        .with_timezone(&Utc);
    if utc_nanos(enveloped_at) != envelope.enveloped_at {
        return Err(invariant(
            "recovery_envelope_time_noncanonical",
            format!("recovery envelope {subject_id} must use exact UTC nanoseconds and Z"),
        ));
    }
    let rebuilt_hash = hash(&envelope)?;
    if rebuilt_hash != row.content_hash {
        return Err(invariant(
            "recovery_envelope_content_hash_mismatch",
            format!("recovery envelope {subject_id} does not reproduce content_hash"),
        ));
    }
    crate::selection::schema_v2::validate_stage_payload_json(
        envelope.subject_kind,
        &envelope.payload_schema,
        &envelope.payload_json,
        &envelope.payload_json_hash,
    )
    .map_err(|error| {
        SelectionV2RepositoryError::Canonical(format!(
            "stored recovery envelope {subject_id} payload is invalid: {error}"
        ))
    })?;
    let run_payload = match envelope.subject_kind {
        SubjectKind::ConfigActivation => {
            let stage: ConfigActivationStageInputPreimage =
                parse_canonical_payload(&envelope.payload_json)?;
            RunPayloadPreimage {
                domain: DOMAIN_CONFIG_ACTIVATION_PAYLOAD.into(),
                subject_kind: SubjectKind::ConfigActivation,
                subject_id: stage.stage_run_id.clone(),
                logical_subject_key: stage.logical_subject_key,
                source_fact_key: None,
                config_activation_run_id: stage.stage_run_id,
                config_hash: stage.config_hash,
                config_snapshot_json_hash: Some(stage.config_snapshot_json_hash),
                config_activation_content_hash: Some(stage.activation_content_hash),
                config_activation_file_content_hash: Some(
                    stage.activation.activation_file_content_hash,
                ),
                config_effective_from_rfc3339_nanos_utc: Some(
                    stage.activation.effective_from_rfc3339_nanos_utc,
                ),
                artifact_valid_from: Some(stage.activation.artifact_valid_from),
                artifact_expires_at: Some(stage.activation.artifact_expires_at),
                executable_revision: Some(stage.activation.executable_revision),
                legacy_cutover_snapshot_hash: Some(stage.legacy_cutover_snapshot_hash),
                generation_market_date: None,
                aggregator_observed_at_rfc3339_nanos_utc: None,
                ingress_source_batch_content_hash: None,
                outcome_phase: None,
                stored_due_date: None,
                outcome_claim_id: None,
                planned_outcome_run_id: None,
                outcome_claim_receipt_content_hash: None,
                outcome_claim_due_binding_hash: None,
                outcome_claim_provider_request_hash: None,
                rows: Vec::new(),
            }
        }
        SubjectKind::IngressRun => {
            let stage: SourceIngressStageInputPreimage =
                parse_canonical_payload(&envelope.payload_json)?;
            let rows = ingress_run_row_hashes(&stage)?;
            RunPayloadPreimage {
                domain: DOMAIN_INGRESS_PAYLOAD.into(),
                subject_kind: SubjectKind::IngressRun,
                subject_id: stage.stage_run_id,
                logical_subject_key: stage.logical_subject_key,
                source_fact_key: None,
                config_activation_run_id: stage.config_activation_run_id,
                config_hash: stage.config_hash,
                config_snapshot_json_hash: None,
                config_activation_content_hash: None,
                config_activation_file_content_hash: None,
                config_effective_from_rfc3339_nanos_utc: None,
                artifact_valid_from: None,
                artifact_expires_at: None,
                executable_revision: None,
                legacy_cutover_snapshot_hash: None,
                generation_market_date: Some(stage.generation_market_date),
                aggregator_observed_at_rfc3339_nanos_utc: Some(
                    stage.aggregator_observed_at_rfc3339_nanos_utc,
                ),
                ingress_source_batch_content_hash: Some(stage.source_batch_content_hash),
                outcome_phase: None,
                stored_due_date: None,
                outcome_claim_id: None,
                planned_outcome_run_id: None,
                outcome_claim_receipt_content_hash: None,
                outcome_claim_due_binding_hash: None,
                outcome_claim_provider_request_hash: None,
                rows,
            }
        }
        SubjectKind::GenerationRun => {
            let stage: GenerationStageInputPreimage =
                parse_canonical_payload(&envelope.payload_json)?;
            let rows = generation_run_row_hashes(&stage)?;
            RunPayloadPreimage {
                domain: DOMAIN_GENERATION_PAYLOAD.into(),
                subject_kind: SubjectKind::GenerationRun,
                subject_id: stage.stage_run_id,
                logical_subject_key: stage.logical_subject_key,
                source_fact_key: Some(stage.source_fact_key),
                config_activation_run_id: stage.config_activation_run_id,
                config_hash: stage.config_hash,
                config_snapshot_json_hash: None,
                config_activation_content_hash: None,
                config_activation_file_content_hash: None,
                config_effective_from_rfc3339_nanos_utc: None,
                artifact_valid_from: None,
                artifact_expires_at: None,
                executable_revision: None,
                legacy_cutover_snapshot_hash: None,
                generation_market_date: Some(stage.generation_market_date),
                aggregator_observed_at_rfc3339_nanos_utc: None,
                ingress_source_batch_content_hash: None,
                outcome_phase: None,
                stored_due_date: None,
                outcome_claim_id: None,
                planned_outcome_run_id: None,
                outcome_claim_receipt_content_hash: None,
                outcome_claim_due_binding_hash: None,
                outcome_claim_provider_request_hash: None,
                rows,
            }
        }
        SubjectKind::OutcomeClaim => {
            let stage: OutcomeClaimStageInputPreimage =
                parse_canonical_payload(&envelope.payload_json)?;
            RunPayloadPreimage {
                domain: DOMAIN_OUTCOME_CLAIM_PAYLOAD.into(),
                subject_kind: SubjectKind::OutcomeClaim,
                subject_id: stage.stage_run_id.clone(),
                logical_subject_key: stage.logical_subject_key,
                source_fact_key: None,
                config_activation_run_id: stage.config_activation_run_id,
                config_hash: stage.config_hash,
                config_snapshot_json_hash: None,
                config_activation_content_hash: None,
                config_activation_file_content_hash: None,
                config_effective_from_rfc3339_nanos_utc: None,
                artifact_valid_from: None,
                artifact_expires_at: None,
                executable_revision: None,
                legacy_cutover_snapshot_hash: None,
                generation_market_date: None,
                aggregator_observed_at_rfc3339_nanos_utc: None,
                ingress_source_batch_content_hash: None,
                outcome_phase: Some(stage.due_binding.outcome_phase),
                stored_due_date: Some(stage.due_binding.stored_due_date),
                outcome_claim_id: Some(stage.stage_run_id),
                planned_outcome_run_id: Some(stage.planned_outcome_run_id),
                outcome_claim_receipt_content_hash: None,
                outcome_claim_due_binding_hash: Some(stage.due_binding_hash),
                outcome_claim_provider_request_hash: Some(stage.provider_request_hash),
                rows: Vec::new(),
            }
        }
        SubjectKind::OutcomeRun => {
            let stage: OutcomeStageInputPreimage = parse_canonical_payload(&envelope.payload_json)?;
            let rows = outcome_run_row_hashes(&stage)?;
            RunPayloadPreimage {
                domain: DOMAIN_OUTCOME_PAYLOAD.into(),
                subject_kind: SubjectKind::OutcomeRun,
                subject_id: stage.stage_run_id,
                logical_subject_key: stage.logical_subject_key,
                source_fact_key: None,
                config_activation_run_id: stage.config_activation_run_id,
                config_hash: stage.config_hash,
                config_snapshot_json_hash: None,
                config_activation_content_hash: None,
                config_activation_file_content_hash: None,
                config_effective_from_rfc3339_nanos_utc: None,
                artifact_valid_from: None,
                artifact_expires_at: None,
                executable_revision: None,
                legacy_cutover_snapshot_hash: None,
                generation_market_date: None,
                aggregator_observed_at_rfc3339_nanos_utc: None,
                ingress_source_batch_content_hash: None,
                outcome_phase: Some(stage.outcome_phase),
                stored_due_date: Some(stage.stored_due_date),
                outcome_claim_id: Some(stage.outcome_claim_id),
                planned_outcome_run_id: None,
                outcome_claim_receipt_content_hash: Some(stage.outcome_claim_receipt_content_hash),
                outcome_claim_due_binding_hash: Some(stage.outcome_claim_due_binding_hash),
                outcome_claim_provider_request_hash: Some(
                    stage.outcome_claim_provider_request_hash,
                ),
                rows,
            }
        }
    };
    if run_payload.subject_id != envelope.stage_run_id
        || run_payload.subject_kind != envelope.subject_kind
        || run_payload.logical_subject_key != envelope.logical_subject_key
        || run_payload.config_activation_run_id != envelope.config_activation_run_id
        || run_payload.config_hash != envelope.config_hash
        || hash(&run_payload)? != envelope.in_memory_payload_hash
    {
        return Err(invariant(
            "recovery_envelope_payload_identity_mismatch",
            format!(
                "recovery envelope {subject_id} does not bind its exact typed stage identity/hash"
            ),
        ));
    }
    Ok(())
}

fn load_subject_ids(
    conn: &mut SqliteConnection,
    query: &'static str,
) -> RepositoryResult<Vec<String>> {
    Ok(diesel::sql_query(query)
        .load::<SubjectIdRow>(conn)?
        .into_iter()
        .map(|row| row.subject_id)
        .collect())
}

fn reconcile_manifest_and_audit<R: ExactSelectionSnapshotReader>(
    reader: &mut R,
    audit_session: &mut LockedSelectionAuditSession<'_>,
    subject_id: &str,
    purpose: ReconciliationPurpose,
) -> RepositoryResult<()> {
    let staged =
        verify_staged_readback_with_reader(reader, subject_id, StageDisposition::ExactReplay)?;
    let manifest = reader.find_manifest(subject_id)?.ok_or_else(|| {
        invariant(
            "startup_manifest_disappeared",
            format!("manifest {subject_id} disappeared during pinned reconciliation"),
        )
    })?;
    let prepared_content = PreparedAuditContentPreimage {
        domain: DOMAIN_PREPARED_AUDIT.into(),
        subject_kind: staged.subject_kind,
        subject_id: staged.subject_id.clone(),
        logical_subject_key: staged.logical_subject_key.clone(),
        recovery_envelope_content_hash: staged.recovery_envelope_content_hash.clone(),
        in_memory_payload_hash: staged.in_memory_payload_hash.clone(),
    };
    let prepared_record = load_persisted_audit_record(
        audit_session,
        prepared_phase(staged.subject_kind),
        subject_id,
        &hash(&prepared_content)?,
    )?;
    if prepared_record.record_hash != manifest.prepared_record_hash {
        return Err(invariant(
            "startup_manifest_prepared_audit_mismatch",
            format!("manifest {subject_id} does not bind the exact Prepared audit record"),
        ));
    }

    let committed_content = CommittedAuditContentPreimage {
        domain: DOMAIN_COMMITTED_AUDIT.into(),
        subject_kind: staged.subject_kind,
        subject_id: staged.subject_id.clone(),
        logical_subject_key: staged.logical_subject_key.clone(),
        recovery_envelope_content_hash: staged.recovery_envelope_content_hash.clone(),
        prepared_record_hash: prepared_record.record_hash.clone(),
        run_manifest_content_hash: staged.run_manifest_content_hash.clone(),
        staged_db_content_hash: staged.staged_db_content_hash.clone(),
    };
    let committed_content_hash = hash(&committed_content)?;
    let Some(receipt_row) = reader.find_commit_receipt(subject_id)? else {
        return match audit_session
            .lookup_exact(
                committed_phase(staged.subject_kind),
                subject_id,
                &committed_content_hash,
            )
            .map_err(|error| SelectionV2RepositoryError::Audit(error.to_string()))?
        {
            AuditExactLookup::Missing => Ok(()),
            AuditExactLookup::Exact(_) if purpose == ReconciliationPurpose::PersistenceRecovery => {
                Ok(())
            }
            AuditExactLookup::Exact(_) => Err(invariant(
                "startup_committed_audit_without_receipt",
                format!("manifest {subject_id} has Committed audit evidence but no receipt"),
            )),
            AuditExactLookup::ContentConflict { existing_record } => Err(invariant(
                "startup_committed_audit_content_conflict",
                format!(
                    "manifest {subject_id} expected={} existing={}",
                    committed_content_hash, existing_record.content_hash
                ),
            )),
        };
    };
    let receipt = rebuild_commit_receipt(&receipt_row)?;
    if hash(&receipt)? != receipt_row.content_hash
        || receipt.subject_kind != staged.subject_kind
        || receipt.subject_id != staged.subject_id
        || receipt.logical_subject_key != staged.logical_subject_key
        || receipt.in_memory_payload_hash != staged.in_memory_payload_hash
        || receipt.recovery_envelope_content_hash != staged.recovery_envelope_content_hash
        || receipt.prepared_audit_hash != prepared_record.record_hash
        || receipt.run_manifest_content_hash != staged.run_manifest_content_hash
        || receipt.staged_db_content_hash != staged.staged_db_content_hash
    {
        return Err(invariant(
            "startup_receipt_staged_evidence_mismatch",
            format!("receipt {subject_id} does not exactly bind its staged evidence"),
        ));
    }
    let committed_record = load_persisted_audit_record(
        audit_session,
        committed_phase(staged.subject_kind),
        subject_id,
        &committed_content_hash,
    )?;
    if receipt.committed_audit_hash != committed_record.record_hash
        || receipt.committed_at_rfc3339_nanos_utc
            != utc_nanos(committed_record.recorded_at.with_timezone(&Utc))
    {
        return Err(invariant(
            "startup_receipt_committed_audit_mismatch",
            format!("receipt {subject_id} does not bind the exact Committed record and time"),
        ));
    }
    Ok(())
}

fn reconcile_audit_record_and_database<R: ExactSelectionSnapshotReader>(
    reader: &mut R,
    record: &SelectionAuditRecord,
    purpose: ReconciliationPurpose,
) -> RepositoryResult<()> {
    if let Some(subject_kind) = prepared_subject_kind(record.phase) {
        let envelope_row = reader.find_envelope(&record.subject_id)?.ok_or_else(|| {
            invariant(
                "startup_prepared_audit_envelope_missing",
                format!(
                    "Prepared audit record {:?}/{} has no recovery envelope",
                    record.phase, record.subject_id
                ),
            )
        })?;
        let envelope = rebuild_envelope(&envelope_row)?;
        let envelope_content_hash = hash(&envelope)?;
        let content = PreparedAuditContentPreimage {
            domain: DOMAIN_PREPARED_AUDIT.into(),
            subject_kind,
            subject_id: record.subject_id.clone(),
            logical_subject_key: envelope.logical_subject_key,
            recovery_envelope_content_hash: envelope_content_hash,
            in_memory_payload_hash: envelope.in_memory_payload_hash,
        };
        if envelope.subject_kind != subject_kind || hash(&content)? != record.content_hash {
            return Err(invariant(
                "startup_prepared_audit_database_mismatch",
                format!(
                    "Prepared audit record {:?}/{} differs from its recovery envelope",
                    record.phase, record.subject_id
                ),
            ));
        }
        if let Some(manifest) = reader.find_manifest(&record.subject_id)? {
            if manifest.prepared_record_hash != record.record_hash {
                return Err(invariant(
                    "startup_prepared_audit_manifest_mismatch",
                    format!(
                        "Prepared audit record {:?}/{} is not the manifest high-water",
                        record.phase, record.subject_id
                    ),
                ));
            }
        }
        return Ok(());
    }

    if let Some(subject_kind) = committed_subject_kind(record.phase) {
        let staged = verify_staged_readback_with_reader(
            reader,
            &record.subject_id,
            StageDisposition::ExactReplay,
        )?;
        let manifest = reader.find_manifest(&record.subject_id)?.ok_or_else(|| {
            invariant(
                "startup_committed_audit_manifest_missing",
                format!(
                    "Committed audit record {:?}/{} has no manifest",
                    record.phase, record.subject_id
                ),
            )
        })?;
        let receipt = reader.find_commit_receipt(&record.subject_id)?;
        let content = CommittedAuditContentPreimage {
            domain: DOMAIN_COMMITTED_AUDIT.into(),
            subject_kind,
            subject_id: record.subject_id.clone(),
            logical_subject_key: staged.logical_subject_key,
            recovery_envelope_content_hash: staged.recovery_envelope_content_hash,
            prepared_record_hash: manifest.prepared_record_hash,
            run_manifest_content_hash: staged.run_manifest_content_hash,
            staged_db_content_hash: staged.staged_db_content_hash,
        };
        if staged.subject_kind != subject_kind || hash(&content)? != record.content_hash {
            return Err(invariant(
                "startup_committed_audit_database_mismatch",
                format!(
                    "Committed audit record {:?}/{} differs from staged manifest evidence",
                    record.phase, record.subject_id
                ),
            ));
        }
        match receipt {
            Some(receipt)
                if receipt.committed_audit_hash == record.record_hash
                    && receipt.committed_at
                        == utc_nanos(record.recorded_at.with_timezone(&Utc)) => {}
            Some(_) => {
                return Err(invariant(
                    "startup_committed_audit_database_mismatch",
                    format!(
                        "Committed audit record {:?}/{} differs from receipt evidence",
                        record.phase, record.subject_id
                    ),
                ));
            }
            None if purpose == ReconciliationPurpose::PersistenceRecovery => {}
            None => {
                return Err(invariant(
                    "startup_committed_audit_receipt_missing",
                    format!(
                        "Committed audit record {:?}/{} has no receipt",
                        record.phase, record.subject_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn prepared_subject_kind(phase: SelectionAuditPhase) -> Option<SubjectKind> {
    match phase {
        SelectionAuditPhase::V2ConfigActivationPrepared => Some(SubjectKind::ConfigActivation),
        SelectionAuditPhase::V2IngressPrepared => Some(SubjectKind::IngressRun),
        SelectionAuditPhase::V2GenerationPrepared => Some(SubjectKind::GenerationRun),
        SelectionAuditPhase::V2OutcomeClaimPrepared => Some(SubjectKind::OutcomeClaim),
        SelectionAuditPhase::V2OutcomePrepared => Some(SubjectKind::OutcomeRun),
        _ => None,
    }
}

fn committed_subject_kind(phase: SelectionAuditPhase) -> Option<SubjectKind> {
    match phase {
        SelectionAuditPhase::V2ConfigActivationCommitted => Some(SubjectKind::ConfigActivation),
        SelectionAuditPhase::V2IngressCommitted => Some(SubjectKind::IngressRun),
        SelectionAuditPhase::V2GenerationCommitted => Some(SubjectKind::GenerationRun),
        SelectionAuditPhase::V2OutcomeClaimCommitted => Some(SubjectKind::OutcomeClaim),
        SelectionAuditPhase::V2OutcomeCommitted => Some(SubjectKind::OutcomeRun),
        _ => None,
    }
}

fn is_selection_v2_audit_phase(phase: SelectionAuditPhase) -> bool {
    matches!(
        phase,
        SelectionAuditPhase::V2ConfigActivationPrepared
            | SelectionAuditPhase::V2ConfigActivationCommitted
            | SelectionAuditPhase::V2IngressPrepared
            | SelectionAuditPhase::V2IngressCommitted
            | SelectionAuditPhase::V2GenerationPrepared
            | SelectionAuditPhase::V2GenerationCommitted
            | SelectionAuditPhase::V2OutcomeClaimPrepared
            | SelectionAuditPhase::V2OutcomeClaimCommitted
            | SelectionAuditPhase::V2OutcomePrepared
            | SelectionAuditPhase::V2OutcomeCommitted
    )
}

enum ImmediateTransactionError {
    Primary(SelectionV2RepositoryError),
    Diesel(diesel::result::Error),
}

impl From<diesel::result::Error> for ImmediateTransactionError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Diesel(error)
    }
}

fn run_immediate_transaction<T, F>(conn: &mut SqliteConnection, operation: F) -> RepositoryResult<T>
where
    F: FnOnce(&mut SqliteConnection) -> RepositoryResult<T>,
{
    let mut primary_failure = None;
    match conn.immediate_transaction::<T, ImmediateTransactionError, _>(|conn| {
        match operation(conn) {
            Ok(value) => Ok(value),
            Err(error) => {
                primary_failure = Some(error.to_string());
                Err(ImmediateTransactionError::Primary(error))
            }
        }
    }) {
        Ok(value) => Ok(value),
        Err(ImmediateTransactionError::Primary(error)) => Err(error),
        Err(ImmediateTransactionError::Diesel(error)) => {
            if let Some(primary) = primary_failure {
                Err(SelectionV2RepositoryError::RollbackFailed {
                    primary,
                    rollback: error.to_string(),
                })
            } else {
                Err(SelectionV2RepositoryError::Database(error))
            }
        }
    }
}

#[derive(Debug, Clone)]
enum CanonicalDomainRows {
    ConfigActivation,
    SourceIngress(Box<SourceIngressStageInputPreimage>),
    Generation(Box<GenerationStageInputPreimage>),
    OutcomeClaim,
    Outcome(Box<OutcomeStageInputPreimage>),
}

#[derive(Debug, Clone)]
struct CanonicalEnvelopeInputs {
    validated_envelope: ValidatedRecoveryEnvelope,
    run_payload: RunPayloadPreimage,
    in_memory_payload_hash: String,
    domain_row_hashes: Vec<String>,
}

#[derive(Debug, Clone)]
struct CanonicalStage {
    validated_envelope: ValidatedRecoveryEnvelope,
    domain: CanonicalDomainRows,
    in_memory_payload_hash: String,
    staged_db_content_hash: String,
    manifest: RunManifestContentPreimage,
    manifest_content_hash: String,
}

fn config_activation_run_payload(stage: &ConfigActivationStageInputPreimage) -> RunPayloadPreimage {
    RunPayloadPreimage {
        domain: DOMAIN_CONFIG_ACTIVATION_PAYLOAD.into(),
        subject_kind: SubjectKind::ConfigActivation,
        subject_id: stage.stage_run_id.clone(),
        logical_subject_key: stage.logical_subject_key.clone(),
        source_fact_key: None,
        config_activation_run_id: stage.stage_run_id.clone(),
        config_hash: stage.config_hash.clone(),
        config_snapshot_json_hash: Some(stage.config_snapshot_json_hash.clone()),
        config_activation_content_hash: Some(stage.activation_content_hash.clone()),
        config_activation_file_content_hash: Some(
            stage.activation.activation_file_content_hash.clone(),
        ),
        config_effective_from_rfc3339_nanos_utc: Some(
            stage.activation.effective_from_rfc3339_nanos_utc.clone(),
        ),
        artifact_valid_from: Some(stage.activation.artifact_valid_from.clone()),
        artifact_expires_at: Some(stage.activation.artifact_expires_at.clone()),
        executable_revision: Some(stage.activation.executable_revision.clone()),
        legacy_cutover_snapshot_hash: Some(stage.legacy_cutover_snapshot_hash.clone()),
        generation_market_date: None,
        aggregator_observed_at_rfc3339_nanos_utc: None,
        ingress_source_batch_content_hash: None,
        outcome_phase: None,
        stored_due_date: None,
        outcome_claim_id: None,
        planned_outcome_run_id: None,
        outcome_claim_receipt_content_hash: None,
        outcome_claim_due_binding_hash: None,
        outcome_claim_provider_request_hash: None,
        rows: Vec::new(),
    }
}

fn source_ingress_run_payload(
    stage: &SourceIngressStageInputPreimage,
) -> RepositoryResult<RunPayloadPreimage> {
    Ok(RunPayloadPreimage {
        domain: DOMAIN_INGRESS_PAYLOAD.into(),
        subject_kind: SubjectKind::IngressRun,
        subject_id: stage.stage_run_id.clone(),
        logical_subject_key: stage.logical_subject_key.clone(),
        source_fact_key: None,
        config_activation_run_id: stage.config_activation_run_id.clone(),
        config_hash: stage.config_hash.clone(),
        config_snapshot_json_hash: None,
        config_activation_content_hash: None,
        config_activation_file_content_hash: None,
        config_effective_from_rfc3339_nanos_utc: None,
        artifact_valid_from: None,
        artifact_expires_at: None,
        executable_revision: None,
        legacy_cutover_snapshot_hash: None,
        generation_market_date: Some(stage.generation_market_date.clone()),
        aggregator_observed_at_rfc3339_nanos_utc: Some(
            stage.aggregator_observed_at_rfc3339_nanos_utc.clone(),
        ),
        ingress_source_batch_content_hash: Some(stage.source_batch_content_hash.clone()),
        outcome_phase: None,
        stored_due_date: None,
        outcome_claim_id: None,
        planned_outcome_run_id: None,
        outcome_claim_receipt_content_hash: None,
        outcome_claim_due_binding_hash: None,
        outcome_claim_provider_request_hash: None,
        rows: ingress_run_row_hashes(stage)?,
    })
}

fn generation_run_payload(
    stage: &GenerationStageInputPreimage,
) -> RepositoryResult<RunPayloadPreimage> {
    Ok(RunPayloadPreimage {
        domain: DOMAIN_GENERATION_PAYLOAD.into(),
        subject_kind: SubjectKind::GenerationRun,
        subject_id: stage.stage_run_id.clone(),
        logical_subject_key: stage.logical_subject_key.clone(),
        source_fact_key: Some(stage.source_fact_key.clone()),
        config_activation_run_id: stage.config_activation_run_id.clone(),
        config_hash: stage.config_hash.clone(),
        config_snapshot_json_hash: None,
        config_activation_content_hash: None,
        config_activation_file_content_hash: None,
        config_effective_from_rfc3339_nanos_utc: None,
        artifact_valid_from: None,
        artifact_expires_at: None,
        executable_revision: None,
        legacy_cutover_snapshot_hash: None,
        generation_market_date: Some(stage.generation_market_date.clone()),
        aggregator_observed_at_rfc3339_nanos_utc: None,
        ingress_source_batch_content_hash: None,
        outcome_phase: None,
        stored_due_date: None,
        outcome_claim_id: None,
        planned_outcome_run_id: None,
        outcome_claim_receipt_content_hash: None,
        outcome_claim_due_binding_hash: None,
        outcome_claim_provider_request_hash: None,
        rows: generation_run_row_hashes(stage)?,
    })
}

fn canonical_config_activation_envelope(
    request: &ConfigActivationStageRequest,
) -> RepositoryResult<CanonicalEnvelopeInputs> {
    let stage = &request.stage_input;
    stage.validate().map_err(|error| {
        SelectionV2RepositoryError::Canonical(format!("config activation stage invalid: {error}"))
    })?;
    let payload_json = canonical(stage)?;
    let run_payload = config_activation_run_payload(stage);
    if request.run_payload != run_payload {
        return Err(invariant(
            "config_run_payload_mismatch",
            "caller run payload differs from reconstruction",
        ));
    }
    let in_memory_payload_hash = hash(&run_payload)?;
    let envelope = rebuild_expected_envelope(
        &request.recovery_envelope,
        SubjectKind::ConfigActivation,
        CONFIG_ACTIVATION_PAYLOAD_SCHEMA,
        &stage.stage_run_id,
        &stage.logical_subject_key,
        &stage.stage_run_id,
        &stage.config_hash,
        &payload_json,
        &in_memory_payload_hash,
    )?;
    let content_hash = hash(&envelope)?;
    Ok(CanonicalEnvelopeInputs {
        validated_envelope: ValidatedRecoveryEnvelope {
            envelope,
            content_hash,
        },
        run_payload,
        in_memory_payload_hash,
        domain_row_hashes: Vec::new(),
    })
}

fn canonical_source_ingress_envelope(
    request: &SourceIngressStageRequest,
) -> RepositoryResult<CanonicalEnvelopeInputs> {
    let stage = &request.stage_input;
    stage.validate().map_err(|error| {
        SelectionV2RepositoryError::Canonical(format!("source ingress stage invalid: {error}"))
    })?;
    let run_payload = source_ingress_run_payload(stage)?;
    let domain_row_hashes = run_payload.rows.clone();
    if request.run_payload != run_payload {
        return Err(invariant(
            "ingress_run_payload_mismatch",
            "caller run payload differs from typed row reconstruction",
        ));
    }
    let payload_json = canonical(stage)?;
    let in_memory_payload_hash = hash(&run_payload)?;
    let envelope = rebuild_expected_envelope(
        &request.recovery_envelope,
        SubjectKind::IngressRun,
        SOURCE_INGRESS_PAYLOAD_SCHEMA,
        &stage.stage_run_id,
        &stage.logical_subject_key,
        &stage.config_activation_run_id,
        &stage.config_hash,
        &payload_json,
        &in_memory_payload_hash,
    )?;
    let content_hash = hash(&envelope)?;
    Ok(CanonicalEnvelopeInputs {
        validated_envelope: ValidatedRecoveryEnvelope {
            envelope,
            content_hash,
        },
        run_payload,
        in_memory_payload_hash,
        domain_row_hashes,
    })
}

fn canonical_generation_envelope(
    request: &GenerationStageRequest,
) -> RepositoryResult<CanonicalEnvelopeInputs> {
    let stage = &request.stage_input;
    stage.validate().map_err(|error| {
        SelectionV2RepositoryError::Canonical(format!("generation stage invalid: {error}"))
    })?;
    let run_payload = generation_run_payload(stage)?;
    let domain_row_hashes = run_payload.rows.clone();
    if request.run_payload != run_payload {
        return Err(invariant(
            "generation_run_payload_mismatch",
            "caller run payload differs from typed generation row reconstruction",
        ));
    }
    let payload_json = canonical(stage)?;
    let in_memory_payload_hash = hash(&run_payload)?;
    let envelope = rebuild_expected_envelope(
        &request.recovery_envelope,
        SubjectKind::GenerationRun,
        GENERATION_PAYLOAD_SCHEMA,
        &stage.stage_run_id,
        &stage.logical_subject_key,
        &stage.config_activation_run_id,
        &stage.config_hash,
        &payload_json,
        &in_memory_payload_hash,
    )?;
    let content_hash = hash(&envelope)?;
    Ok(CanonicalEnvelopeInputs {
        validated_envelope: ValidatedRecoveryEnvelope {
            envelope,
            content_hash,
        },
        run_payload,
        in_memory_payload_hash,
        domain_row_hashes,
    })
}

fn outcome_claim_run_payload(stage: &OutcomeClaimStageInputPreimage) -> RunPayloadPreimage {
    RunPayloadPreimage {
        domain: DOMAIN_OUTCOME_CLAIM_PAYLOAD.into(),
        subject_kind: SubjectKind::OutcomeClaim,
        subject_id: stage.stage_run_id.clone(),
        logical_subject_key: stage.logical_subject_key.clone(),
        source_fact_key: None,
        config_activation_run_id: stage.config_activation_run_id.clone(),
        config_hash: stage.config_hash.clone(),
        config_snapshot_json_hash: None,
        config_activation_content_hash: None,
        config_activation_file_content_hash: None,
        config_effective_from_rfc3339_nanos_utc: None,
        artifact_valid_from: None,
        artifact_expires_at: None,
        executable_revision: None,
        legacy_cutover_snapshot_hash: None,
        generation_market_date: None,
        aggregator_observed_at_rfc3339_nanos_utc: None,
        ingress_source_batch_content_hash: None,
        outcome_phase: Some(stage.due_binding.outcome_phase),
        stored_due_date: Some(stage.due_binding.stored_due_date.clone()),
        outcome_claim_id: Some(stage.stage_run_id.clone()),
        planned_outcome_run_id: Some(stage.planned_outcome_run_id.clone()),
        outcome_claim_receipt_content_hash: None,
        outcome_claim_due_binding_hash: Some(stage.due_binding_hash.clone()),
        outcome_claim_provider_request_hash: Some(stage.provider_request_hash.clone()),
        rows: Vec::new(),
    }
}

fn canonical_outcome_claim_envelope(
    request: &OutcomeClaimStageRequest,
) -> RepositoryResult<CanonicalEnvelopeInputs> {
    let stage = &request.stage_input;
    stage.validate().map_err(|error| {
        SelectionV2RepositoryError::Canonical(format!("outcome claim stage invalid: {error}"))
    })?;
    let run_payload = outcome_claim_run_payload(stage);
    if request.run_payload != run_payload {
        return Err(invariant(
            "outcome_claim_run_payload_mismatch",
            "caller run payload differs from typed outcome claim reconstruction",
        ));
    }
    let payload_json = canonical(stage)?;
    let in_memory_payload_hash = hash(&run_payload)?;
    let envelope = rebuild_expected_envelope(
        &request.recovery_envelope,
        SubjectKind::OutcomeClaim,
        OUTCOME_CLAIM_PAYLOAD_SCHEMA,
        &stage.stage_run_id,
        &stage.logical_subject_key,
        &stage.config_activation_run_id,
        &stage.config_hash,
        &payload_json,
        &in_memory_payload_hash,
    )?;
    let content_hash = hash(&envelope)?;
    Ok(CanonicalEnvelopeInputs {
        validated_envelope: ValidatedRecoveryEnvelope {
            envelope,
            content_hash,
        },
        run_payload,
        in_memory_payload_hash,
        domain_row_hashes: Vec::new(),
    })
}

fn canonical_outcome_envelope(
    request: &OutcomeStageRequest,
) -> RepositoryResult<CanonicalEnvelopeInputs> {
    let stage = &request.stage_input;
    stage.validate().map_err(|error| {
        SelectionV2RepositoryError::Canonical(format!("outcome stage invalid: {error}"))
    })?;
    let domain_row_hashes = outcome_run_row_hashes(stage)?;
    let run_payload = RunPayloadPreimage {
        domain: DOMAIN_OUTCOME_PAYLOAD.into(),
        subject_kind: SubjectKind::OutcomeRun,
        subject_id: stage.stage_run_id.clone(),
        logical_subject_key: stage.logical_subject_key.clone(),
        source_fact_key: None,
        config_activation_run_id: stage.config_activation_run_id.clone(),
        config_hash: stage.config_hash.clone(),
        config_snapshot_json_hash: None,
        config_activation_content_hash: None,
        config_activation_file_content_hash: None,
        config_effective_from_rfc3339_nanos_utc: None,
        artifact_valid_from: None,
        artifact_expires_at: None,
        executable_revision: None,
        legacy_cutover_snapshot_hash: None,
        generation_market_date: None,
        aggregator_observed_at_rfc3339_nanos_utc: None,
        ingress_source_batch_content_hash: None,
        outcome_phase: Some(stage.outcome_phase),
        stored_due_date: Some(stage.stored_due_date.clone()),
        outcome_claim_id: Some(stage.outcome_claim_id.clone()),
        planned_outcome_run_id: None,
        outcome_claim_receipt_content_hash: Some(stage.outcome_claim_receipt_content_hash.clone()),
        outcome_claim_due_binding_hash: Some(stage.outcome_claim_due_binding_hash.clone()),
        outcome_claim_provider_request_hash: Some(
            stage.outcome_claim_provider_request_hash.clone(),
        ),
        rows: domain_row_hashes.clone(),
    };
    if request.run_payload != run_payload {
        return Err(invariant(
            "outcome_run_payload_mismatch",
            "caller run payload differs from typed outcome row reconstruction",
        ));
    }
    let payload_json = canonical(stage)?;
    let in_memory_payload_hash = hash(&run_payload)?;
    let envelope = rebuild_expected_envelope(
        &request.recovery_envelope,
        SubjectKind::OutcomeRun,
        OUTCOME_PAYLOAD_SCHEMA,
        &stage.stage_run_id,
        &stage.logical_subject_key,
        &stage.config_activation_run_id,
        &stage.config_hash,
        &payload_json,
        &in_memory_payload_hash,
    )?;
    let content_hash = hash(&envelope)?;
    Ok(CanonicalEnvelopeInputs {
        validated_envelope: ValidatedRecoveryEnvelope {
            envelope,
            content_hash,
        },
        run_payload,
        in_memory_payload_hash,
        domain_row_hashes,
    })
}

impl CanonicalStage {
    fn config_activation(
        request: &ConfigActivationStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        let stage = &request.stage_input;
        let canonical = canonical_config_activation_envelope(request)?;
        require_persisted_capability(persisted_envelope, &canonical.validated_envelope)?;
        require_prepared_proof(
            prepared_proof,
            SubjectKind::ConfigActivation,
            &stage.stage_run_id,
            &stage.logical_subject_key,
            &canonical.validated_envelope.content_hash,
            &canonical.in_memory_payload_hash,
        )?;
        let staged_rows = staged_row_hashes(
            &canonical.validated_envelope.envelope,
            &canonical.validated_envelope.content_hash,
            &canonical.domain_row_hashes,
        )?;
        let expected_count = u32::try_from(staged_rows.len()).map_err(|_| {
            invariant(
                "staged_row_count_overflow",
                "staged row count does not fit u32",
            )
        })?;
        let staged_db = StagedDbPreimage {
            domain: DOMAIN_STAGED_DB.into(),
            subject_kind: SubjectKind::ConfigActivation,
            subject_id: stage.stage_run_id.clone(),
            expected_staged_row_count: expected_count,
            rows: staged_rows,
        };
        let staged_db_content_hash = hash(&staged_db)?;
        let manifest = manifest_from_run_payload(
            &canonical.run_payload,
            &prepared_proof.record_hash,
            expected_count,
            &staged_db_content_hash,
            &canonical.validated_envelope.content_hash,
            RunStatus::Activated,
            utc_nanos(staged_at),
        )?;
        let manifest_content_hash = hash(&manifest)?;
        Ok(Self {
            validated_envelope: canonical.validated_envelope,
            domain: CanonicalDomainRows::ConfigActivation,
            in_memory_payload_hash: canonical.in_memory_payload_hash,
            staged_db_content_hash,
            manifest,
            manifest_content_hash,
        })
    }

    fn source_ingress(
        request: &SourceIngressStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        let stage = &request.stage_input;
        let canonical = canonical_source_ingress_envelope(request)?;
        require_persisted_capability(persisted_envelope, &canonical.validated_envelope)?;
        require_prepared_proof(
            prepared_proof,
            SubjectKind::IngressRun,
            &stage.stage_run_id,
            &stage.logical_subject_key,
            &canonical.validated_envelope.content_hash,
            &canonical.in_memory_payload_hash,
        )?;
        let staged_rows = staged_row_hashes(
            &canonical.validated_envelope.envelope,
            &canonical.validated_envelope.content_hash,
            &canonical.domain_row_hashes,
        )?;
        let expected_count = u32::try_from(staged_rows.len()).map_err(|_| {
            invariant(
                "staged_row_count_overflow",
                "staged row count does not fit u32",
            )
        })?;
        let staged_db = StagedDbPreimage {
            domain: DOMAIN_STAGED_DB.into(),
            subject_kind: SubjectKind::IngressRun,
            subject_id: stage.stage_run_id.clone(),
            expected_staged_row_count: expected_count,
            rows: staged_rows,
        };
        let staged_db_content_hash = hash(&staged_db)?;
        let manifest = manifest_from_run_payload(
            &canonical.run_payload,
            &prepared_proof.record_hash,
            expected_count,
            &staged_db_content_hash,
            &canonical.validated_envelope.content_hash,
            stage.planned_run_status,
            utc_nanos(staged_at),
        )?;
        let manifest_content_hash = hash(&manifest)?;
        Ok(Self {
            validated_envelope: canonical.validated_envelope,
            domain: CanonicalDomainRows::SourceIngress(Box::new(stage.clone())),
            in_memory_payload_hash: canonical.in_memory_payload_hash,
            staged_db_content_hash,
            manifest,
            manifest_content_hash,
        })
    }

    fn generation(
        request: &GenerationStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        let stage = &request.stage_input;
        let canonical = canonical_generation_envelope(request)?;
        require_persisted_capability(persisted_envelope, &canonical.validated_envelope)?;
        require_prepared_proof(
            prepared_proof,
            SubjectKind::GenerationRun,
            &stage.stage_run_id,
            &stage.logical_subject_key,
            &canonical.validated_envelope.content_hash,
            &canonical.in_memory_payload_hash,
        )?;
        Self::from_canonical_envelope(
            canonical,
            CanonicalDomainRows::Generation(Box::new(stage.clone())),
            prepared_proof,
            stage.planned_run_status,
            staged_at,
        )
    }

    fn outcome_claim(
        request: &OutcomeClaimStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        let stage = &request.stage_input;
        let canonical = canonical_outcome_claim_envelope(request)?;
        require_persisted_capability(persisted_envelope, &canonical.validated_envelope)?;
        require_prepared_proof(
            prepared_proof,
            SubjectKind::OutcomeClaim,
            &stage.stage_run_id,
            &stage.logical_subject_key,
            &canonical.validated_envelope.content_hash,
            &canonical.in_memory_payload_hash,
        )?;
        Self::from_canonical_envelope(
            canonical,
            CanonicalDomainRows::OutcomeClaim,
            prepared_proof,
            RunStatus::Claimed,
            staged_at,
        )
    }

    fn outcome(
        request: &OutcomeStageRequest,
        persisted_envelope: &PersistedRecoveryEnvelope,
        prepared_proof: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        let stage = &request.stage_input;
        let canonical = canonical_outcome_envelope(request)?;
        require_persisted_capability(persisted_envelope, &canonical.validated_envelope)?;
        require_prepared_proof(
            prepared_proof,
            SubjectKind::OutcomeRun,
            &stage.stage_run_id,
            &stage.logical_subject_key,
            &canonical.validated_envelope.content_hash,
            &canonical.in_memory_payload_hash,
        )?;
        Self::from_canonical_envelope(
            canonical,
            CanonicalDomainRows::Outcome(Box::new(stage.clone())),
            prepared_proof,
            stage.planned_run_status,
            staged_at,
        )
    }

    fn from_canonical_envelope(
        canonical: CanonicalEnvelopeInputs,
        domain: CanonicalDomainRows,
        prepared_proof: &PreparedAuditProof,
        run_status: RunStatus,
        staged_at: DateTime<Utc>,
    ) -> RepositoryResult<Self> {
        let staged_rows = staged_row_hashes(
            &canonical.validated_envelope.envelope,
            &canonical.validated_envelope.content_hash,
            &canonical.domain_row_hashes,
        )?;
        let expected_count = u32::try_from(staged_rows.len()).map_err(|_| {
            invariant(
                "staged_row_count_overflow",
                "staged row count does not fit u32",
            )
        })?;
        let staged_db = StagedDbPreimage {
            domain: DOMAIN_STAGED_DB.into(),
            subject_kind: canonical.run_payload.subject_kind,
            subject_id: canonical.run_payload.subject_id.clone(),
            expected_staged_row_count: expected_count,
            rows: staged_rows,
        };
        let staged_db_content_hash = hash(&staged_db)?;
        let manifest = manifest_from_run_payload(
            &canonical.run_payload,
            &prepared_proof.record_hash,
            expected_count,
            &staged_db_content_hash,
            &canonical.validated_envelope.content_hash,
            run_status,
            utc_nanos(staged_at),
        )?;
        let manifest_content_hash = hash(&manifest)?;
        Ok(Self {
            validated_envelope: canonical.validated_envelope,
            domain,
            in_memory_payload_hash: canonical.in_memory_payload_hash,
            staged_db_content_hash,
            manifest,
            manifest_content_hash,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild_expected_envelope(
    supplied: &SelectionRecoveryEnvelopeRowContentPreimage,
    subject_kind: SubjectKind,
    payload_schema: &str,
    stage_run_id: &str,
    logical_subject_key: &str,
    config_activation_run_id: &str,
    config_hash: &str,
    payload_json: &str,
    in_memory_payload_hash: &str,
) -> RepositoryResult<SelectionRecoveryEnvelopeRowContentPreimage> {
    let expected = SelectionRecoveryEnvelopeRowContentPreimage {
        domain: DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
        stage_run_id: stage_run_id.into(),
        subject_kind,
        logical_subject_key: logical_subject_key.into(),
        payload_schema: payload_schema.into(),
        payload_json: payload_json.into(),
        payload_json_hash: crate::selection::schema_v2::sha256_bytes(payload_json.as_bytes()),
        in_memory_payload_hash: in_memory_payload_hash.into(),
        config_activation_run_id: config_activation_run_id.into(),
        config_hash: config_hash.into(),
        enveloped_at: supplied.enveloped_at.clone(),
    };
    if supplied != &expected {
        return Err(invariant(
            "recovery_envelope_mismatch",
            "supplied recovery envelope differs from typed reconstruction",
        ));
    }
    Ok(expected)
}

fn require_prepared_proof(
    proof: &PreparedAuditProof,
    subject_kind: SubjectKind,
    subject_id: &str,
    logical_subject_key: &str,
    envelope_hash: &str,
    in_memory_payload_hash: &str,
) -> RepositoryResult<()> {
    let expected = PreparedAuditContentPreimage {
        domain: DOMAIN_PREPARED_AUDIT.into(),
        subject_kind,
        subject_id: subject_id.into(),
        logical_subject_key: logical_subject_key.into(),
        recovery_envelope_content_hash: envelope_hash.into(),
        in_memory_payload_hash: in_memory_payload_hash.into(),
    };
    if proof.content != expected || proof.content_hash != hash(&expected)? {
        return Err(invariant(
            "prepared_audit_proof_mismatch",
            "Prepared audit proof does not bind this exact stage",
        ));
    }
    Ok(())
}

fn require_persisted_capability(
    persisted: &PersistedRecoveryEnvelope,
    validated: &ValidatedRecoveryEnvelope,
) -> RepositoryResult<()> {
    if persisted.envelope != validated.envelope
        || persisted.content_hash != validated.content_hash
        || hash(&persisted.envelope)? != persisted.content_hash
    {
        return Err(invariant(
            "persisted_envelope_capability_mismatch",
            "stage capability does not bind the exact reconstructed recovery envelope",
        ));
    }
    Ok(())
}

fn manifest_from_run_payload(
    payload: &RunPayloadPreimage,
    prepared_record_hash: &str,
    expected_staged_row_count: u32,
    staged_db_content_hash: &str,
    recovery_envelope_content_hash: &str,
    run_status: RunStatus,
    staged_at: String,
) -> RepositoryResult<RunManifestContentPreimage> {
    let manifest = RunManifestContentPreimage {
        domain: DOMAIN_RUN_MANIFEST.into(),
        subject_kind: payload.subject_kind,
        subject_id: payload.subject_id.clone(),
        in_memory_payload_hash: hash(payload)?,
        prepared_record_hash: prepared_record_hash.into(),
        expected_staged_row_count,
        staged_db_content_hash: staged_db_content_hash.into(),
        recovery_envelope_content_hash: recovery_envelope_content_hash.into(),
        logical_subject_key: payload.logical_subject_key.clone(),
        run_status,
        source_fact_key: payload.source_fact_key.clone(),
        config_activation_run_id: Some(payload.config_activation_run_id.clone()),
        config_hash: Some(payload.config_hash.clone()),
        config_snapshot_json_hash: payload.config_snapshot_json_hash.clone(),
        config_activation_content_hash: payload.config_activation_content_hash.clone(),
        config_activation_file_content_hash: payload.config_activation_file_content_hash.clone(),
        config_effective_from_rfc3339_nanos_utc: payload
            .config_effective_from_rfc3339_nanos_utc
            .clone(),
        artifact_valid_from: payload.artifact_valid_from.clone(),
        artifact_expires_at: payload.artifact_expires_at.clone(),
        executable_revision: payload.executable_revision.clone(),
        legacy_cutover_snapshot_hash: payload.legacy_cutover_snapshot_hash.clone(),
        generation_market_date: payload.generation_market_date.clone(),
        aggregator_observed_at_rfc3339_nanos_utc: payload
            .aggregator_observed_at_rfc3339_nanos_utc
            .clone(),
        ingress_source_batch_content_hash: payload.ingress_source_batch_content_hash.clone(),
        outcome_phase: payload.outcome_phase,
        stored_due_date: payload.stored_due_date.clone(),
        outcome_claim_id: payload.outcome_claim_id.clone(),
        planned_outcome_run_id: payload.planned_outcome_run_id.clone(),
        outcome_claim_receipt_content_hash: payload.outcome_claim_receipt_content_hash.clone(),
        outcome_claim_due_binding_hash: payload.outcome_claim_due_binding_hash.clone(),
        outcome_claim_provider_request_hash: payload.outcome_claim_provider_request_hash.clone(),
        staged_at_rfc3339_nanos_utc: staged_at,
    };
    manifest.validate_kind_matrix().map_err(|error| {
        SelectionV2RepositoryError::Canonical(format!("manifest matrix invalid: {error}"))
    })?;
    Ok(manifest)
}

fn ingress_run_row_hashes(
    stage: &SourceIngressStageInputPreimage,
) -> RepositoryResult<Vec<String>> {
    let mut rows = Vec::new();
    for row in &stage.source_batch_attempt_rows {
        rows.push(run_row_hash(
            TABLE_SOURCE_BATCH_ATTEMPT,
            "selection_source_batch_attempts",
            vec![row.source_batch_attempt_id.clone()],
            hash(row)?,
        )?);
    }
    for row in &stage.source_fact_rows {
        rows.push(run_row_hash(
            TABLE_SOURCE_FACT,
            "selection_source_facts_v2",
            vec![row.source_fact_key.clone()],
            hash(row)?,
        )?);
    }
    for row in &stage.source_fact_attempt_rows {
        rows.push(run_row_hash(
            TABLE_SOURCE_FACT_ATTEMPT,
            "selection_source_fact_attempts",
            vec![row.source_fact_attempt_id.clone()],
            hash(row)?,
        )?);
    }
    rows.sort_by(|left, right| {
        (left.table_ordinal, left.logical_primary_key.as_bytes())
            .cmp(&(right.table_ordinal, right.logical_primary_key.as_bytes()))
    });
    rows.iter().map(hash).collect()
}

fn generation_run_row_hashes(
    stage: &GenerationStageInputPreimage,
) -> RepositoryResult<Vec<String>> {
    let mut rows = Vec::new();
    for row in &stage.relation_attempt_rows {
        rows.push(run_row_hash(
            TABLE_RELATION_ATTEMPT,
            "selection_relation_attempts",
            vec![row.relation_attempt_id.clone()],
            hash(row)?,
        )?);
    }
    for row in &stage.evaluation_attempt_rows {
        rows.push(run_row_hash(
            TABLE_EVALUATION_ATTEMPT,
            "selection_evaluation_attempts",
            vec![row.evaluation_attempt_id.clone()],
            hash(row)?,
        )?);
    }
    for row in &stage.sample_rows {
        rows.push(run_row_hash(
            TABLE_SAMPLE,
            "selection_samples",
            vec![row.sample_key.clone()],
            hash(row)?,
        )?);
    }
    for row in &stage.rejection_rows {
        rows.push(run_row_hash(
            TABLE_REJECTION,
            "selection_rejections",
            vec![row.sample_key.clone(), row.ordinal.to_string()],
            hash(row)?,
        )?);
    }
    rows.sort_by(|left, right| {
        (left.table_ordinal, left.logical_primary_key.as_bytes())
            .cmp(&(right.table_ordinal, right.logical_primary_key.as_bytes()))
    });
    rows.iter().map(hash).collect()
}

fn outcome_run_row_hashes(stage: &OutcomeStageInputPreimage) -> RepositoryResult<Vec<String>> {
    let mut rows = Vec::new();
    for row in &stage.outcome_rows {
        rows.push(run_row_hash(
            TABLE_SAMPLE_OUTCOME,
            "selection_sample_outcomes",
            vec![row.sample_key.clone(), row.phase.as_str().into()],
            hash(row)?,
        )?);
    }
    for row in &stage.outcome_attempt_rows {
        rows.push(run_row_hash(
            TABLE_OUTCOME_ATTEMPT,
            "selection_outcome_attempts",
            vec![row.outcome_attempt_id.clone()],
            hash(row)?,
        )?);
    }
    rows.sort_by(|left, right| {
        (left.table_ordinal, left.logical_primary_key.as_bytes())
            .cmp(&(right.table_ordinal, right.logical_primary_key.as_bytes()))
    });
    rows.iter().map(hash).collect()
}

fn staged_row_hashes(
    envelope: &SelectionRecoveryEnvelopeRowContentPreimage,
    envelope_content_hash: &str,
    domain_row_hashes: &[String],
) -> RepositoryResult<Vec<String>> {
    let mut rows = Vec::with_capacity(domain_row_hashes.len() + 1);
    for serialized_hash in domain_row_hashes {
        rows.push(serialized_hash.clone());
    }
    rows.push(hash(&run_row_hash(
        TABLE_RECOVERY_ENVELOPE,
        "selection_v2_recovery_envelopes",
        vec![envelope.stage_run_id.clone()],
        envelope_content_hash.into(),
    )?)?);
    // Domain row hashes were already ordered by table ordinal.  The envelope
    // ordinal is 10, so appending is the canonical global order.
    Ok(rows)
}

fn run_row_hash(
    table_ordinal: u8,
    table_name: &str,
    key_parts: Vec<String>,
    row_content_hash: String,
) -> RepositoryResult<RunRowHashPreimage> {
    let logical_pk = RunRowLogicalPrimaryKeyPreimage {
        domain: DOMAIN_RUN_ROW_LOGICAL_PK.into(),
        table_ordinal,
        key_parts,
    };
    Ok(RunRowHashPreimage {
        domain: DOMAIN_RUN_ROW.into(),
        table_ordinal,
        table_name: table_name.into(),
        logical_primary_key: hash(&logical_pk)?,
        row_content_hash,
    })
}

fn insert_envelope(
    conn: &mut SqliteConnection,
    row: &SelectionRecoveryEnvelopeRowContentPreimage,
    content_hash: &str,
) -> RepositoryResult<()> {
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_v2_recovery_envelopes (
            stage_run_id, subject_kind, logical_subject_key, payload_schema,
            payload_json, payload_json_hash, in_memory_payload_hash,
            config_activation_run_id, config_hash, enveloped_at, content_hash
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&row.stage_run_id)
    .bind::<Text, _>(row.subject_kind.as_str())
    .bind::<Text, _>(&row.logical_subject_key)
    .bind::<Text, _>(&row.payload_schema)
    .bind::<Text, _>(&row.payload_json)
    .bind::<Text, _>(&row.payload_json_hash)
    .bind::<Text, _>(&row.in_memory_payload_hash)
    .bind::<Text, _>(&row.config_activation_run_id)
    .bind::<Text, _>(&row.config_hash)
    .bind::<Text, _>(&row.enveloped_at)
    .bind::<Text, _>(content_hash)
    .execute(conn)?;
    let stored = find_envelope(conn, &row.stage_run_id)?.ok_or_else(|| {
        invariant(
            "envelope_insert_missing",
            "recovery envelope was not visible after insert",
        )
    })?;
    if stored.content_hash != content_hash || rebuild_envelope(&stored)? != *row {
        return Err(SelectionV2RepositoryError::ReplayConflict {
            subject_kind: row.subject_kind.as_str().into(),
            subject_id: row.stage_run_id.clone(),
        });
    }
    Ok(())
}

fn insert_ingress_rows(
    conn: &mut SqliteConnection,
    stage: &SourceIngressStageInputPreimage,
) -> RepositoryResult<()> {
    for row in &stage.source_batch_attempt_rows {
        insert_source_batch_attempt(conn, row)?;
    }
    for row in &stage.source_fact_rows {
        insert_source_fact(conn, row)?;
    }
    for row in &stage.source_fact_attempt_rows {
        insert_source_fact_attempt(conn, row)?;
    }
    Ok(())
}

fn insert_generation_rows(
    conn: &mut SqliteConnection,
    stage: &GenerationStageInputPreimage,
) -> RepositoryResult<()> {
    for row in &stage.relation_attempt_rows {
        insert_relation_attempt(conn, row)?;
    }
    for row in &stage.evaluation_attempt_rows {
        insert_evaluation_attempt(conn, row)?;
    }
    for row in &stage.sample_rows {
        insert_sample(conn, row)?;
    }
    for row in &stage.rejection_rows {
        insert_rejection(conn, row)?;
    }
    Ok(())
}

fn insert_outcome_rows(
    conn: &mut SqliteConnection,
    stage: &OutcomeStageInputPreimage,
) -> RepositoryResult<()> {
    for row in &stage.outcome_rows {
        insert_sample_outcome(conn, row)?;
    }
    for row in &stage.outcome_attempt_rows {
        insert_outcome_attempt(conn, row)?;
    }
    Ok(())
}

fn insert_source_batch_attempt(
    conn: &mut SqliteConnection,
    row: &SelectionSourceBatchAttemptRowContentPreimage,
) -> RepositoryResult<()> {
    let content_hash = hash(row)?;
    let record_count = row.record_count.map(i64::from);
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_source_batch_attempts (
            source_batch_attempt_id, ingress_run_id, config_activation_run_id,
            config_hash, generation_market_date, registered_feed_identity,
            registered_feed_snapshot_hash, request_hash, request_evidence_json,
            request_evidence_hash, feed_attempt_content_hash, status_kind, record_count,
            provider, source, source_at, observed_at, batch_id, batch_content_hash,
            failed_stage, reason_code, retryable, available_evidence_json,
            available_evidence_hash, error_detail_json, error_detail_hash,
            error_fingerprint, attempted_at, content_hash
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                   ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&row.source_batch_attempt_id)
    .bind::<Text, _>(&row.ingress_run_id)
    .bind::<Text, _>(&row.config_activation_run_id)
    .bind::<Text, _>(&row.config_hash)
    .bind::<Text, _>(&row.generation_market_date)
    .bind::<Text, _>(&row.registered_feed_identity)
    .bind::<Text, _>(&row.registered_feed_snapshot_hash)
    .bind::<Text, _>(&row.request_hash)
    .bind::<Text, _>(&row.request_evidence_json)
    .bind::<Text, _>(&row.request_evidence_hash)
    .bind::<Text, _>(&row.feed_attempt_content_hash)
    .bind::<Text, _>(row.status_kind.as_str())
    .bind::<Nullable<BigInt>, _>(record_count)
    .bind::<Nullable<Text>, _>(row.provider.as_deref())
    .bind::<Nullable<Text>, _>(row.source.as_deref())
    .bind::<Nullable<Text>, _>(row.source_at.as_deref())
    .bind::<Nullable<Text>, _>(row.observed_at.as_deref())
    .bind::<Nullable<Text>, _>(row.batch_id.as_deref())
    .bind::<Nullable<Text>, _>(row.batch_content_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.failed_stage.as_deref())
    .bind::<Nullable<Text>, _>(row.reason_code.as_deref())
    .bind::<Nullable<Integer>, _>(row.retryable.map(i32::from))
    .bind::<Nullable<Text>, _>(row.available_evidence_json.as_deref())
    .bind::<Nullable<Text>, _>(row.available_evidence_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.error_detail_json.as_deref())
    .bind::<Nullable<Text>, _>(row.error_detail_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.error_fingerprint.as_deref())
    .bind::<Text, _>(&row.attempted_at)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    verify_content_hash(
        conn,
        "selection_source_batch_attempts",
        "source_batch_attempt_id",
        &row.source_batch_attempt_id,
        &content_hash,
    )
}

fn insert_source_fact(
    conn: &mut SqliteConnection,
    row: &SelectionSourceFactRowContentPreimage,
) -> RepositoryResult<()> {
    let content_hash = hash(row)?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_source_facts_v2 (
            source_fact_key, event_id, payload_schema, config_activation_run_id,
            config_hash, generation_market_date, provider_source, item_id, title,
            summary, content, publisher, canonical_url, published_at,
            instruments_json, topics_json, language, record_provider, record_source,
            record_source_at, record_observed_at, record_batch_id,
            record_batch_content_hash, provider_content_hash, first_ingress_run_id,
            ingress_gate_version, ingress_gate_input_json, ingress_gate_input_hash,
            ingress_decision, ingress_reason_code, ingress_retryable,
            ingress_gate_receipt_json, ingress_gate_receipt_hash, content_hash
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                   ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&row.source_fact_key)
    .bind::<Text, _>(&row.event_id)
    .bind::<Text, _>(&row.payload_schema)
    .bind::<Text, _>(&row.config_activation_run_id)
    .bind::<Text, _>(&row.config_hash)
    .bind::<Text, _>(&row.generation_market_date)
    .bind::<Text, _>(&row.provider_source)
    .bind::<Text, _>(&row.item_id)
    .bind::<Text, _>(&row.title)
    .bind::<Nullable<Text>, _>(row.summary.as_deref())
    .bind::<Nullable<Text>, _>(row.content.as_deref())
    .bind::<Text, _>(&row.publisher)
    .bind::<Text, _>(&row.canonical_url)
    .bind::<Text, _>(&row.published_at)
    .bind::<Text, _>(&row.instruments_json)
    .bind::<Text, _>(&row.topics_json)
    .bind::<Text, _>(&row.language)
    .bind::<Text, _>(&row.record_provider)
    .bind::<Text, _>(&row.record_source)
    .bind::<Nullable<Text>, _>(row.record_source_at.as_deref())
    .bind::<Text, _>(&row.record_observed_at)
    .bind::<Text, _>(&row.record_batch_id)
    .bind::<Text, _>(&row.record_batch_content_hash)
    .bind::<Text, _>(&row.provider_content_hash)
    .bind::<Text, _>(&row.first_ingress_run_id)
    .bind::<Text, _>(&row.ingress_gate_version)
    .bind::<Text, _>(&row.ingress_gate_input_json)
    .bind::<Text, _>(&row.ingress_gate_input_hash)
    .bind::<Text, _>(row.ingress_decision.as_str())
    .bind::<Nullable<Text>, _>(row.ingress_reason_code.as_deref())
    .bind::<Nullable<Integer>, _>(row.ingress_retryable.map(i32::from))
    .bind::<Text, _>(&row.ingress_gate_receipt_json)
    .bind::<Text, _>(&row.ingress_gate_receipt_hash)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    verify_content_hash(
        conn,
        "selection_source_facts_v2",
        "source_fact_key",
        &row.source_fact_key,
        &content_hash,
    )
}

fn insert_source_fact_attempt(
    conn: &mut SqliteConnection,
    row: &SelectionSourceFactAttemptRowContentPreimage,
) -> RepositoryResult<()> {
    let content_hash = hash(row)?;
    let attempt_result = match row.attempt_result {
        SourceFactAttemptResult::Accepted => "inserted",
        SourceFactAttemptResult::Replay => "exact_replay",
        SourceFactAttemptResult::Conflict => "conflict",
    };
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_source_fact_attempts (
            source_fact_attempt_id, ingress_run_id, source_batch_attempt_id,
            provider_ordinal, source_fact_key, acquired_record_json,
            acquired_record_hash, batch_evidence_json, batch_evidence_hash,
            event_projection_id, attempt_result, conflict_hash, attempted_at,
            content_hash
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(&row.source_fact_attempt_id)
    .bind::<Text, _>(&row.ingress_run_id)
    .bind::<Text, _>(&row.source_batch_attempt_id)
    .bind::<BigInt, _>(i64::from(row.provider_ordinal))
    .bind::<Text, _>(&row.source_fact_key)
    .bind::<Text, _>(&row.acquired_record_json)
    .bind::<Text, _>(&row.acquired_record_hash)
    .bind::<Text, _>(&row.batch_evidence_json)
    .bind::<Text, _>(&row.batch_evidence_hash)
    .bind::<Text, _>(&row.event_projection_id)
    .bind::<Text, _>(attempt_result)
    .bind::<Nullable<Text>, _>(row.conflict_hash.as_deref())
    .bind::<Text, _>(&row.attempted_at)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    verify_content_hash(
        conn,
        "selection_source_fact_attempts",
        "source_fact_attempt_id",
        &row.source_fact_attempt_id,
        &content_hash,
    )
}

fn insert_relation_attempt(
    conn: &mut SqliteConnection,
    row: &SelectionRelationAttemptRowContentPreimage,
) -> RepositoryResult<()> {
    let content_hash = hash(row)?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_relation_attempts (
            relation_attempt_id,relation_key,generation_run_id,source_fact_key,event_id,
            chain_id,config_activation_run_id,config_hash,relation_schema_version,
            relation_kind,relation_source_identity_json,relation_source_identity_hash,
            typed_binding_state_json,typed_binding_state_hash,request_hash,
            request_evidence_json,request_evidence_hash,result_code,failed_stage,
            retryable,raw_identity_json,raw_identity_hash,
            canonical_stock_code,canonical_stock_name,canonical_market,
            artifact_content_hash,binding_audit_hash,provider_board_kind,
            provider_board_code,provider_board_name,provider_source,provider_source_at,
            provider_observed_at,provider_batch_id,provider_batch_content_hash,
            actual_constituent_count,available_evidence_json,available_evidence_hash,
            error_detail_json,error_detail_hash,error_fingerprint,attempted_at,content_hash
         ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,
                   ?,?,?,?,?,?,?,?,?,?)",
    )
    .bind::<Text, _>(&row.relation_attempt_id)
    .bind::<Text, _>(&row.relation_key)
    .bind::<Text, _>(&row.generation_run_id)
    .bind::<Text, _>(&row.source_fact_key)
    .bind::<Text, _>(&row.event_id)
    .bind::<Text, _>(&row.chain_id)
    .bind::<Text, _>(&row.config_activation_run_id)
    .bind::<Text, _>(&row.config_hash)
    .bind::<Text, _>(&row.relation_schema_version)
    .bind::<Text, _>(row.relation_kind.as_str())
    .bind::<Text, _>(&row.relation_source_identity_json)
    .bind::<Text, _>(&row.relation_source_identity_hash)
    .bind::<Text, _>(&row.typed_binding_state_json)
    .bind::<Text, _>(&row.typed_binding_state_hash)
    .bind::<Nullable<Text>, _>(row.request_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.request_evidence_json.as_deref())
    .bind::<Nullable<Text>, _>(row.request_evidence_hash.as_deref())
    .bind::<Text, _>(&row.result_code)
    .bind::<Nullable<Text>, _>(row.failed_stage.as_deref())
    .bind::<Nullable<Integer>, _>(row.retryable.map(i32::from))
    .bind::<Nullable<Text>, _>(row.raw_identity_json.as_deref())
    .bind::<Nullable<Text>, _>(row.raw_identity_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.canonical_stock_code.as_deref())
    .bind::<Nullable<Text>, _>(row.canonical_stock_name.as_deref())
    .bind::<Nullable<Text>, _>(row.canonical_market.as_deref())
    .bind::<Nullable<Text>, _>(row.artifact_content_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.binding_audit_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.provider_board_kind.map(|value| value.as_str()))
    .bind::<Nullable<Text>, _>(row.provider_board_code.as_deref())
    .bind::<Nullable<Text>, _>(row.provider_board_name.as_deref())
    .bind::<Nullable<Text>, _>(row.provider_source.as_deref())
    .bind::<Nullable<Text>, _>(row.provider_source_at.as_deref())
    .bind::<Nullable<Text>, _>(row.provider_observed_at.as_deref())
    .bind::<Nullable<Text>, _>(row.provider_batch_id.as_deref())
    .bind::<Nullable<Text>, _>(row.provider_batch_content_hash.as_deref())
    .bind::<Nullable<BigInt>, _>(row.actual_constituent_count.map(i64::from))
    .bind::<Nullable<Text>, _>(row.available_evidence_json.as_deref())
    .bind::<Nullable<Text>, _>(row.available_evidence_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.error_detail_json.as_deref())
    .bind::<Nullable<Text>, _>(row.error_detail_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.error_fingerprint.as_deref())
    .bind::<Text, _>(&row.attempted_at)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    verify_content_hash(
        conn,
        "selection_relation_attempts",
        "relation_attempt_id",
        &row.relation_attempt_id,
        &content_hash,
    )
}

fn insert_evaluation_attempt(
    conn: &mut SqliteConnection,
    row: &SelectionEvaluationAttemptRowContentPreimage,
) -> RepositoryResult<()> {
    let content_hash = hash(row)?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_evaluation_attempts (
            evaluation_attempt_id,sample_key,generation_run_id,source_fact_key,event_id,
            chain_id,canonical_stock_code,canonical_stock_name,canonical_market,
            relation_evidence_set_hash,market_request_hash,request_evidence_json,
            request_evidence_hash,result_code,failed_stage,retryable,provider,source,
            source_at,observed_at,batch_id,batch_content_hash,available_evidence_json,
            available_evidence_hash,terminal_decision_hash,error_detail_json,
            error_detail_hash,error_fingerprint,attempted_at,content_hash
         ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind::<Text, _>(&row.evaluation_attempt_id)
    .bind::<Text, _>(&row.sample_key)
    .bind::<Text, _>(&row.generation_run_id)
    .bind::<Text, _>(&row.source_fact_key)
    .bind::<Text, _>(&row.event_id)
    .bind::<Text, _>(&row.chain_id)
    .bind::<Text, _>(&row.canonical_stock_code)
    .bind::<Text, _>(&row.canonical_stock_name)
    .bind::<Text, _>(&row.canonical_market)
    .bind::<Text, _>(&row.relation_evidence_set_hash)
    .bind::<Text, _>(&row.market_request_hash)
    .bind::<Text, _>(&row.request_evidence_json)
    .bind::<Text, _>(&row.request_evidence_hash)
    .bind::<Text, _>(&row.result_code)
    .bind::<Nullable<Text>, _>(row.failed_stage.as_deref())
    .bind::<Nullable<Integer>, _>(row.retryable.map(i32::from))
    .bind::<Nullable<Text>, _>(row.provider.as_deref())
    .bind::<Nullable<Text>, _>(row.source.as_deref())
    .bind::<Nullable<Text>, _>(row.source_at.as_deref())
    .bind::<Nullable<Text>, _>(row.observed_at.as_deref())
    .bind::<Nullable<Text>, _>(row.batch_id.as_deref())
    .bind::<Nullable<Text>, _>(row.batch_content_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.available_evidence_json.as_deref())
    .bind::<Nullable<Text>, _>(row.available_evidence_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.terminal_decision_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.error_detail_json.as_deref())
    .bind::<Nullable<Text>, _>(row.error_detail_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.error_fingerprint.as_deref())
    .bind::<Text, _>(&row.attempted_at)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    verify_content_hash(
        conn,
        "selection_evaluation_attempts",
        "evaluation_attempt_id",
        &row.evaluation_attempt_id,
        &content_hash,
    )
}

fn insert_sample(
    conn: &mut SqliteConnection,
    row: &SelectionSampleRowContentPreimage,
) -> RepositoryResult<()> {
    let content_hash = hash(row)?;
    let rejection_hashes = canonical(&row.rejection_row_hashes_in_ordinal_order)?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_samples (
            sample_key,generation_run_id,source_fact_key,source_fact_content_hash,
            source_fact_attempt_id,source_batch_attempt_id,event_id,chain_id,
            config_activation_run_id,config_hash,matched_keyword,canonical_stock_code,
            canonical_stock_name,canonical_market,relation_schema_version,
            relation_evidence_json,relation_evidence_set_hash,feature_version,
            t0_feature_json,t0_feature_hash,market_provider,market_source,market_source_at,
            market_observed_at,market_batch_id,market_batch_content_hash,admission_version,
            decision_kind,rejection_count,rejection_row_hashes_in_ordinal_order,
            evaluation_market_date,t0_due_date,d1_due_date,d2_due_date,d3_due_date,
            d4_due_date,d5_due_date,calendar_version,calendar_hash,
            trading_date_vector_json,trading_date_vector_hash,staged_at,content_hash
         ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,
                   ?,?,?,?,?,?,?,?,?,?)",
    )
    .bind::<Text, _>(&row.sample_key)
    .bind::<Text, _>(&row.generation_run_id)
    .bind::<Text, _>(&row.source_fact_key)
    .bind::<Text, _>(&row.source_fact_content_hash)
    .bind::<Text, _>(&row.source_fact_attempt_id)
    .bind::<Text, _>(&row.source_batch_attempt_id)
    .bind::<Text, _>(&row.event_id)
    .bind::<Text, _>(&row.chain_id)
    .bind::<Text, _>(&row.config_activation_run_id)
    .bind::<Text, _>(&row.config_hash)
    .bind::<Text, _>(&row.matched_keyword)
    .bind::<Text, _>(&row.canonical_stock_code)
    .bind::<Text, _>(&row.canonical_stock_name)
    .bind::<Text, _>(&row.canonical_market)
    .bind::<Text, _>(&row.relation_schema_version)
    .bind::<Text, _>(&row.relation_evidence_json)
    .bind::<Text, _>(&row.relation_evidence_set_hash)
    .bind::<Text, _>(&row.feature_version)
    .bind::<Text, _>(&row.t0_feature_json)
    .bind::<Text, _>(&row.t0_feature_hash)
    .bind::<Text, _>(&row.market_provider)
    .bind::<Text, _>(&row.market_source)
    .bind::<Nullable<Text>, _>(row.market_source_at.as_deref())
    .bind::<Text, _>(&row.market_observed_at)
    .bind::<Text, _>(&row.market_batch_id)
    .bind::<Text, _>(&row.market_batch_content_hash)
    .bind::<Text, _>(&row.admission_version)
    .bind::<Text, _>(row.decision_kind.as_str())
    .bind::<BigInt, _>(i64::from(row.rejection_count))
    .bind::<Text, _>(&rejection_hashes)
    .bind::<Text, _>(&row.evaluation_market_date)
    .bind::<Text, _>(&row.t0_due_date)
    .bind::<Text, _>(&row.d1_due_date)
    .bind::<Text, _>(&row.d2_due_date)
    .bind::<Text, _>(&row.d3_due_date)
    .bind::<Text, _>(&row.d4_due_date)
    .bind::<Text, _>(&row.d5_due_date)
    .bind::<Text, _>(&row.calendar_version)
    .bind::<Text, _>(&row.calendar_hash)
    .bind::<Text, _>(&row.trading_date_vector_json)
    .bind::<Text, _>(&row.trading_date_vector_hash)
    .bind::<Text, _>(&row.staged_at)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    verify_content_hash(
        conn,
        "selection_samples",
        "sample_key",
        &row.sample_key,
        &content_hash,
    )
}

fn insert_rejection(
    conn: &mut SqliteConnection,
    row: &SelectionRejectionRowContentPreimage,
) -> RepositoryResult<()> {
    let content_hash = hash(row)?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_rejections (
            sample_key,ordinal,generation_run_id,reason_code,rule_id,retryable,
            structured_detail_json,structured_detail_hash,provider,source,source_at,
            observed_at,batch_id,batch_content_hash,created_at,content_hash
         ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind::<Text, _>(&row.sample_key)
    .bind::<BigInt, _>(i64::from(row.ordinal))
    .bind::<Text, _>(&row.generation_run_id)
    .bind::<Text, _>(&row.reason_code)
    .bind::<Text, _>(&row.rule_id)
    .bind::<Integer, _>(i32::from(row.retryable))
    .bind::<Text, _>(&row.structured_detail_json)
    .bind::<Text, _>(&row.structured_detail_hash)
    .bind::<Nullable<Text>, _>(row.provider.as_deref())
    .bind::<Nullable<Text>, _>(row.source.as_deref())
    .bind::<Nullable<Text>, _>(row.source_at.as_deref())
    .bind::<Nullable<Text>, _>(row.observed_at.as_deref())
    .bind::<Nullable<Text>, _>(row.batch_id.as_deref())
    .bind::<Nullable<Text>, _>(row.batch_content_hash.as_deref())
    .bind::<Text, _>(&row.created_at)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    let stored = diesel::sql_query(
        "SELECT content_hash FROM selection_rejections
         WHERE sample_key=? AND ordinal=?",
    )
    .bind::<Text, _>(&row.sample_key)
    .bind::<BigInt, _>(i64::from(row.ordinal))
    .get_result::<ContentHashRow>(conn)?;
    if stored.content_hash != content_hash {
        return Err(SelectionV2RepositoryError::ReplayConflict {
            subject_kind: SubjectKind::GenerationRun.as_str().into(),
            subject_id: row.generation_run_id.clone(),
        });
    }
    Ok(())
}

fn insert_sample_outcome(
    conn: &mut SqliteConnection,
    row: &SelectionSampleOutcomeRowContentPreimage,
) -> RepositoryResult<()> {
    let content_hash = hash(row)?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_sample_outcomes (
            sample_key,phase,outcome_run_id,due_trading_date,open,high,low,close,
            volume,amount,return_from_t0_close,cumulative_mfe,cumulative_mae,
            volume_ratio,provider,source,source_at,observed_at,batch_id,
            batch_content_hash,created_at,content_hash
         ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind::<Text, _>(&row.sample_key)
    .bind::<Text, _>(row.phase.as_str())
    .bind::<Text, _>(&row.outcome_run_id)
    .bind::<Text, _>(&row.due_trading_date)
    .bind::<Text, _>(&row.open)
    .bind::<Text, _>(&row.high)
    .bind::<Text, _>(&row.low)
    .bind::<Text, _>(&row.close)
    .bind::<Text, _>(&row.volume)
    .bind::<Text, _>(&row.amount)
    .bind::<Text, _>(&row.return_from_t0_close)
    .bind::<Text, _>(&row.cumulative_mfe)
    .bind::<Text, _>(&row.cumulative_mae)
    .bind::<Text, _>(&row.volume_ratio)
    .bind::<Text, _>(&row.provider)
    .bind::<Text, _>(&row.source)
    .bind::<Nullable<Text>, _>(row.source_at.as_deref())
    .bind::<Text, _>(&row.observed_at)
    .bind::<Text, _>(&row.batch_id)
    .bind::<Text, _>(&row.batch_content_hash)
    .bind::<Text, _>(&row.created_at)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    let stored = diesel::sql_query(
        "SELECT content_hash FROM selection_sample_outcomes
         WHERE sample_key=? AND phase=?",
    )
    .bind::<Text, _>(&row.sample_key)
    .bind::<Text, _>(row.phase.as_str())
    .get_result::<ContentHashRow>(conn)?;
    if stored.content_hash != content_hash {
        return Err(SelectionV2RepositoryError::ReplayConflict {
            subject_kind: SubjectKind::OutcomeRun.as_str().into(),
            subject_id: row.outcome_run_id.clone(),
        });
    }
    Ok(())
}

fn insert_outcome_attempt(
    conn: &mut SqliteConnection,
    row: &SelectionOutcomeAttemptRowContentPreimage,
) -> RepositoryResult<()> {
    let content_hash = hash(row)?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO selection_outcome_attempts (
            outcome_attempt_id,sample_key,phase,stored_due_date,outcome_run_id,
            request_hash,request_evidence_json,request_evidence_hash,
            transport_attempts_json,transport_attempts_hash,
            result_code,reason_code,retryable,provider,source,source_at,observed_at,
            batch_id,batch_content_hash,available_evidence_json,available_evidence_hash,
            error_detail_json,error_detail_hash,error_fingerprint,
            settled_outcome_content_hash,attempted_at,content_hash
         ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind::<Text, _>(&row.outcome_attempt_id)
    .bind::<Text, _>(&row.sample_key)
    .bind::<Text, _>(row.phase.as_str())
    .bind::<Text, _>(&row.stored_due_date)
    .bind::<Text, _>(&row.outcome_run_id)
    .bind::<Nullable<Text>, _>(row.request_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.request_evidence_json.as_deref())
    .bind::<Nullable<Text>, _>(row.request_evidence_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.transport_attempts_json.as_deref())
    .bind::<Nullable<Text>, _>(row.transport_attempts_hash.as_deref())
    .bind::<Text, _>(row.result_code.as_str())
    .bind::<Nullable<Text>, _>(row.reason_code.map(|value| value.as_str()))
    .bind::<Nullable<Integer>, _>(row.retryable.map(i32::from))
    .bind::<Nullable<Text>, _>(row.provider.as_deref())
    .bind::<Nullable<Text>, _>(row.source.as_deref())
    .bind::<Nullable<Text>, _>(row.source_at.as_deref())
    .bind::<Nullable<Text>, _>(row.observed_at.as_deref())
    .bind::<Nullable<Text>, _>(row.batch_id.as_deref())
    .bind::<Nullable<Text>, _>(row.batch_content_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.available_evidence_json.as_deref())
    .bind::<Nullable<Text>, _>(row.available_evidence_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.error_detail_json.as_deref())
    .bind::<Nullable<Text>, _>(row.error_detail_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.error_fingerprint.as_deref())
    .bind::<Nullable<Text>, _>(row.settled_outcome_content_hash.as_deref())
    .bind::<Text, _>(&row.attempted_at)
    .bind::<Text, _>(&content_hash)
    .execute(conn)?;
    verify_content_hash(
        conn,
        "selection_outcome_attempts",
        "outcome_attempt_id",
        &row.outcome_attempt_id,
        &content_hash,
    )
}

fn verify_content_hash(
    conn: &mut SqliteConnection,
    table: &'static str,
    key_column: &'static str,
    key: &str,
    expected_hash: &str,
) -> RepositoryResult<()> {
    let sql = format!("SELECT content_hash FROM {table} WHERE {key_column}=?");
    let stored = diesel::sql_query(sql)
        .bind::<Text, _>(key)
        .get_result::<ContentHashRow>(conn)
        .optional()?
        .ok_or_else(|| {
            invariant(
                "domain_row_insert_missing",
                format!("{table}.{key_column}={key}"),
            )
        })?;
    if stored.content_hash != expected_hash {
        return Err(SelectionV2RepositoryError::ReplayConflict {
            subject_kind: table.into(),
            subject_id: key.into(),
        });
    }
    Ok(())
}

fn insert_manifest(
    conn: &mut SqliteConnection,
    row: &RunManifestContentPreimage,
    content_hash: &str,
) -> RepositoryResult<()> {
    diesel::sql_query(
        "INSERT INTO selection_v2_run_stages (
            subject_kind, subject_id, in_memory_payload_hash, prepared_record_hash,
            expected_staged_row_count, staged_db_content_hash,
            recovery_envelope_content_hash, logical_subject_key, run_status,
            source_fact_key, config_activation_run_id, config_hash,
            config_snapshot_json_hash, config_activation_content_hash,
            config_activation_file_content_hash, config_effective_from,
            artifact_valid_from, artifact_expires_at, executable_revision,
            legacy_cutover_snapshot_hash, generation_market_date,
            aggregator_observed_at, ingress_source_batch_content_hash,
            outcome_phase, stored_due_date, outcome_claim_id,
            planned_outcome_run_id, outcome_claim_receipt_content_hash,
            outcome_claim_due_binding_hash, outcome_claim_provider_request_hash,
            staged_at, manifest_content_hash
         ) VALUES (
            ?, ?, ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?, ?, ?, ?
         )",
    )
    .bind::<Text, _>(row.subject_kind.as_str())
    .bind::<Text, _>(&row.subject_id)
    .bind::<Text, _>(&row.in_memory_payload_hash)
    .bind::<Text, _>(&row.prepared_record_hash)
    .bind::<BigInt, _>(i64::from(row.expected_staged_row_count))
    .bind::<Text, _>(&row.staged_db_content_hash)
    .bind::<Text, _>(&row.recovery_envelope_content_hash)
    .bind::<Text, _>(&row.logical_subject_key)
    .bind::<Text, _>(row.run_status.as_str())
    .bind::<Nullable<Text>, _>(row.source_fact_key.as_deref())
    .bind::<Text, _>(row.config_activation_run_id.as_deref().unwrap_or_default())
    .bind::<Text, _>(row.config_hash.as_deref().unwrap_or_default())
    .bind::<Nullable<Text>, _>(row.config_snapshot_json_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.config_activation_content_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.config_activation_file_content_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.config_effective_from_rfc3339_nanos_utc.as_deref())
    .bind::<Nullable<Text>, _>(row.artifact_valid_from.as_deref())
    .bind::<Nullable<Text>, _>(row.artifact_expires_at.as_deref())
    .bind::<Nullable<Text>, _>(row.executable_revision.as_deref())
    .bind::<Nullable<Text>, _>(row.legacy_cutover_snapshot_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.generation_market_date.as_deref())
    .bind::<Nullable<Text>, _>(row.aggregator_observed_at_rfc3339_nanos_utc.as_deref())
    .bind::<Nullable<Text>, _>(row.ingress_source_batch_content_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.outcome_phase.map(|value| value.as_str()))
    .bind::<Nullable<Text>, _>(row.stored_due_date.as_deref())
    .bind::<Nullable<Text>, _>(row.outcome_claim_id.as_deref())
    .bind::<Nullable<Text>, _>(row.planned_outcome_run_id.as_deref())
    .bind::<Nullable<Text>, _>(row.outcome_claim_receipt_content_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.outcome_claim_due_binding_hash.as_deref())
    .bind::<Nullable<Text>, _>(row.outcome_claim_provider_request_hash.as_deref())
    .bind::<Text, _>(&row.staged_at_rfc3339_nanos_utc)
    .bind::<Text, _>(content_hash)
    .execute(conn)?;
    Ok(())
}

fn insert_commit_receipt_row(
    conn: &mut SqliteConnection,
    row: &CommitReceiptContentPreimage,
    content_hash: &str,
) -> RepositoryResult<()> {
    diesel::sql_query(
        "INSERT INTO selection_v2_commit_receipts (
            subject_kind, subject_id, logical_subject_key, in_memory_payload_hash,
            recovery_envelope_content_hash, prepared_audit_hash,
            run_manifest_content_hash, staged_db_content_hash,
            committed_audit_hash, committed_at, content_hash
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(row.subject_kind.as_str())
    .bind::<Text, _>(&row.subject_id)
    .bind::<Text, _>(&row.logical_subject_key)
    .bind::<Text, _>(&row.in_memory_payload_hash)
    .bind::<Text, _>(&row.recovery_envelope_content_hash)
    .bind::<Text, _>(&row.prepared_audit_hash)
    .bind::<Text, _>(&row.run_manifest_content_hash)
    .bind::<Text, _>(&row.staged_db_content_hash)
    .bind::<Text, _>(&row.committed_audit_hash)
    .bind::<Text, _>(&row.committed_at_rfc3339_nanos_utc)
    .bind::<Text, _>(content_hash)
    .execute(conn)?;
    Ok(())
}

fn validate_receipt_proofs(
    staged: &StagedRunReceipt,
    prepared: &PreparedAuditProof,
    committed: &CommittedAuditProof,
) -> RepositoryResult<()> {
    require_prepared_proof(
        prepared,
        staged.subject_kind,
        &staged.subject_id,
        &staged.logical_subject_key,
        &staged.recovery_envelope_content_hash,
        &staged.in_memory_payload_hash,
    )?;
    let expected_committed = CommittedAuditContentPreimage {
        domain: DOMAIN_COMMITTED_AUDIT.into(),
        subject_kind: staged.subject_kind,
        subject_id: staged.subject_id.clone(),
        logical_subject_key: staged.logical_subject_key.clone(),
        recovery_envelope_content_hash: staged.recovery_envelope_content_hash.clone(),
        prepared_record_hash: prepared.record_hash.clone(),
        run_manifest_content_hash: staged.run_manifest_content_hash.clone(),
        staged_db_content_hash: staged.staged_db_content_hash.clone(),
    };
    if committed.content != expected_committed
        || committed.content_hash != hash(&expected_committed)?
    {
        return Err(invariant(
            "committed_audit_proof_mismatch",
            "Committed audit proof does not bind the verified staged rows",
        ));
    }
    Ok(())
}

#[derive(Debug, QueryableByName)]
struct ContentHashRow {
    #[diesel(sql_type = Text)]
    content_hash: String,
}

#[derive(Debug, QueryableByName)]
struct SubjectIdRow {
    #[diesel(sql_type = Text)]
    subject_id: String,
}

#[derive(Debug, QueryableByName)]
struct LogicalSubjectKeyRow {
    #[diesel(sql_type = Text)]
    logical_subject_key: String,
}

#[cfg(test)]
#[derive(Debug, QueryableByName)]
struct IntegerValueRow {
    #[diesel(sql_type = BigInt)]
    value: i64,
}

#[derive(Debug, QueryableByName)]
struct TypedRowJsonDb {
    #[diesel(sql_type = Text)]
    row_json: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
}

#[derive(Debug, QueryableByName)]
struct EnvelopeRow {
    #[diesel(sql_type = Text)]
    stage_run_id: String,
    #[diesel(sql_type = Text)]
    subject_kind: String,
    #[diesel(sql_type = Text)]
    logical_subject_key: String,
    #[diesel(sql_type = Text)]
    payload_schema: String,
    #[diesel(sql_type = Text)]
    payload_json: String,
    #[diesel(sql_type = Text)]
    payload_json_hash: String,
    #[diesel(sql_type = Text)]
    in_memory_payload_hash: String,
    #[diesel(sql_type = Text)]
    config_activation_run_id: String,
    #[diesel(sql_type = Text)]
    config_hash: String,
    #[diesel(sql_type = Text)]
    enveloped_at: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
}

#[derive(Debug, QueryableByName)]
struct ManifestRow {
    #[diesel(sql_type = Text)]
    subject_kind: String,
    #[diesel(sql_type = Text)]
    subject_id: String,
    #[diesel(sql_type = Text)]
    in_memory_payload_hash: String,
    #[diesel(sql_type = Text)]
    prepared_record_hash: String,
    #[diesel(sql_type = BigInt)]
    expected_staged_row_count: i64,
    #[diesel(sql_type = Text)]
    staged_db_content_hash: String,
    #[diesel(sql_type = Text)]
    recovery_envelope_content_hash: String,
    #[diesel(sql_type = Text)]
    logical_subject_key: String,
    #[diesel(sql_type = Text)]
    run_status: String,
    #[diesel(sql_type = Nullable<Text>)]
    source_fact_key: Option<String>,
    #[diesel(sql_type = Text)]
    config_activation_run_id: String,
    #[diesel(sql_type = Text)]
    config_hash: String,
    #[diesel(sql_type = Nullable<Text>)]
    config_snapshot_json_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    config_activation_content_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    config_activation_file_content_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    config_effective_from: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    artifact_valid_from: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    artifact_expires_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    executable_revision: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    legacy_cutover_snapshot_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    generation_market_date: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    aggregator_observed_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    ingress_source_batch_content_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    outcome_phase: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    stored_due_date: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    outcome_claim_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    planned_outcome_run_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    outcome_claim_receipt_content_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    outcome_claim_due_binding_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    outcome_claim_provider_request_hash: Option<String>,
    #[diesel(sql_type = Text)]
    staged_at: String,
    #[diesel(sql_type = Text)]
    manifest_content_hash: String,
}

#[derive(Debug, QueryableByName)]
struct CommitReceiptRow {
    #[diesel(sql_type = Text)]
    subject_kind: String,
    #[diesel(sql_type = Text)]
    subject_id: String,
    #[diesel(sql_type = Text)]
    logical_subject_key: String,
    #[diesel(sql_type = Text)]
    in_memory_payload_hash: String,
    #[diesel(sql_type = Text)]
    recovery_envelope_content_hash: String,
    #[diesel(sql_type = Text)]
    prepared_audit_hash: String,
    #[diesel(sql_type = Text)]
    run_manifest_content_hash: String,
    #[diesel(sql_type = Text)]
    staged_db_content_hash: String,
    #[diesel(sql_type = Text)]
    committed_audit_hash: String,
    #[diesel(sql_type = Text)]
    committed_at: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
}

#[derive(Debug, QueryableByName)]
struct OutcomeAuthorityRow {
    #[diesel(sql_type = Text)]
    sample_key: String,
    #[diesel(sql_type = Text)]
    event_id: String,
    #[diesel(sql_type = Text)]
    chain_id: String,
    #[diesel(sql_type = Text)]
    canonical_stock_code: String,
    #[diesel(sql_type = Text)]
    relation_schema_version: String,
    #[diesel(sql_type = Text)]
    feature_version: String,
    #[diesel(sql_type = Text)]
    evaluation_market_date: String,
    #[diesel(sql_type = Text)]
    config_activation_run_id: String,
    #[diesel(sql_type = Text)]
    config_hash: String,
    #[diesel(sql_type = Text)]
    generation_run_id: String,
    #[diesel(sql_type = Text)]
    t0_due_date: String,
    #[diesel(sql_type = Text)]
    d1_due_date: String,
    #[diesel(sql_type = Text)]
    d2_due_date: String,
    #[diesel(sql_type = Text)]
    d3_due_date: String,
    #[diesel(sql_type = Text)]
    d4_due_date: String,
    #[diesel(sql_type = Text)]
    d5_due_date: String,
    #[diesel(sql_type = Text)]
    calendar_version: String,
    #[diesel(sql_type = Text)]
    calendar_hash: String,
    #[diesel(sql_type = Text)]
    trading_date_vector_json: String,
    #[diesel(sql_type = Text)]
    trading_date_vector_hash: String,
    #[diesel(sql_type = Text)]
    generation_receipt_content_hash: String,
    #[diesel(sql_type = Text)]
    activation_receipt_content_hash: String,
}

#[allow(
    dead_code,
    reason = "BR-183 keeps config reuse evidence dormant until selection-v2 activation"
)]
struct ConfigActivationEvidence {
    activation_run_id: String,
    manifest_content_hash: String,
    prepared_audit_hash: String,
    receipt: Option<ConfigActivationReceiptEvidence>,
}

#[allow(
    dead_code,
    reason = "BR-183 keeps config reuse evidence dormant until selection-v2 activation"
)]
struct ConfigActivationReceiptEvidence {
    content_hash: String,
    committed_audit_hash: String,
}

fn find_envelope(
    conn: &mut SqliteConnection,
    subject_id: &str,
) -> RepositoryResult<Option<EnvelopeRow>> {
    Ok(diesel::sql_query(
        "SELECT stage_run_id, subject_kind, logical_subject_key, payload_schema,
                payload_json, payload_json_hash, in_memory_payload_hash,
                config_activation_run_id, config_hash, enveloped_at, content_hash
         FROM selection_v2_recovery_envelopes WHERE stage_run_id=?",
    )
    .bind::<Text, _>(subject_id)
    .get_result::<EnvelopeRow>(conn)
    .optional()?)
}

fn find_outcome_claim_envelopes(
    conn: &mut SqliteConnection,
    logical_subject_key: &str,
) -> RepositoryResult<Vec<EnvelopeRow>> {
    Ok(diesel::sql_query(
        "SELECT stage_run_id, subject_kind, logical_subject_key, payload_schema,
                payload_json, payload_json_hash, in_memory_payload_hash,
                config_activation_run_id, config_hash, enveloped_at, content_hash
         FROM selection_v2_recovery_envelopes
         WHERE subject_kind='outcome_claim' AND logical_subject_key=?
         ORDER BY enveloped_at ASC, stage_run_id ASC",
    )
    .bind::<Text, _>(logical_subject_key)
    .load::<EnvelopeRow>(conn)?)
}

fn find_outcome_envelopes_for_logical_subject(
    conn: &mut SqliteConnection,
    logical_subject_key: &str,
) -> RepositoryResult<Vec<EnvelopeRow>> {
    Ok(diesel::sql_query(
        "SELECT stage_run_id, subject_kind, logical_subject_key, payload_schema,
                payload_json, payload_json_hash, in_memory_payload_hash,
                config_activation_run_id, config_hash, enveloped_at, content_hash
         FROM selection_v2_recovery_envelopes
         WHERE subject_kind='outcome_run' AND logical_subject_key=?
         ORDER BY enveloped_at ASC, stage_run_id ASC",
    )
    .bind::<Text, _>(logical_subject_key)
    .load::<EnvelopeRow>(conn)?)
}

fn validate_lifecycle_envelope(
    row: &EnvelopeRow,
    expected_kind: SubjectKind,
    expected_schema: &str,
    expected_subject_id: &str,
    expected_logical_subject_key: &str,
) -> RepositoryResult<SelectionRecoveryEnvelopeRowContentPreimage> {
    let envelope = rebuild_envelope(row)?;
    if envelope.subject_kind != expected_kind
        || envelope.stage_run_id != expected_subject_id
        || envelope.logical_subject_key != expected_logical_subject_key
        || envelope.payload_schema != expected_schema
        || hash(&envelope)? != row.content_hash
        || canonical_payload_hash(&envelope.payload_json)? != envelope.payload_json_hash
    {
        return Err(invariant(
            "outcome_claim_lifecycle_envelope_binding_mismatch",
            format!(
                "subject {} envelope does not reproduce its exact kind/schema/identity/hash",
                expected_subject_id
            ),
        ));
    }
    let enveloped_at = DateTime::parse_from_rfc3339(&envelope.enveloped_at)
        .map_err(|error| {
            invariant(
                "outcome_claim_lifecycle_enveloped_at_invalid",
                format!("subject {expected_subject_id} enveloped_at is invalid: {error}"),
            )
        })?
        .with_timezone(&Utc);
    if utc_nanos(enveloped_at) != envelope.enveloped_at {
        return Err(invariant(
            "outcome_claim_lifecycle_enveloped_at_noncanonical",
            format!(
                "subject {expected_subject_id} enveloped_at must use exact UTC nanoseconds and Z"
            ),
        ));
    }
    Ok(envelope)
}

fn validate_lifecycle_receipt(
    conn: &mut SqliteConnection,
    staged: &StagedRunReceipt,
    receipt_row: &CommitReceiptRow,
) -> RepositoryResult<String> {
    let receipt = rebuild_commit_receipt(receipt_row)?;
    let receipt_content_hash = hash(&receipt)?;
    if receipt_content_hash != receipt_row.content_hash
        || receipt.subject_kind != staged.subject_kind
        || receipt.subject_id != staged.subject_id
        || receipt.logical_subject_key != staged.logical_subject_key
        || receipt.in_memory_payload_hash != staged.in_memory_payload_hash
        || receipt.recovery_envelope_content_hash != staged.recovery_envelope_content_hash
        || receipt.run_manifest_content_hash != staged.run_manifest_content_hash
        || receipt.staged_db_content_hash != staged.staged_db_content_hash
    {
        return Err(invariant(
            "outcome_claim_lifecycle_receipt_binding_mismatch",
            format!(
                "receipt {} does not exactly bind its staged run",
                staged.subject_id
            ),
        ));
    }
    let manifest = find_manifest(conn, &staged.subject_id)?.ok_or_else(|| {
        invariant(
            "outcome_claim_lifecycle_receipt_manifest_missing",
            format!("receipt {} has no manifest", staged.subject_id),
        )
    })?;
    if receipt.prepared_audit_hash != manifest.prepared_record_hash {
        return Err(invariant(
            "outcome_claim_lifecycle_receipt_prepared_mismatch",
            format!(
                "receipt {} does not bind the manifest Prepared record",
                staged.subject_id
            ),
        ));
    }
    Ok(receipt_content_hash)
}

fn classify_one_outcome_claim_lifecycle(
    conn: &mut SqliteConnection,
    claim_row: &EnvelopeRow,
) -> RepositoryResult<OutcomeClaimLifecycle> {
    let claim_envelope = validate_lifecycle_envelope(
        claim_row,
        SubjectKind::OutcomeClaim,
        OUTCOME_CLAIM_STAGE_PAYLOAD_SCHEMA,
        &claim_row.stage_run_id,
        &claim_row.logical_subject_key,
    )?;
    let claim_stage: OutcomeClaimStageInputPreimage =
        parse_canonical_payload(&claim_envelope.payload_json)?;
    claim_stage.validate().map_err(|error| {
        invariant(
            "outcome_claim_lifecycle_claim_payload_invalid",
            format!(
                "claim {} typed payload is invalid: {error}",
                claim_stage.stage_run_id
            ),
        )
    })?;
    if claim_stage.stage_run_id != claim_envelope.stage_run_id
        || claim_stage.logical_subject_key != claim_envelope.logical_subject_key
        || claim_stage.config_activation_run_id != claim_envelope.config_activation_run_id
        || claim_stage.config_hash != claim_envelope.config_hash
        || claim_stage.claim_lock_key != claim_envelope.logical_subject_key
    {
        return Err(invariant(
            "outcome_claim_lifecycle_claim_identity_mismatch",
            format!(
                "claim {} payload differs from its envelope identity",
                claim_stage.stage_run_id
            ),
        ));
    }

    let claim_manifest_row = find_manifest(conn, &claim_stage.stage_run_id)?;
    let claim_receipt_row = find_commit_receipt(conn, &claim_stage.stage_run_id)?;
    let claim_manifest = claim_manifest_row.is_some();
    let (claim_receipt, claim_receipt_content_hash) =
        match (claim_manifest_row.as_ref(), claim_receipt_row.as_ref()) {
            (None, None) => (false, None),
            (Some(_), None) => {
                verify_staged_readback(
                    conn,
                    &claim_stage.stage_run_id,
                    StageDisposition::ExactReplay,
                )?;
                (false, None)
            }
            (Some(_), Some(receipt_row)) => {
                let staged = verify_staged_readback(
                    conn,
                    &claim_stage.stage_run_id,
                    StageDisposition::ExactReplay,
                )?;
                (
                    true,
                    Some(validate_lifecycle_receipt(conn, &staged, receipt_row)?),
                )
            }
            (None, Some(_)) => {
                return Err(invariant(
                    "outcome_claim_lifecycle_claim_receipt_without_manifest",
                    format!(
                        "claim {} has a receipt but no manifest",
                        claim_stage.stage_run_id
                    ),
                ));
            }
        };

    let outcome_envelope_row = find_envelope(conn, &claim_stage.planned_outcome_run_id)?;
    let outcome_manifest_row = find_manifest(conn, &claim_stage.planned_outcome_run_id)?;
    let outcome_receipt_row = find_commit_receipt(conn, &claim_stage.planned_outcome_run_id)?;
    let mut exact_binding = true;
    let outcome_stage = match outcome_envelope_row.as_ref() {
        None => None,
        Some(row) => {
            let envelope = validate_lifecycle_envelope(
                row,
                SubjectKind::OutcomeRun,
                OUTCOME_STAGE_PAYLOAD_SCHEMA,
                &claim_stage.planned_outcome_run_id,
                &claim_stage.logical_subject_key,
            )?;
            let stage: OutcomeStageInputPreimage = parse_canonical_payload(&envelope.payload_json)?;
            stage.validate().map_err(|error| {
                invariant(
                    "outcome_claim_lifecycle_outcome_payload_invalid",
                    format!(
                        "planned outcome {} typed payload is invalid: {error}",
                        claim_stage.planned_outcome_run_id
                    ),
                )
            })?;
            exact_binding = stage.stage_run_id == claim_stage.planned_outcome_run_id
                && stage.outcome_claim_id == claim_stage.stage_run_id
                && stage.logical_subject_key == claim_stage.logical_subject_key
                && stage.config_activation_run_id == claim_stage.config_activation_run_id
                && stage.config_hash == claim_stage.config_hash
                && stage.outcome_phase == claim_stage.due_binding.outcome_phase
                && stage.stored_due_date == claim_stage.due_binding.stored_due_date
                && stage.outcome_claim_due_binding_hash == claim_stage.due_binding_hash
                && stage.outcome_claim_provider_request_hash == claim_stage.provider_request_hash
                && claim_receipt_content_hash.as_deref()
                    == Some(stage.outcome_claim_receipt_content_hash.as_str());
            Some(stage)
        }
    };

    let outcome_manifest = outcome_manifest_row.is_some();
    let outcome_receipt = match (outcome_manifest_row.as_ref(), outcome_receipt_row.as_ref()) {
        (None, None) => false,
        (Some(_manifest_row), None) => {
            if outcome_envelope_row.is_none() {
                return Err(invariant(
                    "outcome_claim_lifecycle_outcome_manifest_without_envelope",
                    format!(
                        "planned outcome {} has a manifest but no envelope",
                        claim_stage.planned_outcome_run_id
                    ),
                ));
            }
            verify_staged_readback(
                conn,
                &claim_stage.planned_outcome_run_id,
                StageDisposition::ExactReplay,
            )?;
            false
        }
        (Some(_manifest_row), Some(receipt_row)) => {
            if outcome_envelope_row.is_none() {
                return Err(invariant(
                    "outcome_claim_lifecycle_outcome_receipt_without_envelope",
                    format!(
                        "planned outcome {} has a receipt but no envelope",
                        claim_stage.planned_outcome_run_id
                    ),
                ));
            }
            let staged = verify_staged_readback(
                conn,
                &claim_stage.planned_outcome_run_id,
                StageDisposition::ExactReplay,
            )?;
            validate_lifecycle_receipt(conn, &staged, receipt_row)?;
            true
        }
        (None, Some(_)) => {
            return Err(invariant(
                "outcome_claim_lifecycle_outcome_receipt_without_manifest",
                format!(
                    "planned outcome {} has a receipt but no manifest",
                    claim_stage.planned_outcome_run_id
                ),
            ));
        }
    };

    let class = classify_outcome_claim_artifact_matrix(&OutcomeClaimArtifactMatrix {
        claim_manifest,
        claim_receipt,
        outcome_envelope: outcome_envelope_row.is_some(),
        outcome_manifest,
        outcome_receipt,
        exact_claim_and_planned_run_binding: exact_binding,
    })?;
    Ok(OutcomeClaimLifecycle {
        class,
        claim_stage,
        claim_enveloped_at: claim_envelope.enveloped_at,
        claim_receipt_content_hash,
        outcome_stage,
    })
}

/// Shared BR-178 claim classifier used by both due anti-join and the claim
/// persistence guard. It never infers closure from a run ID alone: the exact
/// claim, planned outcome, manifest and receipt bindings are reconstructed.
pub(super) fn classify_outcome_claim_lifecycle(
    conn: &mut SqliteConnection,
    logical_subject_key: &str,
) -> RepositoryResult<Option<OutcomeClaimLifecycle>> {
    let rows = find_outcome_claim_envelopes(conn, logical_subject_key)?;
    let row_count = rows.len();
    let mut lifecycles = Vec::with_capacity(row_count);
    for row in &rows {
        lifecycles.push(classify_one_outcome_claim_lifecycle(conn, row)?);
    }
    for outcome_row in find_outcome_envelopes_for_logical_subject(conn, logical_subject_key)? {
        let envelope = validate_lifecycle_envelope(
            &outcome_row,
            SubjectKind::OutcomeRun,
            OUTCOME_STAGE_PAYLOAD_SCHEMA,
            &outcome_row.stage_run_id,
            logical_subject_key,
        )?;
        let stage: OutcomeStageInputPreimage = parse_canonical_payload(&envelope.payload_json)?;
        stage.validate().map_err(|error| {
            invariant(
                "outcome_claim_lifecycle_outcome_payload_invalid",
                format!(
                    "outcome {} typed payload is invalid: {error}",
                    stage.stage_run_id
                ),
            )
        })?;
        let exact_owner = lifecycles.iter().any(|lifecycle| {
            lifecycle.claim_id() == stage.outcome_claim_id
                && lifecycle.planned_outcome_run_id() == stage.stage_run_id
        });
        if !exact_owner {
            return Err(invariant(
                "outcome_claim_lifecycle_cross_binding",
                format!(
                    "outcome {} does not bind an exact claim/planned-run pair for logical subject {}",
                    stage.stage_run_id, logical_subject_key
                ),
            ));
        }
    }
    let open_indices = lifecycles
        .iter()
        .enumerate()
        .filter_map(|(index, lifecycle)| lifecycle.blocks_new_due().then_some(index))
        .collect::<Vec<_>>();
    if open_indices.len() > 1 {
        return Err(invariant(
            "outcome_claim_lifecycle_multiple_open",
            format!(
                "logical subject {logical_subject_key} has {} unclosed claims",
                open_indices.len()
            ),
        ));
    }
    if let Some(index) = open_indices.first().copied() {
        if index + 1 != row_count {
            return Err(invariant(
                "outcome_claim_lifecycle_open_not_latest",
                format!(
                    "logical subject {logical_subject_key} has a later claim after an unclosed claim"
                ),
            ));
        }
        return Ok(Some(lifecycles.remove(index)));
    }
    Ok(lifecycles.pop())
}

pub(super) fn outcome_claim_lifecycles_in_verified_snapshot(
    conn: &mut SqliteConnection,
) -> RepositoryResult<Vec<OutcomeClaimLifecycle>> {
    let keys = diesel::sql_query(
        "SELECT DISTINCT logical_subject_key
         FROM selection_v2_recovery_envelopes
         WHERE subject_kind='outcome_claim'
         ORDER BY logical_subject_key ASC",
    )
    .load::<LogicalSubjectKeyRow>(conn)?;
    let mut lifecycles = Vec::with_capacity(keys.len());
    for row in keys {
        if let Some(lifecycle) = classify_outcome_claim_lifecycle(conn, &row.logical_subject_key)? {
            lifecycles.push(lifecycle);
        }
    }
    lifecycles.sort_by(|left, right| {
        (left.claim_enveloped_at.as_str(), left.claim_id())
            .cmp(&(right.claim_enveloped_at.as_str(), right.claim_id()))
    });
    Ok(lifecycles)
}

fn find_unreceipted_logical_subject(
    conn: &mut SqliteConnection,
    subject_kind: SubjectKind,
    logical_subject_key: &str,
) -> RepositoryResult<Option<String>> {
    Ok(diesel::sql_query(
        "SELECT e.stage_run_id AS subject_id
         FROM selection_v2_recovery_envelopes e
         WHERE e.subject_kind=?
           AND e.logical_subject_key=?
           AND NOT EXISTS (
               SELECT 1
               FROM selection_v2_commit_receipts r
               WHERE r.subject_kind=e.subject_kind
                 AND r.subject_id=e.stage_run_id
           )
         ORDER BY e.enveloped_at ASC, e.stage_run_id ASC
         LIMIT 1",
    )
    .bind::<Text, _>(subject_kind.as_str())
    .bind::<Text, _>(logical_subject_key)
    .get_result::<SubjectIdRow>(conn)
    .optional()?
    .map(|row| row.subject_id))
}

fn load_latest_receipted_logical_subject(
    conn: &mut SqliteConnection,
    session: &mut LockedSelectionAuditSession<'_>,
    subject_kind: SubjectKind,
    logical_subject_key: &str,
) -> RepositoryResult<Option<LatestReceiptedLogicalSubject>> {
    let subject_id =
        find_latest_receipted_logical_subject_id(conn, subject_kind, logical_subject_key)?;
    let Some(subject_id) = subject_id else {
        return Ok(None);
    };

    let staged = verify_staged_readback(conn, &subject_id, StageDisposition::ExactReplay)?;
    let manifest = find_manifest(conn, &subject_id)?.ok_or_else(|| {
        invariant(
            "logical_subject_receipt_manifest_missing",
            format!("receipted subject {subject_id} has no manifest"),
        )
    })?;
    let receipt = find_commit_receipt(conn, &subject_id)?.ok_or_else(|| {
        invariant(
            "logical_subject_receipt_missing",
            format!("latest receipted subject {subject_id} disappeared"),
        )
    })?;
    let rebuilt_receipt = rebuild_commit_receipt(&receipt)?;
    let receipt_content_hash = hash(&rebuilt_receipt)?;
    if receipt.content_hash != receipt_content_hash
        || receipt.subject_kind != subject_kind.as_str()
        || receipt.logical_subject_key != logical_subject_key
        || receipt.in_memory_payload_hash != staged.in_memory_payload_hash
        || receipt.recovery_envelope_content_hash != staged.recovery_envelope_content_hash
        || receipt.run_manifest_content_hash != staged.run_manifest_content_hash
        || receipt.staged_db_content_hash != staged.staged_db_content_hash
        || receipt.prepared_audit_hash != manifest.prepared_record_hash
    {
        return Err(invariant(
            "logical_subject_receipt_readback_mismatch",
            "latest logical-subject receipt does not exactly bind its staged manifest",
        ));
    }

    let prepared_content = PreparedAuditContentPreimage {
        domain: DOMAIN_PREPARED_AUDIT.into(),
        subject_kind,
        subject_id: subject_id.clone(),
        logical_subject_key: logical_subject_key.into(),
        recovery_envelope_content_hash: staged.recovery_envelope_content_hash.clone(),
        in_memory_payload_hash: staged.in_memory_payload_hash.clone(),
    };
    let prepared_record = load_persisted_audit_record(
        session,
        prepared_phase(subject_kind),
        &subject_id,
        &hash(&prepared_content)?,
    )?;
    if prepared_record.record_hash != receipt.prepared_audit_hash {
        return Err(invariant(
            "logical_subject_prepared_audit_mismatch",
            "latest logical-subject receipt does not bind the exact Prepared audit record",
        ));
    }

    let committed_content = CommittedAuditContentPreimage {
        domain: DOMAIN_COMMITTED_AUDIT.into(),
        subject_kind,
        subject_id: subject_id.clone(),
        logical_subject_key: logical_subject_key.into(),
        recovery_envelope_content_hash: staged.recovery_envelope_content_hash,
        prepared_record_hash: prepared_record.record_hash,
        run_manifest_content_hash: staged.run_manifest_content_hash,
        staged_db_content_hash: staged.staged_db_content_hash,
    };
    let committed_record = load_persisted_audit_record(
        session,
        committed_phase(subject_kind),
        &subject_id,
        &hash(&committed_content)?,
    )?;
    if committed_record.record_hash != receipt.committed_audit_hash
        || utc_nanos(committed_record.recorded_at.with_timezone(&Utc)) != receipt.committed_at
    {
        return Err(invariant(
            "logical_subject_committed_audit_mismatch",
            "latest logical-subject receipt does not bind the exact Committed audit record and time",
        ));
    }

    Ok(Some(LatestReceiptedLogicalSubject {
        subject_kind,
        subject_id,
        logical_subject_key: logical_subject_key.into(),
        run_status: parse_run_status(&manifest.run_status)?,
        committed_at_rfc3339_nanos_utc: receipt.committed_at,
        manifest_content_hash: manifest.manifest_content_hash,
        receipt_content_hash,
        prepared_audit_hash: receipt.prepared_audit_hash,
        committed_audit_hash: receipt.committed_audit_hash,
    }))
}

fn find_latest_receipted_logical_subject_id(
    conn: &mut SqliteConnection,
    subject_kind: SubjectKind,
    logical_subject_key: &str,
) -> RepositoryResult<Option<String>> {
    Ok(diesel::sql_query(
        "SELECT r.subject_id
         FROM selection_v2_commit_receipts r
         INNER JOIN selection_v2_run_stages s
                 ON s.subject_kind=r.subject_kind
                AND s.subject_id=r.subject_id
         WHERE r.subject_kind=?
           AND r.logical_subject_key=?
           AND s.logical_subject_key=r.logical_subject_key
         ORDER BY r.committed_at DESC, r.subject_id DESC
         LIMIT 1",
    )
    .bind::<Text, _>(subject_kind.as_str())
    .bind::<Text, _>(logical_subject_key)
    .get_result::<SubjectIdRow>(conn)
    .optional()?
    .map(|row| row.subject_id))
}

/// Return the latest receipted manifest status from an already fully verified
/// and pinned snapshot. The shared selector owns the required
/// `committed_at DESC, subject_id DESC` tie-break semantics.
pub(super) fn latest_receipted_status_in_verified_snapshot(
    conn: &mut SqliteConnection,
    subject_kind: SubjectKind,
    logical_subject_key: &str,
) -> RepositoryResult<Option<RunStatus>> {
    let Some(subject_id) =
        find_latest_receipted_logical_subject_id(conn, subject_kind, logical_subject_key)?
    else {
        return Ok(None);
    };
    let manifest = find_manifest(conn, &subject_id)?.ok_or_else(|| {
        invariant(
            "logical_subject_receipt_manifest_missing",
            format!("receipted subject {subject_id} has no manifest"),
        )
    })?;
    if manifest.subject_kind != subject_kind.as_str()
        || manifest.logical_subject_key != logical_subject_key
    {
        return Err(invariant(
            "logical_subject_receipt_manifest_identity_mismatch",
            format!("receipted subject {subject_id} manifest identity differs from the selector"),
        ));
    }
    Ok(Some(parse_run_status(&manifest.run_status)?))
}

fn find_manifest(
    conn: &mut SqliteConnection,
    subject_id: &str,
) -> RepositoryResult<Option<ManifestRow>> {
    Ok(diesel::sql_query(
        "SELECT subject_kind, subject_id, in_memory_payload_hash,
                prepared_record_hash, expected_staged_row_count,
                staged_db_content_hash, recovery_envelope_content_hash,
                logical_subject_key, run_status, source_fact_key,
                config_activation_run_id, config_hash, config_snapshot_json_hash,
                config_activation_content_hash, config_activation_file_content_hash,
                config_effective_from, artifact_valid_from, artifact_expires_at,
                executable_revision, legacy_cutover_snapshot_hash,
                generation_market_date, aggregator_observed_at,
                ingress_source_batch_content_hash, outcome_phase, stored_due_date,
                outcome_claim_id, planned_outcome_run_id,
                outcome_claim_receipt_content_hash, outcome_claim_due_binding_hash,
                outcome_claim_provider_request_hash,
                staged_at, manifest_content_hash
         FROM selection_v2_run_stages WHERE subject_id=?",
    )
    .bind::<Text, _>(subject_id)
    .get_result::<ManifestRow>(conn)
    .optional()?)
}

fn find_config_hash(
    conn: &mut SqliteConnection,
    config_hash: &str,
) -> RepositoryResult<Option<ManifestRow>> {
    Ok(diesel::sql_query(
        "SELECT subject_kind, subject_id, in_memory_payload_hash,
                prepared_record_hash, expected_staged_row_count,
                staged_db_content_hash, recovery_envelope_content_hash,
                logical_subject_key, run_status, source_fact_key,
                config_activation_run_id, config_hash, config_snapshot_json_hash,
                config_activation_content_hash, config_activation_file_content_hash,
                config_effective_from, artifact_valid_from, artifact_expires_at,
                executable_revision, legacy_cutover_snapshot_hash,
                generation_market_date, aggregator_observed_at,
                ingress_source_batch_content_hash, outcome_phase, stored_due_date,
                outcome_claim_id, planned_outcome_run_id,
                outcome_claim_receipt_content_hash, outcome_claim_due_binding_hash,
                outcome_claim_provider_request_hash,
                staged_at, manifest_content_hash
         FROM selection_v2_run_stages
         WHERE subject_kind='config_activation' AND config_hash=?",
    )
    .bind::<Text, _>(config_hash)
    .get_result::<ManifestRow>(conn)
    .optional()?)
}

#[allow(
    dead_code,
    reason = "BR-183 keeps config reuse verification dormant until selection-v2 activation"
)]
fn load_config_activation_evidence(
    conn: &mut SqliteConnection,
    session: &mut LockedSelectionAuditSession<'_>,
    manifest_row: ManifestRow,
) -> RepositoryResult<ConfigActivationEvidence> {
    let manifest = rebuild_manifest(&manifest_row)?;
    manifest.validate_kind_matrix().map_err(|error| {
        SelectionV2RepositoryError::Canonical(format!(
            "stored config activation manifest matrix invalid: {error}"
        ))
    })?;
    let manifest_content_hash = hash(&manifest)?;
    if manifest.subject_kind != SubjectKind::ConfigActivation
        || manifest.subject_id != manifest_row.subject_id
        || manifest.config_activation_run_id.as_deref() != Some(manifest_row.subject_id.as_str())
        || manifest.config_hash.as_deref() != Some(manifest_row.config_hash.as_str())
        || manifest_content_hash != manifest_row.manifest_content_hash
    {
        return Err(invariant(
            "config_reuse_manifest_readback_mismatch",
            "config activation manifest columns do not reproduce its exact identity and content hash",
        ));
    }

    let envelope_row = find_envelope(conn, &manifest_row.subject_id)?.ok_or_else(|| {
        invariant(
            "config_reuse_envelope_missing",
            format!(
                "config activation {} has a manifest but no recovery envelope",
                manifest_row.subject_id
            ),
        )
    })?;
    let envelope = rebuild_envelope(&envelope_row)?;
    let envelope_content_hash = hash(&envelope)?;
    if envelope.subject_kind != SubjectKind::ConfigActivation
        || envelope.stage_run_id != manifest_row.subject_id
        || envelope.logical_subject_key != manifest.logical_subject_key
        || envelope.in_memory_payload_hash != manifest.in_memory_payload_hash
        || envelope.config_activation_run_id != manifest_row.subject_id
        || envelope.config_hash != manifest_row.config_hash
        || envelope_content_hash != envelope_row.content_hash
        || envelope_content_hash != manifest.recovery_envelope_content_hash
        || canonical_payload_hash(&envelope.payload_json)? != envelope.payload_json_hash
    {
        return Err(invariant(
            "config_reuse_envelope_readback_mismatch",
            "config activation recovery envelope does not exactly bind the stored manifest",
        ));
    }

    let prepared_content = PreparedAuditContentPreimage {
        domain: DOMAIN_PREPARED_AUDIT.into(),
        subject_kind: SubjectKind::ConfigActivation,
        subject_id: manifest_row.subject_id.clone(),
        logical_subject_key: manifest.logical_subject_key.clone(),
        recovery_envelope_content_hash: envelope_content_hash,
        in_memory_payload_hash: envelope.in_memory_payload_hash,
    };
    let prepared_record = load_persisted_audit_record(
        session,
        SelectionAuditPhase::V2ConfigActivationPrepared,
        &manifest_row.subject_id,
        &hash(&prepared_content)?,
    )?;
    if prepared_record.record_hash != manifest.prepared_record_hash {
        return Err(invariant(
            "config_reuse_prepared_audit_mismatch",
            "config activation manifest does not bind the exact Prepared audit record",
        ));
    }

    let receipt = match find_commit_receipt(conn, &manifest_row.subject_id)? {
        None => None,
        Some(receipt_row) => {
            let rebuilt_receipt = rebuild_commit_receipt(&receipt_row)?;
            let receipt_content_hash = hash(&rebuilt_receipt)?;
            if receipt_content_hash != receipt_row.content_hash
                || rebuilt_receipt.subject_kind != SubjectKind::ConfigActivation
                || rebuilt_receipt.subject_id != manifest_row.subject_id
                || rebuilt_receipt.logical_subject_key != manifest.logical_subject_key
                || rebuilt_receipt.in_memory_payload_hash != manifest.in_memory_payload_hash
                || rebuilt_receipt.recovery_envelope_content_hash
                    != manifest.recovery_envelope_content_hash
                || rebuilt_receipt.prepared_audit_hash != prepared_record.record_hash
                || rebuilt_receipt.run_manifest_content_hash != manifest_content_hash
                || rebuilt_receipt.staged_db_content_hash != manifest.staged_db_content_hash
            {
                return Err(invariant(
                    "config_reuse_receipt_readback_mismatch",
                    "config activation receipt does not exactly bind the envelope, Prepared record, and manifest",
                ));
            }

            let committed_content = CommittedAuditContentPreimage {
                domain: DOMAIN_COMMITTED_AUDIT.into(),
                subject_kind: SubjectKind::ConfigActivation,
                subject_id: manifest_row.subject_id.clone(),
                logical_subject_key: manifest.logical_subject_key.clone(),
                recovery_envelope_content_hash: manifest.recovery_envelope_content_hash.clone(),
                prepared_record_hash: prepared_record.record_hash.clone(),
                run_manifest_content_hash: manifest_content_hash.clone(),
                staged_db_content_hash: manifest.staged_db_content_hash.clone(),
            };
            let committed_record = load_persisted_audit_record(
                session,
                SelectionAuditPhase::V2ConfigActivationCommitted,
                &manifest_row.subject_id,
                &hash(&committed_content)?,
            )?;
            if committed_record.record_hash != rebuilt_receipt.committed_audit_hash
                || utc_nanos(committed_record.recorded_at.with_timezone(&Utc))
                    != rebuilt_receipt.committed_at_rfc3339_nanos_utc
            {
                return Err(invariant(
                    "config_reuse_committed_audit_mismatch",
                    "config activation receipt does not bind the exact Committed audit record and time",
                ));
            }
            Some(ConfigActivationReceiptEvidence {
                content_hash: receipt_content_hash,
                committed_audit_hash: committed_record.record_hash,
            })
        }
    };

    Ok(ConfigActivationEvidence {
        activation_run_id: manifest_row.subject_id,
        manifest_content_hash,
        prepared_audit_hash: prepared_record.record_hash,
        receipt,
    })
}

fn load_outcome_authority(
    conn: &mut SqliteConnection,
    stage: &OutcomeStageInputPreimage,
) -> RepositoryResult<OutcomeAuthorityRow> {
    let mut reader = DieselExactSelectionSnapshotReader { conn };
    load_outcome_authority_with_reader(&mut reader, stage)
}

fn load_outcome_authority_with_reader<R: ExactSelectionSnapshotReader>(
    reader: &mut R,
    stage: &OutcomeStageInputPreimage,
) -> RepositoryResult<OutcomeAuthorityRow> {
    validate_receipted_outcome_claim_with_reader(reader, stage)?;
    let row = reader
        .outcome_authority_row(&stage.sample_key)?
        .ok_or_else(|| {
            invariant(
                "outcome_authority_unavailable",
                format!(
                    "sample {} is absent or its generation/config activation is not receipted",
                    stage.sample_key
                ),
            )
        })?;
    validate_outcome_authority_row(stage, &row)?;
    Ok(row)
}

fn query_outcome_authority_row(
    conn: &mut SqliteConnection,
    sample_key: &str,
) -> RepositoryResult<Option<OutcomeAuthorityRow>> {
    Ok(diesel::sql_query(
        "SELECT s.sample_key, s.event_id, s.chain_id, s.canonical_stock_code,
                s.relation_schema_version, s.feature_version,
                s.evaluation_market_date, s.config_activation_run_id,
                s.config_hash, s.generation_run_id, s.t0_due_date,
                s.d1_due_date, s.d2_due_date, s.d3_due_date, s.d4_due_date,
                s.d5_due_date, s.calendar_version, s.calendar_hash,
                s.trading_date_vector_json, s.trading_date_vector_hash,
                gr.content_hash AS generation_receipt_content_hash,
                ar.content_hash AS activation_receipt_content_hash
         FROM selection_samples s
         INNER JOIN selection_v2_run_stages gm
                 ON gm.subject_kind='generation_run'
                AND gm.subject_id=s.generation_run_id
                AND gm.config_activation_run_id=s.config_activation_run_id
                AND gm.config_hash=s.config_hash
         INNER JOIN selection_v2_commit_receipts gr
                 ON gr.subject_kind='generation_run'
                AND gr.subject_id=gm.subject_id
                AND gr.run_manifest_content_hash=gm.manifest_content_hash
         INNER JOIN selection_v2_run_stages am
                 ON am.subject_kind='config_activation'
                AND am.subject_id=s.config_activation_run_id
                AND am.config_activation_run_id=s.config_activation_run_id
                AND am.config_hash=s.config_hash
         INNER JOIN selection_v2_commit_receipts ar
                 ON ar.subject_kind='config_activation'
                AND ar.subject_id=am.subject_id
                AND ar.run_manifest_content_hash=am.manifest_content_hash
         WHERE s.sample_key=?
         LIMIT 1",
    )
    .bind::<Text, _>(sample_key)
    .get_result::<OutcomeAuthorityRow>(conn)
    .optional()?)
}

fn rusqlite_query_outcome_authority_row(
    transaction: &rusqlite::Transaction<'_>,
    sample_key: &str,
) -> RepositoryResult<Option<OutcomeAuthorityRow>> {
    use rusqlite::OptionalExtension as _;
    transaction
        .query_row(
            "SELECT s.sample_key, s.event_id, s.chain_id, s.canonical_stock_code,
                    s.relation_schema_version, s.feature_version,
                    s.evaluation_market_date, s.config_activation_run_id,
                    s.config_hash, s.generation_run_id, s.t0_due_date,
                    s.d1_due_date, s.d2_due_date, s.d3_due_date, s.d4_due_date,
                    s.d5_due_date, s.calendar_version, s.calendar_hash,
                    s.trading_date_vector_json, s.trading_date_vector_hash,
                    gr.content_hash AS generation_receipt_content_hash,
                    ar.content_hash AS activation_receipt_content_hash
             FROM selection_samples s
             INNER JOIN selection_v2_run_stages gm
                     ON gm.subject_kind='generation_run'
                    AND gm.subject_id=s.generation_run_id
                    AND gm.config_activation_run_id=s.config_activation_run_id
                    AND gm.config_hash=s.config_hash
             INNER JOIN selection_v2_commit_receipts gr
                     ON gr.subject_kind='generation_run'
                    AND gr.subject_id=gm.subject_id
                    AND gr.run_manifest_content_hash=gm.manifest_content_hash
             INNER JOIN selection_v2_run_stages am
                     ON am.subject_kind='config_activation'
                    AND am.subject_id=s.config_activation_run_id
                    AND am.config_activation_run_id=s.config_activation_run_id
                    AND am.config_hash=s.config_hash
             INNER JOIN selection_v2_commit_receipts ar
                     ON ar.subject_kind='config_activation'
                    AND ar.subject_id=am.subject_id
                    AND ar.run_manifest_content_hash=am.manifest_content_hash
             WHERE s.sample_key=?1
             LIMIT 1",
            [sample_key],
            |row| {
                Ok(OutcomeAuthorityRow {
                    sample_key: row.get(0)?,
                    event_id: row.get(1)?,
                    chain_id: row.get(2)?,
                    canonical_stock_code: row.get(3)?,
                    relation_schema_version: row.get(4)?,
                    feature_version: row.get(5)?,
                    evaluation_market_date: row.get(6)?,
                    config_activation_run_id: row.get(7)?,
                    config_hash: row.get(8)?,
                    generation_run_id: row.get(9)?,
                    t0_due_date: row.get(10)?,
                    d1_due_date: row.get(11)?,
                    d2_due_date: row.get(12)?,
                    d3_due_date: row.get(13)?,
                    d4_due_date: row.get(14)?,
                    d5_due_date: row.get(15)?,
                    calendar_version: row.get(16)?,
                    calendar_hash: row.get(17)?,
                    trading_date_vector_json: row.get(18)?,
                    trading_date_vector_hash: row.get(19)?,
                    generation_receipt_content_hash: row.get(20)?,
                    activation_receipt_content_hash: row.get(21)?,
                })
            },
        )
        .optional()
        .map_err(SelectionV2RepositoryError::from)
}

#[cfg(test)]
fn validate_receipted_outcome_claim(
    conn: &mut SqliteConnection,
    stage: &OutcomeStageInputPreimage,
) -> RepositoryResult<()> {
    let mut reader = DieselExactSelectionSnapshotReader { conn };
    validate_receipted_outcome_claim_with_reader(&mut reader, stage)
}

fn validate_receipted_outcome_claim_with_reader<R: ExactSelectionSnapshotReader>(
    reader: &mut R,
    stage: &OutcomeStageInputPreimage,
) -> RepositoryResult<()> {
    let manifest_row = reader
        .find_manifest(&stage.outcome_claim_id)?
        .ok_or_else(|| {
            invariant(
                "outcome_claim_manifest_missing",
                format!(
                    "outcome run {} requires claim {}",
                    stage.stage_run_id, stage.outcome_claim_id
                ),
            )
        })?;
    let manifest = rebuild_manifest(&manifest_row)?;
    manifest.validate_kind_matrix().map_err(|error| {
        invariant(
            "outcome_claim_manifest_invalid",
            format!("claim manifest matrix invalid: {error}"),
        )
    })?;
    let manifest_content_hash = hash(&manifest)?;
    if manifest_row.manifest_content_hash != manifest_content_hash
        || manifest.subject_kind != SubjectKind::OutcomeClaim
        || manifest.subject_id != stage.outcome_claim_id
        || manifest.outcome_claim_id.as_deref() != Some(stage.outcome_claim_id.as_str())
        || manifest.planned_outcome_run_id.as_deref() != Some(stage.stage_run_id.as_str())
        || manifest.logical_subject_key != stage.logical_subject_key
        || manifest.config_activation_run_id.as_deref()
            != Some(stage.config_activation_run_id.as_str())
        || manifest.config_hash.as_deref() != Some(stage.config_hash.as_str())
        || manifest.outcome_phase != Some(stage.outcome_phase)
        || manifest.stored_due_date.as_deref() != Some(stage.stored_due_date.as_str())
        || manifest.outcome_claim_due_binding_hash.as_deref()
            != Some(stage.outcome_claim_due_binding_hash.as_str())
        || manifest.outcome_claim_provider_request_hash.as_deref()
            != Some(stage.outcome_claim_provider_request_hash.as_str())
    {
        return Err(invariant(
            "outcome_claim_manifest_binding_mismatch",
            "outcome run does not bind the exact typed claim manifest",
        ));
    }

    let receipt_row = reader
        .find_commit_receipt(&stage.outcome_claim_id)?
        .ok_or_else(|| {
            invariant(
                "outcome_claim_receipt_missing",
                format!("claim {} is not receipted", stage.outcome_claim_id),
            )
        })?;
    let receipt = rebuild_commit_receipt(&receipt_row)?;
    let receipt_content_hash = hash(&receipt)?;
    if receipt_row.content_hash != receipt_content_hash
        || receipt_content_hash != stage.outcome_claim_receipt_content_hash
        || receipt.subject_kind != SubjectKind::OutcomeClaim
        || receipt.subject_id != stage.outcome_claim_id
        || receipt.logical_subject_key != stage.logical_subject_key
        || receipt.run_manifest_content_hash != manifest_content_hash
    {
        return Err(invariant(
            "outcome_claim_receipt_binding_mismatch",
            "outcome run does not bind the exact receipted claim",
        ));
    }
    Ok(())
}

fn validate_outcome_authority_row(
    stage: &OutcomeStageInputPreimage,
    authority: &OutcomeAuthorityRow,
) -> RepositoryResult<()> {
    let sample_key_preimage = SampleKeyPreimage {
        domain: crate::selection::schema_v2::DOMAIN_SAMPLE_KEY.into(),
        event_id: authority.event_id.clone(),
        chain_id: authority.chain_id.clone(),
        stock_code: authority.canonical_stock_code.clone(),
        relation_schema_version: authority.relation_schema_version.clone(),
        feature_version: authority.feature_version.clone(),
        evaluation_market_date: authority.evaluation_market_date.clone(),
    };
    let reconstructed_sample_key = hash(&sample_key_preimage)?;
    if authority.sample_key != reconstructed_sample_key
        || stage.sample_key != authority.sample_key
        || stage.sample_key_preimage != sample_key_preimage
    {
        return Err(invariant(
            "outcome_authority_sample_key_mismatch",
            "outcome request does not bind the exact receipted sample-key preimage",
        ));
    }
    if stage.config_activation_run_id != authority.config_activation_run_id
        || stage.config_hash != authority.config_hash
    {
        return Err(invariant(
            "outcome_authority_config_mismatch",
            "outcome config lineage differs from the receipted sample and activation",
        ));
    }
    let vector: OutcomeTradingDateVectorPreimage =
        serde_json::from_str(&authority.trading_date_vector_json).map_err(|error| {
            invariant(
                "outcome_authority_trading_date_vector_invalid",
                format!("stored trading-date vector is not typed canonical JSON: {error}"),
            )
        })?;
    vector.validate().map_err(|error| {
        invariant(
            "outcome_authority_trading_date_vector_invalid",
            error.to_string(),
        )
    })?;
    if canonical(&vector)? != authority.trading_date_vector_json
        || hash(&vector)? != authority.trading_date_vector_hash
        || [
            authority.t0_due_date.as_str(),
            authority.d1_due_date.as_str(),
            authority.d2_due_date.as_str(),
            authority.d3_due_date.as_str(),
            authority.d4_due_date.as_str(),
            authority.d5_due_date.as_str(),
        ] != [
            vector.t0.as_str(),
            vector.d1.as_str(),
            vector.d2.as_str(),
            vector.d3.as_str(),
            vector.d4.as_str(),
            vector.d5.as_str(),
        ]
        || authority.evaluation_market_date != vector.t0
    {
        return Err(invariant(
            "outcome_authority_trading_date_vector_mismatch",
            "stored sample dates must equal the exact canonical full T0/D1/D2/D3/D4/D5 vector",
        ));
    }
    if let Some(attempt) = stage.outcome_attempt_rows.first() {
        if let Some(request) = crate::selection::schema_v2::validate_request_evidence_columns(
            attempt.request_hash.as_deref(),
            attempt.request_evidence_json.as_deref(),
            attempt.request_evidence_hash.as_deref(),
            Some(RequestKind::OutcomeMarketEvidence),
        )
        .map_err(|error| invariant("outcome_authority_request_invalid", error.to_string()))?
        {
            let parameters: OutcomeMarketRequestParametersPreimage =
                serde_json::from_str(&request.parameters_json).map_err(|error| {
                    invariant(
                        "outcome_authority_request_invalid",
                        format!("typed outcome parameters are invalid: {error}"),
                    )
                })?;
            let applicable_dates = parameters
                .trading_date_vector
                .applicable_dates(stage.outcome_phase)
                .map_err(|error| {
                    invariant("outcome_authority_request_invalid", error.to_string())
                })?;
            if parameters.calendar_version != authority.calendar_version
                || parameters.calendar_hash != authority.calendar_hash
                || parameters.trading_date_vector_hash != authority.trading_date_vector_hash
                || parameters.trading_date_vector != vector
                || parameters.applicable_trading_dates != applicable_dates
            {
                return Err(invariant(
                    "outcome_authority_request_schedule_mismatch",
                    "outcome request must bind the sample calendar, complete vector, and exact phase prefix",
                ));
            }
        }
    } else if stage.planned_run_status != RunStatus::ExpectedWait {
        return Err(invariant(
            "outcome_authority_attempt_missing",
            "only ExpectedWait may persist without a provider attempt",
        ));
    }
    let due_date = match stage.outcome_phase {
        OutcomePhase::T0Close => &authority.t0_due_date,
        OutcomePhase::D1Settled => &authority.d1_due_date,
        OutcomePhase::D3Settled => &authority.d3_due_date,
        OutcomePhase::D5Settled => &authority.d5_due_date,
    };
    if stage.stored_due_date != *due_date {
        return Err(invariant(
            "outcome_authority_due_date_mismatch",
            format!(
                "phase={} requires pinned due date {}; request supplied {}",
                stage.outcome_phase.as_str(),
                due_date,
                stage.stored_due_date
            ),
        ));
    }
    if authority.generation_run_id.is_empty()
        || authority.generation_receipt_content_hash.is_empty()
        || authority.activation_receipt_content_hash.is_empty()
    {
        return Err(invariant(
            "outcome_authority_receipt_identity_missing",
            "outcome authority requires non-empty generation and activation receipt identities",
        ));
    }
    Ok(())
}

fn find_commit_receipt(
    conn: &mut SqliteConnection,
    subject_id: &str,
) -> RepositoryResult<Option<CommitReceiptRow>> {
    Ok(diesel::sql_query(
        "SELECT subject_kind, subject_id, logical_subject_key,
                in_memory_payload_hash, recovery_envelope_content_hash,
                prepared_audit_hash, run_manifest_content_hash,
                staged_db_content_hash, committed_audit_hash, committed_at,
                content_hash
         FROM selection_v2_commit_receipts WHERE subject_id=?",
    )
    .bind::<Text, _>(subject_id)
    .get_result::<CommitReceiptRow>(conn)
    .optional()?)
}

fn rebuild_commit_receipt(
    row: &CommitReceiptRow,
) -> RepositoryResult<CommitReceiptContentPreimage> {
    Ok(CommitReceiptContentPreimage {
        domain: DOMAIN_COMMIT_RECEIPT.into(),
        subject_kind: parse_subject_kind(&row.subject_kind)?,
        subject_id: row.subject_id.clone(),
        logical_subject_key: row.logical_subject_key.clone(),
        in_memory_payload_hash: row.in_memory_payload_hash.clone(),
        recovery_envelope_content_hash: row.recovery_envelope_content_hash.clone(),
        prepared_audit_hash: row.prepared_audit_hash.clone(),
        run_manifest_content_hash: row.run_manifest_content_hash.clone(),
        staged_db_content_hash: row.staged_db_content_hash.clone(),
        committed_audit_hash: row.committed_audit_hash.clone(),
        committed_at_rfc3339_nanos_utc: row.committed_at.clone(),
    })
}

fn rebuild_envelope(
    row: &EnvelopeRow,
) -> RepositoryResult<SelectionRecoveryEnvelopeRowContentPreimage> {
    Ok(SelectionRecoveryEnvelopeRowContentPreimage {
        domain: DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
        stage_run_id: row.stage_run_id.clone(),
        subject_kind: parse_subject_kind(&row.subject_kind)?,
        logical_subject_key: row.logical_subject_key.clone(),
        payload_schema: row.payload_schema.clone(),
        payload_json: row.payload_json.clone(),
        payload_json_hash: row.payload_json_hash.clone(),
        in_memory_payload_hash: row.in_memory_payload_hash.clone(),
        config_activation_run_id: row.config_activation_run_id.clone(),
        config_hash: row.config_hash.clone(),
        enveloped_at: row.enveloped_at.clone(),
    })
}

fn verify_persisted_envelope(
    conn: &mut SqliteConnection,
    expected: &ValidatedRecoveryEnvelope,
) -> RepositoryResult<SelectionRecoveryEnvelopeRowContentPreimage> {
    let row = find_envelope(conn, &expected.envelope.stage_run_id)?.ok_or_else(|| {
        invariant(
            "envelope_readback_missing",
            format!(
                "recovery envelope disappeared after commit: {}",
                expected.envelope.stage_run_id
            ),
        )
    })?;
    let rebuilt = rebuild_envelope(&row)?;
    let rebuilt_hash = hash(&rebuilt)?;
    if rebuilt != expected.envelope
        || row.content_hash != expected.content_hash
        || rebuilt_hash != expected.content_hash
    {
        return Err(invariant(
            "envelope_readback_mismatch",
            "persisted envelope columns/content hash differ from validated typed input",
        ));
    }
    if canonical_payload_hash(&rebuilt.payload_json)? != rebuilt.payload_json_hash {
        return Err(invariant(
            "envelope_payload_rehash_mismatch",
            "persisted envelope payload bytes do not reproduce payload_json_hash",
        ));
    }
    Ok(rebuilt)
}

fn verify_staged_readback(
    conn: &mut SqliteConnection,
    subject_id: &str,
    disposition: StageDisposition,
) -> RepositoryResult<StagedRunReceipt> {
    let mut reader = DieselExactSelectionSnapshotReader { conn };
    verify_staged_readback_with_reader(&mut reader, subject_id, disposition)
}

fn verify_staged_readback_with_reader<R: ExactSelectionSnapshotReader>(
    reader: &mut R,
    subject_id: &str,
    disposition: StageDisposition,
) -> RepositoryResult<StagedRunReceipt> {
    let envelope_row = reader.find_envelope(subject_id)?.ok_or_else(|| {
        invariant(
            "stage_envelope_missing",
            format!("no recovery envelope for subject_id={subject_id}"),
        )
    })?;
    let envelope = rebuild_envelope(&envelope_row)?;
    let envelope_hash = hash(&envelope)?;
    if envelope_hash != envelope_row.content_hash {
        return Err(invariant(
            "envelope_readback_rehash_mismatch",
            "recovery envelope columns do not reproduce content_hash",
        ));
    }
    if canonical_payload_hash(&envelope.payload_json)? != envelope.payload_json_hash {
        return Err(invariant(
            "envelope_payload_rehash_mismatch",
            "recovery envelope payload bytes do not reproduce payload_json_hash",
        ));
    }
    let manifest_row = reader.find_manifest(subject_id)?.ok_or_else(|| {
        invariant(
            "stage_manifest_missing",
            format!("no run manifest for subject_id={subject_id}"),
        )
    })?;
    let subject_kind = parse_subject_kind(&manifest_row.subject_kind)?;
    if subject_kind != envelope.subject_kind {
        return Err(invariant(
            "stage_subject_kind_mismatch",
            "envelope and manifest subject kinds differ",
        ));
    }

    let (run_payload, domain_row_hashes) = match subject_kind {
        SubjectKind::ConfigActivation => {
            let stage: ConfigActivationStageInputPreimage =
                parse_canonical_payload(&envelope.payload_json)?;
            if stage.domain != DOMAIN_CONFIG_ACTIVATION_STAGE
                || stage.stage_run_id != subject_id
                || stage.logical_subject_key != envelope.logical_subject_key
            {
                return Err(invariant(
                    "config_payload_readback_identity_mismatch",
                    "stored config activation payload identity differs from envelope",
                ));
            }
            stage.config_snapshot.validate().map_err(|error| {
                SelectionV2RepositoryError::Canonical(format!(
                    "stored config snapshot invalid: {error}"
                ))
            })?;
            let payload = RunPayloadPreimage {
                domain: DOMAIN_CONFIG_ACTIVATION_PAYLOAD.into(),
                subject_kind,
                subject_id: subject_id.into(),
                logical_subject_key: stage.logical_subject_key.clone(),
                source_fact_key: None,
                config_activation_run_id: subject_id.into(),
                config_hash: stage.config_hash.clone(),
                config_snapshot_json_hash: Some(stage.config_snapshot_json_hash.clone()),
                config_activation_content_hash: Some(stage.activation_content_hash.clone()),
                config_activation_file_content_hash: Some(
                    stage.activation.activation_file_content_hash.clone(),
                ),
                config_effective_from_rfc3339_nanos_utc: Some(
                    stage.activation.effective_from_rfc3339_nanos_utc.clone(),
                ),
                artifact_valid_from: Some(stage.activation.artifact_valid_from.clone()),
                artifact_expires_at: Some(stage.activation.artifact_expires_at.clone()),
                executable_revision: Some(stage.activation.executable_revision.clone()),
                legacy_cutover_snapshot_hash: Some(stage.legacy_cutover_snapshot_hash.clone()),
                generation_market_date: None,
                aggregator_observed_at_rfc3339_nanos_utc: None,
                ingress_source_batch_content_hash: None,
                outcome_phase: None,
                stored_due_date: None,
                outcome_claim_id: None,
                planned_outcome_run_id: None,
                outcome_claim_receipt_content_hash: None,
                outcome_claim_due_binding_hash: None,
                outcome_claim_provider_request_hash: None,
                rows: Vec::new(),
            };
            (payload, Vec::new())
        }
        SubjectKind::IngressRun => {
            let mut stage: SourceIngressStageInputPreimage =
                parse_canonical_payload(&envelope.payload_json)?;
            let actual_batches = reader.source_batch_attempts(subject_id)?;
            let actual_facts = reader.source_facts(subject_id)?;
            let actual_attempts = reader.source_fact_attempts(subject_id)?;
            if stage.source_batch_attempt_rows != actual_batches
                || stage.source_fact_rows != actual_facts
                || stage.source_fact_attempt_rows != actual_attempts
            {
                return Err(invariant(
                    "ingress_domain_readback_mismatch",
                    "persisted ingress domain rows differ from recovery payload",
                ));
            }
            stage.source_batch_attempt_rows = actual_batches;
            stage.source_fact_rows = actual_facts;
            stage.source_fact_attempt_rows = actual_attempts;
            stage.validate().map_err(|error| {
                SelectionV2RepositoryError::Canonical(format!(
                    "stored source ingress stage invalid: {error}"
                ))
            })?;
            let row_hashes = ingress_run_row_hashes(&stage)?;
            let payload = RunPayloadPreimage {
                domain: DOMAIN_INGRESS_PAYLOAD.into(),
                subject_kind,
                subject_id: subject_id.into(),
                logical_subject_key: stage.logical_subject_key.clone(),
                source_fact_key: None,
                config_activation_run_id: stage.config_activation_run_id.clone(),
                config_hash: stage.config_hash.clone(),
                config_snapshot_json_hash: None,
                config_activation_content_hash: None,
                config_activation_file_content_hash: None,
                config_effective_from_rfc3339_nanos_utc: None,
                artifact_valid_from: None,
                artifact_expires_at: None,
                executable_revision: None,
                legacy_cutover_snapshot_hash: None,
                generation_market_date: Some(stage.generation_market_date.clone()),
                aggregator_observed_at_rfc3339_nanos_utc: Some(
                    stage.aggregator_observed_at_rfc3339_nanos_utc.clone(),
                ),
                ingress_source_batch_content_hash: Some(stage.source_batch_content_hash.clone()),
                outcome_phase: None,
                stored_due_date: None,
                outcome_claim_id: None,
                planned_outcome_run_id: None,
                outcome_claim_receipt_content_hash: None,
                outcome_claim_due_binding_hash: None,
                outcome_claim_provider_request_hash: None,
                rows: row_hashes.clone(),
            };
            (payload, row_hashes)
        }
        SubjectKind::GenerationRun => {
            let stage: GenerationStageInputPreimage =
                parse_canonical_payload(&envelope.payload_json)?;
            if stage.stage_run_id != subject_id
                || stage.logical_subject_key != envelope.logical_subject_key
                || stage.config_activation_run_id != envelope.config_activation_run_id
                || stage.config_hash != envelope.config_hash
            {
                return Err(invariant(
                    "generation_payload_readback_identity_mismatch",
                    "stored generation payload identity differs from envelope",
                ));
            }
            stage.validate().map_err(|error| {
                SelectionV2RepositoryError::Canonical(format!(
                    "stored generation stage invalid: {error}"
                ))
            })?;
            let actual = reader.generation_rows(subject_id)?;
            if actual.relations != stage.relation_attempt_rows
                || actual.evaluations != stage.evaluation_attempt_rows
                || actual.samples != stage.sample_rows
                || actual.rejections != stage.rejection_rows
            {
                return Err(invariant(
                    "generation_domain_readback_mismatch",
                    "persisted generation typed rows differ from recovery payload",
                ));
            }
            let actual_stage = GenerationStageInputPreimage {
                relation_attempt_rows: actual.relations,
                evaluation_attempt_rows: actual.evaluations,
                sample_rows: actual.samples,
                rejection_rows: actual.rejections,
                ..stage.clone()
            };
            actual_stage.validate().map_err(|error| {
                SelectionV2RepositoryError::Canonical(format!(
                    "read-back generation stage invalid: {error}"
                ))
            })?;
            let actual_row_hashes = generation_run_row_hashes(&actual_stage)?;
            let payload = RunPayloadPreimage {
                domain: DOMAIN_GENERATION_PAYLOAD.into(),
                subject_kind,
                subject_id: subject_id.into(),
                logical_subject_key: stage.logical_subject_key.clone(),
                source_fact_key: Some(stage.source_fact_key.clone()),
                config_activation_run_id: stage.config_activation_run_id.clone(),
                config_hash: stage.config_hash.clone(),
                config_snapshot_json_hash: None,
                config_activation_content_hash: None,
                config_activation_file_content_hash: None,
                config_effective_from_rfc3339_nanos_utc: None,
                artifact_valid_from: None,
                artifact_expires_at: None,
                executable_revision: None,
                legacy_cutover_snapshot_hash: None,
                generation_market_date: Some(stage.generation_market_date.clone()),
                aggregator_observed_at_rfc3339_nanos_utc: None,
                ingress_source_batch_content_hash: None,
                outcome_phase: None,
                stored_due_date: None,
                outcome_claim_id: None,
                planned_outcome_run_id: None,
                outcome_claim_receipt_content_hash: None,
                outcome_claim_due_binding_hash: None,
                outcome_claim_provider_request_hash: None,
                rows: actual_row_hashes.clone(),
            };
            (payload, actual_row_hashes)
        }
        SubjectKind::OutcomeClaim => {
            let stage: OutcomeClaimStageInputPreimage =
                parse_canonical_payload(&envelope.payload_json)?;
            if stage.stage_run_id != subject_id
                || stage.logical_subject_key != envelope.logical_subject_key
                || stage.config_activation_run_id != envelope.config_activation_run_id
                || stage.config_hash != envelope.config_hash
            {
                return Err(invariant(
                    "outcome_claim_payload_readback_identity_mismatch",
                    "stored outcome claim payload identity differs from envelope",
                ));
            }
            stage.validate().map_err(|error| {
                SelectionV2RepositoryError::Canonical(format!(
                    "stored outcome claim stage invalid: {error}"
                ))
            })?;
            let payload = RunPayloadPreimage {
                domain: DOMAIN_OUTCOME_CLAIM_PAYLOAD.into(),
                subject_kind,
                subject_id: subject_id.into(),
                logical_subject_key: stage.logical_subject_key,
                source_fact_key: None,
                config_activation_run_id: stage.config_activation_run_id,
                config_hash: stage.config_hash,
                config_snapshot_json_hash: None,
                config_activation_content_hash: None,
                config_activation_file_content_hash: None,
                config_effective_from_rfc3339_nanos_utc: None,
                artifact_valid_from: None,
                artifact_expires_at: None,
                executable_revision: None,
                legacy_cutover_snapshot_hash: None,
                generation_market_date: None,
                aggregator_observed_at_rfc3339_nanos_utc: None,
                ingress_source_batch_content_hash: None,
                outcome_phase: Some(stage.due_binding.outcome_phase),
                stored_due_date: Some(stage.due_binding.stored_due_date),
                outcome_claim_id: Some(stage.stage_run_id),
                planned_outcome_run_id: Some(stage.planned_outcome_run_id),
                outcome_claim_receipt_content_hash: None,
                outcome_claim_due_binding_hash: Some(stage.due_binding_hash),
                outcome_claim_provider_request_hash: Some(stage.provider_request_hash),
                rows: Vec::new(),
            };
            (payload, Vec::new())
        }
        SubjectKind::OutcomeRun => {
            let stage: OutcomeStageInputPreimage = parse_canonical_payload(&envelope.payload_json)?;
            if stage.stage_run_id != subject_id
                || stage.logical_subject_key != envelope.logical_subject_key
                || stage.config_activation_run_id != envelope.config_activation_run_id
                || stage.config_hash != envelope.config_hash
            {
                return Err(invariant(
                    "outcome_payload_readback_identity_mismatch",
                    "stored outcome payload identity differs from envelope",
                ));
            }
            stage.validate().map_err(|error| {
                SelectionV2RepositoryError::Canonical(format!(
                    "stored outcome stage invalid: {error}"
                ))
            })?;
            load_outcome_authority_with_reader(reader, &stage)?;
            let actual = reader.outcome_rows(subject_id)?;
            if actual.attempts != stage.outcome_attempt_rows
                || actual.outcomes != stage.outcome_rows
            {
                return Err(invariant(
                    "outcome_domain_readback_mismatch",
                    "persisted outcome typed rows differ from recovery payload",
                ));
            }
            let actual_stage = OutcomeStageInputPreimage {
                outcome_attempt_rows: actual.attempts,
                outcome_rows: actual.outcomes,
                ..stage.clone()
            };
            actual_stage.validate().map_err(|error| {
                SelectionV2RepositoryError::Canonical(format!(
                    "read-back outcome stage invalid: {error}"
                ))
            })?;
            let actual_row_hashes = outcome_run_row_hashes(&actual_stage)?;
            let payload = RunPayloadPreimage {
                domain: DOMAIN_OUTCOME_PAYLOAD.into(),
                subject_kind,
                subject_id: subject_id.into(),
                logical_subject_key: stage.logical_subject_key.clone(),
                source_fact_key: None,
                config_activation_run_id: stage.config_activation_run_id.clone(),
                config_hash: stage.config_hash.clone(),
                config_snapshot_json_hash: None,
                config_activation_content_hash: None,
                config_activation_file_content_hash: None,
                config_effective_from_rfc3339_nanos_utc: None,
                artifact_valid_from: None,
                artifact_expires_at: None,
                executable_revision: None,
                legacy_cutover_snapshot_hash: None,
                generation_market_date: None,
                aggregator_observed_at_rfc3339_nanos_utc: None,
                ingress_source_batch_content_hash: None,
                outcome_phase: Some(stage.outcome_phase),
                stored_due_date: Some(stage.stored_due_date.clone()),
                outcome_claim_id: Some(stage.outcome_claim_id),
                planned_outcome_run_id: None,
                outcome_claim_receipt_content_hash: Some(stage.outcome_claim_receipt_content_hash),
                outcome_claim_due_binding_hash: Some(stage.outcome_claim_due_binding_hash),
                outcome_claim_provider_request_hash: Some(
                    stage.outcome_claim_provider_request_hash,
                ),
                rows: actual_row_hashes.clone(),
            };
            (payload, actual_row_hashes)
        }
    };
    let in_memory_payload_hash = hash(&run_payload)?;
    if in_memory_payload_hash != envelope.in_memory_payload_hash
        || in_memory_payload_hash != manifest_row.in_memory_payload_hash
    {
        return Err(invariant(
            "run_payload_readback_rehash_mismatch",
            "reconstructed run payload differs from envelope/manifest hash",
        ));
    }
    let staged_rows = staged_row_hashes(&envelope, &envelope_hash, &domain_row_hashes)?;
    let expected_count = u32::try_from(staged_rows.len()).map_err(|_| {
        invariant(
            "staged_row_count_overflow",
            "staged readback row count does not fit u32",
        )
    })?;
    let staged_db = StagedDbPreimage {
        domain: DOMAIN_STAGED_DB.into(),
        subject_kind,
        subject_id: subject_id.into(),
        expected_staged_row_count: expected_count,
        rows: staged_rows,
    };
    let staged_db_hash = hash(&staged_db)?;
    if i64::from(expected_count) != manifest_row.expected_staged_row_count
        || staged_db_hash != manifest_row.staged_db_content_hash
        || envelope_hash != manifest_row.recovery_envelope_content_hash
    {
        return Err(invariant(
            "staged_db_readback_rehash_mismatch",
            "manifest count/staged/envelope hash differs from readback",
        ));
    }
    let manifest = rebuild_manifest(&manifest_row)?;
    manifest.validate_kind_matrix().map_err(|error| {
        SelectionV2RepositoryError::Canonical(format!("stored manifest matrix invalid: {error}"))
    })?;
    let manifest_hash = hash(&manifest)?;
    if manifest_hash != manifest_row.manifest_content_hash {
        return Err(invariant(
            "manifest_readback_rehash_mismatch",
            "manifest columns do not reproduce manifest_content_hash",
        ));
    }
    Ok(StagedRunReceipt {
        disposition,
        subject_kind,
        subject_id: subject_id.into(),
        logical_subject_key: envelope.logical_subject_key,
        in_memory_payload_hash,
        recovery_envelope_content_hash: envelope_hash,
        staged_db_content_hash: staged_db_hash,
        run_manifest_content_hash: manifest_hash,
        expected_staged_row_count: expected_count,
    })
}

fn canonical_payload_hash(payload: &str) -> RepositoryResult<String> {
    Ok(crate::selection::schema_v2::sha256_bytes(
        payload.as_bytes(),
    ))
}

fn parse_canonical_payload<T>(payload: &str) -> RepositoryResult<T>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let typed: T = serde_json::from_str(payload)?;
    if canonical(&typed)? != payload {
        return Err(invariant(
            "payload_json_not_canonical",
            "stored payload differs from canonical typed serialization",
        ));
    }
    Ok(typed)
}

fn rebuild_manifest(row: &ManifestRow) -> RepositoryResult<RunManifestContentPreimage> {
    let expected_staged_row_count = u32::try_from(row.expected_staged_row_count).map_err(|_| {
        invariant(
            "manifest_row_count_invalid",
            "manifest expected count does not fit u32",
        )
    })?;
    Ok(RunManifestContentPreimage {
        domain: DOMAIN_RUN_MANIFEST.into(),
        subject_kind: parse_subject_kind(&row.subject_kind)?,
        subject_id: row.subject_id.clone(),
        in_memory_payload_hash: row.in_memory_payload_hash.clone(),
        prepared_record_hash: row.prepared_record_hash.clone(),
        expected_staged_row_count,
        staged_db_content_hash: row.staged_db_content_hash.clone(),
        recovery_envelope_content_hash: row.recovery_envelope_content_hash.clone(),
        logical_subject_key: row.logical_subject_key.clone(),
        run_status: parse_run_status(&row.run_status)?,
        source_fact_key: row.source_fact_key.clone(),
        config_activation_run_id: Some(row.config_activation_run_id.clone()),
        config_hash: Some(row.config_hash.clone()),
        config_snapshot_json_hash: row.config_snapshot_json_hash.clone(),
        config_activation_content_hash: row.config_activation_content_hash.clone(),
        config_activation_file_content_hash: row.config_activation_file_content_hash.clone(),
        config_effective_from_rfc3339_nanos_utc: row.config_effective_from.clone(),
        artifact_valid_from: row.artifact_valid_from.clone(),
        artifact_expires_at: row.artifact_expires_at.clone(),
        executable_revision: row.executable_revision.clone(),
        legacy_cutover_snapshot_hash: row.legacy_cutover_snapshot_hash.clone(),
        generation_market_date: row.generation_market_date.clone(),
        aggregator_observed_at_rfc3339_nanos_utc: row.aggregator_observed_at.clone(),
        ingress_source_batch_content_hash: row.ingress_source_batch_content_hash.clone(),
        outcome_phase: row
            .outcome_phase
            .as_deref()
            .map(parse_outcome_phase)
            .transpose()?,
        stored_due_date: row.stored_due_date.clone(),
        outcome_claim_id: row.outcome_claim_id.clone(),
        planned_outcome_run_id: row.planned_outcome_run_id.clone(),
        outcome_claim_receipt_content_hash: row.outcome_claim_receipt_content_hash.clone(),
        outcome_claim_due_binding_hash: row.outcome_claim_due_binding_hash.clone(),
        outcome_claim_provider_request_hash: row.outcome_claim_provider_request_hash.clone(),
        staged_at_rfc3339_nanos_utc: row.staged_at.clone(),
    })
}

fn parse_subject_kind(value: &str) -> RepositoryResult<SubjectKind> {
    match value {
        "config_activation" => Ok(SubjectKind::ConfigActivation),
        "ingress_run" => Ok(SubjectKind::IngressRun),
        "generation_run" => Ok(SubjectKind::GenerationRun),
        "outcome_claim" => Ok(SubjectKind::OutcomeClaim),
        "outcome_run" => Ok(SubjectKind::OutcomeRun),
        _ => Err(invariant(
            "subject_kind_unknown",
            format!("unknown subject kind {value:?}"),
        )),
    }
}

fn parse_run_status(value: &str) -> RepositoryResult<RunStatus> {
    match value {
        "activated" => Ok(RunStatus::Activated),
        "claimed" => Ok(RunStatus::Claimed),
        "completed" => Ok(RunStatus::Completed),
        "verified_no_relation" => Ok(RunStatus::VerifiedNoRelation),
        "pending_dependency" => Ok(RunStatus::PendingDependency),
        "failed_non_retryable" => Ok(RunStatus::FailedNonRetryable),
        "settled" => Ok(RunStatus::Settled),
        "expected_wait" => Ok(RunStatus::ExpectedWait),
        "failed_retryable" => Ok(RunStatus::FailedRetryable),
        _ => Err(invariant(
            "run_status_unknown",
            format!("unknown run status {value:?}"),
        )),
    }
}

fn parse_outcome_phase(value: &str) -> RepositoryResult<crate::selection::schema_v2::OutcomePhase> {
    use crate::selection::schema_v2::OutcomePhase;
    match value {
        "t0_close" => Ok(OutcomePhase::T0Close),
        "d1_settled" => Ok(OutcomePhase::D1Settled),
        "d3_settled" => Ok(OutcomePhase::D3Settled),
        "d5_settled" => Ok(OutcomePhase::D5Settled),
        _ => Err(invariant(
            "outcome_phase_unknown",
            format!("unknown outcome phase {value:?}"),
        )),
    }
}

#[derive(Debug, QueryableByName)]
struct SourceBatchAttemptDbRow {
    #[diesel(sql_type = Text)]
    source_batch_attempt_id: String,
    #[diesel(sql_type = Text)]
    ingress_run_id: String,
    #[diesel(sql_type = Text)]
    config_activation_run_id: String,
    #[diesel(sql_type = Text)]
    config_hash: String,
    #[diesel(sql_type = Text)]
    generation_market_date: String,
    #[diesel(sql_type = Text)]
    registered_feed_identity: String,
    #[diesel(sql_type = Text)]
    registered_feed_snapshot_hash: String,
    #[diesel(sql_type = Text)]
    request_hash: String,
    #[diesel(sql_type = Text)]
    request_evidence_json: String,
    #[diesel(sql_type = Text)]
    request_evidence_hash: String,
    #[diesel(sql_type = Text)]
    feed_attempt_content_hash: String,
    #[diesel(sql_type = Text)]
    status_kind: String,
    #[diesel(sql_type = Nullable<BigInt>)]
    record_count: Option<i64>,
    #[diesel(sql_type = Nullable<Text>)]
    provider: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    source: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    source_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    observed_at: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    batch_id: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    batch_content_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    failed_stage: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    reason_code: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    retryable: Option<i32>,
    #[diesel(sql_type = Nullable<Text>)]
    available_evidence_json: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    available_evidence_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    error_detail_json: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    error_detail_hash: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    error_fingerprint: Option<String>,
    #[diesel(sql_type = Text)]
    attempted_at: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
}

impl SourceBatchAttemptDbRow {
    fn into_preimage(self) -> RepositoryResult<SelectionSourceBatchAttemptRowContentPreimage> {
        let status_kind = match self.status_kind.as_str() {
            "available" => FeedStatusKind::Available,
            "verified_empty" => FeedStatusKind::VerifiedEmpty,
            "unavailable" => FeedStatusKind::Unavailable,
            value => {
                return Err(invariant(
                    "feed_status_unknown",
                    format!("unknown feed status {value:?}"),
                ));
            }
        };
        let record_count = match (status_kind, self.record_count) {
            (FeedStatusKind::Unavailable, None) => None,
            (FeedStatusKind::Unavailable, Some(_)) => {
                return Err(invariant(
                    "feed_record_count_unavailable_nonnull",
                    "Unavailable feed readback must preserve NULL record_count",
                ));
            }
            (FeedStatusKind::Available | FeedStatusKind::VerifiedEmpty, Some(count)) => {
                Some(u32::try_from(count).map_err(|_| {
                    invariant("feed_record_count_invalid", "record count does not fit u32")
                })?)
            }
            (FeedStatusKind::Available | FeedStatusKind::VerifiedEmpty, None) => {
                return Err(invariant(
                    "feed_record_count_available_null",
                    "Available/VerifiedEmpty feed readback requires record_count",
                ));
            }
        };
        let retryable = parse_optional_bool(self.retryable)?;
        let preimage = SelectionSourceBatchAttemptRowContentPreimage {
            domain: crate::selection::schema_v2::DOMAIN_SOURCE_BATCH_ATTEMPT_ROW.into(),
            source_batch_attempt_id: self.source_batch_attempt_id,
            ingress_run_id: self.ingress_run_id,
            config_activation_run_id: self.config_activation_run_id,
            config_hash: self.config_hash,
            generation_market_date: self.generation_market_date,
            registered_feed_identity: self.registered_feed_identity,
            registered_feed_snapshot_hash: self.registered_feed_snapshot_hash,
            request_hash: self.request_hash,
            request_evidence_json: self.request_evidence_json,
            request_evidence_hash: self.request_evidence_hash,
            feed_attempt_content_hash: self.feed_attempt_content_hash,
            status_kind,
            record_count,
            provider: self.provider,
            source: self.source,
            source_at: self.source_at,
            observed_at: self.observed_at,
            batch_id: self.batch_id,
            batch_content_hash: self.batch_content_hash,
            failed_stage: self.failed_stage,
            reason_code: self.reason_code,
            retryable,
            available_evidence_json: self.available_evidence_json,
            available_evidence_hash: self.available_evidence_hash,
            error_detail_json: self.error_detail_json,
            error_detail_hash: self.error_detail_hash,
            error_fingerprint: self.error_fingerprint,
            attempted_at: self.attempted_at,
        };
        if hash(&preimage)? != self.content_hash {
            return Err(invariant(
                "batch_attempt_readback_rehash_mismatch",
                "batch attempt columns do not reproduce content_hash",
            ));
        }
        Ok(preimage)
    }
}

#[derive(Debug, QueryableByName)]
struct SourceFactDbRow {
    #[diesel(sql_type = Text)]
    source_fact_key: String,
    #[diesel(sql_type = Text)]
    event_id: String,
    #[diesel(sql_type = Text)]
    payload_schema: String,
    #[diesel(sql_type = Text)]
    config_activation_run_id: String,
    #[diesel(sql_type = Text)]
    config_hash: String,
    #[diesel(sql_type = Text)]
    generation_market_date: String,
    #[diesel(sql_type = Text)]
    provider_source: String,
    #[diesel(sql_type = Text)]
    item_id: String,
    #[diesel(sql_type = Text)]
    title: String,
    #[diesel(sql_type = Nullable<Text>)]
    summary: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    content: Option<String>,
    #[diesel(sql_type = Text)]
    publisher: String,
    #[diesel(sql_type = Text)]
    canonical_url: String,
    #[diesel(sql_type = Text)]
    published_at: String,
    #[diesel(sql_type = Text)]
    instruments_json: String,
    #[diesel(sql_type = Text)]
    topics_json: String,
    #[diesel(sql_type = Text)]
    language: String,
    #[diesel(sql_type = Text)]
    record_provider: String,
    #[diesel(sql_type = Text)]
    record_source: String,
    #[diesel(sql_type = Nullable<Text>)]
    record_source_at: Option<String>,
    #[diesel(sql_type = Text)]
    record_observed_at: String,
    #[diesel(sql_type = Text)]
    record_batch_id: String,
    #[diesel(sql_type = Text)]
    record_batch_content_hash: String,
    #[diesel(sql_type = Text)]
    provider_content_hash: String,
    #[diesel(sql_type = Text)]
    first_ingress_run_id: String,
    #[diesel(sql_type = Text)]
    ingress_gate_version: String,
    #[diesel(sql_type = Text)]
    ingress_gate_input_json: String,
    #[diesel(sql_type = Text)]
    ingress_gate_input_hash: String,
    #[diesel(sql_type = Text)]
    ingress_decision: String,
    #[diesel(sql_type = Nullable<Text>)]
    ingress_reason_code: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    ingress_retryable: Option<i32>,
    #[diesel(sql_type = Text)]
    ingress_gate_receipt_json: String,
    #[diesel(sql_type = Text)]
    ingress_gate_receipt_hash: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
}

impl SourceFactDbRow {
    fn into_preimage(self) -> RepositoryResult<SelectionSourceFactRowContentPreimage> {
        let ingress_decision = match self.ingress_decision.as_str() {
            "admitted" => IngressDecision::Admitted,
            "rejected" => IngressDecision::Rejected,
            value => {
                return Err(invariant(
                    "ingress_decision_unknown",
                    format!("unknown ingress decision {value:?}"),
                ));
            }
        };
        let preimage = SelectionSourceFactRowContentPreimage {
            domain: crate::selection::schema_v2::DOMAIN_SOURCE_FACT_ROW.into(),
            source_fact_key: self.source_fact_key,
            event_id: self.event_id,
            payload_schema: self.payload_schema,
            config_activation_run_id: self.config_activation_run_id,
            config_hash: self.config_hash,
            generation_market_date: self.generation_market_date,
            provider_source: self.provider_source,
            item_id: self.item_id,
            title: self.title,
            summary: self.summary,
            content: self.content,
            publisher: self.publisher,
            canonical_url: self.canonical_url,
            published_at: self.published_at,
            instruments_json: self.instruments_json,
            topics_json: self.topics_json,
            language: self.language,
            record_provider: self.record_provider,
            record_source: self.record_source,
            record_source_at: self.record_source_at,
            record_observed_at: self.record_observed_at,
            record_batch_id: self.record_batch_id,
            record_batch_content_hash: self.record_batch_content_hash,
            provider_content_hash: self.provider_content_hash,
            first_ingress_run_id: self.first_ingress_run_id,
            ingress_gate_version: self.ingress_gate_version,
            ingress_gate_input_json: self.ingress_gate_input_json,
            ingress_gate_input_hash: self.ingress_gate_input_hash,
            ingress_decision,
            ingress_reason_code: self.ingress_reason_code,
            ingress_retryable: parse_optional_bool(self.ingress_retryable)?,
            ingress_gate_receipt_json: self.ingress_gate_receipt_json,
            ingress_gate_receipt_hash: self.ingress_gate_receipt_hash,
        };
        if hash(&preimage)? != self.content_hash {
            return Err(invariant(
                "source_fact_readback_rehash_mismatch",
                "source fact columns do not reproduce content_hash",
            ));
        }
        Ok(preimage)
    }
}

#[derive(Debug, QueryableByName)]
struct SourceFactAttemptDbRow {
    #[diesel(sql_type = Text)]
    source_fact_attempt_id: String,
    #[diesel(sql_type = Text)]
    ingress_run_id: String,
    #[diesel(sql_type = Text)]
    source_batch_attempt_id: String,
    #[diesel(sql_type = BigInt)]
    provider_ordinal: i64,
    #[diesel(sql_type = Text)]
    source_fact_key: String,
    #[diesel(sql_type = Text)]
    acquired_record_json: String,
    #[diesel(sql_type = Text)]
    acquired_record_hash: String,
    #[diesel(sql_type = Text)]
    batch_evidence_json: String,
    #[diesel(sql_type = Text)]
    batch_evidence_hash: String,
    #[diesel(sql_type = Text)]
    event_projection_id: String,
    #[diesel(sql_type = Text)]
    attempt_result: String,
    #[diesel(sql_type = Nullable<Text>)]
    conflict_hash: Option<String>,
    #[diesel(sql_type = Text)]
    attempted_at: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
}

impl SourceFactAttemptDbRow {
    fn into_preimage(self) -> RepositoryResult<SelectionSourceFactAttemptRowContentPreimage> {
        let attempt_result = match self.attempt_result.as_str() {
            "inserted" => SourceFactAttemptResult::Accepted,
            "exact_replay" => SourceFactAttemptResult::Replay,
            "conflict" => SourceFactAttemptResult::Conflict,
            value => {
                return Err(invariant(
                    "source_fact_attempt_result_unknown",
                    format!("unknown source fact attempt result {value:?}"),
                ));
            }
        };
        let preimage = SelectionSourceFactAttemptRowContentPreimage {
            domain: crate::selection::schema_v2::DOMAIN_SOURCE_FACT_ATTEMPT_ROW.into(),
            source_fact_attempt_id: self.source_fact_attempt_id,
            ingress_run_id: self.ingress_run_id,
            source_batch_attempt_id: self.source_batch_attempt_id,
            provider_ordinal: u32::try_from(self.provider_ordinal).map_err(|_| {
                invariant(
                    "provider_ordinal_invalid",
                    "provider ordinal does not fit u32",
                )
            })?,
            source_fact_key: self.source_fact_key,
            acquired_record_json: self.acquired_record_json,
            acquired_record_hash: self.acquired_record_hash,
            batch_evidence_json: self.batch_evidence_json,
            batch_evidence_hash: self.batch_evidence_hash,
            event_projection_id: self.event_projection_id,
            attempt_result,
            conflict_hash: self.conflict_hash,
            attempted_at: self.attempted_at,
        };
        if hash(&preimage)? != self.content_hash {
            return Err(invariant(
                "source_fact_attempt_readback_rehash_mismatch",
                "source fact attempt columns do not reproduce content_hash",
            ));
        }
        Ok(preimage)
    }
}

fn parse_optional_bool(value: Option<i32>) -> RepositoryResult<Option<bool>> {
    match value {
        None => Ok(None),
        Some(0) => Ok(Some(false)),
        Some(1) => Ok(Some(true)),
        Some(value) => Err(invariant(
            "sqlite_bool_invalid",
            format!("SQLite bool must be 0/1, got {value}"),
        )),
    }
}

fn load_source_batch_attempts(
    conn: &mut SqliteConnection,
    ingress_run_id: &str,
) -> RepositoryResult<Vec<SelectionSourceBatchAttemptRowContentPreimage>> {
    let rows = diesel::sql_query(
        "SELECT source_batch_attempt_id, ingress_run_id,
                config_activation_run_id, config_hash, generation_market_date,
                registered_feed_identity, registered_feed_snapshot_hash,
                request_hash, request_evidence_json, request_evidence_hash,
                feed_attempt_content_hash, status_kind, record_count, provider,
                source, source_at, observed_at, batch_id, batch_content_hash,
                failed_stage, reason_code, retryable, available_evidence_json,
                available_evidence_hash, error_detail_json, error_detail_hash,
                error_fingerprint, attempted_at, content_hash
         FROM selection_source_batch_attempts
         WHERE ingress_run_id=?
         ORDER BY source_batch_attempt_id ASC",
    )
    .bind::<Text, _>(ingress_run_id)
    .load::<SourceBatchAttemptDbRow>(conn)?;
    rows.into_iter().map(|row| row.into_preimage()).collect()
}

fn load_source_facts(
    conn: &mut SqliteConnection,
    ingress_run_id: &str,
) -> RepositoryResult<Vec<SelectionSourceFactRowContentPreimage>> {
    let rows = diesel::sql_query(
        "SELECT source_fact_key, event_id, payload_schema,
                config_activation_run_id, config_hash, generation_market_date,
                provider_source, item_id, title, summary, content, publisher,
                canonical_url, published_at, instruments_json, topics_json,
                language, record_provider, record_source, record_source_at,
                record_observed_at, record_batch_id, record_batch_content_hash,
                provider_content_hash, first_ingress_run_id,
                ingress_gate_version, ingress_gate_input_json,
                ingress_gate_input_hash, ingress_decision, ingress_reason_code,
                ingress_retryable, ingress_gate_receipt_json,
                ingress_gate_receipt_hash, content_hash
         FROM selection_source_facts_v2
         WHERE first_ingress_run_id=?
         ORDER BY source_fact_key ASC",
    )
    .bind::<Text, _>(ingress_run_id)
    .load::<SourceFactDbRow>(conn)?;
    rows.into_iter().map(|row| row.into_preimage()).collect()
}

fn load_source_fact_attempts(
    conn: &mut SqliteConnection,
    ingress_run_id: &str,
) -> RepositoryResult<Vec<SelectionSourceFactAttemptRowContentPreimage>> {
    let rows = diesel::sql_query(
        "SELECT source_fact_attempt_id, ingress_run_id,
                source_batch_attempt_id, provider_ordinal, source_fact_key,
                acquired_record_json, acquired_record_hash, batch_evidence_json,
                batch_evidence_hash, event_projection_id, attempt_result,
                conflict_hash, attempted_at, content_hash
         FROM selection_source_fact_attempts
         WHERE ingress_run_id=?
         ORDER BY source_fact_attempt_id ASC",
    )
    .bind::<Text, _>(ingress_run_id)
    .load::<SourceFactAttemptDbRow>(conn)?;
    rows.into_iter().map(|row| row.into_preimage()).collect()
}

fn rusqlite_load_source_batch_attempts(
    transaction: &rusqlite::Transaction<'_>,
    ingress_run_id: &str,
) -> RepositoryResult<Vec<SelectionSourceBatchAttemptRowContentPreimage>> {
    let mut statement = transaction.prepare(
        "SELECT source_batch_attempt_id, ingress_run_id,
                config_activation_run_id, config_hash, generation_market_date,
                registered_feed_identity, registered_feed_snapshot_hash,
                request_hash, request_evidence_json, request_evidence_hash,
                feed_attempt_content_hash, status_kind, record_count, provider,
                source, source_at, observed_at, batch_id, batch_content_hash,
                failed_stage, reason_code, retryable, available_evidence_json,
                available_evidence_hash, error_detail_json, error_detail_hash,
                error_fingerprint, attempted_at, content_hash
         FROM selection_source_batch_attempts
         WHERE ingress_run_id=?1
         ORDER BY source_batch_attempt_id ASC",
    )?;
    let rows = statement.query_map([ingress_run_id], |row| {
        Ok(SourceBatchAttemptDbRow {
            source_batch_attempt_id: row.get(0)?,
            ingress_run_id: row.get(1)?,
            config_activation_run_id: row.get(2)?,
            config_hash: row.get(3)?,
            generation_market_date: row.get(4)?,
            registered_feed_identity: row.get(5)?,
            registered_feed_snapshot_hash: row.get(6)?,
            request_hash: row.get(7)?,
            request_evidence_json: row.get(8)?,
            request_evidence_hash: row.get(9)?,
            feed_attempt_content_hash: row.get(10)?,
            status_kind: row.get(11)?,
            record_count: row.get(12)?,
            provider: row.get(13)?,
            source: row.get(14)?,
            source_at: row.get(15)?,
            observed_at: row.get(16)?,
            batch_id: row.get(17)?,
            batch_content_hash: row.get(18)?,
            failed_stage: row.get(19)?,
            reason_code: row.get(20)?,
            retryable: row.get(21)?,
            available_evidence_json: row.get(22)?,
            available_evidence_hash: row.get(23)?,
            error_detail_json: row.get(24)?,
            error_detail_hash: row.get(25)?,
            error_fingerprint: row.get(26)?,
            attempted_at: row.get(27)?,
            content_hash: row.get(28)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(SourceBatchAttemptDbRow::into_preimage)
        .collect()
}

fn rusqlite_load_source_facts(
    transaction: &rusqlite::Transaction<'_>,
    ingress_run_id: &str,
) -> RepositoryResult<Vec<SelectionSourceFactRowContentPreimage>> {
    let mut statement = transaction.prepare(
        "SELECT source_fact_key, event_id, payload_schema,
                config_activation_run_id, config_hash, generation_market_date,
                provider_source, item_id, title, summary, content, publisher,
                canonical_url, published_at, instruments_json, topics_json,
                language, record_provider, record_source, record_source_at,
                record_observed_at, record_batch_id, record_batch_content_hash,
                provider_content_hash, first_ingress_run_id,
                ingress_gate_version, ingress_gate_input_json,
                ingress_gate_input_hash, ingress_decision, ingress_reason_code,
                ingress_retryable, ingress_gate_receipt_json,
                ingress_gate_receipt_hash, content_hash
         FROM selection_source_facts_v2
         WHERE first_ingress_run_id=?1
         ORDER BY source_fact_key ASC",
    )?;
    let rows = statement.query_map([ingress_run_id], |row| {
        Ok(SourceFactDbRow {
            source_fact_key: row.get(0)?,
            event_id: row.get(1)?,
            payload_schema: row.get(2)?,
            config_activation_run_id: row.get(3)?,
            config_hash: row.get(4)?,
            generation_market_date: row.get(5)?,
            provider_source: row.get(6)?,
            item_id: row.get(7)?,
            title: row.get(8)?,
            summary: row.get(9)?,
            content: row.get(10)?,
            publisher: row.get(11)?,
            canonical_url: row.get(12)?,
            published_at: row.get(13)?,
            instruments_json: row.get(14)?,
            topics_json: row.get(15)?,
            language: row.get(16)?,
            record_provider: row.get(17)?,
            record_source: row.get(18)?,
            record_source_at: row.get(19)?,
            record_observed_at: row.get(20)?,
            record_batch_id: row.get(21)?,
            record_batch_content_hash: row.get(22)?,
            provider_content_hash: row.get(23)?,
            first_ingress_run_id: row.get(24)?,
            ingress_gate_version: row.get(25)?,
            ingress_gate_input_json: row.get(26)?,
            ingress_gate_input_hash: row.get(27)?,
            ingress_decision: row.get(28)?,
            ingress_reason_code: row.get(29)?,
            ingress_retryable: row.get(30)?,
            ingress_gate_receipt_json: row.get(31)?,
            ingress_gate_receipt_hash: row.get(32)?,
            content_hash: row.get(33)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(SourceFactDbRow::into_preimage)
        .collect()
}

fn rusqlite_load_source_fact_attempts(
    transaction: &rusqlite::Transaction<'_>,
    ingress_run_id: &str,
) -> RepositoryResult<Vec<SelectionSourceFactAttemptRowContentPreimage>> {
    let mut statement = transaction.prepare(
        "SELECT source_fact_attempt_id, ingress_run_id,
                source_batch_attempt_id, provider_ordinal, source_fact_key,
                acquired_record_json, acquired_record_hash, batch_evidence_json,
                batch_evidence_hash, event_projection_id, attempt_result,
                conflict_hash, attempted_at, content_hash
         FROM selection_source_fact_attempts
         WHERE ingress_run_id=?1
         ORDER BY source_fact_attempt_id ASC",
    )?;
    let rows = statement.query_map([ingress_run_id], |row| {
        Ok(SourceFactAttemptDbRow {
            source_fact_attempt_id: row.get(0)?,
            ingress_run_id: row.get(1)?,
            source_batch_attempt_id: row.get(2)?,
            provider_ordinal: row.get(3)?,
            source_fact_key: row.get(4)?,
            acquired_record_json: row.get(5)?,
            acquired_record_hash: row.get(6)?,
            batch_evidence_json: row.get(7)?,
            batch_evidence_hash: row.get(8)?,
            event_projection_id: row.get(9)?,
            attempt_result: row.get(10)?,
            conflict_hash: row.get(11)?,
            attempted_at: row.get(12)?,
            content_hash: row.get(13)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(SourceFactAttemptDbRow::into_preimage)
        .collect()
}

struct GenerationRowsReadback {
    relations: Vec<SelectionRelationAttemptRowContentPreimage>,
    evaluations: Vec<SelectionEvaluationAttemptRowContentPreimage>,
    samples: Vec<SelectionSampleRowContentPreimage>,
    rejections: Vec<SelectionRejectionRowContentPreimage>,
}

fn decode_typed_rows<T>(
    rows: Vec<TypedRowJsonDb>,
    table_name: &'static str,
) -> RepositoryResult<Vec<T>>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    rows.into_iter()
        .map(|row| {
            let preimage: T = serde_json::from_str(&row.row_json).map_err(|error| {
                invariant(
                    "typed_row_readback_json_invalid",
                    format!("{table_name}: {error}"),
                )
            })?;
            if hash(&preimage)? != row.content_hash {
                return Err(invariant(
                    "typed_row_readback_rehash_mismatch",
                    format!("{table_name} columns do not reproduce content_hash"),
                ));
            }
            Ok(preimage)
        })
        .collect()
}

fn rusqlite_typed_rows(
    transaction: &rusqlite::Transaction<'_>,
    query: &str,
    domain: &str,
    run_id: &str,
) -> RepositoryResult<Vec<TypedRowJsonDb>> {
    let mut statement = transaction.prepare(query)?;
    let rows = statement.query_map(rusqlite::params![domain, run_id], |row| {
        Ok(TypedRowJsonDb {
            row_json: row.get(0)?,
            content_hash: row.get(1)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(SelectionV2RepositoryError::from)
}

const RUSQLITE_RELATION_ROWS: &str = "SELECT json_object(
    'domain', ?1,
    'relation_attempt_id', relation_attempt_id,
    'relation_key', relation_key,
    'generation_run_id', generation_run_id,
    'source_fact_key', source_fact_key,
    'event_id', event_id,
    'chain_id', chain_id,
    'config_activation_run_id', config_activation_run_id,
    'config_hash', config_hash,
    'relation_schema_version', relation_schema_version,
    'relation_kind', relation_kind,
    'relation_source_identity_json', relation_source_identity_json,
    'relation_source_identity_hash', relation_source_identity_hash,
    'typed_binding_state_json', typed_binding_state_json,
    'typed_binding_state_hash', typed_binding_state_hash,
    'request_hash', request_hash,
    'request_evidence_json', request_evidence_json,
    'request_evidence_hash', request_evidence_hash,
    'result_code', result_code,
    'failed_stage', failed_stage,
    'retryable', CASE WHEN retryable IS NULL THEN NULL
                      ELSE json(iif(retryable=1,'true','false')) END,
    'raw_identity_json', raw_identity_json,
    'raw_identity_hash', raw_identity_hash,
    'canonical_stock_code', canonical_stock_code,
    'canonical_stock_name', canonical_stock_name,
    'canonical_market', canonical_market,
    'artifact_content_hash', artifact_content_hash,
    'binding_audit_hash', binding_audit_hash,
    'provider_board_kind', provider_board_kind,
    'provider_board_code', provider_board_code,
    'provider_board_name', provider_board_name,
    'provider_source', provider_source,
    'provider_source_at', provider_source_at,
    'provider_observed_at', provider_observed_at,
    'provider_batch_id', provider_batch_id,
    'provider_batch_content_hash', provider_batch_content_hash,
    'actual_constituent_count', actual_constituent_count,
    'available_evidence_json', available_evidence_json,
    'available_evidence_hash', available_evidence_hash,
    'error_detail_json', error_detail_json,
    'error_detail_hash', error_detail_hash,
    'error_fingerprint', error_fingerprint,
    'attempted_at', attempted_at
) AS row_json, content_hash
FROM selection_relation_attempts
WHERE generation_run_id=?2
ORDER BY relation_attempt_id ASC";

const RUSQLITE_EVALUATION_ROWS: &str = "SELECT json_object(
    'domain', ?1,
    'evaluation_attempt_id', evaluation_attempt_id,
    'sample_key', sample_key,
    'generation_run_id', generation_run_id,
    'source_fact_key', source_fact_key,
    'event_id', event_id,
    'chain_id', chain_id,
    'canonical_stock_code', canonical_stock_code,
    'canonical_stock_name', canonical_stock_name,
    'canonical_market', canonical_market,
    'relation_evidence_set_hash', relation_evidence_set_hash,
    'market_request_hash', market_request_hash,
    'request_evidence_json', request_evidence_json,
    'request_evidence_hash', request_evidence_hash,
    'result_code', result_code,
    'failed_stage', failed_stage,
    'retryable', CASE WHEN retryable IS NULL THEN NULL
                      ELSE json(iif(retryable=1,'true','false')) END,
    'provider', provider,
    'source', source,
    'source_at', source_at,
    'observed_at', observed_at,
    'batch_id', batch_id,
    'batch_content_hash', batch_content_hash,
    'available_evidence_json', available_evidence_json,
    'available_evidence_hash', available_evidence_hash,
    'terminal_decision_hash', terminal_decision_hash,
    'error_detail_json', error_detail_json,
    'error_detail_hash', error_detail_hash,
    'error_fingerprint', error_fingerprint,
    'attempted_at', attempted_at
) AS row_json, content_hash
FROM selection_evaluation_attempts
WHERE generation_run_id=?2
ORDER BY evaluation_attempt_id ASC";

const RUSQLITE_SAMPLE_ROWS: &str = "SELECT json_object(
    'domain', ?1,
    'sample_key', sample_key,
    'generation_run_id', generation_run_id,
    'source_fact_key', source_fact_key,
    'source_fact_content_hash', source_fact_content_hash,
    'source_fact_attempt_id', source_fact_attempt_id,
    'source_batch_attempt_id', source_batch_attempt_id,
    'event_id', event_id,
    'chain_id', chain_id,
    'config_activation_run_id', config_activation_run_id,
    'config_hash', config_hash,
    'matched_keyword', matched_keyword,
    'canonical_stock_code', canonical_stock_code,
    'canonical_stock_name', canonical_stock_name,
    'canonical_market', canonical_market,
    'relation_schema_version', relation_schema_version,
    'relation_evidence_json', relation_evidence_json,
    'relation_evidence_set_hash', relation_evidence_set_hash,
    'feature_version', feature_version,
    't0_feature_json', t0_feature_json,
    't0_feature_hash', t0_feature_hash,
    'market_provider', market_provider,
    'market_source', market_source,
    'market_source_at', market_source_at,
    'market_observed_at', market_observed_at,
    'market_batch_id', market_batch_id,
    'market_batch_content_hash', market_batch_content_hash,
    'admission_version', admission_version,
    'decision_kind', decision_kind,
    'rejection_count', rejection_count,
    'rejection_row_hashes_in_ordinal_order',
        json(rejection_row_hashes_in_ordinal_order),
    'evaluation_market_date', evaluation_market_date,
    't0_due_date', t0_due_date,
    'd1_due_date', d1_due_date,
    'd2_due_date', d2_due_date,
    'd3_due_date', d3_due_date,
    'd4_due_date', d4_due_date,
    'd5_due_date', d5_due_date,
    'calendar_version', calendar_version,
    'calendar_hash', calendar_hash,
    'trading_date_vector_json', trading_date_vector_json,
    'trading_date_vector_hash', trading_date_vector_hash,
    'staged_at', staged_at
) AS row_json, content_hash
FROM selection_samples
WHERE generation_run_id=?2
ORDER BY sample_key ASC";

const RUSQLITE_REJECTION_ROWS: &str = "SELECT json_object(
    'domain', ?1,
    'sample_key', sample_key,
    'ordinal', ordinal,
    'generation_run_id', generation_run_id,
    'reason_code', reason_code,
    'rule_id', rule_id,
    'retryable', json(iif(retryable=1,'true','false')),
    'structured_detail_json', structured_detail_json,
    'structured_detail_hash', structured_detail_hash,
    'provider', provider,
    'source', source,
    'source_at', source_at,
    'observed_at', observed_at,
    'batch_id', batch_id,
    'batch_content_hash', batch_content_hash,
    'created_at', created_at
) AS row_json, content_hash
FROM selection_rejections
WHERE generation_run_id=?2
ORDER BY sample_key ASC, ordinal ASC";

fn rusqlite_load_generation_rows(
    transaction: &rusqlite::Transaction<'_>,
    generation_run_id: &str,
) -> RepositoryResult<GenerationRowsReadback> {
    let relations = rusqlite_typed_rows(
        transaction,
        RUSQLITE_RELATION_ROWS,
        crate::selection::schema_v2::DOMAIN_RELATION_ATTEMPT_ROW,
        generation_run_id,
    )?;
    let evaluations = rusqlite_typed_rows(
        transaction,
        RUSQLITE_EVALUATION_ROWS,
        crate::selection::schema_v2::DOMAIN_EVALUATION_ATTEMPT_ROW,
        generation_run_id,
    )?;
    let samples = rusqlite_typed_rows(
        transaction,
        RUSQLITE_SAMPLE_ROWS,
        crate::selection::schema_v2::DOMAIN_SAMPLE_ROW,
        generation_run_id,
    )?;
    let rejections = rusqlite_typed_rows(
        transaction,
        RUSQLITE_REJECTION_ROWS,
        crate::selection::schema_v2::DOMAIN_REJECTION_ROW,
        generation_run_id,
    )?;
    Ok(GenerationRowsReadback {
        relations: decode_typed_rows(relations, "selection_relation_attempts")?,
        evaluations: decode_typed_rows(evaluations, "selection_evaluation_attempts")?,
        samples: decode_typed_rows(samples, "selection_samples")?,
        rejections: decode_typed_rows(rejections, "selection_rejections")?,
    })
}

fn load_generation_rows(
    conn: &mut SqliteConnection,
    generation_run_id: &str,
) -> RepositoryResult<GenerationRowsReadback> {
    let relations = diesel::sql_query(
        "SELECT json_object(
            'domain', ?,
            'relation_attempt_id', relation_attempt_id,
            'relation_key', relation_key,
            'generation_run_id', generation_run_id,
            'source_fact_key', source_fact_key,
            'event_id', event_id,
            'chain_id', chain_id,
            'config_activation_run_id', config_activation_run_id,
            'config_hash', config_hash,
            'relation_schema_version', relation_schema_version,
            'relation_kind', relation_kind,
            'relation_source_identity_json', relation_source_identity_json,
            'relation_source_identity_hash', relation_source_identity_hash,
            'typed_binding_state_json', typed_binding_state_json,
            'typed_binding_state_hash', typed_binding_state_hash,
            'request_hash', request_hash,
            'request_evidence_json', request_evidence_json,
            'request_evidence_hash', request_evidence_hash,
            'result_code', result_code,
            'failed_stage', failed_stage,
            'retryable', CASE WHEN retryable IS NULL THEN NULL
                              ELSE json(iif(retryable=1,'true','false')) END,
            'raw_identity_json', raw_identity_json,
            'raw_identity_hash', raw_identity_hash,
            'canonical_stock_code', canonical_stock_code,
            'canonical_stock_name', canonical_stock_name,
            'canonical_market', canonical_market,
            'artifact_content_hash', artifact_content_hash,
            'binding_audit_hash', binding_audit_hash,
            'provider_board_kind', provider_board_kind,
            'provider_board_code', provider_board_code,
            'provider_board_name', provider_board_name,
            'provider_source', provider_source,
            'provider_source_at', provider_source_at,
            'provider_observed_at', provider_observed_at,
            'provider_batch_id', provider_batch_id,
            'provider_batch_content_hash', provider_batch_content_hash,
            'actual_constituent_count', actual_constituent_count,
            'available_evidence_json', available_evidence_json,
            'available_evidence_hash', available_evidence_hash,
            'error_detail_json', error_detail_json,
            'error_detail_hash', error_detail_hash,
            'error_fingerprint', error_fingerprint,
            'attempted_at', attempted_at
         ) AS row_json, content_hash
         FROM selection_relation_attempts
         WHERE generation_run_id=?
         ORDER BY relation_attempt_id ASC",
    )
    .bind::<Text, _>(crate::selection::schema_v2::DOMAIN_RELATION_ATTEMPT_ROW)
    .bind::<Text, _>(generation_run_id)
    .load::<TypedRowJsonDb>(conn)?;
    let evaluations = diesel::sql_query(
        "SELECT json_object(
            'domain', ?,
            'evaluation_attempt_id', evaluation_attempt_id,
            'sample_key', sample_key,
            'generation_run_id', generation_run_id,
            'source_fact_key', source_fact_key,
            'event_id', event_id,
            'chain_id', chain_id,
            'canonical_stock_code', canonical_stock_code,
            'canonical_stock_name', canonical_stock_name,
            'canonical_market', canonical_market,
            'relation_evidence_set_hash', relation_evidence_set_hash,
            'market_request_hash', market_request_hash,
            'request_evidence_json', request_evidence_json,
            'request_evidence_hash', request_evidence_hash,
            'result_code', result_code,
            'failed_stage', failed_stage,
            'retryable', CASE WHEN retryable IS NULL THEN NULL
                              ELSE json(iif(retryable=1,'true','false')) END,
            'provider', provider,
            'source', source,
            'source_at', source_at,
            'observed_at', observed_at,
            'batch_id', batch_id,
            'batch_content_hash', batch_content_hash,
            'available_evidence_json', available_evidence_json,
            'available_evidence_hash', available_evidence_hash,
            'terminal_decision_hash', terminal_decision_hash,
            'error_detail_json', error_detail_json,
            'error_detail_hash', error_detail_hash,
            'error_fingerprint', error_fingerprint,
            'attempted_at', attempted_at
         ) AS row_json, content_hash
         FROM selection_evaluation_attempts
         WHERE generation_run_id=?
         ORDER BY evaluation_attempt_id ASC",
    )
    .bind::<Text, _>(crate::selection::schema_v2::DOMAIN_EVALUATION_ATTEMPT_ROW)
    .bind::<Text, _>(generation_run_id)
    .load::<TypedRowJsonDb>(conn)?;
    let samples = diesel::sql_query(
        "SELECT json_object(
            'domain', ?,
            'sample_key', sample_key,
            'generation_run_id', generation_run_id,
            'source_fact_key', source_fact_key,
            'source_fact_content_hash', source_fact_content_hash,
            'source_fact_attempt_id', source_fact_attempt_id,
            'source_batch_attempt_id', source_batch_attempt_id,
            'event_id', event_id,
            'chain_id', chain_id,
            'config_activation_run_id', config_activation_run_id,
            'config_hash', config_hash,
            'matched_keyword', matched_keyword,
            'canonical_stock_code', canonical_stock_code,
            'canonical_stock_name', canonical_stock_name,
            'canonical_market', canonical_market,
            'relation_schema_version', relation_schema_version,
            'relation_evidence_json', relation_evidence_json,
            'relation_evidence_set_hash', relation_evidence_set_hash,
            'feature_version', feature_version,
            't0_feature_json', t0_feature_json,
            't0_feature_hash', t0_feature_hash,
            'market_provider', market_provider,
            'market_source', market_source,
            'market_source_at', market_source_at,
            'market_observed_at', market_observed_at,
            'market_batch_id', market_batch_id,
            'market_batch_content_hash', market_batch_content_hash,
            'admission_version', admission_version,
            'decision_kind', decision_kind,
            'rejection_count', rejection_count,
            'rejection_row_hashes_in_ordinal_order',
                json(rejection_row_hashes_in_ordinal_order),
            'evaluation_market_date', evaluation_market_date,
            't0_due_date', t0_due_date,
            'd1_due_date', d1_due_date,
            'd2_due_date', d2_due_date,
            'd3_due_date', d3_due_date,
            'd4_due_date', d4_due_date,
            'd5_due_date', d5_due_date,
            'calendar_version', calendar_version,
            'calendar_hash', calendar_hash,
            'trading_date_vector_json', trading_date_vector_json,
            'trading_date_vector_hash', trading_date_vector_hash,
            'staged_at', staged_at
         ) AS row_json, content_hash
         FROM selection_samples
         WHERE generation_run_id=?
         ORDER BY sample_key ASC",
    )
    .bind::<Text, _>(crate::selection::schema_v2::DOMAIN_SAMPLE_ROW)
    .bind::<Text, _>(generation_run_id)
    .load::<TypedRowJsonDb>(conn)?;
    let rejections = diesel::sql_query(
        "SELECT json_object(
            'domain', ?,
            'sample_key', sample_key,
            'ordinal', ordinal,
            'generation_run_id', generation_run_id,
            'reason_code', reason_code,
            'rule_id', rule_id,
            'retryable', json(iif(retryable=1,'true','false')),
            'structured_detail_json', structured_detail_json,
            'structured_detail_hash', structured_detail_hash,
            'provider', provider,
            'source', source,
            'source_at', source_at,
            'observed_at', observed_at,
            'batch_id', batch_id,
            'batch_content_hash', batch_content_hash,
            'created_at', created_at
         ) AS row_json, content_hash
         FROM selection_rejections
         WHERE generation_run_id=?
         ORDER BY sample_key ASC, ordinal ASC",
    )
    .bind::<Text, _>(crate::selection::schema_v2::DOMAIN_REJECTION_ROW)
    .bind::<Text, _>(generation_run_id)
    .load::<TypedRowJsonDb>(conn)?;
    Ok(GenerationRowsReadback {
        relations: decode_typed_rows(relations, "selection_relation_attempts")?,
        evaluations: decode_typed_rows(evaluations, "selection_evaluation_attempts")?,
        samples: decode_typed_rows(samples, "selection_samples")?,
        rejections: decode_typed_rows(rejections, "selection_rejections")?,
    })
}

struct OutcomeRowsReadback {
    outcomes: Vec<SelectionSampleOutcomeRowContentPreimage>,
    attempts: Vec<SelectionOutcomeAttemptRowContentPreimage>,
}

const RUSQLITE_SAMPLE_OUTCOME_ROWS: &str = "SELECT json_object(
    'domain', ?1,
    'sample_key', sample_key,
    'phase', phase,
    'outcome_run_id', outcome_run_id,
    'due_trading_date', due_trading_date,
    'open', open,
    'high', high,
    'low', low,
    'close', close,
    'volume', volume,
    'amount', amount,
    'return_from_t0_close', return_from_t0_close,
    'cumulative_mfe', cumulative_mfe,
    'cumulative_mae', cumulative_mae,
    'volume_ratio', volume_ratio,
    'provider', provider,
    'source', source,
    'source_at', source_at,
    'observed_at', observed_at,
    'batch_id', batch_id,
    'batch_content_hash', batch_content_hash,
    'created_at', created_at
) AS row_json, content_hash
FROM selection_sample_outcomes
WHERE outcome_run_id=?2
ORDER BY sample_key ASC, phase ASC";

const RUSQLITE_OUTCOME_ATTEMPT_ROWS: &str = "SELECT json_object(
    'domain', ?1,
    'outcome_attempt_id', outcome_attempt_id,
    'sample_key', sample_key,
    'phase', phase,
    'stored_due_date', stored_due_date,
    'outcome_run_id', outcome_run_id,
    'request_hash', request_hash,
    'request_evidence_json', request_evidence_json,
    'request_evidence_hash', request_evidence_hash,
    'transport_attempts_json', transport_attempts_json,
    'transport_attempts_hash', transport_attempts_hash,
    'result_code', result_code,
    'reason_code', reason_code,
    'retryable', CASE WHEN retryable IS NULL THEN NULL
                      ELSE json(iif(retryable=1,'true','false')) END,
    'provider', provider,
    'source', source,
    'source_at', source_at,
    'observed_at', observed_at,
    'batch_id', batch_id,
    'batch_content_hash', batch_content_hash,
    'available_evidence_json', available_evidence_json,
    'available_evidence_hash', available_evidence_hash,
    'error_detail_json', error_detail_json,
    'error_detail_hash', error_detail_hash,
    'error_fingerprint', error_fingerprint,
    'settled_outcome_content_hash', settled_outcome_content_hash,
    'attempted_at', attempted_at
) AS row_json, content_hash
FROM selection_outcome_attempts
WHERE outcome_run_id=?2
ORDER BY outcome_attempt_id ASC";

fn rusqlite_load_outcome_rows(
    transaction: &rusqlite::Transaction<'_>,
    outcome_run_id: &str,
) -> RepositoryResult<OutcomeRowsReadback> {
    let outcomes = rusqlite_typed_rows(
        transaction,
        RUSQLITE_SAMPLE_OUTCOME_ROWS,
        crate::selection::schema_v2::DOMAIN_SAMPLE_OUTCOME_ROW,
        outcome_run_id,
    )?;
    let attempts = rusqlite_typed_rows(
        transaction,
        RUSQLITE_OUTCOME_ATTEMPT_ROWS,
        crate::selection::schema_v2::DOMAIN_OUTCOME_ATTEMPT_ROW,
        outcome_run_id,
    )?;
    Ok(OutcomeRowsReadback {
        outcomes: decode_typed_rows(outcomes, "selection_sample_outcomes")?,
        attempts: decode_typed_rows(attempts, "selection_outcome_attempts")?,
    })
}

fn load_outcome_rows(
    conn: &mut SqliteConnection,
    outcome_run_id: &str,
) -> RepositoryResult<OutcomeRowsReadback> {
    let outcomes = diesel::sql_query(
        "SELECT json_object(
            'domain', ?,
            'sample_key', sample_key,
            'phase', phase,
            'outcome_run_id', outcome_run_id,
            'due_trading_date', due_trading_date,
            'open', open,
            'high', high,
            'low', low,
            'close', close,
            'volume', volume,
            'amount', amount,
            'return_from_t0_close', return_from_t0_close,
            'cumulative_mfe', cumulative_mfe,
            'cumulative_mae', cumulative_mae,
            'volume_ratio', volume_ratio,
            'provider', provider,
            'source', source,
            'source_at', source_at,
            'observed_at', observed_at,
            'batch_id', batch_id,
            'batch_content_hash', batch_content_hash,
            'created_at', created_at
         ) AS row_json, content_hash
         FROM selection_sample_outcomes
         WHERE outcome_run_id=?
         ORDER BY sample_key ASC, phase ASC",
    )
    .bind::<Text, _>(crate::selection::schema_v2::DOMAIN_SAMPLE_OUTCOME_ROW)
    .bind::<Text, _>(outcome_run_id)
    .load::<TypedRowJsonDb>(conn)?;
    let attempts = diesel::sql_query(
        "SELECT json_object(
            'domain', ?,
            'outcome_attempt_id', outcome_attempt_id,
            'sample_key', sample_key,
            'phase', phase,
            'stored_due_date', stored_due_date,
            'outcome_run_id', outcome_run_id,
            'request_hash', request_hash,
            'request_evidence_json', request_evidence_json,
            'request_evidence_hash', request_evidence_hash,
            'transport_attempts_json', transport_attempts_json,
            'transport_attempts_hash', transport_attempts_hash,
            'result_code', result_code,
            'reason_code', reason_code,
            'retryable', CASE WHEN retryable IS NULL THEN NULL
                              ELSE json(iif(retryable=1,'true','false')) END,
            'provider', provider,
            'source', source,
            'source_at', source_at,
            'observed_at', observed_at,
            'batch_id', batch_id,
            'batch_content_hash', batch_content_hash,
            'available_evidence_json', available_evidence_json,
            'available_evidence_hash', available_evidence_hash,
            'error_detail_json', error_detail_json,
            'error_detail_hash', error_detail_hash,
            'error_fingerprint', error_fingerprint,
            'settled_outcome_content_hash', settled_outcome_content_hash,
            'attempted_at', attempted_at
         ) AS row_json, content_hash
         FROM selection_outcome_attempts
         WHERE outcome_run_id=?
         ORDER BY outcome_attempt_id ASC",
    )
    .bind::<Text, _>(crate::selection::schema_v2::DOMAIN_OUTCOME_ATTEMPT_ROW)
    .bind::<Text, _>(outcome_run_id)
    .load::<TypedRowJsonDb>(conn)?;
    Ok(OutcomeRowsReadback {
        outcomes: decode_typed_rows(outcomes, "selection_sample_outcomes")?,
        attempts: decode_typed_rows(attempts, "selection_outcome_attempts")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::audit::{SelectionAuditRecord, SelectionAuditWriter};
    use crate::selection::schema_v2::{
        build_request_evidence, run_logical_subject_key, AdjustmentKind,
        AdmissionStructuredDetailPreimage, BindingStateKind, BindingStatePreimage,
        DailyIntervalKind, DirectMentionField, DirectMentionSourcePreimage, EvaluationWindow,
        MentionKind, OutcomeAttemptResult, OutcomeClaimDueBindingPreimage,
        OutcomeMarketRequestParametersPreimage, OutcomePhase,
        OutcomeProviderAvailableEvidencePreimage, OutcomeProviderRequestPreimage,
        OutcomeTransportAttemptPreimage, OutcomeTransportAttemptsPreimage,
        OutcomeTransportBarFingerprint, OutcomeTransportBatchContentPreimage,
        OutcomeTransportEvidencePreimage, OutcomeTransportRequestPreimage,
        OutcomeTransportResultPreimage, ProviderAvailableEvidencePreimage,
        ProviderCapabilityHashPreimage, ProviderEvidenceKind, RawSecurityIdentityPreimage,
        RelationEvidenceEntryPreimage, RelationEvidenceSetPreimage, RelationKind,
        RequestEvidenceColumns, RequestEvidencePreimage, RequestParametersPreimage,
        RunLogicalSubjectPreimage, SampleKeyPreimage, T0FeaturePreimage,
        T0MarketRequestParametersPreimage, TerminalDecisionKind,
        VerifiedOutcomeDueDatabaseBindingPreimage, VerifiedOutcomeDueDatabaseObjectBindingPreimage,
        VerifiedOutcomeDueSnapshotPreimage, AMENDMENT_DESIGN_SHA256, DOMAIN_BINDING_STATE,
        DOMAIN_DIRECT_SOURCE, DOMAIN_EVALUATION_ATTEMPT_ROW, DOMAIN_GENERATION_STAGE,
        DOMAIN_OUTCOME_ATTEMPT_ROW, DOMAIN_OUTCOME_CLAIM_DUE_BINDING, DOMAIN_OUTCOME_CLAIM_STAGE,
        DOMAIN_OUTCOME_DUE_DATABASE_BINDING, DOMAIN_OUTCOME_DUE_DATABASE_OBJECT,
        DOMAIN_OUTCOME_MARKET_REQUEST, DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE,
        DOMAIN_OUTCOME_PROVIDER_REQUEST, DOMAIN_OUTCOME_STAGE, DOMAIN_OUTCOME_TRANSPORT_ATTEMPTS,
        DOMAIN_PROVIDER_AVAILABLE_EVIDENCE, DOMAIN_PROVIDER_CAPABILITY, DOMAIN_REJECTION_ROW,
        DOMAIN_RELATION_ATTEMPT_ROW, DOMAIN_RELATION_EVIDENCE_SET, DOMAIN_RUN_LOGICAL_SUBJECT,
        DOMAIN_SAMPLE_KEY, DOMAIN_SAMPLE_OUTCOME_ROW, DOMAIN_SAMPLE_ROW, DOMAIN_T0_FEATURE,
        DOMAIN_T0_MARKET_REQUEST, DOMAIN_VERIFIED_OUTCOME_DUE_SNAPSHOT,
        OUTCOME_ADAPTIVE_POLICY_VERSION, OUTCOME_PARENT_DESIGN_SHA256, UPSTREAM_REVISION,
    };
    use diesel::connection::SimpleConnection;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    const TEST_ACTIVATION_RUN_ID: &str = "019849b1-e800-7000-8000-000000000100";
    const TEST_INGRESS_RUN_ID: &str = "019849b1-e800-7000-8000-000000000101";
    const TEST_GENERATION_RUN_ID: &str = "019849b1-e800-7000-8000-000000000102";
    const TEST_OUTCOME_RUN_ID: &str = "019849b1-e800-7000-8000-000000000103";
    const TEST_OUTCOME_CLAIM_ID: &str = "019849b1-e800-7000-8000-000000000104";
    const TEST_SOURCE_FACT_ATTEMPT_ID: &str = "TEST_CODE_FACT_ATTEMPT";
    const TEST_SOURCE_BATCH_ATTEMPT_ID: &str = "TEST_CODE_BATCH_ATTEMPT";

    fn test_hash(value: char) -> String {
        value.to_string().repeat(64)
    }

    fn magic_tdx_capability(
        capability_name: &str,
        contract_version: &str,
    ) -> ProviderCapabilityHashPreimage {
        ProviderCapabilityHashPreimage {
            domain: DOMAIN_PROVIDER_CAPABILITY.into(),
            provider: "magic-tdx".into(),
            capability_name: capability_name.into(),
            contract_version: contract_version.into(),
            upstream_revision: UPSTREAM_REVISION.into(),
        }
    }

    fn test_sample_key_preimage() -> SampleKeyPreimage {
        SampleKeyPreimage {
            domain: DOMAIN_SAMPLE_KEY.into(),
            event_id: test_hash('4'),
            chain_id: "TEST_CODE_CHAIN".into(),
            stock_code: "TEST_CODE_000001".into(),
            relation_schema_version: "event-relation-v2".into(),
            feature_version: "feature-v1".into(),
            evaluation_market_date: "2026-07-28".into(),
        }
    }

    fn test_sample_key() -> String {
        hash(&test_sample_key_preimage()).expect("hash test sample key")
    }

    fn t0_request_columns(sample: &SelectionSampleRowContentPreimage) -> RequestEvidenceColumns {
        build_request_evidence(
            RequestParametersPreimage::T0MarketEvidence(T0MarketRequestParametersPreimage {
                domain: DOMAIN_T0_MARKET_REQUEST.into(),
                canonical_stock_code: sample.canonical_stock_code.clone(),
                canonical_market: sample.canonical_market.clone(),
                evaluation_market_date: sample.evaluation_market_date.clone(),
                quote_max_age_secs: 5,
                daily_interval: "day".into(),
                daily_limit: 60,
                intraday_interval: "five_minutes".into(),
                intraday_limit: 120,
                adjustment: AdjustmentKind::None,
            }),
            magic_tdx_capability(
                "MagicTdx-T0MarketBundle",
                "stock-analysis.MagicTdxSelectionGateway.t0_market_evidence.v1",
            ),
        )
        .expect("build T0 request evidence")
    }

    fn outcome_request_columns(
        sample_key: &str,
        phase: OutcomePhase,
        stored_due_date: &str,
    ) -> RequestEvidenceColumns {
        let trading_date_vector = OutcomeTradingDateVectorPreimage {
            domain: crate::selection::schema_v2::DOMAIN_OUTCOME_TRADING_DATE_VECTOR.into(),
            t0: "2026-07-28".into(),
            d1: "2026-07-29".into(),
            d2: "2026-07-30".into(),
            d3: "2026-07-31".into(),
            d4: "2026-08-03".into(),
            d5: "2026-08-04".into(),
        };
        let applicable_trading_dates = trading_date_vector
            .applicable_dates(phase)
            .expect("valid test trading-date vector");
        assert_eq!(
            applicable_trading_dates.last().map(String::as_str),
            Some(stored_due_date),
            "stored due must be the phase-prefix terminal date"
        );
        build_request_evidence(
            RequestParametersPreimage::OutcomeMarketEvidence(
                OutcomeMarketRequestParametersPreimage {
                    domain: DOMAIN_OUTCOME_MARKET_REQUEST.into(),
                    sample_key: sample_key.into(),
                    canonical_stock_code: "TEST_CODE_000001".into(),
                    canonical_market: "SZ".into(),
                    phase,
                    stored_due_date: stored_due_date.into(),
                    calendar_version: "calendar-v1".into(),
                    calendar_hash: test_hash('e'),
                    trading_date_vector_hash: hash(&trading_date_vector)
                        .expect("hash test trading-date vector"),
                    trading_date_vector,
                    applicable_trading_dates,
                    window_start: "2026-07-28".into(),
                    window_end: stored_due_date.into(),
                    interval: DailyIntervalKind::Day,
                    adjustment: AdjustmentKind::None,
                },
            ),
            magic_tdx_capability(
                "MagicTdx-UnadjustedDailyBars",
                "magic-market-core.MarketDataProvider.bars.v0.2.0",
            ),
        )
        .expect("build outcome request evidence")
    }

    fn outcome_claim_stage() -> OutcomeClaimStageInputPreimage {
        let outcome = outcome_stage(OutcomeAttemptResult::ExpectedWait);
        let request_columns = outcome_request_columns(
            &outcome.sample_key,
            outcome.outcome_phase,
            &outcome.stored_due_date,
        );
        let provider_request_evidence: RequestEvidencePreimage =
            serde_json::from_str(&request_columns.request_evidence_json)
                .expect("decode typed outcome claim request");
        let object_binding = VerifiedOutcomeDueDatabaseObjectBindingPreimage {
            domain: DOMAIN_OUTCOME_DUE_DATABASE_OBJECT.into(),
            manifest_root_canonical_path: "/TEST_CODE/selection-root".into(),
            manifest_root_device: 1,
            manifest_root_inode: 2,
            manifest_root_mode: 0o40700,
            database_relative_path: "TEST_CODE/selection.db".into(),
            database_device: 1,
            database_inode: 3,
            database_mode: 0o100600,
        };
        let database_binding = VerifiedOutcomeDueDatabaseBindingPreimage {
            domain: DOMAIN_OUTCOME_DUE_DATABASE_BINDING.into(),
            scope: "test".into(),
            object_binding_hash: hash(&object_binding).expect("hash test database object"),
            object_binding,
            database_relative_path: "TEST_CODE/selection.db".into(),
            sqlite_application_id: 1,
            sqlite_user_version: 1,
            sqlite_schema_hash: test_hash('1'),
            receipt_snapshot_high_water_rowid: 3,
            receipt_snapshot_high_water_content_hash: Some(test_hash('2')),
        };
        let trading_date_vector = OutcomeTradingDateVectorPreimage {
            domain: crate::selection::schema_v2::DOMAIN_OUTCOME_TRADING_DATE_VECTOR.into(),
            t0: "2026-07-28".into(),
            d1: "2026-07-29".into(),
            d2: "2026-07-30".into(),
            d3: "2026-07-31".into(),
            d4: "2026-08-03".into(),
            d5: "2026-08-04".into(),
        };
        let trading_date_vector_hash =
            hash(&trading_date_vector).expect("hash claim trading-date vector");
        let due_snapshot = VerifiedOutcomeDueSnapshotPreimage {
            domain: DOMAIN_VERIFIED_OUTCOME_DUE_SNAPSHOT.into(),
            database_binding_hash: hash(&database_binding).expect("hash test database binding"),
            database_binding,
            selection_audit_high_water_record_ordinal: 0,
            selection_audit_high_water_record_hash: test_hash('3'),
            selection_audit_prefix_hash: test_hash('4'),
            receipt_tuples_sorted: Vec::new(),
            sample_key_preimage: outcome.sample_key_preimage.clone(),
            sample_key: outcome.sample_key.clone(),
            logical_subject_key: outcome.logical_subject_key.clone(),
            canonical_stock_code: outcome.sample_key_preimage.stock_code.clone(),
            canonical_market: "SZ".into(),
            config_activation_run_id: outcome.config_activation_run_id.clone(),
            config_hash: outcome.config_hash.clone(),
            outcome_phase: outcome.outcome_phase,
            stored_due_date: outcome.stored_due_date.clone(),
            calendar_version: "calendar-v1".into(),
            calendar_hash: test_hash('e'),
            trading_date_vector: trading_date_vector.clone(),
            trading_date_vector_hash: trading_date_vector_hash.clone(),
            applicable_trading_dates: vec!["2026-07-28".into()],
            expected_provider_bar_count: 1,
            provider_request_hash: request_columns.request_hash.clone(),
            t0_outcome_content_hash: None,
            t0_close: None,
            t0_volume: None,
        };
        let due_binding = OutcomeClaimDueBindingPreimage {
            domain: DOMAIN_OUTCOME_CLAIM_DUE_BINDING.into(),
            verified_due_snapshot_hash: hash(&due_snapshot).expect("hash test due snapshot"),
            verified_due_snapshot: due_snapshot,
            same_subject_high_water_receipt_hash: None,
            outcome_attempt_ordinal: 1,
            previous_same_subject_attempt_receipt_hashes: Vec::new(),
            selection_audit_high_water_record_hash: test_hash('3'),
            sample_key_preimage: outcome.sample_key_preimage,
            sample_key: outcome.sample_key,
            canonical_stock_code: "TEST_CODE_000001".into(),
            canonical_market: "SZ".into(),
            config_activation_run_id: outcome.config_activation_run_id.clone(),
            config_hash: outcome.config_hash.clone(),
            config_activation_receipt_hash: test_hash('5'),
            source_ingress_run_id: TEST_INGRESS_RUN_ID.into(),
            source_ingress_receipt_hash: test_hash('6'),
            generation_run_id: TEST_GENERATION_RUN_ID.into(),
            generation_receipt_hash: test_hash('7'),
            outcome_phase: outcome.outcome_phase,
            t0_market_date: "2026-07-28".into(),
            stored_due_date: outcome.stored_due_date,
            calendar_version: "calendar-v1".into(),
            calendar_hash: test_hash('e'),
            trading_date_vector,
            trading_date_vector_hash,
            applicable_trading_dates: vec!["2026-07-28".into()],
            expected_provider_bar_count: 1,
            preceding_outcome_receipt_hashes: Vec::new(),
            t0_outcome_content_hash: None,
            t0_close: None,
            t0_volume: None,
        };
        let provider_transport_request = OutcomeProviderRequestPreimage {
            domain: DOMAIN_OUTCOME_PROVIDER_REQUEST.into(),
            design_sha256: OUTCOME_PARENT_DESIGN_SHA256.into(),
            amendment_design_sha256: AMENDMENT_DESIGN_SHA256.into(),
            semantic_request_hash: request_columns.request_hash.clone(),
            verified_due_binding_hash: due_binding.verified_due_snapshot_hash.clone(),
            sample_key: due_binding.sample_key.clone(),
            canonical_stock_code: due_binding.canonical_stock_code.clone(),
            canonical_market: due_binding.canonical_market.clone(),
            phase: due_binding.outcome_phase,
            stored_due_date: due_binding.stored_due_date.clone(),
            window_start: "2026-07-28".into(),
            window_end: due_binding.stored_due_date.clone(),
            expected_bar_count: 1,
            calendar_version: due_binding.calendar_version.clone(),
            calendar_hash: due_binding.calendar_hash.clone(),
            trading_date_vector: due_binding.trading_date_vector.clone(),
            trading_date_vector_hash: due_binding.trading_date_vector_hash.clone(),
            expected_trading_dates: due_binding.applicable_trading_dates.clone(),
            receipted_t0_close: None,
            receipted_t0_volume_shares: None,
            request_local_date: "2026-07-29".into(),
            post_close_cutoff: "15:00:00".into(),
            interval: DailyIntervalKind::Day.as_str().into(),
            adjustment: AdjustmentKind::None.as_str().into(),
            acquisition_strategy:
                "phase-minimum_then_exponential-growth_then-cardinality-bisection".into(),
            adaptive_policy_version: OUTCOME_ADAPTIVE_POLICY_VERSION.into(),
            maximum_latest_n: 2,
            volume_conversion_contract: "TEST_CODE_VOLUME_CONTRACT".into(),
            volume_conversion_version: "TEST_CODE_VOLUME_VERSION".into(),
            shares_per_board_lot: "100".into(),
        };
        OutcomeClaimStageInputPreimage {
            domain: DOMAIN_OUTCOME_CLAIM_STAGE.into(),
            stage_run_id: TEST_OUTCOME_CLAIM_ID.into(),
            logical_subject_key: outcome.logical_subject_key.clone(),
            config_activation_run_id: outcome.config_activation_run_id,
            config_hash: outcome.config_hash,
            planned_outcome_run_id: TEST_OUTCOME_RUN_ID.into(),
            due_binding_hash: hash(&due_binding).expect("hash test due binding"),
            due_binding,
            provider_request_hash: request_columns.request_hash,
            provider_request_evidence,
            provider_transport_request_hash: hash(&provider_transport_request)
                .expect("hash test provider transport request"),
            provider_transport_request,
            claim_lock_key: outcome.logical_subject_key,
            planned_run_status: RunStatus::Claimed,
        }
    }

    struct TestAuditRoot(PathBuf);

    impl TestAuditRoot {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let canonical_temp_root = std::fs::canonicalize(std::env::temp_dir())
                .expect("resolve isolated TEST_CODE temp root without symlink components");
            let path = canonical_temp_root.join(format!(
                "stock-analysis-selection-v2-repository-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestAuditRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn test_audit_writer(root: &TestAuditRoot) -> SelectionAuditWriter {
        SelectionAuditWriter::for_test_code_root(root.path())
            .expect("construct isolated TEST_CODE audit writer")
    }

    fn repository() -> (SqliteConnection, SelectionV2Repository) {
        let mut conn =
            SqliteConnection::establish(":memory:").expect("open isolated in-memory SQLite");
        let repository = SelectionV2Repository::initialize_for_final_database_half_test(
            &mut conn,
            SelectionV2StoreMode::Test,
        )
        .expect("install exact final selection-v2 database-half test catalog");
        (conn, repository)
    }

    fn file_repository(root: &TestAuditRoot) -> (PathBuf, SqliteConnection, SelectionV2Repository) {
        let database_root = root.path().join("database");
        std::fs::create_dir_all(&database_root)
            .expect("create isolated TEST_CODE database namespace");
        let database_path = database_root.join("selection-v2.db");
        let database_url = database_path
            .to_str()
            .expect("isolated TEST_CODE database path must be UTF-8");
        let mut conn =
            SqliteConnection::establish(database_url).expect("open isolated file SQLite");
        let repository = SelectionV2Repository::initialize_for_final_database_half_test(
            &mut conn,
            SelectionV2StoreMode::Test,
        )
        .expect("install exact final selection-v2 file catalog");
        (database_path, conn, repository)
    }

    fn seed_receipted_source_fact(conn: &mut SqliteConnection) {
        let config_hash = test_hash('5');
        let source_fact_key = test_hash('3');
        let source_fact_content_hash = test_hash('6');
        let event_id = test_hash('4');
        let available_batch_content_hash = test_hash('7');
        let available_evidence_hash = test_hash('8');
        let sql = format!(
            r#"
            INSERT INTO selection_v2_recovery_envelopes (
                stage_run_id,subject_kind,logical_subject_key,payload_schema,payload_json,
                payload_json_hash,in_memory_payload_hash,config_activation_run_id,config_hash,
                enveloped_at,content_hash
            ) VALUES (
                '{TEST_ACTIVATION_RUN_ID}','config_activation','{logical_activation}',
                'config-activation-stage-v1',
                '{{"domain":"stock_analysis.br174.config_activation_stage.v1"}}',
                '{payload_hash}','{memory_hash}',
                '{TEST_ACTIVATION_RUN_ID}','{config_hash}',
                '2026-07-28T00:00:00.000000000Z','{activation_envelope_hash}'
            );
            INSERT INTO selection_v2_run_stages (
                subject_kind,subject_id,in_memory_payload_hash,prepared_record_hash,
                expected_staged_row_count,staged_db_content_hash,recovery_envelope_content_hash,
                logical_subject_key,run_status,source_fact_key,config_activation_run_id,
                config_hash,config_snapshot_json_hash,config_activation_content_hash,
                config_activation_file_content_hash,config_effective_from,artifact_valid_from,
                artifact_expires_at,executable_revision,legacy_cutover_snapshot_hash,
                generation_market_date,aggregator_observed_at,
                ingress_source_batch_content_hash,outcome_phase,stored_due_date,staged_at,
                manifest_content_hash
            ) VALUES (
                'config_activation','{TEST_ACTIVATION_RUN_ID}','{memory_hash}',
                '{prepared_hash}',1,'{staged_hash}','{activation_envelope_hash}',
                '{logical_activation}','activated',NULL,'{TEST_ACTIVATION_RUN_ID}',
                '{config_hash}','{snapshot_hash}','{activation_content_hash}',
                '{activation_file_hash}','2026-07-28T00:00:00.000000000Z',
                '2026-07-28','2027-07-28','TEST_CODE_REVISION','{legacy_hash}',
                NULL,NULL,NULL,NULL,NULL,'2026-07-28T00:00:01.000000000Z',
                '{activation_manifest_hash}'
            );
            INSERT INTO selection_v2_commit_receipts (
                subject_kind,subject_id,logical_subject_key,in_memory_payload_hash,
                recovery_envelope_content_hash,prepared_audit_hash,
                run_manifest_content_hash,staged_db_content_hash,committed_audit_hash,
                committed_at,content_hash
            ) VALUES (
                'config_activation','{TEST_ACTIVATION_RUN_ID}','{logical_activation}',
                '{memory_hash}','{activation_envelope_hash}','{prepared_hash}',
                '{activation_manifest_hash}','{staged_hash}','{committed_hash}',
                '2026-07-28T00:00:02.000000000Z','{activation_receipt_hash}'
            );

            INSERT INTO selection_v2_recovery_envelopes (
                stage_run_id,subject_kind,logical_subject_key,payload_schema,payload_json,
                payload_json_hash,in_memory_payload_hash,config_activation_run_id,config_hash,
                enveloped_at,content_hash
            ) VALUES (
                '{TEST_INGRESS_RUN_ID}','ingress_run','{logical_ingress}',
                'source-ingress-stage-v2',
                '{{"domain":"stock_analysis.br174.source_ingress_stage.v2"}}',
                '{payload_hash}','{memory_hash}',
                '{TEST_ACTIVATION_RUN_ID}','{config_hash}',
                '2026-07-28T00:00:03.000000000Z','{ingress_envelope_hash}'
            );
            INSERT INTO selection_source_batch_attempts (
                source_batch_attempt_id,ingress_run_id,config_activation_run_id,config_hash,
                generation_market_date,registered_feed_identity,registered_feed_snapshot_hash,
                request_hash,request_evidence_json,request_evidence_hash,
                feed_attempt_content_hash,status_kind,record_count,provider,source,source_at,
                observed_at,batch_id,batch_content_hash,failed_stage,reason_code,retryable,
                available_evidence_json,available_evidence_hash,error_detail_json,
                error_detail_hash,error_fingerprint,attempted_at,content_hash
            ) VALUES
            (
                '{TEST_SOURCE_BATCH_ATTEMPT_ID}','{TEST_INGRESS_RUN_ID}',
                '{TEST_ACTIVATION_RUN_ID}','{config_hash}','2026-07-28','TEST_CODE_FEED_1',
                '{feed_snapshot_hash}','{request_hash}','{{}}','{request_evidence_hash}',
                '{feed_attempt_hash}','available',1,'eastmoney','global-news',
                '2026-07-28T00:00:00.000000000Z',
                '2026-07-28T00:00:01.000000000Z','TEST_CODE_PROVIDER_BATCH',
                '{available_batch_content_hash}',NULL,NULL,NULL,'{{}}',
                '{available_evidence_hash}',NULL,NULL,NULL,
                '2026-07-28T00:00:04.000000000Z','{batch_row_hash}'
            ),
            (
                'TEST_CODE_EMPTY_2','{TEST_INGRESS_RUN_ID}','{TEST_ACTIVATION_RUN_ID}',
                '{config_hash}','2026-07-28','TEST_CODE_FEED_2','{feed_snapshot_hash}',
                '{request_hash}','{{}}','{request_evidence_hash}','{feed_attempt_hash}',
                'verified_empty',0,'cailianpress','global-news',
                '2026-07-28T00:00:00.000000000Z',
                '2026-07-28T00:00:01.000000000Z','TEST_CODE_EMPTY_BATCH_2',
                '{empty_batch_2}',NULL,NULL,NULL,'{{}}','{empty_evidence_2}',
                NULL,NULL,NULL,'2026-07-28T00:00:04.000000000Z','{empty_row_2}'
            ),
            (
                'TEST_CODE_EMPTY_3','{TEST_INGRESS_RUN_ID}','{TEST_ACTIVATION_RUN_ID}',
                '{config_hash}','2026-07-28','TEST_CODE_FEED_3','{feed_snapshot_hash}',
                '{request_hash}','{{}}','{request_evidence_hash}','{feed_attempt_hash}',
                'verified_empty',0,'jin10','global-news',
                '2026-07-28T00:00:00.000000000Z',
                '2026-07-28T00:00:01.000000000Z','TEST_CODE_EMPTY_BATCH_3',
                '{empty_batch_3}',NULL,NULL,NULL,'{{}}','{empty_evidence_3}',
                NULL,NULL,NULL,'2026-07-28T00:00:04.000000000Z','{empty_row_3}'
            ),
            (
                'TEST_CODE_EMPTY_4','{TEST_INGRESS_RUN_ID}','{TEST_ACTIVATION_RUN_ID}',
                '{config_hash}','2026-07-28','TEST_CODE_FEED_4','{feed_snapshot_hash}',
                '{request_hash}','{{}}','{request_evidence_hash}','{feed_attempt_hash}',
                'verified_empty',0,'thepaper','global-news',
                '2026-07-28T00:00:00.000000000Z',
                '2026-07-28T00:00:01.000000000Z','TEST_CODE_EMPTY_BATCH_4',
                '{empty_batch_4}',NULL,NULL,NULL,'{{}}','{empty_evidence_4}',
                NULL,NULL,NULL,'2026-07-28T00:00:04.000000000Z','{empty_row_4}'
            );
            INSERT INTO selection_source_facts_v2 (
                source_fact_key,event_id,payload_schema,config_activation_run_id,config_hash,
                generation_market_date,provider_source,item_id,title,summary,content,publisher,
                canonical_url,published_at,instruments_json,topics_json,language,
                record_provider,record_source,record_source_at,record_observed_at,
                record_batch_id,record_batch_content_hash,provider_content_hash,
                first_ingress_run_id,ingress_gate_version,ingress_gate_input_json,
                ingress_gate_input_hash,ingress_decision,ingress_reason_code,ingress_retryable,
                ingress_gate_receipt_json,ingress_gate_receipt_hash,content_hash
            ) VALUES (
                '{source_fact_key}','{event_id}','global-news-source-fact-v2',
                '{TEST_ACTIVATION_RUN_ID}','{config_hash}','2026-07-28','eastmoney',
                'TEST_CODE_ITEM','TEST_CODE_TITLE',NULL,NULL,'TEST_CODE_PUBLISHER',
                'https://example.invalid/TEST_CODE_ITEM',
                '2026-07-28T00:00:00.000000000Z','[]','[]','zh','eastmoney',
                'global-news','2026-07-28T00:00:00.000000000Z',
                '2026-07-28T00:00:01.000000000Z','TEST_CODE_PROVIDER_BATCH',
                '{available_batch_content_hash}','{provider_content_hash}',
                '{TEST_INGRESS_RUN_ID}','ingress-gate-v1','{{}}','{gate_input_hash}',
                'admitted',NULL,NULL,'{{}}','{gate_receipt_hash}',
                '{source_fact_content_hash}'
            );
            INSERT INTO selection_source_fact_attempts (
                source_fact_attempt_id,ingress_run_id,source_batch_attempt_id,
                provider_ordinal,source_fact_key,acquired_record_json,acquired_record_hash,
                batch_evidence_json,batch_evidence_hash,event_projection_id,attempt_result,
                conflict_hash,attempted_at,content_hash
            ) VALUES (
                '{TEST_SOURCE_FACT_ATTEMPT_ID}','{TEST_INGRESS_RUN_ID}',
                '{TEST_SOURCE_BATCH_ATTEMPT_ID}',0,'{source_fact_key}','{{}}',
                '{acquired_record_hash}','{{}}','{available_evidence_hash}',
                '{event_id}','inserted',NULL,'2026-07-28T00:00:05.000000000Z',
                '{fact_attempt_hash}'
            );
            INSERT INTO selection_v2_run_stages (
                subject_kind,subject_id,in_memory_payload_hash,prepared_record_hash,
                expected_staged_row_count,staged_db_content_hash,recovery_envelope_content_hash,
                logical_subject_key,run_status,source_fact_key,config_activation_run_id,
                config_hash,config_snapshot_json_hash,config_activation_content_hash,
                config_activation_file_content_hash,config_effective_from,artifact_valid_from,
                artifact_expires_at,executable_revision,legacy_cutover_snapshot_hash,
                generation_market_date,aggregator_observed_at,
                ingress_source_batch_content_hash,outcome_phase,stored_due_date,staged_at,
                manifest_content_hash
            ) VALUES (
                'ingress_run','{TEST_INGRESS_RUN_ID}','{memory_hash}','{prepared_hash}',7,
                '{staged_hash}','{ingress_envelope_hash}','{logical_ingress}','completed',
                NULL,'{TEST_ACTIVATION_RUN_ID}','{config_hash}',NULL,NULL,NULL,NULL,NULL,NULL,
                NULL,NULL,'2026-07-28','2026-07-28T00:00:01.000000000Z',
                '{ingress_batch_hash}',NULL,NULL,'2026-07-28T00:00:06.000000000Z',
                '{ingress_manifest_hash}'
            );
            INSERT INTO selection_v2_commit_receipts (
                subject_kind,subject_id,logical_subject_key,in_memory_payload_hash,
                recovery_envelope_content_hash,prepared_audit_hash,
                run_manifest_content_hash,staged_db_content_hash,committed_audit_hash,
                committed_at,content_hash
            ) VALUES (
                'ingress_run','{TEST_INGRESS_RUN_ID}','{logical_ingress}','{memory_hash}',
                '{ingress_envelope_hash}','{prepared_hash}','{ingress_manifest_hash}',
                '{staged_hash}','{committed_hash}',
                '2026-07-28T00:00:07.000000000Z','{ingress_receipt_hash}'
            );
            "#,
            logical_activation = test_hash('a'),
            logical_ingress = test_hash('b'),
            payload_hash = test_hash('c'),
            memory_hash = test_hash('d'),
            activation_envelope_hash = test_hash('e'),
            ingress_envelope_hash = test_hash('f'),
            prepared_hash = test_hash('1'),
            staged_hash = test_hash('2'),
            snapshot_hash = test_hash('3'),
            activation_content_hash = test_hash('4'),
            activation_file_hash = test_hash('6'),
            legacy_hash = test_hash('7'),
            activation_manifest_hash = test_hash('8'),
            committed_hash = test_hash('9'),
            activation_receipt_hash = test_hash('a'),
            ingress_manifest_hash = test_hash('b'),
            ingress_receipt_hash = test_hash('c'),
            feed_snapshot_hash = test_hash('d'),
            request_hash = test_hash('e'),
            request_evidence_hash = test_hash('0'),
            feed_attempt_hash = test_hash('f'),
            batch_row_hash = test_hash('1'),
            empty_batch_2 = test_hash('2'),
            empty_batch_3 = test_hash('3'),
            empty_batch_4 = test_hash('4'),
            empty_evidence_2 = test_hash('5'),
            empty_evidence_3 = test_hash('6'),
            empty_evidence_4 = test_hash('7'),
            empty_row_2 = test_hash('8'),
            empty_row_3 = test_hash('9'),
            empty_row_4 = test_hash('a'),
            provider_content_hash = test_hash('b'),
            gate_input_hash = test_hash('c'),
            gate_receipt_hash = test_hash('d'),
            acquired_record_hash = test_hash('e'),
            fact_attempt_hash = test_hash('f'),
            ingress_batch_hash = test_hash('1'),
        );
        conn.batch_execute(&sql)
            .expect("seed receipted TEST_CODE source fact");
    }

    fn validated_envelope(stage_run_id: &str, payload_json: &str) -> ValidatedRecoveryEnvelope {
        let mut payload: serde_json::Value =
            serde_json::from_str(payload_json).expect("parse TEST_CODE envelope payload");
        payload
            .as_object_mut()
            .expect("TEST_CODE envelope payload is an object")
            .insert(
                "domain".into(),
                serde_json::Value::String(DOMAIN_CONFIG_ACTIVATION_STAGE.into()),
            );
        let payload_json = canonical(&payload).expect("canonical TEST_CODE envelope payload");
        let envelope = SelectionRecoveryEnvelopeRowContentPreimage {
            domain: DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
            stage_run_id: stage_run_id.into(),
            subject_kind: SubjectKind::ConfigActivation,
            logical_subject_key: "TEST_CODE_LOGICAL_CONFIG".into(),
            payload_schema: CONFIG_ACTIVATION_PAYLOAD_SCHEMA.into(),
            payload_json: payload_json.clone(),
            payload_json_hash: crate::selection::schema_v2::sha256_bytes(payload_json.as_bytes()),
            in_memory_payload_hash: "1".repeat(64),
            config_activation_run_id: stage_run_id.into(),
            config_hash: "2".repeat(64),
            enveloped_at: "2026-07-28T01:00:00.000000000Z".into(),
        };
        let content_hash = hash(&envelope).expect("hash test envelope");
        ValidatedRecoveryEnvelope {
            envelope,
            content_hash,
        }
    }

    fn verified_no_relation_stage() -> GenerationStageInputPreimage {
        let source_fact_key = test_hash('3');
        let config_hash = test_hash('5');
        let logical_subject_key = run_logical_subject_key(&RunLogicalSubjectPreimage {
            domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
            subject_kind: SubjectKind::GenerationRun,
            source_fact_key: Some(source_fact_key.clone()),
            config_hash: Some(config_hash.clone()),
            sample_key: None,
            outcome_phase: None,
            stored_due_date: None,
            ingress_source_batch_hash: None,
        })
        .expect("hash generation logical subject");
        GenerationStageInputPreimage {
            domain: DOMAIN_GENERATION_STAGE.into(),
            stage_run_id: TEST_GENERATION_RUN_ID.into(),
            logical_subject_key,
            source_fact_key,
            source_fact_content_hash: test_hash('6'),
            config_activation_run_id: TEST_ACTIVATION_RUN_ID.into(),
            config_hash,
            generation_market_date: "2026-07-28".into(),
            relation_attempt_rows: vec![],
            evaluation_attempt_rows: vec![],
            sample_rows: vec![],
            rejection_rows: vec![],
            planned_run_status: RunStatus::VerifiedNoRelation,
        }
    }

    fn hard_rejected_generation_stage() -> GenerationStageInputPreimage {
        let direct_source = DirectMentionSourcePreimage {
            domain: DOMAIN_DIRECT_SOURCE.into(),
            source_fact_key: test_hash('3'),
            field: DirectMentionField::Title,
            mention_kind: MentionKind::ExactCode,
            normalized_value: "TEST_CODE_000001".into(),
            byte_start: 0,
            byte_end: 16,
        };
        let binding = BindingStatePreimage {
            domain: DOMAIN_BINDING_STATE.into(),
            state: BindingStateKind::DirectNotApplicable,
            artifact_content_hash: None,
            binding_audit_hash: None,
            provider: None,
            kind: None,
            code: None,
            name: None,
            error_fingerprint: None,
        };
        let raw_identity = RawSecurityIdentityPreimage {
            domain: crate::selection::schema_v2::DOMAIN_RAW_SECURITY_IDENTITY.into(),
            provider: "source-event".into(),
            exchange: "SZ".into(),
            code: "TEST_CODE_000001".into(),
            asset_class: "equity".into(),
        };
        let relation = SelectionRelationAttemptRowContentPreimage {
            domain: DOMAIN_RELATION_ATTEMPT_ROW.into(),
            relation_attempt_id: test_hash('a'),
            relation_key: test_hash('b'),
            generation_run_id: TEST_GENERATION_RUN_ID.into(),
            source_fact_key: test_hash('3'),
            event_id: test_hash('4'),
            chain_id: "TEST_CODE_CHAIN".into(),
            config_activation_run_id: TEST_ACTIVATION_RUN_ID.into(),
            config_hash: test_hash('5'),
            relation_schema_version: "event-relation-v2".into(),
            relation_kind: RelationKind::DirectMention,
            relation_source_identity_json: canonical(&direct_source).expect("direct source JSON"),
            relation_source_identity_hash: hash(&direct_source).expect("direct source hash"),
            typed_binding_state_json: canonical(&binding).expect("binding JSON"),
            typed_binding_state_hash: hash(&binding).expect("binding hash"),
            request_hash: None,
            request_evidence_json: None,
            request_evidence_hash: None,
            result_code: "resolved".into(),
            failed_stage: None,
            retryable: None,
            raw_identity_json: Some(canonical(&raw_identity).expect("raw identity JSON")),
            raw_identity_hash: Some(hash(&raw_identity).expect("raw identity hash")),
            canonical_stock_code: Some("TEST_CODE_000001".into()),
            canonical_stock_name: Some("TEST_CODE_NAME".into()),
            canonical_market: Some("SZ".into()),
            artifact_content_hash: None,
            binding_audit_hash: None,
            provider_board_kind: None,
            provider_board_code: None,
            provider_board_name: None,
            provider_source: None,
            provider_source_at: None,
            provider_observed_at: None,
            provider_batch_id: None,
            provider_batch_content_hash: None,
            actual_constituent_count: None,
            available_evidence_json: None,
            available_evidence_hash: None,
            error_detail_json: None,
            error_detail_hash: None,
            error_fingerprint: None,
            attempted_at: "2026-07-28T01:00:00.000000000Z".into(),
        };
        let relation_evidence = RelationEvidenceSetPreimage {
            domain: DOMAIN_RELATION_EVIDENCE_SET.into(),
            source_fact_key: relation.source_fact_key.clone(),
            event_id: relation.event_id.clone(),
            chain_id: relation.chain_id.clone(),
            canonical_stock_code: relation.canonical_stock_code.clone().expect("stock code"),
            entries_in_relation_order: vec![RelationEvidenceEntryPreimage {
                relation_rank: 0,
                relation_key: relation.relation_key.clone(),
                relation_kind: relation.relation_kind,
                relation_attempt_id: relation.relation_attempt_id.clone(),
                relation_attempt_content_hash: hash(&relation).expect("relation row hash"),
            }],
        };
        let feature = T0FeaturePreimage {
            domain: DOMAIN_T0_FEATURE.into(),
            feature_version: "feature-v1".into(),
            evaluation_window: EvaluationWindow::PostClose,
            ma5: Some("10".into()),
            ma10: Some("9.8".into()),
            ma20: Some("9.5".into()),
            five_day_return: Some("0.05".into()),
            volume_vs_5d: Some("1.2".into()),
            volume_vs_20d: Some("1.1".into()),
            intraday_volume_pace: None,
            price_vs_ma5: Some("0.95".into()),
            price_vs_ma10: Some("1.07".into()),
            price_vs_ma20: Some("1.1".into()),
            evaluation_price: "9.5".into(),
            observed_volume: "1000".into(),
            latest_settled_market_date: "2026-07-28".into(),
            latest_settled_close: "9.5".into(),
            latest_settled_volume: "1000".into(),
            prior_5d_average_volume: "900".into(),
            prior_20d_average_volume: "850".into(),
        };
        let sample_key = test_sample_key();
        let details = [
            AdmissionStructuredDetailPreimage::PriceBelowMa5 {
                value: "0.95".into(),
                inclusive_min: "1".into(),
            },
            AdmissionStructuredDetailPreimage::FiveDayReturnOutOfRange {
                value: "0.05".into(),
                inclusive_min: "0.10".into(),
                inclusive_max: "0.20".into(),
            },
        ];
        let rejection_rows = details
            .into_iter()
            .enumerate()
            .map(|(ordinal, detail)| SelectionRejectionRowContentPreimage {
                domain: DOMAIN_REJECTION_ROW.into(),
                sample_key: sample_key.clone(),
                ordinal: u32::try_from(ordinal).expect("small ordinal"),
                generation_run_id: TEST_GENERATION_RUN_ID.into(),
                reason_code: match ordinal {
                    0 => "price_below_ma5",
                    _ => "five_day_return_out_of_range",
                }
                .into(),
                rule_id: format!("BR-178.TEST_CODE.{ordinal}"),
                retryable: false,
                structured_detail_json: canonical(&detail).expect("rejection detail JSON"),
                structured_detail_hash: hash(&detail).expect("rejection detail hash"),
                provider: None,
                source: None,
                source_at: None,
                observed_at: None,
                batch_id: None,
                batch_content_hash: None,
                created_at: "2026-07-28T01:00:00.000000000Z".into(),
            })
            .collect::<Vec<_>>();
        let mut sample = SelectionSampleRowContentPreimage {
            domain: DOMAIN_SAMPLE_ROW.into(),
            sample_key,
            generation_run_id: TEST_GENERATION_RUN_ID.into(),
            source_fact_key: test_hash('3'),
            source_fact_content_hash: test_hash('6'),
            source_fact_attempt_id: TEST_SOURCE_FACT_ATTEMPT_ID.into(),
            source_batch_attempt_id: TEST_SOURCE_BATCH_ATTEMPT_ID.into(),
            event_id: test_hash('4'),
            chain_id: "TEST_CODE_CHAIN".into(),
            config_activation_run_id: TEST_ACTIVATION_RUN_ID.into(),
            config_hash: test_hash('5'),
            matched_keyword: "TEST_CODE".into(),
            canonical_stock_code: "TEST_CODE_000001".into(),
            canonical_stock_name: "TEST_CODE_NAME".into(),
            canonical_market: "SZ".into(),
            relation_schema_version: "event-relation-v2".into(),
            relation_evidence_json: canonical(&relation_evidence).expect("relation evidence JSON"),
            relation_evidence_set_hash: hash(&relation_evidence).expect("relation evidence hash"),
            feature_version: feature.feature_version.clone(),
            t0_feature_json: canonical(&feature).expect("feature JSON"),
            t0_feature_hash: hash(&feature).expect("feature hash"),
            market_provider: "magic-tdx".into(),
            market_source: "tdx".into(),
            market_source_at: Some("2026-07-28T00:59:00.000000000Z".into()),
            market_observed_at: "2026-07-28T01:00:00.000000000Z".into(),
            market_batch_id: "TEST_CODE_MARKET_BATCH".into(),
            market_batch_content_hash: test_hash('d'),
            admission_version: "admission-v1".into(),
            decision_kind: TerminalDecisionKind::HardRejected,
            rejection_count: 2,
            rejection_row_hashes_in_ordinal_order: Vec::new(),
            evaluation_market_date: "2026-07-28".into(),
            t0_due_date: "2026-07-28".into(),
            d1_due_date: "2026-07-29".into(),
            d2_due_date: "2026-07-30".into(),
            d3_due_date: "2026-07-31".into(),
            d4_due_date: "2026-08-03".into(),
            d5_due_date: "2026-08-04".into(),
            calendar_version: "calendar-v1".into(),
            calendar_hash: test_hash('e'),
            trading_date_vector_json: canonical(&OutcomeTradingDateVectorPreimage {
                domain: crate::selection::schema_v2::DOMAIN_OUTCOME_TRADING_DATE_VECTOR.into(),
                t0: "2026-07-28".into(),
                d1: "2026-07-29".into(),
                d2: "2026-07-30".into(),
                d3: "2026-07-31".into(),
                d4: "2026-08-03".into(),
                d5: "2026-08-04".into(),
            })
            .expect("canonical test trading-date vector"),
            trading_date_vector_hash: hash(&OutcomeTradingDateVectorPreimage {
                domain: crate::selection::schema_v2::DOMAIN_OUTCOME_TRADING_DATE_VECTOR.into(),
                t0: "2026-07-28".into(),
                d1: "2026-07-29".into(),
                d2: "2026-07-30".into(),
                d3: "2026-07-31".into(),
                d4: "2026-08-03".into(),
                d5: "2026-08-04".into(),
            })
            .expect("hash test trading-date vector"),
            staged_at: "2026-07-28T01:00:00.000000000Z".into(),
        };
        sample.rejection_row_hashes_in_ordinal_order = rejection_rows
            .iter()
            .map(|row| hash(row).expect("rejection row hash"))
            .collect();
        let evaluation_available = ProviderAvailableEvidencePreimage {
            domain: DOMAIN_PROVIDER_AVAILABLE_EVIDENCE.into(),
            evidence_kind: ProviderEvidenceKind::T0MarketBundle,
            provider: "magic-tdx".into(),
            source: Some("tdx".into()),
            source_at: sample.market_source_at.clone(),
            observed_at: Some(sample.market_observed_at.clone()),
            batch_id: Some(sample.market_batch_id.clone()),
            batch_content_hash: Some(sample.market_batch_content_hash.clone()),
        };
        let market_request = t0_request_columns(&sample);
        let evaluation = SelectionEvaluationAttemptRowContentPreimage {
            domain: DOMAIN_EVALUATION_ATTEMPT_ROW.into(),
            evaluation_attempt_id: test_hash('f'),
            sample_key: sample.sample_key.clone(),
            generation_run_id: TEST_GENERATION_RUN_ID.into(),
            source_fact_key: test_hash('3'),
            event_id: test_hash('4'),
            chain_id: "TEST_CODE_CHAIN".into(),
            canonical_stock_code: sample.canonical_stock_code.clone(),
            canonical_stock_name: sample.canonical_stock_name.clone(),
            canonical_market: sample.canonical_market.clone(),
            relation_evidence_set_hash: sample.relation_evidence_set_hash.clone(),
            market_request_hash: market_request.request_hash,
            request_evidence_json: market_request.request_evidence_json,
            request_evidence_hash: market_request.request_evidence_hash,
            result_code: "completed".into(),
            failed_stage: None,
            retryable: None,
            provider: Some("magic-tdx".into()),
            source: Some("tdx".into()),
            source_at: sample.market_source_at.clone(),
            observed_at: Some(sample.market_observed_at.clone()),
            batch_id: Some(sample.market_batch_id.clone()),
            batch_content_hash: Some(sample.market_batch_content_hash.clone()),
            available_evidence_json: Some(
                canonical(&evaluation_available).expect("evaluation evidence JSON"),
            ),
            available_evidence_hash: Some(
                hash(&evaluation_available).expect("evaluation evidence hash"),
            ),
            terminal_decision_hash: Some(hash(&sample).expect("terminal sample hash")),
            error_detail_json: None,
            error_detail_hash: None,
            error_fingerprint: None,
            attempted_at: "2026-07-28T01:00:00.000000000Z".into(),
        };
        GenerationStageInputPreimage {
            relation_attempt_rows: vec![relation],
            evaluation_attempt_rows: vec![evaluation],
            sample_rows: vec![sample],
            rejection_rows,
            planned_run_status: RunStatus::Completed,
            ..verified_no_relation_stage()
        }
    }

    fn generation_request(stage_input: GenerationStageInputPreimage) -> GenerationStageRequest {
        let rows =
            generation_run_row_hashes(&stage_input).expect("canonical generation row hashes");
        let run_payload = RunPayloadPreimage {
            domain: DOMAIN_GENERATION_PAYLOAD.into(),
            subject_kind: SubjectKind::GenerationRun,
            subject_id: stage_input.stage_run_id.clone(),
            logical_subject_key: stage_input.logical_subject_key.clone(),
            source_fact_key: Some(stage_input.source_fact_key.clone()),
            config_activation_run_id: stage_input.config_activation_run_id.clone(),
            config_hash: stage_input.config_hash.clone(),
            config_snapshot_json_hash: None,
            config_activation_content_hash: None,
            config_activation_file_content_hash: None,
            config_effective_from_rfc3339_nanos_utc: None,
            artifact_valid_from: None,
            artifact_expires_at: None,
            executable_revision: None,
            legacy_cutover_snapshot_hash: None,
            generation_market_date: Some(stage_input.generation_market_date.clone()),
            aggregator_observed_at_rfc3339_nanos_utc: None,
            ingress_source_batch_content_hash: None,
            outcome_phase: None,
            stored_due_date: None,
            outcome_claim_id: None,
            planned_outcome_run_id: None,
            outcome_claim_receipt_content_hash: None,
            outcome_claim_due_binding_hash: None,
            outcome_claim_provider_request_hash: None,
            rows,
        };
        let payload_json = canonical(&stage_input).expect("canonical generation payload");
        let recovery_envelope = SelectionRecoveryEnvelopeRowContentPreimage {
            domain: DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
            stage_run_id: stage_input.stage_run_id.clone(),
            subject_kind: SubjectKind::GenerationRun,
            logical_subject_key: stage_input.logical_subject_key.clone(),
            payload_schema: GENERATION_PAYLOAD_SCHEMA.into(),
            payload_json_hash: crate::selection::schema_v2::sha256_bytes(payload_json.as_bytes()),
            payload_json,
            in_memory_payload_hash: hash(&run_payload).expect("hash generation run payload"),
            config_activation_run_id: stage_input.config_activation_run_id.clone(),
            config_hash: stage_input.config_hash.clone(),
            enveloped_at: "2026-07-28T01:00:00.000000000Z".into(),
        };
        GenerationStageRequest {
            stage_input,
            run_payload,
            recovery_envelope,
        }
    }

    fn prepared_content(persisted: &PersistedRecoveryEnvelope) -> PreparedAuditContentPreimage {
        PreparedAuditContentPreimage {
            domain: DOMAIN_PREPARED_AUDIT.into(),
            subject_kind: persisted.subject_kind(),
            subject_id: persisted.stage_run_id().into(),
            logical_subject_key: persisted.logical_subject_key().into(),
            recovery_envelope_content_hash: persisted.content_hash().into(),
            in_memory_payload_hash: persisted.in_memory_payload_hash().into(),
        }
    }

    fn prepared_proof(
        repository: &SelectionV2Repository,
        conn: &mut SqliteConnection,
        session: &mut LockedSelectionAuditSession<'_>,
        persisted: &PersistedRecoveryEnvelope,
    ) -> PreparedAuditProof {
        let content = prepared_content(persisted);
        let content_hash = hash(&content).expect("hash Prepared audit content");
        let recorded_at = DateTime::parse_from_rfc3339("2026-07-28T01:00:01.000000000Z")
            .expect("fixed Prepared timestamp");
        session
            .append(SelectionAuditRecord::new(
                prepared_phase(content.subject_kind),
                &content.subject_id,
                &content_hash,
                recorded_at,
            ))
            .expect("append Prepared audit record");
        repository
            .load_prepared_proof(conn, session, persisted, content)
            .expect("load Prepared proof")
    }

    fn commit_generation_run(
        repository: &SelectionV2Repository,
        conn: &mut SqliteConnection,
        session: &mut LockedSelectionAuditSession<'_>,
        prepared: &PreparedAuditProof,
        staged: &StagedRunReceipt,
        committed_at: DateTime<chrono::FixedOffset>,
    ) -> CommitReceipt {
        let content = CommittedAuditContentPreimage {
            domain: DOMAIN_COMMITTED_AUDIT.into(),
            subject_kind: staged.subject_kind,
            subject_id: staged.subject_id.clone(),
            logical_subject_key: staged.logical_subject_key.clone(),
            recovery_envelope_content_hash: staged.recovery_envelope_content_hash.clone(),
            prepared_record_hash: prepared.record_hash().into(),
            run_manifest_content_hash: staged.run_manifest_content_hash.clone(),
            staged_db_content_hash: staged.staged_db_content_hash.clone(),
        };
        let content_hash = hash(&content).expect("hash Committed audit content");
        session
            .append(SelectionAuditRecord::new(
                committed_phase(content.subject_kind),
                &content.subject_id,
                &content_hash,
                committed_at,
            ))
            .expect("append Committed audit record");
        let proof =
            CommittedAuditProof::load(session, content).expect("load exact Committed proof");
        repository
            .insert_commit_receipt(conn, session, prepared, &proof)
            .expect("insert exact commit receipt")
    }

    fn stage_config_manifest_fixture(
        conn: &mut SqliteConnection,
        session: &mut LockedSelectionAuditSession<'_>,
        subject_id: &str,
        config_hash: &str,
        with_receipt: bool,
    ) -> (String, Option<String>) {
        let logical_subject_key = test_hash('4');
        let payload_json = canonical(&serde_json::json!({
            "domain": DOMAIN_CONFIG_ACTIVATION_STAGE
        }))
        .expect("canonical config payload");
        let envelope = SelectionRecoveryEnvelopeRowContentPreimage {
            domain: DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
            stage_run_id: subject_id.into(),
            subject_kind: SubjectKind::ConfigActivation,
            logical_subject_key: logical_subject_key.clone(),
            payload_schema: CONFIG_ACTIVATION_PAYLOAD_SCHEMA.into(),
            payload_json_hash: crate::selection::schema_v2::sha256_bytes(payload_json.as_bytes()),
            payload_json,
            in_memory_payload_hash: test_hash('1'),
            config_activation_run_id: subject_id.into(),
            config_hash: config_hash.into(),
            enveloped_at: "2026-07-28T01:00:00.000000000Z".into(),
        };
        let envelope_hash = hash(&envelope).expect("hash config envelope");
        insert_envelope(conn, &envelope, &envelope_hash).expect("insert config envelope");

        let prepared_content = PreparedAuditContentPreimage {
            domain: DOMAIN_PREPARED_AUDIT.into(),
            subject_kind: SubjectKind::ConfigActivation,
            subject_id: subject_id.into(),
            logical_subject_key: logical_subject_key.clone(),
            recovery_envelope_content_hash: envelope_hash.clone(),
            in_memory_payload_hash: envelope.in_memory_payload_hash.clone(),
        };
        let prepared_hash = hash(&prepared_content).expect("hash config Prepared content");
        let prepared_record = session
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::V2ConfigActivationPrepared,
                subject_id,
                &prepared_hash,
                DateTime::parse_from_rfc3339("2026-07-28T01:00:01.000000000Z")
                    .expect("fixed config Prepared timestamp"),
            ))
            .expect("append config Prepared");

        let manifest = RunManifestContentPreimage {
            domain: DOMAIN_RUN_MANIFEST.into(),
            subject_kind: SubjectKind::ConfigActivation,
            subject_id: subject_id.into(),
            in_memory_payload_hash: envelope.in_memory_payload_hash.clone(),
            prepared_record_hash: prepared_record.record_hash.clone(),
            expected_staged_row_count: 1,
            staged_db_content_hash: test_hash('2'),
            recovery_envelope_content_hash: envelope_hash.clone(),
            logical_subject_key: logical_subject_key.clone(),
            run_status: RunStatus::Activated,
            source_fact_key: None,
            config_activation_run_id: Some(subject_id.into()),
            config_hash: Some(config_hash.into()),
            config_snapshot_json_hash: Some(test_hash('5')),
            config_activation_content_hash: Some(test_hash('6')),
            config_activation_file_content_hash: Some(test_hash('7')),
            config_effective_from_rfc3339_nanos_utc: Some("2026-07-28T02:00:00.000000000Z".into()),
            artifact_valid_from: Some("2026-07-28".into()),
            artifact_expires_at: Some("2027-07-28".into()),
            executable_revision: Some(test_hash('8')),
            legacy_cutover_snapshot_hash: Some(test_hash('9')),
            generation_market_date: None,
            aggregator_observed_at_rfc3339_nanos_utc: None,
            ingress_source_batch_content_hash: None,
            outcome_phase: None,
            stored_due_date: None,
            outcome_claim_id: None,
            planned_outcome_run_id: None,
            outcome_claim_receipt_content_hash: None,
            outcome_claim_due_binding_hash: None,
            outcome_claim_provider_request_hash: None,
            staged_at_rfc3339_nanos_utc: "2026-07-28T01:00:02.000000000Z".into(),
        };
        manifest
            .validate_kind_matrix()
            .expect("valid config manifest matrix");
        let manifest_hash = hash(&manifest).expect("hash config manifest");
        insert_manifest(conn, &manifest, &manifest_hash).expect("insert config manifest");
        if !with_receipt {
            return (manifest_hash, None);
        }

        let committed_content = CommittedAuditContentPreimage {
            domain: DOMAIN_COMMITTED_AUDIT.into(),
            subject_kind: SubjectKind::ConfigActivation,
            subject_id: subject_id.into(),
            logical_subject_key: logical_subject_key.clone(),
            recovery_envelope_content_hash: envelope_hash.clone(),
            prepared_record_hash: prepared_record.record_hash.clone(),
            run_manifest_content_hash: manifest_hash.clone(),
            staged_db_content_hash: manifest.staged_db_content_hash.clone(),
        };
        let committed_hash = hash(&committed_content).expect("hash config Committed content");
        let committed_at = DateTime::parse_from_rfc3339("2026-07-28T01:00:03.000000000Z")
            .expect("fixed config Committed timestamp");
        let committed_record = session
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::V2ConfigActivationCommitted,
                subject_id,
                &committed_hash,
                committed_at,
            ))
            .expect("append config Committed");
        let receipt = CommitReceiptContentPreimage {
            domain: DOMAIN_COMMIT_RECEIPT.into(),
            subject_kind: SubjectKind::ConfigActivation,
            subject_id: subject_id.into(),
            logical_subject_key,
            in_memory_payload_hash: envelope.in_memory_payload_hash,
            recovery_envelope_content_hash: envelope_hash,
            prepared_audit_hash: prepared_record.record_hash,
            run_manifest_content_hash: manifest_hash.clone(),
            staged_db_content_hash: manifest.staged_db_content_hash,
            committed_audit_hash: committed_record.record_hash,
            committed_at_rfc3339_nanos_utc: utc_nanos(committed_at.with_timezone(&Utc)),
        };
        let receipt_hash = hash(&receipt).expect("hash config receipt");
        insert_commit_receipt_row(conn, &receipt, &receipt_hash).expect("insert config receipt");
        (manifest_hash, Some(receipt_hash))
    }

    fn insert_generation_receipt_fixture(conn: &mut SqliteConnection) {
        diesel::sql_query(
            "INSERT INTO selection_v2_commit_receipts (
                subject_kind,subject_id,logical_subject_key,in_memory_payload_hash,
                recovery_envelope_content_hash,prepared_audit_hash,
                run_manifest_content_hash,staged_db_content_hash,committed_audit_hash,
                committed_at,content_hash
             )
             SELECT subject_kind,subject_id,logical_subject_key,in_memory_payload_hash,
                    recovery_envelope_content_hash,prepared_record_hash,
                    manifest_content_hash,staged_db_content_hash,?, ?, ?
             FROM selection_v2_run_stages
             WHERE subject_kind='generation_run' AND subject_id=?",
        )
        .bind::<Text, _>(test_hash('7'))
        .bind::<Text, _>("2026-07-28T01:00:03.000000000Z")
        .bind::<Text, _>(test_hash('8'))
        .bind::<Text, _>(TEST_GENERATION_RUN_ID)
        .execute(conn)
        .expect("insert TEST_CODE generation receipt fixture");
    }

    struct TestOutcomeClaimBinding {
        claim_id: String,
        receipt_content_hash: String,
        due_binding_hash: String,
        provider_request_hash: String,
    }

    fn stage_outcome_claim_fixture(
        conn: &mut SqliteConnection,
        repository: &SelectionV2Repository,
        session: &mut LockedSelectionAuditSession<'_>,
    ) -> TestOutcomeClaimBinding {
        let stage = outcome_claim_stage();
        let request = OutcomeClaimStageRequest::from_stage_input(
            stage.clone(),
            DateTime::parse_from_rfc3339("2026-07-28T01:02:10.000000000Z")
                .expect("fixed claim envelope timestamp")
                .with_timezone(&Utc),
        )
        .expect("build typed outcome claim request");
        let envelope = repository
            .persist_outcome_claim_envelope(conn, session, &request)
            .expect("persist typed outcome claim envelope");
        let prepared = prepared_proof(repository, conn, session, &envelope);
        let staged = repository
            .stage_outcome_claim(
                conn,
                &request,
                &envelope,
                session,
                &prepared,
                DateTime::parse_from_rfc3339("2026-07-28T01:02:11.000000000Z")
                    .expect("fixed claim stage timestamp")
                    .with_timezone(&Utc),
            )
            .expect("stage typed outcome claim");
        let receipt = commit_generation_run(
            repository,
            conn,
            session,
            &prepared,
            &staged,
            DateTime::parse_from_rfc3339("2026-07-28T01:02:12.000000000Z")
                .expect("fixed claim commit timestamp"),
        );
        TestOutcomeClaimBinding {
            claim_id: receipt.subject_id().into(),
            receipt_content_hash: receipt.content_hash().into(),
            due_binding_hash: stage.due_binding_hash,
            provider_request_hash: stage.provider_request_hash,
        }
    }

    fn outcome_transport_attempt_columns(
        request_columns: &RequestEvidenceColumns,
        outcome: &SelectionSampleOutcomeRowContentPreimage,
    ) -> (String, String, String) {
        let request: RequestEvidencePreimage =
            serde_json::from_str(&request_columns.request_evidence_json)
                .expect("decode typed outcome request evidence");
        let parameters: OutcomeMarketRequestParametersPreimage =
            serde_json::from_str(&request.parameters_json)
                .expect("decode typed outcome request parameters");
        let expected_bar_count = u16::try_from(parameters.applicable_trading_dates.len())
            .expect("test outcome date prefix fits u16");
        let records = parameters
            .applicable_trading_dates
            .iter()
            .map(|market_date| OutcomeTransportBarFingerprint {
                market_date: market_date.clone(),
                open: outcome.open.clone(),
                high: outcome.high.clone(),
                low: outcome.low.clone(),
                close: outcome.close.clone(),
                core_volume_lots: "10".into(),
                amount: Some(outcome.amount.clone()),
                provider: "Tdx".into(),
                batch_id: outcome.batch_id.clone(),
            })
            .collect();
        let batch_content = OutcomeTransportBatchContentPreimage {
            provider: outcome.provider.clone(),
            source: outcome.source.clone(),
            records,
        };
        let batch_content_hash =
            hash(&batch_content).expect("hash typed outcome transport batch content");
        let evidence = OutcomeTransportEvidencePreimage {
            source: outcome.source.clone(),
            source_at: outcome.source_at.clone(),
            observed_at: outcome.observed_at.clone(),
            batch_id: outcome.batch_id.clone(),
            record_count: u32::from(expected_bar_count),
            batch_content,
            batch_content_hash: batch_content_hash.clone(),
        };
        let transport_request = OutcomeTransportRequestPreimage {
            provider: outcome.provider.clone(),
            source: outcome.source.clone(),
            canonical_stock_code: parameters.canonical_stock_code.clone(),
            canonical_market: parameters.canonical_market.clone(),
            interval: parameters.interval.as_str().into(),
            adjustment: parameters.adjustment.as_str().into(),
            latest_n: expected_bar_count,
        };
        let result = OutcomeTransportResultPreimage {
            terminal_state: "available".into(),
            requested_latest_n: expected_bar_count,
            actual_count: Some(expected_bar_count),
            provider_evidence_hash: Some(
                hash(&evidence).expect("hash typed outcome transport evidence"),
            ),
            provider_evidence: Some(evidence),
            provider_error: None,
            provider_error_hash: None,
        };
        let attempt = OutcomeTransportAttemptPreimage {
            request_ordinal: 0,
            request_hash: hash(&transport_request).expect("hash outcome transport request"),
            request: transport_request,
            result_hash: hash(&result).expect("hash outcome transport result"),
            result,
        };
        let attempts = OutcomeTransportAttemptsPreimage {
            domain: DOMAIN_OUTCOME_TRANSPORT_ATTEMPTS.into(),
            design_sha256: OUTCOME_PARENT_DESIGN_SHA256.into(),
            amendment_design_sha256: AMENDMENT_DESIGN_SHA256.into(),
            row_request_hash: request_columns.request_hash.clone(),
            request_evidence_hash: request_columns.request_evidence_hash.clone(),
            provider_capability_hash: request.provider_capability_hash,
            provider_revision: UPSTREAM_REVISION.into(),
            request_parameters_hash: request.parameters_json_hash,
            provider_request_hash: test_hash('b'),
            verified_due_binding_hash: test_hash('c'),
            adaptive_policy_version: OUTCOME_ADAPTIVE_POLICY_VERSION.into(),
            expected_bar_count,
            maximum_latest_n: 10,
            selected_transport_result_hash: Some(attempt.result_hash.clone()),
            attempts_in_request_order: vec![attempt],
        };
        (
            canonical(&attempts).expect("canonical outcome transport attempts"),
            hash(&attempts).expect("hash outcome transport attempts"),
            batch_content_hash,
        )
    }

    fn outcome_stage(result: OutcomeAttemptResult) -> OutcomeStageInputPreimage {
        let sample_key_preimage = test_sample_key_preimage();
        let sample_key = hash(&sample_key_preimage).expect("hash outcome sample key");
        let mut outcome = SelectionSampleOutcomeRowContentPreimage {
            domain: DOMAIN_SAMPLE_OUTCOME_ROW.into(),
            sample_key: sample_key.clone(),
            phase: OutcomePhase::T0Close,
            outcome_run_id: TEST_OUTCOME_RUN_ID.into(),
            due_trading_date: "2026-07-28".into(),
            open: "10".into(),
            high: "11".into(),
            low: "9".into(),
            close: "10.5".into(),
            volume: "1000".into(),
            amount: "10500".into(),
            return_from_t0_close: "0".into(),
            cumulative_mfe: "0".into(),
            cumulative_mae: "0".into(),
            volume_ratio: "1".into(),
            provider: "magic-tdx".into(),
            source: "tdx-smart".into(),
            source_at: Some("2026-07-28T01:00:00.000000000Z".into()),
            observed_at: "2026-07-28T01:01:00.000000000Z".into(),
            batch_id: "TEST_CODE_OUTCOME_BATCH".into(),
            batch_content_hash: test_hash('9'),
            created_at: "2026-07-28T01:02:00.000000000Z".into(),
        };
        let request =
            outcome_request_columns(&sample_key, outcome.phase, &outcome.due_trading_date);
        let (transport_attempts_json, transport_attempts_hash, batch_content_hash) =
            outcome_transport_attempt_columns(&request, &outcome);
        outcome.batch_content_hash = batch_content_hash;
        let request_evidence: RequestEvidencePreimage =
            serde_json::from_str(&request.request_evidence_json)
                .expect("decode outcome request evidence");
        let request_parameters: OutcomeMarketRequestParametersPreimage =
            serde_json::from_str(&request_evidence.parameters_json)
                .expect("decode outcome request parameters");
        let mut attempt = SelectionOutcomeAttemptRowContentPreimage {
            domain: DOMAIN_OUTCOME_ATTEMPT_ROW.into(),
            outcome_attempt_id: test_hash('a'),
            sample_key: outcome.sample_key.clone(),
            phase: outcome.phase,
            stored_due_date: outcome.due_trading_date.clone(),
            outcome_run_id: outcome.outcome_run_id.clone(),
            request_hash: Some(request.request_hash.clone()),
            request_evidence_json: Some(request.request_evidence_json.clone()),
            request_evidence_hash: Some(request.request_evidence_hash.clone()),
            transport_attempts_json: None,
            transport_attempts_hash: None,
            result_code: result,
            reason_code: None,
            retryable: None,
            provider: None,
            source: None,
            source_at: None,
            observed_at: None,
            batch_id: None,
            batch_content_hash: None,
            available_evidence_json: None,
            available_evidence_hash: None,
            error_detail_json: None,
            error_detail_hash: None,
            error_fingerprint: None,
            settled_outcome_content_hash: None,
            attempted_at: "2026-07-28T01:02:00.000000000Z".into(),
        };
        let (outcome_attempt_rows, outcome_rows, status) = match result {
            OutcomeAttemptResult::Settled => {
                attempt.transport_attempts_json = Some(transport_attempts_json);
                attempt.transport_attempts_hash = Some(transport_attempts_hash);
                let provider_evidence = ProviderAvailableEvidencePreimage {
                    domain: DOMAIN_PROVIDER_AVAILABLE_EVIDENCE.into(),
                    evidence_kind: ProviderEvidenceKind::OutcomeDailyBars,
                    provider: outcome.provider.clone(),
                    source: Some(outcome.source.clone()),
                    source_at: outcome.source_at.clone(),
                    observed_at: Some(outcome.observed_at.clone()),
                    batch_id: Some(outcome.batch_id.clone()),
                    batch_content_hash: Some(outcome.batch_content_hash.clone()),
                };
                let evidence = OutcomeProviderAvailableEvidencePreimage {
                    domain: DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE.into(),
                    request_hash: request.request_hash.clone(),
                    calendar_hash: request_parameters.calendar_hash.clone(),
                    trading_date_vector_hash: request_parameters.trading_date_vector_hash.clone(),
                    expected_trading_dates: request_parameters.applicable_trading_dates.clone(),
                    returned_trading_dates: request_parameters.applicable_trading_dates.clone(),
                    provider_evidence,
                };
                attempt.provider = Some(evidence.provider_evidence.provider.clone());
                attempt.source = evidence.provider_evidence.source.clone();
                attempt.source_at = evidence.provider_evidence.source_at.clone();
                attempt.observed_at = evidence.provider_evidence.observed_at.clone();
                attempt.batch_id = evidence.provider_evidence.batch_id.clone();
                attempt.batch_content_hash = evidence.provider_evidence.batch_content_hash.clone();
                attempt.available_evidence_json =
                    Some(canonical(&evidence).expect("outcome evidence JSON"));
                attempt.available_evidence_hash =
                    Some(hash(&evidence).expect("outcome evidence hash"));
                attempt.settled_outcome_content_hash =
                    Some(hash(&outcome).expect("settled outcome hash"));
                (vec![attempt], vec![outcome], RunStatus::Settled)
            }
            OutcomeAttemptResult::ExpectedWait => (Vec::new(), Vec::new(), RunStatus::ExpectedWait),
            OutcomeAttemptResult::Error => unreachable!("outcome test fixture"),
        };
        let config_hash = test_hash('5');
        let logical_subject_key = run_logical_subject_key(&RunLogicalSubjectPreimage {
            domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
            subject_kind: SubjectKind::OutcomeRun,
            source_fact_key: None,
            config_hash: Some(config_hash.clone()),
            sample_key: Some(sample_key.clone()),
            outcome_phase: Some(OutcomePhase::T0Close),
            stored_due_date: Some("2026-07-28".into()),
            ingress_source_batch_hash: None,
        })
        .expect("hash outcome logical subject");
        OutcomeStageInputPreimage {
            domain: DOMAIN_OUTCOME_STAGE.into(),
            stage_run_id: TEST_OUTCOME_RUN_ID.into(),
            logical_subject_key,
            config_activation_run_id: TEST_ACTIVATION_RUN_ID.into(),
            config_hash,
            outcome_claim_id: TEST_OUTCOME_CLAIM_ID.into(),
            outcome_claim_receipt_content_hash: test_hash('6'),
            outcome_claim_due_binding_hash: test_hash('7'),
            outcome_claim_provider_request_hash: test_hash('8'),
            sample_key_preimage,
            sample_key,
            outcome_phase: OutcomePhase::T0Close,
            stored_due_date: "2026-07-28".into(),
            outcome_attempt_rows,
            outcome_rows,
            planned_run_status: status,
        }
    }

    fn outcome_request(stage_input: OutcomeStageInputPreimage) -> OutcomeStageRequest {
        let rows = outcome_run_row_hashes(&stage_input).expect("canonical outcome row hashes");
        let run_payload = RunPayloadPreimage {
            domain: DOMAIN_OUTCOME_PAYLOAD.into(),
            subject_kind: SubjectKind::OutcomeRun,
            subject_id: stage_input.stage_run_id.clone(),
            logical_subject_key: stage_input.logical_subject_key.clone(),
            source_fact_key: None,
            config_activation_run_id: stage_input.config_activation_run_id.clone(),
            config_hash: stage_input.config_hash.clone(),
            config_snapshot_json_hash: None,
            config_activation_content_hash: None,
            config_activation_file_content_hash: None,
            config_effective_from_rfc3339_nanos_utc: None,
            artifact_valid_from: None,
            artifact_expires_at: None,
            executable_revision: None,
            legacy_cutover_snapshot_hash: None,
            generation_market_date: None,
            aggregator_observed_at_rfc3339_nanos_utc: None,
            ingress_source_batch_content_hash: None,
            outcome_phase: Some(stage_input.outcome_phase),
            stored_due_date: Some(stage_input.stored_due_date.clone()),
            outcome_claim_id: Some(stage_input.outcome_claim_id.clone()),
            planned_outcome_run_id: None,
            outcome_claim_receipt_content_hash: Some(
                stage_input.outcome_claim_receipt_content_hash.clone(),
            ),
            outcome_claim_due_binding_hash: Some(
                stage_input.outcome_claim_due_binding_hash.clone(),
            ),
            outcome_claim_provider_request_hash: Some(
                stage_input.outcome_claim_provider_request_hash.clone(),
            ),
            rows,
        };
        let payload_json = canonical(&stage_input).expect("canonical outcome payload");
        let recovery_envelope = SelectionRecoveryEnvelopeRowContentPreimage {
            domain: DOMAIN_RECOVERY_ENVELOPE_ROW.into(),
            stage_run_id: stage_input.stage_run_id.clone(),
            subject_kind: SubjectKind::OutcomeRun,
            logical_subject_key: stage_input.logical_subject_key.clone(),
            payload_schema: OUTCOME_PAYLOAD_SCHEMA.into(),
            payload_json_hash: crate::selection::schema_v2::sha256_bytes(payload_json.as_bytes()),
            payload_json,
            in_memory_payload_hash: hash(&run_payload).expect("hash outcome run payload"),
            config_activation_run_id: stage_input.config_activation_run_id.clone(),
            config_hash: stage_input.config_hash.clone(),
            enveloped_at: "2026-07-28T01:03:00.000000000Z".into(),
        };
        OutcomeStageRequest {
            stage_input,
            run_payload,
            recovery_envelope,
        }
    }

    fn bind_outcome_claim(
        mut stage: OutcomeStageInputPreimage,
        claim: &TestOutcomeClaimBinding,
    ) -> OutcomeStageInputPreimage {
        stage.outcome_claim_id = claim.claim_id.clone();
        stage.outcome_claim_receipt_content_hash = claim.receipt_content_hash.clone();
        stage.outcome_claim_due_binding_hash = claim.due_binding_hash.clone();
        stage.outcome_claim_provider_request_hash = claim.provider_request_hash.clone();
        stage
    }

    fn stage_generation_receipted(
        conn: &mut SqliteConnection,
        repository: &SelectionV2Repository,
        session: &mut LockedSelectionAuditSession<'_>,
    ) {
        seed_receipted_source_fact(conn);
        let generation = generation_request(hard_rejected_generation_stage());
        let envelope = repository
            .persist_generation_envelope(conn, session, &generation)
            .expect("persist upstream generation envelope");
        let proof = prepared_proof(repository, conn, session, &envelope);
        let staged_at = DateTime::parse_from_rfc3339("2026-07-28T01:00:02.000000000Z")
            .expect("fixed generation timestamp")
            .with_timezone(&Utc);
        repository
            .stage_generation(conn, &generation, &envelope, session, &proof, staged_at)
            .expect("stage upstream generation");
        insert_generation_receipt_fixture(conn);
    }

    fn stage_generation_upstream(
        conn: &mut SqliteConnection,
        repository: &SelectionV2Repository,
        session: &mut LockedSelectionAuditSession<'_>,
    ) -> TestOutcomeClaimBinding {
        stage_generation_receipted(conn, repository, session);
        stage_outcome_claim_fixture(conn, repository, session)
    }

    #[test]
    fn envelope_commit_is_durable_before_any_prepared_or_manifest_row() {
        #[derive(QueryableByName)]
        struct Count {
            #[diesel(sql_type = BigInt)]
            value: i64,
        }

        let (mut conn, repository) = repository();
        let envelope = validated_envelope(
            "019849b1-e800-7000-8000-000000000001",
            r#"{"TEST_CODE":"envelope-only"}"#,
        );

        let receipt = repository
            .persist_validated_envelope(&mut conn, &envelope)
            .expect("persist standalone recovery envelope");
        let persisted = diesel::sql_query(
            "SELECT COUNT(*) AS value
             FROM selection_v2_recovery_envelopes
             WHERE stage_run_id=?
               AND logical_subject_key=?
               AND NOT EXISTS (
                   SELECT 1 FROM selection_v2_run_stages s
                   WHERE s.subject_id=selection_v2_recovery_envelopes.stage_run_id
               )",
        )
        .bind::<Text, _>(&envelope.envelope.stage_run_id)
        .bind::<Text, _>(&envelope.envelope.logical_subject_key)
        .get_result::<Count>(&mut conn)
        .expect("read durable envelope");

        assert_eq!(receipt.disposition, StageDisposition::Inserted);
        assert_eq!(persisted.value, 1);
    }

    #[test]
    fn exact_envelope_replay_is_idempotent() {
        let (mut conn, repository) = repository();
        let envelope = validated_envelope(
            "019849b1-e800-7000-8000-000000000002",
            r#"{"TEST_CODE":"exact-replay"}"#,
        );

        let first = repository
            .persist_validated_envelope(&mut conn, &envelope)
            .expect("insert envelope");
        let replay = repository
            .persist_validated_envelope(&mut conn, &envelope)
            .expect("accept exact envelope replay");

        assert_eq!(first.disposition, StageDisposition::Inserted);
        assert_eq!(replay.disposition, StageDisposition::ExactReplay);
        assert_eq!(first.content_hash, replay.content_hash);
    }

    #[test]
    fn same_run_with_different_envelope_is_a_fatal_conflict() {
        let (mut conn, repository) = repository();
        let stage_run_id = "019849b1-e800-7000-8000-000000000003";
        let first = validated_envelope(stage_run_id, r#"{"TEST_CODE":"first"}"#);
        let conflicting = validated_envelope(stage_run_id, r#"{"TEST_CODE":"different"}"#);

        repository
            .persist_validated_envelope(&mut conn, &first)
            .expect("insert first envelope");
        let error = repository
            .persist_validated_envelope(&mut conn, &conflicting)
            .expect_err("same run with different envelope must fail");

        assert!(matches!(
            error,
            SelectionV2RepositoryError::ReplayConflict {
                subject_kind,
                subject_id
            } if subject_kind == "config_activation" && subject_id == stage_run_id
        ));
    }

    #[test]
    fn stage_guard_rejects_a_validated_envelope_that_is_not_persisted() {
        let (mut conn, repository) = repository();
        let envelope = validated_envelope(
            "019849b1-e800-7000-8000-000000000004",
            r#"{"TEST_CODE":"not-persisted"}"#,
        );

        let error = repository
            .require_exact_persisted_envelope(&mut conn, &envelope)
            .expect_err("domain staging must not proceed without its durable envelope");

        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "stage_envelope_missing",
                ..
            }
        ));
    }

    #[test]
    fn config_prepared_proof_uses_v2_phase_inside_existing_locked_session() {
        let root = TestAuditRoot::new("config-prepared-proof");
        let writer = test_audit_writer(&root);
        let content = PreparedAuditContentPreimage {
            domain: DOMAIN_PREPARED_AUDIT.into(),
            subject_kind: SubjectKind::ConfigActivation,
            subject_id: "019849b1-e800-7000-8000-000000000005".into(),
            logical_subject_key: "TEST_CODE_LOGICAL_CONFIG".into(),
            recovery_envelope_content_hash: "3".repeat(64),
            in_memory_payload_hash: "4".repeat(64),
        };
        let content_hash = hash(&content).expect("hash prepared content");
        let recorded_at = DateTime::parse_from_rfc3339("2026-07-28T01:00:00.000000000Z")
            .expect("fixed test timestamp");

        writer
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::Prepared,
                &content.subject_id,
                &content_hash,
                recorded_at,
            ))
            .expect("append historical generic phase");
        let expected = writer
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::V2ConfigActivationPrepared,
                &content.subject_id,
                &content_hash,
                recorded_at,
            ))
            .expect("append run-kind-specific phase");

        let mut session = writer.locked_session().expect("hold validated audit lock");
        let exact = load_persisted_audit_record(
            &mut session,
            SelectionAuditPhase::V2ConfigActivationPrepared,
            &content.subject_id,
            &content_hash,
        )
        .expect("load exact V2 phase record");
        assert_eq!(exact.record_hash, expected.record_hash);
        session.finish().expect("finish locked audit session");
    }

    #[test]
    fn generation_and_outcome_stage_requests_are_public_repository_capabilities() {
        let _ = std::any::type_name::<GenerationStageRequest>();
        let _ = std::any::type_name::<OutcomeStageRequest>();
    }

    #[test]
    fn non_outcome_recovery_reconstructs_exact_generation_request_and_rejects_outcome_stages() {
        let request = generation_request(verified_no_relation_stage());
        let recovered =
            NonOutcomeRecoveryStageRequest::try_from_envelope(request.recovery_envelope.clone())
                .expect("exact generation recovery envelope")
                .expect("generation is owned by the non-outcome recovery owner");
        assert_eq!(
            recovered,
            NonOutcomeRecoveryStageRequest::Generation(request)
        );

        let outcome = outcome_request(outcome_stage(OutcomeAttemptResult::ExpectedWait));
        assert!(
            NonOutcomeRecoveryStageRequest::try_from_envelope(outcome.recovery_envelope.clone())
                .expect("valid outcome envelope must be classified")
                .is_none(),
            "outcome recovery remains exclusively owned by OutcomeSettlementOwner"
        );
    }

    #[test]
    fn validated_stage_request_constructor_rejects_forged_run_payload() {
        let request = generation_request(verified_no_relation_stage());
        let mut forged_payload = request.run_payload.clone();
        forged_payload.config_hash = test_hash('f');

        let error = GenerationStageRequest::validated(
            request.stage_input,
            forged_payload,
            request.recovery_envelope,
        )
        .expect_err("validated request capability must reject duplicated lineage drift");

        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "generation_run_payload_mismatch",
                ..
            }
        ));
    }

    #[test]
    fn startup_reconciliation_rejects_orphan_prepared_audit_record() {
        let (mut conn, _) = repository();
        let record = SelectionAuditRecord::new(
            SelectionAuditPhase::V2GenerationPrepared,
            TEST_GENERATION_RUN_ID,
            test_hash('1'),
            DateTime::parse_from_rfc3339("2026-07-28T01:00:00.000000000Z")
                .expect("fixed orphan Prepared timestamp"),
        );

        let mut reader = DieselExactSelectionSnapshotReader { conn: &mut conn };
        let error = reconcile_audit_record_and_database(
            &mut reader,
            &record,
            ReconciliationPurpose::AuthoritativeRead,
        )
        .expect_err("Prepared audit without a recovery envelope must fail startup");

        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "startup_prepared_audit_envelope_missing",
                ..
            }
        ));
    }

    #[test]
    fn rusqlite_exact_reader_matches_diesel_inside_the_callers_existing_snapshot() {
        let root = TestAuditRoot::new("exact-reader-parity");
        let (database_path, mut diesel_connection, _) = file_repository(&root);
        let writer = test_audit_writer(&root);

        let mut diesel_audit = writer.locked_session().expect("lock empty audit chain");
        let diesel_snapshot =
            run_immediate_transaction(&mut diesel_connection, |transaction_connection| {
                verify_database_and_audit_in_current_snapshot(
                    transaction_connection,
                    &mut diesel_audit,
                )
            })
            .expect("Diesel exact reader must validate the empty final snapshot");
        diesel_audit.finish().expect("finish Diesel audit snapshot");
        drop(diesel_connection);

        let mut raw_connection =
            rusqlite::Connection::open(&database_path).expect("open same TEST_CODE database");
        let transaction = raw_connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("pin caller-owned rusqlite snapshot");
        let mut rusqlite_audit = writer.locked_session().expect("lock same audit chain");
        let rusqlite_snapshot =
            verify_database_and_audit_in_rusqlite_snapshot(&transaction, &mut rusqlite_audit)
                .expect("rusqlite exact reader must validate the same snapshot");

        assert_eq!(rusqlite_snapshot, diesel_snapshot);
        transaction
            .commit()
            .expect("finish caller-owned rusqlite snapshot");
        rusqlite_audit
            .finish()
            .expect("finish rusqlite audit snapshot");
    }

    #[test]
    fn rusqlite_exact_reader_rehashes_envelopes_instead_of_trusting_stored_hashes() {
        #[derive(QueryableByName)]
        struct TriggerSqlRow {
            #[diesel(sql_type = Text)]
            sql: String,
        }

        let root = TestAuditRoot::new("rusqlite-envelope-tamper");
        let (database_path, mut diesel_connection, _) = file_repository(&root);
        let writer = test_audit_writer(&root);
        let subject_id = "019849b1-e800-7000-8000-000000000199";
        let mut staging_audit = writer.locked_session().expect("lock staging audit chain");
        stage_config_manifest_fixture(
            &mut diesel_connection,
            &mut staging_audit,
            subject_id,
            &test_hash('5'),
            false,
        );
        staging_audit
            .finish()
            .expect("finish exact Prepared audit snapshot");
        let trigger = diesel::sql_query(
            "SELECT sql AS sql FROM sqlite_schema
             WHERE type = 'trigger'
               AND name = 'selection_v2_recovery_envelopes_deny_update'",
        )
        .get_result::<TriggerSqlRow>(&mut diesel_connection)
        .expect("capture exact append-only trigger before offline tamper simulation");
        diesel_connection
            .batch_execute("DROP TRIGGER selection_v2_recovery_envelopes_deny_update")
            .expect("temporarily remove TEST_CODE append-only trigger for offline tamper");
        diesel::sql_query(
            "UPDATE selection_v2_recovery_envelopes
             SET content_hash = ?
             WHERE stage_run_id = ?",
        )
        .bind::<Text, _>(test_hash('f'))
        .bind::<Text, _>(subject_id)
        .execute(&mut diesel_connection)
        .expect("tamper TEST_CODE envelope hash");
        diesel::sql_query(trigger.sql)
            .execute(&mut diesel_connection)
            .expect("restore exact append-only trigger after offline tamper simulation");
        drop(diesel_connection);

        let mut raw_connection =
            rusqlite::Connection::open(&database_path).expect("open tampered TEST_CODE database");
        let transaction = raw_connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("pin tampered caller-owned snapshot");
        let mut audit_session = writer.locked_session().expect("lock exact audit snapshot");
        let error =
            verify_database_and_audit_in_rusqlite_snapshot(&transaction, &mut audit_session)
                .expect_err("copied or tampered envelope hash must not authorize the snapshot");

        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "envelope_readback_rehash_mismatch",
                ..
            }
        ));
        transaction
            .rollback()
            .expect("release tampered caller-owned snapshot");
        audit_session
            .finish()
            .expect("finish unchanged audit snapshot");
    }

    #[test]
    fn generation_verified_no_relation_persists_zero_domain_rows_and_manifest_last() {
        let (mut conn, repository) = repository();
        seed_receipted_source_fact(&mut conn);
        let request = generation_request(verified_no_relation_stage());
        let root = TestAuditRoot::new("generation-verified-no-relation");
        let writer = test_audit_writer(&root);
        let mut session = writer.locked_session().expect("lock audit chain");
        let persisted = repository
            .persist_generation_envelope(&mut conn, &mut session, &request)
            .expect("persist generation recovery envelope");
        let proof = prepared_proof(&repository, &mut conn, &mut session, &persisted);
        let staged_at = DateTime::parse_from_rfc3339("2026-07-28T01:00:02.000000000Z")
            .expect("fixed stage timestamp")
            .with_timezone(&Utc);

        let receipt = repository
            .stage_generation(
                &mut conn,
                &request,
                &persisted,
                &mut session,
                &proof,
                staged_at,
            )
            .expect("stage closed verified-no-relation generation");
        let readback = repository
            .verify_staged_readback(&mut conn, TEST_GENERATION_RUN_ID)
            .expect("typed generation readback");

        assert_eq!(receipt.disposition, StageDisposition::Inserted);
        assert_eq!(receipt.expected_staged_row_count, 1);
        assert_eq!(
            receipt.staged_db_content_hash,
            readback.staged_db_content_hash
        );
        session.finish().expect("finish locked audit session");
    }

    #[test]
    fn generation_sample_and_rejections_are_atomic_counted_and_ordered() {
        let (mut conn, repository) = repository();
        let request = generation_request(hard_rejected_generation_stage());
        let expected_rejection_hashes = request.stage_input.sample_rows[0]
            .rejection_row_hashes_in_ordinal_order
            .clone();
        let root = TestAuditRoot::new("generation-sample-rejections");
        let writer = test_audit_writer(&root);
        let mut session = writer.locked_session().expect("lock audit chain");
        let persisted = repository
            .persist_generation_envelope(&mut conn, &mut session, &request)
            .expect("persist generation recovery envelope");
        let proof = prepared_proof(&repository, &mut conn, &mut session, &persisted);
        let staged_at = DateTime::parse_from_rfc3339("2026-07-28T01:00:02.000000000Z")
            .expect("fixed stage timestamp")
            .with_timezone(&Utc);

        repository
            .stage_generation(
                &mut conn,
                &request,
                &persisted,
                &mut session,
                &proof,
                staged_at,
            )
            .expect_err("missing receipted source lineage must roll back every domain row");

        #[derive(QueryableByName)]
        struct Count {
            #[diesel(sql_type = BigInt)]
            count: i64,
        }
        for table in [
            "selection_relation_attempts",
            "selection_evaluation_attempts",
            "selection_samples",
            "selection_rejections",
            "selection_v2_run_stages",
        ] {
            let count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
                .get_result::<Count>(&mut conn)
                .expect("count rolled-back generation rows");
            assert_eq!(count.count, 0, "{table} must remain empty after failure");
        }

        seed_receipted_source_fact(&mut conn);
        let receipt = repository
            .stage_generation(
                &mut conn,
                &request,
                &persisted,
                &mut session,
                &proof,
                staged_at,
            )
            .expect("stage sample plus ordered rejections atomically");
        assert_eq!(receipt.expected_staged_row_count, 6);

        #[derive(QueryableByName)]
        struct RejectionHash {
            #[diesel(sql_type = Integer)]
            ordinal: i32,
            #[diesel(sql_type = Text)]
            content_hash: String,
        }
        let actual = diesel::sql_query(
            "SELECT ordinal, content_hash
             FROM selection_rejections
             WHERE generation_run_id=?
             ORDER BY ordinal ASC",
        )
        .bind::<Text, _>(TEST_GENERATION_RUN_ID)
        .load::<RejectionHash>(&mut conn)
        .expect("read ordered rejection hashes");
        assert_eq!(
            actual
                .iter()
                .map(|row| (row.ordinal, row.content_hash.clone()))
                .collect::<Vec<_>>(),
            vec![
                (0, expected_rejection_hashes[0].clone()),
                (1, expected_rejection_hashes[1].clone()),
            ]
        );
        repository
            .verify_staged_readback(&mut conn, TEST_GENERATION_RUN_ID)
            .expect("complete generation readback");
        session.finish().expect("finish locked audit session");
    }

    #[test]
    fn outcome_claim_payload_round_trips_the_byte_identical_provider_transport_request() {
        let stage = outcome_claim_stage();
        let request_json =
            canonical_json(&stage.provider_transport_request).expect("canonical provider request");
        let claim_json = canonical_json(&stage).expect("canonical claim payload");
        let reopened: OutcomeClaimStageInputPreimage =
            serde_json::from_str(&claim_json).expect("reopen typed claim payload");
        let reopened_request_json = canonical_json(&reopened.provider_transport_request)
            .expect("canonical reopened provider request");

        assert_eq!(reopened_request_json.as_bytes(), request_json.as_bytes());
        assert_eq!(
            sha256_json(&reopened.provider_transport_request)
                .expect("hash reopened provider transport request"),
            stage.provider_transport_request_hash
        );
        reopened
            .validate()
            .expect("reopened claim retains the exact provider transport binding");
    }

    #[test]
    fn outcome_claim_owner_commits_exact_lineage_replays_and_rejects_tamper() {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new("outcome-claim-lifecycle");
        let writer = test_audit_writer(&root);
        let mut upstream_session = writer.locked_session().expect("lock upstream audit");
        stage_generation_receipted(&mut conn, &repository, &mut upstream_session);
        upstream_session.finish().expect("finish upstream audit");

        let request = OutcomeClaimStageRequest::from_stage_input(
            outcome_claim_stage(),
            DateTime::parse_from_rfc3339("2026-07-28T01:02:10.000000000Z")
                .expect("fixed claim envelope timestamp")
                .with_timezone(&Utc),
        )
        .expect("build typed claim request");
        let inserted = crate::selection::persistence_v2::commit_outcome_claim_request_for_test(
            &mut conn,
            &writer,
            request.clone(),
        )
        .expect("commit typed claim");
        let replayed = crate::selection::persistence_v2::commit_outcome_claim_request_for_test(
            &mut conn, &writer, request,
        )
        .expect("replay typed claim");

        assert_eq!(inserted.disposition(), StageDisposition::Inserted);
        assert_eq!(replayed.disposition(), StageDisposition::ExactReplay);
        assert_eq!(inserted.subject_kind(), SubjectKind::OutcomeClaim);
        assert_eq!(inserted.subject_id(), TEST_OUTCOME_CLAIM_ID);
        assert_eq!(inserted.content_hash(), replayed.content_hash());

        let manifest_row = find_manifest(&mut conn, TEST_OUTCOME_CLAIM_ID)
            .expect("query claim manifest")
            .expect("claim manifest exists");
        let manifest = rebuild_manifest(&manifest_row).expect("rebuild claim manifest");
        assert_eq!(
            manifest.planned_outcome_run_id.as_deref(),
            Some(TEST_OUTCOME_RUN_ID)
        );
        assert_eq!(
            manifest.outcome_claim_id.as_deref(),
            Some(TEST_OUTCOME_CLAIM_ID)
        );
        assert_eq!(manifest.outcome_claim_receipt_content_hash, None);
        repository
            .verify_staged_readback(&mut conn, TEST_OUTCOME_CLAIM_ID)
            .expect("claim readback preserves exact IDs and hashes");

        let claim = TestOutcomeClaimBinding {
            claim_id: TEST_OUTCOME_CLAIM_ID.into(),
            receipt_content_hash: inserted.content_hash().into(),
            due_binding_hash: manifest
                .outcome_claim_due_binding_hash
                .clone()
                .expect("claim due binding hash"),
            provider_request_hash: manifest
                .outcome_claim_provider_request_hash
                .clone()
                .expect("claim provider request hash"),
        };
        let exact = bind_outcome_claim(outcome_stage(OutcomeAttemptResult::ExpectedWait), &claim);
        validate_receipted_outcome_claim(&mut conn, &exact)
            .expect("exact claim authorizes its planned outcome");
        let mut tampered = exact;
        tampered.outcome_claim_due_binding_hash = test_hash('f');
        let error = validate_receipted_outcome_claim(&mut conn, &tampered)
            .expect_err("tampered claim hash must not authorize outcome work");
        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "outcome_claim_manifest_binding_mismatch",
                ..
            }
        ));
    }

    #[test]
    fn outcome_settled_binds_attempt_to_outcome_and_expected_wait_stores_zero_outcomes() {
        for (result, expected_count, expected_outcomes, label) in [
            (OutcomeAttemptResult::Settled, 3, 1, "settled"),
            (OutcomeAttemptResult::ExpectedWait, 1, 0, "expected-wait"),
        ] {
            let (mut conn, repository) = repository();
            let root = TestAuditRoot::new(label);
            let writer = test_audit_writer(&root);
            let mut session = writer.locked_session().expect("lock audit chain");
            let claim = stage_generation_upstream(&mut conn, &repository, &mut session);
            let request = outcome_request(bind_outcome_claim(outcome_stage(result), &claim));
            let envelope = repository
                .persist_outcome_envelope(&mut conn, &mut session, &request)
                .expect("persist outcome envelope");
            let proof = prepared_proof(&repository, &mut conn, &mut session, &envelope);
            let staged_at = DateTime::parse_from_rfc3339("2026-07-28T01:03:02.000000000Z")
                .expect("fixed outcome timestamp")
                .with_timezone(&Utc);

            let receipt = repository
                .stage_outcome(
                    &mut conn,
                    &request,
                    &envelope,
                    &mut session,
                    &proof,
                    staged_at,
                )
                .expect("stage closed outcome matrix");
            assert_eq!(receipt.expected_staged_row_count, expected_count);

            #[derive(QueryableByName)]
            struct OutcomeBinding {
                #[diesel(sql_type = BigInt)]
                outcome_count: i64,
                #[diesel(sql_type = Nullable<Text>)]
                settled_hash: Option<String>,
            }
            let binding = diesel::sql_query(
                "SELECT
                    (SELECT COUNT(*) FROM selection_sample_outcomes
                     WHERE outcome_run_id=?) AS outcome_count,
                    (SELECT settled_outcome_content_hash
                     FROM selection_outcome_attempts
                     WHERE outcome_run_id=?) AS settled_hash",
            )
            .bind::<Text, _>(TEST_OUTCOME_RUN_ID)
            .bind::<Text, _>(TEST_OUTCOME_RUN_ID)
            .get_result::<OutcomeBinding>(&mut conn)
            .expect("read outcome binding");
            assert_eq!(binding.outcome_count, expected_outcomes);
            match result {
                OutcomeAttemptResult::Settled => assert_eq!(
                    binding.settled_hash,
                    Some(hash(&request.stage_input.outcome_rows[0]).expect("outcome hash"))
                ),
                OutcomeAttemptResult::ExpectedWait => assert_eq!(binding.settled_hash, None),
                OutcomeAttemptResult::Error => unreachable!("test matrix"),
            }
            repository
                .verify_staged_readback(&mut conn, TEST_OUTCOME_RUN_ID)
                .expect("complete typed outcome readback");
            session.finish().expect("finish locked audit session");
        }
    }

    #[test]
    fn outcome_envelope_rejects_config_and_due_date_drift_from_receipted_sample() {
        for drift in ["config", "due-date"] {
            let (mut conn, repository) = repository();
            let root = TestAuditRoot::new(drift);
            let writer = test_audit_writer(&root);
            let mut session = writer.locked_session().expect("lock audit chain");
            let claim = stage_generation_upstream(&mut conn, &repository, &mut session);
            let mut stage = outcome_stage(OutcomeAttemptResult::ExpectedWait);
            match drift {
                "config" => stage.config_hash = test_hash('f'),
                "due-date" => {
                    stage.stored_due_date = "2026-07-29".into();
                }
                _ => unreachable!("fixed drift matrix"),
            }
            stage.logical_subject_key = run_logical_subject_key(&RunLogicalSubjectPreimage {
                domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
                subject_kind: SubjectKind::OutcomeRun,
                source_fact_key: None,
                config_hash: Some(stage.config_hash.clone()),
                sample_key: Some(stage.sample_key.clone()),
                outcome_phase: Some(stage.outcome_phase),
                stored_due_date: Some(stage.stored_due_date.clone()),
                ingress_source_batch_hash: None,
            })
            .expect("rehash drifted outcome logical subject");
            let request = outcome_request(bind_outcome_claim(stage, &claim));

            let error = repository
                .persist_outcome_envelope(&mut conn, &mut session, &request)
                .expect_err("outcome authority must reject request drift");

            assert!(matches!(
                error,
                SelectionV2RepositoryError::Invariant {
                    code: "outcome_claim_manifest_binding_mismatch",
                    ..
                }
            ));
            session.finish().expect("finish locked audit session");
        }
    }

    #[test]
    fn outcome_envelope_is_append_only_then_stage_replays_exactly_and_conflicts_on_change() {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new("outcome-replay-conflict");
        let writer = test_audit_writer(&root);
        let mut session = writer.locked_session().expect("lock audit chain");
        let claim = stage_generation_upstream(&mut conn, &repository, &mut session);
        let request = outcome_request(bind_outcome_claim(
            outcome_stage(OutcomeAttemptResult::ExpectedWait),
            &claim,
        ));
        let initially_persisted = repository
            .persist_outcome_envelope(&mut conn, &mut session, &request)
            .expect("persist envelope before simulating durable-row loss");
        let initial_proof =
            prepared_proof(&repository, &mut conn, &mut session, &initially_persisted);
        let delete_error =
            diesel::sql_query("DELETE FROM selection_v2_recovery_envelopes WHERE stage_run_id=?")
                .bind::<Text, _>(TEST_OUTCOME_RUN_ID)
                .execute(&mut conn)
                .expect_err("append-only recovery envelope must not be removed");
        assert!(
            delete_error
                .to_string()
                .contains("BR-174 append-only DELETE denied"),
            "unexpected append-only error: {delete_error}"
        );
        let staged_at = DateTime::parse_from_rfc3339("2026-07-28T01:03:02.000000000Z")
            .expect("fixed outcome timestamp")
            .with_timezone(&Utc);

        let persisted = repository
            .persist_outcome_envelope(&mut conn, &mut session, &request)
            .expect("replay exact outcome envelope");
        assert_eq!(persisted.disposition, StageDisposition::ExactReplay);
        let proof = repository
            .load_prepared_proof(
                &mut conn,
                &mut session,
                &persisted,
                prepared_content(&persisted),
            )
            .expect("recover exact Prepared proof under refreshed lock resolution");
        assert_eq!(proof.record_hash(), initial_proof.record_hash());
        let first = repository
            .stage_outcome(
                &mut conn,
                &request,
                &persisted,
                &mut session,
                &proof,
                staged_at,
            )
            .expect("first outcome stage");
        let replay = repository
            .stage_outcome(
                &mut conn,
                &request,
                &persisted,
                &mut session,
                &proof,
                staged_at,
            )
            .expect("exact outcome replay");
        assert_eq!(first.disposition, StageDisposition::Inserted);
        assert_eq!(replay.disposition, StageDisposition::ExactReplay);

        let changed_time = DateTime::parse_from_rfc3339("2026-07-28T01:03:03.000000000Z")
            .expect("changed stage timestamp")
            .with_timezone(&Utc);
        let conflict = repository
            .stage_outcome(
                &mut conn,
                &request,
                &persisted,
                &mut session,
                &proof,
                changed_time,
            )
            .expect_err("same outcome run with changed manifest must conflict");
        assert!(matches!(
            conflict,
            SelectionV2RepositoryError::ReplayConflict {
                subject_kind,
                subject_id,
            } if subject_kind == "outcome_run" && subject_id == TEST_OUTCOME_RUN_ID
        ));
        session.finish().expect("finish locked audit session");
    }

    #[test]
    fn typed_generation_and_outcome_loaders_reject_tampered_columns_with_copied_hashes() {
        let generation = hard_rejected_generation_stage();
        let expected_relation = generation.relation_attempt_rows[0].clone();
        let mut tampered_relation = expected_relation.clone();
        tampered_relation.canonical_stock_name = Some("TEST_CODE_TAMPERED".into());
        let generation_error = decode_typed_rows::<SelectionRelationAttemptRowContentPreimage>(
            vec![TypedRowJsonDb {
                row_json: canonical(&tampered_relation).expect("tampered relation JSON"),
                content_hash: hash(&expected_relation).expect("copied relation hash"),
            }],
            "selection_relation_attempts",
        )
        .expect_err("copied hash must not authorize tampered generation columns");
        assert!(matches!(
            generation_error,
            SelectionV2RepositoryError::Invariant {
                code: "typed_row_readback_rehash_mismatch",
                ..
            }
        ));

        let outcome = outcome_stage(OutcomeAttemptResult::Settled);
        let expected_attempt = outcome.outcome_attempt_rows[0].clone();
        let mut tampered_attempt = expected_attempt.clone();
        tampered_attempt.request_hash = Some(test_hash('f'));
        let outcome_error = decode_typed_rows::<SelectionOutcomeAttemptRowContentPreimage>(
            vec![TypedRowJsonDb {
                row_json: canonical(&tampered_attempt).expect("tampered attempt JSON"),
                content_hash: hash(&expected_attempt).expect("copied attempt hash"),
            }],
            "selection_outcome_attempts",
        )
        .expect_err("copied hash must not authorize tampered outcome columns");
        assert!(matches!(
            outcome_error,
            SelectionV2RepositoryError::Invariant {
                code: "typed_row_readback_rehash_mismatch",
                ..
            }
        ));
    }

    #[test]
    fn second_writer_cannot_envelope_a_different_run_for_an_unreceipted_logical_subject() {
        let (mut conn, repository) = repository();
        seed_receipted_source_fact(&mut conn);
        let root = TestAuditRoot::new("logical-subject-two-writers");
        let writer = test_audit_writer(&root);

        let first_request = generation_request(verified_no_relation_stage());
        {
            let mut first_session = writer.locked_session().expect("first writer audit lock");
            let first_envelope = repository
                .persist_generation_envelope(&mut conn, &mut first_session, &first_request)
                .expect("first writer persists recovery envelope");
            let first_proof =
                prepared_proof(&repository, &mut conn, &mut first_session, &first_envelope);
            let staged_at = DateTime::parse_from_rfc3339("2026-07-28T01:02:00.000000000Z")
                .expect("fixed first stage timestamp")
                .with_timezone(&Utc);
            repository
                .stage_generation(
                    &mut conn,
                    &first_request,
                    &first_envelope,
                    &mut first_session,
                    &first_proof,
                    staged_at,
                )
                .expect("first writer leaves a recoverable unreceipted manifest");
            first_session.finish().expect("finish first writer session");
        }

        let mut second_stage = verified_no_relation_stage();
        second_stage.stage_run_id = "019849b1-e800-7000-8000-000000000104".into();
        let second_request = generation_request(second_stage);
        let mut second_session = writer.locked_session().expect("second writer audit lock");
        let error = repository
            .persist_generation_envelope(&mut conn, &mut second_session, &second_request)
            .expect_err("different run must recover the first logical subject, not cross Prepared");
        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "logical_subject_unreceipted_conflict",
                ..
            }
        ));
        second_session
            .finish()
            .expect("finish second writer session");
    }

    #[test]
    fn prepared_proof_requires_the_same_locked_logical_subject_high_water() {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new("logical-subject-prepared-high-water");
        let writer = test_audit_writer(&root);
        let mut session = writer.locked_session().expect("hold audit lock");
        let request = generation_request(verified_no_relation_stage());
        let persisted = repository
            .persist_generation_envelope(&mut conn, &mut session, &request)
            .expect("persist envelope with logical-subject capability");

        session
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::Prepared,
                "TEST_CODE_UNRELATED_AUDIT",
                test_hash('a'),
                DateTime::parse_from_rfc3339("2026-07-28T01:00:00.500000000Z")
                    .expect("fixed unrelated timestamp"),
            ))
            .expect("append unrelated record after logical-subject resolution");

        let content = PreparedAuditContentPreimage {
            domain: DOMAIN_PREPARED_AUDIT.into(),
            subject_kind: persisted.subject_kind(),
            subject_id: persisted.stage_run_id().into(),
            logical_subject_key: persisted.logical_subject_key().into(),
            recovery_envelope_content_hash: persisted.content_hash().into(),
            in_memory_payload_hash: persisted.in_memory_payload_hash().into(),
        };
        let content_hash = hash(&content).expect("hash Prepared audit content");
        session
            .append(SelectionAuditRecord::new(
                prepared_phase(content.subject_kind),
                &content.subject_id,
                &content_hash,
                DateTime::parse_from_rfc3339("2026-07-28T01:00:01.000000000Z")
                    .expect("fixed Prepared timestamp"),
            ))
            .expect("append Prepared after stale logical-subject resolution");

        let error = repository
            .load_prepared_proof(&mut conn, &mut session, &persisted, content)
            .expect_err("an intervening audit append must invalidate the lock capability");
        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "logical_subject_lock_audit_tail_changed",
                ..
            }
        ));
        session.finish().expect("finish audit session");
    }

    #[test]
    fn same_run_recovers_the_exact_prepared_capability_without_duplicate_append() {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new("logical-subject-prepared-recovery");
        let writer = test_audit_writer(&root);
        let request = generation_request(verified_no_relation_stage());

        {
            let mut first = writer.locked_session().expect("first recovery writer");
            let persisted = repository
                .persist_generation_envelope(&mut conn, &mut first, &request)
                .expect("persist first envelope");
            prepared_proof(&repository, &mut conn, &mut first, &persisted);
            first.finish().expect("finish first recovery writer");
        }
        let record_count = writer
            .validate()
            .expect("validate first Prepared")
            .record_count;

        let mut recovering = writer.locked_session().expect("recover same run");
        let persisted = repository
            .persist_generation_envelope(&mut conn, &mut recovering, &request)
            .expect("recover exact envelope");
        assert_eq!(
            persisted.logical_subject_state(),
            Some(&LogicalSubjectLockState::Recovering {
                subject_id: TEST_GENERATION_RUN_ID.into(),
                manifest_present: false,
            })
        );
        repository
            .load_prepared_proof(
                &mut conn,
                &mut recovering,
                &persisted,
                prepared_content(&persisted),
            )
            .expect("recover existing exact Prepared without a second append");
        assert_eq!(
            recovering
                .validate()
                .expect("validate recovered audit chain")
                .record_count,
            record_count
        );
        recovering.finish().expect("finish recovery writer");
    }

    #[test]
    fn latest_receipted_logical_subject_breaks_equal_time_ties_by_subject_id_descending() {
        let (mut conn, repository) = repository();
        seed_receipted_source_fact(&mut conn);
        let root = TestAuditRoot::new("logical-subject-receipt-tie-break");
        let writer = test_audit_writer(&root);
        let committed_at = DateTime::parse_from_rfc3339("2026-07-28T01:00:03.000000000Z")
            .expect("fixed equal commit timestamp");

        for (subject_id, staged_at_text) in [
            (
                "019849b1-e800-7000-8000-000000000104",
                "2026-07-28T01:00:02.000000000Z",
            ),
            (
                "019849b1-e800-7000-8000-000000000105",
                "2026-07-28T01:00:04.000000000Z",
            ),
        ] {
            let mut stage = verified_no_relation_stage();
            stage.stage_run_id = subject_id.into();
            let request = generation_request(stage);
            let mut session = writer.locked_session().expect("lock generation writer");
            let persisted = repository
                .persist_generation_envelope(&mut conn, &mut session, &request)
                .expect("persist generation envelope");
            let prepared = prepared_proof(&repository, &mut conn, &mut session, &persisted);
            let staged_at = DateTime::parse_from_rfc3339(staged_at_text)
                .expect("fixed stage timestamp")
                .with_timezone(&Utc);
            let staged = repository
                .stage_generation(
                    &mut conn,
                    &request,
                    &persisted,
                    &mut session,
                    &prepared,
                    staged_at,
                )
                .expect("stage generation run");
            commit_generation_run(
                &repository,
                &mut conn,
                &mut session,
                &prepared,
                &staged,
                committed_at,
            );
            session.finish().expect("finish generation writer");
        }

        let mut next_stage = verified_no_relation_stage();
        next_stage.stage_run_id = "019849b1-e800-7000-8000-000000000106".into();
        let next_request = generation_request(next_stage);
        let mut next = writer
            .locked_session()
            .expect("lock next generation writer");
        let persisted = repository
            .persist_generation_envelope(&mut conn, &mut next, &next_request)
            .expect("resolve latest receipted logical subject");
        assert!(matches!(
            persisted.logical_subject_state(),
            Some(LogicalSubjectLockState::Receipted(latest))
                if latest.subject_id == "019849b1-e800-7000-8000-000000000105"
                    && latest.committed_at_rfc3339_nanos_utc
                        == "2026-07-28T01:00:03.000000000Z"
        ));
        next.finish().expect("finish tie-break observer");
    }

    #[test]
    fn config_hash_reuse_reports_absent_without_manufacturing_evidence() {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new("config-reuse-absent");
        let writer = test_audit_writer(&root);
        let mut session = writer.locked_session().expect("lock empty audit chain");

        let state = repository
            .config_hash_reuse(&mut conn, &mut session, &test_hash('5'), &test_hash('6'))
            .expect("query absent config hash");
        assert_eq!(state, ConfigHashReuse::Absent);
        session.finish().expect("finish absent lookup");
    }

    #[test]
    fn config_hash_reuse_distinguishes_staged_unreceipted_from_receipted_exact() {
        for with_receipt in [false, true] {
            let (mut conn, repository) = repository();
            let root = TestAuditRoot::new(if with_receipt {
                "config-reuse-receipted"
            } else {
                "config-reuse-staged"
            });
            let writer = test_audit_writer(&root);
            let mut session = writer.locked_session().expect("lock config audit chain");
            let config_hash = test_hash('5');
            let subject_id = "019849b1-e800-7000-8000-000000000107";
            let (manifest_hash, receipt_hash) = stage_config_manifest_fixture(
                &mut conn,
                &mut session,
                subject_id,
                &config_hash,
                with_receipt,
            );

            let state = repository
                .config_hash_reuse(&mut conn, &mut session, &config_hash, &manifest_hash)
                .expect("resolve exact config reuse evidence");
            match (with_receipt, state) {
                (
                    false,
                    ConfigHashReuse::StagedUnreceipted {
                        activation_run_id,
                        manifest_content_hash,
                        ..
                    },
                ) => {
                    assert_eq!(activation_run_id, subject_id);
                    assert_eq!(manifest_content_hash, manifest_hash);
                }
                (
                    true,
                    ConfigHashReuse::ReceiptedExact {
                        activation_run_id,
                        manifest_content_hash,
                        receipt_content_hash,
                        ..
                    },
                ) => {
                    assert_eq!(activation_run_id, subject_id);
                    assert_eq!(manifest_content_hash, manifest_hash);
                    assert_eq!(Some(receipt_content_hash), receipt_hash);
                }
                (expected_receipt, actual) => {
                    panic!("unexpected config reuse state receipt={expected_receipt}: {actual:?}")
                }
            }
            session.finish().expect("finish config reuse lookup");
        }
    }

    #[test]
    fn config_hash_reuse_reports_manifest_conflict_only_after_exact_prepared_evidence() {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new("config-reuse-conflict");
        let writer = test_audit_writer(&root);
        let mut session = writer.locked_session().expect("lock config audit chain");
        let config_hash = test_hash('5');
        let subject_id = "019849b1-e800-7000-8000-000000000108";
        let (manifest_hash, _) =
            stage_config_manifest_fixture(&mut conn, &mut session, subject_id, &config_hash, false);

        let state = repository
            .config_hash_reuse(&mut conn, &mut session, &config_hash, &test_hash('f'))
            .expect("resolve exact conflict evidence");
        assert_eq!(
            state,
            ConfigHashReuse::Conflict {
                activation_run_id: subject_id.into(),
                manifest_content_hash: manifest_hash,
            }
        );
        session.finish().expect("finish conflict lookup");
    }

    #[test]
    fn immediate_transaction_rolls_back_primary_failure_without_reusing_an_open_transaction() {
        let (mut conn, repository) = repository();
        conn.batch_execute(
            "CREATE TABLE TEST_CODE_transaction_probe (
                 value INTEGER NOT NULL
             );",
        )
        .expect("create transaction probe");

        let error = run_immediate_transaction(&mut conn, |conn| {
            diesel::sql_query("INSERT INTO TEST_CODE_transaction_probe(value) VALUES (1)")
                .execute(conn)?;
            Err::<(), _>(invariant(
                "TEST_CODE_primary_failure",
                "force a repository operation failure inside BEGIN IMMEDIATE",
            ))
        })
        .expect_err("primary failure must roll back");

        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "TEST_CODE_primary_failure",
                ..
            }
        ));
        #[derive(QueryableByName)]
        struct Count {
            #[diesel(sql_type = BigInt)]
            count: i64,
        }
        let count = diesel::sql_query("SELECT COUNT(*) AS count FROM TEST_CODE_transaction_probe")
            .get_result::<Count>(&mut conn)
            .expect("query rolled-back rows");
        assert_eq!(count.count, 0);
        repository
            .verify(&mut conn)
            .expect("Diesel transaction manager closed the transaction cleanly");
    }

    #[test]
    fn outcome_persistence_owner_commits_once_and_exactly_replays() {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new("outcome-owner-success-replay");
        let writer = test_audit_writer(&root);
        let mut upstream_session = writer.locked_session().expect("lock upstream audit");
        let claim = stage_generation_upstream(&mut conn, &repository, &mut upstream_session);
        upstream_session.finish().expect("finish upstream audit");

        let request = outcome_request(bind_outcome_claim(
            outcome_stage(OutcomeAttemptResult::Settled),
            &claim,
        ));
        let inserted = crate::selection::persistence_v2::commit_outcome_request_for_test(
            &mut conn,
            &writer,
            request.clone(),
        )
        .expect("commit outcome through the sole owner");
        let replayed = crate::selection::persistence_v2::commit_outcome_request_for_test(
            &mut conn, &writer, request,
        )
        .expect("exactly replay outcome through the sole owner");

        assert_eq!(inserted.disposition(), StageDisposition::Inserted);
        assert_eq!(replayed.disposition(), StageDisposition::ExactReplay);
        assert_eq!(inserted.subject_id(), TEST_OUTCOME_RUN_ID);
        assert_eq!(inserted.content_hash(), replayed.content_hash());

        let mut audit = writer.locked_session().expect("read outcome audit");
        let snapshot = audit.validated_records().expect("validate outcome audit");
        for phase in [
            SelectionAuditPhase::V2OutcomePrepared,
            SelectionAuditPhase::V2OutcomeCommitted,
        ] {
            assert_eq!(
                snapshot
                    .records()
                    .iter()
                    .filter(|record| {
                        record.phase == phase && record.subject_id == TEST_OUTCOME_RUN_ID
                    })
                    .count(),
                1,
                "exact replay must not append duplicate {phase:?} audit evidence"
            );
        }
        audit.finish().expect("finish outcome audit read");
    }

    #[test]
    fn outcome_receipt_survives_database_close_reopen_and_exact_owner_replay() {
        let root = TestAuditRoot::new("outcome-owner-file-reopen");
        let (database_path, mut first_connection, first_repository) = file_repository(&root);
        let writer = test_audit_writer(&root);
        let mut upstream_session = writer.locked_session().expect("lock file upstream audit");
        let claim = stage_generation_upstream(
            &mut first_connection,
            &first_repository,
            &mut upstream_session,
        );
        upstream_session
            .finish()
            .expect("finish file upstream audit");

        let request = outcome_request(bind_outcome_claim(
            outcome_stage(OutcomeAttemptResult::Settled),
            &claim,
        ));
        let inserted = crate::selection::persistence_v2::commit_outcome_request_for_test(
            &mut first_connection,
            &writer,
            request.clone(),
        )
        .expect("commit persistent outcome receipt");
        let inserted_hash = inserted.content_hash().to_owned();
        drop(first_connection);

        let database_url = database_path
            .to_str()
            .expect("isolated TEST_CODE database path remains UTF-8");
        let mut reopened =
            SqliteConnection::establish(database_url).expect("reopen persistent outcome database");
        SelectionV2Repository::initialize_for_final_database_half_test(
            &mut reopened,
            SelectionV2StoreMode::Test,
        )
        .expect("verify reopened final database catalog");
        let persisted_receipt = find_commit_receipt(&mut reopened, TEST_OUTCOME_RUN_ID)
            .expect("query receipt after database reopen")
            .expect("receipt must survive database reopen");
        assert_eq!(persisted_receipt.content_hash, inserted_hash);

        let replayed = crate::selection::persistence_v2::commit_outcome_request_for_test(
            &mut reopened,
            &writer,
            request,
        )
        .expect("exact replay after database reopen");
        assert_eq!(replayed.disposition(), StageDisposition::ExactReplay);
        assert_eq!(replayed.content_hash(), inserted_hash);
    }

    #[test]
    fn outcome_persistence_owner_repairs_exact_committed_without_receipt() {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new("outcome-owner-repair-receipt");
        let writer = test_audit_writer(&root);
        let mut upstream_session = writer.locked_session().expect("lock upstream audit");
        let claim = stage_generation_upstream(&mut conn, &repository, &mut upstream_session);
        upstream_session.finish().expect("finish upstream audit");

        let request = outcome_request(bind_outcome_claim(
            outcome_stage(OutcomeAttemptResult::Settled),
            &claim,
        ));
        crate::selection::persistence_v2::commit_outcome_request_for_test(
            &mut conn,
            &writer,
            request.clone(),
        )
        .expect("commit initial outcome");
        conn.batch_execute("DROP TRIGGER selection_v2_commit_receipts_deny_delete")
            .expect("test-only crash injection drops the append-only guard");
        diesel::sql_query(
            "DELETE FROM selection_v2_commit_receipts
             WHERE subject_kind='outcome_run' AND subject_id=?",
        )
        .bind::<Text, _>(TEST_OUTCOME_RUN_ID)
        .execute(&mut conn)
        .expect("simulate crash after Committed audit and before receipt");
        install_selection_v2_final_database_half_for_test(&mut conn, SelectionV2StoreMode::Test)
            .expect("restore and verify the exact final test catalog after crash injection");

        let repaired = crate::selection::persistence_v2::commit_outcome_request_for_test(
            &mut conn, &writer, request,
        )
        .expect("repair exact missing receipt");
        assert_eq!(repaired.disposition(), StageDisposition::Inserted);

        let receipt = find_commit_receipt(&mut conn, TEST_OUTCOME_RUN_ID)
            .expect("query repaired receipt")
            .expect("receipt must be restored");
        assert_eq!(receipt.subject_id, TEST_OUTCOME_RUN_ID);
    }

    #[test]
    fn outcome_persistence_owner_rejects_prepared_content_conflict() {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new("outcome-owner-content-conflict");
        let writer = test_audit_writer(&root);
        let mut upstream_session = writer.locked_session().expect("lock upstream audit");
        let claim = stage_generation_upstream(&mut conn, &repository, &mut upstream_session);
        upstream_session.finish().expect("finish upstream audit");

        writer
            .append(SelectionAuditRecord::new(
                SelectionAuditPhase::V2OutcomePrepared,
                TEST_OUTCOME_RUN_ID,
                test_hash('f'),
                DateTime::parse_from_rfc3339("2026-07-28T01:03:01.000000000Z")
                    .expect("fixed conflicting Prepared timestamp"),
            ))
            .expect("append conflicting Prepared evidence");

        let error = crate::selection::persistence_v2::commit_outcome_request_for_test(
            &mut conn,
            &writer,
            outcome_request(bind_outcome_claim(
                outcome_stage(OutcomeAttemptResult::Settled),
                &claim,
            )),
        )
        .expect_err("owner must reject a conflicting Prepared audit record");
        assert!(
            matches!(
                error,
                crate::selection::persistence_v2::SelectionV2PersistenceError::Repository(
                    SelectionV2RepositoryError::Invariant {
                        code: "logical_subject_prepared_content_conflict",
                        ..
                    }
                )
            ),
            "unexpected conflicting Prepared error: {error:?}"
        );
        assert!(
            find_commit_receipt(&mut conn, TEST_OUTCOME_RUN_ID)
                .expect("query conflicting receipt")
                .is_none(),
            "content conflict must not manufacture a commit receipt"
        );
    }

    fn receipted_claim_lifecycle_fixture(
        label: &str,
    ) -> (
        SqliteConnection,
        SelectionV2Repository,
        TestAuditRoot,
        SelectionAuditWriter,
        TestOutcomeClaimBinding,
    ) {
        let (mut conn, repository) = repository();
        let root = TestAuditRoot::new(label);
        let writer = test_audit_writer(&root);
        let mut session = writer.locked_session().expect("lock claim fixture audit");
        let claim = stage_generation_upstream(&mut conn, &repository, &mut session);
        session.finish().expect("finish claim fixture audit");
        (conn, repository, root, writer, claim)
    }

    #[test]
    fn receipted_claim_without_outcome_receipt_is_recovery_not_due() {
        let (mut conn, _repository, _root, _writer, claim) =
            receipted_claim_lifecycle_fixture("receipted-claim-is-recovery");
        let lifecycle =
            classify_outcome_claim_lifecycle(&mut conn, &outcome_claim_stage().logical_subject_key)
                .expect("classify receipted active claim")
                .expect("claim lifecycle exists");

        assert_eq!(lifecycle.class(), OutcomeClaimLifecycleClass::ClaimActive);
        assert_eq!(lifecycle.claim_id(), claim.claim_id);
        assert!(lifecycle.blocks_new_due());
    }

    #[test]
    fn recovery_reuses_exact_claim_and_planned_outcome_run_ids() {
        let (mut conn, _repository, _root, _writer, claim) =
            receipted_claim_lifecycle_fixture("claim-recovery-reuses-ids");
        let lifecycle =
            classify_outcome_claim_lifecycle(&mut conn, &outcome_claim_stage().logical_subject_key)
                .expect("classify active claim")
                .expect("active claim exists");

        assert_eq!(lifecycle.claim_id(), claim.claim_id);
        assert_eq!(
            lifecycle.planned_outcome_run_id(),
            outcome_claim_stage().planned_outcome_run_id
        );
        assert_eq!(lifecycle.claim_stage().stage_run_id, lifecycle.claim_id());
    }

    #[test]
    fn second_claim_is_rejected_while_exact_claim_is_unclosed() {
        let (mut conn, repository, _root, writer, _claim) =
            receipted_claim_lifecycle_fixture("second-claim-rejected");
        let mut second = outcome_claim_stage();
        second.stage_run_id = "01900000-0000-7000-8000-0000000000aa".into();
        second.planned_outcome_run_id = "01900000-0000-7000-8000-0000000000ab".into();
        let request = OutcomeClaimStageRequest::from_stage_input(
            second,
            DateTime::parse_from_rfc3339("2026-07-28T01:03:10.000000000Z")
                .expect("fixed second claim timestamp")
                .with_timezone(&Utc),
        )
        .expect("build second claim request");
        let mut session = writer.locked_session().expect("lock second claim audit");
        let error = repository
            .persist_outcome_claim_envelope(&mut conn, &mut session, &request)
            .expect_err("an unclosed claim must exclude a second claim");
        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "outcome_claim_unclosed_conflict",
                ..
            }
        ));
    }

    #[test]
    fn active_claim_recovery_closes_original_claim_without_new_claim() {
        let (mut conn, _repository, _root, writer, claim) =
            receipted_claim_lifecycle_fixture("active-claim-original-identity");
        let before =
            classify_outcome_claim_lifecycle(&mut conn, &outcome_claim_stage().logical_subject_key)
                .expect("classify active claim")
                .expect("active claim exists");
        let recovery = before
            .into_recovery_material()
            .expect("active claim yields exact recovery material");

        assert_eq!(recovery.claim_stage.stage_run_id, claim.claim_id);
        assert_eq!(
            recovery.claim_stage.planned_outcome_run_id,
            TEST_OUTCOME_RUN_ID
        );
        assert_eq!(recovery.outcome_stage, None);
        crate::selection::persistence_v2::commit_outcome_request_for_test(
            &mut conn,
            &writer,
            outcome_request(bind_outcome_claim(
                outcome_stage(OutcomeAttemptResult::ExpectedWait),
                &claim,
            )),
        )
        .expect("close the exact active claim");
        let claim_count = diesel::sql_query(
            "SELECT COUNT(*) AS value
             FROM selection_v2_recovery_envelopes
             WHERE subject_kind='outcome_claim' AND logical_subject_key=?",
        )
        .bind::<Text, _>(&recovery.claim_stage.logical_subject_key)
        .get_result::<IntegerValueRow>(&mut conn)
        .expect("count exact claim envelopes");
        assert_eq!(claim_count.value, 1);
        let after =
            classify_outcome_claim_lifecycle(&mut conn, &recovery.claim_stage.logical_subject_key)
                .expect("classify closed original claim")
                .expect("closed claim remains auditable");
        assert_eq!(after.class(), OutcomeClaimLifecycleClass::Closed);
    }

    #[test]
    fn outcome_envelope_without_receipt_recovers_without_provider_refetch() {
        let (mut conn, repository, _root, writer, claim) =
            receipted_claim_lifecycle_fixture("outcome-envelope-recovery");
        let request = outcome_request(bind_outcome_claim(
            outcome_stage(OutcomeAttemptResult::Settled),
            &claim,
        ));
        let mut session = writer
            .locked_session()
            .expect("lock outcome envelope audit");
        repository
            .persist_outcome_envelope(&mut conn, &mut session, &request)
            .expect("persist outcome envelope before simulated crash");
        session.finish().expect("finish outcome envelope audit");

        let lifecycle =
            classify_outcome_claim_lifecycle(&mut conn, &outcome_claim_stage().logical_subject_key)
                .expect("classify partial outcome")
                .expect("claim lifecycle exists");
        assert_eq!(
            lifecycle.class(),
            OutcomeClaimLifecycleClass::OutcomeRecovery
        );
        assert!(!lifecycle.requires_provider_refetch());
        assert_eq!(
            lifecycle
                .into_recovery_material()
                .expect("outcome recovery material")
                .outcome_stage
                .expect("exact outcome stage")
                .stage_run_id,
            TEST_OUTCOME_RUN_ID
        );
    }

    #[test]
    fn exact_outcome_receipt_removes_claim_from_recovery() {
        let (mut conn, _repository, _root, writer, claim) =
            receipted_claim_lifecycle_fixture("closed-claim-lifecycle");
        crate::selection::persistence_v2::commit_outcome_request_for_test(
            &mut conn,
            &writer,
            outcome_request(bind_outcome_claim(
                outcome_stage(OutcomeAttemptResult::ExpectedWait),
                &claim,
            )),
        )
        .expect("commit exact planned outcome");

        let lifecycle =
            classify_outcome_claim_lifecycle(&mut conn, &outcome_claim_stage().logical_subject_key)
                .expect("classify closed claim")
                .expect("closed lifecycle remains auditable");
        assert_eq!(lifecycle.class(), OutcomeClaimLifecycleClass::Closed);
        assert!(!lifecycle.blocks_new_due());
        assert!(lifecycle.into_recovery_material().is_none());
    }

    #[test]
    fn cross_claim_or_wrong_planned_run_is_integrity_error_not_closed() {
        let error = classify_outcome_claim_artifact_matrix(&OutcomeClaimArtifactMatrix {
            claim_manifest: true,
            claim_receipt: true,
            outcome_envelope: true,
            outcome_manifest: true,
            outcome_receipt: true,
            exact_claim_and_planned_run_binding: false,
        })
        .expect_err("cross-claim closure must fail closed");
        assert!(matches!(
            error,
            SelectionV2RepositoryError::Invariant {
                code: "outcome_claim_lifecycle_cross_binding",
                ..
            }
        ));
    }

    #[test]
    fn crash_after_claim_receipt_before_outcome_envelope_recovers_one_logical_claim() {
        let (mut conn, _repository, _root, _writer, claim) =
            receipted_claim_lifecycle_fixture("claim-crash-single-owner");
        let key = outcome_claim_stage().logical_subject_key;
        let first = classify_outcome_claim_lifecycle(&mut conn, &key)
            .expect("first recovery classification")
            .expect("active claim exists");
        let second = classify_outcome_claim_lifecycle(&mut conn, &key)
            .expect("second recovery classification")
            .expect("same active claim exists");

        assert_eq!(first.class(), OutcomeClaimLifecycleClass::ClaimActive);
        assert_eq!(first.claim_id(), claim.claim_id);
        assert_eq!(first.claim_id(), second.claim_id());
        assert_eq!(
            first.planned_outcome_run_id(),
            second.planned_outcome_run_id()
        );
    }
}
