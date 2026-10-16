//! 手动跑一次建议结果回填 (运维工具).
//!
//! 用法:
//!   STOCK_DB=<path> cargo run --release --bin advice_backfill            # 以今天为 as_of
//!   STOCK_DB=<path> cargo run --release --bin advice_backfill -- --date 2026-10-20
//!
//! 说明: monitor 主循环每个交易日盘后 15:35 之后自动跑一次 (见 main.rs),
//!       本工具用于"手动补一次"或"排查为什么没回填".
//!
//! 口径: 以推送日 (business_date) 收盘为基准的"假设执行"结果 —— 见模块注释.

use stock_analysis::database::DatabaseManager;
use stock_analysis::monitor::advice_feedback_report::build_advice_feedback_report;
use stock_analysis::monitor::advice_outcome_backfill::backfill_advice_outcomes;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let as_of = match args.iter().position(|arg| arg == "--date") {
        Some(index) => {
            let raw = args
                .get(index + 1)
                .ok_or_else(|| "--date 需要 YYYY-MM-DD 参数".to_owned())?;
            chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .map_err(|error| format!("日期非法 ({raw}): {error}"))?
        }
        None => chrono::Local::now().date_naive(),
    };

    // 与生产 monitor 同一 retained 作用域: 全量 init 会对数百万行审计链重算哈希,
    // 建议回填只写业务表, 不需要 platform Unit 存储资格.
    let path = std::env::var("STOCK_DB").ok().map(std::path::PathBuf::from);
    DatabaseManager::init_retained_monitor(path)?;

    if args.iter().any(|arg| arg == "--report") {
        let since = match args.iter().position(|arg| arg == "--since") {
            Some(index) => args
                .get(index + 1)
                .ok_or_else(|| "--since 需要 YYYY-MM-DD 参数".to_owned())?
                .clone(),
            None => "1970-01-01".to_owned(),
        };
        let text = build_advice_feedback_report(DatabaseManager::get(), &since, as_of)?;
        if text.is_empty() {
            println!("(窗口 {since} ~ {as_of} 内无已回填样本)");
        } else {
            println!("{text}");
        }
        return Ok(());
    }

    let report = backfill_advice_outcomes(DatabaseManager::get(), as_of)?;
    println!(
        "as_of={as_of} scanned={} filled={} not_mature={} voided={}",
        report.scanned, report.filled, report.not_mature, report.voided
    );
    for error in &report.errors {
        println!("  - {error}");
    }
    Ok(())
}
