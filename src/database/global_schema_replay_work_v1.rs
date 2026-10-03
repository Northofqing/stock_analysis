//! Closed V1 SQL acquisition and cumulative replay work; no financial capability.
//!
//! Pure layout bounds describe reviewed source rules, not runtime qualification.
//! Existing SQL loads and Historical replay are NOT protected by this module.
#![allow(dead_code)] // Complete Target replay remains gated on later slices.

use super::super::global_schema_catalog_v1::{RowsSpecDebitFailure, RowsSpecWork};
use std::mem::{align_of, size_of, MaybeUninit};
use std::ptr::NonNull;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReplaySite {
    SqlRows,
    CodecScratch,
    Collection,
    StateCopy,
    Formatting,
    ErrorStorage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LayoutFailure {
    Overflow,
    InvalidAlignment,
    AddressSpace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResourceCause {
    Debit(RowsSpecDebitFailure),
    Layout(LayoutFailure),
    AllocationFailed,
}

// Fixed error escrow: reporting a resource failure never formats or copies data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplayResourceFailure {
    pub(super) site: ReplaySite,
    pub(super) cause: ResourceCause,
    pub(super) used: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqlOperation {
    Encoding,
    Prepare,
    Bind,
    Step,
    Finalize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplaySqlQualificationFailure {
    Sqlite {
        operation: SqlOperation,
        code: Option<i32>,
    },
    Encoding,
    Shape,
    Null,
    Utf8,
    Extent,
    PinUnavailable,
    SqlProviderUnavailable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplayCodecFailureKind {
    PinUnavailable,
    UnsupportedSerdeProfile,
    MalformedJson,
    UnexpectedType,
    MissingField,
    DuplicateField,
    UnknownField,
    UnknownVariant,
    SequenceArity,
    IntegerRange,
    FloatRange,
    InvalidDate,
    InvalidDateTime,
    InvalidMapKey,
    TypedRecursionLimit,
    Noncanonical,
    RecordExtent,
    PlanMismatch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplayCodecQualificationFailure {
    kind: ReplayCodecFailureKind,
    offset: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplayTerminalFailure {
    Resource(ReplayResourceFailure),
    Qualification(ReplaySqlQualificationFailure),
    CodecQualification(ReplayCodecQualificationFailure),
}

// Constructed once by the actual target owner, never by a phase borrower.
pub(super) struct ReplayTerminalState {
    first: Option<ReplayTerminalFailure>,
}
impl ReplayTerminalState {
    pub(super) fn new(_: super::target::ReplayOwnerInit) -> Self {
        Self { first: None }
    }
    pub(super) fn finish(&self) -> Result<(), ReplayTerminalFailure> {
        self.first.map_or(Ok(()), Err)
    }
    fn latch(&mut self, failure: ReplayTerminalFailure) -> ReplayTerminalFailure {
        *self.first.get_or_insert(failure)
    }
}

/// Short loans borrow both persistent fields; drop has no commit/reset action.
/// Only the actual owner's opaque parts can construct a production borrower.
pub(crate) struct BorrowedReplayWork<'a> {
    metadata: &'a mut RowsSpecWork,
    terminal: &'a mut ReplayTerminalState,
    #[cfg(test)]
    text_copy_requests: usize,
    #[cfg(test)]
    row_buffer_requests: usize,
}
impl<'a> BorrowedReplayWork<'a> {
    pub(super) fn borrow(parts: super::target::ReplayOwnerLoan<'a>) -> Self {
        let (metadata, terminal) = parts.into_parts();
        Self {
            metadata,
            terminal,
            #[cfg(test)]
            text_copy_requests: 0,
            #[cfg(test)]
            row_buffer_requests: 0,
        }
    }
    #[cfg(test)]
    fn test_borrow(metadata: &'a mut RowsSpecWork, terminal: &'a mut ReplayTerminalState) -> Self {
        Self {
            metadata,
            terminal,
            #[cfg(test)]
            text_copy_requests: 0,
            #[cfg(test)]
            row_buffer_requests: 0,
        }
    }
    fn fail(&mut self, site: ReplaySite, cause: ResourceCause) -> ReplayTerminalFailure {
        self.terminal
            .latch(ReplayTerminalFailure::Resource(ReplayResourceFailure {
                site,
                cause,
                used: self.metadata.used(),
            }))
    }
    fn qualify(&mut self, cause: ReplaySqlQualificationFailure) -> ReplayTerminalFailure {
        self.terminal
            .latch(ReplayTerminalFailure::Qualification(cause))
    }
    fn qualify_codec(
        &mut self,
        kind: ReplayCodecFailureKind,
        offset: Option<u64>,
    ) -> ReplayTerminalFailure {
        self.terminal.latch(ReplayTerminalFailure::CodecQualification(
            ReplayCodecQualificationFailure { kind, offset },
        ))
    }
    fn require_sql_provider(&mut self) -> Result<(), ReplayTerminalFailure> {
        self.finish()?;
        Err(self.qualify(ReplaySqlQualificationFailure::SqlProviderUnavailable))
    }
    pub(crate) fn codec_memory<'loan>(
        &'loan mut self,
    ) -> Result<ReplayMemory<'loan, 'a>, ReplayTerminalFailure> {
        self.finish()?;
        let pin = require_reviewed_layout_pin()
            .map_err(|_| self.qualify_codec(ReplayCodecFailureKind::PinUnavailable, None))?;
        pin.codec_rules()
            .map_err(|kind| self.qualify_codec(kind, None))?;
        Ok(ReplayMemory { work: self, pin })
    }
    fn sql<T>(
        &mut self,
        value: rusqlite::Result<T>,
        operation: SqlOperation,
    ) -> Result<T, ReplayTerminalFailure> {
        value.map_err(|error| {
            // Driver-owned error allocation has already happened. Never copy or
            // format its message, retry, or downgrade it to financial unavailable.
            let code = error.sqlite_error().map(|error| error.extended_code);
            self.qualify(ReplaySqlQualificationFailure::Sqlite { operation, code })
        })
    }
    pub(super) fn reserve(
        &mut self,
        site: ReplaySite,
        bytes: u64,
    ) -> Result<Reservation, ReplayTerminalFailure> {
        self.finish()?;
        self.metadata
            .try_charge(bytes)
            .map_err(|cause| self.fail(site, ResourceCause::Debit(cause)))?;
        Ok(Reservation { bytes })
    }
    pub(super) fn reserve_array<T>(
        &mut self,
        site: ReplaySite,
        count: u64,
    ) -> Result<Reservation, ReplayTerminalFailure> {
        self.finish()?;
        let bytes = exact_array_bytes::<T>(count)
            .map_err(|cause| self.fail(site, ResourceCause::Layout(cause)))?;
        self.reserve(site, bytes)
    }
    pub(super) fn finish(&self) -> Result<(), ReplayTerminalFailure> {
        self.terminal.finish()
    }
    pub(super) fn used(&self) -> u64 {
        self.metadata.used()
    }
}

pub(crate) struct V1RowsLoan<'a> {
    connection: &'a rusqlite::Connection,
    work: BorrowedReplayWork<'a>,
}
impl<'a> V1RowsLoan<'a> {
    pub(super) fn from_retained(
        reader: &'a super::target::RetainedTargetReader<'_>,
        parts: super::target::ReplayOwnerLoan<'a>,
    ) -> Self {
        Self {
            connection: reader.connection(),
            work: BorrowedReplayWork::borrow(parts),
        }
    }
    pub(crate) fn events(
        &mut self,
        id: &str,
    ) -> Result<Vec<crate::trading::paper_ledger::EventRow>, ReplayTerminalFailure> {
        let plan = sql_rows::preflight(self, id, SqlExtentKind::V1Event)?;
        self.require_pin()?;
        sql_rows::events(self, plan)
    }
    pub(crate) fn head(
        &mut self,
        id: &str,
    ) -> Result<Option<crate::trading::paper_ledger::HeadRow>, ReplayTerminalFailure> {
        let plan = sql_rows::preflight(self, id, SqlExtentKind::V1Head)?;
        self.require_pin()?;
        sql_rows::head(self, plan)
    }
    fn require_pin(&mut self) -> Result<(), ReplayTerminalFailure> {
        self.work.finish()?;
        let _pin = require_reviewed_layout_pin().map_err(|_| {
            self.work
                .qualify(ReplaySqlQualificationFailure::PinUnavailable)
        })?;
        // A layout proof does not identify the linked SQLite provider.
        self.work.require_sql_provider()
    }
    pub(super) fn finish(&self) -> Result<(), ReplayTerminalFailure> {
        self.work.finish()
    }
    pub(super) fn latch_failure(&mut self, failure: ReplayTerminalFailure) {
        self.work.terminal.latch(failure);
    }
    #[cfg(test)]
    pub(super) fn used(&self) -> u64 {
        self.work.used()
    }
}
#[path = "global_schema_replay_sql_v1.rs"]
mod sql_rows;

/// Move-only evidence of a debit, not an allocation or qualification capability.
/// Dropping or consuming it never refunds the cumulative meter.
#[must_use]
pub(super) struct Reservation {
    bytes: u64,
}
impl Reservation {
    pub(super) fn consume(self) -> u64 {
        self.bytes
    }
}

// Ordinary generated code refuses. Future accepted issuance alone may create
// this child-private token; no test factory or caller-supplied proof exists.
mod layout_qualification {
    use super::{LayoutPinRefusal, ReplayCodecFailureKind};

    pub(crate) struct ReviewedLayoutPin {
        consumer_seed_sha256: [u8; 32],
        proof_rules_sha256: [u8; 32],
        rules: Option<ReviewedRulesV1>,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum ReviewedRulesV1 {
        // Future issuance must bind actual serde_json default/std source/cfg.
        FiniteDtoV1,
    }
    impl ReviewedLayoutPin {
        pub(super) fn codec_rules(&self) -> Result<&ReviewedRulesV1, ReplayCodecFailureKind> {
            self.rules
                .as_ref()
                .ok_or(ReplayCodecFailureKind::UnsupportedSerdeProfile)
        }
    }
    include!(env!("STOCK_REPLAY_PIN_INCLUDE"));
}
pub(crate) use layout_qualification::ReviewedLayoutPin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutPinRefusal {
    BuildVerificationNotInstalled,
}
pub(crate) fn require_reviewed_layout_pin() -> Result<ReviewedLayoutPin, LayoutPinRefusal> {
    layout_qualification::acquire()
}

/// Borrows the existing cumulative meter and terminal; no Drop/reset/refund.
/// Paid closed DTO operations are supplied only by their later reviewed slice.
pub(crate) struct ReplayMemory<'loan, 'pool> {
    work: &'loan mut BorrowedReplayWork<'pool>,
    pin: ReviewedLayoutPin,
}
impl ReplayMemory<'_, '_> {
    fn rules(&self) -> Result<&layout_qualification::ReviewedRulesV1, ReplayCodecFailureKind> {
        self.pin.codec_rules()
    }
    pub(crate) fn finish(&self) -> Result<(), ReplayTerminalFailure> {
        self.work.finish()
    }
    pub(crate) fn refuse_codec(
        &mut self,
        kind: ReplayCodecFailureKind,
        offset: Option<u64>,
    ) -> ReplayTerminalFailure {
        self.work.qualify_codec(kind, offset)
    }
}

pub(super) const REVIEWED_LAYOUT_SPEC: &str =
    "rustc-59807616e1fa2540724bfbac14d7976d7e4a3860/x86_64-apple-darwin/hashbrown-0.16.1/serde_json-1.0.149";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FieldLayout {
    size: u64,
    align: u64,
}
impl FieldLayout {
    pub(super) fn of<T>() -> Self {
        Self {
            size: size_of::<T>() as u64,
            align: align_of::<T>() as u64,
        }
    }
    pub(super) fn bytes(self) -> u64 {
        self.size
    }
}

fn add(a: u64, b: u64) -> Result<u64, LayoutFailure> {
    a.checked_add(b).ok_or(LayoutFailure::Overflow)
}
fn mul(a: u64, b: u64) -> Result<u64, LayoutFailure> {
    a.checked_mul(b).ok_or(LayoutFailure::Overflow)
}
fn round_up(bytes: u64, align: u64) -> Result<u64, LayoutFailure> {
    if !align.is_power_of_two() {
        return Err(LayoutFailure::InvalidAlignment);
    }
    Ok(add(bytes, align - 1)? & !(align - 1))
}
fn layout(size: u64, align: u64) -> Result<FieldLayout, LayoutFailure> {
    if round_up(size, align)? > isize::MAX as u64 {
        return Err(LayoutFailure::AddressSpace);
    }
    Ok(FieldLayout { size, align })
}

pub(super) fn exact_array_bytes<T>(count: u64) -> Result<u64, LayoutFailure> {
    // A ZST still needs a representable element count.
    usize::try_from(count).map_err(|_| LayoutFailure::AddressSpace)?;
    layout(mul(count, size_of::<T>() as u64)?, align_of::<T>() as u64).map(|l| l.size)
}

/// Bounds every field permutation under the reviewed compiler's ordinary
/// record-layout rules (no packing/repr-align override). Not a private mirror.
pub(super) fn record_upper(fields: &[FieldLayout]) -> Result<FieldLayout, LayoutFailure> {
    let mut bytes = 0;
    let mut align = 1;
    for field in fields {
        layout(field.size, field.align)?;
        bytes = add(add(bytes, field.size)?, field.align - 1)?;
        align = align.max(field.align);
    }
    layout(round_up(bytes, align)?, align)
}

/// Requested allocation for RawVec grow_amortized, not its current allocation.
/// Call only when required > capacity; exact reservations use exact_array_bytes.
pub(super) fn amortized_vector_bytes<T>(
    capacity: u64,
    required: u64,
) -> Result<u64, LayoutFailure> {
    usize::try_from(capacity).map_err(|_| LayoutFailure::AddressSpace)?;
    usize::try_from(required).map_err(|_| LayoutFailure::AddressSpace)?;
    if size_of::<T>() == 0 || required <= capacity {
        return Ok(0);
    }
    let minimum = if size_of::<T>() == 1 {
        8
    } else if size_of::<T>() <= 1024 {
        4
    } else {
        1
    };
    exact_array_bytes::<T>(mul(capacity, 2)?.max(required).max(minimum))
}

/// Actual five LeafNode field types at the reviewed std source, B=6. Owned
/// key/value heaps and the number of insertion/clone nodes are separate work.
pub(super) fn btree_node_bounds<K, V>() -> Result<(FieldLayout, FieldLayout), LayoutFailure> {
    let leaf = record_upper(&[
        FieldLayout::of::<Option<NonNull<()>>>(),
        FieldLayout::of::<u16>(),
        FieldLayout::of::<u16>(),
        FieldLayout::of::<[MaybeUninit<K>; 11]>(),
        FieldLayout::of::<[MaybeUninit<V>; 11]>(),
    ])?;
    let edges = FieldLayout::of::<[MaybeUninit<NonNull<()>>; 12]>();
    let align = leaf.align.max(edges.align);
    let internal = layout(
        round_up(add(round_up(leaf.size, edges.align)?, edges.size)?, align)?,
        align,
    )?;
    Ok((leaf, internal))
}

/// Symbolic ErrorCode tagged-envelope + ErrorImpl's two usize fields. Dynamic
/// message/io payloads and repeated error boxing require separate reservations.
pub(super) fn serde_error_box_bound() -> Result<FieldLayout, LayoutFailure> {
    let message = FieldLayout::of::<Box<str>>();
    let io = FieldLayout::of::<std::io::Error>();
    let tag = FieldLayout::of::<u128>();
    let align = message.align.max(io.align).max(tag.align);
    let code = layout(
        round_up(
            add(add(tag.size, message.size.max(io.size))?, align - 1)?,
            align,
        )?,
        align,
    )?;
    record_upper(&[code, FieldLayout::of::<usize>(), FieldLayout::of::<usize>()])
}

/// Closed generic-width8/SSE2-width16 envelope. Consumer target_feature cannot
/// identify the cfg used to build std; runtime build qualification is separate.
pub(super) fn hash_table_bound<T>(capacity: u64) -> Result<u64, LayoutFailure> {
    usize::try_from(capacity).map_err(|_| LayoutFailure::AddressSpace)?;
    if capacity == 0 {
        return Ok(0);
    }
    fn branch<T>(capacity: u64, group: u64) -> Result<u64, LayoutFailure> {
        let size = size_of::<T>() as u64;
        let buckets = if capacity < 15 {
            let minimum = match (group, size) {
                (16, 0..=1) => 14,
                (16, 2..=3) | (8, 0..=1) => 7,
                _ => 3,
            };
            let desired = capacity.max(minimum);
            if desired < 4 {
                4
            } else if desired < 8 {
                8
            } else {
                16
            }
        } else {
            (mul(capacity, 8)? / 7)
                .checked_next_power_of_two()
                .ok_or(LayoutFailure::Overflow)?
        };
        let align = (align_of::<T>() as u64).max(group);
        let bytes = add(add(round_up(mul(size, buckets)?, align)?, buckets)?, group)?;
        layout(bytes, align)?;
        // hashbrown calculate_layout_for explicitly reserves this tail margin.
        if bytes
            > (isize::MAX as u64)
                .checked_sub(align - 1)
                .ok_or(LayoutFailure::AddressSpace)?
        {
            return Err(LayoutFailure::AddressSpace);
        }
        Ok(bytes)
    }
    Ok(branch::<T>(capacity, 8)?.max(branch::<T>(capacity, 16)?))
}

/// Dominates both reviewed stable-sort branches, even if stack scratch would
/// avoid an actual allocation. No claim about a different compiler's algorithm.
pub(super) fn stable_sort_scratch_bytes<T>(count: u64) -> Result<u64, LayoutFailure> {
    usize::try_from(count).map_err(|_| LayoutFailure::AddressSpace)?;
    if size_of::<T>() == 0 {
        return Ok(0);
    }
    let elements = (count - count / 2)
        .max(count.min(8_000_000 / size_of::<T>() as u64))
        .max(48);
    exact_array_bytes::<T>(elements)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SqlExtentKind {
    V1Event,
    V1Head,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScalarCellExtent {
    TextBytes(i64),
    Null,
    InvalidStorage,
}

/// Finite scalar-only carrier. Slots are in owned-field order, not a SQL row or
/// decoded DTO. Unused slots must be Null; arity and nullability are checked.
pub(super) struct ScalarExtentRow {
    pub(super) kind: SqlExtentKind,
    pub(super) arity: u8,
    pub(super) cells: [ScalarCellExtent; 6],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScalarExtentFailure {
    WrongKind,
    WrongArity,
    InvalidStorage,
    RequiredNull,
    NegativeLength,
    Overflow,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ScalarExtentSummary {
    pub(super) rows: u64,
    pub(super) text_bytes: u64,
    pub(super) max_cell_bytes: u64,
}

pub(super) struct ScalarExtentAccumulator {
    kind: SqlExtentKind,
    summary: ScalarExtentSummary,
    first_failure: Option<ScalarExtentFailure>,
}
impl ScalarExtentAccumulator {
    pub(super) fn new(kind: SqlExtentKind) -> Self {
        Self {
            kind,
            summary: ScalarExtentSummary {
                rows: 0,
                text_bytes: 0,
                max_cell_bytes: 0,
            },
            first_failure: None,
        }
    }

    pub(super) fn observe(&mut self, row: ScalarExtentRow) -> Result<(), ScalarExtentFailure> {
        if let Some(failure) = self.first_failure {
            return Err(failure);
        }
        match self.fold_row(row) {
            Ok(summary) => {
                self.summary = summary;
                Ok(())
            }
            Err(failure) => {
                self.first_failure = Some(failure);
                Err(failure)
            }
        }
    }

    fn fold_row(&self, row: ScalarExtentRow) -> Result<ScalarExtentSummary, ScalarExtentFailure> {
        if row.kind != self.kind {
            return Err(ScalarExtentFailure::WrongKind);
        }
        let arity = match self.kind {
            SqlExtentKind::V1Event => 6,
            SqlExtentKind::V1Head => 3,
        };
        if usize::from(row.arity) != arity
            || row.cells[arity..]
                .iter()
                .any(|c| *c != ScalarCellExtent::Null)
        {
            return Err(ScalarExtentFailure::WrongArity);
        }
        let mut result = self.summary;
        for (index, cell) in row.cells[..arity].iter().enumerate() {
            let bytes = match cell {
                ScalarCellExtent::TextBytes(bytes) => {
                    u64::try_from(*bytes).map_err(|_| ScalarExtentFailure::NegativeLength)?
                }
                ScalarCellExtent::Null if self.kind == SqlExtentKind::V1Event && index >= 4 => 0,
                ScalarCellExtent::Null => return Err(ScalarExtentFailure::RequiredNull),
                ScalarCellExtent::InvalidStorage => {
                    return Err(ScalarExtentFailure::InvalidStorage)
                }
            };
            result.text_bytes = result
                .text_bytes
                .checked_add(bytes)
                .ok_or(ScalarExtentFailure::Overflow)?;
            result.max_cell_bytes = result.max_cell_bytes.max(bytes);
        }
        result.rows = result
            .rows
            .checked_add(1)
            .ok_or(ScalarExtentFailure::Overflow)?;
        Ok(result)
    }

    pub(super) fn finish(self) -> Result<ScalarExtentSummary, ScalarExtentFailure> {
        self.first_failure.map_or(Ok(self.summary), Err)
    }
}

#[cfg(test)]
#[path = "global_schema_replay_work_v1_tests.rs"]
mod tests;
