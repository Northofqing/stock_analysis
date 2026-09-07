//! Derive admission claims from executed history, never execution permission.
//!
//! Authentication must still resolve the referenced initialization/activation approvals and every
//! rollback approval, as well as the current deployment, owner and execution fence. In particular,
//! an initial Disabled row is not evidence that a legacy actor was ever approved to run.

#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::fmt;

use crate::monitor::push_job::{Sha256Digest, UnitId};

use super::activation::{DesiredActivationState, PromotionAction};
use super::activation_facts::{ActivationReconciliation, UnitActivationFacts};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum NewWorkAdmissionClaim {
    Closed,
    /// Resolve this exact initialization's authenticated baseline; it can also be unowned/closed.
    InitializationBaseline {
        manifest_sha256: Sha256Digest,
    },
    /// Resolve the approved scope of this exact activation; the state name alone grants nothing.
    ActivationApproval {
        manifest_sha256: Sha256Digest,
    },
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct OwnerAdmissionProjection {
    pub(super) unit_id: UnitId,
    pub(super) generation: u64,
    pub(super) manifest_sha256: Sha256Digest,
    pub(super) physical_owner: String,
    pub(super) state: DesiredActivationState,
    pub(super) new_work: NewWorkAdmissionClaim,
    /// Oldest to newest on the selected target path. Each requires its own scope-bound approval;
    /// the current request still needs fresh authorization, never a reused historical token.
    pub(super) rollback_approval_manifests: Vec<Sha256Digest>,
}

impl fmt::Debug for OwnerAdmissionProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerAdmissionProjection")
            .field("unit_id", &self.unit_id)
            .field("generation", &self.generation)
            .field("manifest_sha256", &self.manifest_sha256)
            .field("state", &self.state)
            .field("new_work", &self.new_work)
            .field(
                "rollback_approval_count",
                &self.rollback_approval_manifests.len(),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum AdmissionProjectionError {
    #[error("activation history has not been registered")]
    Unregistered,
    #[error("activation history requires reconciliation before admission projection")]
    ReconciliationRequired,
    #[error("activation admission history is inconsistent")]
    InvalidHistory,
    #[error("activation action changed an owner it was required to preserve")]
    OwnerNotPreserved,
}

/// Consume T1's integrity-checked facts and add the B-ruling's owner/admission semantics.
///
/// Pending desired state is deliberately not projected as executed, nor is the older executed
/// state returned as usable during unresolved coordination. No source, owner or approval is
/// authenticated here; even a non-Closed result is only a reference to evidence still required.
pub(super) fn project_owner_admission(
    facts: &UnitActivationFacts,
) -> Result<OwnerAdmissionProjection, AdmissionProjectionError> {
    let generation = match facts.reconciliation() {
        ActivationReconciliation::Unregistered => {
            return Err(AdmissionProjectionError::Unregistered);
        }
        ActivationReconciliation::Pending { .. } => {
            return Err(AdmissionProjectionError::ReconciliationRequired);
        }
        ActivationReconciliation::CaughtUp { generation } => generation,
    };
    let manifests = facts.manifests();
    let journal = facts.journal();
    let current = manifests
        .last()
        .ok_or(AdmissionProjectionError::InvalidHistory)?;
    if manifests.len() != journal.len() || current.generation() != generation {
        return Err(AdmissionProjectionError::InvalidHistory);
    }

    let mut indices = BTreeMap::new();
    for (index, (manifest, entry)) in manifests.iter().zip(journal).enumerate() {
        if manifest.unit_id() != facts.unit_id()
            || entry.to_manifest_sha256() != manifest.manifest_sha256()
            || indices.insert(manifest.manifest_sha256(), index).is_some()
        {
            return Err(AdmissionProjectionError::InvalidHistory);
        }
        if matches!(
            entry.action(),
            PromotionAction::EnterShadow | PromotionAction::Drain | PromotionAction::Disable
        ) {
            let previous = index
                .checked_sub(1)
                .and_then(|before| manifests.get(before))
                .ok_or(AdmissionProjectionError::InvalidHistory)?;
            if manifest.physical_owner() != previous.physical_owner() {
                return Err(AdmissionProjectionError::OwnerNotPreserved);
            }
        }
    }

    let mut index = manifests.len() - 1;
    let mut rollback_approval_manifests = Vec::new();
    let new_work = loop {
        let manifest = &manifests[index];
        match journal[index].action() {
            PromotionAction::Initialize => {
                break NewWorkAdmissionClaim::InitializationBaseline {
                    manifest_sha256: manifest.manifest_sha256().clone(),
                };
            }
            PromotionAction::Activate => {
                break NewWorkAdmissionClaim::ActivationApproval {
                    manifest_sha256: manifest.manifest_sha256().clone(),
                };
            }
            PromotionAction::Drain | PromotionAction::Disable => {
                break NewWorkAdmissionClaim::Closed;
            }
            PromotionAction::EnterShadow => {
                index = index
                    .checked_sub(1)
                    .ok_or(AdmissionProjectionError::InvalidHistory)?;
            }
            PromotionAction::Rollback => {
                let target_hash = manifest
                    .rollback_target_sha256()
                    .ok_or(AdmissionProjectionError::InvalidHistory)?;
                let target_index = indices
                    .get(target_hash)
                    .copied()
                    .filter(|target| *target < index)
                    .ok_or(AdmissionProjectionError::InvalidHistory)?;
                let target = &manifests[target_index];
                if target.physical_owner() != manifest.physical_owner()
                    || target.desired_state() != manifest.desired_state()
                {
                    return Err(AdmissionProjectionError::InvalidHistory);
                }
                rollback_approval_manifests.push(manifest.manifest_sha256().clone());
                index = target_index;
            }
        }
    };
    rollback_approval_manifests.reverse();

    Ok(OwnerAdmissionProjection {
        unit_id: facts.unit_id().clone(),
        generation,
        manifest_sha256: current.manifest_sha256().clone(),
        physical_owner: current.physical_owner().to_owned(),
        state: current.desired_state(),
        new_work,
        rollback_approval_manifests,
    })
}
