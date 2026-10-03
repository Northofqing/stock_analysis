//! Bounded Diesel transport into the sole Global catalog classifier.
//! This module constructs no live connection or schema authority.
use super::*;
use crate::database::global_schema_v1::paper_v6::{CatalogLoan, PaperCatalog6Error};
use diesel::sql_types::{BigInt, Nullable, Text};
use diesel::{sql_query, QueryableByName, RunQueryDsl, SqliteConnection};

const FIELD_LIMIT: usize = 64 * 1024;
#[derive(Debug)]
pub(in crate::database) struct CopyWork {
    remaining: usize,
}
impl CopyWork {
    pub(in crate::database) fn new() -> Self {
        Self {
            remaining: 32 * 1024 * 1024,
        }
    }
    pub(in crate::database) fn charge(&mut self, bytes: usize) -> Result<(), PaperCatalog6Error> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        Ok(())
    }
    #[cfg(test)]
    pub(in crate::database) fn remaining_for_test(&self) -> usize {
        self.remaining
    }
    #[cfg(test)]
    pub(in crate::database) fn limited(bytes: usize) -> Self {
        Self { remaining: bytes }
    }
}
#[derive(QueryableByName)]
struct Extent {
    #[diesel(sql_type=BigInt)]
    n: i64,
    #[diesel(sql_type=BigInt)]
    bytes: i64,
    #[diesel(sql_type=BigInt)]
    bad: i64,
}
#[derive(QueryableByName)]
pub(in crate::database) struct IntRow {
    #[diesel(sql_type=BigInt)]
    pub(in crate::database) value: i64,
}
#[derive(QueryableByName)]
struct StringRow {
    #[diesel(sql_type=Text)]
    value: String,
}
#[derive(Debug, PartialEq, Eq, QueryableByName)]
pub(in crate::database) struct Object {
    #[diesel(sql_type=Text)]
    kind: String,
    #[diesel(sql_type=Text)]
    name: String,
    #[diesel(sql_type=Text)]
    tbl_name: String,
    #[diesel(sql_type=Nullable<Text>)]
    sql: Option<String>,
}
#[derive(QueryableByName)]
struct FkRow {
    #[diesel(sql_type=BigInt)]
    id: i64,
    #[diesel(sql_type=BigInt)]
    seq: i64,
    #[diesel(sql_type=Text)]
    target: String,
}
#[derive(QueryableByName)]
struct IndexRow {
    #[diesel(sql_type=Text)]
    name: String,
    #[diesel(sql_type=BigInt)]
    unique_value: i64,
    #[diesel(sql_type=Text)]
    origin: String,
    #[diesel(sql_type=BigInt)]
    partial: i64,
}
#[derive(QueryableByName)]
struct TermRow {
    #[diesel(sql_type=BigInt)]
    seqno: i64,
    #[diesel(sql_type=BigInt)]
    cid: i64,
    #[diesel(sql_type=Nullable<Text>)]
    name: Option<String>,
    #[diesel(sql_type=BigInt)]
    descending: i64,
    #[diesel(sql_type=Nullable<Text>)]
    coll: Option<String>,
    #[diesel(sql_type=BigInt)]
    key_value: i64,
}

fn sql_error(_: diesel::result::Error) -> PaperCatalog6Error {
    PaperCatalog6Error::Sql
}
fn preflight(
    conn: &mut SqliteConnection,
    work: &mut CopyWork,
    relation: &str,
    fields: &[(&str, bool)],
    max_rows: usize,
    max_bytes: usize,
    stride: usize,
) -> Result<(usize, usize), PaperCatalog6Error> {
    let bytes = fields
        .iter()
        .map(|(f, _)| format!("COALESCE(length(CAST({f} AS BLOB)),0)"))
        .collect::<Vec<_>>()
        .join("+");
    let bad = fields
        .iter()
        .map(|(f, nullable)| {
            format!(
                "(typeof({f}) NOT IN ({}) OR COALESCE(length(CAST({f} AS BLOB)),0)>{FIELD_LIMIT})",
                if *nullable { "'text','null'" } else { "'text'" }
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    let extent=sql_query(format!("SELECT COUNT(*) AS n, COALESCE(SUM({bytes}),0) AS bytes, COALESCE(SUM(CASE WHEN {bad} THEN 1 ELSE 0 END),0) AS bad FROM ({relation})")).get_result::<Extent>(conn).map_err(sql_error)?;
    let n = usize::try_from(extent.n).map_err(|_| PaperCatalog6Error::CopyBudgetExceeded)?;
    let bytes =
        usize::try_from(extent.bytes).map_err(|_| PaperCatalog6Error::CopyBudgetExceeded)?;
    if extent.bad != 0 || n > max_rows || bytes > max_bytes {
        return Err(PaperCatalog6Error::CopyBudgetExceeded);
    }
    // Conservative descriptors and actual copied text. This is a copy-work
    // bound, not a claim about the allocator peak of old financial replay.
    let copied = bytes
        .checked_add(
            n.checked_mul(stride)
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?,
        )
        .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
    work.charge(copied)?;
    Ok((n, copied))
}
pub(in crate::database) fn int(
    conn: &mut SqliteConnection,
    sql: &str,
) -> Result<i64, PaperCatalog6Error> {
    sql_query(sql)
        .get_result::<IntRow>(conn)
        .map(|r| r.value)
        .map_err(sql_error)
}
pub(in crate::database) fn preflight_pair(
    conn: &mut SqliteConnection,
    work: &mut CopyWork,
) -> Result<(), PaperCatalog6Error> {
    // This scalar gate precedes either catalog collection. No fields are
    // copied here; main and TEMP share one extent, then each actual copy is
    // charged by objects(). The fixed scalar result is also charged.
    work.charge(64)?;
    let relation="SELECT type,name,tbl_name,sql FROM main.sqlite_schema WHERE type IN ('table','index','trigger','view') UNION ALL SELECT type,name,tbl_name,sql FROM temp.sqlite_schema WHERE type IN ('table','index','trigger','view')";
    let query=format!("SELECT COUNT(*) AS n,COALESCE(SUM(length(CAST(type AS BLOB))+length(CAST(name AS BLOB))+length(CAST(tbl_name AS BLOB))+COALESCE(length(CAST(sql AS BLOB)),0)),0) AS bytes,COALESCE(SUM(CASE WHEN typeof(type)!='text' OR typeof(name)!='text' OR typeof(tbl_name)!='text' OR typeof(sql) NOT IN ('text','null') OR length(CAST(type AS BLOB))>{FIELD_LIMIT} OR length(CAST(name AS BLOB))>{FIELD_LIMIT} OR length(CAST(tbl_name AS BLOB))>{FIELD_LIMIT} OR COALESCE(length(CAST(sql AS BLOB)),0)>{FIELD_LIMIT} THEN 1 ELSE 0 END),0) AS bad FROM ({relation})");
    let e = sql_query(query)
        .get_result::<Extent>(conn)
        .map_err(sql_error)?;
    if e.n < 0 || e.n > 4096 || e.bytes < 0 || e.bytes > 16 * 1024 * 1024 || e.bad != 0 {
        return Err(PaperCatalog6Error::CopyBudgetExceeded);
    }
    Ok(())
}
pub(in crate::database) fn objects(
    conn: &mut SqliteConnection,
    work: &mut CopyWork,
    schema: &str,
) -> Result<Vec<Object>, PaperCatalog6Error> {
    assert!(schema == "main" || schema == "temp");
    let relation=format!("SELECT type AS kind,name,tbl_name,sql FROM {schema}.sqlite_schema WHERE type IN ('table','index','trigger','view')");
    preflight(
        conn,
        work,
        &relation,
        &[
            ("kind", false),
            ("name", false),
            ("tbl_name", false),
            ("sql", true),
        ],
        if schema == "main" { 4092 } else { 4096 },
        16 * 1024 * 1024,
        128,
    )?;
    sql_query(format!("{relation} ORDER BY kind,name,tbl_name"))
        .load(conn)
        .map_err(sql_error)
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::database) struct TempCatalog {
    objects: Vec<Object>,
    geometry: Vec<ManagedIndexGeometry>,
}
pub(in crate::database) fn temporary_catalog(
    loan: &CatalogLoan<'_>,
    conn: &mut SqliteConnection,
    work: &mut CopyWork,
) -> Result<TempCatalog, PaperCatalog6Error> {
    loan.require_instance(conn)?;
    let result = temporary_body(conn, work)?;
    loan.require_instance(conn)?;
    Ok(result)
}
/// Low-authority reference transport only. This does not issue an actual loan,
/// registration, connection authority or VerifiedCatalog6.
pub(in crate::database) fn temporary_reference(
    conn: &mut SqliteConnection,
    work: &mut CopyWork,
) -> Result<TempCatalog, PaperCatalog6Error> {
    temporary_body(conn, work)
}
fn temporary_body(
    conn: &mut SqliteConnection,
    work: &mut CopyWork,
) -> Result<TempCatalog, PaperCatalog6Error> {
    let objects = objects(conn, work, "temp")?;
    let mut geometry = Vec::new();
    let mut count = 0usize;
    let mut bytes = 0usize;
    for object in objects.iter().filter(|r| r.kind == "table") {
        work.charge(
            object
                .name
                .len()
                .checked_mul(4)
                .and_then(|n| n.checked_add(256))
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?,
        )?;
        let relation=format!("SELECT name,\"unique\" AS unique_value,origin,partial FROM pragma_index_list({},'temp')",literal(&object.name));
        let (n, copied) = preflight(
            conn,
            work,
            &relation,
            &[("name", false), ("origin", false)],
            4096 - count,
            2 * 1024 * 1024 - bytes,
            80,
        )?;
        let names = n
            .checked_mul(object.name.len())
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        work.charge(names)?;
        count += n;
        bytes = bytes
            .checked_add(copied)
            .and_then(|n| n.checked_add(names))
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        if bytes > 2 * 1024 * 1024 {
            return Err(PaperCatalog6Error::CopyBudgetExceeded);
        }
        for index in sql_query(&relation)
            .load::<IndexRow>(conn)
            .map_err(sql_error)?
        {
            if !matches!(index.unique_value, 0 | 1) || !matches!(index.partial, 0 | 1) {
                return Err(PaperCatalog6Error::Sql);
            }
            work.charge(
                index
                    .name
                    .len()
                    .checked_mul(4)
                    .and_then(|n| n.checked_add(256))
                    .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?,
            )?;
            let relation=format!("SELECT seqno,cid,name,\"desc\" AS descending,coll,\"key\" AS key_value FROM pragma_index_xinfo({},'temp')",literal(&index.name));
            let (n, copied) = preflight(
                conn,
                work,
                &relation,
                &[("name", true), ("coll", true)],
                4096 - count,
                2 * 1024 * 1024 - bytes,
                96,
            )?;
            count += n;
            bytes = bytes
                .checked_add(copied)
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
            if bytes > 2 * 1024 * 1024 {
                return Err(PaperCatalog6Error::CopyBudgetExceeded);
            }
            let mut terms = Vec::new();
            for term in sql_query(format!("{relation} ORDER BY seqno"))
                .load::<TermRow>(conn)
                .map_err(sql_error)?
            {
                if !matches!(term.descending, 0 | 1) || !matches!(term.key_value, 0 | 1) {
                    return Err(PaperCatalog6Error::Sql);
                }
                terms.push(IndexXinfoTerm {
                    seqno: term.seqno,
                    cid: term.cid,
                    name: term.name,
                    descending: term.descending == 1,
                    collation: term.coll,
                    key: term.key_value == 1,
                });
            }
            geometry.push(ManagedIndexGeometry {
                table_name: object.name.clone(),
                index_name: index.name,
                unique: index.unique_value == 1,
                origin: index.origin,
                partial: index.partial == 1,
                terms,
            });
        }
    }
    geometry.sort();
    Ok(TempCatalog { objects, geometry })
}
fn strings(
    conn: &mut SqliteConnection,
    work: &mut CopyWork,
    relation: &str,
    rows: usize,
    bytes: usize,
) -> Result<Vec<String>, PaperCatalog6Error> {
    preflight(conn, work, relation, &[("value", false)], rows, bytes, 32)?;
    sql_query(format!("SELECT value FROM ({relation}) ORDER BY value"))
        .load::<StringRow>(conn)
        .map(|v| v.into_iter().map(|r| r.value).collect())
        .map_err(sql_error)
}
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn catalog_error(e: GlobalSchemaCatalogError) -> PaperCatalog6Error {
    PaperCatalog6Error::Catalog(e.to_string())
}
fn tables(mode: GlobalSchemaCatalogMode) -> Result<BTreeSet<String>, PaperCatalog6Error> {
    Ok(final_selection_catalog_registry_v1(mode)
        .map_err(catalog_error)?
        .into_iter()
        .filter(|r| r.kind == CatalogObjectKind::Table)
        .map(|r| r.name)
        .chain(
            super::super::paper_ledger_schema_v1::STATEMENTS
                .iter()
                .chain(super::super::daily_change_review_schema_v1::STATEMENTS.iter())
                .chain(super::super::paper_book_v2_schema::STATEMENTS.iter())
                .chain(super::super::paper_book_owner_schema_v1::STATEMENTS.iter())
                .chain(super::super::paper_book_owner_schema_v2::OWNER_STATEMENTS.iter())
                .chain(super::super::paper_book_v2_ledger_schema_v1::STATEMENTS.iter())
                .chain(super::super::paper_book_v2_execution_schema_v1::STATEMENTS.iter())
                .chain(super::super::candidate_scope_observation_schema_v1::STATEMENTS.iter())
                .chain(super::super::investment_decision_schema_v1::STATEMENTS.iter())
                .filter(|(k, _, _, _)| *k == "table")
                .map(|(_, n, _, _)| (*n).to_owned()),
        )
        .collect())
}
pub(in crate::database) fn capture(
    loan: &CatalogLoan<'_>,
    conn: &mut SqliteConnection,
    mode: GlobalSchemaCatalogMode,
    work: &mut CopyWork,
) -> Result<CatalogSnapshot, PaperCatalog6Error> {
    loan.require_instance(conn)?; // before the first scalar query or copy
    let identity = DatabaseSchemaIdentity {
        application_id: int(
            conn,
            "SELECT application_id AS value FROM pragma_application_id",
        )?,
        user_version: int(
            conn,
            "SELECT user_version AS value FROM pragma_user_version",
        )?,
    };
    let source_id = strings(
        conn,
        work,
        "SELECT sqlite_source_id() AS value",
        1,
        FIELD_LIMIT,
    )?
    .pop()
    .ok_or(PaperCatalog6Error::Sql)?;
    let version = strings(
        conn,
        work,
        "SELECT sqlite_version() AS value",
        1,
        FIELD_LIMIT,
    )?
    .pop()
    .ok_or(PaperCatalog6Error::Sql)?;
    let parts = version
        .split('.')
        .map(str::parse::<i32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| PaperCatalog6Error::Sql)?;
    if parts.len() != 3 {
        return Err(PaperCatalog6Error::Sql);
    }
    let compile = strings(
        conn,
        work,
        "SELECT compile_options AS value FROM pragma_compile_options",
        512,
        128 * 1024,
    )?;
    if compile.windows(2).any(|w| w[0] == w[1]) {
        return Err(PaperCatalog6Error::Catalog(
            "duplicate compile option".into(),
        ));
    }
    let mut hash = Sha256::new();
    hash_field(&mut hash, b"stock_analysis.br180.sqlite_compile_options.v1");
    hash.update((compile.len() as u64).to_be_bytes());
    for item in compile {
        hash_field(&mut hash, item.as_bytes());
    }
    let runtime = SqliteRuntimeIdentity {
        libversion_number: parts[0] * 1_000_000 + parts[1] * 1000 + parts[2],
        source_id,
        compile_options_sha256: lower_hex(&hash.finalize()),
    };
    validate_runtime_identity(&runtime).map_err(catalog_error)?;
    let mut named = Vec::new();
    let mut owned = Vec::new();
    for row in objects(conn, work, "main")? {
        let kind = parse_catalog_object_kind("diesel-capture", &row.kind).map_err(catalog_error)?;
        if row.name.to_ascii_lowercase().starts_with("sqlite_") {
            owned.push(SqliteOwnedCatalogObject {
                kind,
                name: row.name,
                table_name: row.tbl_name,
                exact_sql: row.sql,
            });
        } else {
            named.push(CatalogObjectRow {
                identity: CatalogObjectIdentity {
                    kind,
                    name: row.name,
                    table_name: row.tbl_name,
                },
                exact_sql: row.sql.ok_or_else(|| {
                    PaperCatalog6Error::Catalog("application object has no SQL".into())
                })?,
            });
        }
    }
    named.sort_by(|a, b| a.identity.cmp(&b.identity));
    owned.sort();
    let table_copy = named
        .iter()
        .filter(|r| r.identity.kind == CatalogObjectKind::Table)
        .try_fold(0usize, |n, r| n.checked_add(r.identity.name.len() + 64))
        .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
    work.charge(table_copy)?;
    let actual_tables = named
        .iter()
        .filter(|r| r.identity.kind == CatalogObjectKind::Table)
        .map(|r| r.identity.name.clone())
        .collect::<BTreeSet<_>>();
    let mut foreign_keys = Vec::new();
    let mut fk_count = 0usize;
    let mut fk_bytes = 0usize;
    for table in &actual_tables {
        work.charge(
            table
                .len()
                .checked_mul(4)
                .and_then(|n| n.checked_add(256))
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?,
        )?;
        let relation = format!(
            "SELECT id,seq,\"table\" AS target FROM pragma_foreign_key_list({})",
            literal(table)
        );
        let (count, copied) = preflight(
            conn,
            work,
            &relation,
            &[("target", false)],
            4096 - fk_count,
            2 * 1024 * 1024 - fk_bytes,
            64,
        )?;
        let names = count
            .checked_mul(table.len())
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        work.charge(names)?;
        fk_count += count;
        fk_bytes = fk_bytes
            .checked_add(copied)
            .and_then(|n| n.checked_add(names))
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        if fk_bytes > 2 * 1024 * 1024 {
            return Err(PaperCatalog6Error::CopyBudgetExceeded);
        }
        let rows = sql_query(format!("{relation} ORDER BY id,seq"))
            .load::<FkRow>(conn)
            .map_err(sql_error)?;
        for row in rows {
            foreign_keys.push(ForeignKeyDependency {
                source_table: table.clone(),
                id: row.id,
                sequence: row.seq,
                target_table: row.target,
            });
        }
    }
    foreign_keys.sort();
    let mut geometry = Vec::new();
    let mut geometry_count = 0usize;
    let mut geometry_bytes = 0usize;
    for table in tables(mode)?.intersection(&actual_tables) {
        work.charge(
            table
                .len()
                .checked_mul(4)
                .and_then(|n| n.checked_add(256))
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?,
        )?;
        let relation = format!(
            "SELECT name,\"unique\" AS unique_value,origin,partial FROM pragma_index_list({})",
            literal(table)
        );
        let (count, copied) = preflight(
            conn,
            work,
            &relation,
            &[("name", false), ("origin", false)],
            4096 - geometry_count,
            2 * 1024 * 1024 - geometry_bytes,
            80,
        )?;
        let names = count
            .checked_mul(table.len())
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        work.charge(names)?;
        geometry_count += count;
        geometry_bytes = geometry_bytes
            .checked_add(copied)
            .and_then(|n| n.checked_add(names))
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        if geometry_bytes > 2 * 1024 * 1024 {
            return Err(PaperCatalog6Error::CopyBudgetExceeded);
        }
        for index in sql_query(&relation)
            .load::<IndexRow>(conn)
            .map_err(sql_error)?
        {
            if !matches!(index.unique_value, 0 | 1) || !matches!(index.partial, 0 | 1) {
                return Err(PaperCatalog6Error::Sql);
            }
            let relation=format!("SELECT seqno,cid,name,\"desc\" AS descending,coll,\"key\" AS key_value FROM pragma_index_xinfo({})",literal(&index.name));
            let (count, copied) = preflight(
                conn,
                work,
                &relation,
                &[("name", true), ("coll", true)],
                4096 - geometry_count,
                2 * 1024 * 1024 - geometry_bytes,
                96,
            )?;
            geometry_count += count;
            geometry_bytes = geometry_bytes
                .checked_add(copied)
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
            if geometry_bytes > 2 * 1024 * 1024 {
                return Err(PaperCatalog6Error::CopyBudgetExceeded);
            }
            let mut terms = Vec::new();
            for term in sql_query(format!("{relation} ORDER BY seqno"))
                .load::<TermRow>(conn)
                .map_err(sql_error)?
            {
                if !matches!(term.descending, 0 | 1) || !matches!(term.key_value, 0 | 1) {
                    return Err(PaperCatalog6Error::Sql);
                }
                terms.push(IndexXinfoTerm {
                    seqno: term.seqno,
                    cid: term.cid,
                    name: term.name,
                    descending: term.descending == 1,
                    collation: term.coll,
                    key: term.key_value == 1,
                });
            }
            geometry.push(ManagedIndexGeometry {
                table_name: table.clone(),
                index_name: index.name,
                unique: index.unique_value == 1,
                origin: index.origin,
                partial: index.partial == 1,
                terms,
            });
        }
    }
    geometry.sort();
    let attached_schema_names = strings(
        conn,
        work,
        "SELECT name AS value FROM pragma_database_list",
        2,
        8 * 1024,
    )?
    .into_iter()
    .filter(|s| s != "temp")
    .collect();
    let legacy = legacy_table_name_set().map_err(catalog_error)?;
    let mut counts =
        |registry: BTreeSet<String>| -> Result<BTreeMap<String, i64>, PaperCatalog6Error> {
            registry
                .into_iter()
                .map(|table| {
                    let value = if actual_tables.contains(&table) {
                        int(
                            conn,
                            &format!("SELECT COUNT(*) AS value FROM {}", quoted(&table)),
                        )?
                    } else {
                        0
                    };
                    if value < 0 {
                        return Err(PaperCatalog6Error::Sql);
                    }
                    work.charge(table.len() + 48)?;
                    Ok((table, value))
                })
                .collect()
        };
    let legacy_row_counts = counts(legacy)?;
    let selection_row_counts =
        counts(FINAL_SELECTION_TABLES.iter().map(|s| (*s).into()).collect())?;
    let selection_payload_schemas = capture_selection_payload_schema_contract(&named);
    work.charge(selection_payload_schemas.iter().map(|s| s.len() + 32).sum())?;
    loan.require_instance(conn)?;
    Ok(CatalogSnapshot {
        mode,
        identity,
        runtime,
        objects: named,
        managed_index_geometry: geometry,
        foreign_keys,
        sqlite_owned_objects: owned,
        attached_schema_names,
        legacy_row_counts,
        selection_row_counts,
        selection_payload_schemas,
    })
}

fn checked_sum(mut values: impl Iterator<Item = usize>) -> Result<usize, PaperCatalog6Error> {
    values
        .try_fold(0usize, |n, v| n.checked_add(v))
        .ok_or(PaperCatalog6Error::CopyBudgetExceeded)
}
fn object_copies(rows: &[CatalogObjectRow]) -> Result<usize, PaperCatalog6Error> {
    rows.iter().try_fold(0usize, |n, r| {
        n.checked_add(object_copy(r)?)
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)
    })
}
fn object_copy(row: &CatalogObjectRow) -> Result<usize, PaperCatalog6Error> {
    checked_sum(
        [
            row.identity.name.len(),
            row.identity.table_name.len(),
            row.exact_sql.len(),
            128,
        ]
        .into_iter(),
    )
}
fn identity_copy(row: &CatalogObjectRow) -> Result<usize, PaperCatalog6Error> {
    checked_sum([row.identity.name.len(), row.identity.table_name.len(), 128].into_iter())
}
fn geometry_copy(rows: &[ManagedIndexGeometry]) -> Result<usize, PaperCatalog6Error> {
    let mut n = 0usize;
    for row in rows {
        n = n
            .checked_add(checked_sum(
                [
                    row.table_name.len(),
                    row.index_name.len(),
                    row.origin.len(),
                    128,
                ]
                .into_iter(),
            )?)
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        for t in &row.terms {
            n = n
                .checked_add(checked_sum(
                    [
                        t.name.as_ref().map_or(0, String::len),
                        t.collation.as_ref().map_or(0, String::len),
                        96,
                    ]
                    .into_iter(),
                )?)
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        }
    }
    Ok(n)
}
fn foreign_copy(rows: &[ForeignKeyDependency]) -> Result<usize, PaperCatalog6Error> {
    checked_sum(
        rows.iter()
            .map(|r| r.source_table.len() + r.target_table.len() + 96),
    )
}
fn owned_copy(rows: &[SqliteOwnedCatalogObject]) -> Result<usize, PaperCatalog6Error> {
    checked_sum(rows.iter().map(|r| {
        r.name.len() + r.table_name.len() + r.exact_sql.as_ref().map_or(0, String::len) + 128
    }))
}
/// Reserve every known owned-copy/lexer path in the unchanged sole classifier
/// before calling it. This is conservative copy work, not allocator peak RAM.
fn classifier_copy_work(
    snapshot: &CatalogSnapshot,
    references: &SameRuntimeCatalogReferences,
) -> Result<usize, PaperCatalog6Error> {
    let state = match snapshot.identity.user_version {
        5 => &references.owner_v5,
        6 => &references.execution_v6,
        7 => &references.candidate_v7,
        8 => &references.investment_v8,
        _ => return Err(PaperCatalog6Error::Catalog6RequalificationRequired),
    };
    let mut total = 0usize;
    let mut add = |n: usize| {
        total = total
            .checked_add(n)
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?;
        Ok::<_, PaperCatalog6Error>(())
    };
    // The four initial sorted catalogs.
    for rows in [
        &snapshot.objects,
        &state.legacy.objects,
        &state.transitional.objects,
        &state.amended.objects,
    ] {
        add(object_copies(rows)?)?;
    }
    // Whole digest clones the actual catalog once more.
    add(object_copies(&snapshot.objects)?)?;
    // Selection member clone + its digest clone, and the identity set. Use
    // only borrowed identities from the original issued generation-1 refs.
    for row in &snapshot.objects {
        let selected = references
            .amended
            .objects
            .iter()
            .any(|r| r.identity == row.identity)
            && !references
                .legacy
                .objects
                .iter()
                .any(|r| r.identity == row.identity);
        add(identity_copy(row)?
            .checked_mul(2)
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?)?; // validation set/lowercase names
        if selected {
            add(object_copy(row)?
                .checked_mul(2)
                .and_then(|n| n.checked_add(identity_copy(row).ok()?))
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?)?;
        } else {
            add(row
                .exact_sql
                .len()
                .checked_mul(6)
                .and_then(|n| n.checked_add(128))
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?)?;
        } // Vec<char> + all decoded/token strings
    }
    // Safety's selection filter/unique geometry and covered/index name sets,
    // followed by the ancillary canonical copy (five conservative copies).
    add(geometry_copy(&snapshot.managed_index_geometry)?
        .checked_mul(5)
        .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?)?;
    add(foreign_copy(&snapshot.foreign_keys)?)?;
    add(owned_copy(&snapshot.sqlite_owned_objects)?
        .checked_mul(3)
        .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?)?;
    let mut expected_max = 0;
    for r in [&state.legacy, &state.transitional, &state.amended] {
        expected_max = expected_max.max(checked_sum(
            [
                geometry_copy(&r.managed_index_geometry)?,
                foreign_copy(&r.foreign_keys)?,
                owned_copy(&r.sqlite_owned_objects)?,
            ]
            .into_iter(),
        )?);
    }
    add(expected_max)?;
    // Key sets, diagnostic evidence clones, payload sort/clone, runtime/hash
    // output and frozen expected-registry construction (no caller bytes).
    for (k, _) in &snapshot.legacy_row_counts {
        add((k.len() + 64)
            .checked_mul(3)
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?)?;
    }
    for k in snapshot.selection_row_counts.keys() {
        add(k.len() + 64)?;
    }
    for value in &snapshot.selection_payload_schemas {
        add((value.len() + 32)
            .checked_mul(3)
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?)?;
    }
    add(snapshot.runtime.source_id.len() + snapshot.runtime.compile_options_sha256.len() + 512)?;
    add(LEGACY_CATALOG_V1_FIXTURE
        .len()
        .checked_mul(8)
        .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?)?;
    // Two final-selection registry producers allocate identity, frozen entry,
    // unique and validation sets; upper-bound all such fixed identity copies.
    for r in &references.amended.objects {
        if !references
            .legacy
            .objects
            .iter()
            .any(|l| l.identity == r.identity)
        {
            add(identity_copy(r)?
                .checked_mul(12)
                .ok_or(PaperCatalog6Error::CopyBudgetExceeded)?)?;
        }
    }
    Ok(total)
}
pub(in crate::database) fn classify_bounded(
    snapshot: &CatalogSnapshot,
    references: &SameRuntimeCatalogReferences,
    work: &mut CopyWork,
) -> Result<DatabaseHalfDiagnostic, PaperCatalog6Error> {
    work.charge(classifier_copy_work(snapshot, references)?)?;
    classify_database_half(snapshot, references).map_err(catalog_error)
}
#[cfg(test)]
pub(in crate::database) fn ancillary_observation_counts_for_test(
    snapshot: &CatalogSnapshot,
) -> (usize, usize) {
    (
        snapshot.managed_index_geometry.len(),
        snapshot.sqlite_owned_objects.len(),
    )
}
#[cfg(test)]
pub(in crate::database) fn classifier_copy_work_for_test(
    snapshot: &CatalogSnapshot,
    references: &SameRuntimeCatalogReferences,
) -> Result<usize, PaperCatalog6Error> {
    classifier_copy_work(snapshot, references)
}
#[cfg(test)]
pub(in crate::database) fn initial_catalog_copy_work_for_test(
    snapshot: &CatalogSnapshot,
    references: &SameRuntimeCatalogReferences,
) -> Result<usize, PaperCatalog6Error> {
    let state = match snapshot.identity.user_version {
        7 => &references.candidate_v7,
        8 => &references.investment_v8,
        _ => &references.execution_v6,
    };
    [
        &snapshot.objects,
        &state.legacy.objects,
        &state.transitional.objects,
        &state.amended.objects,
    ]
    .into_iter()
    .try_fold(0usize, |n, rows| {
        n.checked_add(object_copies(rows)?)
            .ok_or(PaperCatalog6Error::CopyBudgetExceeded)
    })
}
/// Creates a real TEMP autoindex collation drift while restoring identical
/// sqlite_schema text. Test-only: no registration or catalog capability is issued.
#[cfg(test)]
pub(in crate::database) fn replace_temp_token_geometry_for_test(
    conn: &mut SqliteConnection,
) -> Result<(), PaperCatalog6Error> {
    use diesel::connection::SimpleConnection;
    let original = temporary_body(conn, &mut CopyWork::new())?;
    let table = crate::database::DESCRIPTOR_ATTESTATION_TEMP_TABLE;
    let token = crate::database::connection_attestation_token(conn)
        .map_err(|_| PaperCatalog6Error::Authority)?;
    let original_sql = original
        .objects
        .iter()
        .find(|r| r.kind == "table" && r.name == table)
        .and_then(|r| r.sql.as_ref())
        .ok_or(PaperCatalog6Error::Sql)?;
    conn.batch_execute(&format!("DROP TRIGGER temp.{}; DROP TRIGGER temp.{}; DROP TABLE temp.{}; CREATE TEMP TABLE {} (slot INTEGER NOT NULL PRIMARY KEY CHECK(slot=1), token TEXT COLLATE NOCASE NOT NULL UNIQUE)",
        quoted(crate::database::DESCRIPTOR_ATTESTATION_NO_UPDATE_TRIGGER),
        quoted(crate::database::DESCRIPTOR_ATTESTATION_NO_DELETE_TRIGGER), quoted(table), quoted(table)))?;
    sql_query(format!(
        "INSERT INTO temp.{}(slot,token) VALUES(1,?)",
        quoted(table)
    ))
    .bind::<Text, _>(&token)
    .execute(conn)?;
    for trigger in original.objects.iter().filter(|r| r.kind == "trigger") {
        let sql = trigger.sql.as_ref().ok_or(PaperCatalog6Error::Sql)?;
        conn.batch_execute(&sql.replacen("CREATE TRIGGER", "CREATE TEMP TRIGGER", 1))?;
    }
    conn.batch_execute("PRAGMA writable_schema=ON")?;
    assert_eq!(
        int(
            conn,
            "SELECT writable_schema AS value FROM pragma_writable_schema"
        )?,
        1,
        "the isolated geometry attack must actually enable schema writes"
    );
    let changed_sql =
        sql_query("UPDATE temp.sqlite_schema SET sql=? WHERE type='table' AND name=?")
            .bind::<Text, _>(original_sql)
            .bind::<Text, _>(table)
            .execute(conn);
    let restored = conn.batch_execute("PRAGMA writable_schema=OFF");
    changed_sql?;
    restored?;
    assert_eq!(
        int(
            conn,
            "SELECT writable_schema AS value FROM pragma_writable_schema"
        )?,
        0
    );
    let changed = temporary_body(conn, &mut CopyWork::new())?;
    assert_eq!(
        original.objects, changed.objects,
        "test must retain exact TEMP catalog SQL"
    );
    assert_ne!(
        original.geometry, changed.geometry,
        "test must change actual autoindex geometry"
    );
    assert!(changed
        .geometry
        .iter()
        .flat_map(|r| &r.terms)
        .any(|t| t.collation.as_deref() == Some("NOCASE")));
    Ok(())
}
