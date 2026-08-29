//! BR-184 process-descriptor evidence for an already pinned SQLite object set.
//!
//! This module does not open a database by pathname and does not own SQLite's
//! descriptors. The connection owner captures a snapshot immediately before
//! and after SQLite opens and primes its handles, then retains the resulting
//! descriptor numbers alongside the connection. Checkout validation resolves
//! those descriptor numbers through the process descriptor filesystem and
//! compares the complete file-object identity.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File, Metadata};
use std::io;
use std::mem::ManuallyDrop;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

const FILE_TYPE_MASK: u32 = 0o170_000;
const REGULAR_FILE_TYPE: u32 = 0o100_000;
const DIRECTORY_FILE_TYPE: u32 = 0o040_000;
const SYMLINK_FILE_TYPE: u32 = 0o120_000;
const SOCKET_FILE_TYPE: u32 = 0o140_000;
const FIFO_FILE_TYPE: u32 = 0o010_000;
const CHARACTER_DEVICE_FILE_TYPE: u32 = 0o020_000;
const BLOCK_DEVICE_FILE_TYPE: u32 = 0o060_000;
const BAD_FILE_DESCRIPTOR_OS_ERROR: i32 = 9;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum FileObjectType {
    RegularFile,
    Directory,
    Symlink,
    Socket,
    Fifo,
    CharacterDevice,
    BlockDevice,
    Unknown(u32),
}

impl FileObjectType {
    fn from_mode(mode: u32) -> Self {
        match mode & FILE_TYPE_MASK {
            REGULAR_FILE_TYPE => Self::RegularFile,
            DIRECTORY_FILE_TYPE => Self::Directory,
            SYMLINK_FILE_TYPE => Self::Symlink,
            SOCKET_FILE_TYPE => Self::Socket,
            FIFO_FILE_TYPE => Self::Fifo,
            CHARACTER_DEVICE_FILE_TYPE => Self::CharacterDevice,
            BLOCK_DEVICE_FILE_TYPE => Self::BlockDevice,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct FileObjectIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    object_type: FileObjectType,
}

impl FileObjectIdentity {
    pub(super) fn from_file(file: &File) -> Result<Self, DescriptorAttestationError> {
        let metadata = file.metadata().map_err(|error| {
            DescriptorAttestationError::unavailable(format!(
                "cannot inspect pinned file descriptor: {error}"
            ))
        })?;
        Ok(Self::from_metadata(&metadata))
    }

    pub(super) fn from_descriptor(descriptor: RawFd) -> Result<Self, DescriptorAttestationError> {
        let metadata = descriptor_metadata(descriptor).map_err(|error| {
            DescriptorAttestationError::identity_changed(format!(
                "descriptor {descriptor} cannot be inspected: {error}"
            ))
        })?;
        Ok(Self::from_metadata(&metadata))
    }

    fn from_metadata(metadata: &Metadata) -> Self {
        let mode = metadata.mode();
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode,
            object_type: FileObjectType::from_mode(mode),
        }
    }

    pub(super) fn device(self) -> u64 {
        self.device
    }

    pub(super) fn inode(self) -> u64 {
        self.inode
    }

    pub(super) fn mode(self) -> u32 {
        self.mode
    }

    #[cfg(test)]
    pub(super) fn object_type(self) -> FileObjectType {
        self.object_type
    }

    fn require_regular(self, role: SqliteObjectRole) -> Result<(), DescriptorAttestationError> {
        if self.object_type == FileObjectType::RegularFile {
            return Ok(());
        }
        Err(DescriptorAttestationError::identity_changed(format!(
            "{} descriptor is not a regular file: type={:?} mode={:#o}",
            role.label(),
            self.object_type,
            self.mode
        )))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum SqliteObjectRole {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PinnedSqliteObjectSet {
    main: FileObjectIdentity,
    wal: FileObjectIdentity,
    shm: FileObjectIdentity,
}

impl PinnedSqliteObjectSet {
    pub(super) fn from_files(
        main: &File,
        wal: &File,
        shm: &File,
    ) -> Result<Self, DescriptorAttestationError> {
        Self::from_identities(
            FileObjectIdentity::from_file(main)?,
            FileObjectIdentity::from_file(wal)?,
            FileObjectIdentity::from_file(shm)?,
        )
    }

    fn from_identities(
        main: FileObjectIdentity,
        wal: FileObjectIdentity,
        shm: FileObjectIdentity,
    ) -> Result<Self, DescriptorAttestationError> {
        main.require_regular(SqliteObjectRole::Main)?;
        wal.require_regular(SqliteObjectRole::Wal)?;
        shm.require_regular(SqliteObjectRole::Shm)?;

        let distinct = BTreeSet::from([main, wal, shm]);
        if distinct.len() != 3 {
            return Err(DescriptorAttestationError::ambiguous(
                "main, wal and shm expected identities must be distinct",
            ));
        }

        Ok(Self { main, wal, shm })
    }

    pub(super) fn identity(&self, role: SqliteObjectRole) -> FileObjectIdentity {
        match role {
            SqliteObjectRole::Main => self.main,
            SqliteObjectRole::Wal => self.wal,
            SqliteObjectRole::Shm => self.shm,
        }
    }
}

#[derive(Debug)]
pub(super) struct ProcessDescriptorSnapshot {
    descriptors: BTreeMap<RawFd, FileObjectIdentity>,
}

impl ProcessDescriptorSnapshot {
    pub(super) fn capture() -> Result<Self, DescriptorAttestationError> {
        let root = process_descriptor_root()?;
        let iterator = fs::read_dir(root).map_err(|error| {
            DescriptorAttestationError::unavailable(format!(
                "cannot enumerate {}: {error}",
                root.display()
            ))
        })?;

        // Collect descriptor numbers first and drop ReadDir before inspecting
        // them. This prevents the enumeration handle itself from entering the
        // snapshot.
        let descriptor_numbers = iterator
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<RawFd>().ok())
            })
            .collect::<BTreeSet<_>>();

        let mut descriptors = BTreeMap::new();
        for descriptor in descriptor_numbers {
            match descriptor_metadata(descriptor) {
                Ok(metadata) => {
                    descriptors.insert(descriptor, FileObjectIdentity::from_metadata(&metadata));
                }
                Err(error) if descriptor_disappeared_during_snapshot(&error) => {
                    // An unrelated descriptor may close concurrently. A
                    // relevant SQLite handle will then be absent from the
                    // exact delta and fail closed below.
                }
                Err(error) => {
                    return Err(DescriptorAttestationError::unavailable(format!(
                        "cannot inspect process descriptor {descriptor}: {error}"
                    )));
                }
            }
        }

        Ok(Self { descriptors })
    }

    fn delta(&self, after: &Self) -> Vec<DescriptorCandidate> {
        after
            .descriptors
            .iter()
            .filter_map(|(&descriptor, &identity)| {
                (self.descriptors.get(&descriptor) != Some(&identity)).then_some(
                    DescriptorCandidate {
                        descriptor,
                        identity,
                    },
                )
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DescriptorCandidate {
    descriptor: RawFd,
    identity: FileObjectIdentity,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct RetainedSqliteHandle {
    descriptor: RawFd,
    identity: FileObjectIdentity,
}

impl RetainedSqliteHandle {
    #[cfg(test)]
    pub(super) fn descriptor(&self) -> RawFd {
        self.descriptor
    }

    #[cfg(test)]
    pub(super) fn identity(&self) -> FileObjectIdentity {
        self.identity
    }

    fn validate(
        &self,
        role: SqliteObjectRole,
        expected: FileObjectIdentity,
    ) -> Result<(), DescriptorAttestationError> {
        if self.identity != expected {
            return Err(DescriptorAttestationError::identity_changed(format!(
                "{} captured identity no longer matches the pinned object",
                role.label()
            )));
        }

        let current = FileObjectIdentity::from_descriptor(self.descriptor)?;
        if current != self.identity {
            return Err(DescriptorAttestationError::identity_changed(format!(
                "{} descriptor {} was closed, reused or changed identity",
                role.label(),
                self.descriptor
            )));
        }
        current.require_regular(role)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct AttestedSqliteHandles {
    main: RetainedSqliteHandle,
    wal: RetainedSqliteHandle,
    shm: RetainedSqliteHandle,
}

/// Proof that one newly opened SQLite reader retains the expected main
/// database object. Immutable readers intentionally do not open WAL/SHM; the
/// committing writer's full authority remains live and is verified separately.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct AttestedReadOnlyMainHandle {
    main: RetainedSqliteHandle,
}

impl AttestedReadOnlyMainHandle {
    pub(super) fn from_delta(
        before: &ProcessDescriptorSnapshot,
        after: &ProcessDescriptorSnapshot,
        expected_main: FileObjectIdentity,
    ) -> Result<Self, DescriptorAttestationError> {
        let matches = before
            .delta(after)
            .into_iter()
            .filter(|candidate| candidate.identity == expected_main)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Err(DescriptorAttestationError::unavailable(
                "no persistent main descriptor appeared for the retained read-only connection",
            )),
            [candidate] => Ok(Self {
                main: RetainedSqliteHandle {
                    descriptor: candidate.descriptor,
                    identity: candidate.identity,
                },
            }),
            _ => Err(DescriptorAttestationError::ambiguous(format!(
                "{} descriptors match the retained read-only main identity",
                matches.len()
            ))),
        }
    }

    pub(super) fn validate(
        &self,
        expected_main: FileObjectIdentity,
    ) -> Result<(), DescriptorAttestationError> {
        self.main.validate(SqliteObjectRole::Main, expected_main)
    }
}

impl AttestedSqliteHandles {
    #[cfg(test)]
    pub(super) fn from_delta(
        before: &ProcessDescriptorSnapshot,
        after: &ProcessDescriptorSnapshot,
        expected: &PinnedSqliteObjectSet,
    ) -> Result<Self, DescriptorAttestationError> {
        Self::from_candidates(before.delta(after), expected, None)
    }

    /// SQLite opens a WAL-index SHM file only once per process. The first
    /// connection therefore supplies the process-shared retained SHM pin;
    /// later connections still must introduce exact main and WAL descriptors
    /// but may reuse that already-attested SHM descriptor.
    pub(super) fn from_delta_with_shared_shm(
        before: &ProcessDescriptorSnapshot,
        after: &ProcessDescriptorSnapshot,
        expected: &PinnedSqliteObjectSet,
        shared_shm: Option<&File>,
    ) -> Result<Self, DescriptorAttestationError> {
        Self::from_candidates(before.delta(after), expected, shared_shm)
    }

    fn from_candidates(
        candidates: Vec<DescriptorCandidate>,
        expected: &PinnedSqliteObjectSet,
        shared_shm: Option<&File>,
    ) -> Result<Self, DescriptorAttestationError> {
        let main = exact_candidate(SqliteObjectRole::Main, &candidates, expected)?;
        let wal = exact_candidate(SqliteObjectRole::Wal, &candidates, expected)?;
        let shm = exact_or_process_shared_shm_candidate(&candidates, expected, shared_shm)?;
        Ok(Self { main, wal, shm })
    }

    pub(super) fn validate(
        &self,
        expected: &PinnedSqliteObjectSet,
    ) -> Result<(), DescriptorAttestationError> {
        self.main.validate(SqliteObjectRole::Main, expected.main)?;
        self.wal.validate(SqliteObjectRole::Wal, expected.wal)?;
        self.shm.validate(SqliteObjectRole::Shm, expected.shm)
    }

    #[cfg(test)]
    pub(super) fn main(&self) -> &RetainedSqliteHandle {
        &self.main
    }

    #[cfg(test)]
    pub(super) fn wal(&self) -> &RetainedSqliteHandle {
        &self.wal
    }

    #[cfg(test)]
    pub(super) fn shm(&self) -> &RetainedSqliteHandle {
        &self.shm
    }
}

fn exact_or_process_shared_shm_candidate(
    candidates: &[DescriptorCandidate],
    expected: &PinnedSqliteObjectSet,
    shared_shm: Option<&File>,
) -> Result<RetainedSqliteHandle, DescriptorAttestationError> {
    let expected_identity = expected.identity(SqliteObjectRole::Shm);
    let matches = candidates
        .iter()
        .filter(|candidate| candidate.identity == expected_identity)
        .copied()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [candidate] => Ok(RetainedSqliteHandle {
            descriptor: candidate.descriptor,
            identity: candidate.identity,
        }),
        [] => {
            let shared_shm = shared_shm.ok_or_else(|| {
                DescriptorAttestationError::unavailable(
                    "no persistent shm descriptor appeared and no process-shared SHM proof exists",
                )
            })?;
            let actual = FileObjectIdentity::from_file(shared_shm)?;
            if actual != expected_identity {
                return Err(DescriptorAttestationError::identity_changed(
                    "process-shared SHM descriptor no longer matches the pinned SHM object",
                ));
            }
            Ok(RetainedSqliteHandle {
                descriptor: shared_shm.as_raw_fd(),
                identity: actual,
            })
        }
        _ => Err(DescriptorAttestationError::ambiguous(format!(
            "{} descriptors match the pinned shm identity",
            matches.len()
        ))),
    }
}

fn exact_candidate(
    role: SqliteObjectRole,
    candidates: &[DescriptorCandidate],
    expected: &PinnedSqliteObjectSet,
) -> Result<RetainedSqliteHandle, DescriptorAttestationError> {
    let expected_identity = expected.identity(role);
    let matches = candidates
        .iter()
        .filter(|candidate| candidate.identity == expected_identity)
        .copied()
        .collect::<Vec<_>>();

    match matches.as_slice() {
        [] => Err(DescriptorAttestationError::unavailable(format!(
            "no persistent {} descriptor appeared in the process-fd delta",
            role.label()
        ))),
        [candidate] => Ok(RetainedSqliteHandle {
            descriptor: candidate.descriptor,
            identity: candidate.identity,
        }),
        _ => Err(DescriptorAttestationError::ambiguous(format!(
            "{} descriptors match the pinned {} identity",
            matches.len(),
            role.label()
        ))),
    }
}

pub(super) fn validate_wal_journal_mode(actual: &str) -> Result<(), DescriptorAttestationError> {
    if actual.eq_ignore_ascii_case("wal") {
        return Ok(());
    }
    Err(DescriptorAttestationError::journal_mode_unsupported(actual))
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum DescriptorAttestationError {
    Unavailable { detail: String },
    Ambiguous { detail: String },
    IdentityChanged { detail: String },
    JournalModeUnsupported { detail: String },
}

impl DescriptorAttestationError {
    fn unavailable(detail: impl Into<String>) -> Self {
        Self::Unavailable {
            detail: detail.into(),
        }
    }

    fn ambiguous(detail: impl Into<String>) -> Self {
        Self::Ambiguous {
            detail: detail.into(),
        }
    }

    fn identity_changed(detail: impl Into<String>) -> Self {
        Self::IdentityChanged {
            detail: detail.into(),
        }
    }

    fn journal_mode_unsupported(actual: &str) -> Self {
        Self::JournalModeUnsupported {
            detail: format!("expected WAL journal mode, observed {actual:?}"),
        }
    }

    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "descriptor_attestation_unavailable",
            Self::Ambiguous { .. } => "descriptor_attestation_ambiguous",
            Self::IdentityChanged { .. } => "descriptor_identity_changed",
            Self::JournalModeUnsupported { .. } => "descriptor_journal_mode_unsupported",
        }
    }
}

impl fmt::Display for DescriptorAttestationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = match self {
            Self::Unavailable { detail }
            | Self::Ambiguous { detail }
            | Self::IdentityChanged { detail }
            | Self::JournalModeUnsupported { detail } => detail,
        };
        write!(formatter, "{}: {detail}", self.code())
    }
}

impl Error for DescriptorAttestationError {}

fn descriptor_disappeared_during_snapshot(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || error.raw_os_error() == Some(BAD_FILE_DESCRIPTOR_OS_ERROR)
}

fn process_descriptor_root() -> Result<&'static Path, DescriptorAttestationError> {
    #[cfg(target_os = "linux")]
    {
        return Ok(Path::new("/proc/self/fd"));
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        return Ok(Path::new("/dev/fd"));
    }

    #[allow(unreachable_code)]
    Err(DescriptorAttestationError::unavailable(
        "process descriptor enumeration is unsupported on this platform",
    ))
}

fn descriptor_metadata(descriptor: RawFd) -> io::Result<Metadata> {
    if descriptor < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid negative descriptor {descriptor}"),
        ));
    }
    // SAFETY: the caller supplies an fd from this process. ManuallyDrop
    // prevents this temporary File view from closing SQLite's descriptor.
    let file = ManuallyDrop::new(unsafe { File::from_raw_fd(descriptor) });
    file.metadata()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestCodeDescriptorFixture {
        root: PathBuf,
        main: File,
        wal: File,
        shm: File,
    }

    impl TestCodeDescriptorFixture {
        fn new() -> Self {
            let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("TEST_CODE clock must follow the Unix epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "TEST_CODE_sqlite-descriptor-attestation-{}-{timestamp}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&root).expect("create isolated TEST_CODE fixture");

            let main = Self::create_file(&root.join("TEST_CODE_selection.db"));
            let wal = Self::create_file(&root.join("TEST_CODE_selection.db-wal"));
            let shm = Self::create_file(&root.join("TEST_CODE_selection.db-shm"));
            Self {
                root,
                main,
                wal,
                shm,
            }
        }

        fn create_file(path: &Path) -> File {
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)
                .expect("create isolated TEST_CODE descriptor file");
            file.write_all(b"TEST_CODE_descriptor_identity")
                .expect("write TEST_CODE descriptor identity");
            file.sync_all().expect("sync TEST_CODE descriptor identity");
            file
        }

        fn expected(&self) -> PinnedSqliteObjectSet {
            PinnedSqliteObjectSet::from_files(&self.main, &self.wal, &self.shm)
                .expect("capture TEST_CODE pinned object identities")
        }

        fn open_triplet(&self) -> (File, File, File) {
            (
                File::open(self.root.join("TEST_CODE_selection.db"))
                    .expect("open TEST_CODE main duplicate"),
                File::open(self.root.join("TEST_CODE_selection.db-wal"))
                    .expect("open TEST_CODE wal duplicate"),
                File::open(self.root.join("TEST_CODE_selection.db-shm"))
                    .expect("open TEST_CODE shm duplicate"),
            )
        }
    }

    impl Drop for TestCodeDescriptorFixture {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.root) {
                eprintln!(
                    "TEST_CODE cleanup failed for {}: {error}",
                    self.root.display()
                );
            }
        }
    }

    #[test]
    fn descriptor_delta_identifies_exact_new_main_wal_and_shm_handles() {
        let fixture = TestCodeDescriptorFixture::new();
        let expected = fixture.expected();
        let before = ProcessDescriptorSnapshot::capture().expect("capture before snapshot");
        let opened = fixture.open_triplet();
        let after = ProcessDescriptorSnapshot::capture().expect("capture after snapshot");

        let proof = AttestedSqliteHandles::from_delta(&before, &after, &expected)
            .expect("attest exact TEST_CODE descriptor delta");

        assert_eq!(proof.main().identity(), expected.main);
        assert_eq!(proof.wal().identity(), expected.wal);
        assert_eq!(proof.shm().identity(), expected.shm);
        assert_ne!(proof.main().descriptor(), fixture.main.as_raw_fd());
        assert_ne!(proof.wal().descriptor(), fixture.wal.as_raw_fd());
        assert_ne!(proof.shm().descriptor(), fixture.shm.as_raw_fd());
        assert_ne!(proof.main().descriptor(), proof.wal().descriptor());
        assert_ne!(proof.main().descriptor(), proof.shm().descriptor());
        assert_ne!(proof.wal().descriptor(), proof.shm().descriptor());
        proof
            .validate(&expected)
            .expect("persistent TEST_CODE descriptors remain valid");
        drop(opened);
    }

    #[test]
    fn missing_descriptor_delta_fails_closed() {
        let fixture = TestCodeDescriptorFixture::new();
        let expected = fixture.expected();
        let before = ProcessDescriptorSnapshot::capture().expect("capture before snapshot");
        let after = ProcessDescriptorSnapshot::capture().expect("capture after snapshot");

        let error = AttestedSqliteHandles::from_delta(&before, &after, &expected)
            .expect_err("missing SQLite descriptors must fail closed");

        assert_eq!(error.code(), "descriptor_attestation_unavailable");
    }

    #[test]
    fn ambiguous_descriptor_delta_fails_closed() {
        let fixture = TestCodeDescriptorFixture::new();
        let expected = fixture.expected();
        let before = ProcessDescriptorSnapshot::capture().expect("capture before snapshot");
        let first = fixture.open_triplet();
        let duplicate_main = File::open(fixture.root.join("TEST_CODE_selection.db"))
            .expect("open ambiguous TEST_CODE main duplicate");
        let after = ProcessDescriptorSnapshot::capture().expect("capture after snapshot");

        let error = AttestedSqliteHandles::from_delta(&before, &after, &expected)
            .expect_err("ambiguous SQLite descriptors must fail closed");

        assert_eq!(error.code(), "descriptor_attestation_ambiguous");
        drop(duplicate_main);
        drop(first);
    }

    #[test]
    fn closed_attested_descriptor_reports_identity_changed() {
        let fixture = TestCodeDescriptorFixture::new();
        let expected = fixture.expected();
        let before = ProcessDescriptorSnapshot::capture().expect("capture before snapshot");
        let opened = fixture.open_triplet();
        let after = ProcessDescriptorSnapshot::capture().expect("capture after snapshot");
        let proof = AttestedSqliteHandles::from_delta(&before, &after, &expected)
            .expect("attest exact TEST_CODE descriptor delta");
        drop(opened);

        let error = proof
            .validate(&expected)
            .expect_err("closed SQLite descriptor must fail validation");

        assert_eq!(error.code(), "descriptor_identity_changed");
    }

    #[test]
    fn journal_mode_error_is_typed() {
        let error =
            validate_wal_journal_mode("delete").expect_err("DELETE mode must not be authorized");
        assert_eq!(error.code(), "descriptor_journal_mode_unsupported");
        validate_wal_journal_mode("WAL").expect("WAL mode is supported");
    }

    #[test]
    fn file_identity_records_device_inode_mode_and_type() {
        let fixture = TestCodeDescriptorFixture::new();
        let identity =
            FileObjectIdentity::from_file(&fixture.main).expect("inspect TEST_CODE main file");

        assert!(identity.device() > 0);
        assert!(identity.inode() > 0);
        assert_ne!(identity.mode(), 0);
        assert_eq!(identity.object_type(), FileObjectType::RegularFile);
        assert!(fixture.main.as_raw_fd() >= 0);
    }
}
