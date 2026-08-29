//! BR-251 历史归因只读证据装载。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::Metadata;
use std::ops::Deref;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Timelike};
use diesel::prelude::RunQueryDsl;
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use diesel::sqlite::{Sqlite, SqliteConnection};
use rusqlite::{
    params_from_iter, types::Value, Connection, ErrorCode, OpenFlags, Transaction,
    TransactionBehavior,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::attribution::SignalFamily;
use super::attribution_epoch::{
    canonical_exclusion_manifest_hash, canonical_legacy_carry_manifest_hash,
    canonical_scoped_fill_manifest_hash, scope_epoch_fills, AttributionEpochSelector,
    EpochExclusion, EpochExclusionReason, LegacyCarryPosition,
};
use super::economic_position::{
    rebuild_economic_positions, rebuild_economic_positions_with_replay_fees,
    select_economic_rows_through, CostBasisKind, EconomicFillRow, EntryFamilyComposition,
    FillCostEvidence as EconomicFillCostEvidence, FillCostLedger, NetMetrics,
};
use crate::calendar::{
    resolve_verified_replay_quarter, resolve_verified_replay_range,
    resolve_verified_scheduled_replay, verified_a_share_trading_day,
    verified_replay_quarter_bounds, VerifiedCalendarError, VerifiedCalendarErrorKind,
    VerifiedReplayCalendar,
};
use crate::data_gateway::{
    BenchmarkBar, BenchmarkBarTime, BenchmarkError, BenchmarkRange, BenchmarkReader,
    BenchmarkRequest, BenchmarkUnsupported, HS300_CANONICAL,
};
use crate::database::attribution_epochs::{
    load_selector_with_connection, load_verified_epoch_fills_until, AttributionEpochReceipt,
    AttributionEpochStoreError, ResolvedAttributionEpoch, VerifiedEpochFillSet,
};
use crate::database::attribution_reports::{
    AttributionEvidenceHash, AttributionFailureAppend, AttributionFailureReceipt,
    AttributionInvocation, AttributionReportAppend, AttributionReportEpochBinding,
    AttributionReportReceipt, AttributionReportStore, AttributionReportStoreError,
    AttributionRunMode,
};
use crate::database::order_audit::{
    validate_canonical_order_audit_chain, CanonicalOrderAuditChainRow, CanonicalOrderAuditRow,
};
use crate::database::{AttributionReadTransactionError, DatabaseAuthorityError, DatabaseManager};
use crate::trading::paper_lot_ledger::parse_paper_fill_timestamp;

const STOCK_CLOSE_HASH_DOMAIN: &[u8] = b"BR251_STOCK_CLOSE_MANIFEST_V1\0";
const FEE_EVIDENCE_HASH_DOMAIN: &[u8] = b"BR251_FILL_FEE_EVIDENCE_V1\0";
const STOCK_CLOSE_KEYS_PER_QUERY: usize = 400;
const ATTRIBUTION_REPORT_HASH_DOMAIN: &[u8] = b"BR251_ATTRIBUTION_REPORT_V1\0";
const ATTRIBUTION_REPORT_SEAL_DOMAIN: &[u8] = b"BR251_ATTRIBUTION_REPORT_SEAL_V1\0";
const REPLAY_FEE_BASIS_HASH_DOMAIN: &[u8] = b"BR251_REPLAY_FEE_BASIS_V1\0";
const REPLAY_CAPABILITY_SEAL_DOMAIN: &[u8] = b"BR251_REPLAY_CAPABILITY_SEAL_V1\0";
const REPLAY_TRADE_MANIFEST_HASH_DOMAIN: &[u8] = b"BR251_REPLAY_TRADE_MANIFEST_V1\0";
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
    epoch: AttributionEpochReplayEvidence,
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

    pub fn epoch(&self) -> &AttributionEpochReplayEvidence {
        &self.epoch
    }

    pub fn epoch_selector(&self) -> &AttributionEpochSelector {
        self.epoch.selector()
    }

    pub fn epoch_id(&self) -> Option<&str> {
        self.epoch.epoch_id()
    }

    pub fn epoch_receipt_hash(&self) -> Option<&str> {
        self.epoch.receipt_hash()
    }

    pub fn epoch_effective_date(&self) -> Option<NaiveDate> {
        self.epoch.effective_date()
    }

    pub fn legacy_carry_manifest_hash(&self) -> Option<&str> {
        self.epoch.legacy_carry_manifest_hash()
    }

    pub fn exclusion_manifest_hash(&self) -> Option<&str> {
        self.epoch.exclusion_manifest_hash()
    }

    pub fn remaining_quarantine(&self) -> &[LegacyCarryPosition] {
        self.epoch.remaining_quarantine()
    }

    pub fn released_codes(&self) -> usize {
        self.epoch.released_codes()
    }

    pub fn excluded_fills(&self) -> &[EpochExclusion] {
        self.epoch.excluded_fills()
    }

    pub fn overlap_buy_count(&self) -> usize {
        self.epoch.overlap_buy_count()
    }

    pub fn overlap_sell_count(&self) -> usize {
        self.epoch.overlap_sell_count()
    }

    pub fn mixed_exit_count(&self) -> usize {
        self.epoch.mixed_exit_count()
    }

    pub fn excluded_fill_count(&self) -> usize {
        self.epoch.excluded_fill_count()
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttributionEpochReplayEvidence {
    selector: AttributionEpochSelector,
    epoch_id: Option<String>,
    receipt_hash: Option<String>,
    effective_date: Option<NaiveDate>,
    legacy_carry_manifest_hash: Option<String>,
    exclusion_manifest_hash: Option<String>,
    remaining_quarantine: Vec<LegacyCarryPosition>,
    released_codes: usize,
    excluded_fills: Vec<EpochExclusion>,
    overlap_buy_count: usize,
    overlap_sell_count: usize,
    mixed_exit_count: usize,
    excluded_fill_count: usize,
    #[serde(skip)]
    scoped_fill_manifest_hash: String,
    #[serde(skip)]
    verified_filled_manifest_hash: Option<String>,
    #[serde(skip)]
    verified_terminal_binding_manifest_hash: Option<String>,
    #[serde(skip)]
    verified_order_audit_tip_hash: Option<String>,
}

impl AttributionEpochReplayEvidence {
    fn legacy(scoped_fill_manifest_hash: String) -> Self {
        Self {
            selector: AttributionEpochSelector::Legacy,
            epoch_id: None,
            receipt_hash: None,
            effective_date: None,
            legacy_carry_manifest_hash: None,
            exclusion_manifest_hash: None,
            remaining_quarantine: Vec::new(),
            released_codes: 0,
            excluded_fills: Vec::new(),
            overlap_buy_count: 0,
            overlap_sell_count: 0,
            mixed_exit_count: 0,
            excluded_fill_count: 0,
            scoped_fill_manifest_hash,
            verified_filled_manifest_hash: None,
            verified_terminal_binding_manifest_hash: None,
            verified_order_audit_tip_hash: None,
        }
    }

    fn resolved(
        selector: AttributionEpochSelector,
        receipt: &AttributionEpochReceipt,
        scope: ResolvedAttributionEpochReplayScope,
    ) -> Self {
        let overlap_buy_count = scope
            .exclusions
            .iter()
            .filter(|item| {
                item.reason == EpochExclusionReason::LegacyCarryOverlap && item.direction == "buy"
            })
            .count();
        let overlap_sell_count = scope
            .exclusions
            .iter()
            .filter(|item| {
                item.reason == EpochExclusionReason::LegacyCarryOverlap && item.direction == "sell"
            })
            .count();
        let mixed_exit_count = scope
            .exclusions
            .iter()
            .filter(|item| item.reason == EpochExclusionReason::MixedLegacyCarryExit)
            .count();
        let excluded_fill_count = scope
            .exclusions
            .iter()
            .map(|item| item.fill_id)
            .collect::<BTreeSet<_>>()
            .len();
        Self {
            selector,
            epoch_id: Some(receipt.epoch_id.clone()),
            receipt_hash: Some(receipt.receipt_hash.clone()),
            effective_date: Some(receipt.effective_trading_date),
            legacy_carry_manifest_hash: Some(receipt.legacy_carry_manifest_hash.clone()),
            exclusion_manifest_hash: Some(scope.exclusion_manifest_hash),
            remaining_quarantine: scope.remaining_quarantine,
            released_codes: scope.released_codes,
            excluded_fills: scope.exclusions,
            overlap_buy_count,
            overlap_sell_count,
            mixed_exit_count,
            excluded_fill_count,
            scoped_fill_manifest_hash: scope.scoped_fill_manifest_hash,
            verified_filled_manifest_hash: Some(scope.verified_filled_manifest_hash),
            verified_terminal_binding_manifest_hash: Some(
                scope.verified_terminal_binding_manifest_hash,
            ),
            verified_order_audit_tip_hash: Some(scope.verified_order_audit_tip_hash),
        }
    }

    pub fn selector(&self) -> &AttributionEpochSelector {
        &self.selector
    }

    pub fn epoch_id(&self) -> Option<&str> {
        self.epoch_id.as_deref()
    }

    pub fn receipt_hash(&self) -> Option<&str> {
        self.receipt_hash.as_deref()
    }

    pub fn effective_date(&self) -> Option<NaiveDate> {
        self.effective_date
    }

    pub fn legacy_carry_manifest_hash(&self) -> Option<&str> {
        self.legacy_carry_manifest_hash.as_deref()
    }

    pub fn exclusion_manifest_hash(&self) -> Option<&str> {
        self.exclusion_manifest_hash.as_deref()
    }

    pub fn remaining_quarantine(&self) -> &[LegacyCarryPosition] {
        &self.remaining_quarantine
    }

    pub fn released_codes(&self) -> usize {
        self.released_codes
    }

    pub fn excluded_fills(&self) -> &[EpochExclusion] {
        &self.excluded_fills
    }

    pub fn overlap_buy_count(&self) -> usize {
        self.overlap_buy_count
    }

    pub fn overlap_sell_count(&self) -> usize {
        self.overlap_sell_count
    }

    pub fn mixed_exit_count(&self) -> usize {
        self.mixed_exit_count
    }

    pub fn excluded_fill_count(&self) -> usize {
        self.excluded_fill_count
    }
}

struct ResolvedAttributionEpochReplayScope {
    exclusions: Vec<EpochExclusion>,
    exclusion_manifest_hash: String,
    remaining_quarantine: Vec<LegacyCarryPosition>,
    released_codes: usize,
    scoped_fill_manifest_hash: String,
    verified_filled_manifest_hash: String,
    verified_terminal_binding_manifest_hash: String,
    verified_order_audit_tip_hash: String,
}

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
    epoch: AttributionEpochReplayEvidence,
    trade_manifest_hash: String,
    capability_seal: AttributionReplayCapabilitySeal,
}

impl AttributionReplayEvidence {
    fn issued(
        from: NaiveDate,
        to: NaiveDate,
        fills: Vec<ReplayFillEvidence>,
        stock_closes: StockCloseManifest,
        fees: FeeEvidenceAvailability,
        epoch: AttributionEpochReplayEvidence,
    ) -> Self {
        let trade_manifest_hash = replay_trade_manifest_hash(&epoch, &fills);
        let capability_seal = replay_capability_seal(
            from,
            to,
            &fills,
            &stock_closes,
            &fees,
            &epoch,
            &trade_manifest_hash,
        );
        Self {
            from,
            to,
            fills,
            stock_closes,
            fees,
            epoch,
            trade_manifest_hash,
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

    pub fn epoch(&self) -> &AttributionEpochReplayEvidence {
        &self.epoch
    }

    pub fn trade_manifest_hash(&self) -> &str {
        &self.trade_manifest_hash
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

fn update_canonical_replay_fill(hasher: &mut Sha256, evidence: &ReplayFillEvidence) {
    let fill = &evidence.fill;
    hasher.update(fill.id.to_be_bytes());
    update_len_prefixed(hasher, fill.plan_id.as_bytes());
    update_len_prefixed(hasher, fill.code.as_bytes());
    update_len_prefixed(hasher, fill.name.as_bytes());
    update_len_prefixed(hasher, fill.direction.as_bytes());
    match fill.fill_price {
        Some(price) => {
            hasher.update([1]);
            hasher.update(price.to_bits().to_be_bytes());
        }
        None => hasher.update([0]),
    }
    hasher.update(fill.quantity.to_be_bytes());
    update_len_prefixed(hasher, fill.occurred_at.as_bytes());
    update_len_prefixed(hasher, fill.virtual_reason.as_bytes());
    hasher.update(evidence.terminal_audit_id.to_be_bytes());
    update_len_prefixed(hasher, evidence.terminal_audit_hash.as_bytes());
    update_len_prefixed(hasher, evidence.terminal_time.to_rfc3339().as_bytes());
}

fn update_epoch_replay_evidence(hasher: &mut Sha256, epoch: &AttributionEpochReplayEvidence) {
    update_len_prefixed(hasher, epoch.selector.canonical_value().as_bytes());
    update_optional_text(hasher, epoch.epoch_id.as_deref());
    update_optional_text(hasher, epoch.receipt_hash.as_deref());
    update_optional_text(
        hasher,
        epoch.effective_date.map(|date| date.to_string()).as_deref(),
    );
    update_optional_text(hasher, epoch.legacy_carry_manifest_hash.as_deref());
    update_optional_text(hasher, epoch.exclusion_manifest_hash.as_deref());
    update_len_prefixed(hasher, epoch.scoped_fill_manifest_hash.as_bytes());
    update_optional_text(hasher, epoch.verified_filled_manifest_hash.as_deref());
    update_optional_text(
        hasher,
        epoch.verified_terminal_binding_manifest_hash.as_deref(),
    );
    update_optional_text(hasher, epoch.verified_order_audit_tip_hash.as_deref());
    hasher.update((epoch.remaining_quarantine.len() as u64).to_be_bytes());
    for position in &epoch.remaining_quarantine {
        update_len_prefixed(hasher, position.code.as_bytes());
        hasher.update(position.quantity.to_be_bytes());
    }
    hasher.update((epoch.released_codes as u64).to_be_bytes());
    hasher.update((epoch.excluded_fills.len() as u64).to_be_bytes());
    for exclusion in &epoch.excluded_fills {
        hasher.update(exclusion.fill_id.to_be_bytes());
        update_len_prefixed(hasher, exclusion.code.as_bytes());
        update_len_prefixed(hasher, exclusion.direction.as_bytes());
        hasher.update(exclusion.quantity.to_be_bytes());
        hasher.update([match exclusion.reason {
            EpochExclusionReason::LegacyCarryOverlap => 1,
            EpochExclusionReason::MixedLegacyCarryExit => 2,
        }]);
    }
    for count in [
        epoch.overlap_buy_count,
        epoch.overlap_sell_count,
        epoch.mixed_exit_count,
        epoch.excluded_fill_count,
    ] {
        hasher.update((count as u64).to_be_bytes());
    }
}

fn replay_capability_seal(
    from: NaiveDate,
    to: NaiveDate,
    fills: &[ReplayFillEvidence],
    stock_closes: &StockCloseManifest,
    fees: &FeeEvidenceAvailability,
    epoch: &AttributionEpochReplayEvidence,
    trade_manifest_hash: &str,
) -> AttributionReplayCapabilitySeal {
    let mut hasher = Sha256::new();
    hasher.update(REPLAY_CAPABILITY_SEAL_DOMAIN);
    update_len_prefixed(&mut hasher, trade_manifest_hash.as_bytes());
    update_epoch_replay_evidence(&mut hasher, epoch);
    update_len_prefixed(&mut hasher, from.to_string().as_bytes());
    update_len_prefixed(&mut hasher, to.to_string().as_bytes());
    hasher.update((fills.len() as u64).to_be_bytes());
    for evidence in fills {
        update_canonical_replay_fill(&mut hasher, evidence);
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

fn replay_trade_manifest_hash(
    epoch: &AttributionEpochReplayEvidence,
    fills: &[ReplayFillEvidence],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(REPLAY_TRADE_MANIFEST_HASH_DOMAIN);
    update_len_prefixed(&mut hasher, epoch.selector.canonical_value().as_bytes());
    update_optional_text(&mut hasher, epoch.receipt_hash.as_deref());
    update_len_prefixed(&mut hasher, epoch.scoped_fill_manifest_hash.as_bytes());
    hasher.update((fills.len() as u64).to_be_bytes());
    for evidence in fills {
        update_canonical_replay_fill(&mut hasher, evidence);
    }
    hex::encode(hasher.finalize())
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
        &evidence.epoch,
        &evidence.trade_manifest_hash,
    );
    let economic_rows = evidence
        .fills
        .iter()
        .map(|fill| fill.fill.clone())
        .collect::<Vec<_>>();
    let scoped_fill_manifest_hash =
        canonical_scoped_fill_manifest_hash(&economic_rows).map_err(|detail| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::ReplayEvidence,
                format!("replay scoped fill manifest is invalid: {detail}"),
            )
        })?;
    if scoped_fill_manifest_hash != evidence.epoch.scoped_fill_manifest_hash
        || replay_trade_manifest_hash(&evidence.epoch, &evidence.fills)
            != evidence.trade_manifest_hash
        || expected != evidence.capability_seal
    {
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

#[derive(Debug, Clone, diesel::QueryableByName)]
struct RawStockCloseRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    date: String,
    #[diesel(sql_type = Nullable<Double>)]
    close: Option<f64>,
    #[diesel(sql_type = Nullable<Text>)]
    data_source: Option<String>,
    #[diesel(sql_type = Text)]
    created_at: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttributionReplayLoadStage {
    Trade,
    StockClose,
    Fee,
    Finalize,
}

#[derive(Debug, Clone, Default)]
struct AttributionReplayLoadProgress {
    trade_manifest_hash: Option<String>,
    stock_close_manifest_hash: Option<String>,
    fee: Option<FeeEvidenceAvailability>,
}

#[derive(Debug, Clone)]
struct AttributionReplayLoadFailure {
    error: AttributionReplayError,
    progress: AttributionReplayLoadProgress,
    stage: AttributionReplayLoadStage,
    failure_date: Option<NaiveDate>,
}

impl AttributionReplayLoadProgress {
    fn failure(
        &self,
        error: AttributionReplayError,
        stage: AttributionReplayLoadStage,
        failure_date: Option<NaiveDate>,
    ) -> AttributionReplayLoadFailure {
        AttributionReplayLoadFailure {
            error,
            progress: self.clone(),
            stage,
            failure_date,
        }
    }
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
        self.load_with_progress(request)
            .map_err(|failure| failure.error)
    }

    fn load_with_progress(
        &self,
        request: &AttributionReplayRequest,
    ) -> Result<AttributionReplayEvidence, AttributionReplayLoadFailure> {
        let mut progress = AttributionReplayLoadProgress::default();
        validate_request(request)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        let canonical_database = self.database.canonicalize().map_err(|error| {
            progress.failure(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::DatabaseIdentity,
                    format!("explicit database path cannot be resolved: {error}"),
                ),
                AttributionReplayLoadStage::Trade,
                None,
            )
        })?;
        let before_metadata = canonical_database.metadata().map_err(|error| {
            progress.failure(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::DatabaseIdentity,
                    format!("explicit database metadata unavailable: {error}"),
                ),
                AttributionReplayLoadStage::Trade,
                None,
            )
        })?;
        if !before_metadata.is_file() {
            return Err(progress.failure(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::DatabaseIdentity,
                    "explicit database path is not a regular file",
                ),
                AttributionReplayLoadStage::Trade,
                None,
            ));
        }
        let expected_identity = FileIdentity::of(&before_metadata);
        let mut connection = open_query_only_connection(&canonical_database)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        verify_main_database(&connection, &canonical_database, expected_identity)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| source_read_error("begin one read transaction", error))
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        let all_paper_rows = load_paper_rows(&transaction)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        let audit_rows = load_order_audits(&transaction)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        let chain_rows = load_order_audit_chain(&transaction)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        validate_canonical_order_audit_chain(&audit_rows, &chain_rows)
            .map_err(|detail| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::OrderAuditChain,
                    detail,
                )
            })
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;

        let all_economic_rows = all_paper_rows
            .iter()
            .map(|row| row.fill.clone())
            .collect::<Vec<_>>();
        validate_complete_paper_source(&all_economic_rows, request.to)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        let all_terminals = bind_all_terminals(&all_paper_rows, &audit_rows, &chain_rows)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        let projected_rows = select_economic_rows_through(all_economic_rows, request.to)
            .map_err(|detail| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::PaperTradeSource,
                    detail,
                )
            })
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
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
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        let scoped_fill_manifest_hash = canonical_scoped_fill_manifest_hash(
            &fills
                .iter()
                .map(|evidence| evidence.fill.clone())
                .collect::<Vec<_>>(),
        )
        .map_err(|detail| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::ReplayEvidence,
                format!("legacy replay scoped fill manifest is invalid: {detail}"),
            )
        })
        .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Trade, None))?;
        let epoch = AttributionEpochReplayEvidence::legacy(scoped_fill_manifest_hash);
        progress.trade_manifest_hash = Some(replay_trade_manifest_hash(&epoch, &fills));
        let required_close_keys =
            derive_required_close_keys(&fills, &request.required_trading_dates).map_err(
                |error| progress.failure(error, AttributionReplayLoadStage::Trade, None),
            )?;
        let raw_closes =
            load_stock_closes(&transaction, &required_close_keys).map_err(|error| {
                progress.failure(error, AttributionReplayLoadStage::StockClose, None)
            })?;
        let stock_closes =
            build_stock_close_manifest(raw_closes, &required_close_keys).map_err(|failure| {
                progress.failure(
                    failure.error,
                    AttributionReplayLoadStage::StockClose,
                    failure.failure_date,
                )
            })?;
        progress.stock_close_manifest_hash = Some(stock_closes.manifest_hash().to_owned());
        verify_transaction_main_database(&transaction, &canonical_database, expected_identity)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Finalize, None))?;
        let fees = validate_fee_ledger(request.fee_ledger.as_ref(), &fills)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Fee, None))?;
        progress.fee = Some(fees.clone());

        #[cfg(test)]
        run_after_read_test_hook();
        let during_identity = canonical_database
            .metadata()
            .map(|metadata| FileIdentity::of(&metadata))
            .map_err(|error| {
                progress.failure(
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::DatabaseIdentity,
                        format!("database identity re-check during read failed: {error}"),
                    ),
                    AttributionReplayLoadStage::Finalize,
                    None,
                )
            })?;
        if during_identity != expected_identity {
            return Err(progress.failure(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::DatabaseIdentity,
                    "database file identity changed during read",
                ),
                AttributionReplayLoadStage::Finalize,
                None,
            ));
        }
        transaction
            .commit()
            .map_err(|error| source_read_error("finish read transaction", error))
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Finalize, None))?;
        verify_main_database(&connection, &canonical_database, expected_identity)
            .map_err(|error| progress.failure(error, AttributionReplayLoadStage::Finalize, None))?;
        let after_identity = canonical_database
            .metadata()
            .map(|metadata| FileIdentity::of(&metadata))
            .map_err(|error| {
                progress.failure(
                    AttributionReplayError::integrity(
                        AttributionIntegrityFailure::DatabaseIdentity,
                        format!("database identity re-check after read failed: {error}"),
                    ),
                    AttributionReplayLoadStage::Finalize,
                    None,
                )
            })?;
        if after_identity != expected_identity {
            return Err(progress.failure(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::DatabaseIdentity,
                    "database file identity changed after read",
                ),
                AttributionReplayLoadStage::Finalize,
                None,
            ));
        }

        Ok(AttributionReplayEvidence::issued(
            request.from,
            request.to,
            fills,
            stock_closes,
            fees,
            epoch,
        ))
    }

    fn load_verified_epoch_tail_with_connection(
        connection: &mut SqliteConnection,
        request: &AttributionReplayRequest,
        fills: Vec<ReplayFillEvidence>,
        fees: FeeEvidenceAvailability,
        epoch: AttributionEpochReplayEvidence,
    ) -> Result<AttributionReplayEvidence, AttributionReplayLoadFailure> {
        let mut progress = AttributionReplayLoadProgress {
            trade_manifest_hash: Some(replay_trade_manifest_hash(&epoch, &fills)),
            stock_close_manifest_hash: None,
            fee: Some(fees.clone()),
        };
        validate_request(request).map_err(|error| {
            progress.failure(error, AttributionReplayLoadStage::StockClose, None)
        })?;
        let required_close_keys =
            derive_required_close_keys(&fills, &request.required_trading_dates).map_err(
                |error| progress.failure(error, AttributionReplayLoadStage::StockClose, None),
            )?;
        let raw_closes = load_stock_closes_with_connection(connection, &required_close_keys)
            .map_err(|error| {
                progress.failure(error, AttributionReplayLoadStage::StockClose, None)
            })?;
        let stock_closes =
            build_stock_close_manifest(raw_closes, &required_close_keys).map_err(|failure| {
                progress.failure(
                    failure.error,
                    AttributionReplayLoadStage::StockClose,
                    failure.failure_date,
                )
            })?;
        progress.stock_close_manifest_hash = Some(stock_closes.manifest_hash().to_owned());
        Ok(AttributionReplayEvidence::issued(
            request.from,
            request.to,
            fills,
            stock_closes,
            fees,
            epoch,
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

fn load_stock_closes_with_connection(
    connection: &mut SqliteConnection,
    required_keys: &BTreeSet<(String, NaiveDate)>,
) -> Result<Vec<RawStockCloseRow>, AttributionReplayError> {
    let keys = required_keys.iter().collect::<Vec<_>>();
    let mut result = Vec::new();
    for chunk in keys.chunks(STOCK_CLOSE_KEYS_PER_QUERY) {
        let mut query = diesel::sql_query(
            "SELECT id,code,date,close,data_source,
                    CAST(created_at AS TEXT) AS created_at,
                    CAST(updated_at AS TEXT) AS updated_at
             FROM stock_daily WHERE ",
        )
        .into_boxed::<Sqlite>();
        for (index, (code, date)) in chunk.iter().enumerate() {
            if index != 0 {
                query = query.sql(" OR ");
            }
            query = query
                .sql("(code = ")
                .sql("?")
                .bind::<Text, _>((*code).clone())
                .sql(" AND date = ")
                .sql("?")
                .bind::<Text, _>(date.format("%Y-%m-%d").to_string())
                .sql(")");
        }
        query = query.sql(" ORDER BY code ASC, date ASC, id ASC");
        let rows = query
            .load::<RawStockCloseRow>(connection)
            .map_err(|error| {
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::SourceRead,
                    format!("read exact stock close source: {error}"),
                )
            })?;
        result.extend(rows);
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

#[derive(Debug)]
struct StockCloseManifestFailure {
    error: AttributionReplayError,
    failure_date: Option<NaiveDate>,
}

impl StockCloseManifestFailure {
    fn new(error: AttributionReplayError, failure_date: Option<NaiveDate>) -> Self {
        Self {
            error,
            failure_date,
        }
    }
}

fn build_stock_close_manifest(
    rows: Vec<RawStockCloseRow>,
    required_keys: &BTreeSet<(String, NaiveDate)>,
) -> Result<StockCloseManifest, StockCloseManifestFailure> {
    let mut selected = BTreeMap::<(String, NaiveDate), StockCloseEvidence>::new();
    for row in rows {
        let parsed_date = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d").map_err(|error| {
            StockCloseManifestFailure::new(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::StockCloseSource,
                    format!(
                        "stock_daily id={} date is not exact YYYY-MM-DD: {error}",
                        row.id
                    ),
                ),
                None,
            )
        })?;
        if parsed_date.format("%Y-%m-%d").to_string() != row.date {
            return Err(StockCloseManifestFailure::new(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::StockCloseSource,
                    format!("stock_daily id={} date is not canonical YYYY-MM-DD", row.id),
                ),
                Some(parsed_date),
            ));
        }
        let key = (row.code.clone(), parsed_date);
        if !required_keys.contains(&key) {
            return Err(StockCloseManifestFailure::new(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::StockCloseSource,
                    format!(
                        "stock close query returned unexpected key {} {}",
                        row.code, parsed_date
                    ),
                ),
                Some(parsed_date),
            ));
        }
        if selected.contains_key(&key) {
            return Err(StockCloseManifestFailure::new(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::StockCloseSource,
                    format!(
                        "duplicate stock close fact for {} {}",
                        row.code, parsed_date
                    ),
                ),
                Some(parsed_date),
            ));
        }
        let close = row.close.ok_or_else(|| {
            StockCloseManifestFailure::new(
                AttributionReplayError::unavailable(
                    AttributionUnavailable::StockCloseUnavailable,
                    true,
                    format!("stock close is absent for {} {}", row.code, parsed_date),
                ),
                Some(parsed_date),
            )
        })?;
        if !close.is_finite() || close <= 0.0 {
            return Err(StockCloseManifestFailure::new(
                AttributionReplayError::integrity(
                    AttributionIntegrityFailure::StockCloseSource,
                    format!("stock close is invalid for {} {}", row.code, parsed_date),
                ),
                Some(parsed_date),
            ));
        }
        row.data_source
            .as_deref()
            .filter(|source| !source.trim().is_empty())
            .ok_or_else(|| {
                StockCloseManifestFailure::new(
                    AttributionReplayError::unavailable(
                        AttributionUnavailable::StockCloseUnavailable,
                        true,
                        format!(
                            "stock close source is absent for {} {}",
                            row.code, parsed_date
                        ),
                    ),
                    Some(parsed_date),
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
            return Err(StockCloseManifestFailure::new(
                AttributionReplayError::unavailable(
                    AttributionUnavailable::StockCloseUnavailable,
                    true,
                    format!("stock close is unavailable for {code} {date}"),
                ),
                Some(*date),
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

fn canonical_missing_fill_ids(required: &HashSet<i64>, seen: &HashSet<i64>) -> Vec<i64> {
    let mut missing = required.difference(seen).copied().collect::<Vec<_>>();
    missing.sort_unstable();
    missing
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
        let missing = canonical_missing_fill_ids(&fill_ids, &seen);
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::FeeEvidence,
            format!("fee evidence is missing fill ids {missing:?}"),
        ));
    }
    Ok(FeeEvidenceAvailability::Available(ledger.clone()))
}

fn validate_epoch_fee_ledger(
    ledger: Option<&AuthoritativeFillFeeLedger>,
    verified_fills: &[ReplayFillEvidence],
    attributable_fills: &[ReplayFillEvidence],
) -> Result<FeeEvidenceAvailability, AttributionReplayError> {
    let Some(ledger) = ledger else {
        return Ok(FeeEvidenceAvailability::Unavailable {
            code: AttributionUnavailable::FeeEvidenceUnavailable,
            retryable: false,
            detail: "explicit authoritative per-fill fee ledger is unavailable".to_owned(),
        });
    };
    let verified_ids = verified_fills
        .iter()
        .map(|evidence| evidence.fill.id)
        .collect::<HashSet<_>>();
    let attributable_ids = attributable_fills
        .iter()
        .map(|evidence| evidence.fill.id)
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    for entry in &ledger.entries {
        if !verified_ids.contains(&entry.fill_id) {
            return Err(AttributionReplayError::integrity(
                AttributionIntegrityFailure::FeeEvidence,
                format!(
                    "fee evidence references unknown verified fill id={}",
                    entry.fill_id
                ),
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
    if !attributable_ids.is_subset(&seen) {
        let missing = canonical_missing_fill_ids(&attributable_ids, &seen);
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::FeeEvidence,
            format!("fee evidence is missing attributable fill ids {missing:?}"),
        ));
    }
    let projected = AuthoritativeFillFeeLedger {
        entries: ledger
            .entries
            .iter()
            .filter(|entry| attributable_ids.contains(&entry.fill_id))
            .cloned()
            .collect(),
    };
    validate_fee_ledger(Some(&projected), attributable_fills)
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
        epoch: evidence.epoch.clone(),
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

const ATTRIBUTION_REPLAY_RULE_VERSION: &str = "BR-251-v1";
const RUNNER_SOURCE_SUMMARY_DOMAIN: &[u8] = b"BR255_RUNNER_SOURCE_SUMMARY_V3\0";
const RUNNER_FAILURE_LEAF_DOMAIN: &[u8] = b"BR251_RUNNER_FAILURE_LEAF_V1\0";
#[cfg(test)]
const RUNNER_TEST_SEMANTICS_DOMAIN: &[u8] = b"BR251_RUNNER_TEST_SEMANTICS_V1\0";
const RUNNER_BENCHMARK_MANIFEST_DOMAIN: &[u8] = b"BR251_RUNNER_DAY_MANIFESTS_V1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayMode {
    Scheduled {
        invoked_at: DateTime<FixedOffset>,
    },
    Range {
        from: NaiveDate,
        to: NaiveDate,
        invoked_at: DateTime<FixedOffset>,
    },
    Quarter {
        year: i32,
        quarter: u8,
        invoked_at: DateTime<FixedOffset>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayRequest {
    pub mode: ReplayMode,
    pub epoch: AttributionEpochSelector,
    pub benchmark_day_manifests: Vec<BenchmarkDayManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchmarkDayManifest {
    pub trading_date: NaiveDate,
    pub manifest_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayErrorClass {
    Unavailable,
    FailedIntegrity,
    Storage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayStage {
    Request,
    Calendar,
    Epoch,
    TradeEvidence,
    Benchmark,
    Compute,
    Store,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayEvidenceFailureKind {
    StockCloseAbsent,
    BenchmarkExactAbsent,
}

impl ReplayStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Calendar => "calendar",
            Self::Epoch => "epoch",
            Self::TradeEvidence => "trade_evidence",
            Self::Benchmark => "benchmark",
            Self::Compute => "compute",
            Self::Store => "store",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayError {
    class: ReplayErrorClass,
    stage: ReplayStage,
    code: &'static str,
    retryable: bool,
    redacted_message: String,
    failure_receipt: Option<Box<AttributionFailureReceipt>>,
    failure_date: Option<NaiveDate>,
    failure_fingerprint: [u8; 32],
    evidence_failure_kind: Option<ReplayEvidenceFailureKind>,
}

impl ReplayError {
    fn new(
        class: ReplayErrorClass,
        stage: ReplayStage,
        code: &'static str,
        retryable: bool,
    ) -> Self {
        let failure_fingerprint = runner_failure_leaf_fingerprint(
            class,
            stage,
            code,
            retryable,
            code.as_bytes(),
            None,
            None,
        );
        Self {
            class,
            stage,
            code,
            retryable,
            redacted_message: format!("BR-251 replay failed at {} with {code}", stage.as_str()),
            failure_receipt: None,
            failure_date: None,
            failure_fingerprint,
            evidence_failure_kind: None,
        }
    }

    fn with_failure_date(mut self, failure_date: NaiveDate) -> Self {
        self.failure_date = Some(failure_date);
        self.failure_fingerprint = runner_failure_leaf_fingerprint(
            self.class,
            self.stage,
            self.code,
            self.retryable,
            self.code.as_bytes(),
            Some(failure_date),
            None,
        );
        self
    }

    fn with_typed_failure(
        mut self,
        kind: &'static str,
        detail: &[u8],
        failure_date: Option<NaiveDate>,
        manifest_hash: Option<&str>,
    ) -> Self {
        self.failure_date = failure_date;
        self.failure_fingerprint = runner_failure_leaf_fingerprint(
            self.class,
            self.stage,
            kind,
            self.retryable,
            detail,
            failure_date,
            manifest_hash,
        );
        self
    }

    fn with_benchmark_failure_context(
        mut self,
        failure_date: NaiveDate,
        manifest_hash: &str,
    ) -> Self {
        self.failure_date = Some(failure_date);
        self.failure_fingerprint = runner_failure_leaf_fingerprint(
            self.class,
            self.stage,
            self.code,
            self.retryable,
            self.code.as_bytes(),
            Some(failure_date),
            Some(manifest_hash),
        );
        self
    }

    fn with_evidence_failure_kind(mut self, kind: ReplayEvidenceFailureKind) -> Self {
        self.evidence_failure_kind = Some(kind);
        self
    }

    fn into_current_session_incomplete(mut self) -> Self {
        self.code = "current_session_incomplete";
        self.retryable = true;
        self.redacted_message = format!(
            "BR-251 replay failed at {} with current_session_incomplete",
            self.stage.as_str()
        );
        self
    }

    fn with_failure_receipt(mut self, receipt: AttributionFailureReceipt) -> Self {
        self.failure_receipt = Some(Box::new(receipt));
        self
    }

    pub const fn class(&self) -> ReplayErrorClass {
        self.class
    }

    pub const fn stage(&self) -> ReplayStage {
        self.stage
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub fn redacted_message(&self) -> &str {
        &self.redacted_message
    }

    pub fn failure_receipt(&self) -> Option<&AttributionFailureReceipt> {
        self.failure_receipt.as_deref()
    }
}

fn runner_failure_leaf_fingerprint(
    class: ReplayErrorClass,
    stage: ReplayStage,
    kind: &str,
    retryable: bool,
    detail: &[u8],
    failure_date: Option<NaiveDate>,
    manifest_hash: Option<&str>,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(RUNNER_FAILURE_LEAF_DOMAIN);
    hasher.update([match class {
        ReplayErrorClass::Unavailable => 0,
        ReplayErrorClass::FailedIntegrity => 1,
        ReplayErrorClass::Storage => 2,
    }]);
    update_len_prefixed(&mut hasher, stage.as_str().as_bytes());
    update_len_prefixed(&mut hasher, kind.as_bytes());
    hasher.update([u8::from(retryable)]);
    update_len_prefixed(&mut hasher, detail);
    match failure_date {
        Some(date) => {
            hasher.update([1]);
            update_len_prefixed(&mut hasher, date.to_string().as_bytes());
        }
        None => hasher.update([0]),
    }
    match manifest_hash {
        Some(hash) => {
            hasher.update([1]);
            update_len_prefixed(&mut hasher, hash.as_bytes());
        }
        None => hasher.update([0]),
    }
    hasher.finalize().into()
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} ({:?}, retryable={})",
            self.redacted_message, self.class, self.retryable
        )
    }
}

impl std::error::Error for ReplayError {}

/// Opaque output of the runner's one read-only prepare pipeline.
#[derive(Debug, Clone)]
pub struct PreparedAttributionReport {
    invocation: AttributionInvocation,
    report: AttributionComputationReport,
    canonical_result_bytes: Vec<u8>,
    result_payload: serde_json::Value,
    trade_manifest_hash: String,
    fee: AttributionEvidenceHash,
    stock_close_manifest_hash: String,
    benchmark_manifest_hash: String,
    benchmark_day_manifests: Vec<BenchmarkDayManifest>,
    calendar_authority_hash: String,
}

impl PreparedAttributionReport {
    pub fn invocation(&self) -> &AttributionInvocation {
        &self.invocation
    }

    pub fn report(&self) -> &AttributionComputationReport {
        &self.report
    }

    pub fn epoch_selector(&self) -> &AttributionEpochSelector {
        self.report.epoch_selector()
    }

    pub fn epoch_id(&self) -> Option<&str> {
        self.report.epoch_id()
    }

    pub fn epoch_receipt_hash(&self) -> Option<&str> {
        self.report.epoch_receipt_hash()
    }

    pub fn epoch_effective_date(&self) -> Option<NaiveDate> {
        self.report.epoch_effective_date()
    }

    pub fn legacy_carry_manifest_hash(&self) -> Option<&str> {
        self.report.legacy_carry_manifest_hash()
    }

    pub fn exclusion_manifest_hash(&self) -> Option<&str> {
        self.report.exclusion_manifest_hash()
    }

    pub fn remaining_quarantine(&self) -> &[LegacyCarryPosition] {
        self.report.remaining_quarantine()
    }

    pub fn released_codes(&self) -> usize {
        self.report.released_codes()
    }

    pub fn excluded_fills(&self) -> &[EpochExclusion] {
        self.report.excluded_fills()
    }

    pub fn overlap_buy_count(&self) -> usize {
        self.report.overlap_buy_count()
    }

    pub fn overlap_sell_count(&self) -> usize {
        self.report.overlap_sell_count()
    }

    pub fn mixed_exit_count(&self) -> usize {
        self.report.mixed_exit_count()
    }

    pub fn excluded_fill_count(&self) -> usize {
        self.report.excluded_fill_count()
    }

    pub fn scoped_fill_manifest_hash(&self) -> &str {
        &self.report.epoch.scoped_fill_manifest_hash
    }

    pub fn canonical_result_bytes(&self) -> &[u8] {
        &self.canonical_result_bytes
    }

    pub fn trade_manifest_hash(&self) -> &str {
        &self.trade_manifest_hash
    }

    pub fn stock_close_manifest_hash(&self) -> &str {
        &self.stock_close_manifest_hash
    }

    pub fn benchmark_manifest_hash(&self) -> &str {
        &self.benchmark_manifest_hash
    }

    pub fn benchmark_day_manifests(&self) -> &[BenchmarkDayManifest] {
        &self.benchmark_day_manifests
    }

    pub fn calendar_authority_hash(&self) -> &str {
        &self.calendar_authority_hash
    }

    fn report_epoch_binding(&self) -> Result<AttributionReportEpochBinding, ReplayError> {
        match self.epoch_selector() {
            AttributionEpochSelector::Legacy => Ok(AttributionReportEpochBinding::Legacy),
            AttributionEpochSelector::Active | AttributionEpochSelector::Exact(_) => {
                let (
                    Some(epoch_id),
                    Some(epoch_receipt_hash),
                    Some(effective_date),
                    Some(legacy_carry_manifest_hash),
                    Some(exclusion_manifest_hash),
                ) = (
                    self.epoch_id(),
                    self.epoch_receipt_hash(),
                    self.epoch_effective_date(),
                    self.legacy_carry_manifest_hash(),
                    self.exclusion_manifest_hash(),
                )
                else {
                    return Err(ReplayError::new(
                        ReplayErrorClass::FailedIntegrity,
                        ReplayStage::Epoch,
                        "attribution_report_epoch_binding_missing",
                        false,
                    ));
                };
                Ok(AttributionReportEpochBinding::Epoch {
                    epoch_id: epoch_id.to_owned(),
                    epoch_receipt_hash: epoch_receipt_hash.to_owned(),
                    effective_date,
                    legacy_carry_manifest_hash: legacy_carry_manifest_hash.to_owned(),
                    exclusion_manifest_hash: exclusion_manifest_hash.to_owned(),
                })
            }
        }
    }
}

/// Opaque result of committing exactly one prepared attribution report.
///
/// The report and receipt are returned together so presentation never reruns
/// the evidence pipeline or renders a payload different from the committed one.
#[derive(Debug, Clone)]
pub struct CommittedAttributionReport {
    prepared: PreparedAttributionReport,
    receipt: AttributionReportReceipt,
}

impl CommittedAttributionReport {
    pub fn prepared(&self) -> &PreparedAttributionReport {
        &self.prepared
    }

    pub fn receipt(&self) -> &AttributionReportReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone)]
struct AdmittedReplayRequest {
    mode: ReplayMode,
    epoch: AttributionEpochSelector,
    provisional_invocation: AttributionInvocation,
    benchmark_day_manifests: Vec<BenchmarkDayManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FailureEvidenceState {
    Unknown,
    NotApplicable,
    Unavailable([u8; 32]),
    Available(String),
}

#[derive(Debug, Clone)]
struct FailureEvidenceSummary {
    mode: &'static str,
    invoked_at: DateTime<FixedOffset>,
    target_from: NaiveDate,
    target_to: NaiveDate,
    rule_version: String,
    epoch_selector: String,
    requested_benchmark_days: Vec<BenchmarkDayManifest>,
    calendar: FailureEvidenceState,
    epoch_id: FailureEvidenceState,
    epoch_receipt: FailureEvidenceState,
    legacy_carry: FailureEvidenceState,
    exclusions: FailureEvidenceState,
    remaining_quarantine: FailureEvidenceState,
    released_codes: FailureEvidenceState,
    trade: FailureEvidenceState,
    stock_close: FailureEvidenceState,
    fee: FailureEvidenceState,
    benchmark_days: BTreeMap<NaiveDate, FailureEvidenceState>,
}

impl FailureEvidenceSummary {
    fn new(admitted: &AdmittedReplayRequest) -> Self {
        let mode = match admitted.provisional_invocation.mode {
            AttributionRunMode::Scheduled => "scheduled",
            AttributionRunMode::Range => "range",
            AttributionRunMode::Quarter => "quarter",
        };
        let benchmark_days = admitted
            .benchmark_day_manifests
            .iter()
            .map(|binding| (binding.trading_date, FailureEvidenceState::Unknown))
            .collect();
        Self {
            mode,
            invoked_at: admitted.provisional_invocation.invoked_at,
            target_from: admitted.provisional_invocation.target_from,
            target_to: admitted.provisional_invocation.target_to,
            rule_version: admitted.provisional_invocation.rule_version.clone(),
            epoch_selector: admitted.epoch.canonical_value(),
            requested_benchmark_days: admitted.benchmark_day_manifests.clone(),
            calendar: FailureEvidenceState::Unknown,
            epoch_id: FailureEvidenceState::Unknown,
            epoch_receipt: FailureEvidenceState::Unknown,
            legacy_carry: FailureEvidenceState::Unknown,
            exclusions: FailureEvidenceState::Unknown,
            remaining_quarantine: FailureEvidenceState::Unknown,
            released_codes: FailureEvidenceState::Unknown,
            trade: FailureEvidenceState::Unknown,
            stock_close: FailureEvidenceState::Unknown,
            fee: FailureEvidenceState::Unknown,
            benchmark_days,
        }
    }

    fn record_calendar(&mut self, calendar: &VerifiedReplayCalendar) {
        self.target_from = calendar.target_from();
        self.target_to = calendar.target_to();
        self.calendar = FailureEvidenceState::Available(calendar.authority_hash().to_owned());
    }

    fn record_replay_evidence(&mut self, evidence: &AttributionReplayEvidence) {
        self.record_epoch_evidence(evidence.epoch());
        self.trade = FailureEvidenceState::Available(evidence.trade_manifest_hash().to_owned());
        self.stock_close =
            FailureEvidenceState::Available(evidence.stock_closes().manifest_hash().to_owned());
        self.fee = failure_fee_state(evidence.fees());
    }

    fn record_resolved_epoch(&mut self, epoch: &ResolvedAttributionEpoch) {
        match epoch {
            ResolvedAttributionEpoch::Legacy => {
                self.epoch_id = FailureEvidenceState::NotApplicable;
                self.epoch_receipt = FailureEvidenceState::NotApplicable;
                self.legacy_carry = FailureEvidenceState::NotApplicable;
                self.exclusions = FailureEvidenceState::NotApplicable;
                self.remaining_quarantine = FailureEvidenceState::NotApplicable;
                self.released_codes = FailureEvidenceState::NotApplicable;
            }
            ResolvedAttributionEpoch::Epoch(receipt) => {
                self.epoch_id = FailureEvidenceState::Available(receipt.epoch_id.clone());
                self.epoch_receipt = FailureEvidenceState::Available(receipt.receipt_hash.clone());
                self.legacy_carry =
                    FailureEvidenceState::Available(receipt.legacy_carry_manifest_hash.clone());
            }
        }
    }

    fn record_epoch_evidence(&mut self, epoch: &AttributionEpochReplayEvidence) {
        if matches!(epoch.selector(), AttributionEpochSelector::Legacy) {
            return;
        }
        if let Some(epoch_id) = epoch.epoch_id() {
            self.epoch_id = FailureEvidenceState::Available(epoch_id.to_owned());
        }
        if let Some(receipt_hash) = epoch.receipt_hash() {
            self.epoch_receipt = FailureEvidenceState::Available(receipt_hash.to_owned());
        }
        if let Some(carry_hash) = epoch.legacy_carry_manifest_hash() {
            self.legacy_carry = FailureEvidenceState::Available(carry_hash.to_owned());
        }
        if let Some(exclusion_hash) = epoch.exclusion_manifest_hash() {
            self.exclusions = FailureEvidenceState::Available(exclusion_hash.to_owned());
        }
        self.remaining_quarantine = FailureEvidenceState::Available(
            canonical_legacy_carry_manifest_hash(epoch.remaining_quarantine()),
        );
        self.released_codes = FailureEvidenceState::Available(epoch.released_codes().to_string());
    }

    fn record_scoped_epoch_replay(
        &mut self,
        epoch: &AttributionEpochReplayEvidence,
        fills: &[ReplayFillEvidence],
    ) {
        self.record_epoch_evidence(epoch);
        self.trade = FailureEvidenceState::Available(replay_trade_manifest_hash(epoch, fills));
    }

    fn record_load_failure(
        &mut self,
        failure: &AttributionReplayLoadFailure,
        leaf_fingerprint: [u8; 32],
    ) {
        if let Some(identity) = &failure.progress.trade_manifest_hash {
            self.trade = FailureEvidenceState::Available(identity.clone());
        }
        if let Some(identity) = &failure.progress.stock_close_manifest_hash {
            self.stock_close = FailureEvidenceState::Available(identity.clone());
        }
        if let Some(fee) = &failure.progress.fee {
            self.fee = failure_fee_state(fee);
        }
        match failure.stage {
            AttributionReplayLoadStage::Trade
                if matches!(self.trade, FailureEvidenceState::Unknown) =>
            {
                self.trade = FailureEvidenceState::Unavailable(leaf_fingerprint);
            }
            AttributionReplayLoadStage::StockClose
                if matches!(self.stock_close, FailureEvidenceState::Unknown) =>
            {
                self.stock_close = FailureEvidenceState::Unavailable(leaf_fingerprint);
            }
            AttributionReplayLoadStage::Fee
                if matches!(self.fee, FailureEvidenceState::Unknown) =>
            {
                self.fee = FailureEvidenceState::Unavailable(leaf_fingerprint);
            }
            AttributionReplayLoadStage::Finalize
            | AttributionReplayLoadStage::Trade
            | AttributionReplayLoadStage::StockClose
            | AttributionReplayLoadStage::Fee => {}
        }
    }

    fn record_benchmark_available(&mut self, date: NaiveDate, manifest_hash: &str) {
        self.benchmark_days.insert(
            date,
            FailureEvidenceState::Available(manifest_hash.to_owned()),
        );
    }

    fn record_benchmark_unavailable(&mut self, date: NaiveDate, fingerprint: [u8; 32]) {
        self.benchmark_days
            .insert(date, FailureEvidenceState::Unavailable(fingerprint));
    }

    fn source_summary_hash(&self, error: &ReplayError) -> String {
        let mut hasher = Sha256::new();
        hasher.update(RUNNER_SOURCE_SUMMARY_DOMAIN);
        update_len_prefixed(&mut hasher, self.mode.as_bytes());
        update_len_prefixed(&mut hasher, self.invoked_at.to_rfc3339().as_bytes());
        update_len_prefixed(&mut hasher, self.target_from.to_string().as_bytes());
        update_len_prefixed(&mut hasher, self.target_to.to_string().as_bytes());
        update_len_prefixed(&mut hasher, self.rule_version.as_bytes());
        update_len_prefixed(&mut hasher, self.epoch_selector.as_bytes());
        hasher.update((self.requested_benchmark_days.len() as u64).to_be_bytes());
        for binding in &self.requested_benchmark_days {
            update_len_prefixed(&mut hasher, binding.trading_date.to_string().as_bytes());
            update_len_prefixed(&mut hasher, binding.manifest_hash.as_bytes());
        }
        update_failure_evidence_state(&mut hasher, &self.calendar);
        update_failure_evidence_state(&mut hasher, &self.epoch_id);
        update_failure_evidence_state(&mut hasher, &self.epoch_receipt);
        update_failure_evidence_state(&mut hasher, &self.legacy_carry);
        update_failure_evidence_state(&mut hasher, &self.exclusions);
        update_failure_evidence_state(&mut hasher, &self.remaining_quarantine);
        update_failure_evidence_state(&mut hasher, &self.released_codes);
        update_failure_evidence_state(&mut hasher, &self.trade);
        update_failure_evidence_state(&mut hasher, &self.stock_close);
        update_failure_evidence_state(&mut hasher, &self.fee);
        hasher.update((self.benchmark_days.len() as u64).to_be_bytes());
        for (date, state) in &self.benchmark_days {
            update_len_prefixed(&mut hasher, date.to_string().as_bytes());
            update_failure_evidence_state(&mut hasher, state);
        }
        update_len_prefixed(&mut hasher, error.stage.as_str().as_bytes());
        update_len_prefixed(&mut hasher, error.code.as_bytes());
        hasher.update([u8::from(error.retryable)]);
        hasher.update(error.failure_fingerprint);
        hex::encode(hasher.finalize())
    }
}

fn update_failure_evidence_state(hasher: &mut Sha256, state: &FailureEvidenceState) {
    match state {
        FailureEvidenceState::Unknown => hasher.update([0]),
        FailureEvidenceState::NotApplicable => hasher.update([3]),
        FailureEvidenceState::Unavailable(fingerprint) => {
            hasher.update([1]);
            hasher.update(fingerprint);
        }
        FailureEvidenceState::Available(identity) => {
            hasher.update([2]);
            update_len_prefixed(hasher, identity.as_bytes());
        }
    }
}

fn failure_fee_state(fee: &FeeEvidenceAvailability) -> FailureEvidenceState {
    match fee {
        FeeEvidenceAvailability::Available(ledger) => {
            let mut bindings = ledger
                .entries()
                .iter()
                .map(|entry| FeeEvidenceBinding {
                    fill_id: entry.fill_id(),
                    evidence_hash: entry.evidence_hash().to_owned(),
                })
                .collect::<Vec<_>>();
            bindings.sort_by_key(|binding| binding.fill_id);
            FailureEvidenceState::Available(canonical_replay_fee_basis_id(&bindings))
        }
        FeeEvidenceAvailability::Unavailable {
            code,
            retryable,
            detail,
        } => FailureEvidenceState::Unavailable(runner_failure_leaf_fingerprint(
            ReplayErrorClass::Unavailable,
            ReplayStage::TradeEvidence,
            code.code(),
            *retryable,
            detail.as_bytes(),
            None,
            None,
        )),
    }
}

struct PrepareFailure {
    error: ReplayError,
    invocation: AttributionInvocation,
    evidence: FailureEvidenceSummary,
}

enum EpochReplaySnapshotFailure {
    Epoch(AttributionEpochStoreError),
    Replay(ReplayError),
    Load(AttributionReplayLoadFailure),
}

fn map_epoch_replay_transaction_error(
    error: AttributionReadTransactionError<EpochReplaySnapshotFailure>,
) -> EpochReplaySnapshotFailure {
    match error {
        AttributionReadTransactionError::Operation(error) => error,
        AttributionReadTransactionError::StorageUnavailable { detail }
        | AttributionReadTransactionError::Transaction { detail } => {
            EpochReplaySnapshotFailure::Epoch(AttributionEpochStoreError::Unavailable {
                reason_code: "attribution_epoch_storage_unavailable",
                retryable: true,
                detail: format!("BR-255 epoch replay snapshot unavailable: {detail}"),
            })
        }
        AttributionReadTransactionError::Authority(
            DatabaseAuthorityError::DescriptorAttestationUnavailable { detail },
        ) => EpochReplaySnapshotFailure::Epoch(AttributionEpochStoreError::Unavailable {
            reason_code: "attribution_database_authority_unavailable",
            retryable: false,
            detail: format!("BR-255 epoch replay database authority unavailable: {detail}"),
        }),
        AttributionReadTransactionError::Authority(
            DatabaseAuthorityError::DescriptorIntegrityFailed { detail },
        ) => EpochReplaySnapshotFailure::Epoch(AttributionEpochStoreError::FailedIntegrity {
            reason_code: "attribution_epoch_integrity_failed",
            detail: format!("BR-255 epoch replay database authority changed: {detail}"),
        }),
        AttributionReadTransactionError::SnapshotIntegrity { detail } => {
            EpochReplaySnapshotFailure::Epoch(AttributionEpochStoreError::FailedIntegrity {
                reason_code: "attribution_epoch_integrity_failed",
                detail: format!("BR-255 epoch replay snapshot integrity failed: {detail}"),
            })
        }
    }
}

struct ScopedVerifiedEpochReplay {
    verified_fills: Vec<ReplayFillEvidence>,
    fills: Vec<ReplayFillEvidence>,
    epoch: AttributionEpochReplayEvidence,
}

fn scope_verified_epoch_replay(
    selector: &AttributionEpochSelector,
    receipt: &AttributionEpochReceipt,
    verified: &VerifiedEpochFillSet,
) -> Result<ScopedVerifiedEpochReplay, AttributionReplayError> {
    if canonical_legacy_carry_manifest_hash(verified.carry()) != receipt.legacy_carry_manifest_hash
    {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReplayEvidence,
            "verified retained carry differs from the resolved receipt",
        ));
    }
    let shanghai = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReplayEvidence,
            "Shanghai fixed offset is unavailable",
        )
    })?;
    let verified_fills = verified
        .fills()
        .iter()
        .map(|source| ReplayFillEvidence {
            fill: source.fill().clone(),
            terminal_audit_id: source.terminal_audit_id(),
            terminal_audit_hash: source.terminal_audit_hash().to_owned(),
            terminal_time: source.terminal_time().with_timezone(&shanghai),
        })
        .collect::<Vec<_>>();
    let source_rows = verified_fills
        .iter()
        .map(|evidence| evidence.fill.clone())
        .collect::<Vec<_>>();
    let scoped = scope_epoch_fills(
        &source_rows,
        receipt.effective_trading_date,
        verified.carry(),
    )
    .map_err(|detail| {
        AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReplayEvidence,
            format!("epoch fill scoping failed: {detail}"),
        )
    })?;
    let exclusion_manifest_hash =
        canonical_exclusion_manifest_hash(&scoped.exclusions, &source_rows).map_err(|detail| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::ReplayEvidence,
                format!("epoch exclusion manifest failed: {detail}"),
            )
        })?;
    let scoped_fill_manifest_hash = canonical_scoped_fill_manifest_hash(&scoped.attributable)
        .map_err(|detail| {
            AttributionReplayError::integrity(
                AttributionIntegrityFailure::ReplayEvidence,
                format!("epoch scoped fill manifest failed: {detail}"),
            )
        })?;
    let attributable_ids = scoped
        .attributable
        .iter()
        .map(|row| row.id)
        .collect::<HashSet<_>>();
    let attributable_fills = verified_fills
        .iter()
        .filter(|evidence| attributable_ids.contains(&evidence.fill.id))
        .cloned()
        .collect::<Vec<_>>();
    if attributable_fills.len() != attributable_ids.len() {
        return Err(AttributionReplayError::integrity(
            AttributionIntegrityFailure::ReplayEvidence,
            "scoped epoch fill lost its database-issued terminal evidence",
        ));
    }
    let epoch = AttributionEpochReplayEvidence::resolved(
        selector.clone(),
        receipt,
        ResolvedAttributionEpochReplayScope {
            exclusions: scoped.exclusions,
            exclusion_manifest_hash,
            remaining_quarantine: scoped.remaining_quarantine,
            released_codes: scoped.released_codes,
            scoped_fill_manifest_hash,
            verified_filled_manifest_hash: verified.filled_manifest_hash().to_owned(),
            verified_terminal_binding_manifest_hash: verified
                .terminal_binding_manifest_hash()
                .to_owned(),
            verified_order_audit_tip_hash: verified.order_audit_tip_hash().to_owned(),
        },
    );
    Ok(ScopedVerifiedEpochReplay {
        verified_fills,
        fills: attributable_fills,
        epoch,
    })
}

/// Production construction always keeps minute-label semantics unverified.
/// The TEST_CODE-only constructor is not callable by an external/production caller:
///
/// ```compile_fail
/// use stock_analysis::performance::attribution_replay::{
///     AttributionReplayLoader, AttributionReplayRunner,
/// };
/// use stock_analysis::database::DatabaseManager;
///
/// fn unlock<'a>(database: &'a DatabaseManager, loader: AttributionReplayLoader) {
///     let _ = AttributionReplayRunner::new_for_test(
///         database,
///         loader,
///         "sh000300",
///         "caller_hash",
///     );
/// }
/// ```
///
/// A prepared preview is opaque and cannot be forged or passed to `commit`:
///
/// ```compile_fail
/// use stock_analysis::performance::attribution_replay::PreparedAttributionReport;
///
/// fn forge() -> PreparedAttributionReport {
///     PreparedAttributionReport { todo!() }
/// }
/// ```
pub struct AttributionReplayRunner<'a> {
    database: &'a DatabaseManager,
    loader: AttributionReplayLoader,
    benchmark_instrument: String,
    minute_semantics: MinuteLabelSemantics,
    fee_ledger: Option<AuthoritativeFillFeeLedger>,
}

impl<'a> AttributionReplayRunner<'a> {
    #[must_use]
    pub fn new(database: &'a DatabaseManager, loader: AttributionReplayLoader) -> Self {
        Self {
            database,
            loader,
            benchmark_instrument: HS300_CANONICAL.to_owned(),
            minute_semantics: MinuteLabelSemantics::Unverified,
            fee_ledger: None,
        }
    }

    #[cfg(test)]
    fn new_for_test(
        database: &'a DatabaseManager,
        loader: AttributionReplayLoader,
        benchmark_instrument: &str,
        semantics_evidence: &str,
    ) -> Self {
        assert!(benchmark_instrument.starts_with("TEST_CODE"));
        assert!(semantics_evidence.starts_with("TEST_CODE"));
        let mut hasher = Sha256::new();
        hasher.update(RUNNER_TEST_SEMANTICS_DOMAIN);
        update_len_prefixed(&mut hasher, semantics_evidence.as_bytes());
        Self {
            database,
            loader,
            benchmark_instrument: benchmark_instrument.to_owned(),
            minute_semantics: MinuteLabelSemantics::EndLabelVerified {
                evidence_hash: hex::encode(hasher.finalize()),
            },
            fee_ledger: None,
        }
    }

    #[cfg(test)]
    fn new_for_test_with_fee_ledger(
        database: &'a DatabaseManager,
        loader: AttributionReplayLoader,
        benchmark_instrument: &str,
        semantics_evidence: &str,
        fee_ledger: AuthoritativeFillFeeLedger,
    ) -> Self {
        let mut runner =
            Self::new_for_test(database, loader, benchmark_instrument, semantics_evidence);
        runner.fee_ledger = Some(fee_ledger);
        runner
    }

    pub fn preview(
        &self,
        request: ReplayRequest,
    ) -> Result<PreparedAttributionReport, ReplayError> {
        let admitted = admit_replay_request(request)?;
        self.prepare(&admitted).map_err(|failure| failure.error)
    }

    pub fn commit(&self, request: ReplayRequest) -> Result<AttributionReportReceipt, ReplayError> {
        self.commit_with_report(request)
            .map(|committed| committed.receipt)
    }

    pub fn commit_with_report(
        &self,
        request: ReplayRequest,
    ) -> Result<CommittedAttributionReport, ReplayError> {
        let admitted = admit_replay_request(request)?;
        let prepared = match self.prepare(&admitted) {
            Ok(prepared) => prepared,
            Err(failure) => {
                let error = failure.error;
                let source_summary_hash = failure.evidence.source_summary_hash(&error);
                let receipt = AttributionReportStore::new(self.database)
                    .commit_failure(AttributionFailureAppend {
                        invocation: failure.invocation,
                        stage: error.stage.as_str().to_owned(),
                        code: error.code.to_owned(),
                        retryable: error.retryable,
                        source_summary_hash,
                        redacted_message: error.redacted_message.clone(),
                    })
                    .map_err(map_store_error)?;
                return Err(error.with_failure_receipt(receipt));
            }
        };
        let epoch = prepared.report_epoch_binding()?;
        let receipt = AttributionReportStore::new(self.database)
            .commit_report(AttributionReportAppend {
                invocation: prepared.invocation.clone(),
                epoch,
                trade_hash: prepared.trade_manifest_hash.clone(),
                fee: prepared.fee.clone(),
                stock_close_hash: prepared.stock_close_manifest_hash.clone(),
                benchmark_manifest_hash: prepared.benchmark_manifest_hash.clone(),
                calendar_authority_hash: prepared.calendar_authority_hash.clone(),
                regime: AttributionEvidenceHash::Unavailable(
                    "market_regime_unavailable".to_owned(),
                ),
                result_payload: prepared.result_payload.clone(),
            })
            .map_err(map_store_error)?;
        Ok(CommittedAttributionReport { prepared, receipt })
    }

    fn prepare(
        &self,
        admitted: &AdmittedReplayRequest,
    ) -> Result<PreparedAttributionReport, Box<PrepareFailure>> {
        let mut summary = FailureEvidenceSummary::new(admitted);
        let calendar = match resolve_admitted_calendar(admitted) {
            Ok(calendar) => calendar,
            Err(error) => {
                let error = map_calendar_error(error);
                summary.calendar = FailureEvidenceState::Unavailable(error.failure_fingerprint);
                return Err(Box::new(PrepareFailure {
                    error,
                    invocation: admitted.provisional_invocation.clone(),
                    evidence: summary,
                }));
            }
        };
        summary.record_calendar(&calendar);
        let invocation = AttributionInvocation {
            target_from: calendar.target_from(),
            target_to: calendar.target_to(),
            ..admitted.provisional_invocation.clone()
        };
        let replay_request = AttributionReplayRequest {
            from: calendar.target_from(),
            to: calendar.target_to(),
            required_trading_dates: calendar.required_trading_dates().to_vec(),
            fee_ledger: self.fee_ledger.clone(),
        };
        let loaded = match &admitted.epoch {
            AttributionEpochSelector::Legacy => {
                summary.record_resolved_epoch(&ResolvedAttributionEpoch::Legacy);
                self.loader.load_with_progress(&replay_request)
            }
            selector => {
                let snapshot = self
                    .database
                    .attribution_read_transaction(|connection| {
                        let resolved = load_selector_with_connection(connection, selector)
                            .map_err(EpochReplaySnapshotFailure::Epoch)?;
                        summary.record_resolved_epoch(&resolved);
                        let receipt = match &resolved {
                            ResolvedAttributionEpoch::Epoch(receipt) => receipt,
                            ResolvedAttributionEpoch::Legacy => {
                                unreachable!("Active/Exact cannot resolve Legacy")
                            }
                        };
                        if calendar.target_from() < receipt.effective_trading_date {
                            return Err(EpochReplaySnapshotFailure::Epoch(
                                AttributionEpochStoreError::FailedIntegrity {
                                    reason_code: "attribution_epoch_range_before_effective",
                                    detail: format!(
                                        "BR-255 attribution range {}..={} precedes effective date {}",
                                        calendar.target_from(),
                                        calendar.target_to(),
                                        receipt.effective_trading_date
                                    ),
                                },
                            ));
                        }
                        let verified = load_verified_epoch_fills_until(
                            connection,
                            &resolved,
                            calendar.target_to(),
                        )
                        .map_err(EpochReplaySnapshotFailure::Epoch)?;
                        let scoped = scope_verified_epoch_replay(selector, receipt, &verified)
                            .map_err(|error| {
                                EpochReplaySnapshotFailure::Replay(map_attribution_error(
                                    ReplayStage::Epoch,
                                    error,
                                ))
                            })?;
                        summary.record_scoped_epoch_replay(&scoped.epoch, &scoped.fills);
                        let fees = validate_epoch_fee_ledger(
                            self.fee_ledger.as_ref(),
                            &scoped.verified_fills,
                            &scoped.fills,
                        )
                        .map_err(|error| {
                            let error =
                                map_attribution_error(ReplayStage::TradeEvidence, error);
                            summary.fee =
                                FailureEvidenceState::Unavailable(error.failure_fingerprint);
                            EpochReplaySnapshotFailure::Replay(error)
                        })?;
                        AttributionReplayLoader::load_verified_epoch_tail_with_connection(
                            connection,
                            &replay_request,
                            scoped.fills,
                            fees,
                            scoped.epoch,
                        )
                        .map_err(EpochReplaySnapshotFailure::Load)
                    })
                    .map_err(map_epoch_replay_transaction_error);
                match snapshot {
                    Ok(evidence) => Ok(evidence),
                    Err(EpochReplaySnapshotFailure::Load(failure)) => Err(failure),
                    Err(EpochReplaySnapshotFailure::Epoch(error)) => {
                        return Err(Box::new(PrepareFailure {
                            error: map_epoch_store_error(error),
                            invocation,
                            evidence: summary,
                        }));
                    }
                    Err(EpochReplaySnapshotFailure::Replay(error)) => {
                        return Err(Box::new(PrepareFailure {
                            error,
                            invocation,
                            evidence: summary,
                        }));
                    }
                }
            }
        };
        let evidence = match loaded {
            Ok(evidence) => evidence,
            Err(failure) => {
                let error = map_current_session_evidence_error(
                    admitted,
                    &calendar,
                    map_attribution_load_failure(&failure),
                );
                summary.record_load_failure(&failure, error.failure_fingerprint);
                return Err(Box::new(PrepareFailure {
                    error,
                    invocation,
                    evidence: summary,
                }));
            }
        };
        summary.record_replay_evidence(&evidence);
        if let Err(error) = validate_fill_calendar_authority(&evidence) {
            let error = map_current_session_evidence_error(admitted, &calendar, error);
            return Err(Box::new(PrepareFailure {
                error,
                invocation,
                evidence: summary,
            }));
        }
        let (benchmark_bars, benchmark_manifest_hash, benchmark_day_manifests) =
            match load_runner_benchmarks(
                self.database,
                &self.benchmark_instrument,
                &evidence,
                &calendar,
                &admitted.benchmark_day_manifests,
                &mut summary,
            ) {
                Ok(loaded) => loaded,
                Err(error) => {
                    let error = map_current_session_evidence_error(admitted, &calendar, error);
                    return Err(Box::new(PrepareFailure {
                        error,
                        invocation,
                        evidence: summary,
                    }));
                }
            };
        let report =
            match compute_attribution_range(&evidence, &benchmark_bars, &self.minute_semantics) {
                Ok(report) => report,
                Err(error) => {
                    return Err(Box::new(PrepareFailure {
                        error: map_attribution_error(ReplayStage::Compute, error),
                        invocation,
                        evidence: summary,
                    }));
                }
            };
        let canonical_result_bytes = match canonical_attribution_report_bytes(&report) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(Box::new(PrepareFailure {
                    error: map_attribution_error(ReplayStage::Compute, error),
                    invocation,
                    evidence: summary,
                }));
            }
        };
        let core_result_payload: serde_json::Value =
            match serde_json::from_slice(&canonical_result_bytes) {
                Ok(payload) => payload,
                Err(_) => {
                    return Err(Box::new(PrepareFailure {
                        error: ReplayError::new(
                            ReplayErrorClass::FailedIntegrity,
                            ReplayStage::Compute,
                            "canonical_attribution_report_failed",
                            false,
                        ),
                        invocation,
                        evidence: summary,
                    }));
                }
            };
        let result_payload = serde_json::json!({
            "benchmark_day_manifests": &benchmark_day_manifests,
            "core_report": core_result_payload,
        });
        let fee = match runner_fee_evidence_hash(report.fee_basis()) {
            Ok(fee) => fee,
            Err(error) => {
                return Err(Box::new(PrepareFailure {
                    error,
                    invocation,
                    evidence: summary,
                }));
            }
        };
        Ok(PreparedAttributionReport {
            invocation,
            report,
            canonical_result_bytes,
            result_payload,
            trade_manifest_hash: evidence.trade_manifest_hash().to_owned(),
            fee,
            stock_close_manifest_hash: evidence.stock_closes().manifest_hash().to_owned(),
            benchmark_manifest_hash,
            benchmark_day_manifests,
            calendar_authority_hash: calendar.authority_hash().to_owned(),
        })
    }
}

fn replay_invoked_at(mode: &ReplayMode) -> DateTime<FixedOffset> {
    match mode {
        ReplayMode::Scheduled { invoked_at }
        | ReplayMode::Range { invoked_at, .. }
        | ReplayMode::Quarter { invoked_at, .. } => *invoked_at,
    }
}

fn admit_replay_request(request: ReplayRequest) -> Result<AdmittedReplayRequest, ReplayError> {
    let invoked_at = replay_invoked_at(&request.mode);
    if invoked_at.offset().local_minus_utc() != 8 * 60 * 60 {
        return Err(ReplayError::new(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::Request,
            "invalid_invocation_timezone",
            false,
        ));
    }
    if request
        .benchmark_day_manifests
        .iter()
        .any(|binding| !is_lowercase_sha256(&binding.manifest_hash))
    {
        return Err(ReplayError::new(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::Request,
            "invalid_benchmark_manifest_hash",
            false,
        ));
    }
    if matches!(
        &request.epoch,
        AttributionEpochSelector::Exact(epoch_id) if !is_lowercase_sha256(epoch_id)
    ) {
        return Err(ReplayError::new(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::Request,
            "invalid_attribution_epoch_selector",
            false,
        ));
    }
    let (mode, target_from, target_to) = match &request.mode {
        ReplayMode::Scheduled { invoked_at } => (
            AttributionRunMode::Scheduled,
            invoked_at.date_naive(),
            invoked_at.date_naive(),
        ),
        ReplayMode::Range { from, to, .. } if from <= to => (AttributionRunMode::Range, *from, *to),
        ReplayMode::Range { .. } => {
            return Err(ReplayError::new(
                ReplayErrorClass::FailedIntegrity,
                ReplayStage::Request,
                "invalid_replay_range",
                false,
            ));
        }
        ReplayMode::Quarter { year, quarter, .. } => {
            let (from, to) =
                verified_replay_quarter_bounds(*year, *quarter).map_err(map_calendar_error)?;
            (AttributionRunMode::Quarter, from, to)
        }
    };
    Ok(AdmittedReplayRequest {
        mode: request.mode,
        epoch: request.epoch,
        provisional_invocation: AttributionInvocation {
            mode,
            target_from,
            target_to,
            rule_version: ATTRIBUTION_REPLAY_RULE_VERSION.to_owned(),
            invoked_at,
        },
        benchmark_day_manifests: request.benchmark_day_manifests,
    })
}

fn resolve_admitted_calendar(
    admitted: &AdmittedReplayRequest,
) -> Result<VerifiedReplayCalendar, VerifiedCalendarError> {
    match &admitted.mode {
        ReplayMode::Scheduled { invoked_at } => resolve_verified_scheduled_replay(*invoked_at),
        ReplayMode::Range { from, to, .. } => resolve_verified_replay_range(*from, *to),
        ReplayMode::Quarter { year, quarter, .. } => {
            resolve_verified_replay_quarter(*year, *quarter)
        }
    }
}

fn validate_fill_calendar_authority(
    evidence: &AttributionReplayEvidence,
) -> Result<(), ReplayError> {
    for fill in evidence.fills() {
        let paper_date = parse_paper_fill_timestamp(fill.fill().id, &fill.fill().occurred_at)
            .map_err(|detail| {
                ReplayError::new(
                    ReplayErrorClass::FailedIntegrity,
                    ReplayStage::Calendar,
                    "fill_timestamp_invalid",
                    false,
                )
                .with_typed_failure(
                    "paper_fill_timestamp",
                    detail.as_bytes(),
                    None,
                    None,
                )
            })?
            .date();
        let terminal_date = fill.terminal_time().date_naive();
        for date in [paper_date, terminal_date] {
            match verified_a_share_trading_day(date) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(ReplayError::new(
                        ReplayErrorClass::FailedIntegrity,
                        ReplayStage::Calendar,
                        "fill_non_trading_day",
                        false,
                    )
                    .with_failure_date(date));
                }
                Err(detail) => {
                    return Err(ReplayError::new(
                        ReplayErrorClass::FailedIntegrity,
                        ReplayStage::Calendar,
                        "fill_calendar_authority_failed",
                        false,
                    )
                    .with_typed_failure(
                        "fill_calendar_authority",
                        detail.as_bytes(),
                        Some(date),
                        None,
                    ));
                }
            }
        }
        if paper_date != terminal_date {
            let detail = format!("{paper_date}|{terminal_date}");
            return Err(ReplayError::new(
                ReplayErrorClass::FailedIntegrity,
                ReplayStage::Calendar,
                "fill_terminal_date_mismatch",
                false,
            )
            .with_typed_failure(
                "fill_terminal_date_mismatch",
                detail.as_bytes(),
                Some(paper_date),
                None,
            ));
        }
    }
    Ok(())
}

fn runner_benchmark_request(
    instrument: &str,
    trading_date: NaiveDate,
) -> Result<BenchmarkRequest, ReplayError> {
    let offset = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
        ReplayError::new(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::Benchmark,
            "benchmark_timezone_unavailable",
            false,
        )
    })?;
    let from = offset
        .from_local_datetime(&trading_date.and_hms_opt(9, 31, 0).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorClass::FailedIntegrity,
                ReplayStage::Benchmark,
                "benchmark_range_invalid",
                false,
            )
        })?)
        .single()
        .ok_or_else(|| {
            ReplayError::new(
                ReplayErrorClass::FailedIntegrity,
                ReplayStage::Benchmark,
                "benchmark_range_invalid",
                false,
            )
        })?;
    let to = offset
        .from_local_datetime(&trading_date.and_hms_opt(15, 0, 0).ok_or_else(|| {
            ReplayError::new(
                ReplayErrorClass::FailedIntegrity,
                ReplayStage::Benchmark,
                "benchmark_range_invalid",
                false,
            )
        })?)
        .single()
        .ok_or_else(|| {
            ReplayError::new(
                ReplayErrorClass::FailedIntegrity,
                ReplayStage::Benchmark,
                "benchmark_range_invalid",
                false,
            )
        })?;
    Ok(BenchmarkRequest {
        instrument: instrument.to_owned(),
        range: BenchmarkRange::Minute1 { from, to },
    })
}

fn load_runner_benchmarks(
    database: &DatabaseManager,
    instrument: &str,
    evidence: &AttributionReplayEvidence,
    calendar: &VerifiedReplayCalendar,
    supplied: &[BenchmarkDayManifest],
    summary: &mut FailureEvidenceSummary,
) -> Result<(Vec<BenchmarkBar>, String, Vec<BenchmarkDayManifest>), ReplayError> {
    if supplied
        .windows(2)
        .any(|pair| pair[0].trading_date >= pair[1].trading_date)
    {
        return Err(ReplayError::new(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::Benchmark,
            "benchmark_day_manifests_not_strictly_ordered",
            false,
        ));
    }
    let mut required = calendar
        .required_trading_dates()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    required.extend(
        evidence
            .fills()
            .iter()
            .map(|fill| fill.terminal_time().date_naive()),
    );
    let supplied_dates = supplied
        .iter()
        .map(|binding| binding.trading_date)
        .collect::<BTreeSet<_>>();
    if supplied_dates.iter().any(|date| !required.contains(date)) {
        return Err(ReplayError::new(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::Benchmark,
            "benchmark_day_manifest_extra",
            false,
        ));
    }
    if let Some(missing_date) = required.iter().find(|date| !supplied_dates.contains(date)) {
        let error = ReplayError::new(
            ReplayErrorClass::Unavailable,
            ReplayStage::Benchmark,
            "benchmark_day_manifest_unavailable",
            true,
        )
        .with_failure_date(*missing_date)
        .with_evidence_failure_kind(ReplayEvidenceFailureKind::BenchmarkExactAbsent);
        summary.record_benchmark_unavailable(*missing_date, error.failure_fingerprint);
        return Err(error);
    }
    let reader = BenchmarkReader::new(database);
    let mut bars = Vec::new();
    for binding in supplied {
        let expected = runner_benchmark_request(instrument, binding.trading_date)?;
        let snapshot = match reader.read_exact(&binding.manifest_hash, &expected) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let error = map_benchmark_error(error)
                    .with_benchmark_failure_context(binding.trading_date, &binding.manifest_hash);
                summary
                    .record_benchmark_unavailable(binding.trading_date, error.failure_fingerprint);
                return Err(error);
            }
        };
        summary.record_benchmark_available(binding.trading_date, &binding.manifest_hash);
        bars.extend(snapshot.bars);
    }
    Ok((
        bars,
        requested_benchmark_binding_hash(supplied),
        supplied.to_vec(),
    ))
}

fn requested_benchmark_binding_hash(bindings: &[BenchmarkDayManifest]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(RUNNER_BENCHMARK_MANIFEST_DOMAIN);
    hasher.update((bindings.len() as u64).to_be_bytes());
    for binding in bindings {
        update_len_prefixed(&mut hasher, binding.trading_date.to_string().as_bytes());
        update_len_prefixed(&mut hasher, binding.manifest_hash.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn runner_fee_evidence_hash(
    fee: &MetricAvailability<AttributionFeeBasis>,
) -> Result<AttributionEvidenceHash, ReplayError> {
    match fee {
        MetricAvailability::Unavailable { code, .. } => {
            Ok(AttributionEvidenceHash::Unavailable(code.code().to_owned()))
        }
        MetricAvailability::Available(basis) => {
            if !is_lowercase_sha256(&basis.basis_id) {
                return Err(ReplayError::new(
                    ReplayErrorClass::FailedIntegrity,
                    ReplayStage::Compute,
                    "fee_basis_identity_failed",
                    false,
                ));
            }
            Ok(AttributionEvidenceHash::Available(basis.basis_id.clone()))
        }
    }
}

fn map_calendar_error(error: VerifiedCalendarError) -> ReplayError {
    let kind = error.kind();
    let class = match kind {
        VerifiedCalendarErrorKind::CurrentSessionIncomplete
        | VerifiedCalendarErrorKind::TradingCalendarUnavailable => ReplayErrorClass::Unavailable,
        VerifiedCalendarErrorKind::InvalidRequest => ReplayErrorClass::FailedIntegrity,
    };
    ReplayError::new(
        class,
        ReplayStage::Calendar,
        error.code(),
        error.retryable(),
    )
    .with_typed_failure(
        match kind {
            VerifiedCalendarErrorKind::InvalidRequest => "calendar_invalid_request",
            VerifiedCalendarErrorKind::CurrentSessionIncomplete => {
                "calendar_current_session_incomplete"
            }
            VerifiedCalendarErrorKind::TradingCalendarUnavailable => {
                "calendar_authority_unavailable"
            }
        },
        error.code().as_bytes(),
        None,
        None,
    )
}

fn map_attribution_error(stage: ReplayStage, error: AttributionReplayError) -> ReplayError {
    match error {
        AttributionReplayError::Unavailable {
            code,
            retryable,
            detail,
        } => ReplayError::new(ReplayErrorClass::Unavailable, stage, code.code(), retryable)
            .with_typed_failure(code.code(), detail.as_bytes(), None, None),
        AttributionReplayError::FailedIntegrity { code, detail } => {
            ReplayError::new(ReplayErrorClass::FailedIntegrity, stage, code.code(), false)
                .with_typed_failure(code.code(), detail.as_bytes(), None, None)
        }
    }
}

fn map_epoch_store_error(error: AttributionEpochStoreError) -> ReplayError {
    match error {
        AttributionEpochStoreError::Unavailable {
            reason_code,
            retryable,
            detail,
        } => ReplayError::new(
            ReplayErrorClass::Unavailable,
            ReplayStage::Epoch,
            reason_code,
            retryable,
        )
        .with_typed_failure(reason_code, detail.as_bytes(), None, None),
        AttributionEpochStoreError::FailedIntegrity {
            reason_code,
            detail,
        } => ReplayError::new(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::Epoch,
            reason_code,
            false,
        )
        .with_typed_failure(reason_code, detail.as_bytes(), None, None),
    }
}

fn map_attribution_load_failure(failure: &AttributionReplayLoadFailure) -> ReplayError {
    let mut mapped = match &failure.error {
        AttributionReplayError::Unavailable {
            code,
            retryable,
            detail,
        } => ReplayError::new(
            ReplayErrorClass::Unavailable,
            ReplayStage::TradeEvidence,
            code.code(),
            *retryable,
        )
        .with_typed_failure(code.code(), detail.as_bytes(), failure.failure_date, None),
        AttributionReplayError::FailedIntegrity { code, detail } => ReplayError::new(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::TradeEvidence,
            code.code(),
            false,
        )
        .with_typed_failure(code.code(), detail.as_bytes(), failure.failure_date, None),
    };
    if failure.stage == AttributionReplayLoadStage::StockClose
        && matches!(
            failure.error,
            AttributionReplayError::Unavailable {
                code: AttributionUnavailable::StockCloseUnavailable,
                ..
            }
        )
    {
        mapped = mapped.with_evidence_failure_kind(ReplayEvidenceFailureKind::StockCloseAbsent);
    }
    mapped
}

fn map_benchmark_error(error: BenchmarkError) -> ReplayError {
    match error {
        BenchmarkError::Unavailable { code, retryable } => {
            let mapped = ReplayError::new(
                ReplayErrorClass::Unavailable,
                ReplayStage::Benchmark,
                code,
                retryable,
            );
            if code == "benchmark_manifest_unavailable" {
                mapped.with_evidence_failure_kind(ReplayEvidenceFailureKind::BenchmarkExactAbsent)
            } else {
                mapped
            }
        }
        BenchmarkError::FailedIntegrity { code } => ReplayError::new(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::Benchmark,
            code,
            false,
        ),
        BenchmarkError::Unsupported(BenchmarkUnsupported::UnsupportedInstrument) => {
            ReplayError::new(
                ReplayErrorClass::FailedIntegrity,
                ReplayStage::Benchmark,
                "benchmark_instrument_unsupported",
                false,
            )
        }
        BenchmarkError::Unsupported(BenchmarkUnsupported::TestIdentityRejected) => {
            ReplayError::new(
                ReplayErrorClass::FailedIntegrity,
                ReplayStage::Benchmark,
                "benchmark_test_identity_rejected",
                false,
            )
        }
    }
}

fn map_store_error(error: AttributionReportStoreError) -> ReplayError {
    ReplayError::new(
        ReplayErrorClass::Storage,
        ReplayStage::Store,
        error.reason_code(),
        error.retryable(),
    )
}

fn map_current_session_evidence_error(
    admitted: &AdmittedReplayRequest,
    calendar: &VerifiedReplayCalendar,
    error: ReplayError,
) -> ReplayError {
    let is_actual_current_session = matches!(admitted.mode, ReplayMode::Scheduled { .. })
        && calendar.target_to() == replay_invoked_at(&admitted.mode).date_naive();
    let is_current_session_evidence = error.failure_date == Some(calendar.target_to())
        && matches!(
            error.evidence_failure_kind,
            Some(
                ReplayEvidenceFailureKind::StockCloseAbsent
                    | ReplayEvidenceFailureKind::BenchmarkExactAbsent
            )
        );
    if is_actual_current_session
        && error.class == ReplayErrorClass::Unavailable
        && is_current_session_evidence
    {
        error.into_current_session_incomplete()
    } else {
        error
    }
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

fn validate_epoch_report_projection(
    epoch: &AttributionEpochReplayEvidence,
) -> Result<(), AttributionReplayError> {
    let legacy = matches!(epoch.selector, AttributionEpochSelector::Legacy);
    if legacy {
        if epoch.epoch_id.is_some()
            || epoch.receipt_hash.is_some()
            || epoch.effective_date.is_some()
            || epoch.legacy_carry_manifest_hash.is_some()
            || epoch.exclusion_manifest_hash.is_some()
            || !epoch.remaining_quarantine.is_empty()
            || epoch.released_codes != 0
            || !epoch.excluded_fills.is_empty()
            || epoch.overlap_buy_count != 0
            || epoch.overlap_sell_count != 0
            || epoch.mixed_exit_count != 0
            || epoch.excluded_fill_count != 0
        {
            return Err(canonical_report_error(
                "legacy report fabricated attribution epoch evidence",
            ));
        }
        return Ok(());
    }
    let (
        Some(epoch_id),
        Some(receipt_hash),
        Some(_effective_date),
        Some(carry_hash),
        Some(exclusion_hash),
    ) = (
        epoch.epoch_id.as_deref(),
        epoch.receipt_hash.as_deref(),
        epoch.effective_date,
        epoch.legacy_carry_manifest_hash.as_deref(),
        epoch.exclusion_manifest_hash.as_deref(),
    )
    else {
        return Err(canonical_report_error(
            "resolved epoch report is missing receipt/carry/exclusion evidence",
        ));
    };
    if !is_lowercase_sha256(epoch_id)
        || !is_lowercase_sha256(receipt_hash)
        || !is_lowercase_sha256(carry_hash)
        || !is_lowercase_sha256(exclusion_hash)
        || !epoch
            .verified_filled_manifest_hash
            .as_deref()
            .is_some_and(is_lowercase_sha256)
        || !epoch
            .verified_terminal_binding_manifest_hash
            .as_deref()
            .is_some_and(is_lowercase_sha256)
        || !epoch
            .verified_order_audit_tip_hash
            .as_deref()
            .is_some_and(is_lowercase_sha256)
        || matches!(
            &epoch.selector,
            AttributionEpochSelector::Exact(expected) if expected != epoch_id
        )
        || matches!(epoch.selector, AttributionEpochSelector::Legacy)
    {
        return Err(canonical_report_error(
            "resolved epoch report identities are inconsistent",
        ));
    }
    if epoch.remaining_quarantine.iter().any(|position| {
        position.code.trim().is_empty()
            || position.quantity == 0
            || !position.quantity.is_multiple_of(100)
    }) || epoch
        .remaining_quarantine
        .windows(2)
        .any(|pair| pair[0].code >= pair[1].code)
    {
        return Err(canonical_report_error(
            "resolved epoch remaining quarantine is noncanonical",
        ));
    }
    let overlap_buy_count = epoch
        .excluded_fills
        .iter()
        .filter(|item| {
            item.reason == EpochExclusionReason::LegacyCarryOverlap && item.direction == "buy"
        })
        .count();
    let overlap_sell_count = epoch
        .excluded_fills
        .iter()
        .filter(|item| {
            item.reason == EpochExclusionReason::LegacyCarryOverlap && item.direction == "sell"
        })
        .count();
    let mixed_exit_count = epoch
        .excluded_fills
        .iter()
        .filter(|item| item.reason == EpochExclusionReason::MixedLegacyCarryExit)
        .count();
    let excluded_fill_count = epoch
        .excluded_fills
        .iter()
        .map(|item| item.fill_id)
        .collect::<BTreeSet<_>>()
        .len();
    if epoch.excluded_fills.iter().any(|item| {
        item.fill_id <= 0
            || item.code.trim().is_empty()
            || !matches!(item.direction.as_str(), "buy" | "sell")
            || item.quantity == 0
            || !item.quantity.is_multiple_of(100)
    }) || epoch.overlap_buy_count != overlap_buy_count
        || epoch.overlap_sell_count != overlap_sell_count
        || epoch.mixed_exit_count != mixed_exit_count
        || epoch.excluded_fill_count != excluded_fill_count
    {
        return Err(canonical_report_error(
            "resolved epoch exclusion statistics are inconsistent",
        ));
    }
    Ok(())
}

fn validate_and_normalize_attribution_report_projection(
    report: &AttributionComputationReport,
) -> Result<AttributionComputationReport, AttributionReplayError> {
    let mut report = report.clone();
    validate_epoch_report_projection(&report.epoch)?;
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

    use chrono::{DateTime, Duration, FixedOffset, NaiveDate, TimeZone, Timelike};
    use diesel::connection::SimpleConnection;
    use rusqlite::{params, Connection};

    use super::*;
    use crate::data_gateway::review::AuditedBenchmarkBatch;
    use crate::data_gateway::{
        BatchEvidence, BenchmarkBar, BenchmarkBarTime, BenchmarkCapture, BenchmarkRange,
        BenchmarkRequest, GatewayBatch,
    };
    use crate::database::attribution_epochs::{AttributionEpochStore, EpochActivationRequest};
    use crate::database::attribution_reports::{
        AttributionDatabaseAccess, AttributionDatabaseSession,
    };
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
    use crate::database::order_audit::{
        canonical_order_audit_record_hash, CanonicalOrderAuditRow, AUDIT_CHAIN_GENESIS,
    };
    use crate::magic_compat::ProviderId;

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

    fn create_epoch_replay_schema(connection: &Connection) {
        connection
            .execute_batch(
                "CREATE TABLE paper_trades (
                    id INTEGER PRIMARY KEY, plan_id TEXT NOT NULL UNIQUE,
                    code TEXT NOT NULL, name TEXT NOT NULL, direction TEXT NOT NULL,
                    price REAL NOT NULL, quantity INTEGER NOT NULL, status TEXT NOT NULL,
                    fill_price REAL, not_fill_reason TEXT, virtual_reason TEXT NOT NULL,
                    account_mode TEXT NOT NULL, data_mode TEXT NOT NULL,
                    ts TEXT NOT NULL, updated_at TEXT NOT NULL
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
                    record_hash TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL
                 );
                 CREATE TABLE stock_daily (
                    id INTEGER PRIMARY KEY, code TEXT NOT NULL, date TEXT NOT NULL,
                    close REAL, data_source TEXT, created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );",
            )
            .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn append_epoch_source_fill(
        path: &Path,
        id: i64,
        side: &str,
        price: f64,
        quantity: i64,
        paper_utc: &str,
        quote_shanghai: &str,
        reason: &str,
    ) {
        let connection = Connection::open(path).unwrap();
        let previous_hash = connection
            .query_row(
                "SELECT record_hash FROM order_audit_chain ORDER BY order_audit_id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_else(|_| AUDIT_CHAIN_GENESIS.to_owned());
        let plan_id = format!("TEST_CODE_EPOCH_PLAN_{id}");
        connection
            .execute(
                "INSERT INTO paper_trades
                 (id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,
                  virtual_reason,account_mode,data_mode,ts,updated_at)
                 VALUES (?1,?2,'TEST_CODE_600001','TEST_CODE epoch company',?3,?4,?5,
                         'Filled',?4,NULL,?6,'Normal','Full',?7,?7)",
                params![id, plan_id, side, price, quantity, reason, paper_utc],
            )
            .unwrap();
        let quote = DateTime::parse_from_rfc3339(quote_shanghai).unwrap();
        let created_at = (quote.with_timezone(&chrono::Utc) + Duration::seconds(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let audit = CanonicalOrderAuditRow {
            id,
            business_order_id: plan_id,
            source: "PaperTrade".to_owned(),
            decision_basis: reason.to_owned(),
            side: side.to_owned(),
            code: "TEST_CODE_600001".to_owned(),
            requested_price: price,
            execution_price: Some(price),
            quantity,
            quote_observed_at: Some(quote_shanghai.to_owned()),
            outcome: "Filled".to_owned(),
            failure_reason: None,
            created_at: created_at.clone(),
        };
        connection
            .execute(
                "INSERT INTO order_audit VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    audit.id,
                    audit.business_order_id,
                    audit.source,
                    audit.decision_basis,
                    audit.side,
                    audit.code,
                    audit.requested_price,
                    audit.execution_price,
                    audit.quantity,
                    audit.quote_observed_at,
                    audit.outcome,
                    audit.failure_reason,
                    audit.created_at,
                ],
            )
            .unwrap();
        let record_hash = canonical_order_audit_record_hash(&previous_hash, &audit).unwrap();
        connection
            .execute(
                "INSERT INTO order_audit_chain VALUES (?1,?2,?3,?4)",
                params![id, previous_hash, record_hash, created_at],
            )
            .unwrap();
    }

    fn activated_epoch_database(
        label: &str,
    ) -> (PathBuf, AttributionDatabaseSession, AttributionEpochReceipt) {
        let path = test_database_path(label);
        create_epoch_replay_schema(&Connection::open(&path).unwrap());
        append_epoch_source_fill(
            &path,
            1,
            "buy",
            10.0,
            100,
            "2026-08-20 01:31:05",
            "2026-08-20T09:31:05+08:00",
            "Breakout",
        );
        append_epoch_source_fill(
            &path,
            2,
            "sell",
            11.0,
            100,
            "2026-08-21 06:20:00",
            "2026-08-21T14:20:00+08:00",
            "ExitByRule",
        );
        let session =
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::AppendOnly).unwrap();
        let receipt = AttributionEpochStore::new(session.database())
            .activate_once(EpochActivationRequest {
                source: crate::performance::attribution_epoch::EpochActivationSource::Cli,
                invoked_at: shanghai_at("2026-08-21", 15, 40, 0),
            })
            .unwrap();
        (path, session, receipt)
    }

    fn activated_carry_epoch_database(
        label: &str,
    ) -> (PathBuf, AttributionDatabaseSession, AttributionEpochReceipt) {
        let path = test_database_path(label);
        create_epoch_replay_schema(&Connection::open(&path).unwrap());
        append_epoch_source_fill(
            &path,
            1,
            "buy",
            10.0,
            200,
            "2026-08-20 01:31:05",
            "2026-08-20T09:31:05+08:00",
            "Breakout",
        );
        append_epoch_source_fill(
            &path,
            2,
            "sell",
            11.0,
            100,
            "2026-08-21 06:20:00",
            "2026-08-21T14:20:00+08:00",
            "ExitByRule",
        );
        let session =
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::AppendOnly).unwrap();
        let receipt = AttributionEpochStore::new(session.database())
            .activate_once(EpochActivationRequest {
                source: crate::performance::attribution_epoch::EpochActivationSource::Cli,
                invoked_at: shanghai_at("2026-08-21", 15, 40, 0),
            })
            .unwrap();
        (path, session, receipt)
    }

    fn activated_real_legacy_t_plus_one_epoch_database(
        label: &str,
    ) -> (PathBuf, AttributionDatabaseSession, AttributionEpochReceipt) {
        let path = test_database_path(label);
        create_epoch_replay_schema(&Connection::open(&path).unwrap());
        append_epoch_source_fill(
            &path,
            510,
            "buy",
            10.0,
            400,
            "2026-08-28 02:00:00",
            "2026-08-28T10:00:00+08:00",
            "TEST_CODE legacy buy",
        );
        append_epoch_source_fill(
            &path,
            520,
            "sell",
            11.0,
            100,
            "2026-08-28 06:00:00",
            "2026-08-28T14:00:00+08:00",
            "TEST_CODE legacy same-day sell",
        );
        let session =
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::AppendOnly).unwrap();
        let receipt = AttributionEpochStore::new(session.database())
            .activate_once(EpochActivationRequest {
                source: crate::performance::attribution_epoch::EpochActivationSource::Cli,
                invoked_at: shanghai_at("2026-08-28", 15, 40, 0),
            })
            .unwrap();
        assert_eq!(receipt.paper_trade_high_water, 520);
        assert_eq!(receipt.carry_total_quantity, 300);
        (path, session, receipt)
    }

    fn append_epoch_close(path: &Path, id: i64, day: &str, close: f64) {
        Connection::open(path)
            .unwrap()
            .execute(
                "INSERT INTO stock_daily
                 VALUES (?1,'TEST_CODE_600001',?2,?3,'TEST_CODE_SOURCE',?2,?2)",
                params![id, day, close],
            )
            .unwrap();
    }

    // TEST_CODE fixture mirrors the immutable paper-trade and audit-chain columns.
    #[allow(clippy::too_many_arguments)]
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

    fn shanghai_at(day: &str, hour: u32, minute: u32, second: u32) -> DateTime<FixedOffset> {
        let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
        FixedOffset::east_opt(8 * 60 * 60)
            .unwrap()
            .from_local_datetime(&date.and_hms_opt(hour, minute, second).unwrap())
            .single()
            .unwrap()
    }

    fn runner_benchmark_request(instrument: &str, trading_date: NaiveDate) -> BenchmarkRequest {
        BenchmarkRequest {
            instrument: instrument.to_owned(),
            range: BenchmarkRange::Minute1 {
                from: FixedOffset::east_opt(8 * 60 * 60)
                    .unwrap()
                    .from_local_datetime(&trading_date.and_hms_opt(9, 31, 0).unwrap())
                    .single()
                    .unwrap(),
                to: FixedOffset::east_opt(8 * 60 * 60)
                    .unwrap()
                    .from_local_datetime(&trading_date.and_hms_opt(15, 0, 0).unwrap())
                    .single()
                    .unwrap(),
            },
        }
    }

    fn complete_minute_bars(request: &BenchmarkRequest, price_shift: f64) -> Vec<BenchmarkBar> {
        let BenchmarkRange::Minute1 { from, to } = request.range else {
            panic!("TEST_CODE minute request expected");
        };
        let mut bars = Vec::new();
        let mut cursor = from;
        while cursor <= to {
            let minute = cursor.hour() * 60 + cursor.minute();
            if (9 * 60 + 31..=11 * 60 + 30).contains(&minute)
                || (13 * 60 + 1..=15 * 60).contains(&minute)
            {
                let close = 3_500.0 + price_shift + bars.len() as f64 / 100.0;
                bars.push(BenchmarkBar {
                    at: BenchmarkBarTime::MinuteEnd(cursor),
                    open: close,
                    high: close + 1.0,
                    low: close - 1.0,
                    close,
                    volume: None,
                    amount: None,
                });
            }
            cursor += Duration::minutes(1);
        }
        bars
    }

    fn append_test_benchmark_manifest(
        manager: &crate::database::DatabaseManager,
        trading_date: NaiveDate,
        price_shift: f64,
    ) -> BenchmarkDayManifest {
        let request = runner_benchmark_request("TEST_CODE_000300", trading_date);
        let bars = complete_minute_bars(&request, price_shift);
        let request_hash = request.canonical_request_hash();
        let evidence = BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_magic-tdx-index-bars@task31".to_owned(),
            source_at: Some("2026-08-21T15:00:01+08:00".to_owned()),
            observed_at: "2026-08-21T15:00:02+08:00".to_owned(),
            batch_id: format!("TEST_CODE_task31_batch_{price_shift}"),
        };
        let receipt = manager
            .record_data_acquisition(&DataAcquisitionAuditRecord {
                capability: "BenchmarkBars",
                provider: "Tdx",
                source: &evidence.source,
                request_hash: &request_hash,
                source_at: evidence.source_at.as_deref(),
                observed_at: &evidence.observed_at,
                batch_id: Some(&evidence.batch_id),
                outcome: "available",
                request_count: 1,
                accepted_count: i64::try_from(bars.len()).unwrap(),
                rejected_count: 0,
                reason_code: "accepted",
                retryable: false,
            })
            .expect("TEST_CODE acquisition audit");
        let preview = BenchmarkCapture::new(manager)
            .preview_audited_for_test(
                request,
                AuditedBenchmarkBatch {
                    batch: GatewayBatch::Available {
                        records: bars,
                        evidence,
                    },
                    receipt,
                    request_hash,
                },
            )
            .expect("TEST_CODE benchmark capture preview");
        let manifest_hash = BenchmarkCapture::new(manager)
            .commit(preview)
            .expect("TEST_CODE benchmark capture commit")
            .manifest_hash;
        BenchmarkDayManifest {
            trading_date,
            manifest_hash,
        }
    }

    fn attribution_table_counts(path: &Path) -> Vec<i64> {
        let connection = Connection::open(path).unwrap();
        [
            "attribution_run_audit",
            "attribution_run_chain",
            "attribution_report_revision",
            "attribution_report_chain",
            "attribution_report_epoch_binding",
            "attribution_report_epoch_binding_chain",
            "attribution_failure_audit",
            "attribution_failure_chain",
        ]
        .into_iter()
        .map(|table| {
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        })
        .collect()
    }

    fn runner_readonly_counts(path: &Path) -> Vec<i64> {
        let connection = Connection::open(path).unwrap();
        [
            "paper_trades",
            "order_audit",
            "order_audit_chain",
            "stock_daily",
            "data_acquisition_audit",
            "data_acquisition_audit_chain",
            "benchmark_segment_revision",
            "benchmark_segment_chain",
            "benchmark_manifest",
            "benchmark_manifest_acquisition",
            "benchmark_manifest_chain",
            "attribution_run_audit",
            "attribution_run_chain",
            "attribution_report_revision",
            "attribution_report_chain",
            "attribution_failure_audit",
            "attribution_failure_chain",
        ]
        .into_iter()
        .map(|table| {
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        })
        .collect()
    }

    fn sqlite_object_stats(path: &Path) -> Vec<Option<(u64, u64, u64, std::time::SystemTime)>> {
        [
            path.to_path_buf(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ]
        .into_iter()
        .map(|candidate| {
            candidate.metadata().ok().map(|metadata| {
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.len(),
                    metadata.modified().unwrap(),
                )
            })
        })
        .collect()
    }

    fn scheduled_request(
        invoked_at: DateTime<FixedOffset>,
        benchmark_day_manifests: Vec<BenchmarkDayManifest>,
    ) -> ReplayRequest {
        ReplayRequest {
            mode: ReplayMode::Scheduled { invoked_at },
            epoch: AttributionEpochSelector::Legacy,
            benchmark_day_manifests,
        }
    }

    #[test]
    fn epoch_active_missing_fails_at_epoch_before_legacy_trade_evidence() {
        let path = complete_database("epoch_active_missing");
        Connection::open(&path)
            .unwrap()
            .execute("DROP TABLE paper_trades", [])
            .unwrap();
        let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
        let runner = AttributionReplayRunner::new_for_test(
            &manager,
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );

        let error = runner
            .preview(ReplayRequest {
                mode: ReplayMode::Scheduled {
                    invoked_at: shanghai_at("2026-08-21", 15, 30, 0),
                },
                epoch: AttributionEpochSelector::Active,
                benchmark_day_manifests: Vec::new(),
            })
            .expect_err("TEST_CODE active epoch must not fall back to legacy replay");

        assert_eq!(error.stage(), ReplayStage::Epoch);
        assert_eq!(error.code(), "attribution_epoch_unavailable");
        assert_eq!(ReplayStage::Epoch.as_str(), "epoch");
        drop(runner);
        drop(manager);
        remove_database(path);
    }

    #[test]
    fn public_runner_requires_database_authority_for_active_and_exact_but_not_legacy() {
        let path = complete_database("public_runner_no_attribution_authority");
        let manager = crate::database::attribution_reports::
            test_runner_database_manager_without_attribution_read_authority(&path);
        let benchmark_day_manifests = [date("2026-08-20"), date("2026-08-21")]
            .into_iter()
            .enumerate()
            .map(|(index, day)| append_test_benchmark_manifest(&manager, day, index as f64))
            .collect::<Vec<_>>();
        let request = |epoch| ReplayRequest {
            mode: ReplayMode::Range {
                from: date("2026-08-20"),
                to: date("2026-08-21"),
                invoked_at: shanghai_at("2026-08-21", 15, 30, 0),
            },
            epoch,
            benchmark_day_manifests: benchmark_day_manifests.clone(),
        };

        let legacy_runner = AttributionReplayRunner::new_for_test(
            &manager,
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let legacy = legacy_runner
            .preview(request(AttributionEpochSelector::Legacy))
            .expect("TEST_CODE Legacy remains available through its loader without DB authority");
        assert_eq!(legacy.report().source_fill_ids(), &[1, 2]);
        drop(legacy_runner);

        let runner = AttributionReplayRunner::new(&manager, AttributionReplayLoader::new(&path));
        for selector in [
            AttributionEpochSelector::Active,
            AttributionEpochSelector::Exact("f".repeat(64)),
        ] {
            let error = runner
                .preview(request(selector))
                .expect_err("TEST_CODE Active/Exact require manager-owned database authority");
            assert_eq!(error.class(), ReplayErrorClass::Unavailable);
            assert_eq!(error.stage(), ReplayStage::Epoch);
            assert_eq!(error.code(), "attribution_database_authority_unavailable");
            assert!(!error.retryable());
        }

        drop(runner);
        drop(manager);
        remove_database(path);
    }

    #[test]
    fn epoch_legacy_explicitly_preserves_truthful_t_plus_one_failure() {
        let path = test_database_path("epoch_legacy_t_plus_one");
        create_epoch_replay_schema(&Connection::open(&path).unwrap());
        // Keep the production regression identifiers and ordering intact: id
        // 520 sells the id 510 lot on the same Shanghai trading day.
        append_epoch_source_fill(
            &path,
            510,
            "buy",
            10.0,
            400,
            "2026-08-28 02:00:00",
            "2026-08-28T10:00:00+08:00",
            "TEST_CODE legacy buy",
        );
        append_epoch_source_fill(
            &path,
            520,
            "sell",
            11.0,
            100,
            "2026-08-28 06:00:00",
            "2026-08-28T14:00:00+08:00",
            "TEST_CODE legacy same-day sell",
        );
        let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
        let runner = AttributionReplayRunner::new_for_test(
            &manager,
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );

        let error = runner
            .preview(ReplayRequest {
                mode: ReplayMode::Range {
                    from: date("2026-08-28"),
                    to: date("2026-08-28"),
                    invoked_at: shanghai_at("2026-08-28", 15, 30, 0),
                },
                epoch: AttributionEpochSelector::Legacy,
                benchmark_day_manifests: Vec::new(),
            })
            .expect_err("TEST_CODE legacy replay must preserve the old T+1 failure");

        assert_eq!(error.stage(), ReplayStage::TradeEvidence);
        assert_eq!(error.code(), "paper_trade_source_failed");
        drop(runner);
        drop(manager);
        remove_database(path);
    }

    #[test]
    fn real_legacy_t_plus_one_fixture_replays_only_post_flat_cycle_and_seals_report_epoch() {
        let (path, session, receipt) =
            activated_real_legacy_t_plus_one_epoch_database("real_legacy_t_plus_one_e2e");
        for (id, side, price, quantity, paper_utc, quote_shanghai, reason) in [
            (
                530,
                "buy",
                12.0,
                200,
                "2026-08-31 02:00:00",
                "2026-08-31T10:00:00+08:00",
                "TEST_CODE carry overlap buy",
            ),
            (
                540,
                "sell",
                12.5,
                400,
                "2026-09-01 02:00:00",
                "2026-09-01T10:00:00+08:00",
                "TEST_CODE mixed carry exit",
            ),
            (
                550,
                "sell",
                12.8,
                100,
                "2026-09-01 06:00:00",
                "2026-09-01T14:00:00+08:00",
                "TEST_CODE terminal carry exit",
            ),
            (
                560,
                "buy",
                20.0,
                100,
                "2026-09-02 02:00:00",
                "2026-09-02T10:00:00+08:00",
                "Breakout",
            ),
            (
                570,
                "sell",
                22.0,
                100,
                "2026-09-03 02:00:00",
                "2026-09-03T10:00:00+08:00",
                "ExitByRule",
            ),
        ] {
            append_epoch_source_fill(
                &path,
                id,
                side,
                price,
                quantity,
                paper_utc,
                quote_shanghai,
                reason,
            );
        }
        for (id, day, close) in [
            (1, "2026-08-31", 12.0),
            (2, "2026-09-01", 12.5),
            (3, "2026-09-02", 20.0),
            (4, "2026-09-03", 22.0),
        ] {
            append_epoch_close(&path, id, day, close);
        }
        let benchmark_day_manifests = [
            date("2026-08-31"),
            date("2026-09-01"),
            date("2026-09-02"),
            date("2026-09-03"),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, day)| append_test_benchmark_manifest(session.database(), day, index as f64))
        .collect::<Vec<_>>();
        let runner = AttributionReplayRunner::new_for_test_with_fee_ledger(
            session.database(),
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
            AuthoritativeFillFeeLedger {
                entries: vec![fee(560, 5.0), fee(570, 5.0)],
            },
        );
        let request = |epoch| ReplayRequest {
            mode: ReplayMode::Range {
                from: date("2026-08-31"),
                to: date("2026-09-03"),
                invoked_at: shanghai_at("2026-09-03", 15, 30, 0),
            },
            epoch,
            benchmark_day_manifests: benchmark_day_manifests.clone(),
        };

        let prepared = runner
            .preview(request(AttributionEpochSelector::Active))
            .expect("TEST_CODE active replay quarantines the legacy same-day defect");
        assert_eq!(prepared.epoch_id(), Some(receipt.epoch_id.as_str()));
        assert!(prepared.remaining_quarantine().is_empty());
        assert_eq!(prepared.released_codes(), 1);
        assert_eq!(prepared.overlap_buy_count(), 1);
        assert_eq!(prepared.overlap_sell_count(), 2);
        assert_eq!(prepared.mixed_exit_count(), 1);
        assert_eq!(prepared.report().source_fill_ids(), &[560, 570]);
        assert_eq!(prepared.report().total_closed_cycles(), 1);
        assert_eq!(prepared.report().coverage_days(), Some(2));
        assert!(matches!(
            prepared.report().conclusion(),
            AttributionConclusion::InsufficientSample { reasons, .. }
                if reasons.iter().any(|reason| reason.contains("closed_cycles_1_below_200"))
                    && reasons.iter().any(|reason| reason.contains("coverage_days_2_below_84"))
        ));
        let committed = runner
            .commit_with_report(request(AttributionEpochSelector::Active))
            .expect("TEST_CODE active report must commit through its sealed epoch binding");
        assert!(matches!(
            committed.receipt().epoch,
            AttributionReportEpochBinding::Epoch { ref epoch_id, .. } if epoch_id == &receipt.epoch_id
        ));
        let exact = runner
            .preview(request(AttributionEpochSelector::Exact(
                receipt.epoch_id.clone(),
            )))
            .expect("TEST_CODE exact epoch remains the same sealed receipt");
        assert_eq!(exact.report().source_fill_ids(), &[560, 570]);
        drop(runner);
        drop(session);
        remove_database(path);
    }

    #[test]
    fn active_and_exact_replay_never_mix_epoch_fills_with_loader_database_closes() {
        let (epoch_path, session, receipt) =
            activated_epoch_database("epoch_single_snapshot_manager");
        append_epoch_source_fill(
            &epoch_path,
            3,
            "buy",
            12.0,
            100,
            "2026-08-24 01:31:05",
            "2026-08-24T09:31:05+08:00",
            "Breakout",
        );
        append_epoch_source_fill(
            &epoch_path,
            4,
            "sell",
            13.0,
            100,
            "2026-08-25 06:20:00",
            "2026-08-25T14:20:00+08:00",
            "ExitByRule",
        );
        let benchmark_day_manifests = [date("2026-08-24"), date("2026-08-25")]
            .into_iter()
            .enumerate()
            .map(|(index, day)| {
                append_test_benchmark_manifest(session.database(), day, index as f64)
            })
            .collect::<Vec<_>>();

        let loader_path = test_database_path("epoch_single_snapshot_loader");
        create_epoch_replay_schema(&Connection::open(&loader_path).unwrap());
        append_epoch_close(&loader_path, 1, "2026-08-24", 12.0);
        append_epoch_close(&loader_path, 2, "2026-08-25", 13.0);
        let runner = AttributionReplayRunner::new_for_test(
            session.database(),
            AttributionReplayLoader::new(&loader_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let request = |epoch| ReplayRequest {
            mode: ReplayMode::Range {
                from: date("2026-08-24"),
                to: date("2026-08-25"),
                invoked_at: shanghai_at("2026-08-25", 15, 30, 0),
            },
            epoch,
            benchmark_day_manifests: benchmark_day_manifests.clone(),
        };

        for selector in [
            AttributionEpochSelector::Active,
            AttributionEpochSelector::Exact(receipt.epoch_id.clone()),
        ] {
            let error = runner
                .preview(request(selector))
                .expect_err("TEST_CODE epoch replay must not read closes from the loader database");
            assert_eq!(error.class(), ReplayErrorClass::Unavailable);
            assert_eq!(error.stage(), ReplayStage::TradeEvidence);
            assert_eq!(error.code(), "stock_close_unavailable");
        }

        drop(runner);
        drop(session);
        remove_database(epoch_path);
        remove_database(loader_path);
    }

    #[test]
    fn exact_stock_close_reader_accepts_empty_required_keys_without_source_access() {
        let mut connection = <SqliteConnection as diesel::Connection>::establish(":memory:")
            .expect("TEST_CODE establish empty exact close database");

        let rows = load_stock_closes_with_connection(&mut connection, &BTreeSet::new())
            .expect("TEST_CODE empty exact close keys require no stock_daily source");

        assert!(rows.is_empty());
    }

    #[test]
    fn exact_stock_close_reader_chunks_401_keys_and_preserves_global_order() {
        let mut connection = <SqliteConnection as diesel::Connection>::establish(":memory:")
            .expect("TEST_CODE establish chunked exact close database");
        connection
            .batch_execute(
                "CREATE TABLE stock_daily (
                    id INTEGER PRIMARY KEY, code TEXT NOT NULL, date TEXT NOT NULL,
                    close REAL, data_source TEXT, created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );",
            )
            .expect("TEST_CODE create chunked stock_daily source");
        let day = date("2026-08-21");
        let required_keys = (0..401)
            .rev()
            .map(|index| {
                let code = format!("TEST_CODE_{index:03}");
                diesel::sql_query(
                    "INSERT INTO stock_daily
                     (id,code,date,close,data_source,created_at,updated_at)
                     VALUES (?,?,?,?,?,?,?)",
                )
                .bind::<BigInt, _>(i64::from(index) + 1)
                .bind::<Text, _>(code.clone())
                .bind::<Text, _>(day.format("%Y-%m-%d").to_string())
                .bind::<Double, _>(100.0 + f64::from(index))
                .bind::<Text, _>("TEST_CODE_SOURCE")
                .bind::<Text, _>("2026-08-21")
                .bind::<Text, _>("2026-08-21")
                .execute(&mut connection)
                .expect("TEST_CODE insert chunked stock close");
                (code, day)
            })
            .collect::<BTreeSet<_>>();

        let rows = load_stock_closes_with_connection(&mut connection, &required_keys)
            .expect("TEST_CODE 401 exact close keys span two deterministic chunks");
        let observed = rows
            .iter()
            .map(|row| (row.code.clone(), row.date.clone(), row.id))
            .collect::<Vec<_>>();
        let expected = required_keys
            .iter()
            .enumerate()
            .map(|(index, (code, day))| {
                (
                    code.clone(),
                    day.format("%Y-%m-%d").to_string(),
                    index as i64 + 1,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(observed, expected);
    }

    #[test]
    fn active_and_exact_replay_fail_closed_for_frozen_and_post_boundary_source_tamper() {
        for case in [
            "frozen_prefix",
            "late_old_date",
            "terminal_at_or_below_highwater",
            "post_boundary_audit_chain",
        ] {
            let (path, session, receipt) =
                activated_real_legacy_t_plus_one_epoch_database(&format!("{case}_replay_tamper"));
            match case {
                "frozen_prefix" => {
                    Connection::open(&path)
                        .unwrap()
                        .execute("UPDATE paper_trades SET fill_price=10.5 WHERE id=510", [])
                        .unwrap();
                }
                "late_old_date" => append_epoch_source_fill(
                    &path,
                    530,
                    "buy",
                    12.0,
                    100,
                    "2026-08-30 02:00:00",
                    "2026-08-30T10:00:00+08:00",
                    "TEST_CODE late pre-effective fill",
                ),
                "terminal_at_or_below_highwater" => {
                    append_epoch_source_fill(
                        &path,
                        530,
                        "buy",
                        12.0,
                        100,
                        "2026-08-31 02:00:00",
                        "2026-08-31T10:00:00+08:00",
                        "TEST_CODE low terminal id",
                    );
                    let connection = Connection::open(&path).unwrap();
                    connection
                        .execute("UPDATE order_audit SET id=515 WHERE id=530", [])
                        .unwrap();
                }
                "post_boundary_audit_chain" => {
                    append_epoch_source_fill(
                        &path,
                        530,
                        "buy",
                        12.0,
                        100,
                        "2026-08-31 02:00:00",
                        "2026-08-31T10:00:00+08:00",
                        "TEST_CODE corrupted terminal chain",
                    );
                    let connection = Connection::open(&path).unwrap();
                    connection
                        .execute(
                            "UPDATE order_audit_chain SET record_hash=?1 WHERE order_audit_id=530",
                            ["f".repeat(64)],
                        )
                        .unwrap();
                }
                _ => unreachable!("TEST_CODE fixed replay tamper case"),
            }
            let runner = AttributionReplayRunner::new_for_test(
                session.database(),
                AttributionReplayLoader::new(&path),
                "TEST_CODE_000300",
                "TEST_CODE_MINUTE_END_LABEL",
            );
            for epoch in [
                AttributionEpochSelector::Active,
                AttributionEpochSelector::Exact(receipt.epoch_id.clone()),
            ] {
                let error = runner
                    .preview(ReplayRequest {
                        mode: ReplayMode::Range {
                            from: date("2026-08-31"),
                            to: date("2026-08-31"),
                            invoked_at: shanghai_at("2026-08-31", 15, 30, 0),
                        },
                        epoch,
                        benchmark_day_manifests: Vec::new(),
                    })
                    .expect_err("TEST_CODE source tamper cannot return an Active/Exact report");
                assert!(
                    matches!(
                        error.stage(),
                        ReplayStage::Epoch | ReplayStage::TradeEvidence
                    ),
                    "TEST_CODE {case} must fail before benchmark/compute/store: {error:?}"
                );
            }
            drop(runner);
            drop(session);
            remove_database(path);
        }
    }

    #[test]
    fn epoch_exact_unknown_is_typed_unavailable_after_retained_state_validation() {
        let (path, session, _) = activated_epoch_database("epoch_exact_unknown");
        let runner = AttributionReplayRunner::new_for_test(
            session.database(),
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );

        let error = runner
            .preview(ReplayRequest {
                mode: ReplayMode::Range {
                    from: date("2026-08-24"),
                    to: date("2026-08-24"),
                    invoked_at: shanghai_at("2026-08-24", 15, 30, 0),
                },
                epoch: AttributionEpochSelector::Exact("f".repeat(64)),
                benchmark_day_manifests: Vec::new(),
            })
            .expect_err("TEST_CODE unknown exact epoch must be unavailable");

        assert_eq!(error.stage(), ReplayStage::Epoch);
        assert_eq!(error.code(), "attribution_epoch_unavailable");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "DROP TRIGGER trg_attribution_sample_epoch_receipt_chain_no_update",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE attribution_sample_epoch_receipt_chain
                 SET record_hash=?1 WHERE id=1",
                ["e".repeat(64)],
            )
            .unwrap();
        drop(connection);
        let integrity = runner
            .preview(ReplayRequest {
                mode: ReplayMode::Range {
                    from: date("2026-08-24"),
                    to: date("2026-08-24"),
                    invoked_at: shanghai_at("2026-08-24", 15, 30, 0),
                },
                epoch: AttributionEpochSelector::Exact("d".repeat(64)),
                benchmark_day_manifests: Vec::new(),
            })
            .expect_err("TEST_CODE bad retained state must precede exact absence");
        assert_eq!(integrity.stage(), ReplayStage::Epoch);
        assert_eq!(integrity.code(), "attribution_epoch_integrity_failed");
        drop(runner);
        drop(session);
        remove_database(path);
    }

    #[test]
    fn epoch_range_before_effective_is_rejected_without_clipping() {
        let (path, session, _) = activated_epoch_database("epoch_range_before_effective");
        let runner = AttributionReplayRunner::new_for_test(
            session.database(),
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );

        let error = runner
            .preview(ReplayRequest {
                mode: ReplayMode::Range {
                    from: date("2026-08-21"),
                    to: date("2026-08-24"),
                    invoked_at: shanghai_at("2026-08-24", 15, 30, 0),
                },
                epoch: AttributionEpochSelector::Active,
                benchmark_day_manifests: Vec::new(),
            })
            .expect_err("TEST_CODE epoch range must not be silently clipped");

        assert_eq!(error.stage(), ReplayStage::Epoch);
        assert_eq!(error.code(), "attribution_epoch_range_before_effective");
        drop(runner);
        drop(session);
        remove_database(path);
    }

    #[test]
    fn epoch_failure_summary_binds_selector_resolved_receipt_carry_and_exclusions() {
        let (path, session, receipt) = activated_epoch_database("epoch_failure_summary");
        let mode = ReplayMode::Range {
            from: date("2026-08-24"),
            to: date("2026-08-24"),
            invoked_at: shanghai_at("2026-08-24", 15, 30, 0),
        };
        let legacy = admit_replay_request(ReplayRequest {
            mode: mode.clone(),
            epoch: AttributionEpochSelector::Legacy,
            benchmark_day_manifests: Vec::new(),
        })
        .unwrap();
        let active = admit_replay_request(ReplayRequest {
            mode,
            epoch: AttributionEpochSelector::Active,
            benchmark_day_manifests: Vec::new(),
        })
        .unwrap();
        let error = ReplayError::new(
            ReplayErrorClass::Unavailable,
            ReplayStage::Epoch,
            "attribution_epoch_unavailable",
            false,
        );
        let legacy_hash = FailureEvidenceSummary::new(&legacy).source_summary_hash(&error);
        let mut unresolved = FailureEvidenceSummary::new(&active);
        let unresolved_hash = unresolved.source_summary_hash(&error);
        assert_ne!(legacy_hash, unresolved_hash);

        unresolved.record_resolved_epoch(&ResolvedAttributionEpoch::Epoch(receipt.clone()));
        let resolved_hash = unresolved.source_summary_hash(&error);
        assert_ne!(unresolved_hash, resolved_hash);
        let epoch = AttributionEpochReplayEvidence::resolved(
            AttributionEpochSelector::Active,
            &receipt,
            ResolvedAttributionEpochReplayScope {
                exclusions: Vec::new(),
                exclusion_manifest_hash: canonical_exclusion_manifest_hash(&[], &[]).unwrap(),
                remaining_quarantine: Vec::new(),
                released_codes: 0,
                scoped_fill_manifest_hash: canonical_scoped_fill_manifest_hash(&[]).unwrap(),
                verified_filled_manifest_hash: "1".repeat(64),
                verified_terminal_binding_manifest_hash: "2".repeat(64),
                verified_order_audit_tip_hash: "3".repeat(64),
            },
        );
        unresolved.record_epoch_evidence(&epoch);
        assert_ne!(resolved_hash, unresolved.source_summary_hash(&error));
        drop(session);
        remove_database(path);
    }

    #[test]
    fn epoch_quarantine_excludes_complete_overlap_until_flat_and_projects_fee_denominator() {
        let (path, session, receipt) = activated_carry_epoch_database("epoch_quarantine_and_fees");
        append_epoch_source_fill(
            &path,
            3,
            "buy",
            12.0,
            100,
            "2026-08-24 01:31:05",
            "2026-08-24T09:31:05+08:00",
            "Momentum",
        );
        append_epoch_source_fill(
            &path,
            4,
            "sell",
            12.5,
            200,
            "2026-08-25 01:32:05",
            "2026-08-25T09:32:05+08:00",
            "ExitByRule",
        );
        append_epoch_source_fill(
            &path,
            5,
            "buy",
            20.0,
            100,
            "2026-08-25 02:00:05",
            "2026-08-25T10:00:05+08:00",
            "Breakout",
        );
        append_epoch_source_fill(
            &path,
            6,
            "sell",
            22.0,
            100,
            "2026-08-26 02:00:05",
            "2026-08-26T10:00:05+08:00",
            "ExitByRule",
        );
        for (id, day, close) in [
            (1, "2026-08-24", 12.1),
            (2, "2026-08-25", 20.2),
            (3, "2026-08-26", 22.1),
        ] {
            append_epoch_close(&path, id, day, close);
        }
        let benchmark_day_manifests = vec![
            append_test_benchmark_manifest(session.database(), date("2026-08-24"), 0.0),
            append_test_benchmark_manifest(session.database(), date("2026-08-25"), 10.0),
            append_test_benchmark_manifest(session.database(), date("2026-08-26"), 20.0),
        ];
        let quarantine_runner = AttributionReplayRunner::new_for_test(
            session.database(),
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let still_quarantined = quarantine_runner
            .preview(ReplayRequest {
                mode: ReplayMode::Range {
                    from: date("2026-08-24"),
                    to: date("2026-08-24"),
                    invoked_at: shanghai_at("2026-08-24", 15, 30, 0),
                },
                epoch: AttributionEpochSelector::Active,
                benchmark_day_manifests: vec![benchmark_day_manifests[0].clone()],
            })
            .expect("TEST_CODE unresolved carry remains quarantined");
        assert_eq!(
            still_quarantined.remaining_quarantine(),
            &[LegacyCarryPosition {
                code: "TEST_CODE_600001".to_owned(),
                quantity: 200,
            }]
        );
        assert_eq!(still_quarantined.released_codes(), 0);
        assert_eq!(still_quarantined.excluded_fill_count(), 1);
        assert!(still_quarantined.report().source_fill_ids().is_empty());
        drop(quarantine_runner);
        let runner = AttributionReplayRunner::new_for_test_with_fee_ledger(
            session.database(),
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
            AuthoritativeFillFeeLedger {
                entries: vec![fee(3, 90.0), fee(4, 90.0), fee(5, 5.0), fee(6, 5.0)],
            },
        );

        let active_request = ReplayRequest {
            mode: ReplayMode::Range {
                from: date("2026-08-24"),
                to: date("2026-08-26"),
                invoked_at: shanghai_at("2026-08-26", 15, 30, 0),
            },
            epoch: AttributionEpochSelector::Active,
            benchmark_day_manifests: benchmark_day_manifests.clone(),
        };
        let prepared = runner
            .preview(active_request.clone())
            .expect("TEST_CODE active epoch replay");

        assert_eq!(prepared.epoch_selector(), &AttributionEpochSelector::Active);
        assert_eq!(prepared.epoch_id(), Some(receipt.epoch_id.as_str()));
        assert_eq!(
            prepared.epoch_receipt_hash(),
            Some(receipt.receipt_hash.as_str())
        );
        assert_eq!(prepared.epoch_effective_date(), Some(date("2026-08-24")));
        assert_eq!(prepared.remaining_quarantine(), &[]);
        assert_eq!(prepared.released_codes(), 1);
        assert_eq!(prepared.overlap_buy_count(), 1);
        assert_eq!(prepared.overlap_sell_count(), 1);
        assert_eq!(prepared.mixed_exit_count(), 1);
        assert_eq!(prepared.excluded_fill_count(), 2);
        assert_eq!(prepared.excluded_fills().len(), 3);
        assert_eq!(prepared.report().source_fill_ids(), &[5, 6]);
        assert_eq!(prepared.report().total_closed_cycles(), 1);
        assert!(matches!(
            prepared.report().conclusion(),
            AttributionConclusion::InsufficientSample { reasons, .. }
                if reasons.iter().any(|reason| reason.contains("closed_cycles_1_below_200"))
        ));
        let MetricAvailability::Available(basis) = prepared.report().fee_basis() else {
            panic!("TEST_CODE attributable fee basis must remain complete");
        };
        assert_eq!(
            basis
                .bindings
                .iter()
                .map(|binding| binding.fill_id)
                .collect::<Vec<_>>(),
            vec![5, 6]
        );
        let payload: serde_json::Value =
            serde_json::from_slice(prepared.canonical_result_bytes()).unwrap();
        assert_eq!(payload["epoch"]["selector"], "active");
        assert_eq!(payload["epoch"]["excluded_fill_count"], 2);
        let expected_binding = AttributionReportEpochBinding::Epoch {
            epoch_id: receipt.epoch_id.clone(),
            epoch_receipt_hash: receipt.receipt_hash.clone(),
            effective_date: receipt.effective_trading_date,
            legacy_carry_manifest_hash: receipt.legacy_carry_manifest_hash.clone(),
            exclusion_manifest_hash: prepared.exclusion_manifest_hash().unwrap().to_owned(),
        };
        assert_eq!(prepared.report_epoch_binding().unwrap(), expected_binding);
        let mut missing_binding = prepared.clone();
        missing_binding.report.epoch.receipt_hash = None;
        let missing_error = missing_binding
            .report_epoch_binding()
            .expect_err("TEST_CODE active report cannot omit epoch receipt binding");
        assert_eq!(missing_error.stage(), ReplayStage::Epoch);
        assert_eq!(
            missing_error.code(),
            "attribution_report_epoch_binding_missing"
        );
        let committed = runner
            .commit_with_report(active_request)
            .expect("TEST_CODE active epoch report commit");
        assert_eq!(committed.receipt().epoch, expected_binding);
        assert_eq!(committed.receipt().report_revision, 1);
        let exact_request = ReplayRequest {
            mode: ReplayMode::Range {
                from: date("2026-08-24"),
                to: date("2026-08-26"),
                invoked_at: shanghai_at("2026-08-26", 15, 30, 0),
            },
            epoch: AttributionEpochSelector::Exact(receipt.epoch_id.clone()),
            benchmark_day_manifests,
        };
        let exact = runner
            .preview(exact_request.clone())
            .expect("TEST_CODE retained exact epoch replay");
        assert_eq!(
            exact.epoch_selector(),
            &AttributionEpochSelector::Exact(receipt.epoch_id.clone())
        );
        assert_eq!(exact.report().source_fill_ids(), &[5, 6]);
        assert_ne!(prepared.trade_manifest_hash(), exact.trade_manifest_hash());
        let exact_binding = exact
            .report_epoch_binding()
            .expect("TEST_CODE sealed Exact report binding");
        assert!(matches!(
            &exact_binding,
            AttributionReportEpochBinding::Epoch { .. }
        ));
        let exact_committed = runner
            .commit_with_report(exact_request)
            .expect("TEST_CODE Exact epoch report commit");
        assert_eq!(exact_committed.receipt().epoch, exact_binding);
        drop(runner);
        drop(session);
        remove_database(path);
    }

    #[test]
    fn epoch_fee_failure_preserves_scoped_progress_and_audits_trade_evidence() {
        let (path, session, receipt) = activated_carry_epoch_database("epoch_fee_failure_progress");
        append_epoch_source_fill(
            &path,
            3,
            "buy",
            12.0,
            100,
            "2026-08-24 01:31:05",
            "2026-08-24T09:31:05+08:00",
            "Momentum",
        );
        append_epoch_source_fill(
            &path,
            4,
            "sell",
            12.5,
            200,
            "2026-08-25 01:32:05",
            "2026-08-25T09:32:05+08:00",
            "ExitByRule",
        );
        append_epoch_source_fill(
            &path,
            5,
            "buy",
            20.0,
            100,
            "2026-08-25 02:00:05",
            "2026-08-25T10:00:05+08:00",
            "Breakout",
        );
        append_epoch_source_fill(
            &path,
            6,
            "sell",
            22.0,
            100,
            "2026-08-26 02:00:05",
            "2026-08-26T10:00:05+08:00",
            "ExitByRule",
        );
        let runner = AttributionReplayRunner::new_for_test_with_fee_ledger(
            session.database(),
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
            AuthoritativeFillFeeLedger {
                entries: vec![fee(3, 90.0), fee(4, 90.0)],
            },
        );
        let request = |selector| ReplayRequest {
            mode: ReplayMode::Range {
                from: date("2026-08-24"),
                to: date("2026-08-26"),
                invoked_at: shanghai_at("2026-08-26", 15, 30, 0),
            },
            epoch: selector,
            benchmark_day_manifests: Vec::new(),
        };
        let expected_fee_fingerprint = runner_failure_leaf_fingerprint(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::TradeEvidence,
            "fee_evidence_failed",
            false,
            b"fee evidence is missing attributable fill ids [5, 6]",
            None,
            None,
        );
        let mut selector_summaries = Vec::new();

        for selector in [
            AttributionEpochSelector::Active,
            AttributionEpochSelector::Exact(receipt.epoch_id.clone()),
        ] {
            let admitted = admit_replay_request(request(selector.clone())).unwrap();
            let mut summaries = BTreeSet::new();
            for _ in 0..32 {
                let failure = runner
                    .prepare(&admitted)
                    .expect_err("TEST_CODE incomplete attributable fees must fail preparation");
                assert_eq!(failure.error.stage(), ReplayStage::TradeEvidence);
                assert_eq!(failure.error.code(), "fee_evidence_failed");
                assert_eq!(failure.error.failure_fingerprint, expected_fee_fingerprint);
                assert_eq!(
                    failure.evidence.epoch_id,
                    FailureEvidenceState::Available(receipt.epoch_id.clone())
                );
                assert_eq!(
                    failure.evidence.epoch_receipt,
                    FailureEvidenceState::Available(receipt.receipt_hash.clone())
                );
                assert_eq!(
                    failure.evidence.legacy_carry,
                    FailureEvidenceState::Available(receipt.legacy_carry_manifest_hash.clone())
                );
                assert!(matches!(
                    failure.evidence.exclusions,
                    FailureEvidenceState::Available(_)
                ));
                assert_eq!(
                    failure.evidence.remaining_quarantine,
                    FailureEvidenceState::Available(canonical_legacy_carry_manifest_hash(&[]))
                );
                assert_eq!(
                    failure.evidence.released_codes,
                    FailureEvidenceState::Available("1".to_owned())
                );
                assert!(matches!(
                    failure.evidence.trade,
                    FailureEvidenceState::Available(_)
                ));
                assert_eq!(failure.evidence.stock_close, FailureEvidenceState::Unknown);
                assert_eq!(
                    failure.evidence.fee,
                    FailureEvidenceState::Unavailable(expected_fee_fingerprint)
                );
                summaries.insert(failure.evidence.source_summary_hash(&failure.error));
            }
            assert_eq!(summaries.len(), 1);
            let expected_summary = summaries.into_iter().next().unwrap();
            let error = runner
                .commit(request(selector))
                .expect_err("TEST_CODE epoch fee failure must be audited");
            assert_eq!(error.stage(), ReplayStage::TradeEvidence);
            let failure_id = error.failure_receipt().unwrap().failure_audit_id;
            let retained: (String, String, String) = Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT stage,code,source_summary_hash
                     FROM attribution_failure_audit WHERE id=?1",
                    [failure_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(retained.0, "trade_evidence");
            assert_eq!(retained.1, "fee_evidence_failed");
            assert_eq!(retained.2, expected_summary);
            selector_summaries.push(expected_summary);
        }
        assert_ne!(selector_summaries[0], selector_summaries[1]);
        drop(runner);
        drop(session);
        remove_database(path);
    }

    #[test]
    fn epoch_duplicate_unknown_and_malformed_fees_fail_at_trade_evidence() {
        let (path, session, _) = activated_epoch_database("epoch_fee_integrity_stage");
        append_epoch_source_fill(
            &path,
            3,
            "buy",
            20.0,
            100,
            "2026-08-24 02:00:05",
            "2026-08-24T10:00:05+08:00",
            "Breakout",
        );
        append_epoch_source_fill(
            &path,
            4,
            "sell",
            22.0,
            100,
            "2026-08-25 02:00:05",
            "2026-08-25T10:00:05+08:00",
            "ExitByRule",
        );
        let mut malformed = fee(3, 1.0);
        malformed.evidence_hash = "f".repeat(64);
        let cases = [
            (
                "duplicate",
                AuthoritativeFillFeeLedger {
                    entries: vec![fee(3, 1.0), fee(3, 1.0), fee(4, 1.0)],
                },
            ),
            (
                "unknown",
                AuthoritativeFillFeeLedger {
                    entries: vec![fee(3, 1.0), fee(4, 1.0), fee(999, 1.0)],
                },
            ),
            (
                "malformed",
                AuthoritativeFillFeeLedger {
                    entries: vec![malformed, fee(4, 1.0)],
                },
            ),
        ];

        for (label, ledger) in cases {
            let runner = AttributionReplayRunner::new_for_test_with_fee_ledger(
                session.database(),
                AttributionReplayLoader::new(&path),
                "TEST_CODE_000300",
                "TEST_CODE_MINUTE_END_LABEL",
                ledger,
            );
            let error = runner
                .preview(ReplayRequest {
                    mode: ReplayMode::Range {
                        from: date("2026-08-24"),
                        to: date("2026-08-25"),
                        invoked_at: shanghai_at("2026-08-25", 15, 30, 0),
                    },
                    epoch: AttributionEpochSelector::Active,
                    benchmark_day_manifests: Vec::new(),
                })
                .unwrap_err();
            assert_eq!(error.stage(), ReplayStage::TradeEvidence, "{label}");
            assert_eq!(error.code(), "fee_evidence_failed", "{label}");
        }
        drop(session);
        remove_database(path);
    }

    #[test]
    fn legacy_multi_missing_fee_ids_have_one_canonical_failure_identity() {
        let path = complete_database("legacy_multi_missing_fee_identity");
        let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
        let runner = AttributionReplayRunner::new_for_test_with_fee_ledger(
            &manager,
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
            AuthoritativeFillFeeLedger {
                entries: Vec::new(),
            },
        );
        let request = || scheduled_request(shanghai_at("2026-08-21", 15, 30, 0), Vec::new());
        let admitted = admit_replay_request(request()).unwrap();
        let expected_fee_fingerprint = runner_failure_leaf_fingerprint(
            ReplayErrorClass::FailedIntegrity,
            ReplayStage::TradeEvidence,
            "fee_evidence_failed",
            false,
            b"fee evidence is missing fill ids [1, 2]",
            None,
            None,
        );
        let mut summaries = BTreeSet::new();
        for _ in 0..32 {
            let failure = runner
                .prepare(&admitted)
                .expect_err("TEST_CODE multiple legacy fees are missing");
            assert_eq!(failure.error.stage(), ReplayStage::TradeEvidence);
            assert_eq!(failure.error.failure_fingerprint, expected_fee_fingerprint);
            assert!(matches!(
                failure.evidence.trade,
                FailureEvidenceState::Available(_)
            ));
            assert!(matches!(
                failure.evidence.stock_close,
                FailureEvidenceState::Available(_)
            ));
            assert_eq!(
                failure.evidence.fee,
                FailureEvidenceState::Unavailable(expected_fee_fingerprint)
            );
            summaries.insert(failure.evidence.source_summary_hash(&failure.error));
        }
        assert_eq!(summaries.len(), 1);
        let expected_summary = summaries.into_iter().next().unwrap();
        let error = runner
            .commit(request())
            .expect_err("TEST_CODE canonical legacy fee failure must be audited");
        let failure_id = error.failure_receipt().unwrap().failure_audit_id;
        let retained: String = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT source_summary_hash FROM attribution_failure_audit WHERE id=?1",
                [failure_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, expected_summary);
        drop(runner);
        drop(manager);
        remove_database(path);
    }

    #[test]
    fn runner_preview_uses_one_pipeline_and_is_byte_identical_for_scheduled_and_same_day_range() {
        let path = complete_database("runner_preview");
        let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
        let benchmark_day_manifests = vec![
            append_test_benchmark_manifest(&manager, date("2026-08-20"), 0.0),
            append_test_benchmark_manifest(&manager, date("2026-08-21"), 10.0),
        ];
        let runner = AttributionReplayRunner::new_for_test(
            &manager,
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let before_counts = runner_readonly_counts(&path);
        let before_objects = sqlite_object_stats(&path);
        let scheduled = runner
            .preview(ReplayRequest {
                mode: ReplayMode::Scheduled {
                    invoked_at: shanghai_at("2026-08-21", 15, 30, 0),
                },
                epoch: AttributionEpochSelector::Legacy,
                benchmark_day_manifests: benchmark_day_manifests.clone(),
            })
            .expect("TEST_CODE scheduled preview");
        let range = runner
            .preview(ReplayRequest {
                mode: ReplayMode::Range {
                    from: date("2026-08-21"),
                    to: date("2026-08-21"),
                    invoked_at: shanghai_at("2026-08-22", 15, 30, 0),
                },
                epoch: AttributionEpochSelector::Legacy,
                benchmark_day_manifests,
            })
            .expect("TEST_CODE same-day range preview");
        assert_eq!(
            scheduled.canonical_result_bytes(),
            range.canonical_result_bytes()
        );
        assert_eq!(scheduled.report().total_closed_cycles(), 1);
        assert_eq!(
            scheduled.epoch_selector(),
            &AttributionEpochSelector::Legacy
        );
        assert_eq!(scheduled.epoch_id(), None);
        assert_eq!(scheduled.epoch_receipt_hash(), None);
        assert_eq!(scheduled.legacy_carry_manifest_hash(), None);
        assert_eq!(scheduled.exclusion_manifest_hash(), None);
        assert_eq!(scheduled.trade_manifest_hash().len(), 64);
        assert_eq!(scheduled.calendar_authority_hash().len(), 64);
        assert_eq!(runner_readonly_counts(&path), before_counts);
        assert_eq!(sqlite_object_stats(&path), before_objects);
        drop(runner);
        drop(manager);
        remove_database(path);
    }

    #[test]
    fn runner_failures_are_typed_audited_and_preview_never_writes_or_falls_back() {
        let path = complete_database("runner_failures");
        let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
        let bindings = vec![
            append_test_benchmark_manifest(&manager, date("2026-08-20"), 0.0),
            append_test_benchmark_manifest(&manager, date("2026-08-21"), 10.0),
        ];
        let runner = AttributionReplayRunner::new_for_test(
            &manager,
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );

        let before_counts = runner_readonly_counts(&path);
        let before_objects = sqlite_object_stats(&path);
        let current_missing = runner
            .preview(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                vec![bindings[0].clone()],
            ))
            .expect_err("TEST_CODE current-day missing evidence");
        assert_eq!(current_missing.class(), ReplayErrorClass::Unavailable);
        assert_eq!(current_missing.stage(), ReplayStage::Benchmark);
        assert_eq!(current_missing.code(), "current_session_incomplete");
        assert!(current_missing.retryable());
        assert!(current_missing.failure_receipt().is_none());
        assert_eq!(runner_readonly_counts(&path), before_counts);
        assert_eq!(sqlite_object_stats(&path), before_objects);

        let current_committed = runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                vec![bindings[0].clone()],
            ))
            .expect_err("TEST_CODE formal current-day failure");
        assert_eq!(current_committed.code(), "current_session_incomplete");
        assert!(current_committed.failure_receipt().is_some());
        assert_eq!(
            attribution_table_counts(&path),
            vec![1, 1, 0, 0, 0, 0, 1, 1]
        );

        let weekend_committed = runner
            .commit(scheduled_request(
                shanghai_at("2026-08-22", 15, 30, 0),
                vec![bindings[0].clone()],
            ))
            .expect_err("TEST_CODE weekend must retain exact unavailable");
        assert_eq!(weekend_committed.class(), ReplayErrorClass::Unavailable);
        assert_eq!(
            weekend_committed.code(),
            "benchmark_day_manifest_unavailable"
        );
        assert!(weekend_committed.failure_receipt().is_some());
        assert_eq!(
            attribution_table_counts(&path),
            vec![2, 2, 0, 0, 0, 0, 2, 2]
        );

        let before_preview_failures = runner_readonly_counts(&path);
        let before_preview_objects = sqlite_object_stats(&path);
        let duplicate = runner
            .preview(scheduled_request(
                shanghai_at("2026-08-22", 15, 30, 0),
                vec![
                    bindings[0].clone(),
                    bindings[0].clone(),
                    bindings[1].clone(),
                ],
            ))
            .expect_err("TEST_CODE duplicate day binding");
        assert_eq!(duplicate.class(), ReplayErrorClass::FailedIntegrity);
        assert_eq!(
            duplicate.code(),
            "benchmark_day_manifests_not_strictly_ordered"
        );

        let mut extra = bindings.clone();
        extra.insert(
            0,
            BenchmarkDayManifest {
                trading_date: date("2026-08-19"),
                manifest_hash: bindings[0].manifest_hash.clone(),
            },
        );
        let extra_error = runner
            .preview(scheduled_request(
                shanghai_at("2026-08-22", 15, 30, 0),
                extra,
            ))
            .expect_err("TEST_CODE extra day binding");
        assert_eq!(extra_error.class(), ReplayErrorClass::FailedIntegrity);
        assert_eq!(extra_error.code(), "benchmark_day_manifest_extra");

        let mismatch = runner
            .preview(scheduled_request(
                shanghai_at("2026-08-22", 15, 30, 0),
                vec![
                    BenchmarkDayManifest {
                        trading_date: bindings[0].trading_date,
                        manifest_hash: bindings[1].manifest_hash.clone(),
                    },
                    BenchmarkDayManifest {
                        trading_date: bindings[1].trading_date,
                        manifest_hash: bindings[0].manifest_hash.clone(),
                    },
                ],
            ))
            .expect_err("TEST_CODE manifest request/date mismatch");
        assert_eq!(mismatch.class(), ReplayErrorClass::FailedIntegrity);
        assert_eq!(mismatch.stage(), ReplayStage::Benchmark);
        assert_eq!(mismatch.code(), "benchmark_expected_request_mismatch");
        assert_eq!(runner_readonly_counts(&path), before_preview_failures);
        assert_eq!(sqlite_object_stats(&path), before_preview_objects);

        let before_invalid = attribution_table_counts(&path);
        let zero_offset = FixedOffset::east_opt(0)
            .unwrap()
            .from_local_datetime(&date("2026-08-21").and_hms_opt(15, 30, 0).unwrap())
            .single()
            .unwrap();
        let invalid_time = runner
            .commit(scheduled_request(zero_offset, bindings.clone()))
            .expect_err("TEST_CODE invalid formal invocation cannot be audited");
        assert_eq!(invalid_time.stage(), ReplayStage::Request);
        assert_eq!(invalid_time.code(), "invalid_invocation_timezone");
        assert!(invalid_time.failure_receipt().is_none());
        assert_eq!(attribution_table_counts(&path), before_invalid);

        let incomplete = runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 14, 59, 59),
                bindings,
            ))
            .expect_err("TEST_CODE pre-close formal invocation is incomplete");
        assert_eq!(incomplete.code(), "current_session_incomplete");
        assert!(incomplete.failure_receipt().is_some());
        assert_eq!(
            attribution_table_counts(&path),
            vec![3, 3, 0, 0, 0, 0, 3, 3]
        );
        drop(runner);
        drop(manager);
        remove_database(path);
    }

    #[test]
    fn runner_current_session_mapping_preserves_historical_and_database_unavailability() {
        let trade_path = complete_database("runner_historical_trade_time");
        {
            let connection = Connection::open(&trade_path).unwrap();
            connection
                .execute(
                    "UPDATE order_audit SET quote_observed_at=NULL WHERE id=1",
                    [],
                )
                .unwrap();
            rehash_audits(&connection);
        }
        let trade_manager =
            crate::database::attribution_reports::test_runner_database_manager(&trade_path);
        let trade_runner = AttributionReplayRunner::new_for_test(
            &trade_manager,
            AttributionReplayLoader::new(&trade_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let trade_error = trade_runner
            .preview(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                Vec::new(),
            ))
            .expect_err("TEST_CODE historical missing trade time must remain exact");
        assert_eq!(trade_error.stage(), ReplayStage::TradeEvidence);
        assert_eq!(trade_error.code(), "trade_time_unavailable");
        assert!(!trade_error.retryable());
        drop(trade_runner);
        drop(trade_manager);
        remove_database(trade_path);

        let calendar_path = complete_database("runner_historical_calendar");
        {
            let connection = Connection::open(&calendar_path).unwrap();
            connection
                .execute(
                    "UPDATE paper_trades SET ts='2024-08-20 09:31:05' WHERE id=1",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE paper_trades SET ts='2024-08-21 14:20:00' WHERE id=2",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE order_audit SET quote_observed_at='2024-08-20T09:31:05+08:00' WHERE id=1",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE order_audit SET quote_observed_at='2024-08-21T14:20:00+08:00' WHERE id=2",
                    [],
                )
                .unwrap();
            rehash_audits(&connection);
        }
        let calendar_manager =
            crate::database::attribution_reports::test_runner_database_manager(&calendar_path);
        let calendar_runner = AttributionReplayRunner::new_for_test(
            &calendar_manager,
            AttributionReplayLoader::new(&calendar_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let calendar_error = calendar_runner
            .preview(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                Vec::new(),
            ))
            .expect_err("TEST_CODE historical calendar coverage must remain exact");
        assert_eq!(calendar_error.stage(), ReplayStage::Calendar);
        assert_eq!(calendar_error.code(), "fill_calendar_authority_failed");
        assert!(!calendar_error.retryable());
        drop(calendar_runner);
        drop(calendar_manager);
        remove_database(calendar_path);

        let busy_path = complete_database("runner_busy_source");
        let busy_store_path = complete_database("runner_busy_store");
        let busy_manager =
            crate::database::attribution_reports::test_runner_database_manager(&busy_store_path);
        let writer = Connection::open(&busy_path).unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let busy_runner = AttributionReplayRunner::new_for_test(
            &busy_manager,
            AttributionReplayLoader::new(&busy_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let busy_error = busy_runner
            .preview(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                Vec::new(),
            ))
            .expect_err("TEST_CODE database busy must remain source unavailable");
        assert_eq!(busy_error.stage(), ReplayStage::TradeEvidence);
        assert_eq!(busy_error.code(), "replay_source_unavailable");
        assert!(busy_error.retryable());
        writer.execute_batch("ROLLBACK").unwrap();
        drop(writer);
        drop(busy_runner);
        drop(busy_manager);
        remove_database(busy_path);
        remove_database(busy_store_path);

        let admitted = admit_replay_request(scheduled_request(
            shanghai_at("2026-08-21", 15, 30, 0),
            Vec::new(),
        ))
        .unwrap();
        let calendar = resolve_admitted_calendar(&admitted).unwrap();
        let historical_close_error = map_current_session_evidence_error(
            &admitted,
            &calendar,
            ReplayError::new(
                ReplayErrorClass::Unavailable,
                ReplayStage::TradeEvidence,
                "stock_close_unavailable",
                true,
            )
            .with_failure_date(date("2026-08-20"))
            .with_evidence_failure_kind(ReplayEvidenceFailureKind::StockCloseAbsent),
        );
        assert_eq!(historical_close_error.code(), "stock_close_unavailable");
        assert!(historical_close_error.retryable());
    }

    #[test]
    fn runner_maps_only_target_day_benchmark_unavailability_to_current_session() {
        let path = complete_database("runner_benchmark_failure_day");
        let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
        let bindings = [
            append_test_benchmark_manifest(&manager, date("2026-08-20"), 0.0),
            append_test_benchmark_manifest(&manager, date("2026-08-21"), 10.0),
        ];
        let runner = AttributionReplayRunner::new_for_test(
            &manager,
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );

        let historical_missing = runner
            .preview(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                vec![bindings[1].clone()],
            ))
            .expect_err("TEST_CODE historical benchmark gap must remain exact unavailable");
        assert_eq!(historical_missing.stage(), ReplayStage::Benchmark);
        assert_eq!(
            historical_missing.code(),
            "benchmark_day_manifest_unavailable"
        );
        assert!(historical_missing.retryable());

        let target_missing = runner
            .preview(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                vec![bindings[0].clone()],
            ))
            .expect_err("TEST_CODE target benchmark gap is current-session incomplete");
        assert_eq!(target_missing.stage(), ReplayStage::Benchmark);
        assert_eq!(target_missing.code(), "current_session_incomplete");
        assert!(target_missing.retryable());

        drop(runner);
        drop(manager);
        remove_database(path);

        let storage_path = complete_database("runner_target_benchmark_storage_unavailable");
        {
            let connection = Connection::open(&storage_path).unwrap();
            connection
                .execute("DELETE FROM paper_trades WHERE id=2", [])
                .unwrap();
            connection
                .execute("DELETE FROM order_audit WHERE id=2", [])
                .unwrap();
            connection
                .execute(
                    "UPDATE paper_trades SET ts='2026-08-21 09:31:05' WHERE id=1",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE order_audit SET quote_observed_at='2026-08-21T09:31:05+08:00' WHERE id=1",
                    [],
                )
                .unwrap();
            connection
                .execute("DELETE FROM stock_daily WHERE id=1", [])
                .unwrap();
            rehash_audits(&connection);
        }
        let storage_manager =
            crate::database::attribution_reports::test_runner_database_manager(&storage_path);
        let target_binding =
            append_test_benchmark_manifest(&storage_manager, date("2026-08-21"), 0.0);
        let storage_runner = AttributionReplayRunner::new_for_test(
            &storage_manager,
            AttributionReplayLoader::new(&storage_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let held_pool_connection = storage_manager
            .get_conn()
            .expect("TEST_CODE exhaust the one-connection benchmark pool");
        let storage_unavailable = storage_runner
            .preview(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                vec![target_binding],
            ))
            .expect_err("TEST_CODE target benchmark storage unavailable keeps exact taxonomy");
        assert_eq!(storage_unavailable.stage(), ReplayStage::Benchmark);
        assert_eq!(
            storage_unavailable.code(),
            "benchmark_segment_storage_unavailable"
        );
        assert!(storage_unavailable.retryable());
        drop(held_pool_connection);
        drop(storage_runner);
        drop(storage_manager);
        remove_database(storage_path);
    }

    #[test]
    fn benchmark_failure_context_preserves_typed_code_in_leaf_fingerprint() {
        let manifest_hash = "a".repeat(64);
        let trading_date = date("2026-08-21");
        let exact_absence = map_benchmark_error(BenchmarkError::Unavailable {
            code: "benchmark_manifest_unavailable",
            retryable: true,
        })
        .with_benchmark_failure_context(trading_date, &manifest_hash);
        let storage_unavailable = map_benchmark_error(BenchmarkError::Unavailable {
            code: "benchmark_segment_storage_unavailable",
            retryable: true,
        })
        .with_benchmark_failure_context(trading_date, &manifest_hash);

        assert_eq!(exact_absence.code(), "benchmark_manifest_unavailable");
        assert_eq!(
            storage_unavailable.code(),
            "benchmark_segment_storage_unavailable"
        );
        assert_ne!(
            exact_absence.failure_fingerprint,
            storage_unavailable.failure_fingerprint
        );
    }

    #[test]
    fn runner_validates_paper_and_terminal_dates_with_one_immutable_authority() {
        fn assert_rejected(label: &str, paper_time: &str, expected_code: &str) {
            let path = complete_database(label);
            {
                let connection = Connection::open(&path).unwrap();
                connection
                    .execute("UPDATE paper_trades SET ts=?1 WHERE id=1", [paper_time])
                    .unwrap();
            }
            let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
            let bindings = vec![
                append_test_benchmark_manifest(&manager, date("2026-08-20"), 0.0),
                append_test_benchmark_manifest(&manager, date("2026-08-21"), 10.0),
            ];
            let runner = AttributionReplayRunner::new_for_test(
                &manager,
                AttributionReplayLoader::new(&path),
                "TEST_CODE_000300",
                "TEST_CODE_MINUTE_END_LABEL",
            );
            let error = runner
                .preview(scheduled_request(
                    shanghai_at("2026-08-21", 15, 30, 0),
                    bindings,
                ))
                .expect_err("TEST_CODE bad paper date must fail the whole replay");
            assert_eq!(error.class(), ReplayErrorClass::FailedIntegrity);
            assert_eq!(error.stage(), ReplayStage::Calendar);
            assert_eq!(error.code(), expected_code);
            drop(runner);
            drop(manager);
            remove_database(path);
        }

        assert_rejected(
            "runner_paper_weekend",
            "2026-08-15 09:31:05",
            "fill_non_trading_day",
        );
        assert_rejected(
            "runner_paper_holiday",
            "2026-06-19 09:31:05",
            "fill_non_trading_day",
        );
        assert_rejected(
            "runner_paper_coverage",
            "2024-08-20 09:31:05",
            "fill_calendar_authority_failed",
        );
        assert_rejected(
            "runner_paper_terminal_mismatch",
            "2026-08-19 09:31:05",
            "fill_terminal_date_mismatch",
        );
    }

    #[test]
    fn runner_stores_the_exact_task30_validated_fee_basis_identity() {
        let path = complete_database("runner_fee_identity");
        let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
        let bindings = vec![
            append_test_benchmark_manifest(&manager, date("2026-08-20"), 0.0),
            append_test_benchmark_manifest(&manager, date("2026-08-21"), 10.0),
        ];
        let runner = AttributionReplayRunner::new_for_test_with_fee_ledger(
            &manager,
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
            AuthoritativeFillFeeLedger {
                entries: vec![fee(1, 1.25), fee(2, 1.50)],
            },
        );
        let request = scheduled_request(shanghai_at("2026-08-21", 15, 30, 0), bindings.clone());
        let preview = runner
            .preview(request)
            .expect("TEST_CODE fee-backed preview");
        let MetricAvailability::Available(basis) = preview.report().fee_basis() else {
            panic!("TEST_CODE fee basis must be available");
        };
        let expected_basis_id = basis.basis_id.clone();
        let receipt = runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                bindings,
            ))
            .expect("TEST_CODE fee-backed formal replay");
        let retained_fee: String = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT fee_value FROM attribution_report_revision WHERE id=?1",
                [receipt.report_revision_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained_fee, expected_basis_id);
        drop(runner);
        drop(manager);
        remove_database(path);
    }

    #[test]
    fn runner_loader_failure_progress_is_monotonic_and_never_fabricates_evidence() {
        let stock_path = complete_database("runner_stock_progress");
        Connection::open(&stock_path)
            .unwrap()
            .execute("DELETE FROM stock_daily WHERE date='2026-08-21'", [])
            .unwrap();
        let stock_manager =
            crate::database::attribution_reports::test_runner_database_manager(&stock_path);
        let stock_runner = AttributionReplayRunner::new_for_test(
            &stock_manager,
            AttributionReplayLoader::new(&stock_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let stock_admitted = admit_replay_request(scheduled_request(
            shanghai_at("2026-08-22", 15, 30, 0),
            Vec::new(),
        ))
        .unwrap();
        let stock_failure = match stock_runner.prepare(&stock_admitted) {
            Ok(_) => panic!("TEST_CODE stock-close failure expected"),
            Err(failure) => failure,
        };
        assert!(matches!(
            stock_failure.evidence.trade,
            FailureEvidenceState::Available(_)
        ));
        assert!(matches!(
            stock_failure.evidence.stock_close,
            FailureEvidenceState::Unavailable(_)
        ));
        assert_eq!(stock_failure.evidence.fee, FailureEvidenceState::Unknown);
        drop(stock_runner);
        drop(stock_manager);
        remove_database(stock_path);

        let fee_path = complete_database("runner_fee_progress");
        let fee_manager =
            crate::database::attribution_reports::test_runner_database_manager(&fee_path);
        let fee_runner = AttributionReplayRunner::new_for_test_with_fee_ledger(
            &fee_manager,
            AttributionReplayLoader::new(&fee_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
            AuthoritativeFillFeeLedger {
                entries: vec![fee(1, 1.25)],
            },
        );
        let fee_admitted = admit_replay_request(scheduled_request(
            shanghai_at("2026-08-22", 15, 30, 0),
            Vec::new(),
        ))
        .unwrap();
        let fee_failure = match fee_runner.prepare(&fee_admitted) {
            Ok(_) => panic!("TEST_CODE fee validation failure expected"),
            Err(failure) => failure,
        };
        assert_eq!(fee_failure.error.code(), "fee_evidence_failed");
        assert!(matches!(
            fee_failure.evidence.trade,
            FailureEvidenceState::Available(_)
        ));
        assert!(matches!(
            fee_failure.evidence.stock_close,
            FailureEvidenceState::Available(_)
        ));
        assert!(matches!(
            fee_failure.evidence.fee,
            FailureEvidenceState::Unavailable(_)
        ));
        drop(fee_runner);
        drop(fee_manager);
        remove_database(fee_path);

        let missing = test_database_path("runner_early_source_progress");
        let early = AttributionReplayLoader::new(&missing)
            .load_with_progress(&request_with_no_fees())
            .expect_err("TEST_CODE early source identity failure expected");
        assert_eq!(early.stage, AttributionReplayLoadStage::Trade);
        assert!(early.progress.trade_manifest_hash.is_none());
        assert!(early.progress.stock_close_manifest_hash.is_none());
        assert!(early.progress.fee.is_none());
    }

    #[test]
    fn runner_failure_summary_binds_bad_leaf_and_every_known_stage_identity() {
        fn failure_summaries(path: &Path) -> Vec<String> {
            let connection = Connection::open(path).unwrap();
            let mut statement = connection
                .prepare(
                    "SELECT source_summary_hash FROM attribution_failure_audit ORDER BY id ASC",
                )
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<Vec<String>, _>>()
                .unwrap()
        }

        let leaf_path = complete_database("runner_failure_leaf_identity");
        {
            let connection = Connection::open(&leaf_path).unwrap();
            connection
                .execute(
                    "UPDATE order_audit SET quote_observed_at=NULL WHERE id=1",
                    [],
                )
                .unwrap();
            rehash_audits(&connection);
        }
        let leaf_manager =
            crate::database::attribution_reports::test_runner_database_manager(&leaf_path);
        let leaf_runner = AttributionReplayRunner::new_for_test(
            &leaf_manager,
            AttributionReplayLoader::new(&leaf_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        leaf_runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                Vec::new(),
            ))
            .expect_err("TEST_CODE first missing terminal must be audited");
        {
            let connection = Connection::open(&leaf_path).unwrap();
            connection
                .execute(
                    "UPDATE order_audit SET quote_observed_at='2026-08-20T09:31:05+08:00' WHERE id=1",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE order_audit SET quote_observed_at=NULL WHERE id=2",
                    [],
                )
                .unwrap();
            rehash_audits(&connection);
        }
        leaf_runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                Vec::new(),
            ))
            .expect_err("TEST_CODE second missing terminal must be audited");
        let leaf_summaries = failure_summaries(&leaf_path);
        assert_eq!(leaf_summaries.len(), 2);
        assert_ne!(leaf_summaries[0], leaf_summaries[1]);
        drop(leaf_runner);
        drop(leaf_manager);
        remove_database(leaf_path);

        let stock_path = complete_database("runner_failure_stock_stage_identity");
        Connection::open(&stock_path)
            .unwrap()
            .execute("DELETE FROM stock_daily WHERE date='2026-08-21'", [])
            .unwrap();
        let stock_manager =
            crate::database::attribution_reports::test_runner_database_manager(&stock_path);
        let stock_runner = AttributionReplayRunner::new_for_test(
            &stock_manager,
            AttributionReplayLoader::new(&stock_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        stock_runner
            .commit(scheduled_request(
                shanghai_at("2026-08-22", 15, 30, 0),
                Vec::new(),
            ))
            .expect_err("TEST_CODE first stock-close failure must be audited");
        Connection::open(&stock_path)
            .unwrap()
            .execute(
                "UPDATE paper_trades SET name='TEST_CODE_CHANGED_SOURCE_NAME' WHERE id=1",
                [],
            )
            .unwrap();
        stock_runner
            .commit(scheduled_request(
                shanghai_at("2026-08-22", 15, 30, 0),
                Vec::new(),
            ))
            .expect_err("TEST_CODE same stock-close leaf after trade revision must be audited");
        let stock_summaries = failure_summaries(&stock_path);
        assert_eq!(stock_summaries.len(), 2);
        assert_ne!(stock_summaries[0], stock_summaries[1]);
        drop(stock_runner);
        drop(stock_manager);
        remove_database(stock_path);

        let stage_path = complete_database("runner_failure_stage_identity");
        let stage_manager =
            crate::database::attribution_reports::test_runner_database_manager(&stage_path);
        let historical = append_test_benchmark_manifest(&stage_manager, date("2026-08-20"), 0.0);
        let stage_runner = AttributionReplayRunner::new_for_test(
            &stage_manager,
            AttributionReplayLoader::new(&stage_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        stage_runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                vec![historical.clone()],
            ))
            .expect_err("TEST_CODE target benchmark gap must be audited");
        Connection::open(&stage_path)
            .unwrap()
            .execute(
                "UPDATE paper_trades SET name='TEST_CODE_CHANGED_SOURCE_NAME' WHERE id=1",
                [],
            )
            .unwrap();
        stage_runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                vec![historical.clone()],
            ))
            .expect_err("TEST_CODE same benchmark gap after trade revision must be audited");
        {
            let connection = Connection::open(&stage_path).unwrap();
            connection
                .execute(
                    "UPDATE paper_trades SET name='TEST_CODE公司' WHERE id=1",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE stock_daily SET created_at='2026-08-23' WHERE date='2026-08-21'",
                    [],
                )
                .unwrap();
        }
        stage_runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                vec![historical.clone()],
            ))
            .expect_err("TEST_CODE same benchmark gap after stock-close revision must be audited");
        Connection::open(&stage_path)
            .unwrap()
            .execute(
                "UPDATE stock_daily SET created_at='2026-08-22' WHERE date='2026-08-21'",
                [],
            )
            .unwrap();
        let fee_runner = AttributionReplayRunner::new_for_test_with_fee_ledger(
            &stage_manager,
            AttributionReplayLoader::new(&stage_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
            AuthoritativeFillFeeLedger {
                entries: vec![fee(1, 1.25), fee(2, 1.50)],
            },
        );
        fee_runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                vec![historical],
            ))
            .expect_err("TEST_CODE same benchmark gap after fee revision must be audited");
        let stage_summaries = failure_summaries(&stage_path);
        assert_eq!(stage_summaries.len(), 4);
        assert_ne!(stage_summaries[0], stage_summaries[1]);
        assert_ne!(stage_summaries[0], stage_summaries[2]);
        assert_ne!(stage_summaries[0], stage_summaries[3]);
        drop(fee_runner);
        drop(stage_runner);
        drop(stage_manager);
        remove_database(stage_path);
    }

    #[test]
    fn runner_rejects_non_trading_fill_and_maps_source_and_store_failures_without_leaks() {
        let weekend_path = complete_database("runner_weekend_fill");
        {
            let connection = Connection::open(&weekend_path).unwrap();
            connection
                .execute(
                    "UPDATE paper_trades SET ts='2026-08-22 09:31:05' WHERE id=1",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE paper_trades SET ts='2026-08-24 14:20:00' WHERE id=2",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE order_audit SET quote_observed_at='2026-08-22T09:31:05+08:00' WHERE id=1",
                    [],
                )
                .unwrap();
            connection
                .execute(
                    "UPDATE order_audit SET quote_observed_at='2026-08-24T14:20:00+08:00' WHERE id=2",
                    [],
                )
                .unwrap();
            connection
                .execute("UPDATE stock_daily SET date='2026-08-24' WHERE id=2", [])
                .unwrap();
            rehash_audits(&connection);
        }
        let weekend_manager =
            crate::database::attribution_reports::test_runner_database_manager(&weekend_path);
        let weekend_runner = AttributionReplayRunner::new_for_test(
            &weekend_manager,
            AttributionReplayLoader::new(&weekend_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let weekend_error = weekend_runner
            .preview(ReplayRequest {
                mode: ReplayMode::Range {
                    from: date("2026-08-24"),
                    to: date("2026-08-24"),
                    invoked_at: shanghai_at("2026-08-24", 15, 30, 0),
                },
                epoch: AttributionEpochSelector::Legacy,
                benchmark_day_manifests: Vec::new(),
            })
            .expect_err("TEST_CODE weekend fill must fail before benchmark lookup");
        assert_eq!(weekend_error.class(), ReplayErrorClass::FailedIntegrity);
        assert_eq!(weekend_error.stage(), ReplayStage::Calendar);
        assert_eq!(weekend_error.code(), "fill_non_trading_day");
        assert!(!weekend_error
            .redacted_message()
            .contains(&weekend_path.to_string_lossy().to_string()));
        assert_eq!(attribution_table_counts(&weekend_path), vec![0; 8]);
        drop(weekend_runner);
        drop(weekend_manager);
        remove_database(weekend_path);

        let source_path = complete_database("runner_source_mapping");
        Connection::open(&source_path)
            .unwrap()
            .execute("DELETE FROM stock_daily WHERE date='2026-08-21'", [])
            .unwrap();
        let source_manager =
            crate::database::attribution_reports::test_runner_database_manager(&source_path);
        let source_runner = AttributionReplayRunner::new_for_test(
            &source_manager,
            AttributionReplayLoader::new(&source_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let current = source_runner
            .preview(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                Vec::new(),
            ))
            .expect_err("TEST_CODE current close source incomplete");
        assert_eq!(current.code(), "current_session_incomplete");
        let weekend = source_runner
            .preview(scheduled_request(
                shanghai_at("2026-08-22", 15, 30, 0),
                Vec::new(),
            ))
            .expect_err("TEST_CODE weekend source unavailable remains exact");
        assert_eq!(weekend.code(), "stock_close_unavailable");
        assert_eq!(weekend.stage(), ReplayStage::TradeEvidence);
        drop(source_runner);
        drop(source_manager);
        remove_database(source_path);

        let store_path = complete_database("runner_store_failure");
        let store_manager =
            crate::database::attribution_reports::test_runner_database_manager(&store_path);
        let store_bindings = vec![
            append_test_benchmark_manifest(&store_manager, date("2026-08-20"), 0.0),
            append_test_benchmark_manifest(&store_manager, date("2026-08-21"), 10.0),
        ];
        Connection::open(&store_path)
            .unwrap()
            .execute("DROP TRIGGER trg_attribution_run_audit_no_update", [])
            .unwrap();
        let store_runner = AttributionReplayRunner::new_for_test(
            &store_manager,
            AttributionReplayLoader::new(&store_path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let storage = store_runner
            .commit(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                store_bindings,
            ))
            .expect_err("TEST_CODE store integrity failure cannot return success");
        assert_eq!(storage.class(), ReplayErrorClass::Storage);
        assert_eq!(storage.stage(), ReplayStage::Store);
        assert!(storage.failure_receipt().is_none());
        assert_eq!(attribution_table_counts(&store_path), vec![0; 8]);
        drop(store_runner);
        drop(store_manager);
        remove_database(store_path);
    }

    #[test]
    fn production_runner_keeps_minute_semantics_unverified() {
        let path = complete_database("runner_production_semantics");
        let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
        let runner = AttributionReplayRunner::new(&manager, AttributionReplayLoader::new(&path));
        assert_eq!(runner.benchmark_instrument, HS300_CANONICAL);
        assert_eq!(runner.minute_semantics, MinuteLabelSemantics::Unverified);
        drop(runner);
        drop(manager);
        remove_database(path);
    }

    #[test]
    fn runner_commit_reuses_friday_report_for_weekend_runs_and_appends_manifest_successor() {
        let path = complete_database("runner_commit");
        let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
        let initial = vec![
            append_test_benchmark_manifest(&manager, date("2026-08-20"), 0.0),
            append_test_benchmark_manifest(&manager, date("2026-08-21"), 10.0),
        ];
        let runner = AttributionReplayRunner::new_for_test(
            &manager,
            AttributionReplayLoader::new(&path),
            "TEST_CODE_000300",
            "TEST_CODE_MINUTE_END_LABEL",
        );
        let friday_committed = runner
            .commit_with_report(scheduled_request(
                shanghai_at("2026-08-21", 15, 30, 0),
                initial.clone(),
            ))
            .expect("TEST_CODE Friday formal replay");
        assert_eq!(
            friday_committed.prepared().invocation().target_from,
            date("2026-08-21")
        );
        assert_eq!(
            friday_committed.prepared().invocation().target_to,
            date("2026-08-21")
        );
        assert_eq!(
            friday_committed.prepared().benchmark_day_manifests(),
            initial.as_slice()
        );
        let friday = friday_committed.receipt().clone();
        assert_eq!(friday.epoch, AttributionReportEpochBinding::Legacy);
        let saturday = runner
            .commit(scheduled_request(
                shanghai_at("2026-08-22", 15, 30, 0),
                initial.clone(),
            ))
            .expect("TEST_CODE Saturday formal replay of Friday");
        let sunday = runner
            .commit(scheduled_request(
                shanghai_at("2026-08-23", 15, 30, 0),
                initial.clone(),
            ))
            .expect("TEST_CODE Sunday formal replay of Friday");
        assert_eq!(friday.report_revision_id, saturday.report_revision_id);
        assert_eq!(friday.report_revision_id, sunday.report_revision_id);
        assert_eq!(friday.report_identity, saturday.report_identity);
        assert_eq!(friday.report_identity, sunday.report_identity);
        assert_ne!(friday.run.run_audit_id, saturday.run.run_audit_id);
        assert_ne!(saturday.run.run_audit_id, sunday.run.run_audit_id);
        assert_eq!(
            attribution_table_counts(&path),
            vec![3, 3, 1, 1, 1, 1, 0, 0]
        );

        let retained_payload: String = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT result_payload_json FROM attribution_report_revision WHERE id=?1",
                [friday.report_revision_id],
                |row| row.get(0),
            )
            .unwrap();
        let retained: serde_json::Value = serde_json::from_str(&retained_payload).unwrap();
        assert_eq!(
            retained["benchmark_day_manifests"][0]["trading_date"],
            "2026-08-20"
        );
        assert_eq!(
            retained["benchmark_day_manifests"][1]["trading_date"],
            "2026-08-21"
        );
        assert!(retained["core_report"].is_object());

        let revised_day = append_test_benchmark_manifest(&manager, date("2026-08-21"), 20.0);
        let revised = runner
            .commit(scheduled_request(
                shanghai_at("2026-08-23", 16, 0, 0),
                vec![initial[0].clone(), revised_day],
            ))
            .expect("TEST_CODE revised benchmark report");
        assert_eq!(revised.report_revision, 2);
        assert_eq!(
            revised.predecessor_report_id,
            Some(friday.report_revision_id)
        );
        assert_ne!(revised.report_identity, friday.report_identity);
        assert_eq!(
            attribution_table_counts(&path),
            vec![4, 4, 2, 2, 2, 2, 0, 0]
        );
        drop(runner);
        drop(manager);
        remove_database(path);
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
        assert_eq!(evidence.trade_manifest_hash().len(), 64);
        assert!(evidence
            .trade_manifest_hash()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
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

    // TEST_CODE fixture keeps every replay-evidence field visible at call sites.
    #[allow(clippy::too_many_arguments)]
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
        let scoped_fill_manifest_hash = canonical_scoped_fill_manifest_hash(
            &fills
                .iter()
                .map(|evidence| evidence.fill.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| "f".repeat(64));
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
            AttributionEpochReplayEvidence::legacy(scoped_fill_manifest_hash),
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
    fn epoch_capability_seal_rejects_selector_receipt_carry_exclusion_and_scope_rebinding() {
        let evidence = replay_evidence("2026-08-20", "2026-08-20", Vec::new(), Some(Vec::new()));
        let assert_rejected = |candidate: AttributionReplayEvidence| {
            assert!(matches!(
                compute_attribution_range(&candidate, &[], &verified_minute_labels()),
                Err(AttributionReplayError::FailedIntegrity {
                    code: AttributionIntegrityFailure::ReplayEvidence,
                    ..
                })
            ));
        };

        let mut selector = evidence.clone();
        selector.epoch.selector = AttributionEpochSelector::Active;
        assert_rejected(selector);

        let mut receipt = evidence.clone();
        receipt.epoch.epoch_id = Some("a".repeat(64));
        receipt.epoch.receipt_hash = Some("b".repeat(64));
        assert_rejected(receipt);

        let mut carry = evidence.clone();
        carry.epoch.legacy_carry_manifest_hash = Some("c".repeat(64));
        carry.epoch.remaining_quarantine = vec![LegacyCarryPosition {
            code: "TEST_CODE_600001".to_owned(),
            quantity: 100,
        }];
        assert_rejected(carry);

        let mut exclusion = evidence.clone();
        exclusion.epoch.exclusion_manifest_hash = Some("d".repeat(64));
        exclusion.epoch.excluded_fills = vec![EpochExclusion {
            fill_id: 1,
            code: "TEST_CODE_600001".to_owned(),
            direction: "buy".to_owned(),
            quantity: 100,
            reason: EpochExclusionReason::LegacyCarryOverlap,
        }];
        assert_rejected(exclusion);

        let mut scoped = evidence;
        scoped.epoch.scoped_fill_manifest_hash = "e".repeat(64);
        assert_rejected(scoped);
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
