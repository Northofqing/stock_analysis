//! BR-164 reviewed operator diagnostic for raw TDX 5-min bars. It reveals whether
//! morning session bars
//! (09:35-11:30) are present in the KLINE_5MIN response during trading.
//! Usage: cargo run --release --bin t0_minute_probe -- <code>
#[cfg(feature = "magic-gateway")]
fn main() {
    use magic_tdx_rs::protocol::constants::{fq_type, KLINE_5MIN};
    use magic_tdx_rs::TdxHqClient;
    use std::time::Instant;

    let code = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "600396".to_string());
    let market = if code.starts_with('6') || code.starts_with('9') {
        1u8
    } else {
        0u8
    };
    let client = TdxHqClient::new();
    let start = Instant::now();
    match client.connect_to_any(Some(5.0)) {
        Ok(_) => println!("connected in {:?}", start.elapsed()),
        Err(e) => {
            println!("connect failed: {e}");
            return;
        }
    }
    let bars = match client.get_security_bars(KLINE_5MIN, market, &code, 0, 400, fq_type::NONE) {
        Ok(b) => b,
        Err(e) => {
            println!("fetch failed: {e}");
            return;
        }
    };
    println!("total bars={} elapsed={:?}", bars.len(), start.elapsed());
    let today = chrono::Local::now().date_naive();
    println!("today={today}");
    // 用生产解码器 (five_minute_from_raw) 还原真实 at, 只打印今天 + 最近历史
    let decoded = bars
        .iter()
        .map(|bar| {
            stock_analysis::data_gateway::magic_tdx_t0::five_minute_from_raw(&code, bar.clone())
        })
        .collect::<Result<Vec<_>, _>>();
    match decoded {
        Ok(rows) => {
            let today_rows: Vec<_> = rows.iter().filter(|r| r.at.date() == today).collect();
            println!("today bars={}", today_rows.len());
            for row in &today_rows {
                println!(
                    "  at={} open={} high={} low={} close={} vol={} amount={}",
                    row.at, row.open, row.high, row.low, row.close, row.volume, row.amount
                );
            }
            println!("last 8 historical:");
            for row in rows.iter().rev().take(8) {
                println!("  at={} close={} vol={}", row.at, row.close, row.volume);
            }
        }
        Err(e) => println!("decode failed: {:?}", e),
    }
}

#[cfg(not(feature = "magic-gateway"))]
fn main() {
    eprintln!("requires --features magic-gateway");
    std::process::exit(2);
}
