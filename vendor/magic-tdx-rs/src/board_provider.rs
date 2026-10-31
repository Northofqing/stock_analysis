use crate::block::{BlockType, TdxBlockClient};
use crate::reader::block::BlockRecord;
use crate::TdxError;
use magic_market_core::{
    AssetClass, BoardCategory, BoardConstituentProvider, BoardConstituentRequest, BoardDefinition,
    BoardDirectoryProvider, BoardDirectoryRequest, BoardMembership, BoardMembershipProvider,
    DataBatch, Exchange, InstrumentId, MarketDiscoveryCapabilities, NonEmptyText, PositiveU32,
    Provenance, ProviderId, SourceEvidence,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const SOURCE_NAME: &str = "tdx-block-files";
const MAX_REVERSE_REQUESTS: usize = 10_000;

pub trait TdxBoardSource: Send + Sync {
    fn records(&self, block_type: BlockType) -> Result<Vec<BlockRecord>, TdxError>;
}

struct NetworkBoardSource {
    client: TdxBlockClient,
}

impl TdxBoardSource for NetworkBoardSource {
    fn records(&self, block_type: BlockType) -> Result<Vec<BlockRecord>, TdxError> {
        match block_type {
            BlockType::Industry => self.client.get_industry_blocks(),
            BlockType::Concept => self.client.get_concept_blocks(),
            BlockType::Index => Err(TdxError::Unsupported(
                "TDX mixed index blocks are not admitted as normalized boards".into(),
            )),
        }
    }
}

#[derive(Clone)]
pub struct TdxBoardProvider {
    source: Arc<dyn TdxBoardSource>,
}

impl std::fmt::Debug for TdxBoardProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TdxBoardProvider")
            .finish_non_exhaustive()
    }
}

impl TdxBoardProvider {
    pub fn new(ip: &str, port: u16, timeout: f64) -> Self {
        Self::with_source(NetworkBoardSource {
            client: TdxBlockClient::new(ip, port, timeout),
        })
    }

    pub fn with_default(ip: &str) -> Self {
        Self::with_source(NetworkBoardSource {
            client: TdxBlockClient::with_default(ip),
        })
    }

    pub fn with_source(source: impl TdxBoardSource + 'static) -> Self {
        Self {
            source: Arc::new(source),
        }
    }

    pub const fn market_discovery_capabilities() -> MarketDiscoveryCapabilities {
        MarketDiscoveryCapabilities {
            dragon_tiger_discovery: false,
            board_directory: true,
            board_memberships: true,
            board_constituents: true,
        }
    }
}

impl BoardDirectoryProvider for TdxBoardProvider {
    type Error = TdxError;

    fn boards(
        &self,
        request: &BoardDirectoryRequest,
    ) -> Result<DataBatch<BoardDefinition>, Self::Error> {
        let block_type = block_type(request.category())?;
        let validated = validate_records(request.category(), self.source.records(block_type)?)?;
        let context = OperationContext::new("directory")?;
        let mut counts = BTreeMap::<String, u32>::new();
        for (name, _) in validated {
            let count = counts.entry(name).or_default();
            *count = count
                .checked_add(1)
                .ok_or_else(|| TdxError::InvalidData("TDX board member count overflow".into()))?;
        }
        let records = counts
            .into_iter()
            .take(request.limit().get() as usize)
            .map(|(name, member_count)| {
                Ok(BoardDefinition::new(
                    NonEmptyText::new(board_code(request.category(), &name)?)?,
                    NonEmptyText::new(name)?,
                    request.category(),
                    PositiveU32::new(member_count)?,
                    context.evidence()?,
                )?)
            })
            .collect::<Result<Vec<_>, TdxError>>()?;
        context.finish(records)
    }
}

impl BoardConstituentProvider for TdxBoardProvider {
    type Error = TdxError;

    fn board_constituents(
        &self,
        request: &BoardConstituentRequest,
    ) -> Result<DataBatch<BoardMembership>, Self::Error> {
        let (category, requested_name) = parse_board_code(request.board_code().as_str())?;
        let block_type = block_type(category)?;
        let validated = validate_records(category, self.source.records(block_type)?)?;
        let context = OperationContext::new("constituents")?;
        let records = validated
            .into_iter()
            .filter(|(name, _)| name == &requested_name)
            .take(request.limit().get() as usize)
            .map(|(name, code)| {
                Ok(BoardMembership {
                    instrument: source_instrument(&code)?,
                    board_code: NonEmptyText::new(board_code(category, &name)?)?,
                    board_name: NonEmptyText::new(name)?,
                    category,
                    evidence: context.evidence()?,
                })
            })
            .collect::<Result<Vec<_>, TdxError>>()?;
        context.finish(records)
    }
}

impl BoardMembershipProvider for TdxBoardProvider {
    type Error = TdxError;

    fn board_memberships(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<BoardMembership>, Self::Error> {
        if instruments.is_empty() || instruments.len() > MAX_REVERSE_REQUESTS {
            return Err(TdxError::InvalidData(format!(
                "TDX reverse board request must contain 1..={MAX_REVERSE_REQUESTS} instruments"
            )));
        }
        let mut requested = HashSet::with_capacity(instruments.len());
        let mut requested_by_code = HashMap::with_capacity(instruments.len());
        for instrument in instruments {
            validate_requested_instrument(instrument)?;
            if !requested.insert(instrument.clone()) {
                return Err(TdxError::InvalidData(format!(
                    "duplicate TDX reverse board request for {:?}.{}",
                    instrument.exchange(),
                    instrument.code()
                )));
            }
            requested_by_code.insert(instrument.code().to_owned(), instrument.clone());
        }

        let industry = validate_records(
            BoardCategory::Industry,
            self.source.records(BlockType::Industry)?,
        )?;
        let concept = validate_records(
            BoardCategory::Concept,
            self.source.records(BlockType::Concept)?,
        )?;
        let context = OperationContext::new("memberships")?;
        let records = [
            (BoardCategory::Industry, industry),
            (BoardCategory::Concept, concept),
        ]
        .into_iter()
        .flat_map(|(category, records)| {
            records
                .into_iter()
                .map(move |(name, code)| (category, name, code))
        })
        .filter_map(|(category, name, code)| {
            requested_by_code
                .get(&code)
                .cloned()
                .map(|instrument| (category, name, instrument))
        })
        .map(|(category, name, instrument)| {
            Ok(BoardMembership {
                instrument,
                board_code: NonEmptyText::new(board_code(category, &name)?)?,
                board_name: NonEmptyText::new(name)?,
                category,
                evidence: context.evidence()?,
            })
        })
        .collect::<Result<Vec<_>, TdxError>>()?;
        context.finish(records)
    }
}

fn block_type(category: BoardCategory) -> Result<BlockType, TdxError> {
    match category {
        BoardCategory::Industry => Ok(BlockType::Industry),
        BoardCategory::Concept => Ok(BlockType::Concept),
        BoardCategory::Region | BoardCategory::Unknown => Err(TdxError::Unsupported(format!(
            "TDX normalized boards do not support category {category:?}"
        ))),
    }
}

fn board_code(category: BoardCategory, name: &str) -> Result<String, TdxError> {
    match category {
        BoardCategory::Industry => Ok(format!("tdx:industry:{name}")),
        BoardCategory::Concept => Ok(format!("tdx:concept:{name}")),
        BoardCategory::Region | BoardCategory::Unknown => Err(TdxError::Unsupported(format!(
            "TDX board identity does not support category {category:?}"
        ))),
    }
}

fn parse_board_code(value: &str) -> Result<(BoardCategory, String), TdxError> {
    let (category, name) = if let Some(name) = value.strip_prefix("tdx:industry:") {
        (BoardCategory::Industry, name)
    } else if let Some(name) = value.strip_prefix("tdx:concept:") {
        (BoardCategory::Concept, name)
    } else {
        return Err(TdxError::Unsupported(format!(
            "unsupported TDX board identity {value:?}"
        )));
    };
    Ok((category, NonEmptyText::new(name)?.into_string()))
}

fn validate_records(
    category: BoardCategory,
    records: Vec<BlockRecord>,
) -> Result<Vec<(String, String)>, TdxError> {
    if records.is_empty() {
        return Err(TdxError::InvalidData(format!(
            "TDX {category:?} block source returned no records"
        )));
    }
    let mut identities = HashSet::with_capacity(records.len());
    let mut normalized = Vec::with_capacity(records.len());
    for record in records {
        if record.block_type != 2 {
            return Err(TdxError::InvalidData(format!(
                "TDX board {:?} has unsupported raw block type {}",
                record.blockname, record.block_type
            )));
        }
        let name = NonEmptyText::new(record.blockname)?.into_string();
        validate_code_shape(&record.code)?;
        if !identities.insert((name.clone(), record.code.clone())) {
            return Err(TdxError::InvalidData(format!(
                "duplicate TDX board/source pair {name:?}/{}",
                record.code
            )));
        }
        normalized.push((name, record.code));
    }
    Ok(normalized)
}

fn validate_code_shape(code: &str) -> Result<(), TdxError> {
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TdxError::InvalidData(format!(
            "TDX board source returned invalid security code {code:?}"
        )));
    }
    Ok(())
}

fn source_instrument(code: &str) -> Result<InstrumentId, TdxError> {
    validate_code_shape(code)?;
    let exchange = match code.as_bytes()[0] {
        b'6' => Exchange::Shanghai,
        b'0' | b'3' => Exchange::Shenzhen,
        b'4' | b'8' => Exchange::Beijing,
        b'9' if code.starts_with("920") => Exchange::Beijing,
        b'9' => {
            return Err(TdxError::Unsupported(format!(
                "TDX board code {code} uses an unverified 9-prefix exchange mapping"
            )))
        }
        prefix => {
            return Err(TdxError::Unsupported(format!(
                "TDX board code prefix {:?} has no verified exchange mapping",
                char::from(prefix)
            )))
        }
    };
    Ok(InstrumentId::new(exchange, code, AssetClass::Equity)?)
}

fn validate_requested_instrument(instrument: &InstrumentId) -> Result<(), TdxError> {
    if instrument.asset_class() != AssetClass::Equity {
        return Err(TdxError::Unsupported(
            "TDX normalized board reverse lookup accepts only equities".into(),
        ));
    }
    let source_identity = source_instrument(instrument.code())?;
    if source_identity.exchange() != instrument.exchange() {
        return Err(TdxError::InvalidData(format!(
            "TDX board code {} implies {:?}, not {:?}",
            instrument.code(),
            source_identity.exchange(),
            instrument.exchange()
        )));
    }
    Ok(())
}

struct OperationContext {
    observed_at: String,
    batch_id: String,
}

impl OperationContext {
    fn new(family: &str) -> Result<Self, TdxError> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| {
                TdxError::InvalidData(format!("system clock is before UNIX epoch: {error}"))
            })?
            .as_millis();
        let observed_at = format!("unix-ms:{millis}");
        Ok(Self {
            batch_id: format!("{SOURCE_NAME}:{family}:{observed_at}"),
            observed_at,
        })
    }

    fn evidence(&self) -> Result<SourceEvidence, TdxError> {
        Ok(SourceEvidence::new(
            ProviderId::Tdx,
            self.observed_at.clone(),
            self.batch_id.clone(),
        )?)
    }

    fn finish<T>(&self, records: Vec<T>) -> Result<DataBatch<T>, TdxError> {
        if records.is_empty() {
            return Err(TdxError::InvalidData(
                "TDX normalized board operation returned no records".into(),
            ));
        }
        let provenance = Provenance::new(SOURCE_NAME, self.observed_at.clone())?
            .with_batch_id(self.batch_id.clone())?;
        Ok(DataBatch::strict(records, provenance))
    }
}
