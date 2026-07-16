//! Event envelope contract — v17.1-r2 Task 1
//!
//! Defines `DomainEvent`, `EventEnvelope`, and `PushDeliveryEvent` for the
//! event-seam infrastructure. The `event` module must be free of monitor-bin
//! imports; only `lib.rs` consumers touch it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ========================================================================
// DomainEvent trait
// ========================================================================

/// Trait implemented by all domain events that can be wrapped in an `EventEnvelope`.
pub trait DomainEvent: Send + Sync + 'static {
    /// The event type string, e.g. `"push.delivery"`.
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

        let payload = serde_json::to_value(event)
            .map_err(|_| EnvelopeError::BlankEventType)?;

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
    pub code: Option<String>,
    pub outcome: String,
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
        Self {
            kind,
            code,
            outcome,
            channel,
            rendered_len,
            latency_ms,
        }
    }
}

impl DomainEvent for PushDeliveryEvent {
    fn event_type(&self) -> &'static str {
        "push.delivery"
    }

    fn source(&self) -> &'static str {
        "push_l4"
    }

    fn entity_key(&self) -> Option<&str> {
        self.code.as_deref()
    }

    fn payload(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("PushDeliveryEvent is always serializable")
    }

    fn validate(&self) -> Result<(), EnvelopeError> {
        if self.kind.trim().is_empty() {
            return Err(EnvelopeError::BlankDeliveryKind);
        }
        Ok(())
    }
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
            Some("600519".into()),
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
        assert_eq!(env.event_type, "push.delivery");
        assert_eq!(env.entity_key.as_deref(), Some("600519"));
        assert_eq!(env.payload["outcome"], "Pushed");
        assert_eq!(env.version, 1);
    }

    #[test]
    fn envelope_rejects_empty_identity_fields() {
        let event = PushDeliveryEvent::new(
            "".into(),
            None,
            "Pushed".into(),
            "dry_run".into(),
            0,
            0,
        );
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
