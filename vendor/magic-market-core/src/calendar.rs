use crate::{
    DataBatch, HttpsUrl, IsoDate, NonEmptyText, PositiveU32, SourceEvidence, SourcedRecord,
};
use serde::{de, Deserialize, Deserializer, Serialize};

/// Bounded latest economic-release request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EconomicCalendarRequest {
    limit: PositiveU32,
    country: Option<NonEmptyText>,
}

impl EconomicCalendarRequest {
    pub fn new(limit: PositiveU32) -> Result<Self, crate::CoreError> {
        if limit.get() > 20 {
            return Err(crate::CoreError::InvalidRequest(
                "economic calendar limit must be at most 20".into(),
            ));
        }
        Ok(Self {
            limit,
            country: None,
        })
    }

    pub fn with_country(mut self, country: impl Into<String>) -> Result<Self, crate::CoreError> {
        self.country = Some(NonEmptyText::new(country)?);
        Ok(self)
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }

    pub fn country(&self) -> Option<&NonEmptyText> {
        self.country.as_ref()
    }
}

impl<'de> Deserialize<'de> for EconomicCalendarRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            limit: PositiveU32,
            country: Option<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let mut request = Self::new(wire.limit).map_err(de::Error::custom)?;
        if let Some(country) = wire.country {
            request = request.with_country(country).map_err(de::Error::custom)?;
        }
        Ok(request)
    }
}

/// One scheduled or released macroeconomic indicator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicEvent {
    pub event_id: NonEmptyText,
    pub indicator_id: PositiveU32,
    pub country: NonEmptyText,
    pub name: NonEmptyText,
    pub period: Option<NonEmptyText>,
    pub scheduled_at: NonEmptyText,
    pub released_at: NonEmptyText,
    pub previous: Option<NonEmptyText>,
    pub consensus: Option<NonEmptyText>,
    pub actual: Option<NonEmptyText>,
    pub revised: Option<NonEmptyText>,
    pub unit: Option<NonEmptyText>,
    pub importance: PositiveU32,
    pub impact: Option<NonEmptyText>,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for EconomicEvent {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

pub trait EconomicCalendarProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn economic_calendar(
        &self,
        request: &EconomicCalendarRequest,
    ) -> Result<DataBatch<EconomicEvent>, Self::Error>;
}

/// CFFEX equity-index-futures products admitted by the delivery-notice parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FuturesProduct {
    If,
    Ih,
    Ic,
    Im,
}

/// Request for one CFFEX contract month's official delivery notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FuturesDeliveryRequest {
    year: PositiveU32,
    month: PositiveU32,
}

impl FuturesDeliveryRequest {
    pub fn new(year: PositiveU32, month: PositiveU32) -> Result<Self, crate::CoreError> {
        if !(2000..=9999).contains(&year.get()) {
            return Err(crate::CoreError::InvalidRequest(
                "futures delivery year must be in 2000..=9999".into(),
            ));
        }
        if month.get() > 12 {
            return Err(crate::CoreError::InvalidRequest(
                "futures delivery month must be in 1..=12".into(),
            ));
        }
        Ok(Self { year, month })
    }

    pub fn year(&self) -> PositiveU32 {
        self.year
    }

    pub fn month(&self) -> PositiveU32 {
        self.month
    }
}

impl<'de> Deserialize<'de> for FuturesDeliveryRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            year: PositiveU32,
            month: PositiveU32,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.year, wire.month).map_err(de::Error::custom)
    }
}

/// Delivery method supported by the normalized calendar contract.
///
/// `NotProvided` means the event source proves the delivery event and date but
/// does not independently prove the settlement method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FuturesDeliveryMethod {
    Cash,
    NotProvided,
}

/// One contract delivery event proved by an official CFFEX notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuturesDeliveryEvent {
    pub product: FuturesProduct,
    pub contract_code: NonEmptyText,
    /// Last trading date when the source explicitly proves it.
    ///
    /// An official delivery notice can prove the delivery date without
    /// independently proving the last trading date.
    pub last_trading_date: Option<IsoDate>,
    pub delivery_date: IsoDate,
    pub method: FuturesDeliveryMethod,
    pub notice_url: HttpsUrl,
    pub evidence: SourceEvidence,
}

impl SourcedRecord for FuturesDeliveryEvent {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

pub trait FuturesDeliveryCalendar {
    type Error: std::error::Error + Send + Sync + 'static;

    fn futures_delivery_calendar(
        &self,
        request: &FuturesDeliveryRequest,
    ) -> Result<DataBatch<FuturesDeliveryEvent>, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CalendarCapabilities {
    pub economic_releases: bool,
    pub futures_delivery: bool,
}

#[cfg(test)]
#[path = "../tests/internal/calendar_tests.rs"]
mod tests;
