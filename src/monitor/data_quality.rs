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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

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
}
