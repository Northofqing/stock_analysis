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

use chrono::NaiveDate;

use crate::data_provider::{AdjustType, KlineData};
use crate::database::daily_change_confirmation::DailyChangeConfirmationQuery;
use crate::database::DatabaseManager;
use sha2::{Digest, Sha256};

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
const OBSERVED_PROJECTION_DOMAIN: &str = "stock_analysis.m4.observed_daily_ohlcv_projection.v1";

/// Only a qualified Gateway Adapter may construct discovery authority. Neither
/// CLI JSON nor an error string can deserialize or construct this capability.
pub(crate) struct QualifiedDailyChangeDiscovery {
    snapshot: crate::database::daily_change_review::ReviewSnapshot,
}

impl QualifiedDailyChangeDiscovery {
    pub(crate) fn from_window_pair(
        pair: super::ordinary_daily_change_window::QualifiedPair,
    ) -> Self {
        Self {
            snapshot: pair.into_snapshot(),
        }
    }
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
    requested_days: usize,
    records: Vec<KlineData>,
    evidence: BatchEvidence,
}

impl AdmittedDailyBars {
    /// Exact storage identity supplied to and validated by the production
    /// Gateway request that acquired this batch.
    pub fn target_code(&self) -> &str {
        &self.target_code
    }

    /// Actual Gateway request count; it does not identify exact trading days.
    pub const fn requested_days(&self) -> usize {
        self.requested_days
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

    /// Seal the Gateway's admitted observation. This is not a full-window,
    /// point-in-time, or persisted-read verification.
    pub fn into_observed_projection(self) -> Result<ObservedDailyBarsCapture, GatewayError> {
        ObservedDailyBarsCapture::from_admitted(self)
    }

    /// Only this module can turn the audited transport envelope into the
    /// capability type. Public `GatewayBatch<KlineData>` values therefore
    /// cannot forge proof that identity, quality and freshness admission ran.
    fn from_audited_batch(
        target_code: String,
        requested_days: usize,
        batch: GatewayBatch<KlineData>,
    ) -> Result<Self, GatewayError> {
        match batch {
            GatewayBatch::Available { records, evidence } if !records.is_empty() => Ok(Self {
                target_code,
                requested_days,
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
        let requested_days = records.len();
        Self::from_test_fixture_with_days(target_code, requested_days, records, evidence)
    }

    #[cfg(test)]
    pub(crate) fn from_test_fixture_with_days(
        target_code: &str,
        requested_days: usize,
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
            requested_days,
            GatewayBatch::Available { records, evidence },
        )
    }
}

/// Immutable observed projection. Only these fields, in this order, are bound
/// by the capture hash. `KlineData`'s derived indicators, `pct_chg`, settled
/// state, intraday price, and financial fields are outside this projection.
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedDailyBarProjection {
    date: NaiveDate,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    amount: f64,
    adjust: AdjustType,
}

impl ObservedDailyBarProjection {
    pub const fn date(&self) -> NaiveDate {
        self.date
    }

    /// In order: open, high, low, close, volume, amount.
    pub const fn ohlcv_amount(&self) -> [f64; 6] {
        [
            self.open,
            self.high,
            self.low,
            self.close,
            self.volume,
            self.amount,
        ]
    }

    pub const fn adjust(&self) -> AdjustType {
        self.adjust
    }

    fn from_record(record: KlineData, provider: ProviderId) -> Result<Self, GatewayError> {
        for (field, value) in [
            ("open", record.open),
            ("high", record.high),
            ("low", record.low),
            ("close", record.close),
            ("volume", record.volume),
            ("amount", record.amount),
        ] {
            if !value.is_finite() {
                return Err(GatewayError::invalid_evidence(
                    CAPABILITY,
                    Some(provider),
                    format!(
                        "observed projection has nonfinite {field} on {}",
                        record.date
                    ),
                ));
            }
        }
        Ok(Self {
            date: record.date,
            open: record.open,
            high: record.high,
            low: record.low,
            close: record.close,
            volume: record.volume,
            amount: record.amount,
            adjust: record.adjust,
        })
    }
}

/// A sealed Gateway observation, not a full-window, PIT, or persisted proof.
/// Its hash covers the actual request, ordered projection, and all five
/// `BatchEvidence` fields; it does not identify the provider's original wire.
#[derive(Debug)]
pub struct ObservedDailyBarsCapture {
    target_code: String,
    requested_days: usize,
    bars: Vec<ObservedDailyBarProjection>,
    evidence: BatchEvidence,
    projection_hash: String,
}

impl ObservedDailyBarsCapture {
    pub fn target_code(&self) -> &str {
        &self.target_code
    }

    pub const fn requested_days(&self) -> usize {
        self.requested_days
    }

    pub fn bars(&self) -> &[ObservedDailyBarProjection] {
        &self.bars
    }

    pub const fn evidence(&self) -> &BatchEvidence {
        &self.evidence
    }

    pub fn projection_hash(&self) -> &str {
        &self.projection_hash
    }

    fn from_admitted(admitted: AdmittedDailyBars) -> Result<Self, GatewayError> {
        let AdmittedDailyBars {
            target_code,
            requested_days,
            records,
            evidence,
        } = admitted;
        let bars = records
            .into_iter()
            .map(|record| ObservedDailyBarProjection::from_record(record, evidence.provider))
            .collect::<Result<Vec<_>, _>>()?;
        let projection_hash =
            observed_projection_hash(&target_code, requested_days, &bars, &evidence)?;
        Ok(Self {
            target_code,
            requested_days,
            bars,
            evidence,
            projection_hash,
        })
    }
}

fn hash_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn observed_projection_hash(
    target_code: &str,
    requested_days: usize,
    bars: &[ObservedDailyBarProjection],
    evidence: &BatchEvidence,
) -> Result<String, GatewayError> {
    let provider_wire = serde_json::to_vec(&evidence.provider).map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(evidence.provider),
            format!("provider identity serialization failed: {error}"),
        )
    })?;
    let mut hasher = Sha256::new();
    hash_length_prefixed(&mut hasher, OBSERVED_PROJECTION_DOMAIN.as_bytes());
    hash_length_prefixed(&mut hasher, target_code.as_bytes());
    hasher.update((requested_days as u64).to_be_bytes());
    hasher.update((bars.len() as u64).to_be_bytes());
    for bar in bars {
        hash_length_prefixed(&mut hasher, bar.date.to_string().as_bytes());
        for value in bar.ohlcv_amount() {
            // Admission to this projection rejects NaN and infinity first.
            // IEEE-754 bits retain exact finite values, including -0.0.
            hasher.update(value.to_bits().to_be_bytes());
        }
        hash_length_prefixed(&mut hasher, bar.adjust.as_str().as_bytes());
    }
    hash_length_prefixed(&mut hasher, &provider_wire);
    hash_length_prefixed(&mut hasher, evidence.source.as_bytes());
    match &evidence.source_at {
        Some(source_at) => {
            hasher.update([1]);
            hash_length_prefixed(&mut hasher, source_at.as_bytes());
        }
        None => hasher.update([0]),
    }
    hash_length_prefixed(&mut hasher, evidence.observed_at.as_bytes());
    hash_length_prefixed(&mut hasher, evidence.batch_id.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

impl HistoricalBarsGateway {
    pub const fn new() -> Self {
        Self
    }

    /// R12 原始 15 分钟端点与留存回执。只陈述实际尾部覆盖；800 是
    /// 请求上限，不能证明完整研究窗口。使用独立 ExternalV1 HistoricalBars
    /// Tdx/Minute15 合同，不经过 LocalBridge TechnicalBars 或日线资格链。
    pub fn fifteen_min_bars(
        &self,
        code: &str,
        count: usize,
    ) -> Result<
        super::external_minute15_bars::ReceivedMinute15Bars,
        super::external_minute15_bars::Minute15Error,
    > {
        super::external_minute15_bars::receive_tail(code, count)
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
                return AdmittedDailyBars::from_audited_batch(code.to_owned(), days, audited);
            }
            Err(error) => {
                let audited = audit_routed_gateway_result(CAPABILITY, &request_hash, Err(error))?;
                return AdmittedDailyBars::from_audited_batch(code.to_owned(), days, audited);
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
                return AdmittedDailyBars::from_audited_batch(code, days, audited);
            }
            Err(error) => {
                let audited = audit_routed_gateway_result(CAPABILITY, &request_hash, Err(error))?;
                return AdmittedDailyBars::from_audited_batch(code, days, audited);
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
    } else if error.contains("suspension_evidence_unavailable_v1") {
        "suspension_evidence_unavailable_v1"
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
                    GatewayError::classified(
                        CAPABILITY,
                        Some(provider),
                        error.audit_outcome(),
                        error.reason_code(),
                        error.retryable(),
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
    let pending = crate::monitor::data_quality::validate_daily_kline_structure(&mut records, &code)
        .map_err(|error| final_admission_error(batch.evidence().provider, error))?;
    records.sort_by_key(|r| r.date);
    let ordered = GatewayBatch::Available {
        records,
        evidence: batch.evidence().clone(),
    };
    if pending.is_empty() {
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
    let actual_dates = batch
        .records()
        .iter()
        .map(|record| record.date)
        .collect::<Vec<_>>();
    if actual_dates != raw.expected_trading_dates() {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(batch.evidence().provider),
            "partial",
            "outcome_trading_date_vector_mismatch",
            false,
            format!(
                "outcome immutable provider sequence does not match its receipted trading-date vector: actual={actual_dates:?} expected={:?}",
                raw.expected_trading_dates()
            ),
        ));
    }
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

    fn observed_fixture(
        code: &str,
        days: usize,
        records: Vec<KlineData>,
        evidence: BatchEvidence,
    ) -> ObservedDailyBarsCapture {
        AdmittedDailyBars::from_test_fixture_with_days(code, days, records, evidence)
            .unwrap()
            .into_observed_projection()
            .unwrap()
    }

    #[test]
    fn observed_daily_projection_binds_actual_request_ordered_ohlcv_and_adjust() {
        let (batch, _) = super::super::outcome_daily_bars::task8_review_fixture();
        let records = batch.records().to_vec();
        let evidence = batch.evidence().clone();
        let baseline = observed_fixture("TEST_CODE_300005", 60, records.clone(), evidence.clone());
        let identical = observed_fixture("TEST_CODE_300005", 60, records.clone(), evidence.clone());
        assert_eq!(baseline.projection_hash(), identical.projection_hash());
        assert_eq!(baseline.target_code(), "TEST_CODE_300005");
        assert_eq!(baseline.requested_days(), 60);
        assert_eq!(baseline.bars().len(), 3);
        assert_eq!(baseline.bars()[0].date(), records[0].date);
        assert_eq!(baseline.bars()[0].ohlcv_amount()[3], records[0].close);
        assert_eq!(baseline.bars()[0].adjust(), records[0].adjust);

        let changed_days =
            observed_fixture("TEST_CODE_300005", 61, records.clone(), evidence.clone());
        assert_ne!(baseline.projection_hash(), changed_days.projection_hash());
        let changed_code =
            observed_fixture("TEST_CODE_300006", 60, records.clone(), evidence.clone());
        assert_ne!(baseline.projection_hash(), changed_code.projection_hash());
        let mut changed_close = records.clone();
        changed_close[0].close += 0.01;
        assert_ne!(
            baseline.projection_hash(),
            observed_fixture("TEST_CODE_300005", 60, changed_close, evidence.clone())
                .projection_hash()
        );
        let mut changed_adjust = records.clone();
        changed_adjust[0].adjust = AdjustType::Qfq;
        assert_ne!(
            baseline.projection_hash(),
            observed_fixture("TEST_CODE_300005", 60, changed_adjust, evidence.clone())
                .projection_hash()
        );
        let mut reversed = records.clone();
        reversed.reverse();
        assert_ne!(
            baseline.projection_hash(),
            observed_fixture("TEST_CODE_300005", 60, reversed, evidence.clone()).projection_hash()
        );

        // Derived KlineData fields are deliberately outside this projection.
        let mut changed_derived = records;
        changed_derived[0].pct_chg += 1.0;
        changed_derived[0].settled = !changed_derived[0].settled;
        changed_derived[0].intraday_price = Some(1.0);
        assert_eq!(
            baseline.projection_hash(),
            observed_fixture("TEST_CODE_300005", 60, changed_derived, evidence).projection_hash()
        );
    }

    #[test]
    fn observed_daily_projection_binds_all_batch_evidence_fields() {
        let (batch, _) = super::super::outcome_daily_bars::task8_review_fixture();
        let records = batch.records().to_vec();
        let evidence = batch.evidence().clone();
        let baseline = observed_fixture("TEST_CODE_300005", 60, records.clone(), evidence.clone());

        let mut variants = Vec::new();
        let mut changed = evidence.clone();
        changed.provider = ProviderId::Tencent;
        variants.push(changed);
        let mut changed = evidence.clone();
        changed.source.push_str("_changed");
        variants.push(changed);
        let mut changed = evidence.clone();
        changed.source_at = None;
        variants.push(changed);
        let mut changed = evidence.clone();
        changed.observed_at.push_str("_changed");
        variants.push(changed);
        let mut changed = evidence;
        changed.batch_id.push_str("_changed");
        variants.push(changed);

        for changed in variants {
            let capture = observed_fixture("TEST_CODE_300005", 60, records.clone(), changed);
            assert_ne!(baseline.projection_hash(), capture.projection_hash());
        }
    }

    #[test]
    fn observed_daily_projection_rejects_nonfinite_values_before_hashing() {
        let (batch, _) = super::super::outcome_daily_bars::task8_review_fixture();
        let evidence = batch.evidence().clone();
        let mut nan_close = batch.records().to_vec();
        nan_close[0].close = f64::NAN;
        let error = AdmittedDailyBars::from_test_fixture_with_days(
            "TEST_CODE_300005",
            60,
            nan_close,
            evidence.clone(),
        )
        .unwrap()
        .into_observed_projection()
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");

        let mut infinite_amount = batch.records().to_vec();
        infinite_amount[0].amount = f64::INFINITY;
        let error = AdmittedDailyBars::from_test_fixture_with_days(
            "TEST_CODE_300005",
            60,
            infinite_amount,
            evidence.clone(),
        )
        .unwrap()
        .into_observed_projection()
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");

        let mut positive_zero = batch.records().to_vec();
        positive_zero[0].volume = 0.0;
        let mut negative_zero = positive_zero.clone();
        negative_zero[0].volume = -0.0;
        assert_ne!(
            observed_fixture("TEST_CODE_300005", 60, positive_zero, evidence.clone())
                .projection_hash(),
            observed_fixture("TEST_CODE_300005", 60, negative_zero, evidence).projection_hash(),
        );
    }

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
