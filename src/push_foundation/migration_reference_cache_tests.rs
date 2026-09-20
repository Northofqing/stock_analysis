use super::*;

const TARGET_TRIGGER: &str = "push_intents_insert_guard";
const REGISTRY_GUARD: &str = "push_foundation_objects_update";

fn valid_foundation() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    let sql = DDL_BYTES.strip_prefix(b".bail on\n").unwrap();
    connection
        .execute_batch("PRAGMA temp_store=MEMORY;")
        .unwrap();
    connection
        .execute_batch(std::str::from_utf8(sql).unwrap())
        .unwrap();
    connection
}

fn live_definition(connection: &Connection, name: &str) -> Vec<u8> {
    connection
        .query_row(
            "SELECT CAST(sql AS BLOB) FROM sqlite_schema WHERE name=?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

fn registered_definition(connection: &Connection, name: &str) -> Vec<u8> {
    connection
        .query_row(
            "SELECT CAST(definition AS BLOB) FROM push_foundation_objects WHERE name=?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

fn assert_check(error: FoundationMigrationError, expected: &'static str) {
    assert_eq!(
        error,
        FoundationMigrationError::AttestationFailed { check: expected }
    );
}

#[test]
fn reference_cache_rechecks_warmed_same_connection_and_rollback_restores_it() {
    let connection = valid_foundation();
    let original = live_definition(&connection, TARGET_TRIGGER);
    let first = attest_bundled_connection(&connection).unwrap();
    let repeated = attest_bundled_connection(&connection).unwrap();
    assert_eq!(repeated, first);

    connection
        .execute_batch(&format!(
            "PRAGMA query_only=OFF; BEGIN IMMEDIATE; DROP TRIGGER {TARGET_TRIGGER};"
        ))
        .unwrap();
    assert_check(
        attest_bundled_connection(&connection).unwrap_err(),
        "managed_object_definitions",
    );
    connection
        .execute_batch("ROLLBACK; PRAGMA query_only=OFF;")
        .unwrap();
    assert_eq!(live_definition(&connection, TARGET_TRIGGER), original);
    assert_eq!(attest_bundled_connection(&connection).unwrap(), first);
}

#[test]
fn reference_cache_rechecks_independent_connection_after_warmup() {
    let warm = valid_foundation();
    let warm_receipt = attest_bundled_connection(&warm).unwrap();

    let corrupted = valid_foundation();
    let valid_receipt = attest_bundled_connection(&corrupted).unwrap();
    assert_eq!(valid_receipt, warm_receipt);
    corrupted
        .execute_batch(&format!("PRAGMA query_only=OFF; DROP TRIGGER {TARGET_TRIGGER};"))
        .unwrap();
    assert_check(
        attest_bundled_connection(&corrupted).unwrap_err(),
        "managed_object_definitions",
    );
}

#[test]
fn reference_cache_rejects_matching_tampered_registry_and_sqlite_schema() {
    let connection = valid_foundation();
    let migration = FoundationSchemaMigration::bundled().unwrap();
    let original_target = live_definition(&connection, TARGET_TRIGGER);
    let original_guard = live_definition(&connection, REGISTRY_GUARD);
    let warm_receipt = attest_bundled_connection(&connection).unwrap();

    connection
        .execute_batch(&format!(
            "PRAGMA query_only=OFF; BEGIN IMMEDIATE; \
             DROP TRIGGER {REGISTRY_GUARD}; \
             DROP TRIGGER {TARGET_TRIGGER}; \
             CREATE TRIGGER {TARGET_TRIGGER} BEFORE INSERT ON push_intents \
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE foundation tamper'); END;"
        ))
        .unwrap();
    connection
        .execute(
            "UPDATE push_foundation_objects SET definition=( \
             SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1) WHERE name=?1",
            [TARGET_TRIGGER],
        )
        .unwrap();
    connection
        .execute_batch(std::str::from_utf8(&original_guard).unwrap())
        .unwrap();

    assert_eq!(
        live_definition(&connection, TARGET_TRIGGER),
        registered_definition(&connection, TARGET_TRIGGER)
    );
    assert_eq!(
        live_definition(&connection, REGISTRY_GUARD),
        registered_definition(&connection, REGISTRY_GUARD)
    );
    attest_connection(&connection, migration.ddl_sha256())
        .expect("TEST_CODE mutually matching actual registry and sqlite_schema pass live checks");
    assert_check(
        attest_bundled_connection(&connection).unwrap_err(),
        "bundled_object_definitions",
    );

    connection
        .execute_batch("ROLLBACK; PRAGMA query_only=OFF;")
        .unwrap();
    assert_eq!(live_definition(&connection, TARGET_TRIGGER), original_target);
    assert_eq!(live_definition(&connection, REGISTRY_GUARD), original_guard);
    assert_eq!(attest_bundled_connection(&connection).unwrap(), warm_receipt);
}
