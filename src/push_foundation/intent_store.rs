//! Attested business-intent storage. This module selects no default database and has no sink.

use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::monitor::push_job::{
    derive_decision_id, derive_intent_id, derive_occurrence_id, AudienceId, BusinessDate,
    CompletionOwnerId, IntentId, IntentIdentityMaterial, Namespace, OccurrenceFamily,
    OccurrenceIdentityMaterial, OccurrenceKey, PreparedPush, ReasonCode, RunId, Sha256Digest,
    SourceContractId, SubjectId, UnitId, UtcMicros,
};

use super::migration::{attest_connection, validate_database_path};
use super::{FoundationMigrationError, FoundationSchemaMigration};

const BUSY_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum IntentStoreError {
    #[error(transparent)]
    Foundation(#[from] FoundationMigrationError),
    #[error("business database must already exist before opening intent storage")]
    DatabaseMissing,
    #[error("cannot open business intent database")]
    DatabaseOpenFailed,
    #[error("required SQLite connection safeguards are unavailable")]
    ConnectionSafeguardFailed,
    #[error("invalid initial intent: {check}")]
    InvalidInitialIntent { check: &'static str },
    #[error("immutable material conflicts with an existing intent")]
    ImmutableConflict { intent_id: String },
    #[error("business intent storage operation failed: {operation}")]
    StorageFailed { operation: &'static str },
    #[error("business intent persisted fact failed integrity check: {check}")]
    IntegrityFailed { check: &'static str },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitialDecisionKind {
    Ready,
    NoData,
    Disabled,
}

impl InitialDecisionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::NoData => "NoData",
            Self::Disabled => "Disabled",
        }
    }

    fn parse(value: &str) -> Result<Self, IntentStoreError> {
        match value {
            "Ready" => Ok(Self::Ready),
            "NoData" => Ok(Self::NoData),
            "Disabled" => Ok(Self::Disabled),
            _ => Err(IntentStoreError::IntegrityFailed {
                check: "job_decision_kind",
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntentState {
    PendingDispatch,
    AwaitingAuthority,
    AwaitingFinalizer,
    Completed,
    NotDelivered,
    NoData,
    Disabled,
    ResolutionRequired,
}

impl IntentState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PendingDispatch => "PendingDispatch",
            Self::AwaitingAuthority => "AwaitingAuthority",
            Self::AwaitingFinalizer => "AwaitingFinalizer",
            Self::Completed => "Completed",
            Self::NotDelivered => "NotDelivered",
            Self::NoData => "NoData",
            Self::Disabled => "Disabled",
            Self::ResolutionRequired => "ResolutionRequired",
        }
    }

    fn parse(value: &str) -> Result<Self, IntentStoreError> {
        match value {
            "PendingDispatch" => Ok(Self::PendingDispatch),
            "AwaitingAuthority" => Ok(Self::AwaitingAuthority),
            "AwaitingFinalizer" => Ok(Self::AwaitingFinalizer),
            "Completed" => Ok(Self::Completed),
            "NotDelivered" => Ok(Self::NotDelivered),
            "NoData" => Ok(Self::NoData),
            "Disabled" => Ok(Self::Disabled),
            "ResolutionRequired" => Ok(Self::ResolutionRequired),
            _ => Err(IntentStoreError::IntegrityFailed { check: "state" }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialIntentIdentity {
    namespace: Namespace,
    unit_id: UnitId,
    occurrence: OccurrenceIdentityMaterial,
    completion_owner: CompletionOwnerId,
    source_contract_id: SourceContractId,
    subject: SubjectId,
    audience: AudienceId,
}

impl InitialIntentIdentity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        namespace: Namespace,
        unit_id: UnitId,
        occurrence: OccurrenceIdentityMaterial,
        completion_owner: CompletionOwnerId,
        source_contract_id: SourceContractId,
        subject: SubjectId,
        audience: AudienceId,
    ) -> Self {
        Self {
            namespace,
            unit_id,
            occurrence,
            completion_owner,
            source_contract_id,
            subject,
            audience,
        }
    }

    fn intent_id(&self) -> IntentId {
        derive_intent_id(&IntentIdentityMaterial::new(
            self.namespace.clone(),
            self.unit_id.clone(),
            self.completion_owner.clone(),
            self.source_contract_id.clone(),
            derive_occurrence_id(&self.occurrence),
            self.subject.clone(),
            self.audience.clone(),
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialIntentDraft {
    intent_id: IntentId,
    decision_kind: InitialDecisionKind,
    namespace: String,
    unit_id: String,
    occurrence_family: String,
    occurrence_key: String,
    completion_owner: String,
    source_contract_id: String,
    subject: String,
    audience: String,
    durable_decision_id: String,
    business_date: String,
    prepared_push_bytes: Option<Vec<u8>>,
    rendered_bytes: Option<Vec<u8>>,
    payload_sha256: Option<Sha256Digest>,
    rendered_sha256: Option<Sha256Digest>,
    evidence_sha256: Sha256Digest,
    template_sha256: Sha256Digest,
    source_contract_sha256: Sha256Digest,
    state: IntentState,
    reason: ReasonCode,
    created_at: UtcMicros,
}

impl InitialIntentDraft {
    pub fn ready(
        identity: InitialIntentIdentity,
        prepared: &PreparedPush,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        created_at: UtcMicros,
    ) -> Result<Self, IntentStoreError> {
        let intent_id = identity.intent_id();
        let occurrence = derive_occurrence_id(&identity.occurrence);
        if prepared.intent_id() != &intent_id
            || prepared.decision_id() != &derive_decision_id(&intent_id)
            || prepared.unit_id() != &identity.unit_id
            || prepared.occurrence() != &occurrence
            || prepared.subject() != &identity.subject
            || prepared.source_binding().source_contract_id() != &identity.source_contract_id
        {
            return Err(IntentStoreError::InvalidInitialIntent {
                check: "prepared_push_identity_binding",
            });
        }
        if prepared.rendered_bytes().is_empty() {
            return Err(IntentStoreError::InvalidInitialIntent {
                check: "rendered_bytes_non_empty",
            });
        }
        let prepared_snapshot = prepared.canonical_snapshot_bytes();
        let rendered_bytes = prepared.rendered_bytes().as_bytes().to_vec();
        Ok(Self::from_identity(
            identity,
            InitialDecisionKind::Ready,
            Some(prepared_snapshot.as_bytes().to_vec()),
            Some(rendered_bytes),
            Some(prepared_snapshot.sha256().clone()),
            Some(prepared.rendered_sha256().clone()),
            prepared.source_binding().evidence_fingerprint().clone(),
            template_sha256,
            source_contract_sha256,
            IntentState::PendingDispatch,
            ReasonCode::IntentCreated,
            created_at,
        ))
    }

    pub fn no_data(
        identity: InitialIntentIdentity,
        evidence_sha256: Sha256Digest,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        created_at: UtcMicros,
    ) -> Self {
        Self::from_identity(
            identity,
            InitialDecisionKind::NoData,
            None,
            None,
            None,
            None,
            evidence_sha256,
            template_sha256,
            source_contract_sha256,
            IntentState::NoData,
            ReasonCode::IntentNoData,
            created_at,
        )
    }

    pub fn disabled(
        identity: InitialIntentIdentity,
        evidence_sha256: Sha256Digest,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        created_at: UtcMicros,
    ) -> Self {
        Self::from_identity(
            identity,
            InitialDecisionKind::Disabled,
            None,
            None,
            None,
            None,
            evidence_sha256,
            template_sha256,
            source_contract_sha256,
            IntentState::Disabled,
            ReasonCode::PolicyDisabled,
            created_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_identity(
        identity: InitialIntentIdentity,
        decision_kind: InitialDecisionKind,
        prepared_push_bytes: Option<Vec<u8>>,
        rendered_bytes: Option<Vec<u8>>,
        payload_sha256: Option<Sha256Digest>,
        rendered_sha256: Option<Sha256Digest>,
        evidence_sha256: Sha256Digest,
        template_sha256: Sha256Digest,
        source_contract_sha256: Sha256Digest,
        state: IntentState,
        reason: ReasonCode,
        created_at: UtcMicros,
    ) -> Self {
        let intent_id = identity.intent_id();
        let durable_decision_id = derive_decision_id(&intent_id).as_str().to_owned();
        Self {
            intent_id,
            decision_kind,
            namespace: namespace_storage(&identity.namespace),
            unit_id: identity.unit_id.as_str().to_owned(),
            occurrence_family: identity.occurrence.occurrence_family().as_str().to_owned(),
            occurrence_key: identity.occurrence.occurrence_key().as_str().to_owned(),
            completion_owner: identity.completion_owner.as_str().to_owned(),
            source_contract_id: identity.source_contract_id.as_str().to_owned(),
            subject: subject_storage(&identity.subject),
            audience: identity.audience.as_str().to_owned(),
            durable_decision_id,
            business_date: identity.occurrence.business_date().as_str().to_owned(),
            prepared_push_bytes,
            rendered_bytes,
            payload_sha256,
            rendered_sha256,
            evidence_sha256,
            template_sha256,
            source_contract_sha256,
            state,
            reason,
            created_at,
        }
    }

    pub fn intent_id(&self) -> &IntentId {
        &self.intent_id
    }

    fn initial_snapshot(&self) -> IntentSnapshot {
        IntentSnapshot {
            intent_id: self.intent_id.as_str().to_owned(),
            decision_kind: self.decision_kind,
            namespace: self.namespace.clone(),
            unit_id: self.unit_id.clone(),
            occurrence_family: self.occurrence_family.clone(),
            occurrence_key: self.occurrence_key.clone(),
            completion_owner: self.completion_owner.clone(),
            source_contract_id: self.source_contract_id.clone(),
            subject: self.subject.clone(),
            audience: self.audience.clone(),
            durable_decision_id: self.durable_decision_id.clone(),
            business_date: self.business_date.clone(),
            prepared_push_bytes: self.prepared_push_bytes.clone(),
            rendered_bytes: self.rendered_bytes.clone(),
            payload_sha256: self.payload_sha256.clone(),
            rendered_sha256: self.rendered_sha256.clone(),
            evidence_sha256: self.evidence_sha256.clone(),
            template_sha256: self.template_sha256.clone(),
            source_contract_sha256: self.source_contract_sha256.clone(),
            state: self.state,
            previous_state: None,
            reason: self.reason,
            lease_owner: None,
            lease_until: None,
            lease_generation: 0,
            version: 0,
            created_at: self.created_at,
            updated_at: self.created_at,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntentSnapshot {
    intent_id: String,
    decision_kind: InitialDecisionKind,
    namespace: String,
    unit_id: String,
    occurrence_family: String,
    occurrence_key: String,
    completion_owner: String,
    source_contract_id: String,
    subject: String,
    audience: String,
    durable_decision_id: String,
    business_date: String,
    prepared_push_bytes: Option<Vec<u8>>,
    rendered_bytes: Option<Vec<u8>>,
    payload_sha256: Option<Sha256Digest>,
    rendered_sha256: Option<Sha256Digest>,
    evidence_sha256: Sha256Digest,
    template_sha256: Sha256Digest,
    source_contract_sha256: Sha256Digest,
    state: IntentState,
    previous_state: Option<IntentState>,
    reason: ReasonCode,
    lease_owner: Option<String>,
    lease_until: Option<UtcMicros>,
    lease_generation: u64,
    version: u64,
    created_at: UtcMicros,
    updated_at: UtcMicros,
}

impl IntentSnapshot {
    pub fn intent_id(&self) -> &str {
        &self.intent_id
    }
    pub fn decision_kind(&self) -> InitialDecisionKind {
        self.decision_kind
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    pub fn subject(&self) -> &str {
        &self.subject
    }
    pub fn prepared_push_bytes(&self) -> Option<&[u8]> {
        self.prepared_push_bytes.as_deref()
    }
    pub fn rendered_bytes(&self) -> Option<&[u8]> {
        self.rendered_bytes.as_deref()
    }
    pub fn payload_sha256(&self) -> Option<&Sha256Digest> {
        self.payload_sha256.as_ref()
    }
    pub fn rendered_sha256(&self) -> Option<&Sha256Digest> {
        self.rendered_sha256.as_ref()
    }
    pub fn state(&self) -> IntentState {
        self.state
    }
    pub fn reason(&self) -> ReasonCode {
        self.reason
    }
    pub fn lease_generation(&self) -> u64 {
        self.lease_generation
    }
    pub fn version(&self) -> u64 {
        self.version
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InitialIntentOutcome {
    Inserted(IntentSnapshot),
    ExistingIdentical(IntentSnapshot),
}

impl InitialIntentOutcome {
    pub fn snapshot(&self) -> &IntentSnapshot {
        match self {
            Self::Inserted(snapshot) | Self::ExistingIdentical(snapshot) => snapshot,
        }
    }
}

pub struct BusinessIntentStore {
    connection: Connection,
}

impl BusinessIntentStore {
    pub fn open(database: &Path) -> Result<Self, IntentStoreError> {
        validate_database_path(database)?;
        match fs::symlink_metadata(database) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err(IntentStoreError::DatabaseOpenFailed),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(IntentStoreError::DatabaseMissing)
            }
            Err(_) => return Err(IntentStoreError::DatabaseOpenFailed),
        }
        let canonical_database =
            fs::canonicalize(database).map_err(|_| IntentStoreError::DatabaseOpenFailed)?;
        let connection = Connection::open_with_flags(
            canonical_database,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| IntentStoreError::DatabaseOpenFailed)?;
        let migration = FoundationSchemaMigration::bundled()?;
        attest_connection(&connection, migration.ddl_sha256())?;
        connection
            .execute_batch(
                "PRAGMA query_only=OFF; PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON;",
            )
            .map_err(|_| IntentStoreError::ConnectionSafeguardFailed)?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|_| IntentStoreError::ConnectionSafeguardFailed)?;
        for (pragma, expected) in [
            ("PRAGMA query_only", 0_i64),
            ("PRAGMA foreign_keys", 1_i64),
            ("PRAGMA recursive_triggers", 1_i64),
        ] {
            let actual: i64 = connection
                .query_row(pragma, [], |row| row.get(0))
                .map_err(|_| IntentStoreError::ConnectionSafeguardFailed)?;
            if actual != expected {
                return Err(IntentStoreError::ConnectionSafeguardFailed);
            }
        }
        Ok(Self { connection })
    }

    pub fn record_initial(
        &mut self,
        draft: &InitialIntentDraft,
    ) -> Result<InitialIntentOutcome, IntentStoreError> {
        let expected = draft.initial_snapshot();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "begin_initial",
            })?;
        if let Some(existing) = query_intent(&transaction, draft.intent_id.as_str())? {
            transaction
                .rollback()
                .map_err(|_| IntentStoreError::StorageFailed {
                    operation: "rollback_initial_read",
                })?;
            return if existing == expected {
                Ok(InitialIntentOutcome::ExistingIdentical(existing))
            } else {
                Err(IntentStoreError::ImmutableConflict {
                    intent_id: draft.intent_id.as_str().to_owned(),
                })
            };
        }

        transaction
            .execute(
                "INSERT INTO push_intents(\
                    intent_id,job_decision_kind,namespace,unit_id,occurrence_family,occurrence_key,\
                    completion_owner,source_contract_id,subject,audience,durable_decision_id,\
                    business_date,prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256,\
                    evidence_sha256,template_sha256,source_contract_sha256,state,previous_state,reason,\
                    lease_owner,lease_until,lease_generation,version,created_at,updated_at\
                 ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    draft.intent_id.as_str(),
                    draft.decision_kind.as_str(),
                    draft.namespace,
                    draft.unit_id,
                    draft.occurrence_family,
                    draft.occurrence_key,
                    draft.completion_owner,
                    draft.source_contract_id,
                    draft.subject,
                    draft.audience,
                    draft.durable_decision_id,
                    draft.business_date,
                    draft.prepared_push_bytes,
                    draft.rendered_bytes,
                    draft.payload_sha256.as_ref().map(Sha256Digest::as_str),
                    draft.rendered_sha256.as_ref().map(Sha256Digest::as_str),
                    draft.evidence_sha256.as_str(),
                    draft.template_sha256.as_str(),
                    draft.source_contract_sha256.as_str(),
                    draft.state.as_str(),
                    Option::<&str>::None,
                    draft.reason.as_str(),
                    Option::<&str>::None,
                    Option::<i64>::None,
                    0_i64,
                    0_i64,
                    draft.created_at.get(),
                    draft.created_at.get(),
                ],
            )
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "insert_initial",
            })?;
        transaction
            .commit()
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "commit_initial",
            })?;

        let persisted =
            self.inspect(&draft.intent_id)?
                .ok_or(IntentStoreError::IntegrityFailed {
                    check: "initial_post_commit_missing",
                })?;
        if persisted != expected {
            return Err(IntentStoreError::IntegrityFailed {
                check: "initial_post_commit_mismatch",
            });
        }
        Ok(InitialIntentOutcome::Inserted(persisted))
    }

    pub fn inspect(
        &self,
        intent_id: &IntentId,
    ) -> Result<Option<IntentSnapshot>, IntentStoreError> {
        query_intent(&self.connection, intent_id.as_str())
    }

    #[cfg(test)]
    pub(crate) fn intent_count(&self) -> Result<u64, IntentStoreError> {
        let count: i64 = self
            .connection
            .query_row("SELECT count(*) FROM push_intents", [], |row| row.get(0))
            .map_err(|_| IntentStoreError::StorageFailed {
                operation: "count_intents",
            })?;
        u64::try_from(count).map_err(|_| IntentStoreError::IntegrityFailed {
            check: "intent_count",
        })
    }
}

fn query_intent(
    connection: &Connection,
    intent_id: &str,
) -> Result<Option<IntentSnapshot>, IntentStoreError> {
    let raw = connection
        .query_row(
            "SELECT intent_id,job_decision_kind,namespace,unit_id,occurrence_family,occurrence_key,\
                    completion_owner,source_contract_id,subject,audience,durable_decision_id,\
                    business_date,prepared_push_bytes,rendered_bytes,payload_sha256,rendered_sha256,\
                    evidence_sha256,template_sha256,source_contract_sha256,state,previous_state,reason,\
                    lease_owner,lease_until,lease_generation,version,created_at,updated_at \
             FROM push_intents WHERE intent_id=?",
            [intent_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, Option<Vec<u8>>>(12)?,
                    row.get::<_, Option<Vec<u8>>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, String>(17)?,
                    row.get::<_, String>(18)?,
                    row.get::<_, String>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, String>(21)?,
                    row.get::<_, Option<String>>(22)?,
                    row.get::<_, Option<i64>>(23)?,
                    row.get::<_, i64>(24)?,
                    row.get::<_, i64>(25)?,
                    row.get::<_, i64>(26)?,
                    row.get::<_, i64>(27)?,
                ))
            },
        )
        .optional()
        .map_err(|_| IntentStoreError::StorageFailed {
            operation: "read_intent",
        })?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let snapshot = IntentSnapshot {
        intent_id: raw.0,
        decision_kind: InitialDecisionKind::parse(&raw.1)?,
        namespace: raw.2,
        unit_id: raw.3,
        occurrence_family: raw.4,
        occurrence_key: raw.5,
        completion_owner: raw.6,
        source_contract_id: raw.7,
        subject: raw.8,
        audience: raw.9,
        durable_decision_id: raw.10,
        business_date: raw.11,
        prepared_push_bytes: raw.12,
        rendered_bytes: raw.13,
        payload_sha256: parse_optional_digest("payload_sha256", raw.14)?,
        rendered_sha256: parse_optional_digest("rendered_sha256", raw.15)?,
        evidence_sha256: parse_digest("evidence_sha256", &raw.16)?,
        template_sha256: parse_digest("template_sha256", &raw.17)?,
        source_contract_sha256: parse_digest("source_contract_sha256", &raw.18)?,
        state: IntentState::parse(&raw.19)?,
        previous_state: raw.20.as_deref().map(IntentState::parse).transpose()?,
        reason: ReasonCode::try_from(raw.21.as_str()).map_err(|_| {
            IntentStoreError::IntegrityFailed {
                check: "reason_code",
            }
        })?,
        lease_owner: raw.22,
        lease_until: raw.23.map(parse_micros).transpose()?,
        lease_generation: parse_u64("lease_generation", raw.24)?,
        version: parse_u64("version", raw.25)?,
        created_at: parse_micros(raw.26)?,
        updated_at: parse_micros(raw.27)?,
    };
    verify_snapshot(&snapshot)?;
    Ok(Some(snapshot))
}

fn verify_snapshot(snapshot: &IntentSnapshot) -> Result<(), IntentStoreError> {
    let namespace = parse_namespace(&snapshot.namespace)?;
    let subject = parse_subject(&snapshot.subject)?;
    let occurrence = OccurrenceIdentityMaterial::new(
        BusinessDate::parse(&snapshot.business_date).map_err(|_| integrity("business_date"))?,
        OccurrenceFamily::try_new(snapshot.occurrence_family.clone())
            .map_err(|_| integrity("occurrence_family"))?,
        OccurrenceKey::try_new(snapshot.occurrence_key.clone())
            .map_err(|_| integrity("occurrence_key"))?,
    );
    let derived = derive_intent_id(&IntentIdentityMaterial::new(
        namespace,
        UnitId::try_new(snapshot.unit_id.clone()).map_err(|_| integrity("unit_id"))?,
        CompletionOwnerId::try_new(snapshot.completion_owner.clone())
            .map_err(|_| integrity("completion_owner"))?,
        SourceContractId::try_new(snapshot.source_contract_id.clone())
            .map_err(|_| integrity("source_contract_id"))?,
        derive_occurrence_id(&occurrence),
        subject,
        AudienceId::try_new(snapshot.audience.clone()).map_err(|_| integrity("audience"))?,
    ));
    if derived.as_str() != snapshot.intent_id
        || derive_decision_id(&derived).as_str() != snapshot.durable_decision_id
    {
        return Err(integrity("intent_identity_binding"));
    }
    match snapshot.decision_kind {
        InitialDecisionKind::Ready => {
            let prepared = snapshot
                .prepared_push_bytes
                .as_deref()
                .ok_or_else(|| integrity("ready_payload_group"))?;
            let rendered = snapshot
                .rendered_bytes
                .as_deref()
                .ok_or_else(|| integrity("ready_payload_group"))?;
            if prepared.is_empty()
                || rendered.is_empty()
                || snapshot.payload_sha256.as_ref() != Some(&digest(prepared))
                || snapshot.rendered_sha256.as_ref() != Some(&digest(rendered))
            {
                return Err(integrity("ready_payload_binding"));
            }
        }
        InitialDecisionKind::NoData | InitialDecisionKind::Disabled => {
            if snapshot.prepared_push_bytes.is_some()
                || snapshot.rendered_bytes.is_some()
                || snapshot.payload_sha256.is_some()
                || snapshot.rendered_sha256.is_some()
            {
                return Err(integrity("non_send_payload_group"));
            }
        }
    }
    if snapshot.updated_at < snapshot.created_at {
        return Err(integrity("intent_time_order"));
    }
    Ok(())
}

fn namespace_storage(namespace: &Namespace) -> String {
    match namespace {
        Namespace::Production => "Production".to_owned(),
        Namespace::Test { run_id } => format!("Test:{}", run_id.as_str()),
    }
}

fn subject_storage(subject: &SubjectId) -> String {
    match subject {
        SubjectId::Global => "Global".to_owned(),
        SubjectId::Entity(value) => format!("Entity:{}", value.as_str()),
    }
}

fn parse_namespace(value: &str) -> Result<Namespace, IntentStoreError> {
    if value == "Production" {
        return Ok(Namespace::Production);
    }
    value
        .strip_prefix("Test:")
        .ok_or_else(|| integrity("namespace"))
        .and_then(|run_id| {
            RunId::try_new(run_id.to_owned())
                .map(Namespace::test)
                .map_err(|_| integrity("namespace"))
        })
}

fn parse_subject(value: &str) -> Result<SubjectId, IntentStoreError> {
    if value == "Global" {
        return Ok(SubjectId::Global);
    }
    value
        .strip_prefix("Entity:")
        .ok_or_else(|| integrity("subject"))
        .and_then(|subject| SubjectId::entity(subject.to_owned()).map_err(|_| integrity("subject")))
}

fn parse_digest(field: &'static str, value: &str) -> Result<Sha256Digest, IntentStoreError> {
    Sha256Digest::parse(field, value).map_err(|_| integrity(field))
}

fn parse_optional_digest(
    field: &'static str,
    value: Option<String>,
) -> Result<Option<Sha256Digest>, IntentStoreError> {
    value
        .as_deref()
        .map(|value| parse_digest(field, value))
        .transpose()
}

fn parse_micros(value: i64) -> Result<UtcMicros, IntentStoreError> {
    UtcMicros::try_new(value).map_err(|_| integrity("utc_micros"))
}

fn parse_u64(check: &'static str, value: i64) -> Result<u64, IntentStoreError> {
    u64::try_from(value).map_err(|_| integrity(check))
}

fn digest(bytes: &[u8]) -> Sha256Digest {
    use sha2::{Digest, Sha256};

    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn integrity(check: &'static str) -> IntentStoreError {
    IntentStoreError::IntegrityFailed { check }
}
