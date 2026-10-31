use crate::CoreError;
use std::fmt;

const SECONDS_PER_DAY: i64 = 86_400;
const CHINA_OFFSET_SECONDS: i32 = 8 * 60 * 60;

/// Strict second-precision wall clock used for source-session comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClockTime {
    seconds_since_midnight: u32,
}

impl ClockTime {
    pub fn parse(value: &str) -> Result<Self, CoreError> {
        let bytes = value.as_bytes();
        if bytes.len() != 8
            || bytes[2] != b':'
            || bytes[5] != b':'
            || !bytes
                .iter()
                .enumerate()
                .all(|(index, byte)| matches!(index, 2 | 5) || byte.is_ascii_digit())
        {
            return Err(CoreError::InvalidValue {
                field: "clock_time",
                value: value.to_owned(),
                reason: "must use strict HH:MM:SS",
            });
        }
        let hour = parse_two_digits(bytes[0], bytes[1]);
        let minute = parse_two_digits(bytes[3], bytes[4]);
        let second = parse_two_digits(bytes[6], bytes[7]);
        if hour > 23 || minute > 59 || second > 59 {
            return Err(CoreError::InvalidValue {
                field: "clock_time",
                value: value.to_owned(),
                reason: "contains an invalid hour, minute or second",
            });
        }
        Ok(Self {
            seconds_since_midnight: hour * 3_600 + minute * 60 + second,
        })
    }

    pub const fn seconds_since_midnight(self) -> u32 {
        self.seconds_since_midnight
    }

    pub const fn hour(self) -> u32 {
        self.seconds_since_midnight / 3_600
    }

    pub const fn minute(self) -> u32 {
        self.seconds_since_midnight % 3_600 / 60
    }

    pub const fn second(self) -> u32 {
        self.seconds_since_midnight % 60
    }
}

impl fmt::Display for ClockTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:02}:{:02}:{:02}",
            self.hour(),
            self.minute(),
            self.second()
        )
    }
}

/// Converts Unix seconds to canonical second-precision RFC3339 at a fixed
/// minute-aligned offset.
pub fn unix_seconds_to_fixed_offset_rfc3339(
    seconds: i64,
    offset_seconds: i32,
) -> Result<String, CoreError> {
    if offset_seconds % 60 != 0 || !(-86_340..=86_340).contains(&offset_seconds) {
        return Err(CoreError::InvalidValue {
            field: "utc_offset_seconds",
            value: offset_seconds.to_string(),
            reason: "must be minute-aligned and within -23:59..=+23:59",
        });
    }
    let local = seconds
        .checked_add(i64::from(offset_seconds))
        .ok_or_else(|| CoreError::InvalidRequest("fixed-offset timestamp overflow".into()))?;
    let days = local.div_euclid(SECONDS_PER_DAY);
    let day_seconds = local.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days)?;
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    let sign = if offset_seconds < 0 { '-' } else { '+' };
    let offset = offset_seconds.unsigned_abs();
    let offset_hour = offset / 3_600;
    let offset_minute = offset % 3_600 / 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{sign}{offset_hour:02}:{offset_minute:02}"
    ))
}

/// Converts Unix seconds to canonical China Standard Time (`+08:00`).
pub fn unix_seconds_to_china_rfc3339(seconds: i64) -> Result<String, CoreError> {
    unix_seconds_to_fixed_offset_rfc3339(seconds, CHINA_OFFSET_SECONDS)
}

fn civil_from_days(days_since_epoch: i64) -> Result<(i64, i64, i64), CoreError> {
    let z = days_since_epoch.checked_add(719_468).ok_or_else(|| {
        CoreError::InvalidRequest("timestamp calendar conversion overflow".into())
    })?;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
        - day_of_era / 146_096)
        .div_euclid(365);
    let mut year = year_of_era
        .checked_add(
            era.checked_mul(400)
                .ok_or_else(|| CoreError::InvalidRequest("timestamp year overflow".into()))?,
        )
        .ok_or_else(|| CoreError::InvalidRequest("timestamp year overflow".into()))?;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2).div_euclid(153);
    let day = day_of_year - (153 * month_prime + 2).div_euclid(5) + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    if !(1..=9_999).contains(&year) {
        return Err(CoreError::InvalidRequest(
            "timestamp is outside RFC3339 year range 0001..=9999".into(),
        ));
    }
    Ok((year, month, day))
}

fn parse_two_digits(tens: u8, ones: u8) -> u32 {
    u32::from(tens - b'0') * 10 + u32::from(ones - b'0')
}

#[cfg(test)]
#[path = "../tests/internal/time_tests.rs"]
mod tests;
