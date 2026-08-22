//! RealtimeQuotes 桥探针: 复现 news AI 生产路径同款桥调用 (周末全拒场景)。
//! Usage: cargo run --bin rq_probe -- [code]
#![cfg(feature = "magic-gateway")]

use std::path::PathBuf;

use stock_analysis::data_gateway::MarketDataGateway;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "600396".to_string());
    let codes: Vec<String> = arg
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    stock_analysis::database::DatabaseManager::init(
        std::env::var("DATABASE_PATH").ok().map(PathBuf::from),
    )
    .expect("database init");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let n = codes.len();
    rt.block_on(async {
        let result = tokio::task::spawn_blocking(move || {
            MarketDataGateway::new().realtime_quotes(&codes)
        })
        .await
        .expect("blocking task");
        match result {
            Ok(batch) => println!(
                "OK codes={} records={} verified_empty={} evidence={:?}",
                n,
                batch.records().len(),
                batch.is_verified_empty(),
                batch.evidence()
            ),
            Err(e) => println!("FAIL: {e:#}"),
        }
    });
}
