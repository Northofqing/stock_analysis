//! BR-171/BR-174/BR-178/BR-182 outcome scheduling and settlement ownership.
//!
//! Schedule derivation remains a pure generation-time operation. Production
//! settlement has one owner: it consumes a receipt-verified
//! [`VerifiedOutcomeDue`] and the opaque Magic-TDX admission capability. No
//! public API accepts caller-built due work, run identity, outcome rows or
//! provider evidence.

use crate::data_gateway::outcome_daily_bars::OutcomeAcquisitionFailure;
use crate::data_gateway::{AdmittedOutcomeDailyBars, GatewayError, OutcomeDailyBarsGateway};
use crate::database::selection_v2_read_model::{
    VerifiedOutcomeClaimRecovery, VerifiedOutcomeDue, VerifiedOutcomeSettlementRecovery,
};
use crate::database::selection_v2_repository::OutcomeClaimLifecycleClass;
use crate::database::DatabaseManager;
use crate::selection::outcome_session_gate::{
    outcome_market_session_status, validate_shanghai_tick_instant, OutcomeMarketSessionStatus,
};
use crate::selection::persistence_v2::SelectionV2PersistenceOwner;
use crate::selection::schema_v2::{
    canonical_f64, canonical_json, run_logical_subject_key, sha256_bytes, sha256_json,
    OutcomeAttemptResult, OutcomeErrorFingerprintPreimageV2,
    OutcomeMarketRequestParametersPreimage, OutcomePhase, OutcomeProviderAvailableEvidencePreimage,
    OutcomeReasonCodeV1, OutcomeStageInputPreimage, OutcomeTradingDateVectorPreimage,
    OutcomeTransportAttemptsPreimage, ProviderErrorDetailPreimage, ProviderErrorKind,
    ProviderEvidenceKind, RequestEvidenceColumns, RequestKind, RunLogicalSubjectPreimage,
    RunStatus, SampleKeyPreimage, SelectionOutcomeAttemptRowContentPreimage,
    SelectionSampleOutcomeRowContentPreimage, SubjectKind, DOMAIN_ERROR_FINGERPRINT,
    DOMAIN_OUTCOME_ATTEMPT, DOMAIN_OUTCOME_ATTEMPT_ROW, DOMAIN_OUTCOME_STAGE,
    DOMAIN_OUTCOME_TRADING_DATE_VECTOR, DOMAIN_PROVIDER_ERROR_DETAIL, DOMAIN_RUN_LOGICAL_SUBJECT,
    DOMAIN_SAMPLE_OUTCOME_ROW,
};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, SecondsFormat, Utc, Weekday};
use fs2::FileExt;
use std::collections::BTreeSet;
#[cfg(unix)]
use std::ffi::{CString, OsStr};
use std::fmt;
#[cfg(test)]
use std::fs;
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
#[cfg(unix)]
use std::path::Component;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

const OUTCOME_PROVIDER: &str = "magic-tdx";
const OUTCOME_OPERATION: &str = "outcome_daily_bars";
const OUTCOME_CLAIM_LOCK_RELATIVE_ROOT: &str = "data/locks/production/selection-outcome-claims";
#[cfg(unix)]
const O_RDONLY_FLAG: i32 = 0;
#[cfg(unix)]
const O_RDWR_FLAG: i32 = 2;
#[cfg(target_os = "linux")]
const O_NOFOLLOW_FLAG: i32 = 0x0002_0000;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_NOFOLLOW_FLAG: i32 = 0x0000_0100;
#[cfg(target_os = "linux")]
const O_NONBLOCK_FLAG: i32 = 0x0000_0800;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_NONBLOCK_FLAG: i32 = 0x0000_0004;
#[cfg(target_os = "linux")]
const O_CREAT_FLAG: i32 = 0x0000_0040;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_CREAT_FLAG: i32 = 0x0000_0200;
#[cfg(target_os = "linux")]
const O_CLOEXEC_FLAG: i32 = 0x0008_0000;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const O_CLOEXEC_FLAG: i32 = 0x0100_0000;
#[cfg(target_os = "freebsd")]
const O_CLOEXEC_FLAG: i32 = 0x0010_0000;
#[cfg(target_os = "openbsd")]
const O_CLOEXEC_FLAG: i32 = 0x0001_0000;
#[cfg(target_os = "netbsd")]
const O_CLOEXEC_FLAG: i32 = 0x0040_0000;
#[cfg(target_os = "linux")]
const ELOOP_CODE: i32 = 40;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const ELOOP_CODE: i32 = 62;

#[cfg(unix)]
unsafe extern "C" {
    fn openat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn mkdirat(directory_fd: i32, path: *const std::ffi::c_char, mode: u32) -> i32;
}
static RUN_ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeV2Error {
    pub code: &'static str,
    pub detail: String,
}

impl OutcomeV2Error {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for OutcomeV2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for OutcomeV2Error {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOutcomeSchedule {
    pub evaluation_market_date: NaiveDate,
    pub t0_due_date: NaiveDate,
    pub d1_due_date: NaiveDate,
    pub d2_due_date: NaiveDate,
    pub d3_due_date: NaiveDate,
    pub d4_due_date: NaiveDate,
    pub d5_due_date: NaiveDate,
}

impl StoredOutcomeSchedule {
    pub fn due_date(&self, phase: OutcomePhase) -> NaiveDate {
        match phase {
            OutcomePhase::T0Close => self.t0_due_date,
            OutcomePhase::D1Settled => self.d1_due_date,
            OutcomePhase::D3Settled => self.d3_due_date,
            OutcomePhase::D5Settled => self.d5_due_date,
        }
    }

    pub fn trading_date_vector(&self) -> OutcomeTradingDateVectorPreimage {
        OutcomeTradingDateVectorPreimage {
            domain: DOMAIN_OUTCOME_TRADING_DATE_VECTOR.into(),
            t0: self.t0_due_date.format("%Y-%m-%d").to_string(),
            d1: self.d1_due_date.format("%Y-%m-%d").to_string(),
            d2: self.d2_due_date.format("%Y-%m-%d").to_string(),
            d3: self.d3_due_date.format("%Y-%m-%d").to_string(),
            d4: self.d4_due_date.format("%Y-%m-%d").to_string(),
            d5: self.d5_due_date.format("%Y-%m-%d").to_string(),
        }
    }

    fn validate(&self) -> Result<(), OutcomeV2Error> {
        if self.t0_due_date != self.evaluation_market_date {
            return Err(OutcomeV2Error::new(
                "t0_due_date_mismatch",
                "T0 due date must equal evaluation_market_date",
            ));
        }
        if !(self.t0_due_date < self.d1_due_date
            && self.d1_due_date < self.d2_due_date
            && self.d2_due_date < self.d3_due_date
            && self.d3_due_date < self.d4_due_date
            && self.d4_due_date < self.d5_due_date)
        {
            return Err(OutcomeV2Error::new(
                "outcome_schedule_not_strictly_increasing",
                "stored T0/D1/D2/D3/D4/D5 dates must be strictly increasing",
            ));
        }
        Ok(())
    }
}

/// Derives the immutable schedule from an already source-cited calendar
/// snapshot. Holidays must be absent and the source order must be exact.
pub fn derive_outcome_schedule(
    evaluation_market_date: NaiveDate,
    trading_days: &[NaiveDate],
) -> Result<StoredOutcomeSchedule, OutcomeV2Error> {
    if trading_days.is_empty() {
        return Err(OutcomeV2Error::new(
            "trading_calendar_empty",
            "calendar snapshot has no trading days",
        ));
    }
    for pair in trading_days.windows(2) {
        if pair[0] == pair[1] {
            return Err(OutcomeV2Error::new(
                "trading_calendar_duplicate_date",
                format!("duplicate trading date {}", pair[0]),
            ));
        }
        if pair[0] > pair[1] {
            return Err(OutcomeV2Error::new(
                "trading_calendar_not_sorted",
                "calendar trading dates must be strictly ascending",
            ));
        }
    }
    if let Some(weekend) = trading_days
        .iter()
        .find(|date| matches!(date.weekday(), Weekday::Sat | Weekday::Sun))
    {
        return Err(OutcomeV2Error::new(
            "trading_calendar_weekend_date",
            format!("{weekend} cannot be an A-share trading session"),
        ));
    }
    let t0_index = trading_days
        .binary_search(&evaluation_market_date)
        .map_err(|_| {
            OutcomeV2Error::new(
                "evaluation_date_not_trading_day",
                format!("{evaluation_market_date} is absent from the calendar snapshot"),
            )
        })?;
    let at_offset = |offset: usize, label: &'static str| {
        trading_days.get(t0_index + offset).copied().ok_or_else(|| {
            OutcomeV2Error::new(
                "trading_calendar_coverage_incomplete",
                format!("calendar snapshot has no {label} session"),
            )
        })
    };
    let schedule = StoredOutcomeSchedule {
        evaluation_market_date,
        t0_due_date: evaluation_market_date,
        d1_due_date: at_offset(1, "D1")?,
        d2_due_date: at_offset(2, "D2")?,
        d3_due_date: at_offset(3, "D3")?,
        d4_due_date: at_offset(4, "D4")?,
        d5_due_date: at_offset(5, "D5")?,
    };
    schedule.validate()?;
    Ok(schedule)
}

/// Sole production owner for outcome settlement-stage construction.
///
/// `settle_due` consumes the opaque due capability. The owner creates its own
/// UUIDv7 run identity from the scheduler's single checked `+08:00` tick
/// instant, gates the current session before any Gateway call, and is the only
/// module that turns admitted market evidence into an opaque persistence
/// capability.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutcomeSettlementOwner;

/// Frozen public completion algebra for the dedicated settlement owner.
#[derive(Debug)]
pub enum OutcomeSettlementOwnerResult {
    Receipted(crate::database::selection_v2_repository::CommitReceipt),
    LiveOwnedSkip(OutcomeSettlementObservation),
    Superseded(OutcomeSettlementObservation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeSettlementDisposition {
    LiveOwnedSkip,
    Superseded,
}

impl OutcomeSettlementDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LiveOwnedSkip => "live_owned_skip",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeSettlementObservation {
    pub disposition: OutcomeSettlementDisposition,
    pub logical_subject_key: String,
    pub verified_due_snapshot_hash: String,
    pub reason_code: &'static str,
}

impl OutcomeSettlementObservation {
    fn live_owned(logical_subject_key: String, verified_due_snapshot_hash: String) -> Self {
        Self {
            disposition: OutcomeSettlementDisposition::LiveOwnedSkip,
            logical_subject_key,
            verified_due_snapshot_hash,
            reason_code: "subject_lock_live_owned",
        }
    }

    fn superseded_due(logical_subject_key: String, verified_due_snapshot_hash: String) -> Self {
        Self {
            disposition: OutcomeSettlementDisposition::Superseded,
            logical_subject_key,
            verified_due_snapshot_hash,
            reason_code: "locked_due_superseded",
        }
    }

    fn superseded_recovery(
        logical_subject_key: String,
        verified_due_snapshot_hash: String,
    ) -> Self {
        Self {
            disposition: OutcomeSettlementDisposition::Superseded,
            logical_subject_key,
            verified_due_snapshot_hash,
            reason_code: "locked_recovery_superseded",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutcomeSettlementTickSummary {
    pub recovered_non_outcome: usize,
    pub recovered: usize,
    pub settled_due: usize,
    pub live_owned_skips: usize,
    pub superseded: usize,
    pub observations: Vec<OutcomeSettlementObservation>,
}

enum OutcomeSubjectLockAttempt {
    Acquired(OutcomeSubjectLockGuard),
    LiveOwned,
}

/// Process/cross-process ownership of one exact outcome logical subject.
///
/// The descriptor—not the file timestamp—is the authority. The retained lock
/// object is never deleted or treated as mutable state.
struct OutcomeSubjectLockGuard {
    file: File,
    #[cfg(unix)]
    _parent_directory: File,
    logical_subject_key: String,
    device: u64,
    inode: u64,
}

impl OutcomeSubjectLockGuard {
    fn try_acquire_production(
        logical_subject_key: &str,
    ) -> Result<OutcomeSubjectLockAttempt, OutcomeV2Error> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(OUTCOME_CLAIM_LOCK_RELATIVE_ROOT);
        Self::try_acquire_at(&root, logical_subject_key)
    }

    #[cfg(test)]
    fn try_acquire_for_test(
        root: &Path,
        logical_subject_key: &str,
    ) -> Result<OutcomeSubjectLockAttempt, OutcomeV2Error> {
        Self::try_acquire_at(
            &root.join("test/selection-outcome-claims"),
            logical_subject_key,
        )
    }

    #[cfg(unix)]
    fn try_acquire_at(
        root: &Path,
        logical_subject_key: &str,
    ) -> Result<OutcomeSubjectLockAttempt, OutcomeV2Error> {
        require_sha256(logical_subject_key, "outcome_claim_lock_key")?;
        let parent_directory = open_or_create_pinned_lock_directory(root)?;
        let leaf = format!("{logical_subject_key}.lock");
        let file =
            openat_lock_component(&parent_directory, OsStr::new(&leaf), true).map_err(|error| {
                if error.raw_os_error() == Some(ELOOP_CODE) {
                    return OutcomeV2Error::new(
                        "outcome_subject_lock_object_invalid",
                        format!("claim lock leaf is a symlink: {}/{}", root.display(), leaf),
                    );
                }
                OutcomeV2Error::new(
                    "outcome_subject_lock_open_failed",
                    format!(
                        "descriptor-open fixed claim lock {}/{}: {error}",
                        root.display(),
                        leaf
                    ),
                )
            })?;
        let path = root.join(&leaf);
        let metadata = file.metadata().map_err(|error| {
            OutcomeV2Error::new(
                "outcome_subject_lock_stat_failed",
                format!("fstat fixed claim lock {}: {error}", path.display()),
            )
        })?;
        if !metadata.is_file() {
            return Err(OutcomeV2Error::new(
                "outcome_subject_lock_object_invalid",
                "opened claim lock descriptor is not a regular file",
            ));
        }
        let (device, inode) = (metadata.dev(), metadata.ino());
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Ok(OutcomeSubjectLockAttempt::LiveOwned)
            }
            Err(error) => {
                return Err(OutcomeV2Error::new(
                    "outcome_subject_lock_failed",
                    format!("acquire fixed claim lock {}: {error}", path.display()),
                ))
            }
        }
        {
            let rebound = openat_lock_component(&parent_directory, OsStr::new(&leaf), false)
                .map_err(|error| {
                    OutcomeV2Error::new(
                        "outcome_subject_lock_path_restat_failed",
                        format!(
                            "descriptor-reopen fixed claim lock {}/{}: {error}",
                            root.display(),
                            leaf
                        ),
                    )
                })?;
            let rebound_metadata = rebound.metadata().map_err(|error| {
                OutcomeV2Error::new(
                    "outcome_subject_lock_path_restat_failed",
                    format!(
                        "fstat descriptor-reopened claim lock {}/{}: {error}",
                        root.display(),
                        leaf
                    ),
                )
            })?;
            if !rebound_metadata.is_file()
                || rebound_metadata.dev() != device
                || rebound_metadata.ino() != inode
            {
                let _ = FileExt::unlock(&file);
                return Err(OutcomeV2Error::new(
                    "outcome_subject_lock_path_identity_changed",
                    "retained parent descriptor no longer resolves the held regular lock file",
                ));
            }
        }
        let after = file.metadata().map_err(|error| {
            OutcomeV2Error::new(
                "outcome_subject_lock_restat_failed",
                format!("restat fixed claim lock {}: {error}", path.display()),
            )
        })?;
        #[cfg(unix)]
        if after.dev() != device || after.ino() != inode {
            let _ = FileExt::unlock(&file);
            return Err(OutcomeV2Error::new(
                "outcome_subject_lock_identity_changed",
                "claim lock descriptor identity changed across acquisition",
            ));
        }
        Ok(OutcomeSubjectLockAttempt::Acquired(Self {
            file,
            _parent_directory: parent_directory,
            logical_subject_key: logical_subject_key.into(),
            device,
            inode,
        }))
    }

    #[cfg(not(unix))]
    fn try_acquire_at(
        _root: &Path,
        logical_subject_key: &str,
    ) -> Result<OutcomeSubjectLockAttempt, OutcomeV2Error> {
        require_sha256(logical_subject_key, "outcome_claim_lock_key")?;
        Err(OutcomeV2Error::new(
            "outcome_subject_lock_descriptor_unsupported",
            "outcome subject ownership requires descriptor-relative no-follow filesystem APIs",
        ))
    }

    fn prepare_new_claim(
        &self,
        due: &VerifiedOutcomeDue,
        tick_at: &DateTime<FixedOffset>,
    ) -> Result<PreparedOutcomeClaimStage, OutcomeV2Error> {
        if self.logical_subject_key != due.logical_subject_key() {
            return Err(OutcomeV2Error::new(
                "outcome_subject_lock_key_mismatch",
                "held subject lock does not own the verified due logical subject",
            ));
        }
        validate_shanghai_tick_instant(tick_at)
            .map_err(|error| OutcomeV2Error::new(error.reason_code(), error.to_string()))?;
        let now = tick_at
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Nanos, true);
        let claim_id = new_uuid_v7(&now, due.verified_due_snapshot_hash())?;
        let planned_outcome_run_id = new_uuid_v7(
            &now,
            &sha256_bytes(format!("{claim_id}:planned").as_bytes()),
        )?;
        due.prepare_outcome_claim(claim_id, planned_outcome_run_id)
            .map_err(|error| {
                OutcomeV2Error::new("outcome_claim_read_model_builder_failed", error.to_string())
            })
    }

    #[allow(
        dead_code,
        reason = "BR-183 keeps new selection-v2 outcome recovery disabled until activation evidence closes"
    )]
    fn prepare_recovery(
        &self,
        recovery: VerifiedOutcomeClaimRecovery,
    ) -> Result<PreparedOutcomeClaimStage, OutcomeV2Error> {
        if self.logical_subject_key != recovery.claim_lock_key() {
            return Err(OutcomeV2Error::new(
                "outcome_claim_recovery_lock_key_mismatch",
                "held subject lock does not own the exact recoverable claim",
            ));
        }
        recovery.into_prepared().map_err(|error| {
            OutcomeV2Error::new(
                "outcome_claim_recovery_capability_invalid",
                error.to_string(),
            )
        })
    }
}

impl Drop for OutcomeSubjectLockGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        debug_assert_eq!(
            self.file
                .metadata()
                .ok()
                .map(|metadata| (metadata.dev(), metadata.ino())),
            Some((self.device, self.inode)),
            "outcome subject lock descriptor changed identity"
        );
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(unix)]
fn validate_absolute_lock_path(path: &Path) -> Result<(), OutcomeV2Error> {
    if !path.is_absolute() {
        return Err(OutcomeV2Error::new(
            "outcome_subject_lock_path_not_absolute",
            "claim lock root must be manifest-anchored and absolute",
        ));
    }
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => {}
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(OutcomeV2Error::new(
                    "outcome_subject_lock_path_unsafe",
                    format!(
                        "claim lock path contains a forbidden component: {}",
                        path.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn lock_component_cstring(name: &OsStr) -> Result<CString, std::io::Error> {
    if name.is_empty() || name.as_bytes().contains(&b'/') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "descriptor-relative lock component must be one non-empty segment",
        ));
    }
    CString::new(name.as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "descriptor-relative lock component contains NUL",
        )
    })
}

#[cfg(unix)]
fn openat_lock_component(
    parent: &File,
    name: &OsStr,
    create: bool,
) -> Result<File, std::io::Error> {
    let name = lock_component_cstring(name)?;
    let create_flag = if create { O_CREAT_FLAG } else { 0 };
    // SAFETY: `name` is one live NUL-terminated component, `parent` retains a
    // directory descriptor, and successful `openat` returns one owned fd.
    let descriptor = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            O_RDWR_FLAG | create_flag | O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG,
            0o600_u32,
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the successful descriptor is newly owned by this call.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn openat_lock_directory_component(parent: &File, name: &OsStr) -> Result<File, std::io::Error> {
    let name = lock_component_cstring(name)?;
    // SAFETY: `name` and `parent` satisfy the same invariants as
    // `openat_lock_component`; no create mode argument is consumed here.
    let descriptor = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            O_RDONLY_FLAG | O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG,
        )
    };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the successful descriptor is newly owned by this call.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn mkdirat_lock_component(parent: &File, name: &OsStr) -> Result<(), std::io::Error> {
    let name = lock_component_cstring(name)?;
    // SAFETY: `name` is one live NUL-terminated component and `parent`
    // retains a directory descriptor.
    let result = unsafe { mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700_u32) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(error);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn open_or_create_pinned_lock_directory(root: &Path) -> Result<File, OutcomeV2Error> {
    validate_absolute_lock_path(root)?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG);
    let mut directory = options.open("/").map_err(|error| {
        OutcomeV2Error::new(
            "outcome_subject_lock_path_unavailable",
            format!(
                "descriptor-open filesystem root for {}: {error}",
                root.display()
            ),
        )
    })?;
    let mut traversed = Path::new("/").to_path_buf();
    for component in root.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let next = match openat_lock_directory_component(&directory, name) {
            Ok(next) => next,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                mkdirat_lock_component(&directory, name).map_err(|error| {
                    OutcomeV2Error::new(
                        "outcome_subject_lock_root_create_failed",
                        format!(
                            "descriptor-create claim-lock directory {}: {error}",
                            traversed.join(name).display()
                        ),
                    )
                })?;
                directory.sync_all().map_err(|error| {
                    OutcomeV2Error::new(
                        "outcome_subject_lock_root_sync_failed",
                        format!(
                            "sync parent after creating {}: {error}",
                            traversed.join(name).display()
                        ),
                    )
                })?;
                openat_lock_directory_component(&directory, name).map_err(|error| {
                    OutcomeV2Error::new(
                        "outcome_subject_lock_path_unavailable",
                        format!(
                            "descriptor-open created claim-lock directory {}: {error}",
                            traversed.join(name).display()
                        ),
                    )
                })?
            }
            Err(error) => {
                return Err(OutcomeV2Error::new(
                    "outcome_subject_lock_path_unsafe",
                    format!(
                        "descriptor-traverse claim-lock directory {}: {error}",
                        traversed.join(name).display()
                    ),
                ));
            }
        };
        let metadata = next.metadata().map_err(|error| {
            OutcomeV2Error::new(
                "outcome_subject_lock_path_unavailable",
                format!(
                    "fstat descriptor-traversed claim-lock directory {}: {error}",
                    traversed.join(name).display()
                ),
            )
        })?;
        if !metadata.is_dir() {
            return Err(OutcomeV2Error::new(
                "outcome_subject_lock_path_unsafe",
                format!(
                    "claim-lock component is not a directory: {}",
                    traversed.join(name).display()
                ),
            ));
        }
        traversed.push(name);
        directory = next;
    }
    Ok(directory)
}

/// Opaque, one-shot persistence capability for an owner-built outcome stage.
///
/// The raw schema preimage is deliberately private and has no public accessor:
/// production callers can only move this value into the BR-174 persistence
/// owner.
#[derive(Debug)]
pub struct PreparedOutcomeStage {
    stage_input: OutcomeStageInputPreimage,
}

impl PreparedOutcomeStage {
    pub(crate) fn validated(
        stage_input: OutcomeStageInputPreimage,
    ) -> Result<Self, OutcomeV2Error> {
        stage_input.validate().map_err(schema_error)?;
        Ok(Self { stage_input })
    }

    pub(crate) fn into_stage_input(self) -> OutcomeStageInputPreimage {
        self.stage_input
    }
}

/// Opaque, fully validated outcome-claim intent consumed by the sole durable
/// persistence owner. The raw claim DTO is never a public persistence input.
#[derive(Debug)]
pub struct PreparedOutcomeClaimStage {
    stage_input: crate::selection::schema_v2::OutcomeClaimStageInputPreimage,
}

impl PreparedOutcomeClaimStage {
    pub(crate) fn validated(
        stage_input: crate::selection::schema_v2::OutcomeClaimStageInputPreimage,
    ) -> Result<Self, OutcomeV2Error> {
        stage_input.validate().map_err(schema_error)?;
        Ok(Self { stage_input })
    }

    pub(crate) fn into_stage_input(
        self,
    ) -> crate::selection::schema_v2::OutcomeClaimStageInputPreimage {
        self.stage_input
    }

    pub(crate) fn claim_id(&self) -> &str {
        &self.stage_input.stage_run_id
    }

    pub(crate) fn planned_outcome_run_id(&self) -> &str {
        &self.stage_input.planned_outcome_run_id
    }
}

/// Exact durable claim lineage required before any outcome provider request.
///
/// Construction is crate-private so only the persistence owner can mint this
/// capability after verifying the claim manifest, committed audit record, and
/// receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptedOutcomeClaim {
    outcome_claim_id: String,
    planned_outcome_run_id: String,
    receipt_content_hash: String,
    due_binding_hash: String,
    provider_request_hash: String,
}

impl ReceiptedOutcomeClaim {
    pub(crate) fn validated(
        outcome_claim_id: String,
        planned_outcome_run_id: String,
        receipt_content_hash: String,
        due_binding_hash: String,
        provider_request_hash: String,
    ) -> Result<Self, OutcomeV2Error> {
        require_canonical_uuid_v7(&outcome_claim_id, "outcome_claim_id")?;
        require_canonical_uuid_v7(&planned_outcome_run_id, "planned_outcome_run_id")?;
        if outcome_claim_id == planned_outcome_run_id {
            return Err(OutcomeV2Error::new(
                "outcome_claim_run_identity_collision",
                "claim and planned outcome-run IDs must differ",
            ));
        }
        require_sha256(&receipt_content_hash, "outcome_claim_receipt_content_hash")?;
        require_sha256(&due_binding_hash, "outcome_claim_due_binding_hash")?;
        require_sha256(
            &provider_request_hash,
            "outcome_claim_provider_request_hash",
        )?;
        Ok(Self {
            outcome_claim_id,
            planned_outcome_run_id,
            receipt_content_hash,
            due_binding_hash,
            provider_request_hash,
        })
    }
}

impl OutcomeSettlementOwner {
    pub const fn new() -> Self {
        Self
    }

    /// Owns the complete production claim/provider/outcome choreography for
    /// one previously verified due capability.
    ///
    /// Lock order is fixed:
    ///
    /// 1. retained per-logical-subject OS lock;
    /// 2. fresh receipt/audit/database due revalidation;
    /// 3. claim persistence (its selection-audit/SQLite critical sections);
    /// 4. provider I/O while only the subject lock remains held;
    /// 5. outcome persistence and receipt read-back.
    ///
    /// A changed high-water invalidates the caller's due capability and
    /// returns `Superseded` without creating a claim or calling the provider.
    async fn settle_due(
        &self,
        due: VerifiedOutcomeDue,
        tick_at: DateTime<FixedOffset>,
        gateway: &OutcomeDailyBarsGateway,
    ) -> Result<OutcomeSettlementOwnerResult, OutcomeV2Error> {
        let logical_subject_key = due.logical_subject_key().to_owned();
        let verified_due_snapshot_hash = due.verified_due_snapshot_hash().to_owned();
        let subject_lock =
            match OutcomeSubjectLockGuard::try_acquire_production(&logical_subject_key)? {
                OutcomeSubjectLockAttempt::Acquired(lock) => lock,
                OutcomeSubjectLockAttempt::LiveOwned => {
                    return Ok(OutcomeSettlementOwnerResult::LiveOwnedSkip(
                        OutcomeSettlementObservation::live_owned(
                            logical_subject_key,
                            verified_due_snapshot_hash,
                        ),
                    ));
                }
            };
        let database = DatabaseManager::try_get().ok_or_else(|| {
            OutcomeV2Error::new(
                "outcome_database_not_initialized",
                "production outcome owner requires the process-owned database",
            )
        })?;
        let Some(fresh_due) = database
            .revalidate_outcome_due_for_claim(&due, tick_at)
            .map_err(|error| {
                OutcomeV2Error::new(
                    "outcome_due_revalidation_failed",
                    format!("fresh locked due revalidation failed: {error}"),
                )
            })?
        else {
            return Ok(OutcomeSettlementOwnerResult::Superseded(
                OutcomeSettlementObservation::superseded_due(
                    logical_subject_key,
                    verified_due_snapshot_hash,
                ),
            ));
        };
        let claim_bound_due = gateway
            .bind_claim_request(fresh_due, tick_at)
            .map_err(|error| {
                OutcomeV2Error::new(
                    "outcome_claim_provider_request_binding_failed",
                    error.to_string(),
                )
            })?;
        let prepared_claim = subject_lock.prepare_new_claim(&claim_bound_due, &tick_at)?;
        let receipted_claim = SelectionV2PersistenceOwner::commit_outcome_claim(prepared_claim)
            .map_err(|error| {
                OutcomeV2Error::new(
                    "outcome_claim_persistence_failed",
                    format!("durable outcome claim failed: {error}"),
                )
            })?;
        let prepared_outcome = self
            .prepare_receipted_outcome(claim_bound_due, receipted_claim, tick_at, gateway)
            .await?;
        let receipt =
            SelectionV2PersistenceOwner::commit_outcome(prepared_outcome).map_err(|error| {
                OutcomeV2Error::new(
                    "outcome_persistence_failed",
                    format!("durable outcome receipt failed: {error}"),
                )
            })?;
        Ok(OutcomeSettlementOwnerResult::Receipted(receipt))
    }

    async fn recover_verified(
        &self,
        recovery: VerifiedOutcomeSettlementRecovery,
        tick_at: DateTime<FixedOffset>,
        gateway: &OutcomeDailyBarsGateway,
    ) -> Result<OutcomeSettlementOwnerResult, OutcomeV2Error> {
        let logical_subject_key = recovery.logical_subject_key().to_owned();
        let verified_due_snapshot_hash = recovery.verified_due_snapshot_hash().to_owned();
        let (claim_id, planned_outcome_run_id, class) = {
            let (claim_id, planned_outcome_run_id, class) = recovery.stable_identity();
            (
                claim_id.to_owned(),
                planned_outcome_run_id.to_owned(),
                class,
            )
        };
        let subject_lock =
            match OutcomeSubjectLockGuard::try_acquire_production(&logical_subject_key)? {
                OutcomeSubjectLockAttempt::Acquired(lock) => lock,
                OutcomeSubjectLockAttempt::LiveOwned => {
                    return Ok(OutcomeSettlementOwnerResult::LiveOwnedSkip(
                        OutcomeSettlementObservation::live_owned(
                            logical_subject_key,
                            verified_due_snapshot_hash,
                        ),
                    ));
                }
            };
        let database = DatabaseManager::try_get().ok_or_else(|| {
            OutcomeV2Error::new(
                "outcome_database_not_initialized",
                "production outcome recovery requires the process-owned database",
            )
        })?;
        let Some(fresh) = database
            .revalidate_outcome_settlement_recovery(
                &logical_subject_key,
                &claim_id,
                &planned_outcome_run_id,
                class,
            )
            .map_err(|error| {
                OutcomeV2Error::new(
                    "outcome_recovery_revalidation_failed",
                    format!("fresh locked recovery revalidation failed: {error}"),
                )
            })?
        else {
            return Ok(OutcomeSettlementOwnerResult::Superseded(
                OutcomeSettlementObservation::superseded_recovery(
                    logical_subject_key,
                    verified_due_snapshot_hash,
                ),
            ));
        };

        match fresh {
            VerifiedOutcomeSettlementRecovery::ClaimPartial { due, claim } => {
                if subject_lock.logical_subject_key != due.logical_subject_key() {
                    return Err(OutcomeV2Error::new(
                        "outcome_claim_recovery_lock_key_mismatch",
                        "held subject lock does not own the exact partial claim",
                    ));
                }
                let receipted_claim = SelectionV2PersistenceOwner::commit_outcome_claim(claim)
                    .map_err(|error| {
                        OutcomeV2Error::new(
                            "outcome_claim_recovery_persistence_failed",
                            format!("exact partial claim recovery failed: {error}"),
                        )
                    })?;
                let prepared_outcome = self
                    .prepare_receipted_outcome(due, receipted_claim, tick_at, gateway)
                    .await?;
                let receipt = SelectionV2PersistenceOwner::commit_outcome(prepared_outcome)
                    .map_err(|error| {
                        OutcomeV2Error::new(
                            "outcome_recovery_persistence_failed",
                            format!("outcome after partial claim recovery failed: {error}"),
                        )
                    })?;
                Ok(OutcomeSettlementOwnerResult::Receipted(receipt))
            }
            VerifiedOutcomeSettlementRecovery::ClaimActive {
                due,
                claim,
                claim_receipt_content_hash,
            } => {
                if subject_lock.logical_subject_key != due.logical_subject_key() {
                    return Err(OutcomeV2Error::new(
                        "outcome_claim_recovery_lock_key_mismatch",
                        "held subject lock does not own the exact active claim",
                    ));
                }
                let receipted_claim = ReceiptedOutcomeClaim::validated(
                    claim.claim_id().to_owned(),
                    claim.planned_outcome_run_id().to_owned(),
                    claim_receipt_content_hash,
                    due.claim_due_binding_hash().to_owned(),
                    due.provider_request_hash().to_owned(),
                )?;
                let prepared_outcome = self
                    .prepare_receipted_outcome(due, receipted_claim, tick_at, gateway)
                    .await?;
                let receipt = SelectionV2PersistenceOwner::commit_outcome(prepared_outcome)
                    .map_err(|error| {
                        OutcomeV2Error::new(
                            "outcome_recovery_persistence_failed",
                            format!("outcome after active claim recovery failed: {error}"),
                        )
                    })?;
                Ok(OutcomeSettlementOwnerResult::Receipted(receipt))
            }
            VerifiedOutcomeSettlementRecovery::OutcomeRecovery {
                logical_subject_key: recovered_key,
                verified_due_snapshot_hash: _,
                claim_id: recovered_claim_id,
                planned_outcome_run_id: recovered_run_id,
                outcome,
            } => {
                if subject_lock.logical_subject_key != recovered_key
                    || recovered_claim_id != claim_id
                    || recovered_run_id != planned_outcome_run_id
                    || class != OutcomeClaimLifecycleClass::OutcomeRecovery
                {
                    return Err(OutcomeV2Error::new(
                        "outcome_recovery_exact_identity_mismatch",
                        "locked recovery capability changed claim/planned-run identity",
                    ));
                }
                // The exact outcome envelope is already durable. This branch
                // intentionally has no Gateway argument use and cannot refetch
                // provider evidence.
                let receipt =
                    SelectionV2PersistenceOwner::commit_outcome(outcome).map_err(|error| {
                        OutcomeV2Error::new(
                            "outcome_recovery_persistence_failed",
                            format!("exact outcome envelope recovery failed: {error}"),
                        )
                    })?;
                Ok(OutcomeSettlementOwnerResult::Receipted(receipt))
            }
        }
    }

    /// Production scheduler coordinator. `tick_at` is the sole instant used
    /// for recovery, due reads, locked revalidation, claim identity,
    /// session gating and provider admission. Recovery is drained and freshly
    /// revalidated before any new due claim can be created.
    pub async fn settle_tick(
        &self,
        tick_at: DateTime<FixedOffset>,
        limit: i64,
        gateway: &OutcomeDailyBarsGateway,
    ) -> Result<OutcomeSettlementTickSummary, OutcomeV2Error> {
        validate_shanghai_tick_instant(&tick_at)
            .map_err(|error| OutcomeV2Error::new(error.reason_code(), error.to_string()))?;
        let database = DatabaseManager::try_get().ok_or_else(|| {
            OutcomeV2Error::new(
                "outcome_database_not_initialized",
                "production outcome tick requires the process-owned database",
            )
        })?;
        let non_outcome_recovery = database
            .with_verified_selection_v2_recovery_model(|model| {
                Ok(model.recovery_queues()?.into_ordered_non_outcome())
            })
            .map_err(|error| {
                OutcomeV2Error::new(
                    "non_outcome_recovery_read_failed",
                    format!("cannot materialize typed non-outcome recovery work: {error}"),
                )
            })?;
        let mut summary = OutcomeSettlementTickSummary::default();
        for request in non_outcome_recovery {
            let stage_run_id = request.stage_run_id().to_owned();
            SelectionV2PersistenceOwner::recover_non_outcome(request).map_err(|error| {
                OutcomeV2Error::new(
                    "non_outcome_recovery_persistence_failed",
                    format!("cannot recover exact non-outcome stage {stage_run_id}: {error}"),
                )
            })?;
            summary.recovered_non_outcome += 1;
        }
        let non_outcome_remaining = database
            .with_verified_selection_v2_recovery_model(|model| {
                Ok(model.recovery_queues()?.into_ordered_non_outcome().len())
            })
            .map_err(|error| {
                OutcomeV2Error::new(
                    "non_outcome_recovery_drain_recheck_failed",
                    format!("cannot verify typed non-outcome recovery drain: {error}"),
                )
            })?;
        if non_outcome_remaining > 0 {
            return Err(OutcomeV2Error::new(
                "non_outcome_recovery_not_drained",
                format!(
                    "{non_outcome_remaining} exact non-outcome stages remain; new provider work is blocked"
                ),
            ));
        }
        let recovery = database
            .with_verified_selection_v2_recovery_model(|model| model.outcome_settlement_recovery())
            .map_err(|error| {
                OutcomeV2Error::new(
                    "outcome_recovery_read_failed",
                    format!("cannot materialize verified recovery work: {error}"),
                )
            })?;
        let mut live_owned_recovery_subjects = BTreeSet::new();
        for item in recovery {
            match self.recover_verified(item, tick_at, gateway).await? {
                OutcomeSettlementOwnerResult::Receipted(_) => summary.recovered += 1,
                OutcomeSettlementOwnerResult::LiveOwnedSkip(observation) => {
                    live_owned_recovery_subjects.insert(observation.logical_subject_key.clone());
                    summary.live_owned_skips += 1;
                    summary.observations.push(observation);
                }
                OutcomeSettlementOwnerResult::Superseded(observation) => {
                    summary.superseded += 1;
                    summary.observations.push(observation);
                }
            }
        }
        let recovery_remaining = database
            .with_verified_selection_v2_recovery_model(|model| model.outcome_settlement_recovery())
            .map_err(|error| {
                OutcomeV2Error::new(
                    "outcome_recovery_drain_recheck_failed",
                    format!("cannot verify recovery drain: {error}"),
                )
            })?;
        let blocking_recovery_count = recovery_remaining
            .iter()
            .filter(|item| !live_owned_recovery_subjects.contains(item.logical_subject_key()))
            .count();
        if blocking_recovery_count > 0 {
            return Err(OutcomeV2Error::new(
                "outcome_recovery_not_drained",
                format!(
                    "{} exact claim lifecycles remain; new provider work is blocked",
                    blocking_recovery_count
                ),
            ));
        }
        let due = database
            .with_verified_selection_v2_read_model(|model| model.due_v2_outcomes(tick_at, limit))
            .map_err(|error| {
                OutcomeV2Error::new(
                    "outcome_due_read_failed",
                    format!("cannot materialize verified due work: {error}"),
                )
            })?;
        for item in due {
            match self.settle_due(item, tick_at, gateway).await? {
                OutcomeSettlementOwnerResult::Receipted(_) => summary.settled_due += 1,
                OutcomeSettlementOwnerResult::LiveOwnedSkip(observation) => {
                    summary.live_owned_skips += 1;
                    summary.observations.push(observation);
                }
                OutcomeSettlementOwnerResult::Superseded(observation) => {
                    summary.superseded += 1;
                    summary.observations.push(observation);
                }
            }
        }
        Ok(summary)
    }

    async fn prepare_receipted_outcome(
        &self,
        due: VerifiedOutcomeDue,
        claim: ReceiptedOutcomeClaim,
        tick_at: DateTime<FixedOffset>,
        gateway: &OutcomeDailyBarsGateway,
    ) -> Result<PreparedOutcomeStage, OutcomeV2Error> {
        validate_shanghai_tick_instant(&tick_at)
            .map_err(|error| OutcomeV2Error::new(error.reason_code(), error.to_string()))?;
        let attempted_at = tick_at.with_timezone(&Utc);
        let identity = SettlementIdentity::from_verified_due(&due)?;
        let run = OwnerRun::new(&identity, attempted_at, claim)?;

        match outcome_market_session_status(identity.stored_due_date, tick_at)
            .map_err(|error| OutcomeV2Error::new(error.reason_code(), error.to_string()))?
        {
            OutcomeMarketSessionStatus::Incomplete => {
                return build_expected_wait_stage(&identity, &run).map(prepared_stage);
            }
            OutcomeMarketSessionStatus::Complete => {}
        }

        match gateway.acquire(&due, tick_at).await {
            Ok(admitted) => build_settled_stage(&identity, &run, admitted).map(prepared_stage),
            Err(failure) => build_error_stage(&identity, &run, failure).map(prepared_stage),
        }
    }
}

fn prepared_stage(stage_input: OutcomeStageInputPreimage) -> PreparedOutcomeStage {
    PreparedOutcomeStage { stage_input }
}

#[derive(Debug, Clone)]
struct SettlementIdentity {
    sample_key: String,
    sample_key_preimage: SampleKeyPreimage,
    config_activation_run_id: String,
    config_hash: String,
    phase: OutcomePhase,
    stored_due_date: NaiveDate,
    window_start: NaiveDate,
    window_end: NaiveDate,
    calendar_version: String,
    calendar_hash: String,
    trading_date_vector: OutcomeTradingDateVectorPreimage,
    trading_date_vector_hash: String,
    applicable_trading_dates: Vec<NaiveDate>,
    request_binding_hash: String,
    claim_due_binding_hash: String,
    provider_request_hash: String,
    receipted_t0_close: Option<String>,
    receipted_t0_volume: Option<String>,
}

impl SettlementIdentity {
    fn from_verified_due(due: &VerifiedOutcomeDue) -> Result<Self, OutcomeV2Error> {
        if sha256_json(due.sample_key_preimage()).map_err(schema_error)? != due.sample_key() {
            return Err(OutcomeV2Error::new(
                "outcome_sample_key_preimage_mismatch",
                "verified due sample key does not match its authoritative preimage",
            ));
        }
        require_sha256(due.config_hash(), "config_hash")?;
        require_sha256(due.request_binding_hash(), "request_binding_hash")?;
        require_sha256(due.claim_due_binding_hash(), "claim_due_binding_hash")?;
        require_sha256(due.provider_request_hash(), "provider_request_hash")?;
        Ok(Self {
            sample_key: due.sample_key().to_owned(),
            sample_key_preimage: due.sample_key_preimage().clone(),
            config_activation_run_id: due.config_activation_run_id().to_owned(),
            config_hash: due.config_hash().to_owned(),
            phase: due.phase(),
            stored_due_date: due.stored_due_date(),
            window_start: due.window_start(),
            window_end: due.window_end(),
            calendar_version: due.calendar_version().to_owned(),
            calendar_hash: due.calendar_hash().to_owned(),
            trading_date_vector: due.trading_date_vector().clone(),
            trading_date_vector_hash: due.trading_date_vector_hash().to_owned(),
            applicable_trading_dates: due.applicable_trading_dates().to_vec(),
            request_binding_hash: due.request_binding_hash().to_owned(),
            claim_due_binding_hash: due.claim_due_binding_hash().to_owned(),
            provider_request_hash: due.provider_request_hash().to_owned(),
            receipted_t0_close: due.t0_close().map(str::to_owned),
            receipted_t0_volume: due.t0_volume().map(str::to_owned),
        })
    }
}

#[derive(Debug, Clone)]
struct OwnerRun {
    stage_run_id: String,
    logical_subject_key: String,
    attempted_at: String,
    outcome_claim_id: String,
    outcome_claim_receipt_content_hash: String,
    outcome_claim_due_binding_hash: String,
    outcome_claim_provider_request_hash: String,
}

impl OwnerRun {
    fn new(
        identity: &SettlementIdentity,
        attempted_at: DateTime<Utc>,
        claim: ReceiptedOutcomeClaim,
    ) -> Result<Self, OutcomeV2Error> {
        let attempted_at = attempted_at.to_rfc3339_opts(SecondsFormat::Nanos, true);
        if claim.provider_request_hash != identity.provider_request_hash {
            return Err(OutcomeV2Error::new(
                "outcome_claim_provider_request_mismatch",
                "receipted claim request does not bind the verified due request",
            ));
        }
        if claim.due_binding_hash != identity.claim_due_binding_hash {
            return Err(OutcomeV2Error::new(
                "outcome_claim_due_binding_mismatch",
                "receipted claim due binding does not bind the verified due snapshot",
            ));
        }
        let stage_run_id = claim.planned_outcome_run_id;
        let logical_subject_key = run_logical_subject_key(&RunLogicalSubjectPreimage {
            domain: DOMAIN_RUN_LOGICAL_SUBJECT.into(),
            subject_kind: SubjectKind::OutcomeRun,
            source_fact_key: None,
            config_hash: Some(identity.config_hash.clone()),
            sample_key: Some(identity.sample_key.clone()),
            outcome_phase: Some(identity.phase),
            stored_due_date: Some(identity.stored_due_date.format("%Y-%m-%d").to_string()),
            ingress_source_batch_hash: None,
        })
        .map_err(schema_error)?;
        Ok(Self {
            stage_run_id,
            logical_subject_key,
            attempted_at,
            outcome_claim_id: claim.outcome_claim_id,
            outcome_claim_receipt_content_hash: claim.receipt_content_hash,
            outcome_claim_due_binding_hash: claim.due_binding_hash,
            outcome_claim_provider_request_hash: claim.provider_request_hash,
        })
    }
}

fn build_expected_wait_stage(
    identity: &SettlementIdentity,
    run: &OwnerRun,
) -> Result<OutcomeStageInputPreimage, OutcomeV2Error> {
    finish_stage(identity, run, None, Vec::new(), RunStatus::ExpectedWait)
}

fn build_settled_stage(
    identity: &SettlementIdentity,
    run: &OwnerRun,
    admitted: AdmittedOutcomeDailyBars,
) -> Result<OutcomeStageInputPreimage, OutcomeV2Error> {
    admitted.validate_strict().map_err(|error| {
        OutcomeV2Error::new(
            "admitted_outcome_evidence_invalid",
            format!(
                "strict admitted evidence validation failed reason_code={}: {error}",
                error.reason_code()
            ),
        )
    })?;
    validate_admitted_identity(identity, &admitted)?;
    let request = admitted.request_evidence().clone();
    let request_preimage = request
        .validate(Some(RequestKind::OutcomeMarketEvidence))
        .map_err(schema_error)?;
    let request_parameters: OutcomeMarketRequestParametersPreimage =
        serde_json::from_str(&request_preimage.parameters_json).map_err(|error| {
            OutcomeV2Error::new("outcome_request_parameters_invalid", error.to_string())
        })?;
    validate_request_schedule(identity, &request_parameters)?;
    let evidence = admitted.available_evidence().clone();
    evidence
        .validate_complete(&request_parameters, &request.request_hash)
        .map_err(schema_error)?;
    let provider_evidence = &evidence.provider_evidence;
    if provider_evidence.evidence_kind != ProviderEvidenceKind::OutcomeDailyBars {
        return Err(OutcomeV2Error::new(
            "outcome_evidence_kind_mismatch",
            "settlement requires outcome_daily_bars evidence",
        ));
    }

    let bars = admitted
        .bars()
        .iter()
        .map(|bar| OutcomeMathBar {
            open: bar.open(),
            high: bar.high(),
            low: bar.low(),
            close: bar.close(),
            volume: bar.volume(),
            amount: bar.amount(),
        })
        .collect::<Vec<_>>();
    let numbers = compute_outcome_numbers(
        identity.phase,
        &bars,
        identity.receipted_t0_close.as_deref(),
        identity.receipted_t0_volume.as_deref(),
    )?;
    let source = provider_evidence.source.clone().ok_or_else(|| {
        OutcomeV2Error::new(
            "outcome_complete_evidence_source_missing",
            "complete admitted evidence has no source",
        )
    })?;
    let observed_at = provider_evidence.observed_at.clone().ok_or_else(|| {
        OutcomeV2Error::new(
            "outcome_complete_evidence_observed_at_missing",
            "complete admitted evidence has no observed_at",
        )
    })?;
    let batch_id = provider_evidence.batch_id.clone().ok_or_else(|| {
        OutcomeV2Error::new(
            "outcome_complete_evidence_batch_id_missing",
            "complete admitted evidence has no batch_id",
        )
    })?;
    let batch_content_hash = provider_evidence
        .batch_content_hash
        .clone()
        .ok_or_else(|| {
            OutcomeV2Error::new(
                "outcome_complete_evidence_batch_hash_missing",
                "complete admitted evidence has no batch_content_hash",
            )
        })?;
    let outcome = SelectionSampleOutcomeRowContentPreimage {
        domain: DOMAIN_SAMPLE_OUTCOME_ROW.into(),
        sample_key: identity.sample_key.clone(),
        phase: identity.phase,
        outcome_run_id: run.stage_run_id.clone(),
        due_trading_date: identity.stored_due_date.format("%Y-%m-%d").to_string(),
        open: numbers.open,
        high: numbers.high,
        low: numbers.low,
        close: numbers.close,
        volume: numbers.volume,
        amount: numbers.amount,
        return_from_t0_close: numbers.return_from_t0_close,
        cumulative_mfe: numbers.cumulative_mfe,
        cumulative_mae: numbers.cumulative_mae,
        volume_ratio: numbers.volume_ratio,
        provider: provider_evidence.provider.clone(),
        source,
        source_at: provider_evidence.source_at.clone(),
        observed_at,
        batch_id,
        batch_content_hash,
        created_at: run.attempted_at.clone(),
    };
    let outcome_hash = sha256_json(&outcome).map_err(schema_error)?;
    let evidence_json = canonical_json(&evidence).map_err(schema_error)?;
    let evidence_hash = sha256_json(&evidence).map_err(schema_error)?;
    let transport_attempts = admitted.transport_attempts().clone();
    let transport_attempts_json = canonical_json(&transport_attempts).map_err(schema_error)?;
    let transport_attempts_hash = sha256_json(&transport_attempts).map_err(schema_error)?;
    let mut attempt = base_attempt(identity, run, OutcomeAttemptResult::Settled, Some(&request));
    attempt.transport_attempts_json = Some(transport_attempts_json);
    attempt.transport_attempts_hash = Some(transport_attempts_hash);
    project_evidence(&mut attempt, &evidence, evidence_json, evidence_hash);
    attempt.settled_outcome_content_hash = Some(outcome_hash);
    finish_stage(
        identity,
        run,
        Some(attempt),
        vec![outcome],
        RunStatus::Settled,
    )
}

fn validate_admitted_identity(
    identity: &SettlementIdentity,
    admitted: &AdmittedOutcomeDailyBars,
) -> Result<(), OutcomeV2Error> {
    if admitted.sample_key() != identity.sample_key
        || admitted.canonical_stock_code() != identity.sample_key_preimage.stock_code
        || admitted.phase() != identity.phase
        || admitted.stored_due_date() != identity.stored_due_date
        || admitted.verified_due_binding_hash() != identity.request_binding_hash
        || admitted.trading_date_vector() != &identity.trading_date_vector
        || admitted.trading_date_vector_hash() != identity.trading_date_vector_hash
    {
        return Err(OutcomeV2Error::new(
            "admitted_outcome_identity_mismatch",
            "admitted outcome capability does not equal the consumed verified due identity",
        ));
    }
    if admitted.canonical_market().trim().is_empty() {
        return Err(OutcomeV2Error::new(
            "admitted_outcome_market_missing",
            "admitted outcome capability has no canonical market",
        ));
    }
    if admitted.bars().len() != identity.applicable_trading_dates.len()
        || admitted.window_dates() != identity.applicable_trading_dates.as_slice()
    {
        return Err(OutcomeV2Error::new(
            "admitted_outcome_window_mismatch",
            "admitted bars must equal the exact T0-through-phase stored window",
        ));
    }
    Ok(())
}

fn build_error_stage(
    identity: &SettlementIdentity,
    run: &OwnerRun,
    failure: OutcomeAcquisitionFailure,
) -> Result<OutcomeStageInputPreimage, OutcomeV2Error> {
    let (request, error, available_evidence, transport_attempts) = failure.into_parts();
    let request = request.ok_or_else(|| {
        OutcomeV2Error::new(
            "outcome_pre_provider_failure",
            format!(
                "outcome acquisition failed before provider access; no request may be fabricated: \
                 reason_code={} retryable={}",
                error.reason_code(),
                error.retryable()
            ),
        )
    })?;
    let transport_attempts = transport_attempts.ok_or_else(|| {
        OutcomeV2Error::new(
            "outcome_provider_transport_attempts_missing",
            "provider failure lost its typed ordered transport-attempt evidence",
        )
    })?;
    build_error_stage_from_parts(
        identity,
        run,
        request,
        error,
        available_evidence,
        transport_attempts,
    )
}

fn build_error_stage_from_parts(
    identity: &SettlementIdentity,
    run: &OwnerRun,
    request: RequestEvidenceColumns,
    error: GatewayError,
    available_evidence: Option<OutcomeProviderAvailableEvidencePreimage>,
    transport_attempts: OutcomeTransportAttemptsPreimage,
) -> Result<OutcomeStageInputPreimage, OutcomeV2Error> {
    let request_preimage = request
        .validate(Some(RequestKind::OutcomeMarketEvidence))
        .map_err(schema_error)?;
    let request_parameters: OutcomeMarketRequestParametersPreimage =
        serde_json::from_str(&request_preimage.parameters_json).map_err(|parse_error| {
            OutcomeV2Error::new(
                "outcome_request_parameters_invalid",
                parse_error.to_string(),
            )
        })?;
    validate_request_schedule(identity, &request_parameters)?;
    let reason = map_gateway_reason(error.reason_code());
    let error_kind = match reason {
        OutcomeReasonCodeV1::EvidenceConflict => ProviderErrorKind::Integrity,
        OutcomeReasonCodeV1::ProviderUnavailable => ProviderErrorKind::Transport,
        _ => ProviderErrorKind::InvalidData,
    };
    let detail = ProviderErrorDetailPreimage {
        domain: DOMAIN_PROVIDER_ERROR_DETAIL.into(),
        error_kind,
        provider: OUTCOME_PROVIDER.into(),
        operation: OUTCOME_OPERATION.into(),
        error_code: Some(error.reason_code().into()),
        http_status: None,
        timeout_ms: None,
        invariant_id: (reason == OutcomeReasonCodeV1::EvidenceConflict)
            .then(|| "outcome-evidence-consistency-v1".into()),
        diagnostic_code: error.reason_code().into(),
    };
    detail.validate().map_err(schema_error)?;
    let detail_json = canonical_json(&detail).map_err(schema_error)?;
    let detail_hash = sha256_json(&detail).map_err(schema_error)?;
    let retryable = error.retryable();
    let mut attempt = base_attempt(identity, run, OutcomeAttemptResult::Error, Some(&request));
    let transport_attempts_json = canonical_json(&transport_attempts).map_err(schema_error)?;
    let transport_attempts_hash = sha256_json(&transport_attempts).map_err(schema_error)?;
    attempt.transport_attempts_json = Some(transport_attempts_json);
    attempt.transport_attempts_hash = Some(transport_attempts_hash.clone());
    attempt.reason_code = Some(reason);
    attempt.retryable = Some(retryable);
    if reason == OutcomeReasonCodeV1::SettledBarMissing && available_evidence.is_none() {
        return Err(OutcomeV2Error::new(
            "settled_bar_missing_evidence_required",
            "Gateway settled_bar_missing failure lost the real provider records",
        ));
    }
    let available_evidence_hash = if let Some(evidence) = available_evidence {
        evidence
            .validate_partial(&request_parameters, &request.request_hash)
            .map_err(schema_error)?;
        if evidence.provider_evidence.evidence_kind != ProviderEvidenceKind::OutcomeDailyBars {
            return Err(OutcomeV2Error::new(
                "outcome_failure_evidence_kind_mismatch",
                "outcome failure evidence must be outcome_daily_bars",
            ));
        }
        let evidence_json = canonical_json(&evidence).map_err(schema_error)?;
        let evidence_hash = sha256_json(&evidence).map_err(schema_error)?;
        project_evidence(
            &mut attempt,
            &evidence,
            evidence_json,
            evidence_hash.clone(),
        );
        Some(evidence_hash)
    } else {
        None
    };
    attempt.error_detail_json = Some(detail_json);
    attempt.error_detail_hash = Some(detail_hash.clone());
    attempt.error_fingerprint = Some(
        sha256_json(&OutcomeErrorFingerprintPreimageV2 {
            domain: DOMAIN_ERROR_FINGERPRINT.into(),
            failed_stage: OUTCOME_OPERATION.into(),
            reason_code: reason.as_str().into(),
            retryable,
            available_evidence_hash,
            detail_hash,
            transport_attempts_hash,
        })
        .map_err(schema_error)?,
    );
    finish_stage(
        identity,
        run,
        Some(attempt),
        Vec::new(),
        if retryable {
            RunStatus::FailedRetryable
        } else {
            RunStatus::FailedNonRetryable
        },
    )
}

fn map_gateway_reason(reason_code: &str) -> OutcomeReasonCodeV1 {
    match reason_code {
        "evidence_conflict" => OutcomeReasonCodeV1::EvidenceConflict,
        "evidence_stale" => OutcomeReasonCodeV1::EvidenceStale,
        "manual_confirmation_required" => OutcomeReasonCodeV1::ManualConfirmationRequired,
        "settled_bar_missing" => OutcomeReasonCodeV1::SettledBarMissing,
        "no_verified_batch" | "provider_unavailable" | "unsupported_window" => {
            OutcomeReasonCodeV1::ProviderUnavailable
        }
        // The shared owner gate is the only pre-provider ExpectedWait seam.
        // Seeing this code after provider access is therefore invalid provider
        // data, retained in ProviderErrorDetailPreimage::diagnostic_code.
        "market_session_unsettled" => OutcomeReasonCodeV1::ProviderInvalidData,
        "invalid_evidence" | "missing" => OutcomeReasonCodeV1::EvidenceIncomplete,
        _ => OutcomeReasonCodeV1::ProviderInvalidData,
    }
}

fn validate_request_schedule(
    identity: &SettlementIdentity,
    request: &OutcomeMarketRequestParametersPreimage,
) -> Result<(), OutcomeV2Error> {
    let applicable = identity
        .applicable_trading_dates
        .iter()
        .map(|date| date.format("%Y-%m-%d").to_string())
        .collect::<Vec<_>>();
    if request.calendar_version != identity.calendar_version
        || request.calendar_hash != identity.calendar_hash
        || request.trading_date_vector != identity.trading_date_vector
        || request.trading_date_vector_hash != identity.trading_date_vector_hash
        || request.applicable_trading_dates != applicable
        || request.window_start != identity.window_start.format("%Y-%m-%d").to_string()
        || request.window_end != identity.window_end.format("%Y-%m-%d").to_string()
    {
        return Err(OutcomeV2Error::new(
            "outcome_request_schedule_mismatch",
            "semantic request does not bind the consumed due full vector and phase prefix",
        ));
    }
    Ok(())
}

fn base_attempt(
    identity: &SettlementIdentity,
    run: &OwnerRun,
    result_code: OutcomeAttemptResult,
    request: Option<&RequestEvidenceColumns>,
) -> SelectionOutcomeAttemptRowContentPreimage {
    SelectionOutcomeAttemptRowContentPreimage {
        domain: DOMAIN_OUTCOME_ATTEMPT_ROW.into(),
        outcome_attempt_id: String::new(),
        sample_key: identity.sample_key.clone(),
        phase: identity.phase,
        stored_due_date: identity.stored_due_date.format("%Y-%m-%d").to_string(),
        outcome_run_id: run.stage_run_id.clone(),
        request_hash: request.map(|value| value.request_hash.clone()),
        request_evidence_json: request.map(|value| value.request_evidence_json.clone()),
        request_evidence_hash: request.map(|value| value.request_evidence_hash.clone()),
        transport_attempts_json: None,
        transport_attempts_hash: None,
        result_code,
        reason_code: None,
        retryable: None,
        provider: None,
        source: None,
        source_at: None,
        observed_at: None,
        batch_id: None,
        batch_content_hash: None,
        available_evidence_json: None,
        available_evidence_hash: None,
        error_detail_json: None,
        error_detail_hash: None,
        error_fingerprint: None,
        settled_outcome_content_hash: None,
        attempted_at: run.attempted_at.clone(),
    }
}

fn project_evidence(
    attempt: &mut SelectionOutcomeAttemptRowContentPreimage,
    evidence: &OutcomeProviderAvailableEvidencePreimage,
    evidence_json: String,
    evidence_hash: String,
) {
    let provider_evidence = &evidence.provider_evidence;
    attempt.provider = Some(provider_evidence.provider.clone());
    attempt.source = provider_evidence.source.clone();
    attempt.source_at = provider_evidence.source_at.clone();
    attempt.observed_at = provider_evidence.observed_at.clone();
    attempt.batch_id = provider_evidence.batch_id.clone();
    attempt.batch_content_hash = provider_evidence.batch_content_hash.clone();
    attempt.available_evidence_json = Some(evidence_json);
    attempt.available_evidence_hash = Some(evidence_hash);
}

fn finish_stage(
    identity: &SettlementIdentity,
    run: &OwnerRun,
    attempt: Option<SelectionOutcomeAttemptRowContentPreimage>,
    outcomes: Vec<SelectionSampleOutcomeRowContentPreimage>,
    planned_run_status: RunStatus,
) -> Result<OutcomeStageInputPreimage, OutcomeV2Error> {
    let outcome_attempt_rows = if let Some(mut attempt) = attempt {
        attempt.outcome_attempt_id =
            sha256_json(&crate::selection::schema_v2::OutcomeAttemptPreimage {
                domain: DOMAIN_OUTCOME_ATTEMPT.into(),
                stage_run_id: run.stage_run_id.clone(),
                sample_key: identity.sample_key.clone(),
                phase: identity.phase,
                stored_due_date: identity.stored_due_date.format("%Y-%m-%d").to_string(),
                request_hash: attempt.request_hash.clone(),
                transport_attempts_hash: attempt.transport_attempts_hash.clone(),
                provider_batch_id: attempt.batch_id.clone(),
                provider_observed_at: attempt.observed_at.clone(),
                result_code: attempt.result_code,
                error_fingerprint: attempt.error_fingerprint.clone(),
            })
            .map_err(schema_error)?;
        vec![attempt]
    } else {
        Vec::new()
    };
    let stage = OutcomeStageInputPreimage {
        domain: DOMAIN_OUTCOME_STAGE.into(),
        stage_run_id: run.stage_run_id.clone(),
        logical_subject_key: run.logical_subject_key.clone(),
        config_activation_run_id: identity.config_activation_run_id.clone(),
        config_hash: identity.config_hash.clone(),
        outcome_claim_id: run.outcome_claim_id.clone(),
        outcome_claim_receipt_content_hash: run.outcome_claim_receipt_content_hash.clone(),
        outcome_claim_due_binding_hash: run.outcome_claim_due_binding_hash.clone(),
        outcome_claim_provider_request_hash: run.outcome_claim_provider_request_hash.clone(),
        sample_key_preimage: identity.sample_key_preimage.clone(),
        sample_key: identity.sample_key.clone(),
        outcome_phase: identity.phase,
        stored_due_date: identity.stored_due_date.format("%Y-%m-%d").to_string(),
        outcome_attempt_rows,
        outcome_rows: outcomes,
        planned_run_status,
    };
    stage.validate().map_err(schema_error)?;
    Ok(stage)
}

#[derive(Debug, Clone, Copy)]
struct OutcomeMathBar {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    amount: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutcomeNumbers {
    open: String,
    high: String,
    low: String,
    close: String,
    volume: String,
    amount: String,
    return_from_t0_close: String,
    cumulative_mfe: String,
    cumulative_mae: String,
    volume_ratio: String,
}

fn compute_outcome_numbers(
    phase: OutcomePhase,
    bars: &[OutcomeMathBar],
    receipted_t0_close: Option<&str>,
    receipted_t0_volume: Option<&str>,
) -> Result<OutcomeNumbers, OutcomeV2Error> {
    let expected = phase_window_count(phase);
    if bars.len() != expected {
        return Err(OutcomeV2Error::new(
            "outcome_math_window_cardinality_mismatch",
            format!(
                "{} requires {expected} T0-through-phase bars",
                phase.as_str()
            ),
        ));
    }
    for bar in bars {
        validate_math_bar(*bar)?;
    }
    let phase_bar = *bars.last().expect("phase windows are never empty");
    let (return_from_t0_close, cumulative_mfe, cumulative_mae, volume_ratio) =
        if phase == OutcomePhase::T0Close {
            if receipted_t0_close.is_some() || receipted_t0_volume.is_some() {
                return Err(OutcomeV2Error::new(
                    "t0_receipted_baseline_unexpected",
                    "T0 settlement must establish rather than consume its own baseline",
                ));
            }
            ("0".into(), "0".into(), "0".into(), "1".into())
        } else {
            let t0_close = parse_positive_canonical(receipted_t0_close, "receipted_t0_close")?;
            let t0_volume = parse_positive_canonical(receipted_t0_volume, "receipted_t0_volume")?;
            let post_t0 = &bars[1..];
            let max_high = post_t0
                .iter()
                .map(|bar| bar.high)
                .reduce(f64::max)
                .expect("non-T0 windows contain D1");
            let min_low = post_t0
                .iter()
                .map(|bar| bar.low)
                .reduce(f64::min)
                .expect("non-T0 windows contain D1");
            (
                canonical_metric(phase_bar.close / t0_close - 1.0)?,
                canonical_metric(max_high / t0_close - 1.0)?,
                canonical_metric(min_low / t0_close - 1.0)?,
                canonical_metric(phase_bar.volume / t0_volume)?,
            )
        };
    Ok(OutcomeNumbers {
        open: canonical_metric(phase_bar.open)?,
        high: canonical_metric(phase_bar.high)?,
        low: canonical_metric(phase_bar.low)?,
        close: canonical_metric(phase_bar.close)?,
        volume: canonical_metric(phase_bar.volume)?,
        amount: canonical_metric(phase_bar.amount)?,
        return_from_t0_close,
        cumulative_mfe,
        cumulative_mae,
        volume_ratio,
    })
}

fn validate_math_bar(bar: OutcomeMathBar) -> Result<(), OutcomeV2Error> {
    if [bar.open, bar.high, bar.low, bar.close]
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0)
        || bar.high < bar.open.max(bar.close)
        || bar.low > bar.open.min(bar.close)
        || bar.high < bar.low
        || !bar.volume.is_finite()
        || bar.volume <= 0.0
        || !bar.amount.is_finite()
        || bar.amount < 0.0
    {
        return Err(OutcomeV2Error::new(
            "outcome_math_bar_invalid",
            "outcome OHLCV/amount failed strict finite, positive and relationship gates",
        ));
    }
    Ok(())
}

fn parse_positive_canonical(
    value: Option<&str>,
    field: &'static str,
) -> Result<f64, OutcomeV2Error> {
    let value = value.ok_or_else(|| {
        OutcomeV2Error::new(
            "outcome_t0_baseline_missing",
            format!("{field} is absent from the receipted T0 outcome"),
        )
    })?;
    let parsed = value.parse::<f64>().map_err(|_| {
        OutcomeV2Error::new(
            "outcome_t0_baseline_invalid",
            format!("{field} is not a finite canonical decimal"),
        )
    })?;
    if parsed <= 0.0 || canonical_f64(parsed).map_err(schema_error)?.as_str() != value {
        return Err(OutcomeV2Error::new(
            "outcome_t0_baseline_invalid",
            format!("{field} must be a positive canonical decimal"),
        ));
    }
    Ok(parsed)
}

fn canonical_metric(value: f64) -> Result<String, OutcomeV2Error> {
    canonical_f64(if value == 0.0 { 0.0 } else { value }).map_err(schema_error)
}

fn phase_window_count(phase: OutcomePhase) -> usize {
    match phase {
        OutcomePhase::T0Close => 1,
        OutcomePhase::D1Settled => 2,
        OutcomePhase::D3Settled => 4,
        OutcomePhase::D5Settled => 6,
    }
}

fn new_uuid_v7(
    attempted_at_rfc3339_nanos_utc: &str,
    due_binding_hash: &str,
) -> Result<String, OutcomeV2Error> {
    let parsed = DateTime::parse_from_rfc3339(attempted_at_rfc3339_nanos_utc)
        .map_err(|_| OutcomeV2Error::new("attempted_at_invalid", "owner clock is invalid"))?
        .with_timezone(&Utc);
    let timestamp_ms = u64::try_from(parsed.timestamp_millis()).map_err(|_| {
        OutcomeV2Error::new(
            "attempted_at_before_unix_epoch",
            "UUIDv7 owner clock must not precede the Unix epoch",
        )
    })?;
    if timestamp_ms >= (1_u64 << 48) {
        return Err(OutcomeV2Error::new(
            "attempted_at_uuid_v7_range",
            "UUIDv7 millisecond timestamp exceeds 48 bits",
        ));
    }
    let sequence = RUN_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let entropy = sha256_bytes(
        format!(
            "{attempted_at_rfc3339_nanos_utc}:{due_binding_hash}:{}:{sequence}",
            std::process::id()
        )
        .as_bytes(),
    );
    let timestamp = format!("{timestamp_ms:012x}");
    Ok(format!(
        "{}-{}-7{}-8{}-{}",
        &timestamp[..8],
        &timestamp[8..],
        &entropy[..3],
        &entropy[3..6],
        &entropy[6..18]
    ))
}

fn require_canonical_uuid_v7(value: &str, field: &'static str) -> Result<(), OutcomeV2Error> {
    let bytes = value.as_bytes();
    let hyphens = [8, 13, 18, 23];
    let canonical = bytes.len() == 36
        && hyphens.iter().all(|index| bytes[*index] == b'-')
        && bytes.iter().enumerate().all(|(index, byte)| {
            hyphens.contains(&index) || byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)
        })
        && bytes[14] == b'7'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b');
    if canonical {
        Ok(())
    } else {
        Err(OutcomeV2Error::new(
            "outcome_claim_uuid_v7_invalid",
            format!("{field} must be canonical lowercase UUIDv7"),
        ))
    }
}

fn require_sha256(value: &str, field: &'static str) -> Result<(), OutcomeV2Error> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(OutcomeV2Error::new(
            "invalid_sha256",
            format!("{field} must be lowercase 64-hex"),
        ))
    }
}

fn schema_error(error: crate::selection::schema_v2::SchemaV2Error) -> OutcomeV2Error {
    OutcomeV2Error::new(error.code, error.detail)
}

#[cfg(test)]
#[cfg(feature = "magic-gateway")]
mod tests {
    use super::*;
    use crate::selection::schema_v2::{
        build_request_evidence, AdjustmentKind, DailyIntervalKind,
        OutcomeMarketRequestParametersPreimage, ProviderCapabilityHashPreimage,
        RequestParametersPreimage, DOMAIN_OUTCOME_MARKET_REQUEST,
        DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE, DOMAIN_PROVIDER_AVAILABLE_EVIDENCE,
        DOMAIN_PROVIDER_CAPABILITY, DOMAIN_SAMPLE_KEY, UPSTREAM_REVISION,
    };
    use chrono::TimeZone;

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").expect("valid test date")
    }

    fn shanghai() -> FixedOffset {
        FixedOffset::east_opt(8 * 60 * 60).expect("+08:00 must be valid")
    }

    fn hash(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    fn schedule() -> StoredOutcomeSchedule {
        derive_outcome_schedule(
            date("2026-07-24"),
            &[
                date("2026-07-23"),
                date("2026-07-24"),
                date("2026-07-27"),
                date("2026-07-28"),
                date("2026-07-29"),
                date("2026-07-30"),
                date("2026-07-31"),
            ],
        )
        .expect("covered calendar")
    }

    fn sample_key_preimage() -> SampleKeyPreimage {
        SampleKeyPreimage {
            domain: DOMAIN_SAMPLE_KEY.into(),
            event_id: hash('e'),
            chain_id: "TEST_CODE_chain".into(),
            stock_code: "TEST_CODE_000001".into(),
            relation_schema_version: "relation-v1".into(),
            feature_version: "feature-v1".into(),
            evaluation_market_date: "2026-07-24".into(),
        }
    }

    fn identity(phase: OutcomePhase) -> SettlementIdentity {
        let sample_key_preimage = sample_key_preimage();
        let schedule = schedule();
        let vector = schedule.trading_date_vector();
        let (window_end, t0_close, t0_volume) = match phase {
            OutcomePhase::T0Close => (date("2026-07-24"), None, None),
            OutcomePhase::D1Settled => (date("2026-07-27"), Some("8".into()), Some("100".into())),
            OutcomePhase::D3Settled => (date("2026-07-29"), Some("8".into()), Some("100".into())),
            OutcomePhase::D5Settled => (date("2026-07-31"), Some("8".into()), Some("100".into())),
        };
        let applicable_trading_dates = match phase {
            OutcomePhase::T0Close => vec![schedule.t0_due_date],
            OutcomePhase::D1Settled => vec![schedule.t0_due_date, schedule.d1_due_date],
            OutcomePhase::D3Settled => vec![
                schedule.t0_due_date,
                schedule.d1_due_date,
                schedule.d2_due_date,
                schedule.d3_due_date,
            ],
            OutcomePhase::D5Settled => vec![
                schedule.t0_due_date,
                schedule.d1_due_date,
                schedule.d2_due_date,
                schedule.d3_due_date,
                schedule.d4_due_date,
                schedule.d5_due_date,
            ],
        };
        SettlementIdentity {
            sample_key: sha256_json(&sample_key_preimage).expect("sample key"),
            sample_key_preimage,
            config_activation_run_id: "01900000-0000-7000-8000-000000000002".into(),
            config_hash: hash('c'),
            phase,
            stored_due_date: window_end,
            window_start: date("2026-07-24"),
            window_end,
            calendar_version: "calendar-v1".into(),
            calendar_hash: hash('a'),
            trading_date_vector_hash: sha256_json(&vector).expect("vector hash"),
            trading_date_vector: vector,
            applicable_trading_dates,
            request_binding_hash: hash('b'),
            claim_due_binding_hash: hash('e'),
            provider_request_hash: hash('b'),
            receipted_t0_close: t0_close,
            receipted_t0_volume: t0_volume,
        }
    }

    fn run(identity: &SettlementIdentity) -> OwnerRun {
        let attempted_at = Utc
            .with_ymd_and_hms(2026, 7, 27, 7, 30, 0)
            .single()
            .expect("UTC test time");
        let attempted_at_text = attempted_at.to_rfc3339_opts(SecondsFormat::Nanos, true);
        let planned_outcome_run_id =
            new_uuid_v7(&attempted_at_text, &identity.request_binding_hash)
                .expect("planned outcome run id");
        let claim = ReceiptedOutcomeClaim::validated(
            "01900000-0000-7000-8000-00000000000c".into(),
            planned_outcome_run_id,
            hash('d'),
            identity.claim_due_binding_hash.clone(),
            identity.provider_request_hash.clone(),
        )
        .expect("receipted outcome claim");
        OwnerRun::new(identity, attempted_at, claim).expect("owner run")
    }

    fn request(identity: &SettlementIdentity) -> RequestEvidenceColumns {
        build_request_evidence(
            RequestParametersPreimage::OutcomeMarketEvidence(
                OutcomeMarketRequestParametersPreimage {
                    domain: DOMAIN_OUTCOME_MARKET_REQUEST.into(),
                    sample_key: identity.sample_key.clone(),
                    canonical_stock_code: identity.sample_key_preimage.stock_code.clone(),
                    canonical_market: "sz".into(),
                    phase: identity.phase,
                    stored_due_date: identity.stored_due_date.format("%Y-%m-%d").to_string(),
                    calendar_version: identity.calendar_version.clone(),
                    calendar_hash: identity.calendar_hash.clone(),
                    trading_date_vector: identity.trading_date_vector.clone(),
                    trading_date_vector_hash: identity.trading_date_vector_hash.clone(),
                    applicable_trading_dates: identity
                        .applicable_trading_dates
                        .iter()
                        .map(|date| date.format("%Y-%m-%d").to_string())
                        .collect(),
                    window_start: identity.window_start.format("%Y-%m-%d").to_string(),
                    window_end: identity.window_end.format("%Y-%m-%d").to_string(),
                    interval: DailyIntervalKind::Day,
                    adjustment: AdjustmentKind::None,
                },
            ),
            ProviderCapabilityHashPreimage {
                domain: DOMAIN_PROVIDER_CAPABILITY.into(),
                provider: OUTCOME_PROVIDER.into(),
                capability_name: "MagicTdx-UnadjustedDailyBars".into(),
                contract_version: "magic-market-core.MarketDataProvider.bars.v0.2.0".into(),
                upstream_revision: UPSTREAM_REVISION.into(),
            },
        )
        .expect("typed TEST_CODE request")
    }

    fn bar(open: f64, high: f64, low: f64, close: f64, volume: f64, amount: f64) -> OutcomeMathBar {
        OutcomeMathBar {
            open,
            high,
            low,
            close,
            volume,
            amount,
        }
    }

    #[test]
    fn derives_exact_trading_day_offsets_across_a_weekend() {
        let actual = schedule();
        assert_eq!(actual.t0_due_date, date("2026-07-24"));
        assert_eq!(actual.d1_due_date, date("2026-07-27"));
        assert_eq!(actual.d2_due_date, date("2026-07-28"));
        assert_eq!(actual.d3_due_date, date("2026-07-29"));
        assert_eq!(actual.d4_due_date, date("2026-07-30"));
        assert_eq!(actual.d5_due_date, date("2026-07-31"));
    }

    #[test]
    fn rejects_non_trading_evaluation_and_weekend_calendar_entries() {
        let missing = derive_outcome_schedule(
            date("2026-07-25"),
            &[
                date("2026-07-24"),
                date("2026-07-27"),
                date("2026-07-28"),
                date("2026-07-29"),
                date("2026-07-30"),
                date("2026-07-31"),
            ],
        )
        .expect_err("Saturday evaluation must not be inferred");
        assert_eq!(missing.code, "evaluation_date_not_trading_day");

        let weekend = derive_outcome_schedule(
            date("2026-07-24"),
            &[
                date("2026-07-24"),
                date("2026-07-25"),
                date("2026-07-27"),
                date("2026-07-28"),
                date("2026-07-29"),
                date("2026-07-30"),
            ],
        )
        .expect_err("weekend must not be accepted");
        assert_eq!(weekend.code, "trading_calendar_weekend_date");
    }

    #[test]
    fn t0_math_fixes_all_baseline_ratios() {
        let actual = compute_outcome_numbers(
            OutcomePhase::T0Close,
            &[bar(8.0, 10.0, 7.0, 9.0, 100.0, 900.0)],
            None,
            None,
        )
        .expect("T0 math");
        assert_eq!(actual.open, "8");
        assert_eq!(actual.close, "9");
        assert_eq!(actual.return_from_t0_close, "0");
        assert_eq!(actual.cumulative_mfe, "0");
        assert_eq!(actual.cumulative_mae, "0");
        assert_eq!(actual.volume_ratio, "1");
    }

    #[test]
    fn d1_math_uses_receipted_t0_and_excludes_t0_high_low() {
        let actual = compute_outcome_numbers(
            OutcomePhase::D1Settled,
            &[
                bar(8.0, 40.0, 1.0, 8.0, 100.0, 800.0),
                bar(9.0, 12.0, 6.0, 10.0, 200.0, 2_000.0),
            ],
            Some("8"),
            Some("100"),
        )
        .expect("D1 math");
        assert_eq!(actual.return_from_t0_close, "0.25");
        assert_eq!(actual.cumulative_mfe, "0.5");
        assert_eq!(actual.cumulative_mae, "-0.25");
        assert_eq!(actual.volume_ratio, "2");
    }

    #[test]
    fn expected_wait_has_no_request_provider_or_error_and_no_provider_call_shape() {
        let identity = identity(OutcomePhase::T0Close);
        let run = run(&identity);
        let stage = build_expected_wait_stage(&identity, &run).expect("wait stage");
        assert_eq!(stage.planned_run_status, RunStatus::ExpectedWait);
        assert!(
            stage.outcome_attempt_rows.is_empty(),
            "pre-provider ExpectedWait must not fabricate a provider attempt"
        );
        assert!(stage.outcome_rows.is_empty());
        assert_eq!(stage.expected_staged_row_count(), 1);

        let before_close = shanghai()
            .with_ymd_and_hms(2026, 7, 24, 14, 59, 59)
            .single()
            .expect("local test time");
        assert_eq!(
            outcome_market_session_status(date("2026-07-24"), before_close),
            Ok(OutcomeMarketSessionStatus::Incomplete)
        );
        let exact_close = shanghai()
            .with_ymd_and_hms(2026, 7, 24, 15, 0, 0)
            .single()
            .expect("local test time");
        assert_eq!(
            outcome_market_session_status(date("2026-07-24"), exact_close),
            Ok(OutcomeMarketSessionStatus::Incomplete)
        );
    }

    #[test]
    fn gateway_failure_is_a_typed_error_without_fabricated_evidence() {
        let identity = identity(OutcomePhase::D1Settled);
        let run = run(&identity);
        let failure = OutcomeAcquisitionFailure::test_only_after_provider(
            request(&identity),
            "provider_unavailable",
            true,
            None,
        );
        let stage = build_error_stage(&identity, &run, failure).expect("typed error stage");
        let attempt = &stage.outcome_attempt_rows[0];
        assert_eq!(stage.planned_run_status, RunStatus::FailedRetryable);
        assert_eq!(attempt.result_code, OutcomeAttemptResult::Error);
        assert_eq!(
            attempt.reason_code,
            Some(OutcomeReasonCodeV1::ProviderUnavailable)
        );
        assert_eq!(attempt.retryable, Some(true));
        assert!(attempt.request_evidence_json.is_some());
        assert!(attempt.available_evidence_json.is_none());
        assert!(attempt.error_detail_json.is_some());
        assert!(attempt.error_fingerprint.is_some());
        assert!(stage.outcome_rows.is_empty());
    }

    #[test]
    fn settled_bar_missing_failure_persists_real_partial_evidence_and_exact_reason() {
        let identity = identity(OutcomePhase::D1Settled);
        let run = run(&identity);
        let provider_evidence = crate::selection::schema_v2::ProviderAvailableEvidencePreimage {
            domain: DOMAIN_PROVIDER_AVAILABLE_EVIDENCE.into(),
            evidence_kind: ProviderEvidenceKind::OutcomeDailyBars,
            provider: OUTCOME_PROVIDER.into(),
            source: Some("tdx-smart".into()),
            source_at: Some("2026-07-24".into()),
            observed_at: Some("2026-07-27T15:01:00+08:00".into()),
            batch_id: Some("TEST_CODE_PARTIAL_BATCH".into()),
            batch_content_hash: Some(hash('a')),
        };
        let request = request(&identity);
        let evidence = OutcomeProviderAvailableEvidencePreimage {
            domain: DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE.into(),
            request_hash: request.request_hash.clone(),
            calendar_hash: identity.calendar_hash.clone(),
            trading_date_vector_hash: identity.trading_date_vector_hash.clone(),
            expected_trading_dates: identity
                .applicable_trading_dates
                .iter()
                .map(|date| date.format("%Y-%m-%d").to_string())
                .collect(),
            returned_trading_dates: vec!["2026-07-24".into()],
            provider_evidence,
        };
        let failure = OutcomeAcquisitionFailure::test_only_after_provider(
            request,
            "settled_bar_missing",
            true,
            Some(evidence.clone()),
        );
        let stage =
            build_error_stage(&identity, &run, failure).expect("typed partial failure stage");
        let attempt = &stage.outcome_attempt_rows[0];
        assert_eq!(
            attempt.reason_code,
            Some(OutcomeReasonCodeV1::SettledBarMissing)
        );
        assert_eq!(attempt.result_code, OutcomeAttemptResult::Error);
        assert_eq!(attempt.retryable, Some(true));
        assert_eq!(attempt.provider.as_deref(), Some(OUTCOME_PROVIDER));
        assert_eq!(attempt.source, evidence.provider_evidence.source);
        assert_eq!(attempt.source_at, evidence.provider_evidence.source_at);
        assert_eq!(attempt.observed_at, evidence.provider_evidence.observed_at);
        assert_eq!(attempt.batch_id, evidence.provider_evidence.batch_id);
        assert_eq!(
            attempt.batch_content_hash,
            evidence.provider_evidence.batch_content_hash
        );
        assert!(attempt.available_evidence_json.is_some());
        assert!(attempt.available_evidence_hash.is_some());
        assert!(attempt.error_fingerprint.is_some());
    }

    #[test]
    fn provider_called_market_session_unsettled_is_error_not_expected_wait() {
        let identity = identity(OutcomePhase::D1Settled);
        let run = run(&identity);
        let provider_evidence = crate::selection::schema_v2::ProviderAvailableEvidencePreimage {
            domain: DOMAIN_PROVIDER_AVAILABLE_EVIDENCE.into(),
            evidence_kind: ProviderEvidenceKind::OutcomeDailyBars,
            provider: OUTCOME_PROVIDER.into(),
            source: Some("tdx-smart".into()),
            source_at: Some("2026-07-27".into()),
            observed_at: Some("2026-07-27T14:59:59+08:00".into()),
            batch_id: Some("TEST_CODE_INTRADAY_BATCH".into()),
            batch_content_hash: Some(hash('d')),
        };
        let request = request(&identity);
        let evidence = OutcomeProviderAvailableEvidencePreimage {
            domain: DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE.into(),
            request_hash: request.request_hash.clone(),
            calendar_hash: identity.calendar_hash.clone(),
            trading_date_vector_hash: identity.trading_date_vector_hash.clone(),
            expected_trading_dates: identity
                .applicable_trading_dates
                .iter()
                .map(|date| date.format("%Y-%m-%d").to_string())
                .collect(),
            returned_trading_dates: vec!["2026-07-24".into(), "2026-07-27".into()],
            provider_evidence,
        };
        let failure = OutcomeAcquisitionFailure::test_only_after_provider(
            request,
            "market_session_unsettled",
            true,
            Some(evidence),
        );
        let stage = build_error_stage(&identity, &run, failure)
            .expect("post-provider session state is an error");
        assert_eq!(stage.planned_run_status, RunStatus::FailedRetryable);
        assert_eq!(
            stage.outcome_attempt_rows[0].result_code,
            OutcomeAttemptResult::Error
        );
        assert_eq!(
            stage.outcome_attempt_rows[0].reason_code,
            Some(OutcomeReasonCodeV1::ProviderInvalidData)
        );
        assert!(stage.outcome_attempt_rows[0]
            .request_evidence_json
            .is_some());
        assert!(stage.outcome_attempt_rows[0]
            .available_evidence_json
            .is_some());
    }

    #[test]
    fn fixed_gateway_reason_mapping_preserves_typed_reasons() {
        assert_eq!(
            map_gateway_reason("settled_bar_missing"),
            OutcomeReasonCodeV1::SettledBarMissing
        );
        assert_eq!(
            map_gateway_reason("evidence_conflict"),
            OutcomeReasonCodeV1::EvidenceConflict
        );
    }

    #[test]
    fn public_surface_has_no_caller_forgeable_outcome_stage_inputs() {
        let source = include_str!("outcome_v2.rs");
        for forbidden in [
            ["pub ", "struct OutcomeRunContext"].concat(),
            ["pub ", "enum CompletedSessionTerminal"].concat(),
            ["pub ", "enum OutcomeEvidenceFailure"].concat(),
            ["pub ", "fn prepare_outcome_attempt"].concat(),
            ["pub ", "fn earliest_due_work"].concat(),
            ["pub ", "struct OutcomeDueWork"].concat(),
            ["pub ", "struct OutcomeReceiptKey"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "legacy forgeable API remains: {forbidden}"
            );
        }
        assert!(source.contains("due: VerifiedOutcomeDue"));
        assert!(source.contains("Ok(admitted) => build_settled_stage"));
        assert!(source.contains("Result<OutcomeSettlementOwnerResult, OutcomeV2Error>"));
        assert!(source.contains("SelectionV2PersistenceOwner::commit_outcome_claim"));
        assert!(source.contains("SelectionV2PersistenceOwner::commit_outcome(prepared_outcome)"));
        let public_accessor = ["pub ", "fn into_stage_input"].concat();
        assert!(!source.contains(&public_accessor));
        let public_preparation = ["pub ", "async fn prepare_receipted_outcome"].concat();
        assert!(!source.contains(&public_preparation));
    }

    #[test]
    fn owner_generated_run_identity_is_uuid_v7_and_unique() {
        let identity = identity(OutcomePhase::T0Close);
        let first = run(&identity);
        let second = run(&identity);
        assert_ne!(first.stage_run_id, second.stage_run_id);
        for run_id in [first.stage_run_id, second.stage_run_id] {
            assert_eq!(&run_id[14..15], "7");
            assert!(matches!(&run_id[19..20], "8" | "9" | "a" | "b"));
        }
    }

    #[test]
    fn owner_rejects_a_receipted_claim_from_another_due_or_request() {
        let identity = identity(OutcomePhase::T0Close);
        let attempted_at = Utc
            .with_ymd_and_hms(2026, 7, 27, 7, 30, 0)
            .single()
            .expect("UTC test time");
        let attempted_at_text = attempted_at.to_rfc3339_opts(SecondsFormat::Nanos, true);
        let planned_outcome_run_id =
            new_uuid_v7(&attempted_at_text, &identity.request_binding_hash)
                .expect("planned outcome run id");

        let wrong_due = ReceiptedOutcomeClaim::validated(
            "01900000-0000-7000-8000-00000000000c".into(),
            planned_outcome_run_id.clone(),
            hash('d'),
            hash('f'),
            identity.provider_request_hash.clone(),
        )
        .expect("structurally valid wrong-due claim");
        assert_eq!(
            OwnerRun::new(&identity, attempted_at, wrong_due)
                .expect_err("claim from another due must fail")
                .code,
            "outcome_claim_due_binding_mismatch"
        );

        let wrong_request = ReceiptedOutcomeClaim::validated(
            "01900000-0000-7000-8000-00000000000c".into(),
            planned_outcome_run_id,
            hash('d'),
            identity.claim_due_binding_hash.clone(),
            hash('f'),
        )
        .expect("structurally valid wrong-request claim");
        assert_eq!(
            OwnerRun::new(&identity, attempted_at, wrong_request)
                .expect_err("claim from another request must fail")
                .code,
            "outcome_claim_provider_request_mismatch"
        );
    }

    #[test]
    fn outcome_subject_lock_is_nonblocking_retained_and_reacquirable() {
        let root = fs::canonicalize(std::env::temp_dir())
            .expect("canonical TEST_CODE temp root")
            .join(format!(
                "TEST_CODE-outcome-claim-lock-{}-{}",
                std::process::id(),
                RUN_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        let key = hash('a');
        let first = match OutcomeSubjectLockGuard::try_acquire_for_test(&root, &key)
            .expect("first lock acquisition")
        {
            OutcomeSubjectLockAttempt::Acquired(guard) => guard,
            OutcomeSubjectLockAttempt::LiveOwned => panic!("fresh test lock cannot be busy"),
        };
        let lock_path = root
            .join("test/selection-outcome-claims")
            .join(format!("{key}.lock"));
        assert!(lock_path.is_file());

        let contender_root = root.clone();
        let contender_key = key.clone();
        let contender = std::thread::spawn(move || {
            matches!(
                OutcomeSubjectLockGuard::try_acquire_for_test(&contender_root, &contender_key)
                    .expect("nonblocking contender"),
                OutcomeSubjectLockAttempt::LiveOwned
            )
        })
        .join()
        .expect("contender thread");
        assert!(contender, "second owner must observe live ownership");

        drop(first);
        assert!(
            lock_path.is_file(),
            "lock file is retained permanently after OS unlock"
        );
        let reacquired = OutcomeSubjectLockGuard::try_acquire_for_test(&root, &key)
            .expect("reacquire retained lock");
        assert!(matches!(reacquired, OutcomeSubjectLockAttempt::Acquired(_)));
    }

    #[cfg(unix)]
    #[test]
    fn outcome_subject_lock_rejects_a_symlink_leaf() {
        use std::os::unix::fs::symlink;

        let root = fs::canonicalize(std::env::temp_dir())
            .expect("canonical TEST_CODE temp root")
            .join(format!(
                "TEST_CODE-outcome-claim-lock-symlink-{}-{}",
                std::process::id(),
                RUN_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        let lock_root = root.join("test/selection-outcome-claims");
        fs::create_dir_all(&lock_root).expect("create isolated lock root");
        let key = hash('b');
        let target = root.join("regular-target");
        fs::write(&target, b"not an owned claim lock").expect("write symlink target");
        symlink(&target, lock_root.join(format!("{key}.lock"))).expect("create lock symlink");

        let error = match OutcomeSubjectLockGuard::try_acquire_for_test(&root, &key) {
            Ok(_) => panic!("claim lock symlink must fail closed"),
            Err(error) => error,
        };
        assert_eq!(error.code, "outcome_subject_lock_object_invalid");
    }

    #[cfg(unix)]
    #[test]
    fn outcome_subject_lock_rejects_a_symlink_in_an_intermediate_directory() {
        use std::os::unix::fs::symlink;

        let root = fs::canonicalize(std::env::temp_dir())
            .expect("canonical TEST_CODE temp root")
            .join(format!(
                "TEST_CODE-outcome-claim-lock-intermediate-{}-{}",
                std::process::id(),
                RUN_ID_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).expect("create isolated parent");
        let redirected = root.join("redirected");
        fs::create_dir(&redirected).expect("create redirect target");
        symlink(&redirected, root.join("test")).expect("create intermediate symlink");

        let error = match OutcomeSubjectLockGuard::try_acquire_for_test(&root, &hash('c')) {
            Ok(_) => panic!("intermediate claim-lock symlink must fail closed"),
            Err(error) => error,
        };
        assert_eq!(error.code, "outcome_subject_lock_path_unsafe");
    }

    #[cfg(unix)]
    #[test]
    fn outcome_subject_lock_traverses_every_directory_by_retained_descriptor() {
        let source = include_str!("outcome_v2.rs");
        let start = source
            .find("fn try_acquire_at(")
            .expect("subject-lock acquisition");
        let end = source[start..]
            .find("fn prepare_new_claim(")
            .map(|offset| start + offset)
            .expect("end of subject-lock acquisition");
        let acquisition = &source[start..end];

        assert!(acquisition.contains("open_or_create_pinned_lock_directory(root)"));
        assert!(acquisition.contains("openat_lock_component("));
        assert!(
            !acquisition.contains("fs::create_dir_all"),
            "path-based recursive creation leaves intermediate components raceable"
        );
        assert!(
            !acquisition.contains("options.open(&path)"),
            "the lock leaf must be opened relative to the retained parent descriptor"
        );
    }

    #[test]
    fn outcome_subject_lock_guard_remains_send_for_async_owner() {
        fn assert_send<T: Send>() {}
        assert_send::<OutcomeSubjectLockGuard>();
    }

    #[test]
    fn settlement_skip_observation_retains_exact_subject_snapshot_and_reason() {
        let observation = OutcomeSettlementObservation::live_owned(hash('a'), hash('b'));
        assert_eq!(
            observation.disposition,
            OutcomeSettlementDisposition::LiveOwnedSkip
        );
        assert_eq!(observation.logical_subject_key, hash('a'));
        assert_eq!(observation.verified_due_snapshot_hash, hash('b'));
        assert_eq!(observation.reason_code, "subject_lock_live_owned");
    }

    #[test]
    fn live_owned_recovery_is_excluded_only_from_the_blocking_recheck() {
        let source = include_str!("outcome_v2.rs");
        let tick = source
            .split("pub async fn settle_tick(")
            .nth(1)
            .and_then(|tail| tail.split("async fn prepare_receipted_outcome(").next())
            .expect("settlement tick coordinator");
        assert!(tick.contains("live_owned_recovery_subjects"));
        assert!(tick.contains(
            ".filter(|item| !live_owned_recovery_subjects.contains(item.logical_subject_key()))"
        ));
        assert!(
            tick.contains("model.due_v2_outcomes(tick_at, limit)"),
            "other due subjects must still be evaluated after a live-owned skip"
        );
    }

    #[test]
    fn claim_owner_surface_has_closed_skip_algebra_and_no_lease_or_steal() {
        let source = include_str!("outcome_v2.rs");
        for required in [
            "pub enum OutcomeSettlementOwnerResult",
            "Receipted(crate::database::selection_v2_repository::CommitReceipt)",
            "LiveOwnedSkip",
            "Superseded",
            "FileExt::try_lock_exclusive",
            "openat_lock_component",
            "O_NOFOLLOW_FLAG",
            "OUTCOME_CLAIM_LOCK_RELATIVE_ROOT",
            "prepare_recovery",
            "revalidate_outcome_due_for_claim",
            "SelectionV2PersistenceOwner::commit_outcome_claim",
            "SelectionV2PersistenceOwner::commit_outcome(prepared_outcome)",
        ] {
            assert!(
                source.contains(required),
                "missing claim-owner seam: {required}"
            );
        }
        for forbidden in [
            ["remove_", "file("].concat(),
            ["heart", "beat"].concat(),
            ["lease_", "expires"].concat(),
            ["steal_", "lock"].concat(),
            ["thread_", "local!"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "claim ownership must not use {forbidden}"
            );
        }
    }

    #[test]
    fn production_owner_orders_lock_claim_provider_and_outcome_receipt() {
        let source = include_str!("outcome_v2.rs");
        let owner_start = source
            .find("async fn settle_due")
            .expect("production owner entrypoint");
        let owner_end = source[owner_start..]
            .find("\nfn prepared_stage")
            .map(|offset| owner_start + offset)
            .expect("end of owner implementation");
        let owner = &source[owner_start..owner_end];
        let ordered = [
            "try_acquire_production",
            "revalidate_outcome_due_for_claim",
            "commit_outcome_claim",
            ".prepare_receipted_outcome",
            "commit_outcome(prepared_outcome)",
        ]
        .map(|needle| {
            owner
                .find(needle)
                .unwrap_or_else(|| panic!("owner is missing {needle}"))
        });
        assert!(
            ordered.windows(2).all(|pair| pair[0] < pair[1]),
            "owner must retain subject-lock order across claim, provider and outcome receipt"
        );
        let preparation_start = owner
            .find("async fn prepare_receipted_outcome")
            .expect("private claimed-outcome preparation");
        let preparation = &owner[preparation_start..];
        let session_gate = preparation
            .find("outcome_market_session_status")
            .expect("session gate after claim");
        let provider = preparation
            .find("gateway.acquire")
            .expect("Magic TDX provider after claim");
        assert!(
            session_gate < provider,
            "market-session gate must precede provider I/O"
        );
    }

    #[test]
    fn non_outcome_recovery_is_drained_before_claim_lifecycle_and_provider_work() {
        let source = include_str!("outcome_v2.rs");
        let tick = source
            .split("pub async fn settle_tick(")
            .nth(1)
            .and_then(|tail| tail.split("async fn prepare_receipted_outcome(").next())
            .expect("settlement tick coordinator");
        let non_outcome_read = tick
            .find("into_ordered_non_outcome")
            .expect("typed non-outcome recovery inventory");
        let non_outcome_commit = tick
            .find("SelectionV2PersistenceOwner::recover_non_outcome")
            .expect("typed non-outcome recovery owner");
        let outcome_recovery = tick
            .find("model.outcome_settlement_recovery()")
            .expect("outcome lifecycle recovery");
        let due = tick
            .find("model.due_v2_outcomes(tick_at, limit)")
            .expect("new due work");
        assert!(non_outcome_read < non_outcome_commit);
        assert!(non_outcome_commit < outcome_recovery);
        assert!(outcome_recovery < due);
    }

    #[test]
    fn persisted_outcome_recovery_has_no_provider_refetch_path() {
        let source = include_str!("outcome_v2.rs");
        let branch_start = source
            .find("VerifiedOutcomeSettlementRecovery::OutcomeRecovery {")
            .expect("exact persisted-outcome recovery branch");
        let branch_end = source[branch_start..]
            .find("/// Production scheduler coordinator")
            .map(|offset| branch_start + offset)
            .expect("end of exact persisted-outcome recovery branch");
        let branch = &source[branch_start..branch_end];
        assert!(branch.contains("SelectionV2PersistenceOwner::commit_outcome(outcome)"));
        assert!(!branch.contains("gateway.acquire"));
        assert!(!branch.contains("prepare_receipted_outcome"));
    }

    #[test]
    fn claim_persists_the_exact_provider_request_before_any_provider_or_receipt_work() {
        let source = include_str!("outcome_v2.rs");
        let owner_start = source
            .find("async fn settle_due(")
            .expect("new-due settlement owner");
        let owner_end = source[owner_start..]
            .find("async fn recover_verified(")
            .map(|offset| owner_start + offset)
            .expect("end of new-due owner");
        let owner = &source[owner_start..owner_end];
        let bind = owner
            .find(".bind_claim_request(")
            .expect("pure claim-time provider request binding");
        let claim = owner
            .find("commit_outcome_claim")
            .expect("durable outcome claim");
        let provider = owner
            .find("prepare_receipted_outcome")
            .expect("provider/outcome preparation");
        assert!(
            bind < claim && claim < provider,
            "the exact provider request must be claim-bound before persistence and I/O"
        );

        let gateway = include_str!("../data_gateway/outcome_daily_bars.rs");
        assert!(
            gateway.contains("OutcomeAcquisitionPlan::from_claim_bound_due(due, attempted_at)"),
            "provider acquisition must reconstruct only from the durable claim binding"
        );
        let reconstruction = gateway
            .split("fn from_claim_bound_due(")
            .nth(1)
            .and_then(|tail| tail.split("fn from_unbound_due(").next())
            .expect("claim-bound acquisition-plan reconstruction");
        assert!(
            !reconstruction.contains("request_local_date = attempted_at.date_naive()"),
            "recovery must not mutate the persisted request date using a later tick"
        );
        assert!(
            !reconstruction.contains("natural_day_upper_bound"),
            "recovery must not recalculate the persisted latest-N transport bound"
        );
    }

    #[test]
    fn settlement_owner_threads_one_fixed_tick_instant_without_recapturing_wall_clock() {
        let source = include_str!("outcome_v2.rs");
        let owner_start = source
            .find("impl OutcomeSettlementOwner")
            .expect("settlement owner implementation");
        let owner_end = source[owner_start..]
            .find("struct SettlementIdentity")
            .map(|offset| owner_start + offset)
            .expect("end of settlement owner implementation");
        let owner = &source[owner_start..owner_end];

        assert!(owner.contains("tick_at: DateTime<FixedOffset>"));
        assert!(owner.contains("validate_shanghai_tick_instant(&tick_at)"));
        assert!(owner.contains("due_v2_outcomes(tick_at, limit)"));
        assert!(owner.contains("prepare_new_claim(&claim_bound_due, &tick_at)"));
        assert!(owner.contains("gateway.acquire(&due, tick_at)"));
        assert!(!owner.contains("Local::now()"));
        assert!(!owner.contains("Utc::now()"));
    }
}
