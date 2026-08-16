//! 数据质量门（DQ Gate）。
//!
//! 每条进入检测引擎的 tick 必须先过这四道校验：
//! 1. staleness  — 数据是否过期（超过 N 秒未更新）
//! 2. halt       — 股票是否停牌
//! 3. jump       — 价格是否发生异常跳空
//! 4. price_ok   — 价格是否在合理范围（非零、非负、涨跌幅合理）
//!
//! 脏数据 → 丢弃 + 计数（供系统自监控），不进检测引擎。

use chrono::{DateTime, Local, NaiveDate};
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
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
            "DQ: total={} passed={} ({}%) stale={} halt={} jump={} price={}",
            self.total,
            self.passed,
            (100.0 - self.dirty_rate() * 100.0).round(),
            self.rejected_stale,
            self.rejected_halted,
            self.rejected_jump,
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
// 停牌时间段缓存 (v11-P0-3 commit 2 新建)
//
// 与 HALTED_CODES 不同: HALTED_CODES 只存"现在是否停牌", HALTED_PERIODS 存历史时间段.
// 数据来源: ① K 线缺口推断 (commit 2) ② 交易所公告 (留 P0-4).
//
// 解决"幸存者偏差": apply_limit_flags_inplace 之前 is_suspended 永远 false,
// 回测在停牌日虚成交 → 虚高收益. 现在 is_halted_period(code, date) 查 HALTED_PERIODS.
// ============================================================================

type HaltedPeriod = (NaiveDate, NaiveDate);
type HaltedPeriodsByCode = HashMap<String, Vec<HaltedPeriod>>;
static HALTED_PERIODS: Lazy<RwLock<HaltedPeriodsByCode>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// 喂入停牌时间段. (from, to) 半开区间 [from, to].
pub fn mark_halted_period(code: &str, from: NaiveDate, to: NaiveDate) {
    if let Ok(mut guard) = HALTED_PERIODS.write() {
        guard
            .entry(code.to_string())
            .or_insert_with(Vec::new)
            .push((from, to));
    }
}

/// 查询某股在某日是否处于停牌期间.
pub fn is_halted_period(code: &str, date: NaiveDate) -> bool {
    let periods = match HALTED_PERIODS.read() {
        Ok(g) => g,
        Err(_) => return false,
    };
    match periods.get(code) {
        Some(ps) => ps.iter().any(|&(from, to)| date >= from && date <= to),
        None => false,
    }
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
pub fn validate_tick(
    tick: &Tick,
    prev: Option<&PrevTick>,
    config: &DqConfig,
    stats: &DqStats,
) -> Result<(), DqRejectReason> {
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

    // Gate 3: Price reasonability
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

    // Gate 4: Jump detection (needs previous tick)
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

pub const MAX_UNCONFIRMED_ADJACENT_DAILY_CHANGE_PCT: f64 = 20.0;

/// One structurally valid adjacent daily move that requires BR-171 operator confirmation.
#[derive(Debug, Clone, PartialEq)]
pub struct AdjacentDailyChange {
    pub code: String,
    pub previous_date: NaiveDate,
    pub current_date: NaiveDate,
    pub previous_close: f64,
    pub current_close: f64,
    pub change_pct: f64,
}

/// BR-092 structural validation stage.
///
/// This stage validates prices, volume, amount, trading-day continuity and the
/// provider's reported percentage. It does **not** admit moves above the BR-171
/// threshold: every such move is returned to the final evidence-bound admission
/// stage. Keeping this function crate-private prevents a transport router from
/// turning structural validation into a production admission shortcut.
pub(crate) fn validate_daily_kline_structure(
    kline: &mut [KlineData],
    code: &str,
) -> Result<Vec<AdjacentDailyChange>, String> {
    if kline.is_empty() {
        return Err("日线数据为空".to_string());
    }
    for b in kline.iter() {
        if !b.open.is_finite() || !b.high.is_finite() || !b.low.is_finite() || !b.close.is_finite()
        {
            return Err(format!("[{code}] {} 含 NaN/Inf 价格", b.date));
        }
        if b.open <= 0.0 || b.high <= 0.0 || b.low <= 0.0 || b.close <= 0.0 {
            return Err(format!("[{code}] {} 含非正价格", b.date));
        }
        let max_oc = b.open.max(b.close);
        let min_oc = b.open.min(b.close);
        if b.high + 1e-9 < max_oc || b.low - 1e-9 > min_oc || b.high + 1e-9 < b.low {
            return Err(format!(
                "[{code}] {} OHLC 不一致 open={:.3} high={:.3} low={:.3} close={:.3}",
                b.date, b.open, b.high, b.low, b.close
            ));
        }
        if !b.volume.is_finite() || b.volume <= 0.0 {
            return Err(format!("[{code}] {} 成交量无效: {}", b.date, b.volume));
        }
        if !b.amount.is_finite() || (b.volume > 0.0 && b.amount <= 0.0) {
            return Err(format!("[{code}] {} 成交额无效: {}", b.date, b.amount));
        }
        if !b.pct_chg.is_finite() {
            return Err(format!("[{code}] {} 涨跌幅无效: {}", b.date, b.pct_chg));
        }
        if !calendar::is_trading_day(b.date) {
            return Err(format!("[{code}] {} 不是交易日", b.date));
        }
    }

    kline.sort_by_key(|b| b.date);
    let mut pending_confirmations = Vec::new();
    for w in kline.windows(2) {
        let prev = &w[0];
        let cur = &w[1];
        if prev.date == cur.date {
            return Err(format!("[{code}] 重复交易日: {}", cur.date));
        }
        let expected = calendar::next_trading_day(prev.date);
        if cur.date != expected {
            // BR-228: 停牌容忍 — 缺失交易日按停牌处理 (单次 ≤ 5 个交易日),
            // 跳过该相邻对 (不计算涨跌幅), 记录警告; 超过阈值仍硬拒。
            let mut gap_days = 0usize;
            let mut probe = expected;
            while probe < cur.date && gap_days <= 5 {
                probe = calendar::next_trading_day(probe);
                gap_days += 1;
            }
            if gap_days <= 5 {
                log::warn!(
                    "[{code}] 停牌容忍: {} 后为 {} (缺失 {gap_days} 个交易日, 视为停牌)",
                    prev.date,
                    cur.date
                );
                continue;
            }
            return Err(format!(
                "[{code}] 交易日断档: {} 后应为 {}, 实际为 {}",
                prev.date, expected, cur.date
            ));
        }
        let computed_pct = (cur.close - prev.close) / prev.close * 100.0;
        if cur.pct_chg.abs() > 1e-9 && (cur.pct_chg - computed_pct).abs() > 0.25 {
            return Err(format!(
                "[{code}] {} 源涨跌幅不一致: source={:.3}% computed={computed_pct:.3}%",
                cur.date, cur.pct_chg
            ));
        }
        if computed_pct.abs() > MAX_UNCONFIRMED_ADJACENT_DAILY_CHANGE_PCT {
            pending_confirmations.push(AdjacentDailyChange {
                code: code.to_owned(),
                previous_date: prev.date,
                current_date: cur.date,
                previous_close: prev.close,
                current_close: cur.close,
                change_pct: computed_pct,
            });
        }
    }

    // 还原降序契约: 3 个 provider 都返回降序 (最新在前), 质检内部 sort_by_key 改成升序做跳空检测
    // 后必须 sort 回降序, 否则下游 pipeline/data.rs:49 (`data[0].date` → Stale) 必 bail,
    // chip_distribution.rs:94 / financials.rs:155 (用 .rev() 假设降序) 全错位.
    kline.sort_by_key(|item| std::cmp::Reverse(item.date));

    Ok(pending_confirmations)
}

/// BR-092 strict daily validation with an evidence-bound BR-171 confirmation resolver.
///
/// The resolver must return `true` only for a durable confirmation matching the
/// current provider/lifecycle evidence. This function owns value/date validation;
/// callers own the provider/batch identity used by their confirmation store.
pub fn validate_daily_kline_quality_with_confirmation<Confirm>(
    kline: &mut [KlineData],
    code: &str,
    mut confirm: Confirm,
) -> Result<(), String>
where
    Confirm: FnMut(&AdjacentDailyChange) -> Result<bool, String>,
{
    for change in validate_daily_kline_structure(kline, code)? {
        let confirmed = confirm(&change).map_err(|error| {
            format!(
                "[{}] BR-171 manual_confirmation_lookup_failed {}→{}: {error}",
                change.code, change.previous_date, change.current_date
            )
        })?;
        if !confirmed {
            log::warn!(
                "[BR-171] manual_confirmation_required code={} dates={}→{} \
                 closes={:.6}→{:.6} change={:.4}%",
                change.code,
                change.previous_date,
                change.current_date,
                change.previous_close,
                change.current_close,
                change.change_pct
            );
            return Err(format!(
                "[{}] BR-171 manual_confirmation_required {}→{} \
                 closes={:.6}→{:.6} change={:.4}%",
                change.code,
                change.previous_date,
                change.current_date,
                change.previous_close,
                change.current_close,
                change.change_pct
            ));
        }
    }

    Ok(())
}

/// BR-092 default boundary. No caller may silently auto-confirm a large move.
pub fn validate_daily_kline_quality(kline: &mut [KlineData], code: &str) -> Result<(), String> {
    validate_daily_kline_quality_with_confirmation(kline, code, |_| Ok(false))
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
            code: "TEST_CODE_000001".to_string(),
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
            adjust: crate::data_provider::AdjustType::None,
        }
    }

    #[test]
    fn test_passes_normal_tick() {
        let config = DqConfig::default();
        let tick = make_tick("TEST_CODE_000001", 10.0, 1.5);
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
        mark_halted("TEST_CODE_000002", true);
        let config = DqConfig::default();
        let tick = make_tick("TEST_CODE_000002", 10.0, 1.0);
        let stats = DqStats::new();
        let r = validate_tick(&tick, None, &config, &stats);
        assert!(r.is_err());
        assert_eq!(r.unwrap_err(), DqRejectReason::Halted);
        mark_halted("TEST_CODE_000002", false); // cleanup
    }

    #[test]
    fn test_rejects_zero_price() {
        let config = DqConfig::default();
        let tick = make_tick("TEST_CODE_000001", 0.0, 0.0);
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
        let tick = make_tick("TEST_CODE_000001", -5.0, 0.0);
        let stats = DqStats::new();
        let r = validate_tick(&tick, None, &config, &stats);
        assert!(r.is_err());
    }

    #[test]
    fn test_rejects_extreme_change() {
        let config = DqConfig::default(); // max_change_pct = 20.0
        let tick = make_tick("TEST_CODE_000001", 100.0, 25.0); // 25% change
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
        let tick = make_tick("TEST_CODE_000001", 105.0, 5.0); // 5% jump from 100 → 105
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
        let tick = make_tick("TEST_CODE_000001", 103.0, 3.0); // 3% jump
        let stats = DqStats::new();
        assert!(validate_tick(&tick, Some(&prev), &config, &stats).is_ok());
    }

    #[test]
    fn test_dq_stats_snapshot() {
        let stats = DqStats::new();
        let config = DqConfig::default();

        // Pass a few
        for _ in 0..8 {
            let tick = make_tick("TEST_CODE_000001", 10.0, 1.0);
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
        let monday = Local.with_ymd_and_hms(2026, 7, 6, 10, 0, 0).unwrap();
        let friday = NaiveDate::from_ymd_opt(2026, 7, 3).unwrap();
        assert!(validate_daily_freshness(friday, monday, &cfg, &stats).is_ok());
    }

    #[test]
    fn test_daily_freshness_stale_rejected() {
        let cfg = FreshnessConfig::default();
        let stats = DqStats::new();
        let monday = Local.with_ymd_and_hms(2026, 7, 6, 10, 0, 0).unwrap();
        let thursday = NaiveDate::from_ymd_opt(2026, 7, 2).unwrap();
        let r = validate_daily_freshness(thursday, monday, &cfg, &stats);
        assert!(matches!(r, Err(DqRejectReason::Stale { .. })));
    }

    #[test]
    fn test_daily_kline_quality_ohlc_invalid_rejected() {
        let d = NaiveDate::from_ymd_opt(2026, 6, 20).unwrap();
        let mut bars = vec![make_kline(d, 10.0, 9.8, 9.5, 9.9)];
        let r = validate_daily_kline_quality(&mut bars, "TEST_CODE_000001");
        assert!(r.is_err(), "OHLC 不一致必须整批失败");
        assert_eq!(bars.len(), 1, "失败不得静默删除源行");
    }

    #[test]
    fn br092_large_adjacent_move_requires_manual_confirmation() {
        let d1 = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        let mut bars = vec![
            make_kline(d1, 10.0, 10.5, 9.8, 10.0),
            make_kline(d2, 13.0, 13.2, 12.8, 13.0),
        ];
        bars[1].pct_chg = 30.0;

        let error = validate_daily_kline_quality(&mut bars, "TEST_CODE_000001")
            .expect_err("结构完整的 30% 行情仍须进入人工确认门");
        assert!(error.contains("manual_confirmation_required"));
        assert!(error.contains("30.0000%"));
    }

    #[test]
    fn test_daily_kline_quality_passes() {
        let d1 = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        // 入参顺序故意乱序 (升序), 验证出参被还原为降序 (最新在前)
        let mut bars = vec![
            make_kline(d1, 10.0, 10.5, 9.8, 10.0),
            make_kline(d2, 10.3, 10.6, 10.1, 10.4),
        ];
        assert!(validate_daily_kline_quality(&mut bars, "TEST_CODE_000001").is_ok());
        assert!(
            bars[0].date > bars[1].date,
            "出参必须降序 (最新在前), 实际: {} vs {}",
            bars[0].date,
            bars[1].date
        );
    }

    #[test]
    fn br092_daily_kline_rejects_missing_amount_and_non_finite_pct_change() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let mut missing_amount = vec![make_kline(date, 10.0, 10.5, 9.8, 10.0)];
        missing_amount[0].amount = 0.0;
        assert!(
            validate_daily_kline_quality(&mut missing_amount, "TEST_CODE_000001")
                .expect_err("positive volume with zero amount must fail")
                .contains("成交额")
        );

        let mut bad_pct = vec![make_kline(date, 10.0, 10.5, 9.8, 10.0)];
        bad_pct[0].pct_chg = f64::NAN;
        assert!(
            validate_daily_kline_quality(&mut bad_pct, "TEST_CODE_000001")
                .expect_err("non-finite pct_chg must fail")
                .contains("涨跌幅")
        );
    }

    #[test]
    fn br092_daily_kline_rejects_inconsistent_source_pct_change() {
        let first = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let second = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        let mut bars = vec![
            make_kline(first, 10.0, 10.2, 9.8, 10.0),
            make_kline(second, 10.0, 10.2, 9.8, 10.1),
        ];
        bars[1].pct_chg = 9.0;
        assert!(validate_daily_kline_quality(&mut bars, "TEST_CODE_000001")
            .expect_err("source pct_chg must agree with adjacent closes")
            .contains("涨跌幅不一致"));
    }

    /// Codex review P0 #1 修复验证: 入参是降序, 质检后必须保持降序 (下游契约)
    #[test]
    fn test_daily_kline_quality_preserves_desc_order() {
        // 入参降序: 最新在前 (d2 -> d1)
        let d1 = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        let mut bars = vec![
            make_kline(d2, 10.3, 10.6, 10.1, 10.4), // 最新在前
            make_kline(d1, 10.0, 10.5, 9.8, 10.0),
        ];
        validate_daily_kline_quality(&mut bars, "TEST_CODE_000001").expect("ok");
        assert_eq!(bars[0].date, d2, "出参 [0] 必须是最新 d2");
        assert_eq!(bars[1].date, d1, "出参 [1] 必须是次新 d1");
    }

    /// BR-092: 重复日期是源数据错误，不允许自动去重后继续计算。
    #[test]
    fn test_daily_kline_quality_dedup() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let mut bars = vec![
            make_kline(d, 10.0, 10.5, 9.8, 10.0),
            make_kline(d, 11.0, 11.5, 10.8, 11.0), // 重复日期 (11.00 应该是后到的)
        ];
        let r = validate_daily_kline_quality(&mut bars, "TEST_CODE_000001");
        assert!(r.is_err());
        assert_eq!(bars.len(), 2, "失败不得删除任一重复源行");
    }

    #[test]
    fn test_daily_kline_quality_tolerates_single_day_gap_as_suspension() {
        // BR-228: 单次 ≤5 个交易日的缺口视为停牌, 不再硬拒
        let d1 = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let d3 = NaiveDate::from_ymd_opt(2026, 7, 8).unwrap();
        let mut bars = vec![
            make_kline(d1, 10.0, 10.5, 9.8, 10.0),
            make_kline(d3, 10.2, 10.6, 10.1, 10.4),
        ];
        validate_daily_kline_quality(&mut bars, "TEST_CODE_000001")
            .expect("suspension gap tolerated");
    }

    #[test]
    fn test_daily_kline_quality_rejects_long_gap() {
        // BR-228: 超过 5 个交易日的缺口仍硬拒
        let d1 = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let d3 = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let mut bars = vec![
            make_kline(d1, 10.0, 10.5, 9.8, 10.0),
            make_kline(d3, 10.2, 10.6, 10.1, 10.4),
        ];
        let error = validate_daily_kline_quality(&mut bars, "TEST_CODE_000001")
            .expect_err("long gap must fail");
        assert!(error.contains("交易日断档"));
    }

    #[test]
    fn test_daily_kline_quality_rejects_unconfirmed_adjacent_close_jump() {
        let d1 = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        let mut bars = vec![
            make_kline(d1, 10.0, 10.5, 9.8, 10.0),
            make_kline(d2, 13.0, 13.1, 12.9, 13.0),
        ];
        bars[1].pct_chg = 30.0;

        let error = validate_daily_kline_quality(&mut bars, "TEST_CODE_000001")
            .expect_err("30% close change requires explicit manual confirmation");
        assert!(error.contains("manual_confirmation_required"));
    }

    #[test]
    fn br171_exact_resolver_can_confirm_structurally_valid_large_move() {
        let d1 = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 7, 7).unwrap();
        let mut bars = vec![
            make_kline(d1, 10.0, 10.5, 9.8, 10.0),
            make_kline(d2, 13.0, 13.1, 12.9, 13.0),
        ];
        bars[1].pct_chg = 30.0;

        validate_daily_kline_quality_with_confirmation(&mut bars, "TEST_CODE_000001", |change| {
            Ok(change.previous_date == d1
                && change.current_date == d2
                && (change.change_pct - 30.0).abs() < 1e-9)
        })
        .expect("an exact external confirmation may admit the real move");
    }
}
