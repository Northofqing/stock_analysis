//! BR-174 sole durable persistence owner for schema-v2 stage runs.
//!
//! Provider and network acquisition must finish before entering this module.
//! The production surface owns the process database, the fixed production
//! selection-audit namespace, every choreography timestamp, and the complete
//! OS-lock critical section.

use crate::database::selection_v2::SelectionV2StoreMode;
use crate::database::selection_v2_repository::{
    CommitReceipt, CommittedAuditProof, ConfigActivationStageRequest, GenerationStageRequest,
    NonOutcomeRecoveryStageRequest, OutcomeClaimStageRequest, OutcomeStageRequest,
    PersistedRecoveryEnvelope, PreparedAuditProof, SelectionV2Repository,
    SelectionV2RepositoryError, SourceIngressStageRequest, StagedRunReceipt,
};
use crate::database::DatabaseManager;
use crate::selection::audit::{
    AuditExactLookup, LockedSelectionAuditSession, SelectionAuditError, SelectionAuditPhase,
    SelectionAuditRecord, SelectionAuditWriter,
};
use crate::selection::config_activation_v2::PreparedConfigActivation;
use crate::selection::ingress_v2::PreparedSourceIngress;
use crate::selection::outcome_v2::{
    PreparedOutcomeClaimStage, PreparedOutcomeStage, ReceiptedOutcomeClaim,
};
use crate::selection::schema_v2::{
    sha256_json, CommittedAuditContentPreimage, PreparedAuditContentPreimage, SubjectKind,
    DOMAIN_COMMITTED_AUDIT, DOMAIN_PREPARED_AUDIT,
};
use chrono::{DateTime, Utc};
use diesel::SqliteConnection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SelectionV2PersistenceError {
    #[error("selection-v2 process database is not initialized")]
    DatabaseNotInitialized,
    #[error("selection-v2 process database connection failed: {0}")]
    DatabaseConnection(String),
    #[error(transparent)]
    Repository(#[from] SelectionV2RepositoryError),
    #[error(transparent)]
    Audit(#[from] SelectionAuditError),
    #[error("selection-v2 audit content canonicalization failed: {0}")]
    Canonical(String),
    #[error("selection-v2 outcome claim capability construction failed: {0}")]
    OutcomeClaimCapability(String),
    #[error(
        "selection-v2 audit content conflict: phase={phase:?} subject_id={subject_id} expected={expected_content_hash} existing={existing_content_hash}"
    )]
    AuditContentConflict {
        phase: SelectionAuditPhase,
        subject_id: String,
        expected_content_hash: String,
        existing_content_hash: String,
    },
    #[error(
        "selection-v2 audit append readback mismatch: phase={phase:?} subject_id={subject_id}"
    )]
    AuditAppendReadbackMismatch {
        phase: SelectionAuditPhase,
        subject_id: String,
    },
}

pub type SelectionV2PersistenceResult<T> = Result<T, SelectionV2PersistenceError>;

/// Sole production entry point for schema-v2 durable stage choreography.
///
/// Every method consumes its request/capability by value. No production API
/// accepts a SQLite connection, database path, audit writer/root, locked
/// session, or owner wall-clock value.
pub struct SelectionV2PersistenceOwner;

impl SelectionV2PersistenceOwner {
    /// Completes one exact durable non-outcome stage without reconstructing
    /// provider inputs or crossing into outcome claim/run ownership.
    pub(crate) fn recover_non_outcome(
        request: NonOutcomeRecoveryStageRequest,
    ) -> SelectionV2PersistenceResult<CommitReceipt> {
        let request = match request {
            NonOutcomeRecoveryStageRequest::ConfigActivation(request) => {
                DurableStageRequest::ConfigActivation(Box::new(request))
            }
            NonOutcomeRecoveryStageRequest::SourceIngress(request) => {
                DurableStageRequest::SourceIngress(Box::new(request))
            }
            NonOutcomeRecoveryStageRequest::Generation(request) => {
                DurableStageRequest::Generation(Box::new(request))
            }
        };
        commit_production(request)
    }

    #[allow(
        dead_code,
        reason = "BR-183 keeps selection-v2 activation persistence disabled until release evidence closes"
    )]
    pub(crate) fn commit_config_activation(
        prepared: PreparedConfigActivation,
    ) -> SelectionV2PersistenceResult<CommitReceipt> {
        let request = ConfigActivationStageRequest::try_from_prepared(&prepared)?;
        commit_production(DurableStageRequest::ConfigActivation(Box::new(request)))
    }

    pub fn commit_source_ingress(
        prepared: PreparedSourceIngress,
    ) -> SelectionV2PersistenceResult<CommitReceipt> {
        let request = SourceIngressStageRequest::try_from_prepared(&prepared)?;
        commit_production(DurableStageRequest::SourceIngress(Box::new(request)))
    }

    /// Internal bridge only. Generation must not become a public production
    /// entry point until an independent opaque `PreparedGeneration` owner
    /// capability exists.
    #[allow(
        dead_code,
        reason = "BR-183 keeps selection-v2 generation persistence disabled until release evidence closes"
    )]
    pub(crate) fn commit_generation(
        request: GenerationStageRequest,
    ) -> SelectionV2PersistenceResult<CommitReceipt> {
        commit_production(DurableStageRequest::Generation(Box::new(request)))
    }

    pub(crate) fn commit_outcome_claim(
        prepared: PreparedOutcomeClaimStage,
    ) -> SelectionV2PersistenceResult<ReceiptedOutcomeClaim> {
        let stage_input = prepared.into_stage_input();
        let outcome_claim_id = stage_input.stage_run_id.clone();
        let planned_outcome_run_id = stage_input.planned_outcome_run_id.clone();
        let due_binding_hash = stage_input.due_binding_hash.clone();
        let provider_request_hash = stage_input.provider_request_hash.clone();
        let request = OutcomeClaimStageRequest::from_stage_input(stage_input, Utc::now())?;
        let receipt = commit_production(DurableStageRequest::OutcomeClaim(Box::new(request)))?;
        ReceiptedOutcomeClaim::validated(
            outcome_claim_id,
            planned_outcome_run_id,
            receipt.content_hash().into(),
            due_binding_hash,
            provider_request_hash,
        )
        .map_err(|error| SelectionV2PersistenceError::OutcomeClaimCapability(error.to_string()))
    }

    pub fn commit_outcome(
        prepared: PreparedOutcomeStage,
    ) -> SelectionV2PersistenceResult<CommitReceipt> {
        let request = OutcomeStageRequest::from_prepared_outcome(prepared, Utc::now())?;
        commit_production(DurableStageRequest::Outcome(Box::new(request)))
    }
}

fn commit_production(request: DurableStageRequest) -> SelectionV2PersistenceResult<CommitReceipt> {
    let database =
        DatabaseManager::try_get().ok_or(SelectionV2PersistenceError::DatabaseNotInitialized)?;
    let mut connection = database
        .get_conn()
        .map_err(|error| SelectionV2PersistenceError::DatabaseConnection(error.to_string()))?;
    let writer = SelectionAuditWriter::production()?;
    commit_with_owned_resources(
        &mut connection,
        &writer,
        SelectionV2StoreMode::Production,
        request,
    )
}

enum DurableStageRequest {
    ConfigActivation(Box<ConfigActivationStageRequest>),
    SourceIngress(Box<SourceIngressStageRequest>),
    Generation(Box<GenerationStageRequest>),
    OutcomeClaim(Box<OutcomeClaimStageRequest>),
    Outcome(Box<OutcomeStageRequest>),
}

impl DurableStageRequest {
    fn stage_run_id(&self) -> &str {
        match self {
            Self::ConfigActivation(request) => request.stage_run_id(),
            Self::SourceIngress(request) => request.stage_run_id(),
            Self::Generation(request) => request.stage_run_id(),
            Self::OutcomeClaim(request) => request.stage_run_id(),
            Self::Outcome(request) => request.stage_run_id(),
        }
    }

    fn normalize_enveloped_at(
        self,
        enveloped_at: DateTime<Utc>,
    ) -> Result<Self, SelectionV2RepositoryError> {
        Ok(match self {
            Self::ConfigActivation(request) => {
                Self::ConfigActivation(Box::new((*request).with_owner_enveloped_at(enveloped_at)?))
            }
            Self::SourceIngress(request) => {
                Self::SourceIngress(Box::new((*request).with_owner_enveloped_at(enveloped_at)?))
            }
            Self::Generation(request) => {
                Self::Generation(Box::new((*request).with_owner_enveloped_at(enveloped_at)?))
            }
            Self::OutcomeClaim(request) => {
                Self::OutcomeClaim(Box::new((*request).with_owner_enveloped_at(enveloped_at)?))
            }
            Self::Outcome(request) => {
                Self::Outcome(Box::new((*request).with_owner_enveloped_at(enveloped_at)?))
            }
        })
    }

    fn persist_envelope(
        &self,
        repository: &SelectionV2Repository,
        conn: &mut SqliteConnection,
        session: &mut LockedSelectionAuditSession<'_>,
    ) -> Result<PersistedRecoveryEnvelope, SelectionV2RepositoryError> {
        match self {
            Self::ConfigActivation(request) => {
                repository.persist_config_activation_envelope(conn, session, request)
            }
            Self::SourceIngress(request) => {
                repository.persist_source_ingress_envelope(conn, session, request)
            }
            Self::Generation(request) => {
                repository.persist_generation_envelope(conn, session, request)
            }
            Self::OutcomeClaim(request) => {
                repository.persist_outcome_claim_envelope(conn, session, request)
            }
            Self::Outcome(request) => repository.persist_outcome_envelope(conn, session, request),
        }
    }

    fn stage(
        &self,
        repository: &SelectionV2Repository,
        conn: &mut SqliteConnection,
        session: &mut LockedSelectionAuditSession<'_>,
        envelope: &PersistedRecoveryEnvelope,
        prepared: &PreparedAuditProof,
        staged_at: DateTime<Utc>,
    ) -> Result<StagedRunReceipt, SelectionV2RepositoryError> {
        match self {
            Self::ConfigActivation(request) => repository
                .stage_config_activation(conn, request, envelope, session, prepared, staged_at),
            Self::SourceIngress(request) => repository
                .stage_source_ingress(conn, request, envelope, session, prepared, staged_at),
            Self::Generation(request) => {
                repository.stage_generation(conn, request, envelope, session, prepared, staged_at)
            }
            Self::OutcomeClaim(request) => repository
                .stage_outcome_claim(conn, request, envelope, session, prepared, staged_at),
            Self::Outcome(request) => {
                repository.stage_outcome(conn, request, envelope, session, prepared, staged_at)
            }
        }
    }
}

fn commit_with_owned_resources(
    conn: &mut SqliteConnection,
    writer: &SelectionAuditWriter,
    mode: SelectionV2StoreMode,
    request: DurableStageRequest,
) -> SelectionV2PersistenceResult<CommitReceipt> {
    // Acquiring the writer session is the sole OS-lock boundary. The session
    // remains live until the receipt has been read back and re-hashed.
    let mut session = writer.locked_session()?;
    let repository = match mode {
        SelectionV2StoreMode::Production => {
            SelectionV2Repository::initialize_with_audit_session(conn, mode, &mut session)?
        }
        // Test fixtures are physically isolated and may intentionally model
        // partial predecessor state. Production always takes the complete
        // recovery reconciliation path above.
        SelectionV2StoreMode::Test => SelectionV2Repository::initialize(conn, mode)?,
    };

    let proposed_enveloped_at = Utc::now();
    let enveloped_at =
        repository.owner_enveloped_at(conn, request.stage_run_id(), proposed_enveloped_at)?;
    let request = request.normalize_enveloped_at(enveloped_at)?;
    let envelope = request.persist_envelope(&repository, conn, &mut session)?;

    let prepared_content = PreparedAuditContentPreimage {
        domain: DOMAIN_PREPARED_AUDIT.into(),
        subject_kind: envelope.subject_kind(),
        subject_id: envelope.stage_run_id().into(),
        logical_subject_key: envelope.logical_subject_key().into(),
        recovery_envelope_content_hash: envelope.content_hash().into(),
        in_memory_payload_hash: envelope.in_memory_payload_hash().into(),
    };
    ensure_exact_audit_record(
        &mut session,
        prepared_phase(envelope.subject_kind()),
        envelope.stage_run_id(),
        canonical_hash(&prepared_content)?,
    )?;
    let prepared =
        repository.load_prepared_proof(conn, &mut session, &envelope, prepared_content)?;

    let proposed_staged_at = Utc::now();
    let staged_at = repository.owner_staged_at(conn, request.stage_run_id(), proposed_staged_at)?;
    let staged = request.stage(
        &repository,
        conn,
        &mut session,
        &envelope,
        &prepared,
        staged_at,
    )?;
    let staged = repository.verify_staged_readback(conn, &staged.subject_id)?;

    let committed_content = CommittedAuditContentPreimage {
        domain: DOMAIN_COMMITTED_AUDIT.into(),
        subject_kind: staged.subject_kind,
        subject_id: staged.subject_id.clone(),
        logical_subject_key: staged.logical_subject_key.clone(),
        recovery_envelope_content_hash: staged.recovery_envelope_content_hash.clone(),
        prepared_record_hash: prepared.record_hash().into(),
        run_manifest_content_hash: staged.run_manifest_content_hash.clone(),
        staged_db_content_hash: staged.staged_db_content_hash.clone(),
    };
    ensure_exact_audit_record(
        &mut session,
        committed_phase(staged.subject_kind),
        &staged.subject_id,
        canonical_hash(&committed_content)?,
    )?;
    let committed = CommittedAuditProof::load(&mut session, committed_content)?;
    let receipt = repository.insert_commit_receipt(conn, &mut session, &prepared, &committed)?;

    session.finish()?;
    Ok(receipt)
}

fn ensure_exact_audit_record(
    session: &mut LockedSelectionAuditSession<'_>,
    phase: SelectionAuditPhase,
    subject_id: &str,
    content_hash: String,
) -> SelectionV2PersistenceResult<()> {
    match session.lookup_exact(phase, subject_id, &content_hash)? {
        AuditExactLookup::Exact(_) => Ok(()),
        AuditExactLookup::ContentConflict { existing_record } => {
            Err(SelectionV2PersistenceError::AuditContentConflict {
                phase,
                subject_id: subject_id.into(),
                expected_content_hash: content_hash,
                existing_content_hash: existing_record.content_hash,
            })
        }
        AuditExactLookup::Missing => {
            let appended = session.append(SelectionAuditRecord::new(
                phase,
                subject_id,
                &content_hash,
                Utc::now().fixed_offset(),
            ))?;
            match session.lookup_exact(phase, subject_id, &content_hash)? {
                AuditExactLookup::Exact(record) if record.record_hash == appended.record_hash => {
                    Ok(())
                }
                AuditExactLookup::Exact(_)
                | AuditExactLookup::Missing
                | AuditExactLookup::ContentConflict { .. } => {
                    Err(SelectionV2PersistenceError::AuditAppendReadbackMismatch {
                        phase,
                        subject_id: subject_id.into(),
                    })
                }
            }
        }
    }
}

fn canonical_hash<T: serde::Serialize>(value: &T) -> SelectionV2PersistenceResult<String> {
    sha256_json(value).map_err(|error| SelectionV2PersistenceError::Canonical(error.to_string()))
}

fn prepared_phase(kind: SubjectKind) -> SelectionAuditPhase {
    match kind {
        SubjectKind::ConfigActivation => SelectionAuditPhase::V2ConfigActivationPrepared,
        SubjectKind::IngressRun => SelectionAuditPhase::V2IngressPrepared,
        SubjectKind::GenerationRun => SelectionAuditPhase::V2GenerationPrepared,
        SubjectKind::OutcomeClaim => SelectionAuditPhase::V2OutcomeClaimPrepared,
        SubjectKind::OutcomeRun => SelectionAuditPhase::V2OutcomePrepared,
    }
}

fn committed_phase(kind: SubjectKind) -> SelectionAuditPhase {
    match kind {
        SubjectKind::ConfigActivation => SelectionAuditPhase::V2ConfigActivationCommitted,
        SubjectKind::IngressRun => SelectionAuditPhase::V2IngressCommitted,
        SubjectKind::GenerationRun => SelectionAuditPhase::V2GenerationCommitted,
        SubjectKind::OutcomeClaim => SelectionAuditPhase::V2OutcomeClaimCommitted,
        SubjectKind::OutcomeRun => SelectionAuditPhase::V2OutcomeCommitted,
    }
}

#[cfg(test)]
pub(crate) fn commit_outcome_claim_request_for_test(
    conn: &mut SqliteConnection,
    writer: &SelectionAuditWriter,
    request: OutcomeClaimStageRequest,
) -> SelectionV2PersistenceResult<CommitReceipt> {
    commit_with_owned_resources(
        conn,
        writer,
        SelectionV2StoreMode::Test,
        DurableStageRequest::OutcomeClaim(Box::new(request)),
    )
}

#[cfg(test)]
pub(crate) fn commit_outcome_request_for_test(
    conn: &mut SqliteConnection,
    writer: &SelectionAuditWriter,
    request: OutcomeStageRequest,
) -> SelectionV2PersistenceResult<CommitReceipt> {
    commit_with_owned_resources(
        conn,
        writer,
        SelectionV2StoreMode::Test,
        DurableStageRequest::Outcome(Box::new(request)),
    )
}

#[cfg(test)]
mod api_boundary_tests {
    #[test]
    fn outcome_production_entrypoint_only_consumes_opaque_prepared_stage() {
        let owner_source = include_str!("persistence_v2.rs");
        let repository_source = include_str!("../database/selection_v2_repository.rs");
        let outcome_source = include_str!("outcome_v2.rs");

        assert!(owner_source.contains("prepared: PreparedOutcomeStage"));
        let forbidden_raw_outcome_entrypoint =
            ["pub fn commit_outcome", "(request: OutcomeStageRequest)"].concat();
        assert!(!owner_source.contains(&forbidden_raw_outcome_entrypoint));
        let forbidden_raw_generation_entrypoint = ["pub fn commit_", "generation("].concat();
        assert!(!owner_source.contains(&forbidden_raw_generation_entrypoint));
        assert!(repository_source
            .contains("pub(crate) fn validated(\n        stage_input: OutcomeStageInputPreimage,"));
        assert!(!repository_source
            .contains("pub fn validated(\n        stage_input: OutcomeStageInputPreimage,"));
        assert!(outcome_source.contains(
            "pub struct PreparedOutcomeStage {\n    stage_input: OutcomeStageInputPreimage,"
        ));
        assert!(!outcome_source.contains("pub fn into_stage_input"));
        for raw_constructor in ["pub fn try_from_prepared(", "pub fn validated("] {
            assert!(
                !repository_source.contains(raw_constructor),
                "public raw repository constructor remains: {raw_constructor}"
            );
        }
    }
}
