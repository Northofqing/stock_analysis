use super::*;
use rusqlite::OpenFlags;

const V8_OBJECTS: [&str; 8] = [
    "chain_post_close_position_materials",
    "chain_post_close_position_concept_materials",
    "chain_post_close_position_materials_guard",
    "chain_post_close_position_materials_update",
    "chain_post_close_position_materials_delete",
    "chain_post_close_position_concept_materials_guard",
    "chain_post_close_position_concept_materials_update",
    "chain_post_close_position_concept_materials_delete",
];

#[derive(Debug, PartialEq)]
struct OwnedDatabaseState {
    chain_names: Vec<String>,
    chain_rows: BTreeMap<String, Vec<Vec<Value>>>,
    audits: Vec<Vec<Vec<Value>>>,
    foundation: BTreeMap<String, Vec<Vec<Value>>>,
    foundation_catalog: Vec<Vec<Value>>,
    business: Vec<Vec<Vec<Value>>>,
    defined_catalog: Vec<Vec<Value>>,
    database_identity: (i64, i64),
}

fn ordered_table_rows(connection: &Connection, name: &str) -> Vec<Vec<Value>> {
    let quoted = name.replace('"', "\"\"");
    let query = format!("SELECT * FROM \"{quoted}\"");
    let width = connection.prepare(&query).unwrap().column_count();
    let order = (1..=width)
        .map(|column| column.to_string())
        .collect::<Vec<_>>()
        .join(",");
    all_rows(connection, &format!("{query} ORDER BY {order}"))
}

fn named_table_rows(
    connection: &Connection,
    names: &[String],
) -> BTreeMap<String, Vec<Vec<Value>>> {
    names
        .iter()
        .map(|name| (name.clone(), ordered_table_rows(connection, name)))
        .collect()
}

impl OwnedDatabaseState {
    fn capture(connection: &Connection) -> Self {
        let chain_names = table_names(connection);
        let chain_rows = named_table_rows(connection, &chain_names);
        let foundation_names = [
            "push_foundation_schema",
            "push_foundation_objects",
            "push_intents",
            "push_intent_transitions",
            "push_activation_manifests",
            "push_promotion_journal",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        Self {
            chain_names,
            chain_rows,
            audits: [
                "SELECT * FROM data_acquisition_audit ORDER BY id",
                "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id",
            ]
            .into_iter()
            .map(|sql| all_rows(connection, sql))
            .collect(),
            foundation: named_table_rows(connection, &foundation_names),
            foundation_catalog: all_rows(
                connection,
                "SELECT r.name,r.object_type,CAST(r.definition AS BLOB),CAST(s.sql AS BLOB) \
                 FROM push_foundation_objects AS r JOIN sqlite_schema AS s \
                   ON s.name=r.name AND s.type=r.object_type ORDER BY r.name",
            ),
            business: [
                "SELECT * FROM stock_concepts ORDER BY code",
                "SELECT * FROM chain_daily ORDER BY date,concept",
                "SELECT * FROM stock_position ORDER BY id",
            ]
            .into_iter()
            .map(|sql| all_rows(connection, sql))
            .collect(),
            defined_catalog: all_rows(
                connection,
                "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
                 WHERE sql IS NOT NULL ORDER BY name,type,tbl_name",
            ),
            database_identity: (
                connection
                    .query_row("PRAGMA main.application_id", [], |row| row.get(0))
                    .unwrap(),
                connection
                    .query_row("PRAGMA main.user_version", [], |row| row.get(0))
                    .unwrap(),
            ),
        }
    }
}

fn v8_objects(connection: &Connection) -> Vec<Vec<Value>> {
    let names = V8_OBJECTS
        .iter()
        .map(|name| format!("'{}'", name.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    all_rows(
        connection,
        &format!(
            "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema \
             WHERE name IN ({names}) ORDER BY name"
        ),
    )
}

#[tokio::test]
async fn v7_to_v8_real_commit_failure_preserves_old_facts_and_reopens_for_retry() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let mut fixture = V2BusinessFixture::new();
        fixture.install_v2();
        fixture.chain_post_close().migrate_schema_v2_to_v3().unwrap();
        cluster_tests::install_business_rows(&fixture);
        fixture.chain_post_close().migrate_schema_v3_to_v4().unwrap();
        install_br159(&mut fixture);
        fixture.chain_post_close().migrate_schema_v4_to_v5().unwrap();
        fixture.chain_post_close().migrate_schema_v5_to_v6().unwrap();
        fixture.chain_post_close().migrate_schema_v6_to_v7().unwrap();

        let (client, server) = spawn_board_loopback().await;
        let source = GrpcSource::from_board_loopback_test_client(client);
        let queries = source.connected_board_queries().await.unwrap();
        let stocks = cluster_tests::cluster_stocks();
        let config = local_config(BUILD_A);
        let context = build_single_user_local_chain_post_close_context(
            &config,
            run_input("TEST_CODE_RUN_POSITIONS_V8_MIGRATION_COMMIT"),
        )
        .unwrap();
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .acquire_run(
                context,
                fixed_input(stocks.clone()),
                lease_request(OWNER_A, 1_000_000, 60_000_000, None),
            )
            .unwrap();
        let intent = lease.intent_id().clone();
        let clock = ControlledClock::new(at(2_000_000));
        let mut io = local
            .concept_rpc_preparation_io_v7(
                lease,
                &queries,
                &clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks,
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE real v7 preparation stops at Positions");
        assert!(matches!(
            stopped.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::Positions
            })
        ));
        drop(io);
        let original_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);

        let original_requests = server.snapshot();
        assert_eq!(
            original_requests
                .requests
                .iter()
                .map(|request| request.kind.as_str())
                .collect::<Vec<_>>(),
            ["Industry", "Industry", "Concept"]
        );
        assert_eq!(original_requests.non_board_requests, 0);
        assert!(server.membership_snapshot().is_empty());
        assert_eq!(
            fixture.count("chain_post_close_chain_daily_applications"),
            1
        );
        assert_eq!(fixture.count("chain_post_close_board_directory_materials"), 1);
        assert_eq!(fixture.count("chain_post_close_board_selections"), 1);
        drop(queries);
        drop(source);

        fixture.execute(
            "CREATE TABLE stock_position ( \
               id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL, name TEXT NOT NULL, \
               buy_date TEXT NOT NULL, buy_price REAL NOT NULL, quantity INTEGER NOT NULL, \
               status TEXT NOT NULL, sell_date TEXT, sell_price REAL, return_rate REAL, \
               created_at TEXT NOT NULL, updated_at TEXT NOT NULL, chain_name TEXT, st_type TEXT, \
               UNIQUE(code,buy_date)); \
             INSERT INTO stock_position VALUES \
               (1,'TEST_CODE_CLUSTER_STOCK_A','成员','2026-07-21',10.0,100,'open',NULL,NULL,1.5, \
                '2026-07-21 09:01:02','2026-07-21 14:01:02','TEST_CODE原链',NULL), \
               (2,'TEST_CODE_CLUSTER_POS_ALIAS','别名','2026-07-20',20.0,200,'open',NULL,NULL,NULL, \
                '2026-07-20 09:02:03','2026-07-21 14:02:03',NULL,'ST');",
        );

        let before = OwnedDatabaseState::capture(fixture.connection());
        let legacy_fact_names = before
            .chain_names
            .iter()
            .filter(|name| {
                !matches!(
                    name.as_str(),
                    "chain_post_close_schema"
                        | "chain_post_close_layouts"
                        | "chain_post_close_layout_objects"
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let legacy_fact_rows = named_table_rows(fixture.connection(), &legacy_fact_names);
        let old_schema = all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_schema WHERE schema_version<=7 ORDER BY schema_version",
        );
        let old_layouts = all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layouts WHERE layout_version<=7 ORDER BY layout_version",
        );
        let old_registry = all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layout_objects WHERE layout_version<=7 \
             ORDER BY layout_version,name",
        );
        assert!(v8_objects(fixture.connection()).is_empty());

        let journal_mode: String = fixture
            .connection()
            .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
        fixture
            .connection()
            .busy_timeout(Duration::from_millis(250))
            .unwrap();
        let reader = Connection::open_with_flags(
            fixture.database(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        reader.busy_timeout(Duration::ZERO).unwrap();
        reader.execute_batch("BEGIN DEFERRED;").unwrap();
        assert_eq!(OwnedDatabaseState::capture(&reader), before);

        assert_eq!(
            fixture.chain_post_close().migrate_schema_v7_to_v8(),
            Err(ChainPostCloseError::StorageFailed {
                operation: "commit"
            })
        );
        assert!(fixture.connection().is_autocommit());
        assert_eq!(
            fixture
                .connection()
                .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(OwnedDatabaseState::capture(fixture.connection()), before);
        assert_eq!(OwnedDatabaseState::capture(&reader), before);
        assert!(v8_objects(fixture.connection()).is_empty());
        assert!(all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layouts WHERE layout_version=8"
        )
        .is_empty());
        assert!(all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layout_objects WHERE layout_version=8"
        )
        .is_empty());
        assert_eq!(server.snapshot(), original_requests);
        assert!(server.membership_snapshot().is_empty());

        reader.execute_batch("ROLLBACK;").unwrap();
        reader.close().unwrap();
        fixture.reopen();
        assert_eq!(
            fixture.chain_post_close().verify_schema().unwrap().schema_version(),
            7
        );
        assert_eq!(OwnedDatabaseState::capture(fixture.connection()), before);
        assert_eq!(server.snapshot(), original_requests);
        assert!(server.membership_snapshot().is_empty());

        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v7_to_v8()
                .unwrap()
                .schema_version(),
            8
        );
        assert_eq!(
            fixture.chain_post_close().verify_schema().unwrap().schema_version(),
            8
        );
        assert_eq!(
            named_table_rows(fixture.connection(), &legacy_fact_names),
            legacy_fact_rows
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_schema WHERE schema_version<=7 ORDER BY schema_version"
            ),
            old_schema
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_layouts WHERE layout_version<=7 ORDER BY layout_version"
            ),
            old_layouts
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT * FROM chain_post_close_layout_objects WHERE layout_version<=7 \
                 ORDER BY layout_version,name"
            ),
            old_registry
        );
        let migrated = OwnedDatabaseState::capture(fixture.connection());
        assert_eq!(migrated.audits, before.audits);
        assert_eq!(migrated.foundation, before.foundation);
        assert_eq!(migrated.foundation_catalog, before.foundation_catalog);
        assert_eq!(migrated.business, before.business);
        assert_eq!(migrated.database_identity, before.database_identity);
        assert!(before
            .defined_catalog
            .iter()
            .all(|row| migrated.defined_catalog.contains(row)));
        assert_eq!(
            migrated
                .defined_catalog
                .iter()
                .filter(|row| !before.defined_catalog.contains(row))
                .count(),
            V8_OBJECTS.len()
        );
        assert_eq!(v8_objects(fixture.connection()).len(), V8_OBJECTS.len());
        assert_eq!(server.snapshot(), original_requests);
        assert!(server.membership_snapshot().is_empty());

        fixture.reopen();
        assert_eq!(
            fixture.chain_post_close().verify_schema().unwrap().schema_version(),
            8
        );
        assert_eq!(OwnedDatabaseState::capture(fixture.connection()), migrated);
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        assert_eq!(local.inspect_run(&intent).unwrap().head_version(), original_head);
        drop(local);
        assert_eq!(OwnedDatabaseState::capture(fixture.connection()), migrated);
        assert_eq!(server.snapshot(), original_requests);
        assert!(server.membership_snapshot().is_empty());
        assert_eq!(server.finish().await, original_requests);
    })
    .await
    .expect("TEST_CODE v8 migration COMMIT rollback deadline");
}
