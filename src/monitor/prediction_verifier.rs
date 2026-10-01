//! One frozen-date verifier for scheduled, post-session and manual backfill owners.
use crate::database::DatabaseManager;
use chrono::NaiveDate;
use diesel::{OptionalExtension, RunQueryDsl};

#[derive(Debug, Clone, Copy)]
pub struct VerifyOutcome {
    pub actual_change: f64,
    pub hit: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PredictionVerificationReport {
    pub pending: usize,
    pub verified: usize,
    pub hits: usize,
    pub deferred: usize,
    /// Another verifier won the CAS, or the DB declined the update: not our success.
    pub raced: usize,
    pub errors: Vec<String>,
}
impl PredictionVerificationReport {
    pub fn log(&self) {
        if self.errors.is_empty() && self.deferred == 0 && self.raced == 0 {
            log::info!("[Prediction] 到期验证: {:?}", self);
        } else {
            log::warn!("[Prediction] 到期验证仍有未完成项: {:?}", self);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Up,
    Down,
    Neutral,
}
const DIRECTION_MOVE_THRESHOLD_PERCENT: f64 = 0.5;

fn direction(value: &str) -> Result<Direction, String> {
    match value.trim().to_lowercase().as_str() {
        "up" | "bullish" | "long" | "看多" | "上涨" => Ok(Direction::Up),
        "down" | "bearish" | "short" | "看空" | "下跌" => Ok(Direction::Down),
        "neutral" | "flat" | "sideways" | "中性" | "震荡" | "观望" => Ok(Direction::Neutral),
        _ => Err(format!("不支持的预测方向，保留 pending: {value}")),
    }
}

fn read_exact_close(db: &DatabaseManager, code: &str, date: &str) -> Result<Option<f64>, String> {
    #[derive(diesel::QueryableByName)]
    struct Close {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
        close: Option<f64>,
    }
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let row = diesel::sql_query(
        "SELECT daily.close FROM stock_daily AS daily \
         WHERE daily.code = ?1 AND daily.date = ?2 AND daily.is_suspended = 0 \
         AND EXISTS (SELECT 1 FROM qualified_daily_trading_status AS state \
                     WHERE state.code = daily.code AND state.date = daily.date \
                     AND state.status = 'trading') LIMIT 1",
    )
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(date)
    .get_result::<Close>(&mut conn)
    .optional()
    .map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| r.close))
}

fn verify_exact(
    db: &DatabaseManager,
    code: &str,
    pred_date: &str,
    target_date: &str,
    value: &str,
) -> Result<Option<VerifyOutcome>, String> {
    let start = NaiveDate::parse_from_str(pred_date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let target = NaiveDate::parse_from_str(target_date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    if target < start {
        return Err("target_date 早于 pred_date".into());
    }
    let direction = direction(value)?;
    let Some(previous) = read_exact_close(db, code, pred_date)? else {
        return Ok(None);
    };
    let Some(close) = read_exact_close(db, code, target_date)? else {
        return Ok(None);
    };
    if !previous.is_finite() || previous <= 0. || !close.is_finite() || close <= 0. {
        return Err("close 必须是有限正数".into());
    }
    let actual_change = (close - previous) / previous * 100.;
    if !actual_change.is_finite() || actual_change < -100. {
        return Err("累计收益不符合有限数/非负价格合同".into());
    }
    let hit = match direction {
        Direction::Up => actual_change > DIRECTION_MOVE_THRESHOLD_PERCENT,
        Direction::Down => actual_change < -DIRECTION_MOVE_THRESHOLD_PERCENT,
        Direction::Neutral => actual_change.abs() <= DIRECTION_MOVE_THRESHOLD_PERCENT,
    };
    Ok(Some(VerifyOutcome { actual_change, hit }))
}

/// Compatibility read-only helper; owner-level scans use the typed aggregate below.
pub async fn verify_one(
    db: &DatabaseManager,
    code: &str,
    pred_date: &str,
    target_date: &str,
    direction: &str,
) -> Option<VerifyOutcome> {
    match verify_exact(db, code, pred_date, target_date, direction) {
        Ok(outcome) => outcome,
        Err(error) => {
            log::warn!("[Prediction] {code} {pred_date}→{target_date}: {error}");
            None
        }
    }
}

pub fn verify_due_predictions(
    db: &DatabaseManager,
    as_of: NaiveDate,
) -> Result<PredictionVerificationReport, String> {
    verify_due_predictions_with_page_size(db, as_of, 256)
}

pub(super) fn verify_due_predictions_with_page_size(
    db: &DatabaseManager,
    as_of: NaiveDate,
    page_size: i64,
) -> Result<PredictionVerificationReport, String> {
    let high_water_id = db
        .prediction_verification_high_water_id()
        .map_err(|e| e.to_string())?;
    let mut report = PredictionVerificationReport::default();
    let mut after_id = 0;
    loop {
        let rows = match db.get_due_predictions_page(
            &as_of.to_string(),
            after_id,
            high_water_id,
            page_size,
        ) {
            Ok(rows) => rows,
            Err(error) => {
                report
                    .errors
                    .push(format!("due scan after id={after_id}: {error}"));
                break;
            }
        };
        if rows.is_empty() {
            break;
        }
        for row in rows {
            after_id = row.id; // Advance even for missing code/price, never OFFSET on a shrinking set.
            report.pending += 1;
            let Some(code) = row.stock_code.as_deref().filter(|s| !s.trim().is_empty()) else {
                report.deferred += 1;
                continue;
            };
            let outcome = match verify_exact(
                db,
                code,
                &row.pred_date,
                &row.target_date,
                &row.pred_direction,
            ) {
                Ok(Some(outcome)) => outcome,
                Ok(None) => {
                    report.deferred += 1;
                    continue;
                }
                Err(error) => {
                    report.errors.push(format!("id={}: {error}", row.id));
                    continue;
                }
            };
            match db.update_prediction_result_by_id(row.id, outcome.actual_change, outcome.hit) {
                Ok(1) => {
                    report.verified += 1;
                    if outcome.hit {
                        report.hits += 1;
                    }
                }
                Ok(0) => report.raced += 1,
                Ok(rows) => report
                    .errors
                    .push(format!("id={} CAS affected unexpected {rows} rows", row.id)),
                Err(error) => report.errors.push(format!("id={} update: {error}", row.id)),
            }
        }
    }
    Ok(report)
}
