//! v16.6 #2: Prometheus metrics (6 metric, 暴露 :9090/metrics)
//!
//! 6 metric:
//!   1. intraday_monitor_candidates_total (gauge, 30s tick 候选数)
//!   2. intraday_monitor_tick_duration_seconds (histogram, tick 耗时)
//!   3. strategy_score_total{strategy} (counter, 各 strategy 评分次数)
//!   4. paper_engine_sell_total{reason} (counter, 4 铁律触发次数)
//!   5. performance_snapshot_age_seconds (gauge, 距上次 snapshot 时间)
//!   6. quote_provider_price{code} (counter, 当前价, 60s TTL)
//!
//! run: cargo build --release + ./target/release/monitor --test
//!      curl http://localhost:9090/metrics

use prometheus::{IntGauge, Histogram, IntCounterVec, Registry, TextEncoder};

pub struct MonitorMetrics {
    pub registry: Registry,
    pub intraday_candidates: IntGauge,
    pub tick_duration: Histogram,
    pub strategy_score: IntCounterVec,
    pub paper_engine_sell: IntCounterVec,
    pub perf_snapshot_age: IntGauge,
    pub quote_price: IntCounterVec,
}

impl MonitorMetrics {
    pub fn new() -> Result<Self, String> {
        let registry = Registry::new();
        let intraday_candidates = IntGauge::new(
            "intraday_monitor_candidates_total",
            "30s tick 候选数",
        ).map_err(|e| e.to_string())?;
        let tick_duration = Histogram::with_opts(
            prometheus::HistogramOpts::new("intraday_monitor_tick_duration_seconds", "tick 耗时")
        ).map_err(|e| e.to_string())?;
        let strategy_score = IntCounterVec::new(
            prometheus::Opts::new("strategy_score_total", "strategy 评分次数"),
            &["strategy"],
        ).map_err(|e| e.to_string())?;
        let paper_engine_sell = IntCounterVec::new(
            prometheus::Opts::new("paper_engine_sell_total", "4 铁律触发次数"),
            &["reason"],
        ).map_err(|e| e.to_string())?;
        let perf_snapshot_age = IntGauge::new(
            "performance_snapshot_age_seconds",
            "距上次 snapshot 秒数",
        ).map_err(|e| e.to_string())?;
        let quote_price = IntCounterVec::new(
            prometheus::Opts::new("quote_provider_price", "当前价 (60s TTL)"),
            &["code"],
        ).map_err(|e| e.to_string())?;
        registry.register(Box::new(intraday_candidates.clone())).map_err(|e| e.to_string())?;
        registry.register(Box::new(tick_duration.clone())).map_err(|e| e.to_string())?;
        registry.register(Box::new(strategy_score.clone())).map_err(|e| e.to_string())?;
        registry.register(Box::new(paper_engine_sell.clone())).map_err(|e| e.to_string())?;
        registry.register(Box::new(perf_snapshot_age.clone())).map_err(|e| e.to_string())?;
        registry.register(Box::new(quote_price.clone())).map_err(|e| e.to_string())?;
        Ok(Self { registry, intraday_candidates, tick_duration, strategy_score, paper_engine_sell, perf_snapshot_age, quote_price })
    }

    pub fn expose(&self) -> Result<String, String> {
        let mut buffer = Vec::new();
        let encoder = TextEncoder::new();
        encoder.encode(&self.registry.gather(), &mut buffer).map_err(|e| e.to_string())?;
        Ok(String::from_utf8(buffer).map_err(|e| e.to_string())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_new_succeeds() {
        let m = MonitorMetrics::new().expect("MonitorMetrics::new ok");
        let s = m.expose().expect("expose ok");
        assert!(s.contains("intraday_monitor_candidates_total"));
        assert!(s.contains("intraday_monitor_tick_duration_seconds"));
        assert!(s.contains("strategy_score_total"));
        assert!(s.contains("paper_engine_sell_total"));
        assert!(s.contains("performance_snapshot_age_seconds"));
        assert!(s.contains("quote_provider_price"));
    }

    #[test]
    fn metrics_increment_counters() {
        let m = MonitorMetrics::new().unwrap();
        m.intraday_candidates.set(42);
        m.strategy_score.with_label_values(&["Momentum"]).inc();
        m.strategy_score.with_label_values(&["Momentum"]).inc();
        m.paper_engine_sell.with_label_values(&["铁律1:止损(-8%)"]).inc();
        m.perf_snapshot_age.set(3600);
        m.quote_price.with_label_values(&["600519"]).set(1700);
        let s = m.expose().unwrap();
        assert!(s.contains("42"));
        assert!(s.contains("3600"));
        assert!(s.contains("1700"));
    }
}
