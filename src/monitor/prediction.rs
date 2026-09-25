//! Prediction persistence and frozen-target-date verification.
use crate::database::DatabaseManager;
use chrono::Local;

#[path = "prediction_samples.rs"]
mod samples;
pub use samples::{persist_candidate_samples, save_candidate_samples, CandidateSampleSaveReport};
#[path = "prediction_verifier.rs"]
mod verifier;
pub use verifier::{
    verify_due_predictions, verify_one, PredictionVerificationReport, VerifyOutcome,
};

#[cfg(test)]
use samples::collect_candidate_save_worker;
#[cfg(test)]
use verifier::verify_due_predictions_with_page_size;
#[cfg(test)]
#[path = "prediction_completion_tests.rs"]
mod completion_tests;

/// Legacy prediction producer; existing frozen dates are never rewritten by verification.
pub fn save_prediction(
    theme: Option<&str>,
    stock: Option<&str>,
    direction: &str,
    score: f64,
    detail: Option<&str>,
) {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let tomorrow = (Local::now() + chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();
    let Some(db) = DatabaseManager::try_get() else {
        log::warn!("[Prediction] DB 未初始化");
        return;
    };
    match db.save_prediction_legacy(&today, &tomorrow, theme, stock, direction, score, detail) {
        Err(error) => log::warn!("[Prediction] 保存失败: {error}"),
        Ok(()) => log::info!(
            "[Prediction] ✓ {} {} {}分",
            direction,
            stock.unwrap_or(theme.unwrap_or("?")),
            score
        ),
    }
}

/// Scheduled owner: logs the aggregate and returns it; failure does not stop scheduling.
pub async fn verify_predictions() -> Result<PredictionVerificationReport, String> {
    let as_of = Local::now().date_naive();
    let result = tokio::task::spawn_blocking(move || {
        let db = DatabaseManager::try_get().ok_or_else(|| "Prediction DB 未初始化".to_string())?;
        verify_due_predictions(db, as_of)
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

pub fn recent_hit_rate(days: i32) -> Result<f64, String> {
    let db = DatabaseManager::try_get().ok_or_else(|| "Prediction DB 未初始化".to_string())?;
    db.get_prediction_hit_rate(days).map_err(|e| e.to_string())
}

pub fn hit_rate_summary(days: i32) -> String {
    match recent_hit_rate(days) {
        Ok(rate) => format!("近{}天预测命中率: {:.0}%", days, rate * 100.0),
        Err(error) => format!("近{}天预测命中率: 不可用（{}）", days, error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_hit_rate_format() {
        let s = hit_rate_summary(7);
        assert!(s.contains("命中率"));
        assert!(s.contains('%') || s.contains("不可用"));
    }
    #[test]
    fn test_save_prediction_no_panic() {
        save_prediction(None, Some("TEST_CODE_000001"), "看多", 75., None);
    }
}
