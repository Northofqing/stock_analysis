use super::tests::Fixture as FundingFixture;
use super::*;
use crate::evidence_retention::outbox_v1::{
    EnqueueOutcome, LocalDisposition, OutboxFault, OutboxFixture, UnverifiedOutbox,
};
use crate::evidence_retention::{
    draft_from_claims, DraftClaimsRef, OwnerDomain, TrustState, UtcInstantClaim,
};
use crate::trading::paper_funding_review_store_v1::*;

fn original(fixture: &FundingFixture, proposal: &Proposal) -> StoredFundingReviewV1 {
    let observed = fixture.review(proposal).unwrap();
    read_stored_funding_review(observed.canonical_bytes()).unwrap()
}
fn open(fixture: &OutboxFixture) -> UnverifiedOutbox {
    match fixture.open() {
        Ok(value) => value,
        Err(value) => panic!("actual open {:?}", value.first_fault()),
    }
}
fn command(value: StoredFundingReviewV1) -> FundingMaterialCommand {
    match prepare_review_material(value) {
        Ok(value) => value,
        Err(value) => panic!("actual preparation {:?}", value.first_fault()),
    }
}
fn store(value: FundingMaterialOutcome) -> StoredFundingMaterial {
    match value {
        FundingMaterialOutcome::Stored(value) => value,
        FundingMaterialOutcome::Held(value) => panic!("actual store {:?}", value.first_fault()),
        _ => panic!("unexpected actual store result"),
    }
}
fn read(value: Result<FundingMaterialRead, HeldFundingMaterialRead>) -> FundingMaterialRead {
    match value {
        Ok(value) => value,
        Err(value) => panic!("actual read {:?}", value.first_fault()),
    }
}
fn held(value: Result<FundingMaterialRead, HeldFundingMaterialRead>) -> HeldFundingMaterialRead {
    match value {
        Err(value) => value,
        Ok(_) => panic!("expected actual Held"),
    }
}

#[test]
fn funding_material_read_cold_saved_ids_keep_original_and_family_conflicts() {
    let funding = FundingFixture::new(2);
    let mut proposal = funding.proposal();
    let fixture = OutboxFixture::new();
    let stored = store(command(original(&funding, &proposal)).persist(open(&fixture), 0));
    let package_id = stored.package_id().to_owned();
    let review_id = stored.review_id().to_owned();
    let raw = stored.canonical_review().to_vec();
    drop(stored);
    let cold = read(read_funding_material(
        open(&fixture),
        package_id.clone(),
        review_id.clone(),
    ));
    let actual = cold.original().unwrap();
    assert_eq!(actual.canonical_bytes(), raw);
    assert_eq!(actual.outcome(), Outcome::ConsistentProposal);
    assert!(actual.initial_cash().is_some());
    assert_eq!(
        (
            cold.observed_generation(),
            cold.other_family_materials(),
            cold.trust()
        ),
        (1, 0, TrustState::Unverified)
    );
    proposal.genesis_event_hash = "b".repeat(64);
    let conflicting = store(command(original(&funding, &proposal)).persist(open(&fixture), 1));
    assert_eq!(conflicting.disposition(), LocalDisposition::Conflict);
    let changed_id = conflicting.package_id().to_owned();
    let changed_review_id = conflicting.review_id().to_owned();
    let changed_raw = conflicting.canonical_review().to_vec();
    drop(conflicting);
    drop(cold);
    drop(funding); // Recovery has no funding DB, issuer, proposal DTO or raw body input.
    let old = read(read_funding_material(open(&fixture), package_id, review_id));
    assert_eq!(old.original().unwrap().canonical_bytes(), raw);
    assert_eq!(
        (old.observed_generation(), old.other_family_materials()),
        (2, 1)
    );
    let changed = read(read_funding_material(
        open(&fixture),
        changed_id,
        changed_review_id,
    ));
    assert_eq!(changed.original().unwrap().canonical_bytes(), changed_raw);
    assert_eq!(
        changed.original().unwrap().outcome(),
        Outcome::InconsistentProposal(FundingMismatchV1::GenesisEvent)
    );
    assert!(changed.original().unwrap().initial_cash().is_none());
    assert_eq!(changed.other_family_materials(), 1);
    let mut unchanged = open(&fixture);
    assert_eq!(
        (
            unchanged.generation(),
            unchanged.test_material_count(),
            unchanged.test_conflict_count()
        ),
        (2, 2, 1)
    );
    assert!(unchanged.close().is_ok());
    let wire: serde_json::Value =
        serde_json::from_slice(old.original().unwrap().canonical_bytes()).unwrap();
    assert_eq!(wire["authority_state"], "HistoricalObservationOnly");
    assert_eq!(wire["approval_state"], "NotIssued");
    assert!(crate::decision::approved_paper_intent_v1::require_production_approval().is_err());
}

#[test]
fn funding_material_read_wrong_scope_identity_family_and_approval_keep_material() {
    let funding = FundingFixture::new(2);
    let original = original(&funding, &funding.proposal());
    let raw = original.canonical_bytes();
    let source = command(read_stored_funding_review(raw).unwrap());
    let slot = source.slot().to_owned();
    let original_text = std::str::from_utf8(raw).unwrap();
    let approval_field = "\"approval_state\":\"NotIssued\"";
    assert_eq!(original_text.matches(approval_field).count(), 1);
    // Keep the original canonical field order; change only the forbidden value.
    let promoted_raw = original_text
        .replace(approval_field, "\"approval_state\":\"Approved\"")
        .into_bytes();
    for case in 0..7 {
        let large = vec![b'a'; MATERIAL_REVIEW_LIMIT + 1];
        let body = match case {
            5 => promoted_raw.as_slice(),
            6 => large.as_slice(),
            _ => raw,
        };
        let claims = DraftClaimsRef {
            owner_domain: if case == 0 {
                OwnerDomain::Attribution
            } else {
                OwnerDomain::PaperLedger
            },
            owner_schema_claim: if case == 1 {
                "TEST_CODE_other_schema"
            } else {
                "paper-funding-review-material-v1"
            },
            logical_slot_claim: if case == 3 {
                "TEST_CODE_wrong_family"
            } else {
                &slot
            },
            business_day_claim: "1970-01-01",
            window_start_claim: UtcInstantClaim {
                unix_seconds: 0,
                nanosecond: 0,
            },
            window_end_exclusive_claim: UtcInstantClaim {
                unix_seconds: if case == 2 { 2 } else { 1 },
                nanosecond: 0,
            },
            claimed_record_count: Some(1),
            source_chain_before_claim: None,
            source_chain_after_claim: None,
            artifact_sha256_claim: None,
            activation_id_claim: None,
        };
        let package = draft_from_claims(claims, body).unwrap();
        let package_id = package.id().to_owned();
        let fixture = OutboxFixture::new();
        assert!(matches!(
            open(&fixture).enqueue(package, 0),
            EnqueueOutcome::Stored(_)
        ));
        let review_id = if case == 4 {
            format!("paper-funding-review-v1:{}", "0".repeat(64))
        } else {
            original.review_id().to_owned()
        };
        let failed = held(read_funding_material(
            open(&fixture),
            package_id.clone(),
            review_id.clone(),
        ));
        assert_eq!(
            (failed.package_id(), failed.review_id()),
            (package_id.as_str(), review_id.as_str())
        );
        assert_eq!(failed.test_material().unwrap().id(), package_id);
        assert_eq!(
            failed.first_fault(),
            match case {
                4 => FundingMaterialReadFault::Identity,
                5 => FundingMaterialReadFault::Review(Error::Schema),
                _ => FundingMaterialReadFault::Envelope,
            }
        );
        if case == 3 || case == 4 {
            assert_eq!(failed.test_original().unwrap().canonical_bytes(), raw);
        } else {
            assert!(failed.test_original().is_none());
        }
        if case == 5 {
            assert_eq!(failed.test_body().unwrap(), promoted_raw);
        }
        if case == 6 {
            assert!(failed.test_body().is_none());
        }
        drop(failed.drain_resources_once());
        let mut unchanged = open(&fixture);
        assert_eq!(
            (unchanged.generation(), unchanged.test_material_count()),
            (1, 1)
        );
        assert!(unchanged.close().is_ok());
    }
}

#[test]
fn funding_material_read_close_invalid_query_and_absence_preserve_failures() {
    let funding = FundingFixture::new(2);
    let fixture = OutboxFixture::new();
    let stored = store(command(original(&funding, &funding.proposal())).persist(open(&fixture), 0));
    let package_id = stored.package_id().to_owned();
    let review_id = stored.review_id().to_owned();
    let failed = held(read_funding_material(
        open(&fixture).test_busy_vm(),
        package_id.clone(),
        review_id.clone(),
    ));
    assert_eq!(
        failed.first_fault(),
        FundingMaterialReadFault::Outbox(OutboxFault::CloseHeld)
    );
    assert!(failed.test_outbox().unwrap().test_connection_retained());
    assert_eq!(
        failed.test_outbox().unwrap().test_read_id(),
        Some(package_id.as_str())
    );
    assert_eq!(
        failed
            .test_outbox()
            .unwrap()
            .test_selected_material()
            .unwrap()
            .id(),
        package_id
    );
    let drained = failed.test_finalize_then_drain();
    assert_eq!(
        drained.first_fault(),
        FundingMaterialReadFault::Outbox(OutboxFault::CloseHeld)
    );
    assert!(!drained.test_outbox().unwrap().test_connection_retained());
    assert_eq!(
        drained
            .test_outbox()
            .unwrap()
            .test_selected_material()
            .unwrap()
            .id(),
        package_id
    );
    drop(drained.drain_resources_once());
    let invalid = "TEST_CODE_invalid".repeat(4096);
    let pointer = invalid.as_ptr();
    let failed = held(read_funding_material(
        open(&fixture).test_busy_vm(),
        invalid,
        review_id.clone(),
    ));
    assert_eq!(failed.package_id().as_ptr(), pointer);
    assert_eq!(failed.first_fault(), FundingMaterialReadFault::Input);
    let drained = failed.drain_resources_once();
    assert_eq!(drained.first_fault(), FundingMaterialReadFault::Input);
    assert!(drained.test_outbox().unwrap().test_connection_retained());
    let drained = drained.test_finalize_then_drain();
    assert_eq!(drained.first_fault(), FundingMaterialReadFault::Input);
    assert_eq!(drained.package_id().as_ptr(), pointer);
    drop(drained.drain_resources_once());
    let absent = read(read_funding_material(
        open(&fixture),
        format!("retention-package-draft-v1:{}", "0".repeat(64)),
        review_id,
    ));
    assert!(absent.original().is_none());
    assert_eq!(
        (
            absent.observed_generation(),
            absent.other_family_materials(),
            absent.trust()
        ),
        (1, 0, TrustState::Unverified)
    );
    let original = read(read_funding_material(
        open(&fixture),
        package_id,
        stored.review_id().to_owned(),
    ));
    assert_eq!(
        original.original().unwrap().canonical_bytes(),
        stored.canonical_review()
    );
    let mut unchanged = open(&fixture);
    assert_eq!(
        (unchanged.generation(), unchanged.test_material_count()),
        (1, 1)
    );
    assert!(unchanged.close().is_ok());
}
