use crate::{
    CoreError, DataBatch, FiniteNumber, IsoDate, NonEmptyText, PositiveU32, ProviderId,
    SourceEvidence, SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::collections::HashSet;

/// Frequency used by an official economic series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EconomicFrequency {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Annual,
    Irregular,
}

/// Stable, provider-qualified economic-series identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct EconomicSeriesKey {
    provider: ProviderId,
    namespace: NonEmptyText,
    code: NonEmptyText,
}

impl EconomicSeriesKey {
    pub fn new(
        provider: ProviderId,
        namespace: impl Into<String>,
        code: impl Into<String>,
    ) -> Result<Self, CoreError> {
        Ok(Self {
            provider,
            namespace: NonEmptyText::new(namespace)?,
            code: NonEmptyText::new(code)?,
        })
    }

    pub fn provider(&self) -> ProviderId {
        self.provider
    }

    pub fn namespace(&self) -> &str {
        self.namespace.as_str()
    }

    pub fn code(&self) -> &str {
        self.code.as_str()
    }
}

impl<'de> Deserialize<'de> for EconomicSeriesKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            provider: ProviderId,
            namespace: String,
            code: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.provider, wire.namespace, wire.code).map_err(de::Error::custom)
    }
}

/// Checked period label for economic observations.
///
/// The representation is private so callers cannot bypass the checked
/// constructors.
///
/// ```compile_fail
/// use magic_market_core::{EconomicPeriod, PositiveU32};
///
/// let _ = EconomicPeriod::Monthly {
///     year: PositiveU32::new(2025).unwrap(),
///     month: PositiveU32::new(13).unwrap(),
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EconomicPeriod {
    value: EconomicPeriodValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
enum EconomicPeriodValue {
    Daily(IsoDate),
    Weekly {
        year: PositiveU32,
        week: PositiveU32,
    },
    Monthly {
        year: PositiveU32,
        month: PositiveU32,
    },
    Quarterly {
        year: PositiveU32,
        quarter: PositiveU32,
    },
    Annual {
        year: PositiveU32,
    },
    Irregular(NonEmptyText),
}

impl EconomicPeriod {
    pub fn day(value: impl Into<String>) -> Result<Self, CoreError> {
        Ok(Self {
            value: EconomicPeriodValue::Daily(IsoDate::new(value)?),
        })
    }

    pub fn iso_week(year: u32, week: u32) -> Result<Self, CoreError> {
        Ok(Self {
            value: EconomicPeriodValue::Weekly {
                year: checked_year(year)?,
                week: checked_period_part("economic_week", week, 1, 53)?,
            },
        })
    }

    pub fn month(year: u32, month: u32) -> Result<Self, CoreError> {
        Ok(Self {
            value: EconomicPeriodValue::Monthly {
                year: checked_year(year)?,
                month: checked_period_part("economic_month", month, 1, 12)?,
            },
        })
    }

    pub fn quarter(year: u32, quarter: u32) -> Result<Self, CoreError> {
        Ok(Self {
            value: EconomicPeriodValue::Quarterly {
                year: checked_year(year)?,
                quarter: checked_period_part("economic_quarter", quarter, 1, 4)?,
            },
        })
    }

    pub fn year(year: u32) -> Result<Self, CoreError> {
        Ok(Self {
            value: EconomicPeriodValue::Annual {
                year: checked_year(year)?,
            },
        })
    }

    pub fn irregular(value: impl Into<String>) -> Result<Self, CoreError> {
        Ok(Self {
            value: EconomicPeriodValue::Irregular(NonEmptyText::new(value)?),
        })
    }

    pub fn frequency(&self) -> EconomicFrequency {
        match &self.value {
            EconomicPeriodValue::Daily(_) => EconomicFrequency::Daily,
            EconomicPeriodValue::Weekly { .. } => EconomicFrequency::Weekly,
            EconomicPeriodValue::Monthly { .. } => EconomicFrequency::Monthly,
            EconomicPeriodValue::Quarterly { .. } => EconomicFrequency::Quarterly,
            EconomicPeriodValue::Annual { .. } => EconomicFrequency::Annual,
            EconomicPeriodValue::Irregular(_) => EconomicFrequency::Irregular,
        }
    }

    pub fn as_day(&self) -> Option<&str> {
        match &self.value {
            EconomicPeriodValue::Daily(date) => Some(date.as_str()),
            _ => None,
        }
    }

    pub fn as_iso_week(&self) -> Option<(u32, u32)> {
        match &self.value {
            EconomicPeriodValue::Weekly { year, week } => Some((year.get(), week.get())),
            _ => None,
        }
    }

    pub fn as_month(&self) -> Option<(u32, u32)> {
        match &self.value {
            EconomicPeriodValue::Monthly { year, month } => Some((year.get(), month.get())),
            _ => None,
        }
    }

    pub fn as_quarter(&self) -> Option<(u32, u32)> {
        match &self.value {
            EconomicPeriodValue::Quarterly { year, quarter } => Some((year.get(), quarter.get())),
            _ => None,
        }
    }

    pub fn as_year(&self) -> Option<u32> {
        match &self.value {
            EconomicPeriodValue::Annual { year } => Some(year.get()),
            _ => None,
        }
    }

    pub fn as_irregular(&self) -> Option<&str> {
        match &self.value {
            EconomicPeriodValue::Irregular(label) => Some(label.as_str()),
            _ => None,
        }
    }

    fn comparison_key(&self) -> (u8, u32, u32, &str) {
        match &self.value {
            EconomicPeriodValue::Daily(date) => (0, 0, 0, date.as_str()),
            EconomicPeriodValue::Weekly { year, week } => (1, year.get(), week.get(), ""),
            EconomicPeriodValue::Monthly { year, month } => (2, year.get(), month.get(), ""),
            EconomicPeriodValue::Quarterly { year, quarter } => (3, year.get(), quarter.get(), ""),
            EconomicPeriodValue::Annual { year } => (4, year.get(), 0, ""),
            EconomicPeriodValue::Irregular(label) => (5, 0, 0, label.as_str()),
        }
    }
}

impl Serialize for EconomicPeriod {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.value.serialize(serializer)
    }
}

impl Ord for EconomicPeriod {
    fn cmp(&self, other: &Self) -> Ordering {
        self.comparison_key().cmp(&other.comparison_key())
    }
}

impl PartialOrd for EconomicPeriod {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'de> Deserialize<'de> for EconomicPeriod {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum Wire {
            Daily(String),
            Weekly { year: u32, week: u32 },
            Monthly { year: u32, month: u32 },
            Quarterly { year: u32, quarter: u32 },
            Annual { year: u32 },
            Irregular(String),
        }

        match Wire::deserialize(deserializer)? {
            Wire::Daily(date) => Self::day(date),
            Wire::Weekly { year, week } => Self::iso_week(year, week),
            Wire::Monthly { year, month } => Self::month(year, month),
            Wire::Quarterly { year, quarter } => Self::quarter(year, quarter),
            Wire::Annual { year } => Self::year(year),
            Wire::Irregular(label) => Self::irregular(label),
        }
        .map_err(de::Error::custom)
    }
}

/// Bounded request for one provider and one frequency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EconomicSeriesRequest {
    series: Vec<EconomicSeriesKey>,
    start: EconomicPeriod,
    end: EconomicPeriod,
    max_rows: PositiveU32,
}

impl EconomicSeriesRequest {
    pub fn new(
        series: Vec<EconomicSeriesKey>,
        start: EconomicPeriod,
        end: EconomicPeriod,
        max_rows: PositiveU32,
    ) -> Result<Self, CoreError> {
        if series.is_empty() || series.len() > 100 {
            return Err(CoreError::InvalidRequest(
                "economic-series request accepts 1 through 100 series".into(),
            ));
        }
        if max_rows.get() > 10_000 {
            return Err(CoreError::InvalidRequest(
                "economic-series max_rows must not exceed 10000".into(),
            ));
        }
        let provider = series[0].provider();
        if series.iter().any(|key| key.provider() != provider) {
            return Err(CoreError::InvalidRequest(
                "economic-series request cannot mix providers".into(),
            ));
        }
        let mut seen = HashSet::with_capacity(series.len());
        if series.iter().any(|key| !seen.insert(key.clone())) {
            return Err(CoreError::InvalidRequest(
                "economic-series request contains duplicate series".into(),
            ));
        }
        if start.frequency() != end.frequency() || start > end {
            return Err(CoreError::InvalidRequest(
                "economic-series range must use one frequency with start not after end".into(),
            ));
        }
        Ok(Self {
            series,
            start,
            end,
            max_rows,
        })
    }

    pub fn series(&self) -> &[EconomicSeriesKey] {
        &self.series
    }

    pub fn start(&self) -> &EconomicPeriod {
        &self.start
    }

    pub fn end(&self) -> &EconomicPeriod {
        &self.end
    }

    pub fn max_rows(&self) -> PositiveU32 {
        self.max_rows
    }

    pub fn provider(&self) -> ProviderId {
        self.series[0].provider()
    }
}

impl<'de> Deserialize<'de> for EconomicSeriesRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            series: Vec<EconomicSeriesKey>,
            start: EconomicPeriod,
            end: EconomicPeriod,
            max_rows: PositiveU32,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.series, wire.start, wire.end, wire.max_rows).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomicObservationStatus {
    Present,
    Missing,
    NotApplicable,
    Confidential,
    SourceDefined(NonEmptyText),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EconomicRevisionKind {
    Preliminary,
    Revised,
    Final,
    SourceDefined(NonEmptyText),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicRevision {
    pub kind: EconomicRevisionKind,
    pub label: Option<NonEmptyText>,
}

/// One checked economic observation with source evidence.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EconomicObservation {
    series: EconomicSeriesKey,
    name: NonEmptyText,
    region_code: Option<NonEmptyText>,
    region_name: Option<NonEmptyText>,
    period: EconomicPeriod,
    value: Option<FiniteNumber>,
    unit: NonEmptyText,
    scale: Option<NonEmptyText>,
    seasonal_adjustment: Option<NonEmptyText>,
    status: EconomicObservationStatus,
    released_at: Option<NonEmptyText>,
    revision: Option<EconomicRevision>,
    evidence: SourceEvidence,
}

impl EconomicObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        series: EconomicSeriesKey,
        name: impl Into<String>,
        region_code: Option<NonEmptyText>,
        region_name: Option<NonEmptyText>,
        period: EconomicPeriod,
        value: Option<FiniteNumber>,
        unit: impl Into<String>,
        scale: Option<NonEmptyText>,
        seasonal_adjustment: Option<NonEmptyText>,
        status: EconomicObservationStatus,
        released_at: Option<NonEmptyText>,
        revision: Option<EconomicRevision>,
        evidence: SourceEvidence,
    ) -> Result<Self, CoreError> {
        match (&status, value) {
            (EconomicObservationStatus::Present, Some(_)) => {}
            (EconomicObservationStatus::Present, None) => {
                return Err(CoreError::InvalidRequest(
                    "present economic observation requires a value".into(),
                ));
            }
            (_, None) => {}
            (_, Some(_)) => {
                return Err(CoreError::InvalidRequest(
                    "non-present economic observation cannot contain a value".into(),
                ));
            }
        }
        if series.provider() != evidence.provider() {
            return Err(CoreError::InvalidRequest(
                "economic observation provider must match source evidence".into(),
            ));
        }
        if released_at.as_ref().map(NonEmptyText::as_str) != evidence.source_at() {
            return Err(CoreError::InvalidRequest(
                "economic observation released_at must match source evidence".into(),
            ));
        }
        Ok(Self {
            series,
            name: NonEmptyText::new(name)?,
            region_code,
            region_name,
            period,
            value,
            unit: NonEmptyText::new(unit)?,
            scale,
            seasonal_adjustment,
            status,
            released_at,
            revision,
            evidence,
        })
    }

    pub fn series(&self) -> &EconomicSeriesKey {
        &self.series
    }
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub fn region_code(&self) -> Option<&str> {
        self.region_code.as_ref().map(NonEmptyText::as_str)
    }
    pub fn region_name(&self) -> Option<&str> {
        self.region_name.as_ref().map(NonEmptyText::as_str)
    }
    pub fn period(&self) -> &EconomicPeriod {
        &self.period
    }
    pub fn value(&self) -> Option<FiniteNumber> {
        self.value
    }
    pub fn unit(&self) -> &str {
        self.unit.as_str()
    }
    pub fn scale(&self) -> Option<&str> {
        self.scale.as_ref().map(NonEmptyText::as_str)
    }
    pub fn seasonal_adjustment(&self) -> Option<&str> {
        self.seasonal_adjustment.as_ref().map(NonEmptyText::as_str)
    }
    pub fn status(&self) -> &EconomicObservationStatus {
        &self.status
    }
    pub fn released_at(&self) -> Option<&str> {
        self.released_at.as_ref().map(NonEmptyText::as_str)
    }
    pub fn revision(&self) -> Option<&EconomicRevision> {
        self.revision.as_ref()
    }
    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

impl<'de> Deserialize<'de> for EconomicObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            series: EconomicSeriesKey,
            name: String,
            region_code: Option<NonEmptyText>,
            region_name: Option<NonEmptyText>,
            period: EconomicPeriod,
            value: Option<FiniteNumber>,
            unit: String,
            scale: Option<NonEmptyText>,
            seasonal_adjustment: Option<NonEmptyText>,
            status: EconomicObservationStatus,
            released_at: Option<NonEmptyText>,
            revision: Option<EconomicRevision>,
            evidence: SourceEvidence,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.series,
            wire.name,
            wire.region_code,
            wire.region_name,
            wire.period,
            wire.value,
            wire.unit,
            wire.scale,
            wire.seasonal_adjustment,
            wire.status,
            wire.released_at,
            wire.revision,
            wire.evidence,
        )
        .map_err(de::Error::custom)
    }
}

impl SourcedRecord for EconomicObservation {
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

pub trait EconomicSeriesProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn economic_series(
        &self,
        request: &EconomicSeriesRequest,
    ) -> Result<DataBatch<EconomicObservation>, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EconomicDataCapabilities {
    pub economic_series: bool,
    pub regional_series: bool,
}

fn checked_year(year: u32) -> Result<PositiveU32, CoreError> {
    checked_period_part("economic_year", year, 1900, 9999)
}

fn checked_period_part(
    field: &'static str,
    value: u32,
    minimum: u32,
    maximum: u32,
) -> Result<PositiveU32, CoreError> {
    if !(minimum..=maximum).contains(&value) {
        return Err(CoreError::InvalidValue {
            field,
            value: value.to_string(),
            reason: "is outside the supported range",
        });
    }
    PositiveU32::new(value)
}
