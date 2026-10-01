//! Candidate persistence has its own outcome; it must not trigger replay of a sent card.
use crate::database::p05_prediction_freeze::FrozenCandidateBoardV2;
use crate::database::DatabaseManager;
use chrono::{NaiveDate, NaiveTime};

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
            && self.unknown == 0
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

/// A fixed producer slot and the exact card from one candidate batch.
/// This is not source qualification or counted admission authority.
#[derive(Debug, Clone)]
pub struct CandidateBoardPreparationRequest {
    business_date: String,
    occurrence_identity: String,
    target_date: Option<String>,
    rendered_bytes: Vec<u8>,
    samples: Vec<(String, f64)>,
}

#[derive(Debug, thiserror::Error)]
#[error("{reason}")]
pub struct CandidateBoardPreparationError {
    reason: &'static str,
    save_report: Option<CandidateSampleSaveReport>,
}

impl CandidateBoardPreparationError {
    fn blocked(reason: &'static str, save_report: Option<CandidateSampleSaveReport>) -> Self {
        Self {
            reason,
            save_report,
        }
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }

    pub fn save_report(&self) -> Option<&CandidateSampleSaveReport> {
        self.save_report.as_ref()
    }
}

impl CandidateBoardPreparationRequest {
    pub fn new(
        business_date: &str,
        hhmm: &str,
        rendered_bytes: Vec<u8>,
        samples: Vec<(String, f64)>,
    ) -> Result<Self, CandidateBoardPreparationError> {
        let invalid = || CandidateBoardPreparationError::blocked("p05_request_invalid", None);
        let date = NaiveDate::parse_from_str(business_date, "%Y-%m-%d").map_err(|_| invalid())?;
        let time = NaiveTime::parse_from_str(hhmm, "%H:%M").map_err(|_| invalid())?;
        if date.format("%Y-%m-%d").to_string() != business_date
            || time.format("%H:%M").to_string() != hhmm
            || rendered_bytes.is_empty()
            || std::str::from_utf8(&rendered_bytes).is_err()
        {
            return Err(invalid());
        }
        if !crate::calendar::verified_a_share_trading_day(date).map_err(|_| {
            CandidateBoardPreparationError::blocked("p05_calendar_unavailable", None)
        })? {
            return Err(CandidateBoardPreparationError::blocked(
                "p05_not_trading_day",
                None,
            ));
        }
        let mut seen = std::collections::HashSet::new();
        if samples.iter().any(|(code, score)| {
            code.trim().is_empty() || !seen.insert(code.as_str()) || !score.is_finite()
        }) {
            return Err(invalid());
        }
        // A card without prediction members does not claim a T+5 sample.
        let target_date = if samples.is_empty() {
            None
        } else {
            let mut target = date;
            for _ in 0..5 {
                target =
                    crate::calendar::verified_next_a_share_trading_day(target).map_err(|_| {
                        CandidateBoardPreparationError::blocked("p05_calendar_unavailable", None)
                    })?;
            }
            Some(target.format("%Y-%m-%d").to_string())
        };
        Ok(Self {
            business_date: business_date.to_owned(),
            occurrence_identity: format!("candidate-board:{business_date}:{hhmm}"),
            target_date,
            rendered_bytes,
            samples,
        })
    }
}

#[derive(Debug)]
pub enum CandidateBoardPreparation {
    Frozen {
        record: FrozenCandidateBoardV2,
        /// Includes a concurrent winner; no membership or bytes are replaced.
        reused: bool,
        save_report: Option<CandidateSampleSaveReport>,
    },
    /// A nonempty card with no sampled Strong rows, explicitly not row-linked.
    UnlinkedNoStrong,
}

/// The actual producer workflow, also used with owned isolated DBs in tests.
/// The frozen return value still requires the original counted admission gate.
pub fn prepare_candidate_board_on(
    db: &DatabaseManager,
    request: &CandidateBoardPreparationRequest,
) -> Result<CandidateBoardPreparation, CandidateBoardPreparationError> {
    prepare_candidate_board_on_with_save(db, request, save_candidate_samples)
}

fn prepare_candidate_board_on_with_save<F>(
    db: &DatabaseManager,
    request: &CandidateBoardPreparationRequest,
    save: F,
) -> Result<CandidateBoardPreparation, CandidateBoardPreparationError>
where
    F: FnOnce(&DatabaseManager, &str, &str, &[(String, f64)]) -> CandidateSampleSaveReport,
{
    let codes: Vec<String> = request
        .samples
        .iter()
        .map(|(code, _)| code.clone())
        .collect();
    let existing = db
        .read_candidate_board_v2_freeze(&request.occurrence_identity)
        .map_err(|_| CandidateBoardPreparationError::blocked("p05_freeze_read_failed", None))?;
    if let Some(record) = existing {
        let frozen_codes: Vec<&str> = record.ordered_rows().iter().map(|row| row.code()).collect();
        let expected_codes: Vec<&str> = codes.iter().map(String::as_str).collect();
        if record.business_date() != request.business_date.as_str()
            || Some(record.target_date()) != request.target_date.as_deref()
            || record.rendered_bytes() != request.rendered_bytes.as_slice()
            || frozen_codes != expected_codes
        {
            return Err(CandidateBoardPreparationError::blocked(
                "p05_frozen_card_drift",
                None,
            ));
        }
        return Ok(CandidateBoardPreparation::Frozen {
            record,
            reused: true,
            save_report: None,
        });
    }
    let Some(target_date) = request.target_date.as_deref() else {
        return Ok(CandidateBoardPreparation::UnlinkedNoStrong);
    };
    let report = save(db, &request.business_date, target_date, &request.samples);
    if report.attempted != codes.len()
        || report.saved != report.attempted
        || report.saved_rows.len() != report.saved
        || report.unknown != 0
        || !report.failures.is_empty()
        || report.worker_error.is_some()
    {
        return Err(CandidateBoardPreparationError::blocked(
            "p05_sample_save_incomplete",
            Some(report),
        ));
    }
    let outcome = db
        .freeze_candidate_board_v2(
            &request.business_date,
            &request.occurrence_identity,
            target_date,
            &request.rendered_bytes,
            &codes,
            &report,
        )
        .map_err(|_| {
            CandidateBoardPreparationError::blocked("p05_freeze_failed", Some(report.clone()))
        })?;
    Ok(CandidateBoardPreparation::Frozen {
        reused: !outcome.inserted(),
        record: outcome.into_record(),
        save_report: Some(report),
    })
}

pub async fn prepare_candidate_board(
    request: CandidateBoardPreparationRequest,
) -> Result<CandidateBoardPreparation, CandidateBoardPreparationError> {
    let attempted = request.samples.len();
    let worker = tokio::task::spawn_blocking(move || {
        let db = DatabaseManager::try_get().ok_or_else(|| {
            CandidateBoardPreparationError::blocked("p05_prediction_db_unavailable", None)
        })?;
        prepare_candidate_board_on(db, &request)
    });
    collect_candidate_board_prepare_worker(worker, attempted).await
}

async fn collect_candidate_board_prepare_worker(
    worker: tokio::task::JoinHandle<
        Result<CandidateBoardPreparation, CandidateBoardPreparationError>,
    >,
    attempted: usize,
) -> Result<CandidateBoardPreparation, CandidateBoardPreparationError> {
    worker.await.map_err(|error| {
        CandidateBoardPreparationError::blocked(
            "p05_prepare_worker_unknown",
            Some(CandidateSampleSaveReport {
                attempted,
                unknown: attempted,
                worker_error: Some(error.to_string()),
                ..Default::default()
            }),
        )
    })?
}

#[cfg(test)]
#[path = "prediction_candidate_board_tests.rs"]
mod candidate_board_tests;
