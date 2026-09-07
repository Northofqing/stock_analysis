//! Verified persisted occurrence facts; source registration and execution authority are separate.

#![cfg_attr(not(test), allow(dead_code))]

use std::path::Path;

use crate::monitor::push_job::{
    derive_occurrence_id, BusinessDate, IntentId, MachineCatalog, Namespace, OccurrenceFamily,
    OccurrenceId, OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, Sha256Digest,
    SourceContractId, UnitId,
};
use crate::push_foundation::migration::attest_bundled_connection;
use crate::push_foundation::readiness_store_schema::{
    with_rollback_read_only, ReadinessSchemaError,
};

use super::{parse_namespace, query_intent, query_transition_chain, InitialDecisionKind};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredOccurrenceExpectation {
    pub(crate) intent_id: IntentId,
    pub(crate) namespace: Namespace,
    pub(crate) business_date: BusinessDate,
    pub(crate) unit_id: UnitId,
    pub(crate) producer_id: ProducerId,
    pub(crate) occurrence_id: OccurrenceId,
    pub(crate) source_contract_id: SourceContractId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum StoredOccurrenceReadError {
    #[error("persisted occurrence source cannot be read without side effects")]
    ReadOnlySourceRejected,
    #[error("persisted occurrence source schema is not the bundled schema")]
    SchemaRejected,
    #[error("persisted occurrence does not exist")]
    Missing,
    #[error("persisted occurrence facts failed verification")]
    InvalidFacts,
    #[error("persisted occurrence identity does not match the expected scope")]
    IdentityMismatch,
    #[error("persisted occurrence producer binding is not in the bundled catalog")]
    CatalogRejected,
}

impl From<ReadinessSchemaError> for StoredOccurrenceReadError {
    fn from(_: ReadinessSchemaError) -> Self {
        Self::ReadOnlySourceRejected
    }
}

/// Committed facts in the explicitly selected database, plus a bundled catalog relationship.
///
/// This does not certify that the path is authorized by the current deployment, nor a source
/// contract version/build/generation. Intent rows do not record a producer: the producer here
/// is the requested catalog relationship, not proof of the producer that originally wrote it.
/// No payload, subject, lease owner, or path is retained in this result or its Debug output.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct VerifiedStoredOccurrence {
    binding: StoredOccurrenceExpectation,
    catalog_sha256: Sha256Digest,
    decision_kind: InitialDecisionKind,
    version: u64,
    transition_head_sha256: Option<Sha256Digest>,
    source_contract_sha256: Sha256Digest,
}

impl VerifiedStoredOccurrence {
    pub(crate) fn binding(&self) -> &StoredOccurrenceExpectation {
        &self.binding
    }
    pub(crate) fn catalog_sha256(&self) -> &Sha256Digest {
        &self.catalog_sha256
    }
    pub(crate) fn decision_kind(&self) -> InitialDecisionKind {
        self.decision_kind
    }
    pub(crate) fn version(&self) -> u64 {
        self.version
    }
    pub(crate) fn transition_head_sha256(&self) -> Option<&Sha256Digest> {
        self.transition_head_sha256.as_ref()
    }
    pub(crate) fn source_contract_sha256(&self) -> &Sha256Digest {
        &self.source_contract_sha256
    }
}

/// Validate schema, the actual intent, its full transition chain, and the catalog join in one
/// owned read transaction. It is committed and the connection closed before returning facts.
/// A mutable/WAL or unregistered deployment source is never implicitly upgraded to authority.
pub(crate) fn read_stored_occurrence(
    database: &Path,
    expected: &StoredOccurrenceExpectation,
) -> Result<VerifiedStoredOccurrence, StoredOccurrenceReadError> {
    let catalog =
        MachineCatalog::bundled().map_err(|_| StoredOccurrenceReadError::CatalogRejected)?;
    let producer = catalog
        .producer(&expected.producer_id)
        .ok_or(StoredOccurrenceReadError::CatalogRejected)?;
    if producer.unit_id() != &expected.unit_id {
        return Err(StoredOccurrenceReadError::IdentityMismatch);
    }
    with_rollback_read_only(database, |transaction| {
        attest_bundled_connection(transaction)
            .map_err(|_| StoredOccurrenceReadError::SchemaRejected)?;
        let snapshot = query_intent(transaction, expected.intent_id.as_str())
            .map_err(|_| StoredOccurrenceReadError::InvalidFacts)?
            .ok_or(StoredOccurrenceReadError::Missing)?;
        let chain = query_transition_chain(transaction, &snapshot)
            .map_err(|_| StoredOccurrenceReadError::InvalidFacts)?;
        if snapshot.unit_id != producer.unit_id().as_str()
            || snapshot.occurrence_family != producer.occurrence_family().as_str()
            || snapshot.completion_owner != producer.completion_owner().as_str()
        {
            return Err(StoredOccurrenceReadError::IdentityMismatch);
        }
        let invalid = |_| StoredOccurrenceReadError::InvalidFacts;
        let business_date = BusinessDate::parse(&snapshot.business_date).map_err(invalid)?;
        let occurrence = OccurrenceIdentityMaterial::new(
            business_date.clone(),
            OccurrenceFamily::try_new(snapshot.occurrence_family.clone()).map_err(invalid)?,
            OccurrenceKey::try_new(snapshot.occurrence_key.clone()).map_err(invalid)?,
        );
        let actual = StoredOccurrenceExpectation {
            intent_id: IntentId::from_digest(
                &Sha256Digest::parse("intent_id", &snapshot.intent_id).map_err(invalid)?,
            ),
            namespace: parse_namespace(&snapshot.namespace)
                .map_err(|_| StoredOccurrenceReadError::InvalidFacts)?,
            business_date,
            unit_id: UnitId::try_new(snapshot.unit_id.clone()).map_err(invalid)?,
            producer_id: producer.id().clone(),
            occurrence_id: derive_occurrence_id(&occurrence),
            source_contract_id: SourceContractId::try_new(snapshot.source_contract_id.clone())
                .map_err(invalid)?,
        };
        if &actual != expected {
            return Err(StoredOccurrenceReadError::IdentityMismatch);
        }
        Ok(VerifiedStoredOccurrence {
            binding: actual,
            catalog_sha256: catalog.catalog_sha256().clone(),
            decision_kind: snapshot.decision_kind,
            version: snapshot.version,
            transition_head_sha256: chain.last().map(|event| event.canonical_sha256.clone()),
            source_contract_sha256: snapshot.source_contract_sha256,
        })
    })
}
