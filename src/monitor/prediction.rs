//! Prediction persistence and frozen-target-date verification.
use crate::database::{DatabaseManager, VerifiedPredictionSampleHitRate};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, Utc};

#[path = "prediction_samples.rs"]
mod samples;
pub use samples::{
    persist_candidate_samples, prepare_candidate_board, prepare_candidate_board_on,
    save_candidate_samples, CandidateBoardPreparation, CandidateBoardPreparationError,
    CandidateBoardPreparationRequest, CandidateSampleSaveReport, SavedCandidateSample,
};
#[path = "prediction_verifier.rs"]
mod verifier;
pub(crate) use verifier::read_exact_close_on;
pub use verifier::{
    verify_due_predictions, verify_one, PredictionVerificationReport, VerifyOutcome,
};

#[path = "prediction_horizons.rs"]
mod horizons;
pub use horizons::PredictionWindowVerificationReport;

#[path = "prediction_outcome_tracker.rs"]
mod outcome_tracker;
pub use outcome_tracker::{
    OutcomeDailyWeeklyObservation, OutcomeItemDeliveryStatus, OutcomeItemFeedback,
    OutcomeItemFeedbackRow, OutcomePeriodObservation, OutcomeSampleCounts, OutcomeTracker,
    PhysicalLinkedOutcomeCounts, PhysicalLinkedOutcomeObservation,
};

#[cfg(test)]
use samples::collect_candidate_save_worker;
#[cfg(test)]
use verifier::verify_due_predictions_with_page_size;
#[cfg(test)]
#[path = "prediction_completion_tests.rs"]
mod completion_tests;
#[cfg(test)]
#[path = "prediction_scheduled_tests.rs"]
mod scheduled_tests;

/// Legacy prediction producer; existing frozen dates are never rewritten by verification.
pub fn save_prediction(
    theme: Option<&str>,
    stock: Option<&str>,
    direction: &str,
    score: f64,
    detail: Option<&str>,
) {
    let Some(db) = DatabaseManager::try_get() else {
        log::warn!("[Prediction] DB 未初始化");
        return;
    };
    let today = shanghai_now().date_naive();
    match save_prediction_on(db, today, theme, stock, direction, score, detail) {
        Err(error) => log::warn!("[Prediction] 保存失败: {error}"),
        Ok(target) => log::info!(
            "[Prediction] ✓ {} {} {}分 T+1={}",
            direction,
            stock.unwrap_or(theme.unwrap_or("?")),
            score,
            target
        ),
    }
}

fn save_prediction_on(
    db: &DatabaseManager,
    today: NaiveDate,
    theme: Option<&str>,
    stock: Option<&str>,
    direction: &str,
    score: f64,
    detail: Option<&str>,
) -> Result<NaiveDate, String> {
    if !crate::calendar::verified_a_share_trading_day(today)? {
        return Err(format!("预测生成日期不是已核验 A 股交易日: {today}"));
    }
    let target = crate::calendar::verified_next_a_share_trading_day(today)?;
    db.save_prediction_legacy(
        &today.format("%Y-%m-%d").to_string(),
        &target.format("%Y-%m-%d").to_string(),
        theme,
        stock,
        direction,
        score,
        detail,
    )
    .map_err(|error| error.to_string())?;
    Ok(target)
}

pub fn shanghai_now() -> DateTime<FixedOffset> {
    Utc::now().with_timezone(&FixedOffset::east_opt(8 * 60 * 60).expect("Shanghai +08:00 offset"))
}

/// Scheduled owner: logs the aggregate and returns it; failure does not stop scheduling.
pub async fn verify_predictions() -> Result<PredictionVerificationReport, String> {
    let now = shanghai_now();
    let result = tokio::task::spawn_blocking(move || {
        let db = DatabaseManager::try_get().ok_or_else(|| "Prediction DB 未初始化".to_string())?;
        verify_predictions_on_at(db, now)
    })
    .await
    .map_err(|error| format!("Prediction verifier worker failed: {error}"))
    .and_then(|report| report);
    match &result {
        Ok(report) => report.log(),
        Err(error) => log::error!("[Prediction] 本轮验证失败，调度继续: {error}"),
    }
    result
}

fn verify_predictions_on_at(
    db: &DatabaseManager,
    now: DateTime<FixedOffset>,
) -> Result<PredictionVerificationReport, String> {
    let as_of = completed_session_as_of_at(now)?;
    verify_due_predictions(db, as_of)
}

/// Resolve the latest completed session using the immutable checked-in calendar.
pub fn completed_session_as_of_at(now: DateTime<FixedOffset>) -> Result<NaiveDate, String> {
    if now.offset().local_minus_utc() != 8 * 60 * 60 {
        return Err("命中率观察时间必须为上海 +08:00".to_string());
    }
    let today = now.date_naive();
    let is_trading = crate::calendar::verified_a_share_trading_day(today)?;
    let close = NaiveTime::from_hms_opt(15, 0, 0).expect("valid session close");
    if is_trading && now.time() >= close {
        Ok(today)
    } else {
        crate::calendar::verified_prev_a_share_trading_day(today)
    }
}

pub fn verified_sample_hit_rate(
    as_of: NaiveDate,
    trading_days: usize,
) -> Result<VerifiedPredictionSampleHitRate, String> {
    let db = DatabaseManager::try_get().ok_or_else(|| "Prediction DB 未初始化".to_string())?;
    db.get_verified_prediction_sample_hit_rate(as_of, trading_days)
        .map_err(|error| error.to_string())
}

/// The denominator is verified prediction rows, never confirmed delivered messages.
pub fn hit_rate_summary_at(
    now: DateTime<FixedOffset>,
    trading_days: usize,
) -> Result<String, String> {
    let as_of = completed_session_as_of_at(now)?;
    let stats = verified_sample_hit_rate(as_of, trading_days)?;
    Ok(format!(
        "近{}个已完成交易日({}..={})已验证信号样本命中率: {:.0}% ({}/{}, 非送达消息)",
        stats.trading_days,
        stats.window_start,
        stats.as_of,
        stats.rate * 100.0,
        stats.hits,
        stats.samples
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_save_prediction_no_panic() {
        save_prediction(None, Some("TEST_CODE_000001"), "看多", 75., None);
    }
}
