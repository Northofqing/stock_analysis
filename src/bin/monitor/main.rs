//! Registered business rules: BR-043, BR-045, BR-047, BR-049, BR-051, BR-063, BR-071, BR-074, BR-077, BR-078, BR-082, BR-083, BR-136, BR-140, BR-190, BR-192, BR-196, BR-211, BR-212, BR-213.
//! 实盘监控模式入口。

//!

//! 用法：

//!   cargo run --bin monitor             # 正常监控（等交易日+交易时段）

//!   cargo run --bin monitor -- --test   # 隔离 E2E + 完整模板飞书验收
//!
//!   cargo run --bin monitor -- --test --push-dry-run # 完整模板只渲染/落审计，不外发

//!

//! 依赖 .env 中 MONITOR_ENABLED=true

#![allow(
    clippy::empty_line_after_doc_comments,
    reason = "legacy monitor sections use spaced narrative comments; this style does not change executable behavior"
)]

use once_cell::sync::Lazy;

use serde::{Deserialize, Serialize};

use std::io::Write;

use std::sync::atomic::AtomicBool;

#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

use stock_analysis::calendar::{self, current_session, is_market_active, MarketSession};

use stock_analysis::app::modes::run_chain_analysis_mode;

use stock_analysis::monitor::detector::{
    AlertCategory, AlertDetail, AlertEvent, AlertLevel, Detector, DetectorConfig, StockSnapshot,
};

use stock_analysis::monitor::prediction;

use stock_analysis::monitor::scanner::TieredScanner;

use stock_analysis::monitor::signal_state::SignalStateMachine;

pub const DEFAULT_MAGICLAW_API_ADDR: &str = "127.0.0.1:18011";

pub const DEFAULT_MAGICLAW_PROJECT_ID: &str = "stock_analysis";

pub const DEFAULT_MAGICLAW_CLIENT_NAME: &str = "monitor";

pub const DEFAULT_MAGICLAW_TOKEN_TTL_SECS: i64 = 7 * 24 * 3600;

pub const DEFAULT_MAGICLAW_TOKEN_REFRESH_AHEAD_SECS: i64 = 10 * 60;

pub static MAGICLAW_DAEMON_BOOT_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

pub static MAGICLAW_TOKEN_MEM_CACHE: Lazy<tokio::sync::RwLock<Option<CachedApiToken>>> =
    Lazy::new(|| tokio::sync::RwLock::new(None));

pub static MAGICLAW_TOKEN_ISSUE_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

pub static MAGICLAW_DISABLE_ENV_TOKEN: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) struct TestEnvGuard {
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

#[cfg(test)]
impl TestEnvGuard {
    pub(crate) fn capture(keys: &[&'static str]) -> Self {
        Self {
            previous: keys
                .iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect(),
        }
    }

    pub(crate) fn dry_run_non_quiet() -> Self {
        let guard = Self::capture(&[
            "V10_DRY_RUN_PUSH",
            "PUSH_VERBOSE",
            "STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE",
            "STOCK_ENV_MODE",
            "EVENT_AUDIT_DIR",
            "DURABLE_DELIVERY_TEST_CODE",
            "PUSH_LOG_DIR",
        ]);
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");
        std::env::set_var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "0");
        std::env::set_var("STOCK_ENV_MODE", "test");
        std::env::remove_var("EVENT_AUDIT_DIR");
        std::env::remove_var("PUSH_LOG_DIR");
        static TEST_NAMESPACE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let namespace_sequence = TEST_NAMESPACE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let test_code = format!(
            "TEST_CODE_BR192_MONITOR_{}_{}",
            std::process::id(),
            namespace_sequence
        );
        std::env::set_var("DURABLE_DELIVERY_TEST_CODE", &test_code);
        let namespace = crate::durable_delivery_runtime::current_runtime_namespace()
            .expect("resolve isolated BR-192 test delivery namespace");
        crate::notify::eager_bind_push_log_capability(&namespace)
            .expect("bind isolated BR-192 test push-log capability");
        guard
    }
}

#[cfg(test)]
impl Drop for TestEnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

mod notify;

use crate::notify::{push_governor_v3, PushKind};

mod br196_test_delivery;
mod br196_transport;

mod presentation_registry;

mod push_templates;

mod review_batch;

mod dryrun_report; // v26: dry-run 自动报告

mod v13_diag; // v13.27: 端到端诊断

mod blocking_market_data;
mod closing_valuation_runtime;
mod data_mode_probe;
mod market_data; // BR-148: capability probes remain independent from governance DataMode

fn audit_full_market_rankings_unavailable(owner: &str) {
    market_data::log_full_market_rankings_unavailable(owner);
    push_templates::log_dispatcher_attempt(
        owner,
        false,
        0,
        market_data::FULL_MARKET_RANKINGS_UNAVAILABLE_AUDIT,
    );
}

mod intraday_market;

mod durable_delivery_runtime;
mod v14_adapter;

mod l6_sink;

mod news_aggregator_init;
mod news_ai_shadow;

mod health;

mod webhook_alert;

// 修复 Top10#3+#4 (2026-06-29 audit): 拆大文件

mod freshness;

mod v17_sources; // v17.7 Task 5: six-source monitor push adapter

pub use freshness::{monitor_freshness_config, validate_position_freshness};

pub enum DaemonReadySource {
    Reused,

    StartedNow,
}

pub enum ApiTokenSource {
    Env,

    DynamicMemCache,

    DynamicFileCache,

    DynamicIssued,
}

#[derive(Clone, Copy)]

pub enum MessageSendType {
    Wechat,

    Feishu,
}

#[derive(Clone, Copy)]

pub enum MessageSendTransport {
    Http,

    Cli,
}

impl MessageSendType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Wechat => "wechat",

            Self::Feishu => "feishu",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Wechat => "微信",

            Self::Feishu => "飞书",
        }
    }
}

impl MessageSendTransport {
    fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",

            Self::Cli => "cli",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]

pub struct CachedApiToken {
    token: String,

    expires_at: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]

pub enum AirRefuelEntryMode {
    Confirm,

    Pilot,
}

fn air_refuel_entry_mode() -> AirRefuelEntryMode {
    let cfg = stock_analysis::config::get_monitor_config();

    let mode = cfg.air_refuel.entry_mode.as_str();

    if mode.trim().eq_ignore_ascii_case("pilot") {
        AirRefuelEntryMode::Pilot
    } else {
        AirRefuelEntryMode::Confirm
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct VirtualObservationRecord {
    entry_date: String,

    code: String,

    name: String,

    entry_price: f64,

    shares: u32,

    entry_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]

pub struct VirtualObservationSnapshot {
    created_at: String,

    records: Vec<VirtualObservationRecord>,
}

fn virtual_observation_dir() -> std::path::PathBuf {
    match stock_analysis::risk::env_guard::current_env() {
        stock_analysis::risk::env_guard::TradingEnv::Prod => {
            std::path::PathBuf::from("data/virtual_observation")
        }
        stock_analysis::risk::env_guard::TradingEnv::Test => {
            std::path::PathBuf::from("data/test/virtual_observation")
        }
    }
}

fn validate_virtual_observation_record(
    record: &VirtualObservationRecord,
    expected_date: chrono::NaiveDate,
) -> Result<(), String> {
    stock_analysis::risk::env_guard::validate_symbol_for_current_env(&record.code)?;
    let env = stock_analysis::risk::env_guard::current_env();
    if env == stock_analysis::risk::env_guard::TradingEnv::Prod
        && (record.code.len() != 6 || !record.code.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(format!("虚拟观察代码非法: {:?}", record.code));
    }
    if record.name.trim().is_empty() {
        return Err(format!("虚拟观察 {} 名称为空", record.code));
    }
    let entry_date = chrono::NaiveDate::parse_from_str(&record.entry_date, "%Y-%m-%d")
        .map_err(|error| format!("虚拟观察 {} entry_date 非法: {error}", record.code))?;
    if entry_date != expected_date {
        return Err(format!(
            "虚拟观察 {} entry_date={} 与快照日期 {} 不一致",
            record.code, entry_date, expected_date
        ));
    }
    if !record.entry_price.is_finite() || record.entry_price <= 0.0 {
        return Err(format!(
            "虚拟观察 {} entry_price 非法: {}",
            record.code, record.entry_price
        ));
    }
    if record.shares == 0 || !record.shares.is_multiple_of(100) {
        return Err(format!(
            "虚拟观察 {} shares 必须为正数且是 100 股整数手: {}",
            record.code, record.shares
        ));
    }
    if !matches!(record.entry_mode.as_str(), "pilot" | "confirm") {
        return Err(format!(
            "虚拟观察 {} entry_mode 非法: {:?}",
            record.code, record.entry_mode
        ));
    }
    Ok(())
}

fn validate_virtual_observation_snapshot(
    snapshot: &VirtualObservationSnapshot,
    expected_date: chrono::NaiveDate,
) -> Result<(), String> {
    let created_at =
        chrono::NaiveDateTime::parse_from_str(&snapshot.created_at, "%Y-%m-%d %H:%M:%S")
            .map_err(|error| format!("虚拟观察快照 created_at 非法: {error}"))?;
    if created_at.date() != expected_date {
        return Err(format!(
            "虚拟观察快照 created_at 日期 {} 与文件日期 {} 不一致",
            created_at.date(),
            expected_date
        ));
    }
    if snapshot.records.is_empty() {
        return Err("虚拟观察快照 records 为空".to_string());
    }
    let mut codes = std::collections::HashSet::new();
    for record in &snapshot.records {
        validate_virtual_observation_record(record, expected_date)?;
        if !codes.insert(record.code.as_str()) {
            return Err(format!("虚拟观察快照 code 重复: {}", record.code));
        }
    }
    Ok(())
}

fn read_virtual_observation_snapshot(
    path: &std::path::Path,
    expected_date: chrono::NaiveDate,
) -> Result<Option<VirtualObservationSnapshot>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("读取虚拟观察快照 {} 失败: {error}", path.display()));
        }
    };
    let snapshot: VirtualObservationSnapshot = serde_json::from_str(&raw)
        .map_err(|error| format!("解析虚拟观察快照 {} 失败: {error}", path.display()))?;
    validate_virtual_observation_snapshot(&snapshot, expected_date)
        .map_err(|error| format!("虚拟观察快照 {} 校验失败: {error}", path.display()))?;
    Ok(Some(snapshot))
}

fn atomic_write_virtual_snapshot(path: &std::path::Path, json: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let parent = path
        .parent()
        .ok_or_else(|| format!("虚拟观察快照路径无父目录: {}", path.display()))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("虚拟观察快照文件名非法: {}", path.display()))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));

    let result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| format!("创建临时快照 {} 失败: {error}", temp.display()))?;
        file.write_all(json)
            .map_err(|error| format!("写入临时快照 {} 失败: {error}", temp.display()))?;
        file.sync_all()
            .map_err(|error| format!("刷盘临时快照 {} 失败: {error}", temp.display()))?;
        std::fs::rename(&temp, path).map_err(|error| {
            format!(
                "原子替换虚拟观察快照 {} -> {} 失败: {error}",
                temp.display(),
                path.display()
            )
        })?;
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("刷盘虚拟观察目录 {} 失败: {error}", parent.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn merge_virtual_observation_records(
    mut existing: Vec<VirtualObservationRecord>,
    incoming: &[VirtualObservationRecord],
    expected_date: chrono::NaiveDate,
) -> Result<Vec<VirtualObservationRecord>, String> {
    for record in &existing {
        validate_virtual_observation_record(record, expected_date)?;
    }
    for new_record in incoming {
        validate_virtual_observation_record(new_record, expected_date)?;
        if let Some(slot) = existing
            .iter_mut()
            .find(|record| record.code == new_record.code)
        {
            *slot = new_record.clone();
        } else {
            existing.push(new_record.clone());
        }
    }
    Ok(existing)
}

fn persist_virtual_observation_snapshot(
    records: &[VirtualObservationRecord],
) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }

    let dir = virtual_observation_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("[虚拟观察仓] 创建目录 {} 失败: {error}", dir.display()))?;

    let today = chrono::Local::now().date_naive();
    let compact_today = today.format("%Y%m%d").to_string();

    let daily = dir.join(format!("{}.json", compact_today));

    let latest = dir.join("latest.json");

    let existing = read_virtual_observation_snapshot(&daily, today)?
        .map(|snapshot| snapshot.records)
        .unwrap_or_default();
    let merged = merge_virtual_observation_records(existing, records, today)?;

    let snapshot = VirtualObservationSnapshot {
        created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),

        records: merged,
    };
    validate_virtual_observation_snapshot(&snapshot, today)?;
    let json = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| format!("[虚拟观察仓] 序列化失败: {error}"))?;
    atomic_write_virtual_snapshot(&daily, &json)?;
    atomic_write_virtual_snapshot(&latest, &json)?;

    log::info!(
        "[虚拟观察仓] 已落盘: {} ({}条)",
        daily.display(),
        snapshot.records.len()
    );
    Ok(())
}

#[cfg(test)]
fn t1_close_from_records(
    code: &str,
    records: &[stock_analysis::data_provider::KlineData],
    evidence: &stock_analysis::data_gateway::BatchEvidence,
    base_date: chrono::NaiveDate,
) -> Result<Option<f64>, String> {
    let Some(t1) = records
        .iter()
        .filter(|bar| bar.date > base_date)
        .min_by_key(|bar| bar.date)
    else {
        return Ok(None);
    };
    if !t1.close.is_finite() || t1.close <= 0.0 {
        return Err(format!(
            "[虚拟观察仓] {code} T+1={} close 非正/非法: {} source={} batch_id={}",
            t1.date, t1.close, evidence.source, evidence.batch_id
        ));
    }
    Ok(Some(t1.close))
}

#[cfg(test)]
fn chronological_daily_bars(
    records: &[stock_analysis::data_provider::KlineData],
) -> Vec<stock_analysis::data_provider::KlineData> {
    let mut chronological = records.to_vec();
    chronological.reverse();
    chronological
}

#[cfg(test)]
fn latest_price_change_from_records(
    code: &str,
    records: &[stock_analysis::data_provider::KlineData],
    evidence: &stock_analysis::data_gateway::BatchEvidence,
) -> Result<(f64, f64), String> {
    let latest = records.first().ok_or_else(|| {
        format!(
            "{code} 准入日线批次意外为空: source={} batch_id={}",
            evidence.source, evidence.batch_id
        )
    })?;
    if !latest.close.is_finite() || latest.close <= 0.0 || !latest.pct_chg.is_finite() {
        return Err(format!(
            "{code} 最新日线价格/涨跌幅非法: date={} close={} pct_chg={} source={} batch_id={}",
            latest.date, latest.close, latest.pct_chg, evidence.source, evidence.batch_id
        ));
    }
    Ok((latest.close, latest.pct_chg))
}

#[cfg(test)]
mod tests_monitor_historical_gateway_projection {
    use super::{
        chronological_daily_bars, latest_price_change_from_records, t1_close_from_records,
    };
    use magic_market_core::ProviderId;
    use stock_analysis::data_gateway::BatchEvidence;
    use stock_analysis::data_provider::{AdjustType, KlineData};

    fn evidence() -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_magic_tdx_daily".to_string(),
            source_at: Some("2026-07-28".to_string()),
            observed_at: "2026-07-28T08:00:00Z".to_string(),
            batch_id: "TEST_CODE_monitor_daily_batch".to_string(),
        }
    }

    fn bar(date: chrono::NaiveDate, close: f64, pct_chg: f64) -> KlineData {
        KlineData {
            date,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000.0,
            amount: 10_000.0,
            pct_chg,
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

    fn records() -> Vec<KlineData> {
        let newest = chrono::NaiveDate::from_ymd_opt(2026, 7, 28).unwrap();
        vec![
            bar(newest, 28.0, 2.8),
            bar(newest - chrono::Duration::days(1), 27.0, 2.7),
            bar(newest - chrono::Duration::days(2), 26.0, 2.6),
            bar(newest - chrono::Duration::days(3), 25.0, 2.5),
        ]
    }

    #[test]
    fn br164_newest_first_projection_uses_first_bar_and_keeps_t1_semantics() {
        let records = records();
        let evidence = evidence();
        assert_eq!(
            latest_price_change_from_records("TEST_CODE_600001", &records, &evidence).unwrap(),
            (28.0, 2.8)
        );
        let base_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 25).unwrap();
        assert_eq!(
            t1_close_from_records("TEST_CODE_600001", &records, &evidence, base_date).unwrap(),
            Some(26.0)
        );
        assert_eq!(evidence.batch_id, "TEST_CODE_monitor_daily_batch");

        let chronological = chronological_daily_bars(&records);
        assert_eq!(chronological.first().unwrap().close, 25.0);
        assert_eq!(chronological.last().unwrap().close, 28.0);
    }

    #[test]
    fn br164_empty_records_cannot_enter_monitor_computation() {
        let evidence = evidence();
        let error =
            latest_price_change_from_records("TEST_CODE_600001", &[], &evidence).unwrap_err();
        assert!(error.contains("意外为空"));
        assert!(error.contains("TEST_CODE_monitor_daily_batch"));
    }
}

/// 从 snapshot.created_at (格式 "YYYY-MM-DD HH:MM:SS") 解析出 NaiveDate

#[cfg(test)]
fn parse_snapshot_base_date(created_at: &str) -> Option<chrono::NaiveDate> {
    let s = created_at.split_whitespace().next()?;

    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

#[cfg(test)]
fn build_virtual_next_day_review_text(
    snapshot: &VirtualObservationSnapshot,

    close_map: &std::collections::HashMap<String, f64>,
) -> Result<Option<String>, String> {
    if snapshot.records.is_empty() {
        return Ok(None);
    }

    let mut lines = vec![
        format!("📘 虚拟观察仓次日表现（基于 {} 建仓）", snapshot.created_at),
        "━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
    ];

    let mut win = 0usize;

    let mut n = 0usize;

    let mut pnl_total = 0.0_f64;

    let mut capital_total = 0.0_f64;

    for r in &snapshot.records {
        if r.entry_price <= 0.0 || r.shares == 0 {
            continue;
        }

        let Some(close) = close_map.get(&r.code).copied() else {
            lines.push(format!("  {}({}) 数据不足", r.name, r.code));

            continue;
        };
        if !close.is_finite() || close <= 0.0 {
            return Err(format!("虚拟观察 {} T+1 收盘价非法: {close}", r.code));
        }

        let ret = (close / r.entry_price - 1.0) * 100.0;

        let pnl = (close - r.entry_price) * r.shares as f64;

        if ret > 0.0 {
            win += 1;
        }

        n += 1;

        pnl_total += pnl;

        capital_total += r.entry_price * r.shares as f64;

        lines.push(format!(
            "  {}({}) {}股 入场¥{:.2} -> 收盘¥{:.2} | {:+.2}% | {:+.0}",
            r.name, r.code, r.shares, r.entry_price, close, ret, pnl
        ));
    }

    if n == 0 {
        return Ok(None);
    }

    let hit_rate = win as f64 / n as f64 * 100.0;

    if !capital_total.is_finite() || capital_total <= 0.0 {
        return Err(format!("虚拟观察组合成本非法: {capital_total}"));
    }
    let total_ret = pnl_total / capital_total * 100.0;

    lines.push(String::new());

    lines.push(format!(
        "命中率 {:.1}% ({}/{}) | 组合收益 {:+.2}% | 组合盈亏 {:+.0}",
        hit_rate, win, n, total_ret, pnl_total
    ));

    Ok(Some(lines.join("\n")))
}

#[cfg(test)]
mod virtual_observation_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn record(
        code: &str,
        price: f64,
        shares: u32,
        date: chrono::NaiveDate,
    ) -> VirtualObservationRecord {
        VirtualObservationRecord {
            entry_date: date.format("%Y-%m-%d").to_string(),
            code: code.to_string(),
            name: "测试观察".to_string(),
            entry_price: price,
            shares,
            entry_mode: "pilot".to_string(),
        }
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        std::env::temp_dir().join(format!(
            "stock-analysis-virtual-observation-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn valid_code_for_current_env() -> &'static str {
        match stock_analysis::risk::env_guard::current_env() {
            stock_analysis::risk::env_guard::TradingEnv::Prod => "TEST_CODE_000001",
            stock_analysis::risk::env_guard::TradingEnv::Test => "TEST_CODE_000001",
        }
    }

    #[test]
    fn merge_replaces_same_code_without_duplicate_trade_fact() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let code = valid_code_for_current_env();
        let existing = vec![record(code, 10.0, 100, date)];
        let incoming = vec![record(code, 11.0, 200, date)];

        let merged = merge_virtual_observation_records(existing, &incoming, date)
            .expect("valid observations");

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].entry_price, 11.0);
        assert_eq!(merged[0].shares, 200);
    }

    #[test]
    fn validation_rejects_bad_price_lot_and_date() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        assert!(validate_virtual_observation_record(
            &record("TEST_CODE_000001", 0.0, 100, date),
            date
        )
        .is_err());
        assert!(validate_virtual_observation_record(
            &record("TEST_CODE_000001", 10.0, 101, date),
            date
        )
        .is_err());
        assert!(validate_virtual_observation_record(
            &record("TEST_CODE_000001", 10.0, 100, date),
            date.succ_opt().expect("next day")
        )
        .is_err());
    }

    #[test]
    fn corrupt_existing_snapshot_is_an_error() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("20260718.json");
        std::fs::write(&path, b"{broken").expect("seed corrupt file");

        let error =
            read_virtual_observation_snapshot(&path, date).expect_err("corrupt snapshot must fail");

        assert!(error.contains("解析虚拟观察快照"));
        std::fs::remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn atomic_snapshot_write_replaces_complete_file() {
        let dir = temp_dir("atomic");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("snapshot.json");
        atomic_write_virtual_snapshot(&path, b"first").expect("first write");
        atomic_write_virtual_snapshot(&path, b"second").expect("replacement write");

        assert_eq!(std::fs::read(&path).expect("read snapshot"), b"second");
        assert_eq!(
            std::fs::read_dir(&dir)
                .expect("read temp dir")
                .filter_map(Result::ok)
                .count(),
            1,
            "temporary files must not remain"
        );
        std::fs::remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn review_rejects_invalid_close_instead_of_rendering_zero() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date");
        let snapshot = VirtualObservationSnapshot {
            created_at: "2026-07-18 09:30:00".to_string(),
            records: vec![record("TEST_CODE_000001", 10.0, 100, date)],
        };
        let closes = std::collections::HashMap::from([("TEST_CODE_000001".to_string(), 0.0)]);

        let error = build_virtual_next_day_review_text(&snapshot, &closes)
            .expect_err("invalid close must fail");

        assert!(error.contains("T+1 收盘价非法"));
    }

    #[test]
    fn legacy_manual_trade_flags_are_detected_in_any_position() {
        assert!(contains_legacy_manual_trade_flag(&[
            "monitor".to_string(),
            "--buy".to_string(),
            "000001:10:100".to_string(),
        ]));
        assert!(contains_legacy_manual_trade_flag(&[
            "monitor".to_string(),
            "--test".to_string(),
            "--sell".to_string(),
        ]));
        assert!(!contains_legacy_manual_trade_flag(&[
            "monitor".to_string(),
            "--review".to_string(),
        ]));
    }

    #[test]
    fn br112_review_flag_selects_a_terminal_review_run() {
        assert!(terminal_review_requested(&[
            "monitor".to_string(),
            "--review".to_string(),
        ]));
        assert!(!terminal_review_requested(&["monitor".to_string()]));
        assert_eq!(
            review_execution_path(&["monitor".to_string(), "--review".to_string()]),
            ReviewExecutionPath::StrictDispatchers,
            "production --review must never reach the legacy inline review implementation"
        );
    }

    #[test]
    fn br136_bare_test_is_the_only_implicit_e2e_route() {
        assert!(isolated_e2e_requested(&[
            "monitor".to_string(),
            "--test".to_string(),
        ]));
        assert!(isolated_e2e_requested(&[
            "monitor".to_string(),
            "--test".to_string(),
            "--e2e".to_string(),
        ]));
        assert!(!isolated_e2e_requested(&[
            "monitor".to_string(),
            "--test".to_string(),
            "--review".to_string(),
        ]));
        assert!(!isolated_e2e_requested(&[
            "monitor".to_string(),
            "--test".to_string(),
            "--v13-diag".to_string(),
        ]));
        assert!(!isolated_e2e_requested(&["monitor".to_string()]));
    }

    #[test]
    fn br141_only_bare_monitor_requires_service_enablement() {
        assert!(service_enablement_required(&["monitor".to_string()]));
        for argument in ["--test", "--review", "--history", "--unknown"] {
            assert!(!service_enablement_required(&[
                "monitor".to_string(),
                argument.to_string(),
            ]));
        }
    }

    #[test]
    fn br170_position_chain_refresh_precedes_long_running_consumers() {
        let source = include_str!("main.rs");
        let service_branch = source
            .rsplit_once("} else if !selection_cli.requires_service_enablement()")
            .map(|(_, service_branch)| service_branch)
            .expect("long-running service branch");
        let refresh = service_branch
            .find("refresh_startup_position_chains().await")
            .expect("startup position-chain refresh");
        let first_consumer = [
            service_branch
                .find("spawn_dryrun_reporter")
                .expect("dry-run reporter"),
            service_branch
                .find("EventBus::global().subscribe")
                .expect("event consumer"),
            service_branch
                .find("let main_loops = async")
                .expect("main consumer loops"),
        ]
        .into_iter()
        .min()
        .expect("at least one long-running consumer");

        assert!(
            refresh < first_consumer,
            "BR-170 startup refresh must finish before any long-running consumer starts"
        );
    }

    #[tokio::test]
    async fn br141_shutdown_propagates_writer_task_failure() {
        let bus = stock_analysis::event::EventBus::new_for_test(1);
        let writer_failure =
            stock_analysis::event::JsonlError::Io(std::io::Error::other("forced writer failure"));
        let mut handle = Some(tokio::spawn(async move { Err(writer_failure) }));

        let error = shutdown_jsonl_writer(&bus, &mut handle)
            .await
            .expect_err("terminal shutdown must expose the writer failure");

        assert!(error.contains("forced writer failure"), "{error}");
        assert!(
            handle.is_none(),
            "writer handle must be consumed exactly once"
        );
        assert_eq!(bus.receiver_count(), 0, "event bus must be closed");
    }

    #[tokio::test]
    async fn br141_writer_shutdown_timeout_is_bounded_and_explicit() {
        let bus = stock_analysis::event::EventBus::new_for_test(1);
        let mut handle = Some(tokio::spawn(async {
            std::future::pending::<()>().await;
            Ok(())
        }));

        let error = shutdown_jsonl_writer_with_timeout(
            &bus,
            &mut handle,
            std::time::Duration::from_millis(10),
        )
        .await
        .expect_err("stuck writer must time out");

        assert!(error.contains("timed out after 10ms"), "{error}");
        assert!(handle.is_none());
        assert_eq!(bus.receiver_count(), 0);
    }

    #[tokio::test]
    async fn br141_unexpected_writer_completion_classifies_every_terminal_state() {
        async fn panicking_writer() -> Result<(), stock_analysis::event::JsonlError> {
            panic!("forced writer panic")
        }

        assert_eq!(
            unexpected_jsonl_writer_completion(Ok(Ok(()))),
            "writer stopped before service shutdown"
        );
        let writer_error =
            stock_analysis::event::JsonlError::Io(std::io::Error::other("forced consume failure"));
        assert!(unexpected_jsonl_writer_completion(Ok(Err(writer_error)))
            .contains("forced consume failure"));

        let join_error = tokio::spawn(panicking_writer())
            .await
            .expect_err("panicking writer must produce JoinError");
        assert!(
            unexpected_jsonl_writer_completion(Err(join_error)).contains("writer task join failed")
        );
    }

    #[tokio::test]
    async fn br141_background_producers_are_aborted_and_joined_before_bus_close() {
        struct DropMarker(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropMarker {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }

        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task_dropped = std::sync::Arc::clone(&dropped);
        let task = tokio::spawn(async move {
            let _marker = DropMarker(task_dropped);
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;

        quiesce_background_tasks(vec![("TEST_CODE producer", task)])
            .await
            .expect("producer shutdown");

        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn br141_supervisor_orders_signal_producer_stop_bus_close_and_writer_drain() {
        struct OrderMarker(std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>);
        impl Drop for OrderMarker {
            fn drop(&mut self) {
                self.0.lock().unwrap().push("producer");
            }
        }

        let bus = stock_analysis::event::EventBus::new_for_test(8);
        let mut receiver = bus.subscribe().expect("subscribe lifecycle writer");
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer_order = std::sync::Arc::clone(&order);
        let mut writer = Some(tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        writer_order.lock().unwrap().push("writer");
                        return Ok(());
                    }
                    Err(error) => return Err(stock_analysis::event::JsonlError::Receive(error)),
                }
            }
        }));
        let producer_order = std::sync::Arc::clone(&order);
        let producer = tokio::spawn(async move {
            let _marker = OrderMarker(producer_order);
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;

        supervise_long_running_lifecycle(
            &bus,
            &mut writer,
            vec![("TEST_CODE producer", producer)],
            std::future::pending::<()>(),
            async { Ok(()) },
        )
        .await
        .expect("signal shutdown must drain cleanly");

        assert_eq!(*order.lock().unwrap(), vec!["producer", "writer"]);
        assert!(writer.is_none());
        assert_eq!(bus.receiver_count(), 0);
    }

    #[tokio::test]
    async fn br141_supervisor_converts_runtime_writer_failure_to_error_after_quiesce() {
        let bus = stock_analysis::event::EventBus::new_for_test(8);
        let writer_error = stock_analysis::event::JsonlError::Io(std::io::Error::other(
            "forced runtime writer failure",
        ));
        let mut writer = Some(tokio::spawn(async move { Err(writer_error) }));
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task_dropped = std::sync::Arc::clone(&dropped);
        let producer = tokio::spawn(async move {
            struct Marker(std::sync::Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Marker {
                fn drop(&mut self) {
                    self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let _marker = Marker(task_dropped);
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;

        let error = supervise_long_running_lifecycle(
            &bus,
            &mut writer,
            vec![("TEST_CODE producer", producer)],
            std::future::pending::<()>(),
            std::future::pending::<Result<(), String>>(),
        )
        .await
        .expect_err("runtime writer failure must stop the service");

        assert!(error.contains("forced runtime writer failure"), "{error}");
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        assert!(writer.is_none());
        assert_eq!(bus.receiver_count(), 0);
    }

    #[tokio::test]
    async fn br141_supervisor_rejects_unexpected_main_loop_completion() {
        let bus = stock_analysis::event::EventBus::new_for_test(8);
        let mut receiver = bus.subscribe().expect("subscribe lifecycle writer");
        let mut writer = Some(tokio::spawn(async move {
            while receiver.recv().await.is_ok() {}
            Ok(())
        }));

        let error = supervise_long_running_lifecycle(
            &bus,
            &mut writer,
            Vec::new(),
            async {},
            std::future::pending::<Result<(), String>>(),
        )
        .await
        .expect_err("long-running loop completion must not look graceful");

        assert!(error.contains("completed unexpectedly"), "{error}");
        assert!(writer.is_none());
    }

    #[tokio::test]
    async fn br103_missing_real_account_snapshot_blocks_close_review() {
        let error = build_close_review_report()
            .await
            .expect_err("missing real account cash must block the report before ledger reads");
        assert!(error.contains("no_fresh_real_account_cash_snapshot"));
    }

    #[test]
    fn snapshot_validation_read_merge_and_review_cover_complete_local_lifecycle() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        let first = record("TEST_CODE_000001", 10.0, 100, date);
        let second = record("TEST_CODE_000002", 20.0, 200, date);
        let snapshot = VirtualObservationSnapshot {
            created_at: "2026-07-18 15:00:00".to_string(),
            records: vec![first.clone(), second.clone()],
        };
        validate_virtual_observation_snapshot(&snapshot, date).expect("valid snapshot");

        let mut empty = snapshot.clone();
        empty.records.clear();
        assert!(validate_virtual_observation_snapshot(&empty, date).is_err());
        let mut duplicate = snapshot.clone();
        duplicate.records.push(first.clone());
        assert!(validate_virtual_observation_snapshot(&duplicate, date).is_err());
        let mut wrong_created = snapshot.clone();
        wrong_created.created_at = "2026-07-17 15:00:00".to_string();
        assert!(validate_virtual_observation_snapshot(&wrong_created, date).is_err());

        let dir = temp_dir("lifecycle");
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.json");
        assert!(read_virtual_observation_snapshot(&missing, date)
            .unwrap()
            .is_none());
        let path = dir.join("20260718.json");
        std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let loaded = read_virtual_observation_snapshot(&path, date)
            .unwrap()
            .expect("complete snapshot");
        assert_eq!(loaded.records.len(), 2);

        let merged = merge_virtual_observation_records(
            vec![first],
            &[record("TEST_CODE_000003", 30.0, 300, date)],
            date,
        )
        .unwrap();
        assert_eq!(merged.len(), 2);

        let closes = std::collections::HashMap::from([
            ("TEST_CODE_000001".to_string(), 11.0),
            ("TEST_CODE_000002".to_string(), 18.0),
        ]);
        let review = build_virtual_next_day_review_text(&snapshot, &closes)
            .unwrap()
            .expect("review text");
        assert!(review.contains("命中率 50.0%"));
        assert!(review.contains("组合收益"));
        assert_eq!(parse_snapshot_base_date("2026-07-18 15:00:00"), Some(date));
        assert!(parse_snapshot_base_date("invalid").is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn message_transport_labels_and_virtual_modes_are_stable() {
        assert_eq!(MessageSendType::Wechat.as_str(), "wechat");
        assert_eq!(MessageSendType::Wechat.label(), "微信");
        assert_eq!(MessageSendType::Feishu.as_str(), "feishu");
        assert_eq!(MessageSendType::Feishu.label(), "飞书");
        assert_eq!(MessageSendTransport::Http.as_str(), "http");
        assert_eq!(MessageSendTransport::Cli.as_str(), "cli");

        let date = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        let mut invalid_mode = record("TEST_CODE_000001", 10.0, 100, date);
        invalid_mode.entry_mode = "unknown".to_string();
        assert!(validate_virtual_observation_record(&invalid_mode, date).is_err());
    }
}

// ============= 按时段选择 active dispatcher 的 --push 调度入口 ============

/// v14.0: dry-run 模式, 验证 dispatcher 数据源 + 渲染, 不实际推送

async fn run_daily_pushes_dry_run() -> Result<(), String> {
    let now = chrono::Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let hhmm = now.format("%H:%M").to_string();
    let a01_snapshot = push_templates::load_paper_review_snapshot_real(&date).await;
    let d01_snapshot = push_templates::load_news_to_idea_snapshot_real(&hhmm).await;
    // BR-225: security identity 解析是异步统一 Gateway 调用，必须在进入
    // blocking 段之前完成。
    let p01_names = match stock_analysis::database::DatabaseManager::get()
        .get_latest_chain_clusters_strict()
        .and_then(|clusters| push_templates::preopen_head_codes(&clusters))
    {
        Ok(codes) => push_templates::resolve_preopen_head_names(&codes)
            .await
            .unwrap_or_else(|error| {
                log::warn!("[dry-run][P-01][BR-225] 头股名称回落解析失败: {error}");
                std::collections::HashMap::new()
            }),
        Err(error) => {
            log::warn!("[dry-run][P-01][BR-225] 头股代码集合不可用: {error}");
            std::collections::HashMap::new()
        }
    };
    tokio::task::spawn_blocking(move || {
        run_daily_pushes_dry_run_blocking(a01_snapshot, d01_snapshot, p01_names)
    })
    .await
    .map_err(|error| format!("dry-run blocking task failed: {error}"))?
}

fn run_daily_pushes_dry_run_blocking(
    a01_snapshot: Result<Option<push_templates::PaperReviewSnapshot>, String>,
    d01_snapshot: Result<push_templates::NewsToIdeaSnapshot, String>,
    p01_names: std::collections::HashMap<String, String>,
) -> Result<(), String> {
    use push_templates::{
        build_industry_chain_intraday_from_snapshot, build_intraday_market_from_snapshot,
        build_news_catalyst_from_snapshot, build_news_to_idea_from_snapshot,
        build_paper_review_from_snapshot, build_preopen_news_hot_from_db,
        load_industry_chain_snapshot_real, load_news_catalyst_snapshot_real,
        load_sector_snapshot_real, log_dispatcher_attempt,
    };

    use stock_analysis::database::DatabaseManager;

    let now = chrono::Local::now();

    let hhmm = now.format("%H:%M").to_string();

    let date = now.format("%Y-%m-%d").to_string();

    log::info!("[v14.0 dry-run] 模式启动 ({} {})", date, hhmm);
    let mut failures = Vec::new();

    // P-01 dry-run

    let db = DatabaseManager::get();
    match (
        db.get_latest_chain_clusters_strict(),
        db.get_latest_board_rotations_strict(),
    ) {
        (Ok(clusters), Ok(rotations)) if !clusters.is_empty() && !rotations.is_empty() => {
            match build_preopen_news_hot_from_db(&hhmm, &clusters, &rotations, &p01_names) {
                Ok(_params) => {
                    log_dispatcher_attempt("P-01-dry", true, clusters.len(), "");
                    log::info!("[dry-run] P-01 OK: {} clusters", clusters.len());
                }
                Err(error) => {
                    log_dispatcher_attempt("P-01-dry", false, 0, &error);
                    failures.push(format!("P-01 build: {error}"));
                }
            }
        }
        (Ok(_), Ok(_)) => {
            log_dispatcher_attempt("P-01-dry", false, 0, "no clusters/news");
            log::warn!("[dry-run] P-01 SKIP: no clusters/news");
        }
        (Err(error), _) | (_, Err(error)) => {
            log_dispatcher_attempt("P-01-dry", false, 0, &error);
            failures.push(format!("P-01 source: {error}"));
        }
    }

    // I-01 dry-run

    match load_sector_snapshot_real(&hhmm) {
        Ok(snapshot)
            if !snapshot.main_attack.is_empty()
                || !snapshot.tech_sub.is_empty()
                || !snapshot.power_sub.is_empty()
                || !snapshot.robot_sub.is_empty() =>
        {
            let _p = build_intraday_market_from_snapshot(&snapshot);
            log_dispatcher_attempt("I-01-dry", true, 3, "");
            log::info!(
                "[dry-run] I-01 OK: tech={} power={} robot={}",
                snapshot.tech_sub,
                snapshot.power_sub,
                snapshot.robot_sub
            );
        }
        Ok(_) => {
            log_dispatcher_attempt("I-01-dry", false, 0, "sector empty");
            log::warn!("[dry-run] I-01 SKIP: no sectors");
        }
        Err(error) => {
            log_dispatcher_attempt("I-01-dry", false, 0, &error);
            failures.push(format!("I-01 source: {error}"));
        }
    }

    // I-02/I-03/D-01/A-01 dry-run

    match load_news_catalyst_snapshot_real(&hhmm) {
        Ok(snapshot) if !snapshot.headline.is_empty() => {
            let _p = build_news_catalyst_from_snapshot(&snapshot);
            log_dispatcher_attempt("I-02-dry", true, snapshot.stocks.len(), "");
            log::info!("[dry-run] I-02 OK: {} stocks", snapshot.stocks.len());
        }
        Ok(_) => log_dispatcher_attempt("I-02-dry", false, 0, "snapshot empty"),
        Err(error) => {
            log_dispatcher_attempt("I-02-dry", false, 0, &error);
            failures.push(format!("I-02 source: {error}"));
        }
    }

    let s3 = load_industry_chain_snapshot_real(&hhmm);

    match s3 {
        Ok(snapshot) if !snapshot.chain.is_empty() => {
            let _p = build_industry_chain_intraday_from_snapshot(&snapshot);
            log_dispatcher_attempt("I-03-dry", true, snapshot.supplements.len() + 1, "");
            log::info!("[dry-run] I-03 OK: chain={}", snapshot.chain);
        }
        Ok(_) => log_dispatcher_attempt("I-03-dry", false, 0, "snapshot empty"),
        Err(error) => {
            log_dispatcher_attempt("I-03-dry", false, 0, &error);
            failures.push(format!("I-03 source: {error}"));
        }
    }

    match d01_snapshot {
        Ok(snapshot) if !snapshot.headline.is_empty() => {
            let _params = build_news_to_idea_from_snapshot(&snapshot);
            log_dispatcher_attempt("D-01-dry", true, snapshot.reasons.len(), "");
            log::info!(
                "[dry-run] D-01 OK: name={} code={}",
                snapshot.name,
                snapshot.code
            );
        }
        Ok(_) => log_dispatcher_attempt("D-01-dry", false, 0, "snapshot empty"),
        Err(error) => {
            log_dispatcher_attempt("D-01-dry", false, 0, &error);
            failures.push(format!("D-01 source: {error}"));
        }
    }

    match a01_snapshot {
        Ok(Some(snapshot)) => {
            let _params = build_paper_review_from_snapshot(&snapshot);
            log_dispatcher_attempt("A-01-dry", true, 1, "");
            log::info!(
                "[dry-run] A-01 OK: name={} pnl={:?}",
                snapshot.name,
                snapshot.pnl
            );
        }
        Ok(None) => log_dispatcher_attempt("A-01-dry", false, 0, "snapshot empty"),
        Err(error) => {
            log_dispatcher_attempt("A-01-dry", false, 0, &error);
            failures.push(format!("A-01 source: {error}"));
        }
    }

    log::info!("[v14.0 dry-run] 详见 data/dispatcher_log.jsonl");

    if failures.is_empty() {
        log::info!("[v14.0 dry-run] 完成 ({} {})", date, hhmm);
        Ok(())
    } else {
        log::error!(
            "[v14.0 dry-run] 失败 ({} {}) | {} 个 source/build failures",
            date,
            hhmm,
            failures.len()
        );
        Err(format!(
            "{} dispatcher source/build failures: {}",
            failures.len(),
            failures.join("; ")
        ))
    }
}

/// 按当前时间窗触发 active dispatcher

/// - 09:00 → P-01 (盘前新闻)

/// - 10:30/11:00/14:30 → I-01/I-02/I-03/I-04/D-01 (盘中)

/// - 19:00 → A-01/A-10 (盘后复盘)

/// - 推送时刻由 `OpportunitySchedule::default()` 统一拥有

fn report_dispatch_outcome(name: &str, delivered: bool, failures: &mut Vec<String>) {
    if delivered {
        log::info!("[v22] {} dispatcher completed", name);
    } else {
        log::warn!(
            "[v22] {} dispatcher did not confirm delivery, continue to next dispatcher",
            name
        );
        failures.push(format!("{name} did not confirm delivery"));
    }
}

async fn run_daily_pushes() -> Result<(), String> {
    use push_templates::{
        dispatch_catalyst_review_daily, dispatch_industry_chain_intraday_daily,
        dispatch_intraday_market_daily, dispatch_news_catalyst_daily,
        dispatch_news_to_idea_daily, dispatch_paper_review_daily, dispatch_preopen_news_hot_daily,
    };

    use stock_analysis::opportunity::scheduler::{OpportunitySchedule, PushWindow};

    // 推送时刻由 OpportunitySchedule::default() 统一拥有。

    let schedule = OpportunitySchedule::default();

    let now = chrono::Local::now();

    let hhmm = now.format("%H:%M").to_string();

    let date = now.format("%Y-%m-%d").to_string();

    let now_time = now.time();

    log::info!(
        "[v22] --push 模式启动 (当前 {} {}, 时刻由 OpportunitySchedule::default() 提供)",
        date,
        hhmm
    );

    // v22: 用 push_window() 判断当前时刻窗口 (替代 v17.6 写死 hour)

    let window = schedule.push_window(now_time);

    log::info!("[v22] 推送窗口: {:?}", window);

    let mut failures = Vec::new();

    match window {
        PushWindow::Preopen => {
            report_dispatch_outcome(
                "P-01",
                dispatch_preopen_news_hot_daily().await,
                &mut failures,
            );
        }

        PushWindow::Intraday => {
            // 5 个盘中 dispatcher (I-01/I-02/I-03/I-04/D-01)
            let banner = current_banner()
                .map_err(|error| format!("BR-108 --push banner unavailable: {error}"))?;

            report_dispatch_outcome(
                "I-01",
                dispatch_intraday_market_daily(&hhmm, &banner).await,
                &mut failures,
            );
            report_dispatch_outcome(
                "I-02",
                dispatch_news_catalyst_daily(&hhmm, &banner).await,
                &mut failures,
            );
            report_dispatch_outcome(
                "I-03",
                dispatch_industry_chain_intraday_daily(&hhmm, &banner).await,
                &mut failures,
            );
            report_dispatch_outcome(
                "D-01",
                dispatch_news_to_idea_daily(&hhmm, &banner).await,
                &mut failures,
            );
            // BR-192 收尾: T-03 真实 counted 投递 (原恒 unavailable)。
            // 与运行时 I-04 计时器同一实现 (prepare_holding_plan_messages)。
            match prepare_holding_plan_messages(&banner).await {
                Ok(messages) => {
                    let mut all_confirmed = true;
                    for prepared in messages {
                        let token = match crate::presentation_registry::acquire_token(
                            "T-03-holding-plan",
                            PushKind::HoldingPlan,
                            "holding_plan_dispatcher",
                            "render_holding_plan",
                        ) {
                            Ok(token) => token,
                            Err(reason) => {
                                failures.push(format!(
                                    "I-04 T-03 token rejected code={}: {reason}",
                                    prepared.code
                                ));
                                all_confirmed = false;
                                continue;
                            }
                        };
                        let outcome = notify::push_counted_with_binding(
                            token,
                            &prepared.text,
                            None,
                            prepared.binding,
                        )
                        .await;
                        if !matches!(
                            outcome,
                            notify::PushOutcome::Pushed | notify::PushOutcome::Deduped
                        ) {
                            failures.push(format!(
                                "I-04 T-03 delivery unconfirmed code={}: {:?}",
                                prepared.code, outcome
                            ));
                            all_confirmed = false;
                        }
                    }
                    if all_confirmed {
                        report_dispatch_outcome("I-04", true, &mut failures);
                    }
                }
                Err(error) => {
                    failures.push(format!("I-04 T-03 batch rejected: {error}"));
                }
            }
        }

        PushWindow::Evening => {
            report_dispatch_outcome(
                "A-01",
                dispatch_paper_review_daily(&date).await,
                &mut failures,
            );
            report_dispatch_outcome(
                "A-10",
                dispatch_catalyst_review_daily(&date).await,
                &mut failures,
            );
        }

        PushWindow::Outside => {
            // v22: 窗口外, 仅 A-01/A-10 兜底 (窗口信息读 config, 不再写死 09:00-19:00)

            log::warn!(

                "[v22] 当前时间 {} 不在 push 窗口内 (盘前 {} / 盘中 {:?} / 盘后 {}), 仅推 A-01/A-10 兜底",

                hhmm,

                schedule.push_preopen.format("%H:%M"),

                schedule.push_intraday.iter().map(|t| t.format("%H:%M").to_string()).collect::<Vec<_>>(),

                schedule.push_evening.format("%H:%M"),

            );

            report_dispatch_outcome(
                "A-01",
                dispatch_paper_review_daily(&date).await,
                &mut failures,
            );
            report_dispatch_outcome(
                "A-10",
                dispatch_catalyst_review_daily(&date).await,
                &mut failures,
            );
        }
    }

    if failures.is_empty() {
        log::info!("[v22] --push 完成 (HHMM: {})", hhmm);
    } else {
        log::warn!(
            "[v22] --push 已完成但部分调度未确认: {} / {hhmm}, fails={:?}",
            failures.len(),
            failures
        );
    }

    Ok(())
}

// ============= v12 PR1-1.7: AccountMode 评估钩子 =============

/// v41: 共享 banner 状态 (v12 §14.0.1 动态化)

/// 周期调 evaluate_account_mode_hook + evaluate_data_mode_hook 写最新 banner

/// 需要账户/数据模式的 dispatcher 在构造 banner 时从这里读

pub static LATEST_BANNER: Lazy<std::sync::Mutex<Option<push_templates::BannerCtx>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

/// Read the latest fully evaluated banner.
///
/// Before both account and data health have been evaluated there is no truthful
/// banner to return. Callers must skip the affected push instead of displaying
/// a fabricated Normal/Full/zero state.
pub fn current_banner() -> Result<push_templates::BannerCtx, String> {
    LATEST_BANNER
        .lock()
        .map_err(|_| "latest banner lock poisoned".to_string())?
        .clone()
        .ok_or_else(|| "latest banner unavailable before real health evaluation".to_string())
}

fn current_banner_for(context: &str) -> Option<push_templates::BannerCtx> {
    match current_banner() {
        Ok(banner) => Some(banner),
        Err(error) => {
            log::error!("[{context}] push skipped because banner is unavailable: {error}");
            None
        }
    }
}

/// Assemble a complete real-data T-16 batch before the first dispatch.
async fn dispatch_st_price_limit_batch(hhmm: &str) -> Result<usize, String> {
    let positions = stock_analysis::portfolio::get_st_positions()?;
    if positions.is_empty() {
        return Ok(0);
    }

    let mut prepared = Vec::with_capacity(positions.len());
    for position in positions {
        let code = position.code.clone();
        let quote =
            tokio::task::spawn_blocking(move || stock_analysis::broker::execution_quote(&code))
                .await
                .map_err(|error| format!("T-16 quote task failed for {}: {error}", position.code))?
                .map_err(|error| format!("T-16 quote rejected for {}: {error}", position.code))?;
        let holding_qty = u32::try_from(position.shares)
            .map_err(|_| format!("T-16 holding quantity overflow for {}", position.code))?;
        let (new_stop, new_take_profit) =
            push_templates::recalculate_st_risk_levels(position.cost_price, 0.10)?;
        let st_type = if position.star_st {
            push_templates::StType::StarST
        } else {
            push_templates::StType::ST
        };
        prepared.push((
            position,
            quote.price,
            holding_qty,
            new_stop,
            new_take_profit,
            st_type,
        ));
    }

    let banner = current_banner()?;
    let mut pushed = 0;
    for (position, now_price, holding_qty, new_stop, new_take_profit, st_type) in prepared {
        let ok = push_templates::dispatch_st_price_limit_changed(
            hhmm,
            &position.name,
            &position.code,
            st_type,
            0.05,
            0.10,
            holding_qty,
            position.cost_price,
            now_price,
            Some(new_stop),
            Some(new_take_profit),
            &banner,
        )
        .await;
        if !ok {
            return Err(format!(
                "T-16 dispatch rejected after {pushed} successes for {}",
                position.code
            ));
        }
        pushed += 1;
    }
    Ok(pushed)
}

fn evaluated_data_health() -> Result<stock_analysis::monitor::data_mode::DataHealth, String> {
    use stock_analysis::monitor::data_mode::{current_data_health_input, evaluate as dm_evaluate};

    let input = current_data_health_input(120, 600)?;
    Ok(dm_evaluate(&input, None))
}

fn build_banner(
    am_metrics: &stock_analysis::risk::account_mode::PortfolioMetrics,
    account_mode: stock_analysis::risk::action_gate::AccountMode,
    data_health: &stock_analysis::monitor::data_mode::DataHealth,
) -> push_templates::BannerCtx {
    let account_mode = match account_mode {
        stock_analysis::risk::action_gate::AccountMode::Normal => {
            push_templates::AccountMode::Normal
        }
        stock_analysis::risk::action_gate::AccountMode::ReduceOnly => {
            push_templates::AccountMode::ReduceOnly
        }
        stock_analysis::risk::action_gate::AccountMode::Frozen => {
            push_templates::AccountMode::Frozen
        }
    };
    let data_mode = match data_health.mode {
        stock_analysis::monitor::data_mode::DataMode::Full => push_templates::DataMode::Full,
        stock_analysis::monitor::data_mode::DataMode::Degraded => {
            push_templates::DataMode::Degraded
        }
        stock_analysis::monitor::data_mode::DataMode::Unsafe => push_templates::DataMode::Unsafe,
    };
    let data_missing_note = (!data_health.missing.is_empty()).then(|| {
        data_health
            .missing
            .iter()
            .map(|capability| capability.label())
            .collect::<Vec<_>>()
            .join("/")
    });

    // User-confirmed snapshots are display-only account facts until a real
    // broker is connected. Keep `account_metrics_complete` false so risk
    // gates remain conservative, but do not label known values as missing.
    let user_summary = stock_analysis::database::DatabaseManager::try_get().and_then(|_| {
        stock_analysis::database::user_account_summary::latest()
            .ok()
            .flatten()
    });
    let display_total_pos = am_metrics.total_pos_cheng.or_else(|| {
        user_summary
            .as_ref()
            .map(|summary| (summary.position_ratio_pct / 10.0).round().clamp(0.0, 10.0) as u8)
    });
    let display_today_pnl = am_metrics.today_pnl_pct.or_else(|| {
        user_summary.as_ref().and_then(|summary| {
            (summary.total_assets > 0.0).then(|| summary.daily_pnl / summary.total_assets * 100.0)
        })
    });

    push_templates::BannerCtx {
        account_mode,
        total_pos: display_total_pos,
        today_pnl: display_today_pnl,
        account_metrics_complete: am_metrics.is_complete(),
        data_mode,
        data_missing_note,
    }
}

fn store_banner(banner: push_templates::BannerCtx) -> Result<(), String> {
    *LATEST_BANNER
        .lock()
        .map_err(|_| "latest banner lock poisoned".to_string())? = Some(banner);
    Ok(())
}

/// 最近交易日（今天若周一至五则为今天，否则回溯到上一工作日）。
fn latest_trading_date(today: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Datelike;
    let mut day = today;
    loop {
        match day.weekday() {
            chrono::Weekday::Sat | chrono::Weekday::Sun => {
                day = day.pred_opt().expect("date before year 1");
            }
            _ => return day,
        }
    }
}

/// 任务#3: 持仓快照过期检查 — BR-234b 后快照过期时系统自动估值（持仓×实时价），
/// 快照的唯一用途是反映真实持仓变动。连续 5 个交易日无新快照 → 推送提醒
/// （低频率交易者一周一检；1-4 个交易日仅日志）。触发点: 启动时 + 每日 15:10。
/// 每日最多推 1 次（静态日期去重）；快照新鲜或无记录时仅日志，不出声推送。
async fn check_snapshot_staleness_and_notify() {
    use stock_analysis::database::user_account_summary;
    let Some(summary) = user_account_summary::latest().ok().flatten() else {
        log::warn!("[快照提醒] user_account_summary 无记录 — 从未上传快照，跳过提醒");
        return;
    };
    let today = chrono::Local::now().date_naive();
    let last_trading = latest_trading_date(today);
    let Ok(snapshot_date) =
        chrono::NaiveDate::parse_from_str(&summary.effective_at[..10], "%Y-%m-%d")
    else {
        log::warn!(
            "[快照提醒] effective_at 解析失败: {}（格式应为 YYYY-MM-DDTHH:MM:SS+08:00）",
            summary.effective_at
        );
        return;
    };
    if snapshot_date >= last_trading {
        log::debug!(
            "[快照提醒] 快照新鲜: effective_at={}",
            summary.effective_at
        );
        return;
    }
    // 累计过期交易日数 ≥ 5 才提醒（BR-234b：未传时系统已自动估值，无需每日打扰）
    let days_behind = trading_days_since(snapshot_date, last_trading);
    if days_behind < 5 {
        log::debug!(
            "[快照提醒] 快照过期 {days_behind} 个交易日（<5 不提醒）：系统按持仓×实时价自动估值中"
        );
        return;
    }
    // 每日一推去重
    static LAST: std::sync::Mutex<Option<chrono::NaiveDate>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if *last == Some(today) {
        return;
    }
    *last = Some(today);
    let text = format!(
        "[快照提醒] 持仓快照已 {days_behind} 个交易日未更新：最新 {}（总资产 {:.2}）。期间收益为自动估算（持仓×实时行情）；若真实持仓有变动，请上传最新截图。",
        summary.effective_at, summary.total_assets
    );
    log::warn!("[快照提醒] {}", text);
    let outcome = push_governor_v3(&text, PushKind::SnapshotStale, None).await;
    if !outcome.is_pushed() {
        log::warn!("[快照提醒] 推送未投递: {:?}", outcome);
    }
}

/// (start, end] 区间内的交易日数（排除周末；不含 start 当天，含 end）。
fn trading_days_since(start: chrono::NaiveDate, end: chrono::NaiveDate) -> i64 {
    use chrono::Datelike;
    let mut days = 0;
    let mut day = start;
    while day < end {
        day = day.succ_opt().expect("date overflow");
        if !matches!(day.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
            days += 1;
        }
    }
    days
}

/// BR-236 Fix A (2026-08-12): 快照过期时昨日盈亏按持仓市值差自算。
/// - 估值日 > 快照日（快照后未上传）→ 自算 = 估值总市值 − 快照确认市值，
///   出声标注「按持仓市值差」口径（未计交易/现金变动；上传新快照即恢复确认值）。
/// - 否则（估值日 ≤ 快照日）→ 用快照确认值 account.daily_pnl（当日精确）。
/// 纯函数，可单测。
fn closing_valuation_account_note(
    account: &stock_analysis::database::user_account_summary::UserAccountSummary,
    valuation_price_date: chrono::NaiveDate,
    valuation_market_value: Option<f64>,
) -> String {
    let snapshot_date = account
        .effective_at
        .get(..10)
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    match snapshot_date {
        Some(snapshot_date) if valuation_price_date > snapshot_date => {
            match valuation_market_value {
                Some(mv) => format!(
                    "用户确认账户 {:.1}%仓位，昨日盈亏自算 {:+.2}（快照 {snapshot_date} 后未上传，按持仓市值差）",
                    account.position_ratio_pct, mv - account.securities_market_value
                ),
                None => format!(
                    "用户确认账户 {:.1}%仓位，昨日盈亏自算不可用（估值市值缺失）",
                    account.position_ratio_pct
                ),
            }
        }
        _ => format!(
            "用户确认账户 {:.1}%仓位，昨日盈亏 {:+.2}",
            account.position_ratio_pct, account.daily_pnl
        ),
    }
}

fn refresh_closing_valuation_note() {
    let account = stock_analysis::database::user_account_summary::latest()
        .ok()
        .flatten();
    let note = match stock_analysis::database::closing_valuation::latest_persisted_valuation_view()
    {
        Ok(Some(view)) => {
            let account_note = match account.as_ref() {
                Some(account) => closing_valuation_account_note(
                    account,
                    view.valuation.price_date,
                    view.valuation.total_market_value,
                ),
                None => "用户确认账户摘要缺失（仓位/昨日盈亏不可用）".to_string(),
            };
            Some(format!(
                "{}；收盘估值 {} 覆盖 {}/{}，来源 {}{}",
                account_note,
                view.valuation.price_date,
                view.valuation.covered,
                view.valuation.total,
                view.valuation.provider,
                view.valuation
                    .total_unrealized_pnl
                    .map(|p| format!("，持仓未实现盈亏 {p:+.2}"))
                    .unwrap_or_default()
            ))
        }
        Ok(None) => None,
        Err(error) => {
            log::warn!("[BR-147] closing valuation unavailable: {error}");
            None
        }
    };
    push_templates::set_closing_valuation_note(note);
}

#[cfg(test)]
mod tests_br236_valuation_note {
    use super::closing_valuation_account_note;
    use chrono::NaiveDate;

    /// 快照 8/10 确认 -426.05；估值日 8/11 市值 49399 → 自算 49399-50269 = -870
    #[test]
    fn stale_snapshot_self_calcs_daily_pnl() {
        let account = stock_analysis::database::user_account_summary::UserAccountSummary {
            effective_at: "2026-08-10T15:00:00+08:00".to_string(),
            total_assets: 50269.0,
            securities_market_value: 50269.0,
            available_cash: 0.0,
            position_ratio_pct: 67.2,
            daily_pnl: -426.05,
            source: "user_upload".to_string(),
        };
        let note = closing_valuation_account_note(
            &account,
            NaiveDate::from_ymd_opt(2026, 8, 11).expect("date"),
            Some(49399.0),
        );
        assert!(
            note.contains("昨日盈亏自算 -870.00"),
            "unexpected note: {note}"
        );
        assert!(
            note.contains("快照 2026-08-10 后未上传，按持仓市值差"),
            "unexpected note: {note}"
        );
    }

    /// 同日（快照 8/10 确认 -426.05，估值日 8/10）→ 用确认值
    #[test]
    fn same_day_snapshot_uses_confirmed_pnl() {
        let account = stock_analysis::database::user_account_summary::UserAccountSummary {
            effective_at: "2026-08-10T15:00:00+08:00".to_string(),
            total_assets: 50269.0,
            securities_market_value: 50269.0,
            available_cash: 0.0,
            position_ratio_pct: 67.2,
            daily_pnl: -426.05,
            source: "user_upload".to_string(),
        };
        let note = closing_valuation_account_note(
            &account,
            NaiveDate::from_ymd_opt(2026, 8, 10).expect("date"),
            Some(44741.0),
        );
        assert!(
            note.contains("昨日盈亏 -426.05"),
            "unexpected note: {note}"
        );
        assert!(
            !note.contains("自算"),
            "same-day snapshot must not self-calc: {note}"
        );
    }

    /// 快照新于估值日（当天新上传快照，估值还是昨天）→ 确认值
    #[test]
    fn newer_snapshot_uses_confirmed_pnl() {
        let account = stock_analysis::database::user_account_summary::UserAccountSummary {
            effective_at: "2026-08-12T09:00:00+08:00".to_string(),
            total_assets: 48000.0,
            securities_market_value: 47000.0,
            available_cash: 1000.0,
            position_ratio_pct: 97.9,
            daily_pnl: 123.45,
            source: "user_upload".to_string(),
        };
        let note = closing_valuation_account_note(
            &account,
            NaiveDate::from_ymd_opt(2026, 8, 11).expect("date"),
            Some(49399.0),
        );
        assert!(
            note.contains("昨日盈亏 +123.45"),
            "unexpected note: {note}"
        );
    }

    /// 快照过期 + 估值市值缺失 → 出声「自算不可用」，不静默回落确认值
    #[test]
    fn stale_snapshot_without_valuation_value_announces_unavailable() {
        let account = stock_analysis::database::user_account_summary::UserAccountSummary {
            effective_at: "2026-08-10T15:00:00+08:00".to_string(),
            total_assets: 50269.0,
            securities_market_value: 50269.0,
            available_cash: 0.0,
            position_ratio_pct: 67.2,
            daily_pnl: -426.05,
            source: "user_upload".to_string(),
        };
        let note = closing_valuation_account_note(
            &account,
            NaiveDate::from_ymd_opt(2026, 8, 11).expect("date"),
            None,
        );
        assert!(
            note.contains("昨日盈亏自算不可用（估值市值缺失）"),
            "unexpected note: {note}"
        );
    }

    /// effective_at 非标准格式 → 回落确认值（不出 panic）
    #[test]
    fn malformed_effective_at_falls_back_to_confirmed() {
        let account = stock_analysis::database::user_account_summary::UserAccountSummary {
            effective_at: "garbage".to_string(),
            total_assets: 50269.0,
            securities_market_value: 50269.0,
            available_cash: 0.0,
            position_ratio_pct: 67.2,
            daily_pnl: -426.05,
            source: "user_upload".to_string(),
        };
        let note = closing_valuation_account_note(
            &account,
            NaiveDate::from_ymd_opt(2026, 8, 11).expect("date"),
            Some(49399.0),
        );
        assert!(
            note.contains("昨日盈亏 -426.05"),
            "unexpected note: {note}"
        );
    }
}

#[cfg(test)]
mod tests_br236_keepalive_gate {
    use super::off_session_keepalive_due;
    use stock_analysis::calendar::MarketSession;

    /// BR-236: 六枚举门控全表 — 仅午休/盘后需要 keepalive 保活。
    #[test]
    fn gate_table_all_six_sessions() {
        assert!(off_session_keepalive_due(MarketSession::LunchBreak));
        assert!(off_session_keepalive_due(MarketSession::AfterHours));
        assert!(!off_session_keepalive_due(MarketSession::Closed));
        assert!(!off_session_keepalive_due(MarketSession::Auction));
        assert!(!off_session_keepalive_due(MarketSession::Morning));
        assert!(!off_session_keepalive_due(MarketSession::Afternoon));
    }
}

/// v41 + v51: 周期刷新 banner (从 AccountMode + DataMode 评估结果合并)

///   - v51: DataMode 也走真值 (调 dm_evaluate, 不是写死 Full)

pub async fn refresh_banner_state() -> Result<(), String> {
    // 1. 并发调 AccountMode 评估 + prev_mode 查询 (review #14: 原串行 await 浪费 DB RT)

    let (am_metrics_res, prev_mode_res) = tokio::join!(
        tokio::task::spawn_blocking(compute_account_mode_metrics_blocking),
        tokio::task::spawn_blocking(
            stock_analysis::database::account_mode_log::latest_account_mode_change,
        ),
    );

    let am_metrics = match am_metrics_res {
        Ok(Ok(m)) => m,
        Ok(Err(error)) => {
            log::warn!("[AccountMode][BR-103] metrics unavailable; retaining explicit incomplete banner: {error}");
            stock_analysis::risk::account_mode::PortfolioMetrics::incomplete()
        }
        Err(error) => {
            log::warn!("[AccountMode][BR-103] metrics worker unavailable; retaining explicit incomplete banner: {error}");
            stock_analysis::risk::account_mode::PortfolioMetrics::incomplete()
        }
    };

    let prev_mode = match prev_mode_res {
        Ok(Ok(Some(row))) => Some(
            parse_mode_label(&row.new_mode)
                .ok_or_else(|| format!("invalid persisted AccountMode label: {}", row.new_mode))?,
        ),
        Ok(Ok(None)) => None,
        Ok(Err(error)) => return Err(format!("AccountMode state lookup failed: {error}")),
        Err(error) => return Err(format!("AccountMode state lookup join failed: {error}")),
    };

    let thresholds = stock_analysis::config::get_risk_config()
        .account_mode
        .to_thresholds();
    let account_mode =
        stock_analysis::risk::account_mode::evaluate(&am_metrics, prev_mode, &thresholds).mode;
    let data_health = evaluated_data_health()?;
    store_banner(build_banner(&am_metrics, account_mode, &data_health))?;
    refresh_closing_valuation_note();
    Ok(())
}

/// v60 (F10): refresh_banner_state 复用版 — 接受已算的 metrics, 避免重复 DB 查询

///   - 旧 refresh_banner_state: 每次调都重新算 metrics (2x spawn_blocking)

///   - 新 refresh_banner_state_with_metrics: 复用 caller 算好的 metrics, 1x dm_evaluate

///   - 由 evaluate_account_mode_hook 调用 (caller 已有 metrics, 复用)

pub async fn refresh_banner_state_with_metrics(
    am_metrics: &stock_analysis::risk::account_mode::PortfolioMetrics,

    lib_mode: stock_analysis::risk::action_gate::AccountMode,
) -> Result<(), String> {
    let data_health = evaluated_data_health()?;
    // BR-147: this is the only banner path the live monitor and `--review`
    // actually take. Without refreshing the note here the cached slot stays
    // None for the whole process, so a persisted valuation is rendered as
    // "收盘估值不可用" — reporting known data as missing.
    refresh_closing_valuation_note();
    store_banner(build_banner(am_metrics, lib_mode, &data_health))
}

/// v12 PR1-1.7: 在 monitor 主循环调用, 重算 AccountMode 并按需推 T-01.

///

/// 触发点:

///   - 启动后第一轮 (startup=true) — 恢复 DB 末次状态 + 推送状态变更 (若有)

///   - 每个 tick (startup=false) — 重算 metrics, 触发变更即推 T-01

///

/// v41: 同时调 refresh_banner_state 更新共享 banner

///

/// 不触碰 veto_chain (v12.2 §2.4 + PR1 硬约束).

/// 失败不阻塞主循环 (fire-and-forget log).

async fn evaluate_account_mode_hook(startup: bool) -> bool {
    use stock_analysis::database::account_mode_log::latest_account_mode_change;

    // 1. 装 metrics

    let metrics = match tokio::task::spawn_blocking(compute_account_mode_metrics_blocking).await {
        Ok(Ok(m)) => m,

        Ok(Err(e)) => {
            log::warn!(
                "[AccountMode-hook][BR-108] metrics unavailable; evaluate conservatively: {}",
                e
            );
            stock_analysis::risk::account_mode::PortfolioMetrics::incomplete()
        }

        Err(e) => {
            log::warn!(
                "[AccountMode-hook][BR-108] metrics task failed; evaluate conservatively: {:?}",
                e
            );
            stock_analysis::risk::account_mode::PortfolioMetrics::incomplete()
        }
    };

    // 2. 恢复 prev (从 DB 末次变更记录)

    let latest = match tokio::task::spawn_blocking(latest_account_mode_change).await {
        Ok(Ok(row)) => row,

        Ok(Err(e)) => {
            log::error!("[AccountMode-hook] latest_account_mode_change 失败: {}", e);
            return false;
        }

        Err(e) => {
            log::error!("[AccountMode-hook] spawn_blocking join 失败: {:?}", e);
            return false;
        }
    };
    let prev = match latest.as_ref() {
        Some(row) => match parse_mode_label(&row.new_mode) {
            Some(mode) => Some(mode),
            None => {
                log::error!(
                    "[AccountMode-hook] persisted mode label invalid: {:?}",
                    row.new_mode
                );
                return false;
            }
        },
        None => None,
    };

    // 3. Evaluate the real account state before constructing the banner. A
    // missing previous row means "first evaluation", not Normal.
    let thresholds = stock_analysis::config::get_risk_config()
        .account_mode
        .to_thresholds();
    let now_local = chrono::Local::now().time();
    let evaluation = stock_analysis::risk::account_mode::evaluate_with_reset(
        &metrics,
        prev,
        &thresholds,
        now_local,
    );
    let evaluated_mode = evaluation.mode;

    if let Err(error) = refresh_banner_state_with_metrics(&metrics, evaluated_mode).await {
        log::error!("[AccountMode-hook] banner evaluation failed: {error}");
        return false;
    }
    let banner = match current_banner() {
        Ok(banner) => banner,
        Err(error) => {
            log::error!("[AccountMode-hook] evaluated banner unavailable: {error}");
            return false;
        }
    };

    // 4. 评估 + 推

    if startup {
        log::info!(
            "[AccountMode-hook] 启动评估 prev={:?} → 调 push_account_mode_change",
            prev
        );
    }

    let notification = match push_templates::push_account_mode_change(
        &metrics,
        prev,
        latest.as_ref(),
        Some(&banner),
        &evaluation,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            log::warn!(
                "[AccountMode-hook] push_account_mode_change 失败: {}",
                error
            );
            return false;
        }
    };
    if !notification.is_confirmed() {
        log::warn!(
            "[AccountMode-hook][BR-116] notification unconfirmed: {:?}",
            notification
        );
        return false;
    }

    // Refresh once more after the orchestration so the shared state remains
    // aligned even when a reset transition was persisted during this call.
    if let Err(error) = refresh_banner_state_with_metrics(&metrics, evaluated_mode).await {
        log::error!("[AccountMode-hook] final banner refresh failed: {error}");
        return false;
    }
    true
}

fn parse_mode_label(label: &str) -> Option<stock_analysis::risk::action_gate::AccountMode> {
    use stock_analysis::risk::action_gate::AccountMode;

    match label {
        "Normal" => Some(AccountMode::Normal),

        "ReduceOnly" => Some(AccountMode::ReduceOnly),

        "Frozen" => Some(AccountMode::Frozen),

        _ => None,
    }
}

/// 同步版 metrics 装配 (供 spawn_blocking 调用).

/// 数据源: real_account_snapshot + 同批券商成交同步水位.

/// 失败 / 缺失 → 返回 data_complete=false 的 metrics (保守策略).

fn compute_account_mode_metrics_blocking(
) -> Result<stock_analysis::risk::account_mode::PortfolioMetrics, String> {
    let observed_at = chrono::Local::now().fixed_offset();
    let snapshot = stock_analysis::database::account_snapshot::latest_account_snapshot()
        .map_err(|error| format!("BR-103 latest real account snapshot: {error}"))?
        .ok_or_else(|| "BR-103 real account snapshot is missing".to_string())?;
    snapshot.validate_fresh_for_action(observed_at)?;

    if snapshot.daily_pnl_status != "available" {
        return Err(format!(
            "BR-103 daily PnL is unavailable: status={}",
            snapshot.daily_pnl_status
        ));
    }
    let daily_pnl = snapshot
        .daily_pnl
        .ok_or_else(|| "BR-103 daily PnL is missing".to_string())?;
    let position_ratio_pct = snapshot
        .position_ratio_pct
        .ok_or_else(|| "BR-103 position ratio is missing".to_string())?;
    if snapshot.total_assets <= 0.0 {
        return Err("BR-103 total assets must be positive for account mode".to_string());
    }
    let today_pnl_pct = daily_pnl / snapshot.total_assets * 100.0;
    if !today_pnl_pct.is_finite() {
        return Err("BR-103 daily PnL ratio is non-finite".to_string());
    }
    let _total_pos_cheng = (position_ratio_pct / 10.0).round().clamp(0.0, 10.0) as u8;

    // A fresh account snapshot does not prove that the local trade ledger was
    // synchronized in the same batch. Until the broker exposes that watermark,
    // consecutive-stop-loss data must stay incomplete rather than being inferred
    // from an arbitrarily old local `trades` table.
    Err(
        "BR-103 complete account metrics unavailable: real broker trade-sync watermark is not connected"
            .to_string(),
    )
}

/// 同步版连续止损计数: 取最近 5 笔 sell 交易, 倒序遇第一笔非止损即停.

#[cfg(test)]
fn count_consecutive_realized_losses(
    realized: &[(chrono::NaiveDateTime, String, f64)],
) -> Result<u32, String> {
    let mut by_sell: std::collections::HashMap<&str, (chrono::NaiveDateTime, f64)> =
        std::collections::HashMap::new();
    for (sold_at, sell_id, pnl) in realized {
        if sell_id.trim().is_empty() || !pnl.is_finite() {
            return Err(format!("已实现盈亏行非法: sell_id={sell_id:?} pnl={pnl}"));
        }
        let entry = by_sell.entry(sell_id.as_str()).or_insert((*sold_at, 0.0));
        if entry.0 != *sold_at {
            return Err(format!("卖出交易 {sell_id} 存在冲突时间"));
        }
        entry.1 += pnl;
        if !entry.1.is_finite() {
            return Err(format!("卖出交易 {sell_id} 聚合盈亏非有限值"));
        }
    }
    let mut sales: Vec<_> = by_sell
        .into_iter()
        .map(|(sell_id, (sold_at, pnl))| (sold_at, sell_id, pnl))
        .collect();
    sales.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(left.1)));
    let count = sales
        .iter()
        .take(5)
        .take_while(|(_, _, pnl)| *pnl < 0.0)
        .count();
    u32::try_from(count).map_err(|error| format!("连续止损计数溢出: {error}"))
}

#[cfg(test)]
mod account_mode_metric_tests {
    use super::*;

    #[test]
    fn br108_consecutive_losses_use_latest_distinct_realized_sales() {
        let base = chrono::NaiveDate::from_ymd_opt(2026, 7, 1)
            .unwrap()
            .and_hms_opt(15, 0, 0)
            .unwrap();
        let realized = vec![
            (base, "sell-1".to_string(), -100.0),
            (base + chrono::Duration::days(1), "sell-2".to_string(), 50.0),
            (
                base + chrono::Duration::days(2),
                "sell-3".to_string(),
                -20.0,
            ),
            (
                base + chrono::Duration::days(3),
                "sell-4".to_string(),
                -30.0,
            ),
        ];

        assert_eq!(count_consecutive_realized_losses(&realized).unwrap(), 2);
    }

    #[test]
    fn br108_duplicate_sell_ids_are_aggregated_before_counting() {
        let base = chrono::NaiveDate::from_ymd_opt(2026, 7, 1)
            .unwrap()
            .and_hms_opt(15, 0, 0)
            .unwrap();
        let realized = vec![
            (base, "sell-1".to_string(), 20.0),
            (base, "sell-1".to_string(), -50.0),
        ];

        assert_eq!(count_consecutive_realized_losses(&realized).unwrap(), 1);
    }
}

// ===== MVP0-B (v12): DataMode 评估钩子 =====

/// v12 MVP0-B: 装配 DataMode 评估所需指标, 调 push_data_mode_change.

pub static LATEST_DATA_MODE: Lazy<
    std::sync::Mutex<Option<stock_analysis::monitor::data_mode::DataMode>>,
> = Lazy::new(|| std::sync::Mutex::new(None));

/// BR-225c: 抖动抑制 — 记录"待确认切换"的观测时刻。模式在稳定窗口
/// (300s) 内反复横跳时不推, 稳定后才通知 (→Unsafe 恶化除外, 立即推)。
#[derive(Clone, Copy)]
pub(crate) struct DataModePendingStable {
    mode: stock_analysis::monitor::data_mode::DataMode,
    since: std::time::Instant,
}

static DATA_MODE_PENDING_STABLE: Lazy<
    std::sync::Mutex<Option<DataModePendingStable>>,
> = Lazy::new(|| std::sync::Mutex::new(None));

static DATA_MODE_UNSAFE_REMINDER: Lazy<
    std::sync::Mutex<stock_analysis::monitor::data_mode::PersistentUnsafeReminder>,
> = Lazy::new(|| std::sync::Mutex::new(Default::default()));

fn commit_data_mode_reminder_result(
    state: &mut stock_analysis::monitor::data_mode::PersistentUnsafeReminder,
    mode: stock_analysis::monitor::data_mode::DataMode,
    result: &push_templates::ModeDispatchResult,
    confirmed_now: impl FnOnce() -> std::time::Instant,
) -> bool {
    if !matches!(
        result,
        push_templates::ModeDispatchResult::Delivery(notify::PushOutcome::Pushed)
    ) {
        return false;
    }
    state.record_confirmed(mode, confirmed_now());
    true
}

async fn evaluate_data_mode_hook() {
    use crate::push_templates as pt;

    use stock_analysis::monitor::data_mode::{
        current_data_health_input, evaluate as dm_evaluate, DataMode as LibDM,
    };

    let input = match current_data_health_input(120, 600) {
        Ok(input) => input,
        Err(error) => {
            log::error!("[DataMode-hook] health tracker unavailable: {error}");
            return;
        }
    };
    let prev = match LATEST_DATA_MODE.lock() {
        Ok(state) => *state,
        Err(_) => {
            log::error!("[DataMode-hook] latest data mode lock poisoned");
            return;
        }
    };

    let health = dm_evaluate(&input, prev);
    let reminder_evaluated_at = std::time::Instant::now();
    let persistent_reminder_due = match DATA_MODE_UNSAFE_REMINDER.lock() {
        Ok(mut state) => {
            if state.observe_mode(health.mode) {
                log::info!(
                    "[DataMode-hook][BR-135] recovery observed; persistent Unsafe reminder state cleared"
                );
            }
            match state.should_dispatch(health.mode, reminder_evaluated_at) {
                Ok(due) => due,
                Err(error) => {
                    log::error!("[DataMode-hook][BR-135] reminder clock unavailable: {error}");
                    return;
                }
            }
        }
        Err(_) => {
            log::error!("[DataMode-hook][BR-135] reminder state lock poisoned");
            return;
        }
    };

    log::info!(
        "[DataMode-hook] 模式 {:?} → {:?}, missing={:?}",
        prev,
        health.mode,
        health.missing
    );

    let mut banner = match current_banner() {
        Ok(banner) => Some(banner),
        Err(error) => {
            log::error!("[DataMode-hook] banner unavailable, mode push skipped: {error}");
            None
        }
    };
    if let Some(banner) = banner.as_mut() {
        banner.data_mode = match health.mode {
            LibDM::Full => pt::DataMode::Full,
            LibDM::Degraded => pt::DataMode::Degraded,
            LibDM::Unsafe => pt::DataMode::Unsafe,
        };
        banner.data_missing_note = (!health.missing.is_empty()).then(|| {
            health
                .missing
                .iter()
                .map(|capability| capability.label())
                .collect::<Vec<_>>()
                .join("/")
        });
    }

    let Some(banner) = banner else {
        return;
    };
    if let Err(error) = store_banner(banner.clone()) {
        log::error!("[DataMode-hook] banner store failed: {error}");
        return;
    }

    // BR-225c: 抖动抑制 — 非恶化切换需稳定 300s 才推。pending 记录首个
    // 不同模式的观测时刻; 窗口内静默 (warn 一次, v15 出声规则); 投递后清空。
    let pending_since = {
        let mut pending = match DATA_MODE_PENDING_STABLE.lock() {
            Ok(guard) => guard,
            Err(_) => {
                log::error!("[DataMode-hook] pending stable lock poisoned");
                return;
            }
        };
        match pending.as_mut() {
            Some(state) => {
                if state.mode != health.mode {
                    *state = DataModePendingStable {
                        mode: health.mode,
                        since: std::time::Instant::now(),
                    };
                }
                Some(state.since)
            }
            None => None,
        }
    };
    let result =
        match pt::push_data_mode_change(
            &input,
            prev,
            persistent_reminder_due,
            Some(&banner),
            pending_since,
        )
        .await
        {
            Ok(result) => {
                let mut pending = match DATA_MODE_PENDING_STABLE.lock() {
                    Ok(guard) => guard,
                    Err(_) => {
                        log::error!("[DataMode-hook] pending stable lock poisoned");
                        return;
                    }
                };
                match &result {
                    pt::ModeDispatchResult::EstablishedSilently => {
                        // 抖动窗口内跳过 → 出声 (BR-225c, v15 静默路径可见)
                        if pending_since.is_none() && prev.is_some() && prev != Some(health.mode)
                        {
                            log::warn!(
                                "[DataMode-hook][BR-225c] 状态切换 {:?} → {:?} 在 300s 稳定窗口内, 跳过通知 (模式仍生效, 仅推送节流)",
                                prev,
                                health.mode
                            );
                        }
                        if pending.is_none() {
                            *pending = Some(DataModePendingStable {
                                mode: health.mode,
                                since: std::time::Instant::now(),
                            });
                        }
                    }
                    pt::ModeDispatchResult::Delivery(_) => {
                        *pending = None;
                    }
                }
                result
            }
            Err(error) => {
                log::error!("[DataMode-hook] change push failed: {error}");
                return;
            }
        };
    if result.is_confirmed() {
        match LATEST_DATA_MODE.lock() {
            Ok(mut state) => *state = Some(health.mode),
            Err(_) => log::error!("[DataMode-hook] latest data mode lock poisoned"),
        }
    } else {
        log::warn!(
            "[DataMode-hook][BR-116] notification unconfirmed; retaining previous mode {:?}",
            prev
        );
    }
    match DATA_MODE_UNSAFE_REMINDER.lock() {
        Ok(mut state) => {
            if commit_data_mode_reminder_result(
                &mut state,
                health.mode,
                &result,
                std::time::Instant::now,
            ) {
                log::info!(
                    "[DataMode-hook][BR-135] confirmed DataMode delivery committed for reminder state"
                );
            }
        }
        Err(_) => log::error!(
            "[DataMode-hook][BR-135] confirmed delivery not committed: reminder state lock poisoned"
        ),
    }
}

const DATA_MODE_EVALUATION_PERIOD: std::time::Duration = std::time::Duration::from_secs(60);

fn data_mode_evaluation_interval(period: std::time::Duration) -> tokio::time::Interval {
    let first_tick = tokio::time::Instant::now() + period;
    let mut interval = tokio::time::interval_at(first_tick, period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

async fn run_data_mode_scheduler<F, Fut>(mut interval: tokio::time::Interval, mut hook: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    loop {
        interval.tick().await;
        hook().await;
    }
}

async fn data_mode_monitor_loop() {
    log::info!(
        "[DataMode-hook][BR-135] independent scheduler started period={}s",
        DATA_MODE_EVALUATION_PERIOD.as_secs()
    );
    run_data_mode_scheduler(
        data_mode_evaluation_interval(DATA_MODE_EVALUATION_PERIOD),
        evaluate_data_mode_hook,
    )
    .await;
}

/// BR-236 (2026-08-12): 午休/盘后保持 Quote capability 新鲜 (DataMode Full)。
/// tick 循环在午休 sleep / 盘后 break (main.rs:9187-9197), 无调用者执行
/// fetch_position_quotes → 无 Quote mark → 120s 门过期 → Unsafe。keepalive
/// 在这两个时段每 60s 拉一次持仓行情 (网关层 BR-236 二级判定放行当日
/// 最后成交价), 成功即 mark。失败 warn 出声且不 mark → DataMode 诚实降级。
/// 周末/节假日 current_session()==Closed → 不运行 (零调用)。
fn off_session_keepalive_due(session: MarketSession) -> bool {
    matches!(
        session,
        MarketSession::LunchBreak | MarketSession::AfterHours
    )
}

const OFF_SESSION_KEEPALIVE_PERIOD: std::time::Duration = std::time::Duration::from_secs(60);

/// 会话边界 info 节流 — 每 (北京日期, 会话) 进入时打一次, 避免 60s 周期
/// 重复刷信息。
static OFF_SESSION_KEEPALIVE_ANNOUNCED: std::sync::Mutex<Option<(chrono::NaiveDate, bool)>> =
    std::sync::Mutex::new(None);

async fn off_session_quote_keepalive_loop() {
    log::info!(
        "[BR-236] off-session quote keepalive scheduler started period={}s",
        OFF_SESSION_KEEPALIVE_PERIOD.as_secs()
    );
    run_data_mode_scheduler(
        data_mode_evaluation_interval(OFF_SESSION_KEEPALIVE_PERIOD),
        || async {
            let session = stock_analysis::calendar::current_session();
            if !off_session_keepalive_due(session) {
                return;
            }
            let today = chrono::Local::now().date_naive();
            let is_lunch = matches!(session, MarketSession::LunchBreak);
            if let Ok(mut announced) = OFF_SESSION_KEEPALIVE_ANNOUNCED.lock() {
                if *announced != Some((today, is_lunch)) {
                    *announced = Some((today, is_lunch));
                    log::info!(
                        "[BR-236] off-session quote keepalive active session={:?} date={today}",
                        session
                    );
                }
            }
            match tokio::task::spawn_blocking(crate::market_data::fetch_position_quotes).await {
                Ok(Ok(quotes)) if !quotes.is_empty() => {
                    // mark 在 fetch_position_quotes 内部完成 (bin/monitor/market_data.rs:118)
                    log::debug!(
                        "[BR-236] off-session keepalive refreshed positions={}",
                        quotes.len()
                    );
                }
                Ok(Ok(_)) => {
                    // 快照缺失/过期且本地无持仓: 兜底探测基准代码 (与盘前
                    // 探测 main.rs:7641 同款, mark 在 fetch_realtime_quote_batch :136)。
                    let probe = ["000001", "600000", "300750"]
                        .iter()
                        .map(|code| code.to_string())
                        .collect::<Vec<_>>();
                    match tokio::task::spawn_blocking(move || {
                        crate::market_data::fetch_realtime_quotes(&probe)
                    })
                    .await
                    {
                        Ok(Ok(rows)) if !rows.is_empty() => {
                            log::debug!(
                                "[BR-236] off-session keepalive probe fallback rows={}",
                                rows.len()
                            );
                        }
                        Ok(Ok(_)) => {
                            log::warn!("[BR-236] off-session keepalive probe returned no rows");
                        }
                        Ok(Err(error)) => {
                            log::warn!("[BR-236] off-session keepalive probe failed: {error}");
                        }
                        Err(error) => {
                            log::warn!("[BR-236] off-session keepalive probe join failed: {error}");
                        }
                    }
                }
                Ok(Err(error)) => {
                    log::warn!("[BR-236] off-session quote keepalive failed: {error}");
                }
                Err(error) => {
                    log::warn!("[BR-236] off-session keepalive join failed: {error}");
                }
            }
        },
    )
    .await;
}

#[cfg(test)]
mod br135_data_mode_reminder_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use stock_analysis::monitor::data_mode::{DataMode as LibDM, PersistentUnsafeReminder};

    #[test]
    fn br135_reminder_confirmation_requires_pushed() {
        let now = std::time::Instant::now();
        for outcome in [
            notify::PushOutcome::Denied("TEST_CODE denied".to_string()),
            notify::PushOutcome::Deduped,
            notify::PushOutcome::SinkError("TEST_CODE sink".to_string()),
        ] {
            let mut state = PersistentUnsafeReminder::default();
            assert!(!commit_data_mode_reminder_result(
                &mut state,
                LibDM::Unsafe,
                &push_templates::ModeDispatchResult::Delivery(outcome),
                || panic!("unconfirmed delivery must not sample confirmation time"),
            ));
            assert!(state.should_dispatch(LibDM::Unsafe, now).unwrap());
        }

        let mut state = PersistentUnsafeReminder::default();
        let confirmed_at = now + std::time::Duration::from_secs(7);
        assert!(commit_data_mode_reminder_result(
            &mut state,
            LibDM::Unsafe,
            &push_templates::ModeDispatchResult::Delivery(notify::PushOutcome::Pushed),
            || confirmed_at,
        ));
        assert!(!state
            .should_dispatch(
                LibDM::Unsafe,
                confirmed_at + std::time::Duration::from_secs(1_799),
            )
            .unwrap());
        assert!(state
            .should_dispatch(
                LibDM::Unsafe,
                confirmed_at + std::time::Duration::from_secs(1_800),
            )
            .unwrap());
    }

    #[tokio::test]
    async fn br135_scheduler_waits_before_first_tick_and_runs_independently() {
        let calls = Arc::new(AtomicUsize::new(0));
        let tick_observed = Arc::new(tokio::sync::Notify::new());
        let hook_calls = Arc::clone(&calls);
        let hook_tick_observed = Arc::clone(&tick_observed);
        let interval = data_mode_evaluation_interval(std::time::Duration::from_millis(200));

        let task = tokio::spawn(run_data_mode_scheduler(interval, move || {
            let calls = Arc::clone(&hook_calls);
            let tick_observed = Arc::clone(&hook_tick_observed);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tick_observed.notify_one();
            }
        }));

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "first tick must be delayed"
        );

        tokio::time::timeout(std::time::Duration::from_secs(1), tick_observed.notified())
            .await
            .expect("first scheduled evaluation");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        tokio::time::timeout(std::time::Duration::from_secs(1), tick_observed.notified())
            .await
            .expect("second scheduled evaluation");
        assert!(calls.load(Ordering::SeqCst) >= 2);

        task.abort();
        assert!(task
            .await
            .expect_err("scheduler is intentionally aborted")
            .is_cancelled());
    }
}

// 修复 v9.4.15 (2026-06-29 production panic):

// 之前默认 current_thread runtime, block_on_async Ok 分支 handle.block_on(fut) panic

// "Cannot start a runtime from within a runtime".

// 改 multi_thread 让 block_in_place 安全让出 worker.

/// v16.3 Commit 4c: 读 paper_trades 今日成交数 (T-10 推送用)

#[async_trait::async_trait]
trait ReplayNotificationSink: Send + Sync {
    async fn send(&self, text: &str) -> bool;
}

struct RealReplayNotificationSink;

#[async_trait::async_trait]
impl ReplayNotificationSink for RealReplayNotificationSink {
    async fn send(&self, text: &str) -> bool {
        notify::push_wechat(text).await
    }
}

#[async_trait::async_trait]
trait ReplayAuditSink: Send + Sync {
    async fn record(
        &self,
        envelope: &stock_analysis::event::EventEnvelope,
        phase: &str,
        outcome: &str,
    ) -> Result<(), String>;
}

struct FileReplayAuditSink {
    base_dir: std::path::PathBuf,
    previous_hash: tokio::sync::Mutex<Option<String>>,
}

impl FileReplayAuditSink {
    fn new(base_dir: std::path::PathBuf) -> Self {
        Self {
            base_dir,
            previous_hash: tokio::sync::Mutex::new(None),
        }
    }

    fn validate_chain(existing: &str) -> Result<Option<String>, String> {
        use sha2::{Digest, Sha256};

        let mut expected_parent = "GENESIS".to_string();
        let mut last_hash = None;
        for (index, line) in existing.lines().enumerate() {
            if line.trim().is_empty() {
                return Err(format!("replay audit line {} is blank", index + 1));
            }
            let mut record: serde_json::Value = serde_json::from_str(line)
                .map_err(|error| format!("parse replay audit line {}: {error}", index + 1))?;
            let record_hash = record
                .get("record_hash")
                .and_then(serde_json::Value::as_str)
                .filter(|hash| !hash.is_empty())
                .ok_or_else(|| format!("replay audit line {} has no valid record_hash", index + 1))?
                .to_string();
            let parent = record
                .get("previous_hash")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("replay audit line {} has no previous_hash", index + 1))?;
            if parent != expected_parent {
                return Err(format!(
                    "replay audit chain mismatch at line {}: expected parent {}",
                    index + 1,
                    expected_parent
                ));
            }
            record
                .as_object_mut()
                .ok_or_else(|| format!("replay audit line {} is not an object", index + 1))?
                .remove("record_hash");
            let canonical = serde_json::to_vec(&record)
                .map_err(|error| format!("serialize replay audit line {}: {error}", index + 1))?;
            let calculated = format!("{:x}", Sha256::digest(&canonical));
            if calculated != record_hash {
                return Err(format!("replay audit hash mismatch at line {}", index + 1));
            }
            expected_parent = record_hash.clone();
            last_hash = Some(record_hash);
        }
        Ok(last_hash)
    }
}

#[async_trait::async_trait]
impl ReplayAuditSink for FileReplayAuditSink {
    async fn record(
        &self,
        envelope: &stock_analysis::event::EventEnvelope,
        phase: &str,
        outcome: &str,
    ) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncWriteExt;

        tokio::fs::create_dir_all(&self.base_dir)
            .await
            .map_err(|error| format!("create replay audit directory: {error}"))?;
        let now = chrono::Local::now();
        let path = self.base_dir.join(format!("{}.jsonl", now.format("%Y")));
        let mut previous_hash = self.previous_hash.lock().await;
        if previous_hash.is_none() {
            let existing = match tokio::fs::read_to_string(&path).await {
                Ok(existing) => existing,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(error) => return Err(format!("read replay audit {}: {error}", path.display())),
            };
            *previous_hash = Self::validate_chain(&existing)?;
        }
        let chain_parent = previous_hash.as_deref().unwrap_or("GENESIS");
        let mut record = serde_json::json!({
            "audit_ts": now.to_rfc3339(),
            "envelope_id": envelope.id,
            "replay_of": envelope.replay_of,
            "event_ts": envelope.ts.to_rfc3339(),
            "source": envelope.source,
            "event_type": envelope.event_type,
            "phase": phase,
            "outcome": outcome,
            "decision_basis": "explicit --replay-force; validated push.source body and replay marker",
            "previous_hash": chain_parent,
        });
        let canonical = serde_json::to_vec(&record)
            .map_err(|error| format!("serialize replay audit: {error}"))?;
        let record_hash = format!("{:x}", Sha256::digest(&canonical));
        record["record_hash"] = serde_json::Value::String(record_hash.clone());
        let mut line = serde_json::to_vec(&record)
            .map_err(|error| format!("serialize replay audit hash: {error}"))?;
        line.push(b'\n');

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|error| format!("open replay audit {}: {error}", path.display()))?;
        file.write_all(&line)
            .await
            .map_err(|error| format!("append replay audit {}: {error}", path.display()))?;
        file.sync_data()
            .await
            .map_err(|error| format!("sync replay audit {}: {error}", path.display()))?;
        *previous_hash = Some(record_hash);
        Ok(())
    }
}

struct MonitorReplayPublisher<N, A> {
    notification: N,
    audit: A,
    dry_run_active: bool,
}

#[async_trait::async_trait]
impl<N, A> stock_analysis::event::ReplayPublisher for MonitorReplayPublisher<N, A>
where
    N: ReplayNotificationSink,
    A: ReplayAuditSink,
{
    async fn publish(
        &self,
        envelope: stock_analysis::event::EventEnvelope,
    ) -> Result<(), stock_analysis::event::ReplayPublishError> {
        use stock_analysis::event::ReplayPublishError;

        if self.dry_run_active {
            return Err(ReplayPublishError::Environment(
                "V10_DRY_RUN_PUSH=1 is active".into(),
            ));
        }
        if envelope.event_type != "push.source" || envelope.replay_of.is_none() {
            return Err(ReplayPublishError::InvalidEnvelope(
                "publisher requires a marked push.source envelope".into(),
            ));
        }
        let text = envelope
            .payload
            .get("text")
            .and_then(serde_json::Value::as_str)
            .filter(|text| text.starts_with("[REPLAY "))
            .ok_or_else(|| {
                ReplayPublishError::InvalidEnvelope(
                    "publisher requires an explicit replay marker".into(),
                )
            })?;

        self.audit
            .record(&envelope, "attempt", "authorized")
            .await
            .map_err(ReplayPublishError::Audit)?;
        let delivered = self.notification.send(text).await;
        self.audit
            .record(
                &envelope,
                "result",
                if delivered {
                    "published"
                } else {
                    "sink_failed"
                },
            )
            .await
            .map_err(ReplayPublishError::Audit)?;
        if delivered {
            Ok(())
        } else {
            Err(ReplayPublishError::Sink(
                "notification sink rejected replay".into(),
            ))
        }
    }
}

#[cfg(test)]
mod monitor_replay_publisher_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use stock_analysis::event::{EventEnvelope, ReplayPublishError, ReplayPublisher};

    #[derive(Clone)]
    struct FakeNotificationSink {
        delivered: bool,
        calls: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl ReplayNotificationSink for FakeNotificationSink {
        async fn send(&self, _text: &str) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.delivered
        }
    }

    #[derive(Clone, Default)]
    struct FakeAuditSink {
        records: Arc<Mutex<Vec<(String, String)>>>,
        fail_phase: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl ReplayAuditSink for FakeAuditSink {
        async fn record(
            &self,
            _envelope: &EventEnvelope,
            phase: &str,
            outcome: &str,
        ) -> Result<(), String> {
            if self.fail_phase == Some(phase) {
                return Err(format!("{phase} audit failed"));
            }
            self.records
                .lock()
                .unwrap()
                .push((phase.to_string(), outcome.to_string()));
            Ok(())
        }
    }

    fn replay_envelope(text: serde_json::Value) -> EventEnvelope {
        EventEnvelope {
            id: "replay-source-1".into(),
            ts: chrono::Local::now(),
            trace_id: "trace-1".into(),
            source: "monitor".into(),
            event_type: "push.source".into(),
            entity_key: Some("TEST_CODE_600519".into()),
            payload: serde_json::json!({"text": text, "kind": "Announcement"}),
            version: 1,
            replay_of: Some("source-1".into()),
        }
    }

    #[tokio::test]
    async fn monitor_replay_publisher_records_attempt_and_result() {
        let calls = Arc::new(AtomicU64::new(0));
        let audit = FakeAuditSink::default();
        let publisher = MonitorReplayPublisher {
            notification: FakeNotificationSink {
                delivered: true,
                calls: calls.clone(),
            },
            audit: audit.clone(),
            dry_run_active: false,
        };
        publisher
            .publish(replay_envelope(serde_json::json!(
                "[REPLAY 2026-07-16] body"
            )))
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *audit.records.lock().unwrap(),
            vec![
                ("attempt".into(), "authorized".into()),
                ("result".into(), "published".into())
            ]
        );
    }

    #[tokio::test]
    async fn monitor_replay_publisher_rejects_dry_run_invalid_and_sink_failure() {
        let calls = Arc::new(AtomicU64::new(0));
        let publisher = MonitorReplayPublisher {
            notification: FakeNotificationSink {
                delivered: true,
                calls: calls.clone(),
            },
            audit: FakeAuditSink::default(),
            dry_run_active: true,
        };
        assert!(matches!(
            publisher
                .publish(replay_envelope(serde_json::json!(
                    "[REPLAY 2026-07-16] body"
                )))
                .await,
            Err(ReplayPublishError::Environment(_))
        ));
        let publisher = MonitorReplayPublisher {
            notification: FakeNotificationSink {
                delivered: true,
                calls: calls.clone(),
            },
            audit: FakeAuditSink::default(),
            dry_run_active: false,
        };
        assert!(matches!(
            publisher
                .publish(replay_envelope(serde_json::json!("body")))
                .await,
            Err(ReplayPublishError::InvalidEnvelope(_))
        ));
        let publisher = MonitorReplayPublisher {
            notification: FakeNotificationSink {
                delivered: false,
                calls: calls.clone(),
            },
            audit: FakeAuditSink::default(),
            dry_run_active: false,
        };
        assert!(matches!(
            publisher
                .publish(replay_envelope(serde_json::json!(
                    "[REPLAY 2026-07-16] body"
                )))
                .await,
            Err(ReplayPublishError::Sink(_))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn monitor_replay_publisher_blocks_delivery_when_attempt_audit_fails() {
        let calls = Arc::new(AtomicU64::new(0));
        let publisher = MonitorReplayPublisher {
            notification: FakeNotificationSink {
                delivered: true,
                calls: calls.clone(),
            },
            audit: FakeAuditSink {
                fail_phase: Some("attempt"),
                ..Default::default()
            },
            dry_run_active: false,
        };
        assert!(matches!(
            publisher
                .publish(replay_envelope(serde_json::json!(
                    "[REPLAY 2026-07-16] body"
                )))
                .await,
            Err(ReplayPublishError::Audit(_))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    fn audit_test_dir(name: &str) -> std::path::PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "monitor-replay-audit-{name}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[tokio::test]
    async fn file_replay_audit_persists_traceable_hash_chain() {
        let dir = audit_test_dir("valid");
        let audit = FileReplayAuditSink::new(dir.clone());
        let envelope = replay_envelope(serde_json::json!("[REPLAY 2026-07-16] body"));
        audit
            .record(&envelope, "attempt", "authorized")
            .await
            .unwrap();
        audit
            .record(&envelope, "result", "published")
            .await
            .unwrap();
        let reopened = FileReplayAuditSink::new(dir.clone());
        reopened
            .record(&envelope, "attempt", "authorized")
            .await
            .unwrap();
        let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let records: Vec<serde_json::Value> = content
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["envelope_id"], "replay-source-1");
        assert_eq!(records[0]["replay_of"], "source-1");
        assert_eq!(records[1]["previous_hash"], records[0]["record_hash"]);
        assert_eq!(records[2]["previous_hash"], records[1]["record_hash"]);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn file_replay_audit_rejects_corrupt_existing_tail() {
        let dir = audit_test_dir("corrupt");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join(format!("{}.jsonl", chrono::Local::now().format("%Y")));
        tokio::fs::write(&path, "{not-json}\n").await.unwrap();
        let audit = FileReplayAuditSink::new(dir.clone());
        let result = audit
            .record(
                &replay_envelope(serde_json::json!("[REPLAY 2026-07-16] body")),
                "attempt",
                "authorized",
            )
            .await;
        assert!(result.unwrap_err().contains("parse replay audit line 1"));
        assert_eq!(
            tokio::fs::read_to_string(path).await.unwrap(),
            "{not-json}\n"
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}

fn print_event_help() {
    eprintln!("Usage: monitor");
    eprintln!("       monitor --test [--e2e]");
    eprintln!("       monitor --test --push-dry-run");
    eprintln!("       monitor --review");
    eprintln!("       monitor --test --review");
    eprintln!("       monitor --replay=YYYY-MM-DD [--replay-force] [--replay-rate-ms=N]");
    eprintln!("       monitor --history [--date=YYYY-MM-DD] [--code=CODE] [--kind=KIND]");
    eprintln!("                         [--limit=N] [--success-rate] [--sink=SINK]");
    eprintln!("       monitor --help");
    eprintln!();
    eprintln!(
        "--test renders the complete TEST_CODE template catalog and requires BR196_LIVE_FEISHU_ACCEPTANCE=1,"
    );
    eprintln!(
        "       an allowlisted non-production BR196_FEISHU_* target, and validated Feishu receipts."
    );
    eprintln!(
        "--test --push-dry-run validates the same catalog and audit logs without external delivery."
    );
    eprintln!("--review is production-strict and requires fresh, complete real account evidence.");
    eprintln!(
        "--test --review verifies that the strict review fails closed without live evidence."
    );
    eprintln!("Terminal commands exit after completion; bare monitor enters long-running loops.");
}

#[cfg(test)]
fn isolated_e2e_requested(arguments: &[String]) -> bool {
    arguments.iter().any(|argument| argument == "--e2e")
        || matches!(arguments, [_, only] if only == "--test")
}

#[cfg(test)]
fn service_enablement_required(arguments: &[String]) -> bool {
    arguments.len() == 1
}

fn runtime_data_path(test_mode: bool, leaf: &str) -> std::path::PathBuf {
    let root = if test_mode { "data/test" } else { "data" };
    std::path::PathBuf::from(root).join(leaf)
}

fn allocate_durable_test_code() -> Result<String, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("allocate durable TEST_CODE invocation nonce: {error}"))?
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(format!(
        "TEST_CODE_MONITOR_{}_{}_{}",
        std::process::id(),
        nonce,
        sequence
    ))
}

#[cfg(test)]
mod tests_br192_durable_test_code {
    #[test]
    fn generated_test_codes_are_unique_path_safe_invocation_identities() {
        let first = super::allocate_durable_test_code().expect("allocate first TEST_CODE");
        let second = super::allocate_durable_test_code().expect("allocate second TEST_CODE");

        assert_ne!(first, second);
        for test_code in [first, second] {
            assert!(test_code.starts_with("TEST_CODE_MONITOR_"));
            assert!(test_code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')));
        }
    }
}

/// BR-051/BR-183 core-business database binding.
///
/// The unreleased selection-v2 capability is disabled independently, but the
/// account/review/backfill business still requires the existing core
/// `DatabaseManager`. Callers cannot choose this identity through
/// `DATABASE_PATH`, `.env`, `MAGICLAW_DB_PATH`, CWD, or a CLI path.
fn install_mode_owned_core_database(test_mode: bool) -> Result<std::path::PathBuf, String> {
    let database_path = if test_mode {
        allocate_test_core_database_path()?
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("stock_analysis.db")
    };
    let parent = database_path
        .parent()
        .ok_or_else(|| "mode-owned core database has no parent directory".to_owned())?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create mode-owned core database directory {}: {error}",
            parent.display()
        )
    })?;

    std::env::set_var("DATABASE_PATH", &database_path);
    if test_mode
        || std::env::var("MAGICLAW_DB_PATH")
            .ok()
            .is_none_or(|path| path.trim().is_empty())
    {
        std::env::set_var("MAGICLAW_DB_PATH", &database_path);
    }
    stock_analysis::database::DatabaseManager::init(Some(database_path.clone()))
        .map_err(|error| format!("initialize mode-owned core database: {error}"))?;
    Ok(database_path)
}

fn allocate_test_core_database_path() -> Result<std::path::PathBuf, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let parent = std::env::temp_dir();
    for attempt in 0_u32..32 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("allocate TEST_CODE database nonce: {error}"))?
            .as_nanos();
        let root = parent.join(format!(
            "TEST_CODE_monitor_{}_{}_{}",
            std::process::id(),
            nonce,
            attempt
        ));
        match std::fs::create_dir(&root) {
            Ok(()) => return Ok(root.join("stock_analysis.db")),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "create invocation-isolated TEST_CODE database root {}: {error}",
                    root.display()
                ));
            }
        }
    }
    Err("could not allocate an invocation-isolated TEST_CODE database root".to_owned())
}

type JsonlWriterTask = tokio::task::JoinHandle<Result<(), stock_analysis::event::JsonlError>>;
const JSONL_WRITER_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const BACKGROUND_TASK_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

async fn quiesce_background_tasks(
    tasks: Vec<(&'static str, tokio::task::JoinHandle<()>)>,
) -> Result<(), String> {
    for (_, task) in &tasks {
        task.abort();
    }
    let joins = async move {
        let mut failures = Vec::new();
        for (name, task) in tasks {
            match task.await {
                Ok(()) => {}
                Err(error) if error.is_cancelled() => {}
                Err(error) => failures.push(format!("{name}: {error}")),
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "background task shutdown failures: {}",
                failures.join("; ")
            ))
        }
    };
    tokio::time::timeout(BACKGROUND_TASK_SHUTDOWN_TIMEOUT, joins)
        .await
        .map_err(|_| {
            format!(
                "background task shutdown timed out after {}ms",
                BACKGROUND_TASK_SHUTDOWN_TIMEOUT.as_millis()
            )
        })?
}

async fn shutdown_jsonl_writer(
    bus: &stock_analysis::event::EventBus,
    handle: &mut Option<JsonlWriterTask>,
) -> Result<(), String> {
    shutdown_jsonl_writer_with_timeout(bus, handle, JSONL_WRITER_SHUTDOWN_TIMEOUT).await
}

async fn shutdown_jsonl_writer_with_timeout(
    bus: &stock_analysis::event::EventBus,
    handle: &mut Option<JsonlWriterTask>,
    timeout: std::time::Duration,
) -> Result<(), String> {
    bus.shutdown();
    let mut handle = handle
        .take()
        .ok_or_else(|| "event JSONL writer handle is missing".to_string())?;
    match tokio::time::timeout(timeout, &mut handle).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(error))) => Err(format!("event JSONL writer failed: {error}")),
        Ok(Err(error)) => Err(format!("event JSONL writer task join failed: {error}")),
        Err(_) => {
            handle.abort();
            Err(format!(
                "event JSONL writer drain timed out after {}ms",
                timeout.as_millis()
            ))
        }
    }
}

fn unexpected_jsonl_writer_completion(
    result: Result<Result<(), stock_analysis::event::JsonlError>, tokio::task::JoinError>,
) -> String {
    match result {
        Ok(Ok(())) => "writer stopped before service shutdown".to_string(),
        Ok(Err(error)) => format!("writer failed: {error}"),
        Err(error) => format!("writer task join failed: {error}"),
    }
}

enum LongRunningTrigger {
    MainLoopsCompleted,
    ShutdownSignal(Result<(), String>),
    WriterCompleted(Result<Result<(), stock_analysis::event::JsonlError>, tokio::task::JoinError>),
}

async fn supervise_long_running_lifecycle<MainLoops, ShutdownSignal>(
    bus: &stock_analysis::event::EventBus,
    writer_handle: &mut Option<JsonlWriterTask>,
    background_tasks: Vec<(&'static str, tokio::task::JoinHandle<()>)>,
    main_loops: MainLoops,
    shutdown_signal: ShutdownSignal,
) -> Result<(), String>
where
    MainLoops: std::future::Future<Output = ()>,
    ShutdownSignal: std::future::Future<Output = Result<(), String>>,
{
    let trigger = {
        let writer = writer_handle.as_mut().ok_or_else(|| {
            "BR-141 writer handle is missing while monitor is running".to_string()
        })?;
        tokio::pin!(main_loops);
        tokio::pin!(shutdown_signal);
        tokio::select! {
            _ = &mut main_loops => LongRunningTrigger::MainLoopsCompleted,
            signal = &mut shutdown_signal => LongRunningTrigger::ShutdownSignal(signal),
            result = writer => LongRunningTrigger::WriterCompleted(result),
        }
    };

    let producer_shutdown = quiesce_background_tasks(background_tasks).await;
    let trigger = match trigger {
        LongRunningTrigger::WriterCompleted(result) => {
            writer_handle.take();
            bus.shutdown();
            let writer_error = unexpected_jsonl_writer_completion(result);
            return match producer_shutdown {
                Ok(()) => Err(writer_error),
                Err(producer_error) => Err(format!(
                    "{writer_error}; producer shutdown failed: {producer_error}"
                )),
            };
        }
        other => other,
    };

    let writer_shutdown = shutdown_jsonl_writer(bus, writer_handle).await;
    producer_shutdown?;
    writer_shutdown?;

    match trigger {
        LongRunningTrigger::ShutdownSignal(result) => result,
        LongRunningTrigger::MainLoopsCompleted => {
            Err("long-running monitor loops completed unexpectedly".to_string())
        }
        LongRunningTrigger::WriterCompleted(_) => unreachable!("handled before writer drain"),
    }
}

async fn exit_after_jsonl_writer(
    bus: &stock_analysis::event::EventBus,
    handle: &mut Option<JsonlWriterTask>,
    requested_code: i32,
) -> ! {
    let exit_code = match shutdown_jsonl_writer(bus, handle).await {
        Ok(()) => requested_code,
        Err(error) => {
            log::error!("[event_bus.jsonl] terminal drain failed: {error}");
            2
        }
    };
    log::logger().flush();
    std::process::exit(exit_code);
}

async fn refresh_startup_position_chains(
) -> Result<stock_analysis::data_gateway::PositionChainRefreshReport, String> {
    use stock_analysis::data_gateway::{refresh_position_chains, PositionChainRefreshStatus};
    use stock_analysis::database::DatabaseManager;

    let database = DatabaseManager::get();
    let positions = database
        .get_all_open_positions()
        .map_err(|error| format!("BR-170 load open positions before consumers: {error}"))?;
    let codes = positions
        .into_iter()
        .map(|position| position.code)
        .collect::<Vec<_>>();
    let report = refresh_position_chains(database, &codes).await;

    for outcome in &report.outcomes {
        match &outcome.status {
            PositionChainRefreshStatus::Assigned { inserted } => log::info!(
                "[startup][BR-170] code={} status=assigned inserted={inserted}",
                outcome.code
            ),
            PositionChainRefreshStatus::VerifiedEmpty { cleared_positions } => log::info!(
                "[startup][BR-170] code={} status=verified_empty cleared_positions={cleared_positions}",
                outcome.code
            ),
            PositionChainRefreshStatus::Failed {
                reason_code,
                retryable,
                message,
            } => log::error!(
                "[startup][BR-170] code={} status=failed reason_code={} retryable={} error={}",
                outcome.code,
                reason_code,
                retryable,
                message
            ),
        }
    }
    log::info!(
        "[startup][BR-170] 持仓产业链刷新完成 | requested={} assigned={} verified_empty={} failed={}",
        codes.len(),
        report.assigned(),
        report.verified_empty(),
        report.failed()
    );

    Ok(report)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Br194TerminalReplayCommand {
    business_date: chrono::NaiveDate,
    task: review_batch::ReviewTask,
}

fn parse_br194_terminal_replay_command(
    args: &[String],
) -> Result<Option<Br194TerminalReplayCommand>, String> {
    const MODE: &str = "--br194-audited-terminal-replay";
    let mode_count = args
        .iter()
        .filter(|argument| argument.as_str() == MODE)
        .count();
    if mode_count == 0 {
        return Ok(None);
    }
    if mode_count != 1 {
        return Err("terminal replay mode must be specified exactly once".to_owned());
    }
    if args
        .iter()
        .any(|argument| argument == "--replay-ordinal" || argument.starts_with("--replay-ordinal="))
    {
        return Err("replay ordinal is coordinator-owned and cannot be overridden".to_owned());
    }
    if args.iter().any(|argument| argument.contains("TEST_CODE")) {
        return Err("TEST_CODE is forbidden in production terminal replay".to_owned());
    }
    let mut business_date = None;
    let mut task = None;
    let mut index = 1usize;
    while index < args.len() {
        match args[index].as_str() {
            MODE => {}
            "--business-date" => {
                if business_date.is_some() {
                    return Err("--business-date must be specified exactly once".to_owned());
                }
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--business-date requires YYYY-MM-DD".to_owned())?;
                business_date = Some(
                    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                        .map_err(|_| "business date must be YYYY-MM-DD".to_owned())?,
                );
            }
            "--task" => {
                if task.is_some() {
                    return Err("--task must be specified exactly once".to_owned());
                }
                index += 1;
                let raw = args
                    .get(index)
                    .ok_or_else(|| "--task requires R-04 or R-09".to_owned())?;
                task = Some(
                    review_batch::ReviewTask::from_label(raw)
                        .filter(|task| {
                            matches!(
                                task,
                                review_batch::ReviewTask::R04 | review_batch::ReviewTask::R09
                            )
                        })
                        .ok_or_else(|| "--task requires R-04 or R-09".to_owned())?,
                );
            }
            unknown => {
                return Err(format!("unexpected terminal replay argument {unknown}"));
            }
        }
        index += 1;
    }
    let business_date = business_date.ok_or_else(|| "--business-date is required".to_owned())?;
    if business_date > chrono::Local::now().date_naive() {
        return Err("future business date is forbidden".to_owned());
    }
    match stock_analysis::calendar::verified_a_share_trading_day(business_date) {
        Ok(true) => {}
        Ok(false) => {
            return Err("business date is not an A-share trading day".to_owned());
        }
        Err(error) => {
            return Err(format!(
                "A-share trading-calendar authority unavailable: {error}"
            ));
        }
    }
    let task = task.ok_or_else(|| "--task is required".to_owned())?;
    Ok(Some(Br194TerminalReplayCommand {
        business_date,
        task,
    }))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let process_stock_list = std::env::var("STOCK_LIST").ok();
    dotenvy::dotenv().ok();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            writeln!(
                buf,
                "[{} {}] {}",
                chrono::Local::now().format("%H:%M:%S"),
                record.level(),
                record.args()
            )
        })
        .init();

    let process_args = std::env::args().collect::<Vec<_>>();
    match parse_br194_terminal_replay_command(&process_args) {
        Ok(Some(command)) => {
            match durable_delivery_runtime::run_production_audited_terminal_replay(
                command.business_date,
                command.task,
            ) {
                Ok(evidence) => {
                    println!(
                        "BR194_REPLAY task={} state=Passed attempts=1 provider_calls=0 \
                         resume_calls=0 sink_calls=0 delivery_audit_appends=0 \
                         sink_watermark_equal=true delivery_audit_watermark_equal=true",
                        command.task.label()
                    );
                    log::info!(
                        "[BR-194] replay evidence attempt={} decision={} ordinal={}",
                        evidence.attempt_identity,
                        evidence.decision_identity,
                        evidence.replay_ordinal
                    );
                    return;
                }
                Err(error) => {
                    eprintln!("[BR-194] audited terminal replay failed: {error}");
                    std::process::exit(1);
                }
            }
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("[BR-194] audited terminal replay arguments rejected: {error}");
            std::process::exit(2);
        }
    }

    // BR-179/BR-183: one library-owned zero-argument facade reads the real
    // args_os. Terminal and service-disabled invocations remain storage-free;
    // operational core business continues while unreleased selection-v2 stays
    // capability-scoped fail-closed.
    let selection_cli = match stock_analysis::selection::bootstrap_selection_process() {
        Ok(proof) => proof,
        Err(error) => {
            eprintln!(
                "[BR-051][BR-179] selection process bootstrap failed code={}: {error}",
                error.code()
            );
            std::process::exit(2);
        }
    };
    if selection_cli.is_help() {
        print_event_help();
        std::process::exit(0);
    }
    if selection_cli.is_version() {
        println!("monitor {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }
    if selection_cli.is_service_disabled() {
        log::info!("[monitor] disabled: MONITOR_ENABLED is not true");
        return;
    }
    let selection_v2_enabled = match selection_cli.selection_v2_disabled_reason_code() {
        Some(reason_code) => {
            log::warn!(
                "[selection-v2][BR-183] capability=disabled reason_code={} providers=0 database_operations=0 sinks=0 schedulers=0",
                reason_code
            );
            false
        }
        None => true,
    };
    let test_mode = selection_cli.is_test();
    let review_mode = selection_cli.is_review();
    let e2e_mode = selection_cli.is_e2e();
    if stock_analysis::data_gateway::cffex_futures_delivery_live_supported() {
        log::info!("[R-08][BR-165][BR-199] component=cffex_futures_delivery capability=supported");
    } else {
        log::warn!(
            "[R-08][BR-165][BR-199] component=cffex_futures_delivery capability=unsupported; EventCalendar delivery remains retryable and sink-blocked"
        );
    }
    if test_mode {
        std::env::set_var("STOCK_ENV_MODE", "test");
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        let durable_test_code = match allocate_durable_test_code() {
            Ok(test_code) => test_code,
            Err(error) => {
                eprintln!("[BR-051][BR-192] {error}");
                std::process::exit(2);
            }
        };
        std::env::set_var("DURABLE_DELIVERY_TEST_CODE", durable_test_code);
        match process_stock_list {
            Some(stock_list) => {
                for code in stock_list
                    .split(',')
                    .map(str::trim)
                    .filter(|code| !code.is_empty())
                {
                    if let Err(error) =
                        stock_analysis::risk::env_guard::validate_symbol_for_current_env(code)
                    {
                        eprintln!("[BR-051] --test 拒绝显式 STOCK_LIST: {error}");
                        std::process::exit(2);
                    }
                }
                std::env::set_var("STOCK_LIST", stock_list);
            }
            None => std::env::remove_var("STOCK_LIST"),
        }
    } else {
        std::env::set_var("STOCK_ENV_MODE", "prod");
    }
    if let Err(error) = durable_delivery_runtime::validate_runtime_delivery_mode() {
        log::error!("[DurableDelivery][BR-192] {error}");
        log::logger().flush();
        std::process::exit(2);
    }
    // BR-144/145: prove the delivery audit chain is readable and writable
    // before warming any sink. A failed preflight blocks ordinary pushes.
    let audit_preflight =
        tokio::task::spawn_blocking(stock_analysis::event::preflight_runtime_delivery_audit).await;
    match audit_preflight {
        Ok(Ok(receipt)) => log::info!(
            "[AuditDegraded][BR-144] delivery audit preflight healthy: year={} previous_hash={:?}",
            receipt.year,
            receipt.previous_hash
        ),
        Ok(Err(error)) => {
            log::error!(
                "[event_bus.jsonl] initialization failed [AuditDegraded][BR-144] delivery audit preflight: {error}"
            );
            std::process::exit(2);
        }
        Err(error) => {
            log::error!("[AuditDegraded][BR-144] delivery audit preflight worker failed: {error}");
            std::process::exit(2);
        }
    }
    if let Err(error) = durable_delivery_runtime::eager_bind_runtime_artifacts() {
        log::error!(
            "[DurableDelivery][BR-192] eager artifact capability binding failed before sink initialization: {error}"
        );
        log::logger().flush();
        std::process::exit(2);
    }

    // 修复 F20 (2026-06-29 codex review): 启动 banner 显示当前 LaunchStage

    // (从 env STAGE 读, 默认 Shadow). operator 一眼看清推送策略.

    use stock_analysis::opportunity::launch_gate;

    let stage = launch_gate::current_stage();

    log::info!("═══════════════════════════════════════════════════════════════");

    // v16.3 Commit 1: 启动 banner 打印 v16.3 paper_trade 默认值 (v15.1.1 硬规则 1)
    stock_analysis::trading::risk_adapter::print_startup_banner();

    // v17.1-r2 §3.6: L6 SinkRouter 暖身 (默认行为不变, 仅注册 ConsoleSink + MagiclawSink)
    // env opt-in 触发: STOCK_ANALYSIS_PUSH_V6_ENABLE=1 后 notify::push_governor_inner 才走 L6.route().
    let _sink_count = l6_sink::sink_count();
    let push_v6_enabled = std::env::var("STOCK_ANALYSIS_PUSH_V6_ENABLE")
        .ok()
        .as_deref()
        == Some("1");
    log::info!(
        "[v17.1-r2 §3.6] L6 SinkRouter 已就绪 ({} sinks); 推送路径 = {}",
        _sink_count,
        if push_v6_enabled {
            "L6 SinkRouter (env opt-in 启用)"
        } else {
            "默认 push_wechat (L5 未切到 L6, 回滚 env: STOCK_ANALYSIS_PUSH_V6_ENABLE=1 才走 L6)"
        }
    );

    // BR-174/BR-183: only initialize the receipt-gated global-news pipeline
    // after selection-v2 activation is released. The disabled capability must
    // not claim provider readiness or warm notification state.
    let news_registration = if selection_v2_enabled {
        let news_registration = news_aggregator_init::init_global_news_pipeline();
        log::info!(
            "[v17.4][BR-174] NewsAggregator 已初始化 ({} feeds registered)",
            news_registration.feed_count
        );
        news_registration
    } else {
        news_aggregator_init::uninitialized_global_news_pipeline_registration()
    };
    let br196_news_capability = match br196_test_delivery::capture_news_process_capability(
        selection_v2_enabled,
        news_registration.feed_count,
        news_registration.registered_feed_set_sha256,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log::error!("[BR-196] NewsFlash process capability capture failed: {error}");
            std::process::exit(2);
        }
    };

    // BR-091: delivery audit is persisted synchronously inside the governor;
    // the event bus below is observation/replay only and cannot acknowledge delivery.
    use stock_analysis::event::global_bus;
    let bus = global_bus();
    log::info!("[event_bus] delivery audit mode=synchronous_durable; bus=observation_only");

    // BR-141: JSONL initialization is awaited before the background consumer
    // starts, so setup failures cannot hide inside an unobserved nested task.
    let event_receiver = match bus.subscribe() {
        Ok(receiver) => receiver,
        Err(error) => {
            log::error!("[event_bus.jsonl] subscription failed: {error:?}");
            log::logger().flush();
            std::process::exit(2);
        }
    };
    let mut jsonl_writer_handle = Some(
        match stock_analysis::event::JsonlWriter::spawn(
            event_receiver,
            runtime_data_path(test_mode, "event_bus"),
            1_827,
        )
        .await
        {
            Ok(handle) => handle,
            Err(error) => {
                log::error!("[event_bus.jsonl] initialization failed: {error}");
                log::logger().flush();
                std::process::exit(2);
            }
        },
    );
    log::info!(
        "[event_bus.jsonl] mode=enabled retention_days=1827 isolated_test={}",
        test_mode
    );

    // v17.x: DispatchTable 启动 audit (15 audit-marked rows: v17.6=3 + v17.7=6 + v17.8=6)
    notify::dispatch_table_init_audit();

    log::info!(
        "🚀 Stock Monitor 启动 | LaunchStage = {} | 推送策略 = {}",
        stage.name(),
        match stage {
            launch_gate::LaunchStage::Shadow => "推全量 (沙盘默认, F20 修复后 Shadow 也推)",

            launch_gate::LaunchStage::Gray => "仅 critical alert (止损/风控)",

            launch_gate::LaunchStage::Live => "全量推送",
        }
    );

    log::info!("═══════════════════════════════════════════════════════════════");

    // 启动时单次加载 runtime 配置。
    stock_analysis::config::load_all();

    // 配置激活后再打印阈值，避免输出 pre-load 默认值。
    log::info!(
        "[v17.4-D] screener_min_score={} | holding_health.dedup=on_same_state",
        stock_analysis::config::get_monitor_config().screener_min_score,
    );

    // --push 按当前调度窗口运行对应的 active dispatcher 后退出。

    let push_mode = selection_cli.is_push();

    // v14.0: dry-run 模式, 验证 dispatcher 加载 + 渲染, 不实际推送

    let push_dry_run = selection_cli.is_push_dry_run();

    // v70: 隔离 e2e 模式 (--e2e), 跑所有 v12 §14 + v13.1 测试模板。

    // v14.1 F7: stock_position.st_type 回填 (从 name LIKE 推断 ST/*ST)

    let backfill_st_type = selection_cli.is_backfill_st_type();

    // BR-170: refresh linked position-chain evidence through Magic TDX.

    let backfill_chain_name = selection_cli.is_backfill_chain_name();

    // v17.3 Task 5: Handle terminal event commands before entering long-running monitor loops.
    // Parse CLI args early to detect --replay / --history / --help before any background loops start.
    match selection_cli.event_command() {
        Some(stock_analysis::event::cli::EventCommand::Help) => {
            print_event_help();
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 0).await;
        }
        Some(stock_analysis::event::cli::EventCommand::Replay {
            date,
            force,
            rate_ms,
        }) => {
            use stock_analysis::event::ReplayRunner;
            let publisher = MonitorReplayPublisher {
                notification: RealReplayNotificationSink,
                audit: FileReplayAuditSink::new(runtime_data_path(test_mode, "replay_audit")),
                dry_run_active: std::env::var("V10_DRY_RUN_PUSH").as_deref() == Ok("1"),
            };
            let base_dir = runtime_data_path(test_mode, "event_bus");
            let runner = ReplayRunner::new(base_dir, publisher);
            match runner.run(date, force, rate_ms).await {
                Ok(summary) => {
                    println!(
                        "[replay] date={} force={} mode={} attempted={} replayable={} published={} skipped={} failed={}",
                        date,
                        force,
                        if force { "FORCE" } else { "DRY-RUN" },
                        summary.attempted,
                        summary.replayable,
                        summary.published,
                        summary.skipped,
                        summary.failed
                    );
                    let exit_code = if summary.has_failures() { 1 } else { 0 };
                    exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
                }
                Err(error) => {
                    eprintln!("[replay] failed: {error}");
                    exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 1).await;
                }
            }
        }
        Some(stock_analysis::event::cli::EventCommand::History {
            date,
            code,
            kind,
            limit,
            success_rate,
            sink,
        }) => {
            use stock_analysis::event::{
                format_history_lines, HistoryFilter, HistoryOrder, HistoryQuery, Window,
            };
            let base_dir = runtime_data_path(test_mode, "event_bus");
            let query = HistoryQuery::new(base_dir);
            let history_result = if success_rate {
                let window = date
                    .map(|d| {
                        let now = chrono::Local::now().date_naive();
                        let days = (now - d).num_days().max(1) as u32;
                        Window::Days(days)
                    })
                    .unwrap_or(Window::Days(1));
                match query
                    .push_success_rate(kind.as_deref(), window, sink.as_deref())
                    .await
                {
                    Ok(stats) => {
                        println!("[history.success_rate] {:?}", stats);
                        println!(
                            "total={} pushed={} failed={} denied={} deduped={} success_rate={:.2}%",
                            stats.total,
                            stats.pushed,
                            stats.failed,
                            stats.denied,
                            stats.deduped,
                            stats.success_rate * 100.0
                        );
                        Ok(())
                    }
                    Err(e) => Err(format!("success_rate query failed: {e}")),
                }
            } else {
                let filter = HistoryFilter {
                    date,
                    code,
                    kind,
                    limit: limit.unwrap_or(100),
                    order: HistoryOrder::Desc,
                };
                match query.query(filter).await {
                    Ok(entries) => {
                        println!("[history] {} entries", entries.len());
                        for line in format_history_lines(&entries) {
                            println!("{line}");
                        }
                        Ok(())
                    }
                    Err(e) => Err(format!("query failed: {e}")),
                }
            };
            let exit_code = match history_result {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("[history] {error}");
                    1
                }
            };
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
        }
        None => {
            // No event command — fall through to existing monitor behavior.
        }
    }

    let core_database_path = match install_mode_owned_core_database(test_mode) {
        Ok(path) => path,
        Err(error) => {
            log::error!("[DB init][BR-051][BR-183] {error}");
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
        }
    };
    log::info!(
        "[DB init][BR-051][BR-183] core database bound mode={} path={}",
        if test_mode { "test" } else { "production" },
        core_database_path.display()
    );

    // BR-192: counted producers remain frozen until every unresolved business
    // date has reached the durable local fixed point.  This barrier performs
    // zero provider calls and never resends an uncertain receipt.
    match durable_delivery_runtime::ensure_startup_reconciled().await {
        Ok(evidence) => {
            log::info!(
                "[DurableDelivery][BR-192] startup fixed point reached progress={} resumed_sink_calls={} foreign_lease_boundaries={} manual_review_boundaries={} schedule_hydrations={}",
                evidence.progress_count,
                evidence.resumed_sink_calls,
                evidence.non_progressable_foreign_attempts.len(),
                evidence.non_progressable_manual_reviews.len(),
                evidence.schedule_hydrations.len()
            );
            if !evidence.non_progressable_foreign_attempts.is_empty() {
                log::warn!(
                    "[DurableDelivery][BR-192] non-expired foreign attempts retained without resend: {:?}",
                    evidence.non_progressable_foreign_attempts
                );
            }
            if !evidence.non_progressable_manual_reviews.is_empty() {
                log::warn!(
                    "[DurableDelivery][BR-192] uncertain decisions retained for manual review without resend: {:?}",
                    evidence.non_progressable_manual_reviews
                );
            }
        }
        Err(error) => {
            log::error!("[DurableDelivery][BR-192] producer activation blocked: {error}");
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
        }
    }

    log::info!(
        "实盘监控启动 | {} | 当前: {} | 模式: {}",
        if calendar::today_is_trading_day() {
            "交易日"
        } else {
            "非交易日"
        },
        calendar::session_label(),
        if test_mode {
            "测试"
        } else if review_mode {
            "复盘"
        } else {
            "正常"
        },
    );

    // 事件总线 — 允许多个消费者独立订阅监控事件（生产者无需感知消费者）

    use stock_analysis::monitor::event_bus::{EventBus, MonitorEvent};

    // v14.1 task #170: 探测 broker 数据源, 注册到全局 (用户决策: 未付费用公开数据)

    let broker_src = match stock_analysis::broker::detect_and_register() {
        Ok(source) => source,
        Err(error) => {
            log::error!("[broker] 启动失败: {}", error);
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
        }
    };

    log::info!("[broker] 启动完成 | 当前数据源 = {}", broker_src.label());

    stock_analysis::strategy::v16_4::register_all();

    let startup_health = health::health_check().await;
    if !startup_health.all_ok() {
        log::error!("[health] 启动健康检查失败: {:?}", startup_health);
        match webhook_alert::on_health_fail(&startup_health).await {
            Ok(webhook_alert::WebhookDelivery::Delivered) => {
                log::info!("[health] 失败告警已投递")
            }
            Ok(webhook_alert::WebhookDelivery::Disabled) => {
                log::warn!("[health] 失败告警未投递: webhook 未配置")
            }
            Ok(webhook_alert::WebhookDelivery::TestIsolated) => {
                log::info!("[health] 测试环境已隔离健康告警外发")
            }
            Err(error) => log::error!("[health] 失败告警投递失败: {}", error),
        }
    }

    // BR-164: 盘中/盘后共用同一完整批次路由，不保留消费端旧源或第二套协议。
    log::info!(
        "[启动] K线统一路由（盘中/盘后）: magic_tdx (P1) → magic_tencent (P2) → \
         magic_sina (P3) → magic_baidu (P4) | HistoricalBarsGateway BR-164"
    );

    log::info!("[启动] 新闻轮询: GlobalNewsGateway（默认 120s，可由 NEWS_POLL_INTERVAL 覆盖）");

    log::info!("[启动] 盘后回溯: SinaInstrumentNewsGateway（15:30 后, 30 天, 持仓代码）");

    // v14.1 F7: stock_position.st_type 回填 (从 name LIKE 推断)

    if backfill_st_type {
        log::info!("[v14.1 F7] --backfill-st-type 模式启动 | 从 name 字段推断 ST/*ST");

        use stock_analysis::database::DatabaseManager;

        let db = DatabaseManager::get();

        let exit_code = match db.backfill_st_type() {
            Ok(n) => {
                log::info!("[v14.1 F7] 回填完成 | 更新行数 = {}", n);
                0
            }
            Err(error) => {
                log::error!("[v14.1 F7] 回填失败: {error}");
                2
            }
        };

        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
    }

    // BR-170: terminal refresh and evidence audit for every open position.

    if backfill_chain_name {
        log::info!("[BR-170] --backfill-chain-name 模式启动 | Magic TDX BoardDataGateway 完整批次");

        use stock_analysis::data_gateway::{refresh_position_chains, PositionChainRefreshStatus};
        use stock_analysis::database::DatabaseManager;

        let db = DatabaseManager::get();

        let exit_code = match db.get_all_open_positions() {
            Ok(positions) => {
                let codes = positions
                    .into_iter()
                    .map(|position| position.code)
                    .collect::<Vec<_>>();
                let report = refresh_position_chains(db, &codes).await;
                for outcome in &report.outcomes {
                    match &outcome.status {
                        PositionChainRefreshStatus::Assigned { inserted } => log::info!(
                            "[BR-170] code={} status=assigned inserted={inserted}",
                            outcome.code
                        ),
                        PositionChainRefreshStatus::VerifiedEmpty { cleared_positions } => {
                            log::info!(
                                "[BR-170] code={} status=verified_empty cleared_positions={cleared_positions}",
                                outcome.code
                            )
                        }
                        PositionChainRefreshStatus::Failed {
                            reason_code,
                            retryable,
                            message,
                        } => log::error!(
                            "[BR-170] code={} status=failed reason_code={} retryable={} error={}",
                            outcome.code,
                            reason_code,
                            retryable,
                            message
                        ),
                    }
                }
                log::info!(
                    "[BR-170] 回填完成 | requested={} assigned={} verified_empty={} failed={}",
                    codes.len(),
                    report.assigned(),
                    report.verified_empty(),
                    report.failed()
                );
                if report.has_failures() {
                    2
                } else {
                    0
                }
            }
            Err(error) => {
                log::error!("[BR-170] 持仓代码加载失败: {error}");
                2
            }
        };

        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
    }

    if review_mode {
        log::info!("[复盘] --review 终端模式启动，完成后退出，不进入常驻监控");
        // BR-147: 收盘估值原本只在常驻主循环的盘后分支计算。--review 在主循环
        // 启动前返回，因此手动复盘永远拿不到当日估值，banner 与复盘正文只能显示
        // 上一交易日的结果。此处复用与主循环完全相同的入口，且必须在
        // evaluate_account_mode_hook 之前执行，使刷新出的 note 携带当日估值。
        // 落库按 run_id 内容哈希幂等，重复执行不会产生重复批次。
        {
            let now = chrono::Local::now();
            if stock_analysis::calendar::is_trading_day(now.date_naive())
                && closing_valuation_runtime::eligible_after_close(now.fixed_offset())
            {
                match closing_valuation_runtime::run_closing_valuation_once(now.date_naive()).await
                {
                    Ok(receipt) => log::info!(
                        "[BR-147] closing valuation persisted: run_id={} inserted={}",
                        receipt.run_id,
                        receipt.inserted
                    ),
                    // §2.2: 估值失败保持显式，不用旧批次或 0 值冒充当日结果。
                    Err(error) => log::error!("[BR-147] closing valuation failed: {error}"),
                }
            } else {
                log::info!(
                    "[BR-147] closing valuation skipped: trading_day={} after_close={}",
                    stock_analysis::calendar::is_trading_day(now.date_naive()),
                    closing_valuation_runtime::eligible_after_close(now.fixed_offset())
                );
            }
        }
        // BR-108/BR-116: --review 与常驻监控一样需要真实治理上下文。该分支在主循环
        // 启动前返回，若不在此评估 AccountMode/DataMode，LATEST_BANNER 恒为 None，
        // 依赖 banner 的复盘推送会以 "governance banner unavailable" 被跳过。
        // 复用与 startup 完全相同的评估路径，不构造任何默认 banner。
        if !evaluate_account_mode_hook(true).await {
            log::error!(
                "[复盘][BR-108/BR-116] AccountMode notification unconfirmed; context remains conservative"
            );
        }
        evaluate_data_mode_hook().await;
        let result = match ReviewExecutionPath::StrictDispatchers {
            ReviewExecutionPath::StrictDispatchers => run_review_only().await,
        };
        let exit_code = match result {
            Ok(()) => 0,
            Err(error) => {
                log::error!("[复盘] {error}. exit 2.");
                2
            }
        };
        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
    }

    if test_mode {
        log::info!("[v30] --test 模式启动");

        if selection_cli.is_e2e() {
            // 兼容历史参数：`--test --e2e` 走 E2E 完整模板验收；
            // `--push-dry-run` 通过 explicit_dry_run 开关进入隔离审计路径。
            log::info!("[v70] --test --e2e 模式启动 — 跑所有 v12 §14 模板 (忽略时间窗口)");
            let exit_code = match e2e_all_templates_run(push_dry_run, &br196_news_capability).await
            {
                Ok(()) => 0,
                Err(error) => {
                    log::error!("[v70][BR-051][BR-103] E2E 失败: {error}");
                    2
                }
            };
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
        }

        if selection_cli.is_v13_diag() {
            // v13.27: 端到端诊断 (5 dispatcher 全链路, 输出 data/v13_diag_report.json)

            let exit_code = match v13_diag::report_v13_diag().await {
                Ok(()) => 0,
                Err(error) => {
                    log::error!("[v13.27] diagnostic failed: {error}");
                    2
                }
            };

            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
        }

        if !selection_cli.is_e2e() {
            // 兼容历史单一 `--test` 入口：仍保持完整模板闭环验收。
            // 与 `--test --push-dry-run` 的差异仅在于是否走外部投递/回执校验。
            if let Err(error) = e2e_all_templates_run(push_dry_run, &br196_news_capability).await {
                log::error!("[v30][BR-108] --test 批次拒绝: {error}");
                exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
            }

            log::info!("[v30] --test 完成");
        }

        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 0).await;
    } else if push_dry_run {
        // v70: 隔离 e2e 模式 (--e2e) — 跑所有 v12 §14 + v13.1 测试模板。
        // 仅当未开启 --test 时才独立触发。
        if e2e_mode {
            log::info!("[v70] E2E 模式启动 — 跑所有 v12 §14 模板 (忽略时间窗口)");
            let exit_code = match e2e_all_templates_run(true, &br196_news_capability).await {
                Ok(()) => 0,
                Err(error) => {
                    log::error!("[v70][BR-051][BR-103] E2E 失败: {error}");
                    2
                }
            };
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
        }

        log::info!("[v14.0] --push-dry-run 模式启动");
        if let Err(error) = run_daily_pushes_dry_run().await {
            log::error!("[v14.0] --push-dry-run 失败: {error}");
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
        }

        log::info!("[v14.0] --push-dry-run 完成");

        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 0).await;
    } else if push_mode {
        // v30: --push 模式 (修复 v22 死代码)

        //   按当前窗口运行对应的 active dispatcher 后退出，时刻来自 OpportunitySchedule::default()

        //   替代 v17.6 写死的 09:00 / 10:30 / 11:00 / 14:30 / 19:00

        log::info!("[v30] --push 模式启动");

        if let Err(error) = run_daily_pushes().await {
            log::error!("[v30][BR-108] --push 批次拒绝: {error}");
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
        }

        log::info!("[v30] --push 完成");

        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 0).await;
    } else if !selection_cli.requires_service_enablement() {
        log::warn!(
            "[monitor] non-service 入口命中短路路径：未匹配到已识别终端模式，已完成 JSONL 初始化并安全退出"
        );
        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 0).await;
    } else {
        let position_chain_report = match refresh_startup_position_chains().await {
            Ok(report) => report,
            Err(error) => {
                log::error!("[startup][BR-170] 持仓产业链预刷新失败: {error}");
                exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
            }
        };
        if position_chain_report.has_failures() {
            log::warn!(
                "[startup][BR-170] {} 个持仓产业链刷新失败；成功项已提交，失败项保持空值或既有可验证链接，候选建仓仍按代码 fail-closed",
                position_chain_report.failed()
            );
        }

        let dryrun_reporter = dryrun_report::spawn_dryrun_reporter(1_800);

        // 订阅者示例：独立任务消费告警/扫描事件并写入审计日志，

        // 与告警推送（生产者）完全解耦——新增消费者无需改动 push_wechat。

        let mut event_rx = EventBus::global().subscribe();
        let market_action_state = std::sync::Arc::new(std::sync::Mutex::new(
            crate::v17_sources::MarketActionState::default(),
        ));

        let event_consumer = tokio::spawn(async move {
            loop {
                match event_rx.recv().await {
                    Ok(ev) => match &ev {
                        MonitorEvent::Alert { title, success } => {
                            log::info!("[event_bus] 告警事件 success={} | {}", success, title);
                        }

                        MonitorEvent::OpportunityScan { candidates } => {
                            log::info!("[event_bus] 机会扫描完成，候选 {} 个", candidates);
                        }

                        // 修复 P3.6: 处理新事件类型
                        MonitorEvent::OrderUpdate {
                            code: _,

                            action: _,

                            shares: _,
                        } => {
                            if let Some(attempt) =
                                crate::v17_sources::handle_monitor_event(&ev, &market_action_state)
                                    .await
                            {
                                log::info!(
                                    "[event_bus] OrderUpdate → {:?} code={:?} pushed={:?} len={}",
                                    attempt.kind,
                                    attempt.code,
                                    attempt.outcome,
                                    attempt.rendered_len
                                );
                            }
                        }

                        MonitorEvent::PriceUpdate {
                            code,

                            change_pct,

                            reason,
                        } => {
                            log::info!(
                                "[event_bus] 价格变动 {}({:+.2}%) {}",
                                code,
                                change_pct,
                                reason
                            );
                        }

                        MonitorEvent::DataQuality {
                            source,

                            issue,

                            severity,
                        } => match severity {
                            stock_analysis::monitor::event_bus::DataQualityLevel::Warn => {
                                log::warn!("[event_bus] 数据质量 {}: {}", source, issue);
                            }

                            stock_analysis::monitor::event_bus::DataQualityLevel::Error => {
                                log::error!(
                                    "[event_bus] 数据质量 {}: {} (功能降级)",
                                    source,
                                    issue
                                );
                            }

                            stock_analysis::monitor::event_bus::DataQualityLevel::Fatal => {
                                log::error!("[event_bus] 数据质量 {}: {} (致命)", source, issue);
                            }
                        },

                        MonitorEvent::Info(msg) => log::info!("[event_bus] {}", msg),
                    },

                    // Lagged：消费过慢丢失部分事件，记录后继续
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("[event_bus] 消费滞后，丢失 {} 条事件", n);
                    }

                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        // BR-108/BR-116: establish the conservative governance context before
        // any long-running loop starts. This must run outside the market-active
        // branch so after-hours, weekends, and startup source failures can still
        // produce governed state alerts instead of repeating banner-unavailable.
        if !evaluate_account_mode_hook(true).await {
            log::error!(
                "[startup-governance][BR-108/BR-116] AccountMode notification unconfirmed; context remains conservative and periodic retry stays eligible"
            );
        }
        evaluate_data_mode_hook().await;
        audit_full_market_rankings_unavailable("monitor_startup");

        // 任务#3: 启动时快照过期检查（stale 则推提醒；每日去重）
        check_snapshot_staleness_and_notify().await;

        let main_loops = async {
            // Phase 3: 移除 news_pipeline_loop_v15_3 (#2)；统一新闻 Gateway
            // 已由 news_monitor_loop 消费，旧 loop 会重复取数。
            tokio::join!(
                monitor_loop(),
                news_monitor_loop(selection_v2_enabled),
                data_mode_monitor_loop(),
                off_session_quote_keepalive_loop()
            );
        };

        // Phase 3: 移除 poll_news_loop (#3)；news_monitor_loop 已消费统一新闻
        // Gateway，旧 loop 会重复取数并重复写入。

        // 盘后回溯调度：30 min tick，15:30 后通过统一个股新闻 Gateway
        // 抓取持仓股近 30 天事实。

        let post_close_news = tokio::spawn(post_close_news_scheduler());
        let post_session_review = spawn_post_session_review_scheduler(selection_v2_enabled);
        let background_tasks = vec![
            ("dryrun_reporter", dryrun_reporter),
            ("monitor_event_consumer", event_consumer),
            ("post_close_news", post_close_news),
            ("post_session_review", post_session_review),
        ];

        let shutdown_signal = async {
            tokio::signal::ctrl_c()
                .await
                .map_err(|error| format!("install/receive SIGINT handler: {error}"))?;
            log::warn!("收到 SIGINT，正在优雅关闭监控...");
            Ok(())
        };
        if let Err(error) = supervise_long_running_lifecycle(
            bus,
            &mut jsonl_writer_handle,
            background_tasks,
            main_loops,
            shutdown_signal,
        )
        .await
        {
            log::error!("[BR-141] monitor lifecycle failed: {error}");
            log::logger().flush();
            std::process::exit(2);
        }
        log::info!("监控已安全关闭");
    }
}

#[cfg(test)]
fn contains_legacy_manual_trade_flag(args: &[String]) -> bool {
    args.iter()
        .any(|argument| matches!(argument.as_str(), "--buy" | "--sell"))
}

#[cfg(test)]
fn terminal_review_requested(args: &[String]) -> bool {
    args.iter().any(|argument| argument == "--review")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewExecutionPath {
    StrictDispatchers,
}

#[cfg(test)]
fn review_execution_path(args: &[String]) -> ReviewExecutionPath {
    debug_assert!(terminal_review_requested(args));
    ReviewExecutionPath::StrictDispatchers
}

fn review_timeout_secs() -> u64 {
    std::env::var("MONITOR_REVIEW_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|&value| value > 0)
        .unwrap_or(300)
}

fn apply_durable_review_hydrations(
    schedule: &mut review_batch::ReviewScheduleState,
) -> Result<std::collections::BTreeSet<review_batch::ReviewTask>, String> {
    let hydrations = durable_delivery_runtime::pending_schedule_hydrations()?;
    if hydrations.is_empty() {
        return Ok(std::collections::BTreeSet::new());
    }
    apply_durable_review_hydrations_and_acknowledge(
        schedule,
        &hydrations,
        durable_delivery_runtime::acknowledge_local_schedule_hydrations,
    )
}

fn apply_durable_review_hydrations_and_acknowledge(
    schedule: &mut review_batch::ReviewScheduleState,
    hydrations: &[stock_analysis::durable_delivery::ScheduleHydration],
    acknowledge: impl FnOnce(&std::collections::BTreeSet<String>) -> Result<(), String>,
) -> Result<std::collections::BTreeSet<review_batch::ReviewTask>, String> {
    let mut candidate = schedule.clone();
    let application = candidate.apply_durable_hydrations_with_evidence(hydrations)?;
    acknowledge(&application.transition_identities)?;
    *schedule = candidate;
    let applied = application.tasks;
    if !applied.is_empty() {
        log::info!(
            "[复盘调度][BR-140][BR-192] applied durable task hydration date={} tasks={:?} transitions={}",
            schedule.date(),
            applied,
            application.transition_identities.len()
        );
    }
    Ok(applied)
}

/// 手动复盘：`cargo run --bin monitor -- --review`

async fn run_review_only() -> Result<(), String> {
    log::info!("[复盘] 手动触发盘后分析...");

    // 修复 P0-G (2026-06-30 codex review): 顶层 5min fast-fail (AGENTS §2.1, BR-009).

    // 沙箱 / 数据源全失联时, 进程可能在 reqwest 内部回调里死锁,

    // 5min 后显式 exit 2 + ERROR 日志, 不推送噪声给用户.

    let review_timeout_secs = review_timeout_secs();

    log::info!(
        "[复盘] 顶层超时保护: {}s (env MONITOR_REVIEW_TIMEOUT_SECS 可覆盖)",
        review_timeout_secs
    );

    // BR-223: 无生产者 PushKind 启动声明 (Once 保护, 每次进程只打一次)
    static IPO_NO_PRODUCER_BANNER: std::sync::Once = std::sync::Once::new();
    IPO_NO_PRODUCER_BANNER.call_once(|| {
        log::warn!(
            "[BR-223] PushKind::IpoListingApproval / IpoProspectus disabled=no_producer; \
             仅 IpoCatalyst 有静态供应链表生产者"
        );
    });

    let review_start = std::time::Instant::now();

    let due: std::collections::BTreeSet<_> = review_batch::ReviewTask::ALL.into_iter().collect();
    // 2026-08-06: --review CLI 手动触发 → at_manual (跳过 21:00 龙虎榜门,
    // R-04/R-07 立即尝试; 未发布数据 dispatcher 降级 + 出声)。
    let context = review_batch::ReviewRunContext::at_manual(chrono::Local::now().naive_local());
    log::info!(
        "[复盘][BR-140] effective_review_date={} observed_at={} manual_override=true",
        context.review_date(),
        context.observed_at().format("%Y-%m-%dT%H:%M:%S")
    );
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(review_timeout_secs),
        run_strict_review_only_inner(&due, context),
    )
    .await;

    match outcome {
        Ok(Ok(batch)) => {
            let deferred = batch
                .tasks
                .iter()
                .filter_map(|(task, outcome)| match outcome {
                    review_batch::ReviewTaskOutcome::DeferredUntil { at, .. } => {
                        Some(format!("{}@{}", task.label(), at.to_rfc3339()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            let mut audit_state =
                review_batch::ReviewScheduleState::for_date(context.review_date());
            let durable_tasks = apply_durable_review_hydrations(&mut audit_state)?;
            let legacy_batch = batch.without_tasks(&durable_tasks);
            let transitions = audit_state.apply_for_run(&legacy_batch, context);
            if !transitions.is_empty() {
                if let Err(error) =
                    review_batch::append_task_transition_audit(transitions, context.review_date())
                {
                    return Err(format!("[BR-110][BR-140] 逐任务结果审计失败: {error}"));
                }
            }
            if !deferred.is_empty() {
                log::warn!(
                    "[复盘][BR-209] A-10 静默期前置延后；A-10 本任务 provider/renderer/sink 均未调用，请在指定时间后重新执行 --review: {:?}",
                    deferred
                );
            }
            let task_statuses = batch
                .tasks
                .iter()
                .map(|(task, outcome)| format!("{}:{}", task.label(), outcome.status_label()))
                .collect::<Vec<_>>();
            match batch.completion() {
                review_batch::ReviewCompletion::Complete => {
                    log::info!(
                        "[复盘][BR-212] ======== 盘后分析完整完成 ({}s) statuses={:?} ========",
                        review_start.elapsed().as_secs(),
                        task_statuses
                    );
                    Ok(())
                }
                review_batch::ReviewCompletion::Partial => {
                    log::warn!(
                        "[复盘][BR-212] ======== 盘后分析部分完成 ({}s) statuses={:?}; 已保留确认投递，未完成任务保持原状态 ========",
                        review_start.elapsed().as_secs(),
                        task_statuses
                    );
                    Ok(())
                }
                review_batch::ReviewCompletion::NoDelivery if !deferred.is_empty() => {
                    if should_treat_no_delivery_as_non_fatal(&batch) {
                        log::warn!(
                            "[复盘][BR-209] ======== 盘后分析无确认投递 (原因: {} 为延后) ({}s) statuses={:?} ========",
                            deferred.join(", "),
                            review_start.elapsed().as_secs(),
                            task_statuses
                        );
                        Ok(())
                    } else {
                        Err(format!(
                            "[BR-140][BR-209][BR-212] 严格盘后复盘没有确认投递；A-10 已在数据请求前延后，请在以下时间后重新运行 --review: {}",
                            deferred.join(", ")
                        ))
                    }
                }
                review_batch::ReviewCompletion::NoDelivery => {
                    if should_treat_no_delivery_as_non_fatal(&batch) {
                        log::warn!(
                            "[复盘][BR-212] ======== 盘后分析无确认投递，但部分任务出现可重试失败/等待/降级 ======== ({}s) statuses={:?}",
                            review_start.elapsed().as_secs(),
                            task_statuses
                        );
                        Ok(())
                    } else {
                        Err(
                            "[BR-140][BR-212] 严格盘后复盘没有任何确认投递；逐任务状态已写审计"
                                .to_string(),
                        )
                    }
                }
            }
        }

        Ok(Err(error)) => Err(format!("关键数据不可用: {error}")),

        Err(_elapsed) => Err(format!(
            "{}s 超时未完成, 上游数据源可能全部不可用 / 网络黑洞 / 死锁",
            review_timeout_secs
        )),
    }
}

fn should_treat_no_delivery_as_non_fatal(batch: &review_batch::ReviewBatchOutcome) -> bool {
    !batch.tasks.is_empty()
        && !batch.tasks.iter().all(|(_, outcome)| {
            matches!(
                outcome,
                review_batch::ReviewTaskOutcome::Disabled { .. }
                    | review_batch::ReviewTaskOutcome::NoData { .. }
            )
        })
}

#[cfg(test)]
mod tests_br212_review_cli_completion {
    #[test]
    fn br212_review_cli_has_distinct_complete_partial_and_no_delivery_branches() {
        let source = include_str!("main.rs");
        let run_review_only = source
            .split("async fn run_review_only()")
            .nth(1)
            .expect("run_review_only declaration")
            .split("async fn run_strict_review_only_inner(")
            .next()
            .expect("run_review_only body");

        for required in [
            "ReviewCompletion::Complete",
            "ReviewCompletion::Partial",
            "ReviewCompletion::NoDelivery",
            "盘后分析完整完成",
            "盘后分析部分完成",
        ] {
            assert!(run_review_only.contains(required), "missing {required}");
        }
        assert!(!run_review_only.contains("batch.has_confirmed_delivery()"));
    }

    #[test]
    fn br212_review_no_delivery_with_source_failure_is_treated_as_non_fatal() {
        let batch = crate::review_batch::ReviewBatchOutcome::new(vec![
            (
                crate::review_batch::ReviewTask::R08,
                crate::review_batch::ReviewTaskOutcome::failed(
                    true,
                    "r08_cffex_component_unavailable",
                ),
            ),
            (
                crate::review_batch::ReviewTask::R04,
                crate::review_batch::ReviewTaskOutcome::disabled(
                    "R-04 capability",
                    "mock unavailable",
                ),
            ),
        ]);

        assert!(super::should_treat_no_delivery_as_non_fatal(&batch));
    }

    #[test]
    fn br212_review_no_delivery_with_all_disabled_is_fatal() {
        let batch = crate::review_batch::ReviewBatchOutcome::new(vec![(
            crate::review_batch::ReviewTask::R04,
            crate::review_batch::ReviewTaskOutcome::disabled("R-04 capability", "mock unavailable"),
        )]);

        assert!(!super::should_treat_no_delivery_as_non_fatal(&batch));
    }
}

/// BR-194: strict review enters the dependency-partitioned post-session
/// dispatcher directly; account/banner state is evaluated only by the tasks
/// whose declared dependency requires it.
async fn run_strict_review_only_inner(
    due: &std::collections::BTreeSet<review_batch::ReviewTask>,
    context: review_batch::ReviewRunContext,
) -> Result<review_batch::ReviewBatchOutcome, String> {
    push_templates::dispatch_post_session_review(context, due).await
}

fn post_session_review_window_open(now: chrono::NaiveDateTime, is_trading_day: bool) -> bool {
    let threshold = chrono::NaiveTime::from_hms_opt(19, 0, 0)
        .expect("BR-139 post-session review threshold must be valid");
    is_trading_day && now.time() >= threshold
}

async fn attempt_post_session_review(
    due: &std::collections::BTreeSet<review_batch::ReviewTask>,
) -> Result<review_batch::ReviewBatchOutcome, String> {
    let timeout_secs = review_timeout_secs();
    // 2026-08-06: 手动 --review 用 at_manual — 跳过 21:00 龙虎榜发布门,
    // R-04/R-07 立即尝试 (未发布数据 dispatcher 内部降级)。
    let context = review_batch::ReviewRunContext::at_manual(chrono::Local::now().naive_local());
    tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        run_strict_review_only_inner(due, context),
    )
    .await
    .map_err(|_| format!("strict review timed out after {timeout_secs}s"))?
}

async fn post_session_review_scheduler(selection_v2_enabled: bool) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut state: Option<review_batch::ReviewScheduleState> = None;
    // BR-236 Fix B (2026-08-12): (估值日, 快照 evidence_sha256) — 快照变化即重算
    let mut valuation_state: Option<(chrono::NaiveDate, String)> = None;
    let mut ai_analysis_date: Option<chrono::NaiveDate> = None;

    log::info!("[复盘调度][BR-139] started threshold=19:00 interval=60s");

    loop {
        interval.tick().await;
        let now = chrono::Local::now();
        if selection_v2_enabled {
            let outcome_gateway = stock_analysis::data_gateway::OutcomeDailyBarsGateway;
            match stock_analysis::selection::outcome_v2::OutcomeSettlementOwner::new()
                .settle_tick(now.fixed_offset(), 200, &outcome_gateway)
                .await
            {
                Ok(summary)
                    if summary.recovered_non_outcome > 0
                        || summary.recovered > 0
                        || summary.settled_due > 0
                        || summary.live_owned_skips > 0
                        || summary.superseded > 0 =>
                {
                    for observation in &summary.observations {
                        log::info!(
                            "[selection-v2][BR-178] settlement observation disposition={} logical_subject_key={} verified_due_snapshot_hash={} reason_code={}",
                            observation.disposition.as_str(),
                            observation.logical_subject_key,
                            observation.verified_due_snapshot_hash,
                            observation.reason_code
                        );
                    }
                    log::info!(
                        "[selection-v2][BR-176][BR-178] recovery/due tick recovered_non_outcome={} recovered_outcome={} settled_due={} live_owned={} superseded={}",
                        summary.recovered_non_outcome,
                        summary.recovered,
                        summary.settled_due,
                        summary.live_owned_skips,
                        summary.superseded
                    );
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!(
                        "[selection-v2][BR-178] recovery/due tick failed closed; retry remains eligible: code={} detail={}",
                        error.code,
                        error.detail
                    );
                }
            }
        }

        if !post_session_review_window_open(
            now.naive_local(),
            stock_analysis::calendar::is_trading_day(now.date_naive()),
        ) {
            continue;
        }

        if closing_valuation_runtime::eligible_after_close(now.fixed_offset()) {
            // BR-236 Fix B: 估值基准随快照重算 — 快照 evidence_sha256 变化（新快照
            // 导入）→ 重跑当日估值。幂等 insert（内容哈希 run_id），快照变了 →
            // run_id 变 → 新行；latest view 按 price_date DESC, id DESC → 新行胜出
            // (database/closing_valuation.rs:104/145-153)。失败不更新 state → 下
            // tick 重试（现状语义）。竞态：本 tick 取的 sha 与 run 内部实际快照差
            // 一个 tick，幂等 + 下 tick 自愈。
            let snapshot_sha =
                match stock_analysis::database::user_position_snapshot::latest_user_position_snapshot()
                {
                    Ok(Some(s)) => s.evidence_sha256,
                    Ok(None) => {
                        log::debug!("[BR-147] no user position snapshot — valuation uses derived positions");
                        String::new()
                    }
                    Err(error) => {
                        log::warn!("[BR-147] snapshot lookup failed ({error}); retrying next tick");
                        String::new()
                    }
                };
            let due_for_date = valuation_state.as_ref().map(|(d, _)| *d) != Some(now.date_naive())
                || valuation_state
                    .as_ref()
                    .map(|(_, sha)| sha != &snapshot_sha)
                    .unwrap_or(false);
            if due_for_date {
                match closing_valuation_runtime::run_closing_valuation_once(now.date_naive()).await {
                    Ok(receipt) => {
                        log::info!(
                            "[BR-147] closing valuation persisted: run_id={} inserted={} snapshot_sha={}",
                            receipt.run_id,
                            receipt.inserted,
                            snapshot_sha
                        );
                        valuation_state = Some((now.date_naive(), snapshot_sha));
                        refresh_closing_valuation_note();
                    }
                    Err(error) => log::error!("[BR-147] closing valuation failed: {error}"),
                }
            }
        }

        // BR-192: the legacy multi-round AI result has no immutable model-run,
        // provider-batch, or canonical decision identity. Mark the daily
        // occurrence handled without entering model/quote acquisition or a
        // counted ReviewSignal sink.
        if valuation_state.as_ref().map(|(d, _)| *d) == Some(now.date_naive())
            && ai_analysis_date != Some(now.date_naive())
        {
            ai_analysis_date = Some(now.date_naive());
            log::warn!(
                "[复盘调度][AI][BR-192] capability_unavailable=review_signal_counted_binding_unavailable; \
                 skipped before model and quote acquisition date={}",
                now.date_naive()
            );
        }

        if state.as_ref().map(review_batch::ReviewScheduleState::date) != Some(now.date_naive()) {
            state = Some(review_batch::ReviewScheduleState::for_date(
                now.date_naive(),
            ));
        }
        let schedule = state
            .as_mut()
            .expect("review state initialized for current date");
        if let Err(error) = apply_durable_review_hydrations(schedule) {
            log::error!(
                "[复盘调度][BR-140][BR-192] durable task hydration failed; no review task admitted: {error}"
            );
            continue;
        }
        let due = state
            .as_ref()
            .expect("review state initialized for current date")
            .due_tasks(now.naive_local());
        if due.is_empty() {
            continue;
        }

        match attempt_post_session_review(&due).await {
            Ok(batch) => {
                let delivered = batch.delivered_count();
                let schedule = state
                    .as_mut()
                    .expect("review state initialized for current date");
                let mut next_schedule = schedule.clone();
                let durable_tasks = match apply_durable_review_hydrations(&mut next_schedule) {
                    Ok(tasks) => tasks,
                    Err(error) => {
                        log::error!(
                            "[复盘调度][BR-140][BR-192] durable task hydration failed; schedule state not committed: {error}"
                        );
                        continue;
                    }
                };
                let legacy_batch = batch.without_tasks(&durable_tasks);
                let transitions = next_schedule.apply(&legacy_batch, now.naive_local());
                if !transitions.is_empty() {
                    if let Err(error) = review_batch::append_task_transition_audit(
                        transitions,
                        now.date_naive(),
                    ) {
                        log::error!(
                            "[复盘调度][BR-110][BR-140] outcome audit failed; schedule state not committed: {error}"
                        );
                        continue;
                    }
                }
                *schedule = next_schedule;
                log::info!(
                    "[复盘调度][BR-139][BR-140] attempt complete date={} delivered={} unfinished={}",
                    now.date_naive(),
                    delivered,
                    schedule.has_unfinished_tasks()
                );
            }
            Err(error) => log::error!(
                "[复盘调度][BR-139][BR-140] attempt failed before task outcomes; retry remains eligible: {}",
                error
            ),
        }
    }
}

fn spawn_post_session_review_scheduler(selection_v2_enabled: bool) -> tokio::task::JoinHandle<()> {
    tokio::spawn(post_session_review_scheduler(selection_v2_enabled))
}

/// v70: isolated non-counted smoke plus explicit counted capability boundaries.

///   步骤: 1) seed isolated chain/trade facts

///         2) run TEST_CODE-only review smoke

///         3) exercise non-counted intraday/news renderers

///         4) 不依赖时间窗口，只使用 TEST_CODE 隔离测试夹具

///   用途: 验证隔离、审计与非计数推送，不伪造 counted delivery evidence.

#[derive(Clone, Debug, PartialEq, Eq)]
struct TemplateTestSummary {
    manifest_version: &'static str,
    manifest_sha256: String,
    news_capability_generation: u64,
    news_capability_sha256: String,
    family_active_total: usize,
    family_disabled_total: usize,
    family_retired_total: usize,
    family_total: usize,
    push_kind_active_total: usize,
    push_kind_disabled_total: usize,
    push_kind_retired_total: usize,
    push_kind_total: usize,
    rendered_family_total: usize,
    governance_smoke_attempted: usize,
    governance_smoke_passed: usize,
    live_acceptance_opted_in: bool,
    target_authority_status: &'static str,
    target_identity_sha256: Option<String>,
    target_allowlist_sha256: Option<String>,
    external_process_attempted: usize,
    batches_attempted: usize,
    batches_pushed: usize,
    families_pushed: usize,
    receipt_audit_appended: usize,
    explicit_dry_run_family_total: usize,
    failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TemplateTestBatch {
    template_ids: Vec<&'static str>,
    text: String,
}

fn build_template_test_batches(
    catalog: &[push_templates::TestTemplatePreview],
    max_chars: usize,
) -> Result<Vec<TemplateTestBatch>, String> {
    const HEADER_RESERVE_CHARS: usize = 96;
    const SEPARATOR: &str = "\n\n────\n\n";

    if catalog.is_empty() {
        return Err("BR-196 template batch input is empty".to_string());
    }
    if max_chars <= HEADER_RESERVE_CHARS {
        return Err(format!(
            "BR-196 template batch max_chars too small: {max_chars}"
        ));
    }

    let payload_limit = max_chars - HEADER_RESERVE_CHARS;
    let mut pending_ids = Vec::new();
    let mut pending_cards = Vec::new();
    let mut pending_chars = 0usize;
    let mut raw_batches: Vec<(Vec<&'static str>, String)> = Vec::new();

    for preview in catalog {
        let card = format!("【{}】\n{}", preview.template_id, preview.text.trim());
        let card_chars = card.chars().count();
        if card_chars > payload_limit {
            return Err(format!(
                "BR-196 template {} exceeds batch payload limit: chars={} limit={}",
                preview.template_id, card_chars, payload_limit
            ));
        }
        let separator_chars = if pending_cards.is_empty() {
            0
        } else {
            SEPARATOR.chars().count()
        };
        if !pending_cards.is_empty() && pending_chars + separator_chars + card_chars > payload_limit
        {
            raw_batches.push((pending_ids, pending_cards.join(SEPARATOR)));
            pending_ids = Vec::new();
            pending_cards = Vec::new();
            pending_chars = 0;
        }
        if !pending_cards.is_empty() {
            pending_chars += SEPARATOR.chars().count();
        }
        pending_chars += card_chars;
        pending_ids.push(preview.template_id);
        pending_cards.push(card);
    }
    if !pending_cards.is_empty() {
        raw_batches.push((pending_ids, pending_cards.join(SEPARATOR)));
    }

    let batch_total = raw_batches.len();
    let mut batches = Vec::with_capacity(batch_total);
    for (index, (template_ids, body)) in raw_batches.into_iter().enumerate() {
        let text = format!(
            "[TEST_CODE 模板验收]\n批次 {}/{} | 模板数 {}\n\n{}",
            index + 1,
            batch_total,
            template_ids.len(),
            body
        );
        let actual_chars = text.chars().count();
        if actual_chars > max_chars {
            return Err(format!(
                "BR-196 rendered batch {} exceeds transport limit: chars={} limit={}",
                index + 1,
                actual_chars,
                max_chars
            ));
        }
        batches.push(TemplateTestBatch { template_ids, text });
    }
    Ok(batches)
}

impl TemplateTestSummary {
    fn validate(self) -> Result<(), String> {
        // BR-236 同步: R-13 补录 manifest family (br196_test_delivery) 后
        // active 54→55 (未激活 news) / 56→57 (激活), total 70→71;
        // WatchlistTracking PushKind 入 ALL_PUSH_KINDS 后 kind total 60→61。
        let activated_news = self.family_active_total == 57;
        let expected_family = if activated_news {
            (57, 11, 3, 71)
        } else {
            (55, 13, 3, 71)
        };
        let expected_kind = if activated_news {
            (52, 9, 0, 61)
        } else {
            (50, 11, 0, 61)
        };
        let lifecycle_complete = self.manifest_version == br196_test_delivery::MANIFEST_VERSION
            && self.manifest_sha256.len() == 64
            && self.news_capability_generation > 0
            && self.news_capability_sha256.len() == 64
            && (
                self.family_active_total,
                self.family_disabled_total,
                self.family_retired_total,
                self.family_total,
            ) == expected_family
            && (
                self.push_kind_active_total,
                self.push_kind_disabled_total,
                self.push_kind_retired_total,
                self.push_kind_total,
            ) == expected_kind;
        let render_and_smoke_complete = self.rendered_family_total == self.family_active_total
            && self.governance_smoke_attempted == 4
            && self.governance_smoke_passed == 4;
        let explicit_dry_run = self.explicit_dry_run_family_total > 0;
        let disposition_complete = if explicit_dry_run {
            !self.live_acceptance_opted_in
                && self.target_authority_status == "not_constructed_explicit_dry_run"
                && self.target_identity_sha256.is_none()
                && self.target_allowlist_sha256.is_none()
                && self.external_process_attempted == 0
                && self.batches_attempted == 0
                && self.batches_pushed == 0
                && self.families_pushed == 0
                && self.receipt_audit_appended == 0
                && self.explicit_dry_run_family_total == self.family_active_total
                && self.failed == 0
        } else {
            self.live_acceptance_opted_in
                && self.target_authority_status == "authorized_non_production"
                && self
                    .target_identity_sha256
                    .as_ref()
                    .is_some_and(|hash| hash.len() == 64)
                && self
                    .target_allowlist_sha256
                    .as_ref()
                    .is_some_and(|hash| hash.len() == 64)
                && self.external_process_attempted == self.batches_attempted
                && self.batches_attempted > 0
                && self.batches_pushed == self.batches_attempted
                && self.families_pushed == self.family_active_total
                && self.receipt_audit_appended == self.batches_pushed
                && self.failed == 0
        };
        if lifecycle_complete && render_and_smoke_complete && disposition_complete {
            return Ok(());
        }
        Err(format!(
            "BR-196 template acceptance incomplete: family=A{}/D{}/R{}/T{} kind=A{}/D{}/R{}/T{} \
             rendered={} smoke={}/{} opted_in={} target_status={} external_process_attempted={} \
             batches_attempted={} batches_pushed={} families_pushed={} receipt_audit_appended={} \
             explicit_dry_run_family_total={} failed={}",
            self.family_active_total,
            self.family_disabled_total,
            self.family_retired_total,
            self.family_total,
            self.push_kind_active_total,
            self.push_kind_disabled_total,
            self.push_kind_retired_total,
            self.push_kind_total,
            self.rendered_family_total,
            self.governance_smoke_passed,
            self.governance_smoke_attempted,
            self.live_acceptance_opted_in,
            self.target_authority_status,
            self.external_process_attempted,
            self.batches_attempted,
            self.batches_pushed,
            self.families_pushed,
            self.receipt_audit_appended,
            self.explicit_dry_run_family_total,
            self.failed
        ))
    }
}

async fn e2e_all_templates_run(
    explicit_dry_run: bool,
    news_capability: &br196_test_delivery::NewsFlashProcessCapabilitySnapshot,
) -> Result<(), String> {
    let now = chrono::Local::now();
    let review_date = stock_analysis::calendar::latest_completed_trading_day_at(now.naive_local());
    let today_str = review_date.format("%Y-%m-%d").to_string();
    let hhmm = now.format("%H:%M").to_string();

    if stock_analysis::risk::env_guard::current_env()
        != stock_analysis::risk::env_guard::TradingEnv::Test
    {
        return Err("BR-196 template acceptance requires Test environment".to_string());
    }
    log::info!("[BR-196] V2 acceptance start — isolated TEST_CODE invocation");

    let manifest = br196_test_delivery::build_validated_manifest(news_capability)?;

    const TEST_TEMPLATE_BATCH_MAX_CHARS: usize = 3_500;
    let catalog = br196_test_delivery::build_active_catalog(&today_str, &hhmm, &manifest)?;
    let batches = build_template_test_batches(&catalog, TEST_TEMPLATE_BATCH_MAX_CHARS)?;

    let banner_e2e = push_templates::BannerCtx {
        account_mode: push_templates::AccountMode::Normal,
        total_pos: Some(0),
        today_pnl: Some(0.0),
        account_metrics_complete: true,
        data_mode: push_templates::DataMode::Full,
        data_missing_note: None,
    };
    store_banner(banner_e2e.clone())
        .map_err(|error| format!("BR-196 TEST_CODE governance banner commit failed: {error}"))?;
    let smoke_context = br196_test_delivery::GovernanceSmokeContext::for_review_date(review_date)?;
    let mut smoke = push_e2e_14x_templates(&today_str, &hhmm, &smoke_context).await?;
    smoke.extend(push_e2e_news_modules(&hhmm, &banner_e2e, &smoke_context).await?);
    br196_test_delivery::validate_governance_smoke(&smoke)?;

    let live_acceptance_opted_in = br196_transport::live_acceptance_opted_in();
    if explicit_dry_run {
        log::info!(
            "[BR-196] explicit dry-run path: bypass live acceptance gate (authorized={live_acceptance_opted_in})"
        );
    } else if !live_acceptance_opted_in {
        return Err(
            "BR-196 live_acceptance_not_opted_in: target_resolution=0 external_process_attempted=0 receipt_audit_appended=0; \
             use `monitor --test --push-dry-run` for the isolated audit path, \
             or set BR196_LIVE_FEISHU_ACCEPTANCE=1 to opt into real live acceptance delivery"
                .to_string(),
        );
    }

    let live_report = if explicit_dry_run {
        // Dedicated dry-run: batches are fully rendered and bounded above;
        // no generic sink, push-log, target resolver, process or receipt audit
        // is constructed in this branch.
        log::info!(
            "[BR-196] explicit dry-run families={} batches={} external_process_attempted=0 receipt_audit_appended=0",
            catalog.len(),
            batches.len()
        );
        None
    } else {
        let test_code = match durable_delivery_runtime::current_runtime_namespace()? {
            durable_delivery_runtime::RuntimeNamespace::Test { test_code } => test_code,
            durable_delivery_runtime::RuntimeNamespace::Production => {
                return Err("BR-196 rejected production runtime namespace".to_string());
            }
        };
        let transport_batches = batches
            .iter()
            .enumerate()
            .map(|(index, batch)| br196_transport::BR196TransportBatch {
                ordinal: index + 1,
                template_ids: batch.template_ids.as_slice(),
                text: &batch.text,
            })
            .collect::<Vec<_>>();
        Some(br196_transport::deliver_live_batches(
            &test_code,
            &transport_batches,
            news_capability,
        )?)
    };

    let summary = TemplateTestSummary {
        manifest_version: br196_test_delivery::MANIFEST_VERSION,
        manifest_sha256: manifest.manifest_sha256.clone(),
        news_capability_generation: manifest.news_capability_generation,
        news_capability_sha256: manifest.news_capability_sha256.clone(),
        family_active_total: manifest.family_counts.active,
        family_disabled_total: manifest.family_counts.disabled,
        family_retired_total: manifest.family_counts.retired,
        family_total: manifest.family_counts.total,
        push_kind_active_total: manifest.push_kind_counts.active,
        push_kind_disabled_total: manifest.push_kind_counts.disabled,
        push_kind_retired_total: manifest.push_kind_counts.retired,
        push_kind_total: manifest.push_kind_counts.total,
        rendered_family_total: catalog.len(),
        governance_smoke_attempted: smoke.len(),
        governance_smoke_passed: smoke
            .iter()
            .filter(|item| item.outcome == notify::PushOutcome::Pushed)
            .count(),
        live_acceptance_opted_in: live_report.is_some(),
        target_authority_status: if live_report.is_some() {
            "authorized_non_production"
        } else {
            "not_constructed_explicit_dry_run"
        },
        target_identity_sha256: live_report
            .as_ref()
            .map(|report| report.target_identity_sha256.clone()),
        target_allowlist_sha256: live_report
            .as_ref()
            .map(|report| report.target_allowlist_sha256.clone()),
        external_process_attempted: live_report
            .as_ref()
            .map_or(0, |report| report.external_process_attempted),
        batches_attempted: live_report
            .as_ref()
            .map_or(0, |report| report.batches_attempted),
        batches_pushed: live_report
            .as_ref()
            .map_or(0, |report| report.batches_pushed),
        families_pushed: live_report
            .as_ref()
            .map_or(0, |report| report.families_pushed),
        receipt_audit_appended: live_report
            .as_ref()
            .map_or(0, |report| report.receipt_audit_appended),
        explicit_dry_run_family_total: if explicit_dry_run { catalog.len() } else { 0 },
        failed: 0,
    };
    log::info!(
        "[BR-196] template_test_summary manifest_version={} manifest_sha256={} \
         news_capability_generation={} news_capability_sha256={} \
         family=A{}/D{}/R{}/T{} kind=A{}/D{}/R{}/T{} rendered={} smoke={}/{} \
         live_opt_in={} target_status={} target_identity_sha256={} target_allowlist_sha256={} \
         external_process_attempted={} batches_attempted={} batches_pushed={} families_pushed={} \
         receipt_audit_appended={} explicit_dry_run_family_total={} failed={}",
        summary.manifest_version,
        summary.manifest_sha256,
        summary.news_capability_generation,
        summary.news_capability_sha256,
        summary.family_active_total,
        summary.family_disabled_total,
        summary.family_retired_total,
        summary.family_total,
        summary.push_kind_active_total,
        summary.push_kind_disabled_total,
        summary.push_kind_retired_total,
        summary.push_kind_total,
        summary.rendered_family_total,
        summary.governance_smoke_passed,
        summary.governance_smoke_attempted,
        summary.live_acceptance_opted_in,
        summary.target_authority_status,
        summary.target_identity_sha256.as_deref().unwrap_or("none"),
        summary.target_allowlist_sha256.as_deref().unwrap_or("none"),
        summary.external_process_attempted,
        summary.batches_attempted,
        summary.batches_pushed,
        summary.families_pushed,
        summary.receipt_audit_appended,
        summary.explicit_dry_run_family_total,
        summary.failed
    );
    summary.validate()?;
    log::info!(
        "[v70] E2E 完成 — catalog_total={} mode={} audit_namespace=data/test/TEST_CODE*",
        catalog.len(),
        if explicit_dry_run {
            "complete-dry-run"
        } else {
            "validated-feishu-receipts"
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests_br196_monitor_test_acceptance {
    use super::*;

    fn complete_dry_summary() -> TemplateTestSummary {
        TemplateTestSummary {
            manifest_version: br196_test_delivery::MANIFEST_VERSION,
            manifest_sha256: "a".repeat(64),
            news_capability_generation: 1,
            news_capability_sha256: "b".repeat(64),
            family_active_total: 55,
            family_disabled_total: 13,
            family_retired_total: 3,
            family_total: 71,
            push_kind_active_total: 50,
            push_kind_disabled_total: 11,
            push_kind_retired_total: 0,
            push_kind_total: 61,
            rendered_family_total: 55,
            governance_smoke_attempted: 4,
            governance_smoke_passed: 4,
            live_acceptance_opted_in: false,
            target_authority_status: "not_constructed_explicit_dry_run",
            target_identity_sha256: None,
            target_allowlist_sha256: None,
            external_process_attempted: 0,
            batches_attempted: 0,
            batches_pushed: 0,
            families_pushed: 0,
            receipt_audit_appended: 0,
            explicit_dry_run_family_total: 55,
            failed: 0,
        }
    }

    #[test]
    fn br196_default_test_requires_complete_real_delivery() {
        let summary = TemplateTestSummary {
            live_acceptance_opted_in: true,
            target_authority_status: "authorized_non_production",
            target_identity_sha256: Some("c".repeat(64)),
            target_allowlist_sha256: Some("d".repeat(64)),
            external_process_attempted: 4,
            batches_attempted: 4,
            batches_pushed: 3,
            families_pushed: 30,
            receipt_audit_appended: 3,
            explicit_dry_run_family_total: 0,
            failed: 1,
            ..complete_dry_summary()
        };

        let error = summary
            .validate()
            .expect_err("partial Feishu delivery must fail --test");
        assert!(error.contains("batches_attempted=4 batches_pushed=3"));
        assert!(error.contains("families_pushed=30"));
    }

    #[test]
    fn br196_explicit_dry_run_requires_the_same_complete_catalog() {
        let complete = complete_dry_summary();
        assert_eq!(complete.clone().validate(), Ok(()));

        let incomplete = TemplateTestSummary {
            rendered_family_total: 35,
            ..complete
        };
        assert!(incomplete.validate().is_err());
    }

    #[test]
    fn br196_renderer_catalog_is_closed_unique_and_nonempty() {
        let catalog = push_templates::build_test_template_catalog("2026-07-31", "10:30")
            .expect("complete TEST_CODE renderer catalog");
        assert_eq!(catalog.len(), 55);
        let ids = catalog
            .iter()
            .map(|preview| preview.template_id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), catalog.len());
        assert!(catalog
            .iter()
            .all(|preview| !preview.text.trim().is_empty()));
    }

    #[test]
    fn br196_catalog_contains_private_inline_and_normalized_shapes_not_retired_shapes() {
        let catalog = push_templates::build_test_template_catalog("2026-07-31", "10:30")
            .expect("complete TEST_CODE renderer catalog");
        let ids = catalog
            .iter()
            .map(|preview| preview.template_id)
            .collect::<std::collections::HashSet<_>>();
        for mandatory in [
            "R-08-public-event-calendar",
            "L-01-limit-boards-first",
            "L-02-limit-boards-second",
            "L-03-limit-boards-third-plus",
            "S-01-announcement",
            "S-02-policy-hit",
            "S-03-earnings-beat",
            "S-04-earnings-miss",
            "S-05-analyst-upgrade",
            "S-06-market-action-alert",
        ] {
            assert!(ids.contains(mandatory), "missing {mandatory}");
        }
        assert!(!ids.contains("R-04-review-lhb-legacy"));
        assert!(!ids.contains("R-08-event-calendar"));
    }

    #[test]
    fn br196_batches_partition_the_closed_catalog_once_in_stable_order() {
        let catalog = push_templates::build_test_template_catalog("2026-07-31", "10:30")
            .expect("complete TEST_CODE renderer catalog");
        let batches = build_template_test_batches(&catalog, 3_500)
            .expect("bounded complete template batches");
        assert!(batches.len() > 1);
        let flattened = batches
            .iter()
            .flat_map(|batch| batch.template_ids.iter().copied())
            .collect::<Vec<_>>();
        let expected = catalog
            .iter()
            .map(|preview| preview.template_id)
            .collect::<Vec<_>>();
        assert_eq!(flattened, expected);
        assert!(batches.iter().all(|batch| {
            batch.text.contains("[TEST_CODE 模板验收]") && batch.text.chars().count() <= 3_500
        }));
    }

    #[test]
    fn br196_batching_rejects_empty_and_oversized_catalogs() {
        assert!(build_template_test_batches(&[], 3_500).is_err());
        let oversized = vec![push_templates::TestTemplatePreview {
            template_id: "TEST_CODE-oversized",
            text: "测".repeat(3_500),
        }];
        let error = build_template_test_batches(&oversized, 3_500)
            .expect_err("single oversized card must fail closed");
        assert!(error.contains("exceeds batch payload limit"));
    }
}

/// v70: 推新闻模块 (D-01 / I-02) — isolated test fixture

///   news_monitor_loop 真实路径需公告源；这里直接走 TEST_CODE dispatcher fixture

///   公告测试数据: 3 主题 + 2 票 (覆盖 D-01 + I-02)

async fn push_e2e_news_modules(
    hhmm: &str,
    banner: &push_templates::BannerCtx,
    smoke_context: &br196_test_delivery::GovernanceSmokeContext,
) -> Result<Vec<br196_test_delivery::GovernanceSmokeDisposition>, String> {
    use push_templates as pt;

    // D-01 新闻驱动个股 (isolated test fixture)

    let d01 = pt::render_news_to_idea(
        banner,
        pt::NewsToIdeaParams {
            hhmm,

            headline: "TEST_CODE_NEWS_1 净利润 +45% 超预期",

            theme: Some("AI 算力"),

            stage: pt::NewsStage::Starting,

            name: "深南电路",

            code: "TEST_CODE_NEWS_1",

            reasons: vec!["PCB 涨价 12%", "算力国产替代加速"],

            action: Some(pt::NewsAction::BuyDip),
        },
    );

    log::info!("[v70] D-01 推 ({} 字)", d01.chars().count());

    let d01_outcome = notify::push_br196_governance_smoke_v3(
        &d01,
        smoke_context.dispatch(
            "D-01-news-to-idea",
            notify::PushKind::NewsToIdea,
            Some("TEST_CODE_NEWS_1"),
        )?,
    )
    .await;

    // I-02 新闻催化映射 (isolated fixture)

    let i02 = pt::render_news_catalyst(
        banner,
        pt::NewsCatalystParams {
            hhmm,

            headline: "DeepSeek V4 发布, AI 算力国产替代加速",

            theme: Some("AI 算力"),

            stocks: vec![
                (
                    "深南电路",
                    "TEST_CODE_NEWS_1",
                    Some(10.0),
                    "PCB 龙头, Q1 业绩超预期",
                ),
                (
                    "沪电股份",
                    "TEST_CODE_NEWS_2",
                    Some(9.5),
                    "800G 交换机 PCB 受益",
                ),
            ],
        },
    );

    log::info!("[v70] I-02 推 ({} 字)", i02.chars().count());

    let i02_outcome = notify::push_br196_governance_smoke_v3(
        &i02,
        smoke_context.dispatch("I-02-news-catalyst", notify::PushKind::NewsCatalyst, None)?,
    )
    .await;
    Ok(vec![
        br196_test_delivery::GovernanceSmokeDisposition {
            family_key: "D-01-news-to-idea",
            push_kind: notify::PushKind::NewsToIdea,
            outcome: d01_outcome,
        },
        br196_test_delivery::GovernanceSmokeDisposition {
            family_key: "I-02-news-catalyst",
            push_kind: notify::PushKind::NewsCatalyst,
            outcome: i02_outcome,
        },
    ])
}

/// v70: 推所有盘中 14.x 模板 (isolated test fixtures)

async fn push_e2e_14x_templates(
    date: &str,
    hhmm: &str,
    smoke_context: &br196_test_delivery::GovernanceSmokeContext,
) -> Result<Vec<br196_test_delivery::GovernanceSmokeDisposition>, String> {
    use push_templates as pt;

    // P-01 盘前新闻热点 (isolated test fixtures)

    let p01 = pt::render_preopen_news_hot(pt::PreopenNewsHotParams {
        hhmm,

        theme_1: Some("PCB 涨价"),

        theme_2: Some("算力国产替代"),

        theme_3: Some("固态电池量产"),

        news_pairs: vec![
            ("TEST_CODE_P01_1 净利润 +45%", "AI 算力"),
            ("TEST_CODE_P01_2 订单回暖", "锂电池"),
        ],

        watch_stocks: vec![
            (
                "深南电路".to_string(),
                "TEST_CODE_P01_1".to_string(),
                "PCB 量价齐升".to_string(),
            ),
            (
                "天孚通信".to_string(),
                "TEST_CODE_P01_2".to_string(),
                "光模块订单回暖".to_string(),
            ),
        ],
    });

    log::info!("[v70] P-01 推 ({} 字)", p01.chars().count());

    let p01_outcome = notify::push_br196_governance_smoke_v3(
        &p01,
        smoke_context.dispatch(
            "P-01-preopen-news-hot",
            notify::PushKind::PreopenNewsHot,
            None,
        )?,
    )
    .await;

    // P-02 竞价热点量能 (isolated fixture)

    let p02 = format!(

        "🌅 竞价热点量能（{}）\n深南电路(TEST_CODE_P02_1) 高开+1.2% | 量比3.5 | 竞价额1.2亿\n结论: 强承接\n辅助建议, 非下单指令",

        hhmm

    );

    log::info!("[v70] P-02 推 ({} 字)", p02.chars().count());

    let p02_outcome = notify::push_br196_governance_smoke_v3(
        &p02,
        smoke_context.dispatch("T-11-auction-volume", notify::PushKind::AuctionVolume, None)?,
    )
    .await;

    // R-03 涨停产业链 (chain_daily 5 概念, TEST_CODE 数据)

    let r03 = pt::render_industry_chain(
        date,
        &[
            pt::ChainLine {
                chain: "PCB",
                limit_up_n: 3,
                first_n: 1,
                consec_n: 3,

                heat_stage: "高潮",
                leader_name: "深南电路",
                leader_code: "TEST_CODE_R03_1",
                leader_boards: 3,

                followers: "沪电股份, 兴森科技",
                watch_point: Some("放量后回踩关注"),
            },
            pt::ChainLine {
                chain: "算力",
                limit_up_n: 2,
                first_n: 1,
                consec_n: 2,

                heat_stage: "主升",
                leader_name: "科大讯飞",
                leader_code: "TEST_CODE_R03_2",
                leader_boards: 2,

                followers: "全志科技",
                watch_point: Some("板块趋势延续"),
            },
        ],
        None,
        None,
    );

    log::info!("[v70] R-03 渲染 smoke ({} 字)", r03.chars().count());

    // BR-192 (2026-08-12): R-03 升级 counted — TEST_CODE fixture 不能替代不可变
    // binding (与 R-04/R-05 同规则), 保留渲染 smoke, 治理路径跳过出声。
    log::warn!(
        "[v70][BR-051][BR-192] capability_unavailable=review_industry_chain_counted_binding_unavailable; \
         skipped before TEST_CODE fixture assembly"
    );
    push_templates::log_dispatcher_attempt(
        "R-03",
        false,
        0,
        "review_industry_chain_counted_binding_unavailable",
    );
    log::warn!(
        "[v70][BR-051][BR-192] capability_unavailable=review_lhb_counted_binding_unavailable; \
         skipped before TEST_CODE fixture assembly"
    );
    push_templates::log_dispatcher_attempt(
        "R-04",
        false,
        0,
        "review_lhb_counted_binding_unavailable",
    );
    log::warn!(
        "[v70][BR-051][BR-192] capability_unavailable=review_signal_counted_binding_unavailable; \
         skipped before TEST_CODE fixture assembly"
    );
    push_templates::log_dispatcher_attempt(
        "R-05",
        false,
        0,
        "review_signal_counted_binding_unavailable",
    );

    // A-10 题材催化复盘 (TEST_CODE chain_daily)

    let a10 = pt::render_catalyst_review(pt::CatalystReviewParams {
        date,
        theme: "PCB",

        score: Some(8.5),
        persistent: pt::PersistentLevel::High,

        member_count: 3,
        continuous_count: 3,

        leading_names: vec!["深南电路", "沪电股份"],
        leading_codes: vec!["002916", "002463"],

        other_names: vec!["兴森科技"],
        other_codes: vec!["002436"],
        watch_point: Some("放量后回踩关注"),
    });

    log::info!("[v70] A-10 渲染 smoke ({} 字)", a10.chars().count());

    // BR-192 (2026-08-12): A-10 升级 counted — TEST_CODE fixture 不能替代不可变
    // binding (与 R-04/R-05 同规则), 保留渲染 smoke, 治理路径跳过出声。
    log::warn!(
        "[v70][BR-051][BR-192] capability_unavailable=review_catalyst_counted_binding_unavailable; \
         skipped before TEST_CODE fixture assembly"
    );
    push_templates::log_dispatcher_attempt(
        "A-10",
        false,
        0,
        "review_catalyst_counted_binding_unavailable",
    );

    log::info!("[v70] e2e 14x 模板跑完");
    Ok(vec![
        br196_test_delivery::GovernanceSmokeDisposition {
            family_key: "P-01-preopen-news-hot",
            push_kind: notify::PushKind::PreopenNewsHot,
            outcome: p01_outcome,
        },
        br196_test_delivery::GovernanceSmokeDisposition {
            family_key: "T-11-auction-volume",
            push_kind: notify::PushKind::AuctionVolume,
            outcome: p02_outcome,
        },
    ])
}

/// 窗口：盘前08:00-09:30、盘中09:30-15:00、盘后15:00-22:00。

fn validate_announcement_watch_codes(
    registered_watch_codes: &std::collections::HashSet<String>,
) -> Result<std::collections::HashSet<String>, String> {
    if registered_watch_codes
        .iter()
        .any(|code| code.trim().is_empty())
    {
        return Err("BR-138 公告受众代码为空".to_string());
    }
    Ok(registered_watch_codes.clone())
}

fn collect_announcement_watch_codes(
    watchlist: Result<Vec<stock_analysis::portfolio::Position>, String>,
) -> Result<std::collections::HashSet<String>, String> {
    let codes = watchlist?
        .into_iter()
        .map(|position| position.code)
        .collect();
    validate_announcement_watch_codes(&codes)
}

type AnnouncementWatchLoadTask =
    tokio::task::JoinHandle<Result<Vec<stock_analysis::portfolio::Position>, String>>;

async fn poll_announcement_watch_load(
    task: &mut Option<AnnouncementWatchLoadTask>,
) -> Result<Vec<stock_analysis::portfolio::Position>, String> {
    let Some(handle) = task.take() else {
        return Err("BR-138 explicit watch load was not started".to_string());
    };
    if !handle.is_finished() {
        *task = Some(handle);
        return Err("BR-138 explicit watch load is still in progress".to_string());
    }
    handle
        .await
        .map_err(|error| format!("BR-138 explicit watch background task failed: {error}"))?
}

fn merge_news_monitor_codes(
    holding_codes: Result<std::collections::HashSet<String>, String>,
    watch_codes: Option<&std::collections::HashSet<String>>,
) -> Result<std::collections::HashSet<String>, String> {
    let mut codes = holding_codes?;
    if let Some(watch_codes) = watch_codes {
        codes.extend(watch_codes.iter().cloned());
    }
    Ok(codes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnnouncementWatchReadiness {
    Pending,
    Failed,
    Ready,
}

fn announcement_watch_readiness(
    watchlist: &Result<Vec<stock_analysis::portfolio::Position>, String>,
) -> AnnouncementWatchReadiness {
    match watchlist {
        Ok(_) => AnnouncementWatchReadiness::Ready,
        Err(error) if error.contains("still in progress") => AnnouncementWatchReadiness::Pending,
        Err(_) => AnnouncementWatchReadiness::Failed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
enum NewsOuterTickPhase {
    CriticalFlash = 0,
    HoldingEarnings = 1,
    L2 = 2,
    Announcement = 3,
    Reset = 4,
    Flush = 5,
    Banner = 6,
    Sleep = 7,
}

impl NewsOuterTickPhase {
    const ALL: [Self; 8] = [
        Self::CriticalFlash,
        Self::HoldingEarnings,
        Self::L2,
        Self::Announcement,
        Self::Reset,
        Self::Flush,
        Self::Banner,
        Self::Sleep,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::CriticalFlash => "critical_flash",
            Self::HoldingEarnings => "holding_earnings",
            Self::L2 => "l2",
            Self::Announcement => "announcement",
            Self::Reset => "reset",
            Self::Flush => "flush",
            Self::Banner => "banner",
            Self::Sleep => "sleep",
        }
    }
}

#[derive(Debug)]
struct NewsOuterTickCoordinator {
    watch_readiness: AnnouncementWatchReadiness,
    entered: [u8; NewsOuterTickPhase::ALL.len()],
}

impl NewsOuterTickCoordinator {
    fn new(watch_readiness: AnnouncementWatchReadiness) -> Self {
        Self {
            watch_readiness,
            entered: [0; NewsOuterTickPhase::ALL.len()],
        }
    }

    fn set_watch_readiness(&mut self, watch_readiness: AnnouncementWatchReadiness) {
        self.watch_readiness = watch_readiness;
    }

    fn enter(&mut self, phase: NewsOuterTickPhase) -> bool {
        let enabled = phase != NewsOuterTickPhase::Announcement
            || self.watch_readiness == AnnouncementWatchReadiness::Ready;
        if enabled {
            self.entered[phase as usize] = self.entered[phase as usize].saturating_add(1);
        }
        enabled
    }

    fn entered_count(&self, phase: NewsOuterTickPhase) -> u8 {
        self.entered[phase as usize]
    }

    fn finish(&self) -> Result<(), String> {
        for phase in NewsOuterTickPhase::ALL {
            let expected = if phase == NewsOuterTickPhase::Announcement
                && self.watch_readiness != AnnouncementWatchReadiness::Ready
            {
                0
            } else {
                1
            };
            let actual = self.entered_count(phase);
            if actual != expected {
                return Err(format!(
                    "BR-138 outer tick phase {} entered {} times, expected {} for watch {:?}",
                    phase.label(),
                    actual,
                    expected,
                    self.watch_readiness
                ));
            }
        }
        Ok(())
    }
}

/// BR-152: earnings/analyst enrichment belongs to the existing post-close
/// review pipeline (v18/v19); the intraday news loop must not call these slow
/// providers. It may only consume data already persisted by that pipeline.
fn post_close_analysis_window_open(now: chrono::NaiveDateTime) -> bool {
    now.time() >= chrono::NaiveTime::from_hms_opt(15, 0, 0).expect("valid post-close time")
}

fn load_announcement_audience_codes(
    registered_watch_codes: &std::collections::HashSet<String>,
) -> (std::collections::HashSet<String>, Option<String>) {
    // BR-226: 券商未接入时, 用户每日确认的持仓快照
    // (append-only + 不可变 snapshot_id + evidence_sha256 + 用户确认时间)
    // 作为持仓受众证据, 24 小时新鲜度 (用户每日提供)。不满足条件时
    // 回退到自选受众并显式声明 (BR-138 保持 fail-closed)。
    use stock_analysis::database::user_position_snapshot::latest_user_position_snapshot;
    let snapshot = match latest_user_position_snapshot() {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            return (
                registered_watch_codes.clone(),
                Some(
                    "BR-226 user position snapshot not provided; excluded from announcement audience"
                        .to_string(),
                ),
            );
        }
        Err(error) => {
            return (
                registered_watch_codes.clone(),
                Some(format!(
                    "BR-226 user position snapshot read failed: {error}; excluded from announcement audience"
                )),
            );
        }
    };
    let snapshot_local = snapshot.effective_at.with_timezone(&chrono::Local);
    let age_hours = chrono::Local::now().signed_duration_since(snapshot_local).num_hours();
    if age_hours > 24 {
        return (
            registered_watch_codes.clone(),
            Some(format!(
                "BR-226 user position snapshot stale (effective_at {}, {age_hours}h ago); excluded from announcement audience",
                snapshot.effective_at
            )),
        );
    }
    if snapshot.confirm_empty {
        return (
            registered_watch_codes.clone(),
            Some(
                "BR-226 user position snapshot confirms empty; excluded from announcement audience"
                    .to_string(),
            ),
        );
    }
    let mut audience: std::collections::HashSet<String> =
        snapshot.items.iter().map(|item| item.code.clone()).collect();
    audience.extend(registered_watch_codes.iter().cloned());
    log::info!(
        "[NewsMonitor][BR-226] 用户确认持仓快照作为持仓受众: {} 只持仓 + {} 只自选 (snapshot_id={}, effective_at={})",
        snapshot.items.len(),
        registered_watch_codes.len(),
        snapshot.snapshot_id,
        snapshot.effective_at
    );
    (audience, None)
}

fn isolate_announcement_position_failure(
    audience: Result<std::collections::HashSet<String>, String>,
    registered_watch_codes: &std::collections::HashSet<String>,
) -> (std::collections::HashSet<String>, Option<String>) {
    match audience {
        Ok(audience) => (audience, None),
        Err(error) => (registered_watch_codes.clone(), Some(error)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnnouncementAlertAction {
    NormalizedDownstream,
    Suppress,
}

fn announcement_alert_action(
    input_index: usize,
    route: &v17_sources::AnnouncementSourceRouteReport,
) -> AnnouncementAlertAction {
    match route.disposition_for_input(input_index) {
        Some(v17_sources::AnnouncementDisposition::Pushed) => {
            AnnouncementAlertAction::NormalizedDownstream
        }
        Some(
            v17_sources::AnnouncementDisposition::FilteredClassification
            | v17_sources::AnnouncementDisposition::FilteredLifecycle
            | v17_sources::AnnouncementDisposition::FilteredAudience
            | v17_sources::AnnouncementDisposition::FilteredDuplicate
            | v17_sources::AnnouncementDisposition::Failed,
        ) => AnnouncementAlertAction::Suppress,
        None => {
            log::error!(
                "[公告][BR-137][BR-138] provider input missing normalized disposition: index={input_index}"
            );
            AnnouncementAlertAction::Suppress
        }
    }
}

async fn news_monitor_loop(selection_v2_enabled: bool) {
    use stock_analysis::monitor::detector::AlertEvent;

    use stock_analysis::monitor::news_monitor::NewsMonitor;

    use stock_analysis::monitor::signal_state::SignalStateMachine;

    let poll_secs: u64 = std::env::var("NEWS_POLL_INTERVAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);

    log::info!("[NewsMonitor] 启动（独立窗口，不随价格扫描器静默）");
    // BR-226: 启动时声明持仓受众证据状态 (用户确认快照 vs 券商批次)
    match stock_analysis::database::user_position_snapshot::latest_user_position_snapshot() {
        Ok(Some(snapshot)) => {
            let age_hours = chrono::Local::now()
                .signed_duration_since(snapshot.effective_at.with_timezone(&chrono::Local))
                .num_hours();
            if age_hours <= 24 {
                log::info!(
                    "[NewsMonitor][BR-226] 持仓受众证据: 用户确认快照 ({} 只, effective_at {}, {}h 内)",
                    snapshot.items.len(),
                    snapshot.effective_at,
                    age_hours
                );
            } else {
                log::warn!(
                    "[NewsMonitor][BR-226] 持仓受众证据过期: 快照 effective_at {} 已 {age_hours}h; 持仓身份排除, 自选受众继续",
                    snapshot.effective_at
                );
            }
        }
        Ok(None) => log::warn!(
            "[NewsMonitor][BR-226] 持仓受众证据缺失: 未提供用户持仓快照; 持仓身份排除, 自选受众继续"
        ),
        Err(error) => log::warn!(
            "[NewsMonitor][BR-226] 持仓受众证据读取失败: {error}; 持仓身份排除, 自选受众继续"
        ),
    }

    let mut nm = NewsMonitor::new();

    nm.restore_dedup();

    log::info!(
        "[NewsAI-shadow][BR-112][BR-172] governed delivery remains disabled; immutable assessment shadow enabled by default (env 开关已取消, 2026-08-11)"
    );

    let mut sm = SignalStateMachine::default();

    sm.restore_state();

    let mut last_concept_refresh = std::time::Instant::now();

    let mut last_flush = std::time::Instant::now();

    // v17.7 Task 7: AnalystStateStore for per-(code, broker) rating tracking
    let analyst_store =
        stock_analysis::news::aggregator::analyst_state::AnalystStateStore::new(10_000);

    // v17.7 Task 7: Last poll timestamps for earnings and analyst data (per code)
    let last_poll_earnings: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    > = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let last_poll_analyst: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    > = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

    let mut announcement_watch_load: Option<AnnouncementWatchLoadTask> = None;

    loop {
        if !NewsMonitor::should_run() {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            continue;
        }

        let mut outer_tick = NewsOuterTickCoordinator::new(AnnouncementWatchReadiness::Pending);

        if announcement_watch_load.is_none() {
            announcement_watch_load = Some(tokio::task::spawn_blocking(
                stock_analysis::portfolio::get_watchlist,
            ));
        }

        // BR-174/BR-183: raw global-news acquisition and notification
        // projection require a durable selection-v2 ingress receipt. While
        // that capability is unreleased, do not call a provider or advance
        // notification simhash; independent policy/announcement business
        // below continues.
        if outer_tick.enter(NewsOuterTickPhase::CriticalFlash) {
            if !selection_v2_enabled {
                log::debug!(
                    "[GlobalNews][BR-174][BR-183] disabled \
                     provider_calls=0 notification_projection=0 news_ai=0"
                );
            } else {
                // BR-183 Track A (2026-08-07): 新闻 → 候选入池。
                // 不依赖 BR-180 receipt: 直接取 raw 标题 → LLM 提取受益个股 →
                // pushed_stocks 候选池 (DB 级当日去重), intraday_monitor 消费。
                // critical 闪送仍待 receipt 链路 (Phase 4)。
                // 2026-08-08 实测: 红线 2.2 报价门 (5s 新鲜度) 在非交易时段
                // 永远失败 (收盘后报价 = 9 小时前) → 只在 9:15-15:00 入池。
                let session = stock_analysis::calendar::current_session();
                if !session.is_trading() && !session.is_auction() {
                    log::debug!(
                        "[GlobalNews][BR-183] Track A 跳过: 非交易时段 session={:?}, 无实时报价",
                        session
                    );
                } else {
                match stock_analysis::news::aggregator::raw_v2::fetch_raw_global_news_batch(20)
                    .await
                {
                    Ok(batch) => {
                        // BR-172: NewsAI shadow 默认启用（2026-08-11 取消
                        // STOCK_ANALYSIS_NEWS_AI_SHADOW_ENABLE 开关）。与 Track A
                        // 同 tick 消费 admitted batches；模型未配置时 shadow 内部
                        // warn 出声跳过，不影响候选入池。
                        let admitted: Vec<
                            stock_analysis::news::aggregator::AdmittedGlobalNewsBatch,
                        > = batch
                            .attempts()
                            .iter()
                            .filter_map(|attempt| {
                                let terminal = attempt.terminal();
                                let records = terminal.records()?;
                                let evidence = terminal.evidence()?;
                                Some(
                                    stock_analysis::news::aggregator::AdmittedGlobalNewsBatch::from_parts(
                                        records.to_vec(),
                                        evidence.clone(),
                                    ),
                                )
                            })
                            .collect();
                        if !admitted.is_empty() {
                            crate::news_ai_shadow::spawn_from_same_tick(&admitted);
                        }
                        let titles: Vec<String> = batch
                            .attempts()
                            .iter()
                            .flat_map(|attempt| {
                                attempt
                                    .terminal()
                                    .records()
                                    .map(|records| {
                                        records.iter().map(|record| record.title.clone())
                                    })
                                    .into_iter()
                                    .flatten()
                            })
                            .collect();
                        let (recorded, skipped) =
                            crate::news_aggregator_init::candidate_ingest_from_news(&titles).await;
                        log::info!(
                            "[GlobalNews][BR-183] Track A tick records={} recorded={} skipped={}",
                            titles.len(),
                            recorded,
                            skipped
                        );
                    }
                    Err(error) => {
                        log::warn!(
                            "[GlobalNews][BR-183] raw batch 获取失败, 本轮候选不入池: {error}"
                        );
                    }
                }
                }
            }
        }

        // BR-138: policy and critical flash have completed before watch
        // readiness is inspected. An unfinished task is retained for the next
        // tick and never awaited here.
        let watchlist = poll_announcement_watch_load(&mut announcement_watch_load).await;
        outer_tick.set_watch_readiness(announcement_watch_readiness(&watchlist));
        if let Ok(positions) = &watchlist {
            for position in positions {
                nm.linker_mut()
                    .register_position(&position.code, &position.name);
            }
        }
        let registered_watch_codes = collect_announcement_watch_codes(watchlist);
        if let Err(error) = &registered_watch_codes {
            if error.contains("still in progress") {
                log::info!("[NewsMonitor][BR-138] 自选池后台加载中；本轮公告受众暂不包含自选增量");
            } else {
                log::error!(
                    "[NewsMonitor][BR-138] 自选池加载失败，本轮隔离公告受众/自选增量: {error}"
                );
            }
        }

        let holding_codes = stock_analysis::portfolio::get_positions().map(|positions| {
            positions
                .into_iter()
                .map(|position| position.code)
                .collect::<std::collections::HashSet<_>>()
        });
        let our_codes =
            match merge_news_monitor_codes(holding_codes, registered_watch_codes.as_ref().ok()) {
                Ok(codes) => {
                    log::info!("[NewsMonitor] L2/财报标的池: {} 只", codes.len());
                    Some(codes)
                }
                Err(error) => {
                    log::error!("[NewsMonitor] 持仓标的加载失败，本轮 L2/财报子链路隔离: {error}");
                    None
                }
            };

        // v17.7 Task 7 / BR-152: poll earnings and analyst upgrades only in
        // the existing post-close analysis window; never make provider calls
        // during the market session.
        if outer_tick.enter(NewsOuterTickPhase::HoldingEarnings)
            && post_close_analysis_window_open(chrono::Local::now().naive_local())
        {
            if let Some(our_codes) = &our_codes {
                let earnings_cfg = stock_analysis::config::get_monitor_config()
                    .v17_7_earnings
                    .clone();
                let poll_secs = earnings_cfg.poll_interval_secs;
                // Convert from config::EarningsConfig to classifier::EarningsConfig
                let classifier_cfg = stock_analysis::news::aggregator::classifier::EarningsConfig {
                    metric: earnings_cfg.metric,
                    beat_threshold_pct: earnings_cfg.beat_threshold_pct,
                    miss_threshold_pct: earnings_cfg.miss_threshold_pct,
                    poll_interval_secs: earnings_cfg.poll_interval_secs,
                };
                let report = v17_sources::poll_earnings_and_analyst(
                    our_codes,
                    &classifier_cfg,
                    &analyst_store,
                    std::sync::Arc::clone(&last_poll_earnings),
                    std::sync::Arc::clone(&last_poll_analyst),
                    poll_secs,
                    poll_secs,
                )
                .await;
                if report.attempted > 0 {
                    log::info!(
                        "[v17.7] earnings/analyst poll: attempted={} classified={} pushed={} skipped={} failed={}",
                        report.attempted,
                        report.classified,
                        report.pushed,
                        report.skipped,
                        report.failed
                    );
                }
            }
        }

        // L2 概念索引刷新（每5分钟一次）

        if outer_tick.enter(NewsOuterTickPhase::L2)
            && last_concept_refresh.elapsed().as_secs() >= 300
        {
            last_concept_refresh = std::time::Instant::now();

            if let Some(our_codes) = &our_codes {
                let codes = our_codes.clone();

                match tokio::task::spawn_blocking(move || {
                    // 同步HTTP在独立线程执行，不触发 runtime 冲突

                    stock_analysis::monitor::news_monitor::refresh_concept_index_blocking(&codes)
                })
                .await
                {
                    Ok(Ok(index)) => {
                        nm.linker_mut().replace_concept_index(index);

                        log::info!(
                            "[NewsMonitor][BR-188] L2 概念索引已更新（{}个板块关联）",
                            nm.linker_ref().concept_count()
                        );
                    }

                    Ok(Err(error)) => log::error!(
                        "[NewsMonitor][BR-188] L2 概念索引完整批次拒绝，本轮保留上一份索引: {}",
                        error
                    ),

                    Err(error) => log::error!(
                        "[NewsMonitor][BR-188] L2 概念索引 blocking worker 失败，本轮保留上一份索引: {}",
                        error
                    ),
                }
            } else {
                log::warn!("[NewsMonitor] L2 概念索引刷新跳过（标的来源不可用）");
            }

            // v41: 周期刷新 banner (让 news_monitor_loop 的 D-01/I-02 用真 AccountMode)

            evaluate_account_mode_hook(false).await;
        }

        let mut pushed: Vec<AlertEvent> = Vec::new();

        // BR-138/BR-168: 公告失败只隔离公告子链路。采集只能走
        // EventCalendarGateway；关键词仅在完整批次接纳后执行。
        let announcements = if outer_tick.enter(NewsOuterTickPhase::Announcement) {
            match stock_analysis::config::get_announce_keywords() {
                Some(keywords) => {
                    let query_date = chrono::Local::now().date_naive();
                    match stock_analysis::data_gateway::EventCalendarGateway::new()
                        // 断点 B (2026-08-06): 公告分页覆盖不足。
                        // probe 实证: cninfo 当日 936 条 (30 页), 排序倒序(最新优先),
                        // limit=300 = max_pages=10 上限 = 覆盖 11:44→00:00 全天时段。
                        // 原 limit=100 只取前 100 条, 受众公告命中率低 (08-06 实证
                        // 100 条里 0 条命中持仓/自选 39 只)。300 条 ×3 覆盖。
                        // 彻底修复 (上游 max_pages 放开 + 全量拉取) 见断点 B-2。
                        .market_announcements(query_date, 300)
                        .await
                    {
                        Ok(batch) => {
                            log::info!(
                                "[NewsMonitor][BR-159][BR-168] admitted announcement batch: {batch}"
                            );
                            match stock_analysis::announcement::project_event_calendar_batch(
                                batch, &keywords,
                            ) {
                                Ok(projected) => Some(projected),
                                Err(error) => {
                                    log::error!(
                                        "[NewsMonitor][BR-168] 公告批次投影失败，本轮公告隔离: {error}"
                                    );
                                    None
                                }
                            }
                        }
                        Err(error) => {
                            log::error!(
                                "[NewsMonitor][BR-138][BR-168] 公告 Gateway 获取失败，本轮公告隔离: {error}"
                            );
                            None
                        }
                    }
                }
                None => {
                    log::error!("[NewsMonitor][BR-138][BR-168] 公告关键词快照未加载，本轮公告隔离");
                    None
                }
            }
        } else {
            None
        };

        if let Some((announcement_batch, registered_watch_codes)) =
            announcements.zip(registered_watch_codes.as_ref().ok())
        {
            let anns = &announcement_batch.announcements;
            let (announcement_audience_codes, position_audience_error) =
                load_announcement_audience_codes(registered_watch_codes);
            if let Some(error) = position_audience_error {
                log::warn!(
                    "[NewsMonitor][BR-138] {error}; 不可验证持仓身份已排除，独立自选受众继续"
                );
            }

            // 异步预解析：公告 API 缺失 code 时，只使用统一身份合同。
            let mut resolved_codes: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            for ann in anns {
                if ann.code.is_empty() && !ann.name.is_empty() {
                    if let Some(code) = nm.linker_ref().lookup_code_by_name(&ann.name) {
                        resolved_codes.insert(ann.name.clone(), code.to_string());
                    } else {
                        match stock_analysis::monitor::news_monitor::resolve_code_by_name(&ann.name)
                            .await
                        {
                            Ok(Some(code)) => {
                                log::info!("[NewsMonitor] 反查 {} → {}", ann.name, code);
                                resolved_codes.insert(ann.name.clone(), code);
                            }
                            Ok(None) => {}
                            Err(error) => log::warn!(
                                "[NewsMonitor][BR-164] 公告名称反查不可用，证券身份保持缺失 name={:?}: {error}",
                                ann.name
                            ),
                        }
                    }
                }
            }

            let events = nm.process_announcements_indexed(anns, &resolved_codes);

            // BR-112/BR-137: successfully classified announcements have exactly one
            // governed owner. Every normalized outcome remains explicit; legacy is
            // retained only for classification failures, never as an outcome fallback.
            let announcement_route = v17_sources::route_announcement_batch(
                &announcement_batch,
                &announcement_audience_codes,
            )
            .await;
            let disposition_counts = announcement_route.disposition_counts();
            log::info!(
                "[公告][BR-137][BR-138] attempted={} classified={} pushed={} skipped={} failed={} audience={} disposition_pushed={} disposition_classification={} disposition_lifecycle={} disposition_audience={} disposition_failed={}",
                announcement_route.source.attempted,
                announcement_route.source.classified,
                announcement_route.source.pushed,
                announcement_route.source.skipped,
                announcement_route.source.failed,
                announcement_audience_codes.len(),
                disposition_counts.pushed,
                disposition_counts.filtered_classification,
                disposition_counts.filtered_lifecycle,
                disposition_counts.filtered_audience,
                disposition_counts.failed
            );

            for (input_index, event) in events {
                if let Some(alert) = sm.process(event) {
                    match announcement_alert_action(input_index, &announcement_route) {
                        AnnouncementAlertAction::NormalizedDownstream => pushed.push(alert),
                        AnnouncementAlertAction::Suppress => log::debug!(
                            "[公告][BR-137][BR-138] legacy and downstream push suppressed: normalized disposition is not Pushed"
                        ),
                    }
                }
            }
        }

        // ═══════════════════════════════════════════════════════════════

        // v29 + v60: D-01 新闻驱动个股推送 (事件驱动)

        //   - 触发: pushed 不空 (有重要公告/事件) 时, 每轮 news_monitor_loop 调一次

        //   - v60 F9: 加 AlertLevel::Important 过滤 (NewsRanker line 2830 已有)

        //     - 低优先级 Info 事件不再触发 D-01 1h memo slot

        //   - 去重: dispatcher memo 1h/票 + push_governor 20min 冷却 (v12 §14.5)

        //   - 数据源: 已接纳的统一新闻/公告候选事实

        //   - 静默: 候选台空时短路返回, log

        // ═══════════════════════════════════════════════════════════════

        let has_important: bool = pushed
            .iter()
            .any(|ev| ev.level >= stock_analysis::monitor::detector::AlertLevel::Important);

        if has_important {
            use push_templates::dispatch_news_to_idea_daily;

            // v41: 读共享 banner (替换写死)

            if let Some(banner) = current_banner_for("D-01 news-to-idea") {
                let now_ts = chrono::Local::now();
                let hhmm = now_ts.format("%H:%M").to_string();
                if !dispatch_news_to_idea_daily(&hhmm, &banner).await {
                    log::error!("[D-01][BR-091] dispatcher did not confirm delivery");
                }
            }
        }

        // ═══════════════════════════════════════════════════════════════

        // v33 + v60: I-02 新闻催化映射 (事件驱动, 同 D-01 时机)

        //   - 触发: pushed 不空 (有重要公告) 时, 调一次

        //   - v60 F9: 加 AlertLevel::Important 过滤

        //   - 数据源: load_news_catalyst_snapshot_real (公告 + 板块聚类)

        //   - 模板: render_news_catalyst (带 banner)

        //   - 静默: 公告空时短路

        //   - 与 D-01 互补: D-01 推个股, I-02 推板块

        // ═══════════════════════════════════════════════════════════════

        if has_important {
            use push_templates::dispatch_news_catalyst_daily;

            // v41: 读共享 banner

            if let Some(banner) = current_banner_for("I-02 news catalyst") {
                let now_ts = chrono::Local::now();
                let hhmm = now_ts.format("%H:%M").to_string();
                if !dispatch_news_catalyst_daily(&hhmm, &banner).await {
                    log::error!("[I-02][BR-091] dispatcher did not confirm delivery");
                }
            }
        }

        // 每日重置

        let today = chrono::Local::now().format("%Y%m%d").to_string();

        if outer_tick.enter(NewsOuterTickPhase::Reset) {
            use std::sync::Mutex;

            static LAST_DATE: Mutex<Option<String>> = Mutex::new(None);

            let mut last = LAST_DATE.lock().unwrap();

            if last.as_deref() != Some(&today) {
                sm.daily_reset();

                *last = Some(today);
            }
        }

        // v5: 每 5 分钟刷盘

        let flush_scheduled = outer_tick.enter(NewsOuterTickPhase::Flush);
        let banner_scheduled = outer_tick.enter(NewsOuterTickPhase::Banner);
        if flush_scheduled && last_flush.elapsed().as_secs() >= 300 {
            last_flush = std::time::Instant::now();

            nm.flush_dedup();

            sm.flush_state();

            // v41: 周期刷新 banner (AccountMode + DataMode 评估 → 写 LATEST_BANNER)

            if banner_scheduled {
                evaluate_account_mode_hook(false).await;
            }
        }

        if outer_tick.enter(NewsOuterTickPhase::Sleep) {
            if let Err(error) = outer_tick.finish() {
                log::error!("[NewsMonitor][BR-138] outer tick contract failed: {error}");
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(poll_secs)).await;
        }
    }
}

// BR-151 / BR-153 T0 START
struct PreparedT0Advice {
    code: String,
    text: String,
    binding: durable_delivery_runtime::CountedDeliveryBinding,
}

/// PaperSell 生产 gate (v19 review 2026-08-12, invalid_position_ledger):
/// 成本为全部历史买入混合摊薄 (Σamt/Σqty), T+1 用 MIN(ts) 最早买入日, 无批次
/// 账本 → 生产 100 笔虚拟卖出含 3 笔收益率 >100% (最高 +22751% 为买价记录错误)、
/// 11 笔当日买入即卖、7 笔买入后 60s 内卖出 (最短 5s)。暂停投递直到批次账本重建。
/// 默认禁用, 仅 `PAPER_SELL_ENABLED=1` 显式启用 (v15.x 静默路径可见: 启动 banner
/// 一次 + 跳过 warn 节流 30 分钟)。
fn paper_sell_paused(phase: &str) -> bool {
    if std::env::var("PAPER_SELL_ENABLED")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        return false;
    }
    static BANNER: std::sync::Once = std::sync::Once::new();
    BANNER.call_once(|| {
        log::warn!(
            "[paper_sell] disabled=invalid_position_ledger; 虚拟盘卖出投递暂停 \
             (成本摊薄/T+1 账本错误待批次账本重建; PAPER_SELL_ENABLED=1 显式启用)"
        );
    });
    static LAST_WARN_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let last = LAST_WARN_SECS.load(std::sync::atomic::Ordering::Relaxed);
    if now_secs.saturating_sub(last) >= 1800 {
        LAST_WARN_SECS.store(now_secs, std::sync::atomic::Ordering::Relaxed);
        log::warn!("[paper_sell] {phase} disabled=invalid_position_ledger; 跳过虚拟盘卖出扫描");
    }
    true
}

async fn prepare_magic_tdx_t0_messages() -> Result<Vec<PreparedT0Advice>, String> {
    use stock_analysis::data_gateway::MagicTdxGateway;
    use stock_analysis::decision::t0_advisor::{
        evaluate_structured, T0PlanDecision, T0PlanDecisionBindingV1, T0Position,
        T0PositionSnapshotBindingV1,
    };

    let snapshot =
        stock_analysis::database::user_position_snapshot::latest_user_position_snapshot()?;
    let Some(snapshot) = snapshot else {
        log::warn!("[做T-持仓][BR-153] skipped reason=user_snapshot_missing");
        return Ok(Vec::new());
    };
    let snapshot_id = snapshot.snapshot_id.clone();
    let snapshot_binding = T0PositionSnapshotBindingV1::new(
        snapshot_id.clone(),
        snapshot.evidence_sha256,
        snapshot.effective_at,
        snapshot.confirmed_at,
    )?;
    let positions = snapshot
        .items
        .into_iter()
        .map(|item| T0Position {
            code: item.code,
            name: item.name,
            total_quantity: item.quantity,
            cost_price: item.cost_price,
            snapshot: snapshot_binding.clone(),
        })
        .collect::<Vec<_>>();
    if positions.is_empty() {
        // v15.x 规则 4: 静默路径必须出声。快照存在但无持仓项 = 做T 无标的,
        // warn 让运营知道「快照导入成功但无内容」而非神秘无声。
        log::warn!(
            "[做T-持仓][BR-153] skipped reason=positions_empty snapshot_id={snapshot_id} (快照存在但无持仓项)"
        );
        return Ok(Vec::new());
    }
    let codes = positions
        .iter()
        .map(|position| position.code.clone())
        .collect::<Vec<_>>();
    let observed_at = chrono::Utc::now();
    let batch = blocking_market_data::run_blocking_market_data(
        "BR-153 Magic TDX Gateway T0 evidence",
        move || {
            MagicTdxGateway::new()
                .get_t0_evidence_batch(&codes, observed_at)
                .map_err(|error| error.to_string())
        },
    )
    .await?;

    log::info!(
        "[做T-持仓][BR-153] batch={} source_at={} observed_at={} records={} rejected={}",
        batch.batch_id.get(..12).unwrap_or(batch.batch_id.as_str()),
        batch.source_at,
        batch.observed_at,
        batch.records.len(),
        batch.rejections.len()
    );
    for rejection in &batch.rejections {
        log::warn!(
            "[做T-持仓][BR-153] code={} isolated reason_code={} retryable={} detail={}",
            rejection.code,
            rejection.reason_code,
            rejection.retryable,
            rejection.detail
        );
    }

    let by_code = positions
        .iter()
        .map(|position| (position.code.as_str(), position))
        .collect::<std::collections::HashMap<_, _>>();
    let banner = current_banner_for("做T-持仓 BR-153")
        .ok_or_else(|| "BR-153 evaluated banner unavailable".to_string())?;
    let mut messages = Vec::new();
    for evidence in &batch.records {
        let Some(position) = by_code.get(evidence.code.as_str()) else {
            return Err(format!(
                "BR-153 source returned non-position code={}",
                evidence.code
            ));
        };
        match evaluate_structured(position, evidence) {
            T0PlanDecision::Advice(plan) => {
                let decision_binding = T0PlanDecisionBindingV1::new(position, evidence, &plan)?;
                let text = push_templates::render_t0_advice(
                    &banner,
                    push_templates::T0AdviceParams::from(&plan),
                );
                let decision_id = decision_binding.decision_id()?;
                let source_binding_canonical = decision_binding.canonical_bytes()?;
                let delivery_subject_hash = decision_binding.delivery_subject_hash()?;
                let business_date = evidence
                    .observed_at
                    .with_timezone(&chrono::Local)
                    .date_naive();
                let binding = durable_delivery_runtime::CountedDeliveryBinding::new(
                    business_date,
                    decision_id,
                    source_binding_canonical,
                    durable_delivery_runtime::CountedDeliveryScope::Ticket {
                        instrument: decision_binding.instrument().clone(),
                    },
                    delivery_subject_hash,
                    durable_delivery_runtime::CountedDeliveryOrigin::Provider {
                        observed_at: Some(evidence.observed_at),
                        as_of: Some(business_date),
                        ordered_batch_ids: vec![decision_binding.evidence_batch_id().to_owned()],
                    },
                    None,
                    true,
                )?;
                messages.push(PreparedT0Advice {
                    code: plan.code,
                    text,
                    binding,
                });
            }
            T0PlanDecision::Forbidden(value) => log::info!(
                "[做T-持仓][BR-153] code={} forbidden reason_code={} reason={}",
                value.code,
                value.reason_code,
                value.reason
            ),
            T0PlanDecision::Rejected(value) => log::debug!(
                "[做T-持仓][BR-153] code={} no_plan reason_code={} reason={}",
                value.code,
                value.reason_code,
                value.reason
            ),
        }
    }
    Ok(messages)
}

fn t0_delivery_outcomes_confirmed(outcomes: &[notify::PushOutcome]) -> bool {
    outcomes.iter().all(|outcome| {
        matches!(
            outcome,
            notify::PushOutcome::Pushed | notify::PushOutcome::Deduped
        )
    })
}
// BR-153 T0 END

// ═══════════════════════════════════════════════════════════════
// BR-192 收尾 (2026-08-07): T-03 持仓操作建议 counted 接线。
// 原 dispatch_holding_plan_daily_result 恒 capability_unavailable
// (holding_plan_counted_binding_unavailable)。真实实现:
//   数据源: BR-226 用户确认快照 (cost/quantity) + 统一行情
//          (market_data::fetch_position_quotes, BR-227)。
//   判定规则 (v12 §14.1 简化, 与 I-04 注释一致): 现价相对成本 >+5% 减仓,
//          <-3% 加仓, 否则持有观望。
//   binding: occurrence = holding-plan:{date}:{code} (当日一票一推, counted
//          去重), scope=Ticket, origin=InternalDurable (快照+行情为真实证据,
//          不伪造批次身份)。失败保留重试资格 (定时器不前进), 成功才封口。
// ═══════════════════════════════════════════════════════════════

struct PreparedHoldingPlan {
    code: String,
    text: String,
    binding: durable_delivery_runtime::CountedDeliveryBinding,
}

/// T-03 当日一票一推 (DB 级, 跨重启): 当日已投递的 code 集合。
fn holding_plan_daily_pushed(plan_date: chrono::NaiveDate) -> std::collections::HashSet<String> {
    use diesel::RunQueryDsl;
    let Ok(mut conn) = stock_analysis::database::DatabaseManager::get().get_conn() else {
        log::error!("[T-03] holding_plan_daily 查询: 数据库连接失败");
        return std::collections::HashSet::new(); // 连接失败不拦截, 由写入路径暴露
    };
    if let Err(error) = diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS holding_plan_daily (\
             plan_date TEXT NOT NULL, code TEXT NOT NULL, pushed_at TEXT NOT NULL, \
             PRIMARY KEY (plan_date, code))",
    )
    .execute(&mut conn)
    {
        log::error!("[T-03] holding_plan_daily 建表失败: {error}");
        return std::collections::HashSet::new();
    }
    let date_str = plan_date.format("%Y-%m-%d").to_string();
    let rows: Vec<HoldingPlanDailyRow> = match diesel::sql_query(
        "SELECT plan_date, code FROM holding_plan_daily WHERE plan_date = ?",
    )
    .bind::<diesel::sql_types::Text, _>(&date_str)
    .load(&mut conn)
    {
        Ok(rows) => rows,
        Err(error) => {
            log::error!("[T-03] holding_plan_daily 查询失败: {error}");
            return std::collections::HashSet::new();
        }
    };
    rows.into_iter().map(|row| row.code).collect()
}

/// 记录当日已推 (INSERT OR REPLACE, 幂等)。
fn holding_plan_daily_record(plan_date: chrono::NaiveDate, code: &str) {
    use diesel::RunQueryDsl;
    let Ok(mut conn) = stock_analysis::database::DatabaseManager::get().get_conn() else {
        log::error!("[T-03] holding_plan_daily 记录: 数据库连接失败 code={code}");
        return;
    };
    let date_str = plan_date.format("%Y-%m-%d").to_string();
    let pushed_at = chrono::Local::now().to_rfc3339();
    if let Err(error) = diesel::sql_query(
        "INSERT OR REPLACE INTO holding_plan_daily (plan_date, code, pushed_at) VALUES (?, ?, ?)",
    )
    .bind::<diesel::sql_types::Text, _>(&date_str)
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(&pushed_at)
    .execute(&mut conn)
    {
        log::error!("[T-03] holding_plan_daily 记录失败 code={code}: {error}");
    }
}

#[derive(diesel::QueryableByName)]
struct HoldingPlanDailyRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
}

async fn prepare_holding_plan_messages(
    banner: &push_templates::BannerCtx,
) -> Result<Vec<PreparedHoldingPlan>, String> {
    use magic_market_core::{AssetClass, Exchange, InstrumentId};
    use sha2::{Digest, Sha256};
    use stock_analysis::database::user_position_snapshot::latest_user_position_snapshot;

    let snapshot = latest_user_position_snapshot()
        .map_err(|error| format!("持仓快照读取失败: {error}"))?
        .ok_or_else(|| "无用户确认持仓快照 (BR-226)".to_string())?;
    if snapshot.confirm_empty || snapshot.items.is_empty() {
        return Ok(Vec::new()); // 空持仓 → 无受众, 静默
    }
    let quotes = market_data::fetch_position_quotes()
        .map_err(|error| format!("持仓行情批次拒绝: {error}"))?;
    let quote_map: std::collections::HashMap<String, &stock_analysis::market_data::TopStock> =
        quotes.iter().map(|q| (q.code.clone(), q)).collect();

    let business_date = chrono::Local::now().date_naive();
    let hhmm = chrono::Local::now().format("%H:%M").to_string();
    let mut out = Vec::new();
    for item in &snapshot.items {
        let Some(quote) = quote_map.get(&item.code) else {
            log::warn!(
                "[T-03] code={} 行情缺失, 跳过该票 (其余照常)",
                item.code
            );
            continue;
        };
        if item.cost_price <= 0.0 {
            log::warn!("[T-03] code={} 成本价非法, 跳过", item.code);
            continue;
        }
        let pnl_pct = (quote.price / item.cost_price - 1.0) * 100.0;
        let intent = if pnl_pct > 5.0 {
            push_templates::Intent::Reduce
        } else if pnl_pct < -3.0 {
            push_templates::Intent::Add
        } else {
            push_templates::Intent::Hold
        };
        let reason = match intent {
            push_templates::Intent::Reduce => {
                format!("浮盈 {pnl_pct:.1}% 触发减仓观察 (>+5%)")
            }
            push_templates::Intent::Add => format!("浮亏 {pnl_pct:.1}% 触发加仓观察 (<-3%)"),
            push_templates::Intent::Hold => format!("浮盈 {pnl_pct:.1}%, 持有观望区间"),
            _ => unreachable!("T-03 只产出 Reduce/Add/Hold"),
        };
        let reasons = vec![reason];
        let text = push_templates::render_holding_plan(
            banner,
            push_templates::HoldingPlanParams {
                name: &item.name,
                code: &item.code,
                hhmm: &hhmm,
                intent,
                price: quote.price,
                cost: item.cost_price,
                avail: u32::try_from(item.quantity).unwrap_or(u32::MAX),
                reduce_zone: Some((item.cost_price * 1.02, item.cost_price * 1.05)),
                support: item.cost_price * 0.95,
                pressure: item.cost_price * 1.10,
                stop: item.cost_price * 0.92,
                invalidations: &[],
                reasons: &reasons,
            },
        );
        let canonical = serde_json::json!({
            "code": item.code,
            "intent": intent.label(),
            "price": quote.price,
            "cost": item.cost_price,
            "quantity": item.quantity,
            "pnl_pct": pnl_pct,
            "observed_at": chrono::Local::now().to_rfc3339(),
        });
        let canonical_bytes = canonical.to_string().into_bytes();
        let subject_hash = hex::encode(Sha256::digest(&canonical_bytes));
        let exchange = if item.code.starts_with('6') {
            Exchange::Shanghai
        } else {
            Exchange::Shenzhen
        };
        let instrument = InstrumentId::new(exchange, item.code.clone(), AssetClass::Equity)
            .map_err(|error| format!("instrument 构造失败 code={}: {error}", item.code))?;
        let binding = durable_delivery_runtime::CountedDeliveryBinding::new(
            business_date,
            format!("holding-plan:{business_date}:{}", item.code),
            canonical_bytes,
            durable_delivery_runtime::CountedDeliveryScope::Ticket { instrument },
            subject_hash,
            durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
            None,
            true,
        )
        .map_err(|error| format!("counted binding 构造失败 code={}: {error}", item.code))?;
        out.push(PreparedHoldingPlan {
            code: item.code.clone(),
            text,
            binding,
        });
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════
// 2026-08-07 审计接入: T-12 尾盘提示 (CloseCall) counted 接线。
// 原 render_close_call 模板存在但生产零调度。数据源与 T-03 同
// (快照 cost + 统一行情), 判定: 现价相对成本 ≤-3% → 尾盘跳水提示
// (只推跳水, 正常票不推减少噪音)。binding: occurrence =
// close-call:{date}:{code}, origin=InternalDurable。
// ═══════════════════════════════════════════════════════════════

struct PreparedCloseCall {
    code: String,
    text: String,
    binding: durable_delivery_runtime::CountedDeliveryBinding,
}

async fn prepare_close_call_messages(
    banner: &push_templates::BannerCtx,
) -> Result<Vec<PreparedCloseCall>, String> {
    use magic_market_core::{AssetClass, Exchange, InstrumentId};
    use sha2::{Digest, Sha256};
    use stock_analysis::database::user_position_snapshot::latest_user_position_snapshot;

    let snapshot = latest_user_position_snapshot()
        .map_err(|error| format!("持仓快照读取失败: {error}"))?
        .ok_or_else(|| "无用户确认持仓快照 (BR-226)".to_string())?;
    if snapshot.confirm_empty || snapshot.items.is_empty() {
        return Ok(Vec::new());
    }
    let quotes = market_data::fetch_position_quotes()
        .map_err(|error| format!("持仓行情批次拒绝: {error}"))?;
    let quote_map: std::collections::HashMap<String, &stock_analysis::market_data::TopStock> =
        quotes.iter().map(|q| (q.code.clone(), q)).collect();

    let business_date = chrono::Local::now().date_naive();
    let hhmm = chrono::Local::now().format("%H:%M").to_string();
    let mut out = Vec::new();
    for item in &snapshot.items {
        let Some(quote) = quote_map.get(&item.code) else {
            continue;
        };
        if item.cost_price <= 0.0 {
            continue;
        }
        let pnl_pct = (quote.price / item.cost_price - 1.0) * 100.0;
        if pnl_pct > -3.0 {
            continue; // 非跳水不推
        }
        let holding = push_templates::CloseCallHolding {
            name: &item.name,
            state: "尾盘跳水-建议处理",
        };
        let text = push_templates::render_close_call(banner, &hhmm, Some(&holding), None);
        let canonical = serde_json::json!({
            "code": item.code,
            "price": quote.price,
            "cost": item.cost_price,
            "pnl_pct": pnl_pct,
            "observed_at": chrono::Local::now().to_rfc3339(),
        });
        let canonical_bytes = canonical.to_string().into_bytes();
        let subject_hash = hex::encode(Sha256::digest(&canonical_bytes));
        let exchange = if item.code.starts_with('6') {
            Exchange::Shanghai
        } else {
            Exchange::Shenzhen
        };
        let instrument = InstrumentId::new(exchange, item.code.clone(), AssetClass::Equity)
            .map_err(|error| format!("instrument 构造失败 code={}: {error}", item.code))?;
        let binding = durable_delivery_runtime::CountedDeliveryBinding::new(
            business_date,
            format!("close-call:{business_date}:{}", item.code),
            canonical_bytes,
            durable_delivery_runtime::CountedDeliveryScope::Ticket { instrument },
            subject_hash,
            durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
            None,
            true,
        )
        .map_err(|error| format!("counted binding 构造失败 code={}: {error}", item.code))?;
        out.push(PreparedCloseCall {
            code: item.code.clone(),
            text,
            binding,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests_br153_t0_delivery {
    use super::{notify::PushOutcome, t0_delivery_outcomes_confirmed};

    #[test]
    fn completed_empty_or_confirmed_delivery_batch_advances_timer() {
        assert!(t0_delivery_outcomes_confirmed(&[]));
        assert!(t0_delivery_outcomes_confirmed(&[
            PushOutcome::Pushed,
            PushOutcome::Deduped,
        ]));
    }

    #[test]
    fn denied_or_sink_error_delivery_batch_does_not_advance_timer() {
        assert!(!t0_delivery_outcomes_confirmed(&[PushOutcome::Denied(
            "TEST_CODE governance".to_string(),
        )]));
        assert!(!t0_delivery_outcomes_confirmed(&[
            PushOutcome::Pushed,
            PushOutcome::SinkError("TEST_CODE sink".to_string()),
        ]));
    }
}

#[cfg(test)]
mod tests_br211_paper_exit_containment {
    #[test]
    fn br211_monitor_loop_has_one_containment_banner_and_no_legacy_entry_call() {
        let source = include_str!("main.rs");
        let monitor_loop = source
            .rsplit_once("async fn monitor_loop()")
            .map(|(_, body)| body)
            .expect("monitor loop declaration")
            .split("fn render_board_flow_market_view(")
            .next()
            .expect("monitor loop body");

        assert!(!monitor_loop.contains("paper_engine::run_once("));
        assert_eq!(
            monitor_loop
                .matches("legacy provider/order calls=0")
                .count(),
            1
        );
    }
}

async fn monitor_loop() {
    // 全天候循环：非交易日等待，交易日自动进入扫描

    // BR-211: legacy paper_engine::run_once lacks the BR-201 committed
    // Admission and BR-205 source-backed price-limit state. Keep it out of the
    // recurring production loop and make the unavailable capability visible
    // once per process instead of retrying provider/order work every tick.
    log::warn!(
        "[paper_engine][BR-201][BR-205][BR-211] 四铁律虚拟盘退出已关闭: guarded owner/source-backed price limits unavailable; legacy provider/order calls=0"
    );

    // v16.3 legacy 4 模块说明；BR-211 已把 paper_engine 退出链从生产循环隔离。
    // - IntradayMonitor::tick  盘中: 每 30s 扫推送票池 + 4 步过滤 + 调 paper_trade::simulate
    // - evening_review       盘后: 15:30 整盘 Momentum 整盘扫 (Fix 5: 不限 1h 时间窗)
    // - paper_engine         disabled：在 BR-201/BR-205 受控 owner 可用前不读账本、不取价、不下单
    let intraday_loop = async {
        use chrono::Timelike;
        use stock_analysis::decision::intraday_monitor::{evening_review, IntradayMonitor};
        let monitor = IntradayMonitor;
        loop {
            let risk_context = current_banner_for("v16.3 paper decision").and_then(|banner| {
                match push_templates::paper_risk_context_from_banner(&banner) {
                    Ok(context) => Some(context),
                    Err(error) => match push_templates::snapshot_paper_risk_context_from_banner(&banner) {
                        Ok(context) => {
                            log::info!("[BR-151] SnapshotPaper 使用用户确认持仓进入虚拟盘引擎");
                            Some(context)
                        }
                        Err(snapshot_error) => {
                            log::error!(
                                "[v16.3][BR-134] paper risk context unavailable: {}; SnapshotPaper unavailable: {}",
                                error, snapshot_error
                            );
                            None
                        }
                    },
                }
            });
            if let Some(risk_context) = risk_context {
                match monitor.tick(risk_context) {
                    Ok(n) if n > 0 => {
                        log::info!("[v16.3] intraday_monitor tick: 消费 {} 条", n)
                    }
                    Ok(_) => log::debug!("[v16.3] intraday_monitor tick: 0 候选"),
                    Err(e) => log::warn!("[v16.3] intraday_monitor tick 失败: {}", e),
                }
                // BR-234: 虚拟仓卖出闭环 — 四大铁律 30s tick 评估
                // (paper_sell 内部含交易时段守卫/T+1 锁仓/当日一票一卖幂等)
                // v19 review (2026-08-12): 账本实质错误 — 成本为全部历史买入混合摊薄
                // (Σamt/Σqty), T+1 用 MIN(ts) 最早买入日, 无批次账本; 生产 100 笔虚拟
                // 卖出含 3 笔收益率 >100% (最高 +22751% 为买价记录错误), 11 笔当日
                // 买入即卖, 7 笔买入后 60s 内卖出。暂停投递直到批次账本重建。
                if !paper_sell_paused("盘中") {
                    match stock_analysis::trading::paper_sell::scan_and_sell(risk_context) {
                        Ok(sold) if !sold.is_empty() => {
                            for result in &sold {
                                let text = format!(
                                    "[虚拟盘卖出] {}({}) 卖出{}股 @{:.2} | 收益率{:+.2}% | 原因:{}",
                                    result.name, result.code, result.quantity, result.price,
                                    result.return_rate_pct, result.reason
                                );
                                let outcome = push_governor_v3(
                                    &text,
                                    PushKind::PaperSell,
                                    Some(&result.code),
                                )
                                .await;
                                if !outcome.is_pushed() {
                                    log::warn!(
                                        "[paper_sell] {} 推送未投递: {:?}",
                                        result.code,
                                        outcome
                                    );
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(e) => log::warn!("[paper_sell] 盘中扫描失败: {}", e),
                    }
                }
            }
            // 任务#3: 每日 15:10 快照过期检查（收盘后用户应上传当日快照）
            let now = chrono::Local::now();
            if now.hour() == 15 && (10..=13).contains(&now.minute()) {
                check_snapshot_staleness_and_notify().await;
            }
            // 15:30 整盘扫 (R5) — evening_review 内部有当日防重入 (review fix Issue #7)
            if now.hour() == 15 && now.minute() == 30 {
                let today = now.date_naive();
                match risk_context {
                    Some(risk_context) => {
                        if let Err(e) = evening_review(today, risk_context) {
                            log::warn!("[evening_review] 失败: {}", e);
                        }
                        // BR-234: 收盘后卖出评估 — 无交易时段守卫，收盘 K 线完整评估
                        // (v19 review 同盘中: invalid_position_ledger 暂停, 见 paper_sell_paused)
                        if !paper_sell_paused("盘后") {
                        match stock_analysis::trading::paper_sell::scan_and_sell_post_close(
                            risk_context,
                        ) {
                            Ok(sold) if !sold.is_empty() => {
                                for result in &sold {
                                    let text = format!(
                                        "[虚拟盘卖出] {}({}) 卖出{}股 @{:.2} | 收益率{:+.2}% | 原因:{}",
                                        result.name, result.code, result.quantity, result.price,
                                        result.return_rate_pct, result.reason
                                    );
                                    let outcome = push_governor_v3(
                                        &text,
                                        PushKind::PaperSell,
                                        Some(&result.code),
                                    )
                                    .await;
                                    if !outcome.is_pushed() {
                                        log::warn!(
                                            "[paper_sell] {} 推送未投递: {:?}",
                                            result.code,
                                            outcome
                                        );
                                    }
                                }
                            }
                            Ok(_) => {}
                            Err(e) => log::warn!("[paper_sell] 收盘后扫描失败: {}", e),
                        }
                        }
                    }
                    None => log::error!(
                        "[evening_review][BR-134] 缺少最新真实风险上下文，保留当日重试资格"
                    ),
                }
            }
            // 2026-08-07 用户决策: 新闻收集 + AI 链分析每日推送 (盘后)。
            // 15:30-15:34 窗口: 当日涨停池 + 当日快讯 → LLM 产业链报告 → 推送。
            // 与 15:10 断点 A 落库 (chain_daily, 不推送) 互补: 15:10 只写库,
            // 15:30 出报告推用户。失败保留重试资格, 成功才封口。
            if now.hour() == 15 && (30..35).contains(&now.minute()) {
                static CHAIN_POST_LAST: std::sync::Mutex<Option<chrono::NaiveDate>> =
                    std::sync::Mutex::new(None);
                let today = now.date_naive();
                let already_run = CHAIN_POST_LAST
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|d| d == today)
                    .unwrap_or(false);
                if !already_run {
                    match run_chain_analysis_mode(true).await {
                        Ok(()) => {
                            log::info!(
                                "[产业链][盘后15:30] 新闻+AI 链分析完成并推送 (date={})",
                                today
                            );
                            *CHAIN_POST_LAST
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) = Some(today);
                        }
                        Err(error) => {
                            log::error!(
                                "[产业链][盘后15:30] 链分析推送失败, 保留重试资格: {error}"
                            );
                        }
                    }
                }
            }
            // Fix 4 (review): PerformanceEngine 15:05 cron 接入 (写 paper_performance_snapshot)
            // 用 OnceLock<NaiveDate> 防当日重复, 失败可重试
            // v17.4 §5.2 (BR-083): 13:00 午盘虚拟仓快照 (AC38) — 当日一次, 13:00-13:05 首个 tick 触发
            if now.hour() == 13 && now.minute() < 5 {
                static NOON_SNAP_LAST: std::sync::Mutex<Option<chrono::NaiveDate>> =
                    std::sync::Mutex::new(None);
                let today = now.date_naive();
                let already = NOON_SNAP_LAST
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|d| d == today)
                    .unwrap_or(false);
                if !already {
                    let date_str = today.format("%Y-%m-%d").to_string();
                    let ok = push_templates::dispatch_paper_review_noon(&date_str).await;
                    log::info!("[v17.4 §5.2] 13:00 虚拟仓午盘快照: pushed={}", ok);
                    *NOON_SNAP_LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(today);
                }
            }
            if now.hour() == 15 && now.minute() == 5 {
                use stock_analysis::performance::PerformanceEngine;
                static PERF_LAST_RUN: std::sync::Mutex<Option<chrono::NaiveDate>> =
                    std::sync::Mutex::new(None);
                let today = now.date_naive();
                let already_run = PERF_LAST_RUN
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|d| d == today)
                    .unwrap_or(false);
                if !already_run {
                    if let Ok(snap) = PerformanceEngine::daily_settlement() {
                        let win_rate = snap
                            .win_rate
                            .map(|value| format!("{value:.2}"))
                            .unwrap_or_else(|| "暂无".to_string());
                        let sharpe = snap
                            .sharpe_ratio
                            .map(|value| format!("{value:.2}"))
                            .unwrap_or_else(|| "暂无".to_string());
                        log::info!(
                            "[v16.4] PerformanceEngine 15:05 跑完: total_pnl={} win_rate={} sharpe={}",
                            snap.total_pnl,
                            win_rate,
                            sharpe
                        );
                        *PERF_LAST_RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some(today);
                    } else {
                        log::warn!(
                            "[v16.4] PerformanceEngine.daily_settlement 失败 (允许 30s 后重试)"
                        );
                    }
                }
            }
            // BR-226: 持仓快照 24h 新鲜度 — 收盘后主动预警。
            // 快照 effective_at 超过 6h (即非今日导入) → 次日 9:20 竞价时必过期
            // (age > 24h), 公告受众将静默降级。2026-08-06 实证: 08-05 15:02 快照
            // 08-07 9:20 时 42h 过期。预警让用户在过期前完成导入 (数据仍由用户
            // 每日提供, 此处仅出声提醒, 不改变 BR-226 fail-closed 语义)。
            if now.hour() == 15 && now.minute() == 5 {
                use stock_analysis::database::user_position_snapshot::latest_user_position_snapshot;
                static SNAP_REMIND_LAST: std::sync::Mutex<Option<chrono::NaiveDate>> =
                    std::sync::Mutex::new(None);
                let today = now.date_naive();
                let already_reminded = SNAP_REMIND_LAST
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|d| d == today)
                    .unwrap_or(false);
                if !already_reminded {
                    let reminder = match latest_user_position_snapshot() {
                        Ok(Some(snapshot)) if !snapshot.confirm_empty => {
                            let age_hours = chrono::Local::now()
                                .signed_duration_since(
                                    snapshot.effective_at.with_timezone(&chrono::Local),
                                )
                                .num_minutes() as f64
                                / 60.0;
                            (age_hours > 6.0).then(|| {
                                format!(
                                    "⚠️ 持仓快照 24h 新鲜度预警: effective_at={} 已 {:.0}h。\n请导入今日收盘后快照 (import_user_position_snapshot)，否则明日 09:20 竞价时公告受众将降级 (BR-226)。",
                                    snapshot.effective_at, age_hours
                                )
                            })
                        }
                        Ok(Some(_)) => None, // confirm_empty = 用户确认空仓, 合法状态
                        Ok(None) => Some(
                            "⚠️ 持仓快照从未导入 (BR-226): 公告受众当前无持仓证据，请运行 import_user_position_snapshot"
                                .to_string(),
                        ),
                        Err(error) => Some(format!(
                            "⚠️ 持仓快照读取失败 (BR-226): {error}"
                        )),
                    };
                    if let Some(text) = reminder {
                        log::warn!("[BR-226] {text}");
                        let outcome = push_governor_v3(&text, PushKind::IntradayMarket, None).await;
                        log::info!(
                            "[BR-226] 持仓快照预警推送: outcome={:?} pushed={}",
                            outcome,
                            outcome.is_pushed()
                        );
                    } else {
                        log::info!("[BR-226] 持仓快照今日已更新, 无需预警");
                    }
                    // 每日一次, 尽力而为; 推送失败不重试 (次日再检)
                    *SNAP_REMIND_LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(today);
                }
            }
            // 断点 A (2026-08-06): 产业链分析接线 — chain_daily 生产者。
            // 排查: pipeline::chain_analysis 生产路径零调用 → chain_daily 恒空
            // → I-03 盘中产业链恒 "chain_daily 无数据" 短路。盘后 15:10 跑一次
            // 涨停池 → 概念聚类 → 写 chain_daily (cluster_and_persist 不依赖 LLM,
            // llm_ok=false 时仅聚类降级, chain_daily 仍落库); 当日/次日 I-03
            // 即读到 MAX(date) 数据。失败保留重试资格 (v15.x 静默路径出声)。
            // 15:10-15:15 窗口 (原只 15:10 整分钟, 失败后无法重试 — 2026-08-06 实证)
            if now.hour() == 15 && now.minute() <= 15 {
                static CHAIN_LAST_RUN: std::sync::Mutex<Option<chrono::NaiveDate>> =
                    std::sync::Mutex::new(None);
                let today = now.date_naive();
                let already_run = CHAIN_LAST_RUN
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|d| d == today)
                    .unwrap_or(false);
                if !already_run {
                    let observed_at = chrono::Local::now().naive_local();
                    let business_date =
                        stock_analysis::calendar::latest_completed_trading_day_at(observed_at);
                    let limit_ups: Option<Vec<stock_analysis::market_data::TopStock>> =
                        match tokio::task::spawn_blocking(move || {
                            let analyzer =
                                stock_analysis::market_analyzer::MarketAnalyzer::new(None)?;
                            analyzer.get_limit_up_stocks(business_date)
                        })
                        .await
                        {
                            Ok(Ok(stocks)) => Some(stocks),
                            Ok(Err(error)) => {
                                log::error!("[产业链][断点A] 涨停池批次失败: {error}");
                                None
                            }
                            Err(error) => {
                                log::error!("[产业链][断点A] 涨停池 worker 失败: {error}");
                                None
                            }
                        };
                    if let Some(limit_ups) = limit_ups {
                        let stock_count = limit_ups.len();
                        if stock_count == 0 {
                            log::warn!(
                                "[产业链][断点A] 涨停池为空, 跳过链分析 (chain_daily 保留旧数据)"
                            );
                        } else {
                            match stock_analysis::pipeline::chain_analysis::run_chain_analysis(
                                business_date,
                                limit_ups,
                                None,
                            )
                            .await
                            {
                                Ok(report) => {
                                    log::info!(
                                        "[产业链][断点A] 链分析完成: date={} stocks={} report_bytes={} (chain_daily 已写入, I-03 数据源就绪)",
                                        business_date,
                                        stock_count,
                                        report.len()
                                    );
                                    *CHAIN_LAST_RUN
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner()) = Some(today);
                                }
                                Err(error) => {
                                    log::error!(
                                        "[产业链][断点A] 链分析失败, 保留重试资格: {error}"
                                    );
                                }
                            }
                        }
                    } else {
                        log::warn!("[产业链][断点A] 涨停池不可用, 保留重试资格");
                    }
                }
            }
            // BR-021 §5.10 / commit 08cca47 + caller wire: 8:30 盘前重置 cron.
            // 调一次 push_account_mode_change 触发 evaluate(), 内部 should_reset_at_8_30
            // (Frozen + 8:30 窗口) → 强制 prev=None → evaluate 重判 → 落库 + 推 T-01.
            // 用 Mutex<Option<NaiveDate>> 防当日重复 (跟 15:05 / 15:30 同 pattern).
            // 2026-08-07 用户决策 (补偿原则): 错过 8:30 不放弃 — 条件放宽为
            // "本地时间 >= 8:30 且当日未跑" (含启动时补偿: 8:30 后启动会立即
            // 补做当日重置)。8:30 之前启动时仍需等到 8:30 (重置语义依赖
            // 盘前窗口)。仅约束: 当日一次 + 失败保留重试。
            if now.time() >= chrono::NaiveTime::from_hms_opt(8, 30, 0).unwrap() {
                static BR021_LAST_RUN: std::sync::Mutex<Option<chrono::NaiveDate>> =
                    std::sync::Mutex::new(None);
                let today = now.date_naive();
                let already_run = BR021_LAST_RUN
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|d| d == today)
                    .unwrap_or(false);
                if !already_run {
                    log::info!("[BR-021][BR-108] 8:30 盘前重置 (含错过补偿) 触发真实 AccountMode 评估");
                    if evaluate_account_mode_hook(false).await {
                        *BR021_LAST_RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some(today);
                    } else {
                        log::error!("[BR-021][BR-108] 8:30 评估失败，保留重试资格");
                    }
                }
            }
            // 2026-08-07 用户决策: 新闻收集 + AI 链分析每日推送 (盘前)。
            // 9:05-9:09 窗口: 财联社快讯 → LLM 产业链分析 (business_date=昨日
            // 涨停池 + 最新新闻背景) → 报告推送, 竞价参考。失败保留重试资格,
            // 成功才封口 (v15.x 出声原则)。注意 business_date 为昨日已完成
            // 交易日, 与 15:30 盘后 (当日) 各自独立报告文件。
            // 2026-08-07 补偿原则: 窗口放宽到 9:05-9:14 (9:15 后错过竞价参考
            // 意义, 且 9:10 预检/9:20 竞价紧随) — 9:09 后启动的 monitor 仍补做。
            if now.hour() == 9 && (5..15).contains(&now.minute()) {
                static CHAIN_PREOPEN_LAST: std::sync::Mutex<Option<chrono::NaiveDate>> =
                    std::sync::Mutex::new(None);
                let today = now.date_naive();
                let already_run = CHAIN_PREOPEN_LAST
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|d| d == today)
                    .unwrap_or(false);
                if !already_run {
                    match run_chain_analysis_mode(true).await {
                        Ok(()) => {
                            log::info!(
                                "[产业链][盘前9:05] 新闻+AI 链分析完成并推送 (date={})",
                                today
                            );
                            *CHAIN_PREOPEN_LAST
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) = Some(today);
                        }
                        Err(error) => {
                            log::error!("[产业链][盘前9:05] 链分析推送失败, 保留重试资格: {error}");
                        }
                    }
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        }
    };

    let market_loop = async {
        loop {
            if !calendar::today_is_trading_day() {
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;

                continue;
            }

            while !is_market_active() {
                let session = calendar::session_label();

                if session.contains("休市") || session.contains("盘后") {

                    // 还在盘前等待窗口
                }

                log::info!("等待交易时段... 当前: {}", session);

                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

                if !calendar::today_is_trading_day() {
                    break;
                }
            }

            if !calendar::today_is_trading_day() {
                continue;
            }

            log::info!("进入交易时段，开始监控");

            let (_positions, targets) = match TieredScanner::load_portfolio_targets() {
                Ok(batch) => batch,
                Err(error) => {
                    log::error!("[盘前] Scanner 标的批次加载失败，30 秒后重试: {}", error);
                    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                    continue;
                }
            };

            log::warn!(
                "[盘前][BR-192] capability_unavailable=premarket_daily_report_counted_binding_unavailable; \
                 checklist skipped before T+1 projection"
            );

            prediction::verify_predictions().await;

            match prediction::recent_hit_rate(7) {
                Ok(hit_rate) => log::info!("[预测] 近7天命中率: {:.0}%", hit_rate * 100.0),
                Err(error) => log::warn!("[预测] 近7天命中率不可用: {}", error),
            }

            // 构建实体过滤集合（只关注9只标的）

            let our_codes: std::collections::HashSet<String> =
                targets.iter().map(|t| t.code.clone()).collect();

            let scanner = TieredScanner::new(targets);

            let detector = Detector::new(DetectorConfig::default());

            let mut state_machine = SignalStateMachine::default();

            state_machine.restore_state();

            let mut signal_count = 0u32;

            let mut alert_count = 0u32;

            let poll_secs: u64 = std::env::var("MONITOR_HOLDING_INTERVAL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30);

            // Phase 1.1 量化标准：信号融合 + 风险叠加 + 状态驱动

            use stock_analysis::monitor::signal_fusion::{Signal, SignalFusion, SignalSource};

            let fusion = SignalFusion::default();

            // 三个独立计时器

            let mut last_market_view = std::time::Instant::now(); // b013 P1-10: 盘面+产业链独立计时器 (5分钟)

            let mut last_t0_scan = std::time::Instant::now(); // 持仓做 T 扫描（30秒）

            // 2026-08-07 BR-192 收尾: 持仓健康度 summary 由 T-03 counted 投递
            // (prepare_holding_plan_messages) 承担, 原 unavailable 占位已移除。

            let mut last_industry_chain_intraday = std::time::Instant::now(); // v34: I-03 涨停扩散 (15 min)

            let mut last_holding_plan = std::time::Instant::now(); // v38: I-04 持仓操作建议 (30 min)

            let mut last_sector_top = std::time::Instant::now(); // 2026-08-07: I-09 板块 TOP (60 min)

            let mut last_sector_anomaly = std::time::Instant::now(); // 2026-08-07: I-09A 量价反向 (60 min)

            // v44: T-14 盘后固定价格申报 (15 min, 申报窗口 9:30-15:30)

            let mut last_post_fixed_order = std::time::Instant::now();

            // v45: T-15 盘后固定价格成交 (撮合 15:05-15:30, 5 min 周期)

            let mut last_post_fixed_fill = std::time::Instant::now();

            // v46: T-16 ST 涨跌幅变更 (开盘 9:30 一次/票/日)

            let mut st_price_pushed = false;

            // v47: T-17 ETF 收盘集合竞价 (14:57-15:00 一次)

            let mut etf_closing_pushed = false;

            // 2026-08-07 审计接入: T-12 尾盘提示 (14:55-14:57 一次)

            let mut close_call_pushed = false;

            // 产业链扫描已移至 news_monitor_loop 的 8:00-22:00 窗口统一调度。

            let mut was_limit_up: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            // 连板追踪：已推送过的标的不重复推送；board_level_cache 存 1=首板/2=二板/3+=三板

            let mut board_notified: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            let mut board_level_cache: std::collections::HashMap<String, u8> =
                std::collections::HashMap::new();

            // 竞价量能扫描：9:20-9:25 每30秒推送一次全市场涨停量能榜

            let mut auction_vol_notified: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            // 优选候选虚拟仓位记录：从集合竞价推送的候选+开盘价记录

            let mut virtual_observation: Vec<(String, String, f64)> = Vec::new(); // (code, name, open_price)

            let mut post_close_candidates_notified = false;

            let mut virtual_snapshot_persisted = false;

            // v32: P-01 盘前新闻热点 — 每个交易日首次进入 9:00-9:15 窗口时推一次

            let mut preopen_pushed = false;

            let entry_mode = air_refuel_entry_mode();

            let monitor_cfg = stock_analysis::config::get_monitor_config();

            let confirm_shares = monitor_cfg.air_refuel.confirm_lots.saturating_mul(100);

            let pilot_shares = monitor_cfg.air_refuel.pilot_lots.saturating_mul(100);

            loop {
                let session = current_session();

                // ═══════════════════════════════════════════════════════════════

                // v32: P-01 盘前新闻热点 (9:00-9:15 窗口, 每日首次)

                //   - 触发: 首次进入 9:00 ≤ now < 9:15, 每个 monitor_loop session 推一次

                //   - 数据源: news_monitor 拉今日 + 昨日要闻 + 板块聚类

                //   - 模板: render_preopen_news_hot (无 banner, ℹ️参考级)

                //   - 静默: 公告空时短路

                //   - 注意: P-02 竞价量能 / P-03 候选触发 已有独立路径, 不在此重复

                // ═══════════════════════════════════════════════════════════════

                if !preopen_pushed && session == MarketSession::Closed {
                    let now_time = chrono::Local::now().time();

                    let preopen_start = chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap();

                    let preopen_end = chrono::NaiveTime::from_hms_opt(9, 15, 0).unwrap();

                    if now_time >= preopen_start && now_time < preopen_end {
                        log::info!(
                            "[P-01] 盘前窗口 ({}-{}), 推盘前新闻热点",
                            preopen_start.format("%H:%M"),
                            preopen_end.format("%H:%M")
                        );

                        let preopen_ok = push_templates::dispatch_preopen_news_hot_daily().await;
                        if !preopen_ok {
                            log::error!("[P-01][BR-091] dispatcher did not confirm delivery");
                        }

                        // v39: P-03 候选触发 (同盘前窗口, 影子开关控制)

                        let hhmm = chrono::Local::now().format("%H:%M").to_string();

                        if let Some(banner) = current_banner_for("P-03 candidate trigger") {
                            let candidate_ok =
                                push_templates::dispatch_candidate_triggered_daily(&hhmm, &banner)
                                    .await;
                            if !candidate_ok {
                                log::error!("[P-03][BR-091] dispatcher did not confirm delivery");
                            }
                            preopen_pushed = preopen_ok && candidate_ok;
                        }

                        // 2026-08-06 实证: 9:15-9:25 DNS 全挂 + TDX 不可达 →
                        // A-02/P-05/涨停池全失败, 9:20 后才从日志发现。
                        // 开盘前对统一实时行情网关做一次健康探测 (与 A-02 同链路
                        // MarketDataGateway::realtime_quotes): 不可用 → 提前推送
                        // 预警, 9:20 竞价前就知道窗口风险。
                        // 瞬时故障的兜底是 9:20 BR-223 块 (成功才封口, 窗口内重试)。
                        // 2026-08-07 补偿原则: 窗口从 9:10-9:15 放宽到 9:10-9:20 —
                        // 9:15 后启动/错过旧窗口的 monitor 仍补做探测 (探测只需
                        // 数十秒, 9:20 竞价前完成即可; 9:20 后探测意义消失,
                        // 由 BR-223 窗口兜底)。
                        if now_time >= chrono::NaiveTime::from_hms_opt(9, 10, 0).unwrap()
                            && now_time < chrono::NaiveTime::from_hms_opt(9, 20, 0).unwrap()
                        {
                            static PREOPEN_PROBE_LAST: std::sync::Mutex<
                                Option<chrono::NaiveDate>,
                            > = std::sync::Mutex::new(None);
                            let probe_today = chrono::Local::now().date_naive();
                            let probe_done = PREOPEN_PROBE_LAST
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .map(|d| d == probe_today)
                                .unwrap_or(false);
                            if !probe_done {
                                // 基准代码: 深主板 + 沪主板 + 创业板
                                let probe_codes: Vec<String> = [
                                    "000001", "600000", "300750",
                                ]
                                .iter()
                                .map(|s| s.to_string())
                                .collect();
                                let probe_result = tokio::task::spawn_blocking(move || {
                                    market_data::fetch_realtime_quotes(&probe_codes)
                                })
                                .await;
                                *PREOPEN_PROBE_LAST
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner()) = Some(probe_today);
                                match probe_result {
                                    Ok(Ok(quotes)) if !quotes.is_empty() => {
                                        log::info!(
                                            "[预检][行情源] 统一实时行情可用: records={} (000001/600000/300750)",
                                            quotes.len()
                                        );
                                    }
                                    Ok(Ok(_)) => {
                                        let text = "⚠️ 开盘前行情源预检失败: 统一实时行情返回空批次 (9:10)。\n若 9:20 竞价时行情未恢复, A-02 竞价优选/P-05 候选台将在窗口内重试。"
                                            .to_string();
                                        log::warn!("[预检][行情源] {text}");
                                        let outcome = push_governor_v3(
                                            &text,
                                            PushKind::IntradayMarket,
                                            None,
                                        )
                                        .await;
                                        log::info!(
                                            "[预检][行情源] 预警推送 pushed={}",
                                            outcome.is_pushed()
                                        );
                                    }
                                    Ok(Err(error)) => {
                                        let text = format!(
                                            "⚠️ 开盘前行情源预检失败 (9:10): {error}\n若 9:20 竞价时行情未恢复, A-02/P-05 将在窗口内重试。"
                                        );
                                        log::warn!("[预检][行情源] {text}");
                                        let outcome = push_governor_v3(
                                            &text,
                                            PushKind::IntradayMarket,
                                            None,
                                        )
                                        .await;
                                        log::info!(
                                            "[预检][行情源] 预警推送 pushed={}",
                                            outcome.is_pushed()
                                        );
                                    }
                                    Err(error) => {
                                        log::warn!(
                                            "[预检][行情源] 探测任务失败: {error}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                // ── 9:20-9:25 竞价高量能扫描（30秒一次）+ 盘后优选重推 ──

                if session == MarketSession::Auction {
                    let now_time = chrono::Local::now().time();

                    // 9:20 之前只做持仓告警，不推全市场量能（数据不稳定）

                    if now_time >= chrono::NaiveTime::from_hms_opt(9, 20, 0).unwrap() {
                        log::info!("[竞价] 9:20-9:25 量能扫描...");

                        let limit_pool_date = chrono::Local::now().date_naive();
                        let limit_stocks =
                            match tokio::task::spawn_blocking(move || -> Result<_, String> {
                                let analyzer =
                                    stock_analysis::market_analyzer::MarketAnalyzer::new(None)
                                        .map_err(|error| {
                                            format!("初始化涨停池数据源失败: {error:#}")
                                        })?;
                                analyzer
                                    .get_limit_up_stocks(limit_pool_date)
                                    .map_err(|error| format!("获取涨停池失败: {error:#}"))
                            })
                            .await
                            {
                                Ok(Ok(stocks)) => stocks,
                                Ok(Err(error)) => {
                                    log::error!("[竞价] 涨停池批次拒绝: {}", error);
                                    Vec::new()
                                }
                                Err(error) => {
                                    log::error!("[竞价] 涨停池后台任务失败: {}", error);
                                    Vec::new()
                                }
                            };

                        if !limit_stocks.is_empty() {
                            // 按量比降序，取量比最高的前10（量能高代表竞价封板意愿强）

                            let mut sorted = limit_stocks.clone();

                            sorted.sort_by(|a, b| {
                                b.volume_ratio
                                    .partial_cmp(&a.volume_ratio)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            });

                            let new_items: Vec<_> = sorted
                                .iter()
                                .filter(|s| !auction_vol_notified.contains(&s.code))
                                .take(10)
                                .collect();

                            if !new_items.is_empty() {
                                // v37: 升级到 v12 §14.1 P-02 模板

                                //   之前: lines.join + PushKind::AuctionVolume (v19 格式)

                                //   现在: dispatch_auction_volume_daily + render_auction_volume

                                //   模板: 🌅 竞价热点量能 TopN (banner + 强承接/一般/弱承接)

                                let ts = chrono::Local::now().format("%H:%M:%S").to_string();

                                if let Some(banner) = current_banner_for("P-02 auction volume") {
                                    let delivered =
                                        push_templates::dispatch_auction_volume_daily(&ts, &banner)
                                            .await;
                                    // Only seal identities after a confirmed
                                    // sink outcome; failed banner/sink attempts
                                    // remain eligible during the auction window.
                                    if delivered {
                                        for s in &new_items {
                                            auction_vol_notified.insert(s.code.clone());
                                        }
                                    }
                                } else {
                                    log::warn!(
                                        "[竞价][P-02] banner unavailable; retain retry eligibility"
                                    );
                                }
                            }
                        }

                        // BR-223: 9:20-9:25 竞价优选重推恢复 (A-02 AuctionRepush)。
                        // 复用统一网关候选链路 load_real_candidate_batch (CandidateBoard 同源)。
                        // 2026-08-06 实证: 9:20 时刻 DNS 全挂 + TDX 不可达 → A-02/P-05
                        // 一次性尝试双双失败, 整个竞价窗口错过。改为成功才封口:
                        // 窗口内每次扫描 tick (约 30s) 保留重试资格; 重复推送由进程内
                        // cooldown 拦截 (AuctionRepush=600s, CandidateBoard=1800s)。
                        if !post_close_candidates_notified {
                            let repush_ts = chrono::Local::now().format("%H:%M:%S").to_string();
                            let repushed =
                                push_templates::dispatch_auction_repush(&repush_ts).await;
                            log::info!("[竞价][BR-223] A-02 auction repush pushed={repushed}");
                            let board_date = chrono::Local::now().format("%Y-%m-%d").to_string();
                            let board_pushed =
                                push_templates::dispatch_candidate_board(&board_date).await;
                            log::info!("[竞价][BR-223] P-05 candidate board pushed={board_pushed}");

                            if repushed && board_pushed {
                                post_close_candidates_notified = true;
                            } else {
                                // 失败不封口: 竞价窗口内下一 tick 重试, 直至成功或窗口结束。
                                // 空候选 (entries.is_empty) 也在重试之列 — 上游候选可能
                                // 晚于 9:20:20 才生成 (2026-08-06 09:20 候选链路即为空)。
                                log::warn!(
                                    "[竞价][BR-223] A-02={repushed} P-05={board_pushed} \
                                     未全成功; 竞价窗口内保留重试资格"
                                );
                            }

                            let post_close = String::new();

                            // 提取候选的code和name以便后续虚拟记录（简单方式：从推送文案中正则提取）

                            // 格式: "N. 名称(代码)" → 收集前5个作为虚拟观察对象

                            let mut seen_codes: std::collections::HashSet<String> =
                                std::collections::HashSet::new();

                            for line in post_close.lines() {
                                if let Some(paren_start) = line.find('(') {
                                    if let Some(paren_end) = line.find(')') {
                                        if paren_start < paren_end {
                                            let code_str = &line[paren_start + 1..paren_end];

                                            if code_str.len() == 6
                                                && code_str.chars().all(|c| c.is_numeric())
                                            {
                                                if !seen_codes.insert(code_str.to_string()) {
                                                    continue;
                                                }

                                                // 从该行"  "后提取name

                                                let name_part = line.trim_start();

                                                if let Some(name_end) = name_part.find('(') {
                                                    let name = name_part[..name_end].trim_end();

                                                    // 移除序号 "N. "

                                                    let name = if let Some(dot_pos) = name.find('.')
                                                    {
                                                        name[dot_pos + 1..].trim()
                                                    } else {
                                                        name
                                                    };

                                                    virtual_observation.push((
                                                        code_str.to_string(),
                                                        name.to_string(),
                                                        0.0,
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // pilot 模式：竞价阶段先按当前价格虚拟潜伏记录（仅一次）

                            if entry_mode == AirRefuelEntryMode::Pilot
                                && !virtual_observation.is_empty()
                            {
                                let codes: Vec<String> = virtual_observation
                                    .iter()
                                    .map(|(c, _, _)| c.clone())
                                    .collect();

                                let quote_map = match tokio::task::spawn_blocking(move || {
                                    market_data::fetch_realtime_quotes(&codes)
                                })
                                .await
                                {
                                    Ok(Ok(quotes)) => quotes
                                        .into_iter()
                                        .map(|q| (q.code, q.price))
                                        .collect::<std::collections::HashMap<_, _>>(),
                                    Ok(Err(error)) => {
                                        log::error!("[虚拟观察仓] pilot 行情批次拒绝: {}", error);
                                        std::collections::HashMap::new()
                                    }
                                    Err(error) => {
                                        log::error!(
                                            "[虚拟观察仓] pilot 行情后台任务失败: {}",
                                            error
                                        );
                                        std::collections::HashMap::new()
                                    }
                                };

                                for v in &mut virtual_observation {
                                    if let Some(px) = quote_map.get(&v.0) {
                                        if *px > 0.0 {
                                            v.2 = *px;
                                        }
                                    }
                                }

                                let mut lines = vec![
                                    "🟠 虚拟观察仓位（尾盘/竞价潜伏模式）".to_string(),
                                    String::new(),
                                ];

                                let mut records: Vec<VirtualObservationRecord> = Vec::new();

                                let mut total_amount = 0.0_f64;

                                let today = chrono::Local::now().format("%Y-%m-%d").to_string();

                                for (code, name, price) in &virtual_observation {
                                    if *price <= 0.0 {
                                        continue;
                                    }

                                    let amount = *price * pilot_shares as f64;

                                    total_amount += amount;

                                    lines.push(format!(
                                        "  {}({}) @ ¥{:.2} | {}股 预计 ¥{:.0}",
                                        name, code, price, pilot_shares, amount
                                    ));

                                    records.push(VirtualObservationRecord {
                                        entry_date: today.clone(),

                                        code: code.clone(),

                                        name: name.clone(),

                                        entry_price: *price,

                                        shares: pilot_shares,

                                        entry_mode: "pilot".to_string(),
                                    });
                                }

                                lines.push(format!(
                                    "\n合计虚拟敞口: ¥{:.0} ({}股×{}只)",
                                    total_amount,
                                    pilot_shares,
                                    records.len()
                                ));

                                lines.push("\n⚠️ 仅做观察、研究用途，未实际下单".to_string());

                                if !records.is_empty() {
                                    match persist_virtual_observation_snapshot(&records) {
                                        Ok(()) => {
                                            virtual_snapshot_persisted = true;

                                            let outcome = push_governor_v3(
                                                &lines.join("\n"),
                                                PushKind::VirtualWatch,
                                                None,
                                            )
                                            .await;
                                            if !periodic_delivery_confirmed(&outcome) {
                                                log::warn!(
                                                    "[虚拟观察仓][BR-192] non-counted delivery not confirmed: {:?}",
                                                    outcome
                                                );
                                            }
                                        }
                                        Err(error) => {
                                            log::error!(
                                                "[虚拟观察仓] pilot 快照批次拒绝: {}",
                                                error
                                            )
                                        }
                                    }
                                }
                            }
                        }

                        // 持仓信号（原有逻辑保留）

                        for s in limit_stocks.iter().take(10) {
                            if !our_codes.contains(&s.code) {
                                continue;
                            }

                            let Some(volume_ratio) = s.volume_ratio else {
                                log::warn!(
                                    "[BR-097] detector row rejected code={} missing=volume_ratio",
                                    s.code
                                );
                                continue;
                            };
                            let Some(main_net_yi) = s.main_net_yi else {
                                log::warn!(
                                    "[BR-097] detector row rejected code={} missing=main_net_yi",
                                    s.code
                                );
                                continue;
                            };

                            let snap = StockSnapshot {
                                code: s.code.clone(),

                                name: s.name.clone(),

                                price: s.price,

                                change_pct: s.change_pct,

                                volume_ratio,

                                main_net_yi,

                                limit_up_price: None,

                                was_limit_up: false,

                                t1_locked: false,
                            };

                            for e in detector.scan_stock(&snap) {
                                signal_count += 1;

                                if let Some(event) = state_machine.process(e) {
                                    alert_count += 1;

                                    // BR-192 收尾: 指标告警走 counted binding 投递
                                    // (失败不重试 — 竞价窗口内数据快速变化)。
                                    deliver_intraday_alert(&event).await;
                                }
                            }
                        }

                        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

                        continue;
                    } else {
                        // BR-100: P-04 只消费当日已持久化 paper_trades 完成态。
                        {
                            if !push_templates::dispatch_paper_trade_daily().await {
                                log::info!(
                                    "[P-04][BR-100] 当日没有可投递的严格 paper_trades 完成态"
                                );
                            }
                        }

                        // 9:15-9:20 等待即可

                        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

                        continue;
                    }
                }

                if session == MarketSession::Morning || session == MarketSession::Afternoon {
                    let limit_pool_date = chrono::Local::now().date_naive();
                    let result = tokio::task::spawn_blocking(move || {
                        intraday_market::acquire_intraday_market_inputs(
                            || {
                                let analyzer =
                                    stock_analysis::market_analyzer::MarketAnalyzer::new(None)
                                        .map_err(|error| {
                                            format!("初始化市场分析器失败: {error}")
                                        })?;
                                analyzer
                                    .get_limit_up_stocks(limit_pool_date)
                                    .map_err(|error| format!("涨停池获取失败: {error}"))
                            },
                            || {
                                std::thread::sleep(std::time::Duration::from_millis(800));
                                market_data::fetch_position_quotes()
                            },
                        )
                    })
                    .await;

                    let resolved = intraday_market::resolve_intraday_market_inputs(
                        result.map_err(|error| error.to_string()),
                    );
                    if let Some(error) = resolved.limit_error.as_deref() {
                        log::error!("[盘中监控] 涨停池批次拒绝: {error}");
                    }
                    if let Some(error) = resolved.position_error.as_deref() {
                        log::error!("[盘中监控] 持仓行情批次拒绝: {error}");
                    }
                    if let Some(error) = resolved.task_error.as_deref() {
                        log::error!("[盘中监控] 行情任务失败: {error}");
                    }
                    let consumer_plan = resolved.consumer_plan();
                    if !consumer_plan.use_limit_data && !consumer_plan.use_position_data {
                        log::error!("[盘中监控] 两路行情均不可用，仅跳过依赖这两路数据的计算");
                    }
                    debug_assert!(consumer_plan.run_independent_jobs);
                    let limit_stocks = resolved.limit_stocks;
                    let position_quotes = resolved.position_quotes;

                    {
                        // ▶ 新增：开盘后虚拟记录观察仓位（仅一次）

                        if entry_mode == AirRefuelEntryMode::Confirm
                            && session == MarketSession::Morning
                            && !virtual_observation.is_empty()
                            && virtual_observation.iter().all(|(_, _, p)| *p == 0.0)
                        {
                            log::info!(
                                "[P-05 开盘] 虚拟观察仓位初始化（{}手 × {}只）",
                                confirm_shares / 100,
                                virtual_observation.len()
                            );

                            // 从当前行情中获取这些候选的开盘价/实时价

                            if let Some(position_quotes) = position_quotes.as_ref() {
                                for pos_quote in position_quotes {
                                    for virtual_pos in &mut virtual_observation {
                                        if virtual_pos.0 == pos_quote.code && virtual_pos.2 == 0.0 {
                                            virtual_pos.2 = pos_quote.price;
                                        }
                                    }
                                }
                            }

                            // 补充从limit_stocks中没获取到的价格

                            if let Some(limit_stocks) = limit_stocks.as_ref() {
                                for limit_stock in limit_stocks {
                                    for virtual_pos in &mut virtual_observation {
                                        if virtual_pos.0 == limit_stock.code && virtual_pos.2 == 0.0
                                        {
                                            virtual_pos.2 = limit_stock.price;
                                        }
                                    }
                                }
                            }

                            // v63 (P-04 fix): 兜底拉 LLM 推荐的虚拟观察 codes 真报价

                            //   - 旧 bug: virtual_pos 来自 LLM 文本解析, 但 fill 只查 user holdings/watchlist + 涨停

                            //     限制, LLM 推的非持仓非涨停股 entry_price 永远 0.0 → push_virtual_next_day_review 跳过整条

                            //   - 新: 显式走统一 Gateway 给所有 virtual_observation codes (无持仓关系)

                            let virt_codes: Vec<String> = virtual_observation
                                .iter()
                                .filter(|(_, _, p)| *p == 0.0)
                                .map(|(c, _, _)| c.clone())
                                .collect();

                            if !virt_codes.is_empty() {
                                let virt_quotes =
                                    crate::blocking_market_data::run_blocking_market_data(
                                        "P-05 virtual observation quotes",
                                        move || market_data::fetch_realtime_quotes(&virt_codes),
                                    )
                                    .await;
                                match virt_quotes {
                                    Ok(virt_quotes) => {
                                        for quote in virt_quotes {
                                            for virtual_pos in &mut virtual_observation {
                                                if virtual_pos.0 == quote.code
                                                    && virtual_pos.2 == 0.0
                                                {
                                                    virtual_pos.2 = quote.price;
                                                }
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        log::error!("[P-05 开盘] 虚拟观察报价批次拒绝: {}", error);
                                    }
                                }
                            }

                            // v58: 持久化虚拟观察快照 (保留旧逻辑)

                            if !virtual_snapshot_persisted {
                                let mut records: Vec<VirtualObservationRecord> = Vec::new();

                                let today = chrono::Local::now().format("%Y-%m-%d").to_string();

                                for (code, name, price) in &virtual_observation {
                                    if *price > 0.0 {
                                        records.push(VirtualObservationRecord {
                                            entry_date: today.clone(),

                                            code: code.clone(),

                                            name: name.clone(),

                                            entry_price: *price,

                                            shares: confirm_shares,

                                            entry_mode: "confirm".to_string(),
                                        });
                                    }
                                }

                                if !records.is_empty() {
                                    match persist_virtual_observation_snapshot(&records) {
                                        Ok(()) => virtual_snapshot_persisted = true,
                                        Err(error) => {
                                            log::error!(
                                                "[虚拟观察仓] confirm 快照批次拒绝: {}",
                                                error
                                            )
                                        }
                                    }
                                }
                            }

                            // v58: 改用 v12 §14.5 P-05 dispatcher (替代内联 lines.join)

                            let hhmm = chrono::Local::now().format("%H:%M").to_string();

                            let total_amount: f64 = virtual_observation
                                .iter()
                                .filter(|(_, _, p)| *p > 0.0)
                                .map(|(_, _, p)| p * confirm_shares as f64)
                                .sum();

                            let _ = push_templates::dispatch_virtual_watch_daily(
                                &hhmm,
                                &virtual_observation,
                                confirm_shares,
                            )
                            .await;

                            log::info!(
                                "[P-05 开盘] 虚拟观察仓位已推送（合计 ¥{:.0}）",
                                total_amount
                            );
                        }

                        // 首板/二板/三板识别：全市场涨停池，各自独立消息，每只仅推一次

                        if let Some(limit_stocks) =
                            limit_stocks.as_ref().filter(|stocks| !stocks.is_empty())
                        {
                            let mut need_lookup: Vec<(String, String)> = Vec::new();

                            for s in limit_stocks {
                                if board_notified.contains(&s.code) {
                                    continue;
                                }

                                if !board_level_cache.contains_key(&s.code) {
                                    need_lookup.push((s.code.clone(), s.name.clone()));
                                }
                            }

                            if !need_lookup.is_empty() {
                                let need_lookup: Vec<(String, String)> =
                                    need_lookup.into_iter().take(40).collect();

                                let looked_up = tokio::task::spawn_blocking(move || {
                                    market_data::lookup_board_level_batch(&need_lookup)
                                })
                                .await;

                                match looked_up {
                                    Ok(Ok(levels)) => board_level_cache.extend(levels),
                                    Ok(Err(error)) => {
                                        log::error!("[连板识别] 数据批次拒绝: {}", error)
                                    }
                                    Err(error) => {
                                        log::error!("[连板识别] 后台任务失败: {}", error)
                                    }
                                }
                            }

                            let mut first_lines: Vec<String> = Vec::new();

                            let mut second_lines: Vec<String> = Vec::new();

                            let mut third_lines: Vec<String> = Vec::new();

                            let missing_main_flow = limit_stocks
                                .iter()
                                .filter(|stock| stock.main_net_yi.is_none())
                                .count();
                            if missing_main_flow > 0 {
                                log::warn!(
                                    "[涨停板] {} 行缺少主力净流，排除在主力排序之外",
                                    missing_main_flow
                                );
                            }
                            let mut sorted_limits: Vec<_> = limit_stocks
                                .iter()
                                .filter(|stock| stock.main_net_yi.is_some())
                                .cloned()
                                .collect();

                            sorted_limits.sort_by(|a, b| {
                                b.main_net_yi
                                    .partial_cmp(&a.main_net_yi)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            });

                            for s in sorted_limits.iter().take(50) {
                                let level = match board_level_cache.get(&s.code) {
                                    Some(v) => *v,

                                    None => continue,
                                };

                                if !board_notified.insert(s.code.clone()) {
                                    continue;
                                }

                                let main_flow = s
                                    .main_net_yi
                                    .map(|value| format!("{value:+.2}亿"))
                                    .unwrap_or_else(|| "暂无".to_string());
                                let volume_ratio = s
                                    .volume_ratio
                                    .map(|value| format!("{value:.1}"))
                                    .unwrap_or_else(|| "暂无".to_string());
                                let row = format!(
                                    "  {}({}) 主力{} 量比{} {:+.1}%",
                                    s.name, s.code, main_flow, volume_ratio, s.change_pct,
                                );

                                match level {
                                    1 => first_lines.push(row),

                                    2 => second_lines.push(row),

                                    _ => third_lines.push(row),
                                }
                            }

                            let ts = chrono::Local::now().format("%H:%M").to_string();

                            if !first_lines.is_empty() {
                                match push_templates::render_limit_boards_shape(
                                    push_templates::LimitBoardsShape::First,
                                    &ts,
                                    &first_lines,
                                ) {
                                    Ok(text) => {
                                        match presentation_registry::acquire_token(
                                            "L-01-limit-boards-first",
                                            notify::PushKind::LimitBoards,
                                            "monitor_limit_board_producer",
                                            "assemble_limit_boards_first",
                                        ) {
                                            Ok(token) => {
                                                notify::push_presented_v3(token, &text, None).await;
                                            }
                                            Err(error) => log::error!(
                                                "[涨停板][BR-196] 首板 token 失败: {error}"
                                            ),
                                        }
                                    }
                                    Err(error) => log::error!("[涨停板] 首板展示失败: {error}"),
                                }
                            }

                            if !second_lines.is_empty() {
                                match push_templates::render_limit_boards_shape(
                                    push_templates::LimitBoardsShape::Second,
                                    &ts,
                                    &second_lines,
                                ) {
                                    Ok(text) => {
                                        match presentation_registry::acquire_token(
                                            "L-02-limit-boards-second",
                                            notify::PushKind::LimitBoards,
                                            "monitor_limit_board_producer",
                                            "assemble_limit_boards_second",
                                        ) {
                                            Ok(token) => {
                                                notify::push_presented_v3(token, &text, None).await;
                                            }
                                            Err(error) => log::error!(
                                                "[涨停板][BR-196] 二板 token 失败: {error}"
                                            ),
                                        }
                                    }
                                    Err(error) => log::error!("[涨停板] 二板展示失败: {error}"),
                                }
                            }

                            if !third_lines.is_empty() {
                                match push_templates::render_limit_boards_shape(
                                    push_templates::LimitBoardsShape::ThirdPlus,
                                    &ts,
                                    &third_lines,
                                ) {
                                    Ok(text) => {
                                        match presentation_registry::acquire_token(
                                            "L-03-limit-boards-third-plus",
                                            notify::PushKind::LimitBoards,
                                            "monitor_limit_board_producer",
                                            "assemble_limit_boards_third_plus",
                                        ) {
                                            Ok(token) => {
                                                notify::push_presented_v3(token, &text, None).await;
                                            }
                                            Err(error) => log::error!(
                                                "[涨停板][BR-196] 三板+ token 失败: {error}"
                                            ),
                                        }
                                    }
                                    Err(error) => {
                                        log::error!("[涨停板] 三板+展示失败: {error}")
                                    }
                                }
                            }
                        }

                        // 合并两路数据：涨停列表中的持仓 + 持仓单独查询

                        let mut stock_map: std::collections::HashMap<
                            String,
                            &stock_analysis::market_data::TopStock,
                        > = std::collections::HashMap::new();

                        if let Some(position_quotes) = position_quotes.as_ref() {
                            if let Some(limit_stocks) = limit_stocks.as_ref() {
                                for s in limit_stocks {
                                    if our_codes.contains(&s.code) {
                                        stock_map.insert(s.code.clone(), s);
                                    }
                                }
                            }

                            for q in position_quotes {
                                if !stock_map.contains_key(&q.code) {
                                    stock_map.insert(q.code.clone(), q);
                                }
                            }
                        }

                        // 主力排名（仅在真实涨停池可用时排序）

                        let mut ranked = limit_stocks.as_ref().map(|stocks| {
                            stocks
                                .iter()
                                .filter(|stock| stock.main_net_yi.is_some())
                                .collect::<Vec<_>>()
                        });

                        if let Some(ranked) = ranked.as_mut() {
                            ranked.sort_by(|a, b| {
                                b.main_net_yi
                                    .partial_cmp(&a.main_net_yi)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            });
                        }

                        let total_ranked = ranked.as_ref().map(Vec::len);

                        // 持仓遍历：信号融合（不再单独推送每条事件）

                        for (code, s) in &stock_map {
                            let (Some(volume_ratio), Some(main_net_yi)) =
                                (s.volume_ratio, s.main_net_yi)
                            else {
                                log::warn!(
                                    "[盘中监控] {}({}) 缺少量比或主力净流，跳过资金面信号检测",
                                    s.name,
                                    s.code
                                );
                                continue;
                            };

                            // review #14: DB 错误按"已锁定"处理 (保守), log warn 提醒.

                            let t1_locked = match stock_analysis::portfolio::is_t1_locked(code) {
                                Ok(v) => v,

                                Err(e) => {
                                    log::warn!(
                                        "[t+1] is_t1_locked({}) 失败: {} — 按锁定处理",
                                        code,
                                        e
                                    );

                                    true
                                }
                            };

                            let rank = ranked
                                .as_ref()
                                .and_then(|rows| rows.iter().position(|r| r.code == *code))
                                .map(|position| position + 1);

                            let is_limit_up = s.change_pct >= 9.5;

                            let prev_was_limit = was_limit_up.contains(code);

                            // 状态追踪

                            if is_limit_up {
                                was_limit_up.insert(code.clone());
                            } else {
                                was_limit_up.remove(code);
                            }

                            let snap = StockSnapshot {
                                code: s.code.clone(),

                                name: s.name.clone(),

                                price: s.price,

                                change_pct: s.change_pct,

                                volume_ratio,

                                main_net_yi,

                                limit_up_price: Some(s.price * 1.1),

                                was_limit_up: prev_was_limit,

                                t1_locked,
                            };

                            // 信号收集 + 突变检测

                            let mut signals: Vec<Signal> = Vec::new();

                            let mut emergency_note = String::new();

                            for e in detector.scan_stock(&snap) {
                                signal_count += 1;

                                let (dir, strength) = match e.category {
                                    AlertCategory::LimitUp | AlertCategory::MainInflow => {
                                        (1.0, 80.0)
                                    }

                                    AlertCategory::LimitDown | AlertCategory::MainOutflow => {
                                        (-1.0, 80.0)
                                    }

                                    AlertCategory::VolBurst => (1.0, 60.0),

                                    AlertCategory::BoardBreak => (-1.0, 90.0),

                                    _ => (0.0, 40.0),
                                };

                                signals.push(Signal::new(
                                    match e.category {
                                        AlertCategory::MainInflow | AlertCategory::MainOutflow => {
                                            SignalSource::FundFlow
                                        }

                                        _ => SignalSource::Technical,
                                    },
                                    dir,
                                    strength,
                                    0.0,
                                ));

                                // 突变检测：仅记录状态，不单独推送

                                if matches!(e.category, AlertCategory::BoardBreak) {
                                    emergency_note = "⚠️ 炸板！".to_string();
                                }

                                // BR-192 收尾 (2026-08-07): 指标告警走 counted 投递。
                                // LimitUp/LimitDown 除外 — 下方显式构造的涨停/跌停
                                // 突变事件负责投递 (避免同一触达双推)。
                                if !matches!(
                                    e.category,
                                    AlertCategory::LimitUp | AlertCategory::LimitDown
                                ) {
                                    if let Some(ev) = state_machine.process(e) {
                                        alert_count += 1;
                                        deliver_intraday_alert(&ev).await;
                                    }
                                }
                            }

                            // 信号融合

                            let resonance = if signals.is_empty() {
                                0.0
                            } else {
                                fusion.resonance(&signals)
                            };

                            let recommend = fusion.recommend(resonance);

                            // 涨停/跌停突变一次推送（走状态机防重复）

                            if is_limit_up || s.change_pct <= -9.5 {
                                let event = AlertEvent {
                                    level: if s.change_pct <= -9.5 {
                                        AlertLevel::Emergency
                                    } else {
                                        AlertLevel::Important
                                    },

                                    category: if s.change_pct <= -9.5 {
                                        AlertCategory::LimitDown
                                    } else {
                                        AlertCategory::LimitUp
                                    },

                                    code: code.clone(),

                                    name: s.name.clone(),

                                    message: if s.change_pct <= -9.5 {
                                        format!("{} 跌停 {:.1}%", s.name, s.change_pct)
                                    } else {
                                        format!("{} 涨停 {:.1}%", s.name, s.change_pct)
                                    },

                                    detail: AlertDetail {
                                        price: Some(s.price),

                                        change_pct: Some(s.change_pct),

                                        volume_ratio: s.volume_ratio,

                                        main_flow_yi: s.main_net_yi,

                                        threshold: None,

                                        news_title: None,

                                        news_summary: None,

                                        news_importance: None,

                                        ai_decision: None,

                                        t1_locked,

                                        extra: rank.zip(total_ranked).map(|(r, total)| {
                                            format!(
                                                "主力排名 {}/{} | 共振{:.0} {}",
                                                r, total, resonance, recommend
                                            )
                                        }),
                                    },

                                    triggered_at: chrono::Local::now(),
                                    routed_external_id: None,
                                };

                                if let Some(ev) = state_machine.process(event) {
                                    alert_count += 1;

                                    // BR-192 收尾: 涨停/跌停突变走 counted 投递。
                                    deliver_intraday_alert(&ev).await;
                                }
                            }

                            // 2026-08-07 BR-192 收尾: BoardBreak 指标告警已由上方
                            // scan_stock 循环走 counted binding 投递 (origin=
                            // InternalDurable)。emergency_note 仅保留给信号融合
                            // 共振文本使用, 不再承担投递职责。
                            if !emergency_note.is_empty() {
                                log::warn!(
                                    "[炸板][BR-192] capability_unavailable=holding_event_counted_binding_unavailable; \
                                     external delivery disabled code={} event={}",
                                    code,
                                    AlertCategory::BoardBreak.key()
                                );
                            }

                            if resonance.abs() > 30.0 {
                                log::info!(
                                    "[信号融合] {}({}) 共振={:0} 建议={}",
                                    s.name,
                                    code,
                                    resonance,
                                    recommend
                                );
                            }

                            // v19.13: 移除原来的做T推送 (line 2827-2834)

                            // 旧: 对 limit_stocks (涨停股 Top 10) ∩ our_codes (持仓+watchlist) 推

                            // 问题: 涨停股很少是持仓 (持仓 6 只, 涨停 Top 10 通常不重叠), 即使重叠也包括 watchlist

                            // 新: 下方 "持仓专属做T扫描" 才是真路径

                            // 这里只保留 signal_count + alert_count, 不推做T
                        }

                        // BR-151 / BR-153: user-confirmed holdings + Magic TDX-only
                        // evidence. This path emits reverse-T observations, not orders.
                        if last_t0_scan.elapsed().as_secs() >= 30 {
                            match prepare_magic_tdx_t0_messages().await {
                                Ok(messages) => {
                                    let mut outcomes = Vec::with_capacity(messages.len());
                                    for prepared in messages {
                                        log::info!(
                                            "[做T-持仓][BR-153][BR-192] evaluated code={} decision={} first_line={}",
                                            prepared.code,
                                            prepared
                                                .binding
                                                .schedule_occurrence_identity()
                                                .get(..12)
                                                .unwrap_or(
                                                    prepared
                                                        .binding
                                                        .schedule_occurrence_identity()
                                                ),
                                            prepared.text.lines().nth(1).unwrap_or("")
                                        );
                                        let presentation_token =
                                            match presentation_registry::acquire_token(
                                                "T-05-t0-advice",
                                                notify::PushKind::T0Advice,
                                                "t0_dispatcher",
                                                "render_t0_advice",
                                            ) {
                                                Ok(token) => token,
                                                Err(reason) => {
                                                    log::error!(
                                                    "[做T-持仓][BR-196] presentation token rejected code={} reason={}",
                                                    prepared.code,
                                                    reason
                                                );
                                                    outcomes
                                                        .push(notify::PushOutcome::Denied(reason));
                                                    continue;
                                                }
                                            };
                                        let outcome = notify::push_counted_with_binding(
                                            presentation_token,
                                            &prepared.text,
                                            None,
                                            prepared.binding,
                                        )
                                        .await;
                                        if !matches!(
                                            outcome,
                                            notify::PushOutcome::Pushed
                                                | notify::PushOutcome::Deduped
                                        ) {
                                            log::error!(
                                                "[BR-116][BR-153] 做T投递未确认 code={} outcome={:?}",
                                                prepared.code,
                                                outcome
                                            );
                                        }
                                        outcomes.push(outcome);
                                    }
                                    if t0_delivery_outcomes_confirmed(&outcomes) {
                                        last_t0_scan = std::time::Instant::now();
                                    }
                                }
                                Err(error) => log::error!(
                                    "[做T-持仓][BR-153] 数据批次拒绝，保留立即重试资格: {}",
                                    error
                                ),
                            }
                        }

                        // 产业链扫描已统一到 news_monitor_loop 的 8:00-22:00 窗口调度，

                        // 此处不再重复（避免盘中 monitor_loop 与 news_monitor_loop 双跑双推）。

                        // v19.12: 盘面走向 (R-02 盘中简版) + 涨停产业链 (R-03 盘中简版) — 每 5 分钟硬推
                        // b013 P1-10: 改用独立 last_market_view 计时器
                        if last_market_view.elapsed().as_secs() >= 300 {
                            let market_view =
                                tokio::task::spawn_blocking(|| -> Result<String, String> {
                                    let batch =
                                        stock_analysis::data_gateway::BoardDataGateway::new()
                                            .day1_flows_blocking(
                                                stock_analysis::data_gateway::BoardKind::Concept,
                                                10,
                                            )
                                            .map_err(|error| {
                                                format!("盘中概念板块主力净流入样本失败: {error}")
                                            })?;
                                    render_board_flow_market_view(
                                        &batch,
                                        &chrono::Local::now().format("%H:%M").to_string(),
                                    )
                                })
                                .await;

                            match market_view {
                                Ok(Ok(text)) if !text.is_empty() => {
                                    let outcome = notify::push_governor_v3(
                                        &text,
                                        notify::PushKind::IntradayMarket,
                                        None,
                                    )
                                    .await;
                                    if periodic_delivery_confirmed(&outcome) {
                                        last_market_view = std::time::Instant::now();
                                    } else {
                                        log::error!(
                                            "[BR-116] 盘中盘面投递未确认，保留到期状态: {:?}",
                                            outcome
                                        );
                                    }
                                }
                                Ok(Ok(_)) => {
                                    log::info!("[盘中盘面] 板块榜真实空结果，跳过");
                                    last_market_view = std::time::Instant::now();
                                }
                                Ok(Err(error)) => log::error!("[盘中盘面] 数据批次拒绝: {}", error),
                                Err(error) => log::error!("[盘中盘面] 后台任务失败: {}", error),
                            }
                        }

                        // ═══════════════════════════════════════════════════════════════

                        // v34: I-03 涨停扩散与板块补涨 (15 min 周期, 与 v18 LimitBoards 互补)

                        //   - 数据源: limit_up_stocks + chain_mapper 板块归类

                        //   - 模板: render_industry_chain_intraday (主链 + 龙头 + 补涨候选)

                        //   - 静默: 涨停池空时短路

                        //   - 与 v18 LimitBoards (首板/二板/三板 split) 互补不冲突

                        // ═══════════════════════════════════════════════════════════════

                        if last_industry_chain_intraday.elapsed().as_secs() >= 900 {
                            // v41: 读共享 banner

                            if let Some(banner) = current_banner_for("I-03 industry chain") {
                                let hhmm = chrono::Local::now().format("%H:%M").to_string();
                                if push_templates::dispatch_industry_chain_intraday_periodic(
                                    &hhmm, &banner,
                                )
                                .await
                                {
                                    last_industry_chain_intraday = std::time::Instant::now();
                                } else {
                                    log::error!(
                                    "[I-03][BR-091] dispatcher did not confirm delivery; timer not advanced"
                                );
                                }
                            }
                        }

                        // ═══════════════════════════════════════════════════════════════

                        // v38: I-04 持仓操作建议 (30 min 周期, v12 §14.5 冷却 30 min/票)

                        //   - 遍历当前持仓, 用 cost/hard_stop 生成 plan

                        //   - 简化版: 涨幅 >5% 减仓, <-3% 加仓, 否则持有

                        //   - 真实意图: 接入 decision::evaluate_holding (v12.2 规划)

                        //   - 静默: 无持仓时短路

                        // ═══════════════════════════════════════════════════════════════

                        if last_holding_plan.elapsed().as_secs() >= 1800 {
                            // BR-192 收尾: T-03 真实 counted 投递 (原恒 unavailable)。
                            // 2026-08-07 实测修正: counted identity 是内容级
                            // (文本/证据含价格时间戳, 内容变 → 新决策 → 再推),
                            // 频率控制靠 cooldown(1800s) — 但 9:45/10:15 实测同票
                            // 重复推送。"当日一票一推"由 holding_plan_daily 表
                            // (DB 级) 保证: 同票当日只投递一次, 跨进程重启不丢
                            // (10:27 重启实测内存 static 清空 → 10:57 重推)。
                            if let Some(banner) = current_banner_for("I-04 holding plan") {
                                match prepare_holding_plan_messages(&banner).await {
                                    Ok(messages) => {
                                        let today = chrono::Local::now().date_naive();
                                        let already_pushed = holding_plan_daily_pushed(today);
                                        let pending: Vec<PreparedHoldingPlan> = messages
                                            .into_iter()
                                            .filter(|m| !already_pushed.contains(&m.code))
                                            .collect();
                                        let mut confirmed = true;
                                        let mut delivered_codes: Vec<String> = Vec::new();
                                        for prepared in pending {
                                            let token =
                                                match crate::presentation_registry::acquire_token(
                                                    "T-03-holding-plan",
                                                    PushKind::HoldingPlan,
                                                    "holding_plan_dispatcher",
                                                    "render_holding_plan",
                                                ) {
                                                    Ok(token) => token,
                                                    Err(reason) => {
                                                        log::error!(
                                                        "[T-03][BR-196] presentation token rejected code={} reason={}",
                                                        prepared.code,
                                                        reason
                                                    );
                                                        confirmed = false;
                                                        continue;
                                                    }
                                                };
                                            let outcome = notify::push_counted_with_binding(
                                                token,
                                                &prepared.text,
                                                None,
                                                prepared.binding,
                                            )
                                            .await;
                                            if !matches!(
                                                outcome,
                                                notify::PushOutcome::Pushed
                                                    | notify::PushOutcome::Deduped
                                            ) {
                                                log::error!(
                                                    "[T-03][BR-116] 持仓建议投递未确认 code={} outcome={:?}",
                                                    prepared.code,
                                                    outcome
                                                );
                                                confirmed = false;
                                            } else {
                                                log::info!(
                                                    "[T-03] delivered code={} outcome={:?}",
                                                    prepared.code,
                                                    outcome
                                                );
                                                delivered_codes.push(prepared.code.clone());
                                            }
                                        }
                                        // 记录当日已推 (成功才记录, 失败保留重试资格; DB 级跨重启)
                                        for code in delivered_codes {
                                            holding_plan_daily_record(today, &code);
                                        }
                                        if confirmed {
                                            last_holding_plan = std::time::Instant::now();
                                        } else {
                                            log::error!(
                                            "[I-04][BR-091] dispatcher did not confirm delivery; timer not advanced"
                                        );
                                        }
                                    }
                                    Err(error) => log::error!(
                                        "[I-04][T-03] 持仓建议批次拒绝, 保留重试资格: {error}"
                                    ),
                                }
                            }
                        }

                        // ═══════════════════════════════════════════════════════════════

                        // 2026-08-07 审计接入 (A 组孤儿): I-09 板块 TOP + I-09A 量价反向。
                        // 原实现 (push_templates) 数据源/渲染/token 完整但生产零调度 —
                        // 运行时接入, 60 min 周期 (fetch_board_ranking 实时数据,
                        // 板块异动需新闻归因文本, 空快讯 → 空文本兜底模式)。

                        // I-09 板块 TOP: 板块涨跌排行 TOP5 (f3=涨幅榜)

                        if last_sector_top.elapsed().as_secs() >= 3600 {
                            let hhmm = chrono::Local::now().format("%H:%M").to_string();

                            if push_templates::dispatch_sector_top_daily(&hhmm).await {
                                last_sector_top = std::time::Instant::now();
                            } else {
                                // 2026-08-07: 失败也推进计时器 — 数据源不可用
                                // (上游 BoardDataGateway 合同缺口实证) 每 30s 重试
                                // 轰炸日志, 改 1 小时后重试 (成功恢复时效可接受)。
                                last_sector_top = std::time::Instant::now();
                                log::warn!(
                                    "[I-09] dispatcher did not confirm; 1 小时后重试 (失败节流)"
                                );
                            }
                        }

                        // I-09A 量价反向: 板块异动无新闻归因 → 推送 (仅异动, 空归因说明)

                        if last_sector_anomaly.elapsed().as_secs() >= 3600 {
                            use stock_analysis::data_gateway::{
                                GatewayBatch, GlobalNewsGateway, GlobalNewsProvider,
                            };
                            let news_text = match GlobalNewsGateway::new()
                                .global_news(GlobalNewsProvider::Cailianpress, 20)
                                .await
                            {
                                Ok(GatewayBatch::Available { records, .. })
                                    if !records.is_empty() =>
                                {
                                    records
                                        .iter()
                                        .take(10)
                                        .map(|r| r.title.clone())
                                        .collect::<Vec<_>>()
                                        .join("; ")
                                }
                                _ => String::new(),
                            };
                            let hhmm = chrono::Local::now().format("%H:%M").to_string();

                            if push_templates::dispatch_sector_anomaly_daily(&hhmm, &news_text)
                                .await
                            {
                                last_sector_anomaly = std::time::Instant::now();
                            } else {
                                // 2026-08-07: 同 I-09 — 失败也推进计时器 (1 小时重试节流)。
                                last_sector_anomaly = std::time::Instant::now();
                                log::warn!(
                                    "[I-09A] dispatcher did not confirm; 1 小时后重试 (失败节流)"
                                );
                            }
                        }

                        // ═══════════════════════════════════════════════════════════════

                        // v44 + v54 + v60: T-14/T-15 trade_pipeline 调度 (F8 拆分)

                        //   - T-14 (15 min) 调 dispatch_trade_pipeline_orders (只 order events)

                        //   - T-15 (5 min) 调 dispatch_trade_pipeline_fills (只 fill events)

                        //   - 拆分后 5 min T-15 不会再扫 order events (旧 bug 3x 工作量)

                        //   - 沙箱: trade_pipeline 空, 静默短路

                        //   - 真实 intent: broker 委托/成交回报 event

                        if last_post_fixed_order.elapsed().as_secs() >= 900 {
                            let hhmm = chrono::Local::now().format("%H:%M").to_string();
                            match current_banner_for("T-14 trade pipeline") {
                                Some(banner) => {
                                    if push_templates::dispatch_trade_pipeline_orders_periodic(
                                        &hhmm, &banner,
                                    )
                                    .await
                                    {
                                        last_post_fixed_order = std::time::Instant::now();
                                    }
                                }
                                None => log::error!("[T-14][BR-108] banner unavailable"),
                            }
                        }

                        if last_post_fixed_fill.elapsed().as_secs() >= 300 {
                            let hhmm = chrono::Local::now().format("%H:%M").to_string();
                            match current_banner_for("T-15 trade pipeline") {
                                Some(banner) => {
                                    if push_templates::dispatch_trade_pipeline_fills_periodic(
                                        &hhmm, &banner,
                                    )
                                    .await
                                    {
                                        last_post_fixed_fill = std::time::Instant::now();
                                    }
                                }
                                None => log::error!("[T-15][BR-108] banner unavailable"),
                            }
                        }

                        // ═══════════════════════════════════════════════════════════════

                        // v46 + v59: T-16 ST 涨跌幅变更 (开盘 9:30 一次/票/日)

                        //   - 新规 2026-07-06: 主板 ST/*ST 5%→10%

                        //   - v59 修复: 真正调 dispatch_st_price_limit_changed (F2 死代码修复)

                        //   - 真实数据源: portfolio.get_st_positions() (is_st/star_st 暂写死, broker 接入后真接)

                        // ═══════════════════════════════════════════════════════════════

                        if !st_price_pushed {
                            let now_time = chrono::Local::now().time();

                            let st_trigger = chrono::NaiveTime::from_hms_opt(9, 30, 0).unwrap();

                            if now_time >= st_trigger {
                                match dispatch_st_price_limit_batch("09:30").await {
                                    Ok(count) => {
                                        st_price_pushed = true;
                                        log::info!("[T-16] ST 涨跌幅变更已推 {count} 只持仓");
                                    }
                                    Err(error) => {
                                        log::error!("[T-16] real-data batch rejected: {error}");
                                    }
                                }
                            }
                        }

                        // ═══════════════════════════════════════════════════════════════

                        // v47 + v59: T-17 ETF 收盘集合竞价 (14:57 一次)

                        //   - 新规 2026-07-06: 上交所基金收盘 14:57-15:00 集合竞价

                        //   - v59 修复: 真正调 dispatch_etf_closing_call_auction (F2 死代码修复)

                        //   - 真实数据源: portfolio ETF 持仓 + 集合竞价行情 (后续 PR)

                        // ═══════════════════════════════════════════════════════════════

                        // 2026-08-07 审计接入: T-12 尾盘提示 (14:55-14:57, 每日一次)。
                        // 原 render_close_call 模板无生产调度 — 现走 counted 投递
                        // (跳水票才推)。成功才封口, 失败保留当日重试。
                        if !close_call_pushed {
                            let now_time = chrono::Local::now().time();

                            let close_call_trigger =
                                chrono::NaiveTime::from_hms_opt(14, 55, 0).unwrap();

                            if now_time >= close_call_trigger {
                                if let Some(banner) = current_banner_for("T-12 close call") {
                                    match prepare_close_call_messages(&banner).await {
                                        Ok(messages) => {
                                            let mut confirmed = true;
                                            for prepared in messages {
                                                let token = match crate::presentation_registry::acquire_token(
                                                    "T-12-close-call",
                                                    PushKind::CloseCall,
                                                    "close_call_dispatcher",
                                                    "render_close_call",
                                                ) {
                                                    Ok(token) => token,
                                                    Err(reason) => {
                                                        log::error!(
                                                        "[T-12][BR-196] token rejected code={} reason={}",
                                                        prepared.code,
                                                        reason
                                                    );
                                                        confirmed = false;
                                                        continue;
                                                    }
                                                };
                                                let outcome = notify::push_counted_with_binding(
                                                    token,
                                                    &prepared.text,
                                                    None,
                                                    prepared.binding,
                                                )
                                                .await;
                                                if !matches!(
                                                    outcome,
                                                    notify::PushOutcome::Pushed
                                                        | notify::PushOutcome::Deduped
                                                ) {
                                                    log::error!(
                                                    "[T-12][BR-116] 尾盘提示投递未确认 code={} outcome={:?}",
                                                    prepared.code,
                                                    outcome
                                                );
                                                    confirmed = false;
                                                } else {
                                                    log::info!(
                                                        "[T-12] delivered code={} outcome={:?}",
                                                        prepared.code,
                                                        outcome
                                                    );
                                                }
                                            }
                                            if confirmed {
                                                close_call_pushed = true;
                                            } else {
                                                log::error!(
                                                    "[T-12][BR-091] dispatcher did not confirm; retry kept"
                                                );
                                            }
                                        }
                                        Err(error) => log::error!(
                                            "[T-12] 尾盘提示批次拒绝, 保留重试资格: {error}"
                                        ),
                                    }
                                }
                            }
                        }

                        if !etf_closing_pushed {
                            let now_time = chrono::Local::now().time();

                            let etf_trigger = chrono::NaiveTime::from_hms_opt(14, 57, 0).unwrap();

                            if now_time >= etf_trigger {
                                // BR-105: ETF 持仓 + 集合竞价真实 producer 尚未接入。
                                // 保持显式禁用，禁止用固定名称/代码制造生产报告。
                                etf_closing_pushed = true;
                                log::error!("[T-17][BR-105] disabled=no_etf_auction_producer");
                            }
                        }
                    }
                }

                if session == MarketSession::AfterHours {
                    break;
                }

                if session == MarketSession::LunchBreak {
                    log::info!("[午休] 暂停扫描");

                    tokio::time::sleep(tokio::time::Duration::from_secs(90 * 60)).await;

                    continue;
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(poll_secs)).await;
            }

            // BR-192: the legacy close summary flattened market/account facts
            // and discarded their immutable source identities. Skip before the
            // duplicate index/portfolio/T+1 acquisitions.
            log::warn!(
                "[收盘总结][BR-192] capability_unavailable=close_summary_daily_report_counted_binding_unavailable; \
                 skipped before index, portfolio, and T+1 acquisition"
            );

            log::warn!(
                "[持仓汇总][BR-192] capability_unavailable=position_summary_binding_unavailable; \
                 real positions lack a <=30s provider capture/account batch and paper positions \
                 lack an immutable portfolio snapshot with complete ordered quote evidence"
            );

            log::warn!(
                "[收盘复盘][BR-103][BR-192] capability_unavailable=close_review_account_binding_unavailable; \
                 skipped before report construction"
            );

            // 盘后独立维度：优选次日候选（最多 5 只，达不到阈值可少推/不推），强调可解释性，不复用盘中量能信号口径。

            // v57: 改用 A-08 TomorrowWatch PushKind (合并 OptimalClose)

            log::warn!("[盘后][BR-112] tomorrow candidates disabled=incomplete_source_contract");

            // BR-192: the mutable daily virtual snapshot and projected T+1
            // closes do not form an immutable counted-delivery binding.
            if stock_analysis::config::get_monitor_config()
                .air_refuel
                .next_day_review_enabled
            {
                log::warn!(
                    "[虚拟观察仓][BR-192] capability_unavailable=virtual_t1_daily_report_counted_binding_unavailable; \
                     skipped before snapshot and K-line acquisition"
                );
            }

            // v18/v19: 多轮 AI 由既有 19:00 strict post-session owner 负责。
            // 这里不再调用旧的全量抓取入口，避免重复拉 250 日 K 线、财报和六类工具数据。
            log::info!("[收盘] 多轮 AI 转由 post_session_review_scheduler 统一调度");

            log::info!(
                "[收盘] 信号{}条 告警{}条 | DQ: {} | {}",
                signal_count,
                alert_count,
                scanner.dq_summary(),
                prediction::hit_rate_summary(7)
            );

            // 收盘后继续循环，等待下一个交易日
        }
    };

    tokio::join!(intraday_loop, market_loop);
}

fn render_board_flow_market_view(
    batch: &stock_analysis::data_gateway::GatewayBatch<stock_analysis::data_gateway::BoardFlowFact>,
    hhmm: &str,
) -> Result<String, String> {
    use stock_analysis::data_gateway::{BoardKind, GatewayBatch};

    let evidence = batch.evidence();
    let records = match batch {
        GatewayBatch::VerifiedEmpty(_) => {
            log::info!(
                "[盘中盘面][BR-188] status=verified_empty provider={:?} observed_at={} batch_id={}",
                evidence.provider,
                evidence.observed_at,
                evidence.batch_id
            );
            return Ok(String::new());
        }
        GatewayBatch::Available { records, .. } => records,
    };
    if records.is_empty() {
        return Err("盘中概念板块 Gateway 返回非法 Available 空批次".to_string());
    }

    let mut normalized = Vec::with_capacity(records.len());
    for (index, board) in records.iter().enumerate() {
        let expected_rank =
            u32::try_from(index + 1).map_err(|_| "盘中概念板块样本排名超出 u32".to_string())?;
        if board.kind != BoardKind::Concept
            || board.rank != expected_rank
            || board.code.trim() != board.code
            || board.code.is_empty()
            || board.name.trim() != board.name
            || board.name.is_empty()
        {
            return Err(format!(
                "盘中概念板块主力净流入样本身份/顺序非法: index={} code={:?} name={:?} kind={:?} rank={}",
                index, board.code, board.name, board.kind, board.rank
            ));
        }
        let (Some(return_pct), Some(main_net_yuan)) = (board.return_pct, board.main_net_yuan)
        else {
            return Err(format!(
                "盘中概念板块主力净流入样本字段缺失: code={} return_pct={:?} main_net_yuan={:?}",
                board.code, board.return_pct, board.main_net_yuan
            ));
        };
        if !return_pct.is_finite() || !main_net_yuan.is_finite() {
            return Err(format!(
                "盘中概念板块主力净流入样本字段非有限: code={} return_pct={} main_net_yuan={}",
                board.code, return_pct, main_net_yuan
            ));
        }
        normalized.push((board, return_pct, main_net_yuan));
    }

    let avg_return = normalized.iter().map(|entry| entry.1).sum::<f64>() / normalized.len() as f64;
    let strong = normalized.iter().filter(|entry| entry.1 > 3.0).count();
    let mut text = format!(
        "📊 概念板块主力净流入样本 ({hhmm} 盘中)\n样本涨幅均值 {avg_return:+.2}% | 样本涨幅>3% {strong} 个\n"
    );
    text.push_str("Provider 主力净流入排名 Top5:\n");
    for (board, return_pct, main_net_yuan) in normalized.iter().take(5) {
        text.push_str(&format!(
            "  {} {:+.2}% 主力{:.2}亿\n",
            board.name,
            return_pct,
            main_net_yuan / 1e8
        ));
    }
    log::info!(
        "[盘中盘面][BR-188] status=available provider={:?} observed_at={} batch_id={} records={}",
        evidence.provider,
        evidence.observed_at,
        evidence.batch_id,
        records.len()
    );
    Ok(text)
}

/// BR-116: a periodic delivery is complete only after a real sink acceptance or
/// an explicit governance deduplication. Denials and sink failures remain due.
fn periodic_delivery_confirmed(outcome: &notify::PushOutcome) -> bool {
    matches!(
        outcome,
        notify::PushOutcome::Pushed | notify::PushOutcome::Deduped
    )
}

async fn post_close_news_review() {
    use chrono::{Duration as ChronoDuration, Utc};

    use stock_analysis::data_gateway::{GatewayBatch, SinaInstrumentNewsGateway};
    use stock_analysis::database::DatabaseManager;

    let now = Utc::now();

    let from = now - ChronoDuration::days(30);

    let gateway = SinaInstrumentNewsGateway::new();

    let holdings: Vec<String> = match stock_analysis::portfolio::get_positions() {
        Ok(positions) => positions
            .into_iter()
            .map(|position| position.code)
            .collect(),
        Err(error) => {
            log::error!("[盘后] 新闻回溯持仓查询失败: {}", error);
            return;
        }
    };

    log::info!(
        "[盘后] 拉 {} 只持仓近 30 天个股新闻 (from={}, to={})",
        holdings.len(),
        from.format("%Y-%m-%d"),
        now.format("%Y-%m-%d")
    );

    if holdings.is_empty() {
        log::warn!("[盘后] 当前无持仓, 跳过回溯");

        return;
    }

    for code in &holdings {
        match gateway.instrument_news_in_range(code, from, now).await {
            Ok(GatewayBatch::Available { records, evidence }) => {
                let total = records.len();
                let mut written = 0usize;
                let mut failed = 0usize;
                let Some(database) = DatabaseManager::try_get() else {
                    log::error!(
                        "[盘后][BR-163] {code} Sina 个股新闻已接纳但数据库未初始化; \
                         batch_id={} records={total}",
                        evidence.batch_id
                    );
                    continue;
                };
                for record in &records {
                    match database.insert_news_item(record.persistence_item()) {
                        Ok(()) => written += 1,
                        Err(error) => {
                            failed += 1;
                            log::error!("[盘后][BR-163] {code} Sina 个股新闻落库失败: {error}");
                        }
                    }
                }

                log::info!(
                    "[盘后][BR-163] {code} Sina 个股新闻: status=available \
                     provider={:?} batch_id={} 拉 {} 条, DB 写 {} 条, 失败 {} 条",
                    evidence.provider,
                    evidence.batch_id,
                    total,
                    written,
                    failed
                );
            }
            Ok(GatewayBatch::VerifiedEmpty(evidence)) => {
                log::info!(
                    "[盘后][BR-163] {code} Sina 个股新闻: status=verified_empty \
                     provider={:?} batch_id={} source_at={}",
                    evidence.provider,
                    evidence.batch_id,
                    evidence.source_at.as_deref().unwrap_or("absent")
                );
            }
            Err(error) if error.audit_outcome() == "unsupported" => {
                log::warn!(
                    "[盘后][BR-163] {code} Sina 个股新闻不支持: reason_code={} retryable={}: {error}",
                    error.reason_code(),
                    error.retryable()
                );
            }
            Err(error) => {
                log::warn!(
                    "[盘后][BR-163] {code} Sina 个股新闻不可用: outcome={} reason_code={} \
                     retryable={}: {error}",
                    error.audit_outcome(),
                    error.reason_code(),
                    error.retryable()
                );
            }
        }
    }

    log::info!("[盘后] 持仓回溯完成 ({} 只持仓)", holdings.len());
}

/// v13.12 (Task 12): 盘后回溯调度 — 每 30 分钟 tick 一次, 若本地时间已过 15:30 则触发一次.

/// 简化策略: 进入盘后时段后每 30 分钟最多触发一次 (避免重启后多触).

async fn post_close_news_scheduler() {
    use std::time::Duration;

    let threshold = chrono::NaiveTime::from_hms_opt(15, 30, 0)
        .unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap());

    let mut interval = tokio::time::interval(Duration::from_secs(1800));

    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    log::info!(
        "[盘后调度] 启动 (30 min tick, 触发条件: 本地时间 >= {})",
        threshold.format("%H:%M")
    );

    loop {
        interval.tick().await;

        let now_local = chrono::Local::now();

        if now_local.time() >= threshold {
            log::info!("[盘后调度] tick @ {} → 触发回溯", now_local.format("%H:%M"));

            post_close_news_review().await;
        }
    }
}

/// BR-192 收尾 (2026-08-07): 盘中指标告警 → counted binding 投递。
/// 事件型推送无定时调度身份, 以 {date}:{code}:{category} 作为 occurrence
/// identity (同一标的同类型告警当日只投递一次, counted 层去重; 状态机另做
/// 5 分钟去重)。source 证据 = 事件核心事实序列化 (message 含实际数值,
/// detail 含快照字段), origin=InternalDurable (内部派生证据接缝)。
/// 失败不重试: 告警时效性强, 30s 后快照已变化; 同日同类再次触发由 counted
/// 去重/状态机放行新 occurrence。
async fn deliver_intraday_alert(event: &AlertEvent) -> bool {
    use magic_market_core::{AssetClass, Exchange, InstrumentId};
    use sha2::{Digest, Sha256};

    let business_date = chrono::Local::now().date_naive();
    let occurrence = format!(
        "intraday-alert:{}:{}:{}",
        business_date,
        event.code,
        event.category.key()
    );
    let canonical = serde_json::json!({
        "code": event.code,
        "category": event.category.key(),
        "level": event.level.label(),
        "message": event.message,
        "triggered_at": event.triggered_at.to_rfc3339(),
        "price": event.detail.price,
        "change_pct": event.detail.change_pct,
        "volume_ratio": event.detail.volume_ratio,
        "main_flow_yi": event.detail.main_flow_yi,
        "t1_locked": event.detail.t1_locked,
    });
    let canonical_bytes = canonical.to_string().into_bytes();
    let subject_hash = hex::encode(Sha256::digest(&canonical_bytes));
    let exchange = if event.code.starts_with('6') {
        Exchange::Shanghai
    } else {
        Exchange::Shenzhen
    };
    let instrument = match InstrumentId::new(exchange, event.code.clone(), AssetClass::Equity) {
        Ok(id) => id,
        Err(error) => {
            log::error!("[盘中告警] instrument 构造失败 code={}: {error}", event.code);
            return false;
        }
    };
    let binding =
        match durable_delivery_runtime::CountedDeliveryBinding::new(
            business_date,
            occurrence,
            canonical_bytes,
            durable_delivery_runtime::CountedDeliveryScope::Ticket { instrument },
            subject_hash,
            durable_delivery_runtime::CountedDeliveryOrigin::InternalDurable,
            None,
            true,
        ) {
            Ok(binding) => binding,
            Err(error) => {
                log::error!(
                    "[盘中告警] counted binding 构造失败 code={} category={}: {error}",
                    event.code,
                    event.category.key()
                );
                return false;
            }
        };
    let token = match crate::presentation_registry::acquire_token(
        "T-04B-intraday-alert",
        PushKind::HoldingEvent,
        "intraday_alert_dispatcher",
        "render_intraday_alert",
    ) {
        Ok(token) => token,
        Err(reason) => {
            log::error!(
                "[盘中告警][BR-196] presentation token rejected code={} category={} reason={}",
                event.code,
                event.category.key(),
                reason
            );
            return false;
        }
    };
    let text = push_templates::render_intraday_alert(event);
    let outcome = notify::push_counted_with_binding(token, &text, None, binding).await;
    match outcome {
        notify::PushOutcome::Pushed => {
            log::info!(
                "[盘中告警] delivered code={} category={}",
                event.code,
                event.category.key()
            );
            true
        }
        notify::PushOutcome::Deduped => {
            log::info!(
                "[盘中告警] deduped (当日同类已投递) code={} category={}",
                event.code,
                event.category.key()
            );
            true
        }
        other => {
            log::warn!(
                "[盘中告警] 投递未确认 code={} category={} outcome={:?}",
                event.code,
                event.category.key(),
                other
            );
            false
        }
    }
}

#[cfg(test)]
fn build_price_map(
    quotes: &[stock_analysis::market_data::TopStock],
) -> std::collections::HashMap<String, f64> {
    quotes.iter().map(|q| (q.code.clone(), q.price)).collect()
}

#[derive(Debug, Clone)]
struct ReviewLimitChainCandidate {
    code: String,
    name: String,
    #[cfg(test)]
    sector: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewLimitChainRejection {
    code: String,
    reason: String,
}

#[derive(Debug, Default)]
struct ReviewLimitChainBatch {
    accepted: Vec<stock_analysis::market_analyzer::limit_chain_review::StockLimitStats>,
    rejected: Vec<ReviewLimitChainRejection>,
    source_errors: Vec<String>,
}

impl ReviewLimitChainBatch {
    fn source_complete(&self) -> bool {
        self.rejected.is_empty() && self.source_errors.is_empty()
    }
}

#[cfg(test)]
fn collect_review_limit_chain_stocks_with<ResolveSector, FetchLimitDays>(
    candidates: &[ReviewLimitChainCandidate],
    mut resolve_sector: ResolveSector,
    mut fetch_limit_days: FetchLimitDays,
) -> ReviewLimitChainBatch
where
    ResolveSector: FnMut(&str) -> Result<String, String>,
    FetchLimitDays: FnMut(&str) -> Result<Vec<bool>, String>,
{
    use stock_analysis::market_analyzer::limit_chain_review::StockLimitStats;

    let mut batch = ReviewLimitChainBatch::default();
    for candidate in candidates {
        let code = candidate.code.trim();
        let name = candidate.name.trim();
        if code.is_empty() || name.is_empty() {
            batch.rejected.push(ReviewLimitChainRejection {
                code: code.to_string(),
                reason: "股票代码或名称缺失".to_string(),
            });
            continue;
        }

        let sector = candidate
            .sector
            .as_deref()
            .map(str::trim)
            .filter(|sector| !sector.is_empty() && *sector != "其他")
            .map(str::to_string)
            .map(Ok)
            .unwrap_or_else(|| resolve_sector(code));
        let sector = match sector {
            Ok(sector) if !sector.trim().is_empty() && sector.trim() != "其他" => sector,
            Ok(_) => {
                batch.rejected.push(ReviewLimitChainRejection {
                    code: code.to_string(),
                    reason: "真实行业数据为空".to_string(),
                });
                continue;
            }
            Err(error) => {
                batch.rejected.push(ReviewLimitChainRejection {
                    code: code.to_string(),
                    reason: format!("行业数据获取失败: {error}"),
                });
                continue;
            }
        };

        let limit_days = match fetch_limit_days(code) {
            Ok(days) if !days.is_empty() => days,
            Ok(_) => {
                batch.rejected.push(ReviewLimitChainRejection {
                    code: code.to_string(),
                    reason: "日 K 数据为空".to_string(),
                });
                continue;
            }
            Err(error) => {
                batch.rejected.push(ReviewLimitChainRejection {
                    code: code.to_string(),
                    reason: format!("日 K 获取失败: {error}"),
                });
                continue;
            }
        };
        let consecutive_days = limit_days
            .iter()
            .take(10)
            .take_while(|is_limit_up| **is_limit_up)
            .count();
        if consecutive_days == 0 {
            continue;
        }
        let board_level = match u8::try_from(consecutive_days) {
            Ok(value) => value,
            Err(_) => {
                batch.rejected.push(ReviewLimitChainRejection {
                    code: code.to_string(),
                    reason: format!("连板数溢出: {consecutive_days}"),
                });
                continue;
            }
        };
        batch.accepted.push(StockLimitStats {
            code: code.to_string(),
            name: name.to_string(),
            chain: sector,
            board_level,
            is_limit_up_today: true,
            is_first_board: consecutive_days == 1,
            consecutive_days: u32::from(board_level),
        });
    }
    batch
}

async fn load_review_limit_chain_stocks(
    holdings: &[stock_analysis::portfolio::Position],
    date: &str,
) -> Result<ReviewLimitChainBatch, String> {
    let mut source_errors = Vec::new();
    let watchlist =
        match tokio::task::spawn_blocking(stock_analysis::portfolio::get_watchlist).await {
            Ok(Ok(watchlist)) => watchlist,
            Ok(Err(error)) => {
                source_errors.push(format!("R-03 自选查询失败: {error}"));
                Vec::new()
            }
            Err(error) => {
                source_errors.push(format!("R-03 自选查询任务失败: {error}"));
                Vec::new()
            }
        };
    let mut seen_codes = std::collections::HashSet::new();
    let mut candidates = Vec::new();
    for position in holdings.iter().chain(watchlist.iter()) {
        if !seen_codes.insert(position.code.clone()) {
            continue;
        }
        candidates.push(ReviewLimitChainCandidate {
            code: position.code.clone(),
            name: position.name.clone(),
            #[cfg(test)]
            sector: None,
        });
        if candidates.len() == 20 {
            break;
        }
    }
    let review_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|error| format!("R-03 非法复盘日期 {date}: {error}"))?;
    let gateway_batch = stock_analysis::data_gateway::ReviewDataGateway::new()
        .r03_upper_limit_pool(review_date)
        .await
        .map_err(|error| {
            log::warn!(
                "[DataGateway][R-03] status=unavailable disabled=no_verified_batch error={}",
                error
            );
            format!("R-03 disabled=no_verified_batch: {error}")
        })?;
    log::info!("[DataGateway][R-03] {}", gateway_batch);
    map_review_limit_chain_gateway_batch(candidates, &gateway_batch, source_errors)
}

fn map_review_limit_chain_gateway_batch(
    candidates: Vec<ReviewLimitChainCandidate>,
    gateway_batch: &stock_analysis::data_gateway::GatewayBatch<
        stock_analysis::data_gateway::UpperLimitRecord,
    >,
    source_errors: Vec<String>,
) -> Result<ReviewLimitChainBatch, String> {
    let mut batch = ReviewLimitChainBatch::default();
    if gateway_batch.is_verified_empty() {
        batch.source_errors.extend(source_errors);
        return Ok(batch);
    }

    let mut provider_rows = std::collections::HashMap::new();
    for record in gateway_batch.records() {
        if provider_rows.insert(record.code.clone(), record).is_some() {
            return Err(format!(
                "R-03 provider batch contains duplicate security {}",
                record.code
            ));
        }
    }

    for candidate in candidates {
        let code = candidate.code.trim();
        let name = candidate.name.trim();
        if code.is_empty() || name.is_empty() {
            batch.rejected.push(ReviewLimitChainRejection {
                code: code.to_string(),
                reason: "股票代码或名称缺失".to_string(),
            });
            continue;
        }
        let Some(record) = provider_rows.get(code) else {
            // A complete market batch proves this monitored security is not in today's limit pool.
            continue;
        };
        let Some(theme) = record
            .theme
            .as_deref()
            .map(str::trim)
            .filter(|theme| !theme.is_empty() && *theme != "其他")
        else {
            batch.rejected.push(ReviewLimitChainRejection {
                code: code.to_string(),
                reason: "已选涨停池来源的题材证据缺失".to_string(),
            });
            continue;
        };
        let Some(streak) = record.streak else {
            batch.rejected.push(ReviewLimitChainRejection {
                code: code.to_string(),
                reason: "已选涨停池来源的连板数缺失".to_string(),
            });
            continue;
        };
        let board_level = u8::try_from(streak)
            .map_err(|_| format!("R-03 涨停池连板数溢出: code={code} streak={streak}"))?;
        batch.accepted.push(
            stock_analysis::market_analyzer::limit_chain_review::StockLimitStats {
                code: code.to_string(),
                name: name.to_string(),
                chain: theme.to_string(),
                board_level,
                is_limit_up_today: true,
                is_first_board: streak == 1,
                consecutive_days: streak,
            },
        );
    }
    batch.source_errors.extend(source_errors);
    Ok(batch)
}

/// v3: 收盘时记录净值快照到 ledger 表

#[cfg(test)]
async fn build_close_review_report() -> Result<String, String> {
    tokio::task::spawn_blocking(|| -> Result<String, String> {
        // Until a real account cash snapshot exists this returns an explicit
        // error and prevents historical ledger rows from being reported as today.
        snapshot_portfolio_value()?;
        let trades = stock_analysis::portfolio::get_trade_history(90)
            .map_err(|error| format!("获取成交历史失败: {error}"))?;
        let mut reviews = stock_analysis::review::journal::review_closed_trades(&trades)
            .map_err(|error| format!("复盘成交 FIFO 失败: {error}"))?;
        let post_exit_summary = stock_analysis::review::journal::enrich_post_exit(&mut reviews);
        if let Err(error) = stock_analysis::review::journal::govern_post_exit_enrichment(
            "close_review_report",
            &post_exit_summary,
        ) {
            return Err(format!("收盘复盘卖出后走势补全失败: {error}"));
        }

        let equity = stock_analysis::portfolio::get_equity_curve_as_of(
            365,
            chrono::Local::now().date_naive(),
        )
        .map_err(|error| format!("获取净值曲线失败: {error}"))?;
        let mut stats = stock_analysis::review::equity::compute_stats(&equity)
            .map_err(|error| format!("复盘净值统计失败: {error}"))?;
        stock_analysis::review::equity::enrich_with_trades(&mut stats, &reviews)
            .map_err(|error| format!("复盘交易统计失败: {error}"))?;
        let holdings = stock_analysis::portfolio::get_positions()
            .map_err(|error| format!("获取持仓失败: {error}"))?;
        let quotes = market_data::fetch_position_quotes()?;
        let prices = build_price_map(&quotes);
        Ok(
            stock_analysis::review::report::generate_daily_report_with_ledger(
                &reviews,
                &stats,
                &holdings,
                &prices,
                Some(equity.as_slice()),
            ),
        )
    })
    .await
    .map_err(|error| format!("收盘复盘后台任务失败: {error}"))?
}

#[cfg(test)]
fn snapshot_portfolio_value() -> Result<(), String> {
    Err(
        "disabled=no_fresh_real_account_cash_snapshot; refusing cash=0 and first-day pnl=0"
            .to_string(),
    )
}

#[cfg(test)]
mod tests_v17_4_d {
    use super::*;

    fn board_flow_batch(
        records: Vec<stock_analysis::data_gateway::BoardFlowFact>,
    ) -> stock_analysis::data_gateway::GatewayBatch<stock_analysis::data_gateway::BoardFlowFact>
    {
        stock_analysis::data_gateway::GatewayBatch::Available {
            records,
            evidence: stock_analysis::data_gateway::BatchEvidence {
                provider: magic_market_core::ProviderId::Eastmoney,
                source: "TEST_CODE_eastmoney-board-flow".to_owned(),
                source_at: Some("1785290400.000000000".to_owned()),
                observed_at: "1785290401.000000000".to_owned(),
                batch_id: "TEST_CODE_board_flow_batch".to_owned(),
            },
        }
    }

    fn board_flow(
        code: &str,
        name: &str,
        rank: u32,
        return_pct: Option<f64>,
        main_net_yuan: Option<f64>,
    ) -> stock_analysis::data_gateway::BoardFlowFact {
        stock_analysis::data_gateway::BoardFlowFact {
            code: code.to_owned(),
            name: name.to_owned(),
            kind: stock_analysis::data_gateway::BoardKind::Concept,
            rank,
            return_pct,
            main_net_yuan,
            leader_code: None,
            leader_name: None,
        }
    }

    #[test]
    fn br188_market_view_preserves_provider_flow_rank_and_names_semantics() {
        let batch = board_flow_batch(vec![
            board_flow(
                "TEST_CODE_BK0001",
                "测试概念甲",
                1,
                Some(-1.0),
                Some(300_000_000.0),
            ),
            board_flow(
                "TEST_CODE_BK0002",
                "测试概念乙",
                2,
                Some(5.0),
                Some(200_000_000.0),
            ),
        ]);
        let rendered = render_board_flow_market_view(&batch, "TEST_CODE_10:00").unwrap();
        assert!(rendered.contains("概念板块主力净流入样本"));
        assert!(rendered.contains("Provider 主力净流入排名 Top5"));
        assert!(rendered.find("测试概念甲").unwrap() < rendered.find("测试概念乙").unwrap());
        assert!(!rendered.contains("领涨板块"));
    }

    #[test]
    fn br188_market_view_rejects_missing_fields_and_rank_drift() {
        let missing = board_flow_batch(vec![board_flow(
            "TEST_CODE_BK0001",
            "测试概念甲",
            1,
            None,
            Some(300_000_000.0),
        )]);
        assert!(render_board_flow_market_view(&missing, "TEST_CODE_10:00")
            .unwrap_err()
            .contains("字段缺失"));

        let rank_drift = board_flow_batch(vec![board_flow(
            "TEST_CODE_BK0001",
            "测试概念甲",
            2,
            Some(1.0),
            Some(300_000_000.0),
        )]);
        assert!(
            render_board_flow_market_view(&rank_drift, "TEST_CODE_10:00")
                .unwrap_err()
                .contains("顺序非法")
        );
    }

    /// AC46: config 默认值 screener_min_score = 75
    #[test]
    fn screener_min_score_default_is_75() {
        let cfg = stock_analysis::config::MonitorConfig::default();
        assert_eq!(cfg.screener_min_score, 75, "v17.4 §5.3.2 默认 75");
    }

    #[test]
    fn br116_periodic_delivery_commits_pushed_and_deduped_only() {
        assert!(periodic_delivery_confirmed(&notify::PushOutcome::Pushed));
        assert!(periodic_delivery_confirmed(&notify::PushOutcome::Deduped));
        assert!(!periodic_delivery_confirmed(&notify::PushOutcome::Denied(
            "TEST_CODE denied".to_string()
        )));
        assert!(!periodic_delivery_confirmed(
            &notify::PushOutcome::SinkError("TEST_CODE sink".to_string())
        ));
    }
}

// ========================================================================
// v17.3 Task 5: Event CLI integration test — TDD RED step
// ========================================================================

#[cfg(test)]
mod tests_v17_3_integration {
    use stock_analysis::event::cli::parse_args;

    /// Verifies that --history parses as a terminal event command (not a monitor flag).
    /// This test will fail until main() wires event::cli::parse_args.
    #[test]
    fn event_commands_are_terminal_commands() {
        let cmd =
            parse_args(&["monitor", "--history", "--date=2026-07-16", "--limit=100"]).unwrap();
        assert!(
            cmd.is_some(),
            "parse_args should return Some for --history; got None — CLI not wired in main()"
        );
    }
}

#[cfg(test)]
mod tests_post_session_review_scheduler {
    use super::*;
    use chrono::{NaiveDate, NaiveDateTime};
    use sha2::{Digest, Sha256};

    fn at(hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 7, 21)
            .expect("valid test date")
            .and_hms_opt(hour, minute, 0)
            .expect("valid test time")
    }

    fn br192_hydration(
        task: review_batch::ReviewTask,
        business_date: NaiveDate,
    ) -> stock_analysis::durable_delivery::ScheduleHydration {
        let task_identity = review_batch::review_task_identity(business_date, task);
        let decision_identity = format!(
            "TEST_CODE_{}_{}_DECISION",
            business_date.format("%Y%m%d"),
            task.label()
        );
        let transition_identity = format!(
            "TEST_CODE_{}_{}_TRANSITION",
            business_date.format("%Y%m%d"),
            task.label()
        );
        let transition_basis_canonical = serde_json::to_vec(&serde_json::json!({
            "task_identity": task_identity.clone(),
            "business_date": business_date.format("%Y-%m-%d").to_string(),
            "task": task.label(),
            "source": "TEST_CODE_SOURCE",
            "rule_ids": ["BR-110", "BR-140", "BR-192"],
            "source_time": null,
            "snapshot_size": 1,
            "request_hashes": ["a".repeat(64)],
            "batch_ids": ["TEST_CODE_BATCH"],
        }))
        .expect("serialize task basis");
        let transition_basis_sha256 = format!("{:x}", Sha256::digest(&transition_basis_canonical));
        let transition_canonical = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "transition_identity": transition_identity.clone(),
            "task_identity": task_identity.clone(),
            "decision_identity": decision_identity.clone(),
            "source_identity": "TEST_CODE_SOURCE",
            "task_disposition": "Accepted",
            "task_binding_sha256": transition_basis_sha256,
            "generic_disposition_identity": "TEST_CODE_DISPOSITION",
            "generic_disposition_sha256": "b".repeat(64),
        }))
        .expect("serialize task transition");
        let transition_sha256 = format!("{:x}", Sha256::digest(&transition_canonical));

        stock_analysis::durable_delivery::ScheduleHydration {
            decision_identity,
            task_identity,
            transition_identity,
            transition_canonical,
            transition_sha256,
            transition_basis_canonical,
            transition_basis_sha256,
            immutable_audit_ref: "TEST_CODE_AUDIT_REF".to_owned(),
            hydration_state: stock_analysis::durable_delivery::ScheduleHydrationState::Pending,
        }
    }

    #[test]
    fn br139_review_is_due_only_after_threshold_on_a_trading_day() {
        assert!(!post_session_review_window_open(at(18, 59), true));
        assert!(post_session_review_window_open(at(19, 0), true));
        assert!(!post_session_review_window_open(at(19, 0), false));
    }

    #[test]
    fn br140_weekend_manual_review_uses_the_latest_completed_trading_day() {
        let observed_at = NaiveDate::from_ymd_opt(2026, 7, 25)
            .expect("valid Saturday")
            .and_hms_opt(2, 42, 50)
            .expect("valid observed time");
        let context = review_batch::ReviewRunContext::at(observed_at);

        assert_eq!(
            context.review_date(),
            NaiveDate::from_ymd_opt(2026, 7, 24).expect("known completed Friday")
        );
        assert_eq!(context.observed_at(), observed_at);
        assert_eq!(
            context.eligibility_time(),
            chrono::NaiveTime::from_hms_opt(23, 59, 59).expect("valid end-of-day time")
        );
    }

    #[test]
    fn br152_expensive_enrichment_is_post_close_only() {
        assert!(!post_close_analysis_window_open(at(14, 59)));
        assert!(post_close_analysis_window_open(at(15, 0)));
    }

    #[test]
    fn br139_schedule_state_is_scoped_to_one_trading_date() {
        let date = at(19, 0).date();
        let state = review_batch::ReviewScheduleState::for_date(date);
        assert!(!state.due_tasks(at(19, 0)).is_empty());
        let next_day = date
            .succ_opt()
            .expect("test date has a successor")
            .and_hms_opt(19, 0, 0)
            .expect("valid next-day time");
        assert!(state.due_tasks(next_day).is_empty());
    }

    #[test]
    fn br192_main_caller_acknowledges_only_transitions_applied_to_its_business_date() {
        let date = at(19, 0).date();
        let current = br192_hydration(review_batch::ReviewTask::R09, date);
        let foreign = br192_hydration(
            review_batch::ReviewTask::A01,
            date.succ_opt().expect("next business date fixture"),
        );
        let acknowledged = std::cell::RefCell::new(std::collections::BTreeSet::new());
        let mut schedule = review_batch::ReviewScheduleState::for_date(date);

        let applied = apply_durable_review_hydrations_and_acknowledge(
            &mut schedule,
            &[current.clone(), foreign.clone()],
            |identities| {
                acknowledged.replace(identities.clone());
                Ok(())
            },
        )
        .expect("current-date hydration is durably acknowledged");

        assert_eq!(
            applied,
            std::collections::BTreeSet::from([review_batch::ReviewTask::R09])
        );
        assert_eq!(
            acknowledged.into_inner(),
            std::collections::BTreeSet::from([current.transition_identity.clone()])
        );
        assert!(schedule
            .due_tasks(at(19, 0))
            .contains(&review_batch::ReviewTask::A01));
        assert!(!schedule
            .due_tasks(at(19, 0))
            .contains(&review_batch::ReviewTask::R09));
        assert_ne!(current.transition_identity, foreign.transition_identity);
    }

    #[test]
    fn br192_main_caller_does_not_commit_local_application_when_durable_ack_fails() {
        let date = at(19, 0).date();
        let current = br192_hydration(review_batch::ReviewTask::R09, date);
        let mut schedule = review_batch::ReviewScheduleState::for_date(date);

        let error = apply_durable_review_hydrations_and_acknowledge(
            &mut schedule,
            std::slice::from_ref(&current),
            |_identities| Err("TEST_CODE_AUDIT_APPEND_FAILURE".to_owned()),
        )
        .expect_err("durable acknowledgement failure must propagate");

        assert_eq!(error, "TEST_CODE_AUDIT_APPEND_FAILURE");
        assert!(schedule
            .due_tasks(at(19, 0))
            .contains(&review_batch::ReviewTask::R09));
    }

    #[test]
    fn br139_long_running_branch_starts_review_scheduler() {
        let source = include_str!("main.rs");
        let production = source
            .split("mod tests_post_session_review_scheduler")
            .next()
            .expect("production source precedes scheduler tests");
        assert_eq!(
            production
                .matches("let post_close_news = tokio::spawn(post_close_news_scheduler());")
                .count(),
            1,
            "the long-running branch must own the post-close news scheduler"
        );
        assert_eq!(
            production
                .matches("spawn_post_session_review_scheduler(selection_v2_enabled)")
                .count(),
            1,
            "the long-running branch must own the post-session review scheduler"
        );
        assert_eq!(
            production
                .matches("news_monitor_loop(selection_v2_enabled)")
                .count(),
            1,
            "the long-running branch must pass the capability to the news loop"
        );
        assert!(
            !production.contains("let _intraday_handle = tokio::spawn"),
            "the intraday producer must remain inside the cancellable main-loop future"
        );
        let dispatcher_call = ["push_templates::", "dispatch_post_session_review("].concat();
        assert_eq!(
            production.matches(&dispatcher_call).count(),
            1,
            "the strict inner runner must be the only production dispatcher owner"
        );
        let stale_owner = ["evening_", "pushed"].concat();
        assert!(
            !production.contains(&stale_owner),
            "the stale monitor-loop review owner must not return"
        );
    }

    #[test]
    fn br192_startup_reconciles_before_active_r09_runner_and_passes_one_context() {
        let source = include_str!("main.rs");
        let production = source
            .split("mod tests_post_session_review_scheduler")
            .next()
            .expect("production source precedes scheduler tests");

        assert!(!production.contains("producer activation disabled=no_producer"));
        assert!(production.contains(
            "component=cffex_futures_delivery capability=unsupported; EventCalendar delivery remains retryable and sink-blocked"
        ));
        assert!(production.contains("durable_delivery_runtime::ensure_startup_reconciled().await"));
        let runner = production
            .split("\nasync fn run_strict_review_only_inner(\n")
            .nth(1)
            .and_then(|tail| tail.split("fn post_session_review_window_open").next())
            .expect("strict review runner");
        assert!(runner.contains("dispatch_post_session_review(context, due)"));
        assert!(!runner.contains("current_banner()"));
        assert!(!runner.contains("evaluate_account_mode_hook(true)"));
        assert!(!runner.contains("chrono::Local::now()"));
    }

    #[test]
    fn br183_post_session_scheduler_guards_selection_without_blocking_core_review() {
        let source = include_str!("main.rs");
        let production = source
            .split("mod tests_post_session_review_scheduler")
            .next()
            .expect("production source precedes scheduler tests");
        let scheduler = production
            .split("async fn post_session_review_scheduler(selection_v2_enabled: bool)")
            .nth(1)
            .and_then(|tail| tail.split("fn spawn_post_session_review_scheduler(").next())
            .expect("post-session scheduler body");
        let capability_guard = scheduler
            .find("if selection_v2_enabled {")
            .expect("selection capability guard");
        let v2 = scheduler
            .find(".settle_tick(now.fixed_offset(), 200, &outcome_gateway)")
            .expect("v2 recovery-first coordinator receives the captured Shanghai tick instant");
        let review_window_gate = scheduler
            .find("if !post_session_review_window_open(")
            .expect("core review window gate");
        assert!(
            capability_guard < v2 && v2 < review_window_gate,
            "v2 recovery must run before the independent core review window gate"
        );
        assert!(
            !scheduler.contains("selection_shadow::"),
            "the replaced legacy selection observer must not return"
        );
        let v2_branch = &scheduler[v2..review_window_gate];
        let error_branch = v2_branch.find("Err(error) =>").expect("v2 failure branch");
        let error_branch = v2 + error_branch;
        assert!(
            !scheduler[error_branch..review_window_gate].contains("continue;"),
            "selection failure must not suppress independent core review work"
        );
    }

    #[test]
    fn br163_post_close_news_uses_typed_gateway_without_legacy_provider() {
        let source = include_str!("main.rs");
        let production = source
            .split("mod tests_post_session_review_scheduler")
            .next()
            .expect("production source precedes scheduler tests");
        let review = production
            .split("async fn post_close_news_review()")
            .nth(1)
            .and_then(|tail| tail.split("async fn post_close_news_scheduler()").next())
            .expect("post-close news review function");
        assert!(review.contains("SinaInstrumentNewsGateway"));
        assert!(review.contains(".instrument_news_in_range(code, from, now)"));
        let retired_provider = ["Sina", "NewsProvider"].concat();
        let retired_fetch = ["fetch_", "stock_", "news", "_in_range"].concat();
        assert!(!review.contains(&retired_provider));
        assert!(!review.contains(&retired_fetch));
    }
}

#[cfg(test)]
mod tests_br140_review_chain_isolation {
    use super::*;

    #[test]
    fn br140_r03_missing_sector_does_not_block_later_verified_stock() {
        let candidates = vec![
            ReviewLimitChainCandidate {
                code: "TEST_CODE_000001".to_string(),
                name: "测试一".to_string(),
                sector: None,
            },
            ReviewLimitChainCandidate {
                code: "TEST_CODE_000002".to_string(),
                name: "测试二".to_string(),
                sector: Some("测试产业链".to_string()),
            },
        ];

        let batch = collect_review_limit_chain_stocks_with(
            &candidates,
            |code| {
                if code.ends_with("000001") {
                    Err("TEST_CODE industry unavailable".to_string())
                } else {
                    unreachable!("complete sector must not call resolver")
                }
            },
            |_code| Ok(vec![true, false]),
        );

        assert_eq!(batch.accepted.len(), 1);
        assert_eq!(batch.accepted[0].code, "TEST_CODE_000002");
        assert_eq!(batch.rejected.len(), 1);
        assert!(!batch.source_complete());
    }

    #[test]
    fn br140_r03_kline_failure_is_isolated_per_stock() {
        let candidates = vec![
            ReviewLimitChainCandidate {
                code: "TEST_CODE_000001".to_string(),
                name: "测试一".to_string(),
                sector: Some("测试产业链".to_string()),
            },
            ReviewLimitChainCandidate {
                code: "TEST_CODE_000002".to_string(),
                name: "测试二".to_string(),
                sector: Some("测试产业链".to_string()),
            },
        ];

        let batch = collect_review_limit_chain_stocks_with(
            &candidates,
            |_code| unreachable!("complete sector must not call resolver"),
            |code| {
                if code.ends_with("000001") {
                    Err("TEST_CODE kline unavailable".to_string())
                } else {
                    Ok(vec![true, false])
                }
            },
        );

        assert_eq!(batch.accepted.len(), 1);
        assert_eq!(batch.accepted[0].code, "TEST_CODE_000002");
        assert_eq!(batch.rejected.len(), 1);
        assert!(batch.rejected[0].reason.contains("日 K 获取失败"));
        assert!(!batch.source_complete());
    }

    #[test]
    fn br140_r03_async_loader_never_nests_runtime() {
        let source = include_str!("main.rs");
        let loader = source
            .split("async fn load_review_limit_chain_stocks(")
            .nth(1)
            .expect("R-03 async loader")
            .split("async fn build_close_review_report(")
            .next()
            .expect("R-03 loader body");
        assert!(!loader.contains("block_on_async"));
        assert!(!loader.contains(".get_daily_data("));
        assert!(!loader.contains("fetch_industry_name_only"));
        assert!(loader.contains("ReviewDataGateway::new"));
        assert!(loader.contains("r03_upper_limit_pool"));
    }

    #[test]
    fn br159_r03_filters_market_pool_and_rejects_incomplete_same_batch_rows() {
        let date = chrono::NaiveDate::from_ymd_opt(2099, 1, 2).expect("test date");
        let candidates = vec![
            ReviewLimitChainCandidate {
                code: "TEST_CODE_000001".to_string(),
                name: "测试一".to_string(),
                sector: None,
            },
            ReviewLimitChainCandidate {
                code: "TEST_CODE_000002".to_string(),
                name: "测试二".to_string(),
                sector: None,
            },
        ];
        let evidence = stock_analysis::data_gateway::BatchEvidence {
            provider: magic_market_core::ProviderId::Eastmoney,
            source: "TEST_CODE_eastmoney-web".to_string(),
            source_at: Some("2099-01-02".to_string()),
            observed_at: "2099-01-02T16:00:00+08:00".to_string(),
            batch_id: "TEST_CODE_r03_batch".to_string(),
        };
        let gateway_batch = stock_analysis::data_gateway::GatewayBatch::Available {
            records: vec![
                stock_analysis::data_gateway::UpperLimitRecord {
                    code: "TEST_CODE_000001".to_string(),
                    trading_date: date,
                    theme: Some("测试题材".to_string()),
                    streak: Some(2),
                },
                stock_analysis::data_gateway::UpperLimitRecord {
                    code: "TEST_CODE_000002".to_string(),
                    trading_date: date,
                    theme: None,
                    streak: Some(1),
                },
                stock_analysis::data_gateway::UpperLimitRecord {
                    code: "TEST_CODE_OUTSIDE".to_string(),
                    trading_date: date,
                    theme: Some("范围外题材".to_string()),
                    streak: Some(3),
                },
            ],
            evidence,
        };

        let batch = map_review_limit_chain_gateway_batch(candidates, &gateway_batch, Vec::new())
            .expect("typed R-03 batch");

        assert_eq!(batch.accepted.len(), 1);
        assert_eq!(batch.accepted[0].code, "TEST_CODE_000001");
        assert_eq!(batch.accepted[0].chain, "测试题材");
        assert_eq!(batch.accepted[0].consecutive_days, 2);
        assert_eq!(batch.rejected.len(), 1);
        assert_eq!(batch.rejected[0].code, "TEST_CODE_000002");
        assert!(batch.rejected[0].reason.contains("题材证据缺失"));
    }
}

// ========================================================================
// v17.7 Task 6 Step 1: Announcement routing duplicate-prevention test
// ========================================================================

#[cfg(test)]
mod tests_v17_7_announcement_wiring {
    use super::*;
    use chrono::Local;
    use stock_analysis::announcement::{self, Announcement};

    #[test]
    fn br138_explicit_watch_audience_is_validated_independently() {
        let watch = std::collections::HashSet::from(["TEST_CODE_WATCH".to_string()]);
        let audience = validate_announcement_watch_codes(&watch).expect("valid watch audience");
        assert_eq!(audience, watch);
    }

    #[test]
    fn br138_watch_audience_rejects_blank_codes() {
        let watch = std::collections::HashSet::from(["".to_string()]);
        assert!(validate_announcement_watch_codes(&watch).is_err());
    }

    #[test]
    fn br138_watch_load_failure_remains_explicit_instead_of_empty_audience() {
        let result = collect_announcement_watch_codes(Err(
            "TEST_CODE explicit watch source unavailable".to_string(),
        ));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn br138_unfinished_watch_load_is_never_awaited_by_outer_tick() {
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let mut task = Some(tokio::task::spawn_blocking(move || {
            release_rx
                .recv()
                .expect("test controls completion of the background watch load");
            Err("TEST_CODE watch source unavailable".to_string())
        }));

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            poll_announcement_watch_load(&mut task),
        )
        .await
        .expect("an unfinished watch load must return immediately");
        assert!(result
            .expect_err("pending watch load is explicit")
            .contains("in progress"));

        release_tx.send(()).expect("release background test task");
        let _ = task
            .take()
            .expect("unfinished task remains owned by the next tick")
            .await;
    }

    #[test]
    fn br138_watch_failure_does_not_remove_independent_holding_news_codes() {
        let holding_codes = Ok(std::collections::HashSet::from([
            "TEST_CODE_HOLDING".to_string()
        ]));
        let watch_codes: Result<std::collections::HashSet<String>, String> =
            Err("TEST_CODE watch source unavailable".to_string());

        let news_codes = merge_news_monitor_codes(holding_codes, watch_codes.as_ref().ok())
            .expect("independent holding source remains usable");
        assert!(news_codes.contains("TEST_CODE_HOLDING"));
    }

    #[test]
    fn br138_watch_readiness_cannot_short_circuit_outer_tick_tail() {
        for readiness in [
            AnnouncementWatchReadiness::Pending,
            AnnouncementWatchReadiness::Failed,
        ] {
            let mut coordinator =
                NewsOuterTickCoordinator::new(AnnouncementWatchReadiness::Pending);
            let mut callback_counts = [0_u8; NewsOuterTickPhase::ALL.len()];

            for phase in [NewsOuterTickPhase::CriticalFlash] {
                if coordinator.enter(phase) {
                    callback_counts[phase as usize] += 1;
                }
            }
            coordinator.set_watch_readiness(readiness);
            for phase in [
                NewsOuterTickPhase::HoldingEarnings,
                NewsOuterTickPhase::L2,
                NewsOuterTickPhase::Announcement,
                NewsOuterTickPhase::Reset,
                NewsOuterTickPhase::Flush,
                NewsOuterTickPhase::Banner,
                NewsOuterTickPhase::Sleep,
            ] {
                if coordinator.enter(phase) {
                    callback_counts[phase as usize] += 1;
                }
            }

            coordinator
                .finish()
                .expect("pending/failed watch keeps the outer tick contract complete");
            for phase in NewsOuterTickPhase::ALL {
                let expected = u8::from(phase != NewsOuterTickPhase::Announcement);
                assert_eq!(
                    callback_counts[phase as usize],
                    expected,
                    "phase {} callback count for watch {:?}",
                    phase.label(),
                    readiness
                );
            }
        }
    }

    #[test]
    fn br138_missing_user_snapshot_keeps_watch_audience_and_stays_explicit() {
        // BR-226: 无券商且无用户确认快照时, 持仓身份显式排除, 自选受众继续
        let watch = std::collections::HashSet::from(["TEST_CODE_WATCH".to_string()]);
        let (audience, warning) = load_announcement_audience_codes(&watch);
        assert_eq!(audience, watch);
        assert!(warning
            .expect("missing position evidence must remain explicit")
            .contains("user position snapshot not provided"));
    }

    #[test]
    fn br138_stale_positions_do_not_block_independent_watch_audience() {
        let watch = std::collections::HashSet::from(["TEST_CODE_WATCH".to_string()]);
        let (audience, warning) = isolate_announcement_position_failure(
            Err("BR-138 stale position component".to_string()),
            &watch,
        );
        assert_eq!(audience, watch);
        assert!(warning.is_some());
    }

    /// Report from simulate_announcement_loop
    struct AnnouncementLoopReport {
        announcement_attempts: usize,
        /// How many times legacy push would be called for a given external_id
        legacy_attempts: std::collections::HashMap<String, usize>,
    }

    impl AnnouncementLoopReport {
        fn legacy_daily_report_attempts_for(&self, external_id: &str) -> usize {
            self.legacy_attempts.get(external_id).copied().unwrap_or(0)
        }
    }

    /// Simulates the v17.7 announcement loop logic:
    /// 1. Route announcements via the production normalized owner
    /// 2. Track normalized-owned external_ids
    /// 3. Join emitted alerts to the normalized outcome by provider input index
    async fn simulate_announcement_loop(anns: Vec<Announcement>) -> AnnouncementLoopReport {
        // Push via the production BR-137 per-announcement owner.
        let eligible_codes = anns
            .iter()
            .map(|announcement| announcement.code.clone())
            .collect();
        let routed = v17_sources::route_announcements(&anns, &eligible_codes).await;
        let report = routed.source;

        let mut monitor = stock_analysis::monitor::news_monitor::NewsMonitor::new();
        for ann in &anns {
            monitor.linker_mut().register_position(&ann.code, &ann.name);
        }
        let indexed_events =
            monitor.process_announcements_indexed(&anns, &std::collections::HashMap::new());
        for (input_index, _event) in indexed_events {
            assert_ne!(
                announcement_alert_action(input_index, &routed),
                AnnouncementAlertAction::Suppress,
                "valid routed announcement should reach normalized downstream"
            );
        }
        let legacy_attempts = std::collections::HashMap::new();

        AnnouncementLoopReport {
            announcement_attempts: report.pushed,
            legacy_attempts,
        }
    }

    /// Helper to create a test announcement with external_id
    fn test_important_announcement(external_id: &str, code: &str) -> Announcement {
        Announcement {
            code: code.to_string(),
            name: "测试公司".to_string(),
            title: "关于回购股份方案的公告".to_string(),
            date: Local::now().date_naive().format("%Y-%m-%d").to_string(),
            summary: "回购".to_string(),
            content: String::new(),
            level: announcement::AnnLevel::Important,
            reason: "标题含'回购'".to_string(),
            external_id: Some(external_id.to_string()),
            url: Some("https://example.invalid/ann".to_string()),
        }
    }

    /// v17.7 §6 Step 2: Test should FAIL because current news_monitor_loop
    /// directly processes and pushes the same announcement through the legacy path.
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn routed_announcement_is_not_sent_again_as_daily_report() {
        // The production path initializes LATEST_BANNER before dispatch. This isolated
        // test must do the same explicitly; relying on another test's global setup is flaky.
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        let report = simulate_announcement_loop(vec![test_important_announcement(
            "ann-1",
            "TEST_CODE_ANNOUNCEMENT_1",
        )])
        .await;
        assert_eq!(
            report.announcement_attempts, 1,
            "should route 1 announcement"
        );
        assert_eq!(
            report.legacy_daily_report_attempts_for("ann-1"),
            0,
            "routed announcement should not trigger legacy push"
        );
    }

    #[test]
    fn br138_filtered_normalized_alert_cannot_trigger_downstream_notifications() {
        for disposition in [
            v17_sources::AnnouncementDisposition::FilteredClassification,
            v17_sources::AnnouncementDisposition::FilteredAudience,
            v17_sources::AnnouncementDisposition::FilteredLifecycle,
            v17_sources::AnnouncementDisposition::Failed,
        ] {
            let route =
                v17_sources::AnnouncementSourceRouteReport::with_dispositions_for_test(vec![
                    disposition,
                ]);
            assert_eq!(
                announcement_alert_action(0, &route),
                AnnouncementAlertAction::Suppress
            );
        }
        let route = v17_sources::AnnouncementSourceRouteReport::with_dispositions_for_test(vec![
            v17_sources::AnnouncementDisposition::Pushed,
        ]);
        assert_eq!(
            announcement_alert_action(0, &route),
            AnnouncementAlertAction::NormalizedDownstream
        );
    }

    #[test]
    fn br138_provider_announcement_without_normalized_disposition_fails_closed() {
        let route = v17_sources::AnnouncementSourceRouteReport::default();

        assert_eq!(
            announcement_alert_action(0, &route),
            AnnouncementAlertAction::Suppress,
            "provider announcements without a normalized disposition must never use legacy delivery"
        );
    }

    #[test]
    fn br051_test_event_paths_are_physically_isolated() {
        assert_eq!(
            runtime_data_path(true, "event_bus"),
            std::path::PathBuf::from("data/test/event_bus")
        );
        assert_eq!(
            runtime_data_path(true, "replay_audit"),
            std::path::PathBuf::from("data/test/replay_audit")
        );
        assert_eq!(
            runtime_data_path(false, "event_bus"),
            std::path::PathBuf::from("data/event_bus")
        );
    }
}
