use crate::{CoreError, DataBatch, NonEmptyText, Provenance, ProviderId, SourceEvidence};
use std::collections::HashSet;
use std::fmt;
use std::time::{Duration, Instant};
use thiserror::Error;

const NANOS_PER_SECOND: i128 = 1_000_000_000;
const NANOS_PER_MILLISECOND: i128 = 1_000_000;

/// A parsed provider or observation timestamp, normalized to Unix nanoseconds.
///
/// This type intentionally carries no "source" or "observed" role. Callers must
/// keep those roles explicit and must never substitute one for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceTimestamp {
    unix_nanos: i128,
}

impl EvidenceTimestamp {
    /// Parses the timestamp formats accepted by provider admission.
    pub fn parse(value: &str) -> Result<Self, CoreError> {
        parse_evidence_time(value)
            .map(|unix_nanos| Self { unix_nanos })
            .ok_or_else(|| {
                CoreError::InvalidRequest(format!("invalid evidence timestamp {value:?}"))
            })
    }

    /// Parses a timestamp suitable for sub-minute realtime admission.
    ///
    /// Unlike [`Self::parse`], this rejects date-only values and ISO wall-clock
    /// strings without an explicit UTC/offset suffix. Epoch seconds and
    /// `unix-ms:` values are already unambiguous instants.
    pub fn parse_instant(value: &str) -> Result<Self, CoreError> {
        let parsed = Self::parse(value)?;
        if is_unambiguous_instant(value) {
            Ok(parsed)
        } else {
            Err(CoreError::InvalidRequest(format!(
                "evidence timestamp is not an unambiguous instant {value:?}"
            )))
        }
    }

    /// Returns `self - earlier`, or `None` when `self` is earlier.
    pub fn duration_since(self, earlier: Self) -> Option<Duration> {
        let nanos = self.unix_nanos.checked_sub(earlier.unix_nanos)?;
        let nanos = u128::try_from(nanos).ok()?;
        let seconds = u64::try_from(nanos / NANOS_PER_SECOND as u128).ok()?;
        let subsec_nanos = u32::try_from(nanos % NANOS_PER_SECOND as u128).ok()?;
        Some(Duration::new(seconds, subsec_nanos))
    }
}

fn is_unambiguous_instant(value: &str) -> bool {
    if let Some(millis) = value.strip_prefix("unix-ms:") {
        return !millis.is_empty() && millis.bytes().all(|byte| byte.is_ascii_digit());
    }
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        return !value.is_empty();
    }
    if let Some((seconds, fraction)) = value.split_once('.') {
        if !seconds.is_empty()
            && seconds.bytes().all(|byte| byte.is_ascii_digit())
            && !fraction.is_empty()
            && fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return true;
        }
    }

    let Some(mut suffix) = value.get(19..) else {
        return false;
    };
    if let Some(fractional) = suffix.strip_prefix('.') {
        let boundary = fractional
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(fractional.len());
        if boundary == 0 {
            return false;
        }
        suffix = &fractional[boundary..];
    }
    suffix == "Z"
        || suffix.len() == 6
            && matches!(suffix.as_bytes().first(), Some(b'+' | b'-'))
            && suffix.as_bytes().get(3) == Some(&b':')
}

/// Stable machine state emitted by public-provider probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
    Admitted,
    VerifiedEmpty,
    DiagnosticCompleteUnadmitted,
    SkippedMissingSecret,
    Failed,
}

impl ProbeStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::VerifiedEmpty => "verified_empty",
            Self::DiagnosticCompleteUnadmitted => "diagnostic_complete_unadmitted",
            Self::SkippedMissingSecret => "skipped_missing_secret",
            Self::Failed => "failed",
        }
    }

    pub const fn satisfies_capability(self) -> bool {
        matches!(self, Self::Admitted | Self::VerifiedEmpty)
    }
}

impl fmt::Display for ProbeStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Provider and source-time requirements for one probe family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeAdmissionPolicy {
    expected_provider: ProviderId,
    require_source_at: bool,
    max_source_age: Option<Duration>,
}

impl ProbeAdmissionPolicy {
    pub const fn new(expected_provider: ProviderId) -> Self {
        Self {
            expected_provider,
            require_source_at: false,
            max_source_age: None,
        }
    }

    pub const fn require_source_at(mut self) -> Self {
        self.require_source_at = true;
        self
    }

    pub fn with_max_source_age(mut self, value: Duration) -> Result<Self, CoreError> {
        if value.is_zero() {
            return Err(CoreError::InvalidRequest(
                "probe maximum source age must be positive".into(),
            ));
        }
        self.require_source_at = true;
        self.max_source_age = Some(value);
        Ok(self)
    }
}

/// Clone-shared, provider-internal observation of actual transport starts.
#[derive(Debug, Default)]
pub struct ProbeRequestTracker {
    last_started: Option<Instant>,
    minimum_start_gap: Option<Duration>,
    request_starts: u64,
    active_requests: u32,
    maximum_concurrency: u32,
}

impl ProbeRequestTracker {
    /// Records one actual request immediately before the transport call.
    pub fn request_started(&mut self) {
        let started = Instant::now();
        if let Some(previous) = self.last_started {
            let gap = started.duration_since(previous);
            self.minimum_start_gap = Some(
                self.minimum_start_gap
                    .map_or(gap, |current| current.min(gap)),
            );
        }
        self.last_started = Some(started);
        self.request_starts = self.request_starts.saturating_add(1);
        self.active_requests = self.active_requests.saturating_add(1);
        self.maximum_concurrency = self.maximum_concurrency.max(self.active_requests);
    }

    /// Records completion of one previously started transport call.
    pub fn request_finished(&mut self) -> Result<(), LoadProbeError> {
        if self.active_requests == 0 {
            return Err(LoadProbeError::FinishWithoutStart);
        }
        self.active_requests -= 1;
        Ok(())
    }

    pub const fn snapshot(&self) -> LoadProbeSnapshot {
        LoadProbeSnapshot {
            request_starts: self.request_starts,
            minimum_start_gap: self.minimum_start_gap,
            maximum_concurrency: self.maximum_concurrency,
            active_requests: self.active_requests,
        }
    }
}

/// Secret-free evidence captured from actual provider transport calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadProbeSnapshot {
    request_starts: u64,
    minimum_start_gap: Option<Duration>,
    maximum_concurrency: u32,
    active_requests: u32,
}

impl LoadProbeSnapshot {
    pub const fn request_starts(self) -> u64 {
        self.request_starts
    }

    pub const fn minimum_start_gap(self) -> Option<Duration> {
        self.minimum_start_gap
    }

    pub const fn maximum_concurrency(self) -> u32 {
        self.maximum_concurrency
    }

    pub const fn active_requests(self) -> u32 {
        self.active_requests
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum LoadProbeError {
    #[error("load probe observed no actual request starts")]
    NoRequestStarts,
    #[error("load probe snapshot still has {active} active requests")]
    RequestsStillActive { active: u32 },
    #[error("load probe observed maximum concurrency {maximum}, expected exactly one")]
    ConcurrentRequests { maximum: u32 },
    #[error("load probe is missing a start gap for multiple requests")]
    MissingStartGap,
    #[error("actual request-start gap {actual:?} is shorter than required {required:?}")]
    StartGapTooShort {
        actual: Duration,
        required: Duration,
    },
    #[error("request completion was recorded without an active request")]
    FinishWithoutStart,
}

/// Verifies serial, internally paced load evidence.
pub fn verify_serial_load(
    snapshot: &LoadProbeSnapshot,
    minimum_start_gap: Duration,
) -> Result<ProbeStatus, LoadProbeError> {
    if snapshot.request_starts == 0 {
        return Err(LoadProbeError::NoRequestStarts);
    }
    if snapshot.active_requests != 0 {
        return Err(LoadProbeError::RequestsStillActive {
            active: snapshot.active_requests,
        });
    }
    if snapshot.maximum_concurrency != 1 {
        return Err(LoadProbeError::ConcurrentRequests {
            maximum: snapshot.maximum_concurrency,
        });
    }
    if snapshot.request_starts > 1 {
        let actual = snapshot
            .minimum_start_gap
            .ok_or(LoadProbeError::MissingStartGap)?;
        if actual < minimum_start_gap {
            return Err(LoadProbeError::StartGapTooShort {
                actual,
                required: minimum_start_gap,
            });
        }
    }
    Ok(ProbeStatus::Admitted)
}

/// A source-proven legitimate empty response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEmpty {
    family: NonEmptyText,
    request_identity: NonEmptyText,
    reason: NonEmptyText,
    evidence: SourceEvidence,
    provenance: Provenance,
}

impl VerifiedEmpty {
    pub fn new(
        family: impl Into<String>,
        request_identity: impl Into<String>,
        reason: impl Into<String>,
        evidence: SourceEvidence,
        provenance: Provenance,
    ) -> Result<Self, ProbeAdmissionError> {
        let family = NonEmptyText::new(family)?;
        let request_identity = NonEmptyText::new(request_identity)?;
        let reason = NonEmptyText::new(reason)?;
        let policy = ProbeAdmissionPolicy::new(evidence.provider());
        verify_evidence(&evidence, &provenance, &policy)?;
        Ok(Self {
            family,
            request_identity,
            reason,
            evidence,
            provenance,
        })
    }

    pub fn family(&self) -> &str {
        self.family.as_str()
    }

    pub fn request_identity(&self) -> &str {
        self.request_identity.as_str()
    }

    pub fn reason(&self) -> &str {
        self.reason.as_str()
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }

    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }
}

impl fmt::Display for VerifiedEmpty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "family={} request_identity={} reason={}",
            self.family, self.request_identity, self.reason
        )
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ProbeAdmissionError {
    #[error("ordinary empty DataBatch is not admitted")]
    EmptyBatch,
    #[error("batch quality is incomplete: {issues:?}")]
    IncompleteQuality { issues: Vec<String> },
    #[error("batch provenance is missing batch_id")]
    MissingBatchId,
    #[error("source timestamp is required for this probe family")]
    MissingSourceTime,
    #[error("record provider mismatch: expected {expected:?}, got {actual:?}")]
    ProviderMismatch {
        expected: ProviderId,
        actual: ProviderId,
    },
    #[error("record observed_at mismatch: expected {expected}, got {actual}")]
    ObservedAtMismatch { expected: String, actual: String },
    #[error("record batch_id mismatch: expected {expected}, got {actual}")]
    BatchIdMismatch { expected: String, actual: String },
    #[error("record source_at mismatch: expected {expected:?}, got {actual:?}")]
    SourceAtMismatch {
        expected: Option<String>,
        actual: Option<String>,
    },
    #[error("invalid {field} timestamp {value:?}")]
    InvalidTimestamp { field: &'static str, value: String },
    #[error("source_at {source_at} is later than observed_at {observed_at}")]
    FutureSourceTime {
        source_at: String,
        observed_at: String,
    },
    #[error("source_at is stale by {age_nanos}ns; maximum allowed age is {max_age_nanos}ns")]
    StaleSourceTime {
        age_nanos: u128,
        max_age_nanos: u128,
    },
    #[error("record business identity must not be empty")]
    EmptyIdentity,
    #[error("duplicate record business identity {identity}")]
    DuplicateIdentity { identity: String },
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// Verifies one advertised, non-empty Provider batch.
pub fn verify_admitted_batch<T>(
    batch: &DataBatch<T>,
    policy: &ProbeAdmissionPolicy,
    evidence_of: impl Fn(&T) -> &SourceEvidence,
    identity_of: impl Fn(&T) -> String,
) -> Result<ProbeStatus, ProbeAdmissionError> {
    if batch.records().is_empty() {
        return Err(ProbeAdmissionError::EmptyBatch);
    }
    if !batch.quality().is_complete() || !batch.quality().issues().is_empty() {
        return Err(ProbeAdmissionError::IncompleteQuality {
            issues: batch.quality().issues().to_vec(),
        });
    }

    let mut identities = HashSet::with_capacity(batch.records().len());
    for record in batch.records() {
        verify_evidence(evidence_of(record), batch.provenance(), policy)?;
        let identity = identity_of(record);
        let identity = identity.trim();
        if identity.is_empty() || identity.chars().any(char::is_control) {
            return Err(ProbeAdmissionError::EmptyIdentity);
        }
        if !identities.insert(identity.to_owned()) {
            return Err(ProbeAdmissionError::DuplicateIdentity {
                identity: identity.to_owned(),
            });
        }
    }
    Ok(ProbeStatus::Admitted)
}

/// Verifies a typed, source-proven legitimate empty result.
pub fn verify_verified_empty(
    empty: &VerifiedEmpty,
    policy: &ProbeAdmissionPolicy,
) -> Result<ProbeStatus, ProbeAdmissionError> {
    verify_evidence(empty.evidence(), empty.provenance(), policy)?;
    Ok(ProbeStatus::VerifiedEmpty)
}

fn verify_evidence(
    evidence: &SourceEvidence,
    provenance: &Provenance,
    policy: &ProbeAdmissionPolicy,
) -> Result<(), ProbeAdmissionError> {
    if evidence.provider() != policy.expected_provider {
        return Err(ProbeAdmissionError::ProviderMismatch {
            expected: policy.expected_provider,
            actual: evidence.provider(),
        });
    }
    if evidence.observed_at() != provenance.fetched_at() {
        return Err(ProbeAdmissionError::ObservedAtMismatch {
            expected: provenance.fetched_at().to_owned(),
            actual: evidence.observed_at().to_owned(),
        });
    }
    let batch_id = provenance
        .batch_id()
        .ok_or(ProbeAdmissionError::MissingBatchId)?;
    if evidence.batch_id() != batch_id {
        return Err(ProbeAdmissionError::BatchIdMismatch {
            expected: batch_id.to_owned(),
            actual: evidence.batch_id().to_owned(),
        });
    }
    if evidence.source_at() != provenance.source_at() {
        return Err(ProbeAdmissionError::SourceAtMismatch {
            expected: provenance.source_at().map(str::to_owned),
            actual: evidence.source_at().map(str::to_owned),
        });
    }

    let Some(source_at) = provenance.source_at() else {
        if policy.require_source_at {
            return Err(ProbeAdmissionError::MissingSourceTime);
        }
        return Ok(());
    };
    let parse_timestamp = |value: &str| {
        if policy.max_source_age.is_some() {
            EvidenceTimestamp::parse_instant(value)
        } else {
            EvidenceTimestamp::parse(value)
        }
    };
    let source_time =
        parse_timestamp(source_at).map_err(|_| ProbeAdmissionError::InvalidTimestamp {
            field: "source_at",
            value: source_at.to_owned(),
        })?;
    let observed_at = provenance.fetched_at();
    let observed_time =
        parse_timestamp(observed_at).map_err(|_| ProbeAdmissionError::InvalidTimestamp {
            field: "observed_at",
            value: observed_at.to_owned(),
        })?;
    let Some(age) = observed_time.duration_since(source_time) else {
        return Err(ProbeAdmissionError::FutureSourceTime {
            source_at: source_at.to_owned(),
            observed_at: observed_at.to_owned(),
        });
    };
    if let Some(maximum) = policy.max_source_age {
        if age > maximum {
            return Err(ProbeAdmissionError::StaleSourceTime {
                age_nanos: age.as_nanos(),
                max_age_nanos: maximum.as_nanos(),
            });
        }
    }
    Ok(())
}

fn parse_evidence_time(value: &str) -> Option<i128> {
    if let Some(millis) = value.strip_prefix("unix-ms:") {
        if millis.is_empty() || !millis.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        return i128::from(millis.parse::<i64>().ok()?).checked_mul(NANOS_PER_MILLISECOND);
    }
    let is_digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    match value.split_once('.') {
        Some((seconds, fraction)) if is_digits(seconds) && is_digits(fraction) => {
            return epoch_with_fraction(seconds, fraction);
        }
        None if is_digits(value) => {
            return i128::from(value.parse::<i64>().ok()?).checked_mul(NANOS_PER_SECOND);
        }
        _ => {}
    }

    let bytes = value.as_bytes();
    if bytes.len() < 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let year = parse_component(bytes, 0, 4)?;
    let month = parse_component(bytes, 5, 7)?;
    let day = parse_component(bytes, 8, 10)?;
    if !(1..=12).contains(&month) || !(1..=days_in_month(year, month)).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    if bytes.len() == 10 {
        return i128::from(days)
            .checked_mul(86_400)?
            .checked_mul(NANOS_PER_SECOND);
    }
    if !matches!(bytes.get(10), Some(b'T' | b' ')) || bytes.len() < 19 {
        return None;
    }
    let hour = parse_component(bytes, 11, 13)?;
    let minute = parse_component(bytes, 14, 16)?;
    let second = parse_component(bytes, 17, 19)?;
    if bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let suffix = &value[19..];
    let (fraction_nanos, suffix) = match suffix.strip_prefix('.') {
        Some(fractional) => {
            let boundary = fractional
                .find(|character: char| !character.is_ascii_digit())
                .unwrap_or(fractional.len());
            let digits = &fractional[..boundary];
            if digits.is_empty() {
                return None;
            }
            (fraction_to_nanos(digits)?, &fractional[boundary..])
        }
        None => (0, suffix),
    };
    let offset_seconds = match suffix {
        "" | "Z" => 0,
        _ if suffix.len() == 6
            && matches!(suffix.as_bytes().first(), Some(b'+' | b'-'))
            && suffix.as_bytes().get(3) == Some(&b':') =>
        {
            let offset_hour = parse_component(suffix.as_bytes(), 1, 3)?;
            let offset_minute = parse_component(suffix.as_bytes(), 4, 6)?;
            if offset_hour > 23 || offset_minute > 59 {
                return None;
            }
            let magnitude = offset_hour.checked_mul(3_600)? + offset_minute.checked_mul(60)?;
            if suffix.starts_with('-') {
                -magnitude
            } else {
                magnitude
            }
        }
        _ => return None,
    };
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour.checked_mul(3_600)?)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?
        .checked_sub(offset_seconds)?;
    i128::from(seconds)
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|nanos| nanos.checked_add(fraction_nanos))
}

fn epoch_with_fraction(seconds: &str, fraction: &str) -> Option<i128> {
    i128::from(seconds.parse::<i64>().ok()?)
        .checked_mul(NANOS_PER_SECOND)?
        .checked_add(fraction_to_nanos(fraction)?)
}

fn fraction_to_nanos(fraction: &str) -> Option<i128> {
    if fraction.is_empty()
        || fraction.len() > 9
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let parsed = fraction.parse::<i128>().ok()?;
    parsed.checked_mul(10_i128.pow(u32::try_from(9 - fraction.len()).ok()?))
}

fn parse_component(bytes: &[u8], start: usize, end: usize) -> Option<i64> {
    let text = std::str::from_utf8(bytes.get(start..end)?).ok()?;
    text.bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| text.parse().ok())?
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || year % 4 == 0 && year % 100 != 0 => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    let adjusted_year = year.checked_sub(i64::from(month <= 2))?;
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era.checked_mul(146_097)?
        .checked_add(day_of_era)?
        .checked_sub(719_468)
}
