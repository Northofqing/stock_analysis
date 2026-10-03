//! Closed WG07 wire and native interpretation. No production source profile is delivered.
use super::ordinary_daily_change_window::{
    DiscoveryFailure, FailureKind, OrdinaryDailyChangeWindowRequest,
};
use crate::market_domain::{AssetClass, Exchange, InstrumentId};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

pub(crate) const REQUEST_SCHEMA: &str = "magic.market.ordinary_daily_change_window.request";
pub(crate) const RESULT_SCHEMA: &str = "magic.market.ordinary_daily_change_window.result";
pub(crate) const MIB: usize = 1024 * 1024;
pub(crate) const PROOF_LIMIT: usize = 8 * MIB;
pub(crate) const PREPARE_LIMIT: usize = 16 * MIB;
pub(crate) const TEST_PROFILE: &str = "TEST_CODE_SYNTHETIC@1";
pub(crate) const TEST_SCOPE: &str = "TEST_CODE_SYNTHETIC@1;magic.market.ordinary_daily_change_window.request@1;magic.market.ordinary_daily_change_window.result@1;Equity/Day/Unadjusted/Shanghai";

pub(crate) fn failure(kind: FailureKind, why: &str) -> DiscoveryFailure {
    DiscoveryFailure::one(kind, why)
}
pub(crate) type Result<T> = std::result::Result<T, DiscoveryFailure>;
fn ensure(ok: bool, kind: FailureKind, why: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(failure(kind, why))
    }
}
pub(crate) fn digest(domain: &[u8], bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(domain);
    h.update((bytes.len() as u64).to_be_bytes());
    h.update(bytes);
    hex::encode(h.finalize())
}
pub(crate) fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("WG07 encoding limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(crate) fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    let mut w = BoundedWriter {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut w, value)
        .map_err(|_| failure(FailureKind::EnvelopeRejected, "bounded encoding"))?;
    Ok(w.bytes)
}
/// Allocation-free JSON lexical preflight. serde still validates the grammar and
/// rejects duplicate struct fields. String lengths are conservative encoded limits.
pub(crate) fn preflight(bytes: &[u8], limit: usize) -> Result<()> {
    ensure(
        bytes.len() <= limit,
        FailureKind::EnvelopeRejected,
        "JSON byte limit",
    )?;
    let mut pos = 0;
    let mut depth = 0usize;
    let mut counts = [0usize; 64];
    let mut entry_limits = [1024usize; 64];
    let mut native_arrays = [false; 64];
    let mut native_entries = 0usize;
    let mut nonempty = [false; 64];
    let mut last_key = "";
    while pos < bytes.len() {
        match bytes[pos] {
            b'{' | b'[' => {
                ensure(depth < 64, FailureKind::EnvelopeRejected, "JSON depth")?;
                counts[depth] = 0;
                entry_limits[depth] = if bytes[pos] == b'['
                    && matches!(last_key, "sessions" | "expected_sessions" | "candidates")
                {
                    260
                } else {
                    1024
                };
                native_arrays[depth] = bytes[pos] == b'['
                    && matches!(last_key, "native_requests" | "native_responses");
                nonempty[depth] = false;
                if depth > 0 {
                    nonempty[depth - 1] = true;
                }
                last_key = "";
                depth += 1;
                pos += 1;
            }
            b'}' | b']' => {
                if depth > 0 && native_arrays[depth - 1] {
                    native_entries += if nonempty[depth - 1] {
                        counts[depth - 1] + 1
                    } else {
                        0
                    };
                    ensure(
                        native_entries <= 1024,
                        FailureKind::EnvelopeRejected,
                        "native total entries",
                    )?;
                }
                depth = depth.saturating_sub(1);
                pos += 1;
            }
            b',' => {
                if depth > 0 {
                    counts[depth - 1] += 1;
                    ensure(
                        counts[depth - 1] < entry_limits[depth - 1],
                        FailureKind::EnvelopeRejected,
                        "JSON entry limit",
                    )?;
                }
                pos += 1;
            }
            b'"' => {
                if depth > 0 {
                    nonempty[depth - 1] = true;
                }
                let start = pos + 1;
                pos += 1;
                let mut escaped = false;
                while pos < bytes.len() {
                    let b = bytes[pos];
                    if !escaped && b == b'"' {
                        break;
                    }
                    if !escaped && b == b'\\' {
                        escaped = true
                    } else {
                        escaped = false
                    }
                    pos += 1;
                }
                ensure(
                    pos < bytes.len(),
                    FailureKind::EnvelopeRejected,
                    "unterminated JSON string",
                )?;
                let mut next = pos + 1;
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next += 1;
                }
                let is_key = bytes.get(next) == Some(&b':');
                ensure(
                    !is_key || !bytes[start..pos].contains(&b'\\'),
                    FailureKind::EnvelopeRejected,
                    "canonical unescaped field names required",
                )?;
                let cap = if !is_key && matches!(last_key, "native_hex") {
                    2 * MIB
                } else if !is_key
                    && matches!(
                        last_key,
                        "request_hex" | "response_hex" | "health_hex" | "capabilities_hex"
                    )
                {
                    2 * PROOF_LIMIT
                } else {
                    16 * 1024
                };
                ensure(
                    pos - start <= cap,
                    FailureKind::EnvelopeRejected,
                    "scalar limit",
                )?;
                if is_key {
                    last_key = std::str::from_utf8(&bytes[start..pos]).unwrap_or("");
                } else {
                    last_key = "";
                }
                pos += 1;
            }
            _ => {
                if bytes[pos].is_ascii_whitespace() || bytes[pos] == b':' {
                    pos += 1;
                } else {
                    let start = pos;
                    while pos < bytes.len()
                        && !matches!(bytes[pos], b',' | b'}' | b']')
                        && !bytes[pos].is_ascii_whitespace()
                    {
                        pos += 1;
                    }
                    ensure(
                        pos - start <= 16384,
                        FailureKind::EnvelopeRejected,
                        "scalar token limit",
                    )?;
                    if depth > 0 {
                        nonempty[depth - 1] = true;
                    }
                }
            }
        }
    }
    Ok(())
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T> {
    preflight(bytes, limit)?;
    serde_json::from_slice(bytes).map_err(|error| {
        let message = error.to_string();
        let kind = if message.contains("missing field `terminal`")
            || (message.contains("unknown variant") && message.contains("Suspended"))
        {
            FailureKind::UnknownSourceTerminal
        } else if message.contains("missing field `availability`") {
            FailureKind::PublicationMissingOrAfterAsOf
        } else if message.contains("missing field `revision`") {
            FailureKind::RevisionMissingOrConflict
        } else if message.contains("missing field `lifecycle`") {
            FailureKind::LifecycleCoverageMissing
        } else {
            FailureKind::EnvelopeRejected
        };
        failure(kind, "strict JSON decode")
    })
}
fn text(s: &str) -> bool {
    !s.is_empty() && s.len() <= 16 * 1024 && s.trim() == s
}

pub(crate) struct CompiledWindowProfile {
    pub id: &'static str,
    pub version: u32,
    pub provider: &'static str,
    pub scope: &'static str,
}
pub(crate) fn profile(id: &str) -> Result<CompiledWindowProfile> {
    #[cfg(test)]
    if id == TEST_PROFILE {
        return Ok(CompiledWindowProfile {
            id: "TEST_CODE_SYNTHETIC",
            version: 1,
            provider: "HithinkFinance",
            scope: TEST_SCOPE,
        });
    }
    let _ = id;
    Err(failure(
        FailureKind::UnsupportedProfileOrVersion,
        "no delivered production WG07 profile",
    ))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CalendarV1 {
    pub authority_id: String,
    pub authority_version: u32,
    pub authority_sha256: String,
    pub covered_from: NaiveDate,
    pub covered_to: NaiveDate,
    pub applicable_venue: Exchange,
    pub expected_sessions: Vec<NaiveDate>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestV1 {
    #[serde(deserialize_with = "strict_instrument")]
    pub instrument: InstrumentId,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub interval: String,
    pub adjustment: String,
    pub venue_timezone: String,
    pub as_of: DateTime<Utc>,
    pub source_profile_id: String,
    pub source_profile_version: u32,
    pub calendar: CalendarV1,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrozenRequest {
    pub request: RequestV1,
    pub invoked_at: DateTime<Utc>,
}
pub(crate) fn freeze(
    input: OrdinaryDailyChangeWindowRequest,
    now: DateTime<Utc>,
    p: &CompiledWindowProfile,
) -> Result<FrozenRequest> {
    ensure(
        input.instrument.asset_class() == AssetClass::Equity
            && input.instrument.code().len() == 6
            && input.instrument.code().bytes().all(|b| b.is_ascii_digit())
            && matches!(
                input.instrument.exchange(),
                Exchange::Shanghai | Exchange::Shenzhen
            ),
        FailureKind::InvalidRequest,
        "instrument",
    )?;
    let span = input.to.signed_duration_since(input.from).num_days();
    ensure(
        (0..366).contains(&span) && input.as_of <= now,
        FailureKind::InvalidRequest,
        "range/as_of",
    )?;
    ensure(
        input.instrument.exchange() == Exchange::Shanghai,
        FailureKind::CalendarUnavailableOrInapplicable,
        "profile does not prove SSE calendar applicability to Shenzhen",
    )?;
    let authority_hash = crate::calendar::verified_a_share_calendar_authority_hash(input.from)
        .map_err(|_| {
            failure(
                FailureKind::CalendarUnavailableOrInapplicable,
                "verified SSE authority",
            )
        })?;
    let mut sessions = Vec::new();
    let mut cursor = input.from;
    loop {
        if crate::calendar::verified_a_share_trading_day(cursor).map_err(|_| {
            failure(
                FailureKind::CalendarUnavailableOrInapplicable,
                "verified SSE date",
            )
        })? {
            ensure(
                sessions.len() < 260,
                FailureKind::InvalidRequest,
                "session count",
            )?;
            let close = cursor
                .and_hms_opt(7, 0, 0)
                .ok_or_else(|| failure(FailureKind::IncompleteSession, "close overflow"))?
                .and_utc();
            ensure(
                close <= input.as_of,
                FailureKind::IncompleteSession,
                "session incomplete at as_of",
            )?;
            sessions.push(cursor);
        }
        if cursor == input.to {
            break;
        }
        cursor = cursor
            .succ_opt()
            .ok_or_else(|| failure(FailureKind::InvalidRequest, "date overflow"))?;
    }
    ensure(
        !sessions.is_empty(),
        FailureKind::CalendarUnavailableOrInapplicable,
        "empty expected sessions",
    )?;
    let (id, version, covered_from, covered_to) =
        crate::calendar::ordinary_window_calendar_metadata().map_err(|_| {
            failure(
                FailureKind::CalendarUnavailableOrInapplicable,
                "calendar metadata",
            )
        })?;
    Ok(FrozenRequest {
        invoked_at: now,
        request: RequestV1 {
            instrument: input.instrument,
            from: input.from,
            to: input.to,
            interval: "Day".into(),
            adjustment: "Unadjusted".into(),
            venue_timezone: "Asia/Shanghai".into(),
            as_of: input.as_of,
            source_profile_id: p.id.into(),
            source_profile_version: p.version,
            calendar: CalendarV1 {
                authority_id: id.into(),
                authority_version: version,
                authority_sha256: authority_hash.into(),
                covered_from,
                covered_to,
                applicable_venue: Exchange::Shanghai,
                expected_sessions: sessions,
            },
        },
    })
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDecimal {
    pub value: String,
    pub unit: String,
    pub scale: u8,
}
impl SourceDecimal {
    pub(crate) fn integer(&self, unit: &str) -> Result<i128> {
        ensure(
            self.unit == unit && self.scale <= 8 && self.value.len() <= 64,
            FailureKind::InvalidBar,
            "decimal unit/scale",
        )?;
        let (a, b) = self.value.split_once('.').unwrap_or((&self.value, ""));
        ensure(
            !a.is_empty()
                && a.bytes().all(|b| b.is_ascii_digit())
                && b.bytes().all(|b| b.is_ascii_digit())
                && b.len() == usize::from(self.scale)
                && (a.len() == 1 || !a.starts_with('0'))
                && (self.scale > 0 || !self.value.contains('.')),
            FailureKind::InvalidBar,
            "lossless nonnegative decimal",
        )?;
        let mut n = 0i128;
        for d in a.bytes().chain(b.bytes()) {
            n = n
                .checked_mul(10)
                .and_then(|n| n.checked_add(i128::from(d - b'0')))
                .ok_or_else(|| failure(FailureKind::InvalidBar, "decimal overflow"))?;
        }
        n.checked_mul(10i128.pow(8 - u32::from(self.scale)))
            .ok_or_else(|| failure(FailureKind::InvalidBar, "decimal scale overflow"))
    }
    fn canonical(&self) -> Self {
        let mut s = self.clone();
        while s.scale > 0 && s.value.ends_with('0') {
            s.value.pop();
            s.scale -= 1;
        }
        if s.value.ends_with('.') {
            s.value.pop();
        }
        s
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrdinaryDailyBar {
    #[serde(deserialize_with = "strict_instrument")]
    pub instrument: InstrumentId,
    pub date: NaiveDate,
    pub open: SourceDecimal,
    pub high: SourceDecimal,
    pub low: SourceDecimal,
    pub close: SourceDecimal,
    pub volume: SourceDecimal,
    pub amount: SourceDecimal,
    pub adjustment: String,
}
impl OrdinaryDailyBar {
    pub(crate) fn validate(&self, instrument: &InstrumentId, date: NaiveDate) -> Result<()> {
        ensure(
            &self.instrument == instrument && self.date == date,
            FailureKind::NativeIdentityMissingOrMismatch,
            "bar identity/date",
        )?;
        ensure(
            self.adjustment == "Unadjusted",
            FailureKind::AdjustmentMissingOrMismatch,
            "bar adjustment",
        )?;
        let o = self.open.integer("CNY/share")?;
        let h = self.high.integer("CNY/share")?;
        let l = self.low.integer("CNY/share")?;
        let c = self.close.integer("CNY/share")?;
        self.volume.integer("share")?;
        self.amount.integer("CNY")?;
        ensure(
            l > 0 && l <= o && l <= c && h >= o && h >= c,
            FailureKind::InvalidBar,
            "OHLC",
        )
    }
    fn canonical(&self) -> Self {
        let mut b = self.clone();
        b.open = b.open.canonical();
        b.high = b.high.canonical();
        b.low = b.low.canonical();
        b.close = b.close.canonical();
        b.volume = b.volume.canonical();
        b.amount = b.amount.canonical();
        b
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceRef {
    pub native_response_id: String,
    pub fact_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeRequest {
    pub id: String,
    pub native_hex: String,
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeResponse {
    pub id: String,
    pub request_id: String,
    pub native_hex: String,
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", deny_unknown_fields)]
pub(crate) enum SourceAt {
    Present(DateTime<Utc>),
    NotProvided,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceV1 {
    pub provider: String,
    pub source: String,
    pub profile_id: String,
    pub profile_version: u32,
    pub batch_id: String,
    pub source_at: SourceAt,
    pub observed_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    pub request_id: String,
    pub issued_query_sha256: String,
    pub request: RequestV1,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityProof {
    #[serde(deserialize_with = "strict_instrument")]
    pub instrument: InstrumentId,
    pub evidence_refs: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdjustmentProof {
    pub mode: String,
    pub evidence_refs: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RangeTerminal {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub source_query_identity: String,
    pub exhaustive: String,
    pub pages: Vec<String>,
    pub last_page: String,
    pub snapshot_id: String,
    pub revision_id: String,
    pub evidence_refs: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Availability {
    pub id: String,
    pub available_at: DateTime<Utc>,
    pub time_precision: String,
    pub source_timezone: String,
    pub selected_as_of: DateTime<Utc>,
    pub record_ids: Vec<String>,
    pub evidence_refs: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(crate) enum Correction {
    Initial {
        evidence_refs: Vec<EvidenceRef>,
    },
    Replaces {
        prior_revision_refs: Vec<EvidenceRef>,
        evidence_refs: Vec<EvidenceRef>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Revision {
    pub id: String,
    pub snapshot_id: String,
    pub revision_id: String,
    pub correction: Correction,
    pub selection_as_of_proof_refs: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(crate) enum Terminal {
    Bar { bar: OrdinaryDailyBar },
    Suspended { reason: String, bridge: bool },
    NotYetListed { listing_date: NaiveDate },
    Delisted { delisting_date: NaiveDate },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Session {
    pub id: String,
    pub date: NaiveDate,
    pub terminal: Terminal,
    pub availability_ref: String,
    pub revision_ref: String,
    pub evidence_refs: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Action {
    pub effective_on: NaiveDate,
    pub record_on: Option<NaiveDate>,
    pub ex_on: Option<NaiveDate>,
    pub payable_on: Option<NaiveDate>,
    pub status: String,
    pub category: String,
    pub terms: String,
    pub evidence_refs: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lifecycle {
    #[serde(deserialize_with = "strict_instrument")]
    pub instrument: InstrumentId,
    pub provider: String,
    pub source: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub listing_date: NaiveDate,
    pub delisting_date: Option<NaiveDate>,
    pub action_coverage: String,
    pub actions: Vec<Action>,
    pub evidence_refs: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceError {
    #[serde(deserialize_with = "strict_instrument")]
    pub instrument: InstrumentId,
    pub date: Option<NaiveDate>,
    pub typed_source_reason: String,
    pub retryable: bool,
    pub evidence_refs: Vec<EvidenceRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceV1 {
    pub request_binding: Binding,
    pub source: SourceV1,
    pub native_requests: Vec<NativeRequest>,
    pub native_responses: Vec<NativeResponse>,
    pub identity_proof: IdentityProof,
    pub adjustment_proof: AdjustmentProof,
    pub range_terminal: RangeTerminal,
    pub availability: Vec<Availability>,
    pub revision: Vec<Revision>,
    pub sessions: Vec<Session>,
    pub lifecycle: Lifecycle,
    pub errors: Vec<SourceError>,
}

/// The synthetic protocol returns independent native facts. Outer wrappers must
/// match their reconstruction exactly; a hash/pointer/echo cannot replace them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeQuery {
    pub protocol: String,
    pub query_id: String,
    pub request: RequestV1,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PriorRevision {
    pub id: String,
    pub snapshot_id: String,
    pub revision_id: String,
    pub available_at: DateTime<Utc>,
    pub record_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeFacts {
    pub protocol: String,
    pub request_query_id: String,
    pub source: SourceV1,
    pub identity: IdentityProof,
    pub adjustment: AdjustmentProof,
    pub range: RangeTerminal,
    pub availability: Vec<Availability>,
    pub revision: Vec<Revision>,
    pub sessions: Vec<Session>,
    pub lifecycle: Lifecycle,
    pub errors: Vec<SourceError>,
    pub prior_revisions: Vec<PriorRevision>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaterialAction {
    pub effective_on: NaiveDate,
    pub record_on: Option<NaiveDate>,
    pub ex_on: Option<NaiveDate>,
    pub payable_on: Option<NaiveDate>,
    pub category: String,
    pub terms: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bridge {
    pub date: NaiveDate,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairFact {
    #[serde(deserialize_with = "strict_instrument")]
    pub instrument: InstrumentId,
    pub rule: String,
    pub previous: OrdinaryDailyBar,
    pub current: OrdinaryDailyBar,
    pub provider: String,
    pub source: String,
    pub listing_date: NaiveDate,
    pub delisting_date: Option<NaiveDate>,
    pub actions: Vec<MaterialAction>,
    pub bridges: Vec<Bridge>,
    pub adjustment: String,
}
pub(crate) struct Interpreted {
    pub evidence: EvidenceV1,
    pub pairs: Vec<PairFact>,
    pub bars: Vec<OrdinaryDailyBar>,
}

fn native_bytes(s: &str, total: &mut usize) -> Result<Vec<u8>> {
    ensure(
        s.len() % 2 == 0
            && s.len() / 2 <= MIB
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        FailureKind::EnvelopeRejected,
        "native bytes",
    )?;
    *total = total
        .checked_add(s.len() / 2)
        .filter(|n| *n <= 8 * MIB)
        .ok_or_else(|| failure(FailureKind::EnvelopeRejected, "native cumulative bytes"))?;
    hex::decode(s).map_err(|_| failure(FailureKind::EnvelopeRejected, "native hex"))
}
pub(crate) fn interpret(
    bytes: &[u8],
    frozen: &FrozenRequest,
    query_bytes: &[u8],
    p: &CompiledWindowProfile,
) -> Result<Interpreted> {
    let e: EvidenceV1 = decode(bytes, PROOF_LIMIT)?;
    let query =
        crate::grpc_client::external_pb::magic::market::v1::QueryRequest::decode(query_bytes)
            .map_err(|_| failure(FailureKind::RequestBindingMismatch, "issued protobuf"))?;
    let rid = query
        .context
        .as_ref()
        .map(|c| c.request_id.as_str())
        .unwrap_or("");
    ensure(
        e.request_binding.request_id == rid
            && e.request_binding.issued_query_sha256 == sha(query_bytes)
            && e.request_binding.request == frozen.request,
        FailureKind::RequestBindingMismatch,
        "exact issued request",
    )?;
    ensure(
        e.source.profile_id == p.id
            && e.source.profile_version == p.version
            && e.source.provider == p.provider
            && text(&e.source.source)
            && text(&e.source.batch_id),
        FailureKind::UnsupportedProfileOrVersion,
        "source profile",
    )?;
    ensure(
        e.native_requests.len() + e.native_responses.len() <= 1024
            && !e.native_requests.is_empty()
            && !e.native_responses.is_empty(),
        FailureKind::EnvelopeRejected,
        "native entries",
    )?;
    // Only this grammar is implemented, and only its cfg(test) dispatch can be reached.
    ensure(
        p.id == "TEST_CODE_SYNTHETIC",
        FailureKind::UnsupportedProfileOrVersion,
        "native protocol",
    )?;
    let mut total = 0;
    let mut requests = BTreeMap::new();
    for r in &e.native_requests {
        let bytes = native_bytes(&r.native_hex, &mut total)?;
        ensure(
            text(&r.id) && sha(&bytes) == r.sha256,
            FailureKind::EnvelopeRejected,
            "native request hash/id",
        )?;
        let q: NativeQuery = decode(&bytes, MIB)?;
        ensure(
            q.protocol == "TEST_CODE_NATIVE_WINDOW_V1"
                && q.request == frozen.request
                && q.query_id == e.range_terminal.source_query_identity,
            FailureKind::RequestBindingMismatch,
            "native query range",
        )?;
        ensure(
            requests.insert(r.id.clone(), q).is_none(),
            FailureKind::EnvelopeRejected,
            "duplicate native request",
        )?;
    }
    let mut responses = BTreeMap::new();
    let mut used = BTreeSet::new();
    for r in &e.native_responses {
        let bytes = native_bytes(&r.native_hex, &mut total)?;
        ensure(
            text(&r.id) && sha(&bytes) == r.sha256,
            FailureKind::EnvelopeRejected,
            "native response hash/id",
        )?;
        let q = requests.get(&r.request_id).ok_or_else(|| {
            failure(
                FailureKind::RequestBindingMismatch,
                "native request reference",
            )
        })?;
        let n: NativeFacts = decode(&bytes, MIB)?;
        ensure(
            n.protocol == "TEST_CODE_NATIVE_WINDOW_V1" && n.request_query_id == q.query_id,
            FailureKind::RequestBindingMismatch,
            "native query identity",
        )?;
        ensure(
            n.source == e.source
                && n.identity == e.identity_proof
                && n.adjustment == e.adjustment_proof
                && n.range == e.range_terminal
                && n.availability == e.availability
                && n.revision == e.revision
                && n.sessions == e.sessions
                && n.lifecycle == e.lifecycle
                && n.errors == e.errors,
            FailureKind::EnvelopeRejected,
            "native facts differ from wrapper",
        )?;
        used.insert(r.request_id.clone());
        ensure(
            responses.insert(r.id.clone(), n).is_none(),
            FailureKind::EnvelopeRejected,
            "duplicate native response",
        )?;
    }
    ensure(
        e.range_terminal.pages.iter().collect::<BTreeSet<_>>()
            == responses.keys().collect::<BTreeSet<_>>(),
        FailureKind::IncompleteRange,
        "native page manifest closure",
    )?;
    ensure(
        used.len() == requests.len(),
        FailureKind::RequestBindingMismatch,
        "unused native request",
    )?;
    // Every reference must name an interpreted typed fact, including predecessor revisions.
    for n in responses.values() {
        let mut fact_ids: BTreeSet<&str> = [
            "identity",
            "adjustment",
            "range",
            "listing",
            "actions",
            "initial",
        ]
        .into_iter()
        .collect();
        for id in n
            .sessions
            .iter()
            .map(|s| s.id.as_str())
            .chain(n.availability.iter().map(|a| a.id.as_str()))
            .chain(n.revision.iter().map(|v| v.id.as_str()))
            .chain(n.prior_revisions.iter().map(|p| p.id.as_str()))
        {
            ensure(
                text(id) && fact_ids.insert(id),
                FailureKind::EnvelopeRejected,
                "native fact IDs must be unique",
            )?;
        }

        let mut prior_ids = BTreeSet::new();
        ensure(
            n.prior_revisions.len() <= 1024
                && n.prior_revisions
                    .iter()
                    .all(|p| text(&p.id) && prior_ids.insert(&p.id)),
            FailureKind::CorrectionProofMissing,
            "prior manifest uniqueness",
        )?;
    }
    let refs = |refs: &[EvidenceRef], expected: Option<&str>| -> Result<()> {
        ensure(
            !refs.is_empty() && refs.len() <= 1024,
            FailureKind::EnvelopeRejected,
            "missing evidence refs",
        )?;
        let mut unique = BTreeSet::new();
        for r in refs {
            let n = responses.get(&r.native_response_id).ok_or_else(|| {
                failure(FailureKind::EnvelopeRejected, "unresolved native response")
            })?;
            let known = matches!(
                r.fact_id.as_str(),
                "identity" | "adjustment" | "range" | "listing" | "actions" | "initial"
            ) || n.sessions.iter().any(|s| s.id == r.fact_id)
                || n.availability.iter().any(|a| a.id == r.fact_id)
                || n.revision.iter().any(|v| v.id == r.fact_id)
                || n.prior_revisions.iter().any(|p| p.id == r.fact_id);
            ensure(
                known && expected.is_none_or(|x| r.fact_id == x) && unique.insert(r),
                FailureKind::EnvelopeRejected,
                "unresolved/duplicate native fact",
            )?;
        }
        Ok(())
    };
    ensure(
        e.identity_proof.instrument == frozen.request.instrument,
        FailureKind::NativeIdentityMissingOrMismatch,
        "native returned instrument",
    )?;
    refs(&e.identity_proof.evidence_refs, Some("identity")).map_err(|_| {
        failure(
            FailureKind::NativeIdentityMissingOrMismatch,
            "native identity missing",
        )
    })?;
    ensure(
        e.adjustment_proof.mode == "Unadjusted",
        FailureKind::AdjustmentMissingOrMismatch,
        "native adjustment",
    )?;
    refs(&e.adjustment_proof.evidence_refs, Some("adjustment")).map_err(|_| {
        failure(
            FailureKind::AdjustmentMissingOrMismatch,
            "adjustment evidence",
        )
    })?;
    let r = &e.range_terminal;
    ensure(
        r.from == frozen.request.from && r.to == frozen.request.to,
        FailureKind::IncompleteRange,
        "native exact range",
    )?;
    ensure(
        r.exhaustive == "Exhausted"
            && !r.pages.is_empty()
            && r.pages.last() == Some(&r.last_page)
            && r.pages.iter().collect::<BTreeSet<_>>().len() == r.pages.len()
            && r.pages.iter().all(|s| text(s)),
        FailureKind::UnknownSourceTerminal,
        "range exhaustion/pages",
    )?;
    refs(&r.evidence_refs, Some("range"))?;
    ensure(
        text(&r.snapshot_id) && text(&r.revision_id),
        FailureKind::RevisionMissingOrConflict,
        "range version",
    )?;
    let mut availability = BTreeMap::new();
    let mut selected = BTreeSet::new();
    for a in &e.availability {
        ensure(
            a.available_at <= frozen.request.as_of
                && a.selected_as_of == frozen.request.as_of
                && a.time_precision == "Second"
                && a.source_timezone == "UTC"
                && !a.record_ids.is_empty(),
            FailureKind::PublicationMissingOrAfterAsOf,
            "manifest availability",
        )?;
        refs(&a.evidence_refs, Some(&a.id))?;
        ensure(
            text(&a.id) && availability.insert(a.id.clone(), a).is_none(),
            FailureKind::RevisionMissingOrConflict,
            "duplicate availability",
        )?;
        for id in &a.record_ids {
            ensure(
                selected.insert(id.clone()),
                FailureKind::RevisionMissingOrConflict,
                "record selected twice",
            )?;
        }
    }
    let mut revisions = BTreeMap::new();
    for v in &e.revision {
        ensure(
            v.snapshot_id == r.snapshot_id && v.revision_id == r.revision_id && text(&v.id),
            FailureKind::RevisionMissingOrConflict,
            "version manifest",
        )?;
        refs(&v.selection_as_of_proof_refs, Some(&v.id))?;
        match &v.correction {
            Correction::Initial { evidence_refs } => refs(evidence_refs, Some("initial"))
                .map_err(|_| failure(FailureKind::CorrectionProofMissing, "initial proof"))?,
            Correction::Replaces {
                prior_revision_refs,
                evidence_refs,
            } => {
                refs(evidence_refs, Some(&v.id)).map_err(|_| {
                    failure(FailureKind::CorrectionProofMissing, "replacement proof")
                })?;
                refs(prior_revision_refs, None).map_err(|_| {
                    failure(FailureKind::CorrectionProofMissing, "predecessor proof")
                })?;
                for r in prior_revision_refs {
                    ensure(
                        responses[&r.native_response_id]
                            .prior_revisions
                            .iter()
                            .any(|prior| {
                                prior.id == r.fact_id
                                    && text(&prior.snapshot_id)
                                    && text(&prior.revision_id)
                                    && prior.revision_id != v.revision_id
                                    && prior.available_at <= frozen.request.as_of
                                    && !prior.record_ids.is_empty()
                                    && prior.record_ids.iter().all(|id| selected.contains(id))
                                    && e.availability
                                        .iter()
                                        .all(|a| prior.available_at <= a.available_at)
                            }),
                        FailureKind::CorrectionProofMissing,
                        "predecessor version",
                    )?;
                }
            }
        }
        ensure(
            revisions.insert(v.id.clone(), v).is_none(),
            FailureKind::RevisionMissingOrConflict,
            "duplicate revision",
        )?;
    }
    let lc = &e.lifecycle;
    ensure(
        lc.instrument == frozen.request.instrument
            && lc.provider == e.source.provider
            && lc.source == e.source.source
            && lc.from == frozen.request.from
            && lc.to == frozen.request.to
            && matches!(lc.action_coverage.as_str(), "Complete" | "None")
            && (lc.action_coverage != "None" || lc.actions.is_empty())
            && lc.actions.len() <= 1024
            && lc.delisting_date.is_none_or(|date| date >= lc.listing_date),
        FailureKind::LifecycleCoverageMissing,
        "same-source lifecycle coverage",
    )?;
    refs(&lc.evidence_refs, None)
        .map_err(|_| failure(FailureKind::LifecycleCoverageMissing, "lifecycle refs"))?;
    ensure(
        lc.evidence_refs.iter().any(|r| r.fact_id == "listing")
            && lc.evidence_refs.iter().any(|r| r.fact_id == "actions"),
        FailureKind::LifecycleCoverageMissing,
        "listing/actions proof",
    )?;
    let mut actions = BTreeSet::new();
    for a in &lc.actions {
        ensure(
            [Some(a.effective_on), a.record_on, a.ex_on, a.payable_on]
                .into_iter()
                .flatten()
                .any(|d| d >= lc.from && d <= lc.to)
                && a.status == "Implemented"
                && serde_json::from_value::<crate::market_domain::CorporateActionCategory>(
                    serde_json::Value::String(a.category.clone()),
                )
                .is_ok()
                && text(&a.category)
                && text(&a.terms)
                && actions.insert((a.effective_on, a.category.clone(), a.terms.clone())),
            FailureKind::LifecycleCoverageMissing,
            "action facts",
        )?;
        refs(&a.evidence_refs, Some("actions"))?;
    }
    let expected = &frozen.request.calendar.expected_sessions;
    ensure(
        e.sessions.len() == expected.len() && e.sessions.len() <= 260,
        FailureKind::IncompleteRange,
        "session cardinality",
    )?;
    let mut failures = Vec::new();
    let mut seen = BTreeSet::new();
    let mut bars = Vec::new();
    let mut pairs = Vec::new();
    let mut previous: Option<OrdinaryDailyBar> = None;
    let mut bridges = Vec::new();
    for (s, date) in e.sessions.iter().zip(expected) {
        let result = (|| -> Result<()> {
            ensure(
                s.date == *date && text(&s.id) && seen.insert(s.id.clone()),
                FailureKind::DuplicateOrUnexpectedSession,
                "ordered exact session",
            )?;
            refs(&s.evidence_refs, Some(&s.id))?;
            let a = availability.get(&s.availability_ref).ok_or_else(|| {
                failure(
                    FailureKind::PublicationMissingOrAfterAsOf,
                    "missing manifest",
                )
            })?;
            ensure(
                a.record_ids.contains(&s.id),
                FailureKind::PublicationMissingOrAfterAsOf,
                "manifest record binding",
            )?;
            ensure(
                revisions.contains_key(&s.revision_ref),
                FailureKind::RevisionMissingOrConflict,
                "session revision",
            )?;
            match &s.terminal {
                Terminal::Bar { bar } => {
                    bar.validate(&frozen.request.instrument, *date)?;
                    ensure(
                        *date >= lc.listing_date && lc.delisting_date.is_none_or(|d| *date < d),
                        FailureKind::LifecycleCoverageMissing,
                        "bar outside listing interval",
                    )?;
                    if let Some(prev) = &previous {
                        if anomalous(prev, bar)? {
                            pairs.push(PairFact {
                                instrument: frozen.request.instrument.clone(),
                                rule: "br171-close-change-v1".into(),
                                previous: prev.canonical(),
                                current: bar.canonical(),
                                provider: e.source.provider.clone(),
                                source: e.source.source.clone(),
                                listing_date: lc.listing_date,
                                delisting_date: lc.delisting_date,
                                actions: lc
                                    .actions
                                    .iter()
                                    .filter(|a| {
                                        [Some(a.effective_on), a.record_on, a.ex_on, a.payable_on]
                                            .into_iter()
                                            .flatten()
                                            .any(|d| d > prev.date && d <= bar.date)
                                    })
                                    .map(|a| MaterialAction {
                                        effective_on: a.effective_on,
                                        record_on: a.record_on,
                                        ex_on: a.ex_on,
                                        payable_on: a.payable_on,
                                        category: a.category.clone(),
                                        terms: a.terms.clone(),
                                    })
                                    .collect(),
                                bridges: bridges.clone(),
                                adjustment: "Unadjusted".into(),
                            });
                        }
                    }
                    bars.push(bar.canonical());
                    previous = Some(bar.clone());
                    bridges.clear();
                }
                Terminal::Suspended { reason, bridge } => {
                    ensure(
                        text(reason)
                            && *date >= lc.listing_date
                            && lc.delisting_date.is_none_or(|d| *date < d),
                        FailureKind::LifecycleCoverageMissing,
                        "suspension interval",
                    )?;
                    if *bridge {
                        bridges.push(Bridge {
                            date: *date,
                            reason: reason.clone(),
                        });
                    } else {
                        ensure(
                            previous.is_none(),
                            FailureKind::UnknownSourceTerminal,
                            "suspension bridge not proven",
                        )?;
                        bridges.clear();
                    }
                }
                Terminal::NotYetListed { listing_date } => {
                    ensure(
                        *listing_date == lc.listing_date && *date < *listing_date,
                        FailureKind::LifecycleCoverageMissing,
                        "not listed proof",
                    )?;
                    previous = None;
                    bridges.clear();
                }
                Terminal::Delisted { delisting_date } => {
                    ensure(
                        Some(*delisting_date) == lc.delisting_date && *date >= *delisting_date,
                        FailureKind::LifecycleCoverageMissing,
                        "delisted proof",
                    )?;
                    previous = None;
                    bridges.clear();
                }
            }
            Ok(())
        })();
        if let Err(mut err) = result {
            for item in &mut err.failures {
                item.instrument = Some(frozen.request.instrument.clone());
                item.date = Some(*date);
            }
            failures.extend(err.failures);
            previous = None;
            bridges.clear();
        }
    }
    seen.insert("listing".into());
    seen.insert("actions".into());
    if selected != seen {
        failures.extend(
            failure(
                FailureKind::PublicationMissingOrAfterAsOf,
                "manifest selection not exact sessions",
            )
            .failures,
        );
    }
    for err in &e.errors {
        if refs(&err.evidence_refs, None).is_err() || err.instrument != frozen.request.instrument {
            failures.extend(
                failure(
                    FailureKind::SourceRejected,
                    "source error reference/identity",
                )
                .failures,
            );
        }
        let mut f = failure(FailureKind::SourceRejected, &err.typed_source_reason);
        f.failures[0].retryable = err.retryable;
        f.failures[0].instrument = Some(err.instrument.clone());
        f.failures[0].date = err.date;
        f.failures[0].evidence_refs = err
            .evidence_refs
            .iter()
            .map(|r| format!("{}:{}", r.native_response_id, r.fact_id))
            .collect();
        failures.extend(f.failures);
    }
    if !failures.is_empty() {
        return Err(DiscoveryFailure::from_items(failures));
    }
    for pair in &mut pairs {
        pair.actions.sort_by(|a, b| {
            (&a.effective_on, &a.category, &a.terms).cmp(&(&b.effective_on, &b.category, &b.terms))
        });
    }
    Ok(Interpreted {
        evidence: e,
        pairs,
        bars,
    })
}
use prost::Message as _;
pub(crate) fn anomalous(a: &OrdinaryDailyBar, b: &OrdinaryDailyBar) -> Result<bool> {
    let a = a.close.integer("CNY/share")?;
    let b = b.close.integer("CNY/share")?;
    let delta = b
        .checked_sub(a)
        .and_then(i128::checked_abs)
        .and_then(|n| n.checked_mul(100))
        .ok_or_else(|| failure(FailureKind::InvalidBar, "change overflow"))?;
    Ok(delta
        > a.checked_mul(20)
            .ok_or_else(|| failure(FailureKind::InvalidBar, "threshold overflow"))?)
}
pub(crate) fn percent(pair: &PairFact) -> Result<String> {
    let a = pair.previous.close.integer("CNY/share")?;
    let b = pair.current.close.integer("CNY/share")?;
    ensure(a > 0, FailureKind::InvalidBar, "previous close denominator")?;
    let scaled = b
        .checked_sub(a)
        .and_then(|n| n.checked_mul(100_000_000_000_000))
        .ok_or_else(|| failure(FailureKind::InvalidBar, "percentage overflow"))?
        / a;
    let sign = if scaled < 0 { "-" } else { "" };
    let n = scaled
        .checked_abs()
        .ok_or_else(|| failure(FailureKind::InvalidBar, "percentage absolute overflow"))?;
    let mut text = format!(
        "{sign}{}.{:012}",
        n / 1_000_000_000_000i128,
        n % 1_000_000_000_000i128
    );
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    Ok(text)
}

fn strict_instrument<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<InstrumentId, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Strict {
        exchange: Exchange,
        code: String,
        asset_class: AssetClass,
    }
    let s = Strict::deserialize(d)?;
    if s.code.len() != 6
        || !s.code.bytes().all(|b| b.is_ascii_digit())
        || s.asset_class != AssetClass::Equity
        || !matches!(s.exchange, Exchange::Shanghai | Exchange::Shenzhen)
    {
        return Err(serde::de::Error::custom("canonical WG07 instrument"));
    }
    InstrumentId::new(s.exchange, s.code, s.asset_class).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod bound_tests {
    use super::*;
    #[test]
    fn wg07_native_single_and_cumulative_boundaries_are_checked_before_decode() {
        let mut total = 0;
        assert_eq!(
            native_bytes(&"00".repeat(MIB), &mut total).unwrap().len(),
            MIB
        );
        assert_eq!(total, MIB);
        assert!(native_bytes(&"00".repeat(MIB + 1), &mut total).is_err());
        assert_eq!(total, MIB);
        let mut total = 8 * MIB - 1;
        assert_eq!(native_bytes("00", &mut total).unwrap(), vec![0]);
        assert_eq!(total, 8 * MIB);
        assert!(native_bytes("00", &mut total).is_err());
        assert_eq!(total, 8 * MIB);
    }
    #[test]
    fn wg07_bounded_writer_request_event_and_receipt_exact_limits() {
        for limit in [MIB, PROOF_LIMIT, PREPARE_LIMIT] {
            let value = "x".repeat(limit - 2);
            assert_eq!(encode(&value, limit).unwrap().len(), limit);
            let value = "x".repeat(limit - 1);
            assert!(encode(&value, limit).is_err());
        }
    }
    #[test]
    fn wg07_escaped_collection_keys_cannot_bypass_preflight_bounds() {
        let raw = br#"{"sess\u0069ons":[]}"#;
        assert!(preflight(raw, PROOF_LIMIT).is_err());
        assert!(preflight(&vec![b'1'; 16385], PROOF_LIMIT).is_err());
    }
}
