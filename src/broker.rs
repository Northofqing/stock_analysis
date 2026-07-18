//! Realtime quote integration used by decision and paper-trading paths.
//!
//! The previous module registered logging-only broker implementations and a
//! `MockQuoteProvider` returning zero. That made an unavailable data source look
//! healthy and encouraged callers to substitute cost/push prices. This module
//! keeps one fail-closed, real provider seam.

use crate::data_provider::gtimg_provider::GtimgProvider;
use std::sync::OnceLock;

/// Broker-reported stock status stored by position modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerStType {
    Normal,
    ST,
    StarST,
}

impl BrokerStType {
    pub fn as_db_value(self) -> Option<&'static str> {
        match self {
            Self::Normal => None,
            Self::ST => Some("ST"),
            Self::StarST => Some("*ST"),
        }
    }

    pub fn from_name(name: &str) -> Self {
        if name.starts_with("*ST") || name.starts_with("S*ST") {
            Self::StarST
        } else if name.starts_with("ST") || name.starts_with("SST") {
            Self::ST
        } else {
            Self::Normal
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerSource {
    PublicData,
}

#[derive(Debug, Clone)]
pub struct ExecutionQuote {
    pub price: f64,
    pub limit_down_price: f64,
    pub limit_up_price: f64,
    pub observed_at: chrono::DateTime<chrono::Utc>,
}

impl BrokerSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::PublicData => "腾讯财经实时行情",
        }
    }
}

/// Synchronous quote boundary. Failures and missing data are explicit.
pub trait QuoteProvider: Send + Sync {
    fn get_execution_quote(&self, code: &str) -> Result<ExecutionQuote, String>;
}

struct PublicQuoteProvider {
    provider: GtimgProvider,
}

impl PublicQuoteProvider {
    fn new() -> Result<Self, String> {
        GtimgProvider::new()
            .map(|provider| Self { provider })
            .map_err(|error| format!("construct Tencent quote provider: {error}"))
    }
}

impl QuoteProvider for PublicQuoteProvider {
    fn get_execution_quote(&self, code: &str) -> Result<ExecutionQuote, String> {
        let quote = self
            .provider
            .fetch_realtime_quote(code)
            .map_err(|error| format!("Tencent realtime quote {code}: {error}"))?
            .ok_or_else(|| format!("Tencent realtime quote {code}: empty response"))?;
        let price = validate_quote_price(code, quote.price)?;
        let limit_down_price = validate_quote_price(
            code,
            quote
                .limit_down_price
                .ok_or_else(|| format!("Tencent realtime quote {code}: missing limit-down"))?,
        )?;
        let limit_up_price = validate_quote_price(
            code,
            quote
                .limit_up_price
                .ok_or_else(|| format!("Tencent realtime quote {code}: missing limit-up"))?,
        )?;
        if limit_down_price > limit_up_price {
            return Err(format!(
                "Tencent realtime quote {code}: invalid daily range {limit_down_price}..{limit_up_price}"
            ));
        }
        Ok(ExecutionQuote {
            price,
            limit_down_price,
            limit_up_price,
            observed_at: quote.source_time,
        })
    }
}

static QUOTE_PROVIDER: OnceLock<Box<dyn QuoteProvider>> = OnceLock::new();

pub fn register_quote_provider(provider: Box<dyn QuoteProvider>) -> Result<(), String> {
    QUOTE_PROVIDER.set(provider).map_err(|_| {
        "QuoteProvider already registered; runtime replacement is forbidden".to_string()
    })
}

pub fn quote_provider_registered() -> bool {
    QUOTE_PROVIDER.get().is_some()
}

/// Fetch and validate the current price from the registered real provider.
pub fn quote_price(code: &str) -> Result<f64, String> {
    execution_quote(code).map(|quote| quote.price)
}

pub fn execution_quote(code: &str) -> Result<ExecutionQuote, String> {
    let provider = QUOTE_PROVIDER
        .get()
        .ok_or_else(|| "QuoteProvider is not registered".to_string())?;
    let quote = provider.get_execution_quote(code)?;
    let age_ms = chrono::Utc::now()
        .signed_duration_since(quote.observed_at)
        .num_milliseconds();
    if !(0..=5_000).contains(&age_ms) {
        return Err(format!(
            "realtime quote for {code} is stale: age_ms={age_ms}"
        ));
    }
    crate::monitor::data_mode::mark_capability_success(
        crate::monitor::data_mode::Capability::Quote,
    )?;
    Ok(quote)
}

/// Configure the only implemented production quote source.
///
/// Unsupported selections are rejected instead of silently downgrading to a
/// no-op or a different source.
pub fn detect_and_register() -> Result<BrokerSource, String> {
    let choice = std::env::var("BROKER_SOURCE").unwrap_or_else(|_| "public".to_string());
    match choice.trim().to_ascii_lowercase().as_str() {
        "" | "public" | "tencent" => {
            register_quote_provider(Box::new(PublicQuoteProvider::new()?))?;
            Ok(BrokerSource::PublicData)
        }
        unsupported => Err(format!(
            "BROKER_SOURCE={unsupported} is not implemented; supported values: public, tencent"
        )),
    }
}

fn validate_quote_price(code: &str, price: f64) -> Result<f64, String> {
    if price.is_finite() && price > 0.0 {
        Ok(price)
    } else {
        Err(format!("invalid realtime quote for {code}: price={price}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn st_type_mapping_is_stable() {
        assert_eq!(BrokerStType::Normal.as_db_value(), None);
        assert_eq!(BrokerStType::ST.as_db_value(), Some("ST"));
        assert_eq!(BrokerStType::StarST.as_db_value(), Some("*ST"));
        assert_eq!(BrokerStType::from_name("*ST华微"), BrokerStType::StarST);
        assert_eq!(BrokerStType::from_name("ST康美"), BrokerStType::ST);
        assert_eq!(BrokerStType::from_name("浦发银行"), BrokerStType::Normal);
    }

    #[test]
    fn quote_validation_rejects_missing_and_non_finite_prices() {
        for price in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(validate_quote_price("TEST_CODE_000001", price).is_err());
        }
        assert_eq!(validate_quote_price("TEST_CODE_000001", 10.25), Ok(10.25));
    }

    #[test]
    fn source_label_names_the_real_provider() {
        assert!(BrokerSource::PublicData.label().contains("腾讯"));
    }
}
