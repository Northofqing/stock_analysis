//! Fixed, unapproved byte backup. This local journal is not apply/WORM authority.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::os::unix::fs::FileExt;

const DOMAIN: &[u8] = b"stock_analysis.global_schema.byte_backup_record.v1";
const RECORDS: [&str; 6] = [
    "000-intent.json",
    "001-created-db.json",
    "002-copied-db.json",
    "003-created-audit.json",
    "004-copied-audit.json",
    "005-backup-verified.json",
];
const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;
unsafe extern "C" {
    fn geteuid() -> u32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Role {
    Database,
    Audit,
}
impl Role {
    fn leaf(self) -> &'static str {
        match self {
            Self::Database => "stock_analysis.db.backup",
            Self::Audit => "selection-audit.jsonl.backup",
        }
    }
    fn slots(self) -> (usize, usize) {
        match self {
            Self::Database => (1, 2),
            Self::Audit => (3, 4),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    AfterDirectoryCreated,
    BeforeEmptyRoleSync(Role),
    AfterEmptyRoleSync(Role),
    AfterEmptyRoleParentSync(Role),
    BeforeCreated(Role),
    AfterCreated(Role),
    AfterCopyChunk(Role),
    BeforeRoleSync(Role),
    AfterRoleSync(Role),
    AfterCopied(Role),
    BeforeRecordSync(u8),
    AfterRecordSync(u8),
    AfterRecordReadback(u8),
    AfterRecordParentSync(u8),
    AfterOutputReads,
    AfterTerminalSync,
    BeforeRender,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Limits {
    pub(super) main_extent: u64,
    pub(super) audit_extent: u64,
    pub(super) role_total: u64,
    pub(super) copy_work: u64,
    pub(super) common_work: u64,
    pub(super) intent_bytes: u64,
    pub(super) event_bytes: u64,
    pub(super) record_total: u64,
    pub(super) journal_work: u64,
    pub(super) record_count: usize,
}
impl Limits {
    pub(super) fn production() -> Self {
        Self {
            main_extent: 16 * GIB,
            audit_extent: 32 * MIB,
            role_total: 16 * GIB + 32 * MIB,
            copy_work: 64 * GIB + 128 * MIB,
            common_work: 129 * GIB,
            intent_bytes: MIB,
            event_bytes: 64 * 1024,
            record_total: 2 * MIB,
            journal_work: 32 * MIB,
            record_count: 6,
        }
    }
    fn bounded_by(&self, max: &Self) -> bool {
        self.main_extent <= max.main_extent
            && self.audit_extent <= max.audit_extent
            && self.role_total <= max.role_total
            && self.copy_work <= max.copy_work
            && self.common_work <= max.common_work
            && self.intent_bytes <= max.intent_bytes
            && self.event_bytes <= max.event_bytes
            && self.record_total <= max.record_total
            && self.journal_work <= max.journal_work
            && self.record_count <= max.record_count
    }
    fn record_limit(&self, slot: usize) -> u64 {
        if slot == 0 {
            self.intent_bytes
        } else {
            self.event_bytes
        }
    }
}
pub(super) struct Settings {
    pub(super) limits: Limits,
    #[cfg(test)]
    pub(super) hook: Option<Box<dyn Fn(Phase) -> Result<(), GlobalSchemaV1Error>>>,
}
impl Settings {
    fn phase(&self, phase: Phase) -> Result<(), GlobalSchemaV1Error> {
        #[cfg(test)]
        if let Some(hook) = &self.hook {
            return hook(phase);
        }
        let _ = phase;
        Ok(())
    }
}
pub(super) struct Options {
    pub(super) source: prospective::Options,
    pub(super) settings: Settings,
}
impl Options {
    pub(super) fn production() -> Self {
        Self {
            source: prospective::Options::production(),
            settings: Settings {
                limits: Limits::production(),
                #[cfg(test)]
                hook: None,
            },
        }
    }
    pub(super) fn bind_common_budget(&mut self) -> Result<(), GlobalSchemaV1Error> {
        // Conservatively reserve the entire narrower copy/journal/audit caps
        // before the first source read. All individual counters together can
        // never exceed the common ceiling, including later original readers.
        let reserved = self
            .settings
            .limits
            .copy_work
            .checked_add(self.settings.limits.journal_work)
            .and_then(|n| n.checked_add(self.source.audit_limits.max_total_scan_bytes))
            .ok_or_else(|| refuse("backup common reservation overflow"))?;
        let available = self
            .settings
            .limits
            .common_work
            .checked_sub(reserved)
            .ok_or_else(|| refuse("backup common reservation exceeded"))?;
        self.source.max_total_hash_bytes = self.source.max_total_hash_bytes.min(available);
        Ok(())
    }
    pub(super) fn validate_mode(&self, mode: BoundMode) -> Result<(), GlobalSchemaV1Error> {
        self.source.validate_mode(mode)?;
        if !self.settings.limits.bounded_by(&Limits::production()) {
            return Err(refuse("backup Test limits may only decrease hard bounds"));
        }
        #[cfg(test)]
        if mode != BoundMode::Test
            && (self.settings.hook.is_some() || self.settings.limits != Limits::production())
        {
            return Err(refuse(
                "backup Test seam requires actual isolated Test namespace",
            ));
        }
        Ok(())
    }
}
fn refuse(detail: &'static str) -> GlobalSchemaV1Error {
    prospective::refusal(detail)
}
fn checked_io<T>(result: std::io::Result<T>) -> Result<T, GlobalSchemaV1Error> {
    result.map_err(|_| refuse("backup descriptor IO failed"))
}
fn uid() -> u32 {
    unsafe { geteuid() }
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
fn private_node(file: &File, directory: bool) -> Result<Node, GlobalSchemaV1Error> {
    let m = checked_io(file.metadata())?;
    if m.uid() != uid()
        || m.mode() & 0o7777 != if directory { 0o700 } else { 0o600 }
        || (directory && (!m.is_dir() || m.nlink() == 0))
        || (!directory && (!m.is_file() || m.nlink() != 1))
    {
        return Err(refuse("backup managed inode owner/mode/type/link mismatch"));
    }
    Ok(Node {
        device: m.dev(),
        inode: m.ino(),
        links: m.nlink(),
        owner: m.uid(),
        mode: m.mode() & 0o7777,
    })
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DataDirectory {
    device: u64,
    inode: u64,
}
fn data_identity(file: &File) -> Result<DataDirectory, GlobalSchemaV1Error> {
    let m = checked_io(file.metadata())?;
    if !m.is_dir() || m.nlink() == 0 {
        return Err(refuse("backup data parent is not retained"));
    }
    Ok(DataDirectory {
        device: m.dev(),
        inode: m.ino(),
    })
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NamespaceAnchor {
    data: DataDirectory,
    managed: Node,
    operation: Node,
}
// Directory links are a historical extent observation: adding this
// operation's own regular leaves changes nlink on supported filesystems.
// Reopened/retained identity, private owner/mode and positive links still
// have to agree; regular-file Node equality remains exact with links == 1.
fn same_directory_object(left: &Node, right: &Node) -> bool {
    left.links > 0
        && right.links > 0
        && left.device == right.device
        && left.inode == right.inode
        && left.owner == right.owner
        && left.mode == right.mode
}
impl NamespaceAnchor {
    fn same_directory_objects(&self, other: &Self) -> bool {
        self.data == other.data
            && same_directory_object(&self.managed, &other.managed)
            && same_directory_object(&self.operation, &other.operation)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileWitness {
    node: Node,
    length: u64,
    sha256: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Transition {
    Intent {
        source_canonical: String,
        hard_limits: Limits,
        approval: String,
    },
    Created {
        role: Role,
        file: Node,
    },
    Copied {
        role: Role,
        file: FileWitness,
    },
    BackupVerified {
        database: FileWitness,
        audit: Option<FileWitness>,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u16,
    slot: u8,
    leaf: String,
    directory: NamespaceAnchor,
    self_inode: Node,
    intent_hash: Option<String>,
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
struct SavedRecord {
    slot: usize,
    hash: String,
    record: Record,
    file: File,
}
#[derive(Default)]
struct Work {
    copy: u64,
    journal: u64,
}
impl Work {
    fn journal(&mut self, bytes: u64, limits: &Limits) -> Result<(), GlobalSchemaV1Error> {
        self.journal = self
            .journal
            .checked_add(bytes)
            .ok_or_else(|| refuse("backup journal work overflow"))?;
        if self.journal > limits.journal_work {
            return Err(refuse("backup shared journal work exceeded"));
        }
        Ok(())
    }
    fn common(
        &self,
        source: &prospective::Pending,
        options: &prospective::Options,
        limits: &Limits,
    ) -> Result<(), GlobalSchemaV1Error> {
        // Reserve the entire original bounded audit-session allowance: no
        // audit API change or false claim of observing its internal counter.
        let total = self
            .copy
            .checked_add(self.journal)
            .and_then(|n| n.checked_add(source.backup_hash_work()))
            .and_then(|n| n.checked_add(options.audit_limits.max_total_scan_bytes))
            .ok_or_else(|| refuse("backup common IO work overflow"))?;
        if total > limits.common_work {
            return Err(refuse("backup common IO work exceeded"));
        }
        Ok(())
    }
    fn reserve_copy(
        &mut self,
        length: u64,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        limits: &Limits,
    ) -> Result<(), GlobalSchemaV1Error> {
        let both = length
            .checked_mul(2)
            .ok_or_else(|| refuse("backup copy work overflow"))?;
        self.copy = self
            .copy
            .checked_add(both)
            .ok_or_else(|| refuse("backup copy work overflow"))?;
        if self.copy > limits.copy_work {
            return Err(refuse("backup shared copy work exceeded"));
        }
        source.reserve_backup_reads(length, options)?;
        self.common(source, options, limits)
    }
}
fn new_read_write(parent: &File, leaf: &str) -> Result<File, GlobalSchemaV1Error> {
    let name =
        component_cstring(OsStr::new(leaf)).map_err(|_| refuse("backup fixed leaf invalid"))?;
    let fd = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            O_RDWR_FLAG
                | O_CREAT_FLAG
                | O_EXCL_FLAG
                | O_NOFOLLOW_FLAG
                | O_NONBLOCK_FLAG
                | O_CLOEXEC_FLAG,
            0o600_u32,
        )
    };
    if fd < 0 {
        return Err(refuse("backup no-clobber create refused"));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    private_node(&file, false)?;
    Ok(file)
}
fn existing(
    parent: &File,
    leaf: &str,
    writable: bool,
) -> Result<Option<File>, GlobalSchemaV1Error> {
    match openat_component(
        parent,
        OsStr::new(leaf),
        if writable { O_RDWR_FLAG } else { O_RDONLY_FLAG },
        false,
    ) {
        Ok(f) => {
            private_node(&f, false)?;
            Ok(Some(f))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(refuse("backup existing exact leaf unavailable")),
    }
}
fn domain_hash(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(DOMAIN);
    digest.update([0]);
    digest.update(bytes);
    hex::encode(digest.finalize())
}
fn encode(
    value: &impl Serialize,
    limit: u64,
    work: &mut Work,
    limits: &Limits,
) -> Result<Vec<u8>, GlobalSchemaV1Error> {
    let available = limits
        .journal_work
        .checked_sub(work.journal)
        .ok_or_else(|| refuse("backup journal work exceeded"))?;
    let cap = usize::try_from(limit.min(available))
        .map_err(|_| refuse("backup journal extent overflow"))?;
    let bytes = prospective::bounded_json(value, cap)?;
    work.journal(bytes.len() as u64, limits)?;
    Ok(bytes)
}
fn read_bounded(
    file: &File,
    cap: u64,
    work: &mut Work,
    limits: &Limits,
) -> Result<Vec<u8>, GlobalSchemaV1Error> {
    let node = private_node(file, false)?;
    let length = checked_io(file.metadata())?.len();
    if length == 0 || length > cap {
        return Err(refuse("backup journal record extent invalid"));
    }
    // Charge before allocation/read; never refund a failed reservation.
    work.journal(
        length
            .checked_add(1)
            .ok_or_else(|| refuse("backup journal extent overflow"))?,
        limits,
    )?;
    let size = usize::try_from(length).map_err(|_| refuse("backup journal extent overflow"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| refuse("backup journal allocation refused"))?;
    let mut chunk = [0_u8; 8192];
    let mut offset = 0_u64;
    while offset < length {
        let room = (length - offset).min(chunk.len() as u64) as usize;
        let n = checked_io(file.read_at(&mut chunk[..room], offset))?;
        if n == 0 {
            return Err(refuse("backup journal truncated"));
        }
        bytes.extend_from_slice(&chunk[..n]);
        offset += n as u64;
    }
    let mut sentinel = [0_u8; 1];
    if checked_io(file.read_at(&mut sentinel, length))? != 0
        || checked_io(file.metadata())?.len() != length
        || private_node(file, false)? != node
    {
        return Err(refuse("backup journal changed during read"));
    }
    Ok(bytes)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnchorBinding {
    Fresh,
    AwaitingStoredIntent,
    StoredIntent,
}
enum RecordDirectoryCheck {
    ExactBound,
    FirstStoredIntent,
}
pub(super) struct Workspace {
    directory: PinnedDirectory,
    data_parent: PinnedDirectory,
    managed_parent: PinnedDirectory,
    node: Node,
    anchor: NamespaceAnchor,
    anchor_binding: AnchorBinding,
    fresh: bool,
    records: Vec<SavedRecord>,
    work: Work,
}
impl Workspace {
    pub(super) fn open(
        maintenance: &ExclusiveGlobalSchemaMaintenanceLease,
        settings: &Settings,
    ) -> Result<Self, GlobalSchemaV1Error> {
        let root = &maintenance.namespace.root;
        let data = root.path.join("data");
        let data_parent = PinnedDirectory::open_or_create_exact_directory(
            root,
            &data,
            "backup fixed data parent",
        )?;
        let managed = data.join("global-schema-operations");
        let parent = PinnedDirectory::open_or_create_exact_directory(
            root,
            &managed,
            "backup managed parent",
        )?;
        private_node(&parent.file, true)?;
        let path = managed.join("selection-byte-backup-v1");
        let leaf = OsStr::new("selection-byte-backup-v1");
        let fresh = match openat_component(&parent.file, leaf, O_RDONLY_FLAG, false) {
            Ok(file) => {
                private_node(&file, true)?;
                false
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                checked_io(mkdirat_new_component(&parent.file, leaf))?;
                checked_io(parent.file.sync_all())?;
                true
            }
            Err(_) => return Err(refuse("backup managed operation directory unavailable")),
        };
        let directory = PinnedDirectory::open_or_create_exact_directory(
            root,
            &path,
            "backup exact operation directory",
        )?;
        let node = private_node(&directory.file, true)?;
        let anchor = NamespaceAnchor {
            data: data_identity(&data_parent.file)?,
            managed: private_node(&parent.file, true)?,
            operation: node.clone(),
        };
        data_parent.validate_unchanged()?;
        parent.validate_unchanged()?;
        directory.validate_unchanged()?;
        maintenance.namespace.validate_unchanged()?;
        if fresh {
            settings.phase(Phase::AfterDirectoryCreated)?;
        }
        Ok(Self {
            directory,
            data_parent,
            managed_parent: parent,
            node,
            anchor,
            anchor_binding: if fresh {
                AnchorBinding::Fresh
            } else {
                AnchorBinding::AwaitingStoredIntent
            },
            fresh,
            records: Vec::new(),
            work: Work::default(),
        })
    }
    fn validate_directory(&self) -> Result<(), GlobalSchemaV1Error> {
        self.data_parent.validate_unchanged()?;
        self.managed_parent.validate_unchanged()?;
        self.directory.validate_unchanged()?;
        if data_identity(&self.data_parent.file)? != self.anchor.data
            || !same_directory_object(
                &private_node(&self.managed_parent.file, true)?,
                &self.anchor.managed,
            )
        {
            return Err(refuse("backup recorded operation ancestors changed"));
        }
        if !same_directory_object(&private_node(&self.directory.file, true)?, &self.node) {
            return Err(refuse("backup operation directory changed"));
        }
        Ok(())
    }
    fn same_named(&self, leaf: &str, file: &File, node: &Node) -> Result<(), GlobalSchemaV1Error> {
        self.validate_directory()?;
        let named = existing(&self.directory.file, leaf, false)?
            .ok_or_else(|| refuse("backup recorded leaf missing"))?;
        if private_node(&named, false)? != *node || private_node(file, false)? != *node {
            return Err(refuse("backup recorded named inode changed"));
        }
        self.validate_directory()
    }
    fn load(&mut self, settings: &Settings, binding: &str) -> Result<(), GlobalSchemaV1Error> {
        self.validate_directory()?;
        let mut pinned = Vec::new();
        let mut total = 0_u64;
        for (slot, leaf) in RECORDS.iter().enumerate() {
            if let Some(file) = existing(&self.directory.file, leaf, false)? {
                let length = checked_io(file.metadata())?.len();
                if length == 0 || length > settings.limits.record_limit(slot) {
                    return Err(refuse("backup journal scalar extent invalid"));
                }
                total = total
                    .checked_add(length)
                    .ok_or_else(|| refuse("backup journal total overflow"))?;
                if total > settings.limits.record_total
                    || pinned.len() >= settings.limits.record_count
                {
                    return Err(refuse("backup journal scalar total/count exceeded"));
                }
                pinned.push((slot, file));
            }
        }
        if self.fresh && !pinned.is_empty() {
            return Err(refuse("backup fresh operation gained an unowned journal"));
        }
        if !pinned.is_empty() && pinned[0].0 != 0 {
            return Err(refuse("backup stored operation has no first intent"));
        }
        let mut records = Vec::new();
        for (slot, file) in pinned {
            let (record, hash) = if records.is_empty() && slot == 0 {
                self.load_first_stored_intent(&file, settings, binding)?
            } else {
                self.read_record(slot, &file, settings)?
            };
            records.push(SavedRecord {
                slot,
                hash,
                record,
                file,
            });
        }
        self.records = records;
        self.validate_directory()
    }
    // Only initial load can bind an existing operation's historical anchor.
    // No later verify_recorded/emit readback may take this comparison path.
    fn load_first_stored_intent(
        &mut self,
        file: &File,
        settings: &Settings,
        binding: &str,
    ) -> Result<(Record, String), GlobalSchemaV1Error> {
        if self.fresh
            || self.anchor_binding != AnchorBinding::AwaitingStoredIntent
            || !self.records.is_empty()
        {
            return Err(refuse("backup original anchor cannot be rebound"));
        }
        let (record, hash) =
            self.read_record_checked(0, file, settings, RecordDirectoryCheck::FirstStoredIntent)?;
        if record.intent_hash.is_some() || record.predecessor.is_some() {
            return Err(refuse("backup first stored intent chain mismatch"));
        }
        match &record.transition {
            Transition::Intent {
                source_canonical,
                hard_limits,
                approval,
            } if source_canonical == binding
                && *hard_limits == Limits::production()
                && approval == "not_granted" => {}
            _ => return Err(refuse("backup first stored intent/source mismatch")),
        }
        // The exact codec/hash/self inode and real directory objects are
        // already verified. Preserve the original serialized links as facts.
        self.anchor = record.directory.clone();
        self.anchor_binding = AnchorBinding::StoredIntent;
        self.validate_directory()?;
        Ok((record, hash))
    }
    fn read_record(
        &mut self,
        slot: usize,
        file: &File,
        settings: &Settings,
    ) -> Result<(Record, String), GlobalSchemaV1Error> {
        if self.anchor_binding == AnchorBinding::AwaitingStoredIntent {
            return Err(refuse("backup exact record reader has no bound intent"));
        }
        self.read_record_checked(slot, file, settings, RecordDirectoryCheck::ExactBound)
    }
    fn read_record_checked(
        &mut self,
        slot: usize,
        file: &File,
        settings: &Settings,
        directory_check: RecordDirectoryCheck,
    ) -> Result<(Record, String), GlobalSchemaV1Error> {
        let bytes = read_bounded(
            file,
            settings.limits.record_limit(slot),
            &mut self.work,
            &settings.limits,
        )?;
        let decoded: Envelope = serde_json::from_slice(&bytes)
            .map_err(|_| refuse("backup journal closed codec refused"))?;
        let directory_matches = match directory_check {
            RecordDirectoryCheck::ExactBound => decoded.record.directory == self.anchor,
            RecordDirectoryCheck::FirstStoredIntent => {
                slot == 0
                    && self.anchor_binding == AnchorBinding::AwaitingStoredIntent
                    && self
                        .anchor
                        .same_directory_objects(&decoded.record.directory)
            }
        };
        if decoded.record.version != 1
            || decoded.record.slot as usize != slot
            || decoded.record.leaf != RECORDS[slot]
            || !directory_matches
        {
            return Err(refuse("backup journal version/leaf/directory mismatch"));
        }
        self.same_named(RECORDS[slot], file, &decoded.record.self_inode)?;
        let canonical = encode(
            &decoded.record,
            settings.limits.record_limit(slot),
            &mut self.work,
            &settings.limits,
        )?;
        let hash = domain_hash(&canonical);
        if decoded.sha256 != hash {
            return Err(refuse("backup journal domain hash mismatch"));
        }
        let exact = encode(
            &EnvelopeRef {
                sha256: &hash,
                record: &decoded.record,
            },
            settings.limits.record_limit(slot),
            &mut self.work,
            &settings.limits,
        )?;
        if bytes != exact {
            return Err(refuse("backup journal noncanonical bytes"));
        }
        self.same_named(RECORDS[slot], file, &decoded.record.self_inode)?;
        Ok((decoded.record, hash))
    }
    fn emit(
        &mut self,
        slot: usize,
        transition: Transition,
        settings: &Settings,
    ) -> Result<(), GlobalSchemaV1Error> {
        if self.records.len() >= settings.limits.record_count {
            return Err(refuse("backup journal record count exceeded"));
        }
        self.validate_directory()?;
        let old_total = self.records.iter().try_fold(0_u64, |sum, r| {
            checked_io(r.file.metadata()).and_then(|m| {
                sum.checked_add(m.len())
                    .ok_or_else(|| refuse("backup journal total overflow"))
            })
        })?;
        let remaining = settings
            .limits
            .record_total
            .checked_sub(old_total)
            .ok_or_else(|| refuse("backup persisted record total exceeded"))?;
        let record_limit = settings.limits.record_limit(slot).min(remaining);
        if record_limit == 0 {
            return Err(refuse("backup persisted record total exceeded"));
        }
        let file = new_read_write(&self.directory.file, RECORDS[slot])?;
        let self_inode = private_node(&file, false)?;
        let record = Record {
            version: 1,
            slot: slot as u8,
            leaf: RECORDS[slot].into(),
            directory: self.anchor.clone(),
            self_inode,
            intent_hash: self.records.first().map(|r| r.hash.clone()),
            predecessor: self.records.last().map(|r| r.hash.clone()),
            transition,
        };
        let canonical = encode(&record, record_limit, &mut self.work, &settings.limits)?;
        let hash = domain_hash(&canonical);
        let bytes = encode(
            &EnvelopeRef {
                sha256: &hash,
                record: &record,
            },
            record_limit,
            &mut self.work,
            &settings.limits,
        )?;
        let total = self.records.iter().try_fold(bytes.len() as u64, |sum, r| {
            checked_io(r.file.metadata()).and_then(|m| {
                sum.checked_add(m.len())
                    .ok_or_else(|| refuse("backup journal total overflow"))
            })
        })?;
        if total > settings.limits.record_total {
            return Err(refuse("backup persisted record total exceeded"));
        }
        checked_io(file.write_all_at(&bytes, 0))?;
        settings.phase(Phase::BeforeRecordSync(slot as u8))?;
        checked_io(file.sync_all())?;
        settings.phase(Phase::AfterRecordSync(slot as u8))?;
        let (_, actual_hash) = self.read_record(slot, &file, settings)?;
        if actual_hash != hash {
            return Err(refuse("backup journal readback mismatch"));
        }
        settings.phase(Phase::AfterRecordReadback(slot as u8))?;
        checked_io(self.directory.file.sync_all())?;
        self.same_named(RECORDS[slot], &file, &record.self_inode)?;
        settings.phase(Phase::AfterRecordParentSync(slot as u8))?;
        self.same_named(RECORDS[slot], &file, &record.self_inode)?;
        self.records.push(SavedRecord {
            slot,
            hash,
            record,
            file,
        });
        Ok(())
    }
    fn record(&self, slot: usize) -> Option<&SavedRecord> {
        self.records.iter().find(|r| r.slot == slot)
    }
    fn created(&self, role: Role) -> Option<&Node> {
        match &self.record(role.slots().0)?.record.transition {
            Transition::Created { role: actual, file } if *actual == role => Some(file),
            _ => None,
        }
    }
    fn copied(&self, role: Role) -> Option<&FileWitness> {
        match &self.record(role.slots().1)?.record.transition {
            Transition::Copied { role: actual, file } if *actual == role => Some(file),
            _ => None,
        }
    }
    fn validate_chain(
        &self,
        binding: &str,
        db: &prospective::FileAnchor,
        audit: Option<&prospective::FileAnchor>,
    ) -> Result<(), GlobalSchemaV1Error> {
        let first = self
            .records
            .first()
            .ok_or_else(|| refuse("backup existing operation has no durable intent"))?;
        if first.slot != 0
            || first.record.intent_hash.is_some()
            || first.record.predecessor.is_some()
        {
            return Err(refuse("backup initial intent chain mismatch"));
        }
        match &first.record.transition {
            Transition::Intent {
                source_canonical,
                hard_limits,
                approval,
            } if source_canonical == binding
                && *hard_limits == Limits::production()
                && approval == "not_granted" => {}
            _ => {
                return Err(refuse(
                    "backup original operation/source drift; no rotation",
                ))
            }
        }
        let expected: &[usize] = if audit.is_some() {
            &[0, 1, 2, 3, 4, 5]
        } else {
            &[0, 1, 2, 5]
        };
        if self.records.len() > expected.len() {
            return Err(refuse("backup unexpected journal branch"));
        }
        let mut predecessor = &first.hash;
        for (index, saved) in self.records.iter().enumerate().skip(1) {
            if saved.slot != expected[index]
                || saved.record.intent_hash.as_ref() != Some(&first.hash)
                || saved.record.predecessor.as_ref() != Some(predecessor)
            {
                return Err(refuse("backup exact transition/predecessor mismatch"));
            }
            let source = if saved.slot <= 2 {
                db
            } else {
                audit.unwrap_or(db)
            };
            match (&saved.record.transition, saved.slot) {
                (Transition::Created { role, file }, 1 | 3) if role.slots().0 == saved.slot => {
                    if file.links != 1 || file.owner != uid() || file.mode != 0o600 {
                        return Err(refuse("backup Created witness shape invalid"));
                    }
                }
                (Transition::Copied { role, file }, 2 | 4) if role.slots().1 == saved.slot => {
                    if Some(&file.node) != self.created(*role)
                        || file.length != source.length
                        || file.sha256 != source.sha256
                    {
                        return Err(refuse("backup Copied original byte witness mismatch"));
                    }
                }
                (
                    Transition::BackupVerified {
                        database,
                        audit: actual,
                    },
                    5,
                ) => {
                    if self.copied(Role::Database) != Some(database)
                        || actual.as_ref() != self.copied(Role::Audit)
                        || actual.is_some() != audit.is_some()
                    {
                        return Err(refuse("backup terminal full role binding mismatch"));
                    }
                }
                _ => return Err(refuse("backup unexpected transition payload")),
            }
            predecessor = &saved.hash;
        }
        Ok(())
    }
    fn desired_limits(
        db: &prospective::FileAnchor,
        audit: Option<&prospective::FileAnchor>,
        limits: &Limits,
    ) -> Result<(), GlobalSchemaV1Error> {
        if db.length > limits.main_extent
            || audit.is_some_and(|f| f.length > limits.audit_extent)
            || db
                .length
                .checked_add(audit.map_or(0, |f| f.length))
                .is_none_or(|n| n > limits.role_total)
        {
            return Err(refuse("backup desired role extent budget exceeded"));
        }
        Ok(())
    }
    fn verify_recorded(&mut self, settings: &Settings) -> Result<(), GlobalSchemaV1Error> {
        self.validate_record_names()?;
        // The original expected records remain held; never replace them with a
        // changed valid-looking journal inside one invocation.
        for index in 0..self.records.len() {
            let saved = &self.records[index];
            let file = checked_io(saved.file.try_clone())?;
            let slot = saved.slot;
            let hash = saved.hash.clone();
            let (_, current) = self.read_record(slot, &file, settings)?;
            if hash != current {
                return Err(refuse("backup recorded journal changed"));
            }
            checked_io(file.sync_all())?;
        }
        checked_io(self.directory.file.sync_all())?;
        self.validate_record_names()
    }
    fn validate_record_names(&self) -> Result<(), GlobalSchemaV1Error> {
        self.validate_directory()?;
        // Absence is part of this exact held chain, including the unused audit
        // slots. A newly appeared known record is not adopted by a final reader.
        for (slot, leaf) in RECORDS.iter().enumerate() {
            match (
                existing(&self.directory.file, leaf, false)?,
                self.record(slot),
            ) {
                (None, None) => {}
                (Some(actual), Some(saved)) => {
                    if private_node(&actual, false)? != saved.record.self_inode {
                        return Err(refuse("backup final known journal inode changed"));
                    }
                    self.same_named(leaf, &saved.file, &saved.record.self_inode)?;
                }
                _ => return Err(refuse("backup exact known journal presence changed")),
            }
        }
        self.validate_directory()
    }
    fn copy_role(
        &mut self,
        role: Role,
        input: &File,
        expected: &prospective::FileAnchor,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &Settings,
    ) -> Result<(), GlobalSchemaV1Error> {
        if self.copied(role).is_some() {
            return self.verify_role(role, source, options, settings);
        }
        let file = if let Some(node) = self.created(role).cloned() {
            let file = existing(&self.directory.file, role.leaf(), true)?
                .ok_or_else(|| refuse("backup Created inode missing; cannot heal"))?;
            self.same_named(role.leaf(), &file, &node)?;
            if checked_io(file.metadata())?.len() > expected.length {
                return Err(refuse(
                    "backup Created inode exceeds original source; no truncate",
                ));
            }
            file
        } else {
            if existing(&self.directory.file, role.leaf(), false)?.is_some() {
                return Err(refuse("backup unrecorded role inode; cannot adopt"));
            }
            let file = new_read_write(&self.directory.file, role.leaf())?;
            let node = private_node(&file, false)?;
            if checked_io(file.metadata())?.len() != 0 {
                return Err(refuse("backup new role is not empty"));
            }
            settings.phase(Phase::BeforeEmptyRoleSync(role))?;
            checked_io(file.sync_all())?;
            settings.phase(Phase::AfterEmptyRoleSync(role))?;
            checked_io(self.directory.file.sync_all())?;
            settings.phase(Phase::AfterEmptyRoleParentSync(role))?;
            self.same_named(role.leaf(), &file, &node)?;
            if checked_io(file.metadata())?.len() != 0 {
                return Err(refuse("backup empty role changed before Created"));
            }
            settings.phase(Phase::BeforeCreated(role))?;
            self.same_named(role.leaf(), &file, &node)?;
            if checked_io(file.metadata())?.len() != 0 {
                return Err(refuse(
                    "backup new empty role changed before Created persistence",
                ));
            }
            self.emit(
                role.slots().0,
                Transition::Created { role, file: node },
                settings,
            )?;
            settings.phase(Phase::AfterCreated(role))?;
            file
        };
        let node = self
            .created(role)
            .cloned()
            .ok_or_else(|| refuse("backup Created witness missing"))?;
        self.work
            .reserve_copy(expected.length, source, options, &settings.limits)?;
        // Budget, shape and exact inode are checked before resetting a known
        // partial role; unrecorded/overlong files are never truncated.
        self.same_named(role.leaf(), &file, &node)?;
        if checked_io(file.metadata())?.len() > expected.length {
            return Err(refuse("backup partial role grew before copy"));
        }
        let before = checked_io(input.metadata())?;
        if !before.is_file()
            || before.nlink() != 1
            || before.dev() != expected.device
            || before.ino() != expected.inode
            || before.len() != expected.length
        {
            return Err(refuse("backup original source inode/extent changed"));
        }
        checked_io(file.set_len(0))?;
        let mut chunk = [0_u8; 65_536];
        let mut offset = 0_u64;
        let mut digest = Sha256::new();
        while offset < expected.length {
            let room = (expected.length - offset).min(chunk.len() as u64) as usize;
            let n = checked_io(input.read_at(&mut chunk[..room], offset))?;
            if n == 0 {
                return Err(refuse("backup source truncated during copy"));
            }
            checked_io(file.write_all_at(&chunk[..n], offset))?;
            digest.update(&chunk[..n]);
            offset += n as u64;
            settings.phase(Phase::AfterCopyChunk(role))?;
        }
        let mut sentinel = [0_u8; 1];
        source.reserve_backup_reads(1, options)?;
        if checked_io(input.read_at(&mut sentinel, expected.length))? != 0
            || checked_io(input.metadata())?.len() != expected.length
            || FileIdentity::from_metadata(&checked_io(input.metadata())?)
                != FileIdentity::from_metadata(&before)
            || hex::encode(digest.finalize()) != expected.sha256
        {
            return Err(refuse("backup actual copy source bytes changed"));
        }
        self.same_named(role.leaf(), &file, &node)?;
        settings.phase(Phase::BeforeRoleSync(role))?;
        checked_io(file.sync_all())?;
        checked_io(self.directory.file.sync_all())?;
        settings.phase(Phase::AfterRoleSync(role))?;
        let witness = self.fingerprint_role(role, &file, source, options, settings)?;
        if witness.length != expected.length || witness.sha256 != expected.sha256 {
            return Err(refuse("backup copied bytes differ from original"));
        }
        self.emit(
            role.slots().1,
            Transition::Copied {
                role,
                file: witness,
            },
            settings,
        )?;
        settings.phase(Phase::AfterCopied(role))?;
        Ok(())
    }
    fn fingerprint_role(
        &mut self,
        role: Role,
        file: &File,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &Settings,
    ) -> Result<FileWitness, GlobalSchemaV1Error> {
        let node = private_node(file, false)?;
        self.same_named(role.leaf(), file, &node)?;
        let length = checked_io(file.metadata())?.len();
        let cap = if role == Role::Database {
            settings.limits.main_extent
        } else {
            settings.limits.audit_extent
        };
        if length > cap {
            return Err(refuse("backup actual role extent exceeded"));
        }
        let fingerprint = source.backup_fingerprint(file, options)?;
        if checked_io(file.metadata())?.len() != fingerprint.length {
            return Err(refuse("backup role changed after readback"));
        }
        self.work.common(source, options, &settings.limits)?;
        self.same_named(role.leaf(), file, &node)?;
        Ok(FileWitness {
            node,
            length: fingerprint.length,
            sha256: fingerprint.sha256,
        })
    }
    fn verify_role(
        &mut self,
        role: Role,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &Settings,
    ) -> Result<(), GlobalSchemaV1Error> {
        let expected = self
            .copied(role)
            .cloned()
            .ok_or_else(|| refuse("backup Copied witness missing"))?;
        let file = existing(&self.directory.file, role.leaf(), false)?
            .ok_or_else(|| refuse("backup committed role missing; cannot heal"))?;
        self.same_named(role.leaf(), &file, &expected.node)?;
        let actual = self.fingerprint_role(role, &file, source, options, settings)?;
        if actual != expected {
            return Err(refuse("backup actual stored role mismatch"));
        }
        Ok(())
    }
    fn validate_outputs(
        &mut self,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &Settings,
    ) -> Result<(), GlobalSchemaV1Error> {
        self.verify_recorded(settings)?;
        let (db, audit) = source.backup_expected();
        self.validate_chain(&source.backup_binding(options)?, &db, audit.as_ref())?;
        self.verify_role(Role::Database, source, options, settings)?;
        if audit.is_some() {
            self.verify_role(Role::Audit, source, options, settings)?;
        } else if existing(&self.directory.file, Role::Audit.leaf(), false)?.is_some() {
            return Err(refuse("backup original absent audit role appeared"));
        }
        settings.phase(Phase::AfterOutputReads)?;
        // The scoped last-read phase cannot alter a valid record/role and
        // retain a capability from the earlier bytes. Charge this final
        // full reader to the same original counters, without another hook.
        self.verify_recorded(settings)?;
        self.verify_role(Role::Database, source, options, settings)?;
        if audit.is_some() {
            self.verify_role(Role::Audit, source, options, settings)?;
        } else if existing(&self.directory.file, Role::Audit.leaf(), false)?.is_some() {
            return Err(refuse("backup absent audit appeared at output tail"));
        }
        self.work.common(source, options, &settings.limits)?;
        self.validate_directory()
    }
    fn validate_output_names(&self) -> Result<(), GlobalSchemaV1Error> {
        for role in [Role::Database, Role::Audit] {
            if let Some(expected) = self.copied(role) {
                let file = existing(&self.directory.file, role.leaf(), false)?
                    .ok_or_else(|| refuse("backup final recorded role missing"))?;
                self.same_named(role.leaf(), &file, &expected.node)?;
                if checked_io(file.metadata())?.len() != expected.length {
                    return Err(refuse("backup final recorded role extent changed"));
                }
            } else if existing(&self.directory.file, role.leaf(), false)?.is_some() {
                return Err(refuse("backup unexpected final role"));
            }
        }
        Ok(())
    }
    // No phase callbacks: this is the complete final reader, not the old
    // hookful facade with a different name.
    fn validate_outputs_without_hooks(
        &mut self,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &Settings,
    ) -> Result<(), GlobalSchemaV1Error> {
        self.verify_recorded(settings)?;
        let (db, audit) = source.backup_expected();
        self.validate_chain(&source.backup_binding(options)?, &db, audit.as_ref())?;
        self.verify_role(Role::Database, source, options, settings)?;
        if audit.is_some() {
            self.verify_role(Role::Audit, source, options, settings)?;
        } else if existing(&self.directory.file, Role::Audit.leaf(), false)?.is_some() {
            return Err(refuse("rows tail absent audit role appeared"));
        }
        self.validate_output_names()?;
        self.validate_record_names()?;
        self.work.common(source, options, &settings.limits)?;
        self.validate_directory()
    }
    fn copied_sidecars_absent(&self) -> Result<(), GlobalSchemaV1Error> {
        for suffix in ["-wal", "-shm", "-journal"] {
            let leaf = format!("{}{suffix}", Role::Database.leaf());
            // Only genuine NotFound is absence. Unknown/nonregular/symlink
            // sidecars are refused and are never removed or adopted.
            match openat_component(
                &self.directory.file,
                OsStr::new(&leaf),
                O_RDONLY_FLAG,
                false,
            ) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                _ => return Err(refuse("copied database sidecar appeared")),
            }
        }
        Ok(())
    }
    fn with_copied_rows<T>(
        &mut self,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &Settings,
        work: &mut rows::RowsWork,
        read: impl for<'loan> FnOnce(
            &OwnedBackupRowsView<'loan>,
            &mut rows::RowsWork,
        ) -> Result<T, GlobalSchemaV1Error>,
    ) -> Result<T, GlobalSchemaV1Error> {
        work.reserve_copy_route(
            self.directory.path.as_os_str().len() as u64,
            Role::Database.leaf().len() as u64,
        )?;
        self.validate_outputs_without_hooks(source, options, settings)?;
        self.copied_sidecars_absent()?;
        let expected = self
            .copied(Role::Database)
            .cloned()
            .ok_or_else(|| refuse("rows loan has no durable Copied database"))?;
        let file = existing(&self.directory.file, Role::Database.leaf(), false)?
            .ok_or_else(|| refuse("rows Copied database missing"))?;
        self.same_named(Role::Database.leaf(), &file, &expected.node)?;
        let route = sqlite_open_route_from_retained_parent(
            &self.directory.file,
            OsStr::new(Role::Database.leaf()),
        )
        .map_err(|_| refuse("rows retained copy route unavailable"))?;
        // immutable is restricted to this already closed durable Copied role
        // with proven absent sidecars; it is never used for the live source.
        let mut uri = String::from("file:");
        for b in route.as_os_str().as_bytes() {
            use std::fmt::Write;
            write!(&mut uri, "%{b:02X}").map_err(|_| refuse("rows copy URI encoding"))?;
        }
        uri.push_str("?mode=ro&immutable=1");
        let connection = Connection::open_with_flags(
            &uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
            operation: "open closed Copied rows read-only",
            source,
        })?;
        let result = (|| {
            connection
                .execute_batch("PRAGMA query_only=ON")
                .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                    operation: "restrict Copied rows loan",
                    source,
                })?;
            if !connection
                .is_readonly(rusqlite::DatabaseName::Main)
                .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                    operation: "prove Copied reader is read-only",
                    source,
                })?
            {
                return Err(refuse("Copied rows reader is writable"));
            }
            let integrity = capture_selection_integrity(&connection)?;
            let _ = integrity;
            self.same_named(Role::Database.leaf(), &file, &expected.node)?;
            self.copied_sidecars_absent()?;
            read(
                &OwnedBackupRowsView {
                    connection: &connection,
                },
                work,
            )
        })();
        connection
            .close()
            .map_err(|(_, source)| GlobalSchemaV1Error::SelectionSqlite {
                operation: "close scoped Copied rows reader",
                source,
            })?;
        self.copied_sidecars_absent()?;
        if sqlite_open_route_from_retained_parent(
            &self.directory.file,
            OsStr::new(Role::Database.leaf()),
        )
        .map_err(|_| refuse("rows copy route disappeared"))?
            != route
        {
            return Err(refuse("rows copy route changed"));
        }
        self.same_named(Role::Database.leaf(), &file, &expected.node)?;
        self.validate_outputs_without_hooks(source, options, settings)?;
        result
    }
    pub(super) fn prepare(
        mut self,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        database: &File,
        audit_file: &PinnedSelectionAuditFile,
        settings: &Settings,
    ) -> Result<Pending, GlobalSchemaV1Error> {
        let (db, audit) = source.backup_expected();
        Self::desired_limits(&db, audit.as_ref(), &settings.limits)?;
        self.work.common(source, options, &settings.limits)?;
        let binding = source.backup_binding(options)?;
        self.load(settings, &binding)?;
        if self.records.is_empty() {
            if !self.fresh {
                return Err(refuse(
                    "backup existing operation without recorded intent; no adopt",
                ));
            }
            for role in [Role::Database, Role::Audit] {
                if existing(&self.directory.file, role.leaf(), false)?.is_some() {
                    return Err(refuse("backup role present before intent"));
                }
            }
            self.emit(
                0,
                Transition::Intent {
                    source_canonical: binding.clone(),
                    hard_limits: Limits::production(),
                    approval: "not_granted".into(),
                },
                settings,
            )?;
        }
        self.validate_chain(&binding, &db, audit.as_ref())?;
        self.verify_recorded(settings)?;
        self.copy_role(Role::Database, database, &db, source, options, settings)?;
        if let (Some(expected), PinnedSelectionAuditFile::Present { file, .. }) =
            (audit.as_ref(), audit_file)
        {
            self.copy_role(Role::Audit, file, expected, source, options, settings)?;
        } else if audit.is_some()
            || existing(&self.directory.file, Role::Audit.leaf(), false)?.is_some()
        {
            return Err(refuse("backup audit branch mismatch"));
        }
        self.validate_outputs(source, options, settings)?;
        Ok(Pending { workspace: self })
    }
}
pub(super) struct Pending {
    workspace: Workspace,
}
impl Pending {
    pub(super) fn with_copied_rows<T>(
        &mut self,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &Settings,
        work: &mut rows::RowsWork,
        read: impl for<'loan> FnOnce(
            &OwnedBackupRowsView<'loan>,
            &mut rows::RowsWork,
        ) -> Result<T, GlobalSchemaV1Error>,
    ) -> Result<T, GlobalSchemaV1Error> {
        self.workspace
            .with_copied_rows(source, options, settings, work, read)
    }
    pub(super) fn validate_rows_before_commit_without_hooks(
        &mut self,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &Settings,
    ) -> Result<(), GlobalSchemaV1Error> {
        self.workspace
            .validate_outputs_without_hooks(source, options, settings)
    }
    pub(super) fn validate_before_commit(
        &mut self,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &Settings,
    ) -> Result<(), GlobalSchemaV1Error> {
        self.workspace.validate_outputs(source, options, settings)
    }
    pub(super) fn issue(
        mut self,
        mut source: prospective::PreparedGlobalSchemaProspective,
        settings: Settings,
    ) -> Result<VerifiedUnapprovedByteBackup, GlobalSchemaV1Error> {
        source.backup_validate()?;
        {
            let (pending, options, _, _) = source.backup_parts();
            self.workspace
                .validate_outputs(pending, options, &settings)?;
        }
        source.backup_validate()?;
        if self.workspace.record(5).is_none() {
            let database = self
                .workspace
                .copied(Role::Database)
                .cloned()
                .ok_or_else(|| refuse("backup terminal database missing"))?;
            let audit = self.workspace.copied(Role::Audit).cloned();
            self.workspace
                .emit(5, Transition::BackupVerified { database, audit }, &settings)?;
            settings.phase(Phase::AfterTerminalSync)?;
        }
        let mut result = VerifiedUnapprovedByteBackup {
            source,
            workspace: self.workspace,
            settings,
        };
        result.validate_actual()?;
        Ok(result)
    }
}
/// Only the fresh final reader produces this non-Clone, non-Deserialize value.
/// Its scope is unapproved original bytes, never schema/pool/apply completion.
pub(super) struct VerifiedUnapprovedByteBackup {
    source: prospective::PreparedGlobalSchemaProspective,
    workspace: Workspace,
    settings: Settings,
}
impl VerifiedUnapprovedByteBackup {
    pub(super) fn target_metadata_reservation(&self) -> Result<u64, GlobalSchemaV1Error> {
        // Both before/after passes parse and canonically encode the retained
        // fixed journal and source binding. Reserve their actual extents.
        let bytes = self.workspace.records.iter().try_fold(0u64, |n, saved| {
            checked_io(saved.file.metadata())?
                .len()
                .checked_add(n)
                .ok_or_else(|| refuse("target original journal metadata overflow"))
        })?;
        bytes
            .checked_mul(20)
            .and_then(|n| n.checked_add(65536))
            .ok_or_else(|| refuse("target original journal metadata overflow"))
    }
    pub(super) fn with_target_origin<T>(
        &mut self,
        work: &mut target::TargetWork,
        operation: impl for<'loan> FnOnce(
            &mut CopiedTargetOriginLoan<'loan>,
            &mut target::TargetWork,
        ) -> Result<T, GlobalSchemaV1Error>,
    ) -> Result<T, GlobalSchemaV1Error> {
        work.metadata(self.target_metadata_reservation()?)?;
        let (source, options, _, _) = self.source.backup_parts();
        self.workspace
            .validate_outputs_without_hooks(source, options, &self.settings)?;
        self.workspace.copied_sidecars_absent()?;
        let expected = self
            .workspace
            .copied(Role::Database)
            .ok_or_else(|| refuse("target has no original Copied witness"))?;
        work.metadata(512)?;
        let expected = expected.clone();
        let file = existing(&self.workspace.directory.file, Role::Database.leaf(), false)?
            .ok_or_else(|| refuse("target original Copied missing"))?;
        self.workspace
            .same_named(Role::Database.leaf(), &file, &expected.node)?;
        let result = operation(
            &mut CopiedTargetOriginLoan {
                file: &file,
                expected: &expected,
                workspace: &mut self.workspace,
                source,
                options,
                settings: &self.settings,
            },
            work,
        );
        self.workspace
            .same_named(Role::Database.leaf(), &file, &expected.node)?;
        self.workspace.copied_sidecars_absent()?;
        self.workspace
            .validate_outputs_without_hooks(source, options, &self.settings)?;
        result
    }
    pub(super) fn with_copied_rows<T>(
        &mut self,
        work: &mut rows::RowsWork,
        read: impl for<'loan> FnOnce(
            &OwnedBackupRowsView<'loan>,
            &mut rows::RowsWork,
        ) -> Result<T, GlobalSchemaV1Error>,
    ) -> Result<T, GlobalSchemaV1Error> {
        let (source, options, _, _) = self.source.backup_parts();
        self.workspace
            .with_copied_rows(source, options, &self.settings, work, read)
    }
    pub(super) fn validate_rows_preliminary(&mut self) -> Result<(), GlobalSchemaV1Error> {
        self.validate_actual()
    }
    pub(super) fn before_rows_render(&mut self) -> Result<(), GlobalSchemaV1Error> {
        self.settings.phase(Phase::BeforeRender)?;
        self.validate_actual()
    }
    pub(super) fn validate_rows_tail_without_hooks(
        &mut self,
        tail: &rows::RetainedRowsSourceTail,
    ) -> Result<(), GlobalSchemaV1Error> {
        let (source, options, database, audit) = self.source.backup_parts();
        self.workspace
            .validate_outputs_without_hooks(source, options, &self.settings)?;
        if self.workspace.record(5).is_none() {
            return Err(refuse("rows tail terminal missing"));
        }
        tail.validate_without_hooks(source, options, database, audit, true)?;
        self.workspace.copied_sidecars_absent()?;
        self.workspace.validate_output_names()?;
        self.workspace.validate_record_names()?;
        self.workspace
            .work
            .common(source, options, &self.settings.limits)
    }
    fn validate_actual(&mut self) -> Result<(), GlobalSchemaV1Error> {
        self.source.backup_validate()?;
        {
            let (source, options, _, _) = self.source.backup_parts();
            self.workspace
                .validate_outputs(source, options, &self.settings)?;
            if self.workspace.record(5).is_none() {
                return Err(refuse("backup actual terminal reader incomplete"));
            }
        }
        // Output/journal reads precede the final actual source observation.
        self.source.backup_validate()?;
        self.workspace.validate_directory()?;
        self.workspace.validate_output_names()?;
        self.workspace.validate_record_names()?;
        let (source, options, _, _) = self.source.backup_parts();
        self.workspace
            .work
            .common(source, options, &self.settings.limits)
    }
    pub(super) fn render_unapproved(mut self) -> Result<String, GlobalSchemaV1Error> {
        self.settings.phase(Phase::BeforeRender)?;
        self.validate_actual()?;
        #[derive(Serialize)]
        struct Review<'a> {
            version: u16,
            capability_scope: &'static str,
            source_scope: &'static str,
            intent_hash: &'a str,
            terminal_hash: &'a str,
            records: usize,
            database: &'a FileWitness,
            audit: Option<&'a FileWitness>,
            approval: &'static str,
            row_preservation_proof: bool,
            exchange: &'static str,
            restore: &'static str,
            apply_supported: bool,
            apply_blocker: &'static str,
            single_operation: bool,
        }
        let first = self
            .workspace
            .records
            .first()
            .ok_or_else(|| refuse("backup render intent missing"))?;
        let terminal = self
            .workspace
            .record(5)
            .ok_or_else(|| refuse("backup render terminal missing"))?;
        let database = self
            .workspace
            .copied(Role::Database)
            .ok_or_else(|| refuse("backup render database missing"))?;
        let review = Review {
            version: 1,
            capability_scope: "verified_unapproved_original_byte_backup",
            source_scope: "retained_main_file_bytes_with_zero_owned_wal_not_logical_row_snapshot",
            intent_hash: &first.hash,
            terminal_hash: &terminal.hash,
            records: self.workspace.records.len(),
            database,
            audit: self.workspace.copied(Role::Audit),
            approval: "not_granted",
            row_preservation_proof: false,
            exchange: "not_implemented",
            restore: "not_authorized",
            apply_supported: false,
            apply_blocker: super::super::selection_v2::SELECTION_V2_APPLY_BLOCKER,
            single_operation: true,
        };
        let (_, options, _, _) = self.source.backup_parts();
        let bytes = prospective::bounded_json(&review, options.max_review_bytes)?;
        String::from_utf8(bytes).map_err(|_| refuse("backup review encoding refused"))
    }
}

/// Borrowed only inside the Copied-role reader. No raw/path constructor and
/// no connection, statement, descriptor or maintenance authority can escape.
pub(super) struct OwnedBackupRowsView<'loan> {
    connection: &'loan Connection,
}
impl OwnedBackupRowsView<'_> {
    pub(super) fn connection(&self) -> &Connection {
        self.connection
    }
}

/// Private callback-local access to the exact recorded Copied FD. Neither an
/// FD nor a route is exposed; only the sealed target sink can receive bytes.
pub(super) struct CopiedTargetOriginLoan<'loan> {
    file: &'loan File,
    expected: &'loan FileWitness,
    workspace: &'loan mut Workspace,
    source: &'loan mut prospective::Pending,
    options: &'loan prospective::Options,
    settings: &'loan Settings,
}
impl CopiedTargetOriginLoan<'_> {
    pub(super) fn binding(
        &self,
        work: &mut target::TargetWork,
    ) -> Result<target::OriginalBackupBinding, GlobalSchemaV1Error> {
        #[derive(Serialize)]
        struct Binding<'a> {
            source: &'a str,
            intent: &'a str,
            terminal: &'a str,
            copied: &'a FileWitness,
        }
        let first = self
            .workspace
            .record(0)
            .ok_or_else(|| refuse("target original intent missing"))?;
        let terminal = self
            .workspace
            .record(5)
            .ok_or_else(|| refuse("target original terminal missing"))?;
        let Transition::Intent {
            source_canonical, ..
        } = &first.record.transition
        else {
            return Err(refuse("target original intent shape"));
        };
        let bytes = work.encode_metadata(
            &Binding {
                source: source_canonical,
                intent: &first.hash,
                terminal: &terminal.hash,
                copied: self.expected,
            },
            1024 * 1024,
        )?;
        work.metadata(64)?;
        Ok(target::OriginalBackupBinding {
            canonical: String::from_utf8(bytes)
                .map_err(|_| refuse("target original binding encoding"))?,
            length: self.expected.length,
            sha256: self.expected.sha256.clone(),
        })
    }
    pub(super) fn copy_to(
        &mut self,
        target: &mut target::CreatedTarget<'_>,
        work: &mut target::TargetWork,
    ) -> Result<(), GlobalSchemaV1Error> {
        // Charge the very same original physical/common owner before any read.
        let length = self
            .expected
            .length
            .checked_add(1)
            .ok_or_else(|| refuse("target original read overflow"))?;
        self.source.reserve_backup_reads(length, self.options)?;
        self.workspace
            .work
            .common(self.source, self.options, &self.settings.limits)?;
        self.workspace
            .same_named(Role::Database.leaf(), self.file, &self.expected.node)?;
        target.copy_from_original(self.file, self.expected.length, &self.expected.sha256, work)?;
        self.workspace
            .same_named(Role::Database.leaf(), self.file, &self.expected.node)?;
        self.workspace.copied_sidecars_absent()
    }
}
