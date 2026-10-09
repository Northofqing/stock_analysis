//! Cross-task producer/consumer coverage. Synthetic fixtures grant no source authority.
use super::*;
#[allow(dead_code)]
#[path = "bin/weekly_outcome_review/registry.rs"]
mod registry;
#[allow(dead_code)]
#[path = "bin/weekly_outcome_review/report.rs"]
mod report;
#[allow(dead_code)]
#[path = "bin/weekly_outcome_review/scorecard.rs"]
mod scorecard;
use crate::database::attribution_reports::{AttributionDatabaseAccess, AttributionDatabaseSession};
use crate::llm::{
    bounded::{BoundedFailure, BoundedResponse, SingleAttemptPermit, Usage},
    LlmError, ReceiptBearingJson,
};
use std::os::unix::fs::PermissionsExt;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

struct LocalFake {
    calls: AtomicUsize,
    sizes: Mutex<Vec<Value>>,
}
#[async_trait::async_trait]
impl LlmProvider for LocalFake {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn model(&self) -> &str {
        "fixture-model"
    }
    fn bounded_endpoint(&self) -> Option<String> {
        Some("https://fake.invalid/chat/completions".into())
    }
    async fn chat_json(&self, _: &str, _: &str) -> Result<Value, LlmError> {
        panic!("legacy path")
    }
    async fn chat_json_bounded_with_receipt(
        &self,
        r: BoundedJsonRequest<'_>,
        permit: SingleAttemptPermit,
    ) -> Result<BoundedResponse, BoundedFailure> {
        // Mirror the actual bounded transport request, including escaping and model fields.
        let wire = json!({"model":self.model(),"messages":[{"role":"system","content":r.system},{"role":"user","content":r.user}],"max_tokens":r.limits.max_output_tokens,"n":1,"stream":false,"temperature":0.1,"response_format":{"type":"json_object"},"thinking":{"type":"disabled"}});
        let request_bytes = serde_json::to_vec(&wire).unwrap().len();
        assert!(request_bytes <= r.limits.max_request_bytes);
        assert!(permit.reservation().input_tokens <= r.limits.max_input_tokens);
        self.sizes.lock().unwrap().push(json!({"system_prompt_bytes":r.system.len()+r.user.len(),"serialized_request_bytes":request_bytes,"reserved_input_tokens":permit.reservation().input_tokens}));
        self.calls.fetch_add(1, Ordering::SeqCst);
        let raw = r#"{"claims":[],"inference_codes":["qualification_required"],"check_codes":["monetary_dispute"]}"#;
        Ok(BoundedResponse {
            reservation: permit.reservation().clone(),
            response: ReceiptBearingJson::test_fixture_requested_model(
                "fake",
                "fixture-model",
                "fixture-model",
                None,
                "local-fixture-response",
                r.system,
                r.user,
                raw,
                chrono::Utc::now(),
                chrono::Utc::now(),
            ),
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 10,
                total_tokens: 20,
                cache: None,
            },
        })
    }
}
const SCHEMA: &str = r#"
CREATE TABLE prediction_tracker(id INTEGER PRIMARY KEY,pred_date TEXT,target_date TEXT,stock_code TEXT,pred_direction TEXT,actual_change_t1 REAL,actual_change_t3 REAL,actual_change_t5 REAL,hit_t1 INTEGER,hit_t3 INTEGER,hit_t5 INTEGER);
CREATE TABLE stock_daily(code TEXT,date TEXT,close REAL,is_suspended INTEGER);
CREATE TABLE qualified_daily_trading_status(code TEXT,date TEXT,status TEXT,contract_version TEXT,source TEXT,source_at TEXT,observed_at TEXT,batch_id TEXT);
CREATE TABLE paper_trades(id INTEGER PRIMARY KEY,plan_id TEXT,code TEXT,name TEXT,direction TEXT,price REAL,quantity INTEGER,status TEXT,fill_price REAL,not_fill_reason TEXT,virtual_reason TEXT,account_mode TEXT,data_mode TEXT,ts TEXT,updated_at TEXT);
CREATE TABLE order_audit(id INTEGER PRIMARY KEY,business_order_id TEXT,source TEXT,decision_basis TEXT,side TEXT,code TEXT,requested_price REAL,execution_price REAL,quantity INTEGER,quote_observed_at TEXT,outcome TEXT,failure_reason TEXT,created_at TEXT);
CREATE TABLE order_audit_chain(order_audit_id INTEGER,previous_hash TEXT,record_hash TEXT,created_at TEXT);
"#;
#[tokio::test]
async fn actual_rust_producer_normal_sizes_and_three_arms() {
    let temporary = tempfile::tempdir().unwrap();
    let root = std::env::var_os("GOAL_FIRST_JOIN_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| temporary.path().to_path_buf());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let raw_path = root.join("signal_registry.toml");
    write_new_private(&raw_path, registry::DEFAULT_REGISTRY.as_bytes()).unwrap();
    std::fs::set_permissions(&raw_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let pricing: ReviewedPricing = serde_json::from_value(json!({"schema_version":"assistant-reviewed-pricing-v1","reviewed_by":"synthetic-fixture","contract_version":"fixture-v1","valid_until":"2099-01-01T00:00:00Z","provider":"fake","requested_model":"fixture-model","endpoint":"https://fake.invalid/chat/completions","upstream_models":["fixture-model"],"currency":"CNY","billing_scope":"prompt_completion_only_no_hidden_tokens","input_bound_method":"utf8_bytes_plus_reviewed_framing","framing_tokens":20,"max_output_tokens":8192,"input_micro_cny_per_million":1,"output_micro_cny_per_million":1,"fixed_max_micro_cny":1})).unwrap();
    let mut observations = vec![];
    for rows in [0, 76, 4096] {
        let db = root.join(format!("TEST_CODE_{rows}.db"));
        let writer = rusqlite::Connection::open(&db).unwrap();
        writer.execute_batch(SCHEMA).unwrap();
        for id in 1..=rows {
            writer.execute("INSERT INTO prediction_tracker VALUES(?1,'2026-09-11','2026-09-18','TEST_CODE_000001','up',NULL,NULL,NULL,NULL,NULL,NULL)",[id]).unwrap();
        }
        drop(writer);
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o400)).unwrap();
        let before = std::fs::read(&db).unwrap();
        let snapshot = bytes_sha256(&before);
        let session =
            AttributionDatabaseSession::open(&db, AttributionDatabaseAccess::ReadOnly).unwrap();
        let as_of = "2026-10-08T16:00:00+08:00";
        let period = report::Period::new(
            "2026-09-28".parse().unwrap(),
            "2026-10-04".parse().unwrap(),
            DateTime::parse_from_rfc3339(as_of).unwrap(),
        )
        .unwrap();
        let mut review = report::read(session.database(), period);
        review.input_source = Some(report::InputSource {
            database_path: db.display().to_string(),
            source_main_sha256: snapshot.clone(),
            original_source_label: Some("synthetic TEST_CODE".into()),
            temporary_snapshot_deleted_after_run: false,
            boundary: "detached synthetic read-only fixture",
        });
        scorecard::attach(
            &mut review,
            registry::RegistryInput::load(Some(&raw_path)).unwrap(),
            &snapshot,
        );
        assert_eq!(
            review.predictions.value.as_ref().unwrap().windows.len(),
            rows as usize * 3
        );
        let rb = serde_json::to_vec_pretty(&review).unwrap();
        let mb = serde_json::to_vec_pretty(review.evidence_manifest.as_ref().unwrap()).unwrap();
        let rp = root.join(format!("review-{rows}.json"));
        let mp = root.join(format!("manifest-{rows}.json"));
        // Producer output is intentionally not bounded by the assistant's 8MiB output cap.
        std::fs::write(&rp, &rb).unwrap();
        std::fs::set_permissions(&rp, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_new_private(&mp, &mb).unwrap();
        let pack = FrozenPack::load(
            &rp,
            &mp,
            Some(&raw_path),
            as_of,
            "2026-10-08".parse().unwrap(),
        )
        .unwrap();
        let report_value: Value = serde_json::from_slice(&rb).unwrap();
        for (scope, evidence) in report_value["evidence_manifest"]["metric_scopes"]
            .as_object()
            .unwrap()
        {
            assert_eq!(evidence["input_snapshot_sha256"], snapshot);
            let base = scope.split("/*").next().unwrap();
            assert!(
                report_value.pointer(base).is_some(),
                "missing scope {scope}"
            );
        }
        let fake = LocalFake {
            calls: AtomicUsize::new(0),
            sizes: Mutex::new(vec![]),
        };
        let offline = compare(
            &pack,
            None,
            None,
            cli_limits(0),
            Instant::now() + Duration::from_secs(18),
        )
        .await
        .unwrap();
        assert_eq!(offline["run_reservations"]["attempt_slots_issued"], 0);
        let result = compare(
            &pack,
            Some(&fake),
            Some(&pricing),
            cli_limits(100),
            Instant::now() + Duration::from_secs(18),
        )
        .await
        .unwrap();
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
        assert_eq!(result["arms"][1]["mode"], "bounded_agent");
        assert_eq!(result["arms"][2]["mode"], "bounded_agent");
        assert_eq!(result["run_reservations"]["refunds_micro_cny"], 0);
        let cb = serde_json::to_vec_pretty(&result).unwrap();
        let md = markdown(&result).unwrap();
        write_new_private(&root.join(format!("comparison-{rows}.json")), &cb).unwrap();
        write_new_private(&root.join(format!("comparison-{rows}.md")), md.as_bytes()).unwrap();
        observations.push(json!({"predictions":rows,"windows":rows*3,"report_bytes":rb.len(),"manifest_bytes":mb.len(),"comparison_json_bytes":cb.len(),"comparison_md_bytes":md.len(),"requests":*fake.sizes.lock().unwrap(),"snapshot_sha256":snapshot}));
        let too_low = compare(
            &pack,
            Some(&fake),
            Some(&pricing),
            cli_limits(0),
            Instant::now() + Duration::from_secs(18),
        )
        .await
        .unwrap();
        assert_eq!(too_low["run_reservations"]["attempt_slots_issued"], 0);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
        assert_eq!(std::fs::read(&db).unwrap(), before);
        assert_eq!(std::fs::read(&rp).unwrap(), rb);
        assert_eq!(std::fs::read(&mp).unwrap(), mb);
    }
    let summary = serde_json::to_vec_pretty(&observations).unwrap();
    write_new_private(&root.join("actual-rust-join-sizes.json"), &summary).unwrap();
    println!("{}", String::from_utf8(summary).unwrap());
}
#[test]
fn finite_caps_and_private_raw_resource_reject_before_parsing() {
    let dir = tempfile::tempdir().unwrap();
    for (name, cap) in [
        ("report", MAX_REPORT_BYTES),
        ("manifest", MAX_MANIFEST_BYTES),
    ] {
        let p = dir.path().join(name);
        write_new_private(&p, b"{}").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&p)
            .unwrap()
            .set_len(cap as u64 + 1)
            .unwrap();
        assert!(read_private(&p, cap).is_err());
    }
    let p = dir.path().join("registry");
    write_new_private(&p, b"{}").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_private(&p, 128 * 1024).is_err());
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o400)).unwrap();
    std::fs::hard_link(&p, dir.path().join("alias")).unwrap();
    assert!(read_private(&p, 128 * 1024).is_err());
    let out = dir.path().join("output");
    assert!(write_new_private(&out, &vec![0; MAX_OUTPUT_BYTES + 1]).is_err());
    assert!(!out.exists());
}
