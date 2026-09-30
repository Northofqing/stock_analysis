//! Lossless admission of the delivered ExternalV1 flow records.
//!
//! These facts are separate from the LocalBridgeV1 projections: a money-flow
//! source is a date, while a board-flow row carries interval, five amounts,
//! ratios, and record-level provenance. No consumer route uses these converters
//! until a revision-bound server build has passed the flow acceptance gate.

use super::{BatchEvidence, BoardKind, GatewayBatch, GatewayError};
use crate::grpc_client::envelope::{QueryAdmission, QueryResult};
use crate::market_domain::{
    AssetClass, Exchange, FlowInterval, InstrumentId, ProviderId, SourceEvidence,
};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::Deserialize;
use std::collections::HashSet;

const MONEY_CAPABILITY: &str = "MarketMoneyFlows";
const BOARD_CAPABILITY: &str = "BoardFlows";
const MONEY_SCHEMA: &str = "magic.market.money_flow";
const BOARD_SCHEMA: &str = "magic.market.board_flow";
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const MAX_OBSERVATION_AGE: chrono::TimeDelta = chrono::TimeDelta::seconds(30);
const MAX_CLOCK_SKEW: chrono::TimeDelta = chrono::TimeDelta::seconds(2);

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalMoneyFlowPoint {
    pub instrument: InstrumentId,
    pub source_date: NaiveDate,
    pub main_net: f64,
    pub super_large_net: f64,
    pub large_net: f64,
    pub medium_net: f64,
    pub small_net: f64,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum FlowRatioUnit {
    Percent,
    Decimal,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FlowRatio {
    pub value: f64,
    pub unit: FlowRatioUnit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalBoardFlowRow {
    pub board_code: String,
    pub board_name: String,
    pub category: BoardKind,
    pub interval: FlowInterval,
    pub rank: u32,
    pub return_ratio: Option<FlowRatio>,
    pub main_net: Option<f64>,
    pub super_large_net: Option<f64>,
    pub large_net: Option<f64>,
    pub medium_net: Option<f64>,
    pub small_net: Option<f64>,
    pub leader_instrument: Option<InstrumentId>,
    pub leader_name: Option<String>,
    pub leader_return_ratio: Option<FlowRatio>,
    pub evidence: SourceEvidence,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireInstrument {
    exchange: Exchange,
    code: String,
    asset_class: AssetClass,
}

impl WireInstrument {
    fn into_equity(self, capability: &'static str) -> Result<InstrumentId, GatewayError> {
        if self.asset_class != AssetClass::Equity
            || !matches!(self.exchange, Exchange::Shanghai | Exchange::Shenzhen)
        {
            return Err(invalid(
                capability,
                "flow instrument must be a Shanghai/Shenzhen equity",
            ));
        }
        let code = self.code;
        let instrument = InstrumentId::new(self.exchange, &code, self.asset_class)
            .map_err(|error| invalid(capability, format!("invalid flow instrument: {error}")))?;
        if instrument.code() != code {
            return Err(invalid(capability, "flow instrument code is not canonical"));
        }
        Ok(instrument)
    }
}

#[derive(Debug, Deserialize)]
enum MoneyStatus {
    Available,
    Unavailable,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoneyWire {
    instrument: WireInstrument,
    main_net: Option<f64>,
    super_large_net: Option<f64>,
    large_net: Option<f64>,
    medium_net: Option<f64>,
    small_net: Option<f64>,
    status: MoneyStatus,
    source_at: Option<NaiveDate>,
    observed_at: String,
    provider: ProviderId,
    batch_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardWire {
    board_code: String,
    board_name: String,
    category: String,
    interval: FlowInterval,
    rank: u32,
    return_ratio: Option<FlowRatio>,
    main_net: Option<f64>,
    super_large_net: Option<f64>,
    large_net: Option<f64>,
    medium_net: Option<f64>,
    small_net: Option<f64>,
    leader_instrument: Option<WireInstrument>,
    leader_name: Option<String>,
    leader_return_ratio: Option<FlowRatio>,
    evidence: SourceEvidence,
}

fn invalid(capability: &'static str, detail: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(capability, Some(ProviderId::Eastmoney), detail)
}

fn admitted_envelope(
    q: &QueryResult,
    capability: &'static str,
    now: DateTime<Utc>,
) -> Result<(BatchEvidence, DateTime<Utc>), GatewayError> {
    if q.admission != QueryAdmission::Admitted
        || !q.complete
        || !q.diagnostic_blocker.is_empty()
        || q.selected_provider != "Eastmoney"
        || q.source()
            .strip_prefix("grpc-mtls:")
            .is_none_or(str::is_empty)
        || q.batch_id.trim().is_empty()
        || q.source_at.is_empty()
    {
        return Err(invalid(
            capability,
            "ExternalV1 flow envelope is not admitted and complete",
        ));
    }
    let observed_at = super::parse_evidence_instant(
        capability,
        ProviderId::Eastmoney,
        "observed_at",
        &q.observed_at,
    )?;
    let age = now.signed_duration_since(observed_at);
    if age < -MAX_CLOCK_SKEW || age > MAX_OBSERVATION_AGE {
        return Err(GatewayError::classified(
            capability,
            Some(ProviderId::Eastmoney),
            "stale",
            "observation_stale",
            true,
            format!(
                "flow observation age_ms={} exceeds admission window",
                age.num_milliseconds()
            ),
        ));
    }
    Ok((
        BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: q.source().to_owned(),
            source_at: Some(q.source_at.clone()),
            observed_at: q.observed_at.clone(),
            batch_id: q.batch_id.clone(),
        },
        observed_at,
    ))
}

fn require_record_shape(
    value: &serde_json::Value,
    fields: &[&str],
    capability: &'static str,
) -> Result<(), GatewayError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(capability, "flow record is not a JSON object"))?;
    if let Some(field) = fields.iter().find(|field| !object.contains_key(**field)) {
        return Err(invalid(capability, format!("flow record missing {field}")));
    }
    Ok(())
}

fn parse_payload<T: for<'de> Deserialize<'de>>(
    payload: &crate::grpc_client::envelope::CanonicalRecord,
    schema: &str,
    fields: &[&str],
    capability: &'static str,
) -> Result<T, GatewayError> {
    if payload.schema != schema
        || payload.schema_version != 1
        || payload.content_type != JSON_CONTENT_TYPE
    {
        return Err(invalid(
            capability,
            "flow record schema/version/content type mismatch",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&payload.data)
        .map_err(|error| invalid(capability, format!("flow record JSON invalid: {error}")))?;
    require_record_shape(&value, fields, capability)?;
    serde_json::from_value(value)
        .map_err(|error| invalid(capability, format!("flow record fields invalid: {error}")))
}

fn shanghai_date(time: DateTime<Utc>) -> NaiveDate {
    let offset = FixedOffset::east_opt(8 * 60 * 60).expect("Shanghai UTC offset is valid");
    time.with_timezone(&offset).date_naive()
}

fn require_recent_source_date(
    source_date: NaiveDate,
    observed_at: DateTime<Utc>,
    now: DateTime<Utc>,
    capability: &'static str,
) -> Result<(), GatewayError> {
    let today = shanghai_date(now);
    let oldest = crate::calendar::prev_trading_day(today);
    if source_date < oldest || source_date > today || source_date > shanghai_date(observed_at) {
        return Err(GatewayError::classified(
            capability,
            Some(ProviderId::Eastmoney),
            "stale",
            "daily_source_stale",
            true,
            format!(
                "flow source date {source_date} is outside {oldest}..={}",
                shanghai_date(observed_at)
            ),
        ));
    }
    Ok(())
}

fn finite(value: f64, capability: &'static str, field: &str) -> Result<f64, GatewayError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid(capability, format!("{field} is not finite")))
    }
}

fn required_amount(value: Option<f64>, field: &str) -> Result<f64, GatewayError> {
    finite(
        value.ok_or_else(|| invalid(MONEY_CAPABILITY, format!("{field} missing")))?,
        MONEY_CAPABILITY,
        field,
    )
}

/// One exact request returns one complete daily point. A null/unavailable
/// record is unavailable data, never a verified empty result.
pub fn admit_money_flow(
    requested: &InstrumentId,
    q: &QueryResult,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<ExternalMoneyFlowPoint>, GatewayError> {
    if requested.asset_class() != AssetClass::Equity
        || !matches!(
            requested.exchange(),
            Exchange::Shanghai | Exchange::Shenzhen
        )
    {
        return Err(GatewayError::invalid_request(
            MONEY_CAPABILITY,
            "MoneyFlows requires one Shanghai/Shenzhen equity",
        ));
    }
    let (evidence, observed_at) = admitted_envelope(q, MONEY_CAPABILITY, now)?;
    if q.records.len() != 1 {
        return Err(invalid(
            MONEY_CAPABILITY,
            "MoneyFlows requires exactly one canonical record",
        ));
    }
    let wire: MoneyWire = parse_payload(
        &q.records[0],
        MONEY_SCHEMA,
        &[
            "instrument",
            "main_net",
            "super_large_net",
            "large_net",
            "medium_net",
            "small_net",
            "status",
            "source_at",
            "observed_at",
            "provider",
            "batch_id",
        ],
        MONEY_CAPABILITY,
    )?;
    let instrument = wire.instrument.into_equity(MONEY_CAPABILITY)?;
    if &instrument != requested
        || wire.provider != ProviderId::Eastmoney
        || wire.batch_id != q.batch_id
        || wire.observed_at != q.observed_at
    {
        return Err(invalid(
            MONEY_CAPABILITY,
            "MoneyFlows record scope or provenance conflicts",
        ));
    }
    if matches!(wire.status, MoneyStatus::Unavailable) {
        return Err(GatewayError::classified(
            MONEY_CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unavailable",
            "flow_record_unavailable",
            true,
            "MoneyFlows record says Unavailable",
        ));
    }
    let source_date = wire
        .source_at
        .ok_or_else(|| invalid(MONEY_CAPABILITY, "MoneyFlows source date missing"))?;
    if q.source_at != source_date.to_string() {
        return Err(invalid(
            MONEY_CAPABILITY,
            "MoneyFlows record source date differs from envelope",
        ));
    }
    require_recent_source_date(source_date, observed_at, now, MONEY_CAPABILITY)?;
    let record_evidence =
        SourceEvidence::new(ProviderId::Eastmoney, wire.observed_at, wire.batch_id)
            .and_then(|value| value.with_source_at(source_date.to_string()))
            .map_err(|error| {
                invalid(
                    MONEY_CAPABILITY,
                    format!("MoneyFlows evidence invalid: {error}"),
                )
            })?;
    Ok(GatewayBatch::Available {
        records: vec![ExternalMoneyFlowPoint {
            instrument,
            source_date,
            main_net: required_amount(wire.main_net, "main_net")?,
            super_large_net: required_amount(wire.super_large_net, "super_large_net")?,
            large_net: required_amount(wire.large_net, "large_net")?,
            medium_net: required_amount(wire.medium_net, "medium_net")?,
            small_net: required_amount(wire.small_net, "small_net")?,
            evidence: record_evidence,
        }],
        evidence,
    })
}

fn board_kind(value: &str) -> Option<BoardKind> {
    match value {
        "Industry" => Some(BoardKind::Industry),
        "Concept" => Some(BoardKind::Concept),
        "Region" => Some(BoardKind::Region),
        _ => None,
    }
}

fn optional_finite(value: Option<f64>, field: &str) -> Result<Option<f64>, GatewayError> {
    value
        .map(|number| finite(number, BOARD_CAPABILITY, field))
        .transpose()
}

fn optional_ratio(
    value: Option<FlowRatio>,
    field: &str,
) -> Result<Option<FlowRatio>, GatewayError> {
    value
        .map(|ratio| {
            finite(ratio.value, BOARD_CAPABILITY, field)?;
            Ok(ratio)
        })
        .transpose()
}

/// Admits one bounded ranked page. It does not prove the complete board
/// universe, and never merges pages from separate requests.
pub fn admit_board_flows(
    requested_category: BoardKind,
    requested_interval: FlowInterval,
    limit: u32,
    q: &QueryResult,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<ExternalBoardFlowRow>, GatewayError> {
    if !(1..=200).contains(&limit)
        || !matches!(
            requested_interval,
            FlowInterval::Day1 | FlowInterval::Day5 | FlowInterval::Day10
        )
    {
        return Err(GatewayError::invalid_request(
            BOARD_CAPABILITY,
            "BoardFlows category/interval/limit outside contract",
        ));
    }
    let (evidence, observed_at) = admitted_envelope(q, BOARD_CAPABILITY, now)?;
    if q.records.is_empty() || q.records.len() > limit as usize {
        return Err(invalid(
            BOARD_CAPABILITY,
            "BoardFlows page empty or exceeds request limit",
        ));
    }
    if !q.source_at.bytes().all(|byte| byte.is_ascii_digit())
        || q.source_at
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
    {
        return Err(invalid(
            BOARD_CAPABILITY,
            "BoardFlows source_at must be positive Unix seconds",
        ));
    }
    let source_at = super::parse_evidence_instant(
        BOARD_CAPABILITY,
        ProviderId::Eastmoney,
        "source_at",
        &q.source_at,
    )?;
    if source_at > observed_at {
        return Err(invalid(
            BOARD_CAPABILITY,
            "BoardFlows source_at follows observation",
        ));
    }
    require_recent_source_date(shanghai_date(source_at), observed_at, now, BOARD_CAPABILITY)?;
    let mut seen_codes = HashSet::with_capacity(q.records.len());
    let mut seen_ranks = HashSet::with_capacity(q.records.len());
    let mut records = Vec::with_capacity(q.records.len());
    for payload in &q.records {
        let wire: BoardWire = parse_payload(
            payload,
            BOARD_SCHEMA,
            &[
                "board_code",
                "board_name",
                "category",
                "interval",
                "rank",
                "return_ratio",
                "main_net",
                "super_large_net",
                "large_net",
                "medium_net",
                "small_net",
                "leader_instrument",
                "leader_name",
                "leader_return_ratio",
                "evidence",
            ],
            BOARD_CAPABILITY,
        )?;
        let category = board_kind(&wire.category)
            .ok_or_else(|| invalid(BOARD_CAPABILITY, "unknown board category"))?;
        if category != requested_category
            || wire.interval != requested_interval
            || wire.board_code.trim().is_empty()
            || wire.board_name.trim().is_empty()
            || wire.board_code.trim() != wire.board_code
            || wire.board_name.trim() != wire.board_name
            || wire.rank == 0
            || wire.rank > limit
            || !seen_codes.insert(wire.board_code.clone())
            || !seen_ranks.insert(wire.rank)
            || wire.evidence.provider() != ProviderId::Eastmoney
            || wire.evidence.batch_id() != q.batch_id
            || wire.evidence.observed_at() != q.observed_at
            || wire.evidence.source_at() != Some(q.source_at.as_str())
        {
            return Err(invalid(
                BOARD_CAPABILITY,
                "BoardFlows record scope, rank, or provenance conflicts",
            ));
        }
        let leader_instrument = wire
            .leader_instrument
            .map(|instrument| instrument.into_equity(BOARD_CAPABILITY))
            .transpose()?;
        if wire
            .leader_name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(invalid(BOARD_CAPABILITY, "BoardFlows leader_name is blank"));
        }
        records.push(ExternalBoardFlowRow {
            board_code: wire.board_code,
            board_name: wire.board_name,
            category,
            interval: wire.interval,
            rank: wire.rank,
            return_ratio: optional_ratio(wire.return_ratio, "return_ratio")?,
            main_net: optional_finite(wire.main_net, "main_net")?,
            super_large_net: optional_finite(wire.super_large_net, "super_large_net")?,
            large_net: optional_finite(wire.large_net, "large_net")?,
            medium_net: optional_finite(wire.medium_net, "medium_net")?,
            small_net: optional_finite(wire.small_net, "small_net")?,
            leader_instrument,
            leader_name: wire.leader_name,
            leader_return_ratio: optional_ratio(wire.leader_return_ratio, "leader_return_ratio")?,
            evidence: wire.evidence,
        });
    }
    Ok(GatewayBatch::Available { records, evidence })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::envelope::{AcquisitionProvenance, CanonicalRecord};
    use serde_json::{json, Value};

    fn observed_now(milliseconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(milliseconds + 1_000).expect("captured timestamp")
    }

    fn response(schema: &str, source_at: &str, observed_ms: i64, record: Value) -> QueryResult {
        QueryResult {
            admission: QueryAdmission::Admitted,
            selected_provider: "Eastmoney".into(),
            batch_id: format!(
                "eastmoney-web:{}:unix-ms:{observed_ms}",
                if schema == MONEY_SCHEMA {
                    "fund-flow"
                } else {
                    "board-flow"
                }
            ),
            complete: true,
            observed_at: format!("unix-ms:{observed_ms}"),
            source_at: source_at.into(),
            records: vec![CanonicalRecord {
                schema: schema.into(),
                schema_version: 1,
                content_type: JSON_CONTENT_TYPE.into(),
                data: serde_json::to_vec(&record).expect("test record"),
            }],
            provenance: AcquisitionProvenance::ExternalMtlsAuthority("grpc-mtls:flow-test".into()),
            diagnostic_blocker: String::new(),
        }
    }

    fn money_response() -> QueryResult {
        let observed_ms = 1_790_794_371_033;
        let batch_id = format!("eastmoney-web:fund-flow:unix-ms:{observed_ms}");
        response(
            MONEY_SCHEMA,
            "2026-09-30",
            observed_ms,
            json!({
                "instrument": {"exchange":"Shenzhen", "code":"300005", "asset_class":"Equity"},
                "main_net":4930843.0, "super_large_net":4754088.0, "large_net":176755.0,
                "medium_net":62687144.0, "small_net":-67617987.0,
                "status":"Available", "source_at":"2026-09-30",
                "observed_at":format!("unix-ms:{observed_ms}"),
                "provider":"Eastmoney", "batch_id":batch_id
            }),
        )
    }

    fn board_response() -> QueryResult {
        let observed_ms = 1_790_794_372_052;
        let batch_id = format!("eastmoney-web:board-flow:unix-ms:{observed_ms}");
        response(
            BOARD_SCHEMA,
            "1790753970",
            observed_ms,
            json!({
                "board_code":"BK1216", "board_name":"医药生物", "category":"Industry",
                "interval":"Day1", "rank":1, "return_ratio":{"value":2.03,"unit":"Percent"},
                "main_net":6190539776.0, "super_large_net":3485894656.0,
                "large_net":2704645120.0, "medium_net":-2772193024.0,
                "small_net":-3319524608.0,
                "leader_instrument":{"exchange":"Shanghai","code":"600276","asset_class":"Equity"},
                "leader_name":"恒瑞医药", "leader_return_ratio":null,
                "evidence":{"provider":"Eastmoney", "source_at":"1790753970",
                    "observed_at":format!("unix-ms:{observed_ms}"), "batch_id":batch_id}
            }),
        )
    }

    fn requested_money() -> InstrumentId {
        InstrumentId::new(Exchange::Shenzhen, "300005", AssetClass::Equity).unwrap()
    }

    fn mutate_record(q: &mut QueryResult, field: &str, replacement: Value) {
        let mut value: Value = serde_json::from_slice(&q.records[0].data).unwrap();
        value[field] = replacement;
        q.records[0].data = serde_json::to_vec(&value).unwrap();
    }

    #[test]
    fn recorded_money_point_keeps_date_and_all_five_amounts() {
        let q = money_response();
        let batch =
            admit_money_flow(&requested_money(), &q, observed_now(1_790_794_371_033)).unwrap();
        let point = &batch.records()[0];
        assert_eq!(point.source_date.to_string(), "2026-09-30");
        assert_eq!(point.main_net, 4_930_843.0);
        assert_eq!(point.medium_net, 62_687_144.0);
        assert_eq!(point.small_net, -67_617_987.0);
        assert_eq!(point.evidence.source_at(), Some("2026-09-30"));
        assert_eq!(batch.evidence().batch_id, q.batch_id);
    }

    #[test]
    fn money_flow_rejects_unavailable_incomplete_or_wrong_request() {
        let now = observed_now(1_790_794_371_033);
        let mut unavailable = money_response();
        mutate_record(&mut unavailable, "status", json!("Unavailable"));
        assert_eq!(
            admit_money_flow(&requested_money(), &unavailable, now)
                .unwrap_err()
                .reason_code(),
            "flow_record_unavailable"
        );

        let mut missing_amount = money_response();
        mutate_record(&mut missing_amount, "main_net", Value::Null);
        assert_eq!(
            admit_money_flow(&requested_money(), &missing_amount, now)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let mut duplicate = money_response();
        duplicate.records.push(duplicate.records[0].clone());
        assert_eq!(
            admit_money_flow(&requested_money(), &duplicate, now)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let wrong = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();
        assert_eq!(
            admit_money_flow(&wrong, &money_response(), now)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );
    }

    #[test]
    fn recorded_board_page_keeps_interval_ratios_amounts_and_record_evidence() {
        let q = board_response();
        let batch = admit_board_flows(
            BoardKind::Industry,
            FlowInterval::Day1,
            1,
            &q,
            observed_now(1_790_794_372_052),
        )
        .unwrap();
        let row = &batch.records()[0];
        assert_eq!(row.board_code, "BK1216");
        assert_eq!(
            row.return_ratio.as_ref().unwrap().unit,
            FlowRatioUnit::Percent
        );
        assert_eq!(row.main_net, Some(6_190_539_776.0));
        assert_eq!(row.small_net, Some(-3_319_524_608.0));
        assert_eq!(row.leader_instrument.as_ref().unwrap().code(), "600276");
        assert_eq!(row.evidence.source_at(), Some("1790753970"));
        assert_eq!(batch.evidence().source_at.as_deref(), Some("1790753970"));
    }

    #[test]
    fn board_page_rejects_scope_rank_and_provenance_conflicts() {
        let now = observed_now(1_790_794_372_052);
        let q = board_response();
        assert_eq!(
            admit_board_flows(BoardKind::Concept, FlowInterval::Day1, 1, &q, now)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let mut bad_source = board_response();
        bad_source.source_at = "1790753971".into();
        assert_eq!(
            admit_board_flows(BoardKind::Industry, FlowInterval::Day1, 1, &bad_source, now)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let mut bad_rank = board_response();
        mutate_record(&mut bad_rank, "rank", json!(2));
        assert_eq!(
            admit_board_flows(BoardKind::Industry, FlowInterval::Day1, 1, &bad_rank, now)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );

        let mut bad_ratio = board_response();
        mutate_record(
            &mut bad_ratio,
            "return_ratio",
            json!({"value":2.03,"unit":"Unknown"}),
        );
        assert_eq!(
            admit_board_flows(BoardKind::Industry, FlowInterval::Day1, 1, &bad_ratio, now)
                .unwrap_err()
                .reason_code(),
            "invalid_evidence"
        );
    }
}
