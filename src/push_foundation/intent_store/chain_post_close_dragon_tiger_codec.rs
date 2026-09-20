use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, FixedOffset, NaiveDate};
use prost::Message as _;
use serde::{Deserialize, Serialize};

use crate::data_gateway::grpc_source::{BoardAttemptCompletion, RestoredDragonTigerRequest};
use crate::data_gateway::review::OwnedGatewayAuditRecord;
use crate::data_gateway::review::{store_gateway_error, StoredGatewayError};
use crate::data_gateway::{BatchEvidence, DragonTigerSeatReview, DragonTigerSourceDisclosure};
use crate::data_gateway::{DragonTigerStockReview, GatewayBatch, GatewayError};
use crate::grpc_client::client::ContractProfile;
use crate::grpc_client::pb::magic::market::v1::QueryRequest;
use crate::market_domain::{DragonTigerSide, Exchange, ProviderId};
use crate::pipeline::chain_analysis::preparation::SourceObservation;

use super::positions::PositionStageCompletion;
use super::ChainPostCloseError;

#[derive(Deserialize, Serialize)]
struct ParentEnvelope {
    schema_version: u32,
    kind: String,
    run_id: String,
    context_digest: String,
    input_digest: String,
    positions_version: u64,
    positions_digest: String,
    positions_owner: String,
    positions_generation: u64,
    positions_time: i64,
    concepts_version: Option<u64>,
    concepts_digest: Option<String>,
    concepts_owner: Option<String>,
    concepts_generation: Option<u64>,
    concepts_time: Option<i64>,
    requested_codes: Vec<String>,
    fetched: Vec<ParentFetchedEnvelope>,
    completion_version: u64,
}

#[derive(Deserialize, Serialize)]
struct ParentFetchedEnvelope {
    ordinal: u64,
    code: String,
    occurrence_version: u64,
    occurrence_digest: String,
    occurrence_owner: String,
    occurrence_generation: u64,
    occurrence_time: i64,
    terminal_version: u64,
    terminal_digest: String,
    terminal_owner: String,
    terminal_generation: u64,
    terminal_time: i64,
    final_version: u64,
    final_digest: String,
    final_owner: String,
    final_generation: u64,
    final_time: i64,
    cache_version: u64,
    cache_digest: String,
    cache_owner: String,
    cache_generation: u64,
    cache_time: i64,
}

#[derive(Clone, Deserialize, Serialize)]
struct RequestEnvelope {
    schema_version: u32,
    date: String,
    observed_at: String,
    local_offset_seconds: i32,
    operation: String,
    disclosure_limit: u32,
    stock_limit: usize,
    request_id: String,
    request_wire: Vec<u8>,
    profile: String,
    acquisition_authority: Option<String>,
    retry_max_attempts: u32,
    retry_base_delay_ms: u64,
    retry_max_delay_ms: u64,
    retry_jitter_ms: u64,
}

pub(super) struct RestoredRequest {
    pub(super) date: NaiveDate,
    pub(super) observed_at: String,
    pub(super) local_offset_seconds: i32,
    pub(super) request_id: String,
    pub(super) request_wire: Vec<u8>,
    pub(super) request: QueryRequest,
    pub(super) profile: ContractProfile,
    pub(super) acquisition_authority: Option<String>,
    pub(super) retry_policy: (u32, u64, u64, u64),
}

impl RestoredRequest {
    pub(super) fn resume(&self, next_attempt: u32) -> RestoredDragonTigerRequest {
        RestoredDragonTigerRequest::new(
            self.date,
            100,
            5_000,
            self.request_id.clone(),
            self.request.clone(),
            self.profile,
            self.acquisition_authority.clone(),
            self.retry_policy,
            next_attempt,
        )
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct EvidenceEnvelope {
    provider: ProviderId,
    source: String,
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct SeatEnvelope {
    side: DragonTigerSide,
    rank: u32,
    seat_name: String,
    amount_bits: u64,
    buy_amount_bits: Option<u64>,
    sell_amount_bits: Option<u64>,
    net_amount_bits: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize)]
struct DisclosureEnvelope {
    entry_id: String,
    trade_id: String,
    reason: Option<String>,
    buy_amount_bits: Option<u64>,
    sell_amount_bits: Option<u64>,
    net_amount_bits: Option<u64>,
    turnover_rate_bits: Option<u64>,
    seats: Vec<SeatEnvelope>,
}

#[derive(Clone, Deserialize, Serialize)]
struct StockEnvelope {
    exchange: Exchange,
    code: String,
    ranking_net_amount_bits: u64,
    disclosures: Vec<DisclosureEnvelope>,
}

#[derive(Clone, Deserialize, Serialize)]
enum GatewayEnvelope {
    Available {
        records: Vec<StockEnvelope>,
        evidence: EvidenceEnvelope,
    },
    VerifiedEmpty(EvidenceEnvelope),
    Error(StoredGatewayError),
}

#[derive(Clone, Deserialize, Serialize)]
enum ProjectionEnvelope {
    Available {
        lhb_bits: BTreeMap<String, u64>,
        source: SourceObservation,
    },
    Failed {
        original_reason: String,
    },
}

#[derive(Clone, Deserialize, Serialize)]
struct FinalEnvelope {
    schema_version: u32,
    gateway: GatewayEnvelope,
    projection: ProjectionEnvelope,
}

#[derive(Clone)]
pub(super) enum Projection {
    Available {
        lhb: HashMap<String, f64>,
        source: SourceObservation,
    },
    Failed(String),
}

pub(super) struct Final {
    pub(super) gateway: Result<GatewayBatch<DragonTigerStockReview>, GatewayError>,
    pub(super) projection: Projection,
}

pub(super) fn parent_bytes(
    completion: &PositionStageCompletion,
) -> Result<Vec<u8>, ChainPostCloseError> {
    encode(&ParentEnvelope {
        schema_version: 1,
        kind: completion.kind.as_str().to_owned(),
        run_id: completion.run_id.clone(),
        context_digest: completion.context_digest.clone(),
        input_digest: completion.input_digest.clone(),
        positions_version: completion.positions_version,
        positions_digest: completion.positions_digest.clone(),
        positions_owner: completion.positions_owner.clone(),
        positions_generation: completion.positions_generation,
        positions_time: completion.positions_time,
        concepts_version: completion.concepts_version,
        concepts_digest: completion.concepts_digest.clone(),
        concepts_owner: completion.concepts_owner.clone(),
        concepts_generation: completion.concepts_generation,
        concepts_time: completion.concepts_time,
        requested_codes: completion.requested_codes.clone(),
        fetched: completion
            .fetched
            .iter()
            .map(|fact| ParentFetchedEnvelope {
                ordinal: fact.ordinal,
                code: fact.code.clone(),
                occurrence_version: fact.occurrence_version,
                occurrence_digest: fact.occurrence_digest.clone(),
                occurrence_owner: fact.occurrence_owner.clone(),
                occurrence_generation: fact.occurrence_generation,
                occurrence_time: fact.occurrence_time,
                terminal_version: fact.terminal_version,
                terminal_digest: fact.terminal_digest.clone(),
                terminal_owner: fact.terminal_owner.clone(),
                terminal_generation: fact.terminal_generation,
                terminal_time: fact.terminal_time,
                final_version: fact.final_version,
                final_digest: fact.final_digest.clone(),
                final_owner: fact.final_owner.clone(),
                final_generation: fact.final_generation,
                final_time: fact.final_time,
                cache_version: fact.cache_version,
                cache_digest: fact.cache_digest.clone(),
                cache_owner: fact.cache_owner.clone(),
                cache_generation: fact.cache_generation,
                cache_time: fact.cache_time,
            })
            .collect(),
        completion_version: completion.completion_version,
    })
}

pub(super) fn validate_parent_bytes(
    bytes: &[u8],
    completion: &PositionStageCompletion,
) -> Result<(), ChainPostCloseError> {
    if parent_bytes(completion)? == bytes {
        Ok(())
    } else {
        Err(ChainPostCloseError::SchemaRejected)
    }
}

pub(super) fn request_bytes(
    date: NaiveDate,
    observed_at: String,
    local_offset_seconds: i32,
    request_id: &str,
    request_wire: Vec<u8>,
    profile: &str,
    acquisition_authority: Option<&str>,
    retry_policy: (u32, u64, u64, u64),
) -> Result<Vec<u8>, ChainPostCloseError> {
    encode(&RequestEnvelope {
        schema_version: 1,
        date: date.format("%Y-%m-%d").to_string(),
        observed_at,
        local_offset_seconds,
        operation: "DragonTiger".to_owned(),
        disclosure_limit: 100,
        stock_limit: 5_000,
        request_id: request_id.to_owned(),
        request_wire,
        profile: profile.to_owned(),
        acquisition_authority: acquisition_authority.map(str::to_owned),
        retry_max_attempts: retry_policy.0,
        retry_base_delay_ms: retry_policy.1,
        retry_max_delay_ms: retry_policy.2,
        retry_jitter_ms: retry_policy.3,
    })
}

pub(super) fn decode_request(bytes: &[u8]) -> Result<RestoredRequest, ChainPostCloseError> {
    let value: RequestEnvelope = decode(bytes)?;
    let date = NaiveDate::parse_from_str(&value.date, "%Y-%m-%d")
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let observed = DateTime::<FixedOffset>::parse_from_rfc3339(&value.observed_at)
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let request = QueryRequest::decode(value.request_wire.as_slice())
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    let context = request
        .context
        .as_ref()
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    let payload = request
        .payload
        .as_ref()
        .ok_or(ChainPostCloseError::SchemaRejected)?;
    let expected = serde_json::json!({
        "date": value.date,
        "disclosure_limit": 100,
        "stock_limit": 5_000,
    });
    let actual: serde_json::Value =
        serde_json::from_slice(&payload.data).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if value.schema_version != 1
        || value.operation != "DragonTiger"
        || value.disclosure_limit != 100
        || value.stock_limit != 5_000
        || value.request_id.is_empty()
        || observed.date_naive() != date
        || observed.offset().local_minus_utc() != value.local_offset_seconds
        || value.retry_max_attempts == 0
        || QueryRequest::encode_to_vec(&request) != value.request_wire
        || context.protocol_version != 1
        || context.request_id != value.request_id
        || payload.schema != "market.dragon_tiger"
        || payload.schema_version != 1
        || payload.content_type != "application/json; charset=utf-8"
        || actual != expected
        || !request.preferred_provider.is_empty()
        || request.allow_unadmitted
    {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let profile = match value.profile.as_str() {
        "LocalBridgeV1" => ContractProfile::LocalBridgeV1,
        _ => return Err(ChainPostCloseError::SchemaRejected),
    };
    Ok(RestoredRequest {
        date,
        observed_at: value.observed_at,
        local_offset_seconds: value.local_offset_seconds,
        request_id: value.request_id,
        request_wire: value.request_wire,
        request,
        profile,
        acquisition_authority: value.acquisition_authority,
        retry_policy: (
            value.retry_max_attempts,
            value.retry_base_delay_ms,
            value.retry_max_delay_ms,
            value.retry_jitter_ms,
        ),
    })
}

pub(super) fn result_bytes(
    completion: &BoardAttemptCompletion,
) -> Result<Vec<u8>, ChainPostCloseError> {
    super::board_codec::result_bytes(completion)
}

pub(super) fn decode_result(
    bytes: &[u8],
) -> Result<super::board_codec::ResultEnvelope, ChainPostCloseError> {
    super::board_codec::decode_result(bytes)
}

pub(super) fn status_bytes(diagnostic: Option<&str>) -> Result<Vec<u8>, ChainPostCloseError> {
    super::board_error_codec::status_bytes(diagnostic)
}

pub(super) fn decode_status(bytes: &[u8]) -> Result<Option<String>, ChainPostCloseError> {
    super::board_error_codec::decode_status(bytes)
}

pub(super) fn error_bytes(
    gateway: StoredGatewayError,
    audit: OwnedGatewayAuditRecord,
) -> Result<Vec<u8>, ChainPostCloseError> {
    super::board_error_codec::error_bytes(gateway, audit)
}

pub(super) fn decode_error(
    bytes: &[u8],
) -> Result<(GatewayError, OwnedGatewayAuditRecord), ChainPostCloseError> {
    let (stored, audit) = super::board_error_codec::decode_error(bytes)?;
    let error = crate::data_gateway::dragon_tiger::restore_dragon_tiger_gateway_error(&stored)
        .map_err(|_| ChainPostCloseError::SchemaRejected)?;
    Ok((error, audit))
}

pub(super) fn final_bytes(
    gateway: &Result<GatewayBatch<DragonTigerStockReview>, GatewayError>,
    projection: &Projection,
) -> Result<Vec<u8>, ChainPostCloseError> {
    let gateway = match gateway {
        Ok(GatewayBatch::Available { records, evidence }) => GatewayEnvelope::Available {
            records: records.iter().map(StockEnvelope::from).collect(),
            evidence: EvidenceEnvelope::from(evidence),
        },
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => {
            GatewayEnvelope::VerifiedEmpty(EvidenceEnvelope::from(evidence))
        }
        Err(error) => GatewayEnvelope::Error(store_gateway_error(error)),
    };
    let projection = match projection {
        Projection::Available { lhb, source } => ProjectionEnvelope::Available {
            lhb_bits: lhb
                .iter()
                .map(|(code, value)| (code.clone(), value.to_bits()))
                .collect(),
            source: source.clone(),
        },
        Projection::Failed(reason) => ProjectionEnvelope::Failed {
            original_reason: reason.clone(),
        },
    };
    encode(&FinalEnvelope {
        schema_version: 1,
        gateway,
        projection,
    })
}

pub(super) fn decode_final(bytes: &[u8]) -> Result<Final, ChainPostCloseError> {
    let value: FinalEnvelope = decode(bytes)?;
    if value.schema_version != 1 {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    let gateway = match value.gateway {
        GatewayEnvelope::Available { records, evidence } => Ok(GatewayBatch::Available {
            records: records
                .into_iter()
                .map(StockEnvelope::into_record)
                .collect::<Result<_, _>>()?,
            evidence: evidence.into_evidence()?,
        }),
        GatewayEnvelope::VerifiedEmpty(evidence) => {
            Ok(GatewayBatch::VerifiedEmpty(evidence.into_evidence()?))
        }
        GatewayEnvelope::Error(stored) => Err(
            crate::data_gateway::dragon_tiger::restore_dragon_tiger_gateway_error(&stored)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?,
        ),
    };
    let projection = match value.projection {
        ProjectionEnvelope::Available { lhb_bits, source } => Projection::Available {
            lhb: lhb_bits
                .into_iter()
                .map(|(code, bits)| finite(bits).map(|value| (code, value)))
                .collect::<Result<_, _>>()?,
            source,
        },
        ProjectionEnvelope::Failed { original_reason } if !original_reason.is_empty() => {
            Projection::Failed(original_reason)
        }
        ProjectionEnvelope::Failed { .. } => return Err(ChainPostCloseError::SchemaRejected),
    };
    Ok(Final {
        gateway,
        projection,
    })
}

fn finite(bits: u64) -> Result<f64, ChainPostCloseError> {
    let value = f64::from_bits(bits);
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ChainPostCloseError::SchemaRejected)
    }
}

impl From<&BatchEvidence> for EvidenceEnvelope {
    fn from(value: &BatchEvidence) -> Self {
        Self {
            provider: value.provider,
            source: value.source.clone(),
            source_at: value.source_at.clone(),
            observed_at: value.observed_at.clone(),
            batch_id: value.batch_id.clone(),
        }
    }
}

impl EvidenceEnvelope {
    fn into_evidence(self) -> Result<BatchEvidence, ChainPostCloseError> {
        if self.source.trim().is_empty()
            || self.observed_at.trim().is_empty()
            || self.batch_id.trim().is_empty()
        {
            return Err(ChainPostCloseError::SchemaRejected);
        }
        Ok(BatchEvidence {
            provider: self.provider,
            source: self.source,
            source_at: self.source_at,
            observed_at: self.observed_at,
            batch_id: self.batch_id,
        })
    }
}

impl From<&DragonTigerStockReview> for StockEnvelope {
    fn from(value: &DragonTigerStockReview) -> Self {
        Self {
            exchange: value.exchange,
            code: value.code.clone(),
            ranking_net_amount_bits: value.ranking_net_amount_yuan.to_bits(),
            disclosures: value
                .disclosures
                .iter()
                .map(DisclosureEnvelope::from)
                .collect(),
        }
    }
}

impl StockEnvelope {
    fn into_record(self) -> Result<DragonTigerStockReview, ChainPostCloseError> {
        Ok(DragonTigerStockReview {
            exchange: self.exchange,
            code: self.code,
            ranking_net_amount_yuan: finite(self.ranking_net_amount_bits)?,
            disclosures: self
                .disclosures
                .into_iter()
                .map(DisclosureEnvelope::into_record)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl From<&DragonTigerSourceDisclosure> for DisclosureEnvelope {
    fn from(value: &DragonTigerSourceDisclosure) -> Self {
        Self {
            entry_id: value.entry_id.clone(),
            trade_id: value.trade_id.clone(),
            reason: value.reason.clone(),
            buy_amount_bits: value.buy_amount_yuan.map(f64::to_bits),
            sell_amount_bits: value.sell_amount_yuan.map(f64::to_bits),
            net_amount_bits: value.net_amount_yuan.map(f64::to_bits),
            turnover_rate_bits: value.turnover_rate_pct.map(f64::to_bits),
            seats: value.seats.iter().map(SeatEnvelope::from).collect(),
        }
    }
}

impl DisclosureEnvelope {
    fn into_record(self) -> Result<DragonTigerSourceDisclosure, ChainPostCloseError> {
        Ok(DragonTigerSourceDisclosure {
            entry_id: self.entry_id,
            trade_id: self.trade_id,
            reason: self.reason,
            buy_amount_yuan: self.buy_amount_bits.map(finite).transpose()?,
            sell_amount_yuan: self.sell_amount_bits.map(finite).transpose()?,
            net_amount_yuan: self.net_amount_bits.map(finite).transpose()?,
            turnover_rate_pct: self.turnover_rate_bits.map(finite).transpose()?,
            seats: self
                .seats
                .into_iter()
                .map(SeatEnvelope::into_record)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl From<&DragonTigerSeatReview> for SeatEnvelope {
    fn from(value: &DragonTigerSeatReview) -> Self {
        Self {
            side: value.side,
            rank: value.rank,
            seat_name: value.seat_name.clone(),
            amount_bits: value.amount_yuan.to_bits(),
            buy_amount_bits: value.buy_amount_yuan.map(f64::to_bits),
            sell_amount_bits: value.sell_amount_yuan.map(f64::to_bits),
            net_amount_bits: value.net_amount_yuan.map(f64::to_bits),
        }
    }
}

impl SeatEnvelope {
    fn into_record(self) -> Result<DragonTigerSeatReview, ChainPostCloseError> {
        Ok(DragonTigerSeatReview {
            side: self.side,
            rank: self.rank,
            seat_name: self.seat_name,
            amount_yuan: finite(self.amount_bits)?,
            buy_amount_yuan: self.buy_amount_bits.map(finite).transpose()?,
            sell_amount_yuan: self.sell_amount_bits.map(finite).transpose()?,
            net_amount_yuan: self.net_amount_bits.map(finite).transpose()?,
        })
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ChainPostCloseError> {
    serde_json::to_vec(value).map_err(|_| ChainPostCloseError::SchemaRejected)
}

fn decode<T>(bytes: &[u8]) -> Result<T, ChainPostCloseError>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    let value = serde_json::from_slice(bytes).map_err(|_| ChainPostCloseError::SchemaRejected)?;
    if encode(&value)? != bytes {
        return Err(ChainPostCloseError::SchemaRejected);
    }
    Ok(value)
}
