use super::*;
use rusqlite::types::Value;
use rusqlite::{params, Connection};
use std::collections::BTreeMap;
use std::time::Duration;

const V10_SHA256: &str = "1f66f45fb534da1fa7b77fedf161924b60a6e2b69ae1a7c4d1577ab04125ccd8";
const V11_SHA256: &str = "8ee02c8ab5bb7e23ee7904f4db08ccc86b7f504a7ae86b37fc88c75d4a453faa";
const FUTURE_11_SHA256: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FUTURE_12_SHA256: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FUTURE_13_SHA256: &str =
    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FUTURE_14_SHA256: &str =
    "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

const V11_TABLES: [&str; 8] = [
    "chain_post_close_macro_plans",
    "chain_post_close_macro_request_plans",
    "chain_post_close_macro_readiness_episode_plans",
    "chain_post_close_macro_control_attempt_begins",
    "chain_post_close_macro_control_attempt_results",
    "chain_post_close_macro_attempt_begins",
    "chain_post_close_macro_attempt_results",
    "chain_post_close_macro_source_finals",
];

const V11_OBJECTS: [&str; 66] = [
    "chain_post_close_macro_plans",
    "chain_post_close_macro_request_plans",
    "chain_post_close_macro_readiness_episode_plans",
    "chain_post_close_macro_control_attempt_begins",
    "chain_post_close_macro_control_attempt_results",
    "chain_post_close_macro_attempt_begins",
    "chain_post_close_macro_attempt_results",
    "chain_post_close_macro_source_finals",
    "chain_post_close_macro_plans_guard",
    "chain_post_close_macro_request_plans_guard",
    "chain_post_close_macro_readiness_episode_plans_guard",
    "chain_post_close_macro_control_attempt_begins_guard",
    "chain_post_close_macro_control_attempt_results_guard",
    "chain_post_close_macro_attempt_begins_guard",
    "chain_post_close_macro_attempt_results_guard",
    "chain_post_close_macro_source_finals_guard",
    "chain_post_close_macro_plans_update",
    "chain_post_close_macro_plans_delete",
    "chain_post_close_macro_request_plans_update",
    "chain_post_close_macro_request_plans_delete",
    "chain_post_close_macro_readiness_episode_plans_update",
    "chain_post_close_macro_readiness_episode_plans_delete",
    "chain_post_close_macro_control_attempt_begins_update",
    "chain_post_close_macro_control_attempt_begins_delete",
    "chain_post_close_macro_control_attempt_results_update",
    "chain_post_close_macro_control_attempt_results_delete",
    "chain_post_close_macro_attempt_begins_update",
    "chain_post_close_macro_attempt_begins_delete",
    "chain_post_close_macro_attempt_results_update",
    "chain_post_close_macro_attempt_results_delete",
    "chain_post_close_macro_source_finals_update",
    "chain_post_close_macro_source_finals_delete",
    "chain_post_close_stage_begins_macro_fence",
    "chain_post_close_stage_results_macro_fence",
    "chain_post_close_concept_cache_writes_macro_fence",
    "chain_post_close_cluster_configurations_macro_fence",
    "chain_post_close_cluster_materials_macro_fence",
    "chain_post_close_chain_daily_applications_macro_fence",
    "chain_post_close_board_attempt_begins_macro_fence",
    "chain_post_close_board_attempt_results_macro_fence",
    "chain_post_close_board_kind_finals_macro_fence",
    "chain_post_close_board_directory_materials_macro_fence",
    "chain_post_close_board_selections_macro_fence",
    "chain_post_close_board_status_materials_macro_fence",
    "chain_post_close_board_error_materials_macro_fence",
    "chain_post_close_concept_rpc_occurrences_macro_fence",
    "chain_post_close_concept_rpc_attempt_begins_macro_fence",
    "chain_post_close_concept_rpc_attempt_results_macro_fence",
    "chain_post_close_concept_rpc_status_materials_macro_fence",
    "chain_post_close_concept_rpc_error_materials_macro_fence",
    "chain_post_close_concept_rpc_finals_macro_fence",
    "chain_post_close_position_materials_macro_fence",
    "chain_post_close_position_concept_materials_macro_fence",
    "chain_post_close_position_concept_rpc_occurrences_macro_fence",
    "chain_post_close_position_concept_rpc_attempt_begins_macro_fence",
    "chain_post_close_position_concept_rpc_attempt_results_macro_fence",
    "chain_post_close_position_concept_rpc_status_materials_macro_fence",
    "chain_post_close_position_concept_rpc_error_materials_macro_fence",
    "chain_post_close_position_concept_rpc_finals_macro_fence",
    "chain_post_close_position_concept_cache_writes_macro_fence",
    "chain_post_close_dragon_tiger_occurrences_macro_fence",
    "chain_post_close_dragon_tiger_attempt_begins_macro_fence",
    "chain_post_close_dragon_tiger_attempt_results_macro_fence",
    "chain_post_close_dragon_tiger_status_materials_macro_fence",
    "chain_post_close_dragon_tiger_error_materials_macro_fence",
    "chain_post_close_dragon_tiger_finals_macro_fence",
];

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DatabaseState {
    pub(super) catalog: Vec<Vec<Value>>,
    pub(super) tables: BTreeMap<String, Vec<Vec<Value>>>,
    pub(super) application_id: i64,
    pub(super) user_version: i64,
    pub(super) query_only: i64,
    pub(super) autocommit: bool,
}

fn rows(connection: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = connection.prepare(sql).expect("TEST_CODE v11 snapshot SQL");
    let width = statement.column_count();
    statement
        .query_map([], |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<Value>>>()
        })
        .expect("TEST_CODE v11 snapshot query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("TEST_CODE v11 snapshot rows")
}

fn table_names(connection: &Connection) -> Vec<String> {
    connection
        .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn ordered_table_rows(connection: &Connection, name: &str) -> Vec<Vec<Value>> {
    let quoted = name.replace('"', "\"\"");
    let query = format!("SELECT * FROM \"{quoted}\"");
    let width = connection.prepare(&query).unwrap().column_count();
    let order = (1..=width)
        .map(|column| column.to_string())
        .collect::<Vec<_>>()
        .join(",");
    rows(connection, &format!("{query} ORDER BY {order}"))
}

impl DatabaseState {
    pub(super) fn capture(connection: &Connection) -> Self {
        let tables = table_names(connection)
            .into_iter()
            .map(|name| {
                let rows = ordered_table_rows(connection, &name);
                (name, rows)
            })
            .collect();
        Self {
            catalog: rows(
                connection,
                "SELECT type,name,tbl_name,rootpage,CAST(sql AS BLOB) \
                 FROM sqlite_schema ORDER BY type,name",
            ),
            tables,
            application_id: connection
                .query_row("PRAGMA application_id", [], |row| row.get(0))
                .unwrap(),
            user_version: connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap(),
            query_only: connection
                .query_row("PRAGMA query_only", [], |row| row.get(0))
                .unwrap(),
            autocommit: connection.is_autocommit(),
        }
    }
}

struct RealV10 {
    business: V2BusinessFixture,
    intent: IntentId,
}

async fn real_v10_fixture(run_id: &str) -> RealV10 {
    let mut business = V2BusinessFixture::new();
    let mut parent_server = None;
    let built = std::panic::AssertUnwindSafe(tokio::time::timeout(
        Duration::from_secs(60),
        async {
            control_tests::setup_v10_parent(&mut business, &mut parent_server, run_id).await
        },
    ))
    .catch_unwind()
    .await;
    match built {
        Ok(Ok(baseline)) => {
            let control_tests::ExternalParentBaseline {
                source,
                queries,
                intent,
                ..
            } = baseline;
            drop(queries);
            drop(source);
            let server = parent_server.take().unwrap();
            std::panic::AssertUnwindSafe(server.finish())
                .catch_unwind()
                .await
                .expect("TEST_CODE v10 parent cleanup panic");
            RealV10 { business, intent }
        }
        Ok(Err(_)) => {
            if let Some(server) = parent_server.take() {
                let _ = std::panic::AssertUnwindSafe(server.finish())
                .catch_unwind()
                .await;
            }
            panic!("TEST_CODE v10 fixture body deadline");
        }
        Err(panic) => {
            if let Some(server) = parent_server.take() {
                let _ = std::panic::AssertUnwindSafe(server.finish())
                .catch_unwind()
                .await;
            }
            std::panic::resume_unwind(panic)
        }
    }
}

fn assert_real_v10_facts(fixture: &RealV10) {
    assert_eq!(
        fixture
            .business
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_runs WHERE intent_id=?1",
                [fixture.intent.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        fixture
            .business
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_dragon_tiger_finals \
                 WHERE intent_id=?1",
                [fixture.intent.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        fixture
            .business
            .connection()
            .query_row(
                "SELECT count(*) FROM data_acquisition_audit WHERE capability='R-04'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        fixture
            .business
            .connection()
            .query_row(
                "SELECT count(*) FROM data_acquisition_audit_chain chain \
                 JOIN data_acquisition_audit audit ON audit.id=chain.acquisition_audit_id \
                 WHERE audit.capability='R-04'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

fn assert_v10_rows_preserved(before: &DatabaseState, after: &DatabaseState) {
    assert_eq!(after.application_id, before.application_id);
    assert_eq!(after.user_version, before.user_version);
    assert_eq!(after.query_only, before.query_only);
    assert_eq!(after.autocommit, before.autocommit);
    for old in &before.catalog {
        assert!(after.catalog.contains(old), "TEST_CODE old catalog row changed: {old:?}");
    }
    for (name, old_rows) in &before.tables {
        let current = after.tables.get(name).unwrap();
        match name.as_str() {
            "chain_post_close_layouts" | "chain_post_close_layout_objects" => {
                let old = current
                    .iter()
                    .filter(|row| matches!(row.first(), Some(Value::Integer(version)) if *version <= 10))
                    .cloned()
                    .collect::<Vec<_>>();
                assert_eq!(&old, old_rows, "TEST_CODE old metadata changed: {name}");
            }
            _ => assert_eq!(current, old_rows, "TEST_CODE old table changed: {name}"),
        }
    }
}

fn defined_catalog_addition_names(before: &DatabaseState, after: &DatabaseState) -> Vec<String> {
    let mut names = after
        .catalog
        .iter()
        .filter(|row| !before.catalog.contains(row) && row[4] != Value::Null)
        .map(|row| match &row[1] {
            Value::Text(name) => name.clone(),
            other => panic!("TEST_CODE catalog name type: {other:?}"),
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn assert_only_v11_generated_indexes(before: &DatabaseState, after: &DatabaseState) {
    let generated = after
        .catalog
        .iter()
        .filter(|row| !before.catalog.contains(row) && row[4] == Value::Null)
        .collect::<Vec<_>>();
    assert!(!generated.is_empty());
    for row in generated {
        assert_eq!(row[0], Value::Text("index".to_owned()));
        let name = match &row[1] {
            Value::Text(name) => name,
            other => panic!("TEST_CODE generated index name type: {other:?}"),
        };
        let table = match &row[2] {
            Value::Text(table) => table,
            other => panic!("TEST_CODE generated index table type: {other:?}"),
        };
        assert!(V11_TABLES.contains(&table.as_str()));
        assert!(name.starts_with(&format!("sqlite_autoindex_{table}_")));
    }
}

fn expected_v11_names() -> Vec<String> {
    let mut names = V11_OBJECTS.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
    names.sort();
    names
}

fn assert_v11_metadata(connection: &Connection) {
    let header: (i64, i64, String, i64, i64, i64, String, String) = connection
        .query_row(
            "SELECT layout_version,predecessor_layout_version,predecessor_bundle_sha256,\
             artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
             FROM chain_post_close_layouts WHERE layout_version=11",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        header,
        (
            11,
            10,
            V10_SHA256.to_owned(),
            1,
            1,
            1,
            "chain-post-close-layout-v11".to_owned(),
            V11_SHA256.to_owned(),
        )
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects \
                 WHERE layout_version=11",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        225
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects registry \
                 JOIN sqlite_schema catalog \
                   ON catalog.name=registry.name AND catalog.type=registry.object_type \
                  AND CAST(catalog.sql AS BLOB)=CAST(registry.definition AS BLOB) \
                 WHERE registry.layout_version=11",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        225
    );
    for table in V11_TABLES {
        assert_eq!(
            connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0,
            "TEST_CODE new v11 table must begin empty: {table}"
        );
    }
}

fn install_first_v11_object(connection: &Connection) {
    const DDL: &str = include_str!("chain_post_close.v11.sql");
    let start = DDL
        .find("CREATE TABLE chain_post_close_macro_plans")
        .unwrap();
    let end = DDL
        .find("CREATE TABLE chain_post_close_macro_request_plans")
        .unwrap();
    connection
        .execute_batch(&DDL[start..end])
        .expect("TEST_CODE install only first frozen v11 object");
}

fn append_future_metadata(expected: &mut DatabaseState, versions: &[i64]) {
    let layouts = expected.tables.get_mut("chain_post_close_layouts").unwrap();
    if versions.contains(&11) {
        layouts.push(vec![
            Value::Integer(11),
            Value::Integer(10),
            Value::Text(V10_SHA256.to_owned()),
            Value::Integer(1),
            Value::Integer(1),
            Value::Integer(1),
            Value::Text("TEST_CODE_FUTURE_LAYOUT_11".to_owned()),
            Value::Text(FUTURE_11_SHA256.to_owned()),
        ]);
    }
    if versions.contains(&12) {
        let predecessor_sha = layouts
            .iter()
            .find(|row| row[0] == Value::Integer(11))
            .and_then(|row| match &row[7] {
                Value::Text(digest) => Some(digest.clone()),
                _ => None,
            })
            .expect("TEST_CODE expected unique layout 11 predecessor");
        layouts.push(vec![
            Value::Integer(12),
            Value::Integer(11),
            Value::Text(predecessor_sha),
            Value::Integer(1),
            Value::Integer(1),
            Value::Integer(1),
            Value::Text("TEST_CODE_FUTURE_LAYOUT_12".to_owned()),
            Value::Text(FUTURE_12_SHA256.to_owned()),
        ]);
    }
    if versions.contains(&13) {
        let predecessor_sha = layouts
            .iter()
            .find(|row| row[0] == Value::Integer(12))
            .and_then(|row| match &row[7] {
                Value::Text(digest) => Some(digest.clone()),
                _ => None,
            })
            .expect("TEST_CODE expected unique layout 12 predecessor");
        layouts.push(vec![
            Value::Integer(13),
            Value::Integer(12),
            Value::Text(predecessor_sha),
            Value::Integer(1),
            Value::Integer(1),
            Value::Integer(1),
            Value::Text("TEST_CODE_FUTURE_LAYOUT_13".to_owned()),
            Value::Text(FUTURE_13_SHA256.to_owned()),
        ]);
    }
    if versions.contains(&14) {
        let predecessor_sha = layouts
            .iter()
            .find(|row| row[0] == Value::Integer(13))
            .and_then(|row| match &row[7] {
                Value::Text(digest) => Some(digest.clone()),
                _ => None,
            })
            .expect("TEST_CODE expected unique layout 13 predecessor");
        layouts.push(vec![
            Value::Integer(14),
            Value::Integer(13),
            Value::Text(predecessor_sha),
            Value::Integer(1),
            Value::Integer(1),
            Value::Integer(1),
            Value::Text("TEST_CODE_FUTURE_LAYOUT_14".to_owned()),
            Value::Text(FUTURE_14_SHA256.to_owned()),
        ]);
    }
    let registry = expected
        .tables
        .get_mut("chain_post_close_layout_objects")
        .unwrap();
    if versions.contains(&11) {
        let rows = registry
            .iter()
            .filter(|row| row[0] == Value::Integer(10))
            .cloned()
            .map(|mut row| {
                row[0] = Value::Integer(11);
                row
            })
            .collect::<Vec<_>>();
        registry.extend(rows);
    }
    if versions.contains(&12) {
        let rows = registry
            .iter()
            .filter(|row| row[0] == Value::Integer(11))
            .cloned()
            .map(|mut row| {
                row[0] = Value::Integer(12);
                row
            })
            .collect::<Vec<_>>();
        registry.extend(rows);
    }
    if versions.contains(&13) {
        let rows = registry
            .iter()
            .filter(|row| row[0] == Value::Integer(12))
            .cloned()
            .map(|mut row| {
                row[0] = Value::Integer(13);
                row
            })
            .collect::<Vec<_>>();
        registry.extend(rows);
    }
    if versions.contains(&14) {
        let rows = registry
            .iter()
            .filter(|row| row[0] == Value::Integer(13))
            .cloned()
            .map(|mut row| {
                row[0] = Value::Integer(14);
                row
            })
            .collect::<Vec<_>>();
        registry.extend(rows);
    }
}

fn install_future_11_and_12(connection: &Connection) {
    connection
        .execute_batch(
            "BEGIN IMMEDIATE; \
             INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
             SELECT 11,name,object_type,definition FROM chain_post_close_layout_objects \
             WHERE layout_version=10; \
             INSERT INTO chain_post_close_layouts(layout_version,predecessor_layout_version,\
             predecessor_bundle_sha256,artifact_codec_version,input_codec_version,\
             stage_codec_version,description,bundle_sha256) \
             SELECT 11,10,bundle_sha256,1,1,1,'TEST_CODE_FUTURE_LAYOUT_11',\
             'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' \
             FROM chain_post_close_layouts WHERE layout_version=10; \
             INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
             SELECT 12,name,object_type,definition FROM chain_post_close_layout_objects \
             WHERE layout_version=11; \
             INSERT INTO chain_post_close_layouts(layout_version,predecessor_layout_version,\
             predecessor_bundle_sha256,artifact_codec_version,input_codec_version,\
             stage_codec_version,description,bundle_sha256) \
             SELECT 12,11,bundle_sha256,1,1,1,'TEST_CODE_FUTURE_LAYOUT_12',\
             'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' \
             FROM chain_post_close_layouts WHERE layout_version=11; \
             COMMIT;",
        )
        .unwrap();
}

fn install_future_12(connection: &Connection) {
    connection
        .execute_batch(
            "BEGIN IMMEDIATE; \
             INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
             SELECT 12,name,object_type,definition FROM chain_post_close_layout_objects \
             WHERE layout_version=11; \
             INSERT INTO chain_post_close_layouts(layout_version,predecessor_layout_version,\
             predecessor_bundle_sha256,artifact_codec_version,input_codec_version,\
             stage_codec_version,description,bundle_sha256) \
             SELECT 12,11,bundle_sha256,1,1,1,'TEST_CODE_FUTURE_LAYOUT_12',\
             'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' \
             FROM chain_post_close_layouts WHERE layout_version=11; \
             COMMIT;",
        )
        .unwrap();
}

fn install_future_14(connection: &Connection) {
    connection
        .execute_batch(
            "BEGIN IMMEDIATE; \
             INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
             SELECT 14,name,object_type,definition FROM chain_post_close_layout_objects \
             WHERE layout_version=13; \
             INSERT INTO chain_post_close_layouts(layout_version,predecessor_layout_version,\
             predecessor_bundle_sha256,artifact_codec_version,input_codec_version,\
             stage_codec_version,description,bundle_sha256) \
             SELECT 14,13,bundle_sha256,1,1,1,'TEST_CODE_FUTURE_LAYOUT_14',\
             'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd' \
             FROM chain_post_close_layouts WHERE layout_version=13; \
             COMMIT;",
        )
        .unwrap();
}

fn install_future_13(connection: &Connection) {
    connection
        .execute_batch(
            "BEGIN IMMEDIATE; \
             INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
             SELECT 13,name,object_type,definition FROM chain_post_close_layout_objects \
             WHERE layout_version=12; \
             INSERT INTO chain_post_close_layouts(layout_version,predecessor_layout_version,\
             predecessor_bundle_sha256,artifact_codec_version,input_codec_version,\
             stage_codec_version,description,bundle_sha256) \
             SELECT 13,12,bundle_sha256,1,1,1,'TEST_CODE_FUTURE_LAYOUT_13',\
             'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc' \
             FROM chain_post_close_layouts WHERE layout_version=12; \
             COMMIT;",
        )
        .unwrap();
}

#[derive(Clone, Copy)]
enum MetadataDamage {
    Header,
    Registry,
}

fn inject_v10_metadata_damage(connection: &Connection, damage: MetadataDamage) -> DatabaseState {
    let before = DatabaseState::capture(connection);
    let mut expected = before.clone();
    let registry_damage = match damage {
        MetadataDamage::Header => {
            let row = expected
                .tables
                .get_mut("chain_post_close_layouts")
                .unwrap()
                .iter_mut()
                .find(|row| row[0] == Value::Integer(10))
                .unwrap();
            row[6] = Value::Text("TEST_CODE_DAMAGED_V10_HEADER".to_owned());
            None
        }
        MetadataDamage::Registry => {
            let name: String = connection
                .query_row(
                    "SELECT name FROM chain_post_close_layout_objects \
                     WHERE layout_version=10 ORDER BY name LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let original: String = connection
                .query_row(
                    "SELECT definition FROM chain_post_close_layout_objects \
                     WHERE layout_version=10 AND name=?1",
                    [&name],
                    |row| row.get(0),
                )
                .unwrap();
            let damaged = format!("{original} ");
            let row = expected
                .tables
                .get_mut("chain_post_close_layout_objects")
                .unwrap()
                .iter_mut()
                .find(|row| {
                    row[0] == Value::Integer(10) && row[1] == Value::Text(name.clone())
                })
                .unwrap();
            row[3] = Value::Text(damaged.clone());
            Some((name, damaged))
        }
    };
    let transaction = connection.unchecked_transaction().unwrap();
    match damage {
        MetadataDamage::Header => {
            let trigger: String = transaction
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='trigger' \
                     AND name='chain_post_close_layouts_update'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            transaction
                .execute_batch("DROP TRIGGER chain_post_close_layouts_update")
                .unwrap();
            assert_eq!(
                transaction
                    .execute(
                        "UPDATE chain_post_close_layouts \
                         SET description='TEST_CODE_DAMAGED_V10_HEADER' \
                         WHERE layout_version=10",
                        [],
                    )
                    .unwrap(),
                1
            );
            transaction.execute_batch(&trigger).unwrap();
        }
        MetadataDamage::Registry => {
            let (name, damaged) = registry_damage.unwrap();
            let trigger: String = transaction
                .query_row(
                    "SELECT sql FROM sqlite_schema WHERE type='trigger' \
                     AND name='chain_post_close_layout_objects_update'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            transaction
                .execute_batch("DROP TRIGGER chain_post_close_layout_objects_update")
                .unwrap();
            assert_eq!(
                transaction
                    .execute(
                        "UPDATE chain_post_close_layout_objects SET definition=?1 \
                         WHERE layout_version=10 AND name=?2",
                        params![damaged, name],
                    )
                    .unwrap(),
                1
            );
            transaction.execute_batch(&trigger).unwrap();
        }
    }
    transaction.commit().unwrap();
    assert!(connection.is_autocommit());
    assert_eq!(DatabaseState::capture(connection), expected);
    expected
}

#[tokio::test]
async fn v10_to_v11_preserves_real_parent_facts_and_reopens_with_exact_catalog_additions() {
    let mut fixture = real_v10_fixture("TEST_CODE_V11_MIGRATION_SUCCESS").await;
    assert_real_v10_facts(&fixture);
    let before = DatabaseState::capture(fixture.business.connection());
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .migrate_schema_v10_to_v11()
            .unwrap()
            .schema_version(),
        11
    );
    let after = DatabaseState::capture(fixture.business.connection());
    assert_v10_rows_preserved(&before, &after);
    assert_eq!(
        defined_catalog_addition_names(&before, &after),
        expected_v11_names()
    );
    assert_only_v11_generated_indexes(&before, &after);
    assert_v11_metadata(fixture.business.connection());
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .verify_schema_v10_reader(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    fixture.business.reopen();
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        11
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), after);
    assert_real_v10_facts(&fixture);
}

#[tokio::test]
async fn v10_to_v11_half_installed_first_macro_object_is_rejected_without_repair() {
    let mut fixture = real_v10_fixture("TEST_CODE_V11_MIGRATION_HALF_INSTALL").await;
    install_first_v11_object(fixture.business.connection());
    let damaged = DatabaseState::capture(fixture.business.connection());
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), damaged);
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .migrate_schema_v10_to_v11(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), damaged);
    fixture.business.reopen();
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), damaged);
}

#[tokio::test]
async fn v10_to_v11_real_commit_contention_rolls_back_and_reopens_for_retry() {
    let mut fixture = real_v10_fixture("TEST_CODE_V11_MIGRATION_COMMIT").await;
    let mode: String = fixture
        .business
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "delete");
    let before = DatabaseState::capture(fixture.business.connection());
    let reader = BusinessIntentStore::open(&fixture.business.database()).unwrap();
    reader.connection.execute_batch("BEGIN DEFERRED;").unwrap();
    assert_eq!(
        reader
            .connection
            .query_row("SELECT count(*) FROM chain_post_close_layouts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        9
    );
    let reader_before = DatabaseState::capture(&reader.connection);
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .migrate_schema_v10_to_v11(),
        Err(ChainPostCloseError::StorageFailed {
            operation: "v11 commit"
        })
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), before);
    assert_eq!(DatabaseState::capture(&reader.connection), reader_before);
    assert!(fixture.business.connection().is_autocommit());
    assert_eq!(
        fixture
            .business
            .connection()
            .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    reader.connection.execute_batch("ROLLBACK;").unwrap();
    reader.connection.close().unwrap();
    fixture.business.reopen();
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .verify_schema_v10_reader()
            .unwrap()
            .schema_version(),
        10
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), before);
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .migrate_schema_v10_to_v11()
            .unwrap()
            .schema_version(),
        11
    );
    assert_v11_metadata(fixture.business.connection());
}

#[tokio::test]
async fn exact_v10_catalog_with_future_metadata_is_unsupported_and_never_repaired() {
    let mut fixture = real_v10_fixture("TEST_CODE_V11_MIGRATION_FUTURE_V10").await;
    let before = DatabaseState::capture(fixture.business.connection());
    let mut forged_12 = before.clone();
    append_future_metadata(&mut forged_12, &[11, 12]);
    install_future_11_and_12(fixture.business.connection());
    assert_eq!(DatabaseState::capture(fixture.business.connection()), forged_12);
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), forged_12);

    // Layout 13 is a sealed layout now: a copied catalog under its header is
    // drift of a known layout, not an unknown future version.
    let mut forged_13 = forged_12.clone();
    append_future_metadata(&mut forged_13, &[13]);
    install_future_13(fixture.business.connection());
    assert_eq!(DatabaseState::capture(fixture.business.connection()), forged_13);
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), forged_13);

    let mut expected = forged_13.clone();
    append_future_metadata(&mut expected, &[14]);
    install_future_14(fixture.business.connection());
    assert_eq!(DatabaseState::capture(fixture.business.connection()), expected);
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), expected);
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .verify_schema_v10_reader(),
        Err(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), expected);
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .migrate_schema_v10_to_v11(),
        Err(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), expected);
    fixture.business.reopen();
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), expected);
}

#[tokio::test]
async fn real_v11_catalog_with_future_metadata_distinguishes_current_and_migration_errors() {
    let mut fixture = real_v10_fixture("TEST_CODE_V11_MIGRATION_FUTURE_V11").await;
    fixture
        .business
        .chain_post_close()
        .migrate_schema_v10_to_v11()
        .unwrap();
    let before = DatabaseState::capture(fixture.business.connection());
    let mut forged_12 = before.clone();
    append_future_metadata(&mut forged_12, &[12]);
    install_future_12(fixture.business.connection());
    assert_eq!(DatabaseState::capture(fixture.business.connection()), forged_12);
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), forged_12);

    // Layout 13 is a sealed layout now: a copied catalog under its header is
    // drift of a known layout, not an unknown future version.
    let mut forged_13 = forged_12.clone();
    append_future_metadata(&mut forged_13, &[13]);
    install_future_13(fixture.business.connection());
    assert_eq!(DatabaseState::capture(fixture.business.connection()), forged_13);
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), forged_13);

    let mut expected = forged_13.clone();
    append_future_metadata(&mut expected, &[14]);
    install_future_14(fixture.business.connection());
    assert_eq!(DatabaseState::capture(fixture.business.connection()), expected);
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), expected);
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .migrate_schema_v10_to_v11(),
        Err(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), expected);
    fixture.business.reopen();
    assert_eq!(
        fixture.business.chain_post_close().verify_schema(),
        Err(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), expected);
}

#[tokio::test]
async fn damaged_v10_header_and_registry_are_rejected_without_repair() {
    for (run_id, damage) in [
        ("TEST_CODE_V11_MIGRATION_DAMAGED_HEADER", MetadataDamage::Header),
        (
            "TEST_CODE_V11_MIGRATION_DAMAGED_REGISTRY",
            MetadataDamage::Registry,
        ),
    ] {
        let mut fixture = real_v10_fixture(run_id).await;
        let damaged = inject_v10_metadata_damage(fixture.business.connection(), damage);
        assert_eq!(
            fixture.business.chain_post_close().verify_schema(),
            Err(ChainPostCloseError::SchemaRejected)
        );
        assert_eq!(DatabaseState::capture(fixture.business.connection()), damaged);
        assert_eq!(
            fixture
                .business
                .chain_post_close()
                .verify_schema_v10_reader(),
            Err(ChainPostCloseError::SchemaRejected)
        );
        assert_eq!(
            fixture
                .business
                .chain_post_close()
                .migrate_schema_v10_to_v11(),
            Err(ChainPostCloseError::SchemaRejected)
        );
        assert_eq!(DatabaseState::capture(fixture.business.connection()), damaged);
        fixture.business.reopen();
        assert_eq!(
            fixture.business.chain_post_close().verify_schema(),
            Err(ChainPostCloseError::SchemaRejected)
        );
        assert_eq!(DatabaseState::capture(fixture.business.connection()), damaged);
    }
}

#[tokio::test]
async fn query_only_v10_writer_rejects_migration_without_state_change() {
    let mut fixture = real_v10_fixture("TEST_CODE_V11_MIGRATION_QUERY_ONLY").await;
    fixture
        .business
        .connection()
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    let before = DatabaseState::capture(fixture.business.connection());
    assert_eq!(before.query_only, 1);
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .migrate_schema_v10_to_v11(),
        Err(ChainPostCloseError::ConnectionSafeguardFailed)
    );
    assert_eq!(DatabaseState::capture(fixture.business.connection()), before);
}

#[tokio::test]
async fn already_open_idle_read_only_connection_observes_committed_v11_without_write_upgrade() {
    let mut fixture = real_v10_fixture("TEST_CODE_V11_MIGRATION_OPEN_READER").await;
    let database = fixture.business.database();
    let mut reader = BusinessIntentStore::open(&database).unwrap();
    reader.connection.execute_batch("PRAGMA query_only=ON").unwrap();
    assert!(reader.connection.is_autocommit());
    assert_eq!(
        reader
            .connection
            .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        ChainPostClose { store: &mut reader }
            .verify_schema()
            .unwrap()
            .schema_version(),
        10
    );
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .migrate_schema_v10_to_v11()
            .unwrap()
            .schema_version(),
        11
    );
    assert_eq!(
        ChainPostClose { store: &mut reader }
            .verify_schema()
            .unwrap()
            .schema_version(),
        11
    );
    assert!(reader.connection.is_autocommit());
    assert_eq!(
        reader
            .connection
            .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    reader.connection.close().unwrap();
    fixture.business.reopen();
    assert_eq!(
        fixture
            .business
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        11
    );
}
