//! Registered business rules: BR-043, BR-091, BR-111, BR-130, BR-141.
//! Exact-match dispatcher registry — v17.1-r2 Task 3
//!
//! Provides a `Dispatcher` trait, `DispatcherRegistry` with exact-match routing,
//! and `AuditDispatcher` for observing `push.delivery.audit` without producing side-effects.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use super::envelope::EventEnvelope;

// ========================================================================
// DispatchResult
// ========================================================================

/// Result of a dispatcher handling an envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchResult {
    /// The dispatcher handled the event.
    Handled,
    /// No dispatcher was registered for this event type.
    Skipped(String),
    /// The dispatcher encountered a failure.
    Failed(String),
}

// ========================================================================
// RegistryError
// ========================================================================

/// Errors from `DispatcherRegistry::validate`.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    #[error("duplicate event_type registered: {0}")]
    DuplicateEventType(String),
}

// ========================================================================
// Dispatcher trait
// ========================================================================

/// Trait implemented by event handlers that can be registered in the registry.
///
/// Each dispatcher handles one specific `event_type` and is selected by exact
/// equality — NOT prefix matching.
pub trait Dispatcher: Send + Sync {
    /// Human-readable name of this dispatcher.
    fn name(&self) -> &'static str;

    /// The event type this dispatcher handles, e.g. `"push.delivery.audit"`.
    fn event_type(&self) -> &'static str;

    /// Returns true when this dispatcher can handle the given envelope.
    ///
    /// The default implementation uses exact equality on `event_type`.
    fn accepts(&self, envelope: &EventEnvelope) -> bool {
        self.event_type() == envelope.event_type
    }

    /// Handle the envelope.
    fn dispatch(&self, envelope: EventEnvelope) -> DispatchResult;
}

// ========================================================================
// DispatcherRegistry
// ========================================================================

/// A registry of dispatchers selected by exact `event_type` match.
///
/// Iteration order is registration order; the first dispatcher whose
/// `event_type` matches is used.
#[derive(Default)]
pub struct DispatcherRegistry {
    dispatchers: Vec<Arc<dyn Dispatcher>>,
}

impl DispatcherRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            dispatchers: Vec::new(),
        }
    }

    /// Register a dispatcher.
    ///
    /// Duplicates are not rejected immediately — call `validate()` to check.
    pub fn register(&mut self, dispatcher: Arc<dyn Dispatcher>) {
        self.dispatchers.push(dispatcher);
    }

    /// Validate that no two dispatchers share the same `event_type`.
    ///
    /// # Errors
    ///
    /// Returns `RegistryError::DuplicateEventType` if a duplicate is found.
    pub fn validate(&self) -> Result<(), RegistryError> {
        let mut seen: HashSet<&'static str> = HashSet::new();
        for d in &self.dispatchers {
            let et = d.event_type();
            if !seen.insert(et) {
                return Err(RegistryError::DuplicateEventType(et.to_string()));
            }
        }
        Ok(())
    }

    /// Dispatch an envelope to the first registered handler with a matching
    /// `event_type`.
    ///
    /// Returns `DispatchResult::Skipped("no_dispatcher")` when no handler
    /// matches.
    pub fn dispatch(&self, envelope: EventEnvelope) -> DispatchResult {
        for d in &self.dispatchers {
            if d.accepts(&envelope) {
                return d.dispatch(envelope);
            }
        }
        DispatchResult::Skipped("no_dispatcher".into())
    }
}

// ========================================================================
// AuditDispatcher
// ========================================================================

/// BR-091 durable audit dispatcher for `push.delivery.audit`.
#[derive(Debug)]
pub struct AuditDispatcher {
    handled_count: AtomicU64,
    base_dir: PathBuf,
    chain_state: Mutex<AuditChainState>,
}

#[derive(Debug, Default)]
struct AuditChainState {
    poisoned: Option<String>,
}

impl AuditDispatcher {
    /// Create an audit dispatcher rooted at an explicit durable directory.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            handled_count: AtomicU64::new(0),
            base_dir: base_dir.into(),
            chain_state: Mutex::new(AuditChainState::default()),
        }
    }

    /// Runtime constructor with BR-051 test/prod path isolation.
    pub fn for_runtime() -> Self {
        let runtime_test = crate::risk::env_guard::runtime_is_test_process();
        let environment_test =
            crate::risk::env_guard::current_env() == crate::risk::env_guard::TradingEnv::Test;
        let base_dir = match std::env::var("EVENT_AUDIT_DIR").ok().map(PathBuf::from) {
            Some(base) => namespace_event_audit_override(base, runtime_test || environment_test),
            None => {
                if runtime_test {
                    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
                    std::env::temp_dir().join(format!(
                        "stock-analysis-event-audit-test-{}-{}",
                        std::process::id(),
                        SEQUENCE.fetch_add(1, Ordering::Relaxed)
                    ))
                } else if environment_test {
                    PathBuf::from("data/test/event_audit")
                } else {
                    PathBuf::from("data/event_audit")
                }
            }
        };
        Self::new(base_dir)
    }

    /// Returns the number of envelopes this dispatcher has handled.
    pub fn handled_count(&self) -> u64 {
        self.handled_count.load(Ordering::SeqCst)
    }

    fn persist(&self, envelope: &EventEnvelope) -> Result<(), String> {
        use fs2::FileExt;
        use sha2::{Digest, Sha256};

        fs::create_dir_all(&self.base_dir)
            .map_err(|error| format!("create {}: {error}", self.base_dir.display()))?;
        let year = envelope.ts.format("%Y").to_string();
        let path = self.base_dir.join(format!("{year}.jsonl"));
        let lock_path = self.base_dir.join(format!("{year}.lock"));
        let mut state = self
            .chain_state
            .lock()
            .map_err(|_| "audit chain state lock poisoned".to_string())?;
        if let Some(reason) = state.poisoned.as_deref() {
            return Err(format!(
                "audit chain is poisoned after an earlier persistence failure: {reason}"
            ));
        }

        let persistence_result = (|| -> Result<(), String> {
            let lock_file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lock_path)
                .map_err(|error| format!("open audit lock {}: {error}", lock_path.display()))?;
            FileExt::lock_exclusive(&lock_file)
                .map_err(|error| format!("lock audit {}: {error}", lock_path.display()))?;

            // The kernel lock spans full-chain validation, append and fsync.
            // Revalidate on every append because another monitor process may
            // have extended the chain since this dispatcher last wrote.
            let previous_hash =
                validate_existing_chain(&path)?.unwrap_or_else(|| "GENESIS".to_string());
            let mut record = serde_json::json!({
                "envelope": envelope,
                "previous_hash": previous_hash,
            });
            let canonical = serde_json::to_vec(&record)
                .map_err(|error| format!("serialize audit record: {error}"))?;
            let record_hash = format!("{:x}", Sha256::digest(&canonical));
            record.as_object_mut().expect("json object literal").insert(
                "record_hash".to_string(),
                serde_json::Value::String(record_hash.clone()),
            );
            let mut line = serde_json::to_vec(&record)
                .map_err(|error| format!("serialize audit line: {error}"))?;
            line.push(b'\n');

            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|error| format!("open {}: {error}", path.display()))?;
            file.write_all(&line)
                .map_err(|error| format!("append {}: {error}", path.display()))?;
            file.flush()
                .map_err(|error| format!("flush {}: {error}", path.display()))?;
            file.sync_data()
                .map_err(|error| format!("sync {}: {error}", path.display()))?;
            FileExt::unlock(&lock_file)
                .map_err(|error| format!("unlock audit {}: {error}", lock_path.display()))?;
            Ok(())
        })();
        match persistence_result {
            Ok(()) => Ok(()),
            Err(error) => {
                state.poisoned = Some(error.clone());
                Err(error)
            }
        }
    }
}

fn namespace_event_audit_override(base: PathBuf, is_test: bool) -> PathBuf {
    base.join(if is_test { "test" } else { "prod" })
}

impl Default for AuditDispatcher {
    fn default() -> Self {
        Self::for_runtime()
    }
}

fn validate_existing_chain(path: &Path) -> Result<Option<String>, String> {
    use sha2::{Digest, Sha256};

    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("read existing audit {}: {error}", path.display()))?;
    if !content.is_empty() && !content.ends_with('\n') {
        return Err(format!(
            "audit {} has an incomplete trailing record",
            path.display()
        ));
    }
    let mut expected_parent = "GENESIS".to_string();
    let mut last_hash = None;
    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(format!("audit line {} is blank", index + 1));
        }
        let mut record: serde_json::Value = serde_json::from_str(line)
            .map_err(|error| format!("parse audit line {}: {error}", index + 1))?;
        let record_hash = record
            .get("record_hash")
            .and_then(serde_json::Value::as_str)
            .filter(|hash| !hash.is_empty())
            .ok_or_else(|| format!("audit line {} has no record_hash", index + 1))?
            .to_string();
        let parent = record
            .get("previous_hash")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("audit line {} has no previous_hash", index + 1))?;
        if parent != expected_parent {
            return Err(format!(
                "audit chain mismatch at line {}: expected {}, found {}",
                index + 1,
                expected_parent,
                parent
            ));
        }
        record
            .as_object_mut()
            .ok_or_else(|| format!("audit line {} is not an object", index + 1))?
            .remove("record_hash");
        let canonical = serde_json::to_vec(&record)
            .map_err(|error| format!("serialize audit line {}: {error}", index + 1))?;
        let calculated = format!("{:x}", Sha256::digest(&canonical));
        if calculated != record_hash {
            return Err(format!("audit hash mismatch at line {}", index + 1));
        }
        expected_parent = record_hash.clone();
        last_hash = Some(record_hash);
    }
    Ok(last_hash)
}

impl Dispatcher for AuditDispatcher {
    fn name(&self) -> &'static str {
        "AuditDispatcher"
    }

    fn event_type(&self) -> &'static str {
        "push.delivery.audit"
    }

    fn dispatch(&self, envelope: EventEnvelope) -> DispatchResult {
        // Reject non-matching event types (supports direct dispatch testing).
        if !self.accepts(&envelope) {
            return DispatchResult::Skipped("no_dispatcher".into());
        }

        let record = match super::push_record::PushRecord::try_from(&envelope) {
            Ok(record) => record,
            Err(error) => {
                return DispatchResult::Failed(format!("invalid delivery audit: {error}"));
            }
        };

        if let Err(error) = self.persist(&envelope) {
            return DispatchResult::Failed(error);
        }

        // Extract fields for operational logging after durable persistence.
        let id = &envelope.id;
        let event_type = &envelope.event_type;
        let source = &envelope.source;
        let outcome = envelope
            .payload
            .get("outcome")
            .and_then(|v| v.as_str())
            .expect("PushRecord validation requires string outcome");

        if let Some(ref code) = record.code {
            println!(
                "[AuditDispatcher] id={} event_type={} source={} kind={} outcome={} channel={} code={}",
                id, event_type, source, record.kind, outcome, record.channel, code
            );
        } else {
            println!(
                "[AuditDispatcher] id={} event_type={} source={} kind={} outcome={} channel={}",
                id, event_type, source, record.kind, outcome, record.channel
            );
        }

        self.handled_count.fetch_add(1, Ordering::SeqCst);
        DispatchResult::Handled
    }
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::envelope::EventEnvelope;

    /// A dispatcher that records every dispatch for inspection in tests.
    #[derive(Debug, Default)]
    struct RecordingDispatcher {
        event_type_: &'static str,
        name_: &'static str,
        calls: std::sync::Mutex<Vec<EventEnvelope>>,
    }

    impl RecordingDispatcher {
        fn for_type(event_type: &'static str) -> Self {
            Self {
                event_type_: event_type,
                name_: event_type,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl Dispatcher for RecordingDispatcher {
        fn name(&self) -> &'static str {
            self.name_
        }

        fn event_type(&self) -> &'static str {
            self.event_type_
        }

        fn dispatch(&self, envelope: EventEnvelope) -> DispatchResult {
            self.calls.lock().unwrap().push(envelope.clone());
            DispatchResult::Handled
        }
    }

    fn test_envelope_type(event_type: &str) -> EventEnvelope {
        EventEnvelope {
            id: format!("evt-{}", event_type),
            ts: chrono::Local::now(),
            trace_id: "trace-1".into(),
            source: "push_l4".into(),
            event_type: event_type.into(),
            entity_key: Some("TEST_CODE_AUDIT".into()),
            payload: serde_json::json!({
                "kind": "test_kind",
                "code": "TEST_CODE_AUDIT",
                "outcome": "Pushed",
                "channel": "wechat",
                "rendered_len": 42,
                "latency_ms": 10,
            }),
            version: 1,
            replay_of: None,
        }
    }

    #[test]
    fn registry_routes_only_exact_event_type() {
        let mut registry = DispatcherRegistry::new();
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.retry",
        )));
        registry.validate().unwrap();

        assert_eq!(
            registry.dispatch(test_envelope_type("push.delivery.audit")),
            DispatchResult::Handled
        );
        assert_eq!(
            registry.dispatch(test_envelope_type("push.delivery.retry")),
            DispatchResult::Handled
        );
        assert_eq!(
            registry.dispatch(test_envelope_type("push.delivery.retry.extra")),
            DispatchResult::Skipped("no_dispatcher".into())
        );
    }

    #[test]
    fn duplicate_exact_types_are_rejected_at_validation() {
        let mut registry = DispatcherRegistry::new();
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        assert!(registry.validate().is_err());
    }

    #[test]
    fn duplicate_error_names_the_offending_event_type() {
        let mut registry = DispatcherRegistry::new();
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        registry.register(Arc::new(RecordingDispatcher::for_type(
            "push.delivery.audit",
        )));
        let err = registry.validate().unwrap_err();
        assert!(err.to_string().contains("push.delivery.audit"));
    }

    #[test]
    fn dispatch_returns_skipped_when_no_matching_handler() {
        let registry = DispatcherRegistry::new();
        let result = registry.dispatch(test_envelope_type("unknown.event"));
        assert_eq!(result, DispatchResult::Skipped("no_dispatcher".into()));
    }

    #[test]
    fn dispatch_returns_failed_when_handler_reports_failure() {
        struct FailingDispatcher;
        impl Dispatcher for FailingDispatcher {
            fn name(&self) -> &'static str {
                "FailingDispatcher"
            }
            fn event_type(&self) -> &'static str {
                "push.delivery.audit"
            }
            fn dispatch(&self, _envelope: EventEnvelope) -> DispatchResult {
                DispatchResult::Failed("sink unavailable".into())
            }
        }

        let mut registry = DispatcherRegistry::new();
        registry.register(Arc::new(FailingDispatcher));
        let result = registry.dispatch(test_envelope_type("push.delivery.audit"));
        assert_eq!(result, DispatchResult::Failed("sink unavailable".into()));
    }

    #[test]
    fn audit_dispatcher_increments_counter() {
        let dir =
            std::env::temp_dir().join(format!("audit-dispatcher-count-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let dispatcher = AuditDispatcher::new(&dir);
        assert_eq!(dispatcher.handled_count(), 0);

        dispatcher.dispatch(test_envelope_type("push.delivery.audit"));
        dispatcher.dispatch(test_envelope_type("push.delivery.audit"));

        assert_eq!(dispatcher.handled_count(), 2);
        let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        let content = fs::read_to_string(path).unwrap();
        assert_eq!(content.lines().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn audit_dispatcher_rejects_non_push_delivery() {
        let dispatcher = AuditDispatcher::new(
            std::env::temp_dir().join(format!("audit-dispatcher-reject-{}", std::process::id())),
        );
        let envelope = test_envelope_type("announcement.new");
        let result = dispatcher.dispatch(envelope);
        assert_eq!(result, DispatchResult::Skipped("no_dispatcher".into()));
    }

    #[test]
    fn br130_audit_dispatcher_rejects_invalid_payload_before_persistence() {
        let dir = std::env::temp_dir().join(format!(
            "audit-dispatcher-invalid-payload-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap()
        ));
        let dispatcher = AuditDispatcher::new(&dir);
        let mut envelope = test_envelope_type("push.delivery.audit");
        envelope.payload["outcome"] = serde_json::json!("Unknown");

        let result = dispatcher.dispatch(envelope);

        assert!(matches!(
            result,
            DispatchResult::Failed(error) if error.contains("outcome=Unknown")
        ));
        assert_eq!(dispatcher.handled_count(), 0);
        assert!(
            !dir.exists(),
            "invalid audit must not create persistence output"
        );
    }

    #[test]
    fn audit_dispatcher_rejects_tampered_existing_chain() {
        let dir =
            std::env::temp_dir().join(format!("audit-dispatcher-tamper-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        fs::write(&path, "{not-json}\n").unwrap();
        let dispatcher = AuditDispatcher::new(&dir);

        let result = dispatcher.dispatch(test_envelope_type("push.delivery.audit"));

        assert!(
            matches!(result, DispatchResult::Failed(error) if error.contains("parse audit line 1"))
        );
        assert_eq!(dispatcher.handled_count(), 0);
        assert_eq!(fs::read_to_string(path).unwrap(), "{not-json}\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn br091_persistence_failure_poisons_followup_writes() {
        let dir =
            std::env::temp_dir().join(format!("audit-dispatcher-poison-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        fs::create_dir_all(&path).unwrap();
        let dispatcher = AuditDispatcher::new(&dir);

        let first = dispatcher.dispatch(test_envelope_type("push.delivery.audit"));
        assert!(matches!(first, DispatchResult::Failed(_)));
        assert_eq!(dispatcher.handled_count(), 0);

        fs::remove_dir_all(&path).unwrap();
        let second = dispatcher.dispatch(test_envelope_type("push.delivery.audit"));
        assert!(
            matches!(second, DispatchResult::Failed(error) if error.contains("audit chain is poisoned"))
        );
        assert_eq!(dispatcher.handled_count(), 0);
        assert!(!path.exists(), "poisoned dispatcher must not retry writing");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn br091_existing_valid_chain_is_verified_and_extended() {
        let dir = std::env::temp_dir().join(format!(
            "audit-dispatcher-resume-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap()
        ));
        let _ = fs::remove_dir_all(&dir);

        let first = AuditDispatcher::new(&dir);
        assert_eq!(
            first.dispatch(test_envelope_type("push.delivery.audit")),
            DispatchResult::Handled
        );
        drop(first);

        let second = AuditDispatcher::new(&dir);
        let mut without_code = test_envelope_type("push.delivery.audit");
        without_code.entity_key = None;
        without_code.payload["code"] = serde_json::Value::Null;
        assert_eq!(second.name(), "AuditDispatcher");
        assert_eq!(second.dispatch(without_code), DispatchResult::Handled);

        let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
        assert!(validate_existing_chain(&path).unwrap().is_some());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn br141_event_audit_override_has_physical_test_prod_namespaces() {
        let base = PathBuf::from("audit-override");
        assert_eq!(
            namespace_event_audit_override(base.clone(), true),
            base.join("test")
        );
        assert_eq!(
            namespace_event_audit_override(base.clone(), false),
            base.join("prod")
        );
    }

    #[test]
    fn br141_existing_valid_record_without_newline_is_rejected() {
        let dir = std::env::temp_dir().join(format!(
            "audit-dispatcher-tail-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap()
        ));
        let first = AuditDispatcher::new(&dir);
        assert_eq!(
            first.dispatch(test_envelope_type("push.delivery.audit")),
            DispatchResult::Handled
        );
        drop(first);

        let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        let complete = fs::read_to_string(&path).unwrap();
        fs::write(&path, complete.strip_suffix('\n').unwrap()).unwrap();
        let second = AuditDispatcher::new(&dir);
        let result = second.dispatch(test_envelope_type("push.delivery.audit"));
        assert!(
            matches!(result, DispatchResult::Failed(error) if error.contains("incomplete trailing record"))
        );
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "invoked as a child by the cross-process audit locking test"]
    fn br141_event_audit_process_writer_helper() {
        let Ok(dir) = std::env::var("BR141_EVENT_AUDIT_HELPER_DIR") else {
            return;
        };
        let identity = std::env::var("BR141_EVENT_AUDIT_HELPER_ID").unwrap();
        let mut envelope = test_envelope_type("push.delivery.audit");
        envelope.id = format!("event-audit-{identity}");
        let dispatcher = AuditDispatcher::new(dir);
        assert_eq!(dispatcher.dispatch(envelope), DispatchResult::Handled);
    }

    #[test]
    fn br141_event_audit_serializes_independent_process_writers() {
        let dir = std::env::temp_dir().join(format!(
            "audit-dispatcher-process-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap()
        ));
        let executable = std::env::current_exe().unwrap();
        let mut children = (0..4)
            .map(|index| {
                std::process::Command::new(&executable)
                    .args([
                        "--exact",
                        "event::dispatcher::tests::br141_event_audit_process_writer_helper",
                        "--ignored",
                    ])
                    .env("BR141_EVENT_AUDIT_HELPER_DIR", &dir)
                    .env("BR141_EVENT_AUDIT_HELPER_ID", format!("writer-{index}"))
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }

        let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        assert!(validate_existing_chain(&path).unwrap().is_some());
        assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 4);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn br091_existing_chain_rejects_every_structural_corruption_class() {
        let dir = std::env::temp_dir().join(format!(
            "audit-chain-invalid-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.jsonl");

        for (line, expected) in [
            ("\n", "is blank"),
            (
                "{\"previous_hash\":\"GENESIS\",\"envelope\":{}}\n",
                "no record_hash",
            ),
            (
                "{\"record_hash\":\"x\",\"envelope\":{}}\n",
                "no previous_hash",
            ),
            (
                "{\"record_hash\":\"x\",\"previous_hash\":\"WRONG\",\"envelope\":{}}\n",
                "chain mismatch",
            ),
            (
                "{\"record_hash\":\"deadbeef\",\"previous_hash\":\"GENESIS\",\"envelope\":{}}\n",
                "hash mismatch",
            ),
        ] {
            fs::write(&path, line).unwrap();
            let error = validate_existing_chain(&path).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn audit_runtime_constructor_and_directory_failure_are_explicit() {
        let runtime = AuditDispatcher::default();
        assert_eq!(runtime.name(), "AuditDispatcher");
        assert_eq!(runtime.event_type(), "push.delivery.audit");

        let base_file = std::env::temp_dir().join(format!(
            "audit-base-file-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap()
        ));
        fs::write(&base_file, "TEST_CODE not a directory").unwrap();
        let dispatcher = AuditDispatcher::new(&base_file);
        let result = dispatcher.dispatch(test_envelope_type("push.delivery.audit"));
        assert!(
            matches!(result, DispatchResult::Failed(error) if error.contains("create")),
            "directory creation failure must be visible"
        );
        assert_eq!(dispatcher.handled_count(), 0);
        fs::remove_file(base_file).unwrap();
    }
}
