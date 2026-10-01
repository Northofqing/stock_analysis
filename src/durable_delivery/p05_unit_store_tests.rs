use super::*;
use crate::database::p05_prediction_freeze::FrozenCandidateBoardV2;
use crate::durable_delivery::tests::Fixture;
use crate::monitor::prediction::{
    prepare_candidate_board_on, CandidateBoardPreparation, CandidateBoardPreparationRequest,
};
use chrono::TimeZone;
use std::sync::Barrier;

const DATE: &str = "2026-09-23";
fn at(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    shanghai_offset()
        .with_ymd_and_hms(2026, 9, 23, hour, minute, second)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}
fn operational() -> (tempfile::TempDir, DatabaseManager) {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(
        dir.path().join("TEST_CODE_p05_unit_operational.db"),
    )
    .unwrap();
    (dir, db)
}
fn observed(strong: bool, second: u32) -> P05ObservedDraftInput {
    let entries = vec![CandidateEntry {
        code: "TEST_CODE_P05_STRONG".into(),
        name: "TEST_CODE genuine observed row".into(),
        sources: vec![CandidateSource::StockPick],
        tier: if strong {
            EvidenceTier::Strong
        } else {
            EvidenceTier::Reference
        },
        evidence: vec!["TEST_CODE local observation; no provider authority".into()],
        current_price: Some(12.0),
        change_pct: Some(f64::from_bits(0x7ff8000000000042)),
        heat_score: Some(80.0),
    }];
    let board = crate::opportunity::candidate_panel::format_candidate_board(&entries).into_bytes();
    P05ObservedDraftInput::from_observed(
        at(9, 21, second).with_timezone(&shanghai_offset()),
        &entries,
        P05ObservedSourceBytes {
            quote_evidence: Some(b"TEST_CODE raw observed quote bytes".to_vec()),
            statistics_evidence: None,
            p5_file_witnesses: b"TEST_CODE file witnesses".to_vec(),
            p5_candidate_refs: b"TEST_CODE refs".to_vec(),
            chain_query: b"TEST_CODE query".to_vec(),
            chain_candidate_refs: b"TEST_CODE chain refs".to_vec(),
        },
        b"TEST_CODE original A02 observed card; no physical acceptance".to_vec(),
        board,
    )
    .unwrap()
}
fn origin(fixture: &Fixture, db: &DatabaseManager) -> String {
    fixture
        .coordinator
        .initialize_prospective_p05_family_at(db, at(9, 19, 0), fixture.database_path.parent())
        .unwrap()
}
fn draft(fixture: &Fixture, db: &DatabaseManager, strong: bool) -> StoredP05Draft {
    origin(fixture, db);
    fixture
        .coordinator
        .store_p05_observed_draft_with_clock(&observed(strong, 3), Some(at(9, 24, 59)))
        .unwrap()
}
fn freeze(db: &DatabaseManager, draft: &StoredP05Draft) -> FrozenCandidateBoardV2 {
    let request = CandidateBoardPreparationRequest::new(
        DATE,
        "09:21",
        draft.board_rendered_bytes().to_vec(),
        draft.strong_samples(),
    )
    .unwrap();
    match prepare_candidate_board_on(db, &request).unwrap() {
        CandidateBoardPreparation::Frozen { record, .. } => record,
        _ => panic!("actual Strong helper must freeze"),
    }
}

#[test]
fn p05_unit_store_roundtrip_actual_freeze_started_restart_original_children_and_nan_bits() {
    let fixture = Fixture::new("P05_UNIT_ROUNDTRIP");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, true);
    let decoded: DraftCanonical = decode(draft.canonical_bytes()).unwrap();
    assert_eq!(
        decoded.input.entries[0].change_bits,
        Some(0x7ff8000000000042)
    );
    assert!(matches!(
        decoded.invalidated,
        InvalidatedPreparation::ProspectiveNoPreviousV2Baseline
    ));
    let start = fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    assert!(start.may_start_sampling());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_intents"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
    let frozen = freeze(&db, &draft);
    let intent = fixture
        .coordinator
        .complete_p05_unit_intent_on(&draft, &start, &db)
        .unwrap();
    assert_eq!(intent.children().len(), 2);
    let expected =
        crate::p05_candidate_board_link::frozen_candidate_board_envelope(&frozen).unwrap();
    assert_eq!(intent.children()[1].1, expected.canonical_bytes().unwrap());
    assert_eq!(
        fixture.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        3
    );
    assert_eq!(
        fixture.query_i64("SELECT baseline_revision FROM p05_baseline_heads"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0,
        "local intent is not counted admission"
    );
    let reopened = fixture.second_coordinator("P05_UNIT_RESTART");
    let restored = reopened.read_p05_unit_draft(DATE).unwrap().unwrap();
    assert_eq!(restored, draft);
    let old_start = reopened.claim_p05_prediction_prepare(&restored).unwrap();
    assert!(!old_start.may_start_sampling());
    assert_eq!(old_start.identity(), start.identity());
    assert_eq!(
        reopened
            .complete_p05_unit_intent_on(&restored, &old_start, &db)
            .unwrap(),
        intent
    );
    assert_eq!(reopened.read_p05_unit_intent(DATE).unwrap(), Some(intent));
    assert_eq!(
        db.read_candidate_board_v2_freeze(&draft.board_occurrence().unwrap())
            .unwrap()
            .unwrap()
            .ordered_rows(),
        frozen.ordered_rows()
    );
}

#[test]
fn p05_unit_store_unknown_started_absent_freeze_never_mints_v1_or_reclaims_sampling() {
    let fixture = Fixture::new("P05_STARTED_UNKNOWN");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, true);
    let first = fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    assert!(first.may_start_sampling());
    drop(first);
    let reopened = fixture.second_coordinator("P05_STARTED_UNKNOWN");
    let started = reopened.claim_p05_prediction_prepare(&draft).unwrap();
    assert!(!started.may_start_sampling());
    assert!(reopened
        .complete_p05_unit_intent_on(&draft, &started, &db)
        .is_err());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_prediction_prepare_events"),
        1
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_intents"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
        0
    );
    assert!(db
        .read_candidate_board_v2_freeze(&draft.board_occurrence().unwrap())
        .unwrap()
        .is_none());
}

#[test]
fn p05_unit_store_no_strong_actual_absent_read_is_explicit_unlinked_not_accepted() {
    let fixture = Fixture::new("P05_UNLINKED");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, false);
    let started = fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    let intent = fixture
        .coordinator
        .complete_p05_unit_intent_on(&draft, &started, &db)
        .unwrap();
    let data: IntentCanonical = decode(intent.canonical_bytes()).unwrap();
    assert!(matches!(
        data.prediction,
        PredictionObservation::UnlinkedNoStrong { .. }
    ));
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM sink_results"), 0);
    let retry = fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    assert!(!retry.may_start_sampling());
    assert_eq!(
        fixture
            .coordinator
            .complete_p05_unit_intent_on(&draft, &retry, &db)
            .unwrap(),
        intent
    );
}

#[test]
fn p05_unit_store_draft_race_returns_exact_winner_and_original_clock_outside_window() {
    let fixture = Fixture::new("P05_DRAFT_RACE");
    let (_dir, db) = operational();
    origin(&fixture, &db);
    let second = fixture.second_coordinator("P05_DRAFT_RACE");
    let barrier = Arc::new(Barrier::new(2));
    let first_coordinator = fixture.coordinator.clone();
    let left_barrier = barrier.clone();
    let left = std::thread::spawn(move || {
        left_barrier.wait();
        first_coordinator
            .store_p05_observed_draft_with_clock(&observed(true, 1), Some(at(9, 22, 0)))
            .unwrap()
    });
    let right = std::thread::spawn(move || {
        barrier.wait();
        second
            .store_p05_observed_draft_with_clock(&observed(false, 2), Some(at(9, 22, 0)))
            .unwrap()
    });
    let first = left.join().unwrap();
    let second = right.join().unwrap();
    assert_eq!(first, second);
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM p05_unit_drafts"), 1);
    let outside = fixture
        .coordinator
        .store_p05_observed_draft_with_clock(&observed(false, 59), Some(at(15, 30, 0)))
        .unwrap();
    assert_eq!(outside, first);
}

#[test]
fn p05_unit_store_started_race_has_one_sampling_claim_and_exact_original_event() {
    let fixture = Fixture::new("P05_STARTED_RACE");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, true);
    let first = fixture.coordinator.clone();
    let second = fixture.second_coordinator("P05_STARTED_RACE");
    let other_draft = draft.clone();
    let barrier = Arc::new(Barrier::new(2));
    let left_barrier = barrier.clone();
    let left = std::thread::spawn(move || {
        left_barrier.wait();
        first.claim_p05_prediction_prepare(&draft).unwrap()
    });
    let right = std::thread::spawn(move || {
        barrier.wait();
        second.claim_p05_prediction_prepare(&other_draft).unwrap()
    });
    let left = left.join().unwrap();
    let right = right.join().unwrap();
    assert_eq!(
        (left.may_start_sampling() as u8) + (right.may_start_sampling() as u8),
        1
    );
    assert_eq!(left.identity(), right.identity());
    assert_eq!(left.canonical, right.canonical);
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_prediction_prepare_events"),
        1
    );
    assert_eq!(
        fixture.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        2
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_intents"),
        0
    );
}

#[test]
fn p05_unit_store_capability_cannot_cross_attested_namespace() {
    let left = Fixture::new("P05_NAMESPACE_LEFT");
    let right = Fixture::new("P05_NAMESPACE_RIGHT");
    let (_dir, db) = operational();
    let draft = draft(&left, &db, true);
    assert!(right
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .is_err());
    let start = left
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    assert!(right
        .coordinator
        .complete_p05_unit_intent_on(&draft, &start, &db)
        .is_err());
    assert_eq!(
        right.query_i64("SELECT COUNT(*) FROM p05_prediction_prepare_events"),
        0
    );
}

#[test]
fn p05_unit_store_prospective_requires_verified_day_pre0920_and_absent_real_residue() {
    let fixture = Fixture::new("P05_PROSPECTIVE_GATES");
    let (_dir, db) = operational();
    assert!(fixture
        .coordinator
        .initialize_prospective_p05_family_at(&db, at(9, 20, 0), fixture.database_path.parent())
        .is_err());
    let closed = shanghai_offset()
        .with_ymd_and_hms(2026, 10, 2, 9, 19, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc);
    assert!(fixture
        .coordinator
        .initialize_prospective_p05_family_at(&db, closed, fixture.database_path.parent())
        .is_err());
    db.save_prediction_with_id(
        DATE,
        "2026-10-08",
        None,
        Some("TEST_CODE_P05_STRONG"),
        "up",
        80.0,
        Some("candidate-strong"),
        None,
        None,
    )
    .unwrap();
    assert!(fixture
        .coordinator
        .initialize_prospective_p05_family_at(&db, at(9, 19, 0), fixture.database_path.parent())
        .is_err());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_baseline_origins"),
        0
    );
}

#[test]
fn p05_unit_store_affected_legacy_decision_blocks_origin_but_unrelated_family_does_not() {
    for kind in [
        PushKind::AuctionRepush,
        PushKind::CandidateBoard,
        PushKind::CandidateInvalidated,
        PushKind::HoldingEvent,
    ] {
        let fixture = Fixture::new("P05_PROSPECTIVE_LEGACY_OWNER");
        let (_dir, db) = operational();
        let rendered = b"TEST_CODE existing legacy rendered bytes".to_vec();
        let source = if kind == PushKind::CandidateBoard {
            serde_json::to_vec(&serde_json::json!({
                "schema":"candidate-board-v1", "business_date":DATE,
                "rendered_sha256":sha256_hex(&rendered)
            }))
            .unwrap()
        } else {
            b"TEST_CODE existing legacy observational source".to_vec()
        };
        let hash = sha256_hex(&source);
        let envelope = DeliveryEnvelope::new(
            DATE,
            kind,
            super::super::super::model::DeliverySubKind::None,
            if kind == PushKind::CandidateInvalidated {
                "SSE:EQUITY:TEST_CODE_P05_LEGACY"
            } else {
                "GLOBAL"
            },
            if kind == PushKind::CandidateBoard {
                "candidate-board:2026-09-23:09:18"
            } else {
                "TEST_CODE legacy occurrence"
            },
            &hash,
            source,
            &hash,
            rendered,
            false,
            None,
        )
        .unwrap();
        fixture
            .coordinator
            .prepare(&envelope, 1, at(9, 18, 0))
            .unwrap();
        assert_eq!(
            fixture.query_i64("SELECT COUNT(*) FROM delivery_decisions"),
            1
        );
        let initialized = fixture.coordinator.initialize_prospective_p05_family_at(
            &db,
            at(9, 19, 0),
            fixture.database_path.parent(),
        );
        assert_eq!(
            initialized.is_ok(),
            kind == PushKind::HoldingEvent,
            "only affected family decisions block the new prospective origin"
        );
    }
}

#[test]
fn p05_unit_store_legacy_snapshot_exists_or_alias_blocks_prospective_without_adoption() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new("P05_LEGACY_ABSENCE");
    let (_dir, db) = operational();
    let parent = fixture.database_path.parent().unwrap();
    let dir = parent.join("candidate_board_snapshot");
    fs::create_dir(&dir).unwrap();
    let leaf = dir.join(format!("{DATE}.jsonl"));
    fs::write(&leaf, b"[\"TEST_CODE_P05_STRONG\"]\n").unwrap();
    assert!(fixture
        .coordinator
        .initialize_prospective_p05_family_at(&db, at(9, 19, 0), Some(parent))
        .is_err());
    fs::remove_file(&leaf).unwrap();
    symlink("missing-foreign-file", &leaf).unwrap();
    assert!(fixture
        .coordinator
        .initialize_prospective_p05_family_at(&db, at(9, 19, 0), Some(parent))
        .is_err());
    fs::remove_file(&leaf).unwrap();
    fs::remove_dir(&dir).unwrap();
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_baseline_origins"),
        0
    );
}

#[test]
fn p05_unit_store_fresh_clock_same_window_no_arbitrary_age_limit_and_absent_baseline_blocked() {
    let fixture = Fixture::new("P05_FRESH_CLOCK");
    let (_dir, db) = operational();
    let input = observed(false, 0);
    assert!(fixture
        .coordinator
        .store_p05_observed_draft_with_clock(&input, Some(at(9, 24, 59)))
        .is_err());
    origin(&fixture, &db);
    assert!(fixture
        .coordinator
        .store_p05_observed_draft_with_clock(&input, Some(at(9, 20, 59)))
        .is_err());
    assert!(fixture
        .coordinator
        .store_p05_observed_draft_with_clock(&input, Some(at(9, 25, 0)))
        .is_err());
    assert!(
        fixture
            .coordinator
            .store_p05_observed_draft_with_clock(&input, Some(at(9, 24, 59)))
            .is_ok(),
        "slow loader in original window remains allowed"
    );
}

#[test]
fn p05_unit_store_actual_conflicting_freeze_cannot_fallback_or_replace_first_card() {
    let fixture = Fixture::new("P05_FREEZE_CONFLICT");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, true);
    let started = fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    let wrong = CandidateBoardPreparationRequest::new(
        DATE,
        "09:21",
        b"TEST_CODE different original card".to_vec(),
        draft.strong_samples(),
    )
    .unwrap();
    prepare_candidate_board_on(&db, &wrong).unwrap();
    assert!(fixture
        .coordinator
        .complete_p05_unit_intent_on(&draft, &started, &db)
        .is_err());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_intents"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_children"),
        0
    );
}

#[test]
fn p05_unit_store_same_card_codes_different_actual_score_cannot_adopt_or_resave() {
    let fixture = Fixture::new("P05_SCORE_CONFLICT");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, true);
    let start = fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    assert_eq!(draft.strong_samples()[0].1.to_bits(), 80.0_f64.to_bits());
    let request = CandidateBoardPreparationRequest::new(
        DATE,
        "09:21",
        draft.board_rendered_bytes().to_vec(),
        vec![("TEST_CODE_P05_STRONG".into(), 81.0)],
    )
    .unwrap();
    let frozen = match prepare_candidate_board_on(&db, &request).unwrap() {
        CandidateBoardPreparation::Frozen { record, .. } => record,
        _ => panic!("actual same-card helper must freeze"),
    };
    assert_eq!(frozen.rendered_bytes(), draft.board_rendered_bytes());
    assert_eq!(frozen.ordered_rows()[0].code(), draft.strong_samples()[0].0);
    assert!(fixture
        .coordinator
        .complete_p05_unit_intent_on(&draft, &start, &db)
        .is_err());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_intents"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_children"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        2
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_prediction_prepare_events"),
        1
    );
    let same = db
        .read_candidate_board_v2_freeze(&draft.board_occurrence().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(same, frozen);
    let scores = db
        .read_p05_unit_freeze_with_scores(&draft.board_occurrence().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(scores.ordered_score_bits()[0].1, 81.0_f64.to_bits());
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        count: i64,
    }
    let mut connection = db.get_conn().unwrap();
    let count = diesel::RunQueryDsl::get_result::<Count>(
        diesel::sql_query("SELECT COUNT(*) AS count FROM prediction_tracker"),
        &mut *connection,
    )
    .unwrap();
    assert_eq!(
        count.count, 1,
        "conflicting freeze remains, with no second sample save"
    );
    drop(connection);
    assert!(!fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap()
        .may_start_sampling());
}

#[test]
fn p05_unit_store_test_clock_seams_reject_actual_production_environment_before_io() {
    let mut fixture = Fixture::new("P05_TEST_CLOCK_ENV");
    let (_dir, db) = operational();
    // This object retains only the fixture's Test DB; no production object/path is opened.
    let arc = fixture.coordinator.take().unwrap();
    let mut coordinator = Arc::try_unwrap(arc).ok().expect("one fixture owner");
    let original = coordinator.config.environment.clone();
    coordinator.config.environment = super::super::super::model::StoreEnvironment::Production;
    assert!(coordinator
        .initialize_prospective_p05_family_at(&db, at(9, 19, 0), fixture.database_path.parent())
        .is_err());
    assert!(coordinator
        .store_p05_observed_draft_with_clock(&observed(false, 0), Some(at(9, 21, 0)))
        .is_err());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_baseline_origins"),
        0
    );
    assert_eq!(fixture.query_i64("SELECT COUNT(*) FROM p05_unit_drafts"), 0);
    coordinator.config.environment = original;
    drop(coordinator);
}

#[test]
fn p05_unit_store_no_strong_cannot_bypass_existing_v2_freeze() {
    let fixture = Fixture::new("P05_NO_STRONG_V2_OWNER");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, false);
    let start = fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    let request = CandidateBoardPreparationRequest::new(
        DATE,
        "09:21",
        draft.board_rendered_bytes().to_vec(),
        vec![("TEST_CODE_P05_STRONG".into(), 80.0)],
    )
    .unwrap();
    prepare_candidate_board_on(&db, &request).unwrap();
    assert!(fixture
        .coordinator
        .complete_p05_unit_intent_on(&draft, &start, &db)
        .is_err());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_children"),
        0
    );
}

#[test]
fn p05_unit_store_complete_transaction_failure_retains_started_and_actual_operational_freeze() {
    let fixture = Fixture::new("P05_COMPLETE_ROLLBACK");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, true);
    let start = fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    let frozen = freeze(&db, &draft);
    fixture
        .coordinator
        .install_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
            || Err(invalid("TEST_CODE interrupt complete-intent before COMMIT")),
        )
        .unwrap();
    assert!(fixture
        .coordinator
        .complete_p05_unit_intent_on(&draft, &start, &db)
        .is_err());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_intents"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM p05_unit_children"),
        0
    );
    assert_eq!(
        fixture.query_i64("SELECT mutation_revision FROM p05_unit_heads"),
        2
    );
    assert_eq!(
        db.read_candidate_board_v2_freeze(&draft.board_occurrence().unwrap())
            .unwrap(),
        Some(frozen)
    );
    assert!(
        fixture
            .coordinator
            .complete_p05_unit_intent_on(&draft, &start, &db)
            .is_ok(),
        "read-only actual freeze resumes without first-save"
    );
}

#[test]
fn p05_unit_store_future_unit_revision_shape_is_narrow_but_current_validator_refuses_it() {
    let fixture = Fixture::new("P05_FUTURE_REVISION_CLOSED");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, false);
    let start = fixture
        .coordinator
        .claim_p05_prediction_prepare(&draft)
        .unwrap();
    fixture
        .coordinator
        .complete_p05_unit_intent_on(&draft, &start, &db)
        .unwrap();
    let connection = Connection::open(&fixture.database_path).unwrap();
    assert!(
        connection
            .execute("UPDATE p05_unit_heads SET mutation_revision=5", [])
            .is_err(),
        "future updates remain exact +1"
    );
    assert!(connection.execute("UPDATE p05_unit_heads SET mutation_revision=4,phase='Started',intent_identity=NULL",[]).is_err(),"future preparation cannot regress");
    assert_eq!(
        connection
            .execute("UPDATE p05_unit_heads SET mutation_revision=4", [])
            .unwrap(),
        1,
        "DDL reserves a same-intent next mutation for actual schema14 owners"
    );
    assert!(
        validate_rows(&connection).is_err(),
        "schema13 runtime does not grant future owner effects"
    );
    assert!(fixture.coordinator.read_p05_unit_intent(DATE).is_err());
}

#[test]
fn p05_unit_store_future_completed_baseline_shape_cas_is_reserved_and_current_reader_rejects() {
    let fixture = Fixture::new("P05_FUTURE_BASELINE_CLOSED");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, false);
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::super::schema::register_sha256_function(&connection).unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    let old_bytes = fixture.query_blob("SELECT origin_canonical FROM p05_baseline_origins");
    let mut value: serde_json::Value = serde_json::from_slice(&old_bytes).unwrap();
    value["kind"] = json!("CompletedUnitV2Baseline");
    value["completed_unit_identity"] = json!(draft.identity());
    value["completed_receipt_identity"] = json!("0".repeat(64));
    value["accepted_physical_refs"] = json!([84, 69, 83, 84]);
    let bytes = serde_json::to_vec(&value).unwrap();
    let image = preimage("p05-prospective-origin-v1", &bytes);
    let id = sha256_hex(&image);
    // Deliberately SQL shape only: this is not a verified receipt or completion.
    let insert="INSERT INTO p05_baseline_origins(origin_identity,family,business_date,origin_kind,completed_unit_identity,completed_receipt_identity,accepted_physical_refs,origin_canonical,origin_sha256,origin_preimage) VALUES(?1,?2,?3,'CompletedUnitV2Baseline',?4,?5,?6,?7,?8,?9)";
    assert!(
        connection
            .execute(
                insert,
                params![
                    id,
                    FAMILY,
                    DATE,
                    draft.identity(),
                    Option::<String>::None,
                    b"TEST".to_vec(),
                    bytes,
                    sha256_hex(&bytes),
                    image
                ]
            )
            .is_err(),
        "NULL does not pass the Completed shape branch"
    );
    connection
        .execute(
            insert,
            params![
                id,
                FAMILY,
                DATE,
                draft.identity(),
                "0".repeat(64),
                b"TEST".to_vec(),
                bytes,
                sha256_hex(&bytes),
                image
            ],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "UPDATE p05_baseline_heads SET baseline_revision=2,origin_identity=?1",
                [&id]
            )
            .is_err(),
        "baseline CAS cannot skip revisions"
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE p05_baseline_heads SET baseline_revision=1,origin_identity=?1",
                [&id]
            )
            .unwrap(),
        1,
        "first Completed may replace same-day prospective origin in future schema14"
    );
    assert!(
        connection
            .execute(
                "UPDATE p05_baseline_heads SET baseline_revision=2,origin_identity=?1",
                [&id]
            )
            .is_err(),
        "same immutable origin cannot be advanced twice"
    );
    assert!(load_origin(&connection, &id).is_err());
    assert!(
        validate_rows(&connection).is_err(),
        "current13 must reject even matching hashes of non-authoritative Completed columns"
    );
    assert_eq!(connection.query_row("SELECT origin_canonical FROM p05_baseline_origins WHERE origin_kind='ProspectiveNoPreviousV2Baseline'",[],|r|r.get::<_,Vec<u8>>(0)).unwrap(),old_bytes);
}

#[test]
fn p05_unit_store_closed_codec_unknown_completed_and_tampered_bytes_fail() {
    let fixture = Fixture::new("P05_CLOSED_READER");
    let (_dir, db) = operational();
    let draft = draft(&fixture, &db, false);
    let mut value: serde_json::Value = serde_json::from_slice(draft.canonical_bytes()).unwrap();
    value["completed"] = json!(true);
    assert!(decode::<DraftCanonical>(&serde_json::to_vec(&value).unwrap()).is_err());
    value.as_object_mut().unwrap().remove("completed");
    value["baseline_kind"] = json!("PhysicalAccepted");
    let fake = serde_json::to_vec(&value).unwrap();
    let image = preimage("p05-unit-draft-v1", &fake);
    let id = sha256_hex(&image);
    let connection = Connection::open(&fixture.database_path).unwrap();
    super::super::super::schema::register_sha256_function(&connection).unwrap();
    assert!(
        connection
            .execute("UPDATE p05_unit_drafts SET draft_canonical=?1", [&fake])
            .is_err(),
        "immutable row cannot be rewritten"
    );
    assert!(
        connection
            .execute(
                "INSERT OR REPLACE INTO p05_unit_drafts SELECT * FROM p05_unit_drafts",
                []
            )
            .is_err(),
        "recursive_triggers OFF still cannot replace"
    );
    let trigger_sql: String = connection
        .query_row(
            "SELECT sql FROM main.sqlite_master WHERE name='p05_unit_drafts_no_update'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    connection
        .execute_batch("DROP TRIGGER p05_unit_drafts_no_update; PRAGMA foreign_keys=OFF;")
        .unwrap();
    connection.execute("UPDATE p05_unit_drafts SET draft_identity=?1,draft_canonical=?2,draft_sha256=?3,draft_preimage=?4",params![id,fake,sha256_hex(&fake),image]).unwrap();
    connection.execute_batch(&trigger_sql).unwrap();
    assert!(load_draft(&connection,DATE,fixture.coordinator.p05_namespace().unwrap()).is_err(),"matching raw hashes cannot replace the closed baseline kind even before the FK reader gate");
    assert!(
        fixture.coordinator.read_p05_unit_draft(DATE).is_err(),
        "matching hashes cannot authorize changed closed policy"
    );
}

#[test]
fn p05_unit_store_schema13_temp_shadow_fails_before_any_local_operation() {
    let fixture = Fixture::new("P05_SCHEMA13_TEMP_SHADOW");
    assert!(fixture
        .coordinator
        .with_connection(|connection| {
            connection.execute_batch("CREATE TEMP TABLE p05_unit_drafts(foreign_payload BLOB);")?;
            Ok(())
        })
        .is_err());
    assert!(fixture
        .coordinator
        .read_p05_unit_draft("2026-09-23")
        .is_err());
    assert_eq!(
        fixture.query_i64("SELECT COUNT(*) FROM main.p05_unit_drafts"),
        0
    );
}
