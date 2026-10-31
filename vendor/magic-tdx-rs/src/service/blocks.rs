//! Stable facade for industry, concept, and regional block data.

use crate::block::{BlockFileSnapshot, TdxBlockClient};
use crate::error::TdxError;
use crate::protocol::types::{IndexBar, SecurityQuote};
use crate::reader::block::BlockRecord;
use magic_market_core::{
    AssetClass, BoardCategory, BoardMembership, BoardMembershipProvider, ConceptHit, ConceptHits,
    DataBatch, Exchange, InstrumentId, NonEmptyText, Provenance, ProviderId, SourceEvidence,
};
use std::collections::{HashMap, HashSet};

const BLOCK_MEMBERSHIP_SOURCE: &str = "tdx-block-files";
const BOARD_FILES: [(&str, BoardCategory); 3] = [
    (
        crate::protocol::constants::BLOCK_FG,
        BoardCategory::Industry,
    ),
    (crate::protocol::constants::BLOCK_GN, BoardCategory::Concept),
    (crate::protocol::constants::BLOCK_SZ, BoardCategory::Unknown),
];

trait BlockSnapshotSource {
    fn snapshot(&self, filename: &str) -> Result<BlockFileSnapshot, TdxError>;
}

impl BlockSnapshotSource for BlockService {
    fn snapshot(&self, filename: &str) -> Result<BlockFileSnapshot, TdxError> {
        self.client.get_block_snapshot(filename)
    }
}

fn observed_at() -> Result<String, TdxError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| format!("{}.{:09}", duration.as_secs(), duration.subsec_nanos()))
        .map_err(|error| {
            TdxError::InvalidData(format!("system clock is before UNIX epoch: {error}"))
        })
}

fn valid_six_digit_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit())
}

fn canonical_instruments(instruments: &[InstrumentId]) -> Result<Vec<InstrumentId>, TdxError> {
    if instruments.is_empty() {
        return Err(TdxError::InvalidData(
            "TDX block-data request must not be empty".into(),
        ));
    }
    let mut identities_by_code = HashMap::<String, InstrumentId>::new();
    let mut seen = HashSet::new();
    let mut ordered = Vec::with_capacity(instruments.len());
    for instrument in instruments {
        if instrument.asset_class() != AssetClass::Equity {
            return Err(TdxError::Unsupported(format!(
                "TDX block files do not prove non-equity board membership for {}",
                instrument.code()
            )));
        }
        if instrument.exchange() == Exchange::Beijing {
            return Err(TdxError::Unsupported(format!(
                "TDX block files do not prove Beijing board membership for {}",
                instrument.code()
            )));
        }
        if !valid_six_digit_code(instrument.code()) {
            return Err(TdxError::InvalidData(format!(
                "TDX block-data request has invalid A-share code {:?}",
                instrument.code()
            )));
        }
        if let Some(existing) = identities_by_code.get(instrument.code()) {
            if existing != instrument {
                return Err(TdxError::InvalidData(format!(
                    "TDX block-data request has conflicting identities for code {}",
                    instrument.code()
                )));
            }
        } else {
            identities_by_code.insert(instrument.code().to_owned(), instrument.clone());
        }
        if seen.insert(instrument.clone()) {
            ordered.push(instrument.clone());
        }
    }
    Ok(ordered)
}

fn category_order(category: BoardCategory) -> u8 {
    match category {
        BoardCategory::Industry => 0,
        BoardCategory::Concept => 1,
        BoardCategory::Unknown => 2,
        BoardCategory::Region => 3,
    }
}

fn validate_snapshot(
    expected_filename: &str,
    snapshot: &BlockFileSnapshot,
) -> Result<(), TdxError> {
    if snapshot.filename != expected_filename {
        return Err(TdxError::InvalidData(format!(
            "TDX block snapshot identity mismatch: expected {expected_filename}, received {}",
            snapshot.filename
        )));
    }
    if snapshot.hash.len() != 64 || !snapshot.hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(TdxError::InvalidData(format!(
            "TDX block snapshot {expected_filename} has invalid source hash"
        )));
    }
    if snapshot.records.is_empty() {
        return Err(TdxError::InvalidData(format!(
            "TDX block snapshot {expected_filename} contains no source records"
        )));
    }
    for record in &snapshot.records {
        if record.block_type != 2 {
            return Err(TdxError::InvalidData(format!(
                "TDX block snapshot {expected_filename} contains unsupported block type {}",
                record.block_type
            )));
        }
        if !valid_six_digit_code(&record.code) {
            return Err(TdxError::InvalidData(format!(
                "TDX block snapshot {expected_filename} contains invalid member code {:?}",
                record.code
            )));
        }
        let checked_name = NonEmptyText::new(record.blockname.clone())?;
        if checked_name.as_str() != record.blockname {
            return Err(TdxError::InvalidData(format!(
                "TDX block snapshot {expected_filename} contains non-canonical block name {:?}",
                record.blockname
            )));
        }
    }
    Ok(())
}

fn board_memberships_with(
    source: &impl BlockSnapshotSource,
    instruments: &[InstrumentId],
    observed_at: &str,
) -> Result<DataBatch<BoardMembership>, TdxError> {
    let instruments = canonical_instruments(instruments)?;
    let requested_codes = instruments
        .iter()
        .map(|instrument| instrument.code().to_owned())
        .collect::<HashSet<_>>();
    let mut snapshots = Vec::with_capacity(BOARD_FILES.len());
    for (filename, category) in BOARD_FILES {
        let snapshot = source.snapshot(filename)?;
        validate_snapshot(filename, &snapshot)?;
        snapshots.push((snapshot, category));
    }
    let batch_id = format!(
        "tdx-board-memberships:v1|{}",
        snapshots
            .iter()
            .map(|(snapshot, _)| format!("{}={}", snapshot.filename, snapshot.hash))
            .collect::<Vec<_>>()
            .join("|")
    );
    let provenance =
        Provenance::new(BLOCK_MEMBERSHIP_SOURCE, observed_at)?.with_batch_id(batch_id.clone())?;
    let evidence = SourceEvidence::new(ProviderId::Tdx, observed_at, batch_id)?;

    let mut by_code = HashMap::<String, Vec<(u8, String, String, BoardCategory)>>::new();
    let mut identities = HashMap::<(String, String), (String, BoardCategory)>::new();
    for (snapshot, category) in &snapshots {
        for record in &snapshot.records {
            if !requested_codes.contains(&record.code) {
                continue;
            }
            let board_code = format!("tdx:{}:{}", snapshot.filename, record.blockname);
            let key = (record.code.clone(), board_code.clone());
            match identities.get(&key) {
                Some((name, existing_category))
                    if name != &record.blockname || existing_category != category =>
                {
                    return Err(TdxError::InvalidData(format!(
                        "TDX board membership has conflicting source identity for {} and {board_code}",
                        record.code
                    )));
                }
                Some(_) => continue,
                None => {
                    identities.insert(key, (record.blockname.clone(), *category));
                }
            }
            by_code.entry(record.code.clone()).or_default().push((
                category_order(*category),
                board_code,
                record.blockname.clone(),
                *category,
            ));
        }
    }

    let mut records = Vec::new();
    for instrument in instruments {
        let Some(memberships) = by_code.get_mut(instrument.code()) else {
            continue;
        };
        memberships.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        for (_, board_code, board_name, category) in memberships.iter() {
            records.push(BoardMembership {
                instrument: instrument.clone(),
                board_code: NonEmptyText::new(board_code.clone())?,
                board_name: NonEmptyText::new(board_name.clone())?,
                category: *category,
                evidence: evidence.clone(),
            });
        }
    }
    Ok(DataBatch::strict(records, provenance))
}

fn concept_hits_with(
    source: &impl BlockSnapshotSource,
    instruments: &[InstrumentId],
    observed_at: &str,
) -> Result<DataBatch<ConceptHit>, TdxError> {
    let instruments = canonical_instruments(instruments)?;
    let requested_codes = instruments
        .iter()
        .map(|instrument| instrument.code().to_owned())
        .collect::<HashSet<_>>();
    let filename = crate::protocol::constants::BLOCK_GN;
    let snapshot = source.snapshot(filename)?;
    validate_snapshot(filename, &snapshot)?;
    let batch_id = format!(
        "tdx-concept-hits:v1|{}={}",
        snapshot.filename, snapshot.hash
    );
    let provenance =
        Provenance::new(BLOCK_MEMBERSHIP_SOURCE, observed_at)?.with_batch_id(batch_id.clone())?;
    let evidence = SourceEvidence::new(ProviderId::Tdx, observed_at, batch_id)?;
    let detail = NonEmptyText::new(format!(
        "source_file={};sha256={}",
        snapshot.filename, snapshot.hash
    ))?;

    let mut by_code = HashMap::<String, Vec<String>>::new();
    let mut identities = HashSet::new();
    for record in &snapshot.records {
        if !requested_codes.contains(&record.code) {
            continue;
        }
        if identities.insert((record.code.clone(), record.blockname.clone())) {
            by_code
                .entry(record.code.clone())
                .or_default()
                .push(record.blockname.clone());
        }
    }

    let mut records = Vec::new();
    for instrument in instruments {
        let Some(concepts) = by_code.get_mut(instrument.code()) else {
            continue;
        };
        concepts.sort();
        for concept in concepts.iter() {
            records.push(ConceptHit {
                instrument: instrument.clone(),
                concept: NonEmptyText::new(concept.clone())?,
                detail: Some(detail.clone()),
                evidence: evidence.clone(),
            });
        }
    }
    Ok(DataBatch::strict(records, provenance))
}

impl BoardMembershipProvider for BlockService {
    type Error = TdxError;

    fn board_memberships(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<BoardMembership>, Self::Error> {
        let observed_at = observed_at()?;
        board_memberships_with(self, instruments, &observed_at)
    }
}

impl ConceptHits for BlockService {
    type Error = TdxError;

    fn concept_hits(
        &self,
        instruments: &[InstrumentId],
    ) -> Result<DataBatch<ConceptHit>, Self::Error> {
        let observed_at = observed_at()?;
        concept_hits_with(self, instruments, &observed_at)
    }
}

/// Typed block-data service with the upstream client's safety limits.
pub struct BlockService {
    client: TdxBlockClient,
}

impl BlockService {
    /// Creates a block service for a TDX endpoint.
    pub fn new(ip: &str, port: u16, timeout: f64) -> Self {
        Self {
            client: TdxBlockClient::new(ip, port, timeout),
        }
    }
    /// Uses the default TDX port and timeout.
    pub fn with_default(ip: &str) -> Self {
        Self {
            client: TdxBlockClient::with_default(ip),
        }
    }
    /// Returns block K-lines with enforced category limits.
    pub fn bars(
        &self,
        category: u8,
        code: &str,
        start: u32,
        count: u16,
    ) -> Result<Vec<IndexBar>, TdxError> {
        self.client.get_block_bars(category, code, start, count)
    }
    /// Returns block quotes, preserving the requested code list.
    pub fn quotes(&self, codes: &[&str]) -> Result<Vec<SecurityQuote>, TdxError> {
        self.client.get_block_quotes(codes)
    }
    /// Loads industry block records.
    pub fn industry(&self) -> Result<Vec<BlockRecord>, TdxError> {
        self.client.get_industry_blocks()
    }
    /// Loads concept block records.
    pub fn concept(&self) -> Result<Vec<BlockRecord>, TdxError> {
        self.client.get_concept_blocks()
    }
    /// Loads index/region block records.
    pub fn index(&self) -> Result<Vec<BlockRecord>, TdxError> {
        self.client.get_index_blocks()
    }
    /// Updates the endpoint used by this service.
    pub fn set_server(&self, ip: &str, port: u16) {
        self.client.set_server(ip, port);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockFileSnapshot;
    use magic_market_core::{
        AssetClass, BoardCategory, BoardMembershipProvider, Exchange, InstrumentId, ProviderId,
    };

    struct FixtureSnapshots {
        snapshots: Vec<BlockFileSnapshot>,
    }

    impl BlockSnapshotSource for FixtureSnapshots {
        fn snapshot(&self, filename: &str) -> Result<BlockFileSnapshot, TdxError> {
            self.snapshots
                .iter()
                .find(|snapshot| snapshot.filename == filename)
                .cloned()
                .ok_or_else(|| TdxError::FileNotFound(filename.to_owned()))
        }
    }

    fn instrument(exchange: Exchange, code: &str) -> InstrumentId {
        InstrumentId::new(exchange, code, AssetClass::Equity).unwrap()
    }

    fn snapshot(filename: &str, hash_byte: char, rows: &[(&str, &str)]) -> BlockFileSnapshot {
        BlockFileSnapshot {
            filename: filename.to_owned(),
            hash: std::iter::repeat_n(hash_byte, 64).collect(),
            records: rows
                .iter()
                .enumerate()
                .map(|(index, (name, code))| BlockRecord {
                    blockname: (*name).to_owned(),
                    block_type: 2,
                    code_index: u16::try_from(index).unwrap(),
                    code: (*code).to_owned(),
                })
                .collect(),
        }
    }

    #[test]
    fn board_memberships_are_request_bound_typed_and_share_atomic_evidence() {
        let source = FixtureSnapshots {
            snapshots: vec![
                snapshot(
                    "block_fg.dat",
                    'a',
                    &[("电力", "600396"), ("银行", "000001")],
                ),
                snapshot("block_gn.dat", 'b', &[("绿色电力", "600396")]),
                snapshot(
                    "block_zs.dat",
                    'c',
                    &[("沪深300", "600396"), ("沪深300", "000001")],
                ),
            ],
        };
        let ping_an = instrument(Exchange::Shenzhen, "000001");
        let huadian = instrument(Exchange::Shanghai, "600396");

        let batch = board_memberships_with(
            &source,
            &[ping_an.clone(), huadian.clone(), ping_an.clone()],
            "observed-1",
        )
        .unwrap();

        let actual = batch
            .records()
            .iter()
            .map(|row| {
                (
                    row.instrument.clone(),
                    row.board_code.as_str().to_owned(),
                    row.board_name.as_str().to_owned(),
                    row.category,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                (
                    ping_an,
                    "tdx:block_fg.dat:银行".to_owned(),
                    "银行".to_owned(),
                    BoardCategory::Industry,
                ),
                (
                    instrument(Exchange::Shenzhen, "000001"),
                    "tdx:block_zs.dat:沪深300".to_owned(),
                    "沪深300".to_owned(),
                    BoardCategory::Unknown,
                ),
                (
                    huadian,
                    "tdx:block_fg.dat:电力".to_owned(),
                    "电力".to_owned(),
                    BoardCategory::Industry,
                ),
                (
                    instrument(Exchange::Shanghai, "600396"),
                    "tdx:block_gn.dat:绿色电力".to_owned(),
                    "绿色电力".to_owned(),
                    BoardCategory::Concept,
                ),
                (
                    instrument(Exchange::Shanghai, "600396"),
                    "tdx:block_zs.dat:沪深300".to_owned(),
                    "沪深300".to_owned(),
                    BoardCategory::Unknown,
                ),
            ]
        );
        assert_eq!(batch.provenance().source(), "tdx-block-files");
        assert_eq!(batch.provenance().fetched_at(), "observed-1");
        assert!(batch.provenance().source_at().is_none());
        let batch_id = batch.provenance().batch_id().unwrap();
        for row in batch.records() {
            assert_eq!(row.evidence.provider(), ProviderId::Tdx);
            assert_eq!(row.evidence.observed_at(), "observed-1");
            assert_eq!(row.evidence.batch_id(), batch_id);
            assert!(row.evidence.source_at().is_none());
        }
    }

    #[test]
    fn block_service_satisfies_the_existing_provider_contract() {
        fn assert_provider<T: BoardMembershipProvider<Error = TdxError>>() {}
        assert_provider::<BlockService>();
    }

    #[test]
    fn complete_three_file_no_match_is_an_evidenced_empty_batch() {
        let source = FixtureSnapshots {
            snapshots: vec![
                snapshot("block_fg.dat", 'a', &[("银行", "000001")]),
                snapshot("block_gn.dat", 'b', &[("金融科技", "000001")]),
                snapshot("block_zs.dat", 'c', &[("沪深300", "000001")]),
            ],
        };

        let batch = board_memberships_with(
            &source,
            &[instrument(Exchange::Shanghai, "600396")],
            "observed-empty",
        )
        .unwrap();

        assert!(batch.records().is_empty());
        assert_eq!(batch.provenance().source(), "tdx-block-files");
        assert_eq!(batch.provenance().fetched_at(), "observed-empty");
        assert!(batch.provenance().source_at().is_none());
        assert!(batch.provenance().batch_id().is_some());
    }

    #[test]
    fn concept_projection_is_concept_only_unique_and_file_hash_versioned() {
        let source = FixtureSnapshots {
            snapshots: vec![snapshot(
                "block_gn.dat",
                'b',
                &[
                    ("绿色电力", "600396"),
                    ("绿色电力", "600396"),
                    ("国企改革", "600396"),
                    ("金融科技", "000001"),
                ],
            )],
        };
        let request = [
            instrument(Exchange::Shenzhen, "000001"),
            instrument(Exchange::Shanghai, "600396"),
            instrument(Exchange::Shanghai, "600396"),
        ];

        let batch = concept_hits_with(&source, &request, "observed-concepts").unwrap();
        let actual = batch
            .records()
            .iter()
            .map(|record| {
                (
                    record.instrument.code().to_owned(),
                    record.concept.as_str().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            vec![
                ("000001".into(), "金融科技".into()),
                ("600396".into(), "国企改革".into()),
                ("600396".into(), "绿色电力".into()),
            ]
        );
        assert_eq!(batch.provenance().source(), "tdx-block-files");
        let batch_id = batch.provenance().batch_id().unwrap();
        assert!(batch_id.contains("block_gn.dat="));
        assert!(batch_id.contains(&"b".repeat(64)));
        for record in batch.records() {
            assert_eq!(record.evidence.provider(), ProviderId::Tdx);
            assert_eq!(record.evidence.batch_id(), batch_id);
            let detail = record.detail.as_ref().unwrap().as_str();
            assert!(detail.contains("source_file=block_gn.dat"));
            assert!(detail.contains("sha256="));
        }
    }

    #[test]
    fn production_observation_timestamp_is_an_unambiguous_instant_for_both_projections() {
        let source = FixtureSnapshots {
            snapshots: vec![
                snapshot("block_fg.dat", 'a', &[("电力", "600396")]),
                snapshot("block_gn.dat", 'b', &[("绿色电力", "600396")]),
                snapshot("block_zs.dat", 'c', &[("沪深300", "600396")]),
            ],
        };
        let observed = observed_at().unwrap();
        magic_market_core::EvidenceTimestamp::parse_instant(&observed).unwrap();
        let request = [instrument(Exchange::Shanghai, "600396")];
        let board = board_memberships_with(&source, &request, &observed).unwrap();
        let concepts = concept_hits_with(&source, &request, &observed).unwrap();
        magic_market_core::EvidenceTimestamp::parse_instant(board.provenance().fetched_at())
            .unwrap();
        magic_market_core::EvidenceTimestamp::parse_instant(concepts.provenance().fetched_at())
            .unwrap();
    }

    #[test]
    fn concept_projection_proves_empty_and_rejects_beijing_before_io() {
        let source = FixtureSnapshots {
            snapshots: vec![snapshot("block_gn.dat", 'b', &[("金融科技", "000001")])],
        };
        let empty = concept_hits_with(
            &source,
            &[instrument(Exchange::Shanghai, "600396")],
            "observed-empty",
        )
        .unwrap();
        assert!(empty.records().is_empty());
        assert!(empty.quality().is_complete());
        assert!(empty.provenance().batch_id().is_some());

        let beijing = instrument(Exchange::Beijing, "920118");
        assert!(matches!(
            concept_hits_with(&NoIoExpected, &[beijing], "observed"),
            Err(TdxError::Unsupported(_))
        ));
    }

    #[test]
    fn concept_projection_rejects_wrong_snapshot_identity_or_hash() {
        let request = [instrument(Exchange::Shanghai, "600396")];
        let wrong_file = FixtureSnapshots {
            snapshots: vec![snapshot("block_gn.dat", 'b', &[("绿色电力", "600396")])],
        };
        let mut wrong_identity = wrong_file.snapshots[0].clone();
        wrong_identity.filename = "block_fg.dat".into();
        let source = FixtureSnapshots {
            snapshots: vec![wrong_identity],
        };
        assert!(concept_hits_with(&source, &request, "observed").is_err());

        let mut bad_hash = snapshot("block_gn.dat", 'b', &[("绿色电力", "600396")]);
        bad_hash.hash = "short".into();
        let source = FixtureSnapshots {
            snapshots: vec![bad_hash],
        };
        assert!(concept_hits_with(&source, &request, "observed").is_err());
    }

    struct NoIoExpected;

    impl BlockSnapshotSource for NoIoExpected {
        fn snapshot(&self, filename: &str) -> Result<BlockFileSnapshot, TdxError> {
            panic!("snapshot I/O must not run for unsupported request: {filename}")
        }
    }

    #[test]
    fn unsupported_or_conflicting_requests_fail_before_snapshot_io() {
        let beijing = instrument(Exchange::Beijing, "920118");
        assert!(matches!(
            board_memberships_with(&NoIoExpected, &[beijing], "observed"),
            Err(TdxError::Unsupported(_))
        ));

        let fund = InstrumentId::new(Exchange::Shanghai, "510050", AssetClass::Fund).unwrap();
        assert!(matches!(
            board_memberships_with(&NoIoExpected, &[fund], "observed"),
            Err(TdxError::Unsupported(_))
        ));

        let conflicting = [
            instrument(Exchange::Shanghai, "600396"),
            instrument(Exchange::Shenzhen, "600396"),
        ];
        let error = board_memberships_with(&NoIoExpected, &conflicting, "observed").unwrap_err();
        assert!(error.to_string().contains("conflicting identities"));
    }

    #[test]
    fn empty_source_family_fails_while_equivalent_rows_collapse() {
        let mut source = FixtureSnapshots {
            snapshots: vec![
                snapshot(
                    "block_fg.dat",
                    'a',
                    &[("电力", "600396"), ("电力", "600396")],
                ),
                snapshot("block_gn.dat", 'b', &[("绿色电力", "600396")]),
                snapshot("block_zs.dat", 'c', &[("沪深300", "600396")]),
            ],
        };
        let request = [instrument(Exchange::Shanghai, "600396")];
        let batch = board_memberships_with(&source, &request, "observed-dedup").unwrap();
        assert_eq!(
            batch
                .records()
                .iter()
                .filter(|row| row.board_name.as_str() == "电力")
                .count(),
            1
        );

        source.snapshots[1].records.clear();
        let error = board_memberships_with(&source, &request, "observed-error").unwrap_err();
        assert!(error.to_string().contains("contains no source records"));
    }
}
