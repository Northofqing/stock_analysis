//! 手工录入一笔成交（`source=manual`，价格/时间真实）。
//!
//! 系统没有券商成交回报接口，只能靠两种来源：
//! - 本 CLI 手工录入（精确）
//! - 导入持仓快照时差分推断（数量准、价格是反推均价）
//!
//! ```
//! STOCK_DB=<path> trade_record --code 600396 --name 华电辽能 \
//!   --direction buy --price 11.12 --shares 2000 \
//!   --at "2026-10-19 14:30:00" --reason "板块轮动入场"
//!
//! STOCK_DB=<path> trade_record --show       # 看今天的流水
//! STOCK_DB=<path> trade_record --show --days 5
//! ```

use std::path::PathBuf;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::trade_ledger::{self, NewTrade};

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
        });
    DatabaseManager::init(db_path)?;
    let db = DatabaseManager::try_get().ok_or("数据库未初始化")?;

    if args.iter().any(|arg| arg == "--show") {
        let days = arg_value(&args, "--days")
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or(0);
        let since = (chrono::Local::now().date_naive() - chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string();

        #[derive(QueryableByName)]
        struct Row {
            #[diesel(sql_type = Text)]
            traded_at: String,
            #[diesel(sql_type = Text)]
            code: String,
            #[diesel(sql_type = Text)]
            name: String,
            #[diesel(sql_type = Text)]
            direction: String,
            #[diesel(sql_type = Double)]
            price: f64,
            #[diesel(sql_type = BigInt)]
            shares: i64,
            #[diesel(sql_type = Text)]
            source: String,
            #[diesel(sql_type = Nullable<Text>)]
            reason: Option<String>,
        }

        let mut conn = db.get_conn()?;
        trade_ledger::ensure_schema(&mut conn)?;
        let rows: Vec<Row> = diesel::sql_query(
            "SELECT traded_at, code, name, direction, price, shares, source, reason \
               FROM trades WHERE traded_at >= ? ORDER BY traded_at DESC",
        )
        .bind::<Text, _>(since.clone())
        .load(&mut conn)?;

        println!("=== {} 起 {} 笔 ===", since, rows.len());
        for row in rows {
            println!(
                "{}  {} {}  {}  {:.3} × {}  [{}]  {}",
                row.traded_at,
                row.code,
                row.name,
                row.direction,
                row.price,
                row.shares,
                row.source,
                row.reason.unwrap_or_default()
            );
        }
        return Ok(());
    }

    let code = arg_value(&args, "--code").ok_or("缺少 --code")?;
    let direction = arg_value(&args, "--direction").ok_or("缺少 --direction (buy/sell)")?;
    let price: f64 = arg_value(&args, "--price")
        .ok_or("缺少 --price")?
        .parse()
        .map_err(|_| "价格解析失败")?;
    let shares: u64 = arg_value(&args, "--shares")
        .ok_or("缺少 --shares")?
        .parse()
        .map_err(|_| "股数解析失败")?;
    let name = arg_value(&args, "--name").unwrap_or_default();
    let reason = arg_value(&args, "--reason").unwrap_or_else(|| "手工录入".to_owned());
    let traded_at = arg_value(&args, "--at")
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());

    let trade = NewTrade {
        code: code.clone(),
        name: name.clone(),
        direction: direction.clone(),
        price,
        shares,
        traded_at: traded_at.clone(),
        reason,
        source: "manual".to_string(),
        snapshot_id: None,
    };
    match trade_ledger::record(db, &trade) {
        Ok(()) => {
            println!(
                "已录入: {} {} {} {:.3} × {} 股  @{}",
                traded_at, code, name, price, shares, direction
            );
            Ok(())
        }
        Err(error) => {
            eprintln!("录入失败: {error}");
            std::process::exit(1);
        }
    }
}
