//! BarInterval / Adjustment / Bar 本地镜像 (M5, Task #76, feature 关时使用)。
//! 与上游 magic_market_core rev 75ee2a2 (provider.rs 1140-1538) 同构:
//! 字段/derive 集/校验语义/错误字符串逐字一致。
//! convert.rs (bridge 生产路径) 依赖 Bar 的 Deserialize 表示。

#[cfg(not(feature = "magic-gateway"))]
use serde::{de, Deserialize, Deserializer, Serialize};

#[cfg(not(feature = "magic-gateway"))]
use super::instrument::{CoreError, InstrumentId};
#[cfg(not(feature = "magic-gateway"))]
use super::value::{Money, Price, Quantity};
#[cfg(not(feature = "magic-gateway"))]
use super::ProviderId;

/// OHLCV bar granularity.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BarInterval {
    Minute1,
    Minute5,
    Minute15,
    Minute30,
    Hour1,
    Day,
    Week,
    Month,
    Year,
}

/// Price adjustment applied by the source to a historical bar.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Adjustment {
    Unadjusted,
    Forward,
    Backward,
}

#[cfg(not(feature = "magic-gateway"))]
fn valid_iso_date(value: &str) -> bool {
    if value.len() != 10
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = value[0..4].parse().unwrap_or(0);
    let month: u32 = value[5..7].parse().unwrap_or(0);
    let day: u32 = value[8..10].parse().unwrap_or(0);
    if !(1900..=9999).contains(&year) || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days).contains(&day)
}

#[cfg(not(feature = "magic-gateway"))]
fn valid_clock_time(value: &str) -> bool {
    if value.len() != 8
        || value.as_bytes()[2] != b':'
        || value.as_bytes()[5] != b':'
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 2 || index == 5 || byte.is_ascii_digit())
    {
        return false;
    }
    let hour: u8 = value[0..2].parse().unwrap_or(24);
    let minute: u8 = value[3..5].parse().unwrap_or(60);
    let second: u8 = value[6..8].parse().unwrap_or(60);
    hour < 24 && minute < 60 && second < 60
}

#[cfg(not(feature = "magic-gateway"))]
fn valid_bar_time(value: &str, interval: BarInterval) -> bool {
    match interval {
        BarInterval::Minute1
        | BarInterval::Minute5
        | BarInterval::Minute15
        | BarInterval::Minute30
        | BarInterval::Hour1 => {
            value.len() == 19
                && matches!(value.as_bytes()[10], b' ' | b'T')
                && valid_iso_date(&value[..10])
                && valid_clock_time(&value[11..])
        }
        BarInterval::Day | BarInterval::Week | BarInterval::Month | BarInterval::Year => {
            valid_iso_date(value)
        }
    }
}

#[cfg(not(feature = "magic-gateway"))]
fn checked_text(field: &'static str, value: impl Into<String>) -> Result<String, CoreError> {
    let value = value.into();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidValue {
            field,
            value,
            reason: "must not be empty",
        });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(CoreError::InvalidValue {
            field,
            value,
            reason: "must not contain control characters",
        });
    }
    Ok(trimmed.to_owned())
}

#[cfg(not(feature = "magic-gateway"))]
fn ensure_nonnegative_money(field: &'static str, value: Option<Money>) -> Result<(), CoreError> {
    if let Some(money) = value {
        if money.get() < 0.0 {
            return Err(CoreError::InvalidValue {
                field,
                value: money.get().to_string(),
                reason: "must be non-negative",
            });
        }
    }
    Ok(())
}

/// Provider-neutral OHLCV bar with record-level source evidence.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bar {
    instrument: InstrumentId,
    interval: BarInterval,
    bar_start: String,
    bar_end: String,
    open: Price,
    high: Price,
    low: Price,
    close: Price,
    volume: Quantity,
    amount: Option<Money>,
    adjustment: Adjustment,
    source_at: Option<String>,
    provider: ProviderId,
    batch_id: String,
}

#[cfg(not(feature = "magic-gateway"))]
impl Bar {
    /// Builds a bar and rejects inconsistent OHLC ranges.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instrument: InstrumentId,
        interval: BarInterval,
        bar_start: impl Into<String>,
        bar_end: impl Into<String>,
        open: Price,
        high: Price,
        low: Price,
        close: Price,
        volume: Quantity,
        amount: Option<Money>,
        adjustment: Adjustment,
        provider: ProviderId,
        batch_id: impl Into<String>,
    ) -> Result<Self, CoreError> {
        let bar_start = checked_text("bar_start", bar_start)?;
        let bar_end = checked_text("bar_end", bar_end)?;
        if !valid_bar_time(&bar_start, interval)
            || !valid_bar_time(&bar_end, interval)
            || bar_start.as_bytes().get(10) != bar_end.as_bytes().get(10)
            || bar_start > bar_end
        {
            return Err(CoreError::InvalidRequest("invalid bar time range".into()));
        }
        if low.get() > open.get().min(close.get())
            || high.get() < open.get().max(close.get())
            || low.get() > high.get()
        {
            return Err(CoreError::InvalidRequest("inconsistent OHLC range".into()));
        }
        ensure_nonnegative_money("bar_amount", amount)?;
        Ok(Self {
            instrument,
            interval,
            bar_start,
            bar_end,
            open,
            high,
            low,
            close,
            volume,
            amount,
            adjustment,
            source_at: None,
            provider,
            batch_id: checked_text("batch_id", batch_id)?,
        })
    }

    pub fn with_source_at(mut self, source_at: impl Into<String>) -> Result<Self, CoreError> {
        self.source_at = Some(checked_text("source_at", source_at)?);
        Ok(self)
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn interval(&self) -> BarInterval {
        self.interval
    }
    pub fn bar_start(&self) -> &str {
        &self.bar_start
    }
    pub fn bar_end(&self) -> &str {
        &self.bar_end
    }
    pub fn open(&self) -> Price {
        self.open
    }
    pub fn high(&self) -> Price {
        self.high
    }
    pub fn low(&self) -> Price {
        self.low
    }
    pub fn close(&self) -> Price {
        self.close
    }
    pub fn volume(&self) -> Quantity {
        self.volume
    }
    pub fn amount(&self) -> Option<Money> {
        self.amount
    }
    pub fn adjustment(&self) -> Adjustment {
        self.adjustment
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn batch_id(&self) -> &str {
        &self.batch_id
    }
}

#[cfg(not(feature = "magic-gateway"))]
impl<'de> Deserialize<'de> for Bar {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            instrument: InstrumentId,
            interval: BarInterval,
            bar_start: String,
            bar_end: String,
            open: Price,
            high: Price,
            low: Price,
            close: Price,
            volume: Quantity,
            amount: Option<Money>,
            adjustment: Adjustment,
            source_at: Option<String>,
            provider: ProviderId,
            batch_id: String,
        }
        let repr = Repr::deserialize(deserializer)?;
        let mut bar = Self::new(
            repr.instrument,
            repr.interval,
            repr.bar_start,
            repr.bar_end,
            repr.open,
            repr.high,
            repr.low,
            repr.close,
            repr.volume,
            repr.amount,
            repr.adjustment,
            repr.provider,
            repr.batch_id,
        )
        .map_err(de::Error::custom)?;
        if let Some(source_at) = repr.source_at {
            bar = bar.with_source_at(source_at).map_err(de::Error::custom)?;
        }
        Ok(bar)
    }
}
