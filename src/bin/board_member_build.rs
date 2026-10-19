//! 板块成分反查表构建。
//!
//!     STOCK_DB=<path> GRPC_MARKET_ADDR=http://127.0.0.1:18082 \
//!       board_member_build                 # 全市场（stock_concepts 里的 6 位代码）
//!     ... board_member_build --codes 600396,000338
//!     ... board_member_build --limit 100    # 先跑一小批验证
//!
//! 数据源只给「票 → 板块」，这里反向落成 `board_member`，
//! 供 P5 板块轮动做「板块 → 成分股」。

use std::path::PathBuf;

use diesel::prelude::*;
use diesel::sql_types::Text;

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::board_member;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().collect();

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

    // 榜单模式：当日板块榜 top N → 成分股 → board_member（P5 选票入口用）。
    if args.iter().any(|arg| arg == "--from-ranking") {
        let date = arg_value(&args, "--date")
            .unwrap_or_else(|| chrono::Local::now().date_naive().format("%Y-%m-%d").to_string());
        let top = arg_value(&args, "--top")
            .and_then(|raw| raw.parse::<usize>().ok())
            .unwrap_or(10);
        let per_board_limit = arg_value(&args, "--per-board-limit")
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(10_000);
        println!("[板块成分] 榜单模式 date={date} top={top} limit={per_board_limit}");
        match board_member::build_from_ranking(db, &date, top, per_board_limit).await {
            Ok(report) => {
                report.log();
                println!(
                    "\n汇总: boards={} ok={} failed={} rows={}",
                    report.boards_total, report.boards_ok, report.boards_failed, report.rows
                );
                for (name, code, count) in &report.per_board {
                    println!("  {name} → {code} {count} 只");
                }
                for failure in &report.failures {
                    println!("  ! {failure}");
                }
            }
            Err(error) => {
                eprintln!("[板块成分] 榜单模式失败: {error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    let mut codes: Vec<String> = match arg_value(&args, "--codes") {
        Some(raw) => raw
            .split(',')
            .map(|item| item.trim().to_owned())
            .filter(|item| !item.is_empty())
            .collect(),
        None => {
            let mut conn = db.get_conn()?;
            #[derive(QueryableByName)]
            struct CodeRow {
                #[diesel(sql_type = Text)]
                code: String,
            }
            diesel::sql_query(
                "SELECT DISTINCT code FROM stock_concepts \
                  WHERE code GLOB '[0-9][0-9][0-9][0-9][0-9][0-9]' ORDER BY code",
            )
            .load::<CodeRow>(&mut conn)?
            .into_iter()
            .map(|row| row.code)
            .collect()
        }
    };

    if let Some(limit) = arg_value(&args, "--limit").and_then(|raw| raw.parse::<usize>().ok()) {
        codes.truncate(limit);
    }
    if let Some(offset) = arg_value(&args, "--offset").and_then(|raw| raw.parse::<usize>().ok()) {
        if offset >= codes.len() {
            println!("[板块成分] offset={offset} 超出总数 {}，无事可做", codes.len());
            return Ok(());
        }
        codes.drain(..offset);
    }

    let chunk = arg_value(&args, "--chunk")
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(50);

    println!("[板块成分] 目标 {} 只，每批 {chunk} 只", codes.len());

    match board_member::build(db, &codes, chunk).await {
        Ok(report) => {
            report.log();
            println!(
                "\n汇总: requested={} fetched={} empty={} failed={} rows={} boards={}",
                report.requested,
                report.fetched,
                report.empty,
                report.failed,
                report.rows,
                report.boards
            );
        }
        Err(error) => {
            eprintln!("[板块成分] 失败: {error}");
            std::process::exit(1);
        }
    }

    Ok(())
}
