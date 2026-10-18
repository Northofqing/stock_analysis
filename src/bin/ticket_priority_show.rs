//! 观察池票优先级 CLI（2026-10-18）。
//!
//! 用法：
//!   STOCK_DB=data/stock_analysis.db ticket_priority_show
//!   STOCK_DB=... ticket_priority_show --date 2026-10-16

use std::env;

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::ticket_priority::rank_watch_pool;

fn main() {
    let db_path = env::var("STOCK_DB").unwrap_or_else(|_| {
        eprintln!("[优先级] 必须设置 STOCK_DB（例如 STOCK_DB=data/stock_analysis.db）");
        std::process::exit(2);
    });

    let args: Vec<String> = env::args().skip(1).collect();
    let mut as_of = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--date" => {
                if let Some(value) = args.get(index + 1) {
                    as_of = value.clone();
                }
                index += 2;
            }
            other => {
                eprintln!("[优先级] 未知参数 {other}");
                std::process::exit(2);
            }
        }
    }

    DatabaseManager::init_retained_monitor(Some(std::path::PathBuf::from(&db_path)))
        .unwrap_or_else(|error| {
            eprintln!("[优先级] 数据库初始化失败: {error}");
            std::process::exit(1);
        });

    let db = DatabaseManager::try_get().unwrap_or_else(|| {
        eprintln!("[优先级] 数据库不可用");
        std::process::exit(1);
    });

    let rows = rank_watch_pool(&db, &as_of).unwrap_or_else(|error| {
        eprintln!("[优先级] 计算失败: {error}");
        std::process::exit(1);
    });

    if rows.is_empty() {
        println!("[优先级] as_of={as_of} 观察池为空");
        return;
    }

    println!("[优先级] as_of={as_of} 标的={}", rows.len());
    for (position, row) in rows.iter().enumerate() {
        println!("\n{}", row.render(position + 1));
    }
}
