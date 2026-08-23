//! BR-248 经济仓位只读历史探针。
//!
//! 必须显式指定数据库与评估日；SQLite 以 READ_ONLY 打开。探针不接受费用假设，
//! 因而只验证成交事实和闭环数量，净指标保持 Unavailable。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::NaiveDate;
use rusqlite::{Connection, OpenFlags};
use stock_analysis::performance::economic_position::{
    rebuild_economic_positions, select_economic_rows_through, EconomicFillRow, NetSummary,
};

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

fn read_fills(database: &Path) -> Result<Vec<EconomicFillRow>, String> {
    if !database.is_file() {
        return Err(format!(
            "database path is not an existing file: {}",
            database.display()
        ));
    }
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("open database read-only: {error}"))?;
    let mut statement = connection
        .prepare(
            "SELECT id, plan_id, code, name, direction, fill_price, quantity, \
             CAST(ts AS TEXT), virtual_reason \
             FROM paper_trades WHERE status = 'Filled' ORDER BY ts ASC, id ASC",
        )
        .map_err(|error| format!("prepare read-only paper_trades query: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(EconomicFillRow {
                id: row.get(0)?,
                plan_id: row.get(1)?,
                code: row.get(2)?,
                name: row.get(3)?,
                direction: row.get(4)?,
                fill_price: row.get(5)?,
                quantity: row.get(6)?,
                occurred_at: row.get(7)?,
                virtual_reason: row.get(8)?,
            })
        })
        .map_err(|error| format!("read paper_trades: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("decode paper_trades: {error}"))
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let all_rows = read_fills(&args.database)?;
    let through = select_economic_rows_through(all_rows, args.as_of_date)?;
    let report = rebuild_economic_positions(&through, args.as_of_date, None)?;
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
