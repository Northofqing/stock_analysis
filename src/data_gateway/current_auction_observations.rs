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
    auction_pct: Option<f64>,
    auction_volume_shares: Option<f64>,
    auction_amount: Option<f64>,
    auction_unmatched: f64,
    auction_turnover_pct: Option<f64>,
    auction_volume_ratio: Option<f64>,
    auction_yesterday_ratio_pct: Option<f64>,
    float_market_cap: Option<f64>,
    last_price: Option<f64>,
    open_price: Option<f64>,
    evidence: SourceEvidence,
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::HithinkFinance), message)
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
        let wire: AuctionWire = serde_json::from_value(value)
            .map_err(|error| invalid(format!("auction record fields invalid: {error}")))?;
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
}
