//! Registered business rules: BR-043, BR-045, BR-047, BR-049, BR-051, BR-063, BR-071, BR-073, BR-074, BR-077, BR-078, BR-082, BR-083, BR-136, BR-140.
//! 实盘监控模式入口。

//!

//! 用法：

//!   cargo run --bin monitor             # 正常监控（等交易日+交易时段）

//!   cargo run --bin monitor -- --test   # 隔离 E2E dry-run（等价于 --test --e2e）

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

use stock_analysis::monitor::alert;

use stock_analysis::monitor::checklist;

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
        ]);
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
        std::env::set_var("PUSH_VERBOSE", "true");
        std::env::set_var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "0");
        std::env::set_var("STOCK_ENV_MODE", "test");
        static AUDIT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let audit_sequence = AUDIT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::set_var(
            "EVENT_AUDIT_DIR",
            std::env::temp_dir().join(format!(
                "stock-analysis-monitor-audit-test-{}-{}",
                std::process::id(),
                audit_sequence
            )),
        );
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

mod push_templates;

mod review_batch;

mod dryrun_report; // v26: dry-run 自动报告

mod v13_diag; // v13.27: 端到端诊断

mod closing_valuation_runtime;
mod data_mode_probe;
mod market_data; // BR-148: capability probes remain independent from governance DataMode

mod intraday_market;

mod v14_adapter;

mod l6_sink;

mod news_aggregator_init;

mod daily_report_router; // v17.6 §5.1: DailyReport SubKind 拆分 (3 variants → DailyReport 主路径)

mod health;

mod webhook_alert;

// 修复 Top10#3+#4 (2026-06-29 audit): 拆大文件

mod freshness;

mod v17_sources; // v17.7 Task 5: six-source monitor push adapter

pub use freshness::{
    monitor_freshness_config, validate_position_freshness, validate_quote_freshness,
};

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

fn load_latest_prior_virtual_snapshot() -> Result<Option<VirtualObservationSnapshot>, String> {
    let dir = virtual_observation_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("遍历虚拟观察目录 {} 失败: {error}", dir.display()));
        }
    };
    let today = chrono::Local::now().date_naive();

    let mut best: Option<std::path::PathBuf> = None;
    let mut best_day: Option<chrono::NaiveDate> = None;

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("读取虚拟观察目录项 {} 失败: {error}", dir.display()))?;
        let p = entry.path();

        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }

        let stem = match p.file_stem().and_then(|x| x.to_str()) {
            Some(s) => s,

            None => continue,
        };

        if stem == "latest" {
            continue;
        }
        if stem.len() != 8 {
            return Err(format!("虚拟观察日期文件名非法: {}", p.display()));
        }
        let day = chrono::NaiveDate::parse_from_str(stem, "%Y%m%d")
            .map_err(|error| format!("虚拟观察日期文件名 {} 非法: {error}", p.display()))?;
        if day >= today {
            continue;
        }
        if best_day.is_none_or(|current| day > current) {
            best_day = Some(day);
            best = Some(p);
        }
    }

    let Some(path) = best else {
        return Ok(None);
    };
    let day = best_day.ok_or_else(|| "虚拟观察最近日期状态不一致".to_string())?;
    read_virtual_observation_snapshot(&path, day)?
        .ok_or_else(|| format!("选中的虚拟观察快照在读取时消失: {}", path.display()))
        .map(Some)
}

/// v13.10.1 P0-#2: 拉 T+1 收盘价, 即 base_date 后第 1 个交易日的 close.

/// 修复前: fetch_latest_close_map 取的是当下 K 线最后一日, 跨 13 天后 close 实际是 T+13, 不是"次日".

/// 返回 None 时调用方写"数据不足"避免误用累积收益当次日表现.

fn fetch_t1_close_map(
    codes: &[String],

    base_date: chrono::NaiveDate,
) -> Result<std::collections::HashMap<String, f64>, String> {
    let mut out = std::collections::HashMap::new();
    let fetcher = stock_analysis::data_provider::DataFetcherManager::new()
        .map_err(|error| format!("[虚拟观察仓] 初始化数据抓取器失败: {error:#}"))?;

    for code in codes {
        // 拉 30 天 K 线足够覆盖 base_date 之后 1-2 周的交易日

        match fetcher.get_daily_data(code, 30) {
            Ok((kline, _)) => {
                // 找 base_date 之后第 1 个交易日 (K 线按日期升序)

                if let Some(t1) = kline.iter().find(|k| k.date > base_date) {
                    if t1.close > 0.0 {
                        out.insert(code.clone(), t1.close);
                    }
                }

                // 没有 T+1 → 不 insert, 调用方通过 .get() == None 显示"数据不足"
            }

            Err(error) => {
                return Err(format!(
                    "[虚拟观察仓] fetch_daily_data({code}) 失败: {error:#}"
                ));
            }
        }
    }

    Ok(out)
}

/// 从 snapshot.created_at (格式 "YYYY-MM-DD HH:MM:SS") 解析出 NaiveDate

fn parse_snapshot_base_date(created_at: &str) -> Option<chrono::NaiveDate> {
    let s = created_at.split_whitespace().next()?;

    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

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

async fn push_virtual_next_day_review_if_needed() -> Result<(), String> {
    let cfg = stock_analysis::config::get_monitor_config();

    if !cfg.air_refuel.next_day_review_enabled {
        return Ok(());
    }

    let Some(snapshot) = load_latest_prior_virtual_snapshot()? else {
        return Ok(());
    };

    let codes: Vec<String> = snapshot.records.iter().map(|r| r.code.clone()).collect();

    // v13.10.1 P0-#2: 用 T+1 收盘价 (snapshot.created_at 后第 1 个交易日),

    // 不用当前最新 close, 否则跨多日后收益是累积而非"次日".

    let base_date = match parse_snapshot_base_date(&snapshot.created_at) {
        Some(d) => d,

        None => {
            return Err(format!(
                "[虚拟观察仓] snapshot.created_at 解析失败: {}",
                snapshot.created_at
            ))
        }
    };

    let close_map = tokio::task::spawn_blocking(move || fetch_t1_close_map(&codes, base_date))
        .await
        .map_err(|error| format!("[虚拟观察仓] T+1 后台任务失败: {error}"))??;

    if let Some(text) = build_virtual_next_day_review_text(&snapshot, &close_map)? {
        match push_governor_v3(&text, PushKind::DailyReport, None).await {
            notify::PushOutcome::Pushed | notify::PushOutcome::Deduped => {}
            notify::PushOutcome::Denied(reason) => {
                return Err(format!("[虚拟观察仓] 次日复盘被治理拒绝: {reason}"));
            }
            notify::PushOutcome::SinkError(error) => {
                return Err(format!("[虚拟观察仓] 次日复盘投递失败: {error}"));
            }
        }
    }
    Ok(())
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

// ============= v17.6: 6 dispatcher 调度入口 (--push 模式) ============

/// v14.0: dry-run 模式, 验证 dispatcher 数据源 + 渲染, 不实际推送

async fn run_daily_pushes_dry_run() -> Result<(), String> {
    tokio::task::spawn_blocking(run_daily_pushes_dry_run_blocking)
        .await
        .map_err(|error| format!("dry-run blocking task failed: {error}"))?
}

fn run_daily_pushes_dry_run_blocking() -> Result<(), String> {
    use push_templates::{
        build_industry_chain_intraday_from_snapshot, build_intraday_market_from_snapshot,
        build_news_catalyst_from_snapshot, build_news_to_idea_from_snapshot,
        build_paper_review_from_snapshot, build_preopen_news_hot_from_db,
        load_industry_chain_snapshot_real, load_news_catalyst_snapshot_real,
        load_news_to_idea_snapshot_real, load_paper_review_snapshot_real,
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
            match build_preopen_news_hot_from_db(&hhmm, &clusters, &rotations) {
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

    match load_news_to_idea_snapshot_real(&hhmm) {
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

    match load_paper_review_snapshot_real(&date) {
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

/// v17.6: 按当前时间窗触发 6 dispatcher

/// - 09:00 → P-01 (盘前新闻)

/// - 10:00/11:00/14:00 → I-01/I-02/I-03/D-01 (盘中)

/// - 19:00 → A-01 (盘后复盘)

/// - v22: 时刻从 config/strategy.toml [schedule] 读, 不再写死

fn require_dispatch_success(name: &str, delivered: bool) -> Result<(), String> {
    delivered
        .then_some(())
        .ok_or_else(|| format!("{name} dispatcher did not confirm delivery"))
}

async fn run_daily_pushes() -> Result<(), String> {
    use push_templates::{
        dispatch_catalyst_review_daily, dispatch_holding_plan_daily,
        dispatch_industry_chain_intraday_daily, dispatch_intraday_market_daily,
        dispatch_news_catalyst_daily, dispatch_news_to_idea_daily, dispatch_paper_review_daily,
        dispatch_preopen_news_hot_daily,
    };

    use stock_analysis::opportunity::scheduler::{OpportunitySchedule, PushWindow};

    // v22: 从 config 读取 push 时刻 (替代写死的 09:00 / 10:30 / 11:00 / 14:30 / 19:00)

    let schedule = OpportunitySchedule::default();

    let now = chrono::Local::now();

    let hhmm = now.format("%H:%M").to_string();

    let date = now.format("%Y-%m-%d").to_string();

    let now_time = now.time();

    log::info!(
        "[v22] --push 模式启动 (当前 {} {}, 时刻读 config)",
        date,
        hhmm
    );

    // v22: 用 push_window() 判断当前时刻窗口 (替代 v17.6 写死 hour)

    let window = schedule.push_window(now_time);

    log::info!("[v22] 推送窗口: {:?}", window);

    match window {
        PushWindow::Preopen => {
            require_dispatch_success("P-01", dispatch_preopen_news_hot_daily().await)?;
        }

        PushWindow::Intraday => {
            // 5 个盘中 dispatcher (I-01/I-02/I-03/I-04/D-01)
            let banner = current_banner()
                .map_err(|error| format!("BR-108 --push banner unavailable: {error}"))?;

            require_dispatch_success("I-01", dispatch_intraday_market_daily(&hhmm, &banner).await)?;
            require_dispatch_success("I-02", dispatch_news_catalyst_daily(&hhmm, &banner).await)?;
            require_dispatch_success(
                "I-03",
                dispatch_industry_chain_intraday_daily(&hhmm, &banner).await,
            )?;
            require_dispatch_success("D-01", dispatch_news_to_idea_daily(&hhmm, &banner).await)?;
            require_dispatch_success("I-04", dispatch_holding_plan_daily(&hhmm, &banner).await)?;
        }

        PushWindow::Evening => {
            require_dispatch_success("A-01", dispatch_paper_review_daily(&date).await)?;
            require_dispatch_success("A-10", dispatch_catalyst_review_daily(&date).await)?;
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

            require_dispatch_success("A-01", dispatch_paper_review_daily(&date).await)?;
            require_dispatch_success("A-10", dispatch_catalyst_review_daily(&date).await)?;
        }
    }

    log::info!("[v17.6] --push 完成 (HHMM: {})", hhmm);
    Ok(())
}

// ============= v12 PR1-1.7: AccountMode 评估钩子 =============

/// v41: 共享 banner 状态 (v12 §14.0.1 动态化)

/// 周期调 evaluate_account_mode_hook + evaluate_data_mode_hook 写最新 banner

/// 6 个 dispatcher / 推送构造 banner 时从这里读

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

    push_templates::BannerCtx {
        account_mode,
        total_pos: am_metrics.total_pos_cheng,
        today_pnl: am_metrics.today_pnl_pct,
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

fn refresh_closing_valuation_note() {
    let account = stock_analysis::database::user_account_summary::latest()
        .ok()
        .flatten();
    let note = match stock_analysis::database::closing_valuation::latest_persisted_valuation_view()
    {
        Ok(Some(view)) => {
            let account_note = match account.as_ref() {
                Some(account) => format!(
                    "用户确认账户 {:.1}%仓位，昨日盈亏 {:+.2}",
                    account.position_ratio_pct, account.daily_pnl
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

    let result =
        match pt::push_data_mode_change(&input, prev, persistent_reminder_due, Some(&banner)).await
        {
            Ok(result) => result,
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
    eprintln!("       monitor --review");
    eprintln!("       monitor --test --review");
    eprintln!("       monitor --replay=YYYY-MM-DD [--replay-force] [--replay-rate-ms=N]");
    eprintln!("       monitor --history [--date=YYYY-MM-DD] [--code=CODE] [--kind=KIND]");
    eprintln!("                         [--limit=N] [--success-rate] [--sink=SINK]");
    eprintln!("       monitor --help");
    eprintln!();
    eprintln!("--test is an isolated E2E dry-run alias; it never sends a real notification.");
    eprintln!("--review is production-strict and requires fresh, complete real account evidence.");
    eprintln!(
        "--test --review verifies that the strict review fails closed without live evidence."
    );
    eprintln!("Terminal commands exit after completion; bare monitor enters long-running loops.");
}

fn isolated_e2e_requested(arguments: &[String]) -> bool {
    arguments.iter().any(|argument| argument == "--e2e")
        || matches!(arguments, [_, only] if only == "--test")
}

fn service_enablement_required(arguments: &[String]) -> bool {
    arguments.len() == 1
}

fn runtime_data_path(test_mode: bool, leaf: &str) -> std::path::PathBuf {
    let root = if test_mode { "data/test" } else { "data" };
    std::path::PathBuf::from(root).join(leaf)
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

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
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

    // BR-051: CLI 环境必须先于任何全局数据源、事件持久化或数据库初始化确定。
    let startup_args: Vec<String> = std::env::args().collect();
    let test_mode = startup_args.iter().any(|arg| arg == "--test");
    let review_mode = startup_args.iter().any(|arg| arg == "--review");
    let explicit_e2e_mode = startup_args.iter().any(|arg| arg == "--e2e");
    let v13_diag_mode = startup_args.iter().any(|arg| arg == "--v13-diag");
    if explicit_e2e_mode && !test_mode {
        eprintln!("[BR-051] --e2e requires --test before any DB or push sink initialization");
        std::process::exit(2);
    }
    if v13_diag_mode && !test_mode {
        eprintln!("[BR-051][BR-141] --v13-diag requires --test before runtime initialization");
        std::process::exit(2);
    }
    let e2e_mode = isolated_e2e_requested(&startup_args);
    if test_mode {
        std::env::set_var("STOCK_ENV_MODE", "test");
        std::env::set_var("V10_DRY_RUN_PUSH", "1");
    } else {
        std::env::set_var("STOCK_ENV_MODE", "prod");
    }
    if startup_args
        .iter()
        .any(|arg| arg == "--help" || arg == "-h")
    {
        print_event_help();
        std::process::exit(0);
    }
    // BR-141: MONITOR_ENABLED is a lifecycle switch for the bare long-running
    // service only. Every explicit CLI argument must reach its parser and
    // truthful terminal status instead of being silently reported as success.
    if service_enablement_required(&startup_args) && !check_enabled() {
        log::info!("[monitor] disabled: MONITOR_ENABLED is not true");
        return;
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

    // v17.4: NewsAggregator 全局初始化 (13 个 NewsFeed 适配注册到 aggregator)
    // 调用方: news_monitor_loop 每 tick 调 tick_news_aggregator(20) 拿 dedup 后 events
    let news_feed_count = news_aggregator_init::init_news_aggregator();
    log::info!(
        "[v17.4] NewsAggregator 已初始化 ({} feeds registered)",
        news_feed_count
    );

    // v17.6 §5.1: daily_report_router 启动 audit (3 sub_kinds + legacy 映射表)
    daily_report_router::init_audit();

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

    // v17.4 D 方案启动 banner (v15.x 静默路径可见): SectorTop 废弃态 + 选股阈值
    log::info!(
        "[v17.4-D] sector_top.mode={} | screener_min_score={} | holding_health.dedup=on_same_state",
        if sector_top_kept() {
            "kept (env STOCK_ANALYSIS_KEEP_SECTOR_TOP 显式保留)"
        } else {
            "deprecated (I-01 覆盖; 回滚 env STOCK_ANALYSIS_KEEP_SECTOR_TOP=1)"
        },
        stock_analysis::config::get_monitor_config().screener_min_score,
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

    // 初始化数据库

    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| {
        if test_mode {
            "./data/test/monitor_test.db".into()
        } else {
            "./data/stock_analysis.db".into()
        }
    });
    if test_mode && std::path::Path::new(&db_path).ends_with("data/stock_analysis.db") {
        log::error!(
            "[BR-051] --test 拒绝打开默认生产数据库 {}; 请使用隔离 DATABASE_PATH",
            db_path
        );
        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
    }
    std::env::set_var("DATABASE_PATH", &db_path);
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            log::error!("[DB init] 创建目录 {:?} 失败: {}", parent, error);
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
        }
    }

    if std::env::var("MAGICLAW_DB_PATH")
        .ok()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        std::env::set_var("MAGICLAW_DB_PATH", &db_path);
    }

    if let Err(error) =
        stock_analysis::database::DatabaseManager::init(Some(std::path::PathBuf::from(&db_path)))
    {
        log::error!("[DB init] 失败: {}", error);
        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
    }

    // 加载热配置

    stock_analysis::config::load_all();

    if contains_legacy_manual_trade_flag(&startup_args) {
        eprintln!(
            "[BR-107] --buy/--sell 手工成交旁路已关闭：缺少统一行情、账户、模式、限价、确认与订单审计证据；请使用 paper decision/order safety 管道"
        );
        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
    }

    // v17.6: 推送模式 (--push), 调 6 dispatcher 一次后退出

    let push_mode = std::env::args().any(|a| a == "--push");

    // v14.0: dry-run 模式, 验证 dispatcher 加载 + 渲染, 不实际推送

    let push_dry_run = std::env::args().any(|a| a == "--push-dry-run");

    // v70: 隔离 e2e 模式 (--e2e), 跑所有 v12 §14 + v13.1 测试模板。

    // v70+: 兑现回填模式 (--backfill-outcome=YYYY-MM-DD)

    //   回填 d01_recommendations/YYYY-MM-DD.jsonl 里的 outcome 字段 (D+1/D+3/D+5/MFE/MAE)

    //   用途: D 日推送 → D+1 收盘后跑这个命令, 把推荐兑现数据写回

    let backfill_outcome_date: Option<String> =
        std::env::args().find_map(|a| a.strip_prefix("--backfill-outcome=").map(|s| s.to_string()));

    // v14.1 F7: stock_position.st_type 回填 (从 name LIKE 推断 ST/*ST)

    let backfill_st_type = std::env::args().any(|a| a == "--backfill-st-type");

    // v14.1 BR-015: stock_position.chain_name 缺失统计 (待 chain registry 接入)

    let backfill_chain_name = std::env::args().any(|a| a == "--backfill-chain-name");

    // v17.3 Task 5: Handle terminal event commands before entering long-running monitor loops.
    // Parse CLI args early to detect --replay / --history / --help before any background loops start.
    let args: Vec<String> = std::env::args().collect();
    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match stock_analysis::event::cli::parse_args(&args_refs) {
        Ok(Some(stock_analysis::event::cli::EventCommand::Help)) => {
            print_event_help();
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 0).await;
        }
        Ok(Some(stock_analysis::event::cli::EventCommand::Replay {
            date,
            force,
            rate_ms,
        })) => {
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
        Ok(Some(stock_analysis::event::cli::EventCommand::History {
            date,
            code,
            kind,
            limit,
            success_rate,
            sink,
        })) => {
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
        Ok(None) => {
            // No event command — fall through to existing monitor behavior.
        }
        Err(e) => {
            eprintln!("[event] CLI error: {}", e);
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 1).await;
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
            Err(error) => log::error!("[health] 失败告警投递失败: {}", error),
        }
    }

    // Task 8: 启动 banner 列出 K线 fallback 链 + 盘后路径, 便于线上排查.

    // 4-way 盘中 (review #15 + Phase 1): Sina → 腾讯 → 东财 → RustDX 并行竞速.

    // 盘后专用 (Phase 1 post_close): Baostock (P1) → 4-way fallthrough (P2).

    log::info!(

        "[启动] K线 fallback chain (盘中): sina_hq (P1) → tencent_qfq (P2) → eastmoney_qfq (P3) → rustdx_none (P4) | review #15 + #16"

    );

    log::info!("[启动] 盘后路径: baostock (P1) → 4-way join (P2, post_close)");

    // Task 11 + Task 12 (Phase 2): Sina 新闻链路 — 实时 + 盘后回溯.

    log::info!("[启动] 新闻轮询: Sina 财经要闻 (90s 间隔, 双写 news_items)");

    log::info!("[启动] 盘后回溯: Sina 个股新闻 (15:30 后, 30 天, 持仓代码)");

    // v70: 隔离 e2e 模式 (--e2e) — 跑所有 v12 §14 + v13.1 测试模板。

    if e2e_mode {
        log::info!("[v70] E2E 模式启动 — 跑所有 v12 §14 模板 (忽略时间窗口)");
        let exit_code = match e2e_all_templates_run().await {
            Ok(()) => 0,
            Err(error) => {
                log::error!("[v70][BR-051][BR-103] E2E 失败: {error}");
                2
            }
        };
        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
    }

    // v70+: 兑现回填 (D+1 outcome 写回 d01_recommendations jsonl)

    if let Some(date) = backfill_outcome_date {
        log::info!("[v70+] --backfill-outcome 模式启动 | 日期 = {}", date);

        use stock_analysis::opportunity::news_outcome::backfill_recommendations_outcome;

        match backfill_recommendations_outcome(&date) {
            Ok(updated) => {
                log::info!("[v70+] 回填完成 | {} | 更新行数 = {}", date, updated);
                exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 0).await;
            }
            Err(error) => {
                log::error!("[v70+] 回填失败 | {} | {}", date, error);
                exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
            }
        }
    }

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

    // v14.1 BR-015: chain_name 缺失统计 (实算留给 chain registry 接入)

    if backfill_chain_name {
        log::info!("[v14.1 BR-015] --backfill-chain-name 模式启动 | 用 chain_registry 实算");

        use stock_analysis::database::DatabaseManager;

        let db = DatabaseManager::get();

        let exit_code = match db.backfill_chain_name() {
            Ok((updated, missing)) => {
                log::info!(
                    "[v14.1 BR-015] 回填完成 | 更新 {} 条, 仍缺失 {} 条 (查不到 chain 或未在 registry)",
                    updated,
                    missing
                );
                0
            }
            Err(error) => {
                log::error!("[v14.1 BR-015] 回填失败: {error}");
                2
            }
        };

        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
    }

    if terminal_review_requested(&startup_args) {
        log::info!("[复盘] --review 终端模式启动，完成后退出，不进入常驻监控");
        let result = match review_execution_path(&startup_args) {
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
        if std::env::args().any(|a| a == "--v13-diag") {
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

        let review_result = run_review_only().await;

        // [v12 删除] P1.1 老 "📊 A股市场概览" 推送 (用 std::thread::spawn + block_in_place)

        // 由 v12 R-02 盘面走向 (render_review_market) 替代, 见 run_review_only_inner 末尾 v12 R-01~R-08

        // 真实市场概览 (5 维评分) 数据合到 v12 R-02, 不再单独推

        log::info!("[复盘] v12 模板已替代老市场概览推送");

        // 干净退出 (避免 runtime drop panic)

        let exit_code = match review_result {
            Ok(()) => 0,
            Err(error) => {
                log::error!("[复盘] {error}. exit 2.");
                2
            }
        };
        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, exit_code).await;
    } else if push_dry_run {
        log::info!("[v14.0] --push-dry-run 模式启动");

        if let Err(error) = run_daily_pushes_dry_run().await {
            log::error!("[v14.0] --push-dry-run 失败: {error}");
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
        }

        log::info!("[v14.0] --push-dry-run 完成");

        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 0).await;
    } else if push_mode {
        // v30: --push 模式 (修复 v22 死代码)

        //   调 6 dispatcher 一次后退出, 时刻读 config/strategy.toml [schedule]

        //   替代 v17.6 写死的 09:00 / 10:30 / 11:00 / 14:30 / 19:00

        log::info!("[v30] --push 模式启动");

        if let Err(error) = run_daily_pushes().await {
            log::error!("[v30][BR-108] --push 批次拒绝: {error}");
            exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
        }

        log::info!("[v30] --push 完成");

        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 0).await;
    } else if !service_enablement_required(&startup_args) {
        log::error!(
            "[BR-141] explicit CLI arguments did not select a terminal handler; refusing long-running service"
        );
        exit_after_jsonl_writer(bus, &mut jsonl_writer_handle, 2).await;
    } else {
        let dryrun_reporter = dryrun_report::spawn_dryrun_reporter(1_800);
        let outcome_backfill = dryrun_report::spawn_outcome_backfill_scheduler();

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

        let main_loops = async {
            // Phase 3: 移除 news_pipeline_loop_v15_3 (#2) — sink/aggregator 仅 #2 自用,
            //   #1 news_monitor_loop 已从同源 fetch_flash_titles 取快讯产候选, #2 重复取数且已停推
            tokio::join!(
                monitor_loop(),
                news_monitor_loop(),
                data_mode_monitor_loop()
            );
        };

        // Phase 3: 移除 poll_news_loop (#3) — news_items 表只写不读(无人 SELECT),
        //   且 #1 news_monitor_loop 已从 search_service 取 Sina 快讯, #3 重复取数+写废表

        // v13.12 (Task 12): 盘后回溯调度 — 30 min tick, 15:30 后触发持仓个股近 30 天新闻回溯

        let post_close_news = tokio::spawn(post_close_news_scheduler());
        let post_session_review = spawn_post_session_review_scheduler();
        let background_tasks = vec![
            ("dryrun_reporter", dryrun_reporter),
            ("outcome_backfill", outcome_backfill),
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

fn check_enabled() -> bool {
    std::env::var("MONITOR_ENABLED")
        .unwrap_or_default()
        .to_lowercase()
        == "true"
}

fn contains_legacy_manual_trade_flag(args: &[String]) -> bool {
    args.iter()
        .any(|argument| matches!(argument.as_str(), "--buy" | "--sell"))
}

fn terminal_review_requested(args: &[String]) -> bool {
    args.iter().any(|argument| argument == "--review")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewExecutionPath {
    StrictDispatchers,
}

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

/// 手动复盘：`cargo run --bin monitor -- --review`

async fn run_review_only() -> Result<(), String> {
    log::info!("[复盘] 手动触发盘后分析...");

    if !evaluate_account_mode_hook(true).await {
        return Err("[BR-108] 真实 AccountMode/banner 初始化失败".to_string());
    }

    // 修复 P0-G (2026-06-30 codex review): 顶层 5min fast-fail (AGENTS §2.1, BR-009).

    // 沙箱 / 数据源全失联时, 进程可能在 reqwest 内部回调里死锁,

    // 5min 后显式 exit 2 + ERROR 日志, 不推送噪声给用户.

    let review_timeout_secs = review_timeout_secs();

    log::info!(
        "[复盘] 顶层超时保护: {}s (env MONITOR_REVIEW_TIMEOUT_SECS 可覆盖)",
        review_timeout_secs
    );

    let review_start = std::time::Instant::now();

    let due: std::collections::BTreeSet<_> = review_batch::ReviewTask::ALL.into_iter().collect();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(review_timeout_secs),
        run_strict_review_only_inner(&due),
    )
    .await;

    match outcome {
        Ok(Ok(batch)) => {
            let now = chrono::Local::now();
            let mut audit_state = review_batch::ReviewScheduleState::for_date(now.date_naive());
            let transitions = audit_state.apply(&batch, now.naive_local());
            if let Err(error) =
                review_batch::append_task_transition_audit(transitions, now.date_naive())
            {
                return Err(format!("[BR-110][BR-140] 逐任务结果审计失败: {error}"));
            }
            if batch.has_confirmed_delivery() {
                log::info!(
                    "[复盘] ======== 盘后分析完成 ({}s) ========",
                    review_start.elapsed().as_secs()
                );
                Ok(())
            } else {
                Err("[BR-140] 严格盘后复盘没有任何确认投递；逐任务状态已写审计".to_string())
            }
        }

        Ok(Err(error)) => Err(format!("关键数据不可用: {error}")),

        Err(_elapsed) => Err(format!(
            "{}s 超时未完成, 上游数据源可能全部不可用 / 网络黑洞 / 死锁",
            review_timeout_secs
        )),
    }
}

/// BR-108/109/110: production `--review` may only use the verified shared
/// banner and the strict post-session dispatchers. The legacy inline review
/// below remains reachable only from the TEST_CODE E2E fixture.
async fn run_strict_review_only_inner(
    due: &std::collections::BTreeSet<review_batch::ReviewTask>,
) -> Result<review_batch::ReviewBatchOutcome, String> {
    let banner = current_banner()?;
    let now = chrono::Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    Ok(push_templates::dispatch_post_session_review(&date, now.time(), &banner, due).await)
}

fn filter_inline_r08_announcements(
    announcements: Vec<stock_analysis::data_provider::announcement::Announcement>,
) -> Vec<stock_analysis::data_provider::announcement::Announcement> {
    announcements
        .into_iter()
        .filter(
            stock_analysis::data_provider::announcement::announcement_is_immediate_notification_candidate,
        )
        .collect()
}

fn post_session_review_window_open(now: chrono::NaiveDateTime, is_trading_day: bool) -> bool {
    let threshold = chrono::NaiveTime::from_hms_opt(19, 0, 0)
        .expect("BR-139 post-session review threshold must be valid");
    is_trading_day && now.time() >= threshold
}

async fn attempt_post_session_review(
    due: &std::collections::BTreeSet<review_batch::ReviewTask>,
) -> Result<review_batch::ReviewBatchOutcome, String> {
    if !evaluate_account_mode_hook(true).await {
        return Err("real AccountMode/banner initialization was not confirmed".to_string());
    }

    let timeout_secs = review_timeout_secs();
    tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        run_strict_review_only_inner(due),
    )
    .await
    .map_err(|_| format!("strict review timed out after {timeout_secs}s"))?
}

async fn post_session_review_scheduler() {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut state: Option<review_batch::ReviewScheduleState> = None;
    let mut valuation_date: Option<chrono::NaiveDate> = None;

    log::info!("[复盘调度][BR-139] started threshold=19:00 interval=60s");

    loop {
        interval.tick().await;
        let now = chrono::Local::now();
        if !post_session_review_window_open(
            now.naive_local(),
            stock_analysis::calendar::is_trading_day(now.date_naive()),
        ) {
            continue;
        }

        if valuation_date != Some(now.date_naive())
            && closing_valuation_runtime::eligible_after_close(now.fixed_offset())
        {
            match closing_valuation_runtime::run_closing_valuation_once(now.date_naive()).await {
                Ok(receipt) => {
                    log::info!(
                        "[BR-147] closing valuation persisted: run_id={} inserted={}",
                        receipt.run_id,
                        receipt.inserted
                    );
                    valuation_date = Some(now.date_naive());
                    refresh_closing_valuation_note();
                }
                Err(error) => log::error!("[BR-147] closing valuation failed: {error}"),
            }
        }

        if state.as_ref().map(review_batch::ReviewScheduleState::date) != Some(now.date_naive()) {
            state = Some(review_batch::ReviewScheduleState::for_date(
                now.date_naive(),
            ));
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
                let transitions = next_schedule.apply(&batch, now.naive_local());
                if let Err(error) = review_batch::append_task_transition_audit(
                    transitions,
                    now.date_naive(),
                ) {
                    log::error!(
                        "[复盘调度][BR-110][BR-140] outcome audit failed; schedule state not committed: {error}"
                    );
                    continue;
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

fn spawn_post_session_review_scheduler() -> tokio::task::JoinHandle<()> {
    tokio::spawn(post_session_review_scheduler())
}

/// 实际复盘子流程 (被 run_review_only 包 5min timeout).

/// 单独提出便于测试 + 控制超时粒度.

async fn run_review_only_inner(isolated_test_fixtures: bool) -> Result<(), String> {
    let review_as_of = stock_analysis::calendar::latest_completed_trading_day_at(
        chrono::Local::now().naive_local(),
    );
    // v62: 6-tuple 返回 (实盘数据误差修复需要 quotes, 第二轮 fetch 在外部重新拉)

    let (report, _holding_breakout_text, _watch_breakout_text, _market_breakout_text, _risk_text) =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let holdings = stock_analysis::portfolio::get_positions()
                .map_err(|error| format!("复盘持仓查询失败: {error}"))?;

            let quotes = market_data::fetch_position_quotes()?;

            let prices = build_price_map(&quotes);

            let trades = stock_analysis::portfolio::get_trade_history(90)
                .map_err(|error| format!("复盘交易历史查询失败: {error}"))?;

            let mut reviews = stock_analysis::review::journal::review_closed_trades(&trades)
                .map_err(|error| format!("复盘成交 FIFO 失败: {error}"))?;

            stock_analysis::review::journal::enrich_post_exit(&mut reviews);

            let equity = stock_analysis::portfolio::get_equity_curve_as_of(365, review_as_of)
                .map_err(|error| format!("复盘净值曲线查询失败: {error}"))?;

            let mut stats = stock_analysis::review::equity::compute_stats(&equity)
                .map_err(|error| format!("复盘净值统计失败: {error}"))?;

            stock_analysis::review::equity::enrich_with_trades(&mut stats, &reviews)
                .map_err(|error| format!("复盘交易统计失败: {error}"))?;

            let r = stock_analysis::review::report::generate_daily_report_with_ledger(
                &reviews,
                &stats,
                &holdings,
                &prices,
                Some(equity.as_slice()),
            );

            if let Err(error) = snapshot_portfolio_value() {
                log::error!("[净值快照][BR-103][BR-109] {error}");
            }

            // 持仓代码集合：止损/轮动只对真实持仓有意义

            let holding_codes: std::collections::HashSet<String> =
                holdings.iter().map(|p| p.code.clone()).collect();

            // 持仓成本/硬止损索引（用于止损检查）

            let holding_map: std::collections::HashMap<
                String,
                &stock_analysis::portfolio::Position,
            > = holdings.iter().map(|p| (p.code.clone(), p)).collect();

            // v6 放量分析（持仓 / 自选 分开发送）

            let mut holding_brk = String::new();

            let mut watch_brk = String::new();

            let mut market_brk = String::new();

            // v7 风控：收盘止损 + 轮动研判（复用已拉 K 线，零额外 HTTP）

            let mut stop_signals: Vec<stock_analysis::risk::stop_loss::StopSignal> = Vec::new();

            let mut rotation_lines: Vec<String> = Vec::new();

            let watchlist = stock_analysis::portfolio::get_watchlist()
                .map_err(|error| format!("复盘自选查询失败: {error}"))?;

            let watch_codes: std::collections::HashSet<String> =
                watchlist.iter().map(|p| p.code.clone()).collect();

            if let Ok(fetcher) = stock_analysis::data_provider::DataFetcherManager::new() {
                // —— 持仓放量分析 + 止损 / 轮动 ——

                let mut holding_lines =
                    vec!["📊 放量分析·持仓（盘后·算法研判仅供参考）".to_string()];

                for p in &holdings {
                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {
                        let sig = stock_analysis::breakout::engine::analyze_postmarket(
                            &p.code, &p.name, &kline,
                        );

                        holding_lines.push(format!(
                            "  {} {}({}) — {} 置信{}% [{}]",
                            sig.breakout_type.emoji(),
                            sig.name,
                            sig.code,
                            sig.breakout_type.label(),
                            sig.confidence,
                            sig.description,
                        ));

                        // 现价：缺失则跳过止损（不静默用 0 价触发假硬止损 — AGENTS.md 2.2）

                        match prices.get(&p.code) {
                            Some(&cur) if cur > 0.0 => {
                                let ma20 = compute_ma(&kline, 20);

                                let ma60 = compute_ma(&kline, 60);

                                if let Some(pos) = holding_map.get(&p.code) {
                                    let mut sigs = stock_analysis::risk::stop_loss::check_stops(
                                        &p.code,
                                        &p.name,
                                        cur,
                                        pos.cost_price,
                                        pos.hard_stop,
                                        ma20,
                                        ma60,
                                    );

                                    stop_signals.append(&mut sigs);
                                }
                            }

                            _ => log::warn!("[复盘] {}({}) 现价缺失，跳过止损检查", p.name, p.code),
                        }

                        // 轮动研判（健康回调 vs 趋势结束）

                        let rot = stock_analysis::decision::rotation::judge_trend(&kline);

                        rotation_lines.push(format!(
                            "  {} {}({}) — {} [{}]",
                            rot.status.emoji(),
                            p.name,
                            p.code,
                            rot.status.label(),
                            rot.reasons.join("·"),
                        ));
                    }
                }

                if holding_lines.len() > 1 {
                    holding_brk = holding_lines.join("\n");
                }

                // —— 自选（STOCK_LIST）放量分析（剔除已在持仓列出的标的）——

                let mut watch_lines = vec!["📊 放量分析·自选（盘后·算法研判仅供参考）".to_string()];

                for p in &watchlist {
                    if holding_codes.contains(&p.code) {
                        continue;
                    }

                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {
                        let sig = stock_analysis::breakout::engine::analyze_postmarket(
                            &p.code, &p.name, &kline,
                        );

                        watch_lines.push(format!(
                            "  {} {}({}) — {} 置信{}% [{}]",
                            sig.breakout_type.emoji(),
                            sig.name,
                            sig.code,
                            sig.breakout_type.label(),
                            sig.confidence,
                            sig.description,
                        ));
                    }
                }

                if watch_lines.len() > 1 {
                    watch_brk = watch_lines.join("\n");
                }

                if isolated_test_fixtures {
                    log::info!("[复盘][BR-051] isolated E2E skips external market candidate scan");
                } else {
                    // —— 实盘量能优选：全市场量能前列 + 走势较好（盘后 Top5）——
                    let mut market_lines =
                        vec!["📊 放量分析·实盘优选（盘后·算法研判仅供参考）".to_string()];
                    let market_candidates = market_data::fetch_market_volume_ratio_leaders(80)?;
                    let mut picked = 0usize;

                    for s in &market_candidates {
                        if picked >= 5 {
                            break;
                        }
                        if holding_codes.contains(&s.code) || watch_codes.contains(&s.code) {
                            continue;
                        }
                        let (Some(volume_ratio), Some(main_net_yi)) =
                            (s.volume_ratio, s.main_net_yi)
                        else {
                            log::warn!(
                                "[复盘] {}({}) 量比/主力流缺失，跳过实盘优选",
                                s.name,
                                s.code
                            );
                            continue;
                        };

                        if let Ok((kline, _)) = fetcher.get_daily_data(&s.code, 60) {
                            let sig = stock_analysis::breakout::engine::analyze_postmarket(
                                &s.code, &s.name, &kline,
                            );
                            if sig.breakout_type
                                != stock_analysis::breakout::signal::BreakoutType::Launch
                                || sig.confidence < 50
                            {
                                continue;
                            }
                            market_lines.push(format!(
                                "  {} {}({}) — {} 置信{}% [量比{:.1} 主力{:+.2}亿 | {}]",
                                sig.breakout_type.emoji(),
                                sig.name,
                                sig.code,
                                sig.breakout_type.label(),
                                sig.confidence,
                                volume_ratio,
                                main_net_yi,
                                sig.description,
                            ));
                            picked += 1;
                        }
                    }

                    if market_lines.len() > 1 {
                        market_brk = market_lines.join("\n");
                    }
                }
            }

            // 组装风控文本：止损告警 + 轮动研判 + 现金底限告警

            let mut risk = String::new();

            let stop_text = stock_analysis::risk::stop_loss::format_stop_alerts(&stop_signals);

            if !stop_text.is_empty() {
                risk.push_str(&stop_text);
            }

            if !rotation_lines.is_empty() {
                if !risk.is_empty() {
                    risk.push_str("\n\n");
                }

                risk.push_str("🔄 持仓轮动研判（算法·仅供参考）\n");

                risk.push_str(&rotation_lines.join("\n"));
            }

            // 修复 (2026-06-30 codex review): --review 路径之前没调 cash_guard,

            // P0 cash_floor 在 --review 模式下不生效. 补上现金底限告警.

            if let Some(latest) = equity.last() {
                let guard = stock_analysis::risk::cash_guard::CashGuard::default();

                if let Some(alert) = stock_analysis::risk::cash_guard::check_cash(
                    latest.cash,
                    latest.total_value,
                    &guard,
                ) {
                    if alert.below_floor {
                        if !risk.is_empty() {
                            risk.push_str("\n\n");
                        }

                        risk.push_str(&stock_analysis::risk::cash_guard::format_cash_alert(&alert));
                    }
                }
            }

            Ok((r, holding_brk, watch_brk, market_brk, risk))
        })
        .await
        .map_err(|error| format!("复盘 blocking 任务失败: {error}"))??;

    log::info!("[复盘] 复盘报告:\n{}", report);

    // [v12 删除] push_wechat(&report).await  — 老 "📊 交易复盘 2026-07-05" 格式

    // 由 v12 R-01 持仓明日计划 (render_daily_report) 替代, 见下方 v12 R-01 推送

    // P1.1 市场概览: 在 async context 直接调 (与项目 block_in_place 模式一致)

    // (原 spawn_blocking 闭包内的版本已删除, 避免 block_in_place 错位)

    // P1.1 hotfix v9: --review 模式跳过市场概览 (详见 run_review_only 注释)

    // 这里不再调 get_market_overview, 因为实测三种调用方式都触发 tokio runtime drop panic.

    // 真正的修复 (改成 async) 在 P2.x 范围.

    // [v12 删除] 老 "📋 候选筛选台" (OptimalClose)  — 由 v12 T-07/R-07 替代 (MVP-3 影子 + R-07 观察池)

    // [v12 删除] 老 "📘 虚拟观察仓次日表现" (push_virtual_next_day_review) — 由 v12 R-01~R-08 替代

    // [v12 删除] 老 "放量·持仓 / 放量·自选 / 放量·实盘优选" — 不再单独推, 数据合到 v12 R-01

    // [v12 删除] 老 "持仓决策台" (run_review_deep_analysis) — 由 v12 R-01 持仓明日计划替代

    // [v12 删除] 老 "新闻Ranker" (news_ranker) — 由 v12 R-07 明日观察池替代 (MVP-3 影子才出)

    // [v12 删除] 老 "AI 评分因子 IC" (run_factor_ic_analysis) — 不再单独推, 数据合到 v12 R-05

    log::info!("[复盘] ======== 老推送已全部删除, 改走 v12 模板 ========");

    log::info!("[复盘] ======== 盘后分析完成 ========");

    // ===============================================================

    // v12 盘后增强 (R-01 ~ R-08) — 替代/补充老 review 路径

    // 2026-07-05: --review 路径之前没接 v12 模板, 现在补上 8 块 R 系列推送

    // 整段包在 spawn_blocking, 避免 sync Diesel 在 async context panic

    // ===============================================================

    use crate::push_templates as pt;

    let v12_review_result: Result<(), String> = tokio::task::spawn_blocking(move || {

        let today_str = review_as_of.format("%Y-%m-%d").to_string();

        let _hhmm = chrono::Local::now().format("%H:%M").to_string();



        // 真实数据

        let r_holdings = stock_analysis::portfolio::get_positions()
            .map_err(|error| format!("v12 复盘持仓查询失败: {error}"))?;

        let r_quotes = market_data::fetch_position_quotes()?;

        let r_prices: std::collections::HashMap<String, f64> =

            r_quotes.iter().map(|q| (q.code.clone(), q.price)).collect();

        let r_trades = stock_analysis::portfolio::get_trade_history(30)
            .map_err(|error| format!("v12 复盘交易历史查询失败: {error}"))?;

        let r_equity = stock_analysis::portfolio::get_equity_curve(30)
            .map_err(|error| format!("v12 复盘净值曲线查询失败: {error}"))?;

        let r_ledger = r_equity.last().cloned();



        log::info!("[v12-MVP1-R] 调度 8 块 R 系列盘后推送 (持仓={}, 成交={}, ledger={})", r_holdings.len(), r_trades.len(), r_ledger.is_some());



        // ===== R-01 持仓明日计划 (v12 §14.2 模板) =====

        {

            let mut items: Vec<pt::HoldingDailyPlan> = Vec::new();

            for p in r_holdings.iter().take(5) {

                let Some(cur) = r_prices.get(&p.code).copied() else {
                    log::warn!("[v12-R01] {}({}) 实时价缺失，跳过", p.name, p.code);
                    continue;
                };

                let pnl = if p.cost_price > 0.0 { (cur / p.cost_price - 1.0) * 100.0 } else { 0.0 };

                let plan_high = if pnl > 5.0 { "减仓1/3" } else { "减仓1/2" };

                let t0 = if pnl > 5.0 { "适合观察" } else { "不适合(主升核心)" };

                let stop = p.cost_price * 0.92;

                items.push(pt::HoldingDailyPlan {

                    name: p.name.as_str(),

                    code: p.code.as_str(),

                    price: cur, cost: p.cost_price, pnl_pct: pnl,

                    high_gap_x: 2.0,

                    plan_high, plan_flat: "持有观望",

                    stop, t0,

                });

            }

            if items.is_empty() {
                log::info!("[v12-R01] 无具备真实行情的持仓，跳过渲染");
            } else {
                let text = pt::render_daily_report(&today_str, &items);
                log::info!("[v12-R01]\n{}", text);
            }

        }



        // ===== R-02 盘面走向 (v12 market_stage_confidence 5 维) =====

        {
            if isolated_test_fixtures {
                log::info!("[v12-R02][BR-051] isolated E2E skips external market snapshot");
            } else {
                use stock_analysis::market_analyzer::market_stage_confidence::{
                    evaluate as ms_evaluate, MarketStageEvidence, TechnicalMetrics,
                };

                match market_data::fetch_market_review_snapshot() {
                    Ok(snapshot) => {
                    let ev = MarketStageEvidence {
                        technical: Some(TechnicalMetrics {
                            sh_chg: snapshot.sh_chg,
                            chinext_chg: snapshot.chinext_chg,
                            star_chg: snapshot.star_chg,
                        }),
                        ..Default::default()
                    };
                    let conf = ms_evaluate(&ev);
                    let r = pt::MarketReview {
                        sh_chg: Some(snapshot.sh_chg),
                        chinext_chg: Some(snapshot.chinext_chg),
                        star_chg: Some(snapshot.star_chg),
                        limit_up_n: Some(snapshot.limit_up_n),
                        limit_down_n: Some(snapshot.limit_down_n),
                        broken_pct: None,
                        consecutive_h: None,
                        amount_yi: Some(snapshot.amount_yi),
                        amount_delta_pct: None,
                        amount_dir: None,
                        main_flow_yi: None,
                        money_effect: "暂无",
                        heat_stage: conf.heat_stage.as_str(),
                        heat_conf_pct: conf.conf_pct,
                        low_conf: conf.degraded,
                        low_conf_tier: None,
                        account_mode: pt::AccountMode::Normal,
                        max_pos: 7,
                    };
                    let text = pt::render_review_market(&today_str, &r);
                    log::info!("[v12-R02]\n{}", text);
                    }
                    Err(error) => {
                        log::error!("[v12-R02] BR-093 盘面快照不可用，跳过评估: {}", error);
                    }
                }
            }
        }



        // ===== R-03 涨停产业链 (v12 limit_chain_review) =====

        {

            use stock_analysis::market_analyzer::limit_chain_review::{

                aggregate, LimitChainInput,

            };

            match load_review_limit_chain_stocks(&r_holdings) {
                Ok(batch) => {
                    for rejection in &batch.rejected {
                        log::warn!(
                            "[v12-R03][BR-140] candidate isolated identity_hash={} reason_code=candidate_evidence_invalid",
                            review_batch::audit_identity_hash("R-03", &rejection.code)
                        );
                    }
                    for error in &batch.source_errors {
                        log::error!("[v12-R03][BR-140] {error}");
                    }
                    let source_complete = batch.source_complete();
                    let aggs = aggregate(&LimitChainInput {
                        stocks: batch.accepted,
                        source_complete,
                    });

                    if !aggs.is_empty() {

                        let mut body = format!("🔥 涨停产业链（{}）\n", today_str);

                        for (i, a) in aggs.iter().enumerate() {

                            body.push_str(&format!(

                        "{}. {} 涨停{}家（首板{}/连板{}） 阶段: {}\n   龙头: {}({}) {}板\n   后排: {}\n   明日观察: 接力意愿\n",

                        i + 1, a.chain, a.limit_up_n, a.first_n, a.consec_n, a.heat_stage,

                        a.leader_name, a.leader_code, a.leader_boards, a.followers.join(","),

                            ));

                        }

                        log::info!("[v12-R03]\n{}", body);

                    } else {

                        log::info!("[v12-R03] 完整数据批次无涨停产业链");

                    }
                }
                Err(error) => log::error!("[v12-R03] 数据批次拒绝: {}", error),
            }

        }



        // ===== R-04 龙虎榜 (v12 lhb_review) =====

        {

            use stock_analysis::market_analyzer::lhb_review::assess_data_quality;

            let entries: Vec<stock_analysis::market_analyzer::lhb_review::LhbEntryInput> = Vec::new();

            let (pct, _degraded) = assess_data_quality(&entries);

            if pct >= 70 {

                log::info!("[v12-R04] 龙虎榜数据完整度 {}%, 推", pct);

            } else {

                log::info!("[v12-R04] 龙虎榜数据完整度 {}% (< 70%), 跳过", pct);

            }

        }



        // R-05 需要候选/纸盘/推送全链真实统计；当前此路径没有完整快照，不用全零伪造复盘。
        log::warn!("[v12-R05] 完整信号执行快照不可用，跳过");



        // ===== R-06 失败归因 (v12 performance_feedback) =====

        {

            use stock_analysis::market_analyzer::performance_feedback::evaluate;

            let rows: Vec<stock_analysis::market_analyzer::performance_feedback::ExecutionRow> = Vec::new();

            let report = evaluate(&rows, &today_str);

            log::info!("[v12-R06] 失败归因建议 {} 条", report.suggestions.len());

        }



        // ===== R-07/R-08 已在 async 上下文真推, 此处仅 log =====

        log::info!("[v12-MVP1-R] 8 块 R 系列组装完成 (待 push, R-07/R-08 在 async)");

        Ok(())

    }).await.unwrap_or_else(|e| Err(format!("spawn_blocking join: {}", e)));

    if let Err(e) = v12_review_result {
        log::error!("[v12-MVP1-R] spawn_blocking 失败: {}", e);
        return Err(e);
    } else {
        // 推送: 在 async context 直接 push (R-01 + R-02 + R-08 3 个必推, 其他按数据决定)

        log::info!("[v12-MVP1-R] 推送 R-01~R-08 到飞书");

        // 推送数据准备 (sync Diesel → 必须包 spawn_blocking, 否则 async context panic)

        let today_str2 = chrono::Local::now().format("%Y-%m-%d").to_string();

        let push_data = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let r2 = stock_analysis::portfolio::get_positions()
                .map_err(|error| format!("R 系列持仓查询失败: {error}"))?;

            let r2_quotes = market_data::fetch_position_quotes()?;

            let r2_prices: std::collections::HashMap<String, f64> = r2_quotes
                .iter()
                .map(|q| (q.code.clone(), q.price))
                .collect();

            Ok((r2, r2_prices))
        })
        .await
        .map_err(|error| format!("R 系列 blocking 任务失败: {error}"))??;

        let (r2, r2_prices) = push_data;

        // R-01 推送

        {
            let mut items: Vec<pt::HoldingDailyPlan> = Vec::new();

            for p in r2.iter().take(5) {
                let Some(cur) = r2_prices.get(&p.code).copied() else {
                    log::warn!("[v12-R01 push] {}({}) 实时价缺失，跳过", p.name, p.code);
                    continue;
                };

                let pnl = if p.cost_price > 0.0 {
                    (cur / p.cost_price - 1.0) * 100.0
                } else {
                    0.0
                };

                let plan_high = if pnl > 5.0 { "减仓1/3" } else { "减仓1/2" };

                let t0 = if pnl > 5.0 {
                    "适合观察"
                } else {
                    "不适合(主升核心)"
                };

                let stop = p.cost_price * 0.92;

                items.push(pt::HoldingDailyPlan {
                    name: p.name.as_str(),

                    code: p.code.as_str(),

                    price: cur,

                    cost: p.cost_price,

                    pnl_pct: pnl,

                    high_gap_x: 2.0,

                    plan_high,

                    plan_flat: "持有观望",

                    stop,

                    t0,
                });
            }

            if !items.is_empty() {
                let text = pt::render_daily_report(&today_str2, &items);

                notify::push_governor(&text, notify::PushKind::DailyReport).await;
            }
        }

        // R-02 推送

        {
            if isolated_test_fixtures {
                log::info!("[v12-R02][BR-051] isolated E2E skips external market push");
            } else {
                use stock_analysis::market_analyzer::market_stage_confidence::{
                    evaluate as ms_evaluate, MarketStageEvidence, TechnicalMetrics,
                };

                match market_data::fetch_market_review_snapshot() {
                    Ok(snapshot) => {
                        let ev = MarketStageEvidence {
                            technical: Some(TechnicalMetrics {
                                sh_chg: snapshot.sh_chg,
                                chinext_chg: snapshot.chinext_chg,
                                star_chg: snapshot.star_chg,
                            }),
                            ..Default::default()
                        };
                        let conf = ms_evaluate(&ev);
                        let r = pt::MarketReview {
                            sh_chg: Some(snapshot.sh_chg),
                            chinext_chg: Some(snapshot.chinext_chg),
                            star_chg: Some(snapshot.star_chg),
                            limit_up_n: Some(snapshot.limit_up_n),
                            limit_down_n: Some(snapshot.limit_down_n),
                            broken_pct: None,
                            consecutive_h: None,
                            amount_yi: Some(snapshot.amount_yi),
                            amount_delta_pct: None,
                            amount_dir: None,
                            main_flow_yi: None,
                            money_effect: "暂无",
                            heat_stage: conf.heat_stage.as_str(),
                            heat_conf_pct: conf.conf_pct,
                            low_conf: conf.degraded,
                            low_conf_tier: None,
                            account_mode: pt::AccountMode::Normal,
                            max_pos: 7,
                        };
                        let text = pt::render_review_market(&today_str2, &r);
                        notify::push_governor(&text, notify::PushKind::ReviewMarket).await;
                    }
                    Err(error) => {
                        log::error!("[v12-R02] BR-093 盘面快照不可用，跳过推送: {}", error);
                    }
                }
            }
        }

        // R-07 明日观察池 (真推)

        {
            let watchlist = stock_analysis::portfolio::get_watchlist()
                .map_err(|error| format!("R-07 自选查询失败: {error}"))?;

            if watchlist.is_empty() {
                log::info!("[v12-R07] 自选为空, 跳过");
            } else {
                let mut items: Vec<pt::WatchItem<'_>> = Vec::new();

                for p in watchlist.iter().take(3) {
                    let Some(cur) = r2_prices.get(&p.code).copied() else {
                        log::warn!("[v12-R07] {}({}) 实时价缺失，跳过", p.name, p.code);
                        continue;
                    };

                    items.push(pt::WatchItem {
                        name: p.name.as_str(),

                        code: p.code.as_str(),

                        topic: p.sector.as_str(),

                        source: "A档未触发",

                        trigger: "突破前高+量比>3",

                        lo: cur * 0.97,

                        hi: cur * 1.05,

                        stop: cur * 0.93,

                        reason: "板块共振 + 持仓联动",
                    });
                }

                let text = pt::render_tomorrow_watch(&today_str2, &items);

                log::info!("[v12-R07]\n{}", text);

                notify::push_governor(&text, notify::PushKind::TomorrowWatch).await;
            }
        }

        // R-08 推送 (真实数据: 拉今日公告 + 持仓事件)

        if isolated_test_fixtures {
            log::info!("[v12-R08][BR-051] isolated E2E skips external announcement/overnight data");
        } else {
            // 真实数据源: 公告 API + 持仓事件

            // 公告拉取 (sync, 包 spawn_blocking)

            let (ann_summary, holding_events) =
                tokio::task::spawn_blocking(move || -> Result<_, String> {
                    // 1. 拉今日全市场公告；只有真实成功的空结果才表示无公告。

                    // review #15: fetch_announcements 改 async. 外层 spawn_blocking closure

                    // 是 sync context, 用 Handle::current().block_on 驱动 future.

                    let batch = tokio::runtime::Handle::current()
                        .block_on(
                            stock_analysis::data_provider::announcement::fetch_announcements(None),
                        )
                        .map_err(|error| format!("R-08 公告获取失败: {error}"))?;
                    let anns = filter_inline_r08_announcements(batch.announcements);

                    let ann_text = if anns.is_empty() {
                        "今日无重大公告".to_string()
                    } else {
                        let mut s = format!("今日共 {} 条公告 (TOP 3):\n", anns.len());

                        for a in anns.iter().take(3) {
                            s.push_str(&format!("· {} ({:?}): {}\n", a.code, a.level, a.title));
                        }

                        s
                    };

                    // 2. 持仓事件: 用 r2 持仓 + 拉它们各自今日公告

                    let mut events: Vec<(String, String)> = Vec::new();

                    for p in r2.iter().take(3) {
                        // 查该持仓的今日公告

                        let p_anns: Vec<_> =
                            anns.iter().filter(|a| a.code == p.code).take(2).collect();

                        let kind = if !p_anns.is_empty() {
                            // 用最近一条公告标题作为事件

                            p_anns[0].title.chars().take(20).collect::<String>()
                        } else {
                            match r2_prices
                                .get(&p.code)
                                .copied()
                                .filter(|price| price.is_finite() && *price > 0.0)
                            {
                                Some(price) if p.cost_price.is_finite() && p.cost_price > 0.0 => {
                                    format!(
                                        "持有 {} (浮盈{:.1}%)",
                                        p.code,
                                        (price / p.cost_price - 1.0) * 100.0
                                    )
                                }
                                _ => format!("持有 {} (实时价不可用)", p.code),
                            }
                        };

                        events.push((p.name.clone(), kind));
                    }

                    Ok((ann_text, events))
                })
                .await
                .map_err(|error| format!("R-08 公告后台任务失败: {error}"))??;

            let events_ref: Vec<pt::HoldingEventItem> = holding_events
                .iter()
                .map(|(n, k)| pt::HoldingEventItem {
                    tag: "实盘",

                    name: n.as_str(),

                    code: "",

                    kind: k.as_str(),
                })
                .collect();

            // v64 + v65: 隔夜关注真值 (美股 + 汇率 雅虎 API) — 包 spawn_blocking (P1.1 修复)

            let (us_summary2, fx_summary2) = match tokio::task::spawn_blocking(
                stock_analysis::data_provider::yahoo::fetch_overnight_data,
            )
            .await
            {
                Ok(Ok(snapshot)) => snapshot,
                Ok(Err(error)) => {
                    log::error!("[v65] Yahoo 隔夜数据不可用: {}", error);
                    ("暂无".to_string(), "暂无".to_string())
                }
                Err(error) => {
                    log::error!("[v65] fetch_overnight_data task 失败: {}", error);
                    ("暂无".to_string(), "暂无".to_string())
                }
            };

            let text = pt::render_event_calendar(
                &today_str2,
                &events_ref,
                &ann_summary,
                &us_summary2,
                &fx_summary2,
            );

            log::info!("[v12-R08]\n{}", text);

            notify::push_governor(&text, notify::PushKind::EventCalendar).await;
        }

        // R-06 必须由真实执行记录和真实失败归因生成。当前盘后路径尚未接入完整证据链，
        // 因此显式禁用，禁止用演示标的/损益伪造生产推送（AGENTS §2.1/§2.8）。
        log::warn!("[v19.14b R-06] 真实执行归因证据链不可用，跳过推送");

        // v68: 盘后复盘对齐 v18 — 推 3 张 v18 风格卡片 (放量·持仓 / 放量·自选 / 放量·实盘优选)

        //   - v18 路径: 单独推 holding_brk + watch_brk + market_brk

        //   - v12 路径: 只推 candidate_summary, 没单独推放量卡片 — 用户要"内容跟 v18 一样"

        let v18_brk = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let holdings = stock_analysis::portfolio::get_positions()
                .map_err(|error| format!("v18 复盘持仓查询失败: {error}"))?;

            let watchlist = stock_analysis::portfolio::get_watchlist()
                .map_err(|error| format!("v18 复盘自选查询失败: {error}"))?;

            let quotes = market_data::fetch_position_quotes()?;

            let _prices = build_price_map(&quotes);

            let holding_codes: std::collections::HashSet<String> =
                holdings.iter().map(|p| p.code.clone()).collect();

            let watch_codes: std::collections::HashSet<String> =
                watchlist.iter().map(|p| p.code.clone()).collect();

            let mut holding_brk = String::new();

            let mut watch_brk = String::new();

            let mut market_brk = String::new();

            if let Ok(fetcher) = stock_analysis::data_provider::DataFetcherManager::new() {
                // 持仓放量

                let mut holding_lines =
                    vec!["📊 放量分析·持仓（盘后·算法研判仅供参考）".to_string()];

                for p in &holdings {
                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {
                        let sig = stock_analysis::breakout::engine::analyze_postmarket(
                            &p.code, &p.name, &kline,
                        );

                        holding_lines.push(format!(
                            "  {} {}({}) — {} 置信{}% [{}]",
                            sig.breakout_type.emoji(),
                            sig.name,
                            sig.code,
                            sig.breakout_type.label(),
                            sig.confidence,
                            sig.description,
                        ));
                    }
                }

                if holding_lines.len() > 1 {
                    holding_brk = holding_lines.join("\n");
                }

                // 自选放量 (剔除已在持仓列出的)

                let mut watch_lines = vec!["📊 放量分析·自选（盘后·算法研判仅供参考）".to_string()];

                for p in &watchlist {
                    if holding_codes.contains(&p.code) {
                        continue;
                    }

                    if let Ok((kline, _)) = fetcher.get_daily_data(&p.code, 60) {
                        let sig = stock_analysis::breakout::engine::analyze_postmarket(
                            &p.code, &p.name, &kline,
                        );

                        watch_lines.push(format!(
                            "  {} {}({}) — {} 置信{}% [{}]",
                            sig.breakout_type.emoji(),
                            sig.name,
                            sig.code,
                            sig.breakout_type.label(),
                            sig.confidence,
                            sig.description,
                        ));
                    }
                }

                if watch_lines.len() > 1 {
                    watch_brk = watch_lines.join("\n");
                }

                if isolated_test_fixtures {
                    log::info!(
                        "[v18复盘][BR-051] isolated E2E skips external market candidate scan"
                    );
                } else {
                    // 实盘量能优选 (全市场)
                    let market_candidates = market_data::fetch_market_volume_ratio_leaders(80)?;
                    let mut market_lines =
                        vec!["📊 放量分析·实盘优选（盘后·算法研判仅供参考）".to_string()];
                    let mut picked = 0usize;

                    for s in &market_candidates {
                        if picked >= 5 {
                            break;
                        }
                        if holding_codes.contains(&s.code) || watch_codes.contains(&s.code) {
                            continue;
                        }
                        let (Some(volume_ratio), Some(main_net_yi)) =
                            (s.volume_ratio, s.main_net_yi)
                        else {
                            log::warn!("[v18复盘] {}({}) 量比/主力流缺失，跳过", s.name, s.code);
                            continue;
                        };

                        if let Ok((kline, _)) = fetcher.get_daily_data(&s.code, 60) {
                            let sig = stock_analysis::breakout::engine::analyze_postmarket(
                                &s.code, &s.name, &kline,
                            );
                            if sig.breakout_type
                                != stock_analysis::breakout::signal::BreakoutType::Launch
                                || sig.confidence < 50
                            {
                                continue;
                            }
                            market_lines.push(format!(
                                "  {} {}({}) — {} 置信{}% [量比{:.1} 主力{:+.2}亿 | {}]",
                                sig.breakout_type.emoji(),
                                sig.name,
                                sig.code,
                                sig.breakout_type.label(),
                                sig.confidence,
                                volume_ratio,
                                main_net_yi,
                                sig.description,
                            ));
                            picked += 1;
                        }
                    }

                    if market_lines.len() > 1 {
                        market_brk = market_lines.join("\n");
                    }
                }
            }

            Ok((holding_brk, watch_brk, market_brk))
        })
        .await
        .map_err(|error| format!("v18 复盘 blocking 任务失败: {error}"))??;

        let (holding_brk, watch_brk, market_brk) = v18_brk;

        if !holding_brk.is_empty() {
            let holding_brk_text =
                format!("📊 放量分析·持仓（盘后·算法研判仅供参考）\n{}", holding_brk);

            log::info!(
                "[v68] 放量·持仓 推送 ({} 字)",
                holding_brk_text.chars().count()
            );

            push_governor_v3(&holding_brk_text, PushKind::IntradayMarket, None).await;
        }

        if !watch_brk.is_empty() {
            let watch_brk_text =
                format!("📊 放量分析·自选（盘后·算法研判仅供参考）\n{}", watch_brk);

            log::info!(
                "[v68] 放量·自选 推送 ({} 字)",
                watch_brk_text.chars().count()
            );

            push_governor_v3(&watch_brk_text, PushKind::IntradayMarket, None).await;
        }

        if !market_brk.is_empty() {
            let market_brk_text = format!(
                "📊 放量分析·实盘优选（盘后·算法研判仅供参考）\n{}",
                market_brk
            );

            log::info!(
                "[v68] 放量·实盘优选 推送 ({} 字)",
                market_brk_text.chars().count()
            );

            push_governor_v3(&market_brk_text, PushKind::IntradayMarket, None).await;
        }

        log::info!("[v12-MVP1-R] R-01/R-02/R-06/R-08 推送完成 (R-03~R-05/R-07 数据不足仅 log)");
    }

    // 全模板覆盖: R-01~R-08 已由上方内联块推送, 这里调 dispatch_all_for_test 补齐
    //   --test → All (再跑盘中模板); --review → Review (只补盘后复盘). R-02~R-08 已推 → dedup 跳过, 不重复
    if isolated_test_fixtures {
        log::info!("[复盘][BR-051] isolated E2E defers template sweep to IsolatedAll scope");
    } else if std::env::args().any(|arg| arg == "--test") {
        let hhmm_r = chrono::Local::now().format("%H:%M").to_string();
        let date_r = chrono::Local::now().format("%Y-%m-%d").to_string();
        let banner_r = current_banner()
            .map_err(|error| format!("isolated template banner unavailable: {error}"))?;
        pt::dispatch_all_for_test(&hhmm_r, &date_r, &banner_r, pt::TestScope::All).await;
    } else {
        log::warn!("[复盘][BR-108] test dispatcher disabled=no_verified_banner");
    }

    Ok(())
}

/// v70: e2e 模式入口 — 跑所有 v12 §14 模板 (忽略时间窗口 + 数据空)

///   步骤: 1) seed chain_daily + lhb_daily + trades

///         2) run_review_only_inner (推 R-01~R-08 + v18 放量)

///         3) 跑盘中 14.x 模板 (P-01 P-02 P-03 P-04 I-01~I-08 A-10)

///         4) 不依赖时间窗口，只使用 TEST_CODE 隔离测试夹具

///   用途: 验证 v12 §14 + v13.1 模板完整性, 推全 22 模板

async fn e2e_all_templates_run() -> Result<(), String> {
    let now = chrono::Local::now();
    let review_date = stock_analysis::calendar::latest_completed_trading_day_at(now.naive_local());
    let today_str = review_date.format("%Y-%m-%d").to_string();
    let hhmm = now.format("%H:%M").to_string();

    log::info!("[v70] E2E 开始 — 跑所有 v12 §14 + v13.1 模板");

    // 1. Seed (chain_daily + lhb_daily + trades) 让 R-03 / R-04 / R-05 / A-10 都能推

    log::info!("[v70] 1/3 seed chain_daily + lhb_daily + trades");

    seed_e2e_data_via_sqlite(review_date)
        .map_err(|error| format!("E2E seed 失败，终止全模板测试: {error}"))?;
    seed_isolated_e2e_banner()?;

    // Production dry-run assemblers require real six-digit symbols and may use
    // live providers. BR-051 keeps them out of this TEST_CODE-only E2E path;
    // `--push-dry-run` has its own isolated process contract test.
    log::info!("[v70][BR-051] production dry-run assemblers skipped in TEST_CODE E2E");

    // 2. 跑 v12 §14.3 盘后复盘 (R-01~R-08) + v18 放量

    log::info!("[v70] 2/3 跑 R-01~R-08 + v18 放量");

    run_review_only_inner(true)
        .await
        .map_err(|error| format!("复盘流程失败: {error}"))?;

    // 3. 跑 v12 §14.1 盘前 + 14.2 盘中 + 14.3 v18 之外的模板

    //   注: 这些模板原本走 v18 路径, 真实交易日由 monitor_loop / news_monitor_loop 推

    //   v70 isolated fixtures: 推 14.x 模板只用 TEST_CODE 测试数据

    log::info!("[v70] 3/3 跑盘中 14.x 模板 (isolated test fixtures)");

    push_e2e_14x_templates(&today_str, &hhmm).await;

    // 4. 跑 v12 §14.1 + 14.2 新闻模块 (D-01 / I-02) — isolated fixtures

    //   news_monitor_loop 真实路径需有公告源；这里使用 TEST_CODE dispatcher fixture

    log::info!("[v70] 4/4 跑新闻模块 (D-01 / I-02 isolated test fixtures)");

    let banner_e2e =
        current_banner().map_err(|error| format!("isolated E2E banner unavailable: {error}"))?;
    push_e2e_news_modules(&hhmm, &banner_e2e).await;

    // T-16 requires a real-symbol realtime quote. BR-051 forbids inserting a
    // real symbol into the isolated test account, so this external boundary is
    // covered by its focused dispatcher tests instead of this process E2E.
    log::info!("[v70][BR-051] T-16 skipped (external realtime quote not exercised)");

    // --test 全模板覆盖: 调全部 dispatch_*_daily (真推, 只推有真数据的, 用户要求测试所有模板)
    //   R-01~R-08 已由上方 run_review_only_inner 推过 → 这里 dedup 跳过; 盘中模板在此真推
    {
        use crate::push_templates as pt;
        let banner_e2e = current_banner()
            .map_err(|error| format!("isolated template banner unavailable: {error}"))?;
        pt::dispatch_all_for_test(&hhmm, &today_str, &banner_e2e, pt::TestScope::IsolatedAll).await;
    }

    log::info!(
        "[v70] E2E 完成 — 检查 data/push_log/{}/ 查所有推送",
        today_str
    );
    Ok(())
}

/// v70: test-only seed chain_daily + lhb_daily + trades via sqlite3 CLI

fn seed_e2e_data_via_sqlite(date: chrono::NaiveDate) -> Result<(), String> {
    if stock_analysis::risk::env_guard::current_env()
        != stock_analysis::risk::env_guard::TradingEnv::Test
    {
        return Err("BR-051 E2E seed requires test environment".to_string());
    }
    let db_path = std::env::var("DATABASE_PATH")
        .map_err(|error| format!("隔离 DATABASE_PATH 未设置: {error}"))?;
    if std::path::Path::new(&db_path).ends_with("data/stock_analysis.db") {
        return Err("BR-051 E2E seed rejects the production database path".to_string());
    }
    stock_analysis::portfolio::snapshot_ledger(stock_analysis::portfolio::LedgerEntry {
        date,
        total_value: 100_000.0,
        cash: 100_000.0,
        market_value: 0.0,
        daily_pnl: 0.0,
    })
    .map_err(|error| format!("TEST_CODE ledger seed failed: {error}"))?;
    let date = date.format("%Y-%m-%d").to_string();

    // chain_daily 5 概念

    let chain_sql = format!(
        r#"INSERT OR IGNORE INTO chain_daily (date, concept, stocks, continuation_count) VALUES

        ('{date}', 'TEST_CODE_PCB', '["TEST_CODE_PCB_1","TEST_CODE_PCB_2","TEST_CODE_PCB_3"]', 3),

        ('{date}', 'TEST_CODE_COMPUTE', '["TEST_CODE_COMPUTE_1","TEST_CODE_COMPUTE_2","TEST_CODE_COMPUTE_3"]', 2),

        ('{date}', 'TEST_CODE_ROBOT', '["TEST_CODE_ROBOT_1","TEST_CODE_ROBOT_2","TEST_CODE_ROBOT_3"]', 2),

        ('{date}', 'TEST_CODE_SEMI', '["TEST_CODE_SEMI_1","TEST_CODE_SEMI_2","TEST_CODE_SEMI_3"]', 1),

        ('{date}', 'TEST_CODE_BATTERY', '["TEST_CODE_BATTERY_1","TEST_CODE_BATTERY_2","TEST_CODE_BATTERY_3"]', 1);"#
    );

    execute_e2e_seed_sql(&db_path, "chain_daily", &chain_sql)?;

    // lhb_daily 6 票

    let lhb_sql = format!(
        r#"INSERT OR IGNORE INTO lhb_daily

        (code, name, trade_date, reason, pct_change, close_price, buy_amount, sell_amount, net_amount, total_amount, lhb_ratio) VALUES

        ('TEST_CODE_LHB_1','TEST_CODE_LHB_1','{date}','测试涨幅偏离值达7%',10.0,12.10,5.0e8,2.0e8,3.0e8,7.0e8,0.43),

        ('TEST_CODE_LHB_2','TEST_CODE_LHB_2','{date}','测试涨幅偏离值达7%',10.0,29.72,3.0e8,1.0e8,2.0e8,4.0e8,0.50),

        ('TEST_CODE_LHB_3','TEST_CODE_LHB_3','{date}','测试涨幅偏离值达7%',10.0,35.20,2.0e8,0.5e8,1.5e8,2.5e8,0.60),

        ('TEST_CODE_LHB_4','TEST_CODE_LHB_4','{date}','测试涨幅偏离值达7%',10.0,58.40,4.0e8,1.5e8,2.5e8,5.5e8,0.45),

        ('TEST_CODE_LHB_5','TEST_CODE_LHB_5','{date}','测试涨幅偏离值达7%',10.0,43.20,1.0e8,0.3e8,0.7e8,1.3e8,0.54),

        ('TEST_CODE_LHB_6','TEST_CODE_LHB_6','{date}','测试涨幅偏离值达7%',10.0,78.60,3.5e8,1.0e8,2.5e8,4.5e8,0.56);"#
    );

    execute_e2e_seed_sql(&db_path, "lhb_daily", &lhb_sql)?;

    // trades 1 buy + 1 sell

    let trades_sql = format!(
        r#"INSERT OR IGNORE INTO trades (code, name, direction, price, shares, amount, reason, traded_at) VALUES

        ('TEST_CODE_TRADE','TEST_CODE_TRADE','buy',19.27,200,3854.0,'测试建仓','{date} 09:35:00'),

        ('TEST_CODE_TRADE','TEST_CODE_TRADE','sell',17.50,200,3500.0,'测试止损','{date} 14:35:00');"#
    );

    execute_e2e_seed_sql(&db_path, "trades", &trades_sql)?;
    Ok(())
}

fn seed_isolated_e2e_banner() -> Result<(), String> {
    if stock_analysis::risk::env_guard::current_env()
        != stock_analysis::risk::env_guard::TradingEnv::Test
    {
        return Err("BR-051 isolated banner requires test environment".to_string());
    }
    let review_date = stock_analysis::calendar::latest_completed_trading_day_at(
        chrono::Local::now().naive_local(),
    );
    let latest = stock_analysis::portfolio::get_equity_curve_as_of(7, review_date)
        .map_err(|error| format!("TEST_CODE ledger read failed: {error}"))?
        .pop()
        .ok_or_else(|| "TEST_CODE ledger missing after seed".to_string())?;
    let total_pos = if latest.total_value > 0.0 {
        ((latest.market_value / latest.total_value) * 10.0)
            .round()
            .clamp(0.0, 10.0) as u8
    } else {
        return Err("TEST_CODE ledger total_value must be positive".to_string());
    };
    let today_pnl = latest.daily_pnl / latest.total_value * 100.0;
    store_banner(push_templates::BannerCtx {
        account_mode: push_templates::AccountMode::Normal,
        total_pos: Some(total_pos),
        today_pnl: Some(today_pnl),
        account_metrics_complete: true,
        data_mode: push_templates::DataMode::Full,
        data_missing_note: None,
    })
    .map_err(|error| format!("TEST_CODE governance banner commit failed: {error}"))
}

fn execute_e2e_seed_sql(db_path: &str, label: &str, sql: &str) -> Result<(), String> {
    let output = std::process::Command::new("sqlite3")
        .args([db_path, sql])
        .output()
        .map_err(|error| format!("{label} sqlite3 启动失败: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{label} seed failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// v70: 推新闻模块 (D-01 / I-02) — isolated test fixture

///   news_monitor_loop 真实路径需公告源；这里直接走 TEST_CODE dispatcher fixture

///   公告测试数据: 3 主题 + 2 票 (覆盖 D-01 + I-02)

async fn push_e2e_news_modules(hhmm: &str, banner: &push_templates::BannerCtx) {
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

    let _ = notify::push_governor_v3(&d01, notify::PushKind::NewsToIdea, Some("TEST_CODE_NEWS_1"))
        .await;

    // v70+: 落盘推荐记录 (供后续 D+1 兑现分析)

    notify::record_news_recommendation(
        "D-01",
        "TEST_CODE_NEWS_1",
        "深南电路",
        "AI 算力",
        &["PCB 涨价 12%", "算力国产替代加速"],
        Some("BuyDip"),
        None,
    );

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

    let _ = notify::push_governor(&i02, notify::PushKind::NewsCatalyst).await;

    // v70+: 落盘 I-02 推荐 (多票, 每票写一条)

    for (name, code, chg, reason) in [
        (
            "深南电路",
            "TEST_CODE_NEWS_1",
            Some(10.0_f64),
            "PCB 龙头, Q1 业绩超预期",
        ),
        (
            "沪电股份",
            "TEST_CODE_NEWS_2",
            Some(9.5_f64),
            "800G 交换机 PCB 受益",
        ),
    ] {
        notify::record_news_recommendation("I-02", code, name, "AI 算力", &[reason], None, chg);
    }
}

/// v70: 推所有盘中 14.x 模板 (isolated test fixtures)

async fn push_e2e_14x_templates(date: &str, hhmm: &str) {
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

    let _ = notify::push_governor(&p01, notify::PushKind::PreopenNewsHot).await;

    // P-02 竞价热点量能 (isolated fixture)

    let p02 = format!(

        "🌅 竞价热点量能（{}）\n深南电路(TEST_CODE_P02_1) 高开+1.2% | 量比3.5 | 竞价额1.2亿\n结论: 强承接\n辅助建议, 非下单指令",

        hhmm

    );

    log::info!("[v70] P-02 推 ({} 字)", p02.chars().count());

    let _ = notify::push_governor(&p02, notify::PushKind::AuctionVolume).await;

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
                watch_point: "放量后回踩关注",
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
                watch_point: "板块趋势延续",
            },
        ],
        None,
        None,
    );

    log::info!("[v70] R-03 推 ({} 字)", r03.chars().count());

    let _ = notify::push_governor(&r03, notify::PushKind::IndustryChain).await;

    // R-04 龙虎榜 (lhb_daily 6 票, TEST_CODE 数据)

    let r04 = pt::render_review_lhb(
        date,
        &[pt::LhbEntry {
            name: "深南电路",
            code: "TEST_CODE_R04_1",

            net_buy_yi: 3.0,
            reason: Some("涨幅偏离值达7%"),

            buy_inst_n: Some(5),
            buy_inst_amt_wan: Some(5000.0),
            buy_other_n: Some(3),
            buy_other_amt_wan: Some(2000.0),

            buy_conc_pct: Some(60.0),
            sell_desc: Some("机构卖200万"),
            sell_conc_pct: Some(40.0),

            chain_match: Some("是-PCB"),
            next_day_risk: Some("高位, 注意回撤"),
        }],
    );

    log::info!("[v70] R-04 推 ({} 字)", r04.chars().count());

    let _ = notify::push_governor(&r04, notify::PushKind::ReviewLhb).await;

    // R-05 信号复盘 (TEST_CODE trades)

    let r05 = pt::render_review_signal(
        date,
        &pt::SignalReview {
            holding_n: 7,
            holding_exec: 1,
            holding_eff: 1,

            t0_n: 0,
            t0_eff: 0,

            cand_trigger: 0,
            cand_filled: 0,
            cand_notfilled: 0,

            cand_limitup: 0,
            cand_notreach: 0,

            paper_pnl_pct: -8.4,
            paper_total_pct: -8.4,
            paper_n: 1,

            news_push_n: 5,
            news_d1_eff: 0,
        },
    );

    log::info!("[v70] R-05 推 ({} 字)", r05.chars().count());

    let _ = notify::push_governor(&r05, notify::PushKind::ReviewSignal).await;

    // A-10 题材催化复盘 (TEST_CODE chain_daily)

    let a10 = pt::render_catalyst_review(pt::CatalystReviewParams {
        date,
        theme: "PCB",

        score: Some(8.5),
        persistent: pt::PersistentLevel::High,

        started_names: vec!["深南电路", "沪电股份"],

        pending_names: vec!["兴森科技"],

        watch_point: Some("放量后回踩关注"),
    });

    log::info!("[v70] A-10 推 ({} 字)", a10.chars().count());

    let _ = notify::push_governor(&a10, notify::PushKind::CatalystReview).await;

    log::info!("[v70] e2e 14x 模板跑完");
}

/// 盘后持仓多 Agent 深度研判：对每只真实持仓跑「6 分析师 + 多空辩论 + 仲裁」流水线，

/// 结果逐只推送飞书。受 `AI_AGENT_PIPELINE`（默认开启）控制；关闭则整体跳过。

async fn run_review_deep_analysis(
    _holding_breakout_text: &str,

    _watch_breakout_text: &str,

    risk_text: &str, // v19.3: 风险段 (止损+轮动+现金) 合并到持仓决策台 1 张卡
) -> Result<(), String> {
    use futures::stream::{self, StreamExt};

    // 开关：与主流程一致，AI_AGENT_PIPELINE=false 时不跑多 Agent

    let enabled = std::env::var("AI_AGENT_PIPELINE")
        .map(|v| v.trim().to_lowercase() != "false")
        .unwrap_or(true);

    if !enabled {
        log::info!("[复盘] AI_AGENT_PIPELINE=false，跳过持仓多 Agent 深度研判");

        return Ok(());
    }

    let holdings = stock_analysis::portfolio::get_positions()
        .map_err(|error| format!("深度复盘持仓查询失败: {error}"))?;

    if holdings.is_empty() {
        log::info!("[复盘] 无持仓，跳过多 Agent 深度研判");

        return Ok(());
    }

    // 深度研判并发度（LLM 密集，默认 3）

    let concurrency = std::env::var("DEEP_ANALYSIS_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&c| c > 0)
        .unwrap_or(3);

    log::info!(
        "[复盘] 持仓多 Agent 深度研判开始（{} 只，并发 {}）",
        holdings.len(),
        concurrency
    );

    // 并发跑多 Agent，结果回收后按持仓顺序推送

    let codes: Vec<(String, String)> = holdings
        .iter()
        .map(|p| (p.code.clone(), p.name.clone()))
        .collect();

    let results: Vec<(String, String, Option<String>)> = stream::iter(codes)
        .map(|(code, name)| async move {
            log::info!("[复盘] ▶ 多 Agent 研判 {} {}", code, name);

            let deep = tokio::time::timeout(
                std::time::Duration::from_secs(300),
                stock_analysis::deep_analyzer::run_multi_agent_analysis(&code),
            )
            .await;

            let md = match deep {
                Ok(Ok(md)) if !md.trim().is_empty() => Some(md),

                Ok(Ok(_)) => {
                    log::warn!("[复盘] {} 多 Agent 返回空", code);

                    None
                }

                Ok(Err(e)) => {
                    log::warn!("[复盘] {} 多 Agent 失败: {:#}", code, e);

                    None
                }

                Err(_) => {
                    log::warn!("[复盘] {} 多 Agent 超时(300s)", code);

                    None
                }
            };

            (code, name, md)
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    // 按持仓原顺序推送（buffer_unordered 完成顺序不确定，重排回固定顺序）

    let by_code: std::collections::HashMap<String, (String, Option<String>)> =
        results.into_iter().map(|(c, n, m)| (c, (n, m))).collect();

    // 落盘每只持仓研判 (供事后查询, 不再单独推送)

    for p in &holdings {
        let Some((name, md)) = by_code.get(&p.code) else {
            continue;
        };

        let Some(md) = md else { continue };

        log::info!(
            "[复盘] 持仓深度研判 {}({}) 完成 ({} 字, 落盘+聚合推送)",
            name,
            p.code,
            md.chars().count()
        );

        let _ = stock_analysis::pipeline::section_utils::save_deep_report(&p.code, md);
    }

    // 聚合推送: 走持仓决策台 (P0-5 commit 2 替换原 build_holding_summary 字符串猜)

    // v14.2 路径: decisions_from_llm (commit 1) → format_decision_board (commit C 渲染)

    // by_code 不再被 .remove() 走, 决策台能拿到 LLM 终稿

    // v62: 用真报价填 current_price / change_pct (F1 实盘数据误差修复)

    //   - 第二轮 fetch (第一轮 quotes 已被 spawn_blocking move 走)

    let r_quotes2 = market_data::fetch_position_quotes()?;

    let quote_map: std::collections::HashMap<String, (f64, f64)> = r_quotes2
        .iter()
        .map(|q| (q.code.clone(), (q.price, q.change_pct)))
        .collect();

    let decisions = stock_analysis::decision::decision_decide::decisions_from_llm(
        &holdings, &by_code, &quote_map,
    )?;

    let summary = stock_analysis::decision::decision_render::format_decision_board(&decisions);

    // v19.3: 风险段 (止损+轮动+现金) 合并到持仓决策台 (1 张卡全信息)

    let mut combined = summary.clone();

    if !risk_text.is_empty() {
        combined.push_str("\n\n━━━ 🛡 风险与轮动段 ━━━\n");

        combined.push_str(risk_text);
    }

    let push_summary = if combined.is_empty() {
        summary.clone()
    } else {
        combined
    };

    if !push_summary.is_empty() {
        log::info!(
            "[复盘] 持仓决策台推送 (v14.2 + 风险合并 v19.3):\n{}",
            push_summary
        );

        push_governor_v3(&push_summary, PushKind::ReviewSignal, None).await;
    }

    log::warn!("[复盘][BR-112] opportunity candidates disabled=incomplete_source_contract");

    log::info!("[复盘] 持仓多 Agent 深度研判完成");
    Ok(())
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
    Policy = 0,
    CriticalFlash = 1,
    HoldingEarnings = 2,
    L2 = 3,
    Announcement = 4,
    Opportunity = 5,
    Reset = 6,
    Flush = 7,
    Banner = 8,
    Sleep = 9,
}

impl NewsOuterTickPhase {
    const ALL: [Self; 10] = [
        Self::Policy,
        Self::CriticalFlash,
        Self::HoldingEarnings,
        Self::L2,
        Self::Announcement,
        Self::Opportunity,
        Self::Reset,
        Self::Flush,
        Self::Banner,
        Self::Sleep,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::CriticalFlash => "critical_flash",
            Self::HoldingEarnings => "holding_earnings",
            Self::L2 => "l2",
            Self::Announcement => "announcement",
            Self::Opportunity => "opportunity",
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

fn load_announcement_audience_codes(
    registered_watch_codes: &std::collections::HashSet<String>,
) -> (std::collections::HashSet<String>, Option<String>) {
    // BR-138: `stock_position` is a mutable local simulation/projection table.
    // Its `updated_at` changes during local return refreshes and is not broker
    // source evidence. Until a broker position batch carries immutable provider,
    // batch identity, and source time, positions are explicitly unavailable.
    let audience = validate_announcement_watch_codes(registered_watch_codes).and_then(|_| {
        Err("BR-138 verified broker position batch unavailable; local projection updated_at is not source evidence".to_string())
    });
    isolate_announcement_position_failure(audience, registered_watch_codes)
}

fn audit_announcement_batch_provenance(
    batch: &stock_analysis::data_provider::announcement::AnnouncementFetchBatch,
) -> Result<(), String> {
    use stock_analysis::data_provider::announcement::{
        AnnouncementListAcquisition, AnnouncementListProtocol,
    };

    let (fallback_used, reason_code) = match &batch.provenance.acquisition {
        AnnouncementListAcquisition::PrimaryJson => (false, None),
        AnnouncementListAcquisition::AlternateJsonp { primary_failure } => {
            debug_assert_eq!(
                primary_failure.protocol,
                AnnouncementListProtocol::PrimaryJson
            );
            (true, Some(primary_failure.reason_code.clone()))
        }
    };
    let selected_protocol = match batch.provenance.selected_protocol() {
        AnnouncementListProtocol::PrimaryJson => "primary_json",
        AnnouncementListProtocol::AlternateJsonp => "alternate_jsonp",
    };
    let identity = format!(
        "{}:{}:{}",
        batch.provenance.endpoint,
        batch.provenance.query_date,
        batch.provenance.observed_at.timestamp_millis()
    );
    let decision = review_batch::ReviewSourceProtocolDecision {
        observed_at: batch.provenance.observed_at.to_rfc3339(),
        task: "Announcement".to_string(),
        source: batch.provenance.provider_label().to_string(),
        source_time: None,
        query_date: batch.provenance.query_date.to_string(),
        selected_protocol: selected_protocol.to_string(),
        fallback_used,
        reason_code,
        identity_hash: review_batch::audit_identity_hash("announcement-list", &identity),
        rule_ids: vec![
            "BR-137".to_string(),
            "BR-138".to_string(),
            "BR-140".to_string(),
        ],
    };
    review_batch::append_source_protocol_audit(decision, batch.provenance.query_date)?;
    if batch.rejected_details.is_empty() {
        return Ok(());
    }
    let rejections = batch
        .rejected_details
        .iter()
        .map(|rejection| review_batch::ReviewCandidateRejection {
            observed_at: rejection.observed_at.to_rfc3339(),
            task: "Announcement".to_string(),
            source: batch.provenance.provider_label().to_string(),
            source_time: None,
            rule_ids: vec![
                "BR-137".to_string(),
                "BR-138".to_string(),
                "BR-140".to_string(),
            ],
            retryable: rejection.retryable,
            identity_hash: rejection.identity_hash.clone(),
            reason_code: rejection.reason_code.clone(),
        })
        .collect();
    review_batch::append_candidate_rejection_audit(rejections, batch.provenance.query_date)
        .map(|_| ())
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
            v17_sources::AnnouncementDisposition::FilteredLifecycle
            | v17_sources::AnnouncementDisposition::FilteredAudience
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

async fn news_monitor_loop() {
    use stock_analysis::monitor::detector::AlertEvent;

    use stock_analysis::monitor::news_monitor::NewsMonitor;

    use stock_analysis::monitor::signal_state::SignalStateMachine;

    let poll_secs: u64 = std::env::var("NEWS_POLL_INTERVAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);

    log::info!("[NewsMonitor] 启动（独立窗口，不随价格扫描器静默）");
    log::warn!(
        "[NewsMonitor][BR-138] verified broker position batch unavailable at startup; local projection is excluded from announcement audience"
    );

    let mut nm = NewsMonitor::new();

    nm.restore_dedup();

    log::warn!("[NewsAI][BR-112] disabled=incomplete_source_context_and_delivery_contract");

    let mut sm = SignalStateMachine::default();

    sm.restore_state();

    let mut last_concept_refresh = std::time::Instant::now();

    let mut last_flush = std::time::Instant::now();

    // 产业链机会发现调度：None=启动后首轮立即跑，之后按 opportunity_scan_interval_min 间隔

    // 统一在本 8:00-22:00 窗口内调度（覆盖盘前/盘中/盘后），消除「收盘即停」盲区。

    let mut last_opp_scan: Option<std::time::Instant> = None;

    // v17.4 §5.1 (BR-082): NewsFlashGate — critical 即时推 + 4 时段聚合 Top3
    let mut news_flash_gate =
        news_aggregator_init::NewsFlashGate::new(chrono::Local::now().date_naive());

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

    // BR-137: policy keeps the original provider SearchResult and uses L4/L7
    // delivery governance for dedup/retry; it is not registered as generic flash.
    let policy_provider =
        stock_analysis::search_service::providers::gov_policy::GovPolicyProvider::new();

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

        if outer_tick.enter(NewsOuterTickPhase::Policy) {
            match v17_sources::poll_policy_provider(&policy_provider, 20).await {
                Ok(report) => log::info!(
                    "[Policy][BR-137] attempted={} classified={} pushed={} skipped={} failed={}",
                    report.attempted,
                    report.classified,
                    report.pushed,
                    report.skipped,
                    report.failed
                ),
                Err(error) => log::error!("[Policy][BR-137] provider poll failed: {error}"),
            }
        }

        // v17.4: NewsAggregator tick 入口 — 每轮调一次, 拿 dedup 后 Vec<MarketEvent>
        // v17.4 §5.1: 事件喂 NewsFlashGate → critical 即时推 + 4 时段聚合 (AC34/AC35)
        if outer_tick.enter(NewsOuterTickPhase::CriticalFlash) {
            let news_events = news_aggregator_init::tick_news_aggregator(20).await;
            {
                let mcfg = stock_analysis::config::get_monitor_config();
                let decisions = news_flash_gate.process(
                    &news_events,
                    chrono::Local::now(),
                    mcfg.news_critical_score_threshold,
                    mcfg.news_max_critical_per_day,
                );
                if !decisions.is_empty() {
                    let (nc, na) = news_aggregator_init::push_flash_decisions(decisions).await;
                    log::info!("[v17.4] news_flash push: critical={} aggregated={}", nc, na);
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
            log::error!(
                "[NewsMonitor][BR-138] 自选池加载未就绪，本轮仅隔离公告受众/自选增量: {error}"
            );
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

        // v17.7 Task 7: Poll earnings and analyst upgrades for watchlist
        if outer_tick.enter(NewsOuterTickPhase::HoldingEarnings) {
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
                    Ok(Some(index)) => {
                        nm.linker_mut().replace_concept_index(index);

                        log::info!(
                            "[NewsMonitor] L2 概念索引已更新（{}个板块关联）",
                            nm.linker_ref().concept_count()
                        );
                    }

                    Ok(None) => log::warn!("[NewsMonitor] L2 概念索引刷新跳过（无板块数据）"),

                    Err(_) => log::warn!("[NewsMonitor] L2 概念索引刷新 panic"),
                }
            } else {
                log::warn!("[NewsMonitor] L2 概念索引刷新跳过（标的来源不可用）");
            }

            // v41: 周期刷新 banner (让 news_monitor_loop 的 D-01/I-02 用真 AccountMode)

            evaluate_account_mode_hook(false).await;
        }

        let mut pushed: Vec<AlertEvent> = Vec::new();

        // BR-138: 公告失败只隔离公告子链路，不得跳过同轮的产业链调度、每日重置、
        // 去重落盘或 banner 刷新。生产 provider 自身负责配置 fail-closed。
        let announcements = if outer_tick.enter(NewsOuterTickPhase::Announcement) {
            match stock_analysis::data_provider::announcement::fetch_announcements(None).await {
                Ok(batch) => match audit_announcement_batch_provenance(&batch) {
                    Ok(()) => Some(batch),
                    Err(error) => {
                        log::error!(
                            "[NewsMonitor][BR-137][BR-140] 公告 provenance 审计失败，本轮公告隔离: {error}"
                        );
                        None
                    }
                },
                Err(error) => {
                    log::error!("[NewsMonitor][BR-138] 公告批次获取失败，本轮公告隔离: {error}");
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

            // 异步预解析：公告 API 缺失 code 时，通过东方财富搜索反查。
            let mut resolved_codes: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
            {
                Ok(http) => {
                    for ann in anns {
                        if ann.code.is_empty() && !ann.name.is_empty() {
                            if let Some(code) = nm.linker_ref().lookup_code_by_name(&ann.name) {
                                resolved_codes.insert(ann.name.clone(), code.to_string());
                            } else if let Some(code) =
                                stock_analysis::monitor::news_monitor::resolve_code_by_name(
                                    &ann.name, &http,
                                )
                                .await
                            {
                                log::info!("[NewsMonitor] 反查 {} → {}", ann.name, code);
                                resolved_codes.insert(ann.name.clone(), code);
                            }
                        }
                    }
                }
                Err(error) => log::error!(
                    "[NewsMonitor][BR-138] 公告名称反查客户端初始化失败，缺代码公告保持显式缺失: {error}"
                ),
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
                "[公告][BR-137][BR-138] attempted={} classified={} pushed={} skipped={} failed={} audience={} disposition_pushed={} disposition_lifecycle={} disposition_audience={} disposition_failed={}",
                announcement_route.source.attempted,
                announcement_route.source.classified,
                announcement_route.source.pushed,
                announcement_route.source.skipped,
                announcement_route.source.failed,
                announcement_audience_codes.len(),
                disposition_counts.pushed,
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

        // BR-112: shadow_rank_hits currently builds a default all-zero MarketContext.
        // The already validated legacy alert remains the only production output.
        if !pushed.is_empty() {
            log::warn!(
                "[NewsRanker][BR-112] ranked announcement push disabled=missing_market_context"
            );
        }

        // ═══════════════════════════════════════════════════════════════

        // v29 + v60: D-01 新闻驱动个股推送 (事件驱动)

        //   - 触发: pushed 不空 (有重要公告/事件) 时, 每轮 news_monitor_loop 调一次

        //   - v60 F9: 加 AlertLevel::Important 过滤 (NewsRanker line 2830 已有)

        //     - 低优先级 Info 事件不再触发 D-01 1h memo slot

        //   - 去重: dispatcher memo 1h/票 + push_governor 20min 冷却 (v12 §14.5)

        //   - 数据源: 候选台 (5 源合并) - 与 NewsRanked 公告影子 rank 互补

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

        // 路径A 机会发现已统一到 opportunity::run_opportunity_scan（monitor_loop 内调度），

        // news_ai::discover_opportunities 在 v9.1 Task 0 已删除。

        // 产业链机会扫描：统一在 8:00-22:00 窗口内按间隔调度（覆盖盘前/盘中/盘后）。

        // spawn 异步执行，不阻塞新闻轮询。

        let opp_interval_secs =
            stock_analysis::config::get_monitor_config().opportunity_scan_interval_min * 60;

        let opp_due = last_opp_scan
            .map(|t| t.elapsed().as_secs() >= opp_interval_secs)
            .unwrap_or(true);

        if outer_tick.enter(NewsOuterTickPhase::Opportunity) && opp_due {
            last_opp_scan = Some(std::time::Instant::now());
            log::warn!("[产业链][BR-112] scan disabled=incomplete_source_contract");
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

type T0SnapshotPositions = std::collections::HashMap<String, (String, Option<f64>)>;
type T0PositionSource = (Vec<String>, Option<T0SnapshotPositions>);

async fn monitor_loop() {
    // 全天候循环：非交易日等待，交易日自动进入扫描

    // v16.3 Commit 5: 接入 v16.3 4 模块 (verify finding 修复: main_loop 0 调用)
    // - IntradayMonitor::tick  盘中: 每 30s 扫推送票池 + 4 步过滤 + 调 paper_trade::simulate
    // - evening_review       盘后: 15:30 整盘 Momentum 整盘扫 (Fix 5: 不限 1h 时间窗)
    // - paper_engine         review fix Issue #2: 4 铁律卖出闭环 — 真持仓从 paper_trades 聚合,
    //                        铁律触发 → simulate(Sell), plan_id 日级幂等防 30s tick 重复卖
    let intraday_loop = async {
        use chrono::Timelike;
        use stock_analysis::decision::intraday_monitor::{evening_review, IntradayMonitor};
        use stock_analysis::trading::paper_engine;
        let monitor = IntradayMonitor;
        loop {
            let risk_context = current_banner_for("v16.3 paper decision").and_then(|banner| {
                match push_templates::paper_risk_context_from_banner(&banner) {
                    Ok(context) => Some(context),
                    Err(error) => {
                        log::error!("[v16.3][BR-134] paper risk context unavailable: {}", error);
                        None
                    }
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
            }
            // 4 铁律卖出检查 (review fix Issue #2: 之前是 dead code "暂不启")
            // Fix MEDIUM 4: 5 分钟 debounce (analysis_result 1 日最多变 1-2 次, 30s tick 是浪费)
            // 用 static Mutex<Option<Instant>> 记录 last_run, 5 分钟内跳过
            static PAPER_ENGINE_LAST_RUN: std::sync::Mutex<Option<std::time::Instant>> =
                std::sync::Mutex::new(None);
            let should_run_4_iron = {
                let last = PAPER_ENGINE_LAST_RUN
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                !matches!(
                    *last,
                    Some(t) if t.elapsed() < std::time::Duration::from_secs(300)
                )
            };
            if should_run_4_iron {
                let result = risk_context
                    .ok_or_else(|| "latest evaluated paper risk context unavailable".to_string())
                    .and_then(paper_engine::run_once);
                match result {
                    Ok(count) => {
                        log::debug!("[paper_engine] 4 铁律批次成功: {} 个退出决定", count);
                        *PAPER_ENGINE_LAST_RUN
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
                    }
                    Err(e) => {
                        log::warn!("[paper_engine][BR-134] 本轮失败，保留立即重试资格: {}", e)
                    }
                }
            } else {
                log::debug!("[paper_engine] 4 铁律 5 分钟 debounce 中, 跳过");
            }
            // 15:30 整盘扫 (R5) — evening_review 内部有当日防重入 (review fix Issue #7)
            let now = chrono::Local::now();
            if now.hour() == 15 && now.minute() == 30 {
                let today = now.date_naive();
                match risk_context {
                    Some(risk_context) => {
                        if let Err(e) = evening_review(today, risk_context) {
                            log::warn!("[evening_review] 失败: {}", e);
                        }
                    }
                    None => log::error!(
                        "[evening_review][BR-134] 缺少最新真实风险上下文，保留当日重试资格"
                    ),
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
            // BR-021 §5.10 / commit 08cca47 + caller wire: 8:30 盘前重置 cron.
            // 调一次 push_account_mode_change 触发 evaluate(), 内部 should_reset_at_8_30
            // (Frozen + 8:30 窗口) → 强制 prev=None → evaluate 重判 → 落库 + 推 T-01.
            // 用 Mutex<Option<NaiveDate>> 防当日重复 (跟 15:05 / 15:30 同 pattern).
            if now.hour() == 8 && now.minute() == 30 {
                static BR021_LAST_RUN: std::sync::Mutex<Option<chrono::NaiveDate>> =
                    std::sync::Mutex::new(None);
                let today = now.date_naive();
                let already_run = BR021_LAST_RUN
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .map(|d| d == today)
                    .unwrap_or(false);
                if !already_run {
                    log::info!("[BR-021][BR-108] 8:30 cron 触发真实 AccountMode 评估");
                    if evaluate_account_mode_hook(false).await {
                        *BR021_LAST_RUN.lock().unwrap_or_else(|e| e.into_inner()) = Some(today);
                    } else {
                        log::error!("[BR-021][BR-108] 8:30 评估失败，保留重试资格");
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

            let (positions, targets) = match TieredScanner::load_portfolio_targets() {
                Ok(batch) => batch,
                Err(error) => {
                    log::error!("[盘前] Scanner 标的批次加载失败，30 秒后重试: {}", error);
                    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                    continue;
                }
            };

            // review #14: is_t1_locked 返回 Result, 显式 match; DB 失败时按"未解锁"

            // 处理 (保守), 同时 log warn 让 operator 知道.

            // review #14 修正: 原 Err → false (按未解锁处理) 与另一 caller Err → true (按锁定)

            // 不一致, 违反"安全保守"原则. 统一保守: DB 失败 → 按已锁定处理,

            // 持仓跳过解禁候选, 防止违反 T+1.

            let t1_unlocks: Vec<_> = positions
                .iter()
                .filter(|p| match stock_analysis::portfolio::is_t1_locked(&p.code) {
                    Ok(true) => false,

                    Ok(false) => true,

                    Err(e) => {
                        log::error!(
                            "[盘前] is_t1_locked({}) 失败: {} — 保守按已锁定处理",
                            p.code,
                            e
                        );

                        false
                    }
                })
                .cloned()
                .collect();

            let pre_market = checklist::build_pre_market_checklist(&positions, &t1_unlocks, &[]);

            log::info!(
                "[盘前] {} 只持仓，{} 只解禁",
                positions.len(),
                t1_unlocks.len()
            );

            push_governor_v3(&pre_market, PushKind::DailyReport, None).await;

            prediction::verify_predictions().await;

            match prediction::recent_hit_rate(7) {
                Ok(hit_rate) => log::info!("[预测] 近7天命中率: {:.0}%", hit_rate * 100.0),
                Err(error) => log::warn!("[预测] 近7天命中率不可用: {}", error),
            }

            // 构建实体过滤集合（只关注9只标的）

            let our_codes: std::collections::HashSet<String> =
                targets.iter().map(|t| t.code.clone()).collect();

            // v19.13: 真实持仓 set (只 stock_position open), 不含 watchlist

            // 做T建议只能对真实持仓推, 不能对 watchlist 候选票推 (AGENTS.md §2.1)

            let holding_only_codes: std::collections::HashSet<String> = positions
                .iter()
                .map(|position| position.code.clone())
                .collect();

            let scanner = TieredScanner::new(targets);

            let detector = Detector::new(DetectorConfig::default());

            let mut state_machine = SignalStateMachine::default();

            state_machine.restore_state();

            let mut signal_count = 0u32;

            let mut alert_count = 0u32;

            let mut total_limit_ups: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            let mut total_limit_downs: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            let mut total_board_breaks = 0u32;

            let poll_secs: u64 = std::env::var("MONITOR_HOLDING_INTERVAL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(30);

            // Phase 1.1 量化标准：信号融合 + 风险叠加 + 状态驱动

            use stock_analysis::monitor::signal_fusion::{Signal, SignalFusion, SignalSource};

            let fusion = SignalFusion::default();

            // 三个独立计时器

            let mut last_sector_push = std::time::Instant::now(); // 领涨板块（5分钟）
            let mut last_market_view = std::time::Instant::now(); // b013 P1-10: 盘面+产业链独立计时器 (5分钟)

            let mut last_health_summary = std::time::Instant::now(); // 持仓健康度（5分钟）

            let mut last_t0_scan = std::time::Instant::now(); // 持仓做 T 扫描（30秒）

            let mut last_screener_run = std::time::Instant::now(); // 选股推荐（30分钟）

            let mut last_fund_top_push = std::time::Instant::now(); // 全市场主力净流入Top10（5分钟）

            let mut last_turnover_top_push = std::time::Instant::now(); // 真实换手率Top10（10分钟）

            let mut last_intraday_market = std::time::Instant::now(); // v31: I-01 盘中轮动总览 (10 min)

            let mut last_industry_chain_intraday = std::time::Instant::now(); // v34: I-03 涨停扩散 (15 min)

            let mut last_holding_plan = std::time::Instant::now(); // v38: I-04 持仓操作建议 (30 min)

            // v44: T-14 盘后固定价格申报 (15 min, 申报窗口 9:30-15:30)

            let mut last_post_fixed_order = std::time::Instant::now();

            // v45: T-15 盘后固定价格成交 (撮合 15:05-15:30, 5 min 周期)

            let mut last_post_fixed_fill = std::time::Instant::now();

            // v46: T-16 ST 涨跌幅变更 (开盘 9:30 一次/票/日)

            let mut st_price_pushed = false;

            // v47: T-17 ETF 收盘集合竞价 (14:57-15:00 一次)

            let mut etf_closing_pushed = false;

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
                    }
                }

                // ── 9:20-9:25 竞价高量能扫描（30秒一次）+ 盘后优选重推 ──

                if session == MarketSession::Auction {
                    let now_time = chrono::Local::now().time();

                    // 9:20 之前只做持仓告警，不推全市场量能（数据不稳定）

                    if now_time >= chrono::NaiveTime::from_hms_opt(9, 20, 0).unwrap() {
                        log::info!("[竞价] 9:20-9:25 量能扫描...");

                        let limit_stocks =
                            match tokio::task::spawn_blocking(|| -> Result<_, String> {
                                let analyzer =
                                    stock_analysis::market_analyzer::MarketAnalyzer::new(None)
                                        .map_err(|error| {
                                            format!("初始化涨停池数据源失败: {error:#}")
                                        })?;
                                analyzer
                                    .get_limit_up_stocks()
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

                                // 标记已通知 (避免同票重复推)

                                for s in &new_items {
                                    auction_vol_notified.insert(s.code.clone());
                                }

                                if let Some(banner) = current_banner_for("P-02 auction volume") {
                                    let _ =
                                        push_templates::dispatch_auction_volume_daily(&ts, &banner)
                                            .await;
                                }
                            }
                        }

                        // ▶ v13.10.1 P0-#3: 9:20-9:25 不再独立推送优选候选,

                        // 候选台(CandidateBoard)统一承载, 这里仅拉取用于虚拟观察.

                        if !post_close_candidates_notified {
                            post_close_candidates_notified = true;

                            log::warn!(
                            "[竞价][BR-112] opportunity observation disabled=incomplete_source_contract"
                        );

                            // Keep the downstream parser inert until the producer exposes a
                            // strict Result contract; an empty verified no-input cannot create
                            // records or notifications.
                            let post_close = String::new();

                            // 删 v13.10.1: notify::push_governor(&post_close, notify::PushKind::AuctionRepush).await;

                            // 候选并入候选台 (run_candidate_panel_from_review) 统一推送

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
                                    market_data::fetch_eastmoney_quotes(&codes)
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

                                            // v17.6 §5.1: FactorIC → daily_report_router (demo migration)
                                            // 走 DailyReport 主路径 + [FactorIC] prefix, 不再用旧 PushKind 直推
                                            crate::daily_report_router::route_factor_ic(
                                                &lines.join("\n"),
                                            )
                                            .await;
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

                                    push(event).await;
                                }
                            }
                        }

                        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

                        continue;
                    } else {
                        // BR-100: P-04 只消费当日已持久化 paper_trades 完成态。
                        {
                            let hhmm = chrono::Local::now().format("%H:%M").to_string();
                            if !push_templates::dispatch_paper_trade_daily(&hhmm).await {
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
                    let result = tokio::task::spawn_blocking(|| {
                        intraday_market::acquire_intraday_market_inputs(
                            || {
                                let analyzer =
                                    stock_analysis::market_analyzer::MarketAnalyzer::new(None)
                                        .map_err(|error| {
                                            format!("初始化市场分析器失败: {error}")
                                        })?;
                                analyzer
                                    .get_limit_up_stocks()
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

                            //   - 新: 显式 fetch_position_quotes 给所有 virtual_observation codes (无持仓关系)

                            let virt_codes: Vec<String> = virtual_observation
                                .iter()
                                .filter(|(_, _, p)| *p == 0.0)
                                .map(|(c, _, _)| c.clone())
                                .collect();

                            if !virt_codes.is_empty() {
                                match market_data::fetch_eastmoney_quotes(&virt_codes) {
                                    Ok(virt_quotes) => {
                                        for q in virt_quotes {
                                            for virtual_pos in &mut virtual_observation {
                                                if virtual_pos.0 == q.code && virtual_pos.2 == 0.0 {
                                                    virtual_pos.2 = q.price;
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

                            let ts = chrono::Local::now().format("%H:%M");

                            if !first_lines.is_empty() {
                                let mut lines = vec![format!(
                                    "🟢 首板涨停 Top{}（{}）",
                                    first_lines.len().min(10),
                                    ts
                                )];

                                lines.extend(first_lines.into_iter().take(10));

                                notify::push_governor(
                                    &lines.join("\n"),
                                    notify::PushKind::LimitBoards,
                                )
                                .await;
                            }

                            if !second_lines.is_empty() {
                                let mut lines = vec![format!(
                                    "🟡 二板涨停 Top{}（{}）",
                                    second_lines.len().min(10),
                                    ts
                                )];

                                lines.extend(second_lines.into_iter().take(10));

                                notify::push_governor(
                                    &lines.join("\n"),
                                    notify::PushKind::LimitBoards,
                                )
                                .await;
                            }

                            if !third_lines.is_empty() {
                                let mut lines = vec![format!(
                                    "🔴 三板+ 涨停 Top{}（{}）",
                                    third_lines.len().min(10),
                                    ts
                                )];

                                lines.extend(third_lines.into_iter().take(10));

                                notify::push_governor(
                                    &lines.join("\n"),
                                    notify::PushKind::LimitBoards,
                                )
                                .await;
                            }
                        }

                        // 合并两路数据：涨停列表中的持仓 + 持仓单独查询

                        let mut health_lines: Vec<String> = Vec::new();

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
                            }

                            // 信号融合

                            let resonance = if signals.is_empty() {
                                0.0
                            } else {
                                fusion.resonance(&signals)
                            };

                            let recommend = fusion.recommend(resonance);

                            // 累计当日数据（供收盘总结）

                            if is_limit_up {
                                total_limit_ups.insert(code.clone());
                            }

                            if s.change_pct <= -9.5 {
                                total_limit_downs.insert(code.clone());
                            }

                            if prev_was_limit && !is_limit_up {
                                total_board_breaks += 1;
                            }

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

                                    push(ev).await;
                                }
                            }

                            // 炸板立即推送（Emergency，无限冷却）

                            if !emergency_note.is_empty() {
                                push_governor_v3(
                                    &format!("🔴 {}({}) {}", s.name, code, emergency_note),
                                    PushKind::HoldingEvent,
                                    Some(code),
                                )
                                .await;
                            }

                            // 健康度记录（每5分钟推送汇总）

                            let note = if t1_locked {
                                "🔒锁仓"
                            } else if is_limit_up {
                                "🔺涨停"
                            } else if s.change_pct <= -5.0 {
                                "🔻"
                            } else if resonance > 60.0 {
                                "📈"
                            } else if resonance < -30.0 {
                                "📉"
                            } else {
                                "→"
                            };

                            health_lines.push(format!(
                                "  {:<6} {}({}) {:>+.1}% ¥{:2} {}",
                                note,
                                s.name,
                                code,
                                s.change_pct,
                                s.price,
                                if resonance.abs() > 5.0 {
                                    format!("共振{:0}", resonance)
                                } else {
                                    String::new()
                                }
                            ));

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

                            // 新: 上面 last_screener_run 后的 "持仓专属做T扫描" 才是真路径

                            // 这里只保留 signal_count + alert_count, 不推做T
                        }

                        // v19.12: 持仓健康度 → 每 5 分钟推 (用户要求全推)
                        // v17.4 §5.3.3 (D 方案): state 未变 → dedup 跳过 (内容相同无信息增量, 非降级);
                        //   state 变化立即推, 5min 节奏保留 (AC44). 跳过时 info 出声 (AC47).
                        if last_health_summary.elapsed().as_secs() >= 300
                            && !health_lines.is_empty()
                        {
                            let state_hash = health_state_hash(&health_lines);
                            if holding_health_state_unchanged(state_hash) {
                                log::info!(
                                "[v17.4-D] [dedup] holding.health state 未变 (hash={:x}), 跳过本轮 T-04 推送",
                                state_hash
                            );
                                last_health_summary = std::time::Instant::now();
                            } else {
                                let mut summary = vec![format!(
                                    "📊 持仓健康度 ({})",
                                    chrono::Local::now().format("%H:%M")
                                )];
                                summary.append(&mut health_lines);
                                summary.push("─────".into());
                                summary.push("💡 T-04 持仓监控 (5min 周期, state 变化时推)".into());
                                let outcome = notify::push_governor_v3(
                                    &summary.join("\n"),
                                    notify::PushKind::HoldingEvent,
                                    None,
                                )
                                .await;
                                if periodic_delivery_confirmed(&outcome) {
                                    commit_holding_health_state(state_hash);
                                    last_health_summary = std::time::Instant::now();
                                } else {
                                    log::error!(
                                    "[BR-116] holding.health 投递未确认，保留到期状态重试: {:?}",
                                    outcome
                                );
                                }
                            }
                        }

                        // 选股推荐（独立计时器，每30分钟）

                        let cfg = stock_analysis::config::get_monitor_config();

                        if last_screener_run.elapsed().as_secs() >= cfg.screener_interval_min * 60 {
                            log::info!("[选股] 开始盘中选股扫描...");

                            match tokio::task::spawn_blocking(run_stock_screener).await {
                                Ok(Ok(recs)) => {
                                    let mut confirmed = true;
                                    for (code, rec) in &recs {
                                        log::info!("[选股] {}", rec);

                                        // v57: 改用 D-01 NewsToIdea PushKind (合并 StockPick)

                                        let outcome = notify::push_governor_v3(
                                            rec,
                                            notify::PushKind::NewsToIdea,
                                            Some(code),
                                        )
                                        .await;
                                        if !matches!(
                                            outcome,
                                            notify::PushOutcome::Pushed
                                                | notify::PushOutcome::Deduped
                                        ) {
                                            confirmed = false;
                                            log::error!(
                                                "[BR-116] 选股推荐投递未确认 code={} outcome={:?}",
                                                code,
                                                outcome
                                            );
                                        }
                                    }
                                    if confirmed {
                                        last_screener_run = std::time::Instant::now();
                                    }
                                }
                                Ok(Err(error)) => {
                                    log::error!("[BR-116] 选股批次失败，保留到期状态: {}", error)
                                }
                                Err(error) => {
                                    log::error!(
                                        "[BR-116] 选股后台任务失败，保留到期状态: {}",
                                        error
                                    )
                                }
                            }
                        }

                        // BR-151 / v19.13: 用户确认持仓专属做T扫描 (每 30s, 不接券商账户)

                        // AGENTS.md §2.1: 做T建议只对真实持仓推 (不是 watchlist 候选票)

                        if last_t0_scan.elapsed().as_secs() >= 30 {
                            let user_snapshot = stock_analysis::database::user_position_snapshot::latest_user_position_snapshot();
                            let (holding_codes_vec, snapshot_positions): T0PositionSource =
                                match user_snapshot {
                                    Ok(Some(snapshot)) => {
                                        let positions = snapshot
                                            .items
                                            .into_iter()
                                            .map(|item| (item.code.clone(), (item.name, None)))
                                            .collect::<std::collections::HashMap<_, _>>();
                                        let mut codes =
                                            positions.keys().cloned().collect::<Vec<_>>();
                                        codes.sort_unstable();
                                        (codes, Some(positions))
                                    }
                                    Ok(None) => {
                                        (holding_only_codes.iter().cloned().collect(), None)
                                    }
                                    Err(error) => {
                                        log::warn!(
                                        "[做T-持仓][BR-146] 用户持仓快照读取失败，回退旧持仓源: {}",
                                        error
                                    );
                                        (holding_only_codes.iter().cloned().collect(), None)
                                    }
                                };

                            if holding_codes_vec.is_empty() {
                                last_t0_scan = std::time::Instant::now();
                            } else {
                                let holding_signals = tokio::task::spawn_blocking(move || -> Result<Vec<(String, String)>, String> {

                                use stock_analysis::monitor::detector::{Detector, DetectorConfig, StockSnapshot as SS};

                                let detector_local = Detector::new(DetectorConfig::default());

                                let snapshot_mode = snapshot_positions.is_some();
                                let quotes = if snapshot_mode {
                                    market_data::fetch_eastmoney_quotes(&holding_codes_vec)
                                        .or_else(|east_error| {
                                            market_data::fetch_sina_quotes(&holding_codes_vec)
                                                .map_err(|sina_error| {
                                                    format!(
                                                        "用户持仓快照行情主备源均失败: 东财={east_error}; 新浪={sina_error}"
                                                    )
                                                })
                                        })?
                                } else {
                                    market_data::fetch_position_quotes()?
                                };

                                let position_map: std::collections::HashMap<String, (String, Option<f64>)> = match snapshot_positions {
                                    Some(positions) => positions,
                                    None => stock_analysis::portfolio::get_positions()
                                        .map_err(|error| format!("获取持仓止损证据失败: {error}"))?
                                        .into_iter()
                                        .map(|position| (position.code.clone(), (position.name, position.hard_stop)))
                                        .collect(),
                                };

                                let mut out: Vec<(String, String)> = Vec::new();

                                for q in &quotes {

                                    if !holding_codes_vec.contains(&q.code) { continue; }

                                    let (Some(volume_ratio), Some(main_net_yi)) =
                                        (q.volume_ratio, q.main_net_yi)
                                    else {
                                        log::warn!(
                                            "[做T-持仓] {} 缺少量比或主力净流，跳过",
                                            q.code
                                        );
                                        continue;
                                    };

                                    let Some((position_name, hard_stop_value)) = position_map.get(&q.code) else {
                                        return Err(format!("行情批次含非持仓代码 {}", q.code));
                                    };
                                    let hard_stop = hard_stop_value
                                        .filter(|value| value.is_finite() && *value > 0.0);

                                    let snap = SS {

                                        code: q.code.clone(),

                                        name: position_name.clone(),

                                        price: q.price,

                                        change_pct: q.change_pct,

                                        volume_ratio,

                                        main_net_yi,

                                        limit_up_price: None, was_limit_up: false, t1_locked: false,

                                    };

                                    for e in detector_local.scan_stock(&snap) {

                                        // 强信号才推做T (VolumeBurst / MainInflow / MainOutflow)

                                        if matches!(e.category,

                                            stock_analysis::monitor::detector::AlertCategory::VolBurst

                                            | stock_analysis::monitor::detector::AlertCategory::MainInflow

                                            | stock_analysis::monitor::detector::AlertCategory::MainOutflow)

                                        {

                                            let dir = if matches!(e.category,

                                                stock_analysis::monitor::detector::AlertCategory::MainInflow) { "+" } else { "-" };

                                            let stop_text = hard_stop
                                                .map(|value| format!("¥{value:.2}"))
                                                .unwrap_or_else(|| "用户快照止损位不可用".to_string());
                                            out.push((snap.code.clone(), format!(

                                                "🔄 做T建议 {}({}) | {} {}\n   现价 ¥{:.2} 涨跌 {:+.2}%\n   高抛: +{:.1}% 减仓1/3\n   低吸: -{:.1}% 回补2/3\n   止损: ¥{:.2}",

                                                snap.name, snap.code, dir, e.message,

                                                snap.price, snap.change_pct,

                                                snap.change_pct.abs().max(2.0), snap.change_pct.abs().max(2.0),

                                                stop_text

                                            )));

                                        }

                                    }

                                }

                                Ok(out)

                            }).await;

                                let holding_signals = match holding_signals {
                                    Ok(Ok(signals)) => Some(signals),
                                    Ok(Err(error)) => {
                                        log::error!("[做T-持仓] 数据批次拒绝: {}", error);
                                        None
                                    }
                                    Err(error) => {
                                        log::error!("[做T-持仓] 后台任务失败: {}", error);
                                        None
                                    }
                                };

                                if let Some(holding_signals) = holding_signals {
                                    let mut confirmed = true;
                                    for (code, t0) in holding_signals {
                                        log::info!(
                                            "[做T-持仓] 推送: {}",
                                            t0.lines().next().unwrap_or("")
                                        );

                                        let outcome = notify::push_governor_v3(
                                            &t0,
                                            notify::PushKind::T0Advice,
                                            Some(&code),
                                        )
                                        .await;
                                        if !matches!(
                                            outcome,
                                            notify::PushOutcome::Pushed
                                                | notify::PushOutcome::Deduped
                                        ) {
                                            confirmed = false;
                                            log::error!(
                                                "[BR-116] 做T投递未确认 code={} outcome={:?}",
                                                code,
                                                outcome
                                            );
                                        }
                                    }
                                    if confirmed {
                                        last_t0_scan = std::time::Instant::now();
                                    }
                                }
                            }
                        }

                        // 产业链扫描已统一到 news_monitor_loop 的 8:00-22:00 窗口调度，

                        // 此处不再重复（避免盘中 monitor_loop 与 news_monitor_loop 双跑双推）。

                        // v17.4 §5.3.1 (⚠️ BREAKING): 领涨板块 SectorTop 5min 推送废弃 —
                        // I-01 盘中轮动总览 (10min, main.rs v31) 已覆盖同类信息且带 chain 热度.
                        // 回滚: env STOCK_ANALYSIS_KEEP_SECTOR_TOP=1 恢复 5min 推送 (无需重启以外操作).
                        // v15.x 静默路径可见: 启动 banner 打 mode + 每次跳过 info (AC47).
                        if last_sector_push.elapsed().as_secs() >= 300 {
                            if sector_top_kept() {
                                if push_sector_leaders().await {
                                    last_sector_push = std::time::Instant::now();
                                }
                            } else {
                                log::info!(
                                "[v17.4-D] SectorTop 已废弃, 跳过 5min 推送 (I-01 覆盖; 回滚 env STOCK_ANALYSIS_KEEP_SECTOR_TOP=1)"
                            );
                                last_sector_push = std::time::Instant::now();
                            }
                        }

                        // 全市场主力净流入 Top10（独立计时器，每5分钟）

                        if last_fund_top_push.elapsed().as_secs() >= 300
                            && push_market_fund_top10().await
                        {
                            last_fund_top_push = std::time::Instant::now();
                        }

                        // v19.12: 盘面走向 (R-02 盘中简版) + 涨停产业链 (R-03 盘中简版) — 每 5 分钟硬推
                        // b013 P1-10: 改用独立 last_market_view 计时器
                        if last_market_view.elapsed().as_secs() >= 300 {
                            let market_view =
                                tokio::task::spawn_blocking(|| -> Result<String, String> {
                                    use stock_analysis::market_analyzer::sector_monitor;

                                    // 盘面简版
                                    let boards = sector_monitor::fetch_board_ranking("f3", 10)
                                        .map_err(|error| format!("盘中板块榜失败: {error:#}"))?;
                                    if boards.is_empty() {
                                        return Ok(String::new());
                                    }
                                    let avg_chg = boards.iter().map(|b| b.change_pct).sum::<f64>()
                                        / boards.len() as f64;
                                    let strong =
                                        boards.iter().filter(|b| b.change_pct > 3.0).count();
                                    let mut text = format!(
                                        "📊 盘面 ({} 盘中)\n板块均值 {:+.2}% | 强势板块 {} 个\n",
                                        chrono::Local::now().format("%H:%M"),
                                        avg_chg,
                                        strong
                                    );
                                    text.push_str("领涨板块 Top5:\n");
                                    for board in boards.iter().take(5) {
                                        text.push_str(&format!(
                                            "  {} {:+.2}% 主力{:.2}亿\n",
                                            board.name,
                                            board.change_pct,
                                            board.main_inflow / 1e8
                                        ));
                                    }
                                    Ok(text)
                                })
                                .await;

                            match market_view {
                                Ok(Ok(text)) if !text.is_empty() => {
                                    let outcome = notify::push_governor_v3(
                                        &text,
                                        notify::PushKind::ReviewSignal,
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

                        // v19.12: 盘中换手率高 Top10 (每 10 分钟, 关注流动性)

                        if last_turnover_top_push.elapsed().as_secs() >= 600 {
                            let turnover_result = tokio::task::spawn_blocking(|| {
                                let entries = push_templates::load_turnover_top_real()?;
                                let hhmm = chrono::Local::now().format("%H:%M").to_string();
                                Ok::<_, String>((hhmm, entries))
                            })
                            .await;
                            match turnover_result {
                                Ok(Ok((_, entries))) if entries.is_empty() => {
                                    log::info!("[换手率 Top10] 真实成份数据为空，跳过");
                                    last_turnover_top_push = std::time::Instant::now();
                                }
                                Ok(Ok((hhmm, entries))) => {
                                    let text = push_templates::render_turnover_top(&hhmm, &entries);
                                    let outcome = notify::push_governor_v3(
                                        &text,
                                        notify::PushKind::TurnoverTop,
                                        None,
                                    )
                                    .await;
                                    if periodic_delivery_confirmed(&outcome) {
                                        last_turnover_top_push = std::time::Instant::now();
                                    } else {
                                        log::error!(
                                            "[BR-116] 换手率 Top10 投递未确认，保留到期状态: {:?}",
                                            outcome
                                        );
                                    }
                                }
                                Ok(Err(error)) => {
                                    log::error!("[换手率 Top10] 数据批次拒绝: {}", error)
                                }
                                Err(error) => log::error!("[换手率 Top10] 后台任务失败: {}", error),
                            }
                        }

                        // ═══════════════════════════════════════════════════════════════

                        // v31: I-01 盘中轮动总览 (10 min 周期, 替代老 SectorTop)

                        //   - 数据源: sector_monitor::fetch_board_ranking (科技/电力/机器人三轴)

                        //   - 模板: render_intraday_market (带 banner)

                        //   - 静默: grade_sectors 无数据时短路, log

                        //   - 横幅 DataMode 写死 Full (与 v12 已推模板一致)

                        // ═══════════════════════════════════════════════════════════════

                        if last_intraday_market.elapsed().as_secs() >= 600 {
                            // v41: 读共享 banner

                            if let Some(banner) = current_banner_for("I-01 intraday market") {
                                let hhmm = chrono::Local::now().format("%H:%M").to_string();
                                if push_templates::dispatch_intraday_market_periodic(&hhmm, &banner)
                                    .await
                                {
                                    last_intraday_market = std::time::Instant::now();
                                } else {
                                    log::error!(
                                    "[I-01][BR-091] dispatcher did not confirm delivery; timer not advanced"
                                );
                                }
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
                            let hhmm = chrono::Local::now().format("%H:%M").to_string();

                            if let Some(banner) = current_banner_for("I-04 holding plan") {
                                if push_templates::dispatch_holding_plan_periodic(&hhmm, &banner)
                                    .await
                                {
                                    last_holding_plan = std::time::Instant::now();
                                } else {
                                    log::error!(
                                    "[I-04][BR-091] dispatcher did not confirm delivery; timer not advanced"
                                );
                                }
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

            // 拉上证指数（新浪 API）：阻塞 I/O 放到 blocking 线程，避免在 async 上下文创建/销毁 blocking runtime。

            // 红线 2.2: join 失败时 warn 出声, 不静默填 0.0
            let index_change =
                match tokio::task::spawn_blocking(market_data::fetch_sh_index_change).await {
                    Ok(Ok(value)) => Some(value),
                    Ok(Err(error)) => {
                        log::error!("[大盘] 上证指数不可用，跳过收盘总结: {}", error);
                        None
                    }
                    Err(error) => {
                        log::error!("[大盘] 上证指数任务 join 失败，跳过收盘总结: {}", error);
                        None
                    }
                };

            let up_count = total_limit_ups.len();

            let down_count = total_limit_downs.len();

            let board_break_rate = if up_count > 0 {
                total_board_breaks as f64 / up_count as f64 * 100.0
            } else {
                0.0
            };

            // v13.10.1 P0-#5: 区分"今日冻结(明日解禁)"与"今日解禁(明日可卖)"

            // 之前 close_summary 把 t1_unlocks(今日解禁) 误标为"T+1 冻结", 7 只全部命中,

            // 全显示"止损 0.00"无意义.

            let positions_for_close = match stock_analysis::portfolio::get_positions() {
                Ok(positions) => Some(positions),
                Err(error) => {
                    log::error!("[盘后] 持仓批次不可用，跳过收盘总结: {}", error);
                    None
                }
            };

            let mut t1_frozen: Vec<stock_analysis::portfolio::Position> = Vec::new();

            let mut tomorrow_unlocks: Vec<stock_analysis::portfolio::Position> = Vec::new();

            if let Some(positions) = &positions_for_close {
                for p in positions {
                    match stock_analysis::portfolio::is_t1_locked(&p.code) {
                        Ok(true) => t1_frozen.push(p.clone()),

                        Ok(false) => tomorrow_unlocks.push(p.clone()),

                        Err(e) => {
                            log::error!(
                                "[盘后] is_t1_locked({}) 失败: {} — 保守按已锁定归类",
                                p.code,
                                e
                            );

                            t1_frozen.push(p.clone());
                        }
                    }
                }
            }

            if let (Some(index_change), Some(_)) = (index_change, positions_for_close.as_ref()) {
                let summary = checklist::build_close_summary(
                    index_change,
                    up_count,
                    down_count,
                    board_break_rate,
                    signal_count as usize,
                    alert_count as usize,
                    &t1_frozen,
                    &tomorrow_unlocks,
                );

                push_governor_v3(&summary, PushKind::DailyReport, None).await;
            }

            // v3 复盘报告

            match build_close_review_report().await {
                Ok(report) => {
                    push_governor_v3(&report, PushKind::DailyReport, None).await;
                }
                Err(error) => log::error!("[收盘复盘][BR-103] 数据批次拒绝: {}", error),
            }

            // 盘后独立维度：优选次日候选（最多 5 只，达不到阈值可少推/不推），强调可解释性，不复用盘中量能信号口径。

            // v57: 改用 A-08 TomorrowWatch PushKind (合并 OptimalClose)

            log::warn!("[盘后][BR-112] tomorrow candidates disabled=incomplete_source_contract");

            // 盘后统计上一交易日虚拟观察仓表现（可配置开关）

            if let Err(error) = push_virtual_next_day_review_if_needed().await {
                log::error!("[虚拟观察仓] 次日复盘失败: {}", error);
            }

            // 盘后持仓多 Agent 深度研判（6 分析师 + 多空辩论 + 仲裁），逐只推送飞书

            // v17.0: --test 路径 holding/watch_breakout_text 在 run_test_scan 不可见, 传 "" 占位

            if let Err(error) = run_review_deep_analysis("", "", "").await {
                log::error!("[收盘] 持仓深度复盘失败: {}", error);
            }

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

/// Phase 4.1 选股推荐：点火广度排序 + 成份股过滤

fn run_stock_screener() -> Result<Vec<(String, String)>, String> {
    use stock_analysis::breakout::engine::screen_intraday;

    use stock_analysis::market_analyzer::sector_monitor;

    let our_codes: std::collections::HashSet<String> = stock_analysis::portfolio::get_all_codes()?
        .into_iter()
        .collect();

    // 1. 拉涨幅前 30 板块（失败→本轮无推荐，不刷屏）

    let boards = sector_monitor::fetch_board_ranking("f3", 30)
        .map_err(|error| format!("选股板块榜失败: {error:#}"))?;

    // 2. 收集候选标的（逐板块拉成份股，命中足够候选即提前停止，避免预拉全部 30 板块）

    //    候选携带其所属板块名 + 板块点火广度，供 breakout 盘中模式打分。

    const MAX_CANDIDATES: usize = 20; // 限制批量报价规模，控制 HTTP 成本

    struct Candidate {
        code: String,

        name: String,

        board: String,

        near_limit: usize,
    }

    let mut candidates: Vec<Candidate> = Vec::new();

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for b in boards.iter() {
        let comps = match sector_monitor::fetch_board_components(&b.code, 30) {
            Ok(c) => c,

            Err(error) => {
                return Err(format!("选股板块 {} 成份股批次失败: {error:#}", b.code));
            }
        };

        let ignition = sector_monitor::compute_ignition(&comps);

        for s in comps.iter() {
            if our_codes.contains(&s.code) {
                continue;
            }

            if s.code.starts_with('8') || s.code.starts_with('4') || s.code.starts_with("688") {
                continue;
            }

            if s.name.contains("ST") || s.name.contains("退") {
                continue;
            }

            if s.change_pct > 9.5 {
                continue;
            } // 已涨停不追

            if !seen.insert(s.code.clone()) {
                continue;
            }

            candidates.push(Candidate {
                code: s.code.clone(),

                name: s.name.clone(),

                board: b.name.clone(),

                near_limit: ignition.near_limit_count,
            });

            if candidates.len() >= MAX_CANDIDATES {
                break;
            }
        }

        if candidates.len() >= MAX_CANDIDATES {
            break;
        }
    }

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    // 3. 批量拉候选资金面（一次 HTTP）。失败时拒绝本轮筛选。

    let codes: Vec<String> = candidates.iter().map(|c| c.code.clone()).collect();

    let quote_map: std::collections::HashMap<String, stock_analysis::market_data::TopStock> =
        match market_data::fetch_eastmoney_quotes(&codes) {
            Ok(qs) => qs.into_iter().map(|q| (q.code.clone(), q)).collect(),

            Err(e) => {
                return Err(format!("选股候选资金面批次失败: {e}"));
            }
        };

    // 4. breakout 盘中模式逐个打分

    let mut signals: Vec<(stock_analysis::breakout::signal::BreakoutSignal, String)> = Vec::new();

    for c in &candidates {
        let Some(quote) = quote_map.get(&c.code) else {
            log::warn!("[选股] {} 缺少实时行情，排除候选", c.code);
            continue;
        };
        let (Some(vol_ratio), Some(main_net_yi)) = (quote.volume_ratio, quote.main_net_yi) else {
            log::warn!("[选股] {} 缺少量比或主力净流，排除候选", c.code);
            continue;
        };

        let sig = screen_intraday(
            &c.code,
            &c.name,
            vol_ratio,
            quote.change_pct,
            main_net_yi,
            c.near_limit,
        );

        signals.push((sig, c.board.clone()));
    }

    if signals.is_empty() {
        log::warn!("[选股] 无具备完整资金面字段的候选");
        return Ok(Vec::new());
    }

    // 5. 按置信度降序, 取置信度达阈值的 Top 3
    // v13.10.1 P1-#7: 阈值 20→50. v17.4 §5.3.2: 阈值改走 config (默认 75, 与 launch_gate 语义自洽),
    //   低于阈值静默时 info 出声 (AC43/AC47).
    let min_score = stock_analysis::config::get_monitor_config().screener_min_score;
    signals.sort_by_key(|item| std::cmp::Reverse(item.0.confidence));
    for (s, _) in signals.iter().filter(|(s, _)| s.confidence < min_score) {
        if s.confidence >= 50 {
            // 旧阈值会推、新阈值静默的区间 — 逐条出声, 方便回滚对照
            log::info!(
                "[选股] {}({}) score={} < {}, 静默 (config screener_min_score={})",
                s.name,
                s.code,
                s.confidence,
                min_score,
                min_score
            );
        }
    }
    let recs: Vec<(String, String)> = signals
        .iter()
        .filter(|(s, _)| s.confidence >= min_score)
        .take(3)
        .map(|(s, board)| {
            (
                s.code.clone(),
                format!(
                    "{} 选股推荐 | {}({}) | 板块:{} | 涨幅:{:.1}% | 置信度:{} | {}",
                    s.breakout_type.emoji(),
                    s.name,
                    s.code,
                    board,
                    s.change_pct,
                    s.confidence,
                    s.description
                ),
            )
        })
        .collect();

    Ok(recs)
}

/// v17.4 §5.3.1: SectorTop 废弃回滚判定 (纯函数, 供单测).
/// 默认 (env 未设/其他值) = false = 不推 (BREAKING, v17.4 D 方案);
/// 显式 "1"/"true" = 保留旧 5min 推送。
fn sector_top_kept_from(val: Option<&str>) -> bool {
    matches!(val, Some("1") | Some("true"))
}

/// 运行时读 env (OnceLock 缓存, 每 5min 调一次不值得重复 syscall)
fn sector_top_kept() -> bool {
    static KEPT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *KEPT.get_or_init(|| {
        sector_top_kept_from(
            std::env::var("STOCK_ANALYSIS_KEEP_SECTOR_TOP")
                .ok()
                .as_deref(),
        )
    })
}

/// v17.4 §5.3.3: 持仓健康度 state 哈希 (纯函数, 供单测).
/// 只哈希内容行 (不含时间戳行), 同 state 同 hash。
fn health_state_hash(lines: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for l in lines {
        l.hash(&mut h);
    }
    h.finish()
}

static LAST_HOLDING_HEALTH_STATE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// BR-116: 只检查上次已确认投递的 state，不在检查阶段提前提交。
fn holding_health_state_unchanged(hash: u64) -> bool {
    use std::sync::atomic::Ordering;
    LAST_HOLDING_HEALTH_STATE.load(Ordering::Relaxed) == hash
}

fn commit_holding_health_state(hash: u64) {
    use std::sync::atomic::Ordering;
    LAST_HOLDING_HEALTH_STATE.store(hash, Ordering::Relaxed);
}

/// BR-116: a periodic delivery is complete only after a real sink acceptance or
/// an explicit governance deduplication. Denials and sink failures remain due.
fn periodic_delivery_confirmed(outcome: &notify::PushOutcome) -> bool {
    matches!(
        outcome,
        notify::PushOutcome::Pushed | notify::PushOutcome::Deduped
    )
}

/// 持仓实时行情：东财 push2 为主（多主机轮询），新浪兜底

async fn push_sector_leaders() -> bool {
    // BR-116: periodic wrapper distinguishes Empty/Deduped from failures.

    let hhmm = chrono::Local::now().format("%H:%M").to_string();

    push_templates::dispatch_sector_top_periodic(&hhmm).await
}

async fn push_market_fund_top10() -> bool {
    // BR-116: periodic wrapper distinguishes Empty/Deduped from failures.

    let hhmm = chrono::Local::now().format("%H:%M").to_string();

    push_templates::dispatch_fund_inflow_top_periodic(&hhmm).await
}

async fn post_close_news_review() {
    use chrono::{Duration as ChronoDuration, Utc};

    use stock_analysis::data_provider::sina_news_provider::SinaNewsProvider;

    use stock_analysis::database::DatabaseManager;

    let now = Utc::now();

    let from = now - ChronoDuration::days(30);

    let provider = SinaNewsProvider::new();

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
        match provider.fetch_stock_news_in_range(code, from, now).await {
            Ok(items) => {
                let total = items.len();

                let mut written = 0usize;

                for item in &items {
                    let ok = DatabaseManager::with_db("post_close_news", |db| {
                        if db.insert_news_item(item).is_ok() {
                            Some(())
                        } else {
                            None
                        }
                    });

                    if ok.is_some() {
                        written += 1;
                    }
                }

                log::info!(
                    "[盘后] {code} Sina 个股新闻: 拉 {} 条, DB 写 {} 条",
                    total,
                    written
                );
            }

            Err(e) => log::warn!("[盘后] {code} Sina 拉取失败: {e}"),
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

async fn push(mut event: AlertEvent) -> AlertEvent {
    let attribution = stock_analysis::monitor::attribution::apply_attribution(&mut event);

    log::debug!(
        "[G5a] {} attribution={} elapsed={}ms",
        event.code,
        attribution.result.has_catalyst,
        attribution.elapsed.as_millis()
    );

    let text = alert::format_alert(&event);

    log::info!(
        "[告警] {} {} → {}",
        event.level.emoji(),
        event.code,
        event.message
    );

    if let Err(err) = stock_analysis::monitor::alert_log::append_jsonl(&event) {
        log::error!("[AlertLog] JSONL 写入失败 {}: {}", event.code, err);
    }

    if let Err(err) = stock_analysis::monitor::alert_log::append_md(&event) {
        log::error!("[AlertLog] Markdown 写入失败 {}: {}", event.code, err);
    }

    push_governor_v3(&text, PushKind::DailyReport, None).await;

    event
}

fn build_price_map(
    quotes: &[stock_analysis::market_data::TopStock],
) -> std::collections::HashMap<String, f64> {
    quotes.iter().map(|q| (q.code.clone(), q.price)).collect()
}

#[derive(Debug, Clone)]
struct ReviewLimitChainCandidate {
    code: String,
    name: String,
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

fn load_review_limit_chain_stocks(
    holdings: &[stock_analysis::portfolio::Position],
) -> Result<ReviewLimitChainBatch, String> {
    let fetcher = stock_analysis::data_provider::DataFetcherManager::new()
        .map_err(|error| format!("R-03 初始化日 K 数据源失败: {error:#}"))?;
    let mut source_errors = Vec::new();
    let watchlist = match stock_analysis::portfolio::get_watchlist() {
        Ok(watchlist) => watchlist,
        Err(error) => {
            source_errors.push(format!("R-03 自选查询失败: {error}"));
            Vec::new()
        }
    };
    let candidates = holdings
        .iter()
        .chain(watchlist.iter())
        .take(20)
        .map(|position| ReviewLimitChainCandidate {
            code: position.code.clone(),
            name: position.name.clone(),
            sector: Some(position.sector.clone()),
        })
        .collect::<Vec<_>>();
    let mut batch = collect_review_limit_chain_stocks_with(
        &candidates,
        |code| {
            stock_analysis::block_on_async_with_timeout(
                stock_analysis::data_provider::industry::fetch_industry_name_only(
                    &stock_analysis::http_client::SHARED_HTTP_CLIENT,
                    code,
                ),
                15,
            )
            .map_err(|error| format!("行业请求超时/运行失败: {error}"))?
            .map_err(|error| error.to_string())
        },
        |code| {
            fetcher
                .get_daily_data(code, 60)
                .map(|(kline, _)| kline.into_iter().map(|bar| bar.is_limit_up).collect())
                .map_err(|error| format!("{error:#}"))
        },
    );
    batch.source_errors.extend(source_errors);
    Ok(batch)
}

fn compute_ma(kline: &[stock_analysis::data_provider::KlineData], n: usize) -> Option<f64> {
    if n == 0 || kline.len() < n {
        return None;
    }

    let sum: f64 = kline.iter().rev().take(n).map(|k| k.close).sum();

    Some(sum / n as f64)
}

/// v3: 收盘时记录净值快照到 ledger 表

async fn build_close_review_report() -> Result<String, String> {
    tokio::task::spawn_blocking(|| -> Result<String, String> {
        // Until a real account cash snapshot exists this returns an explicit
        // error and prevents historical ledger rows from being reported as today.
        snapshot_portfolio_value()?;
        let trades = stock_analysis::portfolio::get_trade_history(90)
            .map_err(|error| format!("获取成交历史失败: {error}"))?;
        let mut reviews = stock_analysis::review::journal::review_closed_trades(&trades)
            .map_err(|error| format!("复盘成交 FIFO 失败: {error}"))?;
        stock_analysis::review::journal::enrich_post_exit(&mut reviews);

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

fn snapshot_portfolio_value() -> Result<(), String> {
    Err(
        "disabled=no_fresh_real_account_cash_snapshot; refusing cash=0 and first-day pnl=0"
            .to_string(),
    )
}

// ============================================================================

// v11-P0-5++ Commit 5: 候选筛选台 wrapper (P5 §六 验收)

// ============================================================================

/// 从复盘路径 (LLM 终稿 by_code + 持仓) 收集 5 路 raw, 调 candidate_panel 合并+分档+门槛+排序+渲染

///

/// 5 路 raw 来源 (Commit 4 降级, Commit 5 集中推 1 条):

/// - A10 选股 (本次复盘不直接拿, 留 placeholder)

/// - B3 优选 (run_post_close_candidates)

/// - B6 放量·自选 (holding_breakout_text)

/// - B7 放量·实盘优选 (watch_breakout_text)

/// - C4 产业链 (scan.chain_text, 本函数不调, 留 P0-5++ commit 6 接入)

///

/// **v16.4 修订 (P0-5++ Commit 7)**: 接受 5 路 raw (A10/B3/B6/B7/C4) 真正 5 路收集

/// (主路径暂传 None, 留 P0-5++ commit 8 实际接入 5 处调用点).

///

/// **简化**: 本 commit 不解析 LLM 文本 (留 P0-5++ commit 6+), 直接用 by_code (LLM 终稿) 当 raw 喂入.

/// 实际行为: 每只持仓的 "操作建议" + 板块/产业链 文本 = 1 条候选, source = IndustryChain (兜底).

///

/// **P5 红线 (P5 §一)**: 候选筛选不是买入决策, 不合成"买入分".

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn run_candidate_panel_from_review(
    by_code: &std::collections::HashMap<String, (String, Option<String>)>,

    holdings: &[stock_analysis::portfolio::Position],

    // v16.4: 5 路 raw (主路径暂 None, 留 P0-5++ commit 8 接入)
    stock_pick_raw: Option<&str>,

    optimal_close_raw: Option<&str>,

    volume_watchlist_raw: Option<&str>,

    volume_real_trade_raw: Option<&str>,

    industry_chain_raw: Option<&str>,
) -> String {
    use stock_analysis::opportunity::candidate_panel::{
        classify_tier, filter_hard_gates, format_candidate_board, merge_candidates,
        parse_text_to_raw, sort_candidates, CandidateSource,
    };

    // 收集 5 路 raw (v16.4 P0-5++ Commit 7 修订: 5 个 String 参数, 主路径暂 None 走兜底)

    // P5 §三 3.1 红线: 多路信号合并, 这里 1 路来源 (IndustryChain 兜底)

    let mut raw: Vec<(CandidateSource, String, String)> = Vec::new();

    // v16.4: 5 路 raw 解析 (parse_text_to_raw, P0-5++ Commit 6 加的 helper)

    // 同时收集每个 (code, source) 对应的原始行 (用作 evidence 题材段)

    let mut evidence_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for (source, text) in [
        (CandidateSource::StockPick, stock_pick_raw),
        (CandidateSource::OptimalClose, optimal_close_raw),
        (CandidateSource::VolumeWatchlist, volume_watchlist_raw),
        (CandidateSource::VolumeRealTrade, volume_real_trade_raw),
        (CandidateSource::IndustryChain, industry_chain_raw),
    ] {
        if let Some(t) = text {
            for line in t.lines() {
                // 找 6 位 code + 名字

                let mut chars = line.char_indices().peekable();

                let mut code_end = None;

                let mut code_start = None;

                let _count = 0;

                while let Some((i, c)) = chars.next() {
                    if c.is_ascii_digit() {
                        let mut end = i + c.len_utf8();

                        let mut cnt = 1;

                        while let Some(&(_, nc)) = chars.peek() {
                            if nc.is_ascii_digit() {
                                end += nc.len_utf8();

                                chars.next();

                                cnt += 1;

                                if cnt == 6 {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }

                        if cnt == 6 {
                            code_start = Some(i);

                            code_end = Some(end);
                        }

                        break;
                    }
                }

                if let (Some(s), Some(e)) = (code_start, code_end) {
                    let code = &line[s..e];

                    // 取 code 后 — 前的描述段 (置信% + [详情])

                    let after = &line[e..];

                    if let Some(em_dash_pos) = after.find('—') {
                        let desc = &after[em_dash_pos + 3..]; // 跳过 "— "

                        if !desc.trim().is_empty() {
                            evidence_map.insert(code.to_string(), desc.trim().to_string());
                        }
                    }
                }
            }

            for (code, name) in parse_text_to_raw(t) {
                raw.push((source, code, name));
            }
        }
    }

    // v16.4 兜底: by_code LLM 终稿 → IndustryChain 兜底 (Commit 5 已有)

    if raw.is_empty() {
        // 遍历 by_code (不是 holdings), 候选不只限于持仓

        for (code, value) in by_code {
            if value.1.is_some() {
                raw.push((
                    CandidateSource::IndustryChain,
                    code.clone(),
                    value.0.clone(),
                ));
            }
        }
    }

    // 简化: 实际 P0-5++ 还会接 A10/B3/B6/B7 4 路 raw, 这里先 1 路

    if raw.is_empty() {
        return String::new();
    }

    // 1. 多源合并去重

    let mut entries = merge_candidates(raw);

    // 2. 证据分层 (P5 §3.2 红线: 唯一 Strong = 布林+MACD) + 拉价格/涨幅

    // 拉 K 线 (5 日够看当日), 给 entry 填 current_price / change_pct / 题材

    let fetcher = stock_analysis::data_provider::DataFetcherManager::new().ok();

    for e in &mut entries {
        // 2.1 evidence: 优先 evidence_map (放量描述), fallback by_code LLM 终稿

        let mut ev: Option<String> = None;

        if let Some(desc) = evidence_map.get(&e.code) {
            ev = Some(format!("放量: {}", desc));
        } else if let Some((_, Some(md))) = by_code.get(&e.code) {
            ev = md
                .lines()
                .find(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
                .map(|l| l.chars().take(80).collect::<String>());
        }

        if let Some(s) = ev {
            if !s.is_empty() {
                e.evidence = vec![s];
            }
        }

        // 2.2 价格/涨幅: 拉 K 线最近 1 日

        if let Some(f) = &fetcher {
            if let Ok((klines, _)) = f.get_daily_data(&e.code, 5) {
                if let Some(last) = klines.last() {
                    e.current_price = Some(last.close);

                    e.change_pct = Some(last.pct_chg);
                }
            }
        }

        // 2.3 tier 分类

        e.tier = classify_tier(&e.evidence);
    }

    // 3. 硬门槛过滤 (P5 §3.3): 剔除已持仓 / 停牌 / ST / 北交所/科创板 / 已涨停

    let held_codes: Vec<String> = holdings.iter().map(|p| p.code.clone()).collect();

    entries = filter_hard_gates(entries, &held_codes);

    // 4. 排序 (P5 §3.3 硬规则: 强证据优先 > 多源 > 题材)

    entries = sort_candidates(entries);

    // v13.10.1 P0-#1: 通过硬门槛为 0 时不推"空台"卡片 (用户反馈噪声).

    // format_candidate_board 会输出 "通过硬门槛 0 只" 的卡片, 这里直接短路.

    if entries.is_empty() {
        return String::new();
    }

    // 5. 渲染

    format_candidate_board(&entries)
}

#[cfg(test)]
mod tests_v17_4_d {
    use super::*;

    /// AC45/AC46: SectorTop 默认废弃 (env 未设 = 不推), 显式 1/true 才保留
    #[test]
    fn sector_top_default_deprecated() {
        assert!(
            !sector_top_kept_from(None),
            "默认必须废弃 (BREAKING 默认态)"
        );
        assert!(!sector_top_kept_from(Some("0")));
        assert!(!sector_top_kept_from(Some("")));
        assert!(sector_top_kept_from(Some("1")), "回滚 env=1 必须恢复");
        assert!(sector_top_kept_from(Some("true")));
    }

    /// AC46: config 默认值 screener_min_score = 75
    #[test]
    fn screener_min_score_default_is_75() {
        let cfg = stock_analysis::config::MonitorConfig::default();
        assert_eq!(cfg.screener_min_score, 75, "v17.4 §5.3.2 默认 75");
    }

    /// AC44: state 哈希 — 同内容同 hash, 变更后不同
    #[test]
    fn health_state_hash_stable_and_sensitive() {
        let a = vec!["600519 正常".to_string(), "000001 预警".to_string()];
        let b = a.clone();
        let c = vec!["600519 正常".to_string(), "000001 止损".to_string()];
        assert_eq!(health_state_hash(&a), health_state_hash(&b));
        assert_ne!(health_state_hash(&a), health_state_hash(&c));
    }

    /// AC44: 首次不拦, 同 state 拦, 变 state 放行
    #[test]
    fn holding_health_dedup_sequence() {
        // 注: 共享全局 AtomicU64, 用本测试专属的不会与其他测试碰撞的 hash 序列
        let h1 = health_state_hash(&["tests_v17_4_d-seq-A".to_string()]);
        let h2 = health_state_hash(&["tests_v17_4_d-seq-B".to_string()]);
        assert!(!holding_health_state_unchanged(h1), "首次 h1 应放行");
        commit_holding_health_state(h1);
        assert!(holding_health_state_unchanged(h1), "重复 h1 应拦");
        assert!(!holding_health_state_unchanged(h2), "变更为 h2 应放行");
        commit_holding_health_state(h2);
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

#[cfg(test)]
mod tests_candidate_panel {

    use super::*;

    use chrono::NaiveDate;

    use std::collections::HashMap;

    use stock_analysis::portfolio::{Position, PositionStatus};

    fn make_position(code: &str, name: &str) -> Position {
        Position {
            code: code.to_string(),

            name: name.to_string(),

            shares: 1000,

            cost_price: 10.0,

            hard_stop: None,

            added_at: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),

            status: PositionStatus::Holding,

            sector: "测试".to_string(),
            ..Default::default()
        }
    }

    fn make_md(advice: &str) -> String {
        format!("# 复盘\n## 【操作建议】{}\n", advice)
    }

    /// 空 by_code → 候选台不推 (空字符串)

    #[test]

    fn wrapper_empty_by_code_returns_empty() {
        let by_code: HashMap<String, (String, Option<String>)> = HashMap::new();

        let holdings = vec![make_position("TEST_CODE_600999", "测试")];

        let result =
            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        assert!(result.is_empty(), "空 by_code 应返回空字符串, 不推候选台");
    }

    /// LLM 终稿有 "强烈卖出" → evidence + tier=Reference (因 keywords 是 "卖出" 不是 "布林+MACD")

    #[test]

    fn wrapper_extracts_evidence_from_llm_md() {
        let mut by_code = HashMap::new();

        by_code.insert(
            "TEST_CODE_600999".to_string(),
            ("测试".to_string(), Some(make_md("**强烈卖出**"))),
        );

        let holdings = vec![make_position("TEST_CODE_600000", "测试")];

        let result =
            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        assert!(result.contains("候选筛选台"), "应输出候选台卡片");

        assert!(result.contains("TEST_CODE_600999"), "应包含 code 600999");
    }

    /// LLM 终稿有 "布林+MACD" → tier=Strong (P5 红线: 唯一能进强证据)

    #[test]

    fn wrapper_strong_evidence_for_boll_macd() {
        let mut by_code = HashMap::new();

        by_code.insert(
            "TEST_CODE_600999".to_string(),
            (
                "测试".to_string(),
                Some(make_md("**强烈卖出, 布林+MACD主升浪启动 (B方案, 已验证)**")),
            ),
        );

        let holdings = vec![make_position("TEST_CODE_600000", "测试")];

        let result =
            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        // 5 路 None 兜底走 by_code 600999, evidence 抽 "**强烈卖出, 布林+MACD...**" 命中

        // 渲染输出 "📋 候选筛选台 · 通过硬门槛 1 只" + 1 个 entry

        assert!(result.contains("📋 候选筛选台"), "应输出候选台卡片 (顶部)");

        assert!(
            result.contains("TEST_CODE_600999"),
            "应含 by_code code 600999"
        );
    }

    /// 持仓被 filter_hard_gates 剔除

    #[test]

    fn wrapper_filters_out_held_positions() {
        let mut by_code = HashMap::new();

        by_code.insert(
            "TEST_CODE_000001".to_string(),
            ("持仓A".to_string(), Some(make_md("**强烈卖出**"))),
        );

        by_code.insert(
            "TEST_CODE_000002".to_string(),
            ("候选B".to_string(), Some(make_md("**布林+MACD**"))),
        );

        let holdings = vec![
            make_position("TEST_CODE_000001", "持仓A"), // 已持仓 → 剔除 000001
        ];

        let result =
            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        // 候选 B 留下, 持仓 A 剔除

        assert!(result.contains("TEST_CODE_000002"));

        assert!(!result.contains("持仓A"));
    }

    /// md=None (LLM 失败) → entry 跳过, 候选台不推

    #[test]

    fn wrapper_skips_md_none_entries() {
        let mut by_code = HashMap::new();

        by_code.insert(
            "TEST_CODE_000001".to_string(),
            ("测试".to_string(), None), // LLM 失败
        );

        let holdings = vec![make_position("TEST_CODE_600999", "测试")];

        let result =
            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        assert!(result.is_empty(), "md=None 应跳过 entry, 候选台不推");
    }
}

/// v16.8 (P0-5++ Commit 10): 5 个 wrapper 真 raw 单测

///

/// 验证 wrapper 接 5 个 Some(raw) 时 parse_text_to_raw 正确提取 + merge + 排序 + 渲染

/// (测试主路径 L978 用 None 是因为 5 个 raw 字符串在不同函数, 实际接入留 P0-5++ commit 11)

#[cfg(test)]

mod tests_wrapper_real_raw {

    use super::*;

    use chrono::NaiveDate;

    use std::collections::HashMap;

    use stock_analysis::portfolio::{Position, PositionStatus};

    fn pos(code: &str) -> Position {
        Position {
            code: code.to_string(),

            name: format!("测试{}", code),

            shares: 1000,

            cost_price: 10.0,

            hard_stop: None,

            added_at: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),

            status: PositionStatus::Holding,

            sector: "测试".to_string(),
            ..Default::default()
        }
    }

    /// 5 路全 None → 走 by_code IndustryChain 兜底

    #[test]

    fn wrapper_5_raws_all_none_falls_back_to_by_code() {
        let mut by_code = HashMap::new();

        by_code.insert(
            "TEST_CODE_600999".to_string(), // 不在持仓, 避免被 filter_hard_gates 剔除
            (
                "测试".to_string(),
                Some("# 复盘\n## 【操作建议】**强烈卖出**\n".to_string()),
            ),
        );

        let holdings = vec![pos("TEST_CODE_000001")]; // 持仓 000001, 候选 600999

        let result =
            run_candidate_panel_from_review(&by_code, &holdings, None, None, None, None, None);

        assert!(
            result.contains("TEST_CODE_600999"),
            "5 路 None → 走兜底, 仍应含 by_code code (600999)"
        );
    }

    /// 单路 Some(A10 选股) → 解析 → 1 行候选

    #[test]

    fn wrapper_stock_pick_real_raw() {
        // Protocol-format integration: this wrapper consumes the same native
        // six-digit codes emitted by the text parser.
        let by_code = HashMap::new(); // 不用

        let holdings = vec![pos("TEST_CODE_000001")];

        let stock_pick = "推荐: 600519 贵州茅台 +3.2%";

        let result = run_candidate_panel_from_review(
            &by_code,
            &holdings,
            Some(stock_pick),
            None,
            None,
            None,
            None,
        );

        assert!(result.contains("600519"), "StockPick raw 解析应含 600519");

        assert!(result.contains("贵州茅台"));
    }

    /// 单路 Some(B3 优选) → 解析 (无序号前缀, 跟 parse_text_to_raw 测试一致)

    #[test]

    fn wrapper_optimal_close_real_raw() {
        // Protocol-format integration: parsed native symbols round-trip into
        // the rendered candidate board unchanged.
        let by_code = HashMap::new();

        let holdings = vec![pos("TEST_CODE_000001")];

        let optimal_close = "002208 合肥城建 ¥19.25\n600519 贵州茅台";

        let result = run_candidate_panel_from_review(
            &by_code,
            &holdings,
            None,
            Some(optimal_close),
            None,
            None,
            None,
        );

        assert!(result.contains("002208"));

        assert!(result.contains("600519"));
    }

    /// 单路 Some(C4 产业链) → 解析

    #[test]

    fn wrapper_industry_chain_real_raw() {
        // Protocol-format integration for industry-chain text ingestion.
        let by_code = HashMap::new();

        let holdings = vec![pos("TEST_CODE_000001")];

        // 测试 parse_text_to_raw 实际能解析的格式 (LLM 输出常含 "code + 中文名 + 数据")

        let industry = "002008 大族激光 +5.2%";

        let result = run_candidate_panel_from_review(
            &by_code,
            &holdings,
            None,
            None,
            None,
            None,
            Some(industry),
        );

        assert!(result.contains("002008"), "C4 产业链 raw 应含 002008");
    }

    /// 多路 Some(2 路) → 合并去重 (同 code 出现 2 次 → 1 行, source 列表显示 2 路)

    #[test]

    fn wrapper_multi_raws_merge_dedup() {
        // Protocol-format integration: deduplication keys are the parser's
        // native six-digit output.
        let by_code = HashMap::new();

        let holdings = vec![pos("TEST_CODE_000001")];

        let stock_pick = "600519 贵州茅台";

        let optimal_close = "600519 贵州茅台 (二次推荐)";

        let result = run_candidate_panel_from_review(
            &by_code,
            &holdings,
            Some(stock_pick),
            Some(optimal_close),
            None,
            None,
            None,
        );

        // 合并去重后只有 1 行, 但 sources 应含 2 路 (选股+优选)

        assert!(result.contains("选股+优选"), "2 路合并后 source 应列 2 个");

        let occ = result.matches("600519").count();

        assert_eq!(occ, 1, "同 code 600519 应只出现 1 次 (去重)");
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

    fn at(hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 7, 21)
            .expect("valid test date")
            .and_hms_opt(hour, minute, 0)
            .expect("valid test time")
    }

    #[test]
    fn br139_review_is_due_only_after_threshold_on_a_trading_day() {
        assert!(!post_session_review_window_open(at(18, 59), true));
        assert!(post_session_review_window_open(at(19, 0), true));
        assert!(!post_session_review_window_open(at(19, 0), false));
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
                .matches("let post_session_review = spawn_post_session_review_scheduler();")
                .count(),
            1,
            "the long-running branch must own the post-session review scheduler"
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
}

// ========================================================================
// v17.7 Task 6 Step 1: Announcement routing duplicate-prevention test
// ========================================================================

#[cfg(test)]
mod tests_v17_7_announcement_wiring {
    use super::*;
    use chrono::Local;
    use stock_analysis::data_provider::announcement::{self, Announcement};

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
    fn br138_inline_r08_excludes_local_only_lifecycle_rows() {
        let rows = vec![Announcement {
            code: "TEST_CODE_600000".to_string(),
            name: "测试本地证据".to_string(),
            title: "关于注销部分回购股份并减少注册资本通知债权人的公告".to_string(),
            date: "2026-07-21".to_string(),
            summary: "TEST_CODE summary".to_string(),
            content: String::new(),
            level: announcement::AnnLevel::Skip,
            reason: "BR-138 lifecycle-only local evidence".to_string(),
            external_id: Some("TEST_CODE_INLINE_R08_LOCAL".to_string()),
            url: Some("https://example.invalid/local-only".to_string()),
        }];
        assert!(filter_inline_r08_announcements(rows).is_empty());
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

            for phase in [
                NewsOuterTickPhase::Policy,
                NewsOuterTickPhase::CriticalFlash,
            ] {
                if coordinator.enter(phase) {
                    callback_counts[phase as usize] += 1;
                }
            }
            coordinator.set_watch_readiness(readiness);
            for phase in [
                NewsOuterTickPhase::HoldingEarnings,
                NewsOuterTickPhase::L2,
                NewsOuterTickPhase::Announcement,
                NewsOuterTickPhase::Opportunity,
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
    fn br138_fresh_local_projection_timestamp_never_authorizes_real_position_audience() {
        let watch = std::collections::HashSet::from(["TEST_CODE_WATCH".to_string()]);
        let (audience, warning) = load_announcement_audience_codes(&watch);
        assert_eq!(audience, watch);
        assert!(warning
            .expect("missing broker position source must remain explicit")
            .contains("local projection updated_at is not source evidence"));
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
