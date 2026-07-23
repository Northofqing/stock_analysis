use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};

const MAX_QUOTE_AGE_SECONDS: i64 = 5;
const MAX_ADJACENT_CHANGE: f64 = 0.20;
const CONTINUITY_RELATIVE_TOLERANCE: f64 = 0.000_001;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PriceAdjustment {
    Unadjusted,
    Forward,
    Backward,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionQuote {
    pub code: String,
    pub price: f64,
    pub previous_close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub observed_at: DateTime<Local>,
    pub source_at: DateTime<Local>,
    pub volume: f64,
    pub amount: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionBar {
    pub code: String,
    /// Provider market date. No time-of-day is invented for a daily record.
    pub market_date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub settled: bool,
    pub adjustment: PriceAdjustment,
    /// Provider-supplied prior close when available. It is never synthesized.
    pub reference_previous_close: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityError {
    code: &'static str,
    message: String,
    manual_confirmation_required: bool,
}

impl QualityError {
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn manual_confirmation_required(&self) -> bool {
        self.manual_confirmation_required
    }
}

impl std::fmt::Display for QualityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for QualityError {}

#[derive(Debug, Clone, Copy)]
pub struct ValidatedQuote<'a> {
    quote: &'a SelectionQuote,
}

impl<'a> ValidatedQuote<'a> {
    pub fn quote(&self) -> &'a SelectionQuote {
        self.quote
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ValidatedDailyBars<'a> {
    bars: &'a [SelectionBar],
}

impl<'a> ValidatedDailyBars<'a> {
    pub fn bars(&self) -> &'a [SelectionBar] {
        self.bars
    }
}

pub fn validate_quote(
    quote: &SelectionQuote,
    now: DateTime<Local>,
) -> Result<ValidatedQuote<'_>, QualityError> {
    if quote.code.trim().is_empty() {
        return Err(error("security_code_empty", "quote security code is empty"));
    }
    validate_positive_price(quote.price, "quote price")?;
    validate_positive_price(quote.previous_close, "quote previous close")?;
    validate_positive_price(quote.open, "quote open")?;
    validate_positive_price(quote.high, "quote high")?;
    validate_positive_price(quote.low, "quote low")?;
    validate_flow_value(quote.volume, "volume")?;
    validate_flow_value(quote.amount, "amount")?;
    if quote.low > quote.open.min(quote.price)
        || quote.open.max(quote.price) > quote.high
        || quote.low > quote.high
    {
        return Err(error(
            "quote_ohlc_inconsistent",
            format!(
                "invalid quote OHLC: o={} h={} l={} price={}",
                quote.open, quote.high, quote.low, quote.price
            ),
        ));
    }

    if quote.source_at > quote.observed_at {
        return Err(error(
            "quote_time_inconsistent",
            "quote source time is after local observation time",
        ));
    }
    if quote.observed_at > now || quote.source_at > now {
        return Err(error(
            "quote_time_future",
            "quote source or observation time is in the future",
        ));
    }
    let age = now.signed_duration_since(quote.source_at);
    if age > chrono::Duration::seconds(MAX_QUOTE_AGE_SECONDS) {
        return Err(error(
            "quote_stale",
            format!(
                "quote age {}ms exceeds {}s",
                age.num_milliseconds(),
                MAX_QUOTE_AGE_SECONDS
            ),
        ));
    }

    Ok(ValidatedQuote { quote })
}

pub fn validate_daily(bars: &[SelectionBar]) -> Result<ValidatedDailyBars<'_>, QualityError> {
    let first = bars
        .first()
        .ok_or_else(|| error("daily_empty", "daily bar batch is empty"))?;
    let expected_code = first.code.trim();
    if expected_code.is_empty() {
        return Err(error(
            "security_code_empty",
            "daily bar security code is empty",
        ));
    }

    for bar in bars {
        if bar.code.trim() != expected_code {
            return Err(error(
                "mixed_security_batch",
                "daily bar batch contains more than one security",
            ));
        }
        validate_bar(bar)?;
    }

    for pair in bars.windows(2) {
        let previous_date = pair[0].market_date;
        let current_date = pair[1].market_date;
        if current_date == previous_date {
            return Err(error(
                "duplicate_bar",
                format!("duplicate daily bar for {current_date}"),
            ));
        }
        if current_date < previous_date {
            return Err(error(
                "bar_out_of_order",
                format!("daily bar {current_date} follows {previous_date}"),
            ));
        }
        let expected_date = crate::calendar::next_trading_day(previous_date);
        if current_date != expected_date {
            return Err(error(
                "bar_gap",
                format!(
                    "daily bar gap after {previous_date}: expected {expected_date}, got {current_date}"
                ),
            ));
        }

        if let Some(reference_previous_close) = pair[1].reference_previous_close {
            validate_positive_price(reference_previous_close, "reference previous close")?;
            let relative_difference = (reference_previous_close / pair[0].close - 1.0).abs();
            if relative_difference > CONTINUITY_RELATIVE_TOLERANCE {
                return Err(error(
                    "split_continuity_unverified",
                    format!(
                        "provider previous close {} does not match prior settled close {}",
                        reference_previous_close, pair[0].close
                    ),
                ));
            }
        }

        let change = pair[1].close / pair[0].close - 1.0;
        if change.abs() > MAX_ADJACENT_CHANGE + f64::EPSILON {
            return Err(manual_error(
                "adjacent_change_gt_20pct",
                format!(
                    "adjacent settled close change {:.6}% exceeds ±20%",
                    change * 100.0
                ),
            ));
        }
    }

    Ok(ValidatedDailyBars { bars })
}

pub fn validate_daily_freshness(
    bars: &[SelectionBar],
    expected_latest_settled_date: NaiveDate,
) -> Result<(), QualityError> {
    let validated = validate_daily(bars)?;
    let latest = validated
        .bars()
        .last()
        .expect("validated daily batch is nonempty")
        .market_date;
    if latest > expected_latest_settled_date {
        return Err(error(
            "daily_future",
            format!(
                "latest settled bar {latest} is after expected date {expected_latest_settled_date}"
            ),
        ));
    }
    let oldest_allowed = crate::calendar::prev_trading_day(expected_latest_settled_date);
    if latest < oldest_allowed {
        return Err(error(
            "daily_stale",
            format!("latest settled bar {latest} is older than allowed date {oldest_allowed}"),
        ));
    }
    Ok(())
}

fn validate_bar(bar: &SelectionBar) -> Result<(), QualityError> {
    if !crate::calendar::is_trading_day(bar.market_date) {
        return Err(error(
            "bar_non_trading_day",
            format!("daily bar date {} is not a trading day", bar.market_date),
        ));
    }
    if !bar.settled {
        return Err(error(
            "bar_not_settled",
            format!("daily bar {} is not settled", bar.market_date),
        ));
    }
    if bar.adjustment != PriceAdjustment::Unadjusted {
        return Err(error(
            "adjustment_not_unadjusted",
            format!("daily bar {} is not explicitly unadjusted", bar.market_date),
        ));
    }

    for (label, value) in [
        ("open", bar.open),
        ("high", bar.high),
        ("low", bar.low),
        ("close", bar.close),
    ] {
        validate_positive_price(value, label)?;
    }
    validate_flow_value(bar.volume, "volume")?;
    validate_flow_value(bar.amount, "amount")?;

    if bar.low > bar.open.min(bar.close) || bar.open.max(bar.close) > bar.high || bar.low > bar.high
    {
        return Err(error(
            "ohlc_inconsistent",
            format!(
                "invalid OHLC at {}: o={} h={} l={} c={}",
                bar.market_date, bar.open, bar.high, bar.low, bar.close
            ),
        ));
    }
    Ok(())
}

fn validate_positive_price(value: f64, label: &str) -> Result<(), QualityError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(error(
            "price_non_positive",
            format!("{label} must be finite and positive, got {value}"),
        ));
    }
    Ok(())
}

fn validate_flow_value(value: f64, label: &'static str) -> Result<(), QualityError> {
    if !value.is_finite() {
        return Err(error(
            match label {
                "volume" => "volume_nonfinite",
                "amount" => "amount_nonfinite",
                _ => "flow_nonfinite",
            },
            format!("{label} must be finite, got {value}"),
        ));
    }
    if value < 0.0 {
        return Err(error(
            match label {
                "volume" => "volume_negative",
                "amount" => "amount_negative",
                _ => "flow_negative",
            },
            format!("{label} must be nonnegative, got {value}"),
        ));
    }
    Ok(())
}

fn error(code: &'static str, message: impl Into<String>) -> QualityError {
    QualityError {
        code,
        message: message.into(),
        manual_confirmation_required: false,
    }
}

fn manual_error(code: &'static str, message: impl Into<String>) -> QualityError {
    QualityError {
        code,
        message: message.into(),
        manual_confirmation_required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, NaiveDate, TimeZone};

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("unambiguous local test time")
    }

    fn bar(date: NaiveDate, close: f64) -> SelectionBar {
        SelectionBar {
            code: "TEST_CODE_000001".to_string(),
            market_date: date,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000.0,
            amount: close * 1_000.0,
            settled: true,
            adjustment: PriceAdjustment::Unadjusted,
            reference_previous_close: None,
        }
    }

    fn consecutive_bars(count: usize) -> Vec<SelectionBar> {
        let mut date = NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date");
        let mut bars = Vec::with_capacity(count);
        for index in 0..count {
            bars.push(bar(date, 10.0 + index as f64 * 0.1));
            date = crate::calendar::next_trading_day(date);
        }
        bars
    }

    #[test]
    fn rejects_non_positive_price_duplicate_day_and_large_unexplained_jump() {
        let mut invalid_price = consecutive_bars(2);
        invalid_price[1].close = 0.0;
        assert_eq!(
            validate_daily(&invalid_price).unwrap_err().code(),
            "price_non_positive"
        );

        let mut duplicate = consecutive_bars(2);
        duplicate[1].market_date = duplicate[0].market_date;
        assert_eq!(
            validate_daily(&duplicate).unwrap_err().code(),
            "duplicate_bar"
        );

        let mut jump = consecutive_bars(2);
        jump[1].open = jump[0].close * 1.205;
        jump[1].high = jump[1].open;
        jump[1].low = jump[1].open;
        jump[1].close = jump[1].open;
        assert_eq!(
            validate_daily(&jump).unwrap_err().code(),
            "adjacent_change_gt_20pct"
        );
    }

    #[test]
    fn rejects_stale_intraday_quote_after_five_seconds() {
        let now = at(2026, 7, 23, 10, 0);
        let quote = SelectionQuote {
            code: "TEST_CODE_000001".to_string(),
            price: 10.0,
            previous_close: 9.8,
            open: 9.9,
            high: 10.1,
            low: 9.8,
            observed_at: now - chrono::Duration::seconds(1),
            source_at: now - chrono::Duration::seconds(6),
            volume: 1_000.0,
            amount: 10_000.0,
        };

        assert_eq!(
            validate_quote(&quote, now).unwrap_err().code(),
            "quote_stale"
        );
    }

    #[test]
    fn validates_ohlc_volume_amount_settlement_adjustment_and_continuity() {
        let mut bad_ohlc = consecutive_bars(2);
        bad_ohlc[1].high = bad_ohlc[1].close - 0.01;
        assert_eq!(
            validate_daily(&bad_ohlc).unwrap_err().code(),
            "ohlc_inconsistent"
        );

        let mut bad_amount = consecutive_bars(2);
        bad_amount[1].amount = f64::NAN;
        assert_eq!(
            validate_daily(&bad_amount).unwrap_err().code(),
            "amount_nonfinite"
        );

        let mut bad_volume = consecutive_bars(2);
        bad_volume[1].volume = f64::INFINITY;
        assert_eq!(
            validate_daily(&bad_volume).unwrap_err().code(),
            "volume_nonfinite"
        );

        let mut incomplete = consecutive_bars(2);
        incomplete[1].settled = false;
        assert_eq!(
            validate_daily(&incomplete).unwrap_err().code(),
            "bar_not_settled"
        );

        let mut adjusted = consecutive_bars(2);
        adjusted[1].adjustment = PriceAdjustment::Forward;
        assert_eq!(
            validate_daily(&adjusted).unwrap_err().code(),
            "adjustment_not_unadjusted"
        );

        let mut gap = consecutive_bars(2);
        let skipped = crate::calendar::next_trading_day(gap[0].market_date);
        let after_skipped = crate::calendar::next_trading_day(skipped);
        gap[1].market_date = after_skipped;
        assert_eq!(validate_daily(&gap).unwrap_err().code(), "bar_gap");
    }

    #[test]
    fn rejects_unverified_split_continuity_and_accepts_exact_twenty_percent_boundary() {
        let mut split_mismatch = consecutive_bars(2);
        split_mismatch[1].reference_previous_close = Some(8.0);
        assert_eq!(
            validate_daily(&split_mismatch).unwrap_err().code(),
            "split_continuity_unverified"
        );

        let mut boundary = consecutive_bars(2);
        boundary[1].open = boundary[0].close * 1.2;
        boundary[1].high = boundary[1].open;
        boundary[1].low = boundary[1].open;
        boundary[1].close = boundary[1].open;
        assert!(validate_daily(&boundary).is_ok());
    }

    #[test]
    fn rejects_daily_batch_older_than_one_trading_day() {
        let bars = consecutive_bars(2);
        let latest = bars.last().expect("latest bar").market_date;
        let one_day_later = crate::calendar::next_trading_day(latest);
        let two_days_later = crate::calendar::next_trading_day(one_day_later);

        assert!(validate_daily_freshness(&bars, one_day_later).is_ok());
        assert_eq!(
            validate_daily_freshness(&bars, two_days_later)
                .unwrap_err()
                .code(),
            "daily_stale"
        );
    }
}
