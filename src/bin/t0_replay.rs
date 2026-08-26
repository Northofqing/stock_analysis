//! TDX T0 数据回放 — 周末用最近交易日同时段真实数据跑做T 证据链。
//!
//! 原理: TDX 周末返回最近交易日收盘快照 (servertime=该日最后成交时刻) +
//! 完整的 48 根历史 5 分钟线。注入观测时钟后 freshness 门按
//! age = 注入时刻 - servertime 裁决 — 可模拟生产两条路径:
//!   * 注入 servertime 之后 >5s (默认 15:30:20, servertime≈15:29:53):
//!     命中 2026-08-21 生产场景 (TDX servertime 滞后墙钟 14-27s),
//!     age 门放宽 → time_untrustworthy=true
//!   * 注入 servertime 之前 (如 15:29:30): future_time → 仍硬拒
//!
//! 另附盘中形态模拟: 真实最近交易日 48 根剔除 11:30 bar 后按 14:48
//! 观测校验 (复现 2026-08-21 盘中 five_minute_gap 场景)。
//!
//! 用法: cargo run --bin t0_replay -- [HH:MM:SS] [YYYY-MM-DD] [code...]
//!   注入时刻默认 15:30:20, 日期默认最近交易日
//!   代码默认读 .env STOCK_LIST (前 10 只, 可用 code 参数覆盖)
//! 只读网络请求, 无写入、无推送。

use chrono::TimeZone;
use chrono::{Datelike, NaiveTime};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::protocol::constants::{fq_type, KLINE_5MIN, KLINE_DAILY};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::TdxHqClient;
use stock_analysis::data_gateway::magic_tdx_t0::{
    fetch_magic_tdx_t0_batch_with_clock, five_minute_from_raw, validate_five_minute_bars,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // 参数: [HH:MM:SS] [YYYY-MM-DD] [code...]
    let mut time_arg: Option<&String> = None;
    let mut date_arg: Option<&String> = None;
    let mut codes_arg: Vec<&String> = Vec::new();
    for arg in &args {
        if arg == "--" {
            continue;
        }
        if arg.contains(':') && time_arg.is_none() {
            time_arg = Some(arg);
        } else if arg.contains('-') && date_arg.is_none() {
            date_arg = Some(arg);
        } else {
            codes_arg.push(arg);
        }
    }
    // 回放日期: 参数优先, 否则最近交易日
    let replay_date = match date_arg {
        Some(d) => {
            let mut parts = d.split('-');
            let y: i32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(2026);
            let mo: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(1);
            let da: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(1);
            chrono::NaiveDate::from_ymd_opt(y, mo, da).expect("replay date")
        }
        None => {
            // 2026-08-22 探针: 独立 bin 的 calendar HOLIDAYS 未初始化,
            // prev_trading_day 返回错误日期 (2026-01-01) — 周末场景直接回退
            // 上一个工作日; 假期覆盖请显式传日期参数。
            let mut d = chrono::Local::now().date_naive();
            loop {
                d -= chrono::Duration::days(1);
                if !matches!(d.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
                    break d;
                }
            }
        }
    };

    // 注入时钟: 默认最近交易日 15:30:20 (servertime≈15:29:53 → age≈27s, 命中放宽路径)
    let replay_at = match time_arg {
        Some(t) => {
            let mut parts = t.split(':');
            let h: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(15);
            let m: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(30);
            let s: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(20);
            chrono::FixedOffset::east_opt(8 * 3600)
                .unwrap()
                .with_ymd_and_hms(
                    replay_date.year(),
                    replay_date.month(),
                    replay_date.day(),
                    h,
                    m,
                    s,
                )
                .single()
                .expect("replay time")
                .with_timezone(&chrono::Utc)
        }
        None => chrono::FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(
                replay_date.year(),
                replay_date.month(),
                replay_date.day(),
                15,
                30,
                20,
            )
            .single()
            .expect("default replay time")
            .with_timezone(&chrono::Utc),
    };

    // 代码列表: 参数优先, 否则 .env STOCK_LIST 前 10 (去重, fetch 拒绝重复)
    let mut codes: Vec<String> = if codes_arg.is_empty() {
        std::env::var("STOCK_LIST")
            .ok()
            .map(|s| s.split(',').take(10).map(str::to_owned).collect())
            .unwrap_or_else(|| vec!["605178".to_string()])
    } else {
        codes_arg.iter().map(|s| s.to_string()).collect()
    };
    codes.sort();
    codes.dedup();

    println!(
        "== TDX T0 回放: 注入时刻={} (UTC={}) codes={} ==",
        replay_at
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .format("%Y-%m-%d %H:%M:%S"),
        replay_at,
        codes.len()
    );

    // 直连调试: 生产同款 TdxHqClient 拉日线/5min, 确认周末服务器返回行为
    {
        let client = TdxHqClient::new();
        match client.connect_to_any(Some(5.0)) {
            Ok(_) => {
                let market = if codes[0].starts_with('6') || codes[0].starts_with('9') {
                    1u8
                } else if codes[0].starts_with('0') || codes[0].starts_with('3') {
                    0u8
                } else {
                    1u8
                };
                let daily =
                    client.get_security_bars(KLINE_DAILY, market, &codes[0], 0, 40, fq_type::NONE);
                println!(
                    "DIRECT DAILY (TdxHqClient, market={}): {:?}",
                    market,
                    daily.as_ref().map(|bars| bars.len())
                );
                let m5 =
                    client.get_security_bars(KLINE_5MIN, market, &codes[0], 0, 400, fq_type::NONE);
                println!(
                    "DIRECT 5MIN (TdxHqClient, market={}): {:?}",
                    market,
                    m5.as_ref().map(|bars| bars.len())
                );
                if let Ok(bars) = daily.as_ref() {
                    for b in bars.iter().take(3) {
                        println!(
                            "  daily bar: {}-{:02}-{:02} close={}",
                            b.year, b.month, b.day, b.close
                        );
                    }
                }
                // 盘中形态模拟: 真实最近交易日 48 根剔除 11:30 bar 后按
                // 14:48 观测校验 — 复现 2026-08-21 盘中 five_minute_gap 场景
                // (TDX 盘中午后响应缺 11:30, 收盘后补齐)。
                if let Ok(bars) = m5.as_ref() {
                    let decoded = bars
                        .iter()
                        .map(|bar| five_minute_from_raw(&codes[0], bar.clone()))
                        .collect::<Result<Vec<_>, _>>();
                    if let Ok(rows) = decoded {
                        // 全量历史 (含最近交易日) 参与校验, 与生产 fetch 一致
                        let session_count =
                            rows.iter().filter(|r| r.at.date() == replay_date).count();
                        let mut stripped = rows.clone();
                        stripped.retain(|r| {
                            !(r.at.date() == replay_date
                                && r.at.time() == NaiveTime::from_hms_opt(11, 30, 0).unwrap())
                        });
                        let intraday_at = chrono::FixedOffset::east_opt(8 * 3600)
                            .unwrap()
                            .with_ymd_and_hms(
                                replay_date.year(),
                                replay_date.month(),
                                replay_date.day(),
                                14,
                                48,
                                0,
                            )
                            .single()
                            .expect("intraday sim time")
                            .with_timezone(&chrono::Utc);
                        println!(
                            "SIM 盘中14:48 最近交易日 bars={} 剔除11:30后总bars={}",
                            session_count,
                            stripped.len()
                        );
                        match validate_five_minute_bars(&codes[0], stripped.clone(), intraday_at) {
                            Ok(_) => println!("SIM 盘中14:48 缺11:30 → OK (形态容错放行)"),
                            Err(e) => println!(
                                "SIM 盘中14:48 缺11:30 → REJECT {}: {}",
                                e.reason_code, e.detail
                            ),
                        }
                        // 对照: 全量 (含 11:30) 同样 14:48 校验 (完整形态应通过)
                        match validate_five_minute_bars(&codes[0], rows.clone(), intraday_at) {
                            Ok(_) => println!("SIM 对照 全量含11:30 14:48 → OK"),
                            Err(e) => println!(
                                "SIM 对照 全量含11:30 14:48 → REJECT {}: {}",
                                e.reason_code, e.detail
                            ),
                        }
                    }
                }
            }
            Err(e) => println!("DIRECT CONNECT FAIL: {e}"),
        }
    }

    match fetch_magic_tdx_t0_batch_with_clock(&codes, replay_at, Some(replay_at)) {
        Ok(batch) => {
            println!(
                "BATCH OK: requested_at={} source_at={} observed_at={}",
                batch.requested_at, batch.source_at, batch.observed_at
            );
            println!(
                "  records={} rejections={} time_untrustworthy={}",
                batch.records.len(),
                batch.rejections.len(),
                batch.time_untrustworthy
            );
            for r in &batch.records {
                println!(
                    "  [fresh] code={} quote={} source_at={} settled_daily={} five_min={} avg_price={}",
                    r.code,
                    r.quote.price,
                    r.source_at,
                    r.settled_daily.len(),
                    r.completed_five_minute.len(),
                    r.intraday_average_price
                );
                if let Some(first) = r.completed_five_minute.first() {
                    println!(
                        "          5min_first={} close={}",
                        first.at.format("%Y-%m-%d %H:%M"),
                        first.close
                    );
                }
                if let Some(last) = r.completed_five_minute.last() {
                    println!(
                        "          5min_last ={} close={}",
                        last.at.format("%Y-%m-%d %H:%M"),
                        last.close
                    );
                }
            }
            for rej in &batch.rejections {
                println!(
                    "  [rejected] code={} reason={} retryable={} detail={}",
                    rej.code, rej.reason_code, rej.retryable, rej.detail
                );
            }
        }
        Err(error) => println!("BATCH FAIL: {error:#}"),
    }
}
