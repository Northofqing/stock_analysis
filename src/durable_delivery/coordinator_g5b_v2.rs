//! Contextual v2 counted admission and replay. No model call, seal or new sink.
use super::*;
use crate::monitor::g5b_analysis_v2::{
    classify_stored_source_v2, owner_bytes_from_model_rows, source_is_v2, G5bV2OwnerBytes,
};
use crate::monitor::g5b_selection_v2::G5bSelectionEvidence;

fn mismatch(detail: &str) -> DurableDeliveryError {
    DurableDeliveryError::PolicyMismatch(format!("G5b v2 owner: {detail}"))
}
fn codec_error(error: impl std::fmt::Display) -> DurableDeliveryError {
    mismatch(&error.to_string())
}

/// Only this owner can construct admission, after actual bundle verification.
/// The codec projection alone is never an admission capability.
pub(super) struct Admission<'a> {
    bundle: &'a VerifiedG5bModelBundle,
    bytes: G5bV2OwnerBytes,
}
fn bundle_owner(bundle: &VerifiedG5bModelBundle, index: usize) -> Result<Admission<'_>> {
    let load = |index: usize| {
        let saved = bundle.members().get(index).ok_or_else(|| {
            crate::monitor::g5b_analysis_v2::G5bAnalysisV2Error::Codec("member absent".to_owned())
        })?;
        match (saved.attempt(), saved.frozen()) {
            (Some(a), Some(f)) if a.is_committed() && f.is_committed() => Ok(Some((
                a.identity().to_owned(),
                a.bytes().to_vec(),
                f.identity().to_owned(),
                f.bytes().to_vec(),
            ))),
            _ => Ok(None),
        }
    };
    let archives = bundle
        .archives()
        .iter()
        .map(|a| {
            (
                a.identity().to_owned(),
                a.bytes().to_vec(),
                a.is_committed(),
            )
        })
        .collect::<Vec<_>>();
    let bytes =
        owner_bytes_from_model_rows(bundle.cohort().selection_bytes(), index, &load, &archives)
            .map_err(codec_error)?;
    Ok(Admission { bundle, bytes })
}

pub(super) fn validate_new_prepare_tx(
    tx: &Transaction<'_>,
    envelope: &DeliveryEnvelope,
    admission: Option<&Admission<'_>>,
) -> Result<()> {
    if envelope.push_kind != PushKind::G5bAttribution {
        return Ok(());
    }
    let cohort: Option<String> = tx
        .query_row(
            "SELECT cohort_identity FROM g5b_cohorts WHERE business_date=?1",
            [&envelope.business_date],
            |r| r.get(0),
        )
        .optional()?;
    match admission {
        Some(admission)
            if source_is_v2(&envelope.source_binding_canonical)
                && admission.bundle.cohort().identity() == admission.bytes.cohort
                && admission.bytes.envelope == *envelope
                && cohort.as_deref() == Some(admission.bytes.cohort.as_str()) =>
        {
            Ok(())
        }
        Some(_) => Err(mismatch(
            "contextual admission differs from actual frozen cohort",
        )),
        None if source_is_v2(&envelope.source_binding_canonical) || cohort.is_some() => Err(
            mismatch("fresh G5b requires private actual-cohort admission"),
        ),
        None => Ok(()),
    }
}

/// Pure SQL/codec projection. This does not return a trusted bundle or a send
/// capability. Actual files must be checked by the local owner before delivery.
fn stored_model_owner(
    connection: &Connection,
    date: &str,
    cohort: &str,
    index: usize,
) -> Result<G5bV2OwnerBytes> {
    let selection: Vec<u8> = connection.query_row(
        "SELECT selection_canonical FROM g5b_cohorts WHERE business_date=?1 AND cohort_identity=?2",
        params![date, cohort],
        |r| r.get(0),
    )?;
    let evidence = G5bSelectionEvidence::decode(&selection).map_err(codec_error)?;
    let load = |index: usize| {
        let result = (|| -> Result<Option<(String, Vec<u8>, String, Vec<u8>)>> {
            let occurrence = &evidence
                .occurrences()
                .get(index)
                .ok_or_else(|| mismatch("stored member absent"))?
                .0;
            let read = |role: &str| -> Result<Option<(String, Vec<u8>)>> {
                let mut query=connection.prepare("SELECT p.logical_intent,p.desired_bytes FROM g5b_artifact_events p WHERE p.cohort_identity=?1 AND p.occurrence_identity=?2 AND p.artifact_role=?3 AND p.phase='Prepared' AND EXISTS(SELECT 1 FROM g5b_artifact_events c WHERE c.logical_intent=p.logical_intent AND c.phase='Committed')")?;
                let rows = query
                    .query_map(params![cohort, occurrence, role], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<(String, Vec<u8>)>>>()?;
                if rows.len() > 1 {
                    return Err(mismatch("ambiguous original Committed model artifact"));
                }
                Ok(rows.into_iter().next())
            };
            match (read("Attempt")?, read("Frozen")?) {
                (Some((a, ab)), Some((f, fb))) => Ok(Some((a, ab, f, fb))),
                (None, Some(_)) => Err(mismatch("Committed Frozen missing original Attempt")),
                _ => Ok(None),
            }
        })();
        result.map_err(crate::monitor::g5b_analysis_v2::G5bAnalysisV2Error::Store)
    };
    let mut query=connection.prepare("SELECT p.logical_intent,p.desired_bytes,EXISTS(SELECT 1 FROM g5b_artifact_events c WHERE c.logical_intent=p.logical_intent AND c.phase='Committed') FROM g5b_artifact_events p WHERE p.cohort_identity=?1 AND p.artifact_role='Archive' AND p.phase='Prepared' ORDER BY p.prepared_revision,p.event_identity")?;
    let archives = query
        .query_map([cohort], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<(String, Vec<u8>, bool)>>>()?;
    owner_bytes_from_model_rows(&selection, index, &load, &archives).map_err(codec_error)
}

pub(super) fn validate_owner_tx(
    connection: &Connection,
    envelope: &DeliveryEnvelope,
) -> Result<()> {
    let owner:Option<(String,String,Vec<u8>,Vec<u8>,Vec<u8>,Vec<u8>)>=connection.query_row(
        "SELECT business_date,cohort_identity,frozen_canonical,source_canonical,rendered_bytes,envelope_canonical FROM g5b_occurrence_owners WHERE occurrence_identity=?1 AND decision_identity=?2",
        params![envelope.schedule_occurrence_identity,envelope.decision_identity],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let (date, cohort, frozen, source, rendered, raw) =
        owner.ok_or_else(|| mismatch("v2 decision has no unique immutable owner"))?;
    let index:i64=connection.query_row("SELECT selection_index FROM g5b_selected_occurrences WHERE business_date=?1 AND cohort_identity=?2 AND occurrence_identity=?3",params![date,cohort,envelope.schedule_occurrence_identity],|r|r.get(0))?;
    let actual = stored_model_owner(
        connection,
        &date,
        &cohort,
        usize::try_from(index).map_err(|_| mismatch("negative member index"))?,
    )?;
    if actual.envelope != *envelope
        || actual.date.to_string() != date
        || actual.cohort != cohort
        || actual.occurrence != envelope.schedule_occurrence_identity
        || actual.frozen != frozen
        || actual.source != source
        || actual.rendered != rendered
        || envelope.canonical_bytes()? != raw
    {
        return Err(mismatch(
            "owner differs from original Committed model/archive bytes",
        ));
    }
    Ok(())
}

pub(super) fn validate_global_rows(connection: &Connection) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct SourceProjection {
        source_binding_canonical: Vec<u8>,
    }
    // Real persisted kind/date determine the routing scope. Partial JSON is
    // used only for classification, never to qualify a full envelope/owner.
    let mut query=connection.prepare("SELECT decision_identity,business_date,envelope_canonical FROM delivery_decisions WHERE push_kind='G5bAttribution'")?;
    let rows = query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, date, raw) in rows {
        let cohort_present: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM g5b_cohorts WHERE business_date=?1)",
            [&date],
            |r| r.get(0),
        )?;
        let v2 = match serde_json::from_slice::<SourceProjection>(&raw) {
            Ok(source) => {
                classify_stored_source_v2(&source.source_binding_canonical).map_err(codec_error)?
            }
            Err(_) => false,
        };
        if !v2 {
            if cohort_present {
                return Err(mismatch(
                    "v2 cohort has a legacy/unknown extra counted decision",
                ));
            }
            // Preserve no-cohort historical Unknown bytes. The original
            // v1 reader still rejects them locally; this gate never adopts them.
            continue;
        }
        let stored = load_decision(connection, &id)?
            .ok_or_else(|| mismatch("stored v2 decision disappeared"))?;
        let envelope = parse_envelope(&stored.envelope_canonical)?;
        if envelope.push_kind != PushKind::G5bAttribution
            || envelope.business_date != date
            || envelope.decision_identity != id
        {
            return Err(mismatch("v2 stored kind/date/identity columns differ"));
        }
        validate_owner_tx(connection, &envelope)?;
    }
    // Owners cannot be laundered through the v1 counted reader.
    let mut query = connection.prepare("SELECT envelope_canonical FROM g5b_occurrence_owners")?;
    let rows = query
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for raw in rows {
        let envelope = parse_envelope(&raw)?;
        if !source_is_v2(&envelope.source_binding_canonical) {
            return Err(mismatch("member owner has legacy/unknown source"));
        }
        validate_owner_tx(connection, &envelope)?;
    }
    Ok(())
}

enum AdmissionClock {
    Real,
    #[cfg(test)]
    Test(DateTime<Utc>),
}
impl DurableDeliveryCoordinator {
    pub(crate) fn inspect_g5b_model_dispatch_v2(
        &self,
        date: NaiveDate,
        index: usize,
    ) -> Result<Option<G5bV2OwnerBytes>> {
        let session = self.g5b_day_session(date)?;
        let Some(bundle) = session.read_model_bundle()? else {
            return Ok(None);
        };
        let selected = bundle
            .members()
            .get(index)
            .ok_or_else(|| mismatch("dispatch member index absent"))?;
        if !selected.frozen().is_some_and(|v| v.is_committed())
            || !bundle.archives().iter().any(|v| v.is_committed())
        {
            return Ok(None);
        }
        let admission = bundle_owner(&bundle, index)?;
        bundle.verify_files(&session)?;
        Ok(Some(admission.bytes))
    }
    pub(crate) fn prepare_g5b_model_owner_v2(
        &self,
        date: NaiveDate,
        index: usize,
        sink_count: usize,
    ) -> Result<(PrepareOutcome, DeliveryEnvelope)> {
        self.prepare_g5b_model_owner_local(date, index, sink_count, AdmissionClock::Real)
    }
    #[cfg(test)]
    pub(crate) fn prepare_g5b_model_owner_v2_at_for_test(
        &self,
        date: NaiveDate,
        index: usize,
        sink_count: usize,
        now: DateTime<Utc>,
    ) -> Result<(PrepareOutcome, DeliveryEnvelope)> {
        self.prepare_g5b_model_owner_local(date, index, sink_count, AdmissionClock::Test(now))
    }
    fn prepare_g5b_model_owner_local(
        &self,
        date: NaiveDate,
        index: usize,
        sink_count: usize,
        clock: AdmissionClock,
    ) -> Result<(PrepareOutcome, DeliveryEnvelope)> {
        let session = self.g5b_day_session(date)?;
        let now = match clock {
            AdmissionClock::Real => Utc::now(),
            #[cfg(test)]
            AdmissionClock::Test(now) => {
                session.validate_analysis_test_owner()?;
                now
            }
        };
        let bundle = session
            .read_model_bundle()?
            .ok_or_else(|| mismatch("actual model cohort absent"))?;
        let admission = bundle_owner(&bundle, index)?;
        let envelope = &admission.bytes.envelope;
        let raw = envelope.canonical_bytes()?;
        let sha = sha256_hex(&raw);
        let route = self.prepare_mutation_route(envelope)?;
        let verify = || bundle.verify_files(&session);
        let delta = std::cell::Cell::new(None);
        let validate_sql = |tx: &Transaction<'_>| {
            bundle.verify_sql_with_revision_delta(
                &session,
                tx,
                delta
                    .get()
                    .ok_or_else(|| mismatch("prepare effect was not observed"))?,
            )
        };
        let result=session.with_held_transaction_sql(&verify,Some(&validate_sql),|tx| {
            bundle.verify_sql(&session,tx)?;
            self.mutation_transaction_body(tx,&route,|tx| {
                let effect=self.prepare_transaction_body(tx,&route,envelope,&raw,&sha,sink_count,now,None,Some(&admission))?;
                match &effect {
                    MutationEffect::Changed(PrepareTransactionOutcome::Inserted)=> {
                        tx.execute("INSERT INTO g5b_occurrence_owners(occurrence_identity,business_date,cohort_identity,decision_identity,frozen_canonical,frozen_sha256,source_canonical,source_sha256,rendered_bytes,rendered_sha256,envelope_canonical,envelope_sha256) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![admission.bytes.occurrence,date.to_string(),admission.bytes.cohort,envelope.decision_identity,admission.bytes.frozen,sha256_hex(&admission.bytes.frozen),admission.bytes.source,sha256_hex(&admission.bytes.source),admission.bytes.rendered,sha256_hex(&admission.bytes.rendered),raw,sha])?;
                        validate_owner_tx(tx,envelope)?;
                    }
                    MutationEffect::NoChange(PrepareTransactionOutcome::Existing(_))|MutationEffect::Changed(PrepareTransactionOutcome::Existing(_))=>validate_owner_tx(tx,envelope)?,
                    MutationEffect::Changed(PrepareTransactionOutcome::IdentityConflict)=>{},
                    _=>return Err(mismatch("unexpected prepare effect")),
                }
                delta.set(Some(if matches!(&effect,MutationEffect::Changed(_)) {1} else {0}));
                Ok(effect)
            })
        })?;
        bundle.verify_files(&session)?;
        let outcome = match result {
            PrepareTransactionOutcome::Existing(value) => Ok(*value),
            PrepareTransactionOutcome::IdentityConflict => {
                Err(DurableDeliveryError::DecisionIdentityConflict {
                    decision_identity: envelope.decision_identity.clone(),
                })
            }
            PrepareTransactionOutcome::Inserted => {
                session.with_held_transaction_sql(&verify, Some(&validate_sql), |tx| {
                    validate_owner_tx(tx, envelope)?;
                    let stored = load_decision(tx, &envelope.decision_identity)?
                        .ok_or_else(|| mismatch("inserted owner decision absent"))?;
                    let hydration = load_schedule_hydration(tx, &envelope.decision_identity)?;
                    Ok(outcome_from_stored(&stored, &hydration))
                })
            }
        }?;
        Ok((outcome, envelope.clone()))
    }

    /// Used only immediately before a new physical attempt or retry. Raw sink
    /// results, late receipts and immutable acknowledgements keep the original
    /// date-only route, so damaged model files never discard observed evidence.
    pub(super) fn with_pre_sink_mutation_transaction<T>(
        &self,
        route: &DecisionMutationRoute,
        operation: impl FnOnce(&Transaction<'_>) -> Result<MutationEffect<T>>,
    ) -> Result<T> {
        let v2 = route
            .expected_envelope
            .as_ref()
            .map(|(raw, _)| parse_envelope(raw))
            .transpose()?
            .is_some_and(|e| {
                e.push_kind == PushKind::G5bAttribution && source_is_v2(&e.source_binding_canonical)
            });
        if !v2 {
            return self.with_mutation_transaction(route, operation);
        }
        #[cfg(test)]
        self.run_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterMutationRoutingBeforeDateFence,
        )?;
        let date = route
            .g5b_date
            .ok_or_else(|| mismatch("v2 route missing stored date"))?;
        let session = self.g5b_day_session(date)?;
        let bundle = session
            .read_model_bundle()?
            .ok_or_else(|| mismatch("v2 replay model bundle absent"))?;
        let verify = || bundle.verify_files(&session);
        let delta = std::cell::Cell::new(None);
        let validate_sql = |tx: &Transaction<'_>| {
            bundle.verify_sql_with_revision_delta(
                &session,
                tx,
                delta
                    .get()
                    .ok_or_else(|| mismatch("pre-sink effect was not observed"))?,
            )
        };
        let result = session.with_held_transaction_sql(&verify, Some(&validate_sql), |tx| {
            bundle.verify_sql(&session, tx)?;
            let stored = load_decision(tx, &route.decision_identity)?
                .ok_or_else(|| mismatch("v2 replay decision absent"))?;
            let envelope = parse_envelope(&stored.envelope_canonical)?;
            validate_owner_tx(tx, &envelope)?;
            self.mutation_transaction_body(tx, route, |tx| {
                let effect = operation(tx)?;
                delta.set(Some(if matches!(&effect, MutationEffect::Changed(_)) {
                    1
                } else {
                    0
                }));
                Ok(effect)
            })
        });
        match (result, verify()) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(DurableDeliveryError::IsolationViolation(format!(
                "v2 model validation failed after COMMIT succeeded: {error}"
            ))),
            (Err(primary), Err(post)) => Err(DurableDeliveryError::IsolationViolation(format!(
                "v2 transaction and model validation failed; operation={primary}; post={post}"
            ))),
        }
    }
}
