//! 观察池选股表现（手动跑版）。
//!
//!     STOCK_DB=<path> watch_pool_perf
//!     STOCK_DB=<path> watch_pool_perf --since 2026-10-01 --days 1,3,5
//!
//! 统计逻辑在 `stock_analysis::monitor::watch_pool_perf::render`，
//! monitor 的 16:05 调度用的是同一个函数，所以这里看到什么，存档就是什么。

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::watch_pool_perf::{render, PerfOptions};

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().collect();

    let db_path = std::env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    DatabaseManager::init(db_path)?;
    let db = DatabaseManager::try_get().ok_or("数据库未初始化")?;
    let mut conn = db.get_conn()?;

    // 不传 --days：按分型分窗（情绪 D+1/D+3、趋势 D+5/D+10，D2）。
    // 传了 --days：按显式窗口统计，临时对账用。
    let explicit_periods: Option<Vec<usize>> = arg_value(&args, "--days").map(|value| {
        value
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect()
    });

    let options = PerfOptions {
        periods: explicit_periods.clone().unwrap_or_else(|| vec![1, 3, 5]),
        since: arg_value(&args, "--since"),
        per_regime: explicit_periods.is_none(),
    };

    print!("{}", render(&mut conn, &options)?);
    Ok(())
}
