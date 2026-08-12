//! TDX T0 数据回放 — 周六用周五同时段真实数据跑做T 证据链。
//!
//! 原理: TDX 周六返回周五收盘快照 (servertime=周五最后成交时刻) + 完整的
//! 历史 5 分钟线 (已验证 2026-08-07 48 根齐全)。把观测时钟注入到周五
//! 收盘时刻后, `source_time` 用该日期的日期解码 servertime, freshness 门
//! age = 注入时刻 - 周五收盘 ≤ 5s 通过 — 全链路使用真实 TDX 数据。
//!
//! 用法: cargo run --bin t0_replay -- [HH:MM:SS] [code...]
//!   注入时刻默认 2026-08-07 15:29:50 (周五收盘后 50s, age=1s)
//!   代码默认读 .env STOCK_LIST (前 10 只, 可用 code 参数覆盖)
//! 只读网络请求, 无写入、无推送。

use chrono::TimeZone;
use magic_tdx_rs::protocol::constants::{fq_type, KLINE_5MIN, KLINE_DAILY};
use magic_tdx_rs::TdxHqClient;
use stock_analysis::data_gateway::magic_tdx_t0::fetch_magic_tdx_t0_batch_with_clock;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (time_arg, codes_arg): (Option<&String>, Vec<&String>) =
        match args.first().map(|s| s.as_str()) {
            Some(t) if t.contains(':') => (Some(&args[0]), args[1..].iter().collect()),
            _ => (None, args.iter().collect()),
        };

    // 注入时钟: 默认周五 15:29:50 (age=1s 过 5s 门)
    let replay_at = match time_arg {
        Some(t) => {
            let mut parts = t.split(':');
            let h: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(15);
            let m: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(29);
            let s: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(50);
            chrono::FixedOffset::east_opt(8 * 3600)
                .unwrap()
                .with_ymd_and_hms(2026, 8, 7, h, m, s)
                .single()
                .expect("replay time")
                .with_timezone(&chrono::Utc)
        }
        None => chrono::FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 8, 7, 15, 29, 56)
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
                let (market, first) = if codes[0].starts_with('6') || codes[0].starts_with('9') {
                    (1u8, 1)
                } else if codes[0].starts_with('0') || codes[0].starts_with('3') {
                    (0u8, 1)
                } else {
                    (1u8, 1)
                };
                let daily = client
                    .get_security_bars(KLINE_DAILY, market, &codes[0], 0, 40, fq_type::NONE);
                println!(
                    "DIRECT DAILY (TdxHqClient, market={}): {:?}",
                    market,
                    daily.as_ref().map(|bars| bars.len())
                );
                let m5 = client
                    .get_security_bars(KLINE_5MIN, market, &codes[0], 0, 400, fq_type::NONE);
                println!(
                    "DIRECT 5MIN (TdxHqClient, market={}): {:?}",
                    market,
                    m5.map(|bars| bars.len())
                );
                if let Ok(bars) = daily.as_ref() {
                    for b in bars.iter().take(3) {
                        println!(
                            "  daily bar: {}-{:02}-{:02} close={}",
                            b.year, b.month, b.day, b.close
                        );
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
                "  records={} rejections={}",
                batch.records.len(),
                batch.rejections.len()
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
