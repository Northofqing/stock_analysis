use super::super::super::super::super::tests as fixtures;
use super::super::tests as v1_fixtures;
use super::*;
use rusqlite::{params, Connection};
use std::os::unix::fs::FileExt;
use std::os::unix::io::AsRawFd;

const SEED: &str = r#"{"account_id":"TEST_CODE_ACCOUNT","epoch_id":"TEST_CODE_EPOCH","command_id":"seed\\一\n","cutover_at":"2026-07-02T07:00:00Z","account_effective_at":"2026-07-02T07:00:00Z","positions_effective_at":"2026-07-02T07:00:00Z","source_reference":"TEST_CODE_snapshot","source_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","approved_by":"TEST_CODE_approval","cash":100000000000,"original_total":100000000000,"excluded_residual":null,"lots":[],"marks":[],"policy":{"max_position_bps":1000,"cash_floor_bps":1500,"max_slippage_bps":200}}"#;
// Independently generated with Python compact UTF-8 JSON and SHA-256.
const GOLDEN: &str = "c09bff125e419d71d992e11853bc49caeb27566f7a4cf81652c080296fd2df7b";

fn prepared(original: rows::VerifiedUnapprovedOriginalRowsBackup) -> ManifestContentFrame {
    let mut v1 = v1_fixtures::prepared(original);
    assert!(v1.advance_content());
    assert!(v1
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
    ManifestContentFrame::new(v1)
}
fn checked(frame: &mut ManifestContentFrame) {
    assert!(frame.v1.audit.raw.phase == RawV1AuditLinksPhase::Complete);
    assert!(
        frame.v1.audit.raw.content_hashes
            == RawV1AuditContentHashes::AuditAndV1EventProjectionAndManifestChecked
    );
    assert!(
        frame.facts.started
            && frame.facts.callee_reached
            && frame.facts.callee_returned == Some(true)
            && frame.facts.return_retained
    );
    assert!(matches!(frame.result, Some(Ok(()))) && frame.unaccepted.is_none());
    assert_eq!(
        frame.facts.checked_accounts,
        frame.v1.audit.raw.input.fields.rows[0].len()
    );
    assert_eq!(frame.manifests.len(), frame.facts.checked_accounts);
    assert_eq!(
        frame.facts.tail_account,
        frame.facts.checked_accounts.checked_sub(1)
    );
    for (row, manifest) in frame.v1.audit.raw.input.fields.rows[0]
        .iter()
        .zip(&frame.manifests)
    {
        let binding = manifest.binding().unwrap();
        assert_eq!(binding.account_id, row.text(0).unwrap());
        assert_eq!(binding.epoch_id, row.text(1).unwrap());
        assert_eq!(binding.manifest_hash, row.text(2).unwrap());
    }
    let ro = &mut frame.v1.audit.raw.input.genesis.owner.fee.local.readonly;
    assert!(ro.reader.is_none() && ro.active.is_none());
    assert!(ro
        .facts
        .iter()
        .all(|f| f.closed && f.original_tail_validated));
    assert_eq!(ro.loan().unwrap().3, 2);
}

#[test]
fn task6_manifest_content_same_owner_warm_and_cold() {
    fixtures::task6_with_cold_rows_backup_fixture_for_test(
        |original| {
            let owner = v1_fixtures::transformed(original);
            let fd = owner.frame.base.target().unwrap().as_raw_fd();
            let mut owner = match owner.into_v1_manifest_content_hashes() {
                Ok(owner) => owner,
                Err(held) => panic!("manifest warm: {}", held.first_error()),
            };
            checked(&mut owner.frame);
            assert!(owner.frame.facts.checked_accounts > 0);
            let base = &owner
                .frame
                .v1
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
                owner.frame.v1.audit.raw.input.fields.rows.clone(),
                owner.frame.manifests.clone(),
            );
            drop(owner);
            saved
        },
        |(node, records, fields, manifests), original| {
            let mut owner = match AdditiveStorageLocalV1ManifestContentChecked::create_or_resume(
                original.into_additive_target_source().unwrap(),
            ) {
                Ok(owner) => owner,
                Err(held) => panic!("manifest cold: {}", held.first_error()),
            };
            checked(&mut owner.frame);
            let base = &owner
                .frame
                .v1
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
            assert_eq!(owner.frame.v1.audit.raw.input.fields.rows, fields);
            assert_eq!(owner.frame.manifests, manifests);
            assert!(owner
                .frame
                .v1
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

fn connection(bytes: &str, hash: &str) -> Connection {
    let c = v1_fixtures::fixed_connection();
    c.execute("DELETE FROM paper_ledger_account", []).unwrap();
    c.execute(
        "INSERT INTO paper_ledger_account VALUES('TEST_CODE_ACCOUNT','TEST_CODE_EPOCH',?1,?2)",
        params![hash, bytes],
    )
    .unwrap();
    c
}
fn read_and_check(
    c: &Connection,
    work: &mut target::TargetWork,
) -> (StorageResult<()>, ManifestContentFacts, Vec<SeedManifest>) {
    let fields = v1_fixtures::read_fields(c, work);
    let mut facts = ManifestContentFacts::default();
    let mut manifests = Vec::new();
    let result = manifest_content_hashes(&fields, work, &mut facts, &mut manifests);
    (result, facts, manifests)
}

#[test]
fn task6_manifest_content_normalizes_historical_wire() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut source = original.into_additive_target_source().unwrap();
        let work = source.storage_parts().unwrap().2;
        let original: SeedManifest = serde_json::from_str(SEED).unwrap();
        assert_eq!(original.binding().unwrap().manifest_hash, GOLDEN);
        let mut reordered: serde_json::Value = serde_json::from_str(SEED).unwrap();
        reordered
            .as_object_mut()
            .unwrap()
            .remove("excluded_residual");
        reordered["cutover_at"] = "2026-07-02T15:00:00+08:00".into();
        reordered["account_effective_at"] = "2026-07-02T15:00:00+08:00".into();
        // Historical Deserialize ignores unknown fields; they are not hash input.
        reordered["unknown_historical_field"] = serde_json::json!({"nested": [1, 2]});
        for bytes in [
            SEED.to_owned(),
            serde_json::to_string_pretty(&reordered).unwrap(),
        ] {
            assert_ne!(hex::encode(Sha256::digest(bytes.as_bytes())), GOLDEN);
            let c = connection(&bytes, GOLDEN);
            let (result, facts, manifests) = read_and_check(&c, work);
            result.unwrap();
            assert_eq!(facts.checked_accounts, 1);
            assert_eq!(manifests, vec![original.clone()]);
            c.close().unwrap();
        }
        let c = connection(SEED, GOLDEN);
        c.execute("DELETE FROM paper_ledger_account", []).unwrap();
        let (result, facts, manifests) = read_and_check(&c, work);
        result.unwrap();
        assert_eq!(facts.checked_accounts, 0);
        assert!(manifests.is_empty());
        c.close().unwrap();
        drop(source);
    });
}

#[test]
fn task6_manifest_content_sql_identity_and_dto_tampering() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut source = original.into_additive_target_source().unwrap();
        let work = source.storage_parts().unwrap().2;
        for case in [
            "hash",
            "raw_hash",
            "uppercase_hash",
            "account",
            "epoch",
            "cash",
            "malformed",
            "fractional_money",
            "duplicate",
            "unknown_only",
            "empty_source",
            "bad_source_hash",
            "effective_time",
        ] {
            let mut value: serde_json::Value = serde_json::from_str(SEED).unwrap();
            match case {
                "cash" => value["cash"] = 1.into(),
                "fractional_money" => value["cash"] = serde_json::json!(1.5),
                "empty_source" => value["source_reference"] = "  ".into(),
                "bad_source_hash" => value["source_hash"] = "z".repeat(64).into(),
                "effective_time" => value["positions_effective_at"] = "2026-07-03T07:00:00Z".into(),
                _ => {}
            }
            let bytes = match case {
                "malformed" => "not json".to_owned(),
                "duplicate" => SEED.replacen("{", "{\"account_id\":\"duplicated\",", 1),
                "unknown_only" => "{\"future\":1}".to_owned(),
                _ => serde_json::to_string(&value).unwrap(),
            };
            // Rehash invalid seed identities so this cannot pass by hash equality.
            let hash = if matches!(case, "empty_source" | "bad_source_hash" | "effective_time") {
                serde_json::from_str::<SeedManifest>(&bytes)
                    .unwrap()
                    .binding()
                    .unwrap()
                    .manifest_hash
            } else {
                GOLDEN.to_owned()
            };
            let c = connection(&bytes, &hash);
            match case {
                "hash" => {
                    c.execute(
                        "UPDATE paper_ledger_account SET manifest_hash=?1",
                        ["f".repeat(64)],
                    )
                    .unwrap();
                }
                "raw_hash" => {
                    c.execute(
                        "UPDATE paper_ledger_account SET manifest_hash=?1",
                        [hex::encode(Sha256::digest(bytes.as_bytes()))],
                    )
                    .unwrap();
                }
                "uppercase_hash" => {
                    c.execute(
                        "UPDATE paper_ledger_account SET manifest_hash=upper(manifest_hash)",
                        [],
                    )
                    .unwrap();
                }
                "account" => {
                    c.execute("UPDATE paper_ledger_account SET account_id='other'", [])
                        .unwrap();
                }
                "epoch" => {
                    c.execute("UPDATE paper_ledger_account SET epoch_id='other'", [])
                        .unwrap();
                }
                _ => {}
            }
            let (result, facts, manifests) = read_and_check(&c, work);
            let expected = if matches!(
                case,
                "malformed" | "fractional_money" | "duplicate" | "unknown_only"
            ) {
                "additive manifest historical DTO invalid"
            } else if matches!(case, "account" | "epoch") {
                "additive manifest account/epoch differs"
            } else if matches!(case, "empty_source" | "bad_source_hash" | "effective_time") {
                "additive manifest seed identity/effective time invalid"
            } else {
                "additive manifest normalized hash differs"
            };
            assert!(
                matches!(result.unwrap_err(), GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == expected),
                "{case}"
            );
            assert_eq!(facts.checked_accounts, 0);
            assert_eq!(facts.tail_account, None);
            assert_eq!(
                manifests.len(),
                usize::from(expected != "additive manifest historical DTO invalid")
            );
            c.close().unwrap();
        }
        let c = connection(SEED, GOLDEN);
        let mut last: SeedManifest = serde_json::from_str(SEED).unwrap();
        last.account_id = "TEST_CODE_Z_LAST".into();
        let hash = last.binding().unwrap().manifest_hash;
        // Preserve the original binding while changing the last DTO's cash.
        last.cash = crate::trading::paper_ledger::Money::from_cny(1.0).unwrap();
        c.execute(
            "INSERT INTO paper_ledger_account VALUES('TEST_CODE_Z_LAST','TEST_CODE_EPOCH',?1,?2)",
            params![hash, serde_json::to_string(&last).unwrap()],
        )
        .unwrap();
        let (result, facts, manifests) = read_and_check(&c, work);
        assert!(
            matches!(result.unwrap_err(), GlobalSchemaV1Error::SelectionSnapshotChanged { detail } if detail == "additive manifest normalized hash differs")
        );
        assert_eq!(facts.checked_accounts, 1);
        assert_eq!(facts.tail_account, Some(0));
        assert_eq!(manifests.len(), 2);
        c.close().unwrap();
        drop(source);
    });
}

#[test]
fn task6_manifest_content_budget_and_return_ownership() {
    for entry_short in [true, false] {
        fixtures::task6_with_actual_rows_backup_for_test(|original| {
            let mut frame = prepared(original);
            let allowance =
                manifest_allowance(frame.v1.audit.raw.input.fields.rows[0][0].text(3).unwrap())
                    .unwrap();
            let work = frame
                .v1
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
            assert_eq!(
                frame.facts.callee_returned,
                if entry_short { None } else { Some(false) }
            );
            assert!(frame.manifests.is_empty() && frame.facts.checked_accounts == 0);
            if !entry_short {
                assert!(frame.result.is_none() && frame.facts.return_retained);
                assert!(frame
                    .retain(Err(storage_fail("TEST_CODE duplicate")))
                    .is_err());
            }
            let ro = &mut frame.v1.audit.raw.input.genesis.owner.fee.local.readonly;
            assert!(ro.reader.is_none() && matches!(ro.cleanup_close, Some(Ok(()))));
            assert!(!ro.facts[1].original_tail_validated);
            let used = ro.loan().unwrap().2.metadata_used();
            let first = ro.transform.first.as_ref().unwrap() as *const GlobalSchemaV1Error;
            assert!(!frame.finish() && frame.evaluate().is_none());
            let ro = &mut frame.v1.audit.raw.input.genesis.owner.fee.local.readonly;
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
                frame.v1.audit.raw.input.fields.rows[0][0].cells[2] =
                    Some(V1AuditInputCell::Text("changed".into()));
            }
            let pointer = frame.v1.audit.raw.input.fields.rows[0][0]
                .text(3)
                .unwrap()
                .as_ptr();
            assert!(frame.retain(Ok(())).is_err() && frame.begin());
            let actual = frame.evaluate().unwrap();
            assert_eq!(actual.is_err(), late_error);
            assert!(!frame.manifests.is_empty() && frame.evaluate().is_none());
            assert!(frame
                .retain(if late_error {
                    Ok(())
                } else {
                    Err(storage_fail("TEST_CODE wrong return"))
                })
                .is_err());
            let first = frame.close_and_tail().unwrap_err();
            frame.v1.audit.raw.fail(first);
            let first_ptr = frame
                .v1
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
                frame.v1.audit.raw.input.fields.rows[0][0]
                    .text(3)
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
                .v1
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
            let ro = &mut frame.v1.audit.raw.input.genesis.owner.fee.local.readonly;
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
fn task6_manifest_content_requires_live_v1_return() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = ManifestContentFrame::new(v1_fixtures::prepared(original));
        assert!(!frame.begin() && frame.evaluate().is_none());
        assert!(frame.manifests.is_empty() && !frame.facts.started);
        assert!(frame.v1.audit.raw.content_hashes == RawV1AuditContentHashes::AuditOnlyChecked);
        drop(frame);
    });
}

#[test]
fn task6_manifest_content_busy_close_and_target_tail() {
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = prepared(original);
        assert!(frame.begin());
        let actual = frame.evaluate().unwrap();
        frame.retain(actual).unwrap();
        frame
            .v1
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
        frame.v1.audit.raw.fail(first);
        let ro = &mut frame.v1.audit.raw.input.genesis.owner.fee.local.readonly;
        assert!(ro.reader.is_some());
        let used = ro.loan().unwrap().2.metadata_used();
        assert!(!frame.finish());
        let ro = &mut frame.v1.audit.raw.input.genesis.owner.fee.local.readonly;
        assert_eq!(ro.loan().unwrap().2.metadata_used(), used);
        assert!(ro.finalize_busy_once());
        ro.cleanup_reader_once();
        assert!(matches!(ro.cleanup_close, Some(Ok(()))) && !ro.facts[1].original_tail_validated);
        assert!(!frame.manifests.is_empty() && matches!(frame.result, Some(Ok(()))));
        drop(frame);
    });
    fixtures::task6_with_actual_rows_backup_for_test(|original| {
        let mut frame = prepared(original);
        assert!(frame.begin());
        let actual = frame.evaluate().unwrap();
        frame.retain(actual).unwrap();
        let file = frame
            .v1
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
            .v1
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
        file.sync_all().unwrap();
        assert!(
            matches!(&actual, Err(GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) if detail == "additive readonly retained target bytes changed")
        );
        frame.v1.audit.raw.fail(actual.unwrap_err());
        assert!(!frame.finish());
        assert!(frame
            .v1
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
        assert!(!frame.manifests.is_empty());
        drop(frame);
    });
}
