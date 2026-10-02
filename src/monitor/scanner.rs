//! 阶梯轮询扫描器。
//!
//! 按层级轮询不同标的，集成 RateBudget + DQ Gate + 交易日历门控。

use crate::calendar::{self, MarketSession};
use crate::data_gateway::market_data::AdmittedRealtimeQuote;
use crate::data_gateway::BatchEvidence;
use crate::monitor::data_quality::{
    validate_freshness, validate_tick, DqConfig, DqStats, FreshnessConfig, FreshnessDataType, Tick,
};
use crate::monitor::rate_budget::RateBudget;
use chrono::{DateTime, Duration, Utc};
use log::info;
use std::sync::atomic::Ordering;

const SCANNER_QUOTE_MAX_AGE: Duration = Duration::seconds(5);

/// Why an actual admitted quote cannot be consumed by this scanner now.
/// These checks grant no suspension, volume, board-limit or order authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannerQuoteUnavailable {
    SourceFuture,
    ObservedFuture,
    TimestampInversion,
    Expired,
    InvalidPrice,
    InvalidPreviousClose,
    InvalidChangePercent,
}

impl ScannerQuoteUnavailable {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::SourceFuture => "scanner_quote_source_future",
            Self::ObservedFuture => "scanner_quote_observed_future",
            Self::TimestampInversion => "scanner_quote_timestamp_inversion",
            Self::Expired => "scanner_quote_expired",
            Self::InvalidPrice => "scanner_quote_invalid_price",
            Self::InvalidPreviousClose => "scanner_quote_invalid_previous_close",
            Self::InvalidChangePercent => "scanner_quote_invalid_change_percent",
        }
    }
}

/// One immediate consumption check, borrowing the original Gateway quote.
/// It has no public constructor, clock override or deserialization. A caller
/// must check again after an await or before consuming another quote.
#[derive(Debug)]
pub struct CheckedScannerQuote<'quote> {
    quote: &'quote AdmittedRealtimeQuote,
    checked_at: DateTime<Utc>,
}

impl CheckedScannerQuote<'_> {
    pub fn code(&self) -> &str {
        self.quote.code()
    }

    pub fn name(&self) -> &str {
        self.quote.name()
    }

    pub fn price(&self) -> f64 {
        self.quote.price()
    }

    pub fn change_percent(&self) -> f64 {
        self.quote.change_percent()
    }

    pub fn source_at(&self) -> DateTime<Utc> {
        self.quote.source_at()
    }

    pub fn observed_at(&self) -> DateTime<Utc> {
        self.quote.observed_at()
    }

    pub fn checked_at(&self) -> DateTime<Utc> {
        self.checked_at
    }

    pub fn evidence(&self) -> &BatchEvidence {
        self.quote.evidence()
    }
}

#[cfg(test)]
thread_local! {
    static SCANNER_QUOTE_TEST_NOW: std::cell::Cell<Option<DateTime<Utc>>> = const { std::cell::Cell::new(None) };
}

fn scanner_quote_now() -> DateTime<Utc> {
    #[cfg(test)]
    if let Some(now) = SCANNER_QUOTE_TEST_NOW.with(std::cell::Cell::get) {
        return now;
    }
    Utc::now()
}

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

    /// Recheck the sealed quote at the actual point of scanner consumption.
    /// Native quote v1 carries no volume, qualified halt or board-limit fact;
    /// do not manufacture a Tick or apply the legacy blanket 20% threshold.
    pub fn validate_admitted_quote<'quote>(
        &self,
        quote: &'quote AdmittedRealtimeQuote,
    ) -> Result<CheckedScannerQuote<'quote>, ScannerQuoteUnavailable> {
        self.validate_admitted_quote_at(quote, scanner_quote_now())
    }

    fn validate_admitted_quote_at<'quote>(
        &self,
        quote: &'quote AdmittedRealtimeQuote,
        now: DateTime<Utc>,
    ) -> Result<CheckedScannerQuote<'quote>, ScannerQuoteUnavailable> {
        self.dq_stats.total_ticks.fetch_add(1, Ordering::Relaxed);
        let temporal_failure = if quote.source_at() > now {
            Some(ScannerQuoteUnavailable::SourceFuture)
        } else if quote.observed_at() > now {
            Some(ScannerQuoteUnavailable::ObservedFuture)
        } else if quote.source_at() > quote.observed_at() {
            Some(ScannerQuoteUnavailable::TimestampInversion)
        } else if now.signed_duration_since(quote.source_at()) > SCANNER_QUOTE_MAX_AGE {
            Some(ScannerQuoteUnavailable::Expired)
        } else {
            None
        };
        if let Some(reason) = temporal_failure {
            self.dq_stats.rejected_stale.fetch_add(1, Ordering::Relaxed);
            return Err(reason);
        }
        let numeric_failure = if !quote.price().is_finite() || quote.price() <= 0.0 {
            Some(ScannerQuoteUnavailable::InvalidPrice)
        } else if !quote.previous_close().is_finite() || quote.previous_close() <= 0.0 {
            Some(ScannerQuoteUnavailable::InvalidPreviousClose)
        } else if !quote.change_percent().is_finite() {
            Some(ScannerQuoteUnavailable::InvalidChangePercent)
        } else {
            None
        };
        if let Some(reason) = numeric_failure {
            self.dq_stats.rejected_price.fetch_add(1, Ordering::Relaxed);
            return Err(reason);
        }
        self.dq_stats.passed.fetch_add(1, Ordering::Relaxed);
        Ok(CheckedScannerQuote {
            quote,
            checked_at: now,
        })
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

    fn scanner_quote_fixture(
        source_at: DateTime<Utc>,
        observed_at: DateTime<Utc>,
        price: f64,
        previous_close: f64,
        change_percent: f64,
    ) -> AdmittedRealtimeQuote {
        use crate::data_gateway::RealtimeMarketQuote;
        use crate::market_domain::ProviderId;
        let evidence = BatchEvidence {
            provider: ProviderId::Tencent,
            source: "TEST_CODE_scanner_quote".into(),
            source_at: Some(source_at.to_rfc3339()),
            observed_at: observed_at.to_rfc3339(),
            batch_id: "TEST_CODE_scanner_batch".into(),
        };
        AdmittedRealtimeQuote::from_test_fixture(
            RealtimeMarketQuote {
                code: "TEST_CODE_000001".into(),
                name: "TEST_CODE original quote".into(),
                price,
                previous_close,
                change_percent,
                source_at,
                observed_at,
                provider: evidence.provider,
                batch_id: evidence.batch_id.clone(),
            },
            evidence,
        )
        .expect("only cfg(test) TEST_CODE capability")
    }

    fn scanner_quote_source_time() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-28T02:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    struct ScannerQuoteClockGuard(Option<DateTime<Utc>>);

    impl ScannerQuoteClockGuard {
        fn set(now: DateTime<Utc>) -> Self {
            Self(SCANNER_QUOTE_TEST_NOW.with(|clock| clock.replace(Some(now))))
        }
    }

    impl Drop for ScannerQuoteClockGuard {
        fn drop(&mut self) {
            SCANNER_QUOTE_TEST_NOW.with(|clock| clock.set(self.0));
        }
    }

    #[test]
    fn scanner_quote_exact_five_seconds_preserves_original_quote_and_counts_once() {
        let source = scanner_quote_source_time();
        let observed = source + Duration::milliseconds(200);
        // A structural quote check must not invent a blanket board limit.
        let quote = scanner_quote_fixture(source, observed, 13.0, 10.0, 30.0);
        let scanner = TieredScanner::new(vec![]);
        let now = source + Duration::seconds(5);
        let _clock = ScannerQuoteClockGuard::set(now);
        let checked = scanner.validate_admitted_quote(&quote).unwrap();
        assert_eq!(checked.code(), quote.code());
        assert_eq!(checked.name(), quote.name());
        assert_eq!(checked.price(), 13.0);
        assert_eq!(checked.change_percent(), 30.0);
        assert_eq!(checked.source_at(), source);
        assert_eq!(checked.observed_at(), observed);
        assert_eq!(checked.checked_at(), now);
        assert_eq!(checked.evidence(), quote.evidence());
        let stats = scanner.dq_stats.snapshot();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.passed, 1);
        assert_eq!(stats.rejected_stale + stats.rejected_price, 0);
        assert_eq!(stats.rejected_halted + stats.rejected_jump, 0);
    }

    #[test]
    fn scanner_quote_five_seconds_plus_one_nanosecond_rejects_without_truncation() {
        let source = scanner_quote_source_time();
        let quote = scanner_quote_fixture(source, source, 10.0, 10.0, 0.0);
        let scanner = TieredScanner::new(vec![]);
        let _clock =
            ScannerQuoteClockGuard::set(source + Duration::seconds(5) + Duration::nanoseconds(1));
        assert_eq!(
            scanner.validate_admitted_quote(&quote).unwrap_err(),
            ScannerQuoteUnavailable::Expired
        );
        let stats = scanner.dq_stats.snapshot();
        assert_eq!((stats.total, stats.passed, stats.rejected_stale), (1, 0, 1));
    }

    #[test]
    fn scanner_quote_future_and_inverted_native_instants_are_unavailable() {
        let now = scanner_quote_source_time();
        let cases = [
            (
                now + Duration::nanoseconds(1),
                now + Duration::nanoseconds(1),
                ScannerQuoteUnavailable::SourceFuture,
            ),
            (
                now,
                now + Duration::nanoseconds(1),
                ScannerQuoteUnavailable::ObservedFuture,
            ),
            (
                now,
                now - Duration::nanoseconds(1),
                ScannerQuoteUnavailable::TimestampInversion,
            ),
        ];
        let scanner = TieredScanner::new(vec![]);
        let _clock = ScannerQuoteClockGuard::set(now);
        for (source, observed, reason) in cases {
            let quote = scanner_quote_fixture(source, observed, 10.0, 10.0, 0.0);
            assert_eq!(scanner.validate_admitted_quote(&quote).unwrap_err(), reason);
        }
        let stats = scanner.dq_stats.snapshot();
        assert_eq!((stats.total, stats.passed, stats.rejected_stale), (3, 0, 3));
    }

    #[test]
    fn scanner_quote_nonfinite_and_nonpositive_numeric_fields_are_unavailable() {
        let now = scanner_quote_source_time();
        let cases = [
            (0.0, 10.0, 1.0, ScannerQuoteUnavailable::InvalidPrice),
            (-1.0, 10.0, 1.0, ScannerQuoteUnavailable::InvalidPrice),
            (f64::NAN, 10.0, 1.0, ScannerQuoteUnavailable::InvalidPrice),
            (
                f64::INFINITY,
                10.0,
                1.0,
                ScannerQuoteUnavailable::InvalidPrice,
            ),
            (
                10.0,
                0.0,
                1.0,
                ScannerQuoteUnavailable::InvalidPreviousClose,
            ),
            (
                10.0,
                -1.0,
                1.0,
                ScannerQuoteUnavailable::InvalidPreviousClose,
            ),
            (
                10.0,
                f64::NAN,
                1.0,
                ScannerQuoteUnavailable::InvalidPreviousClose,
            ),
            (
                10.0,
                f64::INFINITY,
                1.0,
                ScannerQuoteUnavailable::InvalidPreviousClose,
            ),
            (
                10.0,
                10.0,
                f64::NAN,
                ScannerQuoteUnavailable::InvalidChangePercent,
            ),
            (
                10.0,
                10.0,
                f64::INFINITY,
                ScannerQuoteUnavailable::InvalidChangePercent,
            ),
        ];
        let scanner = TieredScanner::new(vec![]);
        let _clock = ScannerQuoteClockGuard::set(now);
        for (price, previous, change, reason) in cases {
            let quote = scanner_quote_fixture(now, now, price, previous, change);
            assert_eq!(scanner.validate_admitted_quote(&quote).unwrap_err(), reason);
        }
        let stats = scanner.dq_stats.snapshot();
        assert_eq!(
            (stats.total, stats.passed, stats.rejected_price),
            (10, 0, 10)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scanner_quote_await_expiry_does_not_consume_or_change_transition_baseline() {
        let source = scanner_quote_source_time();
        let quote = scanner_quote_fixture(source, source, 10.0, 10.0, 0.0);
        let scanner = TieredScanner::new(vec![]);
        let _clock = ScannerQuoteClockGuard::set(source + Duration::milliseconds(4_900));
        assert!(scanner.validate_admitted_quote(&quote).is_ok());
        // An actual await separates acquisition-time validity from use.
        tokio::task::yield_now().await;
        SCANNER_QUOTE_TEST_NOW.with(|clock| {
            clock.set(Some(
                source + Duration::seconds(5) + Duration::nanoseconds(1),
            ))
        });
        let mut downstream_calls = 0;
        let mut baseline = std::collections::HashSet::from([quote.code().to_owned()]);
        let original_baseline = baseline.clone();
        let result = scanner.validate_admitted_quote(&quote);
        if let Ok(checked) = result {
            downstream_calls += 1;
            baseline.remove(checked.code());
        }
        assert_eq!(downstream_calls, 0);
        assert_eq!(baseline, original_baseline);
        let stats = scanner.dq_stats.snapshot();
        assert_eq!((stats.total, stats.passed, stats.rejected_stale), (2, 1, 1));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn scanner_quote_each_later_row_rechecks_its_consumption_clock() {
        let source = scanner_quote_source_time();
        let quote = scanner_quote_fixture(source, source, 10.0, 10.0, 0.0);
        let scanner = TieredScanner::new(vec![]);
        let _clock = ScannerQuoteClockGuard::set(source + Duration::milliseconds(4_999));
        let first = scanner.validate_admitted_quote(&quote).unwrap();
        assert_eq!(first.source_at(), source);
        tokio::task::yield_now().await;
        SCANNER_QUOTE_TEST_NOW
            .with(|clock| clock.set(Some(source + Duration::milliseconds(5_001))));
        assert_eq!(
            scanner.validate_admitted_quote(&quote).unwrap_err(),
            ScannerQuoteUnavailable::Expired
        );
        assert_eq!(first.source_at(), source);
        assert_eq!(scanner.dq_stats.snapshot().passed, 1);
    }

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
