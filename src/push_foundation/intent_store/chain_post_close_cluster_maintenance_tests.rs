use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::types::Value;
use rusqlite::{params, Connection, OpenFlags};

use super::*;
use crate::push_foundation::intent_store::chain_post_close::{LocalChainPostClose, RunLease};

fn stock(code: &str, change_pct: f64) -> TopStock {
    TopStock {
        code: code.to_owned(),
        name: format!("TEST_CODE_NAME_{code}"),
        change_pct,
        price: 23.75,
        volume_ratio: Some(1.25),
        main_net_yi: Some(0.375),
    }
}

fn install_v4(fixture: &mut V2BusinessFixture) {
    fixture.install_v2();
    fixture
        .chain_post_close()
        .migrate_schema_v2_to_v3()
        .unwrap();
    fixture.execute(
        "CREATE TABLE chain_daily ( \
           date TEXT NOT NULL, concept TEXT NOT NULL, stocks TEXT NOT NULL, \
           continuation_count INTEGER NOT NULL DEFAULT 0, \
           created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
           PRIMARY KEY (date, concept));",
    );
    fixture
        .chain_post_close()
        .migrate_schema_v3_to_v4()
        .unwrap();
}

fn cache(fixture: &V2BusinessFixture, code: &str, concepts: &[&str]) {
    let bytes = serde_json::to_string(concepts).unwrap();
    fixture
        .connection()
        .execute(
            "INSERT INTO stock_concepts(code,concepts,updated_at) \
             VALUES(?1,?2,'2026-07-21 14:00:00')",
            params![code, bytes],
        )
        .unwrap();
}

fn acquire<'a>(
    local: &mut LocalChainPostClose<'a>,
    config: &LocalChainPostCloseConfig,
    run: &str,
    stocks: Vec<TopStock>,
    owner: &str,
    until: i64,
) -> RunLease {
    let context = build_single_user_local_chain_post_close_context(config, run_input(run)).unwrap();
    local
        .acquire_run(
            context,
            fixed_input(stocks),
            lease_request(owner, 1_000, until, None),
        )
        .unwrap()
}

async fn public_prepare<P: ConceptProviderRawIo, C: ConceptEffectClock>(
    local: &mut LocalChainPostClose<'_>,
    lease: RunLease,
    provider: &P,
    clock: &C,
    configuration: FixedClusterConfiguration,
    stocks: Vec<TopStock>,
) -> anyhow::Error {
    let mut io = local
        .cluster_preparation_io(lease, provider, clock, configuration)
        .unwrap();
    prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks,
        None,
        &mut io,
    )
    .await
    .expect_err("v4 must stop before the board directory")
}

fn assert_board_stop(error: &anyhow::Error) {
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::BoardDirectory,
        })
    ));
}

struct ObservedProvider {
    database: PathBuf,
    replies: HashMap<String, String>,
    calls: RefCell<Vec<String>>,
}

#[async_trait::async_trait(?Send)]
impl ConceptProviderRawIo for ObservedProvider {
    async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
        let connection = Connection::open_with_flags(
            &self.database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM chain_post_close_cluster_configurations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "configuration must commit before provider"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM chain_post_close_stage_begins \
                     WHERE effect_kind='ConceptProvider' AND effect_key=?1",
                    [code],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "begin must commit before provider"
        );
        self.calls.borrow_mut().push(code.to_owned());
        Ok(self.replies.get(code).unwrap().clone())
    }
}

#[test]
fn fixed_cluster_configuration_preserves_legacy_parse_boundaries_and_conflicts() {
    for value in [None, Some("invalid"), Some(" 2"), Some("2 ")] {
        assert_eq!(
            FixedClusterConfiguration::resolve(value).min_cluster_size(),
            3
        );
    }
    assert_eq!(
        FixedClusterConfiguration::resolve(Some("0")).min_cluster_size(),
        0
    );
    assert_eq!(
        FixedClusterConfiguration::resolve(Some(&usize::MAX.to_string())).min_cluster_size(),
        usize::MAX
    );
    let mut non_finite = stock("TEST_CODE_NON_FINITE", 9.9);
    non_finite.price = f64::NAN;
    assert!(matches!(
        FixedChainPreparationInput::try_new(
            BusinessDate::parse(BUSINESS_DATE).unwrap(),
            vec![non_finite],
            None,
            source(SourceStatus::Available, "TEST_CODE_NON_FINITE_LIMIT_UP"),
            source(SourceStatus::VerifiedEmpty, "TEST_CODE_NON_FINITE_MACRO"),
        ),
        Err(ChainPostCloseError::InvalidInput {
            check: "stock numeric value"
        })
    ));

    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    let stocks = vec![stock("TEST_CODE_CONFIG", 9.9)];
    cache(&fixture, "TEST_CODE_CONFIG", &["TEST_CODE_CONFIG_CONCEPT"]);
    let config = local_config(BUILD_A);
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        "TEST_CODE_RUN_CLUSTER_CONFIG",
        stocks,
        "TEST_CODE_CONFIG_OWNER_A",
        2_000,
    );
    let intent = lease.intent_id().clone();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(1_100));
    let maximum = usize::MAX.to_string();
    let io = local
        .cluster_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some(&maximum)),
        )
        .unwrap();
    drop(io);
    let head = local.inspect_run(&intent).unwrap().head_version();
    let lease = local
        .resume_run(
            &intent,
            lease_request("TEST_CODE_CONFIG_OWNER_B", 2_001, 4_000, Some(head)),
        )
        .unwrap();
    let expected_head = local.inspect_run(&intent).unwrap().head_version();
    let conflicting_clock = ControlledClock::new(at(2_100));
    assert!(matches!(
        local.cluster_preparation_io(
            lease,
            &provider,
            &conflicting_clock,
            FixedClusterConfiguration::resolve(Some("2")),
        ),
        Err(ChainPostCloseError::InvalidInput {
            check: "cluster configuration conflict"
        })
    ));
    assert_eq!(
        local.inspect_run(&intent).unwrap().head_version(),
        expected_head
    );
    assert_eq!(provider.calls.get(), 0);
    drop(local);
    assert_eq!(fixture.count("chain_post_close_cluster_configurations"), 1);
    assert_eq!(fixture.count("chain_post_close_cluster_materials"), 0);
    assert_eq!(
        fixture.count("chain_post_close_chain_daily_applications"),
        0
    );
    assert_eq!(fixture.count("chain_daily"), 0);
    fixture.reopen();
    let bytes: Vec<u8> = fixture
        .connection()
        .query_row(
            "SELECT configuration_bytes FROM chain_post_close_cluster_configurations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        bytes,
        format!(
            "{{\"schema_version\":1,\"min_cluster_size\":\"{}\"}}",
            usize::MAX
        )
        .into_bytes()
    );
}

#[tokio::test]
async fn v4_missing_provider_observes_configuration_and_material_saves_complete_map() {
    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    cache(&fixture, "TEST_CODE_ORDER_HIT", &["TEST_CODE_ORDER_MAIN"]);
    cache(
        &fixture,
        "TEST_CODE_ORDER_EXTRA",
        &["TEST_CODE_ORDER_EXTRA_CONCEPT"],
    );
    let stocks = vec![
        stock("TEST_CODE_ORDER_HIT", 9.9),
        stock("TEST_CODE_ORDER_MISSING", 9.8),
    ];
    let config = local_config(BUILD_A);
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        "TEST_CODE_RUN_CLUSTER_ORDER",
        stocks.clone(),
        "TEST_CODE_ORDER_OWNER",
        8_000,
    );
    let intent = lease.intent_id().clone();
    let provider = ObservedProvider {
        database,
        replies: HashMap::from([(
            "TEST_CODE_ORDER_MISSING".to_owned(),
            "{\"all_boards\":[\"TEST_CODE_ORDER_MAIN\"]}".to_owned(),
        )]),
        calls: RefCell::new(Vec::new()),
    };
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &ControlledClock::new(at(1_100)),
        FixedClusterConfiguration::resolve(Some("2")),
        stocks,
    )
    .await;
    assert_board_stop(&error);
    assert_eq!(
        provider.calls.borrow().as_slice(),
        ["TEST_CODE_ORDER_MISSING"]
    );
    assert_eq!(
        local
            .inspect_cluster_application(&intent)
            .unwrap()
            .clusters()
            .len(),
        1
    );
    drop(local);
    let concept_bytes: Vec<u8> = fixture
        .connection()
        .query_row(
            "SELECT concept_map_bytes FROM chain_post_close_cluster_materials",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        concept_bytes.as_slice(),
        b"{\"schema_version\":1,\"concepts\":{\"TEST_CODE_ORDER_EXTRA\":[\"TEST_CODE_ORDER_EXTRA_CONCEPT\"],\"TEST_CODE_ORDER_HIT\":[\"TEST_CODE_ORDER_MAIN\"],\"TEST_CODE_ORDER_MISSING\":[\"TEST_CODE_ORDER_MAIN\"]}}"
    );
}

#[tokio::test]
async fn configuration_can_precede_resume_and_material_application_use_new_generation() {
    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    let stocks = vec![stock("TEST_CODE_RESUME_CLUSTER", 9.9)];
    cache(
        &fixture,
        "TEST_CODE_RESUME_CLUSTER",
        &["TEST_CODE_RESUME_MAIN"],
    );
    let config = local_config(BUILD_A);
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        "TEST_CODE_RUN_CLUSTER_RESUME",
        stocks.clone(),
        "TEST_CODE_RESUME_OWNER_A",
        2_000,
    );
    let intent = lease.intent_id().clone();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let initial_clock = ControlledClock::new(at(1_100));
    let io = local
        .cluster_preparation_io(
            lease,
            &provider,
            &initial_clock,
            FixedClusterConfiguration::resolve(Some("1")),
        )
        .unwrap();
    drop(io);
    let head = local.inspect_run(&intent).unwrap().head_version();
    let lease = local
        .resume_run(
            &intent,
            lease_request("TEST_CODE_RESUME_OWNER_B", 2_001, 5_000, Some(head)),
        )
        .unwrap();
    let resumed_clock = ControlledClock::new(at(2_100));
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &resumed_clock,
        FixedClusterConfiguration::resolve(Some("1")),
        stocks,
    )
    .await;
    assert_board_stop(&error);
    drop(local);
    let generations: (i64, i64, i64) = fixture
        .connection()
        .query_row(
            "SELECT configuration.lease_generation,material.lease_generation,application.lease_generation \
             FROM chain_post_close_cluster_configurations configuration \
             JOIN chain_post_close_cluster_materials material USING(intent_id) \
             JOIN chain_post_close_chain_daily_applications application USING(intent_id)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(generations, (1, 2, 2));
}

#[tokio::test]
async fn first_material_preserves_equal_change_order_and_legal_overlapping_clusters() {
    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    let stocks = ["A", "B", "C", "D"]
        .into_iter()
        .map(|suffix| stock(&format!("TEST_CODE_OVERLAP_{suffix}"), 9.9))
        .collect::<Vec<_>>();
    cache(&fixture, "TEST_CODE_OVERLAP_A", &["TEST_CODE_OVERLAP_X"]);
    cache(
        &fixture,
        "TEST_CODE_OVERLAP_B",
        &["TEST_CODE_OVERLAP_X", "TEST_CODE_OVERLAP_Y"],
    );
    cache(
        &fixture,
        "TEST_CODE_OVERLAP_C",
        &["TEST_CODE_OVERLAP_X", "TEST_CODE_OVERLAP_Y"],
    );
    cache(&fixture, "TEST_CODE_OVERLAP_D", &["TEST_CODE_OVERLAP_Y"]);
    let config = local_config(BUILD_A);
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        "TEST_CODE_RUN_CLUSTER_OVERLAP",
        stocks.clone(),
        "TEST_CODE_OVERLAP_OWNER_A",
        4_000,
    );
    let intent = lease.intent_id().clone();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &ControlledClock::new(at(1_100)),
        FixedClusterConfiguration::resolve(Some("2")),
        stocks.clone(),
    )
    .await;
    assert_board_stop(&error);
    let inspection = local.inspect_cluster_application(&intent).unwrap();
    assert_eq!(inspection.clusters().len(), 2);
    let first_bytes = inspection.material_bytes().to_vec();
    let first_orders = inspection
        .clusters()
        .iter()
        .map(|cluster| {
            cluster
                .stocks
                .iter()
                .map(|stock| stock.code.clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let recovered_values = inspection
        .clusters()
        .iter()
        .flat_map(|cluster| cluster.stocks.iter())
        .chain(inspection.isolated().iter())
        .map(|stock| (stock.code.clone(), serde_json::to_vec(stock).unwrap()))
        .collect::<HashMap<_, _>>();
    for original in &stocks {
        assert_eq!(
            recovered_values.get(&original.code),
            Some(&serde_json::to_vec(original).unwrap())
        );
    }
    assert!(first_orders.iter().all(|codes| {
        codes.contains(&"TEST_CODE_OVERLAP_B".to_owned())
            && codes.contains(&"TEST_CODE_OVERLAP_C".to_owned())
    }));
    let head = local.inspect_run(&intent).unwrap().head_version();
    drop(local);
    fixture.reopen();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .resume_run(
            &intent,
            lease_request("TEST_CODE_OVERLAP_OWNER_B", 4_001, 7_000, Some(head)),
        )
        .unwrap();
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &ControlledClock::new(at(4_100)),
        FixedClusterConfiguration::resolve(Some("2")),
        stocks,
    )
    .await;
    assert_board_stop(&error);
    let recovered = local.inspect_cluster_application(&intent).unwrap();
    assert_eq!(recovered.material_bytes(), first_bytes);
    assert_eq!(
        recovered
            .clusters()
            .iter()
            .map(|cluster| cluster
                .stocks
                .iter()
                .map(|stock| stock.code.clone())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        first_orders
    );
}

#[tokio::test]
async fn empty_cluster_material_applies_empty_lifecycle_without_deleting_existing_rows() {
    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    let stocks = vec![
        stock("TEST_CODE_EMPTY_A", 9.9),
        stock("TEST_CODE_EMPTY_B", 9.8),
    ];
    cache(
        &fixture,
        "TEST_CODE_EMPTY_A",
        &["TEST_CODE_EMPTY_CONCEPT_A"],
    );
    cache(
        &fixture,
        "TEST_CODE_EMPTY_B",
        &["TEST_CODE_EMPTY_CONCEPT_B"],
    );
    fixture.execute(
        "INSERT INTO chain_daily(date,concept,stocks,continuation_count) VALUES \
         ('2026-07-21','TEST_CODE_EXISTING','[\"TEST_CODE_EXISTING_STOCK\"]',7);",
    );
    let existing_created_at: String = fixture
        .connection()
        .query_row(
            "SELECT created_at FROM chain_daily WHERE concept='TEST_CODE_EXISTING'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let config = local_config(BUILD_A);
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        "TEST_CODE_RUN_CLUSTER_EMPTY",
        stocks.clone(),
        "TEST_CODE_EMPTY_OWNER",
        5_000,
    );
    let intent = lease.intent_id().clone();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &ControlledClock::new(at(1_100)),
        FixedClusterConfiguration::resolve(Some("3")),
        stocks,
    )
    .await;
    assert_board_stop(&error);
    let inspection = local.inspect_cluster_application(&intent).unwrap();
    assert!(inspection.clusters().is_empty());
    assert_eq!(inspection.isolated().len(), 2);
    assert!(inspection.lifecycle_days().is_empty());
    drop(local);
    assert_eq!(
        chain_daily_row(&fixture, BUSINESS_DATE, "TEST_CODE_EXISTING"),
        ("[\"TEST_CODE_EXISTING_STOCK\"]".to_owned(), 7)
    );
    assert_eq!(
        fixture
            .connection()
            .query_row(
                "SELECT created_at FROM chain_daily WHERE concept='TEST_CODE_EXISTING'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        existing_created_at
    );
}

fn two_cluster_stocks() -> Vec<TopStock> {
    vec![
        stock("TEST_CODE_ATOMIC_A", 9.9),
        stock("TEST_CODE_ATOMIC_B", 9.8),
    ]
}

fn seed_two_clusters(fixture: &V2BusinessFixture) {
    cache(
        fixture,
        "TEST_CODE_ATOMIC_A",
        &["TEST_CODE_ATOMIC_CONCEPT_A"],
    );
    cache(
        fixture,
        "TEST_CODE_ATOMIC_B",
        &["TEST_CODE_ATOMIC_CONCEPT_B"],
    );
}

#[tokio::test]
async fn second_chain_daily_sql_failure_rolls_back_business_prefix_application_and_head() {
    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    seed_two_clusters(&fixture);
    fixture.execute(
        "CREATE TRIGGER TEST_CODE_FAIL_CHAIN_DAILY \
         BEFORE INSERT ON chain_daily WHEN NEW.concept='TEST_CODE_ATOMIC_CONCEPT_B' \
                 BEGIN SELECT RAISE(ABORT,'TEST_CODE_CHAIN_DAILY_FAILURE'); END;",
    );
    let stocks = two_cluster_stocks();
    let config = local_config(BUILD_A);
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        "TEST_CODE_RUN_CLUSTER_SQL_FAILURE",
        stocks.clone(),
        "TEST_CODE_ATOMIC_OWNER_A",
        4_000,
    );
    let intent = lease.intent_id().clone();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &ControlledClock::new(at(1_100)),
        FixedClusterConfiguration::resolve(Some("1")),
        stocks.clone(),
    )
    .await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { .. })
    ));
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "chain_daily write"
        })
    ));
    let head = local.inspect_run(&intent).unwrap().head_version();
    assert_eq!(head, 2);
    drop(local);
    assert_eq!(fixture.count("chain_post_close_cluster_configurations"), 1);
    assert_eq!(fixture.count("chain_post_close_cluster_materials"), 1);
    assert_eq!(
        fixture.count("chain_post_close_chain_daily_applications"),
        0
    );
    assert_eq!(fixture.count("chain_daily"), 0);

    fixture.execute("DROP TRIGGER TEST_CODE_FAIL_CHAIN_DAILY;");
    fixture.reopen();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let head = local.inspect_run(&intent).unwrap().head_version();
    let lease = local
        .resume_run(
            &intent,
            lease_request("TEST_CODE_ATOMIC_OWNER_B", 4_001, 7_000, Some(head)),
        )
        .unwrap();
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &ControlledClock::new(at(4_100)),
        FixedClusterConfiguration::resolve(Some("1")),
        stocks,
    )
    .await;
    assert_board_stop(&error);
    assert_eq!(provider.calls.get(), 0);
    drop(local);
    assert_eq!(fixture.count("chain_daily"), 2);
    assert_eq!(
        fixture.count("chain_post_close_chain_daily_applications"),
        1
    );
}

struct ApplicationMapCorruptionClock {
    now: i64,
    database: PathBuf,
    forged_map: Vec<u8>,
    original_material: RefCell<Option<Vec<u8>>>,
    corrupted: Cell<bool>,
}

impl ConceptEffectClock for ApplicationMapCorruptionClock {
    fn now(&self) -> UtcMicros {
        if !self.corrupted.get() {
            let connection = Connection::open_with_flags(
                &self.database,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .unwrap();
            let material_ready = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM chain_post_close_cluster_materials) \
                     AND NOT EXISTS(SELECT 1 FROM chain_post_close_chain_daily_applications)",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap();
            if material_ready {
                let original = connection
                    .query_row(
                        "SELECT material_bytes FROM chain_post_close_cluster_materials",
                        [],
                        |row| row.get::<_, Vec<u8>>(0),
                    )
                    .unwrap();
                let definition: String = connection
                    .query_row(
                        "SELECT sql FROM sqlite_schema WHERE type='trigger' \
                         AND name='chain_post_close_cluster_materials_update'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                connection
                    .execute_batch("DROP TRIGGER chain_post_close_cluster_materials_update;")
                    .unwrap();
                connection
                    .execute(
                        "UPDATE chain_post_close_cluster_materials \
                         SET concept_map_bytes=?1,concept_map_length=?2,concept_map_sha256=?3",
                        params![
                            &self.forged_map,
                            i64::try_from(self.forged_map.len()).unwrap(),
                            raw_digest(&self.forged_map).as_str()
                        ],
                    )
                    .unwrap();
                connection.execute_batch(&definition).unwrap();
                *self.original_material.borrow_mut() = Some(original);
                self.corrupted.set(true);
            }
            connection.close().unwrap();
        }
        UtcMicros::try_new(self.now).unwrap()
    }
}

#[tokio::test]
async fn application_rejects_changed_complete_concept_map_before_business_write() {
    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    let stocks = vec![stock("TEST_CODE_RETRY_MAP", 9.9)];
    cache(
        &fixture,
        "TEST_CODE_RETRY_MAP",
        &["TEST_CODE_RETRY_MAP_MAIN"],
    );
    cache(
        &fixture,
        "TEST_CODE_RETRY_MAP_EXTRA",
        &["TEST_CODE_RETRY_MAP_EXTRA_ORIGINAL"],
    );
    let config = local_config(BUILD_A);
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        "TEST_CODE_RUN_CLUSTER_RETRY_BAD_MAP",
        stocks.clone(),
        "TEST_CODE_RETRY_MAP_OWNER_A",
        5_000,
    );
    let intent = lease.intent_id().clone();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let forged_map = b"{\"schema_version\":1,\"concepts\":{\"TEST_CODE_RETRY_MAP\":[\"TEST_CODE_RETRY_MAP_MAIN\"],\"TEST_CODE_RETRY_MAP_EXTRA\":[\"TEST_CODE_RETRY_MAP_EXTRA_FORGED\"]}}".to_vec();
    let clock = ApplicationMapCorruptionClock {
        now: at(1_100),
        database,
        forged_map: forged_map.clone(),
        original_material: RefCell::new(None),
        corrupted: Cell::new(false),
    };
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &clock,
        FixedClusterConfiguration::resolve(Some("1")),
        stocks,
    )
    .await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { .. })
    ));
    assert_eq!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(&ChainPostCloseError::SchemaRejected)
    );
    assert!(clock.corrupted.get());
    assert_eq!(provider.calls.get(), 0);
    assert_eq!(local.inspect_run(&intent).unwrap().head_version(), 2);
    drop(local);
    assert_eq!(fixture.count("chain_daily"), 0);
    assert_eq!(
        fixture.count("chain_post_close_chain_daily_applications"),
        0
    );
    let stored: (Vec<u8>, Vec<u8>) = fixture
        .connection()
        .query_row(
            "SELECT material_bytes,concept_map_bytes FROM chain_post_close_cluster_materials",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        (
            clock.original_material.borrow().clone().unwrap(),
            forged_map
        )
    );
    let damaged = durable_cluster_state(&fixture);
    fixture.reopen();
    {
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(&config)
            .unwrap();
        assert_eq!(
            local.inspect_cluster_application(&intent).err(),
            Some(ChainPostCloseError::SchemaRejected)
        );
    }
    assert_eq!(durable_cluster_state(&fixture), damaged);
}

struct ApplicationCommitClock {
    now: i64,
    database: PathBuf,
    reader: RefCell<Option<Connection>>,
    material_bytes: RefCell<Option<Vec<u8>>>,
}

impl ApplicationCommitClock {
    fn release(&self) {
        let connection = self.reader.borrow_mut().take().unwrap();
        connection.execute_batch("ROLLBACK;").unwrap();
        connection.close().unwrap();
    }
}

impl ConceptEffectClock for ApplicationCommitClock {
    fn now(&self) -> UtcMicros {
        if self.reader.borrow().is_none() {
            let connection = Connection::open_with_flags(
                &self.database,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .unwrap();
            let material_ready = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM chain_post_close_cluster_materials) \
                     AND NOT EXISTS(SELECT 1 FROM chain_post_close_chain_daily_applications)",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap();
            if material_ready {
                connection.execute_batch("BEGIN DEFERRED;").unwrap();
                let bytes = connection
                    .query_row(
                        "SELECT material_bytes FROM chain_post_close_cluster_materials",
                        [],
                        |row| row.get::<_, Vec<u8>>(0),
                    )
                    .unwrap();
                *self.material_bytes.borrow_mut() = Some(bytes);
                *self.reader.borrow_mut() = Some(connection);
            } else {
                connection.close().unwrap();
            }
        }
        UtcMicros::try_new(self.now).unwrap()
    }
}

#[tokio::test]
async fn chain_daily_commit_failure_rolls_back_business_rows_application_and_head() {
    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    seed_two_clusters(&fixture);
    let stocks = two_cluster_stocks();
    let config = local_config(BUILD_A);
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        "TEST_CODE_RUN_CLUSTER_COMMIT_FAILURE",
        stocks.clone(),
        "TEST_CODE_COMMIT_OWNER",
        8_000,
    );
    let intent = lease.intent_id().clone();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ApplicationCommitClock {
        now: at(1_100),
        database,
        reader: RefCell::new(None),
        material_bytes: RefCell::new(None),
    };
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &clock,
        FixedClusterConfiguration::resolve(Some("1")),
        stocks.clone(),
    )
    .await;
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::AuthorityRejected { .. })
    ));
    assert!(matches!(
        error.downcast_ref::<ChainPostCloseError>(),
        Some(ChainPostCloseError::StorageFailed {
            operation: "commit"
        })
    ));
    {
        let reader = clock.reader.borrow();
        let reader = reader.as_ref().unwrap();
        assert_eq!(
            reader
                .query_row("SELECT count(*) FROM chain_daily", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            reader
                .query_row(
                    "SELECT count(*) FROM chain_post_close_chain_daily_applications",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }
    let saved_material = clock.material_bytes.borrow().clone().unwrap();
    clock.release();
    drop(local);
    fixture.reopen();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let head = local.inspect_run(&intent).unwrap().head_version();
    assert_eq!(head, 2);
    let lease = local
        .resume_run(
            &intent,
            lease_request("TEST_CODE_COMMIT_OWNER_B", 8_001, 12_000, Some(head)),
        )
        .unwrap();
    let recovery_clock = ControlledClock::new(at(8_100));
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &recovery_clock,
        FixedClusterConfiguration::resolve(Some("1")),
        stocks,
    )
    .await;
    assert_board_stop(&error);
    assert_eq!(provider.calls.get(), 0);
    assert_eq!(
        local
            .inspect_cluster_application(&intent)
            .unwrap()
            .material_bytes(),
        saved_material.as_slice()
    );
    drop(local);
    assert_eq!(fixture.count("chain_post_close_cluster_materials"), 1);
    assert_eq!(fixture.count("chain_daily"), 2);
    assert_eq!(
        fixture.count("chain_post_close_chain_daily_applications"),
        1
    );
}

async fn completed_for_corruption(
    run: &str,
) -> (V2BusinessFixture, LocalChainPostCloseConfig, IntentId) {
    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    let stocks = vec![stock("TEST_CODE_INTEGRITY", 9.9)];
    cache(
        &fixture,
        "TEST_CODE_INTEGRITY",
        &["TEST_CODE_INTEGRITY_MAIN"],
    );
    let config = local_config(BUILD_A);
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        run,
        stocks.clone(),
        "TEST_CODE_INTEGRITY_OWNER_A",
        3_000,
    );
    let intent = lease.intent_id().clone();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &ControlledClock::new(at(1_100)),
        FixedClusterConfiguration::resolve(Some("1")),
        stocks,
    )
    .await;
    assert_board_stop(&error);
    let head = local.inspect_run(&intent).unwrap().head_version();
    let lease = local
        .resume_run(
            &intent,
            lease_request("TEST_CODE_INTEGRITY_OWNER_B", 3_001, 6_000, Some(head)),
        )
        .unwrap();
    let resume_clock = ControlledClock::new(at(3_100));
    let io = local
        .cluster_preparation_io(
            lease,
            &provider,
            &resume_clock,
            FixedClusterConfiguration::resolve(Some("1")),
        )
        .unwrap();
    drop(io);
    drop(local);
    (fixture, config, intent)
}

fn guarded_update(fixture: &V2BusinessFixture, trigger: &str, sql: &str) {
    let definition: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            [trigger],
            |row| row.get(0),
        )
        .unwrap();
    fixture.execute(&format!("DROP TRIGGER {trigger};"));
    fixture.execute(sql);
    fixture.execute(&definition);
}

fn fact_rows(connection: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = connection.prepare(sql).unwrap();
    let column_count = statement.column_count();
    let rows = statement
        .query_map([], |row| {
            (0..column_count)
                .map(|column| row.get::<_, Value>(column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows
}

#[derive(Debug, PartialEq)]
struct DurableClusterState(Vec<Vec<Vec<Value>>>);

fn durable_cluster_state(fixture: &V2BusinessFixture) -> DurableClusterState {
    DurableClusterState(vec![
        fact_rows(
            fixture.connection(),
            "SELECT name,type,tbl_name,CAST(sql AS BLOB) \
             FROM sqlite_schema ORDER BY name,type",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layouts ORDER BY layout_version",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_layout_objects ORDER BY layout_version,name",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_runs ORDER BY intent_id",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_stage_begins ORDER BY intent_id,run_version",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_stage_results ORDER BY intent_id,run_version",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_concept_cache_writes ORDER BY intent_id,run_version",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_cluster_configurations ORDER BY intent_id",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_cluster_materials ORDER BY intent_id",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_post_close_chain_daily_applications ORDER BY intent_id",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM stock_concepts ORDER BY code",
        ),
        fact_rows(
            fixture.connection(),
            "SELECT * FROM chain_daily ORDER BY date,concept",
        ),
    ])
}

fn assert_rejected_without_repair(
    fixture: &mut V2BusinessFixture,
    config: &LocalChainPostCloseConfig,
    intent: &IntentId,
) {
    let damaged = durable_cluster_state(fixture);
    {
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(config)
            .unwrap();
        assert_eq!(
            local.inspect_cluster_application(intent).err(),
            Some(ChainPostCloseError::SchemaRejected)
        );
    }
    assert_eq!(durable_cluster_state(fixture), damaged);
    fixture.reopen();
    {
        let mut local = fixture
            .store
            .as_mut()
            .unwrap()
            .single_user_local_chain_post_close(config)
            .unwrap();
        assert_eq!(
            local.inspect_cluster_application(intent).err(),
            Some(ChainPostCloseError::SchemaRejected)
        );
    }
    assert_eq!(durable_cluster_state(fixture), damaged);
}

async fn assert_cluster_corruption_rejected(run: &str, trigger: &str, update: &str) {
    let (mut fixture, config, intent) = completed_for_corruption(run).await;
    guarded_update(&fixture, trigger, update);
    assert_rejected_without_repair(&mut fixture, &config, &intent);
}

#[tokio::test]
async fn inspect_rejects_child_fact_from_generation_before_its_parent() {
    assert_cluster_corruption_rejected(
        "TEST_CODE_RUN_CLUSTER_BAD_PARENT_GENERATION",
        "chain_post_close_cluster_configurations_update",
        "UPDATE chain_post_close_cluster_configurations \
         SET lease_owner='TEST_CODE_INTEGRITY_OWNER_B',lease_generation=2;",
    )
    .await;
}

#[tokio::test]
async fn inspect_rejects_same_generation_parent_child_owner_contradiction() {
    assert_cluster_corruption_rejected(
        "TEST_CODE_RUN_CLUSTER_BAD_PARENT_OWNER",
        "chain_post_close_cluster_materials_update",
        "UPDATE chain_post_close_cluster_materials \
         SET lease_owner='TEST_CODE_FORGED_OLD_OWNER';",
    )
    .await;
}

#[tokio::test]
async fn inspect_rejects_self_consistent_material_map_not_bound_to_original_concepts() {
    let (mut fixture, config, intent) =
        completed_for_corruption("TEST_CODE_RUN_CLUSTER_BAD_MAP").await;
    let bytes = b"{\"schema_version\":1,\"concepts\":{\"TEST_CODE_INTEGRITY\":[\"TEST_CODE_FORGED_CONCEPT\"]}}".to_vec();
    let definition: String = fixture.connection().query_row("SELECT sql FROM sqlite_schema WHERE type='trigger' AND name='chain_post_close_cluster_materials_update'", [], |row| row.get(0)).unwrap();
    fixture.execute("DROP TRIGGER chain_post_close_cluster_materials_update;");
    fixture.connection().execute("UPDATE chain_post_close_cluster_materials SET concept_map_bytes=?1,concept_map_length=?2,concept_map_sha256=?3", params![&bytes, i64::try_from(bytes.len()).unwrap(), raw_digest(&bytes).as_str()]).unwrap();
    fixture.execute(&definition);
    assert_rejected_without_repair(&mut fixture, &config, &intent);
}

#[tokio::test]
async fn inspect_rejects_noncanonical_material_bytes_with_matching_length_and_hash() {
    let (mut fixture, config, intent) =
        completed_for_corruption("TEST_CODE_RUN_CLUSTER_NONCANONICAL_MATERIAL").await;
    let original: Vec<u8> = fixture
        .connection()
        .query_row(
            "SELECT material_bytes FROM chain_post_close_cluster_materials",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut bytes = Vec::with_capacity(original.len() + 1);
    bytes.push(b' ');
    bytes.extend(original);
    let definition: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_cluster_materials_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    fixture.execute("DROP TRIGGER chain_post_close_cluster_materials_update;");
    fixture
        .connection()
        .execute(
            "UPDATE chain_post_close_cluster_materials \
             SET material_bytes=?1,material_length=?2,material_sha256=?3",
            params![
                &bytes,
                i64::try_from(bytes.len()).unwrap(),
                raw_digest(&bytes).as_str()
            ],
        )
        .unwrap();
    fixture.execute(&definition);
    assert_rejected_without_repair(&mut fixture, &config, &intent);
}

#[tokio::test]
async fn inspect_rejects_self_consistent_lifecycle_rows_not_bound_to_original_material() {
    let (mut fixture, config, intent) =
        completed_for_corruption("TEST_CODE_RUN_CLUSTER_BAD_LIFECYCLE_ROWS").await;
    let bytes = b"{\"schema_version\":1,\"business_date\":\"2026-07-21\",\"rows\":[{\"concept\":\"TEST_CODE_INTEGRITY_MAIN\",\"stocks\":[\"TEST_CODE_FORGED_STOCK\"],\"continuation_count\":0}],\"days\":{\"TEST_CODE_INTEGRITY_MAIN\":1}}".to_vec();
    let definition: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_chain_daily_applications_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    fixture.execute("DROP TRIGGER chain_post_close_chain_daily_applications_update;");
    fixture
        .connection()
        .execute(
            "UPDATE chain_post_close_chain_daily_applications \
             SET lifecycle_bytes=?1,lifecycle_length=?2,lifecycle_sha256=?3",
            params![
                &bytes,
                i64::try_from(bytes.len()).unwrap(),
                raw_digest(&bytes).as_str()
            ],
        )
        .unwrap();
    fixture.execute(&definition);
    assert_rejected_without_repair(&mut fixture, &config, &intent);
}

#[tokio::test]
async fn inspect_rejects_v4_material_version_reused_from_cache_fact() {
    let mut fixture = V2BusinessFixture::new();
    install_v4(&mut fixture);
    let stocks = vec![stock("TEST_CODE_VERSION_MISSING", 9.9)];
    let config = local_config(BUILD_A);
    let database = fixture.database();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = acquire(
        &mut local,
        &config,
        "TEST_CODE_RUN_CLUSTER_VERSION_COLLISION",
        stocks.clone(),
        "TEST_CODE_VERSION_OWNER",
        7_000,
    );
    let intent = lease.intent_id().clone();
    let provider = ObservedProvider {
        database,
        replies: HashMap::from([(
            "TEST_CODE_VERSION_MISSING".to_owned(),
            "{\"all_boards\":[\"TEST_CODE_VERSION_MAIN\"]}".to_owned(),
        )]),
        calls: RefCell::new(Vec::new()),
    };
    let error = public_prepare(
        &mut local,
        lease,
        &provider,
        &ControlledClock::new(at(1_100)),
        FixedClusterConfiguration::resolve(Some("1")),
        stocks,
    )
    .await;
    assert_board_stop(&error);
    drop(local);
    let cache_version: i64 = fixture
        .connection()
        .query_row(
            "SELECT run_version FROM chain_post_close_concept_cache_writes",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let material_trigger: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_cluster_materials_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let application_trigger: String = fixture
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' \
             AND name='chain_post_close_chain_daily_applications_update'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    fixture.execute(
        "PRAGMA foreign_keys=OFF; \
         DROP TRIGGER chain_post_close_cluster_materials_update; \
         DROP TRIGGER chain_post_close_chain_daily_applications_update;",
    );
    fixture
        .connection()
        .execute(
            "UPDATE chain_post_close_cluster_materials \
             SET concept_state_through_head_version=?1,prior_head_version=?1,run_version=?2",
            params![cache_version - 1, cache_version],
        )
        .unwrap();
    fixture
        .connection()
        .execute(
            "UPDATE chain_post_close_chain_daily_applications SET material_run_version=?1",
            [cache_version],
        )
        .unwrap();
    fixture.execute(&material_trigger);
    fixture.execute(&application_trigger);
    fixture.execute("PRAGMA foreign_keys=ON;");
    assert!(fact_rows(fixture.connection(), "PRAGMA foreign_key_check").is_empty());
    assert_rejected_without_repair(&mut fixture, &config, &intent);
}
