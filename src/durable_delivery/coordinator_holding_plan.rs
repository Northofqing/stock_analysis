//! T03 fixed occurrence ownership. These observations never qualify a source,
//! grant a retry, or create a second delivery state machine.
use super::*;
use crate::durable_delivery::DeliverySubKind;
use crate::market_domain::{AssetClass, Exchange, InstrumentId};
use rusqlite::types::ValueRef;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoldingPlanReceiptKind {
    Pending,
    PhysicalAccepted,
    ManualAccepted,
    Rejected,
    Uncertain,
    ManualRejected,
}

/// A reader-issued snapshot of the exact immutable owner and current receipts.
/// Fields are private; a classification is not an admission/send capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HoldingPlanOwnedOccurrence {
    envelope: DeliveryEnvelope,
    state: DecisionState,
    retry_authorized: bool,
    receipt_kind: HoldingPlanReceiptKind,
    local_drained: bool,
    terminal_ref: Option<String>,
    evidence_sha256: Option<String>,
}
impl HoldingPlanOwnedOccurrence {
    pub fn envelope(&self) -> &DeliveryEnvelope {
        &self.envelope
    }
    pub fn state(&self) -> DecisionState {
        self.state
    }
    pub fn retry_authorized(&self) -> bool {
        self.retry_authorized
    }
    pub fn receipt_kind(&self) -> HoldingPlanReceiptKind {
        self.receipt_kind
    }
    pub fn local_drained(&self) -> bool {
        self.local_drained
    }
    pub fn terminal_ref(&self) -> Option<&str> {
        self.terminal_ref.as_deref()
    }
    pub fn evidence_sha256(&self) -> Option<&str> {
        self.evidence_sha256.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HoldingPlanOccurrenceObservation {
    Missing,
    Owned(HoldingPlanOwnedOccurrence),
}

#[derive(Clone, Debug)]
pub enum HoldingPlanPrepareOutcome {
    Prepared(PrepareOutcome),
    AlreadyOwned(HoldingPlanOwnedOccurrence),
}

fn invalid(reason: &str) -> DurableDeliveryError {
    DurableDeliveryError::PolicyMismatch(format!("holding_plan_{reason}"))
}

/// Stored routes use only original identity context. Legacy source bytes are
/// preserved by raw/audit recovery; they cannot be adopted by the strict reader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OccurrenceContext {
    business_date: String,
    scope_key: String,
}
impl OccurrenceContext {
    pub(super) fn from_envelope(e: &DeliveryEnvelope) -> Result<Option<Self>> {
        if e.push_kind != PushKind::HoldingPlan {
            return Ok(None);
        }
        super::super::model::validate_business_date(&e.business_date)?;
        Ok(Some(Self {
            business_date: e.business_date.clone(),
            scope_key: e.scope_key.clone(),
        }))
    }
    fn for_instrument(date: NaiveDate, instrument: &InstrumentId) -> Result<Self> {
        if instrument.asset_class() != AssetClass::Equity {
            return Err(invalid("ticket_not_equity"));
        }
        let exchange = match instrument.exchange() {
            Exchange::Shanghai => "SHANGHAI",
            Exchange::Shenzhen => "SHENZHEN",
            Exchange::Beijing => "BEIJING",
        };
        let code = instrument.code();
        if code.is_empty()
            || code.trim() != code
            || code.contains(':')
            || code.chars().any(char::is_control)
        {
            return Err(invalid("ticket_code_invalid"));
        }
        Ok(Self {
            business_date: date.format("%Y-%m-%d").to_string(),
            scope_key: format!("{exchange}:EQUITY:{code}"),
        })
    }
    fn code(&self) -> Result<&str> {
        let parts = self.scope_key.split(':').collect::<Vec<_>>();
        match parts.as_slice() {
            ["SHANGHAI" | "SHENZHEN" | "BEIJING", "EQUITY", code]
                if !code.is_empty()
                    && code.trim() == *code
                    && !code.chars().any(char::is_control) =>
            {
                Ok(code)
            }
            _ => Err(invalid("legacy_unknown_ticket")),
        }
    }
    fn occurrence(&self) -> Result<String> {
        Ok(format!(
            "holding-plan:{}:{}",
            self.business_date,
            self.code()?
        ))
    }
}

pub(super) fn validate_fresh_envelope(e: &DeliveryEnvelope) -> Result<OccurrenceContext> {
    e.validate()?;
    let context = OccurrenceContext::from_envelope(e)?.ok_or_else(|| invalid("wrong_kind"))?;
    if e.sub_kind != DeliverySubKind::None
        || e.cooldown_scope != super::super::model::CooldownScope::PerTicket
        || e.schedule_occurrence_identity != context.occurrence()?
    {
        return Err(invalid("legacy_unknown_occurrence"));
    }
    // The existing producer freezes serde_json::Value::to_string bytes. Exact
    // reencoding detects duplicate keys/noncanonical wire; it grants no source
    // eligibility and never substitutes its output for the original bytes.
    let source: serde_json::Value = serde_json::from_slice(&e.source_binding_canonical)
        .map_err(|_| invalid("legacy_unknown_source"))?;
    if serde_json::to_vec(&source)? != e.source_binding_canonical
        || source.get("schema_version").and_then(|v| v.as_str())
            != Some("HOLDING_PLAN_SOURCE_BINDING_V1")
        || source.get("code").and_then(|v| v.as_str()) != Some(context.code()?)
        || e.delivery_subject_hash != sha256_hex(&e.source_binding_canonical)
    {
        return Err(invalid("legacy_unknown_source"));
    }
    let observed = source
        .get("observed_at")
        .and_then(|v| v.as_str())
        .ok_or_else(|| invalid("legacy_unknown_observed_at"))?;
    let observed = DateTime::parse_from_rfc3339(observed)
        .map_err(|_| invalid("legacy_unknown_observed_at"))?;
    if observed
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 60 * 60).expect("valid Shanghai offset"))
        .date_naive()
        .format("%Y-%m-%d")
        .to_string()
        != context.business_date
    {
        return Err(invalid("observed_business_date_mismatch"));
    }
    Ok(context)
}

fn members(
    c: &Connection,
    context: &OccurrenceContext,
) -> Result<Vec<(StoredDecision, DeliveryEnvelope)>> {
    let mut query = c.prepare("SELECT decision_identity,business_date,push_kind,sub_kind,scope_key FROM delivery_decisions WHERE business_date=?1 AND push_kind='HoldingPlan' AND scope_key=?2 ORDER BY decision_identity")?;
    let ids = query
        .query_map(params![context.business_date, context.scope_key], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut result = Vec::new();
    for (id, date, kind, sub, scope) in ids {
        let stored = load_decision(c, &id)?
            .ok_or_else(|| DurableDeliveryError::DecisionNotFound(id.clone()))?;
        let e = parse_envelope(&stored.envelope_canonical)?;
        if e.canonical_bytes()? != stored.envelope_canonical
            || sha256_hex(&stored.envelope_canonical) != stored.envelope_sha256
            || e.decision_identity != id
            || e.business_date != date
            || e.push_kind.as_str() != kind
            || e.sub_kind.as_str() != sub
            || e.scope_key != scope
        {
            return Err(invalid("original_owner_binding_invalid"));
        }
        result.push((stored, e));
    }
    if result.len() > 1 {
        return Err(invalid("multiple_original_owners"));
    }
    Ok(result)
}

pub(super) fn load_strict_owner(
    c: &Connection,
    context: &OccurrenceContext,
) -> Result<Option<HoldingPlanOwnedOccurrence>> {
    let Some((stored, envelope)) = members(c, context)?.pop() else {
        return Ok(None);
    };
    if validate_fresh_envelope(&envelope)? != *context {
        return Err(invalid("original_context_changed"));
    }
    let pending: i64 = c.query_row("SELECT (SELECT COUNT(*) FROM immutable_audit_outbox WHERE decision_identity=?1 AND append_state!='Appended')+(SELECT COUNT(*) FROM delivery_disposition_payloads WHERE decision_identity=?1 AND append_state!='Appended')+(SELECT COUNT(*) FROM task_transition_payloads WHERE decision_identity=?1 AND (append_state!='Appended' OR hydration_state!='Applied'))", [&stored.decision_identity], |r| r.get(0))?;
    let mut result = HoldingPlanOwnedOccurrence {
        envelope,
        state: stored.state,
        retry_authorized: stored.retry_authorized,
        receipt_kind: HoldingPlanReceiptKind::Pending,
        local_drained: pending == 0,
        terminal_ref: None,
        evidence_sha256: None,
    };
    result.receipt_kind = match stored.state {
        DecisionState::RejectedDurable => HoldingPlanReceiptKind::Rejected,
        DecisionState::UncertainManualReview => HoldingPlanReceiptKind::Uncertain,
        DecisionState::ManualResolvedRejected => HoldingPlanReceiptKind::ManualRejected,
        _ => HoldingPlanReceiptKind::Pending,
    };
    // Noncompletion observations never expose a terminal success reference.
    // In particular the existing retry authorization may legally advance the
    // mutable flag while preserving the original Rejected receipt's flag.
    if stored.state == DecisionState::Delivered {
        // The original terminal verifier checks the exact envelope, current
        // attempt/disposition/raw receipt and immutable append references.
        let terminal = build_validated_terminal_evidence(c, &stored, &result.envelope, None)?;
        result.receipt_kind = match terminal.disposition {
            FoundationTerminalDisposition::Accepted => HoldingPlanReceiptKind::PhysicalAccepted,
            FoundationTerminalDisposition::ManualAccepted => HoldingPlanReceiptKind::ManualAccepted,
            _ => return Err(invalid("delivered_terminal_not_accepted")),
        };
        result.terminal_ref = Some(terminal.ref_id);
        result.evidence_sha256 = Some(terminal.evidence_sha256);
    }
    Ok(Some(result))
}

#[cfg(test)]
#[path = "actualholding_plan_occurrence_tests.rs"]
mod tests;

// Exact original SQL rows are captured after the one legitimate mutation, so
// Changed/NoChange effects need no guessed revision normalization. Each late
// evidence/ack operation is allowed to change its rows before this capture.
const AUTHORITY_TABLES: [&str; 17] = [
    "delivery_decisions",
    "immutable_audit_outbox",
    "cooldown_reservations",
    "business_date_once_claims",
    "daily_budget_reservations",
    "delivery_attempts",
    "sink_results",
    "delivery_correlation_observations",
    "review_terminal_replay_attempts",
    "review_terminal_replay_completions",
    "manual_resolutions",
    "delivery_disposition_payloads",
    "task_transition_payloads",
    "delivery_state_events",
    "delivery_attempt_events",
    "cooldown_reservation_events",
    "daily_budget_reservation_events",
];
fn frame(h: &mut Sha256, b: &[u8]) {
    h.update((b.len() as u64).to_be_bytes());
    h.update(b);
}
fn authority_sha(c: &Connection, context: &OccurrenceContext) -> Result<String> {
    let mut h = Sha256::new();
    frame(&mut h, b"holding-plan-exact-owner-sql-witness-v1");
    frame(&mut h, context.business_date.as_bytes());
    frame(&mut h, context.scope_key.as_bytes());
    let owners = members(c, context)?;
    for table in AUTHORITY_TABLES {
        frame(&mut h, table.as_bytes());
        let sql = format!("SELECT rowid,* FROM {table} WHERE decision_identity IN (SELECT decision_identity FROM delivery_decisions WHERE business_date=?1 AND push_kind='HoldingPlan' AND scope_key=?2) ORDER BY rowid");
        let mut query = c.prepare(&sql)?;
        for name in query.column_names() {
            frame(&mut h, name.as_bytes());
        }
        let columns = query.column_count();
        let mut rows = query.query(params![context.business_date, context.scope_key])?;
        while let Some(row) = rows.next()? {
            h.update([1]);
            for column in 0..columns {
                match row.get_ref(column)? {
                    ValueRef::Null => h.update([0]),
                    ValueRef::Integer(v) => {
                        h.update([1]);
                        frame(&mut h, &v.to_be_bytes());
                    }
                    ValueRef::Real(v) => {
                        h.update([2]);
                        frame(&mut h, &v.to_bits().to_be_bytes());
                    }
                    ValueRef::Text(v) => {
                        h.update([3]);
                        frame(&mut h, v);
                    }
                    ValueRef::Blob(v) => {
                        h.update([4]);
                        frame(&mut h, v);
                    }
                }
            }
        }
        h.update([0]);
    }
    frame(&mut h, &(owners.len() as u64).to_be_bytes());
    // Rolling's shared head is contextual to this Ticket, not a whole-DB
    // fingerprint. Preserve unrelated tickets' normal concurrent work.
    let mut query = c.prepare("SELECT push_kind,sub_kind,cooldown_scope,scope_key,current_reservation_identity,state,blocked_until,version FROM cooldown_heads WHERE push_kind='HoldingPlan' AND scope_key=?1 ORDER BY sub_kind,cooldown_scope")?;
    let columns = query.column_count();
    frame(&mut h, b"holding-plan-original-cooldown-head");
    let mut rows = query.query([&context.scope_key])?;
    while let Some(row) = rows.next()? {
        h.update([1]);
        for column in 0..columns {
            match row.get_ref(column)? {
                ValueRef::Null => h.update([0]),
                ValueRef::Integer(v) => {
                    h.update([1]);
                    frame(&mut h, &v.to_be_bytes());
                }
                ValueRef::Text(v) => {
                    h.update([3]);
                    frame(&mut h, v);
                }
                _ => return Err(invalid("cooldown_head_storage_type_invalid")),
            }
        }
    }
    h.update([0]);
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
pub(super) fn inject_sql_fault_for_test(
    coordinator: &DurableDeliveryCoordinator,
    tx: &Transaction<'_>,
    fault: OperationPostvalidationTestFault,
) -> Result<()> {
    if !matches!(
        coordinator.config.environment,
        super::super::model::StoreEnvironment::Test { .. }
    ) {
        return Err(invalid("test_fault_production_forbidden"));
    }
    let id: String = tx.query_row("SELECT decision_identity FROM delivery_decisions WHERE push_kind='HoldingPlan' ORDER BY decision_identity LIMIT 1", [], |r| r.get(0))?;
    if fault == OperationPostvalidationTestFault::HoldingPlanExtraAudit {
        enqueue_audit(
            tx,
            &id,
            None,
            "RecoveryClassified",
            &canonical_json(&json!({"test_fault":"TEST_CODE_T03_EXTRA_AUDIT"}))?,
            DateTime::parse_from_rfc3339("2026-07-30T08:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )?;
        return Ok(());
    }
    let stored = load_decision(tx, &id)?
        .ok_or_else(|| DurableDeliveryError::DecisionNotFound(id.clone()))?;
    let old = parse_envelope(&stored.envelope_canonical)?;
    let mut rendered = old.rendered_content.clone();
    rendered.extend_from_slice(b" TEST_CODE_DUPLICATE_OWNER");
    let duplicate = DeliveryEnvelope::new(
        old.business_date,
        old.push_kind,
        old.sub_kind,
        old.scope_key,
        old.schedule_occurrence_identity,
        old.source_evidence_fingerprint,
        old.source_binding_canonical,
        old.delivery_subject_hash,
        rendered,
        old.retry_authorized,
        old.task_binding,
    )?;
    let raw = duplicate.canonical_bytes()?;
    let at = DateTime::parse_from_rfc3339("2026-07-30T08:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    insert_new_decision(
        tx,
        &duplicate,
        &raw,
        &sha256_hex(&raw),
        DecisionState::Reserved,
        at,
    )?;
    record_state_transition(
        tx,
        &duplicate.decision_identity,
        None,
        DecisionState::Reserved,
        "prepare",
        None,
        canonical_json(&json!({"envelope_sha256":sha256_hex(&raw),"reservation_generation":1}))?,
        at,
    )?;
    let policy = load_policy(tx, duplicate.push_kind, duplicate.sub_kind)?;
    coordinator.reserve_generation(tx, &duplicate, &policy, 1, at)
}
pub(super) struct SqlBinding {
    context: OccurrenceContext,
    authority_sha256: String,
}
pub(super) fn capture_sql_binding(
    c: &Connection,
    context: &OccurrenceContext,
) -> Result<SqlBinding> {
    Ok(SqlBinding {
        context: context.clone(),
        authority_sha256: authority_sha(c, context)?,
    })
}
impl SqlBinding {
    pub(super) fn validate(&self, c: &Connection) -> Result<()> {
        if authority_sha(c, &self.context)? != self.authority_sha256 {
            return Err(invalid("exact_owner_sql_witness_changed"));
        }
        Ok(())
    }
}

impl DurableDeliveryCoordinator {
    /// Observe one fixed T03 occurrence. No DDL, admission, budget, provider,
    /// renderer or sink is invoked. Legacy/ambiguous owners fail closed.
    pub fn inspect_holding_plan_occurrence(
        &self,
        date: NaiveDate,
        instrument: &InstrumentId,
    ) -> Result<HoldingPlanOccurrenceObservation> {
        let context = OccurrenceContext::for_instrument(date, instrument)?;
        let (observation, binding) = self.with_connection(|c| {
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)?;
            let owner = load_strict_owner(&tx, &context)?;
            let binding = capture_sql_binding(&tx, &context)?;
            tx.commit()?;
            binding.validate(c)?;
            Ok((
                owner.map_or(
                    HoldingPlanOccurrenceObservation::Missing,
                    HoldingPlanOccurrenceObservation::Owned,
                ),
                binding,
            ))
        })?;
        // The first read's core post-SQL/FS hooks have all returned. A second
        // actual read rejects a stale observation without replaying any ports.
        self.with_mutation_routing_connection(|c| binding.validate(c))?;
        Ok(observation)
    }

    /// Uses the sole prepare body. A competing fresh card returns its original
    /// owner without allocating a denial, reservation, budget or audit row.
    pub fn prepare_holding_plan_occurrence(
        &self,
        envelope: &DeliveryEnvelope,
        authoritative_sink_count: usize,
        admission_at: DateTime<Utc>,
    ) -> Result<HoldingPlanPrepareOutcome> {
        let context = validate_fresh_envelope(envelope)?;
        let outcome = self.prepare_internal_owner_outcome(
            envelope,
            authoritative_sink_count,
            admission_at,
            None,
        )?;
        if outcome.decision_identity == envelope.decision_identity {
            return Ok(HoldingPlanPrepareOutcome::Prepared(outcome));
        }
        let date = NaiveDate::parse_from_str(&context.business_date, "%Y-%m-%d")
            .map_err(|_| invalid("date_invalid"))?;
        let mut pieces = context.scope_key.split(':');
        let exchange = match pieces.next() {
            Some("SHANGHAI") => Exchange::Shanghai,
            Some("SHENZHEN") => Exchange::Shenzhen,
            Some("BEIJING") => Exchange::Beijing,
            _ => return Err(invalid("ticket_invalid")),
        };
        pieces.next();
        let instrument = InstrumentId::new(exchange, context.code()?, AssetClass::Equity)
            .map_err(|_| invalid("ticket_invalid"))?;
        match self.inspect_holding_plan_occurrence(date, &instrument)? {
            HoldingPlanOccurrenceObservation::Owned(owner)
                if owner.envelope.decision_identity == outcome.decision_identity =>
            {
                Ok(HoldingPlanPrepareOutcome::AlreadyOwned(owner))
            }
            _ => Err(invalid("original_winner_changed")),
        }
    }
}
