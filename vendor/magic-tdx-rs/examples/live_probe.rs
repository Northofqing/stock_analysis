use magic_market_core::{
    AssetClass, BarInterval, BarsRequest, ConceptHits, CorporateActionCategory,
    CorporateActionRequest, CorporateActionStatus, CorporateActionTerms, CorporateActions,
    Exchange, HistoricalBars, InstrumentId, IsoDate, MinuteData, MinuteDataRequest, OrderBooks,
    ProviderId, RealtimeQuotes, SecurityMetadataProvider, Trades, TradesRequest,
};
use magic_tdx_rs::{
    net::utils::today_yyyymmdd,
    protocol::constants::{KLINE_DAILY, PRIMARY_SERVERS},
    TdxDirectClient, TdxError, TdxSmartClient,
};

fn record_error(errors: &mut Vec<String>, label: &str, error: impl std::fmt::Display) {
    let message = format!("{label}: {error}");
    println!("{label}=error error={error}");
    errors.push(message);
}

fn require_count(errors: &mut Vec<String>, label: &str, actual: usize, expected: usize) {
    if actual != expected {
        errors.push(format!(
            "{label}: expected {expected} records, received {actual}"
        ));
    }
}

fn require_nonempty(errors: &mut Vec<String>, label: &str, actual: usize) {
    if actual == 0 {
        errors.push(format!("{label}: expected at least one record, received 0"));
    }
}

fn within_tolerance(actual: f64, expected: f64, tolerance: f64) -> bool {
    (actual - expected).abs() <= tolerance
}

struct MinuteProbe {
    current_count: usize,
    latest_session_count: usize,
    latest_session_date: u32,
    current_status: &'static str,
    server: String,
}

#[derive(Clone, Copy)]
enum CurrentSession {
    Weekend,
    PreOpen,
    Intraday,
    Midday,
    Complete,
}

impl CurrentSession {
    fn label(self) -> &'static str {
        match self {
            Self::Weekend => "expected_unavailable_weekend",
            Self::PreOpen => "expected_unavailable_before_open",
            Self::Intraday => "available_intraday",
            Self::Midday => "expected_unavailable_midday",
            Self::Complete => "expected_unavailable_after_close",
        }
    }

    fn diagnostic_label(self) -> &'static str {
        match self {
            Self::Weekend => "diagnostic_unadmitted_weekend",
            Self::PreOpen => "diagnostic_unadmitted_before_open",
            Self::Intraday => "admitted_intraday",
            Self::Midday => "diagnostic_unadmitted_midday",
            Self::Complete => "diagnostic_unadmitted_after_close",
        }
    }

    const fn is_active(self) -> bool {
        matches!(self, Self::Intraday)
    }
}

fn validate_raw_current_trades(
    errors: &mut Vec<String>,
    label: &str,
    times: &[&str],
    request_limit: usize,
    session: CurrentSession,
) {
    let actual = times.len();
    if actual > request_limit {
        errors.push(format!(
            "{label}: received {actual} records for request limit {request_limit}"
        ));
        return;
    }
    if session.is_active() {
        require_nonempty(errors, label, actual);
    } else {
        println!(
            "{label}_status={} count={actual}",
            session.diagnostic_label()
        );
    }
}

fn validate_normalized_current_trades(
    errors: &mut Vec<String>,
    label: &str,
    times: &[&str],
    request_limit: usize,
    session: CurrentSession,
) {
    if !session.is_active() {
        errors.push(format!(
            "{label}: normalized provider admitted {} records during {}",
            times.len(),
            session.label()
        ));
        return;
    }
    if times.len() > request_limit {
        errors.push(format!(
            "{label}: received {} records for request limit {request_limit}",
            times.len()
        ));
    } else {
        require_nonempty(errors, label, times.len());
    }
}

fn record_raw_current_diagnostic(
    errors: &mut Vec<String>,
    label: &str,
    error: TdxError,
    session: CurrentSession,
) {
    if session.is_active() {
        record_error(errors, label, error);
    } else {
        println!(
            "{label}_status={} diagnostic_error={error}",
            session.diagnostic_label()
        );
    }
}

fn record_normalized_current_error(
    errors: &mut Vec<String>,
    label: &str,
    error: TdxError,
    session: CurrentSession,
    family: &str,
) {
    let expected_message = format!(
        "TDX normalized current {family} is unavailable outside an active A-share weekday session"
    );
    let expected_unavailable = matches!(
        &error,
        TdxError::InvalidData(message) if message == &expected_message
    );
    if !session.is_active() && expected_unavailable {
        println!("{label}_status={} expected_error={error}", session.label());
    } else {
        record_error(errors, label, error);
    }
}

fn compact_date(value: &str) -> Option<u32> {
    let digits = value
        .bytes()
        .filter(u8::is_ascii_digit)
        .take(8)
        .collect::<Vec<_>>();
    if digits.len() != 8 {
        return None;
    }
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

fn china_clock() -> Result<(u8, u64), String> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before UNIX epoch: {error}"))?
        .as_secs()
        .checked_add(8 * 3600)
        .ok_or_else(|| "China-local probe clock overflow".to_string())?;
    let weekday = ((seconds / 86_400 + 4) % 7) as u8;
    Ok((weekday, seconds % 86_400))
}

fn current_session(china_weekday: u8, china_day_seconds: u64) -> CurrentSession {
    if china_weekday == 0 || china_weekday == 6 {
        CurrentSession::Weekend
    } else if china_day_seconds < 9 * 3600 + 30 * 60 {
        CurrentSession::PreOpen
    } else if china_day_seconds <= 11 * 3600 + 30 * 60 {
        CurrentSession::Intraday
    } else if china_day_seconds < 13 * 3600 {
        CurrentSession::Midday
    } else if china_day_seconds <= 15 * 3600 {
        CurrentSession::Intraday
    } else {
        CurrentSession::Complete
    }
}

fn completed_session_date(dates: &[u32], today: u32, session: CurrentSession) -> Option<u32> {
    dates
        .iter()
        .rev()
        .copied()
        .find(|date| session_date_is_complete(*date, today, session))
}

fn session_date_is_complete(date: u32, today: u32, session: CurrentSession) -> bool {
    match session {
        CurrentSession::PreOpen | CurrentSession::Intraday | CurrentSession::Midday => date < today,
        CurrentSession::Weekend | CurrentSession::Complete => date <= today,
    }
}

fn classify_current_minute(
    latest_session_date: u32,
    today: u32,
    china_weekday: u8,
    china_day_seconds: u64,
    current_count: usize,
) -> Result<&'static str, String> {
    if latest_session_date > today {
        return Err(format!(
            "latest session date {latest_session_date} is after local date {today}"
        ));
    }
    if current_count > 240 {
        return Err(format!(
            "current endpoint returned {current_count} records, maximum is 240"
        ));
    }
    let session = current_session(china_weekday, china_day_seconds);
    if session.is_active() {
        if current_count > 0 {
            Ok(session.label())
        } else {
            Err("current endpoint returned no records during an active session".to_string())
        }
    } else {
        Ok(session.diagnostic_label())
    }
}

fn probe_index_bars(
    market: u8,
    code: &str,
    requested: u16,
    minimum: usize,
) -> Result<(usize, String, String), String> {
    let mut failures = Vec::new();
    let today = today_yyyymmdd();
    let (weekday, day_seconds) = china_clock()?;
    let session = current_session(weekday, day_seconds);
    for (name, ip, port) in PRIMARY_SERVERS {
        let client = TdxDirectClient::new(ip, *port, 3.0);
        match client.get_index_bars(KLINE_DAILY, market, code, 0, requested, 0) {
            Ok(items) => {
                let completed = items
                    .iter()
                    .map(|item| compact_date(&item.datetime).map(|date| (date, item)))
                    .collect::<Option<Vec<_>>>();
                let Some(completed) = completed else {
                    failures.push(format!(
                        "{name}({ip}:{port}) returned an index bar without YYYYMMDD"
                    ));
                    continue;
                };
                let completed = completed
                    .into_iter()
                    .filter(|(date, _)| session_date_is_complete(*date, today, session))
                    .collect::<Vec<_>>();
                if completed.len() >= minimum {
                    let first = completed
                        .first()
                        .map_or_else(|| "none".to_string(), |(_, item)| item.datetime.clone());
                    return Ok((completed.len(), first, format!("{name}({ip}:{port})")));
                }
                failures.push(format!(
                    "{name}({ip}:{port}) returned {} total but only {} completed index bars, required {minimum}",
                    items.len(),
                    completed.len()
                ));
            }
            Err(error) => failures.push(format!("{name}({ip}:{port}) failed: {error}")),
        }
    }
    Err(format!(
        "no primary server returned at least {minimum} of {requested} requested index bars: {}",
        failures.join("; ")
    ))
}

fn probe_minute_data(market: u8, code: &str) -> Result<MinuteProbe, String> {
    let mut failures = Vec::new();
    let today = today_yyyymmdd();
    let (china_weekday, china_day_seconds) = china_clock()?;
    let session = current_session(china_weekday, china_day_seconds);
    for (name, ip, port) in PRIMARY_SERVERS {
        let client = TdxDirectClient::new(ip, *port, 3.0);
        let current_count = match client.get_minute_time_data(market, code) {
            Ok(items) => items.len(),
            Err(error) => {
                failures.push(format!("{name}({ip}:{port}) current failed: {error}"));
                continue;
            }
        };
        let daily_dates = match client.get_security_bars(KLINE_DAILY, market, code, 0, 2, 0) {
            Ok(items) => items
                .iter()
                .map(|item| {
                    compact_date(&item.datetime)
                        .ok_or_else(|| "daily bar has no YYYYMMDD date".to_string())
                })
                .collect::<Result<Vec<_>, _>>(),
            Err(error) => Err(format!("latest daily bar failed: {error}")),
        };
        let daily_dates = match daily_dates {
            Ok(value) => value,
            Err(error) => {
                failures.push(format!("{name}({ip}:{port}) {error}"));
                continue;
            }
        };
        let latest_source_date = match daily_dates.last().copied() {
            Some(value) => value,
            None => {
                failures.push(format!("{name}({ip}:{port}) returned no daily bars"));
                continue;
            }
        };
        let completed_date = match completed_session_date(&daily_dates, today, session) {
            Some(value) => value,
            None => {
                failures.push(format!(
                    "{name}({ip}:{port}) has no completed session in {:?}",
                    daily_dates
                ));
                continue;
            }
        };
        let current_status = match classify_current_minute(
            latest_source_date,
            today,
            china_weekday,
            china_day_seconds,
            current_count,
        ) {
            Ok(status) => status,
            Err(error) => {
                failures.push(format!("{name}({ip}:{port}) current invalid: {error}"));
                continue;
            }
        };
        match client.get_history_minute_time_data(market, code, completed_date) {
            Ok(items) if items.len() == 240 => {
                return Ok(MinuteProbe {
                    current_count,
                    latest_session_count: items.len(),
                    latest_session_date: completed_date,
                    current_status,
                    server: format!("{name}({ip}:{port})"),
                });
            }
            Ok(items) => failures.push(format!(
                "{name}({ip}:{port}) date={completed_date} current={current_count} latest={}",
                items.len()
            )),
            Err(error) => failures.push(format!(
                "{name}({ip}:{port}) date={completed_date} history failed: {error}"
            )),
        }
    }
    Err(format!(
        "no primary server returned 240 records for its latest daily-bar date: {}",
        failures.join("; ")
    ))
}

fn probe_normalized_bars(
    errors: &mut Vec<String>,
    client: &TdxSmartClient,
    label: &str,
    instrument: InstrumentId,
    interval: BarInterval,
) {
    let request = match BarsRequest::new(instrument, interval, 1) {
        Ok(request) => request,
        Err(error) => {
            record_error(errors, label, error);
            return;
        }
    };
    let batch = match client.historical_bars(&request) {
        Ok(batch) => batch,
        Err(error) => {
            record_error(errors, label, error);
            return;
        }
    };
    require_count(errors, label, batch.records().len(), 1);
    if batch.provenance().source() != "tdx-smart" {
        errors.push(format!(
            "{label}: unexpected provenance source {:?}",
            batch.provenance().source()
        ));
    }
    if batch.provenance().source_at().is_none() {
        errors.push(format!("{label}: provenance source_at is missing"));
    }
    if batch.provenance().fetched_at().parse::<u64>().is_err() {
        errors.push(format!(
            "{label}: provenance fetched_at is not an epoch timestamp"
        ));
    }
    let Some(batch_id) = batch.provenance().batch_id() else {
        errors.push(format!("{label}: provenance batch_id is missing"));
        return;
    };
    for bar in batch.records() {
        println!(
            "{label} provider={:?} source={} source_at={:?} fetched_at={} batch_id={} code={} interval={:?} start={} end={} open={} high={} low={} close={} volume_lots={} amount_yuan={:?} adjustment={:?} record_source_at={:?} record_batch_id={}",
            bar.provider(),
            batch.provenance().source(),
            batch.provenance().source_at(),
            batch.provenance().fetched_at(),
            batch_id,
            bar.instrument().code(),
            bar.interval(),
            bar.bar_start(),
            bar.bar_end(),
            bar.open().get(),
            bar.high().get(),
            bar.low().get(),
            bar.close().get(),
            bar.volume().get(),
            bar.amount().map(|value| value.get()),
            bar.adjustment(),
            bar.source_at(),
            bar.batch_id()
        );
        if bar.provider() != ProviderId::Tdx {
            errors.push(format!("{label}: record provider is not TDX"));
        }
        if bar.batch_id() != batch_id {
            errors.push(format!("{label}: record batch_id differs from provenance"));
        }
        if bar.source_at().is_none() {
            errors.push(format!("{label}: record source_at is missing"));
        }
        let volume_lots = bar.volume().get();
        if volume_lots > 0.0 {
            let Some(amount_yuan) = bar.amount().map(|value| value.get()) else {
                errors.push(format!("{label}: positive-volume bar has no amount"));
                continue;
            };
            let vwap = amount_yuan / (volume_lots * 100.0);
            let tolerance = ((bar.high().get() - bar.low().get()).abs() * 0.02).max(0.02);
            if vwap < bar.low().get() - tolerance || vwap > bar.high().get() + tolerance {
                errors.push(format!(
                    "{label}: volume unit evidence failed; vwap={vwap} low={} high={} tolerance={tolerance}",
                    bar.low().get(),
                    bar.high().get()
                ));
            }
        }
    }
}

fn main() {
    let mut errors = Vec::new();
    let session = match china_clock() {
        Ok((weekday, seconds)) => Some(current_session(weekday, seconds)),
        Err(error) => {
            record_error(&mut errors, "china_clock", error);
            None
        }
    };
    let client = TdxSmartClient::new();
    client.clear_cache();
    match client.connect_to_any(Some(3.0)) {
        Ok(true) => {
            println!("connected=true");
            println!("connected_server={:?}", client.inner().connected_server());
            match client.get_security_quotes(&[(1, "600396")]) {
                Ok(quotes) => {
                    println!(
                        "quotes={} first_price={}",
                        quotes.len(),
                        quotes.first().map_or(0.0, |q| q.price)
                    );
                    require_count(&mut errors, "quotes", quotes.len(), 1);
                }
                Err(error) => record_error(&mut errors, "quotes", error),
            }
            for candidate_market in [0_u8, 1, 2] {
                match client.get_security_quotes(&[(candidate_market, "920118")]) {
                    Ok(quotes) => {
                        println!(
                            "beijing_market_candidate={} count={} records={:?}",
                            candidate_market,
                            quotes.len(),
                            quotes
                                .iter()
                                .map(|quote| (
                                    quote.market,
                                    quote.code.as_str(),
                                    quote.price,
                                    quote.reversed_bytes0,
                                    quote.servertime.as_str()
                                ))
                                .collect::<Vec<_>>()
                        );
                    }
                    Err(error) => println!(
                        "beijing_market_candidate={} error={}",
                        candidate_market, error
                    ),
                }
            }
            let beijing_instrument =
                InstrumentId::new(Exchange::Beijing, "920118", AssetClass::Equity)
                    .expect("valid Beijing probe instrument");
            match client.realtime_quotes(std::slice::from_ref(&beijing_instrument)) {
                Ok(batch) => {
                    println!(
                        "beijing_quotes={} provenance={:?} quality={:?}",
                        batch.records().len(),
                        batch.provenance(),
                        batch.quality()
                    );
                    for quote in batch.records() {
                        println!(
                            "beijing_quote code={} price={} status={:?} source_at={:?} observed_at={} provider={:?} batch_id={}",
                            quote.instrument().code(),
                            quote.price().get(),
                            quote.status(),
                            quote.source_at(),
                            quote.observed_at(),
                            quote.provider(),
                            quote.batch_id()
                        );
                    }
                    require_count(&mut errors, "beijing_quotes", batch.records().len(), 1);
                }
                Err(error) => record_error(&mut errors, "beijing_quotes", error),
            }
            match client.get_security_bars(4, 1, "600396", 0, 5, 0) {
                Ok(bars) => {
                    println!(
                        "bars={} first_datetime={}",
                        bars.len(),
                        bars.first().map_or("none", |b| b.datetime.as_str())
                    );
                    require_count(&mut errors, "bars", bars.len(), 5);
                }
                Err(error) => record_error(&mut errors, "bars", error),
            }
            match client.get_security_bars(KLINE_DAILY, 2, "920118", 0, 5, 0) {
                Ok(bars) => {
                    println!(
                        "beijing_bars={} first_datetime={}",
                        bars.len(),
                        bars.first().map_or("none", |bar| bar.datetime.as_str())
                    );
                    require_nonempty(&mut errors, "beijing_bars", bars.len());
                }
                Err(error) => record_error(&mut errors, "beijing_bars", error),
            }
            for (label, instrument) in [
                (
                    "normalized_shanghai_daily",
                    InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity)
                        .expect("valid Shanghai probe instrument"),
                ),
                (
                    "normalized_shenzhen_daily",
                    InstrumentId::new(Exchange::Shenzhen, "000001", AssetClass::Equity)
                        .expect("valid Shenzhen probe instrument"),
                ),
                (
                    "normalized_beijing_daily",
                    InstrumentId::new(Exchange::Beijing, "920118", AssetClass::Equity)
                        .expect("valid Beijing probe instrument"),
                ),
            ] {
                probe_normalized_bars(&mut errors, &client, label, instrument, BarInterval::Day);
            }
            probe_normalized_bars(
                &mut errors,
                &client,
                "normalized_shanghai_five_minute",
                InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity)
                    .expect("valid Shanghai probe instrument"),
                BarInterval::Minute5,
            );
            let inner = client.inner();
            println!("full_probe_server={:?}", inner.connected_server());
            if std::env::var_os("TDX_LIFECYCLE_RAW_ONLY").is_some() {
                match inner.get_xdxr_info(1, "600519") {
                    Ok(items) => {
                        let mut histogram = std::collections::BTreeMap::new();
                        for item in &items {
                            *histogram.entry(item.category).or_insert(0usize) += 1;
                            println!(
                                "xdxr_600519_raw date={:04}-{:02}-{:02} category={} name={} fenhong={:?} peigujia={:?} songzhuangu={:?} peigu={:?} suogu={:?} panqianliutong={:?} panhouliutong={:?} qianzongguben={:?} houzongguben={:?} fenshu={:?} xingquanjia={:?}",
                                item.year,
                                item.month,
                                item.day,
                                item.category,
                                item.name,
                                item.fenhong,
                                item.peigujia,
                                item.songzhuangu,
                                item.peigu,
                                item.suogu,
                                item.panqianliutong,
                                item.panhouliutong,
                                item.qianzongguben,
                                item.houzongguben,
                                item.fenshu,
                                item.xingquanjia,
                            );
                        }
                        println!(
                            "xdxr_600519_raw_count={} category_histogram={histogram:?}",
                            items.len()
                        );
                        return;
                    }
                    Err(error) => {
                        eprintln!("xdxr_600519_raw=error error={error}");
                        std::process::exit(1);
                    }
                }
            }
            for category in 0_u8..=11 {
                match inner.get_security_bars(category, 1, "600396", 0, 1, 0) {
                    Ok(items) => {
                        println!("stock_kline category={category} count={}", items.len());
                        require_count(
                            &mut errors,
                            &format!("stock_kline_category_{category}"),
                            items.len(),
                            1,
                        );
                    }
                    Err(error) => record_error(
                        &mut errors,
                        &format!("stock_kline_category_{category}"),
                        error,
                    ),
                }
            }
            match probe_index_bars(1, "000001", 6, 5) {
                Ok((count, first_datetime, server)) => {
                    println!(
                        "index_kline count={count} first_datetime={first_datetime} server={server}"
                    );
                    require_nonempty(&mut errors, "index_kline", count);
                }
                Err(error) => record_error(&mut errors, "index_kline", error),
            }
            match inner.get_security_count(1) {
                Ok(value) => {
                    println!("security_count_sh={value}");
                    require_nonempty(&mut errors, "security_count", usize::from(value));
                }
                Err(error) => record_error(&mut errors, "security_count", error),
            }
            match inner.get_security_list(1, 0) {
                Ok(items) => {
                    println!(
                        "security_list_sh={} first_code={}",
                        items.len(),
                        items.first().map_or("none", |v| v.code.as_str())
                    );
                    require_nonempty(&mut errors, "security_list", items.len());
                }
                Err(error) => record_error(&mut errors, "security_list", error),
            }
            match inner.get_security_count(2) {
                Ok(value) => {
                    println!("security_count_bj={value}");
                    require_nonempty(&mut errors, "security_count_bj", usize::from(value));
                }
                Err(error) => record_error(&mut errors, "security_count_bj", error),
            }
            let metadata_instruments = [
                InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity)
                    .expect("valid Shanghai metadata instrument"),
                InstrumentId::new(Exchange::Shenzhen, "000001", AssetClass::Equity)
                    .expect("valid Shenzhen metadata instrument"),
                InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity)
                    .expect("valid known-listing metadata instrument"),
            ];
            match client.security_metadata(&metadata_instruments) {
                Ok(batch) => {
                    println!(
                        "security_metadata={} provenance={:?} quality={:?}",
                        batch.records().len(),
                        batch.provenance(),
                        batch.quality()
                    );
                    for record in batch.records() {
                        println!(
                            "security code={} exchange={:?} name={:?} board={:?} is_st={:?} listed_on={:?} price_limit_percent={:?} price_limit_version={:?} status={:?} source_at={:?} observed_at={} provider={:?} batch_id={}",
                            record.instrument().code(),
                            record.instrument().exchange(),
                            record.name(),
                            record.board(),
                            record.is_st(),
                            record.listed_on(),
                            record.price_limit().percent(),
                            record.price_limit().version(),
                            record.status(),
                            record.source_at(),
                            record.observed_at(),
                            record.provider(),
                            record.batch_id()
                        );
                    }
                    require_count(&mut errors, "security_metadata", batch.records().len(), 3);
                    if batch
                        .records()
                        .iter()
                        .find(|record| record.instrument().code() == "600519")
                        .and_then(|record| record.listed_on())
                        != Some("2001-08-27")
                    {
                        errors.push(
                            "security_metadata: 600519 listing date did not equal 2001-08-27"
                                .into(),
                        );
                    }
                }
                Err(error) => record_error(&mut errors, "security_metadata", error),
            }
            let action_instrument =
                InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity)
                    .expect("valid corporate-action instrument");
            let action_request = CorporateActionRequest::new(action_instrument.clone())
                .with_range(
                    IsoDate::new("2024-01-01").expect("valid action range start"),
                    IsoDate::new("2024-12-31").expect("valid action range end"),
                )
                .expect("ordered action range");
            match CorporateActions::corporate_actions(&client, &action_request) {
                Ok(response) => {
                    if response.coverage() != &action_request {
                        errors.push(
                            "corporate_actions: response coverage did not echo the exact request"
                                .into(),
                        );
                    }
                    let batch = response.batch();
                    println!(
                        "corporate_actions={} provenance={:?} quality={:?}",
                        batch.records().len(),
                        batch.provenance(),
                        batch.quality()
                    );
                    require_count(&mut errors, "corporate_actions", batch.records().len(), 2);
                    if batch.records().iter().any(|record| {
                        record.instrument() != &action_instrument
                            || record.status() != CorporateActionStatus::Implemented
                            || record.category() != CorporateActionCategory::Distribution
                            || record.evidence().provider() != ProviderId::Tdx
                            || record.evidence().batch_id()
                                != batch.provenance().batch_id().unwrap_or_default()
                            || record.evidence().observed_at() != batch.provenance().fetched_at()
                            || record.evidence().source_at().is_some()
                    }) {
                        errors.push(
                            "corporate_actions: record identity/status/evidence mismatch".into(),
                        );
                    }
                    const TERM_TOLERANCE: f64 = 1e-9;
                    let expected = [
                        ("2024-06-19", 30.8760009765625),
                        ("2024-12-20", 23.882000732421876),
                    ];
                    for (record, (expected_date, expected_cash)) in
                        batch.records().iter().zip(expected)
                    {
                        if record.effective_on().as_str() != expected_date {
                            errors.push(format!(
                                "corporate_actions: expected date {expected_date}, received {}",
                                record.effective_on()
                            ));
                        }
                        match record.terms() {
                            CorporateActionTerms::Distribution {
                                cash_per_share,
                                bonus_per_share,
                                rights_per_share,
                                rights_price,
                            } => {
                                let cash = cash_per_share.as_ref().map(|value| value.get());
                                let bonus = bonus_per_share.as_ref().map(|value| value.get());
                                let rights = rights_per_share.as_ref().map(|value| value.get());
                                if !cash.is_some_and(|value| {
                                    within_tolerance(value, expected_cash, TERM_TOLERANCE)
                                }) || !bonus.is_some_and(|value| {
                                    within_tolerance(value, 0.0, TERM_TOLERANCE)
                                }) || !rights.is_some_and(|value| {
                                    within_tolerance(value, 0.0, TERM_TOLERANCE)
                                }) || rights_price.is_some()
                                {
                                    errors.push(format!(
                                        "corporate_actions: unexpected terms on {expected_date}: {:?}; tolerance={TERM_TOLERANCE}",
                                        record.terms()
                                    ));
                                }
                            }
                            terms => errors.push(format!(
                                "corporate_actions: expected distribution terms on {expected_date}, received {terms:?}"
                            )),
                        }
                    }
                    for record in batch.records() {
                        println!(
                            "corporate_action code={} effective_on={} category={:?} status={:?} terms={:?} source_at={:?} observed_at={} provider={:?} batch_id={}",
                            record.instrument().code(),
                            record.effective_on(),
                            record.category(),
                            record.status(),
                            record.terms(),
                            record.evidence().source_at(),
                            record.evidence().observed_at(),
                            record.evidence().provider(),
                            record.evidence().batch_id()
                        );
                    }
                }
                Err(error) => record_error(&mut errors, "corporate_actions", error),
            }
            let empty_request = CorporateActionRequest::new(action_instrument)
                .with_range(
                    IsoDate::new("1900-01-01").expect("valid empty range start"),
                    IsoDate::new("1900-12-31").expect("valid empty range end"),
                )
                .expect("ordered empty range");
            match CorporateActions::corporate_actions(&client, &empty_request) {
                Ok(response)
                    if response.coverage() == &empty_request
                        && response.batch().records().is_empty()
                        && response.batch().quality().is_complete() =>
                {
                    println!(
                        "corporate_actions_verified_empty=true code={} start={} end={} batch_id={}",
                        response.coverage().instrument().code(),
                        response
                            .coverage()
                            .start()
                            .map_or("all", IsoDate::as_str),
                        response.coverage().end().map_or("all", IsoDate::as_str),
                        response
                            .batch()
                            .provenance()
                            .batch_id()
                            .unwrap_or_default()
                    );
                }
                Ok(response) => errors.push(format!(
                    "corporate_actions_verified_empty: expected exact coverage and complete empty, coverage={:?} received={} quality={:?}",
                    response.coverage(),
                    response.batch().records().len(),
                    response.batch().quality()
                )),
                Err(error) => {
                    record_error(&mut errors, "corporate_actions_verified_empty", error);
                }
            }
            match client.security_metadata(std::slice::from_ref(&beijing_instrument)) {
                Err(TdxError::Unsupported(reason)) => {
                    println!("beijing_security_metadata=expected_unsupported reason={reason}");
                }
                Err(error) => record_error(&mut errors, "beijing_security_metadata", error),
                Ok(batch) => errors.push(format!(
                    "beijing_security_metadata: expected Unsupported, received {} records",
                    batch.records().len()
                )),
            }
            let book_instrument =
                InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity)
                    .expect("valid order-book instrument");
            match client.order_books(&[book_instrument, beijing_instrument.clone()]) {
                Ok(batch) => {
                    println!(
                        "order_books={} provenance={:?} quality={:?}",
                        batch.records().len(),
                        batch.provenance(),
                        batch.quality()
                    );
                    require_count(&mut errors, "order_books", batch.records().len(), 2);
                    for book in batch.records() {
                        println!(
                            "order_book code={} status={:?} total_bid={:?} total_ask={:?} source_at={:?} observed_at={} provider={:?} batch_id={}",
                            book.instrument().code(),
                            book.status(),
                            book.total_bid_quantity().map(|value| value.get()),
                            book.total_ask_quantity().map(|value| value.get()),
                            book.source_at(),
                            book.observed_at(),
                            book.provider(),
                            book.batch_id()
                        );
                        for (index, (bid, ask)) in book.bids().iter().zip(book.asks()).enumerate() {
                            println!(
                                "order_book_level={} bid_price={:?} bid_quantity={:?} ask_price={:?} ask_quantity={:?}",
                                index + 1,
                                bid.price().map(|value| value.get()),
                                bid.quantity().map(|value| value.get()),
                                ask.price().map(|value| value.get()),
                                ask.quantity().map(|value| value.get())
                            );
                        }
                    }
                }
                Err(error) => record_error(&mut errors, "order_books", error),
            }
            let mut verified_minute_date = None;
            match probe_minute_data(1, "600396") {
                Ok(probe) => {
                    println!(
                        "minute_data_current={} current_status={} minute_data_latest_session={} latest_session_date={} server={}",
                        probe.current_count,
                        probe.current_status,
                        probe.latest_session_count,
                        probe.latest_session_date,
                        probe.server
                    );
                    verified_minute_date = Some(probe.latest_session_date);
                    require_count(
                        &mut errors,
                        "minute_data_latest_session",
                        probe.latest_session_count,
                        240,
                    );
                }
                Err(error) => record_error(&mut errors, "minute_data", error),
            }
            if let Some(date) = verified_minute_date {
                match inner.get_history_minute_time_data(1, "600396", date) {
                    Ok(items) => {
                        println!("minute_history date={date} count={}", items.len());
                        require_count(&mut errors, "minute_history", items.len(), 240);
                    }
                    Err(error) => record_error(&mut errors, "minute_history", error),
                }
            }
            match probe_minute_data(2, "920118") {
                Ok(probe) => {
                    println!(
                        "beijing_minute_data_current={} current_status={} beijing_minute_data_latest_session={} latest_session_date={} server={}",
                        probe.current_count,
                        probe.current_status,
                        probe.latest_session_count,
                        probe.latest_session_date,
                        probe.server
                    );
                    require_count(
                        &mut errors,
                        "beijing_minute_data_latest_session",
                        probe.latest_session_count,
                        240,
                    );
                }
                Err(error) => record_error(&mut errors, "beijing_minute_data", error),
            }
            match client.minute_data(&MinuteDataRequest::new(beijing_instrument.clone())) {
                Ok(batch) => {
                    println!(
                        "beijing_normalized_minute={} provenance={:?} quality={:?}",
                        batch.records().len(),
                        batch.provenance(),
                        batch.quality()
                    );
                    if let Some(session) = session {
                        if session.is_active() {
                            require_nonempty(
                                &mut errors,
                                "beijing_normalized_minute",
                                batch.records().len(),
                            );
                        } else {
                            errors.push(format!(
                                "beijing_normalized_minute: normalized provider admitted {} records during {}",
                                batch.records().len(),
                                session.label()
                            ));
                        }
                    }
                    for point in batch.records() {
                        println!(
                            "beijing_minute at={} price={} cumulative_quantity={} cumulative_amount={:?} status={:?} source_at={:?} observed_at={} provider={:?} batch_id={}",
                            point.minute_at(),
                            point.price().get(),
                            point.cumulative_quantity().get(),
                            point.cumulative_amount().map(|value| value.get()),
                            point.status(),
                            point.source_at(),
                            point.observed_at(),
                            point.provider(),
                            point.batch_id()
                        );
                    }
                }
                Err(error) => {
                    if let Some(session) = session {
                        record_normalized_current_error(
                            &mut errors,
                            "beijing_normalized_minute",
                            error,
                            session,
                            "minute data",
                        );
                    } else {
                        record_error(&mut errors, "beijing_normalized_minute", error);
                    }
                }
            }
            match inner.get_transaction_data(1, "600396", 0, 20) {
                Ok(items) => {
                    println!("transactions={} ", items.len());
                    if let Some(session) = session {
                        let times = items
                            .iter()
                            .map(|item| item.time.as_str())
                            .collect::<Vec<_>>();
                        validate_raw_current_trades(
                            &mut errors,
                            "transactions",
                            &times,
                            20,
                            session,
                        );
                    }
                }
                Err(error) => {
                    if let Some(session) = session {
                        record_raw_current_diagnostic(&mut errors, "transactions", error, session);
                    }
                }
            }
            match inner.get_history_transaction_data(1, "600396", 0, 20, 20260721) {
                Ok(items) => {
                    println!("transactions_history date=20260721 count={}", items.len());
                    require_count(&mut errors, "transactions_history", items.len(), 20);
                }
                Err(error) => record_error(&mut errors, "transactions_history", error),
            }
            let instrument = InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity)
                .expect("valid probe instrument");
            let current_request =
                TradesRequest::new(instrument.clone(), 20).expect("valid current trade request");
            match client.trades(&current_request) {
                Ok(batch) => {
                    println!(
                        "normalized_trades_current={} provenance={:?} quality={:?}",
                        batch.records().len(),
                        batch.provenance(),
                        batch.quality()
                    );
                    for trade in batch.records() {
                        println!(
                            "trade current time={} price={} quantity={} count={:?} side={:?} status={:?} source_at={:?} observed_at={} provider={:?} batch_id={}",
                            trade.trade_at(),
                            trade.price().get(),
                            trade.quantity().get(),
                            trade.trade_count(),
                            trade.side(),
                            trade.status(),
                            trade.source_at(),
                            trade.observed_at(),
                            trade.provider(),
                            trade.batch_id()
                        );
                    }
                    if let Some(session) = session {
                        let times = batch
                            .records()
                            .iter()
                            .map(|trade| trade.trade_at())
                            .collect::<Vec<_>>();
                        validate_normalized_current_trades(
                            &mut errors,
                            "normalized_trades_current",
                            &times,
                            20,
                            session,
                        );
                    }
                }
                Err(error) => {
                    if let Some(session) = session {
                        record_normalized_current_error(
                            &mut errors,
                            "normalized_trades_current",
                            error,
                            session,
                            "trades",
                        );
                    }
                }
            }
            let beijing_current_request = TradesRequest::new(beijing_instrument.clone(), 20)
                .expect("valid Beijing current trade request");
            match client.trades(&beijing_current_request) {
                Ok(batch) => {
                    println!(
                        "beijing_normalized_trades_current={} provenance={:?} quality={:?}",
                        batch.records().len(),
                        batch.provenance(),
                        batch.quality()
                    );
                    for trade in batch.records() {
                        println!(
                            "beijing_trade current time={} price={} quantity={} count={:?} side={:?} status={:?} source_at={:?} observed_at={} provider={:?} batch_id={}",
                            trade.trade_at(),
                            trade.price().get(),
                            trade.quantity().get(),
                            trade.trade_count(),
                            trade.side(),
                            trade.status(),
                            trade.source_at(),
                            trade.observed_at(),
                            trade.provider(),
                            trade.batch_id()
                        );
                    }
                    if let Some(session) = session {
                        let times = batch
                            .records()
                            .iter()
                            .map(|trade| trade.trade_at())
                            .collect::<Vec<_>>();
                        validate_normalized_current_trades(
                            &mut errors,
                            "beijing_normalized_trades_current",
                            &times,
                            20,
                            session,
                        );
                    }
                }
                Err(error) => {
                    if let Some(session) = session {
                        record_normalized_current_error(
                            &mut errors,
                            "beijing_normalized_trades_current",
                            error,
                            session,
                            "trades",
                        );
                    }
                }
            }
            let historical_request = TradesRequest::new(instrument.clone(), 20)
                .and_then(|request| request.with_date("2026-07-21"))
                .expect("valid historical trade request");
            match client.trades(&historical_request) {
                Ok(batch) => {
                    println!(
                        "normalized_trades_history={} provenance={:?} quality={:?}",
                        batch.records().len(),
                        batch.provenance(),
                        batch.quality()
                    );
                    for trade in batch.records() {
                        println!(
                            "trade history time={} price={} quantity={} count={:?} side={:?} status={:?} source_at={:?} observed_at={} provider={:?} batch_id={}",
                            trade.trade_at(),
                            trade.price().get(),
                            trade.quantity().get(),
                            trade.trade_count(),
                            trade.side(),
                            trade.status(),
                            trade.source_at(),
                            trade.observed_at(),
                            trade.provider(),
                            trade.batch_id()
                        );
                    }
                    require_count(
                        &mut errors,
                        "normalized_trades_history",
                        batch.records().len(),
                        20,
                    );
                }
                Err(error) => record_error(&mut errors, "normalized_trades_history", error),
            }
            let current_paged =
                TradesRequest::new(instrument.clone(), 1_820).expect("valid paging probe");
            match client.trades(&current_paged) {
                Ok(batch) => {
                    println!(
                        "trade_pagination_current requested=1820 received={} crossed_page={} first_time={} last_time={} quality_complete={}",
                        batch.records().len(),
                        batch.records().len() > 1_800,
                        batch.records().first().map_or("none", |trade| trade.trade_at()),
                        batch.records().last().map_or("none", |trade| trade.trade_at()),
                        batch.quality().is_complete()
                    );
                    if let Some(session) = session {
                        let times = batch
                            .records()
                            .iter()
                            .map(|trade| trade.trade_at())
                            .collect::<Vec<_>>();
                        validate_normalized_current_trades(
                            &mut errors,
                            "trade_pagination_current",
                            &times,
                            1_820,
                            session,
                        );
                    }
                }
                Err(error) => {
                    if let Some(session) = session {
                        record_normalized_current_error(
                            &mut errors,
                            "trade_pagination_current",
                            error,
                            session,
                            "trades",
                        );
                    }
                }
            }
            let historical_paged = TradesRequest::new(instrument, 2_001)
                .and_then(|request| request.with_date("2026-07-21"))
                .expect("valid historical paging probe");
            match client.trades(&historical_paged) {
                Ok(batch) => {
                    println!(
                        "trade_pagination_history requested=2001 received={} crossed_page={} first_time={} last_time={} quality_complete={}",
                        batch.records().len(),
                        batch.records().len() > 2_000,
                        batch.records().first().map_or("none", |trade| trade.trade_at()),
                        batch.records().last().map_or("none", |trade| trade.trade_at()),
                        batch.quality().is_complete()
                    );
                    require_count(
                        &mut errors,
                        "trade_pagination_history",
                        batch.records().len(),
                        2_001,
                    );
                }
                Err(error) => record_error(&mut errors, "trade_pagination_history", error),
            }
            match inner.get_finance_info(1, "600396") {
                Ok(_) => println!("finance_info=ok"),
                Err(error) => record_error(&mut errors, "finance_info", error),
            }
            match inner.get_xdxr_info(1, "600396") {
                Ok(items) => {
                    println!("xdxr={} ", items.len());
                    require_nonempty(&mut errors, "xdxr", items.len());
                }
                Err(error) => record_error(&mut errors, "xdxr", error),
            }
            match client.get_security_quotes(&[(1, "510300")]) {
                Ok(items) => {
                    println!("fund_quotes_via_smart={} ", items.len());
                    require_count(&mut errors, "fund_quotes_via_smart", items.len(), 1);
                }
                Err(error) => record_error(&mut errors, "fund_quotes_via_smart", error),
            }
            let blocks = magic_tdx_rs::TdxBlockClient::with_default("180.153.18.170");
            match blocks.get_industry_blocks() {
                Ok(items) => {
                    println!("blocks_industry={}", items.len());
                    require_nonempty(&mut errors, "blocks_industry", items.len());
                }
                Err(error) => record_error(&mut errors, "blocks_industry", error),
            }
            match blocks.get_concept_blocks() {
                Ok(items) => {
                    println!("blocks_concept={}", items.len());
                    require_nonempty(&mut errors, "blocks_concept", items.len());
                }
                Err(error) => record_error(&mut errors, "blocks_concept", error),
            }
            let concept_service =
                magic_tdx_rs::service::blocks::BlockService::with_default("180.153.18.170");
            let concept_request = [
                InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity)
                    .expect("valid concept probe instrument"),
                InstrumentId::new(Exchange::Shenzhen, "000001", AssetClass::Equity)
                    .expect("valid concept probe instrument"),
            ];
            match concept_service.concept_hits(&concept_request) {
                Ok(batch) => {
                    println!(
                        "concept_hits={} batch_id={:?} quality_complete={}",
                        batch.records().len(),
                        batch.provenance().batch_id(),
                        batch.quality().is_complete()
                    );
                    require_nonempty(&mut errors, "concept_hits", batch.records().len());
                    for record in batch.records() {
                        println!(
                            "concept_hit code={} concept={} version={:?} provider={:?} batch_id={}",
                            record.instrument.code(),
                            record.concept,
                            record.detail,
                            record.evidence.provider(),
                            record.evidence.batch_id()
                        );
                    }
                }
                Err(error) => record_error(&mut errors, "concept_hits", error),
            }
            match blocks.get_index_blocks() {
                Ok(items) => {
                    println!("blocks_index={}", items.len());
                    require_nonempty(&mut errors, "blocks_index", items.len());
                }
                Err(error) => record_error(&mut errors, "blocks_index", error),
            }
            let funds = magic_tdx_rs::TdxHqFundClient::new();
            match funds.connect_to_any(Some(3.0)) {
                Ok(true) => {
                    match funds.get_fund_quotes(&[(1, "510300")]) {
                        Ok(items) => {
                            println!("fund_quotes={} ", items.len());
                            require_count(&mut errors, "fund_quotes", items.len(), 1);
                        }
                        Err(error) => record_error(&mut errors, "fund_quotes", error),
                    }
                    match funds.get_fund_bars(4, 1, "510300", 0, 5) {
                        Ok(items) => {
                            println!("fund_bars={} ", items.len());
                            require_count(&mut errors, "fund_bars", items.len(), 5);
                        }
                        Err(error) => record_error(&mut errors, "fund_bars", error),
                    }
                    match funds.get_fund_xdxr_info(1, "510300") {
                        Ok(items) => {
                            println!("fund_xdxr={} ", items.len());
                            require_nonempty(&mut errors, "fund_xdxr", items.len());
                        }
                        Err(error) => record_error(&mut errors, "fund_xdxr", error),
                    }
                }
                Ok(false) => record_error(&mut errors, "fund_connect", "returned false"),
                Err(error) => record_error(&mut errors, "fund_connect", error),
            }
            let f10 =
                magic_tdx_rs::net::f10_client::TdxF10Client::new("180.153.18.170", 7709, Some(3.0));
            match f10.get_category_auto("600396") {
                Ok(items) => {
                    println!("f10_categories={} ", items.len());
                    require_nonempty(&mut errors, "f10_categories", items.len());
                }
                Err(error) => record_error(&mut errors, "f10_categories", error),
            }
            let finance = magic_tdx_rs::TdxFinanceClient::new("180.153.18.170", 7709, Some(3.0));
            match finance.get_financial_list() {
                Ok(files) => {
                    println!("financial_files={}", files.len());
                    require_nonempty(&mut errors, "financial_files", files.len());
                    for file in files.iter().take(3) {
                        println!(
                            "financial_file name={} size={} hash={}",
                            file.filename, file.filesize, file.hash
                        );
                    }
                    if let Some(file) = files.iter().find(|file| file.filesize >= 20_000) {
                        match finance.get_financial_data(&file.filename, file.filesize) {
                            Ok(records) => {
                                println!(
                                    "financial_records file={} count={}",
                                    file.filename,
                                    records.len()
                                );
                                require_nonempty(&mut errors, "financial_records", records.len());
                                match records.iter().find(|record| record.code == "600396") {
                                    Some(record) => {
                                        let indicators = magic_tdx_rs::protocol::finance_fields::extract_indicators(
                                            &record.fields
                                        );
                                        println!("finance_indicators={}", indicators.len());
                                        require_count(
                                            &mut errors,
                                            "finance_indicators",
                                            indicators.len(),
                                            45,
                                        );
                                    }
                                    None => record_error(
                                        &mut errors,
                                        "finance_indicators",
                                        "600396 missing",
                                    ),
                                }
                            }
                            Err(error) => record_error(&mut errors, "finance_indicators", error),
                        }
                    } else {
                        record_error(
                            &mut errors,
                            "financial_records",
                            "no financial file met the minimum validated size",
                        );
                    }
                }
                Err(error) => record_error(&mut errors, "financial_files", error),
            }
        }
        Ok(false) => record_error(&mut errors, "connected", "returned false"),
        Err(error) => record_error(&mut errors, "connected", error),
    }
    if errors.is_empty() {
        println!("live_probe_status=passed");
    } else {
        eprintln!("live_probe_status=failed failures={}", errors.join(" | "));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_current_minute, completed_session_date, record_normalized_current_error,
        session_date_is_complete, validate_raw_current_trades, CurrentSession,
    };
    use magic_tdx_rs::TdxError;

    #[test]
    fn current_minute_is_classified_independently_from_history() {
        assert_eq!(
            classify_current_minute(20260722, 20260723, 4, 30 * 60, 0).unwrap(),
            "diagnostic_unadmitted_before_open"
        );
        assert_eq!(
            classify_current_minute(20260722, 20260723, 4, 9 * 3600, 1).unwrap(),
            "diagnostic_unadmitted_before_open"
        );
        assert_eq!(
            classify_current_minute(20260722, 20260723, 4, 9 * 3600, 240).unwrap(),
            "diagnostic_unadmitted_before_open"
        );
        assert!(classify_current_minute(20260722, 20260723, 4, 10 * 3600, 0).is_err());
        assert_eq!(
            classify_current_minute(20260723, 20260723, 4, 12 * 3600, 120).unwrap(),
            "diagnostic_unadmitted_midday"
        );
        assert_eq!(
            classify_current_minute(20260723, 20260723, 4, 15 * 3600 + 1, 240).unwrap(),
            "diagnostic_unadmitted_after_close"
        );
        assert_eq!(
            classify_current_minute(20260725, 20260725, 6, 10 * 3600, 0).unwrap(),
            "diagnostic_unadmitted_weekend"
        );
        assert_eq!(
            classify_current_minute(20260725, 20260725, 6, 10 * 3600, 1).unwrap(),
            "diagnostic_unadmitted_weekend"
        );
        let dates = [20260722, 20260723];
        assert_eq!(
            completed_session_date(&dates, 20260723, CurrentSession::PreOpen),
            Some(20260722)
        );
        assert_eq!(
            completed_session_date(&dates, 20260723, CurrentSession::Complete),
            Some(20260723)
        );
        assert!(!session_date_is_complete(
            20260723,
            20260723,
            CurrentSession::Intraday
        ));
    }

    #[test]
    fn off_session_raw_packets_are_diagnostic_but_normalized_errors_are_exact() {
        let mut errors = Vec::new();
        validate_raw_current_trades(
            &mut errors,
            "trades",
            &["09:15", "09:29"],
            20,
            CurrentSession::PreOpen,
        );
        assert!(errors.is_empty());
        validate_raw_current_trades(
            &mut errors,
            "trades",
            &["09:15", "15:00"],
            20,
            CurrentSession::PreOpen,
        );
        assert!(errors.is_empty());

        let mut errors = Vec::new();
        record_normalized_current_error(
            &mut errors,
            "trades",
            TdxError::InvalidData("TDX returned an empty successful response".to_string()),
            CurrentSession::PreOpen,
            "trades",
        );
        assert_eq!(errors.len(), 1);

        let mut errors = Vec::new();
        record_normalized_current_error(
            &mut errors,
            "trades",
            TdxError::InvalidData(
                "TDX normalized current trades is unavailable outside an active A-share weekday session"
                    .to_string(),
            ),
            CurrentSession::PreOpen,
            "trades",
        );
        assert!(errors.is_empty());
        record_normalized_current_error(
            &mut errors,
            "trades",
            TdxError::Connection("network down".to_string()),
            CurrentSession::PreOpen,
            "trades",
        );
        assert_eq!(errors.len(), 1);
    }
}
