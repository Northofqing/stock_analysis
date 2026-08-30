//! BR-171/BR-174/BR-178 Magic TDX-only outcome daily-bar admission.
//!
//! The public type is an opaque capability, not a transport DTO. Its only
//! production constructor consumes a receipt-verified [`VerifiedOutcomeDue`],
//! performs an evidence-preserving adaptive Magic TDX latest-N search, and
//! retains the exact semantic request, every transport result, admitted-window
//! and provider evidence preimage. No router or fallback provider participates
//! in this path.

use crate::market_domain::{
    Adjustment, AssetClass, Bar, BarInterval, DataBatch, InstrumentId, ProviderId,
};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::data_provider::{AdjustType, KlineData};
use crate::database::selection_v2_read_model::VerifiedOutcomeDue;
use crate::monitor::data_quality::{validate_daily_freshness, DqStats, FreshnessConfig};
use crate::selection::outcome_session_gate::validate_shanghai_tick_instant;
use crate::selection::schema_v2::{
    build_request_evidence, canonical_f64, canonical_json, sha256_bytes, sha256_json,
    AdjustmentKind, DailyIntervalKind, OutcomeMarketRequestParametersPreimage, OutcomePhase,
    OutcomeProviderAvailableEvidencePreimage, OutcomeProviderRequestPreimage,
    OutcomeTradingDateVectorPreimage, OutcomeTransportAttemptPreimage,
    OutcomeTransportAttemptsPreimage, OutcomeTransportBarFingerprint,
    OutcomeTransportBatchContentPreimage, ProviderAvailableEvidencePreimage,
    ProviderCapabilityHashPreimage, ProviderEvidenceKind, RequestEvidenceColumns, RequestKind,
    RequestParametersPreimage, AMENDMENT_DESIGN_SHA256, DOMAIN_OUTCOME_MARKET_REQUEST,
    DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE, DOMAIN_OUTCOME_PROVIDER_REQUEST,
    DOMAIN_OUTCOME_TRANSPORT_ATTEMPTS, DOMAIN_PROVIDER_AVAILABLE_EVIDENCE,
    DOMAIN_PROVIDER_CAPABILITY, OUTCOME_ADAPTIVE_POLICY_VERSION, OUTCOME_PARENT_DESIGN_SHA256,
    OUTCOME_TDX_HISTORICAL_PAGE_SIZE, UPSTREAM_REVISION,
};
#[cfg(test)]
use crate::selection::schema_v2::{
    OutcomeProviderErrorPreimage, OutcomeTransportEvidencePreimage,
    OutcomeTransportRequestPreimage, OutcomeTransportResultPreimage,
};

use super::historical_bars::{finalize_outcome_sequence_async, OutcomeLifecycleAdmission};
use super::instrument_identity::{resolve_production_equity, EquitySegment};
use super::review::{audit_gateway_result, BatchEvidence, GatewayBatch, GatewayError};

const CAPABILITY: &str = "OutcomeDailyBarsV2";
const PROVIDER: &str = "magic-tdx";
const PROVIDER_SOURCE: &str = "tdx-smart";
const PROVIDER_CAPABILITY_NAME: &str = "MagicTdx-UnadjustedDailyBars";
const PROVIDER_CONTRACT_VERSION: &str = "magic-market-core.MarketDataProvider.bars.v0.2.0";
const DESIGN_SHA256: &str = OUTCOME_PARENT_DESIGN_SHA256;
const DOMAIN_PROVIDER_BAR: &str = "stock_analysis.br174.outcome_daily_bar.v1";
const DOMAIN_PROVIDER_RESPONSE: &str =
    "stock_analysis.br174.outcome_daily_bars_provider_response.v1";
const DOMAIN_PROVIDER_ORDERED_CONTENT: &str =
    "stock_analysis.br174.outcome_daily_bars_ordered_content.v1";
const DOMAIN_ADMITTED_WINDOW: &str = "stock_analysis.br174.outcome_daily_bars_window.v1";
const DOMAIN_LIFECYCLE_EVIDENCE: &str = "stock_analysis.br174.outcome_daily_bars_lifecycle.v1";
// magic-market-core exposes TDX daily-bar quantity in board lots
// (`SecurityBar.vol / 100`). Selection-v2 T0 receipts predate that adapter and
// persist protocol volume in shares. Retain both values in the typed evidence
// and expose shares to outcome math so D1/T0 ratios use one unit.
const SHARES_PER_BOARD_LOT: f64 = 100.0;
const VOLUME_CONVERSION_CONTRACT: &str =
    "magic-market-data-rs@75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e:BR-022+BR-036";
const VOLUME_CONTRACT_UPSTREAM_REVISION: &str = "75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e";
const VOLUME_CONVERSION_VERSION: &str = "outcome-volume-shares-v1";
const CORE_VOLUME_UNIT: &str = "board_lot";
const ADMITTED_VOLUME_UNIT: &str = "share";
const AMOUNT_UNIT: &str = "CNY_yuan";
const A_SHARE_POST_CLOSE_HOUR: u32 = 15;

/// Magic TDX-only outcome daily-bar Gateway.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutcomeDailyBarsGateway;

/// Opaque, one-shot failure capability for an outcome acquisition.
///
/// The Gateway is the only production constructor. Once provider access has
/// been attempted, `request_evidence` is always the exact canonical semantic
/// request used for that attempt. `available_evidence`, when present, binds
/// only provider records that were actually returned and validated far enough
/// to be retained. The settlement owner must consume this value by ownership;
/// callers cannot manufacture or mutate its fields.
#[derive(Debug)]
pub struct OutcomeAcquisitionFailure {
    request_evidence: Option<RequestEvidenceColumns>,
    error: GatewayError,
    available_evidence: Option<OutcomeProviderAvailableEvidencePreimage>,
    transport_attempts: Option<OutcomeTransportAttemptsPreimage>,
}

impl OutcomeAcquisitionFailure {
    fn before_provider(error: GatewayError) -> Self {
        Self {
            request_evidence: None,
            error,
            available_evidence: None,
            transport_attempts: None,
        }
    }

    fn after_provider(
        plan: &OutcomeAcquisitionPlan,
        error: GatewayError,
        available_evidence: Option<OutcomeProviderAvailableEvidencePreimage>,
        transport_attempts: Vec<OutcomeTransportAttemptPreimage>,
    ) -> Self {
        let selected_transport_result_hash = available_evidence
            .as_ref()
            .and_then(|_| latest_successful_transport_result_hash(&transport_attempts));
        Self {
            request_evidence: Some(plan.request_evidence.clone()),
            error,
            available_evidence,
            transport_attempts: Some(outcome_transport_attempts_preimage(
                plan,
                transport_attempts,
                selected_transport_result_hash,
            )),
        }
    }

    fn after_transport_failure(
        plan: &OutcomeAcquisitionPlan,
        failure: OutcomeTransportFailure,
    ) -> Self {
        Self::after_provider(plan, failure.error, None, failure.attempts)
    }

    /// Canonical transport-attempt evidence for the outcome-attempt
    /// persistence owner. This is deliberately separate from
    /// `available_evidence`: a failed paginated request can retain complete
    /// evidence from earlier successful requests without turning those records
    /// into an admitted `DataBatch`.
    ///
    /// `outcome_v2` will consume this pair in its persistence slice; until that
    /// wiring lands, the opaque failure capability still owns every attempt.
    #[cfg(test)]
    pub(crate) fn transport_attempts_canonical_pair(
        &self,
    ) -> Result<Option<(String, String)>, GatewayError> {
        match (
            self.request_evidence.as_ref(),
            self.transport_attempts.as_ref(),
        ) {
            (Some(request_columns), Some(preimage)) => {
                let request = request_columns
                    .validate(Some(RequestKind::OutcomeMarketEvidence))
                    .map_err(schema_gateway_error)?;
                let parameters = decode_canonical_pair::<OutcomeMarketRequestParametersPreimage>(
                    &request.parameters_json,
                    &request.parameters_json_hash,
                    "outcome failure request parameters",
                )?;
                let capability = decode_canonical_pair::<ProviderCapabilityHashPreimage>(
                    &request.provider_capability_json,
                    &request.provider_capability_hash,
                    "outcome failure provider capability",
                )?;
                preimage
                    .validate(
                        &request_columns.request_hash,
                        &request_columns.request_evidence_hash,
                        &request,
                        &parameters,
                        &capability,
                    )
                    .map_err(schema_gateway_error)?;
                Ok(Some((
                    canonical_json(preimage).map_err(schema_gateway_error)?,
                    sha256_json(preimage).map_err(schema_gateway_error)?,
                )))
            }
            (None, None) => Ok(None),
            _ => Err(invalid_evidence(
                "outcome failure request and transport-attempt pair must have equal presence",
            )),
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        Option<RequestEvidenceColumns>,
        GatewayError,
        Option<OutcomeProviderAvailableEvidencePreimage>,
        Option<OutcomeTransportAttemptsPreimage>,
    ) {
        (
            self.request_evidence,
            self.error,
            self.available_evidence,
            self.transport_attempts,
        )
    }

    #[cfg(test)]
    pub(crate) fn test_only_after_provider(
        request_evidence: RequestEvidenceColumns,
        reason_code: &'static str,
        retryable: bool,
        available_evidence: Option<OutcomeProviderAvailableEvidencePreimage>,
    ) -> Self {
        let transport_attempts =
            test_only_transport_attempts(&request_evidence, available_evidence.as_ref());
        Self {
            request_evidence: Some(request_evidence),
            error: GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Tdx),
                "partial",
                reason_code,
                retryable,
                "TEST_CODE typed acquisition failure",
            ),
            available_evidence,
            transport_attempts: Some(transport_attempts),
        }
    }
}

/// One admitted raw, unadjusted daily bar.
///
/// Fields stay private so callers cannot alter a bar independently of the
/// capability and evidence that admitted it.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeDailyBar {
    market_date: NaiveDate,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    amount: f64,
}

impl OutcomeDailyBar {
    pub(crate) fn market_date(&self) -> NaiveDate {
        self.market_date
    }

    pub(crate) fn open(&self) -> f64 {
        self.open
    }

    pub(crate) fn high(&self) -> f64 {
        self.high
    }

    pub(crate) fn low(&self) -> f64 {
        self.low
    }

    pub(crate) fn close(&self) -> f64 {
        self.close
    }

    pub(crate) fn volume(&self) -> f64 {
        self.volume
    }

    pub(crate) fn amount(&self) -> f64 {
        self.amount
    }
}

/// Non-forgeable proof that a complete, exact outcome window passed all
/// provider, structural, freshness, lifecycle and BR-171 gates.
///
/// This type intentionally implements neither `Clone`, `Default` nor serde.
#[derive(Debug)]
pub struct AdmittedOutcomeDailyBars {
    sample_key: String,
    canonical_stock_code: String,
    canonical_market: String,
    phase: OutcomePhase,
    stored_due_date: NaiveDate,
    window_dates: Vec<NaiveDate>,
    trading_date_vector: OutcomeTradingDateVectorPreimage,
    trading_date_vector_hash: String,
    verified_due_binding_hash: String,
    bars: Vec<OutcomeDailyBar>,
    provider_ordered_content_json: String,
    provider_ordered_content_hash: String,
    request_evidence: RequestEvidenceColumns,
    available_evidence: OutcomeProviderAvailableEvidencePreimage,
    provider_request_json: String,
    provider_request_hash: String,
    provider_response_json: String,
    provider_response_hash: String,
    lifecycle_evidence_json: String,
    lifecycle_evidence_hash: String,
    admitted_window_json: String,
    admitted_window_hash: String,
    transport_attempts: OutcomeTransportAttemptsPreimage,
}

impl AdmittedOutcomeDailyBars {
    pub(crate) fn sample_key(&self) -> &str {
        &self.sample_key
    }

    pub(crate) fn canonical_stock_code(&self) -> &str {
        &self.canonical_stock_code
    }

    pub(crate) fn canonical_market(&self) -> &str {
        &self.canonical_market
    }

    pub(crate) fn phase(&self) -> OutcomePhase {
        self.phase
    }

    pub(crate) fn stored_due_date(&self) -> NaiveDate {
        self.stored_due_date
    }

    pub(crate) fn window_dates(&self) -> &[NaiveDate] {
        &self.window_dates
    }

    pub(crate) fn trading_date_vector(&self) -> &OutcomeTradingDateVectorPreimage {
        &self.trading_date_vector
    }

    pub(crate) fn trading_date_vector_hash(&self) -> &str {
        &self.trading_date_vector_hash
    }

    pub(crate) fn verified_due_binding_hash(&self) -> &str {
        &self.verified_due_binding_hash
    }

    pub(crate) fn bars(&self) -> &[OutcomeDailyBar] {
        &self.bars
    }

    pub(crate) fn request_evidence(&self) -> &RequestEvidenceColumns {
        &self.request_evidence
    }

    pub(crate) fn available_evidence(&self) -> &OutcomeProviderAvailableEvidencePreimage {
        &self.available_evidence
    }

    pub(crate) fn transport_attempts(&self) -> &OutcomeTransportAttemptsPreimage {
        &self.transport_attempts
    }

    pub(crate) fn validate_strict(&self) -> Result<(), GatewayError> {
        validate_admitted_evidence(self)
    }
}

#[derive(Debug, Clone)]
struct OutcomeAcquisitionPlan {
    sample_key: String,
    canonical_stock_code: String,
    canonical_market: String,
    phase: OutcomePhase,
    stored_due_date: NaiveDate,
    window_start: NaiveDate,
    window_end: NaiveDate,
    expected_bar_count: u16,
    calendar_hash: String,
    trading_date_vector: OutcomeTradingDateVectorPreimage,
    trading_date_vector_hash: String,
    applicable_trading_dates: Vec<NaiveDate>,
    receipted_t0_close: Option<String>,
    receipted_t0_volume_shares: Option<String>,
    verified_due_binding_hash: String,
    requested_at: DateTime<FixedOffset>,
    request_local_date: NaiveDate,
    request_evidence: RequestEvidenceColumns,
    provider_capability_hash: String,
    request_parameters_hash: String,
    provider_request: OutcomeProviderRequestPreimage,
    provider_request_json: String,
    provider_request_hash: String,
    instrument: InstrumentId,
    maximum_latest_n: u16,
}

#[derive(Debug)]
pub struct RawOutcomeFetch {
    pub batch: DataBatch<Bar>,
    pub attempts: Vec<OutcomeTransportAttemptPreimage>,
}

#[derive(Debug)]
pub struct OutcomeTransportFailure {
    pub error: GatewayError,
    pub attempts: Vec<OutcomeTransportAttemptPreimage>,
}

impl OutcomeTransportFailure {
    pub(crate) fn new(error: GatewayError, attempts: Vec<OutcomeTransportAttemptPreimage>) -> Self {
        Self { error, attempts }
    }
}

#[derive(Debug)]
struct StructurallyAdmittedBatch {
    bars: Vec<OutcomeDailyBar>,
    validation_records: Vec<KlineData>,
    evidence: BatchEvidence,
    provider_ordered_content_json: String,
    provider_ordered_content_hash: String,
    available_evidence: OutcomeProviderAvailableEvidencePreimage,
    provider_response_json: String,
    provider_response_hash: String,
    window_preimages: Vec<OutcomeProviderBarPreimage>,
    transport_attempts: Vec<OutcomeTransportAttemptPreimage>,
}

#[derive(Debug)]
struct OutcomeProjectionFailure {
    error: GatewayError,
    available_evidence: Option<Box<OutcomeProviderAvailableEvidencePreimage>>,
    transport_attempts: Vec<OutcomeTransportAttemptPreimage>,
}

impl OutcomeProjectionFailure {
    fn with_available_evidence(
        error: GatewayError,
        available_evidence: OutcomeProviderAvailableEvidencePreimage,
    ) -> Self {
        Self {
            error,
            available_evidence: Some(Box::new(available_evidence)),
            transport_attempts: Vec::new(),
        }
    }
}

impl From<GatewayError> for OutcomeProjectionFailure {
    fn from(error: GatewayError) -> Self {
        Self {
            error,
            available_evidence: None,
            transport_attempts: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct OutcomeWindowSelectionFailure {
    error: GatewayError,
    partial_preimages: Vec<OutcomeProviderBarPreimage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeProviderBarPreimage {
    domain: String,
    provider_ordinal: u32,
    canonical_stock_code: String,
    canonical_market: String,
    market_date: String,
    bar_start: String,
    bar_end: String,
    open: String,
    high: String,
    low: String,
    close: String,
    volume_conversion_contract: String,
    volume_conversion_version: String,
    core_volume_unit: String,
    shares_per_board_lot: String,
    core_volume_lots: String,
    admitted_volume_unit: String,
    admitted_volume_shares: String,
    amount_unit: String,
    amount: String,
    interval: String,
    adjustment: String,
    source_at: String,
    provider: String,
    batch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeProviderResponsePreimage {
    domain: String,
    provider_request_hash: String,
    provider: String,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
    record_count: u32,
    trading_date_vector_hash: String,
    expected_trading_dates: Vec<String>,
    returned_trading_dates: Vec<String>,
    selected_transport_result_hash: String,
    transport_attempts_in_request_order: Vec<OutcomeTransportAttemptPreimage>,
    provider_ordered_records: Vec<OutcomeProviderBarPreimage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeProviderOrderedContentPreimage {
    domain: String,
    provider: String,
    source: String,
    canonical_stock_code: String,
    canonical_market: String,
    interval: String,
    adjustment: String,
    provider_ordered_records: Vec<OutcomeProviderBarPreimage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeAdmittedWindowPreimage {
    domain: String,
    design_sha256: String,
    semantic_request_hash: String,
    verified_due_binding_hash: String,
    sample_key: String,
    canonical_stock_code: String,
    canonical_market: String,
    phase: OutcomePhase,
    stored_due_date: String,
    calendar_hash: String,
    trading_date_vector: OutcomeTradingDateVectorPreimage,
    trading_date_vector_hash: String,
    expected_trading_dates: Vec<String>,
    returned_trading_dates: Vec<String>,
    receipted_t0_close: Option<String>,
    receipted_t0_volume_shares: Option<String>,
    provider_response_hash: String,
    lifecycle_evidence_hash: String,
    provider_ordered_records: Vec<OutcomeProviderBarPreimage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeLifecycleEvidencePreimage {
    domain: String,
    design_sha256: String,
    sample_key: String,
    canonical_stock_code: String,
    canonical_market: String,
    phase: OutcomePhase,
    window_start: String,
    window_end: String,
    listing_date: Option<String>,
    listing_batch_id: Option<String>,
    listing_unavailable_reason_code: Option<String>,
    listing_unavailable_retryable: Option<bool>,
    corporate_action_state: String,
    corporate_action_batch_id: String,
    adjacent_evidence: Vec<OutcomeLifecycleAdjacentPreimage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeLifecycleAdjacentPreimage {
    provider: String,
    batch_identity: String,
    listing_date: Option<String>,
    corporate_action_identity: Option<String>,
}

impl OutcomeDailyBarsGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Freeze the complete provider transport request into the due capability
    /// before the claim is persisted. This step is pure: it performs no
    /// provider I/O, and recovery reuses the resulting canonical preimage
    /// rather than deriving a different request from a later scheduler tick.
    pub(crate) fn bind_claim_request(
        &self,
        due: VerifiedOutcomeDue,
        claimed_at: DateTime<FixedOffset>,
    ) -> Result<VerifiedOutcomeDue, GatewayError> {
        let plan = OutcomeAcquisitionPlan::from_unbound_due(&due, claimed_at)?;
        due.bind_provider_transport_request(plan.provider_request, plan.provider_request_hash)
            .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))
    }

    /// Acquire and admit the exact daily window for one receipt-verified due
    /// phase. `attempted_at` is the checked +08:00 observation instant; it may
    /// advance across recovery ticks, but it cannot alter the provider request
    /// already persisted in the claim. There is deliberately no overload
    /// accepting raw identifiers or a caller-built semantic request.
    pub(crate) async fn acquire(
        &self,
        due: &VerifiedOutcomeDue,
        attempted_at: DateTime<FixedOffset>,
    ) -> Result<AdmittedOutcomeDailyBars, OutcomeAcquisitionFailure> {
        let plan = OutcomeAcquisitionPlan::from_claim_bound_due(due, attempted_at)
            .map_err(OutcomeAcquisitionFailure::before_provider)?;
        let instrument = plan.instrument.clone();
        let wire_code = plan.canonical_stock_code.clone();
        let wire_market = plan.canonical_market.clone();
        let expected_bar_count = plan.expected_bar_count;
        let maximum_latest_n = plan.maximum_latest_n;
        let window_start = plan.window_start;
        let fetched = tokio::task::spawn_blocking(move || {
            // P4 M3 transport seam: bridge 可用时 (remote gRPC) 走 gRPC
            // 通道 — 服务端执行同一 adaptive transport, 客户端 convert 重建
            // RawOutcomeFetch / OutcomeTransportFailure (error+attempts 保真)。
            // claim 台账与 audit (下方 audit_gateway_result) 始终留客户端。
            match super::grpc_source::bridge_for("OutcomeDailyBars") {
                Ok(bridge) => bridge.outcome_daily_bars_adaptive(
                    instrument.clone(),
                    wire_market.clone(),
                    wire_code.clone(),
                    expected_bar_count,
                    maximum_latest_n,
                    window_start,
                ),
                Err(error) => Err(OutcomeTransportFailure::new(
                    GatewayError::unavailable(
                        CAPABILITY,
                        Some(ProviderId::Tdx),
                        true,
                        format!("outcome_daily_bars bridge unavailable: {error}"),
                    ),
                    Vec::new(),
                )),
            }
        })
        .await
        .map_err(|error| {
            OutcomeAcquisitionFailure::before_provider(GatewayError::unavailable(
                CAPABILITY,
                Some(ProviderId::Tdx),
                true,
                format!("Magic TDX outcome-bars worker failed before typed receipt: {error}"),
            ))
        })?
        .map_err(|failure| OutcomeAcquisitionFailure::after_transport_failure(&plan, failure))?;

        let mut projected =
            project_magic_tdx_batch(&plan, fetched.batch, fetched.attempts, plan.requested_at)
                .map_err(|failure| {
                    OutcomeAcquisitionFailure::after_provider(
                        &plan,
                        failure.error,
                        failure.available_evidence.map(|evidence| *evidence),
                        failure.transport_attempts,
                    )
                })?;
        let quality_batch = GatewayBatch::Available {
            records: projected.validation_records.clone(),
            evidence: projected.evidence.clone(),
        };
        let (_, lifecycle_admission) =
            finalize_outcome_sequence_async(plan.canonical_stock_code.clone(), quality_batch)
                .await
                .map_err(map_final_admission_error)
                .map_err(|error| {
                    OutcomeAcquisitionFailure::after_provider(
                        &plan,
                        error,
                        Some(projected.available_evidence.clone()),
                        projected.transport_attempts.clone(),
                    )
                })?;
        let (
            lifecycle_evidence_json,
            lifecycle_evidence_hash,
            admitted_window_json,
            admitted_window_hash,
        ) = bind_lifecycle_and_window(&plan, &projected, lifecycle_admission).map_err(|error| {
            OutcomeAcquisitionFailure::after_provider(
                &plan,
                error,
                Some(projected.available_evidence.clone()),
                projected.transport_attempts.clone(),
            )
        })?;

        let audited = tokio::task::spawn_blocking({
            let request_hash = plan.request_evidence.request_hash.clone();
            let evidence = projected.evidence.clone();
            let records = std::mem::take(&mut projected.bars);
            move || {
                audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Tdx,
                    &request_hash,
                    Ok(GatewayBatch::Available { records, evidence }),
                )
            }
        })
        .await
        .map_err(|error| {
            OutcomeAcquisitionFailure::after_provider(
                &plan,
                GatewayError::unavailable(
                    CAPABILITY,
                    Some(ProviderId::Tdx),
                    true,
                    format!("outcome-bars audit worker failed: {error}"),
                ),
                Some(projected.available_evidence.clone()),
                projected.transport_attempts.clone(),
            )
        })?
        .map_err(|error| {
            OutcomeAcquisitionFailure::after_provider(
                &plan,
                error,
                Some(projected.available_evidence.clone()),
                projected.transport_attempts.clone(),
            )
        })?;
        let bars = match audited {
            GatewayBatch::Available { records, .. } => records,
            GatewayBatch::VerifiedEmpty(_) => {
                return Err(OutcomeAcquisitionFailure::after_provider(
                    &plan,
                    GatewayError::unavailable(
                        CAPABILITY,
                        Some(ProviderId::Tdx),
                        true,
                        "Magic TDX outcome window cannot be verified empty",
                    ),
                    Some(projected.available_evidence.clone()),
                    projected.transport_attempts.clone(),
                ))
            }
        };
        let window_dates = bars.iter().map(OutcomeDailyBar::market_date).collect();
        let admitted_transport_attempts = projected.transport_attempts.clone();
        let transport_attempts = outcome_transport_attempts_preimage(
            &plan,
            projected.transport_attempts.clone(),
            latest_successful_transport_result_hash(&projected.transport_attempts),
        );

        let admitted = AdmittedOutcomeDailyBars {
            sample_key: plan.sample_key.clone(),
            canonical_stock_code: plan.canonical_stock_code.clone(),
            canonical_market: plan.canonical_market.clone(),
            phase: plan.phase,
            stored_due_date: plan.stored_due_date,
            window_dates,
            trading_date_vector: plan.trading_date_vector.clone(),
            trading_date_vector_hash: plan.trading_date_vector_hash.clone(),
            verified_due_binding_hash: plan.verified_due_binding_hash.clone(),
            bars,
            provider_ordered_content_json: projected.provider_ordered_content_json,
            provider_ordered_content_hash: projected.provider_ordered_content_hash,
            request_evidence: plan.request_evidence.clone(),
            available_evidence: projected.available_evidence,
            provider_request_json: plan.provider_request_json.clone(),
            provider_request_hash: plan.provider_request_hash.clone(),
            provider_response_json: projected.provider_response_json,
            provider_response_hash: projected.provider_response_hash,
            lifecycle_evidence_json,
            lifecycle_evidence_hash,
            admitted_window_json,
            admitted_window_hash,
            transport_attempts,
        };
        admitted.validate_strict().map_err(|error| {
            OutcomeAcquisitionFailure::after_provider(
                &plan,
                error,
                Some(admitted.available_evidence.clone()),
                admitted_transport_attempts,
            )
        })?;
        Ok(admitted)
    }
}

impl OutcomeAcquisitionPlan {
    fn from_claim_bound_due(
        due: &VerifiedOutcomeDue,
        attempted_at: DateTime<FixedOffset>,
    ) -> Result<Self, GatewayError> {
        validate_shanghai_tick_instant(&attempted_at)
            .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
        validate_unit_contract()?;
        validate_window_contract(
            due.phase(),
            due.stored_due_date(),
            due.applicable_trading_dates(),
        )?;

        let semantic_request = due.provider_request_evidence().clone();
        semantic_request
            .validate(Some(RequestKind::OutcomeMarketEvidence))
            .map_err(schema_gateway_error)?;
        let request_parameters: OutcomeMarketRequestParametersPreimage =
            serde_json::from_str(&semantic_request.parameters_json).map_err(|error| {
                GatewayError::invalid_request(
                    CAPABILITY,
                    format!("claim-bound outcome request parameters are invalid: {error}"),
                )
            })?;
        let (provider_request, persisted_provider_request_hash) = due
            .provider_transport_request()
            .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
        provider_request
            .validate(&semantic_request, &request_parameters)
            .map_err(schema_gateway_error)?;
        let provider_request_hash = sha256_json(provider_request).map_err(schema_gateway_error)?;
        if provider_request_hash != persisted_provider_request_hash
            || provider_request.semantic_request_hash != due.provider_request_hash()
            || provider_request.verified_due_binding_hash != due.request_binding_hash()
        {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "claim-bound provider request hash/identity changed before provider I/O",
            ));
        }

        let baseline_required = due.phase() != OutcomePhase::T0Close;
        let receipted_t0_close =
            canonical_receipted_metric("t0_close", due.t0_close(), baseline_required)?;
        let receipted_t0_volume_shares =
            canonical_receipted_metric("t0_volume", due.t0_volume(), baseline_required)?;
        if provider_request.receipted_t0_close != receipted_t0_close
            || provider_request.receipted_t0_volume_shares != receipted_t0_volume_shares
        {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "claim-bound provider request T0 baseline differs from the verified due",
            ));
        }

        let parse_date = |value: &str, field: &str| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
                GatewayError::invalid_request(
                    CAPABILITY,
                    format!("claim-bound {field} is invalid: {error}"),
                )
            })
        };
        let window_start = parse_date(&provider_request.window_start, "window_start")?;
        let window_end = parse_date(&provider_request.window_end, "window_end")?;
        let request_local_date =
            parse_date(&provider_request.request_local_date, "request_local_date")?;
        if window_start != due.window_start()
            || window_end != due.window_end()
            || attempted_at.date_naive() < request_local_date
        {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "claim-bound provider request date window is not valid for this attempt",
            ));
        }

        let request_evidence_json =
            canonical_json(&semantic_request).map_err(schema_gateway_error)?;
        let request_evidence = RequestEvidenceColumns {
            request_hash: semantic_request.request_hash.clone(),
            request_evidence_hash: sha256_bytes(request_evidence_json.as_bytes()),
            request_evidence_json,
        };
        request_evidence
            .validate(Some(RequestKind::OutcomeMarketEvidence))
            .map_err(schema_gateway_error)?;
        let provider_request_json =
            canonical_json(provider_request).map_err(schema_gateway_error)?;
        let applicable_trading_dates = due.applicable_trading_dates().to_vec();
        let canonical_stock_code = due.canonical_stock_code().to_owned();
        let canonical_market = due.canonical_market().to_owned();
        let instrument = outcome_instrument(&canonical_stock_code, &canonical_market)?;

        Ok(Self {
            sample_key: due.sample_key().to_owned(),
            canonical_stock_code,
            canonical_market,
            phase: due.phase(),
            stored_due_date: due.stored_due_date(),
            window_start,
            window_end,
            expected_bar_count: provider_request.expected_bar_count,
            calendar_hash: due.calendar_hash().to_owned(),
            trading_date_vector: due.trading_date_vector().clone(),
            trading_date_vector_hash: due.trading_date_vector_hash().to_owned(),
            applicable_trading_dates,
            receipted_t0_close,
            receipted_t0_volume_shares,
            verified_due_binding_hash: due.request_binding_hash().to_owned(),
            requested_at: attempted_at,
            request_local_date,
            request_evidence,
            provider_capability_hash: semantic_request.provider_capability_hash.clone(),
            request_parameters_hash: semantic_request.parameters_json_hash.clone(),
            provider_request: provider_request.clone(),
            provider_request_json,
            provider_request_hash,
            instrument,
            maximum_latest_n: provider_request.maximum_latest_n,
        })
    }

    fn from_unbound_due(
        due: &VerifiedOutcomeDue,
        requested_at: DateTime<FixedOffset>,
    ) -> Result<Self, GatewayError> {
        validate_shanghai_tick_instant(&requested_at)
            .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
        validate_unit_contract()?;
        validate_window_contract(
            due.phase(),
            due.stored_due_date(),
            due.applicable_trading_dates(),
        )?;
        let baseline_required = due.phase() != OutcomePhase::T0Close;
        let receipted_t0_close =
            canonical_receipted_metric("t0_close", due.t0_close(), baseline_required)?;
        let receipted_t0_volume_shares =
            canonical_receipted_metric("t0_volume", due.t0_volume(), baseline_required)?;
        let request_local_date = requested_at.date_naive();
        let applicable_trading_dates = due.applicable_trading_dates().to_vec();
        let expected_trading_dates = applicable_trading_dates
            .iter()
            .map(|date| date.format("%Y-%m-%d").to_string())
            .collect::<Vec<_>>();

        let canonical_stock_code = due.canonical_stock_code().to_owned();
        let canonical_market = due.canonical_market().to_owned();
        let instrument = outcome_instrument(&canonical_stock_code, &canonical_market)?;
        let natural_day_upper_bound = request_local_date
            .signed_duration_since(due.window_start())
            .num_days()
            .checked_add(1)
            .ok_or_else(|| {
                GatewayError::invalid_request(CAPABILITY, "outcome latest-N range overflow")
            })?;
        let maximum_latest_n = u16::try_from(natural_day_upper_bound).map_err(|_| {
            GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Tdx),
                "invalid_request",
                "unsupported_window",
                false,
                format!(
                    "outcome recovery window {}..{} exceeds Magic TDX u16 offset range",
                    due.window_start(),
                    request_local_date
                ),
            )
        })?;
        if maximum_latest_n < due.expected_bar_count() {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                format!(
                    "outcome recovery upper bound {maximum_latest_n} is below expected count {}",
                    due.expected_bar_count()
                ),
            ));
        }

        let request_parameters = OutcomeMarketRequestParametersPreimage {
            domain: DOMAIN_OUTCOME_MARKET_REQUEST.into(),
            sample_key: due.sample_key().to_owned(),
            canonical_stock_code: canonical_stock_code.clone(),
            canonical_market: canonical_market.clone(),
            phase: due.phase(),
            stored_due_date: due.stored_due_date().format("%Y-%m-%d").to_string(),
            calendar_version: due.calendar_version().to_owned(),
            calendar_hash: due.calendar_hash().to_owned(),
            trading_date_vector: due.trading_date_vector().clone(),
            trading_date_vector_hash: due.trading_date_vector_hash().to_owned(),
            applicable_trading_dates: expected_trading_dates.clone(),
            window_start: due.window_start().format("%Y-%m-%d").to_string(),
            window_end: due.window_end().format("%Y-%m-%d").to_string(),
            interval: DailyIntervalKind::Day,
            adjustment: AdjustmentKind::None,
        };
        let provider_capability = ProviderCapabilityHashPreimage {
            domain: DOMAIN_PROVIDER_CAPABILITY.into(),
            provider: PROVIDER.into(),
            capability_name: PROVIDER_CAPABILITY_NAME.into(),
            contract_version: PROVIDER_CONTRACT_VERSION.into(),
            upstream_revision: UPSTREAM_REVISION.into(),
        };
        let request_parameters_hash =
            sha256_json(&request_parameters).map_err(schema_gateway_error)?;
        let provider_capability_hash =
            sha256_json(&provider_capability).map_err(schema_gateway_error)?;
        let request_evidence = build_request_evidence(
            RequestParametersPreimage::OutcomeMarketEvidence(request_parameters.clone()),
            provider_capability,
        )
        .map_err(schema_gateway_error)?;
        if request_evidence.request_hash != due.provider_request_hash() {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "claim-time semantic provider request differs from the verified due request",
            ));
        }

        let provider_request = OutcomeProviderRequestPreimage {
            domain: DOMAIN_OUTCOME_PROVIDER_REQUEST.into(),
            design_sha256: DESIGN_SHA256.into(),
            amendment_design_sha256: AMENDMENT_DESIGN_SHA256.into(),
            semantic_request_hash: request_evidence.request_hash.clone(),
            verified_due_binding_hash: due.request_binding_hash().to_owned(),
            sample_key: due.sample_key().to_owned(),
            canonical_stock_code: canonical_stock_code.clone(),
            canonical_market: canonical_market.clone(),
            phase: due.phase(),
            stored_due_date: due.stored_due_date().format("%Y-%m-%d").to_string(),
            window_start: due.window_start().format("%Y-%m-%d").to_string(),
            window_end: due.window_end().format("%Y-%m-%d").to_string(),
            expected_bar_count: due.expected_bar_count(),
            calendar_version: due.calendar_version().to_owned(),
            calendar_hash: due.calendar_hash().to_owned(),
            trading_date_vector: due.trading_date_vector().clone(),
            trading_date_vector_hash: due.trading_date_vector_hash().to_owned(),
            expected_trading_dates: expected_trading_dates.clone(),
            receipted_t0_close: receipted_t0_close.clone(),
            receipted_t0_volume_shares: receipted_t0_volume_shares.clone(),
            request_local_date: request_local_date.format("%Y-%m-%d").to_string(),
            post_close_cutoff: format!("{A_SHARE_POST_CLOSE_HOUR:02}:00:00"),
            interval: DailyIntervalKind::Day.as_str().into(),
            adjustment: AdjustmentKind::None.as_str().into(),
            acquisition_strategy:
                "phase-minimum_then_exponential_growth_then_cardinality_bisection".into(),
            adaptive_policy_version: OUTCOME_ADAPTIVE_POLICY_VERSION.into(),
            maximum_latest_n,
            volume_conversion_contract: VOLUME_CONVERSION_CONTRACT.into(),
            volume_conversion_version: VOLUME_CONVERSION_VERSION.into(),
            shares_per_board_lot: canonical_f64(SHARES_PER_BOARD_LOT)
                .map_err(schema_gateway_error)?,
        };
        provider_request
            .validate(
                &request_evidence
                    .validate(Some(RequestKind::OutcomeMarketEvidence))
                    .map_err(schema_gateway_error)?,
                &request_parameters,
            )
            .map_err(schema_gateway_error)?;
        let provider_request_json =
            canonical_json(&provider_request).map_err(schema_gateway_error)?;
        let provider_request_hash = sha256_json(&provider_request).map_err(schema_gateway_error)?;

        Ok(Self {
            sample_key: due.sample_key().to_owned(),
            canonical_stock_code,
            canonical_market,
            phase: due.phase(),
            stored_due_date: due.stored_due_date(),
            window_start: due.window_start(),
            window_end: due.window_end(),
            expected_bar_count: due.expected_bar_count(),
            calendar_hash: due.calendar_hash().to_owned(),
            trading_date_vector: due.trading_date_vector().clone(),
            trading_date_vector_hash: due.trading_date_vector_hash().to_owned(),
            applicable_trading_dates,
            receipted_t0_close,
            receipted_t0_volume_shares,
            verified_due_binding_hash: due.request_binding_hash().to_owned(),
            requested_at,
            request_local_date,
            request_evidence,
            provider_capability_hash,
            request_parameters_hash,
            provider_request,
            provider_request_json,
            provider_request_hash,
            instrument,
            maximum_latest_n,
        })
    }
}

fn latest_successful_transport_result_hash(
    attempts: &[OutcomeTransportAttemptPreimage],
) -> Option<String> {
    attempts
        .iter()
        .rev()
        .find(|attempt| attempt.result.terminal_state == "available")
        .map(|attempt| attempt.result_hash.clone())
}

#[cfg(test)]
fn test_only_transport_attempts(
    request_columns: &RequestEvidenceColumns,
    available_evidence: Option<&OutcomeProviderAvailableEvidencePreimage>,
) -> OutcomeTransportAttemptsPreimage {
    let request: crate::selection::schema_v2::RequestEvidencePreimage =
        serde_json::from_str(&request_columns.request_evidence_json).expect("typed test request");
    let parameters: OutcomeMarketRequestParametersPreimage =
        serde_json::from_str(&request.parameters_json).expect("typed test parameters");
    let capability: ProviderCapabilityHashPreimage =
        serde_json::from_str(&request.provider_capability_json).expect("typed test capability");
    let expected_bar_count =
        u16::try_from(parameters.applicable_trading_dates.len()).expect("small test window");
    let transport_request = OutcomeTransportRequestPreimage {
        provider: capability.provider.clone(),
        source: PROVIDER_SOURCE.into(),
        canonical_stock_code: parameters.canonical_stock_code.clone(),
        canonical_market: parameters.canonical_market.clone(),
        interval: parameters.interval.as_str().into(),
        adjustment: parameters.adjustment.as_str().into(),
        latest_n: expected_bar_count,
    };
    let result = if let Some(available_evidence) = available_evidence {
        let provider_projection = &available_evidence.provider_evidence;
        let source = provider_projection
            .source
            .clone()
            .unwrap_or_else(|| PROVIDER_SOURCE.into());
        let source_at = provider_projection
            .source_at
            .clone()
            .or_else(|| parameters.applicable_trading_dates.last().cloned());
        let observed_at = provider_projection
            .observed_at
            .clone()
            .unwrap_or_else(|| "2026-07-28T07:01:00.000000000Z".into());
        let batch_id = provider_projection
            .batch_id
            .clone()
            .unwrap_or_else(|| "TEST_CODE_TRANSPORT_BATCH".into());
        let batch_content = OutcomeTransportBatchContentPreimage {
            provider: transport_request.provider.clone(),
            source: source.clone(),
            records: parameters
                .applicable_trading_dates
                .iter()
                .map(|market_date| OutcomeTransportBarFingerprint {
                    market_date: market_date.clone(),
                    open: "10".into(),
                    high: "11".into(),
                    low: "9".into(),
                    close: "10.5".into(),
                    core_volume_lots: "10".into(),
                    amount: Some("10500".into()),
                    provider: "Tdx".into(),
                    batch_id: batch_id.clone(),
                })
                .collect(),
        };
        let provider_evidence = OutcomeTransportEvidencePreimage {
            source,
            source_at,
            observed_at,
            batch_id,
            record_count: u32::from(expected_bar_count),
            batch_content_hash: sha256_json(&batch_content).expect("test content hash"),
            batch_content,
        };
        OutcomeTransportResultPreimage {
            terminal_state: "available".into(),
            requested_latest_n: expected_bar_count,
            actual_count: Some(expected_bar_count),
            provider_evidence_hash: Some(
                sha256_json(&provider_evidence).expect("test evidence hash"),
            ),
            provider_evidence: Some(provider_evidence),
            provider_error: None,
            provider_error_hash: None,
        }
    } else {
        let provider_error = OutcomeProviderErrorPreimage {
            variant: "connection_timeout".into(),
            coded_error: None,
            io_kind: None,
            raw_os_error: None,
            retry_attempts: None,
            structured_detail_hash: None,
            historical_bar_cardinality: None,
        };
        OutcomeTransportResultPreimage {
            terminal_state: "provider_error".into(),
            requested_latest_n: expected_bar_count,
            actual_count: None,
            provider_evidence: None,
            provider_evidence_hash: None,
            provider_error_hash: Some(
                sha256_json(&provider_error).expect("test provider-error hash"),
            ),
            provider_error: Some(provider_error),
        }
    };
    let attempt = OutcomeTransportAttemptPreimage {
        request_ordinal: 0,
        request_hash: sha256_json(&transport_request).expect("test transport request hash"),
        request: transport_request,
        result_hash: sha256_json(&result).expect("test transport result hash"),
        result,
    };
    OutcomeTransportAttemptsPreimage {
        domain: DOMAIN_OUTCOME_TRANSPORT_ATTEMPTS.into(),
        design_sha256: DESIGN_SHA256.into(),
        amendment_design_sha256: AMENDMENT_DESIGN_SHA256.into(),
        row_request_hash: request_columns.request_hash.clone(),
        request_evidence_hash: request_columns.request_evidence_hash.clone(),
        provider_capability_hash: request.provider_capability_hash,
        provider_revision: UPSTREAM_REVISION.into(),
        request_parameters_hash: request.parameters_json_hash,
        provider_request_hash: sha256_json(&"TEST_CODE_PROVIDER_REQUEST").unwrap(),
        verified_due_binding_hash: sha256_json(&"TEST_CODE_VERIFIED_DUE").unwrap(),
        adaptive_policy_version: OUTCOME_ADAPTIVE_POLICY_VERSION.into(),
        expected_bar_count,
        maximum_latest_n: expected_bar_count,
        selected_transport_result_hash: available_evidence.map(|_| attempt.result_hash.clone()),
        attempts_in_request_order: vec![attempt],
    }
}

fn outcome_transport_attempts_preimage(
    plan: &OutcomeAcquisitionPlan,
    attempts_in_request_order: Vec<OutcomeTransportAttemptPreimage>,
    selected_transport_result_hash: Option<String>,
) -> OutcomeTransportAttemptsPreimage {
    OutcomeTransportAttemptsPreimage {
        domain: DOMAIN_OUTCOME_TRANSPORT_ATTEMPTS.into(),
        design_sha256: DESIGN_SHA256.into(),
        amendment_design_sha256: AMENDMENT_DESIGN_SHA256.into(),
        row_request_hash: plan.request_evidence.request_hash.clone(),
        request_evidence_hash: plan.request_evidence.request_evidence_hash.clone(),
        provider_capability_hash: plan.provider_capability_hash.clone(),
        provider_revision: UPSTREAM_REVISION.into(),
        request_parameters_hash: plan.request_parameters_hash.clone(),
        provider_request_hash: plan.provider_request_hash.clone(),
        verified_due_binding_hash: plan.verified_due_binding_hash.clone(),
        adaptive_policy_version: OUTCOME_ADAPTIVE_POLICY_VERSION.into(),
        expected_bar_count: plan.expected_bar_count,
        maximum_latest_n: plan.maximum_latest_n,
        selected_transport_result_hash,
        attempts_in_request_order,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ValidatedHistoricalBarCardinality {
    available_count: u16,
    requested_total: u16,
}

/// 无 feature 构建下 TdxError 类型不存在 (library transport 编译期被移除),
/// 但同一套 cardinality 验证逻辑被两个入口共用: feature 构建的 typed 错误
/// 分支 (`validated_cardinality_mismatch`) 与无条件的历史复核
/// (`validate_transport_attempts`)。验证主体与 TdxError 解耦为单一来源,
/// 避免两条路径行为漂移。
fn validated_cardinality_from_parts(
    offset: u32,
    actual: usize,
    expected_page: u16,
    requested_total: u16,
    current_request: u16,
) -> Result<Option<ValidatedHistoricalBarCardinality>, GatewayError> {
    if requested_total != current_request {
        return Err(invalid_evidence(format!(
            "Magic TDX cardinality total {} does not equal current request {current_request}",
            requested_total
        )));
    }
    if expected_page == 0 {
        return Err(invalid_evidence(
            "Magic TDX cardinality error has zero expected page",
        ));
    }
    if !offset.is_multiple_of(OUTCOME_TDX_HISTORICAL_PAGE_SIZE) {
        return Err(invalid_evidence(format!(
            "Magic TDX cardinality offset={offset} is not aligned to the exact \
             {OUTCOME_TDX_HISTORICAL_PAGE_SIZE}-row page geometry"
        )));
    }
    let requested_total_u32 = u32::from(requested_total);
    if offset >= requested_total_u32 {
        return Err(invalid_evidence(format!(
            "Magic TDX cardinality offset={offset} is outside requested_total={requested_total}"
        )));
    }
    let expected_for_offset = (requested_total_u32 - offset).min(OUTCOME_TDX_HISTORICAL_PAGE_SIZE);
    if u32::from(expected_page) != expected_for_offset {
        return Err(invalid_evidence(format!(
            "Magic TDX cardinality offset={offset} expected_page={expected_page} does not equal \
             exact upstream page size {expected_for_offset} for requested_total={requested_total}"
        )));
    }
    let expected_page_end = offset
        .checked_add(u32::from(expected_page))
        .ok_or_else(|| invalid_evidence("Magic TDX expected page end overflows u32"))?;
    if expected_page_end > u32::from(requested_total) {
        return Err(invalid_evidence(format!(
            "Magic TDX cardinality page offset={offset} expected_page={expected_page} exceeds \
             requested_total={requested_total}"
        )));
    }
    if actual == usize::from(expected_page) {
        return Err(invalid_evidence(
            "Magic TDX cardinality error contradicts an exact page response",
        ));
    }
    if actual > usize::from(expected_page) {
        return Err(invalid_evidence(format!(
            "Magic TDX cardinality page returned {actual} rows above expected_page={expected_page}"
        )));
    }
    let actual = u32::try_from(actual)
        .map_err(|_| invalid_evidence("Magic TDX cardinality actual count exceeds u32"))?;
    let available_count = offset
        .checked_add(actual)
        .ok_or_else(|| invalid_evidence("Magic TDX available cardinality overflows u32"))?;
    let available_count = u16::try_from(available_count)
        .map_err(|_| invalid_evidence("Magic TDX available cardinality exceeds u16"))?;
    if available_count > requested_total {
        return Err(invalid_evidence(format!(
            "Magic TDX available cardinality {available_count} exceeds requested_total={requested_total}"
        )));
    }
    Ok(Some(ValidatedHistoricalBarCardinality {
        available_count,
        requested_total,
    }))
}

fn transport_bar_fingerprint(bar: &Bar) -> Result<OutcomeTransportBarFingerprint, GatewayError> {
    Ok(OutcomeTransportBarFingerprint {
        market_date: bar.bar_end().to_owned(),
        open: canonical_f64(bar.open().get()).map_err(schema_gateway_error)?,
        high: canonical_f64(bar.high().get()).map_err(schema_gateway_error)?,
        low: canonical_f64(bar.low().get()).map_err(schema_gateway_error)?,
        close: canonical_f64(bar.close().get()).map_err(schema_gateway_error)?,
        core_volume_lots: canonical_f64(bar.volume().get()).map_err(schema_gateway_error)?,
        amount: bar
            .amount()
            .map(|amount| canonical_f64(amount.get()))
            .transpose()
            .map_err(schema_gateway_error)?,
        provider: format!("{:?}", bar.provider()),
        batch_id: bar.batch_id().to_owned(),
    })
}

fn transport_batch_content(
    batch: &DataBatch<Bar>,
    evidence: &BatchEvidence,
) -> Result<OutcomeTransportBatchContentPreimage, GatewayError> {
    Ok(OutcomeTransportBatchContentPreimage {
        provider: PROVIDER.into(),
        source: evidence.source.clone(),
        records: batch
            .records()
            .iter()
            .map(transport_bar_fingerprint)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn validate_transport_attempts(
    attempts: &[OutcomeTransportAttemptPreimage],
) -> Result<(), GatewayError> {
    validate_transport_attempt_prefix(attempts)?;
    if attempts.is_empty() {
        return Err(invalid_evidence(
            "Magic TDX outcome response has no transport attempts",
        ));
    }
    for (ordinal, attempt) in attempts.iter().enumerate() {
        match (
            attempt.result.terminal_state.as_str(),
            &attempt.result.provider_evidence,
            &attempt.result.provider_evidence_hash,
            &attempt.result.provider_error,
        ) {
            (
                terminal_state @ ("available" | "provider_cardinality_violation"),
                Some(evidence),
                Some(evidence_hash),
                None,
            ) => {
                let record_count = u32::try_from(evidence.batch_content.records.len())
                    .map_err(|_| invalid_evidence("transport content count exceeds u32"))?;
                let actual_count = attempt.result.actual_count.ok_or_else(|| {
                    invalid_evidence(format!(
                        "Magic TDX transport attempt {ordinal} lost its actual count"
                    ))
                })?;
                let exact_cardinality = actual_count == attempt.request.latest_n;
                if (terminal_state == "available") != exact_cardinality
                    || record_count != u32::from(actual_count)
                    || record_count != evidence.record_count
                    || sha256_json(&evidence.batch_content).map_err(schema_gateway_error)?
                        != evidence.batch_content_hash
                    || sha256_json(evidence).map_err(schema_gateway_error)?
                        != evidence_hash.as_str()
                {
                    return Err(invalid_evidence(format!(
                        "Magic TDX {terminal_state} transport attempt {ordinal} evidence mismatch"
                    )));
                }
            }
            ("cardinality_mismatch", None, None, Some(provider_error)) => {
                let cardinality = provider_error
                    .historical_bar_cardinality
                    .as_ref()
                    .ok_or_else(|| {
                        invalid_evidence(format!(
                            "Magic TDX cardinality attempt {ordinal} lost structured fields"
                        ))
                    })?;
                let actual = usize::try_from(cardinality.actual).map_err(|_| {
                    invalid_evidence("stored TDX cardinality actual does not fit usize")
                })?;
                let validated = validated_cardinality_from_parts(
                    cardinality.offset,
                    actual,
                    cardinality.expected_page,
                    cardinality.requested_total,
                    attempt.request.latest_n,
                )?
                .ok_or_else(|| invalid_evidence("typed cardinality variant was not read"))?;
                let structured_hash = sha256_json(cardinality).map_err(schema_gateway_error)?;
                if provider_error.variant != "historical_bar_cardinality"
                    || provider_error.structured_detail_hash.as_deref()
                        != Some(structured_hash.as_str())
                    || provider_error.coded_error.is_some()
                    || provider_error.io_kind.is_some()
                    || provider_error.raw_os_error.is_some()
                    || provider_error.retry_attempts.is_some()
                    || attempt.result.actual_count != Some(validated.available_count)
                {
                    return Err(invalid_evidence(format!(
                        "Magic TDX cardinality attempt {ordinal} structured result mismatch"
                    )));
                }
            }
            ("provider_error", None, None, Some(provider_error))
                if provider_error.historical_bar_cardinality.is_none()
                    && provider_error.variant != "historical_bar_cardinality" => {}
            _ => {
                return Err(invalid_evidence(format!(
                    "Magic TDX transport attempt {ordinal} terminal-state matrix mismatch"
                )))
            }
        }
    }
    Ok(())
}

fn validate_transport_attempt_prefix(
    attempts: &[OutcomeTransportAttemptPreimage],
) -> Result<(), GatewayError> {
    for (ordinal, attempt) in attempts.iter().enumerate() {
        if attempt.request_ordinal
            != u32::try_from(ordinal)
                .map_err(|_| invalid_evidence("transport attempt ordinal exceeds u32"))?
            || attempt.request.latest_n != attempt.result.requested_latest_n
            || sha256_json(&attempt.request).map_err(schema_gateway_error)? != attempt.request_hash
            || sha256_json(&attempt.result).map_err(schema_gateway_error)? != attempt.result_hash
        {
            return Err(invalid_evidence(format!(
                "Magic TDX transport attempt {ordinal} request/result binding mismatch"
            )));
        }
    }
    Ok(())
}

fn project_magic_tdx_batch(
    plan: &OutcomeAcquisitionPlan,
    batch: DataBatch<Bar>,
    attempts: Vec<OutcomeTransportAttemptPreimage>,
    admission_now: DateTime<FixedOffset>,
) -> Result<StructurallyAdmittedBatch, OutcomeProjectionFailure> {
    let retained_attempts = attempts.clone();
    match project_magic_tdx_batch_inner(plan, batch, attempts, admission_now) {
        Ok(projected) => Ok(projected),
        Err(mut failure) => {
            failure.transport_attempts = retained_attempts;
            Err(failure)
        }
    }
}

fn project_magic_tdx_batch_inner(
    plan: &OutcomeAcquisitionPlan,
    batch: DataBatch<Bar>,
    attempts: Vec<OutcomeTransportAttemptPreimage>,
    admission_now: DateTime<FixedOffset>,
) -> Result<StructurallyAdmittedBatch, OutcomeProjectionFailure> {
    validate_transport_attempts(&attempts)?;
    let retained_attempts = attempts.clone();
    if !batch.quality().is_complete() {
        return Err(invalid_evidence(format!(
            "Magic TDX outcome batch is partial: {}",
            batch.quality().issues().join("; ")
        ))
        .into());
    }
    if batch.records().is_empty() || batch.records().len() > usize::from(plan.maximum_latest_n) {
        return Err(invalid_evidence(format!(
            "Magic TDX outcome cardinality invalid maximum={} actual={}",
            plan.maximum_latest_n,
            batch.records().len()
        ))
        .into());
    }
    let evidence = BatchEvidence::from_provenance(ProviderId::Tdx, batch.provenance())?;
    if evidence.provider != ProviderId::Tdx || evidence.source != PROVIDER_SOURCE {
        return Err(invalid_evidence(format!(
            "outcome provider/source must be Tdx/{PROVIDER_SOURCE}, got {:?}/{}",
            evidence.provider, evidence.source
        ))
        .into());
    }

    let mut full_preimages = Vec::with_capacity(batch.records().len());
    let mut full_by_date = BTreeMap::new();
    let mut seen_dates = BTreeSet::new();
    for (ordinal, bar) in batch.records().iter().enumerate() {
        let preimage = project_provider_bar(plan, &evidence, ordinal, bar)?;
        let date =
            NaiveDate::parse_from_str(&preimage.market_date, "%Y-%m-%d").map_err(|error| {
                invalid_evidence(format!(
                    "canonical Magic TDX outcome date {} failed parse: {error}",
                    preimage.market_date
                ))
            })?;
        if !seen_dates.insert(date) {
            return Err(invalid_evidence(format!(
                "Magic TDX outcome response repeats trading date {date}"
            ))
            .into());
        }
        if full_preimages
            .last()
            .is_some_and(|previous: &OutcomeProviderBarPreimage| {
                previous.market_date.as_bytes() >= preimage.market_date.as_bytes()
            })
        {
            return Err(invalid_evidence(format!(
                "Magic TDX outcome response is not provider-ordered at {date}"
            ))
            .into());
        }
        let volume_shares = admitted_volume_shares(bar.volume().get())?;
        let projected = OutcomeDailyBar {
            market_date: date,
            open: bar.open().get(),
            high: bar.high().get(),
            low: bar.low().get(),
            close: bar.close().get(),
            volume: volume_shares,
            amount: bar
                .amount()
                .expect("project_provider_bar requires amount")
                .get(),
        };
        full_by_date.insert(date, (preimage.clone(), projected));
        full_preimages.push(preimage);
    }
    let latest_date = full_preimages
        .last()
        .map(|bar| {
            NaiveDate::parse_from_str(&bar.market_date, "%Y-%m-%d")
                .expect("project_provider_bar produced canonical date")
        })
        .ok_or_else(|| invalid_evidence("Magic TDX outcome response is empty"))?;
    let post_close_cutoff =
        NaiveTime::from_hms_opt(A_SHARE_POST_CLOSE_HOUR, 0, 0).expect("static post-close cutoff");
    if latest_date == admission_now.date_naive() && admission_now.time() < post_close_cutoff {
        let available_evidence = build_available_evidence(plan, &evidence, &full_preimages)?.2;
        return Err(OutcomeProjectionFailure::with_available_evidence(
            GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Tdx),
                "partial",
                "market_session_unsettled",
                true,
                format!(
                    "Magic TDX latest daily bar {latest_date} is an intraday partial at {}",
                    admission_now.to_rfc3339()
                ),
            ),
            available_evidence,
        ));
    }
    if let Err(reason) = validate_daily_freshness(
        latest_date,
        admission_now.with_timezone(&chrono::Local),
        &FreshnessConfig::default(),
        &DqStats::new(),
    ) {
        let available_evidence = build_available_evidence(plan, &evidence, &full_preimages)?.2;
        return Err(OutcomeProjectionFailure::with_available_evidence(
            GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Tdx),
                "partial",
                "evidence_stale",
                true,
                format!(
                    "latest Magic TDX daily bar {latest_date} failed one-trading-day freshness: {}",
                    reason.label()
                ),
            ),
            available_evidence,
        ));
    }
    if latest_date > plan.request_local_date {
        return Err(invalid_evidence(format!(
            "Magic TDX returned future/unsettled daily bar {latest_date} beyond {}",
            plan.request_local_date
        ))
        .into());
    }
    let latest_date_text = latest_date.format("%Y-%m-%d").to_string();
    if evidence.source_at.as_deref() != Some(latest_date_text.as_str()) {
        return Err(invalid_evidence(format!(
            "Magic TDX batch source_at {:?} does not equal latest provider bar {latest_date}",
            evidence.source_at
        ))
        .into());
    }
    let response_record_count = u32::try_from(full_preimages.len())
        .map_err(|_| invalid_evidence("outcome response count exceeds u32"))?;
    let selected_transport_content = transport_batch_content(&batch, &evidence)?;
    let selected_transport_content_hash =
        sha256_json(&selected_transport_content).map_err(schema_gateway_error)?;
    let selected_transport_result_hash = attempts
        .iter()
        .rev()
        .find(|attempt| {
            attempt.result.terminal_state == "available"
                && attempt
                    .result
                    .provider_evidence
                    .as_ref()
                    .is_some_and(|provider_evidence| {
                        provider_evidence.source == evidence.source
                            && provider_evidence.source_at == evidence.source_at
                            && provider_evidence.observed_at == evidence.observed_at
                            && provider_evidence.batch_id == evidence.batch_id
                            && provider_evidence.record_count == response_record_count
                            && provider_evidence.batch_content == selected_transport_content
                            && provider_evidence.batch_content_hash
                                == selected_transport_content_hash
                    })
        })
        .map(|attempt| attempt.result_hash.clone())
        .ok_or_else(|| {
            invalid_evidence(
                "selected Magic TDX outcome batch has no matching transport result evidence",
            )
        })?;

    let response = OutcomeProviderResponsePreimage {
        domain: DOMAIN_PROVIDER_RESPONSE.into(),
        provider_request_hash: plan.provider_request_hash.clone(),
        provider: PROVIDER.into(),
        source: evidence.source.clone(),
        source_at: evidence.source_at.clone(),
        observed_at: evidence.observed_at.clone(),
        batch_id: evidence.batch_id.clone(),
        record_count: response_record_count,
        trading_date_vector_hash: plan.trading_date_vector_hash.clone(),
        expected_trading_dates: plan
            .applicable_trading_dates
            .iter()
            .map(|date| date.format("%Y-%m-%d").to_string())
            .collect(),
        returned_trading_dates: full_preimages
            .iter()
            .map(|record| record.market_date.clone())
            .collect(),
        selected_transport_result_hash,
        transport_attempts_in_request_order: attempts,
        provider_ordered_records: full_preimages,
    };
    let provider_response_json = canonical_json(&response).map_err(schema_gateway_error)?;
    let provider_response_hash = sha256_json(&response).map_err(schema_gateway_error)?;

    let (window_preimages, bars) =
        match select_window_records(&plan.applicable_trading_dates, &full_by_date) {
            Ok(selected) => selected,
            Err(failure) => {
                let actual_records = if failure.partial_preimages.is_empty() {
                    response.provider_ordered_records.as_slice()
                } else {
                    failure.partial_preimages.as_slice()
                };
                let available_evidence =
                    build_available_evidence(plan, &evidence, actual_records)?.2;
                return Err(OutcomeProjectionFailure::with_available_evidence(
                    failure.error,
                    available_evidence,
                ));
            }
        };
    validate_receipted_t0_baseline(
        plan.phase,
        plan.window_start,
        plan.receipted_t0_close.as_deref(),
        plan.receipted_t0_volume_shares.as_deref(),
        &bars,
    )?;
    let validation_records = project_kline_records(&bars);
    let (provider_ordered_content_json, provider_ordered_content_hash, available_evidence) =
        build_available_evidence(plan, &evidence, &window_preimages)?;

    Ok(StructurallyAdmittedBatch {
        bars,
        validation_records,
        evidence,
        provider_ordered_content_json,
        provider_ordered_content_hash,
        available_evidence,
        provider_response_json,
        provider_response_hash,
        window_preimages,
        transport_attempts: retained_attempts,
    })
}

fn build_available_evidence(
    plan: &OutcomeAcquisitionPlan,
    evidence: &BatchEvidence,
    provider_ordered_records: &[OutcomeProviderBarPreimage],
) -> Result<(String, String, OutcomeProviderAvailableEvidencePreimage), GatewayError> {
    if provider_ordered_records.is_empty() {
        return Err(invalid_evidence(
            "provider available evidence cannot bind an empty outcome record set",
        ));
    }
    let ordered_content = OutcomeProviderOrderedContentPreimage {
        domain: DOMAIN_PROVIDER_ORDERED_CONTENT.into(),
        provider: PROVIDER.into(),
        source: evidence.source.clone(),
        canonical_stock_code: plan.canonical_stock_code.clone(),
        canonical_market: plan.canonical_market.clone(),
        interval: DailyIntervalKind::Day.as_str().into(),
        adjustment: AdjustmentKind::None.as_str().into(),
        provider_ordered_records: provider_ordered_records.to_vec(),
    };
    let provider_ordered_content_json =
        canonical_json(&ordered_content).map_err(schema_gateway_error)?;
    let provider_ordered_content_hash =
        sha256_json(&ordered_content).map_err(schema_gateway_error)?;
    let provider_evidence = ProviderAvailableEvidencePreimage {
        domain: DOMAIN_PROVIDER_AVAILABLE_EVIDENCE.into(),
        evidence_kind: ProviderEvidenceKind::OutcomeDailyBars,
        provider: PROVIDER.into(),
        source: Some(evidence.source.clone()),
        source_at: evidence.source_at.clone(),
        observed_at: Some(evidence.observed_at.clone()),
        batch_id: Some(evidence.batch_id.clone()),
        batch_content_hash: Some(provider_ordered_content_hash.clone()),
    };
    provider_evidence
        .validate_complete()
        .map_err(schema_gateway_error)?;
    let available_evidence = OutcomeProviderAvailableEvidencePreimage {
        domain: DOMAIN_OUTCOME_PROVIDER_AVAILABLE_EVIDENCE.into(),
        request_hash: plan.request_evidence.request_hash.clone(),
        calendar_hash: plan.calendar_hash.clone(),
        trading_date_vector_hash: plan.trading_date_vector_hash.clone(),
        expected_trading_dates: plan
            .applicable_trading_dates
            .iter()
            .map(|date| date.format("%Y-%m-%d").to_string())
            .collect(),
        returned_trading_dates: provider_ordered_records
            .iter()
            .map(|record| record.market_date.clone())
            .collect(),
        provider_evidence,
    };
    let semantic_request = plan
        .request_evidence
        .validate(Some(RequestKind::OutcomeMarketEvidence))
        .map_err(schema_gateway_error)?;
    let parameters = decode_canonical_pair::<OutcomeMarketRequestParametersPreimage>(
        &semantic_request.parameters_json,
        &semantic_request.parameters_json_hash,
        "outcome semantic request parameters",
    )?;
    available_evidence
        .validate_partial(&parameters, &semantic_request.request_hash)
        .map_err(schema_gateway_error)?;
    Ok((
        provider_ordered_content_json,
        provider_ordered_content_hash,
        available_evidence,
    ))
}

fn decode_canonical_pair<T>(
    json: &str,
    expected_hash: &str,
    label: &'static str,
) -> Result<T, GatewayError>
where
    T: DeserializeOwned + Serialize,
{
    if sha256_bytes(json.as_bytes()) != expected_hash {
        return Err(invalid_evidence(format!(
            "{label} hash does not bind its exact canonical JSON bytes"
        )));
    }
    let value = serde_json::from_str::<T>(json).map_err(|error| {
        invalid_evidence(format!("{label} is not its registered typed JSON: {error}"))
    })?;
    let canonical = canonical_json(&value).map_err(schema_gateway_error)?;
    if canonical != json || sha256_json(&value).map_err(schema_gateway_error)? != expected_hash {
        return Err(invalid_evidence(format!(
            "{label} is not canonical or its typed rehash differs"
        )));
    }
    Ok(value)
}

fn parse_canonical_positive(value: &str, field: &'static str) -> Result<f64, GatewayError> {
    let parsed = value
        .parse::<f64>()
        .map_err(|error| invalid_evidence(format!("{field} is not a decimal number: {error}")))?;
    if !parsed.is_finite()
        || parsed <= 0.0
        || canonical_f64(parsed).map_err(schema_gateway_error)? != value
    {
        return Err(invalid_evidence(format!(
            "{field} is not a positive canonical finite decimal"
        )));
    }
    Ok(parsed)
}

fn validate_provider_bar_preimage(record: &OutcomeProviderBarPreimage) -> Result<(), GatewayError> {
    NaiveDate::parse_from_str(&record.market_date, "%Y-%m-%d").map_err(|error| {
        invalid_evidence(format!(
            "provider bar market_date {:?} is invalid: {error}",
            record.market_date
        ))
    })?;
    let open = parse_canonical_positive(&record.open, "provider_bar.open")?;
    let high = parse_canonical_positive(&record.high, "provider_bar.high")?;
    let low = parse_canonical_positive(&record.low, "provider_bar.low")?;
    let close = parse_canonical_positive(&record.close, "provider_bar.close")?;
    let core_volume =
        parse_canonical_positive(&record.core_volume_lots, "provider_bar.core_volume_lots")?;
    let admitted_volume = parse_canonical_positive(
        &record.admitted_volume_shares,
        "provider_bar.admitted_volume_shares",
    )?;
    parse_canonical_positive(&record.amount, "provider_bar.amount")?;
    if high < open.max(close)
        || low > open.min(close)
        || high < low
        || canonical_f64(core_volume * SHARES_PER_BOARD_LOT).map_err(schema_gateway_error)?
            != record.admitted_volume_shares
        || admitted_volume != core_volume * SHARES_PER_BOARD_LOT
    {
        return Err(invalid_evidence(
            "provider bar OHLC or board-lot-to-share conversion is inconsistent",
        ));
    }
    Ok(())
}

fn validate_admitted_evidence(admitted: &AdmittedOutcomeDailyBars) -> Result<(), GatewayError> {
    let semantic_request = admitted
        .request_evidence
        .validate(Some(RequestKind::OutcomeMarketEvidence))
        .map_err(schema_gateway_error)?;
    let semantic_parameters = decode_canonical_pair::<OutcomeMarketRequestParametersPreimage>(
        &semantic_request.parameters_json,
        &semantic_request.parameters_json_hash,
        "outcome semantic request parameters",
    )?;
    let provider_capability = decode_canonical_pair::<ProviderCapabilityHashPreimage>(
        &semantic_request.provider_capability_json,
        &semantic_request.provider_capability_hash,
        "outcome provider capability",
    )?;
    let provider_request = decode_canonical_pair::<OutcomeProviderRequestPreimage>(
        &admitted.provider_request_json,
        &admitted.provider_request_hash,
        "outcome provider request",
    )?;
    let provider_response = decode_canonical_pair::<OutcomeProviderResponsePreimage>(
        &admitted.provider_response_json,
        &admitted.provider_response_hash,
        "outcome provider response",
    )?;
    let ordered_content = decode_canonical_pair::<OutcomeProviderOrderedContentPreimage>(
        &admitted.provider_ordered_content_json,
        &admitted.provider_ordered_content_hash,
        "outcome provider-ordered content",
    )?;
    let lifecycle = decode_canonical_pair::<OutcomeLifecycleEvidencePreimage>(
        &admitted.lifecycle_evidence_json,
        &admitted.lifecycle_evidence_hash,
        "outcome lifecycle evidence",
    )?;
    let admitted_window = decode_canonical_pair::<OutcomeAdmittedWindowPreimage>(
        &admitted.admitted_window_json,
        &admitted.admitted_window_hash,
        "outcome admitted window",
    )?;
    admitted
        .transport_attempts
        .validate(
            &admitted.request_evidence.request_hash,
            &admitted.request_evidence.request_evidence_hash,
            &semantic_request,
            &semantic_parameters,
            &provider_capability,
        )
        .map_err(schema_gateway_error)?;

    let stored_due_date = admitted.stored_due_date.format("%Y-%m-%d").to_string();
    let window_start = admitted
        .window_dates
        .first()
        .ok_or_else(|| invalid_evidence("admitted outcome window has no first date"))?
        .format("%Y-%m-%d")
        .to_string();
    let window_end = admitted
        .window_dates
        .last()
        .ok_or_else(|| invalid_evidence("admitted outcome window has no last date"))?
        .format("%Y-%m-%d")
        .to_string();
    let window_dates = admitted
        .window_dates
        .iter()
        .map(|date| date.format("%Y-%m-%d").to_string())
        .collect::<Vec<_>>();

    if semantic_parameters.sample_key != admitted.sample_key
        || semantic_parameters.canonical_stock_code != admitted.canonical_stock_code
        || semantic_parameters.canonical_market != admitted.canonical_market
        || semantic_parameters.phase != admitted.phase
        || semantic_parameters.stored_due_date != stored_due_date
        || semantic_parameters.trading_date_vector != admitted.trading_date_vector
        || semantic_parameters.trading_date_vector_hash != admitted.trading_date_vector_hash
        || semantic_parameters.applicable_trading_dates != window_dates
        || semantic_parameters.window_start != window_start
        || semantic_parameters.window_end != window_end
        || semantic_parameters.interval != DailyIntervalKind::Day
        || semantic_parameters.adjustment != AdjustmentKind::None
    {
        return Err(invalid_evidence(
            "typed semantic request does not equal the admitted outcome identity/window",
        ));
    }
    if provider_request.domain != DOMAIN_OUTCOME_PROVIDER_REQUEST
        || provider_request.design_sha256 != DESIGN_SHA256
        || provider_request.amendment_design_sha256 != AMENDMENT_DESIGN_SHA256
        || provider_request.semantic_request_hash != admitted.request_evidence.request_hash
        || provider_request.verified_due_binding_hash != admitted.verified_due_binding_hash
        || provider_request.sample_key != admitted.sample_key
        || provider_request.canonical_stock_code != admitted.canonical_stock_code
        || provider_request.canonical_market != admitted.canonical_market
        || provider_request.phase != admitted.phase
        || provider_request.stored_due_date != stored_due_date
        || provider_request.calendar_version != semantic_parameters.calendar_version
        || provider_request.calendar_hash != semantic_parameters.calendar_hash
        || provider_request.trading_date_vector != admitted.trading_date_vector
        || provider_request.trading_date_vector_hash != admitted.trading_date_vector_hash
        || provider_request.expected_trading_dates != window_dates
        || provider_request.window_start != window_start
        || provider_request.window_end != window_end
        || usize::from(provider_request.expected_bar_count) != admitted.bars.len()
        || provider_request.interval != DailyIntervalKind::Day.as_str()
        || provider_request.adjustment != AdjustmentKind::None.as_str()
        || provider_request.adaptive_policy_version != OUTCOME_ADAPTIVE_POLICY_VERSION
        || provider_request.volume_conversion_contract != VOLUME_CONVERSION_CONTRACT
        || provider_request.volume_conversion_version != VOLUME_CONVERSION_VERSION
        || provider_request.shares_per_board_lot
            != canonical_f64(SHARES_PER_BOARD_LOT).map_err(schema_gateway_error)?
    {
        return Err(invalid_evidence(
            "provider request does not bind the admitted semantic request and exact window",
        ));
    }
    provider_request
        .validate(&semantic_request, &semantic_parameters)
        .map_err(schema_gateway_error)?;
    if provider_response.domain != DOMAIN_PROVIDER_RESPONSE
        || provider_response.provider_request_hash != admitted.provider_request_hash
        || provider_response.provider != PROVIDER
        || provider_response.source != PROVIDER_SOURCE
        || provider_response.trading_date_vector_hash != admitted.trading_date_vector_hash
        || provider_response.expected_trading_dates != window_dates
        || provider_response.returned_trading_dates
            != provider_response
                .provider_ordered_records
                .iter()
                .map(|record| record.market_date.clone())
                .collect::<Vec<_>>()
        || usize::try_from(provider_response.record_count).ok()
            != Some(provider_response.provider_ordered_records.len())
        || provider_response.selected_transport_result_hash
            != admitted
                .transport_attempts
                .selected_transport_result_hash
                .clone()
                .ok_or_else(|| {
                    invalid_evidence("admitted transport attempts lost selected result hash")
                })?
        || provider_response.transport_attempts_in_request_order
            != admitted.transport_attempts.attempts_in_request_order
    {
        return Err(invalid_evidence(
            "provider response does not bind the exact request/provider/record count",
        ));
    }
    DateTime::parse_from_rfc3339(&provider_response.observed_at).map_err(|error| {
        invalid_evidence(format!(
            "provider response observed_at is not RFC3339: {error}"
        ))
    })?;
    provider_response
        .source_at
        .as_deref()
        .ok_or_else(|| invalid_evidence("provider response source_at is absent"))
        .and_then(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map(|_| ())
                .map_err(|error| {
                    invalid_evidence(format!(
                        "provider response source_at is not a market date: {error}"
                    ))
                })
        })?;
    if provider_response.batch_id.trim().is_empty() {
        return Err(invalid_evidence("provider response batch_id is empty"));
    }
    validate_transport_attempts(&provider_response.transport_attempts_in_request_order)?;
    for attempt in &provider_response.transport_attempts_in_request_order {
        if attempt.request.provider != PROVIDER
            || attempt.request.source != PROVIDER_SOURCE
            || attempt.request.canonical_stock_code != admitted.canonical_stock_code
            || attempt.request.canonical_market != admitted.canonical_market
            || attempt.request.interval != DailyIntervalKind::Day.as_str()
            || attempt.request.adjustment != AdjustmentKind::None.as_str()
            || attempt.request.latest_n == 0
        {
            return Err(invalid_evidence(
                "transport attempt request does not bind the admitted provider/identity/contract",
            ));
        }
    }
    let selected_attempt = provider_response
        .transport_attempts_in_request_order
        .iter()
        .find(|attempt| attempt.result_hash == provider_response.selected_transport_result_hash)
        .ok_or_else(|| {
            invalid_evidence(
                "provider response selected result hash is absent from its ordered attempts",
            )
        })?;
    let selected_evidence = selected_attempt
        .result
        .provider_evidence
        .as_ref()
        .ok_or_else(|| invalid_evidence("selected transport result has no provider evidence"))?;
    if selected_attempt.result.terminal_state != "available"
        || selected_evidence.source != provider_response.source
        || selected_evidence.source_at != provider_response.source_at
        || selected_evidence.observed_at != provider_response.observed_at
        || selected_evidence.batch_id != provider_response.batch_id
        || selected_evidence.record_count != provider_response.record_count
        || selected_evidence.batch_content.provider != PROVIDER
        || selected_evidence.batch_content.source != provider_response.source
        || selected_evidence.batch_content.records.len()
            != provider_response.provider_ordered_records.len()
    {
        return Err(invalid_evidence(
            "selected transport evidence does not equal the provider response envelope",
        ));
    }
    for (ordinal, record) in provider_response
        .provider_ordered_records
        .iter()
        .enumerate()
    {
        if record.domain != DOMAIN_PROVIDER_BAR
            || usize::try_from(record.provider_ordinal).ok() != Some(ordinal)
            || record.provider != PROVIDER
            || record.batch_id != provider_response.batch_id
            || record.canonical_stock_code != admitted.canonical_stock_code
            || record.canonical_market != admitted.canonical_market
            || record.market_date != record.bar_start
            || record.market_date != record.bar_end
            || record.market_date != record.source_at
            || record.interval != DailyIntervalKind::Day.as_str()
            || record.adjustment != AdjustmentKind::None.as_str()
            || record.volume_conversion_contract != VOLUME_CONVERSION_CONTRACT
            || record.volume_conversion_version != VOLUME_CONVERSION_VERSION
            || record.core_volume_unit != CORE_VOLUME_UNIT
            || record.admitted_volume_unit != ADMITTED_VOLUME_UNIT
            || record.amount_unit != AMOUNT_UNIT
        {
            return Err(invalid_evidence(
                "provider response record identity/order/batch does not match its envelope",
            ));
        }
        validate_provider_bar_preimage(record)?;
        let transport_record = &selected_evidence.batch_content.records[ordinal];
        if transport_record.market_date != record.market_date
            || transport_record.open != record.open
            || transport_record.high != record.high
            || transport_record.low != record.low
            || transport_record.close != record.close
            || transport_record.core_volume_lots != record.core_volume_lots
            || transport_record.amount.as_deref() != Some(record.amount.as_str())
            || transport_record.batch_id != record.batch_id
            || transport_record.provider != format!("{:?}", ProviderId::Tdx)
        {
            return Err(invalid_evidence(
                "selected transport record does not equal the canonical provider response record",
            ));
        }
    }

    admitted
        .available_evidence
        .validate_complete(&semantic_parameters, &semantic_request.request_hash)
        .map_err(schema_gateway_error)?;
    let provider_evidence = &admitted.available_evidence.provider_evidence;
    if provider_evidence.evidence_kind != ProviderEvidenceKind::OutcomeDailyBars
        || provider_evidence.provider != PROVIDER
        || provider_evidence.source.as_deref() != Some(provider_response.source.as_str())
        || provider_evidence.source_at != provider_response.source_at
        || provider_evidence.observed_at.as_deref() != Some(provider_response.observed_at.as_str())
        || provider_evidence.batch_id.as_deref() != Some(provider_response.batch_id.as_str())
        || provider_evidence.batch_content_hash.as_deref()
            != Some(admitted.provider_ordered_content_hash.as_str())
        || admitted.available_evidence.expected_trading_dates != window_dates
        || admitted.available_evidence.returned_trading_dates != window_dates
    {
        return Err(invalid_evidence(
            "available evidence projection does not equal provider response/content facts",
        ));
    }
    if ordered_content.domain != DOMAIN_PROVIDER_ORDERED_CONTENT
        || ordered_content.provider != PROVIDER
        || ordered_content.source != provider_response.source
        || ordered_content.canonical_stock_code != admitted.canonical_stock_code
        || ordered_content.canonical_market != admitted.canonical_market
        || ordered_content.interval != DailyIntervalKind::Day.as_str()
        || ordered_content.adjustment != AdjustmentKind::None.as_str()
        || ordered_content.provider_ordered_records != admitted_window.provider_ordered_records
    {
        return Err(invalid_evidence(
            "provider-ordered content does not equal the admitted window projection",
        ));
    }
    let mut selected_cursor = 0_usize;
    for response_record in &provider_response.provider_ordered_records {
        if ordered_content
            .provider_ordered_records
            .get(selected_cursor)
            .is_some_and(|selected| selected == response_record)
        {
            selected_cursor += 1;
        }
    }
    if selected_cursor != ordered_content.provider_ordered_records.len() {
        return Err(invalid_evidence(
            "admitted provider records are not an order-preserving subset of the real response",
        ));
    }

    if lifecycle.domain != DOMAIN_LIFECYCLE_EVIDENCE
        || lifecycle.design_sha256 != DESIGN_SHA256
        || lifecycle.sample_key != admitted.sample_key
        || lifecycle.canonical_stock_code != admitted.canonical_stock_code
        || lifecycle.canonical_market != admitted.canonical_market
        || lifecycle.phase != admitted.phase
        || lifecycle.window_start != window_start
        || lifecycle.window_end != window_end
    {
        return Err(invalid_evidence(
            "lifecycle evidence does not bind the admitted identity/window",
        ));
    }
    if admitted_window.domain != DOMAIN_ADMITTED_WINDOW
        || admitted_window.design_sha256 != DESIGN_SHA256
        || admitted_window.semantic_request_hash != admitted.request_evidence.request_hash
        || admitted_window.verified_due_binding_hash != admitted.verified_due_binding_hash
        || admitted_window.sample_key != admitted.sample_key
        || admitted_window.canonical_stock_code != admitted.canonical_stock_code
        || admitted_window.canonical_market != admitted.canonical_market
        || admitted_window.phase != admitted.phase
        || admitted_window.stored_due_date != stored_due_date
        || admitted_window.calendar_hash != semantic_parameters.calendar_hash
        || admitted_window.trading_date_vector != admitted.trading_date_vector
        || admitted_window.trading_date_vector_hash != admitted.trading_date_vector_hash
        || admitted_window.expected_trading_dates != window_dates
        || admitted_window.returned_trading_dates != window_dates
        || admitted_window.provider_response_hash != admitted.provider_response_hash
        || admitted_window.lifecycle_evidence_hash != admitted.lifecycle_evidence_hash
    {
        return Err(invalid_evidence(
            "admitted-window evidence does not cross-link request/response/lifecycle identity",
        ));
    }
    if admitted.bars.len() != ordered_content.provider_ordered_records.len() {
        return Err(invalid_evidence(
            "admitted bars and provider-ordered evidence cardinalities differ",
        ));
    }
    for (bar, record) in admitted
        .bars
        .iter()
        .zip(&ordered_content.provider_ordered_records)
    {
        if record.market_date != bar.market_date.format("%Y-%m-%d").to_string()
            || record.open != canonical_f64(bar.open).map_err(schema_gateway_error)?
            || record.high != canonical_f64(bar.high).map_err(schema_gateway_error)?
            || record.low != canonical_f64(bar.low).map_err(schema_gateway_error)?
            || record.close != canonical_f64(bar.close).map_err(schema_gateway_error)?
            || record.admitted_volume_shares
                != canonical_f64(bar.volume).map_err(schema_gateway_error)?
            || record.amount != canonical_f64(bar.amount).map_err(schema_gateway_error)?
        {
            return Err(invalid_evidence(
                "admitted numeric bars do not equal their canonical provider preimages",
            ));
        }
    }
    Ok(())
}

fn bind_lifecycle_and_window(
    plan: &OutcomeAcquisitionPlan,
    projected: &StructurallyAdmittedBatch,
    admission: OutcomeLifecycleAdmission,
) -> Result<(String, String, String, String), GatewayError> {
    if admission.window_start != plan.window_start || admission.window_end != plan.window_end {
        return Err(invalid_evidence(format!(
            "outcome lifecycle admission window {}..{} conflicts with immutable request {}..{}",
            admission.window_start, admission.window_end, plan.window_start, plan.window_end
        )));
    }
    let lifecycle = OutcomeLifecycleEvidencePreimage {
        domain: DOMAIN_LIFECYCLE_EVIDENCE.into(),
        design_sha256: DESIGN_SHA256.into(),
        sample_key: plan.sample_key.clone(),
        canonical_stock_code: plan.canonical_stock_code.clone(),
        canonical_market: plan.canonical_market.clone(),
        phase: plan.phase,
        window_start: admission.window_start.format("%Y-%m-%d").to_string(),
        window_end: admission.window_end.format("%Y-%m-%d").to_string(),
        listing_date: admission
            .listing_date
            .map(|date| date.format("%Y-%m-%d").to_string()),
        listing_batch_id: admission.listing_batch_id,
        listing_unavailable_reason_code: admission.listing_unavailable_reason_code,
        listing_unavailable_retryable: admission.listing_unavailable_retryable,
        corporate_action_state: admission.corporate_action_state,
        corporate_action_batch_id: admission.corporate_action_batch_id,
        adjacent_evidence: admission
            .adjacent_evidence
            .into_iter()
            .map(|evidence| OutcomeLifecycleAdjacentPreimage {
                provider: evidence.provider,
                batch_identity: evidence.batch_identity,
                listing_date: evidence
                    .listing_date
                    .map(|date| date.format("%Y-%m-%d").to_string()),
                corporate_action_identity: evidence.corporate_action_identity,
            })
            .collect(),
    };
    let lifecycle_evidence_json = canonical_json(&lifecycle).map_err(schema_gateway_error)?;
    let lifecycle_evidence_hash = sha256_json(&lifecycle).map_err(schema_gateway_error)?;
    let window = OutcomeAdmittedWindowPreimage {
        domain: DOMAIN_ADMITTED_WINDOW.into(),
        design_sha256: DESIGN_SHA256.into(),
        semantic_request_hash: plan.request_evidence.request_hash.clone(),
        verified_due_binding_hash: plan.verified_due_binding_hash.clone(),
        sample_key: plan.sample_key.clone(),
        canonical_stock_code: plan.canonical_stock_code.clone(),
        canonical_market: plan.canonical_market.clone(),
        phase: plan.phase,
        stored_due_date: plan.stored_due_date.format("%Y-%m-%d").to_string(),
        calendar_hash: plan.calendar_hash.clone(),
        trading_date_vector: plan.trading_date_vector.clone(),
        trading_date_vector_hash: plan.trading_date_vector_hash.clone(),
        expected_trading_dates: plan
            .applicable_trading_dates
            .iter()
            .map(|date| date.format("%Y-%m-%d").to_string())
            .collect(),
        returned_trading_dates: projected
            .bars
            .iter()
            .map(|bar| bar.market_date.format("%Y-%m-%d").to_string())
            .collect(),
        receipted_t0_close: plan.receipted_t0_close.clone(),
        receipted_t0_volume_shares: plan.receipted_t0_volume_shares.clone(),
        provider_response_hash: projected.provider_response_hash.clone(),
        lifecycle_evidence_hash: lifecycle_evidence_hash.clone(),
        provider_ordered_records: projected.window_preimages.clone(),
    };
    let admitted_window_json = canonical_json(&window).map_err(schema_gateway_error)?;
    let admitted_window_hash = sha256_json(&window).map_err(schema_gateway_error)?;
    Ok((
        lifecycle_evidence_json,
        lifecycle_evidence_hash,
        admitted_window_json,
        admitted_window_hash,
    ))
}

fn project_provider_bar(
    plan: &OutcomeAcquisitionPlan,
    evidence: &BatchEvidence,
    ordinal: usize,
    bar: &Bar,
) -> Result<OutcomeProviderBarPreimage, GatewayError> {
    if bar.instrument() != &plan.instrument
        || bar.interval() != BarInterval::Day
        || bar.provider() != ProviderId::Tdx
        || bar.batch_id() != evidence.batch_id
        || bar.adjustment() != Adjustment::Unadjusted
    {
        return Err(invalid_evidence(
            "Magic TDX outcome record identity/provider/batch/interval/adjustment mismatch",
        ));
    }
    if bar.bar_start() != bar.bar_end() {
        return Err(invalid_evidence(format!(
            "daily outcome bar range differs {}..{}",
            bar.bar_start(),
            bar.bar_end()
        )));
    }
    let date = NaiveDate::parse_from_str(bar.bar_end(), "%Y-%m-%d").map_err(|error| {
        invalid_evidence(format!(
            "Magic TDX outcome bar date {:?} is invalid: {error}",
            bar.bar_end()
        ))
    })?;
    let source_at = bar
        .source_at()
        .ok_or_else(|| invalid_evidence(format!("outcome bar {date} has no source_at")))?;
    if source_at != bar.bar_end() {
        return Err(invalid_evidence(format!(
            "outcome bar {date} source_at {source_at:?} differs from bar date"
        )));
    }
    let amount = bar
        .amount()
        .ok_or_else(|| invalid_evidence(format!("outcome bar {date} has no amount")))?;
    let open = bar.open().get();
    let high = bar.high().get();
    let low = bar.low().get();
    let close = bar.close().get();
    let core_volume_lots = bar.volume().get();
    let admitted_volume_shares = admitted_volume_shares(core_volume_lots)?;
    let amount_value = amount.get();
    if [open, high, low, close]
        .into_iter()
        .any(|price| !price.is_finite() || price <= 0.0)
        || high < open.max(close)
        || low > open.min(close)
        || high < low
        || !core_volume_lots.is_finite()
        || core_volume_lots <= 0.0
        || !admitted_volume_shares.is_finite()
        || admitted_volume_shares <= 0.0
        || !amount_value.is_finite()
        || amount_value <= 0.0
    {
        return Err(invalid_evidence(format!(
            "outcome bar {date} has invalid OHLC/volume/amount"
        )));
    }
    Ok(OutcomeProviderBarPreimage {
        domain: DOMAIN_PROVIDER_BAR.into(),
        provider_ordinal: u32::try_from(ordinal)
            .map_err(|_| invalid_evidence("outcome provider ordinal exceeds u32"))?,
        canonical_stock_code: plan.canonical_stock_code.clone(),
        canonical_market: plan.canonical_market.clone(),
        market_date: date.format("%Y-%m-%d").to_string(),
        bar_start: bar.bar_start().to_owned(),
        bar_end: bar.bar_end().to_owned(),
        open: canonical_f64(open).map_err(schema_gateway_error)?,
        high: canonical_f64(high).map_err(schema_gateway_error)?,
        low: canonical_f64(low).map_err(schema_gateway_error)?,
        close: canonical_f64(close).map_err(schema_gateway_error)?,
        volume_conversion_contract: VOLUME_CONVERSION_CONTRACT.into(),
        volume_conversion_version: VOLUME_CONVERSION_VERSION.into(),
        core_volume_unit: CORE_VOLUME_UNIT.into(),
        shares_per_board_lot: canonical_f64(SHARES_PER_BOARD_LOT).map_err(schema_gateway_error)?,
        core_volume_lots: canonical_f64(core_volume_lots).map_err(schema_gateway_error)?,
        admitted_volume_unit: ADMITTED_VOLUME_UNIT.into(),
        admitted_volume_shares: canonical_f64(admitted_volume_shares)
            .map_err(schema_gateway_error)?,
        amount_unit: AMOUNT_UNIT.into(),
        amount: canonical_f64(amount_value).map_err(schema_gateway_error)?,
        interval: DailyIntervalKind::Day.as_str().into(),
        adjustment: AdjustmentKind::None.as_str().into(),
        source_at: source_at.to_owned(),
        provider: PROVIDER.into(),
        batch_id: evidence.batch_id.clone(),
    })
}

fn project_kline_records(bars: &[OutcomeDailyBar]) -> Vec<KlineData> {
    bars.iter()
        .enumerate()
        .map(|(ordinal, bar)| {
            let pct_chg = ordinal
                .checked_sub(1)
                .map(|previous| (bar.close - bars[previous].close) / bars[previous].close * 100.0)
                .unwrap_or(0.0);
            KlineData {
                date: bar.market_date,
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                volume: bar.volume,
                amount: bar.amount,
                pct_chg,
                intraday_price: None,
                settled: true,
                pe_ratio: None,
                pb_ratio: None,
                turnover_rate: None,
                market_cap: None,
                circulating_cap: None,
                eps: None,
                roe: None,
                revenue_yoy: None,
                net_profit_yoy: None,
                gross_margin: None,
                net_margin: None,
                sharpe_ratio: None,
                financials_history: None,
                valuation_history: None,
                consensus: None,
                industry: None,
                is_limit_up: false,
                is_limit_down: false,
                is_suspended: false,
                adjust: AdjustType::None,
            }
        })
        .collect()
}

fn select_window_records(
    expected_dates: &[NaiveDate],
    available: &BTreeMap<NaiveDate, (OutcomeProviderBarPreimage, OutcomeDailyBar)>,
) -> Result<(Vec<OutcomeProviderBarPreimage>, Vec<OutcomeDailyBar>), OutcomeWindowSelectionFailure>
{
    let Some(window_start) = expected_dates.first().copied() else {
        return Err(OutcomeWindowSelectionFailure {
            error: GatewayError::invalid_request(
                CAPABILITY,
                "expected outcome trading-date prefix cannot be empty",
            ),
            partial_preimages: Vec::new(),
        });
    };
    let window_end = *expected_dates.last().expect("checked non-empty");
    let mut preimages = Vec::with_capacity(expected_dates.len());
    let mut bars = Vec::with_capacity(expected_dates.len());
    for (_, (preimage, bar)) in available.range(window_start..=window_end) {
        let preimage = preimage.clone();
        let bar = bar.clone();
        preimages.push(preimage);
        bars.push(bar);
    }
    let actual_dates = bars
        .iter()
        .map(OutcomeDailyBar::market_date)
        .collect::<Vec<_>>();
    if actual_dates.as_slice() != expected_dates {
        return Err(OutcomeWindowSelectionFailure {
            error: GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Tdx),
                "partial",
                "settled_bar_missing",
                true,
                format!(
                    "Magic TDX exact outcome window mismatch start={window_start} end={window_end} \
                     expected_dates={expected_dates:?} actual_dates={actual_dates:?}",
                ),
            ),
            partial_preimages: preimages,
        });
    }
    Ok((preimages, bars))
}

fn validate_receipted_t0_baseline(
    phase: OutcomePhase,
    window_start: NaiveDate,
    receipted_t0_close: Option<&str>,
    receipted_t0_volume_shares: Option<&str>,
    bars: &[OutcomeDailyBar],
) -> Result<(), GatewayError> {
    let Some(first) = bars.first() else {
        return Err(invalid_evidence(
            "outcome window cannot validate a T0 baseline without bars",
        ));
    };
    if first.market_date != window_start {
        return Err(evidence_conflict(format!(
            "admitted first date {} differs from receipted T0 date {}",
            first.market_date, window_start
        )));
    }
    if phase == OutcomePhase::T0Close {
        return Ok(());
    }
    let expected_close = receipted_t0_close
        .ok_or_else(|| evidence_conflict("later outcome phase has no receipted T0 close"))?;
    let expected_volume = receipted_t0_volume_shares
        .ok_or_else(|| evidence_conflict("later outcome phase has no receipted T0 volume"))?;
    let actual_close = canonical_f64(first.close).map_err(schema_gateway_error)?;
    let actual_volume = canonical_f64(first.volume).map_err(schema_gateway_error)?;
    if actual_close != expected_close || actual_volume != expected_volume {
        return Err(evidence_conflict(format!(
            "Magic TDX admitted T0 baseline conflicts with receipt: \
             date={} close(provider={}, receipt={}) volume_shares(provider={}, receipt={})",
            first.market_date, actual_close, expected_close, actual_volume, expected_volume
        )));
    }
    Ok(())
}

fn validate_window_contract(
    phase: OutcomePhase,
    stored_due_date: NaiveDate,
    applicable_trading_dates: &[NaiveDate],
) -> Result<(), GatewayError> {
    let expected_count = match phase {
        OutcomePhase::T0Close => 1,
        OutcomePhase::D1Settled => 2,
        OutcomePhase::D3Settled => 4,
        OutcomePhase::D5Settled => 6,
    };
    if applicable_trading_dates.len() != expected_count {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            format!(
                "{} requires expected_bar_count={expected_count}, got {}",
                phase.as_str(),
                applicable_trading_dates.len()
            ),
        ));
    }
    let window_start = applicable_trading_dates[0];
    let window_end = *applicable_trading_dates
        .last()
        .expect("phase prefix is non-empty");
    if window_end != stored_due_date {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            "outcome window must end on the immutable stored due date",
        ));
    }
    if window_start > window_end
        || (expected_count == 1 && window_start != window_end)
        || (expected_count > 1 && window_start >= window_end)
        || applicable_trading_dates
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            "outcome immutable window endpoints conflict with its phase/count",
        ));
    }
    Ok(())
}

fn canonical_receipted_metric(
    field: &'static str,
    value: Option<&str>,
    required: bool,
) -> Result<Option<String>, GatewayError> {
    let Some(value) = value else {
        return if required {
            Err(GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Tdx),
                "partial",
                "evidence_conflict",
                false,
                format!("verified due is missing required receipted {field}"),
            ))
        } else {
            Ok(None)
        };
    };
    let parsed = value.parse::<f64>().map_err(|error| {
        GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tdx),
            "partial",
            "evidence_conflict",
            false,
            format!("receipted {field} {value:?} is not numeric: {error}"),
        )
    })?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tdx),
            "partial",
            "evidence_conflict",
            false,
            format!("receipted {field} must be positive and finite, got {value:?}"),
        ));
    }
    let canonical = canonical_f64(parsed).map_err(schema_gateway_error)?;
    if canonical != value {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tdx),
            "partial",
            "evidence_conflict",
            false,
            format!("receipted {field} is not canonical: stored={value:?} canonical={canonical:?}"),
        ));
    }
    Ok(Some(canonical))
}

fn outcome_instrument(code: &str, canonical_market: &str) -> Result<InstrumentId, GatewayError> {
    #[cfg(test)]
    let identity = if code.starts_with("TEST_CODE_") {
        super::instrument_identity::resolve_test_equity(code, None)
    } else {
        resolve_production_equity(code, None)
    };
    #[cfg(not(test))]
    let identity = resolve_production_equity(code, None);
    let identity = identity
        .and_then(|identity| {
            identity.require_a_share()?;
            Ok(identity)
        })
        .map_err(|error| {
            GatewayError::invalid_request(
                CAPABILITY,
                format!("invalid outcome equity identity {code:?}: {error}"),
            )
        })?;
    if identity.segment() == EquitySegment::BeijingA {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            "schema-v2 outcome Gateway currently supports Shanghai/Shenzhen A shares only",
        ));
    }
    let expected_market = match identity.exchange() {
        crate::market_domain::Exchange::Shanghai => "SH",
        crate::market_domain::Exchange::Shenzhen => "SZ",
        crate::market_domain::Exchange::Beijing => unreachable!("Beijing rejected above"),
    };
    if canonical_market != expected_market {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            format!(
                "verified due market {canonical_market:?} conflicts with canonical {expected_market}"
            ),
        ));
    }
    InstrumentId::new(
        identity.exchange(),
        identity.canonical_code(),
        AssetClass::Equity,
    )
    .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))
}

fn map_final_admission_error(error: GatewayError) -> GatewayError {
    GatewayError::classified(
        CAPABILITY,
        Some(ProviderId::Tdx),
        error.audit_outcome(),
        error.reason_code(),
        error.retryable(),
        format!("outcome daily-bar final admission failed: {error}"),
    )
}

fn invalid_evidence(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::Tdx), message)
}

fn evidence_conflict(message: impl Into<String>) -> GatewayError {
    GatewayError::classified(
        CAPABILITY,
        Some(ProviderId::Tdx),
        "partial",
        "evidence_conflict",
        false,
        message,
    )
}

fn schema_gateway_error(error: impl std::fmt::Display) -> GatewayError {
    invalid_evidence(format!("outcome canonical evidence rejected: {error}"))
}

fn admitted_volume_shares(core_volume_lots: f64) -> Result<f64, GatewayError> {
    let shares = core_volume_lots * SHARES_PER_BOARD_LOT;
    if !core_volume_lots.is_finite()
        || core_volume_lots <= 0.0
        || !shares.is_finite()
        || shares <= 0.0
    {
        return Err(invalid_evidence(
            "Magic TDX core volume cannot be converted to positive finite shares",
        ));
    }
    Ok(shares)
}

fn validate_unit_contract() -> Result<(), GatewayError> {
    if UPSTREAM_REVISION != VOLUME_CONTRACT_UPSTREAM_REVISION
        || PROVIDER_CONTRACT_VERSION != "magic-market-core.MarketDataProvider.bars.v0.2.0"
    {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tdx),
            "partial",
            "provider_unsupported",
            false,
            "Magic TDX bar volume/amount unit contract is not pinned to audited BR-022",
        ));
    }
    Ok(())
}
