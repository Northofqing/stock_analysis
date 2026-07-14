//! v16.8 #1: criterion benchmark 套件 (关键路径基线)
//!
//! 4 bench case: intraday_monitor tick / 4 铁律 / PerformanceSnapshot / QuoteProvider
//! 启动: cargo bench
//! 目标: intraday_tick < 5s, 4 铁律 < 100ms, snapshot < 500ms
//! HTML 报告: target/criterion/report/index.html

use criterion::{criterion_group, criterion_main, Criterion};
use stock_analysis::decision::intraday_monitor::IntradayMonitor;
use stock_analysis::trading::paper_engine::{check_4_iron_rules, PaperPositionSellCheck};
use stock_analysis::performance::compute_snapshot;
use stock_analysis::broker::MockQuoteProvider;
use stock_analysis::broker::QuoteProvider;
use chrono::Local;

fn make_check(code: &str, qty: u32, cost: f64) -> PaperPositionSellCheck {
    PaperPositionSellCheck {
        code: code.to_string(),
        name: code.to_string(),
        avg_cost: cost,
        quantity: qty,
        current_price: cost,
    }
}

fn bench_intraday_tick(c: &mut Criterion) {
    let monitor = IntradayMonitor;
    c.bench_function("intraday_monitor::tick (DB 0 候选)", |b| b.iter(|| {
        let _ = monitor.tick();
    }));
}

fn bench_4_iron_rules_check(c: &mut Criterion) {
    let checks: Vec<PaperPositionSellCheck> = (0..50)
        .map(|i| make_check(&format!("60{:04}", i), 1000, 1680.0))
        .collect();
    c.bench_function("paper_engine::check_4_iron_rules (50 持仓)", |b| b.iter(|| {
        let _ = check_4_iron_rules(&checks);
    }));
}

fn bench_performance_snapshot(c: &mut Criterion) {
    c.bench_function("PerformanceEngine::snapshot (DB 0 笔)", |b| b.iter(|| {
        let _ = compute_snapshot(Local::now().date_naive());
    }));
}

fn bench_quote_provider(c: &mut Criterion) {
    let provider = MockQuoteProvider;
    c.bench_function("QuoteProvider::get_quote_price (mock)", |b| b.iter(|| {
        let _ = provider.get_quote_price("600519");
        let _ = provider.get_sector("600519");
    }));
}

criterion_group!(
    benches,
    bench_intraday_tick,
    bench_4_iron_rules_check,
    bench_performance_snapshot,
    bench_quote_provider
);
criterion_main!(benches);
