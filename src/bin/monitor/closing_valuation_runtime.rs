//! BR-147 monitor integration seam for the persisted closing valuation.
//!
//! The monitor owns scheduling; this module owns one idempotent, blocking run.
//! Callers must invoke it only after the market close and must surface errors as
//! diagnostics rather than turning them into an empty valuation.

use chrono::NaiveDate;
use diesel::prelude::*;
use stock_analysis::data_gateway::{AdmittedDailyBars, HistoricalBarsGateway};
use stock_analysis::data_provider::AdjustType;
use stock_analysis::database::{self, DatabaseManager};
use stock_analysis::portfolio::closing_valuation::{
    calculate_closing_valuation, ClosingPriceEvidence,
};

/// Runs one valuation on a blocking worker. Duplicate dates are harmless:
/// persistence is keyed by the deterministic valuation run identity.
pub async fn run_closing_valuation_once(
    date: NaiveDate,
) -> Result<database::closing_valuation::SaveClosingValuationReceipt, String> {
    tokio::task::spawn_blocking(move || run_blocking(date))
        .await
        .map_err(|e| format!("BR-147 valuation worker join failed: {e}"))?
}

/// True after the local exchange close; callers should additionally gate on a
/// trading-day calendar before invoking the worker.
pub fn eligible_after_close(now: chrono::DateTime<chrono::FixedOffset>) -> bool {
    let t = now.time();
    t >= chrono::NaiveTime::from_hms_opt(15, 0, 0).expect("valid close")
}

fn settled_closes_from_admitted(
    code: &str,
    batch: &AdmittedDailyBars,
) -> Result<Vec<(NaiveDate, f64)>, String> {
    settled_closes_from_records(code, batch.records(), batch.evidence())
}

fn settled_closes_from_records(
    code: &str,
    records: &[stock_analysis::data_provider::KlineData],
    evidence: &stock_analysis::data_gateway::BatchEvidence,
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

fn run_blocking(
    date: NaiveDate,
) -> Result<database::closing_valuation::SaveClosingValuationReceipt, String> {
    // The resident monitor initializes the singleton during startup. Reusing
    // the same process must not turn a scheduled valuation into a false
    // BR-147 failure merely because initialization already happened.
    if DatabaseManager::try_get().is_none() {
        DatabaseManager::init(Some("data/stock_analysis.db".into())).map_err(|e| e.to_string())?;
    }
    let snapshot = database::user_position_snapshot::latest_user_position_snapshot()?
        .ok_or_else(|| "BR-147 no user-confirmed position snapshot".to_string())?;
    if snapshot.confirm_empty || snapshot.items.is_empty() {
        return Err("BR-147 confirmed-empty snapshot: valuation unavailable".into());
    }
    let gateway = HistoricalBarsGateway::new();
    let mut prices = Vec::new();
    let mut previous = Vec::new();
    for item in &snapshot.items {
        let mut gateway_error = None;
        let (mut closes, mut price_provider, mut provider_batch_id) =
            match gateway.required_daily_bars(&item.code, 10) {
                Ok(batch) => {
                    log::info!(
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
                    log::warn!(
                        "[BR-147] {} {message}; trying validated stock_daily",
                        item.code
                    );
                    gateway_error = Some(message);
                    (Vec::new(), String::new(), String::new())
                }
            };
        // The routed daily batch can legitimately lag the requested settlement
        // date while provider caches are still being published. Only use
        // the validated local daily table when it contains the exact date;
        // never substitute the latest prior close for the requested close.
        if !closes.iter().any(|(d, _)| *d == date) {
            #[derive(diesel::QueryableByName)]
            struct DailyCloseRow {
                #[diesel(sql_type = diesel::sql_types::Text)]
                date: String,
                #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
                close: Option<f64>,
            }
            let mut conn = DatabaseManager::get()
                .get_conn()
                .map_err(|e| format!("{} stock_daily connection: {e}", item.code))?;
            let rows: Vec<DailyCloseRow> = diesel::sql_query(
                "SELECT date, close FROM stock_daily WHERE code=? AND date<=? AND close>0 ORDER BY date DESC LIMIT 10",
            )
            .bind::<diesel::sql_types::Text, _>(&item.code)
            .bind::<diesel::sql_types::Text, _>(date.to_string())
            .load(&mut conn)
            .map_err(|e| format!("{} stock_daily query: {e}", item.code))?;
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
                "BR-147 {} missing validated close for {date}; gateway={}; stock_daily exact date unavailable",
                item.code,
                gateway_error.as_deref().unwrap_or("batch lacked exact date")
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
        "validated_daily_close",
    )?;
    database::closing_valuation::save_closing_valuation(&view)
        .map_err(|e| format!("BR-147 persist failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use magic_market_core::ProviderId;
    use stock_analysis::data_gateway::BatchEvidence;
    use stock_analysis::data_provider::KlineData;

    fn evidence() -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_magic_tdx_daily".to_string(),
            source_at: Some("2026-07-26".to_string()),
            observed_at: "2026-07-26T08:00:00Z".to_string(),
            batch_id: "TEST_CODE_closing_runtime_batch".to_string(),
        }
    }

    fn bar(date: NaiveDate, close: f64) -> KlineData {
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
            settled: true,
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
    fn eligibility_starts_at_close() {
        let before = Local::now()
            .date_naive()
            .and_hms_opt(14, 59, 59)
            .unwrap()
            .and_local_timezone(*Local::now().offset())
            .single()
            .unwrap();
        assert!(!eligible_after_close(before));
        let after = Local::now()
            .date_naive()
            .and_hms_opt(15, 0, 0)
            .unwrap()
            .and_local_timezone(*Local::now().offset())
            .single()
            .unwrap();
        assert!(eligible_after_close(after));
    }

    #[test]
    fn br147_settled_close_projection_preserves_newest_first_batch() {
        let latest = NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();
        let records = vec![
            bar(latest, 12.5),
            bar(latest - chrono::Duration::days(1), 11.8),
        ];
        let evidence = evidence();

        assert_eq!(
            settled_closes_from_records("TEST_CODE_600001", &records, &evidence).unwrap(),
            vec![(latest, 12.5), (latest - chrono::Duration::days(1), 11.8)]
        );
        assert_eq!(evidence.batch_id, "TEST_CODE_closing_runtime_batch");
    }
}
