//! BR-147 one-shot closing valuation runner.
//! Uses only the user-confirmed snapshot and admitted unadjusted settled closes.

use chrono::{Local, NaiveDate};
use diesel::prelude::*;
use stock_analysis::data_gateway::{AdmittedDailyBars, BatchEvidence, HistoricalBarsGateway};
use stock_analysis::data_provider::AdjustType;
use stock_analysis::database::{self, DatabaseManager};
use stock_analysis::portfolio::closing_valuation::{
    calculate_closing_valuation, ClosingPriceEvidence,
};

#[derive(QueryableByName)]
struct DailyRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    date: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    close: Option<f64>,
}

fn settled_closes_from_admitted(
    code: &str,
    batch: &AdmittedDailyBars,
) -> Result<Vec<(NaiveDate, f64)>, String> {
    settled_closes_from_parts(code, batch.records(), batch.evidence())
}

fn settled_closes_from_parts(
    code: &str,
    records: &[stock_analysis::data_provider::KlineData],
    evidence: &BatchEvidence,
) -> Result<Vec<(NaiveDate, f64)>, String> {
    records
        .iter()
        .map(|bar| {
            if bar.adjust != AdjustType::None
                || !bar.settled
                || !bar.close.is_finite()
                || bar.close <= 0.0
            {
                return Err(format!(
                    "BR-147 {code} rejected daily bar date={} adjust={} settled={} close={} source={} batch_id={}",
                    bar.date,
                    bar.adjust.as_str(),
                    bar.settled,
                    bar.close,
                    evidence.source,
                    evidence.batch_id
                ));
            }
            Ok((bar.date, bar.close))
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let date = std::env::args()
        .nth(1)
        .map(|v| NaiveDate::parse_from_str(&v, "%Y-%m-%d"))
        .transpose()?
        .unwrap_or_else(|| Local::now().date_naive());
    DatabaseManager::init(Some("data/stock_analysis.db".into()))?;
    let snapshot = database::user_position_snapshot::latest_user_position_snapshot()?
        .ok_or("no user-confirmed position snapshot")?;
    let gateway = HistoricalBarsGateway::new();
    let mut prices = Vec::new();
    let mut previous = Vec::new();
    for item in &snapshot.items {
        let mut gateway_error = None;
        let (mut closes, mut price_provider, mut provider_batch_id) =
            match gateway.required_daily_bars(&item.code, 10) {
                Ok(batch) => {
                    eprintln!(
                        "[BR-147] code={} provider={:?} source={} batch_id={} records={}",
                        item.code,
                        batch.evidence().provider,
                        batch.evidence().source,
                        batch.evidence().batch_id,
                        batch.records().len()
                    );
                    let closes = settled_closes_from_admitted(&item.code, &batch)?;
                    (
                        closes,
                        batch.evidence().source.clone(),
                        batch.evidence().batch_id.clone(),
                    )
                }
                Err(error) => {
                    let message = format!("unified daily batch unavailable: {error}");
                    eprintln!(
                        "[BR-147] {} {message}; trying validated stock_daily",
                        item.code
                    );
                    gateway_error = Some(message);
                    (Vec::new(), String::new(), String::new())
                }
            };
        if !closes.iter().any(|(row_date, _)| *row_date == date) {
            let mut conn = DatabaseManager::get().get_conn()?;
            let rows: Vec<DailyRow> = diesel::sql_query("SELECT date, close FROM stock_daily WHERE code=? AND date<=? AND close>0 ORDER BY date DESC LIMIT 10")
                .bind::<diesel::sql_types::Text, _>(&item.code)
                .bind::<diesel::sql_types::Text, _>(date.to_string())
                .load(&mut conn)?;
            closes = rows
                .into_iter()
                .map(|row| {
                    let row_date =
                        NaiveDate::parse_from_str(&row.date, "%Y-%m-%d").map_err(|error| {
                            format!("{} stock_daily date {:?}: {error}", item.code, row.date)
                        })?;
                    let close = row
                        .close
                        .filter(|value| value.is_finite() && *value > 0.0)
                        .ok_or_else(|| {
                            format!(
                                "{} stock_daily close missing/invalid at {row_date}",
                                item.code
                            )
                        })?;
                    Ok((row_date, close))
                })
                .collect::<Result<Vec<_>, String>>()?;
            price_provider = "stock_daily_backfill".to_string();
            provider_batch_id = format!("stock_daily:{}:{date}", item.code);
        }
        let current = closes.iter().find(|(d, _)| *d == date).ok_or_else(|| {
            format!(
                "{} missing settled close for {date}; gateway={}",
                item.code,
                gateway_error
                    .as_deref()
                    .unwrap_or("batch lacked exact date")
            )
        })?;
        prices.push(ClosingPriceEvidence {
            code: item.code.clone(),
            price_date: date,
            close: current.1,
            provider: price_provider.clone(),
            evidence_hash: format!(
                "{}:{}:{:.6}:{}:{}",
                item.code, date, current.1, price_provider, provider_batch_id
            ),
        });
        if let Some((_, prev)) = closes.iter().find(|(d, _)| *d < date) {
            previous.push((item.code.clone(), *prev));
        }
    }
    let view = calculate_closing_valuation(
        &snapshot.items,
        &prices,
        &previous,
        date,
        "unified_daily_close",
    )?;
    let receipt = database::closing_valuation::save_closing_valuation(&view)?;
    println!(
        "run_id={} inserted={} covered={}/{} price_date={}",
        receipt.run_id, receipt.inserted, view.covered, view.total, view.price_date
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::magic_compat::ProviderId;
    use stock_analysis::data_provider::KlineData;

    fn evidence() -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_magic_tdx_daily".to_string(),
            source_at: Some("2026-07-26".to_string()),
            observed_at: "2026-07-26T08:00:00Z".to_string(),
            batch_id: "TEST_CODE_closing_runner_batch".to_string(),
        }
    }

    fn bar(date: NaiveDate, close: f64, settled: bool) -> KlineData {
        KlineData {
            date,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000.0,
            amount: 10_000.0,
            pct_chg: 1.0,
            intraday_price: None,
            settled,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            adjust: AdjustType::None,
        }
    }

    #[test]
    fn br147_runner_rejects_unsettled_rows_without_partial_projection() {
        let latest = NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();
        let records = vec![
            bar(latest, 12.5, true),
            bar(latest - chrono::Duration::days(1), 11.8, false),
        ];
        let evidence = evidence();

        let error = settled_closes_from_parts("TEST_CODE_600001", &records, &evidence)
            .expect_err("unsettled row must reject the complete batch");

        assert!(error.contains("rejected daily bar"));
        assert!(error.contains("TEST_CODE_closing_runner_batch"));
    }
}
