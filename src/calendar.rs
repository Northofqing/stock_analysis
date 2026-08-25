//! A股交易日历与时区门控。
//!
//! 功能：
//! - 判断当前是否交易日（周一至周五，排除节假日）
//! - 判断当前处于哪个交易时段（集合竞价/连续竞价/午休/收盘）
//! - 计算下一个交易日
//!
//! 节假日列表从环境变量 `TRADING_HOLIDAYS` 读取（逗号分隔的 YYYYMMDD），
//! 也可通过 `add_holidays` 运行时注入。

use chrono::{
    DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday,
};
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};
use std::sync::RwLock;

// ============================================================================
// 交易时段常量
// ============================================================================

/// 集合竞价开始
const AUCTION_START: NaiveTime = NaiveTime::from_hms_opt(9, 15, 0).unwrap();
/// 集合竞价结束（产生开盘价）
const AUCTION_END: NaiveTime = NaiveTime::from_hms_opt(9, 25, 0).unwrap();
/// 连续竞价上午开始
const MORNING_START: NaiveTime = NaiveTime::from_hms_opt(9, 30, 0).unwrap();
/// 上午收盘
const MORNING_END: NaiveTime = NaiveTime::from_hms_opt(11, 30, 0).unwrap();
/// 下午开盘
const AFTERNOON_START: NaiveTime = NaiveTime::from_hms_opt(13, 0, 0).unwrap();
/// 下午收盘
const AFTERNOON_END: NaiveTime = NaiveTime::from_hms_opt(15, 0, 0).unwrap();

// ============================================================================
// 交易时段枚举
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketSession {
    /// 非交易时段（周末/节假日/收盘后/开盘前）
    Closed,
    /// 集合竞价 09:15-09:25
    Auction,
    /// 上午连续竞价 09:30-11:30
    Morning,
    /// 午休 11:30-13:00
    LunchBreak,
    /// 下午连续竞价 13:00-15:00
    Afternoon,
    /// 盘后（15:00 之后但在交易日）
    AfterHours,
}

impl MarketSession {
    pub fn is_trading(&self) -> bool {
        matches!(self, MarketSession::Morning | MarketSession::Afternoon)
    }

    pub fn is_auction(&self) -> bool {
        matches!(self, MarketSession::Auction)
    }

    pub fn can_trade(&self) -> bool {
        self.is_trading()
    }

    pub fn label(&self) -> &'static str {
        match self {
            MarketSession::Closed => "休市",
            MarketSession::Auction => "集合竞价",
            MarketSession::Morning => "上午盘",
            MarketSession::LunchBreak => "午休",
            MarketSession::Afternoon => "下午盘",
            MarketSession::AfterHours => "盘后",
        }
    }
}

// ============================================================================
// 交易日历
// ============================================================================

static HOLIDAYS: Lazy<RwLock<HashSet<NaiveDate>>> = Lazy::new(|| {
    let mut set = HashSet::new();
    // 仓库内经交易所公告核对的休市日是默认事实源；环境变量只用于追加临时调整。
    for line in include_str!("../config/a_share_market_holidays.csv").lines() {
        let value = line.trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            Ok(date) => {
                set.insert(date);
            }
            Err(error) => log::error!(
                "[calendar] checked-in holiday '{}' is invalid: {}",
                value,
                error
            ),
        }
    }
    // 从环境变量加载
    if let Ok(raw) = std::env::var("TRADING_HOLIDAYS") {
        for s in raw.split(',') {
            let s = s.trim();
            if s.len() == 8 {
                if let Ok(d) = NaiveDate::parse_from_str(s, "%Y%m%d") {
                    set.insert(d);
                }
            }
        }
    }
    RwLock::new(set)
});

#[derive(Debug)]
struct VerifiedTradingCalendar {
    coverage_years: Vec<i32>,
    closures: BTreeSet<NaiveDate>,
    authority_hash: String,
}

const VERIFIED_TRADING_CALENDAR_AUTHORITY_ORIGIN: &str =
    crate::data_gateway::OFFICIAL_SSE_AUTHORITY_ROOT;

static VERIFIED_TRADING_CALENDAR: Lazy<Result<VerifiedTradingCalendar, String>> =
    Lazy::new(|| parse_verified_trading_calendar(VERIFIED_TRADING_CALENDAR_RAW));

const VERIFIED_TRADING_CALENDAR_RAW: &str = include_str!("../config/a_share_market_holidays.csv");

const VERIFIED_REPLAY_CALENDAR_HASH_DOMAIN: &[u8] = b"BR251_VERIFIED_SSE_CALENDAR_V1\0";

fn update_calendar_hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn parse_verified_trading_calendar(raw: &str) -> Result<VerifiedTradingCalendar, String> {
    let mut coverage_years: Vec<i32> = Vec::new();
    let mut source = None;
    let mut closures = BTreeSet::new();
    for (line_index, line) in raw.lines().enumerate() {
        let value = line.trim();
        if value.is_empty() {
            continue;
        }
        if let Some(years) = value.strip_prefix("# year=") {
            // 多覆盖年: 逗号分隔, 如 "# year=2025,2026" (250 天 K 线窗口跨年必需)。
            if !coverage_years.is_empty() {
                return Err("duplicate checked-in trading-calendar coverage year".to_owned());
            }
            for y in years.split(',') {
                coverage_years.push(
                    y.trim().parse::<i32>().map_err(|_| {
                        "invalid checked-in trading-calendar coverage year".to_owned()
                    })?,
                );
            }
            continue;
        }
        if let Some(authority) = value.strip_prefix("# source=") {
            if source.is_some() {
                return Err("duplicate checked-in trading-calendar authority".to_owned());
            }
            if !authority.starts_with(VERIFIED_TRADING_CALENDAR_AUTHORITY_ORIGIN)
                || crate::data_gateway::validate_canonical_sse_announcement_url(authority).is_err()
            {
                return Err("checked-in trading-calendar authority is not SSE".to_owned());
            }
            source = Some(authority.to_owned());
            continue;
        }
        if value.starts_with('#') {
            continue;
        }
        let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
            format!(
                "invalid checked-in trading-calendar date at line {}",
                line_index + 1
            )
        })?;
        if !closures.insert(date) {
            return Err(format!("duplicate checked-in trading-calendar date {date}"));
        }
    }
    if coverage_years.is_empty() {
        return Err("checked-in trading-calendar coverage is missing".to_owned());
    }
    let source =
        source.ok_or_else(|| "checked-in trading-calendar authority is missing".to_owned())?;
    if source.is_empty() {
        return Err("checked-in trading-calendar authority is missing".to_owned());
    }
    if closures.is_empty()
        || closures
            .iter()
            .any(|date| !coverage_years.contains(&date.year()))
    {
        return Err("checked-in trading-calendar coverage is inconsistent".to_owned());
    }
    let mut hasher = Sha256::new();
    hasher.update(VERIFIED_REPLAY_CALENDAR_HASH_DOMAIN);
    update_calendar_hash_field(&mut hasher, raw.as_bytes());
    update_calendar_hash_field(&mut hasher, source.as_bytes());
    hasher.update((coverage_years.len() as u64).to_be_bytes());
    for year in &coverage_years {
        hasher.update(year.to_be_bytes());
    }
    hasher.update((closures.len() as u64).to_be_bytes());
    for closure in &closures {
        update_calendar_hash_field(&mut hasher, closure.to_string().as_bytes());
    }
    Ok(VerifiedTradingCalendar {
        coverage_years,
        closures,
        authority_hash: hex::encode(hasher.finalize()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifiedCalendarErrorKind {
    InvalidRequest,
    CurrentSessionIncomplete,
    TradingCalendarUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCalendarError {
    kind: VerifiedCalendarErrorKind,
    code: &'static str,
    retryable: bool,
}

impl VerifiedCalendarError {
    fn invalid(code: &'static str) -> Self {
        Self {
            kind: VerifiedCalendarErrorKind::InvalidRequest,
            code,
            retryable: false,
        }
    }

    fn current_session_incomplete() -> Self {
        Self {
            kind: VerifiedCalendarErrorKind::CurrentSessionIncomplete,
            code: "current_session_incomplete",
            retryable: true,
        }
    }

    fn unavailable(code: &'static str, retryable: bool) -> Self {
        Self {
            kind: VerifiedCalendarErrorKind::TradingCalendarUnavailable,
            code,
            retryable,
        }
    }

    pub const fn kind(&self) -> VerifiedCalendarErrorKind {
        self.kind
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

impl std::fmt::Display for VerifiedCalendarError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} ({:?}, retryable={})",
            self.code, self.kind, self.retryable
        )
    }
}

impl std::error::Error for VerifiedCalendarError {}

/// Opaque immutable calendar authority used by BR-251 replay preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedReplayCalendar {
    target_from: NaiveDate,
    target_to: NaiveDate,
    required_trading_dates: Vec<NaiveDate>,
    authority_hash: String,
}

impl VerifiedReplayCalendar {
    pub const fn target_from(&self) -> NaiveDate {
        self.target_from
    }

    pub const fn target_to(&self) -> NaiveDate {
        self.target_to
    }

    pub fn required_trading_dates(&self) -> &[NaiveDate] {
        &self.required_trading_dates
    }

    pub fn authority_hash(&self) -> &str {
        &self.authority_hash
    }
}

fn verified_calendar() -> Result<&'static VerifiedTradingCalendar, VerifiedCalendarError> {
    VERIFIED_TRADING_CALENDAR
        .as_ref()
        .map_err(|_| VerifiedCalendarError::unavailable("trading_calendar_unavailable", true))
}

/// Resolve an inclusive replay range using only checked-in SSE calendar bytes.
pub fn resolve_verified_replay_range(
    from: NaiveDate,
    to: NaiveDate,
) -> Result<VerifiedReplayCalendar, VerifiedCalendarError> {
    if from > to {
        return Err(VerifiedCalendarError::invalid(
            "invalid_trading_calendar_range",
        ));
    }
    let calendar = verified_calendar()?;
    let mut required_trading_dates = Vec::new();
    let mut cursor = from;
    loop {
        if verified_a_share_trading_day(cursor)
            .map_err(|_| VerifiedCalendarError::unavailable("trading_calendar_unavailable", true))?
        {
            required_trading_dates.push(cursor);
        }
        if cursor == to {
            break;
        }
        cursor = cursor
            .checked_add_signed(chrono::Duration::days(1))
            .ok_or_else(|| {
                VerifiedCalendarError::unavailable("trading_calendar_arithmetic_failed", false)
            })?;
    }
    if required_trading_dates.is_empty() {
        return Err(VerifiedCalendarError::unavailable(
            "trading_calendar_empty",
            false,
        ));
    }
    Ok(VerifiedReplayCalendar {
        target_from: from,
        target_to: to,
        required_trading_dates,
        authority_hash: calendar.authority_hash.clone(),
    })
}

/// Resolve one fixed natural-quarter replay range using the same immutable authority.
pub fn resolve_verified_replay_quarter(
    year: i32,
    quarter: u8,
) -> Result<VerifiedReplayCalendar, VerifiedCalendarError> {
    let (from, to) = verified_replay_quarter_bounds(year, quarter)?;
    resolve_verified_replay_range(from, to)
}

/// Return validated natural-quarter bounds without consulting mutable runtime state.
pub fn verified_replay_quarter_bounds(
    year: i32,
    quarter: u8,
) -> Result<(NaiveDate, NaiveDate), VerifiedCalendarError> {
    let (from_month, to_month, to_day) = match quarter {
        1 => (1, 3, 31),
        2 => (4, 6, 30),
        3 => (7, 9, 30),
        4 => (10, 12, 31),
        _ => return Err(VerifiedCalendarError::invalid("invalid_replay_quarter")),
    };
    let from = NaiveDate::from_ymd_opt(year, from_month, 1).ok_or_else(|| {
        VerifiedCalendarError::unavailable("trading_calendar_arithmetic_failed", false)
    })?;
    let to = NaiveDate::from_ymd_opt(year, to_month, to_day).ok_or_else(|| {
        VerifiedCalendarError::unavailable("trading_calendar_arithmetic_failed", false)
    })?;
    Ok((from, to))
}

/// Resolve the latest completed replay business date at an explicit +08:00 invocation time.
pub fn resolve_verified_scheduled_replay(
    invoked_at: DateTime<FixedOffset>,
) -> Result<VerifiedReplayCalendar, VerifiedCalendarError> {
    if invoked_at.offset().local_minus_utc() != 8 * 60 * 60 {
        return Err(VerifiedCalendarError::invalid(
            "invalid_invocation_timezone",
        ));
    }
    let date = invoked_at.date_naive();
    let is_trading_day = verified_a_share_trading_day(date)
        .map_err(|_| VerifiedCalendarError::unavailable("trading_calendar_unavailable", true))?;
    let target = if is_trading_day {
        if invoked_at.time() < AFTERNOON_END {
            return Err(VerifiedCalendarError::current_session_incomplete());
        }
        date
    } else {
        verified_prev_a_share_trading_day(date)
            .map_err(|_| VerifiedCalendarError::unavailable("trading_calendar_unavailable", true))?
    };
    resolve_verified_replay_range(target, target)
}

/// Fail-closed, immutable A-share trading-day authority for audited replay.
///
/// Unlike [`is_trading_day`], this API never reads runtime environment
/// overrides and rejects dates outside the checked-in exchange-calendar year.
pub fn verified_a_share_trading_day(date: NaiveDate) -> Result<bool, String> {
    let calendar = VERIFIED_TRADING_CALENDAR
        .as_ref()
        .map_err(std::clone::Clone::clone)?;
    if !calendar.coverage_years.contains(&date.year()) {
        return Err(format!(
            "checked-in A-share trading-calendar coverage unavailable for {}",
            date.year()
        ));
    }
    Ok(
        !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
            && !calendar.closures.contains(&date),
    )
}

/// Resolve the preceding A-share trading day using only the immutable,
/// fail-closed exchange-calendar authority.
pub fn verified_prev_a_share_trading_day(from: NaiveDate) -> Result<NaiveDate, String> {
    let mut candidate = from
        .checked_sub_signed(chrono::Duration::days(1))
        .ok_or_else(|| "A-share trading-calendar previous-date underflow".to_owned())?;
    loop {
        if verified_a_share_trading_day(candidate)? {
            return Ok(candidate);
        }
        candidate = candidate
            .checked_sub_signed(chrono::Duration::days(1))
            .ok_or_else(|| "A-share trading-calendar previous-date underflow".to_owned())?;
    }
}

/// 添加节假日（运行时注入，用于测试或动态更新）
/// review #14: poison 时 log error 而非静默丢弃, 让调用方知道 add 失败.
pub fn add_holidays(dates: &[NaiveDate]) {
    match HOLIDAYS.write() {
        Ok(mut guard) => {
            for d in dates {
                guard.insert(*d);
            }
        }
        Err(e) => log::error!("[calendar] HOLIDAYS RwLock poisoned, add 失败: {}", e),
    }
}

/// 判断指定日期是否为交易日
pub fn is_trading_day(date: NaiveDate) -> bool {
    // 周末
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    // 节假日
    // review #14 修复: RwLock poison 时 .read() 返回 Err, 原 `if let Ok(guard)` 静默
    // fall through → 节假日当交易日. 改为显式处理: poison 时按"非节假日"处理
    // (保守, 让周末检查继续生效) + log::error 提醒 operator 排查.
    match HOLIDAYS.read() {
        Ok(guard) => !guard.contains(&date),
        Err(e) => {
            log::error!(
                "[calendar] HOLIDAYS RwLock poisoned: {} — 当作非节假日处理, 请排查",
                e
            );
            true
        }
    }
}

/// 判断今天是否为交易日
pub fn today_is_trading_day() -> bool {
    is_trading_day(Local::now().date_naive())
}

/// 获取当前市场时段
pub fn current_session() -> MarketSession {
    let now = Local::now();
    let today = now.date_naive();

    if !is_trading_day(today) {
        return MarketSession::Closed;
    }

    let time = now.time();

    if time < AUCTION_START {
        MarketSession::Closed
    } else if time < AUCTION_END {
        MarketSession::Auction
    } else if time < MORNING_START {
        // 09:25-09:30: 竞价结束到开盘的间隙，视为可准备但不可交易
        MarketSession::Closed
    } else if time < MORNING_END {
        MarketSession::Morning
    } else if time < AFTERNOON_START {
        MarketSession::LunchBreak
    } else if time < AFTERNOON_END {
        MarketSession::Afternoon
    } else {
        MarketSession::AfterHours
    }
}

/// 获取当前时间所处的交易时段标签（用于日志/告警上下文）
pub fn session_label() -> &'static str {
    current_session().label()
}

/// 现在是否可以交易（连续竞价时段）
pub fn can_trade_now() -> bool {
    current_session().can_trade()
}

/// 现在是否处于集合竞价（09:15-09:25）
pub fn is_auction_now() -> bool {
    current_session().is_auction()
}

/// 现在是否在盘中（含竞价、连续竞价，用于扫描器是否活跃）
pub fn is_market_active() -> bool {
    matches!(
        current_session(),
        MarketSession::Auction
            | MarketSession::Morning
            | MarketSession::LunchBreak
            | MarketSession::Afternoon
    )
}

/// 计算下一个交易日
pub fn next_trading_day(from: NaiveDate) -> NaiveDate {
    let mut d = from + chrono::Duration::days(1);
    while !is_trading_day(d) {
        d += chrono::Duration::days(1);
    }
    d
}

/// 上一个交易日
pub fn prev_trading_day(from: NaiveDate) -> NaiveDate {
    let mut d = from - chrono::Duration::days(1);
    while !is_trading_day(d) {
        d -= chrono::Duration::days(1);
    }
    d
}

/// BR-103: Return the newest trading day whose closing facts may exist.
///
/// During a trading session the current day is incomplete, so review and NAV
/// consumers must remain on the previous trading day. At and after 15:00 the
/// current trading day becomes eligible. Weekends and holidays always resolve
/// to the preceding trading day.
pub fn latest_completed_trading_day_at(now: NaiveDateTime) -> NaiveDate {
    let date = now.date();
    if is_trading_day(date) && now.time() >= AFTERNOON_END {
        date
    } else {
        prev_trading_day(date)
    }
}

/// 获取最近 N 个交易日（包含 from）
pub fn recent_trading_days(from: NaiveDate, n: usize) -> Vec<NaiveDate> {
    let mut days = Vec::with_capacity(n);
    let mut d = from;
    while days.len() < n {
        if is_trading_day(d) {
            days.push(d);
        }
        d -= chrono::Duration::days(1);
    }
    days
}

/// 将 NaiveDateTime 转换为当前时区的可能时间，判断其所在时段。
/// 用于检查历史数据的时间戳是否在交易时段内。
pub fn session_at(datetime: NaiveDateTime) -> MarketSession {
    let date = datetime.date();
    if !is_trading_day(date) {
        return MarketSession::Closed;
    }
    let time = datetime.time();
    if time < AUCTION_START {
        MarketSession::Closed
    } else if time < AUCTION_END {
        MarketSession::Auction
    } else if time < MORNING_START {
        MarketSession::Closed
    } else if time < MORNING_END {
        MarketSession::Morning
    } else if time < AFTERNOON_START {
        MarketSession::LunchBreak
    } else if time < AFTERNOON_END {
        MarketSession::Afternoon
    } else {
        MarketSession::AfterHours
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset, TimeZone};

    fn shanghai_at(date: NaiveDate, hour: u32, minute: u32, second: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(8 * 60 * 60)
            .unwrap()
            .from_local_datetime(&date.and_hms_opt(hour, minute, second).unwrap())
            .single()
            .unwrap()
    }

    #[test]
    fn br251_verified_replay_calendar_resolves_close_weekend_and_holiday_without_fallbacks() {
        let friday = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let saturday = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap();
        let sunday = NaiveDate::from_ymd_opt(2026, 8, 23).unwrap();

        let incomplete = resolve_verified_scheduled_replay(shanghai_at(friday, 14, 59, 59))
            .expect_err("TEST_CODE trading session before close must stay incomplete");
        assert_eq!(incomplete.code(), "current_session_incomplete");
        assert!(incomplete.retryable());

        for invoked_at in [
            shanghai_at(friday, 15, 0, 0),
            shanghai_at(friday, 15, 30, 0),
            shanghai_at(saturday, 15, 30, 0),
            shanghai_at(sunday, 15, 30, 0),
        ] {
            let resolved = resolve_verified_scheduled_replay(invoked_at)
                .expect("TEST_CODE completed Friday authority");
            assert_eq!(resolved.target_from(), friday);
            assert_eq!(resolved.target_to(), friday);
            assert_eq!(resolved.required_trading_dates(), &[friday]);
            assert_eq!(resolved.authority_hash().len(), 64);
            assert!(resolved
                .authority_hash()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        }

        let friday_holiday = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        let before_holiday = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        let holiday = resolve_verified_scheduled_replay(shanghai_at(friday_holiday, 15, 30, 0))
            .expect("TEST_CODE Friday closure must walk immutable authority backwards");
        assert_eq!(holiday.target_to(), before_holiday);

        let national_day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let before_national_day = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        assert_eq!(
            resolve_verified_scheduled_replay(shanghai_at(national_day, 15, 30, 0))
                .unwrap()
                .target_to(),
            before_national_day
        );
    }

    #[test]
    fn br251_verified_replay_calendar_is_range_bounded_typed_and_runtime_immutable() {
        let from = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 8, 24).unwrap();
        let resolved =
            resolve_verified_replay_range(from, to).expect("TEST_CODE inclusive range authority");
        assert_eq!(resolved.target_from(), from);
        assert_eq!(resolved.target_to(), to);
        assert_eq!(resolved.required_trading_dates(), &[from, to]);

        add_holidays(&[to]);
        std::env::set_var("TRADING_HOLIDAYS", "20260821");
        let immutable = resolve_verified_replay_range(from, to)
            .expect("TEST_CODE verified authority ignores mutable holiday inputs");
        assert_eq!(immutable.required_trading_dates(), &[from, to]);
        std::env::remove_var("TRADING_HOLIDAYS");
        if let Ok(mut guard) = HOLIDAYS.write() {
            guard.remove(&to);
        }

        let reverse = resolve_verified_replay_range(to, from)
            .expect_err("TEST_CODE reversed range must fail before iteration");
        assert_eq!(reverse.code(), "invalid_trading_calendar_range");
        assert!(!reverse.retryable());

        let missing = resolve_verified_replay_range(
            NaiveDate::from_ymd_opt(2027, 1, 4).unwrap(),
            NaiveDate::from_ymd_opt(2027, 1, 4).unwrap(),
        )
        .expect_err("TEST_CODE uncovered year must stay unavailable");
        assert_eq!(missing.code(), "trading_calendar_unavailable");
        assert!(missing.retryable());

        let bad_offset = resolve_verified_scheduled_replay(
            FixedOffset::east_opt(0)
                .unwrap()
                .from_local_datetime(&from.and_hms_opt(15, 30, 0).unwrap())
                .single()
                .unwrap(),
        )
        .expect_err("TEST_CODE replay invocation requires explicit Shanghai offset");
        assert_eq!(bad_offset.code(), "invalid_invocation_timezone");
        assert!(!bad_offset.retryable());
    }

    #[test]
    fn br251_verified_replay_calendar_crosses_covered_year_and_fails_without_predecessor() {
        let new_year_holiday = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let resolved = resolve_verified_scheduled_replay(shanghai_at(new_year_holiday, 15, 30, 0))
            .expect("TEST_CODE predecessor exists in covered 2025 authority");
        assert_eq!(
            resolved.target_to(),
            NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()
        );

        let uncovered = resolve_verified_scheduled_replay(shanghai_at(
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            15,
            30,
            0,
        ))
        .expect_err("TEST_CODE previous completed day is outside calendar coverage");
        assert_eq!(uncovered.code(), "trading_calendar_unavailable");
        assert!(uncovered.retryable());
    }

    #[test]
    fn br251_verified_replay_quarter_uses_four_natural_quarter_boundaries() {
        for (quarter, expected_from, expected_to) in [
            ((1_u8), (1_u32, 1_u32), (3_u32, 31_u32)),
            (2, (4, 1), (6, 30)),
            (3, (7, 1), (9, 30)),
            (4, (10, 1), (12, 31)),
        ] {
            let resolved = resolve_verified_replay_quarter(2026, quarter)
                .expect("TEST_CODE natural-quarter replay authority");
            assert_eq!(
                resolved.target_from(),
                NaiveDate::from_ymd_opt(2026, expected_from.0, expected_from.1).unwrap()
            );
            assert_eq!(
                resolved.target_to(),
                NaiveDate::from_ymd_opt(2026, expected_to.0, expected_to.1).unwrap()
            );
            assert!(resolved
                .required_trading_dates()
                .windows(2)
                .all(|pair| pair[0] < pair[1]));
        }

        for quarter in [0, 5] {
            let error = resolve_verified_replay_quarter(2026, quarter)
                .expect_err("TEST_CODE invalid quarter must fail closed");
            assert_eq!(error.code(), "invalid_replay_quarter");
            assert!(!error.retryable());
        }
    }

    #[test]
    fn test_session_labels() {
        assert_eq!(MarketSession::Closed.label(), "休市");
        assert_eq!(MarketSession::Morning.label(), "上午盘");
        assert_eq!(MarketSession::Afternoon.label(), "下午盘");
    }

    #[test]
    fn test_is_trading_day_weekday() {
        // 2026-06-15 is a Monday
        let mon = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        assert!(is_trading_day(mon));
        // 2026-06-20 is a Saturday
        let sat = NaiveDate::from_ymd_opt(2026, 6, 20).unwrap();
        assert!(!is_trading_day(sat));
    }

    #[test]
    fn br194_verified_calendar_is_immutable_fail_closed_and_coverage_bounded() {
        let trading_day = NaiveDate::from_ymd_opt(2026, 7, 30).unwrap();
        let exchange_holiday = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let weekend = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        assert_eq!(verified_a_share_trading_day(trading_day), Ok(true));
        assert_eq!(verified_a_share_trading_day(exchange_holiday), Ok(false));
        assert_eq!(verified_a_share_trading_day(weekend), Ok(false));
        assert!(
            verified_a_share_trading_day(NaiveDate::from_ymd_opt(2027, 1, 4).unwrap()).is_err()
        );
        assert_eq!(
            verified_prev_a_share_trading_day(NaiveDate::from_ymd_opt(2026, 8, 18).unwrap()),
            Ok(NaiveDate::from_ymd_opt(2026, 8, 17).unwrap())
        );
        assert!(
            verified_prev_a_share_trading_day(NaiveDate::from_ymd_opt(2027, 1, 4).unwrap())
                .is_err()
        );

        add_holidays(&[trading_day]);
        assert!(
            !is_trading_day(trading_day),
            "legacy runtime calendar accepts dynamic overrides"
        );
        assert_eq!(
            verified_a_share_trading_day(trading_day),
            Ok(true),
            "audited replay authority must ignore runtime overrides"
        );
        if let Ok(mut guard) = HOLIDAYS.write() {
            guard.remove(&trading_day);
        }
    }

    #[test]
    fn br194_verified_calendar_authority_origin_is_sse() {
        assert_eq!(
            VERIFIED_TRADING_CALENDAR_AUTHORITY_ORIGIN,
            crate::data_gateway::OFFICIAL_SSE_AUTHORITY_ROOT
        );
        let spoofed = "# year=2026\n# source=https://example.com/sse-calendar\n2026-01-01\n";
        assert_eq!(
            parse_verified_trading_calendar(spoofed).unwrap_err(),
            "checked-in trading-calendar authority is not SSE"
        );
    }

    #[test]
    fn test_current_session_returns_variant() {
        let s = current_session();
        // Just verify it doesn't panic and returns a valid variant
        let _label = s.label();
    }

    #[test]
    fn test_session_at_morning() {
        let dt = NaiveDate::from_ymd_opt(2026, 6, 15)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        assert_eq!(session_at(dt), MarketSession::Morning);
    }

    #[test]
    fn test_session_at_lunch() {
        let dt = NaiveDate::from_ymd_opt(2026, 6, 15)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        assert_eq!(session_at(dt), MarketSession::LunchBreak);
    }

    #[test]
    fn test_session_at_afternoon() {
        let dt = NaiveDate::from_ymd_opt(2026, 6, 15)
            .unwrap()
            .and_hms_opt(14, 0, 0)
            .unwrap();
        assert_eq!(session_at(dt), MarketSession::Afternoon);
    }

    #[test]
    fn test_session_at_auction() {
        let dt = NaiveDate::from_ymd_opt(2026, 6, 15)
            .unwrap()
            .and_hms_opt(9, 20, 0)
            .unwrap();
        assert_eq!(session_at(dt), MarketSession::Auction);
    }

    #[test]
    fn test_session_at_weekend() {
        // Saturday
        let dt = NaiveDate::from_ymd_opt(2026, 6, 20)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        assert_eq!(session_at(dt), MarketSession::Closed);
    }

    #[test]
    fn test_holiday_exclusion() {
        // 用不与其他测试冲突的日期
        let holiday = NaiveDate::from_ymd_opt(2026, 12, 25).unwrap();
        add_holidays(&[holiday]);
        assert!(!is_trading_day(holiday));
        // 清理
        if let Ok(mut guard) = HOLIDAYS.write() {
            guard.remove(&holiday);
        }
    }

    #[test]
    fn test_next_trading_day_skips_weekend() {
        // Friday → Monday
        let fri = NaiveDate::from_ymd_opt(2026, 6, 19).unwrap();
        let next = next_trading_day(fri);
        assert_eq!(next.weekday(), Weekday::Mon);
    }

    #[test]
    fn test_next_trading_day_normal() {
        let mon = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let next = next_trading_day(mon);
        assert_eq!(next.weekday(), Weekday::Tue);
    }

    #[test]
    fn test_recent_trading_days_count() {
        let mon = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let days = recent_trading_days(mon, 5);
        assert_eq!(days.len(), 5);
        // All should be weekdays
        assert!(days
            .iter()
            .all(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)));
    }

    #[test]
    fn latest_completed_trading_day_uses_friday_during_weekend() {
        let sunday = NaiveDate::from_ymd_opt(2026, 7, 19)
            .unwrap()
            .and_hms_opt(15, 35, 0)
            .unwrap();
        assert_eq!(
            latest_completed_trading_day_at(sunday),
            NaiveDate::from_ymd_opt(2026, 7, 17).unwrap()
        );
    }

    #[test]
    fn latest_completed_trading_day_changes_at_market_close() {
        let friday = NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
        assert_eq!(
            latest_completed_trading_day_at(friday.and_hms_opt(14, 59, 59).unwrap()),
            NaiveDate::from_ymd_opt(2026, 7, 16).unwrap()
        );
        assert_eq!(
            latest_completed_trading_day_at(friday.and_hms_opt(15, 0, 0).unwrap()),
            friday
        );
    }

    #[test]
    fn test_can_trade_returns_bool() {
        // Should not panic, return true/false
        let _ = can_trade_now();
        let _ = is_auction_now();
        let _ = is_market_active();
        let _ = today_is_trading_day();
        let _ = session_label();
    }
}
