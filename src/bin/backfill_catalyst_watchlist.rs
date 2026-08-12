//! T+1 关注票跟踪 — 名单快照一次性回填 (R-13 首推前使用)。
//!
//! 用法:
//!   cargo run --bin backfill_catalyst_watchlist -- 2026-08-11
//!
//! 实现: 对给定日期读「最早落库」的 visible batch —— 即当日首次盘后复盘推送
//!       消费的那一版 —— 忠实还原用户当晚看到的名单 (不重新计算: 次日重算
//!       会得到与推送时不同的成员, 2026-08-12 实测 8/11 前排变 3/6),
//!       取带 code/streak 的名单快照, 写 `catalyst_watchlist_run` +
//!       `catalyst_watchlist_daily` (内容哈希幂等, 重复跑跳过)。
//! 回填后当晚 R-13 即核对 8/11 名单并推送。

use std::env;

use chrono::NaiveDate;
use stock_analysis::database::catalyst_watchlist::save_watchlist;
use stock_analysis::database::DatabaseManager;
use stock_analysis::review::catalyst_review::load_catalyst_review_snapshot_stored;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(date) = env::args().nth(1) else {
        eprintln!("用法: cargo run --bin backfill_catalyst_watchlist -- <YYYY-MM-DD>");
        std::process::exit(2);
    };
    let watch_date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|error| format!("非法日期 {date}: {error}"))?;

    let db_path = env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    DatabaseManager::init(db_path).map_err(|error| {
        format!("数据库初始化失败: {error}")
    })?;

    let snapshot = load_catalyst_review_snapshot_stored(&date)?;
    if snapshot.leading_entries.is_empty() && snapshot.other_entries.is_empty() {
        return Err(format!(
            "[backfill] {date} A-10 visible chain 无成员 (theme={}), 拒绝落盘空名单",
            snapshot.theme
        )
        .into());
    }

    println!(
        "[backfill] {date} A-10 主题「{}」: leading {} 只 / other {} 只",
        snapshot.theme,
        snapshot.leading_entries.len(),
        snapshot.other_entries.len()
    );
    for (position, entry) in snapshot
        .leading_entries
        .iter()
        .map(|entry| ("leading", entry))
        .chain(
            snapshot
                .other_entries
                .iter()
                .map(|entry| ("other", entry)),
        )
    {
        println!(
            "[backfill]   {position} {code} {name} (streak={streak})",
            code = entry.code,
            name = entry.name,
            streak = entry.streak,
        );
    }

    let receipt = save_watchlist(watch_date, &snapshot.leading_entries, &snapshot.other_entries)?;
    println!(
        "[backfill] 完成: run_id={} inserted={} (false=幂等跳过, 名单已存在)",
        receipt.run_id, receipt.inserted
    );
    Ok(())
}
