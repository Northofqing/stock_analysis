use crate::{Announcement, DataBatch, IsoDate, PositiveU32};
use serde::{de, Deserialize, Deserializer, Serialize};

/// Bounded native whole-market announcement discovery request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarketAnnouncementRequest {
    start: IsoDate,
    end: IsoDate,
    limit: PositiveU32,
}

impl MarketAnnouncementRequest {
    pub fn new(start: IsoDate, end: IsoDate, limit: PositiveU32) -> Result<Self, crate::CoreError> {
        if start > end {
            return Err(crate::CoreError::InvalidRequest(
                "market announcement start must not exceed end".into(),
            ));
        }
        if limit.get() > 300 {
            return Err(crate::CoreError::InvalidRequest(
                "market announcement limit must be at most 300".into(),
            ));
        }
        Ok(Self { start, end, limit })
    }

    pub fn start(&self) -> &IsoDate {
        &self.start
    }

    pub fn end(&self) -> &IsoDate {
        &self.end
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }
}

#[derive(Deserialize)]
struct MarketAnnouncementRequestWire {
    start: IsoDate,
    end: IsoDate,
    limit: PositiveU32,
}

impl<'de> Deserialize<'de> for MarketAnnouncementRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MarketAnnouncementRequestWire::deserialize(deserializer)?;
        Self::new(wire.start, wire.end, wire.limit).map_err(de::Error::custom)
    }
}

/// Native whole-market announcement discovery.
pub trait MarketAnnouncements {
    type Error: std::error::Error + Send + Sync + 'static;

    fn market_announcements(
        &self,
        request: &MarketAnnouncementRequest,
    ) -> Result<DataBatch<Announcement>, Self::Error>;
}
