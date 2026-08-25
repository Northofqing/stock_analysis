//! BR-251 历史归因只读证据装载。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::Metadata;
use std::ops::Deref;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, NaiveDate, Timelike};
use rusqlite::{
    params_from_iter, types::Value, Connection, ErrorCode, OpenFlags, Transaction,
    TransactionBehavior,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::attribution::SignalFamily;
use super::economic_position::{
    rebuild_economic_positions, rebuild_economic_positions_with_replay_fees,
    select_economic_rows_through, CostBasisKind, EconomicFillRow, EntryFamilyComposition,
    FillCostEvidence as EconomicFillCostEvidence, FillCostLedger, NetMetrics,
};
use crate::data_gateway::{BenchmarkBar, BenchmarkBarTime};
use crate::database::order_audit::{
    validate_canonical_order_audit_chain, CanonicalOrderAuditChainRow, CanonicalOrderAuditRow,
};
use crate::trading::paper_lot_ledger::parse_paper_fill_timestamp;

const STOCK_CLOSE_HASH_DOMAIN: &[u8] = b"BR251_STOCK_CLOSE_MANIFEST_V1\0";
const FEE_EVIDENCE_HASH_DOMAIN: &[u8] = b"BR251_FILL_FEE_EVIDENCE_V1\0";
const STOCK_CLOSE_KEYS_PER_QUERY: usize = 400;
const ATTRIBUTION_REPORT_HASH_DOMAIN: &[u8] = b"BR251_ATTRIBUTION_REPORT_V1\0";
const ATTRIBUTION_REPORT_SEAL_DOMAIN: &[u8] = b"BR251_ATTRIBUTION_REPORT_SEAL_V1\0";
const REPLAY_FEE_BASIS_HASH_DOMAIN: &[u8] = b"BR251_REPLAY_FEE_BASIS_V1\0";
const REPLAY_CAPABILITY_SEAL_DOMAIN: &[u8] = b"BR251_REPLAY_CAPABILITY_SEAL_V1\0";
const MIN_CLOSED_CYCLES: usize = 200;
const MIN_COVERAGE_DAYS: i64 = 84;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AttributionUnavailable {
    SourceUnavailable,
    TradeTimeUnavailable,
    StockCloseUnavailable,
    FeeEvidenceUnavailable,
    BenchmarkTimeSemanticsUnavailable,
    BenchmarkAlignmentUnavailable,
}

impl AttributionUnavailable {
    pub const fn code(self) -> &'static str {
        match self {
            Self::SourceUnavailable => "replay_source_unavailable",
            Self::TradeTimeUnavailable => "trade_time_unavailable",
            Self::StockCloseUnavailable => "stock_close_unavailable",
            Self::FeeEvidenceUnavailable => "fee_evidence_unavailable",
            Self::BenchmarkTimeSemanticsUnavailable => "benchmark_time_semantics_unavailable",
            Self::BenchmarkAlignmentUnavailable => "benchmark_alignment_unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributionIntegrityFailure {
    InvalidRequest,
    DatabaseIdentity,
    ReadOnlyBoundary,
    SourceRead,
    OrderAuditChain,
    PaperTradeSource,
    TerminalBinding,
    StockCloseSource,
    FeeEvidence,
    BenchmarkAlignment,
    ReplayEvidence,
    EconomicPosition,
    CanonicalReport,
}

impl AttributionIntegrityFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_replay_request",
            Self::DatabaseIdentity => "database_identity_failed",
            Self::ReadOnlyBoundary => "read_only_boundary_failed",
            Self::SourceRead => "replay_source_read_failed",
            Self::OrderAuditChain => "order_audit_chain_failed",
            Self::PaperTradeSource => "paper_trade_source_failed",
            Self::TerminalBinding => "terminal_binding_failed",
            Self::StockCloseSource => "stock_close_source_failed",
            Self::FeeEvidence => "fee_evidence_failed",
            Self::BenchmarkAlignment => "benchmark_alignment_failed",
            Self::ReplayEvidence => "replay_evidence_failed",
            Self::EconomicPosition => "economic_position_failed",
            Self::CanonicalReport => "canonical_attribution_report_failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MinuteLabelSemantics {
    Unverified,
    EndLabelVerified { evidence_hash: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum MetricAvailability<T> {
    Available(T),
    Unavailable {
        code: AttributionUnavailable,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum EntryFamilyBucket {
    Single(SignalFamily),
    Mixed(Vec<SignalFamily>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricCoverage {
    pub total_cycles: usize,
    pub available_cycles: usize,
    pub unavailable_cycles: usize,
    pub coverage_ratio: Option<f64>,
    pub unavailable_reasons: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricAggregate {
    pub coverage: MetricCoverage,
    pub sum_return: Option<f64>,
    pub mean_return: Option<f64>,
    pub median_return: Option<f64>,
}

impl Deref for MetricAggregate {
    type Target = MetricCoverage;

    fn deref(&self) -> &Self::Target {
        &self.coverage
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeSummary {
    pub wins: usize,
    pub losses: usize,
    pub breakeven: usize,
    pub directional_denominator: usize,
    pub win_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeeEvidenceBinding {
    pub fill_id: i64,
    pub evidence_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttributionFeeBasis {
    pub basis_id: String,
    pub kind: CostBasisKind,
    pub bindings: Vec<FeeEvidenceBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClosedCycleAttribution {
    pub cycle_open_fill_id: i64,
    pub code: String,
    pub entry_family: EntryFamilyBucket,
    pub entry_composition: Vec<EntryFamilyComposition>,
    pub entry_terminal_time: DateTime<FixedOffset>,
    pub exit_terminal_time: DateTime<FixedOffset>,
    pub gross_return: f64,
    pub benchmark_return: MetricAvailability<f64>,
    pub gross_excess_return: MetricAvailability<f64>,
    pub net_return: MetricAvailability<f64>,
    pub net_excess_return: MetricAvailability<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntryFamilyAttribution {
    pub bucket: EntryFamilyBucket,
    pub total_closed_cycles: usize,
    pub total_open_cycles: usize,
    pub gross: MetricAggregate,
    pub benchmark: MetricAggregate,
    pub gross_excess: MetricAggregate,
    pub net: MetricAggregate,
    pub net_excess: MetricAggregate,
    pub gross_outcome: MetricAvailability<OutcomeSummary>,
    pub net_outcome: MetricAvailability<OutcomeSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum AttributionConclusion {
    InsufficientSample {
        reasons: Vec<String>,
        research_limitations: Vec<String>,
    },
    ResearchOnly {
        research_limitations: Vec<String>,
    },
}

/// 纯计算签发的只读历史归因报告。
///
/// 外部 caller 不能通过 struct update 重组来源成交或其他 canonical 字段：
///
/// ```compile_fail
/// use stock_analysis::performance::attribution_replay::AttributionComputationReport;
///
/// fn forge(existing: AttributionComputationReport) -> AttributionComputationReport {
///     AttributionComputationReport {
///         source_fill_ids: vec![9_999],
///         ..existing
///     }
/// }
/// ```
///
/// 外部 caller 也不能对已签发报告重新赋值：
///
/// ```compile_fail
/// use stock_analysis::performance::attribution_replay::AttributionComputationReport;
///
/// fn rebind(mut report: AttributionComputationReport) {
///     report.source_fill_ids = vec![9_999];
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AttributionComputationReport {
    from: NaiveDate,
    to: NaiveDate,
    #[serde(rename = "source_fill_ids")]
    canonical_source_fill_ids: Vec<i64>,
    total_closed_cycles: usize,
    total_open_cycles: usize,
    coverage_days: Option<i64>,
    closed_cycles: Vec<ClosedCycleAttribution>,
    family_attribution: Vec<EntryFamilyAttribution>,
    gross: MetricAggregate,
    benchmark: MetricAggregate,
    gross_excess: MetricAggregate,
    net: MetricAggregate,
    net_excess: MetricAggregate,
    gross_outcome: MetricAvailability<OutcomeSummary>,
    net_outcome: MetricAvailability<OutcomeSummary>,
    fee_basis: MetricAvailability<AttributionFeeBasis>,
    gross_win_rate: Option<f64>,
    net_win_rate: MetricAvailability<Option<f64>>,
    conclusion: AttributionConclusion,
    /// 仅兼容现有只读 probe 的字段读取；canonical projection 不重复序列化该投影。
    #[serde(skip)]
    read_only_projection: AttributionComputationReportReadOnly,
    /// 只绑定先完成递归验证和 signed-zero 规范化的 canonical projection。
    #[serde(skip)]
    report_seal: Option<AttributionComputationReportSeal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributionComputationReportSeal([u8; 32]);

/// 现有 probe 的只读兼容投影。它不能传给 canonical seam，也没有 `DerefMut`。
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributionComputationReportReadOnly {
    pub source_fill_ids: Vec<i64>,
}

impl Deref for AttributionComputationReport {
    type Target = AttributionComputationReportReadOnly;

    fn deref(&self) -> &Self::Target {
        &self.read_only_projection
    }
}

impl AttributionComputationReport {
    pub fn from(&self) -> NaiveDate {
        self.from
    }

    pub fn to(&self) -> NaiveDate {
        self.to
    }

    pub fn source_fill_ids(&self) -> &[i64] {
        &self.canonical_source_fill_ids
    }

    pub fn total_closed_cycles(&self) -> usize {
        self.total_closed_cycles
    }

    pub fn total_open_cycles(&self) -> usize {
        self.total_open_cycles
    }

    pub fn coverage_days(&self) -> Option<i64> {
        self.coverage_days
    }

    pub fn closed_cycles(&self) -> &[ClosedCycleAttribution] {
        &self.closed_cycles
    }

    pub fn family_attribution(&self) -> &[EntryFamilyAttribution] {
        &self.family_attribution
    }

    pub fn gross(&self) -> &MetricAggregate {
        &self.gross
    }

    pub fn benchmark(&self) -> &MetricAggregate {
        &self.benchmark
    }

    pub fn gross_excess(&self) -> &MetricAggregate {
        &self.gross_excess
    }

    pub fn net(&self) -> &MetricAggregate {
        &self.net
    }

    pub fn net_excess(&self) -> &MetricAggregate {
        &self.net_excess
    }

    pub fn gross_outcome(&self) -> &MetricAvailability<OutcomeSummary> {
        &self.gross_outcome
    }

    pub fn net_outcome(&self) -> &MetricAvailability<OutcomeSummary> {
        &self.net_outcome
    }

    pub fn fee_basis(&self) -> &MetricAvailability<AttributionFeeBasis> {
        &self.fee_basis
    }

    pub fn gross_win_rate(&self) -> Option<f64> {
        self.gross_win_rate
    }

    pub fn net_win_rate(&self) -> &MetricAvailability<Option<f64>> {
        &self.net_win_rate
    }

    pub fn conclusion(&self) -> &AttributionConclusion {
        &self.conclusion
    }
}

fn is_shanghai_offset(timestamp: &DateTime<FixedOffset>) -> bool {
    timestamp.offset().local_minus_utc() == 8 * 60 * 60
}

fn is_minute_end_grid(timestamp: &DateTime<FixedOffset>) -> bool {
    if timestamp.second() != 0 || timestamp.nanosecond() != 0 {
        return false;
    }
    let minute_of_day = timestamp.hour() * 60 + timestamp.minute();
    (9 * 60 + 31..=11 * 60 + 30).contains(&minute_of_day)
        || (13 * 60 + 1..=15 * 60).contains(&minute_of_day)
}

/// BR-251：只选择成交前严格完成、同一上海自然日且不超过 60 秒的唯一分钟结束线。
pub fn align_completed_minute<'a>(
    fill_at: DateTime<FixedOffset>,
    bars: &'a [BenchmarkBar],
    semantics: &MinuteLabelSemantics,
) -> Result<&'a BenchmarkBar, AttributionReplayError> {
    match semantics {
        MinuteLabelSemantics::Unverified => {
            return Err(AttributionReplayError::unavailable(
                AttributionUnavailable::BenchmarkTimeSemanticsUnavailable,
                false,
                "benchmark minute end-label semantics are unverified",
            ));
        }
        MinuteLabelSemantics::EndLabelVerified { evidence_hash }
            if evidence_hash.trim().is_empty() =>
        {
            return Err(AttributionReplayError::unavailable(
                AttributionUnavailable::BenchmarkTimeSemanticsUnavailable,
                false,
                "benchmark minute end-label evidence hash is absent",
            ));
        }
        MinuteLabelSemantics::EndLabelVerified { .. } => {}
    }
    if !is_shanghai_offset(&fill_at) {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::BenchmarkAlignment,
            "fill timestamp is not explicit Asia/Shanghai +08:00",
        ));
    }

    let mut seen = BTreeSet::new();
    let mut candidate = None;
    for bar in bars {
        let BenchmarkBarTime::MinuteEnd(bar_end) = &bar.at else {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::BenchmarkAlignment,
                "daily benchmark bar cannot enter minute alignment",
            ));
        };
        if !is_shanghai_offset(bar_end) || !is_minute_end_grid(bar_end) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::BenchmarkAlignment,
                format!("benchmark minute label is outside the exact Shanghai grid: {bar_end}"),
            ));
        }
        if !seen.insert(*bar_end) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::BenchmarkAlignment,
                format!("duplicate benchmark minute label: {bar_end}"),
            ));
        }
        if !bar.open.is_finite()
            || !bar.high.is_finite()
            || !bar.low.is_finite()
            || !bar.close.is_finite()
            || bar.open <= 0.0
            || bar.high <= 0.0
            || bar.low <= 0.0
            || bar.close <= 0.0
            || bar.low > bar.open
            || bar.low > bar.close
            || bar.high < bar.open
            || bar.high < bar.close
        {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::BenchmarkAlignment,
                format!("benchmark minute OHLC is invalid at {bar_end}"),
            ));
        }
        let delta = fill_at.signed_duration_since(*bar_end);
        if bar_end.date_naive() == fill_at.date_naive()
            && delta > chrono::Duration::zero()
            && delta <= chrono::Duration::seconds(60)
            && candidate.replace(bar).is_some()
        {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::BenchmarkAlignment,
                format!("multiple completed benchmark minutes align to fill {fill_at}"),
            ));
        }
    }
    candidate.ok_or_else(|| {
        AttributionReplayError::unavailable(
            AttributionUnavailable::BenchmarkAlignmentUnavailable,
            false,
            format!("no completed benchmark minute aligns to fill {fill_at}"),
        )
    })
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AttributionReplayError {
    #[error("{code:?}: {detail}")]
    Unavailable {
        code: AttributionUnavailable,
        retryable: bool,
        detail: String,
    },
    #[error("{code:?}: {detail}")]
    FailedIntegrity {
        code: AttributionIntegrityFailure,
        detail: String,
    },
}

impl AttributionReplayError {
    fn unavailable(
        code: AttributionUnavailable,
        retryable: bool,
        detail: impl Into<String>,
    ) -> Self {
        Self::Unavailable {
            code,
            retryable,
            detail: detail.into(),
        }
    }

    fn integrity(code: AttributionIntegrityFailure, detail: impl Into<String>) -> Self {
        Self::FailedIntegrity {
            code,
            detail: detail.into(),
        }
    }
}

/// 费用证据只能由本模块的 loader/未来真实 adapter 签发，外部 caller 只能读取。
///
/// ```compile_fail
/// use stock_analysis::performance::attribution_replay::FillFeeEvidence;
///
/// let forged = FillFeeEvidence {
///     fill_id: 1,
///     adverse_cost: 0.0,
///     source: "caller".to_owned(),
///     authority: "caller".to_owned(),
///     evidence_id: "caller".to_owned(),
///     evidence_hash: "0".repeat(64),
/// };
/// ```
///
/// 普通 caller 也不能导入内部 authoritative fee hash helper：
///
/// ```compile_fail
/// use stock_analysis::performance::attribution_replay::canonical_fill_fee_evidence_hash;
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct FillFeeEvidence {
    fill_id: i64,
    adverse_cost: f64,
    source: String,
    authority: String,
    evidence_id: String,
    evidence_hash: String,
}

impl FillFeeEvidence {
    pub fn fill_id(&self) -> i64 {
        self.fill_id
    }

    pub fn adverse_cost(&self) -> f64 {
        self.adverse_cost
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn evidence_id(&self) -> &str {
        &self.evidence_id
    }

    pub fn evidence_hash(&self) -> &str {
        &self.evidence_hash
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthoritativeFillFeeLedger {
    entries: Vec<FillFeeEvidence>,
}

impl AuthoritativeFillFeeLedger {
    pub fn entries(&self) -> &[FillFeeEvidence] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FeeEvidenceAvailability {
    Available(AuthoritativeFillFeeLedger),
    Unavailable {
        code: AttributionUnavailable,
        retryable: bool,
        detail: String,
    },
}

impl FeeEvidenceAvailability {
    pub fn available_ledger(&self) -> Option<&AuthoritativeFillFeeLedger> {
        match self {
            Self::Available(ledger) => Some(ledger),
            Self::Unavailable { .. } => None,
        }
    }

    pub fn unavailable_reason(&self) -> Option<(AttributionUnavailable, bool, &str)> {
        match self {
            Self::Available(_) => None,
            Self::Unavailable {
                code,
                retryable,
                detail,
            } => Some((*code, *retryable, detail)),
        }
    }
}

#[derive(Debug)]
pub(super) struct ValidatedReplayFeeLedger {
    ledger: FillCostLedger,
}

impl ValidatedReplayFeeLedger {
    pub(super) fn economic_ledger(&self) -> &FillCostLedger {
        &self.ledger
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttributionReplayRequest {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// 由上层已验证交易日 authority 提供；本装载器绝不猜工作日或节假日。
    pub required_trading_dates: Vec<NaiveDate>,
    pub fee_ledger: Option<AuthoritativeFillFeeLedger>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayFillEvidence {
    pub fill: EconomicFillRow,
    pub terminal_audit_id: i64,
    pub terminal_audit_hash: String,
    pub terminal_time: DateTime<FixedOffset>,
}

impl ReplayFillEvidence {
    pub fn fill(&self) -> &EconomicFillRow {
        &self.fill
    }

    pub fn terminal_audit_id(&self) -> i64 {
        self.terminal_audit_id
    }

    pub fn terminal_audit_hash(&self) -> &str {
        &self.terminal_audit_hash
    }

    pub fn terminal_time(&self) -> DateTime<FixedOffset> {
        self.terminal_time
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StockCloseEvidence {
    pub code: String,
    pub date: NaiveDate,
    pub close: f64,
    pub data_source: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StockCloseManifest {
    pub entries: Vec<StockCloseEvidence>,
    pub manifest_hash: String,
}

impl StockCloseManifest {
    pub fn entries(&self) -> &[StockCloseEvidence] {
        &self.entries
    }

    pub fn manifest_hash(&self) -> &str {
        &self.manifest_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributionReplayCapabilitySeal([u8; 32]);

/// Loader 签发的 BR-251 replay capability。
///
/// 外部 caller 可读取兼容投影，但不能构造缺少私有 seal 的 capability；任何投影换绑都会在
/// 进入纯计算前失败关闭。新调用方应使用只读 accessor。
///
/// ```compile_fail
/// use stock_analysis::performance::attribution_replay::AttributionReplayEvidence;
///
/// fn forge() -> AttributionReplayEvidence {
///     AttributionReplayEvidence {
///         from: todo!(),
///         to: todo!(),
///         fills: Vec::new(),
///         stock_closes: todo!(),
///         fees: todo!(),
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AttributionReplayEvidence {
    // Task29 probe 仍读取这些公开投影；私有 seal 绑定全部字段，换绑后不能进入 compute。
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// 含范围开始前 FIFO 前史，但不含 `to` 之后的成交。
    pub fills: Vec<ReplayFillEvidence>,
    pub stock_closes: StockCloseManifest,
    pub fees: FeeEvidenceAvailability,
    capability_seal: AttributionReplayCapabilitySeal,
}

impl AttributionReplayEvidence {
    fn issued(
        from: NaiveDate,
        to: NaiveDate,
        fills: Vec<ReplayFillEvidence>,
        stock_closes: StockCloseManifest,
        fees: FeeEvidenceAvailability,
    ) -> Self {
        let capability_seal = replay_capability_seal(from, to, &fills, &stock_closes, &fees);
        Self {
            from,
            to,
            fills,
            stock_closes,
            fees,
            capability_seal,
        }
    }

    pub fn from(&self) -> NaiveDate {
        self.from
    }

    pub fn to(&self) -> NaiveDate {
        self.to
    }

    pub fn fills(&self) -> &[ReplayFillEvidence] {
        &self.fills
    }

    pub fn stock_closes(&self) -> &StockCloseManifest {
        &self.stock_closes
    }

    pub fn fees(&self) -> &FeeEvidenceAvailability {
        &self.fees
    }

    pub fn into_fills(self) -> Vec<ReplayFillEvidence> {
        self.fills
    }
}

fn update_optional_text(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update([1]);
            update_len_prefixed(hasher, value.as_bytes());
        }
        None => hasher.update([0]),
    }
}

fn replay_capability_seal(
    from: NaiveDate,
    to: NaiveDate,
    fills: &[ReplayFillEvidence],
    stock_closes: &StockCloseManifest,
    fees: &FeeEvidenceAvailability,
) -> AttributionReplayCapabilitySeal {
    let mut hasher = Sha256::new();
    hasher.update(REPLAY_CAPABILITY_SEAL_DOMAIN);
    update_len_prefixed(&mut hasher, from.to_string().as_bytes());
    update_len_prefixed(&mut hasher, to.to_string().as_bytes());
    hasher.update((fills.len() as u64).to_be_bytes());
    for evidence in fills {
        let fill = &evidence.fill;
        hasher.update(fill.id.to_be_bytes());
        update_len_prefixed(&mut hasher, fill.plan_id.as_bytes());
        update_len_prefixed(&mut hasher, fill.code.as_bytes());
        update_len_prefixed(&mut hasher, fill.name.as_bytes());
        update_len_prefixed(&mut hasher, fill.direction.as_bytes());
        match fill.fill_price {
            Some(price) => {
                hasher.update([1]);
                hasher.update(price.to_bits().to_be_bytes());
            }
            None => hasher.update([0]),
        }
        hasher.update(fill.quantity.to_be_bytes());
        update_len_prefixed(&mut hasher, fill.occurred_at.as_bytes());
        update_len_prefixed(&mut hasher, fill.virtual_reason.as_bytes());
        hasher.update(evidence.terminal_audit_id.to_be_bytes());
        update_len_prefixed(&mut hasher, evidence.terminal_audit_hash.as_bytes());
        update_len_prefixed(&mut hasher, evidence.terminal_time.to_rfc3339().as_bytes());
    }
    update_len_prefixed(&mut hasher, stock_closes.manifest_hash.as_bytes());
    hasher.update((stock_closes.entries.len() as u64).to_be_bytes());
    for close in &stock_closes.entries {
        update_len_prefixed(&mut hasher, close.code.as_bytes());
        update_len_prefixed(&mut hasher, close.date.to_string().as_bytes());
        hasher.update(close.close.to_bits().to_be_bytes());
        update_optional_text(&mut hasher, close.data_source.as_deref());
        update_len_prefixed(&mut hasher, close.created_at.as_bytes());
        update_len_prefixed(&mut hasher, close.updated_at.as_bytes());
    }
    match fees {
        FeeEvidenceAvailability::Available(ledger) => {
            hasher.update([1]);
            hasher.update((ledger.entries.len() as u64).to_be_bytes());
            for entry in &ledger.entries {
                hasher.update(entry.fill_id.to_be_bytes());
                hasher.update(entry.adverse_cost.to_bits().to_be_bytes());
                update_len_prefixed(&mut hasher, entry.source.as_bytes());
                update_len_prefixed(&mut hasher, entry.authority.as_bytes());
                update_len_prefixed(&mut hasher, entry.evidence_id.as_bytes());
                update_len_prefixed(&mut hasher, entry.evidence_hash.as_bytes());
            }
        }
        FeeEvidenceAvailability::Unavailable {
            code,
            retryable,
            detail,
        } => {
            hasher.update([0]);
            update_len_prefixed(&mut hasher, code.code().as_bytes());
            hasher.update([u8::from(*retryable)]);
            update_len_prefixed(&mut hasher, detail.as_bytes());
        }
    }
    AttributionReplayCapabilitySeal(hasher.finalize().into())
}

fn validate_replay_capability(
    evidence: &AttributionReplayEvidence,
) -> Result<(), AttributionReplayError> {
    let expected = replay_capability_seal(
        evidence.from,
        evidence.to,
        &evidence.fills,
        &evidence.stock_closes,
        &evidence.fees,
    );
    if expected != evidence.capability_seal {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReplayEvidence,
            "replay capability projection no longer matches the loader-issued seal",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AttributionReplayLoader {
    database: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Debug, Clone)]
struct PaperTradeSourceRow {
    fill: EconomicFillRow,
    requested_price: f64,
}

#[derive(Debug, Clone)]
struct RawStockCloseRow {
    id: i64,
    code: String,
    date: String,
    close: Option<f64>,
    data_source: Option<String>,
    created_at: String,
    updated_at: String,
}

impl AttributionReplayLoader {
    pub fn new(database: impl AsRef<Path>) -> Self {
        Self {
            database: database.as_ref().to_path_buf(),
        }
    }

    pub fn load(
        &self,
        request: &AttributionReplayRequest,
    ) -> Result<AttributionReplayEvidence, AttributionReplayError> {
        validate_request(request)?;
        let canonical_database = self.database.canonicalize().map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("explicit database path cannot be resolved: {error}"),
            )
        })?;
        let before_metadata = canonical_database.metadata().map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("explicit database metadata unavailable: {error}"),
            )
        })?;
        if !before_metadata.is_file() {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                "explicit database path is not a regular file",
            ));
        }
        let expected_identity = FileIdentity::of(&before_metadata);
        let mut connection = open_query_only_connection(&canonical_database)?;
        verify_main_database(&connection, &canonical_database, expected_identity)?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| source_read_error("begin one read transaction", error))?;
        let all_paper_rows = load_paper_rows(&transaction)?;
        let audit_rows = load_order_audits(&transaction)?;
        let chain_rows = load_order_audit_chain(&transaction)?;
        validate_canonical_order_audit_chain(&audit_rows, &chain_rows).map_err(|detail| {
            AttributionReplayError::integrity(AttributionIntegrityFailure::OrderAuditChain, detail)
        })?;

        let all_economic_rows = all_paper_rows
            .iter()
            .map(|row| row.fill.clone())
            .collect::<Vec<_>>();
        validate_complete_paper_source(&all_economic_rows, request.to)?;
        let all_terminals = bind_all_terminals(&all_paper_rows, &audit_rows, &chain_rows)?;
        let projected_rows =
            select_economic_rows_through(all_economic_rows, request.to).map_err(|detail| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::PaperTradeSource,
                    detail,
                )
            })?;
        let terminal_by_fill = all_terminals
            .into_iter()
            .map(|terminal| (terminal.fill.id, terminal))
            .collect::<HashMap<_, _>>();
        let fills = projected_rows
            .into_iter()
            .map(|row| {
                terminal_by_fill.get(&row.id).cloned().ok_or_else(|| {
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::TerminalBinding,
                        format!("validated terminal disappeared for fill id={}", row.id),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let required_close_keys =
            derive_required_close_keys(&fills, &request.required_trading_dates)?;
        let raw_closes = load_stock_closes(&transaction, &required_close_keys)?;
        let stock_closes = build_stock_close_manifest(raw_closes, &required_close_keys)?;
        verify_transaction_main_database(&transaction, &canonical_database, expected_identity)?;
        let fees = validate_fee_ledger(request.fee_ledger.as_ref(), &fills)?;

        #[cfg(test)]
        run_after_read_test_hook();
        let during_identity = canonical_database
            .metadata()
            .map(|metadata| FileIdentity::of(&metadata))
            .map_err(|error| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::DatabaseIdentity,
                    format!("database identity re-check during read failed: {error}"),
                )
            })?;
        if during_identity != expected_identity {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                "database file identity changed during read",
            ));
        }
        transaction
            .commit()
            .map_err(|error| source_read_error("finish read transaction", error))?;
        verify_main_database(&connection, &canonical_database, expected_identity)?;
        let after_identity = canonical_database
            .metadata()
            .map(|metadata| FileIdentity::of(&metadata))
            .map_err(|error| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::DatabaseIdentity,
                    format!("database identity re-check after read failed: {error}"),
                )
            })?;
        if after_identity != expected_identity {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                "database file identity changed after read",
            ));
        }

        Ok(AttributionReplayEvidence::issued(
            request.from,
            request.to,
            fills,
            stock_closes,
            fees,
        ))
    }
}

fn open_query_only_connection(path: &Path) -> Result<Connection, AttributionReplayError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReadOnlyBoundary,
            format!("open explicit SQLite database read-only: {error}"),
        )
    })?;
    connection
        .execute_batch("PRAGMA query_only=ON;")
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::ReadOnlyBoundary,
                format!("enable SQLite query_only: {error}"),
            )
        })?;
    let query_only: i64 = connection
        .query_row("PRAGMA query_only", [], |row| row.get(0))
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::ReadOnlyBoundary,
                format!("verify SQLite query_only: {error}"),
            )
        })?;
    if query_only != 1 {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReadOnlyBoundary,
            format!("SQLite query_only expected 1, got {query_only}"),
        ));
    }
    Ok(connection)
}

fn validate_request(request: &AttributionReplayRequest) -> Result<(), AttributionReplayError> {
    if request.from > request.to {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::InvalidRequest,
            "attribution replay from date is after to date",
        ));
    }
    if request.required_trading_dates.is_empty() {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::InvalidRequest,
            "required trading dates authority must not be empty",
        ));
    }
    let mut previous = None;
    for current in &request.required_trading_dates {
        if *current < request.from || *current > request.to {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::InvalidRequest,
                format!("required trading date {current} is outside requested range"),
            ));
        }
        if previous.is_some_and(|date| date >= *current) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::InvalidRequest,
                "required trading dates must be sorted and unique",
            ));
        }
        previous = Some(*current);
    }
    Ok(())
}

fn verify_main_database(
    connection: &Connection,
    expected_path: &Path,
    expected_identity: FileIdentity,
) -> Result<(), AttributionReplayError> {
    let mut statement = connection
        .prepare("PRAGMA database_list")
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("prepare SQLite database_list: {error}"),
            )
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|error| source_read_error("read SQLite database_list", error))?;
    let databases = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| source_read_error("decode SQLite database_list", error))?;
    let main_files = databases
        .into_iter()
        .filter_map(|(name, file)| (name == "main").then_some(file))
        .collect::<Vec<_>>();
    if main_files.len() != 1 || main_files[0].trim().is_empty() {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::DatabaseIdentity,
            "SQLite database_list does not identify exactly one main file",
        ));
    }
    let main_path = PathBuf::from(&main_files[0])
        .canonicalize()
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("resolve SQLite main file: {error}"),
            )
        })?;
    let main_identity = main_path
        .metadata()
        .map(|metadata| FileIdentity::of(&metadata))
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("read SQLite main identity: {error}"),
            )
        })?;
    if main_path != expected_path || main_identity != expected_identity {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::DatabaseIdentity,
            "SQLite main file does not match pinned explicit database",
        ));
    }
    Ok(())
}

fn verify_transaction_main_database(
    transaction: &Transaction<'_>,
    expected_path: &Path,
    expected_identity: FileIdentity,
) -> Result<(), AttributionReplayError> {
    let file: String = transaction
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| source_read_error("read transaction main file", error))?;
    let main = PathBuf::from(file).canonicalize().map_err(|error| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::DatabaseIdentity,
            format!("resolve transaction main file: {error}"),
        )
    })?;
    let identity = main
        .metadata()
        .map(|metadata| FileIdentity::of(&metadata))
        .map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::DatabaseIdentity,
                format!("read transaction main identity: {error}"),
            )
        })?;
    if main != expected_path || identity != expected_identity {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::DatabaseIdentity,
            "transaction main file identity changed",
        ));
    }
    Ok(())
}

fn source_read_error(context: &str, error: rusqlite::Error) -> AttributionReplayError {
    if matches!(
        &error,
        rusqlite::Error::SqliteFailure(sqlite, _)
            if matches!(sqlite.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    ) {
        return AttributionReplayError::unavailable(
            AttributionUnavailable::SourceUnavailable,
            true,
            format!("{context}: SQLite source is busy or locked"),
        );
    }
    AttributionReplayError::integrity(
        AttributionIntegrityFailure::SourceRead,
        format!("{context}: {error}"),
    )
}

fn load_paper_rows(
    transaction: &Transaction<'_>,
) -> Result<Vec<PaperTradeSourceRow>, AttributionReplayError> {
    let mut statement = transaction
        .prepare(
            "SELECT id, plan_id, code, name, direction, price, fill_price, quantity,
                    CAST(ts AS TEXT), virtual_reason
             FROM paper_trades WHERE status='Filled'
             ORDER BY CAST(ts AS TEXT) ASC, id ASC",
        )
        .map_err(|error| source_read_error("prepare complete Filled paper source", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok(PaperTradeSourceRow {
                fill: EconomicFillRow {
                    id: row.get(0)?,
                    plan_id: row.get(1)?,
                    code: row.get(2)?,
                    name: row.get(3)?,
                    direction: row.get(4)?,
                    fill_price: row.get(6)?,
                    quantity: row.get(7)?,
                    occurred_at: row.get(8)?,
                    virtual_reason: row.get(9)?,
                },
                requested_price: row.get(5)?,
            })
        })
        .map_err(|error| source_read_error("read complete Filled paper source", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| source_read_error("decode complete Filled paper source", error))
}

fn load_order_audits(
    transaction: &Transaction<'_>,
) -> Result<Vec<CanonicalOrderAuditRow>, AttributionReplayError> {
    let mut statement = transaction
        .prepare(
            "SELECT id,business_order_id,source,decision_basis,side,code,
                    requested_price,execution_price,quantity,quote_observed_at,
                    outcome,failure_reason,CAST(created_at AS TEXT)
             FROM order_audit ORDER BY id ASC",
        )
        .map_err(|error| source_read_error("prepare complete order audit source", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok(CanonicalOrderAuditRow {
                id: row.get(0)?,
                business_order_id: row.get(1)?,
                source: row.get(2)?,
                decision_basis: row.get(3)?,
                side: row.get(4)?,
                code: row.get(5)?,
                requested_price: row.get(6)?,
                execution_price: row.get(7)?,
                quantity: row.get(8)?,
                quote_observed_at: row.get(9)?,
                outcome: row.get(10)?,
                failure_reason: row.get(11)?,
                created_at: row.get(12)?,
            })
        })
        .map_err(|error| source_read_error("read complete order audit source", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| source_read_error("decode complete order audit source", error))
}

fn load_order_audit_chain(
    transaction: &Transaction<'_>,
) -> Result<Vec<CanonicalOrderAuditChainRow>, AttributionReplayError> {
    let mut statement = transaction
        .prepare(
            "SELECT order_audit_id,previous_hash,record_hash
             FROM order_audit_chain ORDER BY order_audit_id ASC",
        )
        .map_err(|error| source_read_error("prepare complete order audit chain", error))?;
    let rows = statement
        .query_map([], |row| {
            Ok(CanonicalOrderAuditChainRow {
                order_audit_id: row.get(0)?,
                previous_hash: row.get(1)?,
                record_hash: row.get(2)?,
            })
        })
        .map_err(|error| source_read_error("read complete order audit chain", error))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| source_read_error("decode complete order audit chain", error))
}

fn load_stock_closes(
    transaction: &Transaction<'_>,
    required_keys: &BTreeSet<(String, NaiveDate)>,
) -> Result<Vec<RawStockCloseRow>, AttributionReplayError> {
    let keys = required_keys.iter().collect::<Vec<_>>();
    let mut result = Vec::new();
    for chunk in keys.chunks(STOCK_CLOSE_KEYS_PER_QUERY) {
        let predicate = std::iter::repeat_n("(code = ? AND date = ?)", chunk.len())
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!(
            "SELECT id,code,date,close,data_source,
                    CAST(created_at AS TEXT),CAST(updated_at AS TEXT)
             FROM stock_daily WHERE {predicate}
             ORDER BY code ASC, date ASC, id ASC"
        );
        let values = chunk
            .iter()
            .flat_map(|(code, date)| {
                [
                    Value::Text(code.clone()),
                    Value::Text(date.format("%Y-%m-%d").to_string()),
                ]
            })
            .collect::<Vec<_>>();
        let mut statement = transaction
            .prepare(&sql)
            .map_err(|error| source_read_error("prepare exact stock close source", error))?;
        let rows = statement
            .query_map(params_from_iter(values), |row| {
                Ok(RawStockCloseRow {
                    id: row.get(0)?,
                    code: row.get(1)?,
                    date: row.get(2)?,
                    close: row.get(3)?,
                    data_source: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|error| source_read_error("read exact stock close source", error))?;
        result.extend(
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| source_read_error("decode exact stock close source", error))?,
        );
    }
    Ok(result)
}

fn validate_complete_paper_source(
    rows: &[EconomicFillRow],
    empty_source_date: NaiveDate,
) -> Result<(), AttributionReplayError> {
    let max_date = rows
        .iter()
        .map(|row| {
            parse_paper_fill_timestamp(row.id, &row.occurred_at)
                .map(|timestamp| timestamp.date())
                .map_err(|detail| {
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::PaperTradeSource,
                        detail,
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap_or(empty_source_date);
    select_economic_rows_through(rows.to_vec(), max_date).map_err(|detail| {
        AttributionReplayError::integrity(AttributionIntegrityFailure::PaperTradeSource, detail)
    })?;
    rebuild_economic_positions(rows, max_date, None).map_err(|detail| {
        AttributionReplayError::integrity(AttributionIntegrityFailure::PaperTradeSource, detail)
    })?;
    Ok(())
}

fn bind_all_terminals(
    paper_rows: &[PaperTradeSourceRow],
    audits: &[CanonicalOrderAuditRow],
    chain: &[CanonicalOrderAuditChainRow],
) -> Result<Vec<ReplayFillEvidence>, AttributionReplayError> {
    let hashes = chain
        .iter()
        .map(|row| (row.order_audit_id, row.record_hash.as_str()))
        .collect::<HashMap<_, _>>();
    let paper_plans = paper_rows
        .iter()
        .map(|paper| paper.fill.plan_id.as_str())
        .collect::<HashSet<_>>();
    let mut by_business = HashMap::<&str, Vec<&CanonicalOrderAuditRow>>::new();
    for audit in audits
        .iter()
        .filter(|audit| audit.source == "PaperTrade" && audit.outcome == "Filled")
    {
        if !paper_plans.contains(audit.business_order_id.as_str()) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!(
                    "PaperTrade Filled audit id={} has no Filled paper plan {}",
                    audit.id, audit.business_order_id
                ),
            ));
        }
        by_business
            .entry(audit.business_order_id.as_str())
            .or_default()
            .push(audit);
    }
    let shanghai = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::TerminalBinding,
            "fixed +08:00 offset is unavailable",
        )
    })?;
    let mut result = Vec::with_capacity(paper_rows.len());
    for paper in paper_rows {
        // BR-251：空集合只表示“零条 Filled 终态”，紧接着转为 typed
        // TradeTimeUnavailable；它绝不作为可计算数据或静默成功返回。
        let terminals = by_business
            .get(paper.fill.plan_id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        if terminals.is_empty() {
            return Err(AttributionReplayError::unavailable(
                AttributionUnavailable::TradeTimeUnavailable,
                false,
                format!(
                    "Filled paper id={} has no Filled audit terminal",
                    paper.fill.id
                ),
            ));
        }
        if terminals.len() != 1 {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!(
                    "Filled paper id={} has {} Filled audit terminals",
                    paper.fill.id,
                    terminals.len()
                ),
            ));
        }
        let terminal = terminals[0];
        let execution_price = terminal.execution_price.ok_or_else(|| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!("Filled audit id={} execution price is absent", terminal.id),
            )
        })?;
        let paper_fill_price = paper.fill.fill_price.ok_or_else(|| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::PaperTradeSource,
                format!("Filled paper id={} fill price is absent", paper.fill.id),
            )
        })?;
        let exact = terminal.code == paper.fill.code
            && terminal.side == paper.fill.direction
            && terminal.requested_price.to_bits() == paper.requested_price.to_bits()
            && execution_price.to_bits() == paper_fill_price.to_bits()
            && terminal.quantity == paper.fill.quantity;
        if !exact {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!(
                    "Filled paper id={} does not exactly match audit id={} source/code/side/prices/quantity",
                    paper.fill.id, terminal.id
                ),
            ));
        }
        if !paper.requested_price.is_finite()
            || paper.requested_price <= 0.0
            || !paper_fill_price.is_finite()
            || paper_fill_price <= 0.0
        {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::TerminalBinding,
                format!(
                    "Filled paper id={} contains an invalid price",
                    paper.fill.id
                ),
            ));
        }
        let raw_time = terminal
            .quote_observed_at
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                AttributionReplayError::unavailable(
                    AttributionUnavailable::TradeTimeUnavailable,
                    false,
                    format!("Filled audit id={} has no quote_observed_at", terminal.id),
                )
            })?;
        let terminal_time = DateTime::parse_from_rfc3339(raw_time)
            .map_err(|error| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::TerminalBinding,
                    format!(
                        "Filled audit id={} quote_observed_at is not full RFC3339: {error}",
                        terminal.id
                    ),
                )
            })?
            .with_timezone(&shanghai);
        let terminal_audit_hash = hashes.get(&terminal.id).ok_or_else(|| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::OrderAuditChain,
                format!("Filled audit id={} has no chain hash", terminal.id),
            )
        })?;
        result.push(ReplayFillEvidence {
            fill: paper.fill.clone(),
            terminal_audit_id: terminal.id,
            terminal_audit_hash: (*terminal_audit_hash).to_owned(),
            terminal_time,
        });
    }
    Ok(result)
}

fn derive_required_close_keys(
    fills: &[ReplayFillEvidence],
    required_dates: &[NaiveDate],
) -> Result<BTreeSet<(String, NaiveDate)>, AttributionReplayError> {
    let dated_fills = fills
        .iter()
        .map(|evidence| {
            parse_paper_fill_timestamp(evidence.fill.id, &evidence.fill.occurred_at)
                .map(|timestamp| (timestamp.date(), evidence.fill.clone()))
                .map_err(|detail| {
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::PaperTradeSource,
                        detail,
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut keys = BTreeSet::new();
    let mut prefix = Vec::new();
    let mut next_fill = 0;
    for required_date in required_dates {
        while next_fill < dated_fills.len() && dated_fills[next_fill].0 <= *required_date {
            let (fill_date, fill) = &dated_fills[next_fill];
            if fill_date == required_date {
                keys.insert((fill.code.clone(), *required_date));
            }
            prefix.push(fill.clone());
            next_fill += 1;
        }
        let report =
            rebuild_economic_positions(&prefix, *required_date, None).map_err(|detail| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::PaperTradeSource,
                    detail,
                )
            })?;
        keys.extend(
            report
                .open_positions
                .into_iter()
                .map(|position| (position.code, *required_date)),
        );
    }
    Ok(keys)
}

fn build_stock_close_manifest(
    rows: Vec<RawStockCloseRow>,
    required_keys: &BTreeSet<(String, NaiveDate)>,
) -> Result<StockCloseManifest, AttributionReplayError> {
    let mut selected = BTreeMap::<(String, NaiveDate), StockCloseEvidence>::new();
    for row in rows {
        let parsed_date = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d").map_err(|error| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!(
                    "stock_daily id={} date is not exact YYYY-MM-DD: {error}",
                    row.id
                ),
            )
        })?;
        if parsed_date.format("%Y-%m-%d").to_string() != row.date {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!("stock_daily id={} date is not canonical YYYY-MM-DD", row.id),
            ));
        }
        let key = (row.code.clone(), parsed_date);
        if !required_keys.contains(&key) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!(
                    "stock close query returned unexpected key {} {}",
                    row.code, parsed_date
                ),
            ));
        }
        if selected.contains_key(&key) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!(
                    "duplicate stock close fact for {} {}",
                    row.code, parsed_date
                ),
            ));
        }
        let close = row.close.ok_or_else(|| {
            AttributionReplayError::unavailable(
                AttributionUnavailable::StockCloseUnavailable,
                true,
                format!("stock close is absent for {} {}", row.code, parsed_date),
            )
        })?;
        if !close.is_finite() || close <= 0.0 {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::StockCloseSource,
                format!("stock close is invalid for {} {}", row.code, parsed_date),
            ));
        }
        row.data_source
            .as_deref()
            .filter(|source| !source.trim().is_empty())
            .ok_or_else(|| {
                AttributionReplayError::unavailable(
                    AttributionUnavailable::StockCloseUnavailable,
                    true,
                    format!(
                        "stock close source is absent for {} {}",
                        row.code, parsed_date
                    ),
                )
            })?;
        selected.insert(
            key,
            StockCloseEvidence {
                code: row.code,
                date: parsed_date,
                close,
                data_source: row.data_source,
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
        );
    }
    for (code, date) in required_keys {
        if !selected.contains_key(&(code.clone(), *date)) {
            return Err(AttributionReplayError::unavailable(
                AttributionUnavailable::StockCloseUnavailable,
                true,
                format!("stock close is unavailable for {code} {date}"),
            ));
        }
    }
    let entries = selected.into_values().collect::<Vec<_>>();
    let manifest_hash = canonical_stock_close_manifest_hash(&entries);
    Ok(StockCloseManifest {
        entries,
        manifest_hash,
    })
}

fn update_len_prefixed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

pub fn canonical_stock_close_manifest_hash(entries: &[StockCloseEvidence]) -> String {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|left, right| (&left.code, left.date).cmp(&(&right.code, right.date)));
    let mut hasher = Sha256::new();
    hasher.update(STOCK_CLOSE_HASH_DOMAIN);
    hasher.update((sorted.len() as u64).to_be_bytes());
    for entry in sorted {
        update_len_prefixed(&mut hasher, entry.code.as_bytes());
        update_len_prefixed(
            &mut hasher,
            entry.date.format("%Y-%m-%d").to_string().as_bytes(),
        );
        hasher.update(entry.close.to_bits().to_be_bytes());
        match entry.data_source {
            Some(source) => {
                hasher.update([1]);
                update_len_prefixed(&mut hasher, source.as_bytes());
            }
            None => hasher.update([0]),
        }
        update_len_prefixed(&mut hasher, entry.created_at.as_bytes());
        update_len_prefixed(&mut hasher, entry.updated_at.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn canonical_fill_fee_evidence_hash(evidence: &FillFeeEvidence) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FEE_EVIDENCE_HASH_DOMAIN);
    hasher.update(evidence.fill_id.to_be_bytes());
    hasher.update(evidence.adverse_cost.to_bits().to_be_bytes());
    update_len_prefixed(&mut hasher, evidence.source.as_bytes());
    update_len_prefixed(&mut hasher, evidence.authority.as_bytes());
    update_len_prefixed(&mut hasher, evidence.evidence_id.as_bytes());
    hex::encode(hasher.finalize())
}

fn validate_fee_ledger(
    ledger: Option<&AuthoritativeFillFeeLedger>,
    fills: &[ReplayFillEvidence],
) -> Result<FeeEvidenceAvailability, AttributionReplayError> {
    let Some(ledger) = ledger else {
        return Ok(FeeEvidenceAvailability::Unavailable {
            code: AttributionUnavailable::FeeEvidenceUnavailable,
            retryable: false,
            detail: "explicit authoritative per-fill fee ledger is unavailable".to_owned(),
        });
    };
    let fill_ids = fills
        .iter()
        .map(|evidence| evidence.fill.id)
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    for entry in &ledger.entries {
        if !fill_ids.contains(&entry.fill_id) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::FeeEvidence,
                format!("fee evidence references unknown fill id={}", entry.fill_id),
            ));
        }
        if !seen.insert(entry.fill_id) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::FeeEvidence,
                format!("duplicate fee evidence for fill id={}", entry.fill_id),
            ));
        }
        if !entry.adverse_cost.is_finite()
            || entry.adverse_cost < 0.0
            || entry.source.trim().is_empty()
            || entry.authority.trim().is_empty()
            || entry.evidence_id.trim().is_empty()
            || !is_lowercase_sha256(&entry.evidence_hash)
            || canonical_fill_fee_evidence_hash(entry) != entry.evidence_hash
        {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::FeeEvidence,
                format!(
                    "invalid authoritative fee evidence for fill id={}",
                    entry.fill_id
                ),
            ));
        }
    }
    if seen != fill_ids {
        let missing = fill_ids.difference(&seen).copied().collect::<Vec<_>>();
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::FeeEvidence,
            format!("fee evidence is missing fill ids {missing:?}"),
        ));
    }
    Ok(FeeEvidenceAvailability::Available(ledger.clone()))
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

struct ValidatedReplayFeeState {
    ledger: Option<ValidatedReplayFeeLedger>,
    unavailable: Option<(AttributionUnavailable, String)>,
    basis: MetricAvailability<AttributionFeeBasis>,
}

fn canonical_replay_fee_basis_id(bindings: &[FeeEvidenceBinding]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(REPLAY_FEE_BASIS_HASH_DOMAIN);
    hasher.update((bindings.len() as u64).to_be_bytes());
    for binding in bindings {
        hasher.update(binding.fill_id.to_be_bytes());
        update_len_prefixed(&mut hasher, binding.evidence_hash.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn build_validated_replay_fee_ledger(
    availability: &FeeEvidenceAvailability,
    fills: &[ReplayFillEvidence],
) -> Result<ValidatedReplayFeeState, AttributionReplayError> {
    match availability {
        FeeEvidenceAvailability::Unavailable { code, detail, .. } => {
            if *code != AttributionUnavailable::FeeEvidenceUnavailable || detail.trim().is_empty() {
                return Err(AttributionReplayError::integrity(
                    AttributionIntegrityFailure::FeeEvidence,
                    "fee unavailability must use a nonblank FeeEvidenceUnavailable reason",
                ));
            }
            Ok(ValidatedReplayFeeState {
                ledger: None,
                unavailable: Some((*code, detail.clone())),
                basis: unavailable_metric(*code, detail.clone()),
            })
        }
        FeeEvidenceAvailability::Available(ledger) => {
            let validated = validate_fee_ledger(Some(ledger), fills)?;
            let FeeEvidenceAvailability::Available(validated) = validated else {
                return Err(AttributionReplayError::integrity(
                    AttributionIntegrityFailure::FeeEvidence,
                    "validated fee ledger unexpectedly became unavailable",
                ));
            };
            let mut entries = validated.entries;
            entries.sort_by_key(|entry| entry.fill_id);
            let bindings = entries
                .iter()
                .map(|entry| FeeEvidenceBinding {
                    fill_id: entry.fill_id,
                    evidence_hash: entry.evidence_hash.clone(),
                })
                .collect::<Vec<_>>();
            let basis_id = canonical_replay_fee_basis_id(&bindings);
            let costs = entries
                .into_iter()
                .map(|entry| EconomicFillCostEvidence {
                    fill_id: entry.fill_id,
                    adverse_cost: entry.adverse_cost,
                    evidence_id: entry.evidence_hash,
                })
                .collect();
            Ok(ValidatedReplayFeeState {
                ledger: Some(ValidatedReplayFeeLedger {
                    ledger: FillCostLedger {
                        basis_id: basis_id.clone(),
                        kind: CostBasisKind::Observed,
                        costs,
                    },
                }),
                unavailable: None,
                basis: MetricAvailability::Available(AttributionFeeBasis {
                    basis_id,
                    kind: CostBasisKind::Observed,
                    bindings,
                }),
            })
        }
    }
}

fn entry_family_bucket(
    composition: &[EntryFamilyComposition],
) -> Result<EntryFamilyBucket, AttributionReplayError> {
    let families = composition
        .iter()
        .map(|entry| entry.family)
        .collect::<Vec<_>>();
    match families.as_slice() {
        [] => Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::EconomicPosition,
            "economic cycle has no entry-family composition",
        )),
        [family] => Ok(EntryFamilyBucket::Single(*family)),
        _ => Ok(EntryFamilyBucket::Mixed(families)),
    }
}

fn unavailable_metric<T>(
    code: AttributionUnavailable,
    detail: impl Into<String>,
) -> MetricAvailability<T> {
    MetricAvailability::Unavailable {
        code,
        detail: detail.into(),
    }
}

fn checked_cardinality_add(
    left: usize,
    right: usize,
    failure: AttributionIntegrityFailure,
    field: &str,
) -> Result<usize, AttributionReplayError> {
    left.checked_add(right).ok_or_else(|| {
        AttributionReplayError::integrity(failure, format!("{field} cardinality overflowed"))
    })
}

fn checked_cardinality_sum(
    values: impl IntoIterator<Item = usize>,
    failure: AttributionIntegrityFailure,
    field: &str,
) -> Result<usize, AttributionReplayError> {
    values.into_iter().try_fold(0usize, |sum, value| {
        checked_cardinality_add(sum, value, failure, field)
    })
}

fn metric_coverage<'a, T: 'a>(
    metrics: impl IntoIterator<Item = &'a MetricAvailability<T>>,
) -> Result<MetricCoverage, AttributionReplayError> {
    let mut total_cycles = 0;
    let mut available_cycles = 0;
    let mut unavailable_reasons = BTreeMap::new();
    for metric in metrics {
        total_cycles = checked_cardinality_add(
            total_cycles,
            1,
            AttributionIntegrityFailure::EconomicPosition,
            "metric coverage total",
        )?;
        match metric {
            MetricAvailability::Available(_) => {
                available_cycles = checked_cardinality_add(
                    available_cycles,
                    1,
                    AttributionIntegrityFailure::EconomicPosition,
                    "metric coverage available",
                )?;
            }
            MetricAvailability::Unavailable { code, .. } => {
                let count = unavailable_reasons
                    .entry(code.code().to_owned())
                    .or_insert(0);
                *count = checked_cardinality_add(
                    *count,
                    1,
                    AttributionIntegrityFailure::EconomicPosition,
                    "metric coverage unavailable reason",
                )?;
            }
        }
    }
    let unavailable_cycles = total_cycles.checked_sub(available_cycles).ok_or_else(|| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::EconomicPosition,
            "metric coverage available count exceeds total",
        )
    })?;
    let coverage_ratio =
        (total_cycles > 0).then_some(available_cycles as f64 / total_cycles as f64);
    Ok(MetricCoverage {
        total_cycles,
        available_cycles,
        unavailable_cycles,
        coverage_ratio,
        unavailable_reasons,
    })
}

fn complete_metric_coverage(total_cycles: usize) -> MetricCoverage {
    MetricCoverage {
        total_cycles,
        available_cycles: total_cycles,
        unavailable_cycles: 0,
        coverage_ratio: (total_cycles > 0).then_some(1.0),
        unavailable_reasons: BTreeMap::new(),
    }
}

fn aggregate_available_values(
    coverage: MetricCoverage,
    mut values: Vec<f64>,
    field: &str,
) -> Result<MetricAggregate, AttributionReplayError> {
    let mut sum = 0.0;
    for value in &values {
        if !value.is_finite() {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::EconomicPosition,
                format!("{field} aggregate received a non-finite return"),
            ));
        }
        sum += value;
        if !sum.is_finite() {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::EconomicPosition,
                format!("{field} aggregate sum overflowed"),
            ));
        }
    }
    let (sum_return, mean_return, median_return) = if values.is_empty() {
        (None, None, None)
    } else {
        values.sort_by(f64::total_cmp);
        let mean = sum / values.len() as f64;
        let median = if values.len() % 2 == 1 {
            values[values.len() / 2]
        } else {
            let lower = values[values.len() / 2 - 1];
            let upper = values[values.len() / 2];
            let median = lower / 2.0 + upper / 2.0;
            if !median.is_finite() {
                return Err(AttributionReplayError::integrity(
                    AttributionIntegrityFailure::EconomicPosition,
                    format!("{field} aggregate median overflowed"),
                ));
            }
            median
        };
        if !mean.is_finite() {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::EconomicPosition,
                format!("{field} aggregate mean overflowed"),
            ));
        }
        (Some(sum), Some(mean), Some(median))
    };
    Ok(MetricAggregate {
        coverage,
        sum_return,
        mean_return,
        median_return,
    })
}

fn complete_metric_aggregate(
    values: impl IntoIterator<Item = f64>,
    field: &str,
) -> Result<MetricAggregate, AttributionReplayError> {
    let values = values.into_iter().collect::<Vec<_>>();
    aggregate_available_values(complete_metric_coverage(values.len()), values, field)
}

fn metric_aggregate<'a>(
    metrics: impl IntoIterator<Item = &'a MetricAvailability<f64>>,
    field: &str,
) -> Result<MetricAggregate, AttributionReplayError> {
    let metrics = metrics.into_iter().collect::<Vec<_>>();
    let coverage = metric_coverage(metrics.iter().copied())?;
    let values = metrics
        .iter()
        .filter_map(|metric| match metric {
            MetricAvailability::Available(value) => Some(*value),
            MetricAvailability::Unavailable { .. } => None,
        })
        .collect();
    aggregate_available_values(coverage, values, field)
}

fn outcome_summary(
    values: impl IntoIterator<Item = f64>,
) -> Result<OutcomeSummary, AttributionReplayError> {
    let mut wins = 0;
    let mut losses = 0;
    let mut breakeven = 0;
    for value in values {
        let counter = if value > 0.0 {
            &mut wins
        } else if value < 0.0 {
            &mut losses
        } else {
            &mut breakeven
        };
        *counter = checked_cardinality_add(
            *counter,
            1,
            AttributionIntegrityFailure::EconomicPosition,
            "outcome bucket",
        )?;
    }
    let directional_denominator = checked_cardinality_add(
        wins,
        losses,
        AttributionIntegrityFailure::EconomicPosition,
        "outcome directional denominator",
    )?;
    Ok(OutcomeSummary {
        wins,
        losses,
        breakeven,
        directional_denominator,
        win_rate: (directional_denominator > 0)
            .then_some(wins as f64 / directional_denominator as f64),
    })
}

#[derive(Debug, Clone, PartialEq)]
struct AttributionSliceSummary {
    gross: MetricAggregate,
    benchmark: MetricAggregate,
    gross_excess: MetricAggregate,
    net: MetricAggregate,
    net_excess: MetricAggregate,
    gross_outcome: MetricAvailability<OutcomeSummary>,
    net_outcome: MetricAvailability<OutcomeSummary>,
}

fn summarize_attribution_slice<'a>(
    cycles: impl IntoIterator<Item = &'a ClosedCycleAttribution>,
    fee_unavailable: Option<(AttributionUnavailable, &'a str)>,
    field: &str,
) -> Result<AttributionSliceSummary, AttributionReplayError> {
    let cycles = cycles.into_iter().collect::<Vec<_>>();
    let gross_outcome = MetricAvailability::Available(outcome_summary(
        cycles.iter().map(|cycle| cycle.gross_return),
    )?);
    let net_outcome = if let Some((code, detail)) = fee_unavailable {
        unavailable_metric(code, detail.to_owned())
    } else {
        let values = cycles
            .iter()
            .map(|cycle| match cycle.net_return {
                MetricAvailability::Available(value) => Ok(value),
                MetricAvailability::Unavailable { .. } => Err(AttributionReplayError::integrity(
                    AttributionIntegrityFailure::FeeEvidence,
                    format!("{field} complete fee ledger left an unavailable net cycle"),
                )),
            })
            .collect::<Result<Vec<_>, _>>()?;
        MetricAvailability::Available(outcome_summary(values)?)
    };
    Ok(AttributionSliceSummary {
        gross: complete_metric_aggregate(
            cycles.iter().map(|cycle| cycle.gross_return),
            &format!("{field}.gross"),
        )?,
        benchmark: metric_aggregate(
            cycles.iter().map(|cycle| &cycle.benchmark_return),
            &format!("{field}.benchmark"),
        )?,
        gross_excess: metric_aggregate(
            cycles.iter().map(|cycle| &cycle.gross_excess_return),
            &format!("{field}.gross_excess"),
        )?,
        net: metric_aggregate(
            cycles.iter().map(|cycle| &cycle.net_return),
            &format!("{field}.net"),
        )?,
        net_excess: metric_aggregate(
            cycles.iter().map(|cycle| &cycle.net_excess_return),
            &format!("{field}.net_excess"),
        )?,
        gross_outcome,
        net_outcome,
    })
}

fn align_cycle_benchmark(
    entry_at: DateTime<FixedOffset>,
    exit_at: DateTime<FixedOffset>,
    bars: &[BenchmarkBar],
    semantics: &MinuteLabelSemantics,
) -> Result<MetricAvailability<f64>, AttributionReplayError> {
    let entry = match align_completed_minute(entry_at, bars, semantics) {
        Ok(bar) => bar,
        Err(AttributionReplayError::Unavailable { code, detail, .. }) => {
            return Ok(unavailable_metric(code, format!("entry anchor: {detail}")));
        }
        Err(error) => return Err(error),
    };
    let exit = match align_completed_minute(exit_at, bars, semantics) {
        Ok(bar) => bar,
        Err(AttributionReplayError::Unavailable { code, detail, .. }) => {
            return Ok(unavailable_metric(code, format!("exit anchor: {detail}")));
        }
        Err(error) => return Err(error),
    };
    let value = exit.close / entry.close - 1.0;
    if !value.is_finite() {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::BenchmarkAlignment,
            "cycle benchmark return is non-finite",
        ));
    }
    Ok(MetricAvailability::Available(value))
}

fn validate_replay_fill_bindings(
    evidence: &AttributionReplayEvidence,
) -> Result<BTreeMap<i64, &ReplayFillEvidence>, AttributionReplayError> {
    if evidence.from > evidence.to {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::InvalidRequest,
            "attribution evidence from is after to",
        ));
    }
    let mut by_fill = BTreeMap::new();
    let mut audit_ids = BTreeSet::new();
    let mut audit_hashes = BTreeSet::new();
    for fill in &evidence.fills {
        if fill.terminal_audit_id <= 0
            || !audit_ids.insert(fill.terminal_audit_id)
            || !is_lowercase_sha256(&fill.terminal_audit_hash)
            || !audit_hashes.insert(fill.terminal_audit_hash.as_str())
            || !is_shanghai_offset(&fill.terminal_time)
            || fill.terminal_time.date_naive() > evidence.to
            || by_fill.insert(fill.fill.id, fill).is_some()
        {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::ReplayEvidence,
                format!(
                    "invalid replay terminal binding for fill id={}",
                    fill.fill.id
                ),
            ));
        }
    }
    Ok(by_fill)
}

fn exact_cycle_anchor(
    fill_id: i64,
    expected_code: &str,
    expected_direction: &str,
    by_fill: &BTreeMap<i64, &ReplayFillEvidence>,
) -> Result<DateTime<FixedOffset>, AttributionReplayError> {
    let evidence = by_fill.get(&fill_id).ok_or_else(|| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReplayEvidence,
            format!("economic cycle references unbound fill id={fill_id}"),
        )
    })?;
    if evidence.fill.code != expected_code || evidence.fill.direction != expected_direction {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReplayEvidence,
            format!("economic cycle fill id={fill_id} fields mismatch replay binding"),
        ));
    }
    Ok(evidence.terminal_time)
}

fn research_limitations() -> Vec<String> {
    vec![
        "authoritative_fee_evidence_completeness".to_owned(),
        "benchmark_alignment_completeness".to_owned(),
        "market_regime_evidence_unavailable".to_owned(),
        "code_entry_date_clustering_uncertainty".to_owned(),
        "gate_d_unmet".to_owned(),
        "production_integration_unverified".to_owned(),
    ]
}

fn attribution_conclusion(
    total_closed_cycles: usize,
    coverage_days: Option<i64>,
) -> AttributionConclusion {
    let mut sample_reasons = Vec::new();
    if total_closed_cycles < MIN_CLOSED_CYCLES {
        sample_reasons.push(format!(
            "closed_cycles_{total_closed_cycles}_below_{MIN_CLOSED_CYCLES}"
        ));
    }
    if coverage_days.is_none_or(|days| days < MIN_COVERAGE_DAYS) {
        sample_reasons.push(format!(
            "coverage_days_{}_below_{MIN_COVERAGE_DAYS}",
            coverage_days.map_or_else(|| "unavailable".to_owned(), |days| days.to_string())
        ));
    }
    if sample_reasons.is_empty() {
        AttributionConclusion::ResearchOnly {
            research_limitations: research_limitations(),
        }
    } else {
        AttributionConclusion::InsufficientSample {
            reasons: sample_reasons,
            research_limitations: research_limitations(),
        }
    }
}

/// BR-251 纯归因范围 seam；不访问数据库、provider、日历或持久化层。
pub fn compute_attribution_range(
    evidence: &AttributionReplayEvidence,
    benchmark_bars: &[BenchmarkBar],
    semantics: &MinuteLabelSemantics,
) -> Result<AttributionComputationReport, AttributionReplayError> {
    validate_replay_capability(evidence)?;
    let by_fill = validate_replay_fill_bindings(evidence)?;
    let rows = evidence
        .fills
        .iter()
        .map(|fill| fill.fill.clone())
        .collect::<Vec<_>>();
    let ValidatedReplayFeeState {
        ledger: fee_ledger,
        unavailable: fee_unavailable,
        basis: fee_basis,
    } = build_validated_replay_fee_ledger(&evidence.fees, &evidence.fills)?;
    let economic =
        rebuild_economic_positions_with_replay_fees(&rows, evidence.to, fee_ledger.as_ref())
            .map_err(|detail| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::EconomicPosition,
                    detail,
                )
            })?;

    let mut closed_cycles = Vec::new();
    for cycle in &economic.closed_positions {
        let entry_fill_id = *cycle.buy_fill_ids.first().ok_or_else(|| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::EconomicPosition,
                format!("closed cycle {} has no buy fill", cycle.cycle_open_fill_id),
            )
        })?;
        if entry_fill_id != cycle.cycle_open_fill_id {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::EconomicPosition,
                format!(
                    "closed cycle {} first buy identity mismatch",
                    cycle.cycle_open_fill_id
                ),
            ));
        }
        let exit_fill_id = *cycle.sell_fill_ids.last().ok_or_else(|| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::EconomicPosition,
                format!("closed cycle {} has no sell fill", cycle.cycle_open_fill_id),
            )
        })?;
        let entry_at = exact_cycle_anchor(entry_fill_id, &cycle.code, "buy", &by_fill)?;
        let exit_at = exact_cycle_anchor(exit_fill_id, &cycle.code, "sell", &by_fill)?;
        if exit_at.date_naive() < evidence.from || exit_at.date_naive() > evidence.to {
            continue;
        }
        if entry_at >= exit_at || cycle.gross_buy_notional <= 0.0 {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::EconomicPosition,
                format!(
                    "closed cycle {} anchors/notional invalid",
                    cycle.cycle_open_fill_id
                ),
            ));
        }
        let gross_return = cycle.gross_pnl / cycle.gross_buy_notional;
        if !gross_return.is_finite() {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::EconomicPosition,
                format!(
                    "closed cycle {} gross return invalid",
                    cycle.cycle_open_fill_id
                ),
            ));
        }
        let benchmark_return = align_cycle_benchmark(entry_at, exit_at, benchmark_bars, semantics)?;
        let gross_excess_return = match &benchmark_return {
            MetricAvailability::Available(value) => {
                let excess = gross_return - value;
                if !excess.is_finite() {
                    return Err(AttributionReplayError::integrity(
                        AttributionIntegrityFailure::EconomicPosition,
                        format!(
                            "closed cycle {} gross excess invalid",
                            cycle.cycle_open_fill_id
                        ),
                    ));
                }
                MetricAvailability::Available(excess)
            }
            MetricAvailability::Unavailable { code, detail } => {
                unavailable_metric(*code, detail.clone())
            }
        };
        let net_return = match &cycle.net {
            NetMetrics::Available {
                kind: CostBasisKind::Observed,
                return_on_buy_notional,
                ..
            } if return_on_buy_notional.is_finite() => {
                MetricAvailability::Available(*return_on_buy_notional)
            }
            NetMetrics::Available { .. } => {
                return Err(AttributionReplayError::integrity(
                    AttributionIntegrityFailure::FeeEvidence,
                    format!(
                        "closed cycle {} net result is not validated Observed evidence",
                        cycle.cycle_open_fill_id
                    ),
                ));
            }
            NetMetrics::Unavailable { .. } => {
                let (code, detail) = fee_unavailable.as_ref().ok_or_else(|| {
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::FeeEvidence,
                        "complete fee ledger produced unavailable cycle net metrics",
                    )
                })?;
                unavailable_metric(*code, detail.clone())
            }
        };
        let net_excess_return = match (&net_return, &benchmark_return) {
            (MetricAvailability::Available(net), MetricAvailability::Available(benchmark)) => {
                let excess = net - benchmark;
                if !excess.is_finite() {
                    return Err(AttributionReplayError::integrity(
                        AttributionIntegrityFailure::EconomicPosition,
                        format!(
                            "closed cycle {} net excess invalid",
                            cycle.cycle_open_fill_id
                        ),
                    ));
                }
                MetricAvailability::Available(excess)
            }
            (MetricAvailability::Unavailable { code, detail }, _) => {
                unavailable_metric(*code, detail.clone())
            }
            (_, MetricAvailability::Unavailable { code, detail }) => {
                unavailable_metric(*code, detail.clone())
            }
        };
        closed_cycles.push(ClosedCycleAttribution {
            cycle_open_fill_id: cycle.cycle_open_fill_id,
            code: cycle.code.clone(),
            entry_family: entry_family_bucket(&cycle.entry_composition)?,
            entry_composition: cycle.entry_composition.clone(),
            entry_terminal_time: entry_at,
            exit_terminal_time: exit_at,
            gross_return,
            benchmark_return,
            gross_excess_return,
            net_return,
            net_excess_return,
        });
    }
    closed_cycles.sort_by(|left, right| {
        (left.exit_terminal_time, left.cycle_open_fill_id)
            .cmp(&(right.exit_terminal_time, right.cycle_open_fill_id))
    });

    let total_closed_cycles = closed_cycles.len();
    let total_open_cycles = economic.open_positions.len();
    let mut source_fill_ids = economic.source_fill_ids.clone();
    source_fill_ids.sort_unstable();
    let coverage_days = match (closed_cycles.first(), closed_cycles.last()) {
        (Some(_), Some(_)) => {
            let first = closed_cycles
                .iter()
                .map(|cycle| cycle.entry_terminal_time.date_naive())
                .min()
                .ok_or_else(|| {
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::EconomicPosition,
                        "closed-cycle first entry is unavailable",
                    )
                })?;
            let last = closed_cycles
                .iter()
                .map(|cycle| cycle.exit_terminal_time.date_naive())
                .max()
                .ok_or_else(|| {
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::EconomicPosition,
                        "closed-cycle last exit is unavailable",
                    )
                })?;
            Some(last.signed_duration_since(first).num_days() + 1)
        }
        _ => None,
    };

    let overall = summarize_attribution_slice(
        closed_cycles.iter(),
        fee_unavailable
            .as_ref()
            .map(|(code, detail)| (*code, detail.as_str())),
        "overall",
    )?;
    let gross_win_rate = match &overall.gross_outcome {
        MetricAvailability::Available(outcome) => outcome.win_rate,
        MetricAvailability::Unavailable { .. } => unreachable!("gross outcome is always available"),
    };
    let net_win_rate = match &overall.net_outcome {
        MetricAvailability::Available(outcome) => MetricAvailability::Available(outcome.win_rate),
        MetricAvailability::Unavailable { code, detail } => {
            unavailable_metric(*code, detail.clone())
        }
    };

    let mut buckets = BTreeSet::new();
    buckets.extend(closed_cycles.iter().map(|cycle| cycle.entry_family.clone()));
    let mut open_buckets = Vec::with_capacity(economic.open_positions.len());
    for cycle in &economic.open_positions {
        let bucket = entry_family_bucket(&cycle.entry_composition)?;
        buckets.insert(bucket.clone());
        open_buckets.push(bucket);
    }
    let family_attribution = buckets
        .into_iter()
        .map(
            |bucket| -> Result<EntryFamilyAttribution, AttributionReplayError> {
                let cycles = closed_cycles
                    .iter()
                    .filter(|cycle| cycle.entry_family == bucket)
                    .collect::<Vec<_>>();
                let summary = summarize_attribution_slice(
                    cycles.iter().copied(),
                    fee_unavailable
                        .as_ref()
                        .map(|(code, detail)| (*code, detail.as_str())),
                    "family",
                )?;
                Ok(EntryFamilyAttribution {
                    bucket: bucket.clone(),
                    total_closed_cycles: cycles.len(),
                    total_open_cycles: checked_cardinality_sum(
                        open_buckets
                            .iter()
                            .filter(|open| **open == bucket)
                            .map(|_| 1),
                        AttributionIntegrityFailure::EconomicPosition,
                        "family open total",
                    )?,
                    gross: summary.gross,
                    benchmark: summary.benchmark,
                    gross_excess: summary.gross_excess,
                    net: summary.net,
                    net_excess: summary.net_excess,
                    gross_outcome: summary.gross_outcome,
                    net_outcome: summary.net_outcome,
                })
            },
        )
        .collect::<Result<Vec<_>, _>>()?;

    let conclusion = attribution_conclusion(total_closed_cycles, coverage_days);
    issue_attribution_report(AttributionComputationReport {
        from: evidence.from,
        to: evidence.to,
        canonical_source_fill_ids: source_fill_ids.clone(),
        total_closed_cycles,
        total_open_cycles,
        coverage_days,
        closed_cycles,
        family_attribution,
        gross: overall.gross,
        benchmark: overall.benchmark,
        gross_excess: overall.gross_excess,
        net: overall.net,
        net_excess: overall.net_excess,
        gross_outcome: overall.gross_outcome,
        net_outcome: overall.net_outcome,
        fee_basis,
        gross_win_rate,
        net_win_rate,
        conclusion,
        read_only_projection: AttributionComputationReportReadOnly { source_fill_ids },
        report_seal: None,
    })
}

/// 单日入口仅验证范围身份，随后调用同一纯引擎，保证同日 payload 无模式差异。
pub fn compute_attribution_daily(
    day: NaiveDate,
    evidence: &AttributionReplayEvidence,
    benchmark_bars: &[BenchmarkBar],
    semantics: &MinuteLabelSemantics,
) -> Result<AttributionComputationReport, AttributionReplayError> {
    if evidence.from != day || evidence.to != day {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::InvalidRequest,
            "daily attribution evidence must have from=to=day",
        ));
    }
    compute_attribution_range(evidence, benchmark_bars, semantics)
}

pub fn canonical_attribution_report_bytes(
    report: &AttributionComputationReport,
) -> Result<Vec<u8>, AttributionReplayError> {
    let normalized = validate_and_normalize_attribution_report_projection(report)?;
    let bytes = serialize_normalized_attribution_report_projection(&normalized)?;
    let expected = attribution_report_projection_seal(&bytes);
    if report.report_seal.as_ref() != Some(&expected) {
        return Err(canonical_report_error(
            "canonical report projection does not match its compute-issued seal",
        ));
    }
    Ok(bytes)
}

fn issue_attribution_report(
    draft: AttributionComputationReport,
) -> Result<AttributionComputationReport, AttributionReplayError> {
    if draft.report_seal.is_some() {
        return Err(canonical_report_error(
            "canonical report draft unexpectedly carries a seal",
        ));
    }
    let mut normalized = validate_and_normalize_attribution_report_projection(&draft)?;
    let bytes = serialize_normalized_attribution_report_projection(&normalized)?;
    normalized.report_seal = Some(attribution_report_projection_seal(&bytes));
    Ok(normalized)
}

fn serialize_normalized_attribution_report_projection(
    normalized: &AttributionComputationReport,
) -> Result<Vec<u8>, AttributionReplayError> {
    serde_json::to_vec(normalized).map_err(|error| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::CanonicalReport,
            format!("serialize canonical attribution report: {error}"),
        )
    })
}

fn attribution_report_projection_seal(bytes: &[u8]) -> AttributionComputationReportSeal {
    let mut hasher = Sha256::new();
    hasher.update(ATTRIBUTION_REPORT_SEAL_DOMAIN);
    hasher.update(bytes);
    AttributionComputationReportSeal(hasher.finalize().into())
}

pub fn canonical_attribution_report_hash(
    report: &AttributionComputationReport,
) -> Result<String, AttributionReplayError> {
    let bytes = canonical_attribution_report_bytes(report)?;
    let mut hasher = Sha256::new();
    hasher.update(ATTRIBUTION_REPORT_HASH_DOMAIN);
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn canonical_report_error(detail: impl Into<String>) -> AttributionReplayError {
    AttributionReplayError::integrity(AttributionIntegrityFailure::CanonicalReport, detail)
}

fn normalize_report_float(value: &mut f64, field: &str) -> Result<(), AttributionReplayError> {
    if !value.is_finite() {
        return Err(canonical_report_error(format!(
            "canonical report {field} is non-finite"
        )));
    }
    if *value == 0.0 {
        *value = 0.0;
    }
    Ok(())
}

fn normalize_report_optional_float(
    value: &mut Option<f64>,
    field: &str,
) -> Result<(), AttributionReplayError> {
    if let Some(value) = value {
        normalize_report_float(value, field)?;
    }
    Ok(())
}

fn normalize_report_metric(
    metric: &mut MetricAvailability<f64>,
    field: &str,
) -> Result<(), AttributionReplayError> {
    match metric {
        MetricAvailability::Available(value) => normalize_report_float(value, field),
        MetricAvailability::Unavailable { detail, .. } if detail.trim().is_empty() => Err(
            canonical_report_error(format!("canonical report {field} reason is blank")),
        ),
        MetricAvailability::Unavailable { .. } => Ok(()),
    }
}

fn normalize_report_coverage(
    coverage: &mut MetricCoverage,
    field: &str,
) -> Result<(), AttributionReplayError> {
    let covered_cycles = checked_cardinality_add(
        coverage.available_cycles,
        coverage.unavailable_cycles,
        AttributionIntegrityFailure::CanonicalReport,
        field,
    )?;
    let reason_cycles = checked_cardinality_sum(
        coverage.unavailable_reasons.values().copied(),
        AttributionIntegrityFailure::CanonicalReport,
        field,
    )?;
    if covered_cycles != coverage.total_cycles
        || reason_cycles != coverage.unavailable_cycles
        || coverage
            .unavailable_reasons
            .iter()
            .any(|(code, count)| code.trim().is_empty() || *count == 0)
    {
        return Err(canonical_report_error(format!(
            "canonical report {field} coverage counts are inconsistent"
        )));
    }
    normalize_report_optional_float(&mut coverage.coverage_ratio, field)?;
    let expected = (coverage.total_cycles > 0)
        .then_some(coverage.available_cycles as f64 / coverage.total_cycles as f64);
    if coverage.coverage_ratio != expected {
        return Err(canonical_report_error(format!(
            "canonical report {field} coverage ratio is inconsistent"
        )));
    }
    Ok(())
}

fn normalize_report_aggregate(
    aggregate: &mut MetricAggregate,
    field: &str,
) -> Result<(), AttributionReplayError> {
    normalize_report_coverage(&mut aggregate.coverage, field)?;
    normalize_report_optional_float(&mut aggregate.sum_return, field)?;
    normalize_report_optional_float(&mut aggregate.mean_return, field)?;
    normalize_report_optional_float(&mut aggregate.median_return, field)?;
    if aggregate.coverage.available_cycles == 0 {
        if aggregate.sum_return.is_some()
            || aggregate.mean_return.is_some()
            || aggregate.median_return.is_some()
        {
            return Err(canonical_report_error(format!(
                "canonical report {field} empty aggregate has return values"
            )));
        }
    } else if aggregate.sum_return.is_none()
        || aggregate.mean_return.is_none()
        || aggregate.median_return.is_none()
    {
        return Err(canonical_report_error(format!(
            "canonical report {field} available aggregate lacks return values"
        )));
    }
    Ok(())
}

fn normalize_outcome_summary(
    outcome: &mut OutcomeSummary,
    total_cycles: usize,
    field: &str,
) -> Result<(), AttributionReplayError> {
    normalize_report_optional_float(&mut outcome.win_rate, field)?;
    let directional_denominator = checked_cardinality_add(
        outcome.wins,
        outcome.losses,
        AttributionIntegrityFailure::CanonicalReport,
        field,
    )?;
    let classified_cycles = checked_cardinality_add(
        directional_denominator,
        outcome.breakeven,
        AttributionIntegrityFailure::CanonicalReport,
        field,
    )?;
    if classified_cycles != total_cycles
        || outcome.directional_denominator != directional_denominator
    {
        return Err(canonical_report_error(format!(
            "canonical report {field} outcome counts are inconsistent"
        )));
    }
    let expected = (outcome.directional_denominator > 0)
        .then_some(outcome.wins as f64 / outcome.directional_denominator as f64);
    if outcome.win_rate != expected {
        return Err(canonical_report_error(format!(
            "canonical report {field} outcome win rate is inconsistent"
        )));
    }
    Ok(())
}

fn normalize_outcome_metric(
    outcome: &mut MetricAvailability<OutcomeSummary>,
    total_cycles: usize,
    field: &str,
) -> Result<(), AttributionReplayError> {
    match outcome {
        MetricAvailability::Available(outcome) => {
            normalize_outcome_summary(outcome, total_cycles, field)
        }
        MetricAvailability::Unavailable { detail, .. } if detail.trim().is_empty() => Err(
            canonical_report_error(format!("canonical report {field} reason is blank")),
        ),
        MetricAvailability::Unavailable { .. } => Ok(()),
    }
}

fn validate_dependency_metric(
    dependent: &MetricAvailability<f64>,
    dependency: &MetricAvailability<f64>,
    gross_or_net: f64,
    field: &str,
) -> Result<(), AttributionReplayError> {
    match (dependent, dependency) {
        (MetricAvailability::Available(actual), MetricAvailability::Available(benchmark))
            if *actual == gross_or_net - benchmark =>
        {
            Ok(())
        }
        (
            MetricAvailability::Unavailable {
                code: actual_code, ..
            },
            MetricAvailability::Unavailable {
                code: dependency_code,
                ..
            },
        ) if actual_code == dependency_code => Ok(()),
        _ => Err(canonical_report_error(format!(
            "canonical report {field} dependency is inconsistent"
        ))),
    }
}

fn validate_and_normalize_attribution_report_projection(
    report: &AttributionComputationReport,
) -> Result<AttributionComputationReport, AttributionReplayError> {
    let mut report = report.clone();
    let family_closed_cycles = checked_cardinality_sum(
        report
            .family_attribution
            .iter()
            .map(|family| family.total_closed_cycles),
        AttributionIntegrityFailure::CanonicalReport,
        "family closed total",
    )?;
    let family_open_cycles = checked_cardinality_sum(
        report
            .family_attribution
            .iter()
            .map(|family| family.total_open_cycles),
        AttributionIntegrityFailure::CanonicalReport,
        "family open total",
    )?;
    if report.from > report.to
        || report.canonical_source_fill_ids != report.read_only_projection.source_fill_ids
        || report
            .canonical_source_fill_ids
            .iter()
            .any(|fill_id| *fill_id <= 0)
        || report
            .canonical_source_fill_ids
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || report.total_closed_cycles != report.closed_cycles.len()
        || family_closed_cycles != report.total_closed_cycles
        || family_open_cycles != report.total_open_cycles
    {
        return Err(canonical_report_error(
            "canonical report range or cycle counts are inconsistent",
        ));
    }
    let mut cycle_ids = BTreeSet::new();
    for cycle in &mut report.closed_cycles {
        if cycle.cycle_open_fill_id <= 0
            || !cycle_ids.insert(cycle.cycle_open_fill_id)
            || cycle.code.trim().is_empty()
            || !is_shanghai_offset(&cycle.entry_terminal_time)
            || !is_shanghai_offset(&cycle.exit_terminal_time)
            || cycle.entry_terminal_time >= cycle.exit_terminal_time
            || cycle.exit_terminal_time.date_naive() < report.from
            || cycle.exit_terminal_time.date_naive() > report.to
        {
            return Err(canonical_report_error(
                "canonical report closed-cycle identity/time is inconsistent",
            ));
        }
        let mut previous_family = None;
        for composition in &mut cycle.entry_composition {
            normalize_report_float(
                &mut composition.buy_notional,
                "cycle.entry_composition.buy_notional",
            )?;
            if composition.quantity == 0
                || composition.buy_notional <= 0.0
                || matches!(
                    composition.family,
                    SignalFamily::Unknown | SignalFamily::ExitByRule
                )
                || previous_family.is_some_and(|previous| previous >= composition.family)
            {
                return Err(canonical_report_error(
                    "canonical report cycle entry composition is inconsistent",
                ));
            }
            previous_family = Some(composition.family);
        }
        let expected_bucket = entry_family_bucket(&cycle.entry_composition).map_err(|_| {
            canonical_report_error("canonical report cycle entry composition is empty")
        })?;
        if cycle.entry_family != expected_bucket {
            return Err(canonical_report_error(
                "canonical report cycle family bucket does not match composition",
            ));
        }
        normalize_report_float(&mut cycle.gross_return, "cycle.gross_return")?;
        normalize_report_metric(&mut cycle.benchmark_return, "cycle.benchmark_return")?;
        normalize_report_metric(&mut cycle.gross_excess_return, "cycle.gross_excess_return")?;
        normalize_report_metric(&mut cycle.net_return, "cycle.net_return")?;
        normalize_report_metric(&mut cycle.net_excess_return, "cycle.net_excess_return")?;
        validate_dependency_metric(
            &cycle.gross_excess_return,
            &cycle.benchmark_return,
            cycle.gross_return,
            "cycle.gross_excess_return",
        )?;
        match (&cycle.net_excess_return, &cycle.net_return) {
            (
                MetricAvailability::Unavailable {
                    code: excess_code, ..
                },
                MetricAvailability::Unavailable { code: net_code, .. },
            ) if excess_code == net_code => {}
            (_, MetricAvailability::Available(net)) => validate_dependency_metric(
                &cycle.net_excess_return,
                &cycle.benchmark_return,
                *net,
                "cycle.net_excess_return",
            )?,
            _ => {
                return Err(canonical_report_error(
                    "canonical report cycle.net_excess_return dependency is inconsistent",
                ));
            }
        }
    }
    let expected_coverage_days = if report.closed_cycles.is_empty() {
        None
    } else {
        let first = report
            .closed_cycles
            .iter()
            .map(|cycle| cycle.entry_terminal_time.date_naive())
            .min()
            .ok_or_else(|| canonical_report_error("canonical report first entry is unavailable"))?;
        let last = report
            .closed_cycles
            .iter()
            .map(|cycle| cycle.exit_terminal_time.date_naive())
            .max()
            .ok_or_else(|| canonical_report_error("canonical report last exit is unavailable"))?;
        Some(last.signed_duration_since(first).num_days() + 1)
    };
    if report.coverage_days != expected_coverage_days
        || report.conclusion
            != attribution_conclusion(report.total_closed_cycles, expected_coverage_days)
    {
        return Err(canonical_report_error(
            "canonical report coverage days or conclusion is inconsistent",
        ));
    }
    normalize_report_aggregate(&mut report.gross, "gross")?;
    normalize_report_aggregate(&mut report.benchmark, "benchmark")?;
    normalize_report_aggregate(&mut report.gross_excess, "gross_excess")?;
    normalize_report_aggregate(&mut report.net, "net")?;
    normalize_report_aggregate(&mut report.net_excess, "net_excess")?;
    let fee_unavailable = match &report.fee_basis {
        MetricAvailability::Available(_) => None,
        MetricAvailability::Unavailable { code, detail } => Some((*code, detail.as_str())),
    };
    let expected_summary = summarize_attribution_slice(
        report.closed_cycles.iter(),
        fee_unavailable,
        "canonical.overall",
    )
    .map_err(|error| canonical_report_error(format!("canonical summary failed: {error}")))?;
    if report.gross != expected_summary.gross
        || report.benchmark != expected_summary.benchmark
        || report.gross_excess != expected_summary.gross_excess
        || report.net != expected_summary.net
        || report.net_excess != expected_summary.net_excess
    {
        return Err(canonical_report_error(
            "canonical report overall aggregate does not match cycles",
        ));
    }
    normalize_outcome_metric(
        &mut report.gross_outcome,
        report.total_closed_cycles,
        "gross_outcome",
    )?;
    if report.gross_outcome != expected_summary.gross_outcome {
        return Err(canonical_report_error(
            "canonical report gross outcome does not match cycles",
        ));
    }
    normalize_report_optional_float(&mut report.gross_win_rate, "gross_win_rate")?;
    let MetricAvailability::Available(expected_gross_outcome) = &report.gross_outcome else {
        return Err(canonical_report_error(
            "canonical report gross outcome is unavailable",
        ));
    };
    let expected_gross_win_rate = expected_gross_outcome.win_rate;
    if report.gross_win_rate != expected_gross_win_rate {
        return Err(canonical_report_error(
            "canonical report gross win rate is inconsistent",
        ));
    }
    normalize_outcome_metric(
        &mut report.net_outcome,
        report.total_closed_cycles,
        "net_outcome",
    )?;
    match &report.fee_basis {
        MetricAvailability::Available(basis) => {
            if basis.basis_id.trim().is_empty()
                || basis.kind != CostBasisKind::Observed
                || basis.basis_id != canonical_replay_fee_basis_id(&basis.bindings)
                || basis
                    .bindings
                    .iter()
                    .map(|binding| binding.fill_id)
                    .ne(report.canonical_source_fill_ids.iter().copied())
                || basis
                    .bindings
                    .windows(2)
                    .any(|pair| pair[0].fill_id >= pair[1].fill_id)
                || basis.bindings.iter().any(|binding| {
                    binding.fill_id <= 0 || !is_lowercase_sha256(&binding.evidence_hash)
                })
            {
                return Err(canonical_report_error(
                    "canonical report fee basis is invalid or unsorted",
                ));
            }
            if report
                .closed_cycles
                .iter()
                .any(|cycle| !matches!(cycle.net_return, MetricAvailability::Available(_)))
            {
                return Err(canonical_report_error(
                    "canonical report observed fee basis has unavailable net cycle",
                ));
            }
        }
        MetricAvailability::Unavailable { code, detail }
            if *code == AttributionUnavailable::FeeEvidenceUnavailable
                && !detail.trim().is_empty() =>
        {
            if report.closed_cycles.iter().any(|cycle| {
                !matches!(
                    cycle.net_return,
                    MetricAvailability::Unavailable {
                        code: AttributionUnavailable::FeeEvidenceUnavailable,
                        ..
                    }
                )
            }) {
                return Err(canonical_report_error(
                    "canonical report unavailable fee basis has available net cycle",
                ));
            }
        }
        MetricAvailability::Unavailable { .. } => {
            return Err(canonical_report_error(
                "canonical report fee basis unavailability is invalid",
            ));
        }
    }
    if report.net_outcome != expected_summary.net_outcome {
        return Err(canonical_report_error(
            "canonical report net outcome does not match fee basis/cycles",
        ));
    }
    match (&mut report.net_win_rate, &report.net_outcome) {
        (MetricAvailability::Available(actual), MetricAvailability::Available(outcome)) => {
            normalize_report_optional_float(actual, "net_win_rate")?;
            if *actual != outcome.win_rate {
                return Err(canonical_report_error(
                    "canonical report net win rate does not match net outcome",
                ));
            }
        }
        (
            MetricAvailability::Unavailable {
                code: actual_code,
                detail,
            },
            MetricAvailability::Unavailable {
                code: outcome_code, ..
            },
        ) if actual_code == outcome_code && !detail.trim().is_empty() => {}
        _ => {
            return Err(canonical_report_error(
                "canonical report net win rate availability does not match net outcome",
            ));
        }
    }
    let mut previous_bucket = None;
    for family in &mut report.family_attribution {
        if previous_bucket
            .as_ref()
            .is_some_and(|previous| previous >= &family.bucket)
        {
            return Err(canonical_report_error(
                "canonical report family buckets are duplicated or unsorted",
            ));
        }
        previous_bucket = Some(family.bucket.clone());
        let cycles = report
            .closed_cycles
            .iter()
            .filter(|cycle| cycle.entry_family == family.bucket)
            .collect::<Vec<_>>();
        if family.total_closed_cycles != cycles.len() {
            return Err(canonical_report_error(
                "canonical report family closed count is inconsistent",
            ));
        }
        normalize_report_aggregate(&mut family.gross, "family.gross")?;
        normalize_report_aggregate(&mut family.benchmark, "family.benchmark")?;
        normalize_report_aggregate(&mut family.gross_excess, "family.gross_excess")?;
        normalize_report_aggregate(&mut family.net, "family.net")?;
        normalize_report_aggregate(&mut family.net_excess, "family.net_excess")?;
        let family_fee_unavailable = match &report.fee_basis {
            MetricAvailability::Available(_) => None,
            MetricAvailability::Unavailable { code, detail } => Some((*code, detail.as_str())),
        };
        let expected_summary = summarize_attribution_slice(
            cycles.iter().copied(),
            family_fee_unavailable,
            "canonical.family",
        )
        .map_err(|error| {
            canonical_report_error(format!("canonical family summary failed: {error}"))
        })?;
        if family.gross != expected_summary.gross
            || family.benchmark != expected_summary.benchmark
            || family.gross_excess != expected_summary.gross_excess
            || family.net != expected_summary.net
            || family.net_excess != expected_summary.net_excess
        {
            return Err(canonical_report_error(
                "canonical report family aggregate does not match cycles",
            ));
        }
        normalize_outcome_metric(
            &mut family.gross_outcome,
            family.total_closed_cycles,
            "family.gross_outcome",
        )?;
        if family.gross_outcome != expected_summary.gross_outcome {
            return Err(canonical_report_error(
                "canonical report family gross outcome does not match cycles",
            ));
        }
        normalize_outcome_metric(
            &mut family.net_outcome,
            family.total_closed_cycles,
            "family.net_outcome",
        )?;
        if family.net_outcome != expected_summary.net_outcome {
            return Err(canonical_report_error(
                "canonical report family net outcome does not match fee basis/cycles",
            ));
        }
    }
    Ok(report)
}

#[cfg(test)]
type AfterReadTestHook = Box<dyn FnOnce() + Send + 'static>;

#[cfg(test)]
static AFTER_READ_TEST_HOOK: once_cell::sync::Lazy<std::sync::Mutex<Option<AfterReadTestHook>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

#[cfg(test)]
fn set_after_read_test_hook(hook: AfterReadTestHook) {
    *AFTER_READ_TEST_HOOK.lock().expect("TEST_CODE hook mutex") = Some(hook);
}

#[cfg(test)]
fn run_after_read_test_hook() {
    let hook = AFTER_READ_TEST_HOOK
        .lock()
        .expect("TEST_CODE hook mutex")
        .take();
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{DateTime, NaiveDate};
    use rusqlite::{params, Connection};

    use super::*;
    use crate::data_gateway::{BenchmarkBar, BenchmarkBarTime};
    use crate::database::order_audit::{
        canonical_order_audit_record_hash, CanonicalOrderAuditRow, AUDIT_CHAIN_GENESIS,
    };

    fn date(raw: &str) -> NaiveDate {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d").unwrap()
    }

    fn test_database_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "TEST_CODE_attribution_replay_{label}_{}_{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn create_schema(connection: &Connection) {
        connection
            .execute_batch(
                "CREATE TABLE paper_trades (
                    id INTEGER PRIMARY KEY, plan_id TEXT NOT NULL UNIQUE,
                    code TEXT NOT NULL, name TEXT NOT NULL, direction TEXT NOT NULL,
                    price REAL NOT NULL, quantity INTEGER NOT NULL, status TEXT NOT NULL,
                    fill_price REAL, virtual_reason TEXT NOT NULL, ts TEXT NOT NULL
                 );
                 CREATE TABLE order_audit (
                    id INTEGER PRIMARY KEY, business_order_id TEXT NOT NULL,
                    source TEXT NOT NULL, decision_basis TEXT NOT NULL, side TEXT NOT NULL,
                    code TEXT NOT NULL, requested_price REAL NOT NULL, execution_price REAL,
                    quantity INTEGER NOT NULL, quote_observed_at TEXT, outcome TEXT NOT NULL,
                    failure_reason TEXT, created_at TEXT NOT NULL
                 );
                 CREATE TABLE order_audit_chain (
                    order_audit_id INTEGER PRIMARY KEY, previous_hash TEXT NOT NULL,
                    record_hash TEXT NOT NULL, created_at TEXT NOT NULL
                 );
                 CREATE TABLE stock_daily (
                    id INTEGER PRIMARY KEY, code TEXT NOT NULL, date TEXT NOT NULL,
                    close REAL, data_source TEXT, created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );",
            )
            .unwrap();
    }

    fn append_filled_pair(
        connection: &Connection,
        id: i64,
        plan_id: &str,
        side: &str,
        price: f64,
        quote_observed_at: &str,
        paper_ts: &str,
        previous_hash: &str,
    ) -> String {
        connection
            .execute(
                "INSERT INTO paper_trades
                 (id,plan_id,code,name,direction,price,quantity,status,fill_price,virtual_reason,ts)
                 VALUES (?1,?2,'TEST_CODE_600001','TEST_CODE公司',?3,?4,100,'Filled',?4,?5,?6)",
                params![
                    id,
                    plan_id,
                    side,
                    price,
                    if side == "buy" {
                        "Breakout"
                    } else {
                        "ExitByRule"
                    },
                    paper_ts
                ],
            )
            .unwrap();
        let row = CanonicalOrderAuditRow {
            id,
            business_order_id: plan_id.to_owned(),
            source: "PaperTrade".to_owned(),
            decision_basis: "TEST_CODE terminal".to_owned(),
            side: side.to_owned(),
            code: "TEST_CODE_600001".to_owned(),
            requested_price: price,
            execution_price: Some(price),
            quantity: 100,
            quote_observed_at: Some(quote_observed_at.to_owned()),
            outcome: "Filled".to_owned(),
            failure_reason: None,
            created_at: "2026-08-22 00:00:00".to_owned(),
        };
        connection
            .execute(
                "INSERT INTO order_audit VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    row.id,
                    row.business_order_id,
                    row.source,
                    row.decision_basis,
                    row.side,
                    row.code,
                    row.requested_price,
                    row.execution_price,
                    row.quantity,
                    row.quote_observed_at,
                    row.outcome,
                    row.failure_reason,
                    row.created_at,
                ],
            )
            .unwrap();
        let record_hash = canonical_order_audit_record_hash(previous_hash, &row).unwrap();
        connection
            .execute(
                "INSERT INTO order_audit_chain VALUES (?1,?2,?3,'2026-08-22 00:00:01')",
                params![id, previous_hash, record_hash],
            )
            .unwrap();
        record_hash
    }

    fn complete_database(label: &str) -> PathBuf {
        let path = test_database_path(label);
        let connection = Connection::open(&path).unwrap();
        create_schema(&connection);
        let first_hash = append_filled_pair(
            &connection,
            1,
            "TEST_CODE_PLAN_1",
            "buy",
            10.0,
            "2026-08-20T01:31:05Z",
            "2026-08-20 09:31:05",
            AUDIT_CHAIN_GENESIS,
        );
        append_filled_pair(
            &connection,
            2,
            "TEST_CODE_PLAN_2",
            "sell",
            11.0,
            "2026-08-21T14:20:00+08:00",
            "2026-08-21 14:20:00",
            &first_hash,
        );
        for (id, day, close) in [(1, "2026-08-20", 10.2), (2, "2026-08-21", 11.1)] {
            connection
                .execute(
                    "INSERT INTO stock_daily VALUES (?1,'TEST_CODE_600001',?2,?3,'TEST_CODE_SOURCE','2026-08-22','2026-08-22')",
                    params![id, day, close],
                )
                .unwrap();
        }
        drop(connection);
        path
    }

    fn request_with_no_fees() -> AttributionReplayRequest {
        AttributionReplayRequest {
            from: date("2026-08-20"),
            to: date("2026-08-21"),
            required_trading_dates: vec![date("2026-08-20"), date("2026-08-21")],
            fee_ledger: None,
        }
    }

    fn audit_rows(connection: &Connection) -> Vec<CanonicalOrderAuditRow> {
        let mut statement = connection
            .prepare(
                "SELECT id,business_order_id,source,decision_basis,side,code,
                        requested_price,execution_price,quantity,quote_observed_at,
                        outcome,failure_reason,created_at FROM order_audit ORDER BY id",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok(CanonicalOrderAuditRow {
                    id: row.get(0)?,
                    business_order_id: row.get(1)?,
                    source: row.get(2)?,
                    decision_basis: row.get(3)?,
                    side: row.get(4)?,
                    code: row.get(5)?,
                    requested_price: row.get(6)?,
                    execution_price: row.get(7)?,
                    quantity: row.get(8)?,
                    quote_observed_at: row.get(9)?,
                    outcome: row.get(10)?,
                    failure_reason: row.get(11)?,
                    created_at: row.get(12)?,
                })
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn rehash_audits(connection: &Connection) {
        let rows = audit_rows(connection);
        connection
            .execute("DELETE FROM order_audit_chain", [])
            .unwrap();
        let mut previous = AUDIT_CHAIN_GENESIS.to_owned();
        for row in rows {
            let hash = canonical_order_audit_record_hash(&previous, &row).unwrap();
            connection
                .execute(
                    "INSERT INTO order_audit_chain VALUES (?1,?2,?3,'2026-08-22 00:00:01')",
                    params![row.id, previous, hash],
                )
                .unwrap();
            previous = hash;
        }
    }

    fn remove_database(path: PathBuf) {
        if std::env::var("TEST_CODE_KEEP_REPLAY_DB").as_deref() == Ok("1") {
            eprintln!("TEST_CODE_REPLAY_DB={}", path.display());
            return;
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn loader_returns_verified_history_and_typed_missing_fee() {
        let path = complete_database("happy");
        let request = request_with_no_fees();
        let before = path.metadata().unwrap();
        let before_count: i64 = Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM paper_trades", [], |row| row.get(0))
            .unwrap();

        let evidence = AttributionReplayLoader::new(&path)
            .load(&request)
            .expect("complete read-only evidence");
        assert_eq!(evidence.fills.len(), 2);
        assert_eq!(
            evidence.fills[0].terminal_time.to_rfc3339(),
            "2026-08-20T09:31:05+08:00"
        );
        assert_eq!(evidence.stock_closes.entries.len(), 2);
        assert_eq!(evidence.stock_closes.manifest_hash.len(), 64);
        assert!(matches!(
            evidence.fees,
            FeeEvidenceAvailability::Unavailable {
                code: AttributionUnavailable::FeeEvidenceUnavailable,
                ..
            }
        ));
        let after = path.metadata().unwrap();
        let after_count: i64 = Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM paper_trades", [], |row| row.get(0))
            .unwrap();
        assert_eq!(before.dev(), after.dev());
        assert_eq!(before.ino(), after.ino());
        assert_eq!(before_count, after_count);
        let readonly = open_query_only_connection(&path).unwrap();
        assert_eq!(
            readonly
                .query_row("PRAGMA query_only", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(readonly
            .execute("CREATE TABLE TEST_CODE_FORBIDDEN_WRITE(id INTEGER)", [])
            .is_err());
        assert!(path.is_file());
        remove_database(path);
    }

    #[test]
    fn loader_requires_an_existing_file_and_never_initializes_schema() {
        let missing = test_database_path("missing_file");
        assert!(matches!(
            AttributionReplayLoader::new(&missing).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::DatabaseIdentity,
                ..
            })
        ));
        assert!(!missing.exists());

        let empty = test_database_path("empty_schema");
        Connection::open(&empty).unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&empty).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::SourceRead,
                ..
            })
        ));
        let tables: i64 = Connection::open(&empty)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
        remove_database(empty);

        let path = complete_database("bad_authority_dates");
        let mut request = request_with_no_fees();
        request.required_trading_dates.reverse();
        assert!(matches!(
            AttributionReplayLoader::new(&path).load(&request),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::InvalidRequest,
                ..
            })
        ));
        remove_database(path);

        let empty_dates_path = complete_database("empty_authority_dates");
        let mut empty_dates = request_with_no_fees();
        empty_dates.required_trading_dates.clear();
        assert!(matches!(
            AttributionReplayLoader::new(&empty_dates_path).load(&empty_dates),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::InvalidRequest,
                ..
            })
        ));
        remove_database(empty_dates_path);
    }

    #[test]
    fn external_sqlite_lock_is_retryable_source_unavailable() {
        let path = complete_database("busy_source");
        let writer = Connection::open(&path).unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();

        assert!(matches!(
            AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::SourceUnavailable,
                retryable: true,
                ..
            })
        ));
        writer.execute_batch("ROLLBACK").unwrap();
        drop(writer);
        remove_database(path);
    }

    #[test]
    fn loader_rejects_main_file_replacement_during_the_read_snapshot() {
        let path = complete_database("replace_main");
        let displaced = path.with_extension("TEST_CODE_displaced.sqlite3");
        let hook_path = path.clone();
        let hook_displaced = displaced.clone();
        set_after_read_test_hook(Box::new(move || {
            std::fs::rename(&hook_path, &hook_displaced).unwrap();
            Connection::open(&hook_path).unwrap();
        }));

        assert!(matches!(
            AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::DatabaseIdentity,
                ..
            })
        ));
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(&displaced, &path).unwrap();
        remove_database(path);
    }

    #[test]
    fn loader_rejects_missing_duplicate_and_mismatched_filled_terminals() {
        let missing_path = complete_database("missing_terminal");
        let missing = Connection::open(&missing_path).unwrap();
        missing
            .execute("DELETE FROM order_audit_chain WHERE order_audit_id=2", [])
            .unwrap();
        missing
            .execute("DELETE FROM order_audit WHERE id=2", [])
            .unwrap();
        drop(missing);
        assert!(matches!(
            AttributionReplayLoader::new(&missing_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::TradeTimeUnavailable,
                ..
            })
        ));
        remove_database(missing_path);

        let duplicate_path = complete_database("duplicate_terminal");
        let duplicate = Connection::open(&duplicate_path).unwrap();
        duplicate
            .execute(
                "INSERT INTO order_audit VALUES
                 (3,'TEST_CODE_PLAN_2','PaperTrade','TEST_CODE duplicate','sell',
                  'TEST_CODE_600001',11.0,11.0,100,'2026-08-21T14:20:01+08:00',
                  'Filled',NULL,'2026-08-22 00:00:02')",
                [],
            )
            .unwrap();
        rehash_audits(&duplicate);
        drop(duplicate);
        assert!(matches!(
            AttributionReplayLoader::new(&duplicate_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::TerminalBinding,
                ..
            })
        ));
        remove_database(duplicate_path);

        let source_path = complete_database("source_mismatch");
        let source = Connection::open(&source_path).unwrap();
        source
            .execute(
                "UPDATE order_audit SET source='TEST_CODE_OTHER' WHERE id=2",
                [],
            )
            .unwrap();
        rehash_audits(&source);
        drop(source);
        assert!(matches!(
            AttributionReplayLoader::new(&source_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::TradeTimeUnavailable,
                ..
            })
        ));
        remove_database(source_path);

        for (label, update) in [
            (
                "code_mismatch",
                "UPDATE order_audit SET code='TEST_CODE_OTHER' WHERE id=2",
            ),
            (
                "side_mismatch",
                "UPDATE order_audit SET side='buy' WHERE id=2",
            ),
            (
                "request_price_mismatch",
                "UPDATE order_audit SET requested_price=11.1 WHERE id=2",
            ),
            (
                "execution_price_mismatch",
                "UPDATE order_audit SET execution_price=11.1 WHERE id=2",
            ),
            (
                "quantity_mismatch",
                "UPDATE order_audit SET quantity=200 WHERE id=2",
            ),
        ] {
            let path = complete_database(label);
            let connection = Connection::open(&path).unwrap();
            connection.execute(update, []).unwrap();
            rehash_audits(&connection);
            drop(connection);
            assert!(matches!(
                AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
                Err(AttributionReplayError::FailedIntegrity {
                    code: AttributionIntegrityFailure::TerminalBinding,
                    ..
                })
            ));
            remove_database(path);
        }
    }

    #[test]
    fn paper_fills_and_paper_terminals_are_a_bidirectional_exact_set() {
        let orphan_path = complete_database("orphan_paper_terminal");
        let orphan = Connection::open(&orphan_path).unwrap();
        orphan
            .execute(
                "INSERT INTO order_audit VALUES
                 (3,'TEST_CODE_ORPHAN_PLAN','PaperTrade','TEST_CODE orphan','buy',
                  'TEST_CODE_600002',20.0,20.0,100,'2026-08-21T14:30:00+08:00',
                  'Filled',NULL,'2026-08-22 00:00:02')",
                [],
            )
            .unwrap();
        rehash_audits(&orphan);
        drop(orphan);
        assert!(matches!(
            AttributionReplayLoader::new(&orphan_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::TerminalBinding,
                ..
            })
        ));
        remove_database(orphan_path);

        let other_source_path = complete_database("other_source_filled");
        let other_source = Connection::open(&other_source_path).unwrap();
        other_source
            .execute(
                "INSERT INTO order_audit VALUES
                 (3,'TEST_CODE_PLAN_2','TEST_CODE_BROKER','TEST_CODE unrelated source','sell',
                  'TEST_CODE_600001',11.0,11.0,100,'2026-08-21T14:20:01+08:00',
                  'Filled',NULL,'2026-08-22 00:00:02')",
                [],
            )
            .unwrap();
        rehash_audits(&other_source);
        drop(other_source);
        AttributionReplayLoader::new(&other_source_path)
            .load(&request_with_no_fees())
            .expect("Filled owned by another source is outside the PaperTrade join");
        remove_database(other_source_path);
    }

    #[test]
    fn rejected_retry_never_supplies_terminal_time_and_bad_rfc3339_fails_integrity() {
        let rejected_path = complete_database("rejected_retry");
        let rejected = Connection::open(&rejected_path).unwrap();
        rejected
            .execute(
                "UPDATE order_audit SET outcome='Rejected', failure_reason='TEST_CODE rejected'
                 WHERE id=2",
                [],
            )
            .unwrap();
        rehash_audits(&rejected);
        drop(rejected);
        assert!(matches!(
            AttributionReplayLoader::new(&rejected_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::TradeTimeUnavailable,
                ..
            })
        ));
        remove_database(rejected_path);

        let missing_time_path = complete_database("missing_quote_time");
        let missing_time = Connection::open(&missing_time_path).unwrap();
        missing_time
            .execute(
                "UPDATE order_audit SET quote_observed_at=NULL WHERE id=2",
                [],
            )
            .unwrap();
        rehash_audits(&missing_time);
        drop(missing_time);
        assert!(matches!(
            AttributionReplayLoader::new(&missing_time_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::TradeTimeUnavailable,
                ..
            })
        ));
        remove_database(missing_time_path);

        let bad_time_path = complete_database("bad_rfc3339");
        let bad_time = Connection::open(&bad_time_path).unwrap();
        bad_time
            .execute(
                "UPDATE order_audit SET quote_observed_at='2026-08-21 14:20:00' WHERE id=2",
                [],
            )
            .unwrap();
        rehash_audits(&bad_time);
        drop(bad_time);
        assert!(matches!(
            AttributionReplayLoader::new(&bad_time_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::TerminalBinding,
                ..
            })
        ));
        remove_database(bad_time_path);
    }

    #[test]
    fn loader_validates_future_source_before_range_and_retains_fifo_prehistory() {
        let path = complete_database("source_before_filter");
        let evidence = AttributionReplayLoader::new(&path)
            .load(&AttributionReplayRequest {
                from: date("2026-08-21"),
                to: date("2026-08-21"),
                required_trading_dates: vec![date("2026-08-21")],
                fee_ledger: None,
            })
            .unwrap();
        assert_eq!(
            evidence
                .fills
                .iter()
                .map(|fill| fill.fill.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO paper_trades VALUES
                 (3,'TEST_CODE_PLAN_3','TEST_CODE_600001','TEST_CODE公司','hold',12.0,
                  100,'Filled',12.0,'ExitByRule','2026-08-25 10:00:00')",
                [],
            )
            .unwrap();
        drop(connection);
        assert!(matches!(
            AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::PaperTradeSource,
                ..
            })
        ));
        remove_database(path);

        let t1_path = complete_database("future_t1");
        let t1 = Connection::open(&t1_path).unwrap();
        t1.execute_batch(
            "INSERT INTO paper_trades VALUES
             (3,'TEST_CODE_PLAN_3','TEST_CODE_600001','TEST_CODE公司','buy',12.0,
              100,'Filled',12.0,'Breakout','2026-08-25 10:00:00');
             INSERT INTO paper_trades VALUES
             (4,'TEST_CODE_PLAN_4','TEST_CODE_600001','TEST_CODE公司','sell',12.1,
              100,'Filled',12.1,'ExitByRule','2026-08-25 14:00:00');",
        )
        .unwrap();
        drop(t1);
        let error = AttributionReplayLoader::new(&t1_path)
            .load(&request_with_no_fees())
            .unwrap_err();
        assert!(matches!(
            error,
            AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::PaperTradeSource,
                ..
            }
        ));
        assert!(error.to_string().contains("T+1"));
        remove_database(t1_path);
    }

    #[test]
    fn later_fractional_fills_are_validated_before_an_earlier_projection() {
        let path = complete_database("later_fractional_source");
        let connection = Connection::open(&path).unwrap();
        let previous_hash: String = connection
            .query_row(
                "SELECT record_hash FROM order_audit_chain WHERE order_audit_id=2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let buy_hash = append_filled_pair(
            &connection,
            3,
            "TEST_CODE_PLAN_3",
            "buy",
            12.0,
            "2026-08-25T10:00:00.123+08:00",
            "2026-08-25 10:00:00.123",
            &previous_hash,
        );
        append_filled_pair(
            &connection,
            4,
            "TEST_CODE_PLAN_4",
            "sell",
            12.1,
            "2026-08-26T10:00:00.456+08:00",
            "2026-08-26 10:00:00.456",
            &buy_hash,
        );
        drop(connection);

        let evidence = AttributionReplayLoader::new(&path)
            .load(&request_with_no_fees())
            .expect("valid later fractional source must not poison earlier projection");
        assert_eq!(
            evidence
                .fills
                .iter()
                .map(|fill| fill.fill.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        remove_database(path);
    }

    #[test]
    fn close_manifest_is_order_independent_and_missing_or_bad_close_fails_closed() {
        let entries = vec![
            StockCloseEvidence {
                code: "TEST_CODE_B".to_owned(),
                date: date("2026-08-21"),
                close: 20.0,
                data_source: Some("TEST_CODE_SOURCE".to_owned()),
                created_at: "2026-08-22".to_owned(),
                updated_at: "2026-08-22".to_owned(),
            },
            StockCloseEvidence {
                code: "TEST_CODE_A".to_owned(),
                date: date("2026-08-20"),
                close: 10.0,
                data_source: None,
                created_at: "2026-08-22".to_owned(),
                updated_at: "2026-08-22".to_owned(),
            },
        ];
        let mut reordered = entries.clone();
        reordered.reverse();
        assert_eq!(
            canonical_stock_close_manifest_hash(&entries),
            canonical_stock_close_manifest_hash(&reordered)
        );

        let missing_path = complete_database("missing_close");
        Connection::open(&missing_path)
            .unwrap()
            .execute("DELETE FROM stock_daily WHERE date='2026-08-21'", [])
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&missing_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::StockCloseUnavailable,
                ..
            })
        ));
        remove_database(missing_path);

        let null_path = complete_database("null_close");
        Connection::open(&null_path)
            .unwrap()
            .execute("UPDATE stock_daily SET close=NULL WHERE id=2", [])
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&null_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::StockCloseUnavailable,
                ..
            })
        ));
        remove_database(null_path);

        let duplicate_path = complete_database("duplicate_close");
        Connection::open(&duplicate_path)
            .unwrap()
            .execute(
                "INSERT INTO stock_daily VALUES
                 (3,'TEST_CODE_600001','2026-08-21',11.2,'TEST_CODE_SOURCE',
                  '2026-08-22','2026-08-22')",
                [],
            )
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&duplicate_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::StockCloseSource,
                ..
            })
        ));
        remove_database(duplicate_path);

        let invalid_date_path = complete_database("invalid_close_date");
        Connection::open(&invalid_date_path)
            .unwrap()
            .execute("UPDATE stock_daily SET date='2026-02-30' WHERE id=2", [])
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&invalid_date_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::StockCloseUnavailable,
                ..
            })
        ));
        remove_database(invalid_date_path);

        for (label, value) in [
            ("zero_close", "0.0"),
            ("negative_close", "-1.0"),
            ("infinite_close", "1e999"),
        ] {
            let path = complete_database(label);
            Connection::open(&path)
                .unwrap()
                .execute(
                    &format!("UPDATE stock_daily SET close={value} WHERE id=2"),
                    [],
                )
                .unwrap();
            assert!(matches!(
                AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
                Err(AttributionReplayError::FailedIntegrity {
                    code: AttributionIntegrityFailure::StockCloseSource,
                    ..
                })
            ));
            remove_database(path);
        }
    }

    #[test]
    fn required_close_requires_a_nonblank_source_identity() {
        for (label, source) in [
            ("missing_close_source", "NULL"),
            ("blank_close_source", "'   '"),
        ] {
            let path = complete_database(label);
            Connection::open(&path)
                .unwrap()
                .execute(
                    &format!("UPDATE stock_daily SET data_source={source} WHERE id=2"),
                    [],
                )
                .unwrap();
            assert!(matches!(
                AttributionReplayLoader::new(&path).load(&request_with_no_fees()),
                Err(AttributionReplayError::Unavailable {
                    code: AttributionUnavailable::StockCloseUnavailable,
                    ..
                })
            ));
            remove_database(path);
        }
    }

    #[test]
    fn close_loading_uses_only_range_relevant_exact_keys() {
        let unrelated_path = complete_database("unrelated_bad_stock_row");
        Connection::open(&unrelated_path)
            .unwrap()
            .execute(
                "INSERT INTO stock_daily VALUES
                 (3,'TEST_CODE_OTHER','2026-08-21',X'00','TEST_CODE_SOURCE',
                  '2026-08-22','2026-08-22')",
                [],
            )
            .unwrap();
        AttributionReplayLoader::new(&unrelated_path)
            .load(&request_with_no_fees())
            .expect("unrelated bad stock row must never be decoded");
        remove_database(unrelated_path);

        let preclosed_path = complete_database("fully_closed_before_range");
        let evidence = AttributionReplayLoader::new(&preclosed_path)
            .load(&AttributionReplayRequest {
                from: date("2026-08-25"),
                to: date("2026-08-25"),
                required_trading_dates: vec![date("2026-08-25")],
                fee_ledger: None,
            })
            .expect("fully closed pre-range lifecycle requires no future close");
        assert!(evidence.stock_closes.entries.is_empty());
        remove_database(preclosed_path);

        let required_bad_path = complete_database("required_bad_stock_row");
        Connection::open(&required_bad_path)
            .unwrap()
            .execute("UPDATE stock_daily SET close=X'00' WHERE id=2", [])
            .unwrap();
        assert!(matches!(
            AttributionReplayLoader::new(&required_bad_path).load(&request_with_no_fees()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::SourceRead,
                ..
            }) | Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::StockCloseSource,
                ..
            })
        ));
        remove_database(required_bad_path);
    }

    #[test]
    fn exact_close_query_chunks_below_the_sqlite_variable_limit() {
        let path = complete_database("chunked_exact_close_keys");
        let mut connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "DELETE FROM order_audit_chain WHERE order_audit_id=2;
                 DELETE FROM order_audit WHERE id=2;
                 DELETE FROM paper_trades WHERE id=2;
                 DELETE FROM stock_daily;",
            )
            .unwrap();
        let first = date("2026-08-20");
        let required_dates = (0..=STOCK_CLOSE_KEYS_PER_QUERY)
            .map(|offset| first + chrono::Duration::days(offset as i64))
            .collect::<Vec<_>>();
        let transaction = connection.transaction().unwrap();
        for (offset, required_date) in required_dates.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO stock_daily VALUES
                     (?1,'TEST_CODE_600001',?2,10.0,'TEST_CODE_SOURCE',
                      '2026-08-22','2026-08-22')",
                    params![
                        offset as i64 + 1,
                        required_date.format("%Y-%m-%d").to_string()
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        drop(connection);

        let evidence = AttributionReplayLoader::new(&path)
            .load(&AttributionReplayRequest {
                from: first,
                to: *required_dates.last().unwrap(),
                required_trading_dates: required_dates,
                fee_ledger: None,
            })
            .expect("401 exact keys must cross the fixed 400-key query boundary");
        assert_eq!(
            evidence.stock_closes.entries.len(),
            STOCK_CLOSE_KEYS_PER_QUERY + 1
        );
        remove_database(path);
    }

    fn fee(fill_id: i64, adverse_cost: f64) -> FillFeeEvidence {
        let mut evidence = FillFeeEvidence {
            fill_id,
            adverse_cost,
            source: "TEST_CODE_BROKER_LEDGER".to_owned(),
            authority: "TEST_CODE_SIGNED_EXPORT".to_owned(),
            evidence_id: format!("TEST_CODE_FEE_{fill_id}"),
            evidence_hash: String::new(),
        };
        evidence.evidence_hash = canonical_fill_fee_evidence_hash(&evidence);
        evidence
    }

    #[test]
    fn fee_ledger_requires_exact_authoritative_one_to_one_evidence() {
        let path = complete_database("fees");
        let mut request = request_with_no_fees();
        request.fee_ledger = Some(AuthoritativeFillFeeLedger {
            entries: vec![fee(1, 1.25), fee(2, 1.50)],
        });
        assert!(matches!(
            AttributionReplayLoader::new(&path)
                .load(&request)
                .unwrap()
                .fees,
            FeeEvidenceAvailability::Available(_)
        ));

        let invalid_ledgers = vec![
            vec![fee(1, 1.25)],
            vec![fee(1, 1.25), fee(1, 1.50)],
            vec![fee(1, 1.25), fee(2, 1.50), fee(3, 1.0)],
            {
                let mut entries = vec![fee(1, 1.25), fee(2, 1.50)];
                entries[0].source.clear();
                entries
            },
            {
                let mut entries = vec![fee(1, 1.25), fee(2, 1.50)];
                entries[0].evidence_hash = "A".repeat(64);
                entries
            },
            vec![fee(1, -1.0), fee(2, 1.50)],
            vec![fee(1, f64::NAN), fee(2, 1.50)],
        ];
        for entries in invalid_ledgers {
            request.fee_ledger = Some(AuthoritativeFillFeeLedger { entries });
            assert!(matches!(
                AttributionReplayLoader::new(&path).load(&request),
                Err(AttributionReplayError::FailedIntegrity {
                    code: AttributionIntegrityFailure::FeeEvidence,
                    ..
                })
            ));
        }
        remove_database(path);
    }

    fn instant(raw: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(raw).unwrap()
    }

    fn minute_bar(raw: &str, close: f64) -> BenchmarkBar {
        BenchmarkBar {
            at: BenchmarkBarTime::MinuteEnd(instant(raw)),
            open: close,
            high: close,
            low: close,
            close,
            volume: None,
            amount: None,
        }
    }

    fn verified_minute_labels() -> MinuteLabelSemantics {
        MinuteLabelSemantics::EndLabelVerified {
            evidence_hash: "a".repeat(64),
        }
    }

    #[test]
    fn completed_minute_alignment_requires_verified_end_labels_and_exact_shanghai_grid() {
        let bars = vec![minute_bar("2026-08-20T09:31:00+08:00", 4100.0)];
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-20T09:31:05+08:00"),
                &bars,
                &MinuteLabelSemantics::Unverified,
            ),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::BenchmarkTimeSemanticsUnavailable,
                ..
            })
        ));
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-20T09:31:05+08:00"),
                &bars,
                &MinuteLabelSemantics::EndLabelVerified {
                    evidence_hash: "   ".to_owned(),
                },
            ),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::BenchmarkTimeSemanticsUnavailable,
                ..
            })
        ));

        let wrong_offset = vec![minute_bar("2026-08-20T01:31:00Z", 4100.0)];
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-20T09:31:05+08:00"),
                &wrong_offset,
                &verified_minute_labels(),
            ),
            Err(AttributionReplayError::FailedIntegrity { .. })
        ));
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-20T01:31:05Z"),
                &bars,
                &verified_minute_labels(),
            ),
            Err(AttributionReplayError::FailedIntegrity { .. })
        ));
        let off_grid = vec![minute_bar("2026-08-20T09:31:00.001+08:00", 4100.0)];
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-20T09:31:05+08:00"),
                &off_grid,
                &verified_minute_labels(),
            ),
            Err(AttributionReplayError::FailedIntegrity { .. })
        ));
    }

    #[test]
    fn completed_minute_alignment_is_strictly_before_fill_and_never_crosses_a_break() {
        let bars = vec![
            minute_bar("2026-08-20T09:31:00+08:00", 4100.0),
            minute_bar("2026-08-20T09:32:00+08:00", 4101.0),
            minute_bar("2026-08-20T11:30:00+08:00", 4110.0),
            minute_bar("2026-08-20T13:01:00+08:00", 4120.0),
            minute_bar("2026-08-20T14:59:00+08:00", 4130.0),
            minute_bar("2026-08-20T15:00:00+08:00", 4140.0),
        ];
        assert_eq!(
            align_completed_minute(
                instant("2026-08-20T09:31:05+08:00"),
                &bars,
                &verified_minute_labels(),
            )
            .unwrap()
            .close,
            4100.0
        );
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-20T09:31:00+08:00"),
                &bars,
                &verified_minute_labels(),
            ),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::BenchmarkAlignmentUnavailable,
                ..
            })
        ));
        assert_eq!(
            align_completed_minute(
                instant("2026-08-20T09:32:00+08:00"),
                &bars,
                &verified_minute_labels(),
            )
            .unwrap()
            .close,
            4100.0
        );
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-20T09:32:00.000000001+08:00"),
                &bars[..1],
                &verified_minute_labels(),
            ),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::BenchmarkAlignmentUnavailable,
                ..
            })
        ));
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-20T13:01:00+08:00"),
                &bars,
                &verified_minute_labels(),
            ),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::BenchmarkAlignmentUnavailable,
                ..
            })
        ));
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-20T12:00:00+08:00"),
                &bars,
                &verified_minute_labels(),
            ),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::BenchmarkAlignmentUnavailable,
                ..
            })
        ));
        assert_eq!(
            align_completed_minute(
                instant("2026-08-20T15:00:00+08:00"),
                &bars,
                &verified_minute_labels(),
            )
            .unwrap()
            .close,
            4130.0
        );
    }

    #[test]
    fn completed_minute_alignment_rejects_cross_day_duplicate_and_non_minute_inputs() {
        let semantics = verified_minute_labels();
        assert!(matches!(
            align_completed_minute(
                instant("2026-08-21T09:31:05+08:00"),
                &[minute_bar("2026-08-20T15:00:00+08:00", 4100.0)],
                &semantics,
            ),
            Err(AttributionReplayError::Unavailable {
                code: AttributionUnavailable::BenchmarkAlignmentUnavailable,
                ..
            })
        ));
        let duplicate = vec![
            minute_bar("2026-08-20T09:31:00+08:00", 4100.0),
            minute_bar("2026-08-20T09:31:00+08:00", 4100.0),
        ];
        assert!(matches!(
            align_completed_minute(instant("2026-08-20T09:31:05+08:00"), &duplicate, &semantics,),
            Err(AttributionReplayError::FailedIntegrity { .. })
        ));
        let daily = BenchmarkBar {
            at: BenchmarkBarTime::Daily(date("2026-08-20")),
            open: 4100.0,
            high: 4100.0,
            low: 4100.0,
            close: 4100.0,
            volume: None,
            amount: None,
        };
        assert!(matches!(
            align_completed_minute(instant("2026-08-20T09:31:05+08:00"), &[daily], &semantics,),
            Err(AttributionReplayError::FailedIntegrity { .. })
        ));
    }

    fn replay_fill(
        id: i64,
        code: &str,
        direction: &str,
        price: f64,
        quantity: i64,
        occurred_at: &str,
        terminal_at: &str,
        reason: &str,
    ) -> ReplayFillEvidence {
        ReplayFillEvidence {
            fill: EconomicFillRow {
                id,
                plan_id: format!("TEST_CODE_REPLAY_PLAN_{id}"),
                code: code.to_owned(),
                name: format!("TEST_CODE_{code}"),
                direction: direction.to_owned(),
                fill_price: Some(price),
                quantity,
                occurred_at: occurred_at.to_owned(),
                virtual_reason: reason.to_owned(),
            },
            terminal_audit_id: 10_000 + id,
            terminal_audit_hash: format!("{:064x}", 10_000 + id),
            terminal_time: instant(terminal_at),
        }
    }

    fn replay_evidence(
        from: &str,
        to: &str,
        fills: Vec<ReplayFillEvidence>,
        fees: Option<Vec<FillFeeEvidence>>,
    ) -> AttributionReplayEvidence {
        AttributionReplayEvidence::issued(
            date(from),
            date(to),
            fills,
            StockCloseManifest {
                entries: Vec::new(),
                manifest_hash: "b".repeat(64),
            },
            fees.map_or_else(
                || FeeEvidenceAvailability::Unavailable {
                    code: AttributionUnavailable::FeeEvidenceUnavailable,
                    retryable: false,
                    detail: "TEST_CODE fee ledger unavailable".to_owned(),
                },
                |entries| {
                    FeeEvidenceAvailability::Available(AuthoritativeFillFeeLedger { entries })
                },
            ),
        )
    }

    #[test]
    fn pure_attribution_computes_gross_benchmark_excess_and_observed_net_per_cycle() {
        let fills = vec![
            replay_fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-08-20 09:31:05",
                "2026-08-20T09:31:05+08:00",
                "Momentum",
            ),
            replay_fill(
                2,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-08-21 09:32:05",
                "2026-08-21T09:32:05+08:00",
                "ExitByRule",
            ),
        ];
        let evidence = replay_evidence(
            "2026-08-20",
            "2026-08-21",
            fills,
            Some(vec![fee(1, 5.0), fee(2, 5.0)]),
        );
        let bars = vec![
            minute_bar("2026-08-20T09:31:00+08:00", 100.0),
            minute_bar("2026-08-21T09:32:00+08:00", 110.0),
        ];

        let report = compute_attribution_range(&evidence, &bars, &verified_minute_labels())
            .expect("complete TEST_CODE evidence computes research metrics");

        assert_eq!(report.total_closed_cycles, 1);
        assert_eq!(report.total_open_cycles, 0);
        assert_eq!(report.closed_cycles[0].cycle_open_fill_id, 1);
        assert_eq!(report.closed_cycles[0].gross_return, 0.1);
        assert!(matches!(
            report.closed_cycles[0].benchmark_return,
            MetricAvailability::Available(value) if (value - 0.1).abs() < 1e-12
        ));
        assert!(matches!(
            report.closed_cycles[0].gross_excess_return,
            MetricAvailability::Available(value) if value.abs() < 1e-12
        ));
        assert!(matches!(
            report.closed_cycles[0].net_return,
            MetricAvailability::Available(value) if (value - 0.09).abs() < 1e-12
        ));
        assert!(matches!(
            report.closed_cycles[0].net_excess_return,
            MetricAvailability::Available(value) if (value + 0.01).abs() < 1e-12
        ));
        assert!(matches!(
            report.net_win_rate,
            MetricAvailability::Available(Some(value)) if value == 1.0
        ));
    }

    #[test]
    fn loader_capability_rejects_terminal_and_fee_rebinding_after_issuance() {
        let fills = vec![
            replay_fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-08-20 09:31:05",
                "2026-08-20T09:31:05+08:00",
                "Momentum",
            ),
            replay_fill(
                2,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-08-21 09:32:05",
                "2026-08-21T09:32:05+08:00",
                "ExitByRule",
            ),
        ];
        let bars = vec![
            minute_bar("2026-08-20T09:31:00+08:00", 100.0),
            minute_bar("2026-08-21T09:32:00+08:00", 110.0),
        ];

        let mut rebound_terminal = replay_evidence(
            "2026-08-20",
            "2026-08-21",
            fills.clone(),
            Some(vec![fee(1, 1.0), fee(2, 1.0)]),
        );
        rebound_terminal.fills[0].terminal_time = instant("2026-08-20T09:31:06+08:00");
        assert!(matches!(
            compute_attribution_range(&rebound_terminal, &bars, &verified_minute_labels()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::ReplayEvidence,
                ..
            })
        ));

        let mut rebound_fee = replay_evidence(
            "2026-08-20",
            "2026-08-21",
            fills,
            Some(vec![fee(1, 1.0), fee(2, 1.0)]),
        );
        rebound_fee.fees = FeeEvidenceAvailability::Available(AuthoritativeFillFeeLedger {
            entries: vec![fee(1, 9.0), fee(2, 9.0)],
        });
        assert!(matches!(
            compute_attribution_range(&rebound_fee, &bars, &verified_minute_labels()),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::ReplayEvidence,
                ..
            })
        ));
    }

    #[test]
    fn pure_attribution_keeps_mixed_partial_cycle_open_censoring_and_independent_dimensions() {
        let fills = vec![
            replay_fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-08-20 09:31:05",
                "2026-08-20T09:31:05+08:00",
                "Momentum",
            ),
            replay_fill(
                2,
                "TEST_CODE_600001",
                "buy",
                12.0,
                100,
                "2026-08-20 09:32:05",
                "2026-08-20T09:32:05+08:00",
                "Breakout",
            ),
            replay_fill(
                3,
                "TEST_CODE_600001",
                "sell",
                13.0,
                100,
                "2026-08-21 09:31:05",
                "2026-08-21T09:31:05+08:00",
                "ExitByRule",
            ),
            replay_fill(
                4,
                "TEST_CODE_600001",
                "sell",
                13.0,
                100,
                "2026-08-21 09:32:05",
                "2026-08-21T09:32:05+08:00",
                "ExitByRule",
            ),
            replay_fill(
                5,
                "TEST_CODE_600002",
                "buy",
                20.0,
                100,
                "2026-08-21 09:33:05",
                "2026-08-21T09:33:05+08:00",
                "Momentum",
            ),
        ];
        let full_bars = vec![
            minute_bar("2026-08-20T09:31:00+08:00", 100.0),
            minute_bar("2026-08-21T09:32:00+08:00", 110.0),
        ];

        let no_fee = replay_evidence("2026-08-20", "2026-08-21", fills.clone(), None);
        let no_fee_report =
            compute_attribution_range(&no_fee, &full_bars, &verified_minute_labels()).unwrap();
        assert_eq!(no_fee_report.total_closed_cycles, 1);
        assert_eq!(no_fee_report.total_open_cycles, 1);
        assert!(matches!(
            no_fee_report.closed_cycles[0].entry_family,
            EntryFamilyBucket::Mixed(ref families)
                if families == &[SignalFamily::Breakout, SignalFamily::Momentum]
        ));
        assert_eq!(no_fee_report.gross.available_cycles, 1);
        assert_eq!(no_fee_report.benchmark.available_cycles, 1);
        assert_eq!(no_fee_report.net.unavailable_cycles, 1);
        assert_eq!(
            no_fee_report.net.unavailable_reasons["fee_evidence_unavailable"],
            1
        );
        assert!(matches!(
            no_fee_report.closed_cycles[0].net_return,
            MetricAvailability::Unavailable {
                code: AttributionUnavailable::FeeEvidenceUnavailable,
                ..
            }
        ));
        assert!(matches!(
            no_fee_report.net_outcome,
            MetricAvailability::Unavailable {
                code: AttributionUnavailable::FeeEvidenceUnavailable,
                ..
            }
        ));
        assert!(matches!(
            no_fee_report.fee_basis,
            MetricAvailability::Unavailable {
                code: AttributionUnavailable::FeeEvidenceUnavailable,
                ..
            }
        ));
        assert!(no_fee_report
            .family_attribution
            .iter()
            .all(|family| matches!(
                family.net_outcome,
                MetricAvailability::Unavailable {
                    code: AttributionUnavailable::FeeEvidenceUnavailable,
                    ..
                }
            )));

        let fees = fills.iter().map(|fill| fee(fill.fill.id, 1.0)).collect();
        let complete_fee = replay_evidence("2026-08-20", "2026-08-21", fills, Some(fees));
        let missing_exit =
            compute_attribution_range(&complete_fee, &full_bars[..1], &verified_minute_labels())
                .unwrap();
        assert_eq!(missing_exit.gross.available_cycles, 1);
        assert_eq!(missing_exit.net.available_cycles, 1);
        assert_eq!(missing_exit.benchmark.unavailable_cycles, 1);
        assert_eq!(missing_exit.gross_excess.unavailable_cycles, 1);
        assert_eq!(missing_exit.net_excess.unavailable_cycles, 1);
        assert_eq!(
            missing_exit.benchmark.unavailable_reasons["benchmark_alignment_unavailable"],
            1
        );

        let unverified =
            compute_attribution_range(&complete_fee, &full_bars, &MinuteLabelSemantics::Unverified)
                .unwrap();
        assert_eq!(
            unverified.benchmark.unavailable_reasons["benchmark_time_semantics_unavailable"],
            1
        );
    }

    #[test]
    fn attribution_preserves_complete_outcomes_aggregates_fee_basis_and_mixed_composition() {
        let fills = vec![
            replay_fill(
                1,
                "TEST_CODE_A",
                "buy",
                10.0,
                100,
                "2026-08-20 09:31:05",
                "2026-08-20T09:31:05+08:00",
                "Momentum",
            ),
            replay_fill(
                2,
                "TEST_CODE_A",
                "buy",
                12.0,
                100,
                "2026-08-20 09:32:05",
                "2026-08-20T09:32:05+08:00",
                "Breakout",
            ),
            replay_fill(
                3,
                "TEST_CODE_B",
                "buy",
                10.0,
                100,
                "2026-08-20 09:33:05",
                "2026-08-20T09:33:05+08:00",
                "Breakout",
            ),
            replay_fill(
                4,
                "TEST_CODE_C",
                "buy",
                10.0,
                100,
                "2026-08-20 09:34:05",
                "2026-08-20T09:34:05+08:00",
                "Momentum",
            ),
            replay_fill(
                5,
                "TEST_CODE_A",
                "sell",
                13.0,
                100,
                "2026-08-21 09:31:05",
                "2026-08-21T09:31:05+08:00",
                "ExitByRule",
            ),
            replay_fill(
                6,
                "TEST_CODE_A",
                "sell",
                13.0,
                100,
                "2026-08-21 09:32:05",
                "2026-08-21T09:32:05+08:00",
                "ExitByRule",
            ),
            replay_fill(
                7,
                "TEST_CODE_B",
                "sell",
                9.0,
                100,
                "2026-08-21 09:33:05",
                "2026-08-21T09:33:05+08:00",
                "ExitByRule",
            ),
            replay_fill(
                8,
                "TEST_CODE_C",
                "sell",
                10.0,
                100,
                "2026-08-21 09:34:05",
                "2026-08-21T09:34:05+08:00",
                "ExitByRule",
            ),
            replay_fill(
                9,
                "TEST_CODE_D",
                "buy",
                20.0,
                100,
                "2026-08-21 09:35:05",
                "2026-08-21T09:35:05+08:00",
                "Momentum",
            ),
        ];
        let fee_entries = (1..=9)
            .map(|fill_id| fee(fill_id, if matches!(fill_id, 4 | 8) { 0.0 } else { 1.0 }))
            .collect::<Vec<_>>();
        let expected_hashes = fee_entries
            .iter()
            .map(|entry| (entry.fill_id, entry.evidence_hash.clone()))
            .collect::<Vec<_>>();
        let evidence = replay_evidence("2026-08-20", "2026-08-21", fills, Some(fee_entries));
        let bars = (31..=35)
            .flat_map(|minute| {
                [
                    minute_bar(&format!("2026-08-20T09:{minute:02}:00+08:00"), 100.0),
                    minute_bar(&format!("2026-08-21T09:{minute:02}:00+08:00"), 100.0),
                ]
            })
            .collect::<Vec<_>>();

        let report = compute_attribution_range(&evidence, &bars, &verified_minute_labels())
            .expect("TEST_CODE complete attribution report");

        assert_eq!(report.total_closed_cycles, 3);
        assert_eq!(report.total_open_cycles, 1);
        assert_eq!(report.gross.coverage.total_cycles, 3);
        assert_eq!(report.gross.coverage.available_cycles, 3);
        assert!((report.gross.sum_return.unwrap() - (2.0 / 11.0 - 0.1)).abs() < 1e-12);
        assert!((report.gross.mean_return.unwrap() - (2.0 / 11.0 - 0.1) / 3.0).abs() < 1e-12);
        assert_eq!(report.gross.median_return, Some(0.0));
        assert!(matches!(
            report.gross_outcome,
            MetricAvailability::Available(OutcomeSummary {
                wins: 1,
                losses: 1,
                breakeven: 1,
                directional_denominator: 2,
                win_rate: Some(0.5),
            })
        ));
        assert!(matches!(
            report.net_outcome,
            MetricAvailability::Available(OutcomeSummary {
                wins: 1,
                losses: 1,
                breakeven: 1,
                directional_denominator: 2,
                win_rate: Some(0.5),
            })
        ));
        assert!(matches!(
            report.fee_basis,
            MetricAvailability::Available(ref basis)
                if basis.kind == CostBasisKind::Observed
                    && basis.basis_id.len() == 64
                    && basis.bindings.iter().map(|binding| (binding.fill_id, binding.evidence_hash.clone())).collect::<Vec<_>>() == expected_hashes
        ));

        let mixed = report
            .closed_cycles
            .iter()
            .find(|cycle| matches!(cycle.entry_family, EntryFamilyBucket::Mixed(_)))
            .expect("TEST_CODE mixed cycle");
        assert_eq!(mixed.entry_composition.len(), 2);
        assert_eq!(mixed.entry_composition[0].family, SignalFamily::Breakout);
        assert_eq!(mixed.entry_composition[0].quantity, 100);
        assert_eq!(mixed.entry_composition[0].buy_notional, 1200.0);
        assert_eq!(mixed.entry_composition[1].family, SignalFamily::Momentum);
        assert_eq!(mixed.entry_composition[1].quantity, 100);
        assert_eq!(mixed.entry_composition[1].buy_notional, 1000.0);

        let momentum = report
            .family_attribution
            .iter()
            .find(|family| family.bucket == EntryFamilyBucket::Single(SignalFamily::Momentum))
            .expect("TEST_CODE momentum family");
        assert_eq!(momentum.total_closed_cycles, 1);
        assert_eq!(momentum.total_open_cycles, 1);
        assert!(matches!(
            momentum.gross_outcome,
            MetricAvailability::Available(OutcomeSummary {
                wins: 0,
                losses: 0,
                breakeven: 1,
                directional_denominator: 0,
                win_rate: None,
            })
        ));
    }

    #[test]
    fn pure_attribution_reuses_the_economic_state_machine_and_fails_the_whole_batch() {
        let valid = vec![
            replay_fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-08-20 10:00:00",
                "2026-08-20T10:00:00+08:00",
                "Momentum",
            ),
            replay_fill(
                2,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-08-21 10:00:00",
                "2026-08-21T10:00:00+08:00",
                "ExitByRule",
            ),
        ];
        let mut cases = Vec::new();
        let mut unknown_520 = valid.clone();
        unknown_520[0].fill.id = 520;
        unknown_520[0].terminal_audit_id = 10_520;
        unknown_520[0].terminal_audit_hash = format!("{:064x}", 10_520);
        unknown_520[0].fill.virtual_reason = "TEST_CODE_UNKNOWN_FAMILY".to_owned();
        cases.push(unknown_520);
        let mut t1 = valid.clone();
        t1[1].fill.occurred_at = "2026-08-20 14:00:00".to_owned();
        t1[1].terminal_time = instant("2026-08-20T14:00:00+08:00");
        cases.push(t1);
        let mut oversell = valid.clone();
        oversell[1].fill.quantity = 200;
        cases.push(oversell);
        let mut duplicate = valid.clone();
        duplicate[1].fill.id = 1;
        cases.push(duplicate);
        let mut bad_price = valid.clone();
        bad_price[0].fill.fill_price = Some(f64::INFINITY);
        cases.push(bad_price);
        let mut bad_quantity = valid.clone();
        bad_quantity[0].fill.quantity = 150;
        cases.push(bad_quantity);
        let mut overflow = valid.clone();
        overflow[0].fill.fill_price = Some(f64::MAX);
        cases.push(overflow);
        let mut unordered = valid;
        unordered.reverse();
        cases.push(unordered);

        for fills in cases {
            let evidence = replay_evidence("2026-08-20", "2026-08-21", fills, None);
            assert!(matches!(
                compute_attribution_range(&evidence, &[], &MinuteLabelSemantics::Unverified),
                Err(AttributionReplayError::FailedIntegrity { .. })
            ));
        }
    }

    fn threshold_replay(cycles: usize, exit_day: &str) -> AttributionReplayEvidence {
        let mut fills = Vec::with_capacity(cycles * 2);
        for index in 0..cycles {
            fills.push(replay_fill(
                i64::try_from(index + 1).unwrap(),
                &format!("TEST_CODE_{index:06}"),
                "buy",
                10.0,
                100,
                "2026-01-01 09:31:05",
                "2026-01-01T09:31:05+08:00",
                "Momentum",
            ));
        }
        for index in 0..cycles {
            fills.push(replay_fill(
                i64::try_from(cycles + index + 1).unwrap(),
                &format!("TEST_CODE_{index:06}"),
                "sell",
                11.0,
                100,
                &format!("{exit_day} 09:31:05"),
                &format!("{exit_day}T09:31:05+08:00"),
                "ExitByRule",
            ));
        }
        replay_evidence("2026-01-01", exit_day, fills, None)
    }

    #[test]
    fn pure_attribution_keeps_the_199_200_and_83_84_day_research_boundaries() {
        let report_199 = compute_attribution_range(
            &threshold_replay(199, "2026-03-25"),
            &[],
            &MinuteLabelSemantics::Unverified,
        )
        .unwrap();
        assert_eq!(report_199.coverage_days, Some(84));
        assert!(matches!(
            report_199.conclusion,
            AttributionConclusion::InsufficientSample { ref reasons, .. }
                if reasons.iter().any(|reason| reason.contains("closed_cycles_199"))
        ));

        let report_83 = compute_attribution_range(
            &threshold_replay(200, "2026-03-24"),
            &[],
            &MinuteLabelSemantics::Unverified,
        )
        .unwrap();
        assert_eq!(report_83.coverage_days, Some(83));
        assert!(matches!(
            report_83.conclusion,
            AttributionConclusion::InsufficientSample { ref reasons, .. }
                if reasons.iter().any(|reason| reason.contains("coverage_days_83"))
        ));

        let report_84 = compute_attribution_range(
            &threshold_replay(200, "2026-03-25"),
            &[],
            &MinuteLabelSemantics::Unverified,
        )
        .unwrap();
        assert_eq!(report_84.coverage_days, Some(84));
        assert!(matches!(
            report_84.conclusion,
            AttributionConclusion::ResearchOnly {
                research_limitations: ref limitations
            } if limitations == &research_limitations()
        ));
    }

    #[test]
    fn daily_and_same_day_range_have_identical_canonical_payload_and_hash() {
        let fills = vec![
            replay_fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-08-20 09:31:05",
                "2026-08-20T09:31:05+08:00",
                "Momentum",
            ),
            replay_fill(
                2,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-08-21 09:32:05",
                "2026-08-21T09:32:05+08:00",
                "ExitByRule",
            ),
        ];
        let mut fees = vec![fee(1, 1.0), fee(2, 1.0)];
        fees.reverse();
        let evidence = replay_evidence("2026-08-21", "2026-08-21", fills, Some(fees));
        let mut bars = vec![
            minute_bar("2026-08-20T09:31:00+08:00", 100.0),
            minute_bar("2026-08-21T09:32:00+08:00", 110.0),
        ];
        let daily = compute_attribution_daily(
            date("2026-08-21"),
            &evidence,
            &bars,
            &verified_minute_labels(),
        )
        .unwrap();
        bars.reverse();
        let range = compute_attribution_range(&evidence, &bars, &verified_minute_labels()).unwrap();

        assert_eq!(
            canonical_attribution_report_bytes(&daily).unwrap(),
            canonical_attribution_report_bytes(&range).unwrap()
        );
        assert_eq!(
            canonical_attribution_report_hash(&daily).unwrap(),
            canonical_attribution_report_hash(&range).unwrap()
        );
    }

    #[test]
    fn canonical_report_rejects_recursive_non_finite_values_and_normalizes_signed_zero() {
        let fills = vec![
            replay_fill(
                1,
                "TEST_CODE_600001",
                "buy",
                10.0,
                100,
                "2026-08-20 09:31:05",
                "2026-08-20T09:31:05+08:00",
                "Momentum",
            ),
            replay_fill(
                2,
                "TEST_CODE_600001",
                "sell",
                11.0,
                100,
                "2026-08-21 09:32:05",
                "2026-08-21T09:32:05+08:00",
                "ExitByRule",
            ),
        ];
        let evidence = replay_evidence(
            "2026-08-20",
            "2026-08-21",
            fills,
            Some(vec![fee(1, 1.0), fee(2, 1.0)]),
        );
        let bars = vec![
            minute_bar("2026-08-20T09:31:00+08:00", 100.0),
            minute_bar("2026-08-21T09:32:00+08:00", 110.0),
        ];
        let report =
            compute_attribution_range(&evidence, &bars, &verified_minute_labels()).unwrap();

        let mut cycle_nan = report.clone();
        cycle_nan.closed_cycles[0].gross_return = f64::NAN;
        assert!(matches!(
            canonical_attribution_report_bytes(&cycle_nan),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));
        let mut nested_infinity = report.clone();
        nested_infinity.closed_cycles[0].benchmark_return =
            MetricAvailability::Available(f64::INFINITY);
        assert!(matches!(
            canonical_attribution_report_hash(&nested_infinity),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));
        let mut coverage_infinity = report.clone();
        coverage_infinity.family_attribution[0]
            .net
            .coverage
            .coverage_ratio = Some(f64::NEG_INFINITY);
        assert!(matches!(
            canonical_attribution_report_bytes(&coverage_infinity),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));
        let mut aggregate_infinity = report.clone();
        aggregate_infinity.family_attribution[0].net.sum_return = Some(f64::INFINITY);
        assert!(matches!(
            canonical_attribution_report_hash(&aggregate_infinity),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));
        let mut outcome_mismatch = report.clone();
        let MetricAvailability::Available(outcome) = &mut outcome_mismatch.gross_outcome else {
            panic!("TEST_CODE gross outcome must be available");
        };
        outcome.breakeven += 1;
        assert!(matches!(
            canonical_attribution_report_bytes(&outcome_mismatch),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));
        let mut basis_mismatch = report.clone();
        let MetricAvailability::Available(basis) = &mut basis_mismatch.fee_basis else {
            panic!("TEST_CODE fee basis must be available");
        };
        basis.bindings[0].evidence_hash = "f".repeat(64);
        assert!(matches!(
            canonical_attribution_report_hash(&basis_mismatch),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));
        let mut conclusion_mismatch = report.clone();
        conclusion_mismatch.coverage_days = Some(999);
        assert!(matches!(
            canonical_attribution_report_bytes(&conclusion_mismatch),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));

        let zero_evidence = replay_evidence(
            "2026-08-20",
            "2026-08-21",
            vec![
                replay_fill(
                    1,
                    "TEST_CODE_ZERO",
                    "buy",
                    10.0,
                    100,
                    "2026-08-20 09:31:05",
                    "2026-08-20T09:31:05+08:00",
                    "Momentum",
                ),
                replay_fill(
                    2,
                    "TEST_CODE_ZERO",
                    "sell",
                    10.0,
                    100,
                    "2026-08-21 09:31:05",
                    "2026-08-21T09:31:05+08:00",
                    "ExitByRule",
                ),
            ],
            Some(vec![fee(1, 0.0), fee(2, 0.0)]),
        );
        let zero_bars = vec![
            minute_bar("2026-08-20T09:31:00+08:00", 100.0),
            minute_bar("2026-08-21T09:31:00+08:00", 100.0),
        ];
        let positive_zero =
            compute_attribution_range(&zero_evidence, &zero_bars, &verified_minute_labels())
                .unwrap();
        let mut negative_zero = positive_zero.clone();
        negative_zero.closed_cycles[0].gross_return = -0.0;
        assert_eq!(
            canonical_attribution_report_bytes(&positive_zero).unwrap(),
            canonical_attribution_report_bytes(&negative_zero).unwrap()
        );
        assert_eq!(
            canonical_attribution_report_hash(&positive_zero).unwrap(),
            canonical_attribution_report_hash(&negative_zero).unwrap()
        );
    }

    #[test]
    fn canonical_report_rejects_source_fill_rebinding_without_fee_basis() {
        let evidence = replay_evidence(
            "2026-08-20",
            "2026-08-20",
            vec![replay_fill(
                1,
                "TEST_CODE_OPEN",
                "buy",
                10.0,
                100,
                "2026-08-20 09:31:05",
                "2026-08-20T09:31:05+08:00",
                "Momentum",
            )],
            None,
        );
        let mut rebound =
            compute_attribution_range(&evidence, &[], &verified_minute_labels()).unwrap();
        rebound.canonical_source_fill_ids[0] = 2;
        rebound.read_only_projection.source_fill_ids[0] = 2;

        assert!(matches!(
            canonical_attribution_report_bytes(&rebound),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));
    }

    #[test]
    fn canonical_report_seal_rejects_open_family_rebinding_and_cross_report_swap() {
        let momentum_evidence = replay_evidence(
            "2026-08-20",
            "2026-08-20",
            vec![replay_fill(
                1,
                "TEST_CODE_OPEN",
                "buy",
                10.0,
                100,
                "2026-08-20 09:31:05",
                "2026-08-20T09:31:05+08:00",
                "Momentum",
            )],
            None,
        );
        let momentum =
            compute_attribution_range(&momentum_evidence, &[], &verified_minute_labels()).unwrap();

        let mut rebound_family = momentum.clone();
        rebound_family.family_attribution[0].bucket =
            EntryFamilyBucket::Single(SignalFamily::Breakout);
        assert!(matches!(
            canonical_attribution_report_bytes(&rebound_family),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));

        let breakout_evidence = replay_evidence(
            "2026-08-20",
            "2026-08-20",
            vec![replay_fill(
                2,
                "TEST_CODE_OPEN_2",
                "buy",
                20.0,
                100,
                "2026-08-20 09:32:05",
                "2026-08-20T09:32:05+08:00",
                "Breakout",
            )],
            None,
        );
        let breakout =
            compute_attribution_range(&breakout_evidence, &[], &verified_minute_labels()).unwrap();
        let mut cut_and_pasted = momentum.clone();
        cut_and_pasted.family_attribution = breakout.family_attribution.clone();
        assert!(matches!(
            canonical_attribution_report_bytes(&cut_and_pasted),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));

        let mut swapped_seal = momentum.clone();
        swapped_seal.report_seal = breakout.report_seal.clone();
        assert!(matches!(
            canonical_attribution_report_hash(&swapped_seal),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));
    }

    #[test]
    fn canonical_report_seal_rejects_empty_report_source_injection() {
        let evidence = replay_evidence("2026-08-20", "2026-08-20", Vec::new(), None);
        let mut injected =
            compute_attribution_range(&evidence, &[], &verified_minute_labels()).unwrap();
        injected.canonical_source_fill_ids.push(1);
        injected.read_only_projection.source_fill_ids.push(1);

        assert!(matches!(
            canonical_attribution_report_bytes(&injected),
            Err(AttributionReplayError::FailedIntegrity {
                code: AttributionIntegrityFailure::CanonicalReport,
                ..
            })
        ));
    }

    #[test]
    fn canonical_report_cardinality_overflow_is_typed_and_never_panics() {
        let evidence = replay_evidence(
            "2026-08-20",
            "2026-08-20",
            vec![replay_fill(
                1,
                "TEST_CODE_OPEN",
                "buy",
                10.0,
                100,
                "2026-08-20 09:31:05",
                "2026-08-20T09:31:05+08:00",
                "Momentum",
            )],
            None,
        );
        let report = compute_attribution_range(&evidence, &[], &verified_minute_labels()).unwrap();
        let assert_typed = |candidate: AttributionComputationReport| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                canonical_attribution_report_bytes(&candidate)
            }))
            .expect("canonical cardinality validation must not panic");
            assert!(matches!(
                result,
                Err(AttributionReplayError::FailedIntegrity {
                    code: AttributionIntegrityFailure::CanonicalReport,
                    ..
                })
            ));
        };

        let mut coverage = report.clone();
        coverage.gross.coverage.available_cycles = usize::MAX;
        coverage.gross.coverage.unavailable_cycles = 1;
        assert_typed(coverage);

        let mut outcome = report.clone();
        let MetricAvailability::Available(summary) = &mut outcome.gross_outcome else {
            panic!("TEST_CODE gross outcome must be available");
        };
        summary.wins = usize::MAX;
        summary.losses = 1;
        assert_typed(outcome);

        let mut family = report.clone();
        let mut second = family.family_attribution[0].clone();
        family.family_attribution[0].total_open_cycles = usize::MAX;
        second.bucket = EntryFamilyBucket::Single(SignalFamily::Breakout);
        second.total_open_cycles = 1;
        family.family_attribution.push(second);
        family
            .family_attribution
            .sort_by(|left, right| left.bucket.cmp(&right.bucket));
        assert_typed(family);

        let mut reason = report;
        reason
            .net
            .coverage
            .unavailable_reasons
            .insert("TEST_CODE_MAX".to_owned(), usize::MAX);
        reason
            .net
            .coverage
            .unavailable_reasons
            .insert("TEST_CODE_ONE".to_owned(), 1);
        assert_typed(reason);
    }
}
