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

use stock_analysis::monitor::data_quality::{self, DqStats, FreshnessConfig, FreshnessDataType};

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

pub fn validate_daily_snapshot_freshness(data_date: NaiveDate, source: &str, code: &str) -> bool {
    let stats = DqStats::new();
    let freshness = monitor_freshness_config();
    match data_quality::validate_daily_freshness(
        data_date,
        chrono::Local::now(),
        &freshness,
        &stats,
    ) {
        Ok(()) => true,
        Err(reason) => {
            log::warn!(
                "[DQ_FRESHNESS] rule_id=AGENTS-2.4 data_type=daily_snapshot source={} code={} data_date={} action=reject reason={} timestamp={}",
                source,
                code,
                data_date,
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
    // 修复 (2026-06-30 codex review): 之前用 validate_freshness(_, Local::now(), _)
    // 导致 age = now() - now() = 0 永远 Ok, 违反 AGENTS §2.4.
    // 改用 validate_daily_freshness: calendar-aware, 按交易日阈值判定.
    match data_quality::validate_daily_freshness(nav_date, chrono::Local::now(), &freshness, &stats)
    {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn latest_effective_trading_day() -> chrono::NaiveDate {
        let today = chrono::Local::now().date_naive();
        if stock_analysis::calendar::is_trading_day(today) {
            today
        } else {
            stock_analysis::calendar::prev_trading_day(today)
        }
    }

    #[test]
    fn validate_nav_freshness_passes_recent_date() {
        // 今天或前一个交易日应通过 (阈值默认 86400s = 1 交易日)
        let today = latest_effective_trading_day();
        assert!(validate_nav_freshness(today));
    }

    #[test]
    fn validate_nav_freshness_rejects_old_date() {
        // 修复 (2026-06-30 codex review): 修复前 always-passes,
        // 这个测试现在能正确捕获 stale data.
        let today = chrono::Local::now().date_naive();
        // 一年前肯定 stale (远超 1 交易日阈值)
        let old =
            chrono::NaiveDate::from_ymd_opt(today.year() - 1, today.month(), today.day()).unwrap();
        assert!(!validate_nav_freshness(old));
    }
}
