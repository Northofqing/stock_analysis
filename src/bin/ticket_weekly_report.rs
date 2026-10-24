//! 选股周报（手动跑版，批 4.3 的 CLI 入口）。
//!
//!     STOCK_DB=<path> ticket_weekly_report
//!     STOCK_DB=<path> ticket_weekly_report --date 2026-10-23
//!
//! 统计逻辑在 `stock_analysis::monitor::ticket_weekly_report::build`，
//! monitor 周五 17:00 用的是同一个函数，所以这里看到什么，推出去就是什么。

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::ticket_weekly_report;

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().collect();

    let db_path = std::env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    DatabaseManager::init(db_path)?;
    let db = DatabaseManager::try_get().ok_or("数据库未初始化")?;

    let week_end = arg_value(&args, "--date")
        .map(str::to_string)
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());

    let report = ticket_weekly_report::build(&db, &week_end)?;
    for error in &report.errors {
        eprintln!("[选股周报] {error}");
    }
    print!("{}", report.render());
    Ok(())
}
