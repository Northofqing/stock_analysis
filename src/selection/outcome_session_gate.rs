//! Shared BR-178/BR-182 outcome market-session completion gate.

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveTime, TimeZone, Weekday};
use std::fmt;

const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;
const A_SHARE_CLOSE: NaiveTime =
    NaiveTime::from_hms_opt(15, 0, 0).expect("15:00:00 is a valid fixed time");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutcomeMarketSessionStatus {
    Complete,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutcomeMarketSessionGateError {
    WeekendDueDate(NaiveDate),
    UnexpectedOffsetSeconds(i32),
}

impl OutcomeMarketSessionGateError {
    pub(crate) const fn reason_code(self) -> &'static str {
        match self {
            Self::WeekendDueDate(_) => "outcome_due_date_weekend",
            Self::UnexpectedOffsetSeconds(_) => "outcome_tick_instant_not_shanghai",
        }
    }
}

impl fmt::Display for OutcomeMarketSessionGateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WeekendDueDate(due_date) => write!(
                formatter,
                "{}: {due_date} cannot be an A-share due session",
                self.reason_code()
            ),
            Self::UnexpectedOffsetSeconds(actual) => write!(
                formatter,
                "{}: tick instant offset must be +08:00, got {actual} seconds east of UTC",
                self.reason_code()
            ),
        }
    }
}

impl std::error::Error for OutcomeMarketSessionGateError {}

pub(crate) fn validate_shanghai_tick_instant(
    tick_at: &DateTime<FixedOffset>,
) -> Result<(), OutcomeMarketSessionGateError> {
    let actual = tick_at.offset().local_minus_utc();
    if actual == SHANGHAI_OFFSET_SECONDS {
        Ok(())
    } else {
        Err(OutcomeMarketSessionGateError::UnexpectedOffsetSeconds(
            actual,
        ))
    }
}

fn expected_wait_deadline(due_date: NaiveDate) -> DateTime<FixedOffset> {
    let timezone =
        FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS).expect("+08:00 is a valid fixed offset");
    let local_deadline = due_date
        .and_hms_nano_opt(15, 0, 0, 1)
        .expect("15:00:00.000000001 is valid for every NaiveDate");
    timezone
        .from_local_datetime(&local_deadline)
        .single()
        .expect("fixed offsets have exactly one local instant")
}

pub(crate) fn expected_wait_is_suppressed(
    due_date: NaiveDate,
    tick_at: DateTime<FixedOffset>,
) -> Result<bool, OutcomeMarketSessionGateError> {
    validate_shanghai_tick_instant(&tick_at)?;
    Ok(tick_at < expected_wait_deadline(due_date))
}

/// Classifies whether a stored A-share outcome due session has completed.
///
/// `due_date` is typed, so syntactically invalid calendar dates cannot enter
/// this boundary. A stored weekend due date is still rejected explicitly
/// because it cannot represent an A-share trading session. Holiday validation
/// remains the responsibility of the source-cited immutable trading calendar
/// used to create the stored schedule; this pure gate performs no provider
/// call.
pub(crate) fn outcome_market_session_status(
    due_date: NaiveDate,
    tick_at: DateTime<FixedOffset>,
) -> Result<OutcomeMarketSessionStatus, OutcomeMarketSessionGateError> {
    validate_shanghai_tick_instant(&tick_at)?;
    if matches!(due_date.weekday(), Weekday::Sat | Weekday::Sun) {
        return Err(OutcomeMarketSessionGateError::WeekendDueDate(due_date));
    }

    let local_date = tick_at.date_naive();
    let status = if due_date < local_date {
        OutcomeMarketSessionStatus::Complete
    } else if due_date > local_date || tick_at.time() <= A_SHARE_CLOSE {
        OutcomeMarketSessionStatus::Incomplete
    } else {
        OutcomeMarketSessionStatus::Complete
    };
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::{
        expected_wait_is_suppressed, outcome_market_session_status, OutcomeMarketSessionGateError,
        OutcomeMarketSessionStatus,
    };
    use chrono::{FixedOffset, NaiveDate, TimeZone, Timelike};

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("test date must be valid")
    }

    fn shanghai() -> FixedOffset {
        FixedOffset::east_opt(8 * 60 * 60).expect("+08:00 must be valid")
    }

    #[test]
    fn past_due_date_is_complete_and_future_due_date_is_incomplete() {
        let now = shanghai()
            .with_ymd_and_hms(2026, 7, 28, 9, 30, 0)
            .single()
            .expect("test local time must exist");

        assert_eq!(
            outcome_market_session_status(date(2026, 7, 27), now),
            Ok(OutcomeMarketSessionStatus::Complete)
        );
        assert_eq!(
            outcome_market_session_status(date(2026, 7, 29), now),
            Ok(OutcomeMarketSessionStatus::Incomplete)
        );
    }

    #[test]
    fn same_date_requires_time_strictly_after_fifteen_hundred() {
        let before = shanghai()
            .with_ymd_and_hms(2026, 7, 28, 14, 59, 59)
            .single()
            .expect("test local time must exist");
        let exact = shanghai()
            .with_ymd_and_hms(2026, 7, 28, 15, 0, 0)
            .single()
            .expect("test local time must exist");
        let after = exact
            .clone()
            .with_nanosecond(1)
            .expect("one nanosecond must be valid");

        assert_eq!(
            outcome_market_session_status(date(2026, 7, 28), before),
            Ok(OutcomeMarketSessionStatus::Incomplete)
        );
        assert_eq!(
            outcome_market_session_status(date(2026, 7, 28), exact),
            Ok(OutcomeMarketSessionStatus::Incomplete)
        );
        assert_eq!(
            outcome_market_session_status(date(2026, 7, 28), after),
            Ok(OutcomeMarketSessionStatus::Complete)
        );
    }

    #[test]
    fn weekend_due_date_is_an_explicit_error() {
        let now = shanghai()
            .with_ymd_and_hms(2026, 7, 27, 9, 30, 0)
            .single()
            .expect("test local time must exist");
        let weekend = date(2026, 7, 25);

        assert_eq!(
            outcome_market_session_status(weekend, now),
            Err(OutcomeMarketSessionGateError::WeekendDueDate(weekend))
        );
    }

    #[test]
    fn weekend_wall_clock_does_not_invalidate_an_earlier_trading_session() {
        let saturday = shanghai()
            .with_ymd_and_hms(2026, 7, 25, 9, 30, 0)
            .single()
            .expect("test local time must exist");

        assert_eq!(
            outcome_market_session_status(date(2026, 7, 24), saturday),
            Ok(OutcomeMarketSessionStatus::Complete)
        );
    }

    #[test]
    fn expected_wait_deadline_is_strictly_one_nanosecond_after_close() {
        let exact_close = shanghai()
            .with_ymd_and_hms(2026, 7, 28, 15, 0, 0)
            .single()
            .expect("test local time must exist");
        let one_nanosecond_after = exact_close
            .clone()
            .with_nanosecond(1)
            .expect("one nanosecond must be valid");

        assert!(
            expected_wait_is_suppressed(date(2026, 7, 28), exact_close).expect("Shanghai instant")
        );
        assert!(
            !expected_wait_is_suppressed(date(2026, 7, 28), one_nanosecond_after)
                .expect("Shanghai instant")
        );
    }

    #[test]
    fn non_shanghai_tick_instant_is_rejected() {
        let utc = FixedOffset::east_opt(0)
            .expect("UTC offset")
            .with_ymd_and_hms(2026, 7, 28, 15, 0, 0)
            .single()
            .expect("test UTC time");

        assert_eq!(
            expected_wait_is_suppressed(date(2026, 7, 28), utc),
            Err(OutcomeMarketSessionGateError::UnexpectedOffsetSeconds(0))
        );
    }
}
