//! BR-192 observable cutover guard.
//!
//! Counted delivery is an all-or-nothing production migration.  This test
//! intentionally inspects the production entry modules as a process-level
//! contract: the old in-memory budget owner must be gone and the monitor must
//! expose the durable coordinator at the sole counted-delivery seam.

use chrono::{TimeZone, Utc};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use stock_analysis::durable_delivery::{
    AuthoritativeDeliveryRequest, AuthoritativeSink, AuthoritativeSinkPort,
    AuthoritativeSinkResult, CoordinatorConfig, DecisionState, DeliveryEnvelope, DeliverySubKind,
    DurableDeliveryCoordinator, DurableDeliveryError, ImmutableAppendPort, PushKind, TypedReceipt,
    TypedUncertainty,
};

static NEXT_P01_TEST: AtomicUsize = AtomicUsize::new(1);
type ImmutableRecord = (String, Vec<u8>, String);

struct P01Fixture {
    coordinator: Option<DurableDeliveryCoordinator>,
    root: PathBuf,
    database_path: PathBuf,
    test_code: String,
}

impl P01Fixture {
    fn new(label: &str) -> Self {
        let sequence = NEXT_P01_TEST.fetch_add(1, Ordering::SeqCst);
        let test_code = format!(
            "TEST_CODE_P01_CUTOVER_{label}_{}_{}",
            std::process::id(),
            sequence
        );
        let root = PathBuf::from("data/test").join(&test_code);
        std::fs::create_dir_all("data/test").expect("create isolated test parent");
        std::fs::create_dir(&root).expect("create isolated P-01 test root");
        let database_path = root.join("durable_delivery.sqlite3");
        let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            &database_path,
            &test_code,
            format!("owner-{test_code}-0123456789abcdef"),
        ))
        .expect("open isolated P-01 coordinator");
        Self {
            coordinator: Some(coordinator),
            root,
            database_path,
            test_code,
        }
    }

    fn coordinator(&self) -> &DurableDeliveryCoordinator {
        self.coordinator.as_ref().expect("live P-01 coordinator")
    }

    fn restart(&mut self, label: &str) {
        drop(self.coordinator.take());
        self.coordinator = Some(
            DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                &self.database_path,
                &self.test_code,
                format!("owner-restart-{label}-0123456789abcdef"),
            ))
            .expect("reopen isolated P-01 coordinator"),
        );
    }
}

impl Drop for P01Fixture {
    fn drop(&mut self) {
        drop(self.coordinator.take());
        for suffix in ["-journal", "-shm", "-wal", ""] {
            let path = PathBuf::from(format!("{}{suffix}", self.database_path.display()));
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("remove isolated P-01 test file {}: {error}", path.display()),
            }
        }
        std::fs::remove_dir(&self.root).expect("remove isolated P-01 test root");
    }
}

#[derive(Default)]
struct MemoryAppendPort {
    records: Mutex<BTreeMap<String, ImmutableRecord>>,
}

impl ImmutableAppendPort for MemoryAppendPort {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> stock_analysis::durable_delivery::Result<String> {
        let mut records = self.records.lock().expect("append records");
        if let Some((stored_kind, stored_bytes, stored_sha256)) = records.get(identity) {
            if stored_kind != record_kind
                || stored_bytes.as_slice() != canonical_bytes
                || stored_sha256 != sha256
            {
                return Err(DurableDeliveryError::ImmutableAppendConflict(
                    identity.to_owned(),
                ));
            }
        } else {
            records.insert(
                identity.to_owned(),
                (
                    record_kind.to_owned(),
                    canonical_bytes.to_vec(),
                    sha256.to_owned(),
                ),
            );
        }
        Ok(format!("TEST_CODE_IMMUTABLE_REF_{identity}"))
    }
}

struct AppendThenErrorOnce {
    inner: MemoryAppendPort,
    failed: AtomicBool,
}

impl AppendThenErrorOnce {
    fn new() -> Self {
        Self {
            inner: MemoryAppendPort::default(),
            failed: AtomicBool::new(false),
        }
    }
}

impl ImmutableAppendPort for AppendThenErrorOnce {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> stock_analysis::durable_delivery::Result<String> {
        let immutable_ref =
            self.inner
                .append_exact(record_kind, identity, canonical_bytes, sha256)?;
        if record_kind == "DeliveryAcceptedAudit" && !self.failed.swap(true, Ordering::SeqCst) {
            return Err(DurableDeliveryError::Io(std::io::Error::other(
                "TEST_CODE_CRASH_AFTER_EXTERNAL_AUDIT_APPEND",
            )));
        }
        Ok(immutable_ref)
    }
}

struct AcceptedSink {
    calls: AtomicUsize,
    receipt: TypedReceipt,
}

impl AcceptedSink {
    fn new(receipt: TypedReceipt) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            receipt,
        })
    }
}

impl AuthoritativeSinkPort for AcceptedSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_P01_AUTHORITATIVE_SINK"
    }

    fn deliver(&self, _request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        AuthoritativeSinkResult::Accepted(self.receipt.clone())
    }
}

struct UncertainSink {
    calls: AtomicUsize,
}

impl UncertainSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
        })
    }
}

impl AuthoritativeSinkPort for UncertainSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_P01_UNCERTAIN_SINK"
    }

    fn deliver(&self, _request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        AuthoritativeSinkResult::Uncertain(TypedUncertainty {
            reason_code: "TEST_CODE_P01_TRANSPORT_UNCERTAIN".to_owned(),
            evidence: b"TEST_CODE_P01_UNCERTAINTY_EVIDENCE".to_vec(),
            observed_at: p01_now(),
        })
    }
}

fn p01_now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 18, 7, 30, 0)
        .single()
        .expect("fixed P-01 time")
}

fn p01_receipt() -> TypedReceipt {
    TypedReceipt {
        channel: "TEST_CODE_FEISHU".to_owned(),
        provider: "TEST_CODE_MAGICLAW".to_owned(),
        message_id: "TEST_CODE_P01_MESSAGE".to_owned(),
        platform_message_id: Some("TEST_CODE_P01_PLATFORM_MESSAGE".to_owned()),
        accepted_at: p01_now(),
        latency_ms: Some(19),
    }
}

fn p01_envelope(label: &str) -> DeliveryEnvelope {
    DeliveryEnvelope::new(
        "2026-08-18",
        PushKind::PreopenNewsHot,
        DeliverySubKind::None,
        "GLOBAL",
        "p01:2026-08-18",
        format!("TEST_CODE_P01_EVIDENCE_{label}"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": "P01_SOURCE_BINDING_V1",
            "render_mode": "Compensation"
        }))
        .expect("serialize P-01 source binding"),
        format!("TEST_CODE_P01_SUBJECT_{label}"),
        format!("TEST_CODE_P01_RENDERED_{label}").into_bytes(),
        false,
        None,
    )
    .expect("valid P-01 envelope")
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn p01_first_acceptance_has_one_sink_call_and_exact_join() {
    let fixture = P01Fixture::new("FIRST_ACCEPTANCE");
    let append = MemoryAppendPort::default();
    let envelope = p01_envelope("FIRST_ACCEPTANCE");
    assert_eq!(
        fixture
            .coordinator()
            .prepare(&envelope, 1, p01_now())
            .expect("prepare P-01")
            .state,
        DecisionState::Reserved
    );

    let receipt = p01_receipt();
    let sink = AcceptedSink::new(receipt.clone());
    let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
    let delivered = fixture
        .coordinator()
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            &sinks,
            &append,
            p01_now(),
        )
        .expect("resume P-01")
        .expect("P-01 claim exists");

    let receipt_canonical = serde_json::to_vec(&receipt).expect("serialize receipt");
    let mut expected_preimage = b"stock_analysis.counted_receipt.v1\0".to_vec();
    expected_preimage.extend_from_slice(&receipt_canonical);
    assert_eq!(delivered.state, DecisionState::Delivered);
    assert_eq!(delivered.sink_calls, 1);
    assert!(delivered.current_attempt_identity.is_some());
    assert_eq!(
        delivered.authoritative_receipt_sha256.as_deref(),
        Some(sha256_hex(&expected_preimage).as_str())
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn p01_second_same_day_run_is_preflight_deduped() {
    let fixture = P01Fixture::new("SAME_DAY_DEDUP");
    let append = MemoryAppendPort::default();
    let envelope = p01_envelope("SAME_DAY_DEDUP");
    fixture
        .coordinator()
        .prepare(&envelope, 1, p01_now())
        .expect("prepare first P-01");
    let first_sink = AcceptedSink::new(p01_receipt());
    let first_sinks: Vec<AuthoritativeSink> = vec![first_sink.clone()];
    let first = fixture
        .coordinator()
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            &first_sinks,
            &append,
            p01_now(),
        )
        .expect("deliver first P-01")
        .expect("first P-01 claim");
    assert_eq!(first.state, DecisionState::Delivered);
    assert_eq!(first.sink_calls, 1);

    let forbidden_sink = AcceptedSink::new(p01_receipt());
    let forbidden_sinks: Vec<AuthoritativeSink> = vec![forbidden_sink.clone()];
    let repeated = fixture
        .coordinator()
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            &forbidden_sinks,
            &append,
            p01_now(),
        )
        .expect("inspect repeated P-01")
        .expect("repeated P-01 claim");
    assert_eq!(repeated.state, DecisionState::Delivered);
    assert_eq!(repeated.sink_calls, 0);
    assert_eq!(
        repeated.authoritative_receipt_sha256,
        first.authoritative_receipt_sha256
    );
    assert_eq!(first_sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(forbidden_sink.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn p01_crash_after_remote_accept_before_audit_never_resends() {
    let mut fixture = P01Fixture::new("CRASH_AFTER_ACCEPT");
    let append = MemoryAppendPort::default();
    let envelope = p01_envelope("CRASH_AFTER_ACCEPT");
    fixture
        .coordinator()
        .prepare(&envelope, 1, p01_now())
        .expect("prepare P-01 before crash");
    fixture
        .coordinator()
        .reconcile_all_pending(&append, p01_now())
        .expect("persist prepare audit before sink");

    let accepted_sink = AcceptedSink::new(p01_receipt());
    let accepted_sinks: Vec<AuthoritativeSink> = vec![accepted_sink.clone()];
    let accepted = fixture
        .coordinator()
        .resume_deliverable(&envelope.decision_identity, &accepted_sinks, p01_now())
        .expect("persist remote acceptance");
    assert_eq!(accepted.sink_calls, 1);
    assert_eq!(accepted_sink.calls.load(Ordering::SeqCst), 1);
    assert_ne!(
        fixture
            .coordinator()
            .decision_state(&envelope.decision_identity)
            .expect("state after remote acceptance"),
        DecisionState::Delivered,
        "crash window must exist before immutable acceptance audit is reconciled"
    );

    fixture.restart("CRASH_AFTER_ACCEPT");
    let forbidden_sink = AcceptedSink::new(p01_receipt());
    let forbidden_sinks: Vec<AuthoritativeSink> = vec![forbidden_sink.clone()];
    let recovered = fixture
        .coordinator()
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            &forbidden_sinks,
            &append,
            p01_now(),
        )
        .expect("recover accepted P-01")
        .expect("accepted P-01 claim");
    assert_eq!(recovered.state, DecisionState::Delivered);
    assert_eq!(recovered.sink_calls, 0);
    assert!(recovered.authoritative_receipt_sha256.is_some());
    assert_eq!(forbidden_sink.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn p01_crash_after_audit_before_database_ack_never_resends() {
    let mut fixture = P01Fixture::new("CRASH_AFTER_AUDIT_APPEND");
    let append = AppendThenErrorOnce::new();
    let envelope = p01_envelope("CRASH_AFTER_AUDIT_APPEND");
    fixture
        .coordinator()
        .prepare(&envelope, 1, p01_now())
        .expect("prepare P-01 before audit crash");
    fixture
        .coordinator()
        .reconcile_all_pending(&append, p01_now())
        .expect("persist prepare audit before sink");
    let accepted_sink = AcceptedSink::new(p01_receipt());
    let accepted_sinks: Vec<AuthoritativeSink> = vec![accepted_sink.clone()];
    fixture
        .coordinator()
        .resume_deliverable(&envelope.decision_identity, &accepted_sinks, p01_now())
        .expect("persist authoritative P-01 acceptance");

    let error = fixture
        .coordinator()
        .reconcile_all_pending(&append, p01_now())
        .expect_err("simulate crash after immutable audit append before SQLite acknowledgement");
    assert!(
        error
            .to_string()
            .contains("TEST_CODE_CRASH_AFTER_EXTERNAL_AUDIT_APPEND"),
        "unexpected simulated crash error: {error}"
    );
    assert_eq!(accepted_sink.calls.load(Ordering::SeqCst), 1);
    assert_ne!(
        fixture
            .coordinator()
            .decision_state(&envelope.decision_identity)
            .expect("state before audit acknowledgement recovery"),
        DecisionState::Delivered
    );

    fixture.restart("CRASH_AFTER_AUDIT_APPEND");
    let forbidden_sink = AcceptedSink::new(p01_receipt());
    let forbidden_sinks: Vec<AuthoritativeSink> = vec![forbidden_sink.clone()];
    let recovered = fixture
        .coordinator()
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            &forbidden_sinks,
            &append,
            p01_now(),
        )
        .expect("recover externally appended P-01 audit")
        .expect("P-01 claim after audit crash");
    assert_eq!(recovered.state, DecisionState::Delivered);
    assert_eq!(recovered.sink_calls, 0);
    assert!(recovered.authoritative_receipt_sha256.is_some());
    assert_eq!(forbidden_sink.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn p01_uncertain_result_requires_reconciliation_and_never_blindly_resends() {
    let fixture = P01Fixture::new("UNCERTAIN_NO_RESEND");
    let append = MemoryAppendPort::default();
    let envelope = p01_envelope("UNCERTAIN_NO_RESEND");
    fixture
        .coordinator()
        .prepare(&envelope, 1, p01_now())
        .expect("prepare uncertain P-01");
    let uncertain_sink = UncertainSink::new();
    let uncertain_sinks: Vec<AuthoritativeSink> = vec![uncertain_sink.clone()];
    let uncertain = fixture
        .coordinator()
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            &uncertain_sinks,
            &append,
            p01_now(),
        )
        .expect("persist uncertain P-01")
        .expect("uncertain P-01 claim");
    assert_eq!(uncertain.state, DecisionState::UncertainManualReview);
    assert_eq!(uncertain.sink_calls, 1);
    assert!(uncertain.current_attempt_identity.is_some());
    assert!(uncertain.authoritative_receipt_sha256.is_none());
    assert_eq!(uncertain_sink.calls.load(Ordering::SeqCst), 1);

    let forbidden_sink = AcceptedSink::new(p01_receipt());
    let forbidden_sinks: Vec<AuthoritativeSink> = vec![forbidden_sink.clone()];
    let repeated = fixture
        .coordinator()
        .resume_business_date_once_claim(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            &forbidden_sinks,
            &append,
            p01_now(),
        )
        .expect("inspect uncertain P-01 without resend")
        .expect("uncertain P-01 claim still exists");
    assert_eq!(repeated.state, DecisionState::UncertainManualReview);
    assert_eq!(repeated.sink_calls, 0);
    assert!(repeated.authoritative_receipt_sha256.is_none());
    assert_eq!(forbidden_sink.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn counted_delivery_has_one_durable_owner_and_no_process_local_budget() {
    let push_templates = include_str!("../src/bin/monitor/push_templates.rs");
    let notify = include_str!("../src/bin/monitor/notify.rs");
    let runtime = include_str!("../src/bin/monitor/durable_delivery_runtime.rs");

    for retired in [
        "DAILY_BUDGET_COUNT",
        "DAILY_BUDGET_DAY",
        "reset_budget_if_new_day",
        "counts_against_daily_budget",
    ] {
        assert!(
            !push_templates.contains(retired),
            "BR-192 counted cutover still exposes retired owner {retired}"
        );
    }

    assert!(
        notify.contains("ReviewProviderTopN"),
        "BR-192 counted cutover is missing ReviewProviderTopN"
    );
    let counted_branch = notify
        .find("if crate::durable_delivery_runtime::is_counted_kind(kind)")
        .expect("BR-192 counted branch");
    let legacy_audit_health = notify
        .find("runtime_delivery_audit_health()")
        .expect("BR-144 legacy audit health branch");
    let legacy_l6 = notify
        .find("crate::l6_sink::sink_router().route")
        .expect("legacy L6 route");
    assert!(
        counted_branch < legacy_audit_health && counted_branch < legacy_l6,
        "counted delivery must leave the legacy audit/L6 path before either owner is consulted"
    );
    for required in ["DurableDeliveryCoordinator", "reconcile_all_pending"] {
        assert!(
            runtime.contains(required),
            "BR-192 counted cutover is missing durable production seam {required}"
        );
    }
}
