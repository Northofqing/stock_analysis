//! Concrete deployment observations and calendar joins, not deployment authentication.
//!
//! The caller's catalog/calendar/namespace claims remain untrusted. A future protected-root
//! adapter must authenticate them and the opener before this material can authorize activation.

#![cfg_attr(not(test), allow(dead_code))]

use chrono::{DateTime, FixedOffset, TimeZone, Utc};

use crate::calendar::{verified_a_share_calendar_authority_hash, verified_a_share_trading_day};
use crate::monitor::push_job::{
    BusinessDate, CalendarId, MachineCatalog, Namespace, Sha256Digest, UnitId,
};

use super::activation::PromotionAction;
use super::activation_transaction::UtcMicrosRange;

const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ActivationCalendarClaims {
    pub(super) namespace: Namespace,
    pub(super) catalog_sha256: Sha256Digest,
    /// Full registered catalog, not just the unit selected by this request.
    pub(super) catalog_units: Vec<UnitId>,
    pub(super) calendar_id: CalendarId,
    pub(super) authority_sha256: Sha256Digest,
    pub(super) utc_offset_seconds: i32,
}

pub(super) struct ActivationCalendarScope<'a> {
    pub(super) namespace: &'a Namespace,
    /// Expected mapping from the still-untrusted deployment package; no default ID is invented.
    pub(super) calendar_id: &'a CalendarId,
    pub(super) unit_id: &'a UnitId,
    pub(super) action: PromotionAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ObservedActivationBusinessDay {
    claims: ActivationCalendarClaims,
    unit_id: UnitId,
    action: PromotionAction,
    business_date: BusinessDate,
    interval: UtcMicrosRange,
    approval_window: UtcMicrosRange,
    observed_at: u64,
}

impl ObservedActivationBusinessDay {
    pub(super) fn interval(&self) -> UtcMicrosRange {
        self.interval
    }

    pub(super) fn business_date(&self) -> &BusinessDate {
        &self.business_date
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum ActivationDeploymentError {
    #[error("activation deployment calendar scope differs")]
    CalendarBindingMismatch,
    #[error("activation deployment does not cover the exact registered unit set")]
    UnitCoverageMismatch,
    #[error("immutable exchange calendar does not cover this timestamp")]
    CalendarUnavailable,
    #[error("ordinary activation is not allowed on an exchange closure")]
    NonTradingDay,
    #[error("activation timestamp or approval window is invalid")]
    InvalidTime,
    #[error("activation time moved backwards or crossed the observed business day")]
    TimeContextChanged,
    #[error("deployment material is not a bounded regular file")]
    MaterialRejected,
    #[error("deployment material could not be read")]
    MaterialUnreadable,
    #[error("opened deployment material changed")]
    MaterialChanged,
}

/// Resolve a raw UTC quota interval. Neither the clock nor the claimed deployment is authenticated.
pub(super) fn observe_activation_business_day(
    catalog: &MachineCatalog,
    claims: &ActivationCalendarClaims,
    scope: &ActivationCalendarScope<'_>,
    observed_at: u64,
    approval_window: UtcMicrosRange,
) -> Result<ObservedActivationBusinessDay, ActivationDeploymentError> {
    validate_window(observed_at, approval_window)?;
    if &claims.namespace != scope.namespace
        || &claims.catalog_sha256 != catalog.catalog_sha256()
        || &claims.calendar_id != scope.calendar_id
        || claims.utc_offset_seconds != SHANGHAI_OFFSET_SECONDS
    {
        return Err(ActivationDeploymentError::CalendarBindingMismatch);
    }
    let mut units = claims.catalog_units.clone();
    units.sort();
    if units.len() != catalog.units().len()
        || units.windows(2).any(|pair| pair[0] == pair[1])
        || units.iter().any(|unit| catalog.unit(unit).is_none())
        || catalog.unit(scope.unit_id).is_none()
    {
        return Err(ActivationDeploymentError::UnitCoverageMismatch);
    }
    let timestamp =
        i64::try_from(observed_at).map_err(|_| ActivationDeploymentError::InvalidTime)?;
    let utc = DateTime::<Utc>::from_timestamp_micros(timestamp)
        .ok_or(ActivationDeploymentError::InvalidTime)?;
    let timezone = FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS)
        .ok_or(ActivationDeploymentError::InvalidTime)?;
    let date = utc.with_timezone(&timezone).date_naive();
    let authority = verified_a_share_calendar_authority_hash(date)
        .map_err(|_| ActivationDeploymentError::CalendarUnavailable)?;
    if authority != claims.authority_sha256.as_str() {
        return Err(ActivationDeploymentError::CalendarBindingMismatch);
    }
    if scope.action == PromotionAction::Activate
        && !verified_a_share_trading_day(date)
            .map_err(|_| ActivationDeploymentError::CalendarUnavailable)?
    {
        return Err(ActivationDeploymentError::NonTradingDay);
    }
    let next_date = date
        .succ_opt()
        .ok_or(ActivationDeploymentError::InvalidTime)?;
    let midnight = |day: chrono::NaiveDate| {
        let local = day
            .and_hms_opt(0, 0, 0)
            .ok_or(ActivationDeploymentError::InvalidTime)?;
        let value = timezone
            .from_local_datetime(&local)
            .single()
            .ok_or(ActivationDeploymentError::InvalidTime)?
            .timestamp_micros();
        u64::try_from(value).map_err(|_| ActivationDeploymentError::InvalidTime)
    };
    let interval = UtcMicrosRange {
        start: midnight(date)?,
        end: midnight(next_date)?,
    };
    let business_date = BusinessDate::parse(&date.to_string())
        .map_err(|_| ActivationDeploymentError::InvalidTime)?;
    let mut canonical_claims = claims.clone();
    canonical_claims.catalog_units = units;
    Ok(ObservedActivationBusinessDay {
        claims: canonical_claims,
        unit_id: scope.unit_id.clone(),
        action: scope.action,
        business_date,
        interval,
        approval_window,
        observed_at,
    })
}

/// Re-observe after waiting for a lock. A new business day requires a new approved command.
pub(super) fn revalidate_activation_business_day(
    previous: &ObservedActivationBusinessDay,
    catalog: &MachineCatalog,
    claims: &ActivationCalendarClaims,
    scope: &ActivationCalendarScope<'_>,
    observed_at: u64,
    approval_window: UtcMicrosRange,
) -> Result<ObservedActivationBusinessDay, ActivationDeploymentError> {
    if observed_at < previous.observed_at {
        return Err(ActivationDeploymentError::TimeContextChanged);
    }
    let next =
        observe_activation_business_day(catalog, claims, scope, observed_at, approval_window)?;
    if next.claims != previous.claims
        || next.unit_id != previous.unit_id
        || next.action != previous.action
        || next.approval_window != previous.approval_window
    {
        return Err(ActivationDeploymentError::CalendarBindingMismatch);
    }
    if next.interval != previous.interval {
        return Err(ActivationDeploymentError::TimeContextChanged);
    }
    Ok(next)
}

fn validate_window(now: u64, window: UtcMicrosRange) -> Result<(), ActivationDeploymentError> {
    if window.start >= window.end
        || window.end > i64::MAX as u64
        || now < window.start
        || now >= window.end
    {
        return Err(ActivationDeploymentError::InvalidTime);
    }
    Ok(())
}

#[cfg(unix)]
pub(super) type DeploymentMaterialMetadata = unix_material::DeploymentMaterialMetadata;
#[cfg(unix)]
pub(super) type OpenedDeploymentMaterial = unix_material::OpenedDeploymentMaterial;

#[cfg(unix)]
mod unix_material {
    use std::fs::{File, Metadata};
    use std::os::unix::fs::{FileExt, MetadataExt};

    use sha2::{Digest, Sha256};

    use super::{ActivationDeploymentError, Sha256Digest};

    /// Descriptor metadata is evidence to compare, not a permission/ACL trust decision.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub(in crate::push_foundation) struct DeploymentMaterialMetadata {
        pub(in crate::push_foundation) device: u64,
        pub(in crate::push_foundation) inode: u64,
        pub(in crate::push_foundation) uid: u32,
        pub(in crate::push_foundation) gid: u32,
        pub(in crate::push_foundation) mode: u32,
        pub(in crate::push_foundation) size: u64,
        pub(in crate::push_foundation) links: u64,
        pub(in crate::push_foundation) modified: (i64, i64),
        pub(in crate::push_foundation) changed: (i64, i64),
    }

    impl DeploymentMaterialMetadata {
        fn read(file: &File, maximum_bytes: u64) -> Result<Self, ActivationDeploymentError> {
            let metadata: Metadata = file
                .metadata()
                .map_err(|_| ActivationDeploymentError::MaterialUnreadable)?;
            if !metadata.is_file() || metadata.len() > maximum_bytes {
                return Err(ActivationDeploymentError::MaterialRejected);
            }
            Ok(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
                uid: metadata.uid(),
                gid: metadata.gid(),
                mode: metadata.mode(),
                size: metadata.len(),
                links: metadata.nlink(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
            })
        }
    }

    pub(in crate::push_foundation) struct OpenedDeploymentMaterial {
        file: File,
        maximum_bytes: u64,
        metadata: DeploymentMaterialMetadata,
        sha256: Sha256Digest,
    }

    impl std::fmt::Debug for OpenedDeploymentMaterial {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("OpenedDeploymentMaterial")
                .field("size", &self.metadata.size)
                .finish_non_exhaustive()
        }
    }

    impl OpenedDeploymentMaterial {
        /// The supplied descriptor is not claimed to have a protected or symlink-free origin.
        pub(in crate::push_foundation) fn observe(
            file: File,
            maximum_bytes: u64,
        ) -> Result<Self, ActivationDeploymentError> {
            let (metadata, sha256) = read_observation(
                &file,
                maximum_bytes,
                #[cfg(test)]
                None,
            )?;
            Ok(Self {
                file,
                maximum_bytes,
                metadata,
                sha256,
            })
        }

        pub(in crate::push_foundation) fn metadata(&self) -> &DeploymentMaterialMetadata {
            &self.metadata
        }
        pub(in crate::push_foundation) fn sha256(&self) -> &Sha256Digest {
            &self.sha256
        }

        pub(in crate::push_foundation) fn revalidate(
            &self,
        ) -> Result<(), ActivationDeploymentError> {
            let (metadata, sha256) = read_observation(
                &self.file,
                self.maximum_bytes,
                #[cfg(test)]
                None,
            )?;
            if metadata != self.metadata || sha256 != self.sha256 {
                return Err(ActivationDeploymentError::MaterialChanged);
            }
            Ok(())
        }

        /// Deterministic real-file mutation at the metadata/bytes seam, compiled only in tests.
        #[cfg(test)]
        pub(in crate::push_foundation) fn observe_with_read_hook(
            file: File,
            maximum_bytes: u64,
            after_metadata: &mut dyn FnMut(),
        ) -> Result<Self, ActivationDeploymentError> {
            let (metadata, sha256) = read_observation(&file, maximum_bytes, Some(after_metadata))?;
            Ok(Self {
                file,
                maximum_bytes,
                metadata,
                sha256,
            })
        }
    }

    fn read_observation(
        file: &File,
        maximum_bytes: u64,
        #[cfg(test)] after_metadata: Option<&mut dyn FnMut()>,
    ) -> Result<(DeploymentMaterialMetadata, Sha256Digest), ActivationDeploymentError> {
        let before = DeploymentMaterialMetadata::read(file, maximum_bytes)?;
        #[cfg(test)]
        if let Some(hook) = after_metadata {
            hook();
        }
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut offset = 0_u64;
        while offset < before.size {
            let length = (before.size - offset).min(buffer.len() as u64) as usize;
            let count = file
                .read_at(&mut buffer[..length], offset)
                .map_err(|_| ActivationDeploymentError::MaterialUnreadable)?;
            if count == 0 {
                return Err(ActivationDeploymentError::MaterialChanged);
            }
            hasher.update(&buffer[..count]);
            offset += count as u64;
        }
        if file
            .read_at(&mut buffer[..1], offset)
            .map_err(|_| ActivationDeploymentError::MaterialUnreadable)?
            != 0
        {
            return Err(ActivationDeploymentError::MaterialChanged);
        }
        let after = DeploymentMaterialMetadata::read(file, maximum_bytes)?;
        if before != after {
            return Err(ActivationDeploymentError::MaterialChanged);
        }
        Ok((before, Sha256Digest::from_bytes(hasher.finalize().into())))
    }
}
