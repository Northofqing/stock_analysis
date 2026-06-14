//! A股交易日历与时区门控。
//!
//! 功能：
//! - 判断当前是否交易日（周一至周五，排除节假日）
//! - 判断当前处于哪个交易时段（集合竞价/连续竞价/午休/收盘）
//! - 计算下一个交易日
//!
//! 节假日列表从环境变量 `TRADING_HOLIDAYS` 读取（逗号分隔的 YYYYMMDD），
//! 也可通过 `add_holidays` 运行时注入。

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use once_cell::sync::Lazy;
use std::collections::HashSet;
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

/// 添加节假日（运行时注入，用于测试或动态更新）
pub fn add_holidays(dates: &[NaiveDate]) {
    if let Ok(mut guard) = HOLIDAYS.write() {
        for d in dates {
            guard.insert(*d);
        }
    }
}

/// 判断指定日期是否为交易日
pub fn is_trading_day(date: NaiveDate) -> bool {
    // 周末
    if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    // 节假日
    if let Ok(guard) = HOLIDAYS.read() {
        if guard.contains(&date) {
            return false;
        }
    }
    true
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
        assert!(days.iter().all(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)));
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
