//! P05 actual counted owners, mutation revisions and read-only receipt finalization.
use super::*;
use rusqlite::types::ValueRef;
use sha2::{Digest, Sha256};

const AUTHORITY_TABLES: [&str; 18] = [
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
    "p05_s2_child_owners",
];

fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
// Stream original SQLite bytes; raw/late evidence is never truncated to fit a
// receipt DTO and no operational DB/FS lock is acquired under this connection.
fn authority_sha(c: &Connection, draft: &str, empty: bool) -> Result<String> {
    let mut h = Sha256::new();
    frame(&mut h, b"p05-unit-counted-authority-v1");
    frame(&mut h, draft.as_bytes());
    for table in AUTHORITY_TABLES {
        frame(&mut h, table.as_bytes());
        let sql=format!("SELECT rowid,* FROM {table} WHERE decision_identity IN (SELECT decision_identity FROM p05_unit_children WHERE draft_identity=?1) ORDER BY rowid");
        let mut query = c.prepare(&sql)?;
        for name in query.column_names() {
            frame(&mut h, name.as_bytes());
        }
        if empty {
            h.update([0]);
            continue;
        }
        let columns = query.column_count();
        let mut rows = query.query([draft])?;
        while let Some(row) = rows.next()? {
            h.update([1]);
            for col in 0..columns {
                match row.get_ref(col)? {
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
    Ok(hex::encode(h.finalize()))
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct OwnerCanonical {
    schema: String,
    draft_identity: String,
    intent_identity: String,
    child_identity: String,
    decision_identity: String,
    envelope_sha256: String,
}
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct MutationCanonical {
    schema: String,
    draft_identity: String,
    mutation_revision: i64,
    decision_identity: String,
    before_sha256: String,
    after_sha256: String,
}
pub(in crate::durable_delivery::coordinator) struct MutationBefore {
    draft: String,
    revision: i64,
    fingerprint: String,
}
pub(in crate::durable_delivery::coordinator) fn before_mutation(
    c: &Connection,
    route: &DecisionMutationRoute,
) -> Result<Option<MutationBefore>> {
    let draft: Option<String> = c
        .query_row(
            "SELECT draft_identity FROM p05_unit_children WHERE decision_identity=?1",
            [&route.decision_identity],
            |r| r.get(0),
        )
        .optional()?;
    let Some(draft) = draft else {
        return Ok(None);
    };
    let (revision, phase): (i64, String) = c.query_row(
        "SELECT mutation_revision,phase FROM p05_unit_heads WHERE draft_identity=?1",
        [&draft],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if phase != "IntentComplete" {
        return Err(invalid("counted mutation before complete child set"));
    }
    Ok(Some(MutationBefore {
        fingerprint: authority_sha(c, &draft, false)?,
        draft,
        revision,
    }))
}
pub(in crate::durable_delivery::coordinator) fn verify_no_change(
    c: &Connection,
    before: Option<MutationBefore>,
) -> Result<()> {
    if let Some(before) = before {
        let revision: i64 = c.query_row(
            "SELECT mutation_revision FROM p05_unit_heads WHERE draft_identity=?1",
            [&before.draft],
            |r| r.get(0),
        )?;
        if revision != before.revision
            || authority_sha(c, &before.draft, false)? != before.fingerprint
        {
            return Err(invalid("NoChange altered Unit authority"));
        }
    }
    Ok(())
}
pub(in crate::durable_delivery::coordinator) fn after_mutation(
    tx: &Transaction<'_>,
    before: Option<MutationBefore>,
    route: &DecisionMutationRoute,
) -> Result<()> {
    let Some(before) = before else {
        return Ok(());
    };
    let after = authority_sha(tx, &before.draft, false)?;
    // Some legacy bodies return Changed for an already-enqueued exact audit.
    // The Unit revision tracks actual authority bytes, never an attempted call.
    if before.fingerprint == after {
        return Ok(());
    }
    let revision = before
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("Unit revision exhausted"))?;
    let data = MutationCanonical {
        schema: "p05-counted-mutation-v1".into(),
        draft_identity: before.draft.clone(),
        mutation_revision: revision,
        decision_identity: route.decision_identity.clone(),
        before_sha256: before.fingerprint,
        after_sha256: after,
    };
    let canonical = encode(&data)?;
    let image = preimage("p05-counted-mutation-v1", &canonical);
    let id = sha256_hex(&image);
    tx.execute("INSERT INTO p05_s2_mutation_events(event_identity,draft_identity,mutation_revision,decision_identity,before_sha256,after_sha256,event_canonical,event_sha256,event_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,data.draft_identity,revision,data.decision_identity,data.before_sha256,data.after_sha256,canonical,sha256_hex(&canonical),image])?;
    require_single_cas_update(tx.execute("UPDATE p05_unit_heads SET mutation_revision=?1 WHERE draft_identity=?2 AND mutation_revision=?3 AND phase='IntentComplete'",params![revision,data.draft_identity,before.revision])?,"P05 actual mutation revision")?;
    tx.execute("UPDATE p05_s2_completion_heads SET current_completion_identity=NULL WHERE draft_identity=?1",[&data.draft_identity])?;
    Ok(())
}

// Only the contextual reader creates this admission after its independent
// actual operational snapshot. Caller JSON/hash/enum cannot create one.
pub(in crate::durable_delivery::coordinator) struct ChildAdmission<'a> {
    intent: &'a StoredP05Intent,
    child: &'a str,
    envelope: &'a DeliveryEnvelope,
}

/// A single released actual operational read, never a disk/JSON authority.
/// It is namespace/member-bound and consumed only at a real business opening.
#[derive(Clone)]
pub(in crate::durable_delivery::coordinator) enum ActualP05ConsumerCheck {
    Unowned,
    Failed(&'static str),
    Verified(ActualP05ConsumerCapability),
}
#[derive(Clone)]
pub(in crate::durable_delivery::coordinator) struct ActualP05ConsumerCapability {
    namespace: FileObjectIdentity,
    date: String,
    draft: String,
    intent: String,
    child: String,
    decision: String,
    envelope: Vec<u8>,
    prediction: PredictionObservation,
}
impl ActualP05ConsumerCheck {
    fn require(&self, c: &Connection, route: &DecisionMutationRoute) -> Result<()> {
        let date = mutation_date(c, route)?;
        match (date, self) {
            (None, Self::Unowned) => Ok(()),
            (None, _) => Err(invalid("consumer check no longer matches owned route")),
            (Some(_), Self::Unowned) => {
                Err(invalid("owned P05 business opening needs actual reader"))
            }
            (Some(_), Self::Failed(reason)) => Err(invalid(reason)),
            (Some(date), Self::Verified(cap))
                if date == cap.date && route.decision_identity == cap.decision =>
            {
                cap.validate(c)
            }
            _ => Err(invalid("actual consumer belongs to another Unit/member")),
        }
    }
}
impl ActualP05ConsumerCapability {
    fn validate(&self, c: &Connection) -> Result<()> {
        let draft = load_draft_inner(c, &self.date, self.namespace, false)?
            .ok_or_else(|| invalid("actual consumer draft absent"))?;
        let intent = load_intent_inner(c, &self.date, self.namespace, false)?
            .ok_or_else(|| invalid("actual consumer full intent absent"))?;
        let data: IntentCanonical = decode(&intent.canonical)?;
        if draft.identity != self.draft
            || intent.identity != self.intent
            || data.prediction != self.prediction
            || !intent
                .children
                .iter()
                .any(|(child, raw)| child == &self.child && raw == &self.envelope)
            || parse_envelope(&self.envelope)?.decision_identity != self.decision
        {
            return Err(invalid("actual consumer original intent/member changed"));
        }
        Ok(())
    }
}

impl DurableDeliveryCoordinator {
    fn p05_consumer_recipe(
        &self,
        route: &DecisionMutationRoute,
    ) -> Result<Option<(StoredP05Draft, StoredP05Intent, String, Vec<u8>)>> {
        let namespace = self.p05_namespace()?;
        self.with_mutation_routing_connection(|c| {
            let Some(date) = mutation_date(c, route)? else {
                return Ok(None);
            };
            let draft = load_draft(c, &date, namespace)?
                .ok_or_else(|| invalid("actual consumer draft absent"))?;
            let intent = load_intent(c, &date, namespace)?
                .ok_or_else(|| invalid("actual consumer full intent absent"))?;
            let (child, raw) = intent
                .children
                .iter()
                .find(|(_, raw)| {
                    parse_envelope(raw)
                        .is_ok_and(|e| e.decision_identity == route.decision_identity)
                })
                .ok_or_else(|| invalid("actual consumer original member absent"))?;
            let child = child.clone();
            let raw = raw.clone();
            Ok(Some((draft, intent, child, raw)))
        })
    }
    fn p05_consumer_check_from_recipe(
        &self,
        db: &DatabaseManager,
        recipe: Option<(StoredP05Draft, StoredP05Intent, String, Vec<u8>)>,
    ) -> ActualP05ConsumerCheck {
        let Some((draft, intent, child, raw)) = recipe else {
            return ActualP05ConsumerCheck::Unowned;
        };
        let decision = match parse_envelope(&raw) {
            Ok(envelope) => envelope.decision_identity,
            Err(_) => return ActualP05ConsumerCheck::Failed("P05 actual original member invalid"),
        };
        let observation = read_actual_prediction(db, &draft, &intent);
        match observation {
            Ok(prediction) => ActualP05ConsumerCheck::Verified(ActualP05ConsumerCapability {
                namespace: draft.namespace,
                date: draft.data.business_date,
                draft: draft.identity,
                intent: intent.identity,
                child,
                decision,
                envelope: raw,
                prediction,
            }),
            Err(_) => ActualP05ConsumerCheck::Failed(
                "P05 actual operational freeze/score unavailable or mismatched",
            ),
        }
    }
    pub(in crate::durable_delivery::coordinator) fn p05_consumer_check_global(
        &self,
        route: &DecisionMutationRoute,
    ) -> ActualP05ConsumerCheck {
        let recipe = match self.p05_consumer_recipe(route) {
            Ok(None) => return ActualP05ConsumerCheck::Unowned,
            Ok(recipe) => recipe,
            Err(_) => {
                return ActualP05ConsumerCheck::Failed(
                    "P05 actual owned consumer recipe unavailable",
                )
            }
        };
        if !matches!(
            &self.config.environment,
            super::super::super::model::StoreEnvironment::Production
        ) {
            return ActualP05ConsumerCheck::Failed(
                "Test P05 opening requires actual isolated consumer capability",
            );
        }
        let Some(db) = DatabaseManager::try_get() else {
            return ActualP05ConsumerCheck::Failed("P05 actual operational singleton unavailable");
        };
        self.p05_consumer_check_from_recipe(db, recipe)
    }
    fn p05_consumer_check_on(
        &self,
        db: &DatabaseManager,
        route: &DecisionMutationRoute,
    ) -> ActualP05ConsumerCheck {
        match &self.config.environment {
            super::super::super::model::StoreEnvironment::Production => {
                if !DatabaseManager::try_get().is_some_and(|actual| std::ptr::eq(actual, db)) {
                    return ActualP05ConsumerCheck::Failed(
                        "Production P05 consumer requires actual singleton",
                    );
                }
            }
            super::super::super::model::StoreEnvironment::Test { .. } => {
                #[cfg(test)]
                if !db.has_isolated_p05_consumer_origin() {
                    return ActualP05ConsumerCheck::Failed(
                        "Test P05 consumer DB lacks isolated constructor origin",
                    );
                }
                #[cfg(not(test))]
                return ActualP05ConsumerCheck::Failed(
                    "Test P05 consumer capability unavailable in production library",
                );
            }
        }
        let recipe = match self.p05_consumer_recipe(route) {
            Ok(recipe) => recipe,
            Err(_) => {
                return ActualP05ConsumerCheck::Failed(
                    "P05 actual owned consumer recipe unavailable",
                )
            }
        };
        let check = self.p05_consumer_check_from_recipe(db, recipe);
        #[cfg(test)]
        if matches!(
            &self.config.environment,
            super::super::super::model::StoreEnvironment::Test { .. }
        ) && !db.has_isolated_p05_consumer_origin()
        {
            return ActualP05ConsumerCheck::Failed(
                "Test P05 consumer isolation changed during actual read",
            );
        }
        check
    }
    pub(in crate::durable_delivery::coordinator) fn bind_p05_consumer_check(
        &self,
        check: &ActualP05ConsumerCheck,
    ) -> ActualP05ConsumerCheck {
        if let ActualP05ConsumerCheck::Verified(cap) = check {
            if self.p05_namespace().ok() != Some(cap.namespace) {
                return ActualP05ConsumerCheck::Failed(
                    "actual P05 consumer belongs to another durable namespace",
                );
            }
        }
        check.clone()
    }
    pub(in crate::durable_delivery::coordinator) fn with_p05_business_mutation_transaction<T>(
        &self,
        route: &DecisionMutationRoute,
        supplied: Option<&ActualP05ConsumerCheck>,
        operation: impl FnOnce(&Transaction<'_>, &SqlDependencies) -> Result<MutationEffect<T>>,
    ) -> Result<T> {
        let check = supplied
            .map(|check| self.bind_p05_consumer_check(check))
            .unwrap_or_else(|| self.p05_consumer_check_global(route));
        self.with_mutation_transaction_consumer(route, Some(check), operation)
    }
}
pub(in crate::durable_delivery::coordinator) fn validate_new_prepare(
    c: &Connection,
    envelope: &DeliveryEnvelope,
    admission: Option<&ChildAdmission<'_>>,
) -> Result<()> {
    if !affected(envelope.push_kind) {
        return Ok(());
    }
    let unit: Option<String> = c
        .query_row(
            "SELECT draft_identity FROM p05_unit_drafts WHERE business_date=?1",
            [&envelope.business_date],
            |r| r.get(0),
        )
        .optional()?;
    let Some(unit) = unit else {
        return if admission.is_none() {
            Ok(())
        } else {
            Err(invalid("contextual child has no owned Unit"))
        };
    };
    let a = admission
        .ok_or_else(|| invalid("fresh affected family requires private Unit child admission"))?;
    let canonical = envelope.canonical_bytes()?;
    if a.intent.draft_identity != unit
        || *a.envelope != *envelope
        || !a
            .intent
            .children
            .iter()
            .any(|(id, raw)| id == a.child && raw == &canonical)
    {
        return Err(invalid(
            "contextual child differs from full immutable intent",
        ));
    }
    let stored = load_intent_inner(c, &envelope.business_date, zero_namespace(), false)?
        .ok_or_else(|| invalid("complete intent absent"))?;
    if stored != *a.intent {
        return Err(invalid("contextual intent changed"));
    }
    Ok(())
}
fn affected(kind: PushKind) -> bool {
    matches!(
        kind,
        PushKind::AuctionRepush | PushKind::CandidateBoard | PushKind::CandidateInvalidated
    )
}
fn zero_namespace() -> FileObjectIdentity {
    FileObjectIdentity {
        device: 0,
        inode: 0,
        mode: 0,
        uid: 0,
    }
}
pub(in crate::durable_delivery::coordinator) fn validate_owner(
    c: &Connection,
    envelope: &DeliveryEnvelope,
) -> Result<()> {
    let row:Option<(String,String,String,String,Vec<u8>,String,Vec<u8>,String,Vec<u8>)>=c.query_row("SELECT owner_identity,child_identity,draft_identity,intent_identity,envelope_canonical,envelope_sha256,owner_canonical,owner_sha256,owner_preimage FROM p05_s2_child_owners WHERE decision_identity=?1",[&envelope.decision_identity],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?;
    let (id, child, draft, intent, raw, raw_sha, bytes, hash, image) =
        row.ok_or_else(|| invalid("Unit decision has no exact immutable child owner"))?;
    verify_blob(&id, &bytes, &hash, &image, "p05-unit-child-owner-v1")?;
    let data: OwnerCanonical = decode(&bytes)?;
    let actual = load_intent_inner(c, &envelope.business_date, zero_namespace(), false)?
        .ok_or_else(|| invalid("child owner intent absent"))?;
    if data.schema != "p05-unit-child-owner-v1"
        || data.draft_identity != draft
        || data.intent_identity != intent
        || data.child_identity != child
        || data.decision_identity != envelope.decision_identity
        || data.envelope_sha256 != raw_sha
        || sha256_hex(&raw) != raw_sha
        || raw != envelope.canonical_bytes()?
        || actual.draft_identity != draft
        || actual.identity != intent
        || !actual
            .children
            .iter()
            .any(|(i, b)| i == &child && b == &raw)
    {
        return Err(invalid(
            "child owner differs from exact original intent/envelope",
        ));
    }
    let stored = load_decision(c, &envelope.decision_identity)?
        .ok_or_else(|| invalid("child owner decision absent"))?;
    if stored.envelope_canonical != raw || stored.envelope_sha256 != raw_sha {
        return Err(invalid("owner decision bytes differ"));
    }
    Ok(())
}
pub(in crate::durable_delivery::coordinator) fn require_business_open(
    c: &Connection,
    route: &DecisionMutationRoute,
) -> Result<()> {
    let draft: Option<String> = c
        .query_row(
            "SELECT draft_identity FROM p05_s2_child_owners WHERE decision_identity=?1",
            [&route.decision_identity],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(draft) = draft {
        let closed: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM p05_s2_completion_heads WHERE draft_identity=?1)",
            [&draft],
            |r| r.get(0),
        )?;
        if closed {
            return Err(invalid(
                "Unit business opening closed by immutable first completion",
            ));
        }
        let date: String = c.query_row(
            "SELECT business_date FROM p05_unit_drafts WHERE draft_identity=?1",
            [&draft],
            |r| r.get(0),
        )?;
        let original = load_draft(c, &date, zero_namespace())?
            .ok_or_else(|| invalid("business-opening Unit absent"))?;
        if original.data.baseline_kind == "CompletedUnitV2Baseline" {
            require_current_baseline(c, &original.data.baseline_origin_identity)?;
        }
    }
    Ok(())
}

impl DurableDeliveryCoordinator {
    pub(crate) fn prepare_p05_unit_child_on(
        &self,
        prediction_db: &DatabaseManager,
        date: &str,
        index: usize,
        sink_count: usize,
    ) -> Result<(PrepareOutcome, DeliveryEnvelope)> {
        self.prepare_p05_unit_child_local(prediction_db, date, index, sink_count, None)
    }
    fn prepare_p05_unit_child_local(
        &self,
        prediction_db: &DatabaseManager,
        date: &str,
        index: usize,
        sink_count: usize,
        test_now: Option<DateTime<Utc>>,
    ) -> Result<(PrepareOutcome, DeliveryEnvelope)> {
        if test_now.is_some() {
            self.require_p05_test_clock()?;
        }
        let intent = self
            .read_p05_unit_intent(date)?
            .ok_or_else(|| invalid("all required children must be committed before admission"))?;
        let (_, raw) = intent
            .children
            .get(index)
            .ok_or_else(|| invalid("child ordinal absent"))?;
        let envelope = parse_envelope(raw)?;
        let route = self.prepare_mutation_route(&envelope)?;
        let actual = self.p05_consumer_check_on(prediction_db, &route);
        self.prepare_p05_unit_child_checked(
            date,
            index,
            sink_count,
            test_now.unwrap_or_else(Utc::now),
            &actual,
        )
    }
    fn prepare_p05_unit_child_checked(
        &self,
        date: &str,
        index: usize,
        sink_count: usize,
        now: DateTime<Utc>,
        actual: &ActualP05ConsumerCheck,
    ) -> Result<(PrepareOutcome, DeliveryEnvelope)> {
        let intent = self
            .read_p05_unit_intent(date)?
            .ok_or_else(|| invalid("all required children must be committed before admission"))?;
        let (child, raw) = intent
            .children
            .get(index)
            .ok_or_else(|| invalid("child ordinal absent"))?;
        let envelope = parse_envelope(raw)?;
        let sha = sha256_hex(raw);
        let admission = ChildAdmission {
            intent: &intent,
            child,
            envelope: &envelope,
        };
        let route = self.prepare_mutation_route(&envelope)?;
        let effect=self.with_p05_business_mutation_transaction(&route,Some(actual),|tx,dependencies| {
            validate_new_prepare(tx,&envelope,Some(&admission))?;
            if load_decision(tx,&envelope.decision_identity)?.is_some() {validate_owner(tx,&envelope)?;}
            else {
                dependencies.require_business_open(tx,&route)?;
                let data=OwnerCanonical {schema:"p05-unit-child-owner-v1".into(),draft_identity:intent.draft_identity.clone(),intent_identity:intent.identity.clone(),child_identity:child.clone(),decision_identity:envelope.decision_identity.clone(),envelope_sha256:sha.clone()};
                let canonical=encode(&data)?;let image=preimage("p05-unit-child-owner-v1",&canonical);let id=sha256_hex(&image);
                tx.execute("INSERT INTO p05_s2_child_owners(owner_identity,child_identity,draft_identity,intent_identity,decision_identity,envelope_canonical,envelope_sha256,owner_canonical,owner_sha256,owner_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![id,child,intent.draft_identity,intent.identity,envelope.decision_identity,raw,sha,canonical,sha256_hex(&canonical),image])?;
            }
            let result=self.prepare_transaction_body(tx,&route,&envelope,raw,&sha,sink_count,now,None,None,Some(&admission))?;
            validate_owner(tx,&envelope)?;
            if matches!(&result,MutationEffect::Changed(PrepareTransactionOutcome::Inserted)) {
                dependencies.require_business_open(tx,&route)?;
            }
            Ok(result)
        })?;
        let outcome = match effect {
            PrepareTransactionOutcome::Existing(value) => *value,
            PrepareTransactionOutcome::IdentityConflict => {
                return Err(DurableDeliveryError::DecisionIdentityConflict {
                    decision_identity: envelope.decision_identity.clone(),
                })
            }
            PrepareTransactionOutcome::Inserted => self.with_connection(|c| {
                let stored = load_decision(c, &envelope.decision_identity)?
                    .ok_or_else(|| invalid("prepared child absent"))?;
                Ok(outcome_from_stored(
                    &stored,
                    &load_schedule_hydration(c, &envelope.decision_identity)?,
                ))
            })?,
        };
        Ok((outcome, envelope))
    }
}

fn verify_actual_prediction(
    db: &DatabaseManager,
    draft: &StoredP05Draft,
    intent: &StoredP05Intent,
) -> Result<()> {
    read_actual_prediction(db, draft, intent).map(|_| ())
}
fn read_actual_prediction(
    db: &DatabaseManager,
    draft: &StoredP05Draft,
    intent: &StoredP05Intent,
) -> Result<PredictionObservation> {
    let data: IntentCanonical = decode(&intent.canonical)?;
    let actual = db
        .read_p05_unit_freeze_with_scores(&board_occurrence(&draft.data)?)
        .map_err(|_| invalid("actual prediction verification unknown"))?;
    match (&data.prediction, actual) {
        (PredictionObservation::Frozen { .. }, Some(actual)) => {
            validate_actual_freeze(&draft.data, &actual)?;
            if freeze_observation(&actual) != data.prediction {
                return Err(invalid(
                    "actual prediction observation differs from immutable intent",
                ));
            }
            Ok(data.prediction.clone())
        }
        (PredictionObservation::UnlinkedNoStrong { .. }, None)
            if draft.data.strong_recipe.is_empty() =>
        {
            Ok(data.prediction.clone())
        }
        _ => Err(invalid(
            "original prediction freeze/absence differs; no fallback or resave",
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditReference {
    identity: String,
    sha256: String,
    immutable_ref: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedReference {
    child_identity: String,
    owner_identity: String,
    decision_identity: String,
    disposition_identity: String,
    attempt_identity: String,
    channel: String,
    raw_accepted: Vec<u8>,
    raw_sha256: String,
    audit_refs: Vec<AuditReference>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionCanonical {
    schema: String,
    family: String,
    business_date: String,
    draft_identity: String,
    intent_identity: String,
    mutation_revision: i64,
    authority_sha256: String,
    current_codes: Vec<String>,
    accepted: Vec<AcceptedReference>,
    observed_utc: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletedOrigin {
    schema: String,
    family: String,
    business_date: String,
    kind: String,
    completed_unit_identity: String,
    completed_receipt_identity: String,
    current_codes: Vec<String>,
    accepted_physical_refs: Vec<AcceptedReference>,
}
/// Opaque revision-bound read snapshot, never permission or future-current proof.
/// Its same-call boundary is rechecked; later mutation requires a fresh read/CAS.
/// It cannot be deserialized into authorization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct P05UnitReceiptObservation {
    namespace: Option<FileObjectIdentity>,
    draft_identity: String,
    mutation_revision: i64,
    completion_identity: Option<String>,
    children: Vec<P05ChildReceiptObservation>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum P05ChildReceiptObservation {
    NotPrepared {
        child_identity: String,
    },
    Pending {
        child_identity: String,
        decision_identity: String,
        state: DecisionState,
    },
    PhysicallyAccepted {
        child_identity: String,
        decision_identity: String,
        disposition_identity: String,
        attempt_identity: String,
        raw_sha256: String,
    },
    NonAccepted {
        child_identity: String,
        decision_identity: String,
        disposition: P05NonAcceptedTerminal,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum P05NonAcceptedTerminal {
    ManualAccepted,
    Rejected,
    Uncertain,
    ManualNotDelivered,
}
fn nonaccepted(disposition: FoundationTerminalDisposition) -> Result<P05NonAcceptedTerminal> {
    match disposition {
        FoundationTerminalDisposition::ManualAccepted => Ok(P05NonAcceptedTerminal::ManualAccepted),
        FoundationTerminalDisposition::Rejected => Ok(P05NonAcceptedTerminal::Rejected),
        FoundationTerminalDisposition::Uncertain => Ok(P05NonAcceptedTerminal::Uncertain),
        FoundationTerminalDisposition::ManualNotDelivered => {
            Ok(P05NonAcceptedTerminal::ManualNotDelivered)
        }
        FoundationTerminalDisposition::Accepted => {
            Err(invalid("Accepted cannot be classified as NonAccepted"))
        }
    }
}
impl P05UnitReceiptObservation {
    pub(crate) fn draft_identity(&self) -> &str {
        &self.draft_identity
    }
    pub(crate) fn mutation_revision(&self) -> i64 {
        self.mutation_revision
    }
    pub(crate) fn completion_identity(&self) -> Option<&str> {
        self.completion_identity.as_deref()
    }
    pub(crate) fn children(&self) -> &[P05ChildReceiptObservation] {
        &self.children
    }
}
fn audit_refs(c: &Connection, decision: &str) -> Result<Vec<AuditReference>> {
    let rows=c.prepare("SELECT audit_identity,audit_sha256,immutable_audit_ref FROM immutable_audit_outbox WHERE decision_identity=?1 AND append_state='Appended' ORDER BY audit_identity")?.query_map([decision],|r|Ok(AuditReference {identity:r.get(0)?,sha256:r.get(1)?,immutable_ref:r.get(2)?}))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
fn validate_audit_refs(c: &Connection, decision: &str, refs: &[AuditReference]) -> Result<()> {
    let current = audit_refs(c, decision)?;
    if refs.is_empty()
        || refs.windows(2).any(|w| w[0].identity >= w[1].identity)
        || refs.iter().any(|r| {
            r.immutable_ref.trim().is_empty() || !is_lower_sha256(&r.sha256) || !current.contains(r)
        })
    {
        return Err(invalid("immutable physical audit witness differs"));
    }
    Ok(())
}
fn accepted_reference(
    c: &Connection,
    child: &str,
    envelope: &DeliveryEnvelope,
) -> Result<Option<AcceptedReference>> {
    let Some(stored) = load_decision(c, &envelope.decision_identity)? else {
        return Ok(None);
    };
    validate_owner(c, envelope)?;
    if stored.state != DecisionState::Delivered {
        return Ok(None);
    }
    let terminal = build_validated_terminal_evidence(c, &stored, envelope, None)?;
    if terminal.disposition != FoundationTerminalDisposition::Accepted {
        return Ok(None);
    }
    let owner: String = c.query_row(
        "SELECT owner_identity FROM p05_s2_child_owners WHERE decision_identity=?1",
        [&envelope.decision_identity],
        |r| r.get(0),
    )?;
    Ok(Some(AcceptedReference {
        child_identity: child.into(),
        owner_identity: owner,
        decision_identity: envelope.decision_identity.clone(),
        disposition_identity: terminal.ref_id,
        attempt_identity: terminal
            .attempt_id
            .ok_or_else(|| invalid("actual Accepted attempt absent"))?,
        channel: terminal
            .accepted_channel
            .ok_or_else(|| invalid("actual Accepted channel absent"))?,
        raw_accepted: terminal.evidence_bytes,
        raw_sha256: terminal.evidence_sha256,
        audit_refs: audit_refs(c, &envelope.decision_identity)?,
    }))
}
fn validate_accepted_reference(
    c: &Connection,
    child: &str,
    envelope: &DeliveryEnvelope,
    original: &AcceptedReference,
) -> Result<()> {
    let actual = accepted_reference(c, child, envelope)?
        .ok_or_else(|| invalid("original physical Accepted authority absent"))?;
    let mut normalized = actual;
    normalized.audit_refs = original.audit_refs.clone();
    if normalized != *original {
        return Err(invalid(
            "original Accepted owner/attempt/raw authority changed",
        ));
    }
    validate_audit_refs(c, &envelope.decision_identity, &original.audit_refs)
}
fn require_drained(c: &Connection, draft: &str) -> Result<()> {
    let pending:i64=c.query_row("SELECT (SELECT COUNT(*) FROM immutable_audit_outbox a JOIN p05_unit_children p ON p.decision_identity=a.decision_identity WHERE p.draft_identity=?1 AND a.append_state!='Appended')+(SELECT COUNT(*) FROM delivery_disposition_payloads d JOIN p05_unit_children p ON p.decision_identity=d.decision_identity WHERE p.draft_identity=?1 AND d.append_state!='Appended')+(SELECT COUNT(*) FROM task_transition_payloads t JOIN p05_unit_children p ON p.decision_identity=t.decision_identity WHERE p.draft_identity=?1 AND (t.append_state!='Appended' OR t.hydration_state!='Applied'))",[draft],|r|r.get(0))?;
    if pending != 0 {
        return Err(invalid(
            "Unit has pending audit/payload/hydration; no completion",
        ));
    }
    Ok(())
}
fn load_completion(c: &Connection, id: &str) -> Result<CompletionCanonical> {
    let (draft,rev,bytes,hash,image):(String,i64,Vec<u8>,String,Vec<u8>)=c.query_row("SELECT draft_identity,mutation_revision,completion_canonical,completion_sha256,completion_preimage FROM p05_s2_completion_receipts WHERE completion_identity=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    verify_blob(id, &bytes, &hash, &image, "p05-unit-completion-v1")?;
    let data: CompletionCanonical = decode(&bytes)?;
    if data.schema != "p05-unit-completion-v1"
        || data.family != FAMILY
        || data.draft_identity != draft
        || data.mutation_revision != rev
        || rev < 4
        || !is_lower_sha256(&data.authority_sha256)
    {
        return Err(invalid("completion codec/row differs"));
    }
    parse_utc(&data.observed_utc)?;
    let original = load_draft_inner(c, &data.business_date, zero_namespace(), false)?
        .ok_or_else(|| invalid("completion draft absent"))?;
    let intent = load_intent_inner(c, &data.business_date, zero_namespace(), false)?
        .ok_or_else(|| invalid("completion intent absent"))?;
    if original.identity != draft
        || intent.identity != data.intent_identity
        || original.data.current_codes != data.current_codes
        || intent.children.len() != data.accepted.len()
    {
        return Err(invalid("completion original full intent differs"));
    }
    let fingerprint:String=c.query_row("SELECT after_sha256 FROM p05_s2_mutation_events WHERE draft_identity=?1 AND mutation_revision=?2",params![draft,rev],|r|r.get(0))?;
    if fingerprint != data.authority_sha256 {
        return Err(invalid("completion revision fingerprint differs"));
    }
    for ((child, raw), receipt) in intent.children.iter().zip(&data.accepted) {
        validate_accepted_reference(c, child, &parse_envelope(raw)?, receipt)?;
    }
    Ok(data)
}
pub(super) fn load_completed_origin(c: &Connection, id: &str) -> Result<OriginObservation> {
    let version: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 14 {
        return Err(invalid("pre14 cannot adopt reserved Completed baseline"));
    }
    let (family,date,kind,unit,receipt,refs,bytes,hash,image):(String,String,String,String,String,Vec<u8>,Vec<u8>,String,Vec<u8>)=c.query_row("SELECT family,business_date,origin_kind,completed_unit_identity,completed_receipt_identity,accepted_physical_refs,origin_canonical,origin_sha256,origin_preimage FROM p05_baseline_origins WHERE origin_identity=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?)))?;
    verify_blob(id, &bytes, &hash, &image, "p05-completed-baseline-v2")?;
    let data: CompletedOrigin = decode(&bytes)?;
    let completion = load_completion(c, &receipt)?;
    let first: String = c.query_row(
        "SELECT first_completion_identity FROM p05_s2_completion_heads WHERE draft_identity=?1",
        [&unit],
        |r| r.get(0),
    )?;
    if family != FAMILY
        || data.schema != "p05-completed-baseline-v2"
        || data.family != family
        || data.business_date != date
        || kind != "CompletedUnitV2Baseline"
        || data.kind != kind
        || data.completed_unit_identity != unit
        || data.completed_receipt_identity != receipt
        || first != receipt
        || encode(&data.accepted_physical_refs)? != refs
        || data.accepted_physical_refs != completion.accepted
        || completion.draft_identity != unit
        || completion.business_date != date
        || data.current_codes != completion.current_codes
    {
        return Err(invalid(
            "Completed baseline differs from actual first physical completion",
        ));
    }
    Ok(OriginObservation {
        business_date: date,
        kind,
        codes: data.current_codes,
        completed_unit: Some(unit),
        completed_receipt: Some(receipt),
    })
}
pub(super) fn require_current_baseline(c: &Connection, id: &str) -> Result<()> {
    let origin = load_completed_origin(c, id)?;
    let unit = origin.completed_unit.unwrap();
    let current: Option<String> = c.query_row(
        "SELECT current_completion_identity FROM p05_s2_completion_heads WHERE draft_identity=?1",
        [&unit],
        |r| r.get(0),
    )?;
    let current =
        current.ok_or_else(|| invalid("prior Completed Unit is dirty after late evidence"))?;
    require_current_completion(c, &current)
}
fn require_current_completion(c: &Connection, id: &str) -> Result<()> {
    let data = load_completion(c, id)?;
    let revision: i64 = c.query_row(
        "SELECT mutation_revision FROM p05_unit_heads WHERE draft_identity=?1",
        [&data.draft_identity],
        |r| r.get(0),
    )?;
    if revision != data.mutation_revision
        || authority_sha(c, &data.draft_identity, false)? != data.authority_sha256
    {
        return Err(invalid("current completion revision/actual bytes differ"));
    }
    require_drained(c, &data.draft_identity)
}
pub(super) fn validate_baseline_rows(c: &Connection) -> Result<()> {
    super::super::super::schema_p05_unit_runtime::verify_catalog(c)?;
    let ids=c.prepare("SELECT origin_identity FROM p05_baseline_origins ORDER BY business_date,origin_kind DESC")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut prospective = None;
    let mut completed = Vec::new();
    for id in ids {
        let origin = load_origin(c, &id)?;
        if origin.completed_unit.is_some() {
            completed.push((origin.business_date, id));
        } else if prospective.replace((origin.business_date, id)).is_some() {
            return Err(invalid("multiple prospective family origins"));
        }
    }
    let head: Option<(String, i64, String)> = c
        .query_row(
            "SELECT family,baseline_revision,origin_identity FROM p05_baseline_heads",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if prospective.is_none() {
        if head.is_some() || !completed.is_empty() {
            return Err(invalid("baseline has no real prospective origin"));
        }
        return Ok(());
    }
    completed.sort();
    let (date, first) = prospective.unwrap();
    if completed.windows(2).any(|w| w[0].0 >= w[1].0)
        || completed.first().is_some_and(|v| v.0 < date)
    {
        return Err(invalid("baseline date progression differs"));
    }
    let (family, rev, current) = head.ok_or_else(|| invalid("baseline head absent"))?;
    let count: i64 = c.query_row("SELECT COUNT(*) FROM p05_baseline_heads", [], |r| r.get(0))?;
    let expected = completed.last().map(|(_, id)| id).unwrap_or(&first);
    if count != 1 || family != FAMILY || rev != completed.len() as i64 || current != *expected {
        return Err(invalid("baseline CAS history/revision differs"));
    }
    // Every Completed origin must have used its immediately previous origin.
    let mut previous = first;
    for (index, (date, id)) in completed.iter().enumerate() {
        let draft = load_draft(c, date, zero_namespace())?
            .ok_or_else(|| invalid("completed origin draft absent"))?;
        if draft.data.baseline_origin_identity != previous
            || draft.data.baseline_revision != index as i64
        {
            return Err(invalid("baseline history skipped a prior origin"));
        }
        previous = id.clone();
    }
    Ok(())
}
pub(super) fn validate_runtime_rows(c: &Connection) -> Result<()> {
    super::super::super::schema_p05_unit_runtime::verify_catalog(c)?;
    let owner_decisions = c
        .prepare("SELECT decision_identity FROM p05_s2_child_owners ORDER BY decision_identity")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in owner_decisions {
        let stored = load_decision(c, &id)?.ok_or_else(|| invalid("counted owner orphan"))?;
        validate_owner(c, &parse_envelope(&stored.envelope_canonical)?)?;
    }
    let unowned:i64=c.query_row("SELECT COUNT(*) FROM delivery_decisions d JOIN p05_unit_drafts u ON u.business_date=d.business_date LEFT JOIN p05_s2_child_owners o ON o.decision_identity=d.decision_identity WHERE d.push_kind IN ('AuctionRepush','CandidateBoard','CandidateInvalidated') AND o.owner_identity IS NULL",[],|r|r.get(0))?;
    if unowned != 0 {
        return Err(invalid(
            "same-day ordinary affected decision bypassed Unit owner",
        ));
    }
    let orphans:i64=c.query_row("SELECT (SELECT COUNT(*) FROM p05_s2_mutation_events e JOIN p05_unit_heads h ON h.draft_identity=e.draft_identity WHERE h.phase!='IntentComplete')+(SELECT COUNT(*) FROM p05_s2_completion_receipts r LEFT JOIN p05_s2_completion_heads h ON h.draft_identity=r.draft_identity WHERE h.draft_identity IS NULL)",[],|r|r.get(0))?;
    if orphans != 0 {
        return Err(invalid("runtime effects exist without complete Unit/head"));
    }
    let heads=c.prepare("SELECT draft_identity,mutation_revision FROM p05_unit_heads WHERE phase='IntentComplete' ORDER BY draft_identity")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (draft, revision) in heads {
        let mut fingerprint = authority_sha(c, &draft, true)?;
        let mut next = 4;
        let events=c.prepare("SELECT event_identity,mutation_revision,decision_identity,before_sha256,after_sha256,event_canonical,event_sha256,event_preimage FROM p05_s2_mutation_events WHERE draft_identity=?1 ORDER BY mutation_revision")?.query_map([&draft],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Vec<u8>>(5)?,r.get::<_,String>(6)?,r.get::<_,Vec<u8>>(7)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, rev, decision, before, after, bytes, hash, image) in events {
            verify_blob(&id, &bytes, &hash, &image, "p05-counted-mutation-v1")?;
            let data: MutationCanonical = decode(&bytes)?;
            let child:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM p05_unit_children WHERE draft_identity=?1 AND decision_identity=?2)",params![draft,decision],|r|r.get(0))?;
            if data.schema != "p05-counted-mutation-v1"
                || data.draft_identity != draft
                || data.mutation_revision != rev
                || rev != next
                || data.decision_identity != decision
                || !child
                || data.before_sha256 != before
                || data.after_sha256 != after
                || before != fingerprint
                || before == after
                || !is_lower_sha256(&after)
            {
                return Err(invalid("Unit actual mutation chain differs"));
            }
            fingerprint = after;
            next += 1;
        }
        if revision != next - 1 || authority_sha(c, &draft, false)? != fingerprint {
            return Err(invalid(
                "Unit current revision is not exact actual authority bytes",
            ));
        }
    }
    let receipts=c.prepare("SELECT completion_identity FROM p05_s2_completion_receipts ORDER BY completion_identity")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for id in receipts {
        load_completion(c, &id)?;
    }
    let pointers=c.prepare("SELECT draft_identity,first_completion_identity,current_completion_identity FROM p05_s2_completion_heads ORDER BY draft_identity")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (draft, first, current) in pointers {
        let actual_first:String=c.query_row("SELECT completion_identity FROM p05_s2_completion_receipts WHERE draft_identity=?1 ORDER BY mutation_revision LIMIT 1",[&draft],|r|r.get(0))?;
        if first != actual_first {
            return Err(invalid("immutable first completion changed"));
        }
        if let Some(current) = current {
            require_current_completion(c, &current)?;
        }
        let origins:i64=c.query_row("SELECT COUNT(*) FROM p05_baseline_origins WHERE completed_unit_identity=?1 AND completed_receipt_identity=?2",params![draft,first],|r|r.get(0))?;
        if origins != 1 {
            return Err(invalid("first completion and baseline origin not atomic"));
        }
    }
    Ok(())
}

impl DurableDeliveryCoordinator {
    pub(crate) fn observe_p05_unit_receipts(
        &self,
        date: &str,
    ) -> Result<P05UnitReceiptObservation> {
        date_day(date)?;
        let namespace = self.p05_namespace()?;
        let (observation, binding) = self.with_connection(|c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let mut observation = observe_receipts(&tx, date)?;
            observation.namespace = Some(namespace);
            let binding = capture_sql_binding(&tx, date)?;
            validate_rows(&tx)?;
            tx.commit()?;
            binding.validate(c)?;
            Ok((observation, binding))
        })?;
        // The first core's post-SQL hook has now run. Check one fresh actual
        // snapshot without replaying hooks or acquiring an operational DB lock.
        self.with_mutation_routing_connection(|c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Deferred)?;
            binding.validate(&tx)?;
            validate_rows(&tx)?;
            tx.commit()?;
            Ok(())
        })?;
        Ok(observation)
    }
    pub(crate) fn finalize_p05_unit_observed(
        &self,
        date: &str,
        observed: &P05UnitReceiptObservation,
    ) -> Result<P05UnitReceiptObservation> {
        if observed.namespace != Some(self.p05_namespace()?) {
            return Err(invalid(
                "finalizer observation belongs to another durable namespace",
            ));
        }
        let draft = self
            .read_p05_unit_draft(date)?
            .ok_or_else(|| invalid("finalizer draft absent"))?;
        if observed.draft_identity != draft.identity {
            return Err(invalid("finalizer observation belongs to another Unit"));
        }
        self.finalize_p05_unit(date, observed.mutation_revision)
    }
    /// Only actual terminal authority can create completion; no caller completion boolean.
    pub(crate) fn finalize_p05_unit(
        &self,
        date: &str,
        expected_revision: i64,
    ) -> Result<P05UnitReceiptObservation> {
        date_day(date)?;
        let namespace = self.p05_namespace()?;
        let mut observation=self.with_p05_immediate_transaction_declared(date,|tx,dependencies| {
            let draft=load_draft(tx,date,zero_namespace())?.ok_or_else(||invalid("finalizer draft absent"))?;
            let intent=load_intent(tx,date,zero_namespace())?.ok_or_else(||invalid("finalizer complete intent absent"))?;
            let current:i64=tx.query_row("SELECT mutation_revision FROM p05_unit_heads WHERE draft_identity=?1",[&draft.identity],|r|r.get(0))?;
            if current!=expected_revision {return Err(invalid("finalizer Unit revision changed"));}
            let head:Option<(String,Option<String>)>=tx.query_row("SELECT first_completion_identity,current_completion_identity FROM p05_s2_completion_heads WHERE draft_identity=?1",[&draft.identity],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if head.as_ref().is_some_and(|(_,current)|current.is_some()) {return observe_receipts(tx,date);}
            if head.is_none() && draft.data.baseline_kind=="CompletedUnitV2Baseline" {
                dependencies.require_current_baseline(tx,&draft.data.baseline_origin_identity)?;
            }
            require_drained(tx,&draft.identity)?;
            let mut accepted=Vec::new();
            for (child,raw) in &intent.children {accepted.push(accepted_reference(tx,child,&parse_envelope(raw)?)?.ok_or_else(||invalid("all required children need real physical Accepted terminal"))?);}
            let completion=CompletionCanonical {schema:"p05-unit-completion-v1".into(),family:FAMILY.into(),business_date:date.into(),draft_identity:draft.identity.clone(),intent_identity:intent.identity.clone(),mutation_revision:current,authority_sha256:authority_sha(tx,&draft.identity,false)?,current_codes:draft.data.current_codes.clone(),accepted,observed_utc:utc_text(Utc::now())};
            let bytes=encode(&completion)?;let image=preimage("p05-unit-completion-v1",&bytes);let id=sha256_hex(&image);
            tx.execute("INSERT INTO p05_s2_completion_receipts(completion_identity,draft_identity,mutation_revision,completion_canonical,completion_sha256,completion_preimage) VALUES(?1,?2,?3,?4,?5,?6)",params![id,draft.identity,current,bytes,sha256_hex(&bytes),image])?;
            if head.is_some() {
                require_single_cas_update(tx.execute("UPDATE p05_s2_completion_heads SET current_completion_identity=?1 WHERE draft_identity=?2 AND current_completion_identity IS NULL",params![id,draft.identity])?,"reclose same physical Unit")?;
            } else {
                tx.execute("INSERT INTO p05_s2_completion_heads(draft_identity,first_completion_identity,current_completion_identity) VALUES(?1,?2,?2)",params![draft.identity,id])?;
                let origin=CompletedOrigin {schema:"p05-completed-baseline-v2".into(),family:FAMILY.into(),business_date:date.into(),kind:"CompletedUnitV2Baseline".into(),completed_unit_identity:draft.identity.clone(),completed_receipt_identity:id.clone(),current_codes:draft.data.current_codes.clone(),accepted_physical_refs:completion.accepted};
                let bytes=encode(&origin)?;let image=preimage("p05-completed-baseline-v2",&bytes);let origin_id=sha256_hex(&image);let refs=encode(&origin.accepted_physical_refs)?;
                tx.execute("INSERT INTO p05_baseline_origins(origin_identity,family,business_date,origin_kind,completed_unit_identity,completed_receipt_identity,accepted_physical_refs,origin_canonical,origin_sha256,origin_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![origin_id,FAMILY,date,origin.kind,draft.identity,id,refs,bytes,sha256_hex(&bytes),image])?;
                require_single_cas_update(tx.execute("UPDATE p05_baseline_heads SET baseline_revision=baseline_revision+1,origin_identity=?1 WHERE family=?2 AND baseline_revision=?3 AND origin_identity=?4",params![origin_id,FAMILY,draft.data.baseline_revision,draft.data.baseline_origin_identity])?,"first Completed baseline CAS")?;
            }
            validate_rows(tx)?;observe_receipts(tx,date)
        })?;
        observation.namespace = Some(namespace);
        Ok(observation)
    }
}
fn observe_receipts(c: &Connection, date: &str) -> Result<P05UnitReceiptObservation> {
    let draft =
        load_draft(c, date, zero_namespace())?.ok_or_else(|| invalid("Unit observation absent"))?;
    let revision: i64 = c.query_row(
        "SELECT mutation_revision FROM p05_unit_heads WHERE draft_identity=?1",
        [&draft.identity],
        |r| r.get(0),
    )?;
    let intent = load_intent(c, date, zero_namespace())?;
    let mut children = Vec::new();
    if let Some(intent) = intent {
        for (child, raw) in intent.children {
            let envelope = parse_envelope(&raw)?;
            let observation = match load_decision(c, &envelope.decision_identity)? {
                None => P05ChildReceiptObservation::NotPrepared {
                    child_identity: child,
                },
                Some(stored) => {
                    validate_owner(c, &envelope)?;
                    if stored.state == DecisionState::Delivered
                        || matches!(
                            stored.state,
                            DecisionState::RejectedDurable
                                | DecisionState::UncertainManualReview
                                | DecisionState::ManualResolvedRejected
                        )
                    {
                        let terminal =
                            build_validated_terminal_evidence(c, &stored, &envelope, None)?;
                        if terminal.disposition == FoundationTerminalDisposition::Accepted {
                            P05ChildReceiptObservation::PhysicallyAccepted {
                                child_identity: child,
                                decision_identity: envelope.decision_identity,
                                disposition_identity: terminal.ref_id,
                                attempt_identity: terminal
                                    .attempt_id
                                    .ok_or_else(|| invalid("Accepted attempt absent"))?,
                                raw_sha256: terminal.evidence_sha256,
                            }
                        } else {
                            P05ChildReceiptObservation::NonAccepted {
                                child_identity: child,
                                decision_identity: envelope.decision_identity,
                                disposition: nonaccepted(terminal.disposition)?,
                            }
                        }
                    } else {
                        P05ChildReceiptObservation::Pending {
                            child_identity: child,
                            decision_identity: envelope.decision_identity,
                            state: stored.state,
                        }
                    }
                }
            };
            children.push(observation);
        }
    }
    let completion:Option<Option<String>>=c.query_row("SELECT current_completion_identity FROM p05_s2_completion_heads WHERE draft_identity=?1",[&draft.identity],|r|r.get(0)).optional()?;
    Ok(P05UnitReceiptObservation {
        namespace: None,
        draft_identity: draft.identity,
        mutation_revision: revision,
        completion_identity: completion.flatten(),
        children,
    })
}

pub(super) fn scope_key_for_new_code(code: &str, is_test: bool) -> Result<String> {
    use crate::data_gateway::instrument_identity::resolve_production_equity;
    use crate::market_domain::Exchange;
    let identity = if is_test {
        #[cfg(test)]
        {
            crate::data_gateway::instrument_identity::resolve_test_equity(code, None)
        }
        #[cfg(not(test))]
        {
            resolve_production_equity(code, None)
        }
    } else {
        resolve_production_equity(code, None)
    }
    .map_err(|_| invalid("T08 production equity identity unavailable"))?;
    identity
        .require_a_share()
        .map_err(|_| invalid("T08 requires A-share identity"))?;
    let exchange = match identity.instrument().exchange() {
        Exchange::Shanghai => "SHANGHAI",
        Exchange::Shenzhen => "SHENZHEN",
        Exchange::Beijing => "BEIJING",
    };
    Ok(format!(
        "{exchange}:EQUITY:{}",
        identity.instrument().code()
    ))
}
pub(super) fn scope_key_for_stored_code(code: &str) -> Result<String> {
    // Closed stored observations validate bytes only. This does not mint a
    // Test/Production identity or a source qualification capability.
    scope_key_for_new_code(code, code.starts_with("TEST_CODE_"))
}

#[derive(Clone)]
enum PredictionHandle {
    Global(&'static DatabaseManager),
    #[cfg(test)]
    Isolated(std::sync::Arc<DatabaseManager>),
}
impl PredictionHandle {
    fn db(&self) -> &DatabaseManager {
        match self {
            Self::Global(db) => db,
            #[cfg(test)]
            Self::Isolated(db) => db,
        }
    }
}
impl DurableDeliveryCoordinator {
    /// Fresh Started is persisted before the worker, then both connection guards
    /// are released before spawn/await. Reopened Started never starts sampling.
    pub(crate) async fn finish_p05_unit_preparation_global(
        &self,
        draft: &StoredP05Draft,
    ) -> Result<StoredP05Intent> {
        let db = DatabaseManager::try_get()
            .ok_or_else(|| invalid("actual operational singleton unavailable"))?;
        self.finish_p05_unit_preparation(PredictionHandle::Global(db), draft)
            .await
    }
    #[cfg(test)]
    pub(crate) async fn finish_p05_unit_preparation_on(
        &self,
        prediction_db: std::sync::Arc<DatabaseManager>,
        draft: &StoredP05Draft,
    ) -> Result<StoredP05Intent> {
        self.require_p05_test_clock()?;
        self.finish_p05_unit_preparation(PredictionHandle::Isolated(prediction_db), draft)
            .await
    }
    async fn finish_p05_unit_preparation(
        &self,
        prediction_db: PredictionHandle,
        draft: &StoredP05Draft,
    ) -> Result<StoredP05Intent> {
        let namespace = self.p05_namespace()?;
        if draft.namespace != namespace {
            return Err(invalid("worker draft belongs to another durable namespace"));
        }
        self.with_mutation_routing_connection(|c| require_draft(c, draft, namespace))?;
        if let Some(existing) = self.read_p05_unit_intent(draft.business_date())? {
            verify_actual_prediction(prediction_db.db(), draft, &existing)?;
            return Ok(existing);
        }
        let start = self.claim_p05_prediction_prepare(draft)?;
        if start.may_start_sampling() {
            let request = crate::monitor::prediction::CandidateBoardPreparationRequest::new(
                draft.business_date(),
                &parse_shanghai(&draft.data.input.captured_shanghai)?
                    .format("%H:%M")
                    .to_string(),
                draft.board_rendered_bytes().to_vec(),
                draft.strong_samples(),
            )
            .map_err(|_| invalid("prediction request construction failed after Started"))?;
            let db = prediction_db.clone();
            let captured = parse_shanghai(&draft.data.input.captured_shanghai)?;
            tokio::task::spawn_blocking(move || {
                if matches!(&db, PredictionHandle::Global(_)) {
                    fresh_window(captured, Utc::now())?;
                }
                crate::monitor::prediction::prepare_candidate_board_on(db.db(), &request).map_err(
                    |_| {
                        invalid(
                            "prediction worker incomplete after Started; sampling cannot restart",
                        )
                    },
                )
            })
            .await
            .map_err(|_| {
                invalid("prediction worker unknown after Started; sampling cannot restart")
            })??;
        }
        self.complete_p05_unit_intent_on(draft, &start, prediction_db.db())
    }
    pub(crate) fn validate_p05_unit_prediction_on(
        &self,
        prediction_db: &DatabaseManager,
        date: &str,
    ) -> Result<()> {
        let draft = self
            .read_p05_unit_draft(date)?
            .ok_or_else(|| invalid("actual prediction check draft absent"))?;
        let intent = self
            .read_p05_unit_intent(date)?
            .ok_or_else(|| invalid("actual prediction check full intent absent"))?;
        verify_actual_prediction(prediction_db, &draft, &intent)
    }
    pub(crate) fn dispatch_p05_unit_child_on(
        &self,
        prediction_db: &DatabaseManager,
        date: &str,
        index: usize,
        sinks: &[AuthoritativeSink],
        append: &dyn ImmutableAppendPort,
    ) -> Result<ResumeOutcome> {
        self.dispatch_p05_unit_child_local(prediction_db, date, index, sinks, append, None)
    }
    fn dispatch_p05_unit_child_local(
        &self,
        prediction_db: &DatabaseManager,
        date: &str,
        index: usize,
        sinks: &[AuthoritativeSink],
        append: &dyn ImmutableAppendPort,
        test_now: Option<DateTime<Utc>>,
    ) -> Result<ResumeOutcome> {
        if test_now.is_some() {
            self.require_p05_test_clock()?;
        }
        let (_prepared, envelope) =
            self.prepare_p05_unit_child_local(prediction_db, date, index, sinks.len(), test_now)?;
        self.reconcile_pending(
            ReconcileScope::Decision(&envelope.decision_identity),
            append,
            test_now.unwrap_or_else(Utc::now),
        )?;
        // The independent read is released before either begin/retry transaction.
        let route = self.decision_mutation_route(&envelope.decision_identity)?;
        let actual = self.p05_consumer_check_on(prediction_db, &route);
        let outcome = self.resume_deliverable_with_p05_check(
            &envelope.decision_identity,
            sinks,
            test_now.unwrap_or_else(Utc::now),
            Some(&actual),
        )?;
        self.reconcile_pending(
            ReconcileScope::Decision(&envelope.decision_identity),
            append,
            test_now.unwrap_or_else(Utc::now),
        )?;
        Ok(outcome)
    }
}

/// Opaque readonly original child plus a released operational observation.
/// This does not prepare, reserve budget, qualify source or call a sink.
#[derive(Clone)]
pub(crate) struct P05ChildInspection {
    namespace: FileObjectIdentity,
    date: String,
    draft: String,
    intent: String,
    child: String,
    index: usize,
    envelope: DeliveryEnvelope,
    actual: ActualP05ConsumerCheck,
}
impl P05ChildInspection {
    pub(crate) fn business_date(&self) -> &str {
        &self.date
    }
    pub(crate) fn unit_identity(&self) -> &str {
        &self.draft
    }
    pub(crate) fn intent_identity(&self) -> &str {
        &self.intent
    }
    pub(crate) fn child_identity(&self) -> &str {
        &self.child
    }
    pub(crate) fn ordinal(&self) -> usize {
        self.index
    }
    pub(crate) fn envelope(&self) -> &DeliveryEnvelope {
        &self.envelope
    }
    pub(crate) fn governance_code(&self) -> Option<&str> {
        (self.envelope.push_kind == PushKind::CandidateInvalidated)
            .then(|| self.envelope.scope_key.rsplit(':').next())
            .flatten()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum P05StoredUnitPhase {
    Draft,
    Started,
    IntentComplete,
}
/// An immutable read snapshot, not a lease, completion or permission to sample.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct P05StoredUnitSnapshot {
    namespace: FileObjectIdentity,
    date: String,
    draft: String,
    revision: i64,
    phase: P05StoredUnitPhase,
    intent: Option<String>,
    first_completion: Option<String>,
}
impl P05StoredUnitSnapshot {
    pub fn business_date(&self) -> &str {
        &self.date
    }
    pub fn unit_identity(&self) -> &str {
        &self.draft
    }
    pub fn mutation_revision(&self) -> i64 {
        self.revision
    }
    pub fn phase(&self) -> P05StoredUnitPhase {
        self.phase
    }
    pub fn intent_identity(&self) -> Option<&str> {
        self.intent.as_deref()
    }
    pub fn first_completion_identity(&self) -> Option<&str> {
        self.first_completion.as_deref()
    }
}
#[derive(Eq, PartialEq)]
struct UnitListBinding {
    // Include clean Units too: a lawful late mutation may make one dirty.
    units: Vec<SqlBinding>,
    baseline: Option<(i64, String)>,
}
fn capture_unit_list(
    c: &Connection,
    namespace: FileObjectIdentity,
) -> Result<(Vec<P05StoredUnitSnapshot>, UnitListBinding)> {
    validate_rows(c)?;
    let dates = c
        .prepare("SELECT business_date FROM p05_unit_drafts ORDER BY business_date")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut units = Vec::new();
    let mut unfinished = Vec::new();
    for date in dates {
        date_day(&date)?;
        let binding = capture_sql_binding(c, &date)?;
        let (revision, phase, intent) = binding
            .head
            .as_ref()
            .ok_or_else(|| invalid("listed Unit head absent"))?;
        let phase = match phase.as_str() {
            "Draft" => P05StoredUnitPhase::Draft,
            "Started" => P05StoredUnitPhase::Started,
            "IntentComplete" => P05StoredUnitPhase::IntentComplete,
            _ => return Err(invalid("listed Unit phase unknown")),
        };
        if binding
            .completion_head
            .as_ref()
            .is_none_or(|(_, current)| current.is_none())
        {
            unfinished.push(P05StoredUnitSnapshot {
                namespace,
                date: date.clone(),
                draft: binding
                    .draft_identity
                    .clone()
                    .ok_or_else(|| invalid("listed Unit absent"))?,
                revision: *revision,
                phase,
                intent: intent.clone(),
                first_completion: binding
                    .completion_head
                    .as_ref()
                    .map(|(first, _)| first.clone()),
            });
        }
        units.push(binding);
    }
    let baseline = c
        .query_row(
            "SELECT baseline_revision,origin_identity FROM p05_baseline_heads WHERE family=?1",
            [FAMILY],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok((unfinished, UnitListBinding { units, baseline }))
}
impl UnitListBinding {
    fn validate(&self, c: &Connection, namespace: FileObjectIdentity) -> Result<()> {
        if capture_unit_list(c, namespace)?.1 != *self {
            return Err(invalid(
                "readonly Unit list membership/actual SQL snapshot changed",
            ));
        }
        Ok(())
    }
}
impl DurableDeliveryCoordinator {
    pub(crate) fn require_p05_production_singleton(&self) -> Result<&'static DatabaseManager> {
        if !matches!(
            &self.config.environment,
            super::super::super::model::StoreEnvironment::Production
        ) {
            return Err(invalid(
                "P05 public worker requires Production singleton namespace",
            ));
        }
        DatabaseManager::try_get()
            .ok_or_else(|| invalid("actual operational singleton unavailable"))
    }
    fn validate_p05_child_inspection(&self, view: &P05ChildInspection) -> Result<()> {
        if self.p05_namespace()? != view.namespace {
            return Err(invalid("child view belongs to another durable namespace"));
        }
        self.with_mutation_routing_connection(|c| {
            if let ActualP05ConsumerCheck::Verified(cap) = &view.actual {
                cap.validate(c)?;
            }
            let intent = load_intent(c, &view.date, view.namespace)?
                .ok_or_else(|| invalid("child view intent absent"))?;
            if intent.identity != view.intent
                || intent.draft_identity != view.draft
                || intent.children.get(view.index)
                    != Some(&(view.child.clone(), view.envelope.canonical_bytes()?))
            {
                return Err(invalid("child view differs from exact original member"));
            }
            Ok(())
        })
    }
    pub(crate) fn inspect_p05_unit_child_global(
        &self,
        date: &str,
        index: usize,
    ) -> Result<P05ChildInspection> {
        date_day(date)?;
        let namespace = self.p05_namespace()?;
        let view = self.with_connection(|c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let intent = load_intent(&tx, date, namespace)?
                .ok_or_else(|| invalid("readonly child full intent absent"))?;
            let (child, raw) = intent
                .children
                .get(index)
                .ok_or_else(|| invalid("readonly child ordinal absent"))?;
            let envelope = parse_envelope(raw)?;
            if load_decision(&tx, &envelope.decision_identity)?.is_some() {
                validate_owner(&tx, &envelope)?;
            }
            let binding = capture_sql_binding(&tx, date)?;
            validate_rows(&tx)?;
            let view = P05ChildInspection {
                namespace,
                date: date.into(),
                draft: intent.draft_identity.clone(),
                intent: intent.identity.clone(),
                child: child.clone(),
                index,
                envelope,
                actual: ActualP05ConsumerCheck::Unowned,
            };
            tx.commit()?;
            binding.validate(c)?;
            Ok((view, binding))
        })?;
        self.with_mutation_routing_connection(|c| view.1.validate(c))?;
        let mut view = view.0;
        let route = self.prepare_mutation_route(&view.envelope)?;
        view.actual = self.p05_consumer_check_global(&route);
        Ok(view)
    }
    pub(crate) fn inspect_p05_owned_child_global(
        &self,
        decision: &str,
    ) -> Result<Option<P05ChildInspection>> {
        let route = self.decision_mutation_route(decision)?;
        let member=self.with_mutation_routing_connection(|c| {
            let row=c.query_row("SELECT d.business_date,p.ordinal FROM p05_s2_child_owners o JOIN p05_unit_children p ON p.child_identity=o.child_identity JOIN p05_unit_drafts d ON d.draft_identity=o.draft_identity WHERE o.decision_identity=?1",[decision],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).optional()?;
            if row.is_none() && mutation_date(c,&route)?.is_some() {return Err(invalid("owned P05 decision lacks its original contextual owner"));}
            Ok(row)
        })?;
        match member {
            None => Ok(None),
            Some((date, index)) => Ok(Some(self.inspect_p05_unit_child_global(
                &date,
                usize::try_from(index).map_err(|_| invalid("owned child ordinal invalid"))?,
            )?)),
        }
    }
    pub(crate) fn prepare_p05_child_inspection(
        &self,
        view: &P05ChildInspection,
        sink_count: usize,
    ) -> Result<PrepareOutcome> {
        self.validate_p05_child_inspection(view)?;
        let route = self.prepare_mutation_route(&view.envelope)?;
        // Refresh a real released reader immediately before a fresh opening;
        // failed reads remain deferred so exact Existing can still recover.
        let check = self.p05_consumer_check_global(&route);
        self.prepare_p05_unit_child_checked(&view.date, view.index, sink_count, Utc::now(), &check)
            .map(|(outcome, _)| outcome)
    }
    pub(crate) fn resume_p05_child_inspection(
        &self,
        view: &P05ChildInspection,
        sinks: &[AuthoritativeSink],
    ) -> Result<ResumeOutcome> {
        self.validate_p05_child_inspection(view)?;
        let route = self.decision_mutation_route(&view.envelope.decision_identity)?;
        self.with_mutation_routing_connection(|c| validate_owner(c, &view.envelope))?;
        let check = self.p05_consumer_check_global(&route);
        self.resume_deliverable_with_p05_check(
            &view.envelope.decision_identity,
            sinks,
            Utc::now(),
            Some(&check),
        )
    }
    pub(crate) fn p05_has_first_completion(&self, date: &str) -> Result<bool> {
        self.with_connection(|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM p05_s2_completion_heads h JOIN p05_unit_drafts d ON d.draft_identity=h.draft_identity WHERE d.business_date=?1)",[date],|r|r.get(0))?))
    }
    pub(crate) fn dispatch_p05_child_inspection(
        &self,
        view: &P05ChildInspection,
        sinks: &[AuthoritativeSink],
        append: &dyn ImmutableAppendPort,
    ) -> Result<ResumeOutcome> {
        self.prepare_p05_child_inspection(view, sinks.len())?;
        self.reconcile_pending(
            ReconcileScope::Decision(&view.envelope.decision_identity),
            append,
            Utc::now(),
        )?;
        let outcome = self.resume_p05_child_inspection(view, sinks)?;
        self.reconcile_pending(
            ReconcileScope::Decision(&view.envelope.decision_identity),
            append,
            Utc::now(),
        )?;
        Ok(outcome)
    }
    pub(crate) fn inspect_p05_unfinished_units(&self) -> Result<Vec<P05StoredUnitSnapshot>> {
        let namespace = self.p05_namespace()?;
        let (snapshots, binding) = self.with_connection(|c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Deferred)?;
            let (snapshots, binding) = capture_unit_list(&tx, namespace)?;
            tx.commit()?;
            binding.validate(c, namespace)?;
            Ok((snapshots, binding))
        })?;
        self.with_mutation_routing_connection(|c| binding.validate(c, namespace))?;
        Ok(snapshots)
    }
}

#[cfg(test)]
#[path = "p05_unit_runtime_tests.rs"]
mod tests;

/// An operation-local exact SQL witness, never serialized or caller-created.
/// This is checked after every SQL hook and at the actual commit boundary.
#[derive(Eq, PartialEq)]
pub(in crate::durable_delivery::coordinator) struct SqlBinding {
    date: String,
    draft_identity: Option<String>,
    head: Option<(i64, String, Option<String>)>,
    authority: Option<String>,
    completion_head: Option<(String, Option<String>)>,
    baseline: Option<(i64, String)>,
}
// A dependency on an earlier current closure is declared only by a business
// opening or the first completion. It is deliberately nonrecursive and omits
// the family baseline head, which this operation may itself advance.
struct PriorClosureBinding {
    snapshot: SqlBinding,
}
#[derive(Default)]
pub(in crate::durable_delivery::coordinator) struct SqlDependencies {
    prior: std::cell::RefCell<Vec<PriorClosureBinding>>,
    consumer: Option<ActualP05ConsumerCheck>,
    consumer_used: std::cell::Cell<bool>,
}
impl SqlDependencies {
    pub(in crate::durable_delivery::coordinator) fn with_consumer(
        consumer: Option<ActualP05ConsumerCheck>,
    ) -> Self {
        Self {
            consumer,
            ..Self::default()
        }
    }
    pub(super) fn require_current_baseline(&self, c: &Connection, id: &str) -> Result<()> {
        require_current_baseline(c, id)?;
        let origin = load_completed_origin(c, id)?;
        let snapshot = capture_sql_binding(c, &origin.business_date)?;
        let mut prior = self.prior.borrow_mut();
        if !prior.iter().any(|p| p.snapshot.date == snapshot.date) {
            prior.push(PriorClosureBinding { snapshot });
        }
        Ok(())
    }
    pub(in crate::durable_delivery::coordinator) fn require_business_open(
        &self,
        c: &Connection,
        route: &DecisionMutationRoute,
    ) -> Result<()> {
        let Some(date) = mutation_date(c, route)? else {
            return Ok(());
        };
        self.consumer
            .as_ref()
            .ok_or_else(|| invalid("owned P05 business opening needs actual reader"))?
            .require(c, route)?;
        self.consumer_used.set(true);
        let draft = load_draft(c, &date, zero_namespace())?
            .ok_or_else(|| invalid("business-opening Unit absent"))?;
        if draft.data.baseline_kind == "CompletedUnitV2Baseline" {
            self.require_current_baseline(c, &draft.data.baseline_origin_identity)?;
        }
        Ok(())
    }
    pub(in crate::durable_delivery::coordinator) fn validate(&self, c: &Connection) -> Result<()> {
        if self.consumer_used.get() {
            match self.consumer.as_ref() {
                Some(ActualP05ConsumerCheck::Verified(cap)) => cap.validate(c)?,
                _ => return Err(invalid("consumed actual P05 reader witness absent")),
            }
        }
        for prior in self.prior.borrow().iter() {
            let actual = capture_sql_binding(c, &prior.snapshot.date)?;
            if actual.draft_identity != prior.snapshot.draft_identity
                || actual.head != prior.snapshot.head
                || actual.authority != prior.snapshot.authority
                || actual.completion_head != prior.snapshot.completion_head
            {
                return Err(invalid(
                    "operation-local prior current closure changed after body",
                ));
            }
        }
        Ok(())
    }
}
pub(in crate::durable_delivery::coordinator) fn capture_sql_binding(
    c: &Connection,
    date: &str,
) -> Result<SqlBinding> {
    let draft_identity: Option<String> = c
        .query_row(
            "SELECT draft_identity FROM p05_unit_drafts WHERE business_date=?1",
            [date],
            |r| r.get(0),
        )
        .optional()?;
    let head=draft_identity.as_ref().map(|draft|c.query_row("SELECT mutation_revision,phase,intent_identity FROM p05_unit_heads WHERE draft_identity=?1",[draft],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))).transpose()?;
    let authority = draft_identity
        .as_ref()
        .map(|draft| authority_sha(c, draft, false))
        .transpose()?;
    let completion_head = if let Some(draft) = &draft_identity {
        c.query_row("SELECT first_completion_identity,current_completion_identity FROM p05_s2_completion_heads WHERE draft_identity=?1",[draft],|r|Ok((r.get(0)?,r.get(1)?))).optional()?
    } else {
        None
    };
    let baseline = c
        .query_row(
            "SELECT baseline_revision,origin_identity FROM p05_baseline_heads WHERE family=?1",
            [FAMILY],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(SqlBinding {
        date: date.into(),
        draft_identity,
        head,
        authority,
        completion_head,
        baseline,
    })
}
impl SqlBinding {
    pub(in crate::durable_delivery::coordinator) fn validate(&self, c: &Connection) -> Result<()> {
        let actual = capture_sql_binding(c, &self.date)?;
        if actual.draft_identity != self.draft_identity
            || actual.head != self.head
            || actual.authority != self.authority
            || actual.completion_head != self.completion_head
            || actual.baseline != self.baseline
        {
            return Err(invalid(
                "operation-local exact Unit SQL binding changed after body",
            ));
        }
        Ok(())
    }
}
pub(in crate::durable_delivery::coordinator) fn mutation_date(
    c: &Connection,
    route: &DecisionMutationRoute,
) -> Result<Option<String>> {
    Ok(c.query_row("SELECT d.business_date FROM p05_unit_children p JOIN p05_unit_drafts d ON d.draft_identity=p.draft_identity WHERE p.decision_identity=?1",[&route.decision_identity],|r|r.get(0)).optional()?)
}
impl DurableDeliveryCoordinator {
    pub(super) fn with_p05_immediate_transaction<T>(
        &self,
        date: &str,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        self.with_p05_immediate_transaction_declared(date, |tx, _| operation(tx))
    }
    pub(super) fn with_p05_immediate_transaction_declared<T>(
        &self,
        date: &str,
        operation: impl FnOnce(&Transaction<'_>, &SqlDependencies) -> Result<T>,
    ) -> Result<T> {
        let binding = std::cell::RefCell::new(None::<SqlBinding>);
        let dependencies = SqlDependencies::default();
        let validate = |c: &Transaction<'_>| {
            binding
                .borrow()
                .as_ref()
                .ok_or_else(|| invalid("P05 SQL body witness absent"))?
                .validate(c)?;
            dependencies.validate(c)
        };
        self.with_immediate_transaction_validated_sql(
            SchemaVersionPolicy::Runtime,
            None,
            Some(&validate),
            |tx| {
                let result = operation(tx, &dependencies)?;
                binding.replace(Some(capture_sql_binding(tx, date)?));
                Ok(result)
            },
        )
    }
    // P05 owns its business dependency witness. Other families retain the
    // original pre-sink model/file validation path without a new dependency.
    pub(in crate::durable_delivery::coordinator) fn with_p05_business_pre_sink_transaction<T>(
        &self,
        route: &DecisionMutationRoute,
        operation: impl FnOnce(&Transaction<'_>, &SqlDependencies) -> Result<MutationEffect<T>>,
    ) -> Result<T> {
        self.with_p05_business_pre_sink_transaction_checked(route, None, operation)
    }
    pub(in crate::durable_delivery::coordinator) fn with_p05_business_pre_sink_transaction_checked<
        T,
    >(
        &self,
        route: &DecisionMutationRoute,
        check: Option<&ActualP05ConsumerCheck>,
        operation: impl FnOnce(&Transaction<'_>, &SqlDependencies) -> Result<MutationEffect<T>>,
    ) -> Result<T> {
        let p05 = self
            .with_mutation_routing_connection(|c| mutation_date(c, route))?
            .is_some();
        if p05 {
            self.with_p05_business_mutation_transaction(route, check, operation)
        } else {
            let dependencies = SqlDependencies::default();
            self.with_pre_sink_mutation_transaction(route, |tx| operation(tx, &dependencies))
        }
    }
}

#[cfg(test)]
pub(in crate::durable_delivery::coordinator) fn actual_extra_mutation_for_test(
    tx: &Transaction<'_>,
) -> Result<()> {
    let decision: String = tx.query_row(
        "SELECT decision_identity FROM p05_s2_child_owners ORDER BY rowid LIMIT 1",
        [],
        |r| r.get(0),
    )?;
    let stored =
        load_decision(tx, &decision)?.ok_or_else(|| invalid("TEST_CODE owned decision absent"))?;
    let route = DecisionMutationRoute::from_stored(tx, &stored)?;
    let before = before_mutation(tx, &route)?;
    enqueue_audit(
        tx,
        &decision,
        None,
        "LateReceiptObserved",
        b"TEST_CODE lawful extra immutable observation",
        Utc::now(),
    )?;
    after_mutation(tx, before, &route)?;
    // The extra change is otherwise valid: only the operation-local binding
    // should reject it after the original body has declared its one effect.
    validate_rows(tx)
}
