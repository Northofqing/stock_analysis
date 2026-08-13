use super::model::{
    compiled_policy_catalog, has_non_ascii_whitespace, sha256_hex, stable_identity,
    AcceptedSinkResultCanonical, AuthoritativeDeliveryRequest, AuthoritativeSink,
    AuthoritativeSinkResult, AuthorityWatermark, CoordinatorConfig, DecisionState,
    DeliveryDispositionCanonical, DeliveryEnvelope, DurableDeliveryError, ImmutableAppendPort,
    ManualAcceptedDeliveryAuditEvidence, ManualDisposition, ManualResolutionAuthorizationCanonical,
    ManualResolutionCommand, PolicyRow, PrepareOutcome, ReconcileSummary, Result, ResumeOutcome,
    ReviewTerminalReplayAttempt, ReviewTerminalReplayCompletion,
    ReviewTerminalReplayCompletionCanonical, ReviewTerminalReplayCompletionState,
    ReviewTerminalReplayInput, ReviewTerminalReplayStartCanonical, ScheduleHydration,
    ScheduleHydrationState, TaskTransitionCanonical, WindowMode, DAILY_BUDGET_LIMIT,
    MANUAL_ACCEPTED_DELIVERY_AUDIT_DOMAIN,
};
use super::schema::{
    configure_attested_connection, initialize_schema, load_policy, materialize_wal_capability,
    verify_connection_configuration, SCHEMA_VERSION,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CString, OsStr, OsString};
use std::fs::{self, File, Metadata};
use std::mem::ManuallyDrop;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

const AUDIT_KINDS: [&str; 14] = [
    "DecisionStateChanged",
    "LeaseGranted",
    "LeaseHeartbeat",
    "FenceRevoked",
    "RecoveryClassified",
    "SinkResultAuthorityClassified",
    "LateReceiptObserved",
    "BudgetReservationChanged",
    "CooldownReservationChanged",
    "BusinessDateOnceClaimed",
    "DecisionIdentityConflict",
    "ScheduleHydrationApplied",
    "ReviewTerminalReplayStarted",
    "ReviewTerminalReplayCompleted",
];

const REVIEW_TERMINAL_REPLAY_ATTEMPT_DOMAIN: &str = "BR-194-terminal-replay-attempt-v1";

const PIN_O_RDONLY: i32 = 0;
const PIN_O_RDWR: i32 = 2;
#[cfg(target_os = "linux")]
const PIN_O_NOFOLLOW: i32 = 0x0002_0000;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const PIN_O_NOFOLLOW: i32 = 0x0000_0100;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "ios"))
))]
const PIN_O_NOFOLLOW: i32 = 0;
#[cfg(target_os = "linux")]
const PIN_O_NONBLOCK: i32 = 0x0000_0800;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const PIN_O_NONBLOCK: i32 = 0x0000_0004;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "ios"))
))]
const PIN_O_NONBLOCK: i32 = 0;
#[cfg(target_os = "linux")]
const PIN_O_CREAT: i32 = 0x0000_0040;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const PIN_O_CREAT: i32 = 0x0000_0200;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "ios"))
))]
const PIN_O_CREAT: i32 = 0;
#[cfg(target_os = "linux")]
const PIN_O_CLOEXEC: i32 = 0x0008_0000;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const PIN_O_CLOEXEC: i32 = 0x0100_0000;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "ios"))
))]
const PIN_O_CLOEXEC: i32 = 0;
const BAD_FILE_DESCRIPTOR_OS_ERROR: i32 = 9;
const NO_SUCH_FILE_OS_ERROR: i32 = 2;
// Defense-in-depth raw-fd ABA sentinel. SQLite's Unix main-db locks occupy
// PENDING_BYTE=0x4000_0000 and the immediately following reserved/shared
// bytes; WAL-index locks are small offsets in the SHM object. The BR-192 OFD
// marker domain begins at 1 TiB and assigns one byte per attested SQLite OFD,
// so it cannot overlap SQLite's own lock ranges.
//
// Supported-target evidence:
// - Linux fcntl.h: F_OFD_GETLK=36 / F_OFD_SETLK=37.
// - macOS SDK sys/fcntl.h: F_OFD_SETLK=90 / F_OFD_GETLK=92; `man 2 fcntl`
//   documents automatic unlock when the last descriptor for that OFD closes.
const OFD_LOCK_MARKER_BASE: i64 = 1_i64 << 40;
const OFD_LOCK_MARKER_LENGTH: i64 = 1;
const OFD_LOCK_MARKER_SPAN: u64 = (1_u64 << 61) - (1_u64 << 40);
const SEEK_SET_FROM_START: i16 = 0;
#[cfg(target_os = "linux")]
const F_OFD_GETLK_COMMAND: i32 = 36;
#[cfg(target_os = "linux")]
const F_OFD_SETLK_COMMAND: i32 = 37;
#[cfg(target_os = "linux")]
const F_WRLCK_TYPE: i16 = 1;
#[cfg(all(test, target_os = "linux"))]
const F_UNLCK_TYPE: i16 = 2;
#[cfg(target_os = "macos")]
const F_OFD_SETLK_COMMAND: i32 = 90;
#[cfg(target_os = "macos")]
const F_OFD_GETLK_COMMAND: i32 = 92;
#[cfg(target_os = "macos")]
const F_WRLCK_TYPE: i16 = 3;
#[cfg(all(test, target_os = "macos"))]
const F_UNLCK_TYPE: i16 = 2;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
const F_OFD_SETLK_COMMAND: i32 = 0;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
const F_OFD_GETLK_COMMAND: i32 = 0;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
const F_WRLCK_TYPE: i16 = 0;
#[cfg(all(test, unix, not(any(target_os = "linux", target_os = "macos"))))]
const F_UNLCK_TYPE: i16 = 0;

unsafe extern "C" {
    fn openat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn fcntl(descriptor: i32, command: i32, ...) -> i32;
    fn geteuid() -> u32;
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct OpenFileDescriptionLock {
    l_type: i16,
    l_whence: i16,
    l_start: i64,
    l_len: i64,
    l_pid: i32,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct OpenFileDescriptionLock {
    l_start: i64,
    l_len: i64,
    l_pid: i32,
    l_type: i16,
    l_whence: i16,
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
#[repr(C)]
struct OpenFileDescriptionLock {
    l_type: i16,
    l_whence: i16,
    l_start: i64,
    l_len: i64,
    l_pid: i32,
}

pub struct DurableDeliveryCoordinator {
    // This Arc is deliberately private: no API may expose rusqlite raw
    // handles, close/dup2 its descriptors, or change journal mode. Holding the
    // exact Connection Arc plus its mutex makes normal safe-code fd ABA
    // unreachable; OFD markers below are defense-in-depth against accidental
    // same-process raw-fd misuse. Compromised arbitrary code under the same UID
    // remains outside this isolation boundary.
    connection: Option<Arc<Mutex<Connection>>>,
    database_binding: Option<PinnedDatabaseBinding>,
    config: CoordinatorConfig,
    #[cfg(test)]
    database_operation_test_hook: Mutex<Option<DatabaseOperationTestHook>>,
    #[cfg(test)]
    delivered_reconcile_test_hook: Mutex<Option<DeliveredReconcileTestHook>>,
    #[cfg(test)]
    delivered_precommit_test_fault: Mutex<Option<DeliveredPrecommitTestFault>>,
    #[cfg(test)]
    operation_postvalidation_test_fault: Mutex<Option<OperationPostvalidationTestFault>>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FileObjectIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
}

impl FileObjectIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            uid: metadata.uid(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum SqliteObjectRole {
    Main,
    Wal,
    Shm,
}

impl SqliteObjectRole {
    fn label(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Wal => "wal",
            Self::Shm => "shm",
        }
    }
}

struct PinnedDatabaseNamespace {
    directory_chain: PinnedDirectoryChain,
    main_anchor: Option<File>,
    leaf: OsString,
    main_identity: FileObjectIdentity,
    sqlite_route: PathBuf,
}

struct PinnedDirectoryComponent {
    name: OsString,
    anchor: File,
    identity: FileObjectIdentity,
    retained_link_count: Mutex<u64>,
}

struct PinnedDirectoryChain {
    filesystem_root: File,
    filesystem_root_identity: FileObjectIdentity,
    filesystem_root_link_count: Mutex<u64>,
    components: Vec<PinnedDirectoryComponent>,
}

struct PinnedSqliteObject {
    role: SqliteObjectRole,
    leaf: OsString,
    namespace_anchor: File,
    descriptor_attestation: SqliteDescriptorAttestation,
    identity: FileObjectIdentity,
}

struct PinnedDatabaseBinding {
    directory_chain: PinnedDirectoryChain,
    objects: [PinnedSqliteObject; 3],
}

struct ProcessDescriptorSnapshot {
    descriptors: BTreeMap<RawFd, FileObjectIdentity>,
}

#[cfg(test)]
pub(crate) enum ProcessDescriptorSnapshotTestFault {
    EntryError,
    AmbiguityError,
    InjectAmbiguousDescriptor { absolute_path: PathBuf },
}

#[cfg(test)]
struct ProcessDescriptorSnapshotTestState {
    remaining_captures: usize,
    fault: Option<ProcessDescriptorSnapshotTestFault>,
    retained_descriptors: Vec<File>,
}

#[cfg(test)]
pub(crate) struct ProcessDescriptorSnapshotTestGuard;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DatabaseBootstrapTestPhase {
    BeforeOpenFileDescriptionCapabilityProbe,
    AfterWalMaterializationBeforeMainReattestation,
    AfterMainReattestationBeforeSidecarAttestation,
    AfterSchemaSqlBeforeCommitValidation,
    AfterFinalParentSyncBeforeSuccessValidation,
}

#[cfg(test)]
struct DatabaseBootstrapTestHook {
    phase: DatabaseBootstrapTestPhase,
    callback: Box<dyn FnOnce() -> Result<()> + 'static>,
}

#[cfg(test)]
pub(crate) struct DatabaseBootstrapTestGuard;

#[cfg(test)]
pub(crate) struct TransactionControlTestGuard;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SharedShmKey {
    parent_identity: FileObjectIdentity,
    leaf: OsString,
}

struct DirectShmNode {
    generation: u64,
    identity: FileObjectIdentity,
    sqlite_descriptor: RawFd,
    open_file_description_proof: OpenFileDescriptionProof,
    connection_lifetimes: Mutex<Vec<Weak<Mutex<Connection>>>>,
}

enum SqliteDescriptorAttestation {
    Direct {
        sqlite_descriptor: RawFd,
        open_file_description_proof: OpenFileDescriptionProof,
    },
    DirectShm {
        node: Arc<DirectShmNode>,
    },
    ProcessSharedShm {
        node: Arc<DirectShmNode>,
    },
}

pub(crate) struct OpenFileDescriptionProof {
    marker_start: i64,
    ownership_identity: String,
}

struct AttestedOperationLease<'a> {
    _open_and_operation_guard: MutexGuard<'a, ()>,
    _shm_connection_lifetime: Arc<Mutex<Connection>>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DatabaseOperationTestPhase {
    AfterPreValidationBeforeSql,
    AfterSqlBeforePreCommitValidation,
}

#[cfg(test)]
struct DatabaseOperationTestHook {
    phase: DatabaseOperationTestPhase,
    callback: Box<dyn FnOnce() -> Result<()> + Send + 'static>,
}

#[cfg(test)]
struct DeliveredReconcileTestHook {
    callback: Box<dyn FnOnce() -> Result<()> + Send + 'static>,
}

#[cfg(test)]
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeliveredPrecommitTestFault {
    AuthoritativeDispositionSemanticBinding,
    AcceptedSinkResultReceiptBinding,
    TaskTransitionSemanticBinding,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationPostvalidationTestFault {
    ImmutableAuditOutboxRef,
    DeliveryDispositionRef,
    TaskTransitionRef,
    ManualResolutionRef,
    SinkDeliveryAuditRef,
    TaskHydrationState,
}

#[derive(Clone, Debug)]
struct StoredDecision {
    decision_identity: String,
    state: DecisionState,
    envelope_canonical: Vec<u8>,
    envelope_sha256: String,
    reservation_generation: i64,
    current_budget_reservation_identity: Option<String>,
    current_cooldown_reservation_identity: Option<String>,
    current_attempt_identity: Option<String>,
    current_disposition_identity: Option<String>,
    fence_generation: i64,
    retry_authorized: bool,
    task_binding_present: bool,
}

#[derive(Clone, Debug)]
struct StoredDispositionEvidence {
    disposition_identity: String,
    attempt_identity: Option<String>,
    resolution_identity: Option<String>,
    denial_identity: Option<String>,
    disposition: String,
    canonical: Vec<u8>,
    sha256: String,
    append_state: String,
    immutable_audit_ref: Option<String>,
    created_at: String,
}

#[derive(Clone, Debug)]
struct StoredAcceptedSinkEvidence {
    result_event_identity: String,
    observed_at: String,
    fence_token: i64,
    authoritative_for_state: bool,
    late_after_fence: bool,
    authority_audit_identity: String,
    late_receipt_audit_identity: Option<String>,
    canonical: Vec<u8>,
    sha256: String,
    channel: Option<String>,
    provider: Option<String>,
    message_id: Option<String>,
    platform_message_id: Option<String>,
    accepted_at: Option<String>,
    latency_ms: Option<i64>,
    frozen_delivery_audit_canonical: Option<Vec<u8>>,
    frozen_delivery_audit_sha256: Option<String>,
    delivery_audit_ref: Option<String>,
    attempt_state: String,
}

#[derive(Serialize)]
struct SinkAuthorityIdentity<'a> {
    result_event_identity: &'a str,
    attempt_identity: &'a str,
    result_sha256: &'a str,
}

#[derive(Serialize)]
struct DeliveryAuditAuthorityIdentity<'a> {
    result_event_identity: &'a str,
    delivery_audit_ref: &'a str,
    frozen_delivery_audit_sha256: &'a str,
}

#[derive(Clone, Debug)]
struct PendingAppend {
    record_kind: String,
    identity: String,
    canonical: Vec<u8>,
    sha256: String,
    decision_identity: String,
}

#[derive(Clone, Debug)]
pub(crate) struct AttemptLease {
    pub(crate) attempt_identity: String,
    pub(crate) fence_token: i64,
    pub(crate) request: AuthoritativeDeliveryRequest,
}

#[derive(Clone, Debug)]
enum PrepareDenial {
    InvalidPolicy(String),
    InvalidSinkCardinality(usize),
    CooldownConflict(String),
    BusinessDateClaimed(String),
    DailyBudgetFull,
}

impl PrepareDenial {
    fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidPolicy(_) => "invalid_registered_policy_projection",
            Self::InvalidSinkCardinality(_) => "authoritative_sink_cardinality",
            Self::CooldownConflict(_) => "cooldown_conflict",
            Self::BusinessDateClaimed(_) => "business_date_once_claimed",
            Self::DailyBudgetFull => "daily_budget_full",
        }
    }

    fn evidence(&self) -> serde_json::Value {
        match self {
            Self::InvalidPolicy(reason) | Self::CooldownConflict(reason) => {
                json!({"reason": reason})
            }
            Self::InvalidSinkCardinality(count) => json!({"authoritative_sink_count": count}),
            Self::BusinessDateClaimed(original) => {
                json!({"original_decision_identity": original})
            }
            Self::DailyBudgetFull => json!({"daily_budget_limit": DAILY_BUDGET_LIMIT}),
        }
    }
}

impl PinnedDatabaseNamespace {
    fn open_at_repository_root(database_path: &Path, repository_root: &Path) -> Result<Self> {
        ensure_supported_attestation_target()?;
        let database_parent = database_path.parent().ok_or_else(|| {
            DurableDeliveryError::IsolationViolation(
                "durable-delivery database has no retained parent".to_owned(),
            )
        })?;
        let leaf = database_path
            .file_name()
            .ok_or_else(|| {
                DurableDeliveryError::IsolationViolation(
                    "durable-delivery database has no fixed leaf".to_owned(),
                )
            })?
            .to_os_string();
        let directory_chain = PinnedDirectoryChain::open(repository_root, database_parent)?;
        directory_chain.validate()?;
        let parent_anchor = directory_chain.parent_anchor()?;
        // Pre-existing WAL/SHM namespace entries are validated before O_CREAT
        // can materialize the main database. A hostile sidecar therefore
        // fails closed with the main leaf still absent.
        validate_preexisting_sidecars(parent_anchor, &leaf)?;
        let main_anchor = openat_component(
            parent_anchor,
            &leaf,
            PIN_O_RDWR | PIN_O_CREAT,
            "main database",
        )?;
        let main_identity = require_unique_regular_identity(&main_anchor, "main database")?;
        directory_chain.validate()?;
        sync_directory(parent_anchor, "database parent after main open")?;
        Ok(Self {
            directory_chain,
            main_anchor: Some(main_anchor),
            leaf,
            main_identity,
            sqlite_route: repository_root.join(database_path),
        })
    }

    fn sqlite_route(&self) -> &Path {
        &self.sqlite_route
    }

    fn sync_parent_directory(&self, phase: &str) -> Result<()> {
        sync_directory(self.directory_chain.parent_anchor()?, phase)
    }

    fn validate_main_attestation(&self, main: &PinnedSqliteObject) -> Result<()> {
        self.directory_chain.validate()?;
        validate_pinned_object(self.directory_chain.parent_anchor()?, main)?;
        if !matches!(main.role, SqliteObjectRole::Main) || main.identity != self.main_identity {
            return Err(DurableDeliveryError::IsolationViolation(
                "immediate SQLite main attestation differs from the retained main object"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    fn rebind_main_open_file_description_after_wal(
        &self,
        main: &mut PinnedSqliteObject,
        before_wal: &ProcessDescriptorSnapshot,
        after_wal: &ProcessDescriptorSnapshot,
    ) -> Result<()> {
        #[cfg(test)]
        MAIN_REATTESTATION_CALLS.with(|calls| calls.set(calls.get() + 1));
        self.directory_chain.validate()?;
        let parent = self.directory_chain.parent_anchor()?;
        validate_bootstrap_rebind_target(parent, main)?;
        let (original_descriptor, ownership_identity) = match &main.descriptor_attestation {
            SqliteDescriptorAttestation::Direct {
                sqlite_descriptor,
                open_file_description_proof,
            } => (
                *sqlite_descriptor,
                open_file_description_proof.ownership_identity.clone(),
            ),
            SqliteDescriptorAttestation::DirectShm { .. }
            | SqliteDescriptorAttestation::ProcessSharedShm { .. } => {
                return Err(DurableDeliveryError::IsolationViolation(
                    "SQLite main bootstrap rebind requires a direct descriptor".to_owned(),
                ));
            }
        };
        let sqlite_descriptor = select_post_wal_main_descriptor(
            before_wal,
            after_wal,
            main.identity,
            original_descriptor,
            |descriptor| {
                require_unique_regular_descriptor_identity(
                    descriptor,
                    "original rusqlite main after WAL materialization",
                )
            },
        )?;
        let replacement = OpenFileDescriptionProof::install(
            sqlite_descriptor,
            &main.namespace_anchor,
            main.identity,
            main.role.label(),
            &ownership_identity,
        )?;
        match &mut main.descriptor_attestation {
            SqliteDescriptorAttestation::Direct {
                sqlite_descriptor: retained_descriptor,
                open_file_description_proof,
            } => {
                *retained_descriptor = sqlite_descriptor;
                *open_file_description_proof = replacement;
            }
            SqliteDescriptorAttestation::DirectShm { .. }
            | SqliteDescriptorAttestation::ProcessSharedShm { .. } => {
                return Err(DurableDeliveryError::IsolationViolation(
                    "SQLite main bootstrap rebind target changed descriptor type".to_owned(),
                ));
            }
        }
        self.validate_main_attestation(main)
    }

    fn take_main_anchor(&mut self) -> Result<File> {
        self.main_anchor.take().ok_or_else(|| {
            DurableDeliveryError::IsolationViolation(
                "SQLite main namespace anchor was already consumed".to_owned(),
            )
        })
    }

    fn attest_sqlite_connection(
        self,
        connection: &Arc<Mutex<Connection>>,
        main: PinnedSqliteObject,
        before: &ProcessDescriptorSnapshot,
        after: &ProcessDescriptorSnapshot,
        ownership_identity: &str,
    ) -> Result<PinnedDatabaseBinding> {
        let wal_leaf = sqlite_sidecar_leaf(&self.leaf, "-wal");
        let shm_leaf = sqlite_sidecar_leaf(&self.leaf, "-shm");
        // These pins are deliberately opened after the process-fd snapshot so
        // they cannot enter SQLite's descriptor delta.
        let parent_anchor = self.directory_chain.parent_anchor()?;
        let wal_anchor = openat_component(parent_anchor, &wal_leaf, PIN_O_RDWR, "SQLite WAL")?;
        let shm_anchor = openat_component(parent_anchor, &shm_leaf, PIN_O_RDWR, "SQLite SHM")?;
        let wal_identity = require_unique_regular_identity(&wal_anchor, "SQLite WAL")?;
        let shm_identity = require_unique_regular_identity(&shm_anchor, "SQLite SHM")?;
        let identities = BTreeSet::from([self.main_identity, wal_identity, shm_identity]);
        if identities.len() != 3 {
            return Err(DurableDeliveryError::IsolationViolation(
                "SQLite main/WAL/SHM must be three distinct physical files".to_owned(),
            ));
        }
        let objects = [
            main,
            attest_sqlite_object(
                SqliteObjectRole::Wal,
                wal_leaf,
                wal_anchor,
                wal_identity,
                before,
                after,
                ownership_identity,
            )?,
            attest_sqlite_shm_object(
                SharedShmKey {
                    parent_identity: self.directory_chain.parent_identity()?,
                    leaf: shm_leaf.clone(),
                },
                shm_leaf,
                shm_anchor,
                shm_identity,
                (before, after),
                connection,
                ownership_identity,
            )?,
        ];
        let binding = PinnedDatabaseBinding {
            directory_chain: self.directory_chain,
            objects,
        };
        let _lifetime = binding.validate_under_open_lock()?;
        Ok(binding)
    }
}

impl PinnedDirectoryChain {
    fn open(repository_root: &Path, database_parent: &Path) -> Result<Self> {
        if !repository_root.is_absolute() {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "compile-time repository root must be absolute: {}",
                repository_root.display()
            )));
        }
        if database_parent.is_absolute() {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "database parent must be repository-relative: {}",
                database_parent.display()
            )));
        }
        let repository_components = normal_path_components(repository_root, "repository root")?;
        let repository_component_count = repository_components.len();
        let database_components =
            normal_path_components(database_parent, "database parent relative path")?;
        let filesystem_root = File::open("/").map_err(|error| {
            DurableDeliveryError::IsolationViolation(format!(
                "cannot retain filesystem-root capability: {error}"
            ))
        })?;
        let filesystem_root_identity =
            require_trusted_directory_identity(&filesystem_root, "filesystem root", false)?;
        let filesystem_root_link_count =
            require_directory_link_count(&filesystem_root, "filesystem root")?;
        let mut components =
            Vec::with_capacity(repository_component_count + database_components.len());
        for (index, name) in repository_components
            .into_iter()
            .chain(database_components)
            .enumerate()
        {
            let parent = components
                .last()
                .map(|component: &PinnedDirectoryComponent| &component.anchor)
                .unwrap_or(&filesystem_root);
            let anchor =
                openat_component(parent, &name, PIN_O_RDONLY, "database namespace component")?;
            let identity = require_trusted_directory_identity(
                &anchor,
                "database namespace component",
                index + 1 >= repository_component_count,
            )?;
            let retained_link_count =
                require_directory_link_count(&anchor, "database namespace component")?;
            components.push(PinnedDirectoryComponent {
                name,
                anchor,
                identity,
                retained_link_count: Mutex::new(retained_link_count),
            });
        }
        if components.is_empty() {
            return Err(DurableDeliveryError::IsolationViolation(
                "database namespace has no retained parent components".to_owned(),
            ));
        }
        Ok(Self {
            filesystem_root,
            filesystem_root_identity,
            filesystem_root_link_count: Mutex::new(filesystem_root_link_count),
            components,
        })
    }

    fn parent_anchor(&self) -> Result<&File> {
        self.components
            .last()
            .map(|component| &component.anchor)
            .ok_or_else(|| {
                DurableDeliveryError::IsolationViolation(
                    "database namespace has no retained parent".to_owned(),
                )
            })
    }

    fn parent_identity(&self) -> Result<FileObjectIdentity> {
        self.components
            .last()
            .map(|component| component.identity)
            .ok_or_else(|| {
                DurableDeliveryError::IsolationViolation(
                    "database namespace has no retained parent identity".to_owned(),
                )
            })
    }

    fn validate(&self) -> Result<()> {
        // Directory link count is a mutation detector, not identity. A legitimate
        // child-directory mkdir/rmdir can alter it without rebinding this
        // retained chain. We therefore treat link-count drift as informational and
        // only require successful rebind identity validation, then refresh the
        // retained baseline from the observed chain.
        let observed = self.validate_once()?;
        self.refresh_link_count_baselines(&observed)
    }

    fn validate_once(&self) -> Result<Vec<u64>> {
        let current_root =
            require_trusted_directory_identity(&self.filesystem_root, "filesystem root", false)?;
        if current_root != self.filesystem_root_identity {
            return Err(DurableDeliveryError::IsolationViolation(
                "retained filesystem-root capability changed identity".to_owned(),
            ));
        }
        let current_root_link_count =
            require_directory_link_count(&self.filesystem_root, "filesystem root")?;
        let reopened_root = File::open("/").map_err(|error| {
            DurableDeliveryError::IsolationViolation(format!(
                "cannot reopen fixed filesystem root: {error}"
            ))
        })?;
        let reopened_root_identity =
            require_trusted_directory_identity(&reopened_root, "filesystem root", false)?;
        if reopened_root_identity != self.filesystem_root_identity {
            return Err(DurableDeliveryError::IsolationViolation(
                "fixed filesystem root no longer names the retained root identity".to_owned(),
            ));
        }
        let reopened_root_link_count =
            require_directory_link_count(&reopened_root, "filesystem root")?;
        if reopened_root_link_count != current_root_link_count {
            return Err(DurableDeliveryError::IsolationViolation(
                "filesystem-root link count changed during one chain rebind".to_owned(),
            ));
        }

        let mut current = reopened_root;
        let mut link_counts = Vec::with_capacity(self.components.len() + 1);
        link_counts.push(current_root_link_count);
        for component in &self.components {
            let retained_metadata = component.anchor.metadata().map_err(|error| {
                DurableDeliveryError::IsolationViolation(format!(
                    "cannot inspect retained database namespace component: {error}"
                ))
            })?;
            if !retained_metadata.is_dir()
                || FileObjectIdentity::from_metadata(&retained_metadata) != component.identity
            {
                return Err(DurableDeliveryError::IsolationViolation(
                    "retained database namespace component changed identity".to_owned(),
                ));
            }
            require_not_shared_writable(&retained_metadata, "database namespace component")?;
            let retained_link_count = retained_metadata.nlink();
            if retained_link_count == 0 {
                return Err(DurableDeliveryError::IsolationViolation(
                    "retained database namespace component has zero links".to_owned(),
                ));
            }
            let reopened = openat_component(
                &current,
                &component.name,
                PIN_O_RDONLY,
                "fixed database namespace component",
            )?;
            let reopened_identity = require_trusted_directory_identity(
                &reopened,
                "fixed database namespace component",
                false,
            )?;
            if reopened_identity != component.identity {
                return Err(DurableDeliveryError::IsolationViolation(
                    "fixed database namespace ancestor was renamed or replaced".to_owned(),
                ));
            }
            let reopened_link_count =
                require_directory_link_count(&reopened, "fixed database namespace component")?;
            if reopened_link_count != retained_link_count {
                return Err(DurableDeliveryError::IsolationViolation(
                    "database namespace component link count changed during one chain rebind"
                        .to_owned(),
                ));
            }
            link_counts.push(retained_link_count);
            current = reopened;
        }
        Ok(link_counts)
    }

    fn refresh_link_count_baselines(&self, stable_link_counts: &[u64]) -> Result<()> {
        if stable_link_counts.len() != self.components.len() + 1 {
            return Err(DurableDeliveryError::IsolationViolation(
                "complete-chain link-count evidence has the wrong cardinality".to_owned(),
            ));
        }
        let mut root_baseline = self.filesystem_root_link_count.lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "filesystem-root link-count baseline mutex is poisoned".to_owned(),
            )
        })?;
        if *root_baseline != stable_link_counts[0] {
            *root_baseline = stable_link_counts[0];
        }
        for (component, &observed) in self
            .components
            .iter()
            .zip(stable_link_counts.iter().skip(1))
        {
            let mut baseline = component.retained_link_count.lock().map_err(|_| {
                DurableDeliveryError::IsolationViolation(
                    "database namespace link-count baseline mutex is poisoned".to_owned(),
                )
            })?;
            if *baseline != observed {
                *baseline = observed;
            }
        }
        Ok(())
    }
}

impl PinnedDatabaseBinding {
    fn sync_parent_directory(&self, phase: &str) -> Result<()> {
        sync_directory(self.directory_chain.parent_anchor()?, phase)
    }

    fn acquire_operation_lease(&self) -> Result<AttestedOperationLease<'static>> {
        let open_and_operation_guard = sqlite_attestation_open_lock().lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "SQLite descriptor-attestation operation lock is poisoned".to_owned(),
            )
        })?;
        let shm_connection_lifetime = self.validate_under_open_lock()?;
        Ok(AttestedOperationLease {
            _open_and_operation_guard: open_and_operation_guard,
            _shm_connection_lifetime: shm_connection_lifetime,
        })
    }

    fn validate_under_open_lock(&self) -> Result<Arc<Mutex<Connection>>> {
        self.directory_chain.validate()?;
        let parent_anchor = self.directory_chain.parent_anchor()?;
        let mut shm_connection_lifetime = None;
        for object in &self.objects {
            validate_pinned_object(parent_anchor, object)?;
            match &object.descriptor_attestation {
                SqliteDescriptorAttestation::Direct { .. } => {}
                SqliteDescriptorAttestation::DirectShm { node }
                | SqliteDescriptorAttestation::ProcessSharedShm { node } => {
                    let lifetime = node.acquire_connection_lifetime()?;
                    if shm_connection_lifetime.is_none() {
                        shm_connection_lifetime = Some(lifetime);
                    }
                }
            }
        }
        shm_connection_lifetime.ok_or_else(|| {
            DurableDeliveryError::IsolationViolation(
                "SQLite SHM has no live direct/process-shared connection attestation".to_owned(),
            )
        })
    }
}

fn validate_pinned_object(parent: &File, object: &PinnedSqliteObject) -> Result<()> {
    let role = object.role.label();
    let namespace_identity = require_unique_regular_identity(
        &object.namespace_anchor,
        &format!("SQLite {role} namespace anchor"),
    )?;
    let (sqlite_descriptor, node_identity) = match &object.descriptor_attestation {
        SqliteDescriptorAttestation::Direct {
            sqlite_descriptor,
            open_file_description_proof,
        } => {
            open_file_description_proof.validate(
                *sqlite_descriptor,
                &object.namespace_anchor,
                object.identity,
                role,
            )?;
            (*sqlite_descriptor, None)
        }
        SqliteDescriptorAttestation::DirectShm { node }
        | SqliteDescriptorAttestation::ProcessSharedShm { node } => {
            node.validate_identity(object.identity, &object.namespace_anchor)?;
            (node.sqlite_descriptor, Some(node.identity))
        }
    };
    let sqlite_identity = require_unique_regular_descriptor_identity(
        sqlite_descriptor,
        &format!("live rusqlite {role}"),
    )?;
    if namespace_identity != object.identity
        || sqlite_identity != object.identity
        || node_identity.is_some_and(|identity| identity != object.identity)
    {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "retained SQLite {role}/rusqlite descriptor no longer shares the pinned identity"
        )));
    }
    let reopened = openat_component(
        parent,
        &object.leaf,
        PIN_O_RDWR,
        &format!("fixed SQLite {role} leaf"),
    )?;
    let reopened_identity =
        require_unique_regular_identity(&reopened, &format!("fixed SQLite {role} leaf"))?;
    if reopened_identity != object.identity {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "fixed SQLite {role} leaf no longer names the connected inode"
        )));
    }
    Ok(())
}

fn validate_bootstrap_rebind_target(parent: &File, object: &PinnedSqliteObject) -> Result<()> {
    if !matches!(object.role, SqliteObjectRole::Main) {
        return Err(DurableDeliveryError::IsolationViolation(
            "bootstrap OFD rebind is restricted to the SQLite main object".to_owned(),
        ));
    }
    let namespace_identity = require_unique_regular_identity(
        &object.namespace_anchor,
        "SQLite main bootstrap namespace anchor",
    )?;
    let reopened = openat_component(
        parent,
        &object.leaf,
        PIN_O_RDWR,
        "fixed SQLite main leaf before bootstrap OFD rebind",
    )?;
    let reopened_identity =
        require_unique_regular_identity(&reopened, "fixed SQLite main bootstrap leaf")?;
    if namespace_identity != object.identity || reopened_identity != object.identity {
        return Err(DurableDeliveryError::IsolationViolation(
            "SQLite main bootstrap rebind target changed physical identity".to_owned(),
        ));
    }
    Ok(())
}

impl ProcessDescriptorSnapshot {
    fn capture() -> Result<Self> {
        let descriptor_root = process_descriptor_root()?;
        let iterator = fs::read_dir(descriptor_root).map_err(|error| {
            DurableDeliveryError::IsolationViolation(format!(
                "cannot enumerate process descriptors at {}: {error}",
                descriptor_root.display()
            ))
        })?;
        let mut descriptor_numbers = BTreeSet::new();
        for entry in iterator {
            let entry = entry.map_err(|error| {
                DurableDeliveryError::IsolationViolation(format!(
                    "cannot enumerate one process descriptor entry: {error}"
                ))
            })?;
            if let Some(descriptor) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<RawFd>().ok())
            {
                descriptor_numbers.insert(descriptor);
            }
        }
        let mut descriptors = BTreeMap::new();
        for descriptor in descriptor_numbers {
            match descriptor_metadata(descriptor) {
                Ok(metadata) => {
                    descriptors.insert(descriptor, FileObjectIdentity::from_metadata(&metadata));
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        || error.raw_os_error() == Some(BAD_FILE_DESCRIPTOR_OS_ERROR) => {}
                Err(error) => {
                    return Err(DurableDeliveryError::IsolationViolation(format!(
                        "cannot inspect process descriptor {descriptor}: {error}"
                    )))
                }
            }
        }
        #[cfg(test)]
        apply_process_descriptor_snapshot_test_fault(&mut descriptors)?;
        Ok(Self { descriptors })
    }
}

#[cfg(test)]
thread_local! {
    static PROCESS_DESCRIPTOR_SNAPSHOT_TEST_STATE:
        std::cell::RefCell<Option<ProcessDescriptorSnapshotTestState>> =
            const { std::cell::RefCell::new(None) };
    static DATABASE_BOOTSTRAP_TEST_HOOK:
        std::cell::RefCell<Option<DatabaseBootstrapTestHook>> =
            const { std::cell::RefCell::new(None) };
    static TRANSACTION_CONTROL_TEST_FAULT:
        std::cell::Cell<Option<(bool, bool)>> =
            const { std::cell::Cell::new(None) };
    static MAIN_REATTESTATION_CALLS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn main_reattestation_call_count_for_test() -> usize {
    MAIN_REATTESTATION_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn install_compound_commit_rollback_test_fault() -> Result<TransactionControlTestGuard> {
    TRANSACTION_CONTROL_TEST_FAULT.with(|fault| {
        if fault.get().is_some() {
            return Err(DurableDeliveryError::InvalidConfiguration(
                "a transaction-control test fault is already installed".to_owned(),
            ));
        }
        fault.set(Some((true, true)));
        Ok(TransactionControlTestGuard)
    })
}

#[cfg(test)]
fn take_commit_test_fault() -> bool {
    TRANSACTION_CONTROL_TEST_FAULT.with(|fault| {
        let Some((fail_commit, fail_rollback)) = fault.get() else {
            return false;
        };
        fault.set(Some((false, fail_rollback)));
        fail_commit
    })
}

#[cfg(test)]
fn take_rollback_test_fault() -> bool {
    TRANSACTION_CONTROL_TEST_FAULT.with(|fault| {
        let Some((fail_commit, fail_rollback)) = fault.get() else {
            return false;
        };
        fault.set(Some((fail_commit, false)));
        fail_rollback
    })
}

#[cfg(test)]
impl Drop for TransactionControlTestGuard {
    fn drop(&mut self) {
        TRANSACTION_CONTROL_TEST_FAULT.with(|fault| fault.set(None));
    }
}

#[cfg(test)]
pub(crate) fn install_database_bootstrap_test_hook(
    phase: DatabaseBootstrapTestPhase,
    callback: impl FnOnce() -> Result<()> + 'static,
) -> Result<DatabaseBootstrapTestGuard> {
    DATABASE_BOOTSTRAP_TEST_HOOK.with(|hook| {
        let mut hook = hook.borrow_mut();
        if hook.is_some() {
            return Err(DurableDeliveryError::InvalidConfiguration(
                "a database-bootstrap test hook is already installed".to_owned(),
            ));
        }
        *hook = Some(DatabaseBootstrapTestHook {
            phase,
            callback: Box::new(callback),
        });
        Ok(DatabaseBootstrapTestGuard)
    })
}

#[cfg(test)]
fn run_database_bootstrap_test_hook(phase: DatabaseBootstrapTestPhase) -> Result<()> {
    DATABASE_BOOTSTRAP_TEST_HOOK.with(|hook| {
        let callback = {
            let mut hook = hook.borrow_mut();
            match hook.as_ref() {
                Some(installed) if installed.phase == phase => {
                    hook.take().map(|installed| installed.callback)
                }
                _ => None,
            }
        };
        match callback {
            Some(callback) => callback(),
            None => Ok(()),
        }
    })
}

#[cfg(test)]
impl Drop for DatabaseBootstrapTestGuard {
    fn drop(&mut self) {
        DATABASE_BOOTSTRAP_TEST_HOOK.with(|hook| {
            *hook.borrow_mut() = None;
        });
    }
}

#[cfg(test)]
pub(crate) fn install_process_descriptor_snapshot_test_fault(
    remaining_captures: usize,
    fault: ProcessDescriptorSnapshotTestFault,
) -> Result<ProcessDescriptorSnapshotTestGuard> {
    PROCESS_DESCRIPTOR_SNAPSHOT_TEST_STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.is_some() {
            return Err(DurableDeliveryError::InvalidConfiguration(
                "a process-descriptor snapshot test fault is already installed".to_owned(),
            ));
        }
        *state = Some(ProcessDescriptorSnapshotTestState {
            remaining_captures,
            fault: Some(fault),
            retained_descriptors: Vec::new(),
        });
        Ok(ProcessDescriptorSnapshotTestGuard)
    })
}

#[cfg(test)]
fn apply_process_descriptor_snapshot_test_fault(
    descriptors: &mut BTreeMap<RawFd, FileObjectIdentity>,
) -> Result<()> {
    PROCESS_DESCRIPTOR_SNAPSHOT_TEST_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(state) = state.as_mut() else {
            return Ok(());
        };
        if state.remaining_captures > 0 {
            state.remaining_captures -= 1;
            return Ok(());
        }
        let Some(fault) = state.fault.take() else {
            return Ok(());
        };
        match fault {
            ProcessDescriptorSnapshotTestFault::EntryError => {
                Err(DurableDeliveryError::IsolationViolation(
                    "TEST_CODE injected process-descriptor ReadDir entry error".to_owned(),
                ))
            }
            ProcessDescriptorSnapshotTestFault::AmbiguityError => {
                Err(DurableDeliveryError::IsolationViolation(
                    "TEST_CODE injected process-descriptor enumeration ambiguity".to_owned(),
                ))
            }
            ProcessDescriptorSnapshotTestFault::InjectAmbiguousDescriptor { absolute_path } => {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&absolute_path)?;
                let descriptor = file.as_raw_fd();
                let identity = require_unique_regular_identity(
                    &file,
                    "TEST_CODE ambiguous process descriptor",
                )?;
                descriptors.insert(descriptor, identity);
                state.retained_descriptors.push(file);
                Ok(())
            }
        }
    })
}

#[cfg(test)]
impl Drop for ProcessDescriptorSnapshotTestGuard {
    fn drop(&mut self) {
        PROCESS_DESCRIPTOR_SNAPSHOT_TEST_STATE.with(|state| {
            *state.borrow_mut() = None;
        });
    }
}

fn attest_sqlite_object(
    role: SqliteObjectRole,
    leaf: OsString,
    namespace_anchor: File,
    expected_identity: FileObjectIdentity,
    before: &ProcessDescriptorSnapshot,
    after: &ProcessDescriptorSnapshot,
    ownership_identity: &str,
) -> Result<PinnedSqliteObject> {
    let candidates = sqlite_descriptor_delta_candidates(before, after, expected_identity);
    let sqlite_descriptor = match candidates.as_slice() {
        [descriptor] => *descriptor,
        [] => {
            let before_matches = before
                .descriptors
                .iter()
                .filter_map(|(&descriptor, &identity)| {
                    (identity == expected_identity).then_some(descriptor)
                })
                .collect::<Vec<_>>();
            let after_matches = after
                .descriptors
                .iter()
                .filter_map(|(&descriptor, &identity)| {
                    (identity == expected_identity).then_some(descriptor)
                })
                .collect::<Vec<_>>();
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "rusqlite opened no persistent {} descriptor for the pinned SQLite object; before_matches={before_matches:?} after_matches={after_matches:?}",
                role.label(),
            )));
        }
        _ => {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "rusqlite opened {} ambiguous {} descriptors for the pinned SQLite object",
                candidates.len(),
                role.label()
            )))
        }
    };
    let sqlite_identity = require_unique_regular_descriptor_identity(
        sqlite_descriptor,
        &format!("live rusqlite SQLite {}", role.label()),
    )?;
    if sqlite_identity != expected_identity {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "live rusqlite {} descriptor differs from the pinned SQLite object",
            role.label()
        )));
    }
    let open_file_description_proof = OpenFileDescriptionProof::install(
        sqlite_descriptor,
        &namespace_anchor,
        expected_identity,
        role.label(),
        ownership_identity,
    )?;
    Ok(PinnedSqliteObject {
        role,
        leaf,
        namespace_anchor,
        descriptor_attestation: SqliteDescriptorAttestation::Direct {
            sqlite_descriptor,
            open_file_description_proof,
        },
        identity: expected_identity,
    })
}

fn open_connection_with_attestable_main(
    route: &Path,
    expected_identity: FileObjectIdentity,
    _attestation_guard: &std::sync::MutexGuard<'_, ()>,
) -> Result<(
    Connection,
    ProcessDescriptorSnapshot,
    ProcessDescriptorSnapshot,
)> {
    let mut before = ProcessDescriptorSnapshot::capture()?;
    let maximum_attempts = before
        .descriptors
        .values()
        .filter(|&&identity| identity == expected_identity)
        .count()
        .checked_add(1)
        .ok_or_else(|| {
            DurableDeliveryError::IsolationViolation(
                "SQLite main descriptor reuse bound overflowed".to_owned(),
            )
        })?;
    let mut unproved_connections = Vec::new();

    for attempt in 1..=maximum_attempts {
        let connection = Connection::open_with_flags(
            route,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let after = ProcessDescriptorSnapshot::capture()?;
        let candidates = sqlite_descriptor_delta_candidates(&before, &after, expected_identity);
        match candidates.as_slice() {
            [_] => {
                // BR-206: no unproved handle may escape bootstrap. Their
                // destruction remains under the caller-held attestation lock.
                drop(unproved_connections);
                return Ok((connection, before, after));
            }
            [] if attempt < maximum_attempts => {
                // SQLite's Unix VFS can consume an already-open descriptor
                // from its internal reuse pool. Keep this unproved connection
                // alive so the same pooled descriptor cannot satisfy the next
                // open, then retry without issuing SQL or PRAGMA.
                unproved_connections.push(connection);
                before = after;
            }
            [] => {
                let before_matches = before
                    .descriptors
                    .iter()
                    .filter_map(|(&descriptor, &identity)| {
                        (identity == expected_identity).then_some(descriptor)
                    })
                    .collect::<Vec<_>>();
                let after_matches = after
                    .descriptors
                    .iter()
                    .filter_map(|(&descriptor, &identity)| {
                        (identity == expected_identity).then_some(descriptor)
                    })
                    .collect::<Vec<_>>();
                return Err(DurableDeliveryError::IsolationViolation(format!(
                    "rusqlite exhausted {maximum_attempts} evidence-bounded opens without a persistent main descriptor; before_matches={before_matches:?} after_matches={after_matches:?}"
                )));
            }
            _ => {
                return Err(DurableDeliveryError::IsolationViolation(format!(
                    "rusqlite opened {} ambiguous main descriptors for the pinned SQLite object on attempt {attempt} of {maximum_attempts}",
                    candidates.len()
                )));
            }
        }
    }

    Err(DurableDeliveryError::IsolationViolation(
        "SQLite main descriptor acquisition exhausted an unreachable loop state".to_owned(),
    ))
}

fn attest_sqlite_shm_object(
    key: SharedShmKey,
    leaf: OsString,
    namespace_anchor: File,
    expected_identity: FileObjectIdentity,
    descriptor_snapshots: (&ProcessDescriptorSnapshot, &ProcessDescriptorSnapshot),
    connection: &Arc<Mutex<Connection>>,
    ownership_identity: &str,
) -> Result<PinnedSqliteObject> {
    let (before, after) = descriptor_snapshots;
    let candidates = sqlite_descriptor_delta_candidates(before, after, expected_identity);
    match candidates.as_slice() {
        [sqlite_descriptor] => {
            let actual = require_unique_regular_descriptor_identity(
                *sqlite_descriptor,
                "direct live rusqlite SQLite SHM",
            )?;
            if actual != expected_identity {
                return Err(DurableDeliveryError::IsolationViolation(
                    "direct live rusqlite SHM descriptor differs from the pinned SQLite SHM"
                        .to_owned(),
                ));
            }
            let node = Arc::new(DirectShmNode::new(
                *sqlite_descriptor,
                expected_identity,
                &namespace_anchor,
                connection,
                ownership_identity,
            )?);
            register_direct_shm_node(&key, &node, &namespace_anchor)?;
            Ok(PinnedSqliteObject {
                role: SqliteObjectRole::Shm,
                leaf,
                namespace_anchor,
                descriptor_attestation: SqliteDescriptorAttestation::DirectShm { node },
                identity: expected_identity,
            })
        }
        [] => {
            let mut nodes = process_shared_shm_nodes().lock().map_err(|_| {
                DurableDeliveryError::IsolationViolation(
                    "process-shared SQLite SHM proof lock is poisoned".to_owned(),
                )
            })?;
            nodes.retain(|_, candidate| candidate.strong_count() > 0);
            let node = nodes.get(&key).and_then(Weak::upgrade).ok_or_else(|| {
                DurableDeliveryError::IsolationViolation(
                    "rusqlite opened no persistent shm descriptor and no exact live process-shared SHM node exists"
                        .to_owned(),
                )
            })?;
            node.validate_identity(expected_identity, &namespace_anchor)?;
            node.register_connection(connection)?;
            Ok(PinnedSqliteObject {
                role: SqliteObjectRole::Shm,
                leaf,
                namespace_anchor,
                descriptor_attestation: SqliteDescriptorAttestation::ProcessSharedShm { node },
                identity: expected_identity,
            })
        }
        _ => Err(DurableDeliveryError::IsolationViolation(format!(
            "rusqlite opened {} ambiguous shm descriptors for the pinned SQLite object",
            candidates.len()
        ))),
    }
}

fn sqlite_descriptor_delta_candidates(
    before: &ProcessDescriptorSnapshot,
    after: &ProcessDescriptorSnapshot,
    expected_identity: FileObjectIdentity,
) -> Vec<RawFd> {
    after
        .descriptors
        .iter()
        .filter_map(|(&descriptor, &identity)| {
            (before.descriptors.get(&descriptor) != Some(&identity)
                && identity == expected_identity)
                .then_some(descriptor)
        })
        .collect()
}

fn select_post_wal_main_descriptor(
    before: &ProcessDescriptorSnapshot,
    after: &ProcessDescriptorSnapshot,
    expected_identity: FileObjectIdentity,
    original_descriptor: RawFd,
    validate_original_descriptor: impl FnOnce(RawFd) -> Result<FileObjectIdentity>,
) -> Result<RawFd> {
    let new_candidates = sqlite_descriptor_delta_candidates(before, after, expected_identity);
    match new_candidates.as_slice() {
        [descriptor] => Ok(*descriptor),
        [] => {
            let identity = validate_original_descriptor(original_descriptor)?;
            if identity != expected_identity {
                return Err(DurableDeliveryError::IsolationViolation(
                    "SQLite WAL materialization left no exact main descriptor candidate".to_owned(),
                ));
            }
            Ok(original_descriptor)
        }
        _ => Err(DurableDeliveryError::IsolationViolation(format!(
            "SQLite WAL materialization produced {} ambiguous new main descriptors",
            new_candidates.len()
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod post_wal_selector_tests {
    use super::{
        select_post_wal_main_descriptor, DurableDeliveryError, FileObjectIdentity,
        ProcessDescriptorSnapshot,
    };
    use std::collections::BTreeMap;

    fn identity(inode: u64) -> FileObjectIdentity {
        FileObjectIdentity {
            device: 7,
            inode,
            mode: 0o100600,
            uid: 501,
        }
    }

    fn snapshot(entries: &[(i32, FileObjectIdentity)]) -> ProcessDescriptorSnapshot {
        ProcessDescriptorSnapshot {
            descriptors: entries.iter().copied().collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn br192_post_wal_selector_accepts_same_ofd_or_same_number_reopen_only_after_exact_validation()
    {
        let expected = identity(11);
        let before = snapshot(&[(41, expected)]);
        let after = snapshot(&[(41, expected)]);
        let selected = select_post_wal_main_descriptor(&before, &after, expected, 41, |fd| {
            assert_eq!(fd, 41);
            Ok(expected)
        })
        .expect("same descriptor number requires an exact live identity");
        assert_eq!(selected, 41);
    }

    #[test]
    fn br192_post_wal_selector_prefers_one_unique_new_descriptor_even_when_old_remains_live() {
        let expected = identity(12);
        let before = snapshot(&[(41, expected)]);
        let after = snapshot(&[(41, expected), (57, expected)]);
        let selected = select_post_wal_main_descriptor(&before, &after, expected, 41, |_| {
            panic!("one unique new descriptor must not fall back to the old descriptor")
        })
        .expect("one unique post-WAL descriptor is selected");
        assert_eq!(selected, 57);
    }

    #[test]
    fn br192_post_wal_selector_rejects_missing_or_rebound_original_descriptor() {
        let expected = identity(13);
        let before = snapshot(&[(41, expected)]);
        let after = snapshot(&[]);
        let error =
            select_post_wal_main_descriptor(&before, &after, expected, 41, |_| Ok(identity(99)))
                .expect_err("missing exact descriptor must fail closed");
        assert!(matches!(
            error,
            DurableDeliveryError::IsolationViolation(reason)
                if reason.contains("no exact main descriptor candidate")
        ));
    }

    #[test]
    fn br192_post_wal_selector_rejects_multiple_new_descriptors() {
        let expected = identity(14);
        let before = snapshot(&[(41, expected)]);
        let after = snapshot(&[(57, expected), (63, expected)]);
        let error = select_post_wal_main_descriptor(&before, &after, expected, 41, |_| {
            panic!("ambiguous new descriptors must not use original fallback")
        })
        .expect_err("multiple post-WAL candidates must fail closed");
        assert!(matches!(
            error,
            DurableDeliveryError::IsolationViolation(reason)
                if reason.contains("2 ambiguous new main descriptors")
        ));
    }
}

impl DirectShmNode {
    fn new(
        sqlite_descriptor: RawFd,
        identity: FileObjectIdentity,
        probe: &File,
        connection: &Arc<Mutex<Connection>>,
        ownership_identity: &str,
    ) -> Result<Self> {
        static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            generation: NEXT_GENERATION.fetch_add(1, Ordering::Relaxed),
            identity,
            sqlite_descriptor,
            open_file_description_proof: OpenFileDescriptionProof::install(
                sqlite_descriptor,
                probe,
                identity,
                "shm",
                ownership_identity,
            )?,
            connection_lifetimes: Mutex::new(vec![Arc::downgrade(connection)]),
        })
    }

    fn validate_identity(&self, expected: FileObjectIdentity, probe: &File) -> Result<()> {
        let current = require_unique_regular_descriptor_identity(
            self.sqlite_descriptor,
            "process-shared live rusqlite SQLite SHM",
        )?;
        if current != self.identity || current != expected {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "process-shared SQLite SHM generation {} changed descriptor identity",
                self.generation
            )));
        }
        self.open_file_description_proof.validate(
            self.sqlite_descriptor,
            probe,
            expected,
            "shm",
        )?;
        Ok(())
    }

    fn register_connection(&self, connection: &Arc<Mutex<Connection>>) -> Result<()> {
        let incoming = Arc::downgrade(connection);
        let mut lifetimes = self.connection_lifetimes.lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "process-shared SQLite SHM lifetime registry is poisoned".to_owned(),
            )
        })?;
        lifetimes.retain(|candidate| candidate.strong_count() > 0);
        if !lifetimes
            .iter()
            .any(|candidate| candidate.ptr_eq(&incoming))
        {
            lifetimes.push(incoming);
        }
        Ok(())
    }

    fn acquire_connection_lifetime(&self) -> Result<Arc<Mutex<Connection>>> {
        let current = require_unique_regular_descriptor_identity(
            self.sqlite_descriptor,
            "process-shared live rusqlite SQLite SHM lifetime owner",
        )?;
        if current != self.identity {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "process-shared SQLite SHM generation {} changed descriptor identity",
                self.generation
            )));
        }
        let mut lifetimes = self.connection_lifetimes.lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "process-shared SQLite SHM lifetime registry is poisoned".to_owned(),
            )
        })?;
        let mut live = None;
        lifetimes.retain(|candidate| {
            let upgraded = candidate.upgrade();
            if live.is_none() {
                live = upgraded.clone();
            }
            upgraded.is_some()
        });
        live.ok_or_else(|| {
            DurableDeliveryError::IsolationViolation(format!(
                "process-shared SQLite SHM generation {} has no live connection owner",
                self.generation
            ))
        })
    }
}

fn process_shared_shm_nodes() -> &'static Mutex<BTreeMap<SharedShmKey, Weak<DirectShmNode>>> {
    static NODES: OnceLock<Mutex<BTreeMap<SharedShmKey, Weak<DirectShmNode>>>> = OnceLock::new();
    NODES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn sqlite_attestation_open_lock() -> &'static Mutex<()> {
    static OPEN_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    OPEN_LOCK.get_or_init(|| Mutex::new(()))
}

fn register_direct_shm_node(
    key: &SharedShmKey,
    node: &Arc<DirectShmNode>,
    probe: &File,
) -> Result<()> {
    let mut nodes = process_shared_shm_nodes().lock().map_err(|_| {
        DurableDeliveryError::IsolationViolation(
            "process-shared SQLite SHM proof lock is poisoned".to_owned(),
        )
    })?;
    nodes.retain(|_, candidate| candidate.strong_count() > 0);
    if let Some(existing) = nodes.get(key).and_then(Weak::upgrade) {
        existing.validate_identity(node.identity, probe)?;
        if existing.identity != node.identity {
            return Err(DurableDeliveryError::IsolationViolation(
                "existing process-shared SQLite SHM proof changed identity".to_owned(),
            ));
        }
    }
    nodes.insert(key.clone(), Arc::downgrade(node));
    Ok(())
}

fn sqlite_sidecar_leaf(database_leaf: &OsStr, suffix: &str) -> OsString {
    let mut leaf = database_leaf.to_os_string();
    leaf.push(suffix);
    leaf
}

fn validate_preexisting_sidecars(parent: &File, database_leaf: &OsStr) -> Result<()> {
    for (suffix, role) in [("-wal", "SQLite WAL"), ("-shm", "SQLite SHM")] {
        let leaf = sqlite_sidecar_leaf(database_leaf, suffix);
        let name = component_cstring(&leaf, role)?;
        // SAFETY: name is one NUL-terminated component and parent is a
        // retained directory descriptor. A successful result is newly owned.
        let descriptor = unsafe {
            openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                PIN_O_RDWR | PIN_O_NOFOLLOW | PIN_O_NONBLOCK | PIN_O_CLOEXEC,
                0o600_u32,
            )
        };
        if descriptor < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(NO_SUCH_FILE_OS_ERROR) {
                continue;
            }
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "cannot validate pre-existing {role} with openat(O_NOFOLLOW): {error}"
            )));
        }
        // SAFETY: successful openat returned one newly owned descriptor.
        let file = unsafe { File::from_raw_fd(descriptor) };
        require_unique_regular_identity(&file, &format!("pre-existing {role}"))?;
    }
    Ok(())
}

fn component_cstring(name: &OsStr, role: &str) -> Result<CString> {
    if name.is_empty() || name.as_bytes().contains(&b'/') {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "{role} must be one non-empty path component"
        )));
    }
    CString::new(name.as_bytes()).map_err(|_| {
        DurableDeliveryError::IsolationViolation(format!("{role} contains a NUL byte"))
    })
}

fn openat_component(parent: &File, name: &OsStr, flags: i32, role: &str) -> Result<File> {
    let name = component_cstring(name, role)?;
    // SAFETY: name is one NUL-terminated component, parent is a retained
    // directory descriptor, and success returns one newly owned descriptor.
    let descriptor = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | PIN_O_NOFOLLOW | PIN_O_NONBLOCK | PIN_O_CLOEXEC,
            0o600_u32,
        )
    };
    if descriptor < 0 {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "cannot open pinned {role} with openat(O_NOFOLLOW): {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: successful openat returned one newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn normal_path_components(path: &Path, role: &str) -> Result<Vec<OsString>> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => components.push(name.to_os_string()),
            _ => {
                return Err(DurableDeliveryError::IsolationViolation(format!(
                    "{role} contains a forbidden component: {}",
                    path.display()
                )))
            }
        }
    }
    Ok(components)
}

fn require_directory_identity(file: &File, role: &str) -> Result<FileObjectIdentity> {
    require_trusted_directory_identity(file, role, true)
}

fn require_directory_link_count(file: &File, role: &str) -> Result<u64> {
    let metadata = file.metadata().map_err(|error| {
        DurableDeliveryError::IsolationViolation(format!(
            "cannot inspect retained {role} link count: {error}"
        ))
    })?;
    if !metadata.is_dir() || metadata.nlink() == 0 {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "retained {role} must be a linked directory"
        )));
    }
    Ok(metadata.nlink())
}

fn require_trusted_directory_identity(
    file: &File,
    role: &str,
    require_current_user: bool,
) -> Result<FileObjectIdentity> {
    let metadata = file.metadata().map_err(|error| {
        DurableDeliveryError::IsolationViolation(format!("cannot inspect retained {role}: {error}"))
    })?;
    if !metadata.is_dir() {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "retained {role} is not a directory"
        )));
    }
    require_not_shared_writable(&metadata, role)?;
    if require_current_user {
        require_current_user_owned(&metadata, role)?;
    }
    Ok(FileObjectIdentity::from_metadata(&metadata))
}

fn require_unique_regular_identity(file: &File, role: &str) -> Result<FileObjectIdentity> {
    let metadata = file.metadata().map_err(|error| {
        DurableDeliveryError::IsolationViolation(format!("cannot inspect retained {role}: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "retained {role} is not a regular file"
        )));
    }
    if metadata.nlink() != 1 {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "retained {role} must have exactly one link, observed {}",
            metadata.nlink()
        )));
    }
    require_current_user_owned_and_not_shared_writable(&metadata, role)?;
    Ok(FileObjectIdentity::from_metadata(&metadata))
}

fn require_unique_regular_descriptor_identity(
    descriptor: RawFd,
    role: &str,
) -> Result<FileObjectIdentity> {
    let metadata = descriptor_metadata(descriptor).map_err(|error| {
        DurableDeliveryError::IsolationViolation(format!(
            "cannot inspect retained {role} descriptor: {error}"
        ))
    })?;
    if !metadata.is_file() {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "retained {role} descriptor is not a regular file"
        )));
    }
    if metadata.nlink() != 1 {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "retained {role} descriptor must have exactly one link, observed {}",
            metadata.nlink()
        )));
    }
    require_current_user_owned_and_not_shared_writable(&metadata, role)?;
    Ok(FileObjectIdentity::from_metadata(&metadata))
}

fn require_current_user_owned_and_not_shared_writable(
    metadata: &Metadata,
    role: &str,
) -> Result<()> {
    require_current_user_owned(metadata, role)?;
    require_not_shared_writable(metadata, role)
}

fn require_current_user_owned(metadata: &Metadata, role: &str) -> Result<()> {
    // SAFETY: geteuid has no arguments and returns this process's effective
    // user identifier without mutating process state.
    let effective_user = unsafe { geteuid() };
    if metadata.uid() != effective_user {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "retained {role} owner uid={} differs from effective uid={effective_user}",
            metadata.uid()
        )));
    }
    Ok(())
}

fn require_not_shared_writable(metadata: &Metadata, role: &str) -> Result<()> {
    if metadata.mode() & 0o022 != 0 {
        return Err(DurableDeliveryError::IsolationViolation(format!(
            "retained {role} must not be group/world writable: mode={:#o}",
            metadata.mode()
        )));
    }
    Ok(())
}

fn sync_directory(directory: &File, role: &str) -> Result<()> {
    require_directory_identity(directory, role)?;
    directory.sync_all().map_err(|error| {
        DurableDeliveryError::IsolationViolation(format!(
            "cannot durably sync retained {role}: {error}"
        ))
    })
}

fn descriptor_metadata(descriptor: RawFd) -> std::io::Result<Metadata> {
    if descriptor < 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "negative descriptor",
        ));
    }
    // SAFETY: descriptor belongs to this process. ManuallyDrop prevents this
    // temporary File view from closing the underlying descriptor.
    let file = ManuallyDrop::new(unsafe { File::from_raw_fd(descriptor) });
    file.metadata()
}

impl OpenFileDescriptionProof {
    fn install(
        sqlite_descriptor: RawFd,
        probe: &File,
        expected_identity: FileObjectIdentity,
        role: &str,
        ownership_identity: &str,
    ) -> Result<Self> {
        let marker_start =
            deterministic_ofd_marker_start(expected_identity, role, ownership_identity)?;
        let sqlite_identity = require_unique_regular_descriptor_identity(
            sqlite_descriptor,
            &format!("live rusqlite {role} before OFD proof"),
        )?;
        let probe_identity =
            require_unique_regular_identity(probe, &format!("SQLite {role} OFD probe"))?;
        if sqlite_identity != expected_identity || probe_identity != expected_identity {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "SQLite {role} OFD proof endpoints do not share the pinned identity"
            )));
        }
        let mut marker = open_file_description_lock(F_WRLCK_TYPE, marker_start);
        // SAFETY: sqlite_descriptor is live under the private Connection Arc;
        // marker points to the target-specific repr(C) flock layout.
        let result = unsafe {
            fcntl(
                sqlite_descriptor,
                F_OFD_SETLK_COMMAND,
                &mut marker as *mut OpenFileDescriptionLock,
            )
        };
        if result < 0 {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "cannot install SQLite {role} OFD marker: {}",
                std::io::Error::last_os_error()
            )));
        }
        let proof = Self {
            marker_start,
            ownership_identity: ownership_identity.to_owned(),
        };
        proof.validate(sqlite_descriptor, probe, expected_identity, role)?;
        Ok(proof)
    }

    fn validate(
        &self,
        sqlite_descriptor: RawFd,
        probe: &File,
        expected_identity: FileObjectIdentity,
        role: &str,
    ) -> Result<()> {
        let sqlite_identity = require_unique_regular_descriptor_identity(
            sqlite_descriptor,
            &format!("live rusqlite {role} OFD owner"),
        )?;
        let probe_identity =
            require_unique_regular_identity(probe, &format!("SQLite {role} OFD probe"))?;
        if sqlite_identity != expected_identity || probe_identity != expected_identity {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "SQLite {role} OFD proof endpoints changed pinned identity"
            )));
        }
        let mut conflict_query = open_file_description_lock(F_WRLCK_TYPE, self.marker_start);
        // SAFETY: probe is an owned, separately opened descriptor for the
        // pinned inode and conflict_query has the target-specific flock ABI.
        let result = unsafe {
            fcntl(
                probe.as_raw_fd(),
                F_OFD_GETLK_COMMAND,
                &mut conflict_query as *mut OpenFileDescriptionLock,
            )
        };
        if result < 0 {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "cannot validate SQLite {role} OFD marker: {}",
                std::io::Error::last_os_error()
            )));
        }
        if conflict_query.l_type != F_WRLCK_TYPE || conflict_query.l_start != self.marker_start {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "SQLite {role} raw descriptor lost owner-specific OFD marker {}",
                self.ownership_identity
            )));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn install_for_test(owner: &File, probe: &File) -> Result<Self> {
        let expected_identity = require_unique_regular_identity(owner, "TEST_CODE OFD owner")?;
        Self::install(
            owner.as_raw_fd(),
            probe,
            expected_identity,
            "TEST_CODE",
            "TEST_CODE_OFD_OWNER_A_0123456789abcdef",
        )
    }

    #[cfg(test)]
    pub(crate) fn install_with_owner_for_test(
        owner: &File,
        probe: &File,
        ownership_identity: &str,
    ) -> Result<Self> {
        let expected_identity = require_unique_regular_identity(owner, "TEST_CODE OFD owner")?;
        Self::install(
            owner.as_raw_fd(),
            probe,
            expected_identity,
            "TEST_CODE",
            ownership_identity,
        )
    }

    #[cfg(test)]
    pub(crate) fn validate_descriptor_for_test(
        &self,
        descriptor: RawFd,
        probe: &File,
    ) -> Result<()> {
        let expected_identity = require_unique_regular_identity(probe, "TEST_CODE OFD probe")?;
        self.validate(descriptor, probe, expected_identity, "TEST_CODE")
    }

    #[cfg(test)]
    pub(crate) fn exclusive_probe_is_available_for_test(&self, probe: &File) -> Result<bool> {
        let mut exclusive = open_file_description_lock(F_WRLCK_TYPE, self.marker_start);
        // SAFETY: probe is a live test-owned descriptor and exclusive has the
        // target-specific flock ABI.
        let result = unsafe {
            fcntl(
                probe.as_raw_fd(),
                F_OFD_SETLK_COMMAND,
                &mut exclusive as *mut OpenFileDescriptionLock,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::PermissionDenied
        ) {
            return Ok(false);
        }
        Err(DurableDeliveryError::IsolationViolation(format!(
            "TEST_CODE exclusive OFD probe failed: {error}"
        )))
    }

    #[cfg(test)]
    fn remove_from_descriptor_for_test(&self, descriptor: RawFd) -> Result<()> {
        let mut unlock = open_file_description_lock(F_UNLCK_TYPE, self.marker_start);
        // SAFETY: descriptor is the live TEST_CODE-owned SQLite descriptor and
        // unlock has the target-specific repr(C) flock layout.
        let result = unsafe {
            fcntl(
                descriptor,
                F_OFD_SETLK_COMMAND,
                &mut unlock as *mut OpenFileDescriptionLock,
            )
        };
        if result < 0 {
            return Err(DurableDeliveryError::IsolationViolation(format!(
                "cannot remove TEST_CODE SQLite OFD marker: {}",
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    }
}

fn deterministic_ofd_marker_start(
    identity: FileObjectIdentity,
    role: &str,
    ownership_identity: &str,
) -> Result<i64> {
    let material = format!(
        "br192-ofd-owner-v2|{}|{}|{}|{}|{}|{role}|{ownership_identity}",
        identity.device, identity.inode, identity.mode, identity.uid, OFD_LOCK_MARKER_BASE
    );
    let digest = sha256_hex(material.as_bytes());
    let prefix = digest.get(..16).ok_or_else(|| {
        DurableDeliveryError::IsolationViolation(
            "OFD ownership marker digest was unexpectedly short".to_owned(),
        )
    })?;
    let hash = u64::from_str_radix(prefix, 16).map_err(|error| {
        DurableDeliveryError::IsolationViolation(format!(
            "cannot derive deterministic OFD ownership marker: {error}"
        ))
    })?;
    let offset = hash % OFD_LOCK_MARKER_SPAN;
    OFD_LOCK_MARKER_BASE
        .checked_add(i64::try_from(offset).map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "deterministic OFD ownership marker exceeded signed offset range".to_owned(),
            )
        })?)
        .ok_or_else(|| {
            DurableDeliveryError::IsolationViolation(
                "deterministic OFD ownership marker overflowed".to_owned(),
            )
        })
}

fn open_file_description_lock(lock_type: i16, marker_start: i64) -> OpenFileDescriptionLock {
    OpenFileDescriptionLock {
        l_type: lock_type,
        l_whence: SEEK_SET_FROM_START,
        l_start: marker_start,
        l_len: OFD_LOCK_MARKER_LENGTH,
        l_pid: 0,
    }
}

fn process_descriptor_root() -> Result<&'static Path> {
    #[cfg(target_os = "linux")]
    {
        return Ok(Path::new("/proc/self/fd"));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(Path::new("/dev/fd"));
    }
    #[allow(unreachable_code)]
    Err(DurableDeliveryError::IsolationViolation(
        "process descriptor enumeration is unsupported on this platform".to_owned(),
    ))
}

fn ensure_supported_attestation_target() -> Result<()> {
    #[cfg(all(
        target_pointer_width = "64",
        any(target_os = "linux", target_os = "macos")
    ))]
    {
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(DurableDeliveryError::IsolationViolation(
        "descriptor-attested durable delivery is unsupported on this target".to_owned(),
    ))
}

fn probe_precreation_attestation_capabilities(
    repository_root: &Path,
    ownership_identity: &str,
) -> Result<()> {
    // Enumeration is exercised before any main-database O_CREAT. This is a
    // capability check only; its snapshot is intentionally not reused for the
    // later SQLite descriptor delta.
    ProcessDescriptorSnapshot::capture()?;
    #[cfg(test)]
    run_database_bootstrap_test_hook(
        DatabaseBootstrapTestPhase::BeforeOpenFileDescriptionCapabilityProbe,
    )?;

    // The OFD ABI is probed on an existing repository metadata file. No bytes
    // are read or written; only an owner-specific range lock is installed and
    // released when these two descriptors leave scope.
    let probe_path = repository_root.join("Cargo.toml");
    let owner = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&probe_path)
        .map_err(|error| {
            DurableDeliveryError::IsolationViolation(format!(
                "cannot open OFD capability owner {}: {error}",
                probe_path.display()
            ))
        })?;
    let observer = File::open(&probe_path).map_err(|error| {
        DurableDeliveryError::IsolationViolation(format!(
            "cannot open OFD capability observer {}: {error}",
            probe_path.display()
        ))
    })?;
    let expected_identity = require_unique_regular_identity(&owner, "OFD capability owner file")?;
    let proof = OpenFileDescriptionProof::install(
        owner.as_raw_fd(),
        &observer,
        expected_identity,
        "capability",
        ownership_identity,
    )?;
    proof.validate(
        owner.as_raw_fd(),
        &observer,
        expected_identity,
        "capability",
    )
}

impl Drop for DurableDeliveryCoordinator {
    fn drop(&mut self) {
        // BR-206: descriptor snapshots are process-wide, so connection and
        // retained proof teardown must share the same total order as opens and
        // operations. Drop cannot propagate poison as an operational error;
        // recovering the guard here preserves close serialization only.
        let _attestation_guard = sqlite_attestation_open_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        drop(self.connection.take());
        drop(self.database_binding.take());
    }
}

impl DurableDeliveryCoordinator {
    fn connection_handle(&self) -> Result<&Arc<Mutex<Connection>>> {
        self.connection.as_ref().ok_or_else(|| {
            DurableDeliveryError::IsolationViolation(
                "durable-delivery connection is unavailable during teardown".to_owned(),
            )
        })
    }

    fn database_binding(&self) -> Result<&PinnedDatabaseBinding> {
        self.database_binding.as_ref().ok_or_else(|| {
            DurableDeliveryError::IsolationViolation(
                "durable-delivery database binding is unavailable during teardown".to_owned(),
            )
        })
    }

    pub fn open(config: CoordinatorConfig) -> Result<Self> {
        config.validate()?;
        Self::open_at_repository_root(config, Path::new(env!("CARGO_MANIFEST_DIR")))
    }

    fn open_at_repository_root(config: CoordinatorConfig, repository_root: &Path) -> Result<Self> {
        ensure_supported_attestation_target()?;
        let database_path = config.repository_relative_database_path()?;
        probe_precreation_attestation_capabilities(
            repository_root,
            &config.owner_instance_identity,
        )?;
        // Process-fd snapshots are process-wide. Serialize coordinator opens
        // so another connection to the same object set cannot contaminate the
        // main/WAL delta or race initial process-shared SHM proof creation.
        let attestation_guard = sqlite_attestation_open_lock().lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "SQLite descriptor-attestation open lock is poisoned".to_owned(),
            )
        })?;
        let mut namespace =
            PinnedDatabaseNamespace::open_at_repository_root(&database_path, repository_root)?;
        namespace.directory_chain.validate()?;
        let route = namespace.sqlite_route().to_path_buf();

        // C3: opening the SQLite handle is followed immediately by exact main
        // fd/leaf/ancestor attestation. No PRAGMA, DDL or transaction may run
        // against the connection before this succeeds.
        let (connection, before_main, after_main) = open_connection_with_attestable_main(
            &route,
            namespace.main_identity,
            &attestation_guard,
        )?;
        let connection = Arc::new(Mutex::new(connection));
        let mut main = attest_sqlite_object(
            SqliteObjectRole::Main,
            namespace.leaf.clone(),
            namespace.take_main_anchor()?,
            namespace.main_identity,
            &before_main,
            &after_main,
            &config.owner_instance_identity,
        )?;
        namespace.validate_main_attestation(&main)?;

        // Only the SQLite journaling capability is materialized here. No DDL,
        // policy row or user record is allowed before WAL/SHM are retained,
        // descriptor-attested and post-validated.
        let before_sidecars = ProcessDescriptorSnapshot::capture()?;
        let after_sidecars = {
            let connection_guard = connection.lock().map_err(|_| {
                DurableDeliveryError::IsolationViolation(
                    "new SQLite connection mutex is poisoned".to_owned(),
                )
            })?;
            namespace
                .validate_main_attestation(&main)
                .map_err(|error| {
                    DurableDeliveryError::IsolationViolation(format!(
                        "SQLite main attestation failed before WAL materialization: {error}"
                    ))
                })?;
            materialize_wal_capability(&connection_guard)?;
            #[cfg(test)]
            run_database_bootstrap_test_hook(
                DatabaseBootstrapTestPhase::AfterWalMaterializationBeforeMainReattestation,
            )?;
            let after_wal = ProcessDescriptorSnapshot::capture()?;
            namespace.rebind_main_open_file_description_after_wal(
                &mut main,
                &before_sidecars,
                &after_wal,
            )?;
            after_wal
        };
        #[cfg(test)]
        run_database_bootstrap_test_hook(
            DatabaseBootstrapTestPhase::AfterMainReattestationBeforeSidecarAttestation,
        )?;
        namespace.sync_parent_directory("database parent after SQLite WAL/SHM creation")?;
        let database_binding = namespace.attest_sqlite_connection(
            &connection,
            main,
            &before_sidecars,
            &after_sidecars,
            &config.owner_instance_identity,
        )?;
        let _lifetime = database_binding.validate_under_open_lock()?;
        let coordinator = Self {
            connection: Some(connection),
            database_binding: Some(database_binding),
            config,
            #[cfg(test)]
            database_operation_test_hook: Mutex::new(None),
            #[cfg(test)]
            delivered_reconcile_test_hook: Mutex::new(None),
            #[cfg(test)]
            delivered_precommit_test_fault: Mutex::new(None),
            #[cfg(test)]
            operation_postvalidation_test_fault: Mutex::new(None),
        };
        drop(attestation_guard);

        coordinator.with_connection(|connection| configure_attested_connection(connection))?;
        coordinator.with_immediate_transaction(|transaction| {
            initialize_schema(transaction)?;
            #[cfg(test)]
            run_database_bootstrap_test_hook(
                DatabaseBootstrapTestPhase::AfterSchemaSqlBeforeCommitValidation,
            )?;
            Ok(())
        })?;
        coordinator
            .database_binding()?
            .sync_parent_directory("database parent before coordinator success")?;
        #[cfg(test)]
        run_database_bootstrap_test_hook(
            DatabaseBootstrapTestPhase::AfterFinalParentSyncBeforeSuccessValidation,
        )?;
        // The parent sync is not the success boundary. Rebind the complete
        // manifest-to-parent chain, all three fixed leaves, SQLite descriptors,
        // owner-specific OFD proofs and the live SHM connection lifetime once
        // more, then verify the connection's effective safety PRAGMAs while
        // that same attested operation lease remains held.
        let database_binding = coordinator.database_binding()?;
        let final_lease = database_binding.acquire_operation_lease()?;
        let connection_guard = coordinator.connection_handle()?.lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "final bootstrap connection mutex is poisoned".to_owned(),
            )
        })?;
        verify_connection_configuration(&connection_guard)?;
        let _final_post_connection_lifetime = database_binding.validate_under_open_lock()?;
        drop(connection_guard);
        drop(final_lease);
        Ok(coordinator)
    }

    #[cfg(test)]
    pub(crate) fn install_database_operation_test_hook(
        &self,
        phase: DatabaseOperationTestPhase,
        callback: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Result<()> {
        let mut hook = self.database_operation_test_hook.lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "database-operation test hook mutex is poisoned".to_owned(),
            )
        })?;
        if hook.is_some() {
            return Err(DurableDeliveryError::InvalidConfiguration(
                "a database-operation test hook is already installed".to_owned(),
            ));
        }
        *hook = Some(DatabaseOperationTestHook {
            phase,
            callback: Box::new(callback),
        });
        Ok(())
    }

    #[cfg(test)]
    fn run_database_operation_test_hook(&self, phase: DatabaseOperationTestPhase) -> Result<()> {
        let callback = {
            let mut hook = self.database_operation_test_hook.lock().map_err(|_| {
                DurableDeliveryError::IsolationViolation(
                    "database-operation test hook mutex is poisoned".to_owned(),
                )
            })?;
            match hook.as_ref() {
                Some(installed) if installed.phase == phase => {
                    hook.take().map(|installed| installed.callback)
                }
                _ => None,
            }
        };
        match callback {
            Some(callback) => callback(),
            None => Ok(()),
        }
    }

    #[cfg(test)]
    pub(crate) fn install_delivered_reconcile_test_hook(
        &self,
        callback: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Result<()> {
        let mut hook = self.delivered_reconcile_test_hook.lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "Delivered reconcile test hook mutex is poisoned".to_owned(),
            )
        })?;
        if hook.is_some() {
            return Err(DurableDeliveryError::InvalidConfiguration(
                "a Delivered reconcile test hook is already installed".to_owned(),
            ));
        }
        *hook = Some(DeliveredReconcileTestHook {
            callback: Box::new(callback),
        });
        Ok(())
    }

    #[cfg(test)]
    fn run_delivered_reconcile_test_hook(&self) -> Result<()> {
        let callback = self
            .delivered_reconcile_test_hook
            .lock()
            .map_err(|_| {
                DurableDeliveryError::IsolationViolation(
                    "Delivered reconcile test hook mutex is poisoned".to_owned(),
                )
            })?
            .take()
            .map(|hook| hook.callback);
        callback.map_or(Ok(()), |callback| callback())
    }

    #[cfg(test)]
    pub(crate) fn install_delivered_precommit_test_fault(
        &self,
        fault: DeliveredPrecommitTestFault,
    ) -> Result<()> {
        let mut installed = self.delivered_precommit_test_fault.lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "Delivered precommit test-fault mutex is poisoned".to_owned(),
            )
        })?;
        if installed.is_some() {
            return Err(DurableDeliveryError::InvalidConfiguration(
                "a Delivered precommit test fault is already installed".to_owned(),
            ));
        }
        *installed = Some(fault);
        Ok(())
    }

    #[cfg(test)]
    fn apply_delivered_precommit_test_fault(
        &self,
        transaction: &Transaction<'_>,
        decision_identity: &str,
    ) -> Result<()> {
        let fault = self
            .delivered_precommit_test_fault
            .lock()
            .map_err(|_| {
                DurableDeliveryError::IsolationViolation(
                    "Delivered precommit test-fault mutex is poisoned".to_owned(),
                )
            })?
            .take();
        let Some(fault) = fault else {
            return Ok(());
        };
        let changed = match fault {
            DeliveredPrecommitTestFault::AuthoritativeDispositionSemanticBinding => {
                let (disposition_identity, canonical): (String, Vec<u8>) = transaction.query_row(
                    "SELECT p.disposition_identity,p.disposition_canonical
                         FROM delivery_decisions d
                         JOIN delivery_disposition_payloads p
                           ON p.disposition_identity=d.current_disposition_identity
                         WHERE d.decision_identity=?1 AND p.disposition='Accepted'",
                    [decision_identity],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                let mut payload: serde_json::Value = serde_json::from_slice(&canonical)?;
                payload
                    .as_object_mut()
                    .ok_or_else(|| {
                        DurableDeliveryError::InvalidConfiguration(
                            "TEST_CODE disposition fault payload is not an object".to_owned(),
                        )
                    })?
                    .insert("retry_authorized".to_owned(), serde_json::Value::Bool(true));
                let corrupted = serde_json::to_vec(&payload)?;
                let corrupted_sha256 = sha256_hex(&corrupted);
                transaction.execute_batch("DROP TRIGGER immutable_disposition_payload_update")?;
                transaction.execute(
                    "UPDATE delivery_disposition_payloads
                     SET disposition_canonical=?1,disposition_sha256=?2
                     WHERE disposition_identity=?3",
                    params![corrupted, corrupted_sha256, disposition_identity],
                )?
            }
            DeliveredPrecommitTestFault::AcceptedSinkResultReceiptBinding => {
                let (result_event_identity, canonical): (String, Vec<u8>) = transaction.query_row(
                    "SELECT result_event_identity,result_canonical
                         FROM sink_results
                         WHERE decision_identity=?1
                           AND authoritative_for_state=1 AND result_kind='Accepted'",
                    [decision_identity],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                let mut payload: serde_json::Value = serde_json::from_slice(&canonical)?;
                payload
                    .get_mut("receipt")
                    .and_then(serde_json::Value::as_object_mut)
                    .ok_or_else(|| {
                        DurableDeliveryError::InvalidConfiguration(
                            "TEST_CODE accepted-result receipt fault payload is missing".to_owned(),
                        )
                    })?
                    .insert(
                        "channel".to_owned(),
                        serde_json::Value::String("TEST_CODE_REBOUND_CHANNEL".to_owned()),
                    );
                let corrupted = serde_json::to_vec(&payload)?;
                let corrupted_sha256 = sha256_hex(&corrupted);
                transaction.execute_batch("DROP TRIGGER immutable_sink_result_update")?;
                transaction.execute(
                    "UPDATE sink_results SET result_canonical=?1,result_sha256=?2
                     WHERE result_event_identity=?3",
                    params![corrupted, corrupted_sha256, result_event_identity],
                )?
            }
            DeliveredPrecommitTestFault::TaskTransitionSemanticBinding => {
                let (transition_identity, canonical): (String, Vec<u8>) = transaction.query_row(
                    "SELECT transition_identity,transition_canonical
                         FROM task_transition_payloads
                         WHERE decision_identity=?1",
                    [decision_identity],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                let mut payload: serde_json::Value = serde_json::from_slice(&canonical)?;
                payload
                    .as_object_mut()
                    .ok_or_else(|| {
                        DurableDeliveryError::InvalidConfiguration(
                            "TEST_CODE task-transition fault payload is not an object".to_owned(),
                        )
                    })?
                    .insert(
                        "task_identity".to_owned(),
                        serde_json::Value::String("TEST_CODE_REBOUND_TASK".to_owned()),
                    );
                let corrupted = serde_json::to_vec(&payload)?;
                let corrupted_sha256 = sha256_hex(&corrupted);
                transaction.execute_batch("DROP TRIGGER immutable_task_transition_update")?;
                transaction.execute(
                    "UPDATE task_transition_payloads
                     SET transition_canonical=?1,transition_sha256=?2
                     WHERE transition_identity=?3",
                    params![corrupted, corrupted_sha256, transition_identity],
                )?
            }
        };
        if changed != 1 {
            return Err(DurableDeliveryError::InvalidConfiguration(format!(
                "TEST_CODE Delivered precommit fault {fault:?} affected {changed} rows; expected exactly one"
            )));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn install_operation_postvalidation_test_fault(
        &self,
        fault: OperationPostvalidationTestFault,
    ) -> Result<()> {
        let mut installed = self
            .operation_postvalidation_test_fault
            .lock()
            .map_err(|_| {
                DurableDeliveryError::IsolationViolation(
                    "operation postvalidation test fault mutex is poisoned".to_owned(),
                )
            })?;
        if installed.is_some() {
            return Err(DurableDeliveryError::InvalidConfiguration(
                "an operation postvalidation test fault is already installed".to_owned(),
            ));
        }
        *installed = Some(fault);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn replace_current_disposition_identity_for_test(
        &self,
        decision_identity: &str,
        replacement: &str,
    ) -> Result<()> {
        if !matches!(
            &self.config.environment,
            crate::durable_delivery::model::StoreEnvironment::Test { .. }
        ) || !replacement.starts_with("TEST_CODE")
        {
            return Err(DurableDeliveryError::InvalidConfiguration(
                "Delivered race mutation is restricted to TEST_CODE identities".to_owned(),
            ));
        }
        self.with_immediate_transaction(|transaction| {
            let changed = transaction.execute(
                "UPDATE delivery_decisions SET current_disposition_identity=?1
                 WHERE decision_identity=?2",
                params![replacement, decision_identity],
            )?;
            require_single_cas_update(changed, "TEST_CODE current disposition replacement")
        })
    }

    #[cfg(test)]
    pub(crate) fn remove_bound_main_ofd_marker_for_test(&self) -> Result<()> {
        let main = self.database_binding()?.objects.first().ok_or_else(|| {
            DurableDeliveryError::IsolationViolation(
                "TEST_CODE bound SQLite object set has no main object".to_owned(),
            )
        })?;
        if !matches!(main.role, SqliteObjectRole::Main) {
            return Err(DurableDeliveryError::IsolationViolation(
                "TEST_CODE first bound SQLite object is not main".to_owned(),
            ));
        }
        match &main.descriptor_attestation {
            SqliteDescriptorAttestation::Direct {
                sqlite_descriptor,
                open_file_description_proof,
            } => open_file_description_proof.remove_from_descriptor_for_test(*sqlite_descriptor),
            SqliteDescriptorAttestation::DirectShm { .. }
            | SqliteDescriptorAttestation::ProcessSharedShm { .. } => {
                Err(DurableDeliveryError::IsolationViolation(
                    "TEST_CODE bound SQLite main does not carry a direct OFD proof".to_owned(),
                ))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn transaction_row_count_after_fault_for_test(
        &self,
        decision_identity: &str,
    ) -> Result<i64> {
        let connection = self.connection_handle()?.lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "fault-verification connection mutex is poisoned".to_owned(),
            )
        })?;
        Ok(connection.query_row(
            "SELECT COUNT(*) FROM delivery_decisions WHERE decision_identity=?1",
            [decision_identity],
            |row| row.get(0),
        )?)
    }

    #[cfg(test)]
    pub(crate) fn transaction_persistence_snapshot_after_fault_for_test(
        &self,
        decision_identity: &str,
    ) -> Result<(i64, i64, i64)> {
        let connection = self.connection_handle()?.lock().map_err(|_| {
            DurableDeliveryError::IsolationViolation(
                "fault-verification connection mutex is poisoned".to_owned(),
            )
        })?;
        let decision_rows = connection.query_row(
            "SELECT COUNT(*) FROM delivery_decisions WHERE decision_identity=?1",
            [decision_identity],
            |row| row.get(0),
        )?;
        let policy_rows =
            connection.query_row("SELECT COUNT(*) FROM delivery_policy_catalog", [], |row| {
                row.get(0)
            })?;
        let user_version = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        Ok((decision_rows, policy_rows, user_version))
    }

    pub fn prepare(
        &self,
        envelope: &DeliveryEnvelope,
        authoritative_sink_count: usize,
        admission_at: DateTime<Utc>,
    ) -> Result<PrepareOutcome> {
        enum PrepareTransactionOutcome {
            Existing(Box<PrepareOutcome>),
            IdentityConflict,
            Inserted,
        }
        let raw_canonical = serde_json::to_vec(envelope)?;
        let raw_sha256 = sha256_hex(&raw_canonical);
        let transaction_outcome = self.with_immediate_transaction(|transaction| {
            if let Some(existing) = load_decision(transaction, &envelope.decision_identity)? {
                if existing.envelope_canonical == raw_canonical
                    && existing.envelope_sha256 == raw_sha256
                {
                    envelope.validate()?;
                    let hydration =
                        load_schedule_hydration(transaction, &envelope.decision_identity)?;
                    return Ok(PrepareTransactionOutcome::Existing(Box::new(
                        outcome_from_stored(&existing, &hydration),
                    )));
                }
                let evidence = canonical_json(&json!({
                    "decision_identity": envelope.decision_identity,
                    "stored_envelope_sha256": existing.envelope_sha256,
                    "incoming_envelope_sha256": raw_sha256,
                }))?;
                enqueue_audit(
                    transaction,
                    &envelope.decision_identity,
                    None,
                    "DecisionIdentityConflict",
                    &evidence,
                    admission_at,
                )?;
                return Ok(PrepareTransactionOutcome::IdentityConflict);
            }

            envelope.validate()?;
            let policy = load_policy(transaction, envelope.push_kind, envelope.sub_kind)?;
            let denial = self.evaluate_prepare_denial(
                transaction,
                envelope,
                &policy,
                authoritative_sink_count,
                admission_at,
            )?;
            let initial_state = if denial.is_some() {
                DecisionState::RejectedAuditPending
            } else {
                DecisionState::Reserved
            };
            insert_new_decision(
                transaction,
                envelope,
                &raw_canonical,
                &raw_sha256,
                initial_state,
                admission_at,
            )?;
            record_state_transition(
                transaction,
                &envelope.decision_identity,
                None,
                initial_state,
                "prepare",
                None,
                canonical_json(&json!({
                    "envelope_sha256": raw_sha256,
                    "reservation_generation": if denial.is_some() { 0 } else { 1 },
                }))?,
                admission_at,
            )?;

            if let Some(denial) = denial {
                freeze_pre_sink_denial(transaction, envelope, &policy, denial, admission_at)?;
            } else {
                self.reserve_generation(transaction, envelope, &policy, 1, admission_at)?;
            }
            Ok(PrepareTransactionOutcome::Inserted)
        })?;
        match transaction_outcome {
            PrepareTransactionOutcome::Existing(outcome) => return Ok(*outcome),
            PrepareTransactionOutcome::IdentityConflict => {
                return Err(DurableDeliveryError::DecisionIdentityConflict {
                    decision_identity: envelope.decision_identity.clone(),
                })
            }
            PrepareTransactionOutcome::Inserted => {}
        }
        let stored = self.with_connection(|connection| {
            load_decision(connection, &envelope.decision_identity)?.ok_or_else(|| {
                DurableDeliveryError::DecisionNotFound(envelope.decision_identity.clone())
            })
        })?;
        Ok(outcome_from_stored(&stored, &None))
    }

    pub fn decision_state(&self, decision_identity: &str) -> Result<DecisionState> {
        self.with_connection(|connection| {
            load_decision(connection, decision_identity)?
                .map(|stored| stored.state)
                .ok_or_else(|| DurableDeliveryError::DecisionNotFound(decision_identity.to_owned()))
        })
    }

    /// Return the durable owner of an exact review-task occurrence without
    /// mutating admission state or contacting a sink.
    ///
    /// BR-200 places this read seam before provider acquisition. For
    /// BusinessDateOnce rows the retained claim is authoritative. Rolling
    /// rows use the exact task identity and prefer the sole Delivered row over
    /// a later durable denial created by an older duplicate invocation.
    pub fn inspect_review_task_occurrence(
        &self,
        business_date: &str,
        push_kind: super::model::PushKind,
        sub_kind: super::model::DeliverySubKind,
        scope_key: &str,
        task_identity: &str,
    ) -> Result<Option<super::model::ReviewTaskOccurrenceEvidence>> {
        super::model::validate_business_date(business_date)?;
        if scope_key.trim().is_empty() || task_identity.trim().is_empty() {
            return Err(DurableDeliveryError::PolicyMismatch(
                "review_task_occurrence_identity_invalid".to_owned(),
            ));
        }
        let policy = compiled_policy_catalog()
            .into_iter()
            .find(|row| row.push_kind == push_kind && row.sub_kind == sub_kind)
            .ok_or_else(|| {
                DurableDeliveryError::PolicyMismatch(format!(
                    "review task occurrence has no compiled policy for {push_kind}/{sub_kind}"
                ))
            })?;
        let expected_scope = match policy.cooldown_scope {
            super::model::CooldownScope::Global => "GLOBAL",
            super::model::CooldownScope::PerTicket => scope_key,
        };
        if scope_key != expected_scope {
            return Err(DurableDeliveryError::PolicyMismatch(format!(
                "review task occurrence scope mismatch for {push_kind}/{sub_kind}: {scope_key}"
            )));
        }

        self.with_connection(|connection| {
            let claim = if policy.window_mode == super::model::WindowMode::BusinessDateOnce {
                connection
                    .query_row(
                        "SELECT decision_identity FROM business_date_once_claims
                         WHERE business_date=?1 AND push_kind=?2
                           AND sub_kind=?3 AND scope_key=?4",
                        params![business_date, push_kind.as_str(), sub_kind.as_str(), scope_key],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
            } else {
                None
            };

            let mut statement = connection.prepare(
                "SELECT decision_identity FROM delivery_decisions
                 WHERE business_date=?1 AND push_kind=?2 AND sub_kind=?3
                   AND scope_key=?4 AND task_binding_present=1
                 ORDER BY decision_identity",
            )?;
            let rows = statement.query_map(
                params![business_date, push_kind.as_str(), sub_kind.as_str(), scope_key],
                |row| row.get::<_, String>(0),
            )?;
            let mut matches = Vec::new();
            // BR-214: decisions frozen under a retired `policy_version` are not
            // evidence about the policy in force today. A `Delivered` decision stays
            // authoritative regardless (a delivery is a fact, and re-pushing it would
            // duplicate a real message), but a denial produced by a retired policy
            // must not keep denying under the successor policy.
            let mut current_policy_matches = Vec::new();
            for row in rows {
                let decision_identity = row?;
                let stored = load_decision(connection, &decision_identity)?.ok_or_else(|| {
                    DurableDeliveryError::DecisionNotFound(decision_identity.clone())
                })?;
                if sha256_hex(&stored.envelope_canonical) != stored.envelope_sha256 {
                    return Err(DurableDeliveryError::PolicyMismatch(format!(
                        "review task occurrence envelope hash mismatch for {decision_identity}"
                    )));
                }
                let envelope = parse_envelope(&stored.envelope_canonical)?;
                if envelope.decision_identity != decision_identity
                    || envelope.business_date != business_date
                    || envelope.push_kind != push_kind
                    || envelope.sub_kind != sub_kind
                    || envelope.scope_key != scope_key
                {
                    return Err(DurableDeliveryError::PolicyMismatch(format!(
                        "review task occurrence envelope identity mismatch for {decision_identity}"
                    )));
                }
                if envelope
                    .task_binding
                    .as_ref()
                    .is_some_and(|binding| binding.task_identity == task_identity)
                {
                    if envelope.policy_version == policy.policy_version {
                        current_policy_matches.push(stored.clone());
                    }
                    matches.push(stored);
                }
            }

            let selected = if let Some(claimed_identity) = claim {
                let claimed = matches
                    .into_iter()
                    .find(|stored| stored.decision_identity == claimed_identity)
                    .ok_or_else(|| {
                        DurableDeliveryError::PolicyMismatch(format!(
                            "review task occurrence claim {claimed_identity} does not match task {task_identity}"
                        ))
                    })?;
                Some(claimed)
            } else {
                let mut delivered = matches
                    .iter()
                    .filter(|stored| stored.state == DecisionState::Delivered);
                let first_delivered = delivered.next().cloned();
                if delivered.next().is_some() {
                    return Err(DurableDeliveryError::PolicyMismatch(format!(
                        "review task occurrence {task_identity} has multiple Delivered decisions"
                    )));
                }
                match first_delivered {
                    Some(stored) => Some(stored),
                    None if current_policy_matches.is_empty() => None,
                    None if current_policy_matches.len() == 1 => {
                        current_policy_matches.into_iter().next()
                    }
                    None => {
                        return Err(DurableDeliveryError::PolicyMismatch(format!(
                            "review task occurrence {task_identity} has ambiguous non-Delivered decisions"
                        )))
                    }
                }
            };

            selected
                .map(|stored| {
                    Ok(super::model::ReviewTaskOccurrenceEvidence {
                        schedule_hydration: load_schedule_hydration(
                            connection,
                            &stored.decision_identity,
                        )?,
                        decision_identity: stored.decision_identity,
                        state: stored.state,
                    })
                })
                .transpose()
        })
    }

    pub fn load_exact_terminal_replay_input(
        &self,
        business_date: &str,
        review_task: &str,
        task_identity: &str,
    ) -> Result<ReviewTerminalReplayInput> {
        super::model::validate_business_date(business_date)?;
        let expected_push_kind = replay_push_kind(review_task)?;
        if task_identity.trim().is_empty() {
            return Err(DurableDeliveryError::PolicyMismatch(
                "terminal_replay_identity_invalid".to_owned(),
            ));
        }
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT decision_identity,envelope_canonical,envelope_sha256
                 FROM delivery_decisions
                 WHERE business_date=?1 AND push_kind=?2 AND task_binding_present=1
                 ORDER BY decision_identity",
            )?;
            let rows = statement.query_map(
                params![business_date, expected_push_kind.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )?;
            let mut matches = Vec::new();
            for row in rows {
                let (decision_identity, canonical, stored_sha256) = row?;
                if sha256_hex(&canonical) != stored_sha256 {
                    return Err(DurableDeliveryError::PolicyMismatch(
                        "terminal_replay_identity_invalid".to_owned(),
                    ));
                }
                let envelope = parse_envelope(&canonical)?;
                if envelope
                    .task_binding
                    .as_ref()
                    .is_some_and(|binding| binding.task_identity == task_identity)
                {
                    if envelope.decision_identity != decision_identity
                        || envelope.business_date != business_date
                        || envelope.push_kind != expected_push_kind
                    {
                        return Err(DurableDeliveryError::PolicyMismatch(
                            "terminal_replay_identity_invalid".to_owned(),
                        ));
                    }
                    matches.push(ReviewTerminalReplayInput {
                        business_date: business_date.to_owned(),
                        review_task: review_task.to_owned(),
                        task_identity: task_identity.to_owned(),
                        decision_identity,
                        envelope,
                    });
                }
            }
            if matches.len() != 1 {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "terminal_replay_identity_invalid: expected one decision, observed {}",
                    matches.len()
                )));
            }
            Ok(matches.remove(0))
        })
    }

    pub fn begin_review_terminal_replay(
        &self,
        input: &ReviewTerminalReplayInput,
        started_at: DateTime<Utc>,
    ) -> Result<ReviewTerminalReplayAttempt> {
        let expected_push_kind = replay_push_kind(&input.review_task)?;
        self.with_immediate_transaction(|transaction| {
            let stored =
                load_decision(transaction, &input.decision_identity)?.ok_or_else(|| {
                    DurableDeliveryError::DecisionNotFound(input.decision_identity.clone())
                })?;
            let envelope = parse_envelope(&stored.envelope_canonical)?;
            if envelope != input.envelope
                || envelope.business_date != input.business_date
                || envelope.push_kind != expected_push_kind
                || envelope
                    .task_binding
                    .as_ref()
                    .is_none_or(|binding| binding.task_identity != input.task_identity)
            {
                return Err(DurableDeliveryError::PolicyMismatch(
                    "terminal_replay_identity_invalid".to_owned(),
                ));
            }
            let replay_ordinal: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(replay_ordinal),0)+1
                 FROM review_terminal_replay_attempts
                 WHERE business_date=?1 AND task_identity=?2 AND decision_identity=?3",
                params![
                    input.business_date,
                    input.task_identity,
                    input.decision_identity
                ],
                |row| row.get(0),
            )?;
            let ordinal = replay_ordinal.to_string();
            let attempt_identity = stable_identity(
                REVIEW_TERMINAL_REPLAY_ATTEMPT_DOMAIN,
                &[
                    &input.business_date,
                    &input.review_task,
                    &input.task_identity,
                    &input.decision_identity,
                    &ordinal,
                ],
            );
            let pre_sink_watermark =
                sink_authority_watermark(transaction, &input.decision_identity)?;
            let pre_delivery_audit_watermark =
                delivery_audit_authority_watermark(transaction, &input.decision_identity)?;
            let canonical = ReviewTerminalReplayStartCanonical {
                schema_version: 1,
                attempt_identity: attempt_identity.clone(),
                business_date: input.business_date.clone(),
                review_task: input.review_task.clone(),
                task_identity: input.task_identity.clone(),
                decision_identity: input.decision_identity.clone(),
                replay_ordinal,
                started_at: timestamp(started_at),
                pre_sink_watermark: pre_sink_watermark.clone(),
                pre_delivery_audit_watermark: pre_delivery_audit_watermark.clone(),
                provider_calls: 0,
            };
            let start_canonical = serde_json::to_vec(&canonical)?;
            let start_sha256 = sha256_hex(&start_canonical);
            let start_audit_identity = enqueue_audit(
                transaction,
                &input.decision_identity,
                None,
                "ReviewTerminalReplayStarted",
                &start_canonical,
                started_at,
            )?;
            transaction.execute(
                "INSERT INTO review_terminal_replay_attempts(
                   attempt_identity,business_date,review_task,task_identity,
                   decision_identity,replay_ordinal,started_at,pre_sink_count,
                   pre_sink_set_sha256,pre_delivery_audit_count,
                   pre_delivery_audit_set_sha256,provider_calls,start_canonical,
                   start_sha256,start_audit_identity
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0,?12,?13,?14)",
                params![
                    attempt_identity,
                    input.business_date,
                    input.review_task,
                    input.task_identity,
                    input.decision_identity,
                    replay_ordinal,
                    timestamp(started_at),
                    pre_sink_watermark.count,
                    pre_sink_watermark.ordered_identity_set_sha256,
                    pre_delivery_audit_watermark.count,
                    pre_delivery_audit_watermark.ordered_identity_set_sha256,
                    start_canonical,
                    start_sha256,
                    start_audit_identity,
                ],
            )?;
            Ok(ReviewTerminalReplayAttempt {
                attempt_identity,
                decision_identity: input.decision_identity.clone(),
                replay_ordinal,
                start_audit_identity,
                pre_sink_watermark,
                pre_delivery_audit_watermark,
            })
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_review_terminal_replay(
        &self,
        attempt: &ReviewTerminalReplayAttempt,
        state: ReviewTerminalReplayCompletionState,
        reason_code: &str,
        provider_calls: i64,
        resume_calls: i64,
        sink_calls: i64,
        delivery_audit_appends: i64,
        completed_at: DateTime<Utc>,
    ) -> Result<ReviewTerminalReplayCompletion> {
        validate_replay_reason_code(state, reason_code)?;
        self.with_immediate_transaction(|transaction| {
            let stored_decision: Option<String> = transaction
                .query_row(
                    "SELECT decision_identity
                     FROM review_terminal_replay_attempts
                     WHERE attempt_identity=?1",
                    [&attempt.attempt_identity],
                    |row| row.get(0),
                )
                .optional()?;
            if stored_decision.as_deref() != Some(attempt.decision_identity.as_str()) {
                return Err(DurableDeliveryError::PolicyMismatch(
                    "terminal_replay_identity_invalid".to_owned(),
                ));
            }
            let post_sink_watermark =
                sink_authority_watermark(transaction, &attempt.decision_identity)?;
            let post_delivery_audit_watermark =
                delivery_audit_authority_watermark(transaction, &attempt.decision_identity)?;
            if state == ReviewTerminalReplayCompletionState::Passed
                && (provider_calls != 0
                    || resume_calls != 0
                    || sink_calls != 0
                    || delivery_audit_appends != 0
                    || attempt.pre_sink_watermark.count != 1
                    || attempt.pre_delivery_audit_watermark.count != 1
                    || attempt.pre_sink_watermark != post_sink_watermark
                    || attempt.pre_delivery_audit_watermark != post_delivery_audit_watermark)
            {
                return Err(DurableDeliveryError::PolicyMismatch(
                    "terminal_replay_watermark_changed".to_owned(),
                ));
            }
            let canonical = ReviewTerminalReplayCompletionCanonical {
                schema_version: 1,
                attempt_identity: attempt.attempt_identity.clone(),
                decision_identity: attempt.decision_identity.clone(),
                state,
                completed_at: timestamp(completed_at),
                post_sink_watermark: post_sink_watermark.clone(),
                post_delivery_audit_watermark: post_delivery_audit_watermark.clone(),
                provider_calls,
                resume_calls,
                sink_calls,
                delivery_audit_appends,
                reason_code: reason_code.to_owned(),
            };
            let completion_canonical = serde_json::to_vec(&canonical)?;
            let completion_sha256 = sha256_hex(&completion_canonical);
            let completion_audit_identity = enqueue_audit(
                transaction,
                &attempt.decision_identity,
                None,
                "ReviewTerminalReplayCompleted",
                &completion_canonical,
                completed_at,
            )?;
            transaction.execute(
                "INSERT INTO review_terminal_replay_completions(
                   attempt_identity,decision_identity,state,completed_at,
                   post_sink_count,post_sink_set_sha256,post_delivery_audit_count,
                   post_delivery_audit_set_sha256,provider_calls,resume_calls,
                   sink_calls,delivery_audit_appends,reason_code,
                   completion_canonical,completion_sha256,completion_audit_identity
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                params![
                    attempt.attempt_identity,
                    attempt.decision_identity,
                    state.as_str(),
                    timestamp(completed_at),
                    post_sink_watermark.count,
                    post_sink_watermark.ordered_identity_set_sha256,
                    post_delivery_audit_watermark.count,
                    post_delivery_audit_watermark.ordered_identity_set_sha256,
                    provider_calls,
                    resume_calls,
                    sink_calls,
                    delivery_audit_appends,
                    reason_code,
                    completion_canonical,
                    completion_sha256,
                    completion_audit_identity,
                ],
            )?;
            Ok(ReviewTerminalReplayCompletion {
                attempt_identity: attempt.attempt_identity.clone(),
                decision_identity: attempt.decision_identity.clone(),
                state,
                completion_audit_identity,
                post_sink_watermark,
                post_delivery_audit_watermark,
                reason_code: reason_code.to_owned(),
            })
        })
    }

    pub fn review_terminal_replay_audit_appended(
        &self,
        audit_identity: &str,
        decision_identity: &str,
        expected_kind: &str,
    ) -> Result<bool> {
        if !matches!(
            expected_kind,
            "ReviewTerminalReplayStarted" | "ReviewTerminalReplayCompleted"
        ) {
            return Err(DurableDeliveryError::PolicyMismatch(
                "terminal_replay_evidence_unavailable".to_owned(),
            ));
        }
        self.with_connection(|connection| {
            let state: Option<String> = connection
                .query_row(
                    "SELECT append_state FROM immutable_audit_outbox
                     WHERE audit_identity=?1 AND decision_identity=?2
                       AND attempt_identity IS NULL AND audit_kind=?3",
                    params![audit_identity, decision_identity, expected_kind],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(state.as_deref() == Some("Appended"))
        })
    }

    pub fn append_review_terminal_replay_audit(
        &self,
        audit_identity: &str,
        decision_identity: &str,
        expected_kind: &str,
        append_port: &dyn ImmutableAppendPort,
    ) -> Result<()> {
        if !matches!(
            expected_kind,
            "ReviewTerminalReplayStarted" | "ReviewTerminalReplayCompleted"
        ) {
            return Err(DurableDeliveryError::PolicyMismatch(
                "terminal_replay_evidence_unavailable".to_owned(),
            ));
        }
        let pending = self.with_connection(|connection| {
            Ok(connection
                .query_row(
                    "SELECT o.audit_kind,o.audit_identity,o.audit_canonical,o.audit_sha256,
                            o.decision_identity,o.append_state
                     FROM immutable_audit_outbox o
                     LEFT JOIN immutable_audit_outbox predecessor
                       ON predecessor.audit_identity=o.predecessor_audit_identity
                     WHERE o.audit_identity=?1 AND o.decision_identity=?2
                       AND o.attempt_identity IS NULL AND o.audit_kind=?3
                       AND (
                         o.predecessor_audit_identity IS NULL
                         OR predecessor.append_state='Appended'
                       )",
                    params![audit_identity, decision_identity, expected_kind],
                    |row| {
                        Ok((
                            PendingAppend {
                                record_kind: row.get(0)?,
                                identity: row.get(1)?,
                                canonical: row.get(2)?,
                                sha256: row.get(3)?,
                                decision_identity: row.get(4)?,
                            },
                            row.get::<_, String>(5)?,
                        ))
                    },
                )
                .optional()?)
        })?;
        let Some((pending, append_state)) = pending else {
            return Err(DurableDeliveryError::PolicyMismatch(
                "terminal_replay_evidence_unavailable".to_owned(),
            ));
        };
        if append_state == "Appended" {
            return Ok(());
        }
        if append_state != "Pending" {
            return Err(DurableDeliveryError::PolicyMismatch(
                "terminal_replay_evidence_unavailable".to_owned(),
            ));
        }
        let immutable_ref = require_nonempty_immutable_ref(
            append_port.append_exact(
                &pending.record_kind,
                &pending.identity,
                &pending.canonical,
                &pending.sha256,
            )?,
            &pending.record_kind,
            &pending.identity,
        )?;
        self.with_immediate_transaction(|transaction| {
            let changed = transaction.execute(
                "UPDATE immutable_audit_outbox
                 SET append_state='Appended',immutable_audit_ref=?1
                 WHERE audit_identity=?2 AND decision_identity=?3
                   AND append_state='Pending' AND audit_sha256=?4",
                params![
                    immutable_ref,
                    pending.identity,
                    pending.decision_identity,
                    pending.sha256
                ],
            )?;
            require_single_cas_update(
                changed,
                "review terminal replay audit append acknowledgement",
            )
        })
    }

    pub fn verify_manual_accepted_delivery(&self, decision_identity: &str) -> Result<()> {
        self.with_connection(|connection| {
            let stored = load_decision(connection, decision_identity)?.ok_or_else(|| {
                DurableDeliveryError::DecisionNotFound(decision_identity.to_owned())
            })?;
            if stored.state != DecisionState::Delivered {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "manual accepted delivery verification requires Delivered, observed {}",
                    stored.state
                )));
            }
            load_and_validate_manual_accepted_delivery_evidence(connection, decision_identity)
                .map(|_| ())
        })
    }

    pub fn resume_deliverable(
        &self,
        decision_identity: &str,
        authoritative_sinks: &[AuthoritativeSink],
        now: DateTime<Utc>,
    ) -> Result<ResumeOutcome> {
        let current = self.with_connection(|connection| {
            load_decision(connection, decision_identity)?
                .ok_or_else(|| DurableDeliveryError::DecisionNotFound(decision_identity.to_owned()))
        })?;

        if current.state == DecisionState::RejectedDurable && current.retry_authorized {
            if !self.reacquire_rejected(&current, now)? {
                return Ok(ResumeOutcome {
                    decision_identity: decision_identity.to_owned(),
                    state: DecisionState::RejectedDurable,
                    sink_calls: 0,
                    persisted_receipt: false,
                });
            }
        } else if current.state != DecisionState::Reserved {
            return Ok(ResumeOutcome {
                decision_identity: decision_identity.to_owned(),
                state: current.state,
                sink_calls: 0,
                persisted_receipt: self.has_persisted_receipt(decision_identity)?,
            });
        }

        let attempt = match self.begin_attempt(decision_identity, authoritative_sinks.len(), now)? {
            Some(attempt) => attempt,
            None => {
                return Ok(ResumeOutcome {
                    decision_identity: decision_identity.to_owned(),
                    state: self.decision_state(decision_identity)?,
                    sink_calls: 0,
                    persisted_receipt: false,
                });
            }
        };
        let result = authoritative_sinks[0].deliver(&attempt.request);
        let persisted_receipt = matches!(result, AuthoritativeSinkResult::Accepted(_));
        self.record_sink_result(&attempt.attempt_identity, attempt.fence_token, result, now)?;
        Ok(ResumeOutcome {
            decision_identity: decision_identity.to_owned(),
            state: self.decision_state(decision_identity)?,
            sink_calls: 1,
            persisted_receipt,
        })
    }

    pub fn heartbeat_attempt(
        &self,
        decision_identity: &str,
        attempt_identity: &str,
        fence_token: i64,
        heartbeat_at: DateTime<Utc>,
    ) -> Result<bool> {
        let lease_expires_at = heartbeat_at + Duration::seconds(self.config.attempt_lease_secs);
        self.with_immediate_transaction(|transaction| {
            let changed = transaction.execute(
                "UPDATE delivery_attempts
                 SET lease_expires_at=?1,lease_heartbeat_at=?2
                 WHERE attempt_identity=?3 AND decision_identity=?4
                   AND owner_instance_identity=?5 AND fence_token=?6
                   AND state='AttemptInFlight'
                   AND EXISTS(
                     SELECT 1 FROM delivery_decisions d
                     WHERE d.decision_identity=?4 AND d.state='AttemptInFlight'
                       AND d.current_attempt_identity=?3 AND d.fence_generation=?6
                   )",
                params![
                    timestamp(lease_expires_at),
                    timestamp(heartbeat_at),
                    attempt_identity,
                    decision_identity,
                    self.config.owner_instance_identity,
                    fence_token
                ],
            )?;
            if changed == 1 {
                record_attempt_event(
                    transaction,
                    attempt_identity,
                    decision_identity,
                    "LeaseHeartbeat",
                    canonical_json(&json!({
                        "lease_expires_at": timestamp(lease_expires_at),
                        "fence_token": fence_token,
                    }))?,
                    heartbeat_at,
                )?;
            }
            Ok(changed == 1)
        })
    }

    pub fn inspect_pending_for_date(&self, business_date: &str) -> Result<Vec<String>> {
        super::model::validate_business_date(business_date)?;
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT decision_identity FROM delivery_decisions
                 WHERE business_date=?1 AND state NOT IN
                   ('Delivered','RejectedDurable','ManualResolvedRejected')
                 ORDER BY decision_identity",
            )?;
            let rows = statement.query_map([business_date], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn reconcile_all_pending(
        &self,
        append_port: &dyn ImmutableAppendPort,
        now: DateTime<Utc>,
    ) -> Result<ReconcileSummary> {
        let mut progress_count = 0usize;
        loop {
            let mut progressed = false;
            while self.recover_one_expired_attempt(now)? {
                progress_count += 1;
                progressed = true;
            }
            while self.append_one_audit(append_port)? {
                progress_count += 1;
                progressed = true;
            }
            if self.has_blocked_audit_predecessor()? {
                return Err(DurableDeliveryError::AuditPredecessorBlocked);
            }
            while self.progress_one_pending_payload(append_port, now)? {
                progress_count += 1;
                progressed = true;
                while self.append_one_audit(append_port)? {
                    progress_count += 1;
                }
            }
            if !progressed {
                break;
            }
        }
        self.build_reconcile_summary(progress_count, now)
    }

    pub fn resolve_uncertain(
        &self,
        command: &ManualResolutionCommand,
        append_port: &dyn ImmutableAppendPort,
    ) -> Result<DecisionState> {
        validate_manual_command(command)?;
        self.with_connection(|connection| {
            let stored =
                load_decision(connection, &command.decision_identity)?.ok_or_else(|| {
                    DurableDeliveryError::DecisionNotFound(command.decision_identity.clone())
                })?;
            if stored.state != DecisionState::UncertainManualReview {
                return Err(DurableDeliveryError::InvalidManualResolution(format!(
                    "decision state is {}, expected UncertainManualReview",
                    stored.state
                )));
            }
            if stored.current_attempt_identity.is_none() {
                return Err(DurableDeliveryError::InvalidManualResolution(
                    "uncertain decision has no original attempt".to_owned(),
                ));
            }
            Ok(())
        })?;

        let evidence_sha256 = sha256_hex(&command.external_evidence);
        let disposition_name = match &command.disposition {
            ManualDisposition::Accepted { .. } => "Accepted",
            ManualDisposition::Rejected => "Rejected",
        };
        let resolution_identity = stable_identity(
            "delivery-manual-resolution-v1",
            &[
                &command.decision_identity,
                disposition_name,
                &command.operator_identity,
                &evidence_sha256,
            ],
        );
        let authorization = ManualResolutionAuthorizationCanonical {
            resolution_identity: resolution_identity.clone(),
            decision_identity: command.decision_identity.clone(),
            disposition: disposition_name.to_owned(),
            operator_identity: command.operator_identity.clone(),
            reason: command.reason.clone(),
            evidence_sha256: evidence_sha256.clone(),
            resolved_at: timestamp(command.resolved_at),
        }
        .canonical_bytes()?;
        let authorization_hash = sha256_hex(&authorization);
        let authorization_ref = require_nonempty_immutable_ref(
            append_port.append_exact(
                "ManualResolutionAuthorization",
                &resolution_identity,
                &authorization,
                &authorization_hash,
            )?,
            "ManualResolutionAuthorization",
            &resolution_identity,
        )?;

        self.with_immediate_transaction(|transaction| {
            let stored =
                load_decision(transaction, &command.decision_identity)?.ok_or_else(|| {
                    DurableDeliveryError::DecisionNotFound(command.decision_identity.clone())
                })?;
            if stored.state != DecisionState::UncertainManualReview {
                return Err(DurableDeliveryError::InvalidManualResolution(format!(
                    "decision state is {}, expected UncertainManualReview",
                    stored.state
                )));
            }
            let attempt_identity = stored.current_attempt_identity.clone().ok_or_else(|| {
                DurableDeliveryError::InvalidManualResolution(
                    "uncertain decision has no original attempt".to_owned(),
                )
            })?;
            let (disposition, receipt_canonical, frozen_delivery_audit) = match &command.disposition
            {
                ManualDisposition::Accepted { receipt } => {
                    if let Some(receipt) = receipt {
                        receipt.validate()?;
                    }
                    (
                        "Accepted",
                        receipt.as_ref().map(serde_json::to_vec).transpose()?,
                        Some(canonical_json(&json!({
                            "decision_identity": command.decision_identity,
                            "resolution_identity": resolution_identity,
                            "attempt_identity": attempt_identity,
                            "operator_identity": command.operator_identity,
                            "reason": command.reason,
                            "acceptance_evidence_sha256": evidence_sha256,
                            "authorization_sha256": authorization_hash,
                            "authorization_ref": authorization_ref,
                            "receipt_sha256": receipt
                                .as_ref()
                                .map(serde_json::to_vec)
                                .transpose()?
                                .map(|bytes| sha256_hex(&bytes)),
                            "resolved_at": timestamp(command.resolved_at),
                        }))?),
                    )
                }
                ManualDisposition::Rejected => ("Rejected", None, None),
            };
            let frozen_delivery_audit_sha256 = frozen_delivery_audit
                .as_ref()
                .map(|bytes| sha256_hex(bytes));
            let accepted_audit_identity = frozen_delivery_audit_sha256.as_ref().map(|audit_hash| {
                stable_identity(
                    MANUAL_ACCEPTED_DELIVERY_AUDIT_DOMAIN,
                    &[&resolution_identity, audit_hash],
                )
            });
            let accepted_audit_append_state = accepted_audit_identity.as_ref().map(|_| "Pending");
            transaction.execute(
                "INSERT INTO manual_resolutions(
               resolution_identity,decision_identity,attempt_identity,disposition,
               operator_identity,reason,evidence_canonical,evidence_sha256,
               receipt_canonical,frozen_delivery_audit_canonical,
               frozen_delivery_audit_sha256,immutable_audit_ref,
               accepted_audit_identity,accepted_audit_append_state,
               accepted_audit_ref,resolved_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,NULL,?15)",
                params![
                    resolution_identity,
                    command.decision_identity,
                    attempt_identity,
                    disposition,
                    command.operator_identity,
                    command.reason,
                    command.external_evidence,
                    evidence_sha256,
                    receipt_canonical,
                    frozen_delivery_audit,
                    frozen_delivery_audit_sha256,
                    authorization_ref,
                    accepted_audit_identity,
                    accepted_audit_append_state,
                    timestamp(command.resolved_at)
                ],
            )?;
            let envelope = parse_envelope(&stored.envelope_canonical)?;
            match &command.disposition {
                ManualDisposition::Accepted { .. } => {
                    freeze_disposition(
                        transaction,
                        &envelope,
                        None,
                        Some(&resolution_identity),
                        None,
                        "ManualAccepted",
                        &evidence_sha256,
                        false,
                        command.resolved_at,
                    )?;
                    mutate_reservations(
                        transaction,
                        &stored,
                        "Accepted",
                        command.resolved_at,
                        Some(command.resolved_at),
                    )?;
                    transition_existing_state(
                        transaction,
                        &stored,
                        DecisionState::AcceptedAuditPending,
                        "manual-resolution",
                        Some(&command.operator_identity),
                        canonical_json(&json!({
                            "resolution_identity": resolution_identity,
                            "authorization_ref": authorization_ref,
                        }))?,
                        command.resolved_at,
                    )?;
                }
                ManualDisposition::Rejected => {
                    freeze_disposition(
                        transaction,
                        &envelope,
                        None,
                        Some(&resolution_identity),
                        None,
                        "ManualRejected",
                        &evidence_sha256,
                        false,
                        command.resolved_at,
                    )?;
                    mutate_reservations(
                        transaction,
                        &stored,
                        "Released",
                        command.resolved_at,
                        None,
                    )?;
                    transition_existing_state(
                        transaction,
                        &stored,
                        DecisionState::ManualRejectedAuditPending,
                        "manual-resolution",
                        Some(&command.operator_identity),
                        canonical_json(&json!({
                            "resolution_identity": resolution_identity,
                            "authorization_ref": authorization_ref,
                        }))?,
                        command.resolved_at,
                    )?;
                }
            }
            Ok(())
        })?;
        self.with_connection(|connection| {
            load_decision(connection, &command.decision_identity)?
                .map(|stored| stored.state)
                .ok_or_else(|| {
                    DurableDeliveryError::DecisionNotFound(command.decision_identity.clone())
                })
        })
    }

    pub fn acknowledge_schedule_hydration(
        &self,
        transition_identity: &str,
        transition_sha256: &str,
        acknowledged_at: DateTime<Utc>,
    ) -> Result<bool> {
        if transition_identity.trim().is_empty() || transition_sha256.trim().is_empty() {
            return Err(DurableDeliveryError::PolicyMismatch(
                "hydration acknowledgement requires transition identity and hash".to_owned(),
            ));
        }
        self.with_immediate_transaction(|transaction| {
            let row: Option<(String, String, String, String)> = transaction
                .query_row(
                    "SELECT decision_identity,transition_sha256,append_state,hydration_state
                 FROM task_transition_payloads
                 WHERE transition_identity=?1",
                    [transition_identity],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((decision_identity, stored_sha256, append_state, hydration_state)) = row
            else {
                return Err(DurableDeliveryError::DecisionNotFound(format!(
                    "task transition {transition_identity}"
                )));
            };
            if stored_sha256 != transition_sha256 {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "task transition hash mismatch for {transition_identity}"
                )));
            }
            if append_state != "Appended" {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "task transition {transition_identity} is not durably appended"
                )));
            }
            if hydration_state == "Applied" {
                return Ok(false);
            }
            let acknowledgement_identity = stable_identity(
                "schedule-hydration-ack-v1",
                &[transition_identity, transition_sha256],
            );
            let evidence = canonical_json(&json!({
                "acknowledgement_identity": acknowledgement_identity,
                "decision_identity": decision_identity,
                "transition_identity": transition_identity,
                "transition_sha256": transition_sha256,
                "acknowledged_at": timestamp(acknowledged_at),
            }))?;
            let audit_identity = enqueue_audit(
                transaction,
                &decision_identity,
                None,
                "ScheduleHydrationApplied",
                &evidence,
                acknowledged_at,
            )?;
            let changed = transaction.execute(
                "UPDATE task_transition_payloads
                 SET hydration_state='Applied',hydration_ack_identity=?1,hydrated_at=?2
                 WHERE transition_identity=?3 AND transition_sha256=?4
                   AND append_state='Appended' AND hydration_state='Pending'",
                params![
                    audit_identity,
                    timestamp(acknowledged_at),
                    transition_identity,
                    transition_sha256
                ],
            )?;
            if changed != 1 {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "task transition hydration CAS failed for {transition_identity}"
                )));
            }
            Ok(true)
        })
    }

    /// Persist the scheduler's exact task-transition application and do not
    /// report success until its immutable audit outbox record is appended.
    ///
    /// The SQLite CAS and audit enqueue are atomic. The append itself is an
    /// external durable boundary, so an append failure deliberately leaves the
    /// exact audit row pending. A retry (including after process restart)
    /// finishes that same row without creating a second acknowledgement.
    pub fn persist_schedule_hydration_applied(
        &self,
        transition_identity: &str,
        transition_sha256: &str,
        append_port: &dyn ImmutableAppendPort,
        acknowledged_at: DateTime<Utc>,
    ) -> Result<()> {
        self.acknowledge_schedule_hydration(
            transition_identity,
            transition_sha256,
            acknowledged_at,
        )?;
        self.reconcile_all_pending(append_port, acknowledged_at)?;
        self.verify_schedule_hydration_applied(transition_identity, transition_sha256)
    }

    fn verify_schedule_hydration_applied(
        &self,
        transition_identity: &str,
        transition_sha256: &str,
    ) -> Result<()> {
        type HydrationVerificationRow = (String, String, String, Option<String>, Option<String>);

        self.with_connection(|connection| {
            let row: Option<HydrationVerificationRow> = connection
                .query_row(
                    "SELECT decision_identity,transition_sha256,hydration_state,
                        hydration_ack_identity,hydrated_at
                 FROM task_transition_payloads
                 WHERE transition_identity=?1",
                    [transition_identity],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()?;
            let Some((
                decision_identity,
                stored_sha256,
                hydration_state,
                acknowledgement_identity,
                hydrated_at,
            )) = row
            else {
                return Err(DurableDeliveryError::DecisionNotFound(format!(
                    "task transition {transition_identity}"
                )));
            };
            if stored_sha256 != transition_sha256 {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "task transition hash mismatch for {transition_identity}"
                )));
            }
            if hydration_state != "Applied" {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "task transition {transition_identity} is not applied"
                )));
            }
            let expected_acknowledgement_identity = stable_identity(
                "schedule-hydration-ack-v1",
                &[transition_identity, transition_sha256],
            );
            let hydrated_at = hydrated_at.ok_or_else(|| {
                DurableDeliveryError::PolicyMismatch(format!(
                    "task transition {transition_identity} has no hydration timestamp"
                ))
            })?;
            let evidence = canonical_json(&json!({
                "acknowledgement_identity": expected_acknowledgement_identity,
                "decision_identity": decision_identity,
                "transition_identity": transition_identity,
                "transition_sha256": transition_sha256,
                "acknowledged_at": hydrated_at,
            }))?;
            let evidence_sha256 = sha256_hex(&evidence);
            let expected_audit_identity = stable_identity(
                "delivery-critical-audit-v1",
                &[
                    &decision_identity,
                    "NONE",
                    "ScheduleHydrationApplied",
                    &evidence_sha256,
                ],
            );
            if acknowledgement_identity.as_deref() != Some(expected_audit_identity.as_str()) {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "task transition acknowledgement audit identity mismatch for {transition_identity}"
                )));
            }
            let audit: Option<(Vec<u8>, String, String, Option<String>)> = connection
                .query_row(
                    "SELECT audit_canonical,audit_sha256,append_state,immutable_audit_ref
                 FROM immutable_audit_outbox
                 WHERE audit_identity=?1 AND decision_identity=?2
                   AND audit_kind='ScheduleHydrationApplied'",
                    params![expected_audit_identity, decision_identity],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((stored_evidence, stored_evidence_sha256, append_state, immutable_audit_ref)) =
                audit
            else {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "task transition {transition_identity} has no exact hydration audit"
                )));
            };
            if stored_evidence != evidence
                || stored_evidence_sha256 != evidence_sha256
                || append_state != "Appended"
                || immutable_audit_ref
                    .as_deref()
                    .is_none_or(|value| !has_non_ascii_whitespace(value))
            {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "task transition {transition_identity} hydration audit is not durably confirmed"
                )));
            }
            Ok(())
        })
    }

    fn with_connection<T>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T>,
    ) -> Result<T> {
        // The global lease serializes process-fd attestation with coordinator
        // opens and retains one live SHM-owning Connection Arc throughout the
        // complete pre -> SQLite operation -> post boundary.
        let database_binding = self.database_binding()?;
        let operation_lease = database_binding.acquire_operation_lease()?;
        let mut connection = self.connection_handle()?.lock().map_err(|_| {
            DurableDeliveryError::InvalidConfiguration(
                "durable-delivery connection mutex is poisoned".to_owned(),
            )
        })?;
        let _locked_pre_lifetime = database_binding.validate_under_open_lock()?;
        #[cfg(test)]
        let outcome = match self.run_database_operation_test_hook(
            DatabaseOperationTestPhase::AfterPreValidationBeforeSql,
        ) {
            Ok(()) => operation(&mut connection),
            Err(error) => Err(error),
        };
        #[cfg(not(test))]
        let outcome = operation(&mut connection);
        let reference_post = outcome
            .as_ref()
            .map(|_| validate_persisted_immutable_references(&connection))
            .unwrap_or(Ok(()));
        let post = database_binding.validate_under_open_lock();
        drop(connection);
        drop(operation_lease);
        match (outcome, reference_post, post) {
            (Ok(value), Ok(()), Ok(_post_lifetime)) => Ok(value),
            (Err(primary), Ok(()), Ok(_post_lifetime)) => Err(primary),
            (Ok(_value), Err(reference_error), Ok(_post_lifetime)) => Err(reference_error),
            (Ok(_value), Ok(()), Err(post_error)) => Err(post_error),
            (Err(primary), Ok(()), Err(post_error)) => {
                Err(DurableDeliveryError::IsolationViolation(format!(
                    "database operation and post-operation isolation validation both failed; operation={primary}; post_validation={post_error}"
                )))
            }
            (outcome, reference_post, post) => {
                let operation_evidence = outcome
                    .err()
                    .map_or_else(|| "ok".to_owned(), |error| error.to_string());
                let reference_evidence = reference_post
                    .err()
                    .map_or_else(|| "ok".to_owned(), |error| error.to_string());
                let isolation_evidence = post
                    .err()
                    .map_or_else(|| "ok".to_owned(), |error| error.to_string());
                Err(DurableDeliveryError::IsolationViolation(format!(
                    "database operation post-validation failed; operation={operation_evidence}; immutable_references={reference_evidence}; isolation={isolation_evidence}"
                )))
            }
        }
    }

    fn with_immediate_transaction<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        self.with_connection(|connection| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let value = match operation(&transaction) {
                Ok(value) => value,
                Err(primary) => {
                    return Err(self.rollback_transaction_with_evidence(
                        &transaction,
                        "operation",
                        primary,
                    ));
                }
            };
            #[cfg(test)]
            if let Err(primary) = self.run_database_operation_test_hook(
                DatabaseOperationTestPhase::AfterSqlBeforePreCommitValidation,
            ) {
                return Err(self.rollback_transaction_with_evidence(
                    &transaction,
                    "after-sql test hook",
                    primary,
                ));
            }
            #[cfg(test)]
            if let Err(primary) = self.apply_operation_postvalidation_test_fault(&transaction) {
                return Err(self.rollback_transaction_with_evidence(
                    &transaction,
                    "operation postvalidation test fault",
                    primary,
                ));
            }
            if let Err(primary) = validate_persisted_immutable_references(&transaction) {
                return Err(self.rollback_transaction_with_evidence(
                    &transaction,
                    "pre-commit immutable-reference validation",
                    primary,
                ));
            }
            if let Err(primary) = self.database_binding()?.validate_under_open_lock() {
                return Err(self.rollback_transaction_with_evidence(
                    &transaction,
                    "pre-commit isolation validation",
                    primary,
                ));
            }
            #[cfg(test)]
            if take_commit_test_fault() {
                return Err(self.rollback_transaction_with_evidence(
                    &transaction,
                    "commit",
                    DurableDeliveryError::Sqlite(rusqlite::Error::InvalidQuery),
                ));
            }
            if let Err(error) = transaction.execute_batch("COMMIT") {
                return Err(self.rollback_transaction_with_evidence(
                    &transaction,
                    "commit",
                    DurableDeliveryError::from(error),
                ));
            }
            match self.database_binding()?.validate_under_open_lock() {
                Err(error) => Err(DurableDeliveryError::IsolationViolation(format!(
                    "post-commit isolation validation failed after COMMIT succeeded: {error}"
                ))),
                Ok(_post_commit_lifetime) => Ok(value),
            }
        })
    }

    #[cfg(test)]
    fn apply_operation_postvalidation_test_fault(
        &self,
        transaction: &Transaction<'_>,
    ) -> Result<()> {
        let fault = self
            .operation_postvalidation_test_fault
            .lock()
            .map_err(|_| {
                DurableDeliveryError::IsolationViolation(
                    "operation postvalidation test fault mutex is poisoned".to_owned(),
                )
            })?
            .take();
        let Some(fault) = fault else {
            return Ok(());
        };
        let whitespace = " \t\n\r";
        let changed = match fault {
            OperationPostvalidationTestFault::ImmutableAuditOutboxRef => transaction.execute(
                "UPDATE immutable_audit_outbox SET immutable_audit_ref=?1
                 WHERE audit_identity=(
                   SELECT audit_identity FROM immutable_audit_outbox
                   WHERE append_state='Appended' ORDER BY rowid LIMIT 1
                 )",
                [whitespace],
            )?,
            OperationPostvalidationTestFault::DeliveryDispositionRef => transaction.execute(
                "UPDATE delivery_disposition_payloads SET immutable_audit_ref=?1
                 WHERE disposition_identity=(
                   SELECT disposition_identity FROM delivery_disposition_payloads
                   WHERE append_state='Appended' ORDER BY rowid LIMIT 1
                 )",
                [whitespace],
            )?,
            OperationPostvalidationTestFault::TaskTransitionRef => transaction.execute(
                "UPDATE task_transition_payloads SET immutable_audit_ref=?1
                 WHERE transition_identity=(
                   SELECT transition_identity FROM task_transition_payloads
                   WHERE append_state='Appended' ORDER BY rowid LIMIT 1
                 )",
                [whitespace],
            )?,
            OperationPostvalidationTestFault::ManualResolutionRef => {
                transaction.execute_batch("DROP TRIGGER immutable_manual_resolution_update")?;
                transaction.execute(
                    "UPDATE manual_resolutions SET immutable_audit_ref=?1
                     WHERE resolution_identity=(
                       SELECT resolution_identity FROM manual_resolutions ORDER BY rowid LIMIT 1
                     )",
                    [whitespace],
                )?
            }
            OperationPostvalidationTestFault::SinkDeliveryAuditRef => transaction.execute(
                "UPDATE sink_results SET delivery_audit_ref=?1
                 WHERE result_event_identity=(
                   SELECT result_event_identity FROM sink_results
                   WHERE authoritative_for_state=1 AND result_kind='Accepted'
                   ORDER BY rowid LIMIT 1
                 )",
                [whitespace],
            )?,
            OperationPostvalidationTestFault::TaskHydrationState => {
                transaction.execute_batch("DROP TRIGGER task_transition_hydration_ack_cas")?;
                transaction.execute(
                    "UPDATE task_transition_payloads
                     SET hydration_state='Applied',
                         hydration_ack_identity='TEST_CODE_MISSING_HYDRATION_AUDIT',
                         hydrated_at='2026-07-30T08:00:00.000Z'
                     WHERE transition_identity=(
                       SELECT transition_identity FROM task_transition_payloads
                       WHERE hydration_state='Pending' ORDER BY rowid LIMIT 1
                     )",
                    [],
                )?
            }
        };
        if changed != 1 {
            return Err(DurableDeliveryError::InvalidConfiguration(format!(
                "TEST_CODE operation postvalidation fault {fault:?} affected {changed} rows; expected exactly one"
            )));
        }
        Ok(())
    }

    fn rollback_transaction_with_evidence(
        &self,
        transaction: &Transaction<'_>,
        phase: &str,
        primary: DurableDeliveryError,
    ) -> DurableDeliveryError {
        let rollback_error = transaction
            .execute_batch("ROLLBACK")
            .err()
            .map(DurableDeliveryError::from);
        #[cfg(test)]
        let rollback_error = rollback_error.or_else(|| {
            take_rollback_test_fault()
                .then_some(DurableDeliveryError::Sqlite(rusqlite::Error::InvalidQuery))
        });
        let post_rollback_error = self
            .database_binding()
            .and_then(|binding| binding.validate_under_open_lock().map(|_| ()))
            .err();
        match (rollback_error, post_rollback_error) {
            (None, None) => primary,
            (rollback_error, post_rollback_error) => {
                let rollback_evidence = rollback_error
                    .as_ref()
                    .map_or_else(|| "ok".to_owned(), ToString::to_string);
                let post_rollback_evidence = post_rollback_error
                    .as_ref()
                    .map_or_else(|| "ok".to_owned(), ToString::to_string);
                DurableDeliveryError::IsolationViolation(format!(
                    "transaction {phase} failed; primary={primary}; explicit_rollback={rollback_evidence}; post_rollback_validation={post_rollback_evidence}"
                ))
            }
        }
    }

    fn evaluate_prepare_denial(
        &self,
        transaction: &Transaction<'_>,
        envelope: &DeliveryEnvelope,
        policy: &PolicyRow,
        authoritative_sink_count: usize,
        admission_at: DateTime<Utc>,
    ) -> Result<Option<PrepareDenial>> {
        if envelope.policy_version != policy.policy_version
            || envelope.cooldown_scope != policy.cooldown_scope
        {
            return Ok(Some(PrepareDenial::InvalidPolicy(format!(
                "envelope policy/version differs from registered {}/{} policy",
                policy.push_kind, policy.sub_kind
            ))));
        }
        if authoritative_sink_count != 1 {
            return Ok(Some(PrepareDenial::InvalidSinkCardinality(
                authoritative_sink_count,
            )));
        }
        match policy.window_mode {
            WindowMode::None => {}
            WindowMode::Rolling => {
                if let Some((state, blocked_until)) = transaction
                    .query_row(
                        "SELECT state,blocked_until FROM cooldown_heads
                         WHERE push_kind=?1 AND sub_kind=?2
                           AND cooldown_scope=?3 AND scope_key=?4",
                        params![
                            envelope.push_kind.as_str(),
                            envelope.sub_kind.as_str(),
                            envelope.cooldown_scope.as_str(),
                            envelope.scope_key
                        ],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                    )
                    .optional()?
                {
                    let conflicts = match state.as_str() {
                        "Reserved" | "Uncertain" => true,
                        "Accepted" => blocked_until
                            .as_deref()
                            .map(parse_timestamp)
                            .transpose()?
                            .is_none_or(|until| until > admission_at),
                        "Released" => false,
                        other => {
                            return Err(DurableDeliveryError::PolicyMismatch(format!(
                                "invalid cooldown head state {other}"
                            )))
                        }
                    };
                    if conflicts {
                        return Ok(Some(PrepareDenial::CooldownConflict(format!(
                            "active {state} cooldown head"
                        ))));
                    }
                }
            }
            WindowMode::BusinessDateOnce => {
                if let Some(original) = transaction
                    .query_row(
                        "SELECT decision_identity FROM business_date_once_claims
                         WHERE business_date=?1 AND push_kind=?2
                           AND sub_kind=?3 AND scope_key=?4",
                        params![
                            envelope.business_date,
                            envelope.push_kind.as_str(),
                            envelope.sub_kind.as_str(),
                            envelope.scope_key
                        ],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
                {
                    return Ok(Some(PrepareDenial::BusinessDateClaimed(original)));
                }
            }
        }
        // BR-237: 复盘类 (counts_against_daily_budget=false) 豁免日预算,
        // 不被盘中信号烧满 30 槽后饿死 (8/13 复盘 7 路全失败事故)。
        if policy.counts_against_daily_budget
            && first_available_budget_slot(transaction, &envelope.business_date)?.is_none()
        {
            return Ok(Some(PrepareDenial::DailyBudgetFull));
        }
        Ok(None)
    }

    fn reserve_generation(
        &self,
        transaction: &Transaction<'_>,
        envelope: &DeliveryEnvelope,
        policy: &PolicyRow,
        generation: i64,
        reserved_at: DateTime<Utc>,
    ) -> Result<()> {
        if generation <= 0 {
            return Err(DurableDeliveryError::PolicyMismatch(
                "reservation generation must be positive".to_owned(),
            ));
        }
        if policy.window_mode == WindowMode::BusinessDateOnce {
            let existing = transaction
                .query_row(
                    "SELECT decision_identity FROM business_date_once_claims
                     WHERE business_date=?1 AND push_kind=?2 AND sub_kind=?3 AND scope_key=?4",
                    params![
                        envelope.business_date,
                        envelope.push_kind.as_str(),
                        envelope.sub_kind.as_str(),
                        envelope.scope_key
                    ],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            match existing {
                Some(identity) if identity == envelope.decision_identity => {}
                Some(identity) => {
                    return Err(DurableDeliveryError::PolicyMismatch(format!(
                        "BusinessDateOnce claim belongs to {identity}"
                    )))
                }
                None => {
                    let evidence = canonical_json(&json!({
                        "business_date": envelope.business_date,
                        "push_kind": envelope.push_kind,
                        "sub_kind": envelope.sub_kind,
                        "scope_key": envelope.scope_key,
                        "decision_identity": envelope.decision_identity,
                        "policy_version": policy.policy_version,
                    }))?;
                    let audit_identity = enqueue_audit(
                        transaction,
                        &envelope.decision_identity,
                        None,
                        "BusinessDateOnceClaimed",
                        &evidence,
                        reserved_at,
                    )?;
                    transaction.execute(
                        "INSERT INTO business_date_once_claims(
                           business_date,push_kind,sub_kind,scope_key,
                           decision_identity,policy_version,claimed_at,audit_identity
                         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                        params![
                            envelope.business_date,
                            envelope.push_kind.as_str(),
                            envelope.sub_kind.as_str(),
                            envelope.scope_key,
                            envelope.decision_identity,
                            policy.policy_version,
                            timestamp(reserved_at),
                            audit_identity
                        ],
                    )?;
                }
            }
        }

        let cooldown_identity = if policy.window_mode == WindowMode::None {
            None
        } else {
            let identity = stable_identity(
                "delivery-cooldown-reservation-v1",
                &[&envelope.decision_identity, &generation.to_string()],
            );
            transaction.execute(
                "INSERT INTO cooldown_reservations(
                   cooldown_reservation_identity,decision_identity,reservation_generation,
                   attempt_identity,business_date,push_kind,sub_kind,cooldown_scope,
                   scope_key,policy_version,effective_cooldown_secs,window_mode,
                   reserved_at,accepted_at,blocked_until,released_at,state
                 ) VALUES (?1,?2,?3,NULL,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL,NULL,NULL,'Reserved')",
                params![
                    identity,
                    envelope.decision_identity,
                    generation,
                    envelope.business_date,
                    envelope.push_kind.as_str(),
                    envelope.sub_kind.as_str(),
                    envelope.cooldown_scope.as_str(),
                    envelope.scope_key,
                    policy.policy_version,
                    policy.effective_cooldown_secs(),
                    policy.window_mode.as_str(),
                    timestamp(reserved_at),
                ],
            )?;
            record_cooldown_event(
                transaction,
                &identity,
                &envelope.decision_identity,
                None,
                "Reserved",
                canonical_json(&json!({
                    "reservation_generation": generation,
                    "window_mode": policy.window_mode.as_str(),
                }))?,
                reserved_at,
            )?;
            if policy.window_mode == WindowMode::Rolling {
                transaction.execute(
                    "INSERT INTO cooldown_heads(
                       push_kind,sub_kind,cooldown_scope,scope_key,
                       current_reservation_identity,state,blocked_until,version
                     ) VALUES (?1,?2,?3,?4,?5,'Reserved',NULL,1)
                     ON CONFLICT(push_kind,sub_kind,cooldown_scope,scope_key)
                     DO UPDATE SET current_reservation_identity=excluded.current_reservation_identity,
                       state='Reserved',blocked_until=NULL,version=cooldown_heads.version+1",
                    params![
                        envelope.push_kind.as_str(),
                        envelope.sub_kind.as_str(),
                        envelope.cooldown_scope.as_str(),
                        envelope.scope_key,
                        identity,
                    ],
                )?;
            }
            Some(identity)
        };

        // BR-237: 复盘类 (counts_against_daily_budget=false) 不占预算槽,
        // current_budget_reservation_identity=NULL, release 路径已容忍 None。
        let budget_identity = if policy.counts_against_daily_budget {
            let slot_no = first_available_budget_slot(transaction, &envelope.business_date)?
                .ok_or_else(|| {
                    DurableDeliveryError::PolicyMismatch(
                        "daily budget became full inside reservation transaction".to_owned(),
                    )
                })?;
            let budget_identity = stable_identity(
                "delivery-budget-reservation-v1",
                &[&envelope.decision_identity, &generation.to_string()],
            );
            transaction.execute(
                "INSERT INTO daily_budget_reservations(
                   budget_reservation_identity,decision_identity,reservation_generation,
                   attempt_identity,business_date,slot_no,reserved_at,accepted_at,released_at,state
                 ) VALUES (?1,?2,?3,NULL,?4,?5,?6,NULL,NULL,'Reserved')",
                params![
                    budget_identity,
                    envelope.decision_identity,
                    generation,
                    envelope.business_date,
                    slot_no,
                    timestamp(reserved_at)
                ],
            )?;
            record_budget_event(
                transaction,
                &budget_identity,
                &envelope.decision_identity,
                None,
                "Reserved",
                canonical_json(&json!({
                    "reservation_generation": generation,
                    "slot_no": slot_no,
                }))?,
                reserved_at,
            )?;
            Some(budget_identity)
        } else {
            None
        };
        transaction.execute(
            "UPDATE delivery_decisions SET reservation_generation=?1,
               current_budget_reservation_identity=?2,
               current_cooldown_reservation_identity=?3,updated_at=?4
             WHERE decision_identity=?5",
            params![
                generation,
                budget_identity,
                cooldown_identity,
                timestamp(reserved_at),
                envelope.decision_identity
            ],
        )?;
        Ok(())
    }

    fn reacquire_rejected(&self, stored: &StoredDecision, now: DateTime<Utc>) -> Result<bool> {
        let envelope = parse_envelope(&stored.envelope_canonical)?;
        self.with_immediate_transaction(|transaction| {
            let current =
                load_decision(transaction, &stored.decision_identity)?.ok_or_else(|| {
                    DurableDeliveryError::DecisionNotFound(stored.decision_identity.clone())
                })?;
            if current.state != DecisionState::RejectedDurable || !current.retry_authorized {
                return Ok(false);
            }
            let policy = load_policy(transaction, envelope.push_kind, envelope.sub_kind)?;
            if policy.window_mode == WindowMode::BusinessDateOnce {
                let claim: Option<String> = transaction
                    .query_row(
                        "SELECT decision_identity FROM business_date_once_claims
                     WHERE business_date=?1 AND push_kind=?2 AND sub_kind=?3 AND scope_key=?4",
                        params![
                            envelope.business_date,
                            envelope.push_kind.as_str(),
                            envelope.sub_kind.as_str(),
                            envelope.scope_key
                        ],
                        |row| row.get(0),
                    )
                    .optional()?;
                if claim.as_deref() != Some(&stored.decision_identity) {
                    return Ok(false);
                }
            } else if policy.window_mode == WindowMode::Rolling
                && rolling_head_conflicts(transaction, &envelope, now)?
            {
                return Ok(false);
            }
            // BR-237: 豁免类 (counts_against_daily_budget=false) 重试不占预算。
            if policy.counts_against_daily_budget
                && first_available_budget_slot(transaction, &envelope.business_date)?.is_none()
            {
                return Ok(false);
            }
            let generation = current.reservation_generation + 1;
            self.reserve_generation(transaction, &envelope, &policy, generation, now)?;
            transition_existing_state(
                transaction,
                &current,
                DecisionState::Reserved,
                "authorized-retry",
                None,
                canonical_json(&json!({
                    "reservation_generation": generation,
                    "envelope_sha256": stored.envelope_sha256,
                }))?,
                now,
            )?;
            Ok(true)
        })
    }

    pub(crate) fn begin_attempt(
        &self,
        decision_identity: &str,
        authoritative_sink_count: usize,
        now: DateTime<Utc>,
    ) -> Result<Option<AttemptLease>> {
        self.with_immediate_transaction(|transaction| {
            let stored = load_decision(transaction, decision_identity)?.ok_or_else(|| {
                DurableDeliveryError::DecisionNotFound(decision_identity.to_owned())
            })?;
            if stored.state != DecisionState::Reserved {
                return Ok(None);
            }
            let envelope = parse_envelope(&stored.envelope_canonical)?;
            if authoritative_sink_count != 1 {
                let denial = PrepareDenial::InvalidSinkCardinality(authoritative_sink_count);
                let evidence = canonical_json(&denial.evidence())?;
                let evidence_hash = sha256_hex(&evidence);
                let denial_identity = stable_identity(
                    "delivery-pre-sink-denial-v1",
                    &[
                        decision_identity,
                        &stored.envelope_sha256,
                        &envelope.policy_version.to_string(),
                        denial.reason_code(),
                        &evidence_hash,
                    ],
                );
                freeze_disposition(
                    transaction,
                    &envelope,
                    None,
                    None,
                    Some(&denial_identity),
                    "Rejected",
                    &evidence_hash,
                    false,
                    now,
                )?;
                mutate_reservations(transaction, &stored, "Released", now, None)?;
                transition_existing_state(
                    transaction,
                    &stored,
                    DecisionState::RejectedAuditPending,
                    "attempt-preflight",
                    None,
                    evidence,
                    now,
                )?;
                return Ok(None);
            }
            let attempt_no: i64 = transaction.query_row(
                "SELECT COALESCE(MAX(attempt_no),0)+1 FROM delivery_attempts
             WHERE decision_identity=?1",
                [decision_identity],
                |row| row.get(0),
            )?;
            let fence_token = stored.fence_generation + 1;
            let attempt_identity = stable_identity(
                "delivery-attempt-v1",
                &[
                    decision_identity,
                    &attempt_no.to_string(),
                    &fence_token.to_string(),
                ],
            );
            let lease_expires_at = now + Duration::seconds(self.config.attempt_lease_secs);
            transaction.execute(
                "INSERT INTO delivery_attempts(
               attempt_identity,decision_identity,attempt_no,owner_instance_identity,
               fence_token,lease_expires_at,lease_heartbeat_at,fence_revoked_at,state,started_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL,'AttemptInFlight',?7)",
                params![
                    attempt_identity,
                    decision_identity,
                    attempt_no,
                    self.config.owner_instance_identity,
                    fence_token,
                    timestamp(lease_expires_at),
                    timestamp(now),
                ],
            )?;
            attach_attempt_to_reservations(
                transaction,
                &stored,
                &attempt_identity,
                attempt_no,
                now,
            )?;
            let changed = transaction.execute(
                "UPDATE delivery_decisions SET state='AttemptInFlight',
               current_attempt_identity=?1,fence_generation=?2,updated_at=?3
             WHERE decision_identity=?4 AND state='Reserved'",
                params![
                    attempt_identity,
                    fence_token,
                    timestamp(now),
                    decision_identity
                ],
            )?;
            if changed != 1 {
                return Err(DurableDeliveryError::IllegalTransition {
                    from: stored.state.to_string(),
                    to: DecisionState::AttemptInFlight.to_string(),
                });
            }
            record_state_transition(
                transaction,
                decision_identity,
                Some(DecisionState::Reserved),
                DecisionState::AttemptInFlight,
                "resume-deliverable",
                None,
                canonical_json(&json!({
                    "attempt_identity": attempt_identity,
                    "attempt_no": attempt_no,
                    "fence_token": fence_token,
                }))?,
                now,
            )?;
            record_attempt_event(
                transaction,
                &attempt_identity,
                decision_identity,
                "LeaseGranted",
                canonical_json(&json!({
                    "owner_instance_identity_hash": sha256_hex(
                        self.config.owner_instance_identity.as_bytes()
                    ),
                    "fence_token": fence_token,
                    "lease_expires_at": timestamp(lease_expires_at),
                }))?,
                now,
            )?;
            Ok(Some(AttemptLease {
                attempt_identity: attempt_identity.clone(),
                fence_token,
                request: AuthoritativeDeliveryRequest {
                    decision_identity: decision_identity.to_owned(),
                    attempt_identity,
                    fence_token,
                    push_kind: envelope.push_kind,
                    stable_template_id: envelope.push_kind.stable_template_id().to_owned(),
                    rendered_content: envelope.rendered_content,
                    rendered_content_sha256: envelope.rendered_content_sha256,
                },
            }))
        })
    }

    pub(crate) fn record_sink_result(
        &self,
        attempt_identity: &str,
        fence_token: i64,
        result: AuthoritativeSinkResult,
        recorded_at: DateTime<Utc>,
    ) -> Result<()> {
        self.with_immediate_transaction(|transaction| {
            let (decision_identity, attempt_state): (String, String) = transaction
                .query_row(
                    "SELECT decision_identity,state FROM delivery_attempts
                 WHERE attempt_identity=?1 AND fence_token=?2",
                    params![attempt_identity, fence_token],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| {
                    DurableDeliveryError::DecisionNotFound(format!(
                        "attempt {attempt_identity}/{fence_token}"
                    ))
                })?;
            let stored = load_decision(transaction, &decision_identity)?
                .ok_or_else(|| DurableDeliveryError::DecisionNotFound(decision_identity.clone()))?;
            let authoritative = stored.state == DecisionState::AttemptInFlight
                && stored.current_attempt_identity.as_deref() == Some(attempt_identity)
                && stored.fence_generation == fence_token
                && attempt_state == "AttemptInFlight";
            let late_after_fence = !authoritative;
            let result_canonical = canonical_sink_result(&result)?;
            let result_sha256 = sha256_hex(&result_canonical);
            let result_kind = sink_result_kind(&result);
            let result_event_identity = stable_identity(
                "delivery-sink-result-v1",
                &[
                    attempt_identity,
                    &fence_token.to_string(),
                    result_kind,
                    &result_sha256,
                ],
            );
            let authority_evidence = canonical_json(&json!({
                "attempt_identity": attempt_identity,
                "fence_token": fence_token,
                "result_sha256": result_sha256,
                "authoritative_for_state": authoritative,
            }))?;
            let authority_audit = enqueue_audit(
                transaction,
                &decision_identity,
                Some(attempt_identity),
                "SinkResultAuthorityClassified",
                &authority_evidence,
                recorded_at,
            )?;
            record_attempt_event_with_audit(
                transaction,
                attempt_identity,
                &decision_identity,
                "SinkResultAuthorityClassified",
                authority_evidence,
                &authority_audit,
            )?;
            let late_audit = if late_after_fence {
                let evidence = canonical_json(&json!({
                    "attempt_identity": attempt_identity,
                    "fence_token": fence_token,
                    "result_kind": result_kind,
                    "result_sha256": result_sha256,
                }))?;
                let audit = enqueue_audit(
                    transaction,
                    &decision_identity,
                    Some(attempt_identity),
                    "LateReceiptObserved",
                    &evidence,
                    recorded_at,
                )?;
                record_attempt_event_with_audit(
                    transaction,
                    attempt_identity,
                    &decision_identity,
                    "LateReceiptObserved",
                    evidence,
                    &audit,
                )?;
                Some(audit)
            } else {
                None
            };
            let receipt = match &result {
                AuthoritativeSinkResult::Accepted(receipt) => {
                    receipt.validate()?;
                    Some(receipt)
                }
                _ => None,
            };
            let frozen_delivery_audit = if authoritative {
                receipt
                    .map(|receipt| {
                        canonical_json(&json!({
                            "decision_identity": decision_identity,
                            "attempt_identity": attempt_identity,
                            "fence_token": fence_token,
                            "result_sha256": result_sha256,
                            "receipt": receipt,
                        }))
                    })
                    .transpose()?
            } else {
                None
            };
            transaction.execute(
                "INSERT INTO sink_results(
               result_event_identity,attempt_identity,decision_identity,result_kind,
               observed_at,fence_token,authoritative_for_state,late_after_fence,
               authority_audit_identity,late_receipt_audit_identity,result_canonical,
               result_sha256,channel,provider,message_id,platform_message_id,
               accepted_at,latency_ms,frozen_delivery_audit_canonical,
               frozen_delivery_audit_sha256,delivery_audit_ref
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,
                       ?15,?16,?17,?18,?19,?20,NULL)",
                params![
                    result_event_identity,
                    attempt_identity,
                    decision_identity,
                    result_kind,
                    timestamp(recorded_at),
                    fence_token,
                    authoritative as i64,
                    late_after_fence as i64,
                    authority_audit,
                    late_audit,
                    result_canonical,
                    result_sha256,
                    receipt.map(|value| value.channel.as_str()),
                    receipt.map(|value| value.provider.as_str()),
                    receipt.map(|value| value.message_id.as_str()),
                    receipt.and_then(|value| value.platform_message_id.as_deref()),
                    receipt.map(|value| timestamp(value.accepted_at)),
                    receipt.and_then(|value| value.latency_ms),
                    frozen_delivery_audit,
                    frozen_delivery_audit
                        .as_ref()
                        .map(|value| sha256_hex(value)),
                ],
            )?;
            if !authoritative {
                return Ok(());
            }

            let envelope = parse_envelope(&stored.envelope_canonical)?;
            let evidence_hash = result_evidence_hash(&result)?;
            match result {
                AuthoritativeSinkResult::Accepted(receipt) => {
                    transaction.execute(
                        "UPDATE delivery_attempts SET state='Accepted'
                     WHERE attempt_identity=?1 AND state='AttemptInFlight'",
                        [attempt_identity],
                    )?;
                    freeze_disposition(
                        transaction,
                        &envelope,
                        Some(attempt_identity),
                        None,
                        None,
                        "Accepted",
                        &evidence_hash,
                        false,
                        receipt.accepted_at,
                    )?;
                    mutate_reservations(
                        transaction,
                        &stored,
                        "Accepted",
                        receipt.accepted_at,
                        Some(receipt.accepted_at),
                    )?;
                    transition_existing_state(
                        transaction,
                        &stored,
                        DecisionState::AcceptedAuditPending,
                        "sink-result",
                        None,
                        canonical_json(&json!({
                            "attempt_identity": attempt_identity,
                            "result_sha256": result_sha256,
                            "fence_token": fence_token,
                        }))?,
                        recorded_at,
                    )?;
                }
                AuthoritativeSinkResult::Rejected(rejection) => {
                    transaction.execute(
                        "UPDATE delivery_attempts SET state='Rejected'
                     WHERE attempt_identity=?1 AND state='AttemptInFlight'",
                        [attempt_identity],
                    )?;
                    freeze_disposition(
                        transaction,
                        &envelope,
                        Some(attempt_identity),
                        None,
                        None,
                        "Rejected",
                        &evidence_hash,
                        rejection.retry_authorized,
                        rejection.observed_at,
                    )?;
                    transaction.execute(
                        "UPDATE delivery_decisions SET retry_authorized=?1
                     WHERE decision_identity=?2",
                        params![rejection.retry_authorized as i64, decision_identity],
                    )?;
                    mutate_reservations(
                        transaction,
                        &stored,
                        "Released",
                        rejection.observed_at,
                        None,
                    )?;
                    transition_existing_state(
                        transaction,
                        &stored,
                        DecisionState::RejectedAuditPending,
                        "sink-result",
                        None,
                        canonical_json(&json!({
                            "attempt_identity": attempt_identity,
                            "result_sha256": result_sha256,
                            "fence_token": fence_token,
                        }))?,
                        recorded_at,
                    )?;
                }
                AuthoritativeSinkResult::Uncertain(uncertainty) => {
                    transaction.execute(
                        "UPDATE delivery_attempts SET state='Uncertain'
                     WHERE attempt_identity=?1 AND state='AttemptInFlight'",
                        [attempt_identity],
                    )?;
                    freeze_disposition(
                        transaction,
                        &envelope,
                        Some(attempt_identity),
                        None,
                        None,
                        "Uncertain",
                        &evidence_hash,
                        false,
                        uncertainty.observed_at,
                    )?;
                    mutate_reservations(
                        transaction,
                        &stored,
                        "Uncertain",
                        uncertainty.observed_at,
                        None,
                    )?;
                    transition_existing_state(
                        transaction,
                        &stored,
                        DecisionState::UncertainAuditPending,
                        "sink-result",
                        None,
                        canonical_json(&json!({
                            "attempt_identity": attempt_identity,
                            "result_sha256": result_sha256,
                            "fence_token": fence_token,
                        }))?,
                        recorded_at,
                    )?;
                }
            }
            Ok(())
        })
    }

    fn has_persisted_receipt(&self, decision_identity: &str) -> Result<bool> {
        self.with_connection(|connection| {
            let count: i64 = connection.query_row(
                "SELECT COUNT(*) FROM sink_results
                 WHERE decision_identity=?1 AND result_kind='Accepted'
                   AND authoritative_for_state=1",
                [decision_identity],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })
    }

    fn recover_one_expired_attempt(&self, now: DateTime<Utc>) -> Result<bool> {
        self.with_immediate_transaction(|transaction| {
            let candidate: Option<(String, String, i64, String)> = transaction
                .query_row(
                    "SELECT a.attempt_identity,a.decision_identity,a.fence_token,a.lease_expires_at
                 FROM delivery_attempts a
                 JOIN delivery_decisions d
                   ON d.decision_identity=a.decision_identity
                  AND d.current_attempt_identity=a.attempt_identity
                 WHERE a.state='AttemptInFlight' AND d.state='AttemptInFlight'
                   AND a.lease_expires_at<=?1
                 ORDER BY d.business_date,d.decision_identity LIMIT 1",
                    [timestamp(now)],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((attempt_identity, decision_identity, fence_token, lease_expires_at)) =
                candidate
            else {
                return Ok(false);
            };
            let stored = load_decision(transaction, &decision_identity)?
                .ok_or_else(|| DurableDeliveryError::DecisionNotFound(decision_identity.clone()))?;
            let new_fence = stored.fence_generation + 1;
            let changed = transaction.execute(
                "UPDATE delivery_attempts SET state='Uncertain',fence_revoked_at=?1
             WHERE attempt_identity=?2 AND decision_identity=?3
               AND fence_token=?4 AND state='AttemptInFlight'
               AND lease_expires_at<=?1",
                params![
                    timestamp(now),
                    attempt_identity,
                    decision_identity,
                    fence_token
                ],
            )?;
            if changed != 1 {
                return Ok(false);
            }
            transaction.execute(
                "UPDATE delivery_decisions SET fence_generation=?1,updated_at=?2
             WHERE decision_identity=?3 AND state='AttemptInFlight'
               AND current_attempt_identity=?4 AND fence_generation=?5",
                params![
                    new_fence,
                    timestamp(now),
                    decision_identity,
                    attempt_identity,
                    fence_token
                ],
            )?;
            let fence_evidence = canonical_json(&json!({
                "attempt_identity": attempt_identity,
                "revoked_fence_token": fence_token,
                "replacement_fence_generation": new_fence,
                "lease_expires_at": lease_expires_at,
            }))?;
            record_attempt_event(
                transaction,
                &attempt_identity,
                &decision_identity,
                "FenceRevoked",
                fence_evidence.clone(),
                now,
            )?;
            record_attempt_event(
                transaction,
                &attempt_identity,
                &decision_identity,
                "RecoveryClassified",
                canonical_json(&json!({
                    "classification": "Uncertain",
                    "automatic_resend": false,
                    "persisted_receipt": false,
                    "fence_evidence_sha256": sha256_hex(&fence_evidence),
                }))?,
                now,
            )?;
            let envelope = parse_envelope(&stored.envelope_canonical)?;
            freeze_disposition(
                transaction,
                &envelope,
                Some(&attempt_identity),
                None,
                None,
                "Uncertain",
                &sha256_hex(&fence_evidence),
                false,
                now,
            )?;
            mutate_reservations(transaction, &stored, "Uncertain", now, None)?;
            transition_existing_state(
                transaction,
                &stored,
                DecisionState::UncertainAuditPending,
                "expired-attempt-recovery",
                None,
                canonical_json(&json!({
                    "attempt_identity": attempt_identity,
                    "revoked_fence_token": fence_token,
                    "persisted_receipt": false,
                }))?,
                now,
            )?;
            Ok(true)
        })
    }

    fn append_one_audit(&self, append_port: &dyn ImmutableAppendPort) -> Result<bool> {
        let pending = self.with_connection(|connection| {
            Ok(connection
                .query_row(
                    "SELECT o.audit_kind,o.audit_identity,o.audit_canonical,o.audit_sha256,
                            o.decision_identity
                     FROM immutable_audit_outbox o
                     JOIN delivery_decisions d ON d.decision_identity=o.decision_identity
                     LEFT JOIN immutable_audit_outbox predecessor
                       ON predecessor.audit_identity=o.predecessor_audit_identity
                     WHERE o.append_state='Pending'
                       AND (o.predecessor_audit_identity IS NULL
                            OR predecessor.append_state='Appended')
                     ORDER BY d.business_date,d.decision_identity,o.rowid LIMIT 1",
                    [],
                    |row| {
                        Ok(PendingAppend {
                            record_kind: row.get(0)?,
                            identity: row.get(1)?,
                            canonical: row.get(2)?,
                            sha256: row.get(3)?,
                            decision_identity: row.get(4)?,
                        })
                    },
                )
                .optional()?)
        })?;
        let Some(pending) = pending else {
            return Ok(false);
        };
        let immutable_ref = require_nonempty_immutable_ref(
            append_port.append_exact(
                &pending.record_kind,
                &pending.identity,
                &pending.canonical,
                &pending.sha256,
            )?,
            &pending.record_kind,
            &pending.identity,
        )?;
        self.with_immediate_transaction(|transaction| {
            let changed = transaction.execute(
                "UPDATE immutable_audit_outbox
                 SET append_state='Appended',immutable_audit_ref=?1
                 WHERE audit_identity=?2 AND decision_identity=?3
                   AND append_state='Pending' AND audit_sha256=?4",
                params![
                    immutable_ref,
                    pending.identity,
                    pending.decision_identity,
                    pending.sha256
                ],
            )?;
            require_single_cas_update(changed, "immutable audit append acknowledgement")
        })?;
        Ok(true)
    }

    fn has_blocked_audit_predecessor(&self) -> Result<bool> {
        self.with_connection(|connection| {
            let pending: i64 = connection.query_row(
                "SELECT COUNT(*) FROM immutable_audit_outbox WHERE append_state='Pending'",
                [],
                |row| row.get(0),
            )?;
            if pending == 0 {
                return Ok(false);
            }
            let ready: i64 = connection.query_row(
                "SELECT COUNT(*)
             FROM immutable_audit_outbox o
             LEFT JOIN immutable_audit_outbox predecessor
               ON predecessor.audit_identity=o.predecessor_audit_identity
             WHERE o.append_state='Pending'
               AND (o.predecessor_audit_identity IS NULL
                    OR predecessor.append_state='Appended')",
                [],
                |row| row.get(0),
            )?;
            Ok(ready == 0)
        })
    }

    fn progress_one_pending_payload(
        &self,
        append_port: &dyn ImmutableAppendPort,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let candidate = self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT decision_identity FROM delivery_decisions
                 WHERE state IN (
                   'AcceptedAuditPending','AcceptedTaskTransitionPending',
                   'RejectedAuditPending','RejectedTaskTransitionPending',
                   'UncertainAuditPending','UncertainTaskTransitionPending',
                   'ManualRejectedAuditPending','ManualRejectedTaskTransitionPending')
                 ORDER BY business_date,decision_identity LIMIT 1",
            )?;
            Ok(statement
                .query_row([], |row| row.get::<_, String>(0))
                .optional()?)
        })?;
        let Some(decision_identity) = candidate else {
            return Ok(false);
        };
        if self.decision_has_pending_audit(&decision_identity)? {
            return Ok(false);
        }
        let stored = self.with_connection(|connection| {
            load_decision(connection, &decision_identity)?
                .ok_or_else(|| DurableDeliveryError::DecisionNotFound(decision_identity.clone()))
        })?;

        use DecisionState::*;
        match stored.state {
            AcceptedAuditPending
            | RejectedAuditPending
            | UncertainAuditPending
            | ManualRejectedAuditPending => {
                self.append_current_disposition(&stored, append_port)?;
                if stored.state == AcceptedAuditPending {
                    self.append_delivery_audit(&stored, append_port)?;
                }
                let refreshed = self.with_connection(|connection| {
                    load_decision(connection, &decision_identity)?.ok_or_else(|| {
                        DurableDeliveryError::DecisionNotFound(decision_identity.clone())
                    })
                })?;
                if !self.disposition_and_delivery_are_appended(&refreshed)? {
                    return Ok(true);
                }
                let target = match (stored.state, stored.task_binding_present) {
                    (AcceptedAuditPending, true) => AcceptedTaskTransitionPending,
                    (AcceptedAuditPending, false) => Delivered,
                    (RejectedAuditPending, true) => RejectedTaskTransitionPending,
                    (RejectedAuditPending, false) => RejectedDurable,
                    (UncertainAuditPending, true) => UncertainTaskTransitionPending,
                    (UncertainAuditPending, false) => UncertainManualReview,
                    (ManualRejectedAuditPending, true) => ManualRejectedTaskTransitionPending,
                    (ManualRejectedAuditPending, false) => ManualResolvedRejected,
                    _ => unreachable!("state constrained by match"),
                };
                self.transition_for_reconcile(&refreshed, target, now)?;
                Ok(true)
            }
            AcceptedTaskTransitionPending
            | RejectedTaskTransitionPending
            | UncertainTaskTransitionPending
            | ManualRejectedTaskTransitionPending => {
                self.append_current_task_transition(&stored, append_port)?;
                let target = match stored.state {
                    AcceptedTaskTransitionPending => Delivered,
                    RejectedTaskTransitionPending => RejectedDurable,
                    UncertainTaskTransitionPending => UncertainManualReview,
                    ManualRejectedTaskTransitionPending => ManualResolvedRejected,
                    _ => unreachable!("state constrained by match"),
                };
                let refreshed = self.with_connection(|connection| {
                    load_decision(connection, &decision_identity)?.ok_or_else(|| {
                        DurableDeliveryError::DecisionNotFound(decision_identity.clone())
                    })
                })?;
                if target == Delivered && !self.disposition_and_delivery_are_appended(&refreshed)? {
                    return Err(DurableDeliveryError::PolicyMismatch(
                        "accepted delivery cannot reach Delivered without complete current evidence"
                            .to_owned(),
                    ));
                }
                self.transition_for_reconcile(&refreshed, target, now)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn decision_has_pending_audit(&self, decision_identity: &str) -> Result<bool> {
        self.with_connection(|connection| {
            let count: i64 = connection.query_row(
                "SELECT COUNT(*) FROM immutable_audit_outbox
             WHERE decision_identity=?1 AND append_state='Pending'",
                [decision_identity],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })
    }

    fn append_current_disposition(
        &self,
        stored: &StoredDecision,
        append_port: &dyn ImmutableAppendPort,
    ) -> Result<()> {
        let identity = stored
            .current_disposition_identity
            .as_deref()
            .ok_or_else(|| {
                DurableDeliveryError::PolicyMismatch(format!(
                    "{} has no frozen generic disposition",
                    stored.decision_identity
                ))
            })?;
        let pending = self.with_connection(|connection| {
            Ok(connection
                .query_row(
                    "SELECT disposition_canonical,disposition_sha256
                     FROM delivery_disposition_payloads
                     WHERE disposition_identity=?1 AND decision_identity=?2
                       AND append_state='Pending'",
                    params![identity, stored.decision_identity],
                    |row| {
                        Ok(PendingAppend {
                            record_kind: "DeliveryDisposition".to_owned(),
                            identity: identity.to_owned(),
                            canonical: row.get(0)?,
                            sha256: row.get(1)?,
                            decision_identity: stored.decision_identity.clone(),
                        })
                    },
                )
                .optional()?)
        })?;
        let Some(pending) = pending else {
            return Ok(());
        };
        let immutable_ref = require_nonempty_immutable_ref(
            append_port.append_exact(
                &pending.record_kind,
                &pending.identity,
                &pending.canonical,
                &pending.sha256,
            )?,
            &pending.record_kind,
            &pending.identity,
        )?;
        self.with_immediate_transaction(|transaction| {
            let changed = transaction.execute(
                "UPDATE delivery_disposition_payloads
                 SET append_state='Appended',immutable_audit_ref=?1
                 WHERE disposition_identity=?2 AND append_state='Pending'
                   AND disposition_sha256=?3",
                params![immutable_ref, pending.identity, pending.sha256],
            )?;
            require_single_cas_update(changed, "delivery disposition append acknowledgement")
        })
    }

    fn append_delivery_audit(
        &self,
        stored: &StoredDecision,
        append_port: &dyn ImmutableAppendPort,
    ) -> Result<()> {
        let pending = self.with_connection(|connection| {
            Ok(connection
                .query_row(
                    "SELECT result_event_identity,frozen_delivery_audit_canonical,
                            frozen_delivery_audit_sha256
                     FROM sink_results
                     WHERE decision_identity=?1 AND authoritative_for_state=1
                       AND result_kind='Accepted' AND delivery_audit_ref IS NULL
                     ORDER BY rowid DESC LIMIT 1",
                    [stored.decision_identity.as_str()],
                    |row| {
                        Ok(PendingAppend {
                            record_kind: "DeliveryAcceptedAudit".to_owned(),
                            identity: row.get(0)?,
                            canonical: row.get(1)?,
                            sha256: row.get(2)?,
                            decision_identity: stored.decision_identity.clone(),
                        })
                    },
                )
                .optional()?)
        })?;
        let Some(pending) = pending else {
            let manual_pending = self.with_connection(|connection| {
                Ok(connection
                    .query_row(
                        "SELECT accepted_audit_identity,frozen_delivery_audit_canonical,
                                frozen_delivery_audit_sha256
                         FROM manual_resolutions
                         WHERE decision_identity=?1 AND disposition='Accepted'
                           AND accepted_audit_append_state='Pending'
                           AND accepted_audit_ref IS NULL
                           AND accepted_audit_identity IS NOT NULL
                           AND frozen_delivery_audit_canonical IS NOT NULL",
                        [stored.decision_identity.as_str()],
                        |row| {
                            Ok(PendingAppend {
                                record_kind: "DeliveryAcceptedAudit".to_owned(),
                                identity: row.get(0)?,
                                canonical: row.get(1)?,
                                sha256: row.get(2)?,
                                decision_identity: stored.decision_identity.clone(),
                            })
                        },
                    )
                    .optional()?)
            })?;
            if let Some(manual) = manual_pending {
                let manual_evidence = self.with_connection(|connection| {
                    load_manual_accepted_delivery_evidence(connection, &stored.decision_identity)
                })?;
                manual_evidence.validate_for_migration()?;
                let authorization_canonical = manual_evidence.authorization_canonical()?;
                let authorization_sha256 = sha256_hex(&authorization_canonical);
                let authorization_ref = require_nonempty_immutable_ref(
                    append_port.append_exact(
                        "ManualResolutionAuthorization",
                        &manual_evidence.resolution_identity,
                        &authorization_canonical,
                        &authorization_sha256,
                    )?,
                    "ManualResolutionAuthorization",
                    &manual_evidence.resolution_identity,
                )?;
                if authorization_ref != manual_evidence.authorization_immutable_audit_ref {
                    return Err(DurableDeliveryError::ImmutableAppendConflict(
                        manual_evidence.resolution_identity,
                    ));
                }
                let immutable_ref = require_nonempty_immutable_ref(
                    append_port.append_exact(
                        &manual.record_kind,
                        &manual.identity,
                        &manual.canonical,
                        &manual.sha256,
                    )?,
                    &manual.record_kind,
                    &manual.identity,
                )?;
                self.with_immediate_transaction(|transaction| {
                    let changed = transaction.execute(
                        "UPDATE manual_resolutions
                         SET accepted_audit_append_state='Appended',accepted_audit_ref=?1
                         WHERE accepted_audit_identity=?2
                           AND decision_identity=?3
                           AND accepted_audit_append_state='Pending'
                           AND accepted_audit_ref IS NULL
                           AND frozen_delivery_audit_sha256=?4",
                        params![
                            immutable_ref,
                            manual.identity,
                            manual.decision_identity,
                            manual.sha256
                        ],
                    )?;
                    require_single_cas_update(
                        changed,
                        "manual accepted delivery audit acknowledgement",
                    )
                })?;
            }
            return Ok(());
        };
        let immutable_ref = require_nonempty_immutable_ref(
            append_port.append_exact(
                &pending.record_kind,
                &pending.identity,
                &pending.canonical,
                &pending.sha256,
            )?,
            &pending.record_kind,
            &pending.identity,
        )?;
        self.with_immediate_transaction(|transaction| {
            let changed = transaction.execute(
                "UPDATE sink_results SET delivery_audit_ref=?1
                 WHERE result_event_identity=?2 AND delivery_audit_ref IS NULL
                   AND frozen_delivery_audit_sha256=?3",
                params![immutable_ref, pending.identity, pending.sha256],
            )?;
            require_single_cas_update(changed, "accepted delivery audit acknowledgement")
        })
    }

    fn append_current_task_transition(
        &self,
        stored: &StoredDecision,
        append_port: &dyn ImmutableAppendPort,
    ) -> Result<()> {
        let disposition = stored
            .current_disposition_identity
            .as_deref()
            .ok_or_else(|| {
                DurableDeliveryError::PolicyMismatch(
                    "task transition has no current disposition".to_owned(),
                )
            })?;
        let pending = self.with_connection(|connection| {
            Ok(connection
                .query_row(
                    "SELECT transition_identity,transition_canonical,transition_sha256
                     FROM task_transition_payloads
                     WHERE decision_identity=?1 AND disposition_identity=?2
                       AND append_state='Pending'",
                    params![stored.decision_identity, disposition],
                    |row| {
                        Ok(PendingAppend {
                            record_kind: "BR-140TaskTransition".to_owned(),
                            identity: row.get(0)?,
                            canonical: row.get(1)?,
                            sha256: row.get(2)?,
                            decision_identity: stored.decision_identity.clone(),
                        })
                    },
                )
                .optional()?)
        })?;
        let Some(pending) = pending else {
            return Ok(());
        };
        let immutable_ref = require_nonempty_immutable_ref(
            append_port.append_exact(
                &pending.record_kind,
                &pending.identity,
                &pending.canonical,
                &pending.sha256,
            )?,
            &pending.record_kind,
            &pending.identity,
        )?;
        self.with_immediate_transaction(|transaction| {
            let changed = transaction.execute(
                "UPDATE task_transition_payloads
                 SET append_state='Appended',immutable_audit_ref=?1
                 WHERE transition_identity=?2 AND append_state='Pending'
                   AND transition_sha256=?3",
                params![immutable_ref, pending.identity, pending.sha256],
            )?;
            require_single_cas_update(changed, "task transition append acknowledgement")
        })
    }

    fn disposition_and_delivery_are_appended(&self, stored: &StoredDecision) -> Result<bool> {
        self.with_connection(|connection| {
            let disposition_appended: i64 = connection.query_row(
                "SELECT COUNT(*) FROM delivery_disposition_payloads
             WHERE disposition_identity=?1 AND append_state='Appended'",
                [stored.current_disposition_identity.as_deref().unwrap_or("")],
                |row| row.get(0),
            )?;
            if disposition_appended != 1 {
                return Ok(false);
            }
            if matches!(
                stored.state,
                DecisionState::AcceptedAuditPending
                    | DecisionState::AcceptedTaskTransitionPending
                    | DecisionState::Delivered
            ) {
                let authoritative_accepts: i64 = connection.query_row(
                    "SELECT COUNT(*) FROM sink_results
                 WHERE decision_identity=?1 AND authoritative_for_state=1
                   AND result_kind='Accepted'",
                    [stored.decision_identity.as_str()],
                    |row| row.get(0),
                )?;
                if authoritative_accepts > 0 {
                    let appended: i64 = connection.query_row(
                        "SELECT COUNT(*) FROM sink_results
                     WHERE decision_identity=?1 AND authoritative_for_state=1
                       AND result_kind='Accepted' AND delivery_audit_ref IS NOT NULL",
                        [stored.decision_identity.as_str()],
                        |row| row.get(0),
                    )?;
                    return Ok(appended == authoritative_accepts);
                }
                let manual_accepts: i64 = connection.query_row(
                    "SELECT COUNT(*) FROM manual_resolutions
                     WHERE decision_identity=?1 AND disposition='Accepted'",
                    [stored.decision_identity.as_str()],
                    |row| row.get(0),
                )?;
                if manual_accepts != 1 {
                    return Ok(false);
                }
                load_and_validate_manual_accepted_delivery_evidence(
                    connection,
                    &stored.decision_identity,
                )?;
                return Ok(true);
            }
            Ok(true)
        })
    }

    fn transition_for_reconcile(
        &self,
        stored: &StoredDecision,
        target: DecisionState,
        now: DateTime<Utc>,
    ) -> Result<()> {
        #[cfg(test)]
        if target == DecisionState::Delivered {
            self.run_delivered_reconcile_test_hook()?;
        }
        self.with_immediate_transaction(|transaction| {
            let current =
                load_decision(transaction, &stored.decision_identity)?.ok_or_else(|| {
                    DurableDeliveryError::DecisionNotFound(stored.decision_identity.clone())
                })?;
            if current.state == target || current.state != stored.state {
                return Ok(());
            }
            if target == DecisionState::Delivered {
                #[cfg(test)]
                self.apply_delivered_precommit_test_fault(transaction, &current.decision_identity)?;
                validate_delivered_transition_evidence(transaction, &current)?;
            }
            transition_existing_state(
                transaction,
                &current,
                target,
                "reconcile",
                None,
                canonical_json(&json!({
                    "current_disposition_identity": current.current_disposition_identity,
                    "task_binding_present": current.task_binding_present,
                }))?,
                now,
            )
        })
    }

    fn build_reconcile_summary(
        &self,
        progress_count: usize,
        now: DateTime<Utc>,
    ) -> Result<ReconcileSummary> {
        self.with_connection(|connection| {
            let mut deliverable_decisions = Vec::new();
            let mut locally_pending_decisions = Vec::new();
            let mut foreign_attempts = Vec::new();
            let mut manual_reviews = Vec::new();
            let mut statement = connection.prepare(
                "SELECT d.decision_identity,d.state,d.retry_authorized,
                        a.owner_instance_identity,a.lease_expires_at
                 FROM delivery_decisions d
                 LEFT JOIN delivery_attempts a
                   ON a.attempt_identity=d.current_attempt_identity
                 ORDER BY d.business_date,d.decision_identity",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? == 1,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?;
            for row in rows {
                let (identity, raw_state, retry_authorized, owner, lease) = row?;
                let state = DecisionState::parse(&raw_state)?;
                match state {
                    DecisionState::Reserved => deliverable_decisions.push(identity),
                    DecisionState::RejectedDurable if retry_authorized => {
                        deliverable_decisions.push(identity)
                    }
                    DecisionState::AttemptInFlight => {
                        let lease_live = lease
                            .as_deref()
                            .map(parse_timestamp)
                            .transpose()?
                            .is_some_and(|deadline| deadline > now);
                        if lease_live
                            && owner.as_deref()
                                != Some(self.config.owner_instance_identity.as_str())
                        {
                            foreign_attempts.push(identity);
                        } else {
                            locally_pending_decisions.push(identity);
                        }
                    }
                    DecisionState::Delivered
                    | DecisionState::RejectedDurable
                    | DecisionState::ManualResolvedRejected => {}
                    DecisionState::UncertainManualReview => manual_reviews.push(identity),
                    _ => locally_pending_decisions.push(identity),
                }
            }
            let mut hydrations = Vec::new();
            let mut statement = connection.prepare(
                "SELECT t.decision_identity,d.envelope_canonical,t.transition_identity,
                        t.transition_canonical,t.transition_sha256,t.immutable_audit_ref,
                        t.hydration_state
                 FROM task_transition_payloads t
                 JOIN delivery_decisions d ON d.decision_identity=t.decision_identity
                 WHERE t.append_state='Appended'
                   AND d.state IN ('Delivered','RejectedDurable','UncertainManualReview',
                                   'ManualResolvedRejected')
                 ORDER BY d.business_date,d.decision_identity,t.transition_identity",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })?;
            for row in rows {
                let (
                    decision_identity,
                    envelope_canonical,
                    transition_identity,
                    transition_canonical,
                    transition_sha256,
                    immutable_audit_ref,
                    hydration_state,
                ) = row?;
                hydrations.push(schedule_hydration_from_parts(
                    decision_identity,
                    &envelope_canonical,
                    transition_identity,
                    transition_canonical,
                    transition_sha256,
                    immutable_audit_ref,
                    &hydration_state,
                )?);
            }
            Ok(ReconcileSummary {
                provider_calls: 0,
                sink_calls: 0,
                progress_count,
                locally_pending_decisions,
                deliverable_decisions,
                non_progressable_foreign_attempts: foreign_attempts,
                non_progressable_manual_reviews: manual_reviews,
                schedule_hydrations: hydrations,
            })
        })
    }
}

fn validate_delivered_transition_evidence(
    transaction: &Transaction<'_>,
    stored: &StoredDecision,
) -> Result<()> {
    let expected_state = if stored.task_binding_present {
        DecisionState::AcceptedTaskTransitionPending
    } else {
        DecisionState::AcceptedAuditPending
    };
    if stored.state != expected_state {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "Delivered transition requires current state {expected_state}, observed {}",
            stored.state
        )));
    }
    let envelope = parse_envelope(&stored.envelope_canonical)?;
    if envelope.canonical_sha256()? != stored.envelope_sha256
        || envelope.task_binding.is_some() != stored.task_binding_present
    {
        return Err(DurableDeliveryError::PolicyMismatch(
            "Delivered transition current envelope binding mismatch".to_owned(),
        ));
    }
    let current_disposition_identity =
        stored
            .current_disposition_identity
            .as_deref()
            .ok_or_else(|| {
                DurableDeliveryError::PolicyMismatch(
                    "Delivered transition requires a current disposition".to_owned(),
                )
            })?;
    let disposition = transaction
        .query_row(
            "SELECT disposition_identity,attempt_identity,resolution_identity,
                    denial_identity,disposition,disposition_canonical,
                    disposition_sha256,append_state,immutable_audit_ref,created_at
             FROM delivery_disposition_payloads
             WHERE disposition_identity=?1 AND decision_identity=?2",
            params![current_disposition_identity, stored.decision_identity],
            |row| {
                Ok(StoredDispositionEvidence {
                    disposition_identity: row.get(0)?,
                    attempt_identity: row.get(1)?,
                    resolution_identity: row.get(2)?,
                    denial_identity: row.get(3)?,
                    disposition: row.get(4)?,
                    canonical: row.get(5)?,
                    sha256: row.get(6)?,
                    append_state: row.get(7)?,
                    immutable_audit_ref: row.get(8)?,
                    created_at: row.get(9)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "Delivered transition current disposition is missing or rebound".to_owned(),
            )
        })?;

    let disposition_payload = match disposition.disposition.as_str() {
        "Accepted" => validate_authoritative_accepted_delivery_evidence(
            transaction,
            stored,
            &envelope,
            &disposition,
        )?,
        "ManualAccepted" => {
            let manual = load_and_validate_manual_accepted_delivery_evidence(
                transaction,
                &stored.decision_identity,
            )?;
            if manual.resolution_identity.as_str()
                != disposition
                    .resolution_identity
                    .as_deref()
                    .unwrap_or_default()
            {
                return Err(DurableDeliveryError::PolicyMismatch(
                    "manual accepted disposition resolution binding mismatch".to_owned(),
                ));
            }
            validate_current_disposition_canonical(
                stored,
                &envelope,
                &disposition,
                &manual.acceptance_evidence_sha256,
            )?
        }
        other => {
            return Err(DurableDeliveryError::PolicyMismatch(format!(
                "Delivered transition current disposition must be Accepted or ManualAccepted, observed {other}"
            )));
        }
    };

    if stored.task_binding_present {
        validate_current_task_transition(
            transaction,
            stored,
            &envelope,
            &disposition,
            &disposition_payload,
        )?;
    }
    Ok(())
}

fn validate_current_disposition_canonical(
    stored: &StoredDecision,
    envelope: &DeliveryEnvelope,
    disposition: &StoredDispositionEvidence,
    expected_evidence_sha256: &str,
) -> Result<DeliveryDispositionCanonical> {
    if disposition.disposition_identity
        != stored
            .current_disposition_identity
            .as_deref()
            .unwrap_or_default()
        || disposition.append_state != "Appended"
        || disposition
            .immutable_audit_ref
            .as_deref()
            .is_none_or(|value| !has_non_ascii_whitespace(value))
        || sha256_hex(&disposition.canonical) != disposition.sha256
    {
        return Err(DurableDeliveryError::PolicyMismatch(
            "Delivered current disposition hash/state/reference binding is invalid".to_owned(),
        ));
    }
    parse_timestamp(&disposition.created_at)?;
    let payload = DeliveryDispositionCanonical::parse_exact(
        &disposition.canonical,
        "Delivered current disposition",
    )?;
    let source_identity = disposition
        .attempt_identity
        .as_deref()
        .or(disposition.denial_identity.as_deref())
        .or(disposition.resolution_identity.as_deref())
        .ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "Delivered current disposition source identity is missing".to_owned(),
            )
        })?;
    let source_count = disposition.attempt_identity.is_some() as u8
        + disposition.denial_identity.is_some() as u8
        + disposition.resolution_identity.is_some() as u8;
    let expected_identity = stable_identity(
        "delivery-disposition-v1",
        &[
            &stored.decision_identity,
            source_identity,
            &disposition.disposition,
            expected_evidence_sha256,
        ],
    );
    let exact_binding = source_count == 1
        && payload.schema_version == 1
        && payload.disposition_identity == disposition.disposition_identity
        && payload.disposition_identity == expected_identity
        && payload.decision_identity == stored.decision_identity
        && payload.envelope_sha256 == stored.envelope_sha256
        && payload.envelope_sha256 == envelope.canonical_sha256()?
        && payload.attempt_identity == disposition.attempt_identity
        && payload.denial_identity == disposition.denial_identity
        && payload.resolution_identity == disposition.resolution_identity
        && payload.disposition == disposition.disposition
        && payload.evidence_sha256 == expected_evidence_sha256
        && !payload.retry_authorized
        && !stored.retry_authorized
        && !payload.manual_action_required
        && payload.created_at == disposition.created_at;
    if !exact_binding {
        return Err(DurableDeliveryError::PolicyMismatch(
            "Delivered current disposition exact semantic binding mismatch".to_owned(),
        ));
    }
    Ok(payload)
}

fn validate_current_task_transition(
    transaction: &Transaction<'_>,
    stored: &StoredDecision,
    envelope: &DeliveryEnvelope,
    disposition: &StoredDispositionEvidence,
    disposition_payload: &DeliveryDispositionCanonical,
) -> Result<()> {
    let binding = envelope.task_binding.as_ref().ok_or_else(|| {
        DurableDeliveryError::PolicyMismatch(
            "Delivered task transition has no current envelope task binding".to_owned(),
        )
    })?;
    let rows = {
        let mut statement = transaction.prepare(
            "SELECT transition_identity,task_binding_sha256,transition_canonical,
                    transition_sha256,append_state,immutable_audit_ref,
                    hydration_state,hydration_ack_identity,hydrated_at
             FROM task_transition_payloads
             WHERE decision_identity=?1 AND disposition_identity=?2",
        )?;
        let mapped = statement.query_map(
            params![stored.decision_identity, disposition.disposition_identity],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )?;
        mapped.collect::<std::result::Result<Vec<_>, _>>()?
    };
    if rows.len() != 1 {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "Delivered task-bound transition requires exactly one current task transition, observed {}",
            rows.len()
        )));
    }
    let (
        transition_identity,
        task_binding_sha256,
        canonical,
        sha256,
        append_state,
        immutable_ref,
        hydration_state,
        hydration_ack_identity,
        hydrated_at,
    ) = &rows[0];
    if append_state != "Appended"
        || immutable_ref
            .as_deref()
            .is_none_or(|value| !has_non_ascii_whitespace(value))
        || hydration_state != "Pending"
        || hydration_ack_identity.is_some()
        || hydrated_at.is_some()
        || sha256_hex(canonical) != sha256.as_str()
    {
        return Err(DurableDeliveryError::PolicyMismatch(
            "Delivered current task transition hash/state/reference binding is invalid".to_owned(),
        ));
    }
    let payload = TaskTransitionCanonical::parse_exact(canonical)?;
    let source_identity = disposition_payload
        .attempt_identity
        .as_deref()
        .or(disposition_payload.denial_identity.as_deref())
        .or(disposition_payload.resolution_identity.as_deref())
        .ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "Delivered current task transition source identity is missing".to_owned(),
            )
        })?;
    let expected_transition_identity = stable_identity(
        "BR-140-disposition-v1",
        &[
            &binding.task_identity,
            &stored.decision_identity,
            source_identity,
            "Accepted",
        ],
    );
    let exact_binding = payload.schema_version == 1
        && payload.transition_identity.as_str() == transition_identity.as_str()
        && payload.transition_identity == expected_transition_identity
        && payload.task_identity == binding.task_identity
        && payload.decision_identity == stored.decision_identity
        && payload.source_identity == source_identity
        && payload.task_disposition == "Accepted"
        && payload.task_binding_sha256.as_str() == task_binding_sha256.as_str()
        && payload.task_binding_sha256 == binding.transition_basis_sha256
        && payload.generic_disposition_identity == disposition.disposition_identity
        && payload.generic_disposition_sha256 == disposition.sha256;
    if !exact_binding {
        return Err(DurableDeliveryError::PolicyMismatch(
            "Delivered current task transition exact semantic binding mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn validate_authoritative_accepted_delivery_evidence(
    connection: &Connection,
    stored: &StoredDecision,
    envelope: &DeliveryEnvelope,
    disposition: &StoredDispositionEvidence,
) -> Result<DeliveryDispositionCanonical> {
    let accepted_count: i64 = connection.query_row(
        "SELECT COUNT(*)
         FROM delivery_disposition_payloads p
         JOIN sink_results s
           ON s.attempt_identity=p.attempt_identity
          AND s.decision_identity=p.decision_identity
          AND s.authoritative_for_state=1
          AND s.result_kind='Accepted'
         WHERE p.disposition_identity=?1
           AND p.decision_identity=?2
           AND p.disposition='Accepted'",
        params![disposition.disposition_identity, stored.decision_identity],
        |row| row.get(0),
    )?;
    if accepted_count != 1 {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "Delivered transition requires exactly one authoritative accepted evidence join, observed {accepted_count}"
        )));
    }
    let sink = connection
        .query_row(
            "SELECT s.result_event_identity,s.observed_at,s.fence_token,
                    s.authoritative_for_state,s.late_after_fence,
                    s.authority_audit_identity,s.late_receipt_audit_identity,
                    s.result_canonical,s.result_sha256,
                    s.channel,s.provider,s.message_id,s.platform_message_id,
                    s.accepted_at,s.latency_ms,
                    s.frozen_delivery_audit_canonical,s.frozen_delivery_audit_sha256,
                    s.delivery_audit_ref,a.state
             FROM delivery_disposition_payloads p
             JOIN sink_results s
               ON s.attempt_identity=p.attempt_identity
              AND s.decision_identity=p.decision_identity
               AND s.authoritative_for_state=1
               AND s.result_kind='Accepted'
             JOIN delivery_attempts a
               ON a.attempt_identity=s.attempt_identity
              AND a.decision_identity=s.decision_identity
              AND a.fence_token=s.fence_token
             WHERE p.disposition_identity=?1
               AND p.decision_identity=?2
               AND p.disposition='Accepted'",
            params![disposition.disposition_identity, stored.decision_identity],
            |row| {
                Ok(StoredAcceptedSinkEvidence {
                    result_event_identity: row.get(0)?,
                    observed_at: row.get(1)?,
                    fence_token: row.get(2)?,
                    authoritative_for_state: row.get::<_, i64>(3)? == 1,
                    late_after_fence: row.get::<_, i64>(4)? == 1,
                    authority_audit_identity: row.get(5)?,
                    late_receipt_audit_identity: row.get(6)?,
                    canonical: row.get(7)?,
                    sha256: row.get(8)?,
                    channel: row.get(9)?,
                    provider: row.get(10)?,
                    message_id: row.get(11)?,
                    platform_message_id: row.get(12)?,
                    accepted_at: row.get(13)?,
                    latency_ms: row.get(14)?,
                    frozen_delivery_audit_canonical: row.get(15)?,
                    frozen_delivery_audit_sha256: row.get(16)?,
                    delivery_audit_ref: row.get(17)?,
                    attempt_state: row.get(18)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "Delivered transition has no exact authoritative accepted evidence join".to_owned(),
            )
        })?;
    let attempt_identity = disposition.attempt_identity.as_deref().ok_or_else(|| {
        DurableDeliveryError::PolicyMismatch(
            "authoritative accepted disposition has no attempt identity".to_owned(),
        )
    })?;
    if stored.current_attempt_identity.as_deref() != Some(attempt_identity)
        || disposition.resolution_identity.is_some()
        || disposition.denial_identity.is_some()
        || !sink.authoritative_for_state
        || sink.late_after_fence
        || sink.late_receipt_audit_identity.is_some()
        || !has_non_ascii_whitespace(&sink.authority_audit_identity)
        || sink.attempt_state != "Accepted"
    {
        return Err(DurableDeliveryError::PolicyMismatch(
            "authoritative accepted current attempt authority binding mismatch".to_owned(),
        ));
    }
    parse_timestamp(&sink.observed_at)?;
    if sha256_hex(&sink.canonical) != sink.sha256 {
        return Err(DurableDeliveryError::PolicyMismatch(
            "authoritative accepted result canonical hash mismatch".to_owned(),
        ));
    }
    let result_payload = AcceptedSinkResultCanonical::parse_exact(&sink.canonical)?;
    result_payload.receipt.validate()?;
    let receipt = &result_payload.receipt;
    let accepted_at = timestamp(receipt.accepted_at);
    if result_payload.kind != "Accepted"
        || sink.channel.as_deref() != Some(receipt.channel.as_str())
        || sink.provider.as_deref() != Some(receipt.provider.as_str())
        || sink.message_id.as_deref() != Some(receipt.message_id.as_str())
        || sink.platform_message_id.as_deref() != receipt.platform_message_id.as_deref()
        || sink.accepted_at.as_deref() != Some(accepted_at.as_str())
        || sink.latency_ms != receipt.latency_ms
    {
        return Err(DurableDeliveryError::PolicyMismatch(
            "authoritative accepted result receipt/column exact binding mismatch".to_owned(),
        ));
    }
    let disposition_payload =
        validate_current_disposition_canonical(stored, envelope, disposition, &sink.sha256)?;
    if disposition_payload.attempt_identity.as_deref() != Some(attempt_identity)
        || disposition_payload.created_at != accepted_at
    {
        return Err(DurableDeliveryError::PolicyMismatch(
            "authoritative accepted disposition/result timestamp binding mismatch".to_owned(),
        ));
    }
    let delivery_audit_canonical =
        sink.frozen_delivery_audit_canonical
            .as_deref()
            .ok_or_else(|| {
                DurableDeliveryError::PolicyMismatch(
                    "authoritative accepted frozen delivery audit is missing".to_owned(),
                )
            })?;
    let delivery_audit_sha256 = sink
        .frozen_delivery_audit_sha256
        .as_deref()
        .ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "authoritative accepted frozen delivery audit hash is missing".to_owned(),
            )
        })?;
    if sha256_hex(delivery_audit_canonical) != delivery_audit_sha256
        || sink
            .delivery_audit_ref
            .as_deref()
            .is_none_or(|value| !has_non_ascii_whitespace(value))
    {
        return Err(DurableDeliveryError::PolicyMismatch(
            "authoritative accepted delivery audit is not durably acknowledged".to_owned(),
        ));
    }
    let audit_payload: serde_json::Value = serde_json::from_slice(delivery_audit_canonical)?;
    if serde_json::to_vec(&audit_payload)? != delivery_audit_canonical {
        return Err(DurableDeliveryError::PolicyMismatch(
            "authoritative accepted delivery audit bytes are not canonical JSON".to_owned(),
        ));
    }
    let audit_object = audit_payload.as_object().ok_or_else(|| {
        DurableDeliveryError::PolicyMismatch(
            "authoritative accepted delivery audit canonical payload is not an object".to_owned(),
        )
    })?;
    let receipt_value = serde_json::to_value(receipt)?;
    let expected_fields = [
        "attempt_identity",
        "decision_identity",
        "fence_token",
        "receipt",
        "result_sha256",
    ];
    if audit_object.len() != expected_fields.len()
        || expected_fields
            .iter()
            .any(|field| !audit_object.contains_key(*field))
        || audit_object
            .get("decision_identity")
            .and_then(serde_json::Value::as_str)
            != Some(stored.decision_identity.as_str())
        || audit_object
            .get("attempt_identity")
            .and_then(serde_json::Value::as_str)
            != Some(attempt_identity)
        || audit_object
            .get("fence_token")
            .and_then(serde_json::Value::as_i64)
            != Some(sink.fence_token)
        || audit_object
            .get("result_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(sink.sha256.as_str())
        || audit_object.get("receipt") != Some(&receipt_value)
    {
        return Err(DurableDeliveryError::PolicyMismatch(
            "authoritative accepted delivery audit exact binding mismatch".to_owned(),
        ));
    }
    let expected_result_identity = stable_identity(
        "delivery-sink-result-v1",
        &[
            attempt_identity,
            &sink.fence_token.to_string(),
            "Accepted",
            &sink.sha256,
        ],
    );
    if sink.result_event_identity != expected_result_identity {
        return Err(DurableDeliveryError::PolicyMismatch(
            "authoritative accepted result identity mismatch".to_owned(),
        ));
    }
    Ok(disposition_payload)
}

fn insert_new_decision(
    transaction: &Transaction<'_>,
    envelope: &DeliveryEnvelope,
    envelope_canonical: &[u8],
    envelope_sha256: &str,
    state: DecisionState,
    created_at: DateTime<Utc>,
) -> Result<()> {
    let (binding_present, transition_basis, transition_basis_sha256) = match &envelope.task_binding
    {
        Some(binding) => (
            1_i64,
            Some(binding.transition_basis_canonical.as_slice()),
            Some(binding.transition_basis_sha256.as_str()),
        ),
        None => (0_i64, None, None),
    };
    transaction.execute(
        "INSERT INTO delivery_decisions(
           decision_identity,business_date,push_kind,sub_kind,cooldown_scope,scope_key,
           state,envelope_version,envelope_canonical,envelope_sha256,
           task_binding_present,transition_basis_canonical,transition_basis_sha256,
           reservation_generation,current_budget_reservation_identity,
           current_cooldown_reservation_identity,current_attempt_identity,
           current_disposition_identity,fence_generation,retry_authorized,created_at,updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,0,
                   NULL,NULL,NULL,NULL,0,?14,?15,?15)",
        params![
            envelope.decision_identity,
            envelope.business_date,
            envelope.push_kind.as_str(),
            envelope.sub_kind.as_str(),
            envelope.cooldown_scope.as_str(),
            envelope.scope_key,
            state.as_str(),
            envelope.envelope_version,
            envelope_canonical,
            envelope_sha256,
            binding_present,
            transition_basis,
            transition_basis_sha256,
            envelope.retry_authorized as i64,
            timestamp(created_at),
        ],
    )?;
    Ok(())
}

fn load_decision(
    connection: &Connection,
    decision_identity: &str,
) -> Result<Option<StoredDecision>> {
    let raw = connection
        .query_row(
            "SELECT decision_identity,state,envelope_canonical,envelope_sha256,
                    reservation_generation,current_budget_reservation_identity,
                    current_cooldown_reservation_identity,current_attempt_identity,
                    current_disposition_identity,fence_generation,retry_authorized,
                    task_binding_present
             FROM delivery_decisions WHERE decision_identity=?1",
            [decision_identity],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                ))
            },
        )
        .optional()?;
    raw.map(
        |(
            decision_identity,
            raw_state,
            envelope_canonical,
            envelope_sha256,
            reservation_generation,
            current_budget_reservation_identity,
            current_cooldown_reservation_identity,
            current_attempt_identity,
            current_disposition_identity,
            fence_generation,
            retry_authorized,
            task_binding_present,
        )| {
            Ok(StoredDecision {
                decision_identity,
                state: DecisionState::parse(&raw_state)?,
                envelope_canonical,
                envelope_sha256,
                reservation_generation,
                current_budget_reservation_identity,
                current_cooldown_reservation_identity,
                current_attempt_identity,
                current_disposition_identity,
                fence_generation,
                retry_authorized: retry_authorized == 1,
                task_binding_present: task_binding_present == 1,
            })
        },
    )
    .transpose()
}

fn outcome_from_stored(
    stored: &StoredDecision,
    hydration: &Option<ScheduleHydration>,
) -> PrepareOutcome {
    PrepareOutcome {
        decision_identity: stored.decision_identity.clone(),
        state: stored.state,
        sink_calls: 0,
        reservation_generation: stored.reservation_generation,
        budget_reservation_identity: stored.current_budget_reservation_identity.clone(),
        cooldown_reservation_identity: stored.current_cooldown_reservation_identity.clone(),
        schedule_hydration: hydration.clone(),
    }
}

fn load_schedule_hydration(
    connection: &Connection,
    decision_identity: &str,
) -> Result<Option<ScheduleHydration>> {
    type StoredScheduleHydrationRow = (String, Vec<u8>, String, Vec<u8>, String, String, String);

    let row: Option<StoredScheduleHydrationRow> = connection
        .query_row(
            "SELECT t.decision_identity,d.envelope_canonical,t.transition_identity,
                    t.transition_canonical,t.transition_sha256,t.immutable_audit_ref,
                    t.hydration_state
             FROM task_transition_payloads t
             JOIN delivery_decisions d ON d.decision_identity=t.decision_identity
             WHERE t.decision_identity=?1 AND t.append_state='Appended'
             ORDER BY transition_identity LIMIT 1",
            [decision_identity],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(
            decision_identity,
            envelope_canonical,
            transition_identity,
            transition_canonical,
            transition_sha256,
            immutable_audit_ref,
            hydration_state,
        )| {
            schedule_hydration_from_parts(
                decision_identity,
                &envelope_canonical,
                transition_identity,
                transition_canonical,
                transition_sha256,
                immutable_audit_ref,
                &hydration_state,
            )
        },
    )
    .transpose()
}

#[allow(clippy::too_many_arguments)]
fn schedule_hydration_from_parts(
    decision_identity: String,
    envelope_canonical: &[u8],
    transition_identity: String,
    transition_canonical: Vec<u8>,
    transition_sha256: String,
    immutable_audit_ref: String,
    hydration_state: &str,
) -> Result<ScheduleHydration> {
    let envelope = parse_envelope(envelope_canonical)?;
    let task_binding = envelope.task_binding.ok_or_else(|| {
        DurableDeliveryError::PolicyMismatch(format!(
            "task transition {transition_identity} has no persisted task binding"
        ))
    })?;
    Ok(ScheduleHydration {
        decision_identity,
        task_identity: task_binding.task_identity,
        transition_identity,
        transition_canonical,
        transition_sha256,
        transition_basis_canonical: task_binding.transition_basis_canonical,
        transition_basis_sha256: task_binding.transition_basis_sha256,
        immutable_audit_ref,
        hydration_state: ScheduleHydrationState::parse(hydration_state)?,
    })
}

fn parse_envelope(canonical: &[u8]) -> Result<DeliveryEnvelope> {
    let envelope: DeliveryEnvelope = serde_json::from_slice(canonical)?;
    envelope.validate()?;
    Ok(envelope)
}

fn replay_push_kind(review_task: &str) -> Result<super::model::PushKind> {
    match review_task {
        "R-04" => Ok(super::model::PushKind::ReviewLhb),
        "R-09" => Ok(super::model::PushKind::ReviewProviderTopN),
        _ => Err(DurableDeliveryError::PolicyMismatch(
            "terminal_replay_identity_invalid".to_owned(),
        )),
    }
}

fn validate_replay_reason_code(
    state: ReviewTerminalReplayCompletionState,
    reason_code: &str,
) -> Result<()> {
    let valid = match state {
        ReviewTerminalReplayCompletionState::Passed => reason_code == "existing_terminal_hydrated",
        ReviewTerminalReplayCompletionState::Failed => matches!(
            reason_code,
            "terminal_replay_identity_invalid"
                | "terminal_replay_not_delivered"
                | "terminal_replay_hydration_not_applied"
                | "terminal_replay_would_require_sink"
                | "terminal_replay_watermark_changed"
                | "terminal_replay_evidence_unavailable"
        ),
    };
    if !valid {
        return Err(DurableDeliveryError::PolicyMismatch(
            "terminal_replay_evidence_unavailable".to_owned(),
        ));
    }
    Ok(())
}

fn sink_authority_watermark(
    connection: &Connection,
    decision_identity: &str,
) -> Result<AuthorityWatermark> {
    let mut statement = connection.prepare(
        "SELECT result_event_identity,attempt_identity,result_sha256
         FROM sink_results
         WHERE decision_identity=?1
         ORDER BY result_event_identity ASC",
    )?;
    let rows = statement
        .query_map([decision_identity], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let canonical = rows
        .iter()
        .map(
            |(result_event_identity, attempt_identity, result_sha256)| SinkAuthorityIdentity {
                result_event_identity,
                attempt_identity,
                result_sha256,
            },
        )
        .collect::<Vec<_>>();
    Ok(AuthorityWatermark {
        count: canonical.len() as i64,
        ordered_identity_set_sha256: sha256_hex(&serde_json::to_vec(&canonical)?),
    })
}

fn delivery_audit_authority_watermark(
    connection: &Connection,
    decision_identity: &str,
) -> Result<AuthorityWatermark> {
    let mut statement = connection.prepare(
        "SELECT result_event_identity,delivery_audit_ref,frozen_delivery_audit_sha256
         FROM sink_results
         WHERE decision_identity=?1
           AND delivery_audit_ref IS NOT NULL
           AND frozen_delivery_audit_sha256 IS NOT NULL
         ORDER BY result_event_identity ASC",
    )?;
    let rows = statement
        .query_map([decision_identity], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let canonical = rows
        .iter()
        .map(
            |(result_event_identity, delivery_audit_ref, frozen_delivery_audit_sha256)| {
                DeliveryAuditAuthorityIdentity {
                    result_event_identity,
                    delivery_audit_ref,
                    frozen_delivery_audit_sha256,
                }
            },
        )
        .collect::<Vec<_>>();
    Ok(AuthorityWatermark {
        count: canonical.len() as i64,
        ordered_identity_set_sha256: sha256_hex(&serde_json::to_vec(&canonical)?),
    })
}

fn first_available_budget_slot(
    connection: &Connection,
    business_date: &str,
) -> Result<Option<i64>> {
    let mut statement = connection.prepare(
        "SELECT slot_no FROM daily_budget_reservations
         WHERE business_date=?1 AND state IN ('Reserved','Accepted','Uncertain')",
    )?;
    let occupied = statement
        .query_map([business_date], |row| row.get::<_, i64>(0))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    Ok((1..=DAILY_BUDGET_LIMIT).find(|slot| !occupied.contains(slot)))
}

fn rolling_head_conflicts(
    connection: &Connection,
    envelope: &DeliveryEnvelope,
    admission_at: DateTime<Utc>,
) -> Result<bool> {
    let head = connection
        .query_row(
            "SELECT state,blocked_until FROM cooldown_heads
             WHERE push_kind=?1 AND sub_kind=?2 AND cooldown_scope=?3 AND scope_key=?4",
            params![
                envelope.push_kind.as_str(),
                envelope.sub_kind.as_str(),
                envelope.cooldown_scope.as_str(),
                envelope.scope_key
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((state, blocked_until)) = head else {
        return Ok(false);
    };
    match state.as_str() {
        "Reserved" | "Uncertain" => Ok(true),
        "Released" => Ok(false),
        "Accepted" => Ok(blocked_until
            .as_deref()
            .map(parse_timestamp)
            .transpose()?
            .is_none_or(|until| until > admission_at)),
        other => Err(DurableDeliveryError::PolicyMismatch(format!(
            "invalid rolling head state {other}"
        ))),
    }
}

fn freeze_pre_sink_denial(
    transaction: &Transaction<'_>,
    envelope: &DeliveryEnvelope,
    policy: &PolicyRow,
    denial: PrepareDenial,
    denied_at: DateTime<Utc>,
) -> Result<()> {
    let evidence = canonical_json(&denial.evidence())?;
    let evidence_sha256 = sha256_hex(&evidence);
    let denial_identity = stable_identity(
        "delivery-pre-sink-denial-v1",
        &[
            &envelope.decision_identity,
            &envelope.canonical_sha256()?,
            &policy.policy_version.to_string(),
            denial.reason_code(),
            &evidence_sha256,
        ],
    );
    freeze_disposition(
        transaction,
        envelope,
        None,
        None,
        Some(&denial_identity),
        "Rejected",
        &evidence_sha256,
        false,
        denied_at,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn freeze_disposition(
    transaction: &Transaction<'_>,
    envelope: &DeliveryEnvelope,
    attempt_identity: Option<&str>,
    resolution_identity: Option<&str>,
    denial_identity: Option<&str>,
    disposition: &str,
    evidence_sha256: &str,
    retry_authorized: bool,
    created_at: DateTime<Utc>,
) -> Result<String> {
    let source_identity = attempt_identity
        .or(denial_identity)
        .or(resolution_identity)
        .ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "generic disposition requires attempt, denial or resolution identity".to_owned(),
            )
        })?;
    let disposition_identity = stable_identity(
        "delivery-disposition-v1",
        &[
            &envelope.decision_identity,
            source_identity,
            disposition,
            evidence_sha256,
        ],
    );
    let canonical = canonical_json(&json!({
        "schema_version": 1,
        "disposition_identity": disposition_identity,
        "decision_identity": envelope.decision_identity,
        "envelope_sha256": envelope.canonical_sha256()?,
        "attempt_identity": attempt_identity,
        "denial_identity": denial_identity,
        "resolution_identity": resolution_identity,
        "disposition": disposition,
        "evidence_sha256": evidence_sha256,
        "retry_authorized": retry_authorized,
        "manual_action_required": disposition == "Uncertain",
        "created_at": timestamp(created_at),
    }))?;
    let canonical_hash = sha256_hex(&canonical);
    transaction.execute(
        "INSERT INTO delivery_disposition_payloads(
           disposition_identity,decision_identity,attempt_identity,resolution_identity,
           denial_identity,disposition,disposition_canonical,disposition_sha256,
           append_state,immutable_audit_ref,created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'Pending',NULL,?9)",
        params![
            disposition_identity,
            envelope.decision_identity,
            attempt_identity,
            resolution_identity,
            denial_identity,
            disposition,
            canonical,
            canonical_hash,
            timestamp(created_at),
        ],
    )?;
    if let Some(binding) = &envelope.task_binding {
        let task_disposition = match disposition {
            "ManualAccepted" => "Accepted",
            "ManualRejected" => "ManualRejected",
            other => other,
        };
        let transition_identity = stable_identity(
            "BR-140-disposition-v1",
            &[
                &binding.task_identity,
                &envelope.decision_identity,
                source_identity,
                task_disposition,
            ],
        );
        let transition = canonical_json(&json!({
            "schema_version": 1,
            "transition_identity": transition_identity,
            "task_identity": binding.task_identity,
            "decision_identity": envelope.decision_identity,
            "source_identity": source_identity,
            "task_disposition": task_disposition,
            "task_binding_sha256": binding.transition_basis_sha256,
            "generic_disposition_identity": disposition_identity,
            "generic_disposition_sha256": canonical_hash,
        }))?;
        let transition_hash = sha256_hex(&transition);
        transaction.execute(
            "INSERT INTO task_transition_payloads(
               transition_identity,decision_identity,disposition_identity,
               task_binding_sha256,transition_canonical,transition_sha256,
               append_state,immutable_audit_ref
             ) VALUES (?1,?2,?3,?4,?5,?6,'Pending',NULL)",
            params![
                transition_identity,
                envelope.decision_identity,
                disposition_identity,
                binding.transition_basis_sha256,
                transition,
                transition_hash,
            ],
        )?;
    }
    transaction.execute(
        "UPDATE delivery_decisions SET current_disposition_identity=?1,
           retry_authorized=?2,updated_at=?3 WHERE decision_identity=?4",
        params![
            disposition_identity,
            retry_authorized as i64,
            timestamp(created_at),
            envelope.decision_identity
        ],
    )?;
    Ok(disposition_identity)
}

#[allow(clippy::too_many_arguments)]
fn record_state_transition(
    transaction: &Transaction<'_>,
    decision_identity: &str,
    from_state: Option<DecisionState>,
    to_state: DecisionState,
    actor: &str,
    operator_identity: Option<&str>,
    evidence_canonical: Vec<u8>,
    occurred_at: DateTime<Utc>,
) -> Result<()> {
    let evidence_sha256 = sha256_hex(&evidence_canonical);
    let event_identity = stable_identity(
        "delivery-state-event-v1",
        &[
            decision_identity,
            from_state.map(DecisionState::as_str).unwrap_or("NONE"),
            to_state.as_str(),
            actor,
            &evidence_sha256,
        ],
    );
    let audit_payload = canonical_json(&json!({
        "state_event_identity": event_identity,
        "decision_identity": decision_identity,
        "from_state": from_state,
        "to_state": to_state,
        "actor": actor,
        "operator_identity_hash": operator_identity.map(|value| sha256_hex(value.as_bytes())),
        "evidence_sha256": evidence_sha256,
        "occurred_at": timestamp(occurred_at),
    }))?;
    let audit_identity = enqueue_audit(
        transaction,
        decision_identity,
        None,
        "DecisionStateChanged",
        &audit_payload,
        occurred_at,
    )?;
    transaction.execute(
        "INSERT INTO delivery_state_events(
           state_event_identity,decision_identity,from_state,to_state,actor,
           operator_identity,evidence_canonical,evidence_sha256,audit_identity
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            event_identity,
            decision_identity,
            from_state.map(DecisionState::as_str),
            to_state.as_str(),
            actor,
            operator_identity,
            evidence_canonical,
            evidence_sha256,
            audit_identity
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn transition_existing_state(
    transaction: &Transaction<'_>,
    stored: &StoredDecision,
    to_state: DecisionState,
    actor: &str,
    operator_identity: Option<&str>,
    evidence: Vec<u8>,
    occurred_at: DateTime<Utc>,
) -> Result<()> {
    if !legal_transition(stored.state, to_state) {
        return Err(DurableDeliveryError::IllegalTransition {
            from: stored.state.to_string(),
            to: to_state.to_string(),
        });
    }
    let changed = transaction.execute(
        "UPDATE delivery_decisions SET state=?1,updated_at=?2
         WHERE decision_identity=?3 AND state=?4",
        params![
            to_state.as_str(),
            timestamp(occurred_at),
            stored.decision_identity,
            stored.state.as_str()
        ],
    )?;
    if changed != 1 {
        return Err(DurableDeliveryError::IllegalTransition {
            from: stored.state.to_string(),
            to: to_state.to_string(),
        });
    }
    record_state_transition(
        transaction,
        &stored.decision_identity,
        Some(stored.state),
        to_state,
        actor,
        operator_identity,
        evidence,
        occurred_at,
    )
}

fn legal_transition(from: DecisionState, to: DecisionState) -> bool {
    use DecisionState::*;
    matches!(
        (from, to),
        (Reserved, AttemptInFlight)
            | (Reserved, RejectedAuditPending)
            | (AttemptInFlight, AcceptedAuditPending)
            | (AttemptInFlight, RejectedAuditPending)
            | (AttemptInFlight, UncertainAuditPending)
            | (AcceptedAuditPending, AcceptedTaskTransitionPending)
            | (AcceptedAuditPending, Delivered)
            | (AcceptedTaskTransitionPending, Delivered)
            | (RejectedAuditPending, RejectedTaskTransitionPending)
            | (RejectedAuditPending, RejectedDurable)
            | (RejectedTaskTransitionPending, RejectedDurable)
            | (UncertainAuditPending, UncertainTaskTransitionPending)
            | (UncertainAuditPending, UncertainManualReview)
            | (UncertainTaskTransitionPending, UncertainManualReview)
            | (UncertainManualReview, AcceptedAuditPending)
            | (UncertainManualReview, ManualRejectedAuditPending)
            | (
                ManualRejectedAuditPending,
                ManualRejectedTaskTransitionPending
            )
            | (ManualRejectedAuditPending, ManualResolvedRejected)
            | (ManualRejectedTaskTransitionPending, ManualResolvedRejected)
            | (RejectedDurable, Reserved)
    )
}

fn enqueue_audit(
    transaction: &Transaction<'_>,
    decision_identity: &str,
    attempt_identity: Option<&str>,
    audit_kind: &str,
    audit_canonical: &[u8],
    created_at: DateTime<Utc>,
) -> Result<String> {
    if !AUDIT_KINDS.contains(&audit_kind) {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "unregistered immutable audit kind {audit_kind}"
        )));
    }
    let audit_sha256 = sha256_hex(audit_canonical);
    let audit_identity = stable_identity(
        "delivery-critical-audit-v1",
        &[
            decision_identity,
            attempt_identity.unwrap_or("NONE"),
            audit_kind,
            &audit_sha256,
        ],
    );
    let predecessor: Option<String> = transaction
        .query_row(
            "SELECT audit_identity FROM immutable_audit_outbox
             WHERE decision_identity=?1 ORDER BY rowid DESC LIMIT 1",
            [decision_identity],
            |row| row.get(0),
        )
        .optional()?;
    transaction.execute(
        "INSERT INTO immutable_audit_outbox(
           audit_identity,decision_identity,attempt_identity,audit_kind,
           predecessor_audit_identity,audit_canonical,audit_sha256,
           append_state,immutable_audit_ref,created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,'Pending',NULL,?8)",
        params![
            audit_identity,
            decision_identity,
            attempt_identity,
            audit_kind,
            predecessor,
            audit_canonical,
            audit_sha256,
            timestamp(created_at)
        ],
    )?;
    Ok(audit_identity)
}

fn record_attempt_event(
    transaction: &Transaction<'_>,
    attempt_identity: &str,
    decision_identity: &str,
    event_kind: &str,
    event_canonical: Vec<u8>,
    occurred_at: DateTime<Utc>,
) -> Result<()> {
    let audit_identity = enqueue_audit(
        transaction,
        decision_identity,
        Some(attempt_identity),
        event_kind,
        &event_canonical,
        occurred_at,
    )?;
    record_attempt_event_with_audit(
        transaction,
        attempt_identity,
        decision_identity,
        event_kind,
        event_canonical,
        &audit_identity,
    )
}

fn record_attempt_event_with_audit(
    transaction: &Transaction<'_>,
    attempt_identity: &str,
    decision_identity: &str,
    event_kind: &str,
    event_canonical: Vec<u8>,
    audit_identity: &str,
) -> Result<()> {
    let event_sha256 = sha256_hex(&event_canonical);
    let event_identity = stable_identity(
        "delivery-attempt-event-v1",
        &[attempt_identity, event_kind, &event_sha256, audit_identity],
    );
    transaction.execute(
        "INSERT INTO delivery_attempt_events(
           attempt_event_identity,attempt_identity,decision_identity,event_kind,
           event_canonical,event_sha256,audit_identity
         ) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            event_identity,
            attempt_identity,
            decision_identity,
            event_kind,
            event_canonical,
            event_sha256,
            audit_identity
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_budget_event(
    transaction: &Transaction<'_>,
    reservation_identity: &str,
    decision_identity: &str,
    from_state: Option<&str>,
    to_state: &str,
    event_canonical: Vec<u8>,
    occurred_at: DateTime<Utc>,
) -> Result<()> {
    let event_sha256 = sha256_hex(&event_canonical);
    let audit_canonical = canonical_json(&json!({
        "budget_reservation_identity": reservation_identity,
        "from_state": from_state,
        "to_state": to_state,
        "event_sha256": &event_sha256,
        "occurred_at": timestamp(occurred_at),
    }))?;
    let audit_identity = enqueue_audit(
        transaction,
        decision_identity,
        None,
        "BudgetReservationChanged",
        &audit_canonical,
        occurred_at,
    )?;
    let event_identity = stable_identity(
        "delivery-budget-event-v1",
        &[
            reservation_identity,
            from_state.unwrap_or("NONE"),
            to_state,
            &event_sha256,
        ],
    );
    transaction.execute(
        "INSERT INTO daily_budget_reservation_events(
           event_identity,budget_reservation_identity,decision_identity,
           from_state,to_state,event_canonical,event_sha256,audit_identity
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            event_identity,
            reservation_identity,
            decision_identity,
            from_state,
            to_state,
            event_canonical,
            event_sha256,
            audit_identity
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_cooldown_event(
    transaction: &Transaction<'_>,
    reservation_identity: &str,
    decision_identity: &str,
    from_state: Option<&str>,
    to_state: &str,
    event_canonical: Vec<u8>,
    occurred_at: DateTime<Utc>,
) -> Result<()> {
    let event_sha256 = sha256_hex(&event_canonical);
    let audit_canonical = canonical_json(&json!({
        "cooldown_reservation_identity": reservation_identity,
        "from_state": from_state,
        "to_state": to_state,
        "event_sha256": &event_sha256,
        "occurred_at": timestamp(occurred_at),
    }))?;
    let audit_identity = enqueue_audit(
        transaction,
        decision_identity,
        None,
        "CooldownReservationChanged",
        &audit_canonical,
        occurred_at,
    )?;
    let event_identity = stable_identity(
        "delivery-cooldown-event-v1",
        &[
            reservation_identity,
            from_state.unwrap_or("NONE"),
            to_state,
            &event_sha256,
        ],
    );
    transaction.execute(
        "INSERT INTO cooldown_reservation_events(
           event_identity,cooldown_reservation_identity,decision_identity,
           from_state,to_state,event_canonical,event_sha256,audit_identity
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            event_identity,
            reservation_identity,
            decision_identity,
            from_state,
            to_state,
            event_canonical,
            event_sha256,
            audit_identity
        ],
    )?;
    Ok(())
}

fn attach_attempt_to_reservations(
    transaction: &Transaction<'_>,
    stored: &StoredDecision,
    attempt_identity: &str,
    attempt_no: i64,
    occurred_at: DateTime<Utc>,
) -> Result<()> {
    // BR-237: 豁免类 (counts_against_daily_budget=false) 无 budget reservation,
    // attempt 只附加到 cooldown reservation; 信号类照旧 CAS 附加。
    if let Some(budget_identity) = stored.current_budget_reservation_identity.as_deref() {
        let changed = transaction.execute(
            "UPDATE daily_budget_reservations SET attempt_identity=?1
             WHERE budget_reservation_identity=?2 AND decision_identity=?3
               AND reservation_generation=?4 AND attempt_identity IS NULL
               AND state='Reserved'",
            params![
                attempt_identity,
                budget_identity,
                stored.decision_identity,
                stored.reservation_generation
            ],
        )?;
        if changed != 1 {
            return Err(DurableDeliveryError::PolicyMismatch(
                "budget attempt compare-and-set failed".to_owned(),
            ));
        }
        record_budget_event(
            transaction,
            budget_identity,
            &stored.decision_identity,
            Some("Reserved"),
            "Reserved",
            canonical_json(&json!({
                "attempt_identity": attempt_identity,
                "attempt_no": attempt_no,
                "reservation_generation": stored.reservation_generation,
            }))?,
            occurred_at,
        )?;
    }
    if let Some(cooldown_identity) = stored.current_cooldown_reservation_identity.as_deref() {
        let changed = transaction.execute(
            "UPDATE cooldown_reservations SET attempt_identity=?1
             WHERE cooldown_reservation_identity=?2 AND decision_identity=?3
               AND reservation_generation=?4 AND attempt_identity IS NULL
               AND state='Reserved'",
            params![
                attempt_identity,
                cooldown_identity,
                stored.decision_identity,
                stored.reservation_generation
            ],
        )?;
        if changed != 1 {
            return Err(DurableDeliveryError::PolicyMismatch(
                "cooldown attempt compare-and-set failed".to_owned(),
            ));
        }
        record_cooldown_event(
            transaction,
            cooldown_identity,
            &stored.decision_identity,
            Some("Reserved"),
            "Reserved",
            canonical_json(&json!({
                "attempt_identity": attempt_identity,
                "attempt_no": attempt_no,
                "reservation_generation": stored.reservation_generation,
            }))?,
            occurred_at,
        )?;
    }
    Ok(())
}

fn mutate_reservations(
    transaction: &Transaction<'_>,
    stored: &StoredDecision,
    to_state: &str,
    occurred_at: DateTime<Utc>,
    accepted_at: Option<DateTime<Utc>>,
) -> Result<()> {
    if !matches!(to_state, "Accepted" | "Uncertain" | "Released") {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "unsupported reservation state {to_state}"
        )));
    }
    if let Some(budget_identity) = stored.current_budget_reservation_identity.as_deref() {
        let from_state: String = transaction.query_row(
            "SELECT state FROM daily_budget_reservations
             WHERE budget_reservation_identity=?1",
            [budget_identity],
            |row| row.get(0),
        )?;
        if from_state != to_state {
            let changed = transaction.execute(
                "UPDATE daily_budget_reservations SET state=?1,
                   accepted_at=CASE WHEN ?1='Accepted' THEN ?2 ELSE accepted_at END,
                   released_at=CASE WHEN ?1='Released' THEN ?3 ELSE released_at END
                 WHERE budget_reservation_identity=?4
                   AND state IN ('Reserved','Uncertain')",
                params![
                    to_state,
                    accepted_at.map(timestamp),
                    timestamp(occurred_at),
                    budget_identity
                ],
            )?;
            if changed != 1 {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "budget reservation {budget_identity} cannot move {from_state}->{to_state}"
                )));
            }
            record_budget_event(
                transaction,
                budget_identity,
                &stored.decision_identity,
                Some(&from_state),
                to_state,
                canonical_json(&json!({
                    "reservation_generation": stored.reservation_generation,
                    "occurred_at": timestamp(occurred_at),
                }))?,
                occurred_at,
            )?;
        }
    }
    // BR-237: 豁免类 (counts_against_daily_budget=false) 的决策有 generation
    // (重试代数) 但无 budget identity — 合法状态, 不再视为完整性违反。
    // 信号类 (counts=true) 无 budget identity 时上方 budget 分支被跳过,
    // 该场景仅存在于 prepare 后 budget 行被外部删除的 bug 情形。
    if let Some(cooldown_identity) = stored.current_cooldown_reservation_identity.as_deref() {
        let (from_state, window_mode, cooldown_secs): (String, String, Option<i64>) = transaction
            .query_row(
            "SELECT state,window_mode,effective_cooldown_secs
                 FROM cooldown_reservations
                 WHERE cooldown_reservation_identity=?1",
            [cooldown_identity],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if from_state != to_state {
            let blocked_until = if to_state == "Accepted" {
                let anchor = accepted_at.unwrap_or(occurred_at);
                cooldown_secs.map(|seconds| timestamp(anchor + Duration::seconds(seconds)))
            } else {
                None
            };
            let changed = transaction.execute(
                "UPDATE cooldown_reservations SET state=?1,
                   accepted_at=CASE WHEN ?1='Accepted' THEN ?2 ELSE accepted_at END,
                   blocked_until=CASE WHEN ?1='Accepted' THEN ?3 ELSE NULL END,
                   released_at=CASE WHEN ?1='Released' THEN ?4 ELSE released_at END
                 WHERE cooldown_reservation_identity=?5
                   AND state IN ('Reserved','Uncertain')",
                params![
                    to_state,
                    accepted_at.map(timestamp),
                    blocked_until,
                    timestamp(occurred_at),
                    cooldown_identity
                ],
            )?;
            if changed != 1 {
                return Err(DurableDeliveryError::PolicyMismatch(format!(
                    "cooldown reservation {cooldown_identity} cannot move {from_state}->{to_state}"
                )));
            }
            record_cooldown_event(
                transaction,
                cooldown_identity,
                &stored.decision_identity,
                Some(&from_state),
                to_state,
                canonical_json(&json!({
                    "reservation_generation": stored.reservation_generation,
                    "blocked_until": blocked_until,
                }))?,
                occurred_at,
            )?;
            if window_mode == WindowMode::Rolling.as_str() {
                transaction.execute(
                    "UPDATE cooldown_heads SET state=?1,blocked_until=?2,version=version+1
                     WHERE current_reservation_identity=?3",
                    params![to_state, blocked_until, cooldown_identity],
                )?;
            }
        }
    }
    Ok(())
}

fn validate_manual_command(command: &ManualResolutionCommand) -> Result<()> {
    if command.decision_identity.trim().is_empty()
        || command.operator_identity.trim().is_empty()
        || command.reason.trim().is_empty()
        || command.external_evidence.is_empty()
    {
        return Err(DurableDeliveryError::InvalidManualResolution(
            "decision/operator/reason/canonical external evidence are mandatory".to_owned(),
        ));
    }
    if let ManualDisposition::Accepted {
        receipt: Some(receipt),
    } = &command.disposition
    {
        receipt.validate()?;
    }
    Ok(())
}

fn canonical_sink_result(result: &AuthoritativeSinkResult) -> Result<Vec<u8>> {
    match result {
        AuthoritativeSinkResult::Accepted(receipt) => {
            receipt.validate()?;
            canonical_json(&json!({"kind": "Accepted", "receipt": receipt}))
        }
        AuthoritativeSinkResult::Rejected(rejection) => {
            if rejection.reason_code.trim().is_empty() || rejection.evidence.is_empty() {
                return Err(DurableDeliveryError::InvalidEnvelope(
                    "typed rejection requires reason and evidence".to_owned(),
                ));
            }
            canonical_json(&json!({"kind": "Rejected", "rejection": rejection}))
        }
        AuthoritativeSinkResult::Uncertain(uncertainty) => {
            if uncertainty.reason_code.trim().is_empty() || uncertainty.evidence.is_empty() {
                return Err(DurableDeliveryError::InvalidEnvelope(
                    "typed uncertainty requires reason and evidence".to_owned(),
                ));
            }
            canonical_json(&json!({"kind": "Uncertain", "uncertainty": uncertainty}))
        }
    }
}

fn sink_result_kind(result: &AuthoritativeSinkResult) -> &'static str {
    match result {
        AuthoritativeSinkResult::Accepted(_) => "Accepted",
        AuthoritativeSinkResult::Rejected(_) => "Rejected",
        AuthoritativeSinkResult::Uncertain(_) => "Uncertain",
    }
}

fn result_evidence_hash(result: &AuthoritativeSinkResult) -> Result<String> {
    Ok(sha256_hex(&canonical_sink_result(result)?))
}

fn load_and_validate_manual_accepted_delivery_evidence(
    connection: &Connection,
    decision_identity: &str,
) -> Result<ManualAcceptedDeliveryAuditEvidence> {
    let evidence = load_manual_accepted_delivery_evidence(connection, decision_identity)?;
    evidence.validate()?;
    Ok(evidence)
}

fn load_manual_accepted_delivery_evidence(
    connection: &Connection,
    decision_identity: &str,
) -> Result<ManualAcceptedDeliveryAuditEvidence> {
    let evidence = connection
        .query_row(
            "SELECT m.resolution_identity,
                    m.attempt_identity,d.current_attempt_identity,a.state,
                    m.operator_identity,m.reason,m.immutable_audit_ref,
                    d.envelope_sha256,d.current_disposition_identity,
                    p.disposition_identity,p.disposition_canonical,
                    p.disposition_sha256,p.append_state,p.immutable_audit_ref,
                    m.evidence_canonical,m.evidence_sha256,m.receipt_canonical,m.resolved_at,
                    m.accepted_audit_identity,
                    m.frozen_delivery_audit_canonical,
                    m.frozen_delivery_audit_sha256,m.accepted_audit_ref,
                    m.accepted_audit_append_state
             FROM manual_resolutions m
             JOIN delivery_decisions d
               ON d.decision_identity=m.decision_identity
             LEFT JOIN delivery_attempts a
               ON a.attempt_identity=m.attempt_identity
              AND a.decision_identity=m.decision_identity
             JOIN delivery_disposition_payloads p
               ON p.disposition_identity=d.current_disposition_identity
              AND p.resolution_identity=m.resolution_identity
              AND p.decision_identity=m.decision_identity
              AND p.disposition='ManualAccepted'
             WHERE m.decision_identity=?1 AND m.disposition='Accepted'",
            [decision_identity],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Vec<u8>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Vec<u8>>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, Option<Vec<u8>>>(16)?,
                    row.get::<_, String>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<Vec<u8>>>(19)?,
                    row.get::<_, Option<String>>(20)?,
                    row.get::<_, Option<String>>(21)?,
                    row.get::<_, Option<String>>(22)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "manual accepted decision has no appended disposition evidence".to_owned(),
            )
        })?;
    let (
        resolution_identity,
        attempt_identity,
        decision_current_attempt_identity,
        attempt_state,
        operator_identity,
        reason,
        authorization_immutable_audit_ref,
        envelope_sha256,
        current_disposition_identity,
        disposition_identity,
        disposition_canonical,
        disposition_sha256,
        disposition_append_state,
        disposition_immutable_audit_ref,
        acceptance_evidence_canonical,
        acceptance_evidence_sha256,
        receipt_canonical,
        resolved_at,
        audit_identity,
        canonical,
        sha256,
        immutable_audit_ref,
        append_state,
    ) = evidence;
    let evidence = ManualAcceptedDeliveryAuditEvidence {
        decision_identity: decision_identity.to_owned(),
        resolution_identity,
        attempt_identity,
        decision_current_attempt_identity: decision_current_attempt_identity.ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "manual accepted decision current attempt identity is missing".to_owned(),
            )
        })?,
        attempt_state: attempt_state.ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "manual accepted original attempt is missing".to_owned(),
            )
        })?,
        operator_identity,
        reason,
        authorization_immutable_audit_ref,
        envelope_sha256,
        current_disposition_identity: current_disposition_identity.ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "manual accepted current disposition identity is missing".to_owned(),
            )
        })?,
        disposition_identity,
        disposition_canonical,
        disposition_sha256,
        disposition_append_state,
        disposition_immutable_audit_ref,
        acceptance_evidence_canonical,
        acceptance_evidence_sha256,
        receipt_canonical,
        resolved_at,
        audit_identity: audit_identity.ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit identity is missing".to_owned(),
            )
        })?,
        canonical: canonical.ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit canonical evidence is missing".to_owned(),
            )
        })?,
        sha256: sha256.ok_or_else(|| {
            DurableDeliveryError::PolicyMismatch(
                "manual accepted delivery audit hash is missing".to_owned(),
            )
        })?,
        append_state: append_state.unwrap_or_default(),
        accepted_audit_immutable_ref: immutable_audit_ref,
    };
    Ok(evidence)
}

fn require_single_cas_update(changed: usize, operation: &str) -> Result<()> {
    if changed != 1 {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "{operation} compare-and-set affected {changed} rows; expected exactly one"
        )));
    }
    Ok(())
}

fn validate_persisted_immutable_references(connection: &Connection) -> Result<()> {
    let schema_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema_version != SCHEMA_VERSION {
        return Ok(());
    }

    const INVALID_REFERENCE_QUERIES: [(&str, &str); 6] = [
        (
            "immutable_audit_outbox.immutable_audit_ref",
            "SELECT COUNT(*) FROM immutable_audit_outbox
             WHERE (append_state='Pending' AND immutable_audit_ref IS NOT NULL)
                OR (append_state='Appended' AND (
                     immutable_audit_ref IS NULL
                     OR length(replace(replace(replace(replace(
                       immutable_audit_ref,' ',''),char(9),''),char(10),''),char(13),''))=0
                ))",
        ),
        (
            "delivery_disposition_payloads.immutable_audit_ref",
            "SELECT COUNT(*) FROM delivery_disposition_payloads
             WHERE (append_state='Pending' AND immutable_audit_ref IS NOT NULL)
                OR (append_state='Appended' AND (
                     immutable_audit_ref IS NULL
                     OR length(replace(replace(replace(replace(
                       immutable_audit_ref,' ',''),char(9),''),char(10),''),char(13),''))=0
                ))",
        ),
        (
            "task_transition_payloads.immutable_audit_ref",
            "SELECT COUNT(*) FROM task_transition_payloads
             WHERE (append_state='Pending' AND immutable_audit_ref IS NOT NULL)
                OR (append_state='Appended' AND (
                     immutable_audit_ref IS NULL
                     OR length(replace(replace(replace(replace(
                       immutable_audit_ref,' ',''),char(9),''),char(10),''),char(13),''))=0
                ))",
        ),
        (
            "manual_resolutions immutable references",
            "SELECT COUNT(*) FROM manual_resolutions
             WHERE length(replace(replace(replace(replace(
                     immutable_audit_ref,' ',''),char(9),''),char(10),''),char(13),''))=0
                OR (accepted_audit_append_state='Pending'
                    AND accepted_audit_ref IS NOT NULL)
                OR (accepted_audit_append_state='Appended' AND (
                    accepted_audit_ref IS NULL
                    OR length(replace(replace(replace(replace(
                      accepted_audit_ref,' ',''),char(9),''),char(10),''),char(13),''))=0
                ))
                OR (accepted_audit_ref IS NOT NULL
                    AND length(replace(replace(replace(replace(
                      accepted_audit_ref,' ',''),char(9),''),char(10),''),char(13),''))=0)",
        ),
        (
            "sink_results.delivery_audit_ref",
            "SELECT COUNT(*) FROM sink_results s
             JOIN delivery_decisions d ON d.decision_identity=s.decision_identity
             WHERE (s.delivery_audit_ref IS NOT NULL
                    AND length(replace(replace(replace(replace(
                      s.delivery_audit_ref,' ',''),char(9),''),char(10),''),char(13),''))=0)
                OR (s.authoritative_for_state=1 AND s.result_kind='Accepted'
                    AND d.state IN ('AcceptedTaskTransitionPending','Delivered')
                    AND s.delivery_audit_ref IS NULL)",
        ),
        (
            "task transition hydration audit immutable reference",
            "SELECT COUNT(*)
             FROM task_transition_payloads t
             LEFT JOIN immutable_audit_outbox o
               ON o.audit_identity=t.hydration_ack_identity
              AND o.decision_identity=t.decision_identity
              AND o.audit_kind='ScheduleHydrationApplied'
             WHERE t.hydration_state='Applied'
               AND (t.hydration_ack_identity IS NULL
                    OR t.hydrated_at IS NULL
                    OR o.audit_identity IS NULL
                    OR NOT (
                      (o.append_state='Pending' AND o.immutable_audit_ref IS NULL)
                      OR
                      (o.append_state='Appended'
                        AND o.immutable_audit_ref IS NOT NULL
                        AND length(replace(replace(replace(replace(
                          o.immutable_audit_ref,' ',''),char(9),''),char(10),''),char(13),''))>0)
                    ))",
        ),
    ];

    for (role, query) in INVALID_REFERENCE_QUERIES {
        let invalid_count: i64 = connection.query_row(query, [], |row| row.get(0))?;
        if invalid_count != 0 {
            return Err(DurableDeliveryError::InvalidConfiguration(format!(
                "durable-delivery persisted {role} contains {invalid_count} invalid append-state/reference binding(s)"
            )));
        }
    }
    Ok(())
}

fn require_nonempty_immutable_ref(
    immutable_ref: String,
    record_kind: &str,
    identity: &str,
) -> Result<String> {
    if !has_non_ascii_whitespace(&immutable_ref) {
        return Err(DurableDeliveryError::PolicyMismatch(format!(
            "{record_kind} immutable append returned an empty reference for identity {identity}"
        )));
    }
    Ok(immutable_ref)
}

fn canonical_json(value: &impl Serialize) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            DurableDeliveryError::InvalidEnvelope(format!(
                "stored timestamp is not RFC3339: {error}"
            ))
        })
}
