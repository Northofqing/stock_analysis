//! Read-only funding proposals bound to an actual retained genesis snapshot.
//! Consistency is historical arithmetic, never funds or execution approval.
#[path = "paper_funding_review_codec_v1.rs"]
mod codec;
use super::paper_book_v2_budget_v1::{BudgetRecord, CashPartitions};
use super::paper_ledger::{LedgerError, Projection};
use crate::database::global_schema_v1::investment_v8::{
    investment_catalog8_session, VerifiedCatalog8,
};
use crate::database::{DatabaseConnectionAuthority, DatabaseManager};
use codec::Work;
use diesel::sql_types::{BigInt, Binary, Text};
use diesel::{QueryableByName, RunQueryDsl, SqliteConnection};
use serde::Serialize;
use std::cell::RefCell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("funding review refused: {self:?}")]
pub(crate) enum FundingReviewErrorV1 {
    InputTooLarge,
    Schema,
    NonCanonical,
    Depth,
    Nodes,
    OwnedBudget,
    WorkBudget,
    ArithmeticOverflow,
    SqlShape,
    MissingGenesis,
    UnavailableGenesis,
    ChangedObservation,
    CatalogUnavailable,
    SqlRead,
    InternalEncoding,
}
use FundingReviewErrorV1 as Error;
// One bounded returned legacy error String, including initial_cash's nested validation.
const LEGACY_ERROR_RESERVATION: usize = 256;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum FundingMismatchV1 {
    Account,
    Epoch,
    Cutover,
    GenesisManifest,
    GenesisEvent,
    GenesisProjection,
    FeePolicy,
    SeedReference,
    AllocationAgainstGenesis,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum FundingReviewOutcomeV1 {
    ConsistentProposal,
    InconsistentProposal(FundingMismatchV1),
}
use FundingReviewOutcomeV1 as Outcome;
#[derive(Debug, Serialize)]
struct Proposal {
    schema: String,
    version: u32,
    account_id: String,
    epoch_id: String,
    cutover_id: String,
    genesis_manifest_hash: String,
    genesis_event_hash: String,
    genesis_projection_hash: String,
    fee_policy_instance_id: String,
    budget: BudgetRecord,
}
#[derive(Debug, PartialEq, Eq, Serialize, QueryableByName)]
struct Binding {
    #[diesel(sql_type=Text)]
    account_id: String,
    #[diesel(sql_type=Text)]
    epoch_id: String,
    #[diesel(sql_type=Text)]
    cutover_id: String,
    #[diesel(sql_type=Text)]
    genesis_manifest_hash: String,
    #[diesel(sql_type=Text)]
    v1_epoch_id: String,
    #[diesel(sql_type=Text)]
    v1_manifest_hash: String,
    #[diesel(sql_type=BigInt)]
    v1_head_version: i64,
    #[diesel(sql_type=Text)]
    v1_head_hash: String,
    #[diesel(sql_type=Text)]
    v1_projection_hash: String,
    #[diesel(sql_type=BigInt)]
    genesis_version: i64,
    #[diesel(sql_type=Text)]
    genesis_event_hash: String,
    #[diesel(sql_type=Text)]
    genesis_projection_hash: String,
    #[diesel(sql_type=Text)]
    fee_policy_instance_id: String,
}
#[derive(Debug, Serialize)]
struct Record {
    schema: String,
    version: u32,
    proposal_id: String,
    proposal: Proposal,
    actual_binding: Binding,
    outcome: Outcome,
    initial_cash: Option<CashPartitions>,
    authority_state: String,
    approval_state: String,
}
#[derive(Debug)]
struct ProposalId(String);
#[derive(Debug)]
struct ReviewId(String);
#[derive(Debug)]
pub(crate) struct StoredFundingReviewV1 {
    canonical: Vec<u8>,
    id: ReviewId,
    proposal_id: ProposalId,
    outcome: Outcome,
    cash: Option<CashPartitions>,
}
#[derive(Debug)]
pub(crate) struct ObservedFundingReviewV1 {
    authority: DatabaseConnectionAuthority,
    value: StoredFundingReviewV1,
}
impl StoredFundingReviewV1 {
    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn review_id(&self) -> &str {
        &self.id.0
    }
    pub(crate) fn proposal_id(&self) -> &str {
        &self.proposal_id.0
    }
    pub(crate) fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub(crate) fn initial_cash(&self) -> Option<&CashPartitions> {
        self.cash.as_ref()
    }
}
impl ObservedFundingReviewV1 {
    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.value.canonical
    }
    pub(crate) fn review_id(&self) -> &str {
        &self.value.id.0
    }
    pub(crate) fn proposal_id(&self) -> &str {
        &self.value.proposal_id.0
    }
    pub(crate) fn outcome(&self) -> Outcome {
        self.value.outcome
    }
    pub(crate) fn initial_cash(&self) -> Option<&CashPartitions> {
        self.value.cash.as_ref()
    }
}
fn token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.trim() == s && !s.chars().any(char::is_control)
}
fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn validate_proposal_inner(p: &Proposal, work: &mut Work) -> Result<(), Error> {
    work.scan(
        8192usize
            .checked_add(
                4096usize
                    .checked_mul(p.budget.initial_lots.len())
                    .ok_or(Error::WorkBudget)?,
            )
            .ok_or(Error::WorkBudget)?,
    )?;
    if p.schema != "paper-funding-proposal-v1"
        || p.version != 1
        || [
            &p.account_id,
            &p.epoch_id,
            &p.cutover_id,
            &p.fee_policy_instance_id,
            &p.budget.family_id,
            &p.budget.original_seed_reference,
            &p.budget.review_reference,
        ]
        .iter()
        .any(|s| !token(s))
        || [
            &p.genesis_manifest_hash,
            &p.genesis_event_hash,
            &p.genesis_projection_hash,
        ]
        .iter()
        .any(|s| !hash(s))
        || p.budget
            .initial_lots
            .iter()
            .any(|l| !token(&l.lot_id) || l.chain_id.as_deref().is_some_and(|s| !token(s)))
    {
        return Err(Error::Schema);
    }
    work.own(LEGACY_ERROR_RESERVATION)?;
    #[cfg(test)]
    probe(Probe::LegacyShape);
    p.budget.validate_shape().map_err(|_| Error::Schema)
}
fn validate_binding_inner(b: &Binding, work: &mut Work) -> Result<(), Error> {
    work.scan(8192)?;
    if [
        &b.account_id,
        &b.epoch_id,
        &b.cutover_id,
        &b.v1_epoch_id,
        &b.fee_policy_instance_id,
    ]
    .iter()
    .any(|s| !token(s))
        || [
            &b.genesis_manifest_hash,
            &b.v1_manifest_hash,
            &b.v1_head_hash,
            &b.v1_projection_hash,
            &b.genesis_event_hash,
            &b.genesis_projection_hash,
        ]
        .iter()
        .any(|s| !hash(s))
        || b.v1_head_version <= 0
        || b.genesis_version != 1
    {
        return Err(Error::SqlShape);
    }
    Ok(())
}
fn mismatch_inner(
    p: &Proposal,
    b: &Binding,
    work: &mut Work,
) -> Result<Option<FundingMismatchV1>, Error> {
    use FundingMismatchV1::*;
    work.scan(8192)?;
    for (left, right, reason) in [
        (&p.account_id, &b.account_id, Account),
        (&p.epoch_id, &b.epoch_id, Epoch),
        (&p.cutover_id, &b.cutover_id, Cutover),
        (
            &p.genesis_manifest_hash,
            &b.genesis_manifest_hash,
            GenesisManifest,
        ),
        (&p.genesis_event_hash, &b.genesis_event_hash, GenesisEvent),
        (
            &p.genesis_projection_hash,
            &b.genesis_projection_hash,
            GenesisProjection,
        ),
        (
            &p.fee_policy_instance_id,
            &b.fee_policy_instance_id,
            FeePolicy,
        ),
    ] {
        if left != right {
            return Ok(Some(reason));
        }
    }
    let prefix = "paper-v1-manifest-sha256:";
    if p.budget.original_seed_reference.len() != prefix.len() + 64
        || !p.budget.original_seed_reference.starts_with(prefix)
        || &p.budget.original_seed_reference[prefix.len()..] != b.v1_manifest_hash
    {
        return Ok(Some(SeedReference));
    }
    Ok(None)
}
fn allocation_inner(
    p: &Proposal,
    genesis: &Projection,
    work: &mut Work,
) -> Result<(Outcome, Option<CashPartitions>), Error> {
    let n = genesis.lots.len();
    let m = genesis.marks.len();
    let sq = |x: usize| {
        x.checked_add(1)
            .and_then(|v| v.checked_mul(v))
            .and_then(|v| v.checked_mul(512))
            .ok_or(Error::WorkBudget)
    };
    let scan = 8192usize
        .checked_add(
            4096usize
                .checked_mul(n.checked_add(m).ok_or(Error::WorkBudget)?)
                .ok_or(Error::WorkBudget)?,
        )
        .and_then(|v| v.checked_add(sq(n).ok()?))
        .and_then(|v| v.checked_add(sq(m).ok()?))
        .ok_or(Error::WorkBudget)?;
    work.scan(scan)?;
    work.own(n.checked_mul(512).ok_or(Error::OwnedBudget)?)?;
    work.own(LEGACY_ERROR_RESERVATION)?;
    #[cfg(test)]
    probe(Probe::LegacyInitialCash);
    match p.budget.initial_cash(genesis) {
        Ok(cash) => Ok((Outcome::ConsistentProposal, Some(cash))),
        Err(LedgerError::InvalidInput(_)) => Ok((
            Outcome::InconsistentProposal(FundingMismatchV1::AllocationAgainstGenesis),
            None,
        )),
        Err(LedgerError::Overflow) => Err(Error::ArithmeticOverflow),
        Err(_) => Err(Error::UnavailableGenesis),
    }
}
#[derive(QueryableByName)]
struct Meta {
    #[diesel(sql_type=BigInt)]
    owners: i64,
    #[diesel(sql_type=BigInt)]
    accounts: i64,
    #[diesel(sql_type=BigInt)]
    heads: i64,
    #[diesel(sql_type=BigInt)]
    invalid: i64,
    #[diesel(sql_type=BigInt)]
    bytes: i64,
}
#[derive(QueryableByName)]
struct Blob {
    #[diesel(sql_type=Binary)]
    value: Vec<u8>,
}
fn checked_meta(m: &Meta) -> Result<usize, Error> {
    if m.owners == 0 || m.accounts == 0 || m.heads == 0 {
        return Err(Error::MissingGenesis);
    }
    if m.owners != 1
        || m.accounts != 1
        || m.heads != 1
        || m.invalid != 0
        || m.bytes <= 0
        || m.bytes > codec::GENESIS_LIMIT as i64
    {
        return Err(Error::SqlShape);
    }
    usize::try_from(m.bytes).map_err(|_| Error::SqlShape)
}
// SQL is fixed; scalar bounds are checked before any consumer text/blob load.
const META_SQL:&str = "WITH wanted(id) AS (SELECT ?) SELECT (SELECT COUNT(*) FROM main.paper_book_owner_v2 WHERE account_id=(SELECT id FROM wanted)) AS owners,(SELECT COUNT(*) FROM main.paper_book_v2_account WHERE account_id=(SELECT id FROM wanted)) AS accounts,(SELECT COUNT(*) FROM main.paper_book_v2_head WHERE account_id=(SELECT id FROM wanted)) AS heads,COALESCE((SELECT CASE WHEN typeof(o.account_id)='text' AND length(CAST(o.account_id AS BLOB)) BETWEEN 1 AND 256 AND typeof(o.active_epoch_id)='text' AND length(CAST(o.active_epoch_id AS BLOB)) BETWEEN 1 AND 256 AND typeof(o.active_manifest_hash)='text' AND length(CAST(o.active_manifest_hash AS BLOB)) BETWEEN 1 AND 256 AND typeof(o.cutover_id)='text' AND length(CAST(o.cutover_id AS BLOB)) BETWEEN 1 AND 256 AND typeof(o.active_generation)='integer' AND o.active_generation=2 AND typeof(o.owner_revision)='integer' AND o.owner_revision=2 THEN 0 ELSE 1 END FROM main.paper_book_owner_v2 o WHERE o.account_id=(SELECT id FROM wanted)),1)+COALESCE((SELECT CASE WHEN typeof(a.account_id)='text' AND length(CAST(a.account_id AS BLOB)) BETWEEN 1 AND 256 AND typeof(a.epoch_id)='text' AND length(CAST(a.epoch_id AS BLOB)) BETWEEN 1 AND 256 AND typeof(a.cutover_id)='text' AND length(CAST(a.cutover_id AS BLOB)) BETWEEN 1 AND 256 AND typeof(a.manifest_hash)='text' AND length(CAST(a.manifest_hash AS BLOB)) BETWEEN 1 AND 256 AND typeof(a.v1_epoch_id)='text' AND length(CAST(a.v1_epoch_id AS BLOB)) BETWEEN 1 AND 256 AND typeof(a.v1_manifest_hash)='text' AND length(CAST(a.v1_manifest_hash AS BLOB)) BETWEEN 1 AND 256 AND typeof(a.v1_head_hash)='text' AND length(CAST(a.v1_head_hash AS BLOB)) BETWEEN 1 AND 256 AND typeof(a.v1_projection_hash)='text' AND length(CAST(a.v1_projection_hash AS BLOB)) BETWEEN 1 AND 256 AND typeof(a.fee_policy_instance_id)='text' AND length(CAST(a.fee_policy_instance_id AS BLOB)) BETWEEN 1 AND 256 AND typeof(a.v1_head_version)='integer' AND a.v1_head_version>0 THEN 0 ELSE 1 END FROM main.paper_book_v2_account a WHERE a.account_id=(SELECT id FROM wanted)),1)+COALESCE((SELECT CASE WHEN typeof(h.account_id)='text' AND length(CAST(h.account_id AS BLOB)) BETWEEN 1 AND 256 AND typeof(h.event_hash)='text' AND length(CAST(h.event_hash AS BLOB)) BETWEEN 1 AND 256 AND typeof(h.projection_hash)='text' AND length(CAST(h.projection_hash AS BLOB)) BETWEEN 1 AND 256 AND typeof(h.version)='integer' AND h.version=1 AND typeof(h.projection_bytes)='blob' THEN 0 ELSE 1 END FROM main.paper_book_v2_head h WHERE h.account_id=(SELECT id FROM wanted)),1) AS invalid,COALESCE((SELECT length(projection_bytes) FROM main.paper_book_v2_head WHERE account_id=(SELECT id FROM wanted)),0) AS bytes";
const BINDING_SQL:&str = "SELECT a.account_id,a.epoch_id,a.cutover_id,a.manifest_hash AS genesis_manifest_hash,a.v1_epoch_id,a.v1_manifest_hash,a.v1_head_version,a.v1_head_hash,a.v1_projection_hash,h.version AS genesis_version,h.event_hash AS genesis_event_hash,h.projection_hash AS genesis_projection_hash,a.fee_policy_instance_id FROM main.paper_book_v2_account a JOIN main.paper_book_v2_head h ON h.account_id=a.account_id WHERE a.account_id=? LIMIT 2";
const PAYLOAD_SQL: &str =
    "SELECT projection_bytes AS value FROM main.paper_book_v2_head WHERE account_id=? LIMIT 2";
#[derive(Clone, Copy)]
enum FixedQuery {
    Metadata,
    Binding,
    Payload,
}
impl FixedQuery {
    fn sql(self) -> &'static str {
        match self {
            Self::Metadata => META_SQL,
            Self::Binding => BINDING_SQL,
            Self::Payload => PAYLOAD_SQL,
        }
    }
    fn reservation(self) -> Result<usize, Error> {
        self.sql()
            .len()
            .checked_add(std::mem::size_of::<diesel::query_builder::SqlQuery>())
            .and_then(|n| n.checked_add(std::mem::size_of::<&str>()))
            .ok_or(Error::OwnedBudget)
    }
}
// Application-owned SQL text and finite wrapper reservation, not a driver/RSS bound.
fn fixed_query(
    kind: FixedQuery,
    work: &mut Work,
) -> Result<diesel::query_builder::SqlQuery, Error> {
    let result = (|| {
        work.own(kind.reservation()?)?;
        work.scan(kind.sql().len())?;
        #[cfg(test)]
        probe(match kind {
            FixedQuery::Metadata => Probe::MetadataQuery,
            FixedQuery::Binding => Probe::BindingQuery,
            FixedQuery::Payload => Probe::PayloadQuery,
        });
        Ok(diesel::sql_query(kind.sql()))
    })();
    work.finish(result)
}
fn extract_inner(
    conn: &mut SqliteConnection,
    proof: &VerifiedCatalog8<'_>,
    account: &str,
    work: &mut Work,
) -> Result<(Binding, Vec<u8>), Error> {
    work.scan(0)?; // A failed local budget may not perform another SQL read.
    proof
        .require_decision_instance(conn)
        .map_err(|_| Error::CatalogUnavailable)?;
    let m = fixed_query(FixedQuery::Metadata, work)?
        .bind::<Text, _>(account)
        .get_result::<Meta>(conn)
        .map_err(|_| Error::SqlRead)?;
    let n = checked_meta(&m)?;
    work.own(4096 + std::mem::size_of::<Binding>())?;
    work.own(n)?;
    work.scan(4096 + n)?;
    #[cfg(test)]
    probe(Probe::Payload);
    let b = fixed_query(FixedQuery::Binding, work)?
        .bind::<Text, _>(account)
        .get_result::<Binding>(conn)
        .map_err(|_| Error::SqlRead)?;
    let raw = fixed_query(FixedQuery::Payload, work)?
        .bind::<Text, _>(account)
        .get_result::<Blob>(conn)
        .map_err(|_| Error::SqlRead)?
        .value;
    if raw.len() != n {
        return Err(Error::ChangedObservation);
    }
    validate_binding(&b, work)?;
    Ok((b, raw))
}
struct Pending {
    authority: DatabaseConnectionAuthority,
    binding: Binding,
    raw: Vec<u8>,
    proposal: Proposal,
    proposal_id: String,
    outcome: Outcome,
    cash: Option<CashPartitions>,
}
pub(crate) fn review_funding_proposal(
    db: &DatabaseManager,
    account_id: &str,
    canonical_proposal: &[u8],
) -> Result<ObservedFundingReviewV1, Error> {
    let work = RefCell::new(Work::new());
    if account_id.len() > 256 {
        return Err(Error::InputTooLarge);
    }
    work.borrow_mut().scan(account_id.len())?;
    if !token(account_id) {
        return Err(Error::Schema);
    }
    let proposal = codec::proposal(canonical_proposal, &mut work.borrow_mut())?;
    validate_proposal(&proposal, &mut work.borrow_mut())?;
    codec::canonical(
        &proposal,
        canonical_proposal,
        codec::PROPOSAL_LIMIT,
        &mut work.borrow_mut(),
    )?;
    let proposal_id = codec::id(
        "paper-funding-proposal-v1:",
        b"stock_analysis.paper_funding.proposal.v1\0",
        canonical_proposal,
        &mut work.borrow_mut(),
    )?;
    let mut session = investment_catalog8_session(db).map_err(|_| Error::CatalogUnavailable)?;
    let pending = session.with_readonly_catalog8(
        |conn, proof| {
            let w = &mut *work.borrow_mut();
            let (binding, raw) = extract(conn, proof, account_id, w)?;
            let genesis = codec::projection(&raw, w)?;
            let (outcome, cash) = match mismatch(&proposal, &binding, w)? {
                Some(reason) => (Outcome::InconsistentProposal(reason), None),
                None => allocation(&proposal, &genesis, w)?,
            };
            #[cfg(test)]
            { hook(conn)?; }
            Ok::<_, Error>(Pending {
                authority: proof.connection_authority().clone(), binding, raw,
                proposal, proposal_id, outcome, cash,
            })
        },
        |conn, proof, value| {
            if proof.connection_authority() != &value.authority { return Err(Error::ChangedObservation); }
            let w = &mut *work.borrow_mut();
            let (binding, raw) = extract(conn, proof, account_id, w)?;
            let compare = raw.len().checked_add(value.raw.len()).and_then(|n| n.checked_add(8192)).ok_or(Error::WorkBudget)?;
            w.scan(compare)?;
            if binding != value.binding || raw != value.raw { return Err(Error::ChangedObservation); }
            #[cfg(test)]
            {
                if TAIL_FAIL.with(|f| f.get()) { return Err(Error::ChangedObservation); }
                probe(Probe::Tail);
            }
            Ok(())
        },
    ).map_err(|error| match error {
        crate::database::global_schema_v1::investment_v8::InvestmentCatalog8ReadbackError::Consumer(error) => error,
        _ => Error::CatalogUnavailable,
    })?;
    let w = &mut *work.borrow_mut();
    w.own(std::mem::size_of::<Record>() + 128)?;
    let record = Record {
        schema: "paper-funding-review-v1".into(),
        version: 1,
        proposal_id: pending.proposal_id,
        proposal: pending.proposal,
        actual_binding: pending.binding,
        outcome: pending.outcome,
        initial_cash: pending.cash,
        authority_state: "HistoricalObservationOnly".into(),
        approval_state: "NotIssued".into(),
    };
    let canonical = codec::encode(&record, codec::REVIEW_LIMIT, w)?;
    let id = codec::id(
        "paper-funding-review-v1:",
        b"stock_analysis.paper_funding.review.v1\0",
        &canonical,
        w,
    )?;
    #[cfg(test)]
    probe(Probe::Observed);
    Ok(ObservedFundingReviewV1 {
        authority: pending.authority,
        value: StoredFundingReviewV1 {
            canonical,
            id: ReviewId(id),
            proposal_id: ProposalId(record.proposal_id),
            outcome: record.outcome,
            cash: record.initial_cash,
        },
    })
}
pub(crate) fn read_stored_funding_review(canonical: &[u8]) -> Result<StoredFundingReviewV1, Error> {
    let w = &mut Work::new();
    let r = codec::record(canonical, w)?;
    validate_proposal(&r.proposal, w)?;
    validate_binding(&r.actual_binding, w)?;
    if r.schema != "paper-funding-review-v1"
        || r.version != 1
        || r.authority_state != "HistoricalObservationOnly"
        || r.approval_state != "NotIssued"
    {
        return Err(Error::Schema);
    }
    let bytes = codec::encode(&r.proposal, codec::PROPOSAL_LIMIT, w)?;
    let pid = codec::id(
        "paper-funding-proposal-v1:",
        b"stock_analysis.paper_funding.proposal.v1\0",
        &bytes,
        w,
    )?;
    w.scan(8192)?;
    if pid != r.proposal_id {
        return Err(Error::Schema);
    }
    let expected = mismatch(&r.proposal, &r.actual_binding, w)?;
    match (r.outcome, expected, &r.initial_cash) {
        (Outcome::ConsistentProposal, None, Some(c)) => {
            validate_cash_partitions(c, w)?;
            if c.strategy_cash != r.proposal.budget.initial_strategy_cash_micro_cny
                || c.strategy_cash > r.proposal.budget.authorized_budget_micro_cny
            {
                return Err(Error::Schema);
            }
        }
        (
            Outcome::InconsistentProposal(FundingMismatchV1::AllocationAgainstGenesis),
            None,
            None,
        ) => {}
        (Outcome::InconsistentProposal(a), Some(b), None) if a == b => {}
        _ => return Err(Error::Schema),
    }
    codec::canonical(&r, canonical, codec::REVIEW_LIMIT, w)?;
    let id = codec::id(
        "paper-funding-review-v1:",
        b"stock_analysis.paper_funding.review.v1\0",
        canonical,
        w,
    )?;
    w.own(canonical.len())?;
    w.scan(canonical.len())?;
    Ok(StoredFundingReviewV1 {
        canonical: canonical.to_vec(),
        id: ReviewId(id),
        proposal_id: ProposalId(pid),
        outcome: r.outcome,
        cash: r.initial_cash,
    })
}
#[cfg(test)]
#[derive(Clone, Copy)]
enum Probe {
    Payload,
    Owned,
    Tail,
    LegacyShape,
    LegacyInitialCash,
    LegacyCashPartitions,
    MetadataQuery,
    BindingQuery,
    PayloadQuery,
    Observed,
}
#[cfg(test)]
thread_local! {static HITS:std::cell::Cell<[usize;10]>=const{std::cell::Cell::new([0;10])};static HOOK:RefCell<Option<Box<dyn FnMut(&mut SqliteConnection)->Result<(),Error>>>>=RefCell::new(None);}
#[cfg(test)]
fn probe(p: Probe) {
    HITS.with(|h| {
        let mut x = h.get();
        x[p as usize] += 1;
        h.set(x);
    });
}
#[cfg(test)]
fn hook(c: &mut SqliteConnection) -> Result<(), Error> {
    HOOK.with(|h| match h.borrow_mut().as_mut() {
        Some(f) => f(c),
        None => Ok(()),
    })
}
#[cfg(test)]
#[path = "paper_funding_review_v1_tests.rs"]
mod tests;

#[cfg(test)]
thread_local! { static TAIL_FAIL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

fn validate_proposal(p: &Proposal, w: &mut Work) -> Result<(), Error> {
    let r = validate_proposal_inner(p, w);
    w.finish(r)
}
fn validate_binding(b: &Binding, w: &mut Work) -> Result<(), Error> {
    let r = validate_binding_inner(b, w);
    w.finish(r)
}
fn mismatch(p: &Proposal, b: &Binding, w: &mut Work) -> Result<Option<FundingMismatchV1>, Error> {
    let r = mismatch_inner(p, b, w);
    w.finish(r)
}
fn allocation(
    p: &Proposal,
    g: &Projection,
    w: &mut Work,
) -> Result<(Outcome, Option<CashPartitions>), Error> {
    let r = allocation_inner(p, g, w);
    w.finish(r)
}
fn extract(
    c: &mut SqliteConnection,
    p: &VerifiedCatalog8<'_>,
    account: &str,
    w: &mut Work,
) -> Result<(Binding, Vec<u8>), Error> {
    let r = extract_inner(c, p, account, w);
    w.finish(r)
}

fn validate_cash_partitions(c: &CashPartitions, w: &mut Work) -> Result<(), Error> {
    w.own(LEGACY_ERROR_RESERVATION)?;
    #[cfg(test)]
    probe(Probe::LegacyCashPartitions);
    let result = c.validate().map_err(|_| Error::Schema);
    w.finish(result)
}


// This sibling loan is an owning historical material, not an approval or a
// recovered DatabaseConnectionAuthority. There is no caller-selected slot/time.
pub(super) struct MaterialReviewSource {
    original: StoredFundingReviewV1,
    work: Work,
    record: Option<Record>,
    proposal_bytes: Option<Vec<u8>>,
    proposal_hash: Option<String>,
    review_hash: Option<String>,
    tuple: Option<Vec<u8>>,
    slot: Option<String>,
    first: Option<Error>,
    started: bool,
    ready: bool,
}
impl MaterialReviewSource {
    pub(super) fn new(original: StoredFundingReviewV1) -> Self {
        // Move the whole genuine return before admission, decoding or copying.
        Self { original, work: Work::new(), record: None, proposal_bytes: None,
            proposal_hash: None, review_hash: None, tuple: None, slot: None,
            first: None, started: false, ready: false }
    }
    pub(super) fn canonical(&self) -> &[u8] { self.original.canonical_bytes() }
    pub(super) fn review_id(&self) -> &str { self.original.review_id() }
    pub(super) fn proposal_id(&self) -> &str { self.original.proposal_id() }
    pub(super) fn tuple(&self) -> Option<&[u8]> { self.tuple.as_deref() }
    pub(super) fn slot(&self) -> Option<&str> { self.slot.as_deref() }
    pub(super) fn first(&self) -> Option<Error> { self.first }
    pub(super) fn prepare(&mut self) -> Result<(), Error> {
        if self.started {
            let error = self.first.unwrap_or(Error::ChangedObservation);
            if self.first.is_none() { self.first = Some(error); }
            self.ready = false;
            return self.work.finish(Err(error));
        }
        self.started = true;
        let result = self.prepare_inner();
        let result = self.work.finish(result);
        match result {
            Ok(()) => { self.ready = true; Ok(()) }
            Err(error) => { self.first = Some(error); Err(error) }
        }
    }
    fn prepare_inner(&mut self) -> Result<(), Error> {
        self.record = Some(codec::record(self.original.canonical_bytes(), &mut self.work)?);
        // The actual owned decoder return remains in this frame on every cut.
        let r = self.record.as_ref().unwrap();
        validate_proposal(&r.proposal, &mut self.work)?;
        validate_binding(&r.actual_binding, &mut self.work)?;
        if r.schema != "paper-funding-review-v1" || r.version != 1
            || r.authority_state != "HistoricalObservationOnly" || r.approval_state != "NotIssued" {
            return Err(Error::Schema);
        }
        self.proposal_bytes = Some(codec::encode(&r.proposal, codec::PROPOSAL_LIMIT, &mut self.work)?);
        self.proposal_hash = Some(codec::id("paper-funding-proposal-v1:",
            b"stock_analysis.paper_funding.proposal.v1\0", self.proposal_bytes.as_ref().unwrap(), &mut self.work)?);
        self.work.scan(8192)?;
        if self.proposal_hash.as_deref() != Some(r.proposal_id.as_str())
            || r.proposal_id != self.original.proposal_id() { return Err(Error::Schema); }
        let expected = mismatch(&r.proposal, &r.actual_binding, &mut self.work)?;
        match (r.outcome, expected, &r.initial_cash) {
            (Outcome::ConsistentProposal, None, Some(c)) => {
                validate_cash_partitions(c, &mut self.work)?;
                if c.strategy_cash != r.proposal.budget.initial_strategy_cash_micro_cny
                    || c.strategy_cash > r.proposal.budget.authorized_budget_micro_cny { return Err(Error::Schema); }
            }
            (Outcome::InconsistentProposal(FundingMismatchV1::AllocationAgainstGenesis), None, None) => {}
            (Outcome::InconsistentProposal(a), Some(b), None) if a == b => {}
            _ => return Err(Error::Schema),
        }
        if r.outcome != self.original.outcome || r.initial_cash.as_ref() != self.original.cash.as_ref() {
            return Err(Error::Schema);
        }
        codec::canonical(r, self.original.canonical_bytes(), codec::REVIEW_LIMIT, &mut self.work)?;
        self.review_hash = Some(codec::id("paper-funding-review-v1:",
            b"stock_analysis.paper_funding.review.v1\0", self.original.canonical_bytes(), &mut self.work)?);
        if self.review_hash.as_deref() != Some(self.original.review_id()) { return Err(Error::Schema); }
        let version = r.version.to_le_bytes();
        let fields: [&[u8]; 6] = [b"paper-funding-review-material-v1", r.schema.as_bytes(), &version,
            r.proposal.account_id.as_bytes(), r.proposal.epoch_id.as_bytes(), r.proposal.budget.family_id.as_bytes()];
        let size = fields.iter().try_fold(0usize, |n, field| n.checked_add(8).and_then(|n| n.checked_add(field.len())))
            .ok_or(Error::OwnedBudget)?;
        // All six length prefixes and bytes are reserved before the first copy.
        self.work.own(size)?; self.work.scan(size)?;
        self.tuple = Some(Vec::with_capacity(size));
        let tuple = self.tuple.as_mut().unwrap();
        for field in fields {
            let length = u64::try_from(field.len()).map_err(|_| Error::ArithmeticOverflow)?;
            tuple.extend_from_slice(&length.to_le_bytes()); tuple.extend_from_slice(field);
        }
        self.slot = Some(codec::id("paper-funding-review-material-v1:",
            b"stock_analysis.paper_funding.material.slot.v1\0", tuple, &mut self.work)?);
        Ok(())
    }
    #[cfg(test)]
    fn exhaust_before_prepare(&mut self) {
        assert!(!self.started);
        self.work.own(8 * codec::MIB).unwrap();
    }
}


#[cfg(test)]
mod material_tests {
    use super::*;
    use super::tests::Fixture as FundingFixture;
    use crate::trading::paper_funding_review_store_v1::*;
    use crate::evidence_retention::{TrustState, ValueError};
    use crate::evidence_retention::outbox_v1::{OutboxFixture, UnverifiedOutbox, LocalDisposition,
        OutboxFault, RecoveredPresence, TestCommitObservation};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn actual(fixture: &FundingFixture, proposal: &Proposal) -> StoredFundingReviewV1 {
        let observed = fixture.review(proposal).unwrap();
        assert!(!observed.canonical_bytes().is_empty());
        read_stored_funding_review(observed.canonical_bytes()).unwrap()
    }
    fn command(value: StoredFundingReviewV1) -> FundingMaterialCommand {
        match prepare_review_material(value) {
            Ok(value) => value, Err(value) => panic!("material prepare Held {:?}",value.first_fault()),
        }
    }
    fn open(fixture: &OutboxFixture) -> UnverifiedOutbox {
        match fixture.open() { Ok(value) => value, Err(value) => panic!("actual outbox open {:?}",value.first_fault()) }
    }
    fn close(value: UnverifiedOutbox) {
        if let Err(value) = value.close() { panic!("actual close {:?}",value.first_fault()); }
    }
    fn stored(value: FundingMaterialOutcome) -> StoredFundingMaterial {
        match value { FundingMaterialOutcome::Stored(value) => value,
            FundingMaterialOutcome::Held(value) => panic!("material Held {:?}",value.first_fault()),
            FundingMaterialOutcome::Pending(value) => panic!("material Pending {:?}",value.first_fault()),
            _ => panic!("unexpected recovery"), }
    }
    fn held(value: FundingMaterialOutcome) -> HeldFundingMaterial {
        match value { FundingMaterialOutcome::Held(value) => value, _ => panic!("expected actual Held") }
    }
    fn recovered(value: FundingMaterialOutcome) -> RecoveredFundingMaterial {
        match value { FundingMaterialOutcome::Recovered(value) => value, _ => panic!("expected actual closed recovery") }
    }

    #[test]
    fn funding_material_nonempty_cold_reuse_and_same_family_conflicts() {
        let funding = FundingFixture::new(2);
        let mut proposal = funding.proposal();
        let review = actual(&funding,&proposal);
        let raw = review.canonical_bytes().to_vec();
        let original_pointer = review.canonical_bytes().as_ptr();
        let review_id = review.review_id().to_owned();
        let proposal_id = review.proposal_id().to_owned();
        let first = command(review);
        assert_eq!(first.canonical_review().as_ptr(),original_pointer);
        assert_eq!((first.review_id(),first.proposal_id()),(review_id.as_str(),proposal_id.as_str()));
        let slot = first.slot().to_owned(); let tuple = first.slot_tuple().to_vec();
        let wire: serde_json::Value = serde_json::from_slice(first.test_draft().as_canonical_bytes()).unwrap();
        assert_eq!(wire["owner_domain"],"PaperLedger");
        assert_eq!(wire["owner_schema_claim"],"paper-funding-review-material-v1");
        assert_eq!(wire["business_day_claim"],"1970-01-01");
        assert_eq!(wire["window_start_claim"],serde_json::json!({"unix_seconds":0,"nanosecond":0}));
        assert_eq!(wire["window_end_exclusive_claim"],serde_json::json!({"unix_seconds":1,"nanosecond":0}));
        assert_eq!(wire["claimed_record_count"],1);
        for key in ["source_chain_before_claim","source_chain_after_claim","artifact_sha256_claim","activation_id_claim"] {
            assert!(wire[key].is_null());
        }
        assert_eq!(wire["body_length"].as_u64(),Some(u64::try_from(raw.len()).unwrap()));
        let box_fixture = OutboxFixture::new();
        let installed = stored(first.persist(open(&box_fixture),0));
        assert_eq!((installed.generation(),installed.disposition(),installed.trust()),(1,LocalDisposition::Stored,TrustState::Unverified));
        assert_eq!(installed.canonical_review(),raw);
        assert_eq!((installed.review_id(),installed.proposal_id()),(review_id.as_str(),proposal_id.as_str()));
        assert_eq!(installed.slot_tuple(),tuple);
        // A newly opened actual connection observes the original attempt; no resend.
        let cold = recovered(command(read_stored_funding_review(&raw).unwrap()).observe_previous(open(&box_fixture),1));
        assert_eq!(cold.presence(),RecoveredPresence::ExactUnverified);
        assert_eq!(cold.original_fault(),FundingMaterialFault::Outbox(OutboxFault::CommitUnknown));
        assert_eq!((cold.canonical_review(),cold.trust()),(raw.as_slice(),TrustState::Unverified));
        let reused = stored(command(read_stored_funding_review(&raw).unwrap()).persist(open(&box_fixture),1));
        assert_eq!((reused.generation(),reused.disposition()),(2,LocalDisposition::ExactReuse));
        proposal.budget.concentration_bps = 9_000;
        let policy = command(actual(&funding,&proposal));
        assert_eq!((policy.slot(),policy.slot_tuple()),(slot.as_str(),tuple.as_slice()));
        assert_ne!(policy.canonical_review(),raw);
        let policy_raw = policy.canonical_review().to_vec();
        let changed_policy = stored(policy.persist(open(&box_fixture),2));
        assert_eq!((changed_policy.generation(),changed_policy.disposition()),(3,LocalDisposition::Conflict));
        proposal.genesis_event_hash = "b".repeat(64);
        let anchored = command(actual(&funding,&proposal));
        assert_eq!((anchored.slot(),anchored.slot_tuple()),(slot.as_str(),tuple.as_slice()));
        assert_ne!(anchored.canonical_review(),policy_raw);
        assert_eq!(stored(anchored.persist(open(&box_fixture),3)).disposition(),LocalDisposition::Conflict);
        let mut inventory = open(&box_fixture);
        assert_eq!((inventory.generation(),inventory.test_material_count(),inventory.test_conflict_count()),(4,3,3));
        close(inventory);
        let missing = OutboxFixture::new();
        let absent = recovered(command(read_stored_funding_review(&raw).unwrap()).observe_previous(open(&missing),1));
        assert_eq!(absent.presence(),RecoveredPresence::MissingFactsUnknown);
        let mut empty = open(&missing);
        assert_eq!((empty.generation(),empty.test_material_count()),(0,0)); close(empty);
        assert!(crate::decision::approved_paper_intent_v1::require_production_approval().is_err());
    }

    #[test]
    fn funding_material_actual_cas_unknown_and_consuming_close() {
        let funding = FundingFixture::new(2); let proposal = funding.proposal();
        let raw = actual(&funding,&proposal).canonical_bytes().to_vec();
        let fixture = OutboxFixture::new();
        let a = open(&fixture); let b = open(&fixture);
        assert_eq!((a.generation(),b.generation()),(0,0));
        assert_eq!(stored(command(read_stored_funding_review(&raw).unwrap()).persist(a,0)).generation(),1);
        let stale = held(command(read_stored_funding_review(&raw).unwrap()).persist(b,0));
        assert_eq!(stale.first_fault(),FundingMaterialFault::Outbox(OutboxFault::StaleGeneration));
        assert_eq!(stale.canonical_review(),raw);
        assert!(stale.test_outbox_held().unwrap().test_command_retained());
        let stale = stale.drain_resources_once();
        assert_eq!(stale.first_fault(),FundingMaterialFault::Outbox(OutboxFault::StaleGeneration)); drop(stale);
        let mut a = open(&fixture); let b = open(&fixture); a.test_hold_transaction().unwrap();
        let busy = held(command(read_stored_funding_review(&raw).unwrap()).persist(b,1));
        assert_eq!(busy.first_fault(),FundingMaterialFault::Outbox(OutboxFault::Busy));
        assert!(busy.test_outbox_held().unwrap().test_connection_retained()); drop(busy.drain_resources_once());
        if let Err(value) = a.test_rollback() { panic!("actual rollback {:?}",value.first_fault()); }
        let unknown = OutboxFixture::new();
        let pending = match command(read_stored_funding_review(&raw).unwrap())
            .persist(open(&unknown).test_observation(TestCommitObservation::LoseResponse),0) {
            FundingMaterialOutcome::Pending(value) => value, _ => panic!("actual committed response loss"),
        };
        assert_eq!(pending.first_fault(),FundingMaterialFault::Outbox(OutboxFault::CommitUnknown));
        assert_eq!(pending.canonical_review(),raw); assert!(pending.test_actual_owner_retained());
        let observed = recovered(pending.observe());
        assert_eq!(observed.presence(),RecoveredPresence::ExactUnverified);
        assert_eq!(observed.original_fault(),FundingMaterialFault::Outbox(OutboxFault::CommitUnknown));
        let close_fixture = OutboxFixture::new();
        let failed = held(command(read_stored_funding_review(&raw).unwrap()).persist(open(&close_fixture).test_busy_vm(),0));
        assert_eq!(failed.first_fault(),FundingMaterialFault::Outbox(OutboxFault::CloseHeld));
        assert_eq!(failed.canonical_review(),raw);
        assert!(failed.test_outbox_held().unwrap().test_connection_retained());
        let failed = failed.test_finalize_then_drain();
        assert_eq!(failed.first_fault(),FundingMaterialFault::Outbox(OutboxFault::CloseHeld));
        assert!(!failed.test_outbox_held().unwrap().test_connection_retained());
        assert_eq!(failed.drain_resources_once().canonical_review(),raw);
        let cold = recovered(command(read_stored_funding_review(&raw).unwrap()).observe_previous(open(&close_fixture),1));
        assert_eq!(cold.presence(),RecoveredPresence::ExactUnverified);
    }

    #[test]
    fn funding_material_namespace_false_approval_and_same_work_short() {
        let funding = FundingFixture::new(2); let proposal = funding.proposal();
        let original = actual(&funding,&proposal);
        let raw = original.canonical_bytes().to_vec(); let pointer = original.canonical_bytes().as_ptr();
        let mut source = MaterialReviewSource::new(original); source.exhaust_before_prepare();
        let mut failed = match prepare_exhausted_source(source) { Err(value) => value, Ok(_) => panic!("real short Work") };
        assert_eq!(failed.first_fault(),FundingMaterialFault::Review(Error::OwnedBudget));
        assert_eq!(failed.canonical_review().as_ptr(),pointer); assert_eq!(failed.canonical_review(),raw);
        assert_eq!(failed.trust(),TrustState::Unverified); assert!(failed.slot_tuple().is_none());
        // Drain is resource-only, never another prepare budget or command admission.
        failed = failed.drain_resources_once();
        assert_eq!(failed.first_fault(),FundingMaterialFault::Review(Error::OwnedBudget));
        assert_eq!(failed.canonical_review().as_ptr(),pointer);
        let mut once = MaterialReviewSource::new(read_stored_funding_review(&raw).unwrap());
        let original_pointer = once.canonical().as_ptr();
        once.prepare().unwrap(); let tuple_pointer = once.tuple().unwrap().as_ptr();
        assert_eq!(once.prepare(),Err(Error::ChangedObservation));
        assert_eq!(once.canonical().as_ptr(),original_pointer);
        assert_eq!(once.tuple().unwrap().as_ptr(),tuple_pointer);
        assert_eq!(once.first(),Some(Error::ChangedObservation));
        let wrong = String::from_utf8(raw.clone()).unwrap().replace("NotIssued","Approved");
        assert!(read_stored_funding_review(wrong.as_bytes()).is_err());
        let wrong = String::from_utf8(raw.clone()).unwrap().replace("HistoricalObservationOnly","Approved");
        assert!(read_stored_funding_review(wrong.as_bytes()).is_err());
        let mut noncanonical = raw.clone(); noncanonical.push(b' ');
        assert!(read_stored_funding_review(&noncanonical).is_err());
        let budget = OutboxFixture::new(); let mut short = open(&budget);
        assert_eq!(short.test_spend_owned(8 * codec::MIB),Err(OutboxFault::Work(ValueError::AllocationLimit)));
        let original = read_stored_funding_review(&raw).unwrap(); let pointer = original.canonical_bytes().as_ptr();
        let refused = held(command(original).persist(short,0));
        assert_eq!(refused.first_fault(),FundingMaterialFault::Outbox(OutboxFault::Work(ValueError::AllocationLimit)));
        assert_eq!(refused.canonical_review().as_ptr(),pointer); assert!(refused.test_outbox_held().unwrap().test_command_retained());
        drop(refused.drain_resources_once());
        let mut pending = match command(read_stored_funding_review(&raw).unwrap())
            .persist(open(&budget).test_observation(TestCommitObservation::LoseResponse),0) {
            FundingMaterialOutcome::Pending(value) => value, _ => panic!("actual response loss"),
        };
        pending.test_exhaust_same_work();
        let refused = held(pending.observe());
        assert_eq!(refused.first_fault(),FundingMaterialFault::Outbox(OutboxFault::CommitUnknown));
        assert!(refused.test_outbox_held().unwrap().test_connection_retained());
        assert_eq!(refused.canonical_review(),raw); drop(refused.drain_resources_once());
        let foreign = OutboxFixture::new(); let outbox = open(&foreign);
        fs::write(foreign.directory().join("foreign-entry"),b"unadmitted").unwrap();
        let refused = held(command(read_stored_funding_review(&raw).unwrap()).persist(outbox,0));
        assert_eq!(refused.first_fault(),FundingMaterialFault::Outbox(OutboxFault::ForeignShape));
        assert_eq!(refused.canonical_review(),raw); assert!(refused.test_outbox_held().unwrap().test_command_retained());
        drop(refused.drain_resources_once()); fs::remove_file(foreign.directory().join("foreign-entry")).unwrap();
        let mut untouched = open(&foreign); assert_eq!((untouched.generation(),untouched.test_material_count()),(0,0)); close(untouched);
        let swapped = OutboxFixture::new(); let outbox = open(&swapped);
        let saved = swapped.directory().join("old-main"); fs::rename(swapped.main(),&saved).unwrap();
        fs::copy(&saved,swapped.main()).unwrap(); fs::set_permissions(swapped.main(),fs::Permissions::from_mode(0o600)).unwrap();
        let refused = held(command(read_stored_funding_review(&raw).unwrap()).persist(outbox,0));
        assert_eq!(refused.first_fault(),FundingMaterialFault::Outbox(OutboxFault::RootBinding));
        assert!(refused.test_outbox_held().unwrap().test_connection_retained());
        assert_eq!(refused.canonical_review(),raw); drop(refused.drain_resources_once());
        fs::remove_file(swapped.main()).unwrap(); fs::rename(saved,swapped.main()).unwrap();
        close(open(&swapped));
    }
}
