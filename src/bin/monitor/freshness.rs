//! 修复 Top10#3+#4 (2026-06-29 audit): bin/monitor/main.rs (1934 行) 拆 4 个子模块
//!
//! 这个文件: `bin/monitor/freshness.rs` — freshness 校验 (原 main.rs line 1837-1935, 99 行)
//!
//! 包含:
//! - monitor_freshness_config(): 从全局 config 构造 FreshnessConfig
//! - validate_position_freshness / validate_quote_freshness / validate_nav_freshness:
//!   数据新鲜度校验, 过期数据阻断推送 (AGENTS §2.4 红线)
//!
//! 拆分后 main.rs 从 1934 → ~1820 行

use chrono::{DateTime, Local, NaiveDate};

use stock_analysis::monitor::data_quality::{
    self, DqStats, FreshnessConfig, FreshnessDataType,
};

pub fn validate_position_freshness(fetch_time: DateTime<Local>) -> bool {
    let stats = DqStats::new();
    let freshness = monitor_freshness_config();
    match data_quality::validate_freshness(
        FreshnessDataType::Position,
        fetch_time,
        &freshness,
        &stats,
    ) {
        Ok(()) => true,
        Err(reason) => {
            log::warn!(
                "[DQ_FRESHNESS] rule_id=AGENTS-2.4 data_type=position action=reject reason={} timestamp={}",
                reason.label(),
                chrono::Utc::now().timestamp()
            );
            false
        }
    }
}

pub fn validate_quote_freshness(update_time: DateTime<Local>, source: &str, code: &str) -> bool {
    let stats = DqStats::new();
    let freshness = monitor_freshness_config();
    match data_quality::validate_freshness(
        FreshnessDataType::Quote,
        update_time,
        &freshness,
        &stats,
    ) {
        Ok(()) => true,
        Err(reason) => {
            log::warn!(
                "[DQ_FRESHNESS] rule_id=AGENTS-2.4 data_type=quote source={} code={} action=reject reason={} timestamp={}",
                source,
                code,
                reason.label(),
                chrono::Utc::now().timestamp()
            );
            false
        }
    }
}

pub fn validate_nav_freshness(nav_date: NaiveDate) -> bool {
    let stats = DqStats::new();
    let freshness = monitor_freshness_config();
    match data_quality::validate_freshness(
        FreshnessDataType::Nav,
        chrono::Local::now(),
        &freshness,
        &stats,
    ) {
        Ok(()) => true,
        Err(reason) => {
            log::warn!(
                "[DQ_FRESHNESS] rule_id=AGENTS-2.4 data_type=nav nav_date={} action=reject reason={} timestamp={}",
                nav_date,
                reason.label(),
                chrono::Utc::now().timestamp()
            );
            false
        }
    }
}

pub fn monitor_freshness_config() -> FreshnessConfig {
    let cfg = stock_analysis::config::get_monitor_config();
    FreshnessConfig {
        quote_max_age_secs: cfg.dq_quote_stale_sec,
        position_max_age_secs: cfg.dq_position_stale_sec,
        nav_max_age_secs: cfg.dq_nav_stale_sec,
        daily_max_age_secs: cfg.dq_daily_stale_sec,
    }
}