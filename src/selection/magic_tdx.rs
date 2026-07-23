use crate::selection::model::{
    DirectMentionEvidence, SecurityIdentity, SecurityMarket, SecurityMasterSnapshot,
};
use crate::selection::quality::{
    validate_daily, validate_daily_freshness, validate_quote, PriceAdjustment, SelectionBar,
    SelectionQuote,
};
use crate::selection::relation::direct_mentions;
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use magic_tdx_rs::protocol::constants::{KLINE_5MIN, KLINE_DAILY};
use magic_tdx_rs::{SecurityBar, SecurityInfo, SecurityQuote, TdxService};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const DAILY_FETCH_COUNT: u16 = 64;
const FIVE_MINUTE_FETCH_COUNT: u16 = 288;
const REQUIRED_DAILY_BARS: usize = 21;
const MAGIC_TDX_CONNECT_TIMEOUT_SECONDS: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SelectionMarketWindow {
    Intraday,
    PostClose,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectionEventReference {
    pub event_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionMarketRequest {
    pub event_references: Vec<SelectionEventReference>,
    pub window: SelectionMarketWindow,
    pub evaluation_at: DateTime<Local>,
    pub expected_latest_settled_date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectionFiveMinuteBar {
    pub code: String,
    pub ended_at: DateTime<Local>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectionMarketRecord {
    pub security: SecurityIdentity,
    pub daily_bars: Vec<SelectionBar>,
    pub quote: Option<SelectionQuote>,
    pub five_minute_bars: Vec<SelectionFiveMinuteBar>,
    pub observed_at: DateTime<Local>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectionSourceRejection {
    pub event_id: Option<String>,
    pub security_code: Option<String>,
    pub reason_code: String,
    pub retryable: bool,
}

#[derive(Debug, Clone)]
pub struct SelectionMarketBatch {
    pub master: SecurityMasterSnapshot,
    pub event_mentions: BTreeMap<String, Vec<DirectMentionEvidence>>,
    pub records: Vec<SelectionMarketRecord>,
    pub rejections: Vec<SelectionSourceRejection>,
    pub observed_at: DateTime<Local>,
    pub batch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionSourceError {
    code: &'static str,
    message: String,
    retryable: bool,
}

impl SelectionSourceError {
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }

    fn join(error: tokio::task::JoinError) -> Self {
        source_error(
            "blocking_worker_join_failed",
            format!("Magic TDX blocking worker failed: {error}"),
            true,
        )
    }
}

impl std::fmt::Display for SelectionSourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SelectionSourceError {}

pub async fn run_magic_tdx_blocking<F, T>(operation: F) -> Result<T, SelectionSourceError>
where
    F: FnOnce() -> Result<T, SelectionSourceError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(SelectionSourceError::join)?
}

pub async fn fetch_selection_market_batch(
    request: SelectionMarketRequest,
) -> Result<SelectionMarketBatch, SelectionSourceError> {
    validate_request(&request)?;
    run_magic_tdx_blocking(move || fetch_selection_market_batch_blocking(request)).await
}

fn validate_request(request: &SelectionMarketRequest) -> Result<(), SelectionSourceError> {
    if request.expected_latest_settled_date > request.evaluation_at.date_naive() {
        return Err(source_error(
            "settled_date_future",
            "expected latest settled date is after evaluation date",
            false,
        ));
    }
    let mut event_ids = BTreeSet::new();
    for reference in &request.event_references {
        if reference.event_id.trim().is_empty() {
            return Err(source_error(
                "event_reference_id_empty",
                "selection event reference ID is empty",
                false,
            ));
        }
        if reference.text.trim().is_empty() {
            return Err(source_error(
                "event_reference_text_empty",
                "selection event reference text is empty",
                false,
            ));
        }
        if !event_ids.insert(reference.event_id.as_str()) {
            return Err(source_error(
                "duplicate_event_reference",
                format!(
                    "selection market request repeats event ID {}",
                    reference.event_id
                ),
                false,
            ));
        }
    }
    Ok(())
}

fn fetch_selection_market_batch_blocking(
    request: SelectionMarketRequest,
) -> Result<SelectionMarketBatch, SelectionSourceError> {
    validate_request(&request)?;
    let service = TdxService::new();
    let connected = service
        .client()
        .connect_to_any(Some(MAGIC_TDX_CONNECT_TIMEOUT_SECONDS))
        .map_err(|error| tdx_error("magic_tdx_connect_failed", error, true))?;
    if !connected {
        return Err(source_error(
            "magic_tdx_connect_failed",
            "Magic TDX did not confirm a connected server",
            true,
        ));
    }

    let mut master_records = service
        .security_list_all(1)
        .map_err(|error| tdx_error("security_master_shanghai_unavailable", error, true))?
        .into_iter()
        .map(|record| (1, record))
        .collect::<Vec<_>>();
    master_records.extend(
        service
            .security_list_all(0)
            .map_err(|error| tdx_error("security_master_shenzhen_unavailable", error, true))?
            .into_iter()
            .map(|record| (0, record)),
    );
    let master_observed_at = Local::now();
    let master = normalize_master(master_observed_at, master_records)?;

    let mut event_mentions = BTreeMap::new();
    let mut rejections = Vec::new();
    let mut securities = BTreeMap::<String, SecurityIdentity>::new();
    for reference in &request.event_references {
        match direct_mentions(&reference.text, &master) {
            Ok(mentions) => {
                for mention in &mentions {
                    securities
                        .entry(mention.security.code.clone())
                        .or_insert_with(|| mention.security.clone());
                }
                event_mentions.insert(reference.event_id.clone(), mentions);
            }
            Err(error) => {
                rejections.push(SelectionSourceRejection {
                    event_id: Some(reference.event_id.clone()),
                    security_code: None,
                    reason_code: error.reason_code().to_string(),
                    retryable: false,
                });
                event_mentions.insert(reference.event_id.clone(), Vec::new());
            }
        }
    }

    let mut records = Vec::new();
    for security in securities.into_values() {
        match fetch_security_record(&service, &security, &request) {
            Ok(record) => records.push(record),
            Err(error) => rejections.push(SelectionSourceRejection {
                event_id: None,
                security_code: Some(security.code.clone()),
                reason_code: error.code().to_string(),
                retryable: error.retryable(),
            }),
        }
    }
    records.sort_by(|left, right| left.security.code.cmp(&right.security.code));
    rejections.sort_by(|left, right| {
        left.event_id
            .cmp(&right.event_id)
            .then_with(|| left.security_code.cmp(&right.security_code))
            .then_with(|| left.reason_code.cmp(&right.reason_code))
    });
    let observed_at = Local::now();
    let batch_id = market_batch_id(&master, &event_mentions, &records, &rejections, observed_at)?;
    let batch = SelectionMarketBatch {
        master,
        event_mentions,
        records,
        rejections,
        observed_at,
        batch_id,
    };

    // The synchronous client and all of its resources are dropped before the
    // spawn_blocking closure returns to Tokio.
    drop(service);
    Ok(batch)
}

fn fetch_security_record(
    service: &TdxService,
    security: &SecurityIdentity,
    request: &SelectionMarketRequest,
) -> Result<SelectionMarketRecord, SelectionSourceError> {
    let market = market_number(security.market);
    let raw_daily = service
        .client()
        .get_security_bars(KLINE_DAILY, market, &security.code, 0, DAILY_FETCH_COUNT, 0)
        .map_err(|error| tdx_error("daily_bars_unavailable", error, true))?;
    let daily_bars = normalize_daily_bars(
        &security.code,
        raw_daily,
        request.expected_latest_settled_date,
    )?;

    let observed_at = Local::now();
    let (quote, five_minute_bars) = match request.window {
        SelectionMarketWindow::PostClose => (None, Vec::new()),
        SelectionMarketWindow::Intraday => {
            let mut quotes = service
                .client()
                .get_security_quotes(&[(market, security.code.as_str())])
                .map_err(|error| tdx_error("quote_unavailable", error, true))?;
            if quotes.len() != 1 {
                return Err(source_error(
                    "quote_cardinality_mismatch",
                    format!(
                        "Magic TDX returned {} quotes for one security",
                        quotes.len()
                    ),
                    true,
                ));
            }
            let quote = normalize_quote(quotes.remove(0), observed_at)?;
            if quote.code != security.code {
                return Err(source_error(
                    "quote_identity_mismatch",
                    "Magic TDX quote identity differs from requested security",
                    false,
                ));
            }
            let raw_five_minute = service
                .client()
                .get_security_bars(
                    KLINE_5MIN,
                    market,
                    &security.code,
                    0,
                    FIVE_MINUTE_FETCH_COUNT,
                    0,
                )
                .map_err(|error| tdx_error("five_minute_bars_unavailable", error, true))?;
            let five_minute =
                normalize_five_minute_bars(&security.code, raw_five_minute, request.evaluation_at)?;
            (Some(quote), five_minute)
        }
    };

    Ok(SelectionMarketRecord {
        security: security.clone(),
        daily_bars,
        quote,
        five_minute_bars,
        observed_at,
    })
}

fn normalize_master(
    observed_at: DateTime<Local>,
    records: Vec<(u8, SecurityInfo)>,
) -> Result<SecurityMasterSnapshot, SelectionSourceError> {
    let mut identities = Vec::new();
    for (market, record) in records {
        let Some(code) = normalized_equity_code(market, &record.code) else {
            continue;
        };
        let name = record.name.trim();
        if name.is_empty() {
            return Err(source_error(
                "security_name_missing",
                format!("Magic TDX security name is missing for code {code}"),
                false,
            ));
        }
        identities.push(SecurityIdentity {
            code,
            name: name.to_string(),
            market: security_market(market)?,
        });
    }
    if identities.is_empty() {
        return Err(source_error(
            "security_master_empty",
            "Magic TDX security master contains no supported A-share equities",
            true,
        ));
    }
    identities.sort_by(|left, right| left.code.cmp(&right.code));

    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis.selection_magic_tdx_master.v1\0");
    for identity in &identities {
        hasher.update(identity.code.as_bytes());
        hasher.update(b"\0");
        hasher.update(identity.name.as_bytes());
        hasher.update(b"\0");
        hasher.update(match identity.market {
            SecurityMarket::Shanghai => b"shanghai".as_slice(),
            SecurityMarket::Shenzhen => b"shenzhen".as_slice(),
        });
        hasher.update(b"\0");
    }
    let batch_id = format!(
        "selection_magic_tdx_master_v1_{}",
        hex::encode(hasher.finalize())
    );
    SecurityMasterSnapshot::new(identities, batch_id, observed_at).map_err(|error| {
        source_error(
            error.reason_code(),
            format!("Magic TDX security master rejected: {error}"),
            false,
        )
    })
}

fn normalize_quote(
    raw: SecurityQuote,
    observed_at: DateTime<Local>,
) -> Result<SelectionQuote, SelectionSourceError> {
    let server_time = raw.servertime.trim();
    if server_time.is_empty() {
        return Err(source_error(
            "quote_source_time_missing",
            "Magic TDX quote does not prove provider source time",
            true,
        ));
    }
    let time = NaiveTime::parse_from_str(server_time, "%H:%M:%S").map_err(|error| {
        source_error(
            "quote_source_time_invalid",
            format!("Magic TDX quote source time is invalid: {error}"),
            true,
        )
    })?;
    let source_at = Local
        .from_local_datetime(&observed_at.date_naive().and_time(time))
        .single()
        .ok_or_else(|| {
            source_error(
                "quote_source_time_invalid",
                "Magic TDX quote source time is ambiguous in local timezone",
                true,
            )
        })?;
    let code = normalized_equity_code(raw.market, &raw.code).ok_or_else(|| {
        source_error(
            "quote_security_unsupported",
            format!(
                "Magic TDX quote identity is not a supported A-share equity: market={} code={}",
                raw.market, raw.code
            ),
            false,
        )
    })?;
    let quote = SelectionQuote {
        code,
        price: raw.price,
        previous_close: raw.last_close,
        open: raw.open,
        high: raw.high,
        low: raw.low,
        observed_at,
        source_at,
        volume: raw.vol,
        amount: raw.amount,
    };
    validate_quote(&quote, observed_at).map_err(|error| {
        source_error(
            error.code(),
            format!("Magic TDX quote rejected: {error}"),
            true,
        )
    })?;
    Ok(quote)
}

fn normalize_daily_bars(
    code: &str,
    raw: Vec<SecurityBar>,
    expected_latest_settled_date: NaiveDate,
) -> Result<Vec<SelectionBar>, SelectionSourceError> {
    let mut bars = Vec::new();
    for source in raw {
        let market_date = NaiveDate::from_ymd_opt(source.year as i32, source.month, source.day)
            .ok_or_else(|| {
                source_error(
                    "daily_date_invalid",
                    format!(
                        "Magic TDX daily date is invalid: year={} month={} day={}",
                        source.year, source.month, source.day
                    ),
                    false,
                )
            })?;
        let datetime = source.datetime.trim();
        if !datetime.is_empty() && !datetime.starts_with(&market_date.to_string()) {
            return Err(source_error(
                "daily_date_inconsistent",
                format!("Magic TDX daily components {market_date} disagree with datetime field"),
                false,
            ));
        }
        if market_date > expected_latest_settled_date {
            continue;
        }
        bars.push(SelectionBar {
            code: code.to_string(),
            market_date,
            open: source.open,
            high: source.high,
            low: source.low,
            close: source.close,
            volume: source.vol,
            amount: source.amount,
            settled: true,
            adjustment: PriceAdjustment::Unadjusted,
            reference_previous_close: None,
        });
    }
    bars.sort_by_key(|bar| bar.market_date);
    if bars.len() < REQUIRED_DAILY_BARS {
        return Err(source_error(
            "daily_feature_history_insufficient",
            format!(
                "Magic TDX returned {} settled daily bars; {} required",
                bars.len(),
                REQUIRED_DAILY_BARS
            ),
            true,
        ));
    }
    let bars = bars.split_off(bars.len() - REQUIRED_DAILY_BARS);
    validate_daily(&bars).map_err(|error| {
        source_error(
            error.code(),
            format!("Magic TDX daily bars rejected: {error}"),
            error.code() == "daily_stale",
        )
    })?;
    validate_daily_freshness(&bars, expected_latest_settled_date).map_err(|error| {
        source_error(
            error.code(),
            format!("Magic TDX daily freshness rejected: {error}"),
            true,
        )
    })?;
    Ok(bars)
}

fn normalize_five_minute_bars(
    code: &str,
    raw: Vec<SecurityBar>,
    evaluated_at: DateTime<Local>,
) -> Result<Vec<SelectionFiveMinuteBar>, SelectionSourceError> {
    let mut bars = Vec::new();
    for source in raw {
        let date = NaiveDate::from_ymd_opt(source.year as i32, source.month, source.day)
            .ok_or_else(|| {
                source_error(
                    "five_minute_time_invalid",
                    "Magic TDX five-minute date is invalid",
                    false,
                )
            })?;
        let time = NaiveTime::from_hms_opt(source.hour, source.minute, 0).ok_or_else(|| {
            source_error(
                "five_minute_time_invalid",
                "Magic TDX five-minute clock time is invalid",
                false,
            )
        })?;
        if !valid_five_minute_slot(time) {
            return Err(source_error(
                "five_minute_outside_session",
                format!("Magic TDX five-minute slot {time} is outside the verified session grid"),
                false,
            ));
        }
        let ended_at = Local
            .from_local_datetime(&date.and_time(time))
            .single()
            .ok_or_else(|| {
                source_error(
                    "five_minute_time_invalid",
                    "Magic TDX five-minute timestamp is ambiguous",
                    false,
                )
            })?;
        let expected_datetime = format!("{date} {}", time.format("%H:%M"));
        if !source.datetime.trim().is_empty() && source.datetime.trim() != expected_datetime {
            return Err(source_error(
                "five_minute_time_inconsistent",
                "Magic TDX five-minute components disagree with datetime field",
                false,
            ));
        }
        if ended_at > evaluated_at {
            continue;
        }
        validate_intraday_values(
            ended_at,
            source.open,
            source.high,
            source.low,
            source.close,
            source.vol,
            source.amount,
        )?;
        bars.push(SelectionFiveMinuteBar {
            code: code.to_string(),
            ended_at,
            open: source.open,
            high: source.high,
            low: source.low,
            close: source.close,
            volume: source.vol,
            amount: source.amount,
        });
    }
    bars.sort_by_key(|bar| bar.ended_at);
    if bars.is_empty() {
        return Err(source_error(
            "five_minute_empty",
            "Magic TDX returned no completed five-minute bars",
            true,
        ));
    }
    for pair in bars.windows(2) {
        if pair[0].ended_at == pair[1].ended_at {
            return Err(source_error(
                "five_minute_duplicate",
                format!(
                    "duplicate Magic TDX five-minute bar at {}",
                    pair[0].ended_at
                ),
                false,
            ));
        }
        let expected = next_five_minute_slot(pair[0].ended_at)?;
        if pair[1].ended_at != expected {
            return Err(source_error(
                "five_minute_gap",
                format!(
                    "Magic TDX five-minute gap after {}: expected {}, got {}",
                    pair[0].ended_at, expected, pair[1].ended_at
                ),
                false,
            ));
        }
    }
    Ok(bars)
}

fn validate_intraday_values(
    ended_at: DateTime<Local>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    amount: f64,
) -> Result<(), SelectionSourceError> {
    if [open, high, low, close]
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0)
    {
        return Err(source_error(
            "five_minute_price_invalid",
            format!("Magic TDX five-minute bar {ended_at} has invalid price"),
            false,
        ));
    }
    if low > open.min(close) || open.max(close) > high || low > high {
        return Err(source_error(
            "five_minute_ohlc_inconsistent",
            format!("Magic TDX five-minute bar {ended_at} has inconsistent OHLC"),
            false,
        ));
    }
    if !volume.is_finite() || volume < 0.0 {
        return Err(source_error(
            "five_minute_volume_invalid",
            format!("Magic TDX five-minute bar {ended_at} has invalid volume"),
            false,
        ));
    }
    if !amount.is_finite() || amount < 0.0 {
        return Err(source_error(
            "five_minute_amount_invalid",
            format!("Magic TDX five-minute bar {ended_at} has invalid amount"),
            false,
        ));
    }
    Ok(())
}

fn valid_five_minute_slot(time: NaiveTime) -> bool {
    let morning_start = NaiveTime::from_hms_opt(9, 35, 0).expect("static valid time");
    let morning_end = NaiveTime::from_hms_opt(11, 30, 0).expect("static valid time");
    let afternoon_start = NaiveTime::from_hms_opt(13, 5, 0).expect("static valid time");
    let afternoon_end = NaiveTime::from_hms_opt(15, 0, 0).expect("static valid time");
    time.second() == 0
        && time.minute().is_multiple_of(5)
        && ((morning_start..=morning_end).contains(&time)
            || (afternoon_start..=afternoon_end).contains(&time))
}

fn next_five_minute_slot(
    current: DateTime<Local>,
) -> Result<DateTime<Local>, SelectionSourceError> {
    let morning_end = NaiveTime::from_hms_opt(11, 30, 0).expect("static valid time");
    let afternoon_start = NaiveTime::from_hms_opt(13, 5, 0).expect("static valid time");
    let afternoon_end = NaiveTime::from_hms_opt(15, 0, 0).expect("static valid time");
    let (date, time) = if current.time() < morning_end {
        (
            current.date_naive(),
            current.time() + chrono::Duration::minutes(5),
        )
    } else if current.time() == morning_end {
        (current.date_naive(), afternoon_start)
    } else if current.time() < afternoon_end {
        (
            current.date_naive(),
            current.time() + chrono::Duration::minutes(5),
        )
    } else {
        (
            crate::calendar::next_trading_day(current.date_naive()),
            NaiveTime::from_hms_opt(9, 35, 0).expect("static valid time"),
        )
    };
    Local
        .from_local_datetime(&NaiveDateTime::new(date, time))
        .single()
        .ok_or_else(|| {
            source_error(
                "five_minute_time_invalid",
                "next five-minute timestamp is ambiguous",
                false,
            )
        })
}

fn normalized_equity_code(market: u8, code: &str) -> Option<String> {
    let code = code.trim();
    #[cfg(test)]
    if code
        .strip_prefix("TEST_CODE_")
        .is_some_and(|value| value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return matches!(market, 0 | 1).then(|| code.to_string());
    }

    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let supported = match market {
        1 => ["600", "601", "603", "605", "688", "689"]
            .iter()
            .any(|prefix| code.starts_with(prefix)),
        0 => ["000", "001", "002", "003", "300", "301"]
            .iter()
            .any(|prefix| code.starts_with(prefix)),
        _ => false,
    };
    supported.then(|| code.to_string())
}

fn security_market(market: u8) -> Result<SecurityMarket, SelectionSourceError> {
    match market {
        1 => Ok(SecurityMarket::Shanghai),
        0 => Ok(SecurityMarket::Shenzhen),
        _ => Err(source_error(
            "capability_unavailable",
            format!("Magic TDX market {market} is outside the supported SH/SZ selection slice"),
            false,
        )),
    }
}

fn market_number(market: SecurityMarket) -> u8 {
    match market {
        SecurityMarket::Shanghai => 1,
        SecurityMarket::Shenzhen => 0,
    }
}

fn market_batch_id(
    master: &SecurityMasterSnapshot,
    event_mentions: &BTreeMap<String, Vec<DirectMentionEvidence>>,
    records: &[SelectionMarketRecord],
    rejections: &[SelectionSourceRejection],
    observed_at: DateTime<Local>,
) -> Result<String, SelectionSourceError> {
    #[derive(Serialize)]
    struct Evidence<'a> {
        master_batch_id: &'a str,
        event_mentions: &'a BTreeMap<String, Vec<DirectMentionEvidence>>,
        records: &'a [SelectionMarketRecord],
        rejections: &'a [SelectionSourceRejection],
        observed_at: String,
    }

    let canonical = serde_json::to_vec(&Evidence {
        master_batch_id: &master.batch_id,
        event_mentions,
        records,
        rejections,
        observed_at: observed_at.to_rfc3339(),
    })
    .map_err(|error| {
        source_error(
            "market_batch_hash_failed",
            format!("Magic TDX market batch serialization failed: {error}"),
            false,
        )
    })?;
    let mut hasher = Sha256::new();
    hasher.update(b"stock_analysis.selection_magic_tdx_batch.v1\0");
    hasher.update(canonical);
    Ok(format!(
        "selection_magic_tdx_batch_v1_{}",
        hex::encode(hasher.finalize())
    ))
}

fn tdx_error(
    code: &'static str,
    error: magic_tdx_rs::TdxError,
    retryable: bool,
) -> SelectionSourceError {
    source_error(
        code,
        format!("Magic TDX operation failed: {error}"),
        retryable,
    )
}

fn source_error(
    code: &'static str,
    message: impl Into<String>,
    retryable: bool,
) -> SelectionSourceError {
    let mut message = message.into();
    const MAX_MESSAGE_CHARS: usize = 256;
    if message.chars().count() > MAX_MESSAGE_CHARS {
        message = message.chars().take(MAX_MESSAGE_CHARS).collect();
        message.push('…');
    }
    SelectionSourceError {
        code,
        message,
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
    use magic_tdx_rs::SecurityBar;

    fn observed_at() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 7, 23, 10, 0, 1)
            .single()
            .expect("unambiguous local test time")
    }

    fn security_info(code: &str, name: &str) -> SecurityInfo {
        SecurityInfo {
            code: code.to_string(),
            volunit: 100,
            decimal_point: 2,
            name: name.to_string(),
            pre_close: 10.0,
        }
    }

    fn security_quote(server_time: &str) -> SecurityQuote {
        SecurityQuote {
            market: 0,
            code: "TEST_CODE_000001".to_string(),
            active1: 0,
            price: 10.1,
            last_close: 10.0,
            open: 10.0,
            high: 10.2,
            low: 9.9,
            servertime: server_time.to_string(),
            vol: 1_000.0,
            cur_vol: 100.0,
            amount: 10_100.0,
            s_vol: 0.0,
            b_vol: 0.0,
            bid1: 10.0,
            bid_vol1: 100.0,
            bid2: 0.0,
            bid_vol2: 0.0,
            bid3: 0.0,
            bid_vol3: 0.0,
            bid4: 0.0,
            bid_vol4: 0.0,
            bid5: 0.0,
            bid_vol5: 0.0,
            ask1: 10.1,
            ask_vol1: 100.0,
            ask2: 0.0,
            ask_vol2: 0.0,
            ask3: 0.0,
            ask_vol3: 0.0,
            ask4: 0.0,
            ask_vol4: 0.0,
            ask5: 0.0,
            ask_vol5: 0.0,
            reversed_bytes0: 0,
            reversed_bytes1: 0,
            reversed_bytes2: 0,
            reversed_bytes3: 0,
            reversed_bytes4: 0,
            reversed_bytes5: 0,
            reversed_bytes6: 0,
            reversed_bytes7: 0,
            reversed_bytes8: 0,
            reversed_bytes9: 0,
            active2: 0,
        }
    }

    fn daily_bar(date: NaiveDate, close: f64) -> SecurityBar {
        SecurityBar {
            open: close,
            close,
            high: close,
            low: close,
            vol: 1_000.0,
            amount: close * 1_000.0,
            year: date.year() as u32,
            month: date.month(),
            day: date.day(),
            hour: 15,
            minute: 0,
            datetime: format!("{date}"),
        }
    }

    fn consecutive_daily(count: usize) -> Vec<SecurityBar> {
        let mut date = NaiveDate::from_ymd_opt(2026, 6, 22).expect("valid test date");
        let mut bars = Vec::with_capacity(count);
        for index in 0..count {
            bars.push(daily_bar(date, 10.0 + index as f64 * 0.1));
            date = crate::calendar::next_trading_day(date);
        }
        bars
    }

    fn five_minute_bar(hour: u32, minute: u32) -> SecurityBar {
        SecurityBar {
            open: 10.0,
            close: 10.1,
            high: 10.2,
            low: 9.9,
            vol: 100.0,
            amount: 1_010.0,
            year: 2026,
            month: 7,
            day: 23,
            hour,
            minute,
            datetime: format!("2026-07-23 {hour:02}:{minute:02}"),
        }
    }

    #[test]
    fn normalizes_security_master_without_guessing_names() {
        let snapshot = normalize_master(
            observed_at(),
            vec![(0, security_info("TEST_CODE_000001", "测试银行"))],
        )
        .expect("valid master");

        assert_eq!(snapshot.identities().len(), 1);
        assert_eq!(snapshot.identities()[0].code, "TEST_CODE_000001");
        assert_eq!(snapshot.identities()[0].name, "测试银行");
        assert_eq!(snapshot.identities()[0].market, SecurityMarket::Shenzhen);
    }

    #[test]
    fn rejects_quote_when_server_time_cannot_be_proven() {
        let error = normalize_quote(security_quote(""), observed_at()).unwrap_err();
        assert_eq!(error.code(), "quote_source_time_missing");
    }

    #[test]
    fn quote_normalization_preserves_real_source_time() {
        let quote =
            normalize_quote(security_quote("10:00:00"), observed_at()).expect("valid quote");
        assert_eq!(quote.source_at.time().to_string(), "10:00:00");
        assert_eq!(quote.observed_at, observed_at());
    }

    #[test]
    fn normalizes_exactly_the_latest_twenty_one_settled_unadjusted_daily_bars() {
        let raw = consecutive_daily(25);
        let expected_latest = NaiveDate::from_ymd_opt(2026, 7, 24).expect("valid test date");
        let normalized = normalize_daily_bars("TEST_CODE_000001", raw, expected_latest)
            .expect("valid daily evidence");

        assert_eq!(normalized.len(), 21);
        assert!(normalized.iter().all(|bar| bar.settled));
        assert!(normalized
            .iter()
            .all(|bar| bar.adjustment == PriceAdjustment::Unadjusted));
        assert!(normalized.last().expect("latest bar").market_date <= expected_latest);
    }

    #[test]
    fn five_minute_normalization_rejects_duplicates_and_keeps_only_completed_slots() {
        let evaluated_at = Local
            .with_ymd_and_hms(2026, 7, 23, 9, 41, 0)
            .single()
            .expect("fixed local time");
        let bars = normalize_five_minute_bars(
            "TEST_CODE_000001",
            vec![
                five_minute_bar(9, 35),
                five_minute_bar(9, 40),
                five_minute_bar(9, 45),
            ],
            evaluated_at,
        )
        .expect("valid completed bars");
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[1].ended_at.time().to_string(), "09:40:00");

        let error = normalize_five_minute_bars(
            "TEST_CODE_000001",
            vec![five_minute_bar(9, 35), five_minute_bar(9, 35)],
            evaluated_at,
        )
        .unwrap_err();
        assert_eq!(error.code(), "five_minute_duplicate");
    }

    #[test]
    fn request_rejects_duplicate_event_ids_before_network_access() {
        let request = SelectionMarketRequest {
            event_references: vec![
                SelectionEventReference {
                    event_id: "event-a".to_string(),
                    text: "测试银行".to_string(),
                },
                SelectionEventReference {
                    event_id: "event-a".to_string(),
                    text: "另一条".to_string(),
                },
            ],
            window: SelectionMarketWindow::PostClose,
            evaluation_at: observed_at(),
            expected_latest_settled_date: observed_at().date_naive(),
        };

        assert_eq!(
            validate_request(&request).unwrap_err().code(),
            "duplicate_event_reference"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn blocking_boundary_returns_without_nested_runtime_drop() {
        let caller = std::thread::current().id();
        let worker =
            run_magic_tdx_blocking(|| Ok::<_, SelectionSourceError>(std::thread::current().id()))
                .await
                .expect("blocking worker succeeds");

        assert_ne!(worker, caller);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnected_service_is_created_and_dropped_inside_blocking_worker() {
        run_magic_tdx_blocking(|| {
            let service = magic_tdx_rs::TdxService::new();
            drop(service);
            Ok::<_, SelectionSourceError>(())
        })
        .await
        .expect("service drop stays outside async runtime");
    }

    #[test]
    fn production_adapter_has_no_fallback_or_nested_runtime_path() {
        let source = include_str!("magic_tdx.rs");
        for forbidden in [
            concat!("rust", "dx"),
            concat!("east", "money"),
            concat!("sina", "_hq"),
            concat!("tencent", "_qfq"),
            concat!("bao", "stock"),
            concat!("Runtime", "::new"),
            concat!(".block", "_on("),
        ] {
            assert!(
                !source.contains(forbidden),
                "forbidden selection source/runtime path: {forbidden}"
            );
        }
    }
}
