use crate::{
    CoreError, DataBatch, EconomicRevision, FiniteNumber, IsoDate, NonEmptyText, PositiveU32,
    ProviderId, RatioUnit, SourceEvidence, SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReferenceTenor {
    Overnight,
    OneWeek,
    TwoWeeks,
    OneMonth,
    ThreeMonths,
    SixMonths,
    NineMonths,
    OneYear,
    OverFiveYears,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReferenceRateKind {
    Shibor(ReferenceTenor),
    LoanPrimeRate(ReferenceTenor),
    Dr007,
    SourceDefined(NonEmptyText),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ReferenceRateIdentity {
    provider: ProviderId,
    kind: ReferenceRateKind,
}

impl ReferenceRateIdentity {
    pub fn new(provider: ProviderId, kind: ReferenceRateKind) -> Result<Self, CoreError> {
        match &kind {
            ReferenceRateKind::Shibor(tenor) if !is_shibor_tenor(*tenor) => {
                return Err(CoreError::InvalidRequest(
                    "Shibor accepts only its eight published tenors".into(),
                ));
            }
            ReferenceRateKind::LoanPrimeRate(tenor)
                if !matches!(
                    tenor,
                    ReferenceTenor::OneYear | ReferenceTenor::OverFiveYears
                ) =>
            {
                return Err(CoreError::InvalidRequest(
                    "loan prime rate accepts only one-year and over-five-year tenors".into(),
                ));
            }
            _ => {}
        }
        Ok(Self { provider, kind })
    }

    pub fn provider(&self) -> ProviderId {
        self.provider
    }

    pub fn kind(&self) -> &ReferenceRateKind {
        &self.kind
    }
}

impl<'de> Deserialize<'de> for ReferenceRateIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            provider: ProviderId,
            kind: ReferenceRateKind,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.provider, wire.kind).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReferenceRateRequest {
    rates: Vec<ReferenceRateIdentity>,
    start: IsoDate,
    end: IsoDate,
    max_rows: PositiveU32,
}

impl ReferenceRateRequest {
    pub fn new(
        rates: Vec<ReferenceRateIdentity>,
        start: IsoDate,
        end: IsoDate,
        max_rows: PositiveU32,
    ) -> Result<Self, CoreError> {
        validate_provider_identities(&rates, 50, "reference-rate")?;
        validate_date_bound(&start, &end, max_rows)?;
        Ok(Self {
            rates,
            start,
            end,
            max_rows,
        })
    }

    pub fn rates(&self) -> &[ReferenceRateIdentity] {
        &self.rates
    }
    pub fn start(&self) -> &IsoDate {
        &self.start
    }
    pub fn end(&self) -> &IsoDate {
        &self.end
    }
    pub fn max_rows(&self) -> PositiveU32 {
        self.max_rows
    }
    pub fn provider(&self) -> ProviderId {
        self.rates[0].provider()
    }
}

impl<'de> Deserialize<'de> for ReferenceRateRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            rates: Vec<ReferenceRateIdentity>,
            start: IsoDate,
            end: IsoDate,
            max_rows: PositiveU32,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.rates, wire.start, wire.end, wire.max_rows).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReferenceRateObservation {
    identity: ReferenceRateIdentity,
    fixing_date: IsoDate,
    rate: FiniteNumber,
    unit: RatioUnit,
    published_at: Option<NonEmptyText>,
    revision: Option<EconomicRevision>,
    evidence: SourceEvidence,
}

impl ReferenceRateObservation {
    pub fn new(
        identity: ReferenceRateIdentity,
        fixing_date: IsoDate,
        rate: FiniteNumber,
        unit: RatioUnit,
        published_at: Option<NonEmptyText>,
        revision: Option<EconomicRevision>,
        evidence: SourceEvidence,
    ) -> Result<Self, CoreError> {
        if identity.provider() != evidence.provider() {
            return Err(CoreError::InvalidRequest(
                "reference-rate provider must match source evidence".into(),
            ));
        }
        if published_at.as_ref().map(NonEmptyText::as_str) != evidence.source_at() {
            return Err(CoreError::InvalidRequest(
                "reference-rate published_at must match source evidence".into(),
            ));
        }
        Ok(Self {
            identity,
            fixing_date,
            rate,
            unit,
            published_at,
            revision,
            evidence,
        })
    }

    pub fn identity(&self) -> &ReferenceRateIdentity {
        &self.identity
    }
    pub fn fixing_date(&self) -> &IsoDate {
        &self.fixing_date
    }
    pub fn rate(&self) -> FiniteNumber {
        self.rate
    }
    pub fn unit(&self) -> RatioUnit {
        self.unit
    }
    pub fn published_at(&self) -> Option<&str> {
        self.published_at.as_ref().map(NonEmptyText::as_str)
    }
    pub fn revision(&self) -> Option<&EconomicRevision> {
        self.revision.as_ref()
    }
    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

impl<'de> Deserialize<'de> for ReferenceRateObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            identity: ReferenceRateIdentity,
            fixing_date: IsoDate,
            rate: FiniteNumber,
            unit: RatioUnit,
            published_at: Option<NonEmptyText>,
            revision: Option<EconomicRevision>,
            evidence: SourceEvidence,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.identity,
            wire.fixing_date,
            wire.rate,
            wire.unit,
            wire.published_at,
            wire.revision,
            wire.evidence,
        )
        .map_err(de::Error::custom)
    }
}

impl SourcedRecord for ReferenceRateObservation {
    fn provider_id(&self) -> ProviderId {
        self.evidence.provider()
    }
    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
    fn evidence_source_at(&self) -> Option<&str> {
        self.evidence.source_at()
    }
    fn evidence_observed_at(&self) -> Option<&str> {
        Some(self.evidence.observed_at())
    }
}

/// Checked three-letter currency label.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct CurrencyCode(String);

impl CurrencyCode {
    pub fn new(value: impl Into<String>) -> Result<Self, CoreError> {
        let value = value.into().to_ascii_uppercase();
        if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            return Err(CoreError::InvalidValue {
                field: "currency_code",
                value,
                reason: "must contain exactly three ASCII letters",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CurrencyCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct OfficialFxFixingIdentity {
    provider: ProviderId,
    base: CurrencyCode,
    quote: CurrencyCode,
}

impl OfficialFxFixingIdentity {
    pub fn new(
        provider: ProviderId,
        base: CurrencyCode,
        quote: CurrencyCode,
    ) -> Result<Self, CoreError> {
        if base == quote {
            return Err(CoreError::InvalidRequest(
                "official FX fixing currencies must differ".into(),
            ));
        }
        Ok(Self {
            provider,
            base,
            quote,
        })
    }

    pub fn provider(&self) -> ProviderId {
        self.provider
    }
    pub fn base(&self) -> &CurrencyCode {
        &self.base
    }
    pub fn quote(&self) -> &CurrencyCode {
        &self.quote
    }
}

impl<'de> Deserialize<'de> for OfficialFxFixingIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            provider: ProviderId,
            base: CurrencyCode,
            quote: CurrencyCode,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.provider, wire.base, wire.quote).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfficialFxFixingRequest {
    pairs: Vec<OfficialFxFixingIdentity>,
    start: IsoDate,
    end: IsoDate,
    max_rows: PositiveU32,
}

impl OfficialFxFixingRequest {
    pub fn new(
        pairs: Vec<OfficialFxFixingIdentity>,
        start: IsoDate,
        end: IsoDate,
        max_rows: PositiveU32,
    ) -> Result<Self, CoreError> {
        validate_provider_identities(&pairs, 50, "official FX fixing")?;
        validate_date_bound(&start, &end, max_rows)?;
        Ok(Self {
            pairs,
            start,
            end,
            max_rows,
        })
    }

    pub fn pairs(&self) -> &[OfficialFxFixingIdentity] {
        &self.pairs
    }
    pub fn start(&self) -> &IsoDate {
        &self.start
    }
    pub fn end(&self) -> &IsoDate {
        &self.end
    }
    pub fn max_rows(&self) -> PositiveU32 {
        self.max_rows
    }
    pub fn provider(&self) -> ProviderId {
        self.pairs[0].provider()
    }
}

impl<'de> Deserialize<'de> for OfficialFxFixingRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            pairs: Vec<OfficialFxFixingIdentity>,
            start: IsoDate,
            end: IsoDate,
            max_rows: PositiveU32,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.pairs, wire.start, wire.end, wire.max_rows).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OfficialFxFixing {
    identity: OfficialFxFixingIdentity,
    fixing_date: IsoDate,
    value: FiniteNumber,
    quotation_base: PositiveU32,
    published_at: Option<NonEmptyText>,
    revision: Option<EconomicRevision>,
    evidence: SourceEvidence,
}

impl OfficialFxFixing {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        base: CurrencyCode,
        quote: CurrencyCode,
        fixing_date: IsoDate,
        value: FiniteNumber,
        quotation_base: PositiveU32,
        published_at: Option<NonEmptyText>,
        revision: Option<EconomicRevision>,
        evidence: SourceEvidence,
    ) -> Result<Self, CoreError> {
        if value.get() <= 0.0 {
            return Err(CoreError::InvalidValue {
                field: "official_fx_fixing",
                value: value.get().to_string(),
                reason: "must be positive",
            });
        }
        let identity = OfficialFxFixingIdentity::new(evidence.provider(), base, quote)?;
        if published_at.as_ref().map(NonEmptyText::as_str) != evidence.source_at() {
            return Err(CoreError::InvalidRequest(
                "official FX fixing published_at must match source evidence".into(),
            ));
        }
        Ok(Self {
            identity,
            fixing_date,
            value,
            quotation_base,
            published_at,
            revision,
            evidence,
        })
    }

    pub fn identity(&self) -> &OfficialFxFixingIdentity {
        &self.identity
    }
    pub fn base(&self) -> &CurrencyCode {
        self.identity.base()
    }
    pub fn quote(&self) -> &CurrencyCode {
        self.identity.quote()
    }
    pub fn fixing_date(&self) -> &IsoDate {
        &self.fixing_date
    }
    pub fn value(&self) -> FiniteNumber {
        self.value
    }
    pub fn quotation_base(&self) -> PositiveU32 {
        self.quotation_base
    }
    pub fn published_at(&self) -> Option<&str> {
        self.published_at.as_ref().map(NonEmptyText::as_str)
    }
    pub fn revision(&self) -> Option<&EconomicRevision> {
        self.revision.as_ref()
    }
    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

impl<'de> Deserialize<'de> for OfficialFxFixing {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            identity: OfficialFxFixingIdentity,
            fixing_date: IsoDate,
            value: FiniteNumber,
            quotation_base: PositiveU32,
            published_at: Option<NonEmptyText>,
            revision: Option<EconomicRevision>,
            evidence: SourceEvidence,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.identity.provider() != wire.evidence.provider() {
            return Err(de::Error::custom(
                "official FX fixing provider must match source evidence",
            ));
        }
        Self::new(
            wire.identity.base,
            wire.identity.quote,
            wire.fixing_date,
            wire.value,
            wire.quotation_base,
            wire.published_at,
            wire.revision,
            wire.evidence,
        )
        .map_err(de::Error::custom)
    }
}

impl SourcedRecord for OfficialFxFixing {
    fn provider_id(&self) -> ProviderId {
        self.evidence.provider()
    }
    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
    fn evidence_source_at(&self) -> Option<&str> {
        self.evidence.source_at()
    }
    fn evidence_observed_at(&self) -> Option<&str> {
        Some(self.evidence.observed_at())
    }
}

pub trait ReferenceRateProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn reference_rates(
        &self,
        request: &ReferenceRateRequest,
    ) -> Result<DataBatch<ReferenceRateObservation>, Self::Error>;
}

pub trait OfficialFxFixingProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn official_fx_fixings(
        &self,
        request: &OfficialFxFixingRequest,
    ) -> Result<DataBatch<OfficialFxFixing>, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReferenceDataCapabilities {
    pub benchmark_rates: bool,
    pub official_fx_fixings: bool,
}

trait ProviderIdentity {
    fn provider(&self) -> ProviderId;
}

impl ProviderIdentity for ReferenceRateIdentity {
    fn provider(&self) -> ProviderId {
        self.provider()
    }
}

impl ProviderIdentity for OfficialFxFixingIdentity {
    fn provider(&self) -> ProviderId {
        self.provider()
    }
}

fn validate_provider_identities<T>(
    values: &[T],
    maximum: usize,
    family: &str,
) -> Result<(), CoreError>
where
    T: ProviderIdentity + Clone + Eq + std::hash::Hash,
{
    if values.is_empty() || values.len() > maximum {
        return Err(CoreError::InvalidRequest(format!(
            "{family} request accepts 1 through {maximum} identities"
        )));
    }
    let provider = values[0].provider();
    if values.iter().any(|value| value.provider() != provider) {
        return Err(CoreError::InvalidRequest(format!(
            "{family} request cannot mix providers"
        )));
    }
    let mut seen = HashSet::with_capacity(values.len());
    if values.iter().any(|value| !seen.insert(value.clone())) {
        return Err(CoreError::InvalidRequest(format!(
            "{family} request contains duplicate identities"
        )));
    }
    Ok(())
}

fn validate_date_bound(
    start: &IsoDate,
    end: &IsoDate,
    max_rows: PositiveU32,
) -> Result<(), CoreError> {
    if start > end {
        return Err(CoreError::InvalidRequest(
            "reference-data start must not exceed end".into(),
        ));
    }
    if max_rows.get() > 10_000 {
        return Err(CoreError::InvalidRequest(
            "reference-data max_rows must not exceed 10000".into(),
        ));
    }
    Ok(())
}

fn is_shibor_tenor(tenor: ReferenceTenor) -> bool {
    matches!(
        tenor,
        ReferenceTenor::Overnight
            | ReferenceTenor::OneWeek
            | ReferenceTenor::TwoWeeks
            | ReferenceTenor::OneMonth
            | ReferenceTenor::ThreeMonths
            | ReferenceTenor::SixMonths
            | ReferenceTenor::NineMonths
            | ReferenceTenor::OneYear
    )
}
