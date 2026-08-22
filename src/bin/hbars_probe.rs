//! HistoricalBars 桥探针: 复现 news AI 生产路径
//! (HistoricalBarsGateway::required_daily_bars_async → GrpcBridge → server)。
//! Usage: cargo run --bin hbars_probe -- [code]
#![cfg(feature = "magic-gateway")]
use std::path::PathBuf;

use stock_analysis::data_gateway::HistoricalBarsGateway;

fn main() {
    let code = std::env::args().nth(1).unwrap_or_else(|| "600396".to_string());
    // 生产 monitor 在启动时初始化数据库 (审计写入需要)。
    stock_analysis::database::DatabaseManager::init(
        std::env::var("DATABASE_PATH").ok().map(PathBuf::from),
    )
    .expect("database init");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let result = rt.block_on(async {
        let gateway = HistoricalBarsGateway::new();
        match gateway
            .required_daily_bars_async(&code, 120)
            .await
        {
            Ok(batch) => {
                println!(
                    "OK code={} records={} evidence={:?}",
                    code,
                    batch.records().len(),
                    batch.evidence()
                );
            }
            Err(e) => println!("FAIL: {e:#}"),
        }
    });
    result
}
