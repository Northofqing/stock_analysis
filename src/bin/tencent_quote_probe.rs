//! 腾讯行情探针 — 验证兜底源能否提供带 source_at 的 quote (5 秒门前提)。
//! 用法: cargo run --bin tencent_quote_probe -- [code]
//! 只读网络请求，无写入、无推送。

#[cfg(feature = "magic-gateway")]
use magic_market_core::RealtimeQuotes;
#[cfg(feature = "magic-gateway")]
use magic_tencent_rs::TencentClient;
use stock_analysis::data_gateway::instrument_identity::resolve_production_equity;

fn main() {
    let code = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "605178".to_string());
    let client = TencentClient::new().expect("TencentClient::new");
    let identity = resolve_production_equity(&code, None).expect("resolve_production_equity");
    let instrument = identity.instrument().clone();
    println!("== Tencent quote probe code={code} instrument={instrument:?} ==");
    match client.realtime_quotes(&[instrument]) {
        Ok(batch) => {
            let records = batch.records();
            println!("records={}", records.len());
            for r in records.iter().take(3) {
                println!(
                    "  code={} price={} source_at={:?} status={:?}",
                    r.instrument().code(),
                    r.price().get(),
                    r.source_at(),
                    r.status()
                );
            }
            println!(
                "batch provenance source_at={:?}",
                batch.provenance().source_at()
            );
        }
        Err(e) => println!("TENCENT QUOTE FAIL: {e}"),
    }
}
