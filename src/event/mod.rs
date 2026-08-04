//! Registered business rules: BR-043, BR-091, BR-111, BR-130, BR-141, BR-142, BR-160.
//! Event infrastructure — v17.1-r2 Task 1+2
//!
//! Provides the `DomainEvent` trait, `EventEnvelope` wrapper, and
//! `PushDeliveryEvent` for the event-seam infrastructure, plus a bounded
//! `EventBus` for broadcast distribution.

pub mod bus;
pub mod cli;
pub mod delivery_settlement;
pub mod dispatcher;
pub mod durable_delivery_append;
pub mod envelope;
pub mod history;
pub mod jsonl_writer;
pub mod push_record;
pub mod replay;

pub use bus::{EventBus, EventBusMetrics, PublishOutcome, RejectReason};
pub use cli::{CliError, EventCommand};
pub use delivery_settlement::{settle, DeliverySettlement, IdentityAction};
pub use dispatcher::{
    AuditDispatcher, AuditHealth, AuditPreflightReceipt, DispatchResult, Dispatcher,
    DispatcherRegistry, RegistryError,
};
pub use durable_delivery_append::DurableDeliveryImmutableAppend;
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

use std::sync::{Arc, OnceLock};

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

/// Persist one BR-160 A-10 delivery whose redacted subject is the exact
/// immutable source batch, rather than a generic no-code subject.
#[allow(clippy::too_many_arguments)]
pub fn persist_source_batch_delivery_with(
    dispatcher: &AuditDispatcher,
    kind: &str,
    outcome: &str,
    channel: &str,
    rendered_len: usize,
    latency_ms: u64,
    source_business_date: chrono::NaiveDate,
    source_observed_at: chrono::DateTime<chrono::FixedOffset>,
    source_batch_id: &str,
    source_content_sha256: &str,
) -> Result<EventEnvelope, String> {
    let event = PushDeliveryEvent::new_source_batch(
        kind.to_owned(),
        outcome.to_owned(),
        channel.to_owned(),
        rendered_len,
        latency_ms,
        source_business_date,
        source_observed_at,
        source_batch_id.to_owned(),
        source_content_sha256.to_owned(),
    );
    let envelope = EventEnvelope::from_event(
        &event,
        generate_event_id(),
        generate_trace_id(),
        chrono::Local::now(),
    )
    .map_err(|error| format!("source batch delivery audit envelope: {error}"))?;
    match dispatcher.dispatch(envelope.clone()) {
        DispatchResult::Handled => Ok(envelope),
        DispatchResult::Failed(error) => {
            Err(format!("source batch delivery audit persist: {error}"))
        }
        DispatchResult::Skipped(reason) => Err(format!(
            "source batch delivery audit dispatcher skipped: {reason}"
        )),
    }
}

/// Persist and publish one BR-192 counted-delivery audit whose stable envelope
/// identity is already bound to the artifact, sink result and receipt hashes.
///
/// `Ok` is returned only after the authoritative BR-091 hash-chain append has
/// completed. The event-bus publication remains an observation projection.
pub fn publish_counted_delivery_with(
    dispatcher: &AuditDispatcher,
    event: PushDeliveryEvent,
    event_id: String,
    trace_id: String,
) -> Result<EventEnvelope, String> {
    if event.audit_schema_version != envelope::COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION {
        return Err("counted delivery audit requires schema v3".to_owned());
    }
    if event.counted_join_hash.as_deref() != Some(event_id.as_str()) {
        return Err(
            "counted delivery audit event_id must equal the canonical counted_join_hash".to_owned(),
        );
    }
    let envelope = EventEnvelope::from_event(&event, event_id, trace_id, chrono::Local::now())
        .map_err(|error| format!("counted delivery audit envelope: {error}"))?;
    match dispatcher.dispatch(envelope.clone()) {
        DispatchResult::Handled => {}
        DispatchResult::Failed(error) => {
            return Err(format!("counted delivery audit persist: {error}"));
        }
        DispatchResult::Skipped(reason) => {
            return Err(format!(
                "counted delivery audit dispatcher skipped: {reason}"
            ));
        }
    }
    if let Some(bus) = GLOBAL_BUS.get() {
        match bus.publish(envelope.clone()) {
            PublishOutcome::Published(_) => {}
            PublishOutcome::NoSubscribers => {
                log::warn!("[event] counted delivery audit has no observation subscribers")
            }
            PublishOutcome::Rejected(reason) => {
                log::warn!("[event] counted delivery audit observation rejected: {reason:?}")
            }
        }
    } else {
        log::warn!("[event] counted delivery audit persisted before global bus initialization");
    }
    Ok(envelope)
}

fn delivery_audit_capabilities(
) -> &'static std::sync::Mutex<std::collections::BTreeMap<String, Arc<AuditDispatcher>>> {
    static CAPABILITIES: OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, Arc<AuditDispatcher>>>,
    > = OnceLock::new();
    CAPABILITIES.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()))
}

pub fn bind_production_delivery_audit() -> Result<Arc<AuditDispatcher>, String> {
    bind_delivery_audit_capability("production", AuditDispatcher::for_production)
}

pub fn bind_test_delivery_audit(test_code: &str) -> Result<Arc<AuditDispatcher>, String> {
    let key = format!("test:{test_code}");
    bind_delivery_audit_capability(&key, || AuditDispatcher::for_test_code(test_code))
}

fn bind_delivery_audit_capability<F>(key: &str, factory: F) -> Result<Arc<AuditDispatcher>, String>
where
    F: FnOnce() -> Result<AuditDispatcher, String>,
{
    let mut capabilities = delivery_audit_capabilities()
        .lock()
        .map_err(|_| "delivery audit capability registry mutex poisoned".to_owned())?;
    if let Some(capability) = capabilities.get(key) {
        return Ok(Arc::clone(capability));
    }
    let capability = Arc::new(factory()?);
    capabilities.insert(key.to_owned(), Arc::clone(&capability));
    Ok(capability)
}

fn runtime_delivery_audit() -> Result<Arc<AuditDispatcher>, String> {
    if std::env::var_os("EVENT_AUDIT_DIR").is_some() {
        return Err("BR-192 EVENT_AUDIT_DIR override is forbidden".to_owned());
    }
    if crate::risk::env_guard::current_env() == crate::risk::env_guard::TradingEnv::Test
        || crate::risk::env_guard::runtime_is_test_process()
    {
        let test_code = std::env::var("DURABLE_DELIVERY_TEST_CODE").map_err(|_| {
            "BR-192 test delivery audit requires DURABLE_DELIVERY_TEST_CODE".to_owned()
        })?;
        bind_test_delivery_audit(&test_code)
    } else {
        bind_production_delivery_audit()
    }
}

/// Startup gate for any process that may emit delivery records.
/// This is intentionally synchronous so callers can place it behind
/// `spawn_blocking` and fail closed before starting ordinary sinks.
pub fn preflight_runtime_delivery_audit() -> Result<AuditPreflightReceipt, String> {
    runtime_delivery_audit()?.preflight()
}

pub fn runtime_delivery_audit_health() -> AuditHealth {
    runtime_delivery_audit()
        .map(|dispatcher| dispatcher.health())
        .unwrap_or_else(|error| AuditHealth::Degraded { reason_code: error })
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
    let dispatcher = runtime_delivery_audit()?;
    publish_delivery_with_dispatcher(
        dispatcher.as_ref(),
        kind,
        code,
        outcome,
        channel,
        rendered_len,
        latency_ms,
    )
}

/// Persist and publish one BR-160 A-10 source-batch-bound delivery audit.
#[allow(clippy::too_many_arguments)]
pub fn publish_source_batch_delivery(
    kind: &str,
    outcome: &str,
    channel: &str,
    rendered_len: usize,
    latency_ms: u64,
    source_business_date: chrono::NaiveDate,
    source_observed_at: chrono::DateTime<chrono::FixedOffset>,
    source_batch_id: &str,
    source_content_sha256: &str,
) -> Result<(), String> {
    let dispatcher = runtime_delivery_audit()?;
    publish_source_batch_delivery_with_dispatcher(
        dispatcher.as_ref(),
        kind,
        outcome,
        channel,
        rendered_len,
        latency_ms,
        source_business_date,
        source_observed_at,
        source_batch_id,
        source_content_sha256,
    )
}

fn publish_delivery_with_dispatcher(
    dispatcher: &AuditDispatcher,
    kind: &str,
    code: Option<&str>,
    outcome: &str,
    channel: &str,
    rendered_len: usize,
    latency_ms: u64,
) -> Result<(), String> {
    let envelope = persist_delivery_with(
        dispatcher,
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

#[allow(clippy::too_many_arguments)]
fn publish_source_batch_delivery_with_dispatcher(
    dispatcher: &AuditDispatcher,
    kind: &str,
    outcome: &str,
    channel: &str,
    rendered_len: usize,
    latency_ms: u64,
    source_business_date: chrono::NaiveDate,
    source_observed_at: chrono::DateTime<chrono::FixedOffset>,
    source_batch_id: &str,
    source_content_sha256: &str,
) -> Result<(), String> {
    let envelope = persist_source_batch_delivery_with(
        dispatcher,
        kind,
        outcome,
        channel,
        rendered_len,
        latency_ms,
        source_business_date,
        source_observed_at,
        source_batch_id,
        source_content_sha256,
    )?;
    if let Some(bus) = GLOBAL_BUS.get() {
        match bus.publish(envelope) {
            PublishOutcome::Published(_) => {}
            PublishOutcome::NoSubscribers => {
                log::warn!("[event] source batch delivery audit has no observation subscribers")
            }
            PublishOutcome::Rejected(reason) => {
                log::warn!("[event] source batch delivery audit observation rejected: {reason:?}")
            }
        }
    } else {
        log::warn!(
            "[event] source batch delivery audit persisted before global bus initialization"
        );
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
        let mut rx = bus.subscribe().expect("subscribe delivery observer");
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
        assert!(env.payload.get("code").is_none());
        assert!(env.entity_key.is_none());
        assert_eq!(env.payload["audit_schema_version"], 2);
    }

    #[test]
    fn br091_delivery_is_durable_before_success_returns() {
        let fixture = dispatcher::TestAuditNamespace::new("SYNC_DELIVERY");
        let dispatcher = fixture.dispatcher();

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
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", envelope.ts.format("%Y")));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 1);
    }

    #[test]
    fn br160_source_batch_delivery_is_durable_with_exact_lineage() {
        let fixture = dispatcher::TestAuditNamespace::new("SOURCE_BATCH_DELIVERY");
        let dispatcher = fixture.dispatcher();
        let observed_at = chrono::DateTime::parse_from_rfc3339("2026-07-31T15:01:02+08:00")
            .expect("valid source observation");

        let envelope = persist_source_batch_delivery_with(
            &dispatcher,
            "catalyst_review_v1",
            "Pushed",
            "TEST_CODE_DRY_RUN",
            12,
            37,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
            observed_at,
            "chain-batch:TEST_CODE_A10_PERSISTED",
            &"a".repeat(64),
        )
        .expect("source batch delivery audit persistence");

        assert_eq!(dispatcher.handled_count(), 1);
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", envelope.ts.format("%Y")));
        let content = std::fs::read_to_string(path).expect("read source batch audit");
        let persisted: serde_json::Value =
            serde_json::from_str(content.lines().next().expect("one source batch audit row"))
                .expect("parse source batch audit");
        let payload = &persisted["envelope"]["payload"];
        assert_eq!(payload["audit_schema_version"], 4);
        assert_eq!(
            payload["source_batch_id"],
            "chain-batch:TEST_CODE_A10_PERSISTED"
        );
        assert_eq!(payload["source_content_sha256"], "a".repeat(64));
        assert_eq!(payload["source_business_date"], "2026-07-31");
        assert_eq!(payload["source_as_of"], observed_at.to_rfc3339());
        assert!(payload["rule_ids"]
            .as_array()
            .expect("rule IDs")
            .iter()
            .any(|rule| rule == "BR-160"));
        let record = PushRecord::try_from_authoritative(&envelope)
            .expect("persisted source batch audit remains authoritative");
        assert_eq!(
            record.source_batch_id.as_deref(),
            Some("chain-batch:TEST_CODE_A10_PERSISTED")
        );
    }

    #[test]
    fn br192_counted_delivery_is_persisted_before_success_returns() {
        let fixture = dispatcher::TestAuditNamespace::new("COUNTED_DELIVERY");
        let dispatcher = fixture.dispatcher();
        let event = PushDeliveryEvent::new_counted(
            "HoldingEvent".into(),
            "holding_event_v1".into(),
            "Pushed".into(),
            "TEST_CODE_DRY_RUN".into(),
            12,
            37,
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            "e".repeat(64),
        );
        let event_id = event
            .counted_join_hash
            .clone()
            .expect("counted event has a canonical envelope id");

        let envelope = publish_counted_delivery_with(
            &dispatcher,
            event,
            event_id,
            "TEST_CODE_COUNTED_TRACE".into(),
        )
        .expect("counted audit persistence");

        assert_eq!(dispatcher.handled_count(), 1);
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", envelope.ts.format("%Y")));
        let content = std::fs::read_to_string(&path).expect("read counted audit");
        let persisted: serde_json::Value =
            serde_json::from_str(content.lines().next().expect("one audit row"))
                .expect("parse counted audit");
        assert_eq!(persisted["envelope"]["event_type"], "push.delivery.audit");
        assert_eq!(
            persisted["envelope"]["payload"]["audit_schema_version"],
            envelope::COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION
        );
        assert_eq!(
            persisted["envelope"]["payload"]["attempt_identity_hash"],
            "b".repeat(64)
        );
        dispatcher
            .verify_exact_counted_event(&envelope)
            .expect("exact counted audit terminal verification");
    }

    #[test]
    fn br192_counted_publisher_rejects_multiple_event_ids_for_one_join() {
        let fixture = dispatcher::TestAuditNamespace::new("COUNTED_EVENT_ID_BINDING");
        let dispatcher = fixture.dispatcher();
        let event = PushDeliveryEvent::new_counted(
            "HoldingEvent".into(),
            "holding_event_v1".into(),
            "Pushed".into(),
            "TEST_CODE_DRY_RUN".into(),
            12,
            37,
            "1".repeat(64),
            "2".repeat(64),
            "3".repeat(64),
            "4".repeat(64),
            "5".repeat(64),
        );
        let canonical_id = event
            .counted_join_hash
            .clone()
            .expect("counted event has a canonical envelope id");

        for alternate_id in [
            "TEST_CODE_ALTERNATE_COUNTED_EVENT_A",
            "TEST_CODE_ALTERNATE_COUNTED_EVENT_B",
        ] {
            let error = publish_counted_delivery_with(
                &dispatcher,
                event.clone(),
                alternate_id.to_owned(),
                format!("TEST_CODE_TRACE_{alternate_id}"),
            )
            .expect_err("one counted join must not admit an alternate event id");
            assert!(error.contains("event_id must equal"));
        }
        assert_eq!(
            dispatcher.handled_count(),
            0,
            "fail-fast ID rejection must occur before authoritative persistence"
        );

        let envelope = publish_counted_delivery_with(
            &dispatcher,
            event,
            canonical_id,
            "TEST_CODE_CANONICAL_COUNTED_TRACE".into(),
        )
        .expect("the canonical counted event id remains writable");
        dispatcher
            .verify_exact_counted_event(&envelope)
            .expect("the canonical event is unique and exact");
        assert_eq!(dispatcher.handled_count(), 1);
    }

    #[test]
    fn br192_counted_terminal_verifier_rejects_duplicate_event_identity() {
        let fixture = dispatcher::TestAuditNamespace::new("COUNTED_DUPLICATE");
        let dispatcher = fixture.dispatcher();
        let event = PushDeliveryEvent::new_counted(
            "HoldingEvent".into(),
            "holding_event_v1".into(),
            "Pushed".into(),
            "TEST_CODE_DRY_RUN".into(),
            12,
            37,
            "1".repeat(64),
            "2".repeat(64),
            "3".repeat(64),
            "4".repeat(64),
            "5".repeat(64),
        );
        let event_id = event.counted_join_hash.clone().unwrap();
        let first = publish_counted_delivery_with(
            &dispatcher,
            event.clone(),
            event_id.clone(),
            "TEST_CODE_DUPLICATE_TRACE".into(),
        )
        .expect("first counted audit");
        assert_eq!(
            dispatcher.dispatch(first.clone()),
            DispatchResult::Handled,
            "duplicate append is visible to terminal verifier"
        );

        let error = dispatcher
            .verify_exact_counted_event(&first)
            .expect_err("duplicate counted event ID must fail terminal verification");
        assert!(
            error.contains("exact=2") && error.contains("mismatched=0"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn br130_global_delivery_uses_production_audit_type_and_reaches_observers() {
        let bus = global_bus();
        let mut receiver = bus.subscribe().expect("subscribe global delivery observer");
        let fixture = dispatcher::TestAuditNamespace::new("GLOBAL_OBSERVER");
        let dispatcher = fixture.dispatcher();

        publish_delivery_with_dispatcher(
            &dispatcher,
            "announcement_v1",
            Some("TEST_CODE_GLOBAL_AUDIT"),
            "SinkError",
            "dry_run",
            12,
            37,
        )
        .unwrap();

        let envelope = receiver.recv().await.unwrap();
        assert_eq!(envelope.event_type, "push.delivery.audit");
        assert_eq!(envelope.payload["outcome"], "SinkError");
        assert!(
            publish_delivery_with_dispatcher(&dispatcher, "", None, "Pushed", "dry_run", 0, 0,)
                .is_err()
        );

        let local = EventBus::new_for_test(1);
        publish_delivery_on(&local, "announcement_v1", None, "Pushed", "dry_run", 1, 1);
    }
}
