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

use crate::data_provider::KlineData;
use crate::database::DatabaseManager;
use crate::monitor::data_quality::{
    validate_daily_freshness, validate_daily_kline_quality, DqStats, FreshnessConfig,
};

use super::AnalysisPipeline;

impl AnalysisPipeline {
    /// 获取单只股票的日线数据 + 保存到数据库
    ///
    /// 修复 v9.1 §0: 数据获取走 spawn_blocking, 避免同步 HTTP 阻塞 tokio worker.
    /// 修复 v9.2 R-3: 日线新鲜度校验, 跨日断层阻断推送.
    pub(super) async fn fetch_and_save_data(&self, code: &str) -> Result<Vec<KlineData>> {
        info!("[{}] 开始获取数据...", code);

        // 从数据源获取数据
        // 使用 spawn_blocking 将同步 TCP/HTTP 调用放到独立的阻塞线程池，
        // 不占用 tokio worker 线程，避免饿死异步任务（timeout/新闻搜索/AI 调用）。
        let dm = self.data_manager.clone();
        let code_owned = code.to_string();
        let (data, source) = tokio::task::spawn_blocking(move || {
            dm.get_daily_data(&code_owned, 30)
        }).await.context("spawn_blocking panicked")?.context("获取数据失败")?;

        if data.is_empty() {
            warn!("[{}] 获取到的数据为空", code);
            return Ok(data);
        }

        // AGENTS 2.4: 日线/历史数据超过 1 个交易日直接阻断。
        let latest_date = data[0].date;
        let freshness = FreshnessConfig {
            quote_max_age_secs: self.config.dq_quote_stale_sec,
            position_max_age_secs: self.config.dq_position_stale_sec,
            nav_max_age_secs: self.config.dq_nav_stale_sec,
            daily_max_age_secs: self.config.dq_daily_stale_sec,
        };
        let dq_stats = DqStats::new();
        if let Err(reason) = validate_daily_freshness(latest_date, chrono::Local::now(), &freshness, &dq_stats) {
            anyhow::bail!(
                "[{}] 日线新鲜度校验失败: {} (latest_date={})",
                code,
                reason.label(),
                latest_date
            );
        }

        // 维度4最小闭环：日线 OHLC 一致性 + 异常跳变告警（失败即阻断）
        if let Err(msg) = validate_daily_kline_quality(&data, 20.0) {
            anyhow::bail!("[{}] 日线质量校验失败: {}", code, msg);
        }

        info!("[{}] 从 {} 获取到 {} 条数据", code, source, data.len());

        // 保存到数据库
        if let Ok(db) = std::panic::catch_unwind(|| DatabaseManager::get()) {
            match db.save_kline_data(code, &data, &source) {
                Ok(count) => info!("[{}] 已保存 {} 条K线数据到数据库", code, count),
                Err(e) => warn!("[{}] 保存K线数据到数据库失败: {}", code, e),
            }
        }

        Ok(data)
    }
}