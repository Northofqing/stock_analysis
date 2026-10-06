//! Durable ordinary Draft material through the existing Unverified outbox.
//! This is neither a registration-time witness nor a governance/approval issuer.

use super::model_change_draft_v1::{
    recover_model_change_draft_v1, ModelChangeDraftErrorV1, ModelChangeDraftV1,
};
use crate::evidence_retention::outbox_v1::{
    EnqueueOutcome, HeldOutbox, LocalDisposition, MaterialReadObservation, MaterialReadOutcome,
    OutboxFault, PendingOutbox, RecoveredPresence, RecoveredUnverified, RecoveryOutcome,
    StoredUnverified, UnverifiedOutbox,
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
    package_id: String,
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
        package_id: String::new(),
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
            frame.package_id = frame.package.as_ref().unwrap().id().to_owned();
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
    pub(crate) fn package_id(&self) -> &str {
        &self.frame.package_id
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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DraftMaterialReadFault {
    Input,
    Outbox(OutboxFault),
    Envelope,
    Draft(ModelChangeDraftErrorV1),
}

enum ReadReturn {
    Open(UnverifiedOutbox),
    Held(HeldOutbox),
    Observed(MaterialReadObservation),
}

struct ReadFrame {
    package_id: String,
    draft_id: String,
    returned: Option<ReadReturn>,
    body: Option<Vec<u8>>,
    original: Option<ModelChangeDraftV1>,
    first: Option<DraftMaterialReadFault>,
}

#[must_use = "this read owns the original ordinary material observation"]
pub(crate) struct DraftMaterialRead {
    frame: ReadFrame,
}
#[must_use = "Held retains the exact query, acquired material and actual failure owner"]
pub(crate) struct HeldDraftMaterialRead {
    frame: ReadFrame,
}

fn material_id(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// The containing package was already decoded by the original strict outbox
/// codec. Borrow only; no serde Value tree or caller-declared raw SQL is used.
#[derive(serde::Deserialize)]
struct DraftBodyLoan<'a> {
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
    fn read(&mut self) -> Result<(), DraftMaterialReadFault> {
        use DraftMaterialReadFault as E;
        if !material_id(&self.package_id, "retention-package-draft-v1:")
            || !material_id(&self.draft_id, "model-change-draft-v1:sha256:")
        {
            return Err(E::Input);
        }
        let Some(ReadReturn::Open(outbox)) = self.returned.take() else {
            unreachable!("actual input owner")
        };
        let result = outbox.read_material(self.package_id.clone()); // Fixed bounded query.
        self.returned = Some(match result {
            MaterialReadOutcome::Observed(v) => ReadReturn::Observed(v),
            MaterialReadOutcome::Held(v) => ReadReturn::Held(v),
        }); // Retain the entire genuine callee return before further decoding.
        let observed = match self.returned.as_ref().unwrap() {
            ReadReturn::Observed(v) => v,
            ReadReturn::Held(v) => return Err(E::Outbox(v.first_fault())),
            _ => unreachable!("complete actual return"),
        };
        let Some(package) = observed.material() else {
            return Ok(());
        };
        let loan: DraftBodyLoan<'_> =
            serde_json::from_slice(package.as_canonical_bytes()).map_err(|_| E::Envelope)?;
        if loan.owner_domain != OwnerDomain::Attribution
            || loan.owner_schema_claim != MATERIAL_SCHEMA
            || loan.business_day_claim != "1970-01-01"
            || loan.window_start_claim
                != (UtcInstantClaim {
                    unix_seconds: 0,
                    nanosecond: 0,
                })
            || loan.window_end_exclusive_claim
                != (UtcInstantClaim {
                    unix_seconds: 1,
                    nanosecond: 0,
                })
            || loan.claimed_record_count != Some(1)
            || loan.source_chain_before_claim.is_some()
            || loan.source_chain_after_claim.is_some()
            || loan.artifact_sha256_claim.is_some()
            || loan.activation_id_claim.is_some()
            || loan.body_encoding != "hex"
            || loan.body_length > 64 * 1024
            || loan.body_hex.len() != loan.body_length as usize * 2
        {
            return Err(E::Envelope);
        }
        // This bounded post-read decode begins after the genuine native owner
        // has completed its read/rollback/close. No Held Work is reset or reused.
        self.body = Some(vec![0; loan.body_length as usize]);
        hex::decode_to_slice(loan.body_hex, self.body.as_mut().unwrap())
            .map_err(|_| E::Envelope)?;
        let original = recover_model_change_draft_v1(self.body.as_ref().unwrap(), &self.draft_id)
            .map_err(E::Draft)?;
        self.original = Some(original); // Keep actual decoder return across the last check.
        if family_slot(self.original.as_ref().unwrap()) != loan.logical_slot_claim {
            return Err(E::Envelope);
        }
        Ok(())
    }
}

/// Caller needs the two saved content IDs, never the original entire Draft.
/// Neither a found declaration nor absence provides registration/approval proof.
pub(crate) fn read_draft_material(
    outbox: UnverifiedOutbox,
    package_id: String,
    draft_id: String,
) -> Result<DraftMaterialRead, HeldDraftMaterialRead> {
    let mut frame = ReadFrame {
        package_id,
        draft_id,
        returned: Some(ReadReturn::Open(outbox)),
        body: None,
        original: None,
        first: None,
    };
    match frame.read() {
        Ok(()) => Ok(DraftMaterialRead { frame }),
        Err(error) => {
            frame.first = Some(error);
            Err(HeldDraftMaterialRead { frame })
        }
    }
}

impl DraftMaterialRead {
    pub(crate) fn original(&self) -> Option<&ModelChangeDraftV1> {
        self.frame.original.as_ref()
    }
    pub(crate) fn observed_generation(&self) -> i64 {
        match self.frame.returned.as_ref() {
            Some(ReadReturn::Observed(v)) => v.observed_generation(),
            _ => unreachable!("actual completed read"),
        }
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
    pub(crate) fn other_family_materials(&self) -> usize {
        match self.frame.returned.as_ref() {
            Some(ReadReturn::Observed(v)) => v.other_slot_materials(),
            _ => unreachable!("actual completed read"),
        }
    }
}

impl HeldDraftMaterialRead {
    pub(crate) fn first_fault(&self) -> &DraftMaterialReadFault {
        self.frame.first.as_ref().expect("actual read fault")
    }
    pub(crate) fn package_id(&self) -> &str {
        &self.frame.package_id
    }
    pub(crate) fn draft_id(&self) -> &str {
        &self.frame.draft_id
    }
    pub(crate) fn drain_resources_once(mut self) -> Self {
        self.frame.returned = match self.frame.returned.take() {
            Some(ReadReturn::Open(v)) => match v.close() {
                Ok(()) => None,
                Err(v) => Some(ReadReturn::Held(v)),
            },
            Some(ReadReturn::Held(v)) => Some(ReadReturn::Held(v.drain_resources_once())),
            other => other,
        };
        self
    }
}

#[cfg(test)]
impl HeldDraftMaterialRead {
    pub(super) fn test_outbox(&self) -> Option<&HeldOutbox> {
        match self.frame.returned.as_ref() {
            Some(ReadReturn::Held(v)) => Some(v),
            _ => None,
        }
    }
    pub(super) fn test_observed_material(&self) -> Option<&UnverifiedEvidencePackageDraft> {
        match self.frame.returned.as_ref() {
            Some(ReadReturn::Observed(v)) => v.material(),
            _ => None,
        }
    }
    pub(super) fn test_original(&self) -> Option<&ModelChangeDraftV1> {
        self.frame.original.as_ref()
    }
    pub(super) fn test_finalize_then_drain(mut self) -> Self {
        let Some(ReadReturn::Held(v)) = self.frame.returned.take() else {
            panic!("actual read Held")
        };
        self.frame.returned = Some(ReadReturn::Held(v.test_finalize_then_drain()));
        self
    }
}
