use super::*;
use rusqlite::types::Value;
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::time::Duration;

const V11_SHA256: &str = "8ee02c8ab5bb7e23ee7904f4db08ccc86b7f504a7ae86b37fc88c75d4a453faa";
const V12_SHA256: &str = "2a1a6969708b8df9feacb20b4474039e3b04d3c95ae7f849926defb1377021af";

const V11_MACRO_FACTS: [(&str, i64); 8] = [
    ("chain_post_close_macro_plans", 1),
    ("chain_post_close_macro_request_plans", 1),
    ("chain_post_close_macro_readiness_episode_plans", 1),
    ("chain_post_close_macro_control_attempt_begins", 2),
    ("chain_post_close_macro_control_attempt_results", 2),
    ("chain_post_close_macro_attempt_begins", 1),
    ("chain_post_close_macro_attempt_results", 1),
    ("chain_post_close_macro_source_finals", 1),
];

const REPLACED_GUARDS: [&str; 8] = [
    "chain_post_close_macro_plans_guard",
    "chain_post_close_macro_request_plans_guard",
    "chain_post_close_macro_readiness_episode_plans_guard",
    "chain_post_close_macro_control_attempt_begins_guard",
    "chain_post_close_macro_control_attempt_results_guard",
    "chain_post_close_macro_attempt_begins_guard",
    "chain_post_close_macro_attempt_results_guard",
    "chain_post_close_macro_source_finals_guard",
];

const V12_TABLES: [&str; 4] = [
    "chain_post_close_macro_query_terminals",
    "chain_post_close_macro_dimension_terminals",
    "chain_post_close_macro_finalize_begins",
    "chain_post_close_macro_stage_finals",
];

const V12_OBJECTS: [(&str, &str, &str); 16] = [
    ("chain_post_close_macro_query_terminals", "table", "chain_post_close_macro_query_terminals"),
    ("chain_post_close_macro_query_terminals_guard", "trigger", "chain_post_close_macro_query_terminals"),
    ("chain_post_close_macro_query_terminals_update", "trigger", "chain_post_close_macro_query_terminals"),
    ("chain_post_close_macro_query_terminals_delete", "trigger", "chain_post_close_macro_query_terminals"),
    ("chain_post_close_macro_dimension_terminals", "table", "chain_post_close_macro_dimension_terminals"),
    ("chain_post_close_macro_dimension_terminals_guard", "trigger", "chain_post_close_macro_dimension_terminals"),
    ("chain_post_close_macro_dimension_terminals_update", "trigger", "chain_post_close_macro_dimension_terminals"),
    ("chain_post_close_macro_dimension_terminals_delete", "trigger", "chain_post_close_macro_dimension_terminals"),
    ("chain_post_close_macro_finalize_begins", "table", "chain_post_close_macro_finalize_begins"),
    ("chain_post_close_macro_finalize_begins_guard", "trigger", "chain_post_close_macro_finalize_begins"),
    ("chain_post_close_macro_finalize_begins_update", "trigger", "chain_post_close_macro_finalize_begins"),
    ("chain_post_close_macro_finalize_begins_delete", "trigger", "chain_post_close_macro_finalize_begins"),
    ("chain_post_close_macro_stage_finals", "table", "chain_post_close_macro_stage_finals"),
    ("chain_post_close_macro_stage_finals_guard", "trigger", "chain_post_close_macro_stage_finals"),
    ("chain_post_close_macro_stage_finals_update", "trigger", "chain_post_close_macro_stage_finals"),
    ("chain_post_close_macro_stage_finals_delete", "trigger", "chain_post_close_macro_stage_finals"),
];

const V12_AUTOINDEXES: [(&str, &str); 10] = [
    ("sqlite_autoindex_chain_post_close_macro_query_terminals_1", "chain_post_close_macro_query_terminals"),
    ("sqlite_autoindex_chain_post_close_macro_query_terminals_2", "chain_post_close_macro_query_terminals"),
    ("sqlite_autoindex_chain_post_close_macro_query_terminals_3", "chain_post_close_macro_query_terminals"),
    ("sqlite_autoindex_chain_post_close_macro_dimension_terminals_1", "chain_post_close_macro_dimension_terminals"),
    ("sqlite_autoindex_chain_post_close_macro_dimension_terminals_2", "chain_post_close_macro_dimension_terminals"),
    ("sqlite_autoindex_chain_post_close_macro_finalize_begins_1", "chain_post_close_macro_finalize_begins"),
    ("sqlite_autoindex_chain_post_close_macro_finalize_begins_2", "chain_post_close_macro_finalize_begins"),
    ("sqlite_autoindex_chain_post_close_macro_finalize_begins_3", "chain_post_close_macro_finalize_begins"),
    ("sqlite_autoindex_chain_post_close_macro_stage_finals_1", "chain_post_close_macro_stage_finals"),
    ("sqlite_autoindex_chain_post_close_macro_stage_finals_2", "chain_post_close_macro_stage_finals"),
];

fn text(row: &[Value], column: usize) -> &str {
    match &row[column] {
        Value::Text(value) => value,
        other => panic!("TEST_CODE catalog text column {column}: {other:?}"),
    }
}

fn definition(row: &[Value]) -> &[u8] {
    match &row[4] {
        Value::Blob(value) => value,
        other => panic!("TEST_CODE catalog definition: {other:?}"),
    }
}

fn catalog_row<'a>(
    state: &'a v11_migration_tests::DatabaseState,
    name: &str,
) -> &'a [Value] {
    state
        .catalog
        .iter()
        .find(|row| text(row, 1) == name)
        .unwrap_or_else(|| panic!("TEST_CODE missing catalog object {name}"))
}

fn reference_v12_definitions(
    before: &v11_migration_tests::DatabaseState,
) -> BTreeMap<String, Vec<u8>> {
    let reference = Connection::open_in_memory().expect("TEST_CODE v12 reference connection");
    for kind in ["table", "index", "view", "trigger"] {
        for row in &before.catalog {
            let name = text(row, 1);
            if text(row, 0) != kind || name.starts_with("sqlite_") || row[4] == Value::Null {
                continue;
            }
            let sql = std::str::from_utf8(definition(row))
                .expect("TEST_CODE v11 catalog definition UTF-8");
            reference
                .execute_batch(sql)
                .unwrap_or_else(|error| panic!("TEST_CODE rebuild v11 {name}: {error}"));
        }
    }
    reference
        .execute_batch(include_str!("chain_post_close.v12.sql"))
        .expect("TEST_CODE apply frozen v12 DDL to independent reference");
    REPLACED_GUARDS
        .iter()
        .copied()
        .chain(V12_OBJECTS.iter().map(|(name, _, _)| *name))
        .map(|name| {
            let bytes = reference
                .query_row(
                    "SELECT CAST(sql AS BLOB) FROM sqlite_schema WHERE name=?1",
                    [name],
                    |row| row.get(0),
                )
                .unwrap_or_else(|error| panic!("TEST_CODE reference definition {name}: {error}"));
            (name.to_owned(), bytes)
        })
        .collect()
}

fn assert_real_v11_macro_facts(connection: &Connection) {
    for (table, expected) in V11_MACRO_FACTS {
        assert_eq!(
            connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get::<_, i64>(0))
                .unwrap(),
            expected,
            "TEST_CODE real v11 fact count: {table}"
        );
    }
}

fn assert_old_rows_preserved(
    before: &v11_migration_tests::DatabaseState,
    after: &v11_migration_tests::DatabaseState,
) {
    assert_eq!(after.application_id, before.application_id);
    assert_eq!(after.user_version, before.user_version);
    assert_eq!(after.query_only, before.query_only);
    assert_eq!(after.autocommit, before.autocommit);
    for (name, old_rows) in &before.tables {
        let current = after.tables.get(name).unwrap();
        match name.as_str() {
            "chain_post_close_layouts" | "chain_post_close_layout_objects" => {
                let old = current
                    .iter()
                    .filter(|row| matches!(row.first(), Some(Value::Integer(version)) if *version <= 11))
                    .cloned()
                    .collect::<Vec<_>>();
                assert_eq!(&old, old_rows, "TEST_CODE v1-v11 metadata changed: {name}");
            }
            _ => assert_eq!(current, old_rows, "TEST_CODE old table changed: {name}"),
        }
    }
    assert_eq!(
        after.tables["data_acquisition_audit"],
        before.tables["data_acquisition_audit"]
    );
    assert_eq!(
        after.tables["data_acquisition_audit_chain"],
        before.tables["data_acquisition_audit_chain"]
    );
    assert_eq!(after.tables.len(), before.tables.len() + V12_TABLES.len());
    for table in V12_TABLES {
        assert!(after.tables[table].is_empty(), "TEST_CODE v12 table is not empty: {table}");
    }
}

fn assert_exact_v12_catalog_delta(
    before: &v11_migration_tests::DatabaseState,
    after: &v11_migration_tests::DatabaseState,
    expected_definitions: &BTreeMap<String, Vec<u8>>,
) {
    for old in &before.catalog {
        let name = text(old, 1);
        let current = catalog_row(after, name);
        if REPLACED_GUARDS.contains(&name) {
            assert_eq!(&current[..4], &old[..4], "TEST_CODE guard identity changed: {name}");
            assert_ne!(definition(current), definition(old), "TEST_CODE guard was not replaced: {name}");
            assert_eq!(definition(current), expected_definitions[name].as_slice());
        } else {
            assert_eq!(current, old, "TEST_CODE old catalog row changed: {name}");
        }
    }

    let mut defined_additions = after
        .catalog
        .iter()
        .filter(|row| {
            row[4] != Value::Null
                && !before.catalog.iter().any(|old| old[1] == row[1])
        })
        .map(|row| text(row, 1).to_owned())
        .collect::<Vec<_>>();
    defined_additions.sort();
    let mut expected_names = V12_OBJECTS
        .iter()
        .map(|(name, _, _)| (*name).to_owned())
        .collect::<Vec<_>>();
    expected_names.sort();
    assert_eq!(defined_additions, expected_names);
    for (name, kind, table) in V12_OBJECTS {
        let row = catalog_row(after, name);
        assert_eq!(text(row, 0), kind);
        assert_eq!(text(row, 2), table);
        assert_eq!(definition(row), expected_definitions[name].as_slice());
    }

    let mut generated = after
        .catalog
        .iter()
        .filter(|row| {
            row[4] == Value::Null
                && !before.catalog.iter().any(|old| old[1] == row[1])
        })
        .map(|row| {
            assert_eq!(text(row, 0), "index");
            (text(row, 1).to_owned(), text(row, 2).to_owned())
        })
        .collect::<Vec<_>>();
    generated.sort();
    let mut expected_generated = V12_AUTOINDEXES
        .iter()
        .map(|(name, table)| ((*name).to_owned(), (*table).to_owned()))
        .collect::<Vec<_>>();
    expected_generated.sort();
    assert_eq!(generated, expected_generated);
    assert_eq!(
        after.catalog.len(),
        before.catalog.len() + V12_OBJECTS.len() + V12_AUTOINDEXES.len()
    );
}

fn assert_v12_metadata(connection: &Connection) {
    let header: (i64, i64, String, i64, i64, i64, String, String) = connection
        .query_row(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256,\
             artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
             FROM chain_post_close_layouts WHERE layout_version=12",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)),
        )
        .unwrap();
    assert_eq!(
        header,
        (12, 11, V11_SHA256.to_owned(), 1, 1, 1, "chain-post-close-layout-v12".to_owned(), V12_SHA256.to_owned())
    );
    assert_eq!(
        connection.query_row(
            "SELECT count(*) FROM chain_post_close_layouts WHERE layout_version>=12",
            [],
            |row| row.get::<_, i64>(0),
        ).unwrap(),
        1
    );
    assert_eq!(
        connection.query_row(
            "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version>=12",
            [],
            |row| row.get::<_, i64>(0),
        ).unwrap(),
        241
    );
    assert_eq!(
        connection.query_row(
            "SELECT count(*) FROM chain_post_close_layout_objects registry \
             JOIN sqlite_schema catalog \
               ON catalog.name=registry.name AND catalog.type=registry.object_type \
              AND CAST(catalog.sql AS BLOB)=CAST(registry.definition AS BLOB) \
             WHERE registry.layout_version=12",
            [],
            |row| row.get::<_, i64>(0),
        ).unwrap(),
        241
    );
}

fn assert_real_macro_semantics_unchanged(
    before: &crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery,
    after: &crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery,
) {
    assert_eq!(after.is_complete(), before.is_complete());
    assert!(!after.has_unconfirmed_effect());
    assert_eq!(after.plan_bytes(), before.plan_bytes());
    assert_eq!(after.plan_version(), before.plan_version());
    assert_eq!(after.parent_final_bytes(), before.parent_final_bytes());
    let (before_plan, after_plan) = (before.plan(), after.plan());
    assert_eq!(after_plan.profile(), before_plan.profile());
    assert_eq!(after_plan.acquisition_authority(), before_plan.acquisition_authority());
    assert_eq!(after_plan.endpoint(), before_plan.endpoint());
    assert_eq!(after_plan.started_at(), before_plan.started_at());
    assert_eq!(after_plan.deadline_at(), before_plan.deadline_at());
    assert_eq!(after_plan.observed_local(), before_plan.observed_local());
    assert_eq!(after_plan.research_providers(), before_plan.research_providers());
    assert_eq!(after_plan.research_decision_provenance(), before_plan.research_decision_provenance());
    assert_eq!(after_plan.research_decisions().len(), before_plan.research_decisions().len());
    for (old, current) in before_plan.research_decisions().iter().zip(after_plan.research_decisions()) {
        assert_eq!(current.registration_ordinal(), old.registration_ordinal());
        assert_eq!(current.availability_source(), old.availability_source());
        assert_eq!(current.local_transport_endpoint(), old.local_transport_endpoint());
        assert_eq!(current.remote_health(), old.remote_health());
    }
    let (before_request, after_request) = (
        before_plan.first_source_request(),
        after_plan.first_source_request(),
    );
    assert_eq!(after_request.request_id(), before_request.request_id());
    assert_eq!(after_request.request_bytes(), before_request.request_bytes());
    assert_eq!(after_request.retry_policy(), before_request.retry_policy());

    assert_eq!(after.readiness_episodes().len(), before.readiness_episodes().len());
    for (old, current) in before.readiness_episodes().iter().zip(after.readiness_episodes()) {
        assert_eq!(current.episode_ordinal(), old.episode_ordinal());
        assert_eq!(current.initiating_source(), old.initiating_source());
        assert_eq!(current.ready_result_version(), old.ready_result_version());
        assert_eq!(current.controls().len(), old.controls().len());
        for (old_control, current_control) in old.controls().iter().zip(current.controls()) {
            assert_eq!(current_control.kind(), old_control.kind());
            assert_eq!(current_control.request_id(), old_control.request_id());
            assert_eq!(current_control.request_bytes(), old_control.request_bytes());
            assert_eq!(current_control.begin_version(), old_control.begin_version());
            assert_eq!(current_control.result_version(), old_control.result_version());
            assert_eq!(current_control.outcome(), old_control.outcome());
            assert_eq!(current_control.response_bytes(), old_control.response_bytes());
        }
    }

    assert_eq!(after.attempts().len(), before.attempts().len());
    for (old, current) in before.attempts().iter().zip(after.attempts()) {
        assert_eq!(current.query_key(), old.query_key());
        assert_eq!(current.attempt_ordinal(), old.attempt_ordinal());
        assert_eq!(current.request_id(), old.request_id());
        assert_eq!(current.request_bytes(), old.request_bytes());
        assert_eq!(current.readiness_result_version(), old.readiness_result_version());
        assert_eq!(current.begin_version(), old.begin_version());
        assert_eq!(current.result_version(), old.result_version());
        assert_eq!(current.response_bytes(), old.response_bytes());
        assert_eq!(current.continuation(), old.continuation());
    }

    let before_source = before.global_news(GlobalNewsProvider::Eastmoney).unwrap();
    let after_source = after.global_news(GlobalNewsProvider::Eastmoney).unwrap();
    assert!(after_source.is_complete());
    assert_eq!(after_source.profile(), before_source.profile());
    assert_eq!(after_source.acquisition_authority(), before_source.acquisition_authority());
    assert_eq!(after_source.retry_policy(), before_source.retry_policy());
    assert_eq!(after_source.final_bytes(), before_source.final_bytes());
    assert_eq!(after_source.audit_receipt(), before_source.audit_receipt());
    let (before_batch, after_batch) = (before_source.batch().unwrap(), after_source.batch().unwrap());
    assert_eq!(after_batch.evidence().provider, before_batch.evidence().provider);
    assert_eq!(after_batch.evidence().source, before_batch.evidence().source);
    assert_eq!(after_batch.evidence().source_at, before_batch.evidence().source_at);
    assert_eq!(after_batch.evidence().observed_at, before_batch.evidence().observed_at);
    assert_eq!(after_batch.evidence().batch_id, before_batch.evidence().batch_id);
    assert_eq!(after_batch.records(), before_batch.records());
    assert_eq!(after.pending_source_identities(), before.pending_source_identities());
    assert_eq!(after.pending_research_queries(), before.pending_research_queries());
}

fn assert_reopened_real_macro_semantics(
    business: &mut V2BusinessFixture,
    config: &crate::monitor::push_job::LocalChainPostCloseConfig,
    intent: &crate::monitor::push_job::IntentId,
    before_recovery: &crate::push_foundation::intent_store::chain_post_close::macro_stage::MacroRecovery,
    context: &[u8],
    head: u64,
    generation: u64,
) {
    let mut local = business
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(config)
        .unwrap();
    let recovered = local.inspect_macro(intent).unwrap();
    let run = local.inspect_run(intent).unwrap();
    assert_eq!(run.context().canonical_bytes().as_slice(), context);
    assert_eq!(run.head_version(), head);
    assert_eq!(run.lease_generation(), generation);
    assert_real_macro_semantics_unchanged(before_recovery, &recovered);
    drop(local);
}

#[derive(Clone, Copy)]
enum MacroDigestDamage {
    Context,
    Input,
    Payload,
}

impl MacroDigestDamage {
    fn run_id(self) -> &'static str {
        match self {
            Self::Context => "TEST_CODE_V12_MACRO_CONTEXT_DIGEST_DAMAGE",
            Self::Input => "TEST_CODE_V12_MACRO_INPUT_DIGEST_DAMAGE",
            Self::Payload => "TEST_CODE_V12_MACRO_PAYLOAD_DIGEST_DAMAGE",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Context => "v12 Macro context-digest damage",
            Self::Input => "v12 Macro input-digest damage",
            Self::Payload => "v12 Macro payload-digest damage",
        }
    }

    fn column(self) -> usize {
        match self {
            Self::Context => 2,
            Self::Input => 3,
            Self::Payload => 11,
        }
    }

    fn replacement(self) -> &'static str {
        match self {
            Self::Context => {
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            }
            Self::Input => {
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            }
            Self::Payload => {
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            }
        }
    }

    fn update(self) -> &'static str {
        match self {
            Self::Context => {
                "UPDATE chain_post_close_macro_source_finals \
                 SET run_context_sha256=?1 WHERE intent_id=?2"
            }
            Self::Input => {
                "UPDATE chain_post_close_macro_source_finals \
                 SET input_sha256=?1 WHERE intent_id=?2"
            }
            Self::Payload => {
                "UPDATE chain_post_close_macro_source_finals \
                 SET sha256=?1 WHERE intent_id=?2"
            }
        }
    }
}

fn assert_only_source_final_digest_changed(
    clean: &v11_migration_tests::DatabaseState,
    damaged: &v11_migration_tests::DatabaseState,
    intent: &crate::monitor::push_job::IntentId,
    damage: MacroDigestDamage,
) {
    const TABLE: &str = "chain_post_close_macro_source_finals";
    assert_eq!(damaged.application_id, clean.application_id);
    assert_eq!(damaged.user_version, clean.user_version);
    assert_eq!(damaged.query_only, clean.query_only);
    assert_eq!(damaged.autocommit, clean.autocommit);
    assert_eq!(damaged.catalog, clean.catalog);
    assert_eq!(damaged.tables.len(), clean.tables.len());
    for (name, rows) in &clean.tables {
        if name != TABLE {
            assert_eq!(
                &damaged.tables[name], rows,
                "TEST_CODE digest damage changed unrelated table {name}"
            );
        }
    }
    let mut expected = clean.tables[TABLE].clone();
    let targets = expected
        .iter_mut()
        .filter(|row| row[0] == Value::Text(intent.as_str().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(targets.len(), 1);
    let target = targets.into_iter().next().unwrap();
    assert_ne!(
        target[damage.column()],
        Value::Text(damage.replacement().to_owned())
    );
    target[damage.column()] = Value::Text(damage.replacement().to_owned());
    assert_eq!(damaged.tables[TABLE], expected);
}

async fn run_real_v12_macro_digest_damage(damage: MacroDigestDamage) {
    const TRIGGER: &str = "chain_post_close_macro_source_finals_update";
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let control_tests::ConfirmedExternalBaseline { config, intent, .. } =
                control_tests::establish_confirmed_external_first_source(
                    &mut business,
                    &mut parent_server,
                    &mut external_server,
                    damage.run_id(),
                )
                .await;
            let external_network = external_server.as_ref().unwrap().snapshot();
            let parent_network = parent_server.as_ref().unwrap().snapshot();
            let memberships = parent_server.as_ref().unwrap().membership_snapshot();
            macro_rules! assert_no_rpc {
                () => {
                    assert_eq!(
                        external_server.as_ref().unwrap().snapshot(),
                        external_network
                    );
                    assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_network);
                    assert_eq!(
                        parent_server.as_ref().unwrap().membership_snapshot(),
                        memberships
                    );
                };
            }

            business.reopen();
            let v11 = business.chain_post_close().verify_schema().unwrap();
            assert_eq!(v11.schema_version(), 11);
            assert_eq!(v11.ddl_sha256().as_str(), V11_SHA256);
            assert_real_v11_macro_facts(business.connection());
            let before_v12 = v11_migration_tests::DatabaseState::capture(business.connection());
            let migrated = business
                .chain_post_close()
                .migrate_schema_v11_to_v12()
                .unwrap();
            assert_eq!(migrated.schema_version(), 12);
            assert_eq!(migrated.ddl_sha256().as_str(), V12_SHA256);
            assert_eq!(business.chain_post_close().verify_schema().unwrap(), migrated);
            let clean = v11_migration_tests::DatabaseState::capture(business.connection());
            assert_old_rows_preserved(&before_v12, &clean);
            assert_v12_metadata(business.connection());
            assert_no_rpc!();

            let database = business.database();
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let first = local.inspect_macro(&intent).unwrap();
            let second = local.inspect_macro(&intent).unwrap();
            assert_real_macro_semantics_unchanged(&first, &second);
            assert_no_rpc!();

            let injector = Connection::open(&database).unwrap();
            injector.busy_timeout(Duration::from_millis(250)).unwrap();
            assert_eq!(
                v11_migration_tests::DatabaseState::capture(&injector),
                clean
            );
            let trigger: String = injector
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
                    [TRIGGER],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                trigger,
                "CREATE TRIGGER chain_post_close_macro_source_finals_update BEFORE UPDATE ON \
                 chain_post_close_macro_source_finals BEGIN SELECT RAISE(ABORT,'chain v11 macro immutable'); END"
            );
            let transaction = injector.unchecked_transaction().unwrap();
            transaction
                .execute_batch("DROP TRIGGER chain_post_close_macro_source_finals_update")
                .unwrap();
            assert_eq!(
                transaction
                    .execute(damage.update(), [damage.replacement(), intent.as_str()])
                    .unwrap(),
                1
            );
            transaction.execute_batch(&trigger).unwrap();
            transaction.commit().unwrap();
            assert!(injector.is_autocommit());
            let damaged = v11_migration_tests::DatabaseState::capture(&injector);
            assert_only_source_final_digest_changed(&clean, &damaged, &intent, damage);
            assert_no_rpc!();

            assert!(matches!(
                local.inspect_macro(&intent),
                Err(ChainPostCloseError::SchemaRejected)
            ));
            assert_eq!(
                v11_migration_tests::DatabaseState::capture(&injector),
                damaged
            );
            assert_no_rpc!();
            drop(local);
            assert!(business.connection().is_autocommit());
            assert_eq!(
                v11_migration_tests::DatabaseState::capture(business.connection()),
                damaged
            );
            assert_no_rpc!();

            injector.close().unwrap();
            business.reopen();
            assert!(matches!(
                business
                    .store
                    .as_mut()
                    .unwrap()
                    .single_user_local_chain_post_close(&config),
                Err(ChainPostCloseError::SchemaRejected)
            ));
            assert!(business.connection().is_autocommit());
            assert_eq!(
                v11_migration_tests::DatabaseState::capture(business.connection()),
                damaged
            );
            assert_no_rpc!();
        },
    ))
    .catch_unwind()
    .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        damage.label(),
    )
    .await;
    drop(business);
    match body {
        Ok(Ok(())) => {}
        Ok(Err(_)) => panic!("TEST_CODE {} body deadline", damage.label()),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum V12MigrationScenario {
    Preservation,
    CommitContention,
}

impl V12MigrationScenario {
    fn run_id(self) -> &'static str {
        match self {
            Self::Preservation => "TEST_CODE_V12_MIGRATION_PRESERVES_REAL_MACRO_FACTS",
            Self::CommitContention => "TEST_CODE_V12_MIGRATION_REAL_COMMIT_CONTENTION",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Preservation => "v12 migration preservation",
            Self::CommitContention => "v12 migration commit contention",
        }
    }
}

async fn run_v11_to_v12_migration_scenario(scenario: V12MigrationScenario) {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let mut lock_reader = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let control_tests::ConfirmedExternalBaseline { config, intent, .. } =
                control_tests::establish_confirmed_external_first_source(
                    &mut business,
                    &mut parent_server,
                    &mut external_server,
                    scenario.run_id(),
                )
                .await;
            let external_network = external_server.as_ref().unwrap().snapshot();
            let parent_network = parent_server.as_ref().unwrap().snapshot();
            let memberships = parent_server.as_ref().unwrap().membership_snapshot();

            business.reopen();
            let v11 = business.chain_post_close().verify_schema().unwrap();
            assert_eq!(v11.schema_version(), 11);
            assert_eq!(v11.ddl_sha256().as_str(), V11_SHA256);
            assert_real_v11_macro_facts(business.connection());
            let (before_recovery, context, head, generation) = {
                let mut local = business
                    .store
                    .as_mut()
                    .unwrap()
                    .single_user_local_chain_post_close(&config)
                    .unwrap();
                let recovery = local.inspect_macro(&intent).unwrap();
                assert!(!recovery.is_complete());
                assert!(!recovery.has_unconfirmed_effect());
                assert!(recovery.global_news(GlobalNewsProvider::Eastmoney).unwrap().is_complete());
                let run = local.inspect_run(&intent).unwrap();
                let snapshot = (
                    run.context().canonical_bytes(),
                    run.head_version(),
                    run.lease_generation(),
                );
                drop(local);
                (recovery, snapshot.0, snapshot.1, snapshot.2)
            };
            if scenario == V12MigrationScenario::CommitContention {
                let mode: String = business
                    .connection()
                    .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(mode.to_ascii_lowercase(), "delete");
            }
            let before = v11_migration_tests::DatabaseState::capture(business.connection());
            assert_eq!(before.query_only, 0);
            assert!(before.autocommit);
            let expected_definitions = reference_v12_definitions(&before);

            let migrated = if scenario == V12MigrationScenario::CommitContention {
                let reader = BusinessIntentStore::open(&business.database()).unwrap();
                reader.connection.execute_batch("BEGIN DEFERRED;").unwrap();
                assert_eq!(
                    reader
                        .connection
                        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
                            row.get::<_, i64>(0)
                        })
                        .unwrap(),
                    i64::try_from(before.catalog.len()).unwrap()
                );
                assert_eq!(
                    reader.connection.query_row(
                        "SELECT count(*) FROM chain_post_close_macro_source_finals",
                        [],
                        |row| row.get::<_, i64>(0),
                    ).unwrap(),
                    1
                );
                let reader_before =
                    v11_migration_tests::DatabaseState::capture(&reader.connection);
                lock_reader = Some(reader);

                assert_eq!(
                    business
                        .chain_post_close()
                        .migrate_schema_v11_to_v12(),
                    Err(ChainPostCloseError::StorageFailed {
                        operation: "v12 commit"
                    })
                );
                assert!(business.connection().is_autocommit());
                assert_eq!(
                    business
                        .connection()
                        .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
                        .unwrap(),
                    0
                );
                assert_eq!(
                    business
                        .connection()
                        .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
                        .unwrap(),
                    250
                );
                assert_eq!(
                    v11_migration_tests::DatabaseState::capture(business.connection()),
                    before
                );
                assert_eq!(
                    v11_migration_tests::DatabaseState::capture(
                        &lock_reader.as_ref().unwrap().connection
                    ),
                    reader_before
                );

                let reader = lock_reader.take().unwrap();
                if !reader.connection.is_autocommit() {
                    reader.connection.execute_batch("ROLLBACK;").unwrap();
                }
                reader.connection.close().unwrap();
                business.reopen();
                let fresh_v11 = business.chain_post_close().verify_schema().unwrap();
                assert_eq!(fresh_v11.schema_version(), 11);
                assert_eq!(fresh_v11.ddl_sha256().as_str(), V11_SHA256);
                assert_eq!(
                    v11_migration_tests::DatabaseState::capture(business.connection()),
                    before
                );
                assert_real_v11_macro_facts(business.connection());
                assert_reopened_real_macro_semantics(
                    &mut business,
                    &config,
                    &intent,
                    &before_recovery,
                    &context,
                    head,
                    generation,
                );
                assert_eq!(external_server.as_ref().unwrap().snapshot(), external_network);
                assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_network);
                assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), memberships);

                business
                    .chain_post_close()
                    .migrate_schema_v11_to_v12()
                    .unwrap()
            } else {
                business
                    .chain_post_close()
                    .migrate_schema_v11_to_v12()
                    .unwrap()
            };
            assert_eq!(migrated.schema_version(), 12);
            assert_eq!(migrated.ddl_sha256().as_str(), V12_SHA256);
            let verified = business.chain_post_close().verify_schema().unwrap();
            assert_eq!(verified, migrated);
            let after = v11_migration_tests::DatabaseState::capture(business.connection());
            assert_old_rows_preserved(&before, &after);
            assert_exact_v12_catalog_delta(&before, &after, &expected_definitions);
            assert_v12_metadata(business.connection());
            assert_eq!(external_server.as_ref().unwrap().snapshot(), external_network);
            assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_network);
            assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), memberships);

            business.reopen();
            let reopened = business.chain_post_close().verify_schema().unwrap();
            assert_eq!(reopened.schema_version(), 12);
            assert_eq!(reopened.ddl_sha256().as_str(), V12_SHA256);
            assert_eq!(v11_migration_tests::DatabaseState::capture(business.connection()), after);
            assert_reopened_real_macro_semantics(
                &mut business,
                &config,
                &intent,
                &before_recovery,
                &context,
                head,
                generation,
            );
            assert_eq!(external_server.as_ref().unwrap().snapshot(), external_network);
            assert_eq!(parent_server.as_ref().unwrap().snapshot(), parent_network);
            assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), memberships);
        },
    ))
    .catch_unwind()
    .await;

    let reader_cleanup = lock_reader.take().map(|reader| {
        let rollback = if reader.connection.is_autocommit() {
            Ok(())
        } else {
            reader.connection.execute_batch("ROLLBACK;")
        };
        let close = reader.connection.close().map_err(|(connection, error)| {
            drop(connection);
            error
        });
        rollback.and(close)
    });
    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        scenario.label(),
    )
    .await;
    drop(business);
    if let Some(result) = reader_cleanup {
        result.unwrap_or_else(|error| panic!("TEST_CODE {} reader cleanup: {error}", scenario.label()));
    }
    match body {
        Ok(Ok(())) => {}
        Ok(Err(_)) => panic!("TEST_CODE {} body deadline", scenario.label()),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
async fn v11_to_v12_preserves_real_macro_facts_and_reopens_with_exact_guard_replacements() {
    run_v11_to_v12_migration_scenario(V12MigrationScenario::Preservation).await;
}

#[tokio::test]
async fn v11_to_v12_real_commit_contention_rolls_back_and_reopens_for_retry() {
    run_v11_to_v12_migration_scenario(V12MigrationScenario::CommitContention).await;
}

#[tokio::test]
async fn real_v12_reader_rejects_later_fact_context_digest_corruption_without_repair_or_rpc() {
    run_real_v12_macro_digest_damage(MacroDigestDamage::Context).await;
}

#[tokio::test]
async fn real_v12_reader_rejects_later_fact_input_digest_corruption_without_repair_or_rpc() {
    run_real_v12_macro_digest_damage(MacroDigestDamage::Input).await;
}

#[tokio::test]
async fn real_v12_reader_rejects_per_fact_payload_digest_corruption_without_repair_or_rpc() {
    run_real_v12_macro_digest_damage(MacroDigestDamage::Payload).await;
}

#[tokio::test]
async fn v11_external_raw_v2_migrates_then_appends_v12_native_data_and_reopens_without_rpc() {
    use crate::data_gateway::grpc_source::macro_queries::PreparedMacroQueries;
    use crate::database::data_acquisition_audit::read_acquisition_in_transaction;
    use crate::grpc_client::client::macro_attempt::{
        ExternalMacroAttemptCompletion, MacroQueryIdentity,
    };
    use crate::push_foundation::intent_store::chain_post_close::macro_live::Live;
    use crate::search_service::macro_news::{runner::QueryKey, NativeOutcome};
    use std::rc::Rc;

    const LABEL: &str = "v11 External RawResult V2 plus v12 native DataResult";
    const RUN_ID: &str = "TEST_CODE_V12_MIXED_EXTERNAL_HISTORY";
    const OWNER: &str = "TEST_CODE_V12_MIXED_EXTERNAL_OWNER";
    let started_at = micros("2026-09-14T15:31:00+08:00");

    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let mut external_server = None;
    let body = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(120),
        async {
            let control_tests::ConfirmedExternalBaseline { config, intent, .. } =
                control_tests::establish_confirmed_external_first_source(
                    &mut business,
                    &mut parent_server,
                    &mut external_server,
                    RUN_ID,
                )
                .await;
            let external_before_migration = external_server.as_ref().unwrap().snapshot();
            let parent_before_migration = parent_server
                .as_ref()
                .unwrap()
                .snapshot_with_tcp_for_test();
            let memberships_before_migration =
                parent_server.as_ref().unwrap().membership_snapshot();
            let audit_before = business.count("data_acquisition_audit");

            let old_begin: (u64, Vec<u8>, i64, String) = business
                .connection()
                .query_row(
                    "SELECT run_version,bytes,byte_length,sha256 \
                     FROM chain_post_close_macro_attempt_begins WHERE intent_id=?1",
                    [intent.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
            let old_result: (u64, u64, Vec<u8>, i64, String) = business
                .connection()
                .query_row(
                    "SELECT run_version,begin_version,bytes,byte_length,sha256 \
                     FROM chain_post_close_macro_attempt_results WHERE intent_id=?1",
                    [intent.as_str()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(old_result.1, old_begin.0);
            assert_eq!(old_begin.2, i64::try_from(old_begin.1.len()).unwrap());
            assert_eq!(old_result.3, i64::try_from(old_result.2.len()).unwrap());
            assert_eq!(old_begin.3, raw_digest(&old_begin.1).as_str());
            assert_eq!(old_result.4, raw_digest(&old_result.2).as_str());
            let old_begin_json: serde_json::Value =
                serde_json::from_slice(&old_begin.1).unwrap();
            let old_result_json: serde_json::Value =
                serde_json::from_slice(&old_result.2).unwrap();
            assert_eq!(old_begin_json.get("version").and_then(|v| v.as_u64()), Some(1));
            assert_eq!(old_result_json.get("version").and_then(|v| v.as_u64()), Some(2));
            assert!(old_result_json.get("external_wire").is_some());
            for native_only in ["query", "raw", "native", "native_sha256"] {
                assert!(
                    old_result_json.get(native_only).is_none(),
                    "TEST_CODE legacy RawResult V2 grew native field {native_only}"
                );
            }

            let (
                old_plan,
                old_parent,
                old_attempt,
                old_source,
                old_context,
                old_head,
                old_generation,
            ) = {
                let mut local = business
                    .store
                    .as_mut()
                    .unwrap()
                    .single_user_local_chain_post_close(&config)
                    .unwrap();
                let recovery = local.inspect_macro(&intent).unwrap();
                assert_eq!(recovery.attempts().len(), 1);
                let attempt = &recovery.attempts()[0];
                assert_eq!(attempt.query_key(), QueryKey::Gateway(1));
                assert_eq!(attempt.begin_version(), old_begin.0);
                assert_eq!(attempt.result_version(), Some(old_result.0));
                let source = recovery
                    .global_news(GlobalNewsProvider::Eastmoney)
                    .unwrap();
                let run = local.inspect_run(&intent).unwrap();
                (
                    recovery.plan_bytes().to_vec(),
                    recovery.parent_final_bytes().to_vec(),
                    (
                        attempt.request_id().to_owned(),
                        attempt.request_bytes().to_vec(),
                        attempt.readiness_result_version(),
                        attempt.response_bytes().unwrap().to_vec(),
                        attempt.continuation(),
                    ),
                    (
                        source.final_bytes().unwrap().to_vec(),
                        source.audit_receipt().unwrap().clone(),
                    ),
                    run.context().canonical_bytes().to_vec(),
                    run.head_version(),
                    run.lease_generation(),
                )
            };
            let before_migration =
                v11_migration_tests::DatabaseState::capture(business.connection());

            let migrated = business
                .chain_post_close()
                .migrate_schema_v11_to_v12()
                .unwrap();
            assert_eq!(migrated.schema_version(), 12);
            assert_eq!(migrated.ddl_sha256().as_str(), V12_SHA256);
            assert_eq!(business.chain_post_close().verify_schema().unwrap(), migrated);
            let after_migration =
                v11_migration_tests::DatabaseState::capture(business.connection());
            assert_old_rows_preserved(&before_migration, &after_migration);
            assert_eq!(business.count("data_acquisition_audit"), audit_before);
            assert_eq!(external_server.as_ref().unwrap().snapshot(), external_before_migration);
            assert_eq!(
                parent_server.as_ref().unwrap().snapshot_with_tcp_for_test(),
                parent_before_migration
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                memberships_before_migration
            );

            let source = GrpcSource::from_external_macro_bundle_for_test(
                external_server
                    .as_ref()
                    .unwrap()
                    .bundle_path()
                    .to_path_buf(),
            );
            let PreparedMacroQueries::External(prepared) =
                source.prepare_macro_queries().unwrap()
            else {
                panic!("TEST_CODE mixed history requires prepared External endpoint");
            };
            let identity = MacroQueryIdentity::GlobalNews {
                provider: GlobalNewsProvider::Cailianpress,
                limit: 20,
            };
            // This successor is a real generated call with a terminal evidence refusal, not a second-source success.
            let authorized = prepared.prepare_macro_query(identity.clone()).unwrap();
            let request =
                macro_codec::Request::capture_prepared_for(&identity, &authorized).unwrap();
            let request_bytes = request.bytes.clone();
            let request_sha = raw_digest(&request_bytes).as_str().to_owned();
            let endpoint = prepared.endpoint_uri().to_owned();
            let resumed_at = started_at + 10_000_001;
            let clock = MacroClock {
                now: Cell::new(UtcMicros::try_new(resumed_at).unwrap()),
                observation: DateTime::parse_from_rfc3339(
                    "2026-09-14T15:31:10.000001+08:00",
                )
                .unwrap(),
                observation_calls: Cell::new(0),
            };
            let mut local = business
                .store
                .as_mut()
                .unwrap()
                .single_user_local_chain_post_close(&config)
                .unwrap();
            let migrated_recovery = local.inspect_macro(&intent).unwrap();
            assert_eq!(migrated_recovery.plan_bytes(), old_plan);
            assert_eq!(migrated_recovery.parent_final_bytes(), old_parent);
            assert_eq!(migrated_recovery.plan().deadline_at().get(), started_at + 15_000_000);
            assert_eq!(migrated_recovery.attempts().len(), 1);
            assert_eq!(
                (
                    migrated_recovery.attempts()[0].request_id(),
                    migrated_recovery.attempts()[0].request_bytes(),
                    migrated_recovery.attempts()[0].readiness_result_version(),
                    migrated_recovery.attempts()[0].response_bytes().unwrap(),
                    migrated_recovery.attempts()[0].continuation(),
                ),
                (
                    old_attempt.0.as_str(),
                    old_attempt.1.as_slice(),
                    old_attempt.2,
                    old_attempt.3.as_slice(),
                    old_attempt.4,
                )
            );
            assert_eq!(local.inspect_run(&intent).unwrap().head_version(), old_head);
            assert_eq!(
                local.inspect_run(&intent).unwrap().lease_generation(),
                old_generation
            );
            let lease = local
                .resume_run(
                    &intent,
                    macro_lease(
                        OWNER,
                        resumed_at,
                        started_at + 20_000_000,
                        old_head,
                    ),
                )
                .unwrap();
            assert_eq!(lease.head_version(), old_head + 1);
            let cancelled = Rc::new(Cell::new(false));
            let mut live = Live::open(&mut local, lease, &clock, Rc::clone(&cancelled)).unwrap();
            assert_eq!(live.deadline(), started_at + 15_000_000);
            let ready_result = migrated_recovery.readiness_episodes()[0]
                .ready_result_version()
                .unwrap();
            drop(migrated_recovery);
            let ticket = live
                .begin_data(QueryKey::Gateway(2), 1, request, &endpoint)
                .unwrap();
            assert_eq!(external_server.as_ref().unwrap().snapshot(), external_before_migration);
            assert_eq!(
                parent_server.as_ref().unwrap().snapshot_with_tcp_for_test(),
                parent_before_migration
            );
            assert_eq!(
                parent_server.as_ref().unwrap().membership_snapshot(),
                memberships_before_migration
            );

            external_server.as_ref().unwrap().release_data();
            let completion = tokio::time::timeout(Duration::from_secs(3), authorized.execute())
                .await
                .expect("TEST_CODE mixed-history External unary deadline")
                .expect("TEST_CODE mixed-history External connection");
            let ExternalMacroAttemptCompletion::Unary(unary) = &completion else {
                panic!("TEST_CODE mixed history requires a generated unary response");
            };
            assert!(unary.processed.is_ok());
            assert!(unary.response_bytes.is_some());
            assert!(unary.external_wire.is_some());
            live.record_data(ticket, &completion).unwrap();
            let lease = live.into_lease();
            assert_eq!(lease.head_version(), old_head + 5);
            assert!(!cancelled.get());
            drop(lease);

            let mixed = local.inspect_macro(&intent).unwrap();
            assert!(!mixed.is_complete());
            assert!(!mixed.has_unconfirmed_effect());
            assert_eq!(mixed.plan_bytes(), old_plan);
            assert_eq!(mixed.parent_final_bytes(), old_parent);
            assert_eq!(mixed.plan().deadline_at().get(), started_at + 15_000_000);
            assert_eq!(mixed.attempts().len(), 2);
            let legacy = &mixed.attempts()[0];
            let native = &mixed.attempts()[1];
            assert_eq!(legacy.query_key(), QueryKey::Gateway(1));
            assert_eq!(legacy.begin_version(), old_begin.0);
            assert_eq!(legacy.result_version(), Some(old_result.0));
            assert_eq!(legacy.request_id(), old_attempt.0);
            assert_eq!(legacy.request_bytes(), old_attempt.1);
            assert_eq!(legacy.response_bytes().unwrap(), old_attempt.3);
            assert_eq!(legacy.continuation(), old_attempt.4);
            assert_eq!(native.query_key(), QueryKey::Gateway(2));
            assert_eq!(native.attempt_ordinal(), 1);
            assert_eq!(native.request_bytes(), request_bytes);
            assert_eq!(native.readiness_result_version(), Some(ready_result));
            assert_eq!(native.continuation(), Some(MacroContinuation::Terminal));
            let native_result_version = native.result_version().unwrap();
            let native_begin_version = native.begin_version();
            let native_response = native.response_bytes().unwrap().to_vec();
            let terminal = mixed.query_terminal(QueryKey::Gateway(2)).unwrap();
            assert!(terminal.was_called());
            assert_eq!(terminal.query_key(), QueryKey::Gateway(2));
            assert_eq!(terminal.request_version(), Some(native_begin_version - 1));
            assert_eq!(terminal.version(), native_result_version + 1);
            let NativeOutcome::News(Err(error)) = terminal.native() else {
                panic!("TEST_CODE Gateway(2) must retain exact provider evidence refusal");
            };
            assert_eq!(error.capability(), "GlobalNews");
            assert_eq!(error.provider(), Some(ProviderId::Cailianpress));
            assert_eq!(error.audit_outcome(), "partial");
            assert_eq!(error.reason_code(), "invalid_evidence");
            assert!(!error.retryable());
            assert_eq!(
                error.message(),
                "ExternalV1 GlobalNews selected provider differs from exact request"
            );
            let native_terminal_bytes = terminal.native_bytes().to_vec();
            let native_receipt = terminal.audit_receipt().unwrap().clone();
            let pending_sources = mixed.pending_source_identities().to_vec();
            let pending_research = mixed.pending_research_queries().to_vec();
            drop(mixed);
            let run_after_append = local.inspect_run(&intent).unwrap();
            assert_eq!(run_after_append.context().canonical_bytes(), old_context);
            assert_eq!(run_after_append.head_version(), old_head + 5);
            assert_eq!(run_after_append.lease_generation(), old_generation + 1);
            drop(local);

            let old_result_after: (u64, u64, Vec<u8>, i64, String) = business
                .connection()
                .query_row(
                    "SELECT run_version,begin_version,bytes,byte_length,sha256 \
                     FROM chain_post_close_macro_attempt_results \
                     WHERE intent_id=?1 AND run_version=?2",
                    rusqlite::params![intent.as_str(), old_result.0],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(old_result_after, old_result);
            let native_begin: (u64, u64, Vec<u8>, i64, String, String, Option<u64>, Option<u64>) =
                business.connection().query_row(
                    "SELECT run_version,request_plan_version,bytes,byte_length,sha256, \
                            request_sha256,readiness_result_version,previous_result_version \
                     FROM chain_post_close_macro_attempt_begins \
                     WHERE intent_id=?1 AND run_version=?2",
                    rusqlite::params![intent.as_str(), native_begin_version],
                    |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,
                              row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
                ).unwrap();
            assert_eq!(native_begin.0, native_begin_version);
            assert_eq!(native_begin.3, i64::try_from(native_begin.2.len()).unwrap());
            assert_eq!(native_begin.4, raw_digest(&native_begin.2).as_str());
            assert_eq!(native_begin.5, request_sha);
            assert_eq!(native_begin.6, Some(ready_result));
            assert_eq!(native_begin.7, None);
            let native_begin_json: serde_json::Value = serde_json::from_slice(&native_begin.2).unwrap();
            assert_eq!(native_begin_json["version"], serde_json::json!(2));
            assert_eq!(native_begin_json["query"], serde_json::json!({"Gateway": 2}));
            assert_eq!(native_begin_json["attempt"], serde_json::json!(1));
            let native_result: (u64, u64, Vec<u8>, i64, String, String, String, Option<i64>) =
                business.connection().query_row(
                    "SELECT run_version,begin_version,bytes,byte_length,sha256,request_sha256, \
                            continuation,retry_not_before \
                     FROM chain_post_close_macro_attempt_results \
                     WHERE intent_id=?1 AND run_version=?2",
                    rusqlite::params![intent.as_str(), native_result_version],
                    |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,
                              row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
                ).unwrap();
            assert_eq!(native_result.1, native_begin_version);
            assert_eq!(native_result.3, i64::try_from(native_result.2.len()).unwrap());
            assert_eq!(native_result.4, raw_digest(&native_result.2).as_str());
            assert_eq!(native_result.5, request_sha);
            assert_eq!(native_result.6, "Terminal");
            assert_eq!(native_result.7, None);
            let native_result_json: serde_json::Value = serde_json::from_slice(&native_result.2).unwrap();
            assert_eq!(native_result_json["version"], serde_json::json!(2));
            assert_eq!(native_result_json["query"], serde_json::json!({"Gateway": 2}));
            assert_eq!(native_result_json["attempt"], serde_json::json!(1));
            assert_eq!(native_result_json["raw"]["version"], serde_json::json!(2));
            assert!(native_result_json["raw"].get("external_wire").is_some());
            assert!(native_result_json.get("native").is_some());
            assert!(native_result_json.get("native_sha256").is_some());
            assert_eq!(native_response, external_server.as_ref().unwrap().snapshot().data_responses[1]);

            assert_eq!(business.count("data_acquisition_audit"), audit_before + 1);
            let transaction = business.connection().unchecked_transaction().unwrap();
            let verified = read_acquisition_in_transaction(&transaction, &native_receipt).unwrap();
            assert_eq!(verified.receipt(), &native_receipt);
            let audit = verified.record();
            assert_eq!(audit.capability, "GlobalNews-CLS");
            assert_eq!(audit.provider, "Cailianpress");
            assert_eq!(audit.source, "review-data-gateway");
            assert_eq!(audit.outcome, "partial");
            assert_eq!(audit.reason_code, "invalid_evidence");
            assert!(!audit.retryable);
            assert_eq!((audit.request_count, audit.accepted_count, audit.rejected_count), (1, 0, 1));
            transaction.commit().unwrap();
            let old_source_after = {
                let mut local = business.store.as_mut().unwrap()
                    .single_user_local_chain_post_close(&config).unwrap();
                let recovery = local.inspect_macro(&intent).unwrap();
                let source = recovery.global_news(GlobalNewsProvider::Eastmoney).unwrap();
                (source.final_bytes().unwrap().to_vec(), source.audit_receipt().unwrap().clone())
            };
            assert_eq!(old_source_after, old_source);

            let external_after_data = external_server.as_ref().unwrap().snapshot();
            assert_eq!(external_after_data.tcp_accepts, external_before_migration.tcp_accepts + 1);
            assert_eq!(external_after_data.health_requests, external_before_migration.health_requests);
            assert_eq!(external_after_data.capabilities_calls, external_before_migration.capabilities_calls);
            assert_eq!(external_after_data.data_calls, external_before_migration.data_calls + 1);
            assert_eq!(external_after_data.data_methods.last().map(String::as_str), Some("global_news"));
            assert_eq!(external_after_data.data_authorized.last(), Some(&true));
            assert_eq!(external_after_data.data_responses.len(), external_before_migration.data_responses.len() + 1);
            assert!(external_after_data.data_statuses.is_empty());
            assert_eq!(parent_server.as_ref().unwrap().snapshot_with_tcp_for_test(), parent_before_migration);
            assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), memberships_before_migration);
            let durable_before_reopen =
                v11_migration_tests::DatabaseState::capture(business.connection());

            drop(prepared);
            drop(source);
            external_server.as_ref().unwrap().set_reject_new_connections_for_test(true);
            business.reopen();
            assert_eq!(business.chain_post_close().verify_schema().unwrap(), migrated);
            let mut local = business.store.as_mut().unwrap()
                .single_user_local_chain_post_close(&config).unwrap();
            let reopened = local.inspect_macro(&intent).unwrap();
            assert_eq!(reopened.plan_bytes(), old_plan);
            assert_eq!(reopened.parent_final_bytes(), old_parent);
            assert_eq!(reopened.attempts().len(), 2);
            assert_eq!(reopened.attempts()[0].begin_version(), old_begin.0);
            assert_eq!(reopened.attempts()[0].result_version(), Some(old_result.0));
            assert_eq!(reopened.attempts()[0].response_bytes().unwrap(), old_attempt.3);
            assert_eq!(reopened.attempts()[1].begin_version(), native_begin_version);
            assert_eq!(reopened.attempts()[1].result_version(), Some(native_result_version));
            assert_eq!(reopened.attempts()[1].response_bytes().unwrap(), native_response);
            assert_eq!(reopened.pending_source_identities(), pending_sources);
            assert_eq!(reopened.pending_research_queries(), pending_research);
            let reopened_old_source = reopened
                .global_news(GlobalNewsProvider::Eastmoney)
                .unwrap();
            assert_eq!(
                (
                    reopened_old_source.final_bytes().unwrap(),
                    reopened_old_source.audit_receipt().unwrap(),
                ),
                (old_source.0.as_slice(), &old_source.1)
            );
            let reopened_terminal = reopened.query_terminal(QueryKey::Gateway(2)).unwrap();
            assert!(reopened_terminal.was_called());
            assert_eq!(reopened_terminal.version(), native_result_version + 1);
            assert_eq!(reopened_terminal.native_bytes(), native_terminal_bytes);
            assert_eq!(reopened_terminal.audit_receipt(), Some(&native_receipt));
            let NativeOutcome::News(Err(error)) = reopened_terminal.native() else {
                panic!("TEST_CODE reopened Gateway(2) lost exact provider refusal");
            };
            assert_eq!((error.capability(), error.provider(), error.audit_outcome(),
                        error.reason_code(), error.retryable()),
                       ("GlobalNews", Some(ProviderId::Cailianpress), "partial",
                        "invalid_evidence", false));
            assert_eq!(
                error.message(),
                "ExternalV1 GlobalNews selected provider differs from exact request"
            );
            let reopened_run = local.inspect_run(&intent).unwrap();
            assert_eq!(reopened_run.context().canonical_bytes(), old_context);
            assert_eq!(reopened_run.head_version(), old_head + 5);
            assert_eq!(reopened_run.lease_generation(), old_generation + 1);
            drop(reopened);
            drop(local);
            assert_eq!(
                v11_migration_tests::DatabaseState::capture(business.connection()),
                durable_before_reopen
            );
            assert_eq!(business.count("data_acquisition_audit"), audit_before + 1);
            assert_eq!(external_server.as_ref().unwrap().snapshot(), external_after_data);
            assert_eq!(parent_server.as_ref().unwrap().snapshot_with_tcp_for_test(), parent_before_migration);
            assert_eq!(parent_server.as_ref().unwrap().membership_snapshot(), memberships_before_migration);
        },
    ))
    .catch_unwind()
    .await;

    control_tests::cleanup_external_case(
        &mut business,
        &mut parent_server,
        &mut external_server,
        LABEL,
    )
    .await;
    drop(business);
    match body {
        Ok(Ok(())) => {}
        Ok(Err(_)) => panic!("TEST_CODE {LABEL} body deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[path = "chain_post_close_schema_v12_rejection_tests.rs"]
mod rejection_tests;
