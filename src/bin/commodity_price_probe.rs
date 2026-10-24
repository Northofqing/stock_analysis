//! 商品价格探针（5.1）：手动拉一次新浪报价并落库，验证源与表。
//!
//!   STOCK_DB=<db> commodity_price_probe [YYYY-MM-DD]
//!
//! 默认日期取今天；非交易日建议显式给最近交易日，避免落一行周末数据。
//! 不走 `DatabaseManager::init`（会在生产大库上跑十几分钟的审计链），直接开 diesel 连接。

use diesel::Connection as _;
use stock_analysis::monitor::commodity_price;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let date = std::env::args().nth(1).unwrap_or_else(|| {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    });
    let db_path = std::env::var("STOCK_DB").unwrap_or_else(|_| "data/stock_analysis.db".to_string());
    let mut conn = match diesel::SqliteConnection::establish(&format!("sqlite://{db_path}")) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("打开数据库失败 {db_path}: {error}");
            std::process::exit(2);
        }
    };
    let parsed = match commodity_price::fetch_quotes().await {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("拉取失败: {error}");
            std::process::exit(1);
        }
    };
    match commodity_price::persist(&mut conn, &date, &parsed) {
        Ok(report) => {
            report.log();
            println!(
                "trade_date={date} written={} missing={:?}",
                report.written, report.missing
            );
        }
        Err(error) => {
            eprintln!("落库失败: {error}");
            std::process::exit(1);
        }
    }
}
