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

/// Runs the library's actual blocking workflow, the production v2 factory,
/// the real counted binding consumer and its isolated Test receipt adapter.
/// This does not exercise provider acquisition or presentation/governance gates.
#[tokio::test]
#[ignore = "exact isolated child invoked by the P05 producer restart test"]
async fn p05_counted_producer_real_binding_child() {
    use stock_analysis::monitor::prediction::{
        prepare_candidate_board, CandidateBoardPreparation, CandidateBoardPreparationRequest,
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
    let request = || {
        CandidateBoardPreparationRequest::new(
            "2026-09-23",
            "10:30",
            b"TEST_CODE exact production-binding card".to_vec(),
            vec![
                ("TEST_CODE_p05_a".to_owned(), 80.0),
                ("TEST_CODE_p05_b".to_owned(), 81.0),
            ],
        )
        .unwrap()
    };
    let CandidateBoardPreparation::Frozen {
        record,
        reused,
        save_report,
    } = prepare_candidate_board(request()).await.unwrap()
    else {
        panic!("real Strong producer must freeze its actual committed rows");
    };
    assert_eq!(reused, role == "restart");
    assert_eq!(save_report.is_none(), role == "restart");
    assert_eq!(database.count_predictions().unwrap(), 2);
    let binding = build_candidate_board_counted_binding_v2(&record).unwrap();
    assert_eq!(
        binding.source_binding_canonical(),
        record.source_canonical()
    );
    assert_eq!(
        binding.source_evidence_fingerprint(),
        record.source_sha256()
    );
    assert_eq!(binding.delivery_subject_hash(), record.source_sha256());
    assert_eq!(
        binding.schedule_occurrence_identity(),
        record.occurrence_identity()
    );
    assert!(binding.task_binding().is_none());
    assert!(!binding.retry_authorized());
    let text = String::from_utf8(record.rendered_bytes().to_vec()).unwrap();
    let outcome = crate::durable_delivery_runtime::deliver_counted_binding(
        binding,
        crate::notify::PushKind::CandidateBoard,
        text,
        None,
    )
    .await;
    assert_eq!(outcome, crate::notify::PushOutcome::Pushed);

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
fn p05_counted_producer_real_binding_restarts_without_second_sink_or_prediction_rows() {
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
