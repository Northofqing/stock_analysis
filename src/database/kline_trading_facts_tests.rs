use super::*;
use crate::data_gateway::qualified_trading_facts::{
    AuthorityLifecycle, AuthoritySuspensionCoverage, AuthorityTradingFactsRecord,
    QualifiedTradingFactsRequest, SuspensionWindow,
};
use crate::data_gateway::{BatchEvidence, QualifiedTradingFactsGateway};
use crate::market_domain::{AssetClass, Exchange, InstrumentId, ProviderId};

const CODE: &str = "TEST_CODE_daily_authority";
fn day(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
}
fn database() -> (tempfile::TempDir, DatabaseManager) {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_daily_facts.db"))
        .unwrap();
    (dir, db)
}
fn batch() -> AdmittedDailyBars {
    let bars = ["2026-09-28", "2026-09-29"].map(|date| crate::data_provider::KlineData {
        date: day(date),
        open: 10.,
        high: 11.,
        low: 9.,
        close: 10.,
        volume: 100.,
        amount: 1000.,
        pct_chg: 0.,
        adjust: crate::data_provider::AdjustType::None,
        intraday_price: None,
        settled: true,
        pe_ratio: None,
        pb_ratio: None,
        turnover_rate: None,
        market_cap: None,
        circulating_cap: None,
        eps: None,
        roe: None,
        revenue_yoy: None,
        net_profit_yoy: None,
        gross_margin: None,
        net_margin: None,
        sharpe_ratio: None,
        financials_history: None,
        valuation_history: None,
        consensus: None,
        industry: None,
        is_limit_up: false,
        is_limit_down: false,
        is_suspended: false,
    });
    AdmittedDailyBars::from_test_fixture(
        CODE,
        bars.to_vec(),
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_bars".into(),
            source_at: Some("2026-09-29".into()),
            observed_at: "2026-09-29T07:01:00Z".into(),
            batch_id: "TEST_CODE_bars_batch".into(),
        },
    )
    .unwrap()
}
fn fact(code: &str, date: &str, suspended: bool) -> QualifiedTradingFacts {
    let effective = day(date);
    let instrument = InstrumentId::new(Exchange::Shenzhen, code, AssetClass::Equity).unwrap();
    QualifiedTradingFacts::admit(
        QualifiedTradingFactsRequest::new(instrument.clone(), effective),
        AuthorityTradingFactsRecord {
            instrument,
            lifecycle: Some(AuthorityLifecycle {
                listed_on: day("2020-01-01"),
                delisted_on: None,
                covered_through: effective,
            }),
            price_regime: None,
            suspension: Some(AuthoritySuspensionCoverage::with_windows(
                effective,
                effective,
                if suspended {
                    vec![SuspensionWindow {
                        halted_from: effective,
                        halted_through: effective,
                    }]
                } else {
                    vec![]
                },
            )),
            evidence: BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_independent_authority".into(),
                source_at: Some("2026-09-29T07:00:00Z".into()),
                observed_at: "2026-09-29T07:01:00Z".into(),
                batch_id: "TEST_CODE_authority_batch".into(),
            },
            contract_version: "TEST_CODE_explicit_daily_status_v1".into(),
            fresh_through: effective,
        },
    )
    .unwrap()
}
#[derive(diesel::QueryableByName, Debug, PartialEq)]
struct BoundRow {
    #[diesel(sql_type=diesel::sql_types::Text)]
    date: String,
    #[diesel(sql_type=diesel::sql_types::Text)]
    status: String,
    #[diesel(sql_type=diesel::sql_types::Integer)]
    is_suspended: i32,
    #[diesel(sql_type=diesel::sql_types::Text)]
    source: String,
}
fn rows(db: &DatabaseManager) -> Vec<BoundRow> {
    diesel::sql_query("SELECT d.date,s.status,d.is_suspended,s.source FROM stock_daily d JOIN qualified_daily_trading_status s ON s.code=d.code AND s.date=d.date ORDER BY d.date")
        .load(&mut db.get_conn().unwrap()).unwrap()
}
#[test]
fn qualified_daily_authority_binds_exact_keys_independent_source_and_state_transition() {
    let (_dir, db) = database();
    let batch = batch();
    db.save_admitted_kline_with_trading_facts(
        &batch,
        &[
            fact(CODE, "2026-09-29", true),
            fact(CODE, "2026-09-28", false),
        ],
    )
    .unwrap();
    let first = rows(&db);
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].status, "trading");
    assert_eq!(first[1].is_suspended, 1);
    assert!(first
        .iter()
        .all(|r| r.source == "TEST_CODE_independent_authority"));
    db.save_admitted_kline_with_trading_facts(
        &batch,
        &[
            fact(CODE, "2026-09-28", false),
            fact(CODE, "2026-09-29", false),
        ],
    )
    .unwrap();
    assert!(rows(&db)
        .iter()
        .all(|r| r.status == "trading" && r.is_suspended == 0));
    db.save_admitted_kline_data(&batch).unwrap();
    assert!(rows(&db).is_empty());
}
#[test]
fn qualified_daily_authority_rejects_missing_duplicate_wrong_identity_and_unavailable_contract_before_write(
) {
    let (_dir, db) = database();
    let batch = batch();
    let unavailable =
        QualifiedTradingFactsGateway::new().acquire(QualifiedTradingFactsRequest::new(
            InstrumentId::new(Exchange::Shenzhen, CODE, AssetClass::Equity).unwrap(),
            day("2026-09-29"),
        ));
    for facts in [
        vec![fact(CODE, "2026-09-28", false)],
        vec![
            fact(CODE, "2026-09-28", false),
            fact(CODE, "2026-09-28", false),
        ],
        vec![
            fact(CODE, "2026-09-28", false),
            fact("TEST_CODE_other", "2026-09-29", false),
        ],
        vec![fact(CODE, "2026-09-28", false), unavailable],
    ] {
        assert!(db
            .save_admitted_kline_with_trading_facts(&batch, &facts)
            .is_err());
        assert!(db
            .get_data_range(CODE, day("2026-09-28"), day("2026-09-29"))
            .unwrap()
            .is_empty());
        assert!(rows(&db).is_empty());
    }
}
#[test]
fn qualified_daily_authority_second_state_failure_rolls_back_bars_and_previous_markers() {
    let (_dir, db) = database();
    let batch = batch();
    let facts = [
        fact(CODE, "2026-09-28", false),
        fact(CODE, "2026-09-29", false),
    ];
    db.save_admitted_kline_with_trading_facts(&batch, &facts)
        .unwrap();
    let previous = rows(&db);
    diesel::sql_query("CREATE TRIGGER TEST_CODE_deny_second_state BEFORE INSERT ON qualified_daily_trading_status WHEN NEW.date='2026-09-29' BEGIN SELECT RAISE(ABORT,'TEST_CODE_status_failure'); END")
        .execute(&mut db.get_conn().unwrap()).unwrap();
    assert!(db
        .save_admitted_kline_with_trading_facts(
            &batch,
            &[
                fact(CODE, "2026-09-28", true),
                fact(CODE, "2026-09-29", true)
            ]
        )
        .is_err());
    assert_eq!(rows(&db), previous);
}
