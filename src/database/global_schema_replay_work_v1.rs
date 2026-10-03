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
        self.work
            .reserve_array::<u8>(ReplaySite::Formatting, count as u64)?
            .consume();
        #[cfg(test)]
        {
            self.hits.outputs += 1;
        }
        let mut result = Vec::new();
        result
            .try_reserve_exact(count)
            .map_err(|_| self.allocation_error(ReplaySite::Formatting))?;
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
