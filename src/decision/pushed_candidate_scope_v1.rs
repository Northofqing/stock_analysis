//! Bounded observation of the actual legacy pushed-row scope.
//! This is source preparation for F2, not a persisted investment decision.
use crate::database::global_schema_v1::paper_v6::{PaperCatalog6Error, VerifiedCatalog6};
use crate::database::DatabaseConnectionAuthority;
use chrono::{DateTime, Datelike, Duration, FixedOffset, SecondsFormat, Utc};
use diesel::sql_types::{BigInt, Binary, Double, Nullable, Text};
use diesel::{QueryableByName, RunQueryDsl, SqliteConnection};
use serde::Serialize;
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
#[derive(Serialize)]
struct FrozenPushRow {
    id: i64,
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
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum CalendarObservation {
    Covered {
        contract: &'static str,
        authority_sha256: &'static str,
        open: bool,
    },
    Unavailable {
        reason: &'static str,
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
    let encoding = diesel::sql_query("PRAGMA main.encoding").get_result::<MainEncoding>(conn)?;
    if encoding.encoding != "UTF-8" {
        return Err(CandidateScopeError::Encoding);
    }
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
    let date = cutoff.with_timezone(&offset).date_naive();
    let calendar = match (
        crate::calendar::verified_a_share_trading_day(date),
        crate::calendar::verified_a_share_calendar_authority_hash(date),
    ) {
        (Ok(open), Ok(authority_sha256)) => CalendarObservation::Covered {
            contract: "checked-in-a-share-calendar-replay-v1",
            authority_sha256,
            open,
        },
        _ => CalendarObservation::Unavailable {
            reason: "immutable_calendar_coverage_unavailable",
        },
    };
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
    let id = CandidateScopeCaptureId(format!(
        "candidate-scope-capture-v1:{}",
        hex::encode(Sha256::digest(&canonical))
    ));
    Ok(CapturedPushedCandidateScope {
        authority: proof.connection_authority().clone(),
        cutoff,
        id,
        canonical,
    })
}
