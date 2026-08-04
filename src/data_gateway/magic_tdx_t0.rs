//! BR-164 Magic TDX Gateway evidence boundary for reverse-T observation plans.
//!
//! This module preserves provider timestamps and rejects incomplete or invalid
//! evidence. It never substitutes another data source or manufactures missing
//! price, volume, time, or order-book values.
//!
//! Business rules: BR-092, BR-151, BR-153, BR-171, BR-187.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use magic_market_core::InstrumentId;
use magic_tdx_rs::protocol::constants::{fq_type, KLINE_5MIN, KLINE_RI_K};
use magic_tdx_rs::protocol::types::{MinuteTimePrice, SecurityBar, SecurityQuote};
use magic_tdx_rs::TdxHqClient;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use super::instrument_identity::{resolve_production_equity, EquitySegment};

pub const T0_QUOTE_MAX_AGE_SECS: i64 = 5;
pub const T0_DAILY_MIN_BARS: usize = 20;
pub const T0_TODAY_MIN_COMPLETED_BARS: usize = 6;
pub const T0_HISTORY_MIN_SESSIONS: usize = 3;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct T0BookLevel {
    pub price: f64,
    pub volume: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MagicTdxT0Quote {
    pub price: f64,
    pub last_close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub amount: f64,
    pub bids: [T0BookLevel; 5],
    pub asks: [T0BookLevel; 5],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MagicTdxT0DailyBar {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MagicTdxT0FiveMinuteBar {
    pub at: NaiveDateTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MagicTdxT0Evidence {
    pub instrument: InstrumentId,
    pub code: String,
    pub requested_at: DateTime<Utc>,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub quote: MagicTdxT0Quote,
    pub settled_daily: Vec<MagicTdxT0DailyBar>,
    pub completed_five_minute: Vec<MagicTdxT0FiveMinuteBar>,
    pub intraday_average_price: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MagicTdxT0Rejection {
    pub code: String,
    pub reason_code: &'static str,
    pub detail: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MagicTdxT0Batch {
    pub requested_at: DateTime<Utc>,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub records: Vec<MagicTdxT0Evidence>,
    pub rejections: Vec<MagicTdxT0Rejection>,
}

#[derive(Clone, Debug)]
struct T0RequestIdentity {
    market: u8,
    instrument: InstrumentId,
    canonical_code: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct ValidatedT0Evidence {
    instrument: InstrumentId,
    code: String,
    source_at: DateTime<Utc>,
    quote: MagicTdxT0Quote,
    settled_daily: Vec<MagicTdxT0DailyBar>,
    completed_five_minute: Vec<MagicTdxT0FiveMinuteBar>,
    intraday_average_price: f64,
}

#[derive(Serialize)]
struct CompleteT0BatchBinding<'a> {
    schema: &'static str,
    requested_at: DateTime<Utc>,
    source_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
    requested_instruments: &'a [InstrumentId],
    records: &'a [ValidatedT0Evidence],
    rejections: &'a [MagicTdxT0Rejection],
}

fn rejection(
    code: &str,
    reason_code: &'static str,
    detail: impl Into<String>,
    retryable: bool,
) -> MagicTdxT0Rejection {
    MagicTdxT0Rejection {
        code: code.to_string(),
        reason_code,
        detail: detail.into(),
        retryable,
    }
}

fn valid_price(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn valid_nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn valid_ohlc(open: f64, high: f64, low: f64, close: f64) -> bool {
    [open, high, low, close].into_iter().all(valid_price)
        && high >= open.max(close)
        && low <= open.min(close)
        && high >= low
}

pub fn validate_quote_freshness(
    code: &str,
    source_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
) -> std::result::Result<(), MagicTdxT0Rejection> {
    let age = observed_at.signed_duration_since(source_at).num_seconds();
    if !(0..=T0_QUOTE_MAX_AGE_SECS).contains(&age) {
        return Err(rejection(
            code,
            "quote_stale",
            format!("age_secs={age} max_secs={T0_QUOTE_MAX_AGE_SECS}"),
            true,
        ));
    }
    Ok(())
}

fn complete_batch_id(binding: &CompleteT0BatchBinding<'_>) -> Result<String> {
    let canonical = serde_json::to_vec(binding)
        .map_err(|error| anyhow!("serialize complete Magic TDX T0 batch binding: {error}"))?;
    let mut hasher = Sha256::new();
    hasher.update(b"stock-analysis:magic-tdx-t0-complete-batch:v1\0");
    hasher.update(canonical);
    Ok(hex::encode(hasher.finalize()))
}

fn normalized_identity(code: &str) -> Result<T0RequestIdentity> {
    #[cfg(test)]
    let identity = if code.starts_with("TEST_CODE_") {
        super::instrument_identity::resolve_test_equity(code, None)
    } else {
        resolve_production_equity(code, None)
    };
    #[cfg(not(test))]
    let identity = resolve_production_equity(code, None);
    let identity = identity
        .and_then(|identity| {
            identity.require_a_share()?;
            Ok(identity)
        })
        .map_err(|error| anyhow!("magic-tdx T0 rejected equity identity {code:?}: {error}"))?;
    if identity.segment() == EquitySegment::BeijingA
        && !identity.canonical_code().starts_with("920")
    {
        return Err(anyhow!(
            "magic-tdx T0 has no verified capability for Beijing code {code:?}"
        ));
    }
    let market = match identity.exchange() {
        magic_market_core::Exchange::Shanghai => 1,
        magic_market_core::Exchange::Shenzhen => 0,
        magic_market_core::Exchange::Beijing => 2,
    };
    Ok(T0RequestIdentity {
        market,
        instrument: identity.instrument().clone(),
        canonical_code: identity.canonical_code().to_owned(),
    })
}

fn validate_quote_identities(
    identities: &[T0RequestIdentity],
    quotes: &[SecurityQuote],
) -> Result<()> {
    if quotes.len() != identities.len() {
        return Err(anyhow!(
            "magic-tdx T0 quote batch incomplete expected={} actual={}",
            identities.len(),
            quotes.len()
        ));
    }
    for (expected, quote) in identities.iter().zip(quotes) {
        if quote.market != expected.market || quote.code != expected.canonical_code {
            return Err(anyhow!(
                "magic-tdx T0 quote identity mismatch expected=({expected_market},{expected_code}) \
                 actual=({actual_market},{actual_code})",
                expected_market = expected.market,
                expected_code = expected.canonical_code,
                actual_market = quote.market,
                actual_code = quote.code
            ));
        }
    }
    Ok(())
}

fn source_time(
    raw: &str,
    observed_at: DateTime<Utc>,
) -> Result<DateTime<Utc>, MagicTdxT0Rejection> {
    let time = NaiveTime::parse_from_str(raw.trim(), "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(raw.trim(), "%H:%M:%S"))
        .map_err(|error| {
            rejection(
                "",
                "quote_source_time_invalid",
                format!("value={raw:?} error={error}"),
                true,
            )
        })?;
    let date = observed_at.with_timezone(&Local).date_naive();
    Local
        .from_local_datetime(&date.and_time(time))
        .single()
        .map(|value| value.with_timezone(&Utc))
        .ok_or_else(|| {
            rejection(
                "",
                "quote_source_time_invalid",
                format!("ambiguous local datetime date={date} time={time}"),
                true,
            )
        })
}

fn normalize_book(
    code: &str,
    quote: &SecurityQuote,
) -> std::result::Result<([T0BookLevel; 5], [T0BookLevel; 5]), MagicTdxT0Rejection> {
    let bids = [
        T0BookLevel {
            price: quote.bid1,
            volume: quote.bid_vol1,
        },
        T0BookLevel {
            price: quote.bid2,
            volume: quote.bid_vol2,
        },
        T0BookLevel {
            price: quote.bid3,
            volume: quote.bid_vol3,
        },
        T0BookLevel {
            price: quote.bid4,
            volume: quote.bid_vol4,
        },
        T0BookLevel {
            price: quote.bid5,
            volume: quote.bid_vol5,
        },
    ];
    let asks = [
        T0BookLevel {
            price: quote.ask1,
            volume: quote.ask_vol1,
        },
        T0BookLevel {
            price: quote.ask2,
            volume: quote.ask_vol2,
        },
        T0BookLevel {
            price: quote.ask3,
            volume: quote.ask_vol3,
        },
        T0BookLevel {
            price: quote.ask4,
            volume: quote.ask_vol4,
        },
        T0BookLevel {
            price: quote.ask5,
            volume: quote.ask_vol5,
        },
    ];
    validate_book_levels(code, &bids, &asks)?;
    Ok((bids, asks))
}

fn validate_book_levels(
    code: &str,
    bids: &[T0BookLevel; 5],
    asks: &[T0BookLevel; 5],
) -> std::result::Result<(), MagicTdxT0Rejection> {
    let all_valid = bids
        .iter()
        .chain(asks)
        .all(|level| valid_price(level.price) && valid_nonnegative(level.volume));
    if !all_valid {
        return Err(rejection(
            code,
            "order_book_invalid",
            "five-level book contains non-positive price or invalid volume",
            true,
        ));
    }
    let bid_volume = bids.iter().map(|level| level.volume).sum::<f64>();
    let ask_volume = asks.iter().map(|level| level.volume).sum::<f64>();
    if bid_volume <= 0.0 || ask_volume <= 0.0 {
        return Err(rejection(
            code,
            "order_book_empty_side",
            format!("bid_volume={bid_volume} ask_volume={ask_volume}"),
            true,
        ));
    }
    if bids[0].price > asks[0].price {
        return Err(rejection(
            code,
            "order_book_crossed",
            format!("bid1={} ask1={}", bids[0].price, asks[0].price),
            true,
        ));
    }
    Ok(())
}

fn normalize_quote(
    code: &str,
    quote: &SecurityQuote,
) -> std::result::Result<MagicTdxT0Quote, MagicTdxT0Rejection> {
    if !valid_ohlc(quote.open, quote.high, quote.low, quote.price)
        || !valid_price(quote.last_close)
        || !valid_nonnegative(quote.vol)
        || !valid_nonnegative(quote.amount)
    {
        return Err(rejection(
            code,
            "quote_invalid",
            format!(
                "price={} last_close={} open={} high={} low={} volume={} amount={}",
                quote.price,
                quote.last_close,
                quote.open,
                quote.high,
                quote.low,
                quote.vol,
                quote.amount
            ),
            true,
        ));
    }
    let (bids, asks) = normalize_book(code, quote)?;
    Ok(MagicTdxT0Quote {
        price: quote.price,
        last_close: quote.last_close,
        open: quote.open,
        high: quote.high,
        low: quote.low,
        volume: quote.vol,
        amount: quote.amount,
        bids,
        asks,
    })
}

fn daily_from_raw(
    code: &str,
    bar: SecurityBar,
) -> std::result::Result<MagicTdxT0DailyBar, MagicTdxT0Rejection> {
    let date = NaiveDate::from_ymd_opt(bar.year as i32, bar.month, bar.day).ok_or_else(|| {
        rejection(
            code,
            "daily_date_invalid",
            format!("year={} month={} day={}", bar.year, bar.month, bar.day),
            false,
        )
    })?;
    Ok(MagicTdxT0DailyBar {
        date,
        open: bar.open,
        high: bar.high,
        low: bar.low,
        close: bar.close,
        volume: bar.vol,
        amount: bar.amount,
    })
}

pub fn validate_settled_daily(
    code: &str,
    mut bars: Vec<MagicTdxT0DailyBar>,
) -> std::result::Result<Vec<MagicTdxT0DailyBar>, MagicTdxT0Rejection> {
    bars.sort_by_key(|bar| bar.date);
    if bars.len() < T0_DAILY_MIN_BARS {
        return Err(rejection(
            code,
            "daily_insufficient",
            format!("actual={} required={T0_DAILY_MIN_BARS}", bars.len()),
            true,
        ));
    }
    let mut seen = BTreeSet::new();
    for bar in &bars {
        if !seen.insert(bar.date) {
            return Err(rejection(
                code,
                "daily_duplicate",
                format!("date={}", bar.date),
                false,
            ));
        }
        if !valid_ohlc(bar.open, bar.high, bar.low, bar.close)
            || !valid_nonnegative(bar.volume)
            || !valid_nonnegative(bar.amount)
        {
            return Err(rejection(
                code,
                "daily_invalid",
                format!(
                    "date={} open={} high={} low={} close={} volume={} amount={}",
                    bar.date, bar.open, bar.high, bar.low, bar.close, bar.volume, bar.amount
                ),
                false,
            ));
        }
    }
    for pair in bars.windows(2) {
        let expected = crate::calendar::next_trading_day(pair[0].date);
        if pair[1].date != expected {
            return Err(rejection(
                code,
                "daily_gap",
                format!(
                    "previous={} expected={} actual={}",
                    pair[0].date, expected, pair[1].date
                ),
                false,
            ));
        }
        let change_pct = (pair[1].close / pair[0].close - 1.0) * 100.0;
        if change_pct.abs() > 20.0 {
            log::warn!(
                "[BR-171][BR-187] manual confirmation contract unavailable code={} \
                 dates={}→{} closes={:.6}→{:.6} change={:.4}% \
                 missing=settled_daily_provenance_batch_id,lifecycle_evidence",
                code,
                pair[0].date,
                pair[1].date,
                pair[0].close,
                pair[1].close,
                change_pct
            );
            return Err(rejection(
                code,
                "manual_confirmation_contract_unavailable",
                format!(
                    "dates={}→{} closes={:.6}→{:.6} change_pct={change_pct:.4}; \
                     pinned Magic TDX T0 settled-daily records do not expose an exact \
                     provider provenance/batch ID and this batch has no lifecycle evidence, \
                     so BR-171 confirmation lookup cannot be constructed safely",
                    pair[0].date, pair[1].date, pair[0].close, pair[1].close
                ),
                false,
            ));
        }
    }
    Ok(bars)
}

fn five_minute_from_raw(
    code: &str,
    bar: SecurityBar,
) -> std::result::Result<MagicTdxT0FiveMinuteBar, MagicTdxT0Rejection> {
    let date = NaiveDate::from_ymd_opt(bar.year as i32, bar.month, bar.day).ok_or_else(|| {
        rejection(
            code,
            "five_minute_time_invalid",
            format!("year={} month={} day={}", bar.year, bar.month, bar.day),
            false,
        )
    })?;
    let time = NaiveTime::from_hms_opt(bar.hour, bar.minute, 0).ok_or_else(|| {
        rejection(
            code,
            "five_minute_time_invalid",
            format!("hour={} minute={}", bar.hour, bar.minute),
            false,
        )
    })?;
    Ok(MagicTdxT0FiveMinuteBar {
        at: date.and_time(time),
        open: bar.open,
        high: bar.high,
        low: bar.low,
        close: bar.close,
        volume: bar.vol,
        amount: bar.amount,
    })
}

fn trading_slots() -> Vec<NaiveTime> {
    let mut slots = Vec::with_capacity(48);
    let mut hour = 9;
    let mut minute = 35;
    loop {
        let slot = NaiveTime::from_hms_opt(hour, minute, 0).expect("static trading slot");
        slots.push(slot);
        if hour == 11 && minute == 30 {
            break;
        }
        minute += 5;
        if minute == 60 {
            hour += 1;
            minute = 0;
        }
    }
    hour = 13;
    minute = 5;
    loop {
        let slot = NaiveTime::from_hms_opt(hour, minute, 0).expect("static trading slot");
        slots.push(slot);
        if hour == 15 && minute == 0 {
            break;
        }
        minute += 5;
        if minute == 60 {
            hour += 1;
            minute = 0;
        }
    }
    slots
}

fn completed_slot_cutoff(observed_at: DateTime<Utc>) -> NaiveTime {
    let local = observed_at.with_timezone(&Local);
    let minute = local.time().minute() - local.time().minute() % 5;
    NaiveTime::from_hms_opt(local.time().hour(), minute, 0).expect("valid local time")
}

pub fn validate_five_minute_bars(
    code: &str,
    mut bars: Vec<MagicTdxT0FiveMinuteBar>,
    observed_at: DateTime<Utc>,
) -> std::result::Result<Vec<MagicTdxT0FiveMinuteBar>, MagicTdxT0Rejection> {
    let today = observed_at.with_timezone(&Local).date_naive();
    let allowed_slots = trading_slots();
    let allowed = allowed_slots.iter().copied().collect::<BTreeSet<_>>();
    let cutoff = completed_slot_cutoff(observed_at);
    bars.retain(|bar| bar.at.date() < today || (bar.at.date() == today && bar.at.time() <= cutoff));
    bars.sort_by_key(|bar| bar.at);

    let mut seen = BTreeSet::new();
    for bar in &bars {
        if !seen.insert(bar.at) {
            return Err(rejection(
                code,
                "five_minute_duplicate",
                format!("at={}", bar.at),
                false,
            ));
        }
        if !allowed.contains(&bar.at.time()) {
            return Err(rejection(
                code,
                "five_minute_time_invalid",
                format!("at={}", bar.at),
                false,
            ));
        }
        if !valid_ohlc(bar.open, bar.high, bar.low, bar.close)
            || !valid_nonnegative(bar.volume)
            || !valid_nonnegative(bar.amount)
        {
            return Err(rejection(
                code,
                "five_minute_invalid",
                format!(
                    "at={} open={} high={} low={} close={} volume={} amount={}",
                    bar.at, bar.open, bar.high, bar.low, bar.close, bar.volume, bar.amount
                ),
                false,
            ));
        }
    }

    let mut by_date = BTreeMap::<NaiveDate, Vec<&MagicTdxT0FiveMinuteBar>>::new();
    for bar in &bars {
        by_date.entry(bar.at.date()).or_default().push(bar);
    }
    let today_bars = by_date.get(&today).cloned().unwrap_or_default();
    if today_bars.len() < T0_TODAY_MIN_COMPLETED_BARS {
        return Err(rejection(
            code,
            "five_minute_today_insufficient",
            format!(
                "actual={} required={T0_TODAY_MIN_COMPLETED_BARS}",
                today_bars.len()
            ),
            true,
        ));
    }
    for (index, bar) in today_bars.iter().enumerate() {
        if allowed_slots.get(index).copied() != Some(bar.at.time()) {
            return Err(rejection(
                code,
                "five_minute_gap",
                format!(
                    "date={today} index={index} expected={:?} actual={}",
                    allowed_slots.get(index),
                    bar.at.time()
                ),
                false,
            ));
        }
    }
    let comparable_sessions = by_date
        .iter()
        .filter(|(date, day_bars)| **date < today && day_bars.len() >= today_bars.len())
        .count();
    if comparable_sessions < T0_HISTORY_MIN_SESSIONS {
        return Err(rejection(
            code,
            "history_slots_insufficient",
            format!("actual={comparable_sessions} required={T0_HISTORY_MIN_SESSIONS}"),
            true,
        ));
    }
    Ok(bars)
}

fn normalize_intraday_average(
    code: &str,
    minute: Vec<MinuteTimePrice>,
) -> std::result::Result<f64, MagicTdxT0Rejection> {
    let mut values = minute
        .into_iter()
        .filter_map(|row| {
            NaiveTime::parse_from_str(row.time.trim(), "%H:%M")
                .ok()
                .map(|time| (time, row.avg_price))
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|(time, _)| *time);
    let Some((time, average)) = values.last().copied() else {
        return Err(rejection(
            code,
            "intraday_average_missing",
            "minute time data is empty or has invalid time",
            true,
        ));
    };
    if !valid_price(average) {
        return Err(rejection(
            code,
            "intraday_average_invalid",
            format!("time={time} average={average}"),
            true,
        ));
    }
    Ok(average)
}

fn evidence_for_quote(
    client: &TdxHqClient,
    identity: &T0RequestIdentity,
    quote: SecurityQuote,
    requested_at: DateTime<Utc>,
) -> std::result::Result<ValidatedT0Evidence, MagicTdxT0Rejection> {
    let code = identity.instrument.code().to_owned();
    let quote_received_at = Utc::now();
    let source_at = source_time(&quote.servertime, quote_received_at).map_err(|mut error| {
        error.code.clone_from(&code);
        error
    })?;
    validate_quote_freshness(&code, source_at, quote_received_at)?;
    let normalized_quote = normalize_quote(&code, &quote)?;
    let daily_raw = client
        .get_security_bars(KLINE_RI_K, quote.market, &code, 0, 40, fq_type::NONE)
        .map_err(|error| rejection(&code, "daily_fetch_failed", error.to_string(), true))?;
    let today = requested_at.with_timezone(&Local).date_naive();
    let daily = daily_raw
        .into_iter()
        .map(|bar| daily_from_raw(&code, bar))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|bar| bar.date < today)
        .collect::<Vec<_>>();
    let settled_daily = validate_settled_daily(&code, daily)?;

    let minute_raw = client
        .get_security_bars(KLINE_5MIN, quote.market, &code, 0, 400, fq_type::NONE)
        .map_err(|error| rejection(&code, "five_minute_fetch_failed", error.to_string(), true))?;
    let minute_bars = minute_raw
        .into_iter()
        .map(|bar| five_minute_from_raw(&code, bar))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let completed_five_minute = validate_five_minute_bars(&code, minute_bars, requested_at)?;

    let minute_time = client
        .get_minute_time_data(quote.market, &code)
        .map_err(|error| {
            rejection(
                &code,
                "intraday_average_fetch_failed",
                error.to_string(),
                true,
            )
        })?;
    let intraday_average_price = normalize_intraday_average(&code, minute_time)?;

    Ok(ValidatedT0Evidence {
        instrument: identity.instrument.clone(),
        code,
        source_at,
        quote: normalized_quote,
        settled_daily,
        completed_five_minute,
        intraday_average_price,
    })
}

fn finalize_t0_batch(
    requested_at: DateTime<Utc>,
    source_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
    identities: &[T0RequestIdentity],
    records: Vec<ValidatedT0Evidence>,
    mut rejections: Vec<MagicTdxT0Rejection>,
) -> Result<MagicTdxT0Batch> {
    if observed_at < requested_at {
        return Err(anyhow!(
            "magic-tdx T0 completion precedes request requested_at={requested_at} observed_at={observed_at}"
        ));
    }
    if source_at > observed_at {
        return Err(anyhow!(
            "magic-tdx T0 source time is in the future source_at={source_at} observed_at={observed_at}"
        ));
    }
    let mut requested_instruments = identities
        .iter()
        .map(|identity| identity.instrument.clone())
        .collect::<Vec<_>>();
    requested_instruments.sort_by(|left, right| left.code().cmp(right.code()));
    let requested_codes = requested_instruments
        .iter()
        .map(|instrument| instrument.code().to_owned())
        .collect::<BTreeSet<_>>();
    if requested_codes.len() != requested_instruments.len() {
        return Err(anyhow!(
            "magic-tdx T0 request contains duplicate instruments"
        ));
    }

    let mut fresh_records = Vec::with_capacity(records.len());
    for record in records {
        if record.code != record.instrument.code() {
            return Err(anyhow!(
                "magic-tdx T0 normalized identity mismatch code={} instrument={}",
                record.code,
                record.instrument.code()
            ));
        }
        match validate_quote_freshness(&record.code, record.source_at, observed_at) {
            Ok(()) => fresh_records.push(record),
            Err(error) => rejections.push(error),
        }
    }
    fresh_records.sort_by(|left, right| left.code.cmp(&right.code));
    rejections.sort_by(|left, right| left.code.cmp(&right.code));

    let mut outcome_codes = BTreeSet::new();
    for record in &fresh_records {
        if !requested_codes.contains(&record.code) || !outcome_codes.insert(record.code.clone()) {
            return Err(anyhow!(
                "magic-tdx T0 record is unrequested or duplicated code={}",
                record.code
            ));
        }
    }
    for rejected in &rejections {
        if rejected.code.trim().is_empty()
            || rejected.reason_code.trim().is_empty()
            || rejected.detail.trim().is_empty()
        {
            return Err(anyhow!(
                "magic-tdx T0 rejection is incomplete code={:?} reason={:?}",
                rejected.code,
                rejected.reason_code
            ));
        }
        if !requested_codes.contains(&rejected.code) || !outcome_codes.insert(rejected.code.clone())
        {
            return Err(anyhow!(
                "magic-tdx T0 rejection is unrequested or duplicated code={}",
                rejected.code
            ));
        }
    }
    if outcome_codes != requested_codes {
        return Err(anyhow!(
            "magic-tdx T0 batch outcome incomplete requested={} outcomes={}",
            requested_codes.len(),
            outcome_codes.len()
        ));
    }

    let binding = CompleteT0BatchBinding {
        schema: "magic_tdx_t0_complete_batch_v1",
        requested_at,
        source_at,
        observed_at,
        requested_instruments: &requested_instruments,
        records: &fresh_records,
        rejections: &rejections,
    };
    let batch_id = complete_batch_id(&binding)?;
    let records = fresh_records
        .into_iter()
        .map(|record| MagicTdxT0Evidence {
            instrument: record.instrument,
            code: record.code,
            requested_at,
            source_at: record.source_at,
            observed_at,
            batch_id: batch_id.clone(),
            quote: record.quote,
            settled_daily: record.settled_daily,
            completed_five_minute: record.completed_five_minute,
            intraday_average_price: record.intraday_average_price,
        })
        .collect();
    Ok(MagicTdxT0Batch {
        requested_at,
        source_at,
        observed_at,
        batch_id,
        records,
        rejections,
    })
}

pub fn fetch_magic_tdx_t0_batch(
    codes: &[String],
    requested_at: DateTime<Utc>,
) -> Result<MagicTdxT0Batch> {
    if codes.is_empty() {
        return Err(anyhow!(
            "magic-tdx T0 quote batch requires at least one code"
        ));
    }
    let identities = codes
        .iter()
        .map(|code| normalized_identity(code))
        .collect::<Result<Vec<_>>>()?;
    let client = TdxHqClient::new();
    client
        .connect_to_any(Some(5.0))
        .map_err(|error| anyhow!("magic-tdx T0 connect failed: {error}"))?;
    let request = identities
        .iter()
        .map(|identity| (identity.market, identity.canonical_code.as_str()))
        .collect::<Vec<_>>();
    let quotes = client
        .get_security_quotes(&request)
        .map_err(|error| anyhow!("magic-tdx T0 quote batch failed: {error}"))?;
    validate_quote_identities(&identities, &quotes)?;
    let mut quote_times = Vec::with_capacity(quotes.len());
    let quote_observed_at = Utc::now();
    for quote in &quotes {
        quote_times.push(
            source_time(&quote.servertime, quote_observed_at)
                .map_err(|error| anyhow!("{} {}", error.reason_code, error.detail))?,
        );
    }
    let source_at = quote_times
        .into_iter()
        .min()
        .ok_or_else(|| anyhow!("magic-tdx T0 quote batch empty"))?;
    let mut records = Vec::new();
    let mut rejections = Vec::new();
    for (identity, quote) in identities.iter().zip(quotes) {
        match evidence_for_quote(&client, identity, quote, requested_at) {
            Ok(record) => records.push(record),
            Err(error) => rejections.push(error),
        }
    }
    let observed_at = Utc::now();
    finalize_t0_batch(
        requested_at,
        source_at,
        observed_at,
        &identities,
        records,
        rejections,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(price: f64, volume: f64) -> [T0BookLevel; 5] {
        std::array::from_fn(|index| T0BookLevel {
            price: price + index as f64 * 0.01,
            volume,
        })
    }

    fn settled_daily_fixture() -> Vec<MagicTdxT0DailyBar> {
        let mut date = NaiveDate::from_ymd_opt(2026, 6, 22).expect("fixture date");
        (0..T0_DAILY_MIN_BARS)
            .map(|_| {
                let current = date;
                date = crate::calendar::next_trading_day(date);
                MagicTdxT0DailyBar {
                    date: current,
                    open: 10.0,
                    high: 10.5,
                    low: 9.5,
                    close: 10.0,
                    volume: 1_000.0,
                    amount: 10_000.0,
                }
            })
            .collect()
    }

    fn five_minute_fixture(
        observed_at: DateTime<Utc>,
        history_sessions: usize,
    ) -> Vec<MagicTdxT0FiveMinuteBar> {
        let today = observed_at.with_timezone(&Local).date_naive();
        let mut dates = Vec::new();
        let mut date = today;
        while dates.len() < history_sessions {
            date = crate::calendar::prev_trading_day(date);
            dates.push(date);
        }
        dates.reverse();
        dates.push(today);
        dates
            .into_iter()
            .flat_map(|date| {
                trading_slots()
                    .into_iter()
                    .take(T0_TODAY_MIN_COMPLETED_BARS)
                    .map(move |time| MagicTdxT0FiveMinuteBar {
                        at: date.and_time(time),
                        open: 10.0,
                        high: 10.2,
                        low: 9.8,
                        close: 10.0,
                        volume: 1_000.0,
                        amount: 10_000.0,
                    })
            })
            .collect()
    }

    fn validated_record(
        identity: &T0RequestIdentity,
        source_at: DateTime<Utc>,
        observed_at: DateTime<Utc>,
    ) -> ValidatedT0Evidence {
        ValidatedT0Evidence {
            instrument: identity.instrument.clone(),
            code: identity.instrument.code().to_owned(),
            source_at,
            quote: MagicTdxT0Quote {
                price: 10.0,
                last_close: 9.9,
                open: 9.95,
                high: 10.1,
                low: 9.9,
                volume: 10_000.0,
                amount: 100_000.0,
                bids: std::array::from_fn(|index| T0BookLevel {
                    price: 9.99 - index as f64 * 0.01,
                    volume: 1_000.0,
                }),
                asks: std::array::from_fn(|index| T0BookLevel {
                    price: 10.01 + index as f64 * 0.01,
                    volume: 1_000.0,
                }),
            },
            settled_daily: settled_daily_fixture(),
            completed_five_minute: five_minute_fixture(observed_at, T0_HISTORY_MIN_SESSIONS),
            intraday_average_price: 9.98,
        }
    }

    #[test]
    fn quote_older_than_five_seconds_is_rejected() {
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 1).unwrap();
        let source_at = Utc.with_ymd_and_hms(2026, 7, 23, 1, 59, 55).unwrap();

        let result = validate_quote_freshness("TEST_CODE_600396", source_at, observed_at);

        assert_eq!(result.unwrap_err().reason_code, "quote_stale");
    }

    #[test]
    fn complete_batch_id_binds_request_completion_and_all_normalized_evidence() {
        let requested_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let source_at = requested_at + chrono::Duration::seconds(1);
        let observed_at = requested_at + chrono::Duration::seconds(2);
        let identity = normalized_identity("TEST_CODE_600396").unwrap();
        let record = validated_record(&identity, source_at, observed_at);

        let first = finalize_t0_batch(
            requested_at,
            source_at,
            observed_at,
            std::slice::from_ref(&identity),
            vec![record.clone()],
            Vec::new(),
        )
        .unwrap();
        let second = finalize_t0_batch(
            requested_at,
            source_at,
            observed_at,
            std::slice::from_ref(&identity),
            vec![record.clone()],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(first.batch_id, second.batch_id);
        assert_eq!(first.batch_id.len(), 64);
        assert_eq!(first.records[0].batch_id, first.batch_id);
        assert_eq!(first.records[0].requested_at, requested_at);
        assert_eq!(first.records[0].observed_at, observed_at);

        let mut changed = record;
        changed.intraday_average_price = 9.97;
        let changed = finalize_t0_batch(
            requested_at,
            source_at,
            observed_at,
            &[identity],
            vec![changed],
            Vec::new(),
        )
        .unwrap();
        assert_ne!(changed.batch_id, first.batch_id);
    }

    #[test]
    fn complete_batch_id_binds_explicit_rejection_detail() {
        let requested_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let source_at = requested_at + chrono::Duration::seconds(1);
        let observed_at = requested_at + chrono::Duration::seconds(2);
        let identity = normalized_identity("TEST_CODE_600396").unwrap();
        let first_rejection = rejection(
            identity.instrument.code(),
            "daily_fetch_failed",
            "TEST_CODE provider timeout A",
            true,
        );
        let second_rejection = rejection(
            identity.instrument.code(),
            "daily_fetch_failed",
            "TEST_CODE provider timeout B",
            true,
        );

        let first = finalize_t0_batch(
            requested_at,
            source_at,
            observed_at,
            std::slice::from_ref(&identity),
            Vec::new(),
            vec![first_rejection],
        )
        .unwrap();
        let second = finalize_t0_batch(
            requested_at,
            source_at,
            observed_at,
            &[identity],
            Vec::new(),
            vec![second_rejection],
        )
        .unwrap();

        assert!(first.records.is_empty());
        assert_eq!(first.rejections.len(), 1);
        assert_ne!(first.batch_id, second.batch_id);
    }

    #[test]
    fn actual_completion_freshness_moves_stale_record_to_explicit_rejection() {
        let requested_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let source_at = requested_at + chrono::Duration::seconds(1);
        let completion_at = source_at + chrono::Duration::seconds(T0_QUOTE_MAX_AGE_SECS + 1);
        let identity = normalized_identity("TEST_CODE_600396").unwrap();
        let record = validated_record(&identity, source_at, requested_at);

        let batch = finalize_t0_batch(
            requested_at,
            source_at,
            completion_at,
            &[identity],
            vec![record],
            Vec::new(),
        )
        .unwrap();

        assert!(batch.records.is_empty());
        assert_eq!(batch.rejections.len(), 1);
        assert_eq!(batch.rejections[0].reason_code, "quote_stale");
        assert!(batch.rejections[0].detail.contains("age_secs=6"));
        assert_eq!(batch.observed_at, completion_at);
        assert_eq!(batch.batch_id.len(), 64);
    }

    #[test]
    fn br173_t0_identity_accepts_current_a_shares_and_rejects_aliases_and_b_shares() {
        let sh = normalized_identity("TEST_CODE_600396").unwrap();
        assert_eq!(sh.market, 1);
        assert_eq!(sh.canonical_code, "600396");
        assert_eq!(sh.instrument.code(), "TEST_CODE_600396");
        let sz = normalized_identity("TEST_CODE_000813").unwrap();
        assert_eq!(sz.market, 0);
        assert_eq!(sz.canonical_code, "000813");
        let bj = normalized_identity("TEST_CODE_920118").unwrap();
        assert_eq!(bj.market, 2);
        assert_eq!(bj.canonical_code, "920118");
        for code in [
            "TEST_CODE_430001",
            "TEST_CODE_830001",
            "TEST_CODE_900001",
            "TEST_CODE_200001",
            "TEST_CODE_921001",
            "TEST_CODE_929999",
        ] {
            assert!(normalized_identity(code).is_err());
        }
    }

    #[test]
    fn fewer_than_twenty_settled_daily_bars_is_rejected() {
        let result = validate_settled_daily("TEST_CODE_600396", Vec::new());

        assert_eq!(result.unwrap_err().reason_code, "daily_insufficient");
    }

    #[test]
    fn crossed_and_empty_order_book_sides_are_rejected() {
        let bids = book(10.10, 1_000.0);
        let asks = book(10.00, 1_000.0);
        assert_eq!(
            validate_book_levels("TEST_CODE_600396", &bids, &asks)
                .unwrap_err()
                .reason_code,
            "order_book_crossed"
        );

        let bids = book(9.90, 0.0);
        let asks = book(10.00, 1_000.0);
        assert_eq!(
            validate_book_levels("TEST_CODE_600396", &bids, &asks)
                .unwrap_err()
                .reason_code,
            "order_book_empty_side"
        );
    }

    #[test]
    fn duplicate_daily_bar_and_unconfirmable_change_over_twenty_percent_are_rejected() {
        let mut duplicate = settled_daily_fixture();
        duplicate[1].date = duplicate[0].date;
        assert_eq!(
            validate_settled_daily("TEST_CODE_600396", duplicate)
                .unwrap_err()
                .reason_code,
            "daily_duplicate"
        );

        let mut jump = settled_daily_fixture();
        jump[10].open = 13.0;
        jump[10].high = 13.5;
        jump[10].low = 12.5;
        jump[10].close = 13.0;
        assert_eq!(
            validate_settled_daily("TEST_CODE_600396", jump)
                .unwrap_err()
                .reason_code,
            "manual_confirmation_contract_unavailable"
        );
    }

    #[test]
    fn five_minute_duplicate_gap_and_missing_history_are_rejected() {
        let observed_at = Local
            .with_ymd_and_hms(2026, 7, 23, 10, 5, 0)
            .single()
            .expect("fixture time")
            .with_timezone(&Utc);

        let mut duplicate = five_minute_fixture(observed_at, T0_HISTORY_MIN_SESSIONS);
        duplicate.push(duplicate.last().expect("fixture bar").clone());
        assert_eq!(
            validate_five_minute_bars("TEST_CODE_600396", duplicate, observed_at)
                .unwrap_err()
                .reason_code,
            "five_minute_duplicate"
        );

        let mut gap = five_minute_fixture(observed_at, T0_HISTORY_MIN_SESSIONS);
        let today = observed_at.with_timezone(&Local).date_naive();
        gap.retain(|bar| {
            !(bar.at.date() == today
                && bar.at.time() == NaiveTime::from_hms_opt(9, 45, 0).expect("fixture slot"))
        });
        gap.push(MagicTdxT0FiveMinuteBar {
            at: today.and_hms_opt(10, 5, 0).expect("fixture gap slot"),
            open: 10.0,
            high: 10.2,
            low: 9.8,
            close: 10.0,
            volume: 1_000.0,
            amount: 10_000.0,
        });
        assert_eq!(
            validate_five_minute_bars("TEST_CODE_600396", gap, observed_at)
                .unwrap_err()
                .reason_code,
            "five_minute_gap"
        );

        let missing_history = five_minute_fixture(observed_at, T0_HISTORY_MIN_SESSIONS - 1);
        assert_eq!(
            validate_five_minute_bars("TEST_CODE_600396", missing_history, observed_at)
                .unwrap_err()
                .reason_code,
            "history_slots_insufficient"
        );
    }

    #[test]
    fn invalid_intraday_average_is_rejected_without_defaulting() {
        let minute = vec![MinuteTimePrice {
            time: "10:00".to_string(),
            price: 10.0,
            avg_price: 0.0,
            vol: 1_000.0,
        }];
        let error =
            normalize_intraday_average("TEST_CODE_600396", minute).expect_err("zero is invalid");

        assert_eq!(error.reason_code, "intraday_average_invalid");
    }
}
