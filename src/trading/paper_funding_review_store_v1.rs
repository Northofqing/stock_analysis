//! Durable unapproved historical funding material in the Task8 local outbox.
//! No timestamp observation, funds approval or DatabaseConnectionAuthority issuer.
use super::paper_funding_review_v1::{FundingReviewErrorV1, MaterialReviewSource, StoredFundingReviewV1};
use crate::evidence_retention::{draft_from_claims, DraftClaimsRef, OwnerDomain, TrustState,
    UnverifiedEvidencePackageDraft, UtcInstantClaim, ValueError};
use crate::evidence_retention::outbox_v1::{EnqueueOutcome, HeldOutbox, LocalDisposition,
    OutboxFault, PendingOutbox, RecoveredPresence, RecoveredUnverified, RecoveryOutcome,
    StoredUnverified, UnverifiedOutbox};

const MATERIAL_SCHEMA: &str = "paper-funding-review-material-v1";
// These are fixed no-time grouping claims, never observed/valid/approval dates.
const UNKNOWN_TIME_DAY: &str = "1970-01-01";
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FundingMaterialFault { Review(FundingReviewErrorV1), Draft(ValueError), Outbox(OutboxFault), Phase }

enum OutboxReturn {
    Open(UnverifiedOutbox), Held(HeldOutbox), Pending(PendingOutbox),
    Stored(StoredUnverified), Recovered(RecoveredUnverified),
}
struct Frame {
    source: MaterialReviewSource,
    draft: Option<UnverifiedEvidencePackageDraft>,
    returned: Option<OutboxReturn>,
    first: Option<FundingMaterialFault>,
}
impl Frame {
    fn fault(&mut self, fault: FundingMaterialFault) { if self.first.is_none() { self.first = Some(fault); } }
    fn prepare(&mut self) {
        if let Err(error) = self.source.prepare() { self.fault(FundingMaterialFault::Review(error)); return; }
        let claims = DraftClaimsRef {
            owner_domain: OwnerDomain::PaperLedger, owner_schema_claim: MATERIAL_SCHEMA,
            logical_slot_claim: self.source.slot().unwrap(), business_day_claim: UNKNOWN_TIME_DAY,
            window_start_claim: UtcInstantClaim { unix_seconds: 0, nanosecond: 0 },
            window_end_exclusive_claim: UtcInstantClaim { unix_seconds: 1, nanosecond: 0 },
            claimed_record_count: Some(1), source_chain_before_claim: None, source_chain_after_claim: None,
            artifact_sha256_claim: None, activation_id_claim: None,
        };
        match draft_from_claims(claims, self.source.canonical()) {
            Ok(draft) => self.draft = Some(draft),
            Err(error) => self.fault(FundingMaterialFault::Draft(error)),
        }
    }
    fn classify(self) -> FundingMaterialOutcome {
        // The actual complete callee return has already been moved into Frame.
        match self.returned.as_ref() {
            Some(OutboxReturn::Stored(_)) => FundingMaterialOutcome::Stored(StoredFundingMaterial { frame: self }),
            Some(OutboxReturn::Recovered(_)) => FundingMaterialOutcome::Recovered(RecoveredFundingMaterial { frame: self }),
            Some(OutboxReturn::Pending(_)) => FundingMaterialOutcome::Pending(PendingFundingMaterial { frame: self }),
            _ => FundingMaterialOutcome::Held(HeldFundingMaterial { frame: self }),
        }
    }
    fn retain_enqueue(&mut self, result: EnqueueOutcome) {
        self.returned = Some(match result {
            EnqueueOutcome::Stored(value) => OutboxReturn::Stored(value),
            EnqueueOutcome::Held(value) => OutboxReturn::Held(value),
            EnqueueOutcome::Pending(value) => OutboxReturn::Pending(value),
        });
        self.retain_fault();
    }
    fn retain_recovery(&mut self, result: RecoveryOutcome) {
        self.returned = Some(match result {
            RecoveryOutcome::Observed(value) => OutboxReturn::Recovered(value),
            RecoveryOutcome::Held(value) => OutboxReturn::Held(value),
            RecoveryOutcome::Pending(value) => OutboxReturn::Pending(value),
        });
        self.retain_fault();
    }
    fn retain_fault(&mut self) {
        let fault = match self.returned.as_ref() {
            Some(OutboxReturn::Held(value)) => Some(value.first_fault()),
            Some(OutboxReturn::Pending(value)) => Some(value.first_fault()),
            Some(OutboxReturn::Recovered(value)) => Some(value.original_fault),
            _ => None,
        };
        if let Some(fault) = fault { self.fault(FundingMaterialFault::Outbox(fault)); }
    }
}

#[must_use = "this once-move command owns the original historical review"]
pub(crate) struct FundingMaterialCommand { frame: Frame }
#[must_use = "Held keeps the original review and genuine outbox failure owner"]
pub(crate) struct HeldFundingMaterial { frame: Frame }
#[must_use = "Pending requires actual membership/attempt observation, not a resend"]
pub(crate) struct PendingFundingMaterial { frame: Frame }
pub(crate) struct StoredFundingMaterial { frame: Frame }
pub(crate) struct RecoveredFundingMaterial { frame: Frame }
#[must_use]
pub(crate) enum FundingMaterialOutcome {
    Stored(StoredFundingMaterial), Recovered(RecoveredFundingMaterial),
    Held(HeldFundingMaterial), Pending(PendingFundingMaterial),
}

pub(crate) fn prepare_review_material(original: StoredFundingReviewV1) -> Result<FundingMaterialCommand, HeldFundingMaterial> {
    // No parsing/copying/admission precedes retaining this whole return.
    prepare_source(MaterialReviewSource::new(original))
}
fn prepare_source(source: MaterialReviewSource) -> Result<FundingMaterialCommand, HeldFundingMaterial> {
    let mut frame = Frame { source, draft: None, returned: None, first: None };
    frame.prepare();
    if frame.first.is_some() { Err(HeldFundingMaterial { frame }) } else { Ok(FundingMaterialCommand { frame }) }
}
impl FundingMaterialCommand {
    pub(crate) fn canonical_review(&self) -> &[u8] { self.frame.source.canonical() }
    pub(crate) fn review_id(&self) -> &str { self.frame.source.review_id() }
    pub(crate) fn proposal_id(&self) -> &str { self.frame.source.proposal_id() }
    pub(crate) fn slot(&self) -> &str { self.frame.source.slot().unwrap() }
    pub(crate) fn slot_tuple(&self) -> &[u8] { self.frame.source.tuple().unwrap() }
    pub(crate) fn trust(&self) -> TrustState { TrustState::Unverified }
    pub(crate) fn persist(mut self, outbox: UnverifiedOutbox, expected_generation: i64) -> FundingMaterialOutcome {
        self.frame.returned = Some(OutboxReturn::Open(outbox));
        if self.frame.first.is_some() || self.frame.draft.is_none() {
            self.frame.fault(FundingMaterialFault::Phase); return self.frame.classify();
        }
        let Some(OutboxReturn::Open(outbox)) = self.frame.returned.take() else { unreachable!("owned actual input") };
        let draft = self.frame.draft.take().unwrap();
        let result = outbox.enqueue(draft, expected_generation);
        self.frame.retain_enqueue(result);
        self.frame.classify()
    }
    pub(crate) fn observe_previous(mut self, outbox: UnverifiedOutbox, generation: i64) -> FundingMaterialOutcome {
        self.frame.returned = Some(OutboxReturn::Open(outbox));
        if self.frame.first.is_some() || self.frame.draft.is_none() {
            self.frame.fault(FundingMaterialFault::Phase); return self.frame.classify();
        }
        let Some(OutboxReturn::Open(outbox)) = self.frame.returned.take() else { unreachable!("owned actual input") };
        let draft = self.frame.draft.take().unwrap();
        let result = outbox.observe_previous(draft, generation);
        self.frame.retain_recovery(result);
        self.frame.classify()
    }
}
impl HeldFundingMaterial {
    pub(crate) fn first_fault(&self) -> FundingMaterialFault { self.frame.first.unwrap() }
    pub(crate) fn canonical_review(&self) -> &[u8] { self.frame.source.canonical() }
    pub(crate) fn review_id(&self) -> &str { self.frame.source.review_id() }
    pub(crate) fn proposal_id(&self) -> &str { self.frame.source.proposal_id() }
    pub(crate) fn slot_tuple(&self) -> Option<&[u8]> { self.frame.source.tuple() }
    pub(crate) fn trust(&self) -> TrustState { TrustState::Unverified }
    pub(crate) fn drain_resources_once(mut self) -> Self {
        let result = match self.frame.returned.take() {
            Some(OutboxReturn::Open(value)) => match value.close() {
                Ok(()) => None, Err(value) => Some(OutboxReturn::Held(value)),
            },
            Some(OutboxReturn::Held(value)) => Some(OutboxReturn::Held(value.drain_resources_once())),
            other => other,
        };
        self.frame.returned = result; self.frame.retain_fault(); self
    }
}
impl PendingFundingMaterial {
    pub(crate) fn first_fault(&self) -> FundingMaterialFault { self.frame.first.unwrap() }
    pub(crate) fn canonical_review(&self) -> &[u8] { self.frame.source.canonical() }
    pub(crate) fn trust(&self) -> TrustState { TrustState::Unverified }
    pub(crate) fn observe(mut self) -> FundingMaterialOutcome {
        let Some(OutboxReturn::Pending(pending)) = self.frame.returned.take() else { unreachable!("genuine Pending") };
        let result = pending.observe(); self.frame.retain_recovery(result); self.frame.classify()
    }
}
impl StoredFundingMaterial {
    pub(crate) fn canonical_review(&self) -> &[u8] { self.frame.source.canonical() }
    pub(crate) fn review_id(&self) -> &str { self.frame.source.review_id() }
    pub(crate) fn proposal_id(&self) -> &str { self.frame.source.proposal_id() }
    pub(crate) fn slot_tuple(&self) -> &[u8] { self.frame.source.tuple().unwrap() }
    pub(crate) fn generation(&self) -> i64 {
        match self.frame.returned.as_ref().unwrap() { OutboxReturn::Stored(value) => value.generation, _ => unreachable!("genuine Stored") }
    }
    pub(crate) fn disposition(&self) -> LocalDisposition {
        match self.frame.returned.as_ref().unwrap() { OutboxReturn::Stored(value) => value.disposition, _ => unreachable!("genuine Stored") }
    }
    pub(crate) fn trust(&self) -> TrustState { TrustState::Unverified }
}
impl RecoveredFundingMaterial {
    pub(crate) fn canonical_review(&self) -> &[u8] { self.frame.source.canonical() }
    pub(crate) fn original_fault(&self) -> FundingMaterialFault { self.frame.first.unwrap() }
    pub(crate) fn presence(&self) -> RecoveredPresence {
        match self.frame.returned.as_ref().unwrap() { OutboxReturn::Recovered(value) => value.presence, _ => unreachable!("genuine recovered material") }
    }
    pub(crate) fn trust(&self) -> TrustState { TrustState::Unverified }
}

#[cfg(test)]
pub(super) fn prepare_exhausted_source(source: MaterialReviewSource) -> Result<FundingMaterialCommand,HeldFundingMaterial> {
    // The caller spent the same review Work before admission; never a reset.
    prepare_source(source)
}
#[cfg(test)]
impl FundingMaterialCommand {
    pub(super) fn test_draft(&self) -> &UnverifiedEvidencePackageDraft { self.frame.draft.as_ref().unwrap() }
}
#[cfg(test)]
impl HeldFundingMaterial {
    pub(super) fn test_outbox_held(&self) -> Option<&HeldOutbox> {
        match self.frame.returned.as_ref() { Some(OutboxReturn::Held(value)) => Some(value), _ => None }
    }
    pub(super) fn test_finalize_then_drain(mut self) -> Self {
        let Some(OutboxReturn::Held(value)) = self.frame.returned.take() else { panic!("real held VM") };
        self.frame.returned = Some(OutboxReturn::Held(value.test_finalize_then_drain())); self
    }
}
#[cfg(test)]
impl PendingFundingMaterial {
    pub(super) fn test_actual_owner_retained(&self) -> bool {
        match self.frame.returned.as_ref() {
            Some(OutboxReturn::Pending(value)) => value.test_connection_and_command_retained(), _ => false,
        }
    }
    pub(super) fn test_exhaust_same_work(&mut self) {
        let Some(OutboxReturn::Pending(value)) = self.frame.returned.as_mut() else { panic!("real Pending") };
        assert!(value.test_exhaust_same_work().is_err());
    }
}
