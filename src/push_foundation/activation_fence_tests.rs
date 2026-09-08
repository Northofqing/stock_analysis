//! Same-process Unix and durable-failure tests. Real Child ownership tests live in
//! activation_fence_process_tests; these tests do not claim process-death evidence.
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

use crate::monitor::push_job::{
    AudienceId, BusinessDate, CompletionOwnerId, Namespace, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, RunId, Sha256Digest, SourceContractId, SubjectId,
    UnitId, UtcMicros,
};

use super::activation_fence::*;
use super::activation_fence_ipc::*;
use super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialIntentDraft, InitialIntentIdentity,
    IntentState,
};

pub(super) fn fixture_scope() -> Scope {
    Scope {
        namespace: "Test:activation-fence".into(),
        unit: "w16-initial".into(),
        generation: 11,
        manifest: "a".repeat(64),
        physical_owner: "generic-owner".into(),
        deployment: "fixture-deployment".into(),
        incarnation: "deployment-one".into(),
    }
}

pub(super) fn fixture_draft() -> InitialIntentDraft {
    InitialIntentDraft::no_data(
        InitialIntentIdentity::new(
            Namespace::test(RunId::try_new("activation-fence".into()).unwrap()),
            UnitId::try_new("w16-initial".into()).unwrap(),
            OccurrenceIdentityMaterial::new(
                BusinessDate::parse("2026-09-08").unwrap(),
                OccurrenceFamily::try_new("fixture-occurrence".into()).unwrap(),
                OccurrenceKey::try_new("one".into()).unwrap(),
            ),
            CompletionOwnerId::try_new("fixture-owner".into()).unwrap(),
            SourceContractId::try_new("fixture-source".into()).unwrap(),
            SubjectId::Global,
            AudienceId::try_new("fixture-audience".into()).unwrap(),
        ),
        Sha256Digest::parse("fixture", &"b".repeat(64)).unwrap(),
        Sha256Digest::parse("fixture", &"c".repeat(64)).unwrap(),
        Sha256Digest::parse("fixture", &"d".repeat(64)).unwrap(),
        UtcMicros::try_new(123).unwrap(),
    )
}

pub(super) fn fixture_clients(uid: u32, gid: u32) -> Vec<TestClient> {
    vec![
        TestClient {
            client: "producer".into(),
            incarnation: "client-one".into(),
            credential: "producer-fixture-secret".into(),
            uid,
            gid,
            supervisor: false,
        },
        TestClient {
            client: "supervisor".into(),
            incarnation: "supervisor-one".into(),
            credential: "supervisor-fixture-secret".into(),
            uid,
            gid,
            supervisor: true,
        },
    ]
}

pub(super) fn producer_identity() -> ClientIdentity {
    ClientIdentity {
        client: "producer".into(),
        incarnation: "client-one".into(),
        credential: "producer-fixture-secret".into(),
    }
}

pub(super) fn supervisor_identity() -> ClientIdentity {
    ClientIdentity {
        client: "supervisor".into(),
        incarnation: "supervisor-one".into(),
        credential: "supervisor-fixture-secret".into(),
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    database: PathBuf,
    control: PathBuf,
    socket: PathBuf,
    broker: Arc<EffectBroker>,
    server: tokio::task::JoinHandle<Result<(), FenceError>>,
}

impl Fixture {
    fn start(hooks: TestHooks) -> Self {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("business.sqlite3");
        let control = root.path().join("control.sqlite3");
        let socket = root.path().join("broker.sock");
        FoundationSchemaMigration::bundled()
            .unwrap()
            .apply_to(&database)
            .unwrap();
        let meta = std::fs::metadata(root.path()).unwrap();
        let broker = Arc::new(
            EffectBroker::test_fixture(
                &database,
                &control,
                fixture_scope(),
                "epoch-one".into(),
                fixture_draft(),
                fixture_clients(meta.uid(), meta.gid()),
                hooks,
            )
            .unwrap(),
        );
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(Arc::clone(&broker).serve(listener));
        Self {
            _root: root,
            database,
            control,
            socket,
            broker,
            server,
        }
    }
    async fn send(&self, command: Command) -> Reply {
        let identity = if matches!(&command, Command::Quiesce { .. }) {
            supervisor_identity()
        } else {
            producer_identity()
        };
        EffectClient::request(&self.socket, &Envelope { identity, command })
            .await
            .unwrap()
    }
    async fn execute(&self, request: EffectRequest) -> Reply {
        self.send(Command::ExecuteCurrent {
            request,
            wait: true,
        })
        .await
    }
    async fn close(&self, class: WorkClass) -> ScopeStatus {
        match self
            .send(Command::Quiesce {
                scope: fixture_scope(),
                broker_epoch: "epoch-one".into(),
                class,
            })
            .await
        {
            Reply::Scope(status) => status,
            other => panic!("expected scope, got {other:?}"),
        }
    }
    fn business_count(&self) -> u64 {
        BusinessIntentStore::open(&self.database)
            .unwrap()
            .intent_count()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

#[tokio::test]
async fn exact_initial_commit_and_replay_survive_closed_scope() {
    let f = Fixture::start(TestHooks::default());
    let request = f.broker.test_request("exact-once");
    let first = f.execute(request.clone()).await;
    let Reply::Operation(Some(fact)) = &first else {
        panic!("{first:?}")
    };
    assert_eq!(fact.state, OperationState::Succeeded);
    let result = fact.result.as_ref().unwrap();
    let store = BusinessIntentStore::open(&f.database).unwrap();
    let draft = fixture_draft();
    let snapshot = store.inspect(draft.intent_id()).unwrap().unwrap();
    assert_eq!(snapshot.state(), IntentState::NoData);
    assert_eq!(snapshot.namespace(), fixture_scope().namespace);
    assert_eq!(snapshot.version(), 0);
    assert_eq!(snapshot.lease_generation(), 0);
    assert_eq!(result.intent_id, draft.intent_id().as_str());
    assert_eq!(
        result.initial_intent_sha256,
        store.inspect_activation_initial(&draft).unwrap()
    );
    assert_eq!(result.effect_sha256, request.effect_sha256);
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 0);
    let first_close = f.close(WorkClass::NewWork).await;
    assert!(!first_close.new_work_open && first_close.recovery_open && !first_close.drained);
    let all_close = f.close(WorkClass::Recovery).await;
    assert!(all_close.drained);
    assert_eq!(f.execute(request.clone()).await, first);
    assert_eq!(f.business_count(), 1);
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 0);
    let mut replacement = request;
    replacement.operation_id = "replacement".into();
    assert_eq!(
        f.execute(replacement).await,
        Reply::Refused(FenceError::Closed)
    );
}

#[test]
fn initial_adapter_readback_preserves_original_zero_version_contract() {
    use crate::monitor::push_job::raw_digest;
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("business.sqlite3");
    FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(&database)
        .unwrap();
    let draft = fixture_draft();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let outcome = store.record_initial(&draft).unwrap();
    assert!(matches!(outcome, super::InitialIntentOutcome::Inserted(_)));
    let snapshot = outcome.snapshot();
    assert_eq!(snapshot.version(), 0);
    assert_eq!(snapshot.lease_generation(), 0);
    assert!(store
        .inspect_transition_chain(draft.intent_id())
        .unwrap()
        .is_empty());
    assert_eq!(store.intent_count().unwrap(), 1);
    let initial_digest = raw_digest(&draft.activation_binding().2);
    assert_eq!(
        store.inspect_activation_initial(&draft).unwrap(),
        initial_digest.as_str()
    );
    let replay = store.record_initial(&draft).unwrap();
    assert!(matches!(
        replay,
        super::InitialIntentOutcome::ExistingIdentical(_)
    ));
    assert_eq!(replay.snapshot(), snapshot);
    assert_eq!(store.intent_count().unwrap(), 1);
    assert_eq!(store.transition_count(draft.intent_id()).unwrap(), 0);
    assert_eq!(
        store.inspect_activation_initial(&draft).unwrap(),
        initial_digest.as_str()
    );
}

#[tokio::test]
async fn exact_binding_mutations_and_unknown_clients_never_start_business_work() {
    let f = Fixture::start(TestHooks::default());
    let original = f.broker.test_request("binding-matrix");
    let mut requests = Vec::new();
    macro_rules! changed {
        ($field:ident, $value:expr) => {{
            let mut r = original.clone();
            r.$field = $value;
            requests.push(r);
        }};
        (scope.$field:ident, $value:expr) => {{
            let mut r = original.clone();
            r.scope.$field = $value;
            requests.push(r);
        }};
    }
    changed!(scope.namespace, "Production".into());
    changed!(scope.unit, "other-unit".into());
    changed!(scope.generation, 12);
    changed!(scope.manifest, "e".repeat(64));
    changed!(scope.physical_owner, "other-owner".into());
    changed!(scope.deployment, "other-deployment".into());
    changed!(scope.incarnation, "other-incarnation".into());
    changed!(broker_epoch, "old-epoch".into());
    changed!(client, "unregistered".into());
    changed!(client_incarnation, "unregistered-incarnation".into());
    changed!(actor, "Dispatcher".into());
    changed!(action, "RunShell".into());
    changed!(work_class, WorkClass::Recovery);
    changed!(effect_id, "unknown-effect".into());
    changed!(effect_sha256, "0".repeat(64));
    for request in &requests {
        assert_ne!(request.canonical_bytes(), original.canonical_bytes());
        assert_ne!(request.digest(), original.digest());
        assert!(
            matches!(f.execute(request.clone()).await, Reply::Refused(_)),
            "{request:?}"
        );
        assert_eq!(f.business_count(), 0);
    }
    for identity in [
        ClientIdentity {
            client: "unregistered".into(),
            ..producer_identity()
        },
        ClientIdentity {
            incarnation: "other".into(),
            ..producer_identity()
        },
        ClientIdentity {
            credential: "wrong".into(),
            ..producer_identity()
        },
    ] {
        let mut request = original.clone();
        request.client = identity.client.clone();
        request.client_incarnation = identity.incarnation.clone();
        assert_eq!(
            EffectClient::request(
                &f.socket,
                &Envelope {
                    identity,
                    command: Command::ExecuteCurrent {
                        request,
                        wait: false
                    }
                }
            )
            .await
            .unwrap(),
            Reply::Refused(FenceError::Unauthorized)
        );
    }
    assert_eq!(f.business_count(), 0);
    assert!(matches!(
        f.execute(original.clone()).await,
        Reply::Operation(Some(OperationFact {
            state: OperationState::Succeeded,
            ..
        }))
    ));
    // Once registered, the ID is permanently bound, even for effect/tuple changes.
    for request in requests.into_iter().filter(|r| {
        r.client == original.client && r.client_incarnation == original.client_incarnation
    }) {
        assert_eq!(
            f.execute(request).await,
            Reply::Refused(FenceError::Conflict)
        );
    }
    assert_eq!(f.business_count(), 1);
}

#[tokio::test]
async fn scope_close_is_exact_and_producer_cannot_quiesce() {
    let f = Fixture::start(TestHooks::default());
    let mut scope = fixture_scope();
    scope.unit = "another-unit".into();
    assert_eq!(
        f.send(Command::Quiesce {
            scope,
            broker_epoch: "epoch-one".into(),
            class: WorkClass::NewWork
        })
        .await,
        Reply::Refused(FenceError::Stale)
    );
    assert_eq!(
        f.send(Command::Quiesce {
            scope: fixture_scope(),
            broker_epoch: "old".into(),
            class: WorkClass::NewWork
        })
        .await,
        Reply::Refused(FenceError::Stale)
    );
    assert_eq!(
        EffectClient::request(
            &f.socket,
            &Envelope {
                identity: producer_identity(),
                command: Command::Quiesce {
                    scope: fixture_scope(),
                    broker_epoch: "epoch-one".into(),
                    class: WorkClass::NewWork
                }
            }
        )
        .await
        .unwrap(),
        Reply::Refused(FenceError::Unauthorized)
    );
    let status = f.close(WorkClass::Recovery).await;
    assert!(status.new_work_open && !status.recovery_open && !status.drained);
    assert!(matches!(
        f.execute(f.broker.test_request("still-new")).await,
        Reply::Operation(Some(OperationFact {
            state: OperationState::Succeeded,
            ..
        }))
    ));
    assert!(f.close(WorkClass::NewWork).await.drained);
}

#[tokio::test]
async fn registration_failure_has_zero_effect_and_keeps_gate_blocked() {
    let f = Fixture::start(TestHooks {
        registration_failure: true,
        ..TestHooks::default()
    });
    let request = f.broker.test_request("registration-fault");
    assert_eq!(
        f.execute(request.clone()).await,
        Reply::Refused(FenceError::Store)
    );
    assert_eq!(f.business_count(), 0);
    assert_eq!(
        f.send(Command::QueryOperation {
            request: request.clone(),
            wait: false
        })
        .await,
        Reply::Operation(None)
    );
    let status = f.close(WorkClass::NewWork).await;
    assert!(!status.new_work_open && !status.recovery_open && !status.drained);
    assert_eq!(f.execute(request).await, Reply::Refused(FenceError::Closed));
}

#[tokio::test]
async fn failures_after_admission_preserve_durable_unresolved_and_never_replay() {
    for (hooks, expected_intents) in [
        (
            TestHooks {
                effect_started_failure: true,
                ..TestHooks::default()
            },
            0,
        ),
        (
            TestHooks {
                business_commit_ack_lost: true,
                ..TestHooks::default()
            },
            1,
        ),
        (
            TestHooks {
                result_confirmation_lost: true,
                ..TestHooks::default()
            },
            1,
        ),
    ] {
        let f = Fixture::start(hooks);
        let request = f.broker.test_request("uncertain");
        let response = f.execute(request.clone()).await;
        assert!(
            matches!(
                response,
                Reply::Operation(Some(OperationFact {
                    state: OperationState::Unresolved,
                    result: None,
                    ..
                }))
            ),
            "{response:?}"
        );
        assert_eq!(f.business_count(), expected_intents);
        assert_eq!(f.execute(request.clone()).await, response);
        let status = f.close(WorkClass::NewWork).await;
        assert!(!status.drained);
        assert_eq!(status.unresolved, 1);
        assert!(!f.close(WorkClass::Recovery).await.drained);
        assert_eq!(f.business_count(), expected_intents);
        if expected_intents == 1 {
            let store = BusinessIntentStore::open(&f.database).unwrap();
            assert_eq!(
                store.transition_count(fixture_draft().intent_id()).unwrap(),
                0
            );
            assert!(store.inspect_activation_initial(&fixture_draft()).is_ok());
        }
        // All three paths have a persisted candidate; an unconfirmed candidate is not success.
        let connection = rusqlite::Connection::open(&f.control).unwrap();
        let candidate:String=connection.query_row("SELECT candidate_result_json FROM effect_operations WHERE operation_id='uncertain'",[],|r|r.get(0)).unwrap();
        assert!(!candidate.is_empty());
    }
}

#[tokio::test]
async fn disconnect_and_timeout_do_not_drain_paused_worker() {
    let root = tempfile::tempdir().unwrap();
    let pause_path = root.path().join("pause.sock");
    let pause = UnixListener::bind(&pause_path).unwrap();
    let f = Fixture::start(TestHooks {
        pause_socket: Some(pause_path),
        ..TestHooks::default()
    });
    let request = f.broker.test_request("paused");
    let mut connection = tokio::net::UnixStream::connect(&f.socket).await.unwrap();
    let bytes = serde_json::to_vec(&Envelope {
        identity: producer_identity(),
        command: Command::ExecuteCurrent {
            request: request.clone(),
            wait: true,
        },
    })
    .unwrap();
    connection.write_u32(bytes.len() as u32).await.unwrap();
    connection.write_all(&bytes).await.unwrap();
    let (mut worker, _) = tokio::time::timeout(std::time::Duration::from_secs(5), pause.accept())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(worker.read_u8().await.unwrap(), b'P');
    drop(connection);
    let status = f.close(WorkClass::NewWork).await;
    assert!(
        !status.new_work_open && status.recovery_open && !status.drained && status.unresolved == 1
    );
    assert_eq!(f.business_count(), 0);
    // This waits on a bounded notification, not a guessed sleep or EOF-as-completion.
    assert!(matches!(
        f.send(Command::QueryOperation {
            request: request.clone(),
            wait: true
        })
        .await,
        Reply::Operation(Some(OperationFact {
            state: OperationState::Running,
            ..
        }))
    ));
    assert!(!f.close(WorkClass::Recovery).await.drained);
    worker.write_all(b"G").await.unwrap();
    assert!(matches!(
        f.send(Command::QueryOperation {
            request: request.clone(),
            wait: true
        })
        .await,
        Reply::Operation(Some(OperationFact {
            state: OperationState::Succeeded,
            ..
        }))
    ));
    assert!(f.close(WorkClass::Recovery).await.drained);
    assert_eq!(f.business_count(), 1);
    assert!(matches!(
        f.execute(request).await,
        Reply::Operation(Some(OperationFact {
            state: OperationState::Succeeded,
            ..
        }))
    ));
}

#[tokio::test]
async fn protocol_has_bounded_frames_and_no_raw_effect_payload() {
    let f = Fixture::start(TestHooks::default());
    let mut connection = tokio::net::UnixStream::connect(&f.socket).await.unwrap();
    connection.write_u32(16 * 1024 + 1).await.unwrap();
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            connection.read(&mut byte)
        )
        .await
        .unwrap()
        .unwrap(),
        0
    );
    let envelope = Envelope {
        identity: producer_identity(),
        command: Command::ExecuteCurrent {
            request: f.broker.test_request("raw"),
            wait: false,
        },
    };
    let mut value = serde_json::to_value(envelope).unwrap();
    value["command"]["ExecuteCurrent"]["request"]["database_path"] =
        serde_json::json!("/unauthorized");
    assert!(serde_json::from_value::<Envelope>(value).is_err());
    assert_eq!(f.business_count(), 0);
}

#[test]
fn production_and_test_draft_scope_mismatch_are_refused_before_control_creation() {
    assert!(matches!(
        EffectBroker::production(),
        Err(FenceError::ProductionRefused)
    ));
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("business.sqlite3");
    FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(&database)
        .unwrap();
    let control = root.path().join("control.sqlite3");
    let mut scope = fixture_scope();
    scope.namespace = "Production".into();
    assert!(matches!(
        EffectBroker::test_fixture(
            &database,
            &control,
            scope,
            "epoch".into(),
            fixture_draft(),
            vec![],
            TestHooks::default()
        ),
        Err(FenceError::ProductionRefused)
    ));
    assert!(!control.exists());
    let mut scope = fixture_scope();
    scope.unit = "other-unit".into();
    assert!(matches!(
        EffectBroker::test_fixture(
            &database,
            &control,
            scope,
            "epoch".into(),
            fixture_draft(),
            vec![],
            TestHooks::default()
        ),
        Err(FenceError::EffectMismatch)
    ));
    assert!(!control.exists());
}

#[test]
fn control_store_bootstrap_keeps_sqlite_transactions_and_separate_owner_lock() {
    use super::activation_fence_store::OperationStore;
    use fs2::FileExt;
    let root = tempfile::tempdir().unwrap();
    let control = root.path().join("control.sqlite3");
    let lock = root.path().join("control.sqlite3.broker.lock");
    let store = OperationStore::open(&control, "bootstrap")
        .expect("SQLite bootstrap with independent broker ownership lock");
    let database_meta = std::fs::metadata(&control).unwrap();
    let lock_meta = std::fs::metadata(&lock).unwrap();
    assert_ne!(database_meta.ino(), lock_meta.ino());
    assert_eq!(database_meta.nlink(), 1);
    assert_eq!(lock_meta.nlink(), 1);
    let bytes = std::fs::read(&lock).unwrap();
    let binding: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(binding["format"], "ActivationEffectStoreLock/v1");
    assert_eq!(
        binding["canonical_database_path"],
        std::fs::canonicalize(&control).unwrap().to_str().unwrap()
    );
    assert_eq!(binding["database_device"], database_meta.dev());
    assert_eq!(binding["database_inode"], database_meta.ino());
    let contender = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .unwrap();
    assert!(
        contender.try_lock_exclusive().is_err(),
        "broker retains the actual owner FD"
    );
    // Real SQLite reads and transactions still work while the broker's own lock is held.
    let connection = rusqlite::Connection::open(&control).unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "delete");
    connection
        .execute_batch("BEGIN IMMEDIATE; SELECT count(*) FROM effect_broker_epochs; ROLLBACK;")
        .unwrap();
    drop(connection);
    drop(store);
    contender
        .try_lock_exclusive()
        .expect("only broker lifetime releases its owner lock");
    FileExt::unlock(&contender).unwrap();
    let reopened = OperationStore::open(&control, "after-bootstrap").unwrap();
    assert!(!reopened.fresh);
    assert_eq!(std::fs::read(&lock).unwrap(), bytes);
    assert_eq!(std::fs::metadata(&lock).unwrap().ino(), lock_meta.ino());
}

#[test]
fn control_store_aliases_cannot_bypass_stable_owner_lock() {
    use super::activation_fence_store::OperationStore;
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let control = root.path().join("control.sqlite3");
    let store = OperationStore::open(&control, "owner").unwrap();
    let parent_alias = root.path().join("parent-alias");
    symlink(root.path(), &parent_alias).unwrap();
    assert!(matches!(
        OperationStore::open(&parent_alias.join("control.sqlite3"), "alias"),
        Err(FenceError::AlreadyOwned)
    ));
    drop(store);
    let database_alias = root.path().join("hardlinked.sqlite3");
    std::fs::hard_link(&control, &database_alias).unwrap();
    assert!(matches!(
        OperationStore::open(&database_alias, "hardlink"),
        Err(FenceError::Store)
    ));
    assert!(matches!(
        OperationStore::open(&control, "original-with-hardlink"),
        Err(FenceError::Store)
    ));
    std::fs::remove_file(&database_alias).unwrap();
    let symlinked_database = root.path().join("symlinked.sqlite3");
    symlink(&control, &symlinked_database).unwrap();
    assert!(matches!(
        OperationStore::open(&symlinked_database, "symlink"),
        Err(FenceError::Store)
    ));
    let lock = root.path().join("control.sqlite3.broker.lock");
    let lock_alias = root.path().join("hardlinked-owner");
    std::fs::hard_link(&lock, &lock_alias).unwrap();
    assert!(matches!(
        OperationStore::open(&control, "hardlinked-lock"),
        Err(FenceError::Store)
    ));
    std::fs::remove_file(&lock_alias).unwrap();
    let other_control = root.path().join("other.sqlite3");
    symlink(&lock, root.path().join("other.sqlite3.broker.lock")).unwrap();
    assert!(matches!(
        OperationStore::open(&other_control, "symlinked-lock"),
        Err(FenceError::Store)
    ));
    assert!(!other_control.exists());
    let reopened = OperationStore::open(&control, "unchanged-owner-identity").unwrap();
    assert!(!reopened.fresh);
}

#[test]
fn control_store_rejects_copied_renamed_and_reused_epoch_identities() {
    use super::activation_fence_store::OperationStore;
    let root = tempfile::tempdir().unwrap();
    let control = root.path().join("control.sqlite3");
    let store = OperationStore::open(&control, "epoch-one").unwrap();
    assert!(store.fresh);
    assert!(matches!(
        OperationStore::open(&control, "second-broker"),
        Err(FenceError::AlreadyOwned)
    ));
    drop(store);
    assert!(matches!(
        OperationStore::open(&control, "epoch-one"),
        Err(FenceError::Stale)
    ));
    let copied = root.path().join("copied.sqlite3");
    std::fs::copy(&control, &copied).unwrap();
    assert!(matches!(
        OperationStore::open(&copied, "copy-epoch"),
        Err(FenceError::Store)
    ));
    let renamed = root.path().join("renamed.sqlite3");
    std::fs::rename(&control, &renamed).unwrap();
    assert!(matches!(
        OperationStore::open(&renamed, "rename-epoch"),
        Err(FenceError::Store)
    ));
    std::fs::rename(&renamed, &control).unwrap();
    let reopened = OperationStore::open(&control, "epoch-two").unwrap();
    assert!(!reopened.fresh);
}
