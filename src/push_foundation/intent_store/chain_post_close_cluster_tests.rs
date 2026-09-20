use std::cell::Cell;

use super::*;
use crate::pipeline::chain_analysis::preparation::FixedClusterConfiguration;

#[path = "chain_post_close_cluster_maintenance_tests.rs"]
mod maintenance_tests;

pub(super) const MAIN_CONCEPT: &str = "TEST_CODE_CLUSTER_A_MAIN";
const ALIAS_CONCEPT: &str = "TEST_CODE_CLUSTER_B_ALIAS";

pub(super) struct PanicRawProvider {
    pub(super) calls: Cell<usize>,
}

#[async_trait::async_trait(?Send)]
impl ConceptProviderRawIo for PanicRawProvider {
    async fn call_raw(&self, code: &str) -> std::result::Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        panic!("full cache hit must not call concept provider for {code}")
    }
}

pub(super) fn cluster_stocks() -> Vec<TopStock> {
    [
        ("TEST_CODE_CLUSTER_STOCK_A", 9.91),
        ("TEST_CODE_CLUSTER_STOCK_B", 9.73),
        ("TEST_CODE_CLUSTER_STOCK_ISOLATED", 9.52),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (code, change_pct))| TopStock {
        code: code.to_owned(),
        name: format!("TEST_CODE_CLUSTER_NAME_{index}"),
        change_pct,
        price: 20.0 + index as f64,
        ..TopStock::default()
    })
    .collect()
}

pub(super) fn install_business_rows(fixture: &V2BusinessFixture) {
    fixture.execute(
        "CREATE TABLE chain_daily ( \
           date TEXT NOT NULL, \
           concept TEXT NOT NULL, \
           stocks TEXT NOT NULL, \
           continuation_count INTEGER NOT NULL DEFAULT 0, \
           created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
           PRIMARY KEY (date, concept) \
         ); \
         INSERT INTO stock_concepts(code,concepts,updated_at) VALUES \
           ('TEST_CODE_CLUSTER_STOCK_A', \
            '[\"TEST_CODE_CLUSTER_A_MAIN\",\"TEST_CODE_CLUSTER_B_ALIAS\",\"昨日涨停\"]', \
            '2026-07-21 14:00:00'), \
           ('TEST_CODE_CLUSTER_STOCK_B', \
            '[\"TEST_CODE_CLUSTER_A_MAIN\",\"TEST_CODE_CLUSTER_B_ALIAS\"]', \
            '2026-07-21 14:00:00'), \
           ('TEST_CODE_CLUSTER_STOCK_ISOLATED', \
            '[\"TEST_CODE_CLUSTER_Z_ISOLATED\"]', \
            '2026-07-21 14:00:00'); \
         INSERT INTO chain_daily(date,concept,stocks,continuation_count) VALUES \
           ('2026-07-11','TEST_CODE_CLUSTER_A_MAIN','[\"TEST_CODE_OUTSIDE\"]',0), \
           ('2026-07-12','TEST_CODE_CLUSTER_A_MAIN','[\"TEST_CODE_CUTOFF\"]',0), \
           ('2026-07-15','TEST_CODE_CLUSTER_A_MAIN','[\"TEST_CODE_WINDOW\"]',0), \
           ('2026-07-21','TEST_CODE_CLUSTER_A_MAIN','[\"TEST_CODE_OLD_CURRENT\"]',8), \
           ('2026-07-22','TEST_CODE_CLUSTER_A_MAIN','[\"TEST_CODE_FUTURE\"]',0), \
           ('2026-07-21','TEST_CODE_UNRELATED_SAME_DAY','[\"TEST_CODE_UNRELATED\"]',7);",
    );
}

fn chain_daily_row(fixture: &V2BusinessFixture, date: &str, concept: &str) -> (String, i64) {
    fixture
        .connection()
        .query_row(
            "SELECT stocks,continuation_count FROM chain_daily \
             WHERE date=?1 AND concept=?2",
            [date, concept],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

fn assert_cluster_inspection(
    inspection: &super::super::ClusterApplicationRecovery,
    expected_material_bytes: Option<&[u8]>,
    expected_lifecycle_bytes: Option<&[u8]>,
) {
    assert_eq!(inspection.min_cluster_size(), 2);
    assert_eq!(inspection.clusters().len(), 1);
    let cluster = &inspection.clusters()[0];
    assert_eq!(cluster.concept, MAIN_CONCEPT);
    assert_eq!(cluster.aliases, [ALIAS_CONCEPT]);
    assert_eq!(
        cluster
            .stocks
            .iter()
            .map(|stock| stock.code.as_str())
            .collect::<Vec<_>>(),
        ["TEST_CODE_CLUSTER_STOCK_A", "TEST_CODE_CLUSTER_STOCK_B"]
    );
    assert_eq!(cluster.continuation_count, 1);
    assert_eq!(cluster.streak_days, 3);
    assert_eq!(inspection.isolated().len(), 1);
    assert_eq!(
        inspection.isolated()[0].code,
        "TEST_CODE_CLUSTER_STOCK_ISOLATED"
    );
    assert_eq!(inspection.lifecycle_days().get(MAIN_CONCEPT), Some(&3));
    if let Some(expected) = expected_material_bytes {
        assert_eq!(inspection.material_bytes(), expected);
    }
    if let Some(expected) = expected_lifecycle_bytes {
        assert_eq!(inspection.lifecycle_bytes(), expected);
    }
}

#[tokio::test]
async fn single_user_local_cluster_material_and_chain_lifecycle_reopen_without_recompute() {
    let mut fixture = V2BusinessFixture::new();
    fixture.install_v2();
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v2_to_v3()
            .unwrap()
            .schema_version(),
        3
    );
    install_business_rows(&fixture);
    assert_eq!(
        fixture
            .chain_post_close()
            .migrate_schema_v3_to_v4()
            .unwrap()
            .schema_version(),
        4
    );

    let stocks = cluster_stocks();
    let config = local_config(BUILD_A);
    let context = build_single_user_local_chain_post_close_context(
        &config,
        run_input("TEST_CODE_RUN_CLUSTER_MATERIAL"),
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
            lease_request("TEST_CODE_CLUSTER_OWNER_A", 1_000, 5_000, None),
        )
        .unwrap();
    let intent_id = lease.intent_id().clone();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(1_100));
    let mut io = local
        .cluster_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let error = prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks.clone(),
        None,
        &mut io,
    )
    .await
    .expect_err("v4 must stop before the unmigrated board directory");
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::BoardDirectory,
        })
    ));
    assert_eq!(provider.calls.get(), 0);
    drop(io);

    let inspection = local.inspect_cluster_application(&intent_id).unwrap();
    assert_cluster_inspection(&inspection, None, None);
    let material_bytes = inspection.material_bytes().to_vec();
    let lifecycle_bytes = inspection.lifecycle_bytes().to_vec();
    let head = local.inspect_run(&intent_id).unwrap().head_version();
    drop(local);
    assert_eq!(
        chain_daily_row(&fixture, BUSINESS_DATE, MAIN_CONCEPT),
        (
            "[\"TEST_CODE_CLUSTER_STOCK_A\",\"TEST_CODE_CLUSTER_STOCK_B\"]".to_owned(),
            1,
        )
    );
    assert_eq!(
        chain_daily_row(&fixture, BUSINESS_DATE, "TEST_CODE_UNRELATED_SAME_DAY"),
        ("[\"TEST_CODE_UNRELATED\"]".to_owned(), 7)
    );

    fixture.execute(
        "UPDATE chain_daily \
         SET stocks='[\"TEST_CODE_LIVE_MUTATION\"]',continuation_count=99 \
         WHERE date='2026-07-21' AND concept='TEST_CODE_CLUSTER_A_MAIN'; \
         INSERT INTO chain_daily(date,concept,stocks,continuation_count) VALUES \
           ('2026-07-20','TEST_CODE_CLUSTER_A_MAIN','[\"TEST_CODE_LATE_HISTORY\"]',4);",
    );
    fixture.reopen();
    let mut local = fixture
        .store
        .as_mut()
        .unwrap()
        .single_user_local_chain_post_close(&config)
        .unwrap();
    let lease = local
        .resume_run(
            &intent_id,
            lease_request("TEST_CODE_CLUSTER_OWNER_B", 5_001, 9_000, Some(head)),
        )
        .unwrap();
    let provider = PanicRawProvider {
        calls: Cell::new(0),
    };
    let clock = ControlledClock::new(at(5_100));
    let mut io = local
        .cluster_preparation_io(
            lease,
            &provider,
            &clock,
            FixedClusterConfiguration::resolve(Some("2")),
        )
        .unwrap();
    let error = prepare_chain_analysis_with_io(
        NaiveDate::from_ymd_opt(2026, 7, 21).unwrap(),
        stocks,
        None,
        &mut io,
    )
    .await
    .expect_err("recovery must stop before the unmigrated board directory");
    assert!(matches!(
        error.downcast_ref::<PreparationStop>(),
        Some(PreparationStop::StageNotMigrated {
            next: UnmigratedStage::BoardDirectory,
        })
    ));
    assert_eq!(provider.calls.get(), 0);
    drop(io);
    let inspection = local.inspect_cluster_application(&intent_id).unwrap();
    assert_cluster_inspection(&inspection, Some(&material_bytes), Some(&lifecycle_bytes));
    drop(local);

    assert_eq!(
        chain_daily_row(&fixture, BUSINESS_DATE, MAIN_CONCEPT),
        ("[\"TEST_CODE_LIVE_MUTATION\"]".to_owned(), 99)
    );
    assert_eq!(
        chain_daily_row(&fixture, "2026-07-20", MAIN_CONCEPT),
        ("[\"TEST_CODE_LATE_HISTORY\"]".to_owned(), 4)
    );
    assert_eq!(
        chain_daily_row(&fixture, BUSINESS_DATE, "TEST_CODE_UNRELATED_SAME_DAY"),
        ("[\"TEST_CODE_UNRELATED\"]".to_owned(), 7)
    );
}
