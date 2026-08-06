//! 统一网关行情探针 — 验证 TDX→腾讯→新浪 fallover 链能否建立 Quote 能力。
//! 用法: cargo run --bin gateway_quote_probe -- [code...]
//! 走生产同款入口 MarketDataGateway::realtime_quotes (含 5 秒门 + 逐源 fallover)。
//! 只读网络请求，无写入、无推送。

use stock_analysis::data_gateway::MarketDataGateway;

fn main() {
    let codes: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .collect();
    let codes = if codes.is_empty() {
        vec!["605178".to_string()]
    } else {
        codes
    };
    std::env::set_var("DATABASE_PATH", "/tmp/gw_probe.db");
    stock_analysis::database::DatabaseManager::init(Some("/tmp/gw_probe.db".into()))
        .expect("db init");
    println!("== gateway quote probe codes={codes:?} ==");
    match MarketDataGateway::new().realtime_quotes(&codes) {
        Ok(batch) => {
            println!(
                "QUOTE BATCH OK: provider={:?} records={}",
                batch.evidence().provider,
                batch.records().len()
            );
            for r in batch.records().iter().take(3) {
                println!(
                    "  code={} price={} source_at={:?}",
                    r.code, r.price, r.source_at
                );
            }
        }
        Err(e) => println!("QUOTE BATCH FAIL: {e}"),
    }
}
