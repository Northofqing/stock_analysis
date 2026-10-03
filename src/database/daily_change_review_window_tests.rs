use super::*;
use crate::data_gateway::ordinary_daily_change_window::tests::{
    acquire_fixture, acquire_fixture_at, database, now, reply,
};
use diesel::connection::SimpleConnection;
#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type=diesel::sql_types::BigInt)]
    n: i64,
}
fn rows(c: &mut SqliteConnection) -> i64 {
    diesel::sql_query("SELECT count(*) AS n FROM daily_change_review_event")
        .get_result::<Count>(c)
        .unwrap()
        .n
}
#[tokio::test]
async fn wg07_exact_replay_fresh_acquisition_ttl_and_late_append_rollback() {
    let q = acquire_fixture(reply).await;
    let (_d, path) = database();
    let mut conn =
        crate::data_gateway::ordinary_daily_change_window::open_existing(&path, false).unwrap();
    let r = prepare_window_on_conn(&mut conn, &q, now()).unwrap();
    assert_eq!(rows(&mut conn), 3);
    assert_eq!(
        prepare_window_on_conn(&mut conn, &q, now() + chrono::Duration::days(1)).unwrap(),
        r
    );
    assert_eq!(rows(&mut conn), 3);
    let q2 = acquire_fixture_at(reply, now() + chrono::Duration::days(1)).await;
    let r2 = prepare_window_on_conn(&mut conn, &q2, now() + chrono::Duration::days(1)).unwrap();
    assert_eq!(r2.candidates[0].candidate_id, r.candidates[0].candidate_id);
    assert_eq!(r2.candidates[0].expires_at, r.candidates[0].expires_at);
    assert_eq!(rows(&mut conn), 6);
    drop(conn);
    let mut conn =
        crate::data_gateway::ordinary_daily_change_window::open_existing(&path, false).unwrap();
    load(&mut conn).unwrap();
    let q3 = acquire_fixture(reply).await;
    let error=prepare_window_transaction(&mut conn,&q3,now(),|conn| {
        conn.batch_execute("CREATE TRIGGER TEST_CODE_late_failure BEFORE INSERT ON daily_change_review_event WHEN NEW.command_id LIKE 'window:%' BEGIN SELECT RAISE(ABORT,'TEST_CODE late append'); END;")?;
        Ok(())
    }).unwrap_err();
    assert!(error.to_string().contains("TEST_CODE late append"));
    assert_eq!(rows(&mut conn), 6);
    load(&mut conn).unwrap();
}
#[tokio::test]
async fn wg07_schema2_confirm_never_writes_legacy_and_requires_exact_fact() {
    let q = acquire_fixture(reply).await;
    let (_d, path) = database();
    let mut conn =
        crate::data_gateway::ordinary_daily_change_window::open_existing(&path, false).unwrap();
    let receipt = prepare_window_on_conn(&mut conn, &q, now()).unwrap();
    assert!(admit_window_on_conn(&mut conn, &q).is_err());
    for r in &receipt.candidates {
        decide_on_conn(
            &mut conn,
            &r.candidate_id,
            &r.evidence_token,
            ReviewDecision::Confirm,
            "TEST_CODE_op",
            "checked",
            now(),
        )
        .unwrap();
    }
    assert_eq!(admit_window_on_conn(&mut conn, &q).unwrap().len(), 2);
    let n: Count = diesel::sql_query("SELECT count(*) AS n FROM daily_change_confirmation")
        .get_result(&mut conn)
        .unwrap();
    assert_eq!(n.n, 0);
    let mut snap = q.candidates()[0].snapshot().clone();
    snap.query.daily_source = "different".into();
    assert!(admit_on_conn(&mut conn, &snap).is_err());
    let state = load(&mut conn).unwrap();
    assert!(state
        .candidates
        .values()
        .all(|c| c.decision.as_ref().unwrap().confirmation.is_none()));
}
#[tokio::test]
async fn wg07_window_event_replay_rejects_orphan_and_tampered_proof() {
    let q = acquire_fixture(reply).await;
    let (_d, path) = database();
    let mut conn =
        crate::data_gateway::ordinary_daily_change_window::open_existing(&path, false).unwrap();
    prepare_window_on_conn(&mut conn, &q, now()).unwrap();
    let mut state = load(&mut conn).unwrap();
    state.windows.clear();
    assert!(validate_window_closure(&state).is_err());
    let state = load(&mut conn).unwrap();
    let (mut proof, receipt) = state.windows.values().next().unwrap().clone();
    proof.response_hex.replace_range(..2, "ff");
    assert!(validate_window_receipt(&state, &proof, &receipt).is_err());
}
#[tokio::test]
async fn wg07_exact_six_seven_local_namespace_and_unknown_generation_reject() {
    let q = acquire_fixture(reply).await;
    for generation in [6, 7, 8] {
        let (_d, path) = database();
        let mut conn =
            crate::data_gateway::ordinary_daily_change_window::open_existing(&path, false).unwrap();
        conn.batch_execute(&format!(
            "PRAGMA application_id=1398035265; PRAGMA user_version={generation};"
        ))
        .unwrap();
        assert_eq!(
            prepare_window_on_conn(&mut conn, &q, now()).is_ok(),
            generation != 8
        );
    }
}

#[tokio::test]
async fn wg07_repeated_renewal_keeps_original_window_reference_and_decision() {
    let q = acquire_fixture(reply).await;
    let (_dir, path) = database();
    let mut conn =
        crate::data_gateway::ordinary_daily_change_window::open_existing(&path, false).unwrap();
    let original = prepare_window_on_conn(&mut conn, &q, now()).unwrap();
    for c in &original.candidates {
        let r1 = renew_on_conn(
            &mut conn,
            &c.candidate_id,
            &c.evidence_token,
            now() + chrono::Duration::days(7),
        )
        .unwrap();
        let r2 = renew_on_conn(
            &mut conn,
            &r1.candidate_id,
            &r1.evidence_token,
            now() + chrono::Duration::days(14),
        )
        .unwrap();
        assert_eq!(r2.snapshot, c.snapshot);
        assert_eq!(r2.revision, c.revision + 2);
        decide_on_conn(
            &mut conn,
            &r2.candidate_id,
            &r2.evidence_token,
            ReviewDecision::Confirm,
            "TEST_CODE_op",
            "renewed exact proof",
            now() + chrono::Duration::days(14),
        )
        .unwrap();
    }
    assert_eq!(admit_window_on_conn(&mut conn, &q).unwrap().len(), 2);
    drop(conn);
    let mut conn =
        crate::data_gateway::ordinary_daily_change_window::open_existing(&path, true).unwrap();
    let state = load(&mut conn).unwrap();
    assert_eq!(state.windows.len(), 1);
    assert_eq!(state.windows.values().next().unwrap().1, original);
}
#[tokio::test]
async fn wg07_changed_material_fact_revises_and_supersedes_old_review() {
    let q = acquire_fixture(reply).await;
    let (_dir, path) = database();
    let mut conn =
        crate::data_gateway::ordinary_daily_change_window::open_existing(&path, false).unwrap();
    let original = prepare_window_on_conn(&mut conn, &q, now()).unwrap();
    let changed =
        acquire_fixture(crate::data_gateway::ordinary_daily_change_window::tests::changed_reply)
            .await;
    let receipt = prepare_window_on_conn(&mut conn, &changed, now()).unwrap();
    assert_eq!(receipt.candidates.len(), 2);
    for (a, b) in original.candidates.iter().zip(&receipt.candidates) {
        assert_eq!(b.revision, a.revision + 1);
        assert_eq!(
            review_on_conn(&mut conn, &a.candidate_id, now())
                .unwrap()
                .status,
            "Superseded"
        );
    }
    assert!(admit_window_on_conn(&mut conn, &changed).is_err());
}

#[test]
fn wg07_literal_scope_golden_preserves_legacy_scope_domain() {
    let pair: window_contract::PairFact = window_contract::decode(
        include_bytes!("../../grpc_handoffs/fixtures/wg07/synthetic-pair-fact-v1.json"),
        window_contract::PROOF_LIMIT,
    )
    .unwrap();
    assert_eq!(
        hash(&(
            "scope",
            &pair.instrument,
            pair.previous.date,
            pair.current.date
        ))
        .unwrap(),
        "be0aa363001b9dbfe2506cd8952cfa550b3f4b211995f0f5a2a841a6806740d9"
    );
}

#[tokio::test]
async fn wg07_review_fix_i1_rehashed_observation_requires_exact_native_provenance() {
    let q = acquire_fixture(reply).await;
    for change_batch in [true, false] {
        let (_dir, path) = database();
        let mut conn = window::open_existing(&path, false).unwrap();
        prepare_window_on_conn(&mut conn, &q, now()).unwrap();
        let state = load(&mut conn).unwrap();
        let c = state.candidates.values().next().unwrap();
        let (pair, mut provenance, _) = window::snapshot_fact(&c.review.snapshot).unwrap();
        if change_batch {
            provenance.daily_batch_id = "TEST_CODE_forged_batch".into();
        } else {
            provenance.pair_evidence[0].fact_id = "TEST_CODE_forged_native_ref".into();
        }
        let snapshot = window::snapshot_for(&pair, &provenance).unwrap();
        // Stable identity, pair/query consistency, acquisition hash and command
        // are all valid. Only linkage to the original actual proof is false.
        assert_eq!(
            identities(&snapshot).unwrap(),
            identities(&c.review.snapshot).unwrap()
        );
        let key = acquisition(&snapshot).unwrap();
        append(
            &mut conn,
            &state,
            &c.review,
            &c.scope,
            "Observation",
            &format!("observation:{}:{key}", c.review.candidate_id),
            &Event::Observation {
                candidate_id: c.review.candidate_id.clone(),
                snapshot,
                observed_at: now(),
            },
        )
        .unwrap();
        assert_eq!(rows(&mut conn), 4);
        drop(conn);
        let mut reopened = window::open_existing(&path, true).unwrap();
        let error = load(&mut reopened).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("window snapshot native provenance"),
            "{error}"
        );
    }
}

#[test]
fn wg07_review_fix_i2_event_discriminator_precedes_owned_decode() {
    let scalar = "x".repeat(16385);
    let sessions = vec!["null"; 261].join(",");
    let payloads = [
        format!(r#" {{"proof":{{"bad":"{scalar}"}},"kind":"WindowObservation"}}"#),
        format!(
            r#"{{"snapshot":{{"schema\u005fversion" : 2,"bad":"{scalar}"}},"kind":"Observation"}}"#
        ),
        format!(
            r#"{{"review":{{"snapshot":{{"schema_version" : 2,"sessions":[{sessions}]}}}},"kind":"Candidate"}}"#
        ),
        format!(
            r#"{{"snapshot":{{"schema_version":1,"schema_version":2,"bad":"{scalar}"}},"kind":"Observation"}}"#
        ),
        format!(
            r#"{{"snapshot":{{"schema_version":2,"schema_version":1,"bad":"{scalar}"}},"kind":"Observation"}}"#
        ),
        format!(r#"{{"snapshot":{{"schema_version":1.0,"bad":"{scalar}"}},"kind":"Observation"}}"#),
        format!(r#"{{"snapshot":{{"schema_version":9,"bad":"{scalar}"}},"kind":"Observation"}}"#),
        format!(r#"{{"proof":{{"sessions":[{sessions}]}},"k\u0069nd":"WindowObservation"}}"#),
    ];
    for payload in payloads {
        EVENT_OWNED_DECODE_HITS.with(|hits| hits.set(0));
        assert!(decode_review_event(&payload).is_err());
        EVENT_OWNED_DECODE_HITS.with(|hits| assert_eq!(hits.get(), 0));
    }
    // Unknown nested versions and large strings in schema1 opaque evidence do
    // not opt the old contract into WG07 limits or change its canonical bytes.
    let q = crate::data_gateway::historical_bars::qualified_review_fixture();
    let mut snapshot = q.snapshot().clone();
    snapshot.raw_evidence = serde_json::json!({"schema_version":2,"kind":"WindowObservation","big":scalar,"sessions":vec![0;261],"schema\"version":2});
    let event = Event::Observation {
        candidate_id: "TEST_CODE_legacy".into(),
        snapshot,
        observed_at: now(),
    };
    let payload = json(&event).unwrap();
    assert!(!window_event_budget(payload.as_bytes()).unwrap());
    EVENT_OWNED_DECODE_HITS.with(|hits| hits.set(0));
    assert_eq!(
        json(&decode_review_event(&payload).unwrap()).unwrap(),
        payload
    );
    EVENT_OWNED_DECODE_HITS.with(|hits| assert_eq!(hits.get(), 1));
}
