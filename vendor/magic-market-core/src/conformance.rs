use crate::{
    AuctionSnapshot, CoreError, DataBatch, DataStatus, EvidenceTimestamp, InstrumentId, IsoDate,
    NonEmptyText, ProviderId, SourcedRecord,
};
use std::collections::HashSet;
use std::time::Duration;

/// Fail-closed admission policy for an authorized opening-auction Provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuctionConformancePolicy {
    provider: ProviderId,
    provider_source: NonEmptyText,
    trading_date: IsoDate,
    maximum_source_age: Duration,
}

impl AuctionConformancePolicy {
    pub fn new(
        provider: ProviderId,
        provider_source: NonEmptyText,
        trading_date: IsoDate,
        maximum_source_age: Duration,
    ) -> Result<Self, CoreError> {
        if maximum_source_age.is_zero() {
            return Err(conformance_error(
                "maximum auction source age must be positive",
            ));
        }
        Ok(Self {
            provider,
            provider_source,
            trading_date,
            maximum_source_age,
        })
    }

    pub const fn provider(&self) -> ProviderId {
        self.provider
    }

    pub fn provider_source(&self) -> &NonEmptyText {
        &self.provider_source
    }

    pub fn trading_date(&self) -> &IsoDate {
        &self.trading_date
    }

    pub const fn maximum_source_age(&self) -> Duration {
        self.maximum_source_age
    }
}

/// Verifies that an authorized Provider satisfies the complete opening-auction
/// contract for one exact request.
///
/// This verifier does not grant authorization or advertise a Provider. It gives
/// licensed Level-2 and broker adapters one shared fail-closed contract test.
pub fn verify_auction_conformance(
    requested: &[InstrumentId],
    policy: AuctionConformancePolicy,
    batch: &DataBatch<AuctionSnapshot>,
) -> Result<(), CoreError> {
    if requested.is_empty() {
        return Err(conformance_error("auction request must not be empty"));
    }
    let mut requested_identities = HashSet::with_capacity(requested.len());
    for instrument in requested {
        if !requested_identities.insert(instrument.clone()) {
            return Err(conformance_error(
                "auction request contains a duplicate instrument identity",
            ));
        }
    }
    if !batch.quality().is_complete() {
        return Err(conformance_error(format!(
            "auction batch is incomplete: {}",
            batch.quality().issues().join("; ")
        )));
    }
    if batch.provenance().source() != policy.provider_source().as_str() {
        return Err(conformance_error(format!(
            "auction batch source {:?} does not match expected {:?}",
            batch.provenance().source(),
            policy.provider_source().as_str()
        )));
    }
    if batch.records().len() != requested.len() {
        return Err(conformance_error(format!(
            "auction response cardinality {} does not match request {}",
            batch.records().len(),
            requested.len()
        )));
    }
    let batch_id = batch
        .provenance()
        .batch_id()
        .ok_or_else(|| conformance_error("auction batch evidence has no batch ID"))?;
    let batch_source_at = batch
        .provenance()
        .source_at()
        .ok_or_else(|| conformance_error("auction batch source time is unavailable"))?;
    let batch_observed_at = batch.provenance().fetched_at();
    let source_time = EvidenceTimestamp::parse_instant(batch_source_at)
        .map_err(|_| conformance_error("auction batch source time is malformed"))?;
    let observed_time = EvidenceTimestamp::parse_instant(batch_observed_at)
        .map_err(|_| conformance_error("auction batch observation time is malformed"))?;
    let source_age = observed_time
        .duration_since(source_time)
        .ok_or_else(|| conformance_error("auction batch source time is in the future"))?;
    if source_age > policy.maximum_source_age() {
        return Err(conformance_error(format!(
            "auction batch source age {}ms exceeds the configured {}ms",
            source_age.as_millis(),
            policy.maximum_source_age().as_millis()
        )));
    }
    validate_opening_auction_time(batch_source_at, policy.trading_date())?;
    let mut returned_identities = HashSet::with_capacity(batch.records().len());
    for record in batch.records() {
        if !returned_identities.insert(record.instrument().clone()) {
            return Err(conformance_error(
                "auction response contains a duplicate instrument identity",
            ));
        }
        if !requested_identities.contains(record.instrument()) {
            return Err(conformance_error(format!(
                "auction response instrument identity {:?} is not requested",
                record.instrument()
            )));
        }
        if record.provider_id() != policy.provider() {
            return Err(conformance_error(format!(
                "auction record provider {:?} does not match expected {:?}",
                record.provider_id(),
                policy.provider()
            )));
        }
        if record.evidence_batch_id() != batch_id {
            return Err(conformance_error(format!(
                "auction record batch {} does not match batch evidence {batch_id}",
                record.evidence_batch_id()
            )));
        }
        if record.source_at() != Some(batch_source_at) {
            return Err(conformance_error(
                "auction record source time does not match batch evidence",
            ));
        }
        if record.observed_at() != batch_observed_at {
            return Err(conformance_error(
                "auction record observation time does not match batch evidence",
            ));
        }
        if record.status() != DataStatus::Available
            || record.name().is_none()
            || record.matched_price().is_none()
            || record.previous_close().is_none()
            || record.change_percent().is_none()
            || record.matched_quantity().is_none()
            || record.matched_amount().is_none()
            || record.unmatched_bid_quantity().is_none()
            || record.unmatched_ask_quantity().is_none()
            || record.volume_ratio().is_none()
        {
            return Err(conformance_error(
                "auction record is incomplete for the authorized contract",
            ));
        }
    }
    if returned_identities != requested_identities {
        return Err(conformance_error(
            "auction response identity coverage does not match the request",
        ));
    }
    Ok(())
}

fn validate_opening_auction_time(source_at: &str, trading_date: &IsoDate) -> Result<(), CoreError> {
    if source_at.get(..10) != Some(trading_date.as_str())
        || !matches!(source_at.as_bytes().get(10), Some(b'T') | Some(b' '))
    {
        return Err(conformance_error(
            "auction source time does not match the policy trading date",
        ));
    }
    let suffix = source_at.get(19..).unwrap_or_default();
    let china_offset = suffix == "+08:00"
        || suffix
            .strip_prefix('.')
            .and_then(|fraction| fraction.strip_suffix("+08:00"))
            .is_some_and(|fraction| {
                !fraction.is_empty() && fraction.bytes().all(|byte| byte.is_ascii_digit())
            });
    if !china_offset {
        return Err(conformance_error(
            "auction source time must use the explicit China +08:00 offset",
        ));
    }
    let source_time = EvidenceTimestamp::parse_instant(source_at)
        .map_err(|_| conformance_error("auction source time is malformed"))?;
    let opening =
        EvidenceTimestamp::parse_instant(&format!("{}T09:15:00+08:00", trading_date.as_str()))
            .map_err(|_| conformance_error("auction policy opening boundary is malformed"))?;
    let closing =
        EvidenceTimestamp::parse_instant(&format!("{}T09:25:00+08:00", trading_date.as_str()))
            .map_err(|_| conformance_error("auction policy closing boundary is malformed"))?;
    if !(opening..=closing).contains(&source_time) {
        return Err(conformance_error(
            "auction source time is outside the 09:15:00..=09:25:00 opening-auction window",
        ));
    }
    Ok(())
}

fn conformance_error(message: impl Into<String>) -> CoreError {
    CoreError::InvalidRequest(format!("auction conformance violation: {}", message.into()))
}
