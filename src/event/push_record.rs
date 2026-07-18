//! PushRecord and ReplayablePushEvent — v17.3 Task 1
//!
//! Normalized domain events for the delivery observation seam (`push.delivery`)
//! and the replayable source event seam (`push.source`).

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ========================================================================
// PushOutcomeLabel
// ========================================================================

/// Classification label for a push delivery outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PushOutcomeLabel {
    Pushed,
    Deduped,
    Denied,
    Failed,
}

impl PushOutcomeLabel {
    /// Parse from the string outcome used in audit events.
    ///
    /// `Pushed` and `SinkError` map to `Failed` (the early-return path means
    /// `Deduped`/`Denied` don't reach the publish site).
    pub fn from_audit_str(s: &str) -> Self {
        match s {
            "Pushed" => PushOutcomeLabel::Pushed,
            "SinkError" | "Failed" => PushOutcomeLabel::Failed,
            "Deduped" => PushOutcomeLabel::Deduped,
            "Denied" => PushOutcomeLabel::Denied,
            _ => PushOutcomeLabel::Failed,
        }
    }
}

// ========================================================================
// PushRecord
// ========================================================================

/// A normalized push delivery observation record.
///
/// Produced with `event_type = "push.delivery"` (reserved for the future
/// `PushApplicationService` boundary; the audit event is renamed to
/// `push.delivery.audit` in v17.3 Task 1 to avoid collision).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRecord {
    pub id: String,
    pub kind: String,
    pub code: Option<String>,
    pub trace_id: String,
    pub ts: chrono::DateTime<chrono::Local>,
    pub outcome: PushOutcomeLabel,
    pub channel: String,
    pub latency_ms: u64,
}

/// Errors when extracting a `PushRecord` from an `EventEnvelope`.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PushRecordError {
    #[error("event_type mismatch: expected 'push.delivery', got '{0}'")]
    EventTypeMismatch(String),

    #[error("missing required field: {0}")]
    MissingField(String),

    #[error("invalid field type: {0}")]
    InvalidFieldType(String),
}

impl PushRecord {
    /// Extract a `PushRecord` from an `EventEnvelope`.
    ///
    /// Returns `Err` if the envelope's `event_type` is not `push.delivery`
    /// or if any required field is missing or has an invalid type.
    pub fn try_from(env: &super::envelope::EventEnvelope) -> Result<Self, PushRecordError> {
        if env.event_type != "push.delivery" {
            return Err(PushRecordError::EventTypeMismatch(env.event_type.clone()));
        }

        let id = env.id.clone();
        let trace_id = env.trace_id.clone();
        let ts = env.ts;

        let kind = env
            .payload
            .get("kind")
            .and_then(|v| v.as_str())
            .map(String::from)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| PushRecordError::MissingField("kind".into()))?;

        let code = env
            .payload
            .get("code")
            .and_then(|v| v.as_str().map(String::from));

        let outcome_str = env
            .payload
            .get("outcome")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PushRecordError::MissingField("outcome".into()))?;
        let outcome = PushOutcomeLabel::from_audit_str(outcome_str);

        let channel = env
            .payload
            .get("channel")
            .and_then(|v| v.as_str())
            .map(String::from)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| PushRecordError::MissingField("channel".into()))?;

        let latency_ms = env
            .payload
            .get("latency_ms")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| PushRecordError::MissingField("latency_ms".into()))?;

        Ok(PushRecord {
            id,
            kind,
            code,
            trace_id,
            ts,
            outcome,
            channel,
            latency_ms,
        })
    }
}

// ========================================================================
// ReplayablePushEvent
// ========================================================================

/// A replayable push source event — the original push trigger before delivery.
///
/// Produced with `event_type = "push.source"`. Distinct from `push.delivery`
/// observation events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayablePushEvent {
    pub kind: String,
    pub code: Option<String>,
    pub text: String,
    pub source: String,
}

/// Errors when validating a `ReplayablePushEvent`.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ReplayablePushEventError {
    #[error("body text cannot be empty")]
    EmptyBody,

    #[error("kind cannot be blank")]
    BlankKind,

    #[error("source cannot be blank")]
    BlankSource,
}

impl ReplayablePushEvent {
    pub fn new(kind: String, code: Option<String>, text: String, source: String) -> Self {
        Self {
            kind,
            code,
            text,
            source,
        }
    }

    /// Validate business invariants.
    pub fn validate(&self) -> Result<(), ReplayablePushEventError> {
        if self.text.trim().is_empty() {
            return Err(ReplayablePushEventError::EmptyBody);
        }
        if self.kind.trim().is_empty() {
            return Err(ReplayablePushEventError::BlankKind);
        }
        if self.source.trim().is_empty() {
            return Err(ReplayablePushEventError::BlankSource);
        }
        Ok(())
    }
}

impl super::envelope::DomainEvent for ReplayablePushEvent {
    fn event_type(&self) -> &'static str {
        "push.source"
    }

    fn source(&self) -> &'static str {
        "push_l4"
    }

    fn entity_key(&self) -> Option<&str> {
        self.code.as_deref()
    }

    fn payload(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("ReplayablePushEvent is always serializable")
    }

    fn validate(&self) -> Result<(), super::envelope::EnvelopeError> {
        self.validate()
            .map_err(|_e| super::envelope::EnvelopeError::BlankEventType)
    }
}

// ========================================================================
// Tests
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::envelope::EventEnvelope;

    fn make_delivery_envelope(event_type: &str) -> EventEnvelope {
        EventEnvelope {
            id: "evt-1".into(),
            ts: chrono::Local::now(),
            trace_id: "trace-1".into(),
            source: "push_l4".into(),
            event_type: event_type.into(),
            entity_key: Some("TEST_CODE_600519".into()),
            payload: serde_json::json!({
                "kind": "announcement_v1",
                "code": "TEST_CODE_600519",
                "outcome": "Pushed",
                "channel": "dry_run",
                "rendered_len": 12,
                "latency_ms": 37,
            }),
            version: 1,
            replay_of: None,
        }
    }

    #[test]
    fn delivery_envelope_extracts_push_record() {
        let env = make_delivery_envelope("push.delivery");
        let record = PushRecord::try_from(&env).unwrap();
        assert_eq!(record.kind, "announcement_v1");
        assert_eq!(record.latency_ms, 37);
        assert_eq!(record.outcome, PushOutcomeLabel::Pushed);
        assert_eq!(record.code.as_deref(), Some("TEST_CODE_600519"));
        assert_eq!(record.channel, "dry_run");
    }

    #[test]
    fn non_delivery_event_is_not_counted_as_push_record() {
        let env = make_delivery_envelope("market.policy");
        assert!(PushRecord::try_from(&env).is_err());
    }

    #[test]
    fn replayable_event_rejects_empty_body() {
        let err =
            ReplayablePushEvent::new("Announcement".into(), None, "".into(), "monitor".into())
                .validate()
                .unwrap_err();
        assert!(err.to_string().contains("text"));
    }

    #[test]
    fn replayable_event_rejects_blank_kind() {
        let err = ReplayablePushEvent::new("  ".into(), None, "hello".into(), "monitor".into())
            .validate()
            .unwrap_err();
        assert!(err.to_string().contains("kind"));
    }

    #[test]
    fn replayable_event_rejects_blank_source() {
        let err =
            ReplayablePushEvent::new("Announcement".into(), None, "hello".into(), "  ".into())
                .validate()
                .unwrap_err();
        assert!(err.to_string().contains("source"));
    }

    #[test]
    fn push_record_rejects_missing_kind() {
        let env = EventEnvelope {
            id: "evt-1".into(),
            ts: chrono::Local::now(),
            trace_id: "trace-1".into(),
            source: "push_l4".into(),
            event_type: "push.delivery".into(),
            entity_key: None,
            payload: serde_json::json!({
                "code": "TEST_CODE_600519",
                "outcome": "Pushed",
                "channel": "dry_run",
                "latency_ms": 37,
            }),
            version: 1,
            replay_of: None,
        };
        assert!(PushRecord::try_from(&env).is_err());
    }

    #[test]
    fn push_record_rejects_missing_latency_ms() {
        let env = EventEnvelope {
            id: "evt-1".into(),
            ts: chrono::Local::now(),
            trace_id: "trace-1".into(),
            source: "push_l4".into(),
            event_type: "push.delivery".into(),
            entity_key: None,
            payload: serde_json::json!({
                "kind": "announcement_v1",
                "code": "TEST_CODE_600519",
                "outcome": "Pushed",
                "channel": "dry_run",
            }),
            version: 1,
            replay_of: None,
        };
        assert!(PushRecord::try_from(&env).is_err());
    }

    #[test]
    fn push_record_sink_error_maps_to_failed() {
        let env = EventEnvelope {
            id: "evt-1".into(),
            ts: chrono::Local::now(),
            trace_id: "trace-1".into(),
            source: "push_l4".into(),
            event_type: "push.delivery".into(),
            entity_key: None,
            payload: serde_json::json!({
                "kind": "announcement_v1",
                "code": "TEST_CODE_600519",
                "outcome": "SinkError",
                "channel": "dry_run",
                "latency_ms": 37,
            }),
            version: 1,
            replay_of: None,
        };
        let record = PushRecord::try_from(&env).unwrap();
        assert_eq!(record.outcome, PushOutcomeLabel::Failed);
    }

    #[test]
    fn replayable_push_event_round_trips() {
        let event = ReplayablePushEvent::new(
            "Announcement".into(),
            Some("TEST_CODE_600519".into()),
            "Test message".into(),
            "monitor".into(),
        );
        event.validate().unwrap();
        let env = EventEnvelope::from_event(
            &event,
            "evt-replay".into(),
            "trace-replay".into(),
            chrono::Local::now(),
        )
        .unwrap();
        assert_eq!(env.event_type, "push.source");
        assert_eq!(env.payload["kind"], "Announcement");
        assert_eq!(env.payload["code"], "TEST_CODE_600519");
    }
}
