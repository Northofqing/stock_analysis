use crate::error::TdxError;
use crate::protocol::constants::{
    KLINE_15MIN, KLINE_1HOUR, KLINE_1MIN, KLINE_30MIN, KLINE_5MIN, KLINE_DAILY, KLINE_MONTHLY,
    KLINE_WEEKLY, KLINE_YEARLY, MAX_KLINE_COUNT,
};
use crate::protocol::types::{FinanceInfo, MinuteTimePrice, SecurityInfo, TickData, XdXrInfo};
use crate::{SecurityBar, SecurityQuote, TdxHqClient};
use magic_market_core::{
    Adjustment, AsyncHistoricalBars, AsyncRealtimeQuotes, AsyncTrades, AuctionSnapshot, Auctions,
    Bar, BarInterval, BarsRequest, Board, BookLevel, CorporateAction, CorporateActionCategory,
    CorporateActionRequest, CorporateActionResponse, CorporateActionStatus, CorporateActionTerms,
    CorporateActions, DataBatch, DataStatus, FiniteNumber, HistoricalBars, InstrumentId, IsoDate,
    MinuteData, MinuteDataRequest, MinutePoint, Money, MoneyFlow, MoneyFlows, OrderBook,
    OrderBooks, Price, PriceLimitRule, ProviderId, Quantity, Quote, Ratio, RatioUnit,
    RealtimeQuotes, SecurityMetadata, SecurityMetadataProvider, SourceEvidence, Trade, TradeSide,
    Trades, TradesRequest, UnverifiedSourceUnit,
};
use std::collections::{HashMap, HashSet};

const CURRENT_TRADE_PAGE_SIZE: u16 = 1_800;
const HISTORICAL_TRADE_PAGE_SIZE: u16 = 2_000;
const SHARES_PER_LOT: f64 = 100.0;

impl TdxHqClient {
    /// Returns the data families exposed through the core provider boundary.
    pub const fn capabilities() -> magic_market_core::Capabilities {
        magic_market_core::Capabilities {
            quotes: true,
            bars: true,
            minute: true,
            trades: true,
            fundamentals: true,
            corporate_actions: true,
            blocks: true,
            money_flow: false,
            order_book: true,
            auction: false,
            security_metadata: true,
        }
    }
}

pub(crate) fn market(id: &InstrumentId) -> Result<u8, TdxError> {
    match id.exchange() {
        magic_market_core::Exchange::Shanghai => Ok(1),
        magic_market_core::Exchange::Shenzhen => Ok(0),
        // Live protocol evidence on 2026-07-23 uniquely returned market/code
        // `(2, 920118)` for market 2. Candidates 0 and 1 returned a mismatched
        // Shanghai record and are therefore never accepted for Beijing.
        magic_market_core::Exchange::Beijing => Ok(2),
    }
}
fn category(interval: BarInterval) -> Result<u8, TdxError> {
    match interval {
        BarInterval::Minute1 => Ok(KLINE_1MIN),
        BarInterval::Minute5 => Ok(KLINE_5MIN),
        BarInterval::Minute15 => Ok(KLINE_15MIN),
        BarInterval::Minute30 => Ok(KLINE_30MIN),
        BarInterval::Hour1 => Ok(KLINE_1HOUR),
        BarInterval::Day => Ok(KLINE_DAILY),
        BarInterval::Week => Ok(KLINE_WEEKLY),
        BarInterval::Month => Ok(KLINE_MONTHLY),
        BarInterval::Year => Ok(KLINE_YEARLY),
    }
}
fn reject_unsupported_bar_range(request: &BarsRequest) -> Result<(), TdxError> {
    if request.start().is_some() || request.end().is_some() {
        Err(TdxError::Unsupported(
            "TDX historical bars do not support normalized date ranges; omit start/end".into(),
        ))
    } else {
        Ok(())
    }
}
fn ensure_nonempty<T>(records: &[T]) -> Result<(), TdxError> {
    if records.is_empty() {
        Err(TdxError::InvalidData(
            "TDX returned an empty successful response".into(),
        ))
    } else {
        Ok(())
    }
}
fn fetched_at() -> Result<String, TdxError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .map_err(|error| {
            TdxError::InvalidData(format!("system clock is before UNIX epoch: {error}"))
        })
}
fn ensure_current_session_at(unix_seconds: u64, family: &str) -> Result<(), TdxError> {
    const SECONDS_PER_DAY: u64 = 86_400;
    const CHINA_OFFSET_SECONDS: u64 = 8 * 60 * 60;

    let local_seconds = unix_seconds
        .checked_add(CHINA_OFFSET_SECONDS)
        .ok_or_else(|| TdxError::InvalidData("TDX current-session clock overflow".into()))?;
    let local_days = local_seconds / SECONDS_PER_DAY;
    let local_day_seconds = local_seconds % SECONDS_PER_DAY;
    // 1970-01-01 was Thursday. Sunday=0 and Saturday=6.
    let weekday = (local_days + 4) % 7;
    let is_weekday = !matches!(weekday, 0 | 6);
    let morning = (9 * 3_600 + 30 * 60..=11 * 3_600 + 30 * 60).contains(&local_day_seconds);
    let afternoon = (13 * 3_600..=15 * 3_600).contains(&local_day_seconds);

    if is_weekday && (morning || afternoon) {
        Ok(())
    } else {
        Err(TdxError::InvalidData(format!(
            "TDX normalized current {family} is unavailable outside an active A-share weekday session"
        )))
    }
}

fn ensure_current_session(family: &str) -> Result<(), TdxError> {
    let unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            TdxError::InvalidData(format!("system clock is before UNIX epoch: {error}"))
        })?
        .as_secs();
    ensure_current_session_at(unix_seconds, family)
}
fn normalized_bar_time(
    interval: BarInterval,
    record: &SecurityBar,
) -> Result<(String, String, i64), TdxError> {
    let date = format!("{:04}-{:02}-{:02}", record.year, record.month, record.day);
    IsoDate::new(date.clone()).map_err(|error| {
        TdxError::InvalidData(format!(
            "TDX bar has invalid calendar components for {}: {error}",
            record.datetime
        ))
    })?;
    let intraday = matches!(
        interval,
        BarInterval::Minute1
            | BarInterval::Minute5
            | BarInterval::Minute15
            | BarInterval::Minute30
            | BarInterval::Hour1
    );
    let (expected_source_at, bar_time) = if intraday {
        if record.hour > 23 || record.minute > 59 {
            return Err(TdxError::InvalidData(format!(
                "TDX bar has invalid intraday components for {}",
                record.datetime
            )));
        }
        let source_at = format!("{date} {:02}:{:02}", record.hour, record.minute);
        let bar_time = format!("{source_at}:00");
        (source_at, bar_time)
    } else {
        if record.hour != 0 || record.minute != 0 {
            return Err(TdxError::InvalidData(format!(
                "TDX non-intraday bar has unexpected time components for {}",
                record.datetime
            )));
        }
        (date.clone(), date)
    };
    if record.datetime != expected_source_at {
        return Err(TdxError::InvalidData(format!(
            "TDX bar datetime {:?} contradicts decoded components {expected_source_at:?}",
            record.datetime
        )));
    }

    let adjusted_year = i64::from(record.year) - i64::from(record.month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let month = i64::from(record.month);
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(record.day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days_since_epoch = era * 146_097 + day_of_era - 719_468;
    let source_epoch = days_since_epoch
        .checked_mul(86_400)
        .and_then(|value| value.checked_add(i64::from(record.hour) * 3_600))
        .and_then(|value| value.checked_add(i64::from(record.minute) * 60))
        // TDX A-share timestamps are Asia/Shanghai source time.
        .and_then(|value| value.checked_sub(8 * 3_600))
        .ok_or_else(|| TdxError::InvalidData("TDX bar timestamp overflow".into()))?;
    Ok((expected_source_at, bar_time, source_epoch))
}

pub(crate) fn normalize_bars(
    source: &str,
    request: &BarsRequest,
    records: Vec<SecurityBar>,
) -> Result<DataBatch<Bar>, TdxError> {
    if records.len() != usize::from(request.limit()) {
        return Err(TdxError::HistoricalBarCardinality {
            offset: 0,
            actual: records.len(),
            expected_page: request.limit(),
            requested_total: request.limit(),
        });
    }
    ensure_nonempty(&records)?;
    if !matches!(source, "tdx" | "tdx-smart" | "tdx-direct" | "tdx-async") {
        return Err(TdxError::InvalidData(format!(
            "unexpected TDX bar source {source:?}"
        )));
    }

    let observed_at = fetched_at()?;
    let observed_epoch = observed_at.parse::<i64>().map_err(|error| {
        TdxError::InvalidData(format!("invalid TDX observation timestamp: {error}"))
    })?;
    let mut times = Vec::with_capacity(records.len());
    let mut previous_source_at: Option<String> = None;
    for record in &records {
        let (source_at, bar_time, source_epoch) = normalized_bar_time(request.interval(), record)?;
        if source_epoch > observed_epoch {
            return Err(TdxError::InvalidData(format!(
                "TDX bar source time {source_at} is newer than observation {observed_at}"
            )));
        }
        if previous_source_at
            .as_deref()
            .is_some_and(|previous| previous >= source_at.as_str())
        {
            return Err(TdxError::InvalidData(format!(
                "TDX bar times are duplicate or non-increasing at {source_at}"
            )));
        }
        if !record.amount.is_finite() || record.amount < 0.0 {
            return Err(TdxError::InvalidData(format!(
                "TDX bar amount is invalid at {source_at}"
            )));
        }
        if record.vol > 0.0 && record.amount == 0.0 {
            return Err(TdxError::InvalidData(format!(
                "TDX bar has positive volume with zero amount at {source_at}"
            )));
        }
        previous_source_at = Some(source_at.clone());
        times.push((source_at, bar_time));
    }
    let latest_source_at = times
        .last()
        .map(|(source_at, _)| source_at.clone())
        .ok_or_else(|| TdxError::InvalidData("TDX bar batch has no source time".into()))?;
    let provenance = magic_market_core::Provenance::new(source, observed_at)?
        .with_source_at(latest_source_at)?;
    let batch_id = provenance
        .batch_id()
        .ok_or_else(|| TdxError::InvalidData("TDX bar batch has no batch ID".into()))?
        .to_owned();
    let mut normalized = Vec::with_capacity(records.len());
    for (record, (source_at, bar_time)) in records.into_iter().zip(times) {
        let bar = Bar::new(
            request.instrument().clone(),
            request.interval(),
            bar_time.clone(),
            bar_time,
            Price::new(record.open)?,
            Price::new(record.high)?,
            Price::new(record.low)?,
            Price::new(record.close)?,
            Quantity::new(record.vol / SHARES_PER_LOT)?,
            Some(Money::new(record.amount)?),
            Adjustment::Unadjusted,
            ProviderId::Tdx,
            batch_id.clone(),
        )?
        .with_source_at(source_at)?;
        normalized.push(bar);
    }
    Ok(DataBatch::strict(normalized, provenance))
}

pub(crate) trait BlockingTdxQuery {
    fn security_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError>;

    fn security_quotes(&self, instruments: &[(u8, &str)]) -> Result<Vec<SecurityQuote>, TdxError>;

    fn minute_time_data(&self, market: u8, code: &str) -> Result<Vec<MinuteTimePrice>, TdxError>;

    fn history_minute_time_data(
        &self,
        market: u8,
        code: &str,
        date: u32,
    ) -> Result<Vec<MinuteTimePrice>, TdxError>;

    fn transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<TickData>, TdxError>;

    fn history_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
        date: u32,
    ) -> Result<Vec<TickData>, TdxError>;

    fn security_count(&self, market: u8) -> Result<u16, TdxError>;

    fn security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>, TdxError>;

    fn finance_info(&self, _market: u8, _code: &str) -> Result<FinanceInfo, TdxError> {
        Err(TdxError::Unsupported(
            "blocking query does not expose finance metadata".into(),
        ))
    }

    fn xdxr_info(&self, _market: u8, _code: &str) -> Result<Vec<XdXrInfo>, TdxError> {
        Err(TdxError::Unsupported(
            "blocking query does not expose XDXR metadata".into(),
        ))
    }
}

impl BlockingTdxQuery for TdxHqClient {
    fn security_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError> {
        TdxHqClient::get_security_bars(self, category, market, code, start, count, adjust)
    }

    fn security_quotes(&self, instruments: &[(u8, &str)]) -> Result<Vec<SecurityQuote>, TdxError> {
        TdxHqClient::get_security_quotes(self, instruments)
    }

    fn minute_time_data(&self, market: u8, code: &str) -> Result<Vec<MinuteTimePrice>, TdxError> {
        TdxHqClient::get_minute_time_data(self, market, code)
    }

    fn history_minute_time_data(
        &self,
        market: u8,
        code: &str,
        date: u32,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        TdxHqClient::get_history_minute_time_data(self, market, code, date)
    }

    fn transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<TickData>, TdxError> {
        TdxHqClient::get_transaction_data(self, market, code, start, count)
    }

    fn history_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
        date: u32,
    ) -> Result<Vec<TickData>, TdxError> {
        TdxHqClient::get_history_transaction_data(self, market, code, start, count, date)
    }

    fn security_count(&self, market: u8) -> Result<u16, TdxError> {
        TdxHqClient::get_security_count(self, market)
    }

    fn security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>, TdxError> {
        TdxHqClient::get_security_list(self, market, start)
    }

    fn finance_info(&self, market: u8, code: &str) -> Result<FinanceInfo, TdxError> {
        TdxHqClient::get_finance_info(self, market, code)
    }

    fn xdxr_info(&self, market: u8, code: &str) -> Result<Vec<XdXrInfo>, TdxError> {
        TdxHqClient::get_xdxr_info(self, market, code)
    }
}

impl BlockingTdxQuery for crate::TdxSmartClient {
    fn security_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError> {
        crate::TdxSmartClient::get_security_bars(self, category, market, code, start, count, adjust)
    }

    fn security_quotes(&self, instruments: &[(u8, &str)]) -> Result<Vec<SecurityQuote>, TdxError> {
        crate::TdxSmartClient::get_security_quotes(self, instruments)
    }

    fn minute_time_data(&self, market: u8, code: &str) -> Result<Vec<MinuteTimePrice>, TdxError> {
        TdxHqClient::get_minute_time_data(self.inner(), market, code)
    }

    fn history_minute_time_data(
        &self,
        market: u8,
        code: &str,
        date: u32,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        TdxHqClient::get_history_minute_time_data(self.inner(), market, code, date)
    }

    fn transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<TickData>, TdxError> {
        TdxHqClient::get_transaction_data(self.inner(), market, code, start, count)
    }

    fn history_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
        date: u32,
    ) -> Result<Vec<TickData>, TdxError> {
        TdxHqClient::get_history_transaction_data(self.inner(), market, code, start, count, date)
    }

    fn security_count(&self, market: u8) -> Result<u16, TdxError> {
        TdxHqClient::get_security_count(self.inner(), market)
    }

    fn security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>, TdxError> {
        TdxHqClient::get_security_list(self.inner(), market, start)
    }

    fn finance_info(&self, market: u8, code: &str) -> Result<FinanceInfo, TdxError> {
        TdxHqClient::get_finance_info(self.inner(), market, code)
    }

    fn xdxr_info(&self, market: u8, code: &str) -> Result<Vec<XdXrInfo>, TdxError> {
        TdxHqClient::get_xdxr_info(self.inner(), market, code)
    }
}

impl BlockingTdxQuery for crate::TdxDirectClient {
    fn security_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError> {
        crate::TdxDirectClient::get_security_bars(
            self, category, market, code, start, count, adjust,
        )
    }

    fn security_quotes(&self, instruments: &[(u8, &str)]) -> Result<Vec<SecurityQuote>, TdxError> {
        crate::TdxDirectClient::get_security_quotes(self, instruments)
    }

    fn minute_time_data(&self, market: u8, code: &str) -> Result<Vec<MinuteTimePrice>, TdxError> {
        crate::TdxDirectClient::get_minute_time_data(self, market, code)
    }

    fn history_minute_time_data(
        &self,
        market: u8,
        code: &str,
        date: u32,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        crate::TdxDirectClient::get_history_minute_time_data(self, market, code, date)
    }

    fn transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<TickData>, TdxError> {
        crate::TdxDirectClient::get_transaction_data(self, market, code, start, count)
    }

    fn history_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
        date: u32,
    ) -> Result<Vec<TickData>, TdxError> {
        crate::TdxDirectClient::get_history_transaction_data(self, market, code, start, count, date)
    }

    fn security_count(&self, market: u8) -> Result<u16, TdxError> {
        crate::TdxDirectClient::get_security_count(self, market)
    }

    fn security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>, TdxError> {
        crate::TdxDirectClient::get_security_list(self, market, start)
    }

    fn finance_info(&self, market: u8, code: &str) -> Result<FinanceInfo, TdxError> {
        crate::TdxDirectClient::get_finance_info(self, market, code)
    }

    fn xdxr_info(&self, market: u8, code: &str) -> Result<Vec<XdXrInfo>, TdxError> {
        crate::TdxDirectClient::get_xdxr_info(self, market, code)
    }
}

pub(crate) trait AsyncTdxQuery {
    async fn security_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError>;

    async fn security_quotes(
        &self,
        instruments: &[(u8, &str)],
    ) -> Result<Vec<SecurityQuote>, TdxError>;

    async fn transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<TickData>, TdxError>;

    async fn history_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
        date: u32,
    ) -> Result<Vec<TickData>, TdxError>;

    async fn security_count(&self, market: u8) -> Result<u16, TdxError>;

    async fn security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>, TdxError>;

    async fn minute_time_data(
        &self,
        market: u8,
        code: &str,
    ) -> Result<Vec<MinuteTimePrice>, TdxError>;

    async fn history_minute_time_data(
        &self,
        market: u8,
        code: &str,
        date: u32,
    ) -> Result<Vec<MinuteTimePrice>, TdxError>;
}

impl AsyncTdxQuery for crate::AsyncTdxHqClient {
    async fn security_bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
        adjust: u8,
    ) -> Result<Vec<SecurityBar>, TdxError> {
        crate::AsyncTdxHqClient::get_security_bars(
            self, category, market, code, start, count, adjust,
        )
        .await
    }

    async fn security_quotes(
        &self,
        instruments: &[(u8, &str)],
    ) -> Result<Vec<SecurityQuote>, TdxError> {
        crate::AsyncTdxHqClient::get_security_quotes(self, instruments).await
    }

    async fn transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
    ) -> Result<Vec<TickData>, TdxError> {
        crate::AsyncTdxHqClient::get_transaction_data(self, market, code, start, count).await
    }

    async fn history_transaction_data(
        &self,
        market: u8,
        code: &str,
        start: u16,
        count: u16,
        date: u32,
    ) -> Result<Vec<TickData>, TdxError> {
        crate::AsyncTdxHqClient::get_history_transaction_data(
            self, market, code, start, count, date,
        )
        .await
    }

    async fn security_count(&self, market: u8) -> Result<u16, TdxError> {
        crate::AsyncTdxHqClient::get_security_count(self, market).await
    }

    async fn security_list(&self, market: u8, start: u16) -> Result<Vec<SecurityInfo>, TdxError> {
        crate::AsyncTdxHqClient::get_security_list(self, market, start).await
    }

    async fn minute_time_data(
        &self,
        market: u8,
        code: &str,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        crate::AsyncTdxHqClient::get_minute_time_data(self, market, code).await
    }

    async fn history_minute_time_data(
        &self,
        market: u8,
        code: &str,
        date: u32,
    ) -> Result<Vec<MinuteTimePrice>, TdxError> {
        crate::AsyncTdxHqClient::get_history_minute_time_data(self, market, code, date).await
    }
}

struct HistoricalBarPagination {
    expected: u16,
    remaining: u16,
    offset: u32,
    pages: Vec<Vec<SecurityBar>>,
}

impl HistoricalBarPagination {
    fn new(expected: u16) -> Self {
        Self {
            expected,
            remaining: expected,
            offset: 0,
            pages: Vec::new(),
        }
    }

    fn next_page(&self) -> Option<(u32, u16)> {
        (self.remaining > 0).then_some((self.offset, self.remaining.min(MAX_KLINE_COUNT)))
    }

    fn accept_page(&mut self, page: Vec<SecurityBar>) -> Result<(), TdxError> {
        let (offset, page_limit) = self.next_page().ok_or_else(|| {
            TdxError::InvalidData("TDX historical bar pagination received an extra page".into())
        })?;
        if page.len() != usize::from(page_limit) {
            return Err(TdxError::HistoricalBarCardinality {
                offset,
                actual: page.len(),
                expected_page: page_limit,
                requested_total: self.expected,
            });
        }
        self.pages.push(page);
        self.remaining -= page_limit;
        self.offset = self
            .offset
            .checked_add(u32::from(page_limit))
            .ok_or_else(|| TdxError::InvalidData("TDX historical bar offset overflow".into()))?;
        Ok(())
    }

    fn finish(self) -> Result<Vec<SecurityBar>, TdxError> {
        if self.remaining != 0 {
            return Err(TdxError::InvalidData(format!(
                "TDX historical bar pagination stopped with {} rows missing",
                self.remaining
            )));
        }
        let records: Vec<_> = self.pages.into_iter().rev().flatten().collect();
        if records.len() != usize::from(self.expected) {
            return Err(TdxError::InvalidData(format!(
                "TDX historical bar pagination assembled {} rows for exact request limit {}",
                records.len(),
                self.expected
            )));
        }
        Ok(records)
    }
}

fn historical_bars_with(
    query: &impl BlockingTdxQuery,
    source: &str,
    request: &BarsRequest,
) -> Result<DataBatch<Bar>, TdxError> {
    reject_unsupported_bar_range(request)?;
    let category = category(request.interval())?;
    let market = market(request.instrument())?;
    let mut pagination = HistoricalBarPagination::new(request.limit());
    while let Some((offset, page_limit)) = pagination.next_page() {
        let page = query.security_bars(
            category,
            market,
            request.instrument().code(),
            offset,
            page_limit,
            0,
        )?;
        pagination.accept_page(page)?;
    }
    let records = pagination.finish()?;
    normalize_bars(source, request, records)
}

async fn historical_bars_async_with(
    query: &impl AsyncTdxQuery,
    source: &str,
    request: &BarsRequest,
) -> Result<DataBatch<Bar>, TdxError> {
    reject_unsupported_bar_range(request)?;
    let category = category(request.interval())?;
    let market = market(request.instrument())?;
    let mut pagination = HistoricalBarPagination::new(request.limit());
    while let Some((offset, page_limit)) = pagination.next_page() {
        let page = query
            .security_bars(
                category,
                market,
                request.instrument().code(),
                offset,
                page_limit,
                0,
            )
            .await?;
        pagination.accept_page(page)?;
    }
    let records = pagination.finish()?;
    normalize_bars(source, request, records)
}

async fn realtime_quotes_async_with(
    query: &impl AsyncTdxQuery,
    source: &str,
    instruments: &[InstrumentId],
) -> Result<DataBatch<Quote>, TdxError> {
    let pairs: Vec<(u8, &str)> = instruments
        .iter()
        .map(|id| market(id).map(|market| (market, id.code())))
        .collect::<Result<_, _>>()?;
    let records = query.security_quotes(&pairs).await?;
    normalize_quotes(source, instruments, records)
}

async fn trades_async_with(
    query: &impl AsyncTdxQuery,
    request: &TradesRequest,
) -> Result<DataBatch<Trade>, TdxError> {
    trades_async_with_session(query, request, ensure_current_session).await
}

async fn trades_async_with_session(
    query: &impl AsyncTdxQuery,
    request: &TradesRequest,
    session_check: impl FnOnce(&str) -> Result<(), TdxError>,
) -> Result<DataBatch<Trade>, TdxError> {
    let historical_date = request.date().map(tdx_trade_date).transpose()?;
    if historical_date.is_none() {
        session_check("trades")?;
    }
    let request_market = market(request.instrument())?;
    let page_size = if historical_date.is_some() {
        HISTORICAL_TRADE_PAGE_SIZE
    } else {
        CURRENT_TRADE_PAGE_SIZE
    };
    let mut records = Vec::with_capacity(usize::from(request.limit()));
    let mut start = 0u16;
    let mut remaining = request.limit();
    while remaining != 0 {
        let requested = remaining.min(page_size);
        let page = match historical_date {
            Some(date) => {
                query
                    .history_transaction_data(
                        request_market,
                        request.instrument().code(),
                        start,
                        requested,
                        date,
                    )
                    .await?
            }
            None => {
                query
                    .transaction_data(
                        request_market,
                        request.instrument().code(),
                        start,
                        requested,
                    )
                    .await?
            }
        };
        if page.len() > usize::from(requested) {
            return Err(TdxError::InvalidData(
                "TDX async trade page exceeds requested cardinality".into(),
            ));
        }
        let fetched = u16::try_from(page.len())
            .map_err(|_| TdxError::InvalidData("TDX trade page is too large".into()))?;
        records.extend(page);
        if fetched < requested {
            break;
        }
        remaining -= fetched;
        if remaining == 0 {
            break;
        }
        start = start
            .checked_add(fetched)
            .ok_or_else(|| TdxError::InvalidData("TDX trade offset overflow".into()))?;
    }
    normalize_trade_records(
        if historical_date.is_some() {
            "tdx-async-history"
        } else {
            "tdx-async-current"
        },
        request,
        records,
    )
}

fn realtime_quotes_with(
    query: &impl BlockingTdxQuery,
    source: &str,
    instruments: &[InstrumentId],
) -> Result<DataBatch<Quote>, TdxError> {
    let pairs: Vec<(u8, &str)> = instruments
        .iter()
        .map(|id| market(id).map(|market| (market, id.code())))
        .collect::<Result<_, _>>()?;
    let records = query.security_quotes(&pairs)?;
    normalize_quotes(source, instruments, records)
}

fn minute_data_with(
    query: &impl BlockingTdxQuery,
    source: &str,
    request: &MinuteDataRequest,
) -> Result<DataBatch<MinutePoint>, TdxError> {
    minute_data_with_session(query, source, request, ensure_current_session)
}

fn minute_data_with_session(
    query: &impl BlockingTdxQuery,
    source: &str,
    request: &MinuteDataRequest,
    session_check: impl FnOnce(&str) -> Result<(), TdxError>,
) -> Result<DataBatch<MinutePoint>, TdxError> {
    let (date, records) = match request.date() {
        Some(date) => (
            date.to_owned(),
            query.history_minute_time_data(
                market(request.instrument())?,
                request.instrument().code(),
                compact_date(date)?,
            )?,
        ),
        None => {
            session_check("minute data")?;
            let compact = crate::net::utils::today_yyyymmdd();
            (
                display_date(compact)?,
                query
                    .minute_time_data(market(request.instrument())?, request.instrument().code())?,
            )
        }
    };
    normalize_minute_records(source, request.instrument(), &date, records)
}

fn trades_with(
    query: &impl BlockingTdxQuery,
    current_source: &str,
    history_source: &str,
    request: &TradesRequest,
) -> Result<DataBatch<Trade>, TdxError> {
    trades_with_session(
        query,
        current_source,
        history_source,
        request,
        ensure_current_session,
    )
}

fn trades_with_session(
    query: &impl BlockingTdxQuery,
    current_source: &str,
    history_source: &str,
    request: &TradesRequest,
    session_check: impl FnOnce(&str) -> Result<(), TdxError>,
) -> Result<DataBatch<Trade>, TdxError> {
    let request_market = market(request.instrument())?;
    match request.date() {
        Some(date) => {
            let date = tdx_trade_date(date)?;
            paginate_trades(
                history_source,
                request,
                HISTORICAL_TRADE_PAGE_SIZE,
                |start, count| {
                    query.history_transaction_data(
                        request_market,
                        request.instrument().code(),
                        start,
                        count,
                        date,
                    )
                },
            )
        }
        None => {
            session_check("trades")?;
            paginate_trades(
                current_source,
                request,
                CURRENT_TRADE_PAGE_SIZE,
                |start, count| {
                    query.transaction_data(
                        request_market,
                        request.instrument().code(),
                        start,
                        count,
                    )
                },
            )
        }
    }
}

fn security_metadata_with(
    query: &impl BlockingTdxQuery,
    source: &str,
    instruments: &[InstrumentId],
) -> Result<DataBatch<SecurityMetadata>, TdxError> {
    validate_security_metadata_request(instruments)?;
    let records = fetch_security_records(
        instruments,
        |market| query.security_count(market),
        |market, start| query.security_list(market, start),
    )?;
    let finance = instruments
        .iter()
        .map(|instrument| query.finance_info(market(instrument)?, instrument.code()))
        .collect::<Result<Vec<_>, _>>()?;
    normalize_security_metadata_with_finance(source, instruments, records, finance)
}

fn compact_date(value: &str) -> Result<u32, TdxError> {
    let digits = value.replace('-', "");
    if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TdxError::InvalidData(format!(
            "invalid TDX minute date {value:?}"
        )));
    }
    digits
        .parse()
        .map_err(|_| TdxError::InvalidData(format!("invalid TDX minute date {value:?}")))
}

fn display_date(value: u32) -> Result<String, TdxError> {
    let value = format!("{value:08}");
    if value.len() != 8 {
        return Err(TdxError::InvalidData(
            "TDX minute date must contain eight digits".into(),
        ));
    }
    Ok(format!(
        "{}-{}-{}",
        &value[0..4],
        &value[4..6],
        &value[6..8]
    ))
}

fn valid_tdx_minute(value: &str) -> bool {
    value.len() == 5
        && value.as_bytes()[2] == b':'
        && (("09:31"..="11:30").contains(&value) || ("13:01"..="15:00").contains(&value))
}

fn normalize_minute_records(
    source: &str,
    instrument: &InstrumentId,
    date: &str,
    mut records: Vec<MinuteTimePrice>,
) -> Result<DataBatch<MinutePoint>, TdxError> {
    if records.is_empty() {
        return Err(TdxError::InvalidData(
            "TDX returned an empty minute-data response".into(),
        ));
    }
    if records.len() > 240 {
        return Err(TdxError::InvalidData(format!(
            "TDX returned {} minute points; maximum is 240",
            records.len()
        )));
    }
    records.sort_by(|left, right| left.time.cmp(&right.time));
    let observed_at = fetched_at()?;
    let batch_id = format!("{source}:{observed_at}:minute");
    let mut normalized = Vec::with_capacity(records.len());
    let mut cumulative_quantity = 0.0;
    let mut previous_time: Option<&str> = None;
    for record in &records {
        if !valid_tdx_minute(&record.time) {
            return Err(TdxError::InvalidData(format!(
                "TDX minute time is outside the verified session grid: {:?}",
                record.time
            )));
        }
        if previous_time.is_some_and(|previous| previous >= record.time.as_str()) {
            return Err(TdxError::InvalidData(
                "TDX minute times are duplicated or unordered".into(),
            ));
        }
        if !record.vol.is_finite() || record.vol < 0.0 {
            return Err(TdxError::InvalidData(
                "TDX minute volume must be finite and non-negative".into(),
            ));
        }
        cumulative_quantity += record.vol;
        if !cumulative_quantity.is_finite() {
            return Err(TdxError::InvalidData(
                "TDX cumulative minute volume overflowed".into(),
            ));
        }
        let minute_at = format!("{date} {}", record.time);
        let source_at = format!("{date}T{}:00+08:00", record.time);
        normalized.push(MinutePoint::new(
            instrument.clone(),
            minute_at,
            Price::new(record.price)?,
            Quantity::new(cumulative_quantity)?,
            None,
            DataStatus::Available,
            Some(source_at),
            observed_at.clone(),
            ProviderId::Tdx,
            batch_id.clone(),
        )?);
        previous_time = Some(&record.time);
    }
    let latest_source_at = normalized
        .last()
        .and_then(|point| point.source_at())
        .ok_or_else(|| TdxError::InvalidData("TDX minute source time is missing".into()))?;
    let provenance = magic_market_core::Provenance::new(source, observed_at)?
        .with_source_at(latest_source_at)?
        .with_batch_id(batch_id)?;
    Ok(DataBatch::strict(normalized, provenance))
}

fn optional_quote_price(value: f64, field: &str) -> Result<Option<Price>, TdxError> {
    if !value.is_finite() || value < 0.0 {
        return Err(TdxError::InvalidData(format!(
            "TDX quote {field} must be finite and non-negative"
        )));
    }
    if value == 0.0 {
        Ok(None)
    } else {
        Price::new(value)
            .map(Some)
            .map_err(|error| TdxError::InvalidData(error.to_string()))
    }
}

pub(crate) fn normalize_quotes(
    source: &str,
    instruments: &[InstrumentId],
    records: Vec<SecurityQuote>,
) -> Result<DataBatch<Quote>, TdxError> {
    if instruments.is_empty() {
        return Err(TdxError::InvalidData("TDX quote request is empty".into()));
    }
    let mut requested = HashSet::new();
    if instruments
        .iter()
        .any(|instrument| !requested.insert(instrument.clone()))
    {
        return Err(TdxError::InvalidData(
            "TDX quote request contains duplicate instruments".into(),
        ));
    }
    if records.len() != instruments.len() {
        return Err(TdxError::InvalidData(format!(
            "TDX quote cardinality mismatch: requested {}, received {}",
            instruments.len(),
            records.len()
        )));
    }
    let mut by_key = HashMap::with_capacity(records.len());
    for record in records {
        let key = (record.market, record.code.clone());
        if by_key.insert(key, record).is_some() {
            return Err(TdxError::InvalidData(
                "TDX returned a duplicate quote".into(),
            ));
        }
    }

    let observed_at = fetched_at()?;
    let batch_id = format!("{source}:{observed_at}:quote");
    let mut quotes = Vec::with_capacity(instruments.len());
    let mut issues = Vec::new();
    for instrument in instruments {
        let key = (market(instrument)?, instrument.code().to_owned());
        let record = by_key
            .remove(&key)
            .ok_or_else(|| TdxError::InvalidData("TDX omitted a requested quote".into()))?;
        let price =
            Price::new(record.price).map_err(|error| TdxError::InvalidData(error.to_string()))?;
        let previous_close = optional_quote_price(record.last_close, "previous close")?;
        let open = optional_quote_price(record.open, "open")?;
        let high = optional_quote_price(record.high, "high")?;
        let low = optional_quote_price(record.low, "low")?;
        let volume =
            Quantity::new(record.vol).map_err(|error| TdxError::InvalidData(error.to_string()))?;
        if !record.amount.is_finite() || record.amount < 0.0 {
            return Err(TdxError::InvalidData(
                "TDX quote amount must be finite and non-negative".into(),
            ));
        }
        let amount = Money::new(record.amount)
            .map(Some)
            .map_err(|error| TdxError::InvalidData(error.to_string()))?;
        let change_percent = previous_close
            .map(|value| {
                Ratio::new(
                    (price.get() - value.get()) / value.get() * 100.0,
                    RatioUnit::Percent,
                )
            })
            .transpose()
            .map_err(|error| TdxError::InvalidData(error.to_string()))?;
        let source_at: Option<String> = None;
        let complete = previous_close.is_some()
            && open.is_some()
            && high.is_some()
            && low.is_some()
            && source_at.is_some();
        if !complete {
            issues.push(format!(
                "{}: one or more normalized quote fields unavailable",
                instrument.code()
            ));
        }
        issues.push(format!(
            "{}: security name unavailable from the TDX quote packet",
            instrument.code()
        ));
        issues.push(format!(
            "{}: TDX quote source timestamp format is unverified",
            instrument.code()
        ));
        quotes.push(Quote::from_parts(
            instrument.clone(),
            None,
            price,
            previous_close,
            open,
            high,
            low,
            change_percent,
            volume,
            amount,
            DataStatus::Unavailable,
            source_at,
            observed_at.clone(),
            ProviderId::Tdx,
            batch_id.clone(),
        )?);
    }
    if !by_key.is_empty() {
        return Err(TdxError::InvalidData(
            "TDX returned unexpected quotes".into(),
        ));
    }
    let provenance =
        magic_market_core::Provenance::new(source, observed_at)?.with_batch_id(batch_id)?;
    Ok(DataBatch::best_effort(quotes, provenance, issues)?)
}

fn tdx_trade_date(date: &str) -> Result<u32, TdxError> {
    if date.len() != 10 || date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
        return Err(TdxError::InvalidData(
            "invalid normalized trade date".into(),
        ));
    }
    date.bytes()
        .filter(|byte| *byte != b'-')
        .try_fold(0u32, |value, byte| {
            let digit = byte
                .is_ascii_digit()
                .then_some(u32::from(byte - b'0'))
                .ok_or_else(|| TdxError::InvalidData("invalid normalized trade date".into()))?;
            value
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .ok_or_else(|| TdxError::InvalidData("normalized trade date overflow".into()))
        })
}

fn trade_side(value: u32) -> TradeSide {
    match value {
        0 => TradeSide::Buy,
        1 => TradeSide::Sell,
        2 => TradeSide::Neutral,
        value => TradeSide::Unknown(value),
    }
}

fn normalize_trade_records(
    source: &str,
    request: &TradesRequest,
    records: Vec<TickData>,
) -> Result<DataBatch<Trade>, TdxError> {
    ensure_nonempty(&records)?;
    let observed_at = fetched_at()?;
    let batch_id = format!("{source}:{observed_at}:trades");
    let mut issues = Vec::new();
    let mut trades = Vec::with_capacity(records.len());
    for record in records {
        if record.time.is_empty() {
            return Err(TdxError::InvalidData("TDX trade time is empty".into()));
        }
        let price =
            Price::new(record.price).map_err(|error| TdxError::InvalidData(error.to_string()))?;
        let quantity =
            Quantity::new(record.vol).map_err(|error| TdxError::InvalidData(error.to_string()))?;
        let side = trade_side(record.buyorsell);
        let complete = !matches!(side, TradeSide::Unknown(_));
        if !complete {
            issues.push(format!(
                "{} {}: unknown TDX trade side {}",
                request.instrument().code(),
                record.time,
                record.buyorsell
            ));
        }
        let trade_at = request.date().map_or_else(
            || record.time.clone(),
            |date| format!("{date} {}", record.time),
        );
        trades.push(Trade::new(
            request.instrument().clone(),
            trade_at.clone(),
            price,
            quantity,
            (record.num != 0).then_some(record.num),
            side,
            if complete {
                DataStatus::Available
            } else {
                DataStatus::Unavailable
            },
            // Current-session packets carry a time only; historical packets
            // are qualified with the requested source date.
            Some(trade_at),
            observed_at.clone(),
            ProviderId::Tdx,
            batch_id.clone(),
        )?);
    }
    let mut provenance =
        magic_market_core::Provenance::new(source, observed_at)?.with_batch_id(batch_id)?;
    if let Some(source_at) = trades
        .last()
        .and_then(|trade| trade.source_at().map(str::to_owned))
    {
        provenance = provenance.with_source_at(source_at)?;
    }
    Ok(DataBatch::best_effort(trades, provenance, issues)?)
}

fn paginate_trades<F>(
    source: &str,
    request: &TradesRequest,
    page_size: u16,
    mut fetch: F,
) -> Result<DataBatch<Trade>, TdxError>
where
    F: FnMut(u16, u16) -> Result<Vec<TickData>, TdxError>,
{
    let mut records = Vec::with_capacity(usize::from(request.limit()));
    let mut start = 0u16;
    let mut remaining = request.limit();
    while remaining != 0 {
        let requested = remaining.min(page_size);
        let page = fetch(start, requested)?;
        if page.len() > usize::from(requested) {
            return Err(TdxError::InvalidData(
                "TDX trade page exceeds requested cardinality".into(),
            ));
        }
        let fetched = u16::try_from(page.len())
            .map_err(|_| TdxError::InvalidData("TDX trade page is too large".into()))?;
        records.extend(page);
        if fetched < requested {
            break;
        }
        remaining -= fetched;
        if remaining == 0 {
            break;
        }
        start = start
            .checked_add(fetched)
            .ok_or_else(|| TdxError::InvalidData("TDX trade offset overflow".into()))?;
    }
    normalize_trade_records(source, request, records)
}

fn board(instrument: &InstrumentId) -> Board {
    if instrument.asset_class() != magic_market_core::AssetClass::Equity {
        return Board::Unknown;
    }
    match instrument.exchange() {
        magic_market_core::Exchange::Shanghai
            if instrument.code().starts_with("688") || instrument.code().starts_with("689") =>
        {
            Board::Star
        }
        magic_market_core::Exchange::Shenzhen
            if instrument.code().starts_with("300") || instrument.code().starts_with("301") =>
        {
            Board::ChiNext
        }
        magic_market_core::Exchange::Shanghai | magic_market_core::Exchange::Shenzhen => {
            Board::Main
        }
        magic_market_core::Exchange::Beijing => Board::Beijing,
    }
}

fn st_flag(name: &str) -> Option<bool> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let uppercase = name.to_ascii_uppercase();
    Some(
        uppercase.starts_with("ST")
            || uppercase.starts_with("*ST")
            || uppercase.starts_with("S*ST")
            || uppercase.starts_with("SST"),
    )
}

fn fetch_security_records<Count, Page>(
    instruments: &[InstrumentId],
    mut count: Count,
    mut page: Page,
) -> Result<Vec<(u8, SecurityInfo)>, TdxError>
where
    Count: FnMut(u8) -> Result<u16, TdxError>,
    Page: FnMut(u8, u16) -> Result<Vec<SecurityInfo>, TdxError>,
{
    if instruments.is_empty() {
        return Err(TdxError::InvalidData(
            "TDX security metadata request is empty".into(),
        ));
    }
    let mut requested = HashSet::new();
    let mut markets = HashSet::new();
    for instrument in instruments {
        let instrument_market = market(instrument)?;
        if !requested.insert((instrument_market, instrument.code().to_owned())) {
            return Err(TdxError::InvalidData(
                "TDX security metadata request contains duplicates".into(),
            ));
        }
        markets.insert(instrument_market);
    }

    let mut records = Vec::with_capacity(instruments.len());
    for instrument_market in markets {
        let expected = usize::from(count(instrument_market)?);
        let market_requested = requested
            .iter()
            .filter(|(value, _)| *value == instrument_market)
            .count();
        let mut seen = 0usize;
        let mut found = 0usize;
        while seen < expected && found < market_requested {
            let start = u16::try_from(seen)
                .map_err(|_| TdxError::InvalidData("TDX security list offset overflow".into()))?;
            let source_page = page(instrument_market, start)?;
            if source_page.is_empty() || seen + source_page.len() > expected {
                return Err(TdxError::InvalidData(
                    "TDX security list cardinality mismatch".into(),
                ));
            }
            seen += source_page.len();
            for record in source_page {
                if requested.contains(&(instrument_market, record.code.clone())) {
                    records.push((instrument_market, record));
                    found += 1;
                }
            }
        }
    }
    Ok(records)
}

fn validate_security_metadata_request(instruments: &[InstrumentId]) -> Result<(), TdxError> {
    if instruments
        .iter()
        .any(|instrument| instrument.exchange() == magic_market_core::Exchange::Beijing)
    {
        return Err(TdxError::Unsupported(
            "TDX market=2 serves Beijing quotes, bars, minute data, trades and order books, \
             but live-verified servers close the security-list request required for Beijing \
             security metadata"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn normalize_security_metadata(
    source: &str,
    instruments: &[InstrumentId],
    records: Vec<(u8, SecurityInfo)>,
) -> Result<DataBatch<SecurityMetadata>, TdxError> {
    normalize_security_metadata_inner(source, instruments, records, None)
}

fn normalize_security_metadata_with_finance(
    source: &str,
    instruments: &[InstrumentId],
    records: Vec<(u8, SecurityInfo)>,
    finance: Vec<FinanceInfo>,
) -> Result<DataBatch<SecurityMetadata>, TdxError> {
    if finance.len() != instruments.len() {
        return Err(TdxError::InvalidData(
            "TDX finance response cardinality does not match metadata request".into(),
        ));
    }
    let mut by_key = HashMap::with_capacity(finance.len());
    for record in finance {
        let key = (record.market, record.code.clone());
        if by_key.insert(key, record).is_some() {
            return Err(TdxError::InvalidData(
                "TDX returned duplicate finance metadata".into(),
            ));
        }
    }
    normalize_security_metadata_inner(source, instruments, records, Some(by_key))
}

fn normalize_security_metadata_inner(
    source: &str,
    instruments: &[InstrumentId],
    records: Vec<(u8, SecurityInfo)>,
    mut finance: Option<HashMap<(u8, String), FinanceInfo>>,
) -> Result<DataBatch<SecurityMetadata>, TdxError> {
    let mut by_key = HashMap::with_capacity(records.len());
    for (record_market, record) in records {
        let key = (record_market, record.code.clone());
        if by_key.insert(key, record).is_some() {
            return Err(TdxError::InvalidData(
                "TDX returned duplicate security metadata".into(),
            ));
        }
    }

    let observed_at = fetched_at()?;
    let batch_id = format!("{source}:{observed_at}:security-metadata");
    let mut normalized = Vec::with_capacity(instruments.len());
    let mut issues = Vec::new();
    for instrument in instruments {
        let key = (market(instrument)?, instrument.code().to_owned());
        let record = by_key.remove(&key).ok_or_else(|| {
            TdxError::InvalidData(format!(
                "TDX omitted requested security metadata for {}",
                instrument.code()
            ))
        })?;
        let listed_on = match finance.as_mut() {
            Some(records) => {
                let record = records.remove(&key).ok_or_else(|| {
                    TdxError::InvalidData(format!(
                        "TDX omitted requested finance metadata for {}",
                        instrument.code()
                    ))
                })?;
                normalize_ipo_date(record.ipo_date)?
            }
            None => None,
        };
        let name = (!record.name.trim().is_empty()).then(|| record.name.trim().to_owned());
        let is_st = if instrument.asset_class() == magic_market_core::AssetClass::Equity {
            name.as_deref().and_then(st_flag)
        } else {
            None
        };
        if name.is_none() {
            issues.push(format!("{}: security name unavailable", instrument.code()));
        }
        issues.push(format!(
            "{}: board is derived from exchange/code because the TDX list packet has no board field",
            instrument.code()
        ));
        if listed_on.is_none() {
            issues.push(format!("{}: listing date unavailable", instrument.code()));
        }
        issues.push(format!(
            "{}: source-backed price-limit rule and version unavailable",
            instrument.code()
        ));
        issues.push(format!(
            "{}: source timestamp unavailable",
            instrument.code()
        ));
        normalized.push(SecurityMetadata::new(
            instrument.clone(),
            name,
            Some(board(instrument)),
            is_st,
            listed_on,
            PriceLimitRule::new(None, None)?,
            DataStatus::Unavailable,
            None,
            observed_at.clone(),
            ProviderId::Tdx,
            batch_id.clone(),
        )?);
    }
    if !by_key.is_empty() {
        return Err(TdxError::InvalidData(
            "TDX returned unexpected security metadata".into(),
        ));
    }
    if finance.as_ref().is_some_and(|records| !records.is_empty()) {
        return Err(TdxError::InvalidData(
            "TDX returned unexpected finance metadata".into(),
        ));
    }
    let provenance =
        magic_market_core::Provenance::new(source, observed_at)?.with_batch_id(batch_id)?;
    Ok(DataBatch::best_effort(normalized, provenance, issues)?)
}

fn normalize_ipo_date(value: u32) -> Result<Option<String>, TdxError> {
    if value == 0 {
        return Ok(None);
    }
    let compact = format!("{value:08}");
    if compact.len() != 8 {
        return Err(TdxError::InvalidData(
            "TDX IPO date must contain exactly eight digits".into(),
        ));
    }
    let value = format!("{}-{}-{}", &compact[0..4], &compact[4..6], &compact[6..8]);
    let date = IsoDate::new(value.clone())
        .map_err(|error| TdxError::InvalidData(format!("invalid TDX IPO date: {error}")))?;
    if compact
        .parse::<u32>()
        .map_err(|_| TdxError::InvalidData("invalid TDX IPO date digits".into()))?
        > crate::net::utils::today_yyyymmdd()
    {
        return Err(TdxError::InvalidData(format!(
            "TDX IPO date {} is in the future",
            date.as_str()
        )));
    }
    Ok(Some(value))
}

pub(crate) fn validate_corporate_action_request(
    request: &CorporateActionRequest,
    admission_as_of: &IsoDate,
) -> Result<(), TdxError> {
    if request.instrument().exchange() == magic_market_core::Exchange::Beijing {
        return Err(TdxError::Unsupported(
            "TDX corporate-action response identity is not live-admitted for Beijing".into(),
        ));
    }
    if !matches!(
        request.instrument().asset_class(),
        magic_market_core::AssetClass::Equity | magic_market_core::AssetClass::Fund
    ) {
        return Err(TdxError::Unsupported(
            "TDX normalized corporate actions support equities and exchange-traded funds only"
                .into(),
        ));
    }
    if request.start().is_some_and(|start| start > admission_as_of)
        || request.end().is_some_and(|end| end > admission_as_of)
    {
        return Err(magic_market_core::CoreError::InvalidRequest(
            "TDX corporate-action request range extends into the future".into(),
        )
        .into());
    }
    Ok(())
}

pub(crate) fn current_corporate_action_admission_date() -> Result<IsoDate, TdxError> {
    let compact = format!("{:08}", crate::net::utils::today_yyyymmdd());
    IsoDate::new(format!(
        "{}-{}-{}",
        &compact[0..4],
        &compact[4..6],
        &compact[6..8]
    ))
    .map_err(|error| {
        TdxError::InvalidData(format!(
            "current China corporate-action admission date is invalid: {error}"
        ))
    })
}

pub(crate) fn normalize_corporate_actions(
    source: &str,
    request: &CorporateActionRequest,
    records: Vec<XdXrInfo>,
    admission_as_of: &IsoDate,
) -> Result<DataBatch<CorporateAction>, TdxError> {
    validate_corporate_action_request(request, admission_as_of)?;
    let observed_at = fetched_at()?;
    let batch_id = format!(
        "{source}:{observed_at}:corporate-actions:{:?}:{}:{:?}:{}:{}",
        request.instrument().exchange(),
        request.instrument().code(),
        request.instrument().asset_class(),
        request.start().map_or("all", IsoDate::as_str),
        request.end().map_or("all", IsoDate::as_str),
    );
    let evidence = SourceEvidence::new(ProviderId::Tdx, observed_at.clone(), batch_id.clone())?;
    let mut normalized = Vec::new();
    let mut identities = HashSet::new();
    let mut previous_effective_on = None::<IsoDate>;
    let mut source_order = None::<std::cmp::Ordering>;

    for record in records {
        let effective_on = IsoDate::new(format!(
            "{:04}-{:02}-{:02}",
            record.year, record.month, record.day
        ))
        .map_err(|error| {
            TdxError::InvalidData(format!("invalid TDX corporate-action date: {error}"))
        })?;
        if &effective_on > admission_as_of {
            return Err(TdxError::InvalidData(format!(
                "TDX corporate-action date {} is in the future",
                effective_on.as_str()
            )));
        }
        if let Some(previous) = &previous_effective_on {
            let ordering = previous.cmp(&effective_on);
            if ordering != std::cmp::Ordering::Equal {
                match source_order {
                    Some(expected) if expected != ordering => {
                        return Err(TdxError::InvalidData(
                            "TDX corporate-action packet reverses its source date order".into(),
                        ));
                    }
                    None => source_order = Some(ordering),
                    _ => {}
                }
            }
        }
        previous_effective_on = Some(effective_on.clone());
        let category = match record.category {
            1 => CorporateActionCategory::Distribution,
            2 => CorporateActionCategory::BonusRightsListing,
            3 => CorporateActionCategory::NonTradableShareListing,
            4 => CorporateActionCategory::UnknownCapitalChange,
            5 => CorporateActionCategory::CapitalChange,
            6 => CorporateActionCategory::AdditionalIssuance,
            7 => CorporateActionCategory::ShareRepurchase,
            8 => CorporateActionCategory::AdditionalIssuanceListing,
            9 => CorporateActionCategory::TransferredAllotmentListing,
            10 => CorporateActionCategory::ConvertibleBondListing,
            11 => CorporateActionCategory::CapitalRescaling,
            12 => CorporateActionCategory::NonTradableReverseSplit,
            13 => CorporateActionCategory::SubscriptionWarrantGrant,
            14 => CorporateActionCategory::PutWarrantGrant,
            value => {
                return Err(TdxError::InvalidData(format!(
                    "TDX returned unknown XDXR category {value}"
                )));
            }
        };
        if !identities.insert((effective_on.clone(), category)) {
            return Err(TdxError::InvalidData(format!(
                "TDX returned duplicate {:?} action on {}",
                category,
                effective_on.as_str()
            )));
        }

        let terms = match category {
            CorporateActionCategory::Distribution => {
                let cash_per_share = record
                    .fenhong
                    .map(|value| FiniteNumber::new(value / 10.0))
                    .transpose()?;
                let bonus_per_share = record
                    .songzhuangu
                    .map(|value| FiniteNumber::new(value / 10.0))
                    .transpose()?;
                let rights_per_share = record
                    .peigu
                    .map(|value| FiniteNumber::new(value / 10.0))
                    .transpose()?;
                let rights_price = match record.peigujia {
                    Some(value) if value < 0.0 => {
                        return Err(TdxError::InvalidData(
                            "TDX distribution rights price must be non-negative".into(),
                        ));
                    }
                    Some(value) if value > 0.0 => Some(Price::new(value)?),
                    _ => None,
                };
                CorporateActionTerms::distribution(
                    cash_per_share,
                    bonus_per_share,
                    rights_per_share,
                    rights_price,
                )?
            }
            CorporateActionCategory::BonusRightsListing
            | CorporateActionCategory::NonTradableShareListing
            | CorporateActionCategory::UnknownCapitalChange
            | CorporateActionCategory::CapitalChange
            | CorporateActionCategory::AdditionalIssuance
            | CorporateActionCategory::ShareRepurchase
            | CorporateActionCategory::AdditionalIssuanceListing
            | CorporateActionCategory::TransferredAllotmentListing
            | CorporateActionCategory::ConvertibleBondListing => {
                let tradable_before = record.panqianliutong.ok_or_else(|| {
                    TdxError::InvalidData(
                        "TDX capital-structure action omitted tradable-before".into(),
                    )
                })?;
                let tradable_after = record.panhouliutong.ok_or_else(|| {
                    TdxError::InvalidData(
                        "TDX capital-structure action omitted tradable-after".into(),
                    )
                })?;
                let total_before = record.qianzongguben.ok_or_else(|| {
                    TdxError::InvalidData(
                        "TDX capital-structure action omitted total-before".into(),
                    )
                })?;
                let total_after = record.houzongguben.ok_or_else(|| {
                    TdxError::InvalidData("TDX capital-structure action omitted total-after".into())
                })?;
                CorporateActionTerms::capital_structure(
                    category,
                    FiniteNumber::new(tradable_before)?,
                    FiniteNumber::new(tradable_after)?,
                    FiniteNumber::new(total_before)?,
                    FiniteNumber::new(total_after)?,
                    UnverifiedSourceUnit::ProviderNative,
                )?
            }
            CorporateActionCategory::CapitalRescaling
            | CorporateActionCategory::NonTradableReverseSplit => {
                let source_ratio = record.suogu.ok_or_else(|| {
                    TdxError::InvalidData("TDX split action omitted its source ratio".into())
                })?;
                CorporateActionTerms::provider_native_ratio(
                    category,
                    FiniteNumber::new(source_ratio)?,
                    UnverifiedSourceUnit::ProviderNative,
                )?
            }
            CorporateActionCategory::SubscriptionWarrantGrant
            | CorporateActionCategory::PutWarrantGrant => {
                let exercise_price = record.xingquanjia.ok_or_else(|| {
                    TdxError::InvalidData("TDX warrant grant omitted exercise price".into())
                })?;
                let source_quantity = record.fenshu.ok_or_else(|| {
                    TdxError::InvalidData("TDX warrant grant omitted source quantity".into())
                })?;
                CorporateActionTerms::warrant_grant(
                    category,
                    Price::new(exercise_price)?,
                    FiniteNumber::new(source_quantity)?,
                    UnverifiedSourceUnit::ProviderNative,
                )?
            }
        };
        normalized.push(CorporateAction::new(
            request.instrument().clone(),
            category,
            effective_on,
            CorporateActionStatus::Implemented,
            terms,
            evidence.clone(),
        )?);
    }
    normalized.sort_by(|left, right| {
        left.effective_on()
            .cmp(right.effective_on())
            .then_with(|| left.category().cmp(&right.category()))
    });
    normalized.retain(|record| {
        request
            .start()
            .is_none_or(|start| record.effective_on() >= start)
            && request.end().is_none_or(|end| record.effective_on() <= end)
    });

    let provenance =
        magic_market_core::Provenance::new(source, observed_at)?.with_batch_id(batch_id)?;
    Ok(DataBatch::strict(normalized, provenance))
}

pub(crate) fn corporate_action_response(
    request: &CorporateActionRequest,
    batch: DataBatch<CorporateAction>,
    admission_as_of: IsoDate,
) -> Result<CorporateActionResponse, TdxError> {
    let provenance = batch.provenance();
    let batch_id = provenance.batch_id().ok_or_else(|| {
        TdxError::InvalidData("TDX corporate-action batch omitted its batch ID".into())
    })?;
    let mut evidence = SourceEvidence::new(ProviderId::Tdx, provenance.fetched_at(), batch_id)?;
    if let Some(source_at) = provenance.source_at() {
        evidence = evidence.with_source_at(source_at)?;
    }
    Ok(CorporateActionResponse::new(
        request.clone(),
        admission_as_of,
        evidence,
        batch,
    )?)
}

fn corporate_actions_with(
    query: &impl BlockingTdxQuery,
    source: &str,
    request: &CorporateActionRequest,
) -> Result<CorporateActionResponse, TdxError> {
    let admission_as_of = current_corporate_action_admission_date()?;
    validate_corporate_action_request(request, &admission_as_of)?;
    let raw = query.xdxr_info(market(request.instrument())?, request.instrument().code())?;
    let batch = normalize_corporate_actions(source, request, raw, &admission_as_of)?;
    corporate_action_response(request, batch, admission_as_of)
}

impl SecurityMetadataProvider for TdxHqClient {
    type Error = TdxError;

    fn security_metadata(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<SecurityMetadata>, Self::Error> {
        security_metadata_with(self, "tdx", instruments)
    }
}

impl SecurityMetadataProvider for crate::TdxSmartClient {
    type Error = TdxError;

    fn security_metadata(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<SecurityMetadata>, Self::Error> {
        security_metadata_with(self, "tdx", instruments)
    }
}

macro_rules! impl_corporate_actions {
    ($($client:ty),+ $(,)?) => {
        $(
            impl CorporateActions for $client {
                type Error = TdxError;

                fn corporate_actions(
                    &self,
                    request: &CorporateActionRequest,
                ) -> Result<CorporateActionResponse, Self::Error> {
                    corporate_actions_with(self, "tdx", request)
                }
            }
        )+
    };
}

impl_corporate_actions!(TdxHqClient, crate::TdxSmartClient, crate::TdxDirectClient);

impl HistoricalBars for TdxHqClient {
    type Bar = Bar;
    type Error = TdxError;
    fn historical_bars(&self, request: &BarsRequest) -> Result<DataBatch<Self::Bar>, Self::Error> {
        historical_bars_with(self, "tdx", request)
    }
}

impl MinuteData for TdxHqClient {
    type Error = TdxError;

    fn minute_data(
        &self,
        request: &MinuteDataRequest,
    ) -> Result<DataBatch<MinutePoint>, Self::Error> {
        minute_data_with(self, "tdx", request)
    }
}

impl RealtimeQuotes for TdxHqClient {
    type Quote = Quote;
    type Error = TdxError;
    fn realtime_quotes(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<Self::Quote>, Self::Error> {
        realtime_quotes_with(self, "tdx", instruments)
    }
}

impl Trades for TdxHqClient {
    type Error = TdxError;

    fn trades(&self, request: &TradesRequest) -> Result<DataBatch<Trade>, Self::Error> {
        trades_with(self, "tdx-current", "tdx-history", request)
    }
}

fn book_level(price: f64, quantity: f64) -> Result<BookLevel, TdxError> {
    match (price, quantity) {
        (price, quantity) if !price.is_finite() || !quantity.is_finite() => Err(
            TdxError::InvalidData("TDX order-book level must be finite".into()),
        ),
        (price, quantity) if price < 0.0 || quantity < 0.0 => Err(TdxError::InvalidData(
            "TDX order-book level must be non-negative".into(),
        )),
        (0.0, _) => Ok(BookLevel::unavailable()),
        (price, quantity) => Ok(BookLevel::new(
            Some(Price::new(price)?),
            Some(Quantity::new(quantity)?),
        )?),
    }
}

fn book_depth(levels: &[BookLevel; 5]) -> Result<Option<Quantity>, TdxError> {
    let mut found = false;
    let total =
        levels
            .iter()
            .filter_map(|level| level.quantity())
            .fold(0.0, |accumulator, quantity| {
                found = true;
                accumulator + quantity.get()
            });
    if found {
        Quantity::new(total)
            .map(Some)
            .map_err(|error| TdxError::InvalidData(error.to_string()))
    } else {
        Ok(None)
    }
}

pub(crate) fn order_book_pairs<'a>(
    instruments: &'a [InstrumentId],
    provider: &str,
) -> Result<Vec<(u8, &'a str)>, TdxError> {
    if instruments.is_empty() {
        return Err(TdxError::InvalidData(format!(
            "{provider} order-book request is empty"
        )));
    }
    let mut requested = HashSet::with_capacity(instruments.len());
    instruments
        .iter()
        .map(|instrument| {
            let pair = (market(instrument)?, instrument.code());
            if !requested.insert((pair.0, pair.1.to_owned())) {
                return Err(TdxError::InvalidData(format!(
                    "{provider} order-book request contains a duplicate instrument"
                )));
            }
            Ok(pair)
        })
        .collect()
}

pub(crate) fn ordered_order_book_quotes<'a>(
    instruments: &'a [InstrumentId],
    quotes: Vec<SecurityQuote>,
    provider: &str,
) -> Result<Vec<(&'a InstrumentId, SecurityQuote)>, TdxError> {
    let requested: HashSet<(u8, String)> = order_book_pairs(instruments, provider)?
        .into_iter()
        .map(|(market, code)| (market, code.to_owned()))
        .collect();

    let mut by_instrument = HashMap::with_capacity(quotes.len());
    for quote in quotes {
        let key = (quote.market, quote.code.clone());
        if !requested.contains(&key) {
            return Err(TdxError::InvalidData(format!(
                "{provider} order-book response contains unexpected instrument {}:{}",
                key.0, key.1
            )));
        }
        if by_instrument.insert(key.clone(), quote).is_some() {
            return Err(TdxError::InvalidData(format!(
                "{provider} order-book response contains duplicate instrument {}:{}",
                key.0, key.1
            )));
        }
    }

    instruments
        .iter()
        .map(|instrument| {
            let key = (market(instrument)?, instrument.code().to_owned());
            let quote = by_instrument.remove(&key).ok_or_else(|| {
                TdxError::InvalidData(format!(
                    "{provider} order-book response is missing instrument {}:{}",
                    key.0, key.1
                ))
            })?;
            Ok((instrument, quote))
        })
        .collect()
}

pub(crate) fn normalize_order_books(
    provider: &str,
    source: &str,
    instruments: &[InstrumentId],
    quotes: Vec<SecurityQuote>,
) -> Result<DataBatch<OrderBook>, TdxError> {
    let ordered = ordered_order_book_quotes(instruments, quotes, provider)?;
    let observed_at = fetched_at()?;
    let batch_id = format!("{source}:{observed_at}:order-book");
    let mut books = Vec::with_capacity(ordered.len());
    let mut issues = Vec::new();
    for (id, quote) in ordered {
        let bids = [
            book_level(quote.bid1, quote.bid_vol1)?,
            book_level(quote.bid2, quote.bid_vol2)?,
            book_level(quote.bid3, quote.bid_vol3)?,
            book_level(quote.bid4, quote.bid_vol4)?,
            book_level(quote.bid5, quote.bid_vol5)?,
        ];
        let asks = [
            book_level(quote.ask1, quote.ask_vol1)?,
            book_level(quote.ask2, quote.ask_vol2)?,
            book_level(quote.ask3, quote.ask_vol3)?,
            book_level(quote.ask4, quote.ask_vol4)?,
            book_level(quote.ask5, quote.ask_vol5)?,
        ];
        let total_bid_quantity = book_depth(&bids)?;
        let total_ask_quantity = book_depth(&asks)?;
        let levels_complete = bids
            .iter()
            .chain(&asks)
            .all(|level| level.price().is_some());
        if !levels_complete {
            issues.push(format!(
                "{}: one or more normalized order-book fields unavailable",
                id.code()
            ));
        }
        issues.push(format!(
            "{}: TDX order-book source timestamp format is unverified",
            id.code()
        ));
        books.push(OrderBook::new(
            id.clone(),
            bids,
            asks,
            total_bid_quantity,
            total_ask_quantity,
            DataStatus::Unavailable,
            None,
            observed_at.clone(),
            ProviderId::Tdx,
            batch_id.clone(),
        )?);
    }
    let provenance =
        magic_market_core::Provenance::new(source, observed_at)?.with_batch_id(batch_id)?;
    Ok(DataBatch::best_effort(books, provenance, issues)?)
}

fn order_books_with(
    query: &impl BlockingTdxQuery,
    provider: &str,
    source: &str,
    instruments: &[InstrumentId],
) -> Result<DataBatch<OrderBook>, TdxError> {
    let pairs = order_book_pairs(instruments, provider)?;
    let quotes = query.security_quotes(&pairs)?;
    normalize_order_books(provider, source, instruments, quotes)
}

impl OrderBooks for TdxHqClient {
    type Error = TdxError;
    fn order_books(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<OrderBook>, Self::Error> {
        order_books_with(self, "TDX", "tdx", instruments)
    }
}

impl OrderBooks for crate::TdxSmartClient {
    type Error = TdxError;
    fn order_books(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<OrderBook>, Self::Error> {
        order_books_with(self, "TDX smart", "tdx-smart", instruments)
    }
}

impl HistoricalBars for crate::TdxSmartClient {
    type Bar = Bar;
    type Error = TdxError;
    fn historical_bars(&self, request: &BarsRequest) -> Result<DataBatch<Self::Bar>, Self::Error> {
        historical_bars_with(self, "tdx-smart", request)
    }
}

impl MinuteData for crate::TdxSmartClient {
    type Error = TdxError;

    fn minute_data(
        &self,
        request: &MinuteDataRequest,
    ) -> Result<DataBatch<MinutePoint>, Self::Error> {
        minute_data_with(self, "tdx", request)
    }
}

impl RealtimeQuotes for crate::TdxSmartClient {
    type Quote = Quote;
    type Error = TdxError;
    fn realtime_quotes(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<Self::Quote>, Self::Error> {
        realtime_quotes_with(self, "tdx-smart", instruments)
    }
}

impl Trades for crate::TdxSmartClient {
    type Error = TdxError;

    fn trades(&self, request: &TradesRequest) -> Result<DataBatch<Trade>, Self::Error> {
        trades_with(self, "tdx-current", "tdx-history", request)
    }
}

impl HistoricalBars for crate::TdxDirectClient {
    type Bar = Bar;
    type Error = TdxError;
    fn historical_bars(&self, request: &BarsRequest) -> Result<DataBatch<Self::Bar>, Self::Error> {
        historical_bars_with(self, "tdx-direct", request)
    }
}

impl RealtimeQuotes for crate::TdxDirectClient {
    type Quote = Quote;
    type Error = TdxError;
    fn realtime_quotes(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<Self::Quote>, Self::Error> {
        realtime_quotes_with(self, "tdx-direct", instruments)
    }
}

impl Trades for crate::TdxDirectClient {
    type Error = TdxError;

    fn trades(&self, request: &TradesRequest) -> Result<DataBatch<Trade>, Self::Error> {
        trades_with(self, "tdx-direct-current", "tdx-direct-history", request)
    }
}

impl AsyncHistoricalBars for crate::AsyncTdxHqClient {
    type Bar = Bar;
    type Error = TdxError;
    async fn historical_bars_async(
        &self,
        request: &BarsRequest,
    ) -> Result<DataBatch<Self::Bar>, Self::Error> {
        historical_bars_async_with(self, "tdx-async", request).await
    }
}

impl AsyncRealtimeQuotes for crate::AsyncTdxHqClient {
    type Quote = Quote;
    type Error = TdxError;
    async fn realtime_quotes_async(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<Self::Quote>, Self::Error> {
        realtime_quotes_async_with(self, "tdx-async", instruments).await
    }
}

impl AsyncTrades for crate::AsyncTdxHqClient {
    type Error = TdxError;

    async fn trades_async(&self, request: &TradesRequest) -> Result<DataBatch<Trade>, Self::Error> {
        trades_async_with(self, request).await
    }
}

#[cfg(test)]
#[path = "../tests/internal/adapter.rs"]
mod tests;

macro_rules! unsupported_p0 {
    ($client:ty) => {
        impl MoneyFlows for $client {
            type Error = TdxError;
            fn money_flows(
                &self,
                _instruments: &[InstrumentId],
            ) -> Result<DataBatch<MoneyFlow>, Self::Error> {
                Err(TdxError::Unsupported(
                    "TDX quote/trade packets do not provide auditable main/net inflow fields or \
                     source methodology required by MoneyFlow"
                        .into(),
                ))
            }
        }
        impl Auctions for $client {
            type Error = TdxError;
            fn auction_snapshots(
                &self,
                _instruments: &[InstrumentId],
            ) -> Result<DataBatch<AuctionSnapshot>, Self::Error> {
                Err(TdxError::Unsupported(
                    "TDX packets do not provide the standardized indicative price and matched/\
                     unmatched quantities required by AuctionSnapshot"
                        .into(),
                ))
            }
        }
    };
}
unsupported_p0!(TdxHqClient);
unsupported_p0!(crate::TdxSmartClient);
