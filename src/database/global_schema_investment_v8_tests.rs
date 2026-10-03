mod investment8_tests {
    use super::*;
    use crate::database::global_schema_v1::{candidate_v7 as v7, investment_v8 as v8};
    use crate::decision::investment_decision_v1::{
        changed_bytes_for_test, read_investment_decision as read,
        record_investment_decision_at_for_test as record, InvestmentRecordError,
    };
    use chrono::{Duration, Timelike};
    fn fixture8() -> Fixture {
        let f = Fixture::v6();
        v7::migrate_catalog7_for_isolated_test(&f.db).unwrap();
        v8::migrate_catalog8_for_isolated_test(&f.db).unwrap();
        f
    }
    fn now() -> chrono::DateTime<Utc> {
        instant().with_nanosecond(123_456_789).unwrap()
    }
    fn config() -> crate::config::LiveVetoConfig {
        crate::config::LiveVetoConfig::default()
    }
    fn raw(db: &DatabaseManager) {
        insert_candidate_scope_row(
            &mut db.get_conn().unwrap(),
            1,
            "2026-09-28 09:15:00.000",
            "{}",
            None,
        );
    }
    fn count(db: &DatabaseManager) -> i64 {
        capture::int(
            &mut db.get_conn().unwrap(),
            "SELECT COUNT(*) AS value FROM main.investment_decisions_v1",
        )
        .unwrap()
    }
    fn corrupt(db: &DatabaseManager, assignment: &str) {
        let mut c = db.get_conn().unwrap();
        c.batch_execute(
            "DROP TRIGGER investment_decisions_v1_no_update; PRAGMA ignore_check_constraints=ON",
        )
        .unwrap();
        c.batch_execute(&format!(
            "UPDATE main.investment_decisions_v1 SET {assignment}"
        ))
        .unwrap();
        c.batch_execute("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        c.batch_execute(crate::database::investment_decision_schema_v1::STATEMENTS[1].3)
            .unwrap();
    }
    fn corrupt_bytes(db: &DatabaseManager, bytes: &[u8]) {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(b"stock_analysis.investment_decision.record.v1\0");
        h.update(bytes);
        let digest = hex::encode(h.finalize());
        corrupt(db,&format!("record_canonical=X'{}',record_sha256=X'{}',decision_id='investment-decision-v1:{}'",hex::encode(bytes),digest,digest));
    }
    #[test]
    fn investment8_actual_c7_scope_full_negative_matrix_and_financial_history() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = Fixture::v6();
        v7::migrate_catalog7_for_isolated_test(&f.db).unwrap();
        raw(&f.db);
        insert_candidate_scope_row(
            &mut f.db.get_conn().unwrap(),
            2,
            "2026-09-28 09:15:00.001",
            "{\"original\":true}",
            None,
        );
        let scope =
            crate::decision::candidate_scope_observation_v1::observe_candidate_scope_at_for_test(
                &f.db,
                now(),
            )
            .unwrap();
        let bytes = scope.canonical_bytes().to_vec();
        let cutoff = scope.cutoff();
        v8::migrate_catalog8_for_isolated_test(&f.db).unwrap();
        let result = record(&f.db, &config(), now() + Duration::seconds(7)).unwrap();
        assert_eq!(result.cutoff(), cutoff);
        assert_eq!(result.candidate_count(), 2);
        assert!(!result.is_no_candidates());
        let v: serde_json::Value = serde_json::from_slice(result.canonical_bytes()).unwrap();
        assert_eq!(
            v["scope"]["canonical_utf8"].as_str().unwrap().as_bytes(),
            bytes
        );
        assert_eq!(v["disposition"], "DeniedBeforeFacts");
        assert_eq!(v["required_fields"].as_array().unwrap().len(), 12);
        for c in v["candidate_evaluations"].as_array().unwrap() {
            assert_eq!(c["required_fields"].as_array().unwrap().len(), 12);
            assert_eq!(c["rules"].as_array().unwrap().len(), 3);
            assert_eq!(c["disposition"], "DeniedBeforeFacts");
            for field in c["required_fields"].as_array().unwrap() {
                assert_eq!(field["action_fact_state"], "Unqualified");
            }
            for i in [2, 3, 4] {
                assert_eq!(
                    c["required_fields"][i]["assessment"]["state"],
                    "AcquisitionNotInvoked"
                );
            }
            for i in [6, 7] {
                assert_eq!(
                    c["required_fields"][i]["assessment"]["state"],
                    "NotEvaluated"
                );
            }
        }
        assert!(result.id().as_str().starts_with("investment-decision-v1:"));
        assert!(!result.id().as_str().contains("observation"));
        let mut c = f.db.get_conn().unwrap();
        assert!(
            capture::int(
                &mut c,
                "SELECT COUNT(*) AS value FROM main.paper_ledger_event"
            )
            .unwrap()
                > 0
        );
        crate::trading::paper_book_v2_execution::verify_rows_on_catalog8(&mut c).unwrap();
        assert!(crate::trading::paper_book_v2_execution::verify_rows_on(&mut c).is_err());
        assert!(crate::trading::paper_book_v2_execution::verify_rows_on_catalog7(&mut c).is_err());
        drop(c);
        assert!(paper_catalog6_session(&f.db)
            .unwrap()
            .with_readonly_catalog6(|_, _| Ok::<_, PaperCatalog6Error>(()), |_, _, _| Ok(()))
            .is_err());
        assert!(v7::candidate_catalog7_session(&f.db)
            .unwrap()
            .with_readonly_catalog7(|_, _| Ok::<_, PaperCatalog6Error>(()), |_, _, _| Ok(()))
            .is_err());
    }
    #[test]
    fn investment8_actual_empty_scope_and_exact_retry_changed_config_cold_reopen() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = fixture8();
        let first = record(&f.db, &config(), now()).unwrap();
        assert!(first.is_no_candidates());
        assert_eq!(first.candidate_count(), 0);
        let bytes = first.canonical_bytes().to_vec();
        let id = first.id().as_str().to_owned();
        let slot = first.slot_start_unix_ms();
        raw(&f.db);
        let mut changed = config();
        changed.mode = "x".repeat(65); // A new config is invalid, but existing-key replay never freezes it.
        let retry = record(&f.db, &changed, now() + Duration::seconds(10)).unwrap();
        assert_eq!(retry.canonical_bytes(), bytes);
        assert_eq!(retry.id().as_str(), id);
        assert_eq!(retry.cutoff(), now());
        assert_eq!(count(&f.db), 1);
        let path =
            f.db.isolated_p05_consumer_origin
                .as_ref()
                .unwrap()
                .path
                .clone();
        let Fixture { db, _dir, .. } = f;
        drop(db);
        let db = DatabaseManager::open_frozen_catalog_for_isolated_test(path).unwrap();
        assert_eq!(read(&db, slot).unwrap().unwrap().canonical_bytes(), bytes);
        let new = record(&db, &config(), now() + Duration::seconds(30)).unwrap();
        assert_eq!(new.candidate_count(), 1);
        assert_ne!(new.id().as_str(), id);
        assert_eq!(count(&db), 2);
        drop(db);
        drop(_dir);
    }
    #[test]
    fn investment8_actual_config_collision_and_concurrent_coordinators() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = fixture8();
        raw(&f.db);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = [now(), now() + Duration::seconds(7)]
            .into_iter()
            .map(|time| {
                let db = Arc::clone(&f.db);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    record(&db, &config(), time)
                })
            })
            .collect();
        barrier.wait();
        let mut winners = Vec::new();
        for w in workers {
            winners.push(w.join().unwrap().unwrap());
        }
        assert_eq!(count(&f.db), 1);
        assert_eq!(winners[0].id(), winners[1].id());
        assert_eq!(winners[0].canonical_bytes(), winners[1].canonical_bytes());
        let mut changed = config();
        changed.fundamental_enabled = !changed.fundamental_enabled;
        assert!(matches!(
            changed_bytes_for_test(&f.db, winners[0].slot_start_unix_ms(), &changed),
            Err(v8::InvestmentCatalog8TransactionError::Consumer(
                InvestmentRecordError::Conflict
            ))
        ));
        assert_eq!(
            read(&f.db, winners[0].slot_start_unix_ms())
                .unwrap()
                .unwrap()
                .canonical_bytes(),
            winners[0].canonical_bytes()
        );
    }
    #[test]
    fn investment8_actual_last_hook_source_catalog_and_record_drift_rollback() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        for sql in [
            "UPDATE main.pushed_stocks SET metric_json='late source drift'",
            "CREATE TABLE TEST_CODE_unexpected8(x)",
            "CREATE TEMP TABLE TEST_CODE_unexpected8(x)",
            "ATTACH DATABASE ':memory:' AS TEST_CODE_attached8",
        ] {
            let f = fixture8();
            raw(&f.db);
            let guard = set_hook(move |phase, conn| {
                if phase == TestPhase::BeforeTail {
                    conn.batch_execute(sql)?;
                }
                Ok(())
            });
            assert!(record(&f.db, &config(), now()).is_err(), "{sql}");
            drop(guard);
            assert_eq!(count(&f.db), 0);
            assert_eq!(
                capture::int(
                    &mut f.db.get_conn().unwrap(),
                    "SELECT COUNT(*) AS value FROM main.candidate_scope_observations_v1"
                )
                .unwrap(),
                0
            );
        }
        let f = fixture8();
        raw(&f.db);
        let guard = set_hook(|phase, c| {
            if phase == TestPhase::BeforeTail {
                c.batch_execute("DROP TRIGGER investment_decisions_v1_no_update; UPDATE main.investment_decisions_v1 SET record_sha256=zeroblob(32)")?;
                c.batch_execute(crate::database::investment_decision_schema_v1::STATEMENTS[1].3)?;
            }
            Ok(())
        });
        assert!(record(&f.db, &config(), now()).is_err());
        drop(guard);
        assert_eq!(count(&f.db), 0);
    }
    #[test]
    fn investment8_actual_dtype_digest_utf8_blob_bounds_refuse() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        for a in [
            "record_sha256=zeroblob(32)",
            "record_canonical=X'7b7d'",
            "record_canonical=zeroblob(16777217)",
            "record_canonical=CAST(record_canonical AS TEXT)",
            "cutoff_subsec_nanos=X'01'",
            "decision_id=CAST(X'FF' AS TEXT)",
            "cutoff_unix_seconds=0",
        ] {
            let f = fixture8();
            let first = record(&f.db, &config(), now()).unwrap();
            corrupt(&f.db, a);
            assert!(read(&f.db, first.slot_start_unix_ms()).is_err(), "{a}");
            assert!(record(&f.db, &config(), now()).is_err());
            assert_eq!(count(&f.db), 1);
        }
    }
    #[test]
    fn investment8_actual_closed_matrix_schema_and_preallocation_bounds_refuse() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        for attack in [
            "missing_field",
            "duplicate_field",
            "missing_rule",
            "duplicate_rule",
            "extra_candidate",
            "unknown_schema",
            "approved",
            "oversized_scalar",
            "scope_mismatch",
            "unknown_field",
        ] {
            let f = fixture8();
            raw(&f.db);
            let first = record(&f.db, &config(), now()).unwrap();
            let mut v: serde_json::Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
            match attack {
                "missing_field" => {
                    v["candidate_evaluations"][0]["required_fields"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
                "duplicate_field" => {
                    v["candidate_evaluations"][0]["required_fields"][1] =
                        v["candidate_evaluations"][0]["required_fields"][0].clone();
                }
                "missing_rule" => {
                    v["candidate_evaluations"][0]["rules"]
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
                "duplicate_rule" => {
                    v["candidate_evaluations"][0]["rules"][1] =
                        v["candidate_evaluations"][0]["rules"][0].clone();
                }
                "extra_candidate" => {
                    v["candidate_evaluations"] =
                        serde_json::json!(vec![v["candidate_evaluations"][0].clone(); 51])
                }
                "unknown_schema" => v["schema_version"] = serde_json::json!(2),
                "approved" => v["disposition"] = serde_json::json!("RiskPassed"),
                "oversized_scalar" => v["model_id"] = serde_json::json!("x".repeat(16385)),
                "scope_mismatch" => v["scope"]["observation_row_id"] = serde_json::json!(99),
                "unknown_field" => v["gateway_success"] = serde_json::json!(true),
                _ => unreachable!(),
            };
            corrupt_bytes(&f.db, &serde_json::to_vec(&v).unwrap());
            assert!(read(&f.db, first.slot_start_unix_ms()).is_err(), "{attack}");
            assert_eq!(count(&f.db), 1);
        }
    }
    #[test]
    fn investment8_actual_immutable_storage_and_rowid_overflow() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = fixture8();
        record(&f.db, &config(), now()).unwrap();
        let mut c = f.db.get_conn().unwrap();
        for sql in ["UPDATE main.investment_decisions_v1 SET evaluation_revision=1","DELETE FROM main.investment_decisions_v1","INSERT OR REPLACE INTO main.investment_decisions_v1 SELECT * FROM main.investment_decisions_v1"]{assert!(c.batch_execute(sql).is_err());}
        drop(c);
        corrupt(&f.db, "decision_row_id=9223372036854775807");
        assert!(matches!(
            record(&f.db, &config(), now() + Duration::seconds(30)),
            Err(v8::InvestmentCatalog8TransactionError::Consumer(
                InvestmentRecordError::RowIdExhausted
            ))
        ));
        assert_eq!(count(&f.db), 1);
    }
    #[test]
    fn investment8_actual_wrong_loan_zero_sql_production_and_partial_header_refuse() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = fixture8();
        let mut writer = f.db.attribution_checkout().unwrap();
        v8::investment_catalog8_session(&f.db)
            .unwrap()
            .with_readonly_catalog8(
                |_, p| {
                    let foreign = writer.connection_for_test();
                    let count = count_actual_sql(foreign);
                    confirm_counter_and_reset(foreign, &count);
                    assert!(matches!(
                        p.require_decision_instance(foreign),
                        Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                    ));
                    assert!(matches!(
                        p.validate_on(foreign),
                        Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                    ));
                    assert_eq!(count.load(Ordering::SeqCst), 0);
                    Ok::<_, PaperCatalog6Error>(())
                },
                |_, _, _| Ok(()),
            )
            .unwrap();
        let mut db = DatabaseManager::open_frozen_catalog_for_isolated_test(
            f._dir.path().join("TEST_CODE_unqualified8.db"),
        )
        .unwrap();
        db.isolated_p05_consumer_origin = None;
        assert!(matches!(
            v8::investment_catalog8_session(&db),
            Err(PaperCatalog6Error::Catalog8RequalificationRequired)
        ));
        let f = Fixture::v6();
        v7::migrate_catalog7_for_isolated_test(&f.db).unwrap();
        f.db.get_conn()
            .unwrap()
            .batch_execute("PRAGMA user_version=8")
            .unwrap();
        assert!(record(&f.db, &config(), now()).is_err());
    }
    #[test]
    fn investment8_postcommit_actual_child_before_first_snapshot_is_unknown_preserves_bytes() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = fixture8();
        raw(&f.db);
        let path =
            f.db.isolated_p05_consumer_origin
                .as_ref()
                .unwrap()
                .path
                .clone();
        let child_path = path.clone();
        let guard = crate::database::install_readback_serialization_hook(
            move || {
                let child=std::process::Command::new(std::env::current_exe().unwrap()).args(["--ignored","--exact","database::global_schema_v1::paper_v6::tests::investment8_tests::investment8_external_commit_child","--nocapture","--test-threads=1"]).env("TEST_CODE_CATALOG8_CHILD_DATABASE",&child_path).output().unwrap();
                let out = String::from_utf8(child.stdout).unwrap();
                let err = String::from_utf8(child.stderr).unwrap();
                assert!(child.status.success(), "{out}\n{err}");
                assert!(out.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"));
                assert!(out.contains("TEST_CODE_CATALOG8_EXTERNAL_COMMIT_DONE"));
            },
            || {},
        );
        let result = record(&f.db, &config(), now());
        drop(guard);
        assert!(matches!(
            result,
            Err(v8::InvestmentCatalog8TransactionError::CommittedConsumerOutcomeUnknown(_))
                | Err(v8::InvestmentCatalog8TransactionError::CommitOutcomeUnknown(_))
        ));
        assert_eq!(count(&f.db), 1);
        let c = rusqlite::Connection::open(path).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM pushed_stocks WHERE metric_json='TEST_CODE_child_committed8'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        let bytes: Vec<u8> = c
            .query_row(
                "SELECT record_canonical FROM investment_decisions_v1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let scope: serde_json::Value =
            serde_json::from_str(v["scope"]["canonical_utf8"].as_str().unwrap()).unwrap();
        assert_eq!(scope["candidates"][0]["row"]["metric_json"], "{}");
        assert_eq!(v["disposition"], "DeniedBeforeFacts");
    }
    #[test]
    #[ignore = "parent-owned actual isolated Catalog8 child only"]
    fn investment8_external_commit_child() {
        let path = std::path::PathBuf::from(
            std::env::var_os("TEST_CODE_CATALOG8_CHILD_DATABASE").expect("parent-owned fixture"),
        );
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("TEST_CODE_"));
        let c = rusqlite::Connection::open(path).unwrap();
        c.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        c.execute_batch("BEGIN IMMEDIATE; UPDATE pushed_stocks SET metric_json='TEST_CODE_child_committed8'; COMMIT;").unwrap();
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM investment_decisions_v1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        println!("TEST_CODE_CATALOG8_EXTERNAL_COMMIT_DONE");
    }
    #[test]
    fn investment8_actual_historical_calendar_freezes_a_while_new_capture_uses_b() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = Fixture::v6();
        v7::migrate_catalog7_for_isolated_test(&f.db).unwrap();
        raw(&f.db);
        let old =
            crate::decision::candidate_scope_observation_v1::observe_candidate_scope_at_for_test(
                &f.db,
                now(),
            )
            .unwrap();
        let a = old.canonical_bytes().to_vec();
        let guard =
            crate::decision::pushed_candidate_scope_v1::calendar_override_for_test("b".repeat(64));
        assert_eq!(
            crate::decision::candidate_scope_observation_v1::read_candidate_scope_observation(
                &f.db,
                old.slot_start_unix_ms()
            )
            .unwrap()
            .unwrap()
            .canonical_bytes(),
            a
        );
        v8::migrate_catalog8_for_isolated_test(&f.db).unwrap();
        let first = record(&f.db, &config(), now()).unwrap();
        let historical = read(&f.db, first.slot_start_unix_ms()).unwrap().unwrap();
        assert_eq!(historical.canonical_bytes(), first.canonical_bytes());
        let next = record(&f.db, &config(), now() + Duration::seconds(30)).unwrap();
        let v: serde_json::Value = serde_json::from_slice(next.canonical_bytes()).unwrap();
        assert_eq!(v["calendar"]["authority_sha256"], "b".repeat(64));
        let prior: serde_json::Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
        assert_ne!(prior["calendar"], v["calendar"]);
        drop(guard);
    }
    #[test]
    fn investment8_local_review_exact8_compatible_unknown9_refuses() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = fixture8();
        let mut c = f.db.get_conn().unwrap();
        assert!(crate::database::daily_change_review_schema_v1::is_present(&mut c).unwrap());
        c.batch_execute("PRAGMA user_version=9").unwrap();
        assert!(crate::database::daily_change_review_schema_v1::is_present(&mut c).is_err());
    }
    #[test]
    fn investment8_actual_complete_empty_record_literal_golden() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = fixture8();
        let time = Utc.with_ymd_and_hms(2030, 1, 2, 0, 0, 0).unwrap();
        let first = record(&f.db, &config(), time).unwrap();
        assert_eq!(first.id().as_str(),"investment-decision-v1:0a882b7805d69933a78116a1beeead760426a0038ad727f08a1de424c3bf6d0f");
        assert_eq!(first.canonical_bytes().len(), 5331);
        assert!(first.is_no_candidates());
    }
    #[test]
    fn investment8_actual_complete_top50_same_code_rows_preserved() {
        let _serial = super::super::super::tests::PROSPECTIVE_TEST_SERIAL
            .lock()
            .unwrap();
        let f = fixture8();
        for id in 1..=51 {
            insert_candidate_scope_row(
                &mut f.db.get_conn().unwrap(),
                id,
                "2026-09-28 09:15:00.000",
                "{}",
                None,
            );
        }
        let first = record(&f.db, &config(), now()).unwrap();
        assert_eq!(first.candidate_count(), 50);
        let v: serde_json::Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
        let rows = v["candidate_evaluations"].as_array().unwrap();
        assert_eq!(rows[0]["source_row_id"], 51);
        assert_eq!(rows[49]["source_row_id"], 2);
        assert_eq!(
            rows.iter()
                .map(|r| r["source_row_id"].as_i64().unwrap())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            50
        );
        assert_eq!(
            read(&f.db, first.slot_start_unix_ms())
                .unwrap()
                .unwrap()
                .canonical_bytes(),
            first.canonical_bytes()
        );
    }
}
