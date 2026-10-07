use super::super::super::super::tests as fixtures;
use super::*;
use crate::trading::paper_replay_codec_v1::HistoryOutput;
use rusqlite::{params, Connection};
use std::os::unix::fs::FileExt;
use std::os::unix::io::AsRawFd;

pub(super) fn transformed(
    original: rows::VerifiedUnapprovedOriginalRowsBackup,
) -> AdditiveStorageTransformed {
    let copied =
        match AdditiveStorageCopied::create(original.into_additive_target_source().unwrap()) {
            Ok(owner) => owner,
            Err(held) => panic!("V1 content copy: {}", held.first_error()),
        };
    match copied.into_transformed() {
        Ok(owner) => owner,
        Err(held) => panic!("V1 content transform: {}", held.first_error()),
    }
}
pub(super) fn prepared(original: rows::VerifiedUnapprovedOriginalRowsBackup) -> V1ContentFrame {
    let mut frame = V1ContentFrame::new(RawV1AuditLinksFrame::new(transformed(original).frame));
    assert!(
        frame.audit.raw.prepare(false)
            && frame.audit.raw.advance_links()
            && frame.audit.advance_content()
    );
    assert!(frame
        .audit
        .raw
        .input
        .genesis
        .owner
        .fee
        .local
        .readonly
        .reader
        .is_some());
    frame
}
fn checked(frame: &mut V1ContentFrame) {
    assert!(
        frame.audit.raw.phase == RawV1AuditLinksPhase::Complete && frame.audit.raw.all_returns()
    );
    assert!(
        frame.audit.raw.content_hashes == RawV1AuditContentHashes::AuditAndV1EventProjectionChecked
    );
    assert!(
        frame.facts.started
            && frame.facts.callee_reached
            && frame.facts.callee_returned == Some(true)
    );
    assert!(matches!(frame.result, Some(Ok(()))) && frame.unaccepted.is_none());
    assert_eq!(
        frame.facts.checked_events,
        frame.audit.raw.input.fields.rows[1].len()
    );
    assert_eq!(
        frame.facts.checked_heads,
        frame.audit.raw.input.fields.rows[2].len()
    );
    assert_eq!(
        frame.facts.tail_event,
        frame.facts.checked_events.checked_sub(1)
    );
    assert_eq!(
        frame.facts.tail_head,
        frame.facts.checked_heads.checked_sub(1)
    );
    let ro = &mut frame.audit.raw.input.genesis.owner.fee.local.readonly;
    assert!(ro.reader.is_none() && ro.active.is_none());
    assert!(ro
        .facts
        .iter()
        .all(|f| f.closed && f.original_tail_validated));
    assert_eq!(ro.loan().unwrap().3, 2);
}
#[test]
fn task6_v1_content_same_owner_warm_and_cold() {
    fixtures::task6_with_cold_rows_backup_fixture_for_test(
        |original| {
            let owner = transformed(original);
            let fd = owner.frame.base.target().unwrap().as_raw_fd();
            let mut owner = match owner.into_v1_event_projection_content_hashes() {
                Ok(owner) => owner,
                Err(held) => panic!("V1 content warm: {}", held.first_error()),
            };
            checked(&mut owner.frame);
            assert!(owner.frame.facts.checked_events > 0 && owner.frame.facts.checked_heads > 0);
            let base = &owner
                .frame
                .audit
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
                owner.frame.audit.raw.input.fields.rows.clone(),
            );
            drop(owner);
            saved
        },
        |(node, records, fields), original| {
            let mut owner =
                match AdditiveStorageLocalV1EventProjectionContentChecked::create_or_resume(
                    original.into_additive_target_source().unwrap(),
                ) {
                    Ok(owner) => owner,
                    Err(held) => panic!("V1 content cold: {}", held.first_error()),
                };
            checked(&mut owner.frame);
            let base = &owner
                .frame
                .audit
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
            assert_eq!(owner.frame.audit.raw.input.fields.rows, fields);
            assert!(owner
                .frame
                .audit
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

pub(super) fn fixed_connection() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE TABLE paper_ledger_account(account_id,epoch_id,manifest_hash,manifest_bytes);
        CREATE TABLE paper_ledger_event(account_id,seq,command_id,previous_hash,event_hash,payload,business_plan_id,intent_hash,is_terminal,paper_trade_id,order_audit_id);
        CREATE TABLE paper_ledger_head(account_id,version,event_hash,projection_bytes,projection_hash);").unwrap();
    for account in ["a", "b"] {
        c.execute("INSERT INTO paper_ledger_account VALUES(?1,'epoch','unchecked manifest','not a manifest')", [account]).unwrap();
        let mut previous = "PAPER_LEDGER_GENESIS_V1".to_owned();
        for seq in [1, 2] {
            let command = format!("command\n{seq}一");
            let payload = format!("{{\"escaped\":\"\\u0000一{seq}\"}}");
            let input = HistoryOutput::Event {
                account,
                seq,
                command: &command,
                previous: &previous,
                payload: &payload,
            };
            let hash = hex::encode(input.digest(&input.historical_bytes().unwrap()));
            c.execute(
                "INSERT INTO paper_ledger_event VALUES(?1,?2,?3,?4,?5,?6,NULL,NULL,0,NULL,NULL)",
                params![account, seq, command, previous, hash, payload],
            )
            .unwrap();
            previous = hash;
        }
        // The raw projection digest does not establish DTO validity or replay.
        let projection = format!("stored projection {account}一\n");
        let hash = hex::encode(Sha256::digest(projection.as_bytes()));
        c.execute(
            "INSERT INTO paper_ledger_head VALUES(?1,2,?2,?3,?4)",
            params![account, previous, projection, hash],
        )
        .unwrap();
    }
    c
}
pub(super) fn read_fields(c: &Connection, work: &mut target::TargetWork) -> V1AuditInputFields {
    let mut fields = V1AuditInputFields::default();
    for query in [
        V1AuditInputQuery::Accounts,
        V1AuditInputQuery::Events,
        V1AuditInputQuery::Heads,
    ] {
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
fn task6_v1_content_sql_tampering_and_explicit_scope() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut source = original.into_additive_target_source().unwrap();
        let work = source.storage_parts().unwrap().2;
        for case in [
            "plain",
            "command",
            "payload",
            "last_payload",
            "projection",
            "projection_hash",
            "uppercase_event",
            "unhashed_metadata",
            "manifest",
            "empty",
        ] {
            let c = fixed_connection();
            c.execute_batch(match case {
                "command" => "UPDATE paper_ledger_event SET command_id='changed' WHERE account_id='a' AND seq=1;",
                "payload" => "UPDATE paper_ledger_event SET payload='changed' WHERE account_id='a' AND seq=1;",
                "last_payload" => "UPDATE paper_ledger_event SET payload='changed' WHERE account_id='b' AND seq=2;",
                "projection" => "UPDATE paper_ledger_head SET projection_bytes='changed' WHERE account_id='b';",
                "projection_hash" => "UPDATE paper_ledger_head SET projection_hash=upper(projection_hash) WHERE account_id='b';",
                "uppercase_event" => "UPDATE paper_ledger_event SET event_hash=upper(event_hash) WHERE account_id='b' AND seq=2; UPDATE paper_ledger_head SET event_hash=upper(event_hash) WHERE account_id='b';",
                "unhashed_metadata" => "UPDATE paper_ledger_event SET business_plan_id='changed',intent_hash='changed',is_terminal=1,paper_trade_id=999,order_audit_id=888;",
                "manifest" => "UPDATE paper_ledger_account SET manifest_bytes='changed',manifest_hash='also changed';",
                "empty" => "DELETE FROM paper_ledger_event; DELETE FROM paper_ledger_head; DELETE FROM paper_ledger_account;", _ => "",
            }).unwrap();
            let fields = read_fields(&c, work);
            // Every content attack still passes the genuine predecessor/head scan.
            raw_v1_event_head_links(&fields, &mut RawV1AuditGateFacts::default()).unwrap();
            let mut facts = V1ContentFacts::default();
            let actual = v1_event_projection_hashes(&fields, work, &mut facts);
            if matches!(case, "plain" | "unhashed_metadata" | "manifest" | "empty") {
                actual.unwrap();
                assert_eq!(facts.checked_events, if case == "empty" { 0 } else { 4 });
                assert_eq!(facts.checked_heads, if case == "empty" { 0 } else { 2 });
            } else {
                let projection = matches!(case, "projection" | "projection_hash");
                let detail = if projection {
                    "additive V1 projection content hash differs"
                } else {
                    "additive V1 event content hash differs"
                };
                assert!(
                    matches!(actual.unwrap_err(), GlobalSchemaV1Error::SelectionSnapshotChanged { detail: actual } if actual == detail)
                );
                assert_eq!(
                    facts.checked_events,
                    if projection {
                        4
                    } else if matches!(case, "last_payload" | "uppercase_event") {
                        3
                    } else {
                        0
                    }
                );
                assert_eq!(facts.checked_heads, usize::from(projection));
            }
            c.close().unwrap();
        }
        drop(source);
    });
}
#[test]
fn task6_v1_content_historical_wire_golden() {
    // Independently generated JSON/SHA-256 golden (Python UTF-8, compact JSON).
    let input = HistoryOutput::Event {
        account: "account\\一",
        seq: 1,
        command: "command\n\"",
        previous: "GENESIS",
        payload: "{\"value\":\"\\u0000一\"}",
    };
    let bytes = input.historical_bytes().unwrap();
    assert_eq!(
        hex::encode(input.digest(&bytes)),
        "e0e744665db40548e9da9da2358185319e9c7bbbcdc5ac1c8c394b9d59b42f14"
    );
    let mut writer = AuditHashWriter {
        hash: Sha256::new(),
        remaining: 4096,
    };
    serde_json::to_writer(
        &mut writer,
        &(
            "PAPER_EVENT_V1",
            "account\\一",
            1i64,
            "command\n\"",
            "GENESIS",
            "{\"value\":\"\\u0000一\"}",
        ),
    )
    .unwrap();
    assert!(digest_matches(
        writer.hash,
        "e0e744665db40548e9da9da2358185319e9c7bbbcdc5ac1c8c394b9d59b42f14"
    )
    .unwrap());
}

#[test]
fn task6_v1_content_same_budget_and_late_returns() {
    // Entry payment and first-row payment fail before hash/serializer work.
    for entry_short in [true, false] {
        fixtures::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = prepared(original);
            let allowance = row_allowance(&frame.audit.raw.input.fields.rows[1][0], "").unwrap();
            let work = frame
                .audit
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
            let remaining = 16 * MIB - work.metadata_used();
            work.metadata(if entry_short {
                remaining
            } else {
                remaining - 256 - allowance + 1
            })
            .unwrap();
            assert!(!frame.finish());
            assert_eq!(frame.facts.started, !entry_short);
            assert_eq!(frame.facts.callee_reached, !entry_short);
            assert_eq!(
                frame.facts.callee_returned,
                if entry_short { None } else { Some(false) }
            );
            assert_eq!(frame.facts.checked_events, 0);
            if !entry_short {
                // The actual Err has moved into first, yet a duplicate stays rejected.
                assert!(frame.result.is_none() && frame.facts.return_retained);
                assert!(frame
                    .retain(Err(storage_fail("TEST_CODE second failed return")))
                    .is_err());
            }
            assert!(frame.audit.raw.content_hashes == RawV1AuditContentHashes::AuditOnlyChecked);
            let ro = &mut frame.audit.raw.input.genesis.owner.fee.local.readonly;
            assert!(ro.reader.is_none() && matches!(ro.cleanup_close, Some(Ok(()))));
            assert!(!ro.facts[1].original_tail_validated);
            let used = ro.loan().unwrap().2.metadata_used();
            let first = ro.transform.first.as_ref().unwrap() as *const GlobalSchemaV1Error;
            assert!(!frame.finish() && frame.evaluate().is_none());
            let ro = &mut frame.audit.raw.input.genesis.owner.fee.local.readonly;
            assert_eq!(ro.loan().unwrap().2.metadata_used(), used);
            assert_eq!(
                ro.transform.first.as_ref().unwrap() as *const GlobalSchemaV1Error,
                first
            );
            drop(frame);
        });
    }
    for late_error in [false, true] {
        fixtures::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = prepared(original);
            if late_error {
                frame.audit.raw.input.fields.rows[1][0].cells[5] =
                    Some(V1AuditInputCell::Text("changed".into()));
            }
            let pointer = frame.audit.raw.input.fields.rows[1][0]
                .text(5)
                .unwrap()
                .as_ptr();
            assert!(frame.retain(Ok(())).is_err() && frame.begin());
            let actual = frame.evaluate().unwrap();
            assert_eq!(actual.is_err(), late_error);
            assert!(frame.evaluate().is_none());
            assert!(frame
                .retain(if late_error {
                    Ok(())
                } else {
                    Err(storage_fail("TEST_CODE wrong return"))
                })
                .is_err());
            let first = frame.close_and_tail().unwrap_err();
            frame.audit.raw.fail(first);
            let first_ptr = frame
                .audit
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
                frame.audit.raw.input.fields.rows[1][0]
                    .text(5)
                    .unwrap()
                    .as_ptr(),
                pointer
            );
            let duplicate = storage_fail("TEST_CODE duplicate");
            let ptr = match &duplicate {
                GlobalSchemaV1Error::SelectionSnapshotChanged { detail } => detail.as_ptr(),
                _ => unreachable!(),
            };
            let same = frame.retain(Err(duplicate)).unwrap_err();
            assert!(
                matches!(&same, Err(GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) if detail.as_ptr() == ptr)
            );
            frame.unaccepted = Some(same);
            let used = frame
                .audit
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
            let ro = &mut frame.audit.raw.input.genesis.owner.fee.local.readonly;
            assert_eq!(
                ro.transform.first.as_ref().unwrap() as *const GlobalSchemaV1Error,
                first_ptr
            );
            assert_eq!(ro.loan().unwrap().2.metadata_used(), used);
            assert!(!ro.facts[1].original_tail_validated);
            drop(frame);
        });
    }
}

#[test]
fn task6_v1_content_requires_audit_return_before_scan() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = V1ContentFrame::new(RawV1AuditLinksFrame::new(transformed(original).frame));
        assert!(frame.audit.raw.prepare(false) && frame.audit.raw.advance_links());
        // Genuine typed rows and links are insufficient without actual audit content.
        assert!(!frame.begin() && !frame.facts.started && !frame.facts.callee_reached);
        assert!(frame.evaluate().is_none() && frame.result.is_none());
        assert!(frame.audit.raw.content_hashes == RawV1AuditContentHashes::NotChecked);
        drop(frame);
    });
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = V1ContentFrame::new(RawV1AuditLinksFrame::new(transformed(original).frame));
        assert!(frame.audit.raw.prepare(false) && frame.audit.raw.advance_links());
        frame.audit.raw.input.fields.rows[3][0].cells[3] =
            Some(V1AuditInputCell::Text("bad audit content".into()));
        assert!(!frame.audit.advance_content());
        let first = frame
            .audit
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
        assert!(!frame.finish() && !frame.facts.started && !frame.facts.callee_reached);
        assert_eq!(
            frame
                .audit
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
            first
        );
        assert!(frame.result.is_none() && frame.audit.facts.return_retained);
        assert!(frame.audit.raw.content_hashes == RawV1AuditContentHashes::NotChecked);
        drop(frame);
    });
}

#[test]
fn task6_v1_content_busy_close_and_target_tail() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = prepared(original);
        assert!(frame.begin());
        let actual = frame.evaluate().unwrap();
        frame.retain(actual).unwrap();
        frame
            .audit
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
            matches!(&first, GlobalSchemaV1Error::SelectionSqlite { operation: "close additive readonly", source } if source.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy))
        );
        frame.audit.raw.fail(first);
        let ro = &mut frame.audit.raw.input.genesis.owner.fee.local.readonly;
        assert!(ro.reader.is_some());
        let used = ro.loan().unwrap().2.metadata_used();
        assert!(!frame.finish());
        let ro = &mut frame.audit.raw.input.genesis.owner.fee.local.readonly;
        assert_eq!(ro.loan().unwrap().2.metadata_used(), used);
        assert!(ro.finalize_busy_once());
        ro.cleanup_reader_once();
        assert!(matches!(ro.cleanup_close, Some(Ok(()))) && !ro.facts[1].original_tail_validated);
        assert!(matches!(frame.result, Some(Ok(()))));
        drop(frame);
    });
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = prepared(original);
        assert!(frame.begin());
        let actual = frame.evaluate().unwrap();
        frame.retain(actual).unwrap();
        let file = frame
            .audit
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
            .audit
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
            matches!(&first, GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == "additive readonly retained target bytes changed")
        );
        frame.audit.raw.fail(first);
        assert!(!frame.finish());
        assert!(frame
            .audit
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
