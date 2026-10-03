//! Bounded, unverified historical material. No I/O, provider, owner seal,
//! signing or conversion to live retention / trading authority exists here.
mod codec_v1;
use codec_v1::{Shape, Work, DRAFT_LIMIT, MIB, RECEIPT_LIMIT, ROOT_LIMIT};
use serde::{Deserialize, Serialize};
const DP: &str = "retention-package-draft-v1:";
const RP: &str = "retention-stored-receipt-v1:";
const UP: &str = "retention-incomplete-root-v1:";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueError {
    InputLimit,
    DepthLimit,
    NodeLimit,
    AllocationLimit,
    MalformedJson,
    WrongType,
    UnknownField,
    DuplicateField,
    MissingField,
    InvalidScalar,
    NonCanonical,
    UnsupportedSchema,
    InvalidTime,
    InvalidHash,
    BodyMismatch,
    LogicalSlotConflict,
    RootBindingMismatch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum OwnerDomain {
    Data,
    InvestmentDecision,
    PaperLedger,
    Attribution,
}
const DOMAINS: [OwnerDomain; 4] = [
    OwnerDomain::Data,
    OwnerDomain::InvestmentDecision,
    OwnerDomain::PaperLedger,
    OwnerDomain::Attribution,
];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TrustState {
    Unverified,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CoverageState {
    Incomplete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SignatureState {
    Unsigned,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UtcInstantClaim {
    pub(crate) unix_seconds: u64,
    pub(crate) nanosecond: u32,
}
impl UtcInstantClaim {
    fn valid(self) -> bool {
        self.unix_seconds <= 253402300799 && self.nanosecond < 1_000_000_000
    }
    fn required(self) -> Option<Self> {
        let n = Self {
            unix_seconds: self.unix_seconds.checked_add(158112000)?,
            ..self
        };
        n.valid().then_some(n)
    }
}
fn text_ok(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn hash_ok(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|x| x.is_ascii_digit() || (b'a'..=b'f').contains(&x))
}
fn id_ok(s: &str, p: &str) -> bool {
    s.strip_prefix(p).is_some_and(hash_ok)
}
fn day_ok(s: &str) -> bool {
    s.len() == 10
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s.bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
        && s >= "1970-01-01"
        && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}
fn optional_text(s: &Option<String>, max: usize) -> bool {
    s.as_deref().is_none_or(|s| text_ok(s, max))
}
fn optional_hash(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(hash_ok)
}
fn opt_copy(s: Option<&str>, w: &mut Work) -> Result<Option<String>, ValueError> {
    s.map(|s| codec_v1::text(s, w)).transpose()
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftWire {
    schema: String,
    schema_version: u8,
    trust: TrustState,
    owner_domain: OwnerDomain,
    owner_schema_claim: String,
    logical_slot_claim: String,
    business_day_claim: String,
    window_start_claim: UtcInstantClaim,
    window_end_exclusive_claim: UtcInstantClaim,
    claimed_record_count: Option<u64>,
    source_chain_before_claim: Option<String>,
    source_chain_after_claim: Option<String>,
    artifact_sha256_claim: Option<String>,
    activation_id_claim: Option<String>,
    body_encoding: String,
    body_length: u64,
    body_sha256: String,
    body_hex: String,
}
pub(crate) struct DraftClaimsRef<'a> {
    pub(crate) owner_domain: OwnerDomain,
    pub(crate) owner_schema_claim: &'a str,
    pub(crate) logical_slot_claim: &'a str,
    pub(crate) business_day_claim: &'a str,
    pub(crate) window_start_claim: UtcInstantClaim,
    pub(crate) window_end_exclusive_claim: UtcInstantClaim,
    pub(crate) claimed_record_count: Option<u64>,
    pub(crate) source_chain_before_claim: Option<&'a str>,
    pub(crate) source_chain_after_claim: Option<&'a str>,
    pub(crate) artifact_sha256_claim: Option<&'a str>,
    pub(crate) activation_id_claim: Option<&'a str>,
}
pub(crate) struct UnverifiedEvidencePackageDraft {
    canonical: Vec<u8>,
    id: String,
    sha256: String,
    owner: OwnerDomain,
    slot: String,
    day: String,
}
impl UnverifiedEvidencePackageDraft {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn as_canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
}
fn validate_draft(d: &DraftWire, w: &mut Work) -> Result<(), ValueError> {
    if d.schema != "retention-package-draft-v1" || d.schema_version != 1 || d.body_encoding != "hex"
    {
        return Err(ValueError::UnsupportedSchema);
    }
    if !text_ok(&d.owner_schema_claim, 128)
        || !text_ok(&d.logical_slot_claim, 256)
        || !day_ok(&d.business_day_claim)
        || !optional_text(&d.activation_id_claim, 256)
        || d.claimed_record_count.is_some_and(|n| n > 1_000_000_000)
    {
        return Err(ValueError::InvalidScalar);
    }
    if !d.window_start_claim.valid()
        || !d.window_end_exclusive_claim.valid()
        || d.window_start_claim >= d.window_end_exclusive_claim
    {
        return Err(ValueError::InvalidTime);
    }
    if !optional_hash(&d.source_chain_before_claim)
        || !optional_hash(&d.source_chain_after_claim)
        || !optional_hash(&d.artifact_sha256_claim)
        || !hash_ok(&d.body_sha256)
    {
        return Err(ValueError::InvalidHash);
    }
    if d.body_length > MIB as u64 || d.body_hex.len() != d.body_length as usize * 2 {
        return Err(ValueError::BodyMismatch);
    }
    w.scan(d.body_hex.len())?;
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut chunk = [0u8; 4096];
    let mut n = 0;
    for p in d.body_hex.as_bytes().chunks_exact(2) {
        let f = |b: u8| match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            _ => Err(ValueError::InvalidHash),
        };
        chunk[n] = f(p[0])? * 16 + f(p[1])?;
        n += 1;
        if n == chunk.len() {
            hash.update(chunk);
            n = 0;
        }
    }
    hash.update(&chunk[..n]);
    w.own(64)?;
    if hex::encode(hash.finalize()) != d.body_sha256 {
        return Err(ValueError::BodyMismatch);
    }
    Ok(())
}
fn finish_draft(
    d: DraftWire,
    b: Vec<u8>,
    w: &mut Work,
) -> Result<UnverifiedEvidencePackageDraft, ValueError> {
    let id = codec_v1::id(DP, b"stock_analysis.retention.package-draft.v1\0", &b, w)?;
    let sha256 = codec_v1::digest(&b, w)?;
    Ok(UnverifiedEvidencePackageDraft {
        canonical: b,
        id,
        sha256,
        owner: d.owner_domain,
        slot: d.logical_slot_claim,
        day: d.business_day_claim,
    })
}
pub(crate) fn parse_draft(b: &[u8]) -> Result<UnverifiedEvidencePackageDraft, ValueError> {
    let w = &mut Work::new();
    let d: DraftWire = codec_v1::decode(b, Shape::Draft, DRAFT_LIMIT, w)?;
    validate_draft(&d, w)?;
    codec_v1::canonical(&d, b, w)?;
    let b = codec_v1::copy(b, w)?;
    finish_draft(d, b, w)
}
pub(crate) fn draft_from_claims(
    c: DraftClaimsRef<'_>,
    raw: &[u8],
) -> Result<UnverifiedEvidencePackageDraft, ValueError> {
    if raw.len() > MIB {
        return Err(ValueError::InputLimit);
    }
    if !text_ok(c.owner_schema_claim, 128)
        || !text_ok(c.logical_slot_claim, 256)
        || !day_ok(c.business_day_claim)
        || c.activation_id_claim.is_some_and(|s| !text_ok(s, 256))
        || c.claimed_record_count.is_some_and(|n| n > 1_000_000_000)
    {
        return Err(ValueError::InvalidScalar);
    }
    if !c.window_start_claim.valid()
        || !c.window_end_exclusive_claim.valid()
        || c.window_start_claim >= c.window_end_exclusive_claim
    {
        return Err(ValueError::InvalidTime);
    }
    if [
        c.source_chain_before_claim,
        c.source_chain_after_claim,
        c.artifact_sha256_claim,
    ]
    .into_iter()
    .flatten()
    .any(|s| !hash_ok(s))
    {
        return Err(ValueError::InvalidHash);
    }
    let w = &mut Work::new();
    w.own(std::mem::size_of::<DraftWire>())?;
    w.own(raw.len() * 2)?;
    w.scan(raw.len())?;
    let body_hex = hex::encode(raw);
    let d = DraftWire {
        schema: codec_v1::text("retention-package-draft-v1", w)?,
        schema_version: 1,
        trust: TrustState::Unverified,
        owner_domain: c.owner_domain,
        owner_schema_claim: codec_v1::text(c.owner_schema_claim, w)?,
        logical_slot_claim: codec_v1::text(c.logical_slot_claim, w)?,
        business_day_claim: codec_v1::text(c.business_day_claim, w)?,
        window_start_claim: c.window_start_claim,
        window_end_exclusive_claim: c.window_end_exclusive_claim,
        claimed_record_count: c.claimed_record_count,
        source_chain_before_claim: opt_copy(c.source_chain_before_claim, w)?,
        source_chain_after_claim: opt_copy(c.source_chain_after_claim, w)?,
        artifact_sha256_claim: opt_copy(c.artifact_sha256_claim, w)?,
        activation_id_claim: opt_copy(c.activation_id_claim, w)?,
        body_encoding: codec_v1::text("hex", w)?,
        body_length: raw.len() as u64,
        body_sha256: codec_v1::digest(raw, w)?,
        body_hex,
    };
    validate_draft(&d, w)?;
    let b = codec_v1::encode(&d, DRAFT_LIMIT, w)?;
    codec_v1::preflight(&b, Shape::Draft, DRAFT_LIMIT, w)?;
    finish_draft(d, b, w)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum RetentionClaim {
    Compliance,
    Locked,
    Governance,
    Unlocked,
    Unknown,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptWire {
    schema: String,
    schema_version: u8,
    trust: TrustState,
    package_id: String,
    storage_authority_claim: String,
    container_claim: String,
    object_key_claim: String,
    version_id_claim: Option<String>,
    retention_mode_claim: RetentionClaim,
    clock_evidence: TrustState,
    confirmation_upper_bound_claim: Option<UtcInstantClaim>,
    retain_until_claim: Option<UtcInstantClaim>,
    head_content_length_claim: Option<u64>,
    get_content_length_claim: Option<u64>,
    get_sha256_claim: Option<String>,
    readback_complete_claim: bool,
    request_id_claims: Vec<String>,
}
pub(crate) struct StoredExactVersionReceipt {
    canonical: Vec<u8>,
    id: String,
    value: ReceiptWire,
}
impl StoredExactVersionReceipt {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn as_canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
}
pub(crate) fn parse_stored_receipt(b: &[u8]) -> Result<StoredExactVersionReceipt, ValueError> {
    let w = &mut Work::new();
    let d: ReceiptWire = codec_v1::decode(b, Shape::Receipt, RECEIPT_LIMIT, w)?;
    if d.schema != "retention-stored-version-receipt-v1" || d.schema_version != 1 {
        return Err(ValueError::UnsupportedSchema);
    }
    if !id_ok(&d.package_id, DP) || !optional_hash(&d.get_sha256_claim) {
        return Err(ValueError::InvalidHash);
    }
    if !text_ok(&d.storage_authority_claim, 256)
        || !text_ok(&d.container_claim, 256)
        || !text_ok(&d.object_key_claim, 1024)
        || !optional_text(&d.version_id_claim, 2048)
        || d.version_id_claim.as_deref() == Some("null")
        || [d.head_content_length_claim, d.get_content_length_claim]
            .into_iter()
            .flatten()
            .any(|n| n > DRAFT_LIMIT as u64)
    {
        return Err(ValueError::InvalidScalar);
    }
    if [d.confirmation_upper_bound_claim, d.retain_until_claim]
        .into_iter()
        .flatten()
        .any(|t| !t.valid())
    {
        return Err(ValueError::InvalidTime);
    }
    for (i, s) in d.request_id_claims.iter().enumerate() {
        if !text_ok(s, 256) || d.request_id_claims[..i].contains(s) {
            return Err(ValueError::InvalidScalar);
        }
    }
    codec_v1::canonical(&d, b, w)?;
    let id = codec_v1::id(RP, b"stock_analysis.retention.stored-receipt.v1\0", b, w)?;
    Ok(StoredExactVersionReceipt {
        canonical: codec_v1::copy(b, w)?,
        id,
        value: d,
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ClaimConsistency {
    ConsistentClaims,
    InconsistentClaims,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetentionArithmetic {
    NotComputable,
    ShortClaim,
    SufficientClaim,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceiptIssue {
    PackageIdMismatch,
    VersionMissing,
    RetentionModeUnsupportedClaim,
    ConfirmationMissing,
    RetainUntilMissing,
    TimeOverflow,
    RetentionTooShortClaim,
    HeadLengthMissing,
    HeadLengthMismatch,
    GetLengthMissing,
    GetLengthMismatch,
    GetHashMissing,
    GetHashMismatch,
    ReadbackIncompleteClaim,
}
pub(crate) struct StoredReceiptCheck {
    consistency: ClaimConsistency,
    arithmetic: RetentionArithmetic,
    issues: Vec<ReceiptIssue>,
}
impl StoredReceiptCheck {
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
    pub(crate) fn consistency(&self) -> ClaimConsistency {
        self.consistency
    }
    pub(crate) fn arithmetic(&self) -> RetentionArithmetic {
        self.arithmetic
    }
    pub(crate) fn issues(&self) -> &[ReceiptIssue] {
        &self.issues
    }
}
fn check_with(
    d: &UnverifiedEvidencePackageDraft,
    r: &StoredExactVersionReceipt,
    w: &mut Work,
) -> Result<StoredReceiptCheck, ValueError> {
    use ReceiptIssue::*;
    w.own(14 * std::mem::size_of::<ReceiptIssue>())?;
    let mut issues = Vec::with_capacity(14);
    let r = &r.value;
    // Recompute from the actual immutable canonical carrier, never a caller hash.
    let id = codec_v1::id(
        DP,
        b"stock_analysis.retention.package-draft.v1\0",
        &d.canonical,
        w,
    )?;
    let hash = codec_v1::digest(&d.canonical, w)?;
    if r.package_id != id {
        issues.push(PackageIdMismatch)
    }
    if r.version_id_claim.is_none() {
        issues.push(VersionMissing)
    }
    if !matches!(
        r.retention_mode_claim,
        RetentionClaim::Compliance | RetentionClaim::Locked
    ) {
        issues.push(RetentionModeUnsupportedClaim)
    }
    if r.confirmation_upper_bound_claim.is_none() {
        issues.push(ConfirmationMissing)
    }
    if r.retain_until_claim.is_none() {
        issues.push(RetainUntilMissing)
    }
    let arithmetic = match (r.confirmation_upper_bound_claim, r.retain_until_claim) {
        (Some(t), until) => match t.required() {
            None => {
                issues.push(TimeOverflow);
                RetentionArithmetic::NotComputable
            }
            Some(required) => match until {
                Some(v) if v < required => {
                    issues.push(RetentionTooShortClaim);
                    RetentionArithmetic::ShortClaim
                }
                Some(_) => RetentionArithmetic::SufficientClaim,
                None => RetentionArithmetic::NotComputable,
            },
        },
        _ => RetentionArithmetic::NotComputable,
    };
    match r.head_content_length_claim {
        None => issues.push(HeadLengthMissing),
        Some(n) if n != d.canonical.len() as u64 => issues.push(HeadLengthMismatch),
        _ => {}
    }
    match r.get_content_length_claim {
        None => issues.push(GetLengthMissing),
        Some(n) if n != d.canonical.len() as u64 => issues.push(GetLengthMismatch),
        _ => {}
    }
    match r.get_sha256_claim.as_ref() {
        None => issues.push(GetHashMissing),
        Some(h) if h != &hash => issues.push(GetHashMismatch),
        _ => {}
    }
    if !r.readback_complete_claim {
        issues.push(ReadbackIncompleteClaim)
    }
    Ok(StoredReceiptCheck {
        consistency: if issues.is_empty() {
            ClaimConsistency::ConsistentClaims
        } else {
            ClaimConsistency::InconsistentClaims
        },
        arithmetic,
        issues,
    })
}
pub(crate) fn check_stored_receipt(
    d: &UnverifiedEvidencePackageDraft,
    r: &StoredExactVersionReceipt,
) -> Result<StoredReceiptCheck, ValueError> {
    check_with(d, r, &mut Work::new())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum CoverageClaim {
    NoMaterialProvided,
    UnverifiedMaterialProvided,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum NotObserved {
    NotObserved,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum NotConfigured {
    NotConfigured,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gates {
    owner_seal: NotObserved,
    remote_retention: NotObserved,
    signer: NotConfigured,
    restore: NotObserved,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    owner_domain: OwnerDomain,
    state: CoverageClaim,
    package_count: u64,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ReceiptConsistency {
    NoReceipt,
    ConsistentClaims,
    InconsistentClaims,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    owner_domain: OwnerDomain,
    logical_slot_claim: String,
    package_id: String,
    package_canonical_sha256: String,
    package_canonical_length: u64,
    receipt_id: Option<String>,
    receipt_claim_consistency: ReceiptConsistency,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootWire {
    schema: String,
    schema_version: u8,
    trust: TrustState,
    coverage_state: CoverageState,
    signature_state: SignatureState,
    authority_gates: Gates,
    business_day_claim: String,
    revision: u64,
    previous_day_root_id_claim: Option<String>,
    previous_revision_root_id_claim: Option<String>,
    coverage: Vec<Coverage>,
    entries: Vec<Entry>,
}
pub(crate) struct DailyRootClaimsRef<'a> {
    pub(crate) business_day_claim: &'a str,
    pub(crate) revision: u64,
    pub(crate) previous_day_root_id_claim: Option<&'a str>,
    pub(crate) previous_revision_root_id_claim: Option<&'a str>,
}
pub(crate) struct DraftAndReceiptRef<'a> {
    pub(crate) draft: &'a UnverifiedEvidencePackageDraft,
    pub(crate) receipt: Option<&'a StoredExactVersionReceipt>,
}
pub(crate) struct UnsignedOrIncompleteDailyRoot {
    canonical: Vec<u8>,
    id: String,
}
impl UnsignedOrIncompleteDailyRoot {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn as_canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    pub(crate) fn trust(&self) -> TrustState {
        TrustState::Unverified
    }
    pub(crate) fn coverage_state(&self) -> CoverageState {
        CoverageState::Incomplete
    }
    pub(crate) fn signature_state(&self) -> SignatureState {
        SignatureState::Unsigned
    }
}
fn valid_root_claims(c: &DailyRootClaimsRef<'_>) -> Result<(), ValueError> {
    if !day_ok(c.business_day_claim)
        || !(1..=1_000_000).contains(&c.revision)
        || (c.revision == 1) != c.previous_revision_root_id_claim.is_none()
    {
        return Err(ValueError::InvalidScalar);
    }
    if [
        c.previous_day_root_id_claim,
        c.previous_revision_root_id_claim,
    ]
    .into_iter()
    .flatten()
    .any(|s| !id_ok(s, UP))
    {
        return Err(ValueError::InvalidHash);
    }
    Ok(())
}
fn validate_root(r: &RootWire) -> Result<(), ValueError> {
    if r.schema != "retention-incomplete-daily-root-v1" || r.schema_version != 1 {
        return Err(ValueError::UnsupportedSchema);
    }
    valid_root_claims(&DailyRootClaimsRef {
        business_day_claim: &r.business_day_claim,
        revision: r.revision,
        previous_day_root_id_claim: r.previous_day_root_id_claim.as_deref(),
        previous_revision_root_id_claim: r.previous_revision_root_id_claim.as_deref(),
    })?;
    if r.coverage.len() != 4 || r.entries.len() > 128 {
        return Err(ValueError::RootBindingMismatch);
    }
    let mut counts = [0u64; 4];
    for (i, e) in r.entries.iter().enumerate() {
        if !text_ok(&e.logical_slot_claim, 256)
            || !id_ok(&e.package_id, DP)
            || !hash_ok(&e.package_canonical_sha256)
            || e.package_canonical_length == 0
            || e.package_canonical_length > DRAFT_LIMIT as u64
            || e.receipt_id.as_deref().is_some_and(|s| !id_ok(s, RP))
            || e.receipt_id.is_none()
                != (e.receipt_claim_consistency == ReceiptConsistency::NoReceipt)
        {
            return Err(ValueError::RootBindingMismatch);
        }
        if i > 0 {
            let p = &r.entries[i - 1];
            if (p.owner_domain, p.logical_slot_claim.as_bytes())
                == (e.owner_domain, e.logical_slot_claim.as_bytes())
            {
                return Err(ValueError::LogicalSlotConflict);
            }
            if (
                p.owner_domain,
                p.logical_slot_claim.as_bytes(),
                p.package_id.as_bytes(),
            ) >= (
                e.owner_domain,
                e.logical_slot_claim.as_bytes(),
                e.package_id.as_bytes(),
            ) {
                return Err(ValueError::NonCanonical);
            }
        }
        counts[e.owner_domain as usize] += 1;
    }
    for (i, c) in r.coverage.iter().enumerate() {
        if c.owner_domain != DOMAINS[i]
            || c.package_count != counts[i]
            || c.state
                != if counts[i] == 0 {
                    CoverageClaim::NoMaterialProvided
                } else {
                    CoverageClaim::UnverifiedMaterialProvided
                }
        {
            return Err(ValueError::RootBindingMismatch);
        }
    }
    Ok(())
}
fn root_id(r: &RootWire, b: &[u8], w: &mut Work) -> Result<String, ValueError> {
    let id = codec_v1::id(UP, b"stock_analysis.retention.incomplete-root.v1\0", b, w)?;
    if [
        r.previous_day_root_id_claim.as_deref(),
        r.previous_revision_root_id_claim.as_deref(),
    ]
    .contains(&Some(id.as_str()))
    {
        return Err(ValueError::RootBindingMismatch);
    }
    Ok(id)
}
pub(crate) fn parse_daily_root(b: &[u8]) -> Result<UnsignedOrIncompleteDailyRoot, ValueError> {
    let w = &mut Work::new();
    let r: RootWire = codec_v1::decode(b, Shape::Root, ROOT_LIMIT, w)?;
    validate_root(&r)?;
    codec_v1::canonical(&r, b, w)?;
    Ok(UnsignedOrIncompleteDailyRoot {
        id: root_id(&r, b, w)?,
        canonical: codec_v1::copy(b, w)?,
    })
}
pub(crate) fn build_daily_root(
    c: DailyRootClaimsRef<'_>,
    refs: &[DraftAndReceiptRef<'_>],
) -> Result<UnsignedOrIncompleteDailyRoot, ValueError> {
    valid_root_claims(&c)?;
    if refs.len() > 128 {
        return Err(ValueError::InputLimit);
    }
    let w = &mut Work::new();
    w.own(refs.len() * std::mem::size_of::<usize>())?;
    let mut order = Vec::with_capacity(refs.len());
    for (i, e) in refs.iter().enumerate() {
        if e.draft.day != c.business_day_claim {
            return Err(ValueError::RootBindingMismatch);
        }
        order.push(i)
    }
    order.sort_unstable_by(|a, b| {
        let a = refs[*a].draft;
        let b = refs[*b].draft;
        (a.owner, a.slot.as_bytes(), a.id.as_bytes()).cmp(&(
            b.owner,
            b.slot.as_bytes(),
            b.id.as_bytes(),
        ))
    });
    w.own(
        refs.len() * std::mem::size_of::<Entry>()
            + 4 * std::mem::size_of::<Coverage>()
            + std::mem::size_of::<RootWire>(),
    )?;
    let mut entries = Vec::with_capacity(refs.len());
    let mut counts = [0; 4];
    for i in order {
        let e = &refs[i];
        let d = e.draft;
        if entries
            .last()
            .is_some_and(|p: &Entry| p.owner_domain == d.owner && p.logical_slot_claim == d.slot)
        {
            return Err(ValueError::LogicalSlotConflict);
        }
        let consistency = match e.receipt {
            None => ReceiptConsistency::NoReceipt,
            Some(r) => match check_with(d, r, w)?.consistency {
                ClaimConsistency::ConsistentClaims => ReceiptConsistency::ConsistentClaims,
                ClaimConsistency::InconsistentClaims => ReceiptConsistency::InconsistentClaims,
            },
        };
        counts[d.owner as usize] += 1;
        entries.push(Entry {
            owner_domain: d.owner,
            logical_slot_claim: codec_v1::text(&d.slot, w)?,
            package_id: codec_v1::text(&d.id, w)?,
            package_canonical_sha256: codec_v1::text(&d.sha256, w)?,
            package_canonical_length: d.canonical.len() as u64,
            receipt_id: e.receipt.map(|r| codec_v1::text(&r.id, w)).transpose()?,
            receipt_claim_consistency: consistency,
        });
    }
    let mut coverage = Vec::with_capacity(4);
    for (i, domain) in DOMAINS.into_iter().enumerate() {
        coverage.push(Coverage {
            owner_domain: domain,
            state: if counts[i] == 0 {
                CoverageClaim::NoMaterialProvided
            } else {
                CoverageClaim::UnverifiedMaterialProvided
            },
            package_count: counts[i],
        })
    }
    let r = RootWire {
        schema: codec_v1::text("retention-incomplete-daily-root-v1", w)?,
        schema_version: 1,
        trust: TrustState::Unverified,
        coverage_state: CoverageState::Incomplete,
        signature_state: SignatureState::Unsigned,
        authority_gates: Gates {
            owner_seal: NotObserved::NotObserved,
            remote_retention: NotObserved::NotObserved,
            signer: NotConfigured::NotConfigured,
            restore: NotObserved::NotObserved,
        },
        business_day_claim: codec_v1::text(c.business_day_claim, w)?,
        revision: c.revision,
        previous_day_root_id_claim: opt_copy(c.previous_day_root_id_claim, w)?,
        previous_revision_root_id_claim: opt_copy(c.previous_revision_root_id_claim, w)?,
        coverage,
        entries,
    };
    validate_root(&r)?;
    let b = codec_v1::encode(&r, ROOT_LIMIT, w)?;
    codec_v1::preflight(&b, Shape::Root, ROOT_LIMIT, w)?;
    Ok(UnsignedOrIncompleteDailyRoot {
        id: root_id(&r, &b, w)?,
        canonical: b,
    })
}
#[cfg(test)]
mod tests;
