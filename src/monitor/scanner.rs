//! 阶梯轮询扫描器。
//!
//! 按层级轮询不同标的，集成 RateBudget + DQ Gate + 交易日历门控。

use crate::calendar::{self, MarketSession};
use crate::monitor::data_quality::{
    validate_freshness, validate_tick, DqConfig, DqStats, FreshnessConfig, FreshnessDataType, Tick,
};
use crate::monitor::rate_budget::RateBudget;
use log::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanLevel {
    L0 = 0,
    L1 = 1,
    L2 = 2,
    L3 = 3,
}

impl ScanLevel {
    pub fn default_interval_secs(&self) -> u64 {
        match self {
            ScanLevel::L0 => 30,
            ScanLevel::L1 => 30,
            ScanLevel::L2 => 60,
            ScanLevel::L3 => 300,
        }
    }
}

/// 被扫描的标的
#[derive(Debug, Clone)]
pub struct ScanTarget {
    pub code: String,
    pub name: String,
    pub level: ScanLevel,
    pub t1_locked: bool,
}

/// 扫描结果
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub tick: Option<Tick>,
    pub dq_passed: bool,
    pub dq_reason: Option<String>,
}

/// 阶梯轮询扫描器
pub struct TieredScanner {
    targets: Vec<ScanTarget>,
    budgets: Vec<RateBudget>,
    dq_config: DqConfig,
    freshness: FreshnessConfig,
    pub dq_stats: DqStats,
}

impl TieredScanner {
    pub fn new(targets: Vec<ScanTarget>) -> Self {
        let budgets = vec![
            RateBudget::with_window(60, 60), // L0: 60次/分钟
            RateBudget::with_window(30, 60), // L1: 30次/分钟
            RateBudget::with_window(10, 60), // L2: 10次/分钟
            RateBudget::with_window(5, 60),  // L3: 5次/分钟
        ];
        Self {
            targets,
            budgets,
            dq_config: DqConfig::default(),
            freshness: FreshnessConfig::default(),
            dq_stats: DqStats::new(),
        }
    }

    /// 判断现在是否应该扫描
    pub fn should_scan(&self) -> bool {
        let s = calendar::current_session();
        matches!(
            s,
            MarketSession::Morning | MarketSession::Afternoon | MarketSession::Auction
        )
    }

    /// 获取某层级的有效轮询间隔
    pub fn effective_interval(&self, level: ScanLevel, base_secs: u64) -> u64 {
        let budget = &self.budgets[level as usize];
        let usage = budget.used() as f64 / budget.limit().max(1) as f64;
        if usage > 0.8 {
            base_secs * 2
        } else if usage > 0.5 {
            (base_secs as f64 * 1.5) as u64
        } else {
            base_secs
        }
    }

    /// 尝试获取扫描配额
    pub fn try_acquire(&self, level: ScanLevel) -> bool {
        self.budgets[level as usize].try_acquire()
    }

    /// 为指定层级的目标生成待扫描列表
    pub fn targets_at(&self, level: ScanLevel) -> Vec<&ScanTarget> {
        self.targets.iter().filter(|t| t.level == level).collect()
    }

    /// 验证一个 tick 是否通过数据质量门
    pub fn validate(&self, tick: &Tick) -> ScanResult {
        if let Err(r) = validate_freshness(
            FreshnessDataType::Quote,
            tick.update_time,
            &self.freshness,
            &self.dq_stats,
        ) {
            return ScanResult {
                tick: None,
                dq_passed: false,
                dq_reason: Some(r.label().into()),
            };
        }
        let prev = None; // 简化：不追踪前值
        match validate_tick(tick, prev, &self.dq_config, &self.dq_stats) {
            Ok(()) => ScanResult {
                tick: Some(tick.clone()),
                dq_passed: true,
                dq_reason: None,
            },
            Err(r) => ScanResult {
                tick: None,
                dq_passed: false,
                dq_reason: Some(r.label().into()),
            },
        }
    }

    /// DQ 统计摘要
    pub fn dq_summary(&self) -> String {
        self.dq_stats.snapshot().summary()
    }

    /// 从严格 portfolio API 一次性加载持仓和自选。任何源错误使整批失败。
    pub fn load_portfolio_targets(
    ) -> Result<(Vec<crate::portfolio::Position>, Vec<ScanTarget>), String> {
        let positions = crate::portfolio::get_positions()?;
        let watchlist = crate::portfolio::get_watchlist()?;
        let targets = build_portfolio_targets(&positions, &watchlist);
        info!("[Scanner] 加载 {} 只持仓股", positions.len());
        info!(
            "[Scanner] 加载 {} 只自选股",
            targets.iter().filter(|t| t.level == ScanLevel::L2).count()
        );
        Ok((positions, targets))
    }
}

fn build_portfolio_targets(
    positions: &[crate::portfolio::Position],
    watchlist: &[crate::portfolio::Position],
) -> Vec<ScanTarget> {
    let mut targets = Vec::with_capacity(positions.len() + watchlist.len());
    for position in positions {
        targets.push(ScanTarget {
            code: position.code.clone(),
            name: position.name.clone(),
            level: ScanLevel::L1,
            t1_locked: false,
        });
    }
    for watched in watchlist {
        if !targets.iter().any(|target| target.code == watched.code) {
            targets.push(ScanTarget {
                code: watched.code.clone(),
                name: watched.name.clone(),
                level: ScanLevel::L2,
                t1_locked: false,
            });
        }
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn position(
        code: &str,
        name: &str,
        status: crate::portfolio::PositionStatus,
    ) -> crate::portfolio::Position {
        let holding = status == crate::portfolio::PositionStatus::Holding;
        crate::portfolio::Position {
            code: code.to_string(),
            name: name.to_string(),
            shares: if holding { 100 } else { 0 },
            cost_price: if holding { 10.0 } else { 0.0 },
            hard_stop: None,
            added_at: NaiveDate::from_ymd_opt(2026, 7, 18).expect("valid date"),
            status,
            sector: String::new(),
            is_st: false,
            star_st: false,
        }
    }

    #[test]
    fn test_scan_level_intervals() {
        assert_eq!(ScanLevel::L0.default_interval_secs(), 30);
        assert_eq!(ScanLevel::L3.default_interval_secs(), 300);
    }

    #[test]
    fn test_scanner_creation() {
        let targets = vec![ScanTarget {
            code: "TEST_CODE_000001".into(),
            name: "测试".into(),
            level: ScanLevel::L1,
            t1_locked: false,
        }];
        let scanner = TieredScanner::new(targets);
        assert!(scanner.try_acquire(ScanLevel::L1));
    }

    #[test]
    fn portfolio_targets_keep_real_names_and_deduplicate_codes() {
        let positions = vec![position(
            "TEST_CODE_000001",
            "平安银行",
            crate::portfolio::PositionStatus::Holding,
        )];
        let watchlist = vec![
            position(
                "TEST_CODE_000001",
                "不应覆盖",
                crate::portfolio::PositionStatus::Watching,
            ),
            position(
                "TEST_CODE_600519",
                "贵州茅台",
                crate::portfolio::PositionStatus::Watching,
            ),
        ];

        let targets = build_portfolio_targets(&positions, &watchlist);

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].name, "平安银行");
        assert_eq!(targets[0].level, ScanLevel::L1);
        assert_eq!(targets[1].name, "贵州茅台");
        assert_eq!(targets[1].level, ScanLevel::L2);
    }

    #[test]
    fn test_scanner_quota_exhaustion() {
        let targets = vec![ScanTarget {
            code: "t".into(),
            name: "t".into(),
            level: ScanLevel::L3,
            t1_locked: false,
        }];
        let scanner = TieredScanner::new(targets);
        // L3 budget is 5/min
        for _ in 0..5 {
            assert!(scanner.try_acquire(ScanLevel::L3));
        }
        assert!(!scanner.try_acquire(ScanLevel::L3));
    }

    #[test]
    fn test_validate_tick() {
        let targets = vec![ScanTarget {
            code: "TEST_CODE_000001".into(),
            name: "测试".into(),
            level: ScanLevel::L1,
            t1_locked: false,
        }];
        let scanner = TieredScanner::new(targets);
        let tick = Tick {
            code: "TEST_CODE_000001".into(),
            price: 10.0,
            change_pct: 1.0,
            volume: 1000.0,
            update_time: chrono::Local::now(),
        };
        let r = scanner.validate(&tick);
        assert!(r.dq_passed);
    }

    #[test]
    fn test_validate_stale_tick() {
        let targets = vec![ScanTarget {
            code: "TEST_CODE_000001".into(),
            name: "测试".into(),
            level: ScanLevel::L1,
            t1_locked: false,
        }];
        let scanner = TieredScanner::new(targets);
        let tick = Tick {
            code: "TEST_CODE_000001".into(),
            price: 10.0,
            change_pct: 1.0,
            volume: 1000.0,
            update_time: chrono::Local::now() - chrono::Duration::seconds(300),
        };
        let r = scanner.validate(&tick);
        assert!(!r.dq_passed);
        assert!(r.dq_reason.is_some());
    }

    #[test]
    fn test_effective_interval_increases_under_load() {
        let targets = vec![ScanTarget {
            code: "t".into(),
            name: "t".into(),
            level: ScanLevel::L0,
            t1_locked: false,
        }];
        let scanner = TieredScanner::new(targets);
        let base = scanner.effective_interval(ScanLevel::L0, 30);
        assert_eq!(base, 30); // No load yet

        // Exhaust budget
        for _ in 0..60 {
            scanner.try_acquire(ScanLevel::L0);
        }
        let stressed = scanner.effective_interval(ScanLevel::L0, 30);
        assert!(stressed > 30, "高负载下间隔应增加");
    }

    #[test]
    fn test_should_scan_depends_on_session() {
        let targets = vec![ScanTarget {
            code: "t".into(),
            name: "t".into(),
            level: ScanLevel::L1,
            t1_locked: false,
        }];
        let scanner = TieredScanner::new(targets);
        let _ = scanner.should_scan(); // Should not panic
    }
}
