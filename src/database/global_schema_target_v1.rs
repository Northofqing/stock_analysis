//! An owned exact Catalog6 requalification copy. No approval, receipt, apply,
//! exchange, restore, pool or Paper authority is created by this module.
use super::super::global_schema_catalog_v1::RowsSpecWork;
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::os::unix::fs::FileExt as UnixFileExt;

type Result<T> = std::result::Result<T, GlobalSchemaV1Error>;
const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const DOMAIN: &[u8] = b"stock_analysis.global_schema.requalification_target_record.v1";
const MANAGED: &str = "global-schema-targets";
const OPERATION: &str = "requalification-exact-amended-catalog6-v1";
const TARGET: &str = "stock_analysis.db.target";
const RECORDS: [&str; 4] = [
    "000-intent.json",
    "001-created-target.json",
    "002-copied-target.json",
    "003-target-verified.json",
];
unsafe extern "C" {
    fn geteuid() -> u32;
}
fn fail(detail: &'static str) -> GlobalSchemaV1Error {
    prospective::refusal(detail)
}
fn io<T>(r: std::io::Result<T>) -> Result<T> {
    r.map_err(|_| fail("target descriptor IO failed"))
}
fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or_else(|| fail("target work overflow"))
}
fn mul(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or_else(|| fail("target work overflow"))
}
fn sql(e: rusqlite::Error) -> GlobalSchemaV1Error {
    GlobalSchemaV1Error::SelectionSqlite {
        operation: "verify retained unapproved target",
        source: e,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Limits {
    pub(super) extent: u64,
    pub(super) physical: u64,
    pub(super) metadata: u64,
    pub(super) intent: u64,
    pub(super) event: u64,
    pub(super) records: u64,
    pub(super) journal: u64,
    pub(super) review: u64,
}
impl Limits {
    pub(super) fn production() -> Self {
        Self {
            extent: 16 * GIB,
            physical: 128 * GIB,
            metadata: 16 * MIB,
            intent: MIB,
            event: 64 * 1024,
            records: 2 * MIB,
            journal: 32 * MIB,
            review: MIB,
        }
    }
    fn within(&self, b: &Self) -> bool {
        self.extent <= b.extent
            && self.physical <= b.physical
            && self.metadata <= b.metadata
            && self.intent <= b.intent
            && self.event <= b.event
            && self.records <= b.records
            && self.journal <= b.journal
            && self.review <= b.review
    }
    fn record(&self, slot: usize) -> u64 {
        if slot == 0 { self.intent } else { self.event }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    AfterDirectoryCreated,
    BeforeEmptySync,
    AfterEmptySync,
    AfterEmptyParentSync,
    BeforeCreated,
    AfterCreated,
    AfterCopyChunk,
    AfterCopy,
    BeforeCopySync,
    AfterCopySync,
    AfterCopied,
    BeforeReader,
    AfterReadersClosed,
    BeforeRecordSync(u8),
    AfterRecordSync(u8),
    AfterRecordReadback(u8),
    AfterRecordParentSync(u8),
    BeforeImmutableTail,
}
pub(super) struct Options {
    pub(super) limits: Limits,
    #[cfg(test)]
    pub(super) hook: Option<Box<dyn Fn(Phase) -> Result<()>>>,
    #[cfg(test)]
    pub(super) trace: Option<std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>>,
    // Negative comparator tests only. Even an equal pair cannot issue a cap.
    #[cfg(test)]
    pub(super) comparator_test: bool,
    #[cfg(test)]
    pub(super) reader_fault: Option<fn(&Connection) -> Result<()>>,
}
impl Options {
    pub(super) fn production() -> Self {
        Self {
            limits: Limits::production(),
            #[cfg(test)]
            hook: None,
            #[cfg(test)]
            trace: None,
            #[cfg(test)]
            comparator_test: false,
            #[cfg(test)]
            reader_fault: None,
        }
    }
    fn validate(&self, is_test: bool) -> Result<()> {
        if !self.limits.within(&Limits::production())
            || (!is_test && self.limits != Limits::production())
        {
            return Err(fail("target Test limits may only decrease"));
        }
        #[cfg(test)]
        if !is_test
            && (self.hook.is_some()
                || self.trace.is_some()
                || self.comparator_test
                || self.reader_fault.is_some())
        {
            return Err(fail("target Test seams require actual isolated Test"));
        }
        Ok(())
    }
    fn phase(&self, p: Phase) -> Result<()> {
        #[cfg(test)]
        if let Some(hook) = &self.hook {
            hook(p)?;
        }
        let _ = p;
        Ok(())
    }
    fn event(&self, e: &'static str) {
        #[cfg(test)]
        if let Some(trace) = &self.trace {
            trace.lock().unwrap().push(e);
        }
        let _ = e;
    }
    fn comparator_test(&self) -> bool {
        #[cfg(test)]
        {
            return self.comparator_test;
        }
        #[cfg(not(test))]
        {
            false
        }
    }
}

/// Explicit FD copy/hash/readback and one cumulative target metadata pool.
/// Original validation and both typed sides retain the original RowsWork.
/// This does not claim to measure SQLite VFS cache traffic.
pub(super) struct TargetWork {
    limits: Limits,
    physical: u64,
    metadata: RowsSpecWork,
    journal: u64,
}
impl TargetWork {
    fn new(limits: Limits, source: &rows::OriginalRowsTargetSource) -> Self {
        let metadata = source.new_target_metadata_meter(limits.metadata);
        Self {
            limits,
            physical: 0,
            metadata,
            journal: 0,
        }
    }
    pub(super) fn metadata(&mut self, n: u64) -> Result<()> {
        self.metadata
            .charge(n)
            .map_err(|_| fail("target metadata work exceeded before allocation"))
    }
    pub(super) fn catalog_metadata(&mut self) -> &mut RowsSpecWork {
        &mut self.metadata
    }
    pub(super) fn metadata_used(&self) -> u64 {
        self.metadata.used()
    }
    #[cfg(test)]
    pub(super) fn metadata_diagnostic(&self, stage: &str) {
        eprintln!(
            "target metadata stage={stage} used={} limit={}",
            self.metadata.used(),
            self.limits.metadata
        );
    }
    fn physical(&mut self, n: u64) -> Result<()> {
        self.physical = add(self.physical, n)?;
        if self.physical > self.limits.physical {
            return Err(fail("target physical work exceeded before IO"));
        }
        Ok(())
    }
    fn journal(&mut self, n: u64) -> Result<()> {
        self.journal = add(self.journal, n)?;
        if self.journal > self.limits.journal {
            return Err(fail("target journal work exceeded before IO/encoding"));
        }
        Ok(())
    }
    pub(super) fn encode_metadata(&mut self, value: &impl Serialize, max: u64) -> Result<Vec<u8>> {
        let length = encoded_len(value, max)?;
        self.metadata(length)?;
        prospective::bounded_json(
            value,
            usize::try_from(max).map_err(|_| fail("target encoding limit overflow"))?,
        )
    }
    fn encode_record(&mut self, value: &impl Serialize, max: u64) -> Result<Vec<u8>> {
        let n = encoded_len(value, max)?;
        self.journal(n)?;
        self.encode_metadata(value, max)
    }
}
pub(super) fn encoded_len(value: &impl Serialize, max: u64) -> Result<u64> {
    struct Counter {
        n: u64,
        max: u64,
    }
    impl Write for Counter {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            let n = self
                .n
                .checked_add(b.len() as u64)
                .ok_or_else(|| std::io::Error::other("target encoding overflow"))?;
            if n > self.max {
                return Err(std::io::Error::other("target encoding extent"));
            }
            self.n = n;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut c = Counter { n: 0, max };
    serde_json::to_writer(&mut c, value).map_err(|_| fail("target bounded encoding refused"))?;
    io(c.write_all(b"\n"))?;
    Ok(c.n)
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OriginalBackupBinding {
    pub(super) canonical: String,
    pub(super) length: u64,
    pub(super) sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    device: u64,
    inode: u64,
    links: u64,
    owner: u32,
    mode: u32,
}
fn node(file: &File, directory: bool) -> Result<Node> {
    let m = io(file.metadata())?;
    if m.uid() != unsafe { geteuid() }
        || m.mode() & 0o7777 != if directory { 0o700 } else { 0o600 }
        || (directory && (!m.is_dir() || m.nlink() == 0))
        || (!directory && (!m.is_file() || m.nlink() != 1))
    {
        return Err(fail("target inode type/euid/mode/link mismatch"));
    }
    Ok(Node {
        device: m.dev(),
        inode: m.ino(),
        links: m.nlink(),
        owner: m.uid(),
        mode: m.mode() & 0o7777,
    })
}
fn same_dir(a: &Node, b: &Node) -> bool {
    a.device == b.device
        && a.inode == b.inode
        && a.owner == b.owner
        && a.mode == b.mode
        && a.links > 0
        && b.links > 0
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Anchor {
    main_device: u64,
    main_inode: u64,
    managed: Node,
    operation: Node,
}
impl Anchor {
    fn same(&self, b: &Self) -> bool {
        self.main_device == b.main_device
            && self.main_inode == b.main_inode
            && same_dir(&self.managed, &b.managed)
            && same_dir(&self.operation, &b.operation)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Witness {
    node: Node,
    length: u64,
    sha256: String,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    recipe: String,
    original: OriginalBackupBinding,
    rows: String,
    limits: Limits,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Transition {
    Intent { binding: Intent },
    Created { file: Node },
    Copied { file: Witness },
    Verified { file: Witness },
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u16,
    slot: u8,
    leaf: String,
    anchor: Anchor,
    self_inode: Node,
    intent: Option<String>,
    predecessor: Option<String>,
    transition: Transition,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    sha256: String,
    record: Record,
}
#[derive(Serialize)]
struct EnvelopeRef<'a> {
    sha256: &'a str,
    record: &'a Record,
}
struct Saved {
    record: Record,
    hash: String,
    file: File,
}
fn digest(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(DOMAIN);
    h.update([0]);
    h.update(bytes);
    hex::encode(h.finalize())
}
fn existing(parent: &File, leaf: &str, write: bool) -> Result<Option<File>> {
    match openat_component(
        parent,
        OsStr::new(leaf),
        if write { O_RDWR_FLAG } else { O_RDONLY_FLAG },
        false,
    ) {
        Ok(f) => {
            node(&f, false)?;
            Ok(Some(f))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(fail("target fixed leaf unavailable")),
    }
}
fn create(parent: &File, leaf: &str) -> Result<File> {
    let name =
        component_cstring(OsStr::new(leaf)).map_err(|_| fail("target invalid fixed leaf"))?;
    let fd = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            O_RDWR_FLAG
                | O_CREAT_FLAG
                | O_EXCL_FLAG
                | O_NOFOLLOW_FLAG
                | O_CLOEXEC_FLAG
                | O_NONBLOCK_FLAG,
            0o600u32,
        )
    };
    if fd < 0 {
        return Err(fail("target exclusive create refused"));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    node(&file, false)?;
    Ok(file)
}
struct Workspace {
    directory: PinnedDirectory,
    managed: PinnedDirectory,
    anchor: Anchor,
    fresh: bool,
    records: Vec<Saved>,
}
impl Workspace {
    fn open(ns: &PinnedNamespace, work: &mut TargetWork, options: &Options) -> Result<Self> {
        ns.validate_unchanged()?;
        let main = io(ns.database_parent.file.metadata())?;
        if main.uid() != unsafe { geteuid() } || !main.is_dir() || main.mode() & 0o022 != 0 {
            return Err(fail("target original main parent is not private to euid"));
        }
        let path_bytes = ns.database_parent.path.as_os_str().len() as u64;
        // Reserve the finite namespace traversal/CString routes for all four
        // records and both reader/tail passes before their first allocation.
        // At most 512 validations traverse the two fixed pinned directories.
        work.metadata(add(mul(path_bytes.max(4096), 1024)?, 16384)?)?;
        let managed_path = ns.database_parent.path.join(MANAGED);
        let managed = PinnedDirectory::open_or_create_exact_directory(
            &ns.root,
            &managed_path,
            "target managed parent",
        )?;
        let managed_node = node(&managed.file, true)?;
        if managed_node.device != main.dev() {
            return Err(fail("target parent crosses filesystem"));
        }
        let fresh =
            match openat_component(&managed.file, OsStr::new(OPERATION), O_RDONLY_FLAG, false) {
                Ok(f) => {
                    node(&f, true)?;
                    false
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    io(mkdirat_new_component(&managed.file, OsStr::new(OPERATION)))?;
                    io(managed.file.sync_all())?;
                    true
                }
                Err(_) => return Err(fail("target operation unavailable")),
            };
        let directory = PinnedDirectory::open_or_create_exact_directory(
            &ns.root,
            &managed_path.join(OPERATION),
            "target exact operation",
        )?;
        let operation = node(&directory.file, true)?;
        if operation.device != main.dev() {
            return Err(fail("target operation crosses filesystem"));
        }
        let result = Self {
            directory,
            managed,
            anchor: Anchor {
                main_device: main.dev(),
                main_inode: main.ino(),
                managed: managed_node,
                operation,
            },
            fresh,
            records: Vec::with_capacity(4),
        };
        result.validate_directory()?;
        ns.validate_unchanged()?;
        if fresh {
            options.phase(Phase::AfterDirectoryCreated)?;
        }
        Ok(result)
    }
    fn validate_directory(&self) -> Result<()> {
        self.managed.validate_unchanged()?;
        self.directory.validate_unchanged()?;
        if !same_dir(&node(&self.managed.file, true)?, &self.anchor.managed)
            || !same_dir(&node(&self.directory.file, true)?, &self.anchor.operation)
        {
            return Err(fail("target operation ancestors changed"));
        }
        Ok(())
    }
    fn same_named(&self, leaf: &str, file: &File, expected: &Node) -> Result<()> {
        self.validate_directory()?;
        let named = existing(&self.directory.file, leaf, false)?
            .ok_or_else(|| fail("target recorded leaf missing"))?;
        if node(&named, false)? != *expected
            || node(file, false)? != *expected
            || expected.device != self.anchor.main_device
        {
            return Err(fail("target recorded inode changed"));
        }
        self.validate_directory()
    }
    fn sidecars_absent(&self) -> Result<()> {
        for leaf in [
            "stock_analysis.db.target-wal",
            "stock_analysis.db.target-shm",
            "stock_analysis.db.target-journal",
        ] {
            match openat_component(&self.directory.file, OsStr::new(leaf), O_RDONLY_FLAG, false) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(fail("target unknown sidecar present")),
            }
        }
        Ok(())
    }
    fn read_record(
        &self,
        slot: usize,
        file: &File,
        work: &mut TargetWork,
        first: bool,
    ) -> Result<(Record, String)> {
        let length = io(file.metadata())?.len();
        let max = work.limits.record(slot);
        if length == 0 || length > max {
            return Err(fail("target record scalar extent refused"));
        }
        work.journal(add(length, 1)?)?;
        work.physical(add(length, 1)?)?;
        work.metadata(add(mul(length, 3)?, 4096)?)?;
        let size = usize::try_from(length).map_err(|_| fail("target record extent overflow"))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| fail("target record allocation refused"))?;
        bytes.resize(size, 0);
        io(file.read_exact_at(&mut bytes, 0))?;
        let mut last = [0];
        if io(file.read_at(&mut last, length))? != 0 || io(file.metadata())?.len() != length {
            return Err(fail("target record extent changed"));
        }
        let decoded: Envelope = serde_json::from_slice(&bytes)
            .map_err(|_| fail("target record closed codec refused"))?;
        let r = &decoded.record;
        if r.version != 1
            || r.slot as usize != slot
            || r.leaf != RECORDS[slot]
            || if first {
                !self.anchor.same(&r.anchor)
            } else {
                r.anchor != self.anchor
            }
        {
            return Err(fail("target record slot/anchor mismatch"));
        }
        self.same_named(RECORDS[slot], file, &r.self_inode)?;
        let canonical = work.encode_record(r, max)?;
        work.journal(canonical.len() as u64)?;
        work.metadata(64)?;
        let hash = digest(&canonical);
        if decoded.sha256 != hash {
            return Err(fail("target record domain hash mismatch"));
        }
        let encoded = work.encode_record(
            &EnvelopeRef {
                sha256: &hash,
                record: r,
            },
            max,
        )?;
        if encoded != bytes {
            return Err(fail("target noncanonical record bytes"));
        }
        self.same_named(RECORDS[slot], file, &r.self_inode)?;
        Ok((decoded.record, hash))
    }
    fn load(&mut self, intent: &Intent, work: &mut TargetWork) -> Result<()> {
        self.validate_directory()?;
        self.sidecars_absent()?;
        let mut total = 0;
        let mut gap = false;
        for (slot, leaf) in RECORDS.iter().enumerate() {
            let Some(file) = existing(&self.directory.file, leaf, false)? else {
                gap = true;
                continue;
            };
            if gap || self.fresh {
                return Err(fail("target journal gap or unowned fresh record"));
            }
            total = add(total, io(file.metadata())?.len())?;
            if total > work.limits.records {
                return Err(fail("target persisted record total exceeded"));
            }
            let (record, hash) = self.read_record(slot, &file, work, slot == 0)?;
            if slot == 0 {
                match &record.transition {
                    Transition::Intent { binding } if binding == intent => {}
                    _ => return Err(fail("target original source/recipe/bounds changed")),
                }
                if record.intent.is_some() || record.predecessor.is_some() {
                    return Err(fail("target first record chain mismatch"));
                }
                self.anchor = record.anchor.clone();
            }
            self.records.push(Saved { record, hash, file });
        }
        if !self.fresh && self.records.is_empty() {
            return Err(fail("target existing operation without intent; no adopt"));
        }
        // Re-sync only exact already-recorded facts, never rewrite a record or
        // adopt an unrecorded inode. Unknown partial records failed above.
        for saved in &self.records {
            io(saved.file.sync_all())?;
        }
        io(self.directory.file.sync_all())?;
        self.chain()?;
        self.validate_directory()
    }
    fn chain(&self) -> Result<()> {
        for (slot, s) in self.records.iter().enumerate() {
            if s.record.slot as usize != slot {
                return Err(fail("target noncontiguous chain"));
            }
            if slot > 0
                && (s.record.intent.as_ref() != Some(&self.records[0].hash)
                    || s.record.predecessor.as_ref() != Some(&self.records[slot - 1].hash))
            {
                return Err(fail("target predecessor mismatch"));
            }
            match (&s.record.transition, slot) {
                (Transition::Intent { binding }, 0) if binding.recipe == OPERATION => {}
                (Transition::Created { file }, 1)
                    if file.device == self.anchor.main_device
                        && file.owner == unsafe { geteuid() }
                        && file.links == 1
                        && file.mode == 0o600 => {}
                (Transition::Copied { file }, 2)
                    if Some(&file.node) == self.created()
                        && file.length == self.intent()?.original.length
                        && file.sha256 == self.intent()?.original.sha256 => {}
                (Transition::Verified { file }, 3) if Some(file) == self.copied() => {}
                _ => return Err(fail("target transition binding refused")),
            }
        }
        Ok(())
    }
    fn intent(&self) -> Result<&Intent> {
        match &self
            .records
            .first()
            .ok_or_else(|| fail("target intent missing"))?
            .record
            .transition
        {
            Transition::Intent { binding } => Ok(binding),
            _ => Err(fail("target intent malformed")),
        }
    }
    fn created(&self) -> Option<&Node> {
        match &self.records.get(1)?.record.transition {
            Transition::Created { file } => Some(file),
            _ => None,
        }
    }
    fn copied(&self) -> Option<&Witness> {
        match &self.records.get(2)?.record.transition {
            Transition::Copied { file } => Some(file),
            _ => None,
        }
    }
    fn emit(
        &mut self,
        transition: Transition,
        work: &mut TargetWork,
        options: &Options,
    ) -> Result<()> {
        let slot = self.records.len();
        if slot >= 4 {
            return Err(fail("target fixed four slots exceeded"));
        }
        self.verify_records(work)?;
        self.validate_directory()?;
        self.sidecars_absent()?;
        let total = self
            .records
            .iter()
            .try_fold(0, |n, s| add(n, io(s.file.metadata())?.len()))?;
        let max = work.limits.record(slot).min(
            work.limits
                .records
                .checked_sub(total)
                .ok_or_else(|| fail("target record total exceeded"))?,
        );
        // Bound fixed routing/chain strings before constructing the new record.
        work.metadata(1024)?;
        let file = create(&self.directory.file, RECORDS[slot])?;
        let record = Record {
            version: 1,
            slot: slot as u8,
            leaf: RECORDS[slot].into(),
            anchor: self.anchor.clone(),
            self_inode: node(&file, false)?,
            intent: self.records.first().map(|s| s.hash.clone()),
            predecessor: self.records.last().map(|s| s.hash.clone()),
            transition,
        };
        let canonical = work.encode_record(&record, max)?;
        work.journal(canonical.len() as u64)?;
        work.metadata(64)?;
        let hash = digest(&canonical);
        let bytes = work.encode_record(
            &EnvelopeRef {
                sha256: &hash,
                record: &record,
            },
            max,
        )?;
        work.journal(bytes.len() as u64)?;
        work.physical(bytes.len() as u64)?;
        io(file.write_all_at(&bytes, 0))?;
        options.phase(Phase::BeforeRecordSync(slot as u8))?;
        io(file.sync_all())?;
        options.phase(Phase::AfterRecordSync(slot as u8))?;
        let (readback, actual) = self.read_record(slot, &file, work, false)?;
        if actual != hash || readback != record {
            return Err(fail("target record durable readback differs"));
        }
        options.phase(Phase::AfterRecordReadback(slot as u8))?;
        io(self.directory.file.sync_all())?;
        options.phase(Phase::AfterRecordParentSync(slot as u8))?;
        self.same_named(RECORDS[slot], &file, &record.self_inode)?;
        // The last mutable record hook has finished. Bind the exact bytes
        // again before any later transition may rely on this durable slot.
        let (final_record, final_hash) = self.read_record(slot, &file, work, false)?;
        if final_record != record || final_hash != hash {
            return Err(fail("target record changed after its last hook"));
        }
        io(file.sync_all())?;
        io(self.directory.file.sync_all())?;
        self.records.push(Saved { record, hash, file });
        self.chain()
    }
    fn verify_records(&self, work: &mut TargetWork) -> Result<()> {
        self.validate_directory()?;
        let mut total = 0;
        for (slot, leaf) in RECORDS.iter().enumerate() {
            match (
                existing(&self.directory.file, leaf, false)?,
                self.records.get(slot),
            ) {
                (None, None) => {}
                (Some(file), Some(saved)) => {
                    self.same_named(leaf, &saved.file, &saved.record.self_inode)?;
                    total = add(total, io(file.metadata())?.len())?;
                    if total > work.limits.records {
                        return Err(fail("target final journal extent exceeded"));
                    }
                    let (record, hash) = self.read_record(slot, &file, work, false)?;
                    if record != saved.record || hash != saved.hash {
                        return Err(fail("target retained journal changed"));
                    }
                }
                _ => return Err(fail("target fixed record presence changed")),
            }
        }
        self.chain()
    }
    fn fingerprint(&self, file: &File, work: &mut TargetWork) -> Result<Witness> {
        let actual_node = node(file, false)?;
        self.same_named(TARGET, file, &actual_node)?;
        self.sidecars_absent()?;
        let length = io(file.metadata())?.len();
        if length > work.limits.extent {
            return Err(fail("target extent exceeded"));
        }
        work.physical(add(length, 1)?)?;
        work.metadata(64)?;
        let mut h = Sha256::new();
        let mut offset = 0;
        let mut buffer = [0u8; 65536];
        while offset < length {
            let room = (length - offset).min(buffer.len() as u64) as usize;
            let n = io(file.read_at(&mut buffer[..room], offset))?;
            if n == 0 {
                return Err(fail("target shortened while hashing"));
            }
            h.update(&buffer[..n]);
            offset = add(offset, n as u64)?;
        }
        let mut sentinel = [0];
        if io(file.read_at(&mut sentinel, length))? != 0 || io(file.metadata())?.len() != length {
            return Err(fail("target extent changed while hashing"));
        }
        self.same_named(TARGET, file, &actual_node)?;
        self.sidecars_absent()?;
        Ok(Witness {
            node: actual_node,
            length,
            sha256: hex::encode(h.finalize()),
        })
    }
    fn retained_target(&self, write: bool) -> Result<File> {
        let file = existing(&self.directory.file, TARGET, write)?
            .ok_or_else(|| fail("target recorded file missing"))?;
        self.same_named(
            TARGET,
            &file,
            self.created()
                .ok_or_else(|| fail("target Created witness missing"))?,
        )?;
        Ok(file)
    }
    fn ensure_copy(
        &mut self,
        source: &mut rows::OriginalRowsTargetSource,
        work: &mut TargetWork,
        options: &Options,
    ) -> Result<()> {
        self.sidecars_absent()?;
        if let Some(expected) = self.copied() {
            let file = self.retained_target(false)?;
            if self.fingerprint(&file, work)? != *expected {
                return Err(fail("target recorded copy bytes changed"));
            }
            return Ok(());
        }
        let file = if self.created().is_some() {
            let file = self.retained_target(true)?;
            if io(file.metadata())?.len() != 0 {
                return Err(fail("target unknown partial copy; no truncate/retry"));
            }
            file
        } else {
            if existing(&self.directory.file, TARGET, false)?.is_some() {
                return Err(fail("target unrecorded inode; no adopt"));
            }
            let file = create(&self.directory.file, TARGET)?;
            let n = node(&file, false)?;
            options.phase(Phase::BeforeEmptySync)?;
            io(file.sync_all())?;
            options.phase(Phase::AfterEmptySync)?;
            io(self.directory.file.sync_all())?;
            options.phase(Phase::AfterEmptyParentSync)?;
            self.same_named(TARGET, &file, &n)?;
            if io(file.metadata())?.len() != 0 {
                return Err(fail("target empty inode changed"));
            }
            options.phase(Phase::BeforeCreated)?;
            self.same_named(TARGET, &file, &n)?;
            if io(file.metadata())?.len() != 0 {
                return Err(fail("target empty inode changed before Created"));
            }
            self.emit(Transition::Created { file: n }, work, options)?;
            options.phase(Phase::AfterCreated)?;
            file
        };
        self.verify_records(work)?;
        self.same_named(TARGET, &file, self.created().unwrap())?;
        if io(file.metadata())?.len() != 0 {
            return Err(fail("target Created is not exact empty inode"));
        }
        let mut created = CreatedTarget { file, options };
        source.with_copy_origin(work, |loan, work| loan.copy_to(&mut created, work))?;
        options.phase(Phase::AfterCopy)?;
        self.same_named(TARGET, &created.file, self.created().unwrap())?;
        options.phase(Phase::BeforeCopySync)?;
        io(created.file.sync_all())?;
        io(self.directory.file.sync_all())?;
        options.phase(Phase::AfterCopySync)?;
        self.sidecars_absent()?;
        let witness = self.fingerprint(&created.file, work)?;
        if witness.length != self.intent()?.original.length
            || witness.sha256 != self.intent()?.original.sha256
        {
            return Err(fail("target copied bytes mismatch"));
        }
        // This exact-copy recipe opens no write connection and creates no sidecar.
        self.emit(Transition::Copied { file: witness }, work, options)?;
        options.phase(Phase::AfterCopied)?;
        Ok(())
    }
    fn immutable_tail(&self, work: &mut TargetWork) -> Result<()> {
        self.verify_records(work)?;
        self.sidecars_absent()?;
        let file = self.retained_target(false)?;
        if Some(&self.fingerprint(&file, work)?) != self.copied() {
            return Err(fail("target immutable tail bytes changed"));
        }
        self.verify_records(work)?;
        self.sidecars_absent()?;
        self.validate_directory()
    }
}
/// Only ensure_copy can construct this sink, after durable Created readback.
pub(super) struct CreatedTarget<'options> {
    file: File,
    options: &'options Options,
}
impl CreatedTarget<'_> {
    pub(super) fn copy_from_original(
        &mut self,
        input: &File,
        length: u64,
        expected: &str,
        work: &mut TargetWork,
    ) -> Result<()> {
        if length > work.limits.extent || io(self.file.metadata())?.len() != 0 {
            return Err(fail("target copy requires bounded exact empty inode"));
        }
        work.physical(add(mul(length, 2)?, 1)?)?;
        work.metadata(64)?;
        let before = FileIdentity::from_metadata(&io(input.metadata())?);
        if before.length != length {
            return Err(fail("target original Copied extent changed"));
        }
        let mut offset = 0;
        let mut h = Sha256::new();
        let mut buffer = [0u8; 65536];
        while offset < length {
            let room = (length - offset).min(buffer.len() as u64) as usize;
            let n = io(input.read_at(&mut buffer[..room], offset))?;
            if n == 0 {
                return Err(fail("target original Copied truncated"));
            }
            io(self.file.write_all_at(&buffer[..n], offset))?;
            h.update(&buffer[..n]);
            offset = add(offset, n as u64)?;
            self.options.phase(Phase::AfterCopyChunk)?;
        }
        let mut sentinel = [0];
        if io(input.read_at(&mut sentinel, length))? != 0
            || FileIdentity::from_metadata(&io(input.metadata())?) != before
            || hex::encode(h.finalize()) != expected
        {
            return Err(fail("target actual copy original changed"));
        }
        if io(self.file.metadata())?.len() != length {
            return Err(fail("target copied extent differs"));
        }
        Ok(())
    }
}
fn verify_integrity(connection: &Connection) -> Result<()> {
    // Successful integrity_check(1) still checks the whole database, while
    // refusing the first error without allocating attacker-sized diagnostics.
    let mut statement = connection
        .prepare("PRAGMA main.integrity_check(1)")
        .map_err(sql)?;
    let mut cursor = statement.query([]).map_err(sql)?;
    let Some(row) = cursor.next().map_err(sql)? else {
        return Err(fail("target integrity result missing"));
    };
    if !matches!(row.get_ref(0).map_err(sql)?, rusqlite::types::ValueRef::Text(b) if b == b"ok") {
        return Err(fail("target integrity check refused"));
    }
    if cursor.next().map_err(sql)?.is_some() {
        return Err(fail("target integrity has extra result"));
    }
    let mut statement = connection
        .prepare("PRAGMA main.foreign_key_check")
        .map_err(sql)?;
    if statement
        .query([])
        .map_err(sql)?
        .next()
        .map_err(sql)?
        .is_some()
    {
        return Err(fail("target foreign key violation"));
    }
    Ok(())
}
pub(super) struct RetainedTargetReader<'loan> {
    connection: &'loan Connection,
    options: &'loan Options,
}
impl RetainedTargetReader<'_> {
    pub(super) fn connection(&self) -> &Connection {
        self.connection
    }
    pub(super) fn comparator_entered(&self) {
        self.options.event("target_comparator_entered");
    }
}
/// Retains the original exclusive owner through both fixed target pairs and
/// final finite encoding. It is deliberately non-Clone and non-Deserialize.
pub(super) struct VerifiedUnapprovedRequalificationTarget {
    source: rows::OriginalRowsTargetSource,
    workspace: Workspace,
    work: TargetWork,
    options: Options,
}
pub(super) fn prepare(
    original: rows::VerifiedUnapprovedOriginalRowsBackup,
    options: Options,
) -> Result<VerifiedUnapprovedRequalificationTarget> {
    let mut source = original.into_target_source()?;
    options.validate(source.is_test())?;
    let mut work = TargetWork::new(options.limits.clone(), &source);
    let rows = source.original_binding(&mut work)?;
    let original = source.with_copy_origin(&mut work, |loan, work| loan.binding(work))?;
    if original.length > work.limits.extent {
        return Err(fail("target input extent exceeds limit before creation"));
    }
    work.metadata(256)?;
    let intent = Intent {
        recipe: source.recipe().into(),
        original,
        rows,
        limits: options.limits.clone(),
    };
    encoded_len(&intent, work.limits.intent)?;
    let mut workspace = source.with_namespace(|ns| Workspace::open(ns, &mut work, &options))?;
    workspace.load(&intent, &mut work)?;
    if workspace.records.is_empty() {
        workspace.emit(Transition::Intent { binding: intent }, &mut work, &options)?;
    }
    workspace.ensure_copy(&mut source, &mut work, &options)?;
    let mut result = VerifiedUnapprovedRequalificationTarget {
        source,
        workspace,
        work,
        options,
    };
    result.verify_final(2)?;
    Ok(result)
}
impl VerifiedUnapprovedRequalificationTarget {
    fn verify_final(&mut self, required_streams: u64) -> Result<()> {
        self.workspace.verify_records(&mut self.work)?;
        self.workspace.sidecars_absent()?;
        self.options.phase(Phase::BeforeReader)?;
        let file = self.workspace.retained_target(false)?;
        if !self.options.comparator_test()
            && Some(&self.workspace.fingerprint(&file, &mut self.work)?) != self.workspace.copied()
        {
            return Err(fail("target reader physical input changed"));
        }
        let parent_bytes = self.workspace.directory.path.as_os_str().len() as u64;
        self.work
            .metadata(add(mul(parent_bytes.max(4096), 8)?, 1024)?)?;
        let route = sqlite_open_route_from_retained_parent(
            &self.workspace.directory.file,
            OsStr::new(TARGET),
        )
        .map_err(|_| fail("target retained route unavailable"))?;
        let mut uri = String::from("file:");
        for b in route.as_os_str().as_bytes() {
            use std::fmt::Write;
            write!(&mut uri, "%{b:02X}").map_err(|_| fail("target URI encoding"))?;
        }
        uri.push_str("?mode=ro&immutable=1");
        let connection = Connection::open_with_flags(
            &uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(sql)?;
        let result = (|| {
            connection
                .execute_batch("PRAGMA query_only=ON")
                .map_err(sql)?;
            if !connection
                .is_readonly(rusqlite::DatabaseName::Main)
                .map_err(sql)?
            {
                return Err(fail("target reader is writable"));
            }
            self.workspace
                .same_named(TARGET, &file, self.workspace.created().unwrap())?;
            self.workspace.sidecars_absent()?;
            #[cfg(test)]
            if let Some(fault) = self.options.reader_fault {
                fault(&connection)?;
            }
            self.work.metadata(256)?;
            verify_integrity(&connection)?;
            self.source.compare_owned_target(
                &RetainedTargetReader {
                    connection: &connection,
                    options: &self.options,
                },
                &mut self.work,
            )
        })();
        connection.close().map_err(|(_, e)| sql(e))?;
        self.options.event("target_readers_closed");
        result?;
        #[cfg(test)]
        if self.options.reader_fault.is_some() {
            return Err(fail(
                "target reader-fault Test path cannot issue capability",
            ));
        }
        if self.options.comparator_test() {
            return Err(fail(
                "target comparator-only Test path cannot issue capability",
            ));
        }
        self.options.phase(Phase::AfterReadersClosed)?;
        self.workspace
            .same_named(TARGET, &file, self.workspace.created().unwrap())?;
        self.workspace.sidecars_absent()?;
        if sqlite_open_route_from_retained_parent(
            &self.workspace.directory.file,
            OsStr::new(TARGET),
        )
        .map_err(|_| fail("target route disappeared"))?
            != route
        {
            return Err(fail("target reader route changed"));
        }
        if self.source.observation().target_streams != required_streams {
            return Err(fail("target fixed pair count differs"));
        }
        if self.workspace.records.len() == 3 {
            self.work.metadata(256)?;
            let file = self
                .workspace
                .copied()
                .ok_or_else(|| fail("target Copied witness missing"))?
                .clone();
            self.workspace
                .emit(Transition::Verified { file }, &mut self.work, &self.options)?;
        }
        if self.workspace.records.len() != 4 {
            return Err(fail("target verified record missing"));
        }
        self.options.phase(Phase::BeforeImmutableTail)?;
        self.options.event("target_mutable_hooks_finished");
        // No phase callback or record write occurs below this line.
        self.workspace.immutable_tail(&mut self.work)?;
        self.source.validate_without_hooks()?;
        self.workspace.immutable_tail(&mut self.work)?;
        self.source.with_namespace(|ns| {
            ns.validate_unchanged()?;
            let main = io(ns.database_parent.file.metadata())?;
            if main.dev() != self.workspace.anchor.main_device
                || main.ino() != self.workspace.anchor.main_inode
                || main.uid() != unsafe { geteuid() }
                || main.mode() & 0o022 != 0
            {
                return Err(fail(
                    "target original main parent owner/mode/identity changed",
                ));
            }
            Ok(())
        })?;
        self.options.event("target_immutable_tail_complete");
        Ok(())
    }
    pub(super) fn render_unapproved(mut self) -> Result<String> {
        self.verify_final(4)?;
        #[derive(Serialize)]
        struct Review<'a, E: Serialize> {
            version: u16,
            domain: &'static str,
            capability_scope: &'static str,
            recipe: &'a str,
            target_comparisons: E,
            original_streams: u64,
            target_streams: u64,
            row_observations: u64,
            typed_observation_bytes: u64,
            metadata_work: u64,
            target_physical_work: u64,
            target_metadata_work: u64,
            target_journal_work: u64,
            intent: &'a str,
            terminal: &'a str,
            target: &'a Witness,
            row_preservation_proof: bool,
            approval: &'static str,
            maintenance_receipt: &'static str,
            exchange: &'static str,
            apply_supported: bool,
        }
        let rows = self.source.observation();
        let review = Review {
            version: 1,
            domain: "stock_analysis.global_schema.requalification_target_record.v1",
            capability_scope: "verified_unapproved_exact_catalog6_requalification_target",
            recipe: self.source.recipe(),
            target_comparisons: self.source.pair_evidence(),
            original_streams: rows.original_streams,
            target_streams: rows.target_streams,
            row_observations: rows.row_observations,
            typed_observation_bytes: rows.typed_observation_bytes,
            metadata_work: rows.metadata_work,
            target_physical_work: self.work.physical,
            target_metadata_work: self.work.metadata_used(),
            target_journal_work: self.work.journal,
            intent: &self.workspace.records[0].hash,
            terminal: &self.workspace.records[3].hash,
            target: self
                .workspace
                .copied()
                .ok_or_else(|| fail("target final witness missing"))?,
            row_preservation_proof: true,
            approval: "not_granted",
            maintenance_receipt: "not_created",
            exchange: "not_implemented",
            apply_supported: false,
        };
        // Pure checked encoding after the complete immutable tail. All owners
        // (including the original exclusive lease) are still retained here.
        let bytes = self
            .work
            .encode_metadata(&review, self.options.limits.review)?;
        String::from_utf8(bytes).map_err(|_| fail("target review encoding refused"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Resource counters only: no original capability, reader or target owner.
    fn test_work(limits: Limits) -> TargetWork {
        TargetWork {
            metadata: RowsSpecWork::new(limits.metadata, 0, 0),
            limits,
            physical: 0,
            journal: 0,
        }
    }
    #[test]
    fn target_work_checked_overflow_and_repeated_encoding_remain_cumulative() {
        let mut work = test_work(Limits::production());
        work.physical = u64::MAX;
        assert!(work.physical(1).is_err());
        let mut work = test_work(Limits::production());
        assert!(work.catalog_metadata().charge(u64::MAX).is_err());
        assert!(work.metadata(1).is_err());
        let mut work = test_work(Limits::production());
        work.journal = u64::MAX;
        assert!(work.journal(1).is_err());
        let value = "actual bounded bytes";
        let length = encoded_len(&value, 1024).unwrap();
        let mut limits = Limits::production();
        limits.metadata = length;
        let mut work = test_work(limits);
        assert_eq!(
            work.encode_metadata(&value, 1024).unwrap().len() as u64,
            length
        );
        assert!(work.encode_metadata(&value, 1024).is_err());
        assert!(encoded_len(&value, length - 1).is_err());
    }
    #[test]
    fn target_catalog_and_encoding_share_one_cumulative_metadata_limit() {
        let value = "shared target allocation";
        let encoded = encoded_len(&value, 1024).unwrap();
        let mut limits = Limits::production();
        limits.metadata = encoded + 12;
        let mut work = test_work(limits.clone());
        work.catalog_metadata().charge(5).unwrap();
        work.metadata(3).unwrap();
        assert_eq!(
            work.encode_metadata(&value, 1024).unwrap().len() as u64,
            encoded
        );
        work.catalog_metadata().charge(4).unwrap();
        assert_eq!(work.metadata_used(), limits.metadata);
        assert!(work.encode_metadata(&value, 1024).is_err());
        assert_eq!(work.metadata_used(), limits.metadata + encoded);

        let mut work = test_work(limits.clone());
        work.encode_metadata(&value, 1024).unwrap();
        work.metadata(7).unwrap();
        work.catalog_metadata().charge(5).unwrap();
        assert_eq!(work.metadata_used(), limits.metadata);
        assert!(work.catalog_metadata().charge(1).is_err());
        assert_eq!(work.metadata_used(), limits.metadata + 1);
        assert!(work.metadata(1).is_err());
        assert_eq!(work.metadata_used(), limits.metadata + 2);
    }
    #[test]
    fn target_production_limits_and_test_seams_are_not_increasable_or_portable() {
        let mut options = Options::production();
        options.limits.extent += 1;
        assert!(options.validate(true).is_err());
        let mut options = Options::production();
        options.limits.metadata -= 1;
        assert!(options.validate(false).is_err());
        assert!(options.validate(true).is_ok());
        let mut options = Options::production();
        options.comparator_test = true;
        assert!(options.validate(false).is_err());
    }
}
