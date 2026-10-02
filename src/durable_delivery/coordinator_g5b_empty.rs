//! Closed zero-prefix completion. No provider, model, delivery decision or sink.
//! SQL codecs are evidence checks; only the guarded actual reader mints a seal.
use super::*;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};

const EMPTY_SELECTION: &str = "g5b-empty-closed-selection-v1";
const EMPTY_CLOSING: &str = "g5b-empty-zero-prefix-closing-v1";
const EMPTY_SEAL: &str = "g5b-empty-day-seal-v1";
const EMPTY_REASON: &str = "NoEligibleInVerifiedClosedWindowPrefix";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyClosing {
    material: String,
    business_date: NaiveDate,
    observed_at: DateTime<Utc>,
    calendar_authority_sha256: String,
    prospective_canonical: Vec<u8>,
    prospective_sha256: String,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptySelection {
    version: u8,
    material: String,
    selection_kind: String,
    business_date: NaiveDate,
    reason: String,
    closing_canonical: Vec<u8>,
    closing_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptySeal {
    version: u8,
    material: String,
    business_date: NaiveDate,
    cohort_identity: String,
    revision: i64,
    reason: String,
    selection_sha256: String,
    prospective_sha256: String,
    closing_sha256: String,
    prepared_event_identity: String,
    committed_event_identity: String,
    committed_revision: i64,
    selection_file_witness: artifact::FileWitness,
}

/// Completion of the original closed zero prefix, never whole-day absence.
/// It is not Clone/Deserialize and records only actual observed source identity.
pub(crate) struct VerifiedG5bEmptySeal {
    date: NaiveDate,
    cohort_identity: String,
    revision: i64,
    seal_identity: String,
    seal_sha256: String,
    current_head_canonical: Vec<u8>,
}
impl VerifiedG5bEmptySeal {
    pub(crate) fn business_date(&self) -> NaiveDate {
        self.date
    }
    pub(crate) fn cohort_identity(&self) -> &str {
        &self.cohort_identity
    }
    pub(crate) fn revision(&self) -> i64 {
        self.revision
    }
    pub(crate) fn identity(&self) -> &str {
        &self.seal_identity
    }
    pub(crate) fn sha256(&self) -> &str {
        &self.seal_sha256
    }
    pub(crate) fn reason(&self) -> &'static str {
        EMPTY_REASON
    }
}

/// Routing observations have no model, delivery or completion authority.
pub(crate) enum G5bEmptyDayInspection {
    Absent,
    NonEmpty,
    Pending(G5bEmptyPending),
    Sealed(VerifiedG5bEmptySeal),
}
pub(crate) struct G5bEmptyPending {
    date: NaiveDate,
    cohort_identity: String,
    revision: i64,
}
impl G5bEmptyPending {
    pub(crate) fn business_date(&self) -> NaiveDate {
        self.date
    }
    pub(crate) fn cohort_identity(&self) -> &str {
        &self.cohort_identity
    }
    pub(crate) fn revision(&self) -> i64 {
        self.revision
    }
}

fn routing_snapshot(connection: &Connection, date: NaiveDate) -> Result<Option<(String, String)>> {
    connection
        .query_row(
            "SELECT selection_kind,cohort_identity FROM g5b_cohorts WHERE business_date=?1",
            [date.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(Into::into)
}

fn closing_time(date: NaiveDate, now: DateTime<Utc>) -> bool {
    let local = now.with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap());
    local.date_naive() == date && local.time() >= NaiveTime::from_hms_opt(15, 21, 0).unwrap()
}
fn preimage(domain: &str, bytes: &[u8]) -> Vec<u8> {
    let mut value = domain.as_bytes().to_vec();
    value.push(0);
    value.extend_from_slice(bytes);
    value
}
fn decode_closing(bytes: &[u8]) -> Result<EmptyClosing> {
    let value: EmptyClosing = serde_json::from_slice(bytes)?;
    let prospective = decode_prospective(&value.prospective_canonical)?;
    if canonical_json(&value)? != bytes
        || value.material != EMPTY_CLOSING
        || !closing_time(value.business_date, value.observed_at)
        || value.observed_at <= prospective.observed_at
        || value.business_date != prospective.business_date
        || value.calendar_authority_sha256 != prospective.calendar_authority_sha256
        || sha256_hex(&value.prospective_canonical) != value.prospective_sha256
        || value.input_head_canonical != prospective.input_head_canonical
        || value.input_head_sha256 != prospective.input_head_sha256
        || value.head_witness != prospective.head_witness
        || (value.namespace_device, value.namespace_inode)
            != (prospective.namespace_device, prospective.namespace_inode)
        || (value.lock_device, value.lock_inode)
            != (prospective.lock_device, prospective.lock_inode)
        || (value.database_device, value.database_inode)
            != (prospective.database_device, prospective.database_inode)
        || value.environment != prospective.environment
    {
        return Err(mismatch(
            "Empty closing is not bound to original prospective zero observation",
        ));
    }
    Ok(value)
}
fn decode_selection(bytes: &[u8]) -> Result<(EmptySelection, EmptyClosing)> {
    let value: EmptySelection = serde_json::from_slice(bytes)?;
    let closing = decode_closing(&value.closing_canonical)?;
    if canonical_json(&value)? != bytes
        || value.version != 1
        || value.material != EMPTY_SELECTION
        || value.selection_kind != "Empty"
        || value.reason != EMPTY_REASON
        || value.business_date != closing.business_date
        || sha256_hex(&value.closing_canonical) != value.closing_sha256
    {
        return Err(mismatch("Empty selection canonical mismatch"));
    }
    Ok((value, closing))
}
fn decode_seal(bytes: &[u8]) -> Result<EmptySeal> {
    let value: EmptySeal = serde_json::from_slice(bytes)?;
    if canonical_json(&value)? != bytes
        || value.version != 1
        || value.material != EMPTY_SEAL
        || value.reason != EMPTY_REASON
        || value.revision <= 0
        || value.committed_revision <= 0
        || value.committed_revision > value.revision
    {
        return Err(mismatch("unsupported or malformed Empty seal"));
    }
    Ok(value)
}

struct StoredEmpty {
    selection: EmptySelection,
    closing: EmptyClosing,
    bytes: Vec<u8>,
    identity: String,
    intent: PreparedG5bArtifact,
    committed: Option<(String, i64, artifact::FileWitness)>,
    revision: i64,
    state: String,
    published: Option<String>,
    pointer: Option<String>,
}

#[cfg(test)]
struct EmptyFileReadFault {
    namespace: (u64, u64),
    date: NaiveDate,
    armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    remaining: usize,
    fault: Box<dyn FnOnce() -> Result<()>>,
}
#[cfg(test)]
thread_local! {
    static EMPTY_FILE_READ_FAULT: std::cell::RefCell<Option<EmptyFileReadFault>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn run_file_read_fault(session: &G5bDaySession<'_>) -> Result<()> {
    let fault = EMPTY_FILE_READ_FAULT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(value) = slot.as_mut() {
            if value.namespace == session.namespace_identity
                && value.date == session.date
                && value.armed.load(std::sync::atomic::Ordering::SeqCst)
            {
                value.remaining -= 1;
                if value.remaining == 0 {
                    return slot.take().map(|v| v.fault);
                }
            }
        }
        None
    });
    if let Some(fault) = fault {
        fault()?;
    }
    Ok(())
}
fn load_empty(connection: &Connection, date: NaiveDate) -> Result<Option<StoredEmpty>> {
    let row:Option<(String,String,Vec<u8>)>=connection.query_row(
        "SELECT selection_kind,cohort_identity,selection_canonical FROM g5b_cohorts WHERE business_date=?1",
        [date.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((kind, identity, bytes)) = row else {
        return Ok(None);
    };
    if kind != "Empty" {
        return Ok(None);
    };
    let (selection, closing) = decode_selection(&bytes)?;
    if identity != domain_sha256_hex(EMPTY_SELECTION, &bytes) || selection.business_date != date {
        return Err(mismatch("stored Empty identity differs"));
    }
    let intent = load_selection_intent(connection, &identity)?;
    if intent.desired_bytes != bytes || intent.material.intent.business_date != date {
        return Err(mismatch("Empty original selection intent differs"));
    }
    let committed:Option<(String,i64,Vec<u8>)>=connection.query_row(
        "SELECT event_identity,commit_revision,file_witness_canonical FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Committed'",
        [&intent.logical_intent],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let committed = committed
        .map(|(id, rev, encoded)| -> Result<_> {
            let witness: artifact::FileWitness = serde_json::from_slice(&encoded)?;
            if canonical_json(&witness)? != encoded
                || witness.filename != artifact::filename(&intent)
                || witness.sha256 != sha256_hex(&bytes)
                || witness.byte_length != bytes.len() as u64
            {
                return Err(mismatch("Empty committed file witness differs"));
            }
            Ok((id, rev, witness))
        })
        .transpose()?;
    let (revision,state,published,pointer,prospective):(i64,String,Option<String>,Option<String>,Option<Vec<u8>>)=connection.query_row(
        "SELECT revision,artifact_state,cohort_identity,current_seal_identity,prospective_canonical FROM g5b_day_heads WHERE business_date=?1",
        [date.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?;
    if prospective.as_deref() != Some(closing.prospective_canonical.as_slice()) {
        return Err(mismatch("Empty does not retain exact prospective receipt"));
    }
    Ok(Some(StoredEmpty {
        selection,
        closing,
        bytes,
        identity,
        intent,
        committed,
        revision,
        state,
        published,
        pointer,
    }))
}

fn require_no_decisions(connection: &Connection, date: NaiveDate) -> Result<()> {
    let count:i64=connection.query_row("SELECT COUNT(*) FROM delivery_decisions WHERE business_date=?1 AND push_kind='G5bAttribution'",[date.to_string()],|row|row.get(0))?;
    if count != 0 {
        return Err(mismatch(
            "Empty date has a G5b decision; legacy evidence cannot be adopted",
        ));
    }
    for table in ["g5b_selected_occurrences", "g5b_occurrence_owners"] {
        let count: i64 = connection.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE business_date=?1"),
            [date.to_string()],
            |row| row.get(0),
        )?;
        if count != 0 {
            return Err(mismatch("Empty date contains a member or owner"));
        }
    }
    Ok(())
}
fn require_empty_artifacts(connection: &Connection, stored: &StoredEmpty) -> Result<()> {
    let count:i64=connection.query_row("SELECT COUNT(*) FROM g5b_artifact_events WHERE business_date=?1 AND (cohort_identity!=?2 OR artifact_role!='Selection' OR occurrence_identity IS NOT NULL OR logical_intent!=?3)",
        params![stored.selection.business_date.to_string(),stored.identity,stored.intent.logical_intent],|row|row.get(0))?;
    if count != 0 {
        return Err(mismatch("Empty date contains another artifact"));
    }
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM g5b_artifact_events WHERE logical_intent=?1",
        [&stored.intent.logical_intent],
        |row| row.get(0),
    )?;
    if count != if stored.committed.is_some() { 2 } else { 1 } {
        return Err(mismatch(
            "Empty date lacks its exact original artifact events",
        ));
    }
    Ok(())
}

fn empty_sql_snapshot(connection: &Connection, date: NaiveDate) -> Result<Vec<u8>> {
    let stored =
        load_empty(connection, date)?.ok_or_else(|| mismatch("Empty SQL snapshot absent"))?;
    require_no_decisions(connection, date)?;
    require_empty_artifacts(connection, &stored)?;
    let mut statement = connection.prepare("SELECT seal_identity,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage FROM g5b_day_seals WHERE business_date=?1 ORDER BY seal_identity")?;
    let seals = statement
        .query_map([date.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Vec<u8>>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    canonical_json(&(stored_binding(&stored)?, seals))
}
fn require_empty_sql_snapshot(
    connection: &Connection,
    date: NaiveDate,
    expected: &[u8],
) -> Result<()> {
    if empty_sql_snapshot(connection, date)? != expected {
        return Err(mismatch(
            "Empty exact SQL snapshot changed at transaction boundary",
        ));
    }
    Ok(())
}
fn require_captured_empty_sql_snapshot(
    connection: &Connection,
    date: NaiveDate,
    expected: &std::cell::RefCell<Option<Vec<u8>>>,
) -> Result<()> {
    let value = expected.borrow();
    require_empty_sql_snapshot(
        connection,
        date,
        value
            .as_deref()
            .ok_or_else(|| mismatch("Empty expected SQL snapshot was not captured"))?,
    )
}

#[cfg(all(
    target_pointer_width = "64",
    any(target_os = "macos", target_os = "linux")
))]
#[repr(C)]
struct NativeEmptyDirent {
    inode: u64,
    offset: u64,
    record_length: u16,
    #[cfg(target_os = "macos")]
    name_length: u16,
    kind: u8,
    #[cfg(target_os = "macos")]
    name: [std::ffi::c_char; 1024],
    #[cfg(target_os = "linux")]
    name: [std::ffi::c_char; 256],
}
#[cfg(all(
    target_pointer_width = "64",
    any(target_os = "macos", target_os = "linux")
))]
unsafe extern "C" {
    #[cfg_attr(
        all(target_os = "macos", target_arch = "x86_64"),
        link_name = "fdopendir$INODE64"
    )]
    fn fdopendir(fd: i32) -> *mut std::ffi::c_void;
    #[cfg_attr(
        all(target_os = "macos", not(target_arch = "aarch64")),
        link_name = "readdir$INODE64"
    )]
    fn readdir(directory: *mut std::ffi::c_void) -> *const NativeEmptyDirent;
    fn closedir(directory: *mut std::ffi::c_void) -> i32;
    #[cfg(target_os = "macos")]
    fn __error() -> *mut i32;
    #[cfg(target_os = "linux")]
    fn __errno_location() -> *mut i32;
}
#[cfg(all(
    target_pointer_width = "64",
    any(target_os = "macos", target_os = "linux")
))]
fn retained_directory_leaves(parent: &std::fs::File) -> Result<Vec<std::ffi::OsString>> {
    struct Stream(*mut std::ffi::c_void);
    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe {
                closedir(self.0);
            }
        }
    }
    let descriptor = parent.try_clone().map_err(io_error)?.into_raw_fd();
    // fdopendir consumes this owned duplicate only on success. The original
    // directory pin remains retained through full namespace revalidation.
    let stream = unsafe { fdopendir(descriptor) };
    if stream.is_null() {
        let error = std::io::Error::last_os_error();
        drop(unsafe { std::fs::File::from_raw_fd(descriptor) });
        return Err(io_error(error));
    }
    let stream = Stream(stream);
    let mut leaves = Vec::new();
    loop {
        #[cfg(target_os = "macos")]
        let errno = unsafe { __error() };
        #[cfg(target_os = "linux")]
        let errno = unsafe { __errno_location() };
        unsafe {
            *errno = 0;
        }
        let entry = unsafe { readdir(stream.0) };
        if entry.is_null() {
            let error = unsafe { *errno };
            if error != 0 {
                return Err(io_error(std::io::Error::from_raw_os_error(error)));
            }
            break;
        }
        // These are the native 64-bit dirent layouts used by the already
        // supported attestation targets. Each record is owned by the stream;
        // copy its bounded name before invoking readdir again.
        let name_offset = std::mem::offset_of!(NativeEmptyDirent, name);
        let record_length = unsafe { std::ptr::addr_of!((*entry).record_length).read() } as usize;
        if record_length <= name_offset || record_length > std::mem::size_of::<NativeEmptyDirent>()
        {
            return Err(mismatch("invalid Empty directory record length"));
        }
        #[cfg(target_os = "macos")]
        let name_capacity = 1024;
        #[cfg(target_os = "linux")]
        let name_capacity = 256;
        let available = (record_length - name_offset).min(name_capacity);
        let record =
            unsafe { std::slice::from_raw_parts(entry.cast::<u8>().add(name_offset), available) };
        let name = record
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| mismatch("unterminated Empty directory leaf"))?;
        let bytes = &record[..name];
        if bytes == b"." || bytes == b".." {
            continue;
        }
        if leaves.len() >= 10_000 {
            return Err(mismatch("Empty directory enumeration exceeds bound"));
        }
        leaves.push(std::ffi::OsString::from_vec(bytes.to_vec()));
    }
    Ok(leaves)
}
#[cfg(not(all(
    target_pointer_width = "64",
    any(target_os = "macos", target_os = "linux")
)))]
fn retained_directory_leaves(_: &std::fs::File) -> Result<Vec<std::ffi::OsString>> {
    Err(mismatch(
        "retained Empty directory enumeration unsupported on this target",
    ))
}
fn require_absent_child(parent: &std::fs::File, name: &std::ffi::OsStr) -> Result<()> {
    let name = std::ffi::CString::new(name.as_bytes()).map_err(codec_error)?;
    let descriptor = unsafe {
        super::super::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            super::super::PIN_O_RDONLY
                | super::super::PIN_O_NOFOLLOW
                | super::super::PIN_O_NONBLOCK
                | super::super::PIN_O_CLOEXEC,
            0u32,
        )
    };
    if descriptor >= 0 {
        drop(unsafe { std::fs::File::from_raw_fd(descriptor) });
        return Err(mismatch(
            "Empty legacy component appeared during observation",
        ));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() != std::io::ErrorKind::NotFound {
        return Err(io_error(error));
    }
    Ok(())
}

/// Enumerate only a retained directory descriptor. A missing legacy directory
/// is proved by its first absent component beneath a retained existing parent;
/// aliases, ancestor replacement and a newly appearing component fail closed.
fn empty_directory_leaves(path: &std::path::Path) -> Result<Vec<std::ffi::OsString>> {
    let mut existing = path;
    loop {
        match std::fs::symlink_metadata(existing) {
            Ok(metadata) if metadata.file_type().is_dir() => break,
            Ok(_) => {
                return Err(mismatch(
                    "Empty directory ancestor is an alias or non-directory",
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .ok_or_else(|| mismatch("Empty directory has no retained parent"))?;
            }
            Err(error) => return Err(io_error(error)),
        }
    }
    let chain = super::super::PinnedDirectoryChain::open(existing, std::path::Path::new(""))?;
    chain.validate()?;
    if existing != path {
        let missing = path
            .strip_prefix(existing)
            .map_err(codec_error)?
            .components()
            .next()
            .ok_or_else(|| mismatch("Empty missing component absent"))?;
        require_absent_child(chain.parent_anchor()?, missing.as_os_str())?;
        chain.validate()?;
        require_absent_child(chain.parent_anchor()?, missing.as_os_str())?;
        return Ok(Vec::new());
    }
    let leaves = retained_directory_leaves(chain.parent_anchor()?)?;
    chain.validate()?;
    Ok(leaves)
}

pub(super) fn validate_cohort_row(
    connection: &Connection,
    identity: &str,
    date: &str,
    bytes: &[u8],
    sha: &str,
    cohort_preimage: &[u8],
    count: i64,
    generation: i64,
    offset: i64,
    prefix: &str,
    device: Option<&str>,
    inode: Option<&str>,
    admission: &[u8],
    admission_sha: &str,
) -> Result<()> {
    let (selection, closing) = decode_selection(bytes)?;
    if selection.business_date.to_string() != date
        || identity != domain_sha256_hex(EMPTY_SELECTION, bytes)
        || cohort_preimage != preimage(EMPTY_SELECTION, bytes)
        || sha256_hex(bytes) != sha
        || count != 0
        || generation != 0
        || offset != 0
        || prefix != sha256_hex(&[])
        || device.is_some()
        || inode.is_some()
        || admission != selection.closing_canonical
        || sha256_hex(admission) != admission_sha
    {
        return Err(mismatch(
            "Empty columns do not bind strict closed-zero preimage",
        ));
    }
    let stored = load_empty(connection, closing.business_date)?
        .ok_or_else(|| mismatch("Empty original intent missing"))?;
    require_no_decisions(connection, closing.business_date)?;
    require_empty_artifacts(connection, &stored)
}

fn validate_seal_row(
    connection: &Connection,
    identity: &str,
    date: &str,
    cohort: &str,
    revision: i64,
    bytes: &[u8],
    sha: &str,
    seal_preimage: &[u8],
) -> Result<EmptySeal> {
    let value = decode_seal(bytes)?;
    let stored = load_empty(connection, value.business_date)?
        .ok_or_else(|| mismatch("Physical/non-Empty seal remains unsupported"))?;
    let (committed_id, committed_revision, witness) = stored
        .committed
        .as_ref()
        .ok_or_else(|| mismatch("Empty seal lacks committed selection"))?;
    if value.business_date.to_string() != date
        || value.cohort_identity != cohort
        || stored.identity != cohort
        || value.revision != revision
        || value.revision > stored.revision
        || value.revision != *committed_revision
        || identity != domain_sha256_hex(EMPTY_SEAL, bytes)
        || sha256_hex(bytes) != sha
        || seal_preimage != preimage(EMPTY_SEAL, bytes)
        || value.selection_sha256 != sha256_hex(&stored.bytes)
        || value.closing_sha256 != stored.selection.closing_sha256
        || value.prospective_sha256 != stored.closing.prospective_sha256
        || value.prepared_event_identity != stored.intent.event_identity
        || value.committed_event_identity != *committed_id
        || value.committed_revision != *committed_revision
        || value.selection_file_witness != *witness
    {
        return Err(mismatch("Empty seal does not bind exact immutable closure"));
    }
    require_no_decisions(connection, value.business_date)?;
    require_empty_artifacts(connection, &stored)?;
    Ok(value)
}
pub(super) fn validate_seals(connection: &Connection) -> Result<()> {
    let mut statement=connection.prepare("SELECT seal_identity,business_date,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage FROM g5b_day_seals")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Vec<u8>>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, date, cohort, revision, bytes, sha, preimage) in rows {
        validate_seal_row(
            connection, &id, &date, &cohort, revision, &bytes, &sha, &preimage,
        )?;
    }
    Ok(())
}
pub(super) fn validate_current_pointer(
    connection: &Connection,
    date: &str,
    revision: i64,
    state: &str,
    cohort: Option<&str>,
    pointer: &str,
) -> Result<()> {
    let (seal_date,seal_cohort,seal_revision,bytes,sha,preimage):(String,String,i64,Vec<u8>,String,Vec<u8>)=connection.query_row(
        "SELECT business_date,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage FROM g5b_day_seals WHERE seal_identity=?1",[pointer],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)))?;
    if seal_date != date
        || seal_revision != revision
        || Some(seal_cohort.as_str()) != cohort
        || state != "Clean"
    {
        return Err(mismatch(
            "current Empty seal pointer does not bind current clean revision",
        ));
    }
    validate_seal_row(
        connection,
        pointer,
        date,
        &seal_cohort,
        revision,
        &bytes,
        &sha,
        &preimage,
    )?;
    Ok(())
}

impl G5bDaySession<'_> {
    fn require_empty_clock(&self, test_clock: bool) -> Result<()> {
        self.validate()?;
        if test_clock
            && !matches!(
                self.coordinator.config.environment,
                crate::durable_delivery::StoreEnvironment::Test { .. }
            )
        {
            return Err(mismatch("Test Empty clock cannot access Production"));
        }
        Ok(())
    }
    fn validate_empty_binding(&self, closing: &EmptyClosing) -> Result<()> {
        self.validate()?;
        let db = self.coordinator.database_binding()?.objects[0].identity;
        if closing.business_date != self.date
            || (closing.namespace_device, closing.namespace_inode) != self.namespace_identity
            || self.fence.lock_identity().map_err(io_error)?
                != (closing.lock_device, closing.lock_inode)
            || (closing.database_device, closing.database_inode) != (db.device, db.inode)
            || closing.environment != self.environment()
        {
            return Err(mismatch(
                "Empty closing belongs to another namespace/guard/database",
            ));
        }
        Ok(())
    }
    fn empty_actual_input(
        &self,
        closing: &EmptyClosing,
        allow_suffix: bool,
        known: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        self.validate_empty_binding(closing)?;
        let prefix = self
            .coordinator
            .g5b_input_log
            .inspect_date_input_prefix_locked_bounded(self.date, &self.fence, MAX_ARTIFACT_BYTES)
            .map_err(codec_error)?;
        let current = prefix.current_cutoff().map_err(codec_error)?;
        let current_head = artifact::inspect_bytes(
            self,
            &closing.head_witness.filename,
            current.head_canonical(),
        )?;
        if let Some((device, inode)) = current.source_identity() {
            let mut bytes = Vec::with_capacity(
                usize::try_from(current.head().committed_offset()).map_err(codec_error)?,
            );
            for line in current.lines() {
                bytes.extend_from_slice(line.raw_bytes());
            }
            let source = artifact::inspect_bytes(
                self,
                &format!("{}.jsonl", self.date.format("%Y%m%d")),
                &bytes,
            )?;
            if (source.device, source.inode) != (device, inode)
                || source.sha256 != current.head().prefix_sha256()
            {
                return Err(mismatch("current Empty suffix source witness differs"));
            }
        }
        // Every current byte is checked even though the frozen cutoff is zero.
        prefix
            .cutoff_for_captured_head(&closing.input_head_canonical)
            .map_err(codec_error)?;
        if let Some(known) = known {
            prefix
                .cutoff_for_captured_head(known)
                .map_err(codec_error)?;
        }
        if current.head().generation() == 0 {
            if current.head_canonical() != closing.input_head_canonical
                || current_head != closing.head_witness
            {
                return Err(mismatch("original zero input head was replaced"));
            }
        } else if !allow_suffix {
            return Err(mismatch(
                "initial Empty publication still requires the original absent source",
            ));
        }
        self.validate()?;
        Ok(current.head_canonical().to_vec())
    }
    fn reject_empty_unknown_files(&self, expected: Option<&PreparedG5bArtifact>) -> Result<()> {
        self.validate()?;
        self.reject_legacy_artifacts()?;
        let namespace = self.fence.namespace_path().map_err(io_error)?;
        let expected = expected.map(artifact::filename);
        let date_prefix = format!("{}.", self.date.format("%Y%m%d"));
        let allowed = [
            format!("{}.jsonl", self.date.format("%Y%m%d")),
            format!("{}.input-head.v1.json", self.date.format("%Y%m%d")),
            format!("{}.g5b-day.lock", self.date.format("%Y%m%d")),
        ];
        for leaf in empty_directory_leaves(namespace)? {
            let leaf = leaf
                .to_str()
                .ok_or_else(|| mismatch("non-UTF8 Empty namespace leaf"))?;
            if leaf.starts_with(".g5b-v2-")
                || (leaf.starts_with(&date_prefix)
                    && !allowed.iter().any(|v| v == leaf)
                    && expected.as_deref() != Some(leaf))
            {
                return Err(mismatch("unexplained local artifact prevents Empty"));
            }
        }
        let legacy = match self.coordinator.config.environment {
            crate::durable_delivery::StoreEnvironment::Production => {
                crate::production_root::production_root().join("data/g5b/attempts")
            }
            crate::durable_delivery::StoreEnvironment::Test { .. } => namespace.join("attempts"),
        };
        // Inspect every legacy date leaf, including indices beyond the old top3.
        for leaf in empty_directory_leaves(&legacy)? {
            let leaf = leaf
                .to_str()
                .ok_or_else(|| mismatch("non-UTF8 legacy leaf"))?;
            if leaf.starts_with(&format!("{}.", self.date)) {
                return Err(mismatch("legacy day artifact prevents Empty"));
            }
        }
        self.validate()
    }
    fn empty_fs_validate(
        &self,
        stored: &StoredEmpty,
        allow_suffix: bool,
        known: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        self.reject_empty_unknown_files(Some(&stored.intent))?;
        let current = self.empty_actual_input(&stored.closing, allow_suffix, known)?;
        if let Some((_, _, expected)) = &stored.committed {
            if artifact::inspect(self, &stored.intent)? != *expected {
                return Err(mismatch("Committed Empty selection changed; no healing"));
            }
            #[cfg(test)]
            run_file_read_fault(self)?;
        }
        // Files first, actual input last. Do not let the last artifact read
        // leave an earlier head/source observation as the completion proof.
        let after = self.empty_actual_input(&stored.closing, allow_suffix, Some(&current))?;
        if after != current {
            return Err(mismatch(
                "Empty actual input changed during unified file observation",
            ));
        }
        self.validate()?;
        Ok(after)
    }

    /// Prospective initialization observes a real new zero head before 15:05.
    /// It grants no Empty selection, model work or completion capability.
    pub(crate) fn initialize_empty_prospective(&self) -> Result<()> {
        self.initialize_empty_prospective_at(Utc::now(), false)
    }
    fn initialize_empty_prospective_at(&self, now: DateTime<Utc>, test_clock: bool) -> Result<()> {
        self.require_empty_clock(test_clock)?;
        if !prospective_time(self.date, now)
            || !crate::calendar::verified_a_share_trading_day(self.date).map_err(codec_error)?
        {
            return Err(mismatch(
                "Empty initialization requires current verified trading day before 15:05",
            ));
        }
        self.reject_empty_unknown_files(None)?;
        let prospective = self.transaction(|tx| {
            require_no_decisions(tx, self.date)?;
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM g5b_cohorts WHERE business_date=?1",
                [self.date.to_string()],
                |row| row.get(0),
            )?;
            if count != 0 {
                return Err(mismatch(
                    "prospective initialization cannot adopt an existing cohort",
                ));
            }
            let value: Option<Option<Vec<u8>>> = tx
                .query_row(
                    "SELECT prospective_canonical FROM g5b_day_heads WHERE business_date=?1",
                    [self.date.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(value)
        })?;
        let fresh = if test_clock { now } else { Utc::now() };
        if !prospective_time(self.date, fresh) {
            return Err(mismatch("prospective window expired before initialization"));
        }
        match prospective {
            None => {
                self.coordinator
                    .g5b_input_log
                    .initialize_new_date_input_head_locked(self.date, &self.fence)
                    .map_err(io_error)?;
            }
            Some(Some(_)) => {}
            Some(None) => {
                return Err(mismatch(
                    "existing SQL day head has no prospective receipt; cannot be adopted",
                ))
            }
        }
        self.observe_prospective_zero_head_at(if test_clock { now } else { Utc::now() }, test_clock)
    }

    fn prepare_empty_closure_at(
        &self,
        now: DateTime<Utc>,
        test_clock: bool,
    ) -> Result<PreparedG5bArtifact> {
        self.require_empty_clock(test_clock)?;
        if !closing_time(self.date, now)
            || !crate::calendar::verified_a_share_trading_day(self.date).map_err(codec_error)?
        {
            return Err(mismatch(
                "Empty initial closure requires current verified trading day at or after 15:21",
            ));
        }
        if let Some(stored) = self.transaction(|tx| load_empty(tx, self.date))? {
            let sealed = self.transaction(|tx| {
                tx.query_row(
                    "SELECT COUNT(*) FROM g5b_day_seals WHERE business_date=?1",
                    [self.date.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(Into::into)
            })?;
            self.empty_fs_validate(&stored, sealed != 0, None)?;
            return Ok(stored.intent);
        }
        self.reject_empty_unknown_files(None)?;
        let prospective:Vec<u8>=self.transaction(|tx| {
            require_no_decisions(tx,self.date)?;
            let row:(i64,String,Option<String>,Option<String>,Option<Vec<u8>>)=tx.query_row(
                "SELECT revision,artifact_state,cohort_identity,current_seal_identity,prospective_canonical FROM g5b_day_heads WHERE business_date=?1",[self.date.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?;
            if row.0!=0 || row.1!="Clean" || row.2.is_some() || row.3.is_some() {return Err(mismatch("Empty initial head has prior mutations/cohort"));}
            row.4.ok_or_else(||mismatch("Empty has no prospective observation"))
        })?;
        let observed = decode_prospective(&prospective)?;
        let closing = EmptyClosing {
            material: EMPTY_CLOSING.to_owned(),
            business_date: self.date,
            observed_at: now,
            calendar_authority_sha256: observed.calendar_authority_sha256.clone(),
            prospective_sha256: sha256_hex(&prospective),
            prospective_canonical: prospective,
            input_head_canonical: observed.input_head_canonical.clone(),
            input_head_sha256: observed.input_head_sha256.clone(),
            head_witness: observed.head_witness.clone(),
            namespace_device: observed.namespace_device,
            namespace_inode: observed.namespace_inode,
            lock_device: observed.lock_device,
            lock_inode: observed.lock_inode,
            database_device: observed.database_device,
            database_inode: observed.database_inode,
            environment: observed.environment.clone(),
        };
        let closing_bytes = canonical_json(&closing)?;
        decode_closing(&closing_bytes)?;
        let selection = EmptySelection {
            version: 1,
            material: EMPTY_SELECTION.to_owned(),
            selection_kind: "Empty".to_owned(),
            business_date: self.date,
            reason: EMPTY_REASON.to_owned(),
            closing_sha256: sha256_hex(&closing_bytes),
            closing_canonical: closing_bytes.clone(),
        };
        let bytes = canonical_json(&selection)?;
        decode_selection(&bytes)?;
        let identity = domain_sha256_hex(EMPTY_SELECTION, &bytes);
        let expected = std::cell::RefCell::new(None);
        let validate_sql =
            |tx: &Transaction<'_>| require_captured_empty_sql_snapshot(tx, self.date, &expected);
        let validate = || {
            self.reject_empty_unknown_files(None)?;
            self.empty_actual_input(&closing, false, None)?;
            if !test_clock && !closing_time(self.date, Utc::now()) {
                return Err(mismatch("closing date expired at transaction boundary"));
            }
            Ok(())
        };
        self.coordinator.with_immediate_transaction_validated_sql(SchemaVersionPolicy::Runtime,Some(&validate),Some(&validate_sql),|tx| {
            require_no_decisions(tx,self.date)?;
            for table in ["g5b_cohorts","g5b_artifact_events"] {
                let count:i64=tx.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE business_date=?1"),[self.date.to_string()],|row|row.get(0))?;
                if count!=0 {return Err(mismatch("Empty cannot adopt existing cohort/artifacts"));}
            }
            let (revision,prospective):(i64,Vec<u8>)=tx.query_row("SELECT revision,prospective_canonical FROM g5b_day_heads WHERE business_date=?1 AND cohort_identity IS NULL AND artifact_state='Clean'",[self.date.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))?;
            if revision!=0 || prospective!=closing.prospective_canonical {return Err(mismatch("prospective receipt changed before Empty prepare"));}
            tx.execute("INSERT INTO g5b_cohorts(cohort_identity,business_date,selection_kind,selection_canonical,selection_sha256,cohort_preimage,selected_count,cutoff_generation,cutoff_offset,prefix_sha256,source_device,source_inode,admission_canonical,admission_sha256) VALUES(?1,?2,'Empty',?3,?4,?5,0,0,0,?6,NULL,NULL,?7,?8)",params![identity,self.date.to_string(),bytes,sha256_hex(&bytes),preimage(EMPTY_SELECTION,&bytes),sha256_hex(&[]),closing_bytes,sha256_hex(&closing_bytes)])?;
            let intent=prepare_artifact_tx(tx,self.date,&identity,ArtifactRole::Selection,None,&bytes)?;
            *expected.borrow_mut()=Some(empty_sql_snapshot(tx,self.date)?);
            Ok(intent)
        })
    }
}

pub(super) fn is_empty_intent(_: &G5bDaySession<'_>, intent: &PreparedG5bArtifact) -> Result<bool> {
    // Pure dispatch only: this grants nothing until saved receipt/FS validation.
    Ok(intent.material.intent.role == ArtifactRole::Selection
        && serde_json::from_slice::<EmptySelection>(&intent.desired_bytes).is_ok())
}
pub(super) fn validate_saved_intent(
    session: &G5bDaySession<'_>,
    intent: &PreparedG5bArtifact,
) -> Result<()> {
    let stored = session
        .transaction(|tx| load_empty(tx, session.date))?
        .ok_or_else(|| mismatch("incoming Empty intent has no stored receipt"))?;
    if stored.intent.event_identity != intent.event_identity
        || stored.intent.material != intent.material
        || stored.bytes != intent.desired_bytes
    {
        return Err(mismatch("incoming Empty intent is not the exact original"));
    }
    let sealed = session.transaction(|tx| {
        tx.query_row(
            "SELECT COUNT(*) FROM g5b_day_seals WHERE business_date=?1",
            [session.date.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(Into::into)
    })?;
    session.empty_fs_validate(&stored, sealed != 0, None)?;
    Ok(())
}
pub(super) fn commit_prepared(
    session: &G5bDaySession<'_>,
    intent: &PreparedG5bArtifact,
) -> Result<()> {
    let stored = session
        .transaction(|tx| load_empty(tx, session.date))?
        .ok_or_else(|| mismatch("Empty saved selection missing"))?;
    if stored.intent.material != intent.material
        || stored.intent.event_identity != intent.event_identity
        || stored.bytes != intent.desired_bytes
    {
        return Err(mismatch("Empty commit differs from original intent"));
    }
    let sealed = session.transaction(|tx| {
        tx.query_row(
            "SELECT COUNT(*) FROM g5b_day_seals WHERE business_date=?1",
            [session.date.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(Into::into)
    })?;
    let witness = artifact::inspect(session, intent)?;
    let before_sql = stored_binding(&stored)?;
    let expected = std::cell::RefCell::new(None);
    let validate_sql =
        |tx: &Transaction<'_>| require_captured_empty_sql_snapshot(tx, session.date, &expected);
    let validate = || {
        let before = session.empty_fs_validate(&stored, sealed != 0, None)?;
        if artifact::inspect(session, intent)? != witness {
            return Err(mismatch("Empty selection changed at commit boundary"));
        }
        let after = session.empty_actual_input(&stored.closing, sealed != 0, Some(&before))?;
        if after != before {
            return Err(mismatch(
                "Empty input changed during selection commit validation",
            ));
        }
        session.validate()?;
        Ok(())
    };
    session
        .coordinator
        .with_immediate_transaction_validated_sql(
            SchemaVersionPolicy::Runtime,
            Some(&validate),
            Some(&validate_sql),
            |tx| {
                let before = load_empty(tx, session.date)?
                    .ok_or_else(|| mismatch("Empty commit SQL snapshot absent"))?;
                if stored_binding(&before)? != before_sql {
                    return Err(mismatch("Empty SQL changed before original commit"));
                }
                require_no_decisions(tx, session.date)?;
                commit_exact_artifact_tx(tx, session.date, intent, &witness)?;
                *expected.borrow_mut() = Some(empty_sql_snapshot(tx, session.date)?);
                Ok(())
            },
        )
}

struct EmptyRead {
    stored: StoredEmpty,
    identity: String,
    sha256: String,
}
fn stored_binding(value: &StoredEmpty) -> Result<Vec<u8>> {
    canonical_json(&(
        &value.bytes,
        &value.identity,
        &value.intent.event_identity,
        &value.intent.material,
        &value.intent.desired_bytes,
        &value.committed,
        value.revision,
        &value.state,
        &value.published,
        &value.pointer,
    ))
}
fn load_current(connection: &Connection, date: NaiveDate) -> Result<Option<EmptyRead>> {
    let Some(stored) = load_empty(connection, date)? else {
        return Ok(None);
    };
    let Some(identity) = stored.pointer.as_ref() else {
        return Ok(None);
    };
    validate_current_pointer(
        connection,
        &date.to_string(),
        stored.revision,
        &stored.state,
        stored.published.as_deref(),
        identity,
    )?;
    let sha256: String = connection.query_row(
        "SELECT seal_sha256 FROM g5b_day_seals WHERE seal_identity=?1",
        [identity],
        |row| row.get(0),
    )?;
    let identity = identity.clone();
    Ok(Some(EmptyRead {
        stored,
        identity,
        sha256,
    }))
}

impl G5bDaySession<'_> {
    /// Classify before the NonEmpty model reader. Absent means no v2 cohort,
    /// not an empty input day or permission to start new work.
    pub(crate) fn inspect_empty_day(&self) -> Result<G5bEmptyDayInspection> {
        let routing = self.transaction(|tx| routing_snapshot(tx, self.date))?;
        match &routing {
            None | Some((_, _)) if routing.as_ref().is_none_or(|v| v.0 == "NonEmpty") => {
                let validate_sql = |tx: &Transaction<'_>| {
                    if routing_snapshot(tx, self.date)? != routing {
                        return Err(mismatch("Empty routing snapshot changed"));
                    }
                    Ok(())
                };
                let validate = || self.validate();
                self.coordinator.with_immediate_transaction_validated_sql(
                    SchemaVersionPolicy::Runtime,
                    Some(&validate),
                    Some(&validate_sql),
                    |tx| validate_sql(tx),
                )?;
                return Ok(if routing.is_none() {
                    G5bEmptyDayInspection::Absent
                } else {
                    G5bEmptyDayInspection::NonEmpty
                });
            }
            Some((kind, _)) if kind == "Empty" => {}
            _ => return Err(mismatch("unsupported cohort routing kind")),
        }
        let (stored, binding, historical) = self.transaction(|tx| {
            let stored = load_empty(tx, self.date)?
                .ok_or_else(|| mismatch("Empty routing lost its original cohort"))?;
            let binding = empty_sql_snapshot(tx, self.date)?;
            let historical: i64 = tx.query_row(
                "SELECT COUNT(*) FROM g5b_day_seals WHERE business_date=?1",
                [self.date.to_string()],
                |row| row.get(0),
            )?;
            Ok((stored, binding, historical != 0))
        })?;
        if stored.pointer.is_some() {
            return self
                .read_empty_seal()?
                .map(G5bEmptyDayInspection::Sealed)
                .ok_or_else(|| mismatch("Empty current seal disappeared during inspection"));
        }
        // Prepared-but-unpublished is valid pending evidence. If the exact
        // target exists, verify it; do not silently ignore a corrupt alias.
        let publication = artifact::inspect_if_present(self, &stored.intent)?;
        let validate = || {
            let before = self.empty_fs_validate(&stored, historical, None)?;
            if artifact::inspect_if_present(self, &stored.intent)? != publication {
                return Err(mismatch("pending Empty publication changed"));
            }
            #[cfg(test)]
            run_file_read_fault(self)?;
            let after = self.empty_actual_input(&stored.closing, historical, Some(&before))?;
            if after != before {
                return Err(mismatch("pending Empty input changed during observation"));
            }
            self.validate()
        };
        let validate_sql =
            |tx: &Transaction<'_>| require_empty_sql_snapshot(tx, self.date, &binding);
        validate()?;
        self.coordinator.with_immediate_transaction_validated_sql(
            SchemaVersionPolicy::Runtime,
            Some(&validate),
            Some(&validate_sql),
            |tx| validate_sql(tx),
        )?;
        validate()?;
        Ok(G5bEmptyDayInspection::Pending(G5bEmptyPending {
            date: self.date,
            cohort_identity: stored.identity,
            revision: stored.revision,
        }))
    }

    /// Only recover an already saved Empty Selection. This never creates a
    /// closing receipt, cohort or seal, and never recreates a Committed leaf.
    pub(crate) fn recover_existing_empty_day(&self) -> Result<G5bEmptyDayInspection> {
        let state = self.inspect_empty_day()?;
        if !matches!(&state, G5bEmptyDayInspection::Pending(_)) {
            return Ok(state);
        }
        let stored = self
            .transaction(|tx| load_empty(tx, self.date))?
            .ok_or_else(|| mismatch("pending Empty original intent disappeared"))?;
        if stored.committed.is_none() {
            self.publish_prepared_artifact(&stored.intent)?;
        }
        self.commit_prepared_artifact(&stored.intent)?;
        self.inspect_empty_day()
    }

    /// Only this fresh SQL + guarded actual filesystem reader mints completion.
    pub(crate) fn read_empty_seal(&self) -> Result<Option<VerifiedG5bEmptySeal>> {
        self.read_empty_seal_inner(None)
    }
    /// A previously observed positive source incarnation is never silently
    /// replaced. A zero receipt does not invent the future source's inode.
    pub(crate) fn refresh_empty_seal(
        &self,
        known: &VerifiedG5bEmptySeal,
    ) -> Result<VerifiedG5bEmptySeal> {
        if known.date != self.date {
            return Err(mismatch("Empty capability belongs to another date"));
        }
        let current = self
            .read_empty_seal_inner(Some(&known.current_head_canonical))?
            .ok_or_else(|| mismatch("current Empty pointer absent"))?;
        if current.cohort_identity != known.cohort_identity
            || current.seal_identity != known.seal_identity
            || current.revision != known.revision
            || current.seal_sha256 != known.seal_sha256
        {
            return Err(mismatch("Empty capability identity/revision changed"));
        }
        Ok(current)
    }
    fn read_empty_seal_inner(&self, known: Option<&[u8]>) -> Result<Option<VerifiedG5bEmptySeal>> {
        let Some((snapshot, binding)) = self.transaction(|tx| {
            load_current(tx, self.date)?
                .map(|current| Ok((current, empty_sql_snapshot(tx, self.date)?)))
                .transpose()
        })?
        else {
            return Ok(None);
        };
        let validate_sql =
            |tx: &Transaction<'_>| require_empty_sql_snapshot(tx, self.date, &binding);
        let validate = || {
            self.empty_fs_validate(&snapshot.stored, true, known)
                .map(|_| ())
        };
        validate()?;
        self.coordinator.with_immediate_transaction_validated_sql(
            SchemaVersionPolicy::Runtime,
            Some(&validate),
            Some(&validate_sql),
            |tx| validate_sql(tx),
        )?;
        // Recheck after the last SQL hook; never mint from the earlier snapshot.
        let current_head_canonical = self.empty_fs_validate(&snapshot.stored, true, known)?;
        Ok(Some(VerifiedG5bEmptySeal {
            date: self.date,
            cohort_identity: snapshot.stored.identity,
            revision: snapshot.stored.revision,
            seal_identity: snapshot.identity,
            seal_sha256: snapshot.sha256,
            current_head_canonical,
        }))
    }

    pub(crate) fn close_empty_window(&self) -> Result<VerifiedG5bEmptySeal> {
        self.close_empty_window_at(Utc::now(), false)
    }
    fn close_empty_window_at(
        &self,
        now: DateTime<Utc>,
        test_clock: bool,
    ) -> Result<VerifiedG5bEmptySeal> {
        self.require_empty_clock(test_clock)?;
        // Exact historical seals can be read later; no historical first seal.
        if let Some(current) = self.read_empty_seal()? {
            return Ok(current);
        };
        let intent = self.prepare_empty_closure_at(now, test_clock)?;
        let committed=self.transaction(|tx| {
            tx.query_row("SELECT COUNT(*) FROM g5b_artifact_events WHERE logical_intent=?1 AND phase='Committed'",[&intent.logical_intent],|row|row.get::<_,i64>(0)).map_err(Into::into)
        })?;
        if committed == 0 {
            self.publish_prepared_artifact(&intent)?;
        }
        self.commit_prepared_artifact(&intent)?;
        let stored = self
            .transaction(|tx| load_empty(tx, self.date))?
            .ok_or_else(|| mismatch("Empty closure disappeared"))?;
        self.empty_fs_validate(&stored, false, None)?;
        let (committed_id, committed_revision, witness) = stored
            .committed
            .as_ref()
            .ok_or_else(|| mismatch("Empty selection is not committed"))?;
        if stored.state != "Clean"
            || stored.published.as_deref() != Some(stored.identity.as_str())
            || stored.revision != *committed_revision
        {
            return Err(mismatch(
                "Empty seal requires exact committed clean closure revision",
            ));
        }
        let seal = EmptySeal {
            version: 1,
            material: EMPTY_SEAL.to_owned(),
            business_date: self.date,
            cohort_identity: stored.identity.clone(),
            revision: stored.revision,
            reason: EMPTY_REASON.to_owned(),
            selection_sha256: sha256_hex(&stored.bytes),
            prospective_sha256: stored.closing.prospective_sha256.clone(),
            closing_sha256: stored.selection.closing_sha256.clone(),
            prepared_event_identity: stored.intent.event_identity.clone(),
            committed_event_identity: committed_id.clone(),
            committed_revision: *committed_revision,
            selection_file_witness: witness.clone(),
        };
        let canonical = canonical_json(&seal)?;
        decode_seal(&canonical)?;
        let identity = domain_sha256_hex(EMPTY_SEAL, &canonical);
        let binding = stored_binding(&stored)?;
        let expected = std::cell::RefCell::new(None);
        let validate_sql =
            |tx: &Transaction<'_>| require_captured_empty_sql_snapshot(tx, self.date, &expected);
        let validate = || {
            self.empty_fs_validate(&stored, false, None)?;
            if !test_clock && !closing_time(self.date, Utc::now()) {
                return Err(mismatch("initial Empty seal date expired at SQL boundary"));
            }
            Ok(())
        };
        self.coordinator.with_immediate_transaction_validated_sql(SchemaVersionPolicy::Runtime,Some(&validate),Some(&validate_sql),|tx| {
            let current=load_empty(tx,self.date)?.ok_or_else(||mismatch("Empty closure absent at seal CAS"))?;
            if stored_binding(&current)?!=binding {return Err(mismatch("Empty revision/closure changed at seal CAS"));}
            require_no_decisions(tx,self.date)?;require_empty_artifacts(tx,&current)?;
            let existing:Option<(String,Vec<u8>)>=tx.query_row(
                "SELECT seal_identity,seal_canonical FROM g5b_day_seals WHERE business_date=?1 AND cohort_identity=?2 AND revision=?3",
                params![self.date.to_string(),stored.identity,stored.revision],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
            if let Some((id,bytes))=existing {
                if id!=identity || bytes!=canonical {return Err(mismatch("existing Empty seal has a different exact preimage"));}
            } else {
                tx.execute("INSERT INTO g5b_day_seals(seal_identity,business_date,cohort_identity,revision,seal_canonical,seal_sha256,seal_preimage) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![identity,self.date.to_string(),stored.identity,stored.revision,canonical,sha256_hex(&canonical),preimage(EMPTY_SEAL,&canonical)])?;
            }
            let changed=tx.execute("UPDATE g5b_day_heads SET current_seal_identity=?1 WHERE business_date=?2 AND revision=?3 AND cohort_identity=?4 AND artifact_state='Clean' AND current_seal_identity IS NULL",params![identity,self.date.to_string(),stored.revision,stored.identity])?;
            super::super::require_single_cas_update(changed,"Empty current seal CAS")?;
            *expected.borrow_mut()=Some(empty_sql_snapshot(tx,self.date)?);
            Ok(())
        })?;
        self.read_empty_seal()?
            .ok_or_else(|| mismatch("committed Empty seal is not currently verified"))
    }

    #[cfg(test)]
    pub(crate) fn initialize_empty_prospective_for_test(&self, now: DateTime<Utc>) -> Result<()> {
        self.initialize_empty_prospective_at(now, true)
    }
    #[cfg(test)]
    pub(crate) fn prepare_empty_closure_for_test(
        &self,
        now: DateTime<Utc>,
    ) -> Result<PreparedG5bArtifact> {
        self.prepare_empty_closure_at(now, true)
    }
    #[cfg(test)]
    pub(crate) fn close_empty_window_for_test(
        &self,
        now: DateTime<Utc>,
    ) -> Result<VerifiedG5bEmptySeal> {
        self.close_empty_window_at(now, true)
    }

    #[cfg(test)]
    pub(crate) fn install_empty_final_input_fault_for_test(
        &self,
        armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        remaining: usize,
        fault: impl FnOnce() -> Result<()> + 'static,
    ) -> Result<()> {
        self.require_empty_clock(true)?;
        if remaining == 0 {
            return Err(mismatch("invalid scoped Empty fault phase"));
        }
        EMPTY_FILE_READ_FAULT.with(|slot| {
            *slot.borrow_mut() = Some(EmptyFileReadFault {
                namespace: self.namespace_identity,
                date: self.date,
                armed,
                remaining,
                fault: Box::new(fault),
            });
        });
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn attempt_empty_artifact_reopen_for_test(&self) -> Result<()> {
        self.require_empty_clock(true)?;
        self.transaction(|tx| {
            let stored = load_empty(tx, self.date)?
                .ok_or_else(|| mismatch("actual Empty fixture absent"))?;
            prepare_artifact_tx(
                tx,
                self.date,
                &stored.identity,
                ArtifactRole::Archive,
                None,
                b"TEST_CODE_REOPEN_AFTER_REAL_SEAL",
            )
            .map(|_| ())
        })
    }
}
