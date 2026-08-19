//! Registered business rules: BR-091, BR-130, BR-142, BR-160, BR-192.
//! Event envelope contract — v17.1-r2 Task 1
//!
//! Defines `DomainEvent`, `EventEnvelope`, and `PushDeliveryEvent` for the
//! event-seam infrastructure. The `event` module must be free of monitor-bin
//! imports; only `lib.rs` consumers touch it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const DELIVERY_AUDIT_SCHEMA_VERSION: u32 = 2;
pub const COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION: u32 = 3;
pub const SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION: u32 = 4;
pub const NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION: u32 = 5;
pub const NEWS_FLASH_FAILURE_AUDIT_SCHEMA_VERSION: u32 = 6;
pub const DELIVERY_SUBJECT_HASH_DOMAIN: &str = "stock_analysis.delivery_subject.v2";
pub const DELIVERY_IDENTITY_HASH_DOMAIN: &str = "stock_analysis.delivery_identity.v2";
pub const COUNTED_DELIVERY_JOIN_HASH_DOMAIN: &str = "stock_analysis.counted_delivery_join.v1";
pub const SOURCE_BATCH_DELIVERY_SUBJECT_HASH_DOMAIN: &str =
    "stock_analysis.source_batch_delivery_subject.v1";
pub const NEWS_FLASH_EVIDENCE_HASH_DOMAIN: &str =
    "stock_analysis.news_flash_ordered_source_evidence.v1";
pub const NEWS_FLASH_DELIVERY_JOIN_HASH_DOMAIN: &str = "stock_analysis.news_flash_delivery_join.v1";
pub const NEWS_FLASH_TRANSACTION_JOIN_HASH_DOMAIN: &str =
    "stock_analysis.news_flash_transaction_join.v2";
pub const NEWS_FLASH_SINK_ATTEMPT_IDENTITY_HASH_DOMAIN: &str =
    "stock_analysis.news_flash_sink_attempt_identity.v1";
pub const NEWS_FLASH_SINK_ATTEMPT_HASH_DOMAIN: &str = "stock_analysis.news_flash_sink_attempt.v1";
pub const NEWS_FLASH_REMOTE_RECEIPT_IDENTITY_HASH_DOMAIN: &str =
    "stock_analysis.news_flash_remote_receipt_identity.v1";
pub const NEWS_FLASH_REMOTE_RECEIPT_HASH_DOMAIN: &str =
    "stock_analysis.news_flash_remote_receipt.v1";
pub const NEWS_FLASH_FAILURE_IDENTITY_HASH_DOMAIN: &str = "stock_analysis.news_flash_failure.v3";
pub const DELIVERY_AUDIT_RULE_IDS: [&str; 5] = ["2.7", "BR-091", "BR-111", "BR-130", "BR-142"];
pub const COUNTED_DELIVERY_AUDIT_RULE_IDS: [&str; 6] =
    ["2.7", "BR-091", "BR-111", "BR-130", "BR-142", "BR-192"];
pub const SOURCE_BATCH_DELIVERY_AUDIT_RULE_IDS: [&str; 6] =
    ["2.7", "BR-091", "BR-111", "BR-130", "BR-142", "BR-160"];
pub const NEWS_FLASH_DELIVERY_AUDIT_RULE_IDS: [&str; 7] = [
    "2.7", "BR-082", "BR-091", "BR-111", "BR-130", "BR-142", "BR-244",
];
pub const NEWS_FLASH_FAILURE_AUDIT_RULE_IDS: [&str; 5] =
    ["2.2", "2.7", "BR-091", "BR-142", "BR-244"];

// ========================================================================
// DomainEvent trait
// ========================================================================

/// Trait implemented by all domain events that can be wrapped in an `EventEnvelope`.
pub trait DomainEvent: Send + Sync + 'static {
    /// The event type string, e.g. `"push.delivery.audit"`.
    fn event_type(&self) -> &'static str;
    /// The source subsystem that produced this event, e.g. `"push_l4"`.
    fn source(&self) -> &'static str;
    /// Optional entity key for routing/filtering (e.g. a stock code).
    fn entity_key(&self) -> Option<&str> {
        None
    }
    /// The event payload as a JSON value.
    fn payload(&self) -> serde_json::Value;
    /// Validate the event's business invariants.
    /// Called by `EventEnvelope::from_event`; return `Err` to reject the event.
    fn validate(&self) -> Result<(), EnvelopeError> {
        Ok(())
    }
}

// ========================================================================
// EnvelopeError
// ========================================================================

/// Errors that can occur when constructing an `EventEnvelope`.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    #[error("envelope id cannot be blank")]
    BlankId,

    #[error("trace_id cannot be blank")]
    BlankTraceId,

    #[error("event_type cannot be blank")]
    BlankEventType,

    #[error("delivery kind cannot be blank")]
    BlankDeliveryKind,

    #[error("delivery outcome cannot be blank")]
    BlankDeliveryOutcome,

    #[error("unsupported delivery outcome: {0}")]
    InvalidDeliveryOutcome(String),

    #[error("delivery channel cannot be blank")]
    BlankDeliveryChannel,

    #[error("invalid delivery audit field: {0}")]
    InvalidDeliveryAuditField(String),
}

// ========================================================================
// EventEnvelope
// ========================================================================

/// A wrapper that captures any `DomainEvent` with envelope metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Unique identifier for this envelope.
    pub id: String,
    /// Wall-clock time when the event was captured.
    pub ts: chrono::DateTime<chrono::Local>,
    /// Distributed-trace identifier linking related envelopes.
    pub trace_id: String,
    /// Subsystem that produced the original event.
    pub source: String,
    /// Event type string from the wrapped event.
    pub event_type: String,
    /// Optional entity key (e.g. stock code) from the wrapped event.
    pub entity_key: Option<String>,
    /// Raw JSON payload of the original event.
    pub payload: serde_json::Value,
    /// Schema version; always 1 for now.
    pub version: u32,
    /// If this is a replay, the id of the original envelope.
    pub replay_of: Option<String>,
}

impl EventEnvelope {
    /// Wrap a `DomainEvent` in an `EventEnvelope`.
    ///
    /// # Errors
    ///
    /// Returns `EnvelopeError` if `id`, `trace_id`, `event_type`, or the
    /// delivery event's `kind` is blank.
    pub fn from_event<E: DomainEvent + serde::Serialize>(
        event: &E,
        id: String,
        trace_id: String,
        ts: chrono::DateTime<chrono::Local>,
    ) -> Result<Self, EnvelopeError> {
        if id.trim().is_empty() {
            return Err(EnvelopeError::BlankId);
        }
        if trace_id.trim().is_empty() {
            return Err(EnvelopeError::BlankTraceId);
        }
        let event_type = event.event_type();
        if event_type.trim().is_empty() {
            return Err(EnvelopeError::BlankEventType);
        }

        let payload = serde_json::to_value(event).map_err(|_| EnvelopeError::BlankEventType)?;

        event.validate()?;

        Ok(Self {
            id,
            ts,
            trace_id,
            source: event.source().to_string(),
            event_type: event_type.to_string(),
            entity_key: event.entity_key().map(|s| s.to_string()),
            payload,
            version: 1,
            replay_of: None,
        })
    }
}

// ========================================================================
// PushDeliveryEvent
// ========================================================================

/// One canonical provider record in the exact NewsFlash display order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewsFlashAuditSource {
    pub event_id: String,
    pub provider: String,
    pub source: String,
    pub published_at: chrono::DateTime<chrono::FixedOffset>,
    pub observed_at: chrono::DateTime<chrono::FixedOffset>,
    pub batch_id: String,
}

/// Receipt fields returned by the physical transport. A NewsFlash Accepted
/// audit stores the typed values so its identity/hash can be revalidated
/// independently instead of trusting a boolean or an opaque log line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NewsFlashRemoteReceipt {
    pub channel: String,
    pub provider: String,
    pub message_id: String,
    pub platform_message_id: String,
    pub accepted_at: chrono::DateTime<chrono::FixedOffset>,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsFlashTransactionStage {
    SinkAttempt,
    Accepted,
    DefinitivelyRejected,
    Uncertain,
}

impl NewsFlashTransactionStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SinkAttempt => "SinkAttempt",
            Self::Accepted => "Accepted",
            Self::DefinitivelyRejected => "DefinitivelyRejected",
            Self::Uncertain => "Uncertain",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "SinkAttempt" => Self::SinkAttempt,
            "Accepted" => Self::Accepted,
            "DefinitivelyRejected" => Self::DefinitivelyRejected,
            "Uncertain" => Self::Uncertain,
            _ => return None,
        })
    }
}

/// A domain event representing a push delivery attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushDeliveryEvent {
    pub kind: String,
    pub outcome: String,
    pub decision_status: String,
    pub retryable: bool,
    pub rule_ids: Vec<String>,
    pub reason_code: String,
    pub subject_hash: String,
    pub identity_hash: String,
    pub source_as_of: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub audit_schema_version: u32,
    pub channel: String,
    pub rendered_len: usize,
    pub latency_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_identity_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_identity_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sink_result_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counted_join_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub durable_push_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stable_template_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_business_date: Option<chrono::NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_batch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_content_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_sources: Option<Vec<NewsFlashAuditSource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_reservation_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_evidence_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_render_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_join_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_business_date: Option<chrono::NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_decision_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_transaction_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_attempt_ordinal: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_attempt_observed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_sink_attempt_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_sink_attempt_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_attempt_envelope_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_remote_receipt: Option<NewsFlashRemoteReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_remote_receipt_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_remote_receipt_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_terminal_observed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_terminal_reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_transport_evidence_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_available_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_diagnostic_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_diagnostic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_observed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_source_record_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_batch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news_flash_failure_identity_sha256: Option<String>,
}

impl PushDeliveryEvent {
    pub fn new(
        kind: String,
        code: Option<String>,
        outcome: String,
        channel: String,
        rendered_len: usize,
        latency_ms: u64,
    ) -> Self {
        let (reason_code, retryable) =
            delivery_outcome_metadata(&outcome).unwrap_or(("delivery.invalid", false));
        let subject_hash = delivery_subject_hash(code.as_deref());
        let identity_hash = delivery_identity_hash_from_subject(&kind, &subject_hash, &channel);
        Self {
            kind,
            decision_status: outcome.clone(),
            outcome,
            retryable,
            rule_ids: DELIVERY_AUDIT_RULE_IDS
                .iter()
                .map(|rule| (*rule).to_string())
                .collect(),
            reason_code: reason_code.to_string(),
            subject_hash,
            identity_hash,
            source_as_of: None,
            audit_schema_version: DELIVERY_AUDIT_SCHEMA_VERSION,
            channel,
            rendered_len,
            latency_ms,
            decision_identity_hash: None,
            attempt_identity_hash: None,
            artifact_sha256: None,
            sink_result_sha256: None,
            receipt_sha256: None,
            counted_join_hash: None,
            durable_push_kind: None,
            stable_template_id: None,
            source_business_date: None,
            source_batch_id: None,
            source_content_sha256: None,
            news_flash_sources: None,
            news_flash_reservation_sha256: None,
            news_flash_evidence_sha256: None,
            news_flash_render_sha256: None,
            news_flash_join_sha256: None,
            news_flash_business_date: None,
            news_flash_decision_key: None,
            news_flash_transaction_stage: None,
            news_flash_attempt_ordinal: None,
            news_flash_attempt_observed_at: None,
            news_flash_sink_attempt_identity: None,
            news_flash_sink_attempt_sha256: None,
            news_flash_attempt_envelope_id: None,
            news_flash_remote_receipt: None,
            news_flash_remote_receipt_identity: None,
            news_flash_remote_receipt_sha256: None,
            news_flash_terminal_observed_at: None,
            news_flash_terminal_reason_code: None,
            news_flash_transport_evidence_sha256: None,
            news_flash_failure_provider: None,
            news_flash_failure_available_provider: None,
            news_flash_failure_stage: None,
            news_flash_failure_reason: None,
            news_flash_failure_diagnostic_code: None,
            news_flash_failure_diagnostic: None,
            news_flash_failure_observed_at: None,
            news_flash_failure_source_record_count: None,
            news_flash_failure_batch_id: None,
            news_flash_failure_record_id: None,
            news_flash_failure_identity_sha256: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_counted(
        durable_push_kind: String,
        stable_template_id: String,
        outcome: String,
        channel: String,
        rendered_len: usize,
        latency_ms: u64,
        decision_identity_hash: String,
        attempt_identity_hash: String,
        artifact_sha256: String,
        sink_result_sha256: String,
        receipt_sha256: String,
    ) -> Self {
        let mut event = Self::new(
            stable_template_id.clone(),
            None,
            outcome,
            channel,
            rendered_len,
            latency_ms,
        );
        event.audit_schema_version = COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION;
        event.rule_ids = COUNTED_DELIVERY_AUDIT_RULE_IDS
            .iter()
            .map(|rule| (*rule).to_owned())
            .collect();
        event.subject_hash = decision_identity_hash.clone();
        event.identity_hash =
            delivery_identity_hash_from_subject(&event.kind, &event.subject_hash, &event.channel);
        event.counted_join_hash = Some(counted_delivery_join_hash(
            &event.kind,
            &event.outcome,
            &event.channel,
            &decision_identity_hash,
            &attempt_identity_hash,
            &artifact_sha256,
            &sink_result_sha256,
            &receipt_sha256,
        ));
        event.decision_identity_hash = Some(decision_identity_hash);
        event.attempt_identity_hash = Some(attempt_identity_hash);
        event.artifact_sha256 = Some(artifact_sha256);
        event.sink_result_sha256 = Some(sink_result_sha256);
        event.receipt_sha256 = Some(receipt_sha256);
        event.durable_push_kind = Some(durable_push_kind);
        event.stable_template_id = Some(stable_template_id);
        event
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_source_batch(
        kind: String,
        outcome: String,
        channel: String,
        rendered_len: usize,
        latency_ms: u64,
        source_business_date: chrono::NaiveDate,
        source_observed_at: chrono::DateTime<chrono::FixedOffset>,
        source_batch_id: String,
        source_content_sha256: String,
    ) -> Self {
        let mut event = Self::new(kind, None, outcome, channel, rendered_len, latency_ms);
        event.audit_schema_version = SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION;
        event.rule_ids = SOURCE_BATCH_DELIVERY_AUDIT_RULE_IDS
            .iter()
            .map(|rule| (*rule).to_owned())
            .collect();
        event.source_as_of = Some(source_observed_at);
        event.source_business_date = Some(source_business_date);
        event.source_batch_id = Some(source_batch_id);
        event.source_content_sha256 = Some(source_content_sha256);
        event.subject_hash = source_batch_delivery_subject_hash(
            source_business_date,
            &source_observed_at,
            event
                .source_batch_id
                .as_deref()
                .expect("source batch ID assigned"),
            event
                .source_content_sha256
                .as_deref()
                .expect("source content hash assigned"),
        );
        event.identity_hash =
            delivery_identity_hash_from_subject(&event.kind, &event.subject_hash, &event.channel);
        event
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_news_flash(
        kind: String,
        outcome: String,
        channel: String,
        rendered_len: usize,
        latency_ms: u64,
        reservation_sha256: String,
        sources: Vec<NewsFlashAuditSource>,
        evidence_sha256: String,
        render_sha256: String,
    ) -> Self {
        let mut event = Self::new(kind, None, outcome, channel, rendered_len, latency_ms);
        event.audit_schema_version = NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION;
        event.rule_ids = NEWS_FLASH_DELIVERY_AUDIT_RULE_IDS
            .iter()
            .map(|rule| (*rule).to_owned())
            .collect();
        event.subject_hash = reservation_sha256.clone();
        event.identity_hash =
            delivery_identity_hash_from_subject(&event.kind, &event.subject_hash, &event.channel);
        event.news_flash_join_sha256 = Some(news_flash_delivery_join_hash(
            &event.kind,
            &event.outcome,
            &event.channel,
            &reservation_sha256,
            &evidence_sha256,
            &render_sha256,
        ));
        event.news_flash_reservation_sha256 = Some(reservation_sha256);
        event.news_flash_sources = Some(sources);
        event.news_flash_evidence_sha256 = Some(evidence_sha256);
        event.news_flash_render_sha256 = Some(render_sha256);
        event
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_news_flash_attempt(
        kind: String,
        decision_key: String,
        channel: String,
        rendered_len: usize,
        business_date: chrono::NaiveDate,
        reservation_sha256: String,
        sources: Vec<NewsFlashAuditSource>,
        evidence_sha256: String,
        render_sha256: String,
        attempt_ordinal: u32,
        attempt_observed_at: chrono::DateTime<chrono::FixedOffset>,
    ) -> Self {
        let attempt_identity = news_flash_sink_attempt_identity(
            &reservation_sha256,
            attempt_ordinal,
            &channel,
            &attempt_observed_at,
        );
        let attempt_sha256 = news_flash_sink_attempt_sha256(
            &kind,
            &decision_key,
            business_date,
            &reservation_sha256,
            &evidence_sha256,
            &render_sha256,
            &sources,
            attempt_ordinal,
            &channel,
            &attempt_observed_at,
            &attempt_identity,
        );
        let mut event = Self::new_news_flash(
            kind,
            "Attempted".to_owned(),
            channel,
            rendered_len,
            0,
            reservation_sha256,
            sources,
            evidence_sha256,
            render_sha256,
        );
        event.news_flash_business_date = Some(business_date);
        event.news_flash_decision_key = Some(decision_key);
        event.news_flash_transaction_stage =
            Some(NewsFlashTransactionStage::SinkAttempt.as_str().to_owned());
        event.news_flash_attempt_ordinal = Some(attempt_ordinal);
        event.news_flash_attempt_observed_at = Some(attempt_observed_at);
        event.news_flash_sink_attempt_identity = Some(attempt_identity);
        event.news_flash_sink_attempt_sha256 = Some(attempt_sha256);
        event.news_flash_join_sha256 = Some(news_flash_transaction_join_hash(&event));
        event
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_news_flash_terminal(
        stage: NewsFlashTransactionStage,
        kind: String,
        decision_key: String,
        channel: String,
        rendered_len: usize,
        latency_ms: u64,
        business_date: chrono::NaiveDate,
        reservation_sha256: String,
        sources: Vec<NewsFlashAuditSource>,
        evidence_sha256: String,
        render_sha256: String,
        attempt_ordinal: u32,
        attempt_observed_at: chrono::DateTime<chrono::FixedOffset>,
        sink_attempt_identity: String,
        sink_attempt_sha256: String,
        attempt_envelope_id: String,
        remote_receipt: Option<NewsFlashRemoteReceipt>,
        terminal_observed_at: chrono::DateTime<chrono::FixedOffset>,
        terminal_reason_code: Option<String>,
        transport_evidence_sha256: Option<String>,
    ) -> Self {
        let outcome = match stage {
            NewsFlashTransactionStage::Accepted => "Pushed",
            NewsFlashTransactionStage::DefinitivelyRejected => "SinkError",
            NewsFlashTransactionStage::Uncertain => "Uncertain",
            NewsFlashTransactionStage::SinkAttempt => "Attempted",
        };
        let mut event = Self::new_news_flash(
            kind,
            outcome.to_owned(),
            channel,
            rendered_len,
            latency_ms,
            reservation_sha256,
            sources,
            evidence_sha256,
            render_sha256,
        );
        event.news_flash_business_date = Some(business_date);
        event.news_flash_decision_key = Some(decision_key);
        event.news_flash_transaction_stage = Some(stage.as_str().to_owned());
        event.news_flash_attempt_ordinal = Some(attempt_ordinal);
        event.news_flash_attempt_observed_at = Some(attempt_observed_at);
        event.news_flash_sink_attempt_identity = Some(sink_attempt_identity);
        event.news_flash_sink_attempt_sha256 = Some(sink_attempt_sha256);
        event.news_flash_attempt_envelope_id = Some(attempt_envelope_id);
        event.news_flash_terminal_observed_at = Some(terminal_observed_at);
        event.news_flash_terminal_reason_code = terminal_reason_code;
        event.news_flash_transport_evidence_sha256 = transport_evidence_sha256;
        if let Some(receipt) = remote_receipt {
            event.news_flash_remote_receipt_identity =
                Some(news_flash_remote_receipt_identity(&receipt));
            event.news_flash_remote_receipt_sha256 =
                Some(news_flash_remote_receipt_sha256(&receipt));
            event.news_flash_remote_receipt = Some(receipt);
        }
        event.news_flash_join_sha256 = Some(news_flash_transaction_join_hash(&event));
        event
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_news_flash_failure(
        provider: Option<String>,
        available_provider: Option<String>,
        stage: String,
        reason_code: String,
        diagnostic_code: String,
        diagnostic: String,
        retryable: bool,
        observed_at: chrono::DateTime<chrono::FixedOffset>,
        source_record_count: u32,
        batch_id: Option<String>,
        record_id: Option<String>,
    ) -> Self {
        let mut event = Self::new(
            "news_flash_source_failure_v1".to_owned(),
            None,
            "Failed".to_owned(),
            "none".to_owned(),
            0,
            0,
        );
        event.audit_schema_version = NEWS_FLASH_FAILURE_AUDIT_SCHEMA_VERSION;
        event.rule_ids = NEWS_FLASH_FAILURE_AUDIT_RULE_IDS
            .iter()
            .map(|rule| (*rule).to_owned())
            .collect();
        event.retryable = retryable;
        let identity_sha256 = news_flash_failure_identity_hash(NewsFlashFailureIdentityFields {
            provider: provider.as_deref(),
            available_provider: available_provider.as_deref(),
            stage: &stage,
            reason_code: &reason_code,
            diagnostic_code: &diagnostic_code,
            diagnostic: &diagnostic,
            retryable,
            observed_at: &observed_at,
            source_record_count,
            batch_id: batch_id.as_deref(),
            record_id: record_id.as_deref(),
        });
        event.reason_code = reason_code.clone();
        event.subject_hash = identity_sha256.clone();
        event.identity_hash =
            delivery_identity_hash_from_subject(&event.kind, &event.subject_hash, &event.channel);
        event.news_flash_failure_provider = provider;
        event.news_flash_failure_available_provider = available_provider;
        event.news_flash_failure_stage = Some(stage);
        event.news_flash_failure_reason = Some(reason_code);
        event.news_flash_failure_diagnostic_code = Some(diagnostic_code);
        event.news_flash_failure_diagnostic = Some(diagnostic);
        event.news_flash_failure_observed_at = Some(observed_at);
        event.news_flash_failure_source_record_count = Some(source_record_count);
        event.news_flash_failure_batch_id = batch_id;
        event.news_flash_failure_record_id = record_id;
        event.news_flash_failure_identity_sha256 = Some(identity_sha256);
        event
    }
}

impl DomainEvent for PushDeliveryEvent {
    fn event_type(&self) -> &'static str {
        "push.delivery.audit"
    }

    fn source(&self) -> &'static str {
        "push_l4"
    }

    fn entity_key(&self) -> Option<&str> {
        None
    }

    fn payload(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("PushDeliveryEvent is always serializable")
    }

    fn validate(&self) -> Result<(), EnvelopeError> {
        if self.kind.trim().is_empty() {
            return Err(EnvelopeError::BlankDeliveryKind);
        }
        if self.outcome.trim().is_empty() {
            return Err(EnvelopeError::BlankDeliveryOutcome);
        }
        let failure_schema = self.audit_schema_version == NEWS_FLASH_FAILURE_AUDIT_SCHEMA_VERSION;
        let expected_outcome = if failure_schema {
            if self.outcome != "Failed" {
                return Err(EnvelopeError::InvalidDeliveryOutcome(self.outcome.clone()));
            }
            None
        } else {
            Some(
                delivery_outcome_metadata(&self.outcome)
                    .ok_or_else(|| EnvelopeError::InvalidDeliveryOutcome(self.outcome.clone()))?,
            )
        };
        if self.channel.trim().is_empty() {
            return Err(EnvelopeError::BlankDeliveryChannel);
        }
        if self.kind.contains('\0') || self.channel.contains('\0') {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "kind/channel contains NUL".into(),
            ));
        }
        if !matches!(
            self.audit_schema_version,
            DELIVERY_AUDIT_SCHEMA_VERSION
                | COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION
                | SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION
                | NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION
                | NEWS_FLASH_FAILURE_AUDIT_SCHEMA_VERSION
        ) {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "audit_schema_version".into(),
            ));
        }
        if self.decision_status != self.outcome {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "decision_status".into(),
            ));
        }
        if let Some((expected_reason, expected_retryable)) = expected_outcome {
            if self.retryable != expected_retryable {
                return Err(EnvelopeError::InvalidDeliveryAuditField("retryable".into()));
            }
            if self.reason_code != expected_reason {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "reason_code".into(),
                ));
            }
        }
        let expected_rules = match self.audit_schema_version {
            COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION => COUNTED_DELIVERY_AUDIT_RULE_IDS.as_slice(),
            SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION => {
                SOURCE_BATCH_DELIVERY_AUDIT_RULE_IDS.as_slice()
            }
            NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION => {
                NEWS_FLASH_DELIVERY_AUDIT_RULE_IDS.as_slice()
            }
            NEWS_FLASH_FAILURE_AUDIT_SCHEMA_VERSION => NEWS_FLASH_FAILURE_AUDIT_RULE_IDS.as_slice(),
            DELIVERY_AUDIT_SCHEMA_VERSION => DELIVERY_AUDIT_RULE_IDS.as_slice(),
            _ => unreachable!("schema version checked above"),
        }
        .iter()
        .map(|rule| (*rule).to_string())
        .collect::<Vec<_>>();
        if self.rule_ids != expected_rules {
            return Err(EnvelopeError::InvalidDeliveryAuditField("rule_ids".into()));
        }
        if !is_lower_hex_sha256(&self.identity_hash) {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "identity_hash".into(),
            ));
        }
        if !is_lower_hex_sha256(&self.subject_hash) {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "subject_hash".into(),
            ));
        }
        let expected_identity =
            delivery_identity_hash_from_subject(&self.kind, &self.subject_hash, &self.channel);
        if self.identity_hash != expected_identity {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "identity_hash is not bound to subject/kind/channel".into(),
            ));
        }
        let counted_fields = [
            self.decision_identity_hash.as_deref(),
            self.attempt_identity_hash.as_deref(),
            self.artifact_sha256.as_deref(),
            self.sink_result_sha256.as_deref(),
            self.receipt_sha256.as_deref(),
            self.counted_join_hash.as_deref(),
        ];
        let source_batch_fields = [
            self.source_batch_id.as_deref(),
            self.source_content_sha256.as_deref(),
        ];
        let news_flash_delivery_fields = [
            self.news_flash_reservation_sha256.as_deref(),
            self.news_flash_evidence_sha256.as_deref(),
            self.news_flash_render_sha256.as_deref(),
            self.news_flash_join_sha256.as_deref(),
        ];
        let news_flash_failure_fields = [
            self.news_flash_failure_provider.as_deref(),
            self.news_flash_failure_available_provider.as_deref(),
            self.news_flash_failure_stage.as_deref(),
            self.news_flash_failure_reason.as_deref(),
            self.news_flash_failure_diagnostic_code.as_deref(),
            self.news_flash_failure_diagnostic.as_deref(),
            self.news_flash_failure_batch_id.as_deref(),
            self.news_flash_failure_record_id.as_deref(),
            self.news_flash_failure_identity_sha256.as_deref(),
        ];
        if self.audit_schema_version == COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION {
            let durable_push_kind = self
                .durable_push_kind
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    EnvelopeError::InvalidDeliveryAuditField("durable_push_kind".into())
                })?;
            let stable_template_id = self
                .stable_template_id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    EnvelopeError::InvalidDeliveryAuditField("stable_template_id".into())
                })?;
            let durable_kind = crate::durable_delivery::PushKind::ALL
                .into_iter()
                .find(|candidate| candidate.as_str() == durable_push_kind);
            if self.kind != stable_template_id
                || durable_kind.is_none_or(|kind| kind.stable_template_id() != stable_template_id)
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "durable push kind/template binding".into(),
                ));
            }
            if counted_fields
                .iter()
                .any(|value| value.is_none_or(|value| !is_lower_hex_sha256(value)))
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "counted delivery hashes".into(),
                ));
            }
            let expected_join = counted_delivery_join_hash(
                &self.kind,
                &self.outcome,
                &self.channel,
                self.decision_identity_hash
                    .as_deref()
                    .expect("counted hashes checked"),
                self.attempt_identity_hash
                    .as_deref()
                    .expect("counted hashes checked"),
                self.artifact_sha256
                    .as_deref()
                    .expect("counted hashes checked"),
                self.sink_result_sha256
                    .as_deref()
                    .expect("counted hashes checked"),
                self.receipt_sha256
                    .as_deref()
                    .expect("counted hashes checked"),
            );
            if self.counted_join_hash.as_deref() != Some(expected_join.as_str()) {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "counted_join_hash".into(),
                ));
            }
        } else if counted_fields.iter().any(|value| value.is_some())
            || self.durable_push_kind.is_some()
            || self.stable_template_id.is_some()
        {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "non-counted audit must not contain counted delivery hashes".into(),
            ));
        }
        if self.audit_schema_version == SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION {
            let source_business_date = self.source_business_date.ok_or_else(|| {
                EnvelopeError::InvalidDeliveryAuditField("source_business_date".into())
            })?;
            let source_observed_at = self
                .source_as_of
                .as_ref()
                .ok_or_else(|| EnvelopeError::InvalidDeliveryAuditField("source_as_of".into()))?;
            let source_batch_id = self
                .source_batch_id
                .as_deref()
                .filter(|value| {
                    value.trim() == *value
                        && value.starts_with("chain-batch:")
                        && value.len() > "chain-batch:".len()
                })
                .ok_or_else(|| {
                    EnvelopeError::InvalidDeliveryAuditField("source_batch_id".into())
                })?;
            let source_content_sha256 = self
                .source_content_sha256
                .as_deref()
                .filter(|value| is_lower_hex_sha256(value))
                .ok_or_else(|| {
                    EnvelopeError::InvalidDeliveryAuditField("source_content_sha256".into())
                })?;
            if source_observed_at.date_naive() < source_business_date {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "source_as_of predates source_business_date".into(),
                ));
            }
            let expected_subject = source_batch_delivery_subject_hash(
                source_business_date,
                source_observed_at,
                source_batch_id,
                source_content_sha256,
            );
            if self.subject_hash != expected_subject {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "subject_hash is not bound to source batch lineage".into(),
                ));
            }
        } else if self.source_business_date.is_some()
            || self.source_as_of.is_some()
            || source_batch_fields.iter().any(|value| value.is_some())
        {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "non-source-batch audit must not contain source batch lineage".into(),
            ));
        }
        if self.audit_schema_version == NEWS_FLASH_DELIVERY_AUDIT_SCHEMA_VERSION {
            let sources = self
                .news_flash_sources
                .as_deref()
                .filter(|sources| !sources.is_empty() && sources.len() <= 3)
                .ok_or_else(|| {
                    EnvelopeError::InvalidDeliveryAuditField("news_flash_sources".into())
                })?;
            validate_news_flash_sources(sources)?;
            if news_flash_delivery_fields
                .iter()
                .any(|value| value.is_none_or(|value| !is_lower_hex_sha256(value)))
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "news_flash delivery hashes".into(),
                ));
            }
            let reservation = self
                .news_flash_reservation_sha256
                .as_deref()
                .expect("NewsFlash hashes checked");
            let evidence = self
                .news_flash_evidence_sha256
                .as_deref()
                .expect("NewsFlash hashes checked");
            let render = self
                .news_flash_render_sha256
                .as_deref()
                .expect("NewsFlash hashes checked");
            if news_flash_evidence_sha256(sources) != evidence {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "news_flash_evidence_sha256".into(),
                ));
            }
            let expected_join = if self.news_flash_transaction_stage.is_some() {
                validate_news_flash_transaction(self, sources)?;
                news_flash_transaction_join_hash(self)
            } else {
                news_flash_delivery_join_hash(
                    &self.kind,
                    &self.outcome,
                    &self.channel,
                    reservation,
                    evidence,
                    render,
                )
            };
            if self.news_flash_join_sha256.as_deref() != Some(expected_join.as_str())
                || self.subject_hash != reservation
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "news_flash delivery join".into(),
                ));
            }
        } else if self.news_flash_sources.is_some()
            || news_flash_delivery_fields
                .iter()
                .any(|value| value.is_some())
            || self.news_flash_business_date.is_some()
            || self.news_flash_decision_key.is_some()
            || self.news_flash_transaction_stage.is_some()
            || self.news_flash_attempt_ordinal.is_some()
            || self.news_flash_attempt_observed_at.is_some()
            || self.news_flash_sink_attempt_identity.is_some()
            || self.news_flash_sink_attempt_sha256.is_some()
            || self.news_flash_attempt_envelope_id.is_some()
            || self.news_flash_remote_receipt.is_some()
            || self.news_flash_remote_receipt_identity.is_some()
            || self.news_flash_remote_receipt_sha256.is_some()
            || self.news_flash_terminal_observed_at.is_some()
            || self.news_flash_terminal_reason_code.is_some()
            || self.news_flash_transport_evidence_sha256.is_some()
        {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "non-NewsFlash audit contains NewsFlash delivery binding".into(),
            ));
        }
        if self.audit_schema_version == NEWS_FLASH_FAILURE_AUDIT_SCHEMA_VERSION {
            let required_failure_fields = [
                self.news_flash_failure_stage.as_deref(),
                self.news_flash_failure_reason.as_deref(),
                self.news_flash_failure_diagnostic_code.as_deref(),
                self.news_flash_failure_diagnostic.as_deref(),
                self.news_flash_failure_identity_sha256.as_deref(),
            ];
            if required_failure_fields
                .iter()
                .any(|value| value.is_none_or(|value| value.trim().is_empty()))
                || self.news_flash_failure_observed_at.is_none()
                || self.news_flash_failure_source_record_count.is_none()
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "news_flash failure fields".into(),
                ));
            }
            for (field, value) in [
                ("provider", self.news_flash_failure_provider.as_deref()),
                (
                    "available_provider",
                    self.news_flash_failure_available_provider.as_deref(),
                ),
                ("batch_id", self.news_flash_failure_batch_id.as_deref()),
                ("record_id", self.news_flash_failure_record_id.as_deref()),
            ] {
                if value.is_some_and(|value| {
                    value.trim().is_empty() || value.trim() != value || value.contains('\0')
                }) {
                    return Err(EnvelopeError::InvalidDeliveryAuditField(format!(
                        "news_flash_failure_{field}"
                    )));
                }
            }
            let diagnostic = self
                .news_flash_failure_diagnostic
                .as_deref()
                .expect("failure fields checked");
            if diagnostic.len() > 512 || diagnostic.contains('\0') {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "news_flash_failure_diagnostic".into(),
                ));
            }
            let expected_identity =
                news_flash_failure_identity_hash(NewsFlashFailureIdentityFields {
                    provider: self.news_flash_failure_provider.as_deref(),
                    available_provider: self.news_flash_failure_available_provider.as_deref(),
                    stage: self
                        .news_flash_failure_stage
                        .as_deref()
                        .expect("failure fields checked"),
                    reason_code: self
                        .news_flash_failure_reason
                        .as_deref()
                        .expect("failure fields checked"),
                    diagnostic_code: self
                        .news_flash_failure_diagnostic_code
                        .as_deref()
                        .expect("failure fields checked"),
                    diagnostic,
                    retryable: self.retryable,
                    observed_at: self
                        .news_flash_failure_observed_at
                        .as_ref()
                        .expect("failure fields checked"),
                    source_record_count: self
                        .news_flash_failure_source_record_count
                        .expect("failure fields checked"),
                    batch_id: self.news_flash_failure_batch_id.as_deref(),
                    record_id: self.news_flash_failure_record_id.as_deref(),
                });
            if self.news_flash_failure_identity_sha256.as_deref()
                != Some(expected_identity.as_str())
                || self.subject_hash != expected_identity
                || self.reason_code
                    != self
                        .news_flash_failure_reason
                        .as_deref()
                        .expect("failure fields checked")
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "news_flash failure identity".into(),
                ));
            }
        } else if self.news_flash_failure_observed_at.is_some()
            || self.news_flash_failure_source_record_count.is_some()
            || news_flash_failure_fields
                .iter()
                .any(|value| value.is_some())
        {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "non-NewsFlash-failure audit contains failure binding".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn delivery_outcome_metadata(outcome: &str) -> Option<(&'static str, bool)> {
    Some(match outcome {
        "Attempted" => ("delivery.attempted", false),
        "Pushed" => ("delivery.confirmed", false),
        "SinkError" => ("delivery.sink_error", true),
        "Failed" => ("delivery.failed", true),
        "Deduped" => ("delivery.deduped", false),
        "Denied" => ("delivery.denied", false),
        "Uncertain" => ("delivery.uncertain", false),
        _ => return None,
    })
}

pub fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn delivery_identity_hash(kind: &str, code: Option<&str>, channel: &str) -> String {
    let subject_hash = delivery_subject_hash(code);
    delivery_identity_hash_from_subject(kind, &subject_hash, channel)
}

fn delivery_subject_hash(code: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DELIVERY_SUBJECT_HASH_DOMAIN.as_bytes());
    hasher.update([0]);
    hasher.update(code.unwrap_or("<none>").as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn delivery_identity_hash_from_subject(
    kind: &str,
    subject_hash: &str,
    channel: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DELIVERY_IDENTITY_HASH_DOMAIN.as_bytes());
    hasher.update([0]);
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    hasher.update(subject_hash.as_bytes());
    hasher.update([0]);
    hasher.update(channel.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn counted_delivery_join_hash(
    kind: &str,
    outcome: &str,
    channel: &str,
    decision_identity_hash: &str,
    attempt_identity_hash: &str,
    artifact_sha256: &str,
    sink_result_sha256: &str,
    receipt_sha256: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(COUNTED_DELIVERY_JOIN_HASH_DOMAIN.as_bytes());
    for value in [
        kind,
        outcome,
        channel,
        decision_identity_hash,
        attempt_identity_hash,
        artifact_sha256,
        sink_result_sha256,
        receipt_sha256,
    ] {
        hasher.update([0]);
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub(crate) fn source_batch_delivery_subject_hash(
    business_date: chrono::NaiveDate,
    observed_at: &chrono::DateTime<chrono::FixedOffset>,
    batch_id: &str,
    content_sha256: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SOURCE_BATCH_DELIVERY_SUBJECT_HASH_DOMAIN.as_bytes());
    for value in [
        business_date.format("%Y-%m-%d").to_string(),
        observed_at.to_rfc3339(),
        batch_id.to_owned(),
        content_sha256.to_owned(),
    ] {
        hasher.update([0]);
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn validate_news_flash_sources(sources: &[NewsFlashAuditSource]) -> Result<(), EnvelopeError> {
    for source in sources {
        if [
            source.event_id.as_str(),
            source.provider.as_str(),
            source.source.as_str(),
            source.batch_id.as_str(),
        ]
        .iter()
        .any(|value| value.trim().is_empty() || value.contains('\0'))
            || source.observed_at < source.published_at
        {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "news_flash_sources".into(),
            ));
        }
    }
    Ok(())
}

fn validate_news_flash_transaction(
    event: &PushDeliveryEvent,
    sources: &[NewsFlashAuditSource],
) -> Result<(), EnvelopeError> {
    let stage = event
        .news_flash_transaction_stage
        .as_deref()
        .and_then(NewsFlashTransactionStage::parse)
        .ok_or_else(|| {
            EnvelopeError::InvalidDeliveryAuditField("news_flash_transaction_stage".into())
        })?;
    let business_date = event.news_flash_business_date.ok_or_else(|| {
        EnvelopeError::InvalidDeliveryAuditField("news_flash_business_date".into())
    })?;
    let decision_key = event
        .news_flash_decision_key
        .as_deref()
        .filter(|value| !value.trim().is_empty() && value.trim() == *value && !value.contains('\0'))
        .ok_or_else(|| {
            EnvelopeError::InvalidDeliveryAuditField("news_flash_decision_key".into())
        })?;
    let ordinal = event
        .news_flash_attempt_ordinal
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            EnvelopeError::InvalidDeliveryAuditField("news_flash_attempt_ordinal".into())
        })?;
    let observed_at = event
        .news_flash_attempt_observed_at
        .as_ref()
        .ok_or_else(|| {
            EnvelopeError::InvalidDeliveryAuditField("news_flash_attempt_observed_at".into())
        })?;
    let attempt_identity = event
        .news_flash_sink_attempt_identity
        .as_deref()
        .filter(|value| is_lower_hex_sha256(value))
        .ok_or_else(|| {
            EnvelopeError::InvalidDeliveryAuditField("news_flash_sink_attempt_identity".into())
        })?;
    let attempt_sha256 = event
        .news_flash_sink_attempt_sha256
        .as_deref()
        .filter(|value| is_lower_hex_sha256(value))
        .ok_or_else(|| {
            EnvelopeError::InvalidDeliveryAuditField("news_flash_sink_attempt_sha256".into())
        })?;
    let reservation = event
        .news_flash_reservation_sha256
        .as_deref()
        .expect("NewsFlash reservation validated before transaction");
    let evidence = event
        .news_flash_evidence_sha256
        .as_deref()
        .expect("NewsFlash evidence validated before transaction");
    let render = event
        .news_flash_render_sha256
        .as_deref()
        .expect("NewsFlash render validated before transaction");
    if news_flash_sink_attempt_identity(reservation, ordinal, &event.channel, observed_at)
        != attempt_identity
        || news_flash_sink_attempt_sha256(
            &event.kind,
            decision_key,
            business_date,
            reservation,
            evidence,
            render,
            sources,
            ordinal,
            &event.channel,
            observed_at,
            attempt_identity,
        ) != attempt_sha256
    {
        return Err(EnvelopeError::InvalidDeliveryAuditField(
            "news_flash sink attempt binding".into(),
        ));
    }

    match stage {
        NewsFlashTransactionStage::SinkAttempt => {
            if event.outcome != "Attempted"
                || event.news_flash_attempt_envelope_id.is_some()
                || event.news_flash_remote_receipt.is_some()
                || event.news_flash_remote_receipt_identity.is_some()
                || event.news_flash_remote_receipt_sha256.is_some()
                || event.news_flash_terminal_observed_at.is_some()
                || event.news_flash_terminal_reason_code.is_some()
                || event.news_flash_transport_evidence_sha256.is_some()
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "SinkAttempt terminal fields".into(),
                ));
            }
        }
        NewsFlashTransactionStage::Accepted => {
            if event.outcome != "Pushed" {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "Accepted outcome".into(),
                ));
            }
            let receipt = event.news_flash_remote_receipt.as_ref().ok_or_else(|| {
                EnvelopeError::InvalidDeliveryAuditField("news_flash_remote_receipt".into())
            })?;
            if [
                receipt.channel.as_str(),
                receipt.provider.as_str(),
                receipt.message_id.as_str(),
                receipt.platform_message_id.as_str(),
            ]
            .iter()
            .any(|value| value.trim().is_empty() || value.contains('\0'))
                || receipt.channel != event.channel
                || receipt.accepted_at < *observed_at
                || event.news_flash_remote_receipt_identity.as_deref()
                    != Some(news_flash_remote_receipt_identity(receipt).as_str())
                || event.news_flash_remote_receipt_sha256.as_deref()
                    != Some(news_flash_remote_receipt_sha256(receipt).as_str())
                || event.news_flash_terminal_reason_code.is_some()
                || event.news_flash_transport_evidence_sha256.is_some()
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "news_flash typed remote receipt".into(),
                ));
            }
        }
        NewsFlashTransactionStage::DefinitivelyRejected => {
            if event.outcome != "SinkError"
                || event.news_flash_remote_receipt.is_some()
                || event.news_flash_remote_receipt_identity.is_some()
                || event.news_flash_remote_receipt_sha256.is_some()
                || event
                    .news_flash_terminal_reason_code
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty() || value.contains('\0'))
                || event
                    .news_flash_transport_evidence_sha256
                    .as_deref()
                    .is_none_or(|value| !is_lower_hex_sha256(value))
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "DefinitivelyRejected terminal fields".into(),
                ));
            }
        }
        NewsFlashTransactionStage::Uncertain => {
            if event.outcome != "Uncertain"
                || event.news_flash_remote_receipt.is_some()
                || event.news_flash_remote_receipt_identity.is_some()
                || event.news_flash_remote_receipt_sha256.is_some()
                || event
                    .news_flash_terminal_reason_code
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty() || value.contains('\0'))
                || event.news_flash_transport_evidence_sha256.is_some()
            {
                return Err(EnvelopeError::InvalidDeliveryAuditField(
                    "Uncertain terminal fields".into(),
                ));
            }
        }
    }
    if stage != NewsFlashTransactionStage::SinkAttempt
        && event
            .news_flash_attempt_envelope_id
            .as_deref()
            .is_none_or(|value| !is_lower_hex_sha256(value))
    {
        return Err(EnvelopeError::InvalidDeliveryAuditField(
            "news_flash_attempt_envelope_id".into(),
        ));
    }
    if stage != NewsFlashTransactionStage::SinkAttempt
        && event
            .news_flash_terminal_observed_at
            .is_none_or(|terminal_at| {
                terminal_at < *observed_at || terminal_at.date_naive() < business_date
            })
    {
        return Err(EnvelopeError::InvalidDeliveryAuditField(
            "news_flash_terminal_observed_at".into(),
        ));
    }
    Ok(())
}

pub fn news_flash_evidence_sha256(sources: &[NewsFlashAuditSource]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(NEWS_FLASH_EVIDENCE_HASH_DOMAIN.as_bytes());
    hasher.update((sources.len() as u64).to_be_bytes());
    for source in sources {
        let published_at = source.published_at.to_rfc3339();
        let observed_at = source.observed_at.to_rfc3339();
        for value in [
            source.event_id.as_str(),
            source.provider.as_str(),
            source.source.as_str(),
            published_at.as_str(),
            observed_at.as_str(),
            source.batch_id.as_str(),
        ] {
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

pub fn news_flash_delivery_join_hash(
    kind: &str,
    outcome: &str,
    channel: &str,
    reservation_sha256: &str,
    evidence_sha256: &str,
    render_sha256: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(NEWS_FLASH_DELIVERY_JOIN_HASH_DOMAIN.as_bytes());
    for value in [
        kind,
        outcome,
        channel,
        reservation_sha256,
        evidence_sha256,
        render_sha256,
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub fn news_flash_sink_attempt_identity(
    reservation_sha256: &str,
    attempt_ordinal: u32,
    channel: &str,
    observed_at: &chrono::DateTime<chrono::FixedOffset>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(NEWS_FLASH_SINK_ATTEMPT_IDENTITY_HASH_DOMAIN.as_bytes());
    for value in [
        reservation_sha256.to_owned(),
        attempt_ordinal.to_string(),
        channel.to_owned(),
        observed_at.to_rfc3339(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[allow(clippy::too_many_arguments)]
pub fn news_flash_sink_attempt_sha256(
    kind: &str,
    decision_key: &str,
    business_date: chrono::NaiveDate,
    reservation_sha256: &str,
    evidence_sha256: &str,
    render_sha256: &str,
    sources: &[NewsFlashAuditSource],
    attempt_ordinal: u32,
    channel: &str,
    observed_at: &chrono::DateTime<chrono::FixedOffset>,
    attempt_identity: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(NEWS_FLASH_SINK_ATTEMPT_HASH_DOMAIN.as_bytes());
    for value in [
        kind.to_owned(),
        decision_key.to_owned(),
        business_date.format("%Y-%m-%d").to_string(),
        reservation_sha256.to_owned(),
        evidence_sha256.to_owned(),
        render_sha256.to_owned(),
        news_flash_evidence_sha256(sources),
        attempt_ordinal.to_string(),
        channel.to_owned(),
        observed_at.to_rfc3339(),
        attempt_identity.to_owned(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub fn news_flash_remote_receipt_identity(receipt: &NewsFlashRemoteReceipt) -> String {
    let mut hasher = Sha256::new();
    hasher.update(NEWS_FLASH_REMOTE_RECEIPT_IDENTITY_HASH_DOMAIN.as_bytes());
    for value in [
        receipt.channel.as_str(),
        receipt.provider.as_str(),
        receipt.message_id.as_str(),
        receipt.platform_message_id.as_str(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub fn news_flash_remote_receipt_sha256(receipt: &NewsFlashRemoteReceipt) -> String {
    let mut hasher = Sha256::new();
    hasher.update(NEWS_FLASH_REMOTE_RECEIPT_HASH_DOMAIN.as_bytes());
    for value in [
        receipt.channel.clone(),
        receipt.provider.clone(),
        receipt.message_id.clone(),
        receipt.platform_message_id.clone(),
        receipt.accepted_at.to_rfc3339(),
        receipt.latency_ms.to_string(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

pub fn news_flash_transaction_join_hash(event: &PushDeliveryEvent) -> String {
    let mut hasher = Sha256::new();
    hasher.update(NEWS_FLASH_TRANSACTION_JOIN_HASH_DOMAIN.as_bytes());
    let values = [
        event.kind.clone(),
        event.outcome.clone(),
        event.channel.clone(),
        event.rendered_len.to_string(),
        event.latency_ms.to_string(),
        optional_owned(event.news_flash_transaction_stage.as_ref()),
        event
            .news_flash_business_date
            .map(|value| value.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "<absent>".to_owned()),
        optional_owned(event.news_flash_decision_key.as_ref()),
        optional_owned(event.news_flash_reservation_sha256.as_ref()),
        optional_owned(event.news_flash_evidence_sha256.as_ref()),
        optional_owned(event.news_flash_render_sha256.as_ref()),
        event
            .news_flash_attempt_ordinal
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<absent>".to_owned()),
        event
            .news_flash_attempt_observed_at
            .as_ref()
            .map(chrono::DateTime::to_rfc3339)
            .unwrap_or_else(|| "<absent>".to_owned()),
        optional_owned(event.news_flash_sink_attempt_identity.as_ref()),
        optional_owned(event.news_flash_sink_attempt_sha256.as_ref()),
        optional_owned(event.news_flash_attempt_envelope_id.as_ref()),
        event
            .news_flash_terminal_observed_at
            .as_ref()
            .map(chrono::DateTime::to_rfc3339)
            .unwrap_or_else(|| "<absent>".to_owned()),
        optional_owned(event.news_flash_terminal_reason_code.as_ref()),
        optional_owned(event.news_flash_transport_evidence_sha256.as_ref()),
        optional_owned(event.news_flash_remote_receipt_identity.as_ref()),
        optional_owned(event.news_flash_remote_receipt_sha256.as_ref()),
    ];
    for value in values {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn optional_owned(value: Option<&String>) -> String {
    value.cloned().unwrap_or_else(|| "<absent>".to_owned())
}

#[derive(Debug, Clone, Copy)]
pub struct NewsFlashFailureIdentityFields<'a> {
    pub provider: Option<&'a str>,
    pub available_provider: Option<&'a str>,
    pub stage: &'a str,
    pub reason_code: &'a str,
    pub diagnostic_code: &'a str,
    pub diagnostic: &'a str,
    pub retryable: bool,
    pub observed_at: &'a chrono::DateTime<chrono::FixedOffset>,
    pub source_record_count: u32,
    pub batch_id: Option<&'a str>,
    pub record_id: Option<&'a str>,
}

pub fn news_flash_failure_identity_hash(fields: NewsFlashFailureIdentityFields<'_>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(NEWS_FLASH_FAILURE_IDENTITY_HASH_DOMAIN.as_bytes());
    let retryable = if fields.retryable { "true" } else { "false" };
    let observed_at = fields.observed_at.to_rfc3339();
    let source_record_count = fields.source_record_count.to_string();
    for value in [
        fields.provider.unwrap_or("<absent>"),
        fields.available_provider.unwrap_or("<absent>"),
        fields.stage,
        fields.reason_code,
        fields.diagnostic_code,
        fields.diagnostic,
        retryable,
        observed_at.as_str(),
        source_record_count.as_str(),
        fields.batch_id.unwrap_or("<absent>"),
        fields.record_id.unwrap_or("<absent>"),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_captures_event_metadata_and_payload() {
        let event = PushDeliveryEvent::new(
            "announcement_v1".into(),
            Some("TEST_CODE_600519".into()),
            "Pushed".into(),
            "dry_run".into(),
            42,
            0,
        );
        let env = EventEnvelope::from_event(
            &event,
            "evt-1".into(),
            "trace-1".into(),
            chrono::Local::now(),
        )
        .unwrap();
        assert_eq!(env.event_type, "push.delivery.audit");
        assert_eq!(
            env.entity_key, None,
            "authoritative identity must be redacted"
        );
        assert_eq!(env.payload["outcome"], "Pushed");
        assert_eq!(env.payload["decision_status"], "Pushed");
        assert_eq!(env.payload["retryable"], false);
        assert_eq!(env.payload["reason_code"], "delivery.confirmed");
        assert_eq!(env.payload["audit_schema_version"], 2);
        assert!(env.payload.get("code").is_none());
        assert_eq!(env.payload["identity_hash"].as_str().unwrap().len(), 64);
        assert_eq!(env.payload["subject_hash"].as_str().unwrap().len(), 64);
        assert!(env.payload["rule_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule == "BR-142"));
        assert_eq!(env.version, 1);
    }

    #[test]
    fn envelope_rejects_empty_identity_fields() {
        let event =
            PushDeliveryEvent::new("".into(), None, "Pushed".into(), "dry_run".into(), 0, 0);
        let err = EventEnvelope::from_event(
            &event,
            "evt-1".into(),
            "trace-1".into(),
            chrono::Local::now(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("kind"));
    }

    #[test]
    fn br130_delivery_rejects_blank_or_unknown_audit_fields() {
        for (outcome, channel, expected) in [
            ("", "dry_run", "outcome"),
            ("Unknown", "dry_run", "unsupported"),
            ("Pushed", " ", "channel"),
        ] {
            let event = PushDeliveryEvent::new(
                "announcement_v1".into(),
                Some("TEST_CODE_600519".into()),
                outcome.into(),
                channel.into(),
                1,
                1,
            );
            let error = EventEnvelope::from_event(
                &event,
                "evt-invalid".into(),
                "trace-invalid".into(),
                chrono::Local::now(),
            )
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let event = PushDeliveryEvent::new(
            "announcement_v1".into(),
            None,
            "Failed".into(),
            "wechat".into(),
            0,
            0,
        );
        let env = EventEnvelope::from_event(
            &event,
            "evt-2".into(),
            "trace-2".into(),
            chrono::Local::now(),
        )
        .unwrap();
        let text = serde_json::to_string(&env).unwrap();
        let decoded: EventEnvelope = serde_json::from_str(&text).unwrap();
        assert_eq!(decoded.id, "evt-2");
        assert_eq!(decoded.replay_of, None);
    }

    #[test]
    fn br192_counted_delivery_v3_binds_attempt_artifact_result_and_receipt() {
        let hash = |byte: char| byte.to_string().repeat(64);
        let event = PushDeliveryEvent::new_counted(
            "HoldingEvent".into(),
            "holding_event_v1".into(),
            "Pushed".into(),
            "TEST_CODE_DRY_RUN".into(),
            42,
            7,
            hash('a'),
            hash('b'),
            hash('c'),
            hash('d'),
            hash('e'),
        );
        let envelope = EventEnvelope::from_event(
            &event,
            "TEST_CODE_COUNTED_EVENT".into(),
            "TEST_CODE_COUNTED_TRACE".into(),
            chrono::Local::now(),
        )
        .expect("valid counted delivery audit");

        assert_eq!(
            envelope.payload["audit_schema_version"],
            COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION
        );
        assert_eq!(envelope.payload["decision_identity_hash"], hash('a'));
        assert_eq!(envelope.payload["attempt_identity_hash"], hash('b'));
        assert_eq!(envelope.payload["artifact_sha256"], hash('c'));
        assert_eq!(envelope.payload["sink_result_sha256"], hash('d'));
        assert_eq!(envelope.payload["receipt_sha256"], hash('e'));
        assert_eq!(
            envelope.payload["counted_join_hash"],
            counted_delivery_join_hash(
                "holding_event_v1",
                "Pushed",
                "TEST_CODE_DRY_RUN",
                &hash('a'),
                &hash('b'),
                &hash('c'),
                &hash('d'),
                &hash('e'),
            )
        );
    }

    #[test]
    fn br192_counted_delivery_v3_rejects_a_tampered_join() {
        let mut event = PushDeliveryEvent::new_counted(
            "HoldingEvent".into(),
            "holding_event_v1".into(),
            "Pushed".into(),
            "TEST_CODE_DRY_RUN".into(),
            42,
            7,
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            "e".repeat(64),
        );
        event.counted_join_hash = Some("f".repeat(64));

        let error = EventEnvelope::from_event(
            &event,
            "TEST_CODE_COUNTED_EVENT".into(),
            "TEST_CODE_COUNTED_TRACE".into(),
            chrono::Local::now(),
        )
        .expect_err("tampered counted join must fail closed");
        assert_eq!(
            error,
            EnvelopeError::InvalidDeliveryAuditField("counted_join_hash".into())
        );
    }

    #[test]
    fn br192_counted_uncertain_is_non_retryable_and_lineage_bound() {
        let event = PushDeliveryEvent::new_counted(
            "HoldingEvent".into(),
            "holding_event_v1".into(),
            "Uncertain".into(),
            "authoritative".into(),
            42,
            0,
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            "e".repeat(64),
        );
        event.validate().expect("valid counted uncertainty");
        assert!(!event.retryable);
        assert_eq!(event.reason_code, "delivery.uncertain");
        assert_eq!(
            event.rule_ids,
            COUNTED_DELIVERY_AUDIT_RULE_IDS
                .iter()
                .map(|rule| (*rule).to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(event.durable_push_kind.as_deref(), Some("HoldingEvent"));
        assert_eq!(
            event.stable_template_id.as_deref(),
            Some("holding_event_v1")
        );
    }

    #[test]
    fn br160_source_batch_delivery_v4_binds_batch_hash_business_date_and_observation() {
        let business_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap();
        let observed_at = chrono::DateTime::parse_from_rfc3339("2026-07-31T15:01:02+08:00")
            .expect("valid source observation");
        let event = PushDeliveryEvent::new_source_batch(
            "catalyst_review_v1".into(),
            "Pushed".into(),
            "TEST_CODE_DRY_RUN".into(),
            42,
            7,
            business_date,
            observed_at,
            "chain-batch:TEST_CODE_A10_DELIVERY".into(),
            "a".repeat(64),
        );
        let envelope = EventEnvelope::from_event(
            &event,
            "TEST_CODE_A10_DELIVERY_EVENT".into(),
            "TEST_CODE_A10_DELIVERY_TRACE".into(),
            chrono::Local::now(),
        )
        .expect("source-batch-bound delivery audit");

        assert_eq!(
            envelope.payload["audit_schema_version"],
            SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION
        );
        assert_eq!(envelope.payload["source_business_date"], "2026-07-31");
        assert_eq!(
            envelope.payload["source_batch_id"],
            "chain-batch:TEST_CODE_A10_DELIVERY"
        );
        assert_eq!(envelope.payload["source_content_sha256"], "a".repeat(64));
        assert_eq!(envelope.payload["source_as_of"], observed_at.to_rfc3339());
        assert_eq!(
            envelope.payload["subject_hash"],
            source_batch_delivery_subject_hash(
                business_date,
                &observed_at,
                "chain-batch:TEST_CODE_A10_DELIVERY",
                &"a".repeat(64),
            )
        );
        assert_eq!(
            event.rule_ids,
            SOURCE_BATCH_DELIVERY_AUDIT_RULE_IDS
                .iter()
                .map(|rule| (*rule).to_owned())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn br160_source_batch_delivery_v4_rejects_tampered_lineage() {
        let mut event = PushDeliveryEvent::new_source_batch(
            "catalyst_review_v1".into(),
            "Pushed".into(),
            "TEST_CODE_DRY_RUN".into(),
            42,
            7,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
            chrono::DateTime::parse_from_rfc3339("2026-07-31T15:01:02+08:00").unwrap(),
            "chain-batch:TEST_CODE_A10_DELIVERY".into(),
            "a".repeat(64),
        );
        event.source_content_sha256 = Some("b".repeat(64));

        let error = EventEnvelope::from_event(
            &event,
            "TEST_CODE_A10_TAMPERED_EVENT".into(),
            "TEST_CODE_A10_TAMPERED_TRACE".into(),
            chrono::Local::now(),
        )
        .expect_err("tampered source batch lineage must fail closed");
        assert_eq!(
            error,
            EnvelopeError::InvalidDeliveryAuditField(
                "subject_hash is not bound to source batch lineage".into()
            )
        );
    }
}

#[cfg(test)]
#[path = "../gate_d_event_envelope_regression.rs"]
mod gate_d_regression;
