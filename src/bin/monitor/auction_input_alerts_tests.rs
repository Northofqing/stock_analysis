use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use stock_analysis::durable_delivery::{
    compiled_policy_catalog, AuthoritativeDeliveryRequest, AuthoritativeSink,
    AuthoritativeSinkPort, AuthoritativeSinkResult, CoordinatorConfig, DecisionState,
    DeliveryEnvelope, DeliverySubKind, DurableDeliveryCoordinator, DurableDeliveryError,
    ImmutableAppendPort, PushKind, TypedUncertainty,
};

fn at(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn start() -> DateTime<Utc> {
    at("2026-10-09T01:20:00Z")
}

fn slot() -> Slot {
    Slot::at(start()).expect("fixed verified A-share trading window")
}

fn unavailable(input: Input, now: DateTime<Utc>) -> Fact {
    next_fact(
        slot(),
        input,
        Observation::Unavailable(Reason::AcquisitionUnavailable),
        now,
        |_| Ok(None),
    )
    .unwrap()
    .unwrap()
}

fn prepared(input: Input, now: DateTime<Utc>, mode: DataMode) -> PreparedAuctionInputAlert {
    PreparedAuctionInputAlert::prepare(slot(), unavailable(input, now), mode).unwrap()
}

fn sources(report: &PreparedAuctionInputAlert) -> BTreeMap<String, Vec<u8>> {
    let source: serde_json::Value =
        serde_json::from_slice(report.binding.source_binding_canonical()).unwrap();
    BTreeMap::from([(
        source["fact_fingerprint"].as_str().unwrap().to_owned(),
        report.binding.source_binding_canonical().to_vec(),
    )])
}

fn envelope(binding: &CountedDeliveryBinding, text: &str) -> DeliveryEnvelope {
    DeliveryEnvelope::new(
        binding.business_date().to_string(),
        PushKind::DataMode,
        DeliverySubKind::None,
        "GLOBAL",
        binding.schedule_occurrence_identity(),
        binding.source_evidence_fingerprint(),
        binding.source_binding_canonical().to_vec(),
        binding.delivery_subject_hash(),
        text.as_bytes().to_vec(),
        binding.retry_authorized(),
        None,
    )
    .unwrap()
}

fn request(envelope: &DeliveryEnvelope) -> AuthoritativeDeliveryRequest {
    AuthoritativeDeliveryRequest {
        decision_identity: envelope.decision_identity.clone(),
        attempt_identity: "TEST_CODE_AUCTION_INPUT_ATTEMPT".into(),
        fence_token: 1,
        push_kind: envelope.push_kind,
        stable_template_id: envelope.push_kind.stable_template_id().into(),
        rendered_content: envelope.rendered_content.clone(),
        rendered_content_sha256: envelope.rendered_content_sha256.clone(),
    }
}

#[test]
fn auction_input_repeated_fault_keeps_one_persisted_episode() {
    let old = prepared(Input::PositionQuotes, start(), DataMode::Unsafe);
    let persisted = sources(&old);
    for _ in 0..10 {
        assert!(
            next_fact(
                slot(),
                Input::PositionQuotes,
                Observation::Unavailable(Reason::AcquisitionUnavailable),
                start() + chrono::Duration::seconds(30),
                |key| Ok(persisted.get(key).cloned()),
            )
            .unwrap()
            .is_none(),
            "another tick cannot open another fault envelope"
        );
    }
}

#[test]
fn auction_input_operations_have_separate_persistent_faults() {
    let old = prepared(Input::PositionQuotes, start(), DataMode::Unsafe);
    let persisted = sources(&old);
    let fact = next_fact(
        slot(),
        Input::VolumeCandidates,
        Observation::Unavailable(Reason::VolumeRatioUnavailable),
        start() + chrono::Duration::seconds(30),
        |key| Ok(persisted.get(key).cloned()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(fact.input, Input::VolumeCandidates);
    assert_eq!(fact.episode, 1);
    assert_eq!(fact.phase, Phase::Unavailable);
    assert_ne!(
        fingerprint(slot(), Input::PositionQuotes, 1, Phase::Unavailable),
        fingerprint(slot(), Input::VolumeCandidates, 1, Phase::Unavailable),
    );
}

#[test]
fn auction_input_empty_then_missing_ratio_is_a_new_frozen_state_without_false_recovery() {
    let first = next_fact(
        slot(),
        Input::VolumeCandidates,
        Observation::Unavailable(Reason::CandidatesEmpty),
        start(),
        |_| Ok(None),
    )
    .unwrap()
    .unwrap();
    let first = PreparedAuctionInputAlert::prepare(slot(), first, DataMode::Unsafe).unwrap();
    assert!(first.text.contains("量能候选为空"));
    for forbidden in [
        "RPC",
        "服务端",
        "资金",
        "仓位",
        "Unsafe→",
        "Unsafe →",
        "token",
    ] {
        assert!(!first.text.contains(forbidden));
    }
    let mut persisted = sources(&first);
    let second = next_fact(
        slot(),
        Input::VolumeCandidates,
        Observation::Unavailable(Reason::VolumeRatioUnavailable),
        start() + chrono::Duration::seconds(30),
        |key| Ok(persisted.get(key).cloned()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(second.phase, Phase::Unavailable);
    assert_eq!(second.episode, 2);
    assert_eq!(
        second.preceding_unavailable_sha256.as_deref(),
        Some(hash(first.binding.source_binding_canonical()).as_str())
    );
    let second = PreparedAuctionInputAlert::prepare(slot(), second, DataMode::Full).unwrap();
    assert!(second.text.contains("缺少有效量比"));
    persisted.extend(sources(&second));
    assert!(next_fact(
        slot(),
        Input::VolumeCandidates,
        Observation::Unavailable(Reason::VolumeRatioUnavailable),
        start() + chrono::Duration::seconds(60),
        |key| Ok(persisted.get(key).cloned())
    )
    .unwrap()
    .is_none());
    let recovered = next_fact(
        slot(),
        Input::VolumeCandidates,
        Observation::Fresh {
            observed_at: start() + chrono::Duration::seconds(90),
            batch_sha256: hash(b"TEST_CODE_REAL_NEW_BATCH"),
        },
        start() + chrono::Duration::seconds(90),
        |key| Ok(persisted.get(key).cloned()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(recovered.episode, 2);
    assert_eq!(
        recovered.preceding_unavailable_sha256.as_deref(),
        Some(hash(second.binding.source_binding_canonical()).as_str())
    );
}

#[test]
fn auction_input_observed_instant_supports_original_unix_millisecond_contract() {
    let instant = stock_analysis::data_gateway::parse_evidence_instant(
        "LimitPools",
        stock_analysis::market_domain::ProviderId::Eastmoney,
        "observed_at",
        &format!("unix-ms:{}", start().timestamp_millis()),
    )
    .unwrap();
    assert_eq!(instant, start());
    assert_eq!(Slot::at(instant), Some(slot()));
}

#[test]
fn auction_input_fresh_batch_recovers_exact_fault_then_next_fault_opens_episode_two() {
    let old = prepared(Input::PositionQuotes, start(), DataMode::Degraded);
    let mut persisted = sources(&old);
    let now = start() + chrono::Duration::seconds(30);
    let recovered = next_fact(
        slot(),
        Input::PositionQuotes,
        Observation::Fresh {
            observed_at: now,
            batch_sha256: hash(b"TEST_CODE_NEW_QUOTE_BATCH"),
        },
        now,
        |key| Ok(persisted.get(key).cloned()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(recovered.phase, Phase::Recovered);
    assert_eq!(recovered.episode, 1);
    assert_eq!(recovered.reason, Reason::FreshBatch);
    assert_eq!(
        recovered.preceding_unavailable_sha256.as_deref(),
        Some(hash(old.binding.source_binding_canonical()).as_str())
    );
    let recovery = PreparedAuctionInputAlert::prepare(slot(), recovered, DataMode::Unsafe).unwrap();
    persisted.extend(sources(&recovery));
    assert!(
        next_fact(
            slot(),
            Input::PositionQuotes,
            Observation::Unavailable(Reason::AcquisitionUnavailable),
            now,
            |key| Ok(persisted.get(key).cloned()),
        )
        .unwrap()
        .is_none(),
        "a callback from before recovery cannot reopen the episode"
    );
    let second = next_fact(
        slot(),
        Input::PositionQuotes,
        Observation::Unavailable(Reason::AcquisitionUnavailable),
        now + chrono::Duration::seconds(30),
        |key| Ok(persisted.get(key).cloned()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(second.phase, Phase::Unavailable);
    assert_eq!(second.episode, 2);
}

#[test]
fn auction_input_old_future_wrong_day_and_invalid_hash_batches_cannot_recover() {
    let old = prepared(
        Input::PositionQuotes,
        start() + chrono::Duration::seconds(60),
        DataMode::Unsafe,
    );
    let persisted = sources(&old);
    let now = start() + chrono::Duration::seconds(120);
    for (observed_at, batch_sha256) in [
        (start(), hash(b"TEST_CODE_OLD_BATCH")),
        (
            start() + chrono::Duration::seconds(60),
            hash(b"TEST_CODE_EQUAL_BATCH"),
        ),
        (
            start() + chrono::Duration::seconds(61),
            hash(b"TEST_CODE_STALE_AFTER_FAULT"),
        ),
        (
            now + chrono::Duration::seconds(1),
            hash(b"TEST_CODE_FUTURE_BATCH"),
        ),
        (
            at("2026-10-08T01:22:00Z"),
            hash(b"TEST_CODE_WRONG_DAY_BATCH"),
        ),
        (now, "TEST_CODE_NOT_SHA256".into()),
    ] {
        assert!(next_fact(
            slot(),
            Input::PositionQuotes,
            Observation::Fresh {
                observed_at,
                batch_sha256
            },
            now,
            |key| Ok(persisted.get(key).cloned()),
        )
        .unwrap()
        .is_none());
    }
}

#[test]
fn auction_input_recovery_of_another_operation_and_initial_success_are_neutral() {
    let old = prepared(Input::VolumeCandidates, start(), DataMode::Unsafe);
    let persisted = sources(&old);
    assert!(next_fact(
        slot(),
        Input::PositionQuotes,
        Observation::Fresh {
            observed_at: start() + chrono::Duration::seconds(30),
            batch_sha256: hash(b"TEST_CODE_QUOTE_ONLY")
        },
        start() + chrono::Duration::seconds(30),
        |key| Ok(persisted.get(key).cloned()),
    )
    .unwrap()
    .is_none());
}

#[tokio::test]
async fn auction_input_empty_position_scope_is_neutral_without_runtime_or_rpc() {
    // The empty branch returns before asking for a runtime, account or provider.
    position_batch(
        &crate::market_data::ScannerPositionQuotes::NoPositions,
        start(),
    )
    .await;
}

#[test]
fn auction_input_binding_keeps_real_mode_without_inventing_mode_transition() {
    for mode in [DataMode::Full, DataMode::Degraded, DataMode::Unsafe] {
        let report = prepared(Input::PositionQuotes, start(), mode);
        report.validate_at(start()).unwrap();
        let source: serde_json::Value =
            serde_json::from_slice(report.binding.source_binding_canonical()).unwrap();
        assert_eq!(source["old"], format!("{mode:?}"));
        assert_eq!(source["new"], format!("{mode:?}"));
        assert_eq!(
            source["fact_fingerprint"],
            fingerprint(slot(), Input::PositionQuotes, 1, Phase::Unavailable)
        );
        assert!(report
            .binding
            .schedule_occurrence_identity()
            .contains(&format!(":{mode:?}:")));
        assert!(!report.binding.retry_authorized());
        assert!(report.binding.task_binding().is_none());
        assert_eq!(
            report.binding.source_evidence_fingerprint(),
            hash(report.binding.source_binding_canonical())
        );
    }
}

fn stock(code: &str, volume_ratio: Option<f64>) -> stock_analysis::market_data::TopStock {
    stock_analysis::market_data::TopStock {
        code: code.into(),
        name: format!("TEST_CODE_NAME_{code}"),
        price: 10.0,
        change_pct: 1.0,
        volume_ratio,
        main_net_yi: None,
    }
}

#[test]
fn auction_input_all_valid_notified_is_not_fault_but_missing_ratio_never_recovers() {
    let notified = std::collections::HashSet::from(["TEST_CODE_VALID".to_owned()]);
    let all_notified = crate::push_templates::prepare_auction_volume_snapshot(
        "09:20:00",
        &[stock("TEST_CODE_VALID", Some(2.0))],
        &notified,
    );
    assert!(all_notified.is_err());
    assert_eq!(volume_reason(&all_notified), None);
    let missing = crate::push_templates::prepare_auction_volume_snapshot(
        "09:20:30",
        &[stock("TEST_CODE_MISSING", None)],
        &notified,
    );
    assert_eq!(
        volume_reason(&missing),
        Some(Reason::VolumeRatioUnavailable)
    );
    let mixed = crate::push_templates::prepare_auction_volume_snapshot(
        "09:20:30",
        &[
            stock("TEST_CODE_VALID", Some(2.0)),
            stock("TEST_CODE_MISSING", None),
        ],
        &notified,
    );
    assert_eq!(volume_reason(&mixed), Some(Reason::VolumeRatioUnavailable));
    let old = prepared(Input::VolumeCandidates, start(), DataMode::Degraded);
    let persisted = sources(&old);
    let changed = next_fact(
        slot(),
        Input::VolumeCandidates,
        Observation::Unavailable(volume_reason(&missing).unwrap()),
        start() + chrono::Duration::seconds(30),
        |key| Ok(persisted.get(key).cloned()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        changed.phase,
        Phase::Unavailable,
        "a newer pool that still lacks ratios cannot recover the volume input"
    );
    assert_eq!(changed.reason, Reason::VolumeRatioUnavailable);
}

#[test]
fn auction_input_stored_guard_enforces_exact_day_window_text_and_hash_binding() {
    let report = prepared(Input::PositionQuotes, start(), DataMode::Unsafe);
    let item = envelope(&report.binding, &report.text);
    let original = item.canonical_bytes().unwrap();
    let send = request(&item);
    validate_stored_canonical(&original, &send, start()).unwrap();
    for now in [
        at("2026-10-09T01:19:59Z"),
        at("2026-10-09T01:25:00Z"),
        at("2026-10-12T01:20:00Z"),
    ] {
        assert!(validate_stored_canonical(&original, &send, now).is_err());
    }
    let mut changed = send.clone();
    changed.rendered_content = b"TEST_CODE_REPLACED_TEXT".to_vec();
    assert!(validate_stored_canonical(&original, &changed, start()).is_err());
    changed.rendered_content_sha256 = hash(&changed.rendered_content);
    assert!(validate_stored_canonical(&original, &changed, start()).is_err());
    let mut changed_identity = send.clone();
    changed_identity.decision_identity = hash(b"TEST_CODE_OTHER_OWNER");
    assert!(validate_stored_canonical(&original, &changed_identity, start()).is_err());
    for field in [
        "source_binding_sha256",
        "source_evidence_fingerprint",
        "delivery_subject_hash",
        "rendered_content_sha256",
        "schedule_occurrence_identity",
    ] {
        let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
        changed[field] = serde_json::Value::String(hash(b"TEST_CODE_CHANGED_HASH"));
        assert!(
            validate_stored_canonical(&serde_json::to_vec(&changed).unwrap(), &send, start())
                .is_err(),
            "{field}"
        );
    }
    let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    changed["retry_authorized"] = serde_json::Value::Bool(true);
    assert!(
        validate_stored_canonical(&serde_json::to_vec(&changed).unwrap(), &send, start()).is_err()
    );
}

#[test]
fn auction_input_stored_guard_leaves_ordinary_data_mode_without_metadata_unchanged() {
    let text = "TEST_CODE ordinary DataMode at 00:03";
    let binding = crate::push_templates::build_data_mode_counted_binding(
        slot().date,
        None,
        DataMode::Unsafe,
        &hash(b"TEST_CODE_ORDINARY_MODE_FACT"),
        text,
    )
    .unwrap();
    let item = envelope(&binding, text);
    assert!(binding.retry_authorized());
    validate_stored_canonical(
        &item.canonical_bytes().unwrap(),
        &request(&item),
        at("2026-10-09T16:03:00Z"),
    )
    .unwrap();
}

#[derive(Default)]
struct MemoryAppend(Mutex<BTreeMap<String, (String, Vec<u8>, String)>>);
impl ImmutableAppendPort for MemoryAppend {
    fn append_exact(
        &self,
        kind: &str,
        identity: &str,
        bytes: &[u8],
        digest: &str,
    ) -> stock_analysis::durable_delivery::Result<String> {
        assert_eq!(hash(bytes), digest);
        let mut records = self.0.lock().unwrap();
        let record = (kind.to_owned(), bytes.to_vec(), digest.to_owned());
        match records.get(identity) {
            Some(old) if old != &record => {
                return Err(DurableDeliveryError::ImmutableAppendConflict(
                    identity.into(),
                ))
            }
            Some(_) => {}
            None => {
                records.insert(identity.into(), record);
            }
        }
        Ok(format!("immutable://{kind}/{identity}"))
    }
}

struct MemoryUncertainSink {
    calls: Arc<AtomicUsize>,
    observed_at: DateTime<Utc>,
}
impl AuthoritativeSinkPort for MemoryUncertainSink {
    fn sink_identity(&self) -> &str {
        "TEST_CODE_AUCTION_INPUT_MEMORY_UNCERTAIN"
    }
    fn deliver(&self, _: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        AuthoritativeSinkResult::Uncertain(TypedUncertainty {
            reason_code: "TEST_CODE_UNKNOWN_RECEIPT".into(),
            evidence: b"TEST_CODE_MEMORY_ONLY_NO_MAGICLAW".to_vec(),
            observed_at: self.observed_at,
        })
    }
}

struct TestStore {
    directory: PathBuf,
    retained: std::fs::File,
    code: String,
}
impl TestStore {
    fn new() -> Self {
        let code = format!(
            "TEST_CODE_AUCTION_INPUT_SCHEMA9_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        );
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/test");
        std::fs::create_dir_all(&parent).unwrap();
        let directory = parent.join(&code);
        std::fs::create_dir(&directory).unwrap();
        let retained = std::fs::File::open(&directory).unwrap();
        let result = Self {
            directory,
            retained,
            code,
        };
        let connection = rusqlite::Connection::open(result.path()).unwrap();
        connection
            .execute_batch(include_str!(
                "../../../contracts/durable_monitor_v9/schema.sql"
            ))
            .unwrap();
        for row in compiled_policy_catalog() {
            connection
                .execute(
                    "INSERT INTO delivery_policy_catalog VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    rusqlite::params![
                        row.push_kind.as_str(),
                        row.sub_kind.as_str(),
                        row.cooldown_scope.as_str(),
                        row.base_cooldown_secs,
                        row.override_cooldown_secs,
                        row.window_mode.as_str(),
                        i64::from(row.counts_against_daily_budget),
                        row.policy_version
                    ],
                )
                .unwrap();
        }
        connection.pragma_update(None, "user_version", 9).unwrap();
        result
    }
    fn path(&self) -> PathBuf {
        self.directory.join("durable_delivery.sqlite3")
    }
    fn open(&self, label: &str) -> DurableDeliveryCoordinator {
        DurableDeliveryCoordinator::open_existing_monitor_schema9(CoordinatorConfig::test(
            self.path(),
            &self.code,
            format!("TEST_CODE_AUCTION_INPUT_OWNER_{label}_0123456789abcdef"),
        ))
        .unwrap()
    }
    fn catalog(&self) -> Vec<(String, String, Option<String>)> {
        let connection = rusqlite::Connection::open(self.path()).unwrap();
        let mut statement = connection
            .prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
}
impl Drop for TestStore {
    fn drop(&mut self) {
        use std::os::unix::fs::MetadataExt;
        let retained = self.retained.metadata().unwrap();
        let current = std::fs::symlink_metadata(&self.directory).unwrap();
        assert!(
            current.is_dir() && retained.dev() == current.dev() && retained.ino() == current.ino()
        );
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn persist_unknown(
    coordinator: &DurableDeliveryCoordinator,
    append: &MemoryAppend,
    item: &DeliveryEnvelope,
    now: DateTime<Utc>,
    calls: Arc<AtomicUsize>,
) {
    assert_eq!(
        coordinator.prepare(item, 1, now).unwrap().state,
        DecisionState::Reserved
    );
    coordinator.reconcile_all_pending(append, now).unwrap();
    let sinks: Vec<AuthoritativeSink> = vec![Arc::new(MemoryUncertainSink {
        calls,
        observed_at: now,
    })];
    assert_eq!(
        coordinator
            .resume_deliverable(&item.decision_identity, &sinks, now)
            .unwrap()
            .sink_calls,
        1
    );
    coordinator.reconcile_all_pending(append, now).unwrap();
    assert_eq!(
        coordinator.decision_state(&item.decision_identity).unwrap(),
        DecisionState::UncertainManualReview
    );
}

#[test]
fn auction_input_actual_schema9_unknown_reopen_cross_mode_preserves_frozen_owner_and_catalog() {
    let store = TestStore::new();
    let catalog = store.catalog();
    let append = MemoryAppend::default();
    let coordinator = store.open("FIRST");
    let ordinary_text = "TEST_CODE ordinary DataMode Unsafe at 00:03";
    let ordinary_binding = crate::push_templates::build_data_mode_counted_binding(
        slot().date,
        None,
        DataMode::Unsafe,
        &hash(b"TEST_CODE_ORDINARY_STATUS_FACT"),
        ordinary_text,
    )
    .unwrap();
    let ordinary = envelope(&ordinary_binding, ordinary_text);
    let ordinary_calls = Arc::new(AtomicUsize::new(0));
    persist_unknown(
        &coordinator,
        &append,
        &ordinary,
        at("2026-10-08T16:03:00Z"),
        Arc::clone(&ordinary_calls),
    );
    let report = prepared(Input::PositionQuotes, start(), DataMode::Degraded);
    let item = envelope(&report.binding, &report.text);
    assert_ne!(ordinary.decision_identity, item.decision_identity);
    let calls = Arc::new(AtomicUsize::new(0));
    persist_unknown(&coordinator, &append, &item, start(), Arc::clone(&calls));
    let namespace = crate::durable_delivery_runtime::RuntimeNamespace::Test {
        test_code: store.code.clone(),
    };
    validate_authoritative_request(&namespace, &request(&item), start()).unwrap();
    assert!(validate_authoritative_request(
        &namespace,
        &request(&item),
        at("2026-10-09T01:25:00Z")
    )
    .is_err());
    validate_authoritative_request(&namespace, &request(&ordinary), at("2026-10-09T08:00:00Z"))
        .unwrap();
    assert_eq!(store.catalog(), catalog);
    drop(coordinator);

    let reopened = store.open("REOPEN");
    let fault_fingerprint = fingerprint(slot(), Input::PositionQuotes, 1, Phase::Unavailable);
    let frozen = crate::durable_delivery_runtime::auction_input_occurrence_source_from(
        &reopened,
        slot().date,
        &fault_fingerprint,
    )
    .unwrap()
    .unwrap();
    assert_eq!(frozen, item.source_binding_canonical);
    let would_be_new_mode = prepared(
        Input::PositionQuotes,
        start() + chrono::Duration::seconds(30),
        DataMode::Unsafe,
    );
    assert_ne!(
        would_be_new_mode.binding.schedule_occurrence_identity(),
        item.schedule_occurrence_identity
    );
    assert!(next_fact(
        slot(),
        Input::PositionQuotes,
        Observation::Unavailable(Reason::AcquisitionUnavailable),
        start() + chrono::Duration::seconds(30),
        |key| crate::durable_delivery_runtime::auction_input_occurrence_source_from(
            &reopened,
            slot().date,
            key
        ),
    )
    .unwrap()
    .is_none());
    let owner = reopened
        .inspect_exact_occurrence_owner(
            &slot().date.to_string(),
            PushKind::DataMode,
            DeliverySubKind::None,
            "GLOBAL",
            &item.schedule_occurrence_identity,
        )
        .unwrap()
        .unwrap();
    assert_eq!(owner.state, DecisionState::UncertainManualReview);
    assert_eq!(owner.envelope, item);
    assert_eq!(owner.envelope.rendered_content, report.text.as_bytes());
    let prior = reopened
        .inspect_exact_occurrence_owner(
            &slot().date.to_string(),
            PushKind::DataMode,
            DeliverySubKind::None,
            "GLOBAL",
            &ordinary.schedule_occurrence_identity,
        )
        .unwrap()
        .unwrap();
    assert_eq!(prior.state, DecisionState::UncertainManualReview);
    assert_eq!(prior.envelope, ordinary);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(ordinary_calls.load(Ordering::SeqCst), 1);

    let other = next_fact(
        slot(),
        Input::VolumeCandidates,
        Observation::Unavailable(Reason::VolumeRatioUnavailable),
        start() + chrono::Duration::seconds(30),
        |key| {
            crate::durable_delivery_runtime::auction_input_occurrence_source_from(
                &reopened,
                slot().date,
                key,
            )
        },
    )
    .unwrap()
    .unwrap();
    let other_report = PreparedAuctionInputAlert::prepare(slot(), other, DataMode::Unsafe).unwrap();
    let other_item = envelope(&other_report.binding, &other_report.text);
    assert_ne!(
        other_item.schedule_occurrence_identity,
        item.schedule_occurrence_identity
    );
    assert_eq!(
        reopened
            .prepare(&other_item, 1, start() + chrono::Duration::seconds(30))
            .unwrap()
            .state,
        DecisionState::Reserved
    );
    let recovered = next_fact(
        slot(),
        Input::PositionQuotes,
        Observation::Fresh {
            observed_at: start() + chrono::Duration::seconds(60),
            batch_sha256: hash(b"TEST_CODE_FRESH_BATCH_AFTER_REOPEN"),
        },
        start() + chrono::Duration::seconds(60),
        |key| {
            crate::durable_delivery_runtime::auction_input_occurrence_source_from(
                &reopened,
                slot().date,
                key,
            )
        },
    )
    .unwrap()
    .unwrap();
    let recovery_report =
        PreparedAuctionInputAlert::prepare(slot(), recovered, DataMode::Unsafe).unwrap();
    let recovery_item = envelope(&recovery_report.binding, &recovery_report.text);
    assert_ne!(
        recovery_item.schedule_occurrence_identity,
        item.schedule_occurrence_identity
    );
    assert_eq!(
        reopened
            .prepare(&recovery_item, 1, start() + chrono::Duration::seconds(60))
            .unwrap()
            .state,
        DecisionState::Reserved
    );
    reopened
        .reconcile_all_pending(&append, start() + chrono::Duration::seconds(60))
        .unwrap();
    validate_authoritative_request(
        &namespace,
        &request(&recovery_item),
        start() + chrono::Duration::seconds(60),
    )
    .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "preparing independent facts does not resend the Unknown owner"
    );
    assert_eq!(
        store.catalog(),
        catalog,
        "ordinary Schema9 remains byte-for-byte unchanged"
    );
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM delivery_decisions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        4
    );
}
