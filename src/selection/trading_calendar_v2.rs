//! BR-193 immutable A-share trading-calendar authorities.
//!
//! The notice manifest is a regular JSON file. The raw-notice root is a
//! separate directory with a separately frozen name; neither path may be
//! derived from caller input or from the other at runtime.

use crate::data_gateway::{validate_official_exchange_notice_url, OfficialAshareExchange};
use crate::selection::activation_runtime::SelectionDisabledReason;
use crate::selection::schema_v2::{sha256_bytes, sha256_domain_bytes};
use chrono::{DateTime, Datelike, NaiveDate, SecondsFormat, Weekday};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

pub const CALENDAR_MANIFEST_RELATIVE_PATH: &str =
    "config/selection/a_share_trading_calendar.v1.json";
pub const NOTICE_MANIFEST_RELATIVE_PATH: &str =
    "config/selection/a_share_trading_calendar_notices.v1.json";
pub const RAW_NOTICE_ROOT_RELATIVE_PATH: &str =
    "config/selection/a_share_trading_calendar_notices.v1";
pub const RELEASE_PREREQUISITE_RELATIVE_PATH: &str =
    "config/selection/a_share_trading_calendar_release_prerequisite.v1.json";

const NOTICE_MANIFEST_DOMAIN: &str = "stock_analysis.a_share_calendar_notice_manifest.v1";
const NOTICE_MANIFEST_HASH_DOMAIN: &[u8] = b"stock_analysis.a_share_calendar_notice_manifest.v1\0";
const RAW_NOTICE_SET_DOMAIN: &str = "stock_analysis.a_share_calendar_raw_notice_set.v1";
const RAW_NOTICE_SET_HASH_DOMAIN: &[u8] = b"stock_analysis.a_share_calendar_raw_notice_set.v1\0";
const PARSER_EQUALITY_DOMAIN: &str = "stock_analysis.a_share_calendar_parser_equality.v1";
const PARSER_EQUALITY_HASH_DOMAIN: &[u8] = b"stock_analysis.a_share_calendar_parser_equality.v1\0";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewedCalendarPrerequisiteReason {
    TradingCalendarMissing,
    TradingCalendarUnverified,
    TradingCalendarCoverageIncomplete,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CalendarReleasePrerequisitePayload {
    domain: String,
    schema_version: u64,
    reason_code: ReviewedCalendarPrerequisiteReason,
    reviewed_at: String,
    executable_revision: String,
}

#[derive(Debug)]
pub struct VerifiedCalendarReleasePrerequisite {
    payload: CalendarReleasePrerequisitePayload,
}

impl VerifiedCalendarReleasePrerequisite {
    pub fn reason(&self) -> &ReviewedCalendarPrerequisiteReason {
        &self.payload.reason_code
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CalendarAuthorityClassification {
    Disabled(SelectionDisabledReason),
    Claimed,
}

/// Closed authority-presence facts classified before calendar capability
/// construction.
#[derive(Debug)]
pub struct CalendarAuthorityPresence {
    calendar_manifest_present: bool,
    notice_manifest_present: bool,
    raw_notice_root_present: bool,
    proposal_claim_present: bool,
    activation_claim_present: bool,
    receipt_claim_present: bool,
    reviewed_markers: Vec<ReviewedCalendarPrerequisiteReason>,
}

impl CalendarAuthorityPresence {
    pub fn reviewed_absence(marker: ReviewedCalendarPrerequisiteReason) -> Self {
        Self {
            calendar_manifest_present: false,
            notice_manifest_present: false,
            raw_notice_root_present: false,
            proposal_claim_present: false,
            activation_claim_present: false,
            receipt_claim_present: false,
            reviewed_markers: vec![marker],
        }
    }

    pub fn unreviewed_absence() -> Self {
        Self {
            calendar_manifest_present: false,
            notice_manifest_present: false,
            raw_notice_root_present: false,
            proposal_claim_present: false,
            activation_claim_present: false,
            receipt_claim_present: false,
            reviewed_markers: Vec::new(),
        }
    }

    pub fn multiple_reviewed_absence_markers() -> Self {
        Self {
            reviewed_markers: vec![
                ReviewedCalendarPrerequisiteReason::TradingCalendarMissing,
                ReviewedCalendarPrerequisiteReason::TradingCalendarUnverified,
            ],
            ..Self::unreviewed_absence()
        }
    }

    pub fn partial_authority() -> Self {
        Self {
            calendar_manifest_present: true,
            ..Self::unreviewed_absence()
        }
    }

    pub fn absent_with_activation_claim() -> Self {
        Self {
            activation_claim_present: true,
            ..Self::unreviewed_absence()
        }
    }

    pub fn complete_authority() -> Self {
        Self {
            calendar_manifest_present: true,
            notice_manifest_present: true,
            raw_notice_root_present: true,
            proposal_claim_present: false,
            activation_claim_present: false,
            receipt_claim_present: false,
            reviewed_markers: Vec::new(),
        }
    }

    pub fn complete_authority_with_marker() -> Self {
        Self {
            reviewed_markers: vec![
                ReviewedCalendarPrerequisiteReason::TradingCalendarCoverageIncomplete,
            ],
            ..Self::complete_authority()
        }
    }
}

pub fn classify_calendar_authority_presence(
    presence: &CalendarAuthorityPresence,
) -> Result<CalendarAuthorityClassification, CalendarEvidenceError> {
    let any_fixed_authority = presence.calendar_manifest_present
        || presence.notice_manifest_present
        || presence.raw_notice_root_present;
    let any_claim = presence.proposal_claim_present
        || presence.activation_claim_present
        || presence.receipt_claim_present;

    if !presence.reviewed_markers.is_empty() {
        if any_fixed_authority || any_claim || presence.reviewed_markers.len() != 1 {
            return Err(release_integrity_conflict(
                "reviewed absence marker conflicts with claimed authority",
            ));
        }
        let reason = match presence
            .reviewed_markers
            .first()
            .expect("one marker was proven above")
        {
            ReviewedCalendarPrerequisiteReason::TradingCalendarMissing => {
                SelectionDisabledReason::TradingCalendarMissing
            }
            ReviewedCalendarPrerequisiteReason::TradingCalendarUnverified => {
                SelectionDisabledReason::TradingCalendarUnverified
            }
            ReviewedCalendarPrerequisiteReason::TradingCalendarCoverageIncomplete => {
                SelectionDisabledReason::TradingCalendarCoverageIncomplete
            }
        };
        return Ok(CalendarAuthorityClassification::Disabled(reason));
    }

    if presence.calendar_manifest_present
        && presence.notice_manifest_present
        && presence.raw_notice_root_present
    {
        return Ok(CalendarAuthorityClassification::Claimed);
    }

    Err(release_integrity_conflict(
        if any_fixed_authority || any_claim {
            "claimed calendar authority is incomplete"
        } else {
            "wholly absent calendar authority lacks one reviewed prerequisite marker"
        },
    ))
}

pub fn parse_release_prerequisite_canonical(
    bytes: &[u8],
) -> Result<VerifiedCalendarReleasePrerequisite, CalendarEvidenceError> {
    let payload: CalendarReleasePrerequisitePayload =
        serde_json::from_slice(bytes).map_err(|error| {
            release_integrity_conflict(format!(
                "strict release-prerequisite marker decode failed: {error}"
            ))
        })?;
    let canonical = serde_json::to_vec(&payload).map_err(|error| {
        release_integrity_conflict(format!(
            "release-prerequisite marker serialization failed: {error}"
        ))
    })?;
    if canonical != bytes {
        return Err(release_integrity_conflict(
            "release-prerequisite marker is not canonical",
        ));
    }
    if payload.domain != "stock_analysis.a_share_trading_calendar_release_prerequisite.v1"
        || payload.schema_version != 1
    {
        return Err(release_integrity_conflict(
            "release-prerequisite marker domain/schema mismatch",
        ));
    }
    require_canonical_published_at(&payload.reviewed_at)
        .map_err(|error| release_integrity_conflict(error.detail))?;
    require_revision(&payload.executable_revision)
        .map_err(|error| release_integrity_conflict(error.detail))?;
    Ok(VerifiedCalendarReleasePrerequisite { payload })
}

#[derive(Debug, PartialEq, Eq)]
pub struct CalendarEvidenceError {
    code: &'static str,
    detail: String,
}

impl CalendarEvidenceError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for CalendarEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for CalendarEvidenceError {}

fn release_integrity_conflict(detail: impl Into<String>) -> CalendarEvidenceError {
    CalendarEvidenceError::new("calendar_release_integrity_conflict", detail)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoticeManifestPayload {
    domain: String,
    schema_version: u64,
    entries: Vec<NoticeManifestEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoticeManifestEntry {
    provider: String,
    published_at: String,
    notice_id: String,
    notice_id_sha256: String,
    canonical_url: String,
    raw_artifact_path: String,
    raw_content_sha256: String,
    parser_id: String,
    parser_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNoticeSetPayload {
    domain: String,
    schema_version: u64,
    entries: Vec<RawNoticeSetEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNoticeSetEntry {
    provider: String,
    published_at: String,
    notice_id: String,
    raw_artifact_path: String,
    raw_content_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParserEqualityPayload {
    domain: String,
    schema_version: u64,
    coverage_start: String,
    coverage_end: String,
    parser_descriptors: Vec<ParserDescriptor>,
    session_dates: Vec<String>,
    t0_d5_vectors: Vec<T0D5Vector>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParserDescriptor {
    provider: String,
    notice_id: String,
    parser_id: String,
    parser_version: String,
    executable_revision: String,
    raw_content_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct T0D5Vector {
    t0: String,
    d1: String,
    d2: String,
    d3: String,
    d4: String,
    d5: String,
}

/// Opaque verified notice-manifest evidence.
///
/// Callers can retain its count and content identity but cannot mutate or
/// manufacture the parsed entries.
#[derive(Debug)]
pub struct VerifiedNoticeManifest {
    payload: NoticeManifestPayload,
    content_hash: String,
}

impl VerifiedNoticeManifest {
    pub fn entry_count(&self) -> usize {
        self.payload.entries.len()
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

#[derive(Debug)]
pub struct VerifiedRawNoticeSet {
    payload: RawNoticeSetPayload,
    content_hash: String,
}

impl VerifiedRawNoticeSet {
    pub fn entry_count(&self) -> usize {
        self.payload.entries.len()
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

#[derive(Debug)]
pub struct VerifiedParserEquality {
    payload: ParserEqualityPayload,
    content_hash: String,
}

impl VerifiedParserEquality {
    pub fn session_count(&self) -> usize {
        self.payload.session_dates.len()
    }

    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

/// Strictly decode and bind the immutable notice-manifest bytes.
///
/// The closed payload contains only strings, an integer schema version and
/// arrays, so the checked-in canonical representation is the exact compact
/// serialization of these ordered closed structs. Alternate object-key order,
/// whitespace or unknown fields is rejected before the hash is admitted.
pub fn parse_notice_manifest_canonical(
    bytes: &[u8],
) -> Result<VerifiedNoticeManifest, CalendarEvidenceError> {
    let payload: NoticeManifestPayload = serde_json::from_slice(bytes).map_err(|error| {
        CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            format!("strict JSON decode failed: {error}"),
        )
    })?;
    let canonical = serde_json::to_vec(&payload).map_err(|error| {
        CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            format!("canonical serialization failed: {error}"),
        )
    })?;
    if canonical != bytes {
        return Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_noncanonical",
            "checked-in bytes differ from the closed canonical payload",
        ));
    }
    validate_notice_manifest(&payload)?;
    let content_hash =
        sha256_domain_bytes(NOTICE_MANIFEST_HASH_DOMAIN, &payload).map_err(|error| {
            CalendarEvidenceError::new("calendar_notice_manifest_invalid", error.to_string())
        })?;
    Ok(VerifiedNoticeManifest {
        payload,
        content_hash,
    })
}

pub fn parse_raw_notice_set_canonical(
    bytes: &[u8],
) -> Result<VerifiedRawNoticeSet, CalendarEvidenceError> {
    let payload: RawNoticeSetPayload = decode_closed_canonical_payload(bytes)?;
    validate_raw_notice_set(&payload)?;
    let content_hash =
        sha256_domain_bytes(RAW_NOTICE_SET_HASH_DOMAIN, &payload).map_err(|error| {
            CalendarEvidenceError::new("calendar_auxiliary_payload_invalid", error.to_string())
        })?;
    Ok(VerifiedRawNoticeSet {
        payload,
        content_hash,
    })
}

pub fn parse_parser_equality_canonical(
    bytes: &[u8],
) -> Result<VerifiedParserEquality, CalendarEvidenceError> {
    let payload: ParserEqualityPayload = decode_closed_canonical_payload(bytes)?;
    validate_parser_equality(&payload)?;
    let content_hash =
        sha256_domain_bytes(PARSER_EQUALITY_HASH_DOMAIN, &payload).map_err(|error| {
            CalendarEvidenceError::new("calendar_auxiliary_payload_invalid", error.to_string())
        })?;
    Ok(VerifiedParserEquality {
        payload,
        content_hash,
    })
}

fn decode_closed_canonical_payload<T>(bytes: &[u8]) -> Result<T, CalendarEvidenceError>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let payload: T = serde_json::from_slice(bytes).map_err(|error| {
        CalendarEvidenceError::new(
            "calendar_auxiliary_payload_invalid",
            format!("strict JSON decode failed: {error}"),
        )
    })?;
    let canonical = serde_json::to_vec(&payload).map_err(|error| {
        CalendarEvidenceError::new(
            "calendar_auxiliary_payload_invalid",
            format!("canonical serialization failed: {error}"),
        )
    })?;
    if canonical == bytes {
        Ok(payload)
    } else {
        Err(CalendarEvidenceError::new(
            "calendar_auxiliary_payload_noncanonical",
            "checked-in bytes differ from the closed canonical payload",
        ))
    }
}

fn validate_notice_manifest(payload: &NoticeManifestPayload) -> Result<(), CalendarEvidenceError> {
    if payload.domain != NOTICE_MANIFEST_DOMAIN || payload.schema_version != 1 {
        return Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            "domain and schema_version must match the BR-193 contract",
        ));
    }
    if payload.entries.is_empty() {
        return Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            "claimed notice authority requires a non-empty manifest",
        ));
    }

    let mut providers = BTreeSet::new();
    for entry in &payload.entries {
        validate_notice_entry(entry)?;
        providers.insert(entry.provider.as_str());
    }
    if providers != BTreeSet::from(["sse", "szse"]) {
        return Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            "notice manifest must carry both SSE and SZSE official evidence",
        ));
    }
    for adjacent in payload.entries.windows(2) {
        let left = (
            adjacent[0].provider.as_str(),
            adjacent[0].published_at.as_str(),
            adjacent[0].notice_id.as_str(),
        );
        let right = (
            adjacent[1].provider.as_str(),
            adjacent[1].published_at.as_str(),
            adjacent[1].notice_id.as_str(),
        );
        if left >= right {
            return Err(CalendarEvidenceError::new(
                "calendar_notice_manifest_invalid",
                "entries must be unique and strictly ordered by provider,published_at,notice_id",
            ));
        }
    }
    Ok(())
}

fn validate_raw_notice_set(payload: &RawNoticeSetPayload) -> Result<(), CalendarEvidenceError> {
    if payload.domain != RAW_NOTICE_SET_DOMAIN || payload.schema_version != 1 {
        return Err(auxiliary_invalid(
            "raw-notice-set domain/schema does not match BR-193",
        ));
    }
    if payload.entries.is_empty() {
        return Err(auxiliary_invalid(
            "claimed raw-notice-set authority must be non-empty",
        ));
    }
    let mut providers = BTreeSet::new();
    for entry in &payload.entries {
        if !matches!(entry.provider.as_str(), "sse" | "szse") {
            return Err(auxiliary_invalid("raw-notice provider must be sse or szse"));
        }
        providers.insert(entry.provider.as_str());
        require_nonblank_aux(&entry.notice_id, "notice_id")?;
        require_canonical_published_at_aux(&entry.published_at)?;
        require_lower_hash_aux(&entry.raw_content_sha256, "raw_content_sha256")?;
        let notice_id_sha256 = sha256_bytes(entry.notice_id.as_bytes());
        let expected_path = format!(
            "{RAW_NOTICE_ROOT_RELATIVE_PATH}/{}/{notice_id_sha256}.raw",
            entry.provider
        );
        if entry.raw_artifact_path != expected_path {
            return Err(auxiliary_invalid(
                "raw-notice path does not match fixed provider/notice-id hash",
            ));
        }
    }
    if providers != BTreeSet::from(["sse", "szse"]) {
        return Err(auxiliary_invalid(
            "raw-notice-set must contain both SSE and SZSE evidence",
        ));
    }
    for adjacent in payload.entries.windows(2) {
        let left = (
            adjacent[0].provider.as_str(),
            adjacent[0].published_at.as_str(),
            adjacent[0].notice_id.as_str(),
        );
        let right = (
            adjacent[1].provider.as_str(),
            adjacent[1].published_at.as_str(),
            adjacent[1].notice_id.as_str(),
        );
        if left >= right {
            return Err(auxiliary_invalid(
                "raw-notice entries must be strictly ordered and unique",
            ));
        }
    }
    Ok(())
}

fn validate_parser_equality(payload: &ParserEqualityPayload) -> Result<(), CalendarEvidenceError> {
    if payload.domain != PARSER_EQUALITY_DOMAIN || payload.schema_version != 1 {
        return Err(auxiliary_invalid(
            "parser-equality domain/schema does not match BR-193",
        ));
    }
    let coverage_start = parse_date(&payload.coverage_start, "coverage_start")?;
    let coverage_end = parse_date(&payload.coverage_end, "coverage_end")?;
    if coverage_start > coverage_end {
        return Err(auxiliary_invalid(
            "parser-equality coverage_start exceeds coverage_end",
        ));
    }

    let mut providers = BTreeSet::new();
    for descriptor in &payload.parser_descriptors {
        if !matches!(descriptor.provider.as_str(), "sse" | "szse") {
            return Err(auxiliary_invalid("parser provider must be sse or szse"));
        }
        providers.insert(descriptor.provider.as_str());
        require_nonblank_aux(&descriptor.notice_id, "notice_id")?;
        require_nonblank_aux(&descriptor.parser_id, "parser_id")?;
        require_nonblank_aux(&descriptor.parser_version, "parser_version")?;
        require_lower_hash_aux(&descriptor.raw_content_sha256, "raw_content_sha256")?;
        require_revision(&descriptor.executable_revision)?;
    }
    if providers != BTreeSet::from(["sse", "szse"]) {
        return Err(auxiliary_invalid(
            "parser descriptors must contain both SSE and SZSE evidence",
        ));
    }
    for adjacent in payload.parser_descriptors.windows(2) {
        let left = (
            adjacent[0].provider.as_str(),
            adjacent[0].notice_id.as_str(),
        );
        let right = (
            adjacent[1].provider.as_str(),
            adjacent[1].notice_id.as_str(),
        );
        if left >= right {
            return Err(auxiliary_invalid(
                "parser descriptors must be strictly ordered and unique",
            ));
        }
    }

    if payload.session_dates.len() < 6 {
        return Err(auxiliary_invalid(
            "parser-equality needs at least six verified sessions",
        ));
    }
    let mut sessions = Vec::with_capacity(payload.session_dates.len());
    for value in &payload.session_dates {
        let date = parse_date(value, "session_date")?;
        if date < coverage_start || date > coverage_end {
            return Err(auxiliary_invalid(
                "session date lies outside declared coverage",
            ));
        }
        if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
            return Err(auxiliary_invalid("session_dates cannot contain a weekend"));
        }
        if sessions.last().is_some_and(|previous| *previous >= date) {
            return Err(auxiliary_invalid(
                "session_dates must be strictly ascending and unique",
            ));
        }
        sessions.push(date);
    }
    if payload.t0_d5_vectors.is_empty() {
        return Err(auxiliary_invalid(
            "parser-equality must contain at least one T0..D5 vector",
        ));
    }
    let mut previous_t0 = None;
    for vector in &payload.t0_d5_vectors {
        let dates = [
            parse_date(&vector.t0, "t0")?,
            parse_date(&vector.d1, "d1")?,
            parse_date(&vector.d2, "d2")?,
            parse_date(&vector.d3, "d3")?,
            parse_date(&vector.d4, "d4")?,
            parse_date(&vector.d5, "d5")?,
        ];
        if previous_t0.is_some_and(|previous| previous >= dates[0]) {
            return Err(auxiliary_invalid(
                "T0..D5 vectors must be strictly T0-ascending",
            ));
        }
        previous_t0 = Some(dates[0]);
        let start = sessions
            .binary_search(&dates[0])
            .map_err(|_| auxiliary_invalid("T0 is not a verified session"))?;
        if sessions.get(start..start + 6) != Some(dates.as_slice()) {
            return Err(auxiliary_invalid(
                "T0..D5 vector is not the exact next-six session slice",
            ));
        }
    }
    Ok(())
}

fn validate_notice_entry(entry: &NoticeManifestEntry) -> Result<(), CalendarEvidenceError> {
    if !matches!(entry.provider.as_str(), "sse" | "szse") {
        return Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            "provider must be sse or szse",
        ));
    }
    require_nonblank(&entry.notice_id, "notice_id")?;
    require_nonblank(&entry.parser_id, "parser_id")?;
    require_nonblank(&entry.parser_version, "parser_version")?;
    require_canonical_published_at(&entry.published_at)?;
    require_lower_hash(&entry.notice_id_sha256, "notice_id_sha256")?;
    require_lower_hash(&entry.raw_content_sha256, "raw_content_sha256")?;
    if sha256_bytes(entry.notice_id.as_bytes()) != entry.notice_id_sha256 {
        return Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            "notice_id_sha256 does not bind the canonical notice_id bytes",
        ));
    }
    let expected_path = format!(
        "{RAW_NOTICE_ROOT_RELATIVE_PATH}/{}/{}.raw",
        entry.provider, entry.notice_id_sha256
    );
    if entry.raw_artifact_path != expected_path {
        return Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            "raw_artifact_path does not match the fixed provider/hash formula",
        ));
    }
    require_official_url(&entry.provider, &entry.canonical_url)
}

fn require_nonblank(value: &str, field: &'static str) -> Result<(), CalendarEvidenceError> {
    if !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control) {
        Ok(())
    } else {
        Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            format!("{field} must be nonblank, trim-stable UTF-8"),
        ))
    }
}

fn auxiliary_invalid(detail: impl Into<String>) -> CalendarEvidenceError {
    CalendarEvidenceError::new("calendar_auxiliary_payload_invalid", detail)
}

fn require_nonblank_aux(value: &str, field: &'static str) -> Result<(), CalendarEvidenceError> {
    require_nonblank(value, field).map_err(|error| auxiliary_invalid(error.detail))
}

fn require_lower_hash(value: &str, field: &'static str) -> Result<(), CalendarEvidenceError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            format!("{field} must be 64 lowercase hexadecimal characters"),
        ))
    }
}

fn require_lower_hash_aux(value: &str, field: &'static str) -> Result<(), CalendarEvidenceError> {
    require_lower_hash(value, field).map_err(|error| auxiliary_invalid(error.detail))
}

fn require_canonical_published_at(value: &str) -> Result<(), CalendarEvidenceError> {
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|error| {
        CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            format!("published_at is invalid: {error}"),
        )
    })?;
    let fraction_is_nanos = value
        .split_once('.')
        .and_then(|(_, suffix)| suffix.get(..9))
        .is_some_and(|fraction| {
            fraction.len() == 9 && fraction.bytes().all(|byte| byte.is_ascii_digit())
        });
    let canonical = parsed.to_rfc3339_opts(SecondsFormat::Nanos, value.ends_with('Z'));
    if fraction_is_nanos && canonical == value {
        Ok(())
    } else {
        Err(CalendarEvidenceError::new(
            "calendar_notice_manifest_invalid",
            "published_at must be canonical RFC3339 with nanoseconds and an explicit offset",
        ))
    }
}

fn require_canonical_published_at_aux(value: &str) -> Result<(), CalendarEvidenceError> {
    require_canonical_published_at(value).map_err(|error| auxiliary_invalid(error.detail))
}

fn require_revision(value: &str) -> Result<(), CalendarEvidenceError> {
    if value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(auxiliary_invalid(
            "executable_revision must be 40 lowercase hexadecimal characters",
        ))
    }
}

fn parse_date(value: &str, field: &'static str) -> Result<NaiveDate, CalendarEvidenceError> {
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|error| auxiliary_invalid(format!("{field} is invalid: {error}")))?;
    if parsed.format("%Y-%m-%d").to_string() == value {
        Ok(parsed)
    } else {
        Err(auxiliary_invalid(format!("{field} is not canonical")))
    }
}

fn require_official_url(provider: &str, value: &str) -> Result<(), CalendarEvidenceError> {
    let exchange = match provider {
        "sse" => OfficialAshareExchange::Sse,
        "szse" => OfficialAshareExchange::Szse,
        _ => unreachable!("provider was validated before URL validation"),
    };
    validate_official_exchange_notice_url(exchange, value).map_err(|error| {
        CalendarEvidenceError::new("calendar_notice_manifest_invalid", error.to_string())
    })
}
