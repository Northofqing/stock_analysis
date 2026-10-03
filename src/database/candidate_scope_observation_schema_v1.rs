//! Fixed storage contract for a future closed Catalog7 observation owner.
//! Local DDL is not catalog qualification, a recorded decision or approval.
//! Nothing in this module migrates a database or changes its user_version.

use diesel::{RunQueryDsl, SqliteConnection};

pub(crate) const CATALOG_GENERATION: i64 = 7;
pub(crate) const POLICY: &str = "intraday-unconsumed-pushed-row-top50-v1";
pub(crate) const SLOT_MILLISECONDS: i64 = 30_000;
pub(crate) const MAX_SCOPE_BYTES: usize = 8 * 1024 * 1024;

// Each owner UTC slot and explicit revision is a distinct occurrence. A
// content digest is deliberately not UNIQUE: equal content can recur. The
// closed owner must verify the digest, canonical contract and source binding;
// storage constraints alone cannot authenticate any of those facts.
// The named positive rowid preserves the ordinary-table Rows contract. Guard
// both unique keys: REPLACE can otherwise delete an older physical row even
// when NEW has a different occurrence and recursive delete triggers are off.
pub(crate) const STATEMENTS: &[(&str, &str, &str, &str)] = &[
    ("table", "candidate_scope_observations_v1", "candidate_scope_observations_v1", "CREATE TABLE candidate_scope_observations_v1 (
        observation_row_id INTEGER PRIMARY KEY NOT NULL CHECK(typeof(observation_row_id)='integer' AND observation_row_id>0),
        policy_id TEXT NOT NULL CHECK(typeof(policy_id)='text' AND policy_id='intraday-unconsumed-pushed-row-top50-v1'),
        slot_start_unix_ms INTEGER NOT NULL CHECK(typeof(slot_start_unix_ms)='integer' AND slot_start_unix_ms BETWEEN 0 AND 253402300770000 AND slot_start_unix_ms%30000=0),
        evaluation_revision INTEGER NOT NULL CHECK(typeof(evaluation_revision)='integer' AND evaluation_revision BETWEEN 1 AND 4294967295),
        cutoff_unix_seconds INTEGER NOT NULL CHECK(typeof(cutoff_unix_seconds)='integer' AND cutoff_unix_seconds>=slot_start_unix_ms/1000 AND cutoff_unix_seconds<slot_start_unix_ms/1000+30),
        cutoff_subsec_nanos INTEGER NOT NULL CHECK(typeof(cutoff_subsec_nanos)='integer' AND cutoff_subsec_nanos BETWEEN 0 AND 999999999),
        scope_sha256 BLOB NOT NULL CHECK(typeof(scope_sha256)='blob' AND length(scope_sha256)=32),
        scope_canonical BLOB NOT NULL CHECK(typeof(scope_canonical)='blob' AND length(scope_canonical) BETWEEN 1 AND 8388608),
        UNIQUE(policy_id,slot_start_unix_ms,evaluation_revision))"),
    ("trigger", "candidate_scope_observations_v1_no_update", "candidate_scope_observations_v1", "CREATE TRIGGER candidate_scope_observations_v1_no_update BEFORE UPDATE ON candidate_scope_observations_v1
        BEGIN SELECT RAISE(ABORT,'immutable candidate scope observation'); END"),
    ("trigger", "candidate_scope_observations_v1_no_delete", "candidate_scope_observations_v1", "CREATE TRIGGER candidate_scope_observations_v1_no_delete BEFORE DELETE ON candidate_scope_observations_v1
        BEGIN SELECT RAISE(ABORT,'immutable candidate scope observation'); END"),
    ("trigger", "candidate_scope_observations_v1_no_reinsert", "candidate_scope_observations_v1", "CREATE TRIGGER candidate_scope_observations_v1_no_reinsert BEFORE INSERT ON candidate_scope_observations_v1
        WHEN EXISTS(SELECT 1 FROM candidate_scope_observations_v1 WHERE observation_row_id=NEW.observation_row_id OR (policy_id=NEW.policy_id AND slot_start_unix_ms=NEW.slot_start_unix_ms AND evaluation_revision=NEW.evaluation_revision))
        BEGIN SELECT RAISE(ABORT,'immutable candidate scope observation'); END"),
];

/// For same-runtime reference construction and isolated schema tests. This
/// creates no owner and supplies no migration or database authority.
pub(crate) fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    for (_, _, _, sql) in STATEMENTS {
        diesel::sql_query(*sql).execute(conn)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::sql_types::{BigInt, Binary};
    use diesel::{Connection, QueryableByName};

    fn fixture() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        create_schema(&mut conn).unwrap();
        conn
    }
    fn insert(conn: &mut SqliteConnection, slot: i64, revision: i64, bytes: &[u8]) {
        diesel::sql_query("INSERT INTO main.candidate_scope_observations_v1 (policy_id,slot_start_unix_ms,evaluation_revision,cutoff_unix_seconds,cutoff_subsec_nanos,scope_sha256,scope_canonical) VALUES ('intraday-unconsumed-pushed-row-top50-v1',?,?,?/1000,999999999,zeroblob(32),?)")
            .bind::<BigInt, _>(slot)
            .bind::<BigInt, _>(revision)
            .bind::<BigInt, _>(slot)
            .bind::<Binary, _>(bytes)
            .execute(conn)
            .unwrap();
    }
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        observation_row_id: i64,
        #[diesel(sql_type = BigInt)]
        slot_start_unix_ms: i64,
        #[diesel(sql_type = BigInt)]
        evaluation_revision: i64,
        #[diesel(sql_type = Binary)]
        scope_canonical: Vec<u8>,
    }
    fn rows(conn: &mut SqliteConnection) -> Vec<Row> {
        diesel::sql_query("SELECT observation_row_id,slot_start_unix_ms,evaluation_revision,scope_canonical FROM main.candidate_scope_observations_v1 ORDER BY slot_start_unix_ms,evaluation_revision")
            .load::<Row>(conn)
            .unwrap()
    }

    #[test]
    fn candidate_scope_schema_storage_types_and_occurrence_bounds() {
        let mut conn = fixture();
        let valid = [
            "'intraday-unconsumed-pushed-row-top50-v1'",
            "0",
            "1",
            "0",
            "0",
            "zeroblob(32)",
            "X'61'",
        ];
        // No failed insert may leave an occurrence. Affinity-converted values
        // are judged by their stored SQLite type, not the caller's spelling.
        let invalid = [
            (0, "'different-policy'"),
            (1, "-30000"),
            (1, "1"),
            (1, "0.5"),
            (1, "253402300800000"),
            (2, "0"),
            (2, "4294967296"),
            (2, "1.5"),
            (3, "-1"),
            (3, "30"),
            (3, "0.5"),
            (4, "-1"),
            (4, "1000000000"),
            (4, "0.5"),
            (5, "zeroblob(31)"),
            (5, "zeroblob(33)"),
            (5, "'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'"),
            (6, "zeroblob(0)"),
            (6, "zeroblob(8388609)"),
            (6, "'a'"),
        ];
        for (column, value) in invalid
            .into_iter()
            .chain((0..valid.len()).map(|column| (column, "NULL")))
        {
            let mut values = valid;
            values[column] = value;
            let sql = format!(
                "INSERT INTO main.candidate_scope_observations_v1 (policy_id,slot_start_unix_ms,evaluation_revision,cutoff_unix_seconds,cutoff_subsec_nanos,scope_sha256,scope_canonical) VALUES ({})",
                values.join(",")
            );
            assert!(
                diesel::sql_query(sql).execute(&mut conn).is_err(),
                "column {column}: {value}"
            );
            assert!(rows(&mut conn).is_empty());
        }
        // Accept exactly the declared maximum, including the final possible
        // 30-second UTC slot and nanosecond/revision endpoints.
        let bytes = vec![b'a'; MAX_SCOPE_BYTES];
        insert(&mut conn, 253402300770000, 4294967295, &bytes);
        let actual = rows(&mut conn);
        assert_eq!(actual.len(), 1);
        assert_eq!(actual[0].scope_canonical, bytes);
        assert!(diesel::sql_query("INSERT INTO main.candidate_scope_observations_v1 VALUES (-1,'intraday-unconsumed-pushed-row-top50-v1',0,1,0,0,zeroblob(32),X'61')").execute(&mut conn).is_err());
        diesel::sql_query("INSERT INTO main.candidate_scope_observations_v1 VALUES (2,'intraday-unconsumed-pushed-row-top50-v1',253402300770000,4294967294,253402300799,999999999,zeroblob(32),X'61')")
            .execute(&mut conn).unwrap();
        assert_eq!(rows(&mut conn).len(), 2);
    }

    #[test]
    fn candidate_scope_schema_replacement_and_mutation_preserve_original() {
        let mut conn = fixture();
        insert(&mut conn, 0, 1, b"original\0bytes");
        for sql in [
            "UPDATE main.candidate_scope_observations_v1 SET scope_canonical=X'62'",
            "DELETE FROM main.candidate_scope_observations_v1",
            "INSERT INTO main.candidate_scope_observations_v1 VALUES (NULL,'intraday-unconsumed-pushed-row-top50-v1',0,1,1,0,zeroblob(32),X'62')",
            "INSERT OR REPLACE INTO main.candidate_scope_observations_v1 VALUES (NULL,'intraday-unconsumed-pushed-row-top50-v1',0,1,1,0,zeroblob(32),X'62')",
            "INSERT OR IGNORE INTO main.candidate_scope_observations_v1 VALUES (NULL,'intraday-unconsumed-pushed-row-top50-v1',0,1,1,0,zeroblob(32),X'62')",
            "INSERT INTO main.candidate_scope_observations_v1 VALUES (NULL,'intraday-unconsumed-pushed-row-top50-v1',0,1,1,0,zeroblob(32),X'62') ON CONFLICT DO UPDATE SET scope_canonical=X'62'",
        ] {
            assert!(diesel::sql_query(sql).execute(&mut conn).is_err());
            let actual = rows(&mut conn);
            assert_eq!(actual.len(), 1);
            assert_eq!(actual[0].scope_canonical, b"original\0bytes");
            assert_eq!(actual[0].slot_start_unix_ms, 0);
            assert_eq!(actual[0].evaluation_revision, 1);
        }
    }

    #[test]
    fn candidate_scope_schema_hidden_rowid_replace_preserves_original() {
        let mut conn = fixture();
        diesel::sql_query("PRAGMA recursive_triggers=OFF")
            .execute(&mut conn)
            .unwrap();
        insert(&mut conn, 0, 1, b"original");
        assert_eq!(rows(&mut conn)[0].observation_row_id, 1);
        // A different logical occurrence must not use REPLACE's physical
        // uniqueness conflict to delete history when recursive triggers are
        // disabled. All hidden-rowid aliases point at the explicit positive
        // INTEGER PRIMARY KEY, which the same BEFORE INSERT guard checks.
        for alias in ["rowid", "_rowid_", "oid", "observation_row_id"] {
            let sql = format!("INSERT OR REPLACE INTO main.candidate_scope_observations_v1 ({alias},policy_id,slot_start_unix_ms,evaluation_revision,cutoff_unix_seconds,cutoff_subsec_nanos,scope_sha256,scope_canonical) VALUES (1,'intraday-unconsumed-pushed-row-top50-v1',30000,1,30,0,zeroblob(32),X'62')");
            assert!(
                diesel::sql_query(sql).execute(&mut conn).is_err(),
                "alias {alias}"
            );
            let actual = rows(&mut conn);
            assert_eq!(actual.len(), 1);
            assert_eq!(actual[0].observation_row_id, 1);
            assert_eq!(actual[0].slot_start_unix_ms, 0);
            assert_eq!(actual[0].scope_canonical, b"original");
        }
    }

    #[test]
    fn candidate_scope_schema_equal_content_has_distinct_occurrences() {
        let mut conn = fixture();
        insert(&mut conn, 0, 1, b"same content");
        insert(&mut conn, SLOT_MILLISECONDS, 1, b"same content");
        insert(&mut conn, SLOT_MILLISECONDS, 2, b"same content");
        let actual = rows(&mut conn);
        assert_eq!(actual.len(), 3);
        assert_eq!(
            actual
                .iter()
                .map(|r| (r.slot_start_unix_ms, r.evaluation_revision))
                .collect::<Vec<_>>(),
            vec![(0, 1), (30000, 1), (30000, 2)]
        );
        assert!(actual.iter().all(|r| r.scope_canonical == b"same content"));
    }
}
