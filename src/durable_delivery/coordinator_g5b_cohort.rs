//! Private local cohort/intents, never a send permission or a day seal.

use super::{
    canonical_json, domain_sha256_hex, parse_envelope, sha256_hex, DurableDeliveryCoordinator,
    DurableDeliveryError, G5bDateFence, Result, SchemaVersionPolicy,
};
use crate::llm::{registry::LlmRegistry, LlmProvider};
use crate::monitor::g5b_selection_v2::{G5bSelectionEvidence, G5bSelectionV2Candidate};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[path = "coordinator_g5b_artifact.rs"]
mod artifact;
#[path = "coordinator_g5b_empty.rs"]
mod empty;
#[path = "coordinator_g5b_model_bundle.rs"]
mod model_bundle;
pub(crate) use empty::VerifiedG5bEmptySeal;
pub(crate) use model_bundle::VerifiedG5bModelBundle;

const ADMISSION_MATERIAL: &str = "g5b-configured-analysis-owner-v1";
const MAX_ARTIFACT_BYTES: usize = 32 * 1024 * 1024;

fn mismatch(message: &str) -> DurableDeliveryError {
    DurableDeliveryError::PolicyMismatch(format!("g5b cohort: {message}"))
}
fn io_error(error: std::io::Error) -> DurableDeliveryError {
    DurableDeliveryError::IsolationViolation(format!("g5b artifact namespace: {error}"))
}
fn codec_error(error: impl std::fmt::Debug) -> DurableDeliveryError {
    mismatch(&format!("encoded evidence rejected: {error:?}"))
}
fn checked_integer(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| mismatch("unsigned evidence exceeds SQLite integer range"))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    material: String,
    business_date: NaiveDate,
    cohort_identity: String,
    selection_sha256: String,
    calendar_authority_sha256: String,
    namespace_device: u64,
    namespace_inode: u64,
    database_device: u64,
    database_inode: u64,
    owner_instance: String,
    environment: String,
    observed_at: DateTime<Utc>,
    provider: String,
    model: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProspectiveObservation {
    material: String,
    business_date: NaiveDate,
    observed_at: DateTime<Utc>,
    calendar_authority_sha256: String,
    input_head_canonical: Vec<u8>,
    input_head_sha256: String,
    head_witness: artifact::FileWitness,
    namespace_device: u64,
    namespace_inode: u64,
    lock_device: u64,
    lock_inode: u64,
    database_device: u64,
    database_inode: u64,
    environment: String,
}

fn prospective_time(date: NaiveDate, now: DateTime<Utc>) -> bool {
    let local = now.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap());
    local.date_naive() == date && local.time() < NaiveTime::from_hms_opt(15, 5, 0).unwrap()
}
fn decode_prospective(bytes: &[u8]) -> Result<ProspectiveObservation> {
    let value: ProspectiveObservation = serde_json::from_slice(bytes)?;
    let head = crate::monitor::alert_log::validate_saved_input_head(
        value.business_date,
        &value.input_head_canonical,
    )
    .map_err(codec_error)?;
    if canonical_json(&value)? != bytes
        || value.material != "g5b-observed-zero-head-before-window-v1"
        || !prospective_time(value.business_date, value.observed_at)
        || !crate::calendar::verified_a_share_trading_day(value.business_date)
            .map_err(codec_error)?
        || value.calendar_authority_sha256
            != crate::calendar::verified_a_share_calendar_authority_hash(value.business_date)
                .map_err(codec_error)?
        || head.generation() != 0
        || head.committed_offset() != 0
        || head.source_identity().is_some()
        || sha256_hex(&value.input_head_canonical) != value.input_head_sha256
        || value.head_witness.sha256 != value.input_head_sha256
        || value.head_witness.byte_length != value.input_head_canonical.len() as u64
        || value.head_witness.filename
            != format!(
                "{}.input-head.v1.json",
                value.business_date.format("%Y%m%d")
            )
        || (value.environment != "Production" && !value.environment.starts_with("Test:TEST_CODE"))
    {
        return Err(mismatch("prospective observation preimage mismatch"));
    }
    Ok(value)
}

fn analysis_window(date: NaiveDate, now: DateTime<Utc>) -> bool {
    let local = now.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap());
    local.date_naive() == date
        && crate::calendar::verified_a_share_trading_day(date).unwrap_or(false)
        && local.time() >= NaiveTime::from_hms_opt(15, 5, 0).unwrap()
        && local.time() < NaiveTime::from_hms_opt(15, 21, 0).unwrap()
}

fn decode_admission(bytes: &[u8], evidence: &G5bSelectionEvidence) -> Result<Admission> {
    let value: Admission = serde_json::from_slice(bytes)?;
    if canonical_json(&value)? != bytes
        || value.material != ADMISSION_MATERIAL
        || value.business_date != evidence.encoded().business_date
        || value.cohort_identity != evidence.cohort_identity()
        || value.selection_sha256 != sha256_hex(evidence.canonical())
        || value.calendar_authority_sha256
            != crate::calendar::verified_a_share_calendar_authority_hash(value.business_date)
                .map_err(codec_error)?
        || (value.environment == "Production"
            && !matches!(value.provider.as_str(), "deepseek" | "minimax"))
        || !analysis_window(value.business_date, value.observed_at)
        || value.owner_instance.trim().is_empty()
        || value.provider.trim().is_empty()
        || value.model.trim().is_empty()
        || (value.environment != "Production" && !value.environment.starts_with("Test:TEST_CODE"))
    {
        return Err(mismatch("owner admission preimage mismatch"));
    }
    Ok(value)
}

/// Holds one already-acquired physical date guard. No serialized constructor.
pub(crate) struct G5bDaySession<'coordinator> {
    coordinator: &'coordinator DurableDeliveryCoordinator,
    date: NaiveDate,
    fence: G5bDateFence,
    namespace_identity: (u64, u64),
}

/// The provider is minted by the real registry, not a caller trait object.
/// Its existence is ConfiguredReady, never upstream health or a model result.
pub(crate) struct G5bConfiguredAnalysis {
    provider: Arc<dyn LlmProvider>,
    namespace_identity: (u64, u64),
    date: NaiveDate,
    #[cfg(test)]
    test_now: Option<DateTime<Utc>>,
}
impl G5bConfiguredAnalysis {
    pub(crate) fn provider(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&self.provider)
    }
    #[cfg(test)]
    pub(crate) fn test_clock(&self) -> Option<DateTime<Utc>> {
        self.test_now
    }
    fn now(&self) -> DateTime<Utc> {
        #[cfg(test)]
        if let Some(now) = self.test_now {
            return now;
        }
        Utc::now()
    }
}

/// Opaque attested receipt with actual-prefix verification; still no send/seal.
pub(crate) struct VerifiedStoredG5bCohort {
    evidence: G5bSelectionEvidence,
    admission: Admission,
}
impl VerifiedStoredG5bCohort {
    pub(crate) fn identity(&self) -> String {
        self.evidence.cohort_identity()
    }
    pub(crate) fn selection_bytes(&self) -> &[u8] {
        self.evidence.canonical()
    }
    pub(crate) fn selected_count(&self) -> usize {
        self.evidence.encoded().selected.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum ArtifactRole {
    Selection,
    Attempt,
    Frozen,
    Archive,
}
impl ArtifactRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Selection => "Selection",
            Self::Attempt => "Attempt",
            Self::Frozen => "Frozen",
            Self::Archive => "Archive",
        }
    }
}

/// Opaque local snapshots. These are not model results or send capabilities;
/// the producer in C must supply its own closed payload/receipt validation.
pub(crate) enum G5bSnapshotKind {
    Attempt,
    Frozen,
    Archive,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentMaterial {
    material: String,
    business_date: NaiveDate,
    cohort_identity: String,
    role: ArtifactRole,
    occurrence_identity: Option<String>,
    desired_sha256: String,
    desired_length: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventMaterial {
    material: String,
    logical_intent: String,
    phase: String,
    intent: IntentMaterial,
    prepared_revision: i64,
    commit_revision: Option<i64>,
    before_kind: String,
    before_sha256: Option<String>,
    prepared_event_identity: Option<String>,
    file_witness: Option<artifact::FileWitness>,
}

/// Exact persistent intent; its fields cannot be supplied through JSON.
pub(crate) struct PreparedG5bArtifact {
    event_identity: String,
    logical_intent: String,
    material: EventMaterial,
    desired_bytes: Vec<u8>,
}
impl PreparedG5bArtifact {
    pub(crate) fn identity(&self) -> &str {
        &self.logical_intent
    }
    pub(crate) fn desired_bytes(&self) -> &[u8] {
        &self.desired_bytes
    }
}

impl DurableDeliveryCoordinator {
    pub(crate) fn g5b_day_session(&self, date: NaiveDate) -> Result<G5bDaySession<'_>> {
        // acquire_date_writer_fence runs before any SQLite lease/mutex.
        let fence = self
            .g5b_input_log
            .acquire_date_writer_fence(date)
            .map_err(io_error)?;
        let namespace_identity = fence.namespace_identity().map_err(io_error)?;
        let session = G5bDaySession {
            coordinator: self,
            date,
            fence,
            namespace_identity,
        };
        session.validate()?;
        Ok(session)
    }
}

impl G5bDaySession<'_> {
    pub(crate) fn with_held_transaction_sql<T>(
        &self,
        extra: &dyn Fn() -> Result<()>,
        sql_validator: Option<&dyn Fn(&Transaction<'_>) -> Result<()>>,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let validate = || {
            self.validate()?;
            extra()
        };
        let outcome = self.coordinator.with_immediate_transaction_validated_sql(
            SchemaVersionPolicy::Runtime,
            Some(&validate),
            sql_validator,
            operation,
        );
        // The connection core performs final SQL reference checks after its
        // commit validator. Recheck the preloaded files before the value exits.
        match (outcome,validate()) {
            (Ok(value),Ok(()))=>Ok(value), (Err(primary),Ok(()))=>Err(primary),
            (Ok(_),Err(error))=>Err(DurableDeliveryError::IsolationViolation(format!("held G5b witness validation failed after COMMIT succeeded: {error}"))),
            (Err(primary),Err(post))=>Err(DurableDeliveryError::IsolationViolation(format!("held G5b transaction and witness validation failed; operation={primary}; post={post}"))),
        }
    }
    fn validate(&self) -> Result<()> {
        self.fence.ensure_date(self.date).map_err(io_error)?;
        if self.fence.namespace_identity().map_err(io_error)? != self.namespace_identity {
            return Err(mismatch("date namespace identity changed"));
        }
        Ok(())
    }
    fn verify_actual_prefix(&self, evidence: &G5bSelectionEvidence) -> Result<()> {
        self.validate()?;
        let prefix = self
            .coordinator
            .g5b_input_log
            .inspect_date_input_prefix_locked(self.date, &self.fence)
            .map_err(codec_error)?;
        evidence
            .verify_locked_prefix(&prefix)
            .map_err(codec_error)?;
        self.validate()
    }

    fn transaction<T>(&self, operation: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        self.validate()?;
        let validator = || self.validate();
        self.coordinator.with_immediate_transaction_validated(
            SchemaVersionPolicy::Runtime,
            Some(&validator),
            operation,
        )
    }
    pub(crate) fn configured_analysis(&self) -> Result<G5bConfiguredAnalysis> {
        self.validate()?;
        let provider = LlmRegistry::from_env()
            .select("g5b")
            .ok_or_else(|| mismatch("configured analysis provider absent"))?;
        if !analysis_window(self.date, Utc::now()) {
            return Err(mismatch("fresh analysis window unavailable"));
        }
        Ok(G5bConfiguredAnalysis {
            provider,
            date: self.date,
            namespace_identity: self.namespace_identity,
            #[cfg(test)]
            test_now: None,
        })
    }

    /// Fresh local owner clock; never accepts a caller supplied production time.
    pub(crate) fn analysis_request_time(
        &self,
        ready: &G5bConfiguredAnalysis,
    ) -> Result<DateTime<Utc>> {
        self.validate()?;
        let now = ready.now();
        if ready.date != self.date
            || ready.namespace_identity != self.namespace_identity
            || !analysis_window(self.date, now)
        {
            return Err(mismatch("fresh analysis owner/window unavailable"));
        }
        Ok(now)
    }

    pub(crate) fn validate_analysis_call_time(&self) -> Result<()> {
        self.validate_analysis_call_time_at(Utc::now(), false)
    }
    fn validate_analysis_call_time_at(&self, now: DateTime<Utc>, test_clock: bool) -> Result<()> {
        self.validate()?;
        if test_clock
            && !matches!(
                self.coordinator.config.environment,
                crate::durable_delivery::StoreEnvironment::Test { .. }
            )
        {
            return Err(mismatch("test model clock is unavailable in production"));
        }
        if !analysis_window(self.date, now) {
            return Err(mismatch(
                "original attempt consumed; model invocation window closed",
            ));
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn validate_analysis_test_owner(&self) -> Result<()> {
        self.validate()?;
        if !matches!(
            self.coordinator.config.environment,
            crate::durable_delivery::StoreEnvironment::Test { .. }
        ) {
            return Err(mismatch("test model clock is unavailable in production"));
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn validate_analysis_call_time_for_test(&self, now: DateTime<Utc>) -> Result<()> {
        self.validate_analysis_call_time_at(now, true)
    }

    /// A present-day observation, not a claimed file creation time or Empty.
    pub(crate) fn observe_prospective_zero_head(&self) -> Result<()> {
        self.observe_prospective_zero_head_at(Utc::now(), false)
    }
    fn observe_prospective_zero_head_at(&self, now: DateTime<Utc>, test_clock: bool) -> Result<()> {
        if test_clock
            && !matches!(
                self.coordinator.config.environment,
                crate::durable_delivery::StoreEnvironment::Test { .. }
            )
        {
            return Err(mismatch(
                "test clock cannot access production prospective owner",
            ));
        }
        self.validate()?;
        if !prospective_time(self.date, now)
            || !crate::calendar::verified_a_share_trading_day(self.date).map_err(codec_error)?
        {
            return Err(mismatch(
                "prospective observation requires current verified trading day before 15:05",
            ));
        }
        let prefix = self
            .coordinator
            .g5b_input_log
            .inspect_date_input_prefix_locked_bounded(self.date, &self.fence, MAX_ARTIFACT_BYTES)
            .map_err(codec_error)?;
        let cutoff = prefix.current_cutoff().map_err(codec_error)?;
        if cutoff.head().generation() != 0
            || cutoff.head().committed_offset() != 0
            || cutoff.source_identity().is_some()
            || !cutoff.lines().is_empty()
        {
            return Err(mismatch("prospective head is not verified zero"));
        }
        let bytes = cutoff.head_canonical().to_vec();
        let filename = format!("{}.input-head.v1.json", self.date.format("%Y%m%d"));
        let head_witness = artifact::inspect_bytes(self, &filename, &bytes)?;
        let lock = self.fence.lock_identity().map_err(io_error)?;
        let db = self.coordinator.database_binding()?.objects[0].identity;
        let witness = ProspectiveObservation {
            material: "g5b-observed-zero-head-before-window-v1".to_owned(),
            business_date: self.date,
            observed_at: now,
            calendar_authority_sha256: crate::calendar::verified_a_share_calendar_authority_hash(
                self.date,
            )
            .map_err(codec_error)?
            .to_owned(),
            input_head_canonical: bytes.clone(),
            input_head_sha256: sha256_hex(&bytes),
            head_witness: head_witness.clone(),
            namespace_device: self.namespace_identity.0,
            namespace_inode: self.namespace_identity.1,
            lock_device: lock.0,
            lock_inode: lock.1,
            database_device: db.device,
            database_inode: db.inode,
            environment: self.environment(),
        };
        let canonical = canonical_json(&witness)?;
        decode_prospective(&canonical)?;
        let expected_sql = std::cell::RefCell::new(None::<Vec<u8>>);
        let read_sql = |tx: &Transaction<'_>| -> Result<Vec<u8>> {
            let row:(i64,String,Option<String>,Option<String>,Option<Vec<u8>>,Option<String>)=tx.query_row(
                "SELECT revision,artifact_state,cohort_identity,current_seal_identity,prospective_canonical,prospective_sha256 FROM g5b_day_heads WHERE business_date=?1",[self.date.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)))?;
            canonical_json(&row)
        };
        let validate_sql = |tx: &Transaction<'_>| {
            if expected_sql.borrow().as_deref() != Some(read_sql(tx)?.as_slice()) {
                return Err(mismatch(
                    "prospective exact SQL head changed at transaction boundary",
                ));
            }
            Ok(())
        };
        let validate = || {
            self.validate()?;
            if artifact::inspect_bytes(self, &filename, &bytes)? != head_witness
                || self.fence.lock_identity().map_err(io_error)? != lock
            {
                return Err(mismatch("prospective head or guard changed"));
            }
            let prefix = self
                .coordinator
                .g5b_input_log
                .inspect_date_input_prefix_locked_bounded(
                    self.date,
                    &self.fence,
                    MAX_ARTIFACT_BYTES,
                )
                .map_err(codec_error)?;
            let current = prefix.current_cutoff().map_err(codec_error)?;
            if current.head_canonical() != bytes
                || current.source_identity().is_some()
                || current.head().generation() != 0
            {
                return Err(mismatch("prospective head or guard changed"));
            }
            if !test_clock && !prospective_time(self.date, Utc::now()) {
                return Err(mismatch(
                    "prospective observation window expired at transaction boundary",
                ));
            }
            Ok(())
        };
        self.coordinator.with_immediate_transaction_validated_sql(SchemaVersionPolicy::Runtime,Some(&validate),Some(&validate_sql),|tx| {
            if !test_clock && !prospective_time(self.date,Utc::now()) {return Err(mismatch("prospective window expired before SQL"));}
            let existing:Option<(i64,Option<String>,Option<Vec<u8>>)>=tx.query_row("SELECT revision,cohort_identity,prospective_canonical FROM g5b_day_heads WHERE business_date=?1",[self.date.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            match existing {
                Some((_,_,Some(bytes)))=> {
                    let stored=decode_prospective(&bytes)?;
                    if stored.head_witness!=head_witness || stored.input_head_canonical!=witness.input_head_canonical
                        || stored.namespace_device!=witness.namespace_device || stored.namespace_inode!=witness.namespace_inode
                        || stored.lock_device!=witness.lock_device || stored.lock_inode!=witness.lock_inode
                        || stored.database_device!=witness.database_device || stored.database_inode!=witness.database_inode
                        || stored.environment!=witness.environment {return Err(mismatch("prospective observation cannot change"));}
                    Ok(())
                }
                Some((0,None,None))=> {
                    tx.execute("UPDATE g5b_day_heads SET prospective_canonical=?1,prospective_sha256=?2 WHERE business_date=?3 AND revision=0 AND cohort_identity IS NULL AND prospective_canonical IS NULL",params![canonical,sha256_hex(&canonical),self.date.to_string()])?;
                    Ok(())
                }
                Some(_)=>Err(mismatch("prospective observation cannot be retrofitted")),
                None=> {
                    tx.execute("INSERT INTO g5b_day_heads(business_date,revision,artifact_state,prospective_canonical,prospective_sha256) VALUES(?1,0,'Clean',?2,?3)",params![self.date.to_string(),canonical,sha256_hex(&canonical)])?;
                    Ok(())
                }
            }?;
            *expected_sql.borrow_mut()=Some(read_sql(tx)?);
            Ok(())
        })
    }
    fn environment(&self) -> String {
        match &self.coordinator.config.environment {
            crate::durable_delivery::StoreEnvironment::Production => "Production".to_owned(),
            crate::durable_delivery::StoreEnvironment::Test { test_code } => {
                format!("Test:{test_code}")
            }
        }
    }
    fn admission(
        &self,
        evidence: &G5bSelectionEvidence,
        ready: &G5bConfiguredAnalysis,
    ) -> Result<Admission> {
        self.validate()?;
        let now = ready.now();
        if ready.date != self.date
            || ready.namespace_identity != self.namespace_identity
            || !analysis_window(self.date, now)
        {
            return Err(mismatch(
                "fresh configured owner does not match date session",
            ));
        }
        let db = self.coordinator.database_binding()?.objects[0].identity;
        Ok(Admission {
            material: ADMISSION_MATERIAL.to_owned(),
            business_date: self.date,
            cohort_identity: evidence.cohort_identity(),
            selection_sha256: sha256_hex(evidence.canonical()),
            calendar_authority_sha256: crate::calendar::verified_a_share_calendar_authority_hash(
                self.date,
            )
            .map_err(codec_error)?
            .to_owned(),
            namespace_device: self.namespace_identity.0,
            namespace_inode: self.namespace_identity.1,
            database_device: db.device,
            database_inode: db.inode,
            owner_instance: self.coordinator.config.owner_instance_identity.clone(),
            environment: self.environment(),
            observed_at: now,
            provider: ready.provider.name().to_owned(),
            model: ready.provider.model().to_owned(),
        })
    }

    /// Persist immutable intent rows first; actual file publication is separate.
    pub(crate) fn prepare_cohort(
        &self,
        ready: &G5bConfiguredAnalysis,
    ) -> Result<PreparedG5bArtifact> {
        let prefix = self
            .coordinator
            .g5b_input_log
            .inspect_date_input_prefix_locked(self.date, &self.fence)
            .map_err(codec_error)?;
        // First admission always selects the actual current prefix after the
        // ready owner exists. A caller cannot carry an earlier candidate in.
        let candidate =
            G5bSelectionV2Candidate::from_locked_prefix(&prefix).map_err(codec_error)?;
        let evidence =
            G5bSelectionEvidence::decode(candidate.canonical_bytes()).map_err(codec_error)?;
        if evidence.encoded().business_date != self.date {
            return Err(mismatch("candidate date differs"));
        }
        self.reject_legacy_artifacts()?;
        let snapshot_leaves = std::fs::read_dir(self.fence.namespace_path().map_err(io_error)?)
            .map_err(io_error)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(io_error)?;
        let date_prefix = format!("{}.", self.date.format("%Y%m%d"));
        let unknown_snapshot = snapshot_leaves.iter().any(|leaf| {
            leaf.to_string_lossy().starts_with(&date_prefix)
                && leaf.to_string_lossy().contains(".g5b-")
                && leaf.to_string_lossy().ends_with(".v2")
        });
        let validate_prefix = || {
            self.validate()?;
            let actual = self
                .coordinator
                .g5b_input_log
                .inspect_date_input_prefix_locked(self.date, &self.fence)
                .map_err(codec_error)?;
            evidence.verify_locked_prefix(&actual).map_err(codec_error)
        };
        self.coordinator.with_immediate_transaction_validated(SchemaVersionPolicy::Runtime, Some(&validate_prefix), |tx| {
            let existing:Option<Vec<u8>>=tx.query_row("SELECT selection_canonical FROM g5b_cohorts WHERE business_date=?1",[self.date.to_string()],|r|r.get(0)).optional()?;
            if let Some(bytes)=existing {
                let saved=G5bSelectionEvidence::decode(&bytes).map_err(codec_error)?;
                saved.verify_locked_prefix(&prefix).map_err(codec_error)?;
                return load_selection_intent(tx,&saved.cohort_identity());
            }
            if unknown_snapshot {return Err(mismatch("snapshot without original saved cohort intent cannot be adopted"));}
            let admission=self.admission(&evidence,ready)?;
            decode_admission(&canonical_json(&admission)?,&evidence)?;
            let old_decisions:i64=tx.query_row("SELECT COUNT(*) FROM delivery_decisions WHERE business_date=?1 AND push_kind='G5bAttribution'",[self.date.to_string()],|r|r.get(0))?;
            if old_decisions!=0 { return Err(mismatch("legacy G5b decisions cannot be adopted")); }
            let encoded=evidence.encoded();
            let source=encoded.cutoff.source_identity.as_ref().ok_or_else(||mismatch("nonempty source identity absent"))?;
            tx.execute("INSERT INTO g5b_cohorts(cohort_identity,business_date,selection_kind,selection_canonical,selection_sha256,cohort_preimage,selected_count,cutoff_generation,cutoff_offset,prefix_sha256,source_device,source_inode,admission_canonical,admission_sha256) VALUES(?1,?2,'NonEmpty',?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![evidence.cohort_identity(),self.date.to_string(),evidence.canonical(),sha256_hex(evidence.canonical()),evidence.cohort_preimage(),encoded.selected.len() as i64,checked_integer(encoded.cutoff.generation)?,checked_integer(encoded.cutoff.committed_offset)?,encoded.cutoff.prefix_sha256,source.device.to_string(),source.inode.to_string(),canonical_json(&admission)?,sha256_hex(&canonical_json(&admission)?)])?;
            for (index,(line,(identity,preimage))) in encoded.selected.iter().zip(evidence.occurrences()).enumerate() {
                tx.execute("INSERT INTO g5b_selected_occurrences(occurrence_identity,business_date,cohort_identity,selection_index,line_ordinal,start_offset,end_offset,raw_line_bytes,raw_line_sha256,record_canonical,record_sha256,occurrence_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![identity,self.date.to_string(),evidence.cohort_identity(),index as i64,checked_integer(line.line_ordinal)?,checked_integer(line.start_offset)?,checked_integer(line.end_offset)?,line.raw_line_bytes,line.raw_line_sha256,line.record_canonical,line.record_sha256,preimage])?;
            }
            let head:Option<i64>=tx.query_row("SELECT revision FROM g5b_day_heads WHERE business_date=?1",[self.date.to_string()],|r|r.get(0)).optional()?;
            if head.is_none() { tx.execute("INSERT INTO g5b_day_heads(business_date,revision,artifact_state) VALUES(?1,0,'Dirty')",[self.date.to_string()])?; }
            prepare_artifact_tx(tx,self.date,&evidence.cohort_identity(),ArtifactRole::Selection,None,candidate.canonical_bytes())
        })
    }

    fn reject_legacy_artifacts(&self) -> Result<()> {
        let dir = match self.coordinator.config.environment {
            crate::durable_delivery::StoreEnvironment::Production => {
                crate::production_root::production_root().join("data/g5b/attempts")
            }
            crate::durable_delivery::StoreEnvironment::Test { .. } => self
                .fence
                .namespace_path()
                .map_err(io_error)?
                .join("attempts"),
        };
        // Never follow an isolated-test child alias to an unrelated journal.
        if let Ok(metadata) = std::fs::symlink_metadata(&dir) {
            if !metadata.file_type().is_dir() {
                return Err(mismatch("legacy journal namespace is not a directory"));
            }
        }
        let mut paths = vec![
            dir.join(format!("{}.selection.json", self.date)),
            dir.join(format!("{}.archive.lock", self.date)),
            dir.parent()
                .ok_or_else(|| mismatch("legacy parent absent"))?
                .join(format!("{}.jsonl", self.date)),
        ];
        for index in 0..3 {
            paths.push(dir.join(format!("{}.{index}.attempt", self.date)));
            paths.push(dir.join(format!("{}.{index}.result.json", self.date)));
        }
        for path in paths {
            match std::fs::symlink_metadata(path) {
                Ok(_) => {
                    return Err(mismatch(
                        "legacy journal/artifact cannot be adopted as a new cohort",
                    ))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        Ok(())
    }
}

/// Pure SQLite/data verification. Never obtains a flock, current clock,
/// provider, filesystem witness or root-path capability under the DB mutex.
pub(super) fn validate_rows(connection: &Connection) -> Result<()> {
    crate::durable_delivery::schema_g5b_cohort::verify_foreign_keys(connection)?;
    type CohortRow = (
        String,
        String,
        String,
        Vec<u8>,
        String,
        Vec<u8>,
        i64,
        i64,
        i64,
        String,
        Option<String>,
        Option<String>,
        Vec<u8>,
        String,
    );
    let mut statement=connection.prepare("SELECT cohort_identity,business_date,selection_kind,selection_canonical,selection_sha256,cohort_preimage,selected_count,cutoff_generation,cutoff_offset,prefix_sha256,source_device,source_inode,admission_canonical,admission_sha256 FROM g5b_cohorts ORDER BY business_date")?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<CohortRow>>>()?;
    for row in rows {
        let (
            identity,
            date,
            kind,
            bytes,
            sha,
            preimage,
            count,
            generation,
            offset,
            prefix,
            device,
            inode,
            admission,
            admission_sha,
        ) = row;
        // Empty has a separate closed-zero codec and actual guarded owner.
        // Neither codec alone grants a completion or model capability.
        if kind != "NonEmpty" {
            if kind == "Empty" {
                empty::validate_cohort_row(
                    connection,
                    &identity,
                    &date,
                    &bytes,
                    &sha,
                    &preimage,
                    count,
                    generation,
                    offset,
                    &prefix,
                    device.as_deref(),
                    inode.as_deref(),
                    &admission,
                    &admission_sha,
                )?;
                continue;
            }
            return Err(mismatch("unknown cohort selection kind"));
        }
        let evidence = G5bSelectionEvidence::decode(&bytes).map_err(codec_error)?;
        let encoded = evidence.encoded();
        let source = encoded
            .cutoff
            .source_identity
            .as_ref()
            .ok_or_else(|| mismatch("selected source identity missing"))?;
        if identity != evidence.cohort_identity()
            || date != encoded.business_date.to_string()
            || sha256_hex(&bytes) != sha
            || evidence.cohort_preimage() != preimage
            || count != encoded.selected.len() as i64
            || generation != checked_integer(encoded.cutoff.generation)?
            || offset != checked_integer(encoded.cutoff.committed_offset)?
            || prefix != encoded.cutoff.prefix_sha256
            || device.as_deref() != Some(source.device.to_string().as_str())
            || inode.as_deref() != Some(source.inode.to_string().as_str())
            || sha256_hex(&admission) != admission_sha
        {
            return Err(mismatch(
                "cohort columns do not bind exact selection preimage",
            ));
        }
        decode_admission(&admission, &evidence)?;
        type MemberRow = (
            String,
            i64,
            i64,
            i64,
            i64,
            Vec<u8>,
            String,
            Vec<u8>,
            String,
            Vec<u8>,
            String,
        );
        let mut members=connection.prepare("SELECT occurrence_identity,selection_index,line_ordinal,start_offset,end_offset,raw_line_bytes,raw_line_sha256,record_canonical,record_sha256,occurrence_preimage,business_date FROM g5b_selected_occurrences WHERE cohort_identity=?1 ORDER BY selection_index")?;
        let actual = members
            .query_map([&identity], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<MemberRow>>>()?;
        if actual.len() != encoded.selected.len() {
            return Err(mismatch("selected occurrence set is incomplete"));
        }
        for (index, ((line, (occurrence, preimage)), actual)) in encoded
            .selected
            .iter()
            .zip(evidence.occurrences())
            .zip(actual.iter())
            .enumerate()
        {
            if actual.0 != *occurrence
                || actual.1 != index as i64
                || actual.2 != checked_integer(line.line_ordinal)?
                || actual.3 != checked_integer(line.start_offset)?
                || actual.4 != checked_integer(line.end_offset)?
                || actual.5 != line.raw_line_bytes
                || actual.6 != line.raw_line_sha256
                || actual.7 != line.record_canonical
                || actual.8 != line.record_sha256
                || actual.9 != *preimage
                || actual.10 != date
            {
                return Err(mismatch("member differs from exact codec occurrence"));
            }
        }
        let heads: i64 = connection.query_row(
            "SELECT COUNT(*) FROM g5b_day_heads WHERE business_date=?1",
            [&date],
            |r| r.get(0),
        )?;
        let selections:i64=connection.query_row("SELECT COUNT(*) FROM g5b_artifact_events WHERE cohort_identity=?1 AND phase='Prepared' AND artifact_role='Selection'",[&identity],|r|r.get(0))?;
        if heads != 1 || selections != 1 {
            return Err(mismatch("cohort has no single head/selection intent"));
        }
        let selection = load_selection_intent(connection, &identity)?;
        if selection.desired_bytes != bytes {
            return Err(mismatch(
                "selection artifact intent differs from frozen codec bytes",
            ));
        }
    }
    type OwnerRow = (
        String,
        String,
        String,
        String,
        Vec<u8>,
        String,
        Vec<u8>,
        String,
        Vec<u8>,
        String,
        Vec<u8>,
        String,
    );
    let mut owners=connection.prepare("SELECT occurrence_identity,business_date,cohort_identity,decision_identity,frozen_canonical,frozen_sha256,source_canonical,source_sha256,rendered_bytes,rendered_sha256,envelope_canonical,envelope_sha256 FROM g5b_occurrence_owners")?;
    let rows = owners
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<OwnerRow>>>()?;
    for (
        occurrence,
        date,
        _cohort,
        decision,
        frozen,
        frozen_sha,
        source,
        source_sha,
        rendered,
        rendered_sha,
        envelope_bytes,
        envelope_sha,
    ) in rows
    {
        let stored = super::load_decision(connection, &decision)?
            .ok_or_else(|| mismatch("occurrence owner has no real decision"))?;
        let envelope = parse_envelope(&envelope_bytes)?;
        if stored.envelope_canonical != envelope_bytes
            || stored.envelope_sha256 != envelope_sha
            || sha256_hex(&envelope_bytes) != envelope_sha
            || sha256_hex(&frozen) != frozen_sha
            || sha256_hex(&source) != source_sha
            || sha256_hex(&rendered) != rendered_sha
            || envelope.push_kind != crate::durable_delivery::PushKind::G5bAttribution
            || envelope.business_date != date
            || envelope.decision_identity != decision
            || envelope.schedule_occurrence_identity != occurrence
            || envelope.task_binding.is_some()
            || envelope.source_binding_canonical != source
            || envelope.source_binding_sha256 != source_sha
            || envelope.rendered_content != rendered
            || envelope.rendered_content_sha256 != rendered_sha
        {
            return Err(mismatch(
                "occurrence owner differs from actual immutable decision",
            ));
        }
    }
    super::g5b_v2::validate_global_rows(connection)?;
    type EventRow = (
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
        Option<i64>,
        String,
        Option<String>,
        Vec<u8>,
        String,
        Vec<u8>,
        String,
        Option<String>,
        Option<Vec<u8>>,
        Option<String>,
    );
    let mut events=connection.prepare("SELECT event_identity,logical_intent,phase,business_date,cohort_identity,artifact_role,occurrence_identity,prepared_revision,commit_revision,before_kind,before_sha256,desired_bytes,desired_sha256,event_canonical,event_sha256,prepared_event_identity,file_witness_canonical,file_witness_sha256 FROM g5b_artifact_events")?;
    let rows = events
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
                r.get(14)?,
                r.get(15)?,
                r.get(16)?,
                r.get(17)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<EventRow>>>()?;
    let mut used_revisions = std::collections::BTreeSet::new();
    for (
        identity,
        logical,
        phase,
        date,
        cohort,
        role,
        occurrence,
        prepared,
        commit,
        before,
        before_sha,
        desired,
        desired_sha,
        canonical,
        canonical_sha,
        parent,
        witness,
        witness_sha,
    ) in rows
    {
        let material: EventMaterial = serde_json::from_slice(&canonical)?;
        if material.material != "g5b-artifact-event-v1"
            || material.intent.material != "g5b-fixed-role-snapshot-intent-v1"
            || canonical_json(&material)? != canonical
            || sha256_hex(&canonical) != canonical_sha
            || domain_sha256_hex("g5b-artifact-event-v1", &canonical) != identity
            || domain_sha256_hex(
                "g5b-fixed-role-snapshot-intent-v1",
                &canonical_json(&material.intent)?,
            ) != logical
            || material.logical_intent != logical
            || material.phase != phase
            || material.intent.business_date.to_string() != date
            || material.intent.cohort_identity != cohort
            || material.intent.role.as_str() != role
            || material.intent.occurrence_identity != occurrence
            || material.prepared_revision != prepared
            || material.commit_revision != commit
            || material.before_kind != before
            || material.before_sha256 != before_sha
            || material.prepared_event_identity != parent
            || material.intent.desired_length != desired.len() as u64
            || desired.len() > MAX_ARTIFACT_BYTES
            || material.intent.desired_sha256 != desired_sha
            || sha256_hex(&desired) != desired_sha
        {
            return Err(mismatch(
                "artifact event columns differ from exact canonical preimage",
            ));
        }
        let head_revision: i64 = connection.query_row(
            "SELECT revision FROM g5b_day_heads WHERE business_date=?1",
            [&date],
            |row| row.get(0),
        )?;
        let event_revision = if phase == "Prepared" {
            prepared
        } else {
            commit.ok_or_else(|| mismatch("committed event has no revision"))?
        };
        if prepared > head_revision
            || event_revision > head_revision
            || !used_revisions.insert((date.clone(), event_revision))
        {
            return Err(mismatch(
                "artifact event revision exceeds or repeats the date revision",
            ));
        }
        let saved = load_intent(connection, &logical)?
            .ok_or_else(|| mismatch("event has no original prepared"))?;
        if saved.desired_bytes != desired
            || saved.material.intent != material.intent
            || saved.material.prepared_revision != prepared
            || saved.material.before_kind != before
            || saved.material.before_sha256 != before_sha
        {
            return Err(mismatch("event is not bound to exact original prepared"));
        }
        if phase == "Committed" {
            if parent.as_deref() != Some(saved.event_identity.as_str())
                || commit.is_none_or(|v| v <= prepared)
            {
                return Err(mismatch("committed parent/revision differs"));
            }
            let expected = material
                .file_witness
                .as_ref()
                .ok_or_else(|| mismatch("committed file witness missing"))?;
            let encoded = canonical_json(expected)?;
            if witness.as_deref() != Some(encoded.as_slice())
                || witness_sha.as_deref() != Some(sha256_hex(&encoded).as_str())
                || expected.filename != artifact::filename(&saved)
                || expected.sha256 != desired_sha
                || expected.byte_length != desired.len() as u64
            {
                return Err(mismatch("committed file witness preimage differs"));
            }
        } else if material.file_witness.is_some()
            || witness.is_some()
            || witness_sha.is_some()
            || parent.is_some()
            || commit.is_some()
        {
            return Err(mismatch("prepared event has premature acknowledgement"));
        }
    }
    type HeadRow = (
        String,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<Vec<u8>>,
        Option<String>,
    );
    let mut heads=connection.prepare("SELECT business_date,revision,artifact_state,cohort_identity,current_seal_identity,prospective_canonical,prospective_sha256 FROM g5b_day_heads")?;
    let rows = heads
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<HeadRow>>>()?;
    for (date, revision, state, published, seal, prospective, prospective_sha) in rows {
        crate::durable_delivery::model::validate_business_date(&date)?;
        let pending:i64=connection.query_row("SELECT COUNT(*) FROM g5b_artifact_events p WHERE p.business_date=?1 AND p.phase='Prepared' AND NOT EXISTS(SELECT 1 FROM g5b_artifact_events c WHERE c.prepared_event_identity=p.event_identity AND c.phase='Committed')",[&date],|r|r.get(0))?;
        if (state == "Clean" && pending != 0) || revision < 0 {
            return Err(mismatch("clean head has uncommitted artifact"));
        }
        if let Some(cohort) = &published {
            let committed:i64=connection.query_row("SELECT COUNT(*) FROM g5b_artifact_events WHERE business_date=?1 AND cohort_identity=?2 AND artifact_role='Selection' AND phase='Committed'",params![date,cohort],|r|r.get(0))?;
            if committed != 1 {
                return Err(mismatch("published cohort lacks exact committed selection"));
            }
        }
        if let Some(seal) = seal {
            empty::validate_current_pointer(
                connection,
                &date,
                revision,
                &state,
                published.as_deref(),
                &seal,
            )?;
        }
        match (prospective, prospective_sha) {
            (None, None) => {}
            (Some(bytes), Some(hash)) => {
                if sha256_hex(&bytes) != hash
                    || decode_prospective(&bytes)?.business_date.to_string() != date
                {
                    return Err(mismatch("prospective head columns differ"));
                }
            }
            _ => return Err(mismatch("partial prospective reference")),
        }
    }
    empty::validate_seals(connection)?;
    Ok(())
}

fn prepare_artifact_tx(
    tx: &Transaction<'_>,
    date: NaiveDate,
    cohort: &str,
    role: ArtifactRole,
    occurrence: Option<String>,
    bytes: &[u8],
) -> Result<PreparedG5bArtifact> {
    if bytes.is_empty() || bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(mismatch("artifact size outside fixed snapshot limit"));
    }
    let intent = IntentMaterial {
        material: "g5b-fixed-role-snapshot-intent-v1".to_owned(),
        business_date: date,
        cohort_identity: cohort.to_owned(),
        role,
        occurrence_identity: occurrence,
        desired_sha256: sha256_hex(bytes),
        desired_length: bytes.len() as u64,
    };
    let logical_intent = domain_sha256_hex(
        "g5b-fixed-role-snapshot-intent-v1",
        &canonical_json(&intent)?,
    );
    if let Some(saved) = load_intent(tx, &logical_intent)? {
        if saved.material.intent != intent || saved.desired_bytes != bytes {
            return Err(mismatch("intent identity conflicts with saved bytes"));
        }
        return Ok(saved);
    }
    let sealed: i64 = tx.query_row(
        "SELECT COUNT(*) FROM g5b_day_seals WHERE business_date=?1",
        [date.to_string()],
        |row| row.get(0),
    )?;
    if sealed != 0 {
        return Err(mismatch(
            "historically sealed date cannot open another artifact",
        ));
    }
    let revision: i64 = tx.query_row(
        "SELECT revision FROM g5b_day_heads WHERE business_date=?1",
        [date.to_string()],
        |r| r.get(0),
    )?;
    let prepared_revision = revision
        .checked_add(1)
        .ok_or_else(|| mismatch("revision exhausted"))?;
    let material = EventMaterial {
        material: "g5b-artifact-event-v1".to_owned(),
        logical_intent: logical_intent.clone(),
        phase: "Prepared".to_owned(),
        intent,
        prepared_revision,
        commit_revision: None,
        before_kind: "Absent".to_owned(),
        before_sha256: None,
        prepared_event_identity: None,
        file_witness: None,
    };
    let canonical = canonical_json(&material)?;
    let event_identity = domain_sha256_hex("g5b-artifact-event-v1", &canonical);
    tx.execute("INSERT INTO g5b_artifact_events(event_identity,logical_intent,phase,business_date,cohort_identity,artifact_role,occurrence_identity,prepared_revision,before_kind,desired_bytes,desired_sha256,event_canonical,event_sha256) VALUES(?1,?2,'Prepared',?3,?4,?5,?6,?7,'Absent',?8,?9,?10,?11)",params![event_identity,logical_intent,date.to_string(),cohort,role.as_str(),material.intent.occurrence_identity,prepared_revision,bytes,sha256_hex(bytes),canonical,sha256_hex(&canonical)])?;
    let changed=tx.execute("UPDATE g5b_day_heads SET revision=?1,artifact_state='Dirty',current_seal_identity=NULL WHERE business_date=?2 AND revision=?3",params![prepared_revision,date.to_string(),revision])?;
    super::require_single_cas_update(changed, "prepare cohort artifact")?;
    Ok(PreparedG5bArtifact {
        event_identity,
        logical_intent,
        material,
        desired_bytes: bytes.to_vec(),
    })
}

fn load_intent(connection: &Connection, logical: &str) -> Result<Option<PreparedG5bArtifact>> {
    let row:Option<(String,Vec<u8>,Vec<u8>)>=connection.query_row("SELECT event_identity,event_canonical,desired_bytes FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Prepared'",[logical],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    row.map(|(event_identity, canonical, desired_bytes)| {
        let material: EventMaterial = serde_json::from_slice(&canonical)?;
        if canonical_json(&material)? != canonical
            || material.phase != "Prepared"
            || material.logical_intent != logical
            || sha256_hex(&desired_bytes) != material.intent.desired_sha256
            || desired_bytes.len() as u64 != material.intent.desired_length
            || domain_sha256_hex("g5b-artifact-event-v1", &canonical) != event_identity
            || domain_sha256_hex(
                "g5b-fixed-role-snapshot-intent-v1",
                &canonical_json(&material.intent)?,
            ) != logical
        {
            return Err(mismatch("prepared intent preimage differs"));
        }
        Ok(PreparedG5bArtifact {
            event_identity,
            logical_intent: logical.to_owned(),
            material,
            desired_bytes,
        })
    })
    .transpose()
}
fn load_selection_intent(connection: &Connection, cohort: &str) -> Result<PreparedG5bArtifact> {
    let logical:String=connection.query_row("SELECT logical_intent FROM g5b_artifact_events WHERE cohort_identity=?1 AND artifact_role='Selection' AND phase='Prepared'",[cohort],|r|r.get(0))?;
    load_intent(connection, &logical)?
        .ok_or_else(|| mismatch("selection has no exact saved intent"))
}

fn commit_exact_artifact_tx(
    tx: &Transaction<'_>,
    date: NaiveDate,
    intent: &PreparedG5bArtifact,
    witness: &artifact::FileWitness,
) -> Result<()> {
    let saved = load_intent(tx, &intent.logical_intent)?
        .ok_or_else(|| mismatch("prepared intent disappeared"))?;
    if saved.material != intent.material || saved.desired_bytes != intent.desired_bytes {
        return Err(mismatch("prepared identity conflict at commit"));
    }
    let committed:Option<Vec<u8>>=tx.query_row("SELECT event_canonical FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Committed'",[&intent.logical_intent],|r|r.get(0)).optional()?;
    if let Some(bytes) = committed {
        let existing: EventMaterial = serde_json::from_slice(&bytes)?;
        if existing.file_witness.as_ref() != Some(witness) {
            return Err(mismatch("committed artifact changed; no healing"));
        }
        return Ok(());
    }
    // Fresh CAS allows legitimate late/audit changes since Prepared.
    let revision: i64 = tx.query_row(
        "SELECT revision FROM g5b_day_heads WHERE business_date=?1",
        [date.to_string()],
        |r| r.get(0),
    )?;
    let next = revision
        .checked_add(1)
        .ok_or_else(|| mismatch("revision exhausted"))?;
    let mut material = intent.material.clone();
    material.phase = "Committed".to_owned();
    material.commit_revision = Some(next);
    material.prepared_event_identity = Some(intent.event_identity.clone());
    material.file_witness = Some(witness.clone());
    let canonical = canonical_json(&material)?;
    tx.execute("INSERT INTO g5b_artifact_events(event_identity,logical_intent,phase,business_date,cohort_identity,artifact_role,occurrence_identity,prepared_revision,commit_revision,before_kind,before_sha256,desired_bytes,desired_sha256,event_canonical,event_sha256,prepared_event_identity,file_witness_canonical,file_witness_sha256) VALUES(?1,?2,'Committed',?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",params![domain_sha256_hex("g5b-artifact-event-v1",&canonical),intent.logical_intent,date.to_string(),material.intent.cohort_identity,material.intent.role.as_str(),material.intent.occurrence_identity,material.prepared_revision,next,material.before_kind,material.before_sha256,intent.desired_bytes,material.intent.desired_sha256,canonical,sha256_hex(&canonical),intent.event_identity,canonical_json(&witness)?,sha256_hex(&canonical_json(&witness)?)])?;
    let pending:i64=tx.query_row("SELECT COUNT(*) FROM g5b_artifact_events p WHERE p.business_date=?1 AND p.phase='Prepared' AND NOT EXISTS(SELECT 1 FROM g5b_artifact_events c WHERE c.prepared_event_identity=p.event_identity AND c.phase='Committed')",[date.to_string()],|r|r.get(0))?;
    let publish_cohort = material.intent.role == ArtifactRole::Selection;
    let changed=tx.execute("UPDATE g5b_day_heads SET revision=?1,artifact_state=?2,current_seal_identity=NULL,cohort_identity=CASE WHEN ?3 THEN ?4 ELSE cohort_identity END WHERE business_date=?5 AND revision=?6",params![next,if pending==0{"Clean"}else{"Dirty"},publish_cohort,material.intent.cohort_identity,date.to_string(),revision])?;
    super::require_single_cas_update(changed, "commit exact artifact")
}

impl G5bDaySession<'_> {
    fn validate_stored_admission(&self, admission: &Admission) -> Result<()> {
        let db = self.coordinator.database_binding()?.objects[0].identity;
        if admission.business_date != self.date
            || (admission.namespace_device, admission.namespace_inode) != self.namespace_identity
            || (admission.database_device, admission.database_inode) != (db.device, db.inode)
            || admission.environment != self.environment()
        {
            return Err(mismatch(
                "stored owner receipt belongs to another namespace/database",
            ));
        }
        Ok(())
    }

    /// Attested evidence plus actual locked prefix; saved JSON alone is never enough.
    pub(crate) fn read_cohort(&self) -> Result<Option<VerifiedStoredG5bCohort>> {
        self.validate()?;
        let row=self.transaction(|tx| {
            tx.query_row("SELECT selection_canonical,admission_canonical FROM g5b_cohorts c JOIN g5b_day_heads h ON h.business_date=c.business_date AND h.cohort_identity=c.cohort_identity WHERE c.business_date=?1",[self.date.to_string()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?))).optional().map_err(Into::into)
        })?;
        let Some((bytes, admission_bytes)) = row else {
            return Ok(None);
        };
        let evidence = G5bSelectionEvidence::decode(&bytes).map_err(codec_error)?;
        let admission = decode_admission(&admission_bytes, &evidence)?;
        self.validate_stored_admission(&admission)?;
        let prefix = self
            .coordinator
            .g5b_input_log
            .inspect_date_input_prefix_locked(self.date, &self.fence)
            .map_err(codec_error)?;
        evidence
            .verify_locked_prefix(&prefix)
            .map_err(codec_error)?;
        let (intent,expected)=self.transaction(|tx| {
            let intent=load_selection_intent(tx,&evidence.cohort_identity())?;
            let bytes:Vec<u8>=tx.query_row("SELECT file_witness_canonical FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Committed'",[&intent.logical_intent],|r|r.get(0))?;
            let expected:artifact::FileWitness=serde_json::from_slice(&bytes)?;
            Ok((intent,expected))
        })?;
        if artifact::inspect(self, &intent)? != expected {
            return Err(mismatch("published cohort selection witness changed"));
        }
        // The last DB transaction can run after-SQL hooks. Recheck the real
        // source after that boundary, not merely the namespace and artifact.
        self.verify_actual_prefix(&evidence)?;
        Ok(Some(VerifiedStoredG5bCohort {
            evidence,
            admission,
        }))
    }

    fn validate_saved_intent(&self, intent: &PreparedG5bArtifact) -> Result<()> {
        if empty::is_empty_intent(self, intent)? {
            return empty::validate_saved_intent(self, intent);
        }
        self.validate()?;
        if intent.material.intent.business_date != self.date {
            return Err(mismatch("intent belongs to another date"));
        }
        self.transaction(|tx| {
            let saved=load_intent(tx,&intent.logical_intent)?.ok_or_else(||mismatch("missing prepared intent"))?;
            if saved.event_identity!=intent.event_identity || saved.material!=intent.material || saved.desired_bytes!=intent.desired_bytes {return Err(mismatch("incoming intent is not exact saved preimage"));}
            let (bytes,admission):(Vec<u8>,Vec<u8>)=tx.query_row("SELECT selection_canonical,admission_canonical FROM g5b_cohorts WHERE cohort_identity=?1 AND business_date=?2",params![intent.material.intent.cohort_identity,self.date.to_string()],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let evidence=G5bSelectionEvidence::decode(&bytes).map_err(codec_error)?;
            self.validate_stored_admission(&decode_admission(&admission,&evidence)?)
        })?;
        // Filesystem verification occurs after releasing the DB lease/mutex.
        let (bytes,_)=self.transaction(|tx| tx.query_row("SELECT selection_canonical,admission_canonical FROM g5b_cohorts WHERE cohort_identity=?1",[&intent.material.intent.cohort_identity],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?))).map_err(Into::into))?;
        let evidence = G5bSelectionEvidence::decode(&bytes).map_err(codec_error)?;
        let prefix = self
            .coordinator
            .g5b_input_log
            .inspect_date_input_prefix_locked(self.date, &self.fence)
            .map_err(codec_error)?;
        evidence.verify_locked_prefix(&prefix).map_err(codec_error)
    }

    /// Saved bytes only. No model, physical sink, or immutable-append callback.
    pub(crate) fn publish_prepared_artifact(&self, intent: &PreparedG5bArtifact) -> Result<()> {
        let is_empty = empty::is_empty_intent(self, intent)?;
        self.validate_saved_intent(intent)?;
        artifact::publish(self, intent)?;
        if is_empty {
            empty::validate_saved_intent(self, intent)?;
        }
        Ok(())
    }

    pub(crate) fn commit_prepared_artifact(&self, intent: &PreparedG5bArtifact) -> Result<()> {
        self.commit_prepared_artifact_with_validation(intent, None, None)
    }
    fn commit_prepared_artifact_with_validation(
        &self,
        intent: &PreparedG5bArtifact,
        extra_fs: Option<&dyn Fn() -> Result<()>>,
        extra_sql: Option<&dyn Fn(&Transaction<'_>) -> Result<()>>,
    ) -> Result<()> {
        if empty::is_empty_intent(self, intent)? {
            if extra_fs.is_some() || extra_sql.is_some() {
                return Err(mismatch("model validators do not apply to Empty artifacts"));
            }
            return empty::commit_prepared(self, intent);
        }
        self.validate_saved_intent(intent)?;
        // Fetch SQLite preimages before entering the validated write; the
        // validator itself must never reacquire the DB mutex/date guard.
        let (evidence, committed_selection) = self.transaction(|tx| {
            let bytes: Vec<u8> = tx.query_row(
                "SELECT selection_canonical FROM g5b_cohorts WHERE cohort_identity=?1",
                [&intent.material.intent.cohort_identity], |row| row.get(0),
            )?;
            let evidence = G5bSelectionEvidence::decode(&bytes).map_err(codec_error)?;
            let selection = if intent.material.intent.role == ArtifactRole::Selection {
                None // First Selection commit is the publication boundary.
            } else {
                let selection = load_selection_intent(tx, &evidence.cohort_identity())?;
                let encoded: Vec<u8> = tx.query_row(
                    "SELECT file_witness_canonical FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Committed'",
                    [&selection.logical_intent], |row| row.get(0),
                )?;
                let expected: artifact::FileWitness = serde_json::from_slice(&encoded)?;
                Some((selection, expected))
            };
            Ok((evidence, selection))
        })?;
        let witness = artifact::inspect(self, intent)?;
        let validate = || {
            self.verify_actual_prefix(&evidence)?;
            if let Some((selection, expected)) = &committed_selection {
                if artifact::inspect(self, selection)? != *expected {
                    return Err(mismatch(
                        "committed Selection changed at transaction boundary",
                    ));
                }
            }
            if artifact::inspect(self, intent)? != witness {
                return Err(mismatch("artifact witness changed at transaction boundary"));
            }
            if let Some(validate) = extra_fs {
                validate()?;
            }
            Ok(())
        };
        self.coordinator.with_immediate_transaction_validated(
            SchemaVersionPolicy::Runtime,
            Some(&validate),
            |tx| {
                if let Some(validate) = extra_sql {
                    validate(tx)?;
                }
                commit_exact_artifact_tx(tx, self.date, intent, &witness)
            },
        )
    }

    pub(crate) fn recover_prepared_artifacts(&self) -> Result<usize> {
        let logical=self.transaction(|tx| {
            let mut statement=tx.prepare("SELECT logical_intent FROM g5b_artifact_events WHERE business_date=?1 AND phase='Prepared' ORDER BY prepared_revision,event_identity")?;
            let rows=statement.query_map([self.date.to_string()],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        let mut recovered = 0;
        for logical in logical {
            let (intent,committed)=self.transaction(|tx| {
                let intent=load_intent(tx,&logical)?.ok_or_else(||mismatch("recovery prepared missing"))?;
                let committed:i64=tx.query_row("SELECT COUNT(*) FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Committed'",[&logical],|r|r.get(0))?;
                Ok((intent,committed!=0))
            })?;
            if committed {
                // Committed missing/corrupt files fail closed, never recreate.
                self.commit_prepared_artifact(&intent)?;
            } else {
                self.publish_prepared_artifact(&intent)?;
                self.commit_prepared_artifact(&intent)?;
                recovered += 1;
            }
        }
        Ok(recovered)
    }

    pub(crate) fn prepare_opaque_snapshot(
        &self,
        cohort: &VerifiedStoredG5bCohort,
        kind: G5bSnapshotKind,
        index: Option<usize>,
        bytes: &[u8],
    ) -> Result<PreparedG5bArtifact> {
        self.prepare_snapshot_inner(cohort, kind, index, bytes, None)
    }

    /// Only a newly inserted original attempt can authorize a live model call.
    /// Existing Prepared and Committed attempts both consume that opportunity.
    pub(crate) fn prepare_new_analysis_attempt(
        &self,
        cohort: &VerifiedStoredG5bCohort,
        index: usize,
        bytes: &[u8],
        ready: &G5bConfiguredAnalysis,
    ) -> Result<PreparedG5bArtifact> {
        self.analysis_request_time(ready)?;
        self.prepare_snapshot_inner(
            cohort,
            G5bSnapshotKind::Attempt,
            Some(index),
            bytes,
            Some(ready),
        )
    }

    fn prepare_snapshot_inner(
        &self,
        cohort: &VerifiedStoredG5bCohort,
        kind: G5bSnapshotKind,
        index: Option<usize>,
        bytes: &[u8],
        original_ready: Option<&G5bConfiguredAnalysis>,
    ) -> Result<PreparedG5bArtifact> {
        self.validate_stored_admission(&cohort.admission)?;
        cohort
            .evidence
            .verify_locked_prefix(
                &self
                    .coordinator
                    .g5b_input_log
                    .inspect_date_input_prefix_locked(self.date, &self.fence)
                    .map_err(codec_error)?,
            )
            .map_err(codec_error)?;
        let (role, occurrence) = match (kind, index) {
            (G5bSnapshotKind::Attempt, Some(index)) => (
                ArtifactRole::Attempt,
                Some(
                    cohort
                        .evidence
                        .occurrences()
                        .get(index)
                        .ok_or_else(|| mismatch("snapshot member index absent"))?
                        .0
                        .clone(),
                ),
            ),
            (G5bSnapshotKind::Frozen, Some(index)) => (
                ArtifactRole::Frozen,
                Some(
                    cohort
                        .evidence
                        .occurrences()
                        .get(index)
                        .ok_or_else(|| mismatch("snapshot member index absent"))?
                        .0
                        .clone(),
                ),
            ),
            (G5bSnapshotKind::Archive, None) => (ArtifactRole::Archive, None),
            _ => return Err(mismatch("snapshot role/member mismatch")),
        };
        // The selection artifact must still be physically the committed inode.
        let current = self
            .read_cohort()?
            .ok_or_else(|| mismatch("snapshot cohort is not published"))?;
        if current.identity() != cohort.identity() || current.admission != cohort.admission {
            return Err(mismatch("snapshot receipt differs from stored owner"));
        }
        self.transaction(|tx| {
            if let Some(ready) = original_ready { self.analysis_request_time(ready)?; }
            if role == ArtifactRole::Attempt {
                let mut query = tx.prepare("SELECT logical_intent,desired_bytes FROM g5b_artifact_events WHERE cohort_identity=?1 AND occurrence_identity=?2 AND artifact_role='Attempt' AND phase='Prepared'")?;
                let existing = query.query_map(params![cohort.identity(), occurrence], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                if !existing.is_empty() {
                    if original_ready.is_some() || existing.len() != 1 || existing[0].1 != bytes {
                        return Err(mismatch("one original analysis attempt already consumes this member"));
                    }
                    return load_intent(tx, &existing[0].0)?.ok_or_else(|| mismatch("original attempt missing"));
                }
            }
            prepare_artifact_tx(tx, self.date, &cohort.identity(), role, occurrence, bytes)
        })
    }

    pub(crate) fn has_member_snapshot(
        &self,
        cohort: &VerifiedStoredG5bCohort,
        kind: G5bSnapshotKind,
        index: usize,
    ) -> Result<bool> {
        self.validate_stored_admission(&cohort.admission)?;
        let occurrence = cohort
            .evidence
            .occurrences()
            .get(index)
            .ok_or_else(|| mismatch("snapshot member index absent"))?
            .0
            .clone();
        let role = match kind {
            G5bSnapshotKind::Attempt => "Attempt",
            G5bSnapshotKind::Frozen => "Frozen",
            G5bSnapshotKind::Archive => return Err(mismatch("archive has no member slot")),
        };
        self.transaction(|tx| {
            let count: i64 = tx.query_row("SELECT COUNT(*) FROM g5b_artifact_events WHERE cohort_identity=?1 AND occurrence_identity=?2 AND artifact_role=?3 AND phase='Prepared'", params![cohort.identity(), occurrence, role], |row| row.get(0))?;
            if count > 1 { return Err(mismatch("ambiguous member snapshots")); }
            Ok(count != 0)
        })
    }

    /// Read only a single actual Committed snapshot, not arbitrary saved JSON.
    pub(crate) fn read_committed_member_snapshot(
        &self,
        cohort: &VerifiedStoredG5bCohort,
        kind: G5bSnapshotKind,
        index: usize,
    ) -> Result<Option<PreparedG5bArtifact>> {
        self.validate_stored_admission(&cohort.admission)?;
        let occurrence = cohort
            .evidence
            .occurrences()
            .get(index)
            .ok_or_else(|| mismatch("snapshot member index absent"))?
            .0
            .clone();
        let role = match kind {
            G5bSnapshotKind::Attempt => "Attempt",
            G5bSnapshotKind::Frozen => "Frozen",
            G5bSnapshotKind::Archive => return Err(mismatch("archive has no member slot")),
        };
        let stored = self.transaction(|tx| {
            let mut query = tx.prepare("SELECT logical_intent FROM g5b_artifact_events WHERE cohort_identity=?1 AND occurrence_identity=?2 AND artifact_role=?3 AND phase='Prepared'")?;
            let ids = query.query_map(params![cohort.identity(), occurrence, role], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            if ids.len() > 1 { return Err(mismatch("ambiguous member snapshots")); }
            let Some(id) = ids.first() else { return Ok(None); };
            let intent = load_intent(tx, id)?.ok_or_else(|| mismatch("member intent missing"))?;
            let witness: Option<Vec<u8>> = tx.query_row("SELECT file_witness_canonical FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Committed'", [id], |row| row.get(0)).optional()?;
            let expected: artifact::FileWitness = serde_json::from_slice(&witness.ok_or_else(|| mismatch("member snapshot is Prepared, completion unproven"))?)?;
            Ok(Some((intent, expected)))
        })?;
        let Some((intent, expected)) = stored else {
            return Ok(None);
        };
        self.validate_saved_intent(&intent)?;
        let current = self
            .read_cohort()?
            .ok_or_else(|| mismatch("member cohort not published"))?;
        if current.identity() != cohort.identity() || current.admission != cohort.admission {
            return Err(mismatch("member cohort differs from stored owner"));
        }
        if artifact::inspect(self, &intent)? != expected {
            return Err(mismatch("committed member snapshot witness changed"));
        }
        self.verify_actual_prefix(&cohort.evidence)?;
        Ok(Some(intent))
    }

    #[cfg(test)]
    pub(crate) fn configured_analysis_for_test(
        &self,
        provider: Arc<dyn LlmProvider>,
        now: DateTime<Utc>,
    ) -> Result<G5bConfiguredAnalysis> {
        if !matches!(
            self.coordinator.config.environment,
            crate::durable_delivery::StoreEnvironment::Test { .. }
        ) {
            return Err(mismatch(
                "test configured owner is unavailable in production",
            ));
        }
        self.validate()?;
        if !analysis_window(self.date, now) {
            return Err(mismatch("test analysis window unavailable"));
        }
        Ok(G5bConfiguredAnalysis {
            provider,
            date: self.date,
            namespace_identity: self.namespace_identity,
            test_now: Some(now),
        })
    }
    #[cfg(test)]
    pub(crate) fn prospective_for_test(&self, now: DateTime<Utc>) -> Result<()> {
        self.observe_prospective_zero_head_at(now, true)
    }
}
