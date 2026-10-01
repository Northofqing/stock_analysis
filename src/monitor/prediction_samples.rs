//! Candidate persistence has its own outcome; it must not trigger replay of a sent card.
use crate::database::DatabaseManager;

#[derive(Debug, Clone)]
pub struct CandidateSampleFailure {
    pub code: String,
    pub error: String,
}

/// Identity of a prediction row actually committed by this save attempt.
/// This is not evidence that the candidate card reached a physical sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCandidateSample {
    pub code: String,
    pub prediction_row_id: i64,
}

#[derive(Debug, Clone, Default)]
pub struct CandidateSampleSaveReport {
    pub attempted: usize,
    pub saved: usize,
    pub saved_rows: Vec<SavedCandidateSample>,
    /// A failed worker may already have committed rows. Never claim they all failed.
    pub unknown: usize,
    pub failures: Vec<CandidateSampleFailure>,
    pub worker_error: Option<String>,
}
impl CandidateSampleSaveReport {
    pub fn is_complete(&self) -> bool {
        self.saved == self.attempted
            && self.saved_rows.len() == self.saved
            && self.failures.is_empty()
            && self.worker_error.is_none()
    }
    pub fn log(&self) {
        if self.is_complete() {
            log::info!("[Prediction] 候选样本保存完成: {:?}", self);
        } else {
            log::error!("[Prediction] 候选样本保存不完整（不重发消息）: {:?}", self);
        }
    }
}

pub fn save_candidate_samples(
    db: &DatabaseManager,
    pred_date: &str,
    target_date: &str,
    samples: &[(String, f64)],
) -> CandidateSampleSaveReport {
    let mut report = CandidateSampleSaveReport {
        attempted: samples.len(),
        ..Default::default()
    };
    for (code, score) in samples {
        match db.save_prediction_with_id(
            pred_date,
            target_date,
            None,
            Some(code),
            "up",
            *score,
            Some("candidate-strong"),
            None,
            None,
        ) {
            Ok(prediction_row_id) => {
                report.saved += 1;
                report.saved_rows.push(SavedCandidateSample {
                    code: code.clone(),
                    prediction_row_id,
                });
            }
            Err(error) => report.failures.push(CandidateSampleFailure {
                code: code.clone(),
                error: error.to_string(),
            }),
        }
    }
    report
}

pub(super) async fn collect_candidate_save_worker(
    worker: tokio::task::JoinHandle<CandidateSampleSaveReport>,
    attempted: usize,
) -> CandidateSampleSaveReport {
    match worker.await {
        Ok(report) => report,
        Err(error) => CandidateSampleSaveReport {
            attempted,
            unknown: attempted,
            worker_error: Some(error.to_string()),
            ..Default::default()
        },
    }
}

pub async fn persist_candidate_samples(
    pred_date: String,
    target_date: String,
    samples: Vec<(String, f64)>,
) -> CandidateSampleSaveReport {
    let attempted = samples.len();
    let worker = tokio::task::spawn_blocking(move || match DatabaseManager::try_get() {
        Some(db) => save_candidate_samples(db, &pred_date, &target_date, &samples),
        None => CandidateSampleSaveReport {
            attempted,
            worker_error: Some("DB 未初始化，未开始写库".into()),
            ..Default::default()
        },
    });
    collect_candidate_save_worker(worker, attempted).await
}
