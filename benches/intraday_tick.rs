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
criterion_group!(benches, bench_veto);
criterion_main!(benches);
