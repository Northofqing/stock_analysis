//! Registered business rules: BR-043, BR-091, BR-130, BR-142, BR-160, BR-192.
//! PushRecord and ReplayablePushEvent — v17.3 Task 1
//!
//! Normalized domain events for the delivery observation seam (`push.delivery.audit`)
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
    /// `SinkError` and `Failed` share the failed classification; unknown
    /// strings remain invalid instead of being silently reclassified.
    pub fn from_audit_str(s: &str) -> Option<Self> {
        Some(match s {
            "Pushed" => PushOutcomeLabel::Pushed,
            "SinkError" | "Failed" | "Uncertain" => PushOutcomeLabel::Failed,
            "Deduped" => PushOutcomeLabel::Deduped,
            "Denied" => PushOutcomeLabel::Denied,
            _ => return None,
        })
    }
}

// ========================================================================
// PushRecord
// ========================================================================

/// A normalized push delivery observation record.
///
/// Produced with the authoritative `event_type = "push.delivery.audit"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRecord {
    pub id: String,
    pub kind: String,
    pub code: Option<String>,
    pub trace_id: String,
    pub ts: chrono::DateTime<chrono::Local>,
    pub outcome: PushOutcomeLabel,
    pub channel: String,
    pub rendered_len: usize,
    pub latency_ms: u64,
    /// Present for v2/v3 redacted authoritative delivery audits.
    pub subject_hash: Option<String>,
    pub identity_hash: Option<String>,
    pub decision_status: Option<PushOutcomeLabel>,
    pub retryable: Option<bool>,
    pub rule_ids: Vec<String>,
    pub reason_code: Option<String>,
    pub source_as_of: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub audit_schema_version: Option<u32>,
    pub decision_identity_hash: Option<String>,
    pub attempt_identity_hash: Option<String>,
    pub artifact_sha256: Option<String>,
    pub sink_result_sha256: Option<String>,
    pub receipt_sha256: Option<String>,
    pub counted_join_hash: Option<String>,
    pub durable_push_kind: Option<String>,
    pub stable_template_id: Option<String>,
    pub source_business_date: Option<chrono::NaiveDate>,
    pub source_batch_id: Option<String>,
    pub source_content_sha256: Option<String>,
}

/// Errors when extracting a `PushRecord` from an `EventEnvelope`.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PushRecordError {
    #[error("event_type mismatch: expected 'push.delivery.audit', got '{0}'")]
    EventTypeMismatch(String),

    #[error("missing required field: {0}")]
    MissingField(String),

    #[error("invalid field type: {0}")]
    InvalidFieldType(String),

    #[error("invalid field value: {0}")]
    InvalidFieldValue(String),
}

impl PushRecord {
    /// Extract a `PushRecord` from an `EventEnvelope`.
    ///
    /// Returns `Err` if the envelope's `event_type` is not `push.delivery.audit`
    /// or if any required field is missing or has an invalid type.
    pub fn try_from(env: &super::envelope::EventEnvelope) -> Result<Self, PushRecordError> {
        if env.event_type != "push.delivery.audit" {
            return Err(PushRecordError::EventTypeMismatch(env.event_type.clone()));
        }

        for (field, value) in [
            ("id", env.id.as_str()),
            ("trace_id", env.trace_id.as_str()),
            ("source", env.source.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(PushRecordError::MissingField(field.into()));
            }
        }
        if env.version != 1 {
            return Err(PushRecordError::InvalidFieldValue(format!(
                "version={}",
                env.version
            )));
        }

        let audit_schema_version = match env.payload.get("audit_schema_version") {
            None => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|version| u32::try_from(version).ok())
                    .ok_or_else(|| {
                        PushRecordError::InvalidFieldType("audit_schema_version".into())
                    })?,
            ),
        };
        let expected_fields: &[&str] = match audit_schema_version {
            Some(super::envelope::COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION) => &[
                "kind",
                "outcome",
                "decision_status",
                "retryable",
                "rule_ids",
                "reason_code",
                "subject_hash",
                "identity_hash",
                "source_as_of",
                "audit_schema_version",
                "channel",
                "rendered_len",
                "latency_ms",
                "decision_identity_hash",
                "attempt_identity_hash",
                "artifact_sha256",
                "sink_result_sha256",
                "receipt_sha256",
                "counted_join_hash",
                "durable_push_kind",
                "stable_template_id",
            ],
            Some(super::envelope::SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION) => &[
                "kind",
                "outcome",
                "decision_status",
                "retryable",
                "rule_ids",
                "reason_code",
                "subject_hash",
                "identity_hash",
                "source_as_of",
                "audit_schema_version",
                "channel",
                "rendered_len",
                "latency_ms",
                "source_business_date",
                "source_batch_id",
                "source_content_sha256",
            ],
            Some(_) => &[
                "kind",
                "outcome",
                "decision_status",
                "retryable",
                "rule_ids",
                "reason_code",
                "subject_hash",
                "identity_hash",
                "source_as_of",
                "audit_schema_version",
                "channel",
                "rendered_len",
                "latency_ms",
            ],
            None => &[
                "kind",
                "code",
                "outcome",
                "channel",
                "rendered_len",
                "latency_ms",
            ],
        };
        validate_closed_payload(&env.payload, expected_fields)?;

        let id = env.id.clone();
        let trace_id = env.trace_id.clone();
        let ts = env.ts;

        let required_text = |field: &str| -> Result<String, PushRecordError> {
            let value = env
                .payload
                .get(field)
                .ok_or_else(|| PushRecordError::MissingField(field.into()))?;
            let text = value
                .as_str()
                .ok_or_else(|| PushRecordError::InvalidFieldType(field.into()))?;
            if text.trim().is_empty() {
                return Err(PushRecordError::MissingField(field.into()));
            }
            Ok(text.to_string())
        };

        let kind = required_text("kind")?;

        let code = if audit_schema_version.is_none() {
            parse_legacy_code(env.payload.get("code"), env.entity_key.as_deref())?
        } else {
            match env.payload.get("code") {
                None | Some(serde_json::Value::Null) => None,
                Some(value) => {
                    let code = value
                        .as_str()
                        .ok_or_else(|| PushRecordError::InvalidFieldType("code".into()))?;
                    if code.trim().is_empty() {
                        return Err(PushRecordError::InvalidFieldValue("code".into()));
                    }
                    Some(code.to_string())
                }
            }
        };
        if audit_schema_version.is_some() && code.as_deref() != env.entity_key.as_deref() {
            return Err(PushRecordError::InvalidFieldValue(
                "payload.code does not match envelope.entity_key".into(),
            ));
        }

        let outcome_str = required_text("outcome")?;
        let outcome = PushOutcomeLabel::from_audit_str(&outcome_str)
            .ok_or_else(|| PushRecordError::InvalidFieldValue(format!("outcome={outcome_str}")))?;

        let channel = required_text("channel")?;

        let rendered_len = env
            .payload
            .get("rendered_len")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| PushRecordError::InvalidFieldType("rendered_len".into()))?;

        let latency = env
            .payload
            .get("latency_ms")
            .ok_or_else(|| PushRecordError::MissingField("latency_ms".into()))?;
        let latency_ms = latency
            .as_u64()
            .ok_or_else(|| PushRecordError::InvalidFieldType("latency_ms".into()))?;

        let (
            subject_hash,
            identity_hash,
            decision_status,
            retryable,
            rule_ids,
            reason_code,
            source_as_of,
        ) = if let Some(version) = audit_schema_version {
            if !matches!(
                version,
                super::envelope::DELIVERY_AUDIT_SCHEMA_VERSION
                    | super::envelope::COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION
                    | super::envelope::SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION
            ) {
                return Err(PushRecordError::InvalidFieldValue(format!(
                    "audit_schema_version={version}"
                )));
            }
            if code.is_some() || env.entity_key.is_some() || env.payload.get("code").is_some() {
                return Err(PushRecordError::InvalidFieldValue(
                    "v2 delivery identity must be redacted".into(),
                ));
            }

            let subject_hash = required_text("subject_hash")?;
            if !super::envelope::is_lower_hex_sha256(&subject_hash) {
                return Err(PushRecordError::InvalidFieldValue("subject_hash".into()));
            }
            let identity_hash = required_text("identity_hash")?;
            if !super::envelope::is_lower_hex_sha256(&identity_hash) {
                return Err(PushRecordError::InvalidFieldValue("identity_hash".into()));
            }
            let expected_identity = super::envelope::delivery_identity_hash_from_subject(
                &kind,
                &subject_hash,
                &channel,
            );
            if identity_hash != expected_identity {
                return Err(PushRecordError::InvalidFieldValue(
                    "identity_hash is not bound to subject/kind/channel".into(),
                ));
            }

            let decision_status_text = required_text("decision_status")?;
            if decision_status_text != outcome_str {
                return Err(PushRecordError::InvalidFieldValue(
                    "decision_status does not match outcome".into(),
                ));
            }
            let decision_status = PushOutcomeLabel::from_audit_str(&decision_status_text)
                .ok_or_else(|| {
                    PushRecordError::InvalidFieldValue(format!(
                        "decision_status={decision_status_text}"
                    ))
                })?;

            let retryable = env
                .payload
                .get("retryable")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| PushRecordError::InvalidFieldType("retryable".into()))?;
            let reason_code = required_text("reason_code")?;
            let Some((expected_reason, expected_retryable)) =
                super::envelope::delivery_outcome_metadata(&outcome_str)
            else {
                return Err(PushRecordError::InvalidFieldValue(format!(
                    "outcome={outcome_str}"
                )));
            };
            if retryable != expected_retryable {
                return Err(PushRecordError::InvalidFieldValue(
                    "retryable does not match outcome".into(),
                ));
            }
            if reason_code != expected_reason {
                return Err(PushRecordError::InvalidFieldValue(
                    "reason_code does not match outcome".into(),
                ));
            }

            let rule_ids_value = env
                .payload
                .get("rule_ids")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| PushRecordError::InvalidFieldType("rule_ids".into()))?;
            let mut rule_ids = Vec::with_capacity(rule_ids_value.len());
            for value in rule_ids_value {
                let rule = value
                    .as_str()
                    .filter(|rule| !rule.trim().is_empty())
                    .ok_or_else(|| PushRecordError::InvalidFieldValue("rule_ids".into()))?;
                rule_ids.push(rule.to_string());
            }
            let expected_rules = match version {
                super::envelope::COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION => {
                    super::envelope::COUNTED_DELIVERY_AUDIT_RULE_IDS.as_slice()
                }
                super::envelope::SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION => {
                    super::envelope::SOURCE_BATCH_DELIVERY_AUDIT_RULE_IDS.as_slice()
                }
                super::envelope::DELIVERY_AUDIT_SCHEMA_VERSION => {
                    super::envelope::DELIVERY_AUDIT_RULE_IDS.as_slice()
                }
                _ => unreachable!("schema version checked above"),
            }
            .iter()
            .map(|rule| (*rule).to_string())
            .collect::<Vec<_>>();
            if rule_ids != expected_rules {
                return Err(PushRecordError::InvalidFieldValue("rule_ids".into()));
            }

            let source_as_of =
                match env.payload.get("source_as_of") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(value) => {
                        let value = value.as_str().ok_or_else(|| {
                            PushRecordError::InvalidFieldType("source_as_of".into())
                        })?;
                        Some(chrono::DateTime::parse_from_rfc3339(value).map_err(|_| {
                            PushRecordError::InvalidFieldValue("source_as_of".into())
                        })?)
                    }
                };

            (
                Some(subject_hash),
                Some(identity_hash),
                Some(decision_status),
                Some(retryable),
                rule_ids,
                Some(reason_code),
                source_as_of,
            )
        } else {
            (None, None, None, None, Vec::new(), None, None)
        };

        let (
            decision_identity_hash,
            attempt_identity_hash,
            artifact_sha256,
            sink_result_sha256,
            receipt_sha256,
            counted_join_hash,
            durable_push_kind,
            stable_template_id,
        ) = if audit_schema_version == Some(super::envelope::COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION)
        {
            let decision_identity_hash = required_text("decision_identity_hash")?;
            let attempt_identity_hash = required_text("attempt_identity_hash")?;
            let artifact_sha256 = required_text("artifact_sha256")?;
            let sink_result_sha256 = required_text("sink_result_sha256")?;
            let receipt_sha256 = required_text("receipt_sha256")?;
            let counted_join_hash = required_text("counted_join_hash")?;
            let durable_push_kind = required_text("durable_push_kind")?;
            let stable_template_id = required_text("stable_template_id")?;
            let canonical_template = crate::durable_delivery::PushKind::ALL
                .iter()
                .copied()
                .find(|candidate| candidate.as_str() == durable_push_kind)
                .map(crate::durable_delivery::PushKind::stable_template_id);
            if kind != stable_template_id || canonical_template != Some(stable_template_id.as_str())
            {
                return Err(PushRecordError::InvalidFieldValue(
                    "durable push kind/template binding".into(),
                ));
            }
            for (field, value) in [
                ("decision_identity_hash", &decision_identity_hash),
                ("attempt_identity_hash", &attempt_identity_hash),
                ("artifact_sha256", &artifact_sha256),
                ("sink_result_sha256", &sink_result_sha256),
                ("receipt_sha256", &receipt_sha256),
                ("counted_join_hash", &counted_join_hash),
            ] {
                if !super::envelope::is_lower_hex_sha256(value) {
                    return Err(PushRecordError::InvalidFieldValue(field.into()));
                }
            }
            let expected_join = super::envelope::counted_delivery_join_hash(
                &kind,
                &outcome_str,
                &channel,
                &decision_identity_hash,
                &attempt_identity_hash,
                &artifact_sha256,
                &sink_result_sha256,
                &receipt_sha256,
            );
            if counted_join_hash != expected_join {
                return Err(PushRecordError::InvalidFieldValue(
                    "counted_join_hash".into(),
                ));
            }
            if env.id != counted_join_hash {
                return Err(PushRecordError::InvalidFieldValue(
                    "schema-v3 envelope id must equal counted_join_hash".into(),
                ));
            }
            if subject_hash.as_deref() != Some(decision_identity_hash.as_str()) {
                return Err(PushRecordError::InvalidFieldValue(
                    "subject_hash does not match decision_identity_hash".into(),
                ));
            }
            (
                Some(decision_identity_hash),
                Some(attempt_identity_hash),
                Some(artifact_sha256),
                Some(sink_result_sha256),
                Some(receipt_sha256),
                Some(counted_join_hash),
                Some(durable_push_kind),
                Some(stable_template_id),
            )
        } else {
            (None, None, None, None, None, None, None, None)
        };

        let (source_business_date, source_batch_id, source_content_sha256) = if audit_schema_version
            == Some(super::envelope::SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION)
        {
            let business_date = chrono::NaiveDate::parse_from_str(
                &required_text("source_business_date")?,
                "%Y-%m-%d",
            )
            .map_err(|_| PushRecordError::InvalidFieldValue("source_business_date".into()))?;
            let batch_id = required_text("source_batch_id")?;
            if batch_id.trim() != batch_id
                || !batch_id.starts_with("chain-batch:")
                || batch_id.len() <= "chain-batch:".len()
            {
                return Err(PushRecordError::InvalidFieldValue("source_batch_id".into()));
            }
            let content_sha256 = required_text("source_content_sha256")?;
            if !super::envelope::is_lower_hex_sha256(&content_sha256) {
                return Err(PushRecordError::InvalidFieldValue(
                    "source_content_sha256".into(),
                ));
            }
            let observed_at = source_as_of
                .as_ref()
                .ok_or_else(|| PushRecordError::InvalidFieldValue("source_as_of".into()))?;
            if observed_at.date_naive() < business_date {
                return Err(PushRecordError::InvalidFieldValue(
                    "source_as_of predates source_business_date".into(),
                ));
            }
            let expected_subject = super::envelope::source_batch_delivery_subject_hash(
                business_date,
                observed_at,
                &batch_id,
                &content_sha256,
            );
            if subject_hash.as_deref() != Some(expected_subject.as_str()) {
                return Err(PushRecordError::InvalidFieldValue(
                    "subject_hash is not bound to source batch lineage".into(),
                ));
            }
            (Some(business_date), Some(batch_id), Some(content_sha256))
        } else {
            (None, None, None)
        };

        Ok(PushRecord {
            id,
            kind,
            code,
            trace_id,
            ts,
            outcome,
            channel,
            rendered_len,
            latency_ms,
            subject_hash,
            identity_hash,
            decision_status,
            retryable,
            rule_ids,
            reason_code,
            source_as_of,
            audit_schema_version,
            decision_identity_hash,
            attempt_identity_hash,
            artifact_sha256,
            sink_result_sha256,
            receipt_sha256,
            counted_join_hash,
            durable_push_kind,
            stable_template_id,
            source_business_date,
            source_batch_id,
            source_content_sha256,
        })
    }

    /// Parse a new authoritative audit. Legacy rows remain readable by
    /// `try_from`, but may never enter the current persistence path.
    pub fn try_from_authoritative(
        env: &super::envelope::EventEnvelope,
    ) -> Result<Self, PushRecordError> {
        let record = Self::try_from(env)?;
        if !matches!(
            record.audit_schema_version,
            Some(
                super::envelope::DELIVERY_AUDIT_SCHEMA_VERSION
                    | super::envelope::COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION
                    | super::envelope::SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION
            )
        ) {
            return Err(PushRecordError::InvalidFieldValue(
                "authoritative delivery audit requires schema v2, v3 or v4".into(),
            ));
        }
        Ok(record)
    }
}

fn parse_legacy_code(
    payload_code: Option<&serde_json::Value>,
    entity_key: Option<&str>,
) -> Result<Option<String>, PushRecordError> {
    match (payload_code, entity_key) {
        (Some(serde_json::Value::String(code)), Some(key)) if code.is_empty() && key.is_empty() => {
            Ok(None)
        }
        (None | Some(serde_json::Value::Null), None) => Ok(None),
        (Some(serde_json::Value::String(code)), Some(key))
            if !code.trim().is_empty() && code == key =>
        {
            Ok(Some(code.clone()))
        }
        (Some(serde_json::Value::String(code)), _) if code.trim().is_empty() => {
            Err(PushRecordError::InvalidFieldValue("code".into()))
        }
        (Some(value), _) if !value.is_string() => {
            Err(PushRecordError::InvalidFieldType("code".into()))
        }
        _ => Err(PushRecordError::InvalidFieldValue(
            "payload.code does not match envelope.entity_key".into(),
        )),
    }
}

fn validate_closed_payload(
    payload: &serde_json::Value,
    expected_fields: &[&str],
) -> Result<(), PushRecordError> {
    let object = payload
        .as_object()
        .ok_or_else(|| PushRecordError::InvalidFieldType("payload".into()))?;
    for field in object.keys() {
        if !expected_fields.contains(&field.as_str()) {
            return Err(PushRecordError::InvalidFieldValue(format!(
                "unknown delivery audit field: {field}"
            )));
        }
    }
    for field in expected_fields {
        if !object.contains_key(*field) {
            return Err(PushRecordError::MissingField((*field).into()));
        }
    }
    Ok(())
}

// ========================================================================
// ReplayablePushEvent
// ========================================================================

/// A replayable push source event — the original push trigger before delivery.
///
/// Produced with `event_type = "push.source"`. Distinct from `push.delivery.audit`
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
        let env = make_delivery_envelope("push.delivery.audit");
        let record = PushRecord::try_from(&env).unwrap();
        assert_eq!(record.kind, "announcement_v1");
        assert_eq!(record.latency_ms, 37);
        assert_eq!(record.outcome, PushOutcomeLabel::Pushed);
        assert_eq!(record.code.as_deref(), Some("TEST_CODE_600519"));
        assert_eq!(record.channel, "dry_run");
    }

    #[test]
    fn br142_authoritative_record_requires_complete_redacted_metadata() {
        let event = crate::event::PushDeliveryEvent::new(
            "announcement_v1".into(),
            Some("TEST_CODE_600519".into()),
            "SinkError".into(),
            "dry_run".into(),
            12,
            37,
        );
        let env = EventEnvelope::from_event(
            &event,
            "evt-v2".into(),
            "trace-v2".into(),
            chrono::Local::now(),
        )
        .unwrap();
        let record = PushRecord::try_from_authoritative(&env).unwrap();

        assert_eq!(record.code, None);
        assert_eq!(record.audit_schema_version, Some(2));
        assert_eq!(record.retryable, Some(true));
        assert_eq!(record.reason_code.as_deref(), Some("delivery.sink_error"));
        assert_eq!(record.decision_status, Some(PushOutcomeLabel::Failed));
        assert!(record.rule_ids.iter().any(|rule| rule == "BR-142"));
        assert_eq!(record.subject_hash.as_deref().unwrap().len(), 64);
        assert_eq!(record.identity_hash.as_deref().unwrap().len(), 64);

        let mut incomplete = env;
        incomplete
            .payload
            .as_object_mut()
            .unwrap()
            .remove("retryable");
        assert!(PushRecord::try_from_authoritative(&incomplete).is_err());
    }

    #[test]
    fn br192_authoritative_record_requires_the_exact_counted_join() {
        let event = crate::event::PushDeliveryEvent::new_counted(
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
        let counted_join_hash = event
            .counted_join_hash
            .clone()
            .expect("counted event has a canonical envelope id");
        let mut envelope = EventEnvelope::from_event(
            &event,
            counted_join_hash,
            "TEST_CODE_COUNTED_TRACE".into(),
            chrono::Local::now(),
        )
        .expect("valid counted event");
        let record = PushRecord::try_from_authoritative(&envelope)
            .expect("counted delivery must be authoritative");

        assert_eq!(record.audit_schema_version, Some(3));
        assert_eq!(
            record.decision_identity_hash.as_deref(),
            Some(&*"a".repeat(64))
        );
        assert_eq!(
            record.attempt_identity_hash.as_deref(),
            Some(&*"b".repeat(64))
        );
        assert_eq!(record.artifact_sha256.as_deref(), Some(&*"c".repeat(64)));
        assert!(record.counted_join_hash.is_some());

        envelope.payload["receipt_sha256"] = serde_json::json!("f".repeat(64));
        assert!(matches!(
            PushRecord::try_from_authoritative(&envelope),
            Err(PushRecordError::InvalidFieldValue(field)) if field == "counted_join_hash"
        ));
    }

    #[test]
    fn br192_authoritative_record_rejects_noncanonical_counted_event_id() {
        let event = crate::event::PushDeliveryEvent::new_counted(
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
        let envelope = EventEnvelope::from_event(
            &event,
            "TEST_CODE_NONCANONICAL_COUNTED_EVENT".into(),
            "TEST_CODE_NONCANONICAL_COUNTED_TRACE".into(),
            chrono::Local::now(),
        )
        .expect("the generic envelope is structurally valid");

        assert!(matches!(
            PushRecord::try_from_authoritative(&envelope),
            Err(PushRecordError::InvalidFieldValue(field))
                if field == "schema-v3 envelope id must equal counted_join_hash"
        ));
    }

    #[test]
    fn br160_authoritative_record_preserves_and_revalidates_source_batch_lineage() {
        let observed_at = chrono::DateTime::parse_from_rfc3339("2026-07-31T15:01:02+08:00")
            .expect("valid source observation");
        let event = crate::event::PushDeliveryEvent::new_source_batch(
            "catalyst_review_v1".into(),
            "Pushed".into(),
            "TEST_CODE_DRY_RUN".into(),
            12,
            37,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
            observed_at,
            "chain-batch:TEST_CODE_A10_RECORD".into(),
            "a".repeat(64),
        );
        let mut envelope = EventEnvelope::from_event(
            &event,
            "TEST_CODE_A10_RECORD_EVENT".into(),
            "TEST_CODE_A10_RECORD_TRACE".into(),
            chrono::Local::now(),
        )
        .expect("valid source batch delivery envelope");
        let record = PushRecord::try_from_authoritative(&envelope)
            .expect("source batch delivery is authoritative");

        assert_eq!(record.audit_schema_version, Some(4));
        assert_eq!(
            record.source_business_date,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 31)
        );
        assert_eq!(
            record.source_batch_id.as_deref(),
            Some("chain-batch:TEST_CODE_A10_RECORD")
        );
        assert_eq!(
            record.source_content_sha256.as_deref(),
            Some(&*"a".repeat(64))
        );
        assert_eq!(record.source_as_of, Some(observed_at));
        assert!(record.rule_ids.iter().any(|rule| rule == "BR-160"));

        envelope.payload["source_batch_id"] = serde_json::json!("chain-batch:TEST_CODE_TAMPERED");
        assert!(matches!(
            PushRecord::try_from_authoritative(&envelope),
            Err(PushRecordError::InvalidFieldValue(field))
                if field == "subject_hash is not bound to source batch lineage"
        ));
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
            event_type: "push.delivery.audit".into(),
            entity_key: Some("TEST_CODE_600519".into()),
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
            event_type: "push.delivery.audit".into(),
            entity_key: Some("TEST_CODE_600519".into()),
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
            event_type: "push.delivery.audit".into(),
            entity_key: Some("TEST_CODE_600519".into()),
            payload: serde_json::json!({
                "kind": "announcement_v1",
                "code": "TEST_CODE_600519",
                "outcome": "SinkError",
                "channel": "dry_run",
                "rendered_len": 12,
                "latency_ms": 37,
            }),
            version: 1,
            replay_of: None,
        };
        let record = PushRecord::try_from(&env).unwrap();
        assert_eq!(record.outcome, PushOutcomeLabel::Failed);
    }

    #[test]
    fn br130_outcome_parser_rejects_unknown_values() {
        assert_eq!(
            PushOutcomeLabel::from_audit_str("Pushed"),
            Some(PushOutcomeLabel::Pushed)
        );
        assert_eq!(
            PushOutcomeLabel::from_audit_str("Failed"),
            Some(PushOutcomeLabel::Failed)
        );
        assert_eq!(
            PushOutcomeLabel::from_audit_str("Deduped"),
            Some(PushOutcomeLabel::Deduped)
        );
        assert_eq!(
            PushOutcomeLabel::from_audit_str("Denied"),
            Some(PushOutcomeLabel::Denied)
        );
        assert_eq!(PushOutcomeLabel::from_audit_str("Unknown"), None);

        let mut envelope = make_delivery_envelope("push.delivery.audit");
        envelope.payload["outcome"] = serde_json::json!("Unknown");
        assert!(matches!(
            PushRecord::try_from(&envelope),
            Err(PushRecordError::InvalidFieldValue(value)) if value.contains("outcome")
        ));
    }

    #[test]
    fn br130_push_record_rejects_incomplete_or_inconsistent_audit_fields() {
        let mut blank_id = make_delivery_envelope("push.delivery.audit");
        blank_id.id = " ".into();
        assert!(matches!(
            PushRecord::try_from(&blank_id),
            Err(PushRecordError::MissingField(field)) if field == "id"
        ));

        let mut bad_version = make_delivery_envelope("push.delivery.audit");
        bad_version.version = 2;
        assert!(matches!(
            PushRecord::try_from(&bad_version),
            Err(PushRecordError::InvalidFieldValue(value)) if value.contains("version")
        ));

        for (field, value) in [
            ("kind", serde_json::json!(7)),
            ("outcome", serde_json::Value::Null),
            ("channel", serde_json::json!(false)),
            ("latency_ms", serde_json::json!(-1)),
            ("code", serde_json::json!([])),
        ] {
            let mut envelope = make_delivery_envelope("push.delivery.audit");
            envelope.payload[field] = value;
            assert!(PushRecord::try_from(&envelope).is_err(), "field={field}");
        }

        let mut mismatched_code = make_delivery_envelope("push.delivery.audit");
        mismatched_code.payload["code"] = serde_json::json!("TEST_CODE_000001");
        assert!(matches!(
            PushRecord::try_from(&mismatched_code),
            Err(PushRecordError::InvalidFieldValue(value)) if value.contains("entity_key")
        ));
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
