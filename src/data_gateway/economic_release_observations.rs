//! Jin10's admitted rolling release window, separate from EconomicCalendar.

use super::economic_calendar::EconomicReleaseFact;
use super::{BatchEvidence, GatewayBatch, GatewayError};
use crate::grpc_client::envelope::{QueryAdmission, QueryResult};
use crate::market_domain::{ProviderId, SourceEvidence};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use serde::Deserialize;
use std::collections::HashSet;

const CAPABILITY: &str = "EconomicReleaseObservations";
const RECORD_SCHEMA: &str = "magic.market.economic_release_observation";

#[derive(Debug, Clone)]
pub struct EconomicReleaseObservationsRequest {
    limit: u32,
    country: Option<String>,
}

impl EconomicReleaseObservationsRequest {
    pub fn new(limit: u32, country: Option<String>) -> Result<Self, GatewayError> {
        if limit == 0 || country.as_deref().is_some_and(str::is_empty) {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "release limit must be positive and country, when present, nonempty",
            ));
        }
        Ok(Self { limit, country })
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    pub fn country(&self) -> Option<&str> {
        self.country.as_deref()
    }

    pub(crate) fn params(&self) -> serde_json::Value {
        let mut value = serde_json::json!({"limit": self.limit});
        if let Some(country) = &self.country {
            value["country"] = serde_json::Value::String(country.clone());
        }
        value
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseWire {
    event_id: String,
    indicator_id: u32,
    country: String,
    name: String,
    period: Option<String>,
    scheduled_at: String,
    released_at: String,
    previous: Option<String>,
    consensus: Option<String>,
    actual: Option<String>,
    revised: Option<String>,
    unit: Option<String>,
    importance: u32,
    impact: Option<String>,
    evidence: SourceEvidence,
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::Jin10), message)
}

fn parse_release_time(field: &str, value: &str) -> Result<DateTime<Utc>, GatewayError> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|error| invalid(format!("invalid {field} {value:?}: {error}")))
}

fn parse_jin10_source_time(value: &str) -> Result<DateTime<Utc>, GatewayError> {
    let local = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .map_err(|error| invalid(format!("invalid Jin10 source_at {value:?}: {error}")))?;
    let shanghai = FixedOffset::east_opt(8 * 60 * 60).expect("UTC+08:00 is a valid fixed offset");
    shanghai
        .from_local_datetime(&local)
        .single()
        .map(|time| time.with_timezone(&Utc))
        .ok_or_else(|| invalid("Jin10 source_at has no unique instant"))
}

pub(crate) fn convert_response(
    request: &EconomicReleaseObservationsRequest,
    response: &QueryResult,
) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
    if response.admission != QueryAdmission::Admitted
        || !response.complete
        || !response.diagnostic_blocker.is_empty()
        || response.selected_provider != "Jin10"
        || !response.source().starts_with("grpc-mtls:")
        || response.batch_id.trim().is_empty()
    {
        return Err(invalid(
            "Jin10 release response envelope is not admitted and complete",
        ));
    }
    let batch_time = super::evidence_time::parse_evidence_instant(
        CAPABILITY,
        ProviderId::Jin10,
        "observed_at",
        &response.observed_at,
    )?;
    let evidence = BatchEvidence {
        provider: ProviderId::Jin10,
        source: response.source().to_owned(),
        source_at: (!response.source_at.is_empty()).then(|| response.source_at.clone()),
        observed_at: response.observed_at.clone(),
        batch_id: response.batch_id.clone(),
    };
    if response.records.is_empty() {
        if evidence.source_at.is_some() {
            return Err(invalid("empty Jin10 release window has a source_at"));
        }
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }
    if evidence.source_at.is_none() || response.records.len() > request.limit as usize {
        return Err(invalid(
            "nonempty Jin10 release window has no source_at or exceeds limit",
        ));
    }
    let mut seen = HashSet::with_capacity(response.records.len());
    let mut records = Vec::with_capacity(response.records.len());
    for payload in &response.records {
        if payload.schema != RECORD_SCHEMA
            || payload.schema_version != 1
            || payload.content_type != "application/json; charset=utf-8"
        {
            return Err(invalid(
                "Jin10 release record schema/version/content type mismatch",
            ));
        }
        let value: serde_json::Value = serde_json::from_slice(&payload.data)
            .map_err(|error| invalid(format!("Jin10 release record JSON invalid: {error}")))?;
        let object = value
            .as_object()
            .ok_or_else(|| invalid("Jin10 release record is not an object"))?;
        for field in [
            "event_id",
            "indicator_id",
            "country",
            "name",
            "period",
            "scheduled_at",
            "released_at",
            "previous",
            "consensus",
            "actual",
            "revised",
            "unit",
            "importance",
            "impact",
            "evidence",
        ] {
            if !object.contains_key(field) {
                return Err(invalid(format!("Jin10 release record missing {field}")));
            }
        }
        let wire: ReleaseWire = serde_json::from_value(value)
            .map_err(|error| invalid(format!("Jin10 release fields invalid: {error}")))?;
        if wire.event_id.trim().is_empty()
            || wire.country.trim().is_empty()
            || wire.name.trim().is_empty()
            || !seen.insert(wire.event_id.clone())
            || request
                .country
                .as_deref()
                .is_some_and(|country| wire.country != country)
            || wire.evidence.provider() != ProviderId::Jin10
            || wire.evidence.batch_id() != response.batch_id
        {
            return Err(invalid(
                "Jin10 release identity, country, or evidence conflicts",
            ));
        }
        let scheduled_at = parse_release_time("scheduled_at", &wire.scheduled_at)?;
        let released_at = parse_release_time("released_at", &wire.released_at)?;
        let raw_source_at = wire
            .evidence
            .source_at()
            .ok_or_else(|| invalid("Jin10 release record has no original source_at"))?;
        if parse_jin10_source_time(raw_source_at)? != released_at {
            return Err(invalid("Jin10 release source_at differs from released_at"));
        }
        let record_observed = super::evidence_time::parse_evidence_instant(
            CAPABILITY,
            ProviderId::Jin10,
            "record observed_at",
            wire.evidence.observed_at(),
        )?;
        if record_observed > batch_time {
            return Err(invalid(
                "Jin10 release record observation is newer than batch",
            ));
        }
        records.push(EconomicReleaseFact {
            event_id: wire.event_id,
            indicator_id: wire.indicator_id,
            country: wire.country,
            name: wire.name,
            period: wire.period,
            scheduled_at,
            released_at,
            previous: wire.previous,
            consensus: wire.consensus,
            actual: wire.actual,
            revised: wire.revised,
            unit: wire.unit,
            importance: wire.importance,
            impact: wire.impact,
            evidence: wire.evidence,
        });
    }
    let latest = records
        .iter()
        .map(|record| record.released_at.clone())
        .max()
        .expect("nonempty release records");
    if !records.iter().any(|record| {
        record.released_at == latest
            && record.evidence.source_at() == Some(response.source_at.as_str())
    }) {
        return Err(invalid(
            "Jin10 batch source_at is not the latest original record time",
        ));
    }
    Ok(GatewayBatch::Available { records, evidence })
}

/// A rolling type-1 release window; it is not a future economic calendar.
#[derive(Debug, Clone, Copy, Default)]
pub struct EconomicReleaseObservationsGateway;

impl EconomicReleaseObservationsGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn fetch(
        &self,
        request: &EconomicReleaseObservationsRequest,
    ) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
        let request_hash =
            super::review::acquisition_request_hash(CAPABILITY, request.params().to_string());
        let result = match super::grpc_source::bridge_for("EconomicReleaseObservations") {
            Ok(bridge) => bridge.economic_release_observations_async(request).await,
            Err(error) => Err(error),
        };
        super::review::audit_gateway_result(CAPABILITY, ProviderId::Jin10, &request_hash, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::client::external_query_wire_fixture::ExternalQueryWireFixture;
    use crate::grpc_client::envelope::{AcquisitionProvenance, CanonicalRecord, QueryResult};
    use crate::grpc_client::external_pb::magic::market::v1::QueryRequest as ExternalQueryRequest;
    use prost::Message;
    use serial_test::serial;
    use std::time::Duration;

    #[test]
    fn empty_result_only_proves_the_requested_rolling_window() {
        let request = EconomicReleaseObservationsRequest::new(20, Some("中国".to_owned()))
            .expect("published request");
        let response = QueryResult {
            admission: QueryAdmission::Admitted,
            selected_provider: "Jin10".to_owned(),
            batch_id: "TEST_CODE_EMPTY_RELEASE_WINDOW".to_owned(),
            complete: true,
            observed_at: "1784943002.000000000".to_owned(),
            source_at: String::new(),
            records: Vec::new(),
            provenance: AcquisitionProvenance::ExternalMtlsAuthority(
                "grpc-mtls:TEST_CODE-jin10".to_owned(),
            ),
            diagnostic_blocker: String::new(),
        };
        let batch = convert_response(&request, &response).expect("empty rolling window");
        assert!(batch.is_verified_empty());
        assert_eq!(batch.evidence().source_at, None);
        assert!(EconomicReleaseObservationsRequest::new(0, None).is_err());
        assert!(EconomicReleaseObservationsRequest::new(20, Some(String::new())).is_err());
    }

    #[test]
    fn release_rejects_partial_duplicate_and_conflicting_source_times() {
        let request = EconomicReleaseObservationsRequest::new(20, Some("中国".to_owned()))
            .expect("published request");
        let record = serde_json::json!({
            "event_id":"202607250001", "indicator_id":950, "country":"中国",
            "name":"规模以上工业企业利润", "period":"6月",
            "scheduled_at":"2026-07-25T09:30:00+08:00",
            "released_at":"2026-07-25T09:30:01+08:00",
            "previous":"-9.1", "consensus":null, "actual":"0", "revised":null,
            "unit":"%", "importance":3, "impact":"1",
            "evidence":{
                "provider":"Jin10", "source_at":"2026-07-25 09:30:01",
                "observed_at":"1784943002.000000000",
                "batch_id":"TEST_CODE_RELEASE_EVIDENCE"
            }
        });
        let original = serde_json::to_vec(&record).expect("TEST_CODE record JSON");
        let mut response = QueryResult {
            admission: QueryAdmission::Admitted,
            selected_provider: "Jin10".to_owned(),
            batch_id: "TEST_CODE_RELEASE_EVIDENCE".to_owned(),
            complete: true,
            observed_at: "1784943002.000000000".to_owned(),
            source_at: "2026-07-25 09:30:01".to_owned(),
            records: vec![CanonicalRecord {
                schema: RECORD_SCHEMA.to_owned(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: original.clone(),
            }],
            provenance: AcquisitionProvenance::ExternalMtlsAuthority(
                "grpc-mtls:TEST_CODE-jin10".to_owned(),
            ),
            diagnostic_blocker: String::new(),
        };
        assert_eq!(
            convert_response(&request, &response)
                .unwrap()
                .records()
                .len(),
            1
        );

        response.complete = false;
        assert!(convert_response(&request, &response).is_err());
        response.complete = true;
        response.diagnostic_blocker = "TEST_CODE_DIAGNOSTIC".to_owned();
        assert!(convert_response(&request, &response).is_err());
        response.diagnostic_blocker.clear();

        response.source_at = "2026-07-25 09:29:59".to_owned();
        assert!(convert_response(&request, &response).is_err());
        response.source_at = "2026-07-25 09:30:01".to_owned();

        let mut conflict = record;
        conflict["evidence"]["source_at"] = serde_json::json!("2026-07-25 09:30:02");
        response.records[0].data = serde_json::to_vec(&conflict).unwrap();
        assert!(convert_response(&request, &response).is_err());
        response.records[0].data = original;

        let duplicate = response.records[0].clone();
        response.records.push(duplicate);
        assert!(convert_response(&request, &response).is_err());
    }

    #[tokio::test]
    #[serial]
    async fn gateway_sends_limit_country_through_qualified_external_62() {
        let fixture = ExternalQueryWireFixture::bind_qualified_release_observations()
            .await
            .expect("qualified release fixture");
        let _env = crate::data_gateway::grpc_source::test_grpc_env_guard();
        crate::database::DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", fixture.bundle_path());
        crate::data_gateway::grpc_source::reset_bridge();
        fixture.release_capabilities();
        fixture.release();
        let request = EconomicReleaseObservationsRequest::new(20, Some("中国".to_owned()))
            .expect("published request");
        let batch = tokio::time::timeout(
            Duration::from_secs(20),
            EconomicReleaseObservationsGateway::new().fetch(&request),
        )
        .await
        .expect("release gateway deadline")
        .expect("qualified release batch");
        assert_eq!(batch.records().len(), 1);
        assert_eq!(batch.records()[0].event_id, "202607250001");
        assert_eq!(
            batch.evidence().source_at.as_deref(),
            Some("2026-07-25 09:30:01")
        );
        assert_eq!(batch.records()[0].evidence.provider(), ProviderId::Jin10);
        let observed = fixture.snapshot();
        assert_eq!(observed.capabilities_calls, 1);
        assert_eq!(observed.methods, ["economic_release_observations"]);
        let wire = ExternalQueryRequest::decode(observed.requests[0].as_slice())
            .expect("captured generated RPC request");
        assert_eq!(wire.preferred_provider, "Jin10");
        assert!(!wire.allow_unadmitted);
        let payload = wire.payload.expect("release payload");
        assert_eq!(
            payload.schema,
            "magic.market.economic_release_observations.request"
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap(),
            serde_json::json!({"limit":20,"country":"中国"})
        );
        crate::data_gateway::grpc_source::reset_bridge();
        fixture.finish().await.expect("release fixture cleanup");
    }
}
