//! BR-164 provider-neutral T0 evidence boundary for reverse-T observation plans.
//!
//! This module preserves provider timestamps and rejects incomplete or invalid
//! evidence. It never substitutes another data source or manufactures missing
//! price, volume, time, or order-book values.
//!
//! Business rules: BR-092, BR-151, BR-153, BR-171, BR-187.

use crate::market_domain::{InstrumentId, ProviderId};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct T0BookLevel {
    pub price: f64,
    pub volume: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct T0Quote {
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct T0DailyBar {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct T0FiveMinuteBar {
    pub at: NaiveDateTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct T0Evidence {
    pub instrument: InstrumentId,
    pub code: String,
    pub requested_at: DateTime<Utc>,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub quote: T0Quote,
    pub settled_daily: Vec<T0DailyBar>,
    pub completed_five_minute: Vec<T0FiveMinuteBar>,
    pub intraday_average_price: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct T0Rejection {
    pub code: String,
    pub reason_code: String,
    pub detail: String,
    pub retryable: bool,
}

impl T0Rejection {
    /// Convert the wire-owned reason code to the closed set accepted by
    /// `GatewayError`. Unknown values fail closed as invalid evidence.
    pub fn gateway_reason_code(&self) -> &'static str {
        match self.reason_code.as_str() {
            "quote_stale" => "quote_stale",
            "daily_insufficient" => "daily_insufficient",
            "daily_duplicate" => "daily_duplicate",
            "daily_invalid" => "daily_invalid",
            "manual_confirmation_contract_unavailable" => {
                "manual_confirmation_contract_unavailable"
            }
            "five_minute_duplicate" => "five_minute_duplicate",
            "five_minute_time_invalid" => "five_minute_time_invalid",
            "five_minute_invalid" => "five_minute_invalid",
            "five_minute_today_insufficient" => "five_minute_today_insufficient",
            "five_minute_gap" => "five_minute_gap",
            "history_slots_insufficient" => "history_slots_insufficient",
            _ => "invalid_evidence",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct T0Batch {
    pub provider: ProviderId,
    pub source: String,
    pub requested_at: DateTime<Utc>,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub batch_id: String,
    pub records: Vec<T0Evidence>,
    pub rejections: Vec<T0Rejection>,
    /// 2026-08-21 用户指令「拿不到时间先跳过, 标注时间不可信」:
    /// 桥往返 + server 侧整批组装 (7 codes × 4 TDX 调用) 耗时 11-55s,
    /// 且 TDX servertime 实测滞后墙钟 14-27s (2026-08-21 全天) — 服务端
    /// age 门放宽时置 true; consumer 侧按 record age 复查亦置 true。
    /// 推送方必须标注「时间不可信」。未来时间 (损坏) 仍硬拒, 不置此位。
    pub time_untrustworthy: bool,
}

fn rejection(
    code: &str,
    reason_code: &'static str,
    detail: impl Into<String>,
    retryable: bool,
) -> T0Rejection {
    T0Rejection {
        code: code.to_string(),
        reason_code: reason_code.to_owned(),
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
) -> std::result::Result<(), T0Rejection> {
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

pub fn validate_settled_daily(
    code: &str,
    mut bars: Vec<T0DailyBar>,
) -> std::result::Result<Vec<T0DailyBar>, T0Rejection> {
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
                    the settled-daily records do not expose an exact \
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
    mut bars: Vec<T0FiveMinuteBar>,
    observed_at: DateTime<Utc>,
) -> std::result::Result<Vec<T0FiveMinuteBar>, T0Rejection> {
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

    let mut by_date = BTreeMap::<NaiveDate, Vec<&T0FiveMinuteBar>>::new();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_domain::{AssetClass, Exchange};
    use chrono::TimeZone;

    fn book(price: f64) -> [T0BookLevel; 5] {
        std::array::from_fn(|index| T0BookLevel {
            price: price + index as f64 * 0.25,
            volume: 1_000.0 + index as f64,
        })
    }

    #[test]
    fn t0_batch_wire_round_trip() {
        let requested_at = Utc.with_ymd_and_hms(2099, 1, 2, 1, 29, 59).unwrap();
        let source_at = requested_at + chrono::Duration::seconds(1);
        let observed_at = source_at + chrono::Duration::seconds(7);
        let quote = T0Quote {
            price: 10.25,
            last_close: 10.0,
            open: 10.1,
            high: 10.3,
            low: 9.9,
            volume: 123_456.0,
            amount: 1_234_567.0,
            bids: book(10.25),
            asks: book(10.50),
        };
        let batch = T0Batch {
            provider: ProviderId::Tdx,
            source: "magic_tdx_t0".to_owned(),
            requested_at,
            source_at,
            observed_at,
            batch_id: "TEST_CODE_t0_batch".to_owned(),
            records: vec![T0Evidence {
                instrument: InstrumentId::new(
                    Exchange::Shanghai,
                    "TEST_CODE_600000",
                    AssetClass::Equity,
                )
                .unwrap(),
                code: "TEST_CODE_600000".to_owned(),
                requested_at,
                source_at,
                observed_at,
                batch_id: "TEST_CODE_t0_batch".to_owned(),
                quote: quote.clone(),
                settled_daily: Vec::new(),
                completed_five_minute: Vec::new(),
                intraday_average_price: 10.22,
            }],
            rejections: vec![T0Rejection {
                code: "TEST_CODE_600001".to_owned(),
                reason_code: "quote_stale".to_owned(),
                detail: "TEST_CODE stale quote".to_owned(),
                retryable: true,
            }],
            time_untrustworthy: true,
        };

        let encoded = serde_json::to_vec(&batch).expect("serialize T0 batch");
        let decoded: T0Batch = serde_json::from_slice(&encoded).expect("deserialize T0 batch");

        assert_eq!(decoded.batch_id, batch.batch_id);
        assert_eq!(decoded.source_at, source_at);
        assert_eq!(decoded.records[0].quote.price, quote.price);
        assert_eq!(decoded.records[0].quote.bids, quote.bids);
        assert_eq!(decoded.records[0].quote.asks, quote.asks);
        assert_eq!(decoded.rejections, batch.rejections);
        assert_eq!(decoded.rejections[0].gateway_reason_code(), "quote_stale");
        assert!(decoded.time_untrustworthy);
        assert_eq!(decoded, batch);
    }
}
