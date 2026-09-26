use super::*;
use diesel::Connection;

fn now() -> DateTime<Utc> {
    "2026-09-27T08:00:00Z".parse().unwrap()
}

fn open(path: &std::path::Path) -> SqliteConnection {
    use diesel::connection::SimpleConnection;
    let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    conn.batch_execute("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")
        .unwrap();
    conn
}

#[test]
fn task8_discover_reopen_review_confirm_uses_original_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_review.sqlite");
    let mut conn = open(&path);
    super::super::daily_change_confirmation::create_schema(&mut conn).unwrap();
    super::super::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
    let evidence = crate::data_gateway::historical_bars::qualified_review_fixture();
    let original = discover_on_conn(&mut conn, &evidence, now())
        .expect("qualified evidence must persist pending");
    assert_eq!(original.status, "Pending");
    assert_eq!(original.expires_at, now() + chrono::Duration::days(7));
    drop(conn);
    let mut reopened = open(&path);
    assert_eq!(
        review_on_conn(&mut reopened, &original.candidate_id, now()).unwrap(),
        original
    );
    let confirmed = decide_on_conn(
        &mut reopened,
        &original.candidate_id,
        &original.evidence_token,
        ReviewDecision::Confirm,
        "TEST_CODE_operator",
        "TEST_CODE_reviewed",
        now(),
    )
    .unwrap();
    assert_eq!(confirmed.status, "Confirmed");
    assert!(
        super::super::daily_change_confirmation::has_exact_daily_change_confirmation_on_conn(
            &mut reopened,
            &original.snapshot.query
        )
        .unwrap()
    );
    assert_eq!(confirmed.snapshot, original.snapshot);
}

fn fixture() -> crate::data_gateway::historical_bars::QualifiedDailyChangeDiscovery {
    crate::data_gateway::historical_bars::qualified_review_fixture()
}
fn initialized(path: &std::path::Path) -> SqliteConnection {
    let mut conn = open(path);
    super::super::daily_change_confirmation::create_schema(&mut conn).unwrap();
    super::super::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
    conn
}

#[test]
fn task8_cli_runner_offline_scope_output_failure_and_commit_retry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_cli.sqlite");
    let mut conn = initialized(&path);
    let original = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    drop(conn);
    let mut conn = open(&path);
    let select = || ReviewSelection {
        candidate_id: &original.candidate_id,
        code: Some("TEST_CODE_300005"),
        previous_date: Some(original.snapshot.query.previous_date),
        current_date: Some(original.snapshot.query.current_date),
    };
    let confirm = || ReviewAction::Decide {
        token: &original.evidence_token,
        decision: ReviewDecision::Confirm,
        operator: "TEST_CODE_cli",
        reason: "TEST_CODE_reviewed",
    };
    let mut output = Vec::new();
    assert_eq!(
        run_review_action_on_conn(
            &mut conn,
            select(),
            ReviewAction::Review,
            now(),
            &mut output
        )
        .unwrap(),
        original
    );
    assert_eq!(
        serde_json::from_slice::<CandidateReview>(&output).unwrap(),
        original
    );
    let wrong = ReviewSelection {
        code: Some("TEST_CODE_WRONG"),
        ..select()
    };
    assert!(matches!(
        run_review_action_on_conn(&mut conn, wrong, confirm(), now(), &mut Vec::new()),
        Err(ReviewError::ScopeMismatch)
    ));
    let wrong_date = ReviewSelection {
        current_date: Some(original.snapshot.query.current_date.succ_opt().unwrap()),
        ..select()
    };
    assert!(matches!(
        run_review_action_on_conn(&mut conn, wrong_date, confirm(), now(), &mut Vec::new()),
        Err(ReviewError::ScopeMismatch)
    ));
    struct BrokenAfterFlush {
        allowed_flushes: usize,
    }
    impl std::io::Write for BrokenAfterFlush {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.allowed_flushes == 0 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "TEST_CODE_output_fault",
                ))
            } else {
                self.allowed_flushes -= 1;
                Ok(())
            }
        }
    }
    assert!(matches!(
        run_review_action_on_conn(
            &mut conn,
            select(),
            confirm(),
            now(),
            &mut BrokenAfterFlush { allowed_flushes: 0 }
        ),
        Err(ReviewError::Output(_))
    ));
    assert_eq!(
        review_on_conn(&mut conn, &original.candidate_id, now())
            .unwrap()
            .status,
        "Pending"
    );
    assert!(matches!(
        run_review_action_on_conn(
            &mut conn,
            select(),
            confirm(),
            now(),
            &mut BrokenAfterFlush { allowed_flushes: 1 }
        ),
        Err(ReviewError::Output(_))
    ));
    assert_eq!(
        review_on_conn(&mut conn, &original.candidate_id, now())
            .unwrap()
            .status,
        "Confirmed"
    );
    let events = count(&mut conn, "daily_change_review_event");
    let recovered =
        run_review_action_on_conn(&mut conn, select(), confirm(), now(), &mut Vec::new()).unwrap();
    assert_eq!(recovered.status, "Confirmed");
    assert_eq!(count(&mut conn, "daily_change_review_event"), events);
    assert!(matches!(
        run_review_action_on_conn(
            &mut conn,
            select(),
            ReviewAction::Decide {
                token: &original.evidence_token,
                decision: ReviewDecision::Reject,
                operator: "TEST_CODE_cli",
                reason: "TEST_CODE_reviewed"
            },
            now(),
            &mut Vec::new()
        ),
        Err(ReviewError::Conflict)
    ));
    let next = discover_on_conn(&mut conn, &different_fact(), now()).unwrap();
    let rejected = run_review_action_on_conn(
        &mut conn,
        ReviewSelection {
            candidate_id: &next.candidate_id,
            code: None,
            previous_date: None,
            current_date: None,
        },
        ReviewAction::Decide {
            token: &next.evidence_token,
            decision: ReviewDecision::Reject,
            operator: "TEST_CODE_cli",
            reason: "TEST_CODE_rejected",
        },
        now(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(rejected.status, "Rejected");
    assert_eq!(
        count(&mut conn, "daily_change_confirmation_v2"),
        1,
        "Reject creates no positive alias"
    );
}
fn different_fact() -> crate::data_gateway::historical_bars::QualifiedDailyChangeDiscovery {
    fixture().with_test_mutation(|s| {
        s.query.current_close = "14".into();
        s.query.calculated_pct = format!("{:.12}", (14.0 / 18.59 - 1.0) * 100.0);
        s.query.daily_batch_id = "TEST_CODE_B".into();
    })
}

#[test]
fn task8_batch_rotation_observation_preserves_first_snapshot_and_token() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_rotation.sqlite");
    let mut conn = initialized(&path);
    let first = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    let later = fixture().with_test_mutation(|s| {
        s.query.daily_batch_id = "TEST_CODE_batch2".into();
        s.query.lifecycle_batch_id = "TEST_CODE_lifecycle2".into();
        s.raw_evidence = serde_json::json!({"fixture":"TEST_CODE_later_acquisition"});
    });
    let second = discover_on_conn(&mut conn, &later, now() + chrono::Duration::hours(1)).unwrap();
    assert_eq!(second.candidate_id, first.candidate_id);
    assert_eq!(second.evidence_token, first.evidence_token);
    assert_eq!(second.snapshot, first.snapshot);
    assert_eq!(second.expires_at, first.expires_at);
    assert_eq!(second.observations, vec![later.snapshot().clone()]);
    drop(conn);
    let mut reopened = open(&path);
    assert_eq!(
        discover_on_conn(&mut reopened, &later, now() + chrono::Duration::hours(2)).unwrap(),
        second
    );
}

#[test]
fn task8_revision_a_b_a_and_rule_change_never_resurrect_old_token() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = initialized(&dir.path().join("TEST_CODE_revision.sqlite"));
    let a = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    let b = discover_on_conn(&mut conn, &different_fact(), now()).unwrap();
    assert_eq!(b.revision, 2);
    assert_ne!(a.candidate_id, b.candidate_id);
    let again = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    assert_eq!(again.revision, 3);
    assert_ne!(again.evidence_token, a.evidence_token);
    assert!(matches!(
        decide_on_conn(
            &mut conn,
            &a.candidate_id,
            &a.evidence_token,
            ReviewDecision::Confirm,
            "TEST_CODE_op",
            "TEST_CODE_reason",
            now()
        ),
        Err(ReviewError::Superseded)
    ));
    let changed_rule =
        fixture().with_test_mutation(|s| s.rule_version = "TEST_CODE_rule_revision2".into());
    let revision4 = discover_on_conn(&mut conn, &changed_rule, now()).unwrap();
    assert_eq!(revision4.revision, 4);
    assert_ne!(revision4.evidence_token, again.evidence_token);
    assert_eq!(
        review_on_conn(&mut conn, &again.candidate_id, now())
            .unwrap()
            .status,
        "Superseded"
    );
}

#[test]
fn task8_same_provider_batch_reobserved_with_new_request_metadata_is_observation() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = initialized(&dir.path().join("TEST_CODE_reobserved.sqlite"));
    let first = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    let later = fixture().with_test_mutation(|s| {
        s.raw_evidence =
            serde_json::json!({"fixture":"TEST_CODE_same_provider_batch_new_request_time"})
    });
    let observed = discover_on_conn(&mut conn, &later, now() + chrono::Duration::hours(1))
        .expect("a new qualified acquisition does not conflict with the old immutable snapshot");
    assert_eq!(observed.evidence_token, first.evidence_token);
    assert_eq!(observed.snapshot, first.snapshot);
    assert_eq!(observed.observations, vec![later.snapshot().clone()]);
    assert_eq!(
        discover_on_conn(&mut conn, &later, now() + chrono::Duration::hours(2)).unwrap(),
        observed
    );
}

#[test]
fn task8_expiry_is_persisted_and_explicit_renewal_appends_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_expiry.sqlite");
    let mut conn = initialized(&path);
    let original = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    let deadline = now() + chrono::Duration::days(7);
    drop(conn);
    let mut conn = open(&path);
    assert_eq!(
        review_on_conn(
            &mut conn,
            &original.candidate_id,
            deadline - chrono::Duration::nanoseconds(1)
        )
        .unwrap()
        .status,
        "Pending"
    );
    assert_eq!(
        review_on_conn(&mut conn, &original.candidate_id, deadline)
            .unwrap()
            .status,
        "Expired"
    );
    assert_eq!(
        discover_on_conn(&mut conn, &fixture(), deadline)
            .unwrap()
            .expires_at,
        deadline
    );
    assert!(matches!(
        decide_on_conn(
            &mut conn,
            &original.candidate_id,
            &original.evidence_token,
            ReviewDecision::Confirm,
            "TEST_CODE_op",
            "TEST_CODE_reason",
            deadline
        ),
        Err(ReviewError::Expired)
    ));
    let renewed = renew_on_conn(
        &mut conn,
        &original.candidate_id,
        &original.evidence_token,
        deadline,
    )
    .unwrap();
    assert_eq!(renewed.revision, 2);
    assert_eq!(renewed.expires_at, deadline + chrono::Duration::days(7));
    assert_ne!(renewed.evidence_token, original.evidence_token);
    assert_eq!(
        renew_on_conn(
            &mut conn,
            &original.candidate_id,
            &original.evidence_token,
            deadline + chrono::Duration::hours(1)
        )
        .unwrap(),
        renewed
    );
    assert!(matches!(
        decide_on_conn(
            &mut conn,
            &original.candidate_id,
            &original.evidence_token,
            ReviewDecision::Confirm,
            "TEST_CODE_op",
            "TEST_CODE_reason",
            deadline
        ),
        Err(ReviewError::Superseded)
    ));
}

#[test]
fn task8_v3_latest_decision_overrides_old_alias_but_pure_legacy_stays_valid() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = initialized(&dir.path().join("TEST_CODE_admission.sqlite"));
    let first = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    assert!(!admit_on_conn(&mut conn, fixture().snapshot()).unwrap());
    decide_on_conn(
        &mut conn,
        &first.candidate_id,
        &first.evidence_token,
        ReviewDecision::Confirm,
        "TEST_CODE_original_op",
        "TEST_CODE_original_reason",
        now(),
    )
    .unwrap();
    assert!(admit_on_conn(&mut conn, fixture().snapshot()).unwrap());
    assert!(!admit_on_conn(&mut conn, different_fact().snapshot()).unwrap());
    discover_on_conn(&mut conn, &different_fact(), now()).unwrap();
    let next_a = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    assert!(
        !admit_on_conn(&mut conn, fixture().snapshot()).unwrap(),
        "old v2 alias must not authorize latest Pending"
    );
    decide_on_conn(
        &mut conn,
        &next_a.candidate_id,
        &next_a.evidence_token,
        ReviewDecision::Reject,
        "TEST_CODE_reviewer",
        "TEST_CODE_reject",
        now(),
    )
    .unwrap();
    assert!(!admit_on_conn(&mut conn, fixture().snapshot()).unwrap());
    assert!(
        legacy::has_exact_daily_change_confirmation_on_conn(&mut conn, &first.snapshot.query)
            .unwrap(),
        "raw legacy fact remains immutable"
    );
    discover_on_conn(&mut conn, &different_fact(), now()).unwrap();
    let next_a = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    decide_on_conn(
        &mut conn,
        &next_a.candidate_id,
        &next_a.evidence_token,
        ReviewDecision::Confirm,
        "TEST_CODE_new_op",
        "TEST_CODE_new_reason",
        now(),
    )
    .unwrap();
    assert!(
        admit_on_conn(&mut conn, fixture().snapshot()).unwrap(),
        "explicit new decision may reference old exact alias"
    );
    let mut legacy_conn = initialized(&dir.path().join("TEST_CODE_legacy.sqlite"));
    legacy::append_daily_change_confirmation_on_conn(
        &mut legacy_conn,
        &legacy::DailyChangeConfirmationInput {
            query: fixture().snapshot().query.clone(),
            operator_identity: "TEST_CODE_legacy_op".into(),
            reason: "TEST_CODE_legacy_reason".into(),
            confirmed_at: now().fixed_offset(),
        },
    )
    .unwrap();
    assert!(admit_on_conn(&mut legacy_conn, fixture().snapshot()).unwrap());
    assert!(matches!(
        discover_on_conn(&mut legacy_conn, &fixture(), now()),
        Err(ReviewError::AlreadyConfirmed)
    ));
    assert!(admit_on_conn(&mut legacy_conn, fixture().snapshot()).unwrap());
}

#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type=BigInt)]
    n: i64,
}
fn count(conn: &mut SqliteConnection, table: &str) -> i64 {
    diesel::sql_query(format!("SELECT COUNT(*) AS n FROM {table}"))
        .get_result::<Count>(conn)
        .unwrap()
        .n
}

#[test]
fn task8_concurrent_confirm_reject_single_terminal_and_retry_is_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_concurrent.sqlite");
    let mut conn = initialized(&path);
    let c = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    drop(conn);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = [ReviewDecision::Confirm, ReviewDecision::Reject]
        .into_iter()
        .map(|decision| {
            let path = path.clone();
            let c = c.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut conn = open(&path);
                barrier.wait();
                (
                    decision,
                    decide_on_conn(
                        &mut conn,
                        &c.candidate_id,
                        &c.evidence_token,
                        decision,
                        "TEST_CODE_op",
                        "TEST_CODE_reason",
                        now(),
                    ),
                )
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|(_, r)| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|(_, r)| matches!(r, Err(ReviewError::Conflict)))
            .count(),
        1
    );
    let winner = results.iter().find(|(_, r)| r.is_ok()).unwrap();
    let mut conn = open(&path);
    assert_eq!(count(&mut conn, "daily_change_review_event"), 2);
    let retried = decide_on_conn(
        &mut conn,
        &c.candidate_id,
        &c.evidence_token,
        winner.0,
        "TEST_CODE_op",
        "TEST_CODE_reason",
        now() + chrono::Duration::days(20),
    )
    .unwrap();
    assert_eq!(retried, winner.1.as_ref().unwrap().clone());
    assert_eq!(count(&mut conn, "daily_change_review_event"), 2);
    assert_eq!(
        count(&mut conn, "daily_change_confirmation_v2"),
        if winner.0 == ReviewDecision::Confirm {
            1
        } else {
            0
        }
    );
    assert!(matches!(
        decide_on_conn(
            &mut conn,
            &c.candidate_id,
            &c.evidence_token,
            winner.0,
            "TEST_CODE_op",
            "TEST_CODE_changed",
            now()
        ),
        Err(ReviewError::Conflict)
    ));
    assert!(matches!(
        decide_on_conn(
            &mut conn,
            &c.candidate_id,
            &"0".repeat(64),
            winner.0,
            "TEST_CODE_op",
            "TEST_CODE_reason",
            now()
        ),
        Err(ReviewError::InvalidToken)
    ));
}

#[test]
fn task8_alias_chain_insert_failure_rolls_back_v1_alias_and_decision() {
    use diesel::connection::SimpleConnection;
    let dir = tempfile::tempdir().unwrap();
    let mut conn = initialized(&dir.path().join("TEST_CODE_rollback.sqlite"));
    let c = discover_on_conn(&mut conn, &fixture(), now()).unwrap();
    conn.batch_execute("CREATE TRIGGER TEST_CODE_alias_fault BEFORE INSERT ON daily_change_confirmation_chain_v2 BEGIN SELECT RAISE(ABORT,'TEST_CODE_alias_fault'); END").unwrap();
    assert!(decide_on_conn(
        &mut conn,
        &c.candidate_id,
        &c.evidence_token,
        ReviewDecision::Confirm,
        "TEST_CODE_op",
        "TEST_CODE_reason",
        now()
    )
    .is_err());
    assert_eq!(
        review_on_conn(&mut conn, &c.candidate_id, now()).unwrap(),
        c
    );
    for table in [
        "daily_change_confirmation",
        "daily_change_confirmation_chain",
        "daily_change_confirmation_v2",
        "daily_change_confirmation_chain_v2",
    ] {
        assert_eq!(count(&mut conn, table), 0);
    }
    assert_eq!(count(&mut conn, "daily_change_review_event"), 1);
    conn.batch_execute("DROP TRIGGER TEST_CODE_alias_fault")
        .unwrap();
    decide_on_conn(
        &mut conn,
        &c.candidate_id,
        &c.evidence_token,
        ReviewDecision::Confirm,
        "TEST_CODE_op",
        "TEST_CODE_reason",
        now(),
    )
    .unwrap();
}

#[test]
fn task8_review_namespace_and_truncated_chain_fail_closed() {
    use diesel::connection::SimpleConnection;
    for mutation in [
        "DROP TRIGGER daily_change_review_no_update",
        "CREATE TABLE daily_change_review_shadow(value TEXT)",
        "CREATE TEMP TABLE daily_change_review_shadow(value TEXT)",
        "PRAGMA user_version=99",
        "DROP TRIGGER daily_change_review_no_delete; DELETE FROM daily_change_review_event WHERE seq=2; CREATE TRIGGER daily_change_review_no_delete BEFORE DELETE ON daily_change_review_event BEGIN SELECT RAISE(ABORT,'append-only daily change review'); END",
    ] {
        let dir=tempfile::tempdir().unwrap(); let mut conn=initialized(&dir.path().join("TEST_CODE_tamper.sqlite"));
        let c=discover_on_conn(&mut conn,&fixture(),now()).unwrap();
        decide_on_conn(&mut conn,&c.candidate_id,&c.evidence_token,ReviewDecision::Confirm,"TEST_CODE_op","TEST_CODE_reason",now()).unwrap();
        conn.batch_execute(mutation).unwrap();
        assert!(review_on_conn(&mut conn,&c.candidate_id,now()).is_err(),"accepted {mutation}");
        assert!(admit_on_conn(&mut conn,fixture().snapshot()).is_err(),"authorized {mutation}");
    }
}

#[test]
fn task8_absent_extension_allows_only_exact_legacy_read_never_lazy_installs() {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open(&dir.path().join("TEST_CODE_old.sqlite"));
    super::super::daily_change_confirmation::create_schema(&mut conn).unwrap();
    legacy::append_daily_change_confirmation_on_conn(
        &mut conn,
        &legacy::DailyChangeConfirmationInput {
            query: fixture().snapshot().query.clone(),
            operator_identity: "TEST_CODE_legacy".into(),
            reason: "TEST_CODE_reason".into(),
            confirmed_at: now().fixed_offset(),
        },
    )
    .unwrap();
    assert!(admit_on_conn(&mut conn, fixture().snapshot()).unwrap());
    assert!(!admit_on_conn(&mut conn, different_fact().snapshot()).unwrap());
    assert!(matches!(
        discover_on_conn(&mut conn, &fixture(), now()),
        Err(ReviewError::Unavailable)
    ));
    let n = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM sqlite_master WHERE name LIKE 'daily_change_review_%'",
    )
    .get_result::<Count>(&mut conn)
    .unwrap()
    .n;
    assert_eq!(n, 0);
}
