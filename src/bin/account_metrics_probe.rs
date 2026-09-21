//! 2026-09-21 评估 #12: 账户模式 metrics 装配探针 (只读).
//!
//! 复现 `compute_account_mode_metrics_blocking` 的装配路径 (user_account_summary
//! + paper_trades 账本锚), 用于部署前验收: 打印三指标 + 连续止损计数来源,
//! 失败打印原因 (不写任何数据).

use std::path::PathBuf;
use std::process::ExitCode;

fn parse_args() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--db" {
            return args
                .next()
                .ok_or_else(|| "--db requires an explicit path".to_owned())
                .map(PathBuf::from);
        }
    }
    Ok(PathBuf::from("data/stock_analysis.db"))
}

fn main() -> ExitCode {
    let database = match parse_args() {
        Ok(db) => db,
        Err(error) => {
            eprintln!("args: {error}");
            return ExitCode::from(2);
        }
    };
    stock_analysis::database::DatabaseManager::init(Some(database))
        .expect("database init failed");

    let run = || -> Result<(), String> {
        let observed_at = chrono::Local::now().fixed_offset();
        let summary = stock_analysis::database::user_account_summary::latest()
            .map_err(|error| format!("latest user account summary: {error}"))?
            .ok_or_else(|| "user account summary is missing".to_owned())?;
        let effective_at = chrono::DateTime::parse_from_rfc3339(&summary.effective_at)
            .map_err(|error| format!("effective_at unparseable: {error}"))?;
        let age = observed_at.signed_duration_since(effective_at);
        println!(
            "summary: effective_at={} age={age} total={:.2} pos={:.1}% pnl={:.2}",
            summary.effective_at, summary.total_assets, summary.position_ratio_pct, summary.daily_pnl
        );
        if age < chrono::Duration::zero() {
            return Err(format!("summary from the future: age={age}"));
        }
        if age > chrono::Duration::hours(96) {
            return Err(format!("summary stale: age={age}"));
        }
        let today_pnl_pct = summary.daily_pnl / summary.total_assets * 100.0;
        let total_pos_cheng = (summary.position_ratio_pct / 10.0).round().clamp(0.0, 10.0) as u8;

        let report = stock_analysis::performance::economic_position::compute_economic_position_report(
            chrono::Local::now().date_naive(),
            None,
        )
        .map_err(|error| format!("paper ledger anchor: {error}"))?;
        println!(
            "ledger: closed_positions={} open_positions={}",
            report.closed_positions.len(),
            report.open_positions.len()
        );
        let mut realized: Vec<(chrono::NaiveDateTime, String, f64)> = report
            .closed_positions
            .iter()
            .map(|position| {
                (
                    position.closed_at,
                    format!("economic-cycle-{}", position.cycle_open_fill_id),
                    position.gross_pnl,
                )
            })
            .collect();
        realized.sort_by(|left, right| right.0.cmp(&left.0));
        println!("recent closed cycles (newest first):");
        for (closed_at, identity, pnl) in realized.iter().take(8) {
            println!("  {closed_at}  {identity}  pnl={pnl:+.2}");
        }
        let consecutive = realized
            .iter()
            .take(5)
            .take_while(|(_, _, pnl)| *pnl < 0.0)
            .count();
        println!(
            "metrics: today_pnl_pct={today_pnl_pct:+.3} total_pos_cheng={total_pos_cheng} consecutive_stop_loss_n={consecutive}"
        );
        Ok(())
    };

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("account metrics probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}
