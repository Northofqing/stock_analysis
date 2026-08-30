//! 修复 Top10#3+#4 (2026-06-29 audit): pipeline/mod.rs (1765 行) 拆 4 个子模块
//!
//! 这个文件: `pipeline/data.rs` — 数据获取 + 持久化 (fetch_and_save_data)
//!
//! 原 pipeline/mod.rs impl AnalysisPipeline 块 (~1500 行) 主要由 4 个方法组成:
//!   - fetch_and_save_data (52 行)   → 本文件
//!   - analyze_stock       (897 行)  → pipeline/analyze.rs
//!   - run / process_stock  (~250 行) → pipeline/run.rs
//!   - enrich_key_stocks   (~90 行) → 留在 mod.rs (与 run 关联)
//!
//! 拆分后 mod.rs 只剩 ~600 行 (struct 定义 + new/with_limit_up_codes + 入口 run).
//!
//! Rust 允许跨模块 impl, 所以这里直接 `impl AnalysisPipeline { ... }`.

use anyhow::{Context, Result};
use log::{info, warn};

use crate::data_gateway::{AdmittedDailyBars, HistoricalBarsGateway};
use crate::data_provider::KlineData;
use crate::database::DatabaseManager;
use crate::monitor::data_quality::{validate_daily_freshness, DqStats, FreshnessConfig};

use super::AnalysisPipeline;

impl AnalysisPipeline {
    /// 获取单只股票的日线数据 + 保存到数据库
    ///
    /// 修复 v9.1 §0: 数据获取走 spawn_blocking, 避免同步 HTTP 阻塞 tokio worker.
    /// 修复 v9.2 R-3: 日线新鲜度校验, 跨日断层阻断推送.
    pub(super) async fn fetch_and_save_data(&self, code: &str) -> Result<Vec<KlineData>> {
        info!("[{}] 开始获取数据...", code);

        // HistoricalBarsGateway owns provider construction, routing,
        // validation and blocking isolation. Requiring AdmittedDailyBars keeps
        // the records inseparable from the batch evidence that admitted them.
        let batch = HistoricalBarsGateway::new()
            .required_daily_bars_async(code, 30)
            .await
            .with_context(|| format!("[{code}] 获取统一日线批次失败"))?;
        crate::monitor::data_mode::mark_capability_success(
            crate::monitor::data_mode::Capability::Kline,
        )
        .map_err(anyhow::Error::msg)?;

        self.finalize_fetched_data(code, &batch, chrono::Local::now())
    }

    fn finalize_fetched_data(
        &self,
        code: &str,
        batch: &AdmittedDailyBars,
        observed_at: chrono::DateTime<chrono::Local>,
    ) -> Result<Vec<KlineData>> {
        let data = batch.records();
        let evidence = batch.evidence();

        // AGENTS 2.4: 日线/历史数据超过 1 个交易日直接阻断。
        let latest_date = data[0].date;
        let freshness = FreshnessConfig {
            quote_max_age_secs: self.config.dq_quote_stale_sec,
            position_max_age_secs: self.config.dq_position_stale_sec,
            nav_max_age_secs: self.config.dq_nav_stale_sec,
            daily_max_age_secs: self.config.dq_daily_stale_sec,
        };
        let dq_stats = DqStats::new();
        if let Err(reason) =
            validate_daily_freshness(latest_date, observed_at, &freshness, &dq_stats)
        {
            anyhow::bail!(
                "[{}] 日线新鲜度校验失败: {} (latest_date={})",
                code,
                reason.label(),
                latest_date
            );
        }

        // Codex review P0 #2 修复: 删掉 pipeline 路径的二次质检.
        // 校验统一由 HistoricalBarsGateway admission 完成，pipeline
        // 拿到的就是已通过质检且保留批次证据的 data。

        info!(
            "[{}][BR-159] 日线证据 provider={:?} source={} source_at={} observed_at={} batch_id={} records={}",
            code,
            evidence.provider,
            evidence.source,
            evidence.source_at.as_deref().unwrap_or("absent"),
            evidence.observed_at,
            evidence.batch_id,
            data.len()
        );

        // 保存到数据库
        if let Some(db) = DatabaseManager::try_get() {
            match db.save_kline_data(code, data, &evidence.source) {
                Ok(count) => info!("[{}] 已保存 {} 条K线数据到数据库", code, count),
                Err(e) => warn!("[{}] 保存K线数据到数据库失败: {}", code, e),
            }
        }

        Ok(data.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::BatchEvidence;
    use crate::data_provider::AdjustType;
    use crate::market_domain::ProviderId;

    fn kline(date: chrono::NaiveDate) -> KlineData {
        KlineData {
            date,
            open: 10.0,
            high: 10.2,
            low: 9.9,
            close: 10.1,
            volume: 1_000.0,
            amount: 10_000.0,
            pct_chg: 1.0,
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
            adjust: AdjustType::Qfq,
        }
    }

    fn admitted(records: Vec<KlineData>) -> AdmittedDailyBars {
        AdmittedDailyBars::from_test_fixture(
            "TEST_CODE_600000",
            records,
            BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_magic_tdx_daily".to_string(),
                source_at: Some("2026-07-26T01:00:00Z".to_string()),
                observed_at: "2026-07-26T01:00:01Z".to_string(),
                batch_id: "TEST_CODE_daily_batch".to_string(),
            },
        )
        .expect("TEST_CODE admitted daily batch")
    }

    #[test]
    fn resolved_daily_batch_rejects_empty_and_stale_evidence() {
        let pipeline = AnalysisPipeline::new(super::super::PipelineConfig::default()).unwrap();
        let now = chrono::Local::now();
        assert!(AdmittedDailyBars::from_test_fixture(
            "TEST_CODE_600000",
            Vec::new(),
            BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_magic_tdx_empty".to_string(),
                source_at: Some("2026-07-26T01:00:00Z".to_string()),
                observed_at: "2026-07-26T01:00:01Z".to_string(),
                batch_id: "TEST_CODE_empty_batch".to_string(),
            }
        )
        .is_err());

        let stale = now.date_naive() - chrono::Duration::days(30);
        let batch = admitted(vec![kline(stale)]);
        assert!(pipeline
            .finalize_fetched_data("TEST_CODE_STALE", &batch, now)
            .unwrap_err()
            .to_string()
            .contains("日线新鲜度校验失败"));
    }

    #[test]
    #[serial_test::serial(database)]
    fn resolved_fresh_daily_batch_reaches_existing_persistence_boundary() {
        let pipeline = AnalysisPipeline::new(super::super::PipelineConfig::default()).unwrap();
        let now = chrono::Local::now();
        let latest = crate::calendar::recent_trading_days(now.date_naive(), 1)[0];
        let data = vec![kline(latest)];
        let batch = admitted(data.clone());
        let resolved = pipeline
            .finalize_fetched_data("TEST_CODE_PIPELINE_DATA", &batch, now)
            .unwrap();
        assert_eq!(resolved[0].date, data[0].date);
        assert_eq!(resolved[0].close, 10.1);
    }
}
