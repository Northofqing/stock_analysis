use super::*;

fn fixture(ids: &[i64]) -> (tempfile::TempDir, std::path::PathBuf, String) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("TEST_CODE_paging.db");
    let tail = performance_tests::seed(&path, ids);
    (root, path, tail)
}
fn tamper(path: &std::path::Path, sql: &str) {
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(
        "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
    DROP TRIGGER trg_data_acquisition_audit_no_update;
    DROP TRIGGER trg_data_acquisition_audit_no_delete;
    DROP TRIGGER trg_data_acquisition_audit_chain_no_update;
    DROP TRIGGER trg_data_acquisition_audit_chain_no_delete;",
    )
    .unwrap();
    db.execute_batch(sql).unwrap();
}
fn open(path: &std::path::Path) -> SqliteConnection {
    SqliteConnection::establish(path.to_str().unwrap()).unwrap()
}
#[test]
fn paging_matches_vector_for_empty_boundaries_gaps_and_signed_first_ids() {
    for count in [0, 1, 2, 3, 4, 5, 7] {
        let ids: Vec<i64> = (1..=count).collect();
        let (_root, path, tail) = fixture(&ids);
        let mut db = open(&path);
        for size in [1, 2, 3] {
            assert_eq!(
                validate_audit_chain_paged(&mut db, size, || {}).unwrap(),
                tail
            );
        }
        assert_eq!(
            validate_data_acquisition_audit_chain_rows(
                &load_audit_rows(&mut db).unwrap(),
                &load_chain_rows(&mut db).unwrap()
            )
            .unwrap(),
            tail
        );
    }
    for ids in [
        vec![i64::MIN, -9, 0, 5, 100],
        vec![-7, 0, 3],
        vec![0, 2, 10],
    ] {
        let (_root, path, tail) = fixture(&ids);
        let mut db = open(&path);
        assert_eq!(validate_audit_chain_paged(&mut db, 2, || {}).unwrap(), tail);
        assert!(validate_audit_chain_paged(&mut db, 0, || {}).is_err());
        assert!(validate_audit_chain_paged(&mut db, 1025, || {}).is_err());
    }
}
#[test]
fn paging_detects_every_corruption_position_and_linkage_schema_damage() {
    for id in [1, 2, 3, 4, 7] {
        for sql in [format!("UPDATE data_acquisition_audit SET source='tampered' WHERE id={id}"),format!("UPDATE data_acquisition_audit_chain SET record_hash='tampered' WHERE acquisition_audit_id={id}")] {
            let (_root,path,_) = fixture(&[1,2,3,4,5,6,7]);
            tamper(&path,&sql);
            assert!(validate_audit_chain_paged(&mut open(&path),3,|| {}).is_err(),"{sql}");
        }
    }
    for sql in ["UPDATE data_acquisition_audit SET schema_version=2 WHERE id=4", "UPDATE data_acquisition_audit_chain SET previous_hash='tampered' WHERE acquisition_audit_id=4", "UPDATE data_acquisition_audit_chain SET previous_hash='tampered' WHERE acquisition_audit_id=1"] {
        let (_root,path,_) = fixture(&[1,2,3,4,5,6,7]); tamper(&path,sql);
        assert!(validate_audit_chain_paged(&mut open(&path),3,|| {}).is_err());
    }
}
#[test]
fn paging_detects_missing_extra_and_equal_length_mismatched_ids() {
    for sql in [
        "DELETE FROM data_acquisition_audit_chain WHERE acquisition_audit_id=1",
        "DELETE FROM data_acquisition_audit_chain WHERE acquisition_audit_id=3",
        "DELETE FROM data_acquisition_audit_chain WHERE acquisition_audit_id=4",
        "DELETE FROM data_acquisition_audit_chain WHERE acquisition_audit_id=7",
        "DELETE FROM data_acquisition_audit WHERE id=4",
        "INSERT INTO data_acquisition_audit_chain VALUES (8,'fake','fake','fixed')",
        "UPDATE data_acquisition_audit_chain SET acquisition_audit_id=-1 WHERE acquisition_audit_id=1",
        "UPDATE data_acquisition_audit_chain SET acquisition_audit_id=8 WHERE acquisition_audit_id=7",
        "UPDATE data_acquisition_audit_chain SET acquisition_audit_id=9 WHERE acquisition_audit_id=3",
        "UPDATE data_acquisition_audit_chain SET acquisition_audit_id=10 WHERE acquisition_audit_id=4",
        "DELETE FROM data_acquisition_audit; DELETE FROM data_acquisition_audit_chain WHERE acquisition_audit_id != 7",
        "DELETE FROM data_acquisition_audit_chain; DELETE FROM data_acquisition_audit WHERE id != 7",
    ] {
        let (_root,path,_) = fixture(&[1,2,3,4,5,6,7]); tamper(&path,sql);
        assert!(validate_audit_chain_paged(&mut open(&path),3,|| {}).is_err(),"{sql}");
    }
}
#[test]
fn paging_snapshot_excludes_commit_between_independent_table_pages() {
    let (_root, path, old_tail) = fixture(&[1, 2, 3, 4, 5, 6, 7]);
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    drop(writer);
    let mut reader = open(&path);
    let mut new_tail = None;
    let observed = validate_audit_chain_paged(&mut reader, 3, || {
        let mut writer = rusqlite::Connection::open(&path).unwrap();
        let tx = writer
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let receipt = append_acquisition_in_transaction(
            &tx,
            &record("available", Some("TEST_CODE_concurrent")),
        )
        .unwrap();
        new_tail = Some(receipt.record_hash);
        tx.commit().unwrap();
    })
    .unwrap();
    assert_eq!(observed, old_tail);
    assert_eq!(
        validate_audit_chain_paged(&mut reader, 3, || {}).unwrap(),
        new_tail.unwrap()
    );
    assert_eq!(load_audit_rows(&mut reader).unwrap().len(), 8);
}
#[test]
fn paging_preserves_historical_hash_semantics_without_new_admission() {
    let (_root, path, _) = fixture(&[1, 2, 3]);
    tamper(
        &path,
        "UPDATE data_acquisition_audit SET capability=' ', retryable=9 WHERE id=3",
    );
    let mut db = open(&path);
    let audits = load_audit_rows(&mut db).unwrap();
    let chain = load_chain_rows(&mut db).unwrap();
    let hash = calculate_record_hash(&chain[2].previous_hash, &audits[2]).unwrap();
    diesel::sql_query(
        "UPDATE data_acquisition_audit_chain SET record_hash=? WHERE acquisition_audit_id=3",
    )
    .bind::<Text, _>(&hash)
    .execute(&mut db)
    .unwrap();
    assert!(validate_record(&audits[2].record()).is_err());
    assert_eq!(validate_audit_chain_paged(&mut db, 2, || {}).unwrap(), hash);
    assert_eq!(
        validate_data_acquisition_audit_chain_rows(&audits, &load_chain_rows(&mut db).unwrap())
            .unwrap(),
        hash
    );
}
#[test]
fn paging_orders_by_key_not_fixture_insertion_order() {
    let (_root, path, tail) = fixture(&[1, 3, 5, 7]);
    tamper(&path,"CREATE TEMP TABLE saved_audit AS SELECT * FROM data_acquisition_audit;
        CREATE TEMP TABLE saved_chain AS SELECT * FROM data_acquisition_audit_chain;
        DELETE FROM data_acquisition_audit_chain; DELETE FROM data_acquisition_audit;
        INSERT INTO data_acquisition_audit SELECT * FROM saved_audit ORDER BY id DESC;
        INSERT INTO data_acquisition_audit_chain SELECT * FROM saved_chain ORDER BY acquisition_audit_id DESC;");
    assert_eq!(
        validate_audit_chain_paged(&mut open(&path), 2, || {}).unwrap(),
        tail
    );
}
