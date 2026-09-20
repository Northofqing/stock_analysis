//! Internal current-record lookup. A returned value is still unauthenticated candidate material.
//! The two databases are checked sequentially; this is not a cross-database atomic snapshot and
//! does not promise that either record remains current after the function returns.

#![cfg_attr(not(test), allow(dead_code))]

use std::fmt;
use std::path::Path;

use crate::monitor::push_job::{MachineCatalog, Namespace, Sha256Digest, UnitId};

use super::activation_readiness::{
    reread_activation_deployment_set, ActivationDeploymentSetError, ActivationDeploymentSetRequest,
};
use super::readiness_store::{ReadinessRecordStore, ReadinessStoreError, StoredReadinessRecord};

pub(super) struct CurrentV3CandidateRequest<'a> {
    readiness_database: &'a Path,
    activation_database: &'a Path,
    namespace: &'a Namespace,
    snapshot_id: &'a Sha256Digest,
    selected_unit: &'a UnitId,
    activation: ActivationDeploymentSetRequest,
}

impl<'a> CurrentV3CandidateRequest<'a> {
    pub(super) fn new(
        readiness_database: &'a Path,
        activation_database: &'a Path,
        namespace: &'a Namespace,
        snapshot_id: &'a Sha256Digest,
        selected_unit: &'a UnitId,
        activation: ActivationDeploymentSetRequest,
    ) -> Self {
        Self {
            readiness_database,
            activation_database,
            namespace,
            snapshot_id,
            selected_unit,
            activation,
        }
    }
}

impl fmt::Debug for CurrentV3CandidateRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CurrentV3CandidateRequest")
            .field("namespace", self.namespace)
            .field("snapshot_id", self.snapshot_id)
            .field("selected_unit", self.selected_unit)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum ReadinessQueryError {
    #[error("bundled readiness catalog is unavailable")]
    CatalogUnavailable,
    #[error(transparent)]
    Store(#[from] ReadinessStoreError),
    #[error("only a current v3 readiness record can be queried")]
    UnsupportedSnapshotVersion,
    #[error(transparent)]
    Activation(#[from] ActivationDeploymentSetError),
    #[error("readiness record changed while activation state was checked")]
    ReadinessChanged,
}

pub(super) fn load_current_v3_candidate(
    request: CurrentV3CandidateRequest<'_>,
) -> Result<StoredReadinessRecord, ReadinessQueryError> {
    load_current_v3_candidate_inner(request, || {})
}

#[cfg(test)]
pub(super) fn load_current_v3_candidate_with_checkpoint(
    request: CurrentV3CandidateRequest<'_>,
    checkpoint: impl FnOnce(),
) -> Result<StoredReadinessRecord, ReadinessQueryError> {
    load_current_v3_candidate_inner(request, checkpoint)
}

fn load_current_v3_candidate_inner(
    request: CurrentV3CandidateRequest<'_>,
    checkpoint: impl FnOnce(),
) -> Result<StoredReadinessRecord, ReadinessQueryError> {
    let catalog = MachineCatalog::bundled().map_err(|_| ReadinessQueryError::CatalogUnavailable)?;
    let store = ReadinessRecordStore::at(request.readiness_database, request.namespace, &catalog);
    let first = store.load_current(request.snapshot_id)?;
    let deployment_set = first
        .candidate()
        .snapshot()
        .deployment_set()
        .ok_or(ReadinessQueryError::UnsupportedSnapshotVersion)?;

    reread_activation_deployment_set(
        deployment_set,
        request.activation_database,
        request.selected_unit,
        request.activation,
    )?;
    checkpoint();

    let second = store.load_current(request.snapshot_id)?;
    if second != first {
        return Err(ReadinessQueryError::ReadinessChanged);
    }
    Ok(first)
}
