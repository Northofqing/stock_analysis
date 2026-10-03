//! Fixed calendar request accounting. Pure arithmetic is not an applicability token.
//! Native dispatch/TLS/allocator workspace is outside the logical Rust-request scope.
use super::{
    add, btree_node_bounds, layout, mul, record_upper, round_up, BorrowedReplayWork,
    FieldLayout, LayoutFailure, ReplayMemory, ReplaySite, ReplayTerminalFailure, ResourceCause,
};
use chrono::NaiveDate;
use std::ffi::c_void;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicI8, AtomicUsize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplayCalendarQualificationFailure {
    RuleUnavailable,
    InputMismatch,
    ContradictoryStoredError,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReplayCalendarCallFailure {
    Historical(String),
    Terminal(ReplayTerminalFailure),
}
impl From<ReplayTerminalFailure> for ReplayCalendarCallFailure {
    fn from(value: ReplayTerminalFailure) -> Self {
        Self::Terminal(value)
    }
}

// Only the genuine memory factory initializes production payment state.
// This state owns no budget; a dropped loan never refunds or resets its owner.
pub(super) enum CalendarPaymentState {
    Unpaid,
    Paid,
}
impl CalendarPaymentState {
    pub(super) fn unpaid() -> Self {
        Self::Unpaid
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CalendarRequest {
    Day(NaiveDate),
    Prev(NaiveDate),
    Next(NaiveDate),
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CalendarResponse {
    Day(bool),
    Date(NaiveDate),
}
// No external constructor/Clone/Deserialize: only call_paid can issue a permit,
// and it immediately transfers it to the one fixed calendar-owned dispatcher.
pub(crate) struct CalendarCallPermit {
    request: CalendarRequest,
}
impl CalendarCallPermit {
    pub(crate) fn into_request(self) -> CalendarRequest {
        self.request
    }
}

fn qualify(
    work: &mut BorrowedReplayWork<'_>,
    reason: ReplayCalendarQualificationFailure,
) -> ReplayTerminalFailure {
    work.terminal.latch(ReplayTerminalFailure::CalendarQualification(reason))
}
fn checked_eligibility(
    work: &mut BorrowedReplayWork<'_>,
    observed: Result<(), ReplayCalendarQualificationFailure>,
) -> Result<(), ReplayTerminalFailure> {
    work.finish()?;
    observed.map_err(|reason| qualify(work, reason))
}

// Source-derived field envelopes, not a sizeof(private-layout-mirror) test.
fn calendar_wait_thread_request_upper() -> Result<u64, LayoutFailure> {
    let string = record_upper(&[FieldLayout::of::<Box<[u8]>>()])?;
    let name = record_upper(&[string])?;
    let tag = FieldLayout::of::<u128>();
    let option_align = name.align.max(tag.align);
    let optional_name = layout(
        round_up(add(add(tag.size, name.size)?, option_align - 1)?, option_align)?,
        option_align,
    )?;
    let id = record_upper(&[FieldLayout::of::<NonZeroU64>()])?;
    let parker = record_upper(&[
        FieldLayout::of::<*mut c_void>(),
        FieldLayout::of::<AtomicI8>(),
    ])?;
    let ordinary = record_upper(&[optional_name, id, parker])?;
    let inner_align = ordinary.align.max(8); // Actual Inner has repr(align(8)).
    let inner = layout(round_up(ordinary.size, inner_align)?, inner_align)?;
    let counter = FieldLayout::of::<AtomicUsize>();
    let header_align = counter.align.max(2); // ArcInner repr(C, align(2)).
    let header = round_up(mul(2, counter.size)?, header_align)?;
    let arc_align = header_align.max(inner.align);
    layout(
        round_up(add(round_up(header, inner.align)?, inner.size)?, arc_align)?,
        arc_align,
    ).map(|value| value.size)
}

const QUERY_REQUEST: u64 = 122; // max(2 * original literal prefix61, 48, 43).
const URL_REQUEST: u64 = 2 * (82 + 13);
fn date_node_requests() -> Result<u64, LayoutFailure> {
    let mut total = 0;
    for n in 0_u64..37 {
        let height = if n == 0 { 0 } else { u64::from(64 - n.leading_zeros()) };
        total = add(total, add(height, 2)?)?;
    }
    Ok(total)
}
fn compose_cold(internal: u64, thread: u64) -> Result<u64, LayoutFailure> {
    // coverage Vec16, two URLs190, source82, 37 date Strings (full8 + full16), hex64.
    let body = add(add(add(add(16, URL_REQUEST)?, 82)?, mul(37, 8 + 16)?)?, 64)?;
    add(add(body, mul(date_node_requests()?, internal)?)?, thread)
}
fn cold_request() -> Result<u64, LayoutFailure> {
    let (_, internal) = btree_node_bounds::<NaiveDate, ()>()?;
    compose_cold(internal.bytes(), calendar_wait_thread_request_upper()?)
}

fn call_paid(
    work: &mut BorrowedReplayWork<'_>,
    payment: &mut CalendarPaymentState,
    request: CalendarRequest,
) -> Result<CalendarResponse, ReplayCalendarCallFailure> {
    work.finish()?;
    checked_eligibility(work, crate::calendar::replay_calendar_preflight())?;
    if matches!(payment, CalendarPaymentState::Unpaid) {
        let bytes = cold_request()
            .map_err(|cause| work.fail(ReplaySite::CalendarCold, ResourceCause::Layout(cause)))?;
        work.reserve(ReplaySite::CalendarCold, bytes)?.consume();
        *payment = CalendarPaymentState::Paid;
    }
    work.reserve(ReplaySite::CalendarQuery, QUERY_REQUEST)?.consume();
    let permit = CalendarCallPermit { request };
    match crate::calendar::replay_calendar_dispatch(permit) {
        Ok(value) => Ok(value),
        Err(ReplayCalendarCallFailure::Historical(text)) => {
            Err(ReplayCalendarCallFailure::Historical(text))
        }
        Err(ReplayCalendarCallFailure::Terminal(ReplayTerminalFailure::CalendarQualification(reason))) => {
            Err(qualify(work, reason).into())
        }
        // The closed dispatcher only emits the fixed calendar contradiction.
        Err(ReplayCalendarCallFailure::Terminal(failure)) => {
            Err(work.terminal.latch(failure).into())
        }
    }
}

impl ReplayMemory<'_, '_> {
    fn calendar_call(&mut self, request: CalendarRequest) -> Result<CalendarResponse, ReplayCalendarCallFailure> {
        self.work.finish()?;
        self.pin.calendar_rules().map_err(|reason| qualify(self.work, reason))?;
        call_paid(self.work, &mut self.calendar_payment, request)
    }
    pub(crate) fn calendar_day(&mut self, day: NaiveDate) -> Result<bool, ReplayCalendarCallFailure> {
        match self.calendar_call(CalendarRequest::Day(day))? {
            CalendarResponse::Day(value) => Ok(value),
            CalendarResponse::Date(_) => Err(qualify(self.work, ReplayCalendarQualificationFailure::InputMismatch).into()),
        }
    }
    pub(crate) fn calendar_prev(&mut self, from: NaiveDate) -> Result<NaiveDate, ReplayCalendarCallFailure> {
        match self.calendar_call(CalendarRequest::Prev(from))? {
            CalendarResponse::Date(value) => Ok(value),
            CalendarResponse::Day(_) => Err(qualify(self.work, ReplayCalendarQualificationFailure::InputMismatch).into()),
        }
    }
    pub(crate) fn calendar_next(&mut self, from: NaiveDate) -> Result<NaiveDate, ReplayCalendarCallFailure> {
        match self.calendar_call(CalendarRequest::Next(from))? {
            CalendarResponse::Date(value) => Ok(value),
            CalendarResponse::Day(_) => Err(qualify(self.work, ReplayCalendarQualificationFailure::InputMismatch).into()),
        }
    }
}

#[cfg(test)]
#[path = "global_schema_replay_calendar_v1_tests.rs"]
mod tests;
