use crate::{
    AssetClass, CoreError, DataBatch, Exchange, FiniteNumber, InstrumentId, IsoDate,
    MarketRankingKind, MarketRankingUnit, NonEmptyText, PositiveU32, ProviderId, SourceEvidence,
    SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::collections::HashSet;

/// Independently admitted metrics for the narrow provider Top-N contract.
///
/// These flags do not grant or imply complete-universe `MarketRankings`
/// capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderTopNRankingCapabilities {
    pub volume_ratio: bool,
    pub main_net_inflow: bool,
}

impl ProviderTopNRankingCapabilities {
    pub const fn all_admitted(self) -> bool {
        self.volume_ratio && self.main_net_inflow
    }

    pub fn supports(self, kind: &MarketRankingKind) -> bool {
        match kind {
            MarketRankingKind::VolumeRatio => self.volume_ratio,
            MarketRankingKind::MainNetInflow => self.main_net_inflow,
            _ => false,
        }
    }
}

/// One exact single-page provider Top-N request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderTopNRankingRequest {
    kind: MarketRankingKind,
    trading_date: IsoDate,
    limit: PositiveU32,
    filter_identity: NonEmptyText,
}

impl ProviderTopNRankingRequest {
    pub const MAX_SINGLE_PAGE_LIMIT: u32 = 100;

    pub fn new(
        kind: MarketRankingKind,
        trading_date: IsoDate,
        limit: PositiveU32,
        filter_identity: NonEmptyText,
    ) -> Result<Self, CoreError> {
        validate_provider_top_n_kind(&kind)?;
        if limit.get() > Self::MAX_SINGLE_PAGE_LIMIT {
            return Err(provider_top_n_error(format!(
                "limit {} exceeds the proved single-page cap of {}",
                limit.get(),
                Self::MAX_SINGLE_PAGE_LIMIT
            )));
        }
        Ok(Self {
            kind,
            trading_date,
            limit,
            filter_identity,
        })
    }

    pub fn kind(&self) -> &MarketRankingKind {
        &self.kind
    }

    pub fn trading_date(&self) -> &IsoDate {
        &self.trading_date
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }

    pub fn filter_identity(&self) -> &NonEmptyText {
        &self.filter_identity
    }
}

#[derive(Deserialize)]
struct ProviderTopNRankingRequestWire {
    kind: MarketRankingKind,
    trading_date: IsoDate,
    limit: PositiveU32,
    filter_identity: NonEmptyText,
}

impl<'de> Deserialize<'de> for ProviderTopNRankingRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProviderTopNRankingRequestWire::deserialize(deserializer)?;
        Self::new(
            wire.kind,
            wire.trading_date,
            wire.limit,
            wire.filter_identity,
        )
        .map_err(de::Error::custom)
    }
}

/// One selected row in an exact provider-ordered single response page.
///
/// `source_order_ordinal` is the preserved response order, not a provider tie
/// rank. `latest_trading_date` is per-security date evidence and is never
/// promoted to a ranking source timestamp.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderTopNRankingEntry {
    kind: MarketRankingKind,
    source_order_ordinal: PositiveU32,
    instrument: InstrumentId,
    label: NonEmptyText,
    value: FiniteNumber,
    unit: MarketRankingUnit,
    latest_trading_date: IsoDate,
    filter_identity: NonEmptyText,
    provider_declared_total: PositiveU32,
    inspected_row_count: PositiveU32,
    evidence: SourceEvidence,
}

impl ProviderTopNRankingEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: MarketRankingKind,
        source_order_ordinal: PositiveU32,
        instrument: InstrumentId,
        label: NonEmptyText,
        value: FiniteNumber,
        unit: MarketRankingUnit,
        latest_trading_date: IsoDate,
        filter_identity: NonEmptyText,
        provider_declared_total: PositiveU32,
        inspected_row_count: PositiveU32,
        evidence: SourceEvidence,
    ) -> Result<Self, CoreError> {
        validate_provider_top_n_kind_and_unit(&kind, &unit)?;
        validate_a_share_identity(&instrument)?;
        if matches!(kind, MarketRankingKind::VolumeRatio) && value.get().is_sign_negative() {
            return Err(provider_top_n_error(
                "volume-ratio value must be non-negative",
            ));
        }
        if inspected_row_count > provider_declared_total {
            return Err(provider_top_n_error(
                "inspected row count cannot exceed the provider-declared total",
            ));
        }
        if source_order_ordinal > inspected_row_count {
            return Err(provider_top_n_error(
                "source order ordinal cannot exceed the inspected row count",
            ));
        }
        if evidence.source_at().is_some() {
            return Err(provider_top_n_error(
                "evidence source_at must be absent because provider Top-N has no atomic source time",
            ));
        }
        Ok(Self {
            kind,
            source_order_ordinal,
            instrument,
            label,
            value,
            unit,
            latest_trading_date,
            filter_identity,
            provider_declared_total,
            inspected_row_count,
            evidence,
        })
    }

    pub fn kind(&self) -> &MarketRankingKind {
        &self.kind
    }

    pub fn source_order_ordinal(&self) -> PositiveU32 {
        self.source_order_ordinal
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn label(&self) -> &NonEmptyText {
        &self.label
    }

    pub fn value(&self) -> FiniteNumber {
        self.value
    }

    pub fn unit(&self) -> &MarketRankingUnit {
        &self.unit
    }

    pub fn latest_trading_date(&self) -> &IsoDate {
        &self.latest_trading_date
    }

    pub fn filter_identity(&self) -> &NonEmptyText {
        &self.filter_identity
    }

    pub fn provider_declared_total(&self) -> PositiveU32 {
        self.provider_declared_total
    }

    pub fn inspected_row_count(&self) -> PositiveU32 {
        self.inspected_row_count
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

#[derive(Deserialize)]
struct ProviderTopNRankingEntryWire {
    kind: MarketRankingKind,
    source_order_ordinal: PositiveU32,
    instrument: InstrumentId,
    label: NonEmptyText,
    value: FiniteNumber,
    unit: MarketRankingUnit,
    latest_trading_date: IsoDate,
    filter_identity: NonEmptyText,
    provider_declared_total: PositiveU32,
    inspected_row_count: PositiveU32,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for ProviderTopNRankingEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProviderTopNRankingEntryWire::deserialize(deserializer)?;
        Self::new(
            wire.kind,
            wire.source_order_ordinal,
            wire.instrument,
            wire.label,
            wire.value,
            wire.unit,
            wire.latest_trading_date,
            wire.filter_identity,
            wire.provider_declared_total,
            wire.inspected_row_count,
            wire.evidence,
        )
        .map_err(de::Error::custom)
    }
}

impl SourcedRecord for ProviderTopNRankingEntry {
    fn provider_id(&self) -> ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }

    fn evidence_source_at(&self) -> Option<&str> {
        None
    }

    fn evidence_observed_at(&self) -> Option<&str> {
        Some(self.evidence.observed_at())
    }
}

/// Rechecks the complete narrow Top-N contract at a provider/Router boundary.
pub fn validate_provider_top_n_ranking_batch(
    batch: &DataBatch<ProviderTopNRankingEntry>,
    request: &ProviderTopNRankingRequest,
    capabilities: ProviderTopNRankingCapabilities,
    expected_provider: ProviderId,
    expected_source: &NonEmptyText,
) -> Result<(), CoreError> {
    if !capabilities.supports(request.kind()) {
        return Err(provider_top_n_error(
            "requested metric is not admitted for provider Top-N",
        ));
    }
    if !batch.quality().is_complete() {
        return Err(provider_top_n_error(format!(
            "batch is incomplete: {}",
            batch.quality().issues().join("; ")
        )));
    }
    if batch.provenance().source() != expected_source.as_str() {
        return Err(provider_top_n_error(format!(
            "batch source {:?} does not match expected {:?}",
            batch.provenance().source(),
            expected_source.as_str()
        )));
    }
    if batch.provenance().source_at().is_some() {
        return Err(provider_top_n_error(
            "batch provenance source_at must be absent",
        ));
    }
    validate_post_close_observed_at(batch.provenance().fetched_at(), request.trading_date())?;
    let batch_id = batch
        .provenance()
        .batch_id()
        .ok_or_else(|| provider_top_n_error("batch provenance has no batch ID"))?;
    let first = batch
        .records()
        .first()
        .ok_or_else(|| provider_top_n_error("provider Top-N must not be empty"))?;
    if first.provider_declared_total().get() == 0 {
        return Err(provider_top_n_error(
            "provider-declared total must be non-zero",
        ));
    }
    let expected_len_u32 = request
        .limit()
        .get()
        .min(first.provider_declared_total().get());
    let expected_len = usize::try_from(expected_len_u32)
        .map_err(|_| provider_top_n_error("provider Top-N row count overflow"))?;
    if batch.records().len() != expected_len {
        return Err(provider_top_n_error(format!(
            "response returned {} rows but exactly {expected_len} are required",
            batch.records().len()
        )));
    }
    if first.inspected_row_count().get() != expected_len_u32 {
        return Err(provider_top_n_error(
            "inspected row count does not match the exact returned page",
        ));
    }

    let mut identities = HashSet::with_capacity(batch.records().len());
    let mut previous_value = None;
    for (index, record) in batch.records().iter().enumerate() {
        let expected_ordinal = u32::try_from(index + 1)
            .map_err(|_| provider_top_n_error("source order ordinal overflow"))?;
        if record.kind() != request.kind()
            || record.source_order_ordinal().get() != expected_ordinal
            || record.latest_trading_date() != request.trading_date()
            || record.filter_identity() != request.filter_identity()
            || record.provider_declared_total() != first.provider_declared_total()
            || record.inspected_row_count() != first.inspected_row_count()
        {
            return Err(provider_top_n_error(
                "records do not share the exact requested continuous Top-N context",
            ));
        }
        if record.provider_id() != expected_provider {
            return Err(provider_top_n_error(
                "record provider does not match the admitted provider",
            ));
        }
        if record.evidence().source_at().is_some() {
            return Err(provider_top_n_error(
                "record evidence source_at must be absent",
            ));
        }
        if record.evidence_batch_id() != batch_id {
            return Err(provider_top_n_error(
                "record batch ID does not match batch provenance",
            ));
        }
        if record.evidence().observed_at() != batch.provenance().fetched_at() {
            return Err(provider_top_n_error(
                "record observation time does not match batch provenance",
            ));
        }
        validate_post_close_observed_at(record.evidence().observed_at(), request.trading_date())?;
        if !identities.insert(record.instrument().clone()) {
            return Err(provider_top_n_error(
                "response contains a duplicate instrument identity",
            ));
        }
        if previous_value.is_some_and(|value| value < record.value().get()) {
            return Err(provider_top_n_error(
                "values are not in descending provider response order",
            ));
        }
        previous_value = Some(record.value().get());
    }
    Ok(())
}

/// Provider-neutral acquisition seam for the narrow provider Top-N contract.
pub trait ProviderTopNRankings {
    type Error: std::error::Error + Send + Sync + 'static;

    fn provider_top_n_rankings(
        &self,
        request: &ProviderTopNRankingRequest,
    ) -> Result<DataBatch<ProviderTopNRankingEntry>, Self::Error>;
}

fn validate_provider_top_n_kind(kind: &MarketRankingKind) -> Result<(), CoreError> {
    if !matches!(
        kind,
        MarketRankingKind::VolumeRatio | MarketRankingKind::MainNetInflow
    ) {
        return Err(provider_top_n_error(
            "only volume ratio and main-net inflow are supported provider Top-N metrics",
        ));
    }
    Ok(())
}

fn validate_provider_top_n_kind_and_unit(
    kind: &MarketRankingKind,
    unit: &MarketRankingUnit,
) -> Result<(), CoreError> {
    validate_provider_top_n_kind(kind)?;
    let valid = matches!(
        (kind, unit),
        (MarketRankingKind::VolumeRatio, MarketRankingUnit::Multiple)
            | (MarketRankingKind::MainNetInflow, MarketRankingUnit::Yuan)
    );
    if !valid {
        return Err(provider_top_n_error(
            "metric and unit are inconsistent for provider Top-N",
        ));
    }
    Ok(())
}

fn validate_a_share_identity(instrument: &InstrumentId) -> Result<(), CoreError> {
    if instrument.asset_class() != AssetClass::Equity
        || instrument.code().len() != 6
        || !instrument.code().bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(provider_top_n_error(
            "instrument must be an exact six-digit A-share equity identity",
        ));
    }
    let valid_exchange_prefix = match instrument.exchange() {
        Exchange::Shanghai => instrument.code().starts_with('6'),
        Exchange::Shenzhen => {
            instrument.code().starts_with('0') || instrument.code().starts_with('3')
        }
        Exchange::Beijing => {
            instrument.code().starts_with('4')
                || instrument.code().starts_with('8')
                || instrument.code().starts_with('9')
        }
    };
    if !valid_exchange_prefix {
        return Err(provider_top_n_error(
            "instrument code is inconsistent with its A-share exchange",
        ));
    }
    Ok(())
}

fn validate_post_close_observed_at(
    observed_at: &str,
    trading_date: &IsoDate,
) -> Result<(), CoreError> {
    let local = observed_at
        .strip_suffix("+08:00")
        .ok_or_else(|| provider_top_n_error("observed_at must use an explicit +08:00 offset"))?;
    let (date, time) = local
        .split_once('T')
        .ok_or_else(|| provider_top_n_error("observed_at must use YYYY-MM-DDTHH:MM:SS+08:00"))?;
    let observed_date = IsoDate::new(date.to_owned()).map_err(|_| {
        provider_top_n_error("observed_at must contain a valid China calendar date")
    })?;
    if &observed_date < trading_date {
        return Err(provider_top_n_error(
            "observed_at calendar date must not predate the requested trading date",
        ));
    }
    let clock = time.split_once('.').map_or(time, |(clock, fraction)| {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            ""
        } else {
            clock
        }
    });
    if clock.len() != 8
        || clock.as_bytes().get(2) != Some(&b':')
        || clock.as_bytes().get(5) != Some(&b':')
        || !clock
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 2 | 5) || byte.is_ascii_digit())
    {
        return Err(provider_top_n_error(
            "observed_at must use a valid HH:MM:SS clock",
        ));
    }
    let hour = clock[0..2].parse::<u32>().unwrap_or(u32::MAX);
    let minute = clock[3..5].parse::<u32>().unwrap_or(u32::MAX);
    let second = clock[6..8].parse::<u32>().unwrap_or(u32::MAX);
    if hour > 23 || minute > 59 || second > 59 {
        return Err(provider_top_n_error(
            "observed_at contains an invalid clock time",
        ));
    }
    if &observed_date == trading_date && (hour, minute, second) < (15, 35, 0) {
        return Err(provider_top_n_error(
            "same-date provider Top-N cannot be observed before 15:35:00 Asia/Shanghai",
        ));
    }
    Ok(())
}

fn provider_top_n_error(message: impl Into<String>) -> CoreError {
    CoreError::InvalidRequest(format!("provider Top-N ranking: {}", message.into()))
}
