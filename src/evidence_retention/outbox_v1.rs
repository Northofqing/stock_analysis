//! Local durable storage for existing Unverified drafts. No dispatch/seal issuer.
use super::{
    parse_draft_with_work, OwnerDomain, TrustState, UnverifiedEvidencePackageDraft, ValueError,
    Work, DRAFT_LIMIT, MIB,
};
use crate::database::{self, RetentionMaterialMainProof};
use fs2::FileExt;
use rusqlite::{params, types::ValueRef, Connection, Error as SqlError};
use std::fs::{self, File, OpenOptions, ReadDir};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const DIR: &str = "retention-material-outbox-v1";
const MAIN: &str = "materials-v1.sqlite";
const JOURNAL: &str = "materials-v1.sqlite-journal";
const APPLICATION_ID: i64 = 0x52554f31; // RUO1, local material schema, not an authority tag.
const PAGE_BYTES: i64 = 4096;
const PAGE_LIMIT: i64 = 4096;
const MAIN_LIMIT: u64 = 16 * MIB as u64;
const JOURNAL_LIMIT: u64 = 17 * MIB as u64;
const MATERIAL_SQL: &str = "CREATE TABLE material(id TEXT PRIMARY KEY NOT NULL,sha TEXT NOT NULL,owner INTEGER NOT NULL,day TEXT NOT NULL,slot TEXT NOT NULL,canonical BLOB NOT NULL,bytes INTEGER NOT NULL,trust TEXT NOT NULL CHECK(trust='Unverified'))";
const CONFLICT_SQL: &str = "CREATE TABLE slot_conflict(generation INTEGER NOT NULL,left_id TEXT NOT NULL,right_id TEXT NOT NULL,event BLOB NOT NULL,PRIMARY KEY(generation,left_id,right_id),FOREIGN KEY(left_id) REFERENCES material(id),FOREIGN KEY(right_id) REFERENCES material(id))";
const ATTEMPT_SQL: &str = "CREATE TABLE attempt(generation INTEGER PRIMARY KEY NOT NULL,command_id TEXT NOT NULL,kind TEXT NOT NULL,event BLOB NOT NULL,FOREIGN KEY(command_id) REFERENCES material(id))";
const SLOT_INDEX_SQL: &str = "CREATE INDEX material_slot ON material(owner,day,slot)";
const STORED_EVENT: &[u8] = b"Unverified local material stored";
const REUSE_EVENT: &[u8] = b"Unverified exact local material reused";
const CONFLICT_EVENT: &[u8] = b"Unverified logical slot conflict retained";

#[cfg(target_os = "macos")]
const NOFOLLOW_CLOEXEC: i32 = 0x100 | 0x1000000 | 0x4;
#[cfg(target_os = "macos")]
const CREATE_EXCLUSIVE: i32 = 0x200 | 0x800;
#[cfg(target_os = "linux")]
const NOFOLLOW_CLOEXEC: i32 = 0x20000 | 0x80000 | 0x800;
#[cfg(target_os = "linux")]
const CREATE_EXCLUSIVE: i32 = 0x40 | 0x80;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("retention material descriptor binding requires Linux or macOS");
unsafe extern "C" {
    fn openat(fd: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn geteuid() -> u32;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutboxFault {
    Io,
    Busy,
    RootBinding,
    ForeignShape,
    ResidualJournal,
    Quota,
    Work(ValueError),
    NativeMainBinding,
    StaleGeneration,
    CommitUnknown,
    CloseHeld,
    MissingFactsUnknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalDisposition {
    Stored,
    ExactReuse,
    Conflict,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveredPresence {
    ExactUnverified,
    MissingFactsUnknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TxState {
    None,
    Active,
    Unknown,
    Committed,
    RolledBack,
}

struct Pin {
    file: File,
    identity: Option<(u64, u64, u32, u32)>,
}
impl Pin {
    fn new(file: File) -> Self {
        Self {
            file,
            identity: None,
        }
    }
    fn admit(&mut self, directory: bool, private: bool, cap: u64) -> Result<(), OutboxFault> {
        let m = self.file.metadata().map_err(|_| OutboxFault::Io)?;
        if m.uid() != unsafe { geteuid() }
            || (directory && !m.is_dir())
            || (!directory && (!m.is_file() || m.nlink() != 1 || m.len() > cap))
            || (private && m.mode() & 0o777 != if directory { 0o700 } else { 0o600 })
        {
            return Err(OutboxFault::RootBinding);
        }
        self.identity = Some((m.dev(), m.ino(), m.mode(), m.uid()));
        Ok(())
    }
    fn validate(&self, path: &Path, directory: bool, cap: u64) -> Result<(), OutboxFault> {
        let f = self.file.metadata().map_err(|_| OutboxFault::Io)?;
        let n = fs::symlink_metadata(path).map_err(|_| OutboxFault::RootBinding)?;
        let id = |m: &fs::Metadata| (m.dev(), m.ino(), m.mode(), m.uid());
        if self.identity != Some(id(&f))
            || id(&n) != id(&f)
            || (directory && !n.is_dir())
            || (!directory && (!n.is_file() || n.nlink() != 1 || n.len() > cap))
        {
            return Err(OutboxFault::RootBinding);
        }
        Ok(())
    }
}

/// Fields and constructor stay private: sibling modules can consume a genuine
/// fixed owner loan but cannot turn an arbitrary File/path into this capability.
pub(crate) struct OutboxOpenLoan<'a> {
    parent: &'a File,
    main: &'a File,
}
impl OutboxOpenLoan<'_> {
    pub(crate) fn retained_pair(&self) -> (&File, &File) {
        (self.parent, self.main)
    }
}

#[must_use = "the local outbox owns its files, connection and cumulative Work"]
pub(crate) struct UnverifiedOutbox {
    frame: Box<Frame>,
}
#[must_use = "Held retains the connection, original command and first failure"]
pub(crate) struct HeldOutbox {
    frame: Box<Frame>,
}
#[must_use = "unknown COMMIT is not an automatic rollback or retry permission"]
pub(crate) struct PendingOutbox {
    frame: Box<Frame>,
}
pub(crate) struct StoredUnverified {
    pub(crate) generation: i64,
    pub(crate) disposition: LocalDisposition,
}
impl StoredUnverified {
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
}
pub(crate) struct RecoveredUnverified {
    pub(crate) presence: RecoveredPresence,
    pub(crate) original_fault: OutboxFault,
}
impl RecoveredUnverified {
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
}
#[must_use]
pub(crate) enum EnqueueOutcome {
    Stored(StoredUnverified),
    Held(HeldOutbox),
    Pending(PendingOutbox),
}
#[must_use]
pub(crate) enum RecoveryOutcome {
    Observed(RecoveredUnverified),
    Held(HeldOutbox),
    Pending(PendingOutbox),
}

/// Ordinary material read from a checked local snapshot. Absence is never
/// permission to resubmit or manufacture the original facts.
pub(crate) struct MaterialReadObservation {
    material: Option<UnverifiedEvidencePackageDraft>,
    observed_generation: i64,
    other_slot_materials: usize,
}
impl MaterialReadObservation {
    pub(crate) fn material(&self) -> Option<&UnverifiedEvidencePackageDraft> {
        self.material.as_ref()
    }
    pub(crate) fn observed_generation(&self) -> i64 {
        self.observed_generation
    }
    pub(crate) fn other_slot_materials(&self) -> usize {
        self.other_slot_materials
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
}
#[must_use]
pub(crate) enum MaterialReadOutcome {
    Observed(MaterialReadObservation),
    Held(HeldOutbox),
}

struct Frame {
    // The cfg VM precedes the connection in destruction order. Explicit test
    // finalize is required for the Busy-close continuation; Drop is abandonment.
    #[cfg(test)]
    vm: Option<TestVm>,
    connection: Option<Connection>,
    proof: Option<RetentionMaterialMainProof>,
    open_failure: Option<database::RetentionMaterialOpenFailure>,
    main_failure: Option<database::RetentionMaterialMainFailure>,
    root: Option<Pin>,
    data: Option<Pin>,
    directory: Option<Pin>,
    main: Option<Pin>,
    init_lock: Option<Pin>,
    namespace_lock: Option<Pin>,
    journal: Option<Pin>,
    census: Option<ReadDir>,
    root_path: PathBuf,
    data_path: PathBuf,
    directory_path: PathBuf,
    main_path: PathBuf,
    init_path: PathBuf,
    namespace_path: PathBuf,
    journal_path: PathBuf,
    work: Work,
    first: Option<OutboxFault>,
    errors: [Option<SqlError>; 3],
    inventory: Vec<UnverifiedEvidencePackageDraft>,
    command: Option<UnverifiedEvidencePackageDraft>,
    generation: i64,
    command_generation: Option<i64>,
    read_id: Option<String>,
    selected_material: Option<UnverifiedEvidencePackageDraft>,
    other_slot_materials: usize,
    disposition: Option<LocalDisposition>,
    tx: TxState,
    rollback_attempted: bool,
    close_attempted: bool,
    init_locked: bool,
    lease_locked: bool,
    initialized: bool,
    #[cfg(test)]
    observation: TestCommitObservation,
}
impl Frame {
    fn new(root: &Path) -> Self {
        let mut work = Work::new();
        let cost = root
            .as_os_str()
            .len()
            .checked_mul(7)
            .and_then(|n| n.checked_add(2048));
        let first = if root.as_os_str().len() > 4096 {
            Some(OutboxFault::RootBinding)
        } else {
            cost.and_then(|n| work.own(n).err().map(OutboxFault::Work))
        };
        let route = if root.as_os_str().len() <= 4096 {
            root
        } else {
            Path::new("")
        };
        let data_path = route.join("data");
        let directory_path = data_path.join(DIR);
        let main_path = directory_path.join(MAIN);
        let init_path = directory_path.join("init.lock");
        let namespace_path = directory_path.join("namespace.lock");
        let journal_path = directory_path.join(JOURNAL);
        Self {
            #[cfg(test)]
            vm: None,
            connection: None,
            proof: None,
            open_failure: None,
            main_failure: None,
            root: None,
            data: None,
            directory: None,
            main: None,
            init_lock: None,
            namespace_lock: None,
            journal: None,
            census: None,
            root_path: route.to_path_buf(),
            data_path,
            directory_path,
            main_path,
            init_path,
            namespace_path,
            journal_path,
            work,
            first,
            errors: [None, None, None],
            inventory: Vec::new(),
            command: None,
            generation: 0,
            command_generation: None,
            disposition: None,
            read_id: None,
            selected_material: None,
            other_slot_materials: 0,
            tx: TxState::None,
            rollback_attempted: false,
            close_attempted: false,
            init_locked: false,
            lease_locked: false,
            initialized: false,
            #[cfg(test)]
            observation: TestCommitObservation::Normal,
        }
    }
    fn fault(&mut self, fault: OutboxFault) -> OutboxFault {
        *self.first.get_or_insert(fault)
    }
    fn sql<T>(&mut self, result: rusqlite::Result<T>) -> Result<T, OutboxFault> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                let busy = matches!(&error, SqlError::SqliteFailure(e, _) if e.code == rusqlite::ErrorCode::DatabaseBusy || e.code == rusqlite::ErrorCode::DatabaseLocked);
                self.retain_error(error);
                Err(if busy {
                    OutboxFault::Busy
                } else {
                    OutboxFault::Io
                })
            }
        }
    }
    fn retain_error(&mut self, error: SqlError) {
        // At most initial SQL, once rollback, once close errors are reachable.
        retain_sql_error(&mut self.errors, error);
    }
    fn dir_path(&self) -> &Path {
        &self.directory_path
    }
    fn loan(&self) -> OutboxOpenLoan<'_> {
        OutboxOpenLoan {
            parent: &self.directory.as_ref().expect("owned directory").file,
            main: &self.main.as_ref().expect("owned Main").file,
        }
    }
    fn conn(&self) -> &Connection {
        self.connection.as_ref().expect("owned connection")
    }
    fn charge(&mut self, n: usize) -> Result<(), OutboxFault> {
        self.work.own(n).map_err(OutboxFault::Work)
    }
    fn nodes(&mut self, n: usize) -> Result<(), OutboxFault> {
        for _ in 0..n {
            self.work.node().map_err(OutboxFault::Work)?;
        }
        Ok(())
    }
    fn open(&mut self) -> Result<(), OutboxFault> {
        if let Some(first) = self.first {
            return Err(first);
        }
        self.root = Some(Pin::new(
            OpenOptions::new()
                .read(true)
                .custom_flags(NOFOLLOW_CLOEXEC)
                .open(&self.root_path)
                .map_err(|_| OutboxFault::Io)?,
        ));
        self.root.as_mut().unwrap().admit(true, false, 0)?;
        self.data = Some(Pin::new(open_child(
            &self.root.as_ref().unwrap().file,
            b"data\0",
            false,
            false,
        )?));
        self.data.as_mut().unwrap().admit(true, false, 0)?;
        self.root
            .as_ref()
            .unwrap()
            .validate(&self.root_path, true, 0)?;
        self.data
            .as_ref()
            .unwrap()
            .validate(&self.data_path, true, 0)?;
        // New dedicated directory only, relative to the actual retained data FD.
        let created = unsafe {
            libc::mkdirat(
                self.data.as_ref().unwrap().file.as_raw_fd(),
                b"retention-material-outbox-v1\0".as_ptr().cast(),
                0o700,
            )
        };
        if created < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(OutboxFault::Io);
        }
        self.directory = Some(Pin::new(open_child(
            &self.data.as_ref().unwrap().file,
            b"retention-material-outbox-v1\0",
            false,
            false,
        )?));
        self.directory.as_mut().unwrap().admit(true, true, 0)?;
        self.root
            .as_ref()
            .unwrap()
            .validate(&self.root_path, true, 0)?;
        self.data
            .as_ref()
            .unwrap()
            .validate(&self.data_path, true, 0)?;
        self.directory
            .as_ref()
            .unwrap()
            .validate(self.dir_path(), true, 0)?;
        self.census(false)?;
        self.init_lock = Some(Pin::new(open_lock(
            &self.directory.as_ref().unwrap().file,
            b"init.lock\0",
        )?));
        self.init_lock.as_mut().unwrap().admit(false, true, 0)?;
        FileExt::try_lock_exclusive(&self.init_lock.as_ref().unwrap().file)
            .map_err(|_| OutboxFault::Busy)?;
        self.init_locked = true;
        self.namespace_lock = Some(Pin::new(open_lock(
            &self.directory.as_ref().unwrap().file,
            b"namespace.lock\0",
        )?));
        self.namespace_lock
            .as_mut()
            .unwrap()
            .admit(false, true, 0)?;
        FileExt::try_lock_shared(&self.namespace_lock.as_ref().unwrap().file)
            .map_err(|_| OutboxFault::Busy)?;
        self.lease_locked = true;
        let (file, fresh) = match open_child(
            &self.directory.as_ref().unwrap().file,
            b"materials-v1.sqlite\0",
            true,
            true,
        ) {
            Ok(file) => (file, true),
            Err(OutboxFault::Busy) => (
                open_child(
                    &self.directory.as_ref().unwrap().file,
                    b"materials-v1.sqlite\0",
                    true,
                    false,
                )?,
                false,
            ),
            Err(error) => return Err(error),
        };
        self.main = Some(Pin::new(file));
        self.main.as_mut().unwrap().admit(false, true, MAIN_LIMIT)?;
        if !fresh {
            self.header()?;
        }
        self.binding_tail(false)?; // No journal, WAL, SHM or unknown entry before native open.
        match database::open_retention_material(self.loan()) {
            Ok(opened) => {
                let (connection, proof) = opened.into_owned_parts();
                self.connection = Some(connection);
                self.proof = Some(proof);
            }
            Err(error) => {
                let (connection, failure) = error.into_held_owned_parts();
                self.connection = connection;
                self.open_failure = Some(failure);
                return Err(OutboxFault::NativeMainBinding);
            }
        }
        if fresh {
            self.initialize()?;
        } else {
            self.recognize()?;
        }
        // The per-connection limit is never inferred from another coordinator.
        self.read_inventory()?; // Existing typed shape and quota precede mutating PRAGMAs.
        self.configure()?;
        self.initialized = true;
        self.binding_tail(false)?;
        FileExt::unlock(&self.init_lock.as_ref().unwrap().file).map_err(|_| OutboxFault::Io)?;
        self.init_locked = false;
        Ok(())
    }
    fn header(&mut self) -> Result<(), OutboxFault> {
        let mut header = [0u8; 100];
        self.work.scan(100).map_err(OutboxFault::Work)?;
        std::os::unix::fs::FileExt::read_exact_at(
            &self.main.as_ref().unwrap().file,
            &mut header,
            0,
        )
        .map_err(|_| OutboxFault::ForeignShape)?;
        if &header[..16] != b"SQLite format 3\0"
            || u16::from_be_bytes([header[16], header[17]]) != 4096
            || u32::from_be_bytes(header[68..72].try_into().unwrap()) != APPLICATION_ID as u32
        {
            return Err(OutboxFault::ForeignShape);
        }
        Ok(())
    }
    fn scalar(&mut self, sql: &str) -> Result<i64, OutboxFault> {
        self.nodes(1)?;
        let result = self.conn().query_row(sql, [], |row| row.get::<_, i64>(0));
        self.sql(result)
    }
    fn execute(&mut self, sql: &str) -> Result<(), OutboxFault> {
        let result = self.conn().execute_batch(sql);
        self.sql(result)
    }
    fn initialize(&mut self) -> Result<(), OutboxFault> {
        self.execute("PRAGMA page_size=4096; PRAGMA application_id=1381322545; PRAGMA user_version=1; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=0;")?;
        self.execute("BEGIN IMMEDIATE")?;
        self.tx = TxState::Active;
        for sql in [MATERIAL_SQL, CONFLICT_SQL, ATTEMPT_SQL, SLOT_INDEX_SQL] {
            self.execute(sql)?;
        }
        self.commit()?;
        self.recognize()
    }
    fn recognize(&mut self) -> Result<(), OutboxFault> {
        if self.scalar("PRAGMA application_id")? != APPLICATION_ID
            || self.scalar("PRAGMA user_version")? != 1
            || self.scalar("PRAGMA page_size")? != PAGE_BYTES
            || self.scalar("SELECT count(*) FROM sqlite_master")? != 6
        {
            return Err(OutboxFault::ForeignShape);
        }
        for (name, expected) in [
            ("material", MATERIAL_SQL),
            ("slot_conflict", CONFLICT_SQL),
            ("attempt", ATTEMPT_SQL),
            ("material_slot", SLOT_INDEX_SQL),
        ] {
            self.work.scan(expected.len()).map_err(OutboxFault::Work)?;
            let result = self.conn().query_row("SELECT typeof(sql)='text' AND length(CAST(sql AS BLOB))=?2 AND CAST(sql AS BLOB)=?3 FROM sqlite_master WHERE name=?1", params![name, expected.len() as i64, expected.as_bytes()], |row| row.get::<_, i64>(0));
            if self.sql(result)? != 1 {
                return Err(OutboxFault::ForeignShape);
            }
        }
        // Two implicit unique indexes only, fixed names and table associations.
        if self.scalar("SELECT count(*) FROM sqlite_master WHERE type='index' AND sql IS NULL AND ((name='sqlite_autoindex_material_1' AND tbl_name='material') OR (name='sqlite_autoindex_slot_conflict_1' AND tbl_name='slot_conflict'))")? != 2 {
            return Err(OutboxFault::ForeignShape);
        }
        Ok(())
    }
    fn configure(&mut self) -> Result<(), OutboxFault> {
        // Recognition precedes every mutating connection PRAGMA on an existing DB.
        self.execute("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=0; PRAGMA synchronous=FULL;")?;
        let mode = self.conn().query_row("PRAGMA journal_mode", [], |row| {
            Ok(matches!(row.get_ref(0)?, ValueRef::Text(b"delete")))
        });
        if !self.sql(mode)? {
            return Err(OutboxFault::ForeignShape);
        }
        let limit = self
            .conn()
            .query_row("PRAGMA max_page_count=4096", [], |row| row.get::<_, i64>(0));
        if self.sql(limit)? != PAGE_LIMIT
            || self.scalar("PRAGMA max_page_count")? != PAGE_LIMIT
            || self.scalar("PRAGMA page_count")? > PAGE_LIMIT
            || self.scalar("PRAGMA synchronous")? != 2
            || self.scalar("PRAGMA foreign_keys")? != 1
        {
            return Err(OutboxFault::Quota);
        }
        self.conn()
            .busy_timeout(std::time::Duration::ZERO)
            .map_err(|error| {
                self.retain_error(error);
                OutboxFault::Io
            })?;
        Ok(())
    }
    fn quotas(&mut self) -> Result<(), OutboxFault> {
        if self.scalar("SELECT count(*) FROM material")? > 32
            || self.scalar("SELECT coalesce(sum(length(canonical)),0) FROM material")? > 8 * MIB as i64
            || self.scalar("SELECT (SELECT count(*) FROM attempt)+(SELECT count(*) FROM slot_conflict)")? > 128
            || self.scalar("SELECT count(*) FROM material WHERE typeof(id)!='text' OR length(CAST(id AS BLOB))>128 OR typeof(sha)!='text' OR length(sha)!=64 OR typeof(owner)!='integer' OR owner NOT BETWEEN 0 AND 3 OR typeof(day)!='text' OR length(day)!=10 OR typeof(slot)!='text' OR length(CAST(slot AS BLOB))>256 OR typeof(canonical)!='blob' OR length(canonical)>3145728 OR typeof(bytes)!='integer' OR bytes!=length(canonical) OR typeof(trust)!='text' OR trust!='Unverified'")? != 0
            || self.scalar("SELECT count(*) FROM attempt WHERE typeof(generation)!='integer' OR generation<1 OR typeof(command_id)!='text' OR length(CAST(command_id AS BLOB))>128 OR typeof(kind)!='text' OR kind NOT IN ('Stored','ExactReuse','Conflict') OR typeof(event)!='blob' OR length(event)>4096")? != 0
            || self.scalar("SELECT count(*) FROM slot_conflict WHERE typeof(generation)!='integer' OR generation<1 OR typeof(left_id)!='text' OR typeof(right_id)!='text' OR length(left_id)>128 OR length(right_id)>128 OR left_id=right_id OR typeof(event)!='blob' OR length(event)>4096")? != 0 {
            return Err(OutboxFault::Quota);
        }
        let events = self
            .scalar("SELECT (SELECT count(*) FROM attempt)+(SELECT count(*) FROM slot_conflict)")?;
        self.nodes(usize::try_from(events).map_err(|_| OutboxFault::ForeignShape)? * 5)?;
        let event_bytes = self.scalar("SELECT (SELECT coalesce(sum(length(event)),0) FROM attempt)+(SELECT coalesce(sum(length(event)),0) FROM slot_conflict)")?;
        self.work
            .scan(usize::try_from(event_bytes).map_err(|_| OutboxFault::ForeignShape)?)
            .map_err(OutboxFault::Work)?;
        if self.scalar("SELECT count(*) FROM attempt WHERE (kind='Stored' AND event!=CAST('Unverified local material stored' AS BLOB)) OR (kind='ExactReuse' AND event!=CAST('Unverified exact local material reused' AS BLOB)) OR (kind='Conflict' AND event!=CAST('Unverified logical slot conflict retained' AS BLOB))")? != 0
            || self.scalar("SELECT count(*) FROM slot_conflict c JOIN material l ON l.id=c.left_id JOIN material r ON r.id=c.right_id WHERE c.event!=CAST('Unverified logical slot conflict retained' AS BLOB) OR l.owner!=r.owner OR l.day!=r.day OR l.slot!=r.slot OR l.canonical=r.canonical")? != 0 {
            return Err(OutboxFault::ForeignShape);
        }
        let n = self.scalar("SELECT count(*) FROM attempt")?;
        if self.scalar("SELECT count(*) FROM attempt a WHERE a.kind='Conflict' AND NOT EXISTS(SELECT 1 FROM slot_conflict c WHERE c.generation=a.generation)")? != 0
            || self.scalar("SELECT coalesce(max(generation),0) FROM attempt")? != n
            || self.scalar("SELECT count(*) FROM slot_conflict c LEFT JOIN attempt a USING(generation) WHERE a.generation IS NULL OR a.kind!='Conflict' OR a.command_id!=c.right_id")? != 0 {
            return Err(OutboxFault::ForeignShape);
        }
        self.generation = n;
        Ok(())
    }
    fn read_inventory(&mut self) -> Result<(), OutboxFault> {
        self.quotas()?;
        self.inventory.clear(); // Explicitly consume the previous owned read; Work is not refunded.
        self.charge(32 * std::mem::size_of::<UnverifiedEvidencePackageDraft>())?;
        self.inventory.reserve(32);
        let connection = self.connection.as_ref().unwrap();
        let work = &mut self.work;
        let inventory = &mut self.inventory;
        let errors = &mut self.errors;
        let read = (|| -> Result<(), OutboxFault> {
            let mut statement = connection
                .prepare(
                    "SELECT id,sha,owner,day,slot,canonical,bytes,trust FROM material ORDER BY id",
                )
                .map_err(|error| {
                    retain_sql_error(errors, error);
                    OutboxFault::Io
                })?;
            let mut rows = statement.query([]).map_err(|error| {
                retain_sql_error(errors, error);
                OutboxFault::Io
            })?;
            while let Some(row) = rows.next().map_err(|error| {
                retain_sql_error(errors, error);
                OutboxFault::Io
            })? {
                work.node().map_err(OutboxFault::Work)?;
                let blob = row_blob(row, 5, errors)?;
                if blob.len() > DRAFT_LIMIT {
                    return Err(OutboxFault::Quota);
                }
                let draft = parse_draft_with_work(blob, work).map_err(OutboxFault::Work)?;
                inventory.push(draft); // Move the real acquired value into the frame before a later fault.
                let draft = inventory.last().unwrap();
                if row_text(row, 0, errors)? != draft.id.as_bytes()
                    || row_text(row, 1, errors)? != draft.sha256.as_bytes()
                    || row_integer(row, 2, errors)? != owner_number(draft.owner)
                    || row_text(row, 3, errors)? != draft.day.as_bytes()
                    || row_text(row, 4, errors)? != draft.slot.as_bytes()
                    || row_text(row, 7, errors)? != b"Unverified"
                    || row_integer(row, 6, errors)? != blob.len() as i64
                {
                    return Err(OutboxFault::ForeignShape);
                }
            }
            Ok(())
        })();
        read?;
        // This is a fixed local integrity gate; it creates no Financial owner.
        let connection = self.connection.as_ref().unwrap();
        let errors = &mut self.errors;
        let mut statement = connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(|error| {
                retain_sql_error(errors, error);
                OutboxFault::Io
            })?;
        let mut rows = statement.query([]).map_err(|error| {
            retain_sql_error(errors, error);
            OutboxFault::Io
        })?;
        if rows
            .next()
            .map_err(|error| {
                retain_sql_error(errors, error);
                OutboxFault::Io
            })?
            .is_some()
        {
            return Err(OutboxFault::ForeignShape);
        }
        Ok(())
    }
    fn binding_tail(&mut self, allow_journal: bool) -> Result<(), OutboxFault> {
        self.root
            .as_ref()
            .unwrap()
            .validate(&self.root_path, true, 0)?;
        self.data
            .as_ref()
            .unwrap()
            .validate(&self.data_path, true, 0)?;
        self.directory
            .as_ref()
            .unwrap()
            .validate(self.dir_path(), true, 0)?;
        if let Some(pin) = &self.init_lock {
            pin.validate(&self.init_path, false, 0)?;
        }
        if let Some(pin) = &self.namespace_lock {
            pin.validate(&self.namespace_path, false, 0)?;
        }
        if let Some(pin) = &self.main {
            pin.validate(&self.main_path, false, MAIN_LIMIT)?;
        }
        if self.connection.is_some() {
            let proof = self.proof.as_ref().ok_or(OutboxFault::NativeMainBinding)?;
            if let Err(failure) = proof.validate(self.loan()) {
                self.main_failure = Some(failure);
                return Err(OutboxFault::NativeMainBinding);
            }
        }
        self.census(allow_journal)
    }
    fn census(&mut self, allow_journal: bool) -> Result<(), OutboxFault> {
        self.charge(4 * 512)?;
        self.work.scan(4 * 256).map_err(OutboxFault::Work)?;
        self.census = Some(fs::read_dir(self.dir_path()).map_err(|_| OutboxFault::Io)?);
        let mut count = 0usize;
        let mut journal_seen = false;
        while let Some(entry) = self.census.as_mut().unwrap().next() {
            count += 1;
            self.nodes(1)?;
            if count > 4 {
                return Err(OutboxFault::ForeignShape);
            }
            let entry = entry.map_err(|_| OutboxFault::Io)?;
            match entry.file_name().to_str() {
                Some(MAIN | "init.lock" | "namespace.lock") => (),
                Some(JOURNAL) if allow_journal => {
                    if self.journal.is_none() {
                        self.journal = Some(Pin::new(open_child(
                            &self.directory.as_ref().unwrap().file,
                            b"materials-v1.sqlite-journal\0",
                            false,
                            false,
                        )?));
                        self.journal
                            .as_mut()
                            .unwrap()
                            .admit(false, true, JOURNAL_LIMIT)?;
                    }
                    self.journal.as_ref().unwrap().validate(
                        &self.journal_path,
                        false,
                        JOURNAL_LIMIT,
                    )?;
                    journal_seen = true;
                }
                Some(JOURNAL) => return Err(OutboxFault::ResidualJournal),
                _ => return Err(OutboxFault::ForeignShape),
            }
        }
        self.census.take();
        if !journal_seen {
            self.journal.take();
        } // Observed absent after genuine SQLite deletion; resource pin only.
        Ok(())
    }
    fn enqueue(&mut self, expected_generation: i64) -> Result<(), OutboxFault> {
        if self.first.is_some() || !self.initialized {
            return Err(self.first.unwrap_or(OutboxFault::ForeignShape));
        }
        let draft = self.command.as_ref().unwrap();
        let cost = [
            draft.canonical.len(),
            draft.id.len(),
            draft.sha256.len(),
            draft.day.len(),
            draft.slot.len(),
        ]
        .into_iter()
        .try_fold(0usize, |n, x| n.checked_add(x))
        .ok_or(OutboxFault::Quota)?;
        self.charge(cost)?;
        if !(0..=128).contains(&expected_generation) {
            return Err(OutboxFault::StaleGeneration);
        }
        FileExt::try_lock_shared(&self.init_lock.as_ref().unwrap().file)
            .map_err(|_| OutboxFault::Busy)?;
        self.init_locked = true; // Open/recognition cannot race an admitted writer's journal creation.
        self.binding_tail(false)?;
        self.execute("BEGIN IMMEDIATE")?;
        self.tx = TxState::Active;
        self.read_inventory()?;
        if self.generation != expected_generation {
            return Err(OutboxFault::StaleGeneration);
        }
        let draft = self.command.as_ref().unwrap();
        let same = self.inventory.iter().find(|old| old.id == draft.id);
        if same.is_some_and(|old| {
            old.canonical != draft.canonical
                || old.sha256 != draft.sha256
                || old.owner != draft.owner
                || old.day != draft.day
                || old.slot != draft.slot
        }) {
            return Err(OutboxFault::ForeignShape);
        }
        let exact = same.is_some();
        let mut conflicts = [0usize; 32];
        let mut conflict_count = 0usize;
        for (i, old) in self.inventory.iter().enumerate() {
            if !exact
                && old.owner == draft.owner
                && old.day == draft.day
                && old.slot == draft.slot
                && old.id != draft.id
            {
                conflicts[conflict_count] = i;
                conflict_count += 1;
            }
        }
        let next = self.generation.checked_add(1).ok_or(OutboxFault::Quota)?;
        let bytes = draft.canonical.len() as i64;
        if (!exact && self.inventory.len() >= 32)
            || self.scalar("SELECT coalesce(sum(length(canonical)),0) FROM material")?
                + if exact { 0 } else { bytes }
                > 8 * MIB as i64
            || self.scalar(
                "SELECT (SELECT count(*) FROM attempt)+(SELECT count(*) FROM slot_conflict)",
            )? + 1
                + conflict_count as i64
                > 128
        {
            return Err(OutboxFault::Quota);
        }
        self.charge((conflict_count + 1) * std::mem::size_of::<usize>() + 256)?;
        let draft = self.command.as_ref().unwrap();
        if !exact {
            let result = self.conn().execute(
                "INSERT INTO material VALUES(?1,?2,?3,?4,?5,?6,?7,'Unverified')",
                params![
                    draft.id,
                    draft.sha256,
                    owner_number(draft.owner),
                    draft.day,
                    draft.slot,
                    draft.canonical,
                    bytes
                ],
            );
            self.sql(result)?;
        }
        let disposition = if exact {
            LocalDisposition::ExactReuse
        } else if conflict_count == 0 {
            LocalDisposition::Stored
        } else {
            LocalDisposition::Conflict
        };
        let (kind, event) = match disposition {
            LocalDisposition::Stored => ("Stored", STORED_EVENT),
            LocalDisposition::ExactReuse => ("ExactReuse", REUSE_EVENT),
            LocalDisposition::Conflict => ("Conflict", CONFLICT_EVENT),
        };
        self.charge(
            event
                .len()
                .checked_mul(conflict_count + 1)
                .ok_or(OutboxFault::Quota)?,
        )?;
        let result = self.conn().execute(
            "INSERT INTO attempt VALUES(?1,?2,?3,?4)",
            params![next, self.command.as_ref().unwrap().id, kind, event],
        );
        self.sql(result)?;
        if !exact {
            for &i in &conflicts[..conflict_count] {
                let result = self.conn().execute(
                    "INSERT INTO slot_conflict VALUES(?1,?2,?3,?4)",
                    params![
                        next,
                        self.inventory[i].id,
                        self.command.as_ref().unwrap().id,
                        CONFLICT_EVENT
                    ],
                );
                self.sql(result)?;
            }
        }
        self.command_generation = Some(next);
        self.disposition = Some(disposition);
        self.binding_tail(true)?;
        #[cfg(test)]
        if self.observation == TestCommitObservation::ExitBeforeCommit {
            std::process::exit(73);
        }
        self.commit()?;
        #[cfg(test)]
        if self.observation == TestCommitObservation::ExitAfterCommit {
            std::process::exit(74);
        }
        #[cfg(test)]
        if self.observation == TestCommitObservation::LoseResponse {
            self.tx = TxState::Unknown;
            return Err(OutboxFault::CommitUnknown);
        }
        self.finish_close()?;
        Ok(())
    }
    fn commit(&mut self) -> Result<(), OutboxFault> {
        let result = self.conn().execute_batch("COMMIT");
        match result {
            Ok(()) if self.conn().is_autocommit() => {
                self.tx = TxState::Committed;
                Ok(())
            }
            Ok(()) => {
                self.tx = TxState::Unknown;
                Err(OutboxFault::CommitUnknown)
            }
            Err(error) => {
                self.retain_error(error);
                self.tx = TxState::Unknown;
                Err(OutboxFault::CommitUnknown)
            }
        }
    }
    fn finish_close(&mut self) -> Result<(), OutboxFault> {
        self.binding_tail(false)?;
        if !self.conn().is_autocommit() {
            return Err(OutboxFault::CommitUnknown);
        }
        if self.close_attempted {
            return Err(OutboxFault::CloseHeld);
        }
        self.close_attempted = true;
        let connection = self.connection.take().unwrap();
        match connection.close() {
            Ok(()) => {
                self.proof.take();
            }
            Err((connection, error)) => {
                self.connection = Some(connection); // Restore actual owned Connection before recording any fault.
                self.retain_error(error);
                return Err(OutboxFault::CloseHeld);
            }
        }
        self.binding_tail(false)?;
        if self.init_locked {
            FileExt::unlock(&self.init_lock.as_ref().unwrap().file).map_err(|_| OutboxFault::Io)?;
            self.init_locked = false;
        }
        if self.lease_locked {
            FileExt::unlock(&self.namespace_lock.as_ref().unwrap().file)
                .map_err(|_| OutboxFault::Io)?;
            self.lease_locked = false;
        }
        Ok(())
    }
    fn abandon_once(&mut self) {
        if self.connection.is_none() {
            if self.init_locked && FileExt::unlock(&self.init_lock.as_ref().unwrap().file).is_ok() {
                self.init_locked = false;
            }
            if self.lease_locked
                && FileExt::unlock(&self.namespace_lock.as_ref().unwrap().file).is_ok()
            {
                self.lease_locked = false;
            }
            return;
        }
        if !self.conn().is_autocommit() && !self.rollback_attempted {
            self.rollback_attempted = true;
            let result = self.conn().execute_batch("ROLLBACK");
            match result {
                Ok(()) if self.conn().is_autocommit() => self.tx = TxState::RolledBack,
                Ok(()) => (),
                Err(error) => self.retain_error(error),
            }
        }
        if self.conn().is_autocommit() && !self.close_attempted {
            self.close_attempted = true;
            match self.connection.take().unwrap().close() {
                Ok(()) => {
                    self.proof.take();
                }
                Err((connection, error)) => {
                    self.connection = Some(connection);
                    self.retain_error(error);
                }
            }
        }
        if self.connection.is_none() {
            if self.init_locked && FileExt::unlock(&self.init_lock.as_ref().unwrap().file).is_ok() {
                self.init_locked = false;
            }
            if self.lease_locked
                && FileExt::unlock(&self.namespace_lock.as_ref().unwrap().file).is_ok()
            {
                self.lease_locked = false;
            }
        }
        // This controlled resource drain preserves first; it cannot return Stored.
    }
}
fn row_value<'r>(
    row: &'r rusqlite::Row<'_>,
    index: usize,
    errors: &mut [Option<SqlError>; 3],
) -> Result<ValueRef<'r>, OutboxFault> {
    row.get_ref(index).map_err(|error| {
        retain_sql_error(errors, error);
        OutboxFault::Io
    })
}
fn row_text<'r>(
    row: &'r rusqlite::Row<'_>,
    index: usize,
    errors: &mut [Option<SqlError>; 3],
) -> Result<&'r [u8], OutboxFault> {
    match row_value(row, index, errors)? {
        ValueRef::Text(bytes) => Ok(bytes),
        _ => Err(OutboxFault::ForeignShape),
    }
}
fn row_blob<'r>(
    row: &'r rusqlite::Row<'_>,
    index: usize,
    errors: &mut [Option<SqlError>; 3],
) -> Result<&'r [u8], OutboxFault> {
    match row_value(row, index, errors)? {
        ValueRef::Blob(bytes) => Ok(bytes),
        _ => Err(OutboxFault::ForeignShape),
    }
}
fn row_integer(
    row: &rusqlite::Row<'_>,
    index: usize,
    errors: &mut [Option<SqlError>; 3],
) -> Result<i64, OutboxFault> {
    match row_value(row, index, errors)? {
        ValueRef::Integer(value) => Ok(value),
        _ => Err(OutboxFault::ForeignShape),
    }
}
fn retain_sql_error(slots: &mut [Option<SqlError>; 3], error: SqlError) {
    *slots
        .iter_mut()
        .find(|slot| slot.is_none())
        .expect("fixed once error custody") = Some(error);
}
fn open_child(
    parent: &File,
    leaf: &'static [u8],
    writable: bool,
    exclusive: bool,
) -> Result<File, OutboxFault> {
    let flags = NOFOLLOW_CLOEXEC
        | if writable { 2 } else { 0 }
        | if exclusive { CREATE_EXCLUSIVE } else { 0 };
    // SAFETY: every leaf is a fixed NUL-terminated single component, parent is retained.
    let fd = unsafe { openat(parent.as_raw_fd(), leaf.as_ptr().cast(), flags, 0o600u32) };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        return Err(
            if exclusive && error.kind() == std::io::ErrorKind::AlreadyExists {
                OutboxFault::Busy
            } else {
                OutboxFault::Io
            },
        );
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn open_lock(parent: &File, leaf: &'static [u8]) -> Result<File, OutboxFault> {
    match open_child(parent, leaf, true, true) {
        Ok(file) => Ok(file),
        Err(OutboxFault::Busy) => open_child(parent, leaf, true, false),
        Err(error) => Err(error),
    }
}
fn owner_number(owner: OwnerDomain) -> i64 {
    match owner {
        OwnerDomain::Data => 0,
        OwnerDomain::InvestmentDecision => 1,
        OwnerDomain::PaperLedger => 2,
        OwnerDomain::Attribution => 3,
    }
}

impl UnverifiedOutbox {
    pub(crate) fn open() -> Result<Self, HeldOutbox> {
        Self::open_fixed(crate::production_root::production_root())
    }
    fn open_fixed(root: &Path) -> Result<Self, HeldOutbox> {
        let mut frame = Box::new(Frame::new(root));
        match frame.open() {
            Ok(()) => Ok(Self { frame }),
            Err(fault) => {
                frame.fault(fault);
                Err(HeldOutbox { frame })
            }
        }
    }
    pub(crate) fn generation(&self) -> i64 {
        self.frame.generation
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
    /// Select by original package ID in a read transaction, returning the
    /// original acquired value only after rollback and consuming close succeed.
    pub(crate) fn read_material(mut self, id: String) -> MaterialReadOutcome {
        self.frame.read_id = Some(id); // Retain the exact query before admission.
        let result = (|| -> Result<(), OutboxFault> {
            if self.frame.first.is_some() || !self.frame.initialized {
                return Err(self.frame.first.unwrap_or(OutboxFault::ForeignShape));
            }
            let query_bytes = {
                let id = self.frame.read_id.as_ref().unwrap();
                if !super::id_ok(id, super::DP) {
                    return Err(OutboxFault::ForeignShape);
                }
                id.len()
            };
            self.frame.charge(query_bytes)?;
            FileExt::try_lock_shared(&self.frame.init_lock.as_ref().unwrap().file)
                .map_err(|_| OutboxFault::Busy)?;
            self.frame.init_locked = true;
            self.frame.binding_tail(false)?;
            self.frame.execute("BEGIN DEFERRED")?;
            self.frame.tx = TxState::Active;
            self.frame.read_inventory()?;
            if let Some(index) = self
                .frame
                .inventory
                .iter()
                .position(|value| Some(&value.id) == self.frame.read_id.as_ref())
            {
                // Move the genuine whole decoder return; no re-created package.
                self.frame.selected_material = Some(self.frame.inventory.remove(index));
                let selected = self.frame.selected_material.as_ref().unwrap();
                self.frame.other_slot_materials = self
                    .frame
                    .inventory
                    .iter()
                    .filter(|old| {
                        old.owner == selected.owner
                            && old.day == selected.day
                            && old.slot == selected.slot
                    })
                    .count();
            }
            self.frame.execute("ROLLBACK")?;
            self.frame.tx = TxState::RolledBack;
            self.frame.finish_close()?;
            Ok(())
        })();
        match result {
            Ok(()) => MaterialReadOutcome::Observed(MaterialReadObservation {
                material: self.frame.selected_material.take(),
                observed_generation: self.frame.generation,
                other_slot_materials: self.frame.other_slot_materials,
            }),
            Err(fault) => {
                self.frame.fault(fault);
                MaterialReadOutcome::Held(HeldOutbox { frame: self.frame })
            }
        }
    }
    pub(crate) fn enqueue(
        mut self,
        draft: UnverifiedEvidencePackageDraft,
        expected_generation: i64,
    ) -> EnqueueOutcome {
        self.frame.command = Some(draft); // Own the caller's real value before any admission/phase decision.
        match self.frame.enqueue(expected_generation) {
            Ok(()) => EnqueueOutcome::Stored(StoredUnverified {
                generation: self.frame.command_generation.unwrap(),
                disposition: self.frame.disposition.unwrap(),
            }),
            Err(fault) => {
                self.frame.fault(fault);
                if self.frame.tx == TxState::Unknown {
                    EnqueueOutcome::Pending(PendingOutbox { frame: self.frame })
                } else {
                    EnqueueOutcome::Held(HeldOutbox { frame: self.frame })
                }
            }
        }
    }
    pub(crate) fn observe_previous(
        mut self,
        draft: UnverifiedEvidencePackageDraft,
        generation: i64,
    ) -> RecoveryOutcome {
        self.frame.command = Some(draft);
        self.frame.command_generation = Some(generation);
        let command = self.frame.command.as_ref().unwrap();
        let cost = command.canonical.len()
            + command.id.len()
            + command.sha256.len()
            + command.day.len()
            + command.slot.len();
        if let Err(fault) = self.frame.charge(cost) {
            self.frame.fault(fault);
            return RecoveryOutcome::Held(HeldOutbox { frame: self.frame });
        }
        if !(1..=128).contains(&generation) {
            self.frame.fault(OutboxFault::ForeignShape);
            return RecoveryOutcome::Held(HeldOutbox { frame: self.frame });
        }
        self.frame.tx = TxState::Unknown;
        self.frame.fault(OutboxFault::CommitUnknown);
        PendingOutbox { frame: self.frame }.observe()
    }
    pub(crate) fn close(mut self) -> Result<(), HeldOutbox> {
        match self.frame.finish_close() {
            Ok(()) => Ok(()),
            Err(fault) => {
                self.frame.fault(fault);
                Err(HeldOutbox { frame: self.frame })
            }
        }
    }
}
impl HeldOutbox {
    pub(crate) fn first_fault(&self) -> OutboxFault {
        self.frame.first.unwrap()
    }
    pub(crate) fn drain_resources_once(mut self) -> Self {
        self.frame.abandon_once();
        self
    }
}
impl PendingOutbox {
    pub(crate) fn first_fault(&self) -> OutboxFault {
        self.frame.first.unwrap()
    }
    pub(crate) fn observe(mut self) -> RecoveryOutcome {
        if !self.frame.conn().is_autocommit() {
            return RecoveryOutcome::Pending(self);
        }
        let read = self
            .frame
            .binding_tail(false)
            .and_then(|_| self.frame.read_inventory());
        if let Err(fault) = read {
            self.frame.fault(fault);
            return RecoveryOutcome::Held(HeldOutbox { frame: self.frame });
        }
        let command = self.frame.command.as_ref().unwrap();
        let exact = self
            .frame
            .inventory
            .iter()
            .any(|old| old.id == command.id && old.canonical == command.canonical);
        let result = self.frame.conn().query_row(
            "SELECT count(*) FROM attempt WHERE generation=?1 AND command_id=?2",
            params![self.frame.command_generation, command.id],
            |row| row.get::<_, i64>(0),
        );
        let attempt = match self.frame.sql(result) {
            Ok(n) => n,
            Err(fault) => {
                self.frame.fault(fault);
                return RecoveryOutcome::Held(HeldOutbox { frame: self.frame });
            }
        };
        let presence = match (exact, attempt) {
            (true, 1) => RecoveredPresence::ExactUnverified,
            (false, 0) => RecoveredPresence::MissingFactsUnknown,
            _ => {
                self.frame.fault(OutboxFault::ForeignShape);
                return RecoveryOutcome::Held(HeldOutbox { frame: self.frame });
            }
        };
        if let Err(fault) = self.frame.finish_close() {
            self.frame.fault(fault);
            return RecoveryOutcome::Held(HeldOutbox { frame: self.frame });
        }
        RecoveryOutcome::Observed(RecoveredUnverified {
            presence,
            original_fault: self.frame.first.unwrap(),
        })
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum TestCommitObservation {
    Normal,
    LoseResponse,
    ExitBeforeCommit,
    ExitAfterCommit,
}
#[cfg(test)]
struct TestVm(*mut rusqlite::ffi::sqlite3_stmt);
#[cfg(test)]
impl Drop for TestVm {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                rusqlite::ffi::sqlite3_finalize(self.0);
            }
        }
    }
}
#[cfg(test)]
pub(crate) struct OutboxFixture {
    root: tempfile::TempDir,
}
#[cfg(test)]
impl OutboxFixture {
    pub(crate) fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("retention-outbox-v1-")
            .tempdir()
            .unwrap();
        fs::create_dir(root.path().join("data")).unwrap();
        Self { root }
    }
    pub(crate) fn open(&self) -> Result<UnverifiedOutbox, HeldOutbox> {
        UnverifiedOutbox::open_fixed(self.root.path())
    }
    pub(super) fn root(&self) -> &Path {
        self.root.path()
    }
    pub(crate) fn main(&self) -> PathBuf {
        self.root.path().join("data").join(DIR).join(MAIN)
    }
    pub(crate) fn directory(&self) -> PathBuf {
        self.root.path().join("data").join(DIR)
    }
}
#[cfg(test)]
impl UnverifiedOutbox {
    pub(crate) fn test_observation(mut self, observation: TestCommitObservation) -> Self {
        self.frame.observation = observation;
        self
    }
    pub(crate) fn test_hold_transaction(&mut self) -> Result<(), OutboxFault> {
        FileExt::try_lock_shared(&self.frame.init_lock.as_ref().unwrap().file)
            .map_err(|_| OutboxFault::Busy)?;
        self.frame.init_locked = true;
        self.frame.execute("BEGIN IMMEDIATE")?;
        self.frame.tx = TxState::Active;
        Ok(())
    }
    pub(crate) fn test_rollback(mut self) -> Result<(), HeldOutbox> {
        let result = self.frame.execute("ROLLBACK");
        if let Err(fault) = result {
            self.frame.fault(fault);
            return Err(HeldOutbox { frame: self.frame });
        }
        self.frame.tx = TxState::RolledBack;
        self.close()
    }
    pub(crate) fn test_busy_vm(mut self) -> Self {
        let mut statement = std::ptr::null_mut();
        // Public rusqlite handle, fixed owned SELECT 1, no private layout cast.
        let code = unsafe {
            rusqlite::ffi::sqlite3_prepare_v2(
                self.frame.conn().handle(),
                b"SELECT 1\0".as_ptr().cast(),
                -1,
                &mut statement,
                std::ptr::null_mut(),
            )
        };
        self.frame.vm = Some(TestVm(statement));
        assert_eq!(code, rusqlite::ffi::SQLITE_OK);
        assert_eq!(
            unsafe { rusqlite::ffi::sqlite3_step(statement) },
            rusqlite::ffi::SQLITE_ROW
        );
        self
    }
    pub(super) fn test_corrupt_sql(&mut self, sql: &str) {
        self.frame.execute(sql).unwrap();
    }
    pub(crate) fn test_material_count(&mut self) -> i64 {
        self.frame.scalar("SELECT count(*) FROM material").unwrap()
    }
    pub(crate) fn test_conflict_count(&mut self) -> i64 {
        self.frame
            .scalar("SELECT count(*) FROM slot_conflict")
            .unwrap()
    }
    pub(crate) fn test_spend_owned(&mut self, bytes: usize) -> Result<(), OutboxFault> {
        self.frame
            .charge(bytes)
            .map_err(|fault| self.frame.fault(fault))
    }
    pub(super) fn test_child(
        root: &Path,
        observation: TestCommitObservation,
        draft: UnverifiedEvidencePackageDraft,
    ) -> ! {
        // Only this cfg test call can select a fixture root. Parent passes its
        // own retained private TempDir; production open always uses build root.
        assert!(root.is_absolute());
        assert!(root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("retention-outbox-v1-"));
        let store = match UnverifiedOutbox::open_fixed(root) {
            Ok(store) => store,
            Err(_) => std::process::exit(70),
        };
        let generation = store.generation();
        let outcome = store
            .test_observation(observation)
            .enqueue(draft, generation);
        drop(outcome);
        std::process::exit(71)
    }
}
#[cfg(test)]
impl HeldOutbox {
    pub(crate) fn test_connection_retained(&self) -> bool {
        self.frame.connection.is_some()
    }
    pub(crate) fn test_command_retained(&self) -> bool {
        self.frame.command.is_some()
    }
    pub(crate) fn test_read_id(&self) -> Option<&str> {
        self.frame.read_id.as_deref()
    }
    pub(crate) fn test_selected_material(&self) -> Option<&UnverifiedEvidencePackageDraft> {
        self.frame.selected_material.as_ref()
    }
    pub(super) fn test_inventory_retained(&self) -> bool {
        !self.frame.inventory.is_empty()
    }
    pub(crate) fn test_finalize_then_drain(mut self) -> Self {
        let mut vm = self.frame.vm.take().expect("real held VM");
        let code = unsafe { rusqlite::ffi::sqlite3_finalize(vm.0) };
        vm.0 = std::ptr::null_mut();
        assert_eq!(code, rusqlite::ffi::SQLITE_OK);
        // A new actual finalize fact permits one new consuming close attempt;
        // the first fault remains CloseHeld and never becomes Stored.
        self.frame.close_attempted = false;
        self.frame.abandon_once();
        self
    }
}

#[cfg(test)]
impl PendingOutbox {
    pub(crate) fn test_connection_and_command_retained(&self) -> bool {
        self.frame.connection.is_some() && self.frame.command.is_some()
    }
    pub(crate) fn test_exhaust_same_work(&mut self) -> Result<(), OutboxFault> {
        self.frame.charge(8 * MIB)
    }
}
