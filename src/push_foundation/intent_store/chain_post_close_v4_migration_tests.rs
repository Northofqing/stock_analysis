use rusqlite::types::Value;

use super::*;
use crate::pipeline::chain_analysis::preparation::FixedClusterConfiguration;
use crate::push_foundation::intent_store::chain_post_close::{LocalChainPostClose, RunLease};

fn install_chain_daily(fixture: &V2BusinessFixture) {
    fixture.execute(
        "CREATE TABLE chain_daily ( \
           date TEXT NOT NULL, concept TEXT NOT NULL, stocks TEXT NOT NULL, \
           continuation_count INTEGER NOT NULL DEFAULT 0, \
           created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
           PRIMARY KEY (date, concept));",
    );
}

async fn real_v3_facts_fixture() -> V2Facts {
    let mut facts = v2_facts_fixture();
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .migrate_schema_v2_to_v3()
            .unwrap()
            .schema_version(),
        3
    );
    let mut local = facts
        .fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&facts.config)
        .unwrap();
    let recovery = local.inspect_run(&facts.confirmed).unwrap();
    let lease = local
        .resume_run(
            &facts.confirmed,
            dated_lease(
                "2026-07-21",
                "TEST_CODE_V4_BASELINE_CONFIRMED",
                8_001,
                12_000,
                Some(recovery.head_version()),
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
    drop(local);
    assert_eq!(
        facts.fixture.count("chain_post_close_concept_cache_writes"),
        1
    );
    facts
}

fn durable_v3_state(fixture: &V2BusinessFixture) -> Vec<Vec<Vec<Value>>> {
    [
        "SELECT * FROM chain_post_close_schema ORDER BY schema_version",
        "SELECT * FROM chain_post_close_objects ORDER BY name",
        "SELECT * FROM chain_post_close_layouts WHERE layout_version<=3 ORDER BY layout_version",
        "SELECT * FROM chain_post_close_layout_objects WHERE layout_version<=3 ORDER BY layout_version,name",
        "SELECT * FROM chain_post_close_runs ORDER BY intent_id",
        "SELECT * FROM chain_post_close_stage_begins ORDER BY intent_id,run_version",
        "SELECT * FROM chain_post_close_stage_results ORDER BY intent_id,run_version",
        "SELECT * FROM chain_post_close_concept_cache_writes ORDER BY intent_id,run_version",
        "SELECT * FROM stock_concepts ORDER BY code",
        "SELECT * FROM chain_daily ORDER BY date,concept",
        "SELECT * FROM push_foundation_schema ORDER BY version",
        "SELECT * FROM push_foundation_objects ORDER BY name",
        "PRAGMA application_id",
        "PRAGMA user_version",
    ]
    .into_iter()
    .map(|sql| rows(fixture.connection(), sql))
    .collect()
}

fn v4_fact_state(fixture: &V2BusinessFixture) -> Vec<Vec<Vec<Value>>> {
    [
        "SELECT * FROM chain_post_close_cluster_configurations ORDER BY intent_id",
        "SELECT * FROM chain_post_close_cluster_materials ORDER BY intent_id",
        "SELECT * FROM chain_post_close_chain_daily_applications ORDER BY intent_id",
    ]
    .into_iter()
    .map(|sql| rows(fixture.connection(), sql))
    .collect()
}

fn fixed_v4_reference_additions() -> Vec<Vec<Value>> {
    let reference = Connection::open_in_memory().unwrap();
    reference
        .execute_batch(include_str!("chain_post_close.v1.sql"))
        .unwrap();
    reference
        .execute_batch(include_str!("chain_post_close.v2.sql"))
        .unwrap();
    reference
        .execute_batch(include_str!("chain_post_close.v3.sql"))
        .unwrap();
    let before = full_catalog(&reference);
    reference
        .execute_batch(include_str!("chain_post_close.v4.sql"))
        .unwrap();
    catalog_additions(&full_catalog(&reference), &before)
}

async fn run_v4_recovery(
    local: &mut LocalChainPostClose<'_>,
    lease: RunLease,
    provider: &PanicProvider,
    date: &str,
    code: &str,
    now: i64,
) -> anyhow::Error {
    let clock = ControlledClock::new(captured(date).1.get() + now);
    let mut io = local
        .cluster_preparation_io(
            lease,
            provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("1")),
        )
        .unwrap();
    prepare_chain_analysis_with_io(
        NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
        dated_stocks(code),
        None,
        &mut io,
    )
    .await
    .expect_err("v4 recovery must stop at its next durable boundary")
}

#[tokio::test]
async fn v3_to_v4_preserves_real_old_facts_and_recovers_without_provider() {
    let mut facts = real_v3_facts_fixture().await;
    install_chain_daily(&facts.fixture);
    let before = durable_v3_state(&facts.fixture);
    let before_catalog = full_catalog(facts.fixture.connection());
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .migrate_schema_v3_to_v4()
            .unwrap()
            .schema_version(),
        4
    );
    assert_eq!(durable_v3_state(&facts.fixture), before);
    let after_catalog = full_catalog(facts.fixture.connection());
    assert!(catalog_additions(&before_catalog, &after_catalog).is_empty());
    assert_eq!(
        catalog_additions(&after_catalog, &before_catalog),
        fixed_v4_reference_additions()
    );
    assert_eq!(catalog(facts.fixture.connection()).len(), 43);
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
        facts
            .fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version=4",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        43
    );
    for table in [
        "chain_post_close_cluster_configurations",
        "chain_post_close_cluster_materials",
        "chain_post_close_chain_daily_applications",
    ] {
        assert_eq!(facts.fixture.count(table), 0);
    }
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        4
    );
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema_v3_reader()
            .err(),
        Some(ChainPostCloseError::SchemaRejected)
    );

    facts.fixture.reopen();
    assert_eq!(durable_v3_state(&facts.fixture), before);
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        4
    );
    let provider = PanicProvider {
        calls: RefCell::new(Vec::new()),
    };
    let mut local = facts
        .fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&facts.config)
        .unwrap();
    let recovery = local.inspect_run(&facts.confirmed).unwrap();
    assert_eq!(recovery.results()[0].bytes, ORIGINAL_RAW.as_bytes());
    let lease = local
        .resume_run(
            &facts.confirmed,
            dated_lease(
                "2026-07-21",
                "TEST_CODE_V3_FACTORY_ON_V4",
                12_001,
                16_000,
                Some(recovery.head_version()),
            ),
        )
        .unwrap();
    let v3_clock = ControlledClock::new(captured("2026-07-21").1.get() + 12_100);
    assert_eq!(
        local
            .concept_batch_preparation_io(lease, &provider, &v3_clock)
            .err(),
        Some(ChainPostCloseError::UnsupportedVersion)
    );
    let recovery = local.inspect_run(&facts.confirmed).unwrap();
    let lease = local
        .resume_run(
            &facts.confirmed,
            dated_lease(
                "2026-07-21",
                "TEST_CODE_V4_RECOVERY_CONFIRMED",
                16_001,
                20_000,
                Some(recovery.head_version()),
            ),
        )
        .unwrap();
    let error = run_v4_recovery(
        &mut local,
        lease,
        &provider,
        "2026-07-21",
        CONFIRMED_CODE,
        16_100,
    )
    .await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::BoardDirectory,
        })
    ));
    assert!(provider.calls.borrow().is_empty());

    let recovery = local.inspect_run(&facts.unconfirmed).unwrap();
    assert!(recovery.results().is_empty());
    let lease = local
        .resume_run(
            &facts.unconfirmed,
            dated_lease(
                "2026-07-22",
                "TEST_CODE_V4_RECOVERY_UNCONFIRMED",
                8_001,
                12_000,
                Some(recovery.head_version()),
            ),
        )
        .unwrap();
    let error = run_v4_recovery(
        &mut local,
        lease,
        &provider,
        "2026-07-22",
        UNCONFIRMED_CODE,
        8_100,
    )
    .await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::IncompleteOnReopen { .. })
    ));
    assert!(provider.calls.borrow().is_empty());
}

#[tokio::test]
async fn v3_to_v4_real_commit_failure_rolls_back_old_facts_and_all_new_objects() {
    let mut facts = real_v3_facts_fixture().await;
    install_chain_daily(&facts.fixture);
    let before = durable_v3_state(&facts.fixture);
    let before_layout = layout_catalog_state(&facts.fixture);
    let journal_mode: String = facts
        .fixture
        .connection()
        .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode, "delete");
    let reader = Connection::open_with_flags(
        facts.fixture.database(),
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
        2
    );
    assert_eq!(
        facts.fixture.chain_post_close().migrate_schema_v3_to_v4(),
        Err(ChainPostCloseError::StorageFailed {
            operation: "commit"
        })
    );
    assert!(facts.fixture.connection().is_autocommit());
    assert_eq!(
        facts
            .fixture
            .connection()
            .query_row("PRAGMA query_only", [], |row| { row.get::<_, i64>(0) })
            .unwrap(),
        0
    );
    assert_eq!(durable_v3_state(&facts.fixture), before);
    assert_eq!(layout_catalog_state(&facts.fixture), before_layout);
    assert_eq!(
        facts
            .fixture
            .connection()
            .query_row(
                "SELECT count(*) FROM chain_post_close_layout_objects WHERE layout_version=4",
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
        3
    );
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema_v3_reader()
            .unwrap()
            .schema_version(),
        3
    );
    assert_eq!(durable_v3_state(&facts.fixture), before);
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .migrate_schema_v3_to_v4()
            .unwrap()
            .schema_version(),
        4
    );
    assert_eq!(durable_v3_state(&facts.fixture), before);
    assert_eq!(
        facts
            .fixture
            .chain_post_close()
            .verify_schema()
            .unwrap()
            .schema_version(),
        4
    );
}

#[test]
fn v4_reader_rejects_foreign_index_and_trigger_attachments_without_repair() {
    for (kind, sql) in [
        (
            "index",
            "CREATE INDEX TEST_CODE_FOREIGN_MATERIAL_INDEX \
             ON chain_post_close_cluster_materials(run_id);",
        ),
        (
            "trigger",
            "CREATE TRIGGER TEST_CODE_FOREIGN_APPLICATION_TRIGGER \
             BEFORE INSERT ON chain_post_close_chain_daily_applications \
             BEGIN SELECT 1; END;",
        ),
    ] {
        let mut fixture = V2BusinessFixture::new();
        install_v3(&mut fixture);
        install_chain_daily(&fixture);
        assert_eq!(
            fixture
                .chain_post_close()
                .migrate_schema_v3_to_v4()
                .unwrap()
                .schema_version(),
            4
        );
        fixture.execute(sql);
        let damaged = layout_catalog_state(&fixture);
        let old_facts = durable_v3_state(&fixture);
        let v4_facts = v4_fact_state(&fixture);
        assert_eq!(
            fixture.chain_post_close().verify_schema().err(),
            Some(ChainPostCloseError::SchemaRejected),
            "foreign {kind} must be rejected"
        );
        assert_eq!(layout_catalog_state(&fixture), damaged);
        assert_eq!(durable_v3_state(&fixture), old_facts);
        assert_eq!(v4_fact_state(&fixture), v4_facts);
        fixture.reopen();
        assert_eq!(
            fixture.chain_post_close().verify_schema().err(),
            Some(ChainPostCloseError::SchemaRejected),
            "foreign {kind} must remain rejected after reopen"
        );
        assert_eq!(layout_catalog_state(&fixture), damaged);
        assert_eq!(durable_v3_state(&fixture), old_facts);
        assert_eq!(v4_fact_state(&fixture), v4_facts);
    }
}
