//! Production pure veto observations; fixture/chain construction is outside timing.
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use stock_analysis::risk::{
    veto_chain::{VetoChainConfig, VetoContext},
    veto_execution_report_v1::RuleExecutionStatusV1,
    veto_rules_live::build_chain,
};

fn bench_veto(c: &mut Criterion) {
    let chain = build_chain(&VetoChainConfig::default()).unwrap();
    let context = VetoContext {
        code: "TEST_CODE_BENCH".into(),
        current_price: 10.,
        signal_score: 80,
        is_buy_signal: true,
        bias_ma5: 1.,
        is_bearish: false,
        money_flow_days: Some(vec![stock_analysis::capital_flow::MoneyFlowDay {
            date: "2026-10-08".into(),
            main_net: 0.,
            xl_net: 0.,
            big_net: 0.,
            main_pct: 0.,
            pct_chg: Some(0.),
        }]),
        pct_chg: Some(0.),
        pe_ratio: Some(20.),
        net_profit_yoy: Some(10.),
    };
    assert!(!chain.evaluate_all(&context).force_hold);
    let veto = VetoContext {
        bias_ma5: 8.,
        ..context.clone()
    };
    assert!(chain.evaluate_all(&veto).force_hold);
    let missing = VetoContext {
        money_flow_days: None,
        pe_ratio: None,
        net_profit_yoy: None,
        ..context.clone()
    };
    assert!(chain
        .evaluate_observed(&context)
        .rules
        .iter()
        .all(|r| r.status == RuleExecutionStatusV1::EvaluatedClear));
    assert!(chain
        .evaluate_observed(&missing)
        .rules
        .iter()
        .any(|r| r.status == RuleExecutionStatusV1::InputMissing));
    let off = VetoChainConfig {
        enabled: false,
        ..VetoChainConfig::default()
    };
    assert!(build_chain(&off).is_none());
    for (name, fixture) in [
        ("pass_available_inputs", context),
        ("bias_veto", veto),
        ("missing_flow_and_fundamentals", missing),
    ] {
        c.bench_function(&format!("production_veto/{name}"), |b| {
            b.iter(|| black_box(chain.evaluate_observed(black_box(&fixture))))
        });
    }
    c.bench_function("production_veto/config_off", |b| {
        b.iter(|| black_box(build_chain(black_box(&off))))
    });
}
criterion_group!(
    benches,
    bench_veto,
    bench_prediction_due_page,
    bench_daily_batch
);
criterion_main!(benches);

// These benchmarks call the same public DatabaseManager methods as the monitor.
// Only fixture DDL and input records are constructed here; there is no alternate
// query/UPSERT implementation. No singleton or production-root initialization.
const PREDICTION_ROWS: i32 = 8192;
const PREDICTION_PAGE: i64 = 256;
const DAILY_BATCH: usize = 64;

fn frozen_table_ddl(table: &str) -> Vec<String> {
    let table_prefix = format!("CREATE TABLE {table} (");
    let index_owner = format!(" ON {table}(");
    include_str!("../src/database/fixtures/global_schema_legacy_ddl_v1.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (_, bytes) = line.split_once('|').unwrap();
            let sql = String::from_utf8(hex::decode(bytes).unwrap()).unwrap();
            (sql.starts_with(&table_prefix) || sql.contains(&index_owner)).then_some(sql)
        })
        .collect()
}

/// Reuse the actual existing DDL text needed by save_daily_batch's provenance
/// invalidation. A changed source shape fails fixture setup rather than quietly
/// measuring a mirrored schema or omitting the real DELETE path.
fn daily_status_ddl() -> &'static str {
    let source = include_str!("../src/database/mod.rs");
    let start = source
        .find("CREATE TABLE IF NOT EXISTS qualified_daily_trading_status (")
        .expect("existing daily status DDL");
    let sql = &source[start..];
    &sql[..sql
        .find("\n            \"#,")
        .expect("existing DDL raw-string boundary")]
}

fn create_fixture(path: &std::path::Path, table: &str) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open(path).unwrap();
    let ddl = frozen_table_ddl(table);
    assert!(!ddl.is_empty(), "frozen native table DDL must be present");
    for statement in ddl {
        connection.execute_batch(&statement).unwrap();
    }
    let application: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .unwrap();
    let generation: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!((application, generation), (0, 0));
    connection
}

fn bench_prediction_due_page(c: &mut Criterion) {
    use stock_analysis::database::attribution_reports::{
        AttributionDatabaseAccess, AttributionDatabaseSession,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("TEST_CODE_BENCH_prediction.db");
    let mut connection = create_fixture(&path, "prediction_tracker");
    {
        let transaction = connection.transaction().unwrap();
        {
            let mut insert = transaction.prepare("INSERT INTO prediction_tracker(id,pred_date,target_date,stock_code,pred_direction,pred_score,hit) VALUES(?1,'2026-10-08',?2,?3,'up',80,?4)").unwrap();
            for id in 1..=PREDICTION_ROWS {
                let target = if id % 2 == 0 {
                    "2026-10-09"
                } else {
                    "2026-10-12"
                };
                let hit = (id % 4 == 0).then_some(1);
                insert
                    .execute(rusqlite::params![
                        id,
                        target,
                        format!("TEST_CODE_BENCH_{id:06}"),
                        hit
                    ])
                    .unwrap();
            }
        }
        transaction.commit().unwrap();
    }
    drop(connection);
    // Public ReadOnly constructor creates an immutable detached query-only pool;
    // copying and opening the snapshot are outside the timed page operations.
    let session =
        AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::ReadOnly).unwrap();
    let database = session.database();
    let high_water = database.prediction_verification_high_water_id().unwrap();
    assert_eq!(high_water, PREDICTION_ROWS);
    for (name, after_id) in [("first_page", 0), ("middle_page", PREDICTION_ROWS / 2)] {
        let page = database
            .get_due_predictions_page("2026-10-09", after_id, high_water, PREDICTION_PAGE)
            .unwrap();
        assert_eq!(page.len(), PREDICTION_PAGE as usize);
        assert!(page.iter().all(|row| row.id > after_id
            && row.id <= high_water
            && row.target_date == "2026-10-09"
            && row.hit.is_none()));
        c.bench_function(
            &format!("production_sqlite/prediction_due_page_rows8192_page256/{name}"),
            |b| {
                b.iter(|| {
                    black_box(
                        database
                            .get_due_predictions_page(
                                black_box("2026-10-09"),
                                black_box(after_id),
                                black_box(high_water),
                                black_box(PREDICTION_PAGE),
                            )
                            .unwrap(),
                    )
                })
            },
        );
    }
}

struct DailyFixture {
    // Explicit drop order: the actual connection pools/descriptors close before
    // removing the closed fixture directory and its WAL/SHM files.
    session: stock_analysis::database::attribution_reports::AttributionDatabaseSession,
    _directory: tempfile::TempDir,
}
impl DailyFixture {
    fn new() -> Self {
        use stock_analysis::database::attribution_reports::{
            AttributionDatabaseAccess, AttributionDatabaseSession,
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("TEST_CODE_BENCH_daily.db");
        let connection = create_fixture(&path, "stock_daily");
        connection.execute_batch(daily_status_ddl()).unwrap();
        drop(connection);
        // The only existing public non-singleton write manager constructor.
        // Its four scoped attribution schema groups and WAL bootstrap are real,
        // disclosed setup dependencies and are excluded from batch-write timing.
        let session =
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::AppendOnly).unwrap();
        Self {
            session,
            _directory: directory,
        }
    }
}

fn daily_records() -> Vec<stock_analysis::database::StockDailyRecord> {
    (0..DAILY_BATCH)
        .map(|index| stock_analysis::database::StockDailyRecord {
            code: format!("TEST_CODE_BENCH_{index:06}"),
            date: chrono::NaiveDate::from_ymd_opt(2026, 10, 9).unwrap(),
            open: Some(10.),
            high: Some(10.5),
            low: Some(9.5),
            close: Some(10.1),
            volume: Some(10000.),
            amount: Some(101000.),
            pct_chg: Some(1.),
            ma5: Some(10.),
            ma10: Some(10.),
            ma20: Some(10.),
            volume_ratio: Some(1.),
            data_source: Some("TEST_CODE_BENCH_observed_only_not_qualified".into()),
        })
        .collect()
}

fn bench_daily_batch(c: &mut Criterion) {
    use criterion::{BatchSize, Throughput};
    let records = daily_records();
    // Confirm that the real public operation accepts this minimal native schema
    // before timing. Fixtures are never real holdings or market qualifications.
    let check = DailyFixture::new();
    assert_eq!(
        check.session.database().save_daily_batch(&records).unwrap(),
        DAILY_BATCH
    );
    drop(check);
    let mut group = c.benchmark_group("production_sqlite/daily_batch_native0_append_pool_insert");
    group.throughput(Throughput::Elements(DAILY_BATCH as u64));
    group.bench_function("rows64", |b| {
        b.iter_batched(
            DailyFixture::new,
            |fixture| {
                let count = fixture
                    .session
                    .database()
                    .save_daily_batch(black_box(&records))
                    .unwrap();
                black_box(count);
                // Return ownership so the pools/tempdir teardown is not measured.
                // PerIteration retains at most one DB; no growing shared fixture.
                fixture
            },
            BatchSize::PerIteration,
        )
    });
    group.finish();
}
