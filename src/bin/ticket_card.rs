//! 打印单票决策卡（只读）：日线指标 + 量能 + 进出场 + 当前状态。
//!
//!     STOCK_DB=<path> ticket_card --date 2026-11-13
//!     STOCK_DB=<path> ticket_card --date 2026-11-13 --codes 001216,002205
//!     STOCK_DB=<path> ticket_card --date 2026-11-13 --all --json
//!
//! 默认只打印持仓与 `entry_proposed`；`--all` 含 observing。
//! 指标是日线口径（截至最后一根已入库 K 线），不是盘中实时指标。
//! 连接以 `query_only=ON` 打开，不写库、不跑迁移。

use std::path::PathBuf;

use diesel::connection::Connection;
use diesel::{RunQueryDsl, SqliteConnection};

use stock_analysis::monitor::ticket_card;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
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
        })
        .ok_or("找不到数据库；请设置 STOCK_DB")?;
    let mut conn = SqliteConnection::establish(
        db_path
            .to_str()
            .ok_or_else(|| format!("数据库路径非法: {}", db_path.display()))?,
    )?;
    diesel::sql_query("PRAGMA query_only = ON").execute(&mut conn)?;

    let date = arg_value(&args, "--date").unwrap_or_else(|| {
        chrono::Local::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string()
    });
    let mut cards = ticket_card::build_cards_with_conn(&mut conn, &date)?;

    if let Some(codes) = arg_value(&args, "--codes") {
        let wanted: std::collections::HashSet<String> = codes
            .split(',')
            .map(|item| item.trim().to_owned())
            .filter(|item| !item.is_empty())
            .collect();
        cards.retain(|card| wanted.contains(&card.code));
    } else if !args.iter().any(|arg| arg == "--all") {
        cards.retain(|card| card.holding || card.stage == "entry_proposed");
    }

    if args.iter().any(|arg| arg == "--json") {
        println!("{}", serde_json::to_string_pretty(&cards)?);
    } else {
        for card in &cards {
            print!("{}", card.render());
        }
        println!(
            "— {} 张卡（日期 {}，指标为日线口径，截至最后一根已入库 K 线）",
            cards.len(),
            date
        );
    }
    Ok(())
}
