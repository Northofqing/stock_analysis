use super::*;
use crate::database::global_schema_v1::candidate_v7::{
    self as v7, CandidateCatalog7TransactionError,
};
use crate::decision::candidate_scope_observation_v1::{
    observe_candidate_scope_at_for_test as observe, read_candidate_scope_observation as read,
    ObservationError,
};
use chrono::{Duration, Timelike};

fn fixture7() -> Fixture {
    let f = Fixture::v6();
    v7::migrate_catalog7_for_isolated_test(&f.db).unwrap();
    f
}
fn count_observations(db: &DatabaseManager) -> i64 {
    capture::int(
        &mut db.get_conn().unwrap(),
        "SELECT COUNT(*) AS value FROM main.candidate_scope_observations_v1",
    )
    .unwrap()
}
fn fixed_now() -> chrono::DateTime<Utc> {
    instant().with_nanosecond(123_456_789).unwrap()
}
fn raw_candidate(db: &DatabaseManager) {
    insert_candidate_scope_row(
        &mut db.get_conn().unwrap(),
        1,
        "2026-09-28 09:15:00.000",
        "{}",
        None,
    );
}

#[test]
fn candidate_catalog7_actual_upgrade_preserves_financial_rows_and_exact6_refuses() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    let f = fixture7();
    let mut conn = f.db.get_conn().unwrap();
    assert!(crate::database::paper_book_v2_schema::verify_v6_manifest_on(&mut conn).is_err());
    assert!(crate::trading::paper_book_v2_execution::verify_rows_on(&mut conn).is_err());
    crate::trading::paper_book_v2_execution::verify_rows_on_catalog7(&mut conn).unwrap();
    assert_eq!(
        capture::int(
            &mut conn,
            "SELECT user_version AS value FROM pragma_user_version"
        )
        .unwrap(),
        7
    );
    assert!(
        capture::int(
            &mut conn,
            "SELECT COUNT(*) AS value FROM main.paper_ledger_event"
        )
        .unwrap()
            > 0
    );
    drop(conn);
    assert!(paper_catalog6_session(&f.db)
        .unwrap()
        .with_readonly_catalog6(|_, _| Ok::<_, PaperCatalog6Error>(()), |_, _, _| Ok(()))
        .is_err());
    v7::candidate_catalog7_session(&f.db)
        .unwrap()
        .with_readonly_catalog7(|_, _| Ok::<_, PaperCatalog6Error>(()), |_, _, _| Ok(()))
        .unwrap();
}

#[test]
fn candidate_catalog7_first_retry_cold_reopen_preserves_cutoff_and_original_bytes() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    let f = fixture7();
    raw_candidate(&f.db);
    let first = observe(&f.db, fixed_now()).unwrap();
    let expected = first.canonical_bytes().to_vec();
    let id = first.occurrence_id().to_owned();
    let scope_id = first.scope_id().to_owned();
    assert!(scope_id.starts_with("candidate-observation-scope-v1:"));
    assert!(!id.contains("capture"));
    let path =
        f.db.isolated_p05_consumer_origin
            .as_ref()
            .unwrap()
            .path
            .clone();
    f.db.get_conn()
        .unwrap()
        .batch_execute(
            "UPDATE main.pushed_stocks SET metric_json='changed after original observation'",
        )
        .unwrap();
    let retry = observe(&f.db, fixed_now() + Duration::seconds(10)).unwrap();
    assert_eq!(retry.cutoff(), fixed_now());
    assert_eq!(retry.canonical_bytes(), expected);
    assert_eq!(retry.occurrence_id(), id);
    assert_eq!(count_observations(&f.db), 1);
    let slot = first.slot_start_unix_ms();
    drop(first);
    drop(retry);
    let Fixture {
        _dir,
        db,
        binding: _,
    } = f;
    drop(db);
    let db = DatabaseManager::open_frozen_catalog_for_isolated_test(path).unwrap();
    let retained = read(&db, slot).unwrap().unwrap();
    assert_eq!(retained.canonical_bytes(), expected);
    assert_eq!(retained.occurrence_id(), id);
    let retry = observe(&db, fixed_now() + Duration::seconds(20)).unwrap();
    assert_eq!(retry.cutoff(), fixed_now());
    let next = observe(&db, fixed_now() + Duration::seconds(30)).unwrap();
    assert_ne!(next.occurrence_id(), id);
    assert_eq!(count_observations(&db), 2);
    drop(db);
    drop(_dir);
}

#[test]
fn candidate_catalog7_actual_two_coordinators_same_slot_keep_one_winner() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    let f = fixture7();
    raw_candidate(&f.db);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let workers: Vec<_> = [fixed_now(), fixed_now() + Duration::seconds(7)]
        .into_iter()
        .map(|now| {
            let db = Arc::clone(&f.db);
            let start = Arc::clone(&barrier);
            std::thread::spawn(move || {
                start.wait();
                observe(&db, now)
            })
        })
        .collect();
    barrier.wait();
    let mut winners = Vec::new();
    for worker in workers {
        winners.push(worker.join().unwrap().unwrap());
    }
    assert_eq!(count_observations(&f.db), 1);
    assert_eq!(winners[0].occurrence_id(), winners[1].occurrence_id());
    assert_eq!(winners[0].cutoff(), winners[1].cutoff());
    assert_eq!(winners[0].canonical_bytes(), winners[1].canonical_bytes());
    assert!([fixed_now(), fixed_now() + Duration::seconds(7)].contains(&winners[0].cutoff()));
}

#[test]
fn candidate_catalog7_wrong_writer_and_reader_loans_do_zero_sql() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    let f = fixture7();
    let mut writer = f.db.attribution_checkout().unwrap();
    v7::candidate_catalog7_session(&f.db)
        .unwrap()
        .with_immediate_catalog7(
            |_, _, proof| {
                let mut held = proof.source.readback_connection.lock().unwrap();
                let reader = held.as_mut().unwrap();
                let count = count_actual_sql(reader);
                confirm_counter_and_reset(reader, &count);
                assert!(matches!(
                    proof.validate_on(reader),
                    Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                ));
                assert!(matches!(
                    proof.require_observation_instance(reader),
                    Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                ));
                assert_eq!(count.load(Ordering::SeqCst), 0);
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _, _| Ok(()),
        )
        .unwrap();
    v7::candidate_catalog7_session(&f.db)
        .unwrap()
        .with_readonly_catalog7(
            |_, proof| {
                let foreign = writer.connection_for_test();
                let count = count_actual_sql(foreign);
                confirm_counter_and_reset(foreign, &count);
                assert!(matches!(
                    proof.validate_on(foreign),
                    Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                ));
                assert!(matches!(
                    proof.require_observation_instance(foreign),
                    Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                ));
                assert_eq!(count.load(Ordering::SeqCst), 0);
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _| Ok(()),
        )
        .unwrap();
}

#[test]
fn candidate_catalog7_last_hook_source_or_schema_drift_rolls_back() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    for sql in [
        "UPDATE main.pushed_stocks SET metric_json='late source drift'",
        "CREATE TABLE TEST_CODE_unexpected_catalog7(x)",
        "CREATE TEMP TABLE TEST_CODE_unexpected_temp7(x)",
        "ATTACH DATABASE ':memory:' AS TEST_CODE_attached7",
    ] {
        let f = fixture7();
        raw_candidate(&f.db);
        let guard = set_hook(move |phase, conn| {
            if phase == TestPhase::BeforeTail {
                conn.batch_execute(sql)?;
            }
            Ok(())
        });
        assert!(observe(&f.db, fixed_now()).is_err(), "{sql}");
        drop(guard);
        assert_eq!(count_observations(&f.db), 0);
    }
}

fn corrupt_record(db: &DatabaseManager, assignment: &str) {
    let mut conn = db.get_conn().unwrap();
    conn.batch_execute("DROP TRIGGER candidate_scope_observations_v1_no_update; PRAGMA ignore_check_constraints=ON").unwrap();
    conn.batch_execute(&format!(
        "UPDATE main.candidate_scope_observations_v1 SET {assignment}"
    ))
    .unwrap();
    conn.batch_execute("PRAGMA ignore_check_constraints=OFF")
        .unwrap();
    let sql = crate::database::candidate_scope_observation_schema_v1::STATEMENTS
        .iter()
        .find(|(_, name, _, _)| *name == "candidate_scope_observations_v1_no_update")
        .unwrap()
        .3;
    conn.batch_execute(sql).unwrap();
}
#[test]
fn candidate_catalog7_stored_types_digest_closed_json_and_resource_bounds_refuse() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    for assignment in [
        "scope_sha256=zeroblob(32)",
        "scope_canonical=X'7b7d'",
        "scope_canonical=zeroblob(8388609)",
        "scope_canonical=CAST(scope_canonical AS TEXT)",
        "cutoff_subsec_nanos=X'01'",
    ] {
        let f = fixture7();
        let first = observe(&f.db, fixed_now()).unwrap();
        corrupt_record(&f.db, assignment);
        assert!(
            read(&f.db, first.slot_start_unix_ms()).is_err(),
            "{assignment}"
        );
        assert!(
            observe(&f.db, fixed_now() + Duration::seconds(1)).is_err(),
            "{assignment}"
        );
        assert_eq!(count_observations(&f.db), 1);
    }
    // A matching digest cannot turn an approving or open JSON shape into a negative observation.
    let f = fixture7();
    let first = observe(&f.db, fixed_now()).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
    value["manual_approval"] = serde_json::json!("approved");
    let bytes = serde_json::to_vec(&value).unwrap();
    let mut hash = sha2::Sha256::new();
    use sha2::Digest;
    hash.update(b"stock_analysis.candidate_scope_observation.scope.v1\0");
    hash.update(&bytes);
    corrupt_record(
        &f.db,
        &format!(
            "scope_canonical=X'{}',scope_sha256=X'{}'",
            hex::encode(bytes),
            hex::encode(hash.finalize())
        ),
    );
    assert!(read(&f.db, first.slot_start_unix_ms()).is_err());
    let f = fixture7();
    insert_candidate_scope_row(
        &mut f.db.get_conn().unwrap(),
        1,
        "2026-09-28 09:15:00.000",
        &"x".repeat(65537),
        None,
    );
    assert!(observe(&f.db, fixed_now()).is_err());
    assert_eq!(count_observations(&f.db), 0);
}

#[test]
fn candidate_catalog7_actual_stored_json_preflight_rejects_before_owned_decode() {
    use crate::database::global_schema_v1::candidate_v7::CandidateCatalog7ReadbackError;
    use crate::decision::pushed_candidate_scope_v1::CandidateScopeError;
    use sha2::Digest;
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    for attack in ["51rows", "text", "metric", "row", "scope", "wrong_codec"] {
        let f = fixture7();
        raw_candidate(&f.db);
        let first = observe(&f.db, fixed_now()).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
        match attack {
            "51rows" => {
                value["candidates"] =
                    serde_json::Value::Array(vec![value["candidates"][0].clone(); 51]);
            }
            "text" => {
                value["candidates"][0]["row"]["code"] = serde_json::json!("x".repeat(16385));
            }
            "metric" => {
                value["candidates"][0]["row"]["metric_json"] = serde_json::json!("x".repeat(65537));
            }
            "row" => {
                for name in ["code", "name", "push_kind", "source"] {
                    value["candidates"][0]["row"][name] = serde_json::json!("x".repeat(16384));
                }
                value["candidates"][0]["row"]["metric_json"] = serde_json::json!("x".repeat(65536));
            }
            "scope" => {
                value["candidates"][0]["row"]["metric_json"] = serde_json::json!("x".repeat(65536));
                value["candidates"] =
                    serde_json::Value::Array(vec![value["candidates"][0].clone(); 17]);
            }
            "wrong_codec" => {
                value["candidates"][0]["row"]["push_price_real_bits"] = serde_json::json!("10.25");
            }
            _ => unreachable!(),
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(
            bytes.len() < crate::database::candidate_scope_observation_schema_v1::MAX_SCOPE_BYTES
        );
        let mut digest = sha2::Sha256::new();
        digest.update(b"stock_analysis.candidate_scope_observation.scope.v1\0");
        digest.update(&bytes);
        corrupt_record(
            &f.db,
            &format!(
                "scope_canonical=X'{}',scope_sha256=X'{}'",
                hex::encode(bytes),
                hex::encode(digest.finalize())
            ),
        );
        assert!(
            matches!(
                read(&f.db, first.slot_start_unix_ms()),
                Err(CandidateCatalog7ReadbackError::Consumer(
                    ObservationError::Source(CandidateScopeError::Bounds)
                ))
            ),
            "{attack}"
        );
        assert_eq!(count_observations(&f.db), 1);
    }
}

#[test]
fn candidate_catalog7_rowid_overflow_and_unqualified_origin_refuse() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    let f = fixture7();
    observe(&f.db, fixed_now()).unwrap();
    corrupt_record(&f.db, "observation_row_id=9223372036854775807");
    assert!(matches!(
        observe(&f.db, fixed_now() + Duration::seconds(30)),
        Err(CandidateCatalog7TransactionError::Consumer(
            ObservationError::RowIdExhausted
        ))
    ));
    let mut db = DatabaseManager::open_frozen_catalog_for_isolated_test(
        f._dir.path().join("TEST_CODE_unqualified7.db"),
    )
    .unwrap();
    db.isolated_p05_consumer_origin = None;
    assert!(matches!(
        v7::candidate_catalog7_session(&db),
        Err(PaperCatalog6Error::Catalog7RequalificationRequired)
    ));
    assert!(matches!(
        observe(&db, fixed_now()),
        Err(CandidateCatalog7TransactionError::BeforeCommit(
            PaperCatalog6Error::Catalog7RequalificationRequired
        ))
    ));
}

#[test]
fn candidate_catalog7_foreign_authority_capture_and_final_record_drift_refuse() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    let original = fixture7();
    let foreign = fixture7();
    let capture = v7::candidate_catalog7_session(&original.db)
        .unwrap()
        .with_readonly_catalog7(
            |conn, proof| {
                crate::decision::pushed_candidate_scope_v1::capture_catalog7_at(
                    conn,
                    proof,
                    fixed_now(),
                )
            },
            |_, _, _| Ok(()),
        )
        .unwrap();
    v7::candidate_catalog7_session(&foreign.db).unwrap().with_readonly_catalog7(|conn, proof| {
        let queries = count_actual_sql(conn); confirm_counter_and_reset(conn, &queries);
        assert!(matches!(capture.verify_catalog7_unchanged(conn,proof), Err(crate::decision::pushed_candidate_scope_v1::CandidateScopeError::SourceAuthority)));
        assert_eq!(queries.load(Ordering::SeqCst), 0);
        Ok::<_,PaperCatalog6Error>(())
    }, |_,_,_| Ok(())).unwrap();
    let f = fixture7();
    let guard = set_hook(|phase, conn| {
        if phase == TestPhase::BeforeTail {
            conn.batch_execute("DROP TRIGGER candidate_scope_observations_v1_no_update; UPDATE main.candidate_scope_observations_v1 SET cutoff_subsec_nanos=cutoff_subsec_nanos+1;")?;
            let sql = crate::database::candidate_scope_observation_schema_v1::STATEMENTS
                .iter()
                .find(|(_, name, _, _)| *name == "candidate_scope_observations_v1_no_update")
                .unwrap()
                .3;
            conn.batch_execute(sql)?;
        }
        Ok(())
    });
    assert!(matches!(
        observe(&f.db, fixed_now()),
        Err(CandidateCatalog7TransactionError::Consumer(_))
    ));
    drop(guard);
    assert_eq!(count_observations(&f.db), 0);
}

#[test]
fn candidate_catalog7_empty_header_and_partial_migration_never_qualify() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    let f = Fixture::v6();
    f.db.get_conn()
        .unwrap()
        .batch_execute("PRAGMA user_version=7")
        .unwrap();
    assert!(observe(&f.db, fixed_now()).is_err());
    let f = Fixture::v6();
    f.db.get_conn()
        .unwrap()
        .batch_execute(crate::database::candidate_scope_observation_schema_v1::STATEMENTS[0].3)
        .unwrap();
    assert!(v7::migrate_catalog7_for_isolated_test(&f.db).is_err());
    assert_eq!(
        capture::int(
            &mut f.db.get_conn().unwrap(),
            "SELECT user_version AS value FROM pragma_user_version"
        )
        .unwrap(),
        6
    );
}

#[test]
fn candidate_catalog7_postcommit_actual_child_change_preserves_original_fact() {
    let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
        .lock()
        .unwrap();
    let f = fixture7();
    raw_candidate(&f.db);
    let path =
        f.db.isolated_p05_consumer_origin
            .as_ref()
            .unwrap()
            .path
            .clone();
    let child_path = path.clone();
    let guard = crate::database::install_readback_serialization_hook(
        move || {
            let child=std::process::Command::new(std::env::current_exe().unwrap()).args(["--ignored","--exact","database::global_schema_v1::paper_v6::tests::candidate7_tests::candidate_catalog7_external_commit_child","--nocapture","--test-threads=1"]).env("TEST_CODE_CATALOG7_CHILD_DATABASE",child_path).output().unwrap();
            let stdout = String::from_utf8(child.stdout).unwrap();
            let stderr = String::from_utf8(child.stderr).unwrap();
            assert!(child.status.success(), "child failed: {stdout}\n{stderr}");
            assert!(stdout.contains("running 1 test"));
            assert!(stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"));
            assert!(stdout.contains("TEST_CODE_CATALOG7_EXTERNAL_COMMIT_DONE"));
        },
        || {},
    );
    let result = observe(&f.db, fixed_now());
    drop(guard);
    assert!(matches!(
        result,
        Err(CandidateCatalog7TransactionError::CommittedConsumerOutcomeUnknown(_))
            | Err(CandidateCatalog7TransactionError::CommitOutcomeUnknown(_))
    ));
    assert_eq!(count_observations(&f.db), 1);
    assert_eq!(capture::int(&mut f.db.get_conn().unwrap(),"SELECT COUNT(*) AS value FROM main.pushed_stocks WHERE metric_json='TEST_CODE_child_committed7'").unwrap(),1);
    // Independent inspection verifies the committed original, with no retry/write.
    let conn = rusqlite::Connection::open(path).unwrap();
    let bytes: Vec<u8> = conn
        .query_row(
            "SELECT scope_canonical FROM main.candidate_scope_observations_v1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["candidates"][0]["row"]["metric_json"], "{}");
}
#[test]
#[ignore = "only the parent runs this exact child against its complete isolated Catalog7 fixture"]
fn candidate_catalog7_external_commit_child() {
    let path = std::env::var_os("TEST_CODE_CATALOG7_CHILD_DATABASE")
        .expect("parent-owned isolated fixture");
    let path = std::path::PathBuf::from(path);
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("TEST_CODE_"));
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    conn.execute_batch("BEGIN IMMEDIATE; UPDATE main.pushed_stocks SET metric_json='TEST_CODE_child_committed7'; COMMIT;").unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM main.candidate_scope_observations_v1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    println!("TEST_CODE_CATALOG7_EXTERNAL_COMMIT_DONE");
}
