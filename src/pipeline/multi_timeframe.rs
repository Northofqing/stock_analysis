//! 多周期下钻：当日线产生买入信号时，去 60min/15min K 线寻找精准入场点。
//!
//! 触发条件由调用方判定（评分≥60 / BB+MACD BottomBuy/UptrendStart / RSI Buy 任一）。
//! 本模块只负责在 blocking 线程池抓取 60min+15min K 线并跑入场点评估，返回可注入
//! AI prompt 的 Markdown 片段。数据不足返回 `Ok(None)`，来源失败显式返回 `Err`。

use chrono::{DateTime, FixedOffset, NaiveDate, Timelike, Utc};

use crate::data_gateway::{HistoricalBarsGateway, MarketCapabilitiesGateway, MarketMinutePoint};
use crate::strategy::MinuteBar;

const HISTORY_TRADING_DAYS: usize = 10;

pub(super) async fn fetch_multi_timeframe_section(code: &str) -> Result<Option<String>, String> {
    let storage_code = code.to_owned();
    let daily = tokio::task::spawn_blocking(move || {
        HistoricalBarsGateway::new().required_daily_bars(&storage_code, HISTORY_TRADING_DAYS)
    })
    .await
    .map_err(|error| format!("[{code}] 日线日期任务失败: {error}"))?
    .map_err(|error| format!("[{code}] 日线日期不可用: {error}"))?;
    let dates = daily
        .records()
        .iter()
        .map(|bar| bar.date)
        .collect::<Vec<_>>();
    if dates.len() != HISTORY_TRADING_DAYS {
        return Err(format!(
            "[{code}] 日线日期批次不完整: expected={HISTORY_TRADING_DAYS} actual={}",
            dates.len()
        ));
    }

    let gateway = MarketCapabilitiesGateway::new();
    let mut points = Vec::with_capacity(HISTORY_TRADING_DAYS * 240 + 240);
    for date in dates {
        let batch = gateway
            .minute_data(code, Some(date))
            .await
            .map_err(|error| format!("[{code}] {date} 1min K线不可用: {error}"))?;
        points.extend_from_slice(batch.records());
    }

    // Current-session points are required for an intraday entry decision. The
    // five-second Gateway gate prevents yesterday's cache from masquerading as
    // a current signal.
    let current = gateway
        .minute_data(code, None)
        .await
        .map_err(|error| format!("[{code}] 当前 1min K线不可用: {error}"))?;
    points.extend_from_slice(current.records());
    points.sort_by_key(|point| point.minute_at);
    if let Some(duplicate) = points
        .windows(2)
        .find(|pair| pair[0].minute_at == pair[1].minute_at)
    {
        return Err(format!(
            "[{code}] 1min source batches overlap at {}",
            duplicate[0].minute_at.to_rfc3339()
        ));
    }

    let h1 = aggregate_completed_bars(code, &points, 60)?;
    let m15 = aggregate_completed_bars(code, &points, 15)?;
    resolve_multi_timeframe_results(code, Ok(h1), Ok(m15))
}

fn resolve_multi_timeframe_results(
    code: &str,
    h1_result: Result<Vec<MinuteBar>, String>,
    m15_result: Result<Vec<MinuteBar>, String>,
) -> Result<Option<String>, String> {
    let h1 = h1_result.map_err(|error| format!("[{code}] 60min K线不可用: {error}"))?;
    let m15 = m15_result.map_err(|error| format!("[{code}] 15min K线不可用: {error}"))?;
    Ok(resolve_multi_timeframe_section(&h1, &m15))
}

fn resolve_multi_timeframe_section(h1: &[MinuteBar], m15: &[MinuteBar]) -> Option<String> {
    let assess = crate::strategy::assess_multi_timeframe_entry(h1, m15);
    let section = assess.to_prompt_section();
    if section.trim().is_empty() {
        None
    } else {
        Some(section)
    }
}

#[derive(Debug)]
struct PendingBar {
    key: (NaiveDate, u8, u32),
    timestamp: DateTime<Utc>,
    open: f64,
    close: f64,
    high: f64,
    low: f64,
    volume: f64,
}

fn aggregate_completed_bars(
    code: &str,
    points: &[MarketMinutePoint],
    interval_minutes: u32,
) -> Result<Vec<MinuteBar>, String> {
    if !matches!(interval_minutes, 15 | 60) {
        return Err(format!(
            "[{code}] unsupported intraday aggregation interval {interval_minutes}"
        ));
    }
    if points.is_empty() {
        return Err(format!("[{code}] 1min source series is empty"));
    }
    let shanghai = FixedOffset::east_opt(8 * 60 * 60)
        .ok_or_else(|| "Asia/Shanghai fixed offset unavailable".to_owned())?;
    let mut output = Vec::new();
    let mut pending: Option<PendingBar> = None;
    let mut previous: Option<(NaiveDate, f64)> = None;

    for point in points {
        if point.code != code {
            return Err(format!(
                "[{code}] 1min source identity mismatch: {}",
                point.code
            ));
        }
        let local = point.minute_at.with_timezone(&shanghai);
        let (session, elapsed) = session_elapsed(local.hour(), local.minute())
            .ok_or_else(|| format!("[{code}] off-session minute {}", local.to_rfc3339()))?;
        let adjusted = elapsed.saturating_sub(1);
        let bucket = adjusted / interval_minutes;
        let key = (local.date_naive(), session, bucket);

        let delta_volume = match previous {
            Some((previous_date, previous_quantity)) if previous_date == local.date_naive() => {
                let delta = point.cumulative_quantity - previous_quantity;
                if !delta.is_finite() || delta < 0.0 {
                    return Err(format!(
                        "[{code}] cumulative minute volume regressed at {}",
                        local.to_rfc3339()
                    ));
                }
                delta
            }
            _ => point.cumulative_quantity,
        };
        previous = Some((local.date_naive(), point.cumulative_quantity));

        if pending.as_ref().is_some_and(|bar| bar.key != key) {
            let completed = pending
                .take()
                .ok_or_else(|| format!("[{code}] missing pending intraday bar"))?;
            if is_completed_bucket(completed.timestamp, interval_minutes, &shanghai) {
                output.push(finish_bar(completed, &shanghai));
            }
        }

        match pending.as_mut() {
            Some(bar) => {
                bar.close = point.price;
                bar.high = bar.high.max(point.price);
                bar.low = bar.low.min(point.price);
                bar.volume += delta_volume;
                bar.timestamp = point.minute_at;
            }
            None => {
                pending = Some(PendingBar {
                    key,
                    timestamp: point.minute_at,
                    open: point.price,
                    close: point.price,
                    high: point.price,
                    low: point.price,
                    volume: delta_volume,
                });
            }
        }
    }
    if let Some(completed) = pending {
        if is_completed_bucket(completed.timestamp, interval_minutes, &shanghai) {
            output.push(finish_bar(completed, &shanghai));
        }
    }
    if output.is_empty() {
        return Err(format!(
            "[{code}] no completed {interval_minutes}min bars available"
        ));
    }
    Ok(output)
}

fn session_elapsed(hour: u32, minute: u32) -> Option<(u8, u32)> {
    let clock = hour * 60 + minute;
    let morning_start = 9 * 60 + 30;
    let morning_end = 11 * 60 + 30;
    let afternoon_start = 13 * 60;
    let afternoon_end = 15 * 60;
    if (morning_start..=morning_end).contains(&clock) {
        Some((0, clock - morning_start))
    } else if (afternoon_start..=afternoon_end).contains(&clock) {
        Some((1, clock - afternoon_start))
    } else {
        None
    }
}

fn is_completed_bucket(
    timestamp: DateTime<Utc>,
    interval_minutes: u32,
    shanghai: &FixedOffset,
) -> bool {
    let local = timestamp.with_timezone(shanghai);
    session_elapsed(local.hour(), local.minute())
        .is_some_and(|(_, elapsed)| elapsed > 0 && elapsed % interval_minutes == 0)
}

fn finish_bar(pending: PendingBar, shanghai: &FixedOffset) -> MinuteBar {
    MinuteBar {
        timestamp: pending
            .timestamp
            .with_timezone(shanghai)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        open: pending.open,
        close: pending.close,
        high: pending.high,
        low: pending.low,
        volume: pending.volume,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use crate::magic_compat::ProviderId;

    fn bars(count: usize, step_minutes: i64) -> Vec<MinuteBar> {
        let start = chrono::NaiveDate::from_ymd_opt(2026, 7, 16)
            .unwrap()
            .and_hms_opt(9, 30, 0)
            .unwrap();
        (0..count)
            .map(|index| {
                let close = 10.0 + index as f64 * 0.01;
                MinuteBar {
                    timestamp: (start + chrono::Duration::minutes(step_minutes * index as i64))
                        .format("%Y-%m-%d %H:%M")
                        .to_string(),
                    open: close - 0.01,
                    close,
                    high: close + 0.05,
                    low: close - 0.05,
                    volume: 1_000.0 + index as f64 * 10.0,
                }
            })
            .collect()
    }

    #[test]
    fn resolved_multi_timeframe_distinguishes_insufficient_and_present_batches() {
        assert!(resolve_multi_timeframe_section(&bars(29, 60), &bars(20, 15)).is_none());
        let section = resolve_multi_timeframe_section(&bars(40, 60), &bars(30, 15))
            .expect("complete validated bars must render an assessment");
        assert!(section.contains("多周期入场点"));
        assert!(section.contains("命中入场规则:"));
        assert!(section.contains("结论:"));
    }

    #[test]
    fn resolved_results_preserve_source_specific_failures() {
        let h1_error = resolve_multi_timeframe_results(
            "TEST_CODE_000001",
            Err("60min transport".to_owned()),
            Ok(bars(30, 15)),
        )
        .unwrap_err();
        assert!(h1_error.contains("[TEST_CODE_000001] 60min K线不可用"));
        assert!(h1_error.contains("60min transport"));

        let m15_error = resolve_multi_timeframe_results(
            "TEST_CODE_000001",
            Ok(bars(40, 60)),
            Err("15min transport".to_owned()),
        )
        .unwrap_err();
        assert!(m15_error.contains("[TEST_CODE_000001] 15min K线不可用"));
        assert!(m15_error.contains("15min transport"));
    }

    fn minute_point(
        minute_at: DateTime<Utc>,
        price: f64,
        cumulative_quantity: f64,
    ) -> MarketMinutePoint {
        MarketMinutePoint {
            code: "TEST_CODE_000001".to_owned(),
            minute_at,
            price,
            cumulative_quantity,
            cumulative_amount: None,
            source_at: minute_at,
            observed_at: minute_at,
            provider: ProviderId::Tdx,
            batch_id: "TEST_CODE_minute_batch".to_owned(),
        }
    }

    #[test]
    fn br164_minute_points_form_only_completed_session_bars() {
        let shanghai = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let start = shanghai
            .with_ymd_and_hms(2026, 7, 24, 9, 30, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let points = (0..=31)
            .map(|minute| {
                minute_point(
                    start + chrono::Duration::minutes(minute),
                    10.0 + minute as f64 * 0.01,
                    minute as f64 * 100.0,
                )
            })
            .collect::<Vec<_>>();

        let bars = aggregate_completed_bars("TEST_CODE_000001", &points, 15).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].timestamp, "2026-07-24 09:45");
        assert_eq!(bars[1].timestamp, "2026-07-24 10:00");
        assert_eq!(bars[0].open, 10.0);
        assert_eq!(bars[0].close, 10.15);
        assert_eq!(bars[0].volume, 1_500.0);
    }

    #[test]
    fn br164_lunch_break_never_creates_a_cross_session_candle() {
        let shanghai = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let make = |hour, minute, price, quantity| {
            minute_point(
                shanghai
                    .with_ymd_and_hms(2026, 7, 24, hour, minute, 0)
                    .single()
                    .unwrap()
                    .with_timezone(&Utc),
                price,
                quantity,
            )
        };
        let points = vec![
            make(11, 16, 10.0, 1_000.0),
            make(11, 30, 10.1, 1_500.0),
            make(13, 0, 10.2, 1_500.0),
            make(13, 15, 10.3, 2_000.0),
        ];

        let bars = aggregate_completed_bars("TEST_CODE_000001", &points, 15).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].timestamp, "2026-07-24 11:30");
        assert_eq!(bars[1].timestamp, "2026-07-24 13:15");
        assert_eq!(bars[0].close, 10.1);
        assert_eq!(bars[1].open, 10.2);
    }

    #[test]
    fn br168_aggregation_rejects_invalid_request_and_source_identity() {
        let shanghai = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let at = shanghai
            .with_ymd_and_hms(2026, 7, 24, 9, 45, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let point = minute_point(at, 10.0, 100.0);

        let unsupported =
            aggregate_completed_bars("TEST_CODE_000001", std::slice::from_ref(&point), 30)
                .expect_err("only 15/60 minute projections are registered");
        assert!(unsupported.contains("unsupported intraday aggregation interval 30"));

        let empty = aggregate_completed_bars("TEST_CODE_000001", &[], 15)
            .expect_err("an empty admitted series cannot form a bar");
        assert!(empty.contains("source series is empty"));

        let mismatch = aggregate_completed_bars("TEST_CODE_000002", &[point], 15)
            .expect_err("record identity must match the requested security");
        assert!(mismatch.contains("source identity mismatch"));
        assert!(mismatch.contains("TEST_CODE_000001"));
    }

    #[test]
    fn br168_aggregation_rejects_off_session_and_regressed_volume() {
        let shanghai = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let local = |hour, minute| {
            shanghai
                .with_ymd_and_hms(2026, 7, 24, hour, minute, 0)
                .single()
                .unwrap()
                .with_timezone(&Utc)
        };
        let off_session = minute_point(local(12, 0), 10.0, 100.0);
        let error = aggregate_completed_bars("TEST_CODE_000001", &[off_session], 15)
            .expect_err("lunch-break evidence must not enter a candle");
        assert!(error.contains("off-session minute"));

        let regressed = vec![
            minute_point(local(9, 30), 10.0, 200.0),
            minute_point(local(9, 31), 10.1, 199.0),
        ];
        let error = aggregate_completed_bars("TEST_CODE_000001", &regressed, 15)
            .expect_err("cumulative source volume cannot regress");
        assert!(error.contains("cumulative minute volume regressed"));
    }

    #[test]
    fn br168_incomplete_bucket_is_not_promoted_and_complete_ohlcv_is_exact() {
        let shanghai = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let local = |minute| {
            shanghai
                .with_ymd_and_hms(2026, 7, 24, 9, minute, 0)
                .single()
                .unwrap()
                .with_timezone(&Utc)
        };
        let incomplete = vec![
            minute_point(local(30), 10.0, 100.0),
            minute_point(local(44), 10.5, 400.0),
        ];
        let error = aggregate_completed_bars("TEST_CODE_000001", &incomplete, 15)
            .expect_err("the current unfinished bucket must be omitted");
        assert!(error.contains("no completed 15min bars available"));

        let complete = vec![
            minute_point(local(30), 10.0, 100.0),
            minute_point(local(31), 11.0, 200.0),
            minute_point(local(45), 9.0, 500.0),
        ];
        let bars = aggregate_completed_bars("TEST_CODE_000001", &complete, 15)
            .expect("the source close at 09:45 completes the bucket");
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].timestamp, "2026-07-24 09:45");
        assert_eq!(bars[0].open, 10.0);
        assert_eq!(bars[0].high, 11.0);
        assert_eq!(bars[0].low, 9.0);
        assert_eq!(bars[0].close, 9.0);
        assert_eq!(bars[0].volume, 500.0);
    }
}
