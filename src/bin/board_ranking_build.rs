//! 板块榜落库 CLI（2026-10-18）。
//!
//! monitor 的 15:30 调度也走同一个 `refresh_board_rankings`，
//! 这个 CLI 是为了在非交易时段手工验证数据源通不通。
//!
//! 用法:
//!   STOCK_DB=data/stock_analysis.db board_ranking_build
//!   STOCK_DB=... board_ranking_build --date 2026-10-16

use std::env;

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::board_ranking_snapshot;

#[tokio::main]
async fn main() {
    let db_path = env::var("STOCK_DB").unwrap_or_else(|_| {
        eprintln!("[板块榜] 必须设置 STOCK_DB（例如 STOCK_DB=data/stock_analysis.db）");
        std::process::exit(2);
    });

    let args: Vec<String> = env::args().skip(1).collect();
    let date = args
        .iter()
        .position(|arg| arg == "--date")
        .and_then(|index| args.get(index + 1).cloned())
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());

    DatabaseManager::init_retained_monitor(Some(std::path::PathBuf::from(&db_path)))
        .unwrap_or_else(|error| {
            eprintln!("[板块榜] 数据库初始化失败: {error}");
            std::process::exit(1);
        });
    let db = DatabaseManager::try_get().unwrap_or_else(|| {
        eprintln!("[板块榜] 数据库不可用");
        std::process::exit(1);
    });

    println!("[板块榜] trade_date={date}");
    match board_ranking_snapshot::refresh_board_rankings(&db, &date).await {
        Ok(report) => {
            report.log();
            println!(
                "[板块榜] f3={} f62={} persisted={} errors={}",
                report.f3_count,
                report.f62_count,
                report.persisted,
                report.errors.len()
            );
            for error in &report.errors {
                println!("  ! {error}");
            }
        }
        Err(error) => {
            eprintln!("[板块榜] 失败: {error}");
            std::process::exit(1);
        }
    }

    // 回读确认落库结果
    if let Ok(mut conn) = db.get_conn() {
        match board_ranking_snapshot::top_gaining_boards(&mut conn, &date, 5) {
            Ok(rows) if rows.is_empty() => println!("\n[板块榜] 回读：当日无数据"),
            Ok(rows) => {
                println!("\n[板块榜] 回读 top5：");
                for (index, (code, name, change_pct)) in rows.iter().enumerate() {
                    println!("  {}. {} {} {:+.2}%", index + 1, code, name, change_pct);
                }
            }
            Err(error) => println!("\n[板块榜] 回读失败: {error}"),
        }
    }
}
