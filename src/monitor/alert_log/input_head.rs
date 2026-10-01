//! Opt-in G5b alert input provenance. Legacy append/read entry points remain unchanged.
//! This verifies one fenced prefix at one instant; it is not a day seal. The other
//! alert and journal writers must join this fence before a seal can use this evidence.

use super::{
    dated_file_for, same_file_state, write_jsonl, AlertLog, AlertRecord, AlertRecordOrigin,
};
use crate::monitor::detector::AlertEvent;
use chrono::NaiveDate;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const HEAD_VERSION: u8 = 1;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "linux")]
const O_NOFOLLOW: i32 = 0x0002_0000;
#[cfg(any(target_os = "macos", target_os = "ios"))]
const O_NOFOLLOW: i32 = 0x0000_0100;
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "macos", target_os = "ios"))
))]
const O_NOFOLLOW: i32 = 0;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn file_identity(_metadata: &Metadata) -> io::Result<FileIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "G5b input head requires filesystem identity",
    ))
}

fn nofollow(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(O_NOFOLLOW);
    }
    options
}

fn head_path(dir: &Path, date: NaiveDate) -> PathBuf {
    dir.join(format!("{}.input-head.v1.json", date.format("%Y%m%d")))
}

fn lock_path(dir: &Path, date: NaiveDate) -> PathBuf {
    dir.join(format!("{}.g5b-day.lock", date.format("%Y%m%d")))
}

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Durable committed JSONL prefix. A matching head does not imply the day is closed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertInputHeadV1 {
    version: u8,
    business_date: NaiveDate,
    generation: u64,
    committed_offset: u64,
    prefix_sha256: String,
    source_identity: Option<FileIdentity>,
}

impl AlertInputHeadV1 {
    pub fn business_date(&self) -> NaiveDate {
        self.business_date
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn committed_offset(&self) -> u64 {
        self.committed_offset
    }

    pub fn prefix_sha256(&self) -> &str {
        &self.prefix_sha256
    }

    fn empty(date: NaiveDate) -> Self {
        Self {
            version: HEAD_VERSION,
            business_date: date,
            generation: 0,
            committed_offset: 0,
            prefix_sha256: hash(&[]),
            source_identity: None,
        }
    }

    fn valid_for(&self, date: NaiveDate) -> bool {
        self.version == HEAD_VERSION
            && self.business_date == date
            && if self.generation == 0 {
                self.committed_offset == 0
                    && self.prefix_sha256 == hash(&[])
                    && self.source_identity.is_none()
            } else {
                self.committed_offset > 0
                    && self.source_identity.is_some()
                    && self.prefix_sha256.len() == 64
            }
    }
}

/// An exact, strictly parsed snapshot under the opt-in date fence.
#[derive(Debug)]
pub struct VerifiedAlertInputPrefix {
    head: AlertInputHeadV1,
    records: Vec<AlertRecord>,
}

impl VerifiedAlertInputPrefix {
    pub fn head(&self) -> &AlertInputHeadV1 {
        &self.head
    }

    pub fn records(&self) -> &[AlertRecord] {
        &self.records
    }
}

#[derive(Debug)]
pub enum AlertInputHeadUnknown {
    AccessDenied,
    MissingDirectory,
    MissingFence,
    MissingHead,
    MissingSource,
    NonRegular,
    ChangedDuringRead,
    InvalidHead,
    UnexpectedSource,
    SourceIdentityMismatch,
    OffsetMismatch {
        committed: u64,
        actual: u64,
    },
    DigestMismatch,
    TruncatedFinalLine,
    MalformedLine {
        line: usize,
        source: serde_json::Error,
    },
    IneligibleRecord {
        line: usize,
    },
    Io(io::Error),
}

impl From<io::Error> for AlertInputHeadUnknown {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn io_unknown(error: AlertInputHeadUnknown) -> io::Error {
    match error {
        AlertInputHeadUnknown::Io(error) => error,
        other => io::Error::new(io::ErrorKind::InvalidData, format!("{other:?}")),
    }
}

fn same_identity(left: &Metadata, right: &Metadata) -> bool {
    match (file_identity(left), file_identity(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

struct DateFence {
    requested_dir: PathBuf,
    dir: PathBuf,
    dir_file: File,
    lock_path: PathBuf,
    lock_file: File,
}

impl DateFence {
    fn acquire(dir: &Path, date: NaiveDate, writer: bool) -> Result<Self, AlertInputHeadUnknown> {
        if writer {
            fs::create_dir_all(dir)?;
        }
        let directory = match fs::symlink_metadata(dir) {
            Ok(metadata) => metadata,
            Err(error) if !writer && error.kind() == io::ErrorKind::NotFound => {
                return Err(AlertInputHeadUnknown::MissingDirectory);
            }
            Err(error) => return Err(error.into()),
        };
        if !directory.file_type().is_dir() {
            return Err(AlertInputHeadUnknown::NonRegular);
        }
        let requested_dir = dir.to_path_buf();
        let dir = fs::canonicalize(dir)?;
        let dir_file = File::open(&dir)?;
        if !same_identity(&dir_file.metadata()?, &fs::symlink_metadata(&dir)?) {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "alert directory changed").into(),
            );
        }
        let lock_path = lock_path(&dir, date);
        match fs::symlink_metadata(&lock_path) {
            Ok(before) if !before.file_type().is_file() => {
                return Err(AlertInputHeadUnknown::NonRegular);
            }
            Err(error) if !writer && error.kind() == io::ErrorKind::NotFound => {
                return Err(AlertInputHeadUnknown::MissingFence);
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let lock_file = if writer {
            nofollow(OpenOptions::new().read(true).write(true).create(true)).open(&lock_path)?
        } else {
            nofollow(OpenOptions::new().read(true)).open(&lock_path)?
        };
        if !lock_file.metadata()?.is_file() {
            return Err(AlertInputHeadUnknown::NonRegular);
        }
        if writer {
            lock_file.lock_exclusive()?;
        } else {
            FileExt::lock_shared(&lock_file)?;
        }
        let guard = Self {
            requested_dir,
            dir,
            dir_file,
            lock_path,
            lock_file,
        };
        guard.ensure_current()?;
        Ok(guard)
    }

    fn ensure_current(&self) -> io::Result<()> {
        if !same_identity(
            &self.dir_file.metadata()?,
            &fs::symlink_metadata(&self.dir)?,
        ) || !same_identity(
            &self.dir_file.metadata()?,
            &fs::symlink_metadata(&self.requested_dir)?,
        ) || !same_identity(
            &self.lock_file.metadata()?,
            &fs::symlink_metadata(&self.lock_path)?,
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "G5b date fence path changed",
            ));
        }
        Ok(())
    }
}

fn read_regular(
    path: &Path,
    missing: AlertInputHeadUnknown,
) -> Result<(Vec<u8>, Metadata), AlertInputHeadUnknown> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(missing),
        Err(error) => return Err(error.into()),
    };
    if !before.file_type().is_file() {
        return Err(AlertInputHeadUnknown::NonRegular);
    }
    let mut file = nofollow(OpenOptions::new().read(true))
        .open(path)
        .map_err(AlertInputHeadUnknown::Io)?;
    let opened = file.metadata()?;
    if !same_file_state(&before, &opened) {
        return Err(AlertInputHeadUnknown::ChangedDuringRead);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let after_path =
        fs::symlink_metadata(path).map_err(|_| AlertInputHeadUnknown::ChangedDuringRead)?;
    if !same_file_state(&before, &file.metadata()?)
        || !same_file_state(&before, &after_path)
        || bytes.len() as u64 != before.len()
    {
        return Err(AlertInputHeadUnknown::ChangedDuringRead);
    }
    Ok((bytes, before))
}

struct ReadHead {
    value: AlertInputHeadV1,
    bytes: Vec<u8>,
    metadata: Metadata,
}

fn read_head(path: &Path, date: NaiveDate) -> Result<ReadHead, AlertInputHeadUnknown> {
    let (bytes, metadata) = read_regular(path, AlertInputHeadUnknown::MissingHead)?;
    let value: AlertInputHeadV1 =
        serde_json::from_slice(&bytes).map_err(|_| AlertInputHeadUnknown::InvalidHead)?;
    if !value.valid_for(date) || head_bytes(&value).map_err(AlertInputHeadUnknown::Io)? != bytes {
        return Err(AlertInputHeadUnknown::InvalidHead);
    }
    Ok(ReadHead {
        value,
        bytes,
        metadata,
    })
}

fn head_bytes(head: &AlertInputHeadV1) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(head).map_err(io::Error::other)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn inspect_locked(
    guard: &DateFence,
    date: NaiveDate,
    origin: AlertRecordOrigin,
) -> Result<(VerifiedAlertInputPrefix, ReadHead, Vec<u8>), AlertInputHeadUnknown> {
    guard.ensure_current()?;
    let head_file = head_path(&guard.dir, date);
    let head = read_head(&head_file, date)?;
    let source_path = dated_file_for(&guard.dir, "jsonl", date);
    let (bytes, source_metadata) = if head.value.generation == 0 {
        match fs::symlink_metadata(&source_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Vec::new(), None),
            Ok(_) => return Err(AlertInputHeadUnknown::UnexpectedSource),
            Err(error) => return Err(error.into()),
        }
    } else {
        let (bytes, metadata) = read_regular(&source_path, AlertInputHeadUnknown::MissingSource)?;
        if head.value.source_identity.as_ref() != Some(&file_identity(&metadata)?) {
            return Err(AlertInputHeadUnknown::SourceIdentityMismatch);
        }
        if bytes.len() as u64 != head.value.committed_offset {
            return Err(AlertInputHeadUnknown::OffsetMismatch {
                committed: head.value.committed_offset,
                actual: bytes.len() as u64,
            });
        }
        if hash(&bytes) != head.value.prefix_sha256 {
            return Err(AlertInputHeadUnknown::DigestMismatch);
        }
        (bytes, Some(metadata))
    };
    if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
        return Err(AlertInputHeadUnknown::TruncatedFinalLine);
    }
    let mut records = Vec::new();
    if !bytes.is_empty() {
        for (index, line) in bytes[..bytes.len() - 1]
            .split(|byte| *byte == b'\n')
            .enumerate()
        {
            let line_number = index + 1;
            let record: AlertRecord = serde_json::from_slice(line).map_err(|source| {
                AlertInputHeadUnknown::MalformedLine {
                    line: line_number,
                    source,
                }
            })?;
            if origin == AlertRecordOrigin::Production && !record.is_production_eligible() {
                return Err(AlertInputHeadUnknown::IneligibleRecord { line: line_number });
            }
            records.push(record);
        }
    }
    if records.len() as u64 != head.value.generation {
        return Err(AlertInputHeadUnknown::InvalidHead);
    }
    let checked_head = read_head(&head_file, date)?;
    if checked_head.bytes != head.bytes || !same_file_state(&head.metadata, &checked_head.metadata)
    {
        return Err(AlertInputHeadUnknown::ChangedDuringRead);
    }
    match source_metadata {
        Some(before) => {
            let after = fs::symlink_metadata(&source_path)
                .map_err(|_| AlertInputHeadUnknown::ChangedDuringRead)?;
            if !same_file_state(&before, &after) {
                return Err(AlertInputHeadUnknown::ChangedDuringRead);
            }
        }
        None => match fs::symlink_metadata(&source_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => return Err(AlertInputHeadUnknown::ChangedDuringRead),
        },
    }
    guard.ensure_current()?;
    Ok((
        VerifiedAlertInputPrefix {
            head: head.value.clone(),
            records,
        },
        head,
        bytes,
    ))
}

fn publish_head(
    guard: &DateFence,
    date: NaiveDate,
    new_head: &AlertInputHeadV1,
    previous: Option<&ReadHead>,
) -> io::Result<()> {
    let path = head_path(&guard.dir, date);
    let bytes = head_bytes(new_head)?;
    let candidate = guard.dir.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name().expect("head filename").to_string_lossy(),
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed),
    ));
    let mut temp = nofollow(OpenOptions::new().write(true).create_new(true)).open(&candidate)?;
    let created = temp.metadata()?;
    let result = (|| {
        temp.write_all(&bytes)?;
        temp.sync_all()?;
        if !same_identity(&created, &fs::symlink_metadata(&candidate)?) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "input head temporary path changed",
            ));
        }
        guard.ensure_current()?;
        match previous {
            Some(expected) => {
                let current = read_head(&path, date).map_err(io_unknown)?;
                if current.bytes != expected.bytes
                    || !same_file_state(&current.metadata, &expected.metadata)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "input head changed before publish",
                    ));
                }
            }
            None => match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "input head appeared before publish",
                    ))
                }
                Err(error) => return Err(error),
            },
        }
        fs::rename(&candidate, &path)?;
        guard.dir_file.sync_all()?;
        let current = read_head(&path, date).map_err(io_unknown)?;
        if current.bytes != bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "published input head differs",
            ));
        }
        guard.ensure_current()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&candidate);
    }
    result
}

fn reject_prehead_source(dir: &Path, date: NaiveDate) -> io::Result<()> {
    match fs::symlink_metadata(dated_file_for(dir, "jsonl", date)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pre-head alert file cannot be adopted",
        )),
        Err(error) => Err(error),
    }
}

impl AlertLog {
    /// Start provenance only for a genuinely new date. A pre-head file, even
    /// if empty, is historical/unknown and is never silently adopted.
    pub fn initialize_date_input_head(&self, date: NaiveDate) -> io::Result<AlertInputHeadV1> {
        self.ensure_io_allowed()?;
        let guard = DateFence::acquire(&self.dir, date, true).map_err(io_unknown)?;
        match inspect_locked(&guard, date, self.origin) {
            Ok((snapshot, _, _)) => Ok(snapshot.head),
            Err(AlertInputHeadUnknown::MissingHead) => {
                reject_prehead_source(&guard.dir, date)?;
                let head = AlertInputHeadV1::empty(date);
                publish_head(&guard, date, &head, None)?;
                inspect_locked(&guard, date, self.origin).map_err(io_unknown)?;
                Ok(head)
            }
            Err(error) => Err(io_unknown(error)),
        }
    }

    /// Opt-in append: D is captured by the caller once before lock/path choice.
    /// This does not alter the legacy append_jsonl or append_batch behavior.
    pub fn append_date_jsonl_fenced(
        &self,
        date: NaiveDate,
        event: &AlertEvent,
    ) -> io::Result<AlertInputHeadV1> {
        self.ensure_io_allowed()?;
        let record = AlertRecord::from_event(event, self.origin);
        if self.origin == AlertRecordOrigin::Production && !record.is_production_eligible() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ineligible production alert",
            ));
        }
        let mut line = Vec::new();
        write_jsonl(&mut line, &record)?;
        let guard = DateFence::acquire(&self.dir, date, true).map_err(io_unknown)?;
        let (snapshot, old_head, old_bytes) = match inspect_locked(&guard, date, self.origin) {
            Ok(state) => state,
            Err(AlertInputHeadUnknown::MissingHead) => {
                reject_prehead_source(&guard.dir, date)?;
                publish_head(&guard, date, &AlertInputHeadV1::empty(date), None)?;
                inspect_locked(&guard, date, self.origin).map_err(io_unknown)?
            }
            Err(error) => return Err(io_unknown(error)),
        };
        let path = dated_file_for(&guard.dir, "jsonl", date);
        let mut file = if snapshot.head.generation == 0 {
            nofollow(OpenOptions::new().write(true).append(true).create_new(true)).open(&path)?
        } else {
            nofollow(OpenOptions::new().append(true)).open(&path)?
        };
        let before = file.metadata()?;
        if !before.is_file()
            || (snapshot.head.generation > 0
                && (Some(file_identity(&before)?) != snapshot.head.source_identity
                    || before.len() != snapshot.head.committed_offset))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source changed before append",
            ));
        }
        file.write_all(&line)?;
        file.sync_all()?;
        // A crash before the head rename must leave the new source name/suffix
        // durable, so a zero or older head cannot be read as complete.
        guard.dir_file.sync_all()?;
        let (actual, after) =
            read_regular(&path, AlertInputHeadUnknown::MissingSource).map_err(io_unknown)?;
        let mut expected = old_bytes;
        expected.extend_from_slice(&line);
        if !same_identity(&before, &after) || actual != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source changed during append",
            ));
        }
        let generation = snapshot.head.generation.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "input generation overflow")
        })?;
        let next = AlertInputHeadV1 {
            version: HEAD_VERSION,
            business_date: date,
            generation,
            committed_offset: expected.len() as u64,
            prefix_sha256: hash(&expected),
            source_identity: Some(file_identity(&after)?),
        };
        publish_head(&guard, date, &next, Some(&old_head))?;
        let (verified, _, _) = inspect_locked(&guard, date, self.origin).map_err(io_unknown)?;
        Ok(verified.head)
    }

    /// Snapshot the exact committed prefix; any missing, uncommitted, replaced,
    /// malformed, or concurrently changed state is typed Unknown.
    pub fn inspect_date_input_head(
        &self,
        date: NaiveDate,
    ) -> Result<VerifiedAlertInputPrefix, AlertInputHeadUnknown> {
        self.ensure_io_allowed().map_err(|error| {
            if error.kind() == io::ErrorKind::PermissionDenied {
                AlertInputHeadUnknown::AccessDenied
            } else {
                AlertInputHeadUnknown::Io(error)
            }
        })?;
        let guard = DateFence::acquire(&self.dir, date, false)?;
        let (snapshot, _, _) = inspect_locked(&guard, date, self.origin)?;
        Ok(snapshot)
    }
}

/// Production strict reader stays separate from the legacy loose reader.
pub fn inspect_date_input_head(
    date: NaiveDate,
) -> Result<VerifiedAlertInputPrefix, AlertInputHeadUnknown> {
    AlertLog::production().inspect_date_input_head(date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::detector::{AlertCategory, AlertDetail, AlertLevel};
    use chrono::Local;
    use std::sync::{Arc, Barrier};

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
    }

    fn event(code: &str) -> AlertEvent {
        AlertEvent {
            level: AlertLevel::Important,
            category: AlertCategory::MainOutflow,
            code: code.into(),
            name: "告警".into(),
            message: "测试告警".into(),
            detail: AlertDetail {
                price: Some(10.0),
                change_pct: None,
                volume_ratio: None,
                main_flow_yi: None,
                threshold: None,
                news_title: None,
                news_summary: None,
                news_importance: None,
                ai_decision: None,
                t1_locked: false,
                extra: None,
            },
            triggered_at: Local::now(),
            routed_external_id: None,
        }
    }

    fn rewrite_head_to_match_source(dir: &Path) {
        let source = dated_file_for(dir, "jsonl", date());
        let bytes = fs::read(&source).unwrap();
        let mut head = read_head(&head_path(dir, date()), date()).unwrap().value;
        head.committed_offset = bytes.len() as u64;
        head.prefix_sha256 = hash(&bytes);
        fs::write(head_path(dir, date()), head_bytes(&head).unwrap()).unwrap();
    }

    #[test]
    fn missing_head_and_prehead_file_stay_unknown_but_explicit_zero_head_is_readable() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::production_at(temp.path());
        let absent_dir = temp.path().join("absent");
        let absent_archive = AlertLog::production_at(&absent_dir);
        assert!(matches!(
            absent_archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::MissingDirectory)
        ));
        assert!(!absent_dir.exists());
        assert!(matches!(
            AlertLog::production().inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::AccessDenied)
        ));
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::MissingFence)
        ));
        assert!(!lock_path(temp.path(), date()).exists());
        let source = dated_file_for(temp.path(), "jsonl", date());
        fs::write(&source, []).unwrap();
        assert!(archive.initialize_date_input_head(date()).is_err());
        fs::remove_file(&source).unwrap();

        let head = archive.initialize_date_input_head(date()).unwrap();
        assert_eq!(head.generation(), 0);
        assert_eq!(head.committed_offset(), 0);
        assert!(archive
            .inspect_date_input_head(date())
            .unwrap()
            .records()
            .is_empty());
        fs::write(&source, []).unwrap();
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::UnexpectedSource)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn strict_read_succeeds_with_read_only_directory_and_fence() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::production_at(temp.path());
        archive
            .append_date_jsonl_fenced(date(), &event("600001"))
            .unwrap();
        let lock = lock_path(temp.path(), date());
        let dir_mode = fs::metadata(temp.path()).unwrap().permissions().mode();
        let lock_mode = fs::metadata(&lock).unwrap().permissions().mode();
        fs::set_permissions(&lock, fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let result = archive.inspect_date_input_head(date());
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(dir_mode)).unwrap();
        fs::set_permissions(&lock, fs::Permissions::from_mode(lock_mode)).unwrap();
        assert_eq!(result.unwrap().records()[0].code, "600001");
    }

    #[test]
    fn fenced_append_publishes_exact_prefix_and_rejects_uncommitted_suffix_and_truncation() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::production_at(temp.path());
        let head = archive
            .append_date_jsonl_fenced(date(), &event("600001"))
            .unwrap();
        assert_eq!(head.generation(), 1);
        let snapshot = archive.inspect_date_input_head(date()).unwrap();
        assert_eq!(snapshot.records().len(), 1);
        assert_eq!(snapshot.records()[0].code, "600001");
        let source = dated_file_for(temp.path(), "jsonl", date());
        let committed = fs::read(&source).unwrap();
        assert_eq!(head.committed_offset(), committed.len() as u64);
        assert_eq!(head.prefix_sha256(), hash(&committed));

        OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all(b"{\"partial\"")
            .unwrap();
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::OffsetMismatch { .. })
        ));
        assert!(archive
            .append_date_jsonl_fenced(date(), &event("600002"))
            .is_err());

        fs::write(&source, &committed[..committed.len() - 1]).unwrap();
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::OffsetMismatch { .. })
        ));
    }

    #[test]
    fn matching_head_still_rejects_bad_line_and_changed_source_identity_or_digest() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::production_at(temp.path());
        archive
            .append_date_jsonl_fenced(date(), &event("600001"))
            .unwrap();
        let source = dated_file_for(temp.path(), "jsonl", date());
        let original = fs::read(&source).unwrap();

        fs::write(&source, b"{\"code\":}\n").unwrap();
        rewrite_head_to_match_source(temp.path());
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::MalformedLine { line: 1, .. })
        ));

        fs::write(&source, b"{\"code\":1}").unwrap();
        rewrite_head_to_match_source(temp.path());
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::TruncatedFinalLine)
        ));

        fs::write(&source, &original).unwrap();
        rewrite_head_to_match_source(temp.path());
        let mut changed = original.clone();
        changed[0] = b' ';
        fs::write(&source, changed).unwrap();
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::DigestMismatch)
        ));

        fs::write(&source, &original).unwrap();
        rewrite_head_to_match_source(temp.path());
        fs::rename(&source, temp.path().join("old.jsonl")).unwrap();
        fs::write(&source, &original).unwrap();
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::SourceIdentityMismatch)
        ));
        fs::remove_file(&source).unwrap();
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::MissingSource)
        ));
    }

    #[test]
    fn concurrent_fenced_appends_serialize_one_head_generation_per_line() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::production_at(temp.path());
        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = ["600001", "600002"]
            .into_iter()
            .map(|code| {
                let archive = archive.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    archive
                        .append_date_jsonl_fenced(date(), &event(code))
                        .unwrap()
                })
            })
            .collect();
        barrier.wait();
        for handle in handles {
            handle.join().unwrap();
        }
        let snapshot = archive.inspect_date_input_head(date()).unwrap();
        assert_eq!(snapshot.head().generation(), 2);
        assert_eq!(snapshot.records().len(), 2);
        let mut codes: Vec<_> = snapshot
            .records()
            .iter()
            .map(|record| record.code.as_str())
            .collect();
        codes.sort_unstable();
        assert_eq!(codes, ["600001", "600002"]);
    }

    #[cfg(unix)]
    #[test]
    fn head_and_fence_symlinks_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let archive = AlertLog::production_at(temp.path());
        archive.initialize_date_input_head(date()).unwrap();
        let target = temp.path().join("target");
        fs::write(&target, b"{}\n").unwrap();
        fs::remove_file(head_path(temp.path(), date())).unwrap();
        std::os::unix::fs::symlink(&target, head_path(temp.path(), date())).unwrap();
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::NonRegular)
        ));
        fs::remove_file(head_path(temp.path(), date())).unwrap();
        fs::remove_file(lock_path(temp.path(), date())).unwrap();
        std::os::unix::fs::symlink(&target, lock_path(temp.path(), date())).unwrap();
        assert!(matches!(
            archive.inspect_date_input_head(date()),
            Err(AlertInputHeadUnknown::NonRegular)
        ));
        assert!(archive.initialize_date_input_head(date()).is_err());
    }
}
