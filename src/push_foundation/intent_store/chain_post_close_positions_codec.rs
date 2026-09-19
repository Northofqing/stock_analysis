use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::database::OwnedPositionSourceRow;
use crate::monitor::push_job::UtcMicros;
use crate::pipeline::chain_analysis::preparation::PositionInput;

use super::ChainPostCloseError;

const POSITIONS_SOURCE: &str = "stock_position/open/buy_date_desc/v1";
const CACHE_SOURCE: &str = "stock_concepts/updated_at_gte_local_7d/v1";

#[derive(Clone)]
pub(super) struct PositionCacheSourceRow {
    pub(super) code: String,
    pub(super) concepts: String,
    pub(super) updated_at: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionsEnvelope {
    schema_version: u32,
    source_contract: String,
    business_date: String,
    query_observed_at: i64,
    rows: Vec<PositionRowEnvelope>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionRowEnvelope {
    id: i32,
    code: String,
    name: String,
    buy_date: String,
    buy_price_bits: String,
    quantity: i32,
    status: String,
    sell_date: Option<String>,
    sell_price_bits: Option<String>,
    return_rate_bits: Option<String>,
    created_at: String,
    updated_at: String,
    chain_name: Option<String>,
    st_type: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionConceptEnvelope {
    schema_version: u32,
    source_contract: String,
    query_observed_at: i64,
    local_offset_seconds: i32,
    cutoff_local_offset_seconds: i32,
    cache_cutoff: String,
    positions_run_version: u64,
    positions_sha256: String,
    requested_codes: Vec<String>,
    cache_rows: Vec<CacheRowEnvelope>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheRowEnvelope {
    code: String,
    concepts: String,
    updated_at: String,
}

pub(super) struct DecodedPositions {
    pub(super) business_date: String,
    pub(super) observed_at: UtcMicros,
    pub(super) rows: Vec<OwnedPositionSourceRow>,
}

impl DecodedPositions {
    pub(super) fn projection(&self) -> Vec<PositionInput> {
        self.rows
            .iter()
            .map(|row| PositionInput::new(row.code.clone(), row.name.clone(), row.return_rate))
            .collect()
    }

    pub(super) fn requested_codes(&self) -> Vec<String> {
        self.rows.iter().map(|row| row.code.clone()).collect()
    }
}

pub(super) struct DecodedPositionConcepts {
    pub(super) observed_at: UtcMicros,
    pub(super) local_offset_seconds: i32,
    pub(super) cutoff_local_offset_seconds: i32,
    pub(super) cache_cutoff: String,
    pub(super) positions_run_version: u64,
    pub(super) positions_sha256: String,
    pub(super) requested_codes: Vec<String>,
    pub(super) cache_rows: Vec<PositionCacheSourceRow>,
}

fn encode_bits(value: f64) -> Result<String, ChainPostCloseError> {
    if !value.is_finite() {
        return Err(ChainPostCloseError::InvalidInput {
            check: "position finite number",
        });
    }
    Ok(format!("{:016x}", value.to_bits()))
}

fn decode_bits(value: &str) -> Result<f64, ChainPostCloseError> {
    if value.len() != 16
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let bits = u64::from_str_radix(value, 16).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let decoded = f64::from_bits(bits);
    if !decoded.is_finite() {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(decoded)
}

pub(super) fn encode_positions(
    business_date: NaiveDate,
    observed_at: UtcMicros,
    rows: &[OwnedPositionSourceRow],
) -> Result<Vec<u8>, ChainPostCloseError> {
    if rows.iter().any(|row| {
        !crate::database::valid_sqlite_timestamp(&row.created_at)
            || !crate::database::valid_sqlite_timestamp(&row.updated_at)
    }) {
        return Err(ChainPostCloseError::InvalidInput {
            check: "position timestamp",
        });
    }
    let rows = rows
        .iter()
        .map(|row| {
            Ok(PositionRowEnvelope {
                id: row.id,
                code: row.code.clone(),
                name: row.name.clone(),
                buy_date: row.buy_date.clone(),
                buy_price_bits: encode_bits(row.buy_price)?,
                quantity: row.quantity,
                status: row.status.clone(),
                sell_date: row.sell_date.clone(),
                sell_price_bits: row.sell_price.map(encode_bits).transpose()?,
                return_rate_bits: row.return_rate.map(encode_bits).transpose()?,
                created_at: row.created_at.clone(),
                updated_at: row.updated_at.clone(),
                chain_name: row.chain_name.clone(),
                st_type: row.st_type.clone(),
            })
        })
        .collect::<Result<Vec<_>, ChainPostCloseError>>()?;
    serde_json::to_vec(&PositionsEnvelope {
        schema_version: 1,
        source_contract: POSITIONS_SOURCE.to_owned(),
        business_date: business_date.format("%Y-%m-%d").to_string(),
        query_observed_at: observed_at.get(),
        rows,
    })
    .map_err(|_| ChainPostCloseError::InvalidInput {
        check: "position material codec",
    })
}

pub(super) fn decode_positions(bytes: &[u8]) -> Result<DecodedPositions, ChainPostCloseError> {
    let envelope: PositionsEnvelope =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if envelope.schema_version != 1
        || envelope.source_contract != POSITIONS_SOURCE
        || NaiveDate::parse_from_str(&envelope.business_date, "%Y-%m-%d").is_err()
        || envelope.query_observed_at < 0
        || serde_json::to_vec(&envelope).map_err(|_| ChainPostCloseError::SchemaRejected)? != bytes
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let rows = envelope
        .rows
        .into_iter()
        .map(|row| {
            Ok(OwnedPositionSourceRow {
                id: row.id,
                code: row.code,
                name: row.name,
                buy_date: row.buy_date,
                buy_price: decode_bits(&row.buy_price_bits)?,
                quantity: row.quantity,
                status: row.status,
                sell_date: row.sell_date,
                sell_price: row
                    .sell_price_bits
                    .as_deref()
                    .map(decode_bits)
                    .transpose()?,
                return_rate: row
                    .return_rate_bits
                    .as_deref()
                    .map(decode_bits)
                    .transpose()?,
                created_at: row.created_at,
                updated_at: row.updated_at,
                chain_name: row.chain_name,
                st_type: row.st_type,
            })
        })
        .collect::<Result<Vec<_>, ChainPostCloseError>>()?;
    if rows.iter().any(|row| {
        row.status != "open"
            || !crate::database::valid_sqlite_timestamp(&row.created_at)
            || !crate::database::valid_sqlite_timestamp(&row.updated_at)
    }) || rows
        .windows(2)
        .any(|pair| pair[0].buy_date.as_str() < pair[1].buy_date.as_str())
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(DecodedPositions {
        business_date: envelope.business_date,
        observed_at: UtcMicros::try_new(envelope.query_observed_at)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?,
        rows,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn encode_position_concepts(
    observed_at: UtcMicros,
    local_offset_seconds: i32,
    cutoff_local_offset_seconds: i32,
    cache_cutoff: String,
    positions_run_version: u64,
    positions_sha256: String,
    requested_codes: Vec<String>,
    cache_rows: &[PositionCacheSourceRow],
) -> Result<Vec<u8>, ChainPostCloseError> {
    serde_json::to_vec(&PositionConceptEnvelope {
        schema_version: 1,
        source_contract: CACHE_SOURCE.to_owned(),
        query_observed_at: observed_at.get(),
        local_offset_seconds,
        cutoff_local_offset_seconds,
        cache_cutoff,
        positions_run_version,
        positions_sha256,
        requested_codes,
        cache_rows: cache_rows
            .iter()
            .map(|row| CacheRowEnvelope {
                code: row.code.clone(),
                concepts: row.concepts.clone(),
                updated_at: row.updated_at.clone(),
            })
            .collect(),
    })
    .map_err(|_| ChainPostCloseError::InvalidInput {
        check: "position concept material codec",
    })
}

pub(super) fn decode_position_concepts(
    bytes: &[u8],
) -> Result<DecodedPositionConcepts, ChainPostCloseError> {
    let envelope: PositionConceptEnvelope =
        serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if envelope.schema_version != 1
        || envelope.source_contract != CACHE_SOURCE
        || envelope.query_observed_at < 0
        || envelope.positions_run_version < 1
        || !valid_digest(&envelope.positions_sha256)
        || serde_json::to_vec(&envelope).map_err(|_| ChainPostCloseError::SchemaRejected)? != bytes
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(DecodedPositionConcepts {
        observed_at: UtcMicros::try_new(envelope.query_observed_at)
            .map_err(|_| ChainPostCloseError::SchemaRejected)?,
        local_offset_seconds: envelope.local_offset_seconds,
        cutoff_local_offset_seconds: envelope.cutoff_local_offset_seconds,
        cache_cutoff: envelope.cache_cutoff,
        positions_run_version: envelope.positions_run_version,
        positions_sha256: envelope.positions_sha256,
        requested_codes: envelope.requested_codes,
        cache_rows: envelope
            .cache_rows
            .into_iter()
            .map(|row| PositionCacheSourceRow {
                code: row.code,
                concepts: row.concepts,
                updated_at: row.updated_at,
            })
            .collect(),
    })
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
