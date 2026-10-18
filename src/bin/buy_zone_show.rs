//! 按票输出可买价（P2 运维入口）.
//!
//! 用法:
//!   STOCK_DB=<path> cargo run --release --bin buy_zone_show
//!   STOCK_DB=<path> cargo run --release --bin buy_zone_show -- --date 2026-10-16
//!   STOCK_DB=<path> cargo run --release --bin buy_zone_show -- --code 600703
//!
//! 默认标的 = active tickets ∪ 当前持仓（与 P1.3 / P2 数据补齐同口径）。
//! 只读：不写库，不查行情，全部输入来自 stock_daily + trading_fact_snapshot。
//!
//! 注意: 对生产大库启动会触发 BR-159 全量验链（约 10 分钟）——不是卡死。

use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel::sql_types::Text;

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::buy_zone;

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(|value| value.as_str())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let date = match arg_value(&args, "--date") {
        Some(raw) => chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|error| format!("--date 非法 ({raw}): {error}"))?,
        None => chrono::Local::now().date_naive(),
    };
    let as_of = date.format("%Y-%m-%d").to_string();

    let path = match std::env::var("STOCK_DB") {
        Ok(value) => std::path::PathBuf::from(value),
        Err(_) => {
            eprintln!("[可买价] 必须设置 STOCK_DB（例如 STOCK_DB=data/stock_analysis.db）");
            std::process::exit(2);
        }
    };

    DatabaseManager::init_retained_monitor(Some(path))?;
    let db = DatabaseManager::get();
    let mut conn = db.get_conn()?;

    let codes: Vec<String> = match arg_value(&args, "--code") {
        Some(code) => vec![code.to_string()],
        None => {
            #[derive(QueryableByName)]
            struct CodeRow {
                #[diesel(sql_type = Text)]
                code: String,
            }
            let mut set: BTreeSet<String> = BTreeSet::new();
            let rows: Vec<CodeRow> =
                diesel::sql_query("SELECT code FROM ticket_lifecycle WHERE close_date IS NULL")
                    .load(&mut conn)
                    .unwrap_or_default();
            for row in rows {
                set.insert(row.code);
            }
            for position in stock_analysis::portfolio::load_positions().unwrap_or_default() {
                set.insert(position.code);
            }
            set.into_iter().collect()
        }
    };

    println!("[可买价] as_of={as_of} 标的数={}", codes.len());
    let mut produced = 0usize;
    for code in &codes {
        match buy_zone::compute(&mut conn, code, &as_of) {
            Ok(Some(zone)) => {
                produced += 1;
                println!("\n{}", zone.render());
                let advice = stock_analysis::monitor::position_sizing::advise(code, &zone);
                println!("     {}", advice.render());
            }
            Ok(None) => println!("\n{code}  无日线数据，跳过"),
            Err(error) => println!("\n{code}  失败: {error}"),
        }
    }
    println!("\n汇总: 标的={} 产出={}", codes.len(), produced);
    Ok(())
}
