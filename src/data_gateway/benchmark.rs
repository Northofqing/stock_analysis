use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::review::{BatchEvidence, GatewayBatch, GatewayError};
use crate::magic_compat::ProviderId;

pub const HS300_CANONICAL: &str = "sh000300";

const BENCHMARK_CAPABILITY: &str = "BenchmarkBars";
const TDX_MARKET: u8 = 1;
const TDX_CODE: &str = "000300";
const TDX_DAILY_CATEGORY: u8 = 4;
const TDX_MINUTE1_CATEGORY: u8 = 8;
const TDX_FQ_NONE: u8 = 0;
const TDX_PAGE_SIZE: u16 = 800;
const TDX_DEPENDENCY_REVISION: &str = "75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BenchmarkGranularity {
    Daily,
    Minute1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BenchmarkRange {
    Daily {
        from: NaiveDate,
        to: NaiveDate,
    },
    Minute1 {
        from: DateTime<FixedOffset>,
        to: DateTime<FixedOffset>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkRequest {
    pub instrument: String,
    pub range: BenchmarkRange,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BenchmarkBarTime {
    Daily(NaiveDate),
    MinuteEnd(DateTime<FixedOffset>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkBar {
    pub at: BenchmarkBarTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: Option<f64>,
    pub amount: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchmarkUnsupported {
    UnsupportedInstrument,
    TestIdentityRejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchmarkError {
    Unsupported(BenchmarkUnsupported),
    Unavailable { code: &'static str, retryable: bool },
    FailedIntegrity { code: &'static str },
}

#[derive(Debug, Clone)]
pub struct BenchmarkRegistry {
    allowed_instruments: BTreeSet<String>,
    accepts_test_identities: bool,
}

impl BenchmarkRegistry {
    #[must_use]
    pub fn production_default() -> Self {
        Self {
            allowed_instruments: [HS300_CANONICAL.to_owned()].into_iter().collect(),
            accepts_test_identities: false,
        }
    }

    #[cfg(test)]
    fn test_only(instruments: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            allowed_instruments: instruments.into_iter().map(str::to_owned).collect(),
            accepts_test_identities: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BenchmarkAdmissionCoverage<'a> {
    Daily {
        authoritative_trading_days: &'a [NaiveDate],
    },
    Minute1,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedBenchmarkBatch {
    request: BenchmarkRequest,
    bars: Vec<BenchmarkBar>,
}

impl AdmittedBenchmarkBatch {
    #[must_use]
    pub fn request(&self) -> &BenchmarkRequest {
        &self.request
    }

    #[must_use]
    pub fn bars(&self) -> &[BenchmarkBar] {
        &self.bars
    }

    #[must_use]
    pub fn into_bars(self) -> Vec<BenchmarkBar> {
        self.bars
    }

    #[must_use]
    pub fn into_parts(self) -> (BenchmarkRequest, Vec<BenchmarkBar>) {
        (self.request, self.bars)
    }
}

pub fn admit_benchmark_batch(
    registry: &BenchmarkRegistry,
    request: BenchmarkRequest,
    bars: Vec<BenchmarkBar>,
    coverage: BenchmarkAdmissionCoverage<'_>,
) -> Result<AdmittedBenchmarkBatch, BenchmarkError> {
    validate_benchmark_request(registry, &request)?;

    if bars.is_empty() {
        return Err(BenchmarkError::Unavailable {
            code: "benchmark_batch_empty",
            retryable: true,
        });
    }

    for bar in &bars {
        validate_bar_values(bar)?;
        validate_bar_time(&request.range, &bar.at)?;
    }
    validate_strict_order(&bars)?;

    match (&request.range, coverage) {
        (
            BenchmarkRange::Daily { from, to },
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days,
            },
        ) => validate_daily_coverage(*from, *to, &bars, authoritative_trading_days)?,
        (BenchmarkRange::Minute1 { from, to }, BenchmarkAdmissionCoverage::Minute1) => {
            validate_minute_coverage(*from, *to, &bars)?
        }
        _ => return Err(failed_integrity("benchmark_coverage_kind_mismatch")),
    }

    Ok(AdmittedBenchmarkBatch { request, bars })
}

fn validate_benchmark_request(
    registry: &BenchmarkRegistry,
    request: &BenchmarkRequest,
) -> Result<(), BenchmarkError> {
    validate_instrument(registry, &request.instrument)?;
    validate_range(&request.range)
}

fn validate_instrument(
    registry: &BenchmarkRegistry,
    instrument: &str,
) -> Result<(), BenchmarkError> {
    if instrument.starts_with("TEST_CODE") && !registry.accepts_test_identities {
        return Err(BenchmarkError::Unsupported(
            BenchmarkUnsupported::TestIdentityRejected,
        ));
    }
    if !registry.allowed_instruments.contains(instrument) {
        return Err(BenchmarkError::Unsupported(
            BenchmarkUnsupported::UnsupportedInstrument,
        ));
    }
    Ok(())
}

fn validate_range(range: &BenchmarkRange) -> Result<(), BenchmarkError> {
    match range {
        BenchmarkRange::Daily { from, to } if from <= to => Ok(()),
        BenchmarkRange::Daily { .. } => Err(failed_integrity("benchmark_range_reversed")),
        BenchmarkRange::Minute1 { from, to } => {
            if from > to {
                return Err(failed_integrity("benchmark_range_reversed"));
            }
            if !is_shanghai_offset(from) || !is_shanghai_offset(to) {
                return Err(failed_integrity("benchmark_time_zone_invalid"));
            }
            if from.date_naive() != to.date_naive() {
                return Err(failed_integrity("benchmark_minute_range_crosses_day"));
            }
            if !is_continuous_auction_minute_end(*from) || !is_continuous_auction_minute_end(*to) {
                return Err(failed_integrity("benchmark_minute_range_off_grid"));
            }
            Ok(())
        }
    }
}

fn validate_bar_values(bar: &BenchmarkBar) -> Result<(), BenchmarkError> {
    let prices = [bar.open, bar.high, bar.low, bar.close];
    if prices
        .iter()
        .any(|price| !price.is_finite() || *price <= 0.0)
    {
        return Err(failed_integrity("benchmark_ohlc_not_positive_finite"));
    }
    if bar.low > bar.open || bar.low > bar.close || bar.open > bar.high || bar.close > bar.high {
        return Err(failed_integrity("benchmark_ohlc_inconsistent"));
    }
    for optional_value in [bar.volume, bar.amount].into_iter().flatten() {
        if !optional_value.is_finite() || optional_value < 0.0 {
            return Err(failed_integrity(
                "benchmark_turnover_not_finite_nonnegative",
            ));
        }
    }
    Ok(())
}

fn validate_bar_time(range: &BenchmarkRange, at: &BenchmarkBarTime) -> Result<(), BenchmarkError> {
    match (range, at) {
        (BenchmarkRange::Daily { from, to }, BenchmarkBarTime::Daily(date))
            if date >= from && date <= to =>
        {
            Ok(())
        }
        (BenchmarkRange::Daily { .. }, BenchmarkBarTime::Daily(_)) => {
            Err(failed_integrity("benchmark_bar_outside_range"))
        }
        (BenchmarkRange::Minute1 { from, to }, BenchmarkBarTime::MinuteEnd(at))
            if is_shanghai_offset(at)
                && is_continuous_auction_minute_end(*at)
                && at >= from
                && at <= to =>
        {
            Ok(())
        }
        (BenchmarkRange::Minute1 { .. }, BenchmarkBarTime::MinuteEnd(_)) => {
            Err(failed_integrity("benchmark_minute_bar_invalid"))
        }
        _ => Err(failed_integrity("benchmark_bar_granularity_mismatch")),
    }
}

fn validate_strict_order(bars: &[BenchmarkBar]) -> Result<(), BenchmarkError> {
    for pair in bars.windows(2) {
        let ordered = match (&pair[0].at, &pair[1].at) {
            (BenchmarkBarTime::Daily(previous), BenchmarkBarTime::Daily(current)) => {
                previous < current
            }
            (BenchmarkBarTime::MinuteEnd(previous), BenchmarkBarTime::MinuteEnd(current)) => {
                previous < current
            }
            _ => false,
        };
        if !ordered {
            return Err(failed_integrity("benchmark_bar_order_or_duplicate"));
        }
    }
    Ok(())
}

fn validate_daily_coverage(
    from: NaiveDate,
    to: NaiveDate,
    bars: &[BenchmarkBar],
    authoritative_trading_days: &[NaiveDate],
) -> Result<(), BenchmarkError> {
    if authoritative_trading_days
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
        || authoritative_trading_days
            .iter()
            .any(|date| *date < from || *date > to)
    {
        return Err(failed_integrity("benchmark_authoritative_days_invalid"));
    }
    let mut actual = Vec::with_capacity(bars.len());
    for bar in bars {
        match &bar.at {
            BenchmarkBarTime::Daily(date) => actual.push(*date),
            BenchmarkBarTime::MinuteEnd(_) => {
                return Err(failed_integrity("benchmark_bar_granularity_mismatch"));
            }
        }
    }
    if actual != authoritative_trading_days {
        return Err(failed_integrity("benchmark_daily_coverage_incomplete"));
    }
    Ok(())
}

fn validate_minute_coverage(
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
    bars: &[BenchmarkBar],
) -> Result<(), BenchmarkError> {
    let expected = continuous_auction_minute_ends(from, to);
    let mut actual = Vec::with_capacity(bars.len());
    for bar in bars {
        match &bar.at {
            BenchmarkBarTime::MinuteEnd(at) => actual.push(*at),
            BenchmarkBarTime::Daily(_) => {
                return Err(failed_integrity("benchmark_bar_granularity_mismatch"));
            }
        }
    }
    if actual != expected {
        return Err(failed_integrity("benchmark_minute_coverage_incomplete"));
    }
    Ok(())
}

fn continuous_auction_minute_ends(
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
) -> Vec<DateTime<FixedOffset>> {
    let mut expected = Vec::new();
    let mut cursor = from.naive_local();
    let end = to.naive_local();
    while cursor <= end {
        let at = from
            .offset()
            .from_local_datetime(&cursor)
            .single()
            .expect("fixed offsets have unambiguous local datetimes");
        if is_continuous_auction_minute_end(at) {
            expected.push(at);
        }
        cursor += Duration::minutes(1);
    }
    expected
}

fn is_shanghai_offset(at: &DateTime<FixedOffset>) -> bool {
    at.offset().local_minus_utc() == 8 * 60 * 60
}

fn is_continuous_auction_minute_end(at: DateTime<FixedOffset>) -> bool {
    if at.second() != 0 || at.nanosecond() != 0 {
        return false;
    }
    let minute = at.hour() * 60 + at.minute();
    let morning = 9 * 60 + 30 < minute && minute <= 11 * 60 + 30;
    let afternoon = 13 * 60 < minute && minute <= 15 * 60;
    morning || afternoon
}

fn failed_integrity(code: &'static str) -> BenchmarkError {
    BenchmarkError::FailedIntegrity { code }
}

#[derive(Debug, Clone, Copy)]
enum BenchmarkAuditOutcome {
    Unsupported,
    Unavailable,
    InvalidRequest,
    Partial,
}

impl BenchmarkAuditOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::InvalidRequest => "invalid_request",
            Self::Partial => "partial",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BenchmarkProviderAttestation {
    identity_verified: bool,
    minute_end_semantics_verified: bool,
}

impl BenchmarkProviderAttestation {
    const fn production_default() -> Self {
        Self {
            identity_verified: false,
            minute_end_semantics_verified: false,
        }
    }

    #[cfg(test)]
    const fn test_only(identity_verified: bool, minute_end_semantics_verified: bool) -> Self {
        Self {
            identity_verified,
            minute_end_semantics_verified,
        }
    }

    fn admit(self, request: &BenchmarkRequest) -> Result<(), GatewayError> {
        if !self.identity_verified {
            return Err(benchmark_gateway_error(
                BenchmarkAuditOutcome::Unavailable,
                "benchmark_identity_unverified",
                false,
                "TDX sh000300 identity attestation is unavailable",
            ));
        }
        if matches!(request.range, BenchmarkRange::Minute1 { .. })
            && !self.minute_end_semantics_verified
        {
            return Err(benchmark_gateway_error(
                BenchmarkAuditOutcome::Unavailable,
                "benchmark_time_semantics_unavailable",
                false,
                "TDX Minute1 end-label semantics attestation is unavailable",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexPageRequest {
    market: u8,
    code: &'static str,
    category: u8,
    fq_type: u8,
    offset: u32,
    count: u16,
}

#[derive(Debug, Clone)]
struct RawIndexBar {
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    datetime: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: Option<f64>,
    amount: Option<f64>,
    up_count: u32,
    down_count: u32,
}

trait IndexBarsSource {
    fn fetch_page(&self, request: IndexPageRequest) -> Result<Vec<RawIndexBar>, GatewayError>;
}

#[derive(Debug)]
pub(super) struct PreparedBenchmarkBatch {
    pub(super) batch: GatewayBatch<BenchmarkBar>,
    pub(super) request_hash: String,
}

#[derive(Debug)]
pub(super) struct BenchmarkAcquisitionOutcome {
    pub(super) request_hash: String,
    pub(super) result: Result<GatewayBatch<BenchmarkBar>, GatewayError>,
}

#[derive(Debug)]
enum OwnedBenchmarkCoverage {
    Daily(Vec<NaiveDate>),
    Minute1,
}

#[allow(dead_code)] // Task 25 consumes this private library acquisition seam.
pub(super) async fn acquire_production_benchmark_bars(
    request: BenchmarkRequest,
) -> BenchmarkAcquisitionOutcome {
    let request_hash = canonical_base_request_hash(&request);
    let registry = BenchmarkRegistry::production_default();
    if let Err(error) =
        validate_benchmark_request(&registry, &request).map_err(benchmark_admission_error)
    {
        return BenchmarkAcquisitionOutcome {
            request_hash,
            result: Err(error),
        };
    }
    if let Err(error) = BenchmarkProviderAttestation::production_default().admit(&request) {
        return BenchmarkAcquisitionOutcome {
            request_hash,
            result: Err(error),
        };
    }

    let coverage = match verified_benchmark_coverage(&request) {
        Ok(coverage) => coverage,
        Err(error) => {
            return BenchmarkAcquisitionOutcome {
                request_hash,
                result: Err(error),
            };
        }
    };

    #[cfg(not(feature = "magic-gateway"))]
    {
        let _ = coverage;
        BenchmarkAcquisitionOutcome {
            request_hash,
            result: Err(benchmark_gateway_error(
                BenchmarkAuditOutcome::Unavailable,
                "provider_transport",
                true,
                "Magic TDX library transport is disabled",
            )),
        }
    }

    #[cfg(feature = "magic-gateway")]
    {
        let joined = tokio::task::spawn_blocking(move || {
            let source = TdxIndexBarsSource::connect()?;
            let observed_at =
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            match &coverage {
                OwnedBenchmarkCoverage::Daily(authoritative_trading_days) => {
                    fetch_and_admit_benchmark_batch(
                        &source,
                        request,
                        &registry,
                        BenchmarkAdmissionCoverage::Daily {
                            authoritative_trading_days,
                        },
                        &observed_at,
                    )
                }
                OwnedBenchmarkCoverage::Minute1 => fetch_and_admit_benchmark_batch(
                    &source,
                    request,
                    &registry,
                    BenchmarkAdmissionCoverage::Minute1,
                    &observed_at,
                ),
            }
        })
        .await;
        match joined {
            Ok(Ok(prepared)) => BenchmarkAcquisitionOutcome {
                request_hash: prepared.request_hash,
                result: Ok(prepared.batch),
            },
            Ok(Err(error)) => BenchmarkAcquisitionOutcome {
                request_hash,
                result: Err(error),
            },
            Err(error) => BenchmarkAcquisitionOutcome {
                request_hash,
                result: Err(benchmark_gateway_error(
                    BenchmarkAuditOutcome::Unavailable,
                    "blocking_task_failed",
                    true,
                    format!("TDX benchmark worker failed: {error}"),
                )),
            },
        }
    }
}

fn verified_benchmark_coverage(
    request: &BenchmarkRequest,
) -> Result<OwnedBenchmarkCoverage, GatewayError> {
    match request.range {
        BenchmarkRange::Daily { from, to } => {
            let mut authoritative_trading_days = Vec::new();
            let mut cursor = from;
            loop {
                let is_trading_day = crate::calendar::verified_a_share_trading_day(cursor)
                    .map_err(|error| {
                        benchmark_gateway_error(
                            BenchmarkAuditOutcome::Unavailable,
                            "benchmark_trading_calendar_unavailable",
                            true,
                            error,
                        )
                    })?;
                if is_trading_day {
                    authoritative_trading_days.push(cursor);
                }
                if cursor == to {
                    break;
                }
                cursor = cursor
                    .checked_add_signed(Duration::days(1))
                    .ok_or_else(|| {
                        benchmark_gateway_error(
                            BenchmarkAuditOutcome::Unavailable,
                            "benchmark_trading_calendar_unavailable",
                            false,
                            "benchmark calendar range overflow",
                        )
                    })?;
            }
            Ok(OwnedBenchmarkCoverage::Daily(authoritative_trading_days))
        }
        BenchmarkRange::Minute1 { .. } => Ok(OwnedBenchmarkCoverage::Minute1),
    }
}

fn canonical_base_request_hash(request: &BenchmarkRequest) -> String {
    let category = match request.range {
        BenchmarkRange::Daily { .. } => TDX_DAILY_CATEGORY,
        BenchmarkRange::Minute1 { .. } => TDX_MINUTE1_CATEGORY,
    };
    let mut hasher = Sha256::new();
    hasher.update(b"BR251_TDX_INDEX_REQUEST_BASE_V1\0");
    update_length_prefixed(&mut hasher, request.instrument.as_bytes());
    match &request.range {
        BenchmarkRange::Daily { from, to } => {
            update_length_prefixed(&mut hasher, b"Daily");
            update_length_prefixed(&mut hasher, from.to_string().as_bytes());
            update_length_prefixed(&mut hasher, to.to_string().as_bytes());
        }
        BenchmarkRange::Minute1 { from, to } => {
            update_length_prefixed(&mut hasher, b"Minute1");
            update_length_prefixed(&mut hasher, from.to_rfc3339().as_bytes());
            update_length_prefixed(&mut hasher, to.to_rfc3339().as_bytes());
        }
    }
    hasher.update([TDX_MARKET, category, TDX_FQ_NONE]);
    update_length_prefixed(&mut hasher, TDX_CODE.as_bytes());
    update_length_prefixed(&mut hasher, TDX_DEPENDENCY_REVISION.as_bytes());
    hex::encode(hasher.finalize())
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

#[cfg(feature = "magic-gateway")]
struct TdxIndexBarsSource {
    client: magic_tdx_rs::TdxHqClient,
}

#[cfg(feature = "magic-gateway")]
impl TdxIndexBarsSource {
    fn connect() -> Result<Self, GatewayError> {
        let client = magic_tdx_rs::TdxHqClient::new();
        let connected = client.connect_to_any(None).map_err(|error| {
            benchmark_gateway_error(
                BenchmarkAuditOutcome::Unavailable,
                "provider_transport",
                true,
                format!("Magic TDX connection failed: {error}"),
            )
        })?;
        if !connected {
            return Err(benchmark_gateway_error(
                BenchmarkAuditOutcome::Unavailable,
                "provider_transport",
                true,
                "Magic TDX did not establish a connection",
            ));
        }
        Ok(Self { client })
    }
}

#[cfg(feature = "magic-gateway")]
impl IndexBarsSource for TdxIndexBarsSource {
    fn fetch_page(&self, request: IndexPageRequest) -> Result<Vec<RawIndexBar>, GatewayError> {
        self.client
            .get_index_bars(
                request.category,
                request.market,
                request.code,
                request.offset,
                request.count,
                request.fq_type,
            )
            .map(|rows| {
                rows.into_iter()
                    .map(|row| RawIndexBar {
                        year: row.year,
                        month: row.month,
                        day: row.day,
                        hour: row.hour,
                        minute: row.minute,
                        datetime: row.datetime,
                        open: row.open,
                        high: row.high,
                        low: row.low,
                        close: row.close,
                        volume: Some(row.vol),
                        amount: Some(row.amount),
                        up_count: row.up_count,
                        down_count: row.down_count,
                    })
                    .collect()
            })
            .map_err(|error| {
                benchmark_gateway_error(
                    BenchmarkAuditOutcome::Partial,
                    "provider_transport",
                    true,
                    format!("Magic TDX index page failed: {error}"),
                )
            })
    }
}

#[cfg(test)]
fn acquire_benchmark_batch_from_source(
    source: &impl IndexBarsSource,
    request: BenchmarkRequest,
    registry: &BenchmarkRegistry,
    attestation: BenchmarkProviderAttestation,
    coverage: BenchmarkAdmissionCoverage<'_>,
    observed_at: &str,
) -> Result<PreparedBenchmarkBatch, GatewayError> {
    validate_benchmark_request(registry, &request).map_err(benchmark_admission_error)?;
    attestation.admit(&request)?;
    fetch_and_admit_benchmark_batch(source, request, registry, coverage, observed_at)
}

fn fetch_and_admit_benchmark_batch(
    source: &impl IndexBarsSource,
    request: BenchmarkRequest,
    registry: &BenchmarkRegistry,
    coverage: BenchmarkAdmissionCoverage<'_>,
    observed_at: &str,
) -> Result<PreparedBenchmarkBatch, GatewayError> {
    validate_benchmark_request(registry, &request).map_err(benchmark_admission_error)?;
    let category = match request.range {
        BenchmarkRange::Daily { .. } => TDX_DAILY_CATEGORY,
        BenchmarkRange::Minute1 { .. } => TDX_MINUTE1_CATEGORY,
    };
    let requested_start = range_start_key(&request.range)?;
    let mut pages = Vec::<(u32, Vec<RawIndexBar>)>::new();
    let mut offset = 0u32;
    let mut previous_oldest = None;

    loop {
        let rows = source.fetch_page(IndexPageRequest {
            market: TDX_MARKET,
            code: TDX_CODE,
            category,
            fq_type: TDX_FQ_NONE,
            offset,
            count: TDX_PAGE_SIZE,
        })?;
        if rows.is_empty() {
            return Err(benchmark_gateway_error(
                BenchmarkAuditOutcome::Partial,
                "benchmark_page_empty_before_range",
                true,
                "TDX returned an empty page before the request start was covered",
            ));
        }
        if rows.len() > usize::from(TDX_PAGE_SIZE) {
            return Err(benchmark_gateway_error(
                BenchmarkAuditOutcome::Partial,
                "benchmark_page_size_invalid",
                false,
                "TDX returned more rows than the exact page request",
            ));
        }

        let keys = rows
            .iter()
            .map(|row| raw_time_key(row, &request.range))
            .collect::<Result<Vec<_>, _>>()?;
        if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(benchmark_gateway_error(
                BenchmarkAuditOutcome::Partial,
                "benchmark_page_order_or_duplicate",
                false,
                "TDX page is not strictly oldest-to-newest",
            ));
        }
        let oldest = keys.first().copied().ok_or_else(|| {
            benchmark_gateway_error(
                BenchmarkAuditOutcome::Partial,
                "benchmark_page_empty_before_range",
                true,
                "TDX page lost all rows during time validation",
            )
        })?;
        let newest = keys.last().copied().ok_or_else(|| {
            benchmark_gateway_error(
                BenchmarkAuditOutcome::Partial,
                "benchmark_page_empty_before_range",
                true,
                "TDX page lost all rows during time validation",
            )
        })?;
        if let Some(previous) = previous_oldest {
            if oldest >= previous {
                return Err(benchmark_gateway_error(
                    BenchmarkAuditOutcome::Partial,
                    "benchmark_page_did_not_advance",
                    false,
                    "TDX page oldest time did not move strictly into history",
                ));
            }
            if newest >= previous {
                return Err(benchmark_gateway_error(
                    BenchmarkAuditOutcome::Partial,
                    "benchmark_page_boundary_did_not_advance",
                    false,
                    "TDX page overlaps or repeats the preceding page boundary",
                ));
            }
        }
        previous_oldest = Some(oldest);
        let covered = oldest <= requested_start;
        let short = rows.len() < usize::from(TDX_PAGE_SIZE);
        pages.push((offset, rows));
        if covered {
            break;
        }
        if short {
            return Err(benchmark_gateway_error(
                BenchmarkAuditOutcome::Partial,
                "benchmark_short_page_before_range",
                true,
                "TDX returned a short page before the request start was covered",
            ));
        }
        offset = offset
            .checked_add(u32::from(TDX_PAGE_SIZE))
            .ok_or_else(|| {
                benchmark_gateway_error(
                    BenchmarkAuditOutcome::Partial,
                    "benchmark_page_offset_overflow",
                    false,
                    "TDX page offset overflowed before the request start was covered",
                )
            })?;
    }

    let canonical_bytes = canonical_acquisition_bytes(&request, category, &pages)?;
    let request_hash = domain_hash(b"BR251_TDX_INDEX_REQUEST_V1\0", &canonical_bytes);
    let batch_id = domain_hash(b"BR251_TDX_INDEX_BATCH_V1\0", &canonical_bytes);

    let mut selected = Vec::new();
    for (_, rows) in pages {
        for raw in rows {
            let (key, bar) = raw_to_benchmark_bar(raw, &request.range)?;
            if range_contains_key(&request.range, key)? {
                selected.push((key, bar));
            }
        }
    }
    selected.sort_by_key(|(key, _bar)| *key);
    let bars: Vec<_> = selected.into_iter().map(|(_key, bar)| bar).collect();
    let admitted = admit_benchmark_batch(registry, request, bars, coverage)
        .map_err(benchmark_admission_error)?;
    let evidence = BatchEvidence {
        provider: ProviderId::Tdx,
        source: format!("magic-tdx-index-bars@{TDX_DEPENDENCY_REVISION}"),
        source_at: None,
        observed_at: observed_at.to_owned(),
        batch_id,
    };
    Ok(PreparedBenchmarkBatch {
        batch: GatewayBatch::Available {
            records: admitted.into_bars(),
            evidence,
        },
        request_hash,
    })
}

fn raw_time_key(row: &RawIndexBar, range: &BenchmarkRange) -> Result<i64, GatewayError> {
    let year = i32::try_from(row.year).map_err(|_| {
        benchmark_gateway_error(
            BenchmarkAuditOutcome::Partial,
            "benchmark_raw_time_invalid",
            false,
            "TDX row year exceeds chrono range",
        )
    })?;
    let date = NaiveDate::from_ymd_opt(year, row.month, row.day).ok_or_else(|| {
        benchmark_gateway_error(
            BenchmarkAuditOutcome::Partial,
            "benchmark_raw_time_invalid",
            false,
            "TDX row date is invalid",
        )
    })?;
    let expected_datetime = match range {
        BenchmarkRange::Daily { .. } => {
            if row.hour != 0 || row.minute != 0 {
                return Err(benchmark_gateway_error(
                    BenchmarkAuditOutcome::Partial,
                    "benchmark_datetime_conflict",
                    false,
                    "TDX Daily datetime conflicts with nonzero time components",
                ));
            }
            date.format("%Y-%m-%d").to_string()
        }
        BenchmarkRange::Minute1 { .. } => {
            let local = date.and_hms_opt(row.hour, row.minute, 0).ok_or_else(|| {
                benchmark_gateway_error(
                    BenchmarkAuditOutcome::Partial,
                    "benchmark_raw_time_invalid",
                    false,
                    "TDX row minute is invalid",
                )
            })?;
            local.format("%Y-%m-%d %H:%M").to_string()
        }
    };
    if row.datetime != expected_datetime {
        return Err(benchmark_gateway_error(
            BenchmarkAuditOutcome::Partial,
            "benchmark_datetime_conflict",
            false,
            "TDX datetime conflicts with its numeric time components",
        ));
    }
    match range {
        BenchmarkRange::Daily { .. } => Ok(i64::from(date.num_days_from_ce())),
        BenchmarkRange::Minute1 { .. } => date
            .and_hms_opt(row.hour, row.minute, 0)
            .map(|at| at.and_utc().timestamp())
            .ok_or_else(|| {
                benchmark_gateway_error(
                    BenchmarkAuditOutcome::Partial,
                    "benchmark_raw_time_invalid",
                    false,
                    "TDX row minute is invalid",
                )
            }),
    }
}

fn canonical_acquisition_bytes(
    request: &BenchmarkRequest,
    category: u8,
    pages: &[(u32, Vec<RawIndexBar>)],
) -> Result<Vec<u8>, GatewayError> {
    let mut canonical = Vec::new();
    append_length_prefixed(&mut canonical, b"BR251_TDX_INDEX_ACQUISITION_V1")?;
    append_length_prefixed(&mut canonical, request.instrument.as_bytes())?;
    match &request.range {
        BenchmarkRange::Daily { from, to } => {
            append_length_prefixed(&mut canonical, b"Daily")?;
            append_length_prefixed(&mut canonical, from.to_string().as_bytes())?;
            append_length_prefixed(&mut canonical, to.to_string().as_bytes())?;
        }
        BenchmarkRange::Minute1 { from, to } => {
            append_length_prefixed(&mut canonical, b"Minute1")?;
            append_length_prefixed(&mut canonical, from.to_rfc3339().as_bytes())?;
            append_length_prefixed(&mut canonical, to.to_rfc3339().as_bytes())?;
        }
    }
    canonical.extend_from_slice(&[TDX_MARKET, category, TDX_FQ_NONE]);
    append_length_prefixed(&mut canonical, TDX_CODE.as_bytes())?;
    append_length_prefixed(&mut canonical, TDX_DEPENDENCY_REVISION.as_bytes())?;
    append_collection_len(&mut canonical, pages.len())?;
    for (offset, rows) in pages {
        canonical.extend_from_slice(&offset.to_be_bytes());
        append_collection_len(&mut canonical, rows.len())?;
        for row in rows {
            for value in [row.year, row.month, row.day, row.hour, row.minute] {
                canonical.extend_from_slice(&value.to_be_bytes());
            }
            append_length_prefixed(&mut canonical, row.datetime.as_bytes())?;
            for value in [row.open, row.high, row.low, row.close] {
                canonical.extend_from_slice(&value.to_bits().to_be_bytes());
            }
            append_optional_f64(&mut canonical, row.volume);
            append_optional_f64(&mut canonical, row.amount);
            canonical.extend_from_slice(&row.up_count.to_be_bytes());
            canonical.extend_from_slice(&row.down_count.to_be_bytes());
        }
    }
    Ok(canonical)
}

fn append_collection_len(target: &mut Vec<u8>, len: usize) -> Result<(), GatewayError> {
    let len = u64::try_from(len).map_err(|_| {
        benchmark_gateway_error(
            BenchmarkAuditOutcome::Partial,
            "benchmark_canonical_identity_unavailable",
            false,
            "canonical collection length exceeds u64",
        )
    })?;
    target.extend_from_slice(&len.to_be_bytes());
    Ok(())
}

fn append_length_prefixed(target: &mut Vec<u8>, bytes: &[u8]) -> Result<(), GatewayError> {
    append_collection_len(target, bytes.len())?;
    target.extend_from_slice(bytes);
    Ok(())
}

fn append_optional_f64(target: &mut Vec<u8>, value: Option<f64>) {
    match value {
        Some(value) => {
            target.push(1);
            target.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        None => target.push(0),
    }
}

fn range_start_key(range: &BenchmarkRange) -> Result<i64, GatewayError> {
    match range {
        BenchmarkRange::Daily { from, .. } => Ok(i64::from(from.num_days_from_ce())),
        BenchmarkRange::Minute1 { from, .. } => Ok(from.naive_local().and_utc().timestamp()),
    }
}

fn range_contains_key(range: &BenchmarkRange, key: i64) -> Result<bool, GatewayError> {
    let start = range_start_key(range)?;
    let end = match range {
        BenchmarkRange::Daily { to, .. } => i64::from(to.num_days_from_ce()),
        BenchmarkRange::Minute1 { to, .. } => to.naive_local().and_utc().timestamp(),
    };
    Ok(start <= key && key <= end)
}

fn raw_to_benchmark_bar(
    raw: RawIndexBar,
    range: &BenchmarkRange,
) -> Result<(i64, BenchmarkBar), GatewayError> {
    let key = raw_time_key(&raw, range)?;
    let year = i32::try_from(raw.year).map_err(|_| {
        benchmark_gateway_error(
            BenchmarkAuditOutcome::Partial,
            "benchmark_raw_time_invalid",
            false,
            "TDX row year exceeds chrono range",
        )
    })?;
    let date = NaiveDate::from_ymd_opt(year, raw.month, raw.day).ok_or_else(|| {
        benchmark_gateway_error(
            BenchmarkAuditOutcome::Partial,
            "benchmark_raw_time_invalid",
            false,
            "TDX row date is invalid",
        )
    })?;
    let at = match range {
        BenchmarkRange::Daily { .. } => BenchmarkBarTime::Daily(date),
        BenchmarkRange::Minute1 { .. } => {
            let local = date.and_hms_opt(raw.hour, raw.minute, 0).ok_or_else(|| {
                benchmark_gateway_error(
                    BenchmarkAuditOutcome::Partial,
                    "benchmark_raw_time_invalid",
                    false,
                    "TDX row minute is invalid",
                )
            })?;
            let offset = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
                benchmark_gateway_error(
                    BenchmarkAuditOutcome::Unavailable,
                    "benchmark_time_semantics_unavailable",
                    false,
                    "Asia/Shanghai fixed offset is unavailable",
                )
            })?;
            let at = offset.from_local_datetime(&local).single().ok_or_else(|| {
                benchmark_gateway_error(
                    BenchmarkAuditOutcome::Partial,
                    "benchmark_raw_time_invalid",
                    false,
                    "TDX row minute is ambiguous",
                )
            })?;
            BenchmarkBarTime::MinuteEnd(at)
        }
    };
    Ok((
        key,
        BenchmarkBar {
            at,
            open: raw.open,
            high: raw.high,
            low: raw.low,
            close: raw.close,
            volume: raw.volume,
            amount: raw.amount,
        },
    ))
}

fn domain_hash(domain: &[u8], canonical_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(canonical_bytes);
    hex::encode(hasher.finalize())
}

fn benchmark_admission_error(error: BenchmarkError) -> GatewayError {
    match error {
        BenchmarkError::Unsupported(BenchmarkUnsupported::UnsupportedInstrument) => {
            benchmark_gateway_error(
                BenchmarkAuditOutcome::Unsupported,
                "benchmark_instrument_unsupported",
                false,
                "benchmark instrument is not registered",
            )
        }
        BenchmarkError::Unsupported(BenchmarkUnsupported::TestIdentityRejected) => {
            benchmark_gateway_error(
                BenchmarkAuditOutcome::Unsupported,
                "benchmark_test_identity_rejected",
                false,
                "production benchmark registry rejects TEST_CODE",
            )
        }
        BenchmarkError::Unavailable { code, retryable } => {
            benchmark_gateway_error(BenchmarkAuditOutcome::Unavailable, code, retryable, code)
        }
        BenchmarkError::FailedIntegrity { code } => {
            benchmark_gateway_error(benchmark_integrity_audit_outcome(code), code, false, code)
        }
    }
}

fn benchmark_integrity_audit_outcome(code: &str) -> BenchmarkAuditOutcome {
    match code {
        "benchmark_range_reversed"
        | "benchmark_time_zone_invalid"
        | "benchmark_minute_range_crosses_day"
        | "benchmark_minute_range_off_grid" => BenchmarkAuditOutcome::InvalidRequest,
        _ => BenchmarkAuditOutcome::Partial,
    }
}

fn benchmark_gateway_error(
    audit_outcome: BenchmarkAuditOutcome,
    code: &'static str,
    retryable: bool,
    message: impl Into<String>,
) -> GatewayError {
    GatewayError::classified(
        BENCHMARK_CAPABILITY,
        Some(ProviderId::Tdx),
        audit_outcome.as_str(),
        code,
        retryable,
        message,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use chrono::{DateTime, Datelike, FixedOffset, NaiveDate};
    use diesel::prelude::*;
    use diesel::sql_types::{Integer, Text};
    use serial_test::serial;

    use super::{
        acquire_benchmark_batch_from_source, admit_benchmark_batch, BenchmarkAdmissionCoverage,
        BenchmarkBar, BenchmarkBarTime, BenchmarkError, BenchmarkProviderAttestation,
        BenchmarkRange, BenchmarkRegistry, BenchmarkRequest, BenchmarkUnsupported, IndexBarsSource,
        IndexPageRequest, RawIndexBar, HS300_CANONICAL,
    };
    use crate::data_gateway::GatewayError;
    use crate::database::DatabaseManager;

    #[derive(Debug, QueryableByName)]
    struct BenchmarkAuditRow {
        #[diesel(sql_type = Text)]
        outcome: String,
        #[diesel(sql_type = Text)]
        reason_code: String,
        #[diesel(sql_type = Integer)]
        retryable: i32,
    }

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, day).expect("valid test date")
    }

    fn minute(value: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value).expect("valid +08:00 minute end")
    }

    fn daily_request(from: NaiveDate, to: NaiveDate, instrument: &str) -> BenchmarkRequest {
        BenchmarkRequest {
            instrument: instrument.to_owned(),
            range: BenchmarkRange::Daily { from, to },
        }
    }

    fn minute_request(from: DateTime<FixedOffset>, to: DateTime<FixedOffset>) -> BenchmarkRequest {
        BenchmarkRequest {
            instrument: "TEST_CODE_000300".to_owned(),
            range: BenchmarkRange::Minute1 { from, to },
        }
    }

    fn bar(at: BenchmarkBarTime) -> BenchmarkBar {
        BenchmarkBar {
            at,
            open: 3_500.0,
            high: 3_510.0,
            low: 3_490.0,
            close: 3_505.0,
            volume: None,
            amount: None,
        }
    }

    fn test_registry() -> BenchmarkRegistry {
        BenchmarkRegistry::test_only(["TEST_CODE_000300"])
    }

    fn assert_failed_integrity(result: Result<super::AdmittedBenchmarkBatch, BenchmarkError>) {
        assert!(matches!(
            result,
            Err(BenchmarkError::FailedIntegrity { .. })
        ));
    }

    #[test]
    fn accepts_a_daily_batch_for_an_explicit_test_registry() {
        let trading_day = date(21);
        let request = daily_request(trading_day, trading_day, "TEST_CODE_000300");
        let bars = vec![bar(BenchmarkBarTime::Daily(trading_day))];

        let admitted = admit_benchmark_batch(
            &BenchmarkRegistry::test_only(["TEST_CODE_000300"]),
            request,
            bars,
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[trading_day],
            },
        )
        .expect("explicit test registry admits its test identity");

        assert_eq!(admitted.bars().len(), 1);
    }

    #[test]
    fn production_registry_only_accepts_the_canonical_hs300_identity() {
        let trading_day = date(21);
        let coverage = BenchmarkAdmissionCoverage::Daily {
            authoritative_trading_days: &[trading_day],
        };
        let bars = vec![bar(BenchmarkBarTime::Daily(trading_day))];

        assert!(admit_benchmark_batch(
            &BenchmarkRegistry::production_default(),
            daily_request(trading_day, trading_day, HS300_CANONICAL),
            bars.clone(),
            coverage,
        )
        .is_ok());
        assert_eq!(
            admit_benchmark_batch(
                &BenchmarkRegistry::production_default(),
                daily_request(trading_day, trading_day, "sh000905"),
                bars.clone(),
                coverage,
            ),
            Err(BenchmarkError::Unsupported(
                BenchmarkUnsupported::UnsupportedInstrument
            ))
        );
        assert_eq!(
            admit_benchmark_batch(
                &BenchmarkRegistry::production_default(),
                daily_request(trading_day, trading_day, "sz000001"),
                bars.clone(),
                coverage,
            ),
            Err(BenchmarkError::Unsupported(
                BenchmarkUnsupported::UnsupportedInstrument
            ))
        );
        assert_eq!(
            admit_benchmark_batch(
                &BenchmarkRegistry::production_default(),
                daily_request(trading_day, trading_day, "TEST_CODE_000300"),
                bars,
                coverage,
            ),
            Err(BenchmarkError::Unsupported(
                BenchmarkUnsupported::TestIdentityRejected
            ))
        );
    }

    #[test]
    fn rejects_reversed_ranges_and_non_shanghai_minute_ranges() {
        let day = date(21);
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            daily_request(day, date(20), "TEST_CODE_000300"),
            vec![bar(BenchmarkBarTime::Daily(day))],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            minute_request(
                minute("2026-08-21T09:31:00+09:00"),
                minute("2026-08-21T09:32:00+09:00"),
            ),
            vec![],
            BenchmarkAdmissionCoverage::Minute1,
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            minute_request(
                minute("2026-08-21T09:32:00+08:00"),
                minute("2026-08-21T09:31:00+08:00"),
            ),
            vec![],
            BenchmarkAdmissionCoverage::Minute1,
        ));
    }

    #[test]
    fn rejects_invalid_ohlc_optional_turnover_and_mismatched_time_kinds() {
        let day = date(21);
        for malformed in [
            BenchmarkBar {
                open: 0.0,
                ..bar(BenchmarkBarTime::Daily(day))
            },
            BenchmarkBar {
                high: f64::INFINITY,
                ..bar(BenchmarkBarTime::Daily(day))
            },
            BenchmarkBar {
                low: 3_506.0,
                ..bar(BenchmarkBarTime::Daily(day))
            },
            BenchmarkBar {
                volume: Some(-1.0),
                ..bar(BenchmarkBarTime::Daily(day))
            },
            BenchmarkBar {
                amount: Some(f64::NAN),
                ..bar(BenchmarkBarTime::Daily(day))
            },
            bar(BenchmarkBarTime::MinuteEnd(minute(
                "2026-08-21T09:31:00+08:00",
            ))),
        ] {
            assert_failed_integrity(admit_benchmark_batch(
                &test_registry(),
                daily_request(day, day, "TEST_CODE_000300"),
                vec![malformed],
                BenchmarkAdmissionCoverage::Daily {
                    authoritative_trading_days: &[day],
                },
            ));
        }
    }

    #[test]
    fn daily_batches_require_exact_explicit_authoritative_coverage_and_order() {
        let d1 = date(19);
        let d2 = date(20);
        let d3 = date(21);
        let request = daily_request(d1, d3, "TEST_CODE_000300");
        let complete = vec![
            bar(BenchmarkBarTime::Daily(d1)),
            bar(BenchmarkBarTime::Daily(d2)),
            bar(BenchmarkBarTime::Daily(d3)),
        ];
        assert!(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            complete.clone(),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d2, d3],
            },
        )
        .is_ok());
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            vec![
                bar(BenchmarkBarTime::Daily(d1)),
                bar(BenchmarkBarTime::Daily(d3)),
            ],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d2, d3],
            },
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            vec![
                bar(BenchmarkBarTime::Daily(d1)),
                bar(BenchmarkBarTime::Daily(d1)),
            ],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d2, d3],
            },
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request,
            vec![
                bar(BenchmarkBarTime::Daily(d2)),
                bar(BenchmarkBarTime::Daily(d1)),
                bar(BenchmarkBarTime::Daily(d3)),
            ],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d2, d3],
            },
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            daily_request(d1, d3, "TEST_CODE_000300"),
            complete,
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d3],
            },
        ));
    }

    #[test]
    fn minute_batches_require_the_session_grid_but_allow_the_lunch_break() {
        let m1129 = minute("2026-08-21T11:29:00+08:00");
        let m1130 = minute("2026-08-21T11:30:00+08:00");
        let m1301 = minute("2026-08-21T13:01:00+08:00");
        let m1302 = minute("2026-08-21T13:02:00+08:00");
        let request = minute_request(m1129, m1302);
        let complete = vec![
            bar(BenchmarkBarTime::MinuteEnd(m1129)),
            bar(BenchmarkBarTime::MinuteEnd(m1130)),
            bar(BenchmarkBarTime::MinuteEnd(m1301)),
            bar(BenchmarkBarTime::MinuteEnd(m1302)),
        ];
        assert!(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            complete.clone(),
            BenchmarkAdmissionCoverage::Minute1,
        )
        .is_ok());
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            vec![
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1130)),
                bar(BenchmarkBarTime::MinuteEnd(m1302)),
            ],
            BenchmarkAdmissionCoverage::Minute1,
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            vec![
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1130)),
                bar(BenchmarkBarTime::MinuteEnd(minute(
                    "2026-08-21T11:31:00+08:00",
                ))),
                bar(BenchmarkBarTime::MinuteEnd(m1301)),
                bar(BenchmarkBarTime::MinuteEnd(m1302)),
            ],
            BenchmarkAdmissionCoverage::Minute1,
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request,
            vec![
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1130)),
                bar(BenchmarkBarTime::MinuteEnd(m1301)),
                bar(BenchmarkBarTime::MinuteEnd(m1302)),
            ],
            BenchmarkAdmissionCoverage::Minute1,
        ));
        let descending = admit_benchmark_batch(
            &test_registry(),
            minute_request(m1129, m1301),
            vec![
                bar(BenchmarkBarTime::MinuteEnd(m1130)),
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1301)),
            ],
            BenchmarkAdmissionCoverage::Minute1,
        );
        assert!(matches!(
            descending,
            Err(BenchmarkError::FailedIntegrity { .. })
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            minute_request(m1129, minute("2026-08-22T09:31:00+08:00")),
            complete,
            BenchmarkAdmissionCoverage::Minute1,
        ));
    }

    #[test]
    fn empty_batches_are_typed_unavailable_and_large_source_moves_are_preserved() {
        let day = date(21);
        assert_eq!(
            admit_benchmark_batch(
                &test_registry(),
                daily_request(day, day, "TEST_CODE_000300"),
                vec![],
                BenchmarkAdmissionCoverage::Daily {
                    authoritative_trading_days: &[day],
                },
            ),
            Err(BenchmarkError::Unavailable {
                code: "benchmark_batch_empty",
                retryable: true,
            })
        );
        let large_move = BenchmarkBar {
            open: 100.0,
            high: 135.0,
            low: 99.0,
            close: 130.0,
            ..bar(BenchmarkBarTime::Daily(day))
        };
        assert!(admit_benchmark_batch(
            &test_registry(),
            daily_request(day, day, "TEST_CODE_000300"),
            vec![large_move],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
        )
        .is_ok());
    }

    struct TestIndexBarsSource {
        pages: Mutex<VecDeque<Result<Vec<RawIndexBar>, GatewayError>>>,
        requests: Mutex<Vec<IndexPageRequest>>,
    }

    impl TestIndexBarsSource {
        fn new(pages: Vec<Result<Vec<RawIndexBar>, GatewayError>>) -> Self {
            Self {
                pages: Mutex::new(pages.into()),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn offsets(&self) -> Vec<u32> {
            self.requests
                .lock()
                .expect("TEST_CODE request lock")
                .iter()
                .map(|request| request.offset)
                .collect()
        }
    }

    impl IndexBarsSource for TestIndexBarsSource {
        fn fetch_page(&self, request: IndexPageRequest) -> Result<Vec<RawIndexBar>, GatewayError> {
            self.requests
                .lock()
                .expect("TEST_CODE request lock")
                .push(request);
            self.pages
                .lock()
                .expect("TEST_CODE page lock")
                .pop_front()
                .expect("TEST_CODE page fixture exhausted")
        }
    }

    fn raw_daily(date: NaiveDate) -> RawIndexBar {
        RawIndexBar {
            year: date.year() as u32,
            month: date.month(),
            day: date.day(),
            hour: 0,
            minute: 0,
            datetime: date.format("%Y-%m-%d").to_string(),
            open: 3_500.0,
            high: 3_510.0,
            low: 3_490.0,
            close: 3_505.0,
            volume: None,
            amount: Some(8_000.0),
            up_count: 1_500,
            down_count: 1_200,
        }
    }

    fn daily_rows(from: NaiveDate, count: usize) -> Vec<RawIndexBar> {
        (0..count)
            .map(|offset| {
                raw_daily(
                    from.checked_add_signed(chrono::Duration::days(offset as i64))
                        .expect("TEST_CODE date range"),
                )
            })
            .collect()
    }

    fn source_failure() -> GatewayError {
        super::benchmark_gateway_error(
            super::BenchmarkAuditOutcome::Partial,
            "TEST_CODE_page_failure",
            true,
            "TEST_CODE page failed",
        )
    }

    #[test]
    fn adapter_fetches_offsets_zero_800_1600_and_preserves_optional_turnover() {
        let from = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let all_rows = daily_rows(from, 2_400);
        let source = TestIndexBarsSource::new(vec![
            Ok(all_rows[1_600..2_400].to_vec()),
            Ok(all_rows[800..1_600].to_vec()),
            Ok(all_rows[0..800].to_vec()),
        ]);
        let authority: Vec<_> = all_rows
            .iter()
            .map(|row| NaiveDate::from_ymd_opt(row.year as i32, row.month, row.day).unwrap())
            .collect();

        let prepared = acquire_benchmark_batch_from_source(
            &source,
            daily_request(from, *authority.last().unwrap(), "TEST_CODE_000300"),
            &test_registry(),
            BenchmarkProviderAttestation::test_only(true, true),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &authority,
            },
            "2099-01-02T10:00:00+08:00",
        )
        .expect("three complete TEST_CODE pages");

        assert_eq!(source.offsets(), vec![0, 800, 1_600]);
        assert_eq!(prepared.batch.records().len(), 2_400);
        assert_eq!(prepared.batch.records()[0].volume, None);
        assert_eq!(prepared.batch.records()[0].amount, Some(8_000.0));
        assert_eq!(prepared.batch.evidence().source_at, None);
        let requests = source.requests.lock().unwrap();
        assert!(requests.iter().all(|request| {
            request.market == 1
                && request.code == "000300"
                && request.category == 4
                && request.fq_type == 0
                && request.count == 800
        }));
    }

    #[test]
    fn adapter_discards_all_pages_on_error_empty_duplicate_or_nonadvancing_boundary() {
        let newest_start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let newest = daily_rows(newest_start, 800);
        let overlapping_boundary = vec![
            raw_daily(NaiveDate::from_ymd_opt(2023, 12, 31).unwrap()),
            raw_daily(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
        ];
        let requested_from = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let newest_end = newest.last().unwrap();
        let request = daily_request(
            requested_from,
            NaiveDate::from_ymd_opt(newest_end.year as i32, newest_end.month, newest_end.day)
                .unwrap(),
            "TEST_CODE_000300",
        );
        let authority = [requested_from];
        let cases = [
            (
                TestIndexBarsSource::new(vec![Ok(newest.clone()), Err(source_failure())]),
                "TEST_CODE_page_failure",
                true,
            ),
            (
                TestIndexBarsSource::new(vec![Ok(newest.clone()), Ok(vec![])]),
                "benchmark_page_empty_before_range",
                true,
            ),
            (
                TestIndexBarsSource::new(vec![Ok(newest.clone()), Ok(newest.clone())]),
                "benchmark_page_did_not_advance",
                false,
            ),
            (
                TestIndexBarsSource::new(vec![Ok(newest.clone()), Ok(overlapping_boundary)]),
                "benchmark_page_boundary_did_not_advance",
                false,
            ),
        ];

        for (source, expected_reason, expected_retryable) in cases {
            let error = acquire_benchmark_batch_from_source(
                &source,
                request.clone(),
                &test_registry(),
                BenchmarkProviderAttestation::test_only(true, true),
                BenchmarkAdmissionCoverage::Daily {
                    authoritative_trading_days: &authority,
                },
                "2099-01-02T10:00:00+08:00",
            )
            .expect_err("partial pages must never escape");
            assert_eq!(error.reason_code(), expected_reason);
            assert_eq!(error.audit_outcome(), "partial");
            assert_eq!(error.retryable(), expected_retryable);
        }
    }

    #[test]
    #[serial]
    fn audit_persists_benchmark_outcome_independently_from_retryability() {
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        let day = date(21);
        let empty_source = TestIndexBarsSource::new(vec![]);
        let unsupported = acquire_benchmark_batch_from_source(
            &empty_source,
            daily_request(day, day, "sh000905"),
            &BenchmarkRegistry::production_default(),
            BenchmarkProviderAttestation::test_only(true, true),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
            "2099-01-02T10:00:00+08:00",
        )
        .expect_err("unregistered benchmark must be unsupported");
        let identity_unavailable = acquire_benchmark_batch_from_source(
            &empty_source,
            daily_request(day, day, HS300_CANONICAL),
            &BenchmarkRegistry::production_default(),
            BenchmarkProviderAttestation::production_default(),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
            "2099-01-02T10:00:00+08:00",
        )
        .expect_err("identity attestation is unavailable");
        let time_unavailable = acquire_benchmark_batch_from_source(
            &empty_source,
            minute_request(
                minute("2026-08-21T09:31:00+08:00"),
                minute("2026-08-21T09:31:00+08:00"),
            ),
            &test_registry(),
            BenchmarkProviderAttestation::test_only(true, false),
            BenchmarkAdmissionCoverage::Minute1,
            "2099-01-02T10:00:00+08:00",
        )
        .expect_err("minute end-label attestation is unavailable");
        let invalid_request = acquire_benchmark_batch_from_source(
            &empty_source,
            daily_request(day, date(20), "TEST_CODE_000300"),
            &test_registry(),
            BenchmarkProviderAttestation::test_only(true, true),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
            "2099-01-02T10:00:00+08:00",
        )
        .expect_err("reversed range is a caller error");

        let newest = daily_rows(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), 800);
        let page_source = TestIndexBarsSource::new(vec![Ok(newest.clone()), Ok(newest.clone())]);
        let page_partial = acquire_benchmark_batch_from_source(
            &page_source,
            daily_request(
                NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 8, 21).unwrap(),
                "TEST_CODE_000300",
            ),
            &test_registry(),
            BenchmarkProviderAttestation::test_only(true, true),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
            "2099-01-02T10:00:00+08:00",
        )
        .expect_err("nonadvancing page is a partial acquisition");

        let cases = [
            (
                "unsupported",
                unsupported,
                "unsupported",
                "benchmark_instrument_unsupported",
                false,
            ),
            (
                "identity",
                identity_unavailable,
                "unavailable",
                "benchmark_identity_unverified",
                false,
            ),
            (
                "time",
                time_unavailable,
                "unavailable",
                "benchmark_time_semantics_unavailable",
                false,
            ),
            (
                "invalid-request",
                invalid_request,
                "invalid_request",
                "benchmark_range_reversed",
                false,
            ),
            (
                "page-integrity",
                page_partial,
                "partial",
                "benchmark_page_did_not_advance",
                false,
            ),
        ];

        for (label, error, expected_outcome, expected_reason, expected_retryable) in cases {
            let request_hash = crate::data_gateway::review::acquisition_request_hash(
                "TEST_CODE-BenchmarkOutcome",
                label,
            );
            let returned =
                crate::data_gateway::review::audit_gateway_result_with_receipt::<BenchmarkBar>(
                    "TEST_CODE-BenchmarkOutcome",
                    crate::magic_compat::ProviderId::Tdx,
                    &request_hash,
                    Err(error),
                )
                .expect_err("typed benchmark failure returns only after audit");
            assert_eq!(returned.audit_outcome(), expected_outcome);
            assert_eq!(returned.reason_code(), expected_reason);
            assert_eq!(returned.retryable(), expected_retryable);

            let mut connection = DatabaseManager::get().get_conn().unwrap();
            let row = diesel::sql_query(
                "SELECT outcome, reason_code, retryable FROM data_acquisition_audit \
                 WHERE capability = ? AND request_hash = ? ORDER BY id DESC LIMIT 1",
            )
            .bind::<Text, _>("TEST_CODE-BenchmarkOutcome")
            .bind::<Text, _>(&request_hash)
            .get_result::<BenchmarkAuditRow>(&mut *connection)
            .expect("TEST_CODE benchmark outcome must be durably audited");
            assert_eq!(row.outcome, expected_outcome);
            assert_eq!(row.reason_code, expected_reason);
            assert_eq!(row.retryable, i32::from(expected_retryable));
        }
    }

    #[test]
    fn canonical_request_and_batch_id_bind_instrument_protocol_revision_and_full_rows() {
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let registry = BenchmarkRegistry::test_only(["TEST_CODE_000300", "TEST_CODE_ALT"]);
        let acquire = |instrument: &str, raw: RawIndexBar| {
            let source = TestIndexBarsSource::new(vec![Ok(vec![raw])]);
            acquire_benchmark_batch_from_source(
                &source,
                daily_request(day, day, instrument),
                &registry,
                BenchmarkProviderAttestation::test_only(true, true),
                BenchmarkAdmissionCoverage::Daily {
                    authoritative_trading_days: &[day],
                },
                "2099-01-02T10:00:00+08:00",
            )
            .expect("single-row TEST_CODE acquisition")
        };

        let base = raw_daily(day);
        let first = acquire("TEST_CODE_000300", base.clone());
        let mut amount_changed = base.clone();
        amount_changed.amount = Some(8_001.0);
        let changed_row = acquire("TEST_CODE_000300", amount_changed);
        let changed_instrument = acquire("TEST_CODE_ALT", base.clone());

        assert_ne!(first.request_hash, changed_row.request_hash);
        assert_ne!(
            first.batch.evidence().batch_id,
            changed_row.batch.evidence().batch_id
        );
        assert_ne!(first.request_hash, changed_instrument.request_hash);
        assert_ne!(
            first.batch.evidence().batch_id,
            changed_instrument.batch.evidence().batch_id
        );
        let canonical_identity = |raw| {
            let request = daily_request(day, day, "TEST_CODE_000300");
            let canonical = super::canonical_acquisition_bytes(
                &request,
                super::TDX_DAILY_CATEGORY,
                &[(0, vec![raw])],
            )
            .expect("TEST_CODE canonical identity");
            (
                super::domain_hash(b"BR251_TDX_INDEX_REQUEST_V1\0", &canonical),
                super::domain_hash(b"BR251_TDX_INDEX_BATCH_V1\0", &canonical),
            )
        };
        let base_identity = canonical_identity(base.clone());
        for changed in [
            RawIndexBar {
                datetime: "2026-08-21-changed".to_owned(),
                ..base.clone()
            },
            RawIndexBar {
                up_count: base.up_count + 1,
                ..base.clone()
            },
            RawIndexBar {
                down_count: base.down_count + 1,
                ..base.clone()
            },
        ] {
            let changed_identity = canonical_identity(changed);
            assert_ne!(base_identity.0, changed_identity.0);
            assert_ne!(base_identity.1, changed_identity.1);
        }
        assert!(first
            .batch
            .evidence()
            .source
            .contains("75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e"));

        let mut malformed = raw_daily(day);
        malformed.close = f64::NAN;
        let source = TestIndexBarsSource::new(vec![Ok(vec![malformed])]);
        let error = acquire_benchmark_batch_from_source(
            &source,
            daily_request(day, day, "TEST_CODE_000300"),
            &registry,
            BenchmarkProviderAttestation::test_only(true, true),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
            "2099-01-02T10:00:00+08:00",
        )
        .expect_err("IEEE-754 canonicalization must not hide invalid source values");
        assert_eq!(error.reason_code(), "benchmark_ohlc_not_positive_finite");

        for conflicting in [
            RawIndexBar {
                datetime: "2026-08-20".to_owned(),
                ..base.clone()
            },
            RawIndexBar { hour: 15, ..base },
        ] {
            let source = TestIndexBarsSource::new(vec![Ok(vec![conflicting])]);
            let error = acquire_benchmark_batch_from_source(
                &source,
                daily_request(day, day, "TEST_CODE_000300"),
                &registry,
                BenchmarkProviderAttestation::test_only(true, true),
                BenchmarkAdmissionCoverage::Daily {
                    authoritative_trading_days: &[day],
                },
                "2099-01-02T10:00:00+08:00",
            )
            .expect_err("upstream datetime must match its numeric components");
            assert_eq!(error.reason_code(), "benchmark_datetime_conflict");
        }
    }

    #[test]
    fn daily_authority_is_typed_unavailable_outside_the_immutable_calendar() {
        let day = NaiveDate::from_ymd_opt(2099, 1, 5).unwrap();
        let error = super::verified_benchmark_coverage(&BenchmarkRequest {
            instrument: HS300_CANONICAL.to_owned(),
            range: BenchmarkRange::Daily { from: day, to: day },
        })
        .expect_err("calendar coverage must not be guessed from weekdays");

        assert_eq!(
            error.reason_code(),
            "benchmark_trading_calendar_unavailable"
        );
    }

    #[test]
    fn adapter_rejects_a_short_page_that_does_not_cover_the_request_start() {
        let page = daily_rows(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), 2);
        let source = TestIndexBarsSource::new(vec![Ok(page)]);
        let from = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();

        let error = acquire_benchmark_batch_from_source(
            &source,
            daily_request(
                from,
                NaiveDate::from_ymd_opt(2024, 1, 2).unwrap(),
                "TEST_CODE_000300",
            ),
            &test_registry(),
            BenchmarkProviderAttestation::test_only(true, true),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[from],
            },
            "2099-01-02T10:00:00+08:00",
        )
        .expect_err("short uncovered page is an incomplete batch");

        assert_eq!(error.reason_code(), "benchmark_short_page_before_range");
        assert_eq!(source.offsets(), vec![0]);
    }

    #[test]
    fn attestation_fails_before_source_access_and_minute_semantics_are_independent() {
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let source = TestIndexBarsSource::new(vec![]);
        let identity_error = acquire_benchmark_batch_from_source(
            &source,
            BenchmarkRequest {
                instrument: HS300_CANONICAL.to_owned(),
                range: BenchmarkRange::Daily { from: day, to: day },
            },
            &BenchmarkRegistry::production_default(),
            BenchmarkProviderAttestation::production_default(),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
            "2099-01-02T10:00:00+08:00",
        )
        .expect_err("production identity is not attested");
        assert_eq!(
            identity_error.reason_code(),
            "benchmark_identity_unverified"
        );

        let minute_error = acquire_benchmark_batch_from_source(
            &source,
            minute_request(
                minute("2026-08-21T09:31:00+08:00"),
                minute("2026-08-21T09:32:00+08:00"),
            ),
            &test_registry(),
            BenchmarkProviderAttestation::test_only(true, false),
            BenchmarkAdmissionCoverage::Minute1,
            "2099-01-02T10:00:00+08:00",
        )
        .expect_err("minute-end semantics are independently unattested");
        assert_eq!(
            minute_error.reason_code(),
            "benchmark_time_semantics_unavailable"
        );
        assert!(source.offsets().is_empty());
    }

    #[test]
    fn minute_adapter_is_mechanical_only_after_test_attestation() {
        let at = minute("2026-08-21T09:31:00+08:00");
        let source = TestIndexBarsSource::new(vec![Ok(vec![RawIndexBar {
            year: 2026,
            month: 8,
            day: 21,
            hour: 9,
            minute: 31,
            datetime: "2026-08-21 09:31".to_owned(),
            open: 3_500.0,
            high: 3_510.0,
            low: 3_490.0,
            close: 3_505.0,
            volume: Some(123.0),
            amount: None,
            up_count: 1_501,
            down_count: 1_199,
        }])]);

        let prepared = acquire_benchmark_batch_from_source(
            &source,
            minute_request(at, at),
            &test_registry(),
            BenchmarkProviderAttestation::test_only(true, true),
            BenchmarkAdmissionCoverage::Minute1,
            "2099-01-02T10:00:00+08:00",
        )
        .expect("attested TEST_CODE minute row follows Task 23 admission");

        assert_eq!(prepared.batch.records().len(), 1);
        assert_eq!(prepared.batch.records()[0].volume, Some(123.0));
        assert_eq!(prepared.batch.records()[0].amount, None);
        assert_eq!(prepared.batch.evidence().source_at, None);
        let requests = source.requests.lock().unwrap();
        assert_eq!(requests[0].category, 8);
    }
}
