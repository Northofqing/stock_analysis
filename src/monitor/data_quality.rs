//! 数据质量门（DQ Gate）。
//!
//! 每条进入检测引擎的 tick 必须先过这五道校验：
//! 1. staleness  — 数据是否过期（超过 N 秒未更新）
//! 2. halt       — 股票是否停牌
//! 3. jump       — 价格是否发生异常跳空
//! 4. ex_rights  — 当日是否为除权除息日
//! 5. price_ok   — 价格是否在合理范围（非零、非负、涨跌幅合理）
//!
//! 脏数据 → 丢弃 + 计数（供系统自监控），不进检测引擎。

use chrono::{DateTime, Local, NaiveDate};
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

use crate::calendar;
use crate::data_provider::KlineData;

// ============================================================================
// 质量检查结果
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum DqRejectReason {
    /// 数据过期（最后更新超过阈值秒数）
    Stale { age_secs: u64, max_secs: u64 },
    /// 股票停牌
    Halted,
    /// 价格跳空异常（相对前值的变动超过阈值百分比）
    Jump { change_pct: f64, threshold_pct: f64 },
    /// 除权除息日（复权价格可能异常）
    ExRights,
    /// 价格不合理（零、负、涨跌幅超限）
    UnreasonablePrice { price: f64, last_close: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessDataType {
    Quote,
    Position,
    Nav,
    Daily,
}

impl FreshnessDataType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FreshnessDataType::Quote => "quote",
            FreshnessDataType::Position => "position",
            FreshnessDataType::Nav => "nav",
            FreshnessDataType::Daily => "daily",
        }
    }
}

impl DqRejectReason {
    pub fn label(&self) -> &'static str {
        match self {
            DqRejectReason::Stale { .. } => "数据过期",
            DqRejectReason::Halted => "停牌",
            DqRejectReason::Jump { .. } => "价格跳空",
            DqRejectReason::ExRights => "除权除息",
            DqRejectReason::UnreasonablePrice { .. } => "价格异常",
        }
    }
}

// ============================================================================
// 质量统计（供系统自监控）
// ============================================================================

#[derive(Debug, Default)]
pub struct DqStats {
    pub total_ticks: AtomicU64,
    pub passed: AtomicU64,
    pub rejected_stale: AtomicU64,
    pub rejected_halted: AtomicU64,
    pub rejected_jump: AtomicU64,
    pub rejected_ex_rights: AtomicU64,
    pub rejected_price: AtomicU64,
}

impl DqStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dirty_rate(&self) -> f64 {
        let total = self.total_ticks.load(Ordering::Relaxed) as f64;
        if total == 0.0 {
            return 0.0;
        }
        let passed = self.passed.load(Ordering::Relaxed) as f64;
        1.0 - passed / total
    }

    pub fn snapshot(&self) -> DqStatsSnapshot {
        DqStatsSnapshot {
            total: self.total_ticks.load(Ordering::Relaxed),
            passed: self.passed.load(Ordering::Relaxed),
            rejected_stale: self.rejected_stale.load(Ordering::Relaxed),
            rejected_halted: self.rejected_halted.load(Ordering::Relaxed),
            rejected_jump: self.rejected_jump.load(Ordering::Relaxed),
            rejected_ex_rights: self.rejected_ex_rights.load(Ordering::Relaxed),
            rejected_price: self.rejected_price.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DqStatsSnapshot {
    pub total: u64,
    pub passed: u64,
    pub rejected_stale: u64,
    pub rejected_halted: u64,
    pub rejected_jump: u64,
    pub rejected_ex_rights: u64,
    pub rejected_price: u64,
}

impl DqStatsSnapshot {
    pub fn dirty_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        1.0 - self.passed as f64 / self.total as f64
    }

    pub fn summary(&self) -> String {
        format!(
            "DQ: total={} passed={} ({}%) stale={} halt={} jump={} ex_rights={} price={}",
            self.total,
            self.passed,
            (100.0 - self.dirty_rate() * 100.0).round(),
            self.rejected_stale,
            self.rejected_halted,
            self.rejected_jump,
            self.rejected_ex_rights,
            self.rejected_price,
        )
    }
}

// ============================================================================
// 停牌缓存
// ============================================================================

static HALTED_CODES: Lazy<RwLock<HashSet<String>>> = Lazy::new(|| RwLock::new(HashSet::new()));

pub fn mark_halted(code: &str, halted: bool) {
    if let Ok(mut guard) = HALTED_CODES.write() {
        if halted {
            guard.insert(code.to_string());
        } else {
            guard.remove(code);
        }
    }
}

fn is_halted(code: &str) -> bool {
    HALTED_CODES
        .read()
        .map(|g| g.contains(code))
        .unwrap_or(false)
}

// ============================================================================
// 除权除息日缓存
// ============================================================================

static EX_RIGHTS_DATES: Lazy<RwLock<HashSet<(String, NaiveDate)>>> =
    Lazy::new(|| RwLock::new(HashSet::new()));

pub fn mark_ex_rights(code: &str, date: NaiveDate) {
    if let Ok(mut guard) = EX_RIGHTS_DATES.write() {
        guard.insert((code.to_string(), date));
    }
}

fn is_ex_rights(code: &str, date: NaiveDate) -> bool {
    EX_RIGHTS_DATES
        .read()
        .map(|g| g.contains(&(code.to_string(), date)))
        .unwrap_or(false)
}

// ============================================================================
// 核心质量检查
// ============================================================================

/// Tick 数据结构（简化版，与现有模块解耦）
#[derive(Debug, Clone)]
pub struct Tick {
    pub code: String,
    pub price: f64,
    pub change_pct: f64,
    pub volume: f64,
    pub update_time: DateTime<Local>,
}

/// 前一个 Tick（用于 jump 检测）
#[derive(Debug, Clone)]
pub struct PrevTick {
    pub price: f64,
    pub update_time: DateTime<Local>,
}

/// 质量门配置
#[derive(Debug, Clone)]
pub struct DqConfig {
    /// 最大允许过期秒数（默认 120）
    pub max_staleness_secs: u64,
    /// 价格跳空阈值（默认 5.0%）
    pub jump_threshold_pct: f64,
    /// 涨跌幅异常上限（默认 20%，超过可能是脏数据）
    pub max_change_pct: f64,
}

impl Default for DqConfig {
    fn default() -> Self {
        Self {
            max_staleness_secs: std::env::var("DQ_STALENESS_MAX_SEC")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(120),
            jump_threshold_pct: std::env::var("DQ_JUMP_THRESHOLD_PCT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(5.0),
            max_change_pct: 20.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FreshnessConfig {
    pub quote_max_age_secs: u64,
    pub position_max_age_secs: u64,
    pub nav_max_age_secs: u64,
    pub daily_max_age_secs: u64,
}

impl FreshnessConfig {
    pub fn max_age_secs(&self, data_type: FreshnessDataType) -> u64 {
        match data_type {
            FreshnessDataType::Quote => self.quote_max_age_secs,
            FreshnessDataType::Position => self.position_max_age_secs,
            FreshnessDataType::Nav => self.nav_max_age_secs,
            FreshnessDataType::Daily => self.daily_max_age_secs,
        }
    }
}

impl Default for FreshnessConfig {
    fn default() -> Self {
        Self {
            quote_max_age_secs: 5,
            position_max_age_secs: 30,
            nav_max_age_secs: 24 * 3600,
            daily_max_age_secs: 24 * 3600,
        }
    }
}

pub fn validate_freshness(
    data_type: FreshnessDataType,
    update_time: DateTime<Local>,
    freshness: &FreshnessConfig,
    stats: &DqStats,
) -> Result<(), DqRejectReason> {
    stats.total_ticks.fetch_add(1, Ordering::Relaxed);
    let max_secs = freshness.max_age_secs(data_type);
    let age_secs = Local::now()
        .signed_duration_since(update_time)
        .num_seconds()
        .max(0) as u64;
    if age_secs > max_secs {
        stats.rejected_stale.fetch_add(1, Ordering::Relaxed);
        return Err(DqRejectReason::Stale { age_secs, max_secs });
    }
    stats.passed.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

pub fn validate_daily_freshness(
    data_date: NaiveDate,
    now: DateTime<Local>,
    freshness: &FreshnessConfig,
    stats: &DqStats,
) -> Result<(), DqRejectReason> {
    stats.total_ticks.fetch_add(1, Ordering::Relaxed);
    let today = now.date_naive();
    let mut effective_today = today;
    if !calendar::is_trading_day(today) {
        effective_today = calendar::prev_trading_day(today);
    }
    if data_date > effective_today {
        stats.rejected_stale.fetch_add(1, Ordering::Relaxed);
        return Err(DqRejectReason::Stale {
            age_secs: 0,
            max_secs: freshness.daily_max_age_secs,
        });
    }
    let max_trading_days = (freshness.daily_max_age_secs / (24 * 3600)).max(1) as usize;
    let allowed_dates = calendar::recent_trading_days(effective_today, max_trading_days + 1);
    if !allowed_dates.contains(&data_date) {
        let age_secs = (effective_today - data_date).num_days().max(0) as u64 * 24 * 3600;
        stats.rejected_stale.fetch_add(1, Ordering::Relaxed);
        return Err(DqRejectReason::Stale {
            age_secs,
            max_secs: freshness.daily_max_age_secs,
        });
    }
    stats.passed.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// 主校验函数：对每个 tick 过五道门。
/// 返回 Ok(()) 表示通过，Err(reason) 表示被拒绝。
pub fn validate_tick(tick: &Tick, prev: Option<&PrevTick>, config: &DqConfig, stats: &DqStats) -> Result<(), DqRejectReason> {
    stats.total_ticks.fetch_add(1, Ordering::Relaxed);

    // Gate 1: Staleness
    let now = Local::now();
    let age = now.signed_duration_since(tick.update_time);
    if age.num_seconds() > config.max_staleness_secs as i64 {
        let r = DqRejectReason::Stale {
            age_secs: age.num_seconds() as u64,
            max_secs: config.max_staleness_secs,
        };
        stats.rejected_stale.fetch_add(1, Ordering::Relaxed);
        return Err(r);
    }

    // Gate 2: Halt
    if is_halted(&tick.code) {
        stats.rejected_halted.fetch_add(1, Ordering::Relaxed);
        return Err(DqRejectReason::Halted);
    }

    // Gate 3: Ex-rights
    let today = now.date_naive();
    if is_ex_rights(&tick.code, today) {
        stats.rejected_ex_rights.fetch_add(1, Ordering::Relaxed);
        return Err(DqRejectReason::ExRights);
    }

    // Gate 4: Price reasonability
    if tick.price <= 0.0 || tick.price.is_nan() || tick.price.is_infinite() {
        let r = DqRejectReason::UnreasonablePrice {
            price: tick.price,
            last_close: 0.0,
        };
        stats.rejected_price.fetch_add(1, Ordering::Relaxed);
        return Err(r);
    }

    if tick.change_pct.abs() > config.max_change_pct {
        let r = DqRejectReason::UnreasonablePrice {
            price: tick.price,
            last_close: tick.price / (1.0 + tick.change_pct / 100.0),
        };
        stats.rejected_price.fetch_add(1, Ordering::Relaxed);
        return Err(r);
    }

    // Gate 5: Jump detection (needs previous tick)
    if let Some(prev) = prev {
        let jump_pct = ((tick.price - prev.price) / prev.price * 100.0).abs();
        if jump_pct > config.jump_threshold_pct {
            let r = DqRejectReason::Jump {
                change_pct: jump_pct,
                threshold_pct: config.jump_threshold_pct,
            };
            stats.rejected_jump.fetch_add(1, Ordering::Relaxed);
            return Err(r);
        }
    }

    stats.passed.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// 快速校验（不跟踪统计，用于测试或一次性检查）
pub fn quick_validate(tick: &Tick, config: &DqConfig) -> Result<(), DqRejectReason> {
    let stats = DqStats::new();
    validate_tick(tick, None, config, &stats)
}

/// 日线最小质检：OHLC 一致性 + 开盘跳空异常。
///
/// - OHLC 一致性：`high >= max(open, close)` 且 `low <= min(open, close)` 且 `high >= low`
/// - 跳空异常：相邻交易日 `open` 相对前一日 `close` 的绝对跳变超过阈值
///
/// 返回 Err(原因) 表示应阻断后续分析。
pub fn validate_daily_kline_quality(kline: &[KlineData], max_gap_pct: f64) -> Result<(), String> {
    if kline.is_empty() {
        return Err("日线数据为空".to_string());
    }

    let mut bars: Vec<&KlineData> = kline.iter().collect();
    bars.sort_by_key(|k| k.date);

    for b in &bars {
        if !b.open.is_finite() || !b.high.is_finite() || !b.low.is_finite() || !b.close.is_finite() {
            return Err(format!("{} 存在 NaN/Inf 价格字段", b.date));
        }
        if b.open <= 0.0 || b.high <= 0.0 || b.low <= 0.0 || b.close <= 0.0 {
            return Err(format!("{} 存在非正价格（open/high/low/close）", b.date));
        }
        let max_oc = b.open.max(b.close);
        let min_oc = b.open.min(b.close);
        if b.high + 1e-9 < max_oc || b.low - 1e-9 > min_oc || b.high + 1e-9 < b.low {
            return Err(format!(
                "{} OHLC 不一致: open={:.3} high={:.3} low={:.3} close={:.3}",
                b.date, b.open, b.high, b.low, b.close
            ));
        }
    }

    for w in bars.windows(2) {
        let prev = w[0];
        let cur = w[1];
        if prev.close <= 0.0 {
            continue;
        }
        let gap_pct = ((cur.open - prev.close) / prev.close * 100.0).abs();
        if gap_pct > max_gap_pct {
            return Err(format!(
                "{} 开盘跳变异常 {:.2}% (> {:.2}%)，prev_close={:.3} open={:.3}",
                cur.date, gap_pct, max_gap_pct, prev.close, cur.open
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn make_tick(code: &str, price: f64, change_pct: f64) -> Tick {
        Tick {
            code: code.to_string(),
            price,
            change_pct,
            volume: 10000.0,
            update_time: Local::now(),
        }
    }

    fn stale_tick() -> Tick {
        Tick {
            code: "000001".to_string(),
            price: 10.0,
            change_pct: 1.0,
            volume: 10000.0,
            update_time: Local::now() - Duration::seconds(300),
        }
    }

    fn make_kline(date: NaiveDate, open: f64, high: f64, low: f64, close: f64) -> KlineData {
        KlineData {
            date,
            open,
            high,
            low,
            close,
            volume: 1000.0,
            amount: 1000.0 * close,
            pct_chg: 0.0,
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
        }
    }

    #[test]
    fn test_passes_normal_tick() {
        let config = DqConfig::default();
        let tick = make_tick("000001", 10.0, 1.5);
        let stats = DqStats::new();
        assert!(validate_tick(&tick, None, &config, &stats).is_ok());
    }

    #[test]
    fn test_rejects_stale_tick() {
        let config = DqConfig {
            max_staleness_secs: 60,
            ..DqConfig::default()
        };
        let tick = stale_tick();
        let stats = DqStats::new();
        let r = validate_tick(&tick, None, &config, &stats);
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), DqRejectReason::Stale { .. }));
    }

    #[test]
    fn test_rejects_halted_stock() {
        mark_halted("000002", true);
        let config = DqConfig::default();
        let tick = make_tick("000002", 10.0, 1.0);
        let stats = DqStats::new();
        let r = validate_tick(&tick, None, &config, &stats);
        assert!(r.is_err());
        assert_eq!(r.unwrap_err(), DqRejectReason::Halted);
        mark_halted("000002", false); // cleanup
    }

    #[test]
    fn test_rejects_zero_price() {
        let config = DqConfig::default();
        let tick = make_tick("000001", 0.0, 0.0);
        let stats = DqStats::new();
        let r = validate_tick(&tick, None, &config, &stats);
        assert!(r.is_err());
        assert!(matches!(
            r.unwrap_err(),
            DqRejectReason::UnreasonablePrice { .. }
        ));
    }

    #[test]
    fn test_rejects_negative_price() {
        let config = DqConfig::default();
        let tick = make_tick("000001", -5.0, 0.0);
        let stats = DqStats::new();
        let r = validate_tick(&tick, None, &config, &stats);
        assert!(r.is_err());
    }

    #[test]
    fn test_rejects_extreme_change() {
        let config = DqConfig::default(); // max_change_pct = 20.0
        let tick = make_tick("000001", 100.0, 25.0); // 25% change
        let stats = DqStats::new();
        let r = validate_tick(&tick, None, &config, &stats);
        assert!(r.is_err());
    }

    #[test]
    fn test_rejects_jump() {
        let config = DqConfig {
            jump_threshold_pct: 3.0,
            ..DqConfig::default()
        };
        let prev = PrevTick {
            price: 100.0,
            update_time: Local::now(),
        };
        let tick = make_tick("000001", 105.0, 5.0); // 5% jump from 100 → 105
        let stats = DqStats::new();
        let r = validate_tick(&tick, Some(&prev), &config, &stats);
        assert!(r.is_err());
        assert!(matches!(r.unwrap_err(), DqRejectReason::Jump { .. }));
    }

    #[test]
    fn test_small_jump_passes() {
        let config = DqConfig::default(); // 5% threshold
        let prev = PrevTick {
            price: 100.0,
            update_time: Local::now(),
        };
        let tick = make_tick("000001", 103.0, 3.0); // 3% jump
        let stats = DqStats::new();
        assert!(validate_tick(&tick, Some(&prev), &config, &stats).is_ok());
    }

    #[test]
    fn test_ex_rights_rejected() {
        let today = Local::now().date_naive();
        mark_ex_rights("000003", today);
        let config = DqConfig::default();
        let tick = make_tick("000003", 10.0, -2.0);
        let stats = DqStats::new();
        let r = validate_tick(&tick, None, &config, &stats);
        assert!(r.is_err());
        assert_eq!(r.unwrap_err(), DqRejectReason::ExRights);
    }

    #[test]
    fn test_dq_stats_snapshot() {
        let stats = DqStats::new();
        let config = DqConfig::default();

        // Pass a few
        for _ in 0..8 {
            let tick = make_tick("000001", 10.0, 1.0);
            let _ = validate_tick(&tick, None, &config, &stats);
        }
        // Reject one stale
        let st = stale_tick();
        let _ = validate_tick(&st, None, &config, &stats);

        let snap = stats.snapshot();
        assert_eq!(snap.total, 9);
        assert_eq!(snap.passed, 8);
        assert_eq!(snap.rejected_stale, 1);
        assert!(snap.dirty_rate() > 0.0);
        let summary = snap.summary();
        assert!(summary.contains("DQ:"));
    }

    #[test]
    fn test_reject_reason_labels() {
        assert_eq!(
            DqRejectReason::Stale {
                age_secs: 200,
                max_secs: 120
            }
            .label(),
            "数据过期"
        );
        assert_eq!(DqRejectReason::Halted.label(), "停牌");
        assert_eq!(
            DqRejectReason::Jump {
                change_pct: 10.0,
                threshold_pct: 5.0
            }
            .label(),
            "价格跳空"
        );
        assert_eq!(DqRejectReason::ExRights.label(), "除权除息");
        assert_eq!(
            DqRejectReason::UnreasonablePrice {
                price: 0.0,
                last_close: 10.0
            }
            .label(),
            "价格异常"
        );
    }

    #[test]
    fn test_typed_freshness_quote_threshold() {
        let cfg = FreshnessConfig::default();
        let stats = DqStats::new();
        let fresh = Local::now() - Duration::seconds(3);
        assert!(validate_freshness(FreshnessDataType::Quote, fresh, &cfg, &stats).is_ok());

        let stale = Local::now() - Duration::seconds(6);
        let r = validate_freshness(FreshnessDataType::Quote, stale, &cfg, &stats);
        assert!(matches!(r, Err(DqRejectReason::Stale { .. })));
    }

    #[test]
    fn test_typed_freshness_position_threshold() {
        let cfg = FreshnessConfig::default();
        let stats = DqStats::new();
        let stale = Local::now() - Duration::seconds(31);
        let r = validate_freshness(FreshnessDataType::Position, stale, &cfg, &stats);
        assert!(matches!(r, Err(DqRejectReason::Stale { .. })));
    }

    #[test]
    fn test_daily_freshness_within_one_trading_day() {
        let cfg = FreshnessConfig::default();
        let stats = DqStats::new();
        let monday = Local.with_ymd_and_hms(2026, 6, 22, 10, 0, 0).unwrap();
        let friday = NaiveDate::from_ymd_opt(2026, 6, 19).unwrap();
        assert!(validate_daily_freshness(friday, monday, &cfg, &stats).is_ok());
    }

    #[test]
    fn test_daily_freshness_stale_rejected() {
        let cfg = FreshnessConfig::default();
        let stats = DqStats::new();
        let monday = Local.with_ymd_and_hms(2026, 6, 22, 10, 0, 0).unwrap();
        let thursday = NaiveDate::from_ymd_opt(2026, 6, 18).unwrap();
        let r = validate_daily_freshness(thursday, monday, &cfg, &stats);
        assert!(matches!(r, Err(DqRejectReason::Stale { .. })));
    }

    #[test]
    fn test_daily_kline_quality_ohlc_invalid_rejected() {
        let d = NaiveDate::from_ymd_opt(2026, 6, 20).unwrap();
        let bars = vec![make_kline(d, 10.0, 9.8, 9.5, 9.9)]; // high < open
        let r = validate_daily_kline_quality(&bars, 20.0);
        assert!(r.is_err());
    }

    #[test]
    fn test_daily_kline_quality_gap_jump_rejected() {
        let d1 = NaiveDate::from_ymd_opt(2026, 6, 19).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 6, 20).unwrap();
        let bars = vec![
            make_kline(d1, 10.0, 10.5, 9.8, 10.0),
            make_kline(d2, 13.0, 13.2, 12.8, 13.1), // 开盘相对前收 +30%
        ];
        let r = validate_daily_kline_quality(&bars, 20.0);
        assert!(r.is_err());
    }

    #[test]
    fn test_daily_kline_quality_passes() {
        let d1 = NaiveDate::from_ymd_opt(2026, 6, 19).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 6, 20).unwrap();
        let bars = vec![
            make_kline(d1, 10.0, 10.5, 9.8, 10.0),
            make_kline(d2, 10.3, 10.6, 10.1, 10.4),
        ];
        assert!(validate_daily_kline_quality(&bars, 20.0).is_ok());
    }
}
