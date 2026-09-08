//! Actual Generic sink/append operations in owned broker children; no external transport.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use super::activation_fence::{
    EffectBroker, EffectRequest, EffectResult, OperationFact, OperationState, Scope, ScopeStatus,
    TestClient, TestHooks, WorkClass,
};
use super::activation_fence_ipc::{ClientIdentity, Command, EffectClient, Envelope, Reply};
use super::activation_generic_effect::GenericEffectFixture;
use super::activation_generic_effect_tests::claimed_fixture_at;
use crate::durable_delivery::{
    AuthoritativeDeliveryRequest, AuthoritativeSinkPort, AuthoritativeSinkResult,
    CoordinatorConfig, DurableDeliveryCoordinator, DurableDeliveryError, ImmutableAppendPort,
    TypedReceipt, TypedUncertainty,
};
use crate::monitor::push_job::{raw_digest, UtcMicros};

const BOUND: Duration = Duration::from_secs(12);
const HELPER: &str = "push_foundation::activation_generic_process_tests::w16_generic_process_child";
const DISPATCHED: i64 = 1_788_743_102_000_000;
const VERIFIED: i64 = 1_788_743_103_000_000;

#[derive(Clone, Serialize, Deserialize)]
enum Role {
    Broker {
        epoch: String,
        hooks: TestHooks,
        pause: bool,
        pause_append: bool,
        uncertain: bool,
        fail_append: bool,
    },
    Requester(Envelope),
}

#[derive(Serialize, Deserialize)]
struct ChildInput {
    root: PathBuf,
    test_code: String,
    socket: PathBuf,
    signal: PathBuf,
    role: Role,
}

#[derive(Serialize, Deserialize)]
struct RegisteredRequests {
    dispatch: EffectRequest,
    recovery: EffectRequest,
}

struct OwnedChild(Child);

impl OwnedChild {
    fn stop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            self.0.kill().expect("kill only retained TEST_CODE Child");
        }
        self.0.wait().expect("observe actual child death");
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
            "TEST_CODE_SUPERVISOR"
        } else {
            "TEST_CODE_CLIENT"
        }
        .into(),
    }
}

fn durable_path(test_code: &str) -> PathBuf {
    assert!(test_code.starts_with("TEST_CODE_GENERIC_PROCESS_"));
    assert_eq!(Path::new(test_code).components().count(), 1);
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data/test")
        .join(test_code)
        .join("durable_delivery.sqlite3")
}

struct Fixture {
    root: tempfile::TempDir,
    durable_root: tempfile::TempDir,
    sequence: usize,
    pause_append: bool,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("TEST_CODE_GENERIC_")
            .tempdir_in("/private/tmp")
            .unwrap();
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/test");
        std::fs::create_dir_all(&parent).unwrap();
        let durable_root = tempfile::Builder::new()
            .prefix("TEST_CODE_GENERIC_PROCESS_")
            .tempdir_in(parent)
            .unwrap();
        let connection = rusqlite::Connection::open(root.path().join("ports.sqlite3")).unwrap();
        connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            CREATE TABLE sink_calls(sequence INTEGER PRIMARY KEY, decision_id TEXT, attempt_id TEXT, bytes BLOB, sha256 TEXT);
            CREATE TABLE appended(identity TEXT PRIMARY KEY, kind TEXT, bytes BLOB, sha256 TEXT);").unwrap();
        Self {
            root,
            durable_root,
            sequence: 0,
            pause_append: false,
        }
    }

    fn test_code(&self) -> &str {
        self.durable_root
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
    }

    fn spawn(&mut self, socket: &Path, role: Role) -> (OwnedChild, UnixListener) {
        self.sequence += 1;
        let signal = self
            .root
            .path()
            .join(format!("signal-{}.sock", self.sequence));
        let listener = UnixListener::bind(&signal).unwrap();
        let input = ChildInput {
            root: self.root.path().into(),
            test_code: self.test_code().into(),
            socket: socket.into(),
            signal,
            role,
        };
        let child = ProcessCommand::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", HELPER, "--nocapture"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn isolated Generic libtest helper");
        let mut child = OwnedChild(child);
        child
            .0
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&input).unwrap())
            .unwrap();
        (child, listener)
    }

    async fn broker(
        &mut self,
        epoch: &str,
        hooks: TestHooks,
        pause: bool,
        uncertain: bool,
        fail_append: bool,
    ) -> (OwnedChild, PathBuf, RegisteredRequests) {
        let socket = self.root.path().join(format!("{epoch}.sock"));
        let (child, signal) = self.spawn(
            &socket,
            Role::Broker {
                epoch: epoch.into(),
                hooks,
                pause,
                pause_append: self.pause_append,
                uncertain,
                fail_append,
            },
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
        .expect("actual Generic broker ready handshake");
        let requests = serde_json::from_slice(&bytes).expect("Generic broker registered requests");
        (child, socket, requests)
    }

    fn rows(&self, database: &Path, sql: &str) -> Vec<Vec<rusqlite::types::Value>> {
        let connection = rusqlite::Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut statement = connection.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn count(&self, database: &Path, table: &str) -> i64 {
        assert!(matches!(
            table,
            "sink_calls"
                | "appended"
                | "delivery_attempts"
                | "sink_results"
                | "effect_worker_completions"
        ));
        let rows = self.rows(database, &format!("SELECT count(*) FROM {table}"));
        match rows[0][0] {
            rusqlite::types::Value::Integer(value) => value,
            _ => panic!("integer count"),
        }
    }

    fn assert_single_attempt_and_sink(&self, result_count: i64) {
        assert_eq!(
            self.count(&self.root.path().join("ports.sqlite3"), "sink_calls"),
            1
        );
        let durable = durable_path(self.test_code());
        assert_eq!(self.count(&durable, "delivery_attempts"), 1);
        assert_eq!(self.count(&durable, "sink_results"), result_count);
        let rows = self.rows(
            &self.root.path().join("ports.sqlite3"),
            "SELECT bytes,sha256 FROM sink_calls",
        );
        let [rusqlite::types::Value::Blob(bytes), rusqlite::types::Value::Text(digest)] =
            rows[0].as_slice()
        else {
            panic!("exact sent bytes")
        };
        assert_eq!(bytes, b"first render  \nline two!");
        assert_eq!(raw_digest(bytes).as_str(), digest);
        let sent_identity = self.rows(
            &self.root.path().join("ports.sqlite3"),
            "SELECT decision_id,attempt_id FROM sink_calls",
        );
        assert_eq!(
            self.rows(
                &durable,
                "SELECT decision_identity,attempt_identity FROM delivery_attempts"
            ),
            sent_identity,
        );
        if result_count == 1 {
            assert_eq!(
                self.rows(
                    &durable,
                    "SELECT decision_identity,attempt_identity FROM sink_results"
                ),
                sent_identity,
            );
        }
    }
}

struct LocalSink {
    identity: String,
    ports: PathBuf,
    pause: Option<PathBuf>,
    uncertain: bool,
}

impl AuthoritativeSinkPort for LocalSink {
    fn sink_identity(&self) -> &str {
        &self.identity
    }

    fn deliver(&self, request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        let connection = rusqlite::Connection::open(&self.ports).unwrap();
        connection
            .execute(
                "INSERT INTO sink_calls(decision_id,attempt_id,bytes,sha256) VALUES(?1,?2,?3,?4)",
                rusqlite::params![
                    request.decision_identity,
                    request.attempt_identity,
                    request.rendered_content,
                    request.rendered_content_sha256
                ],
            )
            .unwrap();
        if let Some(path) = &self.pause {
            let mut stream = std::os::unix::net::UnixStream::connect(path).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            stream.set_write_timeout(Some(BOUND)).unwrap();
            stream.write_all(b"S").unwrap();
            let mut ack = [0];
            stream.read_exact(&mut ack).unwrap();
            assert_eq!(ack, *b"G");
        }
        let at = DateTime::<Utc>::from_timestamp_micros(DISPATCHED).unwrap();
        if self.uncertain {
            AuthoritativeSinkResult::Uncertain(TypedUncertainty {
                reason_code: "TEST_CODE_REMOTE_UNCERTAIN".into(),
                evidence: b"TEST_CODE_UNCERTAIN".to_vec(),
                observed_at: at,
            })
        } else {
            AuthoritativeSinkResult::Accepted(TypedReceipt {
                channel: self.identity.clone(),
                provider: "TEST_CODE_LOCAL_SINK".into(),
                message_id: "TEST_CODE_ACCEPTED".into(),
                platform_message_id: None,
                accepted_at: at,
                latency_ms: None,
            })
        }
    }
}

struct LocalAppend {
    database: PathBuf,
    fail_once: AtomicBool,
    pause: Option<PathBuf>,
    pause_once: AtomicBool,
}

impl ImmutableAppendPort for LocalAppend {
    fn append_exact(
        &self,
        kind: &str,
        identity: &str,
        bytes: &[u8],
        digest: &str,
    ) -> crate::durable_delivery::Result<String> {
        if self.pause_once.swap(false, Ordering::SeqCst) {
            if let Some(path) = &self.pause {
                let mut stream = std::os::unix::net::UnixStream::connect(path).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                stream.set_write_timeout(Some(BOUND)).unwrap();
                stream.write_all(b"A").unwrap();
                let mut ack = [0];
                stream.read_exact(&mut ack).unwrap();
                assert_eq!(ack, *b"G");
            }
        }
        if self.fail_once.swap(false, Ordering::SeqCst) {
            return Err(DurableDeliveryError::ImmutableAppendConflict(
                "TEST_CODE_INJECTED_APPEND_FAILURE".into(),
            ));
        }
        let connection = rusqlite::Connection::open(&self.database)?;
        connection.execute(
            "INSERT OR IGNORE INTO appended VALUES(?1,?2,?3,?4)",
            rusqlite::params![identity, kind, bytes, digest],
        )?;
        let actual: (String, Vec<u8>, String) = connection.query_row(
            "SELECT kind,bytes,sha256 FROM appended WHERE identity=?1",
            [identity],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if actual != (kind.into(), bytes.to_vec(), digest.into()) {
            return Err(DurableDeliveryError::ImmutableAppendConflict(
                identity.into(),
            ));
        }
        Ok(format!("TEST_CODE_APPEND:{identity}"))
    }
}

async fn request(socket: &Path, command: Command) -> Reply {
    let supervisor = matches!(command, Command::Quiesce { .. });
    EffectClient::request(
        socket,
        &Envelope {
            identity: identity(supervisor),
            command,
        },
    )
    .await
    .unwrap()
}

fn fact(reply: Reply) -> OperationFact {
    match reply {
        Reply::Operation(Some(fact)) => fact,
        other => panic!("expected actual operation, got {other:?}"),
    }
}

async fn settled(socket: &Path, operation: &EffectRequest) -> OperationFact {
    tokio::time::timeout(BOUND, async {
        loop {
            let observed = fact(
                request(
                    socket,
                    Command::QueryOperation {
                        request: operation.clone(),
                        wait: true,
                    },
                )
                .await,
            );
            if observed.state != OperationState::Running {
                return observed;
            }
        }
    })
    .await
    .expect("bounded query of same executing operation")
}

async fn close(socket: &Path, operation: &EffectRequest, class: WorkClass) -> ScopeStatus {
    match request(
        socket,
        Command::Quiesce {
            scope: operation.scope.clone(),
            broker_epoch: operation.broker_epoch.clone(),
            class,
        },
    )
    .await
    {
        Reply::Scope(status) => status,
        other => panic!("expected quiesce status, got {other:?}"),
    }
}

async fn actual_sink_pause(listener: &UnixListener) -> UnixStream {
    tokio::time::timeout(BOUND, async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut marker = [0];
        stream.read_exact(&mut marker).await.unwrap();
        assert_eq!(marker, *b"S");
        stream
    })
    .await
    .expect("actual deliver reached, not a pre-effect hook")
}

#[test]
#[ignore = "explicitly spawned by w16_generic_process sink/append/recovery parent tests"]
fn w16_generic_process_child() {
    let input: ChildInput = serde_json::from_reader(std::io::stdin()).unwrap();
    assert_eq!(input.root.parent(), Some(Path::new("/private/tmp")));
    assert!(input
        .root
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("TEST_CODE_GENERIC_"));
    assert_eq!(input.socket.parent(), Some(input.root.as_path()));
    assert_eq!(input.signal.parent(), Some(input.root.as_path()));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            match input.role {
                Role::Broker {
                    epoch,
                    hooks,
                    pause,
                    pause_append,
                    uncertain,
                    fail_append,
                } => {
                    let database = input.root.join("business.sqlite3");
                    let (snapshot, route, fence, completion_policy) =
                        claimed_fixture_at(&database, &input.test_code);
                    let coordinator = Arc::new(
                        DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                            durable_path(&input.test_code),
                            &input.test_code,
                            format!("TEST_CODE_OWNER_{}_{}", std::process::id(), epoch),
                        ))
                        .unwrap(),
                    );
                    let ports = input.root.join("ports.sqlite3");
                    let sink = Arc::new(LocalSink {
                        identity: route.required_channel().as_str().into(),
                        ports: ports.clone(),
                        pause: pause.then(|| input.root.join("sink.sock")),
                        uncertain,
                    });
                    let append_port = Arc::new(LocalAppend {
                        database: ports,
                        fail_once: AtomicBool::new(fail_append),
                        pause: pause_append.then(|| input.root.join("append.sock")),
                        pause_once: AtomicBool::new(pause_append),
                    });
                    let scope = Scope {
                        namespace: format!("Test:{}", input.test_code),
                        unit: "MU-auction".into(),
                        generation: 1,
                        manifest: "a".repeat(64),
                        physical_owner: "TEST_CODE_OWNER".into(),
                        deployment: "TEST_CODE_DEPLOYMENT".into(),
                        incarnation: "TEST_CODE_INCARNATION".into(),
                    };
                    let (socket, _peer) = UnixStream::pair().unwrap();
                    let observed =
                        super::activation_authorization::observe_unix_peer(&socket).unwrap();
                    let clients = [false, true]
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
                        .collect();
                    let fixture = GenericEffectFixture {
                        database,
                        coordinator,
                        snapshot,
                        route,
                        fence,
                        completion_policy,
                        sink,
                        append_port,
                        dispatched_at: UtcMicros::try_new(DISPATCHED).unwrap(),
                        verified_at: UtcMicros::try_new(VERIFIED).unwrap(),
                    };
                    let broker = EffectBroker::test_generic_fixture(
                        &input.root.join("control.sqlite3"),
                        scope,
                        epoch,
                        fixture,
                        clients,
                        hooks,
                    )
                    .unwrap();
                    let mut requests = RegisteredRequests {
                        dispatch: broker
                            .test_generic_request("TEST_CODE_DISPATCH", WorkClass::NewWork),
                        recovery: broker
                            .test_generic_request("TEST_CODE_RECOVERY", WorkClass::Recovery),
                    };
                    for request in [&mut requests.dispatch, &mut requests.recovery] {
                        request.client = identity(false).client;
                        request.client_incarnation = identity(false).incarnation;
                    }
                    let listener = UnixListener::bind(&input.socket).unwrap();
                    let mut signal =
                        std::os::unix::net::UnixStream::connect(&input.signal).unwrap();
                    signal
                        .write_all(&serde_json::to_vec(&requests).unwrap())
                        .unwrap();
                    drop(signal);
                    Arc::new(broker).serve(listener).await.unwrap();
                }
                Role::Requester(envelope) => {
                    let mut stream =
                        std::os::unix::net::UnixStream::connect(&input.socket).unwrap();
                    stream.set_write_timeout(Some(BOUND)).unwrap();
                    let bytes = serde_json::to_vec(&envelope).unwrap();
                    stream
                        .write_all(&(u32::try_from(bytes.len()).unwrap()).to_be_bytes())
                        .unwrap();
                    stream.write_all(&bytes).unwrap();
                    let mut lifetime =
                        std::os::unix::net::UnixStream::connect(&input.signal).unwrap();
                    lifetime
                        .set_read_timeout(Some(Duration::from_secs(30)))
                        .unwrap();
                    lifetime.write_all(b"LIVE").unwrap();
                    let mut release = [0];
                    lifetime
                        .read_exact(&mut release)
                        .expect("parent must kill retained requester");
                    panic!("requester must not be released normally");
                }
            }
        });
}

#[tokio::test]
async fn w16_generic_process_client_death_retains_sink_worker_and_confirmed_restart() {
    for completion_write_ack_lost in [false, true] {
        let mut fixture = Fixture::new();
        let sink_pause = UnixListener::bind(fixture.root.path().join("sink.sock")).unwrap();
        let (mut broker, socket, registered) = fixture
            .broker(
                "first",
                TestHooks {
                    completion_write_ack_lost,
                    ..TestHooks::default()
                },
                true,
                false,
                false,
            )
            .await;
        let dispatch = registered.dispatch;
        let (mut requester, _lifetime) = fixture.spawn(
            &socket,
            Role::Requester(Envelope {
                identity: identity(false),
                command: Command::ExecuteCurrent {
                    request: dispatch.clone(),
                    wait: true,
                },
            }),
        );
        let mut release = actual_sink_pause(&sink_pause).await;
        fixture.assert_single_attempt_and_sink(0);
        assert!(requester.0.try_wait().unwrap().is_none());
        requester.stop();
        let status = close(&socket, &dispatch, WorkClass::NewWork).await;
        assert!(!status.new_work_open && status.recovery_open && !status.drained);
        assert_eq!(status.unresolved, 1);
        assert!(!close(&socket, &dispatch, WorkClass::Recovery).await.drained);
        assert_eq!(
            fact(
                request(
                    &socket,
                    Command::QueryOperation {
                        request: dispatch.clone(),
                        wait: false
                    }
                )
                .await
            )
            .state,
            OperationState::Running
        );
        let mut another = dispatch.clone();
        another.operation_id = "TEST_CODE_DENIED_AFTER_CLOSE".into();
        assert!(matches!(
            request(
                &socket,
                Command::ExecuteCurrent {
                    request: another,
                    wait: false
                }
            )
            .await,
            Reply::Refused(_)
        ));
        release.write_all(b"G").await.unwrap();
        let completed = settled(&socket, &dispatch).await;
        assert_eq!(completed.state, OperationState::Succeeded);
        assert!(matches!(completed.result, Some(EffectResult::Generic(_))));
        fixture.assert_single_attempt_and_sink(1);
        assert!(fixture.count(&fixture.root.path().join("ports.sqlite3"), "appended") > 0);
        assert!(close(&socket, &dispatch, WorkClass::Recovery).await.drained);
        assert_eq!(
            fact(
                request(
                    &socket,
                    Command::ExecuteCurrent {
                        request: dispatch.clone(),
                        wait: true
                    }
                )
                .await
            ),
            completed
        );
        broker.stop();
        let (_replacement, next_socket, next) = fixture
            .broker("second", TestHooks::default(), false, false, false)
            .await;
        assert_eq!(
            fact(
                request(
                    &next_socket,
                    Command::ExecuteCurrent {
                        request: dispatch,
                        wait: true
                    }
                )
                .await
            ),
            completed
        );
        let status = close(&next_socket, &next.dispatch, WorkClass::NewWork).await;
        assert!(!status.new_work_open && !status.recovery_open && status.drained);
        fixture.assert_single_attempt_and_sink(1);
    }
}

#[tokio::test]
async fn w16_generic_process_append_in_progress_retains_worker_after_client_death() {
    let mut fixture = Fixture::new();
    fixture.pause_append = true;
    let append_pause = UnixListener::bind(fixture.root.path().join("append.sock")).unwrap();
    let (_broker, socket, registered) = fixture
        .broker("first", TestHooks::default(), false, false, false)
        .await;
    let dispatch = registered.dispatch;
    let (mut requester, _lifetime) = fixture.spawn(
        &socket,
        Role::Requester(Envelope {
            identity: identity(false),
            command: Command::ExecuteCurrent {
                request: dispatch.clone(),
                wait: true,
            },
        }),
    );
    let mut release = tokio::time::timeout(BOUND, async {
        let (mut stream, _) = append_pause.accept().await.unwrap();
        let mut marker = [0];
        stream.read_exact(&mut marker).await.unwrap();
        assert_eq!(marker, *b"A");
        stream
    })
    .await
    .expect("actual append_exact reached after durable sink result");
    fixture.assert_single_attempt_and_sink(1);
    let durable = durable_path(fixture.test_code());
    let attempts = fixture.rows(&durable, "SELECT * FROM delivery_attempts");
    let sink_sql = "SELECT result_event_identity,decision_identity,attempt_identity,result_canonical,result_sha256 FROM sink_results";
    let sink_results = fixture.rows(&durable, sink_sql);
    let ports = fixture.root.path().join("ports.sqlite3");
    let control = fixture.root.path().join("control.sqlite3");
    assert_eq!(fixture.count(&ports, "appended"), 0);
    assert_eq!(fixture.count(&control, "effect_worker_completions"), 0);
    let status = close(&socket, &dispatch, WorkClass::NewWork).await;
    assert!(!status.new_work_open && status.recovery_open && !status.drained);
    assert_eq!(status.unresolved, 1);
    assert!(requester.0.try_wait().unwrap().is_none());
    requester.stop();
    let status = close(&socket, &dispatch, WorkClass::Recovery).await;
    assert!(!status.new_work_open && !status.recovery_open && !status.drained);
    assert_eq!(status.unresolved, 1);
    assert_eq!(
        fact(
            request(
                &socket,
                Command::QueryOperation {
                    request: dispatch.clone(),
                    wait: false
                }
            )
            .await
        )
        .state,
        OperationState::Running
    );
    assert_eq!(fixture.count(&control, "effect_worker_completions"), 0);
    assert_eq!(fixture.count(&ports, "appended"), 0);
    fixture.assert_single_attempt_and_sink(1);

    release.write_all(b"G").await.unwrap();
    let completed = settled(&socket, &dispatch).await;
    assert_eq!(completed.state, OperationState::Succeeded);
    assert!(matches!(completed.result, Some(EffectResult::Generic(_))));
    assert!(fixture.count(&ports, "appended") > 0);
    assert_eq!(fixture.count(&control, "effect_worker_completions"), 1);
    assert!(close(&socket, &dispatch, WorkClass::Recovery).await.drained);
    assert_eq!(
        fact(
            request(
                &socket,
                Command::QueryOperation {
                    request: dispatch.clone(),
                    wait: true
                }
            )
            .await
        ),
        completed
    );
    assert_eq!(
        fact(
            request(
                &socket,
                Command::ExecuteCurrent {
                    request: dispatch,
                    wait: true
                }
            )
            .await
        ),
        completed
    );
    assert_eq!(fixture.count(&control, "effect_worker_completions"), 1);
    fixture.assert_single_attempt_and_sink(1);
    assert_eq!(
        fixture.rows(&durable, "SELECT * FROM delivery_attempts"),
        attempts
    );
    assert_eq!(fixture.rows(&durable, sink_sql), sink_results);
}

#[tokio::test]
async fn w16_generic_process_broker_death_during_sink_never_resends_unknown_attempt() {
    let mut fixture = Fixture::new();
    let sink_pause = UnixListener::bind(fixture.root.path().join("sink.sock")).unwrap();
    let (mut broker, socket, registered) = fixture
        .broker("first", TestHooks::default(), true, false, false)
        .await;
    let dispatch = registered.dispatch;
    assert_eq!(
        fact(
            request(
                &socket,
                Command::ExecuteCurrent {
                    request: dispatch.clone(),
                    wait: false
                }
            )
            .await
        )
        .state,
        OperationState::Running
    );
    let blocked_sink = actual_sink_pause(&sink_pause).await;
    fixture.assert_single_attempt_and_sink(0);
    let before = fixture.rows(
        &durable_path(fixture.test_code()),
        "SELECT * FROM delivery_attempts",
    );
    broker.stop();
    drop(blocked_sink);
    let (_replacement, next_socket, next) = fixture
        .broker("second", TestHooks::default(), false, false, false)
        .await;
    assert_eq!(
        fact(
            request(
                &next_socket,
                Command::ExecuteCurrent {
                    request: dispatch,
                    wait: true
                }
            )
            .await
        )
        .state,
        OperationState::Unresolved
    );
    let status = close(&next_socket, &next.dispatch, WorkClass::NewWork).await;
    assert!(!status.new_work_open && !status.recovery_open && !status.drained);
    assert_eq!(
        fixture.rows(
            &durable_path(fixture.test_code()),
            "SELECT * FROM delivery_attempts"
        ),
        before
    );
    fixture.assert_single_attempt_and_sink(0);
}

#[tokio::test]
async fn w16_generic_process_final_result_read_failure_stays_unresolved_after_restart() {
    let mut fixture = Fixture::new();
    let (mut broker, socket, registered) = fixture
        .broker(
            "first",
            TestHooks {
                final_result_read_failure: true,
                ..TestHooks::default()
            },
            false,
            false,
            false,
        )
        .await;
    let dispatch = registered.dispatch;
    request(
        &socket,
        Command::ExecuteCurrent {
            request: dispatch.clone(),
            wait: false,
        },
    )
    .await;
    assert_eq!(
        settled(&socket, &dispatch).await.state,
        OperationState::Unresolved
    );
    let control = fixture.root.path().join("control.sqlite3");
    assert_eq!(
        fixture.rows(
            &control,
            "SELECT state,result_json IS NOT NULL FROM effect_operations"
        ),
        vec![vec![
            rusqlite::types::Value::Text("Succeeded".into()),
            rusqlite::types::Value::Integer(1)
        ]]
    );
    assert_eq!(fixture.count(&control, "effect_worker_completions"), 0);
    fixture.assert_single_attempt_and_sink(1);
    broker.stop();
    let (_replacement, next_socket, next) = fixture
        .broker("second", TestHooks::default(), false, false, false)
        .await;
    assert_eq!(
        fact(
            request(
                &next_socket,
                Command::ExecuteCurrent {
                    request: dispatch,
                    wait: true
                }
            )
            .await
        )
        .state,
        OperationState::Unresolved
    );
    assert!(
        !close(&next_socket, &next.dispatch, WorkClass::Recovery)
            .await
            .drained
    );
    fixture.assert_single_attempt_and_sink(1);
}

#[tokio::test]
async fn w16_generic_process_uncertain_terminal_remains_unresolved_without_second_send() {
    let mut fixture = Fixture::new();
    let (mut broker, socket, registered) = fixture
        .broker("first", TestHooks::default(), false, true, false)
        .await;
    let dispatch = registered.dispatch;
    request(
        &socket,
        Command::ExecuteCurrent {
            request: dispatch.clone(),
            wait: false,
        },
    )
    .await;
    assert_eq!(
        settled(&socket, &dispatch).await.state,
        OperationState::Unresolved
    );
    fixture.assert_single_attempt_and_sink(1);
    let before = fixture.rows(
        &durable_path(fixture.test_code()),
        "SELECT * FROM sink_results",
    );
    broker.stop();
    let (_replacement, next_socket, next) = fixture
        .broker("second", TestHooks::default(), false, false, false)
        .await;
    assert_eq!(
        fact(
            request(
                &next_socket,
                Command::ExecuteCurrent {
                    request: dispatch,
                    wait: true
                }
            )
            .await
        )
        .state,
        OperationState::Unresolved
    );
    assert!(
        !close(&next_socket, &next.dispatch, WorkClass::Recovery)
            .await
            .drained
    );
    assert_eq!(
        fixture.rows(
            &durable_path(fixture.test_code()),
            "SELECT * FROM sink_results"
        ),
        before
    );
    fixture.assert_single_attempt_and_sink(1);
}

#[tokio::test]
async fn w16_generic_process_recovery_finishes_existing_append_with_new_work_closed() {
    let mut fixture = Fixture::new();
    let (_broker, socket, registered) = fixture
        .broker("first", TestHooks::default(), false, false, true)
        .await;
    request(
        &socket,
        Command::ExecuteCurrent {
            request: registered.dispatch.clone(),
            wait: false,
        },
    )
    .await;
    assert_eq!(
        settled(&socket, &registered.dispatch).await.state,
        OperationState::Unresolved
    );
    fixture.assert_single_attempt_and_sink(1);
    assert_eq!(
        fixture.count(&fixture.root.path().join("ports.sqlite3"), "appended"),
        0
    );
    let status = close(&socket, &registered.dispatch, WorkClass::NewWork).await;
    assert!(!status.new_work_open && status.recovery_open && !status.drained);
    request(
        &socket,
        Command::ExecuteCurrent {
            request: registered.recovery.clone(),
            wait: false,
        },
    )
    .await;
    assert_eq!(
        settled(&socket, &registered.recovery).await.state,
        OperationState::Succeeded
    );
    assert!(fixture.count(&fixture.root.path().join("ports.sqlite3"), "appended") > 0);
    fixture.assert_single_attempt_and_sink(1);
    // A distinct observation cannot silently rewrite the original unresolved operation.
    assert_eq!(
        fact(
            request(
                &socket,
                Command::QueryOperation {
                    request: registered.dispatch.clone(),
                    wait: false
                }
            )
            .await
        )
        .state,
        OperationState::Unresolved
    );
    assert!(
        !close(&socket, &registered.recovery, WorkClass::Recovery)
            .await
            .drained
    );
    let mut denied = registered.recovery;
    denied.operation_id = "TEST_CODE_CLOSED_RECOVERY".into();
    assert!(matches!(
        request(
            &socket,
            Command::ExecuteCurrent {
                request: denied,
                wait: false
            }
        )
        .await,
        Reply::Refused(_)
    ));
    fixture.assert_single_attempt_and_sink(1);
}
