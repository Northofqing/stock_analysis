//! 渲染明日计划（不推送，只打印），用于上线前人工确认格式。
//!
//!     STOCK_DB=<path> tomorrow_plan --date 2026-10-19
//!     STOCK_DB=<path> tomorrow_plan --date 2026-10-19 --out /tmp/plan.md
//!
//! 与 monitor 16:00 调度用的是同一个 `buy_zone_dispatch::render_plan`，
//! 所以这里看到什么，明天推送就是什么。

use std::path::PathBuf;

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::buy_zone_dispatch;

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

    let date = arg_value(&args, "--date")
        .unwrap_or_else(|| chrono::Local::now().date_naive().format("%Y-%m-%d").to_string());

    let text = buy_zone_dispatch::render_plan(db, &date)?;

    match arg_value(&args, "--out") {
        Some(path) => {
            std::fs::write(&path, &text)?;
            println!("已写入 {path}（{} 字）", text.chars().count());
        }
        None => println!("{text}"),
    }
    Ok(())
}
