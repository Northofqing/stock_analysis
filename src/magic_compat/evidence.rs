//! NonEmptyText / SourceEvidence / EvidenceTimestamp 本地镜像 (M5, Task #76,
//! feature 关时使用)。与上游 magic_market_core (pin rev 75ee2a2,
//! crates/magic-market-core/src/validated.rs + evidence.rs + probe.rs) 同构:
//! 字段/serde 表示/校验语义/时间格式接受逻辑一致 (wire 是 JSON)。

#[cfg(not(feature = "magic-gateway"))]
use serde::{de, Deserialize, Deserializer, Serialize};
#[cfg(not(feature = "magic-gateway"))]
use std::fmt;

#[cfg(not(feature = "magic-gateway"))]
const MAX_TEXT_CHARS: usize = 16_384;

/// Trimmed, non-empty, control-free text bounded for untrusted source payloads.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct NonEmptyText(String);

#[cfg(not(feature = "magic-gateway"))]
impl NonEmptyText {
    pub fn new(value: impl Into<String>) -> Result<Self, super::instrument::CoreError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(super::instrument::CoreError::InvalidValue {
                field: "text",
                value,
                reason: "must not be empty",
            });
        }
        if trimmed.chars().any(char::is_control) {
            return Err(super::instrument::CoreError::InvalidValue {
                field: "text",
                value,
                reason: "must not contain control characters",
            });
        }
        if trimmed.chars().count() > MAX_TEXT_CHARS {
            return Err(super::instrument::CoreError::InvalidValue {
                field: "text",
                value: format!("{} characters", trimmed.chars().count()),
                reason: "exceeds maximum length",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(not(feature = "magic-gateway"))]
impl fmt::Display for NonEmptyText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(not(feature = "magic-gateway"))]
impl<'de> Deserialize<'de> for NonEmptyText {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Record-level source and observation evidence.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceEvidence {
    provider: super::provider_id::ProviderId,
    source_at: Option<NonEmptyText>,
    observed_at: NonEmptyText,
    batch_id: NonEmptyText,
}

#[cfg(not(feature = "magic-gateway"))]
impl SourceEvidence {
    pub fn new(
        provider: super::provider_id::ProviderId,
        observed_at: impl Into<String>,
        batch_id: impl Into<String>,
    ) -> Result<Self, super::instrument::CoreError> {
        Ok(Self {
            provider,
            source_at: None,
            observed_at: NonEmptyText::new(observed_at)?,
            batch_id: NonEmptyText::new(batch_id)?,
        })
    }

    pub fn with_source_at(
        mut self,
        source_at: impl Into<String>,
    ) -> Result<Self, super::instrument::CoreError> {
        self.source_at = Some(NonEmptyText::new(source_at)?);
        Ok(self)
    }

    pub fn provider(&self) -> super::provider_id::ProviderId {
        self.provider
    }

    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_ref().map(NonEmptyText::as_str)
    }

    pub fn observed_at(&self) -> &str {
        self.observed_at.as_str()
    }

    pub fn batch_id(&self) -> &str {
        self.batch_id.as_str()
    }
}

#[cfg(not(feature = "magic-gateway"))]
impl<'de> Deserialize<'de> for SourceEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            provider: super::provider_id::ProviderId,
            source_at: Option<String>,
            observed_at: String,
            batch_id: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let mut evidence =
            Self::new(wire.provider, wire.observed_at, wire.batch_id).map_err(de::Error::custom)?;
        if let Some(source_at) = wire.source_at {
            evidence = evidence
                .with_source_at(source_at)
                .map_err(de::Error::custom)?;
        }
        Ok(evidence)
    }
}

/// A parsed provider or observation timestamp, normalized to Unix nanoseconds.
///
/// This type intentionally carries no "source" or "observed" role. Callers must
/// keep those roles explicit and must never substitute one for the other.
#[cfg(not(feature = "magic-gateway"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EvidenceTimestamp {
    unix_nanos: i128,
}

#[cfg(not(feature = "magic-gateway"))]
impl EvidenceTimestamp {
    /// Parses the timestamp formats accepted by provider admission.
    pub fn parse(value: &str) -> Result<Self, super::instrument::CoreError> {
        parse_evidence_time(value)
            .map(|unix_nanos| Self { unix_nanos })
            .ok_or_else(|| {
                super::instrument::CoreError::InvalidRequest(format!(
                    "invalid evidence timestamp {value:?}"
                ))
            })
    }

    /// Parses a timestamp suitable for sub-minute realtime admission.
    ///
    /// Unlike [`Self::parse`], this rejects date-only values and ISO wall-clock
    /// strings without an explicit UTC/offset suffix. Epoch seconds and
    /// `unix-ms:` values are already unambiguous instants.
    pub fn parse_instant(value: &str) -> Result<Self, super::instrument::CoreError> {
        let parsed = Self::parse(value)?;
        if is_unambiguous_instant(value) {
            Ok(parsed)
        } else {
            Err(super::instrument::CoreError::InvalidRequest(format!(
                "evidence timestamp is not an unambiguous instant {value:?}"
            )))
        }
    }
}

#[cfg(not(feature = "magic-gateway"))]
const NANOS_PER_SECOND: i128 = 1_000_000_000;
#[cfg(not(feature = "magic-gateway"))]
const NANOS_PER_MILLISECOND: i128 = 1_000_000;

// ---- 以下镜像 probe.rs 的 parse_evidence_time + is_unambiguous_instant ----
// (格式接受语义是 admission 契约, 与上游逐分支一致; 变更必须对照上游)

#[cfg(not(feature = "magic-gateway"))]
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

#[cfg(not(feature = "magic-gateway"))]
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

#[cfg(not(feature = "magic-gateway"))]
fn epoch_with_fraction(seconds: &str, fraction: &str) -> Option<i128> {
    i128::from(seconds.parse::<i64>().ok()?)
        .checked_mul(NANOS_PER_SECOND)?
        .checked_add(fraction_to_nanos(fraction)?)
}

#[cfg(not(feature = "magic-gateway"))]
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

#[cfg(not(feature = "magic-gateway"))]
fn parse_component(bytes: &[u8], start: usize, end: usize) -> Option<i64> {
    let text = std::str::from_utf8(bytes.get(start..end)?).ok()?;
    text.bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| text.parse().ok())?
}

#[cfg(not(feature = "magic-gateway"))]
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || year % 4 == 0 && year % 100 != 0 => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(not(feature = "magic-gateway"))]
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
