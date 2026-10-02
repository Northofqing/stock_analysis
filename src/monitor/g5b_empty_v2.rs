//! Closed Empty routing and completion observations for the runtime.
//!
//! Every operation uses the caller's existing coordinator and an actual date
//! fence. No returned value holds the fence. Production clocks are owned by the
//! private protocol; callers cannot construct or deserialize completion.

use crate::durable_delivery::{
    DurableDeliveryCoordinator, G5bEmptyDayInspection, G5bEmptyPending, Result,
    VerifiedG5bEmptySeal,
};
use chrono::NaiveDate;
use std::sync::Arc;

/// Route Empty before invoking the NonEmpty model reader.
pub enum G5bEmptyInspectionV2 {
    /// No v2 cohort. This says nothing about input, legacy journals or whether
    /// fresh model work is eligible; the normal admission checks still apply.
    Absent,
    /// A stored NonEmpty cohort. Its actual model reader must verify it.
    NonEmpty,
    /// A real saved Empty closing exists, but no current verified seal.
    EmptyPending(G5bEmptyPendingV2),
    /// A current seal was verified against actual SQL and filesystem evidence.
    EmptySealed(G5bEmptySealV2),
}

/// Low authority routing information, never a completion capability.
pub struct G5bEmptyPendingV2 {
    inner: G5bEmptyPending,
}
impl G5bEmptyPendingV2 {
    pub fn business_date(&self) -> NaiveDate {
        self.inner.business_date()
    }
    pub fn cohort_identity(&self) -> &str {
        self.inner.cohort_identity()
    }
    pub fn revision(&self) -> i64 {
        self.inner.revision()
    }
}

/// Completion of the original verified closed zero prefix. Late legal input
/// can append after that prefix; this never means there was no data all day.
/// Private fields, non-Clone, non-Deserialize, and no public constructor.
pub struct G5bEmptySealV2 {
    inner: VerifiedG5bEmptySeal,
}
impl G5bEmptySealV2 {
    pub fn business_date(&self) -> NaiveDate {
        self.inner.business_date()
    }
    pub fn cohort_identity(&self) -> &str {
        self.inner.cohort_identity()
    }
    pub fn revision(&self) -> i64 {
        self.inner.revision()
    }
    pub fn identity(&self) -> &str {
        self.inner.identity()
    }
    pub fn sha256(&self) -> &str {
        self.inner.sha256()
    }
    pub fn reason(&self) -> &'static str {
        self.inner.reason()
    }
}

fn wrap(state: G5bEmptyDayInspection) -> G5bEmptyInspectionV2 {
    match state {
        G5bEmptyDayInspection::Absent => G5bEmptyInspectionV2::Absent,
        G5bEmptyDayInspection::NonEmpty => G5bEmptyInspectionV2::NonEmpty,
        G5bEmptyDayInspection::Pending(inner) => {
            G5bEmptyInspectionV2::EmptyPending(G5bEmptyPendingV2 { inner })
        }
        G5bEmptyDayInspection::Sealed(inner) => {
            G5bEmptyInspectionV2::EmptySealed(G5bEmptySealV2 { inner })
        }
    }
}

/// Synchronous local read, suitable for a blocking task. Pending verification
/// also checks the original closing input and any actual Selection leaf.
pub fn inspect_empty_day_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
) -> Result<G5bEmptyInspectionV2> {
    let session = coordinator.g5b_day_session(date)?;
    session.inspect_empty_day().map(wrap)
}

/// Observe a genuinely new zero head on the current verified trading day
/// before 15:05. Existing heads without the original receipt remain Unknown.
/// Success creates no cohort, decision, model work or completion capability.
pub fn initialize_empty_day_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
) -> Result<()> {
    let session = coordinator.g5b_day_session(date)?;
    session.initialize_empty_prospective()
}

/// Close only a real original prospective zero prefix on the current verified
/// trading day at or after 15:21, or re-read an already actual historical seal.
pub fn close_empty_day_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
) -> Result<G5bEmptySealV2> {
    let session = coordinator.g5b_day_session(date)?;
    session
        .close_empty_window()
        .map(|inner| G5bEmptySealV2 { inner })
}

/// Refresh before reusing a cache identity. A known positive input incarnation
/// cannot be replaced or truncated; no stored JSON alone grants completion.
pub fn refresh_empty_day_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    known: &G5bEmptySealV2,
) -> Result<G5bEmptySealV2> {
    let session = coordinator.g5b_day_session(known.business_date())?;
    session
        .refresh_empty_seal(&known.inner)
        .map(|inner| G5bEmptySealV2 { inner })
}

/// Recover only the original already-saved Empty Selection bytes. This does
/// not create a closing receipt or seal and never heals a Committed file.
pub fn recover_empty_day_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
) -> Result<G5bEmptyInspectionV2> {
    let session = coordinator.g5b_day_session(date)?;
    session.recover_existing_empty_day().map(wrap)
}

// Test clocks remain crate-private and are checked by the real D3 owner
// against the attested Test namespace. There is no production clock overload.
#[cfg(test)]
pub(crate) fn initialize_empty_day_for_test(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<()> {
    let session = coordinator.g5b_day_session(date)?;
    session.initialize_empty_prospective_for_test(now)
}
#[cfg(test)]
pub(crate) fn close_empty_day_for_test(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<G5bEmptySealV2> {
    let session = coordinator.g5b_day_session(date)?;
    session
        .close_empty_window_for_test(now)
        .map(|inner| G5bEmptySealV2 { inner })
}
