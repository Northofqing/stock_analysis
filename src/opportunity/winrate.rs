//! BR-017: the verified `prediction_tracker` is the single win-rate source.

use diesel::prelude::*;

const MIN_SAMPLES: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedOutcome {
    pub hit: Option<bool>,
    pub actual_change: Option<f64>,
    pub special_case: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WinrateSummary {
    pub winrate: Option<f64>,
    pub mean_return: Option<f64>,
    pub sufficient: bool,
    pub total: usize,
    pub wins: usize,
    pub losses: usize,
}

impl WinrateSummary {
    /// Compatibility score for decision inputs. Insufficient evidence stays None;
    /// a verified sub-50% result is an explicit zero signal.
    pub fn gated_score(&self) -> Option<f64> {
        self.sufficient.then(|| {
            self.winrate
                .map_or(0.0, |value| if value < 0.5 { 0.0 } else { value })
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReasonWinrateReport {
    pub reason: Option<String>,
    pub t1: WinrateSummary,
    pub t3: WinrateSummary,
    pub t5: WinrateSummary,
}

/// Shared calculation used by the DB-backed production loader and tests.
pub fn summarize_verified_rows(rows: &[VerifiedOutcome]) -> WinrateSummary {
    let verified: Vec<&VerifiedOutcome> = rows.iter().filter(|row| row.hit.is_some()).collect();
    let total = verified.len();
    let wins = verified.iter().filter(|row| row.hit == Some(true)).count();
    let losses = total - wins;
    let returns: Vec<f64> = verified
        .iter()
        .filter_map(|row| row.actual_change)
        .filter(|value| value.is_finite())
        .collect();
    WinrateSummary {
        winrate: (total > 0).then(|| wins as f64 / total as f64),
        mean_return: (!returns.is_empty())
            .then(|| returns.iter().sum::<f64>() / returns.len() as f64),
        sufficient: total >= MIN_SAMPLES,
        total,
        wins,
        losses,
    }
}

#[derive(diesel::QueryableByName)]
struct PredictionOutcomeRow {
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    hit_t1: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    hit_t3: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    hit_t5: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    actual_change_t1: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    actual_change_t3: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    actual_change_t5: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    t1_special_case: Option<String>,
}

/// Load one reason (or all reasons) directly from the canonical verified ledger.
pub fn load_reason_winrate(reason: Option<&str>) -> Result<ReasonWinrateReport, String> {
    let db = crate::database::DatabaseManager::try_get()
        .ok_or_else(|| "prediction_tracker DB is not initialized".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("prediction_tracker DB connection: {error}"))?;
    let rows: Vec<PredictionOutcomeRow> = diesel::sql_query(
        "SELECT hit_t1, hit_t3, hit_t5,
                actual_change_t1, actual_change_t3, actual_change_t5, t1_special_case
         FROM prediction_tracker
         WHERE (? IS NULL OR reason = ?)",
    )
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(reason)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Text>, _>(reason)
    .load(&mut conn)
    .map_err(|error| format!("query prediction_tracker winrate: {error}"))?;

    let window = |hit: fn(&PredictionOutcomeRow) -> Option<i32>,
                  change: fn(&PredictionOutcomeRow) -> Option<f64>,
                  include_special: bool| {
        rows.iter()
            .map(|row| VerifiedOutcome {
                hit: hit(row).map(|value| value != 0),
                actual_change: change(row),
                special_case: include_special
                    .then(|| row.t1_special_case.clone())
                    .flatten(),
            })
            .collect::<Vec<_>>()
    };
    let t1 = window(|row| row.hit_t1, |row| row.actual_change_t1, true);
    let t3 = window(|row| row.hit_t3, |row| row.actual_change_t3, false);
    let t5 = window(|row| row.hit_t5, |row| row.actual_change_t5, false);
    Ok(ReasonWinrateReport {
        reason: reason.map(str::to_string),
        t1: summarize_verified_rows(&t1),
        t3: summarize_verified_rows(&t3),
        t5: summarize_verified_rows(&t5),
    })
}
