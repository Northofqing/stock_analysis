//! Backfill pending predictions due by the latest completed Shanghai session.
//! Existing frozen target dates are never rewritten.
//! `STOCK_DB` selects the database. The former positional lookback is deprecated.
use chrono::{DateTime, FixedOffset};
use stock_analysis::{database::DatabaseManager, monitor::prediction};

fn run_on_at(
    db: &DatabaseManager,
    now: DateTime<FixedOffset>,
) -> Result<prediction::PredictionVerificationReport, String> {
    let as_of = prediction::completed_session_as_of_at(now)?;
    prediction::verify_due_predictions(db, as_of)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(days) = std::env::args().nth(1) {
        if days.parse::<u32>().ok().filter(|days| *days > 0).is_none() {
            return Err("旧 days 参数必须是正整数；新语义始终扫描所有到期 pending 行".into());
        }
        eprintln!("[backfill] days={days} 已弃用：本次扫描所有 target_date 不晚于最近已完成上海交易日的 pending 行，不受 7/14 日窗口限制");
    }
    let path = std::env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    DatabaseManager::init(path)?;
    let report = run_on_at(DatabaseManager::get(), prediction::shanghai_now())?;
    println!("[backfill] 已完成交易日到期验证: {report:?}");
    if !report.errors.is_empty() {
        return Err("部分预测验证失败；已完成行不回滚，错误行保持 pending".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "backfill_predictions/tests.rs"]
mod tests;
