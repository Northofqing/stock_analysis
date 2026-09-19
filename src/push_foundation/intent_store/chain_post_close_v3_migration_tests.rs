use std::cell::RefCell;

use rusqlite::types::Value;

use super::*;

#[path = "chain_post_close_v4_migration_tests.rs"]
mod v4_migration_tests;

const CONFIRMED_CODE: &str = "TEST_CODE_MIGRATION_CONFIRMED";
const UNCONFIRMED_CODE: &str = "TEST_CODE_MIGRATION_UNCONFIRMED";
const ORIGINAL_RAW: &str = "  {\"all_boards\":[\"测试板块\",\"TEST_CODE_ORIGINAL\"]} \n";

fn install_v3(fixture: &mut V2BusinessFixture) {
    fixture.install_v2();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v2_to_v3()
            .unwrap()
            .schema_version(),
        3
    );
}

fn captured(date: &str) -> (String, UtcMicros) {
    let text = format!("{date}T15:31:00+08:00");
    let micros = DateTime::parse_from_rfc3339(&text)
        .unwrap()
        .timestamp_micros();
    (text, UtcMicros::try_new(micros).unwrap())
}

fn dated_stocks(code: &str) -> Vec<TopStock> {
    vec![TopStock {
        code: code.to_owned(),
        name: format!("TEST_CODE_NAME_{code}"),
        change_pct: 9.91,
        price: 31.25,
        ..TopStock::default()
    }]
}

fn dated_source(date: &str, status: SourceStatus, batch: &str) -> SourceObservation {
    let (observed_at, _) = captured(date);
    SourceObservation::from_batch_for_request(
        status,
        BatchEvidence {
            provider: ProviderId::Custom,
            source: format!("TEST_CODE_MIGRATION_SOURCE_{date}"),
            source_at: Some(format!("{date}T15:30:00+08:00")),
            observed_at: observed_at.clone(),
            batch_id: batch.to_owned(),
        },
        NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
        observed_at,
    )
    .unwrap()
}

fn dated_input(date: &str, code: &str) -> FixedChainPreparationInput {
    FixedChainPreparationInput::try_new(
        BusinessDate::parse(date).unwrap(),
        dated_stocks(code),
        None,
        dated_source(
            date,
            SourceStatus::Available,
            "TEST_CODE_LIMIT_UP_MIGRATION",
        ),
        dated_source(
            date,
            SourceStatus::VerifiedEmpty,
            "TEST_CODE_MACRO_MIGRATION",
        ),
    )
    .unwrap()
}

fn dated_context(
    config: &LocalChainPostCloseConfig,
    date: &str,
    run_id: &str,
) -> crate::monitor::push_job::LocalChainPostCloseContext {
    let (_, captured_at) = captured(date);
    build_single_user_local_chain_post_close_context(
        config,
        LocalChainPostCloseRunInput::try_new(
            RunId::try_new(run_id.to_owned()).unwrap(),
            CalendarDate::parse(date).unwrap(),
            BusinessDate::parse(date).unwrap(),
            captured_at,
        )
        .unwrap(),
    )
    .unwrap()
}

fn dated_lease(
    date: &str,
    owner: &str,
    now: i64,
    until: i64,
    head: Option<u64>,
) -> RunLeaseRequest {
    let (_, captured_at) = captured(date);
    RunLeaseRequest::try_new(
        LeaseOwnerId::try_new(owner.to_owned()).unwrap(),
        UtcMicros::try_new(captured_at.get() + now).unwrap(),
        UtcMicros::try_new(captured_at.get() + until).unwrap(),
        head,
    )
    .unwrap()
}

fn rows(connection: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = connection.prepare(sql).unwrap();
    let columns = statement.column_count();
    let rows = statement
        .query_map([], |row| {
            (0..columns)
                .map(|column| row.get::<_, Value>(column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows
}

#[derive(Debug, PartialEq)]
struct OldV2State {
    runs: Vec<Vec<Value>>,
    begins: Vec<Vec<Value>>,
    results: Vec<Vec<Value>>,
    v1_headers: Vec<Vec<Value>>,
    v1_registry: Vec<Vec<Value>>,
    v2_header: Vec<Vec<Value>>,
    v2_registry: Vec<Vec<Value>>,
    foundation_schema: Vec<Vec<Value>>,
    foundation_objects: Vec<Vec<Value>>,
    cache: Vec<Vec<Value>>,
    application_id: i64,
    user_version: i64,
}

fn old_v2_state(fixture: &V2BusinessFixture) -> OldV2State {
    let connection = fixture.connection();
    OldV2State {
        runs: rows(
            connection,
            "SELECT * FROM chain_post_close_runs ORDER BY intent_id",
        ),
        begins: rows(
            connection,
            "SELECT * FROM chain_post_close_stage_begins ORDER BY intent_id,effect_ordinal",
        ),
        results: rows(
            connection,
            "SELECT * FROM chain_post_close_stage_results ORDER BY intent_id,effect_ordinal",
        ),
        v1_headers: rows(
            connection,
            "SELECT * FROM chain_post_close_schema ORDER BY schema_version",
        ),
        v1_registry: rows(
            connection,
            "SELECT * FROM chain_post_close_objects ORDER BY name",
        ),
        v2_header: rows(
            connection,
            "SELECT * FROM chain_post_close_layouts WHERE layout_version=2",
        ),
        v2_registry: rows(
            connection,
            "SELECT * FROM chain_post_close_layout_objects \
             WHERE layout_version=2 ORDER BY name",
        ),
        foundation_schema: rows(
            connection,
            "SELECT * FROM push_foundation_schema ORDER BY version",
        ),
        foundation_objects: rows(
            connection,
            "SELECT * FROM push_foundation_objects ORDER BY name",
        ),
        cache: rows(connection, "SELECT * FROM stock_concepts ORDER BY code"),
        application_id: connection
            .query_row("PRAGMA application_id", [], |row| row.get(0))
            .unwrap(),
        user_version: connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap(),
    }
}

struct V2Facts {
    fixture: V2BusinessFixture,
    config: LocalChainPostCloseConfig,
    confirmed: IntentId,
    unconfirmed: IntentId,
}

fn v2_facts_fixture() -> V2Facts {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema_v2_reader()
            .unwrap()
            .schema_version(),
        2
    );
    let config = local_config(BUILD_A);

    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            dated_context(&config, "2026-07-21", "TEST_CODE_MIGRATION_RUN_CONFIRMED"),
            dated_input("2026-07-21", CONFIRMED_CODE),
            dated_lease(
                "2026-07-21",
                "TEST_CODE_MIGRATION_OWNER_CONFIRMED",
                1_000,
                8_000,
                None,
            ),
        )
        .unwrap();
    let confirmed = lease.intent_id().clone();
    let (lease, admission) = local
        .begin_concept_provider(
            lease,
            ConceptProviderRequest::try_new(0, CONFIRMED_CODE.to_owned()).unwrap(),
            UtcMicros::try_new(captured("2026-07-21").1.get() + 1_100).unwrap(),
        )
        .unwrap();
    let ConceptProviderAdmission::Call(call) = admission else {
        panic!("new v2 fact cannot replay");
    };
    local
        .record_concept_provider_result(
            lease,
            call,
            ConceptProviderRawResult::returned(ORIGINAL_RAW.to_owned()),
            UtcMicros::try_new(captured("2026-07-21").1.get() + 1_200).unwrap(),
        )
        .unwrap();
    drop(local);

    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .acquire_run(
            dated_context(&config, "2026-07-22", "TEST_CODE_MIGRATION_RUN_UNCONFIRMED"),
            dated_input("2026-07-22", UNCONFIRMED_CODE),
            dated_lease(
                "2026-07-22",
                "TEST_CODE_MIGRATION_OWNER_UNCONFIRMED",
                1_000,
                8_000,
                None,
            ),
        )
        .unwrap();
    let unconfirmed = lease.intent_id().clone();
    let (_, admission) = local
        .begin_concept_provider(
            lease,
            ConceptProviderRequest::try_new(0, UNCONFIRMED_CODE.to_owned()).unwrap(),
            UtcMicros::try_new(captured("2026-07-22").1.get() + 1_100).unwrap(),
        )
        .unwrap();
    assert!(matches!(admission, ConceptProviderAdmission::Call(_)));
    drop(local);
    V2Facts {
        fixture,
        config,
        confirmed,
        unconfirmed,
    }
}

struct PanicProvider {
    calls: RefCell<Vec<String>>,
}

#[async_trait::async_trait(?Send)]
impl ConceptProviderRawIo for PanicProvider {
    async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
        self.calls.borrow_mut().push(code.to_owned());
        panic!("migration recovery must not call provider")
    }
}

async fn run_recovery(
    local: &mut super::super::LocalChainPostClose<'_>,
    lease: super::super::RunLease,
    provider: &PanicProvider,
    date: &str,
    code: &str,
) -> anyhow::Error {
    let clock = ControlledClock::new(captured(date).1.get() + 8_100);
    let mut io = local
        .concept_batch_preparation_io(lease, provider, &clock)
        .unwrap();
    prepare_chain_analysis_with_io(
        NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
        dated_stocks(code),
        None,
        &mut io,
    )
    .await
    .expect_err("local recovery must stop at its typed boundary")
}

#[tokio::test]
async fn v2_to_v3_preserves_two_real_old_runs_and_recovers_without_provider() {
    let mut facts = v2_facts_fixture();
    let before = old_v2_state(&facts.fixture);
    let before_catalog = full_catalog(facts.fixture.connection());
    assert_eq!(before.runs.len(), 2);
    assert_eq!(before.begins.len(), 2);
    assert_eq!(before.results.len(), 1);
    let receipt = facts
        .fixture
        .chain_post_close()
        .migrate_schema_v2_to_v3()
        .unwrap();
    assert_eq!(receipt.schema_version(), 3);
    assert_eq!(old_v2_state(&facts.fixture), before);
    let after_catalog = full_catalog(facts.fixture.connection());
    assert!(catalog_additions(&before_catalog, &after_catalog).is_empty());
    assert_eq!(
        catalog_additions(&after_catalog, &before_catalog),
        fixed_v3_reference_additions()
    );
    assert_eq!(facts.fixture.count("chain_post_close_objects"), 8);
    assert_eq!(
        facts
            .fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version=2",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        27
    );
    assert_eq!(
        facts
            .fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version=3",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        31
    );
    assert_eq!(
        facts.fixture.count("chain_post_close_concept_cache_writes"),
        0
    );
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        3
    );
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema_v2_reader()
            .err(),
        Some(ChainPostCloseError::SchemaRejected)
    );

    facts.fixture.reopen();
    assert_eq!(old_v2_state(&facts.fixture), before);
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        3
    );
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema_v2_reader()
            .err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    let mut local = facts
        .fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&facts.config)
        .unwrap();
    let confirmed_recovery = local.inspect_run(&facts.confirmed).unwrap();
    assert_eq!(
        confirmed_recovery.results()[0].bytes,
        ORIGINAL_RAW.as_bytes()
    );
    let lease = local
        .resume_run(
            &facts.confirmed,
            dated_lease(
                "2026-07-21",
                "TEST_CODE_MIGRATION_RECOVERY_CONFIRMED",
                8_001,
                12_000,
                Some(confirmed_recovery.head_version()),
            ),
        )
        .unwrap();
    let provider = PanicProvider {
        calls: RefCell::new(Vec::new()),
    };
    let error = run_recovery(&mut local, lease, &provider, "2026-07-21", CONFIRMED_CODE).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::ClusterConfiguration,
        })
    ));
    assert!(provider.calls.borrow().is_empty());

    let unconfirmed_recovery = local.inspect_run(&facts.unconfirmed).unwrap();
    let lease = local
        .resume_run(
            &facts.unconfirmed,
            dated_lease(
                "2026-07-22",
                "TEST_CODE_MIGRATION_RECOVERY_UNCONFIRMED",
                8_001,
                12_000,
                Some(unconfirmed_recovery.head_version()),
            ),
        )
        .unwrap();
    let error = run_recovery(&mut local, lease, &provider, "2026-07-22", UNCONFIRMED_CODE).await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { .. })
    ));
    assert!(provider.calls.borrow().is_empty());
}

#[test]
fn v2_to_v3_real_commit_failure_rolls_back_every_new_object_and_row() {
    let mut facts = v2_facts_fixture();
    let before = old_v2_state(&facts.fixture);
    let before_layout = layout_catalog_state(&facts.fixture);
    let database = facts.fixture.database();
    let journal_mode: String = facts
        .fixture
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode, "delete");
    let reader = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    reader.execute_batch("BEGIN DEFERRED;").unwrap();
    assert_eq!(
        reader
            .query_row("SELECT count(*) FROM chain_post_close_layouts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    assert_eq!(
        facts.fixture.chain_post_close().migrate_schema_v2_to_v3(),
        Err(ChainPostCloseError::StorageFailed {
            operation: "commit"
        })
    );
    assert!(facts.fixture.connection().is_autocommit());
    assert_eq!(
        facts
            .fixture
            .connection()
            .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(old_v2_state(&facts.fixture), before);
    assert_eq!(layout_catalog_state(&facts.fixture), before_layout);
    assert_eq!(
        facts.fixture.connection().query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name GLOB 'chain_post_close_concept_cache_writes*'",
            [],
            |row| row.get::<_, i64>(0),
        ).unwrap(),
        0
    );
    assert_eq!(
        facts
            .fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version=3",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    reader.execute_batch("ROLLBACK;").unwrap();
    reader.close().unwrap();

    facts.fixture.reopen();
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        2
    );
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema_v2_reader()
            .unwrap()
            .schema_version(),
        2
    );
    assert_eq!(old_v2_state(&facts.fixture), before);
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .migrate_schema_v2_to_v3()
            .unwrap()
            .schema_version(),
        3
    );
}

fn catalog(connection: &Connection) -> Vec<(String, String)> {
    let mut statement = connection
        .prepare(
            "SELECT name,type FROM sqlite_schema \
             WHERE lower(name) GLOB 'chain_post_close_*' \
               AND NOT(type='index' AND name GLOB 'sqlite_autoindex_*' AND sql IS NULL) \
             ORDER BY name",
        )
        .unwrap();
    let catalog = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    catalog
}

fn full_catalog(connection: &Connection) -> Vec<Vec<Value>> {
    rows(
        connection,
        "SELECT name,type,tbl_name,CAST(sql AS BLOB) FROM sqlite_schema ORDER BY name,type",
    )
}

fn catalog_additions(after: &[Vec<Value>], before: &[Vec<Value>]) -> Vec<Vec<Value>> {
    after
        .iter()
        .filter(|row| !before.contains(row))
        .cloned()
        .collect()
}

fn fixed_v3_reference_additions() -> Vec<Vec<Value>> {
    let reference = Connection::open_in_memory().unwrap();
    reference
        .execute_batch(include_str!("chain_post_close.v1.sql"))
        .unwrap();
    reference
        .execute_batch(include_str!("chain_post_close.v2.sql"))
        .unwrap();
    let before = full_catalog(&reference);
    reference
        .execute_batch(include_str!("chain_post_close.v3.sql"))
        .unwrap();
    catalog_additions(&full_catalog(&reference), &before)
}

#[derive(Debug, PartialEq)]
struct LayoutCatalogState {
    catalog: Vec<Vec<Value>>,
    headers: Vec<Vec<Value>>,
    registry: Vec<Vec<Value>>,
}

fn layout_catalog_state(fixture: &V2BusinessFixture) -> LayoutCatalogState {
    LayoutCatalogState {
        catalog: full_catalog(fixture.connection()),
        headers: rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layouts ORDER BY layout_version",
        ),
        registry: rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        ),
    }
}

#[test]
fn frozen_sql_reference_has_exact_v1_v2_v3_catalog_sizes_and_v3_names() {
    let reference = Connection::open_in_memory().unwrap();
    let v1 = include_str!("chain_post_close.v1.sql");
    let v2 = include_str!("chain_post_close.v2.sql");
    let v3 = include_str!("chain_post_close.v3.sql");
    reference.execute_batch(v1).unwrap();
    assert_eq!(catalog(&reference).len(), 8);
    reference.execute_batch(v2).unwrap();
    assert_eq!(catalog(&reference).len(), 27);
    reference.execute_batch(v3).unwrap();
    let catalog = catalog(&reference);
    assert_eq!(catalog.len(), 31);
    for name in [
        "chain_post_close_concept_cache_writes",
        "chain_post_close_concept_cache_writes_guard",
        "chain_post_close_concept_cache_writes_update",
        "chain_post_close_concept_cache_writes_delete",
    ] {
        assert!(catalog.iter().any(|entry| entry.0 == name));
    }
    assert_eq!(
        raw_digest(v2.as_bytes()).as_str(),
        "39280ac92da2068c37484cab83124f5bd6de8d220cbfe5fbd2f1427fe15b5608"
    );
    assert_eq!(
        raw_digest(v3.as_bytes()).as_str(),
        "165dbf8bbae2458d6616d6973722e4b056412f6777a9690a670878541cb18f75"
    );
}

#[test]
fn v3_reader_rejects_foreign_attachments_and_v2_migration_rejects_pollution() {
    for (kind, sql) in [
        (
            "index",
            "CREATE INDEX TEST_CODE_FOREIGN_CACHE_INDEX \
             ON chain_post_close_concept_cache_writes(code);",
        ),
        (
            "trigger",
            "CREATE TRIGGER TEST_CODE_FOREIGN_CACHE_TRIGGER \
             BEFORE INSERT ON chain_post_close_concept_cache_writes \
             BEGIN SELECT 1; END;",
        ),
    ] {
        let mut v3 = V2BusinessFixture::new();
        install_v3(&mut v3);
        v3.execute(sql);
        let damaged = layout_catalog_state(&v3);
        assert_eq!(
            v3.chain_post_close().verify_schema().err(),
            Some(ChainPostCloseError::SchemaRejected),
            "foreign {kind} must be rejected"
        );
        assert_eq!(layout_catalog_state(&v3), damaged);
        v3.reopen();
        assert_eq!(
            v3.chain_post_close().verify_schema().err(),
            Some(ChainPostCloseError::SchemaRejected),
            "foreign {kind} must remain rejected after reopen"
        );
        assert_eq!(layout_catalog_state(&v3), damaged);
    }

    let mut v2 = V2BusinessFixture::new();
    v2.install_v2();
    v2.execute("CREATE INDEX TEST_CODE_FOREIGN_RUN_INDEX ON chain_post_close_runs(run_id);");
    let damaged = layout_catalog_state(&v2);
    assert_eq!(
        v2.chain_post_close().migrate_schema_v2_to_v3().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&v2), damaged);
    v2.reopen();
    assert_eq!(
        v2.chain_post_close().migrate_schema_v2_to_v3().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&v2), damaged);
}

fn remove_layout_header_behind_guard(fixture: &V2BusinessFixture, version: i64) {
    let definition: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_layouts_delete'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    fixture.execute("PRAGMA foreign_keys=OFF; DROP TRIGGER chain_post_close_layouts_delete;");
    fixture
        .connection()
        .execute(
            "DELETE FROM chain_post_close_layouts WHERE layout_version=?1",
            [version],
        )
        .unwrap();
    fixture.execute(&definition);
    fixture.execute("PRAGMA foreign_keys=ON;");
}

#[test]
fn readers_reject_future_orphan_and_unsealed_layout_metadata_without_repair() {
    let mut future = V2BusinessFixture::new();
    install_v3(&mut future);
    future.execute("BEGIN IMMEDIATE;");
    future.execute(
        "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
         SELECT 4,name,object_type,definition FROM chain_post_close_layout_objects \
         WHERE layout_version=3;",
    );
    future.execute(
        "INSERT INTO chain_post_close_layouts( \
           layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
           artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
         ) VALUES(4,3,'165dbf8bbae2458d6616d6973722e4b056412f6777a9690a670878541cb18f75', \
                  1,1,1,'TEST_CODE_FUTURE_LAYOUT', \
                  'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'); \
         COMMIT;",
    );
    let future_metadata = layout_catalog_state(&future);
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&future), future_metadata);
    future.reopen();
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&future), future_metadata);
    future.execute("BEGIN IMMEDIATE;");
    future.execute(
        "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
         SELECT 5,name,object_type,definition FROM chain_post_close_layout_objects \
         WHERE layout_version=4;",
    );
    future.execute(
        "INSERT INTO chain_post_close_layouts( \
           layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
           artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
         ) VALUES(5,4,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', \
                  1,1,1,'TEST_CODE_FUTURE_LAYOUT_5', \
                  'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'); \
         COMMIT;",
    );
    let future_five_metadata = layout_catalog_state(&future);
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_five_metadata);
    future.reopen();
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_five_metadata);
    future.execute("BEGIN IMMEDIATE;");
    future.execute(
        "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
         SELECT 6,name,object_type,definition FROM chain_post_close_layout_objects \
         WHERE layout_version=5;",
    );
    future.execute(
        "INSERT INTO chain_post_close_layouts( \
           layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
           artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
         ) VALUES(6,5,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', \
                  1,1,1,'TEST_CODE_FUTURE_LAYOUT_6', \
                  'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'); \
         COMMIT;",
    );
    let future_six_metadata = layout_catalog_state(&future);
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_six_metadata);
    future.reopen();
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_six_metadata);
    future.execute("BEGIN IMMEDIATE;");
    future.execute(
        "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
         SELECT 7,name,object_type,definition FROM chain_post_close_layout_objects \
         WHERE layout_version=6;",
    );
    future.execute(
        "INSERT INTO chain_post_close_layouts( \
           layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
           artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
         ) VALUES(7,6,'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc', \
                  1,1,1,'TEST_CODE_FUTURE_LAYOUT_7', \
                  'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'); \
         COMMIT;",
    );
    let future_seven_metadata = layout_catalog_state(&future);
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_seven_metadata);
    future.reopen();
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_seven_metadata);

    future.execute("BEGIN IMMEDIATE;");
    future.execute(
        "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
         SELECT 8,name,object_type,definition FROM chain_post_close_layout_objects \
         WHERE layout_version=7;",
    );
    future.execute(
        "INSERT INTO chain_post_close_layouts( \
           layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
           artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
         ) VALUES(8,7,'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd', \
                  1,1,1,'TEST_CODE_FUTURE_LAYOUT_8', \
                  'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee'); \
         COMMIT;",
    );
    let future_eight_metadata = layout_catalog_state(&future);
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_eight_metadata);
    future.reopen();
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_eight_metadata);

    future.execute("BEGIN IMMEDIATE;");
    future.execute(
        "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
         SELECT 9,name,object_type,definition FROM chain_post_close_layout_objects \
         WHERE layout_version=8;",
    );
    future.execute(
        "INSERT INTO chain_post_close_layouts( \
           layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
           artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
         ) VALUES(9,8,'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee', \
                  1,1,1,'TEST_CODE_FUTURE_LAYOUT_9', \
                  'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'); \
         COMMIT;",
    );
    let future_nine_metadata = layout_catalog_state(&future);
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_nine_metadata);
    future.reopen();
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_nine_metadata);

    future.execute("BEGIN IMMEDIATE;");
    future.execute(
        "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
         SELECT 10,name,object_type,definition FROM chain_post_close_layout_objects \
         WHERE layout_version=9;",
    );
    future.execute(
        "INSERT INTO chain_post_close_layouts( \
           layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
           artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
         ) VALUES(10,9,'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff', \
                  1,1,1,'TEST_CODE_FUTURE_LAYOUT_10', \
                  '0000000000000000000000000000000000000000000000000000000000000000'); \
         COMMIT;",
    );
    let future_ten_metadata = layout_catalog_state(&future);
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&future), future_ten_metadata);
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_ten_metadata);
    future.reopen();
    assert_eq!(
        future.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&future), future_ten_metadata);
    assert_eq!(
        future.chain_post_close().verify_schema_v3_reader().err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    assert_eq!(layout_catalog_state(&future), future_ten_metadata);

    // Every layout this build knows is verified by its own bundle and refused as
    // SchemaRejected; the first layout it does not know is UnsupportedVersion.
    let current_layout =
        crate::push_foundation::intent_store::chain_post_close::schema::CURRENT_LAYOUT;
    let mut predecessor_digest = "0".repeat(64);
    for layout in 11..=current_layout + 1 {
        let digest = format!("{layout:02x}").repeat(32);
        future.execute("BEGIN IMMEDIATE;");
        future.execute(&format!(
            "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
             SELECT {layout},name,object_type,definition FROM chain_post_close_layout_objects \
             WHERE layout_version={};",
            layout - 1
        ));
        future.execute(&format!(
            "INSERT INTO chain_post_close_layouts( \
               layout_version,predecessor_layout_version,predecessor_bundle_sha256, \
               artifact_codec_version,input_codec_version,stage_codec_version,description,bundle_sha256 \
             ) VALUES({layout},{},'{predecessor_digest}', \
                      1,1,1,'TEST_CODE_FUTURE_LAYOUT_{layout}','{digest}'); \
             COMMIT;",
            layout - 1
        ));
        predecessor_digest = digest;
        let expected = if layout > current_layout {
            ChainPostCloseError::UnsupportedVersion
        } else {
            ChainPostCloseError::SchemaRejected
        };
        let future_metadata = layout_catalog_state(&future);
        for _ in 0..2 {
            assert_eq!(
                future.chain_post_close().verify_schema().err(),
                Some(expected.clone()),
                "TEST_CODE layout {layout}"
            );
            assert_eq!(layout_catalog_state(&future), future_metadata);
            assert_eq!(
                future.chain_post_close().verify_schema_v3_reader().err(),
                Some(ChainPostCloseError::UnsupportedVersion)
            );
            assert_eq!(layout_catalog_state(&future), future_metadata);
            future.reopen();
        }
    }

    let mut orphan = V2BusinessFixture::new();
    install_v3(&mut orphan);
    orphan.execute("PRAGMA foreign_keys=OFF;");
    orphan.execute(
        "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) \
         VALUES(4,'TEST_CODE_ORPHAN','table','CREATE TABLE TEST_CODE_ORPHAN(x INTEGER)');",
    );
    orphan.execute("PRAGMA foreign_keys=ON;");
    let orphan_metadata = layout_catalog_state(&orphan);
    assert_eq!(
        orphan.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&orphan), orphan_metadata);
    orphan.reopen();
    assert_eq!(
        orphan.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&orphan), orphan_metadata);

    let mut unsealed = V2BusinessFixture::new();
    install_v3(&mut unsealed);
    remove_layout_header_behind_guard(&unsealed, 3);
    let unsealed_metadata = layout_catalog_state(&unsealed);
    assert_eq!(
        unsealed.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&unsealed), unsealed_metadata);
    unsealed.reopen();
    assert_eq!(
        unsealed.chain_post_close().verify_schema().err(),
        Some(ChainPostCloseError::SchemaRejected)
    );
    assert_eq!(layout_catalog_state(&unsealed), unsealed_metadata);
}

#[test]
fn sealed_layout_headers_and_registry_rows_reject_all_rewrites() {
    let mut fixture = V2BusinessFixture::new();
    install_v3(&mut fixture);
    let before_headers = rows(
        fixture.connection(),
        "SELECT * FROM chain_post_close_layouts ORDER BY layout_version",
    );
    let before_registry = rows(
        fixture.connection(),
        "SELECT * FROM chain_post_close_layout_objects ORDER BY layout_version,name",
    );
    for sql in [
        "UPDATE chain_post_close_layouts SET description='TEST_CODE_CHANGED' WHERE layout_version=3;",
        "DELETE FROM chain_post_close_layouts WHERE layout_version=3;",
        "INSERT OR REPLACE INTO chain_post_close_layouts SELECT * FROM chain_post_close_layouts WHERE layout_version=3;",
        "INSERT INTO chain_post_close_layout_objects(layout_version,name,object_type,definition) VALUES(3,'TEST_CODE_EXTRA','table','CREATE TABLE TEST_CODE_EXTRA(x INTEGER)');",
        "UPDATE chain_post_close_layout_objects SET definition='TEST_CODE_CHANGED' WHERE layout_version=3 AND name='chain_post_close_concept_cache_writes';",
        "DELETE FROM chain_post_close_layout_objects WHERE layout_version=3 AND name='chain_post_close_concept_cache_writes';",
        "INSERT OR REPLACE INTO chain_post_close_layout_objects SELECT * FROM chain_post_close_layout_objects WHERE layout_version=3 AND name='chain_post_close_concept_cache_writes';",
    ] {
        assert!(fixture.connection().execute_batch(sql).is_err(), "{sql}");
    }
    assert_eq!(
        rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layouts ORDER BY layout_version"
        ),
        before_headers
    );
    assert_eq!(
        rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layout_objects ORDER BY layout_version,name"
        ),
        before_registry
    );
    assert_eq!(
        fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        3
    );
}
