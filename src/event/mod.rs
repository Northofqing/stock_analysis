//! Event infrastructure — v17.1-r2 Task 1+2
//!
//! Provides the `DomainEvent` trait, `EventEnvelope` wrapper, and
//! `PushDeliveryEvent` for the event-seam infrastructure, plus a bounded
//! `EventBus` for broadcast distribution.

pub mod bus;
pub mod dispatcher;
pub mod envelope;
pub mod history;
pub mod jsonl_writer;
pub mod push_record;

pub use bus::{EventBus, EventBusMetrics, PublishOutcome, RejectReason};
pub use dispatcher::{
    AuditDispatcher, DispatchResult, Dispatcher, DispatcherRegistry, RegistryError,
};
pub use envelope::{DomainEvent, EnvelopeError, EventEnvelope, PushDeliveryEvent};
pub use jsonl_writer::{JsonlError, JsonlWriter};
pub use push_record::{PushOutcomeLabel, PushRecord, PushRecordError, ReplayablePushEvent, ReplayablePushEventError};
pub use history::{HistoryEntry, HistoryError, HistoryFilter, HistoryOrder, HistoryQuery, RateStats, Window};

// ========================================================================
// Global bus singleton
// ========================================================================

use std::sync::OnceLock;

static GLOBAL_BUS: OnceLock<EventBus> = OnceLock::new();

fn generate_event_id() -> String {
    format!(
        "{}-{:x}",
        chrono::Local::now().format("%Y%m%d%H%M%S%3f"),
        std::process::id()
    )
}

fn generate_trace_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{:x}-{:x}",
        chrono::Local::now().format("%Y%m%d%H%M%S%3f"),
        std::process::id(),
        count
    )
}

/// Obtain the global event bus, initializing it on first call.
///
/// Initialization is idempotent; subsequent calls return the already-initialized bus.
pub fn global_bus() -> &'static EventBus {
    GLOBAL_BUS.get_or_init(|| {
        let bus = EventBus::new(256);
        bus
    })
}

/// Publish a push delivery observation on the given bus (deterministic, for tests).
pub fn publish_delivery_on(
    bus: &EventBus,
    kind: &str,
    code: Option<&str>,
    outcome: &str,
    channel: &str,
    rendered_len: usize,
    latency_ms: u64,
) {
    let event = PushDeliveryEvent::new(
        kind.to_string(),
        code.map(|s| s.to_string()),
        outcome.to_string(),
        channel.to_string(),
        rendered_len,
        latency_ms,
    );
    let envelope = match EventEnvelope::from_event(
        &event,
        generate_event_id(),
        generate_trace_id(),
        chrono::Local::now(),
    ) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("[event] publish_delivery envelope error: {}", e);
            return;
        }
    };
    let outcome = bus.publish(envelope);
    if matches!(outcome, PublishOutcome::NoSubscribers) {
        log::warn!("[event] publish_delivery dropped (no subscribers): kind={}", kind);
    }
}

/// Publish a push delivery observation on the global bus.
///
/// Logs a visible warning if the global bus has not been initialized.
pub fn publish_delivery(
    kind: &str,
    code: Option<&str>,
    outcome: &str,
    channel: &str,
    rendered_len: usize,
    latency_ms: u64,
) {
    let bus = GLOBAL_BUS.get();
    match bus {
        Some(bus) => {
            publish_delivery_on(bus, kind, code, outcome, channel, rendered_len, latency_ms);
        }
        None => {
            log::warn!(
                "[event] publish_delivery called before global bus initialized: \
                 kind={}, outcome={}, channel={}",
                kind,
                outcome,
                channel
            );
        }
    }
}

// ========================================================================
// Integration test — v17.1-r2 Task 4
// ========================================================================

#[cfg(test)]
mod delivery_observation_tests {
    use super::*;

    #[tokio::test]
    async fn publish_delivery_observation_contains_actual_outcome() {
        let bus = EventBus::new_for_test(8);
        let mut rx = bus.subscribe();
        publish_delivery_on(
            &bus,
            "announcement_v1",
            Some("600519"),
            "Pushed",
            "dry_run",
            12,
            37,
        );
        let env = rx.recv().await.unwrap();
        assert_eq!(env.event_type, "push.delivery.audit");
        assert_eq!(env.payload["outcome"], "Pushed");
        assert_eq!(env.payload["code"], "600519");
    }
}
