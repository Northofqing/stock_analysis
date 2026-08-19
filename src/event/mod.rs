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
pub use envelope::{
    DomainEvent, EnvelopeError, EventEnvelope, NewsFlashAuditSource, PushDeliveryEvent,
};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashAttemptAuditInput {
    pub push_kind: String,
    pub business_date: chrono::NaiveDate,
    pub decision_key: String,
    pub channel: String,
    pub rendered_len: usize,
    pub reservation_sha256: String,
    pub sources: Vec<NewsFlashAuditSource>,
    pub evidence_sha256: String,
    pub render_sha256: String,
    pub attempt_ordinal: u32,
    pub observed_at: chrono::DateTime<chrono::FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewsFlashTerminalDisposition {
    Accepted {
        remote_receipt: envelope::NewsFlashRemoteReceipt,
    },
    DefinitivelyRejected {
        reason_code: String,
        transport_evidence_sha256: String,
    },
    Uncertain {
        reason_code: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashTerminalAuditInput {
    pub disposition: NewsFlashTerminalDisposition,
    pub observed_at: chrono::DateTime<chrono::FixedOffset>,
    pub latency_ms: u64,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum NewsFlashDeliveryAuditError {
    #[error("invalid NewsFlash delivery audit input: {0}")]
    InvalidInput(String),
    #[error("NewsFlash delivery audit authority unavailable: {0}")]
    AuthorityUnavailable(String),
    #[error("NewsFlash delivery audit append failed: {0}")]
    AppendFailed(String),
    #[error("NewsFlash delivery audit exact reread failed: {0}")]
    ExactReadbackFailed(String),
    #[error("NewsFlash delivery audit authority already exists: {envelope_id}")]
    DuplicateAuthority { envelope_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashAttemptReceipt {
    envelope_id: String,
    persisted_at: chrono::DateTime<chrono::Local>,
    input: NewsFlashAttemptAuditInput,
    sink_attempt_identity: String,
    sink_attempt_sha256: String,
}

impl NewsFlashAttemptReceipt {
    pub fn envelope_id(&self) -> &str {
        &self.envelope_id
    }

    pub fn persisted_at(&self) -> chrono::DateTime<chrono::Local> {
        self.persisted_at
    }

    pub fn input(&self) -> &NewsFlashAttemptAuditInput {
        &self.input
    }

    pub fn sink_attempt_identity(&self) -> &str {
        &self.sink_attempt_identity
    }

    pub fn sink_attempt_sha256(&self) -> &str {
        &self.sink_attempt_sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashAcceptedReceipt {
    terminal_envelope_id: String,
    terminal_persisted_at: chrono::DateTime<chrono::Local>,
    attempt: NewsFlashAttemptReceipt,
    remote_receipt: envelope::NewsFlashRemoteReceipt,
    remote_receipt_identity: String,
    remote_receipt_sha256: String,
}

impl NewsFlashAcceptedReceipt {
    pub fn terminal_envelope_id(&self) -> &str {
        &self.terminal_envelope_id
    }

    pub fn terminal_persisted_at(&self) -> chrono::DateTime<chrono::Local> {
        self.terminal_persisted_at
    }

    pub fn attempt(&self) -> &NewsFlashAttemptReceipt {
        &self.attempt
    }

    pub fn remote_receipt(&self) -> &envelope::NewsFlashRemoteReceipt {
        &self.remote_receipt
    }

    pub fn remote_receipt_identity(&self) -> &str {
        &self.remote_receipt_identity
    }

    pub fn remote_receipt_sha256(&self) -> &str {
        &self.remote_receipt_sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashClosedTerminalReceipt {
    terminal_envelope_id: String,
    terminal_persisted_at: chrono::DateTime<chrono::Local>,
    attempt: NewsFlashAttemptReceipt,
    reason_code: String,
}

impl NewsFlashClosedTerminalReceipt {
    pub fn terminal_envelope_id(&self) -> &str {
        &self.terminal_envelope_id
    }

    pub fn attempt(&self) -> &NewsFlashAttemptReceipt {
        &self.attempt
    }

    pub fn terminal_persisted_at(&self) -> chrono::DateTime<chrono::Local> {
        self.terminal_persisted_at
    }

    pub fn reason_code(&self) -> &str {
        &self.reason_code
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewsFlashTerminalReceipt {
    Accepted(NewsFlashAcceptedReceipt),
    DefinitivelyRejected(NewsFlashClosedTerminalReceipt),
    Uncertain(NewsFlashClosedTerminalReceipt),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashFailureAuditInput {
    pub provider: Option<String>,
    pub available_provider: Option<String>,
    pub stage: String,
    pub reason_code: String,
    pub diagnostic_code: String,
    pub diagnostic: String,
    pub retryable: bool,
    pub observed_at: chrono::DateTime<chrono::FixedOffset>,
    pub source_record_count: u32,
    pub batch_id: Option<String>,
    pub record_id: Option<String>,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum NewsFlashFailureAuditError {
    #[error("invalid NewsFlash failure audit input: {0}")]
    InvalidInput(String),
    #[error("NewsFlash failure audit authority unavailable: {0}")]
    AuthorityUnavailable(String),
    #[error("NewsFlash failure audit append failed: {0}")]
    AppendFailed(String),
    #[error("NewsFlash failure audit exact reread failed: {0}")]
    ExactReadbackFailed(String),
    #[error("NewsFlash failure audit authority already exists: {envelope_id}")]
    DuplicateAuthority { envelope_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashFailureAuditReceipt {
    envelope_id: String,
    persisted_at: chrono::DateTime<chrono::Local>,
    identity_sha256: String,
}

impl NewsFlashFailureAuditReceipt {
    pub fn envelope_id(&self) -> &str {
        &self.envelope_id
    }

    pub fn identity_sha256(&self) -> &str {
        &self.identity_sha256
    }

    pub fn persisted_at(&self) -> chrono::DateTime<chrono::Local> {
        self.persisted_at
    }
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum NewsFlashReconcileError {
    #[error("NewsFlash reconcile authority unavailable: {0}")]
    AuthorityUnavailable(String),
    #[error("NewsFlash reconcile chain invalid: {0}")]
    InvalidChain(String),
    #[error("NewsFlash reconcile record conflict: {0}")]
    RecordConflict(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsFlashAuthoritySnapshot {
    business_date: chrono::NaiveDate,
    accepted_event_ids: std::collections::BTreeSet<String>,
    accepted_windows: std::collections::BTreeSet<String>,
    unresolved_reservations: std::collections::BTreeSet<String>,
    definitively_rejected_reservations: std::collections::BTreeSet<String>,
    next_attempt_ordinals: std::collections::BTreeMap<String, u32>,
}

/// Opaque capability required to construct authority snapshots in downstream
/// bin-level tests. Production code cannot construct the private field, and
/// binding fails outside a runtime test process.
#[doc(hidden)]
pub struct NewsFlashAuthoritySnapshotTestCapability {
    _private: (),
}

impl NewsFlashAuthoritySnapshotTestCapability {
    #[doc(hidden)]
    pub fn bind() -> Result<Self, NewsFlashReconcileError> {
        validate_snapshot_fixture_boundary(
            crate::risk::env_guard::runtime_is_test_process(),
            crate::risk::env_guard::current_env(),
        )?;
        Ok(Self { _private: () })
    }
}

fn validate_snapshot_fixture_boundary(
    runtime_is_test_process: bool,
    environment: crate::risk::env_guard::TradingEnv,
) -> Result<(), NewsFlashReconcileError> {
    if !runtime_is_test_process || environment != crate::risk::env_guard::TradingEnv::Test {
        return Err(NewsFlashReconcileError::AuthorityUnavailable(
            "NewsFlash authority fixture capability requires a runtime test process in Test environment"
                .into(),
        ));
    }
    Ok(())
}

impl NewsFlashAuthoritySnapshot {
    pub fn business_date(&self) -> chrono::NaiveDate {
        self.business_date
    }

    pub fn accepted_event_ids(&self) -> &std::collections::BTreeSet<String> {
        &self.accepted_event_ids
    }

    pub fn accepted_windows(&self) -> &std::collections::BTreeSet<String> {
        &self.accepted_windows
    }

    pub fn unresolved_reservations(&self) -> &std::collections::BTreeSet<String> {
        &self.unresolved_reservations
    }

    pub fn definitively_rejected_reservations(&self) -> &std::collections::BTreeSet<String> {
        &self.definitively_rejected_reservations
    }

    pub fn next_attempt_ordinal(&self, reservation_sha256: &str) -> u32 {
        self.next_attempt_ordinals
            .get(reservation_sha256)
            .copied()
            .unwrap_or(1)
    }

    /// Opaque constructor for downstream bin-level tests. It is unavailable
    /// to a production process even though the library is compiled without
    /// `cfg(test)` for bin tests.
    #[doc(hidden)]
    pub fn test_fixture(
        _capability: &NewsFlashAuthoritySnapshotTestCapability,
        business_date: chrono::NaiveDate,
        accepted_event_ids: std::collections::BTreeSet<String>,
        accepted_windows: std::collections::BTreeSet<String>,
        unresolved_reservations: std::collections::BTreeSet<String>,
        definitively_rejected_reservations: std::collections::BTreeSet<String>,
        next_attempt_ordinals: std::collections::BTreeMap<String, u32>,
    ) -> Result<Self, NewsFlashReconcileError> {
        if next_attempt_ordinals.values().any(|ordinal| *ordinal == 0) {
            return Err(NewsFlashReconcileError::RecordConflict(
                "NewsFlash authority fixture ordinal must be positive".into(),
            ));
        }
        Ok(Self {
            business_date,
            accepted_event_ids,
            accepted_windows,
            unresolved_reservations,
            definitively_rejected_reservations,
            next_attempt_ordinals,
        })
    }
}

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

fn persist_news_flash_attempt_with(
    dispatcher: &AuditDispatcher,
    input: NewsFlashAttemptAuditInput,
) -> Result<NewsFlashAttemptReceipt, NewsFlashDeliveryAuditError> {
    let event = PushDeliveryEvent::new_news_flash_attempt(
        input.push_kind.clone(),
        input.decision_key.clone(),
        input.channel.clone(),
        input.rendered_len,
        input.business_date,
        input.reservation_sha256.clone(),
        input.sources.clone(),
        input.evidence_sha256.clone(),
        input.render_sha256.clone(),
        input.attempt_ordinal,
        input.observed_at,
    );
    let envelope_id = event
        .news_flash_join_sha256
        .clone()
        .ok_or_else(|| NewsFlashDeliveryAuditError::InvalidInput("attempt join missing".into()))?;
    let sink_attempt_identity =
        event
            .news_flash_sink_attempt_identity
            .clone()
            .ok_or_else(|| {
                NewsFlashDeliveryAuditError::InvalidInput("attempt identity missing".into())
            })?;
    let sink_attempt_sha256 = event
        .news_flash_sink_attempt_sha256
        .clone()
        .ok_or_else(|| NewsFlashDeliveryAuditError::InvalidInput("attempt hash missing".into()))?;
    let envelope = EventEnvelope::from_event(
        &event,
        envelope_id.clone(),
        generate_trace_id(),
        chrono::Local::now(),
    )
    .map_err(|error| NewsFlashDeliveryAuditError::InvalidInput(error.to_string()))?;
    dispatcher
        .append_exact_news_flash_authority(&envelope)
        .map_err(map_news_flash_delivery_append_error)?;
    Ok(NewsFlashAttemptReceipt {
        envelope_id,
        persisted_at: envelope.ts,
        input,
        sink_attempt_identity,
        sink_attempt_sha256,
    })
}

fn persist_news_flash_terminal_with(
    dispatcher: &AuditDispatcher,
    attempt: &NewsFlashAttemptReceipt,
    input: NewsFlashTerminalAuditInput,
) -> Result<NewsFlashTerminalReceipt, NewsFlashDeliveryAuditError> {
    let (stage, remote_receipt, reason_code, transport_evidence_sha256) = match &input.disposition {
        NewsFlashTerminalDisposition::Accepted { remote_receipt } => (
            envelope::NewsFlashTransactionStage::Accepted,
            Some(remote_receipt.clone()),
            None,
            None,
        ),
        NewsFlashTerminalDisposition::DefinitivelyRejected {
            reason_code,
            transport_evidence_sha256,
        } => (
            envelope::NewsFlashTransactionStage::DefinitivelyRejected,
            None,
            Some(reason_code.clone()),
            Some(transport_evidence_sha256.clone()),
        ),
        NewsFlashTerminalDisposition::Uncertain { reason_code } => (
            envelope::NewsFlashTransactionStage::Uncertain,
            None,
            Some(reason_code.clone()),
            None,
        ),
    };
    let attempt_input = attempt.input();
    let event = PushDeliveryEvent::new_news_flash_terminal(
        stage,
        attempt_input.push_kind.clone(),
        attempt_input.decision_key.clone(),
        attempt_input.channel.clone(),
        attempt_input.rendered_len,
        input.latency_ms,
        attempt_input.business_date,
        attempt_input.reservation_sha256.clone(),
        attempt_input.sources.clone(),
        attempt_input.evidence_sha256.clone(),
        attempt_input.render_sha256.clone(),
        attempt_input.attempt_ordinal,
        attempt_input.observed_at,
        attempt.sink_attempt_identity.clone(),
        attempt.sink_attempt_sha256.clone(),
        attempt.envelope_id.clone(),
        remote_receipt,
        input.observed_at,
        reason_code.clone(),
        transport_evidence_sha256,
    );
    let terminal_envelope_id = event
        .news_flash_join_sha256
        .clone()
        .ok_or_else(|| NewsFlashDeliveryAuditError::InvalidInput("terminal join missing".into()))?;
    let remote_receipt_identity = event.news_flash_remote_receipt_identity.clone();
    let remote_receipt_sha256 = event.news_flash_remote_receipt_sha256.clone();
    let accepted_remote_receipt = event.news_flash_remote_receipt.clone();
    let envelope = EventEnvelope::from_event(
        &event,
        terminal_envelope_id.clone(),
        generate_trace_id(),
        chrono::Local::now(),
    )
    .map_err(|error| NewsFlashDeliveryAuditError::InvalidInput(error.to_string()))?;
    dispatcher
        .append_exact_news_flash_authority(&envelope)
        .map_err(map_news_flash_delivery_append_error)?;
    publish_persisted_delivery_observation(&envelope, "NewsFlash terminal");
    if stage == envelope::NewsFlashTransactionStage::Accepted {
        Ok(NewsFlashTerminalReceipt::Accepted(
            NewsFlashAcceptedReceipt {
                terminal_envelope_id,
                terminal_persisted_at: envelope.ts,
                attempt: attempt.clone(),
                remote_receipt: accepted_remote_receipt.ok_or_else(|| {
                    NewsFlashDeliveryAuditError::ExactReadbackFailed(
                        "accepted terminal typed remote receipt missing".into(),
                    )
                })?,
                remote_receipt_identity: remote_receipt_identity.ok_or_else(|| {
                    NewsFlashDeliveryAuditError::ExactReadbackFailed(
                        "accepted terminal remote receipt identity missing".into(),
                    )
                })?,
                remote_receipt_sha256: remote_receipt_sha256.ok_or_else(|| {
                    NewsFlashDeliveryAuditError::ExactReadbackFailed(
                        "accepted terminal remote receipt hash missing".into(),
                    )
                })?,
            },
        ))
    } else {
        let closed = NewsFlashClosedTerminalReceipt {
            terminal_envelope_id,
            terminal_persisted_at: envelope.ts,
            attempt: attempt.clone(),
            reason_code: reason_code.ok_or_else(|| {
                NewsFlashDeliveryAuditError::ExactReadbackFailed(
                    "closed terminal reason code missing".into(),
                )
            })?,
        };
        Ok(
            if stage == envelope::NewsFlashTransactionStage::DefinitivelyRejected {
                NewsFlashTerminalReceipt::DefinitivelyRejected(closed)
            } else {
                NewsFlashTerminalReceipt::Uncertain(closed)
            },
        )
    }
}

fn map_news_flash_delivery_append_error(
    error: dispatcher::ExactAuthorityAppendError,
) -> NewsFlashDeliveryAuditError {
    match error {
        dispatcher::ExactAuthorityAppendError::Duplicate { envelope_id } => {
            NewsFlashDeliveryAuditError::DuplicateAuthority { envelope_id }
        }
        dispatcher::ExactAuthorityAppendError::Verification(reason) => {
            NewsFlashDeliveryAuditError::ExactReadbackFailed(reason)
        }
        dispatcher::ExactAuthorityAppendError::Persistence(reason) => {
            NewsFlashDeliveryAuditError::AppendFailed(reason)
        }
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

/// Receipt minted only after the authoritative BR-091 dispatcher has
/// persisted and synced the exact delivery envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedDeliveryAuditReceipt {
    envelope_id: String,
    persisted_at: chrono::DateTime<chrono::Local>,
}

impl PersistedDeliveryAuditReceipt {
    pub fn envelope_id(&self) -> &str {
        &self.envelope_id
    }

    pub fn persisted_at(&self) -> chrono::DateTime<chrono::Local> {
        self.persisted_at
    }
}

/// Persist one ordinary delivery audit and return its exact durable envelope
/// identity. Unlike a governance SignalEvent ID, this ID is joinable to the
/// authoritative JSONL record.
pub fn publish_delivery_with_receipt(
    kind: &str,
    code: Option<&str>,
    outcome: &str,
    channel: &str,
    rendered_len: usize,
    latency_ms: u64,
) -> Result<PersistedDeliveryAuditReceipt, String> {
    let dispatcher = runtime_delivery_audit()?;
    publish_delivery_with_dispatcher_receipt(
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

/// Append and sync the immutable pre-sink NewsFlash attempt. The opaque
/// receipt is the only capability accepted by the terminal append API.
pub fn publish_news_flash_attempt(
    input: NewsFlashAttemptAuditInput,
) -> Result<NewsFlashAttemptReceipt, NewsFlashDeliveryAuditError> {
    let dispatcher =
        runtime_delivery_audit().map_err(NewsFlashDeliveryAuditError::AuthorityUnavailable)?;
    persist_news_flash_attempt_with(dispatcher.as_ref(), input)
}

/// Append and sync the authoritative terminal result for one exact attempt.
/// Accepted returns a NewsFlash-specific branded receipt; ordinary delivery
/// receipts cannot be promoted into this authority.
pub fn publish_news_flash_terminal(
    attempt: &NewsFlashAttemptReceipt,
    input: NewsFlashTerminalAuditInput,
) -> Result<NewsFlashTerminalReceipt, NewsFlashDeliveryAuditError> {
    let dispatcher =
        runtime_delivery_audit().map_err(NewsFlashDeliveryAuditError::AuthorityUnavailable)?;
    persist_news_flash_terminal_with(dispatcher.as_ref(), attempt, input)
}

pub fn reconcile_news_flash_business_date(
    business_date: chrono::NaiveDate,
) -> Result<NewsFlashAuthoritySnapshot, NewsFlashReconcileError> {
    let dispatcher =
        runtime_delivery_audit().map_err(NewsFlashReconcileError::AuthorityUnavailable)?;
    reconcile_news_flash_business_date_with(dispatcher.as_ref(), business_date)
}

fn reconcile_news_flash_business_date_with(
    dispatcher: &AuditDispatcher,
    business_date: chrono::NaiveDate,
) -> Result<NewsFlashAuthoritySnapshot, NewsFlashReconcileError> {
    use chrono::Datelike;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum TerminalState {
        Accepted,
        DefinitivelyRejected,
        Uncertain,
    }

    let envelopes = dispatcher
        .read_authoritative_year(business_date.year())
        .map_err(NewsFlashReconcileError::InvalidChain)?;
    let mut attempts = std::collections::BTreeMap::<String, PushRecord>::new();
    let mut terminal_by_attempt = std::collections::BTreeMap::<String, TerminalState>::new();
    let mut next_attempt_ordinals = std::collections::BTreeMap::<String, u32>::new();
    let mut accepted_event_ids = std::collections::BTreeSet::new();
    let mut accepted_windows = std::collections::BTreeSet::new();

    for envelope in envelopes {
        if envelope
            .payload
            .get("audit_schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(u64::from(
                envelope::NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION,
            ))
        {
            continue;
        }
        let record = PushRecord::try_from_authoritative(&envelope)
            .map_err(|error| NewsFlashReconcileError::InvalidChain(error.to_string()))?;
        if record.audit_schema_version != Some(envelope::NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION)
            || record.news_flash_business_date != Some(business_date)
        {
            continue;
        }
        let Some(stage) = record
            .news_flash_transaction_stage
            .as_deref()
            .and_then(envelope::NewsFlashTransactionStage::parse)
        else {
            continue;
        };
        validate_news_flash_decision_key(&record)?;
        let reservation = record
            .news_flash_reservation_sha256
            .as_deref()
            .ok_or_else(|| {
                NewsFlashReconcileError::RecordConflict("reservation hash missing".into())
            })?;
        let ordinal = record.news_flash_attempt_ordinal.ok_or_else(|| {
            NewsFlashReconcileError::RecordConflict("attempt ordinal missing".into())
        })?;
        let next = ordinal.checked_add(1).ok_or_else(|| {
            NewsFlashReconcileError::RecordConflict("attempt ordinal overflow".into())
        })?;
        next_attempt_ordinals
            .entry(reservation.to_owned())
            .and_modify(|current| *current = (*current).max(next))
            .or_insert(next);
        match stage {
            envelope::NewsFlashTransactionStage::SinkAttempt => {
                if attempts.insert(envelope.id.clone(), record).is_some() {
                    return Err(NewsFlashReconcileError::RecordConflict(
                        "duplicate sink attempt envelope".into(),
                    ));
                }
            }
            envelope::NewsFlashTransactionStage::Accepted
            | envelope::NewsFlashTransactionStage::DefinitivelyRejected
            | envelope::NewsFlashTransactionStage::Uncertain => {
                let attempt_id = record
                    .news_flash_attempt_envelope_id
                    .as_deref()
                    .ok_or_else(|| {
                        NewsFlashReconcileError::RecordConflict(
                            "terminal attempt envelope missing".into(),
                        )
                    })?;
                let attempt = attempts.get(attempt_id).ok_or_else(|| {
                    NewsFlashReconcileError::RecordConflict(format!(
                        "terminal precedes or lacks sink attempt {attempt_id}"
                    ))
                })?;
                validate_terminal_attempt_binding(attempt, &record)?;
                let terminal = match stage {
                    envelope::NewsFlashTransactionStage::Accepted => TerminalState::Accepted,
                    envelope::NewsFlashTransactionStage::DefinitivelyRejected => {
                        TerminalState::DefinitivelyRejected
                    }
                    envelope::NewsFlashTransactionStage::Uncertain => TerminalState::Uncertain,
                    envelope::NewsFlashTransactionStage::SinkAttempt => unreachable!(),
                };
                if terminal_by_attempt
                    .insert(attempt_id.to_owned(), terminal)
                    .is_some()
                {
                    return Err(NewsFlashReconcileError::RecordConflict(format!(
                        "multiple terminal records for attempt {attempt_id}"
                    )));
                }
                if terminal == TerminalState::Accepted {
                    match accepted_decision_identity(&record)? {
                        AcceptedDecisionIdentity::Event(event_id) => {
                            accepted_event_ids.insert(event_id);
                        }
                        AcceptedDecisionIdentity::Window(window) => {
                            accepted_windows.insert(window);
                        }
                    }
                }
            }
        }
    }

    let mut unresolved_reservations = std::collections::BTreeSet::new();
    let mut definitively_rejected_reservations = std::collections::BTreeSet::new();
    for (attempt_id, attempt) in attempts {
        let reservation = attempt
            .news_flash_reservation_sha256
            .expect("validated attempt reservation");
        match terminal_by_attempt.get(&attempt_id) {
            None | Some(TerminalState::Uncertain) => {
                unresolved_reservations.insert(reservation);
            }
            Some(TerminalState::DefinitivelyRejected) => {
                definitively_rejected_reservations.insert(reservation);
            }
            Some(TerminalState::Accepted) => {}
        }
    }
    for reservation in &unresolved_reservations {
        definitively_rejected_reservations.remove(reservation);
    }
    Ok(NewsFlashAuthoritySnapshot {
        business_date,
        accepted_event_ids,
        accepted_windows,
        unresolved_reservations,
        definitively_rejected_reservations,
        next_attempt_ordinals,
    })
}

enum AcceptedDecisionIdentity {
    Event(String),
    Window(String),
}

fn validate_news_flash_decision_key(record: &PushRecord) -> Result<(), NewsFlashReconcileError> {
    accepted_decision_identity(record).map(|_| ())
}

fn accepted_decision_identity(
    record: &PushRecord,
) -> Result<AcceptedDecisionIdentity, NewsFlashReconcileError> {
    let decision_key = record
        .news_flash_decision_key
        .as_deref()
        .ok_or_else(|| NewsFlashReconcileError::RecordConflict("decision key missing".into()))?;
    match record.kind.as_str() {
        "news_flash_critical_v1" => {
            if decision_key.starts_with("window:") {
                return Err(NewsFlashReconcileError::RecordConflict(
                    "critical decision key must be an event id".into(),
                ));
            }
            Ok(AcceptedDecisionIdentity::Event(decision_key.to_owned()))
        }
        "news_flash_aggregated_v1" => Ok(AcceptedDecisionIdentity::Window(accepted_window_label(
            decision_key,
        )?)),
        kind => Err(NewsFlashReconcileError::RecordConflict(format!(
            "unsupported NewsFlash push kind {kind}"
        ))),
    }
}

fn accepted_window_label(decision_key: &str) -> Result<String, NewsFlashReconcileError> {
    let label = decision_key.strip_prefix("window:").ok_or_else(|| {
        NewsFlashReconcileError::RecordConflict(
            "aggregate decision key must use window:<label>".into(),
        )
    })?;
    if !matches!(label, "09:30" | "11:30" | "13:00" | "15:00") {
        return Err(NewsFlashReconcileError::RecordConflict(format!(
            "unsupported aggregate window {label}"
        )));
    }
    Ok(label.to_owned())
}

fn validate_terminal_attempt_binding(
    attempt: &PushRecord,
    terminal: &PushRecord,
) -> Result<(), NewsFlashReconcileError> {
    if attempt.kind != terminal.kind
        || attempt.channel != terminal.channel
        || attempt.rendered_len != terminal.rendered_len
        || attempt.news_flash_business_date != terminal.news_flash_business_date
        || attempt.news_flash_decision_key != terminal.news_flash_decision_key
        || attempt.news_flash_reservation_sha256 != terminal.news_flash_reservation_sha256
        || attempt.news_flash_sources != terminal.news_flash_sources
        || attempt.news_flash_evidence_sha256 != terminal.news_flash_evidence_sha256
        || attempt.news_flash_render_sha256 != terminal.news_flash_render_sha256
        || attempt.news_flash_attempt_ordinal != terminal.news_flash_attempt_ordinal
        || attempt.news_flash_attempt_observed_at != terminal.news_flash_attempt_observed_at
        || attempt.news_flash_sink_attempt_identity != terminal.news_flash_sink_attempt_identity
        || attempt.news_flash_sink_attempt_sha256 != terminal.news_flash_sink_attempt_sha256
    {
        return Err(NewsFlashReconcileError::RecordConflict(
            "terminal does not exactly bind its sink attempt".into(),
        ));
    }
    Ok(())
}

pub fn publish_news_flash_failure(
    input: NewsFlashFailureAuditInput,
) -> Result<NewsFlashFailureAuditReceipt, NewsFlashFailureAuditError> {
    let dispatcher =
        runtime_delivery_audit().map_err(NewsFlashFailureAuditError::AuthorityUnavailable)?;
    persist_news_flash_failure_authority_with(dispatcher.as_ref(), input)
}

fn persist_news_flash_failure_authority_with(
    dispatcher: &AuditDispatcher,
    input: NewsFlashFailureAuditInput,
) -> Result<NewsFlashFailureAuditReceipt, NewsFlashFailureAuditError> {
    let event = PushDeliveryEvent::new_news_flash_failure(
        input.provider,
        input.available_provider,
        input.stage,
        input.reason_code,
        input.diagnostic_code,
        input.diagnostic,
        input.retryable,
        input.observed_at,
        input.source_record_count,
        input.batch_id,
        input.record_id,
    );
    let identity_sha256 = event
        .news_flash_failure_identity_sha256
        .clone()
        .ok_or_else(|| NewsFlashFailureAuditError::InvalidInput("identity missing".into()))?;
    let envelope = EventEnvelope::from_event(
        &event,
        identity_sha256.clone(),
        generate_trace_id(),
        chrono::Local::now(),
    )
    .map_err(|error| NewsFlashFailureAuditError::InvalidInput(error.to_string()))?;
    let record = dispatcher
        .append_exact_news_flash_authority(&envelope)
        .map_err(map_news_flash_failure_append_error)?;
    if record.news_flash_failure_identity_sha256.as_deref() != Some(identity_sha256.as_str()) {
        return Err(NewsFlashFailureAuditError::ExactReadbackFailed(
            "persisted failure identity mismatch".into(),
        ));
    }
    publish_persisted_delivery_observation(&envelope, "NewsFlash failure");
    Ok(NewsFlashFailureAuditReceipt {
        envelope_id: envelope.id,
        persisted_at: envelope.ts,
        identity_sha256,
    })
}

fn map_news_flash_failure_append_error(
    error: dispatcher::ExactAuthorityAppendError,
) -> NewsFlashFailureAuditError {
    match error {
        dispatcher::ExactAuthorityAppendError::Duplicate { envelope_id } => {
            NewsFlashFailureAuditError::DuplicateAuthority { envelope_id }
        }
        dispatcher::ExactAuthorityAppendError::Verification(reason) => {
            NewsFlashFailureAuditError::ExactReadbackFailed(reason)
        }
        dispatcher::ExactAuthorityAppendError::Persistence(reason) => {
            NewsFlashFailureAuditError::AppendFailed(reason)
        }
    }
}

fn publish_persisted_delivery_observation(envelope: &EventEnvelope, label: &str) {
    if let Some(bus) = GLOBAL_BUS.get() {
        match bus.publish(envelope.clone()) {
            PublishOutcome::Published(_) => {}
            PublishOutcome::NoSubscribers => {
                log::warn!("[event] {label} audit has no observation subscribers")
            }
            PublishOutcome::Rejected(reason) => {
                log::warn!("[event] {label} audit observation rejected: {reason:?}")
            }
        }
    } else {
        log::warn!("[event] {label} audit persisted before global bus initialization");
    }
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
    publish_delivery_with_dispatcher_receipt(
        dispatcher,
        kind,
        code,
        outcome,
        channel,
        rendered_len,
        latency_ms,
    )
    .map(|_| ())
}

fn publish_delivery_with_dispatcher_receipt(
    dispatcher: &AuditDispatcher,
    kind: &str,
    code: Option<&str>,
    outcome: &str,
    channel: &str,
    rendered_len: usize,
    latency_ms: u64,
) -> Result<PersistedDeliveryAuditReceipt, String> {
    let envelope = persist_delivery_with(
        dispatcher,
        kind,
        code,
        outcome,
        channel,
        rendered_len,
        latency_ms,
    )?;
    let receipt = PersistedDeliveryAuditReceipt {
        envelope_id: envelope.id.clone(),
        persisted_at: envelope.ts,
    };

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
    Ok(receipt)
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

    fn br244_attempt_input(label: &str) -> NewsFlashAttemptAuditInput {
        let observed_at =
            chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:01+08:00").unwrap();
        let sources = vec![NewsFlashAuditSource {
            event_id: format!("TEST_CODE_EVENT_{label}"),
            provider: "Eastmoney".to_owned(),
            source: "eastmoney-web".to_owned(),
            published_at: observed_at,
            observed_at,
            batch_id: format!("TEST_CODE_BATCH_{label}"),
        }];
        NewsFlashAttemptAuditInput {
            push_kind: "news_flash_critical_v1".to_owned(),
            business_date: chrono::NaiveDate::from_ymd_opt(2026, 8, 18).unwrap(),
            decision_key: format!("TEST_CODE_EVENT_{label}"),
            channel: "TEST_CODE_DRY_RUN".to_owned(),
            rendered_len: 44,
            reservation_sha256: "c".repeat(64),
            evidence_sha256: envelope::news_flash_evidence_sha256(&sources),
            sources,
            render_sha256: "d".repeat(64),
            attempt_ordinal: 1,
            observed_at,
        }
    }

    fn br244_failure_input() -> NewsFlashFailureAuditInput {
        NewsFlashFailureAuditInput {
            provider: Some("Jin10".to_owned()),
            available_provider: Some("Eastmoney".to_owned()),
            stage: "global_news_gateway".to_owned(),
            reason_code: "no_verified_batch".to_owned(),
            diagnostic_code: "TEST_CODE_NO_BATCH".to_owned(),
            diagnostic: "TEST_CODE provider returned no verified batch".to_owned(),
            retryable: true,
            observed_at: chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:01+08:00").unwrap(),
            source_record_count: 0,
            batch_id: None,
            record_id: None,
        }
    }

    #[test]
    fn br244_snapshot_fixture_capability_rejects_non_test_process_or_environment() {
        assert!(
            validate_snapshot_fixture_boundary(true, crate::risk::env_guard::TradingEnv::Test)
                .is_ok()
        );
        for (runtime_is_test_process, environment) in [
            (false, crate::risk::env_guard::TradingEnv::Test),
            (true, crate::risk::env_guard::TradingEnv::Prod),
            (false, crate::risk::env_guard::TradingEnv::Prod),
        ] {
            assert!(matches!(
                validate_snapshot_fixture_boundary(runtime_is_test_process, environment),
                Err(NewsFlashReconcileError::AuthorityUnavailable(_))
            ));
        }
    }

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
    fn br172_delivery_receipt_is_the_exact_persisted_envelope_id() {
        let fixture = dispatcher::TestAuditNamespace::new("BR172_RECEIPT_JOIN");
        let dispatcher = fixture.dispatcher();

        let receipt = publish_delivery_with_dispatcher_receipt(
            &dispatcher,
            "news_to_idea_v1",
            Some("TEST_CODE_BR172_JOIN"),
            "Pushed",
            "dry_run",
            77,
            9,
        )
        .expect("authoritative delivery receipt");
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", receipt.persisted_at().format("%Y")));
        let content = std::fs::read_to_string(path).expect("persisted delivery audit");
        let record: serde_json::Value =
            serde_json::from_str(content.trim()).expect("delivery audit JSONL record");

        assert_eq!(record["envelope"]["id"], receipt.envelope_id());
        assert_ne!(receipt.envelope_id(), "TEST_CODE_GOVERNANCE_SIGNAL_ID");
    }

    #[test]
    fn br244_generic_dispatch_rejects_v5_v6_while_exact_apis_append() {
        let fixture = dispatcher::TestAuditNamespace::new("BR244_EXACT_ONLY");
        let dispatcher = fixture.dispatcher();
        let attempt_input = br244_attempt_input("EXACT_ONLY");
        let attempt_event = PushDeliveryEvent::new_news_flash_attempt(
            attempt_input.push_kind.clone(),
            attempt_input.decision_key.clone(),
            attempt_input.channel.clone(),
            attempt_input.rendered_len,
            attempt_input.business_date,
            attempt_input.reservation_sha256.clone(),
            attempt_input.sources.clone(),
            attempt_input.evidence_sha256.clone(),
            attempt_input.render_sha256.clone(),
            attempt_input.attempt_ordinal,
            attempt_input.observed_at,
        );
        let attempt_envelope = EventEnvelope::from_event(
            &attempt_event,
            attempt_event.news_flash_join_sha256.clone().unwrap(),
            "TEST_CODE_GENERIC_V5_TRACE".to_owned(),
            chrono::Local::now(),
        )
        .unwrap();
        let failure_input = br244_failure_input();
        let failure_event = PushDeliveryEvent::new_news_flash_failure(
            failure_input.provider.clone(),
            failure_input.available_provider.clone(),
            failure_input.stage.clone(),
            failure_input.reason_code.clone(),
            failure_input.diagnostic_code.clone(),
            failure_input.diagnostic.clone(),
            failure_input.retryable,
            failure_input.observed_at,
            failure_input.source_record_count,
            failure_input.batch_id.clone(),
            failure_input.record_id.clone(),
        );
        let failure_envelope = EventEnvelope::from_event(
            &failure_event,
            failure_event
                .news_flash_failure_identity_sha256
                .clone()
                .unwrap(),
            "TEST_CODE_GENERIC_V6_TRACE".to_owned(),
            chrono::Local::now(),
        )
        .unwrap();
        let expected_rejection = DispatchResult::Failed(
            "BR-244 schema-v5/v6 NewsFlash authority requires the exact append API".to_owned(),
        );

        assert_eq!(
            dispatcher.dispatch(attempt_envelope),
            expected_rejection.clone()
        );
        assert_eq!(dispatcher.dispatch(failure_envelope), expected_rejection);
        assert_eq!(dispatcher.handled_count(), 0);

        let attempt = persist_news_flash_attempt_with(&dispatcher, attempt_input)
            .expect("typed exact attempt append");
        let failure = persist_news_flash_failure_authority_with(&dispatcher, failure_input)
            .expect("typed exact failure append");

        assert_ne!(attempt.envelope_id(), failure.envelope_id());
        assert_eq!(dispatcher.handled_count(), 2);
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", attempt.persisted_at().format("%Y")));
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 2);
    }

    #[test]
    fn br244_duplicate_attempt_cannot_mint_a_second_receipt() {
        let fixture = dispatcher::TestAuditNamespace::new("BR244_ATTEMPT_DUPLICATE");
        let dispatcher = fixture.dispatcher();
        let input = br244_attempt_input("DUPLICATE");

        let first = persist_news_flash_attempt_with(&dispatcher, input.clone())
            .expect("first exact attempt authority");
        let second = persist_news_flash_attempt_with(&dispatcher, input);

        assert_eq!(
            second,
            Err(NewsFlashDeliveryAuditError::DuplicateAuthority {
                envelope_id: first.envelope_id().to_owned(),
            })
        );
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", first.persisted_at().format("%Y")));
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 1);
    }

    #[test]
    fn br244_concurrent_duplicate_attempts_mint_exactly_one_receipt() {
        let fixture = dispatcher::TestAuditNamespace::new("BR244_ATTEMPT_CONCURRENT");
        let first_dispatcher = fixture.dispatcher();
        let second_dispatcher = fixture.dispatcher();
        let input = br244_attempt_input("CONCURRENT");
        let second_input = input.clone();

        let (first, second) = std::thread::scope(|scope| {
            let first =
                scope.spawn(move || persist_news_flash_attempt_with(&first_dispatcher, input));
            let second = scope
                .spawn(move || persist_news_flash_attempt_with(&second_dispatcher, second_input));
            (first.join().unwrap(), second.join().unwrap())
        });
        let (receipt, duplicate) = match (first, second) {
            (Ok(receipt), Err(duplicate)) | (Err(duplicate), Ok(receipt)) => (receipt, duplicate),
            outcomes => panic!("expected one receipt and one duplicate: {outcomes:?}"),
        };
        assert_eq!(
            duplicate,
            NewsFlashDeliveryAuditError::DuplicateAuthority {
                envelope_id: receipt.envelope_id().to_owned(),
            }
        );
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", receipt.persisted_at().format("%Y")));
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 1);
    }

    #[test]
    fn br244_failure_invalid_input_is_distinct_from_append_failure() {
        let fixture = dispatcher::TestAuditNamespace::new("BR244_FAILURE_ERRORS");
        let dispatcher = fixture.dispatcher();
        let mut invalid = br244_failure_input();
        invalid.available_provider = Some(" ".to_owned());
        assert!(matches!(
            persist_news_flash_failure_authority_with(&dispatcher, invalid),
            Err(NewsFlashFailureAuditError::InvalidInput(_))
        ));
        assert_eq!(dispatcher.handled_count(), 0);

        let unavailable = AuditDispatcher::new("TEST_CODE_RELATIVE_AUDIT_AUTHORITY");
        assert!(matches!(
            persist_news_flash_failure_authority_with(&unavailable, br244_failure_input()),
            Err(NewsFlashFailureAuditError::AppendFailed(_))
        ));
    }

    #[test]
    fn br244_news_flash_terminal_mints_only_a_source_bound_branded_receipt() {
        let fixture = dispatcher::TestAuditNamespace::new("BR244_TERMINAL_BRANDED");
        let dispatcher = fixture.dispatcher();
        let published_at =
            chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:00+08:00").unwrap();
        let attempt_at = chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:01+08:00").unwrap();
        let terminal_at =
            chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:02+08:00").unwrap();
        let sources = vec![NewsFlashAuditSource {
            event_id: "TEST_CODE_EVENT_TERMINAL".to_owned(),
            provider: "Eastmoney".to_owned(),
            source: "eastmoney-web".to_owned(),
            published_at,
            observed_at: attempt_at,
            batch_id: "TEST_CODE_BATCH_TERMINAL".to_owned(),
        }];
        let evidence_sha256 = envelope::news_flash_evidence_sha256(&sources);
        let attempt = persist_news_flash_attempt_with(
            &dispatcher,
            NewsFlashAttemptAuditInput {
                push_kind: "news_flash_aggregated_v1".to_owned(),
                business_date: chrono::NaiveDate::from_ymd_opt(2026, 8, 18).unwrap(),
                decision_key: "TEST_CODE_WINDOW_20260818T093000".to_owned(),
                channel: "TEST_CODE_DRY_RUN".to_owned(),
                rendered_len: 91,
                reservation_sha256: "a".repeat(64),
                sources,
                evidence_sha256,
                render_sha256: "b".repeat(64),
                attempt_ordinal: 1,
                observed_at: attempt_at,
            },
        )
        .expect("durable attempt authority");
        let remote = envelope::NewsFlashRemoteReceipt {
            channel: "TEST_CODE_DRY_RUN".to_owned(),
            provider: "TEST_CODE_CLI".to_owned(),
            message_id: "TEST_CODE_LOCAL_MESSAGE".to_owned(),
            platform_message_id: "TEST_CODE_REMOTE_MESSAGE".to_owned(),
            accepted_at: terminal_at,
            latency_ms: 7,
        };
        let expected_remote_identity = envelope::news_flash_remote_receipt_identity(&remote);
        let expected_remote_sha256 = envelope::news_flash_remote_receipt_sha256(&remote);
        let terminal = persist_news_flash_terminal_with(
            &dispatcher,
            &attempt,
            NewsFlashTerminalAuditInput {
                disposition: NewsFlashTerminalDisposition::Accepted {
                    remote_receipt: remote,
                },
                observed_at: terminal_at,
                latency_ms: 7,
            },
        )
        .expect("durable accepted terminal authority");
        let accepted = match terminal {
            NewsFlashTerminalReceipt::Accepted(receipt) => receipt,
            other => panic!("expected branded Accepted receipt, got {other:?}"),
        };

        assert_eq!(accepted.attempt().envelope_id(), attempt.envelope_id());
        assert_eq!(accepted.remote_receipt_identity(), expected_remote_identity);
        assert_eq!(accepted.remote_receipt_sha256(), expected_remote_sha256);
        assert_ne!(accepted.terminal_envelope_id(), attempt.envelope_id());
        let path = fixture
            .audit_path()
            .join(format!("{}.jsonl", attempt.persisted_at().format("%Y")));
        let content = std::fs::read_to_string(path).expect("read terminal audit chain");
        assert_eq!(content.lines().count(), 2);
        let terminal_row: serde_json::Value =
            serde_json::from_str(content.lines().nth(1).expect("terminal row")).unwrap();
        assert_eq!(
            terminal_row["envelope"]["id"],
            accepted.terminal_envelope_id()
        );
        assert_eq!(
            terminal_row["envelope"]["payload"]["news_flash_attempt_envelope_id"],
            attempt.envelope_id()
        );
        assert_eq!(
            terminal_row["envelope"]["payload"]["news_flash_remote_receipt_sha256"],
            expected_remote_sha256
        );
    }

    #[test]
    fn br244_news_flash_uncertain_terminal_never_mints_accepted_authority() {
        let fixture = dispatcher::TestAuditNamespace::new("BR244_TERMINAL_UNCERTAIN");
        let dispatcher = fixture.dispatcher();
        let attempt_at = chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:01+08:00").unwrap();
        let sources = vec![NewsFlashAuditSource {
            event_id: "TEST_CODE_EVENT_UNCERTAIN".to_owned(),
            provider: "Eastmoney".to_owned(),
            source: "eastmoney-web".to_owned(),
            published_at: attempt_at,
            observed_at: attempt_at,
            batch_id: "TEST_CODE_BATCH_UNCERTAIN".to_owned(),
        }];
        let attempt = persist_news_flash_attempt_with(
            &dispatcher,
            NewsFlashAttemptAuditInput {
                push_kind: "news_flash_critical_v1".to_owned(),
                business_date: chrono::NaiveDate::from_ymd_opt(2026, 8, 18).unwrap(),
                decision_key: "TEST_CODE_EVENT_UNCERTAIN".to_owned(),
                channel: "TEST_CODE_DRY_RUN".to_owned(),
                rendered_len: 44,
                reservation_sha256: "c".repeat(64),
                evidence_sha256: envelope::news_flash_evidence_sha256(&sources),
                sources,
                render_sha256: "d".repeat(64),
                attempt_ordinal: 2,
                observed_at: attempt_at,
            },
        )
        .unwrap();
        let terminal = persist_news_flash_terminal_with(
            &dispatcher,
            &attempt,
            NewsFlashTerminalAuditInput {
                disposition: NewsFlashTerminalDisposition::Uncertain {
                    reason_code: "TEST_CODE_RECEIPT_MISSING".to_owned(),
                },
                observed_at: attempt_at + chrono::Duration::seconds(1),
                latency_ms: 1,
            },
        )
        .unwrap();
        match terminal {
            NewsFlashTerminalReceipt::Uncertain(receipt) => {
                assert_eq!(receipt.attempt().envelope_id(), attempt.envelope_id());
                assert_eq!(receipt.reason_code(), "TEST_CODE_RECEIPT_MISSING");
            }
            other => panic!("Uncertain must not mint Accepted authority: {other:?}"),
        }
    }

    #[test]
    fn br244_reconcile_accepts_only_the_four_frozen_aggregate_windows() {
        for label in ["09:30", "11:30", "13:00", "15:00"] {
            assert_eq!(
                accepted_window_label(&format!("window:{label}")).unwrap(),
                label
            );
        }
        for invalid in [
            "09:29", "10:30", "11:31", "12:59", "13:30", "14:30", "15:01", "09:30", "window:",
        ] {
            assert!(matches!(
                accepted_window_label(invalid),
                Err(NewsFlashReconcileError::RecordConflict(_))
            ));
        }
    }

    #[test]
    fn br244_reconcile_restores_accepted_window_and_uncertain_reservation() {
        let fixture = dispatcher::TestAuditNamespace::new("BR244_RECONCILE");
        let dispatcher = fixture.dispatcher();
        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 8, 18).unwrap();
        let observed_at =
            chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:01+08:00").unwrap();
        dispatcher
            .persist_legacy_envelope_for_test(&EventEnvelope {
                id: "TEST_CODE_LEGACY_PREFIX".to_owned(),
                ts: chrono::Local::now(),
                trace_id: "TEST_CODE_LEGACY_TRACE".to_owned(),
                source: "push_l4".to_owned(),
                event_type: "push.delivery.audit".to_owned(),
                entity_key: None,
                payload: serde_json::json!({
                    "kind": "TEST_CODE_LEGACY_NOTICE",
                    "code": null,
                    "outcome": "Pushed",
                    "channel": "TEST_CODE_DRY_RUN",
                    "rendered_len": 1,
                    "latency_ms": 0
                }),
                version: 1,
                replay_of: None,
            })
            .expect("legal legacy authority prefix");
        let make_sources = |suffix: &str| {
            vec![NewsFlashAuditSource {
                event_id: format!("TEST_CODE_EVENT_{suffix}"),
                provider: "Eastmoney".to_owned(),
                source: "eastmoney-web".to_owned(),
                published_at: observed_at,
                observed_at,
                batch_id: format!("TEST_CODE_BATCH_{suffix}"),
            }]
        };
        let aggregate_sources = make_sources("AGG");
        let aggregate = persist_news_flash_attempt_with(
            &dispatcher,
            NewsFlashAttemptAuditInput {
                push_kind: "news_flash_aggregated_v1".to_owned(),
                business_date,
                decision_key: "window:09:30".to_owned(),
                channel: "TEST_CODE_DRY_RUN".to_owned(),
                rendered_len: 80,
                reservation_sha256: "a".repeat(64),
                evidence_sha256: envelope::news_flash_evidence_sha256(&aggregate_sources),
                sources: aggregate_sources,
                render_sha256: "b".repeat(64),
                attempt_ordinal: 1,
                observed_at,
            },
        )
        .unwrap();
        persist_news_flash_terminal_with(
            &dispatcher,
            &aggregate,
            NewsFlashTerminalAuditInput {
                disposition: NewsFlashTerminalDisposition::Accepted {
                    remote_receipt: envelope::NewsFlashRemoteReceipt {
                        channel: "TEST_CODE_DRY_RUN".to_owned(),
                        provider: "TEST_CODE_CLI".to_owned(),
                        message_id: "TEST_CODE_LOCAL_AGG".to_owned(),
                        platform_message_id: "TEST_CODE_REMOTE_AGG".to_owned(),
                        accepted_at: observed_at + chrono::Duration::seconds(1),
                        latency_ms: 1,
                    },
                },
                observed_at: observed_at + chrono::Duration::seconds(1),
                latency_ms: 1,
            },
        )
        .unwrap();
        let critical_sources = make_sources("UNCERTAIN");
        let uncertain = persist_news_flash_attempt_with(
            &dispatcher,
            NewsFlashAttemptAuditInput {
                push_kind: "news_flash_critical_v1".to_owned(),
                business_date,
                decision_key: "TEST_CODE_EVENT_UNCERTAIN".to_owned(),
                channel: "TEST_CODE_DRY_RUN".to_owned(),
                rendered_len: 81,
                reservation_sha256: "c".repeat(64),
                evidence_sha256: envelope::news_flash_evidence_sha256(&critical_sources),
                sources: critical_sources,
                render_sha256: "d".repeat(64),
                attempt_ordinal: 3,
                observed_at,
            },
        )
        .unwrap();
        persist_news_flash_terminal_with(
            &dispatcher,
            &uncertain,
            NewsFlashTerminalAuditInput {
                disposition: NewsFlashTerminalDisposition::Uncertain {
                    reason_code: "TEST_CODE_RECEIPT_MISSING".to_owned(),
                },
                observed_at: observed_at + chrono::Duration::seconds(1),
                latency_ms: 1,
            },
        )
        .unwrap();

        let snapshot = reconcile_news_flash_business_date_with(&dispatcher, business_date).unwrap();
        assert_eq!(
            snapshot
                .accepted_windows()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["09:30"]
        );
        assert!(snapshot.accepted_event_ids().is_empty());
        assert!(snapshot.unresolved_reservations().contains(&"c".repeat(64)));
        assert_eq!(snapshot.next_attempt_ordinal(&"c".repeat(64)), 4);
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
