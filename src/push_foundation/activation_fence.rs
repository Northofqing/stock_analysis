//! Broker-owned typed execution. No caller can borrow a permit or release a worker.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::activation_fence_store::{CompletionFault, OperationStore};
use super::intent_store::{BusinessIntentStore, InitialIntentDraft};
use crate::monitor::push_job::{canonical_preimage, raw_digest, CanonicalValue};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub(super) enum FenceError {
    #[error("production trust roots unavailable")]
    ProductionRefused,
    #[error("unregistered client, incarnation or credential")]
    Unauthorized,
    #[error("binding is not the current activation tuple")]
    Stale,
    #[error("scope is closed")]
    Closed,
    #[error("unrecognized or mismatched fixed effect")]
    EffectMismatch,
    #[error("operation ID already binds different exact bytes")]
    Conflict,
    #[error("control store unavailable or confirmation uncertain")]
    Store,
    #[error("control store already has a broker")]
    AlreadyOwned,
    #[error("bounded IPC failed")]
    Protocol,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Scope {
    pub(super) namespace: String,
    pub(super) unit: String,
    pub(super) generation: u64,
    pub(super) manifest: String,
    pub(super) physical_owner: String,
    pub(super) deployment: String,
    pub(super) incarnation: String,
}

impl Scope {
    fn canonical_fields(&self) -> BTreeMap<&'static str, CanonicalValue> {
        let mut fields = BTreeMap::new();
        for (key, value) in [
            ("namespace", &self.namespace),
            ("unit", &self.unit),
            ("manifest", &self.manifest),
            ("physical_owner", &self.physical_owner),
            ("deployment", &self.deployment),
            ("incarnation", &self.incarnation),
        ] {
            fields.insert(key, CanonicalValue::String(value.clone()));
        }
        fields.insert("generation", CanonicalValue::Unsigned(self.generation));
        fields
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum WorkClass {
    NewWork,
    Recovery,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EffectRequest {
    pub(super) scope: Scope,
    pub(super) broker_epoch: String,
    pub(super) client: String,
    pub(super) client_incarnation: String,
    pub(super) actor: String,
    pub(super) action: String,
    pub(super) work_class: WorkClass,
    pub(super) operation_id: String,
    pub(super) effect_id: String,
    pub(super) effect_sha256: String,
}

impl EffectRequest {
    pub(super) fn canonical_bytes(&self) -> Vec<u8> {
        let mut fields = self.scope.canonical_fields();
        for (key, value) in [
            ("broker_epoch", &self.broker_epoch),
            ("client", &self.client),
            ("client_incarnation", &self.client_incarnation),
            ("actor", &self.actor),
            ("action", &self.action),
            ("operation_id", &self.operation_id),
            ("effect_id", &self.effect_id),
            ("effect_sha256", &self.effect_sha256),
        ] {
            fields.insert(key, CanonicalValue::String(value.clone()));
        }
        fields.insert(
            "work_class",
            CanonicalValue::String(
                match self.work_class {
                    WorkClass::NewWork => "NewWork",
                    WorkClass::Recovery => "Recovery",
                }
                .into(),
            ),
        );
        canonical_preimage("ActivationEffectOperation/v1", &fields)
    }
    pub(super) fn digest(&self) -> String {
        raw_digest(&self.canonical_bytes()).as_str().to_owned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct EffectResult {
    pub(super) intent_id: String,
    pub(super) initial_intent_sha256: String,
    pub(super) effect_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum OperationState {
    Running,
    Succeeded,
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct OperationFact {
    pub(super) original_epoch: String,
    pub(super) state: OperationState,
    pub(super) result: Option<EffectResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ScopeStatus {
    pub(super) new_work_open: bool,
    pub(super) recovery_open: bool,
    pub(super) unresolved: u64,
    pub(super) drained: bool,
}

/// Fixed broker-side registration. Paths and draft contents never come from IPC.
struct InitialIntentEffect {
    id: String,
    digest: String,
    database: PathBuf,
    device: u64,
    inode: u64,
    draft: InitialIntentDraft,
}

impl InitialIntentEffect {
    fn bind(
        id: String,
        database: &Path,
        draft: InitialIntentDraft,
        scope: &Scope,
    ) -> Result<Self, FenceError> {
        let (namespace, unit, bytes) = draft.activation_binding();
        if namespace == "Production" {
            return Err(FenceError::ProductionRefused);
        }
        if namespace != scope.namespace || unit != scope.unit {
            return Err(FenceError::EffectMismatch);
        }
        let database = std::fs::canonicalize(database).map_err(|_| FenceError::Store)?;
        BusinessIntentStore::open(&database).map_err(|_| FenceError::Store)?;
        let meta = std::fs::metadata(&database).map_err(|_| FenceError::Store)?;
        let mut fields = scope.canonical_fields();
        fields.insert("effect_id", CanonicalValue::String(id.clone()));
        fields.insert("action", CanonicalValue::String("RecordInitial".into()));
        fields.insert("actor", CanonicalValue::String("Producer".into()));
        fields.insert(
            "store_path",
            CanonicalValue::String(database.to_str().ok_or(FenceError::Store)?.to_owned()),
        );
        fields.insert("store_device", CanonicalValue::Unsigned(meta.dev()));
        fields.insert("store_inode", CanonicalValue::Unsigned(meta.ino()));
        fields.insert(
            "draft_bytes",
            CanonicalValue::Array(
                bytes
                    .into_iter()
                    .map(|v| CanonicalValue::Unsigned(u64::from(v)))
                    .collect(),
            ),
        );
        let digest = raw_digest(&canonical_preimage(
            "ActivationInitialIntentEffect/v1",
            &fields,
        ))
        .as_str()
        .to_owned();
        Ok(Self {
            id,
            digest,
            database,
            device: meta.dev(),
            inode: meta.ino(),
            draft,
        })
    }

    fn execute(&self, context: &ExecutionContext) -> Result<EffectResult, FenceError> {
        // The non-cloneable context is owned by this worker across commit AND exact readback.
        context.before_effect()?;
        let meta = std::fs::metadata(&self.database).map_err(|_| FenceError::Store)?;
        if (meta.dev(), meta.ino()) != (self.device, self.inode) {
            return Err(FenceError::Store);
        }
        let mut store = BusinessIntentStore::open(&self.database).map_err(|_| FenceError::Store)?;
        #[cfg(test)]
        if context.hooks.effect_started_failure {
            store
                .record_initial_with_fault(
                    &self.draft,
                    super::intent_store::InitialCommitFault::BeforeCommit,
                )
                .map_err(|_| FenceError::Store)?;
            return Err(FenceError::Store);
        }
        #[cfg(test)]
        if context.hooks.business_commit_ack_lost {
            store
                .record_initial_with_fault(
                    &self.draft,
                    super::intent_store::InitialCommitFault::AfterCommitAckLost,
                )
                .map_err(|_| FenceError::Store)?;
            return Err(FenceError::Store);
        }
        store
            .record_initial(&self.draft)
            .map_err(|_| FenceError::Store)?;
        let receipt = store
            .inspect_activation_initial(&self.draft)
            .map_err(|_| FenceError::Store)?;
        Ok(EffectResult {
            intent_id: self.draft.intent_id().as_str().to_owned(),
            initial_intent_sha256: receipt,
            effect_sha256: self.digest.clone(),
        })
    }
}

struct ClientRegistration {
    client: String,
    incarnation: String,
    credential: String,
    uid: u32,
    gid: u32,
    supervisor: bool,
}

struct State {
    store: OperationStore,
    new_work_open: bool,
    recovery_open: bool,
    confirmation_blocked: bool,
    active: BTreeSet<String>,
}

pub(super) struct EffectBroker {
    scope: Scope,
    epoch: String,
    effect: Arc<InitialIntentEffect>,
    clients: Vec<ClientRegistration>,
    state: Arc<Mutex<State>>,
    changed: Arc<tokio::sync::Notify>,
    #[cfg(test)]
    hooks: TestHooks,
}

// No Clone and no Drop/Release semantics: only run() can record worker completion.
struct ExecutionContext {
    request: EffectRequest,
    state: Arc<Mutex<State>>,
    changed: Arc<tokio::sync::Notify>,
    #[cfg(test)]
    hooks: TestHooks,
}

impl ExecutionContext {
    fn before_effect(&self) -> Result<(), FenceError> {
        #[cfg(test)]
        {
            use std::io::{Read, Write};
            if let Some(path) = &self.hooks.pause_socket {
                let mut stream = std::os::unix::net::UnixStream::connect(path)
                    .map_err(|_| FenceError::Protocol)?;
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(20)))
                    .map_err(|_| FenceError::Protocol)?;
                stream
                    .set_write_timeout(Some(std::time::Duration::from_secs(20)))
                    .map_err(|_| FenceError::Protocol)?;
                stream.write_all(b"P").map_err(|_| FenceError::Protocol)?;
                let mut ack = [0];
                stream
                    .read_exact(&mut ack)
                    .map_err(|_| FenceError::Protocol)?;
                if ack != *b"G" {
                    return Err(FenceError::Protocol);
                }
            }
        }
        Ok(())
    }

    fn run(self, effect: Arc<InitialIntentEffect>) {
        let result = effect.execute(&self);
        let fact = OperationFact {
            original_epoch: self.request.broker_epoch.clone(),
            state: if result.is_ok() {
                OperationState::Succeeded
            } else {
                OperationState::Unresolved
            },
            result: result.ok(),
        };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.store.stage_result(&self.request, &fact).is_err() {
            state.confirmation_blocked = true;
            state.new_work_open = false;
            state.recovery_open = false;
            self.changed.notify_waiters();
            return;
        }
        #[cfg(test)]
        if self.hooks.result_confirmation_lost {
            // The actual candidate is durable, but confirmation was lost. No terminal fact.
            state.confirmation_blocked = true;
            state.new_work_open = false;
            state.recovery_open = false;
            self.changed.notify_waiters();
            return;
        }
        let fault = CompletionFault::None;
        #[cfg(test)]
        let fault = if self.hooks.final_result_read_failure {
            CompletionFault::FinalResultReadFailure
        } else if self.hooks.completion_write_ack_lost {
            CompletionFault::CompletionWriteAckLost
        } else {
            fault
        };
        if state.store.finish(&self.request, &fact, fault).is_err() {
            state.confirmation_blocked = true;
            state.new_work_open = false;
            state.recovery_open = false;
            self.changed.notify_waiters();
            return;
        }
        state.active.remove(&self.request.operation_id);
        if fact.state == OperationState::Unresolved {
            state.new_work_open = false;
        }
        self.changed.notify_waiters();
    }
}

impl EffectBroker {
    /// There is no production-success constructor until genuine deployment roots exist.
    pub(super) fn production() -> Result<Self, FenceError> {
        Err(FenceError::ProductionRefused)
    }

    pub(super) fn authenticate(
        &self,
        client: &str,
        incarnation: &str,
        credential: &str,
        uid: u32,
        gid: u32,
        supervisor: bool,
    ) -> Result<(), FenceError> {
        if self.clients.iter().any(|r| {
            r.client == client
                && r.incarnation == incarnation
                && r.credential == credential
                && r.uid == uid
                && r.gid == gid
                && (!supervisor || r.supervisor)
        }) {
            Ok(())
        } else {
            Err(FenceError::Unauthorized)
        }
    }

    pub(super) fn execute_current(
        &self,
        request: EffectRequest,
    ) -> Result<OperationFact, FenceError> {
        let mut state = self.state.lock().map_err(|_| FenceError::Store)?;
        // Exact repeats are queries, including after quiesce, never a second execution.
        if let Some(fact) = state.store.query(&request)? {
            return Ok(self.visible_fact(&state, &request, fact));
        }
        if request.scope != self.scope || request.broker_epoch != self.epoch {
            return Err(FenceError::Stale);
        }
        if !state.new_work_open || state.confirmation_blocked {
            return Err(FenceError::Closed);
        }
        if request.operation_id.is_empty()
            || request.operation_id.len() > 512
            || request.effect_id != self.effect.id
            || request.effect_sha256 != self.effect.digest
            || request.actor != "Producer"
            || request.action != "RecordInitial"
            || request.work_class != WorkClass::NewWork
        {
            return Err(FenceError::EffectMismatch);
        }
        if let Err(error) = state.store.register(&request) {
            state.confirmation_blocked = true;
            state.new_work_open = false;
            state.recovery_open = false;
            return Err(error);
        }
        state.active.insert(request.operation_id.clone());
        let context = ExecutionContext {
            request: request.clone(),
            state: Arc::clone(&self.state),
            changed: Arc::clone(&self.changed),
            #[cfg(test)]
            hooks: self.hooks.clone(),
        };
        let effect = Arc::clone(&self.effect);
        // Registration, gate checks and worker ownership share this one linearization point.
        if std::thread::Builder::new()
            .name("activation-initial-intent".into())
            .spawn(move || context.run(effect))
            .is_err()
        {
            state.confirmation_blocked = true;
            state.new_work_open = false;
            return Err(FenceError::Store);
        }
        Ok(OperationFact {
            original_epoch: request.broker_epoch,
            state: OperationState::Running,
            result: None,
        })
    }

    fn visible_fact(
        &self,
        state: &State,
        request: &EffectRequest,
        mut fact: OperationFact,
    ) -> OperationFact {
        if state.confirmation_blocked && state.active.contains(&request.operation_id) {
            fact.state = OperationState::Unresolved;
            fact.result = None;
        }
        fact
    }

    pub(super) fn query_operation(
        &self,
        request: &EffectRequest,
    ) -> Result<Option<OperationFact>, FenceError> {
        let state = self.state.lock().map_err(|_| FenceError::Store)?;
        Ok(state
            .store
            .query(request)?
            .map(|fact| self.visible_fact(&state, request, fact)))
    }

    pub(super) async fn query_bounded(
        &self,
        request: &EffectRequest,
    ) -> Result<Option<OperationFact>, FenceError> {
        let notification = self.changed.notified();
        tokio::pin!(notification);
        notification.as_mut().enable();
        let fact = self.query_operation(request)?;
        if fact
            .as_ref()
            .is_none_or(|fact| fact.state != OperationState::Running)
        {
            return Ok(fact);
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), notification).await;
        self.query_operation(request)
    }

    pub(super) fn quiesce(
        &self,
        scope: &Scope,
        epoch: &str,
        class: WorkClass,
    ) -> Result<ScopeStatus, FenceError> {
        if scope != &self.scope || epoch != self.epoch {
            return Err(FenceError::Stale);
        }
        let mut state = self.state.lock().map_err(|_| FenceError::Store)?;
        match class {
            WorkClass::NewWork => state.new_work_open = false,
            WorkClass::Recovery => state.recovery_open = false,
        }
        self.status(&state)
    }

    fn status(&self, state: &State) -> Result<ScopeStatus, FenceError> {
        let unresolved = state
            .store
            .unresolved(&self.scope)?
            .max(state.active.len() as u64);
        Ok(ScopeStatus {
            new_work_open: state.new_work_open,
            recovery_open: state.recovery_open,
            unresolved,
            drained: !state.new_work_open
                && !state.recovery_open
                && unresolved == 0
                && state.active.is_empty()
                && !state.confirmation_blocked,
        })
    }

    #[cfg(test)]
    pub(super) fn test_fixture(
        database: &Path,
        control: &Path,
        scope: Scope,
        epoch: String,
        draft: InitialIntentDraft,
        clients: Vec<TestClient>,
        hooks: TestHooks,
    ) -> Result<Self, FenceError> {
        if !scope.namespace.starts_with("Test:") {
            return Err(FenceError::ProductionRefused);
        }
        let effect = Arc::new(InitialIntentEffect::bind(
            "initial-intent".into(),
            database,
            draft,
            &scope,
        )?);
        let store = OperationStore::open(control, &epoch)?;
        let fresh = store.fresh;
        if hooks.registration_failure {
            store.fail_registration();
        }
        let clients = clients
            .into_iter()
            .map(|r| ClientRegistration {
                client: r.client,
                incarnation: r.incarnation,
                credential: r.credential,
                uid: r.uid,
                gid: r.gid,
                supervisor: r.supervisor,
            })
            .collect();
        Ok(Self {
            scope,
            epoch,
            effect,
            clients,
            state: Arc::new(Mutex::new(State {
                store,
                new_work_open: fresh,
                recovery_open: fresh,
                confirmation_blocked: false,
                active: BTreeSet::new(),
            })),
            changed: Arc::new(tokio::sync::Notify::new()),
            hooks,
        })
    }

    #[cfg(test)]
    pub(super) fn test_request(&self, operation: &str) -> EffectRequest {
        EffectRequest {
            scope: self.scope.clone(),
            broker_epoch: self.epoch.clone(),
            client: "producer".into(),
            client_incarnation: "client-one".into(),
            actor: "Producer".into(),
            action: "RecordInitial".into(),
            work_class: WorkClass::NewWork,
            operation_id: operation.into(),
            effect_id: self.effect.id.clone(),
            effect_sha256: self.effect.digest.clone(),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct TestHooks {
    pub(super) pause_socket: Option<PathBuf>,
    pub(super) registration_failure: bool,
    pub(super) effect_started_failure: bool,
    pub(super) result_confirmation_lost: bool,
    pub(super) final_result_read_failure: bool,
    pub(super) completion_write_ack_lost: bool,
    pub(super) business_commit_ack_lost: bool,
}

#[cfg(test)]
pub(super) struct TestClient {
    pub(super) client: String,
    pub(super) incarnation: String,
    pub(super) credential: String,
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) supervisor: bool,
}
