use crate::portfolio::user_position_snapshot::{UserPositionItemInput, UserPositionSnapshotInput};
use chrono::{DateTime, FixedOffset};
use diesel::prelude::*;

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
    conn.transaction(|conn| {
        let existing: Option<(i64,String)> = diesel::sql_query("SELECT id, evidence_sha256 FROM user_position_snapshot WHERE snapshot_id=? OR evidence_sha256=?")
            .bind::<diesel::sql_types::Text,_>(&input.snapshot_id).bind::<diesel::sql_types::Text,_>(&input.evidence_sha256).get_result(conn).optional().map_err(|e| e.to_string())?;
        if let Some((id, hash)) = existing { if hash != input.evidence_sha256 { return Err("snapshot identity collision".into()); } return Ok(SaveUserPositionSnapshotReceipt { snapshot_row_id:id, inserted:false }); }
        diesel::sql_query("INSERT INTO user_position_snapshot(snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count) VALUES (?,?,?,?,?,?,?)")
            .bind::<diesel::sql_types::Text,_>(&input.snapshot_id).bind::<diesel::sql_types::Text,_>(input.effective_at.to_rfc3339()).bind::<diesel::sql_types::Text,_>(input.confirmed_at.to_rfc3339()).bind::<diesel::sql_types::Text,_>(&input.source).bind::<diesel::sql_types::Integer,_>(input.confirm_empty as i32).bind::<diesel::sql_types::Text,_>(&input.evidence_sha256).bind::<diesel::sql_types::Integer,_>(input.items.len() as i32).execute(conn).map_err(|e| e.to_string())?;
        let id: i64 = diesel::sql_query("SELECT id FROM user_position_snapshot WHERE snapshot_id=?").bind::<diesel::sql_types::Text,_>(&input.snapshot_id).get_result::<(i64,)>(conn).map_err(|e| e.to_string())?.0;
        for item in &input.items { diesel::sql_query("INSERT INTO user_position_snapshot_item(snapshot_id,code,name,quantity,cost_price) VALUES (?,?,?,?,?)").bind::<diesel::sql_types::Text,_>(&input.snapshot_id).bind::<diesel::sql_types::Text,_>(&item.code).bind::<diesel::sql_types::Text,_>(&item.name).bind::<diesel::sql_types::BigInt,_>(item.quantity as i64).bind::<diesel::sql_types::Double,_>(item.cost_price).execute(conn).map_err(|e| e.to_string())?; }
        Ok(SaveUserPositionSnapshotReceipt { snapshot_row_id:id, inserted:true })
    })
}

pub fn latest_user_position_snapshot() -> Result<Option<UserPositionSnapshot>, String> {
    let db = crate::database::DatabaseManager::get();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let row: Option<(i64,String,String,String,String,i32,String)> = diesel::sql_query("SELECT id,snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256 FROM user_position_snapshot ORDER BY effective_at DESC, confirmed_at DESC, snapshot_id DESC LIMIT 1").get_result(&mut conn).optional().map_err(|e| e.to_string())?;
    let Some((id, sid, ea, ca, source, empty, hash)) = row else {
        return Ok(None);
    };
    let effective_at = DateTime::parse_from_rfc3339(&ea).map_err(|e| e.to_string())?;
    let confirmed_at = DateTime::parse_from_rfc3339(&ca).map_err(|e| e.to_string())?;
    let items: Vec<(String,String,i64,f64)> = diesel::sql_query("SELECT code,name,quantity,cost_price FROM user_position_snapshot_item WHERE snapshot_id=? ORDER BY code").bind::<diesel::sql_types::Text,_>(&sid).load(&mut conn).map_err(|e|e.to_string())?;
    Ok(Some(UserPositionSnapshot {
        snapshot_row_id: id,
        snapshot_id: sid,
        effective_at,
        confirmed_at,
        source,
        confirm_empty: empty != 0,
        evidence_sha256: hash,
        items: items
            .into_iter()
            .map(|(code, name, q, cost_price)| UserPositionItemInput {
                code,
                name,
                quantity: q as u64,
                cost_price,
            })
            .collect(),
    }))
}
