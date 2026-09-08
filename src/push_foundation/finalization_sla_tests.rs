use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::business_finalizer::{
    commit_accepted_finalization, prepare_accepted_finalization, AcceptedPreparationOutcome,
    AcceptedPreparationRequest, FinalizerFence,
};
use super::dedicated_transport::DedicatedConformanceRoute;
use super::finalization_sla::*;
use super::generic_transport::{
    build_foundation_envelope, GenericTerminalAuthorityAdapter, GenericTransportRoute,
};
use super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialDecisionKind, InitialIntentDraft,
    InitialIntentIdentity, IntentSnapshot, IntentState, IntentTransitionCommand, LeaseAction,
    LeaseOwnerId, TerminalTemplateBinding, TransitionActor,
};
use crate::durable_delivery::{
    AuthoritativeDeliveryRequest, AuthoritativeSink, AuthoritativeSinkPort,
    AuthoritativeSinkResult, CoordinatorConfig, DeliveryEnvelope, DeliverySubKind,
    DurableDeliveryCoordinator, ImmutableAppendPort, ManualDisposition, ManualResolutionCommand,
    PushKind, TypedReceipt, TypedRejection, TypedUncertainty,
};
use crate::event::envelope::{
    news_flash_evidence_sha256, NewsFlashAuditSource, NewsFlashRemoteReceipt,
    NewsFlashTransactionStage,
};
use crate::event::{AuditDispatcher, EventEnvelope, NewsFlashWindow, PushDeliveryEvent};
use crate::monitor::push_job::{
    raw_digest, w09_completion_policy_fixture, AudienceId, AuthorityClass, BusinessDate, ChannelId,
    CompletionOwnerId, CompletionPolicy, IntentId, Namespace, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, ReasonCode, RunId, SourceContractId, SubjectId,
    TemplateId, TemplateVersion, UnitId, UtcMicros,
};
use chrono::{DateTime, Utc};

const ACCEPTED: i64 = 1_787_020_203_123_456;
const CHANNEL: &str = "TEST_CODE_W19_CHANNEL";
fn micros(value: i64) -> UtcMicros {
    UtcMicros::try_new(value).unwrap()
}
fn utc(value: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(value).unwrap()
}

#[derive(Default)]
struct Append {
    records: Mutex<BTreeMap<String, (String, Vec<u8>, String)>>,
    calls: AtomicUsize,
}
impl ImmutableAppendPort for Append {
    fn append_exact(
        &self,
        kind: &str,
        identity: &str,
        bytes: &[u8],
        sha: &str,
    ) -> crate::durable_delivery::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let proposed = (kind.to_owned(), bytes.to_vec(), sha.to_owned());
        let mut records = self.records.lock().unwrap();
        if let Some(existing) = records.get(identity) {
            assert_eq!(existing, &proposed);
        } else {
            records.insert(identity.to_owned(), proposed);
        }
        Ok(format!("test-immutable://{kind}/{identity}"))
    }
}
struct Sink {
    result: AuthoritativeSinkResult,
    calls: AtomicUsize,
}
impl AuthoritativeSinkPort for Sink {
    fn sink_identity(&self) -> &str {
        CHANNEL
    }
    fn deliver(&self, _: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }
}
fn accepted_result(at: i64) -> AuthoritativeSinkResult {
    AuthoritativeSinkResult::Accepted(TypedReceipt {
        channel: CHANNEL.to_owned(),
        provider: "TEST_CODE_W19_PROVIDER".to_owned(),
        message_id: "SECRET_TEST_MESSAGE_ID".to_owned(),
        platform_message_id: None,
        accepted_at: utc(at),
        latency_ms: Some(7),
    })
}

struct Case {
    store: BusinessIntentStore,
    durable: Option<DurableDeliveryCoordinator>,
    audit: Option<AuditDispatcher>,
    root: tempfile::TempDir,
    business_path: PathBuf,
    durable_path: PathBuf,
    code: String,
    namespace: Namespace,
    unit: UnitId,
    intent: IntentId,
    template: TerminalTemplateBinding,
    policy: CompletionPolicy,
    channel: ChannelId,
    dedicated: Option<DedicatedConformanceRoute>,
    class: AuthorityClass,
    legacy_decision: Option<String>,
    snapshot: IntentSnapshot,
    append: Append,
    sink: Arc<Sink>,
}
impl Case {
    fn new(class: AuthorityClass, result: AuthoritativeSinkResult, seal: bool) -> Self {
        Self::variant(class, result, seal, InitialDecisionKind::Ready, None)
    }
    fn variant(
        class: AuthorityClass,
        result: AuthoritativeSinkResult,
        seal: bool,
        kind: InitialDecisionKind,
        family_override: Option<&str>,
    ) -> Self {
        std::fs::create_dir_all("data/test").unwrap();
        let root = tempfile::Builder::new()
            .prefix("TEST_CODE_W19_SLA_")
            .tempdir_in("data/test")
            .unwrap();
        let code = root
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let business_path = root.path().join("business.sqlite3");
        let durable_path = root.path().join("durable_delivery.sqlite3");
        FoundationSchemaMigration::bundled()
            .unwrap()
            .apply_to(&business_path)
            .unwrap();
        let namespace = Namespace::test(RunId::try_new(code.clone()).unwrap());
        let (unit_name, template_name, owner, family, key) = match class {
            AuthorityClass::GenericCounted => (
                "MU-auction",
                "auction-card",
                "owner-auction",
                "auction-session",
                "main",
            ),
            AuthorityClass::P01Dedicated => (
                "MU-p01",
                "preopen_news_hot_v1",
                "p01-business-date-once",
                "p01-business-date",
                "2026-08-18",
            ),
            AuthorityClass::N02Dedicated => (
                "MU-news-flash-aggregate",
                "news_flash_aggregated_v1",
                "news-flash-accepted-window",
                "news-flash-window",
                "09:30",
            ),
        };
        let unit = UnitId::try_new(unit_name.to_owned()).unwrap();
        let template = TerminalTemplateBinding::new(
            TemplateId::try_new(template_name.to_owned()).unwrap(),
            TemplateVersion::try_new(template_name.to_owned()).unwrap(),
        );
        let identity = InitialIntentIdentity::new(
            namespace.clone(),
            unit.clone(),
            OccurrenceIdentityMaterial::new(
                BusinessDate::parse("2026-08-18").unwrap(),
                OccurrenceFamily::try_new(family_override.unwrap_or(family).to_owned()).unwrap(),
                OccurrenceKey::try_new(key.to_owned()).unwrap(),
            ),
            CompletionOwnerId::try_new(owner.to_owned()).unwrap(),
            SourceContractId::try_new("w19-source".to_owned()).unwrap(),
            SubjectId::Global,
            AudienceId::try_new("test-owner".to_owned()).unwrap(),
        );
        let draft = match kind {
            InitialDecisionKind::Ready => InitialIntentDraft::ready_for_recovery_test(
                identity,
                b"SECRET_TEST_PREPARED".to_vec(),
                b"SECRET_TEST_RENDERED".to_vec(),
                template.sha256().clone(),
                raw_digest(b"TEST_CODE_W19_CONTRACT"),
                micros(ACCEPTED - 10_000_000),
            )
            .unwrap(),
            InitialDecisionKind::NoData => InitialIntentDraft::no_data(
                identity,
                raw_digest(b"evidence"),
                template.sha256().clone(),
                raw_digest(b"TEST_CODE_W19_CONTRACT"),
                micros(ACCEPTED - 10_000_000),
            ),
            InitialDecisionKind::Disabled => InitialIntentDraft::disabled(
                identity,
                raw_digest(b"evidence"),
                template.sha256().clone(),
                raw_digest(b"TEST_CODE_W19_CONTRACT"),
                micros(ACCEPTED - 10_000_000),
            ),
        };
        let intent = draft.intent_id().clone();
        let mut store = BusinessIntentStore::open(&business_path).unwrap();
        let snapshot = store.record_initial(&draft).unwrap().snapshot().clone();
        let policy = w09_completion_policy_fixture(unit_name, owner, vec![class]);
        let channel = ChannelId::try_new(CHANNEL.to_owned()).unwrap();
        let dedicated = (class != AuthorityClass::GenericCounted).then(|| {
            DedicatedConformanceRoute::try_new(template.clone(), channel.clone()).unwrap()
        });
        let sink = Arc::new(Sink {
            result,
            calls: AtomicUsize::new(0),
        });
        let mut case = Self {
            store,
            durable: None,
            audit: None,
            root,
            business_path,
            durable_path,
            code,
            namespace,
            unit,
            intent,
            template,
            policy,
            channel,
            dedicated,
            class,
            legacy_decision: None,
            snapshot,
            append: Append::default(),
            sink,
        };
        if kind != InitialDecisionKind::Ready {
            case.durable = Some(case.open_durable());
            return case;
        }
        if class == AuthorityClass::N02Dedicated {
            case.audit = Some(AuditDispatcher::for_test_code(&case.code).unwrap());
            case.write_n02(seal);
        } else {
            case.durable = Some(case.open_durable());
            let attested = case.snapshot.attested_ready_binding().unwrap();
            let envelope = if class == AuthorityClass::GenericCounted {
                let route = GenericTransportRoute::try_new(
                    PushKind::HoldingEvent,
                    DeliverySubKind::None,
                    "GLOBAL".to_owned(),
                    case.channel.clone(),
                    case.template.clone(),
                )
                .unwrap();
                build_foundation_envelope(&case.snapshot, &attested, &route).unwrap()
            } else {
                DeliveryEnvelope::new(
                    "2026-08-18",
                    PushKind::PreopenNewsHot,
                    DeliverySubKind::None,
                    "GLOBAL",
                    "p01:2026-08-18",
                    attested.source_evidence_fingerprint.as_str(),
                    br#"{"render_mode":"Scheduled","schema_version":"P01_SOURCE_BINDING_V1"}"#
                        .to_vec(),
                    "TEST_CODE_W19_SUBJECT",
                    case.snapshot.rendered_bytes().unwrap().to_vec(),
                    false,
                    None,
                )
                .unwrap()
            };
            case.legacy_decision = Some(envelope.decision_identity.clone());
            let coordinator = case.durable.as_ref().unwrap();
            coordinator
                .prepare(&envelope, 1, utc(ACCEPTED - 2_000_000))
                .unwrap();
            coordinator
                .reconcile_all_pending(&case.append, utc(ACCEPTED - 1_000_000))
                .unwrap();
            let sinks: Vec<AuthoritativeSink> = vec![case.sink.clone()];
            coordinator
                .resume_deliverable(
                    &envelope.decision_identity,
                    &sinks,
                    utc(ACCEPTED + 1_000_000),
                )
                .unwrap();
            if seal {
                coordinator
                    .reconcile_all_pending(&case.append, utc(ACCEPTED + 2_000_000))
                    .unwrap();
            }
        }
        case
    }
    fn open_durable(&self) -> DurableDeliveryCoordinator {
        DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            &self.durable_path,
            &self.code,
            format!("owner-{}", self.code),
        ))
        .unwrap()
    }
    fn restart(&mut self) {
        if self.durable.take().is_some() {
            self.durable = Some(self.open_durable());
        }
        if self.audit.take().is_some() {
            self.audit = Some(AuditDispatcher::for_test_code(&self.code).unwrap());
        }
        self.store = BusinessIntentStore::open(&self.business_path).unwrap();
    }
    fn query(
        &self,
        observed: i64,
        cycle: Duration,
    ) -> Result<FinalizationSlaReport, FinalizationSlaError> {
        let route = match self.class {
            AuthorityClass::GenericCounted => FinalizationSlaRoute::Generic {
                source: self.durable.as_ref().unwrap(),
                required_channel: &self.channel,
            },
            AuthorityClass::P01Dedicated => FinalizationSlaRoute::P01 {
                source: self.durable.as_ref().unwrap(),
                route: self.dedicated.as_ref().unwrap(),
            },
            AuthorityClass::N02Dedicated => FinalizationSlaRoute::N02 {
                source: self.audit.as_ref().unwrap(),
                route: self.dedicated.as_ref().unwrap(),
                window: NewsFlashWindow::H0930,
            },
        };
        inspect_finalization_sla(
            &self.store,
            FinalizationSlaQuery {
                namespace: &self.namespace,
                unit: &self.unit,
                intent: &self.intent,
                template: &self.template,
                policy: &self.policy,
                route,
                observed_at: micros(observed),
                reconcile_cycle: cycle,
            },
        )
    }
    fn rows(&self) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        let mut result = database_rows(&self.business_path);
        if self.durable.is_some() {
            result.extend(database_rows(&self.durable_path));
        }
        result
    }
    fn audit_bytes(&self) -> Vec<Vec<u8>> {
        let mut paths: Vec<_> = std::fs::read_dir(self.root.path().join("event_audit"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .collect();
        paths.sort();
        paths
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect()
    }
    fn write_n02(&self, seal: bool) {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 18).unwrap();
        let sources = vec![NewsFlashAuditSource {
            event_id: "TEST_CODE_W19_EVENT".to_owned(),
            provider: "TEST_CODE_W19_PROVIDER".to_owned(),
            source: "TEST_CODE_W19_SOURCE".to_owned(),
            published_at: utc(ACCEPTED - 9_000_000).fixed_offset(),
            observed_at: utc(ACCEPTED - 8_000_000).fixed_offset(),
            batch_id: "TEST_CODE_W19_BATCH".to_owned(),
        }];
        let evidence = news_flash_evidence_sha256(&sources);
        let rendered_sha = self.snapshot.rendered_sha256().unwrap().as_str().to_owned();
        let rendered_len = self.snapshot.rendered_bytes().unwrap().len();
        let attempt_at = utc(ACCEPTED - 1_000_000).fixed_offset();
        let attempt_event = PushDeliveryEvent::new_news_flash_attempt(
            "news_flash_aggregated_v1".to_owned(),
            "window:09:30".to_owned(),
            CHANNEL.to_owned(),
            rendered_len,
            date,
            "a".repeat(64),
            sources.clone(),
            evidence.clone(),
            rendered_sha.clone(),
            1,
            attempt_at,
        );
        let attempt = EventEnvelope::from_event(
            &attempt_event,
            attempt_event.news_flash_join_sha256.clone().unwrap(),
            "TEST_CODE_W19_ATTEMPT".to_owned(),
            attempt_at.with_timezone(&chrono::Local),
        )
        .unwrap();
        let audit = self.audit.as_ref().unwrap();
        audit.append_exact_news_flash_authority(&attempt).unwrap();
        if !seal {
            return;
        }
        let receipt = NewsFlashRemoteReceipt {
            channel: CHANNEL.to_owned(),
            provider: "TEST_CODE_W19_PROVIDER".to_owned(),
            message_id: "SECRET_TEST_MESSAGE_ID".to_owned(),
            platform_message_id: "SECRET_TEST_PLATFORM_ID".to_owned(),
            accepted_at: utc(ACCEPTED).fixed_offset(),
            latency_ms: 7,
        };
        let terminal_at = utc(ACCEPTED + 2_000_000).fixed_offset();
        let terminal_event = PushDeliveryEvent::new_news_flash_terminal(
            NewsFlashTransactionStage::Accepted,
            "news_flash_aggregated_v1".to_owned(),
            "window:09:30".to_owned(),
            CHANNEL.to_owned(),
            rendered_len,
            3,
            date,
            "a".repeat(64),
            sources,
            evidence,
            rendered_sha,
            1,
            attempt_at,
            attempt_event
                .news_flash_sink_attempt_identity
                .clone()
                .unwrap(),
            attempt_event
                .news_flash_sink_attempt_sha256
                .clone()
                .unwrap(),
            attempt.id,
            Some(receipt),
            terminal_at,
            None,
            None,
        );
        let terminal = EventEnvelope::from_event(
            &terminal_event,
            terminal_event.news_flash_join_sha256.clone().unwrap(),
            "TEST_CODE_W19_TERMINAL".to_owned(),
            terminal_at.with_timezone(&chrono::Local),
        )
        .unwrap();
        audit.append_exact_news_flash_authority(&terminal).unwrap();
    }
    fn claim(&mut self) -> IntentSnapshot {
        self.store
            .apply_nonterminal_transition(
                &IntentTransitionCommand::try_new(
                    self.intent.clone(),
                    IntentState::PendingDispatch,
                    IntentState::AwaitingAuthority,
                    0,
                    TransitionActor::try_new("w19-finalizer".to_owned()).unwrap(),
                    ReasonCode::IntentDispatchClaimed,
                    micros(ACCEPTED - 5_000_000),
                    LeaseAction::Acquire {
                        owner: LeaseOwnerId::try_new("w19-finalizer".to_owned()).unwrap(),
                        until: micros(ACCEPTED + 1_000_000_000),
                    },
                )
                .unwrap(),
            )
            .unwrap();
        self.store.inspect(&self.intent).unwrap().unwrap()
    }
    fn complete(&mut self, at: i64) {
        let claimed = self.claim();
        let authority =
            GenericTerminalAuthorityAdapter::try_new(self.durable.as_ref().unwrap()).unwrap();
        let request = AcceptedPreparationRequest::new(
            self.intent.clone(),
            claimed.version(),
            TransitionActor::try_new("w19-finalizer".to_owned()).unwrap(),
            FinalizerFence::new(
                LeaseOwnerId::try_new("w19-finalizer".to_owned()).unwrap(),
                claimed.lease_generation(),
                claimed.lease_until().unwrap(),
            ),
            micros(at - 2),
            micros(at - 1),
        )
        .unwrap();
        let pending = match prepare_accepted_finalization(
            &mut self.store,
            request,
            &self.template,
            &self.policy,
            &authority,
        )
        .unwrap()
        {
            AcceptedPreparationOutcome::Pending(pending) => pending,
            other => panic!("unexpected {other:?}"),
        };
        commit_accepted_finalization(
            &mut self.store,
            pending,
            &self.template,
            &self.policy,
            &authority,
            micros(at),
            micros(at),
        )
        .unwrap();
    }
}

fn database_rows(path: &Path) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let names = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut statement = connection
                .prepare(&format!(
                    "SELECT * FROM \"{}\" ORDER BY rowid",
                    name.replace('"', "\"\"")
                ))
                .unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|index| row.get(index))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

#[test]
fn real_three_authorities_preserve_original_receipt_across_restart_without_writes() {
    for class in [
        AuthorityClass::GenericCounted,
        AuthorityClass::P01Dedicated,
        AuthorityClass::N02Dedicated,
    ] {
        let mut case = Case::new(class, accepted_result(ACCEPTED), true);
        let before = case.rows();
        let audit_before = case.audit.as_ref().map(|_| case.audit_bytes());
        let append_calls = case.append.calls.load(Ordering::SeqCst);
        let sink_calls = case.sink.calls.load(Ordering::SeqCst);
        let first = case
            .query(ACCEPTED + 123_000_001, Duration::from_secs(30))
            .unwrap();
        assert_eq!(first.accepted_at(), Some(micros(ACCEPTED)));
        assert_eq!(first.elapsed(), Some(Duration::from_micros(123_000_001)));
        assert_eq!(first.status(), FinalizationSlaStatus::AwaitingFinalization);
        assert_eq!(first.business_state(), IntentState::PendingDispatch);
        assert_eq!(first.authority(), class);
        assert_eq!(first.namespace(), format!("Test:{}", case.code));
        assert_eq!(first.unit(), case.unit.as_str());
        assert_eq!(first.intent(), case.intent.as_str());
        assert_eq!(first.decision(), case.snapshot.decision_id());
        assert_eq!(first.version(), 0);
        assert_eq!(first.head(), None);
        assert!(first.terminal_ref_sha256().is_some());
        assert!(first.evidence_sha256().is_some());
        assert!(first.binding_sha256().is_some());
        assert_eq!(first.observed_at(), micros(ACCEPTED + 123_000_001));
        assert_eq!(first.completed_at(), None);
        assert!(first.disposition().is_some());
        let debug = format!("{first:?}");
        for secret in [
            "SECRET_TEST",
            "TEST_CODE_W19_PROVIDER",
            case.root.path().to_str().unwrap(),
        ] {
            assert!(!debug.contains(secret));
        }
        assert_eq!(before, case.rows());
        assert_eq!(
            audit_before,
            case.audit.as_ref().map(|_| case.audit_bytes())
        );
        case.restart();
        let second = case
            .query(ACCEPTED + 123_000_001, Duration::from_secs(30))
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(before, case.rows());
        assert_eq!(
            audit_before,
            case.audit.as_ref().map(|_| case.audit_bytes())
        );
        assert_eq!(case.append.calls.load(Ordering::SeqCst), append_calls);
        assert_eq!(case.sink.calls.load(Ordering::SeqCst), sink_calls);
    }
}

#[test]
fn persisted_pending_and_completed_microsecond_boundaries_are_distinct() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    for (elapsed, exceeded, hard) in [
        (59_999_999, false, false),
        (60_000_000, false, false),
        (60_000_001, true, false),
        (299_999_999, true, false),
        (300_000_000, true, true),
        (300_000_001, true, true),
    ] {
        let report = case
            .query(ACCEPTED + elapsed, Duration::from_secs(30))
            .unwrap();
        assert_eq!(report.two_cycle_target(), Duration::from_secs(60));
        assert_eq!(report.target_exceeded(), Some(exceeded));
        assert_eq!(report.hard_limit_reached(), Some(hard));
        assert_eq!(report.requires_block(), hard);
        assert_eq!(
            report.reason(),
            (exceeded || hard).then_some(ReasonCode::FinalizerDeadlineExceeded)
        );
    }
    case.complete(ACCEPTED + 300_000_000);
    case.restart();
    for observed in [ACCEPTED + 300_000_000, ACCEPTED + 900_000_000] {
        let report = case.query(observed, Duration::from_secs(30)).unwrap();
        assert_eq!(report.status(), FinalizationSlaStatus::Completed);
        assert_eq!(report.elapsed(), Some(Duration::from_secs(300)));
        assert_eq!(report.completed_at(), Some(micros(ACCEPTED + 300_000_000)));
        assert_eq!(report.hard_limit_reached(), Some(true));
        assert!(!report.requires_block());
    }
}

#[test]
fn pending_seal_never_yields_an_accepted_clock() {
    for class in [
        AuthorityClass::GenericCounted,
        AuthorityClass::P01Dedicated,
        AuthorityClass::N02Dedicated,
    ] {
        let case = Case::new(class, accepted_result(ACCEPTED), false);
        let report = case
            .query(ACCEPTED + 400_000_000, Duration::from_secs(30))
            .unwrap();
        assert_eq!(report.status(), FinalizationSlaStatus::PendingSeal);
        assert_eq!(report.accepted_at(), None);
        assert_eq!(report.elapsed(), None);
        assert_eq!(report.hard_limit_reached(), None);
    }
}

#[test]
fn rejected_and_uncertain_real_sources_are_not_transport_accepted_samples() {
    for (result, status) in [
        (
            AuthoritativeSinkResult::Rejected(TypedRejection {
                reason_code: "TEST_CODE_W19_REJECTED".to_owned(),
                evidence: b"TEST_CODE_W19_REJECTION".to_vec(),
                retry_authorized: false,
                observed_at: utc(ACCEPTED),
            }),
            FinalizationSlaStatus::Rejected,
        ),
        (
            AuthoritativeSinkResult::Uncertain(TypedUncertainty {
                reason_code: "TEST_CODE_W19_UNCERTAIN".to_owned(),
                evidence: b"TEST_CODE_W19_UNCERTAINTY".to_vec(),
                observed_at: utc(ACCEPTED),
            }),
            FinalizationSlaStatus::Uncertain,
        ),
    ] {
        let case = Case::new(AuthorityClass::GenericCounted, result, true);
        let report = case
            .query(ACCEPTED + 400_000_000, Duration::from_secs(30))
            .unwrap();
        assert_eq!(report.status(), status);
        assert_eq!(report.accepted_at(), None);
        assert_eq!(report.elapsed(), None);
    }
}

#[test]
fn invalid_cycles_and_backward_clocks_fail_closed() {
    for cycle in [
        Duration::ZERO,
        Duration::from_nanos(1),
        Duration::MAX,
        Duration::from_micros(i64::MAX as u64),
    ] {
        assert_eq!(
            checked_target(cycle),
            Err(FinalizationSlaError::InvalidCycle)
        );
    }
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    assert_eq!(
        case.query(ACCEPTED - 1, Duration::from_secs(30))
            .unwrap()
            .status(),
        FinalizationSlaStatus::ClockUncertain
    );
    case.complete(ACCEPTED + 10_000_000);
    let report = case
        .query(ACCEPTED + 5_000_000, Duration::from_secs(30))
        .unwrap();
    assert_eq!(report.status(), FinalizationSlaStatus::ClockUncertain);
    assert_eq!(report.elapsed(), None);
    let mut before_acceptance = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED + 20_000_000),
        true,
    );
    before_acceptance.complete(ACCEPTED + 10_000_000);
    assert_eq!(
        before_acceptance
            .query(ACCEPTED + 30_000_000, Duration::from_secs(30))
            .unwrap()
            .status(),
        FinalizationSlaStatus::ClockUncertain
    );
    assert_eq!(
        FinalizationSlaError::SourceInvalid.reason(),
        ReasonCode::FinalizerTerminalRefInvalid
    );
}

#[test]
fn explicit_n02_wrong_window_is_rejected_before_source_lookup() {
    let case = Case::new(
        AuthorityClass::N02Dedicated,
        accepted_result(ACCEPTED),
        true,
    );
    let result = inspect_finalization_sla(
        &case.store,
        FinalizationSlaQuery {
            namespace: &case.namespace,
            unit: &case.unit,
            intent: &case.intent,
            template: &case.template,
            policy: &case.policy,
            route: FinalizationSlaRoute::N02 {
                source: case.audit.as_ref().unwrap(),
                route: case.dedicated.as_ref().unwrap(),
                window: NewsFlashWindow::H1130,
            },
            observed_at: micros(ACCEPTED + 10_000_000),
            reconcile_cycle: Duration::from_secs(30),
        },
    );
    assert_eq!(result, Err(FinalizationSlaError::RouteMismatch));
}

#[test]
fn awaiting_authority_and_resolution_keep_accepted_age() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let claimed = case.claim();
    assert_eq!(
        case.query(ACCEPTED + 400_000_000, Duration::from_secs(30))
            .unwrap()
            .status(),
        FinalizationSlaStatus::AwaitingFinalization
    );
    case.store
        .apply_nonterminal_transition(
            &IntentTransitionCommand::try_new(
                case.intent.clone(),
                IntentState::AwaitingAuthority,
                IntentState::ResolutionRequired,
                claimed.version(),
                TransitionActor::try_new("w19-reconcile".to_owned()).unwrap(),
                ReasonCode::TransportUncertain,
                micros(ACCEPTED + 1),
                LeaseAction::Preserve,
            )
            .unwrap(),
        )
        .unwrap();
    let report = case
        .query(ACCEPTED + 400_000_000, Duration::from_secs(30))
        .unwrap();
    assert_eq!(report.status(), FinalizationSlaStatus::ResolutionRequired);
    assert_eq!(report.elapsed(), Some(Duration::from_secs(400)));
    assert!(report.requires_block());
}

#[test]
fn n02_unknown_occurrence_convention_is_unsupported_not_corrupt() {
    let case = Case::variant(
        AuthorityClass::N02Dedicated,
        accepted_result(ACCEPTED),
        true,
        InitialDecisionKind::Ready,
        Some("another-valid-occurrence-family"),
    );
    assert_eq!(
        case.query(ACCEPTED + 10_000_000, Duration::from_secs(30)),
        Err(FinalizationSlaError::UnsupportedOccurrenceRoute)
    );
}

#[test]
fn lawful_nonready_initials_and_ready_to_nonready_contradictions_are_distinct() {
    for (kind, state, reason) in [
        (
            InitialDecisionKind::NoData,
            IntentState::NoData,
            ReasonCode::IntentNoData,
        ),
        (
            InitialDecisionKind::Disabled,
            IntentState::Disabled,
            ReasonCode::PolicyDisabled,
        ),
    ] {
        let case = Case::variant(
            AuthorityClass::GenericCounted,
            accepted_result(ACCEPTED),
            true,
            kind,
            None,
        );
        assert_eq!(
            case.query(ACCEPTED + 400_000_000, Duration::from_secs(30))
                .unwrap()
                .status(),
            FinalizationSlaStatus::NotApplicable
        );
        let mut ready = Case::new(
            AuthorityClass::GenericCounted,
            accepted_result(ACCEPTED),
            true,
        );
        ready
            .store
            .apply_nonterminal_transition(
                &IntentTransitionCommand::try_new(
                    ready.intent.clone(),
                    IntentState::PendingDispatch,
                    state,
                    0,
                    TransitionActor::try_new("w19-source-refresh".to_owned()).unwrap(),
                    reason,
                    micros(ACCEPTED + 1),
                    LeaseAction::Preserve,
                )
                .unwrap(),
            )
            .unwrap();
        let report = ready
            .query(ACCEPTED + 400_000_000, Duration::from_secs(30))
            .unwrap();
        assert_eq!(report.status(), FinalizationSlaStatus::Conflict);
        assert_eq!(report.accepted_at(), Some(micros(ACCEPTED)));
        assert!(report.requires_block());
    }
}

#[test]
fn current_resolution_is_not_hidden_by_a_completed_history() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    case.complete(ACCEPTED + 10_000_000);
    let completed = case.store.inspect(&case.intent).unwrap().unwrap();
    case.store
        .apply_nonterminal_transition(
            &IntentTransitionCommand::try_new(
                case.intent.clone(),
                IntentState::Completed,
                IntentState::ResolutionRequired,
                completed.version(),
                TransitionActor::try_new("w19-conflict".to_owned()).unwrap(),
                ReasonCode::FinalizerCasConflict,
                micros(ACCEPTED + 20_000_000),
                LeaseAction::Preserve,
            )
            .unwrap(),
        )
        .unwrap();
    let report = case
        .query(ACCEPTED + 400_000_000, Duration::from_secs(30))
        .unwrap();
    assert_eq!(report.status(), FinalizationSlaStatus::ResolutionRequired);
    assert_eq!(report.completed_at(), Some(micros(ACCEPTED + 10_000_000)));
    assert_eq!(report.elapsed(), Some(Duration::from_secs(10)));
    assert!(report.requires_block());
}

#[test]
fn missing_source_and_wrong_namespace_unit_channel_template_fail_closed() {
    let case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let empty = Case::variant(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
        InitialDecisionKind::NoData,
        None,
    );
    let query = |namespace, unit, template, source, channel| {
        inspect_finalization_sla(
            &case.store,
            FinalizationSlaQuery {
                namespace,
                unit,
                intent: &case.intent,
                template,
                policy: &case.policy,
                route: FinalizationSlaRoute::Generic {
                    source,
                    required_channel: channel,
                },
                observed_at: micros(ACCEPTED + 400_000_000),
                reconcile_cycle: Duration::from_secs(30),
            },
        )
    };
    assert_eq!(
        query(
            &case.namespace,
            &case.unit,
            &case.template,
            empty.durable.as_ref().unwrap(),
            &case.channel
        )
        .unwrap()
        .status(),
        FinalizationSlaStatus::Missing
    );
    assert_eq!(
        query(
            &empty.namespace,
            &case.unit,
            &case.template,
            case.durable.as_ref().unwrap(),
            &case.channel
        ),
        Err(FinalizationSlaError::RouteMismatch)
    );
    let wrong_unit = UnitId::try_new("MU-wrong".to_owned()).unwrap();
    assert_eq!(
        query(
            &case.namespace,
            &wrong_unit,
            &case.template,
            case.durable.as_ref().unwrap(),
            &case.channel
        ),
        Err(FinalizationSlaError::RouteMismatch)
    );
    let wrong_template = TerminalTemplateBinding::new(
        TemplateId::try_new("wrong".to_owned()).unwrap(),
        TemplateVersion::try_new("wrong".to_owned()).unwrap(),
    );
    assert_eq!(
        query(
            &case.namespace,
            &case.unit,
            &wrong_template,
            case.durable.as_ref().unwrap(),
            &case.channel
        ),
        Err(FinalizationSlaError::RouteMismatch)
    );
    assert_eq!(
        query(
            &case.namespace,
            &case.unit,
            &case.template,
            case.durable.as_ref().unwrap(),
            &ChannelId::try_new("wrong".to_owned()).unwrap()
        ),
        Err(FinalizationSlaError::SourceInvalid)
    );
}

#[test]
fn real_manual_authority_results_never_supply_transport_accepted_latency() {
    for accepted in [true, false] {
        let uncertainty = AuthoritativeSinkResult::Uncertain(TypedUncertainty {
            reason_code: "TEST_CODE_W19_UNCERTAIN".to_owned(),
            evidence: b"TEST_CODE_W19_UNCERTAINTY".to_vec(),
            observed_at: utc(ACCEPTED),
        });
        let mut case = Case::new(AuthorityClass::GenericCounted, uncertainty, true);
        let coordinator = case.durable.as_ref().unwrap();
        let legacy = case.legacy_decision.clone().unwrap();
        coordinator
            .resolve_uncertain(
                &ManualResolutionCommand {
                    decision_identity: legacy,
                    disposition: if accepted {
                        ManualDisposition::Accepted { receipt: None }
                    } else {
                        ManualDisposition::Rejected
                    },
                    operator_identity: "TEST_CODE_W19_OPERATOR_0123456789".to_owned(),
                    reason: "TEST_CODE_W19_EXPLICIT_RESOLUTION".to_owned(),
                    external_evidence: b"TEST_CODE_W19_MANUAL_EVIDENCE".to_vec(),
                    resolved_at: utc(ACCEPTED + 10_000_000),
                },
                &case.append,
            )
            .unwrap();
        coordinator
            .reconcile_all_pending(&case.append, utc(ACCEPTED + 11_000_000))
            .unwrap();
        if accepted {
            case.complete(ACCEPTED + 20_000_000);
        }
        case.restart();
        let report = case
            .query(ACCEPTED + 400_000_000, Duration::from_secs(30))
            .unwrap();
        assert_eq!(
            report.status(),
            if accepted {
                FinalizationSlaStatus::ManualAccepted
            } else {
                FinalizationSlaStatus::ManualNotDelivered
            }
        );
        assert_eq!(report.accepted_at(), None);
        assert_eq!(report.elapsed(), None);
        assert_eq!(report.hard_limit_reached(), None);
    }
}

#[test]
fn actual_completed_chain_must_match_current_exact_terminal_reference() {
    use super::terminal_authority::{
        terminal_binding_sha256, AuthorityQuery, TerminalAuthorityPort,
    };
    use super::terminal_authority_tests::FakeAuthority;
    use crate::monitor::push_job::{TerminalDisposition, TerminalRefId};
    for mismatch in ["ref", "binding", "disposition"] {
        let mut case = Case::new(
            AuthorityClass::GenericCounted,
            accepted_result(ACCEPTED),
            true,
        );
        let claimed = case.claim();
        let adapter =
            GenericTerminalAuthorityAdapter::try_new(case.durable.as_ref().unwrap()).unwrap();
        let decision = case.snapshot.attested_ready_binding().unwrap().decision_id;
        let mut record = match adapter.requery_terminal(&decision).unwrap() {
            AuthorityQuery::Terminal(record) => *record,
            other => panic!("unexpected {other:?}"),
        };
        match mismatch {
            "ref" => {
                record.ref_id =
                    TerminalRefId::try_new("TEST_CODE_DIFFERENT_TERMINAL".to_owned()).unwrap()
            }
            "binding" => {
                record.evidence_bytes.push(b' ');
                record.evidence_sha256 = raw_digest(&record.evidence_bytes);
            }
            "disposition" => {
                record.terminal_disposition = TerminalDisposition::ManualConfirmedAccepted
            }
            _ => unreachable!(),
        }
        record.binding_sha256 = terminal_binding_sha256(&record);
        let descriptor = adapter.descriptor().clone();
        let mut conflicting = FakeAuthority::terminal(record);
        conflicting.descriptor = descriptor;
        let request = AcceptedPreparationRequest::new(
            case.intent.clone(),
            claimed.version(),
            TransitionActor::try_new("w19-finalizer".to_owned()).unwrap(),
            FinalizerFence::new(
                LeaseOwnerId::try_new("w19-finalizer".to_owned()).unwrap(),
                claimed.lease_generation(),
                claimed.lease_until().unwrap(),
            ),
            micros(ACCEPTED + 1),
            micros(ACCEPTED + 2),
        )
        .unwrap();
        let pending = match prepare_accepted_finalization(
            &mut case.store,
            request,
            &case.template,
            &case.policy,
            &conflicting,
        )
        .unwrap()
        {
            AcceptedPreparationOutcome::Pending(pending) => pending,
            other => panic!("unexpected {other:?}"),
        };
        commit_accepted_finalization(
            &mut case.store,
            pending,
            &case.template,
            &case.policy,
            &conflicting,
            micros(ACCEPTED + 3),
            micros(ACCEPTED + 4),
        )
        .unwrap();
        assert_eq!(
            case.query(ACCEPTED + 100_000_000, Duration::from_secs(30))
                .unwrap()
                .status(),
            FinalizationSlaStatus::Conflict
        );
    }
}

#[test]
fn corrupt_persisted_transition_chain_is_rejected_without_exposing_sql() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    case.complete(ACCEPTED + 10_000_000);
    let connection = rusqlite::Connection::open(&case.business_path).unwrap();
    let triggers = connection.prepare("SELECT name FROM sqlite_master WHERE type='trigger' AND tbl_name='push_intent_transitions'").unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    for trigger in triggers {
        connection
            .execute_batch(&format!(
                "DROP TRIGGER \"{}\"",
                trigger.replace('"', "\"\"")
            ))
            .unwrap();
    }
    connection
        .execute(
            "UPDATE push_intent_transitions SET canonical_sha256=? WHERE result_version=1",
            ["a".repeat(64)],
        )
        .unwrap();
    let error = case
        .query(ACCEPTED + 100_000_000, Duration::from_secs(30))
        .unwrap_err();
    assert_eq!(error, FinalizationSlaError::BusinessInvalid);
    assert!(!format!("{error:?} {error}").contains(case.business_path.to_str().unwrap()));
}

#[test]
fn noncompleted_history_must_be_consistent_with_current_persisted_authority() {
    use super::business_finalizer::{
        commit_not_delivered_finalization, prepare_not_delivered_finalization,
        NotDeliveredPreparationOutcome, NotDeliveredPreparationRequest, VerifiedOperatorAuditRef,
    };
    use super::terminal_authority::{
        terminal_binding_sha256, AuthorityQuery, TerminalAuthorityPort,
    };
    use super::terminal_authority_tests::FakeAuthority;
    use crate::monitor::push_job::{TerminalDisposition, TerminalRefId};

    // This sealed seed supplies only the external historical-authority fixture
    // when today's actual reader has no terminal. All observations below query
    // concrete coordinators; none use FakeAuthority as the SLA source.
    let seed = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let seed_adapter =
        GenericTerminalAuthorityAdapter::try_new(seed.durable.as_ref().unwrap()).unwrap();
    let seed_decision = seed.snapshot.attested_ready_binding().unwrap().decision_id;
    let seed_record = match seed_adapter.requery_terminal(&seed_decision).unwrap() {
        AuthorityQuery::Terminal(record) => *record,
        other => panic!("expected sealed seed, got {other:?}"),
    };
    let empty = Case::variant(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
        InitialDecisionKind::NoData,
        None,
    );
    let mut failures = Vec::new();
    for (history, source, mismatch, expected) in [
        (
            "not-delivered",
            "manual-not-delivered",
            "none",
            FinalizationSlaStatus::ManualNotDelivered,
        ),
        (
            "not-delivered",
            "manual-not-delivered",
            "ref",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "not-delivered",
            "manual-not-delivered",
            "binding",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "not-delivered",
            "uncertain",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "not-delivered",
            "missing",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "not-delivered",
            "pending-seal",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "qualified",
            "accepted",
            "none",
            FinalizationSlaStatus::AwaitingFinalization,
        ),
        (
            "qualified",
            "manual-accepted",
            "none",
            FinalizationSlaStatus::ManualAccepted,
        ),
        (
            "qualified",
            "rejected",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "qualified",
            "uncertain",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "qualified",
            "manual-not-delivered",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "qualified",
            "missing",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "qualified",
            "pending-seal",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "resolution-after-qualified",
            "accepted",
            "none",
            FinalizationSlaStatus::ResolutionRequired,
        ),
        (
            "resolution-after-qualified",
            "rejected",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "resolution-after-qualified",
            "missing",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
        (
            "resolution-after-qualified",
            "pending-seal",
            "none",
            FinalizationSlaStatus::Conflict,
        ),
    ] {
        let result = match source {
            "rejected" => AuthoritativeSinkResult::Rejected(TypedRejection {
                reason_code: "TEST_CODE_W19_REJECTED".to_owned(),
                evidence: b"TEST_CODE_W19_REJECTION".to_vec(),
                retry_authorized: false,
                observed_at: utc(ACCEPTED),
            }),
            "uncertain" | "manual-accepted" | "manual-not-delivered" => {
                AuthoritativeSinkResult::Uncertain(TypedUncertainty {
                    reason_code: "TEST_CODE_W19_UNCERTAIN".to_owned(),
                    evidence: b"TEST_CODE_W19_UNCERTAINTY".to_vec(),
                    observed_at: utc(ACCEPTED),
                })
            }
            _ => accepted_result(ACCEPTED),
        };
        let mut case = Case::new(
            AuthorityClass::GenericCounted,
            result,
            source != "pending-seal",
        );
        if matches!(source, "manual-accepted" | "manual-not-delivered") {
            let coordinator = case.durable.as_ref().unwrap();
            coordinator
                .resolve_uncertain(
                    &ManualResolutionCommand {
                        decision_identity: case.legacy_decision.clone().unwrap(),
                        disposition: if source == "manual-accepted" {
                            ManualDisposition::Accepted { receipt: None }
                        } else {
                            ManualDisposition::Rejected
                        },
                        operator_identity: "TEST_CODE_W19_OPERATOR_0123456789".to_owned(),
                        reason: "TEST_CODE_W19_HISTORY_RECONCILIATION".to_owned(),
                        external_evidence: b"TEST_CODE_W19_MANUAL_EVIDENCE".to_vec(),
                        resolved_at: utc(ACCEPTED + 10_000_000),
                    },
                    &case.append,
                )
                .unwrap();
            coordinator
                .reconcile_all_pending(&case.append, utc(ACCEPTED + 11_000_000))
                .unwrap();
        }
        let claimed = case.claim();
        let attested = claimed.attested_ready_binding().unwrap();
        let adapter =
            GenericTerminalAuthorityAdapter::try_new(case.durable.as_ref().unwrap()).unwrap();
        let mut historical = match adapter.requery_terminal(&attested.decision_id).unwrap() {
            AuthorityQuery::Terminal(record) => *record,
            AuthorityQuery::PendingSeal => {
                let mut record = seed_record.clone();
                record.namespace = attested.namespace.clone();
                record.intent_id = attested.intent_id.clone();
                record.decision_id = attested.decision_id.clone();
                record
            }
            other => panic!("unexpected fixture source {other:?}"),
        };
        historical.terminal_disposition = if history == "not-delivered" {
            TerminalDisposition::ManualConfirmedNotDelivered
        } else if source == "manual-accepted" {
            TerminalDisposition::ManualConfirmedAccepted
        } else {
            TerminalDisposition::Accepted
        };
        match mismatch {
            "ref" => {
                historical.ref_id =
                    TerminalRefId::try_new("TEST_CODE_W19_PRIOR_NOT_DELIVERED".to_owned()).unwrap()
            }
            "binding" => {
                historical.evidence_bytes.push(b' ');
                historical.evidence_sha256 = raw_digest(&historical.evidence_bytes);
            }
            _ => {}
        }
        historical.binding_sha256 = terminal_binding_sha256(&historical);
        let mut history_authority = FakeAuthority::terminal(historical);
        history_authority.descriptor = adapter.descriptor().clone();
        let actor = TransitionActor::try_new("w19-finalizer".to_owned()).unwrap();
        let fence = FinalizerFence::new(
            LeaseOwnerId::try_new("w19-finalizer".to_owned()).unwrap(),
            claimed.lease_generation(),
            claimed.lease_until().unwrap(),
        );
        if history == "not-delivered" {
            let audit = VerifiedOperatorAuditRef::for_test(
                case.intent.clone(),
                attested.decision_id,
                claimed.version(),
                "TEST_CODE_W19_OPERATOR_AUDIT".to_owned(),
                raw_digest(b"TEST_CODE_W19_OPERATOR_AUDIT"),
            )
            .unwrap();
            let request = NotDeliveredPreparationRequest::new(
                case.intent.clone(),
                claimed.version(),
                actor,
                fence,
                micros(ACCEPTED + 20_000_000),
                audit,
            )
            .unwrap();
            let pending = match prepare_not_delivered_finalization(
                &mut case.store,
                request,
                &case.template,
                &case.policy,
                &history_authority,
            )
            .unwrap()
            {
                NotDeliveredPreparationOutcome::Pending(pending) => pending,
                other => panic!("expected pending NotDelivered, got {other:?}"),
            };
            commit_not_delivered_finalization(
                &mut case.store,
                pending,
                &case.template,
                &case.policy,
                &history_authority,
                micros(ACCEPTED + 21_000_000),
                micros(ACCEPTED + 22_000_000),
            )
            .unwrap();
        } else {
            let request = AcceptedPreparationRequest::new(
                case.intent.clone(),
                claimed.version(),
                actor,
                fence,
                micros(ACCEPTED + 20_000_000),
                micros(ACCEPTED + 21_000_000),
            )
            .unwrap();
            assert!(matches!(
                prepare_accepted_finalization(
                    &mut case.store,
                    request,
                    &case.template,
                    &case.policy,
                    &history_authority
                )
                .unwrap(),
                AcceptedPreparationOutcome::Pending(_)
            ));
            if history == "resolution-after-qualified" {
                let qualified = case.store.inspect(&case.intent).unwrap().unwrap();
                case.store
                    .apply_nonterminal_transition(
                        &IntentTransitionCommand::try_new(
                            case.intent.clone(),
                            IntentState::AwaitingFinalizer,
                            IntentState::ResolutionRequired,
                            qualified.version(),
                            TransitionActor::try_new("w19-conflict".to_owned()).unwrap(),
                            ReasonCode::FinalizerCasConflict,
                            micros(ACCEPTED + 22_000_000),
                            LeaseAction::Preserve,
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
        }
        case.restart();
        let before = case.rows();
        let sink_calls = case.sink.calls.load(Ordering::SeqCst);
        let append_calls = case.append.calls.load(Ordering::SeqCst);
        let report = if source == "missing" {
            inspect_finalization_sla(
                &case.store,
                FinalizationSlaQuery {
                    namespace: &case.namespace,
                    unit: &case.unit,
                    intent: &case.intent,
                    template: &case.template,
                    policy: &case.policy,
                    route: FinalizationSlaRoute::Generic {
                        source: empty.durable.as_ref().unwrap(),
                        required_channel: &case.channel,
                    },
                    observed_at: micros(ACCEPTED + 400_000_000),
                    reconcile_cycle: Duration::from_secs(30),
                },
            )
            .unwrap()
        } else {
            case.query(ACCEPTED + 400_000_000, Duration::from_secs(30))
                .unwrap()
        };
        assert_eq!(before, case.rows());
        assert_eq!(sink_calls, case.sink.calls.load(Ordering::SeqCst));
        assert_eq!(append_calls, case.append.calls.load(Ordering::SeqCst));
        if report.status() != expected {
            failures.push(format!(
                "{history}/{source}/{mismatch}: expected {expected:?}, observed {:?}",
                report.status()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "persisted non-Completed history conflicts were not rejected: {failures:#?}"
    );
}
