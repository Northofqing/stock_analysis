use super::*;

const CATALOG_SQL: &str =
    "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema ORDER BY name,type";

fn version_set(connection: &Connection) -> Vec<Vec<Value>> {
    all_rows(
        connection,
        "SELECT run_version FROM ( \
           SELECT run_version FROM chain_post_close_position_concept_rpc_occurrences \
           UNION ALL SELECT run_version FROM chain_post_close_position_concept_rpc_attempt_begins \
           UNION ALL SELECT run_version FROM chain_post_close_position_concept_rpc_attempt_results \
           UNION ALL SELECT run_version FROM chain_post_close_position_concept_rpc_status_materials \
           UNION ALL SELECT run_version FROM chain_post_close_position_concept_rpc_error_materials \
           UNION ALL SELECT run_version FROM chain_post_close_position_concept_rpc_finals \
           UNION ALL SELECT run_version FROM chain_post_close_position_concept_cache_writes \
         ) ORDER BY run_version",
    )
}

fn immutable_rpc_rows(connection: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    V9_TABLES[..5]
        .iter()
        .map(|table| {
            (
                (*table).to_owned(),
                all_rows(connection, &format!("SELECT * FROM \"{table}\"")),
            )
        })
        .collect()
}

fn final_content_rows(connection: &Connection) -> Vec<Vec<Value>> {
    all_rows(
        connection,
        "SELECT intent_id,cache_material_run_version,position_ordinal,code,provenance, \
                occurrence_run_version,occurrence_request_sha256,terminal_attempt_ordinal, \
                terminal_result_run_version,terminal_result_sha256,error_material_run_version, \
                error_material_sha256,final_outcome,final_codec_version,final_bytes,final_length, \
                final_sha256,audit_id,audit_record_hash,previous_outcome,current_outcome,run_id, \
                run_context_sha256,input_sha256,lease_owner,lease_generation,applied_at \
         FROM chain_post_close_position_concept_rpc_finals ORDER BY position_ordinal",
    )
}

fn cache_content_rows(connection: &Connection) -> Vec<Vec<Value>> {
    all_rows(
        connection,
        "SELECT intent_id,cache_material_run_version,position_ordinal,code,final_sha256, \
                terminal_result_run_version,concepts_codec_version,concepts_bytes,concepts_length, \
                concepts_sha256,cache_updated_at,run_id,run_context_sha256,input_sha256, \
                lease_owner,lease_generation,written_at \
         FROM chain_post_close_position_concept_cache_writes ORDER BY position_ordinal",
    )
}

#[tokio::test]
async fn position_concept_cache_rejects_a_history_written_before_every_final() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut fixture = V2BusinessFixture::new();
        fixture.install_v2();
        fixture.chain_post_close().migrate_schema_v2_to_v3().unwrap();
        cluster_tests::install_business_rows(&fixture);
        fixture.chain_post_close().migrate_schema_v3_to_v4().unwrap();
        install_br159(&mut fixture);
        fixture.chain_post_close().migrate_schema_v4_to_v5().unwrap();
        fixture.chain_post_close().migrate_schema_v5_to_v6().unwrap();
        fixture.chain_post_close().migrate_schema_v6_to_v7().unwrap();

        let (client, server) = spawn_membership_success_loopback().await;
        let source = GrpcSource::from_board_loopback_test_client(client);
        let queries = source.connected_board_queries().await.unwrap();
        let stocks = cluster_tests::cluster_stocks();
        let config = local_config(BUILD_A);
        let context = build_single_user_local_chain_post_close_context(
            &config,
            run_input("TEST_CODE_RUN_POSITION_CONCEPT_CACHE_BARRIER"),
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
        let v7_clock = ControlledClock::new(at(2_000_000));
        let mut io = local
            .concept_rpc_preparation_io_v7(
                lease,
                &queries,
                &v7_clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE v7 stops before positions");
        assert!(matches!(
            stopped.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::Positions
            })
        ));
        drop(io);
        let v7_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);
        let board_requests = server.snapshot();
        assert_eq!(board_requests.requests.len(), 3);
        assert_eq!(board_requests.non_board_requests, 0);
        assert!(server.membership_snapshot().is_empty());

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
               (2,'TEST_CODE_600001','缺失甲','2026-07-20',20.0,200,'open',NULL,NULL,NULL, \
                '2026-07-20 09:02:03','2026-07-21 14:02:03',NULL,'ST'), \
               (3,'TEST_CODE_600001','缺失乙','2026-07-19',21.0,300,'open',NULL,NULL,NULL, \
                '2026-07-19 09:03:04','2026-07-21 14:03:04',NULL,NULL), \
               (4,'TEST_CODE_CLUSTER_POS_OTHER','无关','2026-07-18',30.0,100,'open',NULL,NULL,-2.0, \
                '2026-07-18 09:04:05','2026-07-21 14:04:05',NULL,'*ST'), \
               (5,'TEST_CODE_CLUSTER_CLOSED','关闭','2026-07-22',40.0,100,'closed','2026-07-22',41.0,2.0, \
                '2026-07-22 09:05:06','2026-07-22 14:05:06',NULL,NULL); \
             INSERT INTO stock_concepts(code,concepts,updated_at) VALUES \
               ('TEST_CODE_CLUSTER_POS_OTHER','[\"TEST_CODE_CLUSTER_Z_OTHER\"]','2026-07-21 14:05:00');",
        );
        assert_eq!(
            fixture
                .connection()
                .query_row(
                    "SELECT count(*) FROM stock_position WHERE code=?1 AND status='open'",
                    [MISSING],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );

        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v7_to_v8()
                .unwrap()
                .schema_version(),
            8
        );
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(OWNER_B, 61_000_000, 360_000_000, Some(v7_head)),
            )
            .unwrap();
        let v8_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:33:00+08:00")).unwrap(),
            observation: Some((
                DateTime::parse_from_rfc3339("2026-07-21T15:33:00+08:00").unwrap(),
                DateTime::parse_from_rfc3339("2026-07-14T15:33:00+08:00").unwrap(),
            )),
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .positions_preparation_io_v8(
                lease,
                &queries,
                &v8_clock,
                FixedClusterConfiguration::resolve(Some("2")),
            )
            .unwrap();
        let stopped = prepare_chain_analysis_with_io(
            NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
            stocks.clone(),
            None,
            &mut io,
        )
        .await
        .expect_err("TEST_CODE v8 stops before position concept provider");
        assert!(matches!(
            stopped.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::PositionConceptProvider
            })
        ));
        assert_eq!(
            stopped
                .downcast_ref::<PreparationFailure>()
                .unwrap()
                .positions()
                .iter()
                .map(|position| position.code())
                .collect::<Vec<_>>(),
            [STOCK_A, MISSING, MISSING, POS_OTHER]
        );
        drop(io);
        assert_eq!(v8_clock.cache_calls.get(), 1);
        let v8_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);
        assert!(server.membership_snapshot().is_empty());
        assert_eq!(server.snapshot(), board_requests);

        let old_names = table_names(fixture.connection());
        let old_facts = old_fact_rows(fixture.connection(), &old_names);
        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v8_to_v9()
                .unwrap()
                .schema_version(),
            9
        );
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        let lease = local
            .resume_run(
                &intent,
                lease_request(OWNER_V9, 361_000_000, 660_000_000, Some(v8_head)),
            )
            .unwrap();
        let v9_clock = PositionsClock {
            now: UtcMicros::try_new(micros("2026-07-21T15:37:02+08:00")).unwrap(),
            observation: None,
            cache_calls: Cell::new(0),
        };
        let mut io = local
            .position_concept_rpc_preparation_io_v9(
                lease,
                &queries,
                &v9_clock,
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
        .expect_err("TEST_CODE v9 stops at DragonTiger");
        assert!(matches!(
            stopped.downcast_ref::<PreparationStop>(),
            Some(PreparationStop::StageNotMigrated {
                next: UnmigratedStage::DragonTiger
            })
        ));
        assert_eq!(
            stopped
                .downcast_ref::<PreparationFailure>()
                .unwrap()
                .completed_stages(),
            [
                PreparationStage::Concepts,
                PreparationStage::ClusterWritesAndLifecycle,
                PreparationStage::Candidates,
                PreparationStage::Positions,
                PreparationStage::PositionConcepts,
            ]
        );
        drop(io);
        assert_eq!(v9_clock.cache_calls.get(), 0);
        let healthy_head = local.inspect_run(&intent).unwrap().head_version();
        drop(local);
        fixture.chain_post_close().verify_schema().unwrap();

        let memberships = server.membership_snapshot();
        assert_eq!(memberships.len(), 2);
        assert!(memberships.iter().all(|request| request.codes == [MISSING]));
        assert_eq!(
            memberships
                .iter()
                .map(|request| request.request_id.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            2
        );
        assert_eq!(server.snapshot(), board_requests);
        let scopes = all_rows(
            fixture.connection(),
            "SELECT occurrence.position_ordinal,occurrence.code,begin.attempt_ordinal, \
                    result.continuation,final.final_outcome,cache.code \
             FROM chain_post_close_position_concept_rpc_occurrences AS occurrence \
             JOIN chain_post_close_position_concept_rpc_attempt_begins AS begin \
               USING(intent_id,cache_material_run_version,position_ordinal) \
             JOIN chain_post_close_position_concept_rpc_attempt_results AS result \
               USING(intent_id,cache_material_run_version,position_ordinal,attempt_ordinal) \
             JOIN chain_post_close_position_concept_rpc_finals AS final \
               USING(intent_id,cache_material_run_version,position_ordinal) \
             JOIN chain_post_close_position_concept_cache_writes AS cache \
               USING(intent_id,cache_material_run_version,position_ordinal) \
             ORDER BY occurrence.position_ordinal",
        );
        assert_eq!(
            scopes,
            [1_i64, 2]
                .into_iter()
                .map(|ordinal| {
                    vec![
                        Value::Integer(ordinal),
                        Value::Text(MISSING.to_owned()),
                        Value::Integer(1),
                        Value::Text("Terminal".to_owned()),
                        Value::Text("Available".to_owned()),
                        Value::Text(MISSING.to_owned()),
                    ]
                })
                .collect::<Vec<_>>()
        );
        assert!(all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_position_concept_rpc_status_materials",
        )
        .is_empty());
        assert!(all_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_position_concept_rpc_error_materials",
        )
        .is_empty());

        let finals = all_rows(
            fixture.connection(),
            "SELECT position_ordinal,terminal_result_run_version,run_version \
             FROM chain_post_close_position_concept_rpc_finals ORDER BY run_version",
        );
        let caches = all_rows(
            fixture.connection(),
            "SELECT position_ordinal,terminal_result_run_version,final_run_version,run_version \
             FROM chain_post_close_position_concept_cache_writes ORDER BY run_version",
        );
        assert_eq!((finals.len(), caches.len()), (2, 2));
        let last_final_ordinal = integer(&finals[1][0]);
        let last_final_run = integer(&finals[1][2]);
        let first_cache_ordinal = integer(&caches[0][0]);
        let first_cache_parent = integer(&caches[0][2]);
        let first_cache_run = integer(&caches[0][3]);
        assert_ne!(last_final_ordinal, first_cache_ordinal);
        assert!(first_cache_parent < first_cache_run);
        assert!(finals.iter().all(|row| integer(&row[2]) < first_cache_run));
        assert!(last_final_run < first_cache_run);

        let healthy_versions = version_set(fixture.connection());
        let healthy_catalog = all_rows(fixture.connection(), CATALOG_SQL);
        let healthy_old_facts = old_fact_rows(fixture.connection(), &old_names);
        assert_eq!(healthy_old_facts, old_facts);
        let healthy_rpc = immutable_rpc_rows(fixture.connection());
        let healthy_final_content = final_content_rows(fixture.connection());
        let healthy_cache_content = cache_content_rows(fixture.connection());
        let healthy_audits = (
            all_rows(
                fixture.connection(),
                "SELECT * FROM data_acquisition_audit ORDER BY id",
            ),
            all_rows(
                fixture.connection(),
                "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id",
            ),
        );
        let healthy_sources = (
            all_rows(fixture.connection(), "SELECT * FROM stock_position ORDER BY id"),
            all_rows(
                fixture.connection(),
                "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code",
            ),
        );
        let final_trigger: String = fixture
            .connection()
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='trigger' \
                 AND name='chain_post_close_position_concept_rpc_finals_update'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let cache_trigger: String = fixture
            .connection()
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='trigger' \
                 AND name='chain_post_close_position_concept_cache_writes_update'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        fixture.execute(
            "PRAGMA foreign_keys=OFF; \
             DROP TRIGGER chain_post_close_position_concept_rpc_finals_update; \
             DROP TRIGGER chain_post_close_position_concept_cache_writes_update;",
        );
        fixture
            .connection()
            .execute(
                "UPDATE chain_post_close_position_concept_rpc_finals \
                 SET prior_head_version=?1,run_version=?2 WHERE position_ordinal=?3",
                rusqlite::params![first_cache_run - 1, first_cache_run, last_final_ordinal],
            )
            .unwrap();
        fixture
            .connection()
            .execute(
                "UPDATE chain_post_close_position_concept_cache_writes \
                 SET prior_head_version=?1,run_version=?2 WHERE position_ordinal=?3",
                rusqlite::params![last_final_run - 1, last_final_run, first_cache_ordinal],
            )
            .unwrap();
        assert_eq!(
            fixture
                .connection()
                .execute(
                    "UPDATE chain_post_close_position_concept_cache_writes \
                     SET final_run_version=?1 WHERE position_ordinal=?2",
                    rusqlite::params![first_cache_run, last_final_ordinal],
                )
                .unwrap(),
            1
        );
        fixture.execute(&final_trigger);
        fixture.execute(&cache_trigger);
        fixture.execute("PRAGMA foreign_keys=ON;");

        assert!(all_rows(fixture.connection(), "PRAGMA foreign_key_check").is_empty());
        assert_eq!(all_rows(fixture.connection(), CATALOG_SQL), healthy_catalog);
        assert_eq!(version_set(fixture.connection()), healthy_versions);
        assert_eq!(
            fixture.connection().query_row(
                "SELECT head_version FROM chain_post_close_runs WHERE intent_id=?1",
                [intent.as_str()],
                |row| row.get::<_, i64>(0),
            ).unwrap(),
            i64::try_from(healthy_head).unwrap()
        );
        assert_eq!(immutable_rpc_rows(fixture.connection()), healthy_rpc);
        assert_eq!(final_content_rows(fixture.connection()), healthy_final_content);
        assert_eq!(cache_content_rows(fixture.connection()), healthy_cache_content);
        assert_eq!(old_fact_rows(fixture.connection(), &old_names), healthy_old_facts);
        assert_eq!(
            (
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit ORDER BY id"),
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"),
            ),
            healthy_audits
        );
        assert_eq!(
            (
                all_rows(fixture.connection(), "SELECT * FROM stock_position ORDER BY id"),
                all_rows(fixture.connection(), "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code"),
            ),
            healthy_sources
        );
        assert_eq!(
            all_rows(
                fixture.connection(),
                "SELECT final.position_ordinal,final.run_version,cache.run_version \
                 FROM chain_post_close_position_concept_cache_writes AS cache \
                 JOIN chain_post_close_position_concept_rpc_finals AS final \
                   ON final.intent_id=cache.intent_id \
                  AND final.cache_material_run_version=cache.cache_material_run_version \
                  AND final.position_ordinal=cache.position_ordinal \
                  AND final.run_version=cache.final_run_version \
                  AND final.final_sha256=cache.final_sha256 \
                 ORDER BY cache.run_version LIMIT 1",
            ),
            vec![vec![
                Value::Integer(first_cache_ordinal),
                Value::Integer(first_cache_parent),
                Value::Integer(last_final_run),
            ]]
        );
        assert_eq!(
            fixture.connection().query_row(
                "SELECT min(run_version) FROM chain_post_close_position_concept_cache_writes",
                [],
                |row| row.get::<_, i64>(0),
            ).unwrap(),
            last_final_run
        );
        assert_eq!(
            fixture.connection().query_row(
                "SELECT max(run_version) FROM chain_post_close_position_concept_rpc_finals",
                [],
                |row| row.get::<_, i64>(0),
            ).unwrap(),
            first_cache_run
        );
        assert!(last_final_run < first_cache_run);
        assert_eq!(
            fixture.connection().query_row(
                "SELECT count(*) FROM chain_post_close_position_concept_cache_writes AS cache \
                 JOIN chain_post_close_position_concept_rpc_finals AS final \
                   ON final.intent_id=cache.intent_id \
                  AND final.cache_material_run_version=cache.cache_material_run_version \
                  AND final.position_ordinal=cache.position_ordinal \
                  AND final.run_version=cache.final_run_version \
                 WHERE cache.run_version < ( \
                   SELECT max(run_version) FROM chain_post_close_position_concept_rpc_finals)",
                [],
                |row| row.get::<_, i64>(0),
            ).unwrap(),
            1
        );

        let damaged_facts = v9_fact_rows(fixture.connection());
        let damaged_audits = healthy_audits.clone();
        let damaged_sources = healthy_sources.clone();
        let first_rejection = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .and_then(|mut local| local.inspect_run(&intent));
        assert_eq!(v9_fact_rows(fixture.connection()), damaged_facts);
        assert_eq!(server.membership_snapshot(), memberships);
        assert_eq!(server.snapshot(), board_requests);
        assert_eq!(
            (
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit ORDER BY id"),
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"),
            ),
            damaged_audits
        );
        assert_eq!(
            (
                all_rows(fixture.connection(), "SELECT * FROM stock_position ORDER BY id"),
                all_rows(fixture.connection(), "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code"),
            ),
            damaged_sources
        );

        fixture.reopen();
        let reopened_rejection = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .and_then(|mut local| local.inspect_run(&intent));
        assert_eq!(v9_fact_rows(fixture.connection()), damaged_facts);
        assert_eq!(old_fact_rows(fixture.connection(), &old_names), healthy_old_facts);
        assert_eq!(version_set(fixture.connection()), healthy_versions);
        assert_eq!(all_rows(fixture.connection(), CATALOG_SQL), healthy_catalog);
        assert!(all_rows(fixture.connection(), "PRAGMA foreign_key_check").is_empty());
        assert_eq!(
            fixture.connection().query_row(
                "SELECT head_version FROM chain_post_close_runs WHERE intent_id=?1",
                [intent.as_str()],
                |row| row.get::<_, i64>(0),
            ).unwrap(),
            i64::try_from(healthy_head).unwrap()
        );
        assert_eq!(
            (
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit ORDER BY id"),
                all_rows(fixture.connection(), "SELECT * FROM data_acquisition_audit_chain ORDER BY acquisition_audit_id"),
            ),
            damaged_audits
        );
        assert_eq!(
            (
                all_rows(fixture.connection(), "SELECT * FROM stock_position ORDER BY id"),
                all_rows(fixture.connection(), "SELECT code,concepts,CAST(updated_at AS TEXT) FROM stock_concepts ORDER BY code"),
            ),
            damaged_sources
        );
        assert_eq!(server.membership_snapshot(), memberships);
        assert_eq!(server.snapshot(), board_requests);
        drop(queries);
        drop(source);
        assert_eq!(server.finish().await, board_requests);

        assert!(matches!(
            first_rejection,
            Err(ChainPostCloseError::SchemaRejected)
        ));
        assert!(matches!(
            reopened_rejection,
            Err(ChainPostCloseError::SchemaRejected)
        ));
    })
    .await
    .expect("TEST_CODE cache history barrier deadline");
}
