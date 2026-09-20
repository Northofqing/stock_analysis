use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

use super::super::activation_fence::{
    EffectBroker, EffectResult, OperationFact, OperationState, Scope, TestHooks, WorkClass,
};
use super::super::activation_fence_tests::{fixture_clients, fixture_scope};
use super::super::reconciler::RecoveryConfig;
use super::super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialIntentDraft, InitialIntentIdentity,
    IntentSnapshot, IntentState, IntentTransitionCommand, LeaseAction, LeaseOwnerId,
    TerminalTemplateBinding, TransitionActor,
};
use super::*;
use crate::durable_delivery::{
    AuthoritativeDeliveryRequest, AuthoritativeSink, AuthoritativeSinkPort,
    AuthoritativeSinkResult, CoordinatorConfig, DeliveryEnvelope, DeliverySubKind,
    ImmutableAppendPort, PushKind, TypedReceipt, TypedRejection, TypedUncertainty,
};
use crate::event::envelope::{
    news_flash_evidence_sha256, NewsFlashAuditSource, NewsFlashRemoteReceipt,
    NewsFlashTransactionStage,
};
use crate::event::{AuditDispatcher, EventEnvelope, NewsFlashWindow, PushDeliveryEvent};
use crate::monitor::push_job::{
    raw_digest, w09_completion_policy_fixture, AudienceId, AuthorityClass, BusinessDate, ChannelId,
    CompletionOwnerId, Namespace, OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey,
    ReasonCode, RunId, SourceContractId, SubjectId, TemplateId, TemplateVersion, UnitId, UtcMicros,
};

const BUSINESS_DATE: &str = "2026-09-07";
const NOW: i64 = 1_788_743_104_000_000;
const UNTIL: i64 = 1_788_743_400_000_000;
const CHANNEL: &str = "TEST_CODE_W16_P01_CHANNEL";
const N02_NOW: i64 = NOW + 1_800_000_000;
const N02_UNTIL: i64 = N02_NOW + 300_000_000;
const N02_CHANNEL: &str = "TEST_CODE_W16_N02_CHANNEL";

fn micros(value: i64) -> UtcMicros {
    UtcMicros::try_new(value).unwrap()
}

#[derive(Default)]
struct Append {
    records: Mutex<BTreeMap<String, (String, Vec<u8>, String)>>,
}

impl ImmutableAppendPort for Append {
    fn append_exact(
        &self,
        kind: &str,
        identity: &str,
        bytes: &[u8],
        sha256: &str,
    ) -> crate::durable_delivery::Result<String> {
        let proposed = (kind.to_owned(), bytes.to_vec(), sha256.to_owned());
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
    calls: AtomicUsize,
    disposition: P01State,
}

impl AuthoritativeSinkPort for Sink {
    fn sink_identity(&self) -> &str {
        CHANNEL
    }

    fn deliver(&self, _: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let observed = Utc.timestamp_micros(NOW - 4_000_000).single().unwrap();
        match self.disposition {
            P01State::Rejected => AuthoritativeSinkResult::Rejected(TypedRejection {
                reason_code: "TEST_CODE_W16_P01_REJECTED".to_owned(),
                evidence: b"TEST_CODE_W16_P01_REJECTION".to_vec(),
                retry_authorized: false,
                observed_at: observed,
            }),
            P01State::Uncertain => AuthoritativeSinkResult::Uncertain(TypedUncertainty {
                reason_code: "TEST_CODE_W16_P01_UNCERTAIN".to_owned(),
                evidence: b"TEST_CODE_W16_P01_UNCERTAINTY".to_vec(),
                observed_at: observed,
            }),
            P01State::Accepted | P01State::PendingSeal => {
                AuthoritativeSinkResult::Accepted(TypedReceipt {
                    channel: CHANNEL.to_owned(),
                    provider: "TEST_CODE_W16_P01_PROVIDER".to_owned(),
                    message_id: "TEST_CODE_W16_P01_MESSAGE".to_owned(),
                    platform_message_id: None,
                    accepted_at: observed,
                    latency_ms: Some(7),
                })
            }
            P01State::Missing => panic!("missing authority never invokes a sink"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum P01State {
    Accepted,
    Rejected,
    Uncertain,
    PendingSeal,
    Missing,
}

struct P01Fixture {
    _authority_root: tempfile::TempDir,
    durable_database: PathBuf,
    _business_root: tempfile::TempDir,
    database: PathBuf,
    control: PathBuf,
    code: String,
    coordinator: Arc<DurableDeliveryCoordinator>,
    sink: Arc<Sink>,
    snapshot: IntentSnapshot,
    template: TerminalTemplateBinding,
}

impl P01Fixture {
    fn new() -> Self {
        Self::with_state(P01State::Accepted)
    }

    fn with_state(disposition: P01State) -> Self {
        std::fs::create_dir_all("data/test").unwrap();
        let authority_root = tempfile::Builder::new()
            .prefix("TEST_CODE_W16_P01_")
            .tempdir_in("data/test")
            .unwrap();
        let code = authority_root
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let durable_database = authority_root.path().join("durable_delivery.sqlite3");
        let coordinator = Arc::new(
            DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                &durable_database,
                &code,
                format!("owner-{code}"),
            ))
            .unwrap(),
        );
        let business_root = tempfile::Builder::new()
            .prefix("TEST_CODE_W16_P01_BUSINESS_")
            .tempdir_in("/private/tmp")
            .unwrap();
        let database = business_root.path().join("business.sqlite3");
        let control = business_root.path().join("control.sqlite3");
        FoundationSchemaMigration::bundled()
            .unwrap()
            .apply_to(&database)
            .unwrap();
        let template = TerminalTemplateBinding::new(
            TemplateId::try_new("preopen_news_hot_v1".to_owned()).unwrap(),
            TemplateVersion::try_new("preopen_news_hot_v1".to_owned()).unwrap(),
        );
        let identity = InitialIntentIdentity::new(
            Namespace::test(RunId::try_new(code.clone()).unwrap()),
            UnitId::try_new("MU-p01".to_owned()).unwrap(),
            OccurrenceIdentityMaterial::new(
                BusinessDate::parse(BUSINESS_DATE).unwrap(),
                OccurrenceFamily::try_new("p01-business-date".to_owned()).unwrap(),
                OccurrenceKey::try_new(BUSINESS_DATE.to_owned()).unwrap(),
            ),
            CompletionOwnerId::try_new("p01-business-date-once".to_owned()).unwrap(),
            SourceContractId::try_new("w16-p01-source".to_owned()).unwrap(),
            SubjectId::Global,
            AudienceId::try_new("test-owner".to_owned()).unwrap(),
        );
        let draft = InitialIntentDraft::ready_for_recovery_test(
            identity,
            b"TEST_CODE_W16_P01_PREPARED".to_vec(),
            b"TEST_CODE_W16_P01_RENDERED".to_vec(),
            template.sha256().clone(),
            raw_digest(b"TEST_CODE_W16_P01_SOURCE_CONTRACT"),
            micros(NOW - 10_000_000),
        )
        .unwrap();
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let initial = store.record_initial(&draft).unwrap().snapshot().clone();
        let intent_id = initial.attested_ready_binding().unwrap().intent_id;
        store
            .apply_nonterminal_transition(
                &IntentTransitionCommand::try_new(
                    intent_id.clone(),
                    IntentState::PendingDispatch,
                    IntentState::AwaitingAuthority,
                    initial.version(),
                    TransitionActor::try_new("w16-p01-recovery".to_owned()).unwrap(),
                    ReasonCode::IntentDispatchClaimed,
                    micros(NOW - 1_000_000),
                    LeaseAction::Acquire {
                        owner: LeaseOwnerId::try_new("w16-p01-recovery".to_owned()).unwrap(),
                        until: micros(UNTIL),
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let snapshot = store.inspect(&intent_id).unwrap().unwrap();
        let attested = snapshot.attested_ready_binding().unwrap();
        let envelope = DeliveryEnvelope::new(
            BUSINESS_DATE,
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            format!("p01:{BUSINESS_DATE}"),
            attested.source_evidence_fingerprint.as_str(),
            br#"{"render_mode":"Scheduled","schema_version":"P01_SOURCE_BINDING_V1"}"#.to_vec(),
            "TEST_CODE_W16_P01_SUBJECT",
            snapshot.rendered_bytes().unwrap().to_vec(),
            false,
            None,
        )
        .unwrap();
        let append = Append::default();
        let sink = Arc::new(Sink {
            calls: AtomicUsize::new(0),
            disposition,
        });
        if disposition != P01State::Missing {
            coordinator
                .prepare(
                    &envelope,
                    1,
                    Utc.timestamp_micros(NOW - 6_000_000).single().unwrap(),
                )
                .unwrap();
            coordinator
                .reconcile_all_pending(
                    &append,
                    Utc.timestamp_micros(NOW - 5_000_000).single().unwrap(),
                )
                .unwrap();
            let sinks: Vec<AuthoritativeSink> = vec![sink.clone()];
            coordinator
                .resume_deliverable(
                    &envelope.decision_identity,
                    &sinks,
                    Utc.timestamp_micros(NOW - 4_000_000).single().unwrap(),
                )
                .unwrap();
            if disposition != P01State::PendingSeal {
                coordinator
                    .reconcile_all_pending(
                        &append,
                        Utc.timestamp_micros(NOW - 3_000_000).single().unwrap(),
                    )
                    .unwrap();
            }
        }
        Self {
            _authority_root: authority_root,
            durable_database,
            _business_root: business_root,
            database,
            control,
            code,
            coordinator,
            sink,
            snapshot,
            template,
        }
    }

    fn scope(&self) -> Scope {
        Scope {
            namespace: format!("Test:{}", self.code),
            unit: "MU-p01".to_owned(),
            ..fixture_scope()
        }
    }

    fn effect(&self) -> BusinessEffectFixture {
        BusinessEffectFixture {
            database: self.database.clone(),
            coordinator: Arc::clone(&self.coordinator),
            snapshot: self.snapshot.clone(),
            template: self.template.clone(),
            completion_policy: w09_completion_policy_fixture(
                "MU-p01",
                "p01-business-date-once",
                vec![AuthorityClass::P01Dedicated],
            ),
            config: RecoveryConfig::try_new(
                LeaseOwnerId::try_new("w16-p01-recovery".to_owned()).unwrap(),
                TransitionActor::try_new("w16-p01-recovery".to_owned()).unwrap(),
                micros(NOW),
                micros(UNTIL),
                10,
                5,
            )
            .unwrap(),
        }
    }

    fn current(&self) -> IntentSnapshot {
        BusinessIntentStore::open(&self.database)
            .unwrap()
            .inspect(&self.snapshot.attested_ready_binding().unwrap().intent_id)
            .unwrap()
            .unwrap()
    }

    fn broker(&self, hooks: TestHooks) -> Result<EffectBroker, FenceError> {
        EffectBroker::test_p01_business_fixture(
            &self.control,
            self.scope(),
            "p01-epoch".to_owned(),
            self.effect(),
            ChannelId::try_new(CHANNEL.to_owned()).unwrap(),
            fixture_clients(0, 0),
            hooks,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum N02State {
    Accepted,
    Uncertain,
    PendingSeal,
    Missing,
}

struct N02Fixture {
    authority_root: tempfile::TempDir,
    retained_root_observation: (u64, u64, u32, u32, u64),
    _business_root: tempfile::TempDir,
    database: PathBuf,
    control: PathBuf,
    code: String,
    audit: Arc<AuditDispatcher>,
    snapshot: IntentSnapshot,
    template: TerminalTemplateBinding,
}

impl N02Fixture {
    fn new() -> Self {
        Self::with_state(N02State::Accepted)
    }

    fn with_state(state: N02State) -> Self {
        Self::with_identity(state, "news-flash-window", "09:30")
    }

    fn with_identity(state: N02State, occurrence_family: &str, occurrence_key: &str) -> Self {
        std::fs::create_dir_all("data/test").unwrap();
        let authority_root = tempfile::Builder::new()
            .prefix("TEST_CODE_W16_N02_")
            .tempdir_in("data/test")
            .unwrap();
        let code = authority_root
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let audit = Arc::new(AuditDispatcher::for_test_code(&code).unwrap());
        let audit_root_metadata =
            std::fs::metadata(authority_root.path().join("event_audit")).unwrap();
        let retained_root_observation = (
            audit_root_metadata.dev(),
            audit_root_metadata.ino(),
            audit_root_metadata.mode(),
            audit_root_metadata.uid(),
            audit_root_metadata.nlink(),
        );
        let business_root = tempfile::Builder::new()
            .prefix("TEST_CODE_W16_N02_BUSINESS_")
            .tempdir_in("/private/tmp")
            .unwrap();
        let database = business_root.path().join("business.sqlite3");
        let control = business_root.path().join("control.sqlite3");
        FoundationSchemaMigration::bundled()
            .unwrap()
            .apply_to(&database)
            .unwrap();
        let template = TerminalTemplateBinding::new(
            TemplateId::try_new("news_flash_aggregated_v1".to_owned()).unwrap(),
            TemplateVersion::try_new("news_flash_aggregated_v1".to_owned()).unwrap(),
        );
        let identity = InitialIntentIdentity::new(
            Namespace::test(RunId::try_new(code.clone()).unwrap()),
            UnitId::try_new("MU-news-flash-aggregate".to_owned()).unwrap(),
            OccurrenceIdentityMaterial::new(
                BusinessDate::parse(BUSINESS_DATE).unwrap(),
                OccurrenceFamily::try_new(occurrence_family.to_owned()).unwrap(),
                OccurrenceKey::try_new(occurrence_key.to_owned()).unwrap(),
            ),
            CompletionOwnerId::try_new("news-flash-accepted-window".to_owned()).unwrap(),
            SourceContractId::try_new("w16-n02-source".to_owned()).unwrap(),
            SubjectId::Global,
            AudienceId::try_new("test-owner".to_owned()).unwrap(),
        );
        let draft = InitialIntentDraft::ready_for_recovery_test(
            identity,
            b"TEST_CODE_W16_N02_PREPARED".to_vec(),
            b"TEST_CODE_W16_N02_RENDERED".to_vec(),
            template.sha256().clone(),
            raw_digest(b"TEST_CODE_W16_N02_SOURCE_CONTRACT"),
            micros(N02_NOW - 600_000_000),
        )
        .unwrap();
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let initial = store.record_initial(&draft).unwrap().snapshot().clone();
        let intent_id = initial.attested_ready_binding().unwrap().intent_id;
        store
            .apply_nonterminal_transition(
                &IntentTransitionCommand::try_new(
                    intent_id.clone(),
                    IntentState::PendingDispatch,
                    IntentState::AwaitingAuthority,
                    initial.version(),
                    TransitionActor::try_new("w16-n02-recovery".to_owned()).unwrap(),
                    ReasonCode::IntentDispatchClaimed,
                    micros(N02_NOW - 60_000_000),
                    LeaseAction::Acquire {
                        owner: LeaseOwnerId::try_new("w16-n02-recovery".to_owned()).unwrap(),
                        until: micros(N02_UNTIL),
                    },
                )
                .unwrap(),
            )
            .unwrap();
        let snapshot = store.inspect(&intent_id).unwrap().unwrap();
        let date = chrono::NaiveDate::parse_from_str(BUSINESS_DATE, "%Y-%m-%d").unwrap();
        let sources = vec![NewsFlashAuditSource {
            event_id: "TEST_CODE_W16_N02_EVENT".to_owned(),
            provider: "TEST_CODE_W16_N02_PROVIDER".to_owned(),
            source: "TEST_CODE_W16_N02_SOURCE".to_owned(),
            published_at: Utc
                .timestamp_micros(N02_NOW - 500_000_000)
                .single()
                .unwrap()
                .fixed_offset(),
            observed_at: Utc
                .timestamp_micros(N02_NOW - 490_000_000)
                .single()
                .unwrap()
                .fixed_offset(),
            batch_id: "TEST_CODE_W16_N02_BATCH".to_owned(),
        }];
        let evidence = news_flash_evidence_sha256(&sources);
        let rendered_sha256 = snapshot.rendered_sha256().unwrap().as_str().to_owned();
        let rendered_len = snapshot.rendered_bytes().unwrap().len();
        let attempt_at = Utc
            .timestamp_micros(N02_NOW - 300_000_000)
            .single()
            .unwrap()
            .fixed_offset();
        let decision_key = if state == N02State::Missing {
            "window:11:30"
        } else {
            "window:09:30"
        };
        let attempt_event = PushDeliveryEvent::new_news_flash_attempt(
            "news_flash_aggregated_v1".to_owned(),
            decision_key.to_owned(),
            N02_CHANNEL.to_owned(),
            rendered_len,
            date,
            "a".repeat(64),
            sources.clone(),
            evidence.clone(),
            rendered_sha256.clone(),
            1,
            attempt_at,
        );
        let attempt = EventEnvelope::from_event(
            &attempt_event,
            attempt_event.news_flash_join_sha256.clone().unwrap(),
            "TEST_CODE_W16_N02_ATTEMPT".to_owned(),
            attempt_at.with_timezone(&chrono::Local),
        )
        .unwrap();
        audit.append_exact_news_flash_authority(&attempt).unwrap();
        if state == N02State::PendingSeal {
            return Self {
                authority_root,
                retained_root_observation,
                _business_root: business_root,
                database,
                control,
                code,
                audit,
                snapshot,
                template,
            };
        }
        let accepted_at = Utc
            .timestamp_micros(N02_NOW - 240_000_000)
            .single()
            .unwrap()
            .fixed_offset();
        let terminal_at = Utc
            .timestamp_micros(N02_NOW - 180_000_000)
            .single()
            .unwrap()
            .fixed_offset();
        let receipt = NewsFlashRemoteReceipt {
            channel: N02_CHANNEL.to_owned(),
            provider: "TEST_CODE_W16_N02_PROVIDER".to_owned(),
            message_id: "TEST_CODE_W16_N02_MESSAGE".to_owned(),
            platform_message_id: "TEST_CODE_W16_N02_PLATFORM_MESSAGE".to_owned(),
            accepted_at,
            latency_ms: 7,
        };
        let terminal_stage = if state == N02State::Uncertain {
            NewsFlashTransactionStage::Uncertain
        } else {
            NewsFlashTransactionStage::Accepted
        };
        let terminal_event = PushDeliveryEvent::new_news_flash_terminal(
            terminal_stage,
            "news_flash_aggregated_v1".to_owned(),
            decision_key.to_owned(),
            N02_CHANNEL.to_owned(),
            rendered_len,
            3,
            date,
            "a".repeat(64),
            sources,
            evidence,
            rendered_sha256,
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
            (terminal_stage == NewsFlashTransactionStage::Accepted).then_some(receipt),
            terminal_at,
            (terminal_stage == NewsFlashTransactionStage::Uncertain)
                .then(|| "TEST_CODE_W16_N02_UNCERTAIN".to_owned()),
            None,
        );
        let terminal = EventEnvelope::from_event(
            &terminal_event,
            terminal_event.news_flash_join_sha256.clone().unwrap(),
            "TEST_CODE_W16_N02_TERMINAL".to_owned(),
            terminal_at.with_timezone(&chrono::Local),
        )
        .unwrap();
        audit.append_exact_news_flash_authority(&terminal).unwrap();
        Self {
            authority_root,
            retained_root_observation,
            _business_root: business_root,
            database,
            control,
            code,
            audit,
            snapshot,
            template,
        }
    }

    fn scope(&self) -> Scope {
        Scope {
            namespace: format!("Test:{}", self.code),
            unit: "MU-news-flash-aggregate".to_owned(),
            ..fixture_scope()
        }
    }

    fn effect(&self) -> N02BusinessEffectFixture {
        N02BusinessEffectFixture {
            database: self.database.clone(),
            audit: Arc::clone(&self.audit),
            snapshot: self.snapshot.clone(),
            template: self.template.clone(),
            completion_policy: w09_completion_policy_fixture(
                "MU-news-flash-aggregate",
                "news-flash-accepted-window",
                vec![AuthorityClass::N02Dedicated],
            ),
            config: RecoveryConfig::try_new(
                LeaseOwnerId::try_new("w16-n02-recovery".to_owned()).unwrap(),
                TransitionActor::try_new("w16-n02-recovery".to_owned()).unwrap(),
                micros(N02_NOW),
                micros(N02_UNTIL),
                10,
                5,
            )
            .unwrap(),
            required_channel: ChannelId::try_new(N02_CHANNEL.to_owned()).unwrap(),
            window: NewsFlashWindow::H0930,
        }
    }

    fn current(&self) -> IntentSnapshot {
        BusinessIntentStore::open(&self.database)
            .unwrap()
            .inspect(&self.snapshot.attested_ready_binding().unwrap().intent_id)
            .unwrap()
            .unwrap()
    }

    fn add_other_window_intent(&self) -> IntentSnapshot {
        let draft = InitialIntentDraft::ready_for_recovery_test(
            InitialIntentIdentity::new(
                Namespace::test(RunId::try_new(self.code.clone()).unwrap()),
                UnitId::try_new("MU-news-flash-aggregate".to_owned()).unwrap(),
                OccurrenceIdentityMaterial::new(
                    BusinessDate::parse(BUSINESS_DATE).unwrap(),
                    OccurrenceFamily::try_new("news-flash-window".to_owned()).unwrap(),
                    OccurrenceKey::try_new("11:30".to_owned()).unwrap(),
                ),
                CompletionOwnerId::try_new("news-flash-accepted-window".to_owned()).unwrap(),
                SourceContractId::try_new("w16-n02-other-source".to_owned()).unwrap(),
                SubjectId::Global,
                AudienceId::try_new("test-owner".to_owned()).unwrap(),
            ),
            b"TEST_CODE_W16_N02_OTHER_PREPARED".to_vec(),
            b"TEST_CODE_W16_N02_OTHER_RENDERED".to_vec(),
            self.template.sha256().clone(),
            raw_digest(b"TEST_CODE_W16_N02_OTHER_SOURCE_CONTRACT"),
            micros(N02_NOW - 700_000_000),
        )
        .unwrap();
        BusinessIntentStore::open(&self.database)
            .unwrap()
            .record_initial(&draft)
            .unwrap()
            .snapshot()
            .clone()
    }

    fn audit_bytes(&self) -> Vec<(String, Vec<u8>)> {
        let mut paths = std::fs::read_dir(self.authority_root.path().join("event_audit"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read(path).unwrap(),
                )
            })
            .collect()
    }

    fn broker(&self, hooks: TestHooks) -> Result<EffectBroker, FenceError> {
        EffectBroker::test_n02_business_fixture(
            &self.control,
            self.scope(),
            "n02-epoch".to_owned(),
            self.effect(),
            fixture_clients(0, 0),
            hooks,
        )
    }
}

async fn finish(broker: &EffectBroker, request: &EffectRequest) -> OperationFact {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let fact = broker.query_bounded(request).await.unwrap().unwrap();
            if fact.state != OperationState::Running {
                break fact;
            }
        }
    })
    .await
    .unwrap()
}

async fn authority_pause(listener: &UnixListener, marker: u8) -> tokio::net::UnixStream {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut observed = [0_u8];
        stream.read_exact(&mut observed).await.unwrap();
        assert_eq!(observed, [marker]);
        stream
    })
    .await
    .expect("actual authority read reached")
}

#[tokio::test]
async fn p01_accepted_real_authority_completes_and_replays_without_resend() {
    let fixture = P01Fixture::new();
    let terminal_before = fixture
        .coordinator
        .inspect_p01_dedicated_terminal(BUSINESS_DATE)
        .unwrap();
    let calls_before = fixture.sink.calls.load(Ordering::SeqCst);
    let broker = fixture.broker(TestHooks::default()).unwrap();
    let request = broker.test_business_request("p01-operation");
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Succeeded);
    let Some(EffectResult::BusinessRecovery(result)) = &fact.result else {
        panic!("dedicated business result required")
    };
    assert_eq!(result.state, "Completed");
    assert_eq!(result.recovery_boundary, "Finalized");
    assert_eq!(result.version, 3);
    assert_eq!(result.transition_count, 3);
    let completed = fixture.current();
    assert_eq!(completed.state(), IntentState::Completed);
    let chain = BusinessIntentStore::open(&fixture.database)
        .unwrap()
        .inspect_transition_chain(&completed.attested_ready_binding().unwrap().intent_id)
        .unwrap();
    assert_eq!(chain.len(), 3);
    assert_eq!(broker.execute_current(request).unwrap(), fact);
    assert_eq!(fixture.current(), completed);
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), calls_before);
    assert_eq!(
        fixture
            .coordinator
            .inspect_p01_dedicated_terminal(BUSINESS_DATE)
            .unwrap(),
        terminal_before
    );
}

#[tokio::test]
async fn n02_accepted_real_authority_completes_and_replays_without_append() {
    let fixture = N02Fixture::new();
    let other_window = fixture.add_other_window_intent();
    let audit_before = fixture.audit_bytes();
    let handled_before = fixture.audit.handled_count();
    let broker = fixture.broker(TestHooks::default()).unwrap();
    let request = broker.test_business_request("n02-operation");
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Succeeded);
    let Some(EffectResult::BusinessRecovery(result)) = &fact.result else {
        panic!("dedicated business result required")
    };
    assert_eq!(result.state, "Completed");
    assert_eq!(result.recovery_boundary, "Finalized");
    assert_eq!(result.version, 3);
    assert_eq!(result.transition_count, 3);
    let completed = fixture.current();
    assert_eq!(completed.state(), IntentState::Completed);
    let chain = BusinessIntentStore::open(&fixture.database)
        .unwrap()
        .inspect_transition_chain(&completed.attested_ready_binding().unwrap().intent_id)
        .unwrap();
    assert_eq!(chain.len(), 3);
    assert_eq!(broker.execute_current(request).unwrap(), fact);
    assert_eq!(fixture.current(), completed);
    assert_eq!(fixture.audit.handled_count(), handled_before);
    assert_eq!(fixture.audit_bytes(), audit_before);
    assert_eq!(
        BusinessIntentStore::open(&fixture.database)
            .unwrap()
            .inspect(&other_window.attested_ready_binding().unwrap().intent_id)
            .unwrap(),
        Some(other_window)
    );
}

#[tokio::test]
async fn p01_nonaccepted_and_unsealed_authority_never_complete_or_resend() {
    for (source, expected_state, expected_boundary) in [
        (
            P01State::Missing,
            IntentState::AwaitingAuthority,
            "AuthorityBlocked",
        ),
        (
            P01State::PendingSeal,
            IntentState::AwaitingAuthority,
            "AuthorityBlocked",
        ),
        (
            P01State::Rejected,
            IntentState::AwaitingAuthority,
            "RejectedAuthorizationRequired",
        ),
        (
            P01State::Uncertain,
            IntentState::ResolutionRequired,
            "ManualResolutionRequired",
        ),
    ] {
        let fixture = P01Fixture::with_state(source);
        let terminal_before = fixture
            .coordinator
            .inspect_p01_dedicated_terminal(BUSINESS_DATE)
            .unwrap();
        let calls_before = fixture.sink.calls.load(Ordering::SeqCst);
        let broker = fixture.broker(TestHooks::default()).unwrap();
        let request = broker.test_business_request(&format!("p01-{source:?}"));
        broker.execute_current(request.clone()).unwrap();
        let fact = finish(&broker, &request).await;
        assert_eq!(fact.state, OperationState::Unresolved);
        let Some(EffectResult::BusinessRecovery(result)) = fact.result else {
            panic!("nonaccepted P01 recovery result required")
        };
        assert_eq!(result.state, expected_state.as_str());
        assert_eq!(result.recovery_boundary, expected_boundary);
        assert_eq!(fixture.current().state(), expected_state);
        assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), calls_before);
        assert_eq!(
            fixture
                .coordinator
                .inspect_p01_dedicated_terminal(BUSINESS_DATE)
                .unwrap(),
            terminal_before
        );
    }
}

#[tokio::test]
async fn n02_missing_pending_and_real_uncertain_never_complete_or_append() {
    for (source, expected_state, expected_boundary) in [
        (
            N02State::Missing,
            IntentState::AwaitingAuthority,
            "AuthorityBlocked",
        ),
        (
            N02State::PendingSeal,
            IntentState::AwaitingAuthority,
            "AuthorityBlocked",
        ),
        (
            N02State::Uncertain,
            IntentState::ResolutionRequired,
            "ManualResolutionRequired",
        ),
    ] {
        let fixture = N02Fixture::with_state(source);
        let audit_before = fixture.audit_bytes();
        let handled_before = fixture.audit.handled_count();
        let broker = fixture.broker(TestHooks::default()).unwrap();
        let request = broker.test_business_request(&format!("n02-{source:?}"));
        broker.execute_current(request.clone()).unwrap();
        let fact = finish(&broker, &request).await;
        assert_eq!(fact.state, OperationState::Unresolved);
        let Some(EffectResult::BusinessRecovery(result)) = fact.result else {
            panic!("nonaccepted N02 recovery result required")
        };
        assert_eq!(result.state, expected_state.as_str());
        assert_eq!(result.recovery_boundary, expected_boundary);
        assert_eq!(fixture.current().state(), expected_state);
        assert_eq!(fixture.audit.handled_count(), handled_before);
        assert_eq!(fixture.audit_bytes(), audit_before);
    }
}

#[tokio::test]
async fn n02_missing_lock_is_a_cold_read_failure_and_never_completes() {
    let fixture = N02Fixture::new();
    let broker = fixture.broker(TestHooks::default()).unwrap();
    let lock = fixture.authority_root.path().join("event_audit/2026.lock");
    let moved = fixture
        .authority_root
        .path()
        .join("event_audit/2026.lock.moved");
    std::fs::rename(&lock, &moved).unwrap();
    let before = fixture.current();
    let request = broker.test_business_request("n02-missing-lock");
    broker.execute_current(request.clone()).unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Unresolved);
    assert!(fact.result.is_none());
    assert_eq!(fixture.current(), before);
    assert!(
        !lock.exists(),
        "the read-only recovery path must not recreate lock"
    );
}

#[tokio::test]
async fn dedicated_binding_rejects_cross_scope_route_policy_and_source_before_writes() {
    let p01 = P01Fixture::new();
    for scope in [
        Scope {
            namespace: "Test:TEST_CODE_OTHER_NAMESPACE".to_owned(),
            ..p01.scope()
        },
        Scope {
            unit: "MU-other".to_owned(),
            ..p01.scope()
        },
    ] {
        assert!(BusinessEffect::bind_p01_fixture(
            &scope,
            p01.effect(),
            ChannelId::try_new(CHANNEL.to_owned()).unwrap(),
        )
        .is_err());
    }
    let mut wrong_template = p01.effect();
    wrong_template.template = TerminalTemplateBinding::new(
        TemplateId::try_new("different-template".to_owned()).unwrap(),
        TemplateVersion::try_new("different-template".to_owned()).unwrap(),
    );
    assert!(BusinessEffect::bind_p01_fixture(
        &p01.scope(),
        wrong_template,
        ChannelId::try_new(CHANNEL.to_owned()).unwrap(),
    )
    .is_err());
    let mut wrong_policy = p01.effect();
    wrong_policy.completion_policy = w09_completion_policy_fixture(
        "MU-p01",
        "p01-business-date-once",
        vec![AuthorityClass::N02Dedicated],
    );
    assert!(BusinessEffect::bind_p01_fixture(
        &p01.scope(),
        wrong_policy,
        ChannelId::try_new(CHANNEL.to_owned()).unwrap(),
    )
    .is_err());

    let n02 = N02Fixture::new();
    let other = N02Fixture::new();
    let mut cross_root = n02.effect();
    cross_root.audit = Arc::clone(&other.audit);
    assert!(BusinessEffect::bind_n02_fixture(&n02.scope(), cross_root).is_err());
    let mut wrong_window = n02.effect();
    wrong_window.window = NewsFlashWindow::H1130;
    assert!(BusinessEffect::bind_n02_fixture(&n02.scope(), wrong_window).is_err());
    for fixture in [
        N02Fixture::with_identity(N02State::Accepted, "unknown-family", "09:30"),
        N02Fixture::with_identity(N02State::Accepted, "news-flash-window", "unknown-key"),
    ] {
        assert!(BusinessEffect::bind_n02_fixture(&fixture.scope(), fixture.effect()).is_err());
        assert_eq!(fixture.current().state(), IntentState::AwaitingAuthority);
    }
}

#[tokio::test]
async fn dedicated_wrong_channel_and_request_identity_never_change_business_state() {
    let p01 = P01Fixture::new();
    let before = p01.current();
    let terminal_before = p01
        .coordinator
        .inspect_p01_dedicated_terminal(BUSINESS_DATE)
        .unwrap();
    let calls_before = p01.sink.calls.load(Ordering::SeqCst);
    assert!(EffectBroker::test_p01_business_fixture(
        &p01.control,
        p01.scope(),
        "p01-wrong-channel".to_owned(),
        p01.effect(),
        ChannelId::try_new("TEST_CODE_WRONG_CHANNEL".to_owned()).unwrap(),
        fixture_clients(0, 0),
        TestHooks::default(),
    )
    .is_err());
    assert_eq!(p01.current(), before);
    assert_eq!(p01.sink.calls.load(Ordering::SeqCst), calls_before);
    assert_eq!(
        p01.coordinator
            .inspect_p01_dedicated_terminal(BUSINESS_DATE)
            .unwrap(),
        terminal_before
    );

    let n02 = N02Fixture::new();
    let before = n02.current();
    let audit_before = n02.audit_bytes();
    let mut effect = n02.effect();
    effect.required_channel = ChannelId::try_new("TEST_CODE_WRONG_CHANNEL".to_owned()).unwrap();
    assert!(EffectBroker::test_n02_business_fixture(
        &n02.control,
        n02.scope(),
        "n02-wrong-channel".to_owned(),
        effect,
        fixture_clients(0, 0),
        TestHooks::default(),
    )
    .is_err());
    assert_eq!(n02.current(), before);
    assert_eq!(n02.audit_bytes(), audit_before);

    let broker = n02.broker(TestHooks::default()).unwrap();
    let valid = broker.test_business_request("n02-wrong-channel");
    for index in 0..10 {
        let mut changed = valid.clone();
        match index {
            0 => changed.scope.namespace.push_str("-other"),
            1 => changed.scope.unit.push_str("-other"),
            2 => changed.scope.generation += 1,
            3 => changed.scope.physical_owner.push_str("-other"),
            4 => changed.broker_epoch.push_str("-other"),
            5 => changed.actor.push_str("-other"),
            6 => changed.action.push_str("-other"),
            7 => changed.work_class = WorkClass::NewWork,
            8 => changed.effect_id.push_str("-other"),
            9 => changed.effect_sha256.push('0'),
            _ => unreachable!(),
        }
        assert!(broker.execute_current(changed).is_err());
    }
    assert_eq!(n02.current(), before);
    assert_eq!(n02.audit_bytes(), audit_before);
}

#[tokio::test]
async fn p01_database_replacement_between_real_queries_cannot_complete() {
    let fixture = P01Fixture::new();
    let pause_path = fixture.control.with_extension("p01-requery.sock");
    let listener = UnixListener::bind(&pause_path).unwrap();
    let broker = fixture
        .broker(TestHooks {
            business_requery_pause_socket: Some(pause_path),
            ..TestHooks::default()
        })
        .unwrap();
    let request = broker.test_business_request("p01-replaced-source");
    broker.execute_current(request.clone()).unwrap();
    let mut paused = authority_pause(&listener, b'P').await;
    let retained = fixture.durable_database.with_extension("retained");
    std::fs::rename(&fixture.durable_database, &retained).unwrap();
    std::fs::copy(&retained, &fixture.durable_database).unwrap();
    paused.write_all(b"G").await.unwrap();
    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Unresolved);
    assert_ne!(fixture.current().state(), IntentState::Completed);
    assert_eq!(fixture.sink.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn n02_bound_reader_rejects_jsonl_swap_even_when_path_is_restored_after_open() {
    let fixture = N02Fixture::new();
    let pause_path = fixture.control.with_extension("n02-bound-read.sock");
    let listener = UnixListener::bind(&pause_path).unwrap();
    let broker = fixture.broker(TestHooks::default()).unwrap();
    fixture.audit.set_activation_read_pause(pause_path);
    let request = broker.test_business_request("n02-open-fd-replaced-source");
    let original = fixture.authority_root.path().join("event_audit/2026.jsonl");
    let replacement = fixture
        .authority_root
        .path()
        .join("event_audit/2026.replacement.jsonl");
    let retained = fixture
        .authority_root
        .path()
        .join("event_audit/2026.retained.jsonl");
    let consumed = fixture
        .authority_root
        .path()
        .join("event_audit/2026.consumed.jsonl");
    std::fs::copy(&original, &replacement).unwrap();

    broker.execute_current(request.clone()).unwrap();
    for marker in [b'B', b'O'] {
        let mut pause = authority_pause(&listener, marker).await;
        pause.write_all(b"G").await.unwrap();
    }
    let mut before_open = authority_pause(&listener, b'B').await;
    std::fs::rename(&original, &retained).unwrap();
    std::fs::rename(&replacement, &original).unwrap();
    before_open.write_all(b"G").await.unwrap();
    let mut after_open = authority_pause(&listener, b'O').await;
    std::fs::rename(&original, &consumed).unwrap();
    std::fs::rename(&retained, &original).unwrap();
    after_open.write_all(b"G").await.unwrap();

    let fact = finish(&broker, &request).await;
    assert_eq!(fact.state, OperationState::Unresolved);
    assert_ne!(fixture.current().state(), IntentState::Completed);
    assert!(original.exists());
}

#[test]
fn dedicated_effect_has_independent_source_literal_and_every_source_field_changes_bytes() {
    let p01 = P01Fixture::new();
    let p01_effect = BusinessEffect::bind_p01_fixture(
        &p01.scope(),
        p01.effect(),
        ChannelId::try_new(CHANNEL.to_owned()).unwrap(),
    )
    .unwrap();
    let p01_bytes = p01_effect.canonical_bytes().unwrap();
    let (domain, body) =
        p01_bytes.split_at(p01_bytes.iter().position(|byte| *byte == 0).unwrap() + 1);
    assert_eq!(domain, b"ActivationDedicatedBusinessRecoveryEffect/v1\0");
    let p01_json: serde_json::Value = serde_json::from_slice(body).unwrap();
    let binding = p01.coordinator.activation_storage_binding().unwrap();
    let p01_source_binding = serde_json::json!({
        "device": binding.1,
        "inode": binding.2,
        "kind": "P01DurableDeliveryCoordinator",
        "namespace": binding.3,
        "owner_instance": binding.4,
        "path": binding.0,
    });
    assert_eq!(p01_json["source_binding"], p01_source_binding);
    assert_eq!(p01_json["authority_class"], "P01Dedicated");
    assert_eq!(
        p01_json["authority_schema_version"],
        format!(
            "p01-durable-v{}",
            crate::durable_delivery::DURABLE_SCHEMA_VERSION
        )
    );
    assert_eq!(p01_json["required_channel"], CHANNEL);
    assert!(p01_json.get("window").is_none());
    let policy = concat!(
        "ActivationCompletionPolicy/v1\0{\"advance_event\":\"AcceptedOrManualBound\",\"allowed_authority\":[\"P01Dedicated\"],\"already_terminal_policy\":\"RequeryExactBinding\",",
        "\"completion_owner\":{\"catalog_sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"completion_owner\":\"p01-business-date-once\",\"unit_id\":\"MU-p01\"},",
        "\"disabled_policy\":\"CloseDisabledOccurrence\",\"finalizer_kind\":\"BoundCursor\",\"id\":\"fixture-policy\",\"no_data_policy\":\"CloseVerifiedOccurrence\",",
        "\"notification_cursor_policy\":\"AcceptedBoundOnly\",\"retention_class\":\"Trading\",\"retry_policy\":{\"kind\":\"Never\",\"max_attempts\":null,\"not_before\":null},",
        "\"schedule_close_policy\":[\"ExplicitDisabled\",\"OnAccepted\",\"SuppressedOccurrence\",\"VerifiedNoData\"],\"uncertain_manual_policy\":\"QuarantineThenVerifiedManual\",\"version\":\"v1\"}"
    );
    assert_eq!(
        p01_effect.completion_policy.activation_binding_bytes(),
        policy.as_bytes()
    );
    let intent = p01.snapshot.attested_ready_binding().unwrap();
    let prepared = b"TEST_CODE_W16_P01_PREPARED";
    let rendered = b"TEST_CODE_W16_P01_RENDERED";
    let snapshot = serde_json::json!({
        "audience":"test-owner","business_date":BUSINESS_DATE,"completion_owner":"p01-business-date-once","created_at":(NOW - 10_000_000).to_string(),
        "decision_id":intent.decision_id.as_str(),"decision_kind":"Ready","evidence_sha256":raw_digest(prepared).as_str(),"intent_id":intent.intent_id.as_str(),
        "lease_generation":1,"lease_owner":"w16-p01-recovery","lease_until":UNTIL.to_string(),"namespace":format!("Test:{}",p01.code),
        "occurrence_family":"p01-business-date","occurrence_key":BUSINESS_DATE,"payload_sha256":raw_digest(prepared).as_str(),
        "prepared_push_bytes":prepared,"previous_state":"PendingDispatch","reason":"intent.dispatch_claimed","rendered_bytes":rendered,
        "rendered_sha256":raw_digest(rendered).as_str(),"source_contract_id":"w16-p01-source",
        "source_contract_sha256":raw_digest(b"TEST_CODE_W16_P01_SOURCE_CONTRACT").as_str(),"state":"AwaitingAuthority","subject":"Global",
        "template_sha256":p01.template.sha256().as_str(),"unit_id":"MU-p01","updated_at":(NOW - 1_000_000).to_string(),"version":1
    });
    let business = std::fs::canonicalize(&p01.database).unwrap();
    let business_metadata = std::fs::metadata(&business).unwrap();
    let expected = serde_json::json!({
        "action":"ReconcileBusiness","actor":"Finalizer","authority_class":"P01Dedicated",
        "authority_schema_version":format!("p01-durable-v{}",crate::durable_delivery::DURABLE_SCHEMA_VERSION),
        "business_store_device":business_metadata.dev(),"business_store_inode":business_metadata.ino(),"business_store_path":business.to_str().unwrap(),
        "completion_policy_bytes":policy.as_bytes(),"deployment":"fixture-deployment","effect_id":"business-reconcile","generation":11,
        "incarnation":"deployment-one","manifest":"a".repeat(64),"namespace":format!("Test:{}",p01.code),"physical_owner":"generic-owner",
        "recovery_config":{"actor":"w16-p01-recovery","lease_until":UNTIL.to_string(),"max_iterations":5,"now":NOW.to_string(),"owner":"w16-p01-recovery","page_size":10},
        "required_channel":CHANNEL,"snapshot":snapshot,"source_binding":p01_source_binding,
        "template_id":"preopen_news_hot_v1","template_sha256":p01.template.sha256().as_str(),"template_version":"preopen_news_hot_v1",
        "unit":"MU-p01","work_class":"Recovery"
    });
    let expected = [
        b"ActivationDedicatedBusinessRecoveryEffect/v1\0".as_slice(),
        serde_json::to_vec(&expected).unwrap().as_slice(),
    ]
    .concat();
    assert_eq!(p01_bytes, expected);
    assert_eq!(p01_effect.digest(), raw_digest(&expected).as_str());
    for index in 0..8 {
        let mut changed = BusinessEffect::bind_p01_fixture(
            &p01.scope(),
            p01.effect(),
            ChannelId::try_new(CHANNEL.to_owned()).unwrap(),
        )
        .unwrap();
        changed.source.mutate_dedicated_canonical_field(index);
        assert_ne!(
            changed.canonical_bytes().unwrap(),
            p01_bytes,
            "P01 field {index}"
        );
    }

    let n02 = N02Fixture::new();
    let n02_effect = BusinessEffect::bind_n02_fixture(&n02.scope(), n02.effect()).unwrap();
    let n02_bytes = n02_effect.canonical_bytes().unwrap();
    assert!(n02_bytes.starts_with(b"ActivationDedicatedBusinessRecoveryEffect/v1\0"));
    let n02_json: serde_json::Value = serde_json::from_slice(
        &n02_bytes[b"ActivationDedicatedBusinessRecoveryEffect/v1\0".len()..],
    )
    .unwrap();
    let audit_root = n02.authority_root.path().join("event_audit");
    let lock = std::fs::metadata(audit_root.join("2026.lock")).unwrap();
    let jsonl = std::fs::metadata(audit_root.join("2026.jsonl")).unwrap();
    let (root_device, root_inode, root_mode, root_uid, root_links) = n02.retained_root_observation;
    let n02_source_binding = serde_json::json!({
        "jsonl_device": jsonl.dev(), "jsonl_inode": jsonl.ino(), "jsonl_links": jsonl.nlink(),
        "jsonl_mode": jsonl.mode(), "jsonl_uid": jsonl.uid(), "kind": "N02AuditDispatcher",
        "lock_device": lock.dev(), "lock_inode": lock.ino(), "lock_links": lock.nlink(),
        "lock_mode": lock.mode(), "lock_uid": lock.uid(), "namespace": format!("Test:{}", n02.code),
        "path": std::fs::canonicalize(&audit_root).unwrap().to_str().unwrap(),
        "root_device": root_device, "root_inode": root_inode, "root_links": root_links,
        "root_mode": root_mode, "root_uid": root_uid, "year": "2026",
    });
    assert_eq!(n02_json["source_binding"], n02_source_binding);
    assert_eq!(n02_json["authority_class"], "N02Dedicated");
    assert_eq!(
        n02_json["authority_schema_version"],
        "news-flash-authority-v5"
    );
    assert_eq!(n02_json["required_channel"], N02_CHANNEL);
    assert_eq!(n02_json["window"], "09:30");
    let n02_policy = concat!(
        "ActivationCompletionPolicy/v1\0{\"advance_event\":\"AcceptedOrManualBound\",\"allowed_authority\":[\"N02Dedicated\"],\"already_terminal_policy\":\"RequeryExactBinding\",",
        "\"completion_owner\":{\"catalog_sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",\"completion_owner\":\"news-flash-accepted-window\",\"unit_id\":\"MU-news-flash-aggregate\"},",
        "\"disabled_policy\":\"CloseDisabledOccurrence\",\"finalizer_kind\":\"BoundCursor\",\"id\":\"fixture-policy\",\"no_data_policy\":\"CloseVerifiedOccurrence\",",
        "\"notification_cursor_policy\":\"AcceptedBoundOnly\",\"retention_class\":\"Trading\",\"retry_policy\":{\"kind\":\"Never\",\"max_attempts\":null,\"not_before\":null},",
        "\"schedule_close_policy\":[\"ExplicitDisabled\",\"OnAccepted\",\"SuppressedOccurrence\",\"VerifiedNoData\"],\"uncertain_manual_policy\":\"QuarantineThenVerifiedManual\",\"version\":\"v1\"}"
    );
    assert_eq!(
        n02_effect.completion_policy.activation_binding_bytes(),
        n02_policy.as_bytes()
    );
    let intent = n02.snapshot.attested_ready_binding().unwrap();
    let prepared = b"TEST_CODE_W16_N02_PREPARED";
    let rendered = b"TEST_CODE_W16_N02_RENDERED";
    let snapshot = serde_json::json!({
        "audience":"test-owner","business_date":BUSINESS_DATE,"completion_owner":"news-flash-accepted-window","created_at":(N02_NOW - 600_000_000).to_string(),
        "decision_id":intent.decision_id.as_str(),"decision_kind":"Ready","evidence_sha256":raw_digest(prepared).as_str(),"intent_id":intent.intent_id.as_str(),
        "lease_generation":1,"lease_owner":"w16-n02-recovery","lease_until":N02_UNTIL.to_string(),"namespace":format!("Test:{}",n02.code),
        "occurrence_family":"news-flash-window","occurrence_key":"09:30","payload_sha256":raw_digest(prepared).as_str(),
        "prepared_push_bytes":prepared,"previous_state":"PendingDispatch","reason":"intent.dispatch_claimed","rendered_bytes":rendered,
        "rendered_sha256":raw_digest(rendered).as_str(),"source_contract_id":"w16-n02-source",
        "source_contract_sha256":raw_digest(b"TEST_CODE_W16_N02_SOURCE_CONTRACT").as_str(),"state":"AwaitingAuthority","subject":"Global",
        "template_sha256":n02.template.sha256().as_str(),"unit_id":"MU-news-flash-aggregate","updated_at":(N02_NOW - 60_000_000).to_string(),"version":1
    });
    let business = std::fs::canonicalize(&n02.database).unwrap();
    let business_metadata = std::fs::metadata(&business).unwrap();
    let expected = serde_json::json!({
        "action":"ReconcileBusiness","actor":"Finalizer","authority_class":"N02Dedicated","authority_schema_version":"news-flash-authority-v5",
        "business_store_device":business_metadata.dev(),"business_store_inode":business_metadata.ino(),"business_store_path":business.to_str().unwrap(),
        "completion_policy_bytes":n02_policy.as_bytes(),"deployment":"fixture-deployment","effect_id":"business-reconcile","generation":11,
        "incarnation":"deployment-one","manifest":"a".repeat(64),"namespace":format!("Test:{}",n02.code),"physical_owner":"generic-owner",
        "recovery_config":{"actor":"w16-n02-recovery","lease_until":N02_UNTIL.to_string(),"max_iterations":5,"now":N02_NOW.to_string(),"owner":"w16-n02-recovery","page_size":10},
        "required_channel":N02_CHANNEL,"snapshot":snapshot,"source_binding":n02_source_binding,
        "template_id":"news_flash_aggregated_v1","template_sha256":n02.template.sha256().as_str(),"template_version":"news_flash_aggregated_v1",
        "unit":"MU-news-flash-aggregate","window":"09:30","work_class":"Recovery"
    });
    let expected = [
        b"ActivationDedicatedBusinessRecoveryEffect/v1\0".as_slice(),
        serde_json::to_vec(&expected).unwrap().as_slice(),
    ]
    .concat();
    assert_eq!(n02_bytes, expected);
    assert_eq!(n02_effect.digest(), raw_digest(&expected).as_str());
    for index in 0..22 {
        let mut changed = BusinessEffect::bind_n02_fixture(&n02.scope(), n02.effect()).unwrap();
        changed.source.mutate_dedicated_canonical_field(index);
        assert_ne!(
            changed.canonical_bytes().unwrap(),
            n02_bytes,
            "N02 field {index}"
        );
    }
}
