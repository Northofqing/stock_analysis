//! BR-164 ownership boundary for official A-share calendar notice URLs.

use std::fmt;

/// Canonical authority root for checked-in SSE calendar evidence.
///
/// BR-164 keeps every external financial-host identity inside the gateway
/// boundary even when a consumer only validates immutable checked-in data.
pub const OFFICIAL_SSE_AUTHORITY_ROOT: &str = "https://www.sse.com.cn/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfficialAshareExchange {
    Sse,
    Szse,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OfficialExchangeUrlError {
    InvalidUrl(String),
    NotOfficial,
}

impl fmt::Display for OfficialExchangeUrlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(error) => write!(formatter, "canonical_url is invalid: {error}"),
            Self::NotOfficial => {
                formatter.write_str("canonical_url must be a canonical official HTTPS SSE/SZSE URL")
            }
        }
    }
}

impl std::error::Error for OfficialExchangeUrlError {}

pub fn validate_official_exchange_notice_url(
    exchange: OfficialAshareExchange,
    value: &str,
) -> Result<(), OfficialExchangeUrlError> {
    let parsed = url::Url::parse(value)
        .map_err(|error| OfficialExchangeUrlError::InvalidUrl(error.to_string()))?;
    let expected_hosts = match exchange {
        OfficialAshareExchange::Sse => ["www.sse.com.cn", "sse.com.cn"],
        OfficialAshareExchange::Szse => ["www.szse.cn", "szse.cn"],
    };
    if parsed.scheme() == "https"
        && parsed
            .host_str()
            .is_some_and(|host| expected_hosts.contains(&host))
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.fragment().is_none()
    {
        Ok(())
    } else {
        Err(OfficialExchangeUrlError::NotOfficial)
    }
}

pub fn validate_canonical_sse_announcement_url(
    value: &str,
) -> Result<(), OfficialExchangeUrlError> {
    validate_official_exchange_notice_url(OfficialAshareExchange::Sse, value)?;
    let parsed = url::Url::parse(value)
        .map_err(|error| OfficialExchangeUrlError::InvalidUrl(error.to_string()))?;
    if parsed.host_str() == Some("www.sse.com.cn") {
        Ok(())
    } else {
        Err(OfficialExchangeUrlError::NotOfficial)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exchange_authority_accepts_only_matching_canonical_https_hosts() {
        assert!(validate_official_exchange_notice_url(
            OfficialAshareExchange::Sse,
            "https://www.sse.com.cn/disclosure/notice"
        )
        .is_ok());
        assert!(validate_official_exchange_notice_url(
            OfficialAshareExchange::Szse,
            "https://www.szse.cn/disclosure/notice"
        )
        .is_ok());
        for invalid in [
            "http://www.sse.com.cn/disclosure/notice",
            "https://user@www.sse.com.cn/disclosure/notice",
            "https://www.sse.com.cn/disclosure/notice#fragment",
            "https://example.invalid/disclosure/notice",
        ] {
            assert!(
                validate_official_exchange_notice_url(OfficialAshareExchange::Sse, invalid)
                    .is_err()
            );
        }
    }

    #[test]
    fn legacy_calendar_authority_keeps_the_www_sse_contract() {
        assert!(validate_canonical_sse_announcement_url(
            "https://www.sse.com.cn/disclosure/notice"
        )
        .is_ok());
        assert!(
            validate_canonical_sse_announcement_url("https://sse.com.cn/disclosure/notice")
                .is_err()
        );
    }
}
