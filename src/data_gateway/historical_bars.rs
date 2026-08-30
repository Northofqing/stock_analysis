//! BR-064/BR-065/BR-108/BR-125/BR-128/BR-158/BR-159/BR-164/BR-172
//! daily-bar boundary.
//!
//! Production routing is ordered and deterministic:
//! Magic TDX -> Magic Tencent -> Magic Sina -> Magic Baidu.
//! A source can only win after the upstream batch is complete, carries source
//! time and batch evidence, and its complete daily series passes BR-092. A
//! missing amount, partial cardinality, stale latest bar, or bad series rejects
//! that source and permits the next registered source. No field is filled or
//! estimated at this boundary.

use crate::market_domain::ProviderId;
use crate::market_domain::SecurityBar;

use chrono::NaiveDate;

use crate::data_provider::KlineData;
use crate::database::daily_change_confirmation::DailyChangeConfirmationQuery;
use crate::database::DatabaseManager;

use crate::monitor::data_quality::{
    AdjacentDailyChange, MAX_UNCONFIRMED_ADJACENT_DAILY_CHANGE_PCT,
};

use super::review::{
    acquisition_request_hash, audit_gateway_result, BatchEvidence, GatewayBatch, GatewayError,
};
use super::security_lifecycle::SecurityLifecycleContext;
use super::security_lifecycle::{
    CorporateActionState, LifecycleConfirmationEvidence, ListingDateState, SecurityLifecycleGateway,
};

const CAPABILITY: &str = "HistoricalDailyBars";

/// BR-216: record Kline liveness at the gateway admission point.
///
/// Sinking the marker here (instead of at each business call site) is what
/// keeps "a production fetch exists but nobody marked the capability" from
/// recurring: every admitted daily-bar batch, whatever the caller, proves the
/// Kline source is alive. Only an admitted batch reaches this point, so the
/// marker never fabricates freshness for a failed acquisition.
fn mark_daily_bars_capability_live() -> Result<(), GatewayError> {
    crate::monitor::data_mode::mark_capability_success(crate::monitor::data_mode::Capability::Kline)
        .map_err(|error| GatewayError::unavailable(CAPABILITY, None, false, error))
}

/// Exact lifecycle proof retained by schema-v2 outcome admission.
#[derive(Debug, Clone)]
pub(super) struct OutcomeLifecycleAdmission {
    pub window_start: NaiveDate,
    pub window_end: NaiveDate,
    pub listing_date: Option<NaiveDate>,
    pub listing_batch_id: Option<String>,
    pub listing_unavailable_reason_code: Option<String>,
    pub listing_unavailable_retryable: Option<bool>,
    pub corporate_action_state: String,
    pub corporate_action_batch_id: String,
    pub adjacent_evidence: Vec<LifecycleConfirmationEvidence>,
}

/// Production daily-bar Gateway. Provider transports and protocol parsing stay
/// exclusively in the pinned `magic-market-data-rs` crates.
#[derive(Debug, Clone, Copy, Default)]
pub struct HistoricalBarsGateway;

/// A non-empty daily-bar batch kept together with the evidence that admitted it.
///
/// Private fields prevent consumers from constructing an evidence-free batch or
/// accidentally replacing the records independently of their provenance.
#[derive(Debug)]
pub struct AdmittedDailyBars {
    target_code: String,
    records: Vec<KlineData>,
    evidence: BatchEvidence,
}

impl AdmittedDailyBars {
    /// Exact storage identity supplied to and validated by the production
    /// Gateway request that acquired this batch.
    pub fn target_code(&self) -> &str {
        &self.target_code
    }

    pub fn records(&self) -> &[KlineData] {
        &self.records
    }

    pub const fn evidence(&self) -> &BatchEvidence {
        &self.evidence
    }

    /// Consume an already-admitted capability while keeping records and
    /// evidence bound until the consumer explicitly takes ownership.
    pub fn into_parts(self) -> (Vec<KlineData>, BatchEvidence) {
        (self.records, self.evidence)
    }

    /// Consume the capability without dropping the request identity that was
    /// bound at the Gateway boundary.
    pub fn into_bound_parts(self) -> (String, Vec<KlineData>, BatchEvidence) {
        (self.target_code, self.records, self.evidence)
    }

    /// Only this module can turn the audited transport envelope into the
    /// capability type. Public `GatewayBatch<KlineData>` values therefore
    /// cannot forge proof that identity, quality and freshness admission ran.
    fn from_audited_batch(
        target_code: String,
        batch: GatewayBatch<KlineData>,
    ) -> Result<Self, GatewayError> {
        match batch {
            GatewayBatch::Available { records, evidence } if !records.is_empty() => Ok(Self {
                target_code,
                records,
                evidence,
            }),
            GatewayBatch::Available { evidence, .. } | GatewayBatch::VerifiedEmpty(evidence) => {
                Err(GatewayError::unavailable(
                    CAPABILITY,
                    Some(evidence.provider),
                    true,
                    format!(
                        "provider returned no admitted daily bars source={} batch_id={}",
                        evidence.source, evidence.batch_id
                    ),
                ))
            }
        }
    }

    /// Pure, crate-local fixture seam for unit tests. This symbol is absent
    /// from production builds and requires the repository TEST_CODE namespace.
    #[cfg(test)]
    pub(crate) fn from_test_fixture(
        target_code: &str,
        records: Vec<KlineData>,
        evidence: BatchEvidence,
    ) -> Result<Self, GatewayError> {
        if !target_code.starts_with("TEST_CODE_")
            || !evidence.source.starts_with("TEST_CODE")
            || !evidence.batch_id.starts_with("TEST_CODE")
        {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "daily-bar test fixture identity/evidence must use TEST_CODE namespace",
            ));
        }
        Self::from_audited_batch(
            target_code.to_owned(),
            GatewayBatch::Available { records, evidence },
        )
    }
}

impl HistoricalBarsGateway {
    pub const fn new() -> Self {
        Self
    }

    /// 15 分钟 K线（升序，旧→新）。R-12 盘后回测取数，覆盖虚拟仓全部
    /// 历史信号（7/14 起，800 根约 50 个交易日）。使用远端 TechnicalBars
    /// operation，不参与 daily-bars route（日 K 使用独立语义）。
    ///
    /// 失败/空 batch 显式返回 GatewayError, 不静默填零。
    pub fn fifteen_min_bars(
        &self,
        code: &str,
        count: usize,
    ) -> Result<Vec<SecurityBar>, GatewayError> {
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                format!("fifteen_min_bars invalid code: {code}"),
            ));
        }
        if count == 0 || count > 800 {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                format!("fifteen_min_bars invalid count: {count} (1..=800)"),
            ));
        }
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; 本地无 audit,
        // 桥路径亦不 audit — 与本地行为一致)。
        match super::grpc_source::bridge_for("TechnicalBars") {
            Ok(bridge) => {
                let batch = bridge
                    .technical_bars(&[code.to_string()], count as u32)
                    .map_err(|error| {
                        GatewayError::unavailable(
                            CAPABILITY,
                            None,
                            true,
                            format!("15min bars gRPC 桥失败 ({code}): {error}"),
                        )
                    })?;
                let records: Vec<SecurityBar> = batch.records().to_vec();
                if records.is_empty() {
                    return Err(GatewayError::unavailable(
                        CAPABILITY,
                        None,
                        false,
                        format!("15min bars gRPC 空 for {code}"),
                    ));
                }
                return Ok(records);
            }
            Err(error) => {
                return Err(GatewayError::unavailable(
                    CAPABILITY,
                    None,
                    true,
                    error.to_string(),
                ));
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }

    pub fn daily_bars(&self, code: &str, days: usize) -> Result<AdmittedDailyBars, GatewayError> {
        let request_hash = acquisition_request_hash(CAPABILITY, format!("{code}:{days}"));
        // P4 M2 钩子: remote gRPC → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("HistoricalBars") {
            Ok(bridge) => {
                let result = bridge.daily_bars(code, days);
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                let audited =
                    audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result)?;
                return AdmittedDailyBars::from_audited_batch(code.to_owned(), audited);
            }
            Err(error) => {
                let audited =
                    audit_gateway_result(CAPABILITY, ProviderId::Tdx, &request_hash, Err(error))?;
                return AdmittedDailyBars::from_audited_batch(code.to_owned(), audited);
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }

    /// Async entry for consumers that already run inside Tokio.
    ///
    /// The Magic provider clients expose a blocking historical-bars contract,
    /// so the blocking work is isolated here instead of being reimplemented
    /// by each consumer.
    pub async fn daily_bars_async(
        &self,
        code: &str,
        days: usize,
    ) -> Result<AdmittedDailyBars, GatewayError> {
        let code = code.to_owned();
        let request_hash = acquisition_request_hash(CAPABILITY, format!("{code}:{days}"));
        // P4 M2 钩子: remote gRPC → gRPC 通道 (async 路径, 不 block_on)。
        match super::grpc_source::bridge_for("HistoricalBars") {
            Ok(bridge) => {
                let result = bridge.daily_bars_async(&code, days).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                let audited =
                    audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result)?;
                return AdmittedDailyBars::from_audited_batch(code, audited);
            }
            Err(error) => {
                let audited =
                    audit_gateway_result(CAPABILITY, ProviderId::Tdx, &request_hash, Err(error))?;
                return AdmittedDailyBars::from_audited_batch(code, audited);
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }

    /// Fetch a daily-bar batch that is guaranteed to be non-empty and whose
    /// source evidence cannot be discarded independently of its records.
    pub fn required_daily_bars(
        &self,
        code: &str,
        days: usize,
    ) -> Result<AdmittedDailyBars, GatewayError> {
        let admitted = self.daily_bars(code, days)?;
        mark_daily_bars_capability_live()?;
        Ok(admitted)
    }

    /// Async counterpart of [`Self::required_daily_bars`].
    pub async fn required_daily_bars_async(
        &self,
        code: &str,
        days: usize,
    ) -> Result<AdmittedDailyBars, GatewayError> {
        let admitted = self.daily_bars_async(code, days).await?;
        mark_daily_bars_capability_live()?;
        Ok(admitted)
    }

    /// Acquire source and lifecycle evidence for every adjacent close that is
    /// awaiting an explicit BR-171 operator decision.
    ///
    /// This is a review-only interface: it never looks up or appends a
    /// confirmation and never constructs [`AdmittedDailyBars`]. Callers must
    /// pass one returned query unchanged to the immutable confirmation ledger.
    pub async fn pending_daily_change_confirmations_async(
        &self,
        code: &str,
        days: usize,
    ) -> Result<Vec<DailyChangeConfirmationQuery>, GatewayError> {
        let code = code.to_owned();
        Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tdx),
            "unavailable",
            "provider_transport",
            true,
            &format!(
                "daily-change confirmation discovery is unavailable over the remote transport \
                 (code={code}, days={days})"
            ),
        ))
    }
}

pub const fn daily_bar_provider_label(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Tdx => "magic_tdx",
        ProviderId::Tencent => "magic_tencent",
        ProviderId::Sina => "magic_sina",
        ProviderId::Baidu => "magic_baidu",
        _ => "magic_unknown",
    }
}

fn final_admission_error(provider: ProviderId, error: String) -> GatewayError {
    let reason_code = if error.contains("manual_confirmation_required") {
        "manual_confirmation_required"
    } else if error.contains("manual_confirmation_lookup_failed") {
        "manual_confirmation_lookup_failed"
    } else {
        "selected_batch_quality_rejected"
    };
    GatewayError::classified(
        CAPABILITY,
        Some(provider),
        "partial",
        reason_code,
        false,
        format!("selected daily batch failed final BR-092/BR-171 admission: {error}"),
    )
}

fn batch_window(batch: &GatewayBatch<KlineData>) -> Result<(NaiveDate, NaiveDate), GatewayError> {
    let first = batch.records().first().ok_or_else(|| {
        GatewayError::unavailable(
            CAPABILITY,
            Some(batch.evidence().provider),
            true,
            "selected daily batch has no records",
        )
    })?;
    let (minimum, maximum) = batch
        .records()
        .iter()
        .map(|record| record.date)
        .fold((first.date, first.date), |(minimum, maximum), date| {
            (minimum.min(date), maximum.max(date))
        });
    Ok((minimum, maximum))
}

fn canonical_decimal(value: f64) -> Result<String, String> {
    if !value.is_finite() {
        return Err(format!("non-finite confirmation decimal {value}"));
    }
    let mut output = format!("{value:.12}");
    while output.contains('.') && output.ends_with('0') {
        output.pop();
    }
    if output.ends_with('.') {
        output.pop();
    }
    Ok(output)
}

fn build_confirmation_query(
    change: &AdjacentDailyChange,
    daily_evidence: &BatchEvidence,
    lifecycle: &LifecycleConfirmationEvidence,
) -> Result<DailyChangeConfirmationQuery, String> {
    Ok(DailyChangeConfirmationQuery {
        code: change.code.clone(),
        previous_date: change.previous_date,
        current_date: change.current_date,
        previous_close: canonical_decimal(change.previous_close)?,
        current_close: canonical_decimal(change.current_close)?,
        calculated_pct: canonical_decimal(change.change_pct)?,
        daily_provider: daily_bar_provider_label(daily_evidence.provider).to_string(),
        daily_source: daily_evidence.source.clone(),
        daily_batch_id: daily_evidence.batch_id.clone(),
        lifecycle_provider: lifecycle.provider.clone(),
        lifecycle_batch_id: lifecycle.batch_identity.clone(),
        listing_date: lifecycle.listing_date,
        corporate_action_identity: lifecycle.corporate_action_identity.clone(),
    })
}

/// BR-174 outcome windows retain the exact immutable T0..due provider
/// sequence. This detector therefore treats that already provider-ordered
/// sequence as the adjacency contract and deliberately does not consult the
/// mutable process calendar to reconstruct interior dates.
fn outcome_pending_changes(
    code: &str,
    batch: &GatewayBatch<KlineData>,
) -> Result<Vec<AdjacentDailyChange>, GatewayError> {
    let provider = batch.evidence().provider;
    let records = match batch {
        GatewayBatch::Available { records, .. } if !records.is_empty() => records,
        GatewayBatch::Available { .. } | GatewayBatch::VerifiedEmpty(_) => {
            return Err(GatewayError::unavailable(
                CAPABILITY,
                Some(provider),
                true,
                "outcome daily-bar sequence cannot be empty",
            ))
        }
    };
    let mut pending = Vec::new();
    for pair in records.windows(2) {
        let previous = &pair[0];
        let current = &pair[1];
        if previous.date >= current.date {
            return Err(final_admission_error(
                provider,
                format!(
                    "[{code}] outcome provider sequence is duplicate/non-increasing at {}→{}",
                    previous.date, current.date
                ),
            ));
        }
        let change_pct = (current.close - previous.close) / previous.close * 100.0;
        if !change_pct.is_finite() {
            return Err(final_admission_error(
                provider,
                format!(
                    "[{code}] outcome adjacent change is non-finite at {}→{}",
                    previous.date, current.date
                ),
            ));
        }
        if change_pct.abs() > MAX_UNCONFIRMED_ADJACENT_DAILY_CHANGE_PCT {
            pending.push(AdjacentDailyChange {
                code: code.to_owned(),
                previous_date: previous.date,
                current_date: current.date,
                previous_close: previous.close,
                current_close: current.close,
                change_pct,
            });
        }
    }
    Ok(pending)
}

fn admit_outcome_lifecycle(
    code: &str,
    batch: &GatewayBatch<KlineData>,
    lifecycle: &SecurityLifecycleContext,
) -> Result<OutcomeLifecycleAdmission, GatewayError> {
    let provider = batch.evidence().provider;
    if lifecycle.instrument.code() != code {
        return Err(final_admission_error(
            provider,
            format!(
                "outcome lifecycle instrument {} conflicts with requested code {code}",
                lifecycle.instrument.code()
            ),
        ));
    }
    let (window_start, window_end) = batch_window(batch)?;
    if lifecycle.window_start != window_start || lifecycle.window_end != window_end {
        return Err(final_admission_error(
            provider,
            format!(
                "outcome lifecycle window {}..{} conflicts with daily window {window_start}..{window_end}",
                lifecycle.window_start, lifecycle.window_end
            ),
        ));
    }

    let (
        listing_date,
        listing_batch_id,
        listing_unavailable_reason_code,
        listing_unavailable_retryable,
    ) = match &lifecycle.listing {
        ListingDateState::Available(listing) => (
            Some(listing.listed_on),
            Some(listing.evidence.batch_id.clone()),
            None,
            None,
        ),
        ListingDateState::Unavailable { evidence, error } => (
            None,
            evidence.as_ref().map(|evidence| evidence.batch_id.clone()),
            Some(error.reason_code().to_string()),
            Some(error.retryable()),
        ),
    };
    let (corporate_action_state, corporate_action_batch_id) = match &lifecycle.corporate_actions {
        CorporateActionState::Available { evidence, .. } => {
            ("available".to_string(), evidence.batch_id.clone())
        }
        CorporateActionState::VerifiedEmpty(evidence) => {
            ("verified_empty".to_string(), evidence.batch_id.clone())
        }
        CorporateActionState::Unavailable(error) => {
            return Err(GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Tdx),
                error.audit_outcome(),
                "corporate_action_context_unavailable",
                error.retryable(),
                format!("outcome lifecycle has no exact corporate-action coverage: {error}"),
            ))
        }
    };

    let adjacent_evidence = batch
        .records()
        .windows(2)
        .map(|pair| {
            lifecycle
                .confirmation_evidence_for(pair[0].date, pair[1].date)
                .map_err(|error| {
                    final_admission_error(
                        provider,
                        format!(
                            "outcome lifecycle adjacency {}→{} rejected: {error}",
                            pair[0].date, pair[1].date
                        ),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(OutcomeLifecycleAdmission {
        window_start,
        window_end,
        listing_date,
        listing_batch_id,
        listing_unavailable_reason_code,
        listing_unavailable_retryable,
        corporate_action_state,
        corporate_action_batch_id,
        adjacent_evidence,
    })
}

fn confirm_outcome_changes(
    batch: &GatewayBatch<KlineData>,
    changes: &[AdjacentDailyChange],
    lifecycle: &SecurityLifecycleContext,
) -> Result<(), GatewayError> {
    let daily_evidence = batch.evidence();
    let provider = daily_evidence.provider;
    let database = DatabaseManager::try_get().ok_or_else(|| {
        final_admission_error(
            provider,
            "BR-171 manual_confirmation_lookup_failed: confirmation database is not initialized"
                .to_string(),
        )
    })?;
    for change in changes {
        let lifecycle_evidence = lifecycle
            .confirmation_evidence_for(change.previous_date, change.current_date)
            .map_err(|error| {
                final_admission_error(
                    provider,
                    format!(
                        "BR-171 manual_confirmation_lookup_failed {}→{}: {error}",
                        change.previous_date, change.current_date
                    ),
                )
            })?;
        let query = build_confirmation_query(change, daily_evidence, &lifecycle_evidence).map_err(
            |error| {
                final_admission_error(
                    provider,
                    format!(
                        "BR-171 manual_confirmation_lookup_failed {}→{}: {error}",
                        change.previous_date, change.current_date
                    ),
                )
            },
        )?;
        let confirmed = database
            .has_exact_daily_change_confirmation(&query)
            .map_err(|error| {
                final_admission_error(
                    provider,
                    format!(
                        "BR-171 manual_confirmation_lookup_failed {}→{}: {error}",
                        change.previous_date, change.current_date
                    ),
                )
            })?;
        if !confirmed {
            log::warn!(
                "[BR-171] manual_confirmation_required code={} dates={}→{} \
                 closes={:.6}→{:.6} change={:.4}%",
                change.code,
                change.previous_date,
                change.current_date,
                change.previous_close,
                change.current_close,
                change.change_pct
            );
            return Err(final_admission_error(
                provider,
                format!(
                    "[{}] BR-171 manual_confirmation_required {}→{} \
                     closes={:.6}→{:.6} change={:.4}%",
                    change.code,
                    change.previous_date,
                    change.current_date,
                    change.previous_close,
                    change.current_close,
                    change.change_pct
                ),
            ));
        }
    }
    Ok(())
}

/// Final BR-171/lifecycle admission for an immutable schema-v2 outcome
/// sequence. Interior dates are provider evidence; no current calendar is
/// allowed to rewrite or reconstruct them.
pub(super) async fn finalize_outcome_sequence_async(
    code: String,
    batch: GatewayBatch<KlineData>,
) -> Result<(GatewayBatch<KlineData>, OutcomeLifecycleAdmission), GatewayError> {
    let pending = outcome_pending_changes(&code, &batch)?;
    let (window_start, window_end) = batch_window(&batch)?;
    let lifecycle = SecurityLifecycleGateway::new()
        .acquire(&code, window_start, window_end)
        .await?;
    let admission = admit_outcome_lifecycle(&code, &batch, &lifecycle)?;
    if pending.is_empty() {
        return Ok((batch, admission));
    }
    tokio::task::spawn_blocking(move || {
        confirm_outcome_changes(&batch, &pending, &lifecycle)?;
        Ok((batch, admission))
    })
    .await
    .map_err(|error| {
        GatewayError::unavailable(
            CAPABILITY,
            None,
            true,
            format!("outcome daily-bars confirmation task failed: {error}"),
        )
    })?
}
