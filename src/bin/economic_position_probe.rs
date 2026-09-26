//! BR-248 经济仓位只读历史探针。
//!
//! 必须显式指定数据库与评估日；SQLite 以 READ_ONLY 打开。
//! 明确 paper-request 时使用统一有效投影及 Scenario 费用，否则仅验证旧 raw 事实。

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
    paper_request: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut database = None;
    let mut as_of_date = None;
    let mut paper_request = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--paper-request" => {
                paper_request = Some(PathBuf::from(
                    args.next()
                        .ok_or("--paper-request requires explicit manifest path")?,
                ));
            }
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
        paper_request,
    })
}

#[cfg(test)]
#[test]
fn effective_fill_probe_accepts_explicit_scope_manifest() {
    let result = parse_args_from(
        [
            "--db",
            "TEST_CODE.db",
            "--as-of",
            "2026-09-15",
            "--paper-request",
            "TEST_CODE_scope.json",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    assert!(result.is_ok(), "explicit effective probe is not reachable");
    assert_eq!(
        result.unwrap().paper_request,
        Some(PathBuf::from("TEST_CODE_scope.json"))
    );
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    if let Some(path) = &args.paper_request {
        use stock_analysis::database::attribution_reports::{
            AttributionDatabaseAccess, AttributionDatabaseSession,
        };
        use stock_analysis::trading::paper_ledger::{EffectiveFillRequest, PaperLedger};
        let request: EffectiveFillRequest =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if request.as_of != args.as_of_date {
            return Err("explicit paper request as_of differs from --as-of".into());
        }
        let session =
            AttributionDatabaseSession::open(&args.database, AttributionDatabaseAccess::ReadOnly)
                .map_err(|e| e.to_string())?;
        let effective = PaperLedger::open(session.database(), &chrono::Utc::now)
            .verified_effective_fills(&request)
            .map_err(|e| e.to_string())?;
        let report =
            stock_analysis::performance::economic_position::report_from_effective(&effective)?;
        println!("经济仓位只读有效投影；费用：Scenario（模型假设，非实扣）\n评估日：{}；完整策略闭环：{}；开放策略仓位：{}\n净指标：{:?}",report.as_of_date,report.closed_positions.len(),report.open_positions.len(),report.net_summary);
        println!("{}",serde_json::to_string_pretty(&serde_json::json!({"schema":"PaperEffectiveEconomicProbeV1","mode":"read_only","fee_kind":"Scenario","projection":effective.receipt(),"lineage":effective.lineage(),"opening_inventory":report.opening_inventory})).map_err(|e|e.to_string())?);
        return Ok(());
    }
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
