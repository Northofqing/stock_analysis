//! Registered business rules: BR-091, BR-130, BR-142.
//! Event envelope contract — v17.1-r2 Task 1
//!
//! Defines `DomainEvent`, `EventEnvelope`, and `PushDeliveryEvent` for the
//! event-seam infrastructure. The `event` module must be free of monitor-bin
//! imports; only `lib.rs` consumers touch it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const DELIVERY_AUDIT_SCHEMA_VERSION: u32 = 2;
pub const DELIVERY_IDENTITY_HASH_DOMAIN: &str = "stock_analysis.delivery_identity.v2";
pub const DELIVERY_AUDIT_RULE_IDS: [&str; 5] = ["2.7", "BR-091", "BR-111", "BR-130", "BR-142"];

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

/// A domain event representing a push delivery attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushDeliveryEvent {
    pub kind: String,
    pub outcome: String,
    pub decision_status: String,
    pub retryable: bool,
    pub rule_ids: Vec<String>,
    pub reason_code: String,
    pub identity_hash: String,
    pub source_as_of: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub audit_schema_version: u32,
    pub channel: String,
    pub rendered_len: usize,
    pub latency_ms: u64,
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
        let identity_hash = delivery_identity_hash(&kind, code.as_deref(), &channel);
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
            identity_hash,
            source_as_of: None,
            audit_schema_version: DELIVERY_AUDIT_SCHEMA_VERSION,
            channel,
            rendered_len,
            latency_ms,
        }
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
        let Some((expected_reason, expected_retryable)) = delivery_outcome_metadata(&self.outcome)
        else {
            return Err(EnvelopeError::InvalidDeliveryOutcome(self.outcome.clone()));
        };
        if self.channel.trim().is_empty() {
            return Err(EnvelopeError::BlankDeliveryChannel);
        }
        if self.kind.contains('\0') || self.channel.contains('\0') {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "kind/channel contains NUL".into(),
            ));
        }
        if self.audit_schema_version != DELIVERY_AUDIT_SCHEMA_VERSION {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "audit_schema_version".into(),
            ));
        }
        if self.decision_status != self.outcome {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "decision_status".into(),
            ));
        }
        if self.retryable != expected_retryable {
            return Err(EnvelopeError::InvalidDeliveryAuditField("retryable".into()));
        }
        if self.reason_code != expected_reason {
            return Err(EnvelopeError::InvalidDeliveryAuditField(
                "reason_code".into(),
            ));
        }
        let expected_rules = DELIVERY_AUDIT_RULE_IDS
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
        Ok(())
    }
}

pub(crate) fn delivery_outcome_metadata(outcome: &str) -> Option<(&'static str, bool)> {
    Some(match outcome {
        "Pushed" => ("delivery.confirmed", false),
        "SinkError" => ("delivery.sink_error", true),
        "Failed" => ("delivery.failed", true),
        "Deduped" => ("delivery.deduped", false),
        "Denied" => ("delivery.denied", false),
        _ => return None,
    })
}

pub(crate) fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn delivery_identity_hash(kind: &str, code: Option<&str>, channel: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DELIVERY_IDENTITY_HASH_DOMAIN.as_bytes());
    hasher.update([0]);
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    hasher.update(code.unwrap_or("<none>").as_bytes());
    hasher.update([0]);
    hasher.update(channel.as_bytes());
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
}

#[cfg(test)]
#[path = "../gate_d_event_envelope_regression.rs"]
mod gate_d_regression;
