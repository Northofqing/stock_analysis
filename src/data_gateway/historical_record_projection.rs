//! Bounded projections of observed HithinkFinance daily record bytes.
//! This does not admit historical coverage, publication time, or a PIT data set.

use super::external_historical_bars::{
    GatewayObservedHistoricalWindowCapture, HistoricalWindowRequest,
};
use crate::grpc_client::envelope::{CanonicalRecord, QueryAdmission, QueryResult};
use crate::market_domain::{
    Adjustment, AssetClass, BarInterval, Exchange, InstrumentId, Money, Price, ProviderId, Quantity,
};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const RECORD_SCHEMA: &str = "magic.market.bar";
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum HistoricalProjectionError {
    #[error("historical capture request binding rejected")]
    CaptureBindingRejected,
    #[error("historical query rejected; original error remains in capture")]
    QueryRejected,
    #[error("invalid historical projection request: {0}")]
    InvalidRequest(&'static str),
    #[error("invalid historical envelope: {0}")]
    InvalidEnvelope(&'static str),
    #[error("invalid historical record {index}: {reason}")]
    InvalidRecord { index: usize, reason: &'static str },
}

/// Private fields and the capture borrow keep this projection attached to its
/// original request/wire/error evidence. It is not an admitted bar capability.
#[derive(Debug)]
pub(crate) struct BoundedHistoricalRecordProjection<'a> {
    capture: &'a GatewayObservedHistoricalWindowCapture,
    parsed: ParsedHistoricalRecords,
}

impl BoundedHistoricalRecordProjection<'_> {
    pub(crate) fn capture_hash(&self) -> &str {
        self.capture.capture_hash()
    }

    pub(crate) fn request(&self) -> &HistoricalWindowRequest {
        self.capture.request()
    }

    pub(crate) fn rows(&self) -> &[ObservedHistoricalDailyRow] {
        &self.parsed.rows
    }

    /// Absence is Unknown; no gap cause, zero bar, or suspension is inferred.
    pub(crate) fn missing_observed_dates(&self) -> &[NaiveDate] {
        &self.parsed.missing_observed_dates
    }

    /// Even true proves only that this row set contains the requested dates.
    pub(crate) fn all_requested_dates_observed(&self) -> bool {
        self.parsed.missing_observed_dates.is_empty()
    }

    /// Retains the server's flag; it is not a coverage certificate.
    pub(crate) fn observed_complete_flag(&self) -> bool {
        self.parsed.observed_complete_flag
    }
}

#[derive(Debug)]
pub(crate) struct ObservedHistoricalDailyRow {
    instrument: InstrumentId,
    source_date: NaiveDate,
    open: Price,
    high: Price,
    low: Price,
    close: Price,
    volume_lots: Quantity,
    amount_cny: Money,
    response_observed_at: DateTime<Utc>,
    raw_record: CanonicalRecord,
    raw_data_sha256: String,
}

impl ObservedHistoricalDailyRow {
    pub(crate) fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub(crate) fn source_date(&self) -> NaiveDate {
        self.source_date
    }

    pub(crate) fn open(&self) -> Price {
        self.open
    }

    pub(crate) fn high(&self) -> Price {
        self.high
    }

    pub(crate) fn low(&self) -> Price {
        self.low
    }

    pub(crate) fn close(&self) -> Price {
        self.close
    }

    /// One lot is 100 shares. Fractional lots remain unchanged.
    pub(crate) fn volume_lots(&self) -> Quantity {
        self.volume_lots
    }

    /// The original CNY turnover; no volume conversion is applied to amount.
    pub(crate) fn amount_cny(&self) -> Money {
        self.amount_cny
    }

    pub(crate) fn record_provider(&self) -> ProviderId {
        ProviderId::Tonghuashun
    }

    pub(crate) fn selected_provider(&self) -> ProviderId {
        ProviderId::HithinkFinance
    }

    /// Response observation only, never a publication/correction instant.
    pub(crate) fn response_observed_at(&self) -> DateTime<Utc> {
        self.response_observed_at
    }

    pub(crate) fn raw_record(&self) -> &CanonicalRecord {
        &self.raw_record
    }

    pub(crate) fn raw_data_sha256(&self) -> &str {
        &self.raw_data_sha256
    }
}

/// Production can enter only through the existing sealed one-shot capture.
/// A rejection does not replace the capture's typed RPC result or raw status.
pub(crate) fn project_observed_historical_records(
    capture: &GatewayObservedHistoricalWindowCapture,
) -> Result<BoundedHistoricalRecordProjection<'_>, HistoricalProjectionError> {
    if capture.request_binding_error().is_some() {
        return Err(HistoricalProjectionError::CaptureBindingRejected);
    }
    let response = capture
        .observation()
        .result
        .as_ref()
        .map_err(|_| HistoricalProjectionError::QueryRejected)?;
    let request = capture.request();
    let parsed = parse_record_set(
        &ProjectionBounds {
            instrument: request.instrument(),
            from: request.from(),
            to: request.to(),
            required_trading_dates: request.required_trading_dates(),
            wire_limit: request.required_trading_dates().len(),
        },
        response,
    )?;
    Ok(BoundedHistoricalRecordProjection { capture, parsed })
}

/// Private parser seam lets tests use the receipts' actual limit 15/1 without
/// inventing a live capture for the Gateway's calendar-count request limit.
struct ProjectionBounds<'a> {
    instrument: &'a InstrumentId,
    from: NaiveDate,
    to: NaiveDate,
    required_trading_dates: &'a [NaiveDate],
    wire_limit: usize,
}

#[derive(Debug)]
struct ParsedHistoricalRecords {
    rows: Vec<ObservedHistoricalDailyRow>,
    missing_observed_dates: Vec<NaiveDate>,
    observed_complete_flag: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireInstrument {
    exchange: Exchange,
    code: String,
    asset_class: AssetClass,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DailyRecordWire {
    instrument: WireInstrument,
    interval: BarInterval,
    bar_start: String,
    bar_end: String,
    open: Price,
    high: Price,
    low: Price,
    close: Price,
    volume: Quantity,
    amount: Money,
    adjustment: Adjustment,
    source_at: String,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}

fn canonical_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .filter(|date| date.to_string() == value)
}

fn observed_instant(value: &str) -> Option<DateTime<Utc>> {
    let (seconds, fraction) = value.split_once('.')?;
    if seconds.is_empty()
        || fraction.is_empty()
        || fraction.len() > 9
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let nanos = fraction
        .parse::<u32>()
        .ok()?
        .checked_mul(10_u32.pow((9 - fraction.len()) as u32))?;
    DateTime::from_timestamp(seconds.parse().ok()?, nanos)
}

fn batch_source_date(value: &str) -> Option<NaiveDate> {
    let milliseconds = value.strip_prefix("unix-ms:")?;
    if milliseconds.is_empty() || !milliseconds.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let instant = DateTime::<Utc>::from_timestamp_millis(milliseconds.parse().ok()?)?;
    let shanghai = FixedOffset::east_opt(8 * 60 * 60)?;
    Some(instant.with_timezone(&shanghai).date_naive())
}

fn parse_record_set(
    bounds: &ProjectionBounds<'_>,
    response: &QueryResult,
) -> Result<ParsedHistoricalRecords, HistoricalProjectionError> {
    use HistoricalProjectionError::{InvalidEnvelope, InvalidRecord, InvalidRequest};
    if bounds.from > bounds.to
        || bounds.wire_limit == 0
        || bounds.instrument.asset_class() != AssetClass::Equity
        || !matches!(
            bounds.instrument.exchange(),
            Exchange::Shanghai | Exchange::Shenzhen
        )
        || bounds.instrument.code().len() != 6
        || !bounds
            .instrument
            .code()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || bounds.required_trading_dates.is_empty()
        || bounds
            .required_trading_dates
            .iter()
            .any(|date| *date < bounds.from || *date > bounds.to)
        || bounds
            .required_trading_dates
            .windows(2)
            .any(|dates| dates[0] >= dates[1])
    {
        return Err(InvalidRequest("invalid exact-date bounds"));
    }
    if response.admission != QueryAdmission::Admitted
        || response.selected_provider != "HithinkFinance"
        || !response.diagnostic_blocker.is_empty()
        || response.batch_id.trim().is_empty()
        || response.batch_id.chars().any(char::is_control)
    {
        return Err(InvalidEnvelope("invalid observed provider or batch"));
    }
    let response_observed_at = observed_instant(&response.observed_at)
        .ok_or(InvalidEnvelope("invalid response observation instant"))?;
    let latest_source_date = batch_source_date(&response.source_at)
        .ok_or(InvalidEnvelope("invalid batch source date"))?;
    if response.records.len() > bounds.wire_limit {
        return Err(InvalidEnvelope("record count exceeds issued limit"));
    }

    let mut rows = Vec::with_capacity(response.records.len());
    let mut dates = Vec::with_capacity(response.records.len());
    for (index, record) in response.records.iter().enumerate() {
        let invalid = |reason| InvalidRecord { index, reason };
        if record.schema != RECORD_SCHEMA
            || record.schema_version != 1
            || record.content_type != JSON_CONTENT_TYPE
        {
            return Err(invalid("unsupported record schema"));
        }
        let wire: DailyRecordWire = serde_json::from_slice(&record.data)
            .map_err(|_| invalid("invalid strict daily record JSON"))?;
        let instrument = InstrumentId::new(
            wire.instrument.exchange,
            &wire.instrument.code,
            wire.instrument.asset_class,
        )
        .map_err(|_| invalid("invalid record instrument"))?;
        if wire.instrument.code != instrument.code() || instrument != *bounds.instrument {
            return Err(invalid("record instrument does not match request"));
        }
        let source_date =
            canonical_date(&wire.bar_start).ok_or_else(|| invalid("invalid daily source date"))?;
        if wire.interval != BarInterval::Day
            || wire.bar_start != wire.bar_end
            || wire.bar_start != wire.source_at
            || source_date < bounds.from
            || source_date > bounds.to
            || bounds
                .required_trading_dates
                .binary_search(&source_date)
                .is_err()
        {
            return Err(invalid(
                "record date is outside the exact trading-date request",
            ));
        }
        if dates
            .last()
            .is_some_and(|previous| *previous >= source_date)
        {
            return Err(invalid("record dates are not unique ascending dates"));
        }
        if wire.low.get() > wire.open.get().min(wire.close.get())
            || wire.high.get() < wire.open.get().max(wire.close.get())
            || wire.low.get() > wire.high.get()
            || wire.amount.get() < 0.0
        {
            return Err(invalid("inconsistent OHLC or negative turnover"));
        }
        if wire.adjustment != Adjustment::Unadjusted
            || wire.provider != ProviderId::Tonghuashun
            || wire.batch_id != response.batch_id
            || wire.observed_at != response.observed_at
        {
            return Err(invalid("record adjustment or flattened evidence mismatch"));
        }
        rows.push(ObservedHistoricalDailyRow {
            instrument,
            source_date,
            open: wire.open,
            high: wire.high,
            low: wire.low,
            close: wire.close,
            volume_lots: wire.volume,
            amount_cny: wire.amount,
            response_observed_at,
            raw_record: record.clone(),
            raw_data_sha256: hex::encode(Sha256::digest(&record.data)),
        });
        dates.push(source_date);
    }
    if dates.last().is_some_and(|date| *date != latest_source_date) {
        return Err(InvalidEnvelope(
            "batch source date is not the latest observed row date",
        ));
    }
    Ok(ParsedHistoricalRecords {
        rows,
        missing_observed_dates: bounds
            .required_trading_dates
            .iter()
            .copied()
            .filter(|date| dates.binary_search(date).is_err())
            .collect(),
        observed_complete_flag: response.complete,
    })
}

#[cfg(test)]
#[path = "historical_record_projection_tests.rs"]
mod tests;
