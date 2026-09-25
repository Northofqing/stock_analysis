//! Backfill all due, pending prediction rows using their frozen target dates.
//! `STOCK_DB` selects the database. The former positional lookback is deprecated.
use stock_analysis::{database::DatabaseManager, monitor::prediction};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(days) = std::env::args().nth(1) {
        if days.parse::<u32>().ok().filter(|days| *days > 0).is_none() {
            return Err("旧 days 参数必须是正整数；新语义始终扫描所有到期 pending 行".into());
        }
        eprintln!("[backfill] days={days} 已弃用：本次扫描所有 target_date <= today 的 pending 行，不受 7/14 日窗口限制");
    }
    let path = std::env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    DatabaseManager::init(path)?;
    let report = prediction::verify_due_predictions(
        DatabaseManager::get(),
        chrono::Local::now().date_naive(),
    )?;
    println!("[backfill] 到期验证: {report:?}");
    if !report.errors.is_empty() {
        return Err("部分预测验证失败；已完成行不回滚，错误行保持 pending".into());
    }
    Ok(())
}
