//! Read original D-01 counted cards and their actual terminal evidence. This
//! never joins archive text, pushed_stocks, a later quote or a made-up receipt.
use super::{
    build_validated_terminal_evidence, load_current_disposition_evidence, load_decision,
    parse_envelope, sha256_hex, validate_authoritative_accepted_delivery_evidence, DecisionState,
    DeliveryEnvelope, DurableDeliveryCoordinator, DurableDeliveryError,
    FoundationTerminalDisposition, PushKind, Result,
};
use crate::durable_delivery::model::{validate_business_date, CooldownScope, DeliverySubKind};
use chrono::{DateTime, NaiveTime, Utc};
use rusqlite::{params, TransactionBehavior};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsToIdeaTerminal {
    Pending,
    Accepted,
    ManualAccepted,
    Rejected,
    Uncertain,
    ManualNotDelivered,
}

/// Only an attested read of the actual counted owner constructs this value.
/// V1 freezes code/date/render bytes, but has no source publication time or
/// push-price capability; these missing fields are never reconstructed.
#[derive(Debug)]
pub struct NewsToIdeaCardObservation {
    code: String,
    business_date: String,
    decision_identity: String,
    occurrence_identity: String,
    source_sha256: String,
    content_sha256: String,
    terminal: NewsToIdeaTerminal,
    accepted_at: Option<DateTime<Utc>>,
    terminal_evidence_sha256: Option<String>,
}
impl NewsToIdeaCardObservation {
    pub fn code(&self) -> &str {
        &self.code
    }
    pub fn business_date(&self) -> &str {
        &self.business_date
    }
    pub fn decision_identity(&self) -> &str {
        &self.decision_identity
    }
    pub fn occurrence_identity(&self) -> &str {
        &self.occurrence_identity
    }
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }
    pub fn terminal(&self) -> NewsToIdeaTerminal {
        self.terminal
    }
    pub fn accepted_at(&self) -> Option<DateTime<Utc>> {
        self.accepted_at
    }
    pub fn terminal_evidence_sha256(&self) -> Option<&str> {
        self.terminal_evidence_sha256.as_deref()
    }
}

fn mismatch(detail: &str) -> DurableDeliveryError {
    DurableDeliveryError::PolicyMismatch(format!("D-01 original card observation {detail}"))
}
fn source_code(envelope: &DeliveryEnvelope, business_date: &str) -> Result<String> {
    if envelope.business_date != business_date
        || envelope.push_kind != PushKind::NewsToIdea
        || envelope.sub_kind != DeliverySubKind::None
        || envelope.cooldown_scope != CooldownScope::PerTicket
        || envelope.retry_authorized
        || envelope.task_binding.is_some()
        || envelope.foundation_binding().is_some()
        || envelope.provider_observed_at.is_some()
        || envelope.provider_as_of.is_some()
        || !envelope.original_batch_ids.is_empty()
    {
        return Err(mismatch("envelope/source family mismatch"));
    }
    let source: serde_json::Value = serde_json::from_slice(&envelope.source_binding_canonical)?;
    let object = source
        .as_object()
        .ok_or_else(|| mismatch("source is not an object"))?;
    let code = object
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| mismatch("source code missing"))?;
    let identity = crate::data_gateway::instrument_identity::resolve_production_equity(code, None)
        .map_err(|e| mismatch(&e.to_string()))?;
    identity
        .require_a_share()
        .map_err(|e| mismatch(&e.to_string()))?;
    let exchange = match identity.instrument().exchange() {
        crate::market_domain::Exchange::Shanghai => "SHANGHAI",
        crate::market_domain::Exchange::Shenzhen => "SHENZHEN",
        crate::market_domain::Exchange::Beijing => "BEIJING",
    };
    let prefix = format!("news-to-idea:{business_date}:{code}:");
    let hhmm = envelope
        .schedule_occurrence_identity
        .strip_prefix(&prefix)
        .ok_or_else(|| mismatch("occurrence code/date mismatch"))?;
    let hash = sha256_hex(&envelope.source_binding_canonical);
    if object.len() != 4
        || source["schema"] != "news-to-idea-v1"
        || source["business_date"] != business_date
        || source["rendered_sha256"] != envelope.rendered_content_sha256
        || serde_json::to_vec(&source)? != envelope.source_binding_canonical
        || envelope.source_binding_sha256 != hash
        || envelope.source_evidence_fingerprint != hash
        || envelope.delivery_subject_hash != hash
        || envelope.scope_key != format!("{exchange}:EQUITY:{code}")
        || hhmm.len() != 5
        || !NaiveTime::parse_from_str(hhmm, "%H:%M")
            .is_ok_and(|time| time.format("%H:%M").to_string() == hhmm)
    {
        return Err(mismatch(
            "frozen entity/content/occurrence binding mismatch",
        ));
    }
    Ok(code.to_owned())
}

impl DurableDeliveryCoordinator {
    /// Keyset pages from the original owner, not raw pushed_stocks or archives.
    pub fn read_news_to_idea_business_dates(
        &self,
        through: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        validate_business_date(through)?;
        if let Some(after) = after {
            validate_business_date(after)?;
        }
        if !(1..=32).contains(&limit) {
            return Err(mismatch("date page must contain 1..=32 days"));
        }
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT business_date FROM delivery_decisions \
                 WHERE push_kind=?1 AND business_date<=?2 AND business_date>?3 \
                 ORDER BY business_date LIMIT ?4",
            )?;
            let rows = statement.query_map(
                params![
                    PushKind::NewsToIdea.as_str(),
                    through,
                    after.unwrap_or(""),
                    limit as i64
                ],
                |row| row.get::<_, String>(0),
            )?;
            let dates = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            for date in &dates {
                validate_business_date(date)?;
            }
            Ok(dates)
        })
    }

    pub fn read_news_to_idea_cards(
        &self,
        business_date: &str,
    ) -> Result<Vec<NewsToIdeaCardObservation>> {
        validate_business_date(business_date)?;
        self.with_connection(|connection| {
            let tx=connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let (rows,bytes):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(length(envelope_canonical)),0) FROM delivery_decisions WHERE business_date=?1 AND push_kind=?2",params![business_date,PushKind::NewsToIdea.as_str()],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if rows>4096 || bytes>16*1024*1024 {return Err(mismatch("read extent exceeded"));}
            let identities={
                let mut statement=tx.prepare("SELECT decision_identity FROM delivery_decisions WHERE business_date=?1 AND push_kind=?2 ORDER BY decision_identity")?;
                let read=statement.query_map(params![business_date,PushKind::NewsToIdea.as_str()],|r|r.get::<_,String>(0))?;
                read.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let mut observations=Vec::with_capacity(identities.len());let mut occurrences=BTreeSet::new();
            for identity in identities {
                let stored=load_decision(&tx,&identity)?.ok_or_else(||mismatch("decision disappeared"))?;
                let envelope=parse_envelope(&stored.envelope_canonical)?;
                if sha256_hex(&stored.envelope_canonical)!=stored.envelope_sha256 || envelope.canonical_bytes()?!=stored.envelope_canonical
                    || envelope.decision_identity!=stored.decision_identity || stored.retry_authorized || stored.task_binding_present
                    || !occurrences.insert(envelope.schedule_occurrence_identity.clone())
                {return Err(mismatch("stored original envelope mismatch"));}
                let code=source_code(&envelope,business_date)?;
                let (terminal,accepted_at,evidence)=if matches!(stored.state,DecisionState::Delivered | DecisionState::RejectedDurable | DecisionState::UncertainManualReview | DecisionState::ManualResolvedRejected) {
                    let verified=build_validated_terminal_evidence(&tx,&stored,&envelope,None)?;
                    let terminal=match verified.disposition {
                        FoundationTerminalDisposition::Accepted=>NewsToIdeaTerminal::Accepted,
                        FoundationTerminalDisposition::ManualAccepted=>NewsToIdeaTerminal::ManualAccepted,
                        FoundationTerminalDisposition::Rejected=>NewsToIdeaTerminal::Rejected,
                        FoundationTerminalDisposition::Uncertain=>NewsToIdeaTerminal::Uncertain,
                        FoundationTerminalDisposition::ManualNotDelivered=>NewsToIdeaTerminal::ManualNotDelivered,
                    };
                    let accepted_at=if terminal==NewsToIdeaTerminal::Accepted {
                        let disposition=load_current_disposition_evidence(&tx,&stored)?;
                        Some(validate_authoritative_accepted_delivery_evidence(&tx,&stored,&envelope,&disposition)?.1.accepted_at)
                    } else {None};
                    (terminal,accepted_at,Some(verified.evidence_sha256))
                } else {(NewsToIdeaTerminal::Pending,None,None)};
                observations.push(NewsToIdeaCardObservation {code,business_date:envelope.business_date,
                    decision_identity:stored.decision_identity,occurrence_identity:envelope.schedule_occurrence_identity,
                    source_sha256:envelope.source_binding_sha256,content_sha256:envelope.rendered_content_sha256,
                    terminal,accepted_at,terminal_evidence_sha256:evidence});
            }
            tx.commit()?;Ok(observations)
        })
    }
}
