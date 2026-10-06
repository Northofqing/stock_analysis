//! Durable unapproved historical funding material in the Task8 local outbox.
//! No timestamp observation, funds approval or DatabaseConnectionAuthority issuer.
use super::paper_funding_review_v1::{read_stored_funding_review, FundingReviewErrorV1,
    MaterialReviewSource, StoredFundingReviewV1, MATERIAL_REVIEW_LIMIT};
use crate::evidence_retention::{draft_from_claims, DraftClaimsRef, OwnerDomain, TrustState,
    UnverifiedEvidencePackageDraft, UtcInstantClaim, ValueError};
use crate::evidence_retention::outbox_v1::{EnqueueOutcome, HeldOutbox, LocalDisposition,
    OutboxFault, PendingOutbox, RecoveredPresence, RecoveredUnverified, RecoveryOutcome,
    StoredUnverified, UnverifiedOutbox, MaterialReadObservation, MaterialReadOutcome};

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
    package_id: String,
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
            Ok(draft) => {
                self.draft = Some(draft);
                self.package_id = self.draft.as_ref().unwrap().id().to_owned();
            }
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
    let mut frame = Frame { source, package_id: String::new(), draft: None, returned: None, first: None };
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
    pub(crate) fn package_id(&self) -> &str { &self.frame.package_id }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FundingMaterialReadFault {
    Input,
    Outbox(OutboxFault),
    Envelope,
    Review(FundingReviewErrorV1),
    Identity,
}

enum ReadReturn {
    Open(UnverifiedOutbox),
    Held(HeldOutbox),
    Observed(MaterialReadObservation),
}
struct ReadFrame {
    package_id: String,
    review_id: String,
    returned: Option<ReadReturn>,
    body: Option<Vec<u8>>,
    source: Option<MaterialReviewSource>,
    first: Option<FundingMaterialReadFault>,
}
#[must_use = "ordinary historical funding observation, never funds approval"]
pub(crate) struct FundingMaterialRead { frame: ReadFrame }
#[must_use = "Held retains query, original material and genuine failure owner"]
pub(crate) struct HeldFundingMaterialRead { frame: ReadFrame }

fn material_id(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|value| value.len() == 64
        && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

// This borrows a package already checked by the strict native outbox codec.
#[derive(serde::Deserialize)]
struct FundingBodyLoan<'a> {
    owner_domain: OwnerDomain,
    owner_schema_claim: &'a str,
    logical_slot_claim: &'a str,
    business_day_claim: &'a str,
    window_start_claim: UtcInstantClaim,
    window_end_exclusive_claim: UtcInstantClaim,
    claimed_record_count: Option<u64>,
    #[serde(borrow)]
    source_chain_before_claim: Option<&'a str>,
    #[serde(borrow)]
    source_chain_after_claim: Option<&'a str>,
    #[serde(borrow)]
    artifact_sha256_claim: Option<&'a str>,
    #[serde(borrow)]
    activation_id_claim: Option<&'a str>,
    body_encoding: &'a str,
    body_length: u64,
    body_hex: &'a str,
}
impl ReadFrame {
    fn read(&mut self) -> Result<(), FundingMaterialReadFault> {
        use FundingMaterialReadFault as E;
        if !material_id(&self.package_id, "retention-package-draft-v1:")
            || !material_id(&self.review_id, "paper-funding-review-v1:") {
            return Err(E::Input);
        }
        let Some(ReadReturn::Open(outbox)) = self.returned.take() else {
            unreachable!("actual input owner")
        };
        let result = outbox.read_material(self.package_id.clone()); // Fixed bounded query.
        self.returned = Some(match result {
            MaterialReadOutcome::Observed(value) => ReadReturn::Observed(value),
            MaterialReadOutcome::Held(value) => ReadReturn::Held(value),
        }); // Retain the genuine whole native return before any body decoding.
        let observed = match self.returned.as_ref().unwrap() {
            ReadReturn::Observed(value) => value,
            ReadReturn::Held(value) => return Err(E::Outbox(value.first_fault())),
            _ => unreachable!("complete actual return"),
        };
        let Some(package) = observed.material() else { return Ok(()); };
        let loan: FundingBodyLoan<'_> =
            serde_json::from_slice(package.as_canonical_bytes()).map_err(|_| E::Envelope)?;
        if loan.owner_domain != OwnerDomain::PaperLedger
            || loan.owner_schema_claim != MATERIAL_SCHEMA
            || loan.business_day_claim != UNKNOWN_TIME_DAY
            || loan.window_start_claim != (UtcInstantClaim { unix_seconds: 0, nanosecond: 0 })
            || loan.window_end_exclusive_claim != (UtcInstantClaim { unix_seconds: 1, nanosecond: 0 })
            || loan.claimed_record_count != Some(1)
            || loan.source_chain_before_claim.is_some()
            || loan.source_chain_after_claim.is_some()
            || loan.artifact_sha256_claim.is_some()
            || loan.activation_id_claim.is_some()
            || loan.body_encoding != "hex"
            || loan.body_length > MATERIAL_REVIEW_LIMIT as u64
            || loan.body_hex.len() != loan.body_length as usize * 2 {
            return Err(E::Envelope);
        }
        // The native read/rollback/close has completed. This separately bounded
        // historical decoder never resets a failed native or review Work.
        self.body = Some(vec![0; loan.body_length as usize]);
        hex::decode_to_slice(loan.body_hex, self.body.as_mut().unwrap()).map_err(|_| E::Envelope)?;
        let original = read_stored_funding_review(self.body.as_ref().unwrap()).map_err(E::Review)?;
        self.source = Some(MaterialReviewSource::new(original)); // Move actual whole return first.
        let source = self.source.as_mut().unwrap();
        if source.review_id() != self.review_id { return Err(E::Identity); }
        source.prepare().map_err(E::Review)?;
        if source.slot() != Some(loan.logical_slot_claim) { return Err(E::Envelope); }
        Ok(())
    }
}

/// The two saved content IDs locate the original NotIssued material. Neither
/// arithmetic consistency nor absence can issue spendable cash or an intent.
pub(crate) fn read_funding_material(
    outbox: UnverifiedOutbox,
    package_id: String,
    review_id: String,
) -> Result<FundingMaterialRead, HeldFundingMaterialRead> {
    let mut frame = ReadFrame { package_id, review_id, returned: Some(ReadReturn::Open(outbox)),
        body: None, source: None, first: None };
    match frame.read() {
        Ok(()) => Ok(FundingMaterialRead { frame }),
        Err(error) => {
            frame.first = Some(error);
            Err(HeldFundingMaterialRead { frame })
        }
    }
}
impl FundingMaterialRead {
    pub(crate) fn original(&self) -> Option<&StoredFundingReviewV1> {
        self.frame.source.as_ref().map(MaterialReviewSource::original)
    }
    pub(crate) fn observed_generation(&self) -> i64 {
        match self.frame.returned.as_ref() {
            Some(ReadReturn::Observed(value)) => value.observed_generation(),
            _ => unreachable!("actual completed read"),
        }
    }
    pub(crate) fn other_family_materials(&self) -> usize {
        match self.frame.returned.as_ref() {
            Some(ReadReturn::Observed(value)) => value.other_slot_materials(),
            _ => unreachable!("actual completed read"),
        }
    }
    pub(crate) fn trust(&self) -> TrustState { TrustState::Unverified }
}
impl HeldFundingMaterialRead {
    pub(crate) fn first_fault(&self) -> FundingMaterialReadFault { self.frame.first.unwrap() }
    pub(crate) fn package_id(&self) -> &str { &self.frame.package_id }
    pub(crate) fn review_id(&self) -> &str { &self.frame.review_id }
    pub(crate) fn drain_resources_once(mut self) -> Self {
        self.frame.returned = match self.frame.returned.take() {
            Some(ReadReturn::Open(value)) => match value.close() {
                Ok(()) => None,
                Err(value) => Some(ReadReturn::Held(value)),
            },
            Some(ReadReturn::Held(value)) => Some(ReadReturn::Held(value.drain_resources_once())),
            other => other,
        };
        self
    }
}
#[cfg(test)]
impl HeldFundingMaterialRead {
    pub(super) fn test_outbox(&self) -> Option<&HeldOutbox> {
        match self.frame.returned.as_ref() { Some(ReadReturn::Held(value)) => Some(value), _ => None }
    }
    pub(super) fn test_material(&self) -> Option<&UnverifiedEvidencePackageDraft> {
        match self.frame.returned.as_ref() { Some(ReadReturn::Observed(value)) => value.material(), _ => None }
    }
    pub(super) fn test_original(&self) -> Option<&StoredFundingReviewV1> {
        self.frame.source.as_ref().map(MaterialReviewSource::original)
    }
    pub(super) fn test_body(&self) -> Option<&[u8]> { self.frame.body.as_deref() }
    pub(super) fn test_finalize_then_drain(mut self) -> Self {
        let Some(ReadReturn::Held(value)) = self.frame.returned.take() else { panic!("actual Held VM"); };
        self.frame.returned = Some(ReadReturn::Held(value.test_finalize_then_drain()));
        self
    }
}
