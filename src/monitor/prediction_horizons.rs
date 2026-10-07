//! T+1/3/5 close-to-close directional observations in the existing ledger.
//! These do not qualify delivery, entry execution, costs or benchmark alpha.
use super::verifier::{read_exact_close_on, recorded_direction_hit};
use crate::database::DatabaseManager;
use chrono::NaiveDate;
use diesel::{Connection, OptionalExtension, RunQueryDsl};

#[derive(Debug, Clone, Default)]
pub struct PredictionWindowVerificationReport {
    pub scanned_rows: usize,
    pub verified_t1: usize,
    pub verified_t3: usize,
    pub verified_t5: usize,
    pub deferred_windows: usize,
    pub retained_windows: usize,
    pub errors: Vec<String>,
}

#[derive(diesel::QueryableByName)]
struct Row {
    #[diesel(sql_type = diesel::sql_types::Integer)]
    id: i32,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pred_date: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    target_date: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    stock_code: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    pred_direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    actual_change_t1: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    actual_change_t3: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    actual_change_t5: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    hit_t1: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    hit_t3: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    hit_t5: Option<i32>,
}

const COLUMNS: &str = "id,pred_date,target_date,stock_code,pred_direction,actual_change_t1,actual_change_t3,actual_change_t5,hit_t1,hit_t3,hit_t5";

#[cfg(test)]
#[path = "prediction_horizons_tests.rs"]
mod tests;

fn target_date(
    start: NaiveDate,
    days: usize,
    as_of: NaiveDate,
) -> Result<Option<NaiveDate>, String> {
    if !crate::calendar::verified_a_share_trading_day(start)? {
        return Err(format!(
            "prediction window starts on a closed session: {start}"
        ));
    }
    let mut date = start;
    for _ in 0..days {
        if date >= as_of {
            return Ok(None);
        }
        date = crate::calendar::verified_next_a_share_trading_day(date)?;
        if date > as_of {
            return Ok(None);
        }
    }
    Ok(Some(date))
}

pub(super) fn verify_windows(
    db: &DatabaseManager,
    as_of: NaiveDate,
    high_water_id: i32,
    page_size: i64,
) -> Result<PredictionWindowVerificationReport, String> {
    if !(1..=1000).contains(&page_size) || high_water_id < 0 {
        return Err("invalid prediction-window cursor/limit".into());
    }
    let mut report = PredictionWindowVerificationReport::default();
    let mut after_id = 0;
    loop {
        let mut conn = db.get_conn().map_err(|error| error.to_string())?;
        let rows = diesel::sql_query(format!("SELECT {COLUMNS} FROM prediction_tracker WHERE pred_date < ?1 AND id > ?2 AND id <= ?3 AND (actual_change_t1 IS NULL OR hit_t1 IS NULL OR actual_change_t3 IS NULL OR hit_t3 IS NULL OR actual_change_t5 IS NULL OR hit_t5 IS NULL) ORDER BY id LIMIT ?4"))
            .bind::<diesel::sql_types::Text,_>(as_of.to_string())
            .bind::<diesel::sql_types::Integer,_>(after_id)
            .bind::<diesel::sql_types::Integer,_>(high_water_id)
            .bind::<diesel::sql_types::BigInt,_>(page_size)
            .load::<Row>(&mut conn).map_err(|error| error.to_string())?;
        if rows.is_empty() {
            break;
        }
        for planned in rows {
            after_id = planned.id;
            report.scanned_rows += 1;
            match conn.immediate_transaction::<_, Box<dyn std::error::Error>, _>(|conn| {
                settle_row(conn, &planned, as_of)
            }) {
                Ok(row_report) => {
                    report.verified_t1 += row_report.verified_t1;
                    report.verified_t3 += row_report.verified_t3;
                    report.verified_t5 += row_report.verified_t5;
                    report.deferred_windows += row_report.deferred_windows;
                    report.retained_windows += row_report.retained_windows;
                    report.errors.extend(row_report.errors);
                }
                Err(error) => report
                    .errors
                    .push(format!("window id={}: {error}", planned.id)),
            }
        }
    }
    Ok(report)
}

fn settle_row(
    conn: &mut diesel::SqliteConnection,
    planned: &Row,
    as_of: NaiveDate,
) -> Result<PredictionWindowVerificationReport, Box<dyn std::error::Error>> {
    let row = diesel::sql_query(format!(
        "SELECT {COLUMNS} FROM prediction_tracker WHERE id=?1"
    ))
    .bind::<diesel::sql_types::Integer, _>(planned.id)
    .get_result::<Row>(conn)
    .optional()?
    .ok_or("prediction row disappeared")?;
    if (
        row.id,
        &row.pred_date,
        &row.target_date,
        &row.stock_code,
        &row.pred_direction,
    ) != (
        planned.id,
        &planned.pred_date,
        &planned.target_date,
        &planned.stock_code,
        &planned.pred_direction,
    ) {
        return Err("prediction identity changed during window scan".into());
    }
    let mut report = PredictionWindowVerificationReport::default();
    let start = NaiveDate::parse_from_str(&row.pred_date, "%Y-%m-%d")?;
    for (days, change, hit) in [
        (1, row.actual_change_t1, row.hit_t1),
        (3, row.actual_change_t3, row.hit_t3),
        (5, row.actual_change_t5, row.hit_t5),
    ] {
        match (change, hit) {
            (Some(value), Some(hit))
                if value.is_finite() && value >= -100. && matches!(hit, 0 | 1) =>
            {
                report.retained_windows += 1;
                continue;
            }
            (None, None) => {}
            _ => {
                report.errors.push(format!(
                    "window id={} T+{days}: incomplete/invalid stored result retained",
                    row.id
                ));
                continue;
            }
        }
        let Some(target) = target_date(start, days, as_of)? else {
            report.deferred_windows += 1;
            continue;
        };
        let Some(code) = row
            .stock_code
            .as_deref()
            .filter(|code| !code.trim().is_empty())
        else {
            report.deferred_windows += 1;
            continue;
        };
        let Some(previous) = read_exact_close_on(conn, code, &row.pred_date)? else {
            report.deferred_windows += 1;
            continue;
        };
        let Some(close) = read_exact_close_on(conn, code, &target.to_string())? else {
            report.deferred_windows += 1;
            continue;
        };
        if !previous.is_finite() || previous <= 0. || !close.is_finite() || close <= 0. {
            return Err("window close must be finite and positive".into());
        }
        let actual_change = (close - previous) / previous * 100.;
        let hit = recorded_direction_hit(&row.pred_direction, actual_change)?;
        // Only the closed set 1/3/5 forms identifiers; no caller SQL fragment.
        let changed = diesel::sql_query(format!("UPDATE prediction_tracker SET actual_change_t{days}=?1,hit_t{days}=?2 WHERE id=?3 AND actual_change_t{days} IS NULL AND hit_t{days} IS NULL"))
            .bind::<diesel::sql_types::Double,_>(actual_change)
            .bind::<diesel::sql_types::Integer,_>(i32::from(hit))
            .bind::<diesel::sql_types::Integer,_>(row.id).execute(conn)?;
        if changed != 1 {
            return Err("prediction window CAS did not update exactly one row".into());
        }
        match days {
            1 => report.verified_t1 += 1,
            3 => report.verified_t3 += 1,
            5 => report.verified_t5 += 1,
            _ => unreachable!(),
        }
    }
    Ok(report)
}
