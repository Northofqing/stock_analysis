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

use chrono::{Local, NaiveDate};
use magic_baidu_rs::{BaiduClient, BaiduError};
#[cfg(test)]
use magic_market_core::Exchange;
use magic_market_core::{
    Adjustment, AssetClass, Bar, BarInterval, BarsRequest, DataBatch, HistoricalBars, InstrumentId,
    ProviderId,
};
use magic_market_router::{
    AcceptancePolicy, AttemptStatus, BarsRouter, FailureKind, RouterError, SourceError, SourceFn,
};
use magic_sina_rs::{SinaClient, SinaError};
use magic_tdx_rs::protocol::constants::{fq_type, KLINE_15MIN};
use magic_tdx_rs::protocol::types::SecurityBar;
use magic_tdx_rs::{TdxError, TdxSmartClient};
use magic_tencent_rs::{TencentClient, TencentError};
use std::sync::Arc;

use crate::data_provider::{AdjustType, KlineData};
use crate::database::daily_change_confirmation::DailyChangeConfirmationQuery;
use crate::database::DatabaseManager;
use crate::monitor::data_quality::{
    validate_daily_freshness, validate_daily_kline_quality_with_confirmation,
    validate_daily_kline_structure, AdjacentDailyChange, DqStats, FreshnessConfig,
    MAX_UNCONFIRMED_ADJACENT_DAILY_CHANGE_PCT,
};

use super::instrument_identity::{resolve_production_equity, EquitySegment};
use super::review::{
    acquisition_request_hash, audit_gateway_result, BatchEvidence, GatewayBatch, GatewayError,
};
use super::security_lifecycle::{
    CorporateActionState, LifecycleConfirmationEvidence, ListingDateState,
    SecurityLifecycleContext, SecurityLifecycleGateway,
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
    /// 历史信号 (7/14 起, 800 根 ≈ 50 交易日)。走 magic-tdx 直连
    /// (与 magic_tdx_t0 同构), 复用进程级 cached_tdx_hq_client 连接,
    /// 不参与 daily-bars router (router 只承载日K语义)。
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
        // P4 M3: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; 本地无 audit,
        // 桥路径亦不 audit — 与本地行为一致)。
        match super::grpc_source::bridge_for("TechnicalBars") {
            Ok(Some(bridge)) => {
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
            Ok(None) => {}
            Err(error) => {
                return Err(GatewayError::unavailable(CAPABILITY, None, true, error.to_string()));
            }
        }
        let market = if code.starts_with('6') { 1u8 } else { 0u8 };
        let client = super::magic_tdx_t0::cached_tdx_hq_client().map_err(|error| {
            GatewayError::unavailable(CAPABILITY, None, true, error.to_string())
        })?;
        let bars = client
            .get_security_bars(
                KLINE_15MIN,
                market,
                code,
                0,
                count as u16,
                fq_type::NONE,
            )
            .map_err(|error| {
                GatewayError::unavailable(
                    CAPABILITY,
                    None,
                    true,
                    format!("magic-tdx 15min bars failed for {code}: {error}"),
                )
            })?;
        if bars.is_empty() {
            return Err(GatewayError::unavailable(
                CAPABILITY,
                None,
                false,
                format!("magic-tdx 15min bars empty for {code}"),
            ));
        }
        Ok(bars)
    }

    pub fn daily_bars(&self, code: &str, days: usize) -> Result<AdmittedDailyBars, GatewayError> {
        let (request_hash, terminal_provider, result) = acquire_structural_batch(code, days);
        // P4 M2 钩子: DATA_GATEWAY_GRPC=1 → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("HistoricalBars") {
            Ok(Some(bridge)) => {
                let result = bridge.daily_bars(code, days);
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                let audited =
                    audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result)?;
                return AdmittedDailyBars::from_audited_batch(code.to_owned(), audited);
            }
            Ok(None) => {}
            Err(error) => {
                let audited =
                    audit_gateway_result(CAPABILITY, ProviderId::Tdx, &request_hash, Err(error))?;
                return AdmittedDailyBars::from_audited_batch(code.to_owned(), audited);
            }
        }
        let result = result.and_then(|mut batch| {
            finalize_selected_batch_sync(code, &mut batch)?;
            Ok(batch)
        });
        let audited = audit_gateway_result(CAPABILITY, terminal_provider, &request_hash, result)?;
        AdmittedDailyBars::from_audited_batch(code.to_owned(), audited)
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
        let worker_code = code.clone();
        let (request_hash, terminal_provider, result) =
            tokio::task::spawn_blocking(move || acquire_structural_batch(&worker_code, days))
                .await
                .map_err(|error| {
                    GatewayError::unavailable(
                        CAPABILITY,
                        None,
                        true,
                        format!("historical-bars blocking task failed: {error}"),
                    )
                })?;
        // P4 M2 钩子: DATA_GATEWAY_GRPC=1 → gRPC 通道 (async 路径, 不 block_on)。
        match super::grpc_source::bridge_for("HistoricalBars") {
            Ok(Some(bridge)) => {
                let result = bridge.daily_bars_async(&code, days).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                let audited =
                    audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result)?;
                return AdmittedDailyBars::from_audited_batch(code, audited);
            }
            Ok(None) => {}
            Err(error) => {
                let audited =
                    audit_gateway_result(CAPABILITY, ProviderId::Tdx, &request_hash, Err(error))?;
                return AdmittedDailyBars::from_audited_batch(code, audited);
            }
        }
        let result = finalize_selected_batch_async(code.clone(), result).await;
        let audited = tokio::task::spawn_blocking(move || {
            audit_gateway_result(CAPABILITY, terminal_provider, &request_hash, result)
        })
        .await
        .map_err(|error| {
            GatewayError::unavailable(
                CAPABILITY,
                Some(terminal_provider),
                true,
                format!("historical-bars audit task failed: {error}"),
            )
        })??;
        AdmittedDailyBars::from_audited_batch(code, audited)
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
        let worker_code = code.clone();
        let (request_hash, terminal_provider, result) =
            tokio::task::spawn_blocking(move || acquire_structural_batch(&worker_code, days))
                .await
                .map_err(|error| {
                    GatewayError::unavailable(
                        CAPABILITY,
                        None,
                        true,
                        format!("historical-bars review task failed: {error}"),
                    )
                })?;
        let mut batch = tokio::task::spawn_blocking(move || {
            audit_gateway_result(CAPABILITY, terminal_provider, &request_hash, result)
        })
        .await
        .map_err(|error| {
            GatewayError::unavailable(
                CAPABILITY,
                Some(terminal_provider),
                true,
                format!("historical-bars review audit task failed: {error}"),
            )
        })??;
        if pending_changes(&code, &mut batch)?.is_empty() {
            return Ok(Vec::new());
        }
        let (window_start, window_end) = batch_window(&batch)?;
        let lifecycle = SecurityLifecycleGateway::new()
            .acquire(&code, window_start, window_end)
            .await?;
        tokio::task::spawn_blocking(move || {
            confirmation_queries_for_pending_batch(&code, &mut batch, &lifecycle)
        })
        .await
        .map_err(|error| {
            GatewayError::unavailable(
                CAPABILITY,
                Some(terminal_provider),
                true,
                format!("historical-bars review projection task failed: {error}"),
            )
        })?
    }
}

fn acquire_structural_batch(
    code: &str,
    days: usize,
) -> (
    String,
    ProviderId,
    Result<GatewayBatch<KlineData>, GatewayError>,
) {
    let request_hash = acquisition_request_hash(CAPABILITY, &format!("{code}:{days}"));
    let request = match build_request(code, days) {
        Ok(request) => request,
        Err(error) => return (request_hash, ProviderId::Tdx, Err(error)),
    };
    let (provider, result) = route_daily_bars(code, &request);
    (request_hash, provider, result)
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

fn build_request(code: &str, days: usize) -> Result<BarsRequest, GatewayError> {
    let storage_code = code;
    #[cfg(test)]
    let identity = if storage_code.starts_with("TEST_CODE_") {
        super::instrument_identity::resolve_test_equity(storage_code, None)
    } else {
        resolve_production_equity(storage_code, None)
    };
    #[cfg(not(test))]
    let identity = resolve_production_equity(storage_code, None);
    let identity = identity
        .and_then(|identity| {
            identity.require_a_share()?;
            Ok(identity)
        })
        .map_err(|error| {
            GatewayError::invalid_request(
                CAPABILITY,
                format!("invalid historical-bars equity identity {storage_code:?}: {error}"),
            )
        })?;
    if identity.segment() == EquitySegment::BeijingA
        && !identity.canonical_code().starts_with("920")
    {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            format!("daily-bar providers have no verified capability for {storage_code:?}"),
        ));
    }
    let limit = u16::try_from(days).map_err(|_| {
        GatewayError::invalid_request(CAPABILITY, format!("daily-bar limit {days} exceeds u16"))
    })?;
    let instrument = InstrumentId::new(
        identity.exchange(),
        identity.canonical_code(),
        AssetClass::Equity,
    )
    .map_err(|error| {
        GatewayError::invalid_request(
            CAPABILITY,
            format!("validated instrument {storage_code:?} failed core invariant: {error}"),
        )
    })?;
    BarsRequest::new(instrument, BarInterval::Day, limit)
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))
}

fn route_daily_bars(
    storage_code: &str,
    request: &BarsRequest,
) -> (ProviderId, Result<GatewayBatch<KlineData>, GatewayError>) {
    let tencent = match TencentClient::new() {
        Ok(client) => Arc::new(client),
        Err(error) => {
            return (
                ProviderId::Tencent,
                Err(provider_initialization_error(
                    ProviderId::Tencent,
                    error.to_string(),
                )),
            );
        }
    };
    let sina = match SinaClient::new() {
        Ok(client) => Arc::new(client),
        Err(error) => {
            return (
                ProviderId::Sina,
                Err(provider_initialization_error(
                    ProviderId::Sina,
                    error.to_string(),
                )),
            );
        }
    };
    let baidu = match BaiduClient::new() {
        Ok(client) => Arc::new(client),
        Err(error) => {
            return (
                ProviderId::Baidu,
                Err(provider_initialization_error(
                    ProviderId::Baidu,
                    error.to_string(),
                )),
            );
        }
    };

    let mut router = BarsRouter::new(
        AcceptancePolicy::new()
            .with_require_complete(true)
            .with_require_source_at(true),
    );
    let registration = router
        .register(validated_source(
            ProviderId::Tdx,
            Arc::new(TdxSmartClient::new()),
            classify_tdx_error,
            storage_code,
        ))
        .and_then(|router| {
            router.register(validated_source(
                ProviderId::Tencent,
                tencent,
                classify_tencent_error,
                storage_code,
            ))
        })
        .and_then(|router| {
            router.register(validated_source(
                ProviderId::Sina,
                sina,
                classify_sina_error,
                storage_code,
            ))
        })
        .and_then(|router| {
            router.register(validated_source(
                ProviderId::Baidu,
                baidu,
                classify_baidu_error,
                storage_code,
            ))
        });
    if let Err(error) = registration {
        return (
            ProviderId::Tdx,
            Err(router_gateway_error(error, ProviderId::Tdx)),
        );
    }

    match router.route(request) {
        Ok(outcome) => {
            let provider = outcome.selected_provider();
            let batch = outcome.into_batch();
            (
                provider,
                project_selected_batch(storage_code, request, provider, batch),
            )
        }
        Err(error) => {
            let provider = error
                .attempts()
                .last()
                .map(|attempt| attempt.provider_id())
                .unwrap_or(ProviderId::Tdx);
            (provider, Err(router_gateway_error(error, provider)))
        }
    }
}

fn validated_source<Provider, Classify>(
    provider_id: ProviderId,
    provider: Arc<Provider>,
    classify: Classify,
    storage_code: &str,
) -> SourceFn<BarsRequest, Bar>
where
    Provider: HistoricalBars<Bar = Bar> + Send + Sync + 'static,
    Classify: Fn(Provider::Error) -> SourceError + Send + Sync + Clone + 'static,
{
    let storage_code = storage_code.to_owned();
    SourceFn::new(provider_id, move |request| {
        let batch = provider
            .historical_bars(request)
            .map_err(classify.clone())?;
        validate_candidate_batch(&storage_code, request, provider_id, &batch)?;
        Ok(batch)
    })
}

fn validate_candidate_batch(
    storage_code: &str,
    request: &BarsRequest,
    provider: ProviderId,
    batch: &DataBatch<Bar>,
) -> Result<Vec<KlineData>, SourceError> {
    if !batch.quality().is_complete() {
        return Err(SourceError::try_next(
            FailureKind::Quality,
            format!(
                "{provider:?} daily batch is partial: {}",
                batch.quality().issues().join("; ")
            ),
        ));
    }
    let provenance_source_at = batch.provenance().source_at().ok_or_else(|| {
        SourceError::try_next(
            FailureKind::Evidence,
            format!("{provider:?} daily batch source_at is missing"),
        )
    })?;
    let batch_id = batch.provenance().batch_id().ok_or_else(|| {
        SourceError::try_next(
            FailureKind::Evidence,
            format!("{provider:?} daily batch_id is missing"),
        )
    })?;
    if batch.records().len() != usize::from(request.limit()) {
        return Err(SourceError::try_next(
            FailureKind::Quality,
            format!(
                "{provider:?} daily cardinality mismatch requested={} actual={}",
                request.limit(),
                batch.records().len()
            ),
        ));
    }

    let mut output = Vec::with_capacity(batch.records().len());
    let mut previous_close = None;
    for bar in batch.records() {
        if bar.instrument() != request.instrument()
            || bar.interval() != BarInterval::Day
            || bar.provider() != provider
            || bar.batch_id() != batch_id
        {
            return Err(SourceError::try_next(
                FailureKind::Evidence,
                format!("{provider:?} daily record identity/evidence mismatch"),
            ));
        }
        let source_at = bar.source_at().ok_or_else(|| {
            SourceError::try_next(
                FailureKind::Evidence,
                format!("{provider:?} daily record source_at is missing"),
            )
        })?;
        let date = NaiveDate::parse_from_str(bar.bar_end(), "%Y-%m-%d").map_err(|error| {
            SourceError::try_next(
                FailureKind::Protocol,
                format!("{provider:?} invalid daily bar date: {error}"),
            )
        })?;
        if source_at != bar.bar_end() {
            return Err(SourceError::try_next(
                FailureKind::Evidence,
                format!(
                    "{provider:?} daily source_at {source_at:?} differs from bar date {:?}",
                    bar.bar_end()
                ),
            ));
        }
        let amount = bar.amount().ok_or_else(|| {
            SourceError::try_next(
                FailureKind::Quality,
                format!("{provider:?} daily amount is unavailable at {date}"),
            )
        })?;
        let adjust = match bar.adjustment() {
            Adjustment::Unadjusted => AdjustType::None,
            Adjustment::Forward => AdjustType::Qfq,
            Adjustment::Backward => {
                return Err(SourceError::try_next(
                    FailureKind::Unsupported,
                    format!("{provider:?} backward-adjusted daily bars are unsupported"),
                ))
            }
        };
        let close = bar.close().get();
        let pct_chg = previous_close
            .map(|previous: f64| (close - previous) / previous * 100.0)
            .unwrap_or(0.0);
        previous_close = Some(close);
        output.push(KlineData {
            date,
            open: bar.open().get(),
            high: bar.high().get(),
            low: bar.low().get(),
            close,
            volume: bar.volume().get(),
            amount: amount.get(),
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
            adjust,
        });
    }
    if output
        .last()
        .is_none_or(|latest| latest.date.to_string() != provenance_source_at)
    {
        return Err(SourceError::try_next(
            FailureKind::Evidence,
            format!(
                "{provider:?} batch source_at {provenance_source_at:?} does not identify latest bar"
            ),
        ));
    }
    let _pending_confirmations = validate_daily_kline_structure(&mut output, storage_code)
        .map_err(|error| {
            SourceError::try_next(
                FailureKind::Quality,
                format!("{provider:?} BR-092 rejected daily batch: {error}"),
            )
        })?;
    let latest = output.first().ok_or_else(|| {
        SourceError::try_next(
            FailureKind::NoData,
            format!("{provider:?} daily batch is empty after validation"),
        )
    })?;
    validate_daily_freshness(
        latest.date,
        Local::now(),
        &FreshnessConfig::default(),
        &DqStats::new(),
    )
    .map_err(|reason| {
        SourceError::try_next(
            FailureKind::Quality,
            format!(
                "{provider:?} latest daily bar {} failed one-trading-day freshness: {}",
                latest.date,
                reason.label()
            ),
        )
    })?;
    Ok(output)
}

fn project_selected_batch(
    storage_code: &str,
    request: &BarsRequest,
    provider: ProviderId,
    batch: DataBatch<Bar>,
) -> Result<GatewayBatch<KlineData>, GatewayError> {
    let evidence = BatchEvidence::from_provenance(provider, batch.provenance())?;
    let records =
        validate_candidate_batch(storage_code, request, provider, &batch).map_err(|error| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                format!("selected daily batch failed final admission: {error}"),
            )
        })?;
    Ok(GatewayBatch::Available { records, evidence })
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

fn pending_changes(
    code: &str,
    batch: &mut GatewayBatch<KlineData>,
) -> Result<Vec<AdjacentDailyChange>, GatewayError> {
    match batch {
        GatewayBatch::Available { records, evidence } => {
            validate_daily_kline_structure(records, code)
                .map_err(|error| final_admission_error(evidence.provider, error))
        }
        GatewayBatch::VerifiedEmpty(evidence) => Err(GatewayError::unavailable(
            CAPABILITY,
            Some(evidence.provider),
            true,
            "historical daily bars cannot be verified empty",
        )),
    }
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

fn confirmation_queries_for_pending_batch(
    code: &str,
    batch: &mut GatewayBatch<KlineData>,
    lifecycle: &SecurityLifecycleContext,
) -> Result<Vec<DailyChangeConfirmationQuery>, GatewayError> {
    let daily_evidence = batch.evidence().clone();
    let provider = daily_evidence.provider;
    pending_changes(code, batch)?
        .into_iter()
        .map(|change| {
            let lifecycle_evidence = lifecycle
                .confirmation_evidence_for(change.previous_date, change.current_date)
                .map_err(|error| final_admission_error(provider, error.to_string()))?;
            build_confirmation_query(&change, &daily_evidence, &lifecycle_evidence)
                .map_err(|error| final_admission_error(provider, error))
        })
        .collect()
}

fn finalize_with_lifecycle(
    code: &str,
    batch: &mut GatewayBatch<KlineData>,
    lifecycle: &SecurityLifecycleContext,
) -> Result<(), GatewayError> {
    let daily_evidence = batch.evidence().clone();
    let provider = daily_evidence.provider;
    let records = match batch {
        GatewayBatch::Available { records, .. } => records,
        GatewayBatch::VerifiedEmpty(_) => {
            return Err(GatewayError::unavailable(
                CAPABILITY,
                Some(provider),
                true,
                "historical daily bars cannot be verified empty",
            ))
        }
    };
    validate_daily_kline_quality_with_confirmation(records, code, |change| {
        // BR-229: 板内涨停跳空自动确认 — 相邻跳空恰为板内涨停幅
        // (创业板/科创板 20%, 北交所 30%, 主板 10%, 容差 ±0.5%) 是市场事实
        // (涨停日), 不是除权/数据错误, 无需 DB 确认; BR-171 只针对需
        // 生命周期证据的跳空 (送转/除权等)。
        if change.change_pct > 0.0 {
            if let Some(limit_pct) = board_limit_up_pct_for_code(&change.code) {
                if (change.change_pct - limit_pct).abs() <= 0.5 {
                    return Ok(true);
                }
            }
        }
        let lifecycle = lifecycle
            .confirmation_evidence_for(change.previous_date, change.current_date)
            .map_err(|error| error.to_string())?;
        let query = build_confirmation_query(change, &daily_evidence, &lifecycle)?;
        let database = DatabaseManager::try_get()
            .ok_or_else(|| "BR-171 confirmation database is not initialized".to_string())?;
        database.has_exact_daily_change_confirmation(&query)
    })
    .map_err(|error| final_admission_error(provider, error))
}

/// BR-229: 按代码前缀判定板别涨停幅 (创业板/科创板 20%, 北交所 30%, 主板 10%)。
fn board_limit_up_pct_for_code(code: &str) -> Option<f64> {
    if code.starts_with("300") || code.starts_with("301") || code.starts_with("688") {
        Some(20.0)
    } else if code.starts_with('8') || code.starts_with('4') || code.starts_with("920") {
        Some(30.0)
    } else if code.starts_with("60") || code.starts_with("00") || code.starts_with("001") {
        Some(10.0)
    } else {
        None
    }
}

fn finalize_without_pending_change(
    code: &str,
    batch: &mut GatewayBatch<KlineData>,
) -> Result<(), GatewayError> {
    let provider = batch.evidence().provider;
    let records = match batch {
        GatewayBatch::Available { records, .. } => records,
        GatewayBatch::VerifiedEmpty(_) => {
            return Err(GatewayError::unavailable(
                CAPABILITY,
                Some(provider),
                true,
                "historical daily bars cannot be verified empty",
            ))
        }
    };
    validate_daily_kline_quality_with_confirmation(records, code, |_| Ok(false))
        .map_err(|error| final_admission_error(provider, error))
}

fn acquire_lifecycle_sync(
    code: &str,
    window_start: NaiveDate,
    window_end: NaiveDate,
) -> Result<SecurityLifecycleContext, GatewayError> {
    let code = code.to_owned();
    std::thread::Builder::new()
        .name("security-lifecycle-gateway".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    GatewayError::unavailable(
                        CAPABILITY,
                        Some(ProviderId::Tdx),
                        true,
                        format!("cannot build isolated lifecycle runtime: {error}"),
                    )
                })?;
            runtime.block_on(SecurityLifecycleGateway::new().acquire(
                &code,
                window_start,
                window_end,
            ))
        })
        .map_err(|error| {
            GatewayError::unavailable(
                CAPABILITY,
                Some(ProviderId::Tdx),
                true,
                format!("cannot spawn isolated lifecycle worker: {error}"),
            )
        })?
        .join()
        .map_err(|_| {
            GatewayError::unavailable(
                CAPABILITY,
                Some(ProviderId::Tdx),
                true,
                "isolated lifecycle worker panicked",
            )
        })?
}

fn finalize_selected_batch_sync(
    code: &str,
    batch: &mut GatewayBatch<KlineData>,
) -> Result<(), GatewayError> {
    if pending_changes(code, batch)?.is_empty() {
        return finalize_without_pending_change(code, batch);
    }
    let (window_start, window_end) = batch_window(batch)?;
    let lifecycle = acquire_lifecycle_sync(code, window_start, window_end)?;
    finalize_with_lifecycle(code, batch, &lifecycle)
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

async fn finalize_selected_batch_async(
    code: String,
    result: Result<GatewayBatch<KlineData>, GatewayError>,
) -> Result<GatewayBatch<KlineData>, GatewayError> {
    let mut batch = result?;
    if pending_changes(&code, &mut batch)?.is_empty() {
        finalize_without_pending_change(&code, &mut batch)?;
        return Ok(batch);
    }
    let (window_start, window_end) = batch_window(&batch)?;
    let lifecycle = SecurityLifecycleGateway::new()
        .acquire(&code, window_start, window_end)
        .await?;
    tokio::task::spawn_blocking(move || {
        finalize_with_lifecycle(&code, &mut batch, &lifecycle)?;
        Ok(batch)
    })
    .await
    .map_err(|error| {
        GatewayError::unavailable(
            CAPABILITY,
            None,
            true,
            format!("historical-bars confirmation task failed: {error}"),
        )
    })?
}

fn provider_initialization_error(provider: ProviderId, message: String) -> GatewayError {
    GatewayError::classified(
        CAPABILITY,
        Some(provider),
        "unavailable",
        "provider_initialization_failed",
        true,
        format!("{provider:?} client initialization failed: {message}"),
    )
}

fn router_gateway_error(error: RouterError, provider: ProviderId) -> GatewayError {
    let attempts = error
        .attempts()
        .iter()
        .map(|attempt| {
            let status = match attempt.status() {
                AttemptStatus::Selected => "selected".to_owned(),
                AttemptStatus::Rejected { kind, message } => {
                    format!("rejected:{kind:?}:{message}")
                }
                AttemptStatus::Failed {
                    kind,
                    action,
                    message,
                } => format!("failed:{kind:?}:{action:?}:{message}"),
            };
            format!("{:?}={status}", attempt.provider_id())
        })
        .collect::<Vec<_>>()
        .join(",");
    let last_kind = error
        .attempts()
        .last()
        .and_then(|attempt| match attempt.status() {
            AttemptStatus::Selected => None,
            AttemptStatus::Rejected { kind, .. } | AttemptStatus::Failed { kind, .. } => {
                Some(*kind)
            }
        });
    let (outcome, reason_code, retryable) = match last_kind {
        None | Some(FailureKind::InvalidRequest | FailureKind::Unsupported) => {
            ("invalid_request", "router_request_rejected", false)
        }
        Some(
            FailureKind::Transport
            | FailureKind::Timeout
            | FailureKind::RateLimited
            | FailureKind::Provider
            | FailureKind::NoData,
        ) => ("unavailable", "router_sources_exhausted", true),
        Some(FailureKind::Protocol | FailureKind::Quality | FailureKind::Evidence) => {
            ("partial", "router_batch_rejected", false)
        }
    };
    GatewayError::classified(
        CAPABILITY,
        Some(provider),
        outcome,
        reason_code,
        retryable,
        format!("{error}; attempts=[{attempts}]"),
    )
}

fn classify_tdx_error(error: TdxError) -> SourceError {
    let message = error.to_string();
    match error {
        TdxError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        TdxError::Io(_)
        | TdxError::Connection(_)
        | TdxError::ConnectionTimeout
        | TdxError::SetupFailed(_)
        | TdxError::Disconnected
        | TdxError::RetryExhausted(_) => SourceError::try_next(FailureKind::Transport, message),
        TdxError::HistoricalBarCardinality {
            offset,
            actual,
            expected_page,
            requested_total,
        } => SourceError::try_next(
            FailureKind::Protocol,
            format!(
                "Magic TDX historical-bar cardinality mismatch: offset={offset} actual={actual} \
                 expected_page={expected_page} requested_total={requested_total}"
            ),
        ),
        TdxError::Parse(_)
        | TdxError::InvalidData(_)
        | TdxError::ResponseParse(_)
        | TdxError::Core(_)
        | TdxError::Coded(_)
        | TdxError::FileNotFound(_) => SourceError::try_next(FailureKind::Protocol, message),
    }
}

fn classify_tencent_error(error: TencentError) -> SourceError {
    let message = error.to_string();
    match error {
        TencentError::InvalidRequest(_) => SourceError::stop(FailureKind::InvalidRequest, message),
        TencentError::Transport(_) => SourceError::try_next(FailureKind::Transport, message),
        TencentError::Decode(_) | TencentError::Protocol(_) => {
            SourceError::try_next(FailureKind::Protocol, message)
        }
        TencentError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        TencentError::Core(_) => SourceError::try_next(FailureKind::Evidence, message),
    }
}

fn classify_sina_error(error: SinaError) -> SourceError {
    let message = error.to_string();
    match error {
        SinaError::InvalidRequest(_) => SourceError::stop(FailureKind::InvalidRequest, message),
        SinaError::Transport(_) => SourceError::try_next(FailureKind::Transport, message),
        SinaError::Decode(_) | SinaError::Protocol(_) => {
            SourceError::try_next(FailureKind::Protocol, message)
        }
        SinaError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        SinaError::Core(_) => SourceError::try_next(FailureKind::Evidence, message),
    }
}

fn classify_baidu_error(error: BaiduError) -> SourceError {
    let message = error.to_string();
    match error {
        BaiduError::InvalidRequest(_) => SourceError::stop(FailureKind::InvalidRequest, message),
        BaiduError::Transport(_) => SourceError::try_next(FailureKind::Transport, message),
        BaiduError::Decode(_) | BaiduError::Protocol(_) => {
            SourceError::try_next(FailureKind::Protocol, message)
        }
        BaiduError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        BaiduError::Core(_) => SourceError::try_next(FailureKind::Evidence, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::security_lifecycle::{
        AdmittedListingDate, CorporateActionState, ListingDateState,
    };
    use magic_market_core::{Money, Price, Provenance, Quantity};

    fn bar(
        date: &str,
        close: f64,
        provider: ProviderId,
        batch_id: &str,
        amount: Option<f64>,
    ) -> Bar {
        Bar::new(
            InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap(),
            BarInterval::Day,
            date,
            date,
            Price::new(close).unwrap(),
            Price::new(close).unwrap(),
            Price::new(close).unwrap(),
            Price::new(close).unwrap(),
            Quantity::new(100.0).unwrap(),
            amount.map(|value| Money::new(value).unwrap()),
            Adjustment::Unadjusted,
            provider,
            batch_id,
        )
        .unwrap()
        .with_source_at(date)
        .unwrap()
    }

    fn batch(provider: ProviderId, amount: Option<f64>) -> DataBatch<Bar> {
        batch_with_closes(provider, amount, 10.0, 10.1)
    }

    fn batch_with_closes(
        provider: ProviderId,
        amount: Option<f64>,
        previous_close: f64,
        current_close: f64,
    ) -> DataBatch<Bar> {
        let current_date =
            crate::calendar::latest_completed_trading_day_at(Local::now().naive_local());
        let previous_date = crate::calendar::prev_trading_day(current_date);
        let current_date_text = current_date.to_string();
        let previous_date_text = previous_date.to_string();
        let batch_id = "TEST_CODE_daily_batch";
        let bars = vec![
            bar(
                &previous_date_text,
                previous_close,
                provider,
                batch_id,
                amount,
            ),
            bar(
                &current_date_text,
                current_close,
                provider,
                batch_id,
                amount,
            ),
        ];
        let provenance = Provenance::new("TEST_CODE_provider", Local::now().to_rfc3339())
            .unwrap()
            .with_source_at(current_date_text)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        DataBatch::strict(bars, provenance)
    }

    #[test]
    fn br173_request_uses_canonical_a_share_identity() {
        assert!(build_request("TEST_CODE_600396", 0).is_err());
        assert!(build_request("TEST_CODE_600396", usize::from(u16::MAX) + 1).is_err());
        assert!(build_request("TEST_CODE_BAD", 2).is_err());
        assert!(build_request("TEST_CODE_600396", 2).is_ok());
        assert!(build_request("TEST_CODE_000001", 2).is_ok());
        assert!(build_request("TEST_CODE_920118", 2).is_ok());
        assert!(build_request("TEST_CODE_921001", 2).is_err());
        assert!(build_request("TEST_CODE_929999", 2).is_err());
        assert!(build_request("TEST_CODE_430047", 2).is_err());
        assert!(build_request("TEST_CODE_830001", 2).is_err());
        assert!(build_request("TEST_CODE_900001", 2).is_err());
        assert!(build_request("TEST_CODE_200001", 2).is_err());
        assert!(build_request("TEST_CODE_100001", 2).is_err());
    }

    #[test]
    fn br164_complete_batch_keeps_evidence_and_passes_br092() {
        let request = build_request("TEST_CODE_600396", 2).unwrap();
        let output = validate_candidate_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Baidu,
            &batch(ProviderId::Baidu, Some(1_000.0)),
        )
        .unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(
            output[0].date,
            crate::calendar::latest_completed_trading_day_at(Local::now().naive_local())
        );
        assert_eq!(output[0].amount, 1_000.0);
    }

    #[test]
    fn br164_missing_amount_and_cardinality_are_explicit_rejections() {
        let request = build_request("TEST_CODE_600396", 2).unwrap();
        let missing_amount = validate_candidate_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Tencent,
            &batch(ProviderId::Tencent, None),
        )
        .unwrap_err();
        assert_eq!(missing_amount.kind(), FailureKind::Quality);

        let request = build_request("TEST_CODE_600396", 3).unwrap();
        let partial = validate_candidate_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Baidu,
            &batch(ProviderId::Baidu, Some(1_000.0)),
        )
        .unwrap_err();
        assert_eq!(partial.kind(), FailureKind::Quality);
    }

    #[test]
    fn br171_router_keeps_structurally_valid_source_but_final_admission_requires_confirmation() {
        let request = build_request("TEST_CODE_600396", 2).unwrap();
        let source_batch = batch_with_closes(ProviderId::Tdx, Some(1_000.0), 10.0, 13.0);

        let structurally_valid =
            validate_candidate_batch("TEST_CODE_600396", &request, ProviderId::Tdx, &source_batch)
                .expect("large source-backed move is not a provider-routing failure");
        assert_eq!(structurally_valid.len(), 2);

        let mut projected =
            project_selected_batch("TEST_CODE_600396", &request, ProviderId::Tdx, source_batch)
                .expect("TEST_CODE structurally admitted batch");
        let error = finalize_without_pending_change("TEST_CODE_600396", &mut projected)
            .expect_err("unconfirmed large move must fail final admission");
        assert_eq!(error.reason_code(), "manual_confirmation_required");
    }

    #[test]
    fn br171_exact_confirmation_query_binds_daily_and_lifecycle_evidence() {
        let daily = BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_tdx-smart".to_string(),
            source_at: Some("2026-07-24".to_string()),
            observed_at: "2026-07-24T15:01:00+08:00".to_string(),
            batch_id: "TEST_CODE_daily_batch".to_string(),
        };
        let lifecycle = LifecycleConfirmationEvidence {
            provider: "TEST_CODE_magic_tdx".to_string(),
            batch_identity: "TEST_CODE_window=2026-07-23:2026-07-24|listing=L1|actions=A1"
                .to_string(),
            listing_date: Some(NaiveDate::from_ymd_opt(2001, 8, 27).unwrap()),
            corporate_action_identity: Some("TEST_CODE_action_hash".to_string()),
        };
        let query = build_confirmation_query(
            &AdjacentDailyChange {
                code: "TEST_CODE_600396".to_string(),
                previous_date: NaiveDate::from_ymd_opt(2026, 7, 23).unwrap(),
                current_date: NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
                previous_close: 10.0,
                current_close: 13.0,
                change_pct: 30.0,
            },
            &daily,
            &lifecycle,
        )
        .expect("TEST_CODE exact confirmation query");
        assert_eq!(query.daily_provider, "magic_tdx");
        assert_eq!(query.daily_batch_id, "TEST_CODE_daily_batch");
        assert_eq!(query.lifecycle_batch_id, lifecycle.batch_identity);
        assert_eq!(
            query.corporate_action_identity.as_deref(),
            Some("TEST_CODE_action_hash")
        );
        assert_eq!(query.calculated_pct, "30");
    }

    #[test]
    fn br171_review_projection_returns_exact_pending_query_without_admitting_batch() {
        let current_date =
            crate::calendar::latest_completed_trading_day_at(Local::now().naive_local());
        let previous_date = crate::calendar::prev_trading_day(current_date);
        let observed_at = Local::now().to_rfc3339();
        let request = build_request("TEST_CODE_600396", 2).unwrap();
        let mut selected = project_selected_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Tdx,
            batch_with_closes(ProviderId::Tdx, Some(1_000.0), 10.0, 13.0),
        )
        .expect("TEST_CODE selected structural batch");
        let listing_evidence = BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_tdx".to_string(),
            source_at: Some(current_date.to_string()),
            observed_at: observed_at.clone(),
            batch_id: "TEST_CODE_listing_batch".to_string(),
        };
        let action_evidence = BatchEvidence {
            batch_id: "TEST_CODE_actions_batch".to_string(),
            ..listing_evidence.clone()
        };
        let lifecycle = SecurityLifecycleContext {
            instrument: InstrumentId::new(
                Exchange::Shanghai,
                "TEST_CODE_600396",
                AssetClass::Equity,
            )
            .unwrap(),
            window_start: previous_date,
            window_end: current_date,
            listing: ListingDateState::Available(AdmittedListingDate {
                listed_on: NaiveDate::from_ymd_opt(2010, 1, 1).unwrap(),
                evidence: listing_evidence,
            }),
            corporate_actions: CorporateActionState::VerifiedEmpty(action_evidence),
        };

        let queries =
            confirmation_queries_for_pending_batch("TEST_CODE_600396", &mut selected, &lifecycle)
                .expect("TEST_CODE review queries");

        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].previous_close, "10");
        assert_eq!(queries[0].current_close, "13");
        assert_eq!(queries[0].calculated_pct, "30");
        assert_eq!(queries[0].daily_batch_id, "TEST_CODE_daily_batch");
        assert!(queries[0]
            .lifecycle_batch_id
            .contains("actions=TEST_CODE_actions_batch"));
    }

    #[test]
    fn br171_outcome_sequence_gates_t0_to_d1_without_calendar_reconstruction() {
        let request = build_request("TEST_CODE_600396", 2).unwrap();
        let mut selected = project_selected_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Tdx,
            batch_with_closes(ProviderId::Tdx, Some(1_000.0), 10.0, 13.0),
        )
        .expect("TEST_CODE provider-ordered T0..D1 sequence");
        if let GatewayBatch::Available { records, .. } = &mut selected {
            records.reverse();
        }

        let pending =
            outcome_pending_changes("TEST_CODE_600396", &selected).expect("TEST_CODE BR-171 gate");

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].previous_close, 10.0);
        assert_eq!(pending[0].current_close, 13.0);
        assert_eq!(pending[0].change_pct, 30.0);
    }

    #[test]
    fn outcome_lifecycle_is_admitted_even_without_large_adjacent_move() {
        let current_date =
            crate::calendar::latest_completed_trading_day_at(Local::now().naive_local());
        let previous_date = crate::calendar::prev_trading_day(current_date);
        let request = build_request("TEST_CODE_600396", 2).unwrap();
        let mut selected = project_selected_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Tdx,
            batch_with_closes(ProviderId::Tdx, Some(1_000.0), 10.0, 10.1),
        )
        .expect("TEST_CODE provider-ordered T0..D1 sequence");
        if let GatewayBatch::Available { records, .. } = &mut selected {
            records.reverse();
        }
        assert!(outcome_pending_changes("TEST_CODE_600396", &selected)
            .expect("TEST_CODE structural gate")
            .is_empty());
        let listing_evidence = BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_tdx".into(),
            source_at: Some(current_date.to_string()),
            observed_at: Local::now().to_rfc3339(),
            batch_id: "TEST_CODE_listing_batch".into(),
        };
        let actions_evidence = BatchEvidence {
            batch_id: "TEST_CODE_actions_batch".into(),
            ..listing_evidence.clone()
        };
        let lifecycle = SecurityLifecycleContext {
            instrument: InstrumentId::new(
                Exchange::Shanghai,
                "TEST_CODE_600396",
                AssetClass::Equity,
            )
            .unwrap(),
            window_start: previous_date,
            window_end: current_date,
            listing: ListingDateState::Available(AdmittedListingDate {
                listed_on: NaiveDate::from_ymd_opt(2010, 1, 1).unwrap(),
                evidence: listing_evidence,
            }),
            corporate_actions: CorporateActionState::VerifiedEmpty(actions_evidence),
        };

        let admission = admit_outcome_lifecycle("TEST_CODE_600396", &selected, &lifecycle)
            .expect("TEST_CODE lifecycle admission");
        assert_eq!(admission.window_start, previous_date);
        assert_eq!(admission.window_end, current_date);
        assert_eq!(admission.corporate_action_state, "verified_empty");
        assert_eq!(
            admission.corporate_action_batch_id,
            "TEST_CODE_actions_batch"
        );
        assert_eq!(admission.adjacent_evidence.len(), 1);
    }

    #[test]
    fn admitted_daily_bar_records_require_non_empty_evidence_batch() {
        let request = build_request("TEST_CODE_600396", 2).unwrap();
        let records = validate_candidate_batch(
            "TEST_CODE_600396",
            &request,
            ProviderId::Baidu,
            &batch(ProviderId::Baidu, Some(1_000.0)),
        )
        .unwrap();
        let evidence = BatchEvidence::from_provenance(
            ProviderId::Baidu,
            batch(ProviderId::Baidu, Some(1.0)).provenance(),
        )
        .unwrap();

        let admitted = AdmittedDailyBars::from_audited_batch(
            "TEST_CODE_600396".to_owned(),
            GatewayBatch::Available {
                records,
                evidence: evidence.clone(),
            },
        )
        .unwrap();
        assert_eq!(admitted.target_code(), "TEST_CODE_600396");
        assert_eq!(admitted.records().len(), 2);
        assert_eq!(admitted.evidence().provider, ProviderId::Baidu);
        assert_eq!(admitted.evidence().batch_id, "TEST_CODE_daily_batch");

        let empty_available = AdmittedDailyBars::from_audited_batch(
            "TEST_CODE_600396".to_owned(),
            GatewayBatch::Available {
                records: Vec::new(),
                evidence: evidence.clone(),
            },
        )
        .unwrap_err();
        assert_eq!(empty_available.reason_code(), "no_verified_batch");
        assert!(empty_available.retryable());

        let verified_empty = AdmittedDailyBars::from_audited_batch(
            "TEST_CODE_600396".to_owned(),
            GatewayBatch::VerifiedEmpty(evidence),
        )
        .unwrap_err();
        assert_eq!(verified_empty.reason_code(), "no_verified_batch");

        let real_identity_fixture = AdmittedDailyBars::from_test_fixture(
            "600396",
            Vec::new(),
            BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_daily".to_owned(),
                source_at: Some("2026-07-26T07:00:00Z".to_owned()),
                observed_at: "2026-07-26T07:00:01Z".to_owned(),
                batch_id: "TEST_CODE_daily_fixture".to_owned(),
            },
        )
        .unwrap_err();
        assert_eq!(real_identity_fixture.reason_code(), "invalid_request");
    }

    #[test]
    fn provider_labels_and_error_classifiers_are_stable() {
        assert_eq!(daily_bar_provider_label(ProviderId::Tdx), "magic_tdx");
        assert_eq!(
            daily_bar_provider_label(ProviderId::Tencent),
            "magic_tencent"
        );
        assert_eq!(daily_bar_provider_label(ProviderId::Sina), "magic_sina");
        assert_eq!(daily_bar_provider_label(ProviderId::Baidu), "magic_baidu");
        assert_eq!(
            daily_bar_provider_label(ProviderId::Eastmoney),
            "magic_unknown"
        );

        assert_eq!(
            classify_tdx_error(TdxError::ConnectionTimeout).kind(),
            FailureKind::Transport
        );
        assert_eq!(
            classify_tdx_error(TdxError::Unsupported("TEST_CODE".to_owned())).kind(),
            FailureKind::Unsupported
        );
        assert_eq!(
            classify_tdx_error(TdxError::Parse("TEST_CODE".to_owned())).kind(),
            FailureKind::Protocol
        );
        let cardinality = classify_tdx_error(TdxError::HistoricalBarCardinality {
            offset: 800,
            actual: 99,
            expected_page: 100,
            requested_total: 900,
        });
        assert_eq!(cardinality.kind(), FailureKind::Protocol);
        assert_eq!(
            cardinality.action(),
            magic_market_router::FailureAction::TryNext
        );
        for expected in [
            "offset=800",
            "actual=99",
            "expected_page=100",
            "requested_total=900",
        ] {
            assert!(cardinality.message().contains(expected));
        }
        assert_eq!(
            classify_tencent_error(TencentError::InvalidRequest("TEST_CODE".to_owned())).kind(),
            FailureKind::InvalidRequest
        );
        assert_eq!(
            classify_tencent_error(TencentError::Transport("TEST_CODE".to_owned())).kind(),
            FailureKind::Transport
        );
        assert_eq!(
            classify_sina_error(SinaError::Protocol("TEST_CODE".to_owned())).kind(),
            FailureKind::Protocol
        );
        assert_eq!(
            classify_baidu_error(BaiduError::Unsupported("TEST_CODE".to_owned())).kind(),
            FailureKind::Unsupported
        );
    }
}
