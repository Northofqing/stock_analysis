//! Read-only G5b terminal observation for one exact frozen event summary.
//! A frozen LLM result and a JSONL archive are not delivery evidence.

use super::{
    build_validated_terminal_evidence, load_decision, parse_envelope, sha256_hex, DecisionState,
    DeliveryEnvelope, DurableDeliveryCoordinator, DurableDeliveryError,
    FoundationTerminalDisposition, PushKind, Result, StoredDecision,
};
use crate::durable_delivery::model::{validate_business_date, CooldownScope, DeliverySubKind};
use rusqlite::{params, TransactionBehavior};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum G5bCountedTerminalV1 {
    Pending,
    Accepted,
    ManualAccepted,
    Rejected,
    Uncertain,
    ManualNotDelivered,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct G5bCountedObservationV1 {
    decision_identity: String,
    terminal: G5bCountedTerminalV1,
    authoritative_attempt_identity: Option<String>,
    immutable_audit_ref: Option<String>,
    terminal_evidence_sha256: Option<String>,
    accepted_channel: Option<String>,
}

impl G5bCountedObservationV1 {
    pub fn decision_identity(&self) -> &str {
        &self.decision_identity
    }

    pub const fn terminal(&self) -> G5bCountedTerminalV1 {
        self.terminal
    }

    /// Only a typed physical Accepted receipt, not a manual disposition.
    pub const fn is_authoritative_accepted(&self) -> bool {
        matches!(self.terminal, G5bCountedTerminalV1::Accepted)
    }

    pub fn authoritative_attempt_identity(&self) -> Option<&str> {
        self.authoritative_attempt_identity.as_deref()
    }

    pub fn immutable_audit_ref(&self) -> Option<&str> {
        self.immutable_audit_ref.as_deref()
    }

    pub fn terminal_evidence_sha256(&self) -> Option<&str> {
        self.terminal_evidence_sha256.as_deref()
    }

    pub fn accepted_channel(&self) -> Option<&str> {
        self.accepted_channel.as_deref()
    }
}

impl DurableDeliveryCoordinator {
    /// Inspect the one G5b decision for a frozen event without admission or
    /// sink access. Expected hashes must come from the validated frozen row
    /// and its exact summary, never from a newly generated LLM result.
    pub fn g5b_counted_observation_for_frozen(
        &self,
        business_date: &str,
        occurrence_identity: &str,
        source_binding_sha256: &str,
        rendered_content_sha256: &str,
    ) -> Result<Option<G5bCountedObservationV1>> {
        validate_business_date(business_date)?;
        if !valid_sha256(source_binding_sha256) || !valid_sha256(rendered_content_sha256) {
            return Err(g5b_mismatch("expected hash is invalid"));
        }
        self.with_connection(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let identities = {
                let mut query = transaction.prepare(
                    "SELECT decision_identity FROM delivery_decisions
                     WHERE business_date=?1 AND push_kind=?2
                     ORDER BY decision_identity",
                )?;
                let rows = query.query_map(
                    params![business_date, PushKind::G5bAttribution.as_str()],
                    |row| row.get::<_, String>(0),
                )?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut observation = None;
            for identity in identities {
                let stored = load_decision(&transaction, &identity)?
                    .ok_or_else(|| DurableDeliveryError::DecisionNotFound(identity.clone()))?;
                if sha256_hex(&stored.envelope_canonical) != stored.envelope_sha256 {
                    return Err(g5b_mismatch("frozen envelope hash mismatch"));
                }
                let envelope = parse_envelope(&stored.envelope_canonical)?;
                if envelope.canonical_bytes()? != stored.envelope_canonical {
                    return Err(g5b_mismatch("frozen envelope is not canonical"));
                }
                if envelope.schedule_occurrence_identity != occurrence_identity {
                    continue;
                }
                if observation.is_some() {
                    return Err(g5b_mismatch("multiple decisions own one occurrence"));
                }
                validate_g5b_binding(&stored, &envelope, business_date)?;
                if envelope.source_binding_sha256 != source_binding_sha256
                    || envelope.rendered_content_sha256 != rendered_content_sha256
                {
                    return Err(g5b_mismatch("frozen source or summary mismatch"));
                }
                let (terminal, attempt, audit_ref, evidence_sha256, accepted_channel) = if matches!(
                    stored.state,
                    DecisionState::Delivered
                        | DecisionState::RejectedDurable
                        | DecisionState::UncertainManualReview
                        | DecisionState::ManualResolvedRejected
                ) {
                    let verified =
                        build_validated_terminal_evidence(&transaction, &stored, &envelope, None)?;
                    let terminal = match verified.disposition {
                        FoundationTerminalDisposition::Accepted => G5bCountedTerminalV1::Accepted,
                        FoundationTerminalDisposition::ManualAccepted => {
                            G5bCountedTerminalV1::ManualAccepted
                        }
                        FoundationTerminalDisposition::Rejected => G5bCountedTerminalV1::Rejected,
                        FoundationTerminalDisposition::Uncertain => G5bCountedTerminalV1::Uncertain,
                        FoundationTerminalDisposition::ManualNotDelivered => {
                            G5bCountedTerminalV1::ManualNotDelivered
                        }
                    };
                    (
                        terminal,
                        verified.attempt_id,
                        Some(verified.ref_id),
                        Some(verified.evidence_sha256),
                        verified.accepted_channel,
                    )
                } else {
                    (G5bCountedTerminalV1::Pending, None, None, None, None)
                };
                observation = Some(G5bCountedObservationV1 {
                    decision_identity: stored.decision_identity,
                    terminal,
                    authoritative_attempt_identity: attempt,
                    immutable_audit_ref: audit_ref,
                    terminal_evidence_sha256: evidence_sha256,
                    accepted_channel,
                });
            }
            transaction.commit()?;
            Ok(observation)
        })
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn g5b_mismatch(detail: &'static str) -> DurableDeliveryError {
    DurableDeliveryError::PolicyMismatch(format!("G5b counted observation {detail}"))
}

fn validate_g5b_binding(
    stored: &StoredDecision,
    envelope: &DeliveryEnvelope,
    business_date: &str,
) -> Result<()> {
    // The stored retry flag may be revoked by a terminal disposition; the
    // producer's initial authorization remains frozen in the envelope.
    if envelope.decision_identity != stored.decision_identity
        || envelope.business_date != business_date
        || envelope.push_kind != PushKind::G5bAttribution
        || envelope.sub_kind != DeliverySubKind::None
        || envelope.cooldown_scope != CooldownScope::Global
        || envelope.scope_key != "GLOBAL"
        || !envelope.retry_authorized
        || envelope.task_binding.is_some()
        || stored.task_binding_present
        || envelope.foundation_binding().is_some()
        || envelope.provider_observed_at.is_some()
        || envelope.provider_as_of.is_some()
        || !envelope.original_batch_ids.is_empty()
    {
        return Err(g5b_mismatch("decision/envelope binding mismatch"));
    }
    let source: serde_json::Value = serde_json::from_slice(&envelope.source_binding_canonical)?;
    let fields = source
        .as_object()
        .ok_or_else(|| g5b_mismatch("source binding is not an object"))?;
    let string = |field| fields.get(field).and_then(serde_json::Value::as_str);
    let (Some(code), Some(triggered_at), Some(category), Some(message)) = (
        string("code"),
        string("triggered_at"),
        string("category"),
        string("message"),
    ) else {
        return Err(g5b_mismatch("source event fields are missing"));
    };
    let event_facts = format!("{triggered_at}|{code}|{category}|{message}");
    let occurrence = format!(
        "g5b-attribution:{business_date}:{code}:{}",
        sha256_hex(event_facts.as_bytes())
    );
    if fields.len() != 8
        || string("schema") != Some("g5b-attribution-v1")
        || string("business_date") != Some(business_date)
        || string("level").is_none()
        || string("rendered_sha256") != Some(envelope.rendered_content_sha256.as_str())
        || occurrence != envelope.schedule_occurrence_identity
        || serde_json::to_vec(&source)? != envelope.source_binding_canonical
    {
        return Err(g5b_mismatch("source binding content mismatch"));
    }
    let source_hash = sha256_hex(&envelope.source_binding_canonical);
    if envelope.source_evidence_fingerprint != source_hash
        || envelope.source_binding_sha256 != source_hash
        || envelope.delivery_subject_hash != source_hash
    {
        return Err(g5b_mismatch("source/subject digest mismatch"));
    }
    Ok(())
}
