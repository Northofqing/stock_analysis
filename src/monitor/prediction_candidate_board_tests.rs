use super::*;
use diesel::RunQueryDsl;

const DATE: &str = "2026-09-23";
const OCCURRENCE: &str = "candidate-board:2026-09-23:10:30";

fn private_db() -> (tempfile::TempDir, DatabaseManager) {
    let directory = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(directory.path().join("TEST_CODE_p05_producer.db"))
            .unwrap();
    (directory, db)
}

fn request(card: &[u8], codes: &[&str]) -> CandidateBoardPreparationRequest {
    CandidateBoardPreparationRequest::new(
        DATE,
        "10:30",
        card.to_vec(),
        codes
            .iter()
            .map(|code| ((*code).to_owned(), 80.0))
            .collect(),
    )
    .unwrap()
}

fn frozen(prepared: CandidateBoardPreparation) -> (FrozenCandidateBoardV2, bool) {
    let CandidateBoardPreparation::Frozen { record, reused, .. } = prepared else {
        panic!("Strong production helper must return a committed freeze");
    };
    (record, reused)
}

#[test]
fn p05_counted_producer_commits_real_rows_and_reopens_without_resaving() {
    let (directory, db) = private_db();
    let request = request(
        b"TEST_CODE exact Strong card",
        &["TEST_CODE_p05_a", "TEST_CODE_p05_b"],
    );
    let (first, reused) = frozen(prepare_candidate_board_on(&db, &request).unwrap());
    assert!(!reused);
    assert_eq!(db.count_predictions().unwrap(), 2);
    assert_eq!(first.occurrence_identity(), OCCURRENCE);
    assert_eq!(first.target_date(), "2026-10-08");
    assert_eq!(first.rendered_bytes(), request.rendered_bytes);
    for row in first.ordered_rows() {
        let actual = db.get_prediction_by_code_date(row.code(), DATE).unwrap();
        assert_eq!(i64::from(actual.id), row.prediction_row_id());
        assert_eq!(actual.target_date, first.target_date());
    }
    drop(db);
    let reopened =
        DatabaseManager::open_isolated_for_test(directory.path().join("TEST_CODE_p05_producer.db"))
            .unwrap();
    let replay = prepare_candidate_board_on_with_save(&reopened, &request, |_, _, _, _| {
        panic!("exact replay must not call the save port");
    })
    .unwrap();
    let CandidateBoardPreparation::Frozen {
        record,
        reused,
        save_report,
    } = replay
    else {
        panic!("reopen must retain the committed card");
    };
    assert!(reused);
    assert!(save_report.is_none());
    assert_eq!(record, first);
    assert_eq!(reopened.count_predictions().unwrap(), 2);
}

#[test]
fn p05_counted_producer_blocks_partial_commit_and_retains_save_report() {
    let (_directory, db) = private_db();
    diesel::sql_query(
        "CREATE TRIGGER TEST_CODE_p05_fail_second BEFORE INSERT ON prediction_tracker
         WHEN NEW.stock_code='TEST_CODE_p05_b'
         BEGIN SELECT RAISE(ABORT,'TEST_CODE intentional second-row failure'); END;",
    )
    .execute(&mut *db.get_conn().unwrap())
    .unwrap();
    let error = prepare_candidate_board_on(
        &db,
        &request(
            b"TEST_CODE partial card",
            &["TEST_CODE_p05_a", "TEST_CODE_p05_b"],
        ),
    )
    .unwrap_err();
    assert_eq!(error.reason(), "p05_sample_save_incomplete");
    let report = error.save_report().unwrap();
    assert_eq!((report.attempted, report.saved, report.unknown), (2, 1, 0));
    assert_eq!(report.failures.len(), 1);
    assert!(!report.is_complete());
    assert_eq!(
        db.count_predictions().unwrap(),
        1,
        "the committed first sample survives"
    );
    assert!(db
        .read_candidate_board_v2_freeze(OCCURRENCE)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn p05_counted_producer_worker_unknown_does_not_erase_real_commits() {
    let (_directory, db) = private_db();
    let db = std::sync::Arc::new(db);
    let worker_db = std::sync::Arc::clone(&db);
    let request = request(
        b"TEST_CODE worker card",
        &["TEST_CODE_p05_a", "TEST_CODE_p05_b"],
    );
    let worker = tokio::task::spawn_blocking(move || {
        prepare_candidate_board_on_with_save(&worker_db, &request, |db, date, target, samples| {
            let saved = save_candidate_samples(db, date, target, samples);
            assert!(saved.is_complete());
            panic!("TEST_CODE worker lost its save result after real commits");
        })
    });
    let error = collect_candidate_board_prepare_worker(worker, 2)
        .await
        .unwrap_err();
    assert_eq!(error.reason(), "p05_prepare_worker_unknown");
    let report = error.save_report().unwrap();
    assert_eq!((report.attempted, report.saved, report.unknown), (2, 0, 2));
    assert!(report.worker_error.is_some());
    assert_eq!(db.count_predictions().unwrap(), 2);
    assert!(db
        .read_candidate_board_v2_freeze(OCCURRENCE)
        .unwrap()
        .is_none());
}

#[test]
fn p05_counted_producer_complete_counts_with_unknown_are_blocked_and_not_logged_complete() {
    let (_directory, db) = private_db();
    let request = request(b"TEST_CODE unknown card", &["TEST_CODE_p05_a"]);
    let error = prepare_candidate_board_on_with_save(&db, &request, |db, date, target, samples| {
        let mut report = save_candidate_samples(db, date, target, samples);
        assert!(report.is_complete());
        report.unknown = 1;
        assert!(
            !report.is_complete(),
            "unknown cannot be logged as a complete save"
        );
        report
    })
    .unwrap_err();
    assert_eq!(error.reason(), "p05_sample_save_incomplete");
    assert_eq!(error.save_report().unwrap().saved, 1);
    assert_eq!(db.count_predictions().unwrap(), 1);
    assert!(db
        .read_candidate_board_v2_freeze(OCCURRENCE)
        .unwrap()
        .is_none());
}

#[test]
fn p05_counted_producer_unlinked_no_strong_cannot_bypass_existing_freeze_or_drift() {
    let (_directory, db) = private_db();
    let no_strong = request(b"TEST_CODE same card", &[]);
    assert!(matches!(
        prepare_candidate_board_on(&db, &no_strong).unwrap(),
        CandidateBoardPreparation::UnlinkedNoStrong
    ));
    assert_eq!(db.count_predictions().unwrap(), 0);
    let strong = request(
        b"TEST_CODE same card",
        &["TEST_CODE_p05_a", "TEST_CODE_p05_b"],
    );
    let (first, _) = frozen(prepare_candidate_board_on(&db, &strong).unwrap());
    for changed in [
        no_strong,
        request(
            b"TEST_CODE changed card",
            &["TEST_CODE_p05_a", "TEST_CODE_p05_b"],
        ),
        request(
            b"TEST_CODE same card",
            &["TEST_CODE_p05_b", "TEST_CODE_p05_a"],
        ),
    ] {
        assert_eq!(
            prepare_candidate_board_on(&db, &changed)
                .unwrap_err()
                .reason(),
            "p05_frozen_card_drift"
        );
    }
    assert_eq!(db.count_predictions().unwrap(), 2);
    assert_eq!(
        db.read_candidate_board_v2_freeze(OCCURRENCE).unwrap(),
        Some(first)
    );
}

#[test]
fn p05_counted_producer_rejects_bad_request_before_any_rows() {
    let (_directory, db) = private_db();
    for (date, hhmm) in [
        ("not-a-date", "10:30"),
        ("2026-9-23", "10:30"),
        ("2026-09-23", "10:3"),
        ("2026-09-23", "25:00"),
        ("2026-10-04", "10:30"),
        ("2027-01-04", "10:30"),
        ("2026-12-31", "10:30"),
    ] {
        assert!(CandidateBoardPreparationRequest::new(
            date,
            hhmm,
            b"TEST_CODE invalid request".to_vec(),
            vec![("TEST_CODE_p05_a".to_owned(), 80.0)],
        )
        .is_err());
    }
    assert!(CandidateBoardPreparationRequest::new(DATE, "10:30", vec![0xff], Vec::new()).is_err());
    assert!(CandidateBoardPreparationRequest::new(
        DATE,
        "10:30",
        b"TEST_CODE duplicate".to_vec(),
        vec![
            ("TEST_CODE_p05_a".to_owned(), 80.0),
            ("TEST_CODE_p05_a".to_owned(), 81.0)
        ],
    )
    .is_err());
    assert_eq!(db.count_predictions().unwrap(), 0);
}

fn race(conflicting_card: bool) {
    let (directory, first_db) = private_db();
    let second_db =
        DatabaseManager::open_isolated_for_test(directory.path().join("TEST_CODE_p05_producer.db"))
            .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = [first_db, second_db]
        .into_iter()
        .enumerate()
        .map(|(index, db)| {
            let barrier = std::sync::Arc::clone(&barrier);
            let card: &[u8] = if index == 1 && conflicting_card {
                b"TEST_CODE conflicting card"
            } else {
                b"TEST_CODE same card"
            };
            let request = request(card, &["TEST_CODE_p05_a", "TEST_CODE_p05_b"]);
            std::thread::spawn(move || {
                prepare_candidate_board_on_with_save(&db, &request, |db, date, target, samples| {
                    // Both real DB readers observed absent before either starts saving.
                    barrier.wait();
                    save_candidate_samples(db, date, target, samples)
                })
            })
        })
        .collect();
    let mut results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    let reader =
        DatabaseManager::open_isolated_for_test(directory.path().join("TEST_CODE_p05_producer.db"))
            .unwrap();
    let winner = reader
        .read_candidate_board_v2_freeze(OCCURRENCE)
        .unwrap()
        .unwrap();
    assert_eq!(
        reader.count_predictions().unwrap(),
        4,
        "race loser rows remain observation samples"
    );
    if conflicting_card {
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        for result in results {
            match result {
                Ok(prepared) => assert_eq!(frozen(prepared).0, winner),
                Err(error) => {
                    assert_eq!(error.reason(), "p05_freeze_failed");
                    assert_eq!(error.save_report().unwrap().saved, 2);
                }
            }
        }
    } else {
        let (second, second_reused) = frozen(results.pop().unwrap().unwrap());
        let (first, first_reused) = frozen(results.pop().unwrap().unwrap());
        assert_eq!(first, winner);
        assert_eq!(second, winner);
        assert_ne!(first_reused, second_reused);
    }
}

#[test]
fn p05_counted_producer_concurrent_identical_save_uses_one_winners_original_members() {
    race(false);
}

#[test]
fn p05_counted_producer_concurrent_card_conflict_preserves_one_freeze() {
    race(true);
}

#[test]
fn p05_counted_producer_tampered_freeze_does_not_resave_or_downgrade() {
    let (_directory, db) = private_db();
    let request = request(b"TEST_CODE frozen card", &["TEST_CODE_p05_a"]);
    prepare_candidate_board_on(&db, &request).unwrap();
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("DROP TRIGGER trg_candidate_board_prediction_freeze_v2_no_update")
        .execute(&mut *conn)
        .unwrap();
    diesel::sql_query("UPDATE candidate_board_prediction_freeze_v2 SET rendered_bytes=X'01'")
        .execute(&mut *conn)
        .unwrap();
    drop(conn);
    assert_eq!(
        prepare_candidate_board_on(&db, &request)
            .unwrap_err()
            .reason(),
        "p05_freeze_read_failed"
    );
    assert_eq!(db.count_predictions().unwrap(), 1);
}
