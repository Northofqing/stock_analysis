//! Pure recorded-only consumption of Hithink observation coverage JSON v2.
//! Neither parsing nor an upstream admission flag qualifies a live connection,
//! Gateway capture, historical calendar, publication time or trading bar.

use crate::grpc_client::external_pb::magic::market::v1 as wire;
use crate::market_domain::{
    Adjustment, AssetClass, Bar, BarInterval, Exchange, InstrumentId, Money, Price, ProviderId,
    Quantity,
};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveTime, Utc};
use prost::Message;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const REQUEST_SCHEMA: &str = "magic.market.historical_bars.request";
const COVERAGE_SCHEMA: &str = "magic.market.historical_bars.coverage";
const CONTENT_TYPE: &str = "application/json; charset=utf-8";
const SCOPE: &str = "HithinkNativeDateRangeResponseObservationOnly";
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOURCE_ROWS: usize = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RecordedHistoricalCoverageV2Error {
    #[error("recorded historical v2 input exceeds its bound")]
    InputBound,
    #[error("recorded historical v2 request wire is invalid")]
    RequestWire,
    #[error("recorded historical v2 request is invalid")]
    Request,
    #[error("recorded historical v2 response wire is invalid")]
    ResponseWire,
    #[error("recorded historical v2 envelope is invalid")]
    Envelope,
    #[error("recorded historical v2 request binding conflicts")]
    RequestBinding,
    #[error("recorded historical v2 coverage claims are unsupported or contradictory")]
    Coverage,
    #[error("recorded historical v2 native context conflicts")]
    NativeContext,
    #[error("recorded historical v2 transport receipt is invalid")]
    Receipt,
    #[error("recorded historical v2 row {index} is invalid")]
    Row { index: usize },
}

/// Diagnostic facts and original bytes only. No Deserialize or conversion to
/// any Gateway/admission/connection/Bar capability is provided.
#[derive(Debug)]
pub struct RecordedHistoricalCoverageV2Observation {
    request_wire: Vec<u8>,
    response_wire: Vec<u8>,
    coverage_json: Vec<u8>,
    request_payload_sha256: String,
    instrument: InstrumentId,
    start: NaiveDate,
    end: NaiveDate,
    limit: usize,
    native_row_dates: Vec<NaiveDate>,
    validated_source_rows_claim: usize,
    caller_limit_truncated: bool,
    observed_complete_flag: bool,
    claimed_body_sha256: String,
    claimed_body_byte_length: usize,
}

impl RecordedHistoricalCoverageV2Observation {
    pub fn request_wire(&self) -> &[u8] {
        &self.request_wire
    }
    pub fn response_wire(&self) -> &[u8] {
        &self.response_wire
    }
    pub fn coverage_json(&self) -> &[u8] {
        &self.coverage_json
    }
    pub fn request_payload_sha256(&self) -> &str {
        &self.request_payload_sha256
    }
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub fn start(&self) -> NaiveDate {
        self.start
    }
    pub fn end(&self) -> NaiveDate {
        self.end
    }
    pub fn caller_limit(&self) -> usize {
        self.limit
    }
    pub fn native_row_dates(&self) -> &[NaiveDate] {
        &self.native_row_dates
    }
    pub fn validated_source_rows_claim(&self) -> usize {
        self.validated_source_rows_claim
    }
    pub fn caller_limit_truncated(&self) -> bool {
        self.caller_limit_truncated
    }
    pub fn observed_complete_flag(&self) -> bool {
        self.observed_complete_flag
    }
    pub fn claimed_upstream_body_sha256(&self) -> &str {
        &self.claimed_body_sha256
    }
    pub fn claimed_upstream_body_byte_length(&self) -> usize {
        self.claimed_body_byte_length
    }
    pub fn upstream_body_digest_independently_verified(&self) -> bool {
        false
    }
    pub fn source_exhaustion(&self) -> &'static str {
        "Unknown"
    }
    pub fn authority_calendar_coverage(&self) -> &'static str {
        "Unknown"
    }
    pub fn missing_date_reasons(&self) -> &'static str {
        "Unknown"
    }
    pub fn source_revision(&self) -> &'static str {
        "NotProvided"
    }
    pub fn historical_publication_time(&self) -> &'static str {
        "NotProvided"
    }
    pub fn pit_guarantee(&self) -> bool {
        false
    }
    pub fn outer_provider_claim(&self) -> &'static str {
        "HithinkFinance"
    }
    pub fn record_provider_claim(&self) -> ProviderId {
        ProviderId::Tonghuashun
    }
    /// Unit from the closed upstream v2 contract; raw numeric JSON is unchanged.
    pub fn volume_unit(&self) -> &'static str {
        "LotsOf100Shares"
    }
    pub fn amount_unit(&self) -> &'static str {
        "CNY"
    }
    pub fn upstream_repository_admitted_claim(&self) -> bool {
        true
    }
    pub fn scope(&self) -> &'static str {
        "RecordedOnlyNotGrpcAcceptance"
    }
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct InstrumentWire {
    exchange: Exchange,
    code: String,
    asset_class: AssetClass,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RequestWire {
    instrument: InstrumentWire,
    interval: BarInterval,
    start: String,
    end: String,
    limit: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    request_id: String,
    request_payload_sha256: String,
    request: RequestWire,
    coverage_scope: String,
    result: Outcome,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    batch: Batch,
    coverage: Coverage,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    records: Vec<Row>,
    provenance: Provenance,
    quality: Quality,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    source: String,
    source_at: String,
    fetched_at: String,
    batch_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Quality {
    complete: bool,
    issues: Vec<String>,
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
enum Unknown {
    Unknown,
}
impl TryFrom<String> for Unknown {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "Unknown" => Ok(Self::Unknown),
            _ => Err("unsupported historical coverage state"),
        }
    }
}
#[derive(Deserialize)]
#[serde(try_from = "String")]
enum NotProvided {
    NotProvided,
}
impl TryFrom<String> for NotProvided {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "NotProvided" => Ok(Self::NotProvided),
            _ => Err("unsupported historical evidence state"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    response_validated: bool,
    validated_source_rows: usize,
    returned_rows: usize,
    caller_limit_truncated: bool,
    source_exhaustion: Unknown,
    authority_calendar_coverage: Unknown,
    missing_date_reasons: Unknown,
    source_revision: NotProvided,
    historical_publication_time: NotProvided,
    pit_guarantee: bool,
    native_response: NativeResponse,
    response_receipt: Receipt,
}
#[derive(Deserialize)]
#[serde(tag = "state", content = "value", deny_unknown_fields)]
enum NativeAdjustment {
    Absent,
    Null,
    Value(String),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeResponse {
    request_id: String,
    thscode: String,
    interval: String,
    adjust: NativeAdjustment,
    timestamp_ms: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    body_sha256: String,
    body_byte_length: usize,
    final_url: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    instrument: InstrumentWire,
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

fn date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .filter(|date| (1900..=9999).contains(&date.year()) && date.to_string() == value)
}
fn text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
fn sha(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

fn check_receipt(
    receipt: &Receipt,
    request: &RequestWire,
    start: NaiveDate,
    end: NaiveDate,
    thscode: &str,
) -> bool {
    if !digest(&receipt.body_sha256)
        || receipt.body_byte_length == 0
        || receipt.body_byte_length > MAX_RESPONSE_BYTES
        || receipt.final_url.len() > 2_048
    {
        return false;
    }
    let Ok(url) = url::Url::parse(&receipt.final_url) else {
        return false;
    };
    if url.scheme() != "https"
        || url.host_str() != Some("fuyao.aicubes.cn")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path() != "/api/a-share/prices/historical"
    {
        return false;
    }
    let offset = FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let millis = |date: NaiveDate, time: NaiveTime| {
        date.and_time(time)
            .and_local_timezone(offset)
            .single()
            .map(|instant| instant.timestamp_millis().to_string())
    };
    let Some(start_ms) = millis(start, NaiveTime::MIN) else {
        return false;
    };
    let Some(end_ms) = millis(end, NaiveTime::from_hms_milli_opt(23, 59, 59, 999).unwrap()) else {
        return false;
    };
    let pairs: Vec<_> = url.query_pairs().collect();
    let expected = [
        ("thscode", thscode),
        ("interval", "1d"),
        ("start", start_ms.as_str()),
        ("end", end_ms.as_str()),
        ("adjust", "none"),
        ("offset", "0"),
    ];
    request.interval == BarInterval::Day
        && pairs.len() == expected.len()
        && expected
            .into_iter()
            .all(|(key, value)| pairs.iter().filter(|(k, v)| k == key && v == value).count() == 1)
}

/// Parses recorded bytes only. Canonical protobuf validation rejects duplicate
/// or unknown wire fields; JSON payload bytes are hashed and retained exactly.
/// No Health, capability, descriptor-policy, provider or RPC call is made.
pub fn parse_recorded_historical_coverage_v2(
    request_bytes: &[u8],
    response_bytes: &[u8],
) -> Result<RecordedHistoricalCoverageV2Observation, RecordedHistoricalCoverageV2Error> {
    use RecordedHistoricalCoverageV2Error as Error;
    if request_bytes.len() > MAX_REQUEST_BYTES || response_bytes.len() > MAX_RESPONSE_BYTES {
        return Err(Error::InputBound);
    }
    let request = wire::QueryRequest::decode(request_bytes).map_err(|_| Error::RequestWire)?;
    if request.encode_to_vec() != request_bytes {
        return Err(Error::RequestWire);
    }
    let context = request.context.as_ref().ok_or(Error::Request)?;
    let payload = request.payload.as_ref().ok_or(Error::Request)?;
    if context.protocol_version != 1
        || !text(&context.request_id)
        || request.preferred_provider != "HithinkFinance"
        || request.allow_unadmitted
        || payload.schema != REQUEST_SCHEMA
        || payload.schema_version != 2
        || payload.content_type != CONTENT_TYPE
    {
        return Err(Error::Request);
    }
    let requested: RequestWire =
        serde_json::from_slice(&payload.data).map_err(|_| Error::Request)?;
    let instrument = InstrumentId::new(
        requested.instrument.exchange,
        &requested.instrument.code,
        requested.instrument.asset_class,
    )
    .map_err(|_| Error::Request)?;
    let start = date(&requested.start).ok_or(Error::Request)?;
    let end = date(&requested.end).ok_or(Error::Request)?;
    if instrument.asset_class() != AssetClass::Equity
        || !matches!(
            instrument.exchange(),
            Exchange::Shanghai | Exchange::Shenzhen
        )
        || instrument.code() != requested.instrument.code
        || instrument.code().len() != 6
        || !instrument.code().bytes().all(|byte| byte.is_ascii_digit())
        || requested.interval != BarInterval::Day
        || start > end
        || requested.limit == 0
        || requested.limit > u16::MAX as usize
    {
        return Err(Error::Request);
    }
    let response = wire::QueryResponse::decode(response_bytes).map_err(|_| Error::ResponseWire)?;
    if response.encode_to_vec() != response_bytes {
        return Err(Error::ResponseWire);
    }
    if response.request_id != context.request_id
        || response.operation != wire::Operation::HistoricalBars as i32
        || response.admission != wire::AdmissionState::Admitted as i32
        || response.selected_provider != "HithinkFinance"
        || !response.diagnostic_blocker.is_empty()
        || !text(&response.batch_id)
        || response.records.len() != 1
    {
        return Err(Error::Envelope);
    }
    let record = &response.records[0];
    if record.schema != COVERAGE_SCHEMA
        || record.schema_version != 2
        || record.content_type != CONTENT_TYPE
    {
        return Err(Error::Envelope);
    }
    let envelope: Envelope = serde_json::from_slice(&record.data).map_err(|_| Error::Envelope)?;
    let payload_sha = sha(&payload.data);
    if envelope.request_id != context.request_id
        || envelope.request_payload_sha256 != payload_sha
        || envelope.request != requested
    {
        return Err(Error::RequestBinding);
    }
    let batch = &envelope.result.batch;
    let coverage = &envelope.result.coverage;
    // Touch all closed fields explicitly: they are parsed observations, not
    // ignored annotations which could later acquire a stronger meaning.
    let (
        Unknown::Unknown,
        Unknown::Unknown,
        Unknown::Unknown,
        NotProvided::NotProvided,
        NotProvided::NotProvided,
    ) = (
        &coverage.source_exhaustion,
        &coverage.authority_calendar_coverage,
        &coverage.missing_date_reasons,
        &coverage.source_revision,
        &coverage.historical_publication_time,
    );
    let expected_returned = coverage.validated_source_rows.min(requested.limit);
    let truncated = coverage.validated_source_rows > expected_returned;
    let expected_issues = if truncated {
        vec![format!(
            "caller limit {} retained {} of {} validated historical rows",
            requested.limit, expected_returned, coverage.validated_source_rows
        )]
    } else {
        Vec::new()
    };
    if envelope.coverage_scope != SCOPE
        || !coverage.response_validated
        || coverage.pit_guarantee
        || coverage.validated_source_rows > MAX_SOURCE_ROWS
        || coverage.returned_rows != batch.records.len()
        || coverage.returned_rows != expected_returned
        || coverage.caller_limit_truncated != truncated
        || batch.quality.issues != expected_issues
        || batch.quality.complete != batch.quality.issues.is_empty()
        || response.complete != batch.quality.complete
    {
        return Err(Error::Coverage);
    }
    let native = &coverage.native_response;
    let thscode = format!(
        "{}.{}",
        instrument.code(),
        if instrument.exchange() == Exchange::Shanghai {
            "SH"
        } else {
            "SZ"
        }
    );
    let timestamp =
        DateTime::<Utc>::from_timestamp_millis(native.timestamp_ms).ok_or(Error::NativeContext)?;
    if native.timestamp_ms <= 0
        || !text(&native.request_id)
        || native.request_id != response.batch_id
        || native.thscode != thscode
        || native.interval != "1d"
        || !matches!(&native.adjust, NativeAdjustment::Value(value) if value == "none")
    {
        return Err(Error::NativeContext);
    }
    if !check_receipt(&coverage.response_receipt, &requested, start, end, &thscode) {
        return Err(Error::Receipt);
    }
    let provenance = &batch.provenance;
    if provenance.source != "HithinkFinance"
        || provenance.batch_id != response.batch_id
        || provenance.fetched_at != response.observed_at
        || provenance.source_at != response.source_at
        || provenance.source_at != format!("unix-ms:{}", native.timestamp_ms)
        || observed_instant(&provenance.fetched_at).is_none()
    {
        return Err(Error::Envelope);
    }
    let mut dates = Vec::with_capacity(batch.records.len());
    for (index, row) in batch.records.iter().enumerate() {
        let invalid = Error::Row { index };
        let row_instrument = InstrumentId::new(
            row.instrument.exchange,
            &row.instrument.code,
            row.instrument.asset_class,
        )
        .map_err(|_| invalid)?;
        let row_date = date(&row.bar_start).ok_or(invalid)?;
        if row_instrument != instrument
            || row.instrument.code != row_instrument.code()
            || row.interval != BarInterval::Day
            || row.bar_start != row.bar_end
            || row.bar_start != row.source_at
            || row_date < start
            || row_date > end
            || dates.last().is_some_and(|previous| *previous >= row_date)
            || row.adjustment != Adjustment::Unadjusted
            || row.provider != ProviderId::Tonghuashun
            || row.batch_id != response.batch_id
            || row.observed_at != response.observed_at
        {
            return Err(invalid);
        }
        // Reuse the existing domain's OHLC and nonnegative amount validation.
        // This temporary value never leaves the recorded-only parser.
        let _ = Bar::new(
            row_instrument,
            row.interval,
            &row.bar_start,
            &row.bar_end,
            row.open,
            row.high,
            row.low,
            row.close,
            row.volume,
            Some(row.amount),
            row.adjustment,
            row.provider,
            &row.batch_id,
        )
        .map_err(|_| invalid)?;
        dates.push(row_date);
    }
    let native_date = timestamp
        .with_timezone(&FixedOffset::east_opt(8 * 60 * 60).unwrap())
        .date_naive();
    if dates.last().is_some_and(|date| *date != native_date) {
        return Err(Error::NativeContext);
    }
    Ok(RecordedHistoricalCoverageV2Observation {
        request_wire: request_bytes.to_vec(),
        response_wire: response_bytes.to_vec(),
        coverage_json: record.data.clone(),
        request_payload_sha256: payload_sha,
        instrument,
        start,
        end,
        limit: requested.limit,
        native_row_dates: dates,
        validated_source_rows_claim: coverage.validated_source_rows,
        caller_limit_truncated: truncated,
        observed_complete_flag: response.complete,
        claimed_body_sha256: coverage.response_receipt.body_sha256.clone(),
        claimed_body_byte_length: coverage.response_receipt.body_byte_length,
    })
}

#[cfg(test)]
#[path = "historical_coverage_v2_tests.rs"]
mod tests;
