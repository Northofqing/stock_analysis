//! CatalogV5 prepared owner namespace. Installation is test-only and leaves
//! every account V1Active. V2 genesis/cutover is a separate transaction owner.

use super::paper_book_owner_schema_v1::PaperBookOwnerError;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};

pub(crate) const CATALOG_GENERATION: i64 = 5;
const APPLICATION_ID: i64 = 1_398_035_265;
pub(crate) const OWNER_TRANSITION_GUARD_DDL: &str =
    "CREATE TRIGGER paper_book_owner_v2_transition BEFORE UPDATE ON paper_book_owner_v2
        BEGIN SELECT RAISE(ABORT,'owner transition requires isolated verified cutover'); END";

pub(crate) const OWNER_STATEMENTS: &[(&str, &str, &str, &str)] = &[
    ("table", "paper_book_owner_v2", "paper_book_owner_v2", "CREATE TABLE paper_book_owner_v2 (
        account_id TEXT PRIMARY KEY NOT NULL,
        active_generation INTEGER NOT NULL CHECK(active_generation IN (1,2)),
        active_epoch_id TEXT NOT NULL UNIQUE,
        active_manifest_hash TEXT NOT NULL CHECK(length(active_manifest_hash)=64),
        owner_revision INTEGER NOT NULL CHECK(owner_revision IN (1,2)),
        cutover_id TEXT UNIQUE,
        CHECK((active_generation=1 AND owner_revision=1 AND cutover_id IS NULL)
           OR (active_generation=2 AND owner_revision=2 AND cutover_id IS NOT NULL AND length(cutover_id)>0)))"),
    ("trigger", "paper_book_owner_v2_no_delete", "paper_book_owner_v2", "CREATE TRIGGER paper_book_owner_v2_no_delete BEFORE DELETE ON paper_book_owner_v2
        BEGIN SELECT RAISE(ABORT,'immutable paper book owner'); END"),
    ("trigger", "paper_book_owner_v2_no_reinsert", "paper_book_owner_v2", "CREATE TRIGGER paper_book_owner_v2_no_reinsert BEFORE INSERT ON paper_book_owner_v2
        WHEN NEW.active_generation!=1 OR NEW.owner_revision!=1 OR NEW.cutover_id IS NOT NULL
          OR EXISTS(SELECT 1 FROM paper_book_owner_v2 WHERE account_id=NEW.account_id
              OR active_epoch_id=NEW.active_epoch_id
              OR (NEW.cutover_id IS NOT NULL AND cutover_id=NEW.cutover_id))
          OR NOT EXISTS(SELECT 1 FROM paper_ledger_account a WHERE a.account_id=NEW.account_id
              AND a.epoch_id=NEW.active_epoch_id AND a.manifest_hash=NEW.active_manifest_hash)
        BEGIN SELECT RAISE(ABORT,'invalid paper book owner insert'); END"),
    ("trigger", "paper_book_owner_v2_transition", "paper_book_owner_v2", OWNER_TRANSITION_GUARD_DDL),
];

/// Frozen V1 DDL is unchanged. These generation-5 guards replace only the
/// generation-4 guards and refer to the one current owner table.
pub(crate) const V1_GUARD_STATEMENTS: &[(&str, &str, &str, &str)] = &[
    ("trigger", "paper_book_owner_v2_account_insert", "paper_ledger_account", "CREATE TRIGGER paper_book_owner_v2_account_insert BEFORE INSERT ON paper_ledger_account
        WHEN EXISTS(SELECT 1 FROM paper_ledger_account WHERE account_id=NEW.account_id OR epoch_id=NEW.epoch_id)
          OR NOT EXISTS(SELECT 1 FROM paper_book_owner_v2 WHERE account_id=NEW.account_id
              AND active_generation=1 AND active_epoch_id=NEW.epoch_id AND active_manifest_hash=NEW.manifest_hash)
        BEGIN SELECT RAISE(ABORT,'V1 account owner mismatch'); END"),
    ("trigger", "paper_book_owner_v2_event_insert", "paper_ledger_event", "CREATE TRIGGER paper_book_owner_v2_event_insert BEFORE INSERT ON paper_ledger_event
        WHEN NOT EXISTS(SELECT 1 FROM paper_book_owner_v2 o JOIN paper_ledger_account a
              ON a.account_id=o.account_id WHERE o.account_id=NEW.account_id
                AND o.active_generation=1 AND o.active_epoch_id=a.epoch_id
                AND o.active_manifest_hash=a.manifest_hash)
          OR EXISTS(SELECT 1 FROM paper_ledger_event
              WHERE (account_id=NEW.account_id AND (seq=NEW.seq OR command_id=NEW.command_id
                  OR (NEW.is_terminal=1 AND NEW.business_plan_id IS NOT NULL
                      AND is_terminal=1 AND business_plan_id=NEW.business_plan_id)))
                OR (NEW.paper_trade_id IS NOT NULL AND paper_trade_id=NEW.paper_trade_id)
                OR (NEW.order_audit_id IS NOT NULL AND order_audit_id=NEW.order_audit_id))
        BEGIN SELECT RAISE(ABORT,'V1 event owner mismatch or replacement'); END"),
    ("trigger", "paper_book_owner_v2_head_insert", "paper_ledger_head", "CREATE TRIGGER paper_book_owner_v2_head_insert BEFORE INSERT ON paper_ledger_head
        WHEN NOT EXISTS(SELECT 1 FROM paper_book_owner_v2 o JOIN paper_ledger_account a
              ON a.account_id=o.account_id WHERE o.account_id=NEW.account_id
                AND o.active_generation=1 AND o.active_epoch_id=a.epoch_id
                AND o.active_manifest_hash=a.manifest_hash)
          OR EXISTS(SELECT 1 FROM paper_ledger_head WHERE account_id=NEW.account_id)
        BEGIN SELECT RAISE(ABORT,'V1 head owner mismatch or replacement'); END"),
    ("trigger", "paper_book_owner_v2_head_update", "paper_ledger_head", "CREATE TRIGGER paper_book_owner_v2_head_update BEFORE UPDATE ON paper_ledger_head
        WHEN NEW.account_id!=OLD.account_id OR NOT EXISTS(
            SELECT 1 FROM paper_book_owner_v2 o JOIN paper_ledger_account a
              ON a.account_id=o.account_id WHERE o.account_id=NEW.account_id
                AND o.active_generation=1 AND o.active_epoch_id=a.epoch_id
                AND o.active_manifest_hash=a.manifest_hash)
        BEGIN SELECT RAISE(ABORT,'V1 head owner mismatch'); END"),
    ("trigger", "paper_book_owner_v2_head_delete", "paper_ledger_head", "CREATE TRIGGER paper_book_owner_v2_head_delete BEFORE DELETE ON paper_ledger_head
        WHEN EXISTS(SELECT 1 FROM paper_book_owner_v2 WHERE account_id=OLD.account_id AND active_generation=2)
        BEGIN SELECT RAISE(ABORT,'inactive V1 head cannot be deleted'); END"),
];

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

fn identity(conn: &mut SqliteConnection) -> Result<Identity, PaperBookOwnerError> {
    Ok(diesel::sql_query(
        "SELECT application_id,user_version FROM pragma_application_id(),pragma_user_version()",
    )
    .get_result(conn)?)
}

fn objects(
    conn: &mut SqliteConnection,
    prefix: &str,
) -> Result<Vec<CatalogObject>, PaperBookOwnerError> {
    Ok(diesel::sql_query(
        "SELECT 'main' AS namespace,type AS kind,name,tbl_name AS table_name,sql
         FROM main.sqlite_master WHERE (name GLOB ? OR tbl_name GLOB ?) AND sql IS NOT NULL
         UNION ALL SELECT 'temp',type,name,tbl_name,sql FROM temp.sqlite_master
         WHERE (name GLOB ? OR tbl_name GLOB ?) AND sql IS NOT NULL
         ORDER BY namespace,kind,name,table_name,sql",
    )
    .bind::<Text, _>(prefix)
    .bind::<Text, _>(prefix)
    .bind::<Text, _>(prefix)
    .bind::<Text, _>(prefix)
    .load(conn)?)
}

pub(crate) fn create_schema(conn: &mut SqliteConnection) -> Result<(), PaperBookOwnerError> {
    for (_, _, _, statement) in OWNER_STATEMENTS {
        diesel::sql_query(*statement).execute(conn)?;
    }
    for (_, _, _, statement) in V1_GUARD_STATEMENTS {
        diesel::sql_query(*statement).execute(conn)?;
    }
    Ok(())
}

/// Read-only structural check for the exact CatalogV5 namespace. This does not
/// validate any account owner row and cannot grant a V1 or V2 write capability.
pub(crate) fn verify_catalog_v5_structure_on(
    conn: &mut SqliteConnection,
) -> Result<(), PaperBookOwnerError> {
    let found = identity(conn)?;
    if found.application_id != APPLICATION_ID || found.user_version != CATALOG_GENERATION {
        return Err(PaperBookOwnerError::WrongDatabaseIdentity);
    }
    let mut reference = SqliteConnection::establish(":memory:")?;
    super::paper_ledger_schema_v1::create_schema(&mut reference)?;
    super::paper_book_v2_schema::create_schema(&mut reference)?;
    super::paper_book_v2_ledger_schema_v1::create_schema(&mut reference)?;
    create_schema(&mut reference)?;
    for prefix in ["paper_ledger_*", "paper_book_owner_*", "paper_book_v2_*"] {
        if objects(conn, prefix)? != objects(&mut reference, prefix)? {
            return Err(PaperBookOwnerError::CatalogMismatch);
        }
    }
    if !super::daily_change_review_schema_v1::is_present(conn)
        .map_err(|_| PaperBookOwnerError::CatalogMismatch)?
    {
        return Err(PaperBookOwnerError::CatalogMismatch);
    }
    super::paper_book_v2_schema::verify_v5_manifest_on(conn)?;
    Ok(())
}

/// Read-only owner proof. V2Active rows require complete V1 replay and exact
/// anchor, genesis, and projection verification.
pub(crate) fn verify_catalog_v5_on(conn: &mut SqliteConnection) -> Result<(), PaperBookOwnerError> {
    verify_catalog_v5_structure_on(conn)?;
    crate::trading::paper_book_v2::verify_owner_rows_on(conn)
        .map_err(|_| PaperBookOwnerError::InactiveOwner)
}

#[cfg(test)]
pub(crate) fn require_isolated_for_test(
    conn: &mut SqliteConnection,
) -> Result<(), PaperBookOwnerError> {
    super::paper_book_owner_schema_v1::require_isolated(conn)
}

pub(crate) fn require_v1_owner_on(
    conn: &mut SqliteConnection,
    account_id: &str,
    epoch_id: &str,
    manifest_hash: &str,
) -> Result<(), PaperBookOwnerError> {
    verify_catalog_v5_on(conn)?;
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        value: i64,
    }
    let matches = diesel::sql_query(
        "SELECT COUNT(*) AS value FROM paper_book_owner_v2 WHERE account_id=? AND active_generation=1
          AND active_epoch_id=? AND active_manifest_hash=? AND owner_revision=1 AND cutover_id IS NULL",
    )
    .bind::<Text, _>(account_id)
    .bind::<Text, _>(epoch_id)
    .bind::<Text, _>(manifest_hash)
    .get_result::<Count>(conn)?.value;
    if matches != 1 {
        return Err(PaperBookOwnerError::InactiveOwner);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn install_catalog_v5_for_isolated_test(
    conn: &mut SqliteConnection,
) -> Result<(), PaperBookOwnerError> {
    install_with_fault(conn, false)
}

#[cfg(test)]
fn install_with_fault(
    conn: &mut SqliteConnection,
    fail_after_version: bool,
) -> Result<(), PaperBookOwnerError> {
    conn.immediate_transaction(|conn| {
        super::paper_book_owner_schema_v1::require_isolated(conn)?;
        super::paper_book_owner_schema_v1::verify_catalog_v4_on(conn)?;
        #[derive(QueryableByName)]
        struct Owner {
            #[diesel(sql_type = Text)]
            account_id: String,
            #[diesel(sql_type = Text)]
            active_epoch_id: String,
            #[diesel(sql_type = Text)]
            active_manifest_hash: String,
        }
        let owners: Vec<Owner> = diesel::sql_query(
            "SELECT account_id,active_epoch_id,active_manifest_hash FROM paper_book_owner_v1 ORDER BY account_id",
        ).load(conn)?;
        for (_, name, _, _) in super::paper_book_owner_schema_v1::V1_GUARD_STATEMENTS {
            diesel::sql_query(format!("DROP TRIGGER {name}")).execute(conn)?;
        }
        for (_, name, _, _) in &super::paper_book_owner_schema_v1::STATEMENTS[1..] {
            diesel::sql_query(format!("DROP TRIGGER {name}")).execute(conn)?;
        }
        diesel::sql_query("DROP TABLE paper_book_owner_v1").execute(conn)?;
        super::paper_book_v2_ledger_schema_v1::create_schema(conn)?;
        create_schema(conn)?;
        for owner in owners {
            diesel::sql_query("INSERT INTO paper_book_owner_v2(account_id,active_generation,active_epoch_id,active_manifest_hash,owner_revision,cutover_id) VALUES (?,1,?,?,1,NULL)")
                .bind::<Text, _>(owner.account_id)
                .bind::<Text, _>(owner.active_epoch_id)
                .bind::<Text, _>(owner.active_manifest_hash)
                .execute(conn)?;
        }
        diesel::sql_query("PRAGMA user_version=5").execute(conn)?;
        if fail_after_version {
            return Err(PaperBookOwnerError::InjectedFailure);
        }
        verify_catalog_v5_on(conn)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::fee_policy::AShareFeePolicyV2;
    use diesel::connection::SimpleConnection;

    #[derive(QueryableByName)]
    struct Value {
        #[diesel(sql_type = Text)]
        value: String,
    }

    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        value: i64,
    }

    fn value(conn: &mut SqliteConnection, sql: &str) -> String {
        diesel::sql_query(sql)
            .get_result::<Value>(conn)
            .unwrap()
            .value
    }

    fn count(conn: &mut SqliteConnection, sql: &str) -> i64 {
        diesel::sql_query(sql)
            .get_result::<Count>(conn)
            .unwrap()
            .value
    }

    fn v3() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        super::super::paper_ledger_schema_v1::create_schema(&mut conn).unwrap();
        super::super::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
        conn.batch_execute(
            "INSERT INTO paper_ledger_account VALUES
            ('acct-a','epoch-a','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
             'manifest-old','micro-cny-half-up-v1','lot-rates-v1');
            INSERT INTO paper_ledger_event
              (account_id,seq,command_id,previous_hash,event_hash,payload)
              VALUES ('acct-a',1,'seed','genesis','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc','event-old-bytes');
            INSERT INTO paper_ledger_head VALUES
              ('acct-a',1,'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
               'projection-old-bytes','dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd');
            PRAGMA application_id=1398035265; PRAGMA user_version=3",
        )
        .unwrap();
        conn
    }

    fn v4() -> SqliteConnection {
        let mut conn = v3();
        super::super::paper_book_owner_schema_v1::install_catalog_v4_for_isolated_test(
            &mut conn,
            &AShareFeePolicyV2::fixed_compatibility_assumption(),
        )
        .unwrap();
        conn
    }

    #[test]
    fn upgrade_preserves_v1_bytes_and_installs_only_v1_active_owners() {
        let mut conn = v4();
        let before = [
            value(&mut conn, "SELECT manifest_bytes AS value FROM paper_ledger_account WHERE account_id='acct-a'"),
            value(&mut conn, "SELECT payload AS value FROM paper_ledger_event WHERE account_id='acct-a' AND seq=1"),
            value(&mut conn, "SELECT projection_bytes AS value FROM paper_ledger_head WHERE account_id='acct-a'"),
        ];
        install_catalog_v5_for_isolated_test(&mut conn).unwrap();
        verify_catalog_v5_on(&mut conn).unwrap();
        assert!(super::super::paper_book_owner_schema_v1::verify_catalog_v4_on(&mut conn).is_err());
        assert_eq!(count(&mut conn, "SELECT COUNT(*) AS value FROM paper_book_owner_v2 WHERE active_generation=1 AND owner_revision=1 AND cutover_id IS NULL"), 1);
        assert_eq!(count(&mut conn, "SELECT (SELECT COUNT(*) FROM paper_book_v2_account)+(SELECT COUNT(*) FROM paper_book_v2_event)+(SELECT COUNT(*) FROM paper_book_v2_head) AS value"), 0);
        assert_eq!(before, [
            value(&mut conn, "SELECT manifest_bytes AS value FROM paper_ledger_account WHERE account_id='acct-a'"),
            value(&mut conn, "SELECT payload AS value FROM paper_ledger_event WHERE account_id='acct-a' AND seq=1"),
            value(&mut conn, "SELECT projection_bytes AS value FROM paper_ledger_head WHERE account_id='acct-a'"),
        ]);
        super::super::paper_book_owner_schema_v1::require_v1_owner_on(
            &mut conn,
            "acct-a",
            "epoch-a",
            &"a".repeat(64),
        )
        .unwrap();
        assert!(matches!(
            super::super::paper_book_owner_schema_v1::require_v1_owner_on(
                &mut conn,
                "acct-a",
                "wrong-epoch",
                &"a".repeat(64)
            ),
            Err(PaperBookOwnerError::InactiveOwner)
        ));
    }

    #[test]
    fn injected_failure_rolls_back_owner_and_generation_together() {
        let mut conn = v4();
        let before = objects(&mut conn, "paper_book_owner_*").unwrap();
        assert!(matches!(
            install_with_fault(&mut conn, true),
            Err(PaperBookOwnerError::InjectedFailure)
        ));
        super::super::paper_book_owner_schema_v1::verify_catalog_v4_on(&mut conn).unwrap();
        assert_eq!(before, objects(&mut conn, "paper_book_owner_*").unwrap());
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS value FROM paper_book_owner_v1"
            ),
            1
        );
        assert_eq!(count(&mut conn, "SELECT COUNT(*) AS value FROM sqlite_master WHERE name='paper_book_owner_v2' OR name='paper_book_v2_account'"), 0);
    }

    #[test]
    fn exact_verifier_rejects_added_missing_and_tampered_objects() {
        for mutation in [
            "CREATE TABLE paper_book_v2_shadow(value TEXT)",
            "CREATE TRIGGER paper_book_owner_v2_extra BEFORE INSERT ON paper_ledger_event BEGIN SELECT RAISE(ABORT,'extra'); END",
            "DROP TRIGGER paper_book_owner_v2_event_insert",
            "DROP TRIGGER paper_ledger_event_no_delete",
        ] {
            let mut conn = v4();
            install_catalog_v5_for_isolated_test(&mut conn).unwrap();
            conn.batch_execute(mutation).unwrap();
            assert!(matches!(verify_catalog_v5_on(&mut conn), Err(PaperBookOwnerError::CatalogMismatch)), "{mutation}");
        }
    }

    #[test]
    fn owner_cannot_be_forged_and_v1_replace_is_rejected() {
        let mut conn = v4();
        install_catalog_v5_for_isolated_test(&mut conn).unwrap();
        assert!(diesel::sql_query("INSERT INTO paper_book_owner_v2 VALUES ('new',1,'new-epoch','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',1,NULL)")
            .execute(&mut conn).is_err());
        assert!(diesel::sql_query("UPDATE paper_book_owner_v2 SET active_generation=2,owner_revision=2,cutover_id='cutover' WHERE account_id='acct-a'")
            .execute(&mut conn).is_err());
        assert!(diesel::sql_query(
            "INSERT OR REPLACE INTO paper_ledger_event
            (account_id,seq,command_id,previous_hash,event_hash,payload) VALUES
            ('acct-a',1,'seed','genesis','replacement','replacement')"
        )
        .execute(&mut conn)
        .is_err());
        assert!(diesel::sql_query(
            "INSERT OR REPLACE INTO paper_ledger_head VALUES
            ('acct-a',1,'replacement','replacement','replacement')"
        )
        .execute(&mut conn)
        .is_err());
        assert_eq!(value(&mut conn, "SELECT payload AS value FROM paper_ledger_event WHERE account_id='acct-a' AND seq=1"), "event-old-bytes");
        assert_eq!(
            value(
                &mut conn,
                "SELECT projection_bytes AS value FROM paper_ledger_head WHERE account_id='acct-a'"
            ),
            "projection-old-bytes"
        );
        diesel::sql_query(
            "INSERT INTO paper_ledger_event
            (account_id,seq,command_id,previous_hash,event_hash,payload) VALUES
            ('acct-a',2,'next','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc','event-next','next-bytes')",
        )
        .execute(&mut conn)
        .unwrap();
        diesel::sql_query("UPDATE paper_ledger_head SET version=2,event_hash='event-next',projection_bytes='projection-next',projection_hash='projection-hash-next' WHERE account_id='acct-a'")
            .execute(&mut conn).unwrap();
        verify_catalog_v5_on(&mut conn).unwrap();
    }

    #[test]
    fn prepared_verifier_rejects_uncommitted_v2_rows() {
        let mut conn = v4();
        install_catalog_v5_for_isolated_test(&mut conn).unwrap();
        diesel::sql_query("INSERT INTO paper_book_v2_account
            (account_id,epoch_id,manifest_hash,manifest_bytes,fee_policy_instance_id,
             v1_epoch_id,v1_manifest_hash,v1_head_version,v1_head_hash,v1_projection_hash,cutover_id)
            SELECT 'acct-a','epoch-v2','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                   'manifest-v2',policy_instance_id,'epoch-a',
                   'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                   1,'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                   'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd','cutover-test'
            FROM paper_book_v2_fee_manifest WHERE singleton=1")
            .execute(&mut conn)
            .unwrap();
        verify_catalog_v5_structure_on(&mut conn).unwrap();
        assert!(matches!(
            verify_catalog_v5_on(&mut conn),
            Err(PaperBookOwnerError::InactiveOwner)
        ));
        assert!(matches!(
            require_v1_owner_on(&mut conn, "acct-a", "epoch-a", &"a".repeat(64)),
            Err(PaperBookOwnerError::InactiveOwner)
        ));
    }

    #[test]
    fn raw_transition_is_blocked_even_with_staged_genesis() {
        let mut conn = v4();
        install_catalog_v5_for_isolated_test(&mut conn).unwrap();
        conn.batch_execute("INSERT INTO paper_book_v2_account
            (account_id,epoch_id,manifest_hash,manifest_bytes,fee_policy_instance_id,
             v1_epoch_id,v1_manifest_hash,v1_head_version,v1_head_hash,v1_projection_hash,cutover_id)
            SELECT 'acct-a','epoch-v2','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                   'manifest-v2',policy_instance_id,'epoch-a',
                   'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                   1,'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                   'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd','cutover-test'
            FROM paper_book_v2_fee_manifest WHERE singleton=1;
            INSERT INTO paper_book_v2_event VALUES
              ('acct-a',1,'genesis-v2','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
               'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee','Genesis','genesis-payload');
            INSERT INTO paper_book_v2_head VALUES
              ('acct-a',1,'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
               'projection-v2','ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff')")
            .unwrap();
        for keyword in ["UPDATE", "UPDATE OR REPLACE"] {
            let sql = format!("{keyword} paper_book_owner_v2 SET active_generation=2,active_epoch_id='epoch-v2',
              active_manifest_hash='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
              owner_revision=2,cutover_id='cutover-test' WHERE account_id='acct-a'");
            assert!(diesel::sql_query(sql).execute(&mut conn).is_err());
        }
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS value FROM paper_book_owner_v2 WHERE active_generation=1"
            ),
            1
        );
        verify_catalog_v5_structure_on(&mut conn).unwrap();
        assert!(matches!(
            verify_catalog_v5_on(&mut conn),
            Err(PaperBookOwnerError::InactiveOwner)
        ));
        assert_eq!(value(&mut conn, "SELECT payload AS value FROM paper_ledger_event WHERE account_id='acct-a' AND seq=1"), "event-old-bytes");
    }

    #[test]
    fn v2_epoch_cannot_reuse_any_v1_epoch() {
        let mut conn = v3();
        conn.batch_execute(
            "INSERT INTO paper_ledger_account VALUES
            ('acct-b','epoch-b','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
             'manifest-b','micro-cny-half-up-v1','lot-rates-v1');
            INSERT INTO paper_ledger_event
              (account_id,seq,command_id,previous_hash,event_hash,payload)
              VALUES ('acct-b',1,'seed-b','genesis','event-b','payload-b');
            INSERT INTO paper_ledger_head VALUES
              ('acct-b',1,'event-b','projection-b','projection-hash-b')",
        )
        .unwrap();
        super::super::paper_book_owner_schema_v1::install_catalog_v4_for_isolated_test(
            &mut conn,
            &AShareFeePolicyV2::fixed_compatibility_assumption(),
        )
        .unwrap();
        install_catalog_v5_for_isolated_test(&mut conn).unwrap();
        for epoch in ["epoch-a", "epoch-b"] {
            let sql = "INSERT INTO paper_book_v2_account
                (account_id,epoch_id,manifest_hash,manifest_bytes,fee_policy_instance_id,
                 v1_epoch_id,v1_manifest_hash,v1_head_version,v1_head_hash,v1_projection_hash,cutover_id)
                SELECT 'acct-a',?,'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                       'manifest-v2',policy_instance_id,'epoch-a',
                       'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                       1,'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                       'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd','cutover-conflict'
                FROM paper_book_v2_fee_manifest WHERE singleton=1";
            assert!(diesel::sql_query(sql)
                .bind::<Text, _>(epoch)
                .execute(&mut conn)
                .is_err());
        }
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS value FROM paper_book_v2_account"
            ),
            0
        );
        assert_eq!(
            count(
                &mut conn,
                "SELECT COUNT(*) AS value FROM paper_book_owner_v2 WHERE active_generation=1"
            ),
            2
        );
        verify_catalog_v5_on(&mut conn).unwrap();
    }

    #[test]
    fn file_outside_test_isolation_is_not_upgraded() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("real.db");
        let mut conn = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        super::super::paper_ledger_schema_v1::create_schema(&mut conn).unwrap();
        super::super::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
        conn.batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=3")
            .unwrap();
        assert!(matches!(
            install_catalog_v5_for_isolated_test(&mut conn),
            Err(PaperBookOwnerError::NotIsolated)
        ));
    }
}
