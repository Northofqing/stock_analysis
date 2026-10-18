//! 全球新闻 provider 探测（2026-10-18）。
//!
//! ## 要回答的问题
//!
//! `news_ai_assessment` 从 2026-10-14T06:50 起零新增，
//! `NewsFlashGate` 日志显示 `input=67 duplicate=58 stale=6 buffered=0`。
//!
//! 这有两种可能：
//!   a) 所有 provider 的内容源都枯竭了
//!   b) 只有默认的 Eastmoney 枯竭，别的 provider 还活着
//!
//! 逐个 provider 拉一次就能区分。如果 Cailianpress / Jin10 有今天的内容，
//! 那新闻催化这一维只需切 provider，不用写新抓取器。
//!
//! ## 必须初始化数据库
//!
//! BR-159 要求每次采集都写 `data_acquisition_audit`。
//! 不 init 会得到 `acquisition_audit_unavailable: database is not initialized`，
//! 那不是 provider 故障，别误判。
//!
//! 用法:
//!   GRPC_MARKET_CLIENT_BUNDLE=... STOCK_DB=... global_news_probe

use std::env;

use stock_analysis::data_gateway::global_news::{GlobalNewsGateway, GlobalNewsProvider};
use stock_analysis::database::DatabaseManager;

#[tokio::main]
async fn main() {
    let db_path = env::var("STOCK_DB").unwrap_or_else(|_| {
        eprintln!("[新闻] 必须设置 STOCK_DB（BR-159 审计要写库）");
        std::process::exit(2);
    });
    DatabaseManager::init_retained_monitor(Some(std::path::PathBuf::from(&db_path)))
        .unwrap_or_else(|error| {
            eprintln!("[新闻] 数据库初始化失败: {error}");
            std::process::exit(1);
        });

    let providers = [
        GlobalNewsProvider::Eastmoney,
        GlobalNewsProvider::Cailianpress,
        GlobalNewsProvider::Jin10,
        GlobalNewsProvider::ThePaper,
    ];

    let gateway = GlobalNewsGateway::new();
    let mut alive = Vec::new();

    for provider in providers {
        println!("\n[新闻] === {:?} ===", provider);
        match gateway.global_news(provider, 5).await {
            Ok(batch) => {
                let records = batch.records();
                println!("  records={}", records.len());
                if records.is_empty() {
                    println!("  → 空批次");
                    continue;
                }
                alive.push(format!("{:?}({})", provider, records.len()));
                for record in records.iter().take(3) {
                    println!(
                        "  {}  {}",
                        record.published_at.format("%Y-%m-%d %H:%M"),
                        record.title.chars().take(42).collect::<String>()
                    );
                }
            }
            Err(error) => {
                println!("  失败: {error}");
            }
        }
    }

    println!(
        "\n[新闻] 有数据的 provider: {}",
        if alive.is_empty() {
            "无".to_string()
        } else {
            alive.join(", ")
        }
    );
}
