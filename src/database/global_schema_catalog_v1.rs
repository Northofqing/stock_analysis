//! BR-180/BR-186 whole-application generation-1 catalog evidence.
//!
//! This module is deliberately database-half only. It owns no maintenance
//! lease, migration transaction, PRAGMA writer, audit session, backup,
//! exchange, recovery, or startup capability. Its typed diagnostic can inform
//! the sole global schema owner, but can never authorize migration or ordinary
//! startup by itself.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use rusqlite::{params, Connection, OptionalExtension};

use super::global_schema_v1::SelectionCatalogCaptureAuthority;
use super::selection_v2::{
    selection_v2_catalog_ddl_plan, SelectionV2CatalogDdlPhase, SelectionV2CatalogObjectKind,
    SelectionV2StoreMode,
};

const LEGACY_CATALOG_V1_FIXTURE: &str =
    include_str!("fixtures/global_schema_legacy_catalog_v1.tsv");
const LEGACY_DDL_V1_FIXTURE: &str = include_str!("fixtures/global_schema_legacy_ddl_v1.tsv");
const LEGACY_DDL_V1_FIXTURE_SHA256: &str =
    "938c1bc039e0b2443ab0f2e3a8c5bebe218b2c6fe928f6ad2c15d3019fde9922";
const FINAL_SELECTION_TABLES: [&str; 12] = [
    "selection_source_batch_attempts",
    "selection_source_facts_v2",
    "selection_source_fact_attempts",
    "selection_relation_attempts",
    "selection_evaluation_attempts",
    "selection_samples",
    "selection_rejections",
    "selection_sample_outcomes",
    "selection_outcome_attempts",
    "selection_v2_recovery_envelopes",
    "selection_v2_run_stages",
    "selection_v2_commit_receipts",
];
const FINAL_SELECTION_INDEXES: [(&str, &str); 5] = [
    (
        "selection_v2_one_activation_per_config",
        "selection_v2_run_stages",
    ),
    (
        "selection_v2_source_facts_pending",
        "selection_source_facts_v2",
    ),
    ("selection_v2_samples_generation", "selection_samples"),
    (
        "selection_v2_outcome_attempt_run",
        "selection_outcome_attempts",
    ),
    (
        "selection_v2_receipt_subject",
        "selection_v2_commit_receipts",
    ),
];
const FINAL_SELECTION_STATIC_TRIGGERS: [(&str, &str); 17] = [
    (
        "selection_v2_batch_lineage",
        "selection_source_batch_attempts",
    ),
    ("selection_v2_fact_lineage", "selection_source_facts_v2"),
    (
        "selection_v2_fact_attempt_lineage",
        "selection_source_fact_attempts",
    ),
    (
        "selection_v2_relation_requires_admitted_source",
        "selection_relation_attempts",
    ),
    (
        "selection_v2_evaluation_requires_admitted_source",
        "selection_evaluation_attempts",
    ),
    (
        "selection_v2_sample_requires_admitted_source",
        "selection_samples",
    ),
    (
        "selection_v2_rejection_requires_admitted_source",
        "selection_rejections",
    ),
    (
        "selection_v2_manifest_envelope_binding",
        "selection_v2_run_stages",
    ),
    (
        "selection_v2_config_manifest_closure",
        "selection_v2_run_stages",
    ),
    (
        "selection_v2_ingress_manifest_closure",
        "selection_v2_run_stages",
    ),
    (
        "selection_v2_generation_manifest_closure",
        "selection_v2_run_stages",
    ),
    (
        "selection_v2_outcome_manifest_closure",
        "selection_v2_run_stages",
    ),
    (
        "selection_v2_receipt_manifest_binding",
        "selection_v2_commit_receipts",
    ),
    (
        "selection_v2_config_receipt_closure",
        "selection_v2_commit_receipts",
    ),
    (
        "selection_v2_ingress_receipt_closure",
        "selection_v2_commit_receipts",
    ),
    (
        "selection_v2_generation_receipt_closure",
        "selection_v2_commit_receipts",
    ),
    (
        "selection_v2_outcome_receipt_closure",
        "selection_v2_commit_receipts",
    ),
];
const FINAL_SELECTION_STAGE_TABLES: [&str; 9] = [
    "selection_source_batch_attempts",
    "selection_source_facts_v2",
    "selection_source_fact_attempts",
    "selection_relation_attempts",
    "selection_evaluation_attempts",
    "selection_samples",
    "selection_rejections",
    "selection_sample_outcomes",
    "selection_outcome_attempts",
];
const FINAL_SELECTION_SYMBOL_TABLES: [(&str, &str); 3] = [
    ("relation", "selection_relation_attempts"),
    ("evaluation", "selection_evaluation_attempts"),
    ("sample", "selection_samples"),
];

pub(crate) const FINAL_SELECTION_PAYLOAD_SCHEMAS: [&str; 5] = [
    "config-activation-stage-v1",
    "source-ingress-stage-v2",
    "generation-stage-v3",
    "outcome-claim-stage-v2",
    "outcome-stage-v3",
];
pub(crate) const TRANSITIONAL_SELECTION_PAYLOAD_SCHEMAS: [&str; 4] = [
    "config-activation-stage-v1",
    "source-ingress-stage-v2",
    "generation-stage-v3",
    "outcome-stage-v2",
];
const STOCK_ANALYSIS_SQLITE_APPLICATION_ID: i64 = 1_398_035_265;
const STOCK_ANALYSIS_DB_SCHEMA_GENERATION: i64 = 1;
const SQLITE_MINIMUM_LIBVERSION_NUMBER: i32 = 3_035_000;
const SQLITE_NEXT_MAJOR_LIBVERSION_NUMBER: i32 = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlobalSchemaCatalogMode {
    Production,
    #[cfg(test)]
    Test,
}

impl GlobalSchemaCatalogMode {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Production => "production",
            #[cfg(test)]
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CatalogObjectKind {
    Table,
    Index,
    Trigger,
    View,
}

impl CatalogObjectKind {
    const fn ordinal(self) -> u8 {
        match self {
            Self::Table => 0,
            Self::Index => 1,
            Self::Trigger => 2,
            Self::View => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CatalogObjectIdentity {
    pub(crate) kind: CatalogObjectKind,
    pub(crate) name: String,
    pub(crate) table_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrozenCatalogRegistryEntry {
    pub(crate) identity: CatalogObjectIdentity,
    pub(crate) ddl_id: String,
    pub(crate) source_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CheckedInDdlStatement {
    ddl_id: String,
    exact_sql: String,
    source_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CatalogObjectRow {
    pub(crate) identity: CatalogObjectIdentity,
    pub(crate) exact_sql: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GlobalSchemaCatalogError {
    InvalidFrozenRegistry {
        line: usize,
        detail: String,
    },
    FrozenRegistryCountMismatch {
        tables: usize,
        indexes: usize,
        triggers: usize,
    },
    FrozenDdlFixtureDigestMismatch {
        expected: &'static str,
        actual: String,
    },
    InvalidFrozenDdlFixture {
        line: usize,
        detail: String,
    },
    FrozenDdlRegistryMismatch {
        missing: Vec<String>,
        extra: Vec<String>,
    },
    LegacyRowRegistryMismatch {
        side: &'static str,
        missing: Vec<String>,
        extra: Vec<String>,
    },
    InvalidLegacyRowCount {
        side: &'static str,
        table: String,
        count: i64,
    },
    #[cfg(test)]
    LegacyRowCountChanged {
        table: String,
        source: i64,
        candidate: i64,
    },
    GeneratedRegistryMismatch {
        catalog: &'static str,
        detail: String,
    },
    ModeMismatch {
        expected: GlobalSchemaCatalogMode,
        actual: GlobalSchemaCatalogMode,
    },
    InvalidRuntimeIdentity {
        detail: String,
    },
    UnsupportedSqliteRuntime {
        actual: i32,
    },
    RuntimeIdentityMismatch,
    InvalidCatalogReference {
        catalog: &'static str,
        detail: String,
    },
    ReferenceDdlRegistryMismatch {
        catalog: &'static str,
        missing: Vec<String>,
        extra: Vec<String>,
    },
    SqliteReferenceBuildFailure {
        stage: &'static str,
        ddl_id: Option<String>,
        detail: String,
    },
    AttachedSchemaMismatch {
        actual: Vec<String>,
    },
    ManagedIndexGeometryMismatch {
        detail: String,
    },
    ExternalForeignKeyToManagedTable {
        source_table: String,
        target_table: String,
    },
    ExternalObjectTargetsManagedTable {
        object: String,
        target_table: String,
    },
    SqlScanFailure {
        object: String,
        detail: String,
    },
    SqliteOwnedCatalogMismatch {
        detail: String,
    },
    UnsupportedFutureGeneration {
        actual: i64,
        supported: i64,
    },
    UnsupportedSchemaIdentity {
        application_id: i64,
        user_version: i64,
    },
    CatalogMismatch {
        detail: String,
    },
    PayloadSchemaMismatch {
        expected: Vec<String>,
        actual: Vec<String>,
    },
}

impl fmt::Display for GlobalSchemaCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFrozenRegistry { line, detail } => {
                write!(formatter, "invalid frozen global catalog at line {line}: {detail}")
            }
            Self::FrozenRegistryCountMismatch {
                tables,
                indexes,
                triggers,
            } => write!(
                formatter,
                "frozen global catalog count mismatch: tables={tables},indexes={indexes},triggers={triggers}"
            ),
            Self::FrozenDdlFixtureDigestMismatch { expected, actual } => write!(
                formatter,
                "frozen legacy DDL fixture digest mismatch: expected={expected},actual={actual}"
            ),
            Self::InvalidFrozenDdlFixture { line, detail } => {
                write!(formatter, "invalid frozen legacy DDL at line {line}: {detail}")
            }
            Self::FrozenDdlRegistryMismatch { missing, extra } => write!(
                formatter,
                "frozen legacy DDL registry mismatch: missing={missing:?},extra={extra:?}"
            ),
            Self::LegacyRowRegistryMismatch {
                side,
                missing,
                extra,
            } => write!(
                formatter,
                "{side} legacy row registry mismatch: missing={missing:?},extra={extra:?}"
            ),
            Self::InvalidLegacyRowCount { side, table, count } => write!(
                formatter,
                "{side} legacy row count is invalid: table={table},count={count}"
            ),
            #[cfg(test)]
            Self::LegacyRowCountChanged {
                table,
                source,
                candidate,
            } => write!(
                formatter,
                "legacy row count changed: table={table},source={source},candidate={candidate}"
            ),
            Self::GeneratedRegistryMismatch { catalog, detail } => {
                write!(formatter, "generated {catalog} registry mismatch: {detail}")
            }
            Self::ModeMismatch { expected, actual } => write!(
                formatter,
                "catalog mode mismatch: expected={},actual={}",
                expected.label(),
                actual.label()
            ),
            Self::InvalidRuntimeIdentity { detail } => {
                write!(formatter, "invalid SQLite runtime identity: {detail}")
            }
            Self::UnsupportedSqliteRuntime { actual } => write!(
                formatter,
                "unsupported SQLite runtime {actual}; expected >=3.35.0,<4.0.0"
            ),
            Self::RuntimeIdentityMismatch => {
                write!(formatter, "SQLite runtime identity differs from same-runtime reference")
            }
            Self::InvalidCatalogReference { catalog, detail } => {
                write!(formatter, "invalid {catalog} same-runtime reference: {detail}")
            }
            Self::ReferenceDdlRegistryMismatch {
                catalog,
                missing,
                extra,
            } => write!(
                formatter,
                "{catalog} checked-in DDL registry mismatch: missing={missing:?},extra={extra:?}"
            ),
            Self::SqliteReferenceBuildFailure {
                stage,
                ddl_id,
                detail,
            } => write!(
                formatter,
                "same-runtime SQLite reference build failed: stage={stage},ddl_id={ddl_id:?},detail={detail}"
            ),
            Self::AttachedSchemaMismatch { actual } => write!(
                formatter,
                "catalog snapshot must contain main and may contain only SQLite's built-in temp schema: actual={actual:?}"
            ),
            Self::ManagedIndexGeometryMismatch { detail } => {
                write!(formatter, "selection-managed index geometry mismatch: {detail}")
            }
            Self::ExternalForeignKeyToManagedTable {
                source_table,
                target_table,
            } => write!(
                formatter,
                "external foreign key targets selection-managed table: source={source_table},target={target_table}"
            ),
            Self::ExternalObjectTargetsManagedTable {
                object,
                target_table,
            } => write!(
                formatter,
                "non-catalog object targets selection-managed table: object={object},target={target_table}"
            ),
            Self::SqlScanFailure { object, detail } => {
                write!(formatter, "strict SQLite SQL scan failed for {object}: {detail}")
            }
            Self::SqliteOwnedCatalogMismatch { detail } => {
                write!(formatter, "SQLite-owned catalog mismatch: {detail}")
            }
            Self::UnsupportedFutureGeneration { actual, supported } => write!(
                formatter,
                "unsupported future global schema generation {actual}; supported={supported}"
            ),
            Self::UnsupportedSchemaIdentity {
                application_id,
                user_version,
            } => write!(
                formatter,
                "unsupported database-half identity application_id={application_id},user_version={user_version}"
            ),
            Self::CatalogMismatch { detail } => {
                write!(formatter, "whole-application catalog mismatch: {detail}")
            }
            Self::PayloadSchemaMismatch { expected, actual } => write!(
                formatter,
                "selection payload schema mismatch: expected={expected:?},actual={actual:?}"
            ),
        }
    }
}

impl Error for GlobalSchemaCatalogError {}

pub(crate) fn legacy_catalog_registry_v1(
) -> Result<Vec<CatalogObjectIdentity>, GlobalSchemaCatalogError> {
    Ok(legacy_catalog_registry_entries_v1()?
        .into_iter()
        .map(|entry| entry.identity)
        .collect())
}

pub(crate) fn legacy_catalog_registry_entries_v1(
) -> Result<Vec<FrozenCatalogRegistryEntry>, GlobalSchemaCatalogError> {
    let mut rows = Vec::new();
    let mut identities = BTreeSet::new();
    let mut ddl_ids = BTreeSet::new();
    let mut table_names = BTreeSet::new();
    let mut previous: Option<CatalogObjectIdentity> = None;
    let mut counts = [0_usize; 3];

    for (offset, raw_line) in LEGACY_CATALOG_V1_FIXTURE.lines().enumerate() {
        let line_number = offset + 1;
        if raw_line.starts_with('#') {
            continue;
        }
        let fields = raw_line.split('|').collect::<Vec<_>>();
        if fields.len() != 4 || fields.iter().any(|value| value.is_empty()) {
            return Err(GlobalSchemaCatalogError::InvalidFrozenRegistry {
                line: line_number,
                detail: "expected exactly four nonempty pipe-delimited fields".to_owned(),
            });
        }
        let kind = match fields[0] {
            "table" => CatalogObjectKind::Table,
            "index" => CatalogObjectKind::Index,
            "trigger" => CatalogObjectKind::Trigger,
            other => {
                return Err(GlobalSchemaCatalogError::InvalidFrozenRegistry {
                    line: line_number,
                    detail: format!("unsupported object kind {other:?}"),
                });
            }
        };
        let identity = CatalogObjectIdentity {
            kind,
            name: fields[1].to_owned(),
            table_name: fields[2].to_owned(),
        };
        let ddl_id = fields[3].to_owned();
        if !ddl_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-_".contains(&byte)
        }) {
            return Err(GlobalSchemaCatalogError::InvalidFrozenRegistry {
                line: line_number,
                detail: format!("ddl_id contains unsupported bytes: {ddl_id:?}"),
            });
        }
        if kind == CatalogObjectKind::Table && identity.name != identity.table_name {
            return Err(GlobalSchemaCatalogError::InvalidFrozenRegistry {
                line: line_number,
                detail: "table name and tbl_name must match".to_owned(),
            });
        }
        if let Some(previous) = previous.as_ref() {
            if identity <= *previous {
                return Err(GlobalSchemaCatalogError::InvalidFrozenRegistry {
                    line: line_number,
                    detail: "entries must be strictly ordered by kind and name".to_owned(),
                });
            }
        }
        if !identities.insert(identity.clone()) {
            return Err(GlobalSchemaCatalogError::InvalidFrozenRegistry {
                line: line_number,
                detail: "duplicate object identity".to_owned(),
            });
        }
        if !ddl_ids.insert(ddl_id.clone()) {
            return Err(GlobalSchemaCatalogError::InvalidFrozenRegistry {
                line: line_number,
                detail: format!("duplicate ddl_id {ddl_id:?}"),
            });
        }
        if kind == CatalogObjectKind::Table {
            table_names.insert(identity.name.clone());
        }
        counts[match kind {
            CatalogObjectKind::Table => 0,
            CatalogObjectKind::Index => 1,
            CatalogObjectKind::Trigger => 2,
            CatalogObjectKind::View => unreachable!("legacy fixture parser rejects views"),
        }] += 1;
        previous = Some(identity.clone());
        rows.push(FrozenCatalogRegistryEntry {
            identity,
            ddl_id,
            source_line: line_number,
        });
    }

    if counts != [53, 44, 63] {
        return Err(GlobalSchemaCatalogError::FrozenRegistryCountMismatch {
            tables: counts[0],
            indexes: counts[1],
            triggers: counts[2],
        });
    }
    for row in &rows {
        if row.identity.kind != CatalogObjectKind::Table
            && !table_names.contains(&row.identity.table_name)
        {
            return Err(GlobalSchemaCatalogError::InvalidFrozenRegistry {
                line: row.source_line,
                detail: format!(
                    "{} {:?} references unregistered table {:?}",
                    match row.identity.kind {
                        CatalogObjectKind::Table => "table",
                        CatalogObjectKind::Index => "index",
                        CatalogObjectKind::Trigger => "trigger",
                        CatalogObjectKind::View => "view",
                    },
                    row.identity.name,
                    row.identity.table_name
                ),
            });
        }
    }
    Ok(rows)
}

fn legacy_catalog_ddl_entries_v1() -> Result<Vec<CheckedInDdlStatement>, GlobalSchemaCatalogError> {
    let actual_fixture_sha256 = lower_hex(&Sha256::digest(LEGACY_DDL_V1_FIXTURE.as_bytes()));
    if actual_fixture_sha256 != LEGACY_DDL_V1_FIXTURE_SHA256 {
        return Err(GlobalSchemaCatalogError::FrozenDdlFixtureDigestMismatch {
            expected: LEGACY_DDL_V1_FIXTURE_SHA256,
            actual: actual_fixture_sha256,
        });
    }
    let registry = legacy_catalog_registry_entries_v1()?;
    parse_legacy_ddl_fixture_rows(LEGACY_DDL_V1_FIXTURE, &registry)
}

fn parse_legacy_ddl_fixture_rows(
    fixture: &str,
    expected_registry: &[FrozenCatalogRegistryEntry],
) -> Result<Vec<CheckedInDdlStatement>, GlobalSchemaCatalogError> {
    let expected_ids = expected_registry
        .iter()
        .map(|entry| entry.ddl_id.clone())
        .collect::<BTreeSet<_>>();
    let mut actual_ids = BTreeSet::new();
    let mut rows = Vec::with_capacity(expected_registry.len());

    for (offset, raw_line) in fixture.lines().enumerate() {
        let line_number = offset + 1;
        if raw_line.starts_with('#') {
            continue;
        }
        let fields = raw_line.split('|').collect::<Vec<_>>();
        if fields.len() != 2 || fields.iter().any(|field| field.is_empty()) {
            return Err(GlobalSchemaCatalogError::InvalidFrozenDdlFixture {
                line: line_number,
                detail: "expected exactly two nonempty pipe-delimited fields".to_owned(),
            });
        }
        let ddl_id = fields[0].to_owned();
        if !actual_ids.insert(ddl_id.clone()) {
            return Err(GlobalSchemaCatalogError::InvalidFrozenDdlFixture {
                line: line_number,
                detail: format!("duplicate ddl_id {ddl_id:?}"),
            });
        }
        let encoded = fields[1];
        if encoded.len() % 2 != 0
            || !encoded
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'A'..=b'F'))
        {
            return Err(GlobalSchemaCatalogError::InvalidFrozenDdlFixture {
                line: line_number,
                detail: "SQL hex must be nonempty, even-length uppercase hexadecimal".to_owned(),
            });
        }
        let decoded = hex::decode(encoded).map_err(|error| {
            GlobalSchemaCatalogError::InvalidFrozenDdlFixture {
                line: line_number,
                detail: format!("SQL hex decode failed: {error}"),
            }
        })?;
        let exact_sql = String::from_utf8(decoded).map_err(|error| {
            GlobalSchemaCatalogError::InvalidFrozenDdlFixture {
                line: line_number,
                detail: format!("decoded SQL is not UTF-8: {error}"),
            }
        })?;
        if !exact_sql.starts_with("CREATE ") || exact_sql.contains('\0') {
            return Err(GlobalSchemaCatalogError::InvalidFrozenDdlFixture {
                line: line_number,
                detail: "decoded SQL must be one nonempty CREATE statement without NUL".to_owned(),
            });
        }
        rows.push(CheckedInDdlStatement {
            ddl_id,
            exact_sql,
            source_line: line_number,
        });
    }

    if actual_ids != expected_ids {
        return Err(GlobalSchemaCatalogError::FrozenDdlRegistryMismatch {
            missing: expected_ids.difference(&actual_ids).cloned().collect(),
            extra: actual_ids.difference(&expected_ids).cloned().collect(),
        });
    }
    for (row, expected) in rows.iter().zip(expected_registry) {
        if row.ddl_id != expected.ddl_id {
            return Err(GlobalSchemaCatalogError::InvalidFrozenDdlFixture {
                line: row.source_line,
                detail: format!(
                    "ddl_id order mismatch: expected={:?},actual={:?}",
                    expected.ddl_id, row.ddl_id
                ),
            });
        }
    }
    Ok(rows)
}

pub(crate) fn final_selection_catalog_registry_v1(
    mode: GlobalSchemaCatalogMode,
) -> Result<Vec<CatalogObjectIdentity>, GlobalSchemaCatalogError> {
    Ok(
        selection_catalog_registry_entries_v1(mode, SelectionCatalogDdlPhase::Final)?
            .into_iter()
            .map(|entry| entry.identity)
            .collect(),
    )
}

#[derive(Debug, Clone, Copy)]
enum SelectionCatalogDdlPhase {
    Transitional,
    Final,
}

impl SelectionCatalogDdlPhase {
    const fn label(self) -> &'static str {
        match self {
            Self::Transitional => "transitional",
            Self::Final => "final",
        }
    }
}

fn selection_catalog_registry_entries_v1(
    mode: GlobalSchemaCatalogMode,
    phase: SelectionCatalogDdlPhase,
) -> Result<Vec<FrozenCatalogRegistryEntry>, GlobalSchemaCatalogError> {
    let mut rows = Vec::with_capacity(70);
    rows.extend(
        FINAL_SELECTION_TABLES
            .into_iter()
            .map(|table| CatalogObjectIdentity {
                kind: CatalogObjectKind::Table,
                name: table.to_owned(),
                table_name: table.to_owned(),
            }),
    );
    rows.extend(
        FINAL_SELECTION_INDEXES
            .into_iter()
            .map(|(name, table_name)| CatalogObjectIdentity {
                kind: CatalogObjectKind::Index,
                name: name.to_owned(),
                table_name: table_name.to_owned(),
            }),
    );
    rows.extend(
        FINAL_SELECTION_STATIC_TRIGGERS
            .into_iter()
            .map(|(name, table_name)| CatalogObjectIdentity {
                kind: CatalogObjectKind::Trigger,
                name: name.to_owned(),
                table_name: table_name.to_owned(),
            }),
    );
    rows.extend(
        FINAL_SELECTION_STAGE_TABLES
            .into_iter()
            .map(|table_name| CatalogObjectIdentity {
                kind: CatalogObjectKind::Trigger,
                name: format!("selection_v2_{table_name}_stage_membership"),
                table_name: table_name.to_owned(),
            }),
    );
    for table_name in FINAL_SELECTION_TABLES {
        rows.push(CatalogObjectIdentity {
            kind: CatalogObjectKind::Trigger,
            name: format!("{table_name}_deny_update"),
            table_name: table_name.to_owned(),
        });
        rows.push(CatalogObjectIdentity {
            kind: CatalogObjectKind::Trigger,
            name: format!("{table_name}_deny_delete"),
            table_name: table_name.to_owned(),
        });
    }
    for (short_name, table_name) in FINAL_SELECTION_SYMBOL_TABLES {
        rows.push(CatalogObjectIdentity {
            kind: CatalogObjectKind::Trigger,
            name: format!(
                "selection_v2_{short_name}_symbol_isolation_{}",
                mode.label()
            ),
            table_name: table_name.to_owned(),
        });
    }
    rows.sort();

    let unique = rows.iter().cloned().collect::<BTreeSet<_>>();
    let counts = [
        rows.iter()
            .filter(|row| row.kind == CatalogObjectKind::Table)
            .count(),
        rows.iter()
            .filter(|row| row.kind == CatalogObjectKind::Index)
            .count(),
        rows.iter()
            .filter(|row| row.kind == CatalogObjectKind::Trigger)
            .count(),
    ];
    if rows.len() != 70 || unique.len() != 70 || counts != [12, 5, 53] {
        return Err(GlobalSchemaCatalogError::GeneratedRegistryMismatch {
            catalog: "final selection",
            detail: format!(
                "rows={},unique={},tables={},indexes={},triggers={}",
                rows.len(),
                unique.len(),
                counts[0],
                counts[1],
                counts[2]
            ),
        });
    }
    Ok(rows
        .into_iter()
        .enumerate()
        .map(|(offset, identity)| FrozenCatalogRegistryEntry {
            ddl_id: format!(
                "selection-v2-{}-{}.{}.{:03}.{}",
                phase.label(),
                mode.label(),
                match identity.kind {
                    CatalogObjectKind::Table => "table",
                    CatalogObjectKind::Index => "index",
                    CatalogObjectKind::Trigger => "trigger",
                    CatalogObjectKind::View => unreachable!("selection registry has no views"),
                },
                offset + 1,
                identity.name
            ),
            identity,
            // Generated registry entries are checked-in Rust constants rather
            // than fixture lines. Zero explicitly means "not line-backed".
            source_line: 0,
        })
        .collect())
}

#[cfg(test)]
pub(crate) fn verify_legacy_row_preservation(
    source: &BTreeMap<String, i64>,
    candidate: &BTreeMap<String, i64>,
) -> Result<(), GlobalSchemaCatalogError> {
    let expected = legacy_catalog_registry_v1()?
        .into_iter()
        .filter(|row| row.kind == CatalogObjectKind::Table)
        .map(|row| row.name)
        .collect::<BTreeSet<_>>();
    validate_row_count_registry("source", source, &expected)?;
    validate_row_count_registry("candidate", candidate, &expected)?;

    for table in expected {
        let source_count = source[&table];
        let candidate_count = candidate[&table];
        if source_count != candidate_count {
            return Err(GlobalSchemaCatalogError::LegacyRowCountChanged {
                table,
                source: source_count,
                candidate: candidate_count,
            });
        }
    }
    Ok(())
}

fn validate_row_count_registry(
    side: &'static str,
    counts: &BTreeMap<String, i64>,
    expected: &BTreeSet<String>,
) -> Result<(), GlobalSchemaCatalogError> {
    let actual = counts.keys().cloned().collect::<BTreeSet<_>>();
    if actual != *expected {
        return Err(GlobalSchemaCatalogError::LegacyRowRegistryMismatch {
            side,
            missing: expected.difference(&actual).cloned().collect(),
            extra: actual.difference(expected).cloned().collect(),
        });
    }
    for (table, count) in counts {
        if *count < 0 {
            return Err(GlobalSchemaCatalogError::InvalidLegacyRowCount {
                side,
                table: table.clone(),
                count: *count,
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SqliteRuntimeIdentity {
    pub(crate) libversion_number: i32,
    pub(crate) source_id: String,
    pub(crate) compile_options_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DatabaseSchemaIdentity {
    pub(crate) application_id: i64,
    pub(crate) user_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct IndexXinfoTerm {
    pub(crate) seqno: i64,
    pub(crate) cid: i64,
    pub(crate) name: Option<String>,
    pub(crate) descending: bool,
    pub(crate) collation: Option<String>,
    pub(crate) key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ManagedIndexGeometry {
    pub(crate) table_name: String,
    pub(crate) index_name: String,
    pub(crate) unique: bool,
    pub(crate) origin: String,
    pub(crate) partial: bool,
    pub(crate) terms: Vec<IndexXinfoTerm>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ForeignKeyDependency {
    pub(crate) source_table: String,
    pub(crate) id: i64,
    pub(crate) sequence: i64,
    pub(crate) target_table: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SqliteOwnedCatalogObject {
    pub(crate) kind: CatalogObjectKind,
    pub(crate) name: String,
    pub(crate) table_name: String,
    pub(crate) exact_sql: Option<String>,
}

#[derive(Debug)]
struct CatalogReferenceState {
    objects: Vec<CatalogObjectRow>,
    managed_index_geometry: Vec<ManagedIndexGeometry>,
    foreign_keys: Vec<ForeignKeyDependency>,
    sqlite_owned_objects: Vec<SqliteOwnedCatalogObject>,
}

/// Non-forgeable same-linked-runtime reference issued only inside this module
/// after the private global owner has executed checked-in DDL in an isolated
/// in-memory SQLite database.
///
/// Fields are intentionally private and the type is not `Clone`: callers may
/// compare an actual capture against it, but cannot inject or mutate the
/// expected SQLite-emitted SQL bytes.
#[derive(Debug)]
pub(crate) struct SameRuntimeCatalogReferences {
    mode: GlobalSchemaCatalogMode,
    runtime: SqliteRuntimeIdentity,
    legacy: CatalogReferenceState,
    transitional: CatalogReferenceState,
    amended: CatalogReferenceState,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CatalogSnapshot {
    mode: GlobalSchemaCatalogMode,
    identity: DatabaseSchemaIdentity,
    runtime: SqliteRuntimeIdentity,
    objects: Vec<CatalogObjectRow>,
    managed_index_geometry: Vec<ManagedIndexGeometry>,
    foreign_keys: Vec<ForeignKeyDependency>,
    sqlite_owned_objects: Vec<SqliteOwnedCatalogObject>,
    attached_schema_names: Vec<String>,
    legacy_row_counts: BTreeMap<String, i64>,
    selection_row_counts: BTreeMap<String, i64>,
    selection_payload_schemas: Vec<String>,
}

impl CatalogSnapshot {
    pub(crate) fn selection_row_counts(&self) -> &BTreeMap<String, i64> {
        &self.selection_row_counts
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WholeApplicationSchemaCatalogSha256(String);

impl WholeApplicationSchemaCatalogSha256 {
    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectionManagedSchemaCatalogSha256(String);

impl SelectionManagedSchemaCatalogSha256 {
    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DatabaseHalfEvidence {
    pub(crate) mode: GlobalSchemaCatalogMode,
    pub(crate) identity: DatabaseSchemaIdentity,
    pub(crate) runtime: SqliteRuntimeIdentity,
    pub(crate) whole_application_catalog_sha256: WholeApplicationSchemaCatalogSha256,
    pub(crate) selection_managed_catalog_sha256: Option<SelectionManagedSchemaCatalogSha256>,
    pub(crate) legacy_row_counts: BTreeMap<String, i64>,
    pub(crate) selection_payload_schemas: Vec<String>,
}

/// Diagnostic-only classification of the database half.
///
/// None of these variants is an audit-backed capability. In particular,
/// `AmendedDatabaseHalf` is not authoritative `Amended`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DatabaseHalfDiagnostic {
    AbsentDatabaseHalf(DatabaseHalfEvidence),
    PreAmendment(DatabaseHalfEvidence),
    Transitional(DatabaseHalfEvidence),
    AmendedDatabaseHalf(DatabaseHalfEvidence),
}

pub(crate) fn classify_database_half(
    actual: &CatalogSnapshot,
    references: &SameRuntimeCatalogReferences,
) -> Result<DatabaseHalfDiagnostic, GlobalSchemaCatalogError> {
    if actual.mode != references.mode {
        return Err(GlobalSchemaCatalogError::ModeMismatch {
            expected: references.mode,
            actual: actual.mode,
        });
    }
    validate_runtime_identity(&references.runtime)?;
    validate_runtime_identity(&actual.runtime)?;
    if actual.runtime != references.runtime {
        return Err(GlobalSchemaCatalogError::RuntimeIdentityMismatch);
    }
    validate_catalog_rows("actual", &actual.objects)?;
    validate_attached_schema_names(&actual.attached_schema_names)?;
    validate_row_count_registry(
        "actual",
        &actual.legacy_row_counts,
        &legacy_table_name_set()?,
    )?;
    validate_row_count_registry(
        "actual-selection",
        &actual.selection_row_counts,
        &FINAL_SELECTION_TABLES
            .iter()
            .map(|table| (*table).to_owned())
            .collect(),
    )?;
    if actual.identity.application_id == STOCK_ANALYSIS_SQLITE_APPLICATION_ID
        && actual.identity.user_version > STOCK_ANALYSIS_DB_SCHEMA_GENERATION
    {
        return Err(GlobalSchemaCatalogError::UnsupportedFutureGeneration {
            actual: actual.identity.user_version,
            supported: STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
        });
    }
    validate_catalog_safety(
        "actual",
        actual.mode,
        &actual.objects,
        &actual.managed_index_geometry,
        &actual.foreign_keys,
        &actual.sqlite_owned_objects,
    )?;

    if actual.objects.is_empty() {
        let empty_identity = DatabaseSchemaIdentity {
            application_id: 0,
            user_version: 0,
        };
        if actual.identity != empty_identity
            || !actual.managed_index_geometry.is_empty()
            || !actual.foreign_keys.is_empty()
            || !actual.sqlite_owned_objects.is_empty()
            || actual.legacy_row_counts.values().any(|count| *count != 0)
            || actual
                .selection_row_counts
                .values()
                .any(|count| *count != 0)
            || !actual.selection_payload_schemas.is_empty()
        {
            return Err(GlobalSchemaCatalogError::CatalogMismatch {
                detail: "empty application catalog carries contradictory identity, dependency, row-count, or payload evidence".to_owned(),
            });
        }
        return Ok(DatabaseHalfDiagnostic::AbsentDatabaseHalf(
            DatabaseHalfEvidence {
                mode: actual.mode,
                identity: actual.identity,
                runtime: actual.runtime.clone(),
                whole_application_catalog_sha256: whole_application_catalog_sha256(
                    actual.mode,
                    &actual.runtime,
                    &actual.objects,
                ),
                selection_managed_catalog_sha256: None,
                legacy_row_counts: actual.legacy_row_counts.clone(),
                selection_payload_schemas: Vec::new(),
            },
        ));
    }

    let legacy = canonical_catalog(&references.legacy.objects);
    let transitional = canonical_catalog(&references.transitional.objects);
    let final_catalog = canonical_catalog(&references.amended.objects);
    let actual_catalog = canonical_catalog(&actual.objects);
    let (expected_identity, expected_payloads, state, expected_reference) = if actual_catalog
        == legacy
    {
        (
            DatabaseSchemaIdentity {
                application_id: 0,
                user_version: 0,
            },
            Vec::new(),
            DatabaseHalfState::PreAmendment,
            &references.legacy,
        )
    } else if actual_catalog == transitional {
        (
            DatabaseSchemaIdentity {
                application_id: 0,
                user_version: 0,
            },
            TRANSITIONAL_SELECTION_PAYLOAD_SCHEMAS
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            DatabaseHalfState::Transitional,
            &references.transitional,
        )
    } else if actual_catalog == final_catalog {
        (
            DatabaseSchemaIdentity {
                application_id: STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
                user_version: STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
            },
            FINAL_SELECTION_PAYLOAD_SCHEMAS
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            DatabaseHalfState::Amended,
            &references.amended,
        )
    } else {
        return Err(GlobalSchemaCatalogError::CatalogMismatch {
            detail: format!(
                "actual named objects={} do not equal exact legacy={}, transitional={}, or final={} reference",
                actual_catalog.len(),
                legacy.len(),
                transitional.len(),
                final_catalog.len()
            ),
        });
    };
    require_exact_ancillary_catalog(actual, expected_reference)?;
    if actual.identity != expected_identity {
        return Err(GlobalSchemaCatalogError::UnsupportedSchemaIdentity {
            application_id: actual.identity.application_id,
            user_version: actual.identity.user_version,
        });
    }
    let mut actual_payloads = actual.selection_payload_schemas.clone();
    actual_payloads.sort();
    if actual_payloads.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(GlobalSchemaCatalogError::PayloadSchemaMismatch {
            expected: sorted_strings(expected_payloads),
            actual: actual_payloads,
        });
    }
    let expected_payloads = sorted_strings(expected_payloads);
    if actual_payloads != expected_payloads {
        return Err(GlobalSchemaCatalogError::PayloadSchemaMismatch {
            expected: expected_payloads,
            actual: actual_payloads,
        });
    }

    let selection_managed_catalog_sha256 = if state == DatabaseHalfState::PreAmendment {
        None
    } else {
        let selection_rows = selection_managed_rows(actual.mode, &actual.objects)?;
        Some(selection_managed_catalog_sha256(
            actual.mode,
            &actual.runtime,
            &selection_rows,
        ))
    };
    let evidence = DatabaseHalfEvidence {
        mode: actual.mode,
        identity: actual.identity,
        runtime: actual.runtime.clone(),
        whole_application_catalog_sha256: whole_application_catalog_sha256(
            actual.mode,
            &actual.runtime,
            &actual.objects,
        ),
        selection_managed_catalog_sha256,
        legacy_row_counts: actual.legacy_row_counts.clone(),
        selection_payload_schemas: actual.selection_payload_schemas.clone(),
    };
    Ok(match state {
        DatabaseHalfState::PreAmendment => DatabaseHalfDiagnostic::PreAmendment(evidence),
        DatabaseHalfState::Transitional => DatabaseHalfDiagnostic::Transitional(evidence),
        DatabaseHalfState::Amended => DatabaseHalfDiagnostic::AmendedDatabaseHalf(evidence),
    })
}

/// Capture the actual whole-application catalog and row-count evidence from
/// the exact connection retained by the global owner.
///
/// The authority argument is non-forgeable outside `global_schema_v1`.
/// `CatalogSnapshot` is intentionally non-`Clone` with private fields so this
/// value cannot be assembled or detached by application callers.
pub(super) fn capture_catalog_snapshot(
    _authority: &SelectionCatalogCaptureAuthority,
    connection: &Connection,
    mode: GlobalSchemaCatalogMode,
) -> Result<CatalogSnapshot, GlobalSchemaCatalogError> {
    let identity = DatabaseSchemaIdentity {
        application_id: connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(|error| sqlite_reference_build_error("capture-application-id", None, error))?,
        user_version: connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| sqlite_reference_build_error("capture-user-version", None, error))?,
    };
    let runtime = capture_sqlite_runtime_identity(connection)?;
    let catalog = capture_catalog_reference_state(connection, mode)?;
    let attached_schema_names = capture_attached_schema_names(connection)?;
    let legacy_row_counts = capture_legacy_row_counts(connection, &catalog.objects)?;
    let selection_row_counts = capture_selection_row_counts(connection, &catalog.objects)?;
    let selection_payload_schemas = capture_selection_payload_schema_contract(&catalog.objects);

    Ok(CatalogSnapshot {
        mode,
        identity,
        runtime,
        objects: catalog.objects,
        managed_index_geometry: catalog.managed_index_geometry,
        foreign_keys: catalog.foreign_keys,
        sqlite_owned_objects: catalog.sqlite_owned_objects,
        attached_schema_names,
        legacy_row_counts,
        selection_row_counts,
        selection_payload_schemas,
    })
}

fn capture_legacy_row_counts(
    connection: &Connection,
    objects: &[CatalogObjectRow],
) -> Result<BTreeMap<String, i64>, GlobalSchemaCatalogError> {
    let present_tables = objects
        .iter()
        .filter(|row| row.identity.kind == CatalogObjectKind::Table)
        .map(|row| row.identity.name.as_str())
        .collect::<BTreeSet<_>>();
    let legacy_tables = legacy_table_name_set()?;
    let mut counts = BTreeMap::new();
    for table in legacy_tables {
        let count = if present_tables.contains(table.as_str()) {
            capture_table_row_counts(connection, [table.as_str()], "legacy")?
                .remove(table.as_str())
                .ok_or_else(|| GlobalSchemaCatalogError::SqliteReferenceBuildFailure {
                    stage: "capture-legacy-row-count",
                    ddl_id: None,
                    detail: format!("legacy row count disappeared for {table}"),
                })?
        } else {
            0
        };
        counts.insert(table, count);
    }
    Ok(counts)
}

fn capture_selection_row_counts(
    connection: &Connection,
    objects: &[CatalogObjectRow],
) -> Result<BTreeMap<String, i64>, GlobalSchemaCatalogError> {
    let present_tables = objects
        .iter()
        .filter(|row| row.identity.kind == CatalogObjectKind::Table)
        .map(|row| row.identity.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut counts = BTreeMap::new();
    for table in FINAL_SELECTION_TABLES {
        let count = if present_tables.contains(table) {
            capture_table_row_counts(connection, [table], "selection")?
                .remove(table)
                .ok_or_else(|| GlobalSchemaCatalogError::SqliteReferenceBuildFailure {
                    stage: "capture-selection-row-count",
                    ddl_id: None,
                    detail: format!("selection row count disappeared for {table}"),
                })?
        } else {
            0
        };
        counts.insert(table.to_owned(), count);
    }
    Ok(counts)
}

fn capture_attached_schema_names(
    connection: &Connection,
) -> Result<Vec<String>, GlobalSchemaCatalogError> {
    let mut statement = connection
        .prepare("PRAGMA database_list")
        .map_err(|error| sqlite_reference_build_error("prepare-database-list", None, error))?;
    let mut names = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| sqlite_reference_build_error("query-database-list", None, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| sqlite_reference_build_error("read-database-list", None, error))?;
    names.sort();
    Ok(names)
}

fn capture_table_row_counts<'a>(
    connection: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
    label: &'static str,
) -> Result<BTreeMap<String, i64>, GlobalSchemaCatalogError> {
    let mut counts = BTreeMap::new();
    for table in tables {
        if table.is_empty()
            || !table
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(GlobalSchemaCatalogError::SqliteReferenceBuildFailure {
                stage: "validate-row-count-table",
                ddl_id: None,
                detail: format!("{label} table is not a frozen bare identifier: {table:?}"),
            });
        }
        let count = connection
            .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| {
                sqlite_reference_build_error("capture-row-count", Some(table), error)
            })?;
        counts.insert(table.to_owned(), count);
    }
    Ok(counts)
}

fn capture_selection_payload_schema_contract(objects: &[CatalogObjectRow]) -> Vec<String> {
    let Some(recovery_table) = objects.iter().find(|row| {
        row.identity.kind == CatalogObjectKind::Table
            && row.identity.name == "selection_v2_recovery_envelopes"
    }) else {
        return Vec::new();
    };
    FINAL_SELECTION_PAYLOAD_SCHEMAS
        .iter()
        .chain(TRANSITIONAL_SELECTION_PAYLOAD_SCHEMAS.iter())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|schema| {
            recovery_table
                .exact_sql
                .contains(&format!("payload_schema='{schema}'"))
        })
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DatabaseHalfState {
    PreAmendment,
    Transitional,
    Amended,
}

#[derive(Debug)]
struct OwnerBuiltCatalogState {
    reference: CatalogReferenceState,
    executed_ddl_ids: BTreeSet<String>,
}

#[derive(Debug)]
struct OwnerBuiltSameRuntimeCatalogs {
    mode: GlobalSchemaCatalogMode,
    runtime: SqliteRuntimeIdentity,
    legacy: OwnerBuiltCatalogState,
    transitional: OwnerBuiltCatalogState,
    amended: OwnerBuiltCatalogState,
}

/// Execute all three registered generation-1 catalog states in isolated
/// in-memory databases linked to this process's SQLite library, then issue an
/// opaque comparison capability. No expected sqlite_schema SQL is synthesized:
/// every reference row is captured from SQLite after executing the frozen
/// legacy fixture and the selection module's own read-only DDL plan.
pub(crate) fn build_same_runtime_catalog_references(
    mode: GlobalSchemaCatalogMode,
) -> Result<SameRuntimeCatalogReferences, GlobalSchemaCatalogError> {
    issue_same_runtime_catalog_references(build_owner_catalogs_from_linked_sqlite(mode)?)
}

#[cfg(test)]
fn build_legacy_same_runtime_catalog(
) -> Result<(SqliteRuntimeIdentity, OwnerBuiltCatalogState), GlobalSchemaCatalogError> {
    build_same_runtime_catalog_state(GlobalSchemaCatalogMode::Test, None)
}

fn build_owner_catalogs_from_linked_sqlite(
    mode: GlobalSchemaCatalogMode,
) -> Result<OwnerBuiltSameRuntimeCatalogs, GlobalSchemaCatalogError> {
    let (runtime, legacy) = build_same_runtime_catalog_state(mode, None)?;
    let (transitional_runtime, transitional) =
        build_same_runtime_catalog_state(mode, Some(SelectionCatalogDdlPhase::Transitional))?;
    let (amended_runtime, amended) =
        build_same_runtime_catalog_state(mode, Some(SelectionCatalogDdlPhase::Final))?;
    if runtime != transitional_runtime || runtime != amended_runtime {
        return Err(GlobalSchemaCatalogError::RuntimeIdentityMismatch);
    }
    Ok(OwnerBuiltSameRuntimeCatalogs {
        mode,
        runtime,
        legacy,
        transitional,
        amended,
    })
}

fn build_same_runtime_catalog_state(
    mode: GlobalSchemaCatalogMode,
    phase: Option<SelectionCatalogDdlPhase>,
) -> Result<(SqliteRuntimeIdentity, OwnerBuiltCatalogState), GlobalSchemaCatalogError> {
    let label = phase.map_or("legacy", SelectionCatalogDdlPhase::label);
    let connection = Connection::open_in_memory()
        .map_err(|error| sqlite_reference_build_error("open-in-memory", None, error))?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| sqlite_reference_build_error("configure", None, error))?;
    let runtime = capture_sqlite_runtime_identity(&connection)?;
    let registry = legacy_catalog_registry_entries_v1()?;
    let ddl = legacy_catalog_ddl_entries_v1()?;
    let mut executed_ddl_ids = BTreeSet::new();

    execute_legacy_catalog_ddl(&connection, &registry, &ddl, label, &mut executed_ddl_ids)?;
    if let Some(phase) = phase {
        execute_selection_catalog_ddl(&connection, mode, phase, &mut executed_ddl_ids)?;
    }

    let expected_registry = match phase {
        Some(phase) => whole_application_registry(mode, phase)?,
        None => registry,
    };
    let built = OwnerBuiltCatalogState {
        reference: capture_catalog_reference_state(&connection, mode)?,
        executed_ddl_ids,
    };
    validate_reference_state(label, mode, &built, &expected_registry)?;
    Ok((runtime, built))
}

#[cfg(test)]
pub(super) fn install_exact_selection_catalog_for_test(
    connection: &Connection,
    mode: GlobalSchemaCatalogMode,
    final_catalog: bool,
) -> Result<(), GlobalSchemaCatalogError> {
    let registry = legacy_catalog_registry_entries_v1()?;
    let ddl = legacy_catalog_ddl_entries_v1()?;
    let mut executed_ddl_ids = BTreeSet::new();
    execute_legacy_catalog_ddl(
        connection,
        &registry,
        &ddl,
        "owner-inspection-test",
        &mut executed_ddl_ids,
    )?;
    execute_selection_catalog_ddl(
        connection,
        mode,
        if final_catalog {
            SelectionCatalogDdlPhase::Final
        } else {
            SelectionCatalogDdlPhase::Transitional
        },
        &mut executed_ddl_ids,
    )
}

fn execute_legacy_catalog_ddl(
    connection: &Connection,
    registry: &[FrozenCatalogRegistryEntry],
    ddl: &[CheckedInDdlStatement],
    catalog: &'static str,
    executed_ddl_ids: &mut BTreeSet<String>,
) -> Result<(), GlobalSchemaCatalogError> {
    if registry.len() != ddl.len() {
        return Err(GlobalSchemaCatalogError::FrozenDdlRegistryMismatch {
            missing: registry.iter().map(|entry| entry.ddl_id.clone()).collect(),
            extra: ddl.iter().map(|entry| entry.ddl_id.clone()).collect(),
        });
    }
    for (registry_entry, ddl_entry) in registry.iter().zip(ddl) {
        if registry_entry.ddl_id != ddl_entry.ddl_id {
            return Err(GlobalSchemaCatalogError::FrozenDdlRegistryMismatch {
                missing: vec![registry_entry.ddl_id.clone()],
                extra: vec![ddl_entry.ddl_id.clone()],
            });
        }
        connection
            .execute_batch(&ddl_entry.exact_sql)
            .map_err(|error| {
                sqlite_reference_build_error(
                    "execute-legacy-ddl",
                    Some(ddl_entry.ddl_id.as_str()),
                    error,
                )
            })?;
        let emitted = capture_exact_catalog_object(
            connection,
            &registry_entry.identity,
            ddl_entry.ddl_id.as_str(),
            catalog,
        )?;
        if emitted.exact_sql != ddl_entry.exact_sql {
            return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
                catalog,
                detail: format!(
                    "SQLite-emitted SQL differs from frozen SQL for ddl_id={:?}",
                    ddl_entry.ddl_id
                ),
            });
        }
        if !executed_ddl_ids.insert(ddl_entry.ddl_id.clone()) {
            return Err(GlobalSchemaCatalogError::InvalidFrozenDdlFixture {
                line: ddl_entry.source_line,
                detail: format!("ddl_id executed more than once: {:?}", ddl_entry.ddl_id),
            });
        }
    }
    Ok(())
}

fn execute_selection_catalog_ddl(
    connection: &Connection,
    mode: GlobalSchemaCatalogMode,
    phase: SelectionCatalogDdlPhase,
    executed_ddl_ids: &mut BTreeSet<String>,
) -> Result<(), GlobalSchemaCatalogError> {
    let plan =
        selection_v2_catalog_ddl_plan(selection_store_mode(mode), selection_ddl_phase(phase))
            .map_err(|error| {
                sqlite_reference_build_error("build-selection-ddl-plan", None, error)
            })?;
    let registry = selection_catalog_registry_entries_v1(mode, phase)?;
    let by_identity = registry
        .iter()
        .map(|entry| ((entry.identity.kind, entry.identity.name.as_str()), entry))
        .collect::<BTreeMap<_, _>>();
    let mut emitted_identities = BTreeSet::new();

    for statement in &plan {
        let key = (
            selection_catalog_object_kind(statement.kind),
            statement.name.as_str(),
        );
        let registry_entry = by_identity.get(&key).ok_or_else(|| {
            GlobalSchemaCatalogError::GeneratedRegistryMismatch {
                catalog: "selection DDL plan",
                detail: format!(
                    "unregistered {:?} {} in {} plan",
                    statement.kind,
                    statement.name,
                    phase.label()
                ),
            }
        })?;
        connection
            .execute_batch(&statement.exact_sql)
            .map_err(|error| {
                sqlite_reference_build_error(
                    "execute-selection-ddl",
                    Some(registry_entry.ddl_id.as_str()),
                    error,
                )
            })?;
        let emitted = capture_exact_catalog_object(
            connection,
            &registry_entry.identity,
            registry_entry.ddl_id.as_str(),
            phase.label(),
        )?;
        if !emitted_identities.insert(emitted.identity) {
            return Err(GlobalSchemaCatalogError::GeneratedRegistryMismatch {
                catalog: "selection DDL plan",
                detail: format!("object emitted more than once: {key:?}"),
            });
        }
        if !executed_ddl_ids.insert(registry_entry.ddl_id.clone()) {
            return Err(GlobalSchemaCatalogError::GeneratedRegistryMismatch {
                catalog: "selection DDL plan",
                detail: format!(
                    "ddl_id executed more than once: {:?}",
                    registry_entry.ddl_id
                ),
            });
        }
    }
    let expected_identities = registry
        .iter()
        .map(|entry| entry.identity.clone())
        .collect::<BTreeSet<_>>();
    if emitted_identities != expected_identities {
        return Err(GlobalSchemaCatalogError::GeneratedRegistryMismatch {
            catalog: "selection DDL plan",
            detail: format!(
                "{} plan identities differ: expected={},actual={}",
                phase.label(),
                expected_identities.len(),
                emitted_identities.len()
            ),
        });
    }
    Ok(())
}

const fn selection_store_mode(mode: GlobalSchemaCatalogMode) -> SelectionV2StoreMode {
    match mode {
        GlobalSchemaCatalogMode::Production => SelectionV2StoreMode::Production,
        #[cfg(test)]
        GlobalSchemaCatalogMode::Test => SelectionV2StoreMode::Test,
    }
}

const fn selection_ddl_phase(phase: SelectionCatalogDdlPhase) -> SelectionV2CatalogDdlPhase {
    match phase {
        SelectionCatalogDdlPhase::Transitional => SelectionV2CatalogDdlPhase::Transitional,
        SelectionCatalogDdlPhase::Final => SelectionV2CatalogDdlPhase::Final,
    }
}

const fn selection_catalog_object_kind(kind: SelectionV2CatalogObjectKind) -> CatalogObjectKind {
    match kind {
        SelectionV2CatalogObjectKind::Table => CatalogObjectKind::Table,
        SelectionV2CatalogObjectKind::Index => CatalogObjectKind::Index,
        SelectionV2CatalogObjectKind::Trigger => CatalogObjectKind::Trigger,
    }
}

fn capture_sqlite_runtime_identity(
    connection: &Connection,
) -> Result<SqliteRuntimeIdentity, GlobalSchemaCatalogError> {
    let source_id = connection
        .query_row("SELECT sqlite_source_id()", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| sqlite_reference_build_error("capture-source-id", None, error))?;
    let mut statement = connection
        .prepare("PRAGMA compile_options")
        .map_err(|error| sqlite_reference_build_error("prepare-compile-options", None, error))?;
    let mut compile_options = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| sqlite_reference_build_error("query-compile-options", None, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| sqlite_reference_build_error("read-compile-options", None, error))?;
    compile_options.sort();
    if compile_options.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(GlobalSchemaCatalogError::InvalidRuntimeIdentity {
            detail: "SQLite reports duplicate compile options".to_owned(),
        });
    }
    let mut digest = Sha256::new();
    hash_field(
        &mut digest,
        b"stock_analysis.br180.sqlite_compile_options.v1",
    );
    digest.update((compile_options.len() as u64).to_be_bytes());
    for option in compile_options {
        hash_field(&mut digest, option.as_bytes());
    }
    let identity = SqliteRuntimeIdentity {
        libversion_number: rusqlite::version_number(),
        source_id,
        compile_options_sha256: lower_hex(&digest.finalize()),
    };
    validate_runtime_identity(&identity)?;
    Ok(identity)
}

fn capture_exact_catalog_object(
    connection: &Connection,
    expected: &CatalogObjectIdentity,
    ddl_id: &str,
    catalog: &'static str,
) -> Result<CatalogObjectRow, GlobalSchemaCatalogError> {
    let object_type = catalog_object_kind_label(expected.kind);
    let emitted = connection
        .query_row(
            "SELECT type, name, tbl_name, sql
             FROM sqlite_schema
             WHERE type = ?1 AND name = ?2 AND sql IS NOT NULL",
            params![object_type, expected.name],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            sqlite_reference_build_error("capture-emitted-object", Some(ddl_id), error)
        })?
        .ok_or_else(|| GlobalSchemaCatalogError::InvalidCatalogReference {
            catalog,
            detail: format!("ddl_id={ddl_id:?} did not emit its registered object"),
        })?;
    let identity = CatalogObjectIdentity {
        kind: parse_catalog_object_kind("capture-emitted-object", &emitted.0)?,
        name: emitted.1,
        table_name: emitted.2,
    };
    if identity != *expected {
        return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
            catalog,
            detail: format!(
                "ddl_id={ddl_id:?} emitted identity {identity:?}, expected {expected:?}"
            ),
        });
    }
    Ok(CatalogObjectRow {
        identity,
        exact_sql: emitted.3,
    })
}

fn capture_catalog_reference_state(
    connection: &Connection,
    mode: GlobalSchemaCatalogMode,
) -> Result<CatalogReferenceState, GlobalSchemaCatalogError> {
    let mut statement = connection
        .prepare(
            "SELECT type, name, tbl_name, sql
             FROM sqlite_schema
             WHERE type IN ('table','index','trigger','view')
             ORDER BY type, name, tbl_name",
        )
        .map_err(|error| sqlite_reference_build_error("prepare-catalog-capture", None, error))?;
    let raw_objects = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|error| sqlite_reference_build_error("query-catalog", None, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| sqlite_reference_build_error("read-catalog", None, error))?;

    let mut objects = Vec::new();
    let mut sqlite_owned_objects = Vec::new();
    for (object_type, name, table_name, exact_sql) in raw_objects {
        let kind = parse_catalog_object_kind("capture-catalog", &object_type)?;
        if name.to_ascii_lowercase().starts_with("sqlite_") {
            sqlite_owned_objects.push(SqliteOwnedCatalogObject {
                kind,
                name,
                table_name,
                exact_sql,
            });
        } else {
            let exact_sql =
                exact_sql.ok_or_else(|| GlobalSchemaCatalogError::InvalidCatalogReference {
                    catalog: "captured",
                    detail: format!(
                        "application object has NULL sqlite_schema.sql: type={object_type},name={name}"
                    ),
                })?;
            objects.push(CatalogObjectRow {
                identity: CatalogObjectIdentity {
                    kind,
                    name,
                    table_name,
                },
                exact_sql,
            });
        }
    }
    objects.sort();
    sqlite_owned_objects.sort();

    let foreign_keys = capture_foreign_keys(connection, &objects)?;
    let managed_index_geometry = capture_managed_index_geometry(connection, mode, &objects)?;
    Ok(CatalogReferenceState {
        objects,
        managed_index_geometry,
        foreign_keys,
        sqlite_owned_objects,
    })
}

fn capture_foreign_keys(
    connection: &Connection,
    objects: &[CatalogObjectRow],
) -> Result<Vec<ForeignKeyDependency>, GlobalSchemaCatalogError> {
    let mut foreign_keys = Vec::new();
    for table in objects
        .iter()
        .filter(|object| object.identity.kind == CatalogObjectKind::Table)
        .map(|object| object.identity.name.as_str())
    {
        let mut statement = connection
            .prepare(
                "SELECT id, seq, \"table\"
                 FROM pragma_foreign_key_list(?1)
                 ORDER BY id, seq",
            )
            .map_err(|error| {
                sqlite_reference_build_error("prepare-foreign-key-capture", None, error)
            })?;
        let rows = statement
            .query_map(params![table], |row| {
                Ok(ForeignKeyDependency {
                    source_table: table.to_owned(),
                    id: row.get(0)?,
                    sequence: row.get(1)?,
                    target_table: row.get(2)?,
                })
            })
            .map_err(|error| {
                sqlite_reference_build_error("query-foreign-key-capture", None, error)
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                sqlite_reference_build_error("read-foreign-key-capture", None, error)
            })?;
        foreign_keys.extend(rows);
    }
    foreign_keys.sort();
    Ok(foreign_keys)
}

fn capture_managed_index_geometry(
    connection: &Connection,
    mode: GlobalSchemaCatalogMode,
    objects: &[CatalogObjectRow],
) -> Result<Vec<ManagedIndexGeometry>, GlobalSchemaCatalogError> {
    let existing_tables = objects
        .iter()
        .filter(|object| object.identity.kind == CatalogObjectKind::Table)
        .map(|object| object.identity.name.as_str())
        .collect::<BTreeSet<_>>();
    let managed_tables = final_selection_catalog_registry_v1(mode)?
        .into_iter()
        .filter(|identity| identity.kind == CatalogObjectKind::Table)
        .map(|identity| identity.name)
        .filter(|table| existing_tables.contains(table.as_str()))
        .collect::<Vec<_>>();
    let mut geometry = Vec::new();
    for table in managed_tables {
        let mut statement = connection
            .prepare(
                "SELECT name, \"unique\", origin, partial
                 FROM pragma_index_list(?1)
                 ORDER BY seq",
            )
            .map_err(|error| {
                sqlite_reference_build_error("prepare-index-list-capture", None, error)
            })?;
        let indexes = statement
            .query_map(params![table], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|error| sqlite_reference_build_error("query-index-list-capture", None, error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                sqlite_reference_build_error("read-index-list-capture", None, error)
            })?;
        for (index_name, unique, origin, partial) in indexes {
            let mut xinfo_statement = connection
                .prepare(
                    "SELECT seqno, cid, name, \"desc\", coll, \"key\"
                     FROM pragma_index_xinfo(?1)
                     ORDER BY seqno",
                )
                .map_err(|error| {
                    sqlite_reference_build_error("prepare-index-xinfo-capture", None, error)
                })?;
            let terms = xinfo_statement
                .query_map(params![index_name], |row| {
                    Ok(IndexXinfoTerm {
                        seqno: row.get(0)?,
                        cid: row.get(1)?,
                        name: row.get(2)?,
                        descending: row.get::<_, i64>(3)? != 0,
                        collation: row.get(4)?,
                        key: row.get::<_, i64>(5)? != 0,
                    })
                })
                .map_err(|error| {
                    sqlite_reference_build_error("query-index-xinfo-capture", None, error)
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    sqlite_reference_build_error("read-index-xinfo-capture", None, error)
                })?;
            geometry.push(ManagedIndexGeometry {
                table_name: table.clone(),
                index_name,
                unique: unique != 0,
                origin,
                partial: partial != 0,
                terms,
            });
        }
    }
    geometry.sort();
    Ok(geometry)
}

const fn catalog_object_kind_label(kind: CatalogObjectKind) -> &'static str {
    match kind {
        CatalogObjectKind::Table => "table",
        CatalogObjectKind::Index => "index",
        CatalogObjectKind::Trigger => "trigger",
        CatalogObjectKind::View => "view",
    }
}

fn parse_catalog_object_kind(
    stage: &'static str,
    value: &str,
) -> Result<CatalogObjectKind, GlobalSchemaCatalogError> {
    match value {
        "table" => Ok(CatalogObjectKind::Table),
        "index" => Ok(CatalogObjectKind::Index),
        "trigger" => Ok(CatalogObjectKind::Trigger),
        "view" => Ok(CatalogObjectKind::View),
        other => Err(sqlite_reference_build_error(
            stage,
            None,
            format!("unsupported sqlite_schema.type {other:?}"),
        )),
    }
}

fn sqlite_reference_build_error(
    stage: &'static str,
    ddl_id: Option<&str>,
    detail: impl fmt::Display,
) -> GlobalSchemaCatalogError {
    GlobalSchemaCatalogError::SqliteReferenceBuildFailure {
        stage,
        ddl_id: ddl_id.map(str::to_owned),
        detail: detail.to_string(),
    }
}

/// Private issuance boundary for the output of the global owner's isolated
/// same-linked-runtime SQLite reference build.
///
/// `OwnerBuiltSameRuntimeCatalogs` is module-private, so raw SQL/reference
/// rows cannot be supplied by an application caller. The public crate seam
/// above executes the exact checked-in DDL selected by every `ddl_id`, captures
/// SQLite-emitted rows/PRAGMAs, and only then invokes this issuer.
fn issue_same_runtime_catalog_references(
    built: OwnerBuiltSameRuntimeCatalogs,
) -> Result<SameRuntimeCatalogReferences, GlobalSchemaCatalogError> {
    validate_runtime_identity(&built.runtime)?;
    let legacy_registry = legacy_catalog_registry_entries_v1()?;
    let transitional_registry =
        whole_application_registry(built.mode, SelectionCatalogDdlPhase::Transitional)?;
    let amended_registry = whole_application_registry(built.mode, SelectionCatalogDdlPhase::Final)?;
    validate_reference_state("legacy", built.mode, &built.legacy, &legacy_registry)?;
    validate_reference_state(
        "transitional",
        built.mode,
        &built.transitional,
        &transitional_registry,
    )?;
    validate_reference_state("amended", built.mode, &built.amended, &amended_registry)?;
    Ok(SameRuntimeCatalogReferences {
        mode: built.mode,
        runtime: built.runtime,
        legacy: built.legacy.reference,
        transitional: built.transitional.reference,
        amended: built.amended.reference,
    })
}

fn whole_application_registry(
    mode: GlobalSchemaCatalogMode,
    phase: SelectionCatalogDdlPhase,
) -> Result<Vec<FrozenCatalogRegistryEntry>, GlobalSchemaCatalogError> {
    let mut registry = legacy_catalog_registry_entries_v1()?;
    registry.extend(selection_catalog_registry_entries_v1(mode, phase)?);
    Ok(registry)
}

fn validate_reference_state(
    label: &'static str,
    mode: GlobalSchemaCatalogMode,
    built: &OwnerBuiltCatalogState,
    expected_registry: &[FrozenCatalogRegistryEntry],
) -> Result<(), GlobalSchemaCatalogError> {
    validate_catalog_rows(label, &built.reference.objects)?;
    let actual_identities = built
        .reference
        .objects
        .iter()
        .map(|row| row.identity.clone())
        .collect::<BTreeSet<_>>();
    let expected_identities = expected_registry
        .iter()
        .map(|entry| entry.identity.clone())
        .collect::<BTreeSet<_>>();
    if actual_identities != expected_identities {
        return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
            catalog: label,
            detail: format!(
                "object identities differ: expected={},actual={}",
                expected_identities.len(),
                actual_identities.len()
            ),
        });
    }
    let expected_ddl_ids = expected_registry
        .iter()
        .map(|entry| entry.ddl_id.clone())
        .collect::<BTreeSet<_>>();
    if built.executed_ddl_ids != expected_ddl_ids {
        return Err(GlobalSchemaCatalogError::ReferenceDdlRegistryMismatch {
            catalog: label,
            missing: expected_ddl_ids
                .difference(&built.executed_ddl_ids)
                .cloned()
                .collect(),
            extra: built
                .executed_ddl_ids
                .difference(&expected_ddl_ids)
                .cloned()
                .collect(),
        });
    }
    validate_catalog_safety(
        label,
        mode,
        &built.reference.objects,
        &built.reference.managed_index_geometry,
        &built.reference.foreign_keys,
        &built.reference.sqlite_owned_objects,
    )
}

fn validate_catalog_rows(
    label: &'static str,
    rows: &[CatalogObjectRow],
) -> Result<(), GlobalSchemaCatalogError> {
    let identities = rows
        .iter()
        .map(|row| row.identity.clone())
        .collect::<BTreeSet<_>>();
    if identities.len() != rows.len() {
        return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
            catalog: label,
            detail: "duplicate object identity".to_owned(),
        });
    }
    for row in rows {
        if row.exact_sql.is_empty() || row.exact_sql.contains('\0') {
            return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
                catalog: label,
                detail: format!(
                    "registered sqlite_schema.sql is empty or contains NUL: {}",
                    row.identity.name
                ),
            });
        }
        if row
            .identity
            .name
            .to_ascii_lowercase()
            .starts_with("sqlite_")
        {
            return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
                catalog: label,
                detail: format!(
                    "application catalog contains forbidden SQLite-owned name: {}",
                    row.identity.name
                ),
            });
        }
        if matches!(
            row.identity.kind,
            CatalogObjectKind::Table | CatalogObjectKind::View
        ) && row.identity.name != row.identity.table_name
        {
            return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
                catalog: label,
                detail: format!(
                    "{:?} name/tbl_name mismatch: name={},tbl_name={}",
                    row.identity.kind, row.identity.name, row.identity.table_name
                ),
            });
        }
    }
    Ok(())
}

fn validate_attached_schema_names(actual: &[String]) -> Result<(), GlobalSchemaCatalogError> {
    if actual != ["main"] && actual != ["main", "temp"] {
        return Err(GlobalSchemaCatalogError::AttachedSchemaMismatch {
            actual: actual.to_vec(),
        });
    }
    Ok(())
}

fn require_exact_ancillary_catalog(
    actual: &CatalogSnapshot,
    expected: &CatalogReferenceState,
) -> Result<(), GlobalSchemaCatalogError> {
    if canonical_index_geometry(&actual.managed_index_geometry)
        != canonical_index_geometry(&expected.managed_index_geometry)
    {
        return Err(GlobalSchemaCatalogError::ManagedIndexGeometryMismatch {
            detail: format!(
                "actual={} expected={}",
                actual.managed_index_geometry.len(),
                expected.managed_index_geometry.len()
            ),
        });
    }
    if canonical_foreign_keys(&actual.foreign_keys)
        != canonical_foreign_keys(&expected.foreign_keys)
    {
        return Err(GlobalSchemaCatalogError::CatalogMismatch {
            detail: format!(
                "foreign-key geometry differs: actual={} expected={}",
                actual.foreign_keys.len(),
                expected.foreign_keys.len()
            ),
        });
    }
    if canonical_sqlite_owned(&actual.sqlite_owned_objects)
        != canonical_sqlite_owned(&expected.sqlite_owned_objects)
    {
        return Err(GlobalSchemaCatalogError::SqliteOwnedCatalogMismatch {
            detail: format!(
                "actual={} expected={}",
                actual.sqlite_owned_objects.len(),
                expected.sqlite_owned_objects.len()
            ),
        });
    }
    Ok(())
}

fn validate_catalog_safety(
    label: &'static str,
    mode: GlobalSchemaCatalogMode,
    objects: &[CatalogObjectRow],
    index_geometry: &[ManagedIndexGeometry],
    foreign_keys: &[ForeignKeyDependency],
    sqlite_owned_objects: &[SqliteOwnedCatalogObject],
) -> Result<(), GlobalSchemaCatalogError> {
    let selection_catalog = final_selection_catalog_registry_v1(mode)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let managed = selection_catalog
        .iter()
        .filter(|identity| identity.kind == CatalogObjectKind::Table)
        .map(|identity| identity.name.clone())
        .collect::<BTreeSet<_>>();
    let contains_managed_catalog = objects.iter().any(|object| {
        object.identity.kind == CatalogObjectKind::Table && managed.contains(&object.identity.name)
    });
    validate_index_geometry(label, index_geometry, &managed, contains_managed_catalog)?;
    validate_sqlite_owned_objects(label, sqlite_owned_objects)?;
    for foreign_key in foreign_keys {
        if managed.contains(&foreign_key.target_table)
            && !managed.contains(&foreign_key.source_table)
        {
            return Err(GlobalSchemaCatalogError::ExternalForeignKeyToManagedTable {
                source_table: foreign_key.source_table.clone(),
                target_table: foreign_key.target_table.clone(),
            });
        }
    }
    for object in objects {
        let is_managed = managed.contains(&object.identity.table_name)
            && selection_catalog.contains(&object.identity);
        if !is_managed
            && matches!(
                object.identity.kind,
                CatalogObjectKind::Index | CatalogObjectKind::Trigger
            )
            && managed.contains(&object.identity.table_name)
        {
            return Err(
                GlobalSchemaCatalogError::ExternalObjectTargetsManagedTable {
                    object: object.identity.name.clone(),
                    target_table: object.identity.table_name.clone(),
                },
            );
        }
        if !is_managed {
            scan_external_sql_for_managed_references(
                &object.identity.name,
                &object.exact_sql,
                &managed,
            )?;
        }
    }
    Ok(())
}

fn validate_index_geometry(
    label: &'static str,
    geometry: &[ManagedIndexGeometry],
    managed_tables: &BTreeSet<String>,
    contains_managed_catalog: bool,
) -> Result<(), GlobalSchemaCatalogError> {
    let unique = geometry.iter().cloned().collect::<BTreeSet<_>>();
    if unique.len() != geometry.len() {
        return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
            catalog: label,
            detail: "duplicate index_list/index_xinfo geometry".to_owned(),
        });
    }
    if !contains_managed_catalog {
        if geometry.is_empty() {
            return Ok(());
        }
        return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
            catalog: label,
            detail: "pre-amendment catalog contains selection-managed index geometry".to_owned(),
        });
    }
    let covered_tables = geometry
        .iter()
        .map(|index| index.table_name.clone())
        .collect::<BTreeSet<_>>();
    let missing_tables = managed_tables
        .difference(&covered_tables)
        .cloned()
        .collect::<Vec<_>>();
    let expected_explicit_indexes = FINAL_SELECTION_INDEXES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    let actual_explicit_indexes = geometry
        .iter()
        .filter(|index| index.origin == "c")
        .map(|index| index.index_name.clone())
        .collect::<BTreeSet<_>>();
    if !missing_tables.is_empty() || actual_explicit_indexes != expected_explicit_indexes {
        return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
            catalog: label,
            detail: format!(
                "incomplete selection-managed index geometry: missing_tables={missing_tables:?},explicit={actual_explicit_indexes:?}"
            ),
        });
    }
    for index in geometry {
        if !managed_tables.contains(&index.table_name) {
            return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
                catalog: label,
                detail: format!(
                    "managed index geometry names non-managed table {}",
                    index.table_name
                ),
            });
        }
        if !matches!(index.origin.as_str(), "c" | "u" | "pk") {
            return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
                catalog: label,
                detail: format!(
                    "unsupported index origin {:?} for {}",
                    index.origin, index.index_name
                ),
            });
        }
        let mut previous = None;
        for term in &index.terms {
            if term.seqno < 0 || previous.is_some_and(|value| term.seqno <= value) {
                return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
                    catalog: label,
                    detail: format!(
                        "index_xinfo seqno is negative, duplicate, or unordered for {}",
                        index.index_name
                    ),
                });
            }
            previous = Some(term.seqno);
        }
    }
    Ok(())
}

fn validate_sqlite_owned_objects(
    label: &'static str,
    objects: &[SqliteOwnedCatalogObject],
) -> Result<(), GlobalSchemaCatalogError> {
    let unique = objects.iter().cloned().collect::<BTreeSet<_>>();
    if unique.len() != objects.len() {
        return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
            catalog: label,
            detail: "duplicate SQLite-owned object".to_owned(),
        });
    }
    if let Some(object) = objects
        .iter()
        .find(|object| !object.name.to_ascii_lowercase().starts_with("sqlite_"))
    {
        return Err(GlobalSchemaCatalogError::InvalidCatalogReference {
            catalog: label,
            detail: format!(
                "claimed SQLite-owned object lacks sqlite_ prefix: {}",
                object.name
            ),
        });
    }
    Ok(())
}

fn selection_managed_rows(
    mode: GlobalSchemaCatalogMode,
    objects: &[CatalogObjectRow],
) -> Result<Vec<CatalogObjectRow>, GlobalSchemaCatalogError> {
    let expected = final_selection_catalog_registry_v1(mode)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let rows = objects
        .iter()
        .filter(|row| expected.contains(&row.identity))
        .cloned()
        .collect::<Vec<_>>();
    let actual = rows
        .iter()
        .map(|row| row.identity.clone())
        .collect::<BTreeSet<_>>();
    if rows.len() != 70 || actual != expected {
        return Err(GlobalSchemaCatalogError::CatalogMismatch {
            detail: format!(
                "selection-managed catalog must contain exact same-mode 70 rows: actual={}",
                rows.len()
            ),
        });
    }
    Ok(rows)
}

fn canonical_index_geometry(rows: &[ManagedIndexGeometry]) -> Vec<ManagedIndexGeometry> {
    let mut canonical = rows.to_vec();
    canonical.sort();
    canonical
}

fn canonical_foreign_keys(rows: &[ForeignKeyDependency]) -> Vec<ForeignKeyDependency> {
    let mut canonical = rows.to_vec();
    canonical.sort();
    canonical
}

fn canonical_sqlite_owned(rows: &[SqliteOwnedCatalogObject]) -> Vec<SqliteOwnedCatalogObject> {
    let mut canonical = rows.to_vec();
    canonical.sort();
    canonical
}

fn scan_external_sql_for_managed_references(
    object: &str,
    sql: &str,
    managed_tables: &BTreeSet<String>,
) -> Result<(), GlobalSchemaCatalogError> {
    if sql.contains('\0') {
        return sql_scan_error(object, "NUL byte");
    }
    let chars = sql.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current.is_whitespace() {
            index += 1;
            continue;
        }
        if current == '-' && chars.get(index + 1) == Some(&'-') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if current == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            let mut closed = false;
            while index < chars.len() {
                if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
                    return sql_scan_error(object, "nested block comment");
                }
                if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
                    index += 2;
                    closed = true;
                    break;
                }
                index += 1;
            }
            if !closed {
                return sql_scan_error(object, "unterminated block comment");
            }
            continue;
        }
        if matches!(current, 'x' | 'X') && chars.get(index + 1) == Some(&'\'') {
            index += 2;
            let start = index;
            while index < chars.len() && chars[index] != '\'' {
                if !chars[index].is_ascii_hexdigit() {
                    return sql_scan_error(object, "invalid hex-blob token");
                }
                index += 1;
            }
            if index == chars.len() {
                return sql_scan_error(object, "unterminated hex-blob token");
            }
            if (index - start) % 2 != 0 {
                return sql_scan_error(object, "odd-length hex-blob token");
            }
            index += 1;
            continue;
        }
        if matches!(current, '\'' | '"' | '`' | '[') {
            let (decoded, next) = decode_quoted_sql_token(object, &chars, index)?;
            if managed_tables
                .iter()
                .any(|managed| decoded.eq_ignore_ascii_case(managed))
            {
                return sql_scan_error(
                    object,
                    &format!("quoted token references managed table {decoded:?}"),
                );
            }
            index = next;
            continue;
        }
        if is_bare_identifier_start(current) {
            let start = index;
            index += 1;
            while index < chars.len() && is_bare_identifier_continue(chars[index]) {
                index += 1;
            }
            let token = chars[start..index].iter().collect::<String>();
            if managed_tables
                .iter()
                .any(|managed| token.eq_ignore_ascii_case(managed))
            {
                return sql_scan_error(
                    object,
                    &format!("bare token references managed table {token:?}"),
                );
            }
            continue;
        }
        if current.is_ascii_digit() {
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric()
                    || matches!(chars[index], '.' | '_' | '+' | '-'))
            {
                index += 1;
            }
            continue;
        }
        if "(),.;=<>+-*/%|&~?!:@".contains(current) {
            index += 1;
            continue;
        }
        return sql_scan_error(
            object,
            &format!("unparsed token class starting with {current:?}"),
        );
    }
    Ok(())
}

fn decode_quoted_sql_token(
    object: &str,
    chars: &[char],
    start: usize,
) -> Result<(String, usize), GlobalSchemaCatalogError> {
    let opening = chars[start];
    let closing = if opening == '[' { ']' } else { opening };
    let mut decoded = String::new();
    let mut index = start + 1;
    while index < chars.len() {
        if chars[index] == closing {
            if chars.get(index + 1) == Some(&closing) {
                decoded.push(closing);
                index += 2;
                continue;
            }
            return Ok((decoded, index + 1));
        }
        decoded.push(chars[index]);
        index += 1;
    }
    sql_scan_error(object, "unterminated quoted token")
}

fn is_bare_identifier_start(value: char) -> bool {
    value == '_' || value == '$' || value.is_alphabetic() || !value.is_ascii()
}

fn is_bare_identifier_continue(value: char) -> bool {
    is_bare_identifier_start(value) || value.is_ascii_digit()
}

fn sql_scan_error<T>(object: &str, detail: &str) -> Result<T, GlobalSchemaCatalogError> {
    Err(GlobalSchemaCatalogError::SqlScanFailure {
        object: object.to_owned(),
        detail: detail.to_owned(),
    })
}

fn validate_runtime_identity(
    runtime: &SqliteRuntimeIdentity,
) -> Result<(), GlobalSchemaCatalogError> {
    if !(SQLITE_MINIMUM_LIBVERSION_NUMBER..SQLITE_NEXT_MAJOR_LIBVERSION_NUMBER)
        .contains(&runtime.libversion_number)
    {
        return Err(GlobalSchemaCatalogError::UnsupportedSqliteRuntime {
            actual: runtime.libversion_number,
        });
    }
    if runtime.source_id.trim().is_empty() {
        return Err(GlobalSchemaCatalogError::InvalidRuntimeIdentity {
            detail: "source_id is empty".to_owned(),
        });
    }
    if !is_lower_hex_sha256(&runtime.compile_options_sha256) {
        return Err(GlobalSchemaCatalogError::InvalidRuntimeIdentity {
            detail: "compile-options SHA-256 is not lowercase hexadecimal".to_owned(),
        });
    }
    Ok(())
}

fn legacy_table_name_set() -> Result<BTreeSet<String>, GlobalSchemaCatalogError> {
    Ok(legacy_catalog_registry_v1()?
        .into_iter()
        .filter(|row| row.kind == CatalogObjectKind::Table)
        .map(|row| row.name)
        .collect())
}

fn canonical_catalog(rows: &[CatalogObjectRow]) -> Vec<CatalogObjectRow> {
    let mut canonical = rows.to_vec();
    canonical.sort();
    canonical
}

fn sorted_strings(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn whole_application_catalog_sha256(
    mode: GlobalSchemaCatalogMode,
    runtime: &SqliteRuntimeIdentity,
    rows: &[CatalogObjectRow],
) -> WholeApplicationSchemaCatalogSha256 {
    WholeApplicationSchemaCatalogSha256(catalog_digest(
        b"stock_analysis.br180.whole_application_schema_catalog.v1",
        mode,
        runtime,
        rows,
    ))
}

fn selection_managed_catalog_sha256(
    mode: GlobalSchemaCatalogMode,
    runtime: &SqliteRuntimeIdentity,
    rows: &[CatalogObjectRow],
) -> SelectionManagedSchemaCatalogSha256 {
    debug_assert_eq!(
        rows.len(),
        70,
        "selection-managed digest requires exact same-mode 70 rows"
    );
    SelectionManagedSchemaCatalogSha256(catalog_digest(
        b"stock_analysis.br180.selection_managed_schema_catalog.v2",
        mode,
        runtime,
        rows,
    ))
}

fn catalog_digest(
    domain: &[u8],
    mode: GlobalSchemaCatalogMode,
    runtime: &SqliteRuntimeIdentity,
    rows: &[CatalogObjectRow],
) -> String {
    let mut digest = Sha256::new();
    hash_field(&mut digest, domain);
    hash_field(&mut digest, mode.label().as_bytes());
    digest.update(runtime.libversion_number.to_be_bytes());
    hash_field(&mut digest, runtime.source_id.as_bytes());
    hash_field(&mut digest, runtime.compile_options_sha256.as_bytes());
    let rows = canonical_catalog(rows);
    digest.update((rows.len() as u64).to_be_bytes());
    for row in rows {
        digest.update([row.identity.kind.ordinal()]);
        hash_field(&mut digest, row.identity.name.as_bytes());
        hash_field(&mut digest, row.identity.table_name.as_bytes());
        hash_field(&mut digest, row.exact_sql.as_bytes());
    }
    lower_hex(&digest.finalize())
}

fn hash_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_legacy_registry_names_every_whole_application_object() {
        let registry = legacy_catalog_registry_v1().expect("frozen legacy registry");
        let table_count = registry
            .iter()
            .filter(|row| row.kind == CatalogObjectKind::Table)
            .count();
        let index_count = registry
            .iter()
            .filter(|row| row.kind == CatalogObjectKind::Index)
            .count();
        let trigger_count = registry
            .iter()
            .filter(|row| row.kind == CatalogObjectKind::Trigger)
            .count();

        assert_eq!(registry.len(), 160);
        assert_eq!((table_count, index_count, trigger_count), (53, 44, 63));
        assert_eq!(
            registry.first().map(|row| row.name.as_str()),
            Some("account_mode_log")
        );
        assert!(registry.iter().any(|row| {
            row.kind == CatalogObjectKind::Table && row.name == "selection_event_inbox"
        }));
        assert!(registry.iter().any(|row| {
            row.kind == CatalogObjectKind::Trigger && row.name == "user_position_snapshot_no_update"
        }));
    }

    #[test]
    fn frozen_legacy_registry_binds_ddl_ids_and_real_fixture_lines() {
        let registry =
            legacy_catalog_registry_entries_v1().expect("frozen legacy registry entries");
        let first = registry.first().expect("first frozen registry entry");
        assert_eq!(first.identity.name, "account_mode_log");
        assert_eq!(first.ddl_id, "legacy-v1.table.account_mode_log");
        assert_eq!(first.source_line, 5);
        assert_eq!(registry.last().expect("last entry").source_line, 164);
    }

    #[test]
    fn frozen_legacy_ddl_is_an_exact_one_to_one_registry_mapping() {
        let registry =
            legacy_catalog_registry_entries_v1().expect("frozen legacy registry entries");
        let ddl = legacy_catalog_ddl_entries_v1().expect("frozen legacy DDL entries");

        assert_eq!(ddl.len(), 160);
        assert_eq!(
            ddl.iter()
                .map(|entry| entry.ddl_id.as_str())
                .collect::<Vec<_>>(),
            registry
                .iter()
                .map(|entry| entry.ddl_id.as_str())
                .collect::<Vec<_>>()
        );
        assert!(ddl.iter().all(|entry| !entry.exact_sql.is_empty()));
        assert_eq!(
            ddl.first().map(|entry| entry.exact_sql.as_str()),
            Some(
                "CREATE TABLE account_mode_log (\n                id              INTEGER PRIMARY KEY AUTOINCREMENT,\n                ts              TIMESTAMP NOT NULL,\n                prev_mode       TEXT NOT NULL,\n                new_mode        TEXT NOT NULL,\n                trigger_reason  TEXT NOT NULL,\n                today_pnl_pct   REAL,\n                consecutive_n   INTEGER,\n                total_pos_cheng INTEGER,\n                data_complete   INTEGER NOT NULL DEFAULT 1,\n                pushed          INTEGER NOT NULL DEFAULT 0,\n                push_attempted_at TIMESTAMP\n            )"
            )
        );
    }

    #[test]
    fn frozen_legacy_ddl_parser_rejects_invalid_hex_and_registry_drift() {
        let registry =
            legacy_catalog_registry_entries_v1().expect("frozen legacy registry entries");
        let one = registry[..1].to_vec();
        let invalid_hex = format!("{}|0g\n", one[0].ddl_id);
        assert!(matches!(
            parse_legacy_ddl_fixture_rows(&invalid_hex, &one),
            Err(GlobalSchemaCatalogError::InvalidFrozenDdlFixture { line: 1, .. })
        ));

        let unknown = "legacy-v1.table.unknown|435245415445205441424C4520756E6B6E6F776E28696420494E544547455229\n"
            .to_owned();
        assert!(matches!(
            parse_legacy_ddl_fixture_rows(&unknown, &one),
            Err(GlobalSchemaCatalogError::FrozenDdlRegistryMismatch { .. })
        ));

        assert!(matches!(
            parse_legacy_ddl_fixture_rows("", &one),
            Err(GlobalSchemaCatalogError::FrozenDdlRegistryMismatch { .. })
        ));
    }

    #[test]
    fn legacy_reference_is_executed_and_captured_by_the_linked_sqlite_runtime() {
        let (runtime, built) =
            build_legacy_same_runtime_catalog().expect("real in-memory legacy reference");
        let registry =
            legacy_catalog_registry_entries_v1().expect("frozen legacy registry entries");
        let ddl = legacy_catalog_ddl_entries_v1().expect("frozen legacy DDL entries");

        validate_runtime_identity(&runtime).expect("captured runtime identity");
        validate_reference_state("legacy", GlobalSchemaCatalogMode::Test, &built, &registry)
            .expect("captured legacy reference");
        assert_eq!(built.reference.objects.len(), 160);
        assert_eq!(
            canonical_catalog(&built.reference.objects),
            registry
                .iter()
                .zip(ddl.iter())
                .map(|(registry_entry, ddl_entry)| CatalogObjectRow {
                    identity: registry_entry.identity.clone(),
                    exact_sql: ddl_entry.exact_sql.clone(),
                })
                .collect::<Vec<_>>()
        );
        assert!(built.reference.managed_index_geometry.is_empty());
        assert!(built
            .reference
            .sqlite_owned_objects
            .iter()
            .any(|object| object.name == "sqlite_sequence"));
    }

    #[test]
    fn all_three_references_are_executed_and_captured_by_one_linked_sqlite_runtime() {
        for mode in [
            GlobalSchemaCatalogMode::Production,
            GlobalSchemaCatalogMode::Test,
        ] {
            let references = build_same_runtime_catalog_references(mode)
                .expect("build real legacy/transitional/final references");
            validate_runtime_identity(&references.runtime).expect("real SQLite runtime identity");
            assert_eq!(references.legacy.objects.len(), 160);
            assert_eq!(references.transitional.objects.len(), 230);
            assert_eq!(references.amended.objects.len(), 230);
            assert!(references.legacy.managed_index_geometry.is_empty());
            assert!(!references.transitional.managed_index_geometry.is_empty());
            assert!(!references.amended.managed_index_geometry.is_empty());
            assert!(references
                .transitional
                .objects
                .iter()
                .chain(&references.amended.objects)
                .all(|row| row.exact_sql.starts_with("CREATE ")));
        }
    }

    #[test]
    fn owner_issued_capture_reads_all_twelve_selection_row_counts_from_one_connection() {
        let connection = Connection::open_in_memory().expect("open TEST_CODE SQLite");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("configure TEST_CODE SQLite");
        let registry =
            legacy_catalog_registry_entries_v1().expect("frozen legacy registry entries");
        let ddl = legacy_catalog_ddl_entries_v1().expect("frozen legacy DDL entries");
        let mut executed = BTreeSet::new();
        execute_legacy_catalog_ddl(
            &connection,
            &registry,
            &ddl,
            "owner-capture-test",
            &mut executed,
        )
        .expect("install exact legacy catalog");
        execute_selection_catalog_ddl(
            &connection,
            GlobalSchemaCatalogMode::Test,
            SelectionCatalogDdlPhase::Final,
            &mut executed,
        )
        .expect("install exact final test catalog");
        connection
            .pragma_update(None, "application_id", STOCK_ANALYSIS_SQLITE_APPLICATION_ID)
            .expect("set TEST_CODE application_id");
        connection
            .pragma_update(None, "user_version", STOCK_ANALYSIS_DB_SCHEMA_GENERATION)
            .expect("set TEST_CODE user_version");

        let authority =
            super::super::global_schema_v1::SelectionCatalogCaptureAuthority::for_test_code();
        let snapshot =
            capture_catalog_snapshot(&authority, &connection, GlobalSchemaCatalogMode::Test)
                .expect("owner captures the exact TEST_CODE catalog");

        assert_eq!(snapshot.selection_row_counts().len(), 12);
        assert!(FINAL_SELECTION_TABLES
            .iter()
            .all(|table| { snapshot.selection_row_counts().get(*table) == Some(&0) }));
        let references = same_runtime_references(GlobalSchemaCatalogMode::Test);
        assert!(matches!(
            classify_database_half(&snapshot, &references),
            Ok(DatabaseHalfDiagnostic::AmendedDatabaseHalf(_))
        ));
    }

    #[test]
    fn owner_capture_classifies_empty_zero_zero_database_as_diagnostic_half_only() {
        let connection = Connection::open_in_memory().expect("open empty TEST_CODE SQLite");
        let authority =
            super::super::global_schema_v1::SelectionCatalogCaptureAuthority::for_test_code();
        let snapshot =
            capture_catalog_snapshot(&authority, &connection, GlobalSchemaCatalogMode::Test)
                .expect("capture empty TEST_CODE database half");
        let references = same_runtime_references(GlobalSchemaCatalogMode::Test);

        assert!(matches!(
            classify_database_half(&snapshot, &references),
            Ok(DatabaseHalfDiagnostic::AbsentDatabaseHalf(_))
        ));
        assert_eq!(snapshot.selection_row_counts().len(), 12);
        assert!(snapshot
            .selection_row_counts()
            .values()
            .all(|count| *count == 0));
    }

    #[test]
    fn test_mode_real_references_capture_only_test_symbol_isolation_triggers() {
        let references = build_same_runtime_catalog_references(GlobalSchemaCatalogMode::Test)
            .expect("build real Test-mode legacy/transitional/final references");

        for catalog in [&references.transitional, &references.amended] {
            let test_symbols = catalog
                .objects
                .iter()
                .filter(|row| {
                    row.identity.kind == CatalogObjectKind::Trigger
                        && row.identity.name.ends_with("_symbol_isolation_test")
                })
                .count();
            let production_symbols = catalog
                .objects
                .iter()
                .filter(|row| {
                    row.identity.kind == CatalogObjectKind::Trigger
                        && row.identity.name.ends_with("_symbol_isolation_production")
                })
                .count();
            assert_eq!(test_symbols, 3);
            assert_eq!(production_symbols, 0);
        }
    }

    #[test]
    fn row_preservation_requires_every_legacy_table_count_to_match() {
        let registry = legacy_catalog_registry_v1().expect("frozen legacy registry");
        let mut source = registry
            .iter()
            .filter(|row| row.kind == CatalogObjectKind::Table)
            .map(|row| (row.name.clone(), 0_i64))
            .collect::<BTreeMap<_, _>>();
        source.insert("selection_event_inbox".to_owned(), 669);
        source.insert("selection_event_completions".to_owned(), 295);
        let candidate = source.clone();

        verify_legacy_row_preservation(&source, &candidate)
            .expect("all 53 legacy table counts are preserved");

        let mut changed = candidate.clone();
        changed.insert("selection_event_inbox".to_owned(), 668);
        assert!(matches!(
            verify_legacy_row_preservation(&source, &changed),
            Err(GlobalSchemaCatalogError::LegacyRowCountChanged {
                ref table,
                source: 669,
                candidate: 668,
            }) if table == "selection_event_inbox"
        ));

        let mut incomplete = candidate;
        incomplete.remove("ledger");
        assert!(matches!(
            verify_legacy_row_preservation(&source, &incomplete),
            Err(GlobalSchemaCatalogError::LegacyRowRegistryMismatch { .. })
        ));
    }

    #[test]
    fn final_selection_registry_is_exact_and_mode_keyed() {
        let production = final_selection_catalog_registry_v1(GlobalSchemaCatalogMode::Production)
            .expect("production selection registry");
        let test = final_selection_catalog_registry_v1(GlobalSchemaCatalogMode::Test)
            .expect("test selection registry");

        for registry in [&production, &test] {
            assert_eq!(registry.len(), 70);
            assert_eq!(
                registry
                    .iter()
                    .filter(|row| row.kind == CatalogObjectKind::Table)
                    .count(),
                12
            );
            assert_eq!(
                registry
                    .iter()
                    .filter(|row| row.kind == CatalogObjectKind::Index)
                    .count(),
                5
            );
            assert_eq!(
                registry
                    .iter()
                    .filter(|row| row.kind == CatalogObjectKind::Trigger)
                    .count(),
                53
            );
        }
        assert!(production
            .iter()
            .any(|row| { row.name == "selection_v2_relation_symbol_isolation_production" }));
        assert!(!production
            .iter()
            .any(|row| row.name == "selection_v2_relation_symbol_isolation_test"));
        assert!(test
            .iter()
            .any(|row| row.name == "selection_v2_relation_symbol_isolation_test"));
        assert!(!test
            .iter()
            .any(|row| { row.name == "selection_v2_relation_symbol_isolation_production" }));
    }

    #[test]
    fn exact_same_runtime_catalogs_classify_three_database_half_states() {
        let references = same_runtime_references(GlobalSchemaCatalogMode::Test);
        let pre = snapshot_for_state(&references, DatabaseHalfState::PreAmendment);
        assert!(matches!(
            classify_database_half(&pre, &references),
            Ok(DatabaseHalfDiagnostic::PreAmendment(_))
        ));

        let transitional = snapshot_for_state(&references, DatabaseHalfState::Transitional);
        assert!(matches!(
            classify_database_half(&transitional, &references),
            Ok(DatabaseHalfDiagnostic::Transitional(_))
        ));

        let amended = snapshot_for_state(&references, DatabaseHalfState::Amended);
        assert!(matches!(
            classify_database_half(&amended, &references),
            Ok(DatabaseHalfDiagnostic::AmendedDatabaseHalf(_))
        ));
    }

    #[test]
    fn exact_catalog_verification_rejects_ddl_and_runtime_drift() {
        let references = same_runtime_references(GlobalSchemaCatalogMode::Production);
        let mut actual = snapshot_for_state(&references, DatabaseHalfState::PreAmendment);
        let ledger = actual
            .objects
            .iter_mut()
            .find(|row| row.identity.name == "ledger")
            .expect("ledger catalog row");
        ledger.exact_sql.push(' ');
        assert!(matches!(
            classify_database_half(&actual, &references),
            Err(GlobalSchemaCatalogError::CatalogMismatch { .. })
        ));

        actual = snapshot_for_state(&references, DatabaseHalfState::PreAmendment);
        actual.runtime.source_id.push_str("-different");
        assert!(matches!(
            classify_database_half(&actual, &references),
            Err(GlobalSchemaCatalogError::RuntimeIdentityMismatch)
        ));
    }

    #[test]
    fn future_generation_and_incomplete_final_payload_fail_closed() {
        let references = same_runtime_references(GlobalSchemaCatalogMode::Test);
        let mut actual = snapshot_for_state(&references, DatabaseHalfState::Amended);
        actual.identity.user_version = 2;
        assert!(matches!(
            classify_database_half(&actual, &references),
            Err(GlobalSchemaCatalogError::UnsupportedFutureGeneration {
                actual: 2,
                supported: 1,
            })
        ));

        actual.identity.user_version = 1;
        actual
            .selection_payload_schemas
            .retain(|schema| schema != "outcome-claim-stage-v2");
        assert!(matches!(
            classify_database_half(&actual, &references),
            Err(GlobalSchemaCatalogError::PayloadSchemaMismatch { .. })
        ));
    }

    #[test]
    fn ancillary_catalog_and_external_dependency_checks_fail_closed() {
        let references = same_runtime_references(GlobalSchemaCatalogMode::Production);

        let mut missing_geometry = snapshot_for_state(&references, DatabaseHalfState::Amended);
        missing_geometry.managed_index_geometry.clear();
        assert!(matches!(
            classify_database_half(&missing_geometry, &references),
            Err(GlobalSchemaCatalogError::InvalidCatalogReference { .. })
        ));

        let mut extra_view = snapshot_for_state(&references, DatabaseHalfState::Amended);
        extra_view.objects.push(CatalogObjectRow {
            identity: CatalogObjectIdentity {
                kind: CatalogObjectKind::View,
                name: "external_safe_view".to_owned(),
                table_name: "external_safe_view".to_owned(),
            },
            exact_sql: "CREATE VIEW external_safe_view AS SELECT 1".to_owned(),
        });
        assert!(matches!(
            classify_database_half(&extra_view, &references),
            Err(GlobalSchemaCatalogError::CatalogMismatch { .. })
        ));

        let mut external_fk = snapshot_for_state(&references, DatabaseHalfState::Amended);
        external_fk.foreign_keys.push(ForeignKeyDependency {
            source_table: "ledger".to_owned(),
            id: 99,
            sequence: 0,
            target_table: "selection_samples".to_owned(),
        });
        assert!(matches!(
            classify_database_half(&external_fk, &references),
            Err(GlobalSchemaCatalogError::ExternalForeignKeyToManagedTable { .. })
        ));

        let mut external_trigger = snapshot_for_state(&references, DatabaseHalfState::Amended);
        external_trigger.objects.push(CatalogObjectRow {
            identity: CatalogObjectIdentity {
                kind: CatalogObjectKind::Trigger,
                name: "external_selection_samples_trigger".to_owned(),
                table_name: "selection_samples".to_owned(),
            },
            exact_sql:
                "CREATE TRIGGER external_selection_samples_trigger BEFORE INSERT ON selection_samples BEGIN SELECT 1; END"
                    .to_owned(),
        });
        assert!(matches!(
            classify_database_half(&external_trigger, &references),
            Err(GlobalSchemaCatalogError::ExternalObjectTargetsManagedTable { .. })
        ));
    }

    #[test]
    fn strict_external_sql_scanner_rejects_managed_tokens_and_malformed_input() {
        let references = same_runtime_references(GlobalSchemaCatalogMode::Test);
        let mut managed_reference = snapshot_for_state(&references, DatabaseHalfState::Amended);
        managed_reference.objects.push(CatalogObjectRow {
            identity: CatalogObjectIdentity {
                kind: CatalogObjectKind::View,
                name: "external_managed_view".to_owned(),
                table_name: "external_managed_view".to_owned(),
            },
            exact_sql:
                "CREATE VIEW external_managed_view AS SELECT * FROM main.\"selection_samples\""
                    .to_owned(),
        });
        assert!(matches!(
            classify_database_half(&managed_reference, &references),
            Err(GlobalSchemaCatalogError::SqlScanFailure { .. })
        ));

        let mut malformed = snapshot_for_state(&references, DatabaseHalfState::PreAmendment);
        malformed
            .objects
            .first_mut()
            .expect("legacy object")
            .exact_sql
            .push_str(" /* unterminated");
        assert!(matches!(
            classify_database_half(&malformed, &references),
            Err(GlobalSchemaCatalogError::SqlScanFailure { .. })
        ));

        let managed = FINAL_SELECTION_TABLES
            .iter()
            .map(|table| (*table).to_owned())
            .collect::<BTreeSet<_>>();
        scan_external_sql_for_managed_references(
            "hex-literal",
            "CREATE VIEW v AS SELECT X'73656c656374696f6e5f73616d706c6573'",
            &managed,
        )
        .expect("hex blob content is never an identifier");
    }

    #[test]
    fn selection_and_whole_application_digests_are_typed_and_domain_separated() {
        let production = same_runtime_references(GlobalSchemaCatalogMode::Production);
        let test = same_runtime_references(GlobalSchemaCatalogMode::Test);
        let production_evidence = match classify_database_half(
            &snapshot_for_state(&production, DatabaseHalfState::Amended),
            &production,
        )
        .expect("production amended diagnostic")
        {
            DatabaseHalfDiagnostic::AmendedDatabaseHalf(evidence) => evidence,
            other => panic!("unexpected state: {other:?}"),
        };
        let test_evidence = match classify_database_half(
            &snapshot_for_state(&test, DatabaseHalfState::Amended),
            &test,
        )
        .expect("test amended diagnostic")
        {
            DatabaseHalfDiagnostic::AmendedDatabaseHalf(evidence) => evidence,
            other => panic!("unexpected state: {other:?}"),
        };
        let selection = production_evidence
            .selection_managed_catalog_sha256
            .as_ref()
            .expect("amended selection digest");
        assert_eq!(selection.as_str().len(), 64);
        assert_eq!(
            production_evidence
                .whole_application_catalog_sha256
                .as_str()
                .len(),
            64
        );
        assert_ne!(
            selection.as_str(),
            production_evidence
                .whole_application_catalog_sha256
                .as_str()
        );
        assert_ne!(
            selection.as_str(),
            test_evidence
                .selection_managed_catalog_sha256
                .as_ref()
                .expect("test selection digest")
                .as_str()
        );

        let pre = match classify_database_half(
            &snapshot_for_state(&production, DatabaseHalfState::PreAmendment),
            &production,
        )
        .expect("pre-amendment diagnostic")
        {
            DatabaseHalfDiagnostic::PreAmendment(evidence) => evidence,
            other => panic!("unexpected state: {other:?}"),
        };
        assert!(pre.selection_managed_catalog_sha256.is_none());
    }

    #[test]
    fn reference_issuance_requires_every_frozen_ddl_id() {
        let mut built = build_owner_catalogs_from_linked_sqlite(GlobalSchemaCatalogMode::Test)
            .expect("build real same-runtime owner catalogs");
        built
            .amended
            .executed_ddl_ids
            .remove("legacy-v1.table.ledger");
        assert!(matches!(
            issue_same_runtime_catalog_references(built),
            Err(GlobalSchemaCatalogError::ReferenceDdlRegistryMismatch {
                catalog: "amended",
                ..
            })
        ));
    }

    fn same_runtime_references(mode: GlobalSchemaCatalogMode) -> SameRuntimeCatalogReferences {
        build_same_runtime_catalog_references(mode)
            .expect("private owner issues real exact same-runtime references")
    }

    fn snapshot_for_state(
        references: &SameRuntimeCatalogReferences,
        state: DatabaseHalfState,
    ) -> CatalogSnapshot {
        let (reference, identity, payloads) = match state {
            DatabaseHalfState::PreAmendment => (
                &references.legacy,
                DatabaseSchemaIdentity {
                    application_id: 0,
                    user_version: 0,
                },
                Vec::new(),
            ),
            DatabaseHalfState::Transitional => (
                &references.transitional,
                DatabaseSchemaIdentity {
                    application_id: 0,
                    user_version: 0,
                },
                TRANSITIONAL_SELECTION_PAYLOAD_SCHEMAS
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            ),
            DatabaseHalfState::Amended => (
                &references.amended,
                DatabaseSchemaIdentity {
                    application_id: STOCK_ANALYSIS_SQLITE_APPLICATION_ID,
                    user_version: STOCK_ANALYSIS_DB_SCHEMA_GENERATION,
                },
                FINAL_SELECTION_PAYLOAD_SCHEMAS
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            ),
        };
        CatalogSnapshot {
            mode: references.mode,
            identity,
            runtime: references.runtime.clone(),
            objects: reference.objects.clone(),
            managed_index_geometry: reference.managed_index_geometry.clone(),
            foreign_keys: reference.foreign_keys.clone(),
            sqlite_owned_objects: reference.sqlite_owned_objects.clone(),
            attached_schema_names: vec!["main".to_owned()],
            legacy_row_counts: legacy_row_counts(0),
            selection_row_counts: FINAL_SELECTION_TABLES
                .iter()
                .map(|table| ((*table).to_owned(), 0))
                .collect(),
            selection_payload_schemas: payloads,
        }
    }

    #[test]
    fn attached_schema_gate_allows_only_main_and_builtin_temp() {
        validate_attached_schema_names(&["main".to_owned()]).expect("main is mandatory");
        validate_attached_schema_names(&["main".to_owned(), "temp".to_owned()])
            .expect("SQLite may materialize its connection-local temp schema");

        for rejected in [
            Vec::<String>::new(),
            vec!["temp".to_owned()],
            vec!["main".to_owned(), "attached".to_owned()],
            vec!["main".to_owned(), "main".to_owned()],
            vec!["temp".to_owned(), "main".to_owned()],
        ] {
            assert!(matches!(
                validate_attached_schema_names(&rejected),
                Err(GlobalSchemaCatalogError::AttachedSchemaMismatch { .. })
            ));
        }
    }

    fn legacy_row_counts(value: i64) -> BTreeMap<String, i64> {
        legacy_catalog_registry_v1()
            .expect("legacy registry")
            .into_iter()
            .filter(|row| row.kind == CatalogObjectKind::Table)
            .map(|row| (row.name, value))
            .collect()
    }
}
