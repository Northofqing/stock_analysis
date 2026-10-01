//! Byte-preserving observations, never an admission or a restored live capture.

use super::external_historical_bars::{
    hash_capture_parts_v1, GatewayObservedHistoricalWindowCapture,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const STORE_MATERIAL: &str = "stored-observed-historical-evidence-v1";
const CAPTURE_MATERIAL: &[u8] = b"gateway-observed-external-historical-window-v1";
const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_PART_BYTES: usize = 32 * 1024 * 1024;
const PART_NAMES: [&str; 16] = [
    "capture_domain",
    "request_calendar",
    "connection_identity",
    "health_wire",
    "health_parsed",
    "server_build_identity",
    "capabilities_wire",
    "capabilities_parsed",
    "historical_capability",
    "request_id_correlation",
    "issued_request_wire",
    "observed_request_wire",
    "query_wire_evidence",
    "raw_status_and_trailer",
    "typed_query_outcome",
    "request_binding_outcome",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedArtifactRef {
    capture_sha256: String,
    file_sha256: String,
    byte_length: u64,
}

impl ObservedArtifactRef {
    pub(crate) fn capture_sha256(&self) -> &str {
        &self.capture_sha256
    }

    pub(crate) fn file_sha256(&self) -> &str {
        &self.file_sha256
    }

    pub(crate) fn byte_length(&self) -> u64 {
        self.byte_length
    }

    pub(crate) fn filename(&self) -> String {
        format!("{}.observed.json", self.capture_sha256)
    }
}

/// Checked opaque bytes. Intentionally no Deserialize, session constructor,
/// Gateway capture conversion, projection constructor or admission API.
#[derive(Debug)]
pub(crate) struct StoredObservedEvidence {
    artifact: ObservedArtifactRef,
    parts: [Vec<u8>; 16],
}

impl StoredObservedEvidence {
    pub(crate) fn artifact(&self) -> &ObservedArtifactRef {
        &self.artifact
    }

    pub(crate) fn raw_part(&self, name: &str) -> Option<&[u8]> {
        PART_NAMES
            .iter()
            .position(|candidate| *candidate == name)
            .map(|index| self.parts[index].as_slice())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Publication {
    Published,
    ExistingExact,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceFile {
    material: String,
    version: u32,
    scope: String,
    capture_sha256: String,
    parts: Vec<EvidencePart>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidencePart {
    name: String,
    byte_length: u64,
    sha256: String,
    bytes_hex: String,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn encode_parts(parts: &[Vec<u8>; 16], capture_sha256: &str) -> Result<Vec<u8>> {
    ensure!(is_digest(capture_sha256), "invalid capture hash");
    ensure!(parts[0] == CAPTURE_MATERIAL, "invalid capture domain");
    ensure!(
        hash_capture_parts_v1(parts) == capture_sha256,
        "capture hash mismatch"
    );
    // Bound serialization before building the hex DTO. Fixed names/hashes and
    // JSON punctuation fit within 16 KiB; binary data expands exactly twice.
    let raw_length = parts.iter().try_fold(0usize, |total, bytes| {
        ensure!(
            bytes.len() <= MAX_PART_BYTES,
            "observed part exceeds format limit"
        );
        total
            .checked_add(bytes.len())
            .context("observed evidence size overflow")
    })?;
    ensure!(
        raw_length <= (MAX_FILE_BYTES - 16 * 1024) / 2,
        "observed evidence exceeds format limit"
    );
    let file = EvidenceFile {
        material: STORE_MATERIAL.to_owned(),
        version: 1,
        scope: "ObservedOnly".to_owned(),
        capture_sha256: capture_sha256.to_owned(),
        parts: PART_NAMES
            .iter()
            .zip(parts)
            .map(|(name, bytes)| EvidencePart {
                name: (*name).to_owned(),
                byte_length: bytes.len() as u64,
                sha256: digest(bytes),
                bytes_hex: hex::encode(bytes),
            })
            .collect(),
    };
    let mut bytes = serde_json::to_vec(&file).context("encode observed evidence")?;
    bytes.push(b'\n');
    ensure!(
        bytes.len() <= MAX_FILE_BYTES,
        "observed evidence exceeds format limit"
    );
    Ok(bytes)
}

fn reference(capture_sha256: &str, bytes: &[u8]) -> ObservedArtifactRef {
    ObservedArtifactRef {
        capture_sha256: capture_sha256.to_owned(),
        file_sha256: digest(bytes),
        byte_length: bytes.len() as u64,
    }
}

fn decode_checked(bytes: &[u8], artifact: &ObservedArtifactRef) -> Result<StoredObservedEvidence> {
    ensure!(
        is_digest(&artifact.capture_sha256) && is_digest(&artifact.file_sha256),
        "invalid artifact reference"
    );
    ensure!(
        artifact.byte_length <= MAX_FILE_BYTES as u64 && bytes.len() as u64 == artifact.byte_length,
        "observed file length mismatch"
    );
    ensure!(
        digest(bytes) == artifact.file_sha256,
        "observed file hash mismatch"
    );
    let file: EvidenceFile =
        serde_json::from_slice(bytes).context("invalid observed evidence format")?;
    ensure!(
        file.material == STORE_MATERIAL && file.version == 1 && file.scope == "ObservedOnly",
        "unsupported observed evidence format"
    );
    ensure!(
        file.capture_sha256 == artifact.capture_sha256 && file.parts.len() == 16,
        "observed capture identity mismatch"
    );
    let mut parts: [Vec<u8>; 16] = std::array::from_fn(|_| Vec::new());
    for (index, part) in file.parts.into_iter().enumerate() {
        ensure!(
            part.name == PART_NAMES[index],
            "observed part order mismatch"
        );
        ensure!(
            part.byte_length <= MAX_PART_BYTES as u64
                && part.bytes_hex.len() as u64 == part.byte_length * 2,
            "observed part length mismatch"
        );
        ensure!(
            part.bytes_hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "noncanonical observed part encoding"
        );
        ensure!(is_digest(&part.sha256), "invalid observed part hash");
        let raw = hex::decode(&part.bytes_hex).context("invalid observed part encoding")?;
        ensure!(digest(&raw) == part.sha256, "observed part hash mismatch");
        parts[index] = raw;
    }
    ensure!(
        encode_parts(&parts, &artifact.capture_sha256)? == bytes,
        "noncanonical observed file bytes"
    );
    Ok(StoredObservedEvidence {
        artifact: artifact.clone(),
        parts,
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod anchored {
    use super::*;
    use fs2::FileExt as _;
    use std::cell::Cell;
    use std::ffi::{CString, OsStr, OsString};
    use std::fs::{File, Metadata, OpenOptions, Permissions};
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
    use std::path::{Component, Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    const O_RDONLY: i32 = 0;
    const O_WRONLY: i32 = 1;
    #[cfg(target_os = "linux")]
    const O_CREAT: i32 = 0x40;
    #[cfg(target_os = "macos")]
    const O_CREAT: i32 = 0x200;
    #[cfg(target_os = "linux")]
    const O_EXCL: i32 = 0x80;
    #[cfg(target_os = "macos")]
    const O_EXCL: i32 = 0x800;
    #[cfg(target_os = "linux")]
    const O_NOFOLLOW: i32 = 0x20000;
    #[cfg(target_os = "macos")]
    const O_NOFOLLOW: i32 = 0x100;
    #[cfg(target_os = "linux")]
    const O_NONBLOCK: i32 = 0x800;
    #[cfg(target_os = "macos")]
    const O_NONBLOCK: i32 = 4;
    #[cfg(target_os = "linux")]
    const O_CLOEXEC: i32 = 0x80000;
    #[cfg(target_os = "macos")]
    const O_CLOEXEC: i32 = 0x1000000;

    unsafe extern "C" {
        fn openat(dirfd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
        fn linkat(
            oldfd: i32,
            oldpath: *const std::ffi::c_char,
            newfd: i32,
            newpath: *const std::ffi::c_char,
            flags: i32,
        ) -> i32;
        fn unlinkat(dirfd: i32, path: *const std::ffi::c_char, flags: i32) -> i32;
        fn geteuid() -> u32;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Identity {
        device: u64,
        inode: u64,
    }

    fn identity(metadata: &Metadata) -> Identity {
        Identity {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    struct Directory {
        file: File,
        name: Option<OsString>,
        identity: Identity,
    }

    /// Advisory locking coordinates this API's publishers/readers; a retained
    /// inode and full path rechecks detect namespace drift independently.
    pub(crate) struct HistoricalObservedStore {
        chain: Vec<Directory>,
        gate: Mutex<()>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Phase {
        BeforePublish,
        AfterPublish,
        BeforeReturn,
        AfterRead,
    }

    #[cfg(test)]
    type Hook = (Phase, Box<dyn FnOnce()>);
    #[cfg(test)]
    thread_local! { static HOOK: std::cell::RefCell<Option<Hook>> = const { std::cell::RefCell::new(None) }; }

    fn phase(at: Phase) {
        #[cfg(test)]
        HOOK.with(|hook| {
            let callback = {
                let mut slot = hook.borrow_mut();
                if slot.as_ref().is_some_and(|(expected, _)| *expected == at) {
                    slot.take().map(|(_, callback)| callback)
                } else {
                    None
                }
            };
            if let Some(callback) = callback {
                callback();
            }
        });
        #[cfg(not(test))]
        let _ = at;
    }

    struct Unlock<'a>(&'a File);
    impl Drop for Unlock<'_> {
        fn drop(&mut self) {
            let _ = fs2::FileExt::unlock(self.0);
        }
    }

    struct Temporary<'a> {
        parent: &'a File,
        name: CString,
        identity: Identity,
        removed: Cell<bool>,
    }
    impl Temporary<'_> {
        fn remove_owned(&self) -> Result<()> {
            if self.removed.get() {
                return Ok(());
            }
            let file = open_leaf(
                self.parent,
                OsStr::from_bytes(self.name.as_bytes()),
                O_RDONLY,
            )?;
            let metadata = file.metadata()?;
            ensure!(
                metadata.is_file() && identity(&metadata) == self.identity,
                "temporary observed inode changed"
            );
            // Namespace is private to the effective user. No unknown inode is
            // swept or adopted; a failed owned-inode check leaves it intact.
            let rc = unsafe { unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0) };
            ensure!(rc == 0, "cannot unlink owned observed temporary file");
            self.removed.set(true);
            Ok(())
        }
    }
    impl Drop for Temporary<'_> {
        fn drop(&mut self) {
            let _ = self.remove_owned();
        }
    }

    impl HistoricalObservedStore {
        pub(crate) fn open_existing(root: &Path, forbidden_roots: &[PathBuf]) -> Result<Self> {
            let names = normal_absolute_components(root)?;
            let root_file = OpenOptions::new()
                .read(true)
                .custom_flags(O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
                .open("/")?;
            let root_metadata = root_file.metadata()?;
            require_directory(&root_metadata, false)?;
            let mut chain = vec![Directory {
                file: root_file,
                name: None,
                identity: identity(&root_metadata),
            }];
            for name in names {
                let file = open_leaf(&chain.last().unwrap().file, &name, O_RDONLY)?;
                let metadata = file.metadata()?;
                require_directory(&metadata, false)?;
                chain.push(Directory {
                    file,
                    name: Some(name),
                    identity: identity(&metadata),
                });
            }
            require_directory(&chain.last().unwrap().file.metadata()?, true)?;
            // Isolation is mandatory even for another crate-private caller.
            // The runtime is documented in CLAUDE and distinct from a default
            // development build root; neither namespace may host this store.
            let mut forbidden_roots = forbidden_roots.to_vec();
            forbidden_roots.push(crate::production_root::production_root().to_path_buf());
            forbidden_roots.push(PathBuf::from(
                "/Users/zhangzhen/.local/share/stock-analysis-runtime",
            ));
            for forbidden in &forbidden_roots {
                let absolute = if forbidden.is_absolute() {
                    forbidden.clone()
                } else {
                    std::env::current_dir()?.join(forbidden)
                };
                ensure!(
                    !root.starts_with(&absolute) && !absolute.starts_with(root),
                    "observed output overlaps a forbidden namespace"
                );
                if let Ok(canonical) = std::fs::canonicalize(&absolute) {
                    ensure!(
                        !root.starts_with(&canonical) && !canonical.starts_with(root),
                        "observed output aliases a forbidden namespace"
                    );
                    let forbidden_identity = identity(&std::fs::metadata(&canonical)?);
                    ensure!(
                        !chain.iter().any(|part| part.identity == forbidden_identity),
                        "observed output has a forbidden directory identity"
                    );
                }
            }
            let store = Self {
                chain,
                gate: Mutex::new(()),
            };
            store.validate_chain()?;
            Ok(store)
        }

        fn parent(&self) -> &File {
            &self.chain.last().unwrap().file
        }

        fn validate_chain(&self) -> Result<()> {
            let mut reopened = OpenOptions::new()
                .read(true)
                .custom_flags(O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
                .open("/")?;
            for (index, retained) in self.chain.iter().enumerate() {
                if index != 0 {
                    reopened = open_leaf(&reopened, retained.name.as_ref().unwrap(), O_RDONLY)?;
                }
                let pinned = retained.file.metadata()?;
                let current = reopened.metadata()?;
                require_directory(&pinned, index + 1 == self.chain.len())?;
                require_directory(&current, index + 1 == self.chain.len())?;
                ensure!(
                    identity(&pinned) == retained.identity
                        && identity(&current) == retained.identity,
                    "observed directory namespace changed"
                );
            }
            Ok(())
        }

        pub(crate) fn persist(
            &self,
            capture: &GatewayObservedHistoricalWindowCapture,
        ) -> Result<(ObservedArtifactRef, Publication)> {
            let parts = capture
                .hash_parts_v1()
                .map_err(|_| anyhow::anyhow!("cannot encode observed capture parts"))?;
            let bytes = encode_parts(&parts, capture.capture_hash())?;
            self.publish_bytes(&bytes, &reference(capture.capture_hash(), &bytes))
        }

        fn publish_bytes(
            &self,
            bytes: &[u8],
            artifact: &ObservedArtifactRef,
        ) -> Result<(ObservedArtifactRef, Publication)> {
            decode_checked(bytes, artifact)?;
            let _gate = self
                .gate
                .lock()
                .map_err(|_| anyhow::anyhow!("observed store mutex poisoned"))?;
            self.validate_chain()?;
            self.parent().lock_exclusive()?;
            let _unlock = Unlock(self.parent());
            self.validate_chain()?;
            let leaf = artifact.filename();
            match open_leaf(self.parent(), OsStr::new(&leaf), O_RDONLY) {
                Ok(file) => {
                    let (existing, identity) = read_regular(file, artifact)?;
                    ensure!(existing == bytes, "existing observed bytes differ");
                    self.finish_checked(artifact, bytes, identity)?;
                    return Ok((artifact.clone(), Publication::ExistingExact));
                }
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
                Err(error) => return Err(error),
            }
            static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let temporary_name = format!(
                ".observed-{}-{}-{}.tmp",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                artifact.capture_sha256
            );
            let mut file = open_leaf(
                self.parent(),
                OsStr::new(&temporary_name),
                O_WRONLY | O_CREAT | O_EXCL,
            )?;
            let temporary = Temporary {
                parent: self.parent(),
                name: component(OsStr::new(&temporary_name))?,
                identity: identity(&file.metadata()?),
                removed: Cell::new(false),
            };
            require_regular(&file.metadata()?)?;
            file.write_all(bytes).context("write observed evidence")?;
            file.set_permissions(Permissions::from_mode(0o400))?;
            file.sync_all().context("fsync observed evidence")?;
            ensure!(
                identity(&file.metadata()?) == temporary.identity,
                "observed temporary identity changed"
            );
            self.validate_chain()?;
            let reopened = open_leaf(self.parent(), OsStr::new(&temporary_name), O_RDONLY)?;
            require_regular(&reopened.metadata()?)?;
            ensure!(
                identity(&reopened.metadata()?) == temporary.identity,
                "observed temporary namespace changed"
            );
            phase(Phase::BeforePublish);
            self.validate_chain()?;
            let (temporary_bytes, temporary_identity) = read_regular(
                open_leaf(self.parent(), OsStr::new(&temporary_name), O_RDONLY)?,
                artifact,
            )?;
            ensure!(
                temporary_identity == temporary.identity && temporary_bytes == bytes,
                "observed temporary bytes or inode changed before publication"
            );
            let target = component(OsStr::new(&leaf))?;
            let rc = unsafe {
                linkat(
                    self.parent().as_raw_fd(),
                    temporary.name.as_ptr(),
                    self.parent().as_raw_fd(),
                    target.as_ptr(),
                    0,
                )
            };
            if rc != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(error.into());
                }
                let (existing, identity) = read_regular(
                    open_leaf(self.parent(), OsStr::new(&leaf), O_RDONLY)?,
                    artifact,
                )?;
                ensure!(existing == bytes, "concurrent observed bytes differ");
                temporary.remove_owned()?;
                self.finish_checked(artifact, bytes, identity)?;
                return Ok((artifact.clone(), Publication::ExistingExact));
            }
            // Two names refer to exactly our owned file until temporary unlink.
            let final_file = open_leaf(self.parent(), OsStr::new(&leaf), O_RDONLY)?;
            let metadata = final_file.metadata()?;
            ensure!(
                metadata.is_file()
                    && metadata.nlink() == 2
                    && identity(&metadata) == temporary.identity,
                "published observed inode changed"
            );
            phase(Phase::AfterPublish);
            temporary.remove_owned()?;
            self.finish_checked(artifact, bytes, temporary.identity)?;
            Ok((artifact.clone(), Publication::Published))
        }

        fn finish_checked(
            &self,
            artifact: &ObservedArtifactRef,
            expected_bytes: &[u8],
            expected_identity: Identity,
        ) -> Result<()> {
            phase(Phase::BeforeReturn);
            self.validate_chain()?;
            self.parent()
                .sync_all()
                .context("fsync observed directory")?;
            let (bytes, identity) = read_regular(
                open_leaf(self.parent(), OsStr::new(&artifact.filename()), O_RDONLY)?,
                artifact,
            )?;
            ensure!(
                bytes == expected_bytes && identity == expected_identity,
                "published observed bytes or inode changed"
            );
            self.validate_chain()?;
            self.require_leaf_identity(artifact, expected_identity)?;
            Ok(())
        }

        fn require_leaf_identity(
            &self,
            artifact: &ObservedArtifactRef,
            expected: Identity,
        ) -> Result<()> {
            let metadata =
                open_leaf(self.parent(), OsStr::new(&artifact.filename()), O_RDONLY)?.metadata()?;
            require_regular(&metadata)?;
            ensure!(
                identity(&metadata) == expected && metadata.len() == artifact.byte_length,
                "observed leaf namespace changed"
            );
            Ok(())
        }

        pub(crate) fn read_checked(
            &self,
            artifact: &ObservedArtifactRef,
        ) -> Result<StoredObservedEvidence> {
            ensure!(
                is_digest(&artifact.capture_sha256)
                    && is_digest(&artifact.file_sha256)
                    && artifact.byte_length <= MAX_FILE_BYTES as u64,
                "invalid observed artifact reference"
            );
            let _gate = self
                .gate
                .lock()
                .map_err(|_| anyhow::anyhow!("observed store mutex poisoned"))?;
            self.validate_chain()?;
            self.parent().lock_exclusive()?;
            let _unlock = Unlock(self.parent());
            self.validate_chain()?;
            let (bytes, expected_identity) = read_regular(
                open_leaf(self.parent(), OsStr::new(&artifact.filename()), O_RDONLY)?,
                artifact,
            )?;
            let recorded = decode_checked(&bytes, artifact)?;
            phase(Phase::AfterRead);
            // Reopen the namespace after parsing, then validate the full chain.
            let (reopened, identity) = read_regular(
                open_leaf(self.parent(), OsStr::new(&artifact.filename()), O_RDONLY)?,
                artifact,
            )?;
            ensure!(
                reopened == bytes && identity == expected_identity,
                "observed leaf replaced while reading"
            );
            self.validate_chain()?;
            self.require_leaf_identity(artifact, expected_identity)?;
            Ok(recorded)
        }
    }

    fn normal_absolute_components(path: &Path) -> Result<Vec<OsString>> {
        let text = path.to_str().context("observed root must be UTF-8")?;
        ensure!(
            path.is_absolute() && text != "/" && !text.chars().any(char::is_control),
            "observed root must be an absolute isolated directory"
        );
        ensure!(
            !text
                .split('/')
                .skip(1)
                .any(|part| matches!(part, "" | "." | "..")),
            "observed root has a forbidden component"
        );
        path.components()
            .filter_map(|part| match part {
                Component::RootDir => None,
                Component::Normal(name) => Some(Ok(name.to_os_string())),
                _ => Some(Err(anyhow::anyhow!(
                    "observed root has a forbidden component"
                ))),
            })
            .collect()
    }

    fn component(name: &OsStr) -> Result<CString> {
        let bytes = name.as_bytes();
        ensure!(
            !bytes.is_empty() && !bytes.contains(&b'/') && bytes != b"." && bytes != b"..",
            "invalid observed leaf"
        );
        CString::new(bytes).context("invalid observed leaf")
    }

    fn open_leaf(parent: &File, name: &OsStr, flags: i32) -> Result<File> {
        let name = component(name)?;
        let fd = unsafe {
            openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                flags | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC,
                0o600_u32,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: openat success returned one newly owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn require_directory(metadata: &Metadata, output_root: bool) -> Result<()> {
        ensure!(
            metadata.is_dir() && metadata.nlink() > 0,
            "observed namespace is not a linked directory"
        );
        let uid = unsafe { geteuid() };
        ensure!(
            metadata.uid() == 0 || metadata.uid() == uid,
            "observed namespace has an untrusted owner"
        );
        if output_root {
            ensure!(
                metadata.uid() == uid && metadata.mode() & 0o077 == 0,
                "observed root must be private to the effective user"
            );
        }
        Ok(())
    }

    fn require_regular(metadata: &Metadata) -> Result<()> {
        ensure!(
            metadata.is_file()
                && metadata.nlink() == 1
                && metadata.uid() == unsafe { geteuid() }
                && metadata.mode() & 0o077 == 0,
            "observed leaf is not a private regular single-link file"
        );
        ensure!(
            metadata.len() <= MAX_FILE_BYTES as u64,
            "observed file exceeds format limit"
        );
        Ok(())
    }

    fn read_regular(mut file: File, artifact: &ObservedArtifactRef) -> Result<(Vec<u8>, Identity)> {
        let before = file.metadata()?;
        require_regular(&before)?;
        ensure!(
            before.len() == artifact.byte_length,
            "observed file length mismatch"
        );
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_FILE_BYTES,
            "observed file exceeds format limit"
        );
        let after = file.metadata()?;
        require_regular(&after)?;
        ensure!(
            identity(&before) == identity(&after)
                && before.len() == after.len()
                && before.mtime() == after.mtime()
                && before.mtime_nsec() == after.mtime_nsec()
                && before.ctime() == after.ctime()
                && before.ctime_nsec() == after.ctime_nsec(),
            "observed inode changed while reading"
        );
        ensure!(
            bytes.len() as u64 == artifact.byte_length && digest(&bytes) == artifact.file_sha256,
            "observed file byte pin mismatch"
        );
        Ok((bytes, identity(&before)))
    }

    #[cfg(test)]
    mod tests {
        include!("historical_observed_store_tests.rs");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) use anchored::HistoricalObservedStore;
