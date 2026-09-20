use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::types::Value;
use rusqlite::{params, Connection};

use crate::monitor::push_job::{raw_digest, MachineCatalog, Sha256Digest, UnitId};

use super::activation_codec::{
    journal_canonical_bytes, manifest_canonical_bytes, promotion_event_id,
};
use super::migration::FoundationSchemaMigration;
use super::{
    inspect_raw_activation_facts, ActivationInspectError, ActivationReconciliation,
    DesiredActivationState, PromotionAction,
};

const UNIT: &str = "MU-p01";
const OTHER_UNIT: &str = "MU-d01";
const MANIFEST_SHA: &str = "116c905e3d8fda9bb789150de90f790c4e9435e4a78672b5140d5db1f8179393";
const EVENT_ID: &str = "cd2dc9557629fe43fac56c3946aa3707b24d232e4175cf3bfa91519f994fc2e2";
const JOURNAL_SHA: &str = "09857ea72b66050fc5b683090a6d7a0c8c7f1605360e024a9a3e311eaab82fdf";

fn unit(value: &str) -> UnitId {
    UnitId::try_new(value.to_owned()).expect("TEST_CODE unit")
}

fn initialized_database(name: &str) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("TEST_CODE temp root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join(name);
    let migration = FoundationSchemaMigration::bundled().expect("TEST_CODE bundled schema");
    let ddl = migration
        .ddl_bytes()
        .strip_prefix(b".bail on\n")
        .expect("TEST_CODE library-safe DDL suffix");
    let ddl = std::str::from_utf8(ddl).expect("TEST_CODE UTF-8 DDL");
    let connection = Connection::open(&database).expect("TEST_CODE database");
    connection
        .execute_batch(ddl)
        .expect("TEST_CODE synthetic frozen schema");
    drop(connection);
    (root, database)
}

fn seeded_database(name: &str) -> (tempfile::TempDir, PathBuf) {
    let (root, database) = initialized_database(name);
    let connection = Connection::open(&database).expect("TEST_CODE seed connection");
    connection
        .execute(
            "INSERT INTO push_activation_manifests(\
             manifest_sha256,unit_id,generation,previous_manifest_sha256,desired_state,\
             physical_owner,build_commit,build_sha256,catalog_sha256,business_schema_sha256,\
             durable_schema_sha256,template_sha256,source_contract_sha256,evidence_sha256,\
             approved_by,approved_at,window_start,window_end,rollback_target_sha256,created_at\
             ) VALUES(?1,?2,1,NULL,'Disabled','owner-legacy',?3,?4,?5,?6,?7,?8,?9,?10,\
             'operator-a',10,10,100,NULL,11)",
            params![
                MANIFEST_SHA,
                UNIT,
                "a".repeat(40),
                "b".repeat(64),
                "c".repeat(64),
                "d".repeat(64),
                "e".repeat(64),
                "f".repeat(64),
                "1".repeat(64),
                "2".repeat(64),
            ],
        )
        .expect("TEST_CODE fixed manifest");
    connection
        .execute(
            "INSERT INTO push_promotion_journal(\
             event_id,unit_id,generation,from_manifest_sha256,to_manifest_sha256,actor,action,\
             reason,window_start,window_end,evidence_sha256,rollback_target_sha256,\
             previous_sha256,canonical_sha256,occurred_at\
             ) VALUES(?1,?2,1,NULL,?3,'operator-a','Initialize','activation.applied',10,100,\
             ?4,NULL,NULL,?5,12)",
            params![EVENT_ID, UNIT, MANIFEST_SHA, "2".repeat(64), JOURNAL_SHA],
        )
        .expect("TEST_CODE fixed journal");
    drop(connection);
    (root, database)
}

fn golden_manifest_bytes() -> Vec<u8> {
    format!(
        "ActivationManifestV1\0{{\"approved_at\":10,\"approved_by\":\"operator-a\",\
         \"build_commit\":\"{}\",\"build_sha256\":\"{}\",\
         \"business_schema_sha256\":\"{}\",\"catalog_sha256\":\"{}\",\
         \"created_at\":11,\"desired_state\":\"Disabled\",\
         \"durable_schema_sha256\":\"{}\",\"evidence_sha256\":\"{}\",\
         \"generation\":1,\"physical_owner\":\"owner-legacy\",\
         \"previous_manifest_sha256\":null,\"rollback_target_sha256\":null,\
         \"source_contract_sha256\":\"{}\",\"template_sha256\":\"{}\",\
         \"unit_id\":\"MU-p01\",\"window_end\":100,\"window_start\":10}}",
        "a".repeat(40),
        "b".repeat(64),
        "d".repeat(64),
        "c".repeat(64),
        "e".repeat(64),
        "2".repeat(64),
        "1".repeat(64),
        "f".repeat(64),
    )
    .into_bytes()
}

fn golden_journal_bytes() -> Vec<u8> {
    format!(
        "PromotionJournalV1\0{{\"action\":\"Initialize\",\"actor\":\"operator-a\",\
         \"event_id\":\"{EVENT_ID}\",\"evidence_sha256\":\"{}\",\
         \"from_manifest_sha256\":null,\"generation\":1,\"occurred_at\":12,\
         \"previous_sha256\":null,\"reason\":\"activation.applied\",\
         \"rollback_target_sha256\":null,\"to_manifest_sha256\":\"{MANIFEST_SHA}\",\
         \"unit_id\":\"MU-p01\",\"window_end\":100,\"window_start\":10}}",
        "2".repeat(64),
    )
    .into_bytes()
}

fn golden_promotion_id_bytes() -> &'static [u8] {
    b"PromotionV1\0{\"generation\":1,\"unit_id\":\"MU-p01\"}"
}

fn golden_second_manifest_bytes() -> Vec<u8> {
    format!(
        "ActivationManifestV1\0{{\"approved_at\":20,\"approved_by\":\"operator-a\",\
         \"build_commit\":\"{}\",\"build_sha256\":\"{}\",\
         \"business_schema_sha256\":\"{}\",\"catalog_sha256\":\"{}\",\
         \"created_at\":21,\"desired_state\":\"Shadow\",\
         \"durable_schema_sha256\":\"{}\",\"evidence_sha256\":\"{}\",\
         \"generation\":2,\"physical_owner\":\"owner-legacy\",\
         \"previous_manifest_sha256\":\"{MANIFEST_SHA}\",\
         \"rollback_target_sha256\":null,\"source_contract_sha256\":\"{}\",\
         \"template_sha256\":\"{}\",\"unit_id\":\"MU-p01\",\
         \"window_end\":200,\"window_start\":20}}",
        "a".repeat(40),
        "b".repeat(64),
        "d".repeat(64),
        "c".repeat(64),
        "e".repeat(64),
        "2".repeat(64),
        "1".repeat(64),
        "f".repeat(64),
    )
    .into_bytes()
}

fn insert_second_manifest(database: &Path) -> Sha256Digest {
    let sha256 = raw_digest(&golden_second_manifest_bytes());
    let connection = Connection::open(database).expect("TEST_CODE second manifest connection");
    connection
        .execute(
            "INSERT INTO push_activation_manifests(\
             manifest_sha256,unit_id,generation,previous_manifest_sha256,desired_state,\
             physical_owner,build_commit,build_sha256,catalog_sha256,business_schema_sha256,\
             durable_schema_sha256,template_sha256,source_contract_sha256,evidence_sha256,\
             approved_by,approved_at,window_start,window_end,rollback_target_sha256,created_at\
             ) VALUES(?1,?2,2,?3,'Shadow','owner-legacy',?4,?5,?6,?7,?8,?9,?10,?11,\
             'operator-a',20,20,200,NULL,21)",
            params![
                sha256.as_str(),
                UNIT,
                MANIFEST_SHA,
                "a".repeat(40),
                "b".repeat(64),
                "c".repeat(64),
                "d".repeat(64),
                "e".repeat(64),
                "f".repeat(64),
                "1".repeat(64),
                "2".repeat(64),
            ],
        )
        .expect("TEST_CODE independent second manifest");
    sha256
}

#[derive(Debug)]
struct GoldenChainEntry {
    manifest_sha256: Sha256Digest,
    journal_sha256: Sha256Digest,
    manifest_bytes: Vec<u8>,
    journal_bytes: Vec<u8>,
}

fn json_optional_digest(value: Option<&str>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| format!("\"{value}\""))
}

fn full_chain_manifest_bytes(
    generation: u64,
    desired_state: &str,
    previous_manifest_sha256: Option<&str>,
    rollback_target_sha256: Option<&str>,
) -> Vec<u8> {
    let approved_at = generation * 10;
    let created_at = approved_at + 1;
    let window_end = approved_at + 100;
    format!(
        "ActivationManifestV1\0{{\"approved_at\":{approved_at},\
         \"approved_by\":\"operator-a\",\"build_commit\":\"{}\",\
         \"build_sha256\":\"{}\",\"business_schema_sha256\":\"{}\",\
         \"catalog_sha256\":\"{}\",\"created_at\":{created_at},\
         \"desired_state\":\"{desired_state}\",\"durable_schema_sha256\":\"{}\",\
         \"evidence_sha256\":\"{}\",\"generation\":{generation},\
         \"physical_owner\":\"owner-legacy\",\"previous_manifest_sha256\":{},\
         \"rollback_target_sha256\":{},\"source_contract_sha256\":\"{}\",\
         \"template_sha256\":\"{}\",\"unit_id\":\"MU-p01\",\
         \"window_end\":{window_end},\"window_start\":{approved_at}}}",
        "a".repeat(40),
        "b".repeat(64),
        "d".repeat(64),
        "c".repeat(64),
        "e".repeat(64),
        "2".repeat(64),
        json_optional_digest(previous_manifest_sha256),
        json_optional_digest(rollback_target_sha256),
        "1".repeat(64),
        "f".repeat(64),
    )
    .into_bytes()
}

#[allow(clippy::too_many_arguments)]
fn full_chain_journal_bytes(
    generation: u64,
    action: &str,
    event_id: &str,
    from_manifest_sha256: Option<&str>,
    to_manifest_sha256: &str,
    rollback_target_sha256: Option<&str>,
    previous_sha256: Option<&str>,
) -> Vec<u8> {
    let window_start = generation * 10;
    let window_end = window_start + 100;
    let occurred_at = window_start + 2;
    format!(
        "PromotionJournalV1\0{{\"action\":\"{action}\",\"actor\":\"operator-a\",\
         \"event_id\":\"{event_id}\",\"evidence_sha256\":\"{}\",\
         \"from_manifest_sha256\":{},\"generation\":{generation},\
         \"occurred_at\":{occurred_at},\"previous_sha256\":{},\
         \"reason\":\"activation.applied\",\"rollback_target_sha256\":{},\
         \"to_manifest_sha256\":\"{to_manifest_sha256}\",\"unit_id\":\"MU-p01\",\
         \"window_end\":{window_end},\"window_start\":{window_start}}}",
        "2".repeat(64),
        json_optional_digest(from_manifest_sha256),
        json_optional_digest(previous_sha256),
        json_optional_digest(rollback_target_sha256),
    )
    .into_bytes()
}

fn seed_full_chain(database: &Path) -> Vec<GoldenChainEntry> {
    let states_and_actions = [
        ("Disabled", "Initialize"),
        ("Shadow", "EnterShadow"),
        ("Active", "Activate"),
        ("Draining", "Drain"),
        ("Disabled", "Disable"),
        ("Shadow", "Rollback"),
    ];
    let connection = Connection::open(database).expect("TEST_CODE full-chain connection");
    let mut entries = Vec::<GoldenChainEntry>::new();
    for (index, (state, action)) in states_and_actions.into_iter().enumerate() {
        let generation = index as u64 + 1;
        let previous_manifest_sha256 = entries.last().map(|entry| entry.manifest_sha256.as_str());
        let rollback_target_sha256 = (generation == 6).then(|| entries[1].manifest_sha256.as_str());
        let manifest_bytes = full_chain_manifest_bytes(
            generation,
            state,
            previous_manifest_sha256,
            rollback_target_sha256,
        );
        let manifest_sha256 = raw_digest(&manifest_bytes);
        let event_bytes =
            format!("PromotionV1\0{{\"generation\":{generation},\"unit_id\":\"MU-p01\"}}")
                .into_bytes();
        let event_id = raw_digest(&event_bytes);
        let previous_sha256 = entries.last().map(|entry| entry.journal_sha256.as_str());
        let journal_bytes = full_chain_journal_bytes(
            generation,
            action,
            event_id.as_str(),
            previous_manifest_sha256,
            manifest_sha256.as_str(),
            rollback_target_sha256,
            previous_sha256,
        );
        let journal_sha256 = raw_digest(&journal_bytes);
        let approved_at = generation * 10;
        let window_end = approved_at + 100;
        connection
            .execute(
                "INSERT INTO push_activation_manifests(\
                 manifest_sha256,unit_id,generation,previous_manifest_sha256,desired_state,\
                 physical_owner,build_commit,build_sha256,catalog_sha256,business_schema_sha256,\
                 durable_schema_sha256,template_sha256,source_contract_sha256,evidence_sha256,\
                 approved_by,approved_at,window_start,window_end,rollback_target_sha256,created_at\
                 ) VALUES(?1,?2,?3,?4,?5,'owner-legacy',?6,?7,?8,?9,?10,?11,?12,?13,\
                 'operator-a',?14,?14,?15,?16,?17)",
                params![
                    manifest_sha256.as_str(),
                    UNIT,
                    generation as i64,
                    previous_manifest_sha256,
                    state,
                    "a".repeat(40),
                    "b".repeat(64),
                    "c".repeat(64),
                    "d".repeat(64),
                    "e".repeat(64),
                    "f".repeat(64),
                    "1".repeat(64),
                    "2".repeat(64),
                    approved_at as i64,
                    window_end as i64,
                    rollback_target_sha256,
                    (approved_at + 1) as i64,
                ],
            )
            .expect("TEST_CODE independent full-chain manifest");
        connection
            .execute(
                "INSERT INTO push_promotion_journal(\
                 event_id,unit_id,generation,from_manifest_sha256,to_manifest_sha256,actor,\
                 action,reason,window_start,window_end,evidence_sha256,rollback_target_sha256,\
                 previous_sha256,canonical_sha256,occurred_at\
                 ) VALUES(?1,?2,?3,?4,?5,'operator-a',?6,'activation.applied',?7,?8,?9,\
                 ?10,?11,?12,?13)",
                params![
                    event_id.as_str(),
                    UNIT,
                    generation as i64,
                    previous_manifest_sha256,
                    manifest_sha256.as_str(),
                    action,
                    approved_at as i64,
                    window_end as i64,
                    "2".repeat(64),
                    rollback_target_sha256,
                    previous_sha256,
                    journal_sha256.as_str(),
                    (approved_at + 2) as i64,
                ],
            )
            .expect("TEST_CODE independent full-chain journal");
        entries.push(GoldenChainEntry {
            manifest_sha256,
            journal_sha256,
            manifest_bytes,
            journal_bytes,
        });
    }
    entries
}

#[test]
fn w16_three_domains_have_independent_exact_golden_bytes() {
    let (_root, database) = seeded_database("golden.sqlite3");
    let facts = inspect_raw_activation_facts(&database, &unit(UNIT)).expect("TEST_CODE raw facts");
    let selected = facts.selected_unit();
    let manifest = &selected.manifests()[0];
    let journal = &selected.journal()[0];

    assert_eq!(manifest_canonical_bytes(manifest), golden_manifest_bytes());
    assert_eq!(journal_canonical_bytes(journal), golden_journal_bytes());
    assert_eq!(
        promotion_event_id(UNIT, 1),
        raw_digest(golden_promotion_id_bytes())
    );
    assert_eq!(raw_digest(&golden_manifest_bytes()).as_str(), MANIFEST_SHA);
    assert_eq!(raw_digest(&golden_journal_bytes()).as_str(), JOURNAL_SHA);
    assert_eq!(raw_digest(golden_promotion_id_bytes()).as_str(), EVENT_ID);
}

#[test]
fn w16_complete_applied_chain_and_same_unit_rollback_load_all_raw_rows() {
    let (_root, database) = initialized_database("full-chain.sqlite3");
    let golden = seed_full_chain(&database);
    let facts = inspect_raw_activation_facts(&database, &unit(UNIT))
        .expect("TEST_CODE complete applied chain");
    let selected = facts.selected_unit();
    assert_eq!(
        selected.reconciliation(),
        ActivationReconciliation::CaughtUp { generation: 6 }
    );
    assert_eq!(selected.manifests().len(), 6);
    assert_eq!(selected.journal().len(), 6);
    assert_eq!(
        selected
            .journal()
            .iter()
            .map(|entry| entry.action())
            .collect::<Vec<_>>(),
        vec![
            PromotionAction::Initialize,
            PromotionAction::EnterShadow,
            PromotionAction::Activate,
            PromotionAction::Drain,
            PromotionAction::Disable,
            PromotionAction::Rollback,
        ]
    );
    assert_eq!(
        selected.manifests()[5].rollback_target_sha256(),
        Some(selected.manifests()[1].manifest_sha256())
    );
    for (index, expected) in golden.iter().enumerate() {
        assert_eq!(
            manifest_canonical_bytes(&selected.manifests()[index]),
            expected.manifest_bytes
        );
        assert_eq!(
            journal_canonical_bytes(&selected.journal()[index]),
            expected.journal_bytes
        );
    }
}

#[test]
fn w16_recomputed_middle_journal_predecessor_break_is_rejected() {
    let (_root, database) = initialized_database("middle-break.sqlite3");
    let golden = seed_full_chain(&database);
    let generation = 4;
    let wrong_previous = golden[1].journal_sha256.as_str();
    let event_id = raw_digest(
        format!("PromotionV1\0{{\"generation\":{generation},\"unit_id\":\"MU-p01\"}}").as_bytes(),
    );
    let replacement_bytes = full_chain_journal_bytes(
        generation,
        "Drain",
        event_id.as_str(),
        Some(golden[2].manifest_sha256.as_str()),
        golden[3].manifest_sha256.as_str(),
        None,
        Some(wrong_previous),
    );
    let replacement_sha = raw_digest(&replacement_bytes);
    let connection = Connection::open(&database).expect("TEST_CODE middle-break connection");
    let guard: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_promotion_journal_update'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE middle-break guard");
    connection
        .execute_batch("DROP TRIGGER push_promotion_journal_update;")
        .expect("TEST_CODE remove middle-break guard");
    connection
        .execute(
            "UPDATE push_promotion_journal SET previous_sha256=?1,canonical_sha256=?2 \
             WHERE unit_id=?3 AND generation=4",
            params![wrong_previous, replacement_sha.as_str(), UNIT],
        )
        .expect("TEST_CODE install recomputed middle break");
    connection
        .execute_batch(&guard)
        .expect("TEST_CODE restore middle-break guard");
    drop(connection);
    assert_eq!(
        inspect_raw_activation_facts(&database, &unit(UNIT)),
        Err(ActivationInspectError::InvalidFacts)
    );
}

#[test]
fn w16_complete_raw_facts_cover_catalog_and_preserve_non_authoritative_rows() {
    let (_root, database) = seeded_database("facts.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let facts = inspect_raw_activation_facts(&database, &unit(UNIT)).expect("TEST_CODE raw facts");

    assert_eq!(facts.units().len(), catalog.units().len());
    assert_eq!(
        facts
            .units()
            .iter()
            .map(|facts| facts.unit_id().as_str())
            .collect::<BTreeSet<_>>(),
        catalog
            .units()
            .iter()
            .map(|registration| registration.id().as_str())
            .collect::<BTreeSet<_>>()
    );
    let selected = facts.selected_unit();
    assert_eq!(
        selected.reconciliation(),
        ActivationReconciliation::CaughtUp { generation: 1 }
    );
    assert_eq!(
        selected.manifests()[0].desired_state(),
        DesiredActivationState::Disabled
    );
    assert_eq!(selected.manifests()[0].physical_owner(), "owner-legacy");
    assert_eq!(selected.manifests()[0].approved_by(), "operator-a");
    let historical_catalog_sha256 = "c".repeat(64);
    assert_eq!(
        selected.manifests()[0].catalog_sha256().as_str(),
        historical_catalog_sha256.as_str()
    );
    assert_ne!(
        selected.manifests()[0].catalog_sha256(),
        catalog.catalog_sha256(),
        "a self-consistent copied database remains raw facts even with a historical catalog hash"
    );
    assert_eq!(selected.journal()[0].action(), PromotionAction::Initialize);
    assert_eq!(selected.journal()[0].actor(), "operator-a");
    assert!(facts
        .units()
        .iter()
        .filter(|unit| unit.unit_id().as_str() != UNIT)
        .all(|unit| unit.reconciliation() == ActivationReconciliation::Unregistered));

    let debug = format!("{facts:?}");
    assert!(!debug.contains("owner-legacy"));
    assert!(!debug.contains("operator-a"));
    assert!(!debug.contains(&"a".repeat(40)));
}

fn replace_value(database: &Path, table: &str, guard: &str, column: &str, value: Value) {
    let connection = Connection::open(database).expect("TEST_CODE mutation connection");
    connection
        .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;")
        .expect("TEST_CODE mutation safeguards");
    let trigger_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
            [guard],
            |row| row.get(0),
        )
        .expect("TEST_CODE preserve exact guard");
    connection
        .execute_batch(&format!("DROP TRIGGER {guard};"))
        .expect("TEST_CODE remove immutable guard");
    connection
        .execute(&format!("UPDATE {table} SET {column}=?1"), [value])
        .expect("TEST_CODE mutate row");
    connection
        .execute_batch(&trigger_sql)
        .expect("TEST_CODE restore exact guard");
}

#[test]
fn w16_every_manifest_column_is_bound_and_illegal_types_and_nul_are_rejected() {
    let mutations = vec![
        ("manifest_sha256", Value::Text("9".repeat(64))),
        ("unit_id", Value::Text("rogue-unit".to_owned())),
        ("generation", Value::Integer(2)),
        ("previous_manifest_sha256", Value::Text("8".repeat(64))),
        ("desired_state", Value::Text("Shadow".to_owned())),
        ("physical_owner", Value::Text("owner-other".to_owned())),
        ("build_commit", Value::Text("b".repeat(40))),
        ("build_sha256", Value::Text("3".repeat(64))),
        ("catalog_sha256", Value::Text("4".repeat(64))),
        ("business_schema_sha256", Value::Text("5".repeat(64))),
        ("durable_schema_sha256", Value::Text("6".repeat(64))),
        ("template_sha256", Value::Text("7".repeat(64))),
        ("source_contract_sha256", Value::Text("8".repeat(64))),
        ("evidence_sha256", Value::Text("9".repeat(64))),
        ("approved_by", Value::Text("operator-b".to_owned())),
        ("approved_at", Value::Integer(9)),
        ("window_start", Value::Integer(9)),
        ("window_end", Value::Integer(101)),
        (
            "rollback_target_sha256",
            Value::Text(MANIFEST_SHA.to_owned()),
        ),
        ("created_at", Value::Integer(12)),
    ];
    for (index, (column, value)) in mutations.into_iter().enumerate() {
        let (_root, database) = seeded_database(&format!("manifest-{index}.sqlite3"));
        replace_value(
            &database,
            "push_activation_manifests",
            "push_activation_manifests_update",
            column,
            value,
        );
        assert_eq!(
            inspect_raw_activation_facts(&database, &unit(UNIT)),
            Err(ActivationInspectError::InvalidFacts),
            "manifest column {column} must be covered"
        );
    }

    for (index, value) in [
        Value::Blob(vec![1]),
        Value::Text("owner\0suffix".to_owned()),
    ]
    .into_iter()
    .enumerate()
    {
        let (_root, database) = seeded_database(&format!("manifest-type-{index}.sqlite3"));
        let column = if index == 0 {
            "generation"
        } else {
            "physical_owner"
        };
        replace_value(
            &database,
            "push_activation_manifests",
            "push_activation_manifests_update",
            column,
            value,
        );
        assert_eq!(
            inspect_raw_activation_facts(&database, &unit(UNIT)),
            Err(ActivationInspectError::InvalidFacts)
        );
    }
}

#[test]
fn w16_every_journal_column_is_bound_and_invalid_actor_window_reason_are_rejected() {
    let mutations = vec![
        ("event_id", Value::Text("9".repeat(64))),
        ("unit_id", Value::Text("rogue-unit".to_owned())),
        ("generation", Value::Integer(2)),
        ("from_manifest_sha256", Value::Text("8".repeat(64))),
        ("to_manifest_sha256", Value::Text("7".repeat(64))),
        ("actor", Value::Text("operator-b".to_owned())),
        ("action", Value::Text("Activate".to_owned())),
        ("reason", Value::Text("activation.wrong".to_owned())),
        ("window_start", Value::Integer(9)),
        ("window_end", Value::Integer(101)),
        ("evidence_sha256", Value::Text("6".repeat(64))),
        (
            "rollback_target_sha256",
            Value::Text(MANIFEST_SHA.to_owned()),
        ),
        ("previous_sha256", Value::Text("5".repeat(64))),
        ("canonical_sha256", Value::Text("4".repeat(64))),
        ("occurred_at", Value::Integer(13)),
    ];
    for (index, (column, value)) in mutations.into_iter().enumerate() {
        let (_root, database) = seeded_database(&format!("journal-{index}.sqlite3"));
        replace_value(
            &database,
            "push_promotion_journal",
            "push_promotion_journal_update",
            column,
            value,
        );
        assert_eq!(
            inspect_raw_activation_facts(&database, &unit(UNIT)),
            Err(ActivationInspectError::InvalidFacts),
            "journal column {column} must be covered"
        );
    }
}

fn drop_all_journal_rows(database: &Path) {
    let connection = Connection::open(database).expect("TEST_CODE pending connection");
    let trigger_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_promotion_journal_delete'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE preserve delete guard");
    connection
        .execute_batch(
            "DROP TRIGGER push_promotion_journal_delete; DELETE FROM push_promotion_journal;",
        )
        .expect("TEST_CODE create pending state");
    connection
        .execute_batch(&trigger_sql)
        .expect("TEST_CODE restore delete guard");
}

#[test]
fn w16_missing_unit_and_one_pending_generation_are_explicit_and_not_execution() {
    let (_empty_root, empty) = initialized_database("empty.sqlite3");
    let facts = inspect_raw_activation_facts(&empty, &unit(UNIT)).expect("TEST_CODE empty facts");
    assert_eq!(
        facts.selected_unit().reconciliation(),
        ActivationReconciliation::Unregistered
    );
    assert!(facts.selected_unit().manifests().is_empty());

    let (_pending_root, pending) = seeded_database("pending.sqlite3");
    drop_all_journal_rows(&pending);
    let facts =
        inspect_raw_activation_facts(&pending, &unit(UNIT)).expect("TEST_CODE pending facts");
    assert_eq!(
        facts.selected_unit().reconciliation(),
        ActivationReconciliation::Pending {
            executed_generation: None,
            pending_generation: 1,
        }
    );
    assert!(facts.selected_unit().journal().is_empty());
}

#[test]
fn w16_two_unjournaled_generations_skip_and_duplicate_attempts_are_rejected() {
    let (_gap_root, gap) = seeded_database("gap.sqlite3");
    drop_all_journal_rows(&gap);
    insert_second_manifest(&gap);
    assert_eq!(
        inspect_raw_activation_facts(&gap, &unit(UNIT)),
        Err(ActivationInspectError::InvalidFacts),
        "only one final generation may await reconciliation"
    );

    let (_skip_root, skip) = seeded_database("skip.sqlite3");
    insert_second_manifest(&skip);
    let facts = inspect_raw_activation_facts(&skip, &unit(UNIT)).expect("TEST_CODE pending gen2");
    let mut second = facts.selected_unit().manifests()[1].clone();
    second.generation = 3;
    let second_sha = super::activation_codec::manifest_digest(&second);
    let connection = Connection::open(&skip).expect("TEST_CODE skip connection");
    connection
        .execute_batch("PRAGMA ignore_check_constraints=ON;")
        .expect("TEST_CODE skip safeguards");
    let guard: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_activation_manifests_update'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE skip guard");
    connection
        .execute_batch("DROP TRIGGER push_activation_manifests_update;")
        .expect("TEST_CODE remove skip guard");
    connection
        .execute(
            "UPDATE push_activation_manifests SET generation=3,manifest_sha256=?1 \
             WHERE generation=2 AND unit_id=?2",
            params![second_sha.as_str(), UNIT],
        )
        .expect("TEST_CODE make self-consistent generation skip");
    connection
        .execute_batch(&guard)
        .expect("TEST_CODE restore skip guard");
    drop(connection);
    assert_eq!(
        inspect_raw_activation_facts(&skip, &unit(UNIT)),
        Err(ActivationInspectError::InvalidFacts)
    );

    let (_duplicate_root, duplicate) = seeded_database("duplicate.sqlite3");
    let connection = Connection::open(&duplicate).expect("TEST_CODE duplicate connection");
    assert!(connection
        .execute(
            "INSERT INTO push_promotion_journal SELECT * FROM push_promotion_journal",
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO push_activation_manifests SELECT * FROM push_activation_manifests",
            [],
        )
        .is_err());
}

#[test]
fn w16_recomputed_journal_still_rejects_wrong_actor_window_reason_and_action() {
    enum Mutation {
        Actor,
        Window,
        Reason,
        Action,
    }
    for (index, mutation) in [
        Mutation::Actor,
        Mutation::Window,
        Mutation::Reason,
        Mutation::Action,
    ]
    .into_iter()
    .enumerate()
    {
        let (_root, database) = seeded_database(&format!("journal-binding-{index}.sqlite3"));
        let facts = inspect_raw_activation_facts(&database, &unit(UNIT)).expect("TEST_CODE facts");
        let mut entry = facts.selected_unit().journal()[0].clone();
        let (column, value) = match mutation {
            Mutation::Actor => {
                entry.actor = "operator-b".to_owned();
                ("actor", Value::Text(entry.actor.clone()))
            }
            Mutation::Window => {
                entry.window_end = 101;
                ("window_end", Value::Integer(101))
            }
            Mutation::Reason => {
                entry.reason = "activation.wrong".to_owned();
                ("reason", Value::Text(entry.reason.clone()))
            }
            Mutation::Action => {
                entry.action = PromotionAction::Activate;
                ("action", Value::Text("Activate".to_owned()))
            }
        };
        let canonical_sha256 = super::activation_codec::journal_digest(&entry);
        let connection = Connection::open(&database).expect("TEST_CODE binding mutation");
        connection
            .execute_batch("PRAGMA ignore_check_constraints=ON;")
            .expect("TEST_CODE binding safeguards");
        let guard: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='push_promotion_journal_update'",
                [],
                |row| row.get(0),
            )
            .expect("TEST_CODE binding guard");
        connection
            .execute_batch("DROP TRIGGER push_promotion_journal_update;")
            .expect("TEST_CODE remove binding guard");
        connection
            .execute(
                &format!("UPDATE push_promotion_journal SET {column}=?1,canonical_sha256=?2"),
                params![value, canonical_sha256.as_str()],
            )
            .expect("TEST_CODE recomputed journal mutation");
        connection
            .execute_batch(&guard)
            .expect("TEST_CODE restore binding guard");
        drop(connection);
        assert_eq!(
            inspect_raw_activation_facts(&database, &unit(UNIT)),
            Err(ActivationInspectError::InvalidFacts)
        );
    }
}

#[test]
fn w16_recomputed_cross_unit_rollback_target_is_rejected() {
    let (_root, database) = seeded_database("cross-unit-rollback.sqlite3");
    let other_bytes = String::from_utf8(golden_manifest_bytes())
        .expect("TEST_CODE golden UTF-8")
        .replace(UNIT, OTHER_UNIT)
        .into_bytes();
    let other_sha = raw_digest(&other_bytes);
    let connection = Connection::open(&database).expect("TEST_CODE other unit connection");
    connection
        .execute(
            "INSERT INTO push_activation_manifests(\
             manifest_sha256,unit_id,generation,previous_manifest_sha256,desired_state,\
             physical_owner,build_commit,build_sha256,catalog_sha256,business_schema_sha256,\
             durable_schema_sha256,template_sha256,source_contract_sha256,evidence_sha256,\
             approved_by,approved_at,window_start,window_end,rollback_target_sha256,created_at\
             ) VALUES(?1,?2,1,NULL,'Disabled','owner-legacy',?3,?4,?5,?6,?7,?8,?9,?10,\
             'operator-a',10,10,100,NULL,11)",
            params![
                other_sha.as_str(),
                OTHER_UNIT,
                "a".repeat(40),
                "b".repeat(64),
                "c".repeat(64),
                "d".repeat(64),
                "e".repeat(64),
                "f".repeat(64),
                "1".repeat(64),
                "2".repeat(64),
            ],
        )
        .expect("TEST_CODE other unit manifest");
    drop(connection);
    insert_second_manifest(&database);

    let facts = inspect_raw_activation_facts(&database, &unit(UNIT)).expect("TEST_CODE raw facts");
    let mut second = facts.selected_unit().manifests()[1].clone();
    second.desired_state = DesiredActivationState::Disabled;
    second.rollback_target_sha256 = Some(other_sha.clone());
    let replacement_sha = super::activation_codec::manifest_digest(&second);
    let connection = Connection::open(&database).expect("TEST_CODE rollback mutation");
    connection
        .execute_batch("PRAGMA ignore_check_constraints=ON;")
        .expect("TEST_CODE rollback safeguards");
    let guard: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_activation_manifests_update'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE rollback guard");
    connection
        .execute_batch("DROP TRIGGER push_activation_manifests_update;")
        .expect("TEST_CODE remove rollback guard");
    connection
        .execute(
            "UPDATE push_activation_manifests SET desired_state='Disabled',\
             rollback_target_sha256=?1,manifest_sha256=?2 WHERE unit_id=?3 AND generation=2",
            params![other_sha.as_str(), replacement_sha.as_str(), UNIT],
        )
        .expect("TEST_CODE cross-unit rollback mutation");
    connection
        .execute_batch(&guard)
        .expect("TEST_CODE restore rollback guard");
    drop(connection);
    assert_eq!(
        inspect_raw_activation_facts(&database, &unit(UNIT)),
        Err(ActivationInspectError::InvalidFacts)
    );
}

#[test]
fn w16_schema_replacements_and_corruption_in_an_unselected_unit_fail_closed() {
    let (_trigger_root, trigger_database) = seeded_database("trigger.sqlite3");
    let connection = Connection::open(&trigger_database).expect("TEST_CODE trigger replacement");
    connection
        .execute_batch(
            "DROP TRIGGER push_promotion_journal_update; \
             CREATE TRIGGER push_promotion_journal_update BEFORE UPDATE ON push_promotion_journal \
             BEGIN SELECT 1; END;",
        )
        .expect("TEST_CODE same-name trigger replacement");
    drop(connection);
    assert_eq!(
        inspect_raw_activation_facts(&trigger_database, &unit(UNIT)),
        Err(ActivationInspectError::SchemaRejected)
    );

    let (_table_root, table_database) = seeded_database("table.sqlite3");
    let connection = Connection::open(&table_database).expect("TEST_CODE table replacement");
    let table_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master \
             WHERE type='table' AND name='push_activation_manifests'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE preserve table definition");
    let replacement_sql = table_sql.replacen("physical_owner TEXT", "physical_owner BLOB", 1);
    assert_ne!(replacement_sql, table_sql);
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF; \
             DROP TRIGGER push_activation_manifests_chain; \
             DROP TRIGGER push_activation_manifests_update; \
             DROP TRIGGER push_activation_manifests_delete; \
             DROP TABLE push_activation_manifests;",
        )
        .expect("TEST_CODE remove original table");
    connection
        .execute_batch(&replacement_sql)
        .expect("TEST_CODE install same-name table replacement");
    drop(connection);
    assert_eq!(
        inspect_raw_activation_facts(&table_database, &unit(UNIT)),
        Err(ActivationInspectError::SchemaRejected)
    );

    let (_other_root, other_database) = seeded_database("other-unit.sqlite3");
    replace_value(
        &other_database,
        "push_activation_manifests",
        "push_activation_manifests_update",
        "unit_id",
        Value::Text("corrupt-unregistered-unit".to_owned()),
    );
    assert_eq!(
        inspect_raw_activation_facts(&other_database, &unit(OTHER_UNIT)),
        Err(ActivationInspectError::InvalidFacts),
        "whole-database validation must precede the selected-unit projection"
    );
}

fn directory_names(root: &Path) -> BTreeSet<String> {
    fs::read_dir(root)
        .expect("TEST_CODE read directory")
        .map(|entry| {
            entry
                .expect("TEST_CODE directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

fn directory_bytes(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut entries = fs::read_dir(root)
        .expect("TEST_CODE read directory bytes")
        .map(|entry| {
            let entry = entry.expect("TEST_CODE byte directory entry");
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).expect("TEST_CODE entry bytes"),
            )
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

#[test]
fn w16_inspection_is_byte_and_sidecar_stable_and_rejects_missing_and_wal() {
    let (root, database) = seeded_database("stable.sqlite3");
    let before_bytes = fs::read(&database).expect("TEST_CODE before bytes");
    let before_names = directory_names(root.path());
    inspect_raw_activation_facts(&database, &unit(UNIT)).expect("TEST_CODE raw inspection");
    assert_eq!(
        fs::read(&database).expect("TEST_CODE after bytes"),
        before_bytes
    );
    assert_eq!(directory_names(root.path()), before_names);

    let missing = root.path().join("missing.sqlite3");
    assert_eq!(
        inspect_raw_activation_facts(&missing, &unit(UNIT)),
        Err(ActivationInspectError::ReadOnlySourceRejected)
    );
    assert!(!missing.exists());

    let (_wal_root, wal_database) = seeded_database("wal.sqlite3");
    let connection = Connection::open(&wal_database).expect("TEST_CODE WAL connection");
    let mode: String = connection
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .expect("TEST_CODE WAL mode");
    assert_eq!(mode, "wal");
    connection
        .execute_batch("PRAGMA wal_autocheckpoint=0; PRAGMA user_version=7;")
        .expect("TEST_CODE materialize WAL");
    assert_eq!(
        inspect_raw_activation_facts(&wal_database, &unit(UNIT)),
        Err(ActivationInspectError::ReadOnlySourceRejected)
    );
}

#[test]
fn w16_hot_rollback_journal_is_rejected_without_recovery_side_effects() {
    let (root, database) = seeded_database("hot-journal.sqlite3");
    let mut journal_name = database.as_os_str().to_os_string();
    journal_name.push("-journal");
    let journal = PathBuf::from(journal_name);
    let fault = Connection::open(&database).expect("TEST_CODE hot journal connection");
    fault
        .execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA cache_size=1; PRAGMA cache_spill=ON; \
             BEGIN IMMEDIATE; CREATE TABLE activation_hot_fault(value BLOB); \
             INSERT INTO activation_hot_fault(value) VALUES(zeroblob(262144));",
        )
        .expect("TEST_CODE materialize rollback journal");
    let journal_bytes = fs::read(&journal).expect("TEST_CODE capture hot journal");
    assert!(journal_bytes.len() > 512);
    assert_eq!(
        &journal_bytes[..8],
        &[0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7]
    );
    fault
        .execute_batch("ROLLBACK;")
        .expect("TEST_CODE rollback hot transaction");
    drop(fault);
    fs::write(&journal, &journal_bytes).expect("TEST_CODE restore hot journal");
    let before = directory_bytes(root.path());
    let result = inspect_raw_activation_facts(&database, &unit(UNIT));
    assert_eq!(result, Err(ActivationInspectError::SchemaRejected));
    assert_eq!(directory_bytes(root.path()), before);
}

#[test]
fn w16_errors_and_debug_are_redacted_and_unknown_selection_is_not_filled() {
    let (_root, database) = seeded_database("redacted.sqlite3");
    let unknown = unit("not-a-catalog-unit");
    let error = inspect_raw_activation_facts(&database, &unknown).expect_err("TEST_CODE unknown");
    assert_eq!(error, ActivationInspectError::UnknownRequestedUnit);
    let display = error.to_string();
    let debug = format!("{error:?}");
    let database_text = database.to_string_lossy();
    for secret in [
        database_text.as_ref(),
        "operator-a",
        "owner-legacy",
        MANIFEST_SHA,
    ] {
        assert!(!display.contains(secret));
        assert!(!debug.contains(secret));
    }
}
