//! Private original-source to exact closed-byte-backup row proof.
//! This does not qualify a target, authorize restore, or create a writer/pool.
use super::super::global_schema_catalog_v1::{
    capture_whole_rows_read_spec, RowsSpecWork, SameRuntimeCatalogReferences, WholeRowsReadSpec,
};
use super::*;
use rusqlite::types::ValueRef;
use serde::Serialize;
use sha2::{Digest, Sha256};

const DOMAIN: &[u8] = b"stock_analysis.global_schema.original_source_to_exact_byte_backup_rows.v1";
const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Limits {
    pub(super) tables: u64,
    pub(super) columns: u64,
    pub(super) cell_bytes: u64,
    pub(super) row_bytes: u64,
    pub(super) row_observations: u64,
    pub(super) observation_bytes: u64,
    pub(super) metadata_bytes: u64,
    pub(super) review_bytes: usize,
}
impl Limits {
    pub(super) fn production() -> Self {
        Self {
            tables: 4096,
            columns: 1024,
            cell_bytes: 16 * MIB,
            row_bytes: 64 * MIB,
            row_observations: 64_000_000,
            observation_bytes: 256 * GIB,
            metadata_bytes: 16 * MIB,
            review_bytes: MIB as usize,
        }
    }
    fn bounded_by(&self, max: &Self) -> bool {
        self.tables <= max.tables
            && self.columns <= max.columns
            && self.cell_bytes <= max.cell_bytes
            && self.row_bytes <= max.row_bytes
            && self.row_observations <= max.row_observations
            && self.observation_bytes <= max.observation_bytes
            && self.metadata_bytes <= max.metadata_bytes
            && self.review_bytes <= max.review_bytes
    }
}
pub(super) struct Options {
    pub(super) backup: backup::Options,
    pub(super) limits: Limits,
    // A fixed event sink, never an arbitrary callback after the last reader.
    #[cfg(test)]
    pub(super) trace: Option<std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>>,
}
impl Options {
    pub(super) fn production() -> Self {
        Self {
            backup: backup::Options::production(),
            limits: Limits::production(),
            #[cfg(test)]
            trace: None,
        }
    }
    pub(super) fn validate_mode(&self, mode: BoundMode) -> Result<(), GlobalSchemaV1Error> {
        self.backup.validate_mode(mode)?;
        if !self.limits.bounded_by(&Limits::production()) {
            return Err(fail("rows Test bounds may only decrease"));
        }
        if mode != BoundMode::Test && self.limits != Limits::production() {
            return Err(fail("rows reduced bounds require isolated Test"));
        }
        #[cfg(test)]
        if mode != BoundMode::Test && self.trace.is_some() {
            return Err(fail("rows event sink requires isolated Test"));
        }
        Ok(())
    }
}
fn fail(detail: &'static str) -> GlobalSchemaV1Error {
    prospective::refusal(detail)
}
fn sql_error(source: rusqlite::Error) -> GlobalSchemaV1Error {
    GlobalSchemaV1Error::SelectionSqlite {
        operation: "read exact typed original/backup rows",
        source,
    }
}
fn catalog_error(source: GlobalSchemaCatalogError) -> GlobalSchemaV1Error {
    GlobalSchemaV1Error::SelectionCatalog { source }
}
fn add(left: u64, right: u64) -> Result<u64, GlobalSchemaV1Error> {
    left.checked_add(right)
        .ok_or_else(|| fail("rows checked arithmetic overflow"))
}
fn multiply(left: u64, right: u64) -> Result<u64, GlobalSchemaV1Error> {
    left.checked_mul(right)
        .ok_or_else(|| fail("rows checked arithmetic overflow"))
}

pub(super) struct RowsWork {
    metadata: RowsSpecWork,
    limits: Limits,
    rows: u64,
    bytes: u64,
    streams: u64,
}
impl RowsWork {
    #[cfg(test)]
    fn target_metadata_diagnostic(&self, stage: &str, needed: u64) {
        eprintln!(
            "TEST_CODE target metadata stage={stage} used={} limit={} needed={needed}",
            self.metadata.used(),
            self.limits.metadata_bytes
        );
    }
    fn new(limits: Limits) -> Self {
        Self {
            metadata: RowsSpecWork::new(limits.metadata_bytes, limits.tables, limits.columns),
            limits,
            rows: 0,
            bytes: 0,
            streams: 0,
        }
    }
    pub(super) fn reserve_copy_route(
        &mut self,
        parent: u64,
        leaf: u64,
    ) -> Result<(), GlobalSchemaV1Error> {
        // macOS retained F_GETPATH is at most its fixed 4096-byte route buffer;
        // Linux /proc/self/fd/<i32>/<leaf> is shorter than this same bound.
        // Include both route observations, percent URI and fixed SQL control.
        let route = add(add(parent.max(4096), leaf)?, 128)?;
        self.metadata
            .charge(add(multiply(route, 8)?, 256)?)
            .map_err(catalog_error)
    }
    fn reserve_transcript(&mut self, tables: usize) -> Result<(), GlobalSchemaV1Error> {
        self.metadata
            .charge(multiply(
                tables as u64,
                std::mem::size_of::<TableMatch>() as u64,
            )?)
            .map_err(catalog_error)
    }
    fn row(&mut self) -> Result<(), GlobalSchemaV1Error> {
        self.rows = add(self.rows, 1)?;
        if self.rows > self.limits.row_observations {
            return Err(fail("rows cumulative row observation budget exceeded"));
        }
        Ok(())
    }
    fn value(
        &mut self,
        value: ValueRef<'_>,
        row_bytes: &mut u64,
    ) -> Result<u64, GlobalSchemaV1Error> {
        let bytes = match value {
            ValueRef::Null => 1,
            ValueRef::Integer(_) | ValueRef::Real(_) => 9,
            ValueRef::Text(b) | ValueRef::Blob(b) => {
                if b.len() as u64 > self.limits.cell_bytes {
                    return Err(fail("rows individual cell budget exceeded"));
                }
                add(9, b.len() as u64)?
            }
        };
        *row_bytes = add(*row_bytes, bytes)?;
        if *row_bytes > self.limits.row_bytes {
            return Err(fail("rows individual row budget exceeded"));
        }
        self.bytes = add(self.bytes, bytes)?;
        if self.bytes > self.limits.observation_bytes {
            return Err(fail("rows cumulative typed byte budget exceeded"));
        }
        Ok(bytes)
    }
    fn preflight(
        &mut self,
        view: &OriginalRowsView<'_, '_>,
        spec: &WholeRowsReadSpec,
    ) -> Result<(), GlobalSchemaV1Error> {
        let mut rows = 0;
        let mut bytes = 0;
        for table in spec.tables() {
            // SQL contract generation was reserved before constructing spec.
            let sql = table.preflight_sql();
            let (count, extent, row, cell): (i64, i64, i64, i64) = view
                .connection()
                .query_row(&sql, [], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(sql_error)?;
            let count = u64::try_from(count).map_err(|_| fail("rows negative preflight count"))?;
            let extent =
                u64::try_from(extent).map_err(|_| fail("rows negative preflight byte extent"))?;
            let row = u64::try_from(row).map_err(|_| fail("rows negative preflight row extent"))?;
            let cell =
                u64::try_from(cell).map_err(|_| fail("rows negative preflight cell extent"))?;
            if row > self.limits.row_bytes || cell > self.limits.cell_bytes {
                return Err(fail("rows preflight individual cell/row limit exceeded"));
            }
            rows = add(rows, count)?;
            bytes = add(bytes, extent)?;
        }
        // Four direct streams (two pairs), issue copy, render copy. Reserve
        // their worst fixed cost before any role is created/copied.
        if multiply(rows, 6)? > self.limits.row_observations
            || multiply(bytes, 6)? > self.limits.observation_bytes
        {
            return Err(fail("rows six-stream preflight budget exceeded"));
        }
        Ok(())
    }
    fn finished(&mut self, n: u64) -> Result<(), GlobalSchemaV1Error> {
        self.streams = add(self.streams, n)?;
        if self.streams > 6 {
            return Err(fail("rows stream lifecycle exceeded"));
        }
        Ok(())
    }
}

/// Only this constructor borrows the original owner's actual transaction.
struct OriginalRowsView<'view, 'tx> {
    transaction: &'view Transaction<'tx>,
}
impl OriginalRowsView<'_, '_> {
    fn connection(&self) -> &Connection {
        self.transaction
    }
}
fn original_view<'view, 'tx>(
    snapshot: &'view VerifiedSelectionSchemaSnapshot<'tx, '_>,
) -> OriginalRowsView<'view, 'tx> {
    OriginalRowsView {
        transaction: &snapshot.transaction,
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct TableMatch {
    name: String,
    rows: u64,
    cells: u64,
    typed_bytes: u64,
    sha256: String,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
struct Transcript {
    tables: Vec<TableMatch>,
}
fn same_value(a: ValueRef<'_>, b: ValueRef<'_>) -> bool {
    match (a, b) {
        (ValueRef::Null, ValueRef::Null) => true,
        (ValueRef::Integer(a), ValueRef::Integer(b)) => a == b,
        (ValueRef::Real(a), ValueRef::Real(b)) => a.to_bits() == b.to_bits(),
        (ValueRef::Text(a), ValueRef::Text(b)) | (ValueRef::Blob(a), ValueRef::Blob(b)) => a == b,
        _ => false,
    }
}
fn hash_value(hash: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hash.update([0]),
        ValueRef::Integer(n) => {
            hash.update([1]);
            hash.update(n.to_be_bytes());
        }
        ValueRef::Real(n) => {
            hash.update([2]);
            hash.update(n.to_bits().to_be_bytes());
        }
        ValueRef::Text(b) | ValueRef::Blob(b) => {
            hash.update([if matches!(value, ValueRef::Text(_)) {
                3
            } else {
                4
            }]);
            hash.update((b.len() as u64).to_be_bytes());
            hash.update(b);
        }
    }
}
fn table_hash(name: &str, columns: usize) -> Sha256 {
    let mut h = Sha256::new();
    h.update(DOMAIN);
    h.update([0]);
    h.update((name.len() as u64).to_be_bytes());
    h.update(name.as_bytes());
    h.update((columns as u64).to_be_bytes());
    h
}
// Low-authority primitive: real statements and ValueRef, but this function
// cannot create a source view, a closed spec, Pending, or any capability.
fn read_table_pair(
    source: &Connection,
    copy: &Connection,
    sql: &str,
    name: &str,
    columns: usize,
    work: &mut RowsWork,
) -> Result<TableMatch, GlobalSchemaV1Error> {
    let mut left_stmt = source.prepare(sql).map_err(sql_error)?;
    let mut right_stmt = copy.prepare(sql).map_err(sql_error)?;
    if left_stmt.column_count() != columns + 1 || right_stmt.column_count() != columns + 1 {
        return Err(fail("rows projection shape changed"));
    }
    let mut left = left_stmt.query([]).map_err(sql_error)?;
    let mut right = right_stmt.query([]).map_err(sql_error)?;
    let mut h = table_hash(name, columns);
    let mut rows = 0;
    let mut cells = 0;
    let mut bytes = 0;
    loop {
        match (
            left.next().map_err(sql_error)?,
            right.next().map_err(sql_error)?,
        ) {
            (None, None) => break,
            (Some(a), Some(b)) => {
                work.row()?;
                work.row()?;
                let left_id = a.get_ref(0).map_err(sql_error)?;
                let right_id = b.get_ref(0).map_err(sql_error)?;
                if !matches!(left_id, ValueRef::Integer(_)) || !same_value(left_id, right_id) {
                    return Err(fail("rows actual rowid differs"));
                }
                let mut left_bytes = 0;
                let mut right_bytes = 0;
                for c in 0..=columns {
                    let av = a.get_ref(c).map_err(sql_error)?;
                    let bv = b.get_ref(c).map_err(sql_error)?;
                    let n = work.value(av, &mut left_bytes)?;
                    work.value(bv, &mut right_bytes)?;
                    if !same_value(av, bv) {
                        return Err(fail("rows actual storage class or cell differs"));
                    }
                    bytes = add(bytes, n)?;
                    cells = add(cells, 1)?;
                    hash_value(&mut h, av);
                }
                rows = add(rows, 1)?;
            }
            _ => return Err(fail("rows actual EOF/multiplicity differs")),
        }
    }
    h.update([255]);
    h.update(rows.to_be_bytes());
    work.metadata
        .charge(add(name.len() as u64, 128)?)
        .map_err(catalog_error)?;
    Ok(TableMatch {
        name: name.to_owned(),
        rows,
        cells,
        typed_bytes: bytes,
        sha256: hex::encode(h.finalize()),
    })
}
fn read_table(
    connection: &Connection,
    sql: &str,
    name: &str,
    columns: usize,
    work: &mut RowsWork,
) -> Result<TableMatch, GlobalSchemaV1Error> {
    let mut statement = connection.prepare(sql).map_err(sql_error)?;
    if statement.column_count() != columns + 1 {
        return Err(fail("rows final projection shape changed"));
    }
    let mut cursor = statement.query([]).map_err(sql_error)?;
    let mut h = table_hash(name, columns);
    let mut rows = 0;
    let mut cells = 0;
    let mut bytes = 0;
    while let Some(row) = cursor.next().map_err(sql_error)? {
        work.row()?;
        let mut row_bytes = 0;
        if !matches!(row.get_ref(0).map_err(sql_error)?, ValueRef::Integer(_)) {
            return Err(fail("rows final rowid is not integer"));
        }
        for c in 0..=columns {
            let v = row.get_ref(c).map_err(sql_error)?;
            bytes = add(bytes, work.value(v, &mut row_bytes)?)?;
            cells = add(cells, 1)?;
            hash_value(&mut h, v);
        }
        rows = add(rows, 1)?;
    }
    h.update([255]);
    h.update(rows.to_be_bytes());
    work.metadata
        .charge(add(name.len() as u64, 128)?)
        .map_err(catalog_error)?;
    Ok(TableMatch {
        name: name.to_owned(),
        rows,
        cells,
        typed_bytes: bytes,
        sha256: hex::encode(h.finalize()),
    })
}

pub(super) struct RetainedRowsSourceTail {
    namespace: PinnedNamespace,
    lock: File,
    lock_identity: FileIdentity,
    database_path: PathBuf,
    database_identity: FileIdentity,
    audit_parent: PinnedDirectory,
    audit_leaf: OsString,
    audit_path: PathBuf,
    database: prospective::FileAnchor,
    audit: Option<prospective::FileAnchor>,
}
fn clone_file(file: &File) -> Result<File, GlobalSchemaV1Error> {
    file.try_clone()
        .map_err(|_| fail("rows retain original descriptor failed"))
}
fn clone_directory(dir: &PinnedDirectory) -> Result<PinnedDirectory, GlobalSchemaV1Error> {
    Ok(PinnedDirectory {
        path: dir.path.clone(),
        root: clone_file(&dir.root)?,
        relative_components: dir.relative_components.clone(),
        file: clone_file(&dir.file)?,
        identity: dir.identity,
    })
}
impl RetainedRowsSourceTail {
    fn capture(
        snapshot: &VerifiedSelectionSchemaSnapshot<'_, '_>,
        source: &prospective::Pending,
        work: &mut RowsWork,
    ) -> Result<Self, GlobalSchemaV1Error> {
        let ns = &snapshot.maintenance.namespace;
        let dirs = [&ns.database_parent, &ns.lock_parent, snapshot.audit_parent];
        let mut extent = add(
            ns.root.path.as_os_str().len() as u64,
            snapshot.database_path.as_os_str().len() as u64,
        )?;
        extent = add(extent, snapshot.audit_path.as_os_str().len() as u64)?;
        for d in dirs {
            extent = add(extent, d.path.as_os_str().len() as u64)?;
            for c in &d.relative_components {
                extent = add(extent, c.len() as u64)?;
                extent = add(extent, std::mem::size_of::<OsString>() as u64)?;
            }
        }
        for leaf in [&ns.database_leaf, &ns.lock_leaf, &snapshot.audit_leaf] {
            extent = add(extent, leaf.len() as u64)?;
        }
        work.metadata
            .charge(add(extent, std::mem::size_of::<Self>() as u64 + 256)?)
            .map_err(catalog_error)?;
        ns.validate_unchanged()?;
        let (database, audit) = source.backup_expected();
        let tail = Self {
            namespace: PinnedNamespace {
                root: PinnedRoot {
                    path: ns.root.path.clone(),
                    file: clone_file(&ns.root.file)?,
                    identity: ns.root.identity,
                },
                database_parent: clone_directory(&ns.database_parent)?,
                database_leaf: ns.database_leaf.clone(),
                lock_parent: clone_directory(&ns.lock_parent)?,
                lock_leaf: ns.lock_leaf.clone(),
            },
            lock: clone_file(&snapshot.maintenance.lock_file)?,
            lock_identity: snapshot.maintenance.lock_identity,
            database_path: snapshot.database_path.clone(),
            database_identity: snapshot.database_identity,
            audit_parent: clone_directory(snapshot.audit_parent)?,
            audit_leaf: snapshot.audit_leaf.clone(),
            audit_path: snapshot.audit_path.clone(),
            database,
            audit,
        };
        // These clones never lock/unlock and never implement a lease Drop.
        tail.namespace.validate_unchanged()?;
        ns.validate_unchanged()?;
        Ok(tail)
    }
    fn reserve_validation(&self, work: &mut RowsWork) -> Result<(), GlobalSchemaV1Error> {
        // Before constructing lock/sidecar paths or cloning the two fixed
        // FileAnchor digests in the existing source getter. No new FD read
        // counter is introduced by this metadata reservation.
        let paths = add(
            self.database_path.as_os_str().len() as u64,
            self.namespace.lock_parent.path.as_os_str().len() as u64,
        )?;
        work.metadata
            .charge(add(multiply(paths, 16)?, 1024)?)
            .map_err(catalog_error)
    }
    pub(super) fn validate_without_hooks(
        &self,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        database: &File,
        audit: &PinnedSelectionAuditFile,
        closed: bool,
    ) -> Result<(), GlobalSchemaV1Error> {
        let (expected_database, expected_audit) = source.backup_expected();
        if expected_database != self.database || expected_audit != self.audit {
            return Err(fail("rows tail is not the original byte owner"));
        }
        self.namespace.validate_unchanged()?;
        self.audit_parent.validate_unchanged()?;
        require_same_file_identity(
            &self.namespace.database_parent,
            &self.namespace.database_leaf,
            &self.database_path,
            database,
            self.database_identity,
            "rows original main",
        )?;
        require_same_file_identity(
            &self.namespace.lock_parent,
            &self.namespace.lock_leaf,
            &self
                .namespace
                .lock_parent
                .path
                .join(&self.namespace.lock_leaf),
            &self.lock,
            self.lock_identity,
            "rows retained original lock",
        )?;
        revalidate_selection_audit_file(
            &self.audit_parent,
            &self.audit_leaf,
            &self.audit_path,
            audit,
        )?;
        if source.backup_fingerprint(database, options)? != self.database {
            return Err(fail("rows original main bytes changed"));
        }
        match (&self.audit, audit) {
            (None, PinnedSelectionAuditFile::Missing) => {}
            (Some(expected), PinnedSelectionAuditFile::Present { file, .. })
                if source.backup_fingerprint(file, options)? == *expected => {}
            _ => return Err(fail("rows original audit bytes/presence changed")),
        }
        if closed {
            require_no_live_sidecars_for_bound_namespace(&self.namespace, &self.database_path)?;
        }
        revalidate_selection_audit_file(
            &self.audit_parent,
            &self.audit_leaf,
            &self.audit_path,
            audit,
        )?;
        require_same_file_identity(
            &self.namespace.database_parent,
            &self.namespace.database_leaf,
            &self.database_path,
            database,
            self.database_identity,
            "rows final original main",
        )?;
        require_same_file_identity(
            &self.namespace.lock_parent,
            &self.namespace.lock_leaf,
            &self
                .namespace
                .lock_parent
                .path
                .join(&self.namespace.lock_leaf),
            &self.lock,
            self.lock_identity,
            "rows final retained lock",
        )?;
        self.audit_parent.validate_unchanged()?;
        self.namespace.validate_unchanged()
    }
}

pub(super) struct Pending {
    spec: WholeRowsReadSpec,
    authority: SelectionCatalogCaptureAuthority,
    references: Option<SameRuntimeCatalogReferences>,
    work: RowsWork,
    tail: RetainedRowsSourceTail,
    initial: Option<Transcript>,
    final_match: bool,
    #[cfg(test)]
    trace: Option<std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>>,
}
impl Pending {
    pub(super) fn capture(
        snapshot: &VerifiedSelectionSchemaSnapshot<'_, '_>,
        references: &SameRuntimeCatalogReferences,
        source: &prospective::Pending,
        options: &Options,
    ) -> Result<Self, GlobalSchemaV1Error> {
        // CAST(TEXT AS BLOB) measures the database encoding, while ValueRef::Text
        // returns UTF-8. Require their byte units to agree before preflight and
        // before creating any durable intent or copy role.
        let utf8 = snapshot
            .transaction
            .query_row("PRAGMA main.encoding", [], |row| {
                Ok(matches!(row.get_ref(0)?, ValueRef::Text(bytes) if bytes == b"UTF-8"))
            })
            .map_err(|source| GlobalSchemaV1Error::SelectionSqlite {
                operation: "check Rows source encoding",
                source,
            })?;
        if !utf8 {
            return Err(fail("rows source encoding must be UTF-8"));
        }
        let mut work = RowsWork::new(options.limits.clone());
        let spec = capture_whole_rows_read_spec(
            &snapshot.authority,
            &snapshot.transaction,
            &snapshot.initial_catalog,
            references,
            &mut work.metadata,
        )
        .map_err(catalog_error)?;
        work.preflight(&original_view(snapshot), &spec)?;
        let tail = RetainedRowsSourceTail::capture(snapshot, source, &mut work)?;
        Ok(Self {
            spec,
            authority: SelectionCatalogCaptureAuthority::new(),
            references: None,
            work,
            tail,
            initial: None,
            final_match: false,
            #[cfg(test)]
            trace: options.trace.clone(),
        })
    }
    fn event(&self, event: &'static str) {
        #[cfg(test)]
        if let Some(trace) = &self.trace {
            trace.lock().unwrap().push(event);
        }
        let _ = event;
    }
    pub(super) fn pair(
        &mut self,
        snapshot: &VerifiedSelectionSchemaSnapshot<'_, '_>,
        references: &SameRuntimeCatalogReferences,
        backup: &mut backup::Pending,
        source: &mut prospective::Pending,
        options: &prospective::Options,
        settings: &backup::Settings,
        final_pair: bool,
    ) -> Result<(), GlobalSchemaV1Error> {
        let view = original_view(snapshot);
        let spec = &self.spec;
        let authority = &self.authority;
        let work = &mut self.work;
        let actual = backup.with_copied_rows(source, options, settings, work, |copy, work| {
            spec.validate_connection(authority, copy.connection(), references, &mut work.metadata)
                .map_err(catalog_error)?;
            work.reserve_transcript(spec.tables().len())?;
            let mut tables = Vec::with_capacity(spec.tables().len());
            for table in spec.tables() {
                tables.push(read_table_pair(
                    view.connection(),
                    copy.connection(),
                    table.sql(),
                    table.name(),
                    table.column_count(),
                    work,
                )?);
            }
            work.finished(2)?;
            Ok(Transcript { tables })
        })?;
        if final_pair {
            if self.final_match || self.initial.as_ref() != Some(&actual) {
                return Err(fail("rows last same-TX pair differs from initial match"));
            }
            self.final_match = true;
            self.event("rows_final_pair_closed");
        } else {
            if self.initial.is_some() {
                return Err(fail("rows initial pair repeated"));
            }
            self.initial = Some(actual);
            self.event("rows_initial_pair_closed");
        }
        Ok(())
    }
    pub(super) fn before_commit_tail(
        &mut self,
        snapshot: &mut VerifiedSelectionSchemaSnapshot<'_, '_>,
        source: &mut prospective::Pending,
        options: &prospective::Options,
    ) -> Result<(), GlobalSchemaV1Error> {
        if !self.final_match {
            return Err(fail("rows final pair missing"));
        }
        self.tail.reserve_validation(&mut self.work)?;
        self.tail.validate_without_hooks(
            source,
            options,
            snapshot.database_file,
            snapshot.audit_file,
            false,
        )?;
        self.work
            .metadata
            .before_catalog_capture(&snapshot.transaction)
            .map_err(catalog_error)?;
        super::super::global_schema_catalog_v1::require_empty_temp_for_rows(&snapshot.transaction)
            .map_err(catalog_error)?;
        if capture_catalog_snapshot(
            &snapshot.authority,
            &snapshot.transaction,
            snapshot.catalog_mode,
        )
        .map_err(catalog_error)?
            != snapshot.initial_catalog
            || capture_selection_pragmas(&snapshot.transaction)? != snapshot.initial_pragmas
            || capture_selection_integrity(&snapshot.transaction)? != snapshot.initial_integrity
        {
            return Err(fail("rows final original TX evidence changed"));
        }
        if snapshot
            .audit_session
            .validated_records()
            .map_err(|source| GlobalSchemaV1Error::SelectionAudit { source })?
            != snapshot.initial_audit
        {
            return Err(fail("rows final original audit prefix changed"));
        }
        snapshot
            .inspection_sidecars
            .validate_present_exact(&snapshot.maintenance.namespace, &snapshot.database_path)?;
        prospective::require_zero_owned_wal(snapshot.inspection_sidecars)?;
        self.event("rows_precommit_tail_complete");
        Ok(())
    }
    pub(super) fn transaction_finished(
        &mut self,
        references: SameRuntimeCatalogReferences,
    ) -> Result<(), GlobalSchemaV1Error> {
        if !self.final_match || self.work.streams != 4 || self.references.is_some() {
            return Err(fail("rows original transaction lifecycle incomplete"));
        }
        self.references = Some(references);
        Ok(())
    }
    fn read_last_copy(
        &mut self,
        backup: &mut backup::VerifiedUnapprovedByteBackup,
    ) -> Result<(), GlobalSchemaV1Error> {
        let references = self
            .references
            .as_ref()
            .ok_or_else(|| fail("rows actual original transaction not finished"))?;
        let spec = &self.spec;
        let authority = &self.authority;
        let work = &mut self.work;
        let actual = backup.with_copied_rows(work, |copy, work| {
            spec.validate_connection(authority, copy.connection(), references, &mut work.metadata)
                .map_err(catalog_error)?;
            work.reserve_transcript(spec.tables().len())?;
            let mut tables = Vec::with_capacity(spec.tables().len());
            for table in spec.tables() {
                tables.push(read_table(
                    copy.connection(),
                    table.sql(),
                    table.name(),
                    table.column_count(),
                    work,
                )?);
            }
            work.finished(1)?;
            Ok(Transcript { tables })
        })?;
        if self.initial.as_ref() != Some(&actual) {
            return Err(fail("rows final actual Copied transcript differs"));
        }
        self.event(if self.work.streams == 5 {
            "rows_issue_reader_closed"
        } else {
            "rows_render_reader_closed"
        });
        self.tail.reserve_validation(&mut self.work)?;
        backup.validate_rows_tail_without_hooks(&self.tail)?;
        self.event("rows_hook_free_tail_complete");
        Ok(())
    }
    pub(super) fn issue(
        mut self,
        mut backup: backup::VerifiedUnapprovedByteBackup,
    ) -> Result<VerifiedUnapprovedOriginalRowsBackup, GlobalSchemaV1Error> {
        backup.validate_rows_preliminary()?;
        self.read_last_copy(&mut backup)?;
        if self.work.streams != 5 {
            return Err(fail("rows issue stream count is not five"));
        }
        Ok(VerifiedUnapprovedOriginalRowsBackup {
            backup,
            pending: self,
        })
    }
}
/// Non-Clone, non-Deserialize. It retains the original byte cap/exclusive lease
/// until the final reader, hook-free tail and bounded encoding have returned.
pub(super) struct VerifiedUnapprovedOriginalRowsBackup {
    backup: backup::VerifiedUnapprovedByteBackup,
    pending: Pending,
}
impl VerifiedUnapprovedOriginalRowsBackup {
    pub(super) fn into_target_source(
        mut self,
    ) -> Result<OriginalRowsTargetSource, GlobalSchemaV1Error> {
        #[cfg(test)]
        self.pending
            .work
            .target_metadata_diagnostic("consume_before_recipe", 0);
        let recipe = self
            .pending
            .spec
            .select_exact_amended_catalog6()
            .map_err(catalog_error)?;
        self.backup.before_rows_render()?;
        self.pending.read_last_copy(&mut self.backup)?;
        #[cfg(test)]
        self.pending
            .work
            .target_metadata_diagnostic("consume_after_original_six", 0);
        if self.pending.work.streams != 6 {
            return Err(fail("target requires exactly six original streams"));
        }
        self.pending
            .work
            .metadata
            .charge(2 * std::mem::size_of::<Transcript>() as u64)
            .map_err(catalog_error)?;
        Ok(OriginalRowsTargetSource {
            original: self,
            recipe,
            target_streams: 0,
            target_comparisons: Vec::with_capacity(2),
        })
    }
    pub(super) fn render_unapproved(mut self) -> Result<String, GlobalSchemaV1Error> {
        self.backup.before_rows_render()?;
        self.pending.read_last_copy(&mut self.backup)?;
        if self.pending.work.streams != 6 {
            return Err(fail("rows render stream count is not six"));
        }
        #[derive(Serialize)]
        struct Review<'a> {
            version: u16,
            domain: &'static str,
            capability_scope: &'static str,
            row_preservation_proof: bool,
            target_row_preservation_proof: bool,
            tables: &'a [TableMatch],
            row_observations: u64,
            typed_observation_bytes: u64,
            metadata_work: u64,
            streams: u64,
            approval: &'static str,
            exchange: &'static str,
            restore: &'static str,
            apply_supported: bool,
            apply_blocker: &'static str,
        }
        let review = Review {
            version: 1,
            domain: "stock_analysis.global_schema.original_source_to_exact_byte_backup_rows.v1",
            capability_scope: "original_source_to_exact_byte_backup_rows",
            row_preservation_proof: true,
            target_row_preservation_proof: false,
            tables: &self
                .pending
                .initial
                .as_ref()
                .ok_or_else(|| fail("rows render original transcript missing"))?
                .tables,
            row_observations: self.pending.work.rows,
            typed_observation_bytes: self.pending.work.bytes,
            metadata_work: self.pending.work.metadata.used(),
            streams: self.pending.work.streams,
            approval: "not_granted",
            exchange: "not_implemented",
            restore: "not_authorized",
            apply_supported: false,
            apply_blocker: super::super::selection_v2::SELECTION_V2_APPLY_BLOCKER,
        };
        let bytes = prospective::bounded_json(&review, self.pending.work.limits.review_bytes)?;
        String::from_utf8(bytes).map_err(|_| fail("rows bounded review encoding failed"))
    }
}

/// One-way move out of the original Rows renderer. The original counters,
/// transcript, actual backup and exclusive lease remain the same objects.
pub(super) struct OriginalRowsTargetSource {
    original: VerifiedUnapprovedOriginalRowsBackup,
    recipe: super::super::global_schema_catalog_v1::ClosedRequalificationRecipe,
    target_streams: u64,
    target_comparisons: Vec<Transcript>,
}
#[derive(Serialize)]
pub(super) struct TargetRowsObservation {
    pub(super) original_streams: u64,
    pub(super) target_streams: u64,
    pub(super) row_observations: u64,
    pub(super) typed_observation_bytes: u64,
    pub(super) metadata_work: u64,
}
impl OriginalRowsTargetSource {
    pub(super) fn recipe(&self) -> &str {
        self.recipe.id()
    }
    pub(super) fn is_test(&self) -> bool {
        self.recipe.is_test()
    }
    // Constructs only the target's resource meter from sealed source bounds.
    // The caller has already validated the target limit; no authority is issued.
    pub(super) fn new_target_metadata_meter(&self, limit: u64) -> RowsSpecWork {
        let limits = &self.original.pending.work.limits;
        RowsSpecWork::new(limit, limits.tables, limits.columns)
    }
    pub(super) fn with_namespace<T>(
        &self,
        operation: impl for<'loan> FnOnce(&'loan PinnedNamespace) -> Result<T, GlobalSchemaV1Error>,
    ) -> Result<T, GlobalSchemaV1Error> {
        self.original.pending.tail.namespace.validate_unchanged()?;
        operation(&self.original.pending.tail.namespace)
    }
    pub(super) fn original_binding(
        &mut self,
        work: &mut target::TargetWork,
    ) -> Result<String, GlobalSchemaV1Error> {
        #[derive(Serialize)]
        struct Binding<'a> {
            domain: &'static str,
            limits: &'a Limits,
            tables: &'a Transcript,
        }
        let pending = &mut self.original.pending;
        let evidence = Binding {
            domain: "stock_analysis.global_schema.original_source_to_exact_byte_backup_rows.v1",
            limits: &pending.work.limits,
            tables: pending
                .initial
                .as_ref()
                .ok_or_else(|| fail("target missing original transcript"))?,
        };
        let length = target::encoded_len(&evidence, 1024 * 1024)?;
        #[cfg(test)]
        pending
            .work
            .target_metadata_diagnostic("original_proof_binding", length);
        pending
            .work
            .metadata
            .charge(length)
            .map_err(catalog_error)?;
        let bytes = work.encode_metadata(&evidence, 1024 * 1024)?;
        String::from_utf8(bytes).map_err(|_| fail("target original proof encoding"))
    }
    pub(super) fn with_copy_origin<T>(
        &mut self,
        work: &mut target::TargetWork,
        operation: impl for<'loan> FnOnce(
            &mut backup::CopiedTargetOriginLoan<'loan>,
            &mut target::TargetWork,
        ) -> Result<T, GlobalSchemaV1Error>,
    ) -> Result<T, GlobalSchemaV1Error> {
        self.original.backup.with_target_origin(
            &mut self.original.pending.work.metadata,
            work,
            operation,
        )
    }
    #[cfg(test)]
    pub(super) fn test_original_origin_metadata_accounting(
        &mut self,
        work: &mut target::TargetWork,
    ) {
        assert!(self.is_test());
        let reservation = self.original.backup.target_metadata_reservation().unwrap();
        let remaining = reservation.checked_mul(3).unwrap() - 1;
        let original = &mut self.original.pending.work;
        let spend = original
            .limits
            .metadata_bytes
            .checked_sub(original.metadata.used())
            .unwrap()
            .checked_sub(remaining)
            .unwrap();
        original.metadata.charge(spend).unwrap();
        let initial = original.metadata.used();
        for completed in 1..=2 {
            let journal_before = self.original.backup.target_test_original_journal_work();
            self.with_copy_origin(work, |loan, work| loan.binding(work))
                .unwrap();
            assert_eq!(
                self.original.pending.work.metadata.used(),
                initial + completed * reservation
            );
            assert!(self.original.backup.target_test_original_journal_work() > journal_before);
        }
        let journal_before = self.original.backup.target_test_original_journal_work();
        let target_before = work.metadata_used();
        assert!(
            target_before.checked_add(reservation + 512).unwrap() < 16 * MIB,
            "target pool could pay the loan but must not fund original validation"
        );
        let mut entered = false;
        let error = self
            .with_copy_origin(work, |_, _| {
                entered = true;
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(error, GlobalSchemaV1Error::SelectionCatalog {
            source: GlobalSchemaCatalogError::CatalogMismatch { detail }
        } if detail == "whole rows: metadata work exceeded"));
        assert!(
            !entered,
            "exhausted original metadata must refuse before the loan"
        );
        assert_eq!(
            self.original.backup.target_test_original_journal_work(),
            journal_before,
            "exhausted original metadata must refuse before original validation allocation/read"
        );
        assert_eq!(work.metadata_used(), target_before);
        assert_eq!(
            self.original.pending.work.metadata.used(),
            initial + 3 * reservation
        );
        assert_eq!(
            self.original.pending.work.metadata.used(),
            self.original.pending.work.limits.metadata_bytes + 1
        );
        assert_eq!(self.original.pending.work.streams, 6);
    }
    pub(super) fn pair_evidence(&self) -> impl Serialize + '_ {
        &self.target_comparisons
    }
    pub(super) fn observation(&self) -> TargetRowsObservation {
        let w = &self.original.pending.work;
        TargetRowsObservation {
            original_streams: w.streams,
            target_streams: self.target_streams,
            row_observations: w.rows,
            typed_observation_bytes: w.bytes,
            metadata_work: w.metadata.used(),
        }
    }
    pub(super) fn compare_owned_target(
        &mut self,
        target: &target::RetainedTargetReader<'_>,
        target_work: &mut target::TargetWork,
    ) -> Result<(), GlobalSchemaV1Error> {
        if self.original.pending.work.streams != 6 || !matches!(self.target_streams, 0 | 2) {
            return Err(fail("target fixed pair lifecycle exceeded"));
        }
        let journal_metadata = self.original.backup.target_metadata_reservation()?;
        #[cfg(test)]
        self.original
            .pending
            .work
            .target_metadata_diagnostic("pair_before_original_journal", journal_metadata);
        self.original
            .pending
            .work
            .metadata
            .charge(journal_metadata)
            .map_err(catalog_error)?;
        let p = &mut self.original.pending;
        let references = p
            .references
            .as_ref()
            .ok_or_else(|| fail("target original references missing"))?;
        let spec = &p.spec;
        let authority = &p.authority;
        let expected = p
            .initial
            .as_ref()
            .ok_or_else(|| fail("target original transcript missing"))?;
        let work = &mut p.work;
        // Both typed sides consume the original cumulative allowance.
        // The old finished()/six-stream proof lifecycle is never expanded.
        let actual = self.original.backup.with_copied_rows(work, |copy, work| {
            #[cfg(test)]
            target_work.metadata_diagnostic("pair_before_actual_target_catalog");
            spec.validate_connection(
                authority,
                target.connection(),
                references,
                target_work.catalog_metadata(),
            )
            .map_err(|error| {
                #[cfg(test)]
                target_work.metadata_diagnostic("pair_target_catalog_rejected");
                catalog_error(error)
            })?;
            #[cfg(test)]
            target_work.metadata_diagnostic("pair_after_actual_target_catalog");
            let rows = expected.tables.iter().try_fold(0, |n, t| add(n, t.rows))?;
            let bytes = expected
                .tables
                .iter()
                .try_fold(0, |n, t| add(n, t.typed_bytes))?;
            if add(work.rows, multiply(rows, 2)?)? > work.limits.row_observations
                || add(work.bytes, multiply(bytes, 2)?)? > work.limits.observation_bytes
            {
                return Err(fail(
                    "target pair exceeds remaining original Rows allowance",
                ));
            }
            // The original Copied bytes are checked by the loan on both sides;
            // its six original streams already closed the catalog/cell bounds.
            // The target pre-reader fingerprint binds those same individual
            // bounds. No redundant catalog/preflight allocation grants budget.
            work.reserve_transcript(spec.tables().len())?;
            let mut tables = Vec::with_capacity(spec.tables().len());
            for table in spec.tables() {
                target.comparator_entered();
                tables.push(read_table_pair(
                    copy.connection(),
                    target.connection(),
                    table.sql(),
                    table.name(),
                    table.column_count(),
                    work,
                )?);
            }
            Ok(Transcript { tables })
        })?;
        if &actual != expected {
            return Err(fail("target pair differs from original proof transcript"));
        }
        self.target_comparisons.push(actual);
        self.target_streams = add(self.target_streams, 2)?;
        Ok(())
    }
    pub(super) fn validate_without_hooks(&mut self) -> Result<(), GlobalSchemaV1Error> {
        let journal_metadata = self.original.backup.target_metadata_reservation()?;
        #[cfg(test)]
        self.original
            .pending
            .work
            .target_metadata_diagnostic("tail_before_original_journal", journal_metadata);
        self.original
            .pending
            .work
            .metadata
            .charge(journal_metadata)
            .map_err(catalog_error)?;
        self.original
            .pending
            .tail
            .reserve_validation(&mut self.original.pending.work)?;
        self.original
            .backup
            .validate_rows_tail_without_hooks(&self.original.pending.tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair_db(ddl: &str, insert: &str) -> (Connection, Connection) {
        let a = Connection::open_in_memory().unwrap();
        let b = Connection::open_in_memory().unwrap();
        for c in [&a, &b] {
            c.execute_batch(ddl).unwrap();
            c.execute_batch(insert).unwrap();
        }
        (a, b)
    }
    fn compare(a: &Connection, b: &Connection) -> Result<TableMatch, GlobalSchemaV1Error> {
        read_table_pair(
            a,
            b,
            "SELECT _rowid_,v FROM sample ORDER BY _rowid_",
            "sample",
            1,
            &mut RowsWork::new(Limits::production()),
        )
    }
    #[test]
    fn rows_backup_comparator_same_count_cell_and_real_bits_are_actual() {
        let (a, b) = pair_db(
            "CREATE TABLE sample(v)",
            "INSERT INTO sample(rowid,v) VALUES(-9,1),(30,1.125),(100,NULL)",
        );
        let expected = compare(&a, &b).unwrap();
        assert_eq!(expected.rows, 3);
        assert_eq!(
            a.query_row("SELECT v FROM sample WHERE rowid=30", [], |r| Ok(
                matches!(r.get_ref(0)?,ValueRef::Real(n) if n.to_bits()==1.125_f64.to_bits())
            ))
            .unwrap(),
            true
        );
        b.execute("UPDATE sample SET v=1.25 WHERE rowid=30", [])
            .unwrap();
        assert_eq!(
            b.query_row("SELECT COUNT(*) FROM sample", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        let err = compare(&a, &b).unwrap_err();
        assert!(
            matches!(err,GlobalSchemaV1Error::SelectionSnapshotChanged{detail} if detail=="rows actual storage class or cell differs")
        );
    }
    #[test]
    fn rows_backup_comparator_storage_classes_and_raw_text_blob_are_distinct() {
        for (left, right) in [
            ("1", "1.0"),
            ("'1'", "X'31'"),
            ("NULL", "0"),
            ("CAST(X'FF0041' AS TEXT)", "CAST(X'FF0042' AS TEXT)"),
            ("X'FF0041'", "X'FF0042'"),
        ] {
            let (a, b) = pair_db(
                "CREATE TABLE sample(v)",
                "INSERT INTO sample(rowid,v) VALUES(7,NULL)",
            );
            a.execute_batch(&format!("UPDATE sample SET v={left}"))
                .unwrap();
            b.execute_batch(&format!("UPDATE sample SET v={right}"))
                .unwrap();
            let actual = a
                .query_row("SELECT v FROM sample", [], |r| {
                    Ok(match r.get_ref(0)? {
                        ValueRef::Null => (0, vec![]),
                        ValueRef::Integer(n) => (1, n.to_be_bytes().to_vec()),
                        ValueRef::Real(n) => (2, n.to_bits().to_be_bytes().to_vec()),
                        ValueRef::Text(b) => (3, b.to_vec()),
                        ValueRef::Blob(b) => (4, b.to_vec()),
                    })
                })
                .unwrap();
            if left == "1" {
                assert_eq!(actual.0, 1);
                assert!(b
                    .query_row("SELECT v FROM sample", [], |r| Ok(matches!(
                        r.get_ref(0)?,
                        ValueRef::Real(_)
                    )))
                    .unwrap());
            }
            if left == "'1'" {
                assert_eq!(actual, (3, b"1".to_vec()));
                assert!(b
                    .query_row("SELECT v FROM sample", [], |r| Ok(
                        matches!(r.get_ref(0)?,ValueRef::Blob(b) if b==b"1")
                    ))
                    .unwrap());
            }
            if left.starts_with("CAST") {
                assert_eq!(actual, (3, vec![255, 0, 65]));
            }
            let err = compare(&a, &b).unwrap_err();
            assert!(
                matches!(err,GlobalSchemaV1Error::SelectionSnapshotChanged{detail} if detail=="rows actual storage class or cell differs")
            );
        }
        let (a, b) = pair_db(
            "CREATE TABLE sample(v)",
            "INSERT INTO sample VALUES(CAST(X'FF0041' AS TEXT)),(X'FF0041')",
        );
        assert_eq!(compare(&a, &b).unwrap().rows, 2);
    }
    #[test]
    fn rows_backup_comparator_rowid_duplicate_multiplicity_and_both_eofs() {
        let (a, b) = pair_db(
            "CREATE TABLE sample(v)",
            "INSERT INTO sample(rowid,v) VALUES(-10,'same'),(50,'same')",
        );
        assert_eq!(compare(&a, &b).unwrap().rows, 2);
        b.execute("UPDATE sample SET rowid=51 WHERE rowid=50", [])
            .unwrap();
        assert!(
            matches!(compare(&a,&b),Err(GlobalSchemaV1Error::SelectionSnapshotChanged{detail}) if detail=="rows actual rowid differs")
        );
        b.execute("UPDATE sample SET rowid=50 WHERE rowid=51", [])
            .unwrap();
        b.execute("DELETE FROM sample WHERE rowid=50", []).unwrap();
        assert!(
            matches!(compare(&a,&b),Err(GlobalSchemaV1Error::SelectionSnapshotChanged{detail}) if detail=="rows actual EOF/multiplicity differs")
        );
        assert!(
            matches!(compare(&b,&a),Err(GlobalSchemaV1Error::SelectionSnapshotChanged{detail}) if detail=="rows actual EOF/multiplicity differs")
        );
    }
    #[test]
    fn rows_backup_comparator_sequence_all_rows_types_and_duplicates() {
        let (a,b)=pair_db("CREATE TABLE sample(id INTEGER PRIMARY KEY AUTOINCREMENT);","INSERT INTO sample DEFAULT VALUES;INSERT INTO sqlite_sequence(name,seq) VALUES('sample',55),('unknown',X'31')");
        let sql = "SELECT _rowid_,name,seq FROM sqlite_sequence ORDER BY _rowid_";
        let run = |left: &Connection, right: &Connection| {
            read_table_pair(
                left,
                right,
                sql,
                "sqlite_sequence",
                2,
                &mut RowsWork::new(Limits::production()),
            )
        };
        assert_eq!(run(&a, &b).unwrap().rows, 3);
        assert!(a
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name='unknown'",
                [],
                |r| Ok(matches!(r.get_ref(0)?,ValueRef::Blob(v) if v==b"1"))
            )
            .unwrap());
        b.execute(
            "UPDATE sqlite_sequence SET seq=CAST(seq AS TEXT) WHERE name='unknown'",
            [],
        )
        .unwrap();
        assert!(
            matches!(run(&a,&b),Err(GlobalSchemaV1Error::SelectionSnapshotChanged{detail}) if detail=="rows actual storage class or cell differs")
        );
        b.execute(
            "UPDATE sqlite_sequence SET seq=X'31' WHERE name='unknown'",
            [],
        )
        .unwrap();
        b.execute(
            "UPDATE sqlite_sequence SET seq=56 WHERE name='sample' AND seq=55",
            [],
        )
        .unwrap();
        assert!(run(&a, &b).is_err());
    }
    #[test]
    fn rows_backup_work_is_cumulative_and_checked_without_reset_or_truncation() {
        let (a, b) = pair_db("CREATE TABLE sample(v)", "INSERT INTO sample VALUES('abc')");
        let mut limits = Limits::production();
        limits.row_observations = 5;
        let mut work = RowsWork::new(limits);
        for _ in 0..2 {
            read_table_pair(
                &a,
                &b,
                "SELECT rowid,v FROM sample ORDER BY rowid",
                "sample",
                1,
                &mut work,
            )
            .unwrap();
        }
        assert!(read_table_pair(
            &a,
            &b,
            "SELECT rowid,v FROM sample ORDER BY rowid",
            "sample",
            1,
            &mut work
        )
        .is_err());
        assert_eq!(work.rows, 6);
        let mut work = RowsWork::new(Limits::production());
        work.rows = u64::MAX;
        assert!(work.row().is_err());
        work.bytes = u64::MAX;
        assert!(work.value(ValueRef::Null, &mut 0).is_err());
        assert!(multiply(u64::MAX, 6).is_err());
        let mut limits = Limits::production();
        limits.cell_bytes = 2;
        let mut work = RowsWork::new(limits);
        assert!(work.value(ValueRef::Text(b"abc"), &mut 0).is_err());
        assert_eq!(work.bytes, 0);
        let mut limits = Limits::production();
        limits.row_bytes = 9;
        let mut work = RowsWork::new(limits);
        let mut row = 0;
        work.value(ValueRef::Integer(1), &mut row).unwrap();
        assert!(work.value(ValueRef::Null, &mut row).is_err());
    }
}
