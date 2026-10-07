use super::outbox_v1::*;
use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn draft(slot: &str, body: &[u8]) -> UnverifiedEvidencePackageDraft {
    draft_from_claims(
        DraftClaimsRef {
            owner_domain: OwnerDomain::Data,
            owner_schema_claim: "outbox-fixture-unverified",
            logical_slot_claim: slot,
            business_day_claim: "1970-01-01",
            window_start_claim: UtcInstantClaim {
                unix_seconds: 0,
                nanosecond: 0,
            },
            window_end_exclusive_claim: UtcInstantClaim {
                unix_seconds: 1,
                nanosecond: 0,
            },
            claimed_record_count: Some(0),
            source_chain_before_claim: None,
            source_chain_after_claim: None,
            artifact_sha256_claim: None,
            activation_id_claim: None,
        },
        body,
    )
    .unwrap()
}
fn open(fixture: &OutboxFixture) -> UnverifiedOutbox {
    match fixture.open() {
        Ok(store) => store,
        Err(held) => panic!("real local open Held: {:?}", held.first_fault()),
    }
}
fn stored(outcome: EnqueueOutcome) -> StoredUnverified {
    match outcome {
        EnqueueOutcome::Stored(stored) => stored,
        EnqueueOutcome::Held(held) => panic!("enqueue Held: {:?}", held.first_fault()),
        EnqueueOutcome::Pending(pending) => panic!("enqueue Pending: {:?}", pending.first_fault()),
    }
}
fn held(outcome: EnqueueOutcome) -> HeldOutbox {
    match outcome {
        EnqueueOutcome::Held(held) => held,
        _ => panic!("expected genuine Held owner"),
    }
}
fn close(store: UnverifiedOutbox) {
    if let Err(held) = store.close() {
        panic!("real local close Held: {:?}", held.first_fault());
    }
}
fn observed(outcome: RecoveryOutcome) -> RecoveredUnverified {
    match outcome {
        RecoveryOutcome::Observed(value) => value,
        _ => panic!("expected closed Unverified readback"),
    }
}
fn crash_child(fixture: &OutboxFixture, cut: &str, expected: i32) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "evidence_retention::outbox_v1_tests::retention_outbox_unknown_commit_and_before_owned_budget", "--nocapture"])
        .env("RETENTION_OUTBOX_TEST_CHILD",cut).env("RETENTION_OUTBOX_TEST_ROOT",fixture.root())
        .output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(expected),
        "child stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn retention_outbox_enqueue_reuse_conflict_and_namespace_quota() {
    let fixture = OutboxFixture::new();
    let first = stored(open(&fixture).enqueue(draft("slot", b"a"), 0));
    assert_eq!(
        (first.generation, first.disposition, first.trust()),
        (1, LocalDisposition::Stored, TrustState::Unverified)
    );
    let cold = open(&fixture);
    assert_eq!(cold.generation(), 1);
    assert_eq!(cold.trust(), TrustState::Unverified);
    let exact = stored(cold.enqueue(draft("slot", b"a"), 1));
    assert_eq!(
        (exact.generation, exact.disposition),
        (2, LocalDisposition::ExactReuse)
    );
    let conflict = stored(open(&fixture).enqueue(draft("slot", b"b"), 2));
    assert_eq!(
        (conflict.generation, conflict.disposition),
        (3, LocalDisposition::Conflict)
    );
    let mut count = open(&fixture);
    assert_eq!(count.test_material_count(), 2);
    assert_eq!(count.test_conflict_count(), 1);
    close(count);

    let quota = OutboxFixture::new();
    for i in 0..32 {
        let slot = format!("slot-{i}");
        stored(open(&quota).enqueue(draft(&slot, b"fixed"), i));
    }
    let refused = held(open(&quota).enqueue(draft("slot-33", b"fixed"), 32));
    assert_eq!(refused.first_fault(), OutboxFault::Quota);
    assert!(refused.test_command_retained());
    drop(refused.drain_resources_once());
    let mut still = open(&quota);
    assert_eq!(still.test_material_count(), 32);
    assert_eq!(still.generation(), 32);
    close(still);

    let replacement = OutboxFixture::new();
    let store = open(&replacement);
    let old = replacement.directory().join("old-main");
    fs::rename(replacement.main(), &old).unwrap();
    fs::copy(&old, replacement.main()).unwrap();
    fs::set_permissions(replacement.main(), fs::Permissions::from_mode(0o600)).unwrap();
    let refused = held(store.enqueue(draft("bound", b"not-installed"), 0));
    assert_eq!(refused.first_fault(), OutboxFault::RootBinding);
    assert!(refused.test_connection_retained() && refused.test_command_retained());
    drop(refused.drain_resources_once());
    fs::remove_file(replacement.main()).unwrap();
    fs::rename(old, replacement.main()).unwrap(); // Fixture cleanup only.
    let restored = open(&replacement);
    assert_eq!(restored.generation(), 0);
    close(restored);

    let root_swap = OutboxFixture::new();
    let store = open(&root_swap);
    let previous = root_swap.root().join("old-data");
    fs::rename(root_swap.root().join("data"), &previous).unwrap();
    fs::create_dir(root_swap.root().join("data")).unwrap();
    let refused = held(store.enqueue(draft("bound", b"not-installed"), 0));
    assert_eq!(refused.first_fault(), OutboxFault::RootBinding);
    drop(refused.drain_resources_once());
    fs::remove_dir(root_swap.root().join("data")).unwrap();
    fs::rename(previous, root_swap.root().join("data")).unwrap();
}

#[test]
fn retention_outbox_unknown_commit_and_before_owned_budget() {
    if let Ok(cut) = std::env::var("RETENTION_OUTBOX_TEST_CHILD") {
        let root =
            std::path::PathBuf::from(std::env::var_os("RETENTION_OUTBOX_TEST_ROOT").unwrap());
        let observation = match cut.as_str() {
            "before" => TestCommitObservation::ExitBeforeCommit,
            "after" => TestCommitObservation::ExitAfterCommit,
            _ => panic!("fixed child cut"),
        };
        UnverifiedOutbox::test_child(&root, observation, draft("child", b"real transaction"));
    }
    let fixture = OutboxFixture::new();
    let pending = match open(&fixture)
        .test_observation(TestCommitObservation::LoseResponse)
        .enqueue(draft("response", b"committed"), 0)
    {
        EnqueueOutcome::Pending(pending) => pending,
        _ => panic!("real committed response-loss must retain Pending"),
    };
    assert_eq!(pending.first_fault(), OutboxFault::CommitUnknown);
    assert!(pending.test_connection_and_command_retained());
    let result = observed(pending.observe());
    assert_eq!(
        (result.presence, result.original_fault, result.trust()),
        (
            RecoveredPresence::ExactUnverified,
            OutboxFault::CommitUnknown,
            TrustState::Unverified
        )
    );
    let cold = open(&fixture);
    assert_eq!(cold.generation(), 1);
    close(cold);

    let before = OutboxFixture::new();
    close(open(&before));
    crash_child(&before, "before", 73);
    let journal = before.directory().join("materials-v1.sqlite-journal");
    assert!(journal.metadata().unwrap().len() > 0);
    let rejected = match before.open() {
        Err(held) => held,
        Ok(_) => panic!("residual journal must reject before native open"),
    };
    assert_eq!(rejected.first_fault(), OutboxFault::ResidualJournal);
    assert!(!rejected.test_connection_retained());
    assert!(journal.exists());
    drop(rejected); // No auto-recovery, deletion or successful-crash claim.

    let after = OutboxFixture::new();
    close(open(&after));
    crash_child(&after, "after", 74);
    let readback = observed(open(&after).observe_previous(draft("child", b"real transaction"), 1));
    assert_eq!(readback.presence, RecoveredPresence::ExactUnverified);
    assert_eq!(readback.original_fault, OutboxFault::CommitUnknown);
    let missing = OutboxFixture::new();
    let readback =
        observed(open(&missing).observe_previous(draft("unknown", b"not reconstructed"), 1));
    assert_eq!(readback.presence, RecoveredPresence::MissingFactsUnknown);
    let unchanged = open(&missing);
    assert_eq!(unchanged.generation(), 0);
    close(unchanged);

    let budget = OutboxFixture::new();
    let mut short = open(&budget);
    assert_eq!(
        short.test_spend_owned(8 * MIB),
        Err(OutboxFault::Work(ValueError::AllocationLimit))
    );
    let failed = held(short.enqueue(draft("short", b"owned command"), 0));
    assert_eq!(
        failed.first_fault(),
        OutboxFault::Work(ValueError::AllocationLimit)
    );
    assert!(failed.test_command_retained());
    drop(failed.drain_resources_once());
    let mut pending = match open(&budget)
        .test_observation(TestCommitObservation::LoseResponse)
        .enqueue(draft("pending", b"owned command"), 0)
    {
        EnqueueOutcome::Pending(pending) => pending,
        _ => panic!("response loss"),
    };
    assert!(pending.test_exhaust_same_work().is_err());
    let retained = match pending.observe() {
        RecoveryOutcome::Held(held) => held,
        _ => panic!("same Work must not reset during observe"),
    };
    assert_eq!(retained.first_fault(), OutboxFault::CommitUnknown);
    assert!(retained.test_connection_retained() && retained.test_command_retained());
    drop(retained.drain_resources_once());

    let oversized = OutboxFixture::new();
    stored(open(&oversized).enqueue(draft("shape", b"bytes"), 0));
    let mut changed = open(&oversized);
    changed.test_corrupt_sql("UPDATE material SET canonical=zeroblob(3145729),bytes=3145729");
    close(changed);
    codec_v1::OWNED_HITS.with(|hits| hits.set(0));
    let rejected = match oversized.open() {
        Err(held) => held,
        Ok(_) => panic!("oversized database BLOB must reject"),
    };
    assert_eq!(rejected.first_fault(), OutboxFault::Quota);
    codec_v1::OWNED_HITS.with(|hits| assert_eq!(hits.get(), 0));
    drop(rejected.drain_resources_once());

    let bad_hash = OutboxFixture::new();
    stored(open(&bad_hash).enqueue(draft("hash", b"bytes"), 0));
    let mut changed = open(&bad_hash);
    changed.test_corrupt_sql("UPDATE material SET sha=printf('%064d',0)");
    close(changed);
    let rejected = match bad_hash.open() {
        Err(held) => held,
        Ok(_) => panic!("canonical checksum must reject"),
    };
    assert_eq!(rejected.first_fault(), OutboxFault::ForeignShape);
    assert!(rejected.test_inventory_retained());
    drop(rejected.drain_resources_once());

    // One actual shared Work reads many tiny, already closed DraftWire values;
    // scanning is cumulative in input bytes, never one schema maximum per row.
    let quoted = draft("slot-\"\\éλ", b"fixed");
    let bytes = quoted.as_canonical_bytes();
    let mut shared = Work::new();
    for _ in 0..64 {
        let parsed = parse_draft_with_work(bytes, &mut shared).unwrap();
        assert_eq!(parsed.as_canonical_bytes(), bytes);
        assert_eq!(parsed.id(), quoted.id());
    }
    let mut noncanonical = bytes.to_vec();
    noncanonical.push(b' ');
    assert!(matches!(
        parse_draft_with_work(&noncanonical, &mut Work::new()),
        Err(ValueError::NonCanonical)
    ));
    let mut short_scan = Work::new();
    short_scan.scan(32 * MIB - bytes.len() * 2 + 1).unwrap();
    codec_v1::OWNED_HITS.with(|hits| hits.set(0));
    assert!(matches!(
        parse_draft_with_work(bytes, &mut short_scan),
        Err(ValueError::InputLimit)
    ));
    codec_v1::OWNED_HITS.with(|hits| assert_eq!(hits.get(), 0));
    assert!(matches!(
        parse_draft_with_work(bytes, &mut short_scan),
        Err(ValueError::InputLimit)
    ));
    codec_v1::OWNED_HITS.with(|hits| assert_eq!(hits.get(), 0));

    let busy_close = OutboxFixture::new();
    let rejected = match open(&busy_close).test_busy_vm().close() {
        Err(held) => held,
        Ok(()) => panic!("true unresolved VM must cause consuming close Err"),
    };
    assert_eq!(rejected.first_fault(), OutboxFault::CloseHeld);
    assert!(rejected.test_connection_retained());
    let drained = rejected.test_finalize_then_drain();
    assert_eq!(drained.first_fault(), OutboxFault::CloseHeld);
    assert!(!drained.test_connection_retained());
    drop(drained);
}

#[test]
fn retention_outbox_two_coordinators_busy_stale_and_exact_content() {
    let fixture = OutboxFixture::new();
    let mut a = open(&fixture);
    let b = open(&fixture);
    assert_eq!((a.generation(), b.generation()), (0, 0));
    a.test_hold_transaction().unwrap();
    let busy = held(b.enqueue(draft("shared", b"b"), 0));
    assert_eq!(busy.first_fault(), OutboxFault::Busy);
    assert!(busy.test_connection_retained() && busy.test_command_retained());
    let busy = busy.drain_resources_once();
    assert_eq!(busy.first_fault(), OutboxFault::Busy);
    drop(busy);
    if let Err(held) = a.test_rollback() {
        panic!("actual rollback/close: {:?}", held.first_fault());
    }

    let a = open(&fixture);
    let b = open(&fixture);
    let installed = stored(a.enqueue(draft("shared", b"a"), 0));
    assert_eq!(installed.generation, 1);
    let stale = held(b.enqueue(draft("shared", b"b"), 0));
    assert_eq!(stale.first_fault(), OutboxFault::StaleGeneration);
    assert!(stale.test_command_retained());
    drop(stale.drain_resources_once());
    let conflict = stored(open(&fixture).enqueue(draft("shared", b"b"), 1));
    assert_eq!(conflict.disposition, LocalDisposition::Conflict);

    let a = open(&fixture);
    let b = open(&fixture);
    assert_eq!((a.generation(), b.generation()), (2, 2));
    let reused = stored(a.enqueue(draft("shared", b"a"), 2));
    assert_eq!(reused.disposition, LocalDisposition::ExactReuse);
    let stale = held(b.enqueue(draft("shared", b"a"), 2));
    assert_eq!(stale.first_fault(), OutboxFault::StaleGeneration);
    drop(stale.drain_resources_once());
    let mut final_state = open(&fixture);
    assert_eq!(final_state.generation(), 3);
    assert_eq!(final_state.test_material_count(), 2);
    assert_eq!(final_state.test_conflict_count(), 1);
    close(final_state);

    let forbidden = OutboxFixture::new();
    let store = open(&forbidden);
    fs::write(forbidden.directory().join("unexpected"), b"foreign").unwrap();
    let refused = held(store.enqueue(draft("strict", b"retained"), 0));
    assert_eq!(refused.first_fault(), OutboxFault::ForeignShape);
    assert!(refused.test_command_retained());
    drop(refused.drain_resources_once());
    fs::remove_file(forbidden.directory().join("unexpected")).unwrap();
    let known = open(&forbidden);
    assert_eq!(known.generation(), 0);
    close(known);
}
