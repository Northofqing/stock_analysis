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
    acquisition_request_hash, audit_routed_gateway_result, BatchEvidence, GatewayBatch,
    GatewayError,
};
use super::security_lifecycle::SecurityLifecycleContext;
use super::security_lifecycle::{
    CorporateActionState, LifecycleConfirmationEvidence, ListingDateState, SecurityLifecycleGateway,
};

const CAPABILITY: &str = "HistoricalDailyBars";

/// Only a qualified Gateway Adapter may construct discovery authority. Neither
/// CLI JSON nor an error string can deserialize or construct this capability.
pub(crate) struct QualifiedDailyChangeDiscovery {
    snapshot: crate::database::daily_change_review::ReviewSnapshot,
}

impl QualifiedDailyChangeDiscovery {
    pub(crate) fn snapshot(&self) -> &crate::database::daily_change_review::ReviewSnapshot {
        &self.snapshot
    }
    #[cfg(test)]
    pub(crate) fn with_test_mutation(
        mut self,
        mutate: impl FnOnce(&mut crate::database::daily_change_review::ReviewSnapshot),
    ) -> Self {
        mutate(&mut self.snapshot);
        self
    }
}

#[cfg(test)]
pub(crate) fn qualified_review_fixture() -> QualifiedDailyChangeDiscovery {
    use crate::market_domain::{AssetClass, Exchange, InstrumentId};
    let previous_date = NaiveDate::from_ymd_opt(2026, 7, 16).unwrap();
    let current_date = NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
    QualifiedDailyChangeDiscovery {
        snapshot: crate::database::daily_change_review::ReviewSnapshot {
            schema_version: 1,
            discovery_contract: "outcome-provider-sequence-v1".into(),
            rule_version: "br171-close-change-v1".into(),
            instrument: InstrumentId::new(
                Exchange::Shenzhen,
                "TEST_CODE_300005",
                AssetClass::Equity,
            )
            .unwrap(),
            query: DailyChangeConfirmationQuery {
                code: "TEST_CODE_300005".into(),
                previous_date,
                current_date,
                previous_close: "18.59".into(),
                current_close: "14.87".into(),
                calculated_pct: canonical_decimal((14.87 / 18.59 - 1.0) * 100.0).unwrap(),
                daily_provider: "magic_tdx".into(),
                daily_source: "TEST_CODE_tdx".into(),
                daily_batch_id: "TEST_CODE_batch1".into(),
                lifecycle_provider: "magic_tdx".into(),
                lifecycle_batch_id: "TEST_CODE_lifecycle1".into(),
                listing_date: Some(NaiveDate::from_ymd_opt(2010, 1, 1).unwrap()),
                corporate_action_identity: None,
            },
            fact_payload: serde_json::json!({"fixture":"TEST_CODE_synthetic_ohlcv"}),
            raw_evidence: serde_json::json!({"fixture":"TEST_CODE_synthetic_raw_not_production"}),
        },
    }
}

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
                let result = bridge.daily_bars(code, days).and_then(|batch| {
                    super::grpc_source::block_on_gateway(finalize_ordinary_batch_async(
                        code.to_owned(),
                        batch,
                    ))
                });
                let audited = audit_routed_gateway_result(CAPABILITY, &request_hash, result)?;
                return AdmittedDailyBars::from_audited_batch(code.to_owned(), audited);
            }
            Err(error) => {
                let audited = audit_routed_gateway_result(CAPABILITY, &request_hash, Err(error))?;
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
                let result = match bridge.daily_bars_async(&code, days).await {
                    Ok(batch) => finalize_ordinary_batch_async(code.clone(), batch).await,
                    Err(error) => Err(error),
                };
                let audited = audit_routed_gateway_result(CAPABILITY, &request_hash, result)?;
                return AdmittedDailyBars::from_audited_batch(code, audited);
            }
            Err(error) => {
                let audited = audit_routed_gateway_result(CAPABILITY, &request_hash, Err(error))?;
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
            None,
            "unavailable",
            "daily_change_discovery_unavailable_v1",
            false,
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

pub(super) fn review_batch_facts(
    batch: &GatewayBatch<KlineData>,
) -> Result<serde_json::Value, GatewayError> {
    let records = batch
        .records()
        .iter()
        .map(review_bar_fact)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| final_admission_error(batch.evidence().provider, e))?;
    Ok(serde_json::json!({"records":records,"evidence":review_evidence(batch.evidence())}))
}

fn review_bar_fact(r: &KlineData) -> Result<serde_json::Value, String> {
    if r.close <= 0.0
        || r.open <= 0.0
        || r.low <= 0.0
        || r.high < r.low
        || r.volume <= 0.0
        || r.amount <= 0.0
        || r.close < r.low
        || r.close > r.high
        || r.open < r.low
        || r.open > r.high
    {
        return Err("review bar has invalid OHLCV/amount".into());
    }
    Ok(
        serde_json::json!({"date":r.date,"open":canonical_decimal(r.open)?,"high":canonical_decimal(r.high)?,"low":canonical_decimal(r.low)?,"close":canonical_decimal(r.close)?,"volume":canonical_decimal(r.volume)?,"amount":canonical_decimal(r.amount)?,"adjustment":format!("{:?}",r.adjust),"settled":r.settled}),
    )
}
fn review_evidence(e: &BatchEvidence) -> serde_json::Value {
    serde_json::json!({"provider":e.provider,"source":e.source,"source_at":e.source_at,"observed_at":e.observed_at,"batch_id":e.batch_id})
}
fn review_lifecycle(lc: &SecurityLifecycleContext) -> serde_json::Value {
    let listing = match &lc.listing {
        ListingDateState::Available(r) => {
            serde_json::json!({"state":"Available","listed_on":r.listed_on,"evidence":review_evidence(&r.evidence)})
        }
        ListingDateState::Unavailable { evidence, error } => {
            serde_json::json!({"state":"Unavailable","evidence":evidence.as_ref().map(review_evidence),"error":super::review::store_gateway_error(error)})
        }
    };
    let actions = match &lc.corporate_actions {
        CorporateActionState::Available { records, evidence } => {
            serde_json::json!({"state":"Available","evidence":review_evidence(evidence),"records":records.iter().map(|r|serde_json::json!({"code":r.code,"category":r.category,"effective_on":r.effective_on,"record_on":r.record_on,"ex_on":r.ex_on,"payable_on":r.payable_on,"terms":r.terms})).collect::<Vec<_>>()})
        }
        CorporateActionState::VerifiedEmpty(e) => {
            serde_json::json!({"state":"VerifiedEmpty","evidence":review_evidence(e)})
        }
        CorporateActionState::Unavailable(e) => {
            serde_json::json!({"state":"Unavailable","error":super::review::store_gateway_error(e)})
        }
    };
    serde_json::json!({"instrument":lc.instrument,"window_start":lc.window_start,"window_end":lc.window_end,"listing":listing,"actions":actions})
}

fn finalize_changes_on_conn(
    conn: &mut diesel::SqliteConnection,
    code: &str,
    batch: &GatewayBatch<KlineData>,
    lifecycle: &SecurityLifecycleContext,
    raw: Option<&super::outcome_daily_bars::OutcomeReviewEvidence>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), GatewayError> {
    use crate::database::daily_change_review::{self as review, ReviewError, ReviewSnapshot};
    let provider = batch.evidence().provider;
    let fail = |e: String| {
        final_admission_error(provider, format!("manual_confirmation_lookup_failed: {e}"))
    };
    admit_outcome_lifecycle(code, batch, lifecycle)?;
    let raw_payload = raw
        .map(|raw| {
            if raw.instrument() != &lifecycle.instrument {
                return Err(fail("raw/lifecycle instrument mismatch".into()));
            }
            raw.for_batch(batch).cloned()
        })
        .transpose()?;
    let changes = outcome_pending_changes(code, batch)?;
    let mut unconfirmed = Vec::new();
    for change in &changes {
        let lifecycle_fact =
            lifecycle.confirmation_evidence_for(change.previous_date, change.current_date)?;
        let query =
            build_confirmation_query(change, batch.evidence(), &lifecycle_fact).map_err(&fail)?;
        let pair = batch
            .records()
            .iter()
            .filter(|r| r.date == change.previous_date || r.date == change.current_date)
            .map(review_bar_fact)
            .collect::<Result<Vec<_>, _>>()
            .map_err(&fail)?;
        let snapshot = ReviewSnapshot {
            schema_version: 1,
            discovery_contract: "outcome-provider-sequence-v1".into(),
            rule_version: "br171-close-change-v1".into(),
            instrument: lifecycle.instrument.clone(),
            query,
            fact_payload: serde_json::json!({"adjacent_bars":pair}),
            raw_evidence: serde_json::json!({"daily":raw_payload,"lifecycle":review_lifecycle(lifecycle)}),
        };
        let mut candidate_id = None;
        if raw.is_some() {
            match review::discover_on_conn(
                conn,
                &QualifiedDailyChangeDiscovery {
                    snapshot: snapshot.clone(),
                },
                now,
            ) {
                Ok(candidate) => candidate_id = Some(candidate.candidate_id),
                Err(ReviewError::AlreadyConfirmed) => {}
                // A valid old database without this extension can still read
                // exact legacy confirmations; it can never invent a candidate.
                Err(ReviewError::Unavailable)
                    if review::admit_on_conn(conn, &snapshot)
                        .map_err(|e| fail(e.to_string()))? => {}
                Err(e) => return Err(fail(e.to_string())),
            }
        }
        if !review::admit_on_conn(conn, &snapshot).map_err(|e| fail(e.to_string()))? {
            unconfirmed.push(candidate_id.unwrap_or_else(|| {
                format!("{}:{}:{}", code, change.previous_date, change.current_date)
            }));
        }
    }
    if unconfirmed.is_empty() {
        return Ok(());
    }
    Err(GatewayError::classified(
        CAPABILITY,
        Some(provider),
        "unavailable",
        if raw.is_some() {
            "manual_confirmation_required"
        } else {
            "daily_change_discovery_unavailable_v1"
        },
        false,
        format!(
            "BR-171 pending/expired/rejected facts require exact persisted review; candidates={}",
            unconfirmed.join(",")
        ),
    ))
}

fn confirm_changes_with_database(
    code: &str,
    batch: &GatewayBatch<KlineData>,
    lifecycle: &SecurityLifecycleContext,
    raw: Option<&super::outcome_daily_bars::OutcomeReviewEvidence>,
) -> Result<(), GatewayError> {
    let fail = |e: String| {
        final_admission_error(
            batch.evidence().provider,
            format!("manual_confirmation_lookup_failed: {e}"),
        )
    };
    let database = DatabaseManager::try_get()
        .ok_or_else(|| fail("confirmation database is not initialized".into()))?;
    let mut conn = database.get_conn().map_err(|e| fail(e.to_string()))?;
    finalize_changes_on_conn(&mut conn, code, batch, lifecycle, raw, chrono::Utc::now())
}

async fn finalize_ordinary_batch_async(
    code: String,
    batch: GatewayBatch<KlineData>,
) -> Result<GatewayBatch<KlineData>, GatewayError> {
    let mut records = batch.records().to_vec();
    records.sort_by_key(|r| r.date);
    let ordered = GatewayBatch::Available {
        records,
        evidence: batch.evidence().clone(),
    };
    if outcome_pending_changes(&code, &ordered)?.is_empty() {
        return Ok(batch);
    }
    let (start, end) = batch_window(&ordered)?;
    let lifecycle = SecurityLifecycleGateway::new()
        .acquire(&code, start, end)
        .await?;
    tokio::task::spawn_blocking(move || {
        confirm_changes_with_database(&code, &ordered, &lifecycle, None)?;
        Ok(batch)
    })
    .await
    .map_err(|e| {
        GatewayError::unavailable(CAPABILITY, None, true, format!("daily review worker: {e}"))
    })?
}

/// Final BR-171/lifecycle admission for an immutable schema-v2 outcome
/// sequence. Interior dates are provider evidence; no current calendar is
/// allowed to rewrite or reconstruct them.
pub(super) async fn finalize_outcome_sequence_async(
    code: String,
    batch: GatewayBatch<KlineData>,
    raw: super::outcome_daily_bars::OutcomeReviewEvidence,
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
        confirm_changes_with_database(&code, &batch, &lifecycle, Some(&raw))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::RunQueryDsl;

    #[derive(diesel::QueryableByName)]
    struct AuditProviderRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        provider: String,
    }

    #[tokio::test]
    async fn historical_bridge_failure_does_not_claim_tdx_provider() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();

        let result = HistoricalBarsGateway::new()
            .daily_bars_async("399991", 5)
            .await;

        std::env::remove_var("GRPC_MARKET_ADDR");
        super::super::grpc_source::reset_bridge();

        let error = result.expect_err("unreachable bridge must fail closed");
        assert_eq!(error.provider(), None);

        let request_hash = acquisition_request_hash(CAPABILITY, "399991:5");
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT provider FROM data_acquisition_audit \
             WHERE capability = 'HistoricalDailyBars' AND request_hash = ? \
             ORDER BY id DESC LIMIT 1",
        )
        .bind::<diesel::sql_types::Text, _>(request_hash)
        .get_result::<AuditProviderRow>(&mut *connection)
        .expect("bridge failure must be audited");
        assert_eq!(row.provider, "Custom");
    }
}

#[cfg(test)]
#[path = "historical_bars_review_tests.rs"]
mod review_tests;
