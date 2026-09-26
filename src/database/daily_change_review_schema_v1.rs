//! Explicit CatalogV3 extension; ordinary startup and consumers never install it.
pub(crate) const CATALOG_GENERATION: i64 = 3;
pub(crate) const STATEMENTS: &[(&str, &str, &str, &str)] = &[
    ("table", "daily_change_review_event", "daily_change_review_event", "CREATE TABLE IF NOT EXISTS daily_change_review_event (
        seq INTEGER PRIMARY KEY AUTOINCREMENT, schema_version INTEGER NOT NULL CHECK(schema_version=1),
        command_id TEXT NOT NULL UNIQUE, scope_key TEXT NOT NULL, candidate_id TEXT NOT NULL,
        revision INTEGER NOT NULL CHECK(revision>0), kind TEXT NOT NULL CHECK(kind IN ('Candidate','Observation','Decision')),
        payload TEXT NOT NULL, previous_hash TEXT NOT NULL, record_hash TEXT NOT NULL UNIQUE CHECK(length(record_hash)=64))"),
    ("index", "daily_change_review_candidate", "daily_change_review_event", "CREATE UNIQUE INDEX IF NOT EXISTS daily_change_review_candidate ON daily_change_review_event(candidate_id) WHERE kind='Candidate'"),
    ("index", "daily_change_review_revision", "daily_change_review_event", "CREATE UNIQUE INDEX IF NOT EXISTS daily_change_review_revision ON daily_change_review_event(scope_key,revision) WHERE kind='Candidate'"),
    ("index", "daily_change_review_decision", "daily_change_review_event", "CREATE UNIQUE INDEX IF NOT EXISTS daily_change_review_decision ON daily_change_review_event(candidate_id) WHERE kind='Decision'"),
    ("trigger", "daily_change_review_no_update", "daily_change_review_event", "CREATE TRIGGER IF NOT EXISTS daily_change_review_no_update BEFORE UPDATE ON daily_change_review_event BEGIN SELECT RAISE(ABORT,'immutable daily change review'); END"),
    ("trigger", "daily_change_review_no_delete", "daily_change_review_event", "CREATE TRIGGER IF NOT EXISTS daily_change_review_no_delete BEFORE DELETE ON daily_change_review_event BEGIN SELECT RAISE(ABORT,'append-only daily change review'); END"),
];

pub(crate) fn create_schema(conn: &mut diesel::SqliteConnection) -> diesel::QueryResult<()> {
    use diesel::connection::SimpleConnection;
    for (_, _, _, sql) in STATEMENTS {
        conn.batch_execute(sql)?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq, diesel::QueryableByName)]
struct Object {
    #[diesel(sql_type=diesel::sql_types::Text)]
    namespace: String,
    #[diesel(sql_type=diesel::sql_types::Text)]
    kind: String,
    #[diesel(sql_type=diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type=diesel::sql_types::Text)]
    table_name: String,
    #[diesel(sql_type=diesel::sql_types::Text)]
    sql: String,
}

fn objects(conn: &mut diesel::SqliteConnection) -> diesel::QueryResult<Vec<Object>> {
    use diesel::RunQueryDsl;
    diesel::sql_query("SELECT 'main' AS namespace,type AS kind,name,tbl_name AS table_name,coalesce(sql,'') AS sql FROM main.sqlite_master WHERE name GLOB 'daily_change_review_*' OR tbl_name GLOB 'daily_change_review_*' UNION ALL SELECT 'temp',type,name,tbl_name,coalesce(sql,'') FROM temp.sqlite_master WHERE name GLOB 'daily_change_review_*' OR tbl_name GLOB 'daily_change_review_*' ORDER BY namespace,kind,name").load(conn)
}

/// Same-transaction local namespace proof. Global whole-app qualification remains
/// the maintenance owner's responsibility; this reader never installs anything.
pub(crate) fn is_present(
    conn: &mut diesel::SqliteConnection,
) -> super::daily_change_review::ReviewResult<bool> {
    use super::daily_change_review::ReviewError;
    use diesel::{Connection, RunQueryDsl};
    #[derive(diesel::QueryableByName)]
    struct Identity {
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        application_id: i64,
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        user_version: i64,
    }
    let identity: Identity = diesel::sql_query(
        "SELECT application_id,user_version FROM pragma_application_id(),pragma_user_version()",
    )
    .get_result(conn)?;
    let actual = objects(conn)?;
    let legacy = (identity.application_id == 0 && identity.user_version == 0)
        || (identity.application_id == 1398035265 && matches!(identity.user_version, 1 | 2));
    if actual.is_empty() && legacy {
        return Ok(false);
    }
    let qualified_generation =
        identity.application_id == 1398035265 && identity.user_version == CATALOG_GENERATION;
    #[cfg(test)]
    let qualified_generation =
        qualified_generation || (identity.application_id == 0 && identity.user_version == 0);
    if !qualified_generation {
        return Err(ReviewError::Audit(
            "review catalog generation mismatch".into(),
        ));
    }
    let mut reference = diesel::SqliteConnection::establish(":memory:")
        .map_err(|e| ReviewError::Audit(e.to_string()))?;
    create_schema(&mut reference)?;
    if actual != objects(&mut reference)? {
        return Err(ReviewError::Audit(
            "review catalog namespace mismatch".into(),
        ));
    }
    Ok(true)
}
