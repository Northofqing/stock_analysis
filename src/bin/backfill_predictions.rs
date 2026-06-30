//! 一次性回填历史 prediction 的 actual_change 和 hit
//!
//! 用法:
//!   cargo run --bin backfill_predictions -- 14
//!   cargo run --bin backfill_predictions -- 30
//!
//! 实现: 循环过去 N 天的每个日期, 把 prediction_tracker 中 hit IS NULL 的行,
//!       按 (pred_date+1) 作为 target_date 重跑 verify 逻辑。
//!       核心 verify 计算复用 `monitor::prediction::verify_one`, 与生产盘后回填保持一致。
//!
//! 配合 `tools/one_shot/backfill_predictions.sh` 使用。

use chrono::{Duration, Local};
use std::env;
use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::prediction;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let days: i64 = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(14);

    let db_path = env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    let _ = DatabaseManager::init(db_path);

    let db = DatabaseManager::get();
    let today = Local::now().date_naive();

    let mut total = 0usize;
    let mut hit_count = 0usize;

    for offset in 1..=days {
        let pred_date = today - Duration::days(offset);
        let target_date = pred_date + Duration::days(1);
        let pred_date_s = pred_date.format("%Y-%m-%d").to_string();
        let target_date_s = target_date.format("%Y-%m-%d").to_string();

        let pending = db.get_pending_predictions(&pred_date_s)?;
        if pending.is_empty() {
            continue;
        }

        println!(
            "[backfill] {} -> {}: {} 条 pending",
            pred_date_s,
            target_date_s,
            pending.len()
        );

        for pred in pending {
            let Some(code) = pred.stock_code.as_deref() else { continue; };
            if code.is_empty() { continue; }

            // 复用共享 verify 逻辑 (与生产盘后回填 verify_predictions 完全一致)
            let Some(outcome) = prediction::verify_one(
                &db, code, &pred_date_s, &target_date_s, &pred.pred_direction,
            ).await else { continue; };

            db.update_prediction_result(&pred_date_s, Some(code), outcome.actual_change, outcome.hit)?;
            total += 1;
            if outcome.hit { hit_count += 1; }
        }
    }

    println!(
        "[backfill] 完成: {} 条已 verify, 命中 {} 条 ({:.0}%)",
        total,
        hit_count,
        if total > 0 { hit_count as f64 / total as f64 * 100.0 } else { 0.0 }
    );

    Ok(())
}
