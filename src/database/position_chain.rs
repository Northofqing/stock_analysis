//! BR-123/BR-170 append-only Magic TDX position-chain assignment persistence.

use crate::data_gateway::position_chain::validate_position_chain_assignment;
use crate::data_gateway::{BoardKind, PositionChainAssignment};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{Nullable, Text};
use thiserror::Error;

use super::DatabaseManager;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS position_chain_assignment (
    assignment_id TEXT PRIMARY KEY,
    code TEXT NOT NULL,
    board_code TEXT NOT NULL,
    board_name TEXT NOT NULL,
    board_kind TEXT NOT NULL CHECK (board_kind IN ('industry', 'concept')),
    memberships_json TEXT NOT NULL,
    provider TEXT NOT NULL CHECK (provider = 'Tdx'),
    source TEXT NOT NULL,
    source_at TEXT,
    observed_at TEXT NOT NULL,
    source_batch_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (__POSITION_CODE_CHECK__),
    CHECK (length(trim(assignment_id)) > 0),
    CHECK (length(trim(board_code)) > 0),
    CHECK (length(trim(board_name)) > 0),
    CHECK (length(trim(source)) > 0),
    CHECK (length(trim(observed_at)) > 0),
    CHECK (length(trim(source_batch_id)) > 0),
    CHECK (length(content_hash) = 64)
);
CREATE INDEX IF NOT EXISTS idx_position_chain_assignment_code
    ON position_chain_assignment(code, created_at, assignment_id);
"#;

#[derive(Debug, Error)]
pub enum PositionChainStoreError {
    #[error("position chain assignment conflict identity={0}")]
    Conflict(String),
    #[error("invalid position chain assignment: {0}")]
    InvalidInput(String),
    #[error("position chain database error: {0}")]
    Database(#[from] diesel::result::Error),
}

pub type PositionChainStoreResult<T> = Result<T, PositionChainStoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedPositionChain {
    pub assignment_id: String,
    pub code: String,
    pub board_code: String,
    pub board_name: String,
    pub kind: BoardKind,
    pub provider: String,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub source_batch_id: String,
    pub content_hash: String,
}

#[derive(QueryableByName)]
struct ExistingHashRow {
    #[diesel(sql_type = Text)]
    content_hash: String,
}

#[derive(QueryableByName)]
struct LinkedRow {
    #[diesel(sql_type = Text)]
    assignment_id: String,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    board_code: String,
    #[diesel(sql_type = Text)]
    board_name: String,
    #[diesel(sql_type = Text)]
    board_kind: String,
    #[diesel(sql_type = Text)]
    provider: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Nullable<Text>)]
    source_at: Option<String>,
    #[diesel(sql_type = Text)]
    observed_at: String,
    #[diesel(sql_type = Text)]
    source_batch_id: String,
    #[diesel(sql_type = Text)]
    content_hash: String,
}

#[cfg(not(test))]
fn position_code_check() -> &'static str {
    "length(code) = 6 AND code NOT GLOB '*[^0-9]*'"
}

#[cfg(test)]
fn position_code_check() -> &'static str {
    "code GLOB 'TEST_CODE_[0-9][0-9][0-9][0-9][0-9][0-9]'"
}

fn board_kind_label(kind: BoardKind) -> &'static str {
    match kind {
        BoardKind::Industry => "industry",
        BoardKind::Concept => "concept",
        BoardKind::Region => "region",
    }
}

fn parse_board_kind(value: &str) -> PositionChainStoreResult<BoardKind> {
    match value {
        "industry" => Ok(BoardKind::Industry),
        "concept" => Ok(BoardKind::Concept),
        _ => Err(PositionChainStoreError::InvalidInput(format!(
            "persisted board kind is unsupported: {value:?}"
        ))),
    }
}

pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    let schema = SCHEMA.replace("__POSITION_CODE_CHECK__", position_code_check());
    conn.batch_execute(&schema)
        .map_err(|error| error.to_string())?;
    for action in ["UPDATE", "DELETE"] {
        let suffix = action.to_ascii_lowercase();
        conn.batch_execute(&format!(
            "CREATE TRIGGER IF NOT EXISTS trg_position_chain_assignment_no_{suffix}
             BEFORE {action} ON position_chain_assignment
             BEGIN
                 SELECT RAISE(ABORT, 'BR-170 position_chain_assignment is append-only');
             END;"
        ))
        .map_err(|error| error.to_string())?;
    }
    DatabaseManager::add_column_if_missing(conn, "stock_position", "chain_assignment_id", "TEXT")
        .map_err(|error| error.to_string())?;
    conn.batch_execute(
        "CREATE INDEX IF NOT EXISTS idx_stock_position_chain_assignment_id
             ON stock_position(chain_assignment_id);
         UPDATE stock_position
            SET chain_name = NULL,
                chain_assignment_id = NULL
          WHERE chain_name IS NOT NULL
            AND (
                chain_assignment_id IS NULL
                OR NOT EXISTS (
                    SELECT 1
                      FROM position_chain_assignment AS assignment
                     WHERE assignment.assignment_id = stock_position.chain_assignment_id
                       AND assignment.code = stock_position.code
                       AND assignment.board_name = stock_position.chain_name
                )
            );",
    )
    .map_err(|error| error.to_string())
}

pub struct PositionChainStore<'a> {
    conn: &'a mut SqliteConnection,
}

pub(crate) fn append_assignment_and_link_on_conn(
    conn: &mut SqliteConnection,
    assignment: &PositionChainAssignment,
) -> PositionChainStoreResult<bool> {
    validate_position_chain_assignment(assignment)
        .map_err(|error| PositionChainStoreError::InvalidInput(error.to_string()))?;
    let memberships_json = serde_json::to_string(&assignment.memberships)
        .map_err(|error| PositionChainStoreError::InvalidInput(error.to_string()))?;
    let existing = diesel::sql_query(
        "SELECT content_hash
           FROM position_chain_assignment
          WHERE assignment_id = ?",
    )
    .bind::<Text, _>(&assignment.assignment_id)
    .get_result::<ExistingHashRow>(conn)
    .optional()?;
    let inserted = match existing {
        Some(existing) if existing.content_hash == assignment.content_hash => false,
        Some(_) => {
            return Err(PositionChainStoreError::Conflict(
                assignment.assignment_id.clone(),
            ));
        }
        None => {
            diesel::sql_query(
                "INSERT INTO position_chain_assignment (
                    assignment_id, code, board_code, board_name, board_kind,
                    memberships_json, provider, source, source_at, observed_at,
                    source_batch_id, content_hash
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind::<Text, _>(&assignment.assignment_id)
            .bind::<Text, _>(&assignment.code)
            .bind::<Text, _>(&assignment.primary.board_code)
            .bind::<Text, _>(&assignment.primary.board_name)
            .bind::<Text, _>(board_kind_label(assignment.primary.kind))
            .bind::<Text, _>(&memberships_json)
            .bind::<Text, _>(format!("{:?}", assignment.evidence.provider))
            .bind::<Text, _>(&assignment.evidence.source)
            .bind::<Nullable<Text>, _>(assignment.evidence.source_at.as_deref())
            .bind::<Text, _>(&assignment.evidence.observed_at)
            .bind::<Text, _>(&assignment.evidence.batch_id)
            .bind::<Text, _>(&assignment.content_hash)
            .execute(conn)?;
            true
        }
    };

    let linked = diesel::sql_query(
        "UPDATE stock_position
            SET chain_name = ?,
                chain_assignment_id = ?
          WHERE code = ?
            AND status = 'open'",
    )
    .bind::<Text, _>(&assignment.primary.board_name)
    .bind::<Text, _>(&assignment.assignment_id)
    .bind::<Text, _>(&assignment.code)
    .execute(conn)?;
    if linked == 0 {
        return Err(PositionChainStoreError::InvalidInput(format!(
            "no open position exists for {}",
            assignment.code
        )));
    }
    Ok(inserted)
}

impl<'a> PositionChainStore<'a> {
    pub fn new(conn: &'a mut SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn commit(
        &mut self,
        assignment: &PositionChainAssignment,
    ) -> PositionChainStoreResult<bool> {
        self.conn
            .immediate_transaction::<_, PositionChainStoreError, _>(|conn| {
                append_assignment_and_link_on_conn(conn, assignment)
            })
    }

    pub fn linked(&mut self, code: &str) -> PositionChainStoreResult<Option<LinkedPositionChain>> {
        let row = diesel::sql_query(
            "SELECT assignment.assignment_id,
                    assignment.code,
                    assignment.board_code,
                    assignment.board_name,
                    assignment.board_kind,
                    assignment.provider,
                    assignment.source,
                    assignment.source_at,
                    assignment.observed_at,
                    assignment.source_batch_id,
                    assignment.content_hash
               FROM stock_position AS position
               JOIN position_chain_assignment AS assignment
                 ON assignment.assignment_id = position.chain_assignment_id
                AND assignment.code = position.code
              WHERE position.code = ?
                AND position.status = 'open'
              ORDER BY position.id DESC
              LIMIT 1",
        )
        .bind::<Text, _>(code)
        .get_result::<LinkedRow>(self.conn)
        .optional()?;
        row.map(|row| {
            Ok(LinkedPositionChain {
                assignment_id: row.assignment_id,
                code: row.code,
                board_code: row.board_code,
                board_name: row.board_name,
                kind: parse_board_kind(&row.board_kind)?,
                provider: row.provider,
                source: row.source,
                source_at: row.source_at,
                observed_at: row.observed_at,
                source_batch_id: row.source_batch_id,
                content_hash: row.content_hash,
            })
        })
        .transpose()
    }

    pub fn clear_link(&mut self, code: &str) -> PositionChainStoreResult<usize> {
        diesel::sql_query(
            "UPDATE stock_position
                SET chain_name = NULL,
                    chain_assignment_id = NULL
              WHERE code = ?
                AND status = 'open'",
        )
        .bind::<Text, _>(code)
        .execute(self.conn)
        .map_err(PositionChainStoreError::from)
    }
}

impl DatabaseManager {
    pub fn commit_position_chain_assignment(
        &self,
        assignment: &PositionChainAssignment,
    ) -> Result<bool, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-170 position chain connection failed: {error}"))?;
        PositionChainStore::new(&mut conn)
            .commit(assignment)
            .map_err(|error| error.to_string())
    }

    pub fn linked_position_chain(&self, code: &str) -> Result<Option<LinkedPositionChain>, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-170 position chain reader failed: {error}"))?;
        PositionChainStore::new(&mut conn)
            .linked(code)
            .map_err(|error| error.to_string())
    }

    pub fn clear_position_chain_link(&self, code: &str) -> Result<usize, String> {
        let mut conn = self
            .get_conn()
            .map_err(|error| format!("BR-170 position chain clear failed: {error}"))?;
        PositionChainStore::new(&mut conn)
            .clear_link(code)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::data_gateway::{BatchEvidence, BoardKind, PositionChainAssignment};
    use diesel::connection::SimpleConnection;
    use diesel::prelude::*;
    use crate::magic_compat::ProviderId;

    use super::*;

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }

    #[derive(QueryableByName)]
    struct NullableChainRow {
        #[diesel(sql_type = Nullable<Text>)]
        chain_name: Option<String>,
        #[diesel(sql_type = Nullable<Text>)]
        chain_assignment_id: Option<String>,
    }

    fn connection() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").expect("memory sqlite");
        conn.batch_execute(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE stock_position (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 code TEXT NOT NULL,
                 status TEXT NOT NULL,
                 chain_name TEXT
             );
             INSERT INTO stock_position (code, status, chain_name)
             VALUES ('TEST_CODE_600396', 'open', NULL);",
        )
        .expect("position schema");
        create_schema(&mut conn).expect("position chain schema");
        conn
    }

    fn assignment() -> PositionChainAssignment {
        crate::data_gateway::derive_position_chain(
            "TEST_CODE_600396",
            crate::data_gateway::GatewayBatch::Available {
                records: vec![crate::data_gateway::BoardMembershipRecord {
                    instrument_code: "TEST_CODE_600396".to_owned(),
                    board_code: "TEST_CODE_INDUSTRY".to_owned(),
                    board_name: "测试电力".to_owned(),
                    kind: BoardKind::Industry,
                }],
                evidence: BatchEvidence {
                    provider: ProviderId::Tdx,
                    source: "TEST_CODE_tdx-board-memberships".to_owned(),
                    source_at: None,
                    observed_at: "TEST_CODE_observed_1".to_owned(),
                    batch_id: "TEST_CODE_board_batch_1".to_owned(),
                },
            },
        )
        .expect("valid assignment batch")
        .expect("position chain")
    }

    #[test]
    fn committed_assignment_is_atomically_linked_to_the_position() {
        let mut conn = connection();
        let assignment = assignment();
        let mut store = PositionChainStore::new(&mut conn);

        assert!(store.commit(&assignment).expect("first commit"));
        let linked = store
            .linked("TEST_CODE_600396")
            .expect("linked read")
            .expect("linked assignment");

        assert_eq!(linked.assignment_id, assignment.assignment_id);
        assert_eq!(linked.board_code, "TEST_CODE_INDUSTRY");
        assert_eq!(linked.board_name, "测试电力");
        assert_eq!(linked.kind, BoardKind::Industry);
        assert_eq!(linked.source_batch_id, "TEST_CODE_board_batch_1");
    }

    #[test]
    fn identical_replay_is_idempotent_and_conflicting_identity_is_rejected() {
        let mut conn = connection();
        let assignment = assignment();
        let mut store = PositionChainStore::new(&mut conn);
        assert!(store.commit(&assignment).expect("first commit"));
        assert!(!store.commit(&assignment).expect("idempotent replay"));

        let mut conflict_conn = connection();
        diesel::sql_query(
            "INSERT INTO position_chain_assignment (
                assignment_id, code, board_code, board_name, board_kind,
                memberships_json, provider, source, source_at, observed_at,
                source_batch_id, content_hash
             ) VALUES (?, 'TEST_CODE_600396', 'TEST_CODE_CONFLICT', '冲突',
                       'industry', '[]', 'Tdx', 'TEST_CODE_source', NULL,
                       'TEST_CODE_observed', 'TEST_CODE_batch', ?)",
        )
        .bind::<Text, _>(&assignment.assignment_id)
        .bind::<Text, _>(std::iter::repeat_n('b', 64).collect::<String>())
        .execute(&mut conflict_conn)
        .expect("seed conflicting immutable row");
        assert!(matches!(
            PositionChainStore::new(&mut conflict_conn).commit(&assignment),
            Err(PositionChainStoreError::Conflict(_))
        ));
    }

    #[test]
    fn assignment_table_is_append_only() {
        let mut conn = connection();
        PositionChainStore::new(&mut conn)
            .commit(&assignment())
            .expect("commit assignment");

        diesel::sql_query("UPDATE position_chain_assignment SET board_name = 'TEST_CODE_changed'")
            .execute(&mut conn)
            .expect_err("UPDATE must be blocked");
        diesel::sql_query("DELETE FROM position_chain_assignment")
            .execute(&mut conn)
            .expect_err("DELETE must be blocked");
    }

    #[test]
    fn missing_open_position_rolls_back_assignment_insert() {
        let mut conn = connection();
        diesel::sql_query("DELETE FROM stock_position")
            .execute(&mut conn)
            .expect("remove position fixture");

        PositionChainStore::new(&mut conn)
            .commit(&assignment())
            .expect_err("assignment cannot exist without an open position link");

        let count = diesel::sql_query("SELECT COUNT(*) AS count FROM position_chain_assignment")
            .get_result::<CountRow>(&mut conn)
            .expect("assignment count");
        assert_eq!(count.count, 0);
    }

    #[test]
    fn schema_initialization_clears_unlinked_legacy_chain_name() {
        let mut conn = SqliteConnection::establish(":memory:").expect("memory sqlite");
        conn.batch_execute(
            "CREATE TABLE stock_position (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 code TEXT NOT NULL,
                 status TEXT NOT NULL,
                 chain_name TEXT
             );
             INSERT INTO stock_position (code, status, chain_name)
             VALUES ('TEST_CODE_600396', 'open', '旧静态产业链');",
        )
        .expect("legacy position fixture");

        create_schema(&mut conn).expect("BR-170 schema");

        let row = diesel::sql_query(
            "SELECT chain_name, chain_assignment_id
               FROM stock_position
              WHERE code = 'TEST_CODE_600396'",
        )
        .get_result::<NullableChainRow>(&mut conn)
        .expect("normalized legacy position");
        assert_eq!(row.chain_name, None);
        assert_eq!(row.chain_assignment_id, None);
    }
}
