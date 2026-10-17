//! ExternalV1 CurrentAuctionObservations: source-time-free auction facts.

use crate::data_gateway::{BatchEvidence, GatewayBatch, GatewayError};
use crate::grpc_client::envelope::{QueryAdmission, QueryResult};
use crate::market_domain::{EvidenceTimestamp, InstrumentId, ProviderId, SourceEvidence};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const CAPABILITY: &str = "CurrentAuctionObservations";
const RECORD_SCHEMA: &str = "magic.market.current_auction_observation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AuctionStage {
    Live,
    Final,
}

#[derive(Debug, Clone)]
pub struct CurrentAuctionRequest {
    instruments: Vec<InstrumentId>,
    stage: AuctionStage,
}

impl CurrentAuctionRequest {
    pub fn new(instruments: Vec<InstrumentId>, stage: AuctionStage) -> Result<Self, GatewayError> {
        if instruments.is_empty()
            || instruments.iter().collect::<HashSet<_>>().len() != instruments.len()
            || instruments.iter().any(|instrument| {
                instrument.asset_class() != crate::market_domain::AssetClass::Equity
            })
        {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "auction instruments must be nonempty, unique equities",
            ));
        }
        Ok(Self { instruments, stage })
    }

    pub fn instruments(&self) -> &[InstrumentId] {
        &self.instruments
    }

    pub fn stage(&self) -> AuctionStage {
        self.stage
    }

    pub(crate) fn params(&self) -> serde_json::Value {
        serde_json::json!({"instruments": self.instruments, "stage": self.stage})
    }
}

/// Provider-native auction values. A missing source time or trading date is
/// never inferred from the response observation time.
#[derive(Debug, Clone)]
pub struct CurrentAuctionObservation {
    pub instrument: InstrumentId,
    pub name: Option<String>,
    pub requested_stage: AuctionStage,
    pub auction_phase: String,
    pub data_status: String,
    pub auction_price: Option<f64>,
    pub pre_close_price: Option<f64>,
    pub auction_pct: Option<f64>,
    pub auction_volume_shares: Option<f64>,
    pub auction_amount: Option<f64>,
    pub auction_unmatched: f64,
    pub auction_turnover_pct: Option<f64>,
    pub auction_volume_ratio: Option<f64>,
    pub auction_yesterday_ratio_pct: Option<f64>,
    pub float_market_cap: Option<f64>,
    pub last_price: Option<f64>,
    pub open_price: Option<f64>,
    pub evidence: SourceEvidence,
}

/// 2026-10-17（响应 Windows 侧评估回执 §1.3）：
///
/// op 61 的 **4 个比率字段**实测返回 `{"value": f64, "unit": "Percent"|"Decimal"}`
/// 映射形态 —— `auction_pct` / `auction_turnover_pct` / `auction_volume_ratio` /
/// `auction_yesterday_ratio_pct`；其余 8 个数值字段（auction_price / pre_close_price /
/// auction_volume_shares / auction_amount / auction_unmatched / float_market_cap /
/// last_price / open_price）为裸 f64 或 null。这是全 API 的既有形状（对端引
/// `grpc-external-api.md` 同类字段），并非 live 特有 —— `stage=live` 与 `stage=final`
/// 由同一构造器产出，字段形态一致。
///
/// 原始解析器把这 4 个声明成 `Option<f64>`，遇 map 即报
/// `invalid type: map, expected f64`（2026-10-16 早盘竞价实测）。
/// 本 deserializer 同时接受 **裸值 / {value,unit} / null**，统一归一为 f64（只取 value，
/// unit 语义由字段名承载）。这样修不动 wire 合同、也不必改任何消费方。
mod ratio_f64 {
    use serde::{Deserialize, Deserializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Ratio {
        Scalar(f64),
        WithUnit {
            value: f64,
            #[allow(dead_code)]
            #[serde(default)]
            unit: Option<String>,
        },
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Option::<Ratio>::deserialize(deserializer)?;
        Ok(raw.map(|ratio| match ratio {
            Ratio::Scalar(value) => value,
            Ratio::WithUnit { value, .. } => value,
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuctionWire {
    instrument: InstrumentId,
    name: Option<String>,
    requested_stage: AuctionStage,
    auction_phase: String,
    data_status: String,
    auction_price: Option<f64>,
    pre_close_price: Option<f64>,
    #[serde(default, deserialize_with = "ratio_f64::deserialize")]
    auction_pct: Option<f64>,
    auction_volume_shares: Option<f64>,
    auction_amount: Option<f64>,
    auction_unmatched: f64,
    #[serde(default, deserialize_with = "ratio_f64::deserialize")]
    auction_turnover_pct: Option<f64>,
    #[serde(default, deserialize_with = "ratio_f64::deserialize")]
    auction_volume_ratio: Option<f64>,
    #[serde(default, deserialize_with = "ratio_f64::deserialize")]
    auction_yesterday_ratio_pct: Option<f64>,
    float_market_cap: Option<f64>,
    last_price: Option<f64>,
    open_price: Option<f64>,
    evidence: SourceEvidence,
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::HithinkFinance), message)
}

/// 2026-10-16: serde 的 `invalid type: map, expected f64` 不带字段路径——生产
/// 报错只能看到「auction record fields invalid: invalid type: map, expected f64」,
/// 无法判断是哪个字段、服务端返回的 map 长什么样（live 路径契约不符时的定位盲点）。
/// 解析失败时逐字段复检数值域, 报出首个类型不符的字段名 + 实际类型 + 原值片段
/// （截断 200 字符）。纯观测增强, 不改变接受/拒绝判定。
fn locate_numeric_field_mismatch(
    object: &serde_json::Map<String, serde_json::Value>,
) -> String {
    const NUMERIC_FIELDS: [&str; 12] = [
        "auction_price",
        "pre_close_price",
        "auction_pct",
        "auction_volume_shares",
        "auction_amount",
        "auction_unmatched",
        "auction_turnover_pct",
        "auction_volume_ratio",
        "auction_yesterday_ratio_pct",
        "float_market_cap",
        "last_price",
        "open_price",
    ];
    for field in NUMERIC_FIELDS {
        let Some(value) = object.get(field) else {
            continue;
        };
        if value.is_null() || value.as_f64().is_some() {
            continue;
        }
        let kind = match value {
            serde_json::Value::Bool(_) => "bool",
            serde_json::Value::String(_) => "string",
            serde_json::Value::Array(_) => "array",
            serde_json::Value::Object(_) => "map",
            _ => "other",
        };
        let raw = value.to_string();
        let raw = if raw.chars().count() > 200 {
            format!("{}...", raw.chars().take(200).collect::<String>())
        } else {
            raw
        };
        return format!("field={field} expected=f64 actual={kind} raw={raw}");
    }
    "no numeric field mismatch; failure is in a non-numeric field".to_string()
}

pub(crate) fn convert_response(
    request: &CurrentAuctionRequest,
    response: &QueryResult,
) -> Result<GatewayBatch<CurrentAuctionObservation>, GatewayError> {
    if response.admission != QueryAdmission::Admitted
        || !response.complete
        || !response.diagnostic_blocker.is_empty()
        || response.selected_provider != "HithinkFinance"
        || !response.source_at.is_empty()
        || !response.source().starts_with("grpc-mtls:")
        || response.batch_id.trim().is_empty()
    {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            super::grpc_source::convert::parse_provider(&response.selected_provider).ok(),
            "auction response envelope is not admitted and complete",
        ));
    }
    let batch_time = EvidenceTimestamp::parse_instant(&response.observed_at)
        .map_err(|error| invalid(format!("auction batch observed_at invalid: {error}")))?;
    let evidence = BatchEvidence {
        provider: ProviderId::HithinkFinance,
        source: response.source().to_owned(),
        source_at: None,
        observed_at: response.observed_at.clone(),
        batch_id: response.batch_id.clone(),
    };
    if response.records.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }
    let expected = request.instruments.iter().cloned().collect::<HashSet<_>>();
    let mut seen = HashSet::with_capacity(response.records.len());
    let mut records = Vec::with_capacity(response.records.len());
    for payload in &response.records {
        if payload.schema != RECORD_SCHEMA
            || payload.schema_version != 1
            || payload.content_type != "application/json; charset=utf-8"
        {
            return Err(invalid(
                "auction record schema/version/content type mismatch",
            ));
        }
        let value: serde_json::Value = serde_json::from_slice(&payload.data)
            .map_err(|error| invalid(format!("auction record JSON invalid: {error}")))?;
        let object = value
            .as_object()
            .ok_or_else(|| invalid("auction record is not an object"))?;
        for field in [
            "instrument",
            "name",
            "requested_stage",
            "auction_phase",
            "data_status",
            "auction_price",
            "pre_close_price",
            "auction_pct",
            "auction_volume_shares",
            "auction_amount",
            "auction_unmatched",
            "auction_turnover_pct",
            "auction_volume_ratio",
            "auction_yesterday_ratio_pct",
            "float_market_cap",
            "last_price",
            "open_price",
            "evidence",
        ] {
            if !object.contains_key(field) {
                return Err(invalid(format!("auction record missing {field}")));
            }
        }
        let wire: AuctionWire = serde_json::from_value(value.clone()).map_err(|error| {
            invalid(format!(
                "auction record fields invalid: {error} ({})",
                locate_numeric_field_mismatch(object)
            ))
        })?;
        if !expected.contains(&wire.instrument) || !seen.insert(wire.instrument.clone()) {
            return Err(invalid(
                "auction record instrument missing, unexpected, or duplicate",
            ));
        }
        if wire.requested_stage != request.stage
            || wire.auction_phase.trim().is_empty()
            || wire.data_status.trim().is_empty()
            || wire.evidence.provider() != ProviderId::Tonghuashun
            || wire.evidence.source_at().is_some()
            || wire.evidence.batch_id() != response.batch_id
            || !wire.auction_unmatched.is_finite()
        {
            return Err(invalid(
                "auction stage, provider, evidence, or unmatched value conflicts",
            ));
        }
        let record_time = EvidenceTimestamp::parse_instant(wire.evidence.observed_at())
            .map_err(|error| invalid(format!("auction record observed_at invalid: {error}")))?;
        if record_time > batch_time {
            return Err(invalid(
                "auction record observation is newer than its batch",
            ));
        }
        let numeric = [
            wire.auction_price,
            wire.pre_close_price,
            wire.auction_pct,
            wire.auction_volume_shares,
            wire.auction_amount,
            wire.auction_turnover_pct,
            wire.auction_volume_ratio,
            wire.auction_yesterday_ratio_pct,
            wire.float_market_cap,
            wire.last_price,
            wire.open_price,
        ];
        if numeric
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return Err(invalid("auction numeric field is not finite"));
        }
        records.push(CurrentAuctionObservation {
            instrument: wire.instrument,
            name: wire.name,
            requested_stage: wire.requested_stage,
            auction_phase: wire.auction_phase,
            data_status: wire.data_status,
            auction_price: wire.auction_price,
            pre_close_price: wire.pre_close_price,
            auction_pct: wire.auction_pct,
            auction_volume_shares: wire.auction_volume_shares,
            auction_amount: wire.auction_amount,
            auction_unmatched: wire.auction_unmatched,
            auction_turnover_pct: wire.auction_turnover_pct,
            auction_volume_ratio: wire.auction_volume_ratio,
            auction_yesterday_ratio_pct: wire.auction_yesterday_ratio_pct,
            float_market_cap: wire.float_market_cap,
            last_price: wire.last_price,
            open_price: wire.open_price,
            evidence: wire.evidence,
        });
    }
    if seen != expected {
        return Err(invalid(
            "auction response does not cover every requested instrument",
        ));
    }
    Ok(GatewayBatch::Available { records, evidence })
}

/// Qualified ExternalV1 access to the narrow current auction contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct CurrentAuctionObservationsGateway;

impl CurrentAuctionObservationsGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn fetch(
        &self,
        request: &CurrentAuctionRequest,
    ) -> Result<GatewayBatch<CurrentAuctionObservation>, GatewayError> {
        let request_hash =
            super::review::acquisition_request_hash(CAPABILITY, request.params().to_string());
        let result = match super::grpc_source::bridge_for("CurrentAuctionObservations") {
            Ok(bridge) => bridge.current_auction_observations_async(request).await,
            Err(error) => Err(error),
        };
        super::review::audit_routed_gateway_result(CAPABILITY, &request_hash, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::client::external_query_wire_fixture::ExternalQueryWireFixture;
    use crate::grpc_client::envelope::{
        AcquisitionProvenance, CanonicalRecord, QueryAdmission, QueryResult,
    };
    use crate::market_domain::{AssetClass, Exchange, InstrumentId, ProviderId};
    use serial_test::serial;
    use std::time::Duration;

    #[test]
    fn request_rejects_empty_and_duplicate_instruments() {
        assert!(CurrentAuctionRequest::new(vec![], AuctionStage::Live).is_err());
        let instrument = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity)
            .expect("TEST_CODE equity");
        assert!(CurrentAuctionRequest::new(
            vec![instrument.clone(), instrument],
            AuctionStage::Final,
        )
        .is_err());
    }

    #[test]
    fn live_auction_preserves_null_price_ratio_and_signed_unmatched() {
        let instrument = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity)
            .expect("published example instrument");
        let request = CurrentAuctionRequest::new(vec![instrument], AuctionStage::Live)
            .expect("published request");
        let record = serde_json::json!({
            "instrument": {"exchange":"Shanghai","code":"600519","asset_class":"Equity"},
            "name":"贵州茅台", "requested_stage":"live", "auction_phase":"matching",
            "data_status":"live", "auction_price":null, "pre_close_price":1316.01,
            "auction_pct":null, "auction_volume_shares":0.0, "auction_amount":0.0,
            "auction_unmatched":-321.0, "auction_turnover_pct":null,
            "auction_volume_ratio":null, "auction_yesterday_ratio_pct":null,
            "float_market_cap":1653000000000.0, "last_price":null, "open_price":null,
            "evidence": {"provider":"Tonghuashun", "source_at":null,
                         "observed_at":"unix-ms:1788956044416", "batch_id":"TEST_CODE_AUCTION_BATCH"}
        });
        let response = QueryResult {
            admission: QueryAdmission::Admitted,
            selected_provider: "HithinkFinance".to_owned(),
            batch_id: "TEST_CODE_AUCTION_BATCH".to_owned(),
            complete: true,
            observed_at: "unix-ms:1788956044416".to_owned(),
            source_at: String::new(),
            records: vec![CanonicalRecord {
                schema: "magic.market.current_auction_observation".to_owned(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: serde_json::to_vec(&record).unwrap(),
            }],
            provenance: AcquisitionProvenance::ExternalMtlsAuthority(
                "grpc-mtls:TEST_CODE-auction".to_owned(),
            ),
            diagnostic_blocker: String::new(),
        };
        let batch = convert_response(&request, &response).expect("typed auction batch");
        assert_eq!(batch.evidence().provider, ProviderId::HithinkFinance);
        assert_eq!(batch.evidence().source_at, None);
        let auction = &batch.records()[0];
        assert_eq!(auction.auction_price, None);
        assert_eq!(auction.auction_volume_ratio, None);
        assert_eq!(auction.auction_unmatched, -321.0);
        assert_eq!(auction.evidence.provider(), ProviderId::Tonghuashun);
        assert_eq!(auction.evidence.source_at(), None);

        let mut response = response;
        response.complete = false;
        assert!(convert_response(&request, &response).is_err());
        response.complete = true;
        response.records[0].schema_version = 2;
        assert!(convert_response(&request, &response).is_err());
        response.records[0].schema_version = 1;
        let duplicate = response.records[0].clone();
        response.records.push(duplicate);
        assert!(convert_response(&request, &response).is_err());
        response.records.pop();
        let second = InstrumentId::new(Exchange::Shenzhen, "000001", AssetClass::Equity)
            .expect("TEST_CODE second equity");
        let wider_request = CurrentAuctionRequest::new(
            vec![request.instruments()[0].clone(), second],
            AuctionStage::Live,
        )
        .expect("TEST_CODE wider request");
        assert!(convert_response(&wider_request, &response).is_err());
        response.selected_provider = "Tonghuashun".to_owned();
        assert!(convert_response(&request, &response).is_err());
    }

    #[tokio::test]
    #[serial]
    async fn gateway_queries_qualified_external_auction_without_local_61_collision() {
        let fixture = ExternalQueryWireFixture::bind_qualified_current_auction()
            .await
            .expect("qualified auction fixture");
        let _env = crate::data_gateway::grpc_source::test_grpc_env_guard();
        crate::database::DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", fixture.bundle_path());
        crate::data_gateway::grpc_source::reset_bridge();
        fixture.release_capabilities();
        fixture.release();
        let instrument = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity)
            .expect("published example instrument");
        let request = CurrentAuctionRequest::new(vec![instrument], AuctionStage::Live)
            .expect("auction request");
        let batch = tokio::time::timeout(
            Duration::from_secs(20),
            CurrentAuctionObservationsGateway::new().fetch(&request),
        )
        .await
        .expect("auction gateway deadline")
        .expect("qualified auction batch");
        assert_eq!(batch.records().len(), 1);
        assert_eq!(batch.records()[0].auction_unmatched, -321.0);
        assert_eq!(batch.evidence().provider, ProviderId::HithinkFinance);
        let observed = fixture.snapshot();
        assert_eq!(observed.capabilities_calls, 1);
        assert_eq!(observed.methods, ["current_auction_observations"]);
        crate::data_gateway::grpc_source::reset_bridge();
        fixture.finish().await.expect("auction fixture cleanup");
    }

    /// 2026-10-17: op 61 的 4 个比率字段实测为 `{"value","unit"}` 映射
    /// （对端评估回执 §1.3），原解析器声明成 `Option<f64>` 即报
    /// `invalid type: map, expected f64`。本测试锁住宽松解析的四种输入形态，
    /// 并确认其他类型仍报错（不掩盖真问题）。
    #[test]
    fn ratio_f64_accepts_scalar_map_null_and_missing() {
        #[derive(serde::Deserialize)]
        struct Probe {
            #[serde(default, deserialize_with = "ratio_f64::deserialize")]
            value: Option<f64>,
        }

        // ① 裸标量（其余 8 个数值字段的形态）
        let scalar: Probe = serde_json::from_str(r#"{"value": 1.25}"#).expect("scalar");
        assert_eq!(scalar.value, Some(1.25));

        // ② {"value","unit"} 映射（4 个比率字段的形态）
        let mapped: Probe =
            serde_json::from_str(r#"{"value": {"value": 1.25, "unit": "Percent"}}"#)
                .expect("map with unit");
        assert_eq!(mapped.value, Some(1.25));

        // ③ 显式 null
        let null: Probe = serde_json::from_str(r#"{"value": null}"#).expect("null");
        assert_eq!(null.value, None);

        // ④ 字段缺失
        let missing: Probe = serde_json::from_str("{}").expect("missing");
        assert_eq!(missing.value, None);

        // ⑤ 其他类型仍必须失败（避免把真错误静默成 None）
        assert!(serde_json::from_str::<Probe>(r#"{"value": "abc"}"#).is_err());
        assert!(serde_json::from_str::<Probe>(r#"{"value": {"unit": "Percent"}}"#).is_err());
    }
}
