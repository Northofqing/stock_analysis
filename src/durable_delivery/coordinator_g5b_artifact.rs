//! Immutable fixed-role snapshots beneath the already pinned date namespace.

use super::super::{geteuid, openat, PIN_O_CLOEXEC, PIN_O_CREAT, PIN_O_NOFOLLOW, PIN_O_NONBLOCK};
use super::{
    io_error, mismatch, sha256_hex, G5bDaySession, PreparedG5bArtifact, Result, MAX_ARTIFACT_BYTES,
};
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::fs::{File, Metadata, Permissions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "linux")]
const EXCL: i32 = 0x80;
#[cfg(target_os = "macos")]
const EXCL: i32 = 0x800;

unsafe extern "C" {
    fn unlinkat(dirfd: i32, name: *const std::ffi::c_char, flags: i32) -> i32;
}
#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn renameat2(
        oldfd: i32,
        oldname: *const std::ffi::c_char,
        newfd: i32,
        newname: *const std::ffi::c_char,
        flags: u32,
    ) -> i32;
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn renameatx_np(
        oldfd: i32,
        oldname: *const std::ffi::c_char,
        newfd: i32,
        newname: *const std::ffi::c_char,
        flags: u32,
    ) -> i32;
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn publish_no_replace(parent: &File, oldname: &CString, newname: &CString) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    let result = unsafe {
        renameat2(
            parent.as_raw_fd(),
            oldname.as_ptr(),
            parent.as_raw_fd(),
            newname.as_ptr(),
            1,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe {
        renameatx_np(
            parent.as_raw_fd(),
            oldname.as_ptr(),
            parent.as_raw_fd(),
            newname.as_ptr(),
            4,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn publish_no_replace(_: &File, _: &CString, _: &CString) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace publication is unavailable on this platform",
    ))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileWitness {
    pub(super) filename: String,
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) byte_length: u64,
    pub(super) sha256: String,
}

pub(super) fn filename(intent: &PreparedG5bArtifact) -> String {
    format!(
        "{}.{}.{}.g5b-{}.v2",
        intent.material.intent.business_date.format("%Y%m%d"),
        intent.material.intent.cohort_identity,
        intent.logical_intent,
        intent.material.intent.role.as_str().to_ascii_lowercase()
    )
}

pub(super) fn inspect_bytes(
    session: &G5bDaySession<'_>,
    name: &str,
    bytes: &[u8],
) -> Result<FileWitness> {
    session.validate()?;
    let value = read_inner(
        session.fence.namespace_file().map_err(io_error)?,
        name,
        bytes,
        false,
    )?;
    session.validate()?;
    Ok(value)
}

fn open(parent: &File, name: &str, flags: i32) -> std::io::Result<File> {
    if name.contains('/') || name == "." || name == ".." {
        return Err(std::io::Error::other("invalid fixed artifact leaf"));
    }
    let name = CString::new(name).map_err(std::io::Error::other)?;
    let fd = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | PIN_O_NOFOLLOW | PIN_O_NONBLOCK | PIN_O_CLOEXEC,
            0o600_u32,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn require_regular(meta: &Metadata, links: u64) -> Result<()> {
    if !meta.is_file()
        || meta.nlink() != links
        || meta.uid() != unsafe { geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.len() > MAX_ARTIFACT_BYTES as u64
    {
        return Err(mismatch("artifact is not a private linked regular file"));
    }
    Ok(())
}
fn same_metadata(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
}
fn read(parent: &File, name: &str, desired: &[u8]) -> Result<FileWitness> {
    read_inner(parent, name, desired, true)
}
fn read_inner(parent: &File, name: &str, desired: &[u8], private: bool) -> Result<FileWitness> {
    let mut file = open(parent, name, 0).map_err(io_error)?;
    let before = file.metadata().map_err(io_error)?;
    require_readable(&before, private)?;
    if before.len() != desired.len() as u64 {
        return Err(mismatch("artifact length differs from saved intent"));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_ARTIFACT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    let after = file.metadata().map_err(io_error)?;
    require_readable(&after, private)?;
    if !same_metadata(&before, &after) || bytes != desired {
        return Err(mismatch("artifact bytes or metadata changed"));
    }
    file.sync_all().map_err(io_error)?;
    let reopened = open(parent, name, 0)
        .map_err(io_error)?
        .metadata()
        .map_err(io_error)?;
    require_readable(&reopened, private)?;
    if !same_metadata(&after, &reopened) {
        return Err(mismatch("artifact leaf identity changed"));
    }
    Ok(FileWitness {
        filename: name.to_owned(),
        device: before.dev(),
        inode: before.ino(),
        byte_length: before.len(),
        sha256: sha256_hex(&bytes),
    })
}

fn require_readable(meta: &Metadata, private: bool) -> Result<()> {
    if private {
        return require_regular(meta, 1);
    }
    // Existing AlertInputHead bytes are public alert facts, commonly 0644.
    // Preserve that reader contract; never allow group/other write or aliases.
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != unsafe { geteuid() }
        || meta.mode() & 0o022 != 0
        || meta.len() > MAX_ARTIFACT_BYTES as u64
    {
        return Err(mismatch(
            "input head witness is not a stable owned regular file",
        ));
    }
    Ok(())
}

struct OwnedTemp<'a> {
    parent: &'a File,
    name: CString,
    device: u64,
    inode: u64,
    removed: bool,
}
impl OwnedTemp<'_> {
    fn remove(&mut self) -> Result<()> {
        if self.removed {
            return Ok(());
        }
        let file = open(
            self.parent,
            self.name.to_str().map_err(|_| mismatch("nonutf temp"))?,
            0,
        )
        .map_err(io_error)?;
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_file() || metadata.dev() != self.device || metadata.ino() != self.inode {
            return Err(mismatch("foreign temporary inode must remain untouched"));
        }
        if unsafe { unlinkat(self.parent.as_raw_fd(), self.name.as_ptr(), 0) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        self.removed = true;
        Ok(())
    }
}
impl Drop for OwnedTemp<'_> {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

pub(super) fn inspect(
    session: &G5bDaySession<'_>,
    intent: &PreparedG5bArtifact,
) -> Result<FileWitness> {
    session.validate()?;
    let parent = session.fence.namespace_file().map_err(io_error)?;
    let witness = read(parent, &filename(intent), &intent.desired_bytes)?;
    session.validate()?;
    Ok(witness)
}

pub(super) fn publish(
    session: &G5bDaySession<'_>,
    intent: &PreparedG5bArtifact,
) -> Result<FileWitness> {
    session.validate()?;
    let parent = session.fence.namespace_file().map_err(io_error)?;
    let target = filename(intent);
    match open(parent, &target, 0) {
        Ok(_) => {
            let witness = read(parent, &target, &intent.desired_bytes)?;
            parent.sync_all().map_err(io_error)?;
            session.validate()?;
            if read(parent, &target, &intent.desired_bytes)? != witness {
                return Err(mismatch("existing artifact changed before return"));
            }
            return Ok(witness);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let temporary = format!(
        ".g5b-v2-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let mut file = open(parent, &temporary, 1 | PIN_O_CREAT | EXCL).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    require_regular(&metadata, 1)?;
    let mut owned = OwnedTemp {
        parent,
        name: CString::new(temporary.clone()).map_err(|_| mismatch("invalid temp"))?,
        device: metadata.dev(),
        inode: metadata.ino(),
        removed: false,
    };
    file.write_all(&intent.desired_bytes).map_err(io_error)?;
    file.set_permissions(Permissions::from_mode(0o400))
        .map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    let temporary_witness = read(parent, &temporary, &intent.desired_bytes)?;
    if (temporary_witness.device, temporary_witness.inode) != (owned.device, owned.inode) {
        return Err(mismatch("temporary publication identity changed"));
    }
    session.validate()?;
    let target_c = CString::new(target.clone()).map_err(|_| mismatch("invalid target"))?;
    if let Err(error) = publish_no_replace(parent, &owned.name, &target_c) {
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(io_error(error));
        }
        owned.remove()?;
        let witness = read(parent, &target, &intent.desired_bytes)?;
        parent.sync_all().map_err(io_error)?;
        session.validate()?;
        if read(parent, &target, &intent.desired_bytes)? != witness {
            return Err(mismatch("concurrent artifact changed before return"));
        }
        return Ok(witness);
    }
    // Atomic no-replace rename has no two-link interval. A crash leaves either
    // an unpublished private temp or the exact single-link saved artifact.
    owned.removed = true;
    let linked = open(parent, &target, 0)
        .map_err(io_error)?
        .metadata()
        .map_err(io_error)?;
    require_regular(&linked, 1)?;
    if (linked.dev(), linked.ino()) != (owned.device, owned.inode) {
        return Err(mismatch("published artifact is not owned temporary"));
    }
    parent.sync_all().map_err(io_error)?;
    session.validate()?;
    let witness = read(parent, &target, &intent.desired_bytes)?;
    if (witness.device, witness.inode) != (owned.device, owned.inode) {
        return Err(mismatch("published artifact changed before return"));
    }
    session.validate()?;
    Ok(witness)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn g5b_cohort_b_atomic_no_replace_has_single_link_and_preserves_competing_target() {
        let root = tempfile::tempdir().unwrap();
        let parent = File::open(root.path()).unwrap();
        let source = CString::new("TEST_CODE_saved_temp").unwrap();
        let target = CString::new("TEST_CODE_fixed_target").unwrap();
        std::fs::write(root.path().join(source.to_str().unwrap()), b"saved intent").unwrap();
        let original = root
            .path()
            .join(source.to_str().unwrap())
            .metadata()
            .unwrap();
        publish_no_replace(&parent, &source, &target).unwrap();
        assert!(!root.path().join(source.to_str().unwrap()).exists());
        let published = root
            .path()
            .join(target.to_str().unwrap())
            .metadata()
            .unwrap();
        assert_eq!(
            (published.dev(), published.ino()),
            (original.dev(), original.ino())
        );
        assert_eq!(published.nlink(), 1);

        std::fs::write(
            root.path().join(source.to_str().unwrap()),
            b"conflicting intent",
        )
        .unwrap();
        let error = publish_no_replace(&parent, &source, &target).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read(root.path().join(target.to_str().unwrap())).unwrap(),
            b"saved intent"
        );
        assert_eq!(
            std::fs::read(root.path().join(source.to_str().unwrap())).unwrap(),
            b"conflicting intent"
        );
        let unchanged = root
            .path()
            .join(target.to_str().unwrap())
            .metadata()
            .unwrap();
        assert_eq!(
            (unchanged.dev(), unchanged.ino(), unchanged.nlink()),
            (published.dev(), published.ino(), 1)
        );
    }
}
