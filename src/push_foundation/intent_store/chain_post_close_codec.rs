use std::collections::HashSet;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::market_data::TopStock;
use crate::monitor::push_job::BusinessDate;
use crate::pipeline::chain_analysis::preparation::{SourceObservation, SourceStatus};

use super::ChainPostCloseError;

const INPUT_CODEC_VERSION: u32 = 1;
const MAX_TEXT_BYTES: usize = 512;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheRow {
    code: String,
    concepts: String,
    updated_at: String,
}

impl CacheRow {
    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(super) fn try_new(
        code: String,
        concepts: String,
        updated_at: String,
    ) -> Result<Self, ChainPostCloseError> {
        validate_text("cache code", &code)?;
        validate_text("cache updated_at", &updated_at)?;
        validate_concepts(&concepts)?;
        Ok(Self {
            code,
            concepts,
            updated_at,
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputEnvelope {
    codec_version: u32,
    input: FixedChainPreparationInput,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FixedChainPreparationInput {
    business_date: String,
    stocks: Vec<TopStock>,
    macro_news: Option<String>,
    limit_up_source: SourceObservation,
    macro_source: SourceObservation,
    cache_cutoff: Option<String>,
    cache_rows: Vec<CacheRow>,
}

impl FixedChainPreparationInput {
    pub(crate) fn try_new(
        business_date: BusinessDate,
        stocks: Vec<TopStock>,
        macro_news: Option<String>,
        limit_up_source: SourceObservation,
        macro_source: SourceObservation,
    ) -> Result<Self, ChainPostCloseError> {
        let input = Self {
            business_date: business_date.as_str().to_owned(),
            stocks,
            macro_news,
            limit_up_source,
            macro_source,
            cache_cutoff: None,
            cache_rows: Vec::new(),
        };
        input.validate(false)?;
        Ok(input)
    }

    pub(super) fn attach_cache(
        &mut self,
        cutoff: String,
        cache_rows: Vec<CacheRow>,
    ) -> Result<(), ChainPostCloseError> {
        validate_text("cache cutoff", &cutoff)?;
        self.cache_cutoff = Some(cutoff);
        self.cache_rows = cache_rows;
        self.validate(true)
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, ChainPostCloseError> {
        self.validate(true)?;
        serde_json::to_vec(&InputEnvelope {
            codec_version: INPUT_CODEC_VERSION,
            input: self.clone(),
        })
        .map_err(|_| invalid("input codec"))
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, ChainPostCloseError> {
        let envelope: InputEnvelope =
            serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if envelope.codec_version != INPUT_CODEC_VERSION {
            return Err(ChainPostCloseError::UnsupportedVersion);
        }
        envelope
            .input
            .validate(true)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?;
        let encoded =
            serde_json::to_vec(&envelope).map_err(|_| ChainPostCloseError::SchemaRejected)?;
        if encoded != bytes {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        Ok(envelope.input)
    }

    pub(crate) fn cache_cutoff(&self) -> &str {
        self.cache_cutoff.as_deref().unwrap_or("")
    }

    pub(crate) fn cache_rows(&self) -> &[CacheRow] {
        &self.cache_rows
    }

    pub(super) fn stocks(&self) -> &[TopStock] {
        &self.stocks
    }

    pub(super) fn macro_news(&self) -> &Option<String> {
        &self.macro_news
    }

    pub(super) fn business_date(&self) -> &str {
        &self.business_date
    }

    pub(super) fn caller_bytes(&self) -> Result<Vec<u8>, ChainPostCloseError> {
        let mut caller = self.clone();
        caller.cache_cutoff = None;
        caller.cache_rows.clear();
        caller.validate(false)?;
        serde_json::to_vec(&InputEnvelope {
            codec_version: INPUT_CODEC_VERSION,
            input: caller,
        })
        .map_err(|_| invalid("input codec"))
    }

    pub(super) fn cached_concepts(
        &self,
        code: &str,
    ) -> Result<Option<Vec<String>>, ChainPostCloseError> {
        self.cache_rows
            .iter()
            .find(|row| row.code == code)
            .map(|row| {
                serde_json::from_str(&row.concepts).map_err(|_| ChainPostCloseError::SchemaRejected)
            })
            .transpose()
    }

    fn validate(&self, require_cache: bool) -> Result<(), ChainPostCloseError> {
        let date = NaiveDate::parse_from_str(&self.business_date, "%Y-%m-%d")
            .map_err(|_| invalid("business date"))?;
        if date.format("%Y-%m-%d").to_string() != self.business_date {
            return Err(invalid("business date"));
        }
        validate_stocks(&self.stocks)?;
        validate_source(&self.limit_up_source, date)?;
        validate_source(&self.macro_source, date)?;

        match self.limit_up_source.status() {
            SourceStatus::Available if self.stocks.is_empty() => {
                return Err(invalid("available limit-up source is empty"));
            }
            SourceStatus::VerifiedEmpty if !self.stocks.is_empty() => {
                return Err(invalid("verified-empty limit-up source has rows"));
            }
            SourceStatus::Available | SourceStatus::VerifiedEmpty => {}
            _ => return Err(invalid("limit-up source status")),
        }
        match (self.macro_source.status(), self.macro_news.as_deref()) {
            (SourceStatus::Available, Some(text))
                if !text.trim().is_empty() && !text.contains('\0') => {}
            (SourceStatus::VerifiedEmpty, None) | (SourceStatus::Unavailable, None) => {}
            _ => return Err(invalid("macro source/input binding")),
        }
        if self.macro_source.status() == &SourceStatus::Unavailable
            && self
                .macro_source
                .reason()
                .is_none_or(|reason| reason.trim().is_empty() || reason.contains('\0'))
        {
            return Err(invalid("macro unavailable reason"));
        }

        if require_cache != self.cache_cutoff.is_some() {
            return Err(invalid("cache snapshot presence"));
        }
        let mut cache_codes = HashSet::new();
        for row in &self.cache_rows {
            CacheRow::try_new(
                row.code.clone(),
                row.concepts.clone(),
                row.updated_at.clone(),
            )?;
            if !cache_codes.insert(row.code.as_str()) {
                return Err(invalid("duplicate cache code"));
            }
        }
        Ok(())
    }
}

fn validate_stocks(stocks: &[TopStock]) -> Result<(), ChainPostCloseError> {
    let mut codes = HashSet::new();
    for stock in stocks {
        validate_text("stock code", &stock.code)?;
        validate_text("stock name", &stock.name)?;
        if !stock.change_pct.is_finite()
            || !stock.price.is_finite()
            || stock.volume_ratio.is_some_and(|value| !value.is_finite())
            || stock.main_net_yi.is_some_and(|value| !value.is_finite())
        {
            return Err(invalid("stock numeric value"));
        }
        if !codes.insert(stock.code.as_str()) {
            return Err(invalid("duplicate stock code"));
        }
    }
    Ok(())
}

fn validate_source(
    source: &SourceObservation,
    business_date: NaiveDate,
) -> Result<(), ChainPostCloseError> {
    match source.status() {
        SourceStatus::Available | SourceStatus::VerifiedEmpty => {
            for value in [
                source.batch_id(),
                source.source(),
                source.observed_at(),
                source.request_observed_at(),
            ] {
                validate_text(
                    "source evidence",
                    value.ok_or_else(|| invalid("source evidence"))?,
                )?;
            }
            if let Some(source_at) = source.source_at() {
                validate_text("source timestamp", source_at)?;
            }
            if source.provider().is_none() || source.request_date() != Some(business_date) {
                return Err(invalid("source request binding"));
            }
            if source.reason().is_some() {
                return Err(invalid("source reason"));
            }
        }
        SourceStatus::Unavailable => {
            validate_text(
                "source unavailable reason",
                source
                    .reason()
                    .ok_or_else(|| invalid("source unavailable reason"))?,
            )?;
        }
        SourceStatus::Unknown | SourceStatus::NotRequested => {
            return Err(invalid("source status"));
        }
    }
    Ok(())
}

fn validate_concepts(bytes: &str) -> Result<(), ChainPostCloseError> {
    let concepts: Vec<String> = serde_json::from_str(bytes).map_err(|_| invalid("cache JSON"))?;
    if concepts.is_empty() {
        return Err(invalid("cache concepts"));
    }
    for concept in concepts {
        validate_text("cache concept", &concept)?;
    }
    Ok(())
}

fn validate_text(check: &'static str, value: &str) -> Result<(), ChainPostCloseError> {
    if value.trim().is_empty() || value.contains('\0') || value.len() > MAX_TEXT_BYTES {
        return Err(invalid(check));
    }
    Ok(())
}

fn invalid(check: &'static str) -> ChainPostCloseError {
    ChainPostCloseError::InvalidInput { check }
}
