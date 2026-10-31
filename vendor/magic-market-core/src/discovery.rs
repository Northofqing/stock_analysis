use crate::{
    BoardCategory, BoardMembership, DataBatch, DragonTigerEntry, Exchange, IsoDate, NonEmptyText,
    PositiveU32, SourceEvidence, SourcedRecord,
};
use serde::{Deserialize, Serialize};

const MAX_DISCOVERY_LIMIT: u32 = 10_000;

/// A provider-scoped market board and its proved source membership count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BoardDefinitionWire")]
pub struct BoardDefinition {
    board_code: NonEmptyText,
    board_name: NonEmptyText,
    category: BoardCategory,
    member_count: PositiveU32,
    evidence: SourceEvidence,
}

impl BoardDefinition {
    pub fn new(
        board_code: NonEmptyText,
        board_name: NonEmptyText,
        category: BoardCategory,
        member_count: PositiveU32,
        evidence: SourceEvidence,
    ) -> Result<Self, crate::CoreError> {
        Ok(Self {
            board_code,
            board_name,
            category,
            member_count,
            evidence,
        })
    }

    pub fn board_code(&self) -> &NonEmptyText {
        &self.board_code
    }

    pub fn board_name(&self) -> &NonEmptyText {
        &self.board_name
    }

    pub fn category(&self) -> BoardCategory {
        self.category
    }

    pub fn member_count(&self) -> PositiveU32 {
        self.member_count
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

#[derive(Deserialize)]
struct BoardDefinitionWire {
    board_code: NonEmptyText,
    board_name: NonEmptyText,
    category: BoardCategory,
    member_count: PositiveU32,
    evidence: SourceEvidence,
}

impl TryFrom<BoardDefinitionWire> for BoardDefinition {
    type Error = crate::CoreError;

    fn try_from(value: BoardDefinitionWire) -> Result<Self, Self::Error> {
        Self::new(
            value.board_code,
            value.board_name,
            value.category,
            value.member_count,
            value.evidence,
        )
    }
}

impl SourcedRecord for BoardDefinition {
    fn provider_id(&self) -> crate::ProviderId {
        self.evidence.provider()
    }

    fn evidence_batch_id(&self) -> &str {
        self.evidence.batch_id()
    }
}

/// Explicit trading-day request for full-market dragon-tiger discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DragonTigerDiscoveryRequestWire")]
pub struct DragonTigerDiscoveryRequest {
    trading_date: IsoDate,
    exchange: Option<Exchange>,
    limit: PositiveU32,
}

impl DragonTigerDiscoveryRequest {
    pub fn new(trading_date: IsoDate, limit: PositiveU32) -> Result<Self, crate::CoreError> {
        validate_limit("dragon-tiger discovery", limit)?;
        Ok(Self {
            trading_date,
            exchange: None,
            limit,
        })
    }

    pub fn with_exchange(mut self, exchange: Exchange) -> Self {
        self.exchange = Some(exchange);
        self
    }

    pub fn trading_date(&self) -> &IsoDate {
        &self.trading_date
    }

    pub fn exchange(&self) -> Option<Exchange> {
        self.exchange
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }
}

#[derive(Deserialize)]
struct DragonTigerDiscoveryRequestWire {
    trading_date: IsoDate,
    exchange: Option<Exchange>,
    limit: PositiveU32,
}

impl TryFrom<DragonTigerDiscoveryRequestWire> for DragonTigerDiscoveryRequest {
    type Error = crate::CoreError;

    fn try_from(value: DragonTigerDiscoveryRequestWire) -> Result<Self, Self::Error> {
        let request = Self::new(value.trading_date, value.limit)?;
        Ok(match value.exchange {
            Some(exchange) => request.with_exchange(exchange),
            None => request,
        })
    }
}

/// Bounded directory request for one board category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BoardDirectoryRequestWire")]
pub struct BoardDirectoryRequest {
    category: BoardCategory,
    limit: PositiveU32,
}

impl BoardDirectoryRequest {
    pub fn new(category: BoardCategory, limit: PositiveU32) -> Result<Self, crate::CoreError> {
        validate_limit("board directory", limit)?;
        Ok(Self { category, limit })
    }

    pub fn category(&self) -> BoardCategory {
        self.category
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }
}

#[derive(Deserialize)]
struct BoardDirectoryRequestWire {
    category: BoardCategory,
    limit: PositiveU32,
}

impl TryFrom<BoardDirectoryRequestWire> for BoardDirectoryRequest {
    type Error = crate::CoreError;

    fn try_from(value: BoardDirectoryRequestWire) -> Result<Self, Self::Error> {
        Self::new(value.category, value.limit)
    }
}

/// Bounded request for the members of one provider-scoped board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BoardConstituentRequestWire")]
pub struct BoardConstituentRequest {
    board_code: NonEmptyText,
    limit: PositiveU32,
}

impl BoardConstituentRequest {
    pub fn new(board_code: NonEmptyText, limit: PositiveU32) -> Result<Self, crate::CoreError> {
        validate_limit("board constituent", limit)?;
        Ok(Self { board_code, limit })
    }

    pub fn board_code(&self) -> &NonEmptyText {
        &self.board_code
    }

    pub fn limit(&self) -> PositiveU32 {
        self.limit
    }
}

#[derive(Deserialize)]
struct BoardConstituentRequestWire {
    board_code: NonEmptyText,
    limit: PositiveU32,
}

impl TryFrom<BoardConstituentRequestWire> for BoardConstituentRequest {
    type Error = crate::CoreError;

    fn try_from(value: BoardConstituentRequestWire) -> Result<Self, Self::Error> {
        Self::new(value.board_code, value.limit)
    }
}

fn validate_limit(family: &str, limit: PositiveU32) -> Result<(), crate::CoreError> {
    if limit.get() > MAX_DISCOVERY_LIMIT {
        return Err(crate::CoreError::InvalidRequest(format!(
            "{family} limit must be at most {MAX_DISCOVERY_LIMIT}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MarketDiscoveryCapabilities {
    pub dragon_tiger_discovery: bool,
    pub board_directory: bool,
    pub board_memberships: bool,
    pub board_constituents: bool,
}

pub trait DragonTigerDiscovery {
    type Error: std::error::Error + Send + Sync + 'static;

    fn discover_dragon_tiger(
        &self,
        request: &DragonTigerDiscoveryRequest,
    ) -> Result<DataBatch<DragonTigerEntry>, Self::Error>;
}

pub trait BoardDirectoryProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn boards(
        &self,
        request: &BoardDirectoryRequest,
    ) -> Result<DataBatch<BoardDefinition>, Self::Error>;
}

pub trait BoardConstituentProvider {
    type Error: std::error::Error + Send + Sync + 'static;

    fn board_constituents(
        &self,
        request: &BoardConstituentRequest,
    ) -> Result<DataBatch<BoardMembership>, Self::Error>;
}
