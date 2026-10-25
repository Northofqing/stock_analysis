//! 持仓财报报告（CLI）：从 financial_statement 表算单季指标并渲染建议。
//!
//!   holding_earnings_report --db <path> [--codes 002131,600396] [--period 2026-06-30]
//!
//! 只读；同步(`sync`)由 monitor 的 20:30 调度负责，或 scripts/sync_financial_statements.py。

use diesel::Connection as _;
use diesel::SqliteConnection;
use stock_analysis::monitor::holding_earnings;

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let db = arg(&args, "--db")
        .or_else(|| std::env::var("STOCK_DB").ok())
        .unwrap_or_else(|| "data/stock_analysis.db".to_string());
    let mut conn = SqliteConnection::establish(&format!("sqlite://{db}"))?;
    holding_earnings::ensure_schema(&mut conn)?;

    let codes: Vec<String> = match arg(&args, "--codes") {
        Some(raw) => raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        None => holding_earnings::covered_codes(&mut conn),
    };
    let period = arg(&args, "--period");

    for code in &codes {
        let target = match &period {
            Some(period) => Some(period.clone()),
            None => latest_period(&mut conn, code),
        };
        let Some(target) = target else {
            println!("{code}: 无财报数据（先跑 sync）");
            continue;
        };
        let Some(metrics) = holding_earnings::compute(&mut conn, code, &target) else {
            println!("{code}: 无法计算（缺收入表）");
            continue;
        };
        let advice = holding_earnings::advise(&metrics);
        println!("{}", holding_earnings::render(&metrics, &advice, false));
        println!();
    }
    Ok(())
}

fn latest_period(conn: &mut SqliteConnection, code: &str) -> Option<String> {
    use diesel::sql_types::Text;
    use diesel::RunQueryDsl as _;
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        report_period: String,
    }
    diesel::sql_query(
        "SELECT report_period FROM financial_statement WHERE code = ?1 AND kind = 'Income' ORDER BY report_period DESC LIMIT 1",
    )
    .bind::<Text, _>(code)
    .get_result::<Row>(conn)
    .ok()
    .map(|row| row.report_period)
}
