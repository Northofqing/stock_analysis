//! BR-146: immutable persistence for complete user-confirmed snapshots.
use crate::portfolio::user_position_snapshot::{UserPositionItemInput, UserPositionSnapshotInput};
use chrono::{DateTime, FixedOffset};
use diesel::prelude::*;

#[derive(QueryableByName)]
struct SnapshotRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    snapshot_id: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    effective_at: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    confirmed_at: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source: String,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    confirm_empty: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    evidence_sha256: String,
}
#[derive(QueryableByName)]
struct SnapshotIdentity {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    evidence_sha256: String,
}
#[derive(QueryableByName)]
struct SnapshotIdOnly {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
}
#[derive(QueryableByName)]
struct SnapshotItem {
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    cost_price: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveUserPositionSnapshotReceipt {
    pub snapshot_row_id: i64,
    pub inserted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserPositionSnapshot {
    pub snapshot_row_id: i64,
    pub snapshot_id: String,
    pub effective_at: DateTime<FixedOffset>,
    pub confirmed_at: DateTime<FixedOffset>,
    pub source: String,
    pub confirm_empty: bool,
    pub evidence_sha256: String,
    pub items: Vec<UserPositionItemInput>,
}

pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS user_position_snapshot (id INTEGER PRIMARY KEY AUTOINCREMENT, snapshot_id TEXT NOT NULL UNIQUE, effective_at TEXT NOT NULL, confirmed_at TEXT NOT NULL, source TEXT NOT NULL, confirm_empty INTEGER NOT NULL CHECK(confirm_empty IN (0,1)), evidence_sha256 TEXT NOT NULL UNIQUE, item_count INTEGER NOT NULL CHECK(item_count >= 0), recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)",
        "CREATE TABLE IF NOT EXISTS user_position_snapshot_item (snapshot_id TEXT NOT NULL REFERENCES user_position_snapshot(snapshot_id), code TEXT NOT NULL, name TEXT NOT NULL, quantity INTEGER NOT NULL CHECK(quantity > 0), cost_price REAL NOT NULL CHECK(cost_price > 0), PRIMARY KEY(snapshot_id, code))",
        "CREATE TRIGGER IF NOT EXISTS user_position_snapshot_no_update BEFORE UPDATE ON user_position_snapshot BEGIN SELECT RAISE(ABORT, 'user_position_snapshot is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS user_position_snapshot_no_delete BEFORE DELETE ON user_position_snapshot BEGIN SELECT RAISE(ABORT, 'user_position_snapshot is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS user_position_snapshot_item_no_update BEFORE UPDATE ON user_position_snapshot_item BEGIN SELECT RAISE(ABORT, 'user_position_snapshot_item is append-only'); END",
        "CREATE TRIGGER IF NOT EXISTS user_position_snapshot_item_no_delete BEFORE DELETE ON user_position_snapshot_item BEGIN SELECT RAISE(ABORT, 'user_position_snapshot_item is append-only'); END",
    ] { diesel::sql_query(sql).execute(conn).map_err(|e| e.to_string())?; }
    Ok(())
}

pub fn save_user_position_snapshot(
    input: &UserPositionSnapshotInput,
) -> Result<SaveUserPositionSnapshotReceipt, String> {
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    save_user_position_snapshot_with_conn(&mut conn, input)
}

fn save_user_position_snapshot_with_conn(
    conn: &mut SqliteConnection,
    input: &UserPositionSnapshotInput,
) -> Result<SaveUserPositionSnapshotReceipt, String> {
    conn.transaction(|conn| {
        let existing: Option<SnapshotIdentity> = diesel::sql_query("SELECT id AS id, evidence_sha256 AS evidence_sha256 FROM user_position_snapshot WHERE snapshot_id=? OR evidence_sha256=?")
            .bind::<diesel::sql_types::Text,_>(&input.snapshot_id).bind::<diesel::sql_types::Text,_>(&input.evidence_sha256).get_result(conn).optional()?;
        if let Some(row) = existing { if row.evidence_sha256 != input.evidence_sha256 { return Err(diesel::result::Error::RollbackTransaction); } return Ok(SaveUserPositionSnapshotReceipt { snapshot_row_id:row.id, inserted:false }); }
        diesel::sql_query("INSERT INTO user_position_snapshot(snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count) VALUES (?,?,?,?,?,?,?)")
            .bind::<diesel::sql_types::Text,_>(&input.snapshot_id).bind::<diesel::sql_types::Text,_>(input.effective_at.to_rfc3339()).bind::<diesel::sql_types::Text,_>(input.confirmed_at.to_rfc3339()).bind::<diesel::sql_types::Text,_>(&input.source).bind::<diesel::sql_types::Integer,_>(input.confirm_empty as i32).bind::<diesel::sql_types::Text,_>(&input.evidence_sha256).bind::<diesel::sql_types::Integer,_>(input.items.len() as i32).execute(conn)?;
        let id: i64 = diesel::sql_query("SELECT id FROM user_position_snapshot WHERE snapshot_id=?").bind::<diesel::sql_types::Text,_>(&input.snapshot_id).get_result::<SnapshotIdOnly>(conn)?.id;
        for item in &input.items { diesel::sql_query("INSERT INTO user_position_snapshot_item(snapshot_id,code,name,quantity,cost_price) VALUES (?,?,?,?,?)").bind::<diesel::sql_types::Text,_>(&input.snapshot_id).bind::<diesel::sql_types::Text,_>(&item.code).bind::<diesel::sql_types::Text,_>(&item.name).bind::<diesel::sql_types::BigInt,_>(item.quantity as i64).bind::<diesel::sql_types::Double,_>(item.cost_price).execute(conn)?; }
        Ok(SaveUserPositionSnapshotReceipt { snapshot_row_id:id, inserted:true })
    }).map_err(|e| e.to_string())
}

pub fn latest_user_position_snapshot() -> Result<Option<UserPositionSnapshot>, String> {
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    latest_user_position_snapshot_with_conn(&mut conn)
}

fn latest_user_position_snapshot_with_conn(
    conn: &mut SqliteConnection,
) -> Result<Option<UserPositionSnapshot>, String> {
    let row: Option<SnapshotRow> = diesel::sql_query("SELECT id,snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256 FROM user_position_snapshot ORDER BY effective_at DESC, confirmed_at DESC, snapshot_id DESC LIMIT 1").get_result(&mut *conn).optional().map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let effective_at =
        DateTime::parse_from_rfc3339(&row.effective_at).map_err(|e| e.to_string())?;
    let confirmed_at =
        DateTime::parse_from_rfc3339(&row.confirmed_at).map_err(|e| e.to_string())?;
    let items: Vec<SnapshotItem> = diesel::sql_query("SELECT code,name,quantity,cost_price FROM user_position_snapshot_item WHERE snapshot_id=? ORDER BY code").bind::<diesel::sql_types::Text,_>(&row.snapshot_id).load(&mut *conn).map_err(|e|e.to_string())?;
    Ok(Some(UserPositionSnapshot {
        snapshot_row_id: row.id,
        snapshot_id: row.snapshot_id,
        effective_at,
        confirmed_at,
        source: row.source,
        confirm_empty: row.confirm_empty != 0,
        evidence_sha256: row.evidence_sha256,
        items: items
            .into_iter()
            .map(|item| UserPositionItemInput {
                code: item.code,
                name: item.name,
                quantity: item.quantity as u64,
                cost_price: item.cost_price,
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use diesel::connection::SimpleConnection;

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        conn.batch_execute("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        create_schema(&mut conn).expect("position snapshot schema");
        conn
    }

    fn input(snapshot_id: &str, evidence: char) -> UserPositionSnapshotInput {
        UserPositionSnapshotInput {
            snapshot_id: snapshot_id.to_owned(),
            effective_at: DateTime::parse_from_rfc3339("2026-07-24T15:00:00+08:00")
                .expect("effective timestamp"),
            confirmed_at: DateTime::parse_from_rfc3339("2026-07-24T15:01:00+08:00")
                .expect("confirmed timestamp"),
            source: "TEST_CODE_USER_CONFIRMED".to_owned(),
            confirm_empty: false,
            evidence_sha256: evidence.to_string().repeat(64),
            items: vec![
                UserPositionItemInput {
                    code: "TEST_CODE_600000".to_owned(),
                    name: "TEST_CODE_乙".to_owned(),
                    quantity: 200,
                    cost_price: 20.0,
                },
                UserPositionItemInput {
                    code: "TEST_CODE_000001".to_owned(),
                    name: "TEST_CODE_甲".to_owned(),
                    quantity: 100,
                    cost_price: 10.0,
                },
            ],
        }
    }

    #[test]
    fn sqlite_round_trip_deduplicates_evidence_and_is_append_only() {
        let mut conn = connection();
        assert!(latest_user_position_snapshot_with_conn(&mut conn)
            .expect("empty read")
            .is_none());
        let first = input("TEST_CODE_SNAPSHOT_A", 'a');
        let inserted =
            save_user_position_snapshot_with_conn(&mut conn, &first).expect("first insert");
        assert!(inserted.inserted);

        let mut same_evidence = first.clone();
        same_evidence.snapshot_id = "TEST_CODE_SNAPSHOT_B".to_owned();
        let duplicate = save_user_position_snapshot_with_conn(&mut conn, &same_evidence)
            .expect("same evidence is idempotent");
        assert!(!duplicate.inserted);
        assert_eq!(duplicate.snapshot_row_id, inserted.snapshot_row_id);

        let latest = latest_user_position_snapshot_with_conn(&mut conn)
            .expect("latest read")
            .expect("persisted snapshot");
        assert_eq!(latest.snapshot_id, "TEST_CODE_SNAPSHOT_A");
        assert_eq!(latest.items[0].code, "TEST_CODE_000001");
        assert_eq!(latest.items[1].quantity, 200);

        let mutation = diesel::sql_query(
            "DELETE FROM user_position_snapshot_item
             WHERE snapshot_id='TEST_CODE_SNAPSHOT_A'",
        )
        .execute(&mut conn)
        .expect_err("append-only trigger");
        assert!(mutation.to_string().contains("append-only"));
    }

    #[test]
    fn conflicting_identity_and_duplicate_child_are_atomic_failures() {
        let mut conn = connection();
        let first = input("TEST_CODE_SNAPSHOT_A", 'a');
        save_user_position_snapshot_with_conn(&mut conn, &first).expect("seed");
        let conflicting = input("TEST_CODE_SNAPSHOT_A", 'b');
        assert!(save_user_position_snapshot_with_conn(&mut conn, &conflicting).is_err());

        let mut duplicate_child = input("TEST_CODE_SNAPSHOT_C", 'c');
        duplicate_child.items[1].code = duplicate_child.items[0].code.clone();
        assert!(save_user_position_snapshot_with_conn(&mut conn, &duplicate_child).is_err());
        let count: i64 = diesel::sql_query(
            "SELECT COUNT(*) AS count FROM user_position_snapshot
             WHERE snapshot_id='TEST_CODE_SNAPSHOT_C'",
        )
        .get_result::<SnapshotCount>(&mut conn)
        .expect("count")
        .count;
        assert_eq!(count, 0);
    }

    #[derive(QueryableByName)]
    struct SnapshotCount {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }

    #[test]
    fn malformed_persisted_timestamp_fails_explicitly() {
        let mut conn = connection();
        diesel::sql_query(
            "INSERT INTO user_position_snapshot(
                snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count
             ) VALUES (
                'TEST_CODE_BAD_TIME','not-a-time','not-a-time','TEST_CODE_SOURCE',1,
                'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',0
             )",
        )
        .execute(&mut conn)
        .expect("historical malformed row");
        let error =
            latest_user_position_snapshot_with_conn(&mut conn).expect_err("malformed stored time");
        assert!(!error.is_empty());
    }
}
