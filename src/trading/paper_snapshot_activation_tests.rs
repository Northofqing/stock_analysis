use super::*;
use crate::trading::paper_ledger::{
    EffectiveFillRequest, EffectiveFillScope, EffectiveHistory, RiskPolicyV1, SeedLot,
};
use chrono::TimeZone;

fn instant() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 9, 13, 30, 0).unwrap()
}
#[derive(QueryableByName)]
struct TextRow {
    #[diesel(sql_type=Text)]
    value: String,
}
#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type=BigInt)]
    value: i64,
}
fn count(conn: &mut SqliteConnection, sql: &str) -> i64 {
    diesel::sql_query(sql)
        .get_result::<Count>(conn)
        .unwrap()
        .value
}
fn text(conn: &mut SqliteConnection, sql: &str) -> String {
    diesel::sql_query(sql)
        .get_result::<TextRow>(conn)
        .unwrap()
        .value
}

struct Fixture {
    _dir: tempfile::TempDir,
    database: PathBuf,
    request: SnapshotPaperActivationRequest,
}
impl Fixture {
    fn open(&self, writable: bool) -> SqliteConnection {
        open_snapshot_activation_database(&self.database, writable).unwrap()
    }
}
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("TEST_CODE_snapshot_activation.db");
    let mut conn = SqliteConnection::establish(database.to_str().unwrap()).unwrap();
    schema::create_source_reference_on(&mut conn).unwrap();
    conn.batch_execute("CREATE TABLE TEST_CODE_untouched(id INTEGER PRIMARY KEY,payload TEXT); INSERT INTO TEST_CODE_untouched VALUES(1,'TEST_CODE_old_fact');
        INSERT INTO paper_trades(plan_id,code,name,direction,price,quantity,status,fill_price,virtual_reason,account_mode,data_mode,ts)
          VALUES('TEST_CODE_old_trade','TEST_CODE_000001','TEST_CODE_fixture','buy',9,100,'Filled',9,'TEST_CODE_legacy','Normal','Full','2026-10-08 02:00:00');
        INSERT INTO user_account_summary(effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source)
          VALUES('2026-10-09T21:13:00+08:00',11000,1000,10000,9.090909,-100,'TEST_CODE_user_confirmed_account');").unwrap();
    crate::database::order_audit::insert_order_audit_query(
        &mut conn,
        &crate::database::order_audit::OrderAuditRecord {
            business_order_id: "TEST_CODE_old_attempt",
            source: "TEST_CODE_fixture",
            decision_basis: "TEST_CODE_old_attempt",
            side: "buy",
            code: "TEST_CODE_000001",
            requested_price: 9.0,
            execution_price: None,
            quantity: 100,
            quote_observed_at: None,
            outcome: "Rejected",
            failure_reason: Some("TEST_CODE_old_failure"),
        },
    )
    .unwrap();
    let input = crate::portfolio::user_position_snapshot::user_position_snapshot_input_from_json(
        r#"{"schema_version":1,"effective_at":"2026-10-09T21:13:00+08:00","confirm_empty":false,"items":[{"code":"TEST_CODE_000001","name":"TEST_CODE_fixture","quantity":100,"cost_price":12.0}]}"#,
        DateTime::parse_from_rfc3339("2026-10-09T21:20:00+08:00").unwrap()).unwrap();
    diesel::sql_query("INSERT INTO user_position_snapshot(snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count) VALUES(?,'2026-10-09T21:13:00+08:00','2026-10-09T21:20:00+08:00','user_confirmed_full_snapshot',0,?,1)")
        .bind::<Text,_>(&input.snapshot_id).bind::<Text,_>(&input.evidence_sha256).execute(&mut conn).unwrap();
    diesel::sql_query("INSERT INTO user_position_snapshot_item VALUES(?,'TEST_CODE_000001','TEST_CODE_fixture',100,12)")
        .bind::<Text,_>(&input.snapshot_id).execute(&mut conn).unwrap();
    let summary = diesel::sql_query("SELECT id,effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source,recorded_at FROM user_account_summary WHERE id=1")
        .get_result::<SummaryFact>(&mut conn).unwrap();
    let snapshot = diesel::sql_query("SELECT id,snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count,recorded_at FROM user_position_snapshot WHERE id=1")
        .get_result::<SnapshotFact>(&mut conn).unwrap();
    let items = diesel::sql_query(
        "SELECT code,name,quantity,cost_price FROM user_position_snapshot_item ORDER BY code",
    )
    .load::<PositionFact>(&mut conn)
    .unwrap();
    let image_path = dir.path().join("TEST_CODE_original.jpg");
    std::fs::write(&image_path, b"TEST_CODE_same_batch_image_bytes").unwrap();
    let image_sha256 = schema::hash_bytes(b"TEST_CODE_same_batch_image_bytes");
    let evidence = serde_json::json!({"original_image_sha256":image_sha256,"effective_at":snapshot.effective_at,
        "confirmed_at":snapshot.confirmed_at,"snapshot_id":input.snapshot_id,"position_evidence_sha256":input.evidence_sha256,
        "market_prices_role":SNAPSHOT_MARK_SOURCE,"positions":[{"code":"TEST_CODE_000001","name":"TEST_CODE_fixture","quantity":100,
          "cost_price":"12.000","current_price":"10.000","market_value":"1000.00"}]}).to_string();
    let metadata = std::fs::metadata(&database).unwrap();
    let receipt = serde_json::json!({"status":"committed_and_verified","database":database,
        "database_identity":{"device":metadata.dev(),"inode":metadata.ino()},"effective_at":snapshot.effective_at,"confirmed_at":snapshot.confirmed_at,
        "position_row_id":1,"account_summary_row_id":1,"snapshot_id":input.snapshot_id,"image_sha256":image_sha256,
        "readback":{"summary":summary,"positions":snapshot,"items":items}}).to_string();
    let snapshot_evidence_path = dir.path().join("TEST_CODE_snapshot_evidence.json");
    let import_receipt_path = dir.path().join("TEST_CODE_import_receipt.json");
    std::fs::write(&snapshot_evidence_path, &evidence).unwrap();
    std::fs::write(&import_receipt_path, &receipt).unwrap();
    let cutover = time(&snapshot.effective_at).unwrap();
    let request = SnapshotPaperActivationRequest {
        schema_version: 1,
        summary_row_id: 1,
        snapshot_id: input.snapshot_id,
        source_batch_reference: "TEST_CODE_user_snapshot_image_batch".into(),
        source_evidence: SnapshotSourceEvidence {
            image_path,
            image_sha256,
            snapshot_evidence_path,
            snapshot_evidence_sha256: schema::hash_bytes(evidence.as_bytes()),
            import_receipt_path,
            import_receipt_sha256: schema::hash_bytes(receipt.as_bytes()),
        },
        establish_after_hours_close: true,
        seed: SeedManifest {
            account_id: "TEST_CODE_paper_account".into(),
            epoch_id: "TEST_CODE_paper_epoch".into(),
            command_id: "TEST_CODE_activate".into(),
            cutover_at: cutover,
            account_effective_at: cutover,
            positions_effective_at: cutover,
            source_reference: "TEST_CODE_user_snapshot_image_batch".into(),
            source_hash: String::new(),
            approved_by: "TEST_CODE_explicit_user".into(),
            cash: Money::from_cny(10000.0).unwrap(),
            original_total: Money::from_cny(11000.0).unwrap(),
            excluded_residual: None,
            lots: vec![SeedLot {
                code: "TEST_CODE_000001".into(),
                name: "TEST_CODE_fixture".into(),
                quantity: 100,
                reported_cost: Some(Money::from_cny(12.0).unwrap()),
                sellable_from: None,
                sellability_evidence: None,
            }],
            marks: vec![Mark {
                code: "TEST_CODE_000001".into(),
                price: Money::from_cny(10.0).unwrap(),
                observed_at: cutover,
                source: SNAPSHOT_MARK_SOURCE.into(),
            }],
            policy: RiskPolicyV1::default(),
        },
    };
    Fixture {
        _dir: dir,
        database,
        request,
    }
}
fn old_state(conn: &mut SqliteConnection) -> Vec<String> {
    [
        "SELECT json_group_array(json_array(type,name,tbl_name,sql)) AS value FROM (SELECT * FROM main.sqlite_master WHERE lower(name) NOT GLOB 'paper_ledger_*' AND lower(tbl_name) NOT GLOB 'paper_ledger_*' AND lower(name) NOT GLOB 'paper_snapshot_activation_*' AND lower(tbl_name) NOT GLOB 'paper_snapshot_activation_*' ORDER BY type,name)",
        "SELECT json_group_array(json_array(id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,virtual_reason,account_mode,data_mode,ts,updated_at)) AS value FROM paper_trades",
        "SELECT json_group_array(json_array(id,business_order_id,source,decision_basis,side,code,requested_price,execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at)) AS value FROM order_audit",
        "SELECT json_group_array(json_array(order_audit_id,previous_hash,record_hash,created_at)) AS value FROM order_audit_chain",
        "SELECT json_group_array(json_array(id,effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source,recorded_at)) AS value FROM user_account_summary",
        "SELECT json_group_array(json_array(id,snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count,recorded_at)) AS value FROM user_position_snapshot",
        "SELECT json_group_array(json_array(snapshot_id,code,name,quantity,cost_price)) AS value FROM user_position_snapshot_item",
        "SELECT json_group_array(json_array(id,payload)) AS value FROM TEST_CODE_untouched",
        "SELECT json_group_array(json_array(name,seq)) AS value FROM sqlite_sequence",
    ].iter().map(|sql| text(conn,sql)).collect()
}
fn prepared(fixture: &Fixture) -> SnapshotPaperActivationRequest {
    preview_snapshot_paper_activation(&mut fixture.open(false), &fixture.request, instant())
        .unwrap()
        .prepared_request
}

#[test]
fn real_file_bootstrap_is_atomic_independent_replayable_and_preserves_all_old_facts() {
    let fixture = fixture();
    let mut conn = fixture.open(true);
    let before = old_state(&mut conn);
    let preview =
        preview_snapshot_paper_activation(&mut conn, &fixture.request, instant()).unwrap();
    assert!(!preview.applied);
    assert!(!schema::is_present_on(&mut conn).unwrap());
    assert_eq!(old_state(&mut conn), before);
    assert_eq!(preview.projection.seed_equity.cny(), 11000.0);
    assert_eq!(preview.projection.unrealized_pnl().unwrap(), Money::ZERO);
    assert_eq!(preview.projection.lots[0].basis_price.cny(), 10.0);
    assert_eq!(
        preview.projection.lots[0].reported_cost.unwrap().cny(),
        12.0
    );
    assert_eq!(
        preview.projection.lots[0].sellable_from,
        NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()
    );
    assert!(apply_snapshot_paper_activation(&mut conn, &fixture.request, instant()).is_err());
    let applied =
        apply_snapshot_paper_activation(&mut conn, &preview.prepared_request, instant()).unwrap();
    assert!(applied.applied);
    assert_eq!(old_state(&mut conn), before);
    assert_eq!(
        count(
            &mut conn,
            "SELECT application_id AS value FROM pragma_application_id()"
        ),
        0
    );
    assert_eq!(
        count(
            &mut conn,
            "SELECT user_version AS value FROM pragma_user_version()"
        ),
        0
    );
    assert_eq!(
        count(
            &mut conn,
            "SELECT COUNT(*) AS value FROM paper_ledger_event"
        ),
        2
    );
    assert_eq!(applied.opening_close_date, Some(shanghai_day(instant())));
    assert_eq!(
        applied.projection.closes[&shanghai_day(instant())].cny(),
        11000.0
    );
    assert_eq!(
        applied.projection.marks["TEST_CODE_000001"].source,
        OPENING_CLOSE_SOURCE
    );
    drop(conn);
    let mut reopened = fixture.open(false);
    let view = paper_ledger::read_on(&mut reopened, &applied.binding).unwrap();
    assert_eq!(view.version, 2);
    assert_eq!(view.cash.cny(), 10000.0);
    let facts = paper_ledger::verified_effective_fills_on(
        &mut reopened,
        &EffectiveFillRequest {
            scope: EffectiveFillScope::Epoch(applied.binding.clone()),
            history: EffectiveHistory::RestatedLatest,
            as_of: shanghai_day(instant()),
        },
    )
    .unwrap();
    assert!(facts.rows().unwrap().is_empty());
    assert_eq!(facts.receipt().catalog_generation, 0);
    let mut reopened = fixture.open(true);
    let retry =
        apply_snapshot_paper_activation(&mut reopened, &preview.prepared_request, instant())
            .unwrap();
    assert!(!retry.applied);
    assert!(retry.already_applied);
    assert_eq!(retry.projection, applied.projection);
    assert_eq!(
        count(
            &mut reopened,
            "SELECT COUNT(*) AS value FROM paper_ledger_event"
        ),
        2
    );
    assert_eq!(old_state(&mut reopened), before);
}

#[test]
fn same_batch_source_proof_rejects_mixed_marks_costs_ids_hashes_and_stale_sources() {
    let fixture = fixture();
    for mutate in [0, 1, 2, 3, 4, 5, 6, 7] {
        let mut request = fixture.request.clone();
        match mutate {
            0 => request.seed.marks[0].price = Money::from_cny(12.0).unwrap(),
            1 => request.seed.lots[0].reported_cost = Some(Money::from_cny(10.0).unwrap()),
            2 => request.summary_row_id = 2,
            3 => request.source_evidence.image_sha256 = "a".repeat(64),
            4 => request.seed.cash = Money::from_cny(10001.0).unwrap(),
            5 => request.seed.lots[0].sellable_from = Some(shanghai_day(instant())),
            6 => request.seed.source_hash = "a".repeat(64),
            7 => request.seed.marks[0].observed_at = instant(),
            _ => unreachable!(),
        }
        assert!(
            preview_snapshot_paper_activation(&mut fixture.open(false), &request, instant())
                .is_err(),
            "case{mutate}"
        );
    }
    assert!(preview_snapshot_paper_activation(
        &mut fixture.open(false),
        &fixture.request,
        instant() + chrono::Duration::days(3)
    )
    .is_err());
    let request = prepared(&fixture);
    let mut conn = fixture.open(true);
    conn.batch_execute("INSERT INTO user_account_summary(effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source)
       VALUES('2026-10-09T21:21:00+08:00',20000,0,20000,0,0,'TEST_CODE_newer_summary')").unwrap();
    assert!(apply_snapshot_paper_activation(&mut conn, &request, instant()).is_err());
    assert!(!schema::is_present_on(&mut conn).unwrap());
}

#[test]
fn partial_extra_shadowed_or_changed_namespace_is_never_admitted() {
    for sql in [
        "CREATE TABLE paper_ledger_account(account_id TEXT)",
        "CREATE TABLE paper_snapshot_activation_v1(singleton INTEGER)",
        "CREATE TABLE paper_book_owner_v1(account_id TEXT)",
        "CREATE TABLE paper_book_v2_unapproved(x TEXT)",
        "CREATE TEMP TABLE user_account_summary(id INTEGER)",
        "CREATE TRIGGER TEST_CODE_extra_source AFTER INSERT ON user_account_summary BEGIN SELECT 1; END",
    ] {
        let fixture=fixture(); let mut conn=fixture.open(true); conn.batch_execute(sql).unwrap();
        assert!(preview_snapshot_paper_activation(&mut conn,&fixture.request,instant()).is_err(),"{sql}");
    }
    let fixture = fixture();
    let request = prepared(&fixture);
    let mut conn = fixture.open(true);
    let applied = apply_snapshot_paper_activation(&mut conn, &request, instant()).unwrap();
    let mut wrong = applied.binding.clone();
    wrong.epoch_id = "TEST_CODE_other_epoch".into();
    assert!(matches!(
        schema::require_owner_on(&mut conn, &wrong),
        Err(LedgerError::InactiveEpoch)
    ));
    assert!(conn
        .batch_execute("UPDATE paper_snapshot_activation_v1 SET epoch_id='TEST_CODE_other'")
        .is_err());
    assert!(conn.batch_execute("INSERT OR REPLACE INTO paper_snapshot_activation_v1 SELECT * FROM paper_snapshot_activation_v1").is_err());
    assert!(conn
        .batch_execute(
            "INSERT OR REPLACE INTO paper_ledger_account SELECT * FROM paper_ledger_account"
        )
        .is_err());
    conn.batch_execute("DROP TRIGGER paper_snapshot_activation_head_update")
        .unwrap();
    assert!(paper_ledger::read_on(&mut conn, &applied.binding).is_err());
    assert!(schema::require_owner_on(&mut conn, &applied.binding).is_err());
}

#[test]
fn source_failure_after_schema_creation_rolls_back_the_entire_installation() {
    let fixture = fixture();
    let request = prepared(&fixture);
    let mut conn = fixture.open(true);
    // An appended raw audit without a matching chain is a data failure, while
    // its unchanged schema still qualifies. Seed validation must roll DDL back.
    conn.batch_execute("INSERT INTO order_audit(business_order_id,source,decision_basis,side,code,requested_price,quantity,outcome,failure_reason)
       VALUES('TEST_CODE_corrupt_audit','TEST_CODE_fixture','TEST_CODE_fixture','buy','TEST_CODE_000001',10,100,'Rejected','TEST_CODE_missing_chain')").unwrap();
    let before = old_state(&mut conn);
    assert!(apply_snapshot_paper_activation(&mut conn, &request, instant()).is_err());
    assert!(!schema::is_present_on(&mut conn).unwrap());
    assert_eq!(
        count(
            &mut conn,
            "SELECT COUNT(*) AS value FROM sqlite_master WHERE name LIKE 'paper_ledger_%'"
        ),
        0
    );
    assert_eq!(old_state(&mut conn), before);
}

#[test]
fn newer_actual_snapshot_never_refills_or_invalidates_the_activated_paper_epoch() {
    let fixture = fixture();
    let request = prepared(&fixture);
    let mut conn = fixture.open(true);
    let applied = apply_snapshot_paper_activation(&mut conn, &request, instant()).unwrap();
    let before = paper_ledger::read_on(&mut conn, &applied.binding).unwrap();
    conn.batch_execute("INSERT INTO user_account_summary(effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source)
        VALUES('2026-10-12T15:00:00+08:00',20000,0,20000,0,0,'TEST_CODE_newer_actual');
        INSERT INTO user_position_snapshot(snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count)
        VALUES('TEST_CODE_newer_actual_snapshot','2026-10-12T15:00:00+08:00','2026-10-12T15:01:00+08:00','user_confirmed_full_snapshot',1,'TEST_CODE_newer_actual_evidence',0);").unwrap();
    assert_eq!(
        schema::verify_active_on(&mut conn).unwrap(),
        applied.binding
    );
    assert_eq!(
        paper_ledger::read_on(&mut conn, &applied.binding).unwrap(),
        before
    );
    let retry =
        apply_snapshot_paper_activation(&mut conn, &request, instant() + chrono::Duration::days(3))
            .unwrap();
    assert!(retry.already_applied);
    assert_eq!(retry.projection.cash, before.cash);
    assert_eq!(retry.projection.lots, before.lots);
}

#[test]
fn already_applied_preview_reports_the_current_paper_projection() {
    let fixture = fixture();
    let request = prepared(&fixture);
    let mut conn = fixture.open(true);
    let applied = apply_snapshot_paper_activation(&mut conn, &request, instant()).unwrap();
    let view = paper_ledger::read_on(&mut conn, &applied.binding).unwrap();
    let mut marks = request.seed.marks.clone();
    marks[0].price = Money::from_cny(11.0).unwrap();
    marks[0].observed_at = instant();
    marks[0].source = "TEST_CODE_later_market_mark".into();
    paper_ledger::mark_on(
        &mut conn,
        ValuationBatch {
            binding: applied.binding.clone(),
            command_id: "TEST_CODE_later_mark".into(),
            expected_version: view.version,
            inventory_fingerprint: view.inventory_fingerprint().unwrap(),
            as_of: instant(),
            closing: false,
            marks,
        },
        instant(),
    )
    .unwrap();
    let current = paper_ledger::read_on(&mut conn, &applied.binding).unwrap();
    let preview = preview_snapshot_paper_activation(&mut conn, &request, instant()).unwrap();
    assert!(preview.already_applied);
    assert_eq!(preview.projection, *current);
    assert_eq!(preview.projection.equity().unwrap().cny(), 11100.0);
}

#[test]
fn native_activation_refuses_unbound_raw_history_and_keeps_explicit_cutover_history() {
    let fixture = fixture();
    let request = prepared(&fixture);
    let mut conn = fixture.open(true);
    let raw = EffectiveFillRequest {
        scope: EffectiveFillScope::LegacyRaw,
        history: EffectiveHistory::AsKnown {
            ledger_version: None,
        },
        as_of: shanghai_day(instant()),
    };
    paper_ledger::verified_effective_fills_on(&mut conn, &raw)
        .expect("TEST_CODE valid uninitialized legacy fixture remains readable");
    let applied = apply_snapshot_paper_activation(&mut conn, &request, instant()).unwrap();
    assert!(
        matches!(paper_ledger::verified_effective_fills_on(&mut conn, &raw),
        Err(LedgerError::InvalidInput(message)) if message.contains("explicit bound economic scope"))
    );
    let prefix = EffectiveFillRequest {
        scope: EffectiveFillScope::LegacyBeforeCutover(applied.binding),
        history: EffectiveHistory::AsKnown {
            ledger_version: Some(2),
        },
        as_of: shanghai_day(instant()),
    };
    paper_ledger::verified_effective_fills_on(&mut conn, &prefix)
        .expect("TEST_CODE explicit cutover prefix remains readable after native activation");
}

#[test]
fn explicit_rehearsal_copy_preserves_original_source_provenance_and_reports_target_identity() {
    let fixture = fixture();
    let copy = fixture._dir.path().join("TEST_CODE_rehearsal_copy.db");
    std::fs::copy(&fixture.database, &copy).unwrap();
    let mut conn = open_snapshot_activation_database(&copy, true).unwrap();
    let preview =
        preview_snapshot_paper_activation(&mut conn, &fixture.request, instant()).unwrap();
    let applied =
        apply_snapshot_paper_activation(&mut conn, &preview.prepared_request, instant()).unwrap();
    assert_eq!(applied.database_identity.path, copy.canonicalize().unwrap());
    assert_ne!(
        applied.database_identity.inode,
        std::fs::metadata(&fixture.database).unwrap().ino()
    );
    assert_eq!(
        applied.prepared_request.source_evidence,
        fixture.request.source_evidence
    );
    assert!(!schema::is_present_on(&mut fixture.open(false)).unwrap());
    let missing = fixture._dir.path().join("TEST_CODE_missing.db");
    assert!(open_snapshot_activation_database(&missing, true).is_err());
    assert!(!missing.exists());
}
