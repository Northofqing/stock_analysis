//! TDX 5 分钟线回放可行性探针 — 验证周六能否拉到周五全天分钟数据。
//!
//! 用法: cargo run --bin tdx_5min_probe -- [code] [market]
//! 输出:
//!   1. 实时报价 (servertime 原始值 + 价格) — 验证周六 quote 时间戳行为
//!   2. 5 分钟 K 线 (KLINE_5MIN, 400 根) — 验证周五 9:30-15:00 的 bar 是否齐全
//! 只读探测，无写入、无推送。

#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::protocol::constants::{fq_type, KLINE_5MIN};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::TdxHqClient;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = args.get(1).cloned().unwrap_or_else(|| "605178".to_string());
    let market: u8 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    let client = TdxHqClient::new();
    client
        .connect_to_any(Some(5.0))
        .unwrap_or_else(|e| panic!("connect failed: {e}"));

    // 1. 实时报价 — 看 servertime 原始行为
    let quotes = client
        .get_security_quotes(&[(market, code.as_str())])
        .unwrap_or_else(|e| panic!("quote failed: {e}"));
    for q in &quotes {
        println!(
            "QUOTE: code={} market={} price={} servertime={:?} last_close={}",
            q.code, q.market, q.price, q.servertime, q.last_close
        );
        println!(
            "       open={} high={} low={} vol={} cur_vol={} amount={}",
            q.open, q.high, q.low, q.vol, q.cur_vol, q.amount
        );
    }

    // 2. 5 分钟 K 线
    let bars = client
        .get_security_bars(KLINE_5MIN, market, &code, 0, 400, fq_type::NONE)
        .unwrap_or_else(|e| panic!("5min bars failed: {e}"));
    println!("5MIN BARS: total={}", bars.len());
    if bars.is_empty() {
        return;
    }
    let bar_ts = |b: &stock_analysis::magic_compat::SecurityBar| -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            b.year, b.month, b.day, b.hour, b.minute
        )
    };
    // 统计周五 (2026-08-07) 的 bar
    let mut friday_count = 0usize;
    let mut friday_first: Option<&stock_analysis::magic_compat::SecurityBar> = None;
    let mut friday_last: Option<&stock_analysis::magic_compat::SecurityBar> = None;
    for b in bars.iter() {
        if b.year == 2026 && b.month == 8 && b.day == 7 {
            friday_count += 1;
            if friday_first.is_none() {
                friday_first = Some(b);
            }
            friday_last = Some(b);
        }
    }
    println!("FRIDAY_2026-08-07: bars={friday_count}");
    if let Some(first) = friday_first {
        println!(
            "  first: {} open={} close={} high={} low={} vol={}",
            bar_ts(first),
            first.open,
            first.close,
            first.high,
            first.low,
            first.vol
        );
    }
    if let Some(last) = friday_last {
        println!(
            "  last:  {} open={} close={} high={} low={} vol={}",
            bar_ts(last),
            last.open,
            last.close,
            last.high,
            last.low,
            last.vol
        );
    }
    println!("TAIL_3:");
    for b in bars.iter().rev().take(3) {
        println!(
            "  {} open={} close={} high={} low={} vol={}",
            bar_ts(b),
            b.open,
            b.close,
            b.high,
            b.low,
            b.vol
        );
    }
}
