use super::*;
use crate::trading::paper_book_v2::{
    cutover_for_isolated_test, TestCutoverFault, TestCutoverRequest,
};
use crate::trading::paper_ledger::{
    AccountBinding, Money, PaperCommand, PaperLedger, RiskPolicyV1, SeedManifest,
};
use chrono::{TimeZone, Utc};
use diesel::connection::SimpleConnection;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn count_actual_sql(conn: &mut SqliteConnection) -> Arc<AtomicUsize> {
    let count = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&count);
    conn.set_instrumentation(move |event: diesel::connection::InstrumentationEvent<'_>| {
        if matches!(
            event,
            diesel::connection::InstrumentationEvent::StartQuery { .. }
        ) {
            recorded.fetch_add(1, Ordering::SeqCst);
        }
    });
    count
}
fn confirm_counter_and_reset(conn: &mut SqliteConnection, count: &AtomicUsize) {
    assert_eq!(capture::int(conn, "SELECT 1 AS value").unwrap(), 1);
    assert!(count.load(Ordering::SeqCst) > 0);
    count.store(0, Ordering::SeqCst);
}

fn instant() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 28, 1, 30, 0).unwrap()
}
struct Fixture {
    _dir: tempfile::TempDir,
    db: Arc<DatabaseManager>,
    binding: AccountBinding,
}
impl Fixture {
    fn v5() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("TEST_CODE_global6_")
            .tempdir()
            .unwrap();
        let db = Arc::new(
            DatabaseManager::open_frozen_catalog_for_isolated_test(
                dir.path().join("TEST_CODE_global6.db"),
            )
            .unwrap(),
        );
        crate::database::paper_ledger_schema_v1::create_schema(&mut db.get_conn().unwrap())
            .unwrap();
        db.get_conn()
            .unwrap()
            .batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=2")
            .unwrap();
        let ledger = PaperLedger::open(&db, &instant);
        let seed = SeedManifest {
            account_id: "TEST_CODE_ACCOUNT_G6".into(),
            epoch_id: "TEST_CODE_EPOCH_G6_V1".into(),
            command_id: "TEST_CODE_SEED_G6".into(),
            cutover_at: instant(),
            account_effective_at: instant(),
            positions_effective_at: instant(),
            source_reference: "TEST_CODE_explicit_seed".into(),
            source_hash: "a".repeat(64),
            approved_by: "TEST_CODE_explicit_approval".into(),
            cash: Money::from_cny(100_000.0).unwrap(),
            original_total: Money::from_cny(100_000.0).unwrap(),
            excluded_residual: None,
            lots: vec![],
            marks: vec![],
            policy: RiskPolicyV1::default(),
        };
        let binding = seed.binding().unwrap();
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        // Preserve a real nonempty original V1 position and its order audit,
        // not only an empty seed. This is the original isolated recipe.
        let view = ledger.read(&binding).unwrap();
        ledger
            .apply(PaperCommand::Execute(
                crate::trading::paper_ledger::ExecuteIntent {
                    price_intent: crate::trading::paper_ledger::PriceIntent::FixedSignalPriceV1,
                    binding: binding.clone(),
                    command_id: "TEST_CODE_G6_BUY_V1".into(),
                    expected_version: view.version,
                    inventory_fingerprint: view.inventory_fingerprint().unwrap(),
                    signal: crate::trading::paper_trade::PaperSignal {
                        plan_id: format!("paper:{}:TEST_CODE_G6_PLAN_V1", binding.epoch_id),
                        code: "TEST_CODE_000001".into(),
                        name: "fixture".into(),
                        direction: crate::trading::paper_trade::Direction::Buy,
                        price: 10.0,
                        quantity: 100,
                        virtual_reason: "TEST_CODE_evidence".into(),
                        is_limit_up: false,
                        is_limit_down: false,
                        is_suspended: false,
                        limit_up_price: Some(11.0),
                        limit_down_price: Some(9.0),
                        secondary_confirmed: false,
                        quote_observed_at: instant(),
                        risk_context: crate::trading::paper_trade::PaperRiskContext::new(
                            crate::risk::action_gate::AccountMode::Normal,
                            crate::monitor::data_mode::DataMode::Full,
                        ),
                    },
                    quote_price: Money::from_cny(10.0).unwrap(),
                    marks: vec![crate::trading::paper_ledger::Mark {
                        code: "TEST_CODE_000001".into(),
                        price: Money::from_cny(10.0).unwrap(),
                        observed_at: instant(),
                        source: "TEST_CODE_realtime".into(),
                    }],
                },
            ))
            .unwrap();

        let policy =
            crate::performance::fee_policy::AShareFeePolicyV2::fixed_compatibility_assumption();
        {
            let mut conn = db.get_conn().unwrap();
            crate::database::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
            conn.batch_execute("PRAGMA user_version=3").unwrap();
            crate::database::paper_book_owner_schema_v1::install_catalog_v4_for_isolated_test(
                &mut conn, &policy,
            )
            .unwrap();
            crate::database::paper_book_owner_schema_v2::install_catalog_v5_for_isolated_test(
                &mut conn,
            )
            .unwrap();
        }
        let old = crate::trading::paper_ledger::verified_v1_snapshot_on(
            &mut db.get_conn().unwrap(),
            &binding,
        )
        .unwrap();
        cutover_for_isolated_test(
            &db,
            &TestCutoverRequest {
                old_binding: binding.clone(),
                new_epoch_id: "TEST_CODE_EPOCH_G6_V2".into(),
                cutover_id: "TEST_CODE_CUTOVER_G6".into(),
                command_id: "TEST_CODE_GENESIS_G6".into(),
                expected_v1_version: old.version,
                expected_v1_head_hash: old.event_hash,
                expected_v1_projection_hash: old.projection_hash,
                reviewed_fee_policy: policy,
            },
            TestCutoverFault::None,
        )
        .unwrap();
        prepare_final_selection_for_isolated_v5_test(&db).unwrap();
        Self {
            _dir: dir,
            db,
            binding,
        }
    }
    fn v6() -> Self {
        let f = Self::v5();
        migrate_catalog6_for_isolated_test(&f.db).unwrap();
        f
    }
    fn daily_count(&self) -> i64 {
        capture::int(
            &mut self.db.get_conn().unwrap(),
            "SELECT COUNT(*) AS value FROM stock_daily WHERE code='TEST_CODE_G6_SENTINEL'",
        )
        .unwrap()
    }
}
fn insert(conn: &mut SqliteConnection, date: &str) -> Result<(), PaperCatalog6Error> {
    diesel::sql_query("INSERT INTO stock_daily(code,date,open,high,low,close,volume) VALUES ('TEST_CODE_G6_SENTINEL',?,10,10,10,10,100)").bind::<diesel::sql_types::Text,_>(date).execute(conn)?;
    Ok(())
}

fn insert_candidate_scope_row(
    conn: &mut SqliteConnection,
    id: i64,
    time: &str,
    metric: &str,
    consumed: Option<&str>,
) {
    use diesel::sql_types::{BigInt, Nullable, Text};
    diesel::sql_query("INSERT INTO main.pushed_stocks(id,push_time,push_kind,code,name,push_price,metric_json,source,consumed_at,consumed_by,outcome) VALUES(?,?,'raw_kind','SAME_RAW_CODE','原始候选',10.25,?,'TEST_CODE_raw_source',?,'raw_owner','raw_outcome')")
        .bind::<BigInt,_>(id)
        .bind::<Text,_>(time)
        .bind::<Text,_>(metric)
        .bind::<Nullable<Text>,_>(consumed)
        .execute(conn).unwrap();
}

#[test]
fn f2_candidate_scope_actual_top50_ties_bounds_and_raw_identity() {
    use crate::decision::pushed_candidate_scope_v1::{
        capture_pushed_candidate_scope_at_for_test as capture_scope, CandidateScopeError,
    };
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let base = i64::from(i32::MAX) + 1;
    {
        let mut conn = f.db.get_conn().unwrap();
        for i in 0..51 {
            insert_candidate_scope_row(&mut conn, base + i, "2026-09-28 09:15:00.000", "{}", None);
        }
        for (id, time, consumed) in [
            (100, "2026-09-28 08:30:00.000", None),
            (101, "2026-09-28 09:30:00.000", None),
            (102, "2026-09-28 08:29:59.999", None),
            (103, "2026-09-28 09:30:00.001", None),
            (
                104,
                "2026-09-28 09:20:00.000",
                Some("2026-09-28 09:21:00.000"),
            ),
        ] {
            insert_candidate_scope_row(&mut conn, id, time, "{}", consumed);
        }
    }
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let captured = session
        .with_immediate_catalog6(
            |conn, _, proof| {
                let first = capture_scope(conn, proof, instant())?;
                let same = capture_scope(conn, proof, instant())?;
                assert_eq!(first.id(), same.id());
                assert_eq!(first.canonical_bytes(), same.canonical_bytes());
                Ok::<_, CandidateScopeError>(first)
            },
            |conn, _, proof, value| value.verify_unchanged(conn, proof),
        )
        .unwrap();
    assert!(captured
        .id()
        .as_str()
        .starts_with("candidate-scope-capture-v1:"));
    let content: serde_json::Value = serde_json::from_slice(captured.canonical_bytes()).unwrap();
    assert_eq!(
        content["lower_exclusive_shanghai"],
        "2026-09-28 08:30:00.000"
    );
    assert_eq!(
        content["upper_exclusive_shanghai"],
        "2026-09-28 09:30:00.000"
    );
    assert_eq!(content["calendar"]["state"], "covered");
    let candidates = content["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 50);
    for (index, candidate) in candidates.iter().enumerate() {
        let row = &candidate["row"];
        assert_eq!(row["id"], base + 50 - index as i64);
        assert_eq!(row["code"], "SAME_RAW_CODE");
        assert_eq!(row.as_object().unwrap().len(), 11);
        assert_eq!(row["push_price_real_bits"], 10.25f64.to_bits());
        assert_eq!(row["source"], "TEST_CODE_raw_source");
        assert!(row["consumed_at"].is_null());
        assert_eq!(row["consumed_by"], "raw_owner");
        assert_eq!(row["outcome"], "raw_outcome");
        assert_eq!(candidate["disposition"], "identity_unqualified");
        let facts = candidate["facts"].as_array().unwrap();
        assert_eq!(facts.len(), 3);
        assert_eq!(
            facts
                .iter()
                .map(|fact| fact["requirement"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["lifecycle", "price_regime", "suspension"]
        );
        for fact in facts {
            assert_eq!(fact["state"], "unqualified");
            assert_eq!(fact["reason"], "not_requested_identity_unavailable");
        }
        for field in [
            "risk_inventory",
            "cost_model",
            "liquidity_model",
            "budget_allocation",
            "manual_approval",
        ] {
            assert!(candidate[field]
                .as_str()
                .unwrap()
                .starts_with("unavailable_"));
        }
        assert_eq!(
            candidate["risk_evaluation"],
            "not_evaluated_identity_unqualified"
        );
    }
    // Exact raw source and all candidate state, including unqualified fields,
    // are part of identity rather than being dropped by a code-based grouping.
    f.db.get_conn().unwrap().batch_execute("UPDATE main.pushed_stocks SET source='changed_raw_source',outcome=NULL WHERE id=(SELECT MAX(id) FROM main.pushed_stocks)").unwrap();
    drop(session);
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let changed = session
        .with_readonly_catalog6(
            |conn, proof| capture_scope(conn, proof, instant()),
            |conn, proof, value| value.verify_unchanged(conn, proof),
        )
        .unwrap();
    assert_ne!(captured.id(), changed.id());
    // Without the tie pool, lower-bound rows cannot hide below LIMIT 50.
    diesel::sql_query("DELETE FROM main.pushed_stocks WHERE id>=?")
        .bind::<diesel::sql_types::BigInt, _>(base)
        .execute(&mut f.db.get_conn().unwrap())
        .unwrap();
    {
        let mut conn = f.db.get_conn().unwrap();
        insert_candidate_scope_row(&mut conn, 105, "2026-09-28 08:30:00.001", "{}", None);
        insert_candidate_scope_row(&mut conn, 106, "2026-09-28 09:29:59.999", "{}", None);
    }
    // Separate evaluations use fresh bounded owner sessions. A live session's
    // cumulative copy-work counter is never reset or increased.
    drop(session);
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let boundaries = session
        .with_readonly_catalog6(
            |conn, proof| capture_scope(conn, proof, instant()),
            |conn, proof, value| value.verify_unchanged(conn, proof),
        )
        .unwrap();
    let content: serde_json::Value = serde_json::from_slice(boundaries.canonical_bytes()).unwrap();
    let ids = content["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| candidate["row"]["id"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids, [106, 105]);
}

#[test]
fn f2_candidate_scope_malformed_text_is_rejected_before_string_construction() {
    use crate::decision::pushed_candidate_scope_v1::{
        capture_pushed_candidate_scope_at_for_test as capture_scope, CandidateScopeError,
    };
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    insert_candidate_scope_row(
        &mut f.db.get_conn().unwrap(),
        1,
        "2026-09-28 09:15:00.000",
        "{}",
        None,
    );
    for (field, original) in [
        ("push_time", "2026-09-28 09:15:00.000"),
        ("push_kind", "raw_kind"),
        ("code", "SAME_RAW_CODE"),
        ("name", "原始候选"),
        ("metric_json", "{}"),
        ("source", "TEST_CODE_raw_source"),
        ("consumed_by", "raw_owner"),
        ("outcome", "raw_outcome"),
    ] {
        let bytes = if field == "push_time" {
            [b"2026-09-28 09:15:".as_slice(), &[0xff, 0, b'A']].concat()
        } else {
            vec![0xff, 0, b'A']
        };
        // Only test-owned fixed field names enter SQL. No Diesel TEXT read of
        // the damaged value occurs, including in the assertion or diagnostics.
        f.db.get_conn()
            .unwrap()
            .batch_execute(&format!(
                "UPDATE main.pushed_stocks SET {field}=CAST(X'{}' AS TEXT)",
                hex::encode(&bytes)
            ))
            .unwrap();
        let mut session = paper_catalog6_session(&f.db).unwrap();
        let result = session.with_readonly_catalog6(
            |conn, proof| capture_scope(conn, proof, instant()),
            |_, _, _| Ok::<_, CandidateScopeError>(()),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("{field} malformed TEXT cannot yield a capture"),
        };
        assert!(
            matches!(
                error,
                PaperCatalog6ReadbackError::Consumer(CandidateScopeError::InvalidText)
            ),
            "{field}: {error:?}"
        );
        diesel::sql_query(format!("UPDATE main.pushed_stocks SET {field}=?"))
            .bind::<diesel::sql_types::Text, _>(original)
            .execute(&mut f.db.get_conn().unwrap())
            .unwrap();
    }
}

#[test]
fn f2_candidate_scope_retains_original_namespace_even_for_equal_content() {
    use crate::decision::pushed_candidate_scope_v1::{
        capture_pushed_candidate_scope_at_for_test as capture_scope, CandidateScopeError,
    };
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let original = Fixture::v6();
    let foreign = Fixture::v6();
    let original_capture = paper_catalog6_session(&original.db)
        .unwrap()
        .with_readonly_catalog6(
            |conn, proof| capture_scope(conn, proof, instant()),
            |conn, proof, value| value.verify_unchanged(conn, proof),
        )
        .unwrap();
    let mut session = paper_catalog6_session(&foreign.db).unwrap();
    let foreign_capture = session
        .with_readonly_catalog6(
            |conn, proof| capture_scope(conn, proof, instant()),
            |conn, proof, value| value.verify_unchanged(conn, proof),
        )
        .unwrap();
    // These are content IDs, while retained authority is nonserializable and
    // still identifies the original source objects independently of contents.
    assert_eq!(original_capture.id(), foreign_capture.id());
    let result = session.with_readonly_catalog6(
        |conn, proof| {
            let count = count_actual_sql(conn);
            confirm_counter_and_reset(conn, &count);
            let result = original_capture.verify_unchanged(conn, proof);
            assert_eq!(count.load(Ordering::SeqCst), 0);
            result
        },
        |_, _, _| Ok::<_, CandidateScopeError>(()),
    );
    assert!(matches!(
        result,
        Err(PaperCatalog6ReadbackError::Consumer(
            CandidateScopeError::SourceAuthority
        ))
    ));
}

#[test]
fn candidate_scope_schema_local_ddl_cannot_qualify_global6() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let operation_completed = std::cell::Cell::new(false);
    let result = session.with_immediate_catalog6(
        |conn, _, _| {
            crate::database::candidate_scope_observation_schema_v1::create_schema(conn)?;
            diesel::sql_query("PRAGMA user_version=7").execute(conn)?;
            operation_completed.set(true);
            Ok::<_, diesel::result::Error>(())
        },
        |_, _, _, _| Ok(()),
    );
    assert!(operation_completed.get());
    assert!(
        matches!(
            result,
            Err(PaperCatalog6TransactionError::BeforeCommit(
                PaperCatalog6Error::Catalog6RequalificationRequired
                    | PaperCatalog6Error::Catalog(_)
            ))
        ),
        "{result:?}"
    );
    drop(session);
    // The rollback retained the actual Catalog6 borrower and financial family.
    let mut session = paper_catalog6_session(&f.db).unwrap();
    session
        .with_readonly_catalog6(
            |conn, _| {
                assert_eq!(capture::int(conn, "SELECT user_version AS value FROM pragma_user_version")?, 6);
                assert_eq!(capture::int(conn, "SELECT COUNT(*) AS value FROM main.sqlite_master WHERE name GLOB 'candidate_scope_observations_v1*'")?, 0);
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _| Ok(()),
        )
        .unwrap();
}

#[test]
fn f2_candidate_scope_actual_limits_types_and_empty_calendar_refusal() {
    use crate::decision::pushed_candidate_scope_v1::{
        capture_pushed_candidate_scope_at_for_test as capture_scope, CandidateScopeError,
    };
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let empty = session
        .with_readonly_catalog6(
            |conn, proof| {
                capture_scope(
                    conn,
                    proof,
                    Utc.with_ymd_and_hms(2027, 1, 4, 1, 30, 0).unwrap(),
                )
            },
            |conn, proof, value| value.verify_unchanged(conn, proof),
        )
        .unwrap();
    let content: serde_json::Value = serde_json::from_slice(empty.canonical_bytes()).unwrap();
    assert_eq!(content["scope_state"], "empty_bounded_scope");
    assert_eq!(content["calendar"]["state"], "unavailable");
    assert!(content["candidates"].as_array().unwrap().is_empty());
    let closed = session
        .with_readonly_catalog6(
            |conn, proof| {
                capture_scope(
                    conn,
                    proof,
                    Utc.with_ymd_and_hms(2026, 10, 3, 1, 30, 0).unwrap(),
                )
            },
            |conn, proof, value| value.verify_unchanged(conn, proof),
        )
        .unwrap();
    let content: serde_json::Value = serde_json::from_slice(closed.canonical_bytes()).unwrap();
    assert_eq!(content["calendar"]["state"], "covered");
    assert_eq!(content["calendar"]["open"], false);
    {
        let mut conn = f.db.get_conn().unwrap();
        insert_candidate_scope_row(
            &mut conn,
            1,
            "2026-09-28 09:15:00.000",
            &"x".repeat(64 * 1024 + 1),
            None,
        );
    }
    let too_large = session.with_readonly_catalog6(
        |conn, proof| capture_scope(conn, proof, instant()),
        |_, _, _| Ok::<_, CandidateScopeError>(()),
    );
    assert!(matches!(
        too_large,
        Err(PaperCatalog6ReadbackError::Consumer(
            CandidateScopeError::Bounds
        ))
    ));
    f.db.get_conn()
        .unwrap()
        .batch_execute("UPDATE main.pushed_stocks SET metric_json=X'7B7D'")
        .unwrap();
    let wrong_type = session.with_readonly_catalog6(
        |conn, proof| capture_scope(conn, proof, instant()),
        |_, _, _| Ok::<_, CandidateScopeError>(()),
    );
    assert!(matches!(
        wrong_type,
        Err(PaperCatalog6ReadbackError::Consumer(
            CandidateScopeError::Bounds
        ))
    ));
    assert_eq!(
        capture::int(
            &mut f.db.get_conn().unwrap(),
            "SELECT COUNT(*) AS value FROM main.pushed_stocks"
        )
        .unwrap(),
        1
    );
}

#[test]
fn f2_candidate_scope_actual_last_sql_mutation_rolls_back() {
    use crate::decision::pushed_candidate_scope_v1::{
        capture_pushed_candidate_scope_at_for_test as capture_scope, CandidateScopeError,
    };
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    insert_candidate_scope_row(
        &mut f.db.get_conn().unwrap(),
        1,
        "2026-09-28 09:15:00.000",
        "{}",
        None,
    );
    let _hook = set_hook(|phase, conn| {
        if phase == TestPhase::BeforeTail {
            conn.batch_execute("UPDATE main.pushed_stocks SET metric_json='late_change'")?;
        }
        Ok(())
    });
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let outcome = session.with_immediate_catalog6(
        |conn, _, proof| capture_scope(conn, proof, instant()),
        |conn, _, proof, value| value.verify_unchanged(conn, proof),
    );
    assert!(matches!(
        outcome,
        Err(PaperCatalog6TransactionError::Consumer(
            CandidateScopeError::Changed
        ))
    ));
    assert_eq!(
        capture::int(
            &mut f.db.get_conn().unwrap(),
            "SELECT COUNT(*) AS value FROM main.pushed_stocks WHERE metric_json='{}'"
        )
        .unwrap(),
        1
    );
}

#[test]
fn f2_candidate_scope_foreign_loan_rejects_with_zero_sql() {
    use crate::decision::pushed_candidate_scope_v1::{
        capture_pushed_candidate_scope_at_for_test as capture_scope, CandidateScopeError,
    };
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut foreign = SqliteConnection::establish(":memory:").unwrap();
    let count = count_actual_sql(&mut foreign);
    confirm_counter_and_reset(&mut foreign, &count);
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let outcome = session.with_immediate_catalog6(
        |_, _, proof| capture_scope(&mut foreign, proof, instant()),
        |_, _, _, _| Ok::<_, CandidateScopeError>(()),
    );
    assert!(matches!(
        outcome,
        Err(PaperCatalog6TransactionError::Consumer(
            CandidateScopeError::Catalog(PaperCatalog6Error::ConnectionInstanceMismatch)
        ))
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
}
struct HookGuard;
impl Drop for HookGuard {
    fn drop(&mut self) {
        HOOK.with(|h| *h.borrow_mut() = None);
    }
}
fn set_hook(
    f: impl FnMut(TestPhase, &mut SqliteConnection) -> Result<(), PaperCatalog6Error> + 'static,
) -> HookGuard {
    HOOK.with(|h| {
        assert!(h.borrow().is_none());
        *h.borrow_mut() = Some(Box::new(f));
    });
    HookGuard
}
fn tail_count(conn: &mut SqliteConnection, expected: i64) -> Result<(), PaperCatalog6Error> {
    if capture::int(
        conn,
        "SELECT COUNT(*) AS value FROM stock_daily WHERE code='TEST_CODE_G6_SENTINEL'",
    )? == expected
    {
        Ok(())
    } else {
        Err(PaperCatalog6Error::Catalog(
            "consumer exact membership changed".into(),
        ))
    }
}

#[derive(diesel::QueryableByName)]
struct OriginalText {
    #[diesel(sql_type=diesel::sql_types::Text)]
    value: String,
}
#[derive(diesel::QueryableByName)]
struct OriginalBlob {
    #[diesel(sql_type=diesel::sql_types::Binary)]
    value: Vec<u8>,
}
fn original_payloads(db: &DatabaseManager) -> (Vec<Vec<String>>, Vec<Vec<Vec<u8>>>) {
    let mut conn = db.get_conn().unwrap();
    let texts = [
        "SELECT manifest_bytes AS value FROM paper_ledger_account ORDER BY account_id",
        "SELECT payload AS value FROM paper_ledger_event ORDER BY account_id,seq",
        "SELECT projection_bytes AS value FROM paper_ledger_head ORDER BY account_id",
    ]
    .into_iter()
    .map(|q| {
        diesel::sql_query(q)
            .load::<OriginalText>(&mut conn)
            .unwrap()
            .into_iter()
            .map(|r| r.value)
            .collect()
    })
    .collect();
    let blobs = [
        "SELECT descriptor_bytes AS value FROM paper_book_v2_fee_manifest ORDER BY singleton",
        "SELECT manifest_bytes AS value FROM paper_book_v2_account ORDER BY account_id",
        "SELECT payload AS value FROM paper_book_v2_event ORDER BY account_id,seq",
        "SELECT projection_bytes AS value FROM paper_book_v2_head ORDER BY account_id",
    ]
    .into_iter()
    .map(|q| {
        diesel::sql_query(q)
            .load::<OriginalBlob>(&mut conn)
            .unwrap()
            .into_iter()
            .map(|r| r.value)
            .collect()
    })
    .collect();
    (texts, blobs)
}

#[test]
fn global_catalog6_actual_migration_and_fresh_reader_preserve_original_financial_bytes() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v5();
    let original = original_payloads(&f.db);
    let old = crate::trading::paper_ledger::verified_v1_snapshot_on(
        &mut f.db.get_conn().unwrap(),
        &f.binding,
    )
    .unwrap();
    migrate_catalog6_for_isolated_test(&f.db).unwrap();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let view = session
        .with_readonly_catalog6(
            |conn, proof| {
                proof.validate_on(conn)?;
                crate::trading::paper_ledger::read_verified_original_v1_body_on(conn, &f.binding)
                    .map_err(|_| PaperCatalog6Error::Catalog("old V1 replay".into()))
            },
            |conn, proof, view| {
                proof.validate_on(conn)?;
                let fresh = crate::trading::paper_ledger::read_verified_original_v1_body_on(
                    conn, &f.binding,
                )
                .map_err(|_| PaperCatalog6Error::Catalog("old V1 replay".into()))?;
                if &fresh != view {
                    return Err(PaperCatalog6Error::Catalog("V1 reader changed".into()));
                }
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(original_payloads(&f.db), original);
    assert_eq!(view.version, old.version);
    assert_eq!(view.event_hash, old.event_hash);
    assert_eq!(
        capture::int(
            &mut f.db.get_conn().unwrap(),
            "SELECT user_version AS value FROM pragma_user_version"
        )
        .unwrap(),
        6
    );
    assert_eq!(
        capture::int(
            &mut f.db.get_conn().unwrap(),
            "SELECT COUNT(*) AS value FROM paper_book_v2_execution_event"
        )
        .unwrap(),
        0
    );
}
#[test]
fn global_catalog6_missing_constructor_origin_refuses_before_checkout() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v5();
    // This is a real isolated manager, but its private constructor marker is
    // deliberately removed. A path/header cannot recover it.
    let mut manager = Arc::try_unwrap(f.db).ok().unwrap();
    manager.isolated_p05_consumer_origin = None;
    assert!(matches!(
        paper_catalog6_session(&manager),
        Err(PaperCatalog6Error::Catalog6RequalificationRequired)
    ));
}
#[test]
fn global_catalog6_copied_writer_token_second_connection_is_not_same_loan() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let path =
        f.db.isolated_p05_consumer_origin
            .as_ref()
            .unwrap()
            .path
            .clone();
    session
        .with_immediate_catalog6(
            |conn, _, proof| {
                let token = crate::database::connection_attestation_token(conn).unwrap();
                let mut other = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
                crate::database::install_connection_attestation_token(&mut other, &token).unwrap();
                other
                    .batch_execute("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL")
                    .unwrap();
                assert_eq!(
                    capture::temporary_reference(conn, &mut CopyWork::new())?,
                    capture::temporary_reference(&mut other, &mut CopyWork::new())?
                );
                let queries = count_actual_sql(&mut other);
                confirm_counter_and_reset(&mut other, &queries);
                let before = proof.work.borrow().remaining_for_test();
                assert!(matches!(
                    proof.validate_on(&mut other),
                    Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                ));
                assert_eq!(proof.work.borrow().remaining_for_test(), before);
                assert_eq!(queries.load(Ordering::SeqCst), 0);
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _, _| Ok(()),
        )
        .unwrap();
}
#[test]
fn global_catalog6_copied_reader_token_and_writer_proof_are_separate_loans() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let path =
        f.db.isolated_p05_consumer_origin
            .as_ref()
            .unwrap()
            .path
            .clone();
    let writer_address = &mut *session.checkout.connection as *mut SqliteConnection as usize;
    session
        .with_readonly_catalog6(
            |conn, proof| {
                assert_ne!(conn as *mut SqliteConnection as usize, writer_address);
                let token = crate::database::connection_attestation_token(conn).unwrap();
                let mut other = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
                crate::database::install_connection_attestation_token(&mut other, &token).unwrap();
                other
                    .batch_execute(
                        "PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA query_only=ON",
                    )
                    .unwrap();
                assert_eq!(
                    capture::temporary_reference(conn, &mut CopyWork::new())?,
                    capture::temporary_reference(&mut other, &mut CopyWork::new())?
                );
                let queries = count_actual_sql(&mut other);
                confirm_counter_and_reset(&mut other, &queries);
                let before = proof.work.borrow().remaining_for_test();
                assert!(matches!(
                    proof.validate_on(&mut other),
                    Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                ));
                assert_eq!(proof.work.borrow().remaining_for_test(), before);
                assert_eq!(queries.load(Ordering::SeqCst), 0);
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _| Ok(()),
        )
        .unwrap();
}
#[test]
fn global_catalog6_same_transaction_body_is_visible_to_mandatory_tail() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    session
        .with_immediate_catalog6(
            |conn, _, _| {
                insert(conn, "2026-09-28")?;
                Ok::<_, PaperCatalog6Error>(1)
            },
            |conn, _, _, expected| tail_count(conn, *expected),
        )
        .unwrap();
    assert_eq!(f.daily_count(), 1);
}
#[test]
fn global_catalog6_last_legal_append_is_caught_by_exact_tail_and_rolled_back() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let guard = set_hook(|phase, conn| {
        if phase == TestPhase::BeforeTail {
            insert(conn, "2026-09-29")?;
        }
        Ok(())
    });
    let result = session.with_immediate_catalog6(
        |conn, _, _| {
            insert(conn, "2026-09-28")?;
            Ok::<_, PaperCatalog6Error>(1)
        },
        |conn, _, _, expected| tail_count(conn, *expected),
    );
    assert!(matches!(
        result,
        Err(PaperCatalog6TransactionError::Consumer(_))
    ));
    drop(guard);
    assert_eq!(f.daily_count(), 0);
}
#[test]
fn global_catalog6_postcommit_failure_keeps_original_persisted_fact() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let guard = set_hook(|phase, _| {
        if phase == TestPhase::AfterCommit {
            Err(PaperCatalog6Error::InjectedFailure)
        } else {
            Ok(())
        }
    });
    let result = session.with_immediate_catalog6(
        |conn, _, _| {
            insert(conn, "2026-09-28")?;
            Ok::<_, PaperCatalog6Error>(1)
        },
        |conn, _, _, expected| tail_count(conn, *expected),
    );
    assert!(matches!(
        result,
        Err(PaperCatalog6TransactionError::CommitOutcomeUnknown(
            PaperCatalog6Error::InjectedFailure
        ))
    ));
    drop(guard);
    assert_eq!(f.daily_count(), 1);
}
#[test]
fn global_catalog6_postcommit_consumer_failure_is_explicit_unknown() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let result = session.with_immediate_catalog6(
        |conn, _, _| {
            insert(conn, "2026-09-28")?;
            Ok::<_, PaperCatalog6Error>(1)
        },
        |conn, _, proof, expected| {
            tail_count(conn, *expected)?;
            if proof.purpose == Purpose::Reader {
                Err(PaperCatalog6Error::InjectedFailure)
            } else {
                Ok(())
            }
        },
    );
    assert!(matches!(
        result,
        Err(
            PaperCatalog6TransactionError::CommittedConsumerOutcomeUnknown(
                PaperCatalog6Error::InjectedFailure
            )
        )
    ));
    assert_eq!(f.daily_count(), 1);
}
#[test]
fn global_catalog6_temp_alias_and_attached_schema_refuse_before_callback() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    for attack in [
        "CREATE TEMP TABLE PAPER_BOOK_V2_EXECUTION_HEAD(fake INTEGER)",
        "ATTACH DATABASE ':memory:' AS extra",
    ] {
        let f = Fixture::v6();
        let mut session = paper_catalog6_session(&f.db).unwrap();
        session.checkout.connection.batch_execute(attack).unwrap();
        let mut called = false;
        let result = session.with_immediate_catalog6(
            |_, _, _| {
                called = true;
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _, _| Ok(()),
        );
        assert!(result.is_err());
        assert!(!called);
        assert_eq!(f.daily_count(), 0);
    }
}
#[test]
fn global_catalog6_changed_index_geometry_or_trigger_never_heals() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    for attack in [
        "DROP TRIGGER paper_book_v2_execution_event_no_update",
        "CREATE INDEX unknown_execution_index ON paper_book_v2_execution_event(account_id)",
    ] {
        let f = Fixture::v6();
        f.db.get_conn().unwrap().batch_execute(attack).unwrap();
        let mut session = paper_catalog6_session(&f.db).unwrap();
        let mut called = false;
        assert!(session
            .with_immediate_catalog6(
                |_, _, _| {
                    called = true;
                    Ok::<_, PaperCatalog6Error>(())
                },
                |_, _, _, _| Ok(())
            )
            .is_err());
        assert!(!called);
    }
}
#[test]
fn global_catalog6_catalog_copy_budget_is_shared_and_checked_before_body() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    session.work = RefCell::new(CopyWork::limited(64));
    let mut called = false;
    let result = session.with_immediate_catalog6(
        |_, _, _| {
            called = true;
            Ok::<_, PaperCatalog6Error>(())
        },
        |_, _, _, _| Ok(()),
    );
    assert!(matches!(
        result,
        Err(PaperCatalog6TransactionError::BeforeCommit(
            PaperCatalog6Error::CopyBudgetExceeded
        ))
    ));
    assert!(!called);
    assert_eq!(f.daily_count(), 0);
}
#[test]
fn global_catalog6_migration_failure_rolls_back_whole_extension_and_header() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v5();
    let guard = set_hook(|phase, _| {
        if phase == TestPhase::DuringMigration {
            Err(PaperCatalog6Error::InjectedFailure)
        } else {
            Ok(())
        }
    });
    assert!(matches!(
        migrate_catalog6_for_isolated_test(&f.db),
        Err(PaperCatalog6TransactionError::BeforeCommit(
            PaperCatalog6Error::InjectedFailure
        ))
    ));
    drop(guard);
    let mut conn = f.db.get_conn().unwrap();
    assert_eq!(
        capture::int(
            &mut conn,
            "SELECT user_version AS value FROM pragma_user_version"
        )
        .unwrap(),
        5
    );
    assert_eq!(capture::int(&mut conn,"SELECT COUNT(*) AS value FROM main.sqlite_schema WHERE lower(name) GLOB 'paper_book_v2_execution_*'").unwrap(),0);
    crate::database::paper_book_owner_schema_v2::verify_catalog_v5_on(&mut conn).unwrap();
}
#[test]
fn global_catalog6_future_header_and_corrupt_original_financial_replay_refuse() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    f.db.get_conn()
        .unwrap()
        .batch_execute("PRAGMA user_version=7")
        .unwrap();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let mut called = false;
    assert!(session
        .with_immediate_catalog6(
            |_, _, _| {
                called = true;
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _, _| Ok(())
        )
        .is_err());
    assert!(!called);
    let f = Fixture::v6();
    {
        let mut conn = f.db.get_conn().unwrap();
        conn.batch_execute("DROP TRIGGER paper_book_owner_v2_head_update; UPDATE paper_ledger_head SET projection_hash='corrupt-original'").unwrap();
        let sql = crate::database::paper_book_owner_schema_v2::V1_GUARD_STATEMENTS
            .iter()
            .find(|(_, n, _, _)| *n == "paper_book_owner_v2_head_update")
            .unwrap()
            .3;
        conn.batch_execute(sql).unwrap();
    }
    let mut session = paper_catalog6_session(&f.db).unwrap();
    assert!(session
        .with_readonly_catalog6(|_, _| Ok::<_, PaperCatalog6Error>(()), |_, _, _| Ok(()))
        .is_err());
}

#[test]
fn global_catalog6_oversized_catalog_field_is_rejected_before_callback_copy() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let sql = format!(
        "CREATE VIEW TEST_CODE_OVERSIZE_G6 AS SELECT '{}' AS value",
        "x".repeat(65 * 1024)
    );
    f.db.get_conn().unwrap().batch_execute(&sql).unwrap();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let mut called = false;
    let result = session.with_immediate_catalog6(
        |_, _, _| {
            called = true;
            Ok::<_, PaperCatalog6Error>(())
        },
        |_, _, _, _| Ok(()),
    );
    assert!(matches!(
        result,
        Err(PaperCatalog6TransactionError::BeforeCommit(
            PaperCatalog6Error::CopyBudgetExceeded
        ))
    ));
    assert!(!called);
    assert_eq!(f.daily_count(), 0);
}
#[test]
fn global_catalog6_readonly_last_sql_drift_is_not_an_old_observation() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let guard = set_hook(|phase, conn| {
        if phase == TestPhase::AfterRead {
            conn.batch_execute("PRAGMA query_only=OFF")?;
            insert(conn, "2026-09-29")?;
            conn.batch_execute("PRAGMA query_only=ON")?;
        }
        Ok(())
    });
    let result = session.with_readonly_catalog6(
        |conn, _| {
            tail_count(conn, 0)?;
            Ok::<_, PaperCatalog6Error>(0)
        },
        |conn, _, expected| tail_count(conn, *expected),
    );
    assert!(matches!(
        result,
        Err(PaperCatalog6ReadbackError::Consumer(_))
    ));
    drop(guard);
    assert_eq!(f.daily_count(), 0);
}
#[test]
fn global_catalog6_partial_execution_schema_cannot_be_adopted_by_migration() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v5();
    f.db.get_conn()
        .unwrap()
        .batch_execute(crate::database::paper_book_v2_execution_schema_v1::STATEMENTS[0].3)
        .unwrap();
    assert!(migrate_catalog6_for_isolated_test(&f.db).is_err());
    let mut conn = f.db.get_conn().unwrap();
    assert_eq!(
        capture::int(
            &mut conn,
            "SELECT user_version AS value FROM pragma_user_version"
        )
        .unwrap(),
        5
    );
    assert_eq!(capture::int(&mut conn,"SELECT COUNT(*) AS value FROM main.sqlite_schema WHERE name='paper_book_v2_execution_manifest'").unwrap(),1);
}

#[test]
fn global_catalog6_original_writer_witness_rejects_actual_retained_reader_without_sql() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    session
        .with_immediate_catalog6(
            |conn, authority, proof| {
                assert!(proof.connection_authority() == authority);
                let correct_queries = count_actual_sql(conn);
                proof.validate_on(conn)?;
                assert!(correct_queries.load(Ordering::SeqCst) > 0);
                let mut held = proof.source.readback_connection.lock().unwrap();
                let actual_reader = held.as_mut().unwrap();
                assert_ne!(
                    conn as *mut SqliteConnection as usize,
                    actual_reader as *mut SqliteConnection as usize
                );
                let queries = count_actual_sql(actual_reader);
                confirm_counter_and_reset(actual_reader, &queries);
                let before = proof.work.borrow().remaining_for_test();
                assert!(matches!(
                    proof.validate_on(actual_reader),
                    Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                ));
                assert_eq!(queries.load(Ordering::SeqCst), 0);
                assert_eq!(proof.work.borrow().remaining_for_test(), before);
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _, _| Ok(()),
        )
        .unwrap();
}

#[test]
fn global_catalog6_original_reader_witness_rejects_actual_branded_writer_without_sql() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut other_writer = f.db.attribution_checkout().unwrap();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    session
        .with_readonly_catalog6(
            |conn, proof| {
                let correct_queries = count_actual_sql(conn);
                proof.validate_on(conn)?;
                assert!(correct_queries.load(Ordering::SeqCst) > 0);
                let actual_writer = other_writer.connection_for_test();
                let queries = count_actual_sql(actual_writer);
                confirm_counter_and_reset(actual_writer, &queries);
                assert!(
                    crate::database::registered_descriptor_connection_authority(
                        proof.source,
                        actual_writer
                    )
                    .unwrap()
                        == *proof.connection_authority()
                );
                queries.store(0, Ordering::SeqCst);
                let before = proof.work.borrow().remaining_for_test();
                assert!(matches!(
                    proof.validate_on(actual_writer),
                    Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                ));
                assert_eq!(queries.load(Ordering::SeqCst), 0);
                assert_eq!(proof.work.borrow().remaining_for_test(), before);
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _| Ok(()),
        )
        .unwrap();
}

#[test]
fn global_catalog6_temp_identical_sql_with_changed_autoindex_collation_is_rejected() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    #[cfg(not(target_os = "macos"))]
    assert_temp_geometry_rejection_on_actual_connection();
    #[cfg(target_os = "macos")]
    {
        // macOS SQLite defaults to DEFENSIVE, which silently ignores
        // writable_schema=ON. Only this exact child's isolated fixture
        // receives the attack configuration; the parent and application do not.
        let native = tempfile::Builder::new()
            .prefix("TEST_CODE_temp_geometry_native_")
            .tempdir()
            .unwrap();
        let source = native.path().join("geometry.c");
        let library = native.path().join("geometry.dylib");
        std::fs::write(&source, TEMP_GEOMETRY_INTERPOSE).unwrap();
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x86_64"
        };
        let compiler = std::process::Command::new("/usr/bin/clang")
            .args(["-dynamiclib", "-arch", arch])
            .arg(&source)
            .args(["-lsqlite3", "-o"])
            .arg(&library)
            .output()
            .unwrap();
        assert!(
            compiler.status.success(),
            "native fixture compile failed: {}",
            String::from_utf8_lossy(&compiler.stderr)
        );
        let name =
            "database::global_schema_v1::paper_v6::tests::temp_geometry_writable_schema_child";
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                name,
                "--nocapture",
                "--test-threads=1",
            ])
            .env("TEST_CODE_TEMP_GEOMETRY_CHILD", "isolated_geometry_attack")
            .env("DYLD_INSERT_LIBRARIES", &library)
            .output()
            .unwrap();
        let stdout = String::from_utf8(child.stdout).unwrap();
        let stderr = String::from_utf8(child.stderr).unwrap();
        assert!(
            child.status.success(),
            "actual geometry child failed: {stdout}\n{stderr}"
        );
        assert!(stdout.contains("running 1 test"));
        assert!(stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"));
        assert_eq!(
            stdout
                .matches("TEST_CODE_TEMP_GEOMETRY_TYPED_REJECTION_CHECKED")
                .count(),
            1
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "only invoked by the exact parent with an isolated geometry attack configuration"]
fn temp_geometry_writable_schema_child() {
    assert_eq!(
        std::env::var("TEST_CODE_TEMP_GEOMETRY_CHILD").as_deref(),
        Ok("isolated_geometry_attack")
    );
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    assert_temp_geometry_rejection_on_actual_connection();
    println!("TEST_CODE_TEMP_GEOMETRY_TYPED_REJECTION_CHECKED");
}

fn assert_temp_geometry_rejection_on_actual_connection() {
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let result = session.with_immediate_catalog6(
        |conn, authority, proof| {
            capture::replace_temp_token_geometry_for_test(conn)?;
            // The original readable registration token is still accepted by
            // descriptor object checking. Only the additional geometry gate
            // can reject the actual TEMP NOCASE autoindex change.
            assert!(
                &crate::database::registered_descriptor_connection_authority(proof.source, conn)
                    .unwrap()
                    == authority
            );
            proof.validate_on(conn)
        },
        |_, _, _, _| Ok::<_, PaperCatalog6Error>(()),
    );
    assert!(
        matches!(result, Err(PaperCatalog6TransactionError::Consumer(PaperCatalog6Error::Catalog(ref detail))) if detail == "unexpected TEMP catalog"),
        "TEMP geometry rejection must be typed; actual result: {result:?}",
    );
    assert_eq!(f.daily_count(), 0);
    session
        .with_readonly_catalog6(|_, _| Ok::<_, PaperCatalog6Error>(()), |_, _, _| Ok(()))
        .unwrap();
}

// Test-only Mach-O interpose tuple, matching Apple's dyld static interpose ABI.
// Keep the system SQLite library and original VFS. No raw Rust handle access.
#[cfg(target_os = "macos")]
const TEMP_GEOMETRY_INTERPOSE: &str = r#"
#include <sqlite3.h>
#include <string.h>
static int geometry_test_open(const char *name, sqlite3 **db, int flags, const char *vfs) {
    int rc = sqlite3_open_v2(name, db, flags, vfs);
    if (rc != SQLITE_OK || db == 0 || *db == 0) return rc;
    const char *actual = sqlite3_db_filename(*db, "main");
    if (actual == 0) return rc;
    const char *slash = strrchr(actual, '/');
    if (slash == 0 || strcmp(slash + 1, "TEST_CODE_global6.db") != 0) return rc;
    const char *parent = slash;
    while (parent > actual && parent[-1] != '/') --parent;
    const char prefix[] = "TEST_CODE_global6_";
    if (slash - parent < sizeof(prefix) - 1 || memcmp(parent, prefix, sizeof(prefix) - 1) != 0) return rc;
    int effective = -1;
    rc = sqlite3_db_config(*db, SQLITE_DBCONFIG_DEFENSIVE, 0, &effective);
    if (rc == SQLITE_OK && effective == 0) return SQLITE_OK;
    sqlite3_close_v2(*db);
    *db = 0;
    return rc == SQLITE_OK ? SQLITE_ERROR : rc;
}
__attribute__((used)) static const struct {
    const void *replacement;
    const void *original;
} geometry_test_interpose __attribute__((section("__DATA,__interpose"))) = {
    (const void *)geometry_test_open, (const void *)sqlite3_open_v2
};
"#;

#[test]
fn global_catalog6_canonical_and_ancillary_copy_work_is_reserved_before_classifier() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut session = paper_catalog6_session(&f.db).unwrap();
    session
        .with_immediate_catalog6(
            |conn, _, proof| {
                let snapshot = capture::capture(
                    &proof.loan,
                    conn,
                    GlobalSchemaCatalogMode::Test,
                    &mut CopyWork::new(),
                )?;
                let (index_geometry_count, sqlite_owned_count) =
                    capture::ancillary_observation_counts_for_test(&snapshot);
                assert!(index_geometry_count > 0);
                assert!(sqlite_owned_count > 0);
                let original_four =
                    capture::initial_catalog_copy_work_for_test(&snapshot, proof.references)?;
                let complete = capture::classifier_copy_work_for_test(&snapshot, proof.references)?;
                assert!(complete > original_four);
                let mut old_limit = CopyWork::limited(original_four);
                assert!(matches!(
                    capture::classify_bounded(&snapshot, proof.references, &mut old_limit),
                    Err(PaperCatalog6Error::CopyBudgetExceeded)
                ));
                let mut cumulative = CopyWork::limited(complete * 2 - 1);
                assert!(matches!(
                    capture::classify_bounded(&snapshot, proof.references, &mut cumulative)?,
                    DatabaseHalfDiagnostic::AmendedDatabaseHalf(_)
                ));
                assert_eq!(cumulative.remaining_for_test(), complete - 1);
                assert!(matches!(
                    capture::classify_bounded(&snapshot, proof.references, &mut cumulative),
                    Err(PaperCatalog6Error::CopyBudgetExceeded)
                ));
                Ok::<_, PaperCatalog6Error>(())
            },
            |_, _, _, _| Ok(()),
        )
        .unwrap();
}

#[test]
fn global_catalog6_external_commit_before_retained_reader_snapshot_is_observed_not_old_success() {
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let child_db = Arc::clone(&f.db);
    let child_committed = Arc::new(AtomicUsize::new(0));
    let witnessed = Arc::clone(&child_committed);
    let guard = crate::database::install_retained_readback_after_checkpoint_hook(move || {
        let mut child = child_db.attribution_checkout().unwrap();
        child
            .immediate_transaction_with_authority(
                |_| PaperCatalog6Error::Authority,
                |conn, _| insert(conn, "2026-09-29"),
            )
            .unwrap();
        witnessed.store(1, Ordering::SeqCst);
    });
    let mut session = paper_catalog6_session(&f.db).unwrap();
    let result = session.with_immediate_catalog6(
        |conn, _, _| {
            insert(conn, "2026-09-28")?;
            Ok::<_, PaperCatalog6Error>(1)
        },
        |conn, _, _, expected| tail_count(conn, *expected),
    );
    assert!(matches!(
        result,
        Err(
            PaperCatalog6TransactionError::CommittedConsumerOutcomeUnknown(
                PaperCatalog6Error::Catalog(_)
            )
        )
    ));
    assert_eq!(child_committed.load(Ordering::SeqCst), 1);
    drop(guard);
    assert_eq!(f.daily_count(), 2);
}

#[test]
fn global_catalog6_original_retained_reader_witness_rejects_original_committing_writer_without_sql()
{
    let _serial = super::super::tests::PROSPECTIVE_TEST_SERIAL.lock().unwrap();
    let f = Fixture::v6();
    let mut committing = paper_catalog6_session(&f.db).unwrap();
    // Complete the original wrapper's real COMMIT and retained readback before
    // attempting a reader loan. Keep its exact committing checkout in place.
    committing
        .with_immediate_catalog6(
            |conn, _, _| {
                insert(conn, "2026-09-28")?;
                Ok::<_, PaperCatalog6Error>(1)
            },
            |conn, _, _, expected| tail_count(conn, *expected),
        )
        .unwrap();
    assert_eq!(f.daily_count(), 1);
    let mut observing = paper_catalog6_session(&f.db).unwrap();
    observing
        .with_readonly_catalog6(
            |conn, proof| {
                assert!(Arc::ptr_eq(proof.source, &committing.checkout.source));
                let correct_queries = count_actual_sql(conn);
                proof.validate_on(conn)?;
                assert!(correct_queries.load(Ordering::SeqCst) > 0);
                let original_writer = committing.checkout.connection_for_test();
                assert!(!std::ptr::eq(&*conn, &*original_writer));
                let queries = count_actual_sql(original_writer);
                confirm_counter_and_reset(original_writer, &queries);
                assert!(
                    crate::database::registered_descriptor_connection_authority(
                        proof.source,
                        original_writer,
                    )
                    .unwrap()
                        == *proof.connection_authority()
                );
                queries.store(0, Ordering::SeqCst);
                let before = proof.work.borrow().remaining_for_test();
                assert!(matches!(
                    proof.validate_on(original_writer),
                    Err(PaperCatalog6Error::ConnectionInstanceMismatch)
                ));
                assert_eq!(queries.load(Ordering::SeqCst), 0);
                assert_eq!(proof.work.borrow().remaining_for_test(), before);
                Ok::<_, PaperCatalog6Error>(())
            },
            |conn, _, _| tail_count(conn, 1),
        )
        .unwrap();
    assert_eq!(f.daily_count(), 1);
}

#[path = "global_schema_candidate_v7_tests.rs"]
mod candidate7_tests;
