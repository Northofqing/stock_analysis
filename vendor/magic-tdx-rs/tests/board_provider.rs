use magic_market_core::{
    AssetClass, BoardCategory, BoardConstituentProvider, BoardConstituentRequest,
    BoardDirectoryProvider, BoardDirectoryRequest, BoardMembershipProvider, Exchange, InstrumentId,
    NonEmptyText, PositiveU32, ProviderId, SourcedRecord,
};
use magic_tdx_rs::block::BlockType;
use magic_tdx_rs::reader::block::BlockRecord;
use magic_tdx_rs::{TdxBoardProvider, TdxBoardSource, TdxError};

#[derive(Clone)]
struct FixtureSource {
    industry: Vec<BlockRecord>,
    concept: Vec<BlockRecord>,
}

impl FixtureSource {
    fn new() -> Self {
        Self {
            industry: vec![
                record("电力", "600000", 0),
                record("电力", "000001", 1),
                record("高股息", "600000", 0),
            ],
            concept: vec![
                record("人工智能", "002230", 0),
                record("人工智能", "300750", 1),
                record("人工智能", "920118", 2),
                record("华为概念", "002230", 0),
            ],
        }
    }
}

impl TdxBoardSource for FixtureSource {
    fn records(&self, block_type: BlockType) -> Result<Vec<BlockRecord>, TdxError> {
        match block_type {
            BlockType::Industry => Ok(self.industry.clone()),
            BlockType::Concept => Ok(self.concept.clone()),
            BlockType::Index => Err(TdxError::Unsupported("index blocks".into())),
        }
    }
}

fn record(board: &str, code: &str, code_index: u16) -> BlockRecord {
    BlockRecord {
        blockname: board.into(),
        block_type: 2,
        code_index,
        code: code.into(),
    }
}

fn limit(value: u32) -> PositiveU32 {
    PositiveU32::new(value).unwrap()
}

#[test]
fn directory_constituents_and_reverse_memberships_are_consistent() {
    let provider = TdxBoardProvider::with_source(FixtureSource::new());
    let boards = provider
        .boards(&BoardDirectoryRequest::new(BoardCategory::Concept, limit(10)).unwrap())
        .unwrap();
    assert_eq!(boards.records().len(), 2);
    assert_eq!(
        boards.records()[0].board_code().as_str(),
        "tdx:concept:人工智能"
    );
    assert_eq!(boards.records()[0].member_count().get(), 3);
    assert_eq!(boards.records()[0].provider_id(), ProviderId::Tdx);
    assert!(boards.provenance().source_at().is_none());

    let members = provider
        .board_constituents(
            &BoardConstituentRequest::new(
                NonEmptyText::new("tdx:concept:人工智能").unwrap(),
                limit(10),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(members.records().len(), 3);
    assert!(members
        .records()
        .iter()
        .all(|row| row.board_code.as_str() == "tdx:concept:人工智能"));

    let requested =
        vec![InstrumentId::new(Exchange::Shenzhen, "002230", AssetClass::Equity).unwrap()];
    let reverse = provider.board_memberships(&requested).unwrap();
    assert_eq!(reverse.records().len(), 2);
    assert!(reverse
        .records()
        .iter()
        .all(|record| record.instrument == requested[0]));
}

#[test]
fn rejects_duplicate_source_pairs_and_unverified_codes() {
    let mut duplicate = FixtureSource::new();
    duplicate.concept.push(record("人工智能", "002230", 2));
    let provider = TdxBoardProvider::with_source(duplicate);
    assert!(provider
        .boards(&BoardDirectoryRequest::new(BoardCategory::Concept, limit(10)).unwrap())
        .is_err());

    let mut invalid = FixtureSource::new();
    invalid.industry[0].code = "500001".into();
    let provider = TdxBoardProvider::with_source(invalid);
    assert!(provider
        .board_constituents(
            &BoardConstituentRequest::new(
                NonEmptyText::new("tdx:industry:电力").unwrap(),
                limit(10),
            )
            .unwrap(),
        )
        .is_err());
}

#[test]
fn accepts_verified_beijing_920_codes_and_rejects_other_nine_prefixes() {
    let verified = TdxBoardProvider::with_source(FixtureSource {
        industry: vec![record("北交所", "920118", 0)],
        concept: Vec::new(),
    });
    let members = verified
        .board_constituents(
            &BoardConstituentRequest::new(
                NonEmptyText::new("tdx:industry:北交所").unwrap(),
                limit(10),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        members.records()[0].instrument.exchange(),
        Exchange::Beijing
    );
    assert_eq!(members.records()[0].instrument.code(), "920118");

    let unverified = TdxBoardProvider::with_source(FixtureSource {
        industry: vec![record("B股", "900901", 0)],
        concept: Vec::new(),
    });
    assert!(matches!(
        unverified.board_constituents(
            &BoardConstituentRequest::new(
                NonEmptyText::new("tdx:industry:B股").unwrap(),
                limit(10),
            )
            .unwrap(),
        ),
        Err(TdxError::Unsupported(message)) if message.contains("unverified")
    ));
}

#[test]
fn rejects_duplicate_requests_unknown_boards_and_unsupported_categories() {
    let provider = TdxBoardProvider::with_source(FixtureSource::new());
    let instrument = InstrumentId::new(Exchange::Shenzhen, "002230", AssetClass::Equity).unwrap();
    assert!(provider
        .board_memberships(&[instrument.clone(), instrument])
        .is_err());

    assert!(provider
        .board_constituents(
            &BoardConstituentRequest::new(
                NonEmptyText::new("tdx:concept:不存在").unwrap(),
                limit(10),
            )
            .unwrap(),
        )
        .is_err());

    for category in [BoardCategory::Region, BoardCategory::Unknown] {
        assert!(provider
            .boards(&BoardDirectoryRequest::new(category, limit(10)).unwrap())
            .is_err());
    }
    assert!(provider
        .board_constituents(
            &BoardConstituentRequest::new(
                NonEmptyText::new("tdx:index:沪深300").unwrap(),
                limit(10),
            )
            .unwrap(),
        )
        .is_err());
}

#[test]
fn empty_source_and_unmatched_reverse_lookup_are_explicit_errors() {
    let empty = TdxBoardProvider::with_source(FixtureSource {
        industry: Vec::new(),
        concept: Vec::new(),
    });
    assert!(empty
        .boards(&BoardDirectoryRequest::new(BoardCategory::Industry, limit(10)).unwrap())
        .is_err());

    let provider = TdxBoardProvider::with_source(FixtureSource::new());
    let unknown = InstrumentId::new(Exchange::Shanghai, "600999", AssetClass::Equity).unwrap();
    assert!(provider.board_memberships(&[unknown]).is_err());
}
