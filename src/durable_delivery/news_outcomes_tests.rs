//! Actual isolated counted delivery and read-only qualified-price observations.
use super::*;
use crate::database::DatabaseManager;
use crate::monitor::news_outcomes::read_news_outcomes;
use diesel::RunQueryDsl;

fn d01_card(code: &str, label: &str) -> DeliveryEnvelope {
    let day = "2026-07-30";
    let content = format!("TEST_CODE D01 original {label}").into_bytes();
    let source=serde_json::to_vec(&serde_json::json!({"schema":"news-to-idea-v1","business_date":day,"code":code,"rendered_sha256":sha256_hex(&content)})).unwrap();
    let scope = if code.starts_with('6') {
        "SHANGHAI"
    } else {
        "SHENZHEN"
    };
    DeliveryEnvelope::new(
        day,
        PushKind::NewsToIdea,
        DeliverySubKind::None,
        format!("{scope}:EQUITY:{code}"),
        format!("news-to-idea:{day}:{code}:10:30"),
        sha256_hex(&source),
        source.clone(),
        sha256_hex(&source),
        content,
        false,
        None,
    )
    .unwrap()
}
fn database() -> (tempfile::TempDir, DatabaseManager) {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_news.db")).unwrap();
    (dir, db)
}
fn qualified_price(db: &DatabaseManager, code: &str, day: &str, price: f64, qualified: bool) {
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("INSERT INTO stock_daily(code,date,close,data_source) VALUES (?1,?2,?3,'TEST_CODE_synthetic')")
        .bind::<diesel::sql_types::Text,_>(code).bind::<diesel::sql_types::Text,_>(day).bind::<diesel::sql_types::Double,_>(price).execute(&mut conn).unwrap();
    if qualified {
        diesel::sql_query("INSERT INTO qualified_daily_trading_status(code,date,status,contract_version,source,source_at,observed_at,batch_id) VALUES (?1,?2,'trading','TEST_CODE','TEST_CODE','2026-08-06T07:00:00Z','2026-08-06T07:00:01Z','TEST_CODE')")
        .bind::<diesel::sql_types::Text,_>(code).bind::<diesel::sql_types::Text,_>(day).execute(&mut conn).unwrap();
    }
}
#[test]
fn news_outcomes_actual_accepted_card_matures_independently_and_raw_pool_never_adds_membership() {
    let f = Fixture::new("D01_OUTCOME_ACTUAL");
    let append = MemoryAppendPort::default();
    let card = d01_card("600000", "accepted");
    prepare_reserved(&f, &card, &append);
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    f.coordinator
        .resume_deliverable(&card.decision_identity, &[sink.clone()], now())
        .unwrap();
    reconcile_terminal(
        &f,
        &append,
        DecisionState::Delivered,
        &card.decision_identity,
    );
    let (_dir, db) = database();
    for (day, price) in [
        ("2026-07-30", 10.),
        ("2026-07-31", 11.),
        ("2026-08-04", 9.),
        ("2026-08-06", 12.),
    ] {
        qualified_price(&db, "600000", day, price, true);
    }
    diesel::sql_query("INSERT INTO pushed_stocks(push_time,push_kind,code,name,push_price,metric_json,source) VALUES ('2026-07-30','D-01','TEST_CODE_unsent','TEST_CODE',10,'{}','TEST_CODE')").execute(&mut db.get_conn().unwrap()).unwrap();
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 30).unwrap();
    assert_eq!(
        f.coordinator
            .read_news_to_idea_business_dates("2026-08-06", None, 1)
            .unwrap(),
        vec!["2026-07-30"]
    );
    assert!(f
        .coordinator
        .read_news_to_idea_business_dates("2026-08-06", Some("2026-07-30"), 1)
        .unwrap()
        .is_empty());
    assert!(f
        .coordinator
        .read_news_to_idea_business_dates("2026-08-06", None, 33)
        .is_err());
    let first = read_news_outcomes(
        &db,
        &f.coordinator,
        date,
        "2026-07-31T15:01:00+08:00".parse().unwrap(),
    )
    .unwrap();
    assert_eq!(
        (
            first.original_cards,
            first.physically_accepted_cards,
            first.rows.len()
        ),
        (1, 1, 1)
    );
    assert!((first.rows[0].windows[0].change_pct.unwrap() - 10.).abs() < 1e-9);
    assert_eq!(
        first.rows[0].windows[1].unavailable.as_deref(),
        Some("window_not_mature")
    );
    let before = audit_v4_upgrade_database_snapshot(&Connection::open(&f.database_path).unwrap());
    let report = read_news_outcomes(
        &db,
        &f.coordinator,
        date,
        "2026-08-06T15:01:00+08:00".parse().unwrap(),
    )
    .unwrap();
    assert!((report.rows[0].windows[1].change_pct.unwrap() + 10.).abs() < 1e-9);
    assert!((report.rows[0].windows[2].change_pct.unwrap() - 20.).abs() < 1e-9);
    assert!(
        !report.rows[0].original_push_price_available
            && !report.rows[0].original_publication_time_available
    );
    assert_eq!(
        audit_v4_upgrade_database_snapshot(&Connection::open(&f.database_path).unwrap()),
        before
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(report.rows[0].accepted_at, Some(now()));
    assert!(report.rows[0].terminal_evidence_sha256.is_some());
    assert_eq!(
        report.rows[0].occurrence_identity,
        card.schedule_occurrence_identity
    );
    let rendered = report.render_markdown().unwrap();
    assert!(rendered.contains("| T+5 | 1 | 0 |"));
    assert!(rendered.contains(&card.decision_identity));
}
#[test]
fn news_outcomes_bare_daily_prices_remain_unknown_and_reserved_cards_are_not_a_denominator() {
    let f = Fixture::new("D01_OUTCOME_UNKNOWN");
    let append = MemoryAppendPort::default();
    let card = d01_card("600000", "unknown");
    prepare_reserved(&f, &card, &append);
    let (_dir, db) = database();
    let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 30).unwrap();
    let at = "2026-08-06T15:01:00+08:00".parse().unwrap();
    let pending = read_news_outcomes(&db, &f.coordinator, date, at).unwrap();
    assert_eq!(pending.physically_accepted_cards, 0);
    assert!(pending.rows[0]
        .windows
        .iter()
        .all(|w| w.change_pct.is_none()));
    let sink = StaticSink::new(AuthoritativeSinkResult::Accepted(receipt(now())));
    f.coordinator
        .resume_deliverable(&card.decision_identity, &[sink], now())
        .unwrap();
    reconcile_terminal(
        &f,
        &append,
        DecisionState::Delivered,
        &card.decision_identity,
    );
    qualified_price(&db, "600000", "2026-07-30", 10., true);
    qualified_price(&db, "600000", "2026-07-31", 11., false);
    let report = read_news_outcomes(&db, &f.coordinator, date, at).unwrap();
    assert_eq!(report.physically_accepted_cards, 1);
    assert!(report.rows[0]
        .windows
        .iter()
        .all(|w| w.change_pct.is_none()));
    assert_eq!(
        report.rows[0].windows[0].unavailable.as_deref(),
        Some("exact_close_or_trading_authority_unavailable")
    );
}
#[test]
fn news_outcomes_rejects_reserved_card_scope_source_code_disagreement() {
    let f = Fixture::new("D01_OUTCOME_BINDING");
    let append = MemoryAppendPort::default();
    let mut card = d01_card("600000", "bad-scope");
    card.scope_key = "SHANGHAI:EQUITY:600519".into();
    // Rebuild the canonical decision under the conflicting but otherwise valid
    // generic envelope; the D-01 reader must validate the family binding too.
    card = DeliveryEnvelope::new(
        &card.business_date,
        card.push_kind,
        card.sub_kind,
        &card.scope_key,
        &card.schedule_occurrence_identity,
        &card.source_evidence_fingerprint,
        card.source_binding_canonical.clone(),
        &card.delivery_subject_hash,
        card.rendered_content.clone(),
        false,
        None,
    )
    .unwrap();
    prepare_reserved(&f, &card, &append);
    assert!(f.coordinator.read_news_to_idea_cards("2026-07-30").is_err());
}
