//! Unapproved prospective observations; no migration, pool, or apply authority.
use super::super::global_schema_catalog_v1::{
    prospective_target_reference, ProspectiveCatalogReference, SameRuntimeCatalogReferences,
};
use super::*;
use crate::selection::audit::AuditReadLimits;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::FileExt as UnixFileExt;

const DOMAIN: &[u8] = b"stock_analysis.global_schema.prospective_prepare.v1";

pub(super) struct Options {
    pub(super) max_file_bytes: u64,
    pub(super) max_total_hash_bytes: u64,
    pub(super) audit_limits: AuditReadLimits,
    pub(super) max_catalog_objects: u64,
    pub(super) max_catalog_bytes: u64,
    pub(super) max_review_bytes: usize,
    #[cfg(test)]
    pub(super) hook: Option<Box<dyn Fn(Phase) -> Result<(), GlobalSchemaV1Error>>>,
}

impl Options {
    pub(super) fn production() -> Self {
        Self {
            max_file_bytes: 16 * 1024 * 1024 * 1024,
            max_total_hash_bytes: 64 * 1024 * 1024 * 1024,
            audit_limits: AuditReadLimits {
                max_scan_bytes: 32 * 1024 * 1024,
                max_total_scan_bytes: 256 * 1024 * 1024,
                max_records: 65_536,
            },
            max_catalog_objects: 4096,
            max_catalog_bytes: 16 * 1024 * 1024,
            max_review_bytes: 1024 * 1024,
            #[cfg(test)]
            hook: None,
        }
    }

    pub(super) fn validate_mode(&self, mode: BoundMode) -> Result<(), GlobalSchemaV1Error> {
        #[cfg(test)]
        if mode != BoundMode::Test
            && (self.hook.is_some() || !self.same_limits(&Self::production()))
        {
            return Err(refusal(
                "Test restrictions require an actual isolated Test namespace",
            ));
        }
        let maximum = Self::production();
        if self.max_file_bytes > maximum.max_file_bytes
            || self.max_total_hash_bytes > maximum.max_total_hash_bytes
            || self.audit_limits.max_scan_bytes > maximum.audit_limits.max_scan_bytes
            || self.audit_limits.max_total_scan_bytes > maximum.audit_limits.max_total_scan_bytes
            || self.audit_limits.max_records > maximum.audit_limits.max_records
            || self.max_catalog_objects > maximum.max_catalog_objects
            || self.max_catalog_bytes > maximum.max_catalog_bytes
            || self.max_review_bytes > maximum.max_review_bytes
        {
            return Err(refusal(
                "prospective Test limits may only reduce fixed production bounds",
            ));
        }
        let _ = mode;
        Ok(())
    }

    #[cfg(test)]
    fn same_limits(&self, other: &Self) -> bool {
        self.max_file_bytes == other.max_file_bytes
            && self.max_total_hash_bytes == other.max_total_hash_bytes
            && self.audit_limits.max_scan_bytes == other.audit_limits.max_scan_bytes
            && self.audit_limits.max_total_scan_bytes == other.audit_limits.max_total_scan_bytes
            && self.audit_limits.max_records == other.audit_limits.max_records
            && self.max_catalog_objects == other.max_catalog_objects
            && self.max_catalog_bytes == other.max_catalog_bytes
            && self.max_review_bytes == other.max_review_bytes
    }

    pub(super) fn phase(&self, phase: Phase) -> Result<(), GlobalSchemaV1Error> {
        #[cfg(test)]
        if let Some(hook) = &self.hook {
            return hook(phase);
        }
        let _ = phase;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    BeforeInitialCapture,
    AfterInitialCapture,
    BeforeFinalCapture,
    AfterReadOnlyCommit,
    AfterSidecarCleanup,
    AfterPhysicalReads,
    BeforeRender,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct DirectoryAnchor {
    device: u64,
    inode: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct FileAnchor {
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) length: u64,
    pub(super) links: u64,
    pub(super) sha256: String,
}
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum AuditAnchor {
    Absent,
    Present {
        file: FileAnchor,
        record_count: usize,
        tail_hash: Option<String>,
    },
}
#[derive(Serialize)]
struct Review {
    version: u16,
    domain: &'static str,
    capability_scope: &'static str,
    mode: &'static str,
    observed_at: String,
    database_relative_identity: String,
    root: DirectoryAnchor,
    database_parent: DirectoryAnchor,
    maintenance_parent: DirectoryAnchor,
    maintenance_device: u64,
    maintenance_inode: u64,
    audit_parent: DirectoryAnchor,
    database: FileAnchor,
    audit: AuditAnchor,
    original_application_id: i64,
    original_user_version: i64,
    selection_counts_diagnostic_only: BTreeMap<String, i64>,
    legacy_counts_diagnostic_only: BTreeMap<String, i64>,
    target: ProspectiveCatalogReference,
    owned_wal_bytes_during_sql: u64,
    source_scope: &'static str,
    approval: &'static str,
    backup: &'static str,
    exchange: &'static str,
    maintenance_receipt: &'static str,
    apply_supported: bool,
    apply_blocker: &'static str,
}

pub(super) struct Pending {
    review: Review,
    hashed_bytes: u64,
}

/// Private and non-Clone. Rendering consumes it; this never enters a DB pool.
pub(super) struct PreparedGlobalSchemaProspective {
    pending: Pending,
    options: Options,
    database_file: File,
    database_identity: FileIdentity,
    audit_parent: PinnedDirectory,
    audit_file: PinnedSelectionAuditFile,
    maintenance: ExclusiveGlobalSchemaMaintenanceLease,
}

impl fmt::Debug for PreparedGlobalSchemaProspective {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnapprovedGlobalSchemaProspective")
            .finish_non_exhaustive()
    }
}

pub(super) fn render_error(error: &GlobalSchemaV1Error) -> String {
    let reason = match error {
        GlobalSchemaV1Error::SelectionAudit { source } => source.code(),
        _ => error.code(),
    };
    format!("unapproved prospective prepare refused: {reason}")
}

pub(super) fn refusal(detail: &'static str) -> GlobalSchemaV1Error {
    GlobalSchemaV1Error::SelectionSnapshotChanged {
        detail: detail.into(),
    }
}

fn directory_anchor(file: &File) -> Result<DirectoryAnchor, GlobalSchemaV1Error> {
    let metadata = file
        .metadata()
        .map_err(|_| refusal("prospective directory metadata unavailable"))?;
    if !metadata.is_dir() || metadata.nlink() == 0 {
        return Err(refusal("prospective directory is not retained"));
    }
    Ok(DirectoryAnchor {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn fingerprint(
    file: &File,
    options: &Options,
    total: &mut u64,
) -> Result<FileAnchor, GlobalSchemaV1Error> {
    let before = file
        .metadata()
        .map_err(|_| refusal("prospective source metadata unavailable"))?;
    if !before.is_file() || before.nlink() != 1 {
        return Err(refusal(
            "prospective source must have one retained regular-file link",
        ));
    }
    if before.len() > options.max_file_bytes
        || total
            .checked_add(before.len())
            .is_none_or(|n| n > options.max_total_hash_bytes)
    {
        return Err(refusal("prospective physical bytes budget exceeded"));
    }
    let mut digest = Sha256::new();
    let mut chunk = [0_u8; 65_536];
    let mut offset = 0_u64;
    loop {
        // The last read is at most one stack sentinel beyond the captured size.
        let room = before.len().saturating_sub(offset);
        let request = if room >= chunk.len() as u64 {
            chunk.len()
        } else {
            room as usize + 1
        };
        let count = file
            .read_at(&mut chunk[..request], offset)
            .map_err(|_| refusal("prospective source read failed"))?;
        if count == 0 {
            break;
        }
        offset = offset
            .checked_add(count as u64)
            .ok_or_else(|| refusal("prospective source extent overflow"))?;
        if offset > before.len() {
            return Err(refusal("prospective source grew during hashing"));
        }
        *total = total
            .checked_add(count as u64)
            .ok_or_else(|| refusal("prospective hash budget overflow"))?;
        digest.update(&chunk[..count]);
    }
    let after = file
        .metadata()
        .map_err(|_| refusal("prospective source final metadata unavailable"))?;
    if offset != before.len()
        || FileIdentity::from_metadata(&after) != FileIdentity::from_metadata(&before)
        || after.nlink() != 1
    {
        return Err(refusal("prospective source changed during hashing"));
    }
    Ok(FileAnchor {
        device: before.dev(),
        inode: before.ino(),
        length: offset,
        links: 1,
        sha256: hex::encode(digest.finalize()),
    })
}

pub(super) fn require_zero_owned_wal(
    sidecars: &OwnerCreatedSqliteSidecars,
) -> Result<(), GlobalSchemaV1Error> {
    let metadata = sidecars
        .wal
        .file
        .metadata()
        .map_err(|_| refusal("prospective owned WAL metadata unavailable"))?;
    if metadata.len() != 0 || metadata.nlink() != 1 {
        return Err(refusal(
            "prospective main bytes cannot represent nonempty owned WAL",
        ));
    }
    Ok(())
}

impl Pending {
    pub(super) fn capture(
        snapshot: &VerifiedSelectionSchemaSnapshot<'_, '_>,
        references: &SameRuntimeCatalogReferences,
        half: &DatabaseHalfDiagnostic,
        exact_amended: bool,
        mode: BoundMode,
        options: &Options,
    ) -> Result<Self, GlobalSchemaV1Error> {
        if matches!(half, DatabaseHalfDiagnostic::AmendedDatabaseHalf(e) if e.identity.user_version == 1)
            && !exact_amended
        {
            return Err(refusal(
                "prospective generation1 final source lacks actual reconciled authority",
            ));
        }
        require_zero_owned_wal(snapshot.inspection_sidecars)?;
        let target = prospective_target_reference(&snapshot.initial_catalog, references)
            .map_err(|source| GlobalSchemaV1Error::SelectionCatalog { source })?;
        let mut hashed_bytes = 0;
        let database = fingerprint(snapshot.database_file, options, &mut hashed_bytes)?;
        let audit = match snapshot.audit_file {
            PinnedSelectionAuditFile::Missing => AuditAnchor::Absent,
            PinnedSelectionAuditFile::Present { file, .. } => AuditAnchor::Present {
                file: fingerprint(file, options, &mut hashed_bytes)?,
                record_count: snapshot.initial_audit.validation().record_count,
                tail_hash: snapshot.initial_audit.validation().tail_hash.clone(),
            },
        };
        let namespace = &snapshot.maintenance.namespace;
        let evidence = match half {
            DatabaseHalfDiagnostic::AbsentDatabaseHalf(e)
            | DatabaseHalfDiagnostic::PreAmendment(e)
            | DatabaseHalfDiagnostic::Transitional(e)
            | DatabaseHalfDiagnostic::AmendedDatabaseHalf(e) => e,
        };
        let mut relative = PathBuf::new();
        for part in &namespace.database_parent.relative_components {
            relative.push(part);
        }
        relative.push(&namespace.database_leaf);
        let review = Review {
            version: 1,
            domain: "stock_analysis.global_schema.prospective_prepare.v1",
            capability_scope: "unapproved_prospective_read_only",
            mode: mode.label(),
            observed_at: chrono::Utc::now().to_rfc3339(),
            database_relative_identity: relative
                .to_str()
                .ok_or_else(|| refusal("prospective relative identity is not UTF8"))?
                .into(),
            root: directory_anchor(&namespace.root.file)?,
            database_parent: directory_anchor(&namespace.database_parent.file)?,
            maintenance_parent: directory_anchor(&namespace.lock_parent.file)?,
            maintenance_device: snapshot.maintenance.lock_identity.device,
            maintenance_inode: snapshot.maintenance.lock_identity.inode,
            audit_parent: directory_anchor(&snapshot.audit_parent.file)?,
            database,
            audit,
            original_application_id: evidence.identity.application_id,
            original_user_version: evidence.identity.user_version,
            selection_counts_diagnostic_only: snapshot
                .initial_catalog
                .selection_row_counts()
                .clone(),
            legacy_counts_diagnostic_only: evidence.legacy_row_counts.clone(),
            target,
            owned_wal_bytes_during_sql: 0,
            source_scope: "retained_main_file_bytes_with_zero_owned_wal_not_logical_row_snapshot",
            approval: "not_granted",
            backup: "not_created",
            exchange: "not_implemented",
            maintenance_receipt: "not_created",
            apply_supported: false,
            apply_blocker: super::super::selection_v2::SELECTION_V2_APPLY_BLOCKER,
        };
        bounded_json(&review, options.max_review_bytes)?;
        Ok(Self {
            review,
            hashed_bytes,
        })
    }

    // Low-authority bytes only: the backup journal compares this freshly
    // derived binding. Stored JSON cannot recreate this source owner.
    pub(super) fn backup_binding(&self, options: &Options) -> Result<String, GlobalSchemaV1Error> {
        #[derive(Serialize)]
        struct Binding<'a> {
            version: u16,
            domain: &'static str,
            mode: &'a str,
            database_relative_identity: &'a str,
            root: &'a DirectoryAnchor,
            database_parent: &'a DirectoryAnchor,
            maintenance_parent: &'a DirectoryAnchor,
            maintenance_device: u64,
            maintenance_inode: u64,
            audit_parent: &'a DirectoryAnchor,
            database: &'a FileAnchor,
            audit: &'a AuditAnchor,
            original_application_id: i64,
            original_user_version: i64,
            selection_counts_diagnostic_only: &'a BTreeMap<String, i64>,
            legacy_counts_diagnostic_only: &'a BTreeMap<String, i64>,
            target: &'a ProspectiveCatalogReference,
            source_scope: &'a str,
        }
        let r = &self.review;
        String::from_utf8(bounded_json(
            &Binding {
                version: 1,
                domain: "stock_analysis.global_schema.backup_source_bytes.v1",
                mode: r.mode,
                database_relative_identity: &r.database_relative_identity,
                root: &r.root,
                database_parent: &r.database_parent,
                maintenance_parent: &r.maintenance_parent,
                maintenance_device: r.maintenance_device,
                maintenance_inode: r.maintenance_inode,
                audit_parent: &r.audit_parent,
                database: &r.database,
                audit: &r.audit,
                original_application_id: r.original_application_id,
                original_user_version: r.original_user_version,
                selection_counts_diagnostic_only: &r.selection_counts_diagnostic_only,
                legacy_counts_diagnostic_only: &r.legacy_counts_diagnostic_only,
                target: &r.target,
                source_scope: r.source_scope,
            },
            options.max_review_bytes,
        )?)
        .map_err(|_| refusal("backup source binding encoding"))
    }

    pub(super) fn backup_expected(&self) -> (FileAnchor, Option<FileAnchor>) {
        (
            self.review.database.clone(),
            match &self.review.audit {
                AuditAnchor::Absent => None,
                AuditAnchor::Present { file, .. } => Some(file.clone()),
            },
        )
    }

    pub(super) fn backup_hash_work(&self) -> u64 {
        self.hashed_bytes
    }

    pub(super) fn reserve_backup_reads(
        &mut self,
        bytes: u64,
        options: &Options,
    ) -> Result<(), GlobalSchemaV1Error> {
        let next = self
            .hashed_bytes
            .checked_add(bytes)
            .ok_or_else(|| refusal("backup physical read work overflow"))?;
        if next > options.max_total_hash_bytes {
            return Err(refusal("backup shared physical read work exceeded"));
        }
        self.hashed_bytes = next;
        Ok(())
    }

    pub(super) fn backup_fingerprint(
        &mut self,
        file: &File,
        options: &Options,
    ) -> Result<FileAnchor, GlobalSchemaV1Error> {
        fingerprint(file, options, &mut self.hashed_bytes)
    }

    pub(super) fn validate_snapshot(
        &mut self,
        snapshot: &VerifiedSelectionSchemaSnapshot<'_, '_>,
        options: &Options,
    ) -> Result<(), GlobalSchemaV1Error> {
        require_zero_owned_wal(snapshot.inspection_sidecars)?;
        self.validate_files(snapshot.database_file, snapshot.audit_file, options)?;
        options.phase(Phase::AfterPhysicalReads)?;
        require_same_file_identity(
            &snapshot.maintenance.namespace.database_parent,
            &snapshot.maintenance.namespace.database_leaf,
            &snapshot.database_path,
            snapshot.database_file,
            snapshot.database_identity,
            "prospective final database",
        )?;
        revalidate_selection_audit_file(
            snapshot.audit_parent,
            &snapshot.audit_leaf,
            &snapshot.audit_path,
            snapshot.audit_file,
        )?;
        snapshot
            .inspection_sidecars
            .validate_present_exact(&snapshot.maintenance.namespace, &snapshot.database_path)?;
        require_zero_owned_wal(snapshot.inspection_sidecars)?;
        snapshot.maintenance.namespace.validate_unchanged()?;
        require_same_file_identity(
            &snapshot.maintenance.namespace.lock_parent,
            &snapshot.maintenance.namespace.lock_leaf,
            &snapshot
                .maintenance
                .namespace
                .lock_parent
                .path
                .join(&snapshot.maintenance.namespace.lock_leaf),
            &snapshot.maintenance.lock_file,
            snapshot.maintenance.lock_identity,
            "prospective maintenance lock",
        )
    }

    fn validate_files(
        &mut self,
        database: &File,
        audit: &PinnedSelectionAuditFile,
        options: &Options,
    ) -> Result<(), GlobalSchemaV1Error> {
        if fingerprint(database, options, &mut self.hashed_bytes)? != self.review.database {
            return Err(refusal("prospective main file bytes changed"));
        }
        match (&self.review.audit, audit) {
            (AuditAnchor::Absent, PinnedSelectionAuditFile::Missing) => {}
            (
                AuditAnchor::Present { file: expected, .. },
                PinnedSelectionAuditFile::Present { file, .. },
            ) => {
                if fingerprint(file, options, &mut self.hashed_bytes)? != *expected {
                    return Err(refusal("prospective audit bytes changed"));
                }
            }
            _ => return Err(refusal("prospective audit presence changed")),
        }
        Ok(())
    }

    pub(super) fn issue(
        self,
        options: Options,
        database_file: File,
        database_identity: FileIdentity,
        audit_parent: PinnedDirectory,
        audit_file: PinnedSelectionAuditFile,
        maintenance: ExclusiveGlobalSchemaMaintenanceLease,
    ) -> Result<PreparedGlobalSchemaProspective, GlobalSchemaV1Error> {
        let mut result = PreparedGlobalSchemaProspective {
            pending: self,
            options,
            database_file,
            database_identity,
            audit_parent,
            audit_file,
            maintenance,
        };
        result.options.phase(Phase::AfterSidecarCleanup)?;
        result.validate_after_cleanup()?;
        Ok(result)
    }
}

impl PreparedGlobalSchemaProspective {
    fn validate_after_cleanup(&mut self) -> Result<(), GlobalSchemaV1Error> {
        let namespace = &self.maintenance.namespace;
        let db_path = namespace
            .database_parent
            .path
            .join(&namespace.database_leaf);
        require_no_live_sidecars_for_bound_namespace(namespace, &db_path)?;
        require_same_file_identity(
            &namespace.database_parent,
            &namespace.database_leaf,
            &db_path,
            &self.database_file,
            self.database_identity,
            "prospective database after cleanup",
        )?;
        let audit_path = match self.pending.review.mode {
            "production" => namespace
                .root
                .path
                .join("data/audit/production/selection-audit.jsonl"),
            _ => self.audit_parent.path.join("selection-audit.jsonl"),
        };
        revalidate_selection_audit_file(
            &self.audit_parent,
            OsStr::new("selection-audit.jsonl"),
            &audit_path,
            &self.audit_file,
        )?;
        self.pending
            .validate_files(&self.database_file, &self.audit_file, &self.options)?;
        self.options.phase(Phase::AfterPhysicalReads)?;
        if directory_anchor(&namespace.root.file)? != self.pending.review.root
            || directory_anchor(&namespace.database_parent.file)?
                != self.pending.review.database_parent
            || directory_anchor(&namespace.lock_parent.file)?
                != self.pending.review.maintenance_parent
            || directory_anchor(&self.audit_parent.file)? != self.pending.review.audit_parent
        {
            return Err(refusal("prospective namespace anchors changed"));
        }
        // Recheck the named leaves and sidecar absence after the physical
        // reads; a retained FD alone cannot detect a same-bytes leaf swap.
        require_same_file_identity(
            &namespace.database_parent,
            &namespace.database_leaf,
            &db_path,
            &self.database_file,
            self.database_identity,
            "prospective final named database",
        )?;
        revalidate_selection_audit_file(
            &self.audit_parent,
            OsStr::new("selection-audit.jsonl"),
            &audit_path,
            &self.audit_file,
        )?;
        require_no_live_sidecars_for_bound_namespace(namespace, &db_path)?;
        namespace.validate_unchanged()?;
        require_same_file_identity(
            &namespace.lock_parent,
            &namespace.lock_leaf,
            &namespace.lock_parent.path.join(&namespace.lock_leaf),
            &self.maintenance.lock_file,
            self.maintenance.lock_identity,
            "prospective lock after cleanup",
        )
    }

    pub(super) fn backup_validate(&mut self) -> Result<(), GlobalSchemaV1Error> {
        self.validate_after_cleanup()
    }

    pub(super) fn backup_parts(
        &mut self,
    ) -> (&mut Pending, &Options, &File, &PinnedSelectionAuditFile) {
        (
            &mut self.pending,
            &self.options,
            &self.database_file,
            &self.audit_file,
        )
    }

    pub(super) fn render_unapproved(mut self) -> Result<String, GlobalSchemaV1Error> {
        self.options.phase(Phase::BeforeRender)?;
        self.validate_after_cleanup()?;
        let canonical = bounded_json(&self.pending.review, self.options.max_review_bytes)?;
        let mut digest = Sha256::new();
        digest.update(DOMAIN);
        digest.update([0]);
        digest.update(&canonical);
        #[derive(Serialize)]
        struct Output<'a> {
            preview_sha256: String,
            review: &'a Review,
        }
        let bytes = bounded_json(
            &Output {
                preview_sha256: hex::encode(digest.finalize()),
                review: &self.pending.review,
            },
            self.options.max_review_bytes,
        )?;
        String::from_utf8(bytes).map_err(|_| refusal("prospective review encoding failed"))
    }
}

pub(super) fn bounded_json(
    value: &impl Serialize,
    max: usize,
) -> Result<Vec<u8>, GlobalSchemaV1Error> {
    struct Writer {
        bytes: Vec<u8>,
        max: usize,
    }
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let length = self
                .bytes
                .len()
                .checked_add(bytes.len())
                .ok_or_else(|| io::Error::other("prospective review budget"))?;
            if length > self.max {
                return Err(io::Error::other("prospective review budget"));
            }
            self.bytes
                .try_reserve_exact(bytes.len())
                .map_err(|_| io::Error::other("prospective review allocation"))?;
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer {
        bytes: Vec::new(),
        max,
    };
    serde_json::to_writer(&mut writer, value)
        .map_err(|_| refusal("prospective review budget or encoding failed"))?;
    writer
        .write_all(b"\n")
        .map_err(|_| refusal("prospective review budget exceeded"))?;
    Ok(writer.bytes)
}
