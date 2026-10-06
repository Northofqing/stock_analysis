use super::super::super::tests as fixtures;
use super::*;
use crate::database::order_audit::{canonical_order_audit_record_hash, CanonicalOrderAuditRow};
use rusqlite::{params, Connection};
use std::os::unix::fs::FileExt;
use std::os::unix::io::AsRawFd;

fn transformed(original: rows::VerifiedUnapprovedOriginalRowsBackup) -> AdditiveStorageTransformed {
    let copied =
        match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
            Ok(owner) => owner,
            Err(held) => panic!("audit content copy: {}", held.first_error()),
        };
    match copied.into_transformed() {
        Ok(owner) => owner,
        Err(held) => panic!("audit content transform: {}", held.first_error()),
    }
}
fn prepared(original: rows::VerifiedUnapprovedOriginalRowsBackup) -> AuditContentFrame {
    let mut frame = AuditContentFrame::new(RawV1AuditLinksFrame::new(transformed(original).frame));
    assert!(frame.raw.prepare(false) && frame.raw.advance_links());
    frame
}
fn checked(frame: &mut AuditContentFrame) {
    assert!(frame.raw.phase == RawV1AuditLinksPhase::Complete && frame.raw.all_returns());
    assert!(frame.raw.content_hashes == RawV1AuditContentHashes::AuditOnlyChecked);
    assert!(
        frame.facts.started
            && frame.facts.callee_reached
            && frame.facts.callee_returned == Some(true)
    );
    assert!(matches!(frame.result, Some(Ok(()))) && frame.unaccepted.is_none());
    assert_eq!(
        frame.facts.checked_rows,
        frame.raw.input.fields.rows[3].len()
    );
    assert_eq!(
        frame.facts.tail_row,
        frame.facts.checked_rows.checked_sub(1)
    );
    let readonly = &mut frame.raw.input.genesis.owner.fee.local.readonly;
    assert!(readonly.reader.is_none() && readonly.active.is_none());
    assert!(readonly
        .facts
        .iter()
        .all(|r| r.closed && r.original_tail_validated));
    assert_eq!(readonly.loan().unwrap().3, 2);
}
#[test]
fn task6_audit_content_same_owner_warm_and_cold() {
    fixtures::task6_with_cold_rows_backup_fixture_for_test(
        |original| {
            let owner = transformed(original);
            let fd = owner.frame.base.target().unwrap().as_raw_fd();
            let mut checked_owner = match owner.into_audit_content_hashes() {
                Ok(owner) => owner,
                Err(held) => panic!("audit content warm: {}", held.first_error()),
            };
            checked(&mut checked_owner.frame);
            assert!(checked_owner.frame.facts.checked_rows > 0);
            let base = &checked_owner
                .frame
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .base;
            assert_eq!(base.target().unwrap().as_raw_fd(), fd);
            let saved = (
                base.target_node.unwrap(),
                base.records
                    .iter()
                    .flatten()
                    .map(|r| (r.node, r.bytes.clone()))
                    .collect::<Vec<_>>(),
                checked_owner.frame.raw.input.fields.rows.clone(),
            );
            drop(checked_owner);
            saved
        },
        |(node, records, fields), original| {
            let mut owner = match AdditiveStorageLocalAuditContentChecked::create_or_resume(
                original.into_additive_target_source().unwrap(),
            ) {
                Ok(owner) => owner,
                Err(held) => panic!("audit content cold: {}", held.first_error()),
            };
            checked(&mut owner.frame);
            let base = &owner
                .frame
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .base;
            assert_eq!(base.target_node, Some(node));
            assert_eq!(
                base.records
                    .iter()
                    .flatten()
                    .map(|r| (r.node, r.bytes.clone()))
                    .collect::<Vec<_>>(),
                records
            );
            assert_eq!(owner.frame.raw.input.fields.rows, fields);
            assert!(owner
                .frame
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .begin_return
                .is_none());
            drop(owner);
        },
    );
}

fn audit_row(id: i64) -> CanonicalOrderAuditRow {
    CanonicalOrderAuditRow {
        id,
        business_order_id: format!("order{id}"),
        source: "source".into(),
        decision_basis: "审计\n依据".into(),
        side: "legacy-side".into(),
        code: "code".into(),
        requested_price: 10.0,
        execution_price: Some(9.75),
        quantity: -1,
        quote_observed_at: None,
        outcome: "legacy-outcome".into(),
        failure_reason: Some("bad\tresponse".into()),
        created_at: "legacy-time".into(),
    }
}
fn fixed_connection() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE order_audit(id,business_order_id,source,decision_basis,side,code,requested_price,execution_price,quantity,quote_observed_at,outcome,failure_reason,created_at);
        CREATE TABLE order_audit_chain(order_audit_id,previous_hash,record_hash);").unwrap();
    let mut previous = AUDIT_CHAIN_GENESIS.to_owned();
    for id in [1, 2] {
        let row = audit_row(id);
        let hash = canonical_order_audit_record_hash(&previous, &row).unwrap();
        c.execute(
            "INSERT INTO order_audit VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                row.id,
                row.business_order_id,
                row.source,
                row.decision_basis,
                row.side,
                row.code,
                row.requested_price,
                row.execution_price,
                row.quantity,
                row.quote_observed_at,
                row.outcome,
                row.failure_reason,
                row.created_at
            ],
        )
        .unwrap();
        c.execute(
            "INSERT INTO order_audit_chain VALUES(?1,?2,?3)",
            params![id, previous, hash],
        )
        .unwrap();
        previous = hash;
    }
    c
}
fn read_fields(c: &Connection, work: &mut target::TargetWork) -> V1AuditInputFields {
    let mut fields = V1AuditInputFields::default();
    for query in [V1AuditInputQuery::Audits, V1AuditInputQuery::Chain] {
        let mut facts = V1AuditInputReadFacts::default();
        V1AuditInputReadLoan {
            connection: c,
            work,
            rows: &mut fields.rows[query.slot()],
            facts: &mut facts,
        }
        .read(query)
        .unwrap();
        assert!(facts.eof && facts.scopes_ended && facts.returned == Some(true));
    }
    fields
}
#[test]
fn task6_audit_content_rejects_real_sql_changes_after_links_pass() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut source = original.into_additive_target_source().unwrap();
        let work = source.storage_parts().unwrap().2;
        for case in [
            "plain",
            "basis",
            "real",
            "optional_null",
            "quantity",
            "created",
            "hash",
            "second",
            "empty",
        ] {
            let c = fixed_connection();
            c.execute_batch(match case {
                "basis" => "UPDATE order_audit SET decision_basis='different' WHERE id=1;",
                "real" => "UPDATE order_audit SET requested_price=10.125 WHERE id=1;",
                "optional_null" => "UPDATE order_audit SET failure_reason=NULL WHERE id=1;",
                "quantity" => "UPDATE order_audit SET quantity=100 WHERE id=1;",
                "created" => "UPDATE order_audit SET created_at='new time' WHERE id=1;",
                "hash" => "UPDATE order_audit_chain SET record_hash=upper(record_hash) WHERE order_audit_id=2;",
                "second" => "UPDATE order_audit SET code='other' WHERE id=2;",
                "empty" => "DELETE FROM order_audit; DELETE FROM order_audit_chain;", _ => "",
            }).unwrap();
            let fields = read_fields(&c, work);
            let mut links = RawV1AuditGateFacts::default();
            // These mutations retain the genuine predecessor chain, yet alter its content.
            raw_audit_predecessor_links(&fields, &mut links).unwrap();
            let mut facts = AuditContentFacts::default();
            let actual = audit_content_hashes(&fields, work, &mut facts);
            if matches!(case, "plain" | "empty") {
                actual.unwrap();
                assert_eq!(facts.checked_rows, if case == "plain" { 2 } else { 0 });
            } else {
                assert!(
                    matches!(actual.unwrap_err(), GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
                    if detail == "additive audit content hash differs")
                );
                assert_eq!(
                    facts.checked_rows,
                    usize::from(matches!(case, "hash" | "second"))
                );
            }
            c.close().unwrap();
        }
        drop(source);
    });
}

#[test]
fn task6_audit_content_historical_wire_golden_and_scope() {
    let row = audit_row(1);
    let expected = "{\"id\":1,\"business_order_id\":\"order1\",\"source\":\"source\",\"decision_basis\":\"审计\\n依据\",\"side\":\"legacy-side\",\"code\":\"code\",\"requested_price\":10.0,\"execution_price\":9.75,\"quantity\":-1,\"quote_observed_at\":null,\"outcome\":\"legacy-outcome\",\"failure_reason\":\"bad\\tresponse\",\"created_at\":\"legacy-time\"}";
    assert_eq!(serde_json::to_vec(&row).unwrap(), expected.as_bytes());
    let mut hash = Sha256::new();
    hash.update(b"BR086_ORDER_AUDIT_V1\0");
    hash.update(AUDIT_CHAIN_GENESIS.as_bytes());
    hash.update(b"\0");
    hash.update(expected.as_bytes());
    assert_eq!(
        canonical_order_audit_record_hash(AUDIT_CHAIN_GENESIS, &row).unwrap(),
        hex::encode(hash.finalize())
    );
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = prepared(original);
        // Explicit scope control: changing V1 payload does not claim a V1 content check.
        frame.raw.input.fields.rows[1][0].cells[5] =
            Some(V1AuditInputCell::Text("unchecked V1 payload".into()));
        assert!(frame.finish());
        checked(&mut frame);
        assert_eq!(
            frame.raw.input.fields.rows[1][0].text(5).unwrap(),
            "unchecked V1 payload"
        );
        drop(frame);
    });
}

#[test]
fn task6_audit_content_same_budget_and_owned_late_returns() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = prepared(original);
        let work = frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .loan()
            .unwrap()
            .2;
        work.metadata(16 * MIB - work.metadata_used()).unwrap();
        assert!(!frame.finish() && !frame.facts.started && !frame.facts.callee_reached);
        assert!(
            matches!(frame.raw.input.genesis.owner.fee.local.readonly.transform.first.as_ref().unwrap(),
            GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == "target metadata work exceeded before allocation")
        );
        let used = frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .loan()
            .unwrap()
            .2
            .metadata_used();
        assert!(!frame.finish());
        assert_eq!(
            frame
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used(),
            used
        );
        let readonly = &frame.raw.input.genesis.owner.fee.local.readonly;
        assert!(readonly.reader.is_none() && readonly.active.is_none());
        assert!(matches!(readonly.cleanup_close, Some(Ok(()))));
        assert!(!readonly.facts[1].original_tail_validated);
        assert!(frame
            .raw
            .input
            .fields
            .rows
            .iter()
            .all(|rows| !rows.is_empty()));
        drop(frame);
    });
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = prepared(original);
        let allowance =
            row_allowance(&frame.raw.input.fields.rows[3][0], AUDIT_CHAIN_GENESIS).unwrap();
        let pointer = frame.raw.input.fields.rows[3][0].text(1).unwrap().as_ptr();
        let work = frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .loan()
            .unwrap()
            .2;
        // Entry is paid, but the first row's serializer/hash allowance is short by one.
        let remaining = 16 * MIB - work.metadata_used();
        work.metadata(remaining - 256 - allowance + 1).unwrap();
        assert!(frame.begin());
        let actual = frame.evaluate().unwrap();
        assert!(actual.is_err());
        frame.retain(actual).unwrap();
        assert_eq!(frame.facts.callee_returned, Some(false));
        assert_eq!(frame.facts.checked_rows, 0);
        assert!(frame.facts.tail_row.is_none());
        assert_eq!(
            frame.raw.input.fields.rows[3][0].text(1).unwrap().as_ptr(),
            pointer
        );
        assert!(frame.raw.content_hashes == RawV1AuditContentHashes::NotChecked);
        let readonly = &mut frame.raw.input.genesis.owner.fee.local.readonly;
        assert!(readonly.reader.is_none() && matches!(readonly.cleanup_close, Some(Ok(()))));
        assert!(!readonly.facts[1].original_tail_validated);
        let used = readonly.loan().unwrap().2.metadata_used();
        assert!(!frame.finish() && frame.evaluate().is_none());
        assert_eq!(
            frame
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used(),
            used
        );
        drop(frame);
    });
    for late_error in [false, true] {
        fixtures::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = prepared(original);
            if late_error {
                frame.raw.input.fields.rows[3][0].cells[3] =
                    Some(V1AuditInputCell::Text("changed basis".into()));
            }
            let pointer = frame.raw.input.fields.rows[3][0].text(1).unwrap().as_ptr();
            assert!(frame.retain(Ok(())).is_err() && frame.begin());
            let actual = frame.evaluate().unwrap();
            assert_eq!(actual.is_err(), late_error);
            assert!(frame.evaluate().is_none());
            let contradiction = if late_error {
                Ok(())
            } else {
                Err(storage_fail("TEST_CODE wrong return"))
            };
            assert!(frame.retain(contradiction).is_err());
            let first = frame.close_and_tail().unwrap_err();
            frame.raw.fail(first);
            let first_ptr = frame
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .transform
                .first
                .as_ref()
                .unwrap() as *const GlobalSchemaV1Error;
            frame.retain(actual).unwrap();
            assert_eq!(frame.result.as_ref().unwrap().is_err(), late_error);
            assert_eq!(
                frame.raw.input.fields.rows[3][0].text(1).unwrap().as_ptr(),
                pointer
            );
            let duplicate = storage_fail("TEST_CODE duplicate owned return");
            let detail_ptr = match &duplicate {
                GlobalSchemaV1Error::SelectionSnapshotChanged { detail } => detail.as_ptr(),
                _ => unreachable!(),
            };
            let same = frame.retain(Err(duplicate)).unwrap_err();
            assert!(
                matches!(&same, Err(GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) if detail.as_ptr() == detail_ptr)
            );
            frame.unaccepted = Some(same);
            let used = frame
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used();
            assert!(!frame.finish() && frame.evaluate().is_none());
            assert_eq!(
                frame
                    .raw
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .loan()
                    .unwrap()
                    .2
                    .metadata_used(),
                used
            );
            assert_eq!(
                frame
                    .raw
                    .input
                    .genesis
                    .owner
                    .fee
                    .local
                    .readonly
                    .transform
                    .first
                    .as_ref()
                    .unwrap() as *const GlobalSchemaV1Error,
                first_ptr
            );
            assert!(
                !frame.raw.input.genesis.owner.fee.local.readonly.facts[1].original_tail_validated
            );
            drop(frame);
        });
    }
}

#[test]
fn task6_audit_content_retains_busy_close_and_rejects_target_drift() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = prepared(original);
        assert!(frame.begin());
        let actual = frame.evaluate().unwrap();
        frame.retain(actual).unwrap();
        frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .prepare_busy_vm();
        let first = frame.close_and_tail().unwrap_err();
        assert!(
            matches!(&first, GlobalSchemaV1Error::SelectionSqlite { operation: "close additive readonly", source }
            if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
        );
        frame.raw.fail(first);
        assert!(frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .reader
            .is_some());
        let used = frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .loan()
            .unwrap()
            .2
            .metadata_used();
        assert!(!frame.finish());
        assert_eq!(
            frame
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .loan()
                .unwrap()
                .2
                .metadata_used(),
            used
        );
        assert!(frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .finalize_busy_once());
        frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .cleanup_reader_once();
        assert!(matches!(
            frame
                .raw
                .input
                .genesis
                .owner
                .fee
                .local
                .readonly
                .cleanup_close,
            Some(Ok(()))
        ));
        assert!(!frame.raw.input.genesis.owner.fee.local.readonly.facts[1].original_tail_validated);
        drop(frame);
    });
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = prepared(original);
        assert!(frame.begin());
        let actual = frame.evaluate().unwrap();
        frame.retain(actual).unwrap();
        let file = frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .base
            .target()
            .unwrap();
        let mut old = [0];
        file.read_exact_at(&mut old, 100).unwrap();
        file.write_all_at(&[old[0] ^ 1], 100).unwrap();
        file.sync_all().unwrap();
        let actual = frame.close_and_tail();
        let file = frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .transform
            .base
            .target()
            .unwrap();
        file.write_all_at(&old, 100).unwrap();
        file.sync_all().unwrap(); // Isolated fixture cleanup.
        let first = actual.unwrap_err();
        assert!(
            matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail }
            if detail == "additive readonly retained target bytes changed")
        );
        frame.raw.fail(first);
        assert!(!frame.finish());
        assert!(frame
            .raw
            .input
            .genesis
            .owner
            .fee
            .local
            .readonly
            .reader
            .is_none());
        drop(frame);
    });
}
