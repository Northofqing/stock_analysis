//! 生成观察池（票的一等公民第 2 步）.
//!
//! 用法:
//!   STOCK_DB=<path> cargo run --release --bin watch_pool_build -- --dry-run
//!   STOCK_DB=<path> cargo run --release --bin watch_pool_build
//!   STOCK_DB=<path> cargo run --release --bin watch_pool_build -- --date 2026-10-19 --lookback 5
//!
//! 口径: 新闻催化 (news_ai_assessment 的 positive/major_positive + target_code)
//!       板块轮动这一路未接 —— board_rotation_daily 的写入方
//!       (save_board_rotations) 零生产调用, 数据停在 2026-07-16.
//!
//! 建议: 首次运行加 `--dry-run` 只看不写, 确认候选合理后再落库.

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::ticket_watch_pool::build_watch_pool;

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(|value| value.as_str())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let date = match arg_value(&args, "--date") {
        Some(raw) => chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|error| format!("--date 非法 ({raw}): {error}"))?,
        None => chrono::Local::now().date_naive(),
    };
    let lookback = match arg_value(&args, "--lookback") {
        Some(raw) => Some(
            raw.parse::<i64>()
                .map_err(|error| format!("--lookback 非法 ({raw}): {error}"))?,
        ),
        None => None,
    };

    let path = std::env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    DatabaseManager::init_retained_monitor(path)?;
    let db = DatabaseManager::get();

    // CLI 不拉实时涨停池：离线重放时按历史日线排序即可，
    // 涨停优先只在 monitor 的 15:40 调度里生效（那里有实时 LimitPools）。
    let limit_up_codes = std::collections::HashSet::new();
    let leader_codes = std::collections::HashSet::new();
    // CLI 不带实时东财成分股：离线重放只用 board_member（TDX）那一路。
    let eastmoney_members = std::collections::HashMap::new();
    let report = build_watch_pool(
        db,
        &date.format("%Y-%m-%d").to_string(),
        lookback,
        dry_run,
        &limit_up_codes,
        &leader_codes,
        &eastmoney_members,
    )?;
    println!(
        "\n汇总: scanned={} candidates={} opened={} appended={} skipped_invalid={}{}",
        report.scanned_rows,
        report.candidates,
        report.opened,
        report.appended,
        report.skipped_invalid_code,
        if report.dry_run {
            "  (dry-run, 未写库)"
        } else {
            ""
        }
    );
    for error in &report.errors {
        println!("  ! {error}");
    }
    Ok(())
}
