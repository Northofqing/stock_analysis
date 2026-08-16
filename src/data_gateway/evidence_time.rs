//! BR-210 shared conversion for immutable Magic evidence instants.

use chrono::{DateTime, Utc};
use crate::magic_compat::ProviderId;
use crate::magic_compat::EvidenceTimestamp;

use super::review::GatewayError;

pub fn parse_evidence_instant(
    capability: &'static str,
    provider: ProviderId,
    field: &'static str,
    value: &str,
) -> Result<DateTime<Utc>, GatewayError> {
    let invalid = |detail: &dyn std::fmt::Display| {
        GatewayError::invalid_evidence(
            capability,
            Some(provider),
            format!("invalid {field} timestamp {value:?}: {detail}"),
        )
    };

    EvidenceTimestamp::parse_instant(value).map_err(|error| invalid(&error))?;

    if let Some(milliseconds) = value.strip_prefix("unix-ms:") {
        let milliseconds = milliseconds
            .parse::<i64>()
            .map_err(|error| invalid(&error))?;
        return DateTime::<Utc>::from_timestamp_millis(milliseconds)
            .ok_or_else(|| invalid(&"epoch milliseconds are outside chrono range"));
    }

    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        let seconds = value.parse::<i64>().map_err(|error| invalid(&error))?;
        return DateTime::<Utc>::from_timestamp(seconds, 0)
            .ok_or_else(|| invalid(&"epoch seconds are outside chrono range"));
    }

    if let Some((seconds, fraction)) = value.split_once('.') {
        if seconds.bytes().all(|byte| byte.is_ascii_digit())
            && fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            let seconds = seconds.parse::<i64>().map_err(|error| invalid(&error))?;
            let fraction_value = fraction.parse::<u32>().map_err(|error| invalid(&error))?;
            let scale = 10_u32.pow(
                u32::try_from(9_usize.saturating_sub(fraction.len()))
                    .map_err(|error| invalid(&error))?,
            );
            let nanoseconds = fraction_value
                .checked_mul(scale)
                .ok_or_else(|| invalid(&"fractional nanoseconds overflow"))?;
            return DateTime::<Utc>::from_timestamp(seconds, nanoseconds)
                .ok_or_else(|| invalid(&"fractional epoch is outside chrono range"));
        }
    }

    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| invalid(&error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn br210_accepts_every_magic_instant_encoding_exactly() {
        for (encoded, expected) in [
            ("2026-08-03T21:32:59Z", "2026-08-03T21:32:59+00:00"),
            ("2026-08-04T05:32:59+08:00", "2026-08-03T21:32:59+00:00"),
            ("1785799979", "2026-08-03T23:32:59+00:00"),
            ("1785799979.3", "2026-08-03T23:32:59.300+00:00"),
            ("1785799979.851045000", "2026-08-03T23:32:59.851045+00:00"),
            ("unix-ms:1785799979851", "2026-08-03T23:32:59.851+00:00"),
        ] {
            let parsed = parse_evidence_instant(
                "TEST_CODE_BR210",
                ProviderId::Cninfo,
                "observed_at",
                encoded,
            )
            .expect("unambiguous Magic evidence instant must be admitted");
            assert_eq!(parsed.to_rfc3339(), expected, "encoding={encoded}");
        }
    }

    #[test]
    fn br210_rejects_ambiguous_or_malformed_instants() {
        for invalid in [
            "",
            "-1",
            "1785799979.",
            ".851045000",
            "1785799979.8510450000",
            "unix-ms:-1",
            "2026-08-04T05:32:59",
            "2026-08-04",
            "9223372036854775807",
            "9223372036854775807.1",
            "unix-ms:9223372036854775807",
        ] {
            let error = parse_evidence_instant(
                "TEST_CODE_BR210",
                ProviderId::Cninfo,
                "observed_at",
                invalid,
            )
            .expect_err("ambiguous or malformed evidence must fail closed");
            assert_eq!(error.reason_code(), "invalid_evidence", "value={invalid}");
            assert!(!error.retryable(), "value={invalid}");
        }
    }
}
