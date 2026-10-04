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
    CalendarCold,
    CalendarQuery,
    TransitionText,
    TransitionCollection,
    TransitionSort,
    HistoryCollection,
    HistoryText,
    HistoryRawRow,
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
    CalendarQualification(ReplayCalendarQualificationFailure),
    TransitionQualification(ReplayTransitionQualificationFailure),
    HistoryQualification(ReplayHistoryQualificationFailure),
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
    #[cfg(test)]
    transition_fault: Option<TransitionFixtureFault>,
    #[cfg(test)]
    transition_entries: [usize; 11],
    #[cfg(test)]
    history_entries: [usize; 7],
    #[cfg(test)]
    history_boundary_entries: [usize; 7],
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
            #[cfg(test)]
            transition_fault: None,
            #[cfg(test)]
            transition_entries: [0; 11],
            #[cfg(test)]
            history_entries: [0; 7],
            #[cfg(test)]
            history_boundary_entries: [0; 7],
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
            #[cfg(test)]
            transition_fault: None,
            #[cfg(test)]
            transition_entries: [0; 11],
            #[cfg(test)]
            history_entries: [0; 7],
            #[cfg(test)]
            history_boundary_entries: [0; 7],
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
        Ok(ReplayMemory {
            work: self,
            pin,
            calendar_payment: paid_calendar::CalendarPaymentState::unpaid(),
        })
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

// Only G's fixed owning carriers may use the unqualified sibling-field port.
pub(super) use sql_rows::original_native::{
    FixedDrainLedger, NativeOriginalOwner, OriginalOwnerFields, OriginalInitialReadLoan,
    OriginalIntegrityReadLoan, OriginalCapturePrefixLoan, OriginalCompileOptionsLoan,
};
#[cfg(test)]
pub(super) use sql_rows::original_native::{CaptureErrorCase, CaptureTerminalCut, LifecycleA00Case, LifecycleConstructorCase};

#[path = "global_schema_replay_calendar_v1.rs"]
mod paid_calendar;
pub(crate) use paid_calendar::{
    CalendarCallPermit, CalendarRequest, CalendarResponse, ReplayCalendarCallFailure,
    ReplayCalendarQualificationFailure,
};

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
    use super::{
        LayoutPinRefusal,
        ReplayCalendarQualificationFailure,
        ReplayCodecFailureKind,
        ReplayTransitionQualificationFailure,
        ReplayHistoryQualificationFailure
    };

    pub(crate) struct ReviewedLayoutPin {
        consumer_seed_sha256: [u8; 32],
        proof_rules_sha256: [u8; 32],
        rules: Option<ReviewedRulesV1>,
        // Independent selected once_cell/std/URL/input proof; codec rules do not suffice.
        calendar_rules: Option<ReviewedCalendarRulesV1>,
        transition_rules: Option<ReviewedTransitionRulesV1>,
        history_rules: Option<ReviewedHistoryRulesV1>,
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum ReviewedRulesV1 {
        // Future issuance must bind actual serde_json default/std source/cfg.
        FiniteDtoV1,
    }
    pub(super) enum ReviewedCalendarRulesV1 {
        CalendarColdWaitStdAppleV1,
    }
    pub(super) fn require_calendar_rules(
        rules: Option<&ReviewedCalendarRulesV1>,
    ) -> Result<&ReviewedCalendarRulesV1, ReplayCalendarQualificationFailure> {
        rules.ok_or(ReplayCalendarQualificationFailure::RuleUnavailable)
    }
    impl ReviewedLayoutPin {
        pub(super) fn calendar_rules(
            &self,
        ) -> Result<&ReviewedCalendarRulesV1, ReplayCalendarQualificationFailure> {
            require_calendar_rules(self.calendar_rules.as_ref())
        }
        pub(super) fn codec_rules(&self) -> Result<&ReviewedRulesV1, ReplayCodecFailureKind> {
            self.rules
                .as_ref()
                .ok_or(ReplayCodecFailureKind::UnsupportedSerdeProfile)
        }
    }
    pub(super) enum ReviewedTransitionRulesV1 { TransitionCollectionsStdV1 }
    impl ReviewedLayoutPin {
        pub(super) fn transition_rules(&self)->Result<&ReviewedTransitionRulesV1, ReplayTransitionQualificationFailure>{
            self.transition_rules.as_ref().ok_or(ReplayTransitionQualificationFailure::RuleUnavailable)
        }
    }
    // Independent default/std Raw15, collection, Chrono and error recipes.
    // A DTO/calendar/transition rule cannot issue this applicability proof.
    pub(super) enum ReviewedHistoryRulesV1 {
        FinancialHistoryCollectionsV1,
    }
    impl ReviewedLayoutPin {
        pub(super) fn history_rules(&self) -> Result<&ReviewedHistoryRulesV1, ReplayHistoryQualificationFailure> {
            self.history_rules.as_ref().ok_or(ReplayHistoryQualificationFailure::RuleUnavailable)
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
    calendar_payment: paid_calendar::CalendarPaymentState,
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

// The mechanics core owns no qualification. Only the genuine loan supplies it
// in production; the fixed cfg(test) dispatcher below can exercise this same core.
pub(crate) struct CodecMechanics<'loan, 'pool> {
    work: &'loan mut BorrowedReplayWork<'pool>,
    #[cfg(test)]
    pub(crate) hits: CodecHits,
    #[cfg(test)]
    fault: Option<crate::trading::paper_replay_codec_v1::SeedFaultCase>,
    #[cfg(test)]
    pub(crate) scratch: crate::trading::paper_replay_shapes_v1::ScratchObservation,
}
#[cfg(test)]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodecHits {
    pub(crate) frames: usize,
    pub(crate) decoder: usize,
    pub(crate) scratch_plans: usize,
    pub(crate) escrows: usize,
    pub(crate) escrow_bytes: u64,
    pub(crate) unit_maps: usize,
    pub(crate) active_seeds: usize,
    pub(crate) max_active_seeds: usize,
    pub(crate) denial_depth: usize,
    pub(crate) denial_strings: usize,
    pub(crate) denial_vectors: usize,
    pub(crate) denial_maps: usize,
    pub(crate) denial_units: usize,
    pub(crate) strings: usize,
    pub(crate) vectors: usize,
    pub(crate) maps: usize,
    pub(crate) outputs: usize,
}
impl<'loan, 'pool> ReplayMemory<'loan, 'pool> {
    pub(crate) fn mechanics<'short>(
        &'short mut self,
    ) -> Result<CodecMechanics<'short, 'pool>, ReplayTerminalFailure> {
        self.finish()?;
        if let Err(kind) = self.rules() {
            return Err(self.work.qualify_codec(kind, None));
        }
        Ok(CodecMechanics {
            work: self.work,
            #[cfg(test)]
            hits: CodecHits::default(),
            #[cfg(test)]
            fault: None,
            #[cfg(test)]
            scratch: Default::default(),
        })
    }
}
impl CodecMechanics<'_, '_> {
    pub(crate) fn finish(&self) -> Result<(), ReplayTerminalFailure> {
        self.work.finish()
    }
    pub(crate) fn refuse(
        &mut self,
        kind: ReplayCodecFailureKind,
        offset: Option<u64>,
    ) -> ReplayTerminalFailure {
        self.work.qualify_codec(kind, offset)
    }
    fn layout_error(&mut self, site: ReplaySite, error: LayoutFailure) -> ReplayTerminalFailure {
        self.work.fail(site, ResourceCause::Layout(error))
    }
    pub(crate) fn scratch_layout_failure(&mut self) -> ReplayTerminalFailure {
        self.layout_error(ReplaySite::CodecScratch, LayoutFailure::Overflow)
    }
    fn allocation_error(&mut self, site: ReplaySite) -> ReplayTerminalFailure {
        self.work.fail(site, ResourceCause::AllocationFailed)
    }
    pub(crate) fn frames(
        &mut self,
        count: usize,
    ) -> Result<Vec<crate::trading::paper_replay_shapes_v1::ScanFrame>, ReplayTerminalFailure> {
        use crate::trading::paper_replay_shapes_v1::ScanFrame;
        self.work
            .reserve_array::<ScanFrame>(ReplaySite::CodecScratch, count as u64)?
            .consume();
        #[cfg(test)]
        {
            self.hits.frames += 1;
        }
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(count)
            .map_err(|_| self.allocation_error(ReplaySite::CodecScratch))?;
        Ok(frames)
    }
    pub(crate) fn decode_escrow(
        &mut self,
        root: crate::trading::paper_replay_shapes_v1::RootKind,
    ) -> Result<(), ReplayTerminalFailure> {
        let bytes = serde_error_box_bound()
            .and_then(|e| add(mul(root.q() + 2, e.bytes())?, 88))
            .map_err(|e| self.layout_error(ReplaySite::ErrorStorage, e))?;
        self.work
            .reserve(ReplaySite::ErrorStorage, bytes)?
            .consume();
        #[cfg(test)]
        {
            self.hits.escrows += 1;
            self.hits.escrow_bytes += bytes;
        }
        Ok(())
    }
    pub(crate) fn decoder_scratch(
        &mut self,
        trace: &crate::trading::paper_replay_shapes_v1::ScratchTrace,
    ) -> Result<(), ReplayTerminalFailure> {
        self.work
            .reserve(ReplaySite::CodecScratch, trace.requested())?
            .consume();
        #[cfg(test)]
        {
            self.hits.scratch_plans += 1;
            self.scratch = trace.observation();
        }
        Ok(())
    }
    pub(crate) fn serializer_escrow(&mut self) -> Result<(), ReplayTerminalFailure> {
        let bytes = serde_error_box_bound()
            .map_err(|e| self.layout_error(ReplaySite::ErrorStorage, e))?
            .bytes();
        self.work
            .reserve(ReplaySite::ErrorStorage, bytes)?
            .consume();
        Ok(())
    }
    pub(crate) fn string(&mut self, value: &str) -> Result<String, ReplayTerminalFailure> {
        #[cfg(test)]
        self.fixture_deplete(
            crate::trading::paper_replay_codec_v1::SeedFaultCase::StringAfterUnit,
            value.len() as u64,
            self.hits.unit_maps > 0,
        )?;
        self.work
            .reserve_array::<u8>(ReplaySite::StateCopy, value.len() as u64)?
            .consume();
        #[cfg(test)]
        {
            self.hits.strings += 1;
            self.work.history_entries[6] += 1;
        }
        let mut result = String::new();
        result
            .try_reserve_exact(value.len())
            .map_err(|_| self.allocation_error(ReplaySite::StateCopy))?;
        result.push_str(value);
        Ok(result)
    }
    pub(crate) fn vector<T: crate::trading::paper_replay_codec_v1::ArrayElement>(
        &mut self,
        count: usize,
    ) -> Result<Vec<T>, ReplayTerminalFailure> {
        #[cfg(test)]
        {
            let cost = exact_array_bytes::<T>(count as u64)
                .map_err(|e| self.layout_error(ReplaySite::Collection, e))?;
            self.fixture_deplete(
                crate::trading::paper_replay_codec_v1::SeedFaultCase::Vector,
                cost,
                true,
            )?;
        }
        self.work
            .reserve_array::<T>(ReplaySite::Collection, count as u64)?
            .consume();
        #[cfg(test)]
        {
            self.hits.vectors += 1;
        }
        let mut result = Vec::new();
        result
            .try_reserve_exact(count)
            .map_err(|_| self.allocation_error(ReplaySite::Collection))?;
        Ok(result)
    }
    pub(crate) fn insert<K: Ord, V>(
        &mut self,
        map: &mut std::collections::BTreeMap<K, V>,
        key: K,
        value: V,
    ) -> Result<(), ReplayTerminalFailure>
    where
        (K, V): crate::trading::paper_replay_codec_v1::MapEntry,
    {
        let n = map.len() as u64;
        let bytes = (|| {
            let next = add(n, 1)?;
            let height = if next <= 1 {
                0
            } else {
                u64::from(u64::BITS - (next - 1).leading_zeros())
            };
            let (_, internal) = btree_node_bounds::<K, V>()?;
            mul(add(height, 2)?, internal.bytes())
        })()
        .map_err(|e| self.layout_error(ReplaySite::Collection, e))?;
        #[cfg(test)]
        self.fixture_deplete(
            crate::trading::paper_replay_codec_v1::SeedFaultCase::Map,
            bytes,
            true,
        )?;
        self.work.reserve(ReplaySite::Collection, bytes)?.consume();
        #[cfg(test)]
        {
            self.hits.maps += 1;
        }
        map.insert(key, value);
        Ok(())
    }
    pub(crate) fn output(&mut self, count: usize) -> Result<Vec<u8>, ReplayTerminalFailure> {
        self.work .reserve_array::<u8>(ReplaySite::Formatting, count as u64)? .consume();
        #[cfg(test)]
        {
            self.hits.outputs += 1;
            self.work.transition_entries[10] += 1;
        }
        let mut result = Vec::new();
        result .try_reserve_exact(count) .map_err(|_| self.allocation_error(ReplaySite::Formatting))?;
        Ok(result)
    }
    pub(crate) fn hex_digest(
        &mut self,
        digest: &[u8; 32],
    ) -> Result<String, ReplayTerminalFailure> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut bytes = [0u8; 64];
        for (i, byte) in digest.iter().enumerate() {
            bytes[2 * i] = HEX[(byte >> 4) as usize];
            bytes[2 * i + 1] = HEX[(byte & 15) as usize];
        }
        self.string(std::str::from_utf8(&bytes).expect("hex is ASCII"))
    }
    #[cfg(test)]
    fn fixture_deplete(
        &mut self,
        site: crate::trading::paper_replay_codec_v1::SeedFaultCase,
        cost: u64,
        enabled: bool,
    ) -> Result<(), ReplayTerminalFailure> {
        self.finish()?;
        if self.fault == Some(site)
            && enabled
            && cost > 0
            && self.hits.active_seeds > 0
            && self.hits.strings > 0
        {
            self.fault = None;
            let leave = cost - 1;
            let debit = (16 * 1024 * 1024_u64)
                .checked_sub(self.used())
                .unwrap()
                .checked_sub(leave)
                .unwrap();
            self.work.reserve(ReplaySite::StateCopy, debit)?.consume();
            self.hits.denial_depth = self.hits.active_seeds;
            self.hits.denial_strings = self.hits.strings;
            self.hits.denial_vectors = self.hits.vectors;
            self.hits.denial_maps = self.hits.maps;
            self.hits.denial_units = self.hits.unit_maps;
        }
        Ok(())
    }
    // Fixed test-only denied one-byte request while the second (disposition)
    // empty-unit-map visitor and its eight real serde container frames are live.
    #[cfg(test)]
    pub(crate) fn fixture_unit_failure(&mut self) -> Result<(), ReplayTerminalFailure> {
        use crate::trading::paper_replay_codec_v1::SeedFaultCase;
        self.finish()?;
        if self.fault == Some(SeedFaultCase::UnitMap) && self.hits.unit_maps == 2 {
            self.fault = None;
            let debit=16*1024*1024-self.used();
            self.work.reserve(ReplaySite::StateCopy,debit)?.consume();
            self.hits.denial_depth = self.hits.active_seeds;
            self.hits.denial_strings = self.hits.strings;
            self.hits.denial_vectors = self.hits.vectors;
            self.hits.denial_maps = self.hits.maps;
            self.hits.denial_units = self.hits.unit_maps;
            self.string("x")?;
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn seeded_failure_matches(
        &self,
        kind: crate::trading::paper_replay_codec_v1::SeedFaultCase,
    ) -> bool {
        use crate::trading::paper_replay_codec_v1::SeedFaultCase as F;
        let site = match kind {
            F::UnitMap | F::StringAfterUnit => ReplaySite::StateCopy,
            F::Vector | F::Map => ReplaySite::Collection,
        };
        matches!(self.finish(),Err(ReplayTerminalFailure::Resource(f)) if f.site==site)
    }
    #[cfg(test)]
    pub(crate) fn failure_kind(&self) -> Option<ReplayCodecFailureKind> {
        match self.finish().err() {
            Some(ReplayTerminalFailure::CodecQualification(f)) => Some(f.kind),
            _ => None,
        }
    }
    #[cfg(test)]
    pub(crate) fn used(&self) -> u64 {
        self.work.used()
    }
}

#[cfg(test)]
pub(crate) fn codec_fixture(case: crate::trading::paper_replay_codec_v1::CodecFixtureCase) {
    // No caller limit/callback or pin construction. Repeated work inside a case
    // retains this one real meter and the actual target terminal.
    let mut metadata = RowsSpecWork::new(16 * 1024 * 1024, 1, 1);
    let mut terminal = super::target::test_replay_terminal();
    let mut work = BorrowedReplayWork::test_borrow(&mut metadata, &mut terminal);
    if matches!(
        case,
        crate::trading::paper_replay_codec_v1::CodecFixtureCase::Qualification
    ) {
        assert!(work.codec_memory().is_err());
        assert!(work.finish().is_err());
        return;
    }
    let mut core = CodecMechanics {
        work: &mut work,
        hits: CodecHits::default(),
        fault: if let crate::trading::paper_replay_codec_v1::CodecFixtureCase::SeedFault(kind) =
            case
        {
            Some(kind)
        } else {
            None
        },
        scratch: Default::default(),
    };
    if let crate::trading::paper_replay_codec_v1::CodecFixtureCase::Boundary(kind) = case {
        codec_boundary(kind, &mut core);
        return;
    }
    crate::trading::paper_replay_codec_v1::run_fixture(case, &mut core);
    if matches!(
        case,
        crate::trading::paper_replay_codec_v1::CodecFixtureCase::SeedFault(
            crate::trading::paper_replay_codec_v1::SeedFaultCase::StringAfterUnit
                | crate::trading::paper_replay_codec_v1::SeedFaultCase::UnitMap
        )
    ) {
        assert_eq!(core.hits.escrows, 1);
        assert_eq!(
            core.hits.escrow_bytes,
            10 * serde_error_box_bound().unwrap().bytes() + 88
        );
        assert!(core.hits.unit_maps > 0);
    }
}

#[cfg(test)]
fn codec_boundary(
    case: crate::trading::paper_replay_codec_v1::BoundaryCase,
    core: &mut CodecMechanics<'_, '_>,
) {
    use crate::trading::paper_replay_codec_v1::BoundaryCase as B;
    use crate::trading::paper_replay_shapes_v1::{RootKind, ScanFrame};
    if matches!(case, B::LengthOverflow) {
        assert!(matches!(
            core.frames(usize::MAX),
            Err(ReplayTerminalFailure::Resource(_))
        ));
        assert_eq!(core.hits.frames, 0);
        assert_eq!(core.used(), 0);
        let first = core.finish().unwrap_err();
        assert_eq!(core.frames(1).err(), Some(first));
        assert_eq!(core.hits.frames, 0);
        assert_eq!(core.used(), 0);
        return;
    }
    let e = serde_error_box_bound().unwrap().bytes();
    let cost = match case {
        B::FramesExact | B::FramesShort => 3 * std::mem::size_of::<ScanFrame>() as u64,
        B::Escrow8Exact | B::Escrow8Short => 10 * e + 88,
        B::Escrow4Exact | B::Escrow4Short => 6 * e + 88,
        B::ScratchExact | B::ScratchShort => 135,
        B::VectorExact | B::VectorShort => 2 * std::mem::size_of::<String>() as u64,
        B::MapExact | B::MapShort => 2 * btree_node_bounds::<String, String>().unwrap().1.bytes(),
        B::OutputExact | B::OutputShort => 47,
        B::LengthOverflow => unreachable!(),
    };
    let short = matches!(
        case,
        B::FramesShort
            | B::Escrow8Short
            | B::Escrow4Short
            | B::ScratchShort
            | B::VectorShort
            | B::MapShort
            | B::OutputShort
    );
    let remaining = cost - u64::from(short);
    core.work
        .reserve(ReplaySite::StateCopy, 16 * 1024 * 1024 - remaining)
        .unwrap()
        .consume();
    let before = core.used();
    let hits = core.hits;
    let result = match case {
        B::FramesExact | B::FramesShort => core.frames(3).map(|v| assert!(v.capacity() >= 3)),
        B::Escrow8Exact | B::Escrow8Short => core.decode_escrow(RootKind::ExecutionFact),
        B::Escrow4Exact | B::Escrow4Short => core.decode_escrow(RootKind::ExecutionManifest),
        B::ScratchExact | B::ScratchShort => {
            let t = crate::trading::paper_replay_shapes_v1::fixed_scratch_trace();
            assert_eq!(t.requested(), 135);
            core.decoder_scratch(&t)
        }
        B::VectorExact | B::VectorShort => {
            core.vector::<String>(2).map(|v| assert!(v.capacity() >= 2))
        }
        B::MapExact | B::MapShort => {
            let mut map = std::collections::BTreeMap::new();
            let r = core.insert(&mut map, String::new(), String::new());
            assert_eq!(map.len(), usize::from(!short));
            r
        }
        B::OutputExact | B::OutputShort => core.output(47).map(|v| assert!(v.capacity() >= 47)),
        B::LengthOverflow => unreachable!(),
    };
    if short {
        let site = match case {
            B::FramesShort | B::ScratchShort => ReplaySite::CodecScratch,
            B::Escrow8Short | B::Escrow4Short => ReplaySite::ErrorStorage,
            B::VectorShort | B::MapShort => ReplaySite::Collection,
            B::OutputShort => ReplaySite::Formatting,
            _ => unreachable!("fixed short case"),
        };
        // The original meter keeps the attempted debit before reporting Exceeded.
        // The denied owned operation does not run; its budget debit is not refunded.
        let attempted = before + cost;
        assert_eq!(attempted, 16 * 1024 * 1024 + 1);
        let expected = ReplayTerminalFailure::Resource(ReplayResourceFailure {
            site,
            cause: ResourceCause::Debit(RowsSpecDebitFailure::Exceeded),
            used: attempted,
        });
        assert_eq!(result, Err(expected));
        assert_eq!(core.finish(), Err(expected));
        assert_eq!(core.used(), attempted);
        assert_eq!(core.hits, hits);
    } else {
        result.unwrap();
        assert_eq!(core.used() - before, cost);
        assert_eq!(core.used(), 16 * 1024 * 1024);
        assert!(core.finish().is_ok());
    }
    let h = core.hits;
    let latched = core.finish().err();
    let first = core.output(1).unwrap_err();
    if let Some(previous) = latched {
        assert_eq!(first, previous, "retry preserves the original denied site");
    } else {
        assert_eq!(
            first,
            ReplayTerminalFailure::Resource(ReplayResourceFailure {
                site: ReplaySite::Formatting,
                cause: ResourceCause::Debit(RowsSpecDebitFailure::Exceeded),
                used: 16 * 1024 * 1024 + 1,
            })
        );
    }
    assert_eq!(core.finish(), Err(first));
    assert_eq!(core.used(), 16 * 1024 * 1024 + 1);
    assert_eq!(core.output(1).err(), Some(first));
    assert_eq!(core.used(), 16 * 1024 * 1024 + 1);
    assert_eq!(core.hits, h);
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

// Closed transition requests borrow the same persistent metadata and first failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplayTransitionQualificationFailure {
    RuleUnavailable,
    WriterMismatch
}
pub(crate) struct TransitionOps<'loan, 'pool>{
    work:&'loan mut BorrowedReplayWork<'pool>
}
impl<'loan, 'pool> ReplayMemory<'loan, 'pool>{
    pub(crate) fn transition_ops<'short>(&'short mut self)->Result<TransitionOps<'short,
    'pool>,
    ReplayTerminalFailure>{
        self.finish()?;
        self.pin.transition_rules().map_err(|e|self.work.terminal.latch(ReplayTerminalFailure::TransitionQualification(e)))?;
        Ok(TransitionOps{
            work:self.work
        })
    }
}
impl TransitionOps<'_, '_>{
    #[cfg(test)]
    fn boundary(&mut self, fault:TransitionFixtureFault, cost:u64)->Result<(),
    ReplayTerminalFailure>{
        self.finish()?;
        let selected = self.work.transition_fault;
        let exact = matches!((selected, fault), (Some(TransitionFixtureFault::NodeExact), TransitionFixtureFault::Node) | (Some(TransitionFixtureFault::GrowExact), TransitionFixtureFault::Grow) | (Some(TransitionFixtureFault::SortExact), TransitionFixtureFault::Sort) | (Some(TransitionFixtureFault::TextExact), TransitionFixtureFault::Text));
        if cost > 0 && (selected == Some(fault) || exact) {
            self.work.transition_fault = None;
            let remaining = 16 * 1024 * 1024_u64 - self.work.used();
            let available = if exact {
                cost
            } else {
                cost - 1
            };
            let prefix = remaining.checked_sub(available).expect("fixed lower boundary fits");
            // Deliberate test debt, not evidence of an owned financial prefix.
            self.work.reserve(ReplaySite::StateCopy, prefix)?.consume();
        }
        Ok(())
    }
    pub(crate) fn finish(&self)->Result<(),
    ReplayTerminalFailure>{
        self.work.finish()
    }
    fn node<K,
    V>(&mut self, len:usize)->Result<(),
    ReplayTerminalFailure>{
        let bytes=(||{
            let n=add(len as u64, 1)?;
            let height=if n<=1{
                0
            } else{
                u64::from(u64::BITS-(n-1).leading_zeros())
            };
            let(_, node)=btree_node_bounds::<K,
            V>()?;
            mul(add(height, 2)?, node.bytes())
        })();
        let bytes=bytes.map_err(|e|self.work.fail(ReplaySite::TransitionCollection, ResourceCause::Layout(e)))?;
        #[cfg(test)]
        self.boundary(TransitionFixtureFault::Node, bytes)?;
        self.work.reserve(ReplaySite::TransitionCollection, bytes)?.consume();
        Ok(())
    }
    pub(crate) fn str_set_insert<'a>(&mut self, set:&mut std::collections::BTreeSet<&'a str>, value:&'a str)->Result<bool,
    ReplayTerminalFailure>{
        self.finish()?;
        self.node::<&str,
        ()>(set.len())?;
        #[cfg(test)]
        {
            self.work.transition_entries[0] += 1;
        }
        Ok(set.insert(value))
    }
    pub(crate) fn lot_ref_insert<'a>(&mut self, map:&mut std::collections::BTreeMap<&'a str, &'a crate::trading::paper_ledger::Lot>, key:&'a str, value:&'a crate::trading::paper_ledger::Lot)->Result<Option<&'a crate::trading::paper_ledger::Lot>,
    ReplayTerminalFailure>{
        self.finish()?;
        self.node::<&str,
        &crate::trading::paper_ledger::Lot>(map.len())?;
        #[cfg(test)]
        {
            self.work.transition_entries[1] += 1;
        }
        Ok(map.insert(key, value))
    }
    pub(crate) fn descriptor_field_insert<'a>(&mut self, map:&mut std::collections::BTreeMap<&'a str, &'a str>, key:&'a str, value:&'a str)->Result<Option<&'a str>,
    ReplayTerminalFailure>{
        self.finish()?;
        self.node::<&str,
        &str>(map.len())?;
        #[cfg(test)]
        {
            self.work.transition_entries[2] += 1;
        }
        Ok(map.insert(key, value))
    }
    pub(crate) fn claim_slot<'m,
    'a>(&mut self, map:&'m mut std::collections::BTreeMap<&'a str, u32>, key:&'a str)->Result<&'m mut u32,
    ReplayTerminalFailure>{
        self.finish()?;
        self.node::<&str,
        u32>(map.len())?;
        #[cfg(test)]
        {
            self.work.transition_entries[3] += 1;
        }
        Ok(map.entry(key).or_default())
    }
    pub(crate) fn exposure_slot<'m,
    'a>(&mut self, map:&'m mut std::collections::BTreeMap<&'a str, i128>, key:&'a str)->Result<&'m mut i128,
    ReplayTerminalFailure>{
        self.finish()?;
        self.node::<&str,
        i128>(map.len())?;
        #[cfg(test)]
        {
            self.work.transition_entries[4] += 1;
        }
        Ok(map.entry(key).or_default())
    }
    pub(crate) fn push_transition<T:crate::trading::paper_replay_financial_work_v1::TransitionElement>(&mut self, vec:&mut Vec<T>, value:T)->Result<(),
    ReplayTerminalFailure>{
        self.finish()?;
        if vec.len()==vec.capacity(){
            let required=vec.len().checked_add(1).ok_or_else(||self.work.fail(ReplaySite::TransitionCollection, ResourceCause::Layout(LayoutFailure::Overflow)))?;
            let bytes=amortized_vector_bytes::<T>(vec.capacity() as u64, required as u64).map_err(|e|self.work.fail(ReplaySite::TransitionCollection, ResourceCause::Layout(e)))?;
            #[cfg(test)]
            self.boundary(TransitionFixtureFault::Grow, bytes)?;
            self.work.reserve(ReplaySite::TransitionCollection, bytes)?.consume();
            #[cfg(test)]
            {
                self.work.transition_entries[5] += 1;
            }
            vec.try_reserve(1).map_err(|_|self.work.fail(ReplaySite::TransitionCollection, ResourceCause::AllocationFailed))?;
        }
        #[cfg(test)]
        {
            self.work.transition_entries[6] += 1;
        }
        vec.push(value);
        Ok(())
    }
    pub(crate) fn sort_fifo_lots(&mut self, lots:&mut Vec<&crate::trading::paper_ledger::Lot>)->Result<(),
    ReplayTerminalFailure>{
        self.finish()?;
        let bytes=stable_sort_scratch_bytes::<&crate::trading::paper_ledger::Lot>(lots.len() as u64).map_err(|e|self.work.fail(ReplaySite::TransitionSort, ResourceCause::Layout(e)))?;
        #[cfg(test)]
        self.boundary(TransitionFixtureFault::Sort, bytes)?;
        self.work.reserve(ReplaySite::TransitionSort, bytes)?.consume();
        #[cfg(test)]
        {
            self.work.transition_entries[7] += 1;
        }
        crate::trading::paper_book_v2_execution::sort_fifo_lots_owner(lots);
        Ok(())
    }
    pub(crate) fn financial_output(&mut self, plan:crate::trading::paper_replay_financial_work_v1::FinancialOutput<'_>)->Result<Vec<u8>,
    ReplayTerminalFailure>{
        use crate::trading::paper_replay_financial_work_v1::FinancialSink;
        self.finish()?;
        let mut count=FinancialSink::Count(0);
        plan.write(&mut count).map_err(|_|self.work.fail(ReplaySite::TransitionText, ResourceCause::Layout(LayoutFailure::Overflow)))?;
        let n=count.count();
        #[cfg(test)]
        self.boundary(TransitionFixtureFault::Text, n as u64)?;
        self.work.reserve_array::<u8>(ReplaySite::TransitionText, n as u64)?.consume();
        #[cfg(test)]
        {
            self.work.transition_entries[8] += 1;
        }
        let mut bytes=Vec::new();
        bytes.try_reserve_exact(n).map_err(|_|self.work.fail(ReplaySite::TransitionText, ResourceCause::AllocationFailed))?;
        let limit=n;
        #[cfg(test)]
        let limit=if self.work.transition_fault==Some(TransitionFixtureFault::WriterMismatch){
            self.work.transition_fault=None;
            n.saturating_sub(1)
        } else{
            limit
        };
        #[cfg(test)]
        {
            self.work.transition_entries[9] += 1;
        }
        if plan.write(&mut FinancialSink::Output{
            bytes:&mut bytes,
            limit
        }).is_err()||bytes.len()!=n{
            return Err(self.work.terminal.latch(ReplayTerminalFailure::TransitionQualification(ReplayTransitionQualificationFailure::WriterMismatch)));
        }
        Ok(bytes)
    }
}
// No successful pin or ReplayMemory is constructed by lower fixtures.
#[cfg(test)]
pub(crate) struct FinancialFixtureLoan<'loan, 'pool>{
    work:&'loan mut BorrowedReplayWork<'pool>,
    calendar_payment:paid_calendar::CalendarPaymentState,
    pub(crate) hash_hits:[usize;4],
    case: crate::trading::paper_replay_transition_v1_tests::Case,
}
#[cfg(test)]
impl<'loan, 'pool> FinancialFixtureLoan<'loan, 'pool>{
    pub(crate) fn finish(&self)->Result<(),
    ReplayTerminalFailure>{
        self.work.finish()
    }
    pub(crate) fn mechanics(&mut self)->Result<CodecMechanics<'_,
    'pool>,
    ReplayTerminalFailure>{
        self.finish()?;
        Ok(CodecMechanics{
            work:self.work,
            hits:Default::default(),
            fault:None,
            scratch:Default::default()
        })
    }
    pub(crate) fn transition_ops(&mut self)->Result<TransitionOps<'_,
    'pool>,
    ReplayTerminalFailure>{
        self.finish()?;
        Ok(TransitionOps{
            work:self.work
        })
    }
    pub(crate) fn decode<T:crate::trading::paper_replay_codec_v1::Root>(&mut self, bytes:&[u8])->Result<T,
    ReplayTerminalFailure>{
        use crate::trading::paper_replay_codec_v1 as c;
        let mut m=self.mechanics()?;
        let value=c::decode_core::<T>(bytes, &mut m)?;
        if T::CANONICAL&&c::encode_core(&value, &mut m)?!=bytes{
            return Err(m.refuse(ReplayCodecFailureKind::Noncanonical, None));
        }
        Ok(value)
    }
    pub(crate) fn calendar_day(&mut self, day:chrono::NaiveDate)->Result<bool,
    ReplayCalendarCallFailure>{
        match paid_calendar::fixture_call_paid(self.work, &mut self.calendar_payment, CalendarRequest::Day(day))?{
            CalendarResponse::Day(v)=>Ok(v),
            _=>unreachable!()
        }
    }
    pub(crate) fn calendar_prev(&mut self, day:chrono::NaiveDate)->Result<chrono::NaiveDate,
    ReplayCalendarCallFailure>{
        match paid_calendar::fixture_call_paid(self.work, &mut self.calendar_payment, CalendarRequest::Prev(day))?{
            CalendarResponse::Date(v)=>Ok(v),
            _=>unreachable!()
        }
    }
    pub(crate) fn calendar_next(&mut self, day:chrono::NaiveDate)->Result<chrono::NaiveDate,
    ReplayCalendarCallFailure>{
        match paid_calendar::fixture_call_paid(self.work, &mut self.calendar_payment, CalendarRequest::Next(day))?{
            CalendarResponse::Date(v)=>Ok(v),
            _=>unreachable!()
        }
    }
}
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum TransitionFixtureFault{
    Node,
    Grow,
    Sort,
    Text,
    NodeExact,
    GrowExact,
    SortExact,
    TextExact,
    WriterMismatch
}
#[cfg(test)]
impl FinancialFixtureLoan<'_, '_>{
    pub(crate) fn used(&self)->u64{
        self.work.used()
    }
    pub(crate) fn entries(&self)->[usize;11]{
        self.work.transition_entries
    }
}
#[cfg(test)]
pub(crate) fn financial_fixture(case:crate::trading::paper_replay_transition_v1_tests::Case){
    use crate::trading::paper_replay_transition_v1_tests::{
        self as test,
        Case
    };
    let mut metadata=RowsSpecWork::new(16*1024*1024, 1, 1);
    let mut terminal=super::target::test_replay_terminal();
    let mut work=BorrowedReplayWork::test_borrow(&mut metadata, &mut terminal);
    if matches!(case, Case::Qualification){
        let e=work.codec_memory().err().expect("ordinary refuses");
        assert_eq!(work.finish(), Err(e));
        return;
    }
    work.transition_fault=match case{
        Case::NodeShort|Case::LotMapShort|Case::DescriptorMapShort|Case::ClaimShort|Case::ExposureShort=>Some(TransitionFixtureFault::Node),
        Case::SetExact|Case::LotMapExact|Case::DescriptorMapExact|Case::ClaimExact|Case::ExposureExact=>Some(TransitionFixtureFault::NodeExact),
        Case::GrowExact=>Some(TransitionFixtureFault::GrowExact),
        Case::SortExact=>Some(TransitionFixtureFault::SortExact),
        Case::TextExact=>Some(TransitionFixtureFault::TextExact),
        Case::GrowShort=>Some(TransitionFixtureFault::Grow),
        Case::SortShort=>Some(TransitionFixtureFault::Sort),
        Case::TextShort=>Some(TransitionFixtureFault::Text),
        Case::WriterMismatch=>Some(TransitionFixtureFault::WriterMismatch),
        _=>None
    };
    {
        let loan=FinancialFixtureLoan{
            work:&mut work,
            calendar_payment:paid_calendar::CalendarPaymentState::unpaid(),
            hash_hits:[0; 4],
            case,
        };
        let mut financial=crate::trading::paper_replay_financial_work_v1::FinancialWork::Fixture(loan);
        test::run(case, &mut financial);
    }
    if matches!(case, Case::NodeShort|Case::LotMapShort|Case::DescriptorMapShort|Case::ClaimShort|Case::ExposureShort|Case::GrowShort|Case::SortShort|Case::TextShort){
        let e=work.finish().unwrap_err();
        let site=match case{
            Case::NodeShort|Case::LotMapShort|Case::DescriptorMapShort|Case::ClaimShort|Case::ExposureShort|Case::GrowShort=>ReplaySite::TransitionCollection,
            Case::SortShort=>ReplaySite::TransitionSort,
            _=>ReplaySite::TransitionText
        };
        assert_eq!(e, ReplayTerminalFailure::Resource(ReplayResourceFailure{
            site,
            cause:ResourceCause::Debit(RowsSpecDebitFailure::Exceeded),
            used:16*1024*1024+1
        }));
        assert_eq!(work.used(), 16*1024*1024+1);
    } else if matches!(case, Case::SetExact|Case::LotMapExact|Case::DescriptorMapExact|Case::ClaimExact|Case::ExposureExact|Case::GrowExact|Case::SortExact|Case::TextExact){
        assert!(matches!(work.finish(), Err(ReplayTerminalFailure::Resource(_))));
    } else if matches!(case, Case::WriterMismatch){
        assert_eq!(work.finish(), Err(ReplayTerminalFailure::TransitionQualification(ReplayTransitionQualificationFailure::WriterMismatch)));
    } else if matches!(case, Case::Cumulative|Case::OversizedResource|Case::StagedResource){
        assert!(matches!(work.finish(), Err(ReplayTerminalFailure::Resource(_))));
    } else{
        assert_eq!(work.finish(), Ok(()));
    }
}
#[cfg(test)]
pub(crate) fn financial_fixture_serializer_escrow_bytes() -> u64 {
    serde_error_box_bound().expect("reviewed pure error layout").bytes()
}
#[cfg(test)]
impl FinancialFixtureLoan<'_, '_> {
    // This is an assertion oracle only: no reserve, latch, mutation, or reset.
    pub(crate) fn assert_resource(&self, expected_used: u64) {
        use crate::trading::paper_replay_transition_v1_tests::Case;
        let site = match self.case {
            Case::OversizedResource => ReplaySite::Formatting,
            Case::SortShort | Case::SortExact => ReplaySite::TransitionSort,
            Case::TextShort | Case::TextExact => ReplaySite::TransitionText,
            Case::NodeShort | Case::SetExact | Case::LotMapShort | Case::LotMapExact | Case::DescriptorMapShort | Case::DescriptorMapExact | Case::ClaimShort | Case::ClaimExact | Case::ExposureShort | Case::ExposureExact | Case::GrowShort | Case::GrowExact => ReplaySite::TransitionCollection,
            _ => panic!("not a fixed resource boundary fixture"),
        };
        assert_eq!(self.work.used(), expected_used);
        assert_eq!(self.work.finish(), Err(ReplayTerminalFailure::Resource(ReplayResourceFailure {
            site,
            cause: ResourceCause::Debit(RowsSpecDebitFailure::Exceeded),
            used: expected_used,
        })));
    }
    // Independent fixed-case expectation from the reviewed source bounds.
    // The impending operation's private computed cost is not read here.
    pub(crate) fn expected_boundary_cost(&self) -> u64 {
        use crate::trading::paper_replay_transition_v1_tests::Case;
        match self.case {
            Case::NodeShort | Case::SetExact => 2 * btree_node_bounds::<&str,
            ()>().unwrap().1.bytes(),
            Case::LotMapShort | Case::LotMapExact => 2 * btree_node_bounds::<&str,
            &crate::trading::paper_ledger::Lot>().unwrap().1.bytes(),
            Case::DescriptorMapShort | Case::DescriptorMapExact => 2 * btree_node_bounds::<&str,
            &str>().unwrap().1.bytes(),
            Case::ClaimShort | Case::ClaimExact => 2 * btree_node_bounds::<&str,
            u32>().unwrap().1.bytes(),
            Case::ExposureShort | Case::ExposureExact => 2 * btree_node_bounds::<&str,
            i128>().unwrap().1.bytes(),
            Case::GrowShort | Case::GrowExact => exact_array_bytes::<String>(4).unwrap(),
            Case::SortShort | Case::SortExact => exact_array_bytes::<&crate::trading::paper_ledger::Lot>(48).unwrap(),
            Case::TextShort | Case::TextExact => b"paper-parent-projection/v1".len() as u64,
            _ => panic!("not a fixed boundary fixture"),
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplayHistoryQualificationFailure {
    RuleUnavailable,
    WriterMismatch,
    RowContract,
    IdentityContextUnavailable,
}
pub(crate) struct HistoryOps<'loan, 'pool> {
    work: &'loan mut BorrowedReplayWork<'pool>,
}
impl<'loan, 'pool> ReplayMemory<'loan, 'pool> {
    pub(crate) fn history_ops(&mut self) -> Result<HistoryOps<'_, 'pool>, ReplayTerminalFailure> {
        self.finish()?;
        self.pin.history_rules().map_err(|e| {
            self.work.terminal.latch(ReplayTerminalFailure::HistoryQualification(e))
        })?;
        Ok(HistoryOps {
            work: self.work
        })
    }
}
impl HistoryOps<'_, '_> {
    pub(crate) fn finish(&self) -> Result<(), ReplayTerminalFailure> {
        self.work.finish()
    }
    pub(crate) fn refuse(&mut self, kind: ReplayHistoryQualificationFailure) -> ReplayTerminalFailure {
        self.work.terminal.latch(ReplayTerminalFailure::HistoryQualification(kind))
    }
    fn layout_error(&mut self, error: LayoutFailure) -> ReplayTerminalFailure {
        self.work.fail(ReplaySite::HistoryCollection, ResourceCause::Layout(error))
    }
    fn allocation_error(&mut self, site: ReplaySite) -> ReplayTerminalFailure {
        self.work.fail(site, ResourceCause::AllocationFailed)
    }
    pub(crate) fn copy_raw_sql_result(&mut self, raw: &str) -> Result<String, ReplayTerminalFailure> {
        self.work.reserve_array::<u8>(ReplaySite::HistoryRawRow, raw.len() as u64)?.consume();
        #[cfg(test)]
        {
            self.work.history_entries[0] += 1;
        }
        let mut owned = String::new();
        owned.try_reserve_exact(raw.len()).map_err(|_| self.allocation_error(ReplaySite::HistoryRawRow))?;
        owned.push_str(raw);
        Ok(owned)
    }
    pub(crate) fn vector<T: crate::trading::paper_replay_financial_work_v1::HistoryElement>(
        &mut self, count: usize,
    ) -> Result<Vec<T>, ReplayTerminalFailure> {
        self.work.reserve_array::<T>(ReplaySite::HistoryCollection, count as u64)?.consume();
        #[cfg(test)]
        {
            self.work.history_entries[1] += 1;
        }
        let mut vector = Vec::new();
        vector.try_reserve_exact(count).map_err(|_| self.allocation_error(ReplaySite::HistoryCollection))?;
        Ok(vector)
    }
    pub(crate) fn push<T: crate::trading::paper_replay_financial_work_v1::HistoryElement>(
        &mut self, vector: &mut Vec<T>, value: T,
    ) -> Result<(), ReplayTerminalFailure> {
        self.finish()?;
        if vector.len() == vector.capacity() {
            let required = vector.len().checked_add(1).ok_or_else(|| self.layout_error(LayoutFailure::Overflow))?;
            let bytes = amortized_vector_bytes::<T>(vector.capacity() as u64, required as u64)
                .map_err(|e| self.layout_error(e))?;
            self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
            #[cfg(test)]
            {
                self.work.history_entries[1] += 1;
            }
            vector.try_reserve(1).map_err(|_| self.allocation_error(ReplaySite::HistoryCollection))?;
        }
        vector.push(value);
        Ok(())
    }
    pub(crate) fn tree_slot<'a, K: Ord, V>(
        &mut self, map: &'a mut std::collections::BTreeMap<K, V>, key: K,
    ) -> Result<HistoryTreeSlot<'a, K, V>, ReplayTerminalFailure>
    where (K, V): crate::trading::paper_replay_financial_work_v1::HistoryTreeEntry {
        self.finish()?;
        let previous_len = map.len();
        Ok(match map.entry(key) {
            std::collections::btree_map::Entry::Occupied(entry) => HistoryTreeSlot::Occupied(entry.into_mut()),
            std::collections::btree_map::Entry::Vacant(entry) => HistoryTreeSlot::Vacant(HistoryVacant {
                entry, previous_len
            }),
        })
    }
    pub(crate) fn tree_insert<'a, K: Ord, V>(
        &mut self, vacant: HistoryVacant<'a, K, V>, value: V,
    ) -> Result<&'a mut V, ReplayTerminalFailure>
    where (K, V): crate::trading::paper_replay_financial_work_v1::HistoryTreeEntry {
        self.finish()?;
        let bytes = (|| {
            let n = add(vacant.previous_len as u64, 1)?;
            let height = if n <= 1 {
                0
            }
            else {
                u64::from(u64::BITS - (n - 1).leading_zeros())
            };
            let (leaf, internal) = btree_node_bounds::<K, V>()?;
            mul(add(height, 2)?, leaf.bytes().max(internal.bytes()))
        })().map_err(|e| self.layout_error(e))?;
        self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
        #[cfg(test)]
        {
            self.work.history_entries[2] += 1;
        }
        Ok(vacant.entry.insert(value))
    }
    pub(crate) fn hash_set_insert<T: Eq + std::hash::Hash>(
        &mut self, set: &mut std::collections::HashSet<T>, value: T,
    ) -> Result<bool, ReplayTerminalFailure>
    where (T, ()): crate::trading::paper_replay_financial_work_v1::HistoryHashEntry {
        self.finish()?;
        if set.len() == set.capacity() {
            let bytes = hash_table_bound::<(T, ())>(add(set.capacity() as u64, 1).map_err(|e| self.layout_error(e))?)
                .map_err(|e| self.layout_error(e))?;
            self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
        }
        // The selected hashbrown insert reserves before testing duplicates.
        #[cfg(test)]
        {
            self.work.history_entries[3] += 1;
        }
        Ok(set.insert(value))
    }
    pub(crate) fn clone_name(&mut self, target: &mut String, source: &String) -> Result<(), ReplayTerminalFailure> {
        self.finish()?;
        let bytes = amortized_vector_bytes::<u8>(target.capacity() as u64, source.len() as u64)
            .map_err(|e| self.layout_error(e))?;
        self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
        #[cfg(test)]
        {
            self.work.history_boundary_entries[2] += 1;
        }
        target.clone_from(source);
        Ok(())
    }
    pub(crate) fn text(&mut self, request: crate::trading::paper_replay_financial_work_v1::HistoryText<'_>) -> Result<String, ReplayTerminalFailure> {
        use crate::trading::paper_replay_financial_work_v1::FinancialSink;
        self.finish()?;
        let float = match request.unexpected_float() {
            Some(value) => Some(format_history_float(value, Some(&mut *self.work))
                .map_err(|_| self.refuse(ReplayHistoryQualificationFailure::WriterMismatch))?),
            None => None,
        };
        let mut count = FinancialSink::Count(0);
        request.write(&mut count, float.as_ref()).map_err(|_| self.layout_error(LayoutFailure::Overflow))?;
        let n = count.count();
        self.work.reserve_array::<u8>(ReplaySite::HistoryText, n as u64)?.consume();
        #[cfg(test)]
        {
            self.work.history_entries[4] += 1;
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(n).map_err(|_| self.allocation_error(ReplaySite::HistoryText))?;
        let mut output = FinancialSink::Output {
            bytes: &mut bytes,
            limit: n
        };
        if request.write(&mut output, float.as_ref()).is_err() || bytes.len() != n {
            return Err(self.refuse(ReplayHistoryQualificationFailure::WriterMismatch));
        }
        String::from_utf8(bytes).map_err(|_| self.refuse(ReplayHistoryQualificationFailure::WriterMismatch))
    }
}
#[cfg(test)]
impl<'loan, 'pool> FinancialFixtureLoan<'loan, 'pool> {
    pub(crate) fn history_ops(&mut self) -> Result<HistoryOps<'_, 'pool>, ReplayTerminalFailure> {
        self.finish()?;
        Ok(HistoryOps {
            work: self.work
        })
    }
}

impl HistoryOps<'_, '_> {
    pub(crate) fn chrono_text(&mut self, request: crate::trading::paper_replay_financial_work_v1::HistoryChrono<'_>) -> Result<String, ReplayTerminalFailure> {
        use crate::trading::paper_replay_financial_work_v1::{
            FinancialSink,
            HistoryChrono
        };
        use std::fmt::Write;
        self.finish()?;
        // Offset ownership is paid before constructing DelayedFormat. Count via
        // write_to, never Display (which itself constructs a hidden String).
        match request {
            HistoryChrono::FixedNanos(value) => {
                self.work.reserve(ReplaySite::HistoryText, 4 * 8)?.consume();
                #[cfg(test)]
                {
                    self.work.history_boundary_entries[5] += 1;
                }
                let delayed = value.format("%Y-%m-%d %H:%M:%S%.9f");
                let mut count = FinancialSink::Count(0);
                delayed.write_to(&mut count).map_err(|_| self.refuse(ReplayHistoryQualificationFailure::WriterMismatch))?;
                self.delayed_text_request(count.count())?;
                #[cfg(test)]
                {
                    self.work.history_boundary_entries[6] += 1;
                }
                Ok(delayed.to_string())
            }
            HistoryChrono::Whole(value) | HistoryChrono::NaiveNanos(value) => {
                let format = if matches!(request, HistoryChrono::Whole(_)) { "%Y-%m-%d %H:%M:%S" } else { "%Y-%m-%d %H:%M:%S%.9f" };
                let delayed = value.format(format);
                let mut count = FinancialSink::Count(0);
                delayed.write_to(&mut count).map_err(|_| self.refuse(ReplayHistoryQualificationFailure::WriterMismatch))?;
                self.delayed_text_request(count.count())?;
                #[cfg(test)]
                {
                    self.work.history_boundary_entries[6] += 1;
                }
                Ok(delayed.to_string())
            }
            HistoryChrono::Date(value) => {
                let mut count = FinancialSink::Count(0);
                write!(&mut count, "{value}").map_err(|_| self.refuse(ReplayHistoryQualificationFailure::WriterMismatch))?;
                let n = count.count();
                let bytes = if n == 0 {
                    0
                }
                else {
                    mul(4, (n as u64).max(8)).map_err(|e| self.layout_error(e))?
                };
                self.work.reserve(ReplaySite::HistoryText, bytes)?.consume();
                Ok(value.to_string())
            }
            HistoryChrono::UtcMillis(value) => {
                self.work.reserve(ReplaySite::HistoryText, 38)?.consume();
                Ok(value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            }
            HistoryChrono::Dotted(value) => {
                let bytes = mul(6, add(value.len() as u64, 1).map_err(|e| self.layout_error(e))?.max(8)).map_err(|e| self.layout_error(e))?;
                self.work.reserve(ReplaySite::HistoryText, bytes)?.consume();
                Ok(format!("{value}."))
            }
        }
    }
    fn delayed_text_request(&mut self, n: usize) -> Result<(), ReplayTerminalFailure> {
        let bytes = if n == 0 {
            0
        }
        else {
            mul(5, (n as u64).max(8)).map_err(|e| self.layout_error(e))?
        };
        self.work.reserve(ReplaySite::HistoryText, bytes)?.consume();
        Ok(())
    }
}

pub(crate) enum HistoryTreeSlot<'a, K: Ord, V> {
    Occupied(&'a mut V),
    Vacant(HistoryVacant<'a, K, V>),
}
pub(crate) struct HistoryVacant<'a, K: Ord, V> {
    entry: std::collections::btree_map::VacantEntry<'a, K, V>,
    previous_len: usize,
}
impl HistoryOps<'_, '_> {
    pub(crate) fn open_lot_push(&mut self, lots: &mut std::collections::VecDeque<crate::trading::paper_lot_ledger::OpenPaperLot>, lot: crate::trading::paper_lot_ledger::OpenPaperLot) -> Result<(), ReplayTerminalFailure> {
        self.finish()?;
        if lots.len() == lots.capacity() {
            let next = add(lots.len() as u64, 1).map_err(|e| self.layout_error(e))?;
            let bytes = amortized_vector_bytes::<crate::trading::paper_lot_ledger::OpenPaperLot>(lots.capacity() as u64, next).map_err(|e| self.layout_error(e))?;
            self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
            #[cfg(test)]
            {
                self.work.history_boundary_entries[1] += 1;
            }
            lots.try_reserve(1).map_err(|_| self.allocation_error(ReplaySite::HistoryCollection))?;
        }
        lots.push_back(lot);
        Ok(())
    }
}

pub(crate) fn historical_history_entry<K: Ord, V>(map: &mut std::collections::BTreeMap<K, V>, key: K) -> HistoryTreeSlot<'_, K, V>
where (K, V): crate::trading::paper_replay_financial_work_v1::HistoryTreeEntry {
    let previous_len = map.len();
    match map.entry(key) {
        std::collections::btree_map::Entry::Occupied(entry) => HistoryTreeSlot::Occupied(entry.into_mut()),
        std::collections::btree_map::Entry::Vacant(entry) => HistoryTreeSlot::Vacant(HistoryVacant {
            entry, previous_len
        }),
    }
}
pub(crate) fn historical_history_insert<'a, K: Ord, V>(vacant: HistoryVacant<'a, K, V>, value: V) -> &'a mut V
where (K, V): crate::trading::paper_replay_financial_work_v1::HistoryTreeEntry {
    vacant.entry.insert(value)
}

impl HistoryOps<'_, '_> {
    pub(crate) fn sort(&mut self, values: crate::trading::paper_replay_financial_work_v1::HistorySort<'_>) -> Result<(), ReplayTerminalFailure> {
        use crate::trading::paper_replay_financial_work_v1::HistorySort;
        use crate::trading::paper_ledger::{
            RecomputeFill,
            RecomputeMarket,
            OrderedEconomic
        };
        self.finish()?;
        let bytes = match &values {
            HistorySort::RecomputeFills(v) => history_sort_scratch_bytes::<RecomputeFill>(v.len() as u64),
            HistorySort::RecomputeMarkets(v) => history_sort_scratch_bytes::<RecomputeMarket>(v.len() as u64),
            HistorySort::Economic(v) => history_sort_scratch_bytes::<OrderedEconomic>(v.len() as u64),
            HistorySort::Frozen(v) => history_sort_scratch_bytes::<crate::database::attribution_epochs::FrozenPaperFill>(v.len() as u64),
        }.map_err(|e| self.layout_error(e))?;
        self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
        #[cfg(test)]
        {
            self.work.history_boundary_entries[3] += 1;
        }
        values.sort();
        Ok(())
    }
    pub(crate) fn collect_marked_pairs(&mut self, pairs: Vec<(String, crate::trading::paper_ledger::Mark)>) -> Result<std::collections::BTreeMap<String, crate::trading::paper_ledger::Mark>, ReplayTerminalFailure> {
        use crate::trading::paper_ledger::Mark;
        self.finish()?;
        let bytes = (|| {
            let n = pairs.len() as u64;
            let scratch = history_sort_scratch_bytes::<(String, Mark)>(n)?;
            let height = if n <= 1 {
                0
            }
            else {
                u64::from(u64::BITS - (n - 1).leading_zeros())
            };
            let (leaf, internal) = btree_node_bounds::<String, Mark>()?;
            let nodes = if n == 0 {
                0
            }
            else {
                add(1, mul(n, add(height, 2)?)?)?
            };
            add(scratch, mul(nodes, leaf.bytes().max(internal.bytes()))?)
        })().map_err(|e| self.layout_error(e))?;
        self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
        #[cfg(test)]
        {
            self.work.history_boundary_entries[4] += 1;
        }
        Ok(pairs.into_iter().collect())
    }
}

impl HistoryOps<'_, '_> {
    pub(crate) fn hash_set<T: Eq + std::hash::Hash>(&mut self, count: usize)
        -> Result<std::collections::HashSet<T>, ReplayTerminalFailure>
    where (T, ()): crate::trading::paper_replay_financial_work_v1::HistoryHashEntry {
        self.finish()?;
        let bytes = hash_table_bound::<(T, ())>(count as u64)
            .map_err(|error| self.layout_error(error))?;
        self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
        Ok(std::collections::HashSet::with_capacity(count))
    }
    pub(crate) fn hash_map<K: Eq + std::hash::Hash, V>(&mut self, count: usize)
        -> Result<std::collections::HashMap<K, V>, ReplayTerminalFailure>
    where (K, V): crate::trading::paper_replay_financial_work_v1::HistoryHashEntry {
        self.finish()?;
        let bytes = hash_table_bound::<(K, V)>(count as u64)
            .map_err(|error| self.layout_error(error))?;
        self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
        Ok(std::collections::HashMap::with_capacity(count))
    }
    pub(crate) fn hash_insert<K: Eq + std::hash::Hash, V>(
        &mut self, map: &mut std::collections::HashMap<K, V>, key: K, value: V,
    ) -> Result<Option<V>, ReplayTerminalFailure>
    where (K, V): crate::trading::paper_replay_financial_work_v1::HistoryHashEntry {
        self.finish()?;
        if map.len() == map.capacity() {
            let count = add(map.capacity() as u64, 1).map_err(|error| self.layout_error(error))?;
            let bytes = hash_table_bound::<(K, V)>(count).map_err(|error| self.layout_error(error))?;
            self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
        }
        Ok(map.insert(key, value))
    }
    pub(crate) fn terminal_rows_index<'a>(
        &mut self,
        map: &mut std::collections::HashMap<&'a str, Vec<&'a crate::database::order_audit::CanonicalOrderAuditRow>>,
        row: &'a crate::database::order_audit::CanonicalOrderAuditRow,
    ) -> Result<(), ReplayTerminalFailure> {
        self.finish()?;
        let key = row.business_order_id.as_str();
        // The selected std rustc_entry only grows for an absent key. This
        // borrowed lookup precedes that reserve and constructs no owned key.
        if !map.contains_key(key) && map.len() == map.capacity() {
            let count = add(map.capacity() as u64, 1).map_err(|error| self.layout_error(error))?;
            let bytes = hash_table_bound::<(&str, Vec<&crate::database::order_audit::CanonicalOrderAuditRow>)>(count)
                .map_err(|error| self.layout_error(error))?;
            self.work.reserve(ReplaySite::HistoryCollection, bytes)?.consume();
        }
        #[cfg(test)]
        {
            self.work.history_boundary_entries[0] += 1;
        }
        let rows = map.entry(key).or_default();
        self.push(rows, row)
    }
}

impl HistoryOps<'_, '_> {
    pub(crate) fn count_overflow(&mut self) -> ReplayTerminalFailure {
        self.layout_error(LayoutFailure::Overflow)
    }
}

#[cfg(test)]
impl FinancialFixtureLoan<'_, '_> {
    pub(crate) fn history_used(&self) -> u64 {
        self.work.used()
    }
    pub(crate) fn history_entries(&self) -> [usize; 7] {
        self.work.history_entries
    }
}

#[cfg(test)]
pub(crate) fn history_fixture(case: crate::trading::paper_replay_history_v1_tests::Case) {
    use crate::trading::paper_replay_history_v1_tests as test;
    let mut metadata = RowsSpecWork::new(16 * 1024 * 1024, 1, 1);
    let mut terminal = super::target::test_replay_terminal();
    let mut work = BorrowedReplayWork::test_borrow(&mut metadata, &mut terminal);
    if let test::Case::Boundary(boundary) = case {
        history_boundary_witnesses::run(boundary, &mut work);
        return;
    }
    if matches!(case, test::Case::FloatSink) {
        use std::io::Write;
        let mut sink = HistoryFloatSink {
            work: Some(&mut work),
            value: HistoryFloat {
                bytes: [0; 24],
                len: 0
            },
        };
        assert_eq!(sink.write(&[b'x'; 25]).unwrap_err().kind(), std::io::ErrorKind::WriteZero);
        assert_eq!(sink.value.len, 0);
        assert_eq!(sink.write(b"x").unwrap_err().kind(), std::io::ErrorKind::WriteZero);
        drop(sink);
        assert_eq!(work.finish(), Err(ReplayTerminalFailure::HistoryQualification(
            ReplayHistoryQualificationFailure::WriterMismatch,
        )));
        assert_eq!(work.used(), 0);
        assert_eq!(work.history_entries, [0; 7]);
        return;
    }
    if matches!(case, test::Case::Qualification) {
        let first = work.codec_memory().err().expect("ordinary profile refuses");
        assert!(matches!(first, ReplayTerminalFailure::CodecQualification(
            ReplayCodecQualificationFailure {
                kind: ReplayCodecFailureKind::PinUnavailable,
                offset: None
            }
        )));
        assert_eq!(work.finish(), Err(first));
        assert_eq!(work.codec_memory().err(), Some(first));
        assert_eq!(work.used(), 0);
        return;
    }
    {
        let loan = FinancialFixtureLoan {
            work: &mut work,
            calendar_payment: paid_calendar::CalendarPaymentState::unpaid(),
            hash_hits: [0; 4],
            // This existing neutral diagnostic branch never injects debt or
            // bypasses a gate. The history dispatcher below owns test selection.
            case: crate::trading::paper_replay_transition_v1_tests::Case::Grown,
        };
        let financial = crate::trading::paper_replay_financial_work_v1::FinancialWork::Fixture(loan);
        drop(test::run(case, financial));
    }
    if matches!(case, test::Case::RawExact | test::Case::RawShort | test::Case::Cumulative | test::Case::RawErrorShort) {
        let site = if matches!(case, test::Case::RawErrorShort) {
            ReplaySite::HistoryText
        }
        else {
            ReplaySite::HistoryRawRow
        };
        assert_eq!(work.finish(), Err(ReplayTerminalFailure::Resource(ReplayResourceFailure {
            site,
            cause: ResourceCause::Debit(RowsSpecDebitFailure::Exceeded),
            used: work.used(),
        })));
    }
    else if matches!(case, test::Case::Identity) {
        assert_eq!(work.finish(), Err(ReplayTerminalFailure::HistoryQualification(
            ReplayHistoryQualificationFailure::IdentityContextUnavailable
        )));
    }
    else {
        assert_eq!(work.finish(), Ok(()));
        assert!(work.used() > 0);
    }
}

impl HistoryOps<'_, '_> {
    pub(crate) fn known_audit_boxes(&mut self) -> Result<(), ReplayTerminalFailure> {
        self.finish()?;
        let bytes = (|| {
            let string_error = record_upper(&[FieldLayout::of::<String>()])?;
            let custom = record_upper(&[
                FieldLayout::of::<std::io::ErrorKind>(),
                FieldLayout::of::<Box<dyn std::error::Error + Send + Sync>>(),
            ])?;
            let alignment = custom.align.max(4);
            let custom = layout(round_up(custom.size, alignment)?, alignment)?;
            add(add(string_error.size, custom.size)?, FieldLayout::of::<std::io::Error>().size)
        })().map_err(|error| self.layout_error(error))?;
        self.work.reserve(ReplaySite::ErrorStorage, bytes)?.consume();
        Ok(())
    }
    pub(crate) fn known_audit_display(
        &mut self, error: &crate::database::order_audit::KnownAuditError,
    ) -> Result<(), ReplayTerminalFailure> {
        self.finish()?;
        let count = error.message_bytes() as u64;
        // The known Display performs one push_str into the new String.
        let count = if count == 0 {
            0
        }
        else {
            count.max(8)
        };
        self.work.reserve_array::<u8>(ReplaySite::ErrorStorage, count)?.consume();
        Ok(())
    }
}

impl HistoryOps<'_, '_> {
    pub(crate) fn known_source_lowercase(&mut self, detail: &crate::database::attribution_epochs::KnownSourceDetail)
        -> Result<(), ReplayTerminalFailure> {
        self.finish()?;
        self.work.reserve_array::<u8>(ReplaySite::ErrorStorage, detail.bytes() as u64)?.consume();
        Ok(())
    }
}

fn history_sort_scratch_bytes<T>(count: u64) -> Result<u64, LayoutFailure> {
    if count < 2 {
        Ok(0)
    }
    else {
        stable_sort_scratch_bytes::<T>(count)
    }
}

// JsonUnexpected::Float and CompactFormatter share the retained zmij finite
// formatter. This closed 24-byte sink latches before returning WriteZero and
// never manufactures a serde_json error or a String on the failure path.
pub(crate) struct HistoryFloat {
    bytes: [u8; 24],
    len: usize,
}
impl HistoryFloat {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}
struct HistoryFloatSink<'loan, 'pool> {
    work: Option<&'loan mut BorrowedReplayWork<'pool>>,
    value: HistoryFloat,
}
impl HistoryFloatSink<'_, '_> {
    fn refused(&mut self) -> std::io::Error {
        if let Some(work) = self.work.as_deref_mut() {
            work.terminal.latch(ReplayTerminalFailure::HistoryQualification(
                ReplayHistoryQualificationFailure::WriterMismatch,
            ));
        }
        std::io::ErrorKind::WriteZero.into()
    }
}
impl std::io::Write for HistoryFloatSink<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Some(work) = self.work.as_deref() {
            if work.finish().is_err() {
                return Err(std::io::ErrorKind::WriteZero.into());
            }
        }
        let end = match self.value.len.checked_add(bytes.len()) {
            Some(end) if end <= self.value.bytes.len() => end,
            _ => return Err(self.refused()),
        };
        self.value.bytes[self.value.len..end].copy_from_slice(bytes);
        self.value.len = end;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.work.as_deref().is_some_and(|work| work.finish().is_err()) {
            Err(std::io::ErrorKind::WriteZero.into())
        }
        else {
            Ok(())
        }
    }
}
fn format_history_float(
    value: f64,
    work: Option<&mut BorrowedReplayWork<'_>>,
) -> Result<HistoryFloat, ()> {
    use serde_json::ser::Formatter;
    let mut sink = HistoryFloatSink {
        work,
        value: HistoryFloat {
            bytes: [0; 24],
            len: 0
        }
    };
    if !value.is_finite() {
        let _ = sink.refused();
        return Err(());
    }
    serde_json::ser::CompactFormatter.write_f64(&mut sink, value).map_err(|_| ())?;
    Ok(sink.value)
}
pub(crate) fn historical_history_float(value: f64) -> Result<HistoryFloat, ()> {
    format_history_float(value, None)
}


#[cfg(test)]
mod history_boundary_witnesses {
    use super::*;
    use crate::trading::paper_replay_financial_work_v1::{FinancialFailure, FinancialWork, HistoryChrono, HistorySort, RawRowFrame};
    use crate::trading::paper_replay_history_v1_tests::{self as data, Boundary};
    use crate::database::order_audit::CanonicalOrderAuditRow;
    use std::collections::{BTreeMap, VecDeque};

    const LIMIT: u64 = 16 * 1024 * 1024;

    // Independent source-formula oracles. These never call production layout
    // helpers or derive a cost from a measured execution of the paid operation.
    fn align(n: u64, a: u64) -> u64 {
        n.div_ceil(a) * a
    }
    fn hash_request<T>(c: usize) -> u64 {
        if c == 0 {
            return 0;
        }
        let s = size_of::<T>() as u64;
        let a = align_of::<T>() as u64;
        [8_u64, 16].into_iter().map(|group| {
            let minimum = if group == 16 && s <= 1 { 14 }
                else if (group == 16 && s <= 3) || (group == 8 && s <= 1) { 7 }
                else { 3 };
            let buckets = if c < 15 {
                let wanted = (c as u64).max(minimum);
                if wanted < 4 { 4 } else if wanted < 8 { 8 } else { 16 }
            } else { ((c as u64 * 8) / 7).next_power_of_two() };
            align(buckets * s, a.max(group)) + buckets + group
        }).max().unwrap()
    }
    fn grow_request<T>(c: usize, n: usize) -> u64 {
        let s = size_of::<T>();
        if s == 0 || n <= c {
            return 0;
        }
        let floor = if s == 1 { 8 } else if s <= 1024 { 4 } else { 1 };
        (n.max(2 * c).max(floor) * s) as u64
    }
    fn node_request<K, V>() -> u64 {
        // Actual LeafNode's five fields, conservative all-permutation bound;
        // InternalNode adds twelve pointer edges. No private sizeof mirror.
        let fields = [
            (size_of::<Option<NonNull<()>>>(), align_of::<Option<NonNull<()>>>()),
            (2, align_of::<u16>()), (2, align_of::<u16>()),
            (11 * size_of::<K>(), align_of::<K>()),
            (11 * size_of::<V>(), align_of::<V>()),
        ];
        let a = fields.iter().map(|f| f.1).max().unwrap() as u64;
        let leaf = align(fields.iter().map(|(s, a)| (s + a - 1) as u64).sum(), a);
        let pointer = align_of::<NonNull<()>>() as u64;
        let internal = align(align(leaf, pointer) + (12 * size_of::<NonNull<()>>()) as u64, a.max(pointer));
        leaf.max(internal)
    }
    fn sort_request<T>(n: usize) -> u64 {
        if n < 2 || size_of::<T>() == 0 {
            return 0;
        }
        let elements = (n - n / 2).max(n.min(8_000_000 / size_of::<T>())).max(48);
        (elements * size_of::<T>()) as u64
    }
    fn financial<'a, 'pool>(work: &'a mut BorrowedReplayWork<'pool>) -> FinancialWork<'a, 'pool> {
        FinancialWork::Fixture(FinancialFixtureLoan {
            work,
            calendar_payment: paid_calendar::CalendarPaymentState::unpaid(),
            hash_hits: [0; 4],
            case: crate::trading::paper_replay_transition_v1_tests::Case::Grown,
        })
    }
    fn prefix(work: &mut BorrowedReplayWork<'_>, prior: u64, remaining: u64) {
        assert_eq!(work.used(), prior, "independently accounted setup");
        let bytes = LIMIT.checked_sub(prior + remaining).unwrap();
        // This source is ordinary fixture/oracle storage. The distinct owned
        // destination is really paid, allocated and copied by the paired frame.
        let source = "p".repeat(bytes as usize);
        let before = work.history_entries[0];
        let frame = match RawRowFrame::fixture_copy(&source, financial(work)) {
            Ok(frame) => frame,
            Err((error, _)) => panic!("real prefix copy failed: {error:?}"),
        };
        assert_eq!(frame.copied_bytes(), source);
        drop(frame.finish()); // Drop the owned row, retain the same pool's charge.
        assert_eq!(work.used(), LIMIT - remaining);
        assert_eq!(work.history_entries[0], before + 1);
        assert_eq!(work.finish(), Ok(()));
    }
    fn expected(site: ReplaySite, used: u64) -> ReplayTerminalFailure {
        ReplayTerminalFailure::Resource(ReplayResourceFailure {
            site, cause: ResourceCause::Debit(RowsSpecDebitFailure::Exceeded), used,
        })
    }
    fn next_raw(work: &mut BorrowedReplayWork<'_>) -> ReplayTerminalFailure {
        assert_eq!(work.used(), LIMIT);
        let entries = work.history_entries;
        let failure = (HistoryOps { work }).copy_raw_sql_result("!").unwrap_err();
        assert_eq!(failure, expected(ReplaySite::HistoryRawRow, LIMIT + 1));
        assert_eq!(work.history_entries, entries);
        failure
    }
    fn sticky(work: &mut BorrowedReplayWork<'_>, first: ReplayTerminalFailure,
              used: u64, entries: [usize; 7], boundary: [usize; 7]) {
        assert_eq!(work.used(), used);
        assert_eq!(work.history_entries, entries);
        assert_eq!(work.history_boundary_entries, boundary);
        assert_eq!(work.finish(), Err(first));
        // A fresh short operation loan still observes the persistent terminal.
        assert_eq!((HistoryOps { work }).finish(), Err(first));
        assert_eq!(work.finish(), Err(first));
    }
    fn terminal(error: FinancialFailure) -> ReplayTerminalFailure {
        match error {
            FinancialFailure::Terminal(e) => e,
            other => panic!("{other:?}"),
        }
    }
    fn audit(id: i64) -> CanonicalOrderAuditRow {
        CanonicalOrderAuditRow {
            id, business_order_id: format!("TEST_CODE_PLAN_{id}"), source: "PaperTrade".into(),
            decision_basis: "decision".into(), side: "buy".into(), code: "600001".into(),
            requested_price: 10.0, execution_price: Some(10.0), quantity: 100,
            quote_observed_at: Some("2026-09-24T01:59:59.500Z".into()), outcome: "Filled".into(),
            failure_reason: None, created_at: "2026-09-24 02:00:00".into(),
        }
    }
    pub(super) fn run(case: Boundary, work: &mut BorrowedReplayWork<'_>) {
        match case {
            Boundary::HashExact | Boundary::HashShort => {
                let mut set = (HistoryOps { work }).hash_set::<i64>(3).unwrap();
                let capacity = set.capacity();
                for id in 0..capacity { assert!((HistoryOps {
                    work }).hash_set_insert(&mut set, id as i64).unwrap());
                }
                let cost = hash_request::<(i64, ())>(capacity + 1);
                let short = matches!(case, Boundary::HashShort);
                prefix(work, hash_request::<(i64, ())>(3), cost - u64::from(short));
                let entries = work.history_entries;
                let result = (HistoryOps { work }).hash_set_insert(&mut set, 0);
                let first = if short {
                    assert_eq!(result, Err(expected(ReplaySite::HistoryCollection, LIMIT + 1)));
                    assert_eq!(set.capacity(), capacity);
                    assert_eq!(work.history_entries, entries);
                    result.unwrap_err()
                } else {
                    assert_eq!(result, Ok(false));
                    assert!(set.capacity() > capacity);
                    assert_eq!(work.history_entries[3], entries[3] + 1);
                    next_raw(work)
                };
                assert_eq!(set.len(), capacity);
                for id in 0..capacity {
                    assert!(set.contains(&(id as i64)));
                }
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                let capacity = set.capacity();
                let len = set.len();
                assert_eq!((HistoryOps { work }).hash_set_insert(&mut set, 0), Err(first));
                assert_eq!(set.capacity(), capacity);
                assert_eq!(set.len(), len);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
            Boundary::TerminalExact | Boundary::TerminalTableShort | Boundary::TerminalVectorShort => {
                let audits: Vec<_> = (0..4).map(audit).collect();
                let mut map = (HistoryOps { work }).hash_map::<&str, Vec<&CanonicalOrderAuditRow>>(3).unwrap();
                assert_eq!(map.capacity(), 3, "fixed retained table branch");
                for row in &audits[..3] { (HistoryOps {
                    work }).terminal_rows_index(&mut map, row).unwrap();
                }
                let vector_cost = grow_request::<&CanonicalOrderAuditRow>(0, 1);
                let prior = hash_request::<(&str, Vec<&CanonicalOrderAuditRow>)>(3) + 3 * vector_cost;
                assert_eq!(work.used(), prior);
                (HistoryOps { work }).terminal_rows_index(&mut map, &audits[0]).unwrap();
                assert_eq!(work.used(), prior, "occupied table and spare nested Vec");
                let table_cost = hash_request::<(&str, Vec<&CanonicalOrderAuditRow>)>(map.capacity() + 1);
                let allowance = match case {
                    Boundary::TerminalTableShort => table_cost - 1,
                    Boundary::TerminalVectorShort => table_cost + vector_cost - 1,
                    _ => table_cost + vector_cost,
                };
                prefix(work, prior, allowance);
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                let first = match (HistoryOps { work }).terminal_rows_index(&mut map, &audits[3]) {
                    Ok(()) => {
                        assert!(matches!(case, Boundary::TerminalExact));
                        assert_eq!(map[&*audits[3].business_order_id].len(), 1);
                        assert_eq!(work.history_entries[1], entries[1] + 1);
                        assert_eq!(work.history_boundary_entries[0], boundary[0] + 1);
                        next_raw(work)
                    }
                    Err(first) => {
                        assert_eq!(first, expected(ReplaySite::HistoryCollection, LIMIT + 1));
                        assert_eq!(work.history_entries, entries, "nested backing not entered");
                        if matches!(case, Boundary::TerminalTableShort) {
                            assert!(!map.contains_key(audits[3].business_order_id.as_str()));
                            assert_eq!(map.len(), 3);
                            assert_eq!(work.history_boundary_entries, boundary);
                        } else {
                            assert!(matches!(case, Boundary::TerminalVectorShort));
                            let rows = &map[&*audits[3].business_order_id];
                            assert!(rows.is_empty());
                            assert_eq!(rows.capacity(), 0);
                            assert_eq!(work.history_boundary_entries[0], boundary[0] + 1);
                        }
                        first
                    }
                };
                assert_eq!(map[&*audits[0].business_order_id].len(), 2);
                let lengths: Vec<_> = map.values().map(Vec::len).collect();
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                assert_eq!((HistoryOps { work }).terminal_rows_index(&mut map, &audits[3]), Err(first));
                assert_eq!(map.values().map(Vec::len).collect::<Vec<_>>(), lengths);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
            Boundary::VectorNewExact | Boundary::VectorNewShort => {
                let cost = 3 * size_of::<i64>() as u64;
                let short = matches!(case, Boundary::VectorNewShort);
                prefix(work, 0, cost - u64::from(short));
                let entries = work.history_entries;
                let result = (HistoryOps { work }).vector::<i64>(3);
                let first = if short {
                    assert_eq!(work.history_entries, entries);
                    let first = result.unwrap_err();
                    assert_eq!(first, expected(ReplaySite::HistoryCollection, LIMIT + 1));
                    first
                } else {
                    let values = result.unwrap();
                    assert!(values.is_empty());
                    assert!(values.capacity() >= 3);
                    assert_eq!(work.history_entries[1], entries[1] + 1);
                    next_raw(work)
                };
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                assert_eq!((HistoryOps { work }).vector::<i64>(3).unwrap_err(), first);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
            Boundary::VectorExact | Boundary::VectorShort => {
                let mut values = (HistoryOps { work }).vector::<i64>(2).unwrap();
                assert_eq!(values.capacity(), 2);
                (HistoryOps { work }).push(&mut values, 11).unwrap();
                (HistoryOps { work }).push(&mut values, 22).unwrap();
                let cost = grow_request::<i64>(values.capacity(), 3);
                let short = matches!(case, Boundary::VectorShort);
                prefix(work, 2 * size_of::<i64>() as u64, cost - u64::from(short));
                let entries = work.history_entries;
                let result = (HistoryOps { work }).push(&mut values, 33);
                let first = if short {
                    assert_eq!(result, Err(expected(ReplaySite::HistoryCollection, LIMIT + 1)));
                    assert_eq!(values, [11, 22]);
                    assert_eq!(values.capacity(), 2);
                    assert_eq!(work.history_entries, entries);
                    result.unwrap_err()
                } else {
                    result.unwrap();
                    assert_eq!(values, [11, 22, 33]);
                    assert_eq!(work.history_entries[1], entries[1] + 1);
                    next_raw(work)
                };
                let saved = values.clone();
                let capacity = values.capacity();
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                assert_eq!((HistoryOps { work }).push(&mut values, 44), Err(first));
                assert_eq!(values, saved);
                assert_eq!(values.capacity(), capacity);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
            Boundary::TreeExact | Boundary::TreeShort => {
                let mut map = BTreeMap::<String, u64>::new();
                let retry_key = String::from("paid-key");
                let key = financial(work).copy(&retry_key).unwrap();
                let occupied_key = financial(work).copy(&retry_key).unwrap();
                let prior = 2 * retry_key.len() as u64;
                let cost = 2 * node_request::<String, u64>();
                let short = matches!(case, Boundary::TreeShort);
                prefix(work, prior, cost - u64::from(short));
                let entries = work.history_entries;
                let first = match insert_tree(work, &mut map, key) {
                    Ok(()) => {
                        assert!(!short);
                        assert_eq!(map.get("paid-key"), Some(&7));
                        assert_eq!(work.history_entries[2], entries[2] + 1);
                        // Occupied entry performs no node request, even at L.
                        assert!(matches!((HistoryOps { work }).tree_slot(&mut map, occupied_key).unwrap(), HistoryTreeSlot::Occupied(_)));
                        assert_eq!(work.used(), LIMIT);
                        next_raw(work)
                    }
                    Err(first) => {
                        assert!(short);
                        assert!(map.is_empty());
                        assert_eq!(work.history_entries, entries);
                        assert_eq!(first, expected(ReplaySite::HistoryCollection, LIMIT + 1));
                        first
                    }
                };
                let len = map.len();
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                assert_eq!(insert_tree(work, &mut map, retry_key), Err(first));
                assert_eq!(map.len(), len);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
            Boundary::MarkedExact | Boundary::MarkedShort => {
                use crate::trading::paper_ledger::{Mark, Money};
                let mut marks = data::seed().marks;
                marks.push(marks[0].clone());
                marks[1].price = Money::from_micros(99);
                // Consuming collector drops its input on refusal. Prepare an
                // equivalent ordinary fixture input before the measured cut.
                let retry_marks = marks.clone();
                let key_bytes: u64 = marks.iter().map(|m| m.code.len() as u64).sum();
                let pairs = 2 * size_of::<(String, Mark)>() as u64;
                // n=2: ceil-log2=1, U=1+2*(1+2)=7, plus full pair sort envelope.
                let collector = sort_request::<(String, Mark)>(2) + 7 * node_request::<String, Mark>();
                let short = matches!(case, Boundary::MarkedShort);
                prefix(work, 0, pairs + key_bytes + collector - u64::from(short));
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                let result = financial(work).collect_marked_history_map(marks);
                assert_eq!(work.history_entries[1], entries[1] + 1, "real pair backing");
                assert_eq!(work.history_entries[6], entries[6] + 2, "both real key copies before collector");
                let first = if short {
                    assert_eq!(work.history_boundary_entries[4], boundary[4]);
                    let first = terminal(result.unwrap_err());
                    assert_eq!(first, expected(ReplaySite::HistoryCollection, LIMIT + 1));
                    first
                } else {
                    let map = result.unwrap();
                    assert_eq!(map.len(), 1);
                    assert_eq!(map["600001"].price, Money::from_micros(99));
                    assert_eq!(work.history_boundary_entries[4], boundary[4] + 1);
                    next_raw(work)
                };
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                assert_eq!(terminal(financial(work).collect_marked_history_map(retry_marks).unwrap_err()), first);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
            Boundary::SortExact | Boundary::SortShort => {
                use crate::database::attribution_epochs::{history_boundary_frozen_rows, history_boundary_frozen_ids, FrozenPaperFill};
                let mut rows = history_boundary_frozen_rows();
                let cost = sort_request::<FrozenPaperFill>(rows.len());
                assert!(cost > 4096, "chosen genuine heap-scratch-sized lower input");
                let short = matches!(case, Boundary::SortShort);
                prefix(work, 0, cost - u64::from(short));
                let boundary = work.history_boundary_entries;
                let result = (HistoryOps { work }).sort(HistorySort::Frozen(&mut rows));
                let first = if short {
                    assert_eq!(result, Err(expected(ReplaySite::HistoryCollection, LIMIT + 1)));
                    assert_eq!(history_boundary_frozen_ids(&rows), (0..1000).rev().collect::<Vec<i64>>());
                    assert_eq!(work.history_boundary_entries, boundary);
                    result.unwrap_err()
                } else {
                    result.unwrap();
                    assert_eq!(history_boundary_frozen_ids(&rows), (0..1000).collect::<Vec<i64>>());
                    assert_eq!(work.history_boundary_entries[3], boundary[3] + 1);
                    next_raw(work)
                };
                let ids = history_boundary_frozen_ids(&rows);
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                assert_eq!((HistoryOps { work }).sort(HistorySort::Frozen(&mut rows)), Err(first));
                assert_eq!(history_boundary_frozen_ids(&rows), ids);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
            Boundary::QueueExact | Boundary::QueueShort => {
                use crate::trading::paper_lot_ledger::{history_boundary_open_lot, history_boundary_open_lots, OpenPaperLot};
                let mut queue = VecDeque::new();
                let mut lots = history_boundary_open_lots().into_iter();
                for _ in 0..4 { (HistoryOps {
                    work }).open_lot_push(&mut queue, lots.next().unwrap()).unwrap();
                }
                assert_eq!(queue.capacity(), 4);
                let prior = grow_request::<OpenPaperLot>(0, 1);
                queue.pop_front();
                queue.pop_front();
                assert_eq!(work.used(), prior, "pops do not refund");
                for _ in 0..2 { (HistoryOps {
                    work }).open_lot_push(&mut queue, lots.next().unwrap()).unwrap();
                }
                assert!(!queue.as_slices().1.is_empty(), "actual wrapped full queue");
                assert_eq!(work.used(), prior, "spare pushes request no backing");
                let cost = grow_request::<OpenPaperLot>(queue.capacity(), queue.len() + 1);
                let short = matches!(case, Boundary::QueueShort);
                prefix(work, prior, cost - u64::from(short));
                let before = format!("{queue:?}");
                let mut expected_order: Vec<_> = queue.iter().map(|lot| format!("{lot:?}")).collect();
                expected_order.push(format!("{:?}", history_boundary_open_lot()));
                let boundary = work.history_boundary_entries;
                let result = (HistoryOps { work }).open_lot_push(&mut queue, history_boundary_open_lot());
                let first = if short {
                    assert_eq!(result, Err(expected(ReplaySite::HistoryCollection, LIMIT + 1)));
                    assert_eq!(format!("{queue:?}"), before);
                    assert_eq!(queue.capacity(), 4);
                    assert_eq!(work.history_boundary_entries, boundary);
                    result.unwrap_err()
                } else {
                    result.unwrap();
                    assert_eq!(queue.len(), 5);
                    assert_eq!(queue.iter().map(|lot| format!("{lot:?}")).collect::<Vec<_>>(), expected_order);
                    assert!(queue.capacity() >= 8);
                    assert_eq!(work.history_boundary_entries[1], boundary[1] + 1);
                    next_raw(work)
                };
                let before = format!("{queue:?}");
                let capacity = queue.capacity();
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                assert_eq!((HistoryOps { work }).open_lot_push(&mut queue, history_boundary_open_lot()), Err(first));
                assert_eq!(format!("{queue:?}"), before);
                assert_eq!(queue.capacity(), capacity);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
            Boundary::NameExact | Boundary::NameShort => {
                let mut target = financial(work).copy(&String::from("old")).unwrap();
                let capacity = target.capacity();
                (HistoryOps { work }).clone_name(&mut target, &String::from("a")).unwrap();
                (HistoryOps { work }).clone_name(&mut target, &String::new()).unwrap();
                assert_eq!(work.used(), 3, "shorter/empty names do not refund or grow");
                assert_eq!(target.capacity(), capacity);
                let source = String::from("grown name containing 中文");
                let cost = grow_request::<u8>(capacity, source.len());
                let short = matches!(case, Boundary::NameShort);
                prefix(work, 3, cost - u64::from(short));
                let boundary = work.history_boundary_entries;
                let result = (HistoryOps { work }).clone_name(&mut target, &source);
                let first = if short {
                    assert_eq!(result, Err(expected(ReplaySite::HistoryCollection, LIMIT + 1)));
                    assert!(target.is_empty());
                    assert_eq!(target.capacity(), capacity);
                    assert_eq!(work.history_boundary_entries, boundary);
                    result.unwrap_err()
                } else {
                    result.unwrap();
                    assert_eq!(target, source);
                    assert_eq!(work.history_boundary_entries[2], boundary[2] + 1);
                    next_raw(work)
                };
                let saved = target.clone();
                let capacity = target.capacity();
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                assert_eq!((HistoryOps { work }).clone_name(&mut target, &source), Err(first));
                assert_eq!(target, saved);
                assert_eq!(target.capacity(), capacity);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
            Boundary::ChronoExact | Boundary::ChronoOffsetShort | Boundary::ChronoFormatterShort => {
                use chrono::TimeZone;
                let value = chrono::FixedOffset::east_opt(28_800).unwrap()
                    .with_ymd_and_hms(2026, 9, 24, 10, 0, 0).unwrap();
                let request = HistoryChrono::FixedNanos(value);
                let original = request.historical();
                assert_eq!(original, "2026-09-24 10:00:00.000000000");
                // Retained Chrono proof: offset-owned String 4*8; then the
                // hidden DelayedFormat String and output requests total 5*N.
                let offset = 4 * 8;
                let formatter = 5 * (original.len() as u64).max(8);
                let allowance = match case {
                    Boundary::ChronoOffsetShort => offset - 1,
                    Boundary::ChronoFormatterShort => offset + formatter - 1,
                    _ => offset + formatter,
                };
                prefix(work, 0, allowance);
                let boundary = work.history_boundary_entries;
                let first = match (HistoryOps { work }).chrono_text(request) {
                    Ok(actual) => {
                        assert!(matches!(case, Boundary::ChronoExact));
                        assert_eq!(actual, original);
                        assert_eq!(work.history_boundary_entries[5], boundary[5] + 1);
                        assert_eq!(work.history_boundary_entries[6], boundary[6] + 1);
                        next_raw(work)
                    }
                    Err(first) => {
                        assert_eq!(first, expected(ReplaySite::HistoryText, LIMIT + 1));
                        assert_eq!(work.history_boundary_entries[6], boundary[6]);
                        let offset_entries = usize::from(matches!(case, Boundary::ChronoFormatterShort));
                        assert_eq!(work.history_boundary_entries[5], boundary[5] + offset_entries);
                        first
                    }
                };
                let entries = work.history_entries;
                let boundary = work.history_boundary_entries;
                assert_eq!((HistoryOps { work }).chrono_text(request).unwrap_err(), first);
                sticky(work, first, LIMIT + 1, entries, boundary);
            }
        }
    }
    fn insert_tree(work: &mut BorrowedReplayWork<'_>, map: &mut BTreeMap<String, u64>, key: String) -> Result<(), ReplayTerminalFailure> {
        let slot = (HistoryOps { work }).tree_slot(map, key)?;
        match slot {
            HistoryTreeSlot::Occupied(_) => panic!("fixed vacant boundary"),
            HistoryTreeSlot::Vacant(slot) => {
                (HistoryOps { work }).tree_insert(slot, 7)?;
            }
        }
        Ok(())
    }
}
