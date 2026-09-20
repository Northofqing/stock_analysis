use super::*;

fn open_caller_connection(database: &std::path::Path) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .expect("TEST_CODE open caller-owned audit connection")
}

fn row_counts(connection: &rusqlite::Connection) -> (i64, i64) {
    let audits = connection
        .query_row("SELECT COUNT(*) FROM data_acquisition_audit", [], |row| {
            row.get(0)
        })
        .expect("TEST_CODE count acquisition audit rows");
    let chain = connection
        .query_row(
            "SELECT COUNT(*) FROM data_acquisition_audit_chain",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE count acquisition chain rows");
    (audits, chain)
}

fn assert_diesel_reader(
    database: &std::path::Path,
    receipt: &DataAcquisitionAuditReceipt,
    expected: &DataAcquisitionAuditRecord<'_>,
) {
    let path = database.to_str().expect("TEST_CODE UTF-8 audit path");
    let mut reader = SqliteConnection::establish(path).expect("TEST_CODE Diesel audit reader");
    let verified = read_verified_acquisition_audit(&mut reader, receipt)
        .expect("TEST_CODE existing Diesel reader verifies receipt");
    assert_eq!(verified.receipt(), receipt);
    assert_record_matches(verified.record(), expected);
}

fn audit_database() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().expect("TEST_CODE audit maintenance root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical audit maintenance root")
        .join("acquisition.sqlite3");
    let path = database
        .to_str()
        .expect("TEST_CODE UTF-8 audit maintenance path");
    let mut connection =
        SqliteConnection::establish(path).expect("TEST_CODE audit maintenance schema writer");
    create_schema(&mut connection).expect("TEST_CODE production audit maintenance schema");
    drop(connection);
    (root, database)
}

fn legacy_append(
    database: &std::path::Path,
    expected: &DataAcquisitionAuditRecord<'_>,
) -> DataAcquisitionAuditReceipt {
    let path = database.to_str().expect("TEST_CODE UTF-8 legacy path");
    let mut writer = SqliteConnection::establish(path).expect("TEST_CODE legacy audit writer");
    writer
        .immediate_transaction::<_, diesel::result::Error, _>(|connection| {
            insert_acquisition_audit_query(connection, expected)
        })
        .expect("TEST_CODE legacy audit append")
}

fn caller_append(
    database: &std::path::Path,
    expected: &DataAcquisitionAuditRecord<'_>,
) -> DataAcquisitionAuditReceipt {
    let mut writer = open_caller_connection(database);
    let transaction = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("TEST_CODE caller audit append transaction");
    let receipt = append_acquisition_in_transaction(&transaction, expected)
        .expect("TEST_CODE caller audit append");
    assert!(!transaction.is_autocommit());
    transaction
        .commit()
        .expect("TEST_CODE caller commits audit append");
    assert!(writer.is_autocommit());
    receipt
}

fn rows(
    connection: &rusqlite::Connection,
    sql: &str,
    width: usize,
) -> Vec<Vec<rusqlite::types::Value>> {
    let mut statement = connection
        .prepare(sql)
        .expect("TEST_CODE snapshot statement");
    let mapped = statement
        .query_map([], |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
        })
        .expect("TEST_CODE snapshot query");
    mapped
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("TEST_CODE snapshot rows")
}

fn audit_state(
    connection: &rusqlite::Connection,
) -> (
    Vec<Vec<rusqlite::types::Value>>,
    Vec<Vec<rusqlite::types::Value>>,
) {
    (
        rows(
            connection,
            "SELECT * FROM data_acquisition_audit ORDER BY id",
            16,
        ),
        rows(
            connection,
            "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id",
            4,
        ),
    )
}

fn catalog(connection: &rusqlite::Connection) -> Vec<Vec<rusqlite::types::Value>> {
    rows(
        connection,
        "SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name,tbl_name,sql",
        4,
    )
}

fn varied_record(
    index: usize,
    outcome: &'static str,
    primary: bool,
) -> DataAcquisitionAuditRecord<'static> {
    const OBSERVED: [&str; 9] = [
        "2099-01-02T11:00:00+08:00",
        "2099-01-02T11:01:00+08:00",
        "2099-01-02T11:02:00+08:00",
        "2099-01-02T11:03:00+08:00",
        "2099-01-02T11:04:00+08:00",
        "2099-01-02T11:05:00+08:00",
        "2099-01-02T11:06:00+08:00",
        "2099-01-02T11:07:00+08:00",
        "2099-01-02T11:08:00+08:00",
    ];
    let success = matches!(outcome, "available" | "verified_empty");
    DataAcquisitionAuditRecord {
        capability: if primary {
            "TEST_CODE_PRIMARY_CAPABILITY"
        } else {
            "TEST_CODE_OTHER_CAPABILITY"
        },
        provider: if primary {
            "TEST_CODE_PRIMARY_PROVIDER"
        } else {
            "TEST_CODE_OTHER_PROVIDER"
        },
        source: if primary {
            "TEST_CODE_primary-source"
        } else {
            "TEST_CODE_other-source"
        },
        request_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        source_at: (index % 2 == 0).then_some("2099-01-02"),
        observed_at: OBSERVED[index],
        batch_id: if success || index % 3 == 0 {
            Some("TEST_CODE_optional_batch")
        } else {
            None
        },
        outcome,
        request_count: index as i64 + 1,
        accepted_count: if outcome == "available" {
            index as i64 + 1
        } else {
            0
        },
        rejected_count: index as i64,
        reason_code: "TEST_CODE_maintenance_reason",
        retryable: index % 2 == 1,
    }
}

fn assert_safe_error(error: AcquisitionAuditAppendError, operation: &'static str) {
    assert_eq!(error.operation(), operation);
    assert!(!error.to_string().contains("TEST_CODE_SECRET"));
    assert!(!format!("{error:?}").contains("TEST_CODE_SECRET"));
    assert!(std::error::Error::source(&error).is_none());
}

#[test]
fn caller_owned_acquisition_append_preserves_legacy_chain_and_commit_ownership() {
    let root = tempfile::tempdir().expect("TEST_CODE audit transaction root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical audit root")
        .join("acquisition.sqlite3");
    let path = database.to_str().expect("TEST_CODE UTF-8 audit path");

    let first = record("verified_empty", Some("TEST_CODE_batch_empty"));
    let first_receipt = {
        let mut legacy = SqliteConnection::establish(path).expect("TEST_CODE legacy writer");
        create_schema(&mut legacy).expect("TEST_CODE production audit schema");
        legacy
            .immediate_transaction::<_, diesel::result::Error, _>(|connection| {
                insert_acquisition_audit_query(connection, &first)
            })
            .expect("TEST_CODE legacy append")
    };
    assert_eq!(first_receipt.audit_id, 1);
    assert_eq!(first_receipt.previous_outcome, None);
    assert_eq!(first_receipt.current_outcome, "verified_empty");
    let first_hash = first_receipt.record_hash.clone();

    let mut second = record("available", Some("TEST_CODE_batch_available"));
    second.observed_at = "2099-01-02T10:01:00+08:00";
    let second_receipt = {
        let mut caller = open_caller_connection(&database);
        let transaction = caller
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("TEST_CODE caller begins append transaction");
        let receipt = append_acquisition_in_transaction(&transaction, &second)
            .expect("TEST_CODE append through caller transaction");
        assert_eq!(receipt.audit_id, 2);
        assert_eq!(receipt.previous_outcome.as_deref(), Some("verified_empty"));
        assert_eq!(receipt.current_outcome, "available");
        assert!(!transaction.is_autocommit());
        let verified = read_acquisition_in_transaction(&transaction, &receipt)
            .expect("TEST_CODE transaction sees candidate receipt");
        assert_eq!(verified.receipt(), &receipt);
        assert_record_matches(verified.record(), &second);
        assert!(!transaction.is_autocommit());
        transaction
            .commit()
            .expect("TEST_CODE caller commits audit append");
        assert!(caller.is_autocommit());
        receipt
    };

    let first_sqlite = read_with_rusqlite(&database, &first_receipt);
    let second_sqlite = read_with_rusqlite(&database, &second_receipt);
    assert_eq!(first_sqlite.receipt(), &first_receipt);
    assert_eq!(first_sqlite.receipt().record_hash, first_hash);
    assert_record_matches(first_sqlite.record(), &first);
    assert_eq!(second_sqlite.receipt(), &second_receipt);
    assert_record_matches(second_sqlite.record(), &second);
    assert_diesel_reader(&database, &first_receipt, &first);
    assert_diesel_reader(&database, &second_receipt, &second);

    let mut rolled_back_record = record("verified_empty", Some("TEST_CODE_batch_rollback"));
    rolled_back_record.observed_at = "2099-01-02T10:02:00+08:00";
    let rolled_back_receipt = {
        let mut caller = open_caller_connection(&database);
        let transaction = caller
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("TEST_CODE caller begins rollback transaction");
        let receipt = append_acquisition_in_transaction(&transaction, &rolled_back_record)
            .expect("TEST_CODE append rollback candidate");
        assert_eq!(receipt.audit_id, 3);
        assert_eq!(receipt.previous_outcome.as_deref(), Some("available"));
        assert_eq!(receipt.current_outcome, "verified_empty");
        let verified = read_acquisition_in_transaction(&transaction, &receipt)
            .expect("TEST_CODE transaction sees rollback candidate");
        assert_record_matches(verified.record(), &rolled_back_record);
        assert!(!transaction.is_autocommit());
        transaction
            .rollback()
            .expect("TEST_CODE caller rolls back audit append");
        assert!(caller.is_autocommit());
        receipt
    };

    let mut reopened = open_caller_connection(&database);
    assert_eq!(row_counts(&reopened), (2, 2));
    let transaction = reopened
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .expect("TEST_CODE caller begins verification transaction");
    assert!(read_acquisition_in_transaction(&transaction, &rolled_back_receipt).is_err());
    let first_verified = read_acquisition_in_transaction(&transaction, &first_receipt)
        .expect("TEST_CODE original legacy receipt survives rollback");
    let second_verified = read_acquisition_in_transaction(&transaction, &second_receipt)
        .expect("TEST_CODE committed caller receipt survives rollback");
    assert_eq!(first_verified.receipt().record_hash, first_hash);
    assert_record_matches(first_verified.record(), &first);
    assert_record_matches(second_verified.record(), &second);
    assert!(!transaction.is_autocommit());
    transaction
        .rollback()
        .expect("TEST_CODE caller ends verification transaction");
    assert!(reopened.is_autocommit());
}

#[test]
fn alternating_diesel_and_rusqlite_appends_preserve_global_chain_and_provider_state() {
    let (_root, database) = audit_database();
    let outcomes = [
        "available",
        "verified_empty",
        "invalid_request",
        "unavailable",
        "stale",
        "partial",
        "conflict",
        "unsupported",
    ];
    let expected_previous = [
        None,
        None,
        None,
        Some("available"),
        Some("verified_empty"),
        Some("invalid_request"),
        Some("unavailable"),
        None,
    ];
    let key_variants = [0, 1, 2, 0, 1, 2, 0, 3];
    let mut receipts = Vec::new();

    for (index, outcome) in outcomes.into_iter().enumerate() {
        let mut expected = varied_record(index, outcome, true);
        match key_variants[index] {
            1 => expected.provider = "TEST_CODE_OTHER_PROVIDER",
            2 => expected.capability = "TEST_CODE_OTHER_CAPABILITY",
            3 => {
                expected.capability = "TEST_CODE_OTHER_CAPABILITY";
                expected.provider = "TEST_CODE_OTHER_PROVIDER";
            }
            _ => {}
        }
        let receipt = if index % 2 == 0 {
            caller_append(&database, &expected)
        } else {
            legacy_append(&database, &expected)
        };
        assert_eq!(receipt.audit_id, index as i64 + 1);
        assert_eq!(
            receipt.previous_outcome.as_deref(),
            expected_previous[index]
        );
        assert_eq!(receipt.current_outcome, outcome);
        assert_eq!(
            receipt.provider_state_changed(),
            expected_previous[index].is_some_and(|previous| previous != outcome)
        );
        receipts.push(receipt);
    }

    for (index, (outcome, receipt)) in outcomes.into_iter().zip(&receipts).enumerate() {
        let mut expected = varied_record(index, outcome, true);
        match key_variants[index] {
            1 => expected.provider = "TEST_CODE_OTHER_PROVIDER",
            2 => expected.capability = "TEST_CODE_OTHER_CAPABILITY",
            3 => {
                expected.capability = "TEST_CODE_OTHER_CAPABILITY";
                expected.provider = "TEST_CODE_OTHER_PROVIDER";
            }
            _ => {}
        }
        let sqlite = read_with_rusqlite(&database, receipt);
        let path = database.to_str().expect("TEST_CODE UTF-8 mixed path");
        let mut diesel = SqliteConnection::establish(path).expect("TEST_CODE mixed Diesel reader");
        let diesel = read_verified_acquisition_audit(&mut diesel, receipt)
            .expect("TEST_CODE mixed Diesel verification");
        assert_record_matches(sqlite.record(), &expected);
        assert_record_matches(diesel.record(), &expected);
        assert_persisted_audits_match(&sqlite.audit, &diesel.audit);
    }
}

#[test]
fn append_rejects_invalid_records_and_damaged_tails_without_repair() {
    let (_root, database) = audit_database();
    let mut missing_batch = varied_record(0, "available", true);
    missing_batch.batch_id = None;
    missing_batch.source = "TEST_CODE_SECRET_missing_batch";
    let mut unknown = varied_record(1, "TEST_CODE_SECRET_unknown", true);
    unknown.batch_id = Some("TEST_CODE_batch");
    let mut bad_hash = varied_record(2, "stale", true);
    bad_hash.request_hash = "TEST_CODE_SECRET_hash";
    let mut negative = varied_record(3, "partial", true);
    negative.request_count = -1;
    negative.source = "TEST_CODE_SECRET_negative";
    let mut blank = varied_record(4, "conflict", true);
    blank.capability = " \t";
    blank.source = "TEST_CODE_SECRET_blank";

    for invalid in [missing_batch, unknown, bad_hash, negative, blank] {
        let mut caller = open_caller_connection(&database);
        let before = audit_state(&caller);
        let transaction = caller
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("TEST_CODE invalid append transaction");
        let error = append_acquisition_in_transaction(&transaction, &invalid)
            .expect_err("TEST_CODE invalid audit record must fail");
        assert_safe_error(error, "record validation");
        assert!(!transaction.is_autocommit());
        assert_eq!(audit_state(&transaction), before);
        transaction
            .rollback()
            .expect("TEST_CODE caller rolls back invalid append");
        assert!(caller.is_autocommit());
        assert_eq!(audit_state(&caller), before);
    }

    for damage in ["hash", "linkage"] {
        let (_root, database) = audit_database();
        let original = varied_record(0, "available", true);
        let receipt = legacy_append(&database, &original);
        let mut connection = open_caller_connection(&database);
        let guard_name = match damage {
            "hash" => "trg_data_acquisition_audit_chain_no_update",
            "linkage" => "trg_data_acquisition_audit_chain_no_delete",
            _ => unreachable!(),
        };
        let guard_sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
                [guard_name],
                |row| row.get(0),
            )
            .expect("TEST_CODE read original immutable guard");
        let original_catalog = catalog(&connection);
        match damage {
            "hash" => connection
                .execute_batch(
                    "DROP TRIGGER trg_data_acquisition_audit_chain_no_update;
                     UPDATE data_acquisition_audit_chain
                     SET record_hash='TEST_CODE_SECRET_damaged_tail';",
                )
                .expect("TEST_CODE damage tail hash"),
            "linkage" => connection
                .execute_batch(
                    "DROP TRIGGER trg_data_acquisition_audit_chain_no_delete;
                     DELETE FROM data_acquisition_audit_chain;",
                )
                .expect("TEST_CODE damage tail linkage"),
            _ => unreachable!(),
        }
        connection
            .execute_batch(&guard_sql)
            .expect("TEST_CODE restore exact immutable guard");
        assert_eq!(catalog(&connection), original_catalog);
        let damaged = audit_state(&connection);
        let next = varied_record(1, "verified_empty", true);
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("TEST_CODE damaged tail append transaction");
        let error = append_acquisition_in_transaction(&transaction, &next)
            .expect_err("TEST_CODE damaged tail must reject append");
        assert_safe_error(error, "tail validation");
        assert!(!transaction.is_autocommit());
        assert_eq!(audit_state(&transaction), damaged);
        transaction
            .rollback()
            .expect("TEST_CODE caller rolls back damaged tail append");
        assert_eq!(audit_state(&connection), damaged);
        let read_transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .expect("TEST_CODE damaged read transaction");
        assert!(read_acquisition_in_transaction(&read_transaction, &receipt).is_err());
        read_transaction
            .rollback()
            .expect("TEST_CODE caller ends damaged read transaction");
    }
}

#[test]
fn append_sql_failures_remain_caller_owned_and_atomic_on_rollback() {
    {
        let (_root, database) = audit_database();
        let mut caller = open_caller_connection(&database);
        caller
            .execute_batch(
                "CREATE TRIGGER test_abort_audit_insert
                 BEFORE INSERT ON data_acquisition_audit
                 BEGIN SELECT RAISE(ABORT, 'TEST_CODE_SECRET_audit_insert'); END;",
            )
            .expect("TEST_CODE audit insert abort trigger");
        let before = audit_state(&caller);
        let transaction = caller
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("TEST_CODE audit insert failure transaction");
        let error =
            append_acquisition_in_transaction(&transaction, &varied_record(0, "available", true))
                .expect_err("TEST_CODE audit insert must fail");
        assert_safe_error(error, "audit row insert");
        assert!(!transaction.is_autocommit());
        assert_eq!(audit_state(&transaction), before);
        transaction
            .rollback()
            .expect("TEST_CODE rollback audit insert failure");
        assert_eq!(audit_state(&caller), before);
    }

    {
        let (_root, database) = audit_database();
        let original = varied_record(0, "available", true);
        let original_receipt = legacy_append(&database, &original);
        let mut caller = open_caller_connection(&database);
        caller
            .execute_batch(
                "CREATE TRIGGER test_abort_chain_insert
                 BEFORE INSERT ON data_acquisition_audit_chain
                 BEGIN SELECT RAISE(ABORT, 'TEST_CODE_SECRET_chain_insert'); END;",
            )
            .expect("TEST_CODE chain insert abort trigger");
        let before = audit_state(&caller);
        let transaction = caller
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("TEST_CODE chain insert failure transaction");
        let error = append_acquisition_in_transaction(
            &transaction,
            &varied_record(1, "verified_empty", true),
        )
        .expect_err("TEST_CODE chain insert must fail");
        assert_safe_error(error, "chain row insert");
        assert!(!transaction.is_autocommit());
        assert_eq!(row_counts(&transaction), (2, 1));
        transaction
            .rollback()
            .expect("TEST_CODE rollback chain insert failure");
        assert_eq!(audit_state(&caller), before);
        drop(caller);
        let verified = read_with_rusqlite(&database, &original_receipt);
        assert_record_matches(verified.record(), &original);
    }
}

#[test]
fn candidate_receipt_is_not_commit_proof_when_real_commit_is_busy() {
    let (_root, database) = audit_database();
    let original = varied_record(0, "available", true);
    let original_receipt = legacy_append(&database, &original);
    let mut caller = open_caller_connection(&database);
    let mode: String = caller
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .expect("TEST_CODE force delete journal");
    assert_eq!(mode.to_ascii_lowercase(), "delete");
    caller
        .busy_timeout(std::time::Duration::ZERO)
        .expect("TEST_CODE zero caller busy timeout");
    let mut observer = open_caller_connection(&database);
    let observer_transaction = observer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .expect("TEST_CODE observer read transaction");
    assert_eq!(row_counts(&observer_transaction), (1, 1));

    let candidate_record = varied_record(1, "verified_empty", true);
    let transaction = caller
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("TEST_CODE commit failure writer transaction");
    let candidate = append_acquisition_in_transaction(&transaction, &candidate_record)
        .expect("TEST_CODE append commit-failure candidate");
    assert_eq!(row_counts(&transaction), (2, 2));
    let commit_error = transaction
        .commit()
        .expect_err("TEST_CODE read lock must fail actual COMMIT");
    assert!(matches!(
        commit_error,
        rusqlite::Error::SqliteFailure(ref failure, _)
            if failure.code == rusqlite::ErrorCode::DatabaseBusy
    ));
    assert!(caller.is_autocommit());
    assert_eq!(row_counts(&caller), (1, 1));
    observer_transaction
        .rollback()
        .expect("TEST_CODE release observer read lock");
    assert!(observer.is_autocommit());
    drop(observer);
    drop(caller);

    let mut reopened = open_caller_connection(&database);
    let verification = reopened
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .expect("TEST_CODE reopen after failed commit");
    assert!(read_acquisition_in_transaction(&verification, &candidate).is_err());
    let original_verified = read_acquisition_in_transaction(&verification, &original_receipt)
        .expect("TEST_CODE old receipt survives failed commit");
    assert_record_matches(original_verified.record(), &original);
    verification
        .rollback()
        .expect("TEST_CODE end failed-commit verification");
    drop(reopened);

    let retried = caller_append(&database, &candidate_record);
    let sqlite = read_with_rusqlite(&database, &retried);
    assert_record_matches(sqlite.record(), &candidate_record);
    assert_diesel_reader(&database, &retried, &candidate_record);
}

#[test]
fn missing_tables_and_query_only_connections_are_rejected_without_repair() {
    for (table, operation) in [
        ("data_acquisition_audit", "audit tail read"),
        ("data_acquisition_audit_chain", "chain tail read"),
    ] {
        let (_root, database) = audit_database();
        let mut caller = open_caller_connection(&database);
        caller
            .execute_batch(&format!("DROP TABLE {table};"))
            .expect("TEST_CODE drop required audit table");
        let before = catalog(&caller);
        let transaction = caller
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .expect("TEST_CODE missing-table append transaction");
        let error =
            append_acquisition_in_transaction(&transaction, &varied_record(0, "available", true))
                .expect_err("TEST_CODE missing table must reject append");
        assert_safe_error(error, operation);
        assert!(!transaction.is_autocommit());
        assert_eq!(catalog(&transaction), before);
        transaction
            .rollback()
            .expect("TEST_CODE caller ends missing-table transaction");
        assert_eq!(catalog(&caller), before);
        assert_eq!(
            caller
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get::<_, i64>(0),
                )
                .expect("TEST_CODE missing table remains absent"),
            0
        );
    }

    let (_root, database) = audit_database();
    let mut caller = open_caller_connection(&database);
    caller
        .execute_batch("PRAGMA query_only=ON;")
        .expect("TEST_CODE query-only audit connection");
    let before_catalog = catalog(&caller);
    let before_state = audit_state(&caller);
    let transaction = caller
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .expect("TEST_CODE query-only append transaction");
    assert_eq!(
        transaction
            .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
            .expect("TEST_CODE query-only transaction state"),
        1
    );
    let error =
        append_acquisition_in_transaction(&transaction, &varied_record(0, "available", true))
            .expect_err("TEST_CODE query-only append must fail");
    assert_safe_error(error, "audit row insert");
    assert!(!transaction.is_autocommit());
    assert_eq!(catalog(&transaction), before_catalog);
    assert_eq!(audit_state(&transaction), before_state);
    transaction
        .rollback()
        .expect("TEST_CODE caller ends query-only transaction");
    assert!(caller.is_autocommit());
    assert_eq!(
        caller
            .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
            .expect("TEST_CODE query-only connection remains read-only"),
        1
    );
    assert_eq!(catalog(&caller), before_catalog);
    assert_eq!(audit_state(&caller), before_state);
}
