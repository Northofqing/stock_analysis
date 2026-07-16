//! Exact-match dispatcher registry — v17.1-r2 Task 3
//!
//! Provides a `Dispatcher` trait, `DispatcherRegistry` with exact-match routing,
//! and `AuditDispatcher` for observing `push.delivery` without producing side-effects.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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

    /// The event type this dispatcher handles, e.g. `"push.delivery"`.
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

/// A dispatcher for `"push.delivery"` that only logs and increments a counter.
///
/// This dispatcher NEVER calls `push_governor_v3`, `push_wechat`, or any external
/// sink — it purely observes the delivery path for audit purposes.
#[derive(Debug)]
pub struct AuditDispatcher {
    handled_count: AtomicU64,
}

impl AuditDispatcher {
    /// Create a new `AuditDispatcher`.
    pub fn new() -> Self {
        Self {
            handled_count: AtomicU64::new(0),
        }
    }

    /// Returns the number of envelopes this dispatcher has handled.
    pub fn handled_count(&self) -> u64 {
        self.handled_count.load(Ordering::SeqCst)
    }
}

impl Default for AuditDispatcher {
    fn default() -> Self {
        Self::new()
    }
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

        // Extract fields for logging — do NOT call any sink.
        let id = &envelope.id;
        let event_type = &envelope.event_type;
        let source = &envelope.source;
        let kind = envelope.payload.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        let outcome = envelope.payload.get("outcome").and_then(|v| v.as_str()).unwrap_or("?");
        let channel = envelope.payload.get("channel").and_then(|v| v.as_str()).unwrap_or("?");
        let code = envelope
            .payload
            .get("code")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Log to stdout — this is the ONLY side-effect of AuditDispatcher.
        if let Some(ref c) = code {
            println!(
                "[AuditDispatcher] id={} event_type={} source={} kind={} outcome={} channel={} code={}",
                id, event_type, source, kind, outcome, channel, c
            );
        } else {
            println!(
                "[AuditDispatcher] id={} event_type={} source={} kind={} outcome={} channel={}",
                id, event_type, source, kind, outcome, channel
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
    use crate::event::envelope::{DomainEvent, EventEnvelope, PushDeliveryEvent};

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

        fn take_calls(&self) -> Vec<EventEnvelope> {
            self.calls.lock().unwrap().drain(..).collect()
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

    fn make_envelope(event_type: &str) -> EventEnvelope {
        EventEnvelope::from_event(
            &PushDeliveryEvent::new(
                "test_kind".into(),
                Some("600519".into()),
                "Pushed".into(),
                "wechat".into(),
                42,
                10,
            ),
            format!("evt-{}", event_type),
            "trace-1".into(),
            chrono::Local::now(),
        )
        .unwrap()
    }

    fn test_envelope_type(event_type: &str) -> EventEnvelope {
        EventEnvelope {
            id: format!("evt-{}", event_type),
            ts: chrono::Local::now(),
            trace_id: "trace-1".into(),
            source: "push_l4".into(),
            event_type: event_type.into(),
            entity_key: Some("600519".into()),
            payload: serde_json::json!({
                "kind": "test_kind",
                "code": "600519",
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
        registry.register(Arc::new(RecordingDispatcher::for_type("push.delivery.audit")));
        registry.register(Arc::new(RecordingDispatcher::for_type("push.delivery.retry")));
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
        registry.register(Arc::new(RecordingDispatcher::for_type("push.delivery.audit")));
        registry.register(Arc::new(RecordingDispatcher::for_type("push.delivery.audit")));
        assert!(registry.validate().is_err());
    }

    #[test]
    fn duplicate_error_names_the_offending_event_type() {
        let mut registry = DispatcherRegistry::new();
        registry.register(Arc::new(RecordingDispatcher::for_type("push.delivery.audit")));
        registry.register(Arc::new(RecordingDispatcher::for_type("push.delivery.audit")));
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
    fn audit_dispatcher_does_not_call_sinks() {
        use std::sync::atomic::AtomicBool;
        static CALLED: AtomicBool = AtomicBool::new(false);

        struct SinkSpy;
        impl Dispatcher for SinkSpy {
            fn name(&self) -> &'static str {
                "SinkSpy"
            }
            fn event_type(&self) -> &'static str {
                "push.delivery.audit"
            }
            fn dispatch(&self, _envelope: EventEnvelope) -> DispatchResult {
                CALLED.store(true, Ordering::SeqCst);
                DispatchResult::Handled
            }
        }

        let dispatcher = AuditDispatcher::new();
        let envelope = test_envelope_type("push.delivery.audit");
        dispatcher.dispatch(envelope);

        // The dispatcher never calls the spy — it only logs.
        assert!(!CALLED.load(Ordering::SeqCst));
    }

    #[test]
    fn audit_dispatcher_increments_counter() {
        let dispatcher = AuditDispatcher::new();
        assert_eq!(dispatcher.handled_count(), 0);

        dispatcher.dispatch(test_envelope_type("push.delivery.audit"));
        dispatcher.dispatch(test_envelope_type("push.delivery.audit"));

        assert_eq!(dispatcher.handled_count(), 2);
    }

    #[test]
    fn audit_dispatcher_rejects_non_push_delivery() {
        let dispatcher = AuditDispatcher::new();
        let envelope = test_envelope_type("announcement.new");
        let result = dispatcher.dispatch(envelope);
        assert_eq!(result, DispatchResult::Skipped("no_dispatcher".into()));
    }
}
