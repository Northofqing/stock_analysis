//! Read-only production Magic TDX capability probe for event selection.

use anyhow::{bail, Result};
use chrono::Local;
use clap::Parser;
use std::collections::BTreeSet;
use stock_analysis::calendar::{current_session, latest_completed_trading_day_at};
use stock_analysis::selection::features::compute_daily_features;
use stock_analysis::selection::magic_tdx::{
    fetch_selection_market_batch, validate_production_stock_code, SelectionEventReference,
    SelectionMarketRequest, SelectionMarketWindow,
};

#[derive(Debug, Parser)]
#[command(
    name = "selection_live_probe",
    about = "Read validated selection evidence from production Magic TDX without writing data"
)]
struct Args {
    #[arg(long = "code", required = true, num_args = 1)]
    codes: Vec<String>,
}

fn validated_codes(args: &Args) -> Result<Vec<String>> {
    if args.codes.is_empty() {
        bail!("at least one explicit SH/SZ --code is required");
    }
    let mut codes = BTreeSet::new();
    for raw in &args.codes {
        let code = raw.trim();
        validate_production_stock_code(code)?;
        codes.insert(code.to_owned());
    }
    Ok(codes.into_iter().collect())
}

fn market_window() -> SelectionMarketWindow {
    if current_session().is_trading() {
        SelectionMarketWindow::Intraday
    } else {
        SelectionMarketWindow::PostClose
    }
}

async fn run(args: Args) -> Result<String> {
    let codes = validated_codes(&args)?;
    let evaluation_at = Local::now();
    let expected_latest_settled_date = latest_completed_trading_day_at(evaluation_at.naive_local());
    let event_references = codes
        .iter()
        .enumerate()
        .map(|(index, code)| SelectionEventReference {
            event_id: format!("selection_live_probe_{index:04}"),
            text: code.clone(),
        })
        .collect();
    let batch = fetch_selection_market_batch(SelectionMarketRequest {
        event_references,
        window: market_window(),
        evaluation_at,
        expected_latest_settled_date,
    })
    .await?;

    let mut rendered = format!(
        "Magic TDX selection live probe\nbatch_id={}\nsource_observed_at={}\nmaster_batch_id={}\nmaster_observed_at={}\n",
        batch.batch_id,
        batch.observed_at.to_rfc3339(),
        batch.master.batch_id,
        batch.master.observed_at.to_rfc3339(),
    );
    for code in codes {
        let record = batch
            .records
            .iter()
            .find(|record| record.security.code == code)
            .ok_or_else(|| {
                let reasons = batch
                    .rejections
                    .iter()
                    .filter(|rejection| rejection.security_code.as_deref() == Some(code.as_str()))
                    .map(|rejection| rejection.reason_code.as_str())
                    .collect::<Vec<_>>();
                anyhow::anyhow!(
                    "Magic TDX returned no validated record for {code}; reason_codes={reasons:?}"
                )
            })?;
        let feature_available = compute_daily_features(&record.daily_bars).is_ok();
        rendered.push_str(&format!(
            "code={} name={} market={:?} record_observed_at={} validated_daily_bars={} quote_available={} validated_five_minute_bars={} feature_available={}\n",
            record.security.code,
            record.security.name,
            record.security.market,
            record.observed_at.to_rfc3339(),
            record.daily_bars.len(),
            record.quote.is_some(),
            record.five_minute_bars.len(),
            feature_available,
        ));
    }
    Ok(rendered)
}

#[tokio::main]
async fn main() -> Result<()> {
    let rendered = run(Args::parse()).await?;
    print!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_requires_at_least_one_explicit_code() {
        assert!(Args::try_parse_from(["selection_live_probe"]).is_err());
    }

    #[test]
    fn identity_gate_accepts_only_real_supported_sh_sz_codes() {
        let args = Args::try_parse_from([
            "selection_live_probe",
            "--code",
            "600396",
            "--code",
            "002421",
        ])
        .expect("valid codes");
        assert_eq!(
            validated_codes(&args).expect("validated codes"),
            ["002421", "600396"]
        );

        for code in ["TEST_CODE_600396", "920001", "60039", "ABCDEF"] {
            let args =
                Args::try_parse_from(["selection_live_probe", "--code", code]).expect("CLI shape");
            assert!(validated_codes(&args).is_err(), "must reject {code}");
        }
    }

    #[test]
    fn command_definition_is_valid() {
        Args::command().debug_assert();
    }

    #[test]
    fn production_source_contains_no_database_delivery_or_trading_path() {
        let source = include_str!("selection_live_probe.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        for forbidden in [
            "DatabaseManager",
            "SqliteConnection",
            "INSERT INTO",
            "push_wechat",
            "place_order",
            "TradingBus",
        ] {
            assert!(!source.contains(forbidden), "forbidden path: {forbidden}");
        }
        assert!(source.contains("fetch_selection_market_batch"));
    }
}
