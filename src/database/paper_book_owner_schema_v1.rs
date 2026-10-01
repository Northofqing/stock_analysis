//! CatalogV4 account-wide V1 owner fence. The account has no V2 seed or fill
//! authority here; V4 can only record the existing V1 owner.

#[cfg(test)]
use crate::performance::fee_policy::AShareFeePolicyV2;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};

pub(crate) const CATALOG_GENERATION: i64 = 4;
const APPLICATION_ID: i64 = 1_398_035_265;

pub(crate) const STATEMENTS: &[(&str, &str, &str, &str)] = &[
    (
        "table",
        "paper_book_owner_v1",
        "paper_book_owner_v1",
        "CREATE TABLE paper_book_owner_v1 (
        account_id TEXT PRIMARY KEY,
        active_generation INTEGER NOT NULL CHECK(active_generation=1),
        active_epoch_id TEXT NOT NULL,
        active_manifest_hash TEXT NOT NULL CHECK(length(active_manifest_hash)=64),
        owner_revision INTEGER NOT NULL CHECK(owner_revision=1))",
    ),
    (
        "trigger",
        "paper_book_owner_v1_no_update",
        "paper_book_owner_v1",
        "CREATE TRIGGER paper_book_owner_v1_no_update BEFORE UPDATE ON paper_book_owner_v1
        BEGIN SELECT RAISE(ABORT,'immutable V1 owner; cutover not installed'); END",
    ),
    (
        "trigger",
        "paper_book_owner_v1_no_delete",
        "paper_book_owner_v1",
        "CREATE TRIGGER paper_book_owner_v1_no_delete BEFORE DELETE ON paper_book_owner_v1
        BEGIN SELECT RAISE(ABORT,'immutable V1 owner; cutover not installed'); END",
    ),
    (
        "trigger",
        "paper_book_owner_v1_no_reinsert",
        "paper_book_owner_v1",
        "CREATE TRIGGER paper_book_owner_v1_no_reinsert BEFORE INSERT ON paper_book_owner_v1
        WHEN EXISTS(SELECT 1 FROM paper_book_owner_v1 WHERE account_id=NEW.account_id)
          OR NOT EXISTS(SELECT 1 FROM paper_ledger_account a WHERE a.account_id=NEW.account_id
              AND a.epoch_id=NEW.active_epoch_id AND a.manifest_hash=NEW.active_manifest_hash)
        BEGIN SELECT RAISE(ABORT,'immutable V1 owner; cutover not installed'); END",
    ),
];

/// New CatalogV4 objects only: frozen PaperLedgerV1 DDL is not modified.
/// These are included in the whole-catalog registry and V1 local reader's V4
/// expected object list because their `tbl_name` points at V1 tables.
pub(crate) const V1_GUARD_STATEMENTS: &[(&str, &str, &str, &str)] = &[
    (
        "trigger",
        "paper_book_owner_v1_account_insert",
        "paper_ledger_account",
        "CREATE TRIGGER paper_book_owner_v1_account_insert BEFORE INSERT ON paper_ledger_account
        WHEN EXISTS(SELECT 1 FROM paper_ledger_account
              WHERE account_id=NEW.account_id OR epoch_id=NEW.epoch_id)
          OR NOT EXISTS(SELECT 1 FROM paper_book_owner_v1
              WHERE account_id=NEW.account_id AND active_generation=1
                AND active_epoch_id=NEW.epoch_id AND active_manifest_hash=NEW.manifest_hash)
        BEGIN SELECT RAISE(ABORT,'V1 account owner mismatch'); END",
    ),
    (
        "trigger",
        "paper_book_owner_v1_event_insert",
        "paper_ledger_event",
        "CREATE TRIGGER paper_book_owner_v1_event_insert BEFORE INSERT ON paper_ledger_event
        WHEN NOT EXISTS(SELECT 1 FROM paper_book_owner_v1 o JOIN paper_ledger_account a
              ON a.account_id=o.account_id WHERE o.account_id=NEW.account_id
                AND o.active_generation=1 AND o.active_epoch_id=a.epoch_id
                AND o.active_manifest_hash=a.manifest_hash)
          OR EXISTS(SELECT 1 FROM paper_ledger_event
              WHERE (account_id=NEW.account_id AND (seq=NEW.seq OR command_id=NEW.command_id
                  OR (NEW.is_terminal=1 AND NEW.business_plan_id IS NOT NULL
                      AND is_terminal=1 AND business_plan_id=NEW.business_plan_id)))
                OR (NEW.paper_trade_id IS NOT NULL AND paper_trade_id=NEW.paper_trade_id)
                OR (NEW.order_audit_id IS NOT NULL AND order_audit_id=NEW.order_audit_id))
        BEGIN SELECT RAISE(ABORT,'V1 event owner mismatch or replacement'); END",
    ),
    (
        "trigger",
        "paper_book_owner_v1_head_insert",
        "paper_ledger_head",
        "CREATE TRIGGER paper_book_owner_v1_head_insert BEFORE INSERT ON paper_ledger_head
        WHEN NOT EXISTS(SELECT 1 FROM paper_book_owner_v1 o JOIN paper_ledger_account a
              ON a.account_id=o.account_id WHERE o.account_id=NEW.account_id
                AND o.active_generation=1 AND o.active_epoch_id=a.epoch_id
                AND o.active_manifest_hash=a.manifest_hash)
          OR EXISTS(SELECT 1 FROM paper_ledger_head WHERE account_id=NEW.account_id)
        BEGIN SELECT RAISE(ABORT,'V1 head owner mismatch or replacement'); END",
    ),
    (
        "trigger",
        "paper_book_owner_v1_head_update",
        "paper_ledger_head",
        "CREATE TRIGGER paper_book_owner_v1_head_update BEFORE UPDATE ON paper_ledger_head
        WHEN NEW.account_id!=OLD.account_id OR NOT EXISTS(
            SELECT 1 FROM paper_book_owner_v1 o JOIN paper_ledger_account a
              ON a.account_id=o.account_id WHERE o.account_id=NEW.account_id
                AND o.active_generation=1 AND o.active_epoch_id=a.epoch_id
                AND o.active_manifest_hash=a.manifest_hash)
        BEGIN SELECT RAISE(ABORT,'V1 head owner mismatch'); END",
    ),
];

#[derive(Debug, thiserror::Error)]
pub(crate) enum PaperBookOwnerError {
    #[error("paper book owner requires CatalogV4/V5 or an unchanged legacy V1 catalog")]
    WrongDatabaseIdentity,
    #[error("paper book owner or fee namespace is missing, altered, or has extra objects")]
    CatalogMismatch,
    #[error("paper book V1 account owner does not match the bound epoch and manifest")]
    InactiveOwner,
    #[cfg(test)]
    #[error("CatalogV4 installation requires an isolated TEST_CODE database")]
    NotIsolated,
    #[error(transparent)]
    FeeManifest(#[from] super::paper_book_v2_schema::StagedPaperBookV2Error),
    #[error(transparent)]
    Database(#[from] diesel::result::Error),
    #[error(transparent)]
    Connection(#[from] diesel::ConnectionError),
    #[cfg(test)]
    #[error("TEST_CODE interrupted CatalogV4 installation")]
    InjectedFailure,
}

#[derive(QueryableByName)]
struct Identity {
    #[diesel(sql_type = BigInt)]
    application_id: i64,
    #[diesel(sql_type = BigInt)]
    user_version: i64,
}

#[derive(Debug, Eq, PartialEq, QueryableByName)]
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

#[derive(QueryableByName)]
struct OwnerRow {
    #[diesel(sql_type = BigInt)]
    active_generation: i64,
    #[diesel(sql_type = Text)]
    active_epoch_id: String,
    #[diesel(sql_type = Text)]
    active_manifest_hash: String,
    #[diesel(sql_type = BigInt)]
    owner_revision: i64,
}

fn identity(conn: &mut SqliteConnection) -> Result<Identity, PaperBookOwnerError> {
    Ok(diesel::sql_query(
        "SELECT application_id,user_version FROM pragma_application_id(),pragma_user_version()",
    )
    .get_result(conn)?)
}

fn owner_objects(conn: &mut SqliteConnection) -> Result<Vec<CatalogObject>, PaperBookOwnerError> {
    Ok(diesel::sql_query(
        "SELECT 'main' AS namespace,type AS kind,name,tbl_name AS table_name,sql
         FROM main.sqlite_master
         WHERE (name GLOB 'paper_book_owner_*' OR tbl_name GLOB 'paper_book_owner_*') AND sql IS NOT NULL
         UNION ALL
         SELECT 'temp' AS namespace,type AS kind,name,tbl_name AS table_name,sql
         FROM temp.sqlite_master
         WHERE (name GLOB 'paper_book_owner_*' OR tbl_name GLOB 'paper_book_owner_*') AND sql IS NOT NULL
         ORDER BY namespace,kind,name,table_name,sql",
    )
    .load(conn)?)
}

pub(crate) fn create_schema(conn: &mut SqliteConnection) -> Result<(), PaperBookOwnerError> {
    for (_, _, _, statement) in STATEMENTS {
        diesel::sql_query(*statement).execute(conn)?;
    }
    for (_, _, _, statement) in V1_GUARD_STATEMENTS {
        diesel::sql_query(*statement).execute(conn)?;
    }
    Ok(())
}

/// Structural V4 proof for historical V1 reads. This checks the fee row's
/// self-consistency, not approval of its economic policy or any active V2 owner.
pub(crate) fn verify_catalog_v4_on(conn: &mut SqliteConnection) -> Result<(), PaperBookOwnerError> {
    let found = identity(conn)?;
    if found.application_id != APPLICATION_ID || found.user_version != CATALOG_GENERATION {
        return Err(PaperBookOwnerError::WrongDatabaseIdentity);
    }
    let mut reference = SqliteConnection::establish(":memory:")?;
    super::paper_ledger_schema_v1::create_schema(&mut reference)?;
    create_schema(&mut reference)?;
    if owner_objects(conn)? != owner_objects(&mut reference)?
        || v1_objects(conn)? != v1_objects(&mut reference)?
        || !super::daily_change_review_schema_v1::is_present(conn)
            .map_err(|_| PaperBookOwnerError::CatalogMismatch)?
    {
        return Err(PaperBookOwnerError::CatalogMismatch);
    }
    super::paper_book_v2_schema::verify_v4_manifest_on(conn)?;
    verify_owner_rows(conn)?;
    Ok(())
}

fn verify_owner_rows(conn: &mut SqliteConnection) -> Result<(), PaperBookOwnerError> {
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        value: i64,
    }
    let missing_or_mismatched = diesel::sql_query(
        "SELECT COUNT(*) AS value FROM main.paper_ledger_account a
         LEFT JOIN main.paper_book_owner_v1 o ON o.account_id=a.account_id
         WHERE o.account_id IS NULL OR o.active_generation!=1 OR o.owner_revision!=1
           OR o.active_epoch_id!=a.epoch_id OR o.active_manifest_hash!=a.manifest_hash",
    )
    .get_result::<Count>(conn)?
    .value;
    let orphan_or_mismatched = diesel::sql_query(
        "SELECT COUNT(*) AS value FROM main.paper_book_owner_v1 o
         LEFT JOIN main.paper_ledger_account a ON a.account_id=o.account_id
         WHERE a.account_id IS NULL OR o.active_generation!=1 OR o.owner_revision!=1
           OR o.active_epoch_id!=a.epoch_id OR o.active_manifest_hash!=a.manifest_hash",
    )
    .get_result::<Count>(conn)?
    .value;
    if missing_or_mismatched != 0 || orphan_or_mismatched != 0 {
        return Err(PaperBookOwnerError::InactiveOwner);
    }
    Ok(())
}

/// A same-transaction V1 write fence. V1/V2/V3 retain their previous V1 behavior;
/// any partial V4 namespace or unknown generation fails closed.
pub(crate) fn require_v1_owner_on(
    conn: &mut SqliteConnection,
    account_id: &str,
    epoch_id: &str,
    manifest_hash: &str,
) -> Result<(), PaperBookOwnerError> {
    let found = identity(conn)?;
    if found.application_id == APPLICATION_ID
        && found.user_version == super::paper_book_owner_schema_v2::CATALOG_GENERATION
    {
        return super::paper_book_owner_schema_v2::require_v1_owner_on(
            conn,
            account_id,
            epoch_id,
            manifest_hash,
        );
    }
    if found.application_id == APPLICATION_ID && matches!(found.user_version, 1 | 2 | 3) {
        let fee_namespace_ok = if !has_v2_objects(conn)? {
            true
        } else if found.user_version == 2 {
            super::paper_book_v2_schema::verify_inactive_staged_manifest_on(conn).is_ok()
        } else {
            false
        };
        return if owner_objects(conn)?.is_empty() && fee_namespace_ok {
            Ok(())
        } else {
            Err(PaperBookOwnerError::CatalogMismatch)
        };
    }
    #[cfg(test)]
    if found.application_id == 0
        && found.user_version == 0
        && owner_objects(conn)?.is_empty()
        && !has_v2_objects(conn)?
    {
        return Ok(());
    }
    verify_catalog_v4_on(conn)?;
    let owner = diesel::sql_query(
        "SELECT active_generation,active_epoch_id,active_manifest_hash,owner_revision
         FROM main.paper_book_owner_v1 WHERE account_id=?",
    )
    .bind::<Text, _>(account_id)
    .get_result::<OwnerRow>(conn)
    .optional()?;
    if !matches!(owner, Some(owner) if owner.active_generation == 1
        && owner.active_epoch_id == epoch_id
        && owner.active_manifest_hash == manifest_hash
        && owner.owner_revision == 1)
    {
        return Err(PaperBookOwnerError::InactiveOwner);
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn require_isolated(conn: &mut SqliteConnection) -> Result<(), PaperBookOwnerError> {
    #[derive(QueryableByName)]
    struct MainFile {
        #[diesel(sql_type = Text)]
        file: String,
    }
    let file = diesel::sql_query("SELECT file FROM pragma_database_list() WHERE name='main'")
        .get_result::<MainFile>(conn)?
        .file;
    if file.is_empty() {
        return Ok(());
    }
    let path = std::path::Path::new(&file);
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("TEST_CODE_") && name.ends_with(".db"));
    let under_temp = path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .zip(std::env::temp_dir().canonicalize().ok())
        .is_some_and(|(parent, temp)| parent.starts_with(temp));
    if named && under_temp {
        Ok(())
    } else {
        Err(PaperBookOwnerError::NotIsolated)
    }
}

#[cfg(test)]
pub(crate) fn install_catalog_v4_for_isolated_test(
    conn: &mut SqliteConnection,
    policy: &AShareFeePolicyV2,
) -> Result<(), PaperBookOwnerError> {
    install_catalog_v4_with_fault(conn, policy, false)
}

#[cfg(test)]
fn install_catalog_v4_with_fault(
    conn: &mut SqliteConnection,
    policy: &AShareFeePolicyV2,
    fail_after_schema: bool,
) -> Result<(), PaperBookOwnerError> {
    conn.immediate_transaction(|conn| {
        require_isolated(conn)?;
        let found = identity(conn)?;
        if found.application_id != APPLICATION_ID || found.user_version != 3 {
            return Err(PaperBookOwnerError::WrongDatabaseIdentity);
        }
        if !super::daily_change_review_schema_v1::is_present(conn)
            .map_err(|_| PaperBookOwnerError::CatalogMismatch)?
            || !owner_objects(conn)?.is_empty()
        {
            return Err(PaperBookOwnerError::CatalogMismatch);
        }
        let mut reference = SqliteConnection::establish(":memory:")?;
        super::paper_ledger_schema_v1::create_schema(&mut reference)?;
        if v1_objects(conn)? != v1_objects(&mut reference)? || has_v2_objects(conn)? {
            return Err(PaperBookOwnerError::CatalogMismatch);
        }
        let accounts = v1_accounts(conn)?;
        super::paper_book_v2_schema::create_schema(conn)?;
        super::paper_book_v2_schema::insert_policy_for_isolated_test(conn, policy)?;
        diesel::sql_query(STATEMENTS[0].3).execute(conn)?;
        for account in accounts {
            diesel::sql_query("INSERT INTO paper_book_owner_v1(account_id,active_generation,active_epoch_id,active_manifest_hash,owner_revision) VALUES (?,1,?,?,1)")
                .bind::<Text, _>(account.account_id)
                .bind::<Text, _>(account.epoch_id)
                .bind::<Text, _>(account.manifest_hash)
                .execute(conn)?;
        }
        for (_, _, _, statement) in &STATEMENTS[1..] {
            diesel::sql_query(*statement).execute(conn)?;
        }
        for (_, _, _, statement) in V1_GUARD_STATEMENTS {
            diesel::sql_query(*statement).execute(conn)?;
        }
        diesel::sql_query("PRAGMA user_version=4").execute(conn)?;
        if fail_after_schema {
            return Err(PaperBookOwnerError::InjectedFailure);
        }
        verify_catalog_v4_on(conn)?;
        Ok(())
    })
}

#[cfg(test)]
#[derive(QueryableByName)]
struct AccountForBackfill {
    #[diesel(sql_type = Text)]
    account_id: String,
    #[diesel(sql_type = Text)]
    epoch_id: String,
    #[diesel(sql_type = Text)]
    manifest_hash: String,
}

#[cfg(test)]
fn v1_accounts(
    conn: &mut SqliteConnection,
) -> Result<Vec<AccountForBackfill>, PaperBookOwnerError> {
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        value: i64,
    }
    let missing = diesel::sql_query(
        "SELECT COUNT(*) AS value FROM paper_ledger_account a
         LEFT JOIN paper_ledger_head h ON h.account_id=a.account_id
         WHERE h.account_id IS NULL OR h.version<1 OR h.event_hash=''",
    )
    .get_result::<Count>(conn)?
    .value;
    if missing != 0 {
        return Err(PaperBookOwnerError::CatalogMismatch);
    }
    Ok(diesel::sql_query(
        "SELECT account_id,epoch_id,manifest_hash FROM paper_ledger_account ORDER BY account_id",
    )
    .load(conn)?)
}

fn v1_objects(conn: &mut SqliteConnection) -> Result<Vec<CatalogObject>, PaperBookOwnerError> {
    Ok(diesel::sql_query(
        "SELECT 'main' AS namespace,type AS kind,name,tbl_name AS table_name,sql
         FROM main.sqlite_master WHERE (name GLOB 'paper_ledger_*' OR tbl_name GLOB 'paper_ledger_*') AND sql IS NOT NULL
         UNION ALL SELECT 'temp',type,name,tbl_name,sql FROM temp.sqlite_master
         WHERE (name GLOB 'paper_ledger_*' OR tbl_name GLOB 'paper_ledger_*') AND sql IS NOT NULL
         ORDER BY namespace,kind,name,table_name,sql",
    )
    .load(conn)?)
}

fn has_v2_objects(conn: &mut SqliteConnection) -> Result<bool, PaperBookOwnerError> {
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        value: i64,
    }
    let count = diesel::sql_query(
        "SELECT (SELECT COUNT(*) FROM main.sqlite_master WHERE name GLOB 'paper_book_v2_*' OR tbl_name GLOB 'paper_book_v2_*')
              + (SELECT COUNT(*) FROM temp.sqlite_master WHERE name GLOB 'paper_book_v2_*' OR tbl_name GLOB 'paper_book_v2_*') AS value",
    )
    .get_result::<Count>(conn)?
    .value;
    Ok(count != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;

    #[derive(QueryableByName)]
    struct TextValue {
        #[diesel(sql_type = Text)]
        value: String,
    }

    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        value: i64,
    }

    fn text_value(conn: &mut SqliteConnection, sql: &str) -> String {
        diesel::sql_query(sql)
            .get_result::<TextValue>(conn)
            .unwrap()
            .value
    }

    fn count(conn: &mut SqliteConnection, sql: &str) -> i64 {
        diesel::sql_query(sql)
            .get_result::<Count>(conn)
            .unwrap()
            .value
    }

    fn v3_with_two_accounts() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        super::super::paper_ledger_schema_v1::create_schema(&mut conn).unwrap();
        super::super::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
        conn.batch_execute(
            "INSERT INTO paper_ledger_account VALUES
                ('acct-a','epoch-a','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','manifest-a','micro-cny-half-up-v1','lot-rates-v1'),
                ('acct-b','epoch-b','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','manifest-b','micro-cny-half-up-v1','lot-rates-v1');
             INSERT INTO paper_ledger_event
                (account_id,seq,command_id,previous_hash,event_hash,payload,business_plan_id,intent_hash,is_terminal,paper_trade_id,order_audit_id)
                VALUES ('acct-a',1,'command-a','genesis','event-a','event-payload-a','plan-a','intent-a',1,7,9),
                       ('acct-b',1,'command-b','genesis','event-b','event-payload-b',NULL,NULL,0,NULL,NULL);
             INSERT INTO paper_ledger_head VALUES
                ('acct-a',1,'event-a','projection-a','projection-hash-a'),
                ('acct-b',1,'event-b','projection-b','projection-hash-b');
             PRAGMA application_id=1398035265; PRAGMA user_version=3",
        )
        .unwrap();
        conn
    }

    fn installed() -> SqliteConnection {
        let mut conn = v3_with_two_accounts();
        install_catalog_v4_for_isolated_test(
            &mut conn,
            &AShareFeePolicyV2::fixed_compatibility_assumption(),
        )
        .unwrap();
        conn
    }

    #[test]
    fn isolated_upgrade_backfills_exact_owners_and_preserves_v1_rows() {
        let mut conn = v3_with_two_accounts();
        let before_account = text_value(
            &mut conn,
            "SELECT manifest_bytes AS value FROM paper_ledger_account WHERE account_id='acct-a'",
        );
        let before_event = text_value(
            &mut conn,
            "SELECT payload AS value FROM paper_ledger_event WHERE account_id='acct-a' AND seq=1",
        );
        let before_head = text_value(
            &mut conn,
            "SELECT projection_bytes AS value FROM paper_ledger_head WHERE account_id='acct-a'",
        );
        install_catalog_v4_for_isolated_test(
            &mut conn,
            &AShareFeePolicyV2::fixed_compatibility_assumption(),
        )
        .unwrap();
        verify_catalog_v4_on(&mut conn).unwrap();
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS value FROM paper_book_owner_v1"
            ),
            2
        );
        assert_eq!(before_account, text_value(&mut conn, "SELECT manifest_bytes AS value FROM paper_ledger_account WHERE account_id='acct-a'"));
        assert_eq!(before_event, text_value(&mut conn, "SELECT payload AS value FROM paper_ledger_event WHERE account_id='acct-a' AND seq=1"));
        assert_eq!(
            before_head,
            text_value(
                &mut conn,
                "SELECT projection_bytes AS value FROM paper_ledger_head WHERE account_id='acct-a'"
            )
        );
        require_v1_owner_on(&mut conn, "acct-a", "epoch-a", &"a".repeat(64)).unwrap();
        assert!(matches!(
            require_v1_owner_on(&mut conn, "acct-a", "epoch-other", &"a".repeat(64)),
            Err(PaperBookOwnerError::InactiveOwner)
        ));
    }

    #[test]
    fn owner_insert_cannot_precede_a_new_v1_account_seed() {
        let mut conn = installed();
        let before_owners = count(
            &mut conn,
            "SELECT COUNT(*) AS value FROM paper_book_owner_v1",
        );
        let before_accounts = count(
            &mut conn,
            "SELECT COUNT(*) AS value FROM paper_ledger_account",
        );
        assert!(diesel::sql_query("INSERT INTO paper_book_owner_v1 VALUES ('acct-new',1,'epoch-new','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',1)")
            .execute(&mut conn).is_err());
        assert!(diesel::sql_query("INSERT INTO paper_ledger_account VALUES ('acct-new','epoch-new','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc','new','micro-cny-half-up-v1','lot-rates-v1')")
            .execute(&mut conn).is_err());
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS value FROM paper_book_owner_v1"
            ),
            before_owners
        );
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS value FROM paper_ledger_account"
            ),
            before_accounts
        );
        verify_catalog_v4_on(&mut conn).unwrap();
    }

    #[test]
    fn raw_replace_cannot_remove_or_change_v1_account_event_or_head() {
        let mut conn = installed();
        assert!(diesel::sql_query("INSERT OR REPLACE INTO paper_ledger_account VALUES ('acct-a','epoch-a','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','replaced','micro-cny-half-up-v1','lot-rates-v1')")
            .execute(&mut conn).is_err());
        assert!(diesel::sql_query("INSERT OR REPLACE INTO paper_ledger_account VALUES ('acct-new','epoch-b','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','replaced','micro-cny-half-up-v1','lot-rates-v1')")
            .execute(&mut conn).is_err());
        assert_eq!(text_value(&mut conn, "SELECT manifest_bytes AS value FROM paper_ledger_account WHERE account_id='acct-a'"), "manifest-a");
        assert_eq!(text_value(&mut conn, "SELECT manifest_bytes AS value FROM paper_ledger_account WHERE account_id='acct-b'"), "manifest-b");

        for values in [
            "('acct-a',1,'new-command','event-a','event-a2','changed',NULL,0,NULL,NULL)",
            "('acct-a',2,'command-a','event-a','event-a2','changed',NULL,0,NULL,NULL)",
            "('acct-a',2,'new-command','event-a','event-a2','changed',NULL,0,7,NULL)",
            "('acct-a',2,'new-command','event-a','event-a2','changed',NULL,0,NULL,9)",
            "('acct-a',2,'new-command','event-a','event-a2','changed','plan-a',1,NULL,NULL)",
        ] {
            let sql = format!("INSERT OR REPLACE INTO paper_ledger_event
                (account_id,seq,command_id,previous_hash,event_hash,payload,business_plan_id,is_terminal,paper_trade_id,order_audit_id)
                VALUES {values}");
            assert!(diesel::sql_query(sql).execute(&mut conn).is_err());
            assert_eq!(
                count(
                    &mut conn,
                    "SELECT COUNT(*) AS value FROM paper_ledger_event WHERE account_id='acct-a'"
                ),
                1
            );
            assert_eq!(text_value(&mut conn, "SELECT payload AS value FROM paper_ledger_event WHERE account_id='acct-a' AND seq=1"), "event-payload-a");
        }
        assert!(diesel::sql_query("INSERT OR REPLACE INTO paper_ledger_head VALUES ('acct-a',9,'changed','replaced','changed')")
            .execute(&mut conn).is_err());
        assert_eq!(
            text_value(
                &mut conn,
                "SELECT projection_bytes AS value FROM paper_ledger_head WHERE account_id='acct-a'"
            ),
            "projection-a"
        );
        assert_eq!(
            count(&mut conn, "SELECT COUNT(*) AS value FROM paper_ledger_head"),
            2
        );
        verify_catalog_v4_on(&mut conn).unwrap();
    }

    #[test]
    fn same_owner_can_append_event_and_advance_head() {
        let mut conn = installed();
        conn.batch_execute("INSERT INTO paper_ledger_event
            (account_id,seq,command_id,previous_hash,event_hash,payload,is_terminal)
            VALUES ('acct-a',2,'command-a2','event-a','event-a2','event-payload-a2',0);
            UPDATE paper_ledger_head SET version=2,event_hash='event-a2',projection_bytes='projection-a2',projection_hash='projection-hash-a2' WHERE account_id='acct-a'")
            .unwrap();
        assert_eq!(
            text_value(
                &mut conn,
                "SELECT projection_bytes AS value FROM paper_ledger_head WHERE account_id='acct-a'"
            ),
            "projection-a2"
        );
        verify_catalog_v4_on(&mut conn).unwrap();
    }

    #[test]
    fn verifier_rejects_extra_missing_or_altered_namespaces_and_owner_rows() {
        let mut conn = installed();
        conn.batch_execute("CREATE TABLE paper_book_owner_v1_shadow (value TEXT)")
            .unwrap();
        assert!(matches!(
            verify_catalog_v4_on(&mut conn),
            Err(PaperBookOwnerError::CatalogMismatch)
        ));
        conn.batch_execute(
            "DROP TABLE paper_book_owner_v1_shadow; DROP TRIGGER paper_ledger_event_no_delete",
        )
        .unwrap();
        assert!(matches!(
            verify_catalog_v4_on(&mut conn),
            Err(PaperBookOwnerError::CatalogMismatch)
        ));
        conn.batch_execute(super::super::paper_ledger_schema_v1::STATEMENTS[7].3)
            .unwrap();
        conn.batch_execute("DROP TRIGGER daily_change_review_no_delete")
            .unwrap();
        assert!(matches!(
            verify_catalog_v4_on(&mut conn),
            Err(PaperBookOwnerError::CatalogMismatch)
        ));
        conn.batch_execute(super::super::daily_change_review_schema_v1::STATEMENTS[5].3)
            .unwrap();
        conn.batch_execute("DROP TRIGGER paper_book_v2_fee_manifest_no_delete")
            .unwrap();
        assert!(verify_catalog_v4_on(&mut conn).is_err());
        conn.batch_execute(super::super::paper_book_v2_schema::STATEMENTS[2].3)
            .unwrap();
        conn.batch_execute("DROP TRIGGER paper_book_owner_v1_no_delete; DELETE FROM paper_book_owner_v1 WHERE account_id='acct-b'").unwrap();
        conn.batch_execute(STATEMENTS[2].3).unwrap();
        assert!(matches!(
            verify_catalog_v4_on(&mut conn),
            Err(PaperBookOwnerError::InactiveOwner)
        ));
    }

    #[test]
    fn failed_upgrade_rolls_back_schema_rows_and_generation() {
        let mut conn = v3_with_two_accounts();
        let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
        assert!(matches!(
            install_catalog_v4_with_fault(&mut conn, &policy, true),
            Err(PaperBookOwnerError::InjectedFailure)
        ));
        assert!(owner_objects(&mut conn).unwrap().is_empty());
        assert!(!has_v2_objects(&mut conn).unwrap());
        assert_eq!(identity(&mut conn).unwrap().user_version, 3);
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS value FROM paper_ledger_account"
            ),
            2
        );
        install_catalog_v4_for_isolated_test(&mut conn, &policy).unwrap();
    }

    #[test]
    fn upgrade_rejects_a_file_outside_the_test_database_namespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ordinary_database.db");
        let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
        assert!(matches!(
            install_catalog_v4_for_isolated_test(&mut conn, &policy),
            Err(PaperBookOwnerError::NotIsolated)
        ));
        assert!(owner_objects(&mut conn).unwrap().is_empty());
        assert!(!has_v2_objects(&mut conn).unwrap());
    }

    #[test]
    fn legacy_v1_owner_check_allows_exact_generation_one_but_rejects_partial_v4() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        super::super::paper_ledger_schema_v1::create_schema(&mut conn).unwrap();
        conn.batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=1")
            .unwrap();
        require_v1_owner_on(
            &mut conn,
            "legacy-account",
            "legacy-epoch",
            "legacy-manifest",
        )
        .unwrap();
        conn.batch_execute(super::super::paper_book_v2_schema::STATEMENTS[0].3)
            .unwrap();
        assert!(matches!(
            require_v1_owner_on(
                &mut conn,
                "legacy-account",
                "legacy-epoch",
                "legacy-manifest"
            ),
            Err(PaperBookOwnerError::CatalogMismatch)
        ));
        conn.batch_execute("DROP TABLE paper_book_v2_fee_manifest")
            .unwrap();
        conn.batch_execute(STATEMENTS[0].3).unwrap();
        assert!(matches!(
            require_v1_owner_on(
                &mut conn,
                "legacy-account",
                "legacy-epoch",
                "legacy-manifest"
            ),
            Err(PaperBookOwnerError::CatalogMismatch)
        ));
    }

    #[test]
    fn legacy_v2_owner_check_preserves_complete_inactive_fee_staging() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        super::super::paper_ledger_schema_v1::create_schema(&mut conn).unwrap();
        conn.batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=2")
            .unwrap();
        let policy = AShareFeePolicyV2::fixed_compatibility_assumption();
        super::super::paper_book_v2_schema::stage_for_isolated_test(&mut conn, &policy).unwrap();
        require_v1_owner_on(
            &mut conn,
            "legacy-account",
            "legacy-epoch",
            "legacy-manifest",
        )
        .unwrap();
        conn.batch_execute("DROP TRIGGER paper_book_v2_fee_manifest_no_delete")
            .unwrap();
        assert!(matches!(
            require_v1_owner_on(
                &mut conn,
                "legacy-account",
                "legacy-epoch",
                "legacy-manifest"
            ),
            Err(PaperBookOwnerError::CatalogMismatch)
        ));
    }
}
