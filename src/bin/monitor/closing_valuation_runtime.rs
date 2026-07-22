//! BR-147 monitor integration seam for the persisted closing valuation.
//!
//! The monitor owns scheduling; this module owns one idempotent, blocking run.
//! Callers must invoke it only after the market close and must surface errors as
//! diagnostics rather than turning them into an empty valuation.

use chrono::NaiveDate;
use diesel::prelude::*;
use stock_analysis::data_provider::{AdjustType, DataProvider, RustdxProvider};
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
    let provider = RustdxProvider::new().map_err(|e| e.to_string())?;
    let mut prices = Vec::new();
    let mut previous = Vec::new();
    for item in &snapshot.items {
        let bars = provider.get_daily_data(&item.code, 10).ok();
        let mut closes: Vec<(NaiveDate, f64)> = bars
            .as_ref()
            .map(|rows| {
                rows.iter()
                    .filter(|b| {
                        b.adjust == AdjustType::None
                            && b.settled
                            && b.close.is_finite()
                            && b.close > 0.0
                    })
                    .map(|b| (b.date, b.close))
                    .collect()
            })
            .unwrap_or_default();
        if closes.is_empty() {
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
                .filter_map(|r| {
                    Some((
                        NaiveDate::parse_from_str(&r.date, "%Y-%m-%d").ok()?,
                        r.close?,
                    ))
                })
                .collect();
        }
        let current = closes.iter().find(|(d, _)| *d == date).ok_or_else(|| {
            format!(
                "BR-147 {} missing settled RustDX close for {date}",
                item.code
            )
        })?;
        prices.push(ClosingPriceEvidence {
            code: item.code.clone(),
            price_date: date,
            close: current.1,
            provider: if bars.is_some() {
                "rustdx_none"
            } else {
                "stock_daily_backfill"
            }
            .into(),
            evidence_hash: format!("{}:{}:{:.6}", item.code, date, current.1),
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
}
