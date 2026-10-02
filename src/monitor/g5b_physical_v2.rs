//! Fresh observations of original selected occurrences' physical completion.
//! The private owner alone checks actual SQL/files/receipts. No value carries
//! a date fence, can be deserialized, or promises it remains current later.
use crate::durable_delivery::{
    DurableDeliveryCoordinator, DurableDeliveryError, G5bPhysicalSealAttempt, Result,
    VerifiedG5bPhysicalSeal,
};
use chrono::NaiveDate;
use std::sync::Arc;

pub struct G5bPhysicalSealV2 {
    coordinator: Arc<DurableDeliveryCoordinator>,
    inner: VerifiedG5bPhysicalSeal,
}
impl G5bPhysicalSealV2 {
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
    pub fn selected_count(&self) -> usize {
        self.inner.selected_count()
    }
    pub fn reason(&self) -> &'static str {
        self.inner.reason()
    }
    fn require_owner(
        &self,
        coordinator: &Arc<DurableDeliveryCoordinator>,
        date: NaiveDate,
    ) -> Result<()> {
        if !Arc::ptr_eq(&self.coordinator, coordinator) || self.business_date() != date {
            return Err(DurableDeliveryError::PolicyMismatch(
                "G5b physical observation belongs to another actual owner/date".into(),
            ));
        }
        Ok(())
    }
}

/// Incomplete is routing only; it does not erase a previously observed prefix.
pub enum G5bPhysicalAttemptV2 {
    Incomplete,
    Sealed(G5bPhysicalSealV2),
}

/// No provider, model reconstruction, business admission or physical send.
pub fn read_physical_day_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
) -> Result<Option<G5bPhysicalSealV2>> {
    let inner = {
        let session = coordinator.g5b_day_session(date)?;
        session.read_physical_seal()?
    };
    Ok(inner.map(|inner| G5bPhysicalSealV2 { coordinator, inner }))
}

/// A known observation pins its original real prefix even when a legitimate
/// late mutation removed the current pointer. New verified revisions may seal;
/// Incomplete and errors leave the caller's original observation intact.
pub fn try_seal_physical_day_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    date: NaiveDate,
    known: Option<&G5bPhysicalSealV2>,
) -> Result<G5bPhysicalAttemptV2> {
    if let Some(known) = known {
        known.require_owner(&coordinator, date)?;
    }
    let value = {
        let session = coordinator.g5b_day_session(date)?;
        match known {
            Some(known) => session.try_seal_physical_cohort_known(&known.inner)?,
            None => session.try_seal_physical_cohort()?,
        }
    };
    Ok(match value {
        G5bPhysicalSealAttempt::Incomplete => G5bPhysicalAttemptV2::Incomplete,
        G5bPhysicalSealAttempt::Sealed(inner) => {
            G5bPhysicalAttemptV2::Sealed(G5bPhysicalSealV2 { coordinator, inner })
        }
    })
}

/// Strict same-id/revision refresh; callers needing legitimate reclosure use
/// the known-aware try operation above rather than discarding the old anchor.
pub fn refresh_physical_day_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
    known: &G5bPhysicalSealV2,
) -> Result<G5bPhysicalSealV2> {
    known.require_owner(&coordinator, known.business_date())?;
    let inner = {
        let session = coordinator.g5b_day_session(known.business_date())?;
        session.refresh_physical_seal(&known.inner)?
    };
    Ok(G5bPhysicalSealV2 { coordinator, inner })
}

/// Closed SQL membership snapshot only, not source or completion authority.
/// Each returned date still needs its own actual guarded file/receipt reader.
pub fn list_physical_cohort_dates_v2(
    coordinator: Arc<DurableDeliveryCoordinator>,
) -> Result<Vec<NaiveDate>> {
    coordinator.list_g5b_nonempty_cohort_dates()
}
