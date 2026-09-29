//! Prepared V2 book namespace. CatalogV5 contains no V2 account rows or
//! executable fills; its only event shape is a future cutover genesis.

use diesel::prelude::*;

pub(crate) const STATEMENTS: &[(&str, &str, &str, &str)] = &[
    ("table", "paper_book_v2_account", "paper_book_v2_account", "CREATE TABLE paper_book_v2_account (
        account_id TEXT PRIMARY KEY NOT NULL REFERENCES paper_ledger_account(account_id),
        epoch_id TEXT NOT NULL UNIQUE,
        manifest_hash TEXT NOT NULL CHECK(length(manifest_hash)=64),
        manifest_bytes BLOB NOT NULL CHECK(length(manifest_bytes)>0),
        fee_policy_instance_id TEXT NOT NULL,
        v1_epoch_id TEXT NOT NULL,
        v1_manifest_hash TEXT NOT NULL CHECK(length(v1_manifest_hash)=64),
        v1_head_version INTEGER NOT NULL CHECK(v1_head_version>0),
        v1_head_hash TEXT NOT NULL CHECK(length(v1_head_hash)=64),
        v1_projection_hash TEXT NOT NULL CHECK(length(v1_projection_hash)=64),
        cutover_id TEXT NOT NULL UNIQUE)"),
    ("table", "paper_book_v2_event", "paper_book_v2_event", "CREATE TABLE paper_book_v2_event (
        account_id TEXT NOT NULL REFERENCES paper_book_v2_account(account_id),
        seq INTEGER NOT NULL CHECK(seq=1),
        command_id TEXT NOT NULL,
        previous_hash TEXT NOT NULL,
        event_hash TEXT NOT NULL CHECK(length(event_hash)=64),
        kind TEXT NOT NULL CHECK(kind='Genesis'),
        payload BLOB NOT NULL CHECK(length(payload)>0),
        PRIMARY KEY(account_id,seq), UNIQUE(account_id,command_id))"),
    ("table", "paper_book_v2_head", "paper_book_v2_head", "CREATE TABLE paper_book_v2_head (
        account_id TEXT PRIMARY KEY NOT NULL REFERENCES paper_book_v2_account(account_id),
        version INTEGER NOT NULL CHECK(version=1),
        event_hash TEXT NOT NULL CHECK(length(event_hash)=64),
        projection_bytes BLOB NOT NULL CHECK(length(projection_bytes)>0),
        projection_hash TEXT NOT NULL CHECK(length(projection_hash)=64))"),
    ("trigger", "paper_book_v2_account_no_update", "paper_book_v2_account", "CREATE TRIGGER paper_book_v2_account_no_update BEFORE UPDATE ON paper_book_v2_account
        BEGIN SELECT RAISE(ABORT,'immutable V2 account'); END"),
    ("trigger", "paper_book_v2_account_no_delete", "paper_book_v2_account", "CREATE TRIGGER paper_book_v2_account_no_delete BEFORE DELETE ON paper_book_v2_account
        BEGIN SELECT RAISE(ABORT,'immutable V2 account'); END"),
    ("trigger", "paper_book_v2_account_no_reinsert", "paper_book_v2_account", "CREATE TRIGGER paper_book_v2_account_no_reinsert BEFORE INSERT ON paper_book_v2_account
        WHEN EXISTS(SELECT 1 FROM paper_book_v2_account WHERE account_id=NEW.account_id OR epoch_id=NEW.epoch_id OR cutover_id=NEW.cutover_id)
        BEGIN SELECT RAISE(ABORT,'immutable V2 account'); END"),
    ("trigger", "paper_book_v2_event_no_update", "paper_book_v2_event", "CREATE TRIGGER paper_book_v2_event_no_update BEFORE UPDATE ON paper_book_v2_event
        BEGIN SELECT RAISE(ABORT,'append-only V2 event'); END"),
    ("trigger", "paper_book_v2_event_no_delete", "paper_book_v2_event", "CREATE TRIGGER paper_book_v2_event_no_delete BEFORE DELETE ON paper_book_v2_event
        BEGIN SELECT RAISE(ABORT,'append-only V2 event'); END"),
    ("trigger", "paper_book_v2_event_no_reinsert", "paper_book_v2_event", "CREATE TRIGGER paper_book_v2_event_no_reinsert BEFORE INSERT ON paper_book_v2_event
        WHEN EXISTS(SELECT 1 FROM paper_book_v2_event WHERE account_id=NEW.account_id AND (seq=NEW.seq OR command_id=NEW.command_id))
        BEGIN SELECT RAISE(ABORT,'append-only V2 event'); END"),
    ("trigger", "paper_book_v2_head_no_update", "paper_book_v2_head", "CREATE TRIGGER paper_book_v2_head_no_update BEFORE UPDATE ON paper_book_v2_head
        BEGIN SELECT RAISE(ABORT,'immutable V2 genesis head'); END"),
    ("trigger", "paper_book_v2_head_no_delete", "paper_book_v2_head", "CREATE TRIGGER paper_book_v2_head_no_delete BEFORE DELETE ON paper_book_v2_head
        BEGIN SELECT RAISE(ABORT,'immutable V2 genesis head'); END"),
    ("trigger", "paper_book_v2_head_no_reinsert", "paper_book_v2_head", "CREATE TRIGGER paper_book_v2_head_no_reinsert BEFORE INSERT ON paper_book_v2_head
        WHEN EXISTS(SELECT 1 FROM paper_book_v2_head WHERE account_id=NEW.account_id)
        BEGIN SELECT RAISE(ABORT,'immutable V2 genesis head'); END"),
];

pub(crate) fn create_schema(conn: &mut SqliteConnection) -> QueryResult<()> {
    for (_, _, _, statement) in STATEMENTS {
        diesel::sql_query(*statement).execute(conn)?;
    }
    Ok(())
}
