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
    let m = diesel::sql_query(META_SQL)
        .bind::<Text, _>(account)
        .get_result::<Meta>(conn)
        .map_err(|_| Error::SqlRead)?;
    let n = checked_meta(&m)?;
    work.own(4096 + std::mem::size_of::<Binding>())?;
    work.own(n)?;
    work.scan(4096 + n)?;
    #[cfg(test)]
    probe(Probe::Payload);
    let b = diesel::sql_query(BINDING_SQL)
        .bind::<Text, _>(account)
        .get_result::<Binding>(conn)
        .map_err(|_| Error::SqlRead)?;
    let raw = diesel::sql_query(
        "SELECT projection_bytes AS value FROM main.paper_book_v2_head WHERE account_id=? LIMIT 2",
    )
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
}
#[cfg(test)]
thread_local! {static HITS:std::cell::Cell<[usize;6]>=const{std::cell::Cell::new([0;6])};static HOOK:RefCell<Option<Box<dyn FnMut(&mut SqliteConnection)->Result<(),Error>>>>=RefCell::new(None);}
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
