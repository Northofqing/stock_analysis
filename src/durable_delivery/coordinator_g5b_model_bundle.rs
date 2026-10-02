//! One SQLite snapshot plus one final verification of all actual model files.
//! No model, provider, caller JSON factory, counted owner or completion seal.
use super::*;

/// One typed layer retains every original byte and file witness. Nested JSON
/// is opaque bytes, never parsed or normalized by this binding codec.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SealModelArtifactBinding {
    pub(super) event_identity: String,
    pub(super) material: EventMaterial,
    pub(super) desired_bytes: physical::EvidenceBytes,
    pub(super) committed: artifact::FileWitness,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SealModelBinding {
    pub(super) selection_bytes: physical::EvidenceBytes,
    pub(super) admission: Admission,
    pub(super) artifacts: Vec<SealModelArtifactBinding>,
}

#[derive(Serialize)]
struct ArtifactBindingRef<'a> {
    event_identity: &'a str,
    material: &'a EventMaterial,
    desired_bytes: physical::ByteRef<'a>,
    committed: Option<&'a artifact::FileWitness>,
}
impl G5bModelArtifact {
    fn binding_ref(&self) -> ArtifactBindingRef<'_> {
        ArtifactBindingRef {
            event_identity: &self.intent.event_identity,
            material: &self.intent.material,
            desired_bytes: physical::ByteRef(&self.intent.desired_bytes),
            committed: self.committed.as_ref(),
        }
    }
}

pub(crate) struct G5bModelArtifact {
    intent: PreparedG5bArtifact,
    committed: Option<artifact::FileWitness>,
}
impl G5bModelArtifact {
    pub(crate) fn identity(&self) -> &str {
        self.intent.identity()
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        self.intent.desired_bytes()
    }
    pub(crate) fn is_committed(&self) -> bool {
        self.committed.is_some()
    }
    pub(crate) fn intent(&self) -> &PreparedG5bArtifact {
        &self.intent
    }
}
pub(crate) struct G5bModelMemberArtifacts {
    index: usize,
    occurrence: String,
    attempt: Option<G5bModelArtifact>,
    frozen: Option<G5bModelArtifact>,
}
impl G5bModelMemberArtifacts {
    pub(crate) fn index(&self) -> usize {
        self.index
    }
    pub(crate) fn occurrence(&self) -> &str {
        &self.occurrence
    }
    pub(crate) fn attempt(&self) -> Option<&G5bModelArtifact> {
        self.attempt.as_ref()
    }
    pub(crate) fn frozen(&self) -> Option<&G5bModelArtifact> {
        self.frozen.as_ref()
    }
}

/// Private actual-source capability. Every referenced Committed model file and
/// original Selection is checked together after the last SQL hook. Archive
/// bytes remain opaque here; the C codec supplies their schema qualification.
pub(crate) struct VerifiedG5bModelBundle {
    cohort: VerifiedStoredG5bCohort,
    selection: G5bModelArtifact,
    members: Vec<G5bModelMemberArtifacts>,
    archives: Vec<G5bModelArtifact>,
    revision: i64,
    head_state: String,
    seal_pointer: Option<String>,
    sql_binding: Vec<u8>,
}
impl VerifiedG5bModelBundle {
    pub(crate) fn cohort(&self) -> &VerifiedStoredG5bCohort {
        &self.cohort
    }
    pub(crate) fn members(&self) -> &[G5bModelMemberArtifacts] {
        &self.members
    }
    pub(crate) fn archives(&self) -> &[G5bModelArtifact] {
        &self.archives
    }

    pub(super) fn seal_model_binding(
        &self,
        budget: &mut physical::WitnessBudget,
    ) -> Result<SealModelBinding> {
        budget.reserve(self.cohort.selection_bytes().len())?;
        let mut items = Vec::new();
        let mut add = |value: &G5bModelArtifact| -> Result<()> {
            let committed = value
                .committed
                .as_ref()
                .ok_or_else(|| mismatch("Physical seal has uncommitted model artifact"))?;
            budget.reserve(value.intent.desired_bytes.len())?;
            items.push(SealModelArtifactBinding {
                event_identity: value.intent.event_identity.clone(),
                material: value.intent.material.clone(),
                desired_bytes: physical::EvidenceBytes::Owned(value.intent.desired_bytes.clone()),
                committed: committed.clone(),
            });
            Ok(())
        };
        add(&self.selection)?;
        for member in &self.members {
            add(member
                .attempt
                .as_ref()
                .ok_or_else(|| mismatch("Physical seal model Attempt missing"))?)?;
            add(member
                .frozen
                .as_ref()
                .ok_or_else(|| mismatch("Physical seal model Frozen missing"))?)?;
        }
        for archive in &self.archives {
            add(archive)?;
        }
        Ok(SealModelBinding {
            selection_bytes: physical::EvidenceBytes::Owned(self.cohort.selection_bytes().to_vec()),
            admission: self.cohort.admission.clone(),
            artifacts: items,
        })
    }
    /// Actual borrowed bodies are interned before any Physical-only copy.
    /// Original model/file codecs and the v1 binding path above are unchanged.
    pub(super) fn seal_model_binding_shared(
        &self,
        budget: &mut physical::WitnessBudget,
    ) -> Result<SealModelBinding> {
        let mut items = Vec::new();
        let mut add = |value: &G5bModelArtifact| -> Result<()> {
            let committed = value
                .committed
                .as_ref()
                .ok_or_else(|| mismatch("Physical seal has uncommitted model artifact"))?;
            budget.reserve(4 * std::mem::size_of::<SealModelArtifactBinding>())?;
            budget.reserve(value.intent.event_identity.len())?;
            budget.reserve_clone(&value.intent.material)?;
            budget.reserve_clone(committed)?;
            let desired_bytes = budget.intern(&value.intent.desired_bytes)?;
            items.push(SealModelArtifactBinding {
                event_identity: value.intent.event_identity.clone(),
                material: value.intent.material.clone(),
                desired_bytes,
                committed: committed.clone(),
            });
            Ok(())
        };
        add(&self.selection)?;
        for member in &self.members {
            add(member
                .attempt
                .as_ref()
                .ok_or_else(|| mismatch("Physical seal model Attempt missing"))?)?;
            add(member
                .frozen
                .as_ref()
                .ok_or_else(|| mismatch("Physical seal model Frozen missing"))?)?;
        }
        for archive in &self.archives {
            add(archive)?;
        }
        budget.reserve_clone(&self.cohort.admission)?;
        Ok(SealModelBinding {
            selection_bytes: budget.intern(self.cohort.selection_bytes())?,
            admission: self.cohort.admission.clone(),
            artifacts: items,
        })
    }
    pub(super) fn revision(&self) -> i64 {
        self.revision
    }
    pub(super) fn head_state(&self) -> &str {
        &self.head_state
    }
    pub(super) fn artifact_filenames(&self) -> Vec<String> {
        let mut names = vec![artifact::filename(&self.selection.intent)];
        for member in &self.members {
            if let Some(value) = &member.attempt {
                names.push(artifact::filename(&value.intent));
            }
            if let Some(value) = &member.frozen {
                names.push(artifact::filename(&value.intent));
            }
        }
        for value in &self.archives {
            names.push(artifact::filename(&value.intent));
        }
        names
    }
    fn binding(&self) -> Result<Vec<u8>> {
        let mut artifacts = vec![self.selection.binding_ref()];
        for member in &self.members {
            if let Some(attempt) = &member.attempt {
                artifacts.push(attempt.binding_ref());
            }
            if let Some(frozen) = &member.frozen {
                artifacts.push(frozen.binding_ref());
            }
        }
        for archive in &self.archives {
            artifacts.push(archive.binding_ref());
        }
        physical::encode_bounded(
            &(
                physical::ByteRef(self.cohort.selection_bytes()),
                &self.cohort.admission,
                self.revision,
                &self.head_state,
                &self.seal_pointer,
                artifacts,
            ),
            &mut physical::WitnessBudget::maximum(),
        )
    }
    pub(crate) fn verify_files(&self, session: &G5bDaySession<'_>) -> Result<()> {
        session.validate_stored_admission(&self.cohort.admission)?;
        session.verify_actual_prefix(&self.cohort.evidence)?;
        let verify = |value: &G5bModelArtifact| -> Result<()> {
            if let Some(expected) = &value.committed {
                if artifact::inspect(session, &value.intent)? != *expected {
                    return Err(mismatch("actual model bundle file witness changed"));
                }
            }
            Ok(())
        };
        verify(&self.selection)?;
        for member in &self.members {
            if let Some(attempt) = &member.attempt {
                verify(attempt)?;
            }
            if let Some(frozen) = &member.frozen {
                verify(frozen)?;
            }
        }
        for archive in &self.archives {
            verify(archive)?;
        }
        // The prefix must still match after reading every model/archive file.
        session.verify_actual_prefix(&self.cohort.evidence)
    }
    pub(crate) fn verify_sql(
        &self,
        session: &G5bDaySession<'_>,
        tx: &Transaction<'_>,
    ) -> Result<()> {
        self.verify_sql_with_revision_delta(session, tx, 0)
    }
    pub(crate) fn verify_sql_with_revision_delta(
        &self,
        session: &G5bDaySession<'_>,
        tx: &Transaction<'_>,
        delta: i64,
    ) -> Result<()> {
        if !matches!(delta, 0 | 1) {
            return Err(mismatch("unsupported own mutation revision delta"));
        }
        let mut current = load_bundle(tx, session.date)?
            .ok_or_else(|| mismatch("actual model bundle disappeared"))?;
        let expected = self
            .revision
            .checked_add(delta)
            .ok_or_else(|| mismatch("expected revision exhausted"))?;
        if current.revision != expected {
            return Err(mismatch(
                "actual model bundle revision differs from own mutation effect",
            ));
        }
        // Normalize only our explicitly declared effect. Every original cohort,
        // Selection/model/archive intent and actual witness remains exact.
        current.revision = self.revision;
        if current.binding()? != self.sql_binding {
            return Err(mismatch("actual model bundle SQL snapshot changed"));
        }
        Ok(())
    }
}

fn load_artifact(tx: &Connection, identity: &str) -> Result<G5bModelArtifact> {
    let intent =
        load_intent(tx, identity)?.ok_or_else(|| mismatch("bundle original intent absent"))?;
    let committed: Option<Vec<u8>> = tx.query_row(
        "SELECT file_witness_canonical FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Committed'",
        [identity], |row|row.get(0)).optional()?;
    Ok(G5bModelArtifact {
        intent,
        committed: committed
            .map(|bytes| serde_json::from_slice(&bytes))
            .transpose()?,
    })
}

// SQL-only loader is not a trusted factory: read_model_bundle performs the
// unified physical verification before its result can escape the owner.
pub(super) fn load_bundle(
    tx: &Connection,
    date: NaiveDate,
) -> Result<Option<VerifiedG5bModelBundle>> {
    let head: Option<(i64,String,Option<String>,Option<String>)> = tx
        .query_row(
            "SELECT revision,artifact_state,cohort_identity,current_seal_identity FROM g5b_day_heads WHERE business_date=?1",
            [date.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?,row.get(2)?,row.get(3)?)),
        )
        .optional()?;
    let Some((revision, head_state, Some(identity), seal_pointer)) = head else {
        return Ok(None);
    };
    let (selection_bytes,admission_bytes):(Vec<u8>,Vec<u8>)=tx.query_row(
        "SELECT selection_canonical,admission_canonical FROM g5b_cohorts WHERE business_date=?1 AND cohort_identity=?2",
        params![date.to_string(),identity], |row|Ok((row.get(0)?,row.get(1)?)))?;
    let evidence = G5bSelectionEvidence::decode(&selection_bytes).map_err(codec_error)?;
    if evidence.cohort_identity() != identity {
        return Err(mismatch("bundle cohort identity differs"));
    }
    let admission = decode_admission(&admission_bytes, &evidence)?;
    let selection = load_selection_intent(tx, &identity)?;
    let selection = load_artifact(tx, selection.identity())?;
    if !selection.is_committed() {
        return Err(mismatch("model bundle Selection uncommitted"));
    }
    let mut members = Vec::new();
    for (index, (occurrence, _)) in evidence.occurrences().iter().enumerate() {
        let load = |role: &str| -> Result<Option<G5bModelArtifact>> {
            let mut query=tx.prepare("SELECT logical_intent FROM g5b_artifact_events WHERE cohort_identity=?1 AND occurrence_identity=?2 AND artifact_role=?3 AND phase='Prepared'")?;
            let values = query
                .query_map(params![identity, occurrence, role], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if values.len() > 1 {
                return Err(mismatch("ambiguous original model bundle artifacts"));
            }
            values
                .first()
                .map(|value| load_artifact(tx, value))
                .transpose()
        };
        members.push(G5bModelMemberArtifacts {
            index,
            occurrence: occurrence.clone(),
            attempt: load("Attempt")?,
            frozen: load("Frozen")?,
        });
    }
    let mut query=tx.prepare("SELECT logical_intent FROM g5b_artifact_events WHERE cohort_identity=?1 AND artifact_role='Archive' AND phase='Prepared' ORDER BY prepared_revision,event_identity")?;
    let identities = query
        .query_map([&identity], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let archives = identities
        .iter()
        .map(|value| load_artifact(tx, value))
        .collect::<Result<Vec<_>>>()?;
    let mut value = VerifiedG5bModelBundle {
        cohort: VerifiedStoredG5bCohort {
            evidence,
            admission,
        },
        selection,
        members,
        archives,
        revision,
        head_state,
        seal_pointer,
        sql_binding: Vec::new(),
    };
    value.sql_binding = value.binding()?;
    Ok(Some(value))
}

impl G5bDaySession<'_> {
    pub(crate) fn read_model_bundle(&self) -> Result<Option<VerifiedG5bModelBundle>> {
        let bundle = self.transaction(|tx| load_bundle(tx, self.date))?;
        if let Some(bundle) = &bundle {
            bundle.verify_files(self)?;
        }
        Ok(bundle)
    }
    pub(crate) fn prepare_model_archive(
        &self,
        bundle: &VerifiedG5bModelBundle,
        bytes: &[u8],
    ) -> Result<PreparedG5bArtifact> {
        bundle.verify_files(self)?;
        let validate = || bundle.verify_files(self);
        self.coordinator.with_immediate_transaction_validated(
            SchemaVersionPolicy::Runtime,
            Some(&validate),
            |tx| {
                bundle.verify_sql(self, tx)?;
                prepare_artifact_tx(
                    tx,
                    self.date,
                    &bundle.cohort.identity(),
                    ArtifactRole::Archive,
                    None,
                    bytes,
                )
            },
        )
    }
    fn validate_bundle_archive(
        &self,
        bundle: &VerifiedG5bModelBundle,
        intent: &PreparedG5bArtifact,
    ) -> Result<()> {
        if intent.material.intent.role != ArtifactRole::Archive
            || intent.material.intent.business_date != self.date
            || intent.material.intent.cohort_identity != bundle.cohort.identity()
            || !bundle.archives.iter().any(|saved| {
                saved.identity() == intent.identity() && saved.bytes() == intent.desired_bytes()
            })
        {
            return Err(mismatch(
                "archive is not an original intent in this actual bundle",
            ));
        }
        Ok(())
    }
    pub(crate) fn publish_model_archive(
        &self,
        bundle: &VerifiedG5bModelBundle,
        intent: &PreparedG5bArtifact,
    ) -> Result<()> {
        self.validate_bundle_archive(bundle, intent)?;
        bundle.verify_files(self)?;
        self.publish_prepared_artifact(intent)?;
        bundle.verify_files(self)
    }
    pub(crate) fn commit_model_archive(
        &self,
        bundle: &VerifiedG5bModelBundle,
        intent: &PreparedG5bArtifact,
    ) -> Result<()> {
        self.validate_bundle_archive(bundle, intent)?;
        let validate = || bundle.verify_files(self);
        let validate_sql = |tx: &Transaction<'_>| bundle.verify_sql(self, tx);
        self.commit_prepared_artifact_with_validation(intent, Some(&validate), Some(&validate_sql))
    }
}
