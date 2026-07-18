//! Event infrastructure — v17.1-r2 Task 1+2
//!
//! Provides the `DomainEvent` trait, `EventEnvelope` wrapper, and
//! `PushDeliveryEvent` for the event-seam infrastructure, plus a bounded
//! `EventBus` for broadcast distribution.

pub mod bus;
pub mod cli;
pub mod dispatcher;
pub mod envelope;
pub mod history;
pub mod jsonl_writer;
pub mod push_record;
pub mod replay;

pub use bus::{EventBus, EventBusMetrics, PublishOutcome, RejectReason};
pub use cli::{CliError, EventCommand};
pub use dispatcher::{
    AuditDispatcher, DispatchResult, Dispatcher, DispatcherRegistry, RegistryError,
};
pub use envelope::{DomainEvent, EnvelopeError, EventEnvelope, PushDeliveryEvent};
pub use history::{
    format_history_lines, HistoryEntry, HistoryError, HistoryFilter, HistoryOrder, HistoryQuery,
    RateStats, Window,
};
pub use jsonl_writer::{JsonlError, JsonlWriter};
pub use push_record::{
    PushOutcomeLabel, PushRecord, PushRecordError, ReplayablePushEvent, ReplayablePushEventError,
};
pub use replay::{ReplayError, ReplayPublishError, ReplayPublisher, ReplayRunner, ReplaySummary};

// ========================================================================
// Global bus singleton
// ========================================================================

use std::sync::OnceLock;

static GLOBAL_BUS: OnceLock<EventBus> = OnceLock::new();

fn generate_event_id() -> String {
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
    GLOBAL_BUS.get_or_init(|| EventBus::new(256))
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
        log::warn!(
            "[event] publish_delivery dropped (no subscribers): kind={}",
            kind
        );
    }
}

/// Persist one delivery envelope through the BR-091 authoritative dispatcher.
/// Returning `Ok` proves the hash-chain record has been appended and synced.
pub fn persist_delivery_with(
    dispatcher: &AuditDispatcher,
    kind: &str,
    code: Option<&str>,
    outcome: &str,
    channel: &str,
    rendered_len: usize,
    latency_ms: u64,
) -> Result<EventEnvelope, String> {
    let event = PushDeliveryEvent::new(
        kind.to_string(),
        code.map(str::to_string),
        outcome.to_string(),
        channel.to_string(),
        rendered_len,
        latency_ms,
    );
    let envelope = EventEnvelope::from_event(
        &event,
        generate_event_id(),
        generate_trace_id(),
        chrono::Local::now(),
    )
    .map_err(|error| format!("delivery audit envelope: {error}"))?;
    match dispatcher.dispatch(envelope.clone()) {
        DispatchResult::Handled => Ok(envelope),
        DispatchResult::Failed(error) => Err(format!("delivery audit persist: {error}")),
        DispatchResult::Skipped(reason) => {
            Err(format!("delivery audit dispatcher skipped: {reason}"))
        }
    }
}

fn runtime_delivery_audit() -> &'static AuditDispatcher {
    static DISPATCHER: OnceLock<AuditDispatcher> = OnceLock::new();
    DISPATCHER.get_or_init(AuditDispatcher::for_runtime)
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
) -> Result<(), String> {
    let envelope = persist_delivery_with(
        runtime_delivery_audit(),
        kind,
        code,
        outcome,
        channel,
        rendered_len,
        latency_ms,
    )?;

    if let Some(bus) = GLOBAL_BUS.get() {
        match bus.publish(envelope) {
            PublishOutcome::Published(_) => {}
            PublishOutcome::NoSubscribers => {
                log::warn!("[event] durable delivery audit has no observation subscribers")
            }
            PublishOutcome::Rejected(reason) => {
                log::warn!("[event] durable delivery audit observation rejected: {reason:?}")
            }
        }
    } else {
        log::warn!("[event] durable delivery audit persisted before global bus initialization");
    }
    Ok(())
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
            Some("TEST_CODE_600519"),
            "Pushed",
            "dry_run",
            12,
            37,
        );
        let env = rx.recv().await.unwrap();
        assert_eq!(env.event_type, "push.delivery.audit");
        assert_eq!(env.payload["outcome"], "Pushed");
        assert_eq!(env.payload["code"], "TEST_CODE_600519");
    }

    #[test]
    fn br091_delivery_is_durable_before_success_returns() {
        let dir = std::env::temp_dir().join(format!(
            "delivery-audit-sync-{}-{}",
            std::process::id(),
            chrono::Local::now().timestamp_nanos_opt().unwrap()
        ));
        let dispatcher = AuditDispatcher::new(&dir);

        let envelope = persist_delivery_with(
            &dispatcher,
            "announcement_v1",
            Some("TEST_CODE_AUDIT"),
            "Pushed",
            "dry_run",
            12,
            37,
        )
        .unwrap();

        assert_eq!(dispatcher.handled_count(), 1);
        let path = dir.join(format!("{}.jsonl", envelope.ts.format("%Y")));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
