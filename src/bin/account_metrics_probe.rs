//! 2026-09-21 评估 #12: 账户模式 metrics 装配探针 (只读).
//!
//! 复现 `compute_account_mode_metrics_blocking` 的装配路径 (user_account_summary
//! + paper_trades 账本锚), 用于部署前验收: 打印三指标 + 连续止损计数来源,
//! 失败打印原因 (不写任何数据).

use std::path::PathBuf;
use std::process::ExitCode;

fn current_day_pnl_pct(
    snapshot_at: chrono::DateTime<chrono::FixedOffset>,
    evaluated_at: chrono::DateTime<chrono::FixedOffset>,
    daily_pnl: f64,
    total_assets: f64,
) -> Result<f64, String> {
    if total_assets <= 0.0 {
        return Err("total assets must be positive for account mode".to_owned());
    }
    let pnl_pct = daily_pnl / total_assets * 100.0;
    if !pnl_pct.is_finite() {
        return Err("daily PnL ratio is non-finite".to_owned());
    }
    let china_offset = chrono::FixedOffset::east_opt(8 * 60 * 60).expect("China offset");
    if snapshot_at.with_timezone(&china_offset).date_naive()
        != evaluated_at.with_timezone(&china_offset).date_naive()
    {
        return Err("account summary does not contain today's PnL".to_owned());
    }
    Ok(pnl_pct)
}

fn available_net_pnl(
    net: &stock_analysis::performance::economic_position::NetMetrics,
) -> Result<f64, String> {
    use stock_analysis::performance::economic_position::NetMetrics;
    match net {
        NetMetrics::Available { net_pnl, .. } => Ok(*net_pnl),
        NetMetrics::Unavailable { .. } => Err("closed position net PnL is unavailable".to_owned()),
    }
}

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
    stock_analysis::database::DatabaseManager::init(Some(database)).expect("database init failed");

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
            summary.effective_at,
            summary.total_assets,
            summary.position_ratio_pct,
            summary.daily_pnl
        );
        if age < chrono::Duration::zero() {
            return Err(format!("summary from the future: age={age}"));
        }
        if age > chrono::Duration::hours(96) {
            return Err(format!("summary stale: age={age}"));
        }
        if summary.source.trim().is_empty() {
            return Err("account summary source is empty".to_owned());
        }
        let today_pnl_pct = current_day_pnl_pct(
            effective_at,
            observed_at,
            summary.daily_pnl,
            summary.total_assets,
        )?;
        let total_pos_cheng = (summary.position_ratio_pct / 10.0).round().clamp(0.0, 10.0) as u8;

        let report = {
            // 与生产 compute_account_mode_metrics_blocking 同路径: 费率口径
            // 逐笔成本 ledger 喂引擎 (评估 #1), 净口径计数.
            let as_of = chrono::Local::now().date_naive();
            stock_analysis::performance::economic_position::compute_economic_position_report(as_of)
                .map_err(|error| format!("paper ledger anchor: {error}"))?
        };
        println!(
            "ledger: closed_positions={} open_positions={} (费率逐笔成本净口径)",
            report.closed_positions.len(),
            report.open_positions.len()
        );
        if let Some(opening) = &report.opening_inventory {
            println!("opening inventory: remaining_lots={} excluded_exit_parts={} projection={} seed={:?} (期初份额不计策略连续止损)",opening.remaining_opening_lots.len(),opening.excluded_exits.len(),opening.projection_hash,opening.seed_binding);
        }
        let mut realized: Vec<(chrono::NaiveDateTime, String, f64)> = report
            .closed_positions
            .iter()
            .map(|position| {
                Ok((
                    position.closed_at,
                    format!("economic-cycle-{}", position.cycle_open_fill_id),
                    available_net_pnl(&position.net)?,
                ))
            })
            .collect::<Result<_, String>>()?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::performance::economic_position::NetMetrics;

    #[test]
    fn previous_day_summary_cannot_be_reported_as_today_pnl() {
        let yesterday = chrono::DateTime::parse_from_rfc3339("2026-09-28T16:00:00+08:00").unwrap();
        let today = chrono::DateTime::parse_from_rfc3339("2026-09-29T10:00:00+08:00").unwrap();
        assert!(current_day_pnl_pct(yesterday, today, 10.0, 1000.0).is_err());
        assert_eq!(current_day_pnl_pct(today, today, 10.0, 1000.0), Ok(1.0));
    }

    #[test]
    fn unavailable_net_cost_cannot_fall_back_to_gross_pnl() {
        assert!(available_net_pnl(&NetMetrics::Unavailable {
            reason: "missing fee evidence".to_owned(),
        })
        .is_err());
    }
}
