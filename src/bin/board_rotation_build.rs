//! P5 板块轮动 CLI。
//!
//!     board_rotation_build --date 2026-10-19
//!     board_rotation_build --show --date 2026-10-19
//!
//! 需要至少两个交易日的板块榜数据；单日会直接报错说明。

use std::path::PathBuf;

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::board_rotation_signal;

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
            eprintln!("用法: board_rotation_build [--show] --date YYYY-MM-DD");
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
        match board_rotation_signal::has_comparable_days(db, &trade_date) {
            Ok(false) => {
                eprintln!(
                    "[轮动] {trade_date} 之前没有第二个交易日的板块榜，无法判断轮动。\
                     需要至少两天的 board_ranking_daily；单日只能看排名，看不出接力。"
                );
                std::process::exit(1);
            }
            Err(error) => {
                eprintln!("[轮动] 预检失败: {error}");
                std::process::exit(1);
            }
            Ok(true) => {}
        }

        match board_rotation_signal::build(db, &trade_date) {
            Ok(result) => {
                println!(
                    "[轮动] {} prev={} 主线={} 接力={} 退潮={}",
                    result.trade_date,
                    result.prev_date.as_deref().unwrap_or("-"),
                    result.mainline,
                    result.new,
                    result.fading
                );
            }
            Err(error) => {
                eprintln!("构建失败: {error}");
                std::process::exit(1);
            }
        }
    }

    match board_rotation_signal::show(db, &trade_date) {
        Ok(rows) => {
            if rows.is_empty() {
                println!("[轮动] {trade_date} 无数据");
                return;
            }
            println!();
            println!(
                "{:<18} {:<10} {:>6} {:>6} {:>8} {:>8}",
                "板块", "状态", "今日", "昨日", "今涨幅", "昨涨幅"
            );
            for row in &rows {
                let status = match row.status.as_str() {
                    "new" => "接力",
                    "mainline" => "主线",
                    "fading" => "退潮",
                    other => other,
                };
                println!(
                    "{:<18} {:<10} {:>6} {:>6} {:>8} {:>8}",
                    format!("{} {}", row.board_code, row.board_name),
                    status,
                    row.rank_today
                        .map(|v| (v + 1).to_string())
                        .unwrap_or_else(|| "-".to_owned()),
                    row.rank_prev
                        .map(|v| (v + 1).to_string())
                        .unwrap_or_else(|| "-".to_owned()),
                    row.pct_today
                        .map(|v| format!("{v:+.2}"))
                        .unwrap_or_else(|| "-".to_owned()),
                    row.pct_prev
                        .map(|v| format!("{v:+.2}"))
                        .unwrap_or_else(|| "-".to_owned()),
                );
            }
        }
        Err(error) => {
            eprintln!("读取失败: {error}");
            std::process::exit(1);
        }
    }
}
