//! Frozen PaperLedgerV1 extension, shared by schema creation and CatalogV2.
//! Changes require a new explicit generation, never rewriting these DDL bytes.
pub(crate) const CATALOG_GENERATION: i64 = 2;
pub(crate) const STATEMENTS: &[(&str, &str, &str, &str)] = &[
    ("table","paper_ledger_account","paper_ledger_account", "CREATE TABLE IF NOT EXISTS paper_ledger_account (
      account_id TEXT PRIMARY KEY,epoch_id TEXT NOT NULL UNIQUE,manifest_hash TEXT NOT NULL,manifest_bytes TEXT NOT NULL,
      money_model TEXT NOT NULL CHECK(money_model='micro-cny-half-up-v1'),fee_model TEXT NOT NULL CHECK(fee_model='lot-rates-v1'))"),
    ("table","paper_ledger_event","paper_ledger_event", "CREATE TABLE IF NOT EXISTS paper_ledger_event (
      account_id TEXT NOT NULL REFERENCES paper_ledger_account(account_id), seq INTEGER NOT NULL CHECK(seq>0),command_id TEXT NOT NULL,
      previous_hash TEXT NOT NULL,event_hash TEXT NOT NULL,payload TEXT NOT NULL,
      business_plan_id TEXT,intent_hash TEXT,is_terminal INTEGER NOT NULL DEFAULT 0 CHECK(is_terminal IN (0,1)),
      paper_trade_id INTEGER UNIQUE REFERENCES paper_trades(id),order_audit_id INTEGER UNIQUE REFERENCES order_audit(id),
      PRIMARY KEY(account_id,seq),UNIQUE(account_id,command_id))"),
    ("index","paper_ledger_terminal_plan","paper_ledger_event", "CREATE UNIQUE INDEX IF NOT EXISTS paper_ledger_terminal_plan ON paper_ledger_event(account_id,business_plan_id) WHERE is_terminal=1"),
    ("table","paper_ledger_head","paper_ledger_head", "CREATE TABLE IF NOT EXISTS paper_ledger_head (
      account_id TEXT PRIMARY KEY REFERENCES paper_ledger_account(account_id),version INTEGER NOT NULL,event_hash TEXT NOT NULL,
      projection_bytes TEXT NOT NULL,projection_hash TEXT NOT NULL)"),
    ("trigger","paper_ledger_account_no_update","paper_ledger_account", "CREATE TRIGGER IF NOT EXISTS paper_ledger_account_no_update BEFORE UPDATE ON paper_ledger_account BEGIN SELECT RAISE(ABORT,'immutable paper account'); END"),
    ("trigger","paper_ledger_account_no_delete","paper_ledger_account", "CREATE TRIGGER IF NOT EXISTS paper_ledger_account_no_delete BEFORE DELETE ON paper_ledger_account BEGIN SELECT RAISE(ABORT,'immutable paper account'); END"),
    ("trigger","paper_ledger_event_no_update","paper_ledger_event", "CREATE TRIGGER IF NOT EXISTS paper_ledger_event_no_update BEFORE UPDATE ON paper_ledger_event BEGIN SELECT RAISE(ABORT,'append-only paper event'); END"),
    ("trigger","paper_ledger_event_no_delete","paper_ledger_event", "CREATE TRIGGER IF NOT EXISTS paper_ledger_event_no_delete BEFORE DELETE ON paper_ledger_event BEGIN SELECT RAISE(ABORT,'append-only paper event'); END"),
];

pub(crate) fn create_schema(conn: &mut diesel::SqliteConnection) -> diesel::QueryResult<()> {
    use diesel::connection::SimpleConnection;
    for (_, _, _, sql) in STATEMENTS {
        conn.batch_execute(sql)?;
    }
    Ok(())
}
