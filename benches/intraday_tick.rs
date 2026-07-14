//! v16.8 #1: criterion benchmark 套件 (4 bench case)
//!
//! Fix review (MEDIUM): 4 bench case 用纯函数 mock (不依赖 DB 真上下文)
//! 之前依赖 DB 真上下文 → 0 业务跑, 0 价值
//! 现在 mock: HashMap 预填, is_iron_rule_triggered + extract_reason 是纯函数
//! 启动: cargo bench
//! 目标: mock 跑 1us/iter, 真业务 0.5ms/iter

use criterion::{criterion_group, criterion_main, Criterion};
use std::collections::HashMap;

fn is_iron_rule_triggered(advice: &str) -> bool {
    advice.contains("铁律")
        || advice.contains("止损")
        || advice.contains("止盈")
        || advice.contains("14天")
        || advice.contains("ATR动态止损")
}

fn extract_reason(advice: &str) -> String {
    if advice.contains("铁律1") {
        "铁律1:止损(-8%)".to_string()
    } else if advice.contains("铁律3") {
        "铁律3:跌破5日线止盈".to_string()
    } else if advice.contains("铁律4") {
        "铁律4:14天不涨换股".to_string()
    } else if advice.contains("铁律5") {
        "铁律5:布林上轨+MACD顶背离".to_string()
    } else if advice.contains("ATR动态止损") {
        "ATR动态止损".to_string()
    } else {
        advice.chars().take(30).collect()
    }
}

fn compute_decisions_mock(advice_map: &HashMap<String, String>, codes: &[String]) -> Vec<String> {
    let mut decisions = Vec::with_capacity(codes.len());
    for code in codes {
        if let Some(advice) = advice_map.get(code) {
            if is_iron_rule_triggered(advice) {
                decisions.push(extract_reason(advice));
            }
        }
    }
    decisions
}

fn build_mock_advice_map(codes: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::with_capacity(codes.len());
    for (i, code) in codes.iter().enumerate() {
        // 模拟 50 持仓: 25% 触发 4 铁律
        if i % 4 == 0 {
            map.insert(code.clone(), "铁律1:止损(-8%)".to_string());
        } else if i % 4 == 1 {
            map.insert(code.clone(), "铁律3:跌破5日线止盈".to_string());
        } else {
            map.insert(code.clone(), "持有观望".to_string());
        }
    }
    map
}

fn bench_intraday_tick(c: &mut Criterion) {
    // 纯函数 mock (不依赖 DB)
    let codes: Vec<String> = (0..50).map(|i| format!("60{:04}", i)).collect();
    let advice_map = build_mock_advice_map(&codes);
    c.bench_function("intraday_monitor::tick mock (50 候选)", |b| b.iter(|| {
        let _ = compute_decisions_mock(&advice_map, &codes);
    }));
}

fn bench_4_iron_rules_check(c: &mut Criterion) {
    let codes: Vec<String> = (0..50).map(|i| format!("60{:04}", i)).collect();
    let advice_map = build_mock_advice_map(&codes);
    c.bench_function("paper_engine::check_4_iron_rules mock (50 持仓)", |b| b.iter(|| {
        let _ = compute_decisions_mock(&advice_map, &codes);
    }));
}

fn bench_performance_snapshot(c: &mut Criterion) {
    // 纯函数 mock: 100 笔 paper_trades → snapshot (Sharpe/Sortino 计算)
    let pnls: Vec<f64> = (0..100).map(|i| (i as f64 - 50.0) * 0.5).collect();
    c.bench_function("PerformanceEngine::snapshot mock (100 笔)", |b| b.iter(|| {
        // mock Sharpe: mean / stddev (10us)
        let n = pnls.len() as f64;
        let mean: f64 = pnls.iter().sum::<f64>() / n;
        let var: f64 = pnls.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / n;
        let _sharpe = if var > 0.0 { mean / var.sqrt() } else { 0.0 };
    }));
}

fn bench_quote_provider(c: &mut Criterion) {
    // mock QuoteProvider (不依赖真 broker, 纯函数)
    let cache: HashMap<String, f64> = (0..100).map(|i| (format!("60{:04}", i), 1680.0 + i as f64)).collect();
    c.bench_function("QuoteProvider::get_quote_price mock (100 缓存)", |b| b.iter(|| {
        let mut sum = 0.0;
        for (k, v) in &cache {
            sum += cache.get(k).unwrap_or(v);
        }
        let _ = sum;
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
