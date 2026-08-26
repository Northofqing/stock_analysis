//! BR-248 经济仓位只读历史探针。
//!
//! 必须显式指定数据库与评估日；SQLite 以 READ_ONLY 打开。探针不接受费用假设，
//! 因而只验证成交事实和闭环数量，净指标保持 Unavailable。

use std::path::PathBuf;
use std::process::ExitCode;

use chrono::NaiveDate;
use stock_analysis::performance::attribution_replay::{
    AttributionReplayLoader, AttributionReplayRequest, FeeEvidenceAvailability,
};
use stock_analysis::performance::economic_position::{rebuild_economic_positions, NetSummary};

struct Args {
    database: PathBuf,
    as_of_date: NaiveDate,
}

fn parse_args() -> Result<Args, String> {
    let mut database = None;
    let mut as_of_date = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--db" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--db requires an explicit path".to_owned())?;
                database = Some(PathBuf::from(value));
            }
            "--as-of" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--as-of requires YYYY-MM-DD".to_owned())?;
                as_of_date = Some(
                    NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                        .map_err(|error| format!("--as-of invalid: {error}"))?,
                );
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        database: database.ok_or_else(|| "--db is required".to_owned())?,
        as_of_date: as_of_date.ok_or_else(|| "--as-of is required".to_owned())?,
    })
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let evidence = AttributionReplayLoader::new(&args.database)
        .load(&AttributionReplayRequest {
            from: args.as_of_date,
            to: args.as_of_date,
            required_trading_dates: vec![args.as_of_date],
            fee_ledger: None,
        })
        .map_err(|error| format!("BR-251 replay evidence: {error}"))?;
    if !matches!(evidence.fees, FeeEvidenceAvailability::Unavailable { .. }) {
        return Err("read-only probe unexpectedly received fee authority".to_owned());
    }
    let fills = evidence
        .fills
        .into_iter()
        .map(|evidence| evidence.fill)
        .collect::<Vec<_>>();
    let report = rebuild_economic_positions(&fills, args.as_of_date, None)?;
    println!("BR-248 经济仓位只读探针");
    println!("评估日: {}", report.as_of_date);
    println!("来源成交: {}", report.source_fill_ids.len());
    println!("闭合经济仓位: {}", report.closed_positions.len());
    println!("开放右删失仓位: {}", report.open_positions.len());
    println!(
        "覆盖天数: {}",
        report
            .coverage_days
            .map_or_else(|| "不可用".to_owned(), |days| days.to_string())
    );
    match report.net_summary {
        NetSummary::Unavailable { reason } => println!("净结果: 不可用 ({reason})"),
        NetSummary::Available { .. } => {
            return Err("read-only probe unexpectedly produced net metrics".to_owned());
        }
    }
    println!("验证状态: {:?}", report.validation_status);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("BR-248 经济仓位探针失败: {error}");
            ExitCode::FAILURE
        }
    }
}
