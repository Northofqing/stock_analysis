use super::*;

#[test]
fn p05_counted_producer_slot_uses_one_shanghai_date_and_minute_before_await() {
    let captured = "2026-09-23T10:30:59+08:00".parse().unwrap();
    let (date, hhmm) = candidate_board_slot_at("2026-09-23", captured).unwrap();
    let after_await: chrono::DateTime<chrono::FixedOffset> =
        "2026-09-23T10:31:05+08:00".parse().unwrap();
    assert_ne!(hhmm, after_await.format("%H:%M").to_string());
    assert_eq!(date.to_string(), "2026-09-23");
    assert_eq!(hhmm, "10:30");
    for (date, now) in [
        ("2026-09-22", "2026-09-23T10:30:59+08:00"),
        ("2026-9-23", "2026-09-23T10:30:59+08:00"),
        ("2026-09-23", "2026-09-23T10:30:59+00:00"),
        ("2026-10-04", "2026-10-04T10:30:59+08:00"),
        ("2027-01-04", "2027-01-04T10:30:59+08:00"),
    ] {
        assert!(candidate_board_slot_at(date, now.parse().unwrap()).is_err());
    }
}

const CHILD_ENV: &str = "TEST_CODE_P05_PRODUCER_BINDING_CHILD";
const CHILD_TEST: &str =
    "push_templates::p05_counted_producer_tests::p05_counted_producer_real_binding_child";

/// Reuses the producer process fixture for the actual shared orchestration,
/// renderer, blocking workflow, v2 factory and counted consumer/Test receipt.
/// The single acquisition callback returns observed test data, not admitted
/// provider authority; A02/T08 outcomes are negative local observations.
#[tokio::test]
#[ignore = "exact isolated child invoked by the P05 producer restart test"]
async fn p05_counted_producer_real_binding_child() {
    use stock_analysis::monitor::prediction::{
        prepare_candidate_board, CandidateBoardPreparationRequest,
    };
    use stock_analysis::p05_candidate_board_link::{
        read_candidate_board_occurrence_link, CandidateBoardOccurrenceLinkV1,
    };
    assert_eq!(std::env::var(CHILD_ENV).unwrap(), "1");
    assert_eq!(
        stock_analysis::risk::env_guard::current_env(),
        stock_analysis::risk::env_guard::TradingEnv::Test
    );
    let code = std::env::var("DURABLE_DELIVERY_TEST_CODE").unwrap();
    assert!(code.starts_with("TEST_CODE_P05_PRODUCER_"));
    let role = std::env::var("TEST_CODE_P05_PRODUCER_ROLE").unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data/test")
        .join(&code);
    let prediction_path = root.join("TEST_CODE_p05_prediction.db");
    stock_analysis::database::DatabaseManager::init(Some(prediction_path)).unwrap();
    let database = stock_analysis::database::DatabaseManager::get();
    use p05_shared_unit::{CandidateChildObservation, InvalidatedObservation};
    use std::cell::{Cell, RefCell};
    use stock_analysis::opportunity::candidate_panel::EvidenceTier;
    let observed_batch = || {
        p05_shared_unit::tests::batch(vec![
            p05_shared_unit::tests::entry(
                "TEST_CODE_p05_a",
                EvidenceTier::Strong,
                Some(10.0),
                Some(80.0),
            ),
            p05_shared_unit::tests::entry(
                "TEST_CODE_p05_b",
                EvidenceTier::Strong,
                Some(11.0),
                Some(81.0),
            ),
        ])
    };
    let expected_board = stock_analysis::opportunity::candidate_panel::format_candidate_board(
        &observed_batch().entries,
    );
    let mut auction_entries = observed_batch().entries;
    auction_entries.reverse();
    let expected_auction = render_auction_repush("10:30:59", &auction_entries);
    let occurrence = "candidate-board:2026-09-23:10:30";
    let before = database.read_candidate_board_v2_freeze(occurrence).unwrap();
    assert_eq!(before.is_some(), role == "restart");
    let acquisitions = Cell::new(0);
    let events = RefCell::new(Vec::new());
    let auction_denied =
        crate::notify::PushOutcome::Denied("TEST_CODE original auction denial".to_owned());
    let invalidated_error =
        crate::notify::PushOutcome::SinkError("TEST_CODE original T08 sink uncertainty".to_owned());
    let observations = p05_shared_unit::dispatch_with(
        "2026-09-23",
        "2026-09-23T10:30:59+08:00".parse().unwrap(),
        || async {
            acquisitions.set(acquisitions.get() + 1);
            // The captured slot must survive an actual await before preparation.
            tokio::task::yield_now().await;
            Ok(observed_batch())
        },
        |child| {
            let kind = child.token.descriptor().push_kind;
            assert_eq!(child.binding.business_date().to_string(), "2026-09-23");
            assert!(child.binding.task_binding().is_none());
            let outcome = match kind {
                crate::notify::PushKind::AuctionRepush => {
                    events.borrow_mut().push("A02");
                    assert_eq!(child.text, expected_auction);
                    assert_eq!(
                        child.binding.schedule_occurrence_identity(),
                        "auction-repush:2026-09-23:10:30:59"
                    );
                    assert!(!child.binding.retry_authorized());
                    Some(auction_denied.clone())
                }
                crate::notify::PushKind::CandidateInvalidated => {
                    events.borrow_mut().push("T08");
                    assert_eq!(
                        child.text,
                        render_candidate_invalidated(
                            "10:30:59",
                            "600009",
                            "600009",
                            "候选",
                            "从候选台消失"
                        )
                    );
                    assert_eq!(
                        child.binding.schedule_occurrence_identity(),
                        "candidate-invalidated:2026-09-23:600009"
                    );
                    assert_eq!(child.binding.governance_code(), Some("600009"));
                    assert!(child.binding.retry_authorized());
                    Some(invalidated_error.clone())
                }
                crate::notify::PushKind::CandidateBoard => {
                    events.borrow_mut().push("P05");
                    let frozen = database
                        .read_candidate_board_v2_freeze(occurrence)
                        .unwrap()
                        .unwrap();
                    assert_eq!(child.text, expected_board);
                    assert_eq!(child.text.as_bytes(), frozen.rendered_bytes());
                    assert_eq!(
                        child.binding.source_binding_canonical(),
                        frozen.source_canonical()
                    );
                    assert_eq!(
                        child.binding.source_evidence_fingerprint(),
                        frozen.source_sha256()
                    );
                    assert_eq!(
                        child.binding.delivery_subject_hash(),
                        frozen.source_sha256()
                    );
                    assert_eq!(child.binding.schedule_occurrence_identity(), occurrence);
                    assert!(!child.binding.retry_authorized());
                    None
                }
                _ => panic!("shared candidate batch cannot dispatch another kind"),
            };
            async move {
                match outcome {
                    Some(outcome) => outcome,
                    None => {
                        crate::durable_delivery_runtime::deliver_counted_binding(
                            child.binding,
                            kind,
                            child.text,
                            None,
                        )
                        .await
                    }
                }
            }
        },
        || {
            events.borrow_mut().push("previous");
            Some(["600009".to_owned()].into())
        },
        |codes| {
            events.borrow_mut().push("snapshot");
            let expected_codes: std::collections::BTreeSet<_> =
                ["TEST_CODE_p05_a".to_owned(), "TEST_CODE_p05_b".to_owned()].into();
            assert_eq!(codes, &expected_codes);
            assert_eq!(database.count_predictions().unwrap(), 2);
        },
    )
    .await;
    assert_eq!(acquisitions.get(), 1);
    assert_eq!(
        *events.borrow(),
        vec!["A02", "previous", "T08", "snapshot", "P05"]
    );
    assert_eq!(
        observations.auction_repush,
        CandidateChildObservation::Observed(auction_denied)
    );
    assert_eq!(
        observations.candidate_board,
        CandidateChildObservation::Observed(crate::notify::PushOutcome::Pushed)
    );
    assert_eq!(
        observations.invalidated,
        InvalidatedObservation::Items(vec![(
            "600009".to_owned(),
            CandidateChildObservation::Observed(invalidated_error)
        )])
    );
    assert!(!observations.auction_repush.was_pushed() && observations.candidate_board.was_pushed());
    let record = database
        .read_candidate_board_v2_freeze(occurrence)
        .unwrap()
        .unwrap();
    assert_eq!(
        record
            .ordered_rows()
            .iter()
            .map(|row| row.code())
            .collect::<Vec<_>>(),
        vec!["TEST_CODE_p05_a", "TEST_CODE_p05_b"]
    );
    assert_eq!(record.rendered_bytes(), expected_board.as_bytes());
    if let Some(before) = before {
        assert_eq!(
            record, before,
            "restart must reuse exact bytes and original prediction row IDs"
        );
    }
    assert_eq!(database.count_predictions().unwrap(), 2);

    let counted = stock_analysis::durable_delivery::DurableDeliveryCoordinator::open(
        stock_analysis::durable_delivery::CoordinatorConfig::test(
            std::path::PathBuf::from("data/test")
                .join(&code)
                .join("durable_delivery.sqlite3"),
            &code,
            format!("TEST_CODE_P05_READER_{}", std::process::id()),
        ),
    )
    .unwrap();
    let link = read_candidate_board_occurrence_link(
        database,
        &counted,
        record.business_date(),
        record.occurrence_identity(),
    )
    .unwrap();
    assert_eq!(link.accepted_rows().unwrap(), record.ordered_rows());
    let CandidateBoardOccurrenceLinkV1::VerifiedV2 { card, .. } = &link else {
        panic!("actual counted consumer must leave matching v2 evidence");
    };
    let expected = stock_analysis::durable_delivery::DeliveryEnvelope::new(
        record.business_date(),
        stock_analysis::durable_delivery::PushKind::CandidateBoard,
        stock_analysis::durable_delivery::DeliverySubKind::None,
        "GLOBAL",
        record.occurrence_identity(),
        record.source_sha256(),
        record.source_canonical().to_vec(),
        record.source_sha256(),
        record.rendered_bytes().to_vec(),
        false,
        None,
    )
    .unwrap();
    assert_eq!(card.decision_identity(), expected.decision_identity);
    assert_eq!(card.envelope_sha256(), expected.canonical_sha256().unwrap());

    // A missing legacy baseline and an explicitly empty observed difference
    // stay distinct. Neither can manufacture another child's Accepted result.
    for (previous, outcome, expected_delta) in [
        (
            None,
            crate::notify::PushOutcome::SinkError(
                "TEST_CODE exact board sink uncertainty".to_owned(),
            ),
            InvalidatedObservation::NoPreviousSnapshot,
        ),
        (
            Some(["TEST_CODE_p05_a".to_owned(), "TEST_CODE_p05_b".to_owned()].into()),
            crate::notify::PushOutcome::Deduped,
            InvalidatedObservation::EmptyObservedDifference,
        ),
        (
            None,
            crate::notify::PushOutcome::Denied("TEST_CODE exact board denial".to_owned()),
            InvalidatedObservation::NoPreviousSnapshot,
        ),
    ] {
        let reads = Cell::new(0);
        let snapshots = Cell::new(0);
        let observed = p05_shared_unit::dispatch_with(
            "2026-09-23",
            "2026-09-23T10:30:59+08:00".parse().unwrap(),
            || {
                reads.set(reads.get() + 1);
                std::future::ready(Ok(observed_batch()))
            },
            |child| {
                let kind = child.token.descriptor().push_kind;
                std::future::ready(match kind {
                    crate::notify::PushKind::AuctionRepush => crate::notify::PushOutcome::Deduped,
                    crate::notify::PushKind::CandidateBoard => {
                        assert_eq!(
                            snapshots.get(),
                            1,
                            "legacy snapshot precedes even a negative board send"
                        );
                        assert_eq!(child.text.as_bytes(), record.rendered_bytes());
                        outcome.clone()
                    }
                    _ => panic!("missing or empty baseline must not invent T08 delivery"),
                })
            },
            || previous,
            |_| {
                snapshots.set(snapshots.get() + 1);
            },
        )
        .await;
        assert_eq!(reads.get(), 1);
        assert_eq!(snapshots.get(), 1);
        assert_eq!(observed.invalidated, expected_delta);
        assert_eq!(
            observed.candidate_board,
            CandidateChildObservation::Observed(outcome)
        );
        assert!(!observed.auction_repush.was_pushed() && !observed.candidate_board.was_pushed());
        assert_eq!(
            database
                .read_candidate_board_v2_freeze(occurrence)
                .unwrap()
                .unwrap(),
            record
        );
    }

    let legacy_binding = build_candidate_board_counted_binding(
        chrono::NaiveDate::from_ymd_opt(2026, 9, 23).unwrap(),
        "10:30",
        std::str::from_utf8(record.rendered_bytes()).unwrap(),
    )
    .unwrap();
    let competing = crate::durable_delivery_runtime::deliver_counted_binding(
        legacy_binding,
        crate::notify::PushKind::CandidateBoard,
        String::from_utf8(record.rendered_bytes().to_vec()).unwrap(),
        None,
    )
    .await;
    assert_ne!(
        competing,
        crate::notify::PushOutcome::Pushed,
        "a v1 fallback must not bypass an existing counted v2 owner"
    );

    // A newly empty Strong set or changed card cannot downgrade this slot.
    for (bytes, samples) in [
        (record.rendered_bytes().to_vec(), Vec::new()),
        (
            b"TEST_CODE changed card".to_vec(),
            vec![("TEST_CODE_p05_a".to_owned(), 80.0)],
        ),
    ] {
        let changed =
            CandidateBoardPreparationRequest::new("2026-09-23", "10:30", bytes, samples).unwrap();
        assert_eq!(
            prepare_candidate_board(changed).await.unwrap_err().reason(),
            "p05_frozen_card_drift"
        );
    }
    assert_eq!(database.count_predictions().unwrap(), 2);
    assert_eq!(
        read_candidate_board_occurrence_link(
            database,
            &counted,
            record.business_date(),
            record.occurrence_identity()
        )
        .unwrap(),
        link
    );
    println!("p05-real-binding-{role}-verified");
}

#[test]
fn p05_shared_unit_one_acquisition_reuses_real_binding_fixture_across_restart() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let code = format!(
        "TEST_CODE_P05_PRODUCER_{}_{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    );
    let parent = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/test");
    std::fs::create_dir_all(&parent).unwrap();
    let root = parent.join(&code);
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let retained = std::fs::File::open(&root).unwrap();
    let identity = retained.metadata().unwrap();
    for role in ["first", "restart"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", CHILD_TEST, "--nocapture"])
            .env(CHILD_ENV, "1")
            .env("TEST_CODE_P05_PRODUCER_ROLE", role)
            .env("STOCK_ENV_MODE", "test")
            .env("V10_DRY_RUN_PUSH", "1")
            .env("DURABLE_DELIVERY_TEST_CODE", &code)
            .env_remove("PUSH_LOG_DIR")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "P05 {role} child failed\nstdout={stdout}\nstderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("running 1 test"));
        assert!(
            stdout.contains(&format!("p05-real-binding-{role}-verified")),
            "zero-test or wrong child filter cannot pass"
        );
    }
    let counted = rusqlite::Connection::open_with_flags(
        root.join("durable_delivery.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let attempts: i64 = counted
        .query_row("SELECT COUNT(*) FROM delivery_attempts", [], |row| {
            row.get(0)
        })
        .unwrap();
    let results: i64 = counted
        .query_row("SELECT COUNT(*) FROM sink_results", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (attempts, results),
        (1, 1),
        "restart must not perform a second Test sink attempt"
    );
    drop(counted);
    let current = std::fs::symlink_metadata(&root).unwrap();
    assert!(current.is_dir() && current.dev() == identity.dev() && current.ino() == identity.ino());
    let final_retained = retained.metadata().unwrap();
    assert_eq!(
        (final_retained.dev(), final_retained.ino()),
        (identity.dev(), identity.ino())
    );
    std::fs::remove_dir_all(&root).unwrap();
}
