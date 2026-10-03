//! Bounded observation of the actual legacy pushed-row scope.
//! This is source preparation for F2, not a persisted investment decision.
use crate::database::global_schema_v1::candidate_v7::VerifiedCatalog7;
use crate::database::global_schema_v1::investment_v8::VerifiedCatalog8;
use crate::database::global_schema_v1::paper_v6::{PaperCatalog6Error, VerifiedCatalog6};
use crate::database::DatabaseConnectionAuthority;
use chrono::{DateTime, Datelike, Duration, FixedOffset, SecondsFormat, Utc};
use diesel::sql_types::{BigInt, Binary, Double, Nullable, Text};
use diesel::{QueryableByName, RunQueryDsl, SqliteConnection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DOMAIN: &str = "stock_analysis.m4.bounded_pushed_row_scope_observation.v1";
const POLICY: &str = "intraday-unconsumed-pushed-row-top50-v1";
const TOP: &str = "SELECT id,push_time,push_kind,code,name,push_price,metric_json,source,consumed_at,consumed_by,outcome FROM main.pushed_stocks WHERE consumed_at IS NULL AND push_time < ? AND push_time > ? ORDER BY push_time COLLATE BINARY DESC,id DESC LIMIT 50";
const FIELD_BYTES: i64 = 16 * 1024;
const METRIC_BYTES: i64 = 64 * 1024;
const ROW_BYTES: i64 = 128 * 1024;
const SCOPE_BYTES: i64 = 1024 * 1024;
const CANONICAL_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CandidateScopeError {
    #[error(transparent)]
    Catalog(#[from] PaperCatalog6Error),
    #[error("candidate scope SQLite read failed")]
    Sql(#[from] diesel::result::Error),
    #[error("candidate scope requires UTF-8 main encoding")]
    Encoding,
    #[error("candidate scope TEXT contains invalid UTF-8")]
    InvalidText,
    #[error("candidate scope original database authority differs")]
    SourceAuthority,
    #[error("candidate scope storage type or byte bound differs")]
    Bounds,
    #[error("candidate scope cutoff is outside representable Shanghai time")]
    Cutoff,
    #[error("candidate scope canonical encoding failed")]
    Canonical,
    #[error("candidate scope changed after its initial capture")]
    Changed,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CandidateScopeCaptureId(String);
impl CandidateScopeCaptureId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Immutable negative observation; no Deserialize or approving constructor.
pub(crate) struct CapturedPushedCandidateScope {
    authority: DatabaseConnectionAuthority,
    cutoff: DateTime<Utc>,
    id: CandidateScopeCaptureId,
    canonical: Vec<u8>,
}
impl CapturedPushedCandidateScope {
    pub(crate) fn id(&self) -> &CandidateScopeCaptureId {
        &self.id
    }
    pub(crate) fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }
    /// Use the original cutoff in the mandatory consumer tail, including the
    /// separate committed reader. This does not create a durable occurrence.
    pub(crate) fn verify_unchanged(
        &self,
        conn: &mut SqliteConnection,
        proof: &VerifiedCatalog6<'_>,
    ) -> Result<(), CandidateScopeError> {
        if proof.connection_authority() != &self.authority {
            return Err(CandidateScopeError::SourceAuthority);
        }
        let current = capture_at(conn, proof, self.cutoff)?;
        if current.canonical == self.canonical {
            Ok(())
        } else {
            Err(CandidateScopeError::Changed)
        }
    }
}

/// The closed producer chooses its own clock once. Production Catalog6 still
/// refuses before checkout; this component cannot authorize that qualification.
pub(crate) fn capture_pushed_candidate_scope(
    conn: &mut SqliteConnection,
    proof: &VerifiedCatalog6<'_>,
) -> Result<CapturedPushedCandidateScope, CandidateScopeError> {
    capture_at(conn, proof, Utc::now())
}

#[cfg(test)]
pub(crate) fn capture_pushed_candidate_scope_at_for_test(
    conn: &mut SqliteConnection,
    proof: &VerifiedCatalog6<'_>,
    cutoff: DateTime<Utc>,
) -> Result<CapturedPushedCandidateScope, CandidateScopeError> {
    capture_at(conn, proof, cutoff)
}

/// Only a live, exact7 loan can create this capture; stored bytes never can.
pub(crate) fn capture_catalog7_at(
    conn: &mut SqliteConnection,
    proof: &VerifiedCatalog7<'_>,
    cutoff: DateTime<Utc>,
) -> Result<CapturedPushedCandidateScope, CandidateScopeError> {
    proof.validate_on(conn)?;
    capture_validated(conn, proof.connection_authority(), cutoff)
}
impl CapturedPushedCandidateScope {
    pub(crate) fn verify_catalog7_unchanged(
        &self,
        conn: &mut SqliteConnection,
        proof: &VerifiedCatalog7<'_>,
    ) -> Result<(), CandidateScopeError> {
        if proof.connection_authority() != &self.authority {
            return Err(CandidateScopeError::SourceAuthority);
        }
        let current = capture_catalog7_at(conn, proof, self.cutoff)?;
        if current.canonical == self.canonical {
            Ok(())
        } else {
            Err(CandidateScopeError::Changed)
        }
    }
}

/// Only a live, exact8 loan can create this capture; stored bytes never can.
pub(crate) fn capture_catalog8_at(
    conn: &mut SqliteConnection,
    proof: &VerifiedCatalog8<'_>,
    cutoff: DateTime<Utc>,
) -> Result<CapturedPushedCandidateScope, CandidateScopeError> {
    proof.validate_on(conn)?;
    capture_validated(conn, proof.connection_authority(), cutoff)
}
impl CapturedPushedCandidateScope {
    pub(crate) fn verify_catalog8_unchanged(
        &self,
        conn: &mut SqliteConnection,
        proof: &VerifiedCatalog8<'_>,
    ) -> Result<(), CandidateScopeError> {
        if proof.connection_authority() != &self.authority {
            return Err(CandidateScopeError::SourceAuthority);
        }
        let current = capture_catalog8_at(conn, proof, self.cutoff)?;
        if current.canonical == self.canonical {
            Ok(())
        } else {
            Err(CandidateScopeError::Changed)
        }
    }
}

#[derive(QueryableByName)]
struct MainEncoding {
    #[diesel(sql_type = Text)]
    encoding: String,
}
#[derive(QueryableByName)]
struct Extent {
    #[diesel(sql_type = BigInt)]
    count: i64,
    #[diesel(sql_type = BigInt)]
    bytes: i64,
    #[diesel(sql_type = BigInt)]
    largest_row: i64,
    #[diesel(sql_type = BigInt)]
    bad: i64,
}
#[derive(QueryableByName)]
struct RawPushRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Binary)]
    push_time: Vec<u8>,
    #[diesel(sql_type = Binary)]
    push_kind: Vec<u8>,
    #[diesel(sql_type = Binary)]
    code: Vec<u8>,
    #[diesel(sql_type = Binary)]
    name: Vec<u8>,
    #[diesel(sql_type = Double)]
    push_price: f64,
    #[diesel(sql_type = Binary)]
    metric_json: Vec<u8>,
    #[diesel(sql_type = Binary)]
    source: Vec<u8>,
    #[diesel(sql_type = Nullable<Binary>)]
    consumed_at: Option<Vec<u8>>,
    #[diesel(sql_type = Nullable<Binary>)]
    consumed_by: Option<Vec<u8>>,
    #[diesel(sql_type = Nullable<Binary>)]
    outcome: Option<Vec<u8>>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FrozenPushRow {
    pub(super) id: i64,
    push_time: String,
    push_kind: String,
    code: String,
    name: String,
    push_price_real_bits: u64,
    metric_json: String,
    source: String,
    consumed_at: Option<String>,
    consumed_by: Option<String>,
    outcome: Option<String>,
}
fn checked_text(bytes: Vec<u8>) -> Result<String, CandidateScopeError> {
    String::from_utf8(bytes).map_err(|_| CandidateScopeError::InvalidText)
}
impl TryFrom<RawPushRow> for FrozenPushRow {
    type Error = CandidateScopeError;
    fn try_from(row: RawPushRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            push_time: checked_text(row.push_time)?,
            push_kind: checked_text(row.push_kind)?,
            code: checked_text(row.code)?,
            name: checked_text(row.name)?,
            push_price_real_bits: row.push_price.to_bits(),
            metric_json: checked_text(row.metric_json)?,
            source: checked_text(row.source)?,
            consumed_at: row.consumed_at.map(checked_text).transpose()?,
            consumed_by: row.consumed_by.map(checked_text).transpose()?,
            outcome: row.outcome.map(checked_text).transpose()?,
        })
    }
}
#[derive(Serialize)]
struct FactNotRequested {
    requirement: &'static str,
    state: &'static str,
    reason: &'static str,
}
#[derive(Serialize)]
struct DeniedRow {
    row: FrozenPushRow,
    identity: &'static str,
    facts: [FactNotRequested; 3],
    disposition: &'static str,
    risk_inventory: &'static str,
    risk_evaluation: &'static str,
    cost_model: &'static str,
    liquidity_model: &'static str,
    budget_allocation: &'static str,
    manual_approval: &'static str,
}
impl From<FrozenPushRow> for DeniedRow {
    fn from(row: FrozenPushRow) -> Self {
        Self {
            row,
            identity: "unqualified_missing_source_venue_asset_identity",
            facts: ["lifecycle", "price_regime", "suspension"].map(|requirement| {
                FactNotRequested {
                    requirement,
                    state: "unqualified",
                    reason: "not_requested_identity_unavailable",
                }
            }),
            disposition: "identity_unqualified",
            risk_inventory: "unavailable_formal_rule_inventory_not_delivered",
            risk_evaluation: "not_evaluated_identity_unqualified",
            cost_model: "unavailable_formal_cost_model_not_delivered",
            liquidity_model: "unavailable_formal_liquidity_model_not_delivered",
            budget_allocation: "unavailable_explicit_budget_allocation_not_delivered",
            manual_approval: "unavailable_formal_approval_not_delivered",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CalendarObservation {
    Covered {
        contract: String,
        authority_sha256: String,
        open: bool,
    },
    Unavailable {
        reason: String,
    },
}
#[derive(Serialize)]
struct Canonical<'a> {
    domain: &'static str,
    policy: &'static str,
    scope_query_sha256: String,
    predicate_clock: &'static str,
    scope: &'static str,
    source_layout: &'static str,
    cutoff_utc: String,
    lower_exclusive_shanghai: &'a str,
    upper_exclusive_shanghai: &'a str,
    shanghai_civil_date: String,
    calendar: CalendarObservation,
    scope_state: &'static str,
    candidates: Vec<DeniedRow>,
}

fn capture_at(
    conn: &mut SqliteConnection,
    proof: &VerifiedCatalog6<'_>,
    cutoff: DateTime<Utc>,
) -> Result<CapturedPushedCandidateScope, CandidateScopeError> {
    proof.validate_on(conn)?; // Wrong callback-local instance rejects with zero SQL.
    capture_validated(conn, proof.connection_authority(), cutoff)
}
fn capture_validated(
    conn: &mut SqliteConnection,
    authority: &DatabaseConnectionAuthority,
    cutoff: DateTime<Utc>,
) -> Result<CapturedPushedCandidateScope, CandidateScopeError> {
    let encoding = diesel::sql_query("PRAGMA main.encoding").get_result::<MainEncoding>(conn)?;
    if encoding.encoding != "UTF-8" {
        return Err(CandidateScopeError::Encoding);
    }
    let upper_time = cutoff
        .checked_add_signed(Duration::hours(8))
        .ok_or(CandidateScopeError::Cutoff)?;
    if !(1..=9999).contains(&upper_time.year()) {
        return Err(CandidateScopeError::Cutoff);
    }
    let lower_time = upper_time
        .checked_sub_signed(Duration::hours(1))
        .ok_or(CandidateScopeError::Cutoff)?;
    // Format UTC-shaped civil values explicitly, independent of host Local TZ.
    // The millisecond text predicate is a versioned legacy-row policy, not a
    // qualification of source publication time.
    let upper = upper_time.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let lower = lower_time.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let text_fields = [
        ("push_time", false, FIELD_BYTES),
        ("push_kind", false, FIELD_BYTES),
        ("code", false, FIELD_BYTES),
        ("name", false, FIELD_BYTES),
        ("metric_json", false, METRIC_BYTES),
        ("source", false, FIELD_BYTES),
        ("consumed_at", true, FIELD_BYTES),
        ("consumed_by", true, FIELD_BYTES),
        ("outcome", true, FIELD_BYTES),
    ];
    let sizes = text_fields
        .iter()
        .map(|(field, _, _)| {
            format!(
                "CASE WHEN typeof({field})='null' THEN 1 ELSE 9+length(CAST({field} AS BLOB)) END"
            )
        })
        .collect::<Vec<_>>()
        .join("+");
    let sizes = format!("18+{sizes}");
    let bad = text_fields
        .iter()
        .map(|(field, nullable, limit)| {
            let types = if *nullable { "'text','null'" } else { "'text'" };
            format!("typeof({field}) NOT IN ({types}) OR COALESCE(length(CAST({field} AS BLOB)),0)>{limit}")
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    let preflight = format!("SELECT COUNT(*) AS count,COALESCE(SUM({sizes}),0) AS bytes,COALESCE(MAX({sizes}),0) AS largest_row,COALESCE(SUM(CASE WHEN typeof(id)!='integer' OR typeof(push_price)!='real' OR {bad} THEN 1 ELSE 0 END),0) AS bad FROM ({TOP})");
    let extent = diesel::sql_query(preflight)
        .bind::<Text, _>(&upper)
        .bind::<Text, _>(&lower)
        .get_result::<Extent>(conn)?;
    if !(0..=50).contains(&extent.count)
        || !(0..=SCOPE_BYTES).contains(&extent.bytes)
        || !(0..=ROW_BYTES).contains(&extent.largest_row)
        || extent.bad != 0
    {
        return Err(CandidateScopeError::Bounds);
    }
    // The same transaction freezes membership between preflight and loading.
    // SQLite TEXT storage does not imply valid UTF-8. Read bounded raw bytes
    // before constructing Rust strings; Diesel's Text decoder is unsuitable
    // for damaged TEXT. CAST preserves NULL and the original UTF-8 byte image.
    let byte_select = text_fields
        .iter()
        .map(|(field, _, _)| format!("CAST({field} AS BLOB) AS {field}"))
        .collect::<Vec<_>>()
        .join(",");
    let load = format!("SELECT id,push_price,{byte_select} FROM ({TOP})");
    let rows = diesel::sql_query(load)
        .bind::<Text, _>(&upper)
        .bind::<Text, _>(&lower)
        .load::<RawPushRow>(conn)?;
    if rows.len() as i64 != extent.count {
        return Err(CandidateScopeError::Changed);
    }
    let rows = rows
        .into_iter()
        .map(FrozenPushRow::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let canonical = canonical_for(rows, cutoff)?;
    let id = CandidateScopeCaptureId(format!(
        "candidate-scope-capture-v1:{}",
        hex::encode(Sha256::digest(&canonical))
    ));
    Ok(CapturedPushedCandidateScope {
        authority: authority.clone(),
        cutoff,
        id,
        canonical,
    })
}

#[cfg(test)]
thread_local! { static CALENDAR_OVERRIDE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(crate) struct CalendarOverrideGuard(Option<String>);
#[cfg(test)]
impl Drop for CalendarOverrideGuard {
    fn drop(&mut self) {
        CALENDAR_OVERRIDE.with(|v| *v.borrow_mut() = self.0.take());
    }
}
#[cfg(test)]
pub(crate) fn calendar_override_for_test(hash: String) -> CalendarOverrideGuard {
    CalendarOverrideGuard(CALENDAR_OVERRIDE.with(|v| v.borrow_mut().replace(hash)))
}
fn calendar_for(date: chrono::NaiveDate) -> CalendarObservation {
    #[cfg(test)]
    if let Some(hash) = CALENDAR_OVERRIDE.with(|v| v.borrow().clone()) {
        return CalendarObservation::Covered {
            contract: "checked-in-a-share-calendar-replay-v1".into(),
            authority_sha256: hash,
            open: true,
        };
    }
    match (
        crate::calendar::verified_a_share_trading_day(date),
        crate::calendar::verified_a_share_calendar_authority_hash(date),
    ) {
        (Ok(open), Ok(authority_sha256)) => CalendarObservation::Covered {
            contract: "checked-in-a-share-calendar-replay-v1".into(),
            authority_sha256: authority_sha256.into(),
            open,
        },
        _ => CalendarObservation::Unavailable {
            reason: "immutable_calendar_coverage_unavailable".into(),
        },
    }
}
fn canonical_for(
    rows: Vec<FrozenPushRow>,
    cutoff: DateTime<Utc>,
) -> Result<Vec<u8>, CandidateScopeError> {
    let offset = FixedOffset::east_opt(8 * 3600).ok_or(CandidateScopeError::Cutoff)?;
    canonical_with_calendar(
        rows,
        cutoff,
        calendar_for(cutoff.with_timezone(&offset).date_naive()),
    )
}
fn canonical_with_calendar(
    rows: Vec<FrozenPushRow>,
    cutoff: DateTime<Utc>,
    calendar: CalendarObservation,
) -> Result<Vec<u8>, CandidateScopeError> {
    let offset = FixedOffset::east_opt(8 * 3600).ok_or(CandidateScopeError::Cutoff)?;
    let upper_time = cutoff
        .checked_add_signed(Duration::hours(8))
        .ok_or(CandidateScopeError::Cutoff)?;
    if !(1..=9999).contains(&upper_time.year()) {
        return Err(CandidateScopeError::Cutoff);
    }
    let lower_time = upper_time
        .checked_sub_signed(Duration::hours(1))
        .ok_or(CandidateScopeError::Cutoff)?;
    let upper = upper_time.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let lower = lower_time.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let date = cutoff.with_timezone(&offset).date_naive();
    // At most 1 MiB of UTF-8 text can expand sixfold under JSON escaping;
    // bounded row descriptors fit within the remaining canonical allowance.
    let canonical = serde_json::to_vec(&Canonical {
        domain: DOMAIN,
        policy: POLICY,
        scope_query_sha256: hex::encode(Sha256::digest(TOP.as_bytes())),
        predicate_clock: "shanghai_millisecond_text_strict_previous_one_hour",
        scope: "bounded_top50_raw_rows_not_full_hour_or_market",
        source_layout: "global-legacy-pushed-stocks-11-columns-v1",
        cutoff_utc: cutoff.to_rfc3339_opts(SecondsFormat::Nanos, true),
        lower_exclusive_shanghai: &lower,
        upper_exclusive_shanghai: &upper,
        shanghai_civil_date: date.format("%Y-%m-%d").to_string(),
        calendar,
        scope_state: if rows.is_empty() {
            "empty_bounded_scope"
        } else {
            "identity_unqualified"
        },
        candidates: rows.into_iter().map(DeniedRow::from).collect(),
    })
    .map_err(|_| CandidateScopeError::Canonical)?;
    if canonical.len() > CANONICAL_BYTES {
        return Err(CandidateScopeError::Bounds);
    }
    Ok(canonical)
}

/// A value-only closed JSON check. It does not issue a capture or authority.
pub(crate) fn validate_stored_canonical(
    bytes: &[u8],
    cutoff: DateTime<Utc>,
) -> Result<(), CandidateScopeError> {
    decode_historical_scope(bytes, cutoff).map(|_| ())
}
/// Value-only historical projection, never a live capture or writer proof.
pub(super) struct HistoricalScope {
    pub(super) rows: Vec<FrozenPushRow>,
    pub(super) calendar: CalendarObservation,
}
pub(super) fn decode_historical_scope(
    bytes: &[u8],
    cutoff: DateTime<Utc>,
) -> Result<HistoricalScope, CandidateScopeError> {
    if bytes.is_empty() || bytes.len() > CANONICAL_BYTES {
        return Err(CandidateScopeError::Bounds);
    }
    preflight_stored_json(bytes)?;
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| CandidateScopeError::Canonical)?;
    let candidates = value
        .get("candidates")
        .and_then(|v| v.as_array())
        .ok_or(CandidateScopeError::Canonical)?;
    if candidates.len() > 50 {
        return Err(CandidateScopeError::Bounds);
    }
    let mut rows = Vec::with_capacity(candidates.len());
    let mut total = 0usize;
    let upper_time = cutoff
        .checked_add_signed(Duration::hours(8))
        .ok_or(CandidateScopeError::Cutoff)?;
    let lower_time = upper_time
        .checked_sub_signed(Duration::hours(1))
        .ok_or(CandidateScopeError::Cutoff)?;
    let upper = upper_time.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let lower = lower_time.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let mut previous: Option<(String, i64)> = None;
    let mut row_ids = std::collections::BTreeSet::new();
    for candidate in candidates {
        let row: FrozenPushRow = serde_json::from_value(
            candidate
                .get("row")
                .ok_or(CandidateScopeError::Canonical)?
                .clone(),
        )
        .map_err(|_| CandidateScopeError::Canonical)?;
        if row.push_time.as_bytes() <= lower.as_bytes()
            || row.push_time.as_bytes() >= upper.as_bytes()
            || !row_ids.insert(row.id)
            || previous.as_ref().is_some_and(|(time, id)| {
                (row.push_time.as_bytes(), row.id) >= (time.as_bytes(), *id)
            })
        {
            return Err(CandidateScopeError::Canonical);
        }
        previous = Some((row.push_time.clone(), row.id));
        let fields = [
            Some(row.push_time.as_str()),
            Some(row.push_kind.as_str()),
            Some(row.code.as_str()),
            Some(row.name.as_str()),
            Some(row.metric_json.as_str()),
            Some(row.source.as_str()),
            row.consumed_at.as_deref(),
            row.consumed_by.as_deref(),
            row.outcome.as_deref(),
        ];
        let mut row_size = 18usize;
        for (i, field) in fields.into_iter().enumerate() {
            let limit = if i == 4 { METRIC_BYTES } else { FIELD_BYTES } as usize;
            if field.is_some_and(|v| v.len() > limit) {
                return Err(CandidateScopeError::Bounds);
            }
            row_size += field.map_or(1, |v| 9 + v.len());
        }
        if row_size > ROW_BYTES as usize
            || row.consumed_at.is_some()
            || !f64::from_bits(row.push_price_real_bits).is_finite()
        {
            return Err(CandidateScopeError::Bounds);
        }
        total += row_size;
        if total > SCOPE_BYTES as usize {
            return Err(CandidateScopeError::Bounds);
        }
        rows.push(row);
    }
    let calendar: CalendarObservation = serde_json::from_value(
        value
            .get("calendar")
            .ok_or(CandidateScopeError::Canonical)?
            .clone(),
    )
    .map_err(|_| CandidateScopeError::Canonical)?;
    match &calendar {
        CalendarObservation::Covered {
            contract,
            authority_sha256,
            ..
        } if contract == "checked-in-a-share-calendar-replay-v1"
            && authority_sha256.len() == 64
            && authority_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) => {}
        CalendarObservation::Unavailable { reason }
            if reason == "immutable_calendar_coverage_unavailable" => {}
        _ => return Err(CandidateScopeError::Canonical),
    }
    let expected = canonical_with_calendar(rows.clone(), cutoff, calendar.clone())?;
    if expected != bytes {
        return Err(CandidateScopeError::Canonical);
    }
    Ok(HistoricalScope { rows, calendar })
}

// Streaming allocation preflight: do not construct a serde Value/Vec/String
// until every container and source field has passed the original capture bounds.
// serde_json may use its bounded input scratch for escaped strings; this visitor
// never copies them and rejects them before any owned representation is built.
#[derive(Clone, Copy)]
enum JsonPart {
    Root,
    Candidates,
    Candidate,
    Row,
    Facts,
    Fact,
    Calendar,
    Text {
        limit: usize,
        raw: bool,
        nullable: bool,
    },
    Number,
    Boolean,
}
#[derive(Default)]
struct JsonBudget {
    selected: usize,
    row: usize,
}
struct JsonSeed<'a> {
    part: JsonPart,
    budget: &'a mut JsonBudget,
}
impl<'de> serde::de::DeserializeSeed<'de> for JsonSeed<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}
impl JsonBudget {
    fn field<E: serde::de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.row = self
            .row
            .checked_add(bytes)
            .ok_or_else(|| E::custom("row extent overflow"))?;
        if self.row > ROW_BYTES as usize {
            return Err(E::custom("row byte bound"));
        }
        Ok(())
    }
}
impl<'de> serde::de::Visitor<'de> for JsonSeed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("bounded closed candidate observation JSON")
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<(), E> {
        match self.part {
            JsonPart::Text { limit, raw, .. } if value.len() <= limit => {
                if raw {
                    self.budget.field(9 + value.len())?;
                }
                Ok(())
            }
            _ => Err(E::custom("text shape or byte bound")),
        }
    }
    fn visit_borrowed_str<E: serde::de::Error>(self, value: &'de str) -> Result<(), E> {
        self.visit_str(value)
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        match self.part {
            JsonPart::Text {
                raw,
                nullable: true,
                ..
            } => {
                if raw {
                    self.budget.field(1)?;
                }
                Ok(())
            }
            _ => Err(E::custom("unexpected null")),
        }
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        if matches!(self.part, JsonPart::Number) {
            Ok(())
        } else {
            Err(E::custom("unexpected integer"))
        }
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        if matches!(self.part, JsonPart::Number) {
            Ok(())
        } else {
            Err(E::custom("unexpected integer"))
        }
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        if matches!(self.part, JsonPart::Boolean) {
            Ok(())
        } else {
            Err(E::custom("unexpected boolean"))
        }
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let (part, max) = match self.part {
            JsonPart::Candidates => (JsonPart::Candidate, 50),
            JsonPart::Facts => (JsonPart::Fact, 3),
            _ => return Err(serde::de::Error::custom("unexpected array")),
        };
        struct Item<'a> {
            part: JsonPart,
            budget: &'a mut JsonBudget,
            index: usize,
            max: usize,
        }
        impl<'de> serde::de::DeserializeSeed<'de> for Item<'_> {
            type Value = ();
            fn deserialize<D: serde::Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<(), D::Error> {
                if self.index >= self.max {
                    return Err(serde::de::Error::custom("array item bound"));
                }
                serde::de::DeserializeSeed::deserialize(
                    JsonSeed {
                        part: self.part,
                        budget: self.budget,
                    },
                    deserializer,
                )
            }
        }
        let mut count = 0;
        while seq
            .next_element_seed(Item {
                part,
                budget: self.budget,
                index: count,
                max,
            })?
            .is_some()
        {
            count += 1;
        }
        if matches!(self.part, JsonPart::Facts) && count != 3 {
            return Err(serde::de::Error::custom("fact cardinality"));
        }
        Ok(())
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        use JsonPart::*;
        let keys: &[&str] = match self.part {
            Root => &[
                "domain",
                "policy",
                "scope_query_sha256",
                "predicate_clock",
                "scope",
                "source_layout",
                "cutoff_utc",
                "lower_exclusive_shanghai",
                "upper_exclusive_shanghai",
                "shanghai_civil_date",
                "calendar",
                "scope_state",
                "candidates",
            ],
            Candidate => &[
                "row",
                "identity",
                "facts",
                "disposition",
                "risk_inventory",
                "risk_evaluation",
                "cost_model",
                "liquidity_model",
                "budget_allocation",
                "manual_approval",
            ],
            Row => &[
                "id",
                "push_time",
                "push_kind",
                "code",
                "name",
                "push_price_real_bits",
                "metric_json",
                "source",
                "consumed_at",
                "consumed_by",
                "outcome",
            ],
            Fact => &["requirement", "state", "reason"],
            Calendar => &["state", "contract", "authority_sha256", "open", "reason"],
            _ => return Err(<A::Error as serde::de::Error>::custom("unexpected object")),
        };
        if matches!(self.part, Row) {
            self.budget.row = 18;
        }
        let mut seen = 0u32;
        // Canonical keys contain no escapes, so borrowing also forbids allocating
        // a malicious escaped or oversized key before rejecting it.
        while let Some(key) = map.next_key::<&str>()? {
            let index = keys
                .iter()
                .position(|allowed| *allowed == key)
                .ok_or_else(|| <A::Error as serde::de::Error>::custom("unknown field"))?;
            let bit = 1u32 << index;
            if seen & bit != 0 {
                return Err(<A::Error as serde::de::Error>::custom("duplicate field"));
            }
            seen |= bit;
            let part = match (self.part, key) {
                (Root, "candidates") => Candidates,
                (Root, "calendar") => Calendar,
                (Candidate, "row") => Row,
                (Candidate, "facts") => Facts,
                (Row, "id" | "push_price_real_bits") => Number,
                (Calendar, "open") => Boolean,
                (Row, "metric_json") => Text {
                    limit: METRIC_BYTES as usize,
                    raw: true,
                    nullable: false,
                },
                (Row, "consumed_at" | "consumed_by" | "outcome") => Text {
                    limit: FIELD_BYTES as usize,
                    raw: true,
                    nullable: true,
                },
                (Row, _) => Text {
                    limit: FIELD_BYTES as usize,
                    raw: true,
                    nullable: false,
                },
                _ => Text {
                    limit: FIELD_BYTES as usize,
                    raw: false,
                    nullable: false,
                },
            };
            map.next_value_seed(JsonSeed {
                part,
                budget: self.budget,
            })?;
        }
        if !matches!(self.part, Calendar) && seen != (1u32 << keys.len()) - 1 {
            return Err(<A::Error as serde::de::Error>::custom("missing field"));
        }
        if matches!(self.part, Row) {
            self.budget.selected = self
                .budget
                .selected
                .checked_add(self.budget.row)
                .ok_or_else(|| <A::Error as serde::de::Error>::custom("scope extent overflow"))?;
            if self.budget.selected > SCOPE_BYTES as usize {
                return Err(<A::Error as serde::de::Error>::custom("scope byte bound"));
            }
        }
        Ok(())
    }
}
fn preflight_stored_json(bytes: &[u8]) -> Result<(), CandidateScopeError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    serde::de::DeserializeSeed::deserialize(
        JsonSeed {
            part: JsonPart::Root,
            budget: &mut JsonBudget::default(),
        },
        &mut deserializer,
    )
    .map_err(|_| CandidateScopeError::Bounds)?;
    deserializer
        .end()
        .map_err(|_| CandidateScopeError::Canonical)
}

#[cfg(test)]
mod historical_calendar_tests {
    use super::*;
    #[test]
    fn candidate_scope_historical_calendar_closed_shape_rejects_unknown_and_malformed() {
        let cutoff = DateTime::parse_from_rfc3339("2026-09-28T01:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let bytes = canonical_for(Vec::new(), cutoff).unwrap();
        for calendar in [
            serde_json::json!({"state":"covered","contract":"unknown","authority_sha256":"a".repeat(64),"open":true}),
            serde_json::json!({"state":"covered","contract":"checked-in-a-share-calendar-replay-v1","authority_sha256":"a".repeat(63),"open":true}),
            serde_json::json!({"state":"unavailable","reason":"guess"}),
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value["calendar"] = calendar;
            assert!(
                validate_stored_canonical(&serde_json::to_vec(&value).unwrap(), cutoff).is_err()
            );
        }
    }
}
