//! BR-157 read-only raw report for visible event-scoped shadow samples.

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use diesel::connection::SimpleConnection;
use diesel::{Connection, SqliteConnection};
use std::path::PathBuf;
use stock_analysis::database::selection::{ReportFilter, SelectionRepository};
use stock_analysis::selection::report::{build_report, render_text};

#[derive(Debug, Parser)]
#[command(
    name = "selection_backtest",
    about = "Read immutable visible T0/D1 shadow outcomes without changing the database"
)]
struct Args {
    #[arg(long, value_parser = parse_date)]
    from: Option<NaiveDate>,
    #[arg(long, value_parser = parse_date)]
    to: Option<NaiveDate>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    chain: Option<String>,
    #[arg(long)]
    code: Option<String>,
    #[arg(long, default_value_t = 10_000)]
    limit: usize,
    #[arg(long)]
    database: Option<PathBuf>,
}

fn parse_date(value: &str) -> std::result::Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|error| format!("invalid date {value:?}: {error}"))
}

fn database_path(args: &Args) -> PathBuf {
    args.database
        .clone()
        .or_else(|| std::env::var_os("DATABASE_PATH").map(PathBuf::from))
        .or_else(|| std::env::var_os("STOCK_DB").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("./data/stock_analysis.db"))
}

fn report_filter(args: &Args) -> ReportFilter {
    ReportFilter {
        from_market_date: args.from,
        to_market_date: args.to,
        provider: args.provider.clone(),
        chain_id: args.chain.clone(),
        stock_code: args.code.clone(),
        limit: args.limit,
    }
}

fn run(args: Args) -> Result<String> {
    let path = database_path(&args);
    if !path.is_file() {
        bail!(
            "selection report database does not exist or is not a file: {}",
            path.display()
        );
    }
    let database_url = path
        .to_str()
        .context("selection report database path is not valid UTF-8")?;
    let mut connection = SqliteConnection::establish(database_url)
        .with_context(|| format!("open selection report database {}", path.display()))?;
    connection
        .batch_execute("PRAGMA query_only = ON; PRAGMA foreign_keys = ON;")
        .context("enable SQLite query-only mode")?;
    let filter = report_filter(&args);
    let samples = SelectionRepository::new(&mut connection)
        .visible_samples(&filter)
        .context("load visible selection samples")?;
    let report = build_report(&samples, &filter).context("build raw selection report")?;
    Ok(render_text(&report))
}

fn main() -> Result<()> {
    let rendered = run(Args::parse())?;
    print!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_accepts_exact_report_filters() {
        let args = Args::try_parse_from([
            "selection_backtest",
            "--from",
            "2026-07-23",
            "--to",
            "2026-08-23",
            "--provider",
            "provider-a",
            "--chain",
            "power-grid",
        ])
        .expect("valid CLI");

        let filter = report_filter(&args);
        assert_eq!(
            filter.from_market_date,
            NaiveDate::from_ymd_opt(2026, 7, 23)
        );
        assert_eq!(filter.provider.as_deref(), Some("provider-a"));
        assert_eq!(filter.chain_id.as_deref(), Some("power-grid"));
    }

    #[test]
    fn cli_rejects_invalid_dates() {
        assert!(Args::try_parse_from(["selection_backtest", "--from", "2026-07-99"]).is_err());
    }

    #[test]
    fn command_definition_is_valid() {
        Args::command().debug_assert();
    }

    #[test]
    fn production_source_contains_no_write_or_delivery_path() {
        let source = include_str!("selection_backtest.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        for forbidden in [
            "INSERT INTO",
            "UPDATE selection_",
            "DELETE FROM",
            "run_migrations",
            "push_wechat",
            "place_order",
        ] {
            assert!(!source.contains(forbidden), "forbidden path: {forbidden}");
        }
        assert!(source.contains("PRAGMA query_only = ON"));
    }
}
