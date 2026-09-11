//! Real Generic delivery followed by broker-owned business recovery in separate children.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use super::activation_business_effect::BusinessEffectFixture;
use super::activation_fence::{
    EffectBroker, EffectRequest, EffectResult, OperationFact, OperationState, Scope, TestClient,
    TestHooks, WorkClass,
};
use super::activation_fence_ipc::{Command, Envelope, Reply};
use super::activation_generic_process_tests::{
    close, durable_path, fact, identity, request, settled, Fixture, OwnedChild, Role, BOUND,
};
use super::reconciler::RecoveryConfig;
use super::{BusinessIntentStore, IntentSnapshot, IntentState, LeaseOwnerId, TransitionActor};
use crate::durable_delivery::{CoordinatorConfig, DurableDeliveryCoordinator};
use crate::monitor::push_job::{
    raw_digest, w09_completion_policy_fixture, AuthorityClass, IntentId, Sha256Digest, TemplateId,
    TemplateVersion, UtcMicros,
};

const RECOVERY_AT: i64 = 1_788_743_104_000_000;
const RECOVERY_UNTIL: i64 = 1_788_743_400_000_000;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct BusinessRole {
    epoch: String,
    hooks: TestHooks,
}

pub(super) fn only_intent(database: &Path) -> IntentId {
    let connection =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let mut statement = connection
        .prepare("SELECT intent_id FROM push_intents")
        .unwrap();
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(ids.len(), 1, "the fixture owns exactly one actual intent");
    IntentId::from_digest(&Sha256Digest::parse("actual fixture intent", &ids[0]).unwrap())
}

fn business_snapshot(fixture: &Fixture) -> IntentSnapshot {
    let database = fixture.root.path().join("business.sqlite3");
    let id = only_intent(&database);
    BusinessIntentStore::open(&database)
        .unwrap()
        .inspect(&id)
        .unwrap()
        .unwrap()
}

fn assert_business(fixture: &Fixture, state: IntentState, version: u64) {
    let database = fixture.root.path().join("business.sqlite3");
    let store = BusinessIntentStore::open(&database).unwrap();
    let id = only_intent(&database);
    let snapshot = store.inspect(&id).unwrap().unwrap();
    let chain = store.inspect_transition_chain(&id).unwrap();
    assert_eq!(snapshot.state(), state);
    assert_eq!(snapshot.version(), version);
    assert_eq!(chain.len() as u64, version);
    assert_eq!(snapshot.lease_generation(), 1);
    assert_eq!(
        snapshot.namespace(),
        format!("Test:{}", fixture.test_code())
    );
    let attested = snapshot.attested_ready_binding().unwrap();
    assert_eq!(attested.unit_id.as_str(), "MU-auction");
    assert_eq!(attested.intent_id, id);
    assert_eq!(chain.last().unwrap().to_state(), state);
}

fn transport_rows(fixture: &Fixture) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    let durable = durable_path(fixture.test_code());
    vec![
        fixture.rows(&durable, "SELECT * FROM delivery_attempts"),
        fixture.rows(&durable, "SELECT * FROM sink_results"),
        fixture.rows(
            &fixture.root.path().join("ports.sqlite3"),
            "SELECT * FROM sink_calls",
        ),
        fixture.rows(
            &fixture.root.path().join("ports.sqlite3"),
            "SELECT * FROM appended ORDER BY identity",
        ),
    ]
}

fn literal_bytes(domain: &str, value: &impl Serialize) -> Vec<u8> {
    let mut bytes = domain.as_bytes().to_vec();
    bytes.push(0);
    bytes.extend(serde_json::to_vec(value).unwrap());
    bytes
}

pub(super) fn assert_result_matches_business(
    fixture: &Fixture,
    operation: &EffectRequest,
    fact: &OperationFact,
) {
    let Some(EffectResult::BusinessRecovery(result)) = &fact.result else {
        panic!("actual business recovery result required");
    };
    let database = fixture.root.path().join("business.sqlite3");
    let store = BusinessIntentStore::open(&database).unwrap();
    let id = only_intent(&database);
    let snapshot = store.inspect(&id).unwrap().unwrap();
    let chain = store.inspect_transition_chain(&id).unwrap();
    let attested = snapshot.attested_ready_binding().unwrap();
    assert_eq!(result.intent_id, id.as_str());
    assert_eq!(result.decision_id, attested.decision_id.as_str());
    assert_eq!(result.effect_sha256, operation.effect_sha256);
    assert_eq!(result.state, snapshot.state().as_str());
    assert_eq!(result.version, snapshot.version());
    assert_eq!(result.lease_generation, snapshot.lease_generation());
    assert_eq!(result.transition_count as usize, chain.len());
    assert_eq!(
        result.last_event_id.as_deref(),
        chain.last().map(|event| event.event_id().as_str())
    );
    assert_eq!(
        result.last_event_sha256.as_deref(),
        chain.last().map(|event| event.canonical_sha256().as_str())
    );
    let events: Vec<&str> = chain
        .iter()
        .map(|event| event.canonical_sha256().as_str())
        .collect();
    let chain_bytes = literal_bytes(
        "ActivationBusinessRecoveryTransitionChain/v1",
        &serde_json::json!({ "events": events }),
    );
    assert_eq!(
        result.transition_chain_sha256,
        raw_digest(&chain_bytes).as_str()
    );

    // Independent SQL projection, not IntentSnapshot::activation_snapshot_fields or its codec.
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut statement = connection.prepare(
        "SELECT intent_id,job_decision_kind AS decision_kind,namespace,unit_id,occurrence_family,occurrence_key,\
         completion_owner,source_contract_id,subject,audience,durable_decision_id AS decision_id,business_date,\
         prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256,evidence_sha256,template_sha256,\
         source_contract_sha256,state,previous_state,reason,lease_owner,CAST(lease_until AS TEXT) AS lease_until,\
         lease_generation,version,CAST(created_at AS TEXT) AS created_at,CAST(updated_at AS TEXT) AS updated_at \
         FROM push_intents WHERE intent_id=?1",
    ).unwrap();
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(names.len(), 28);
    let fields: BTreeMap<String, serde_json::Value> = statement
        .query_row([id.as_str()], |row| {
            names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    let value = match row.get::<_, rusqlite::types::Value>(index)? {
                        rusqlite::types::Value::Null => serde_json::Value::Null,
                        rusqlite::types::Value::Integer(value) => serde_json::json!(value),
                        rusqlite::types::Value::Text(value) => serde_json::json!(value),
                        rusqlite::types::Value::Blob(value) => serde_json::json!(value),
                        rusqlite::types::Value::Real(_) => {
                            panic!("no floating-point field in attested intent")
                        }
                    };
                    Ok((name.clone(), value))
                })
                .collect()
        })
        .unwrap();
    let bytes = literal_bytes("ActivationBusinessRecoverySnapshot/v1", &fields);
    assert_eq!(result.snapshot_sha256, raw_digest(&bytes).as_str());
    assert_eq!(
        result.recovery_boundary,
        if snapshot.state() == IntentState::Completed {
            "Finalized"
        } else {
            "ManualResolutionRequired"
        }
    );
}

pub(super) fn assert_completion_proof(
    fixture: &Fixture,
    operation: &EffectRequest,
    fact: &OperationFact,
) {
    assert_result_matches_business(fixture, operation, fact);
    let Some(EffectResult::BusinessRecovery(result)) = &fact.result else {
        unreachable!()
    };
    let result_fields = serde_json::json!({
        "intent_id": result.intent_id, "decision_id": result.decision_id, "effect_sha256": result.effect_sha256,
        "state": result.state, "version": result.version, "lease_generation": result.lease_generation,
        "snapshot_sha256": result.snapshot_sha256, "transition_chain_sha256": result.transition_chain_sha256,
        "transition_count": result.transition_count, "last_event_id": result.last_event_id,
        "last_event_sha256": result.last_event_sha256, "recovery_boundary": result.recovery_boundary,
    });
    let expected = literal_bytes(
        "ActivationBusinessRecoveryWorkerCompletion/v1",
        &serde_json::json!({
            "operation_id": operation.operation_id, "request_sha256": operation.digest(),
            "original_epoch": fact.original_epoch, "state": "Succeeded", "result": result_fields,
        }),
    );
    let control = fixture.root.path().join("business-control.sqlite3");
    assert_eq!(
        fixture.rows(
            &control,
            "SELECT completion_bytes,completion_sha256 FROM effect_worker_completions"
        ),
        vec![vec![
            rusqlite::types::Value::Blob(expected.clone()),
            rusqlite::types::Value::Text(raw_digest(&expected).as_str().into()),
        ]]
    );
}

async fn seed_terminal(fixture: &mut Fixture, uncertain: bool) {
    let (mut sender, socket, registered) = fixture
        .broker("sender", TestHooks::default(), false, uncertain, false)
        .await;
    request(
        &socket,
        Command::ExecuteCurrent {
            request: registered.dispatch.clone(),
            wait: false,
        },
    )
    .await;
    let outcome = settled(&socket, &registered.dispatch).await;
    assert_eq!(
        outcome.state,
        if uncertain {
            OperationState::Unresolved
        } else {
            OperationState::Succeeded
        }
    );
    assert!(matches!(outcome.result, Some(EffectResult::Generic(_))));
    fixture.assert_single_attempt_and_sink(1);
    assert_business(fixture, IntentState::AwaitingAuthority, 1);
    sender.stop();
}

async fn business_broker(
    fixture: &mut Fixture,
    epoch: &str,
    hooks: TestHooks,
) -> (OwnedChild, PathBuf, EffectRequest) {
    let socket = fixture.root.path().join(format!("{epoch}.sock"));
    let (child, signal) = fixture.spawn(
        &socket,
        Role::BusinessBroker(BusinessRole {
            epoch: epoch.into(),
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
    .expect("actual business broker ready handshake");
    let operation = serde_json::from_slice(&bytes).unwrap();
    (child, socket, operation)
}

pub(super) async fn run_broker(
    root: &Path,
    test_code: &str,
    socket: &Path,
    signal: &Path,
    role: BusinessRole,
) {
    let database = root.join("business.sqlite3");
    let store = BusinessIntentStore::open(&database).unwrap();
    let snapshot = store.inspect(&only_intent(&database)).unwrap().unwrap();
    drop(store);
    let coordinator = Arc::new(
        DurableDeliveryCoordinator::open(CoordinatorConfig::test(
            durable_path(test_code),
            test_code,
            format!(
                "TEST_CODE_BUSINESS_OWNER_{}_{}",
                std::process::id(),
                role.epoch
            ),
        ))
        .unwrap(),
    );
    let template = super::TerminalTemplateBinding::new(
        TemplateId::try_new("auction-card".into()).unwrap(),
        TemplateVersion::try_new("auction-card-v3".into()).unwrap(),
    );
    let completion_policy = w09_completion_policy_fixture(
        "MU-auction",
        "owner-auction",
        vec![AuthorityClass::GenericCounted],
    );
    let config = RecoveryConfig::try_new(
        LeaseOwnerId::try_new("w16-dispatcher".into()).unwrap(),
        TransitionActor::try_new("w16-business-recovery".into()).unwrap(),
        UtcMicros::try_new(RECOVERY_AT).unwrap(),
        UtcMicros::try_new(RECOVERY_UNTIL).unwrap(),
        1,
        8,
    )
    .unwrap();
    let scope = Scope {
        namespace: format!("Test:{test_code}"),
        unit: "MU-auction".into(),
        generation: 1,
        manifest: "a".repeat(64),
        physical_owner: "TEST_CODE_OWNER".into(),
        deployment: "TEST_CODE_DEPLOYMENT".into(),
        incarnation: "TEST_CODE_INCARNATION".into(),
    };
    let (peer, _other) = UnixStream::pair().unwrap();
    let observed = super::activation_authorization::observe_unix_peer(&peer).unwrap();
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
    let broker = EffectBroker::test_business_fixture(
        &root.join("business-control.sqlite3"),
        scope,
        role.epoch,
        BusinessEffectFixture {
            database,
            coordinator,
            snapshot,
            template,
            completion_policy,
            config,
        },
        clients,
        role.hooks,
    )
    .unwrap();
    let mut operation = broker.test_business_request("TEST_CODE_BUSINESS_RECOVERY");
    operation.client = identity(false).client;
    operation.client_incarnation = identity(false).incarnation;
    let listener = UnixListener::bind(socket).unwrap();
    let mut signal_stream = std::os::unix::net::UnixStream::connect(signal).unwrap();
    signal_stream
        .write_all(&serde_json::to_vec(&operation).unwrap())
        .unwrap();
    drop(signal_stream);
    Arc::new(broker).serve(listener).await.unwrap();
}

async fn requery_pause(listener: &UnixListener) -> UnixStream {
    tokio::time::timeout(BOUND, async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut marker = [0];
        stream.read_exact(&mut marker).await.unwrap();
        assert_eq!(marker, *b"P");
        stream
    })
    .await
    .expect("second actual authority query reached after qualification")
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

#[tokio::test]
async fn business_completion_retains_worker_after_requester_death_and_confirmed_restart() {
    for completion_write_ack_lost in [false, true] {
        let mut fixture = Fixture::new();
        seed_terminal(&mut fixture, false).await;
        let transport = transport_rows(&fixture);
        let pause_path = fixture.root.path().join("business-query.sock");
        let listener = UnixListener::bind(&pause_path).unwrap();
        let (mut broker, socket, operation) = business_broker(
            &mut fixture,
            "business-first",
            TestHooks {
                business_requery_pause_socket: Some(pause_path),
                completion_write_ack_lost,
                ..TestHooks::default()
            },
        )
        .await;
        let admission = close(&socket, &operation, WorkClass::NewWork).await;
        assert!(!admission.new_work_open && admission.recovery_open);
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
        let mut paused = requery_pause(&listener).await;
        assert_business(&fixture, IntentState::AwaitingFinalizer, 2);
        let status = close(&socket, &operation, WorkClass::NewWork).await;
        assert!(!status.new_work_open && status.recovery_open && !status.drained);
        requester.stop();
        let status = close(&socket, &operation, WorkClass::Recovery).await;
        assert!(!status.new_work_open && !status.recovery_open && !status.drained);
        assert_eq!(status.unresolved, 1);
        assert_eq!(
            fact(
                request(
                    &socket,
                    Command::QueryOperation {
                        request: operation.clone(),
                        wait: false
                    }
                )
                .await
            )
            .state,
            OperationState::Running
        );
        let control = fixture.root.path().join("business-control.sqlite3");
        assert_eq!(fixture.count(&control, "effect_worker_completions"), 0);
        assert_eq!(transport_rows(&fixture), transport);
        paused.write_all(b"G").await.unwrap();
        let done = settled(&socket, &operation).await;
        assert_eq!(done.state, OperationState::Succeeded);
        assert!(matches!(
            done.result,
            Some(EffectResult::BusinessRecovery(_))
        ));
        assert_business(&fixture, IntentState::Completed, 3);
        assert_completion_proof(&fixture, &operation, &done);
        assert_eq!(fixture.count(&control, "effect_worker_completions"), 1);
        assert!(
            close(&socket, &operation, WorkClass::Recovery)
                .await
                .drained
        );
        let business = business_snapshot(&fixture);
        exact_replay(&socket, &operation, &done).await;
        broker.stop();
        let (_replacement, socket, next) =
            business_broker(&mut fixture, "business-second", TestHooks::default()).await;
        exact_replay(&socket, &operation, &done).await;
        let status = close(&socket, &next, WorkClass::Recovery).await;
        assert_completion_proof(&fixture, &operation, &done);
        assert!(!status.new_work_open && !status.recovery_open && status.drained);
        assert!(matches!(
            request(
                &socket,
                Command::ExecuteCurrent {
                    request: next,
                    wait: false
                }
            )
            .await,
            Reply::Refused(_)
        ));
        assert_eq!(business_snapshot(&fixture), business);
        assert_eq!(transport_rows(&fixture), transport);
        fixture.assert_single_attempt_and_sink(1);
    }
}

#[tokio::test]
async fn business_broker_death_after_qualification_retains_unresolved_and_never_resends() {
    let mut fixture = Fixture::new();
    seed_terminal(&mut fixture, false).await;
    let transport = transport_rows(&fixture);
    let pause_path = fixture.root.path().join("business-query.sock");
    let listener = UnixListener::bind(&pause_path).unwrap();
    let (mut broker, socket, operation) = business_broker(
        &mut fixture,
        "business-first",
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
    let _paused = requery_pause(&listener).await;
    assert_business(&fixture, IntentState::AwaitingFinalizer, 2);
    let business = business_snapshot(&fixture);
    broker.stop();
    let (_replacement, socket, next) =
        business_broker(&mut fixture, "business-second", TestHooks::default()).await;
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
    assert_eq!(
        fixture.count(
            &fixture.root.path().join("business-control.sqlite3"),
            "effect_worker_completions"
        ),
        0
    );
    assert_eq!(business_snapshot(&fixture), business);
    assert_eq!(transport_rows(&fixture), transport);
    fixture.assert_single_attempt_and_sink(1);
}

#[tokio::test]
async fn business_completion_confirmation_read_failure_cannot_become_success_after_restart() {
    let mut fixture = Fixture::new();
    seed_terminal(&mut fixture, false).await;
    let transport = transport_rows(&fixture);
    let (mut broker, socket, operation) = business_broker(
        &mut fixture,
        "business-first",
        TestHooks {
            final_result_read_failure: true,
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
    assert_eq!(
        settled(&socket, &operation).await.state,
        OperationState::Unresolved
    );
    assert_business(&fixture, IntentState::Completed, 3);
    let business = business_snapshot(&fixture);
    let control = fixture.root.path().join("business-control.sqlite3");
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
    broker.stop();
    let (_replacement, socket, next) =
        business_broker(&mut fixture, "business-second", TestHooks::default()).await;
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
    assert_eq!(business_snapshot(&fixture), business);
    assert_eq!(transport_rows(&fixture), transport);
    assert_eq!(fixture.count(&control, "effect_worker_completions"), 0);
    fixture.assert_single_attempt_and_sink(1);
}

#[tokio::test]
async fn business_uncertainty_keeps_manual_resolution_and_zero_recovery_sends_across_restart() {
    let mut fixture = Fixture::new();
    seed_terminal(&mut fixture, true).await;
    let transport = transport_rows(&fixture);
    let (mut broker, socket, operation) =
        business_broker(&mut fixture, "business-first", TestHooks::default()).await;
    let status = close(&socket, &operation, WorkClass::NewWork).await;
    assert!(!status.new_work_open && status.recovery_open);
    request(
        &socket,
        Command::ExecuteCurrent {
            request: operation.clone(),
            wait: false,
        },
    )
    .await;
    let observed = settled(&socket, &operation).await;
    assert_eq!(observed.state, OperationState::Unresolved);
    assert!(matches!(
        observed.result,
        Some(EffectResult::BusinessRecovery(_))
    ));
    assert_business(&fixture, IntentState::ResolutionRequired, 2);
    assert_result_matches_business(&fixture, &operation, &observed);
    let business = business_snapshot(&fixture);
    exact_replay(&socket, &operation, &observed).await;
    broker.stop();
    let (_replacement, socket, next) =
        business_broker(&mut fixture, "business-second", TestHooks::default()).await;
    exact_replay(&socket, &operation, &observed).await;
    assert!(!close(&socket, &next, WorkClass::Recovery).await.drained);
    assert_eq!(business_snapshot(&fixture), business);
    assert_eq!(transport_rows(&fixture), transport);
    assert_eq!(
        fixture.count(
            &fixture.root.path().join("business-control.sqlite3"),
            "effect_worker_completions"
        ),
        0
    );
    fixture.assert_single_attempt_and_sink(1);
}
