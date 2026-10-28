//! P6 二层归因 CLI。
//!
//!     attribution_benchmark_build --date 2026-10-16
//!     attribution_benchmark_build --show --date 2026-10-16
//!
//! 读 `STOCK_DB` 或默认运行根主库。

use std::path::PathBuf;

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::attribution_benchmark;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let show_only = args.iter().any(|arg| arg == "--show");

    let trade_date = match arg_value(&args, "--date") {
        Some(value) => value,
        None => {
            eprintln!("用法: attribution_benchmark_build [--show] --date YYYY-MM-DD");
            std::process::exit(2);
        }
    };

    let db_path = std::env::var("STOCK_DB")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            let runtime = std::env::var("HOME").ok().map(|home| {
                PathBuf::from(home)
                    .join(".local/share/stock-analysis-runtime/data/stock_analysis.db")
            })?;
            runtime.exists().then_some(runtime)
        });

    if let Err(error) = DatabaseManager::init(db_path) {
        eprintln!("数据库初始化失败: {error}");
        std::process::exit(1);
    }
    let db = match DatabaseManager::try_get() {
        Some(db) => db,
        None => {
            eprintln!("数据库未初始化");
            std::process::exit(1);
        }
    };

    if !show_only {
        match attribution_benchmark::build(db, &trade_date) {
            Ok(result) => {
                println!(
                    "[归因] {} rows={} with_benchmark={} with_board={} sample={} 基准={}",
                    result.trade_date,
                    result.rows,
                    result.with_benchmark,
                    result.with_board,
                    result.sample_size,
                    if result.benchmark_accepted {
                        "采纳"
                    } else {
                        "样本不足(留 NULL)"
                    }
                );
            }
            Err(error) => {
                eprintln!("构建失败: {error}");
                std::process::exit(1);
            }
        }
    }

    match attribution_benchmark::show(db, &trade_date) {
        Ok(rows) => {
            if rows.is_empty() {
                println!("[归因] {trade_date} 无数据");
                return;
            }
            println!();
            println!(
                "{:<8} {:>8} {:>8} {:>8} {:>8}",
                "代码", "个股%", "市场%", "超额%", "板块%"
            );
            for row in rows.iter().take(30) {
                println!(
                    "{:<8} {:>8} {:>8} {:>8} {:>8}",
                    row.code,
                    row.stock_pct
                        .map(|v| format!("{v:+.2}"))
                        .unwrap_or_else(|| "-".to_owned()),
                    row.market_pct
                        .map(|v| format!("{v:+.2}"))
                        .unwrap_or_else(|| "-".to_owned()),
                    row.excess_pct
                        .map(|v| format!("{v:+.2}"))
                        .unwrap_or_else(|| "-".to_owned()),
                    row.board_pct
                        .map(|v| format!("{v:+.2}"))
                        .unwrap_or_else(|| "未落地".to_owned()),
                );
            }
            if rows.len() > 30 {
                println!("… 共 {} 只", rows.len());
            }
            if let Some(benchmark) = rows.first().and_then(|row| row.benchmark.clone()) {
                println!();
                println!("基准: {benchmark}");
            }
        }
        Err(error) => {
            eprintln!("读取失败: {error}");
            std::process::exit(1);
        }
    }
}
