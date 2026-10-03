//! Staged fee-policy manifest for a future paper-book generation.
//!
//! This namespace has no account, order, fill, or active-owner tables. Only
//! isolated unit tests may install it. Verification is read-only and is not a
//! whole-application catalog or production cutover authority.

use crate::performance::fee_evidence::A_SHARE_FEE_SCHEDULE_V2;
use crate::performance::fee_policy::AShareFeePolicyV2;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary, Text};
use serde::Serialize;
use sha2::{Digest, Sha256};

const STAGED_SCHEMA: &str = "paper-book-v2-fee-manifest/v1";
const APPLICATION_ID: i64 = 1_398_035_265;
const TEST_PARENT_USER_VERSION: i64 = 2;

pub(crate) const STATEMENTS: &[(&str, &str, &str, &str)] = &[
    ("table", "paper_book_v2_fee_manifest", "paper_book_v2_fee_manifest", "CREATE TABLE paper_book_v2_fee_manifest (
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        schema_id TEXT NOT NULL CHECK(schema_id='paper-book-v2-fee-manifest/v1'),
        policy_instance_id TEXT NOT NULL,
        descriptor_sha256 TEXT NOT NULL CHECK(length(descriptor_sha256)=64),
        descriptor_bytes BLOB NOT NULL CHECK(length(descriptor_bytes)>0))"),
    ("trigger", "paper_book_v2_fee_manifest_no_update", "paper_book_v2_fee_manifest", "CREATE TRIGGER paper_book_v2_fee_manifest_no_update BEFORE UPDATE ON paper_book_v2_fee_manifest
        BEGIN SELECT RAISE(ABORT,'immutable V2 fee policy manifest'); END"),
    ("trigger", "paper_book_v2_fee_manifest_no_delete", "paper_book_v2_fee_manifest", "CREATE TRIGGER paper_book_v2_fee_manifest_no_delete BEFORE DELETE ON paper_book_v2_fee_manifest
        BEGIN SELECT RAISE(ABORT,'immutable V2 fee policy manifest'); END"),
    ("trigger", "paper_book_v2_fee_manifest_no_reinsert", "paper_book_v2_fee_manifest", "CREATE TRIGGER paper_book_v2_fee_manifest_no_reinsert BEFORE INSERT ON paper_book_v2_fee_manifest
        WHEN EXISTS(SELECT 1 FROM paper_book_v2_fee_manifest)
        BEGIN SELECT RAISE(ABORT,'immutable V2 fee policy manifest'); END"),
];

#[derive(Debug, thiserror::Error)]
pub(crate) enum StagedPaperBookV2Error {
    #[error("V2 staging requires the expected test database application ID and user version")]
    WrongDatabaseIdentity,
    #[error("V2 staged namespace is missing, altered, or has extra objects")]
    CatalogMismatch,
    #[error("V2 fee-policy manifest does not match the reviewed descriptor")]
    ManifestMismatch,
    #[error("V2 staged namespace already exists")]
    AlreadyStaged,
    #[cfg(test)]
    #[error("V2 staging requires an in-memory or TEST_CODE temporary database")]
    NotIsolated,
    #[error("V2 staged catalog encoding failed: {0}")]
    Encoding(#[from] serde_json::Error),
    #[error(transparent)]
    Database(#[from] diesel::result::Error),
    #[error(transparent)]
    Connection(#[from] diesel::ConnectionError),
    #[cfg(test)]
    #[error("TEST_CODE interrupted V2 staged installation")]
    InjectedFailure,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StagedPaperBookV2Catalog {
    catalog_sha256: String,
    policy_instance_id: String,
}

impl StagedPaperBookV2Catalog {
    pub(crate) fn catalog_sha256(&self) -> &str {
        &self.catalog_sha256
    }

    pub(crate) fn policy_instance_id(&self) -> &str {
        &self.policy_instance_id
    }
}

#[derive(QueryableByName, Serialize)]
struct DatabaseIdentity {
    #[diesel(sql_type = BigInt)]
    application_id: i64,
    #[diesel(sql_type = BigInt)]
    user_version: i64,
}

#[derive(Debug, Eq, PartialEq, QueryableByName, Serialize)]
struct CatalogObject {
    #[diesel(sql_type = Text)]
    namespace: String,
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    table_name: String,
    #[diesel(sql_type = Text)]
    sql: String,
}

#[derive(QueryableByName, Serialize)]
struct FeeManifestRow {
    #[diesel(sql_type = Text)]
    schema_id: String,
    #[diesel(sql_type = Text)]
    policy_instance_id: String,
    #[diesel(sql_type = Text)]
    descriptor_sha256: String,
    #[diesel(sql_type = Binary)]
    descriptor_bytes: Vec<u8>,
}

fn database_identity(
    conn: &mut SqliteConnection,
) -> Result<DatabaseIdentity, StagedPaperBookV2Error> {
    Ok(diesel::sql_query(
        "SELECT application_id,user_version FROM pragma_application_id(),pragma_user_version()",
    )
    .get_result(conn)?)
}

fn objects(conn: &mut SqliteConnection) -> Result<Vec<CatalogObject>, StagedPaperBookV2Error> {
    Ok(diesel::sql_query(
        "SELECT 'main' AS namespace,type AS kind,name,tbl_name AS table_name,sql
         FROM main.sqlite_master
         WHERE (lower(name) GLOB 'paper_book_v2_*' OR lower(tbl_name) GLOB 'paper_book_v2_*') AND sql IS NOT NULL
         UNION ALL
         SELECT 'temp' AS namespace,type AS kind,name,tbl_name AS table_name,sql
         FROM temp.sqlite_master
         WHERE (lower(name) GLOB 'paper_book_v2_*' OR lower(tbl_name) GLOB 'paper_book_v2_*') AND sql IS NOT NULL
         ORDER BY namespace,kind,name,table_name,sql",
    )
    .load(conn)?)
}

fn fee_objects(conn: &mut SqliteConnection) -> Result<Vec<CatalogObject>, StagedPaperBookV2Error> {
    Ok(diesel::sql_query(
        "SELECT 'main' AS namespace,type AS kind,name,tbl_name AS table_name,sql
         FROM main.sqlite_master
         WHERE (lower(name) GLOB 'paper_book_v2_fee_manifest*' OR lower(tbl_name)='paper_book_v2_fee_manifest') AND sql IS NOT NULL
         UNION ALL
         SELECT 'temp' AS namespace,type AS kind,name,tbl_name AS table_name,sql
         FROM temp.sqlite_master
         WHERE (lower(name) GLOB 'paper_book_v2_fee_manifest*' OR lower(tbl_name)='paper_book_v2_fee_manifest') AND sql IS NOT NULL
         ORDER BY namespace,kind,name,table_name,sql",
    )
    .load(conn)?)
}

pub(crate) fn create_schema(conn: &mut SqliteConnection) -> Result<(), StagedPaperBookV2Error> {
    for (_, _, _, statement) in STATEMENTS {
        diesel::sql_query(*statement).execute(conn)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn insert_policy_for_isolated_test(
    conn: &mut SqliteConnection,
    policy: &AShareFeePolicyV2,
) -> Result<(), StagedPaperBookV2Error> {
    diesel::sql_query(
        "INSERT INTO paper_book_v2_fee_manifest
         (singleton,schema_id,policy_instance_id,descriptor_sha256,descriptor_bytes)
         VALUES (1,?,?,?,?)",
    )
    .bind::<Text, _>(STAGED_SCHEMA)
    .bind::<Text, _>(policy.instance_id())
    .bind::<Text, _>(policy.descriptor_hash())
    .bind::<Binary, _>(policy.canonical_bytes())
    .execute(conn)?;
    Ok(())
}

/// A V4 historical-reader check only. A V2 writer must additionally bind a
/// reviewed policy descriptor; a self-consistent database row cannot grant it.
pub(crate) fn verify_v4_manifest_on(
    conn: &mut SqliteConnection,
) -> Result<(), StagedPaperBookV2Error> {
    verify_manifest_row_on(conn, 4)
}

/// CatalogV5 adds other paper_book_v2_* tables. Verify the frozen fee namespace
/// and row without treating those new tables as part of this policy manifest.
pub(crate) fn verify_v5_manifest_on(
    conn: &mut SqliteConnection,
) -> Result<(), StagedPaperBookV2Error> {
    verify_manifest_row_on(conn, 5)
}

/// CatalogV6 preserves the exact original fee namespace and descriptor.
/// This is not whole-catalog qualification or execution authority.
pub(crate) fn verify_v6_manifest_on(
    conn: &mut SqliteConnection,
) -> Result<(), StagedPaperBookV2Error> {
    verify_execution_manifest_types_on(conn)?;
    verify_manifest_row_on(conn, 6)
}

/// Fixed7 historical validation only; never an execution capability.
pub(crate) fn verify_v7_manifest_on(
    conn: &mut SqliteConnection,
) -> Result<(), StagedPaperBookV2Error> {
    verify_execution_manifest_types_on(conn)?;
    verify_manifest_row_on(conn, 7)
}

/// Fixed8 historical validation only; never execution authority.
pub(crate) fn verify_v8_manifest_on(
    conn: &mut SqliteConnection,
) -> Result<(), StagedPaperBookV2Error> {
    verify_execution_manifest_types_on(conn)?;
    verify_manifest_row_on(conn, 8)
}

fn verify_execution_manifest_types_on(
    conn: &mut SqliteConnection,
) -> Result<(), StagedPaperBookV2Error> {
    #[derive(QueryableByName)]
    struct InvalidRow {
        #[diesel(sql_type = BigInt)]
        value: i64,
    }
    let invalid = diesel::sql_query(
        "SELECT COUNT(*) AS value FROM paper_book_v2_fee_manifest WHERE
         typeof(singleton)!='integer' OR singleton!=1 OR typeof(schema_id)!='text'
         OR typeof(policy_instance_id)!='text' OR typeof(descriptor_sha256)!='text'
         OR typeof(descriptor_bytes)!='blob'",
    )
    .get_result::<InvalidRow>(conn)?
    .value;
    if invalid != 0 {
        return Err(StagedPaperBookV2Error::ManifestMismatch);
    }
    Ok(())
}

/// A complete inactive gen2 staging row may coexist with legacy V1 writes.
/// This is only a structural check; it never activates a V2 owner or fill.
pub(crate) fn verify_inactive_staged_manifest_on(
    conn: &mut SqliteConnection,
) -> Result<(), StagedPaperBookV2Error> {
    verify_manifest_row_on(conn, TEST_PARENT_USER_VERSION)
}

fn verify_manifest_row_on(
    conn: &mut SqliteConnection,
    expected_user_version: i64,
) -> Result<(), StagedPaperBookV2Error> {
    let identity = database_identity(conn)?;
    if identity.application_id != APPLICATION_ID || identity.user_version != expected_user_version {
        return Err(StagedPaperBookV2Error::WrongDatabaseIdentity);
    }
    let mut reference = SqliteConnection::establish(":memory:")?;
    create_schema(&mut reference)?;
    let matches_reference = if matches!(expected_user_version, 5 | 6 | 7 | 8) {
        fee_objects(conn)? == fee_objects(&mut reference)?
    } else {
        objects(conn)? == objects(&mut reference)?
    };
    if !matches_reference {
        return Err(StagedPaperBookV2Error::CatalogMismatch);
    }
    let rows: Vec<FeeManifestRow> = diesel::sql_query(
        "SELECT schema_id,policy_instance_id,descriptor_sha256,descriptor_bytes
         FROM paper_book_v2_fee_manifest ORDER BY singleton",
    )
    .load(conn)?;
    if rows.len() != 1 || rows[0].schema_id != STAGED_SCHEMA {
        return Err(StagedPaperBookV2Error::ManifestMismatch);
    }
    let row = &rows[0];
    let mut hasher = Sha256::new();
    hasher.update(b"a-share-fee-policy-descriptor/v1\n");
    hasher.update(&row.descriptor_bytes);
    let digest = hex::encode(hasher.finalize());
    if row.descriptor_sha256 != digest
        || row.policy_instance_id != format!("{A_SHARE_FEE_SCHEDULE_V2}:sha256:{digest}")
    {
        return Err(StagedPaperBookV2Error::ManifestMismatch);
    }
    Ok(())
}

#[cfg(test)]
fn require_isolated_test_database(
    conn: &mut SqliteConnection,
) -> Result<(), StagedPaperBookV2Error> {
    #[derive(QueryableByName)]
    struct MainFile {
        #[diesel(sql_type = Text)]
        file: String,
    }
    let path = diesel::sql_query("SELECT file FROM pragma_database_list() WHERE name='main'")
        .get_result::<MainFile>(conn)?
        .file;
    if path.is_empty() {
        return Ok(());
    }
    let path = std::path::Path::new(&path);
    let test_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("TEST_CODE_") && name.ends_with(".db"));
    let in_temp = path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .zip(std::env::temp_dir().canonicalize().ok())
        .is_some_and(|(parent, temp)| parent.starts_with(temp));
    if test_name && in_temp {
        Ok(())
    } else {
        Err(StagedPaperBookV2Error::NotIsolated)
    }
}

/// Read-only same-runtime proof of the staged namespace and one reviewed policy.
/// The caller must not use this receipt to authorize an account or fill.
pub(crate) fn verify_staged_policy(
    conn: &mut SqliteConnection,
    policy: &AShareFeePolicyV2,
) -> Result<StagedPaperBookV2Catalog, StagedPaperBookV2Error> {
    let identity = database_identity(conn)?;
    if identity.application_id != APPLICATION_ID
        || identity.user_version != TEST_PARENT_USER_VERSION
    {
        return Err(StagedPaperBookV2Error::WrongDatabaseIdentity);
    }
    let actual = objects(conn)?;
    let mut reference = SqliteConnection::establish(":memory:")?;
    create_schema(&mut reference)?;
    if actual != objects(&mut reference)? {
        return Err(StagedPaperBookV2Error::CatalogMismatch);
    }
    let rows: Vec<FeeManifestRow> = diesel::sql_query(
        "SELECT schema_id,policy_instance_id,descriptor_sha256,descriptor_bytes
         FROM paper_book_v2_fee_manifest ORDER BY singleton",
    )
    .load(conn)?;
    if rows.len() != 1
        || rows[0].schema_id != STAGED_SCHEMA
        || rows[0].policy_instance_id != policy.instance_id()
        || rows[0].descriptor_sha256 != policy.descriptor_hash()
        || rows[0].descriptor_bytes != policy.canonical_bytes()
    {
        return Err(StagedPaperBookV2Error::ManifestMismatch);
    }
    let canonical = serde_json::to_vec(&(STAGED_SCHEMA, &identity, &actual, &rows))?;
    let mut hasher = Sha256::new();
    hasher.update(b"paper-book-v2-staged-catalog/v1\0");
    hasher.update(canonical);
    Ok(StagedPaperBookV2Catalog {
        catalog_sha256: hex::encode(hasher.finalize()),
        policy_instance_id: rows[0].policy_instance_id.clone(),
    })
}

/// Install only in an isolated test database. No production creation entry is
/// compiled until the global catalog and owner fence have their own authority.
#[cfg(test)]
pub(crate) fn stage_for_isolated_test(
    conn: &mut SqliteConnection,
    policy: &AShareFeePolicyV2,
) -> Result<StagedPaperBookV2Catalog, StagedPaperBookV2Error> {
    stage_for_isolated_test_with_fault(conn, policy, false)
}

#[cfg(test)]
fn stage_for_isolated_test_with_fault(
    conn: &mut SqliteConnection,
    policy: &AShareFeePolicyV2,
    fail_after_schema: bool,
) -> Result<StagedPaperBookV2Catalog, StagedPaperBookV2Error> {
    conn.immediate_transaction(|conn| {
        require_isolated_test_database(conn)?;
        let identity = database_identity(conn)?;
        if identity.application_id != APPLICATION_ID
            || identity.user_version != TEST_PARENT_USER_VERSION
        {
            return Err(StagedPaperBookV2Error::WrongDatabaseIdentity);
        }
        if !objects(conn)?.is_empty() {
            return Err(StagedPaperBookV2Error::AlreadyStaged);
        }
        create_schema(conn)?;
        if fail_after_schema {
            return Err(StagedPaperBookV2Error::InjectedFailure);
        }
        insert_policy_for_isolated_test(conn, policy)?;
        verify_staged_policy(conn, policy)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;

    fn isolated_catalog_v2() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        conn.batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=2")
            .unwrap();
        conn
    }

    #[test]
    fn staged_manifest_is_immutable_and_exactly_verifiable() {
        let mut conn = isolated_catalog_v2();
        let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
        let staged = stage_for_isolated_test(&mut conn, &policy).unwrap();
        assert_eq!(staged.policy_instance_id(), policy.instance_id());
        assert_eq!(staged.catalog_sha256().len(), 64);
        assert_eq!(verify_staged_policy(&mut conn, &policy).unwrap(), staged);
        assert!(diesel::sql_query(
            "UPDATE paper_book_v2_fee_manifest SET policy_instance_id='other'"
        )
        .execute(&mut conn)
        .is_err());
        assert!(diesel::sql_query("DELETE FROM paper_book_v2_fee_manifest")
            .execute(&mut conn)
            .is_err());
        assert!(diesel::sql_query(
            "INSERT OR REPLACE INTO paper_book_v2_fee_manifest
             (singleton,schema_id,policy_instance_id,descriptor_sha256,descriptor_bytes)
             VALUES (1,'paper-book-v2-fee-manifest/v1','other',?,?)",
        )
        .bind::<Text, _>(policy.descriptor_hash())
        .bind::<Binary, _>(policy.canonical_bytes())
        .execute(&mut conn)
        .is_err());
        assert_eq!(verify_staged_policy(&mut conn, &policy).unwrap(), staged);

        let other_policy = AShareFeePolicyV2::new(
            policy.scope(),
            crate::performance::fee_policy::FeeRate::new(3, 10_000).unwrap(),
            5_000_000,
            policy.coverage(),
            "different-reviewed-revision",
        )
        .unwrap();
        assert!(matches!(
            verify_staged_policy(&mut conn, &other_policy),
            Err(StagedPaperBookV2Error::ManifestMismatch)
        ));
    }

    #[test]
    fn unknown_generation_and_extra_namespace_object_fail_closed() {
        let mut conn = isolated_catalog_v2();
        let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
        stage_for_isolated_test(&mut conn, &policy).unwrap();
        conn.batch_execute("PRAGMA user_version=99").unwrap();
        assert!(matches!(
            verify_staged_policy(&mut conn, &policy),
            Err(StagedPaperBookV2Error::WrongDatabaseIdentity)
        ));
        conn.batch_execute("PRAGMA user_version=2; CREATE TABLE paper_book_v2_extra (id INTEGER)")
            .unwrap();
        assert!(matches!(
            verify_staged_policy(&mut conn, &policy),
            Err(StagedPaperBookV2Error::CatalogMismatch)
        ));

        conn.batch_execute(
            "DROP TABLE paper_book_v2_extra; DROP TRIGGER paper_book_v2_fee_manifest_no_update",
        )
        .unwrap();
        assert!(matches!(
            verify_staged_policy(&mut conn, &policy),
            Err(StagedPaperBookV2Error::CatalogMismatch)
        ));
    }

    #[test]
    fn failed_staging_rolls_back_schema_and_manifest() {
        let mut conn = isolated_catalog_v2();
        let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
        assert!(matches!(
            stage_for_isolated_test_with_fault(&mut conn, &policy, true),
            Err(StagedPaperBookV2Error::InjectedFailure)
        ));
        assert!(objects(&mut conn).unwrap().is_empty());
        assert!(matches!(
            verify_staged_policy(&mut conn, &policy),
            Err(StagedPaperBookV2Error::CatalogMismatch)
        ));
        stage_for_isolated_test(&mut conn, &policy).unwrap();
    }

    #[test]
    fn staging_refuses_a_file_without_the_isolated_test_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_a_test_database.db");
        let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        conn.batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=2")
            .unwrap();
        let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
        assert!(matches!(
            stage_for_isolated_test(&mut conn, &policy),
            Err(StagedPaperBookV2Error::NotIsolated)
        ));
        assert!(objects(&mut conn).unwrap().is_empty());
    }

    #[test]
    fn paper_namespace_casefold_staged_fee_rejects_temp_aliases_and_v5_fee_shadow() {
        for sql in [
            "CREATE TEMP TABLE PAPER_BOOK_V2_FEE_MANIFEST AS SELECT * FROM main.paper_book_v2_fee_manifest",
            "CREATE TEMP VIEW PaPeR_BoOk_V2_FeE_MaNiFeSt AS SELECT * FROM main.paper_book_v2_fee_manifest",
            "CREATE TABLE PaPeR_BoOk_V2_unknown(value TEXT)",
            "CREATE TEMP TABLE TEST_CODE_foreign(value TEXT); CREATE TEMP TRIGGER PAPER_BOOK_V2_extra BEFORE INSERT ON TEST_CODE_foreign BEGIN SELECT 1; END",
        ] {
            let mut conn = isolated_catalog_v2();
            let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
            stage_for_isolated_test(&mut conn, &policy).unwrap();
            let before = objects(&mut conn).unwrap();
            conn.batch_execute(sql).unwrap();
            assert!(matches!(verify_staged_policy(&mut conn, &policy), Err(StagedPaperBookV2Error::CatalogMismatch)), "{sql}");
            assert_eq!(database_identity(&mut conn).unwrap().user_version, 2);
            // The V5 fee-only gate must also reject upper-case aliases. Other
            // V5 book tables legitimately do not belong to this fee manifest.
            if sql.contains("AS SELECT * FROM main.paper_book_v2_fee_manifest") {
                conn.batch_execute("PRAGMA user_version=5").unwrap();
                assert!(matches!(verify_v5_manifest_on(&mut conn), Err(StagedPaperBookV2Error::CatalogMismatch)), "{sql}");
            }
            let main = objects(&mut conn).unwrap().into_iter().filter(|row| row.namespace == "main").collect::<Vec<_>>();
            if sql.starts_with("CREATE TEMP") {
                assert_eq!(before, main);
            }
        }
    }

    #[test]
    fn paper_namespace_casefold_staging_does_not_adopt_preexisting_mixed_case() {
        let mut conn = isolated_catalog_v2();
        conn.batch_execute("CREATE TEMP VIEW PaPeR_BoOk_V2_unknown AS SELECT 1 AS value")
            .unwrap();
        let before = objects(&mut conn).unwrap();
        assert!(matches!(
            stage_for_isolated_test(
                &mut conn,
                &AShareFeePolicyV2::fixed_compatibility_assumption()
            ),
            Err(StagedPaperBookV2Error::AlreadyStaged)
        ));
        assert_eq!(objects(&mut conn).unwrap(), before);
        assert_eq!(database_identity(&mut conn).unwrap().user_version, 2);
    }
}
