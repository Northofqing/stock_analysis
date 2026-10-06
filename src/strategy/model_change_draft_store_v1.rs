//! Durable ordinary Draft material through the existing Unverified outbox.
//! This is neither a registration-time witness nor a governance/approval issuer.

use super::model_change_draft_v1::ModelChangeDraftV1;
use crate::evidence_retention::outbox_v1::{
    EnqueueOutcome, HeldOutbox, LocalDisposition, OutboxFault, PendingOutbox, RecoveredPresence,
    RecoveredUnverified, RecoveryOutcome, StoredUnverified, UnverifiedOutbox,
};
use crate::evidence_retention::{
    draft_from_claims, DraftClaimsRef, OwnerDomain, TrustState, UnverifiedEvidencePackageDraft,
    UtcInstantClaim, ValueError,
};
use sha2::{Digest, Sha256};

const MATERIAL_SCHEMA: &str = "model-change-draft-material-v1";
const FAMILY_DOMAIN: &[u8] = b"stock_analysis.model-change-draft-material-family/v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DraftMaterialFault {
    Envelope(ValueError),
    Outbox(OutboxFault),
}

enum OutboxReturn {
    Held(HeldOutbox),
    Pending(PendingOutbox),
    Stored(StoredUnverified),
    Recovered(RecoveredUnverified),
}

struct Frame {
    original: ModelChangeDraftV1,
    slot: String,
    package: Option<UnverifiedEvidencePackageDraft>,
    returned: Option<OutboxReturn>,
    first: Option<DraftMaterialFault>,
}

impl Frame {
    fn retain_fault(&mut self) {
        let error = match self.returned.as_ref() {
            Some(OutboxReturn::Held(v)) => Some(v.first_fault()),
            Some(OutboxReturn::Pending(v)) => Some(v.first_fault()),
            Some(OutboxReturn::Recovered(v)) => Some(v.original_fault),
            _ => None,
        };
        if self.first.is_none() {
            self.first = error.map(DraftMaterialFault::Outbox);
        }
    }

    fn retain_enqueue(mut self, result: EnqueueOutcome) -> DraftMaterialOutcome {
        self.returned = Some(match result {
            EnqueueOutcome::Stored(v) => OutboxReturn::Stored(v),
            EnqueueOutcome::Held(v) => OutboxReturn::Held(v),
            EnqueueOutcome::Pending(v) => OutboxReturn::Pending(v),
        });
        self.retain_fault();
        self.classify()
    }

    fn retain_recovery(mut self, result: RecoveryOutcome) -> DraftMaterialOutcome {
        self.returned = Some(match result {
            RecoveryOutcome::Observed(v) => OutboxReturn::Recovered(v),
            RecoveryOutcome::Held(v) => OutboxReturn::Held(v),
            RecoveryOutcome::Pending(v) => OutboxReturn::Pending(v),
        });
        self.retain_fault();
        self.classify()
    }

    fn classify(self) -> DraftMaterialOutcome {
        match self.returned.as_ref() {
            Some(OutboxReturn::Stored(_)) => {
                DraftMaterialOutcome::Stored(StoredDraftMaterial { frame: self })
            }
            Some(OutboxReturn::Recovered(_)) => {
                DraftMaterialOutcome::Recovered(RecoveredDraftMaterial { frame: self })
            }
            Some(OutboxReturn::Pending(_)) => {
                DraftMaterialOutcome::Pending(PendingDraftMaterial { frame: self })
            }
            _ => DraftMaterialOutcome::Held(HeldDraftMaterial { frame: self }),
        }
    }
}

/// Fixed six bounded tokens, each prefixed with its byte length in big endian.
/// Policy/model/config/code/rollback/window/time changes remain in this family
/// and conflict instead of silently replacing a predeclared proposal.
fn family_slot(original: &ModelChangeDraftV1) -> String {
    let r = original.request();
    let mut hash = Sha256::new();
    hash.update(FAMILY_DOMAIN);
    for s in [
        &r.champion.strategy_id,
        &r.champion.strategy_version,
        &r.challenger.strategy_id,
        &r.challenger.strategy_version,
        &r.champion_paper_book_id,
        &r.challenger_paper_book_id,
    ] {
        hash.update((s.len() as u64).to_be_bytes());
        hash.update(s.as_bytes());
    }
    format!(
        "model-change-draft-family-v1:sha256:{}",
        hex::encode(hash.finalize())
    )
}

#[must_use = "this consuming command owns the original immutable draft"]
pub(crate) struct DraftMaterialCommand {
    frame: Frame,
}
#[must_use = "Held retains the original draft, actual outbox resources and first fault"]
pub(crate) struct HeldDraftMaterial {
    frame: Frame,
}
#[must_use = "Pending is not a resend or governance approval permission"]
pub(crate) struct PendingDraftMaterial {
    frame: Frame,
}
pub(crate) struct StoredDraftMaterial {
    frame: Frame,
}
pub(crate) struct RecoveredDraftMaterial {
    frame: Frame,
}

#[must_use]
pub(crate) enum DraftMaterialOutcome {
    Stored(StoredDraftMaterial),
    Recovered(RecoveredDraftMaterial),
    Held(HeldDraftMaterial),
    Pending(PendingDraftMaterial),
}

pub(crate) fn prepare_draft_material(
    original: ModelChangeDraftV1,
) -> Result<DraftMaterialCommand, HeldDraftMaterial> {
    // Retain the complete input before creating any derived material.
    let mut frame = Frame {
        original,
        slot: String::new(),
        package: None,
        returned: None,
        first: None,
    };
    frame.slot = family_slot(&frame.original);
    let claims = DraftClaimsRef {
        owner_domain: OwnerDomain::Attribution,
        owner_schema_claim: MATERIAL_SCHEMA,
        logical_slot_claim: &frame.slot,
        // Fixed no-time grouping, never the caller's declared registration date.
        business_day_claim: "1970-01-01",
        window_start_claim: UtcInstantClaim {
            unix_seconds: 0,
            nanosecond: 0,
        },
        window_end_exclusive_claim: UtcInstantClaim {
            unix_seconds: 1,
            nanosecond: 0,
        },
        claimed_record_count: Some(1),
        source_chain_before_claim: None,
        source_chain_after_claim: None,
        artifact_sha256_claim: None,
        activation_id_claim: None,
    };
    match draft_from_claims(claims, frame.original.canonical_bytes()) {
        Ok(package) => {
            frame.package = Some(package);
            Ok(DraftMaterialCommand { frame })
        }
        Err(error) => {
            frame.first = Some(DraftMaterialFault::Envelope(error));
            Err(HeldDraftMaterial { frame })
        }
    }
}

impl DraftMaterialCommand {
    pub(crate) fn original(&self) -> &ModelChangeDraftV1 {
        &self.frame.original
    }
    pub(crate) fn family_slot(&self) -> &str {
        &self.frame.slot
    }

    pub(crate) fn persist(
        mut self,
        outbox: UnverifiedOutbox,
        expected_generation: i64,
    ) -> DraftMaterialOutcome {
        let package = self
            .frame
            .package
            .take()
            .expect("prepared consuming command");
        let result = outbox.enqueue(package, expected_generation);
        self.frame.retain_enqueue(result)
    }

    /// Observe the original saved attempt generation using the original bytes;
    /// no insert, updated proposal, missing-facts fill or approval is performed.
    pub(crate) fn observe_previous(
        mut self,
        outbox: UnverifiedOutbox,
        generation: i64,
    ) -> DraftMaterialOutcome {
        let package = self
            .frame
            .package
            .take()
            .expect("prepared consuming command");
        let result = outbox.observe_previous(package, generation);
        self.frame.retain_recovery(result)
    }
}

impl HeldDraftMaterial {
    pub(crate) fn original(&self) -> &ModelChangeDraftV1 {
        &self.frame.original
    }
    pub(crate) fn first_fault(&self) -> DraftMaterialFault {
        self.frame.first.expect("genuine Held")
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
    pub(crate) fn drain_resources_once(mut self) -> Self {
        self.frame.returned = match self.frame.returned.take() {
            Some(OutboxReturn::Held(value)) => {
                Some(OutboxReturn::Held(value.drain_resources_once()))
            }
            other => other,
        };
        self.frame.retain_fault();
        self
    }
}

impl PendingDraftMaterial {
    pub(crate) fn original(&self) -> &ModelChangeDraftV1 {
        &self.frame.original
    }
    pub(crate) fn first_fault(&self) -> DraftMaterialFault {
        self.frame.first.expect("genuine Pending")
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
    pub(crate) fn observe(mut self) -> DraftMaterialOutcome {
        let Some(OutboxReturn::Pending(pending)) = self.frame.returned.take() else {
            unreachable!("genuine Pending")
        };
        let result = pending.observe();
        self.frame.retain_recovery(result)
    }
}

impl StoredDraftMaterial {
    pub(crate) fn original(&self) -> &ModelChangeDraftV1 {
        &self.frame.original
    }
    pub(crate) fn family_slot(&self) -> &str {
        &self.frame.slot
    }
    pub(crate) fn generation(&self) -> i64 {
        match self.frame.returned.as_ref() {
            Some(OutboxReturn::Stored(v)) => v.generation,
            _ => unreachable!("genuine Stored"),
        }
    }
    /// Conflict means conflicting material was recorded, not that it replaced
    /// the original proposal or became eligible for review/promotion.
    pub(crate) fn disposition(&self) -> LocalDisposition {
        match self.frame.returned.as_ref() {
            Some(OutboxReturn::Stored(v)) => v.disposition,
            _ => unreachable!("genuine Stored"),
        }
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
}

impl RecoveredDraftMaterial {
    pub(crate) fn original(&self) -> &ModelChangeDraftV1 {
        &self.frame.original
    }
    pub(crate) fn presence(&self) -> RecoveredPresence {
        match self.frame.returned.as_ref() {
            Some(OutboxReturn::Recovered(v)) => v.presence,
            _ => unreachable!("genuine Recovered"),
        }
    }
    pub(crate) fn original_fault(&self) -> DraftMaterialFault {
        self.frame.first.expect("original observation fault")
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
}

#[cfg(test)]
impl DraftMaterialCommand {
    pub(super) fn test_package(&self) -> &UnverifiedEvidencePackageDraft {
        self.frame.package.as_ref().unwrap()
    }
}
#[cfg(test)]
impl HeldDraftMaterial {
    pub(super) fn test_outbox(&self) -> Option<&HeldOutbox> {
        match self.frame.returned.as_ref() {
            Some(OutboxReturn::Held(v)) => Some(v),
            _ => None,
        }
    }
    pub(super) fn test_finalize_then_drain(mut self) -> Self {
        let Some(OutboxReturn::Held(v)) = self.frame.returned.take() else {
            panic!("actual Held VM")
        };
        self.frame.returned = Some(OutboxReturn::Held(v.test_finalize_then_drain()));
        self.frame.retain_fault();
        self
    }
}
#[cfg(test)]
impl PendingDraftMaterial {
    pub(super) fn test_actual_owner_retained(&self) -> bool {
        match self.frame.returned.as_ref() {
            Some(OutboxReturn::Pending(v)) => v.test_connection_and_command_retained(),
            _ => false,
        }
    }
    pub(super) fn test_exhaust_same_work(&mut self) {
        let Some(OutboxReturn::Pending(v)) = self.frame.returned.as_mut() else {
            panic!("actual Pending")
        };
        assert!(v.test_exhaust_same_work().is_err());
    }
}
