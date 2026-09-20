//! Dedicated P01/N02 business recovery through the real child-process Unix wire.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use super::activation_business_effect::{BusinessEffectFixture, N02BusinessEffectFixture};
use super::activation_business_process_tests::{assert_completion_proof, only_intent};
use super::activation_fence::{
    EffectBroker, EffectRequest, EffectResult, OperationFact, OperationState, Scope, TestClient,
    TestHooks, WorkClass,
};
use super::activation_fence_ipc::{Command, Envelope};
use super::activation_generic_process_tests::{
    close, durable_path, fact, identity, request, settled, Fixture, OwnedChild, Role, BOUND,
};
use super::reconciler::RecoveryConfig;
use super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialIntentDraft, InitialIntentIdentity,
    IntentState, IntentTransitionCommand, LeaseAction, LeaseOwnerId, TerminalTemplateBinding,
    TransitionActor,
};
use crate::durable_delivery::{
    AuthoritativeDeliveryRequest, AuthoritativeSink, AuthoritativeSinkPort,
    AuthoritativeSinkResult, CoordinatorConfig, DeliveryEnvelope, DeliverySubKind,
    DurableDeliveryCoordinator, ImmutableAppendPort, PushKind, TypedReceipt,
};
use crate::event::envelope::{
    news_flash_evidence_sha256, NewsFlashAuditSource, NewsFlashRemoteReceipt,
};
use crate::event::{AuditDispatcher, EventEnvelope, NewsFlashWindow, PushDeliveryEvent};
use crate::monitor::push_job::{
    raw_digest, w09_completion_policy_fixture, AudienceId, AuthorityClass, BusinessDate, ChannelId,
    CompletionOwnerId, Namespace, OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey,
    ReasonCode, RunId, SourceContractId, SubjectId, TemplateId, TemplateVersion, UnitId, UtcMicros,
};

const DATE: &str = "2026-09-07";
const NOW: i64 = 1_788_743_104_000_000;
const UNTIL: i64 = 1_788_743_400_000_000;
const N02_NOW: i64 = NOW + 1_800_000_000;
const N02_UNTIL: i64 = N02_NOW + 300_000_000;
const P01_CHANNEL: &str = "TEST_CODE_W16_PROCESS_P01_CHANNEL";
const N02_CHANNEL: &str = "TEST_CODE_W16_PROCESS_N02_CHANNEL";

fn micros(value: i64) -> UtcMicros {
    UtcMicros::try_new(value).unwrap()
}

fn recovery_times(kind: DedicatedBusinessKind) -> (i64, i64) {
    match kind {
        DedicatedBusinessKind::P01 => (NOW, UNTIL),
        DedicatedBusinessKind::N02 => (N02_NOW, N02_UNTIL),
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) enum DedicatedBusinessKind {
    P01,
    N02,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct DedicatedBusinessRole {
    kind: DedicatedBusinessKind,
    epoch: String,
    hooks: TestHooks,
}

#[derive(Default)]
struct Append {
    records: std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
}

impl ImmutableAppendPort for Append {
    fn append_exact(
        &self,
        _: &str,
        identity: &str,
        bytes: &[u8],
        _: &str,
    ) -> crate::durable_delivery::Result<String> {
        let mut records = self.records.lock().unwrap();
        if let Some(existing) = records.get(identity) {
            assert_eq!(existing, bytes);
        } else {
            records.insert(identity.to_owned(), bytes.to_vec());
        }
        Ok(format!("TEST_CODE_APPEND:{identity}"))
    }
}

struct Sink;

impl AuthoritativeSinkPort for Sink {
    fn sink_identity(&self) -> &str {
        P01_CHANNEL
    }

    fn deliver(&self, _: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        AuthoritativeSinkResult::Accepted(TypedReceipt {
            channel: P01_CHANNEL.to_owned(),
            provider: "TEST_CODE_W16_PROCESS_PROVIDER".to_owned(),
            message_id: "TEST_CODE_W16_PROCESS_MESSAGE".to_owned(),
            platform_message_id: None,
            accepted_at: Utc.timestamp_micros(NOW - 4_000_000).single().unwrap(),
            latency_ms: Some(1),
        })
    }
}

fn seed_business(fixture: &Fixture, kind: DedicatedBusinessKind) {
    let database = fixture.root.path().join("business.sqlite3");
    FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(&database)
        .unwrap();
    let (unit, family, key, owner, source, template_id) = match kind {
        DedicatedBusinessKind::P01 => (
            "MU-p01",
            "p01-business-date",
            DATE,
            "p01-business-date-once",
            "w16-process-p01-source",
            "preopen_news_hot_v1",
        ),
        DedicatedBusinessKind::N02 => (
            "MU-news-flash-aggregate",
            "news-flash-window",
            "09:30",
            "news-flash-accepted-window",
            "w16-process-n02-source",
            "news_flash_aggregated_v1",
        ),
    };
    let template = TerminalTemplateBinding::new(
        TemplateId::try_new(template_id.to_owned()).unwrap(),
        TemplateVersion::try_new(template_id.to_owned()).unwrap(),
    );
    let identity = InitialIntentIdentity::new(
        Namespace::test(RunId::try_new(fixture.test_code().to_owned()).unwrap()),
        UnitId::try_new(unit.to_owned()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse(DATE).unwrap(),
            OccurrenceFamily::try_new(family.to_owned()).unwrap(),
            OccurrenceKey::try_new(key.to_owned()).unwrap(),
        ),
        CompletionOwnerId::try_new(owner.to_owned()).unwrap(),
        SourceContractId::try_new(source.to_owned()).unwrap(),
        SubjectId::Global,
        AudienceId::try_new("test-owner".to_owned()).unwrap(),
    );
    let (recovery_now, recovery_until) = recovery_times(kind);
    let draft = InitialIntentDraft::ready_for_recovery_test(
        identity,
        format!("TEST_CODE_{kind:?}_PREPARED").into_bytes(),
        format!("TEST_CODE_{kind:?}_RENDERED").into_bytes(),
        template.sha256().clone(),
        raw_digest(format!("TEST_CODE_{kind:?}_SOURCE").as_bytes()),
        micros(recovery_now - 600_000_000),
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
                TransitionActor::try_new(format!("w16-process-{kind:?}")).unwrap(),
                ReasonCode::IntentDispatchClaimed,
                micros(recovery_now - 60_000_000),
                LeaseAction::Acquire {
                    owner: LeaseOwnerId::try_new(format!("w16-process-{kind:?}")).unwrap(),
                    until: micros(recovery_until),
                },
            )
            .unwrap(),
        )
        .unwrap();
    let snapshot = store.inspect(&intent_id).unwrap().unwrap();
    drop(store);

    match kind {
        DedicatedBusinessKind::P01 => seed_p01(fixture, &snapshot),
        DedicatedBusinessKind::N02 => seed_n02(fixture, &snapshot),
    }
}

fn seed_p01(fixture: &Fixture, snapshot: &super::IntentSnapshot) {
    let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::test(
        durable_path(fixture.test_code()),
        fixture.test_code(),
        "TEST_CODE_W16_PROCESS_SEED",
    ))
    .unwrap();
    let attested = snapshot.attested_ready_binding().unwrap();
    let envelope = DeliveryEnvelope::new(
        DATE,
        PushKind::PreopenNewsHot,
        DeliverySubKind::None,
        "GLOBAL",
        format!("p01:{DATE}"),
        attested.source_evidence_fingerprint.as_str(),
        br#"{"render_mode":"Scheduled","schema_version":"P01_SOURCE_BINDING_V1"}"#.to_vec(),
        "TEST_CODE_W16_PROCESS_SUBJECT",
        snapshot.rendered_bytes().unwrap().to_vec(),
        false,
        None,
    )
    .unwrap();
    let append = Append::default();
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
    let sinks: Vec<AuthoritativeSink> = vec![Arc::new(Sink)];
    coordinator
        .resume_deliverable(
            &envelope.decision_identity,
            &sinks,
            Utc.timestamp_micros(NOW - 4_000_000).single().unwrap(),
        )
        .unwrap();
    coordinator
        .reconcile_all_pending(
            &append,
            Utc.timestamp_micros(NOW - 3_000_000).single().unwrap(),
        )
        .unwrap();
}

fn seed_n02(fixture: &Fixture, snapshot: &super::IntentSnapshot) {
    let audit = AuditDispatcher::for_test_code(fixture.test_code()).unwrap();
    let date = chrono::NaiveDate::parse_from_str(DATE, "%Y-%m-%d").unwrap();
    let at = Utc
        .timestamp_micros(N02_NOW - 300_000_000)
        .single()
        .unwrap()
        .fixed_offset();
    let sources = vec![NewsFlashAuditSource {
        event_id: "TEST_CODE_W16_PROCESS_EVENT".to_owned(),
        provider: "TEST_CODE_W16_PROCESS_PROVIDER".to_owned(),
        source: "TEST_CODE_W16_PROCESS_SOURCE".to_owned(),
        published_at: at,
        observed_at: at,
        batch_id: "TEST_CODE_W16_PROCESS_BATCH".to_owned(),
    }];
    let evidence = news_flash_evidence_sha256(&sources);
    let attempt_event = PushDeliveryEvent::new_news_flash_attempt(
        "news_flash_aggregated_v1".to_owned(),
        "window:09:30".to_owned(),
        N02_CHANNEL.to_owned(),
        snapshot.rendered_bytes().unwrap().len(),
        date,
        "a".repeat(64),
        sources.clone(),
        evidence.clone(),
        snapshot.rendered_sha256().unwrap().as_str().to_owned(),
        1,
        at,
    );
    let attempt = EventEnvelope::from_event(
        &attempt_event,
        attempt_event.news_flash_join_sha256.clone().unwrap(),
        "TEST_CODE_W16_PROCESS_ATTEMPT".to_owned(),
        at.with_timezone(&chrono::Local),
    )
    .unwrap();
    audit.append_exact_news_flash_authority(&attempt).unwrap();
    let terminal_at = Utc
        .timestamp_micros(N02_NOW - 180_000_000)
        .single()
        .unwrap()
        .fixed_offset();
    let terminal_event = PushDeliveryEvent::new_news_flash_terminal(
        crate::event::envelope::NewsFlashTransactionStage::Accepted,
        "news_flash_aggregated_v1".to_owned(),
        "window:09:30".to_owned(),
        N02_CHANNEL.to_owned(),
        snapshot.rendered_bytes().unwrap().len(),
        3,
        date,
        "a".repeat(64),
        sources,
        evidence,
        snapshot.rendered_sha256().unwrap().as_str().to_owned(),
        1,
        at,
        attempt_event
            .news_flash_sink_attempt_identity
            .clone()
            .unwrap(),
        attempt_event
            .news_flash_sink_attempt_sha256
            .clone()
            .unwrap(),
        attempt.id,
        Some(NewsFlashRemoteReceipt {
            channel: N02_CHANNEL.to_owned(),
            provider: "TEST_CODE_W16_PROCESS_PROVIDER".to_owned(),
            message_id: "TEST_CODE_W16_PROCESS_MESSAGE".to_owned(),
            platform_message_id: "TEST_CODE_W16_PROCESS_PLATFORM".to_owned(),
            accepted_at: terminal_at,
            latency_ms: 1,
        }),
        terminal_at,
        None,
        None,
    );
    let terminal = EventEnvelope::from_event(
        &terminal_event,
        terminal_event.news_flash_join_sha256.clone().unwrap(),
        "TEST_CODE_W16_PROCESS_TERMINAL".to_owned(),
        terminal_at.with_timezone(&chrono::Local),
    )
    .unwrap();
    audit.append_exact_news_flash_authority(&terminal).unwrap();
}

fn process_clients() -> Vec<TestClient> {
    let (peer, _other) = UnixStream::pair().unwrap();
    let observed = super::activation_authorization::observe_unix_peer(&peer).unwrap();
    [false, true]
        .into_iter()
        .map(|supervisor| {
            let id = identity(supervisor);
            TestClient {
                client: id.client,
                incarnation: id.incarnation,
                credential: id.credential,
                uid: observed.uid(),
                gid: observed.gid(),
                supervisor,
            }
        })
        .collect()
}

pub(super) async fn run_broker(
    root: &Path,
    test_code: &str,
    socket: &Path,
    signal: &Path,
    role: DedicatedBusinessRole,
) {
    let database = root.join("business.sqlite3");
    let snapshot = BusinessIntentStore::open(&database)
        .unwrap()
        .inspect(&only_intent(&database))
        .unwrap()
        .unwrap();
    let (unit, owner, template_id, authority, channel) = match role.kind {
        DedicatedBusinessKind::P01 => (
            "MU-p01",
            "p01-business-date-once",
            "preopen_news_hot_v1",
            AuthorityClass::P01Dedicated,
            P01_CHANNEL,
        ),
        DedicatedBusinessKind::N02 => (
            "MU-news-flash-aggregate",
            "news-flash-accepted-window",
            "news_flash_aggregated_v1",
            AuthorityClass::N02Dedicated,
            N02_CHANNEL,
        ),
    };
    let template = TerminalTemplateBinding::new(
        TemplateId::try_new(template_id.to_owned()).unwrap(),
        TemplateVersion::try_new(template_id.to_owned()).unwrap(),
    );
    let policy = w09_completion_policy_fixture(unit, owner, vec![authority]);
    let (recovery_now, recovery_until) = recovery_times(role.kind);
    let config = RecoveryConfig::try_new(
        LeaseOwnerId::try_new(format!("w16-process-{:?}", role.kind)).unwrap(),
        TransitionActor::try_new(format!("w16-process-{:?}", role.kind)).unwrap(),
        micros(recovery_now),
        micros(recovery_until),
        10,
        5,
    )
    .unwrap();
    let scope = Scope {
        namespace: format!("Test:{test_code}"),
        unit: unit.to_owned(),
        generation: 1,
        manifest: "a".repeat(64),
        physical_owner: "TEST_CODE_OWNER".to_owned(),
        deployment: "TEST_CODE_DEPLOYMENT".to_owned(),
        incarnation: "TEST_CODE_INCARNATION".to_owned(),
    };
    let control = root.join("business-control.sqlite3");
    let broker = match role.kind {
        DedicatedBusinessKind::P01 => EffectBroker::test_p01_business_fixture(
            &control,
            scope,
            role.epoch,
            BusinessEffectFixture {
                database,
                coordinator: Arc::new(
                    DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                        durable_path(test_code),
                        test_code,
                        format!("TEST_CODE_PROCESS_OWNER_{}", std::process::id()),
                    ))
                    .unwrap(),
                ),
                snapshot,
                template,
                completion_policy: policy,
                config,
            },
            ChannelId::try_new(channel.to_owned()).unwrap(),
            process_clients(),
            role.hooks,
        ),
        DedicatedBusinessKind::N02 => EffectBroker::test_n02_business_fixture(
            &control,
            scope,
            role.epoch,
            N02BusinessEffectFixture {
                database,
                audit: Arc::new(AuditDispatcher::for_test_code(test_code).unwrap()),
                snapshot,
                template,
                completion_policy: policy,
                config,
                required_channel: ChannelId::try_new(channel.to_owned()).unwrap(),
                window: NewsFlashWindow::H0930,
            },
            process_clients(),
            role.hooks,
        ),
    }
    .unwrap();
    let mut operation = broker.test_business_request("TEST_CODE_DEDICATED_BUSINESS_RECOVERY");
    operation.client = identity(false).client;
    operation.client_incarnation = identity(false).incarnation;
    let listener = UnixListener::bind(socket).unwrap();
    let mut ready = std::os::unix::net::UnixStream::connect(signal).unwrap();
    ready
        .write_all(&serde_json::to_vec(&operation).unwrap())
        .unwrap();
    drop(ready);
    Arc::new(broker).serve(listener).await.unwrap();
}

async fn business_broker(
    fixture: &mut Fixture,
    kind: DedicatedBusinessKind,
    epoch: &str,
    hooks: TestHooks,
) -> (OwnedChild, PathBuf, EffectRequest) {
    let socket = fixture
        .root
        .path()
        .join(format!("dedicated-{kind:?}-{epoch}.sock"));
    let (child, signal) = fixture.spawn(
        &socket,
        Role::DedicatedBusinessBroker(DedicatedBusinessRole {
            kind,
            epoch: epoch.to_owned(),
            hooks,
        }),
    );
    let bytes = tokio::time::timeout(BOUND, async {
        let (stream, _) = signal.accept().await.unwrap();
        let mut bytes = Vec::new();
        stream
            .take(16 * 1024)
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        bytes
    })
    .await
    .expect("dedicated broker ready");
    (child, socket, serde_json::from_slice(&bytes).unwrap())
}

async fn pause(listener: &UnixListener) -> UnixStream {
    tokio::time::timeout(BOUND, async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut marker = [0_u8];
        stream.read_exact(&mut marker).await.unwrap();
        assert_eq!(marker, *b"P");
        stream
    })
    .await
    .expect("second dedicated query reached")
}

async fn exact_replay(socket: &Path, operation: &EffectRequest, expected: &OperationFact) {
    for command in [
        Command::QueryOperation {
            request: operation.clone(),
            wait: true,
        },
        Command::ExecuteCurrent {
            request: operation.clone(),
            wait: true,
        },
    ] {
        assert_eq!(&fact(request(socket, command).await), expected);
    }
}

#[derive(Debug, Eq, PartialEq)]
enum SourceProjection {
    P01 { attempts: i64, results: i64 },
    N02(Vec<u8>),
}

fn source_projection(fixture: &Fixture, kind: DedicatedBusinessKind) -> SourceProjection {
    match kind {
        DedicatedBusinessKind::P01 => {
            let durable = durable_path(fixture.test_code());
            SourceProjection::P01 {
                attempts: fixture.count(&durable, "delivery_attempts"),
                results: fixture.count(&durable, "sink_results"),
            }
        }
        DedicatedBusinessKind::N02 => SourceProjection::N02(
            std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("data/test")
                    .join(fixture.test_code())
                    .join("event_audit/2026.jsonl"),
            )
            .unwrap(),
        ),
    }
}

#[tokio::test]
async fn dedicated_process_requester_death_worker_completion_replay_and_quiesce() {
    for kind in [DedicatedBusinessKind::P01, DedicatedBusinessKind::N02] {
        let mut fixture = Fixture::new();
        seed_business(&fixture, kind);
        let source_before = source_projection(&fixture, kind);
        let pause_path = fixture
            .root
            .path()
            .join(format!("dedicated-{kind:?}-pause.sock"));
        let listener = UnixListener::bind(&pause_path).unwrap();
        let (mut broker, socket, operation) = business_broker(
            &mut fixture,
            kind,
            "first",
            TestHooks {
                business_requery_pause_socket: Some(pause_path),
                ..TestHooks::default()
            },
        )
        .await;
        assert!(
            close(&socket, &operation, WorkClass::NewWork)
                .await
                .recovery_open
        );
        let (mut requester, _lifetime) = fixture.spawn(
            &socket,
            Role::Requester(Envelope {
                identity: identity(false),
                command: Command::ExecuteCurrent {
                    request: operation.clone(),
                    wait: true,
                },
            }),
        );
        let mut paused = pause(&listener).await;
        requester.stop();
        let status = close(&socket, &operation, WorkClass::Recovery).await;
        assert!(!status.drained && status.unresolved == 1);
        paused.write_all(b"G").await.unwrap();
        let done = settled(&socket, &operation).await;
        assert_eq!(done.state, OperationState::Succeeded);
        assert!(matches!(
            done.result,
            Some(EffectResult::BusinessRecovery(_))
        ));
        assert_completion_proof(&fixture, &operation, &done);
        exact_replay(&socket, &operation, &done).await;
        assert!(
            close(&socket, &operation, WorkClass::Recovery)
                .await
                .drained
        );
        broker.stop();
        let (_replacement, socket, _) =
            business_broker(&mut fixture, kind, "second", TestHooks::default()).await;
        exact_replay(&socket, &operation, &done).await;
        assert_completion_proof(&fixture, &operation, &done);
        assert_eq!(source_projection(&fixture, kind), source_before);
    }
}

#[tokio::test]
async fn p01_process_broker_death_after_qualification_cannot_fabricate_success_or_resend() {
    let mut fixture = Fixture::new();
    seed_business(&fixture, DedicatedBusinessKind::P01);
    let durable = durable_path(fixture.test_code());
    let attempts = fixture.count(&durable, "delivery_attempts");
    let results = fixture.count(&durable, "sink_results");
    let pause_path = fixture.root.path().join("dedicated-death-pause.sock");
    let listener = UnixListener::bind(&pause_path).unwrap();
    let (mut broker, socket, operation) = business_broker(
        &mut fixture,
        DedicatedBusinessKind::P01,
        "first",
        TestHooks {
            business_requery_pause_socket: Some(pause_path),
            ..TestHooks::default()
        },
    )
    .await;
    request(
        &socket,
        Command::ExecuteCurrent {
            request: operation.clone(),
            wait: false,
        },
    )
    .await;
    let _paused = pause(&listener).await;
    broker.stop();
    let (_replacement, socket, next) = business_broker(
        &mut fixture,
        DedicatedBusinessKind::P01,
        "second",
        TestHooks::default(),
    )
    .await;
    let observed = fact(
        request(
            &socket,
            Command::QueryOperation {
                request: operation.clone(),
                wait: false,
            },
        )
        .await,
    );
    assert_eq!(observed.state, OperationState::Unresolved);
    exact_replay(&socket, &operation, &observed).await;
    assert!(!close(&socket, &next, WorkClass::Recovery).await.drained);
    assert_eq!(fixture.count(&durable, "delivery_attempts"), attempts);
    assert_eq!(fixture.count(&durable, "sink_results"), results);
    assert_eq!(
        BusinessIntentStore::open(&fixture.root.path().join("business.sqlite3"))
            .unwrap()
            .inspect(&only_intent(&fixture.root.path().join("business.sqlite3")))
            .unwrap()
            .unwrap()
            .state(),
        IntentState::AwaitingFinalizer
    );
}
