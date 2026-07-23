//! BR-157 immutable T0-close and D+1 raw outcomes for admitted shadow candidates.

use crate::database::selection::{
    DueOutcome, OutcomePhase, SelectionOutcomeInput, SelectionRepository,
};
use crate::selection::audit::{
    SelectionAuditContext, SelectionAuditEnvironment, SelectionAuditPhase, SelectionAuditRecord,
    SelectionAuditWriter,
};
use crate::selection::features::{RawSelectionFeatures, T0MarketEvidence};
use crate::selection::magic_tdx::{fetch_settled_daily_bar, SettledDailyEvidence};
use crate::selection::quality::{validate_daily, SelectionBar};
use chrono::{DateTime, FixedOffset, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const OUTCOME_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct T0CloseSnapshot {
    pub schema_version: u16,
    pub candidate_id: String,
    pub stock_code: String,
    pub market_date: NaiveDate,
    pub evaluation_price: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub close_return: f64,
    pub observed_volume: f64,
    pub close_volume_vs_observed: f64,
    pub prior_5d_average_volume: f64,
    pub prior_20d_average_volume: f64,
    pub close_volume_vs_5d: f64,
    pub close_volume_vs_20d: f64,
    pub ma5: f64,
    pub ma10: f64,
    pub ma20: f64,
    pub close_vs_ma5: f64,
    pub close_vs_ma10: f64,
    pub close_vs_ma20: f64,
    pub trend_alignment_maintained: bool,
    pub source_batch_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct D1SettledOutcome {
    pub schema_version: u16,
    pub candidate_id: String,
    pub stock_code: String,
    pub t0_market_date: NaiveDate,
    pub d1_market_date: NaiveDate,
    pub t0_close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub open_return: f64,
    pub close_return: f64,
    pub mfe: f64,
    pub mae: f64,
    pub volume_vs_t0: f64,
    pub volume_vs_prior_5d: f64,
    pub volume_vs_prior_20d: f64,
    pub close_vs_ma5: f64,
    pub close_vs_ma10: f64,
    pub close_vs_ma20: f64,
    pub source_batch_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComputedOutcome {
    T0Close(T0CloseSnapshot),
    D1Settled(D1SettledOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeWait {
    pub candidate_id: String,
    pub phase: OutcomePhase,
    pub reason_code: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeRejection {
    pub candidate_id: String,
    pub phase: OutcomePhase,
    pub reason_code: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OutcomeAttempt {
    Ready(ComputedOutcome),
    ExpectedWait(OutcomeWait),
    Rejected(OutcomeRejection),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DueOutcomeRequest {
    pub candidate_id: String,
    pub stock_code: String,
    pub phase: OutcomePhase,
    pub due_market_date: NaiveDate,
    pub t0_feature_baseline: Option<T0FeatureBaseline>,
    pub t0_baseline: Option<T0CloseSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct T0FeatureBaseline {
    pub market: T0MarketEvidence,
    pub features: RawSelectionFeatures,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutcomeSettlementSummary {
    pub attempted: usize,
    pub settled: usize,
    pub expected_wait: usize,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct SelectionOutcomeError {
    reason_code: &'static str,
    message: String,
    retryable: bool,
}

impl SelectionOutcomeError {
    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }
}

pub fn compute_t0_outcome(
    request: &DueOutcomeRequest,
    bar: &SelectionBar,
    source_batch_id: &str,
) -> Result<T0CloseSnapshot, SelectionOutcomeError> {
    validate_due_bar(request, bar)?;
    if request.phase != OutcomePhase::T0Close {
        return Err(outcome_error(
            "outcome_phase_mismatch",
            "T0 computation requires the T0Close phase",
            false,
        ));
    }
    require_non_empty("source_batch_id", source_batch_id)?;
    let baseline = request.t0_feature_baseline.as_ref().ok_or_else(|| {
        outcome_error(
            "t0_feature_baseline_missing",
            "T0 close outcome has no immutable feature/market baseline",
            false,
        )
    })?;
    validate_feature_baseline(baseline)?;
    let ma5 = required_feature("ma5", baseline.features.ma5)?;
    let ma10 = required_feature("ma10", baseline.features.ma10)?;
    let ma20 = required_feature("ma20", baseline.features.ma20)?;
    let evaluation_price = baseline.market.evaluation_price;
    Ok(T0CloseSnapshot {
        schema_version: OUTCOME_SCHEMA_VERSION,
        candidate_id: request.candidate_id.clone(),
        stock_code: request.stock_code.clone(),
        market_date: bar.market_date,
        evaluation_price,
        open: bar.open,
        high: bar.high,
        low: bar.low,
        close: bar.close,
        volume: bar.volume,
        amount: bar.amount,
        close_return: bar.close / evaluation_price - 1.0,
        observed_volume: baseline.market.observed_volume,
        close_volume_vs_observed: bar.volume / baseline.market.observed_volume,
        prior_5d_average_volume: baseline.market.prior_5d_average_volume,
        prior_20d_average_volume: baseline.market.prior_20d_average_volume,
        close_volume_vs_5d: bar.volume / baseline.market.prior_5d_average_volume,
        close_volume_vs_20d: bar.volume / baseline.market.prior_20d_average_volume,
        ma5,
        ma10,
        ma20,
        close_vs_ma5: bar.close / ma5 - 1.0,
        close_vs_ma10: bar.close / ma10 - 1.0,
        close_vs_ma20: bar.close / ma20 - 1.0,
        trend_alignment_maintained: bar.close >= ma5 && ma5 >= ma10 && ma10 >= ma20,
        source_batch_id: source_batch_id.to_owned(),
    })
}

pub fn compute_d1_outcome(
    baseline: &T0CloseSnapshot,
    bar: &SelectionBar,
    source_batch_id: &str,
) -> Result<D1SettledOutcome, SelectionOutcomeError> {
    validate_snapshot(baseline)?;
    validate_daily(std::slice::from_ref(bar)).map_err(|error| {
        outcome_error(
            error.code(),
            format!("D+1 settled bar rejected: {error}"),
            false,
        )
    })?;
    let expected_d1 = crate::calendar::next_trading_day(baseline.market_date);
    if bar.code != baseline.stock_code || bar.market_date != expected_d1 {
        return Err(outcome_error(
            "outcome_evidence_identity_mismatch",
            format!(
                "D+1 evidence identity/date mismatch: expected {} {}, got {} {}",
                baseline.stock_code, expected_d1, bar.code, bar.market_date
            ),
            false,
        ));
    }
    require_non_empty("source_batch_id", source_batch_id)?;
    let base = baseline.close;
    Ok(D1SettledOutcome {
        schema_version: OUTCOME_SCHEMA_VERSION,
        candidate_id: baseline.candidate_id.clone(),
        stock_code: baseline.stock_code.clone(),
        t0_market_date: baseline.market_date,
        d1_market_date: bar.market_date,
        t0_close: base,
        open: bar.open,
        high: bar.high,
        low: bar.low,
        close: bar.close,
        volume: bar.volume,
        amount: bar.amount,
        open_return: bar.open / base - 1.0,
        close_return: bar.close / base - 1.0,
        mfe: bar.high / base - 1.0,
        mae: bar.low / base - 1.0,
        volume_vs_t0: bar.volume / baseline.volume,
        volume_vs_prior_5d: bar.volume / baseline.prior_5d_average_volume,
        volume_vs_prior_20d: bar.volume / baseline.prior_20d_average_volume,
        close_vs_ma5: bar.close / baseline.ma5 - 1.0,
        close_vs_ma10: bar.close / baseline.ma10 - 1.0,
        close_vs_ma20: bar.close / baseline.ma20 - 1.0,
        source_batch_id: source_batch_id.to_owned(),
    })
}

pub fn compute_due_outcome(
    request: &DueOutcomeRequest,
    evidence: Option<&SelectionBar>,
    latest_settled_market_date: NaiveDate,
    source_batch_id: &str,
) -> OutcomeAttempt {
    if request.due_market_date > latest_settled_market_date {
        return OutcomeAttempt::ExpectedWait(OutcomeWait {
            candidate_id: request.candidate_id.clone(),
            phase: request.phase,
            reason_code: "market_session_unsettled",
        });
    }
    let Some(bar) = evidence else {
        return OutcomeAttempt::ExpectedWait(OutcomeWait {
            candidate_id: request.candidate_id.clone(),
            phase: request.phase,
            reason_code: "settled_bar_missing",
        });
    };
    let computed = match request.phase {
        OutcomePhase::T0Close => {
            compute_t0_outcome(request, bar, source_batch_id).map(ComputedOutcome::T0Close)
        }
        OutcomePhase::D1Settled => request
            .t0_baseline
            .as_ref()
            .ok_or_else(|| {
                outcome_error(
                    "t0_baseline_missing",
                    "D+1 outcome has no immutable T0 close baseline",
                    false,
                )
            })
            .and_then(|baseline| {
                if baseline.candidate_id != request.candidate_id {
                    return Err(outcome_error(
                        "t0_baseline_identity_mismatch",
                        "D+1 candidate identity differs from its T0 baseline",
                        false,
                    ));
                }
                compute_d1_outcome(baseline, bar, source_batch_id).map(ComputedOutcome::D1Settled)
            }),
    };
    match computed {
        Ok(outcome) => OutcomeAttempt::Ready(outcome),
        Err(error) => OutcomeAttempt::Rejected(OutcomeRejection {
            candidate_id: request.candidate_id.clone(),
            phase: request.phase,
            reason_code: error.reason_code(),
        }),
    }
}

pub async fn settle_due_outcomes(
    now: DateTime<Local>,
) -> Result<OutcomeSettlementSummary, SelectionOutcomeError> {
    let latest_settled_market_date =
        crate::calendar::latest_completed_trading_day_at(now.naive_local());
    let due = load_due_outcomes(latest_settled_market_date)?;
    let audit =
        SelectionAuditWriter::for_environment("data/audit", SelectionAuditEnvironment::Production);
    let mut summary = OutcomeSettlementSummary {
        attempted: due.len(),
        ..OutcomeSettlementSummary::default()
    };
    let mut first_failure: Option<SelectionOutcomeError> = None;

    for due_outcome in due {
        let request = due_request(due_outcome)?;
        let evidence = match fetch_settled_daily_bar(
            request.stock_code.clone(),
            request.due_market_date,
        )
        .await
        {
            Ok(evidence) => evidence,
            Err(error) => {
                append_attempt_audit(
                    &audit,
                    &request,
                    None,
                    error.code(),
                    error.retryable(),
                    now.fixed_offset(),
                )?;
                first_failure.get_or_insert_with(|| {
                    outcome_error(
                        error.code(),
                        format!(
                            "Magic TDX outcome evidence unavailable for {}: {error}",
                            request.stock_code
                        ),
                        error.retryable(),
                    )
                });
                continue;
            }
        };
        let batch_id = evidence
            .as_ref()
            .map(|item| item.batch_id.as_str())
            .unwrap_or("magic_tdx_settled_bar_missing");
        match compute_due_outcome(
            &request,
            evidence.as_ref().map(|item| &item.bar),
            latest_settled_market_date,
            batch_id,
        ) {
            OutcomeAttempt::Ready(computed) => {
                persist_computed_outcome(&audit, computed, now.fixed_offset())?;
                summary.settled += 1;
            }
            OutcomeAttempt::ExpectedWait(wait) => {
                append_attempt_audit(
                    &audit,
                    &request,
                    evidence.as_ref(),
                    wait.reason_code,
                    true,
                    now.fixed_offset(),
                )?;
                summary.expected_wait += 1;
            }
            OutcomeAttempt::Rejected(rejection) => {
                append_attempt_audit(
                    &audit,
                    &request,
                    evidence.as_ref(),
                    rejection.reason_code,
                    false,
                    now.fixed_offset(),
                )?;
                first_failure.get_or_insert_with(|| {
                    outcome_error(
                        rejection.reason_code,
                        format!(
                            "selection outcome rejected for candidate {}",
                            rejection.candidate_id
                        ),
                        false,
                    )
                });
            }
        }
    }
    first_failure.map_or(Ok(summary), Err)
}

fn due_request(due: DueOutcome) -> Result<DueOutcomeRequest, SelectionOutcomeError> {
    #[derive(Deserialize)]
    struct FeatureEnvelope {
        features: RawSelectionFeatures,
        t0_market_evidence: T0MarketEvidence,
    }

    let feature =
        serde_json::from_str::<FeatureEnvelope>(&due.feature_payload_json).map_err(|error| {
            outcome_error(
                "t0_feature_baseline_invalid",
                format!("persisted T0 feature baseline is invalid: {error}"),
                false,
            )
        })?;
    let t0_baseline = due
        .t0_outcome_payload_json
        .map(|payload| {
            serde_json::from_str::<T0CloseSnapshot>(&payload).map_err(|error| {
                outcome_error(
                    "t0_baseline_invalid",
                    format!("persisted T0 baseline is invalid: {error}"),
                    false,
                )
            })
        })
        .transpose()?;
    Ok(DueOutcomeRequest {
        candidate_id: due.candidate_id,
        stock_code: due.stock_code,
        phase: due.phase,
        due_market_date: due.due_market_date,
        t0_feature_baseline: Some(T0FeatureBaseline {
            market: feature.t0_market_evidence,
            features: feature.features,
        }),
        t0_baseline,
    })
}

fn load_due_outcomes(as_of: NaiveDate) -> Result<Vec<DueOutcome>, SelectionOutcomeError> {
    with_repository(|repository| repository.due_outcomes(as_of))
}

fn persist_computed_outcome(
    audit: &SelectionAuditWriter,
    computed: ComputedOutcome,
    observed_at: DateTime<FixedOffset>,
) -> Result<(), SelectionOutcomeError> {
    let (candidate_id, stock_code, phase, market_date, source_batch_id, payload_json) =
        match computed {
            ComputedOutcome::T0Close(snapshot) => {
                let payload_json = serde_json::to_string(&snapshot).map_err(|error| {
                    outcome_error(
                        "outcome_serialize_failed",
                        format!("serialize T0 outcome: {error}"),
                        false,
                    )
                })?;
                (
                    snapshot.candidate_id,
                    snapshot.stock_code,
                    OutcomePhase::T0Close,
                    snapshot.market_date,
                    snapshot.source_batch_id,
                    payload_json,
                )
            }
            ComputedOutcome::D1Settled(outcome) => {
                let payload_json = serde_json::to_string(&outcome).map_err(|error| {
                    outcome_error(
                        "outcome_serialize_failed",
                        format!("serialize D+1 outcome: {error}"),
                        false,
                    )
                })?;
                (
                    outcome.candidate_id,
                    outcome.stock_code,
                    OutcomePhase::D1Settled,
                    outcome.d1_market_date,
                    outcome.source_batch_id,
                    payload_json,
                )
            }
        };
    let content_hash = stable_hash("stock_analysis.selection_outcome.v1", &payload_json);
    let outcome_id = format!(
        "selection_outcome_{}_{}",
        phase.as_storage_str(),
        stable_hash(
            "stock_analysis.selection_outcome_identity.v1",
            &(candidate_id.as_str(), market_date)
        )
    );
    let audit_phase = audit_phase(phase);
    audit
        .append(
            SelectionAuditRecord::new(audit_phase, &outcome_id, &content_hash, observed_at)
                .with_context(SelectionAuditContext {
                    security_identity_hash: Some(stable_hash(
                        "stock_analysis.selection_security.v1",
                        &stock_code,
                    )),
                    provider: Some("magic_tdx".to_owned()),
                    magic_tdx_batch_id: Some(source_batch_id),
                    rule_ids: vec!["BR-157".to_owned()],
                    retryable: Some(false),
                    ..SelectionAuditContext::default()
                }),
        )
        .map_err(|error| {
            outcome_error(
                error.code(),
                format!("append selection outcome audit: {error}"),
                true,
            )
        })?;
    with_repository(|repository| {
        repository.append_outcome(&SelectionOutcomeInput {
            outcome_id,
            candidate_id,
            phase,
            market_date,
            content_hash,
            payload_json,
            observed_at,
        })
    })
    .map(|_| ())
}

fn append_attempt_audit(
    audit: &SelectionAuditWriter,
    request: &DueOutcomeRequest,
    evidence: Option<&SettledDailyEvidence>,
    reason_code: &'static str,
    retryable: bool,
    observed_at: DateTime<FixedOffset>,
) -> Result<(), SelectionOutcomeError> {
    let content_hash = stable_hash(
        "stock_analysis.selection_outcome_attempt.v1",
        &(
            request.candidate_id.as_str(),
            request.phase.as_storage_str(),
            request.due_market_date,
            reason_code,
        ),
    );
    audit
        .append(
            SelectionAuditRecord::new(
                audit_phase(request.phase),
                format!(
                    "selection_outcome_attempt_{}_{}",
                    request.phase.as_storage_str(),
                    request.candidate_id
                ),
                content_hash,
                observed_at,
            )
            .with_context(SelectionAuditContext {
                security_identity_hash: Some(stable_hash(
                    "stock_analysis.selection_security.v1",
                    &request.stock_code,
                )),
                provider: Some("magic_tdx".to_owned()),
                magic_tdx_batch_id: evidence.map(|item| item.batch_id.clone()),
                reason_codes: vec![reason_code.to_owned()],
                rule_ids: vec!["BR-157".to_owned()],
                retryable: Some(retryable),
                ..SelectionAuditContext::default()
            }),
        )
        .map(|_| ())
        .map_err(|error| {
            outcome_error(
                error.code(),
                format!("append selection outcome attempt audit: {error}"),
                true,
            )
        })
}

fn with_repository<T>(
    operation: impl FnOnce(
        &mut SelectionRepository<'_>,
    ) -> Result<T, crate::database::selection::SelectionStoreError>,
) -> Result<T, SelectionOutcomeError> {
    let database = crate::database::DatabaseManager::try_get().ok_or_else(|| {
        outcome_error(
            "selection_database_unavailable",
            "database manager is not initialized",
            true,
        )
    })?;
    let mut connection = database.get_conn().map_err(|error| {
        outcome_error(
            "selection_database_unavailable",
            format!("acquire selection database connection: {error}"),
            true,
        )
    })?;
    let mut repository = SelectionRepository::new(&mut connection);
    operation(&mut repository).map_err(|error| {
        outcome_error(
            "selection_database_failure",
            format!("selection outcome database operation failed: {error}"),
            true,
        )
    })
}

fn validate_due_bar(
    request: &DueOutcomeRequest,
    bar: &SelectionBar,
) -> Result<(), SelectionOutcomeError> {
    validate_daily(std::slice::from_ref(bar)).map_err(|error| {
        outcome_error(
            error.code(),
            format!("settled outcome bar rejected: {error}"),
            false,
        )
    })?;
    if bar.code != request.stock_code || bar.market_date != request.due_market_date {
        return Err(outcome_error(
            "outcome_evidence_identity_mismatch",
            format!(
                "outcome evidence identity/date mismatch: expected {} {}, got {} {}",
                request.stock_code, request.due_market_date, bar.code, bar.market_date
            ),
            false,
        ));
    }
    Ok(())
}

fn validate_snapshot(snapshot: &T0CloseSnapshot) -> Result<(), SelectionOutcomeError> {
    if snapshot.schema_version != OUTCOME_SCHEMA_VERSION {
        return Err(outcome_error(
            "t0_baseline_schema_unsupported",
            format!(
                "T0 baseline schema version {} is unsupported",
                snapshot.schema_version
            ),
            false,
        ));
    }
    for (field, value) in [
        ("candidate_id", snapshot.candidate_id.as_str()),
        ("stock_code", snapshot.stock_code.as_str()),
        ("source_batch_id", snapshot.source_batch_id.as_str()),
    ] {
        require_non_empty(field, value)?;
    }
    for (field, value) in [
        ("evaluation_price", snapshot.evaluation_price),
        ("observed_volume", snapshot.observed_volume),
        ("prior_5d_average_volume", snapshot.prior_5d_average_volume),
        (
            "prior_20d_average_volume",
            snapshot.prior_20d_average_volume,
        ),
        ("ma5", snapshot.ma5),
        ("ma10", snapshot.ma10),
        ("ma20", snapshot.ma20),
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(outcome_error(
                "t0_baseline_invalid",
                format!("{field} must be finite and positive, got {value}"),
                false,
            ));
        }
    }
    for (field, value) in [
        ("close_return", snapshot.close_return),
        (
            "close_volume_vs_observed",
            snapshot.close_volume_vs_observed,
        ),
        ("close_volume_vs_5d", snapshot.close_volume_vs_5d),
        ("close_volume_vs_20d", snapshot.close_volume_vs_20d),
        ("close_vs_ma5", snapshot.close_vs_ma5),
        ("close_vs_ma10", snapshot.close_vs_ma10),
        ("close_vs_ma20", snapshot.close_vs_ma20),
    ] {
        if !value.is_finite() {
            return Err(outcome_error(
                "t0_baseline_invalid",
                format!("{field} must be finite, got {value}"),
                false,
            ));
        }
    }
    let bar = SelectionBar {
        code: snapshot.stock_code.clone(),
        market_date: snapshot.market_date,
        open: snapshot.open,
        high: snapshot.high,
        low: snapshot.low,
        close: snapshot.close,
        volume: snapshot.volume,
        amount: snapshot.amount,
        settled: true,
        adjustment: crate::selection::quality::PriceAdjustment::Unadjusted,
        reference_previous_close: None,
    };
    validate_daily(&[bar]).map_err(|error| {
        outcome_error(
            "t0_baseline_invalid",
            format!("T0 baseline values are invalid: {error}"),
            false,
        )
    })?;
    Ok(())
}

fn validate_feature_baseline(baseline: &T0FeatureBaseline) -> Result<(), SelectionOutcomeError> {
    for (field, value) in [
        ("evaluation_price", baseline.market.evaluation_price),
        ("observed_volume", baseline.market.observed_volume),
        ("latest_settled_close", baseline.market.latest_settled_close),
        (
            "latest_settled_volume",
            baseline.market.latest_settled_volume,
        ),
        (
            "prior_5d_average_volume",
            baseline.market.prior_5d_average_volume,
        ),
        (
            "prior_20d_average_volume",
            baseline.market.prior_20d_average_volume,
        ),
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(outcome_error(
                "t0_feature_baseline_invalid",
                format!("{field} must be finite and positive, got {value}"),
                false,
            ));
        }
    }
    Ok(())
}

fn required_feature(field: &'static str, value: Option<f64>) -> Result<f64, SelectionOutcomeError> {
    let value = value.ok_or_else(|| {
        outcome_error(
            "t0_feature_baseline_missing",
            format!("T0 feature {field} is missing"),
            false,
        )
    })?;
    if !value.is_finite() || value <= 0.0 {
        return Err(outcome_error(
            "t0_feature_baseline_invalid",
            format!("T0 feature {field} must be finite and positive, got {value}"),
            false,
        ));
    }
    Ok(value)
}

fn require_non_empty(field: &'static str, value: &str) -> Result<(), SelectionOutcomeError> {
    if value.trim().is_empty() {
        return Err(outcome_error(
            "outcome_input_invalid",
            format!("{field} must not be empty"),
            false,
        ));
    }
    Ok(())
}

fn audit_phase(phase: OutcomePhase) -> SelectionAuditPhase {
    match phase {
        OutcomePhase::T0Close => SelectionAuditPhase::T0Close,
        OutcomePhase::D1Settled => SelectionAuditPhase::D1Settled,
    }
}

fn stable_hash<T: Serialize>(domain: &str, value: &T) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update(b"\0");
    hasher
        .update(serde_json::to_vec(value).expect("selection outcome hash payload must serialize"));
    hex::encode(hasher.finalize())
}

fn outcome_error(
    reason_code: &'static str,
    message: impl Into<String>,
    retryable: bool,
) -> SelectionOutcomeError {
    SelectionOutcomeError {
        reason_code,
        message: message.into(),
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::quality::PriceAdjustment;
    use chrono::NaiveDate;

    fn market_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 23).expect("valid market date")
    }

    fn test_snapshot(close: f64) -> T0CloseSnapshot {
        T0CloseSnapshot {
            schema_version: OUTCOME_SCHEMA_VERSION,
            candidate_id: "TEST_CODE_candidate".to_owned(),
            stock_code: "TEST_CODE_000001".to_owned(),
            market_date: market_date(),
            evaluation_price: close,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000_000.0,
            amount: close * 1_000_000.0,
            close_return: 0.0,
            observed_volume: 800_000.0,
            close_volume_vs_observed: 1.25,
            prior_5d_average_volume: 900_000.0,
            prior_20d_average_volume: 850_000.0,
            close_volume_vs_5d: 1_000_000.0 / 900_000.0,
            close_volume_vs_20d: 1_000_000.0 / 850_000.0,
            ma5: close,
            ma10: close,
            ma20: close,
            close_vs_ma5: 0.0,
            close_vs_ma10: 0.0,
            close_vs_ma20: 0.0,
            trend_alignment_maintained: true,
            source_batch_id: "TEST_CODE_magic_batch".to_owned(),
        }
    }

    fn settled_bar(open: f64, high: f64, low: f64, close: f64, volume: f64) -> SelectionBar {
        SelectionBar {
            code: "TEST_CODE_000001".to_owned(),
            market_date: crate::calendar::next_trading_day(market_date()),
            open,
            high,
            low,
            close,
            volume,
            amount: close * volume,
            settled: true,
            adjustment: PriceAdjustment::Unadjusted,
            reference_previous_close: None,
        }
    }

    fn feature_baseline() -> T0FeatureBaseline {
        T0FeatureBaseline {
            market: T0MarketEvidence {
                evaluation_price: 10.0,
                observed_volume: 800_000.0,
                latest_settled_market_date: crate::calendar::prev_trading_day(market_date()),
                latest_settled_close: 9.8,
                latest_settled_volume: 750_000.0,
                prior_5d_average_volume: 700_000.0,
                prior_20d_average_volume: 650_000.0,
            },
            features: RawSelectionFeatures {
                ma5: Some(9.8),
                ma10: Some(9.6),
                ma20: Some(9.4),
                five_day_return: Some(0.08),
                volume_vs_5d: Some(1.2),
                volume_vs_20d: Some(1.1),
                intraday_volume_pace: Some(1.3),
                price_vs_ma5: Some(10.0 / 9.8 - 1.0),
                price_vs_ma10: Some(10.0 / 9.6 - 1.0),
                price_vs_ma20: Some(10.0 / 9.4 - 1.0),
            },
        }
    }

    #[test]
    fn t0_close_retains_selection_time_price_volume_and_trend_baselines() {
        let request = DueOutcomeRequest {
            candidate_id: "TEST_CODE_candidate".to_owned(),
            stock_code: "TEST_CODE_000001".to_owned(),
            phase: OutcomePhase::T0Close,
            due_market_date: market_date(),
            t0_feature_baseline: Some(feature_baseline()),
            t0_baseline: None,
        };
        let bar = SelectionBar {
            code: "TEST_CODE_000001".to_owned(),
            market_date: market_date(),
            open: 10.1,
            high: 10.6,
            low: 9.9,
            close: 10.5,
            volume: 1_000_000.0,
            amount: 10_500_000.0,
            settled: true,
            adjustment: PriceAdjustment::Unadjusted,
            reference_previous_close: None,
        };
        let outcome =
            compute_t0_outcome(&request, &bar, "TEST_CODE_magic_t0_batch").expect("T0 outcome");
        assert!((outcome.close_return - 0.05).abs() < 1e-12);
        assert!((outcome.close_volume_vs_observed - 1.25).abs() < 1e-12);
        assert!(outcome.trend_alignment_maintained);
    }

    #[test]
    fn d1_outcome_uses_immutable_t0_baseline() {
        let outcome = compute_d1_outcome(
            &test_snapshot(10.0),
            &settled_bar(10.5, 11.5, 9.5, 11.0, 1_200_000.0),
            "TEST_CODE_magic_d1_batch",
        )
        .expect("valid D+1 outcome");
        assert!((outcome.open_return - 0.05).abs() < 1e-12);
        assert!((outcome.close_return - 0.10).abs() < 1e-12);
        assert!((outcome.mfe - 0.15).abs() < 1e-12);
        assert!((outcome.mae - -0.05).abs() < 1e-12);
    }

    #[test]
    fn unsettled_or_missing_session_is_expected_wait_not_empty() {
        let attempt = compute_due_outcome(
            &DueOutcomeRequest {
                candidate_id: "TEST_CODE_candidate".to_owned(),
                stock_code: "TEST_CODE_000001".to_owned(),
                phase: OutcomePhase::D1Settled,
                due_market_date: crate::calendar::next_trading_day(market_date()),
                t0_feature_baseline: None,
                t0_baseline: Some(test_snapshot(10.0)),
            },
            None,
            market_date(),
            "TEST_CODE_magic_batch",
        );
        assert!(matches!(attempt, OutcomeAttempt::ExpectedWait(_)));
    }

    #[test]
    fn d1_rejects_a_mutated_or_wrong_candidate_baseline() {
        let mut baseline = test_snapshot(10.0);
        baseline.candidate_id = "TEST_CODE_other".to_owned();
        let attempt = compute_due_outcome(
            &DueOutcomeRequest {
                candidate_id: "TEST_CODE_candidate".to_owned(),
                stock_code: "TEST_CODE_000001".to_owned(),
                phase: OutcomePhase::D1Settled,
                due_market_date: crate::calendar::next_trading_day(market_date()),
                t0_feature_baseline: None,
                t0_baseline: Some(baseline),
            },
            Some(&settled_bar(10.5, 11.5, 9.5, 11.0, 1_200_000.0)),
            crate::calendar::next_trading_day(market_date()),
            "TEST_CODE_magic_d1_batch",
        );
        assert!(matches!(
            attempt,
            OutcomeAttempt::Rejected(OutcomeRejection {
                reason_code: "t0_baseline_identity_mismatch",
                ..
            })
        ));
    }
}
