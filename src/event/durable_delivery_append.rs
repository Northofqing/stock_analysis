//! BR-192 exact-byte append adapter for the durable-delivery coordinator.
//!
//! The coordinator freezes canonical payload bytes before state progression.
//! This adapter appends those exact bytes to an independently locked,
//! hash-chained JSONL file and returns only after `fsync`.  Replaying the same
//! identity and bytes is idempotent; reusing an identity with different bytes
//! is a hard conflict.
//!
//! Supported targets are the Unix families listed below. Physical isolation
//! assumes all same-UID processes are trusted and use this adapter: Unix does
//! not provide a portable way to prevent a hostile same-UID process from
//! creating a hard link in the instant between two metadata checks. Pinned
//! descriptors, owner-only writable directories, the base-directory lock and
//! pre/post-write identity checks prevent conforming writers from forking.

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
)))]
compile_error!(
    "BR-192 durable immutable audit requires openat/mkdirat/flock Unix semantics; \
     supported targets: Linux, macOS/iOS, FreeBSD, OpenBSD and NetBSD"
);

use crate::durable_delivery::{DurableDeliveryError, ImmutableAppendPort, Result as DurableResult};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::ffi::{CString, OsStr, OsString};
#[cfg(test)]
use std::fs;
use std::fs::{File, OpenOptions};
#[cfg(test)]
use std::io::Read;
use std::io::{self, BufRead, BufReader, ErrorKind, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
#[cfg(test)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const HASH_DOMAIN: &str = "stock_analysis.durable_delivery_immutable_append.v1";
const RECORD_FILE: &str = "durable_delivery_v1.jsonl";
const LOCK_FILE: &str = ".durable_delivery_v1.lock";
const PRODUCTION_BASE_DIR: &str = "data/durable_delivery_audit";
const O_RDONLY_FLAG: i32 = 0;
const O_RDWR_FLAG: i32 = 2;
const GROUP_OR_WORLD_WRITE_BITS: u32 = 0o022;

#[cfg(test)]
static PARENT_DIRECTORY_SYNC_COUNT: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

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
const O_APPEND_FLAG: i32 = 0x0000_0400;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
const O_APPEND_FLAG: i32 = 0x0000_0008;
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

unsafe extern "C" {
    fn openat(directory_fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn mkdirat(directory_fd: i32, path: *const std::ffi::c_char, mode: u32) -> i32;
    fn geteuid() -> u32;
}

#[derive(Clone, Debug)]
pub struct DurableDeliveryImmutableAppend {
    base_dir: PathBuf,
    base: Arc<File>,
    namespace_binding: Option<Arc<PinnedDirectoryBinding>>,
    base_identity: FileIdentity,
    lock: Arc<File>,
    lock_identity: FileIdentity,
    #[cfg(test)]
    hooks: Arc<TestHooks>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    is_directory: bool,
    is_file: bool,
}

#[derive(Debug)]
struct PinnedDirectory {
    file: File,
    binding: PinnedDirectoryBinding,
}

#[derive(Debug)]
struct PinnedDirectoryBinding {
    anchor: File,
    anchor_identity: FileIdentity,
    components: Vec<OsString>,
    identities: Vec<FileIdentity>,
    directories: Vec<File>,
    link_count_baselines: Mutex<Vec<u64>>,
}

#[derive(Debug)]
struct VerifiedChain {
    tail_hash: Option<String>,
    matching_record: Option<StoredAppendRecord>,
}

#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    after_base_lock: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    before_common_epilogue: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

#[cfg(test)]
impl std::fmt::Debug for TestHooks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("TestHooks").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct StoredAppendRecord {
    hash_domain: String,
    record_kind: String,
    identity: String,
    canonical_hex: String,
    canonical_sha256: String,
    previous_hash: Option<String>,
    record_hash: String,
}

#[derive(Serialize)]
struct RecordHashMaterial<'a> {
    hash_domain: &'static str,
    record_kind: &'a str,
    identity: &'a str,
    canonical_hex: &'a str,
    canonical_sha256: &'a str,
    previous_hash: Option<&'a str>,
}

impl DurableDeliveryImmutableAppend {
    #[cfg(test)]
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        fs::create_dir_all(&base_dir).expect("create arbitrary test immutable-audit directory");
        let base = OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG)
            .open(&base_dir)
            .expect("pin arbitrary test immutable-audit directory");
        let base_identity = directory_identity(&base, &base_dir)
            .expect("validate arbitrary test immutable-audit directory");
        Self::from_pinned_base(base_dir, base, None, base_identity)
            .expect("pin arbitrary test immutable-audit lock")
    }

    pub fn for_production() -> DurableResult<Self> {
        Self::bind_fixed(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            Path::new(PRODUCTION_BASE_DIR),
        )
    }

    pub fn for_test_code(test_code: &str) -> DurableResult<Self> {
        validate_test_code(test_code)?;
        Self::bind_fixed(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            &PathBuf::from("data/test")
                .join(test_code)
                .join("durable_delivery_audit"),
        )
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    fn bind_fixed(anchor_dir: &Path, relative_base_dir: &Path) -> DurableResult<Self> {
        let base_dir = anchor_dir.join(relative_base_dir);
        let pinned = open_or_create_absolute_directory_no_follow(&base_dir, anchor_dir)?;
        let base_identity = directory_identity(&pinned.file, &base_dir)?;
        revalidate_directory_binding(&pinned.binding, &base_dir)?;
        let append =
            Self::from_pinned_base(base_dir, pinned.file, Some(pinned.binding), base_identity)?;
        append.validate_base_binding()?;
        Ok(append)
    }

    #[cfg(test)]
    fn bind_test_anchor(anchor_dir: &Path, relative_base_dir: &Path) -> DurableResult<Self> {
        if !anchor_dir.is_absolute() {
            return Err(isolation("test immutable-audit anchor must be absolute"));
        }
        Self::bind_fixed(anchor_dir, relative_base_dir)
    }

    fn from_pinned_base(
        base_dir: PathBuf,
        base: File,
        namespace_binding: Option<PinnedDirectoryBinding>,
        base_identity: FileIdentity,
    ) -> DurableResult<Self> {
        base.lock_exclusive()?;
        let lock_path = base_dir.join(LOCK_FILE);
        let binding = (|| {
            let lock = open_unique_regular_at(
                &base,
                OsStr::new(LOCK_FILE),
                O_RDWR_FLAG | O_CREAT_FLAG,
                &lock_path,
            )?;
            let lock_identity = regular_identity(&lock, &lock_path)?;
            base.sync_all()?;
            revalidate_regular_at(
                &base,
                OsStr::new(LOCK_FILE),
                O_RDWR_FLAG,
                &lock_path,
                lock_identity,
            )?;
            Ok((lock, lock_identity))
        })();
        let unlock = FileExt::unlock(&base);
        let (lock, lock_identity) = match (binding, unlock) {
            (Ok(value), Ok(())) => value,
            (Err(error), Ok(())) => return Err(error),
            (_, Err(error)) => return Err(error.into()),
        };

        Ok(Self {
            base_dir,
            base: Arc::new(base),
            namespace_binding: namespace_binding.map(Arc::new),
            base_identity,
            lock: Arc::new(lock),
            lock_identity,
            #[cfg(test)]
            hooks: Arc::new(TestHooks::default()),
        })
    }

    fn validate_base_binding(&self) -> DurableResult<()> {
        let Some(binding) = &self.namespace_binding else {
            return Ok(());
        };
        let rebound = revalidate_directory_binding(binding, &self.base_dir)?;
        let rebound_identity = directory_identity(&rebound, &self.base_dir)?;
        if rebound_identity != self.base_identity {
            return Err(isolation(format!(
                "immutable-audit base directory identity changed: {}",
                self.base_dir.display()
            )));
        }
        let pinned_identity = directory_identity(&self.base, &self.base_dir)?;
        if pinned_identity != self.base_identity {
            return Err(isolation(format!(
                "immutable-audit pinned base directory identity changed: {}",
                self.base_dir.display()
            )));
        }
        Ok(())
    }

    fn append_exact_inner(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        canonical_sha256: &str,
    ) -> DurableResult<String> {
        validate_input(record_kind, identity, canonical_bytes, canonical_sha256)?;
        // The process mutex must precede `flock` because cloned adapters share
        // one open file description: concurrent flock/unlock calls on that
        // descriptor are not reference-counted. This mutex keeps exactly one
        // in-process owner while the base -> child lock order protects all
        // conforming cross-process writers.
        let _process_guard = process_append_mutex()
            .lock()
            .map_err(|_| isolation("immutable-audit process mutex is poisoned"))?;
        self.base.lock_exclusive()?;
        #[cfg(test)]
        self.run_after_base_lock_hook();
        let outcome = (|| {
            self.validate_base_binding()?;
            revalidate_regular_at(
                &self.base,
                OsStr::new(LOCK_FILE),
                O_RDWR_FLAG,
                &self.base_dir.join(LOCK_FILE),
                self.lock_identity,
            )?;
            self.lock.lock_exclusive()?;
            let child_outcome =
                self.append_while_locked(record_kind, identity, canonical_bytes, canonical_sha256);
            let child_unlock = FileExt::unlock(&*self.lock);
            match (child_outcome, child_unlock) {
                (Ok(value), Ok(())) => Ok(value),
                (Err(error), Ok(())) => Err(error),
                (_, Err(error)) => Err(error.into()),
            }
        })();
        let base_unlock = FileExt::unlock(&*self.base);
        match (outcome, base_unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (_, Err(error)) => Err(error.into()),
        }
    }

    fn append_while_locked(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        canonical_sha256: &str,
    ) -> DurableResult<String> {
        let record_path = self.base_dir.join(RECORD_FILE);
        let mut record_file = open_unique_regular_at(
            &self.base,
            OsStr::new(RECORD_FILE),
            O_RDWR_FLAG | O_APPEND_FLAG | O_CREAT_FLAG,
            &record_path,
        )?;
        let record_identity = regular_identity(&record_file, &record_path)?;
        revalidate_regular_at(
            &self.base,
            OsStr::new(RECORD_FILE),
            O_RDWR_FLAG | O_APPEND_FLAG,
            &record_path,
            record_identity,
        )?;
        let verified = verify_complete_chain(&mut record_file, &record_path, identity)?;
        let existing = verified.matching_record;
        let previous_hash = verified.tail_hash;
        let canonical_hex = hex::encode(canonical_bytes);

        let (disposition, wrote_record) = match existing {
            Some(existing) => {
                if existing.record_kind == record_kind
                    && existing.canonical_sha256 == canonical_sha256
                    && existing.canonical_hex == canonical_hex
                {
                    (Ok(audit_ref(&existing.record_hash)), false)
                } else {
                    (
                        Err(DurableDeliveryError::ImmutableAppendConflict(
                            identity.to_owned(),
                        )),
                        false,
                    )
                }
            }
            None => {
                let record_hash = compute_record_hash(
                    record_kind,
                    identity,
                    &canonical_hex,
                    canonical_sha256,
                    previous_hash.as_deref(),
                )?;
                let record = StoredAppendRecord {
                    hash_domain: HASH_DOMAIN.to_owned(),
                    record_kind: record_kind.to_owned(),
                    identity: identity.to_owned(),
                    canonical_hex: canonical_hex.clone(),
                    canonical_sha256: canonical_sha256.to_owned(),
                    previous_hash,
                    record_hash,
                };
                let mut encoded = serde_json::to_vec(&record)?;
                encoded.push(b'\n');
                revalidate_regular_at(
                    &self.base,
                    OsStr::new(LOCK_FILE),
                    O_RDWR_FLAG,
                    &self.base_dir.join(LOCK_FILE),
                    self.lock_identity,
                )?;
                revalidate_regular_at(
                    &self.base,
                    OsStr::new(RECORD_FILE),
                    O_RDWR_FLAG | O_APPEND_FLAG,
                    &record_path,
                    record_identity,
                )?;
                record_file.seek(SeekFrom::End(0))?;
                record_file.write_all(&encoded)?;
                (Ok(audit_ref(&record.record_hash)), true)
            }
        };

        #[cfg(test)]
        self.run_before_common_epilogue_hook();
        record_file.sync_all()?;
        self.base.sync_all()?;
        revalidate_regular_at(
            &self.base,
            OsStr::new(RECORD_FILE),
            O_RDWR_FLAG | O_APPEND_FLAG,
            &record_path,
            record_identity,
        )?;
        revalidate_regular_at(
            &self.base,
            OsStr::new(LOCK_FILE),
            O_RDWR_FLAG,
            &self.base_dir.join(LOCK_FILE),
            self.lock_identity,
        )?;
        self.validate_base_binding()?;
        let post_write = verify_complete_chain(&mut record_file, &record_path, identity)?;
        let persisted = post_write.matching_record.ok_or_else(|| {
            DurableDeliveryError::InvalidEnvelope(format!(
                "immutable append identity {identity} is missing after persistence: {}",
                record_path.display()
            ))
        })?;
        if wrote_record
            && (persisted.record_kind != record_kind
                || persisted.canonical_sha256 != canonical_sha256
                || persisted.canonical_hex != canonical_hex)
        {
            return Err(DurableDeliveryError::InvalidEnvelope(format!(
                "immutable append identity {identity} changed after persistence: {}",
                record_path.display()
            )));
        }
        disposition
    }

    #[cfg(test)]
    fn set_after_base_lock_hook(&self, hook: impl FnOnce() + Send + 'static) {
        *self.hooks.after_base_lock.lock().expect("test hook mutex") = Some(Box::new(hook));
    }

    #[cfg(test)]
    fn run_after_base_lock_hook(&self) {
        if let Some(hook) = self
            .hooks
            .after_base_lock
            .lock()
            .expect("test hook mutex")
            .take()
        {
            hook();
        }
    }

    #[cfg(test)]
    fn set_before_common_epilogue_hook(&self, hook: impl FnOnce() + Send + 'static) {
        *self
            .hooks
            .before_common_epilogue
            .lock()
            .expect("test hook mutex") = Some(Box::new(hook));
    }

    #[cfg(test)]
    fn run_before_common_epilogue_hook(&self) {
        if let Some(hook) = self
            .hooks
            .before_common_epilogue
            .lock()
            .expect("test hook mutex")
            .take()
        {
            hook();
        }
    }
}

impl ImmutableAppendPort for DurableDeliveryImmutableAppend {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> DurableResult<String> {
        self.append_exact_inner(record_kind, identity, canonical_bytes, sha256)
    }
}

fn process_append_mutex() -> &'static Mutex<()> {
    static STATE: OnceLock<Mutex<()>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(()))
}

fn verify_complete_chain(
    file: &mut File,
    path: &Path,
    target_identity: &str,
) -> DurableResult<VerifiedChain> {
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(0))?;
    let mut reader = BufReader::new(&mut *file);
    let mut expected_previous = None;
    let mut verified_offset = 0_u64;
    let mut line_number = 0_u64;
    let mut identities = HashSet::new();
    let mut matching_record = None;
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }
        if !line.ends_with('\n') {
            return Err(DurableDeliveryError::InvalidEnvelope(format!(
                "immutable append file {} has an incomplete trailing record",
                path.display()
            )));
        }
        let serialized = line
            .strip_suffix('\n')
            .expect("newline suffix checked above");
        let serialized = serialized.strip_suffix('\r').unwrap_or(serialized);
        line_number = line_number.checked_add(1).ok_or_else(|| {
            DurableDeliveryError::InvalidEnvelope(
                "immutable append record count overflow".to_owned(),
            )
        })?;
        let record =
            parse_and_verify_record(serialized, line_number, expected_previous.as_deref())?;
        if !identities.insert(record.identity.clone()) {
            return Err(DurableDeliveryError::InvalidEnvelope(format!(
                "immutable append identity {} is duplicated at line {}",
                record.identity, line_number
            )));
        }
        expected_previous = Some(record.record_hash.clone());
        if record.identity == target_identity {
            matching_record = Some(record);
        }
        verified_offset = verified_offset
            .checked_add(u64::try_from(bytes_read).map_err(|_| {
                DurableDeliveryError::InvalidEnvelope(
                    "immutable append byte offset overflow".to_owned(),
                )
            })?)
            .ok_or_else(|| {
                DurableDeliveryError::InvalidEnvelope(
                    "immutable append byte offset overflow".to_owned(),
                )
            })?;
    }

    if verified_offset != length {
        return Err(DurableDeliveryError::InvalidEnvelope(format!(
            "immutable append verification ended at offset {verified_offset}, expected {length}: {}",
            path.display()
        )));
    }
    Ok(VerifiedChain {
        tail_hash: expected_previous,
        matching_record,
    })
}

fn parse_and_verify_record(
    line: &str,
    line_number: u64,
    expected_previous: Option<&str>,
) -> DurableResult<StoredAppendRecord> {
    if line.trim().is_empty() {
        return Err(DurableDeliveryError::InvalidEnvelope(format!(
            "immutable append line {line_number} is blank"
        )));
    }
    let record: StoredAppendRecord = serde_json::from_str(line)?;
    if record.hash_domain != HASH_DOMAIN {
        return Err(DurableDeliveryError::InvalidEnvelope(format!(
            "immutable append line {line_number} has unsupported hash domain"
        )));
    }
    if record.previous_hash.as_deref() != expected_previous {
        return Err(DurableDeliveryError::InvalidEnvelope(format!(
            "immutable append chain mismatch at line {line_number}"
        )));
    }
    let canonical = hex::decode(&record.canonical_hex).map_err(|error| {
        DurableDeliveryError::InvalidEnvelope(format!(
            "immutable append line {line_number} has invalid canonical hex: {error}"
        ))
    })?;
    validate_input(
        &record.record_kind,
        &record.identity,
        &canonical,
        &record.canonical_sha256,
    )?;
    let expected_hash = compute_record_hash(
        &record.record_kind,
        &record.identity,
        &record.canonical_hex,
        &record.canonical_sha256,
        record.previous_hash.as_deref(),
    )?;
    if expected_hash != record.record_hash {
        return Err(DurableDeliveryError::InvalidEnvelope(format!(
            "immutable append hash mismatch at line {line_number}"
        )));
    }
    Ok(record)
}

fn isolation(detail: impl Into<String>) -> DurableDeliveryError {
    DurableDeliveryError::IsolationViolation(detail.into())
}

fn validate_test_code(test_code: &str) -> DurableResult<()> {
    if !test_code.starts_with("TEST_CODE") {
        return Err(isolation(
            "immutable-audit test namespace must start with TEST_CODE",
        ));
    }
    let mut components = Path::new(test_code).components();
    if !matches!(components.next(), Some(Component::Normal(component)) if component == OsStr::new(test_code))
        || components.next().is_some()
    {
        return Err(isolation(
            "immutable-audit TEST_CODE must be one exact path component",
        ));
    }
    Ok(())
}

fn component_cstring(component: &OsStr) -> DurableResult<CString> {
    CString::new(component.as_bytes())
        .map_err(|_| isolation("immutable-audit path component contains NUL"))
}

fn openat_file(parent: &File, name: &OsStr, flags: i32, mode: u32) -> io::Result<File> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "path component contains NUL"))?;
    // SAFETY: `name` is one live NUL-terminated component, `parent` owns a
    // valid directory descriptor, and a successful descriptor is transferred
    // exactly once into `File`.
    let descriptor = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | O_NOFOLLOW_FLAG | O_NONBLOCK_FLAG | O_CLOEXEC_FLAG,
            mode,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful `openat` returns one newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn mkdirat_component(parent: &File, name: &OsStr, path: &Path) -> DurableResult<()> {
    let name = component_cstring(name)?;
    // SAFETY: `name` is one live NUL-terminated component and `parent`
    // retains a valid directory descriptor.
    let result = unsafe { mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700_u32) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != ErrorKind::AlreadyExists {
            return Err(isolation(format!(
                "create immutable-audit directory {}: {error}",
                path.display()
            )));
        }
    }
    // Sync even after `EEXIST`: another process may have won the mkdir race
    // and crashed before making the parent entry durable.
    parent.sync_all().map_err(|error| {
        isolation(format!(
            "sync immutable-audit parent after ensuring {}: {error}",
            path.display()
        ))
    })?;
    #[cfg(test)]
    PARENT_DIRECTORY_SYNC_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

fn openat_directory(parent: &File, name: &OsStr, path: &Path) -> DurableResult<File> {
    let file = openat_file(parent, name, O_RDONLY_FLAG, 0).map_err(|error| {
        isolation(format!(
            "open immutable-audit directory without symlink traversal {}: {error}",
            path.display()
        ))
    })?;
    directory_identity(&file, path)?;
    Ok(file)
}

fn absolute_normal_components(path: &Path, label: &str) -> DurableResult<Vec<OsString>> {
    if !path.is_absolute() {
        return Err(isolation(format!(
            "{label} must be absolute: {}",
            path.display()
        )));
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => components.push(name.to_os_string()),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(isolation(format!(
                    "{label} is not lexically exact: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(components)
}

fn open_or_create_absolute_directory_no_follow(
    path: &Path,
    creation_boundary: &Path,
) -> DurableResult<PinnedDirectory> {
    let normal_components = absolute_normal_components(path, "fixed immutable-audit namespace")?;
    let boundary_components =
        absolute_normal_components(creation_boundary, "immutable-audit creation boundary")?;
    if normal_components.len() <= boundary_components.len()
        || !normal_components.starts_with(&boundary_components)
    {
        return Err(isolation(format!(
            "fixed immutable-audit namespace must be below its creation boundary: {}",
            path.display()
        )));
    }

    let anchor = OpenOptions::new().read(true).open("/")?;
    let anchor_identity = directory_identity(&anchor, Path::new("/"))?;
    let mut link_count_baselines = vec![directory_link_count(&anchor, Path::new("/"))?];
    let mut directory = anchor.try_clone()?;
    let mut traversed = PathBuf::from("/");
    let mut identities = Vec::with_capacity(normal_components.len());
    let mut directories = Vec::with_capacity(normal_components.len());
    for (index, name) in normal_components.iter().enumerate() {
        traversed.push(name);
        let next = match openat_file(&directory, name, O_RDONLY_FLAG, 0) {
            Ok(file) => {
                directory_identity(&file, &traversed)?;
                file
            }
            Err(error)
                if error.kind() == ErrorKind::NotFound && index >= boundary_components.len() =>
            {
                mkdirat_component(&directory, name, &traversed)?;
                openat_directory(&directory, name, &traversed)?
            }
            Err(error) => {
                return Err(isolation(format!(
                    "traverse immutable-audit namespace without symlink traversal {}: {error}",
                    traversed.display()
                )));
            }
        };
        let identity = directory_identity(&next, &traversed)?;
        link_count_baselines.push(directory_link_count(&next, &traversed)?);
        identities.push(identity);
        directories.push(next.try_clone()?);
        directory = next;
    }
    Ok(PinnedDirectory {
        file: directory,
        binding: PinnedDirectoryBinding {
            anchor,
            anchor_identity,
            components: normal_components,
            identities,
            directories,
            link_count_baselines: Mutex::new(link_count_baselines),
        },
    })
}

fn revalidate_directory_binding(
    binding: &PinnedDirectoryBinding,
    path: &Path,
) -> DurableResult<File> {
    let (first_rebound, first_link_counts) = revalidate_directory_binding_once(binding, path)?;
    drop(first_rebound);
    let (second_rebound, second_link_counts) = revalidate_directory_binding_once(binding, path)?;
    if first_link_counts != second_link_counts {
        return Err(isolation(
            "immutable-audit directory link counts changed during complete-chain validation",
        ));
    }
    let mut baselines = binding
        .link_count_baselines
        .lock()
        .map_err(|_| isolation("immutable-audit directory link-count baseline is poisoned"))?;
    if baselines.len() != second_link_counts.len() {
        return Err(isolation(
            "immutable-audit directory link-count baseline has invalid cardinality",
        ));
    }
    *baselines = second_link_counts;
    Ok(second_rebound)
}

fn revalidate_directory_binding_once(
    binding: &PinnedDirectoryBinding,
    path: &Path,
) -> DurableResult<(File, Vec<u64>)> {
    let anchor_identity = directory_identity(&binding.anchor, Path::new("/"))?;
    if anchor_identity != binding.anchor_identity {
        return Err(isolation(
            "immutable-audit filesystem anchor identity changed",
        ));
    }
    if binding.components.len() != binding.identities.len()
        || binding.components.len() != binding.directories.len()
    {
        return Err(isolation(
            "immutable-audit retained directory binding is internally inconsistent",
        ));
    }
    let retained_anchor_link_count = directory_link_count(&binding.anchor, Path::new("/"))?;
    let mut rebound = OpenOptions::new().read(true).open("/")?;
    let rebound_anchor_identity = directory_identity(&rebound, Path::new("/"))?;
    if rebound_anchor_identity != binding.anchor_identity {
        return Err(isolation(
            "immutable-audit fixed filesystem anchor identity changed",
        ));
    }
    let rebound_anchor_link_count = directory_link_count(&rebound, Path::new("/"))?;
    if rebound_anchor_link_count != retained_anchor_link_count {
        return Err(isolation(
            "immutable-audit filesystem-root link count changed during one chain rebind",
        ));
    }
    let mut link_counts = Vec::with_capacity(binding.components.len() + 1);
    link_counts.push(retained_anchor_link_count);
    let mut traversed = PathBuf::from("/");
    for ((component, expected), retained) in binding
        .components
        .iter()
        .zip(&binding.identities)
        .zip(&binding.directories)
    {
        traversed.push(component);
        let retained_identity = directory_identity(retained, &traversed)?;
        if retained_identity != *expected {
            return Err(isolation(format!(
                "immutable-audit retained ancestor identity changed: {}",
                traversed.display()
            )));
        }
        let retained_link_count = directory_link_count(retained, &traversed)?;
        let next = openat_directory(&rebound, component, &traversed)?;
        let observed = directory_identity(&next, &traversed)?;
        if observed != *expected {
            return Err(isolation(format!(
                "immutable-audit ancestor identity changed while bound: {}",
                traversed.display()
            )));
        }
        let observed_link_count = directory_link_count(&next, &traversed)?;
        if observed_link_count != retained_link_count {
            return Err(isolation(format!(
                "immutable-audit ancestor link count changed during one chain rebind: {}",
                traversed.display()
            )));
        }
        link_counts.push(retained_link_count);
        rebound = next;
    }
    let final_identity = directory_identity(&rebound, path)?;
    if binding.identities.last().copied() != Some(final_identity) {
        return Err(isolation(format!(
            "immutable-audit final directory identity changed: {}",
            path.display()
        )));
    }
    Ok((rebound, link_counts))
}

fn directory_link_count(file: &File, path: &Path) -> DurableResult<u64> {
    let metadata = file.metadata()?;
    if !metadata.is_dir() || metadata.nlink() == 0 {
        return Err(isolation(format!(
            "immutable-audit directory has no stable physical link count: {}",
            path.display()
        )));
    }
    Ok(metadata.nlink())
}

fn directory_identity(file: &File, path: &Path) -> DurableResult<FileIdentity> {
    let metadata = file.metadata()?;
    if !metadata.is_dir() {
        return Err(isolation(format!(
            "immutable-audit directory binding is not a directory: {}",
            path.display()
        )));
    }
    if metadata.nlink() == 0 {
        return Err(isolation(format!(
            "immutable-audit directory has no physical links: {}",
            path.display()
        )));
    }
    validate_owner_and_mode(&metadata, "directory", path)?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        is_directory: metadata.is_dir(),
        is_file: metadata.is_file(),
    })
}

fn regular_identity(file: &File, path: &Path) -> DurableResult<FileIdentity> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(isolation(format!(
            "immutable-audit leaf is not a regular file: {}",
            path.display()
        )));
    }
    if metadata.nlink() != 1 {
        return Err(isolation(format!(
            "immutable-audit leaf has {} physical links, expected exactly one: {}",
            metadata.nlink(),
            path.display()
        )));
    }
    validate_owner_and_mode(&metadata, "leaf", path)?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        uid: metadata.uid(),
        is_directory: metadata.is_dir(),
        is_file: metadata.is_file(),
    })
}

fn validate_owner_and_mode(
    metadata: &std::fs::Metadata,
    kind: &str,
    path: &Path,
) -> DurableResult<()> {
    // SAFETY: `geteuid` has no arguments and returns the effective UID for
    // this process without retaining pointers or borrowing Rust memory.
    let effective_uid = unsafe { geteuid() };
    if !immutable_owner_allowed(metadata.uid(), effective_uid, kind == "directory") {
        return Err(isolation(format!(
            "immutable-audit {kind} is owned by uid {}, expected root or effective uid {}: {}",
            metadata.uid(),
            effective_uid,
            path.display()
        )));
    }
    if metadata.mode() & GROUP_OR_WORLD_WRITE_BITS != 0 {
        return Err(isolation(format!(
            "immutable-audit {kind} is group/world writable (mode {:o}): {}",
            metadata.mode() & 0o7777,
            path.display()
        )));
    }
    Ok(())
}

fn immutable_owner_allowed(uid: u32, effective_uid: u32, is_directory: bool) -> bool {
    uid == effective_uid || (is_directory && uid == 0)
}

fn open_unique_regular_at(
    parent: &File,
    name: &OsStr,
    flags: i32,
    path: &Path,
) -> DurableResult<File> {
    let file = openat_file(parent, name, flags, 0o600_u32).map_err(|error| {
        isolation(format!(
            "open immutable-audit leaf without symlink traversal {}: {error}",
            path.display()
        ))
    })?;
    regular_identity(&file, path)?;
    Ok(file)
}

fn revalidate_regular_at(
    parent: &File,
    name: &OsStr,
    flags: i32,
    path: &Path,
    expected: FileIdentity,
) -> DurableResult<()> {
    let reopened = open_unique_regular_at(parent, name, flags, path)?;
    let observed = regular_identity(&reopened, path)?;
    if observed != expected {
        return Err(isolation(format!(
            "immutable-audit leaf identity changed while pinned: {}",
            path.display()
        )));
    }
    Ok(())
}

fn validate_input(
    record_kind: &str,
    identity: &str,
    canonical_bytes: &[u8],
    canonical_sha256: &str,
) -> DurableResult<()> {
    if record_kind.trim().is_empty()
        || identity.trim().is_empty()
        || canonical_bytes.is_empty()
        || canonical_sha256.trim().is_empty()
    {
        return Err(DurableDeliveryError::InvalidEnvelope(
            "immutable append requires kind, identity, bytes and sha256".to_owned(),
        ));
    }
    if sha256_hex(canonical_bytes) != canonical_sha256 {
        return Err(DurableDeliveryError::InvalidEnvelope(
            "immutable append canonical sha256 mismatch".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn read_and_verify(path: &Path) -> DurableResult<Vec<StoredAppendRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = File::open(path)?;
    read_and_verify_file(&mut file, path)
}

#[cfg(test)]
fn read_and_verify_file(file: &mut File, path: &Path) -> DurableResult<Vec<StoredAppendRecord>> {
    file.seek(SeekFrom::Start(0))?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    if !content.is_empty() && !content.ends_with('\n') {
        return Err(DurableDeliveryError::InvalidEnvelope(format!(
            "immutable append file {} has an incomplete trailing record",
            path.display()
        )));
    }

    let mut records = Vec::new();
    let mut expected_previous: Option<String> = None;
    for (index, line) in content.lines().enumerate() {
        let line_number = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                DurableDeliveryError::InvalidEnvelope(
                    "immutable append record count overflow".to_owned(),
                )
            })?;
        let record = parse_and_verify_record(line, line_number, expected_previous.as_deref())?;
        expected_previous = Some(record.record_hash.clone());
        records.push(record);
    }
    Ok(records)
}

fn compute_record_hash(
    record_kind: &str,
    identity: &str,
    canonical_hex: &str,
    canonical_sha256: &str,
    previous_hash: Option<&str>,
) -> DurableResult<String> {
    let material = RecordHashMaterial {
        hash_domain: HASH_DOMAIN,
        record_kind,
        identity,
        canonical_hex,
        canonical_sha256,
        previous_hash,
    };
    Ok(sha256_hex(&serde_json::to_vec(&material)?))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn audit_ref(record_hash: &str) -> String {
    format!("durable-delivery:{record_hash}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Output};
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    };
    use std::time::Duration;

    const NAMESPACE_CHILD_CASE_ENV: &str = "BR192_IMMUTABLE_APPEND_CHILD_CASE";
    const NAMESPACE_CHILD_TEST_CODE_ENV: &str = "BR192_IMMUTABLE_APPEND_TEST_CODE";
    const NAMESPACE_CHILD_ANCHOR_ENV: &str = "BR192_IMMUTABLE_APPEND_ANCHOR";
    const NAMESPACE_CHILD_TEST: &str =
        "event::durable_delivery_append::tests::TEST_CODE_br192_immutable_append_namespace_child";
    static NEXT_NAMESPACE_TEST_ID: AtomicU64 = AtomicU64::new(0);

    struct NamespaceWorkspace {
        root: PathBuf,
        test_code: String,
        retained: File,
        device: u64,
        inode: u64,
    }

    impl NamespaceWorkspace {
        fn new(label: &str) -> Self {
            use std::os::unix::fs::MetadataExt;
            let id = NEXT_NAMESPACE_TEST_ID.fetch_add(1, Ordering::SeqCst);
            let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/test");
            fs::create_dir_all(&parent).expect("create TEST_CODE append fixture parent");
            let root = parent.join(format!(
                "TEST_CODE_BR192_APPEND_NAMESPACE_{label}_{}_{}",
                std::process::id(),
                id
            ));
            fs::create_dir(&root).expect("create fresh isolated namespace workspace");
            fs::create_dir(root.join("unrelated_cwd"))
                .expect("create CWD-independent namespace fixture");
            let retained = File::open(&root).expect("retain append namespace inode");
            let metadata = retained
                .metadata()
                .expect("inspect retained append namespace inode");
            Self {
                root,
                test_code: format!("TEST_CODE_BR192_APPEND_{label}_{id}"),
                retained,
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }

        fn production_base(&self) -> PathBuf {
            self.root.join("data/durable_delivery_audit")
        }

        fn test_base(&self) -> PathBuf {
            self.root
                .join("data/test")
                .join(&self.test_code)
                .join("durable_delivery_audit")
        }

        fn run_child(&self, case: &str) -> Output {
            Command::new(std::env::current_exe().expect("current test executable"))
                .args(["--ignored", "--exact", NAMESPACE_CHILD_TEST, "--nocapture"])
                .current_dir(self.root.join("unrelated_cwd"))
                .env(NAMESPACE_CHILD_CASE_ENV, case)
                .env(NAMESPACE_CHILD_TEST_CODE_ENV, &self.test_code)
                .env(NAMESPACE_CHILD_ANCHOR_ENV, &self.root)
                .env_remove("STOCK_ENV_MODE")
                .output()
                .expect("run immutable-append namespace child")
        }

        fn assert_child_rejects(&self, case: &str) {
            let output = self.run_child(case);
            assert!(
                output.status.success(),
                "namespace child did not observe the required isolation rejection\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    impl Drop for NamespaceWorkspace {
        fn drop(&mut self) {
            use std::os::unix::fs::MetadataExt;
            let retained = self
                .retained
                .metadata()
                .expect("inspect retained append namespace before cleanup");
            let current = fs::symlink_metadata(&self.root)
                .expect("append namespace must still exist before cleanup");
            assert!(
                current.file_type().is_dir()
                    && current.dev() == self.device
                    && current.ino() == self.inode
                    && retained.dev() == self.device
                    && retained.ino() == self.inode,
                "refuse to remove a replaced append TEST_CODE namespace"
            );
            fs::remove_dir_all(&self.root)
                .expect("remove retained exact append TEST_CODE namespace");
        }
    }

    struct AppendFixture {
        root: PathBuf,
        retained: File,
        device: u64,
        inode: u64,
    }

    impl AppendFixture {
        fn new(label: &str) -> Self {
            use std::os::unix::fs::MetadataExt;
            let id = NEXT_NAMESPACE_TEST_ID.fetch_add(1, Ordering::SeqCst);
            let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/test");
            fs::create_dir_all(&parent).expect("create TEST_CODE append fixture parent");
            let root = parent.join(format!(
                "TEST_CODE_BR192_APPEND_{label}_{}_{}",
                std::process::id(),
                id
            ));
            fs::create_dir(&root).expect("create fresh append fixture");
            let retained = File::open(&root).expect("retain append fixture inode");
            let metadata = retained
                .metadata()
                .expect("inspect retained append fixture inode");
            Self {
                root,
                retained,
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }

        fn path(&self) -> &Path {
            &self.root
        }
    }

    impl Drop for AppendFixture {
        fn drop(&mut self) {
            use std::os::unix::fs::MetadataExt;
            let retained = self
                .retained
                .metadata()
                .expect("inspect retained append fixture before cleanup");
            let current = fs::symlink_metadata(&self.root)
                .expect("append fixture must still exist before cleanup");
            assert!(
                current.file_type().is_dir()
                    && current.dev() == self.device
                    && current.ino() == self.inode
                    && retained.dev() == self.device
                    && retained.ino() == self.inode,
                "refuse to remove a replaced append fixture"
            );
            fs::remove_dir_all(&self.root).expect("remove retained exact append fixture");
        }
    }

    fn append_probe(
        append: DurableResult<DurableDeliveryImmutableAppend>,
    ) -> DurableResult<String> {
        let append = append?;
        let bytes = br#"{"state":"Delivered"}"#;
        append.append_exact(
            "DecisionStateChanged",
            "TEST_CODE_BR192_NAMESPACE_PROBE",
            bytes,
            &sha256_hex(bytes),
        )
    }

    fn assert_isolation_violation<T>(result: DurableResult<T>) {
        assert!(
            matches!(result, Err(DurableDeliveryError::IsolationViolation(_))),
            "physical namespace alias must fail with IsolationViolation"
        );
    }

    fn fixture(label: &str) -> (AppendFixture, DurableDeliveryImmutableAppend) {
        let fixture = AppendFixture::new(label);
        let append = DurableDeliveryImmutableAppend::new(fixture.path().to_path_buf());
        (fixture, append)
    }

    #[test]
    fn exact_replay_is_idempotent_and_identity_conflict_is_rejected() {
        let (root, append) = fixture("REPLAY_CONFLICT");
        let bytes = br#"{"state":"Delivered"}"#;
        let digest = sha256_hex(bytes);

        let first = append
            .append_exact("DecisionStateChanged", "TEST_CODE_ID_1", bytes, &digest)
            .expect("first append");
        let replay = append
            .append_exact("DecisionStateChanged", "TEST_CODE_ID_1", bytes, &digest)
            .expect("byte-identical replay");
        assert_eq!(first, replay);
        assert!(matches!(
            append.append_exact(
                "DecisionStateChanged",
                "TEST_CODE_ID_1",
                br#"{"state":"Rejected"}"#,
                &sha256_hex(br#"{"state":"Rejected"}"#)
            ),
            Err(DurableDeliveryError::ImmutableAppendConflict(identity))
                if identity == "TEST_CODE_ID_1"
        ));

        let lines = fs::read_to_string(root.path().join(RECORD_FILE)).expect("audit file");
        assert_eq!(lines.lines().count(), 1);
    }

    #[test]
    fn authoritative_record_rejects_unknown_unhashed_fields() {
        let (root, append) = fixture("UNKNOWN_FIELD_TAMPER");
        let bytes = br#"{"state":"Delivered"}"#;
        append
            .append_exact(
                "DecisionStateChanged",
                "TEST_CODE_UNKNOWN_FIELD",
                bytes,
                &sha256_hex(bytes),
            )
            .expect("seed authoritative record");
        let record_path = root.path().join(RECORD_FILE);
        let line = fs::read_to_string(&record_path).expect("read authoritative record");
        let mut value: serde_json::Value =
            serde_json::from_str(line.trim_end()).expect("parse authoritative record");
        value
            .as_object_mut()
            .expect("authoritative record is an object")
            .insert(
                "unhashed_extension".to_owned(),
                serde_json::Value::String("TEST_CODE_TAMPER".to_owned()),
            );
        let mut tampered = serde_json::to_vec(&value).expect("encode unknown-field tamper");
        tampered.push(b'\n');
        fs::write(&record_path, tampered).expect("persist unknown-field tamper");

        assert!(
            read_and_verify(&record_path).is_err(),
            "unknown fields outside the hash material must fail closed"
        );
    }

    #[test]
    fn concurrent_unique_appends_form_one_valid_hash_chain() {
        let (root, append) = fixture("CONCURRENT_CHAIN");
        let append = Arc::new(append);
        let mut threads = Vec::new();
        for index in 0..8 {
            let append = Arc::clone(&append);
            threads.push(std::thread::spawn(move || {
                let bytes = format!(r#"{{"index":{index}}}"#).into_bytes();
                append
                    .append_exact(
                        "BudgetReservationChanged",
                        &format!("TEST_CODE_ID_{index}"),
                        &bytes,
                        &sha256_hex(&bytes),
                    )
                    .expect("concurrent append");
            }));
        }
        for thread in threads {
            thread.join().expect("thread");
        }

        let records = read_and_verify(&root.path().join(RECORD_FILE)).expect("valid chain");
        assert_eq!(records.len(), 8);
    }

    #[test]
    fn exact_replay_runs_common_sync_and_binding_epilogue() {
        let (root, append) = fixture("REPLAY_EPILOGUE");
        let bytes = br#"{"state":"Delivered"}"#;
        let digest = sha256_hex(bytes);
        append
            .append_exact("DecisionStateChanged", "TEST_CODE_REPLAY", bytes, &digest)
            .expect("first append");

        let record_path = root.path().join(RECORD_FILE);
        let displaced_path = root.path().join("durable_delivery_v1.displaced.jsonl");
        append.set_before_common_epilogue_hook({
            let record_path = record_path.clone();
            let displaced_path = displaced_path.clone();
            move || {
                fs::rename(&record_path, &displaced_path)
                    .expect("displace record before replay epilogue");
                fs::write(&record_path, b"").expect("replace record before replay epilogue");
            }
        });

        assert_isolation_violation(append.append_exact(
            "DecisionStateChanged",
            "TEST_CODE_REPLAY",
            bytes,
            &digest,
        ));
    }

    #[test]
    fn base_lock_serializes_and_pinned_lock_replacement_fails_closed() {
        let root = AppendFixture::new("LOCK_REPLACEMENT");
        let append_a = DurableDeliveryImmutableAppend::new(root.path().to_path_buf());
        let append_b = DurableDeliveryImmutableAppend::new(root.path().to_path_buf());
        let (base_locked_tx, base_locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        append_a.set_after_base_lock_hook(move || {
            base_locked_tx.send(()).expect("signal pinned base lock");
            release_rx.recv().expect("release pinned base lock");
        });

        let first = std::thread::spawn(move || {
            let bytes = br#"{"writer":"a"}"#;
            append_a.append_exact(
                "DecisionStateChanged",
                "TEST_CODE_LOCK_REPLACEMENT_A",
                bytes,
                &sha256_hex(bytes),
            )
        });
        base_locked_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first writer must hold the base lock");
        let lock_probe = FileExt::try_lock_exclusive(&*append_b.base)
            .expect_err("independent pinned base descriptor must serialize");
        assert_eq!(lock_probe.kind(), ErrorKind::WouldBlock);

        let lock_path = root.path().join(LOCK_FILE);
        let displaced_lock_path = root.path().join(".durable_delivery_v1.displaced.lock");
        fs::rename(&lock_path, &displaced_lock_path).expect("displace pinned lock leaf");
        fs::write(&lock_path, b"replacement lock").expect("replace pinned lock leaf");
        release_tx.send(()).expect("release first writer");

        assert_isolation_violation(first.join().expect("first writer thread"));
        let bytes = br#"{"writer":"b"}"#;
        assert_isolation_violation(append_b.append_exact(
            "DecisionStateChanged",
            "TEST_CODE_LOCK_REPLACEMENT_B",
            bytes,
            &sha256_hex(bytes),
        ));
    }

    #[test]
    fn fixed_namespace_rejects_an_ancestor_swap_before_append() {
        let workspace = NamespaceWorkspace::new("ANCESTOR_SWAP");
        let append = DurableDeliveryImmutableAppend::bind_test_anchor(
            &workspace.root,
            Path::new(PRODUCTION_BASE_DIR),
        )
        .expect("bind fixed production-semantic test namespace");
        let data = workspace.root.join("data");
        let displaced = workspace.root.join("displaced_data");
        append.set_after_base_lock_hook({
            let data = data.clone();
            let displaced = displaced.clone();
            move || {
                fs::rename(&data, &displaced).expect("displace fixed data ancestor");
                fs::create_dir(&data).expect("install replacement data ancestor");
            }
        });
        let bytes = br#"{"state":"Delivered"}"#;
        assert_isolation_violation(append.append_exact(
            "DecisionStateChanged",
            "TEST_CODE_ANCESTOR_SWAP",
            bytes,
            &sha256_hex(bytes),
        ));
        assert!(
            !displaced
                .join("durable_delivery_audit")
                .join(RECORD_FILE)
                .exists(),
            "displaced ancestor must not receive an audit record"
        );
        assert!(
            !data
                .join("durable_delivery_audit")
                .join(RECORD_FILE)
                .exists(),
            "replacement ancestor must not receive an audit record"
        );
    }

    #[test]
    fn stable_directory_nlink_drift_refreshes_after_two_complete_rebinds() {
        let workspace = NamespaceWorkspace::new("NLINK_REFRESH");
        let append = DurableDeliveryImmutableAppend::bind_test_anchor(
            &workspace.root,
            Path::new(PRODUCTION_BASE_DIR),
        )
        .expect("bind fixed production-semantic TEST_CODE namespace");
        let transient_child = append.base_dir().join("TEST_CODE_TRANSIENT_CHILD");
        fs::create_dir(&transient_child).expect("create legitimate child directory");
        let first = br#"{"phase":"mkdir"}"#;
        append
            .append_exact(
                "DecisionStateChanged",
                "TEST_CODE_NLINK_MKDIR",
                first,
                &sha256_hex(first),
            )
            .expect("stable mkdir nlink drift refreshes after full-chain validation");

        fs::remove_dir(&transient_child).expect("remove legitimate child directory");
        let second = br#"{"phase":"rmdir"}"#;
        append
            .append_exact(
                "DecisionStateChanged",
                "TEST_CODE_NLINK_RMDIR",
                second,
                &sha256_hex(second),
            )
            .expect("stable rmdir nlink drift refreshes after full-chain validation");
    }

    #[test]
    fn fixed_namespace_creation_syncs_each_new_parent_entry() {
        let workspace = NamespaceWorkspace::new("PARENT_SYNC");
        let output = workspace.run_child("parent_sync");
        assert!(
            output.status.success(),
            "namespace child did not sync every created directory entry\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn group_or_world_writable_directory_is_rejected() {
        let workspace = NamespaceWorkspace::new("WRITABLE_DIRECTORY");
        fs::create_dir_all(workspace.root.join("data")).expect("create writable directory");
        fs::set_permissions(
            workspace.root.join("data"),
            fs::Permissions::from_mode(0o777),
        )
        .expect("make directory group/world writable");
        workspace.assert_child_rejects("group_world_writable_directory");
    }

    #[test]
    fn group_or_world_writable_lock_leaf_is_rejected() {
        let workspace = NamespaceWorkspace::new("WRITABLE_LOCK");
        fs::create_dir_all(workspace.production_base()).expect("create production audit namespace");
        let lock_path = workspace.production_base().join(LOCK_FILE);
        fs::write(&lock_path, b"lock").expect("seed writable lock");
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o660))
            .expect("make lock group writable");
        workspace.assert_child_rejects("group_world_writable_lock");
    }

    #[test]
    fn br192_immutable_directory_safe_mode_drift_is_rejected() {
        let workspace = NamespaceWorkspace::new("SAFE_MODE_DRIFT");
        let append = DurableDeliveryImmutableAppend::bind_test_anchor(
            &workspace.root,
            Path::new(PRODUCTION_BASE_DIR),
        )
        .expect("bind fixed production-semantic TEST_CODE namespace");
        let path = append.base_dir.clone();
        let original = fs::metadata(&path).unwrap().permissions();
        let original_mode = original.mode() & 0o7777;
        let drifted_mode = if original_mode == 0o700 { 0o750 } else { 0o700 };
        fs::set_permissions(&path, fs::Permissions::from_mode(drifted_mode))
            .expect("apply another safe TEST_CODE immutable-audit mode");
        let result = append.validate_base_binding();
        fs::set_permissions(&path, original).expect("restore immutable-audit permissions");
        assert_isolation_violation(result);
    }

    #[test]
    fn br192_immutable_directory_allowed_owner_drift_is_rejected() {
        let effective_uid = 501;
        assert!(immutable_owner_allowed(0, effective_uid, true));
        assert!(immutable_owner_allowed(effective_uid, effective_uid, true));
        let root_owned = FileIdentity {
            device: 1,
            inode: 2,
            mode: 0o040700,
            uid: 0,
            is_directory: true,
            is_file: false,
        };
        let effective_owned = FileIdentity {
            uid: effective_uid,
            ..root_owned
        };
        assert_ne!(
            root_owned, effective_owned,
            "two individually allowed owners are not the same retained authority"
        );
    }

    #[cfg(unix)]
    #[test]
    fn br192_production_base_directory_symlink_to_test_namespace_is_rejected() {
        use std::os::unix::fs::symlink;

        let workspace = NamespaceWorkspace::new("PROD_BASE_SYMLINK");
        fs::create_dir_all(workspace.test_base()).expect("create physical test audit namespace");
        fs::create_dir_all(workspace.root.join("data"))
            .expect("create production namespace parent");
        symlink(workspace.test_base(), workspace.production_base())
            .expect("symlink production audit base to test namespace");

        workspace.assert_child_rejects("production_base_symlink");
    }

    #[cfg(unix)]
    #[test]
    fn br192_test_base_directory_symlink_to_production_namespace_is_rejected() {
        use std::os::unix::fs::symlink;

        let workspace = NamespaceWorkspace::new("TEST_BASE_SYMLINK");
        fs::create_dir_all(workspace.production_base())
            .expect("create physical production audit namespace");
        fs::create_dir_all(
            workspace
                .test_base()
                .parent()
                .expect("test namespace parent"),
        )
        .expect("create test namespace parent");
        symlink(workspace.production_base(), workspace.test_base())
            .expect("symlink test audit base to production namespace");

        workspace.assert_child_rejects("test_base_symlink");
    }

    #[cfg(unix)]
    #[test]
    fn br192_lock_file_symlink_across_namespaces_is_rejected() {
        use std::os::unix::fs::symlink;

        let workspace = NamespaceWorkspace::new("LOCK_SYMLINK");
        fs::create_dir_all(workspace.production_base()).expect("create production audit namespace");
        fs::create_dir_all(workspace.test_base()).expect("create test audit namespace");
        let test_lock = workspace.test_base().join(LOCK_FILE);
        fs::write(&test_lock, b"TEST_CODE lock").expect("seed physical test lock");
        symlink(&test_lock, workspace.production_base().join(LOCK_FILE))
            .expect("symlink production lock to test lock");

        workspace.assert_child_rejects("production_lock_symlink");
    }

    #[cfg(unix)]
    #[test]
    fn br192_record_file_symlink_across_namespaces_is_rejected() {
        use std::os::unix::fs::symlink;

        let workspace = NamespaceWorkspace::new("RECORD_SYMLINK");
        fs::create_dir_all(workspace.production_base()).expect("create production audit namespace");
        fs::create_dir_all(workspace.test_base()).expect("create test audit namespace");
        let test_record = workspace.test_base().join(RECORD_FILE);
        fs::write(&test_record, b"").expect("seed physical test record");
        symlink(&test_record, workspace.production_base().join(RECORD_FILE))
            .expect("symlink production record to test record");

        workspace.assert_child_rejects("production_record_symlink");
    }

    #[cfg(unix)]
    #[test]
    fn br192_lock_file_hardlink_across_namespaces_is_rejected() {
        let workspace = NamespaceWorkspace::new("LOCK_HARDLINK");
        fs::create_dir_all(workspace.production_base()).expect("create production audit namespace");
        fs::create_dir_all(workspace.test_base()).expect("create test audit namespace");
        let production_lock = workspace.production_base().join(LOCK_FILE);
        fs::write(&production_lock, b"production lock").expect("seed production lock");
        fs::hard_link(&production_lock, workspace.test_base().join(LOCK_FILE))
            .expect("hardlink test lock to production lock inode");

        workspace.assert_child_rejects("test_lock_hardlink");
    }

    #[cfg(unix)]
    #[test]
    fn br192_record_file_hardlink_across_namespaces_is_rejected() {
        let workspace = NamespaceWorkspace::new("RECORD_HARDLINK");
        fs::create_dir_all(workspace.production_base()).expect("create production audit namespace");
        fs::create_dir_all(workspace.test_base()).expect("create test audit namespace");
        let production_record = workspace.production_base().join(RECORD_FILE);
        fs::write(&production_record, b"").expect("seed production record");
        fs::hard_link(&production_record, workspace.test_base().join(RECORD_FILE))
            .expect("hardlink test record to production record inode");

        workspace.assert_child_rejects("test_record_hardlink");
    }

    #[test]
    fn br192_constructor_binding_is_immutable_after_environment_flip() {
        let workspace = NamespaceWorkspace::new("ENV_FLIP");
        let output = workspace.run_child("production_environment_flip");
        assert!(
            output.status.success(),
            "namespace child did not preserve the constructor binding\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            workspace.production_base().join(RECORD_FILE).is_file(),
            "environment flip must retain the production audit binding"
        );
        assert!(
            !workspace
                .root
                .join("data/test/durable_delivery_audit")
                .exists(),
            "environment flip must not redirect the append into the test namespace"
        );
        assert!(
            !workspace.test_base().join(RECORD_FILE).exists(),
            "environment flip must not redirect the append into a TEST_CODE namespace"
        );
    }

    #[test]
    #[ignore = "invoked as an isolated-cwd child by BR-192 namespace tests"]
    #[allow(non_snake_case)]
    fn TEST_CODE_br192_immutable_append_namespace_child() {
        let case =
            std::env::var(NAMESPACE_CHILD_CASE_ENV).expect("namespace child case must be supplied");
        let test_code = std::env::var(NAMESPACE_CHILD_TEST_CODE_ENV)
            .expect("namespace child TEST_CODE must be supplied");
        let anchor = PathBuf::from(
            std::env::var(NAMESPACE_CHILD_ANCHOR_ENV)
                .expect("namespace child absolute anchor must be supplied"),
        );
        let production_append = || {
            DurableDeliveryImmutableAppend::bind_test_anchor(
                &anchor,
                Path::new(PRODUCTION_BASE_DIR),
            )
        };
        let test_append = || {
            DurableDeliveryImmutableAppend::bind_test_anchor(
                &anchor,
                &PathBuf::from("data/test")
                    .join(&test_code)
                    .join("durable_delivery_audit"),
            )
        };

        match case.as_str() {
            "production_base_symlink" | "production_lock_symlink" | "production_record_symlink" => {
                assert_isolation_violation(append_probe(production_append()));
            }
            "group_world_writable_directory" | "group_world_writable_lock" => {
                assert_isolation_violation(append_probe(production_append()));
            }
            "test_base_symlink" | "test_lock_hardlink" | "test_record_hardlink" => {
                assert_isolation_violation(append_probe(test_append()));
            }
            "parent_sync" => {
                assert_eq!(
                    PARENT_DIRECTORY_SYNC_COUNT.load(Ordering::SeqCst),
                    0,
                    "isolated child must start with no observed parent sync"
                );
                test_append().expect("create fixed TEST_CODE audit namespace");
                assert_eq!(
                    PARENT_DIRECTORY_SYNC_COUNT.load(Ordering::SeqCst),
                    4,
                    "data/test/TEST_CODE/durable_delivery_audit each require a parent sync"
                );
            }
            "production_environment_flip" => {
                std::env::set_var("STOCK_ENV_MODE", "prod");
                let append = production_append().expect("bind production audit namespace");
                assert_eq!(
                    append.base_dir(),
                    anchor.join(PRODUCTION_BASE_DIR),
                    "production constructor must expose its fixed lexical namespace"
                );
                std::env::set_var("STOCK_ENV_MODE", "test");
                append_probe(Ok(append)).expect("append through immutable production binding");
            }
            other => panic!("unknown immutable-append namespace child case: {other}"),
        }
    }
}
