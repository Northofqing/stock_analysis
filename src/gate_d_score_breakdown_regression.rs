use super::*;
use crate::data_provider::financials::FinancialPeriod;
use crate::data_provider::{AdjustType, KlineData};

fn kline() -> KlineData {
    KlineData {
        date: chrono::NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid fixture date"),
        open: 10.0,
        high: 10.0,
        low: 10.0,
        close: 10.0,
        volume: 1_000.0,
        amount: 10_000.0,
        pct_chg: 0.0,
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
        adjust: AdjustType::None,
    }
}

fn inputs(section: Option<&str>) -> ScoreInputs<'_> {
    ScoreInputs {
        sentiment_score: 50,
        money_flow: None,
        money_flow_section: section,
        volume_ratio_5d: None,
    }
}

#[test]
fn empty_history_and_intermediate_cash_quality_remain_explicit() {
    let mut empty = kline();
    empty.financials_history = Some(Vec::new());
    let score = compute(&inputs(None), &empty);
    assert_eq!(score.fundamental_quality, 50);
    assert_eq!(score.growth_sustainability, 50);

    let mut intermediate = kline();
    intermediate.financials_history = Some(vec![
        FinancialPeriod {
            eps: Some(1.0),
            op_cash_flow_ps: Some(0.7),
            gross_margin: Some(20.0),
            ..FinancialPeriod::default()
        },
        FinancialPeriod {
            eps: Some(1.0),
            op_cash_flow_ps: Some(0.7),
            gross_margin: Some(30.0),
            ..FinancialPeriod::default()
        },
    ]);
    assert_eq!(
        compute(&inputs(None), &intermediate).fundamental_quality,
        90
    );
}

#[test]
fn legacy_money_flow_parser_executes_every_remaining_score_band() {
    for (section, expected) in [
        ("近5日: +1.00亿", 75),
        ("近5日: +0.10亿", 60),
        ("近5日: -0.10亿", 40),
        ("近5日: -3.00亿", 10),
    ] {
        assert_eq!(
            compute(&inputs(Some(section)), &kline()).capital_flow,
            expected,
            "section={section}"
        );
    }
}
