use crate::{
    CoreError, DataBatch, FiniteNumber, InstrumentId, IsoDate, Price, Ratio, RatioUnit,
    SourceEvidence, SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};

/// Provider-neutral action family whose source semantics have been normalized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CorporateActionCategory {
    Distribution,
    BonusRightsListing,
    NonTradableShareListing,
    UnknownCapitalChange,
    CapitalChange,
    AdditionalIssuance,
    ShareRepurchase,
    AdditionalIssuanceListing,
    TransferredAllotmentListing,
    ConvertibleBondListing,
    CapitalRescaling,
    NonTradableReverseSplit,
    SubscriptionWarrantGrant,
    PutWarrantGrant,
}

/// A provider-native quantity whose physical unit has not been independently verified.
///
/// Consumers must not interpret values carrying this unit as shares, lots, or a per-share ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UnverifiedSourceUnit {
    ProviderNative,
}

/// Source-published lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CorporateActionStatus {
    Implemented,
    Proposed,
    Cancelled,
    Unknown,
}

/// Checked economic terms for one corporate action.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum CorporateActionTerms {
    Distribution {
        cash_per_share: Option<FiniteNumber>,
        bonus_per_share: Option<FiniteNumber>,
        rights_per_share: Option<FiniteNumber>,
        rights_price: Option<Price>,
    },
    CapitalRescaling {
        ratio: Ratio,
    },
    NonTradableReverseSplit {
        ratio: Ratio,
    },
    ProviderNativeRatio {
        category: CorporateActionCategory,
        source_ratio: FiniteNumber,
        source_ratio_unit: UnverifiedSourceUnit,
    },
    CapitalStructure {
        category: CorporateActionCategory,
        tradable_before: FiniteNumber,
        tradable_after: FiniteNumber,
        total_before: FiniteNumber,
        total_after: FiniteNumber,
        unit: UnverifiedSourceUnit,
    },
    WarrantGrant {
        category: CorporateActionCategory,
        exercise_price: Price,
        source_quantity: FiniteNumber,
        source_quantity_unit: UnverifiedSourceUnit,
    },
}

impl<'de> Deserialize<'de> for CorporateActionTerms {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum Wire {
            Distribution {
                cash_per_share: Option<FiniteNumber>,
                bonus_per_share: Option<FiniteNumber>,
                rights_per_share: Option<FiniteNumber>,
                rights_price: Option<Price>,
            },
            CapitalRescaling {
                ratio: Ratio,
            },
            NonTradableReverseSplit {
                ratio: Ratio,
            },
            ProviderNativeRatio {
                category: CorporateActionCategory,
                source_ratio: FiniteNumber,
                source_ratio_unit: UnverifiedSourceUnit,
            },
            CapitalStructure {
                category: CorporateActionCategory,
                tradable_before: FiniteNumber,
                tradable_after: FiniteNumber,
                total_before: FiniteNumber,
                total_after: FiniteNumber,
                unit: UnverifiedSourceUnit,
            },
            WarrantGrant {
                category: CorporateActionCategory,
                exercise_price: Price,
                source_quantity: FiniteNumber,
                source_quantity_unit: UnverifiedSourceUnit,
            },
        }

        match Wire::deserialize(deserializer)? {
            Wire::Distribution {
                cash_per_share,
                bonus_per_share,
                rights_per_share,
                rights_price,
            } => Self::distribution(
                cash_per_share,
                bonus_per_share,
                rights_per_share,
                rights_price,
            ),
            Wire::CapitalRescaling { ratio } => {
                Self::capital_rescaling(CorporateActionCategory::CapitalRescaling, ratio)
            }
            Wire::NonTradableReverseSplit { ratio } => {
                Self::capital_rescaling(CorporateActionCategory::NonTradableReverseSplit, ratio)
            }
            Wire::ProviderNativeRatio {
                category,
                source_ratio,
                source_ratio_unit,
            } => Self::provider_native_ratio(category, source_ratio, source_ratio_unit),
            Wire::CapitalStructure {
                category,
                tradable_before,
                tradable_after,
                total_before,
                total_after,
                unit,
            } => Self::capital_structure(
                category,
                tradable_before,
                tradable_after,
                total_before,
                total_after,
                unit,
            ),
            Wire::WarrantGrant {
                category,
                exercise_price,
                source_quantity,
                source_quantity_unit,
            } => Self::warrant_grant(
                category,
                exercise_price,
                source_quantity,
                source_quantity_unit,
            ),
        }
        .map_err(de::Error::custom)
    }
}

impl CorporateActionTerms {
    pub fn distribution(
        cash_per_share: Option<FiniteNumber>,
        bonus_per_share: Option<FiniteNumber>,
        rights_per_share: Option<FiniteNumber>,
        rights_price: Option<Price>,
    ) -> Result<Self, CoreError> {
        for (field, value) in [
            ("cash_per_share", cash_per_share),
            ("bonus_per_share", bonus_per_share),
            ("rights_per_share", rights_per_share),
        ] {
            if let Some(value) = value {
                if value.get() < 0.0 {
                    return Err(CoreError::InvalidValue {
                        field,
                        value: value.get().to_string(),
                        reason: "must be non-negative",
                    });
                }
            }
        }
        if ![cash_per_share, bonus_per_share, rights_per_share]
            .into_iter()
            .flatten()
            .any(|value| value.get() > 0.0)
        {
            return Err(CoreError::InvalidRequest(
                "distribution requires at least one positive per-share term".into(),
            ));
        }
        if rights_price.is_some() && !rights_per_share.is_some_and(|quantity| quantity.get() > 0.0)
        {
            return Err(CoreError::InvalidRequest(
                "rights price requires a positive rights-per-share quantity".into(),
            ));
        }
        Ok(Self::Distribution {
            cash_per_share,
            bonus_per_share,
            rights_per_share,
            rights_price,
        })
    }

    pub fn capital_rescaling(
        category: CorporateActionCategory,
        ratio: Ratio,
    ) -> Result<Self, CoreError> {
        if ratio.unit() != RatioUnit::Decimal || ratio.get() <= 0.0 || ratio.get() == 1.0 {
            return Err(CoreError::InvalidValue {
                field: "corporate_action_ratio",
                value: ratio.get().to_string(),
                reason: "must be a positive non-identity decimal ratio",
            });
        }
        match category {
            CorporateActionCategory::CapitalRescaling => Ok(Self::CapitalRescaling { ratio }),
            CorporateActionCategory::NonTradableReverseSplit => {
                Ok(Self::NonTradableReverseSplit { ratio })
            }
            _ => Err(CoreError::InvalidRequest(
                "only split categories use split terms".into(),
            )),
        }
    }

    /// Preserves a provider-native ratio whose physical meaning or scale is not verified.
    pub fn provider_native_ratio(
        category: CorporateActionCategory,
        source_ratio: FiniteNumber,
        source_ratio_unit: UnverifiedSourceUnit,
    ) -> Result<Self, CoreError> {
        if !matches!(
            category,
            CorporateActionCategory::CapitalRescaling
                | CorporateActionCategory::NonTradableReverseSplit
        ) {
            return Err(CoreError::InvalidRequest(
                "category does not use provider-native ratio terms".into(),
            ));
        }
        if source_ratio.get() <= 0.0 {
            return Err(CoreError::InvalidValue {
                field: "source_ratio",
                value: source_ratio.get().to_string(),
                reason: "must be positive",
            });
        }
        Ok(Self::ProviderNativeRatio {
            category,
            source_ratio,
            source_ratio_unit,
        })
    }

    pub fn capital_structure(
        category: CorporateActionCategory,
        tradable_before: FiniteNumber,
        tradable_after: FiniteNumber,
        total_before: FiniteNumber,
        total_after: FiniteNumber,
        unit: UnverifiedSourceUnit,
    ) -> Result<Self, CoreError> {
        if !matches!(
            category,
            CorporateActionCategory::BonusRightsListing
                | CorporateActionCategory::NonTradableShareListing
                | CorporateActionCategory::UnknownCapitalChange
                | CorporateActionCategory::CapitalChange
                | CorporateActionCategory::AdditionalIssuance
                | CorporateActionCategory::ShareRepurchase
                | CorporateActionCategory::AdditionalIssuanceListing
                | CorporateActionCategory::TransferredAllotmentListing
                | CorporateActionCategory::ConvertibleBondListing
        ) {
            return Err(CoreError::InvalidRequest(
                "category does not use capital-structure terms".into(),
            ));
        }
        for (field, value) in [
            ("tradable_before", tradable_before),
            ("tradable_after", tradable_after),
            ("total_before", total_before),
            ("total_after", total_after),
        ] {
            if value.get() < 0.0 {
                return Err(CoreError::InvalidValue {
                    field,
                    value: value.get().to_string(),
                    reason: "must be non-negative",
                });
            }
        }
        Ok(Self::CapitalStructure {
            category,
            tradable_before,
            tradable_after,
            total_before,
            total_after,
            unit,
        })
    }

    pub fn warrant_grant(
        category: CorporateActionCategory,
        exercise_price: Price,
        source_quantity: FiniteNumber,
        source_quantity_unit: UnverifiedSourceUnit,
    ) -> Result<Self, CoreError> {
        if !matches!(
            category,
            CorporateActionCategory::SubscriptionWarrantGrant
                | CorporateActionCategory::PutWarrantGrant
        ) {
            return Err(CoreError::InvalidRequest(
                "category does not use warrant-grant terms".into(),
            ));
        }
        if source_quantity.get() <= 0.0 {
            return Err(CoreError::InvalidValue {
                field: "source_quantity",
                value: source_quantity.get().to_string(),
                reason: "must be positive",
            });
        }
        Ok(Self::WarrantGrant {
            category,
            exercise_price,
            source_quantity,
            source_quantity_unit,
        })
    }

    pub fn category(&self) -> CorporateActionCategory {
        match self {
            Self::Distribution { .. } => CorporateActionCategory::Distribution,
            Self::CapitalRescaling { .. } => CorporateActionCategory::CapitalRescaling,
            Self::NonTradableReverseSplit { .. } => {
                CorporateActionCategory::NonTradableReverseSplit
            }
            Self::ProviderNativeRatio { category, .. }
            | Self::CapitalStructure { category, .. }
            | Self::WarrantGrant { category, .. } => *category,
        }
    }

    fn validate(&self) -> Result<(), CoreError> {
        match self {
            Self::Distribution {
                cash_per_share,
                bonus_per_share,
                rights_per_share,
                rights_price,
            } => Self::distribution(
                *cash_per_share,
                *bonus_per_share,
                *rights_per_share,
                *rights_price,
            )
            .map(|_| ()),
            Self::CapitalRescaling { ratio } => {
                Self::capital_rescaling(CorporateActionCategory::CapitalRescaling, *ratio)
                    .map(|_| ())
            }
            Self::NonTradableReverseSplit { ratio } => {
                Self::capital_rescaling(CorporateActionCategory::NonTradableReverseSplit, *ratio)
                    .map(|_| ())
            }
            Self::ProviderNativeRatio {
                category,
                source_ratio,
                source_ratio_unit,
            } => Self::provider_native_ratio(*category, *source_ratio, *source_ratio_unit)
                .map(|_| ()),
            Self::CapitalStructure {
                category,
                tradable_before,
                tradable_after,
                total_before,
                total_after,
                unit,
            } => Self::capital_structure(
                *category,
                *tradable_before,
                *tradable_after,
                *total_before,
                *total_after,
                *unit,
            )
            .map(|_| ()),
            Self::WarrantGrant {
                category,
                exercise_price,
                source_quantity,
                source_quantity_unit,
            } => Self::warrant_grant(
                *category,
                *exercise_price,
                *source_quantity,
                *source_quantity_unit,
            )
            .map(|_| ()),
        }
    }
}

/// One provider-neutral, evidence-preserving security lifecycle event.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CorporateAction {
    instrument: InstrumentId,
    category: CorporateActionCategory,
    effective_on: IsoDate,
    record_on: Option<IsoDate>,
    ex_on: Option<IsoDate>,
    payable_on: Option<IsoDate>,
    status: CorporateActionStatus,
    terms: CorporateActionTerms,
    evidence: SourceEvidence,
}

impl CorporateAction {
    pub fn new(
        instrument: InstrumentId,
        category: CorporateActionCategory,
        effective_on: IsoDate,
        status: CorporateActionStatus,
        terms: CorporateActionTerms,
        evidence: SourceEvidence,
    ) -> Result<Self, CoreError> {
        terms.validate()?;
        if terms.category() != category {
            return Err(CoreError::InvalidRequest(
                "corporate-action category does not match its terms".into(),
            ));
        }
        Ok(Self {
            instrument,
            category,
            effective_on,
            record_on: None,
            ex_on: None,
            payable_on: None,
            status,
            terms,
            evidence,
        })
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn category(&self) -> CorporateActionCategory {
        self.category
    }

    pub fn effective_on(&self) -> &IsoDate {
        &self.effective_on
    }

    /// Adds optional source-published record, ex-rights and payable dates.
    pub fn with_dates(
        mut self,
        record_on: Option<IsoDate>,
        ex_on: Option<IsoDate>,
        payable_on: Option<IsoDate>,
    ) -> Self {
        self.record_on = record_on;
        self.ex_on = ex_on;
        self.payable_on = payable_on;
        self
    }

    pub fn record_on(&self) -> Option<&IsoDate> {
        self.record_on.as_ref()
    }

    pub fn ex_on(&self) -> Option<&IsoDate> {
        self.ex_on.as_ref()
    }

    pub fn payable_on(&self) -> Option<&IsoDate> {
        self.payable_on.as_ref()
    }

    pub fn status(&self) -> CorporateActionStatus {
        self.status
    }

    /// Whether this source fact may explain an observed historical discontinuity.
    ///
    /// Proposed, cancelled and unknown records remain representable so providers
    /// do not have to erase source-published lifecycle state.
    pub fn is_implemented(&self) -> bool {
        self.status == CorporateActionStatus::Implemented
    }

    pub fn terms(&self) -> &CorporateActionTerms {
        &self.terms
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

#[derive(Deserialize)]
struct CorporateActionWire {
    instrument: InstrumentId,
    category: CorporateActionCategory,
    effective_on: IsoDate,
    record_on: Option<IsoDate>,
    ex_on: Option<IsoDate>,
    payable_on: Option<IsoDate>,
    status: CorporateActionStatus,
    terms: CorporateActionTerms,
    evidence: SourceEvidence,
}

impl<'de> Deserialize<'de> for CorporateAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CorporateActionWire::deserialize(deserializer)?;
        Self::new(
            wire.instrument,
            wire.category,
            wire.effective_on,
            wire.status,
            wire.terms,
            wire.evidence,
        )
        .map(|action| action.with_dates(wire.record_on, wire.ex_on, wire.payable_on))
        .map_err(de::Error::custom)
    }
}

impl SourcedRecord for CorporateAction {
    fn provider_id(&self) -> crate::ProviderId {
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

/// Complete corporate-action history request for one instrument and optional range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CorporateActionRequest {
    instrument: InstrumentId,
    start: Option<IsoDate>,
    end: Option<IsoDate>,
}

impl CorporateActionRequest {
    pub fn new(instrument: InstrumentId) -> Self {
        Self {
            instrument,
            start: None,
            end: None,
        }
    }

    pub fn with_range(mut self, start: IsoDate, end: IsoDate) -> Result<Self, CoreError> {
        if start > end {
            return Err(CoreError::InvalidRequest(
                "corporate-action range start must not exceed end".into(),
            ));
        }
        self.start = Some(start);
        self.end = Some(end);
        Ok(self)
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn start(&self) -> Option<&IsoDate> {
        self.start.as_ref()
    }

    pub fn end(&self) -> Option<&IsoDate> {
        self.end.as_ref()
    }
}

#[derive(Deserialize)]
struct CorporateActionRequestWire {
    instrument: InstrumentId,
    start: Option<IsoDate>,
    end: Option<IsoDate>,
}

impl<'de> Deserialize<'de> for CorporateActionRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CorporateActionRequestWire::deserialize(deserializer)?;
        let mut request = Self::new(wire.instrument);
        match (wire.start, wire.end) {
            (Some(start), Some(end)) => {
                request = request.with_range(start, end).map_err(de::Error::custom)?;
            }
            (None, None) => {}
            _ => {
                return Err(de::Error::custom(
                    "corporate-action range requires both start and end",
                ));
            }
        }
        Ok(request)
    }
}

/// One corporate-action batch together with the exact request coverage it proves.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CorporateActionResponse {
    coverage: CorporateActionRequest,
    admission_as_of: IsoDate,
    evidence: SourceEvidence,
    batch: DataBatch<CorporateAction>,
}

impl CorporateActionResponse {
    pub fn new(
        coverage: CorporateActionRequest,
        admission_as_of: IsoDate,
        evidence: SourceEvidence,
        batch: DataBatch<CorporateAction>,
    ) -> Result<Self, CoreError> {
        validate_response_evidence(&coverage, &admission_as_of, &evidence, &batch)?;
        Ok(Self {
            coverage,
            admission_as_of,
            evidence,
            batch,
        })
    }

    pub fn coverage(&self) -> &CorporateActionRequest {
        &self.coverage
    }

    /// Explicit calendar boundary used to reject future lifecycle records.
    pub fn admission_as_of(&self) -> &IsoDate {
        &self.admission_as_of
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }

    pub fn batch(&self) -> &DataBatch<CorporateAction> {
        &self.batch
    }

    /// Projects only implemented actions for continuity and adjustment logic.
    pub fn implemented_actions(&self) -> impl Iterator<Item = &CorporateAction> {
        self.batch
            .records()
            .iter()
            .filter(|record| record.is_implemented())
    }

    pub fn into_batch(self) -> DataBatch<CorporateAction> {
        self.batch
    }

    pub fn into_parts(
        self,
    ) -> (
        CorporateActionRequest,
        IsoDate,
        SourceEvidence,
        DataBatch<CorporateAction>,
    ) {
        (
            self.coverage,
            self.admission_as_of,
            self.evidence,
            self.batch,
        )
    }
}

fn validate_response_evidence(
    coverage: &CorporateActionRequest,
    admission_as_of: &IsoDate,
    evidence: &SourceEvidence,
    batch: &DataBatch<CorporateAction>,
) -> Result<(), CoreError> {
    if !batch.quality().is_complete() {
        return Err(CoreError::InvalidRequest(
            "corporate-action response must be complete".into(),
        ));
    }
    if coverage
        .start()
        .is_some_and(|start| start > admission_as_of)
        || coverage.end().is_some_and(|end| end > admission_as_of)
    {
        return Err(CoreError::InvalidRequest(
            "corporate-action response coverage extends beyond admission_as_of".into(),
        ));
    }
    let provenance = batch.provenance();
    let batch_id = provenance.batch_id().ok_or_else(|| {
        CoreError::InvalidRequest("corporate-action response has no batch ID".into())
    })?;
    if evidence.batch_id() != batch_id {
        return Err(CoreError::InvalidRequest(
            "corporate-action response evidence batch ID does not match provenance".into(),
        ));
    }
    if evidence.observed_at() != provenance.fetched_at() {
        return Err(CoreError::InvalidRequest(
            "corporate-action response observation time does not match provenance".into(),
        ));
    }
    if evidence.source_at() != provenance.source_at() {
        return Err(CoreError::InvalidRequest(
            "corporate-action response source time does not match provenance".into(),
        ));
    }
    let observed_time = crate::EvidenceTimestamp::parse_instant(evidence.observed_at())?;
    let source_time = evidence
        .source_at()
        .map(crate::EvidenceTimestamp::parse)
        .transpose()?;
    if source_time.is_some_and(|source| observed_time.duration_since(source).is_none()) {
        return Err(CoreError::InvalidRequest(
            "corporate-action response source time is later than observation time".into(),
        ));
    }

    let mut previous_identity = None;
    for record in batch.records() {
        if record.instrument() != coverage.instrument() {
            return Err(CoreError::InvalidRequest(
                "corporate-action record instrument is outside response coverage".into(),
            ));
        }
        if record.effective_on() > admission_as_of {
            return Err(CoreError::InvalidRequest(
                "corporate-action response contains a future effective date".into(),
            ));
        }
        if coverage
            .start()
            .is_some_and(|start| record.effective_on() < start)
            || coverage
                .end()
                .is_some_and(|end| record.effective_on() > end)
        {
            return Err(CoreError::InvalidRequest(
                "corporate-action record date is outside response coverage".into(),
            ));
        }
        if record.evidence() != evidence {
            return Err(CoreError::InvalidRequest(
                "corporate-action record evidence does not match response evidence".into(),
            ));
        }
        let identity = (record.effective_on().clone(), record.category());
        if previous_identity
            .as_ref()
            .is_some_and(|previous| previous >= &identity)
        {
            return Err(CoreError::InvalidRequest(
                "corporate-action response contains duplicate or unordered identities".into(),
            ));
        }
        previous_identity = Some(identity);
    }
    Ok(())
}

#[derive(Deserialize)]
struct CorporateActionResponseWire {
    coverage: CorporateActionRequest,
    admission_as_of: IsoDate,
    evidence: SourceEvidence,
    batch: DataBatch<CorporateAction>,
}

impl<'de> Deserialize<'de> for CorporateActionResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = CorporateActionResponseWire::deserialize(deserializer)?;
        Self::new(
            wire.coverage,
            wire.admission_as_of,
            wire.evidence,
            wire.batch,
        )
        .map_err(de::Error::custom)
    }
}

pub trait CorporateActions {
    type Error: std::error::Error + Send + Sync + 'static;

    fn corporate_actions(
        &self,
        request: &CorporateActionRequest,
    ) -> Result<CorporateActionResponse, Self::Error>;
}
