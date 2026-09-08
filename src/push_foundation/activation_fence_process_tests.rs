//! Real broker/client process tests. Every child is the current isolated libtest executable.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use super::activation_fence::{
    EffectBroker, EffectRequest, FenceError, OperationFact, OperationState, Scope, ScopeStatus,
    TestClient, TestHooks, WorkClass,
};
use super::activation_fence_ipc::{ClientIdentity, Command, EffectClient, Envelope, Reply};
use super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialIntentDraft, InitialIntentIdentity,
};
use crate::monitor::push_job::{
    AudienceId, BusinessDate, CompletionOwnerId, Namespace, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, RunId, Sha256Digest, SourceContractId, SubjectId,
    UnitId, UtcMicros,
};

const BOUND: Duration = Duration::from_secs(8);
const HELPER: &str = "push_foundation::activation_fence_process_tests::w16_effect_process_child";

#[derive(Clone, Serialize, Deserialize)]
enum Role {
    Broker {
        scope: Scope,
        epoch: String,
        hooks: TestHooks,
    },
    Client(Envelope),
    HoldRequest(Envelope),
}

#[derive(Serialize, Deserialize)]
struct ChildInput {
    root: PathBuf,
    database: PathBuf,
    control: PathBuf,
    socket: PathBuf,
    signal: PathBuf,
    role: Role,
}

#[derive(Debug, Serialize, Deserialize)]
enum ChildSignal {
    Listening(EffectRequest),
    Answer(Reply),
}

struct OwnedChild(Child);

impl OwnedChild {
    fn stop(&mut self) {
        if self
            .0
            .try_wait()
            .expect("TEST_CODE inspect owned child")
            .is_none()
        {
            self.0.kill().expect("TEST_CODE kill exact owned Child");
        }
        let _ = self.0.wait().expect("TEST_CODE reap exact owned Child");
    }

    async fn assert_success(&mut self) {
        tokio::time::timeout(BOUND, async {
            loop {
                if let Some(status) = self.0.try_wait().expect("TEST_CODE observe owned Child") {
                    assert!(status.success(), "TEST_CODE child exited: {status}");
                    return;
                }
                // Poll only an actual process handle; business ordering uses socket handshakes.
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TEST_CODE bounded child completion");
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn draft() -> InitialIntentDraft {
    let identity = InitialIntentIdentity::new(
        Namespace::test(RunId::try_new("TEST_CODE_FENCE_PROCESS".into()).unwrap()),
        UnitId::try_new("MU-auction".into()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-07").unwrap(),
            OccurrenceFamily::try_new("auction-session".into()).unwrap(),
            OccurrenceKey::try_new("main".into()).unwrap(),
        ),
        CompletionOwnerId::try_new("owner-auction".into()).unwrap(),
        SourceContractId::try_new("auction-source".into()).unwrap(),
        SubjectId::entity("TEST_CODE_SUBJECT".into()).unwrap(),
        AudienceId::try_new("TEST_CODE_AUDIENCE".into()).unwrap(),
    );
    InitialIntentDraft::no_data(
        identity,
        Sha256Digest::from_bytes([1; 32]),
        Sha256Digest::from_bytes([2; 32]),
        Sha256Digest::from_bytes([3; 32]),
        UtcMicros::try_new(1_788_743_100_000_000).unwrap(),
    )
}

fn scope(generation: u64) -> Scope {
    Scope {
        namespace: "Test:TEST_CODE_FENCE_PROCESS".into(),
        unit: "MU-auction".into(),
        generation,
        manifest: "a".repeat(64),
        physical_owner: "TEST_CODE_OWNER".into(),
        deployment: "TEST_CODE_DEPLOYMENT".into(),
        incarnation: format!("TEST_CODE_OWNER_{generation}"),
    }
}

fn identity(supervisor: bool) -> ClientIdentity {
    ClientIdentity {
        client: if supervisor { "supervisor" } else { "producer" }.into(),
        incarnation: if supervisor {
            "supervisor-one"
        } else {
            "client-one"
        }
        .into(),
        credential: if supervisor {
            "TEST_CODE_SUPERVISOR_CREDENTIAL"
        } else {
            "TEST_CODE_CLIENT_CREDENTIAL"
        }
        .into(),
    }
}

struct Fixture {
    root: tempfile::TempDir,
    database: PathBuf,
    control: PathBuf,
    sequence: usize,
}

impl Fixture {
    fn new() -> Self {
        // Short isolated paths also fit the macOS Unix-domain socket path bound.
        let root = tempfile::Builder::new()
            .prefix("TEST_CODE_FENCE_")
            .tempdir_in("/private/tmp")
            .unwrap();
        let database = root.path().join("business.sqlite3");
        FoundationSchemaMigration::bundled()
            .unwrap()
            .apply_to(&database)
            .unwrap();
        let control = root.path().join("control.sqlite3");
        Self {
            root,
            database,
            control,
            sequence: 0,
        }
    }

    fn spawn(&mut self, socket: &Path, role: Role) -> (OwnedChild, UnixListener) {
        self.sequence += 1;
        let signal = self
            .root
            .path()
            .join(format!("signal-{}.sock", self.sequence));
        let listener = UnixListener::bind(&signal).unwrap();
        let input = ChildInput {
            root: self.root.path().to_owned(),
            database: self.database.clone(),
            control: self.control.clone(),
            socket: socket.to_owned(),
            signal,
            role,
        };
        let child = ProcessCommand::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", HELPER, "--nocapture"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("TEST_CODE spawn exact libtest helper");
        let mut owned = OwnedChild(child);
        {
            let mut stdin = owned.0.stdin.take().unwrap();
            stdin
                .write_all(&serde_json::to_vec(&input).unwrap())
                .unwrap();
        }
        (owned, listener)
    }

    async fn broker(
        &mut self,
        name: &str,
        generation: u64,
        hooks: TestHooks,
    ) -> (OwnedChild, PathBuf, EffectRequest) {
        let socket = self.root.path().join(format!("{name}.sock"));
        let (child, signal) = self.spawn(
            &socket,
            Role::Broker {
                scope: scope(generation),
                epoch: name.into(),
                hooks,
            },
        );
        let reply = receive_signal(&signal).await;
        let ChildSignal::Listening(request) = reply else {
            panic!("TEST_CODE expected a successfully bound broker, received {reply:?}")
        };
        (child, socket, request)
    }

    async fn child_request(&mut self, socket: &Path, command: Command, supervisor: bool) -> Reply {
        let (mut child, signal) = self.spawn(
            socket,
            Role::Client(Envelope {
                identity: identity(supervisor),
                command,
            }),
        );
        let ChildSignal::Answer(reply) = receive_signal(&signal).await else {
            panic!("TEST_CODE expected client response")
        };
        child.assert_success().await;
        reply
    }

    fn count(&self, table: &str) -> i64 {
        assert!(matches!(table, "push_intents" | "push_intent_transitions"));
        let connection = rusqlite::Connection::open_with_flags(
            &self.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn assert_one_exact_intent(&self) -> String {
        assert_eq!(self.count("push_intents"), 1);
        // Initial insertion is version zero; transitions begin only with a later CAS.
        assert_eq!(self.count("push_intent_transitions"), 0);
        let store = BusinessIntentStore::open(&self.database).unwrap();
        let draft = draft();
        let snapshot = store.inspect(draft.intent_id()).unwrap().unwrap();
        assert_eq!(snapshot.version(), 0);
        assert_eq!(snapshot.lease_generation(), 0);
        assert_eq!(snapshot.state().as_str(), "NoData");
        assert!(store
            .inspect_transition_chain(draft.intent_id())
            .unwrap()
            .is_empty());
        store.inspect_activation_initial(&draft).unwrap()
    }
}

async fn receive_signal(listener: &UnixListener) -> ChildSignal {
    tokio::time::timeout(BOUND, async {
        let (stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        stream
            .take(16 * 1024)
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).expect("TEST_CODE structured child signal")
    })
    .await
    .expect("TEST_CODE child handshake deadline")
}

async fn paused(listener: &UnixListener) -> UnixStream {
    tokio::time::timeout(BOUND, async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut marker = [0];
        stream.read_exact(&mut marker).await.unwrap();
        assert_eq!(marker, *b"P");
        stream
    })
    .await
    .expect("TEST_CODE actual effect pause handshake")
}

fn operation(reply: Reply) -> OperationFact {
    match reply {
        Reply::Operation(Some(fact)) => fact,
        other => panic!("TEST_CODE expected operation: {other:?}"),
    }
}

async fn quiesce(socket: &Path, request: &EffectRequest, class: WorkClass) -> ScopeStatus {
    let reply = EffectClient::request(
        socket,
        &Envelope {
            identity: identity(true),
            command: Command::Quiesce {
                scope: request.scope.clone(),
                broker_epoch: request.broker_epoch.clone(),
                class,
            },
        },
    )
    .await
    .unwrap();
    match reply {
        Reply::Scope(status) => status,
        other => panic!("TEST_CODE expected scope: {other:?}"),
    }
}

#[test]
#[ignore = "explicitly spawned by the four w16_effect_process_* parent tests"]
fn w16_effect_process_child() {
    let input: ChildInput = serde_json::from_reader(std::io::stdin()).unwrap();
    assert_eq!(input.root.parent(), Some(Path::new("/private/tmp")));
    assert!(input
        .root
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("TEST_CODE_FENCE_"));
    for path in [
        &input.database,
        &input.control,
        &input.socket,
        &input.signal,
    ] {
        assert_eq!(path.parent(), Some(input.root.as_path()));
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let announce = |signal: ChildSignal| {
            let mut channel = std::os::unix::net::UnixStream::connect(&input.signal).unwrap();
            channel
                .write_all(&serde_json::to_vec(&signal).unwrap())
                .unwrap();
        };
        match input.role {
            Role::Broker {
                scope,
                epoch,
                hooks,
            } => {
                let (observation_socket, _observation_peer) = UnixStream::pair().unwrap();
                let peer = super::activation_authorization::observe_unix_peer(&observation_socket)
                    .expect("TEST_CODE observe real fixture kernel identity");
                let clients = [false, true]
                    .into_iter()
                    .map(|supervisor| {
                        let identity = identity(supervisor);
                        TestClient {
                            client: identity.client,
                            incarnation: identity.incarnation,
                            credential: identity.credential,
                            // Kernel IDs of this test process; fixture credentials remain synthetic.
                            uid: peer.uid(),
                            gid: peer.gid(),
                            supervisor,
                        }
                    })
                    .collect();
                match EffectBroker::test_fixture(
                    &input.database,
                    &input.control,
                    scope,
                    epoch,
                    draft(),
                    clients,
                    hooks,
                ) {
                    Ok(broker) => {
                        let listener = UnixListener::bind(&input.socket).unwrap();
                        announce(ChildSignal::Listening(
                            broker.test_request("TEST_CODE_OPERATION"),
                        ));
                        Arc::new(broker).serve(listener).await.unwrap();
                    }
                    Err(error) => announce(ChildSignal::Answer(Reply::Refused(error))),
                }
            }
            Role::Client(envelope) => {
                let reply = EffectClient::request(&input.socket, &envelope)
                    .await
                    .unwrap();
                announce(ChildSignal::Answer(reply));
            }
            Role::HoldRequest(envelope) => {
                // Retain the requesting socket until the parent kills this exact process.
                // A separate bounded handshake keeps a live Child, not a timing assumption.
                let mut stream = std::os::unix::net::UnixStream::connect(&input.socket).unwrap();
                stream.set_write_timeout(Some(BOUND)).unwrap();
                let bytes = serde_json::to_vec(&envelope).unwrap();
                stream
                    .write_all(&(u32::try_from(bytes.len()).unwrap()).to_be_bytes())
                    .unwrap();
                stream.write_all(&bytes).unwrap();
                let mut lifetime = std::os::unix::net::UnixStream::connect(&input.signal).unwrap();
                lifetime
                    .set_read_timeout(Some(Duration::from_secs(20)))
                    .unwrap();
                lifetime.write_all(b"TEST_CODE_REQUEST_SENT").unwrap();
                let mut release = [0];
                lifetime
                    .read_exact(&mut release)
                    .expect("TEST_CODE parent must kill the live requester");
                panic!("TEST_CODE requester must be killed, not released");
            }
        }
    });
}

#[tokio::test]
async fn w16_effect_process_client_death_keeps_worker_and_quiesce_pending_until_real_commit() {
    let mut fixture = Fixture::new();
    let pause_path = fixture.root.path().join("pause.sock");
    let pause = UnixListener::bind(&pause_path).unwrap();
    let (_broker, socket, request) = fixture
        .broker(
            "first",
            1,
            TestHooks {
                pause_socket: Some(pause_path),
                ..TestHooks::default()
            },
        )
        .await;
    let (mut requester, _request_signal) = fixture.spawn(
        &socket,
        Role::HoldRequest(Envelope {
            identity: identity(false),
            command: Command::ExecuteCurrent {
                request: request.clone(),
                wait: true,
            },
        }),
    );
    let mut release = paused(&pause).await;
    assert_eq!(fixture.count("push_intents"), 0);
    let reply = fixture
        .child_request(
            &socket,
            Command::Quiesce {
                scope: request.scope.clone(),
                broker_epoch: request.broker_epoch.clone(),
                class: WorkClass::NewWork,
            },
            true,
        )
        .await;
    let Reply::Scope(status) = reply else {
        panic!("TEST_CODE quiesce reply")
    };
    assert!(!status.new_work_open && status.recovery_open && !status.drained);
    assert_eq!(status.unresolved, 1);
    assert!(
        requester.0.try_wait().unwrap().is_none(),
        "TEST_CODE requester is genuinely live before kill"
    );
    requester.stop();
    let query = Command::QueryOperation {
        request: request.clone(),
        wait: false,
    };
    assert_eq!(
        operation(fixture.child_request(&socket, query, false).await).state,
        OperationState::Running
    );
    let mut denied = request.clone();
    denied.operation_id = "TEST_CODE_AFTER_QUIESCE".into();
    assert_eq!(
        fixture
            .child_request(
                &socket,
                Command::ExecuteCurrent {
                    request: denied,
                    wait: false
                },
                false
            )
            .await,
        Reply::Refused(FenceError::Closed)
    );
    let closed = quiesce(&socket, &request, WorkClass::Recovery).await;
    assert!(!closed.drained && !closed.recovery_open);
    assert_eq!(closed.unresolved, 1);
    assert_eq!(fixture.count("push_intents"), 0);
    release.write_all(b"G").await.unwrap();
    let fact = operation(
        fixture
            .child_request(
                &socket,
                Command::QueryOperation {
                    request: request.clone(),
                    wait: true,
                },
                false,
            )
            .await,
    );
    assert_eq!(fact.state, OperationState::Succeeded);
    let receipt = fixture.assert_one_exact_intent();
    let result = fact.result.as_ref().unwrap();
    assert_eq!(result.intent_id, draft().intent_id().as_str());
    assert_eq!(result.initial_intent_sha256, receipt);
    assert_eq!(result.effect_sha256, request.effect_sha256);
    assert_eq!(
        operation(
            fixture
                .child_request(
                    &socket,
                    Command::ExecuteCurrent {
                        request: request.clone(),
                        wait: true,
                    },
                    false
                )
                .await
        ),
        fact
    );
    assert_eq!(fixture.assert_one_exact_intent(), receipt);
    assert!(
        quiesce(&socket, &request, WorkClass::Recovery)
            .await
            .drained
    );
}

#[tokio::test]
async fn w16_effect_process_broker_crash_keeps_prior_generation_unresolved_on_restart() {
    let mut fixture = Fixture::new();
    let pause_path = fixture.root.path().join("pause.sock");
    let pause = UnixListener::bind(&pause_path).unwrap();
    let (mut broker, socket, request) = fixture
        .broker(
            "old",
            1,
            TestHooks {
                pause_socket: Some(pause_path),
                ..TestHooks::default()
            },
        )
        .await;
    let started = fixture
        .child_request(
            &socket,
            Command::ExecuteCurrent {
                request: request.clone(),
                wait: false,
            },
            false,
        )
        .await;
    assert_eq!(operation(started).state, OperationState::Running);
    let _unreleased_worker = paused(&pause).await;
    broker.stop();
    assert_eq!(fixture.count("push_intents"), 0);
    let (_replacement, new_socket, next) =
        fixture.broker("replacement", 2, TestHooks::default()).await;
    let unresolved = operation(
        fixture
            .child_request(
                &new_socket,
                Command::QueryOperation {
                    request: request.clone(),
                    wait: false,
                },
                false,
            )
            .await,
    );
    assert_eq!(unresolved.state, OperationState::Unresolved);
    assert_eq!(unresolved.original_epoch, "old");
    let status = quiesce(&new_socket, &next, WorkClass::NewWork).await;
    assert!(!status.new_work_open && !status.recovery_open && !status.drained);
    assert_eq!(status.unresolved, 1);
    let mut fresh = next.clone();
    fresh.operation_id = "TEST_CODE_RESTART_NEW".into();
    assert_eq!(
        fixture
            .child_request(
                &new_socket,
                Command::ExecuteCurrent {
                    request: fresh,
                    wait: false
                },
                false
            )
            .await,
        Reply::Refused(FenceError::Closed)
    );
    let mut stale = request.clone();
    stale.operation_id = "TEST_CODE_OLD_EPOCH_NEW_OPERATION".into();
    assert_eq!(
        fixture
            .child_request(
                &new_socket,
                Command::ExecuteCurrent {
                    request: stale,
                    wait: false
                },
                false
            )
            .await,
        Reply::Refused(FenceError::Stale)
    );
    assert_eq!(
        operation(
            fixture
                .child_request(
                    &new_socket,
                    Command::ExecuteCurrent {
                        request,
                        wait: false
                    },
                    false
                )
                .await
        ),
        unresolved
    );
    assert_eq!(fixture.count("push_intents"), 0);
}

#[tokio::test]
async fn w16_effect_process_second_broker_cannot_own_the_same_control_store() {
    let mut fixture = Fixture::new();
    let (mut broker, _socket, _request) = fixture.broker("owner", 1, TestHooks::default()).await;
    let second_socket = fixture.root.path().join("second.sock");
    let (mut second, signal) = fixture.spawn(
        &second_socket,
        Role::Broker {
            scope: scope(1),
            epoch: "contender".into(),
            hooks: TestHooks::default(),
        },
    );
    assert!(matches!(
        receive_signal(&signal).await,
        ChildSignal::Answer(Reply::Refused(FenceError::AlreadyOwned))
    ));
    second.assert_success().await;
    let connection = rusqlite::Connection::open_with_flags(
        &fixture.control,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let epochs: i64 = connection
        .query_row("SELECT count(*) FROM effect_broker_epochs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        epochs, 1,
        "TEST_CODE denied second broker must not append an epoch"
    );
    drop(connection);
    broker.stop();
    let (_replacement, socket, request) = fixture
        .broker("after-owner-death", 1, TestHooks::default())
        .await;
    let status = quiesce(&socket, &request, WorkClass::NewWork).await;
    assert!(!status.new_work_open && !status.recovery_open && status.drained);
    assert_eq!(fixture.count("push_intents"), 0);
}

#[tokio::test]
async fn w16_effect_process_commit_ack_loss_preserves_actual_intent_without_reexecution() {
    let mut fixture = Fixture::new();
    let (mut broker, socket, request) = fixture
        .broker(
            "ack-lost",
            1,
            TestHooks {
                business_commit_ack_lost: true,
                ..TestHooks::default()
            },
        )
        .await;
    let original = operation(
        fixture
            .child_request(
                &socket,
                Command::ExecuteCurrent {
                    request: request.clone(),
                    wait: true,
                },
                false,
            )
            .await,
    );
    assert_eq!(original.state, OperationState::Unresolved);
    assert!(original.result.is_none());
    let receipt = fixture.assert_one_exact_intent();
    broker.stop();
    let (_replacement, next_socket, next) = fixture
        .broker("after-ack-loss", 2, TestHooks::default())
        .await;
    assert_eq!(
        operation(
            fixture
                .child_request(
                    &next_socket,
                    Command::QueryOperation {
                        request: request.clone(),
                        wait: false
                    },
                    false
                )
                .await
        ),
        original
    );
    assert_eq!(
        operation(
            fixture
                .child_request(
                    &next_socket,
                    Command::ExecuteCurrent {
                        request,
                        wait: true
                    },
                    false
                )
                .await
        ),
        original
    );
    assert_eq!(fixture.assert_one_exact_intent(), receipt);
    let status = quiesce(&next_socket, &next, WorkClass::Recovery).await;
    assert!(!status.drained);
    assert_eq!(status.unresolved, 1);
}
