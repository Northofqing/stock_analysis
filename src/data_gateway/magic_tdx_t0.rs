//! BR-164 Magic TDX Gateway evidence boundary for reverse-T observation plans.
//!
//! This module preserves provider timestamps and rejects incomplete or invalid
//! evidence. It never substitutes another data source or manufactures missing
//! price, volume, time, or order-book values.
//!
//! Business rules: BR-092, BR-151, BR-153, BR-171, BR-187.

use crate::magic_compat::{InstrumentId, ProviderId};
use anyhow::{anyhow, Result};
#[cfg(feature = "magic-gateway")]
use chrono::TimeZone;
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::protocol::constants::{fq_type, KLINE_5MIN, KLINE_DAILY};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::protocol::types::{MinuteTimePrice, SecurityBar, SecurityQuote};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::TdxHqClient;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
#[cfg(feature = "magic-gateway")]
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use super::instrument_identity::{resolve_production_equity, EquitySegment};

pub const T0_QUOTE_MAX_AGE_SECS: i64 = 5;
pub const T0_DAILY_MIN_BARS: usize = 20;
pub const T0_TODAY_MIN_COMPLETED_BARS: usize = 6;
pub const T0_HISTORY_MIN_SESSIONS: usize = 3;

/// BR-231 诊断/重放检测缓存：code → (最近通过五秒门的最新价,
/// 其 provider servertime)。缓存不参与 freshness 准入裁决，不得把过期 quote
/// 因首次播种、价格变化或时间前进而放行；拒绝数据不写入缓存。
type T0QuoteCache = HashMap<String, (f64, DateTime<Utc>)>;

static LAST_T0_QUOTES: OnceLock<Mutex<T0QuoteCache>> = OnceLock::new();

/// 缓存容量上限：超出时重建（保最近语义, 低频事件损失可接受; 候选池
/// 代码数量级远低于此, 正常路径永不触发）。
const LAST_T0_QUOTE_CACHE_MAX: usize = 500;

fn last_t0_quotes() -> &'static Mutex<T0QuoteCache> {
    LAST_T0_QUOTES.get_or_init(|| Mutex::new(HashMap::new()))
}

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
    pub provider: ProviderId,
    pub source: String,
    pub requested_at: DateTime<Utc>,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub records: Vec<MagicTdxT0Evidence>,
    pub rejections: Vec<MagicTdxT0Rejection>,
    /// 2026-08-21 用户指令「拿不到时间先跳过, 标注时间不可信」:
    /// 桥往返 + server 侧整批组装 (7 codes × 4 TDX 调用) 耗时 11-55s,
    /// 且 TDX servertime 实测滞后墙钟 14-27s (2026-08-21 全天) — 服务端
    /// age 门放宽时置 true; consumer 侧按 record age 复查亦置 true。
    /// 推送方必须标注「时间不可信」。未来时间 (损坏) 仍硬拒, 不置此位。
    pub time_untrustworthy: bool,
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

/// BR-231 做T实时 quote 五秒 freshness 门。
///
/// `age = observed_at - source_at` 仅允许 `0..=5s`。未来时间或超过五秒
/// 一律显式 `quote_stale`。`quote_price` 只用于在准入后记录诊断缓存，
/// 价格变化、servertime 前进和首次见 code 都不参与准入。
pub fn validate_quote_freshness(
    code: &str,
    source_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
    quote_price: Option<f64>,
) -> std::result::Result<(), MagicTdxT0Rejection> {
    if source_at > observed_at {
        let future_by_ms = source_at
            .signed_duration_since(observed_at)
            .num_milliseconds();
        return Err(rejection(
            code,
            "quote_stale",
            format!(
                "future_time future_by_ms={future_by_ms} max_age_ms={}",
                T0_QUOTE_MAX_AGE_SECS * 1_000
            ),
            true,
        ));
    }
    let age = observed_at.signed_duration_since(source_at);
    if age > chrono::Duration::seconds(T0_QUOTE_MAX_AGE_SECS) {
        return Err(rejection(
            code,
            "quote_stale",
            format!(
                "stale age_ms={} max_secs={T0_QUOTE_MAX_AGE_SECS}",
                age.num_milliseconds()
            ),
            true,
        ));
    }
    if let Some(price) = quote_price {
        record_last_quote(code, price, source_at);
    }
    Ok(())
}

fn record_last_quote(code: &str, price: f64, source_at: DateTime<Utc>) {
    if let Ok(mut cache) = last_t0_quotes().lock() {
        if cache.len() >= LAST_T0_QUOTE_CACHE_MAX {
            cache.clear();
        }
        cache.insert(code.to_string(), (price, source_at));
    }
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
        crate::magic_compat::Exchange::Shanghai => 1,
        crate::magic_compat::Exchange::Shenzhen => 0,
        crate::magic_compat::Exchange::Beijing => 2,
    };
    Ok(T0RequestIdentity {
        market,
        instrument: identity.instrument().clone(),
        canonical_code: identity.canonical_code().to_owned(),
    })
}

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
pub fn five_minute_from_raw(
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
    // 2026-08-12 探针确认: TDX 盘中流在午后开盘时多发一根 13:00:00 快照 bar
    // (open=13:00 瞬间, vol 仅竞价量 62300), 已收盘数据无此 bar (8/11: 11:30 →
    // 13:05 直接衔接)。13:00-13:05 完整窗口由 13:05 bar 承载 (vol=579800),
    // 丢弃 13:00 后今日 bars 与 trading_slots() 48 槽位对齐, 且历史日比较
    // (day_bars.len() >= today_bars.len()) 不受 25 根午后 vs 24 根的错位影响。
    let afternoon_open_snapshot = NaiveTime::from_hms_opt(13, 0, 0).expect("static slot");
    bars.retain(|bar| {
        bar.at.time() != afternoon_open_snapshot
            && (bar.at.date() < today || (bar.at.date() == today && bar.at.time() <= cutoff))
    });
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
    // 2026-08-21 实盘探针确认: TDX 盘中午后 KLINE_5MIN 响应缺 11:30 上午收盘 bar
    // (09:35..11:25 后直接跳 13:05), 收盘后数据补齐为全 48 槽位; 当日 7 只持仓
    // 全同形。属已知数据形态而非断档: 识别条件 = 观测已过午后开盘 && 今日 bar 数
    // 恰为已完成槽位-1 && 槽位 23 (11:30) 实为 13:05。命中则从对齐槽位剔除 11:30
    // 再逐一对齐; 其余场景仍严格对齐 —— 剔除后任意真实断档(缺 10:00/11:25 等)
    // 都会在错位索引处报 five_minute_gap, 不会被形态容错掩盖。
    let local_time = observed_at.with_timezone(&Local).time();
    let afternoon_started = local_time >= NaiveTime::from_hms_opt(13, 5, 0).expect("static slot");
    let completed_slot_count = allowed_slots.iter().filter(|t| **t <= cutoff).count();
    let mut alignment_slots = allowed_slots.clone();
    if afternoon_started
        && today_bars.len() + 1 == completed_slot_count
        && today_bars.get(23).map(|bar| bar.at.time())
            == Some(NaiveTime::from_hms_opt(13, 5, 0).expect("static slot"))
    {
        log::warn!(
            "[T0Evidence][数据形态] code={code} TDX 盘中午后缺 11:30 bar, 剔除该槽位对齐 (已知形态, 非断档)"
        );
        alignment_slots.remove(23);
    }
    for (index, bar) in today_bars.iter().enumerate() {
        if alignment_slots.get(index).copied() != Some(bar.at.time()) {
            return Err(rejection(
                code,
                "five_minute_gap",
                format!(
                    "date={today} index={index} expected={:?} actual={}",
                    alignment_slots.get(index),
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

#[cfg(feature = "magic-gateway")]
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

#[cfg(feature = "magic-gateway")]
fn evidence_for_quote(
    client: &TdxHqClient,
    identity: &T0RequestIdentity,
    quote: SecurityQuote,
    requested_at: DateTime<Utc>,
    clock: Option<DateTime<Utc>>,
    time_untrustworthy: &mut bool,
) -> std::result::Result<ValidatedT0Evidence, MagicTdxT0Rejection> {
    let code = identity.instrument.code().to_owned();
    let quote_received_at = clock.unwrap_or_else(Utc::now);
    let source_at = source_time(&quote.servertime, quote_received_at).map_err(|mut error| {
        error.code.clone_from(&code);
        error
    })?;
    // BR-231: normalize 先行（纯本地校验, 无网络），随后只按
    // provider source_at 执行五秒 freshness 门。价格仅在通过后记入
    // 诊断 cache，不放宽时效；网络调用（daily/minute）仍在该门后。
    let normalized_quote = normalize_quote(&code, &quote)?;
    // 2026-08-21 用户指令「拿不到时间先跳过, 标注时间不可信」: TDX servertime
    // 生产实测滞后墙钟 14-27s (2026-08-21 全天), 5s 门全量硬拒致做T 0 records。
    // 放宽**仅 age 门**: stale → 置 time_untrustworthy + WARN 继续; future_time
    // (损坏) 仍是硬错误, 绝不放行。
    match validate_quote_freshness(
        &code,
        source_at,
        quote_received_at,
        Some(normalized_quote.price),
    ) {
        Ok(()) => {}
        Err(rejection) if rejection.detail.starts_with("future_time") => return Err(rejection),
        Err(rejection) => {
            *time_untrustworthy = true;
            log::warn!(
                "[T0Evidence][时间不可信] code={code} 跳过 entry age 门: {} — 推送须标注",
                rejection.detail
            );
        }
    }
    // 2026-08-12 实测: TDX 主站 KLINE_RI_K(9) 在 fq_type::NONE 下只返回最新
    // 1 根日K (count=40/800 均如此), 生产 8/11-8/12 全天 settled_daily
    // actual=0 → 做T 证据 0 records。KLINE_DAILY(4) + NONE (不复权) 返回
    // 完整 40 根 (探针: 40/800 根, 2026-06-17..2026-08-12), 语义不变。
    let daily_raw = client
        .get_security_bars(KLINE_DAILY, quote.market, &code, 0, 40, fq_type::NONE)
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
    time_untrustworthy: bool,
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
        // BR-231 批次完成二次门：quote 在进入后续日线/分钟线采集
        // 前即使曾通过，完成时也必须仍处于 `0..=5s`。缓存和价格
        // 变化不参与本门；未来记录 fail-closed 且不更新缓存。
        // 2026-08-21 用户指令「拿不到时间先跳过, 标注时间不可信」:
        // time_untrustworthy=true 时 stale 记录放行 (WARN 出声), future_time
        // (损坏) 永远硬拒; false (测试/严格路径) 保持原 fail-closed 语义。
        let completion_age = observed_at.signed_duration_since(record.source_at);
        if record.source_at > observed_at {
            rejections.push(rejection(
                &record.code,
                "quote_stale",
                format!(
                    "completion_future_time age_ms={} max_secs={T0_QUOTE_MAX_AGE_SECS}",
                    completion_age.num_milliseconds()
                ),
                true,
            ));
            continue;
        }
        if completion_age > chrono::Duration::seconds(T0_QUOTE_MAX_AGE_SECS) {
            if !time_untrustworthy {
                rejections.push(rejection(
                    &record.code,
                    "quote_stale",
                    format!(
                        "completion_stale age_ms={} max_secs={T0_QUOTE_MAX_AGE_SECS}",
                        completion_age.num_milliseconds()
                    ),
                    true,
                ));
                continue;
            }
            log::warn!(
                "[T0Evidence][时间不可信] code={} 跳过 completion age 门: stale \
                 age_ms={} max_secs={T0_QUOTE_MAX_AGE_SECS} — 推送须标注",
                record.code,
                completion_age.num_milliseconds()
            );
        }
        fresh_records.push(record);
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
        provider: ProviderId::Tdx,
        source: "magic_tdx_t0".to_owned(),
        requested_at,
        source_at,
        observed_at,
        batch_id,
        records,
        rejections,
        time_untrustworthy,
    })
}

/// 生产入口: 以当前墙钟作为观测时刻。
#[cfg(feature = "magic-gateway")]
pub fn fetch_magic_tdx_t0_batch(
    codes: &[String],
    requested_at: DateTime<Utc>,
) -> Result<MagicTdxT0Batch> {
    fetch_magic_tdx_t0_batch_with_clock(codes, requested_at, None)
}

/// 回放入口: 注入观测时钟 (None = 墙钟)。
///
/// `source_time` 用观测时刻的**日期**解码 TDX servertime (HH:MM:SS 无日期),
/// freshness 门也用观测时刻计算 age — 回放周五历史数据时必须把时钟注入
/// 到周五盘中/收盘时刻, 否则周六墙钟会把周五收盘快照判定为 quote_stale
/// (age≈9.5h) 或未来时间。生产路径传 None, 行为与注入前完全一致。
/// 进程级共享 TDX 行情 client — 连接复用。做T 每 30s tick 全量重连
/// (单次握手实测 150-222ms, 2026-08-11 全天 222 次 batch); 上游
/// connect_to_any 已连接即短路 (v0.6.7), 断线后下次调用自动重连
/// (last_server → PRIMARY), 跨请求共享安全。连接失败时不缓存, 下轮
/// 重建重试 — fail-closed 语义不变。
///
/// pub(super): data_gateway 内共享 — R-12 盘后回测 (historical_bars::fifteen_min_bars)
/// 复用同一连接, 避免盘后 60 只票回测各建一条 TCP。
#[cfg(feature = "magic-gateway")]
pub(super) fn cached_tdx_hq_client() -> Result<Arc<TdxHqClient>> {
    static CACHE: OnceLock<Mutex<Option<Arc<TdxHqClient>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(client) = guard.as_ref() {
        return Ok(client.clone());
    }
    let client = Arc::new(TdxHqClient::new());
    client
        .connect_to_any(Some(5.0))
        .map_err(|error| anyhow!("magic-tdx T0 connect failed: {error}"))?;
    *guard = Some(client.clone());
    Ok(client)
}

#[cfg(feature = "magic-gateway")]
pub fn fetch_magic_tdx_t0_batch_with_clock(
    codes: &[String],
    requested_at: DateTime<Utc>,
    clock: Option<DateTime<Utc>>,
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
    let client = cached_tdx_hq_client()?;
    let request = identities
        .iter()
        .map(|identity| (identity.market, identity.canonical_code.as_str()))
        .collect::<Vec<_>>();
    let quotes = client
        .get_security_quotes(&request)
        .map_err(|error| anyhow!("magic-tdx T0 quote batch failed: {error}"))?;
    validate_quote_identities(&identities, &quotes)?;
    // BR-230: quote source_time 逐代码隔离 — 单只 quote 缺 servertime
    // (上游 magic-tdx-rs 契约缺口) 只跳过该代码 (显式 rejection),
    // 不再整批失败; 批次 source_at = 有效 quote 的最小值。
    let mut quote_times: Vec<(String, DateTime<Utc>)> = Vec::with_capacity(quotes.len());
    let mut quote_rejections = Vec::new();
    let quote_observed_at = clock.unwrap_or_else(Utc::now);
    for (identity, quote) in identities.iter().zip(&quotes) {
        match source_time(&quote.servertime, quote_observed_at) {
            Ok(time) => quote_times.push((identity.instrument.code().to_string(), time)),
            Err(mut error) => {
                error.code = identity.instrument.code().to_string();
                quote_rejections.push(error);
            }
        }
    }
    let source_at = quote_times
        .iter()
        .map(|(_, time)| *time)
        .min()
        .ok_or_else(|| {
            anyhow!(
                "magic-tdx T0 quote batch has no valid source time: all quotes lack server time ({} codes rejected)",
                quote_rejections.len()
            )
        })?;
    let skip_codes: std::collections::HashSet<String> = quote_rejections
        .iter()
        .map(|rejection| rejection.code.clone())
        .collect();
    let mut records = Vec::new();
    let mut rejections = Vec::new();
    // 2026-08-21 用户指令「拿不到时间先跳过, 标注时间不可信」: TDX servertime
    // 滞后墙钟 14-27s → entry 门放宽; 标志贯穿 finalize (completion 门同步放宽)。
    let mut time_untrustworthy = false;
    for (identity, quote) in identities.iter().zip(quotes) {
        if skip_codes.contains(identity.instrument.code()) {
            continue;
        }
        match evidence_for_quote(
            &client,
            identity,
            quote,
            requested_at,
            clock,
            &mut time_untrustworthy,
        ) {
            Ok(record) => records.push(record),
            Err(error) => rejections.push(error),
        }
    }
    rejections.extend(quote_rejections);
    let observed_at = clock.unwrap_or_else(Utc::now);
    let batch = finalize_t0_batch(
        requested_at,
        source_at,
        observed_at,
        &identities,
        records,
        rejections,
        time_untrustworthy,
    )?;
    if batch.time_untrustworthy {
        log::warn!(
            "[T0Evidence][时间不可信] 服务端整批放宽 age 门: source_at={} \
             observed_at={} records={} — 推送必须带标注",
            batch.source_at,
            batch.observed_at,
            batch.records.len()
        );
    }
    Ok(batch)
}

#[cfg(test)]
#[cfg(feature = "magic-gateway")]
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
    fn stale_first_sighting_is_rejected_without_cache_pollution() {
        // BR-231 / AGENTS §2.4: 没有 cache anchor 不能把过期 quote 变成
        // “首次播种”证据；拒绝路径也不得污染诊断 cache。
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 1).unwrap();
        let source_at = Utc.with_ymd_and_hms(2026, 7, 23, 1, 59, 55).unwrap(); // age=6s
        let code = "TEST_CODE_SEED_1";

        let error = validate_quote_freshness(code, source_at, observed_at, Some(10.0))
            .expect_err("age=6s must fail the five-second realtime quote gate");
        assert_eq!(error.reason_code, "quote_stale");
        assert!(error.detail.contains("max_secs=5"));
        assert!(!last_t0_quotes()
            .lock()
            .expect("quote cache lock")
            .contains_key(code));
    }

    #[test]
    fn movement_evidence_never_relaxes_five_second_gate_or_pollutes_cache() {
        // 先以 age=5s 的真实新鲜 quote 建立诊断记录。
        let code = "TEST_CODE_BR231_1";
        let source_1 = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 10).unwrap();
        let observed_1 = source_1 + chrono::Duration::seconds(5);
        validate_quote_freshness(code, source_1, observed_1, Some(12.34))
            .expect("age=5s is inside the realtime quote gate");

        // 价格和 servertime 都前进，但该快照 age=6s，仍必须拒绝。
        let source_2 = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 30).unwrap();
        let observed_2 = source_2 + chrono::Duration::seconds(6);
        let changed = validate_quote_freshness(code, source_2, observed_2, Some(12.35))
            .expect_err("price/time movement cannot admit age=6s quote");
        assert_eq!(changed.reason_code, "quote_stale");

        // 拒绝数据不得改写上一个已准入证据。
        let cached = last_t0_quotes()
            .lock()
            .expect("quote cache lock")
            .get(code)
            .copied();
        assert_eq!(cached, Some((12.34, source_1)));

        // 价格不变而 servertime 前进也不构成五秒外例外。
        let source_3 = source_2 + chrono::Duration::seconds(20);
        let observed_3 = source_3 + chrono::Duration::seconds(6);
        let unchanged = validate_quote_freshness(code, source_3, observed_3, Some(12.34))
            .expect_err("timestamp movement cannot admit age=6s quote");
        assert_eq!(unchanged.reason_code, "quote_stale");
        assert_eq!(
            last_t0_quotes()
                .lock()
                .expect("quote cache lock")
                .get(code)
                .copied(),
            Some((12.34, source_1))
        );
    }

    #[test]
    fn stale_quote_without_price_history_stays_rejected_and_does_not_pollute_cache() {
        // 无价格参数不会改变时间门：age=30s 仍显式拒绝，
        // 且拒绝路径不写入诊断 cache。
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 1, 0).unwrap();
        let stale_source_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 30).unwrap();
        let result =
            validate_quote_freshness("TEST_CODE_BR231_2", stale_source_at, observed_at, None);
        assert_eq!(result.unwrap_err().reason_code, "quote_stale");
    }

    #[test]
    fn far_stale_quote_is_rejected_against_five_second_limit() {
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 6, 0).unwrap();
        let source_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 55).unwrap(); // age=305s
        let result =
            validate_quote_freshness("TEST_CODE_FAR_STALE_1", source_at, observed_at, Some(10.0));
        let err = result.unwrap_err();
        assert_eq!(err.reason_code, "quote_stale");
        assert!(err.detail.contains("max_secs=5"));
    }

    #[test]
    fn future_quote_time_is_rejected() {
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let source_at = observed_at + chrono::Duration::seconds(3);
        let result =
            validate_quote_freshness("TEST_CODE_FUTURE_1", source_at, observed_at, Some(10.0));
        let err = result.unwrap_err();
        assert_eq!(err.reason_code, "quote_stale");
        assert!(err.detail.contains("future_time"));
    }

    #[test]
    fn quote_five_seconds_and_one_millisecond_old_is_rejected() {
        let source_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let observed_at = source_at + chrono::Duration::milliseconds(5_001);
        let code = "TEST_CODE_OVER_FIVE_SECONDS_1";

        let error = validate_quote_freshness(code, source_at, observed_at, Some(10.0))
            .expect_err("5.001s must not be truncated to the accepted five-second boundary");
        assert_eq!(error.reason_code, "quote_stale");
        assert!(error.detail.contains("age_ms=5001"));
        assert!(!last_t0_quotes()
            .lock()
            .expect("quote cache lock")
            .contains_key(code));
    }

    #[test]
    fn subsecond_future_quote_time_is_rejected() {
        let observed_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let source_at = observed_at + chrono::Duration::milliseconds(1);

        let error = validate_quote_freshness(
            "TEST_CODE_FUTURE_SUBSECOND_1",
            source_at,
            observed_at,
            Some(10.0),
        )
        .expect_err("even a 1ms future provider time must fail closed");
        assert_eq!(error.reason_code, "quote_stale");
        assert!(error.detail.contains("future_time"));
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
            false,
        )
        .unwrap();
        let second = finalize_t0_batch(
            requested_at,
            source_at,
            observed_at,
            std::slice::from_ref(&identity),
            vec![record.clone()],
            Vec::new(),
            false,
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
            false,
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
            false,
        )
        .unwrap();
        let second = finalize_t0_batch(
            requested_at,
            source_at,
            observed_at,
            &[identity],
            Vec::new(),
            vec![second_rejection],
            false,
        )
        .unwrap();

        assert!(first.records.is_empty());
        assert_eq!(first.rejections.len(), 1);
        assert_ne!(first.batch_id, second.batch_id);
    }

    #[test]
    fn completion_freshness_reapplies_five_second_gate() {
        // quote 初始接收时可以新鲜，但后续采集耗时使完成时 age=6s，
        // 批次必须排除该记录。
        let requested_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let source_at = requested_at + chrono::Duration::seconds(1);
        let completion_at = source_at + chrono::Duration::milliseconds(5_001);
        let identity = normalized_identity("TEST_CODE_600396").unwrap();
        let record = validated_record(&identity, source_at, requested_at);

        let batch = finalize_t0_batch(
            requested_at,
            source_at,
            completion_at,
            &[identity],
            vec![record],
            Vec::new(),
            false,
        )
        .unwrap();

        assert!(batch.records.is_empty());
        assert_eq!(batch.rejections.len(), 1);
        assert_eq!(batch.rejections[0].reason_code, "quote_stale");
        assert!(batch.rejections[0].detail.contains("completion_stale"));
        assert_eq!(batch.observed_at, completion_at);
        assert_eq!(batch.batch_id.len(), 64);
    }

    #[test]
    fn br243_completion_stale_is_admitted_with_time_untrustworthy_flag() {
        // 2026-08-21 用户指令「拿不到时间先跳过, 标注时间不可信」: 完成时
        // stale 记录在 time_untrustworthy=true 下放行 (WARN), 不置 rejection。
        let requested_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let source_at = requested_at + chrono::Duration::seconds(1);
        let completion_at = source_at + chrono::Duration::milliseconds(5_001);
        let identity = normalized_identity("TEST_CODE_600396").unwrap();
        let record = validated_record(&identity, source_at, requested_at);

        let batch = finalize_t0_batch(
            requested_at,
            source_at,
            completion_at,
            &[identity],
            vec![record.clone()],
            Vec::new(),
            true,
        )
        .unwrap();

        assert_eq!(batch.records.len(), 1);
        assert!(batch.rejections.is_empty());
        assert!(batch.time_untrustworthy);
        assert_eq!(batch.records[0].code, "TEST_CODE_600396");
        assert_eq!(batch.observed_at, completion_at);
    }

    #[test]
    fn br243_future_time_is_never_admitted_even_with_flag() {
        // 未来时间 = 损坏 ≠ 延迟: time_untrustworthy=true 也不能放行 future。
        let requested_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let observed_at = requested_at + chrono::Duration::seconds(2);
        let record_source_at = observed_at + chrono::Duration::milliseconds(1);
        let identity = normalized_identity("TEST_CODE_600396").unwrap();
        let record = validated_record(&identity, record_source_at, observed_at);

        let batch = finalize_t0_batch(
            requested_at,
            requested_at + chrono::Duration::seconds(1),
            observed_at,
            &[identity],
            vec![record],
            Vec::new(),
            true,
        )
        .unwrap();

        assert!(batch.records.is_empty());
        assert_eq!(batch.rejections.len(), 1);
        assert_eq!(batch.rejections[0].reason_code, "quote_stale");
        assert!(batch.rejections[0]
            .detail
            .contains("completion_future_time"));
    }

    #[test]
    fn completion_freshness_rejects_subsecond_future_record_time() {
        let requested_at = Utc.with_ymd_and_hms(2026, 7, 23, 2, 0, 0).unwrap();
        let observed_at = requested_at + chrono::Duration::seconds(2);
        let record_source_at = observed_at + chrono::Duration::milliseconds(1);
        let identity = normalized_identity("TEST_CODE_600396").unwrap();
        let record = validated_record(&identity, record_source_at, observed_at);

        let batch = finalize_t0_batch(
            requested_at,
            requested_at + chrono::Duration::seconds(1),
            observed_at,
            &[identity],
            vec![record],
            Vec::new(),
            false,
        )
        .unwrap();

        assert!(batch.records.is_empty());
        assert_eq!(batch.rejections.len(), 1);
        assert_eq!(batch.rejections[0].reason_code, "quote_stale");
        assert!(batch.rejections[0]
            .detail
            .contains("completion_future_time"));
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
    fn afternoon_open_snapshot_bar_13_00_is_dropped_and_gap_check_stays_aligned() {
        let observed_at = Local
            .with_ymd_and_hms(2026, 7, 23, 14, 5, 0)
            .single()
            .expect("fixture time")
            .with_timezone(&Utc);

        // TDX 盘中流: 午后开盘多一根 13:00:00 快照 bar (仅竞价量), 已收盘日无此 bar。
        // 构造与真实盘中流一致的全 48 槽位数据 (今天 + 3 个历史交易日)。
        let today = observed_at.with_timezone(&Local).date_naive();
        let mut live = Vec::new();
        let mut date = today;
        for _ in 0..=T0_HISTORY_MIN_SESSIONS {
            for time in trading_slots() {
                live.push(MagicTdxT0FiveMinuteBar {
                    at: date.and_time(time),
                    open: 10.0,
                    high: 10.2,
                    low: 9.8,
                    close: 10.0,
                    volume: 1_000.0,
                    amount: 10_000.0,
                });
            }
            date = crate::calendar::prev_trading_day(date);
        }
        let snapshot_at = today
            .and_hms_opt(13, 0, 0)
            .expect("afternoon open snapshot slot");
        live.push(MagicTdxT0FiveMinuteBar {
            at: snapshot_at,
            open: 10.0,
            high: 10.1,
            low: 9.95,
            close: 10.0,
            volume: 62_300.0,
            amount: 62_300.0 * 10.0,
        });

        let validated =
            validate_five_minute_bars("TEST_CODE_600396", live, observed_at).expect("live bars ok");
        assert!(
            validated
                .iter()
                .all(|bar| bar.at.time() != NaiveTime::from_hms_opt(13, 0, 0).unwrap()),
            "13:00 快照 bar 必须被丢弃"
        );
        let today_bars: Vec<_> = validated
            .iter()
            .filter(|bar| bar.at.date() == today)
            .collect();
        // 丢弃后今日 bars 与 trading_slots() 槽位逐一对齐 (five_minute_gap 检查)。
        let slots = trading_slots();
        assert_eq!(today_bars.len(), 37); // 24 上午 (9:35-11:30) + 13 午后 (13:05-14:05)
        for (index, bar) in today_bars.iter().enumerate() {
            assert_eq!(slots.get(index).copied(), Some(bar.at.time()));
        }
    }

    #[test]
    fn missing_1130_intraday_shape_is_admitted_after_afternoon_open() {
        // 2026-08-21 实盘: TDX 盘中午后 KLINE_5MIN 响应缺 11:30 bar
        // (09:35..11:25 直接跳 13:05), 收盘后补齐为全 48 槽位; 当日持仓全同形。
        // 构造观测 14:48 (cutoff 14:45) 的盘中形态: 今日 44 根 (23 上午 + 21 午后),
        // 历史 3 个完整交易日 → 剔除 11:30 槽位后逐一对齐通过。
        let observed_at = Local
            .with_ymd_and_hms(2026, 8, 21, 14, 48, 0)
            .single()
            .expect("fixture time")
            .with_timezone(&Utc);
        let today = observed_at.with_timezone(&Local).date_naive();
        let bar = |date: NaiveDate, time: NaiveTime| MagicTdxT0FiveMinuteBar {
            at: date.and_time(time),
            open: 10.0,
            high: 10.2,
            low: 9.8,
            close: 10.0,
            volume: 1_000.0,
            amount: 10_000.0,
        };
        let mut live = Vec::new();
        let mut date = crate::calendar::prev_trading_day(today);
        for _ in 0..T0_HISTORY_MIN_SESSIONS {
            for time in trading_slots() {
                live.push(bar(date, time));
            }
            date = crate::calendar::prev_trading_day(date);
        }
        let cutoff = completed_slot_cutoff(observed_at); // 14:45
        let missing_1130 = NaiveTime::from_hms_opt(11, 30, 0).expect("static slot");
        let today_bars_count = trading_slots()
            .iter()
            .filter(|time| **time <= cutoff && **time != missing_1130)
            .count();
        assert_eq!(today_bars_count, 44, "fixture sanity: 45 已完成槽位 - 11:30");
        for time in trading_slots()
            .into_iter()
            .filter(|time| *time <= cutoff && *time != missing_1130)
        {
            live.push(bar(today, time));
        }

        let validated = validate_five_minute_bars("TEST_CODE_600396", live, observed_at)
            .expect("缺 11:30 的盘中形态必须被接收");
        let today_validated: Vec<_> = validated
            .iter()
            .filter(|b| b.at.date() == today)
            .collect();
        assert_eq!(today_validated.len(), 44);
        assert!(
            today_validated
                .iter()
                .all(|b| b.at.time() != missing_1130),
            "11:30 不在 TDX 盘中响应里, 不得凭空出现在输出中"
        );
    }

    #[test]
    fn real_morning_gap_is_still_rejected_despite_1130_tolerance() {
        // 形态容错只覆盖"仅缺 11:30"。缺 10:00 时今日 bar 数同样少 1、槽位 23
        // 同样为 13:05 (误判形态命中), 但剔除 11:30 后错位索引处必须仍报
        // five_minute_gap —— 真实断档不得被形态容错掩盖。
        let observed_at = Local
            .with_ymd_and_hms(2026, 8, 21, 14, 48, 0)
            .single()
            .expect("fixture time")
            .with_timezone(&Utc);
        let today = observed_at.with_timezone(&Local).date_naive();
        let bar = |date: NaiveDate, time: NaiveTime| MagicTdxT0FiveMinuteBar {
            at: date.and_time(time),
            open: 10.0,
            high: 10.2,
            low: 9.8,
            close: 10.0,
            volume: 1_000.0,
            amount: 10_000.0,
        };
        let mut live = Vec::new();
        let mut date = crate::calendar::prev_trading_day(today);
        for _ in 0..T0_HISTORY_MIN_SESSIONS {
            for time in trading_slots() {
                live.push(bar(date, time));
            }
            date = crate::calendar::prev_trading_day(date);
        }
        let cutoff = completed_slot_cutoff(observed_at);
        let missing_1000 = NaiveTime::from_hms_opt(10, 0, 0).expect("static slot");
        for time in trading_slots()
            .into_iter()
            .filter(|time| *time <= cutoff && *time != missing_1000)
        {
            live.push(bar(today, time));
        }

        let error = validate_five_minute_bars("TEST_CODE_600396", live, observed_at)
            .expect_err("缺 10:00 是真实断档, 必须拒绝");
        assert_eq!(error.reason_code, "five_minute_gap");
        assert!(
            error.detail.contains("index=5"),
            "错位应指向 10:05 槽: {}",
            error.detail
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
