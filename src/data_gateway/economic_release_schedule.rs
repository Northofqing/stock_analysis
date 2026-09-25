//! FRED's date-only release schedule, separate from published release facts.

use super::{BatchEvidence, GatewayBatch, GatewayError};
use crate::grpc_client::envelope::{QueryAdmission, QueryResult};
use crate::market_domain::{ProviderId, SourceEvidence};
use chrono::NaiveDate;
use serde::Deserialize;

const CAPABILITY: &str = "EconomicReleaseSchedule";
const RECORD_SCHEMA: &str = "magic.market.economic_release_schedule_entry";

#[derive(Debug, Clone)]
pub struct EconomicReleaseScheduleRequest {
    start: NaiveDate,
    end: NaiveDate,
    limit: u32,
}

impl EconomicReleaseScheduleRequest {
    pub fn new(start: NaiveDate, end: NaiveDate, limit: u32) -> Result<Self, GatewayError> {
        if end < start
            || end.signed_duration_since(start).num_days() > 365
            || !(1..=100).contains(&limit)
        {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "FRED release range must span at most 366 inclusive days and limit must be 1..=100",
            ));
        }
        Ok(Self { start, end, limit })
    }

    pub fn start(&self) -> NaiveDate {
        self.start
    }

    pub fn end(&self) -> NaiveDate {
        self.end
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    pub(crate) fn params(&self) -> serde_json::Value {
        serde_json::json!({"start":self.start,"end":self.end,"limit":self.limit})
    }
}

/// FRED's date-only schedule entry. `release_last_updated` is an opaque source
/// metadata label; neither it nor `release_date` establishes a source instant.
#[derive(Debug, Clone)]
pub struct EconomicReleaseScheduleEntry {
    pub release_id: u64,
    pub release_name: String,
    pub release_date: NaiveDate,
    pub release_last_updated: String,
    pub evidence: SourceEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleWire {
    release_id: u64,
    release_name: String,
    release_date: String,
    release_last_updated: String,
    evidence: SourceEvidence,
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::Fred), message)
}

pub(crate) fn convert_response(
    request: &EconomicReleaseScheduleRequest,
    response: &QueryResult,
) -> Result<GatewayBatch<EconomicReleaseScheduleEntry>, GatewayError> {
    if response.admission != QueryAdmission::Admitted
        || !response.complete
        || !response.diagnostic_blocker.is_empty()
        || response.selected_provider != "Fred"
        || !response.source_at.is_empty()
        || !response.source().starts_with("grpc-mtls:")
        || response.batch_id.trim().is_empty()
    {
        return Err(invalid(
            "FRED schedule envelope is not admitted and complete",
        ));
    }
    let batch_time = super::evidence_time::parse_evidence_instant(
        CAPABILITY,
        ProviderId::Fred,
        "observed_at",
        &response.observed_at,
    )?;
    let evidence = BatchEvidence {
        provider: ProviderId::Fred,
        source: response.source().to_owned(),
        source_at: None,
        observed_at: response.observed_at.clone(),
        batch_id: response.batch_id.clone(),
    };
    if response.records.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }
    if response.records.len() > request.limit as usize {
        return Err(invalid("FRED schedule exceeds requested limit"));
    }
    let mut prior: Option<(NaiveDate, u64)> = None;
    let mut records = Vec::with_capacity(response.records.len());
    for payload in &response.records {
        if payload.schema != RECORD_SCHEMA
            || payload.schema_version != 1
            || payload.content_type != "application/json; charset=utf-8"
        {
            return Err(invalid(
                "FRED schedule record schema/version/content type mismatch",
            ));
        }
        let value: serde_json::Value = serde_json::from_slice(&payload.data)
            .map_err(|error| invalid(format!("FRED schedule record JSON invalid: {error}")))?;
        let object = value
            .as_object()
            .ok_or_else(|| invalid("FRED schedule record is not an object"))?;
        for field in [
            "release_id",
            "release_name",
            "release_date",
            "release_last_updated",
            "evidence",
        ] {
            if !object.contains_key(field) {
                return Err(invalid(format!("FRED schedule record missing {field}")));
            }
        }
        let wire: ScheduleWire = serde_json::from_value(value)
            .map_err(|error| invalid(format!("FRED schedule fields invalid: {error}")))?;
        let release_date = NaiveDate::parse_from_str(&wire.release_date, "%Y-%m-%d")
            .map_err(|error| invalid(format!("FRED release_date invalid: {error}")))?;
        let key = (release_date, wire.release_id);
        if release_date < request.start
            || release_date > request.end
            || prior.is_some_and(|prior| key <= prior)
            || wire.release_name.trim().is_empty()
            || wire.evidence.provider() != ProviderId::Fred
            || wire.evidence.source_at().is_some()
            || wire.evidence.batch_id() != response.batch_id
        {
            return Err(invalid(
                "FRED schedule date/order/identity/evidence conflicts",
            ));
        }
        let record_time = super::evidence_time::parse_evidence_instant(
            CAPABILITY,
            ProviderId::Fred,
            "record observed_at",
            wire.evidence.observed_at(),
        )?;
        if record_time > batch_time {
            return Err(invalid(
                "FRED schedule record observation is newer than batch",
            ));
        }
        prior = Some(key);
        records.push(EconomicReleaseScheduleEntry {
            release_id: wire.release_id,
            release_name: wire.release_name,
            release_date,
            release_last_updated: wire.release_last_updated,
            evidence: wire.evidence,
        });
    }
    Ok(GatewayBatch::Available { records, evidence })
}

/// Exact FRED date-range query; a zero-row result only covers this request.
#[derive(Debug, Clone, Copy, Default)]
pub struct EconomicReleaseScheduleGateway;

impl EconomicReleaseScheduleGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn fetch(
        &self,
        request: &EconomicReleaseScheduleRequest,
    ) -> Result<GatewayBatch<EconomicReleaseScheduleEntry>, GatewayError> {
        let request_hash =
            super::review::acquisition_request_hash(CAPABILITY, request.params().to_string());
        let result = match super::grpc_source::bridge_for("EconomicReleaseSchedule") {
            Ok(bridge) => bridge.economic_release_schedule_async(request).await,
            Err(error) => Err(error),
        };
        super::review::audit_gateway_result(CAPABILITY, ProviderId::Fred, &request_hash, result)
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

    fn date(raw: &str) -> NaiveDate {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d").expect("TEST_CODE ISO date")
    }

    fn schedule_record(release_id: u64, release_date: &str) -> CanonicalRecord {
        let value = serde_json::json!({
            "release_id":release_id,
            "release_name":"Consumer Price Index",
            "release_date":release_date,
            "release_last_updated":"2026-08-01 09:30:00-05",
            "evidence":{
                "provider":"Fred", "source_at":null,
                "observed_at":"1789257600.000000000",
                "batch_id":"TEST_CODE_FRED_SCHEDULE_BATCH"
            }
        });
        CanonicalRecord {
            schema: RECORD_SCHEMA.to_owned(),
            schema_version: 1,
            content_type: "application/json; charset=utf-8".to_owned(),
            data: serde_json::to_vec(&value).expect("TEST_CODE schedule JSON"),
        }
    }

    fn response(records: Vec<CanonicalRecord>) -> QueryResult {
        QueryResult {
            admission: QueryAdmission::Admitted,
            selected_provider: "Fred".to_owned(),
            batch_id: "TEST_CODE_FRED_SCHEDULE_BATCH".to_owned(),
            complete: true,
            observed_at: "1789257600.000000000".to_owned(),
            source_at: String::new(),
            records,
            provenance: AcquisitionProvenance::ExternalMtlsAuthority(
                "grpc-mtls:TEST_CODE-fred".to_owned(),
            ),
            diagnostic_blocker: String::new(),
        }
    }

    #[test]
    fn inclusive_range_and_empty_scope_keep_source_time_absent() {
        let start = date("2026-01-01");
        let end = date("2027-01-01");
        let request =
            EconomicReleaseScheduleRequest::new(start, end, 100).expect("366 inclusive days");
        let batch = convert_response(&request, &response(Vec::new())).expect("empty FRED range");
        assert!(batch.is_verified_empty());
        assert_eq!(batch.evidence().source_at, None);
        assert!(EconomicReleaseScheduleRequest::new(start, date("2027-01-02"), 20).is_err());
        assert!(EconomicReleaseScheduleRequest::new(end, start, 20).is_err());
        assert!(EconomicReleaseScheduleRequest::new(start, end, 0).is_err());
        assert!(EconomicReleaseScheduleRequest::new(start, end, 101).is_err());
    }

    #[test]
    fn same_release_on_two_dates_is_valid_but_duplicate_key_or_bad_scope_is_not() {
        let request =
            EconomicReleaseScheduleRequest::new(date("2026-09-13"), date("2026-10-13"), 20)
                .expect("published range");
        let records = vec![
            schedule_record(10, "2026-09-15"),
            schedule_record(10, "2026-09-16"),
        ];
        let batch = convert_response(&request, &response(records)).expect("distinct date keys");
        assert_eq!(batch.records().len(), 2);
        assert_eq!(batch.records()[0].release_id, batch.records()[1].release_id);
        assert_ne!(
            batch.records()[0].release_date,
            batch.records()[1].release_date
        );
        assert_eq!(batch.records()[0].evidence.source_at(), None);
        assert_eq!(batch.evidence().source_at, None);

        assert!(convert_response(
            &request,
            &response(vec![
                schedule_record(10, "2026-09-15"),
                schedule_record(10, "2026-09-15"),
            ])
        )
        .is_err());
        assert!(convert_response(
            &request,
            &response(vec![
                schedule_record(10, "2026-09-16"),
                schedule_record(10, "2026-09-15"),
            ])
        )
        .is_err());
        assert!(convert_response(
            &request,
            &response(vec![schedule_record(10, "2026-10-14"),])
        )
        .is_err());
        let mut partial = response(vec![schedule_record(10, "2026-09-15")]);
        partial.complete = false;
        assert!(convert_response(&request, &partial).is_err());
        partial.complete = true;
        partial.source_at = "2026-09-15".to_owned();
        assert!(convert_response(&request, &partial).is_err());
    }

    #[tokio::test]
    #[serial]
    async fn gateway_sends_range_through_qualified_external_63() {
        let fixture = ExternalQueryWireFixture::bind_qualified_release_schedule()
            .await
            .expect("qualified FRED fixture");
        let _env = crate::data_gateway::grpc_source::test_grpc_env_guard();
        crate::database::DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::set_var("GRPC_MARKET_CLIENT_BUNDLE", fixture.bundle_path());
        crate::data_gateway::grpc_source::reset_bridge();
        fixture.release_capabilities();
        fixture.release();
        let request =
            EconomicReleaseScheduleRequest::new(date("2026-09-13"), date("2026-10-13"), 20)
                .expect("published request");
        let batch = tokio::time::timeout(
            Duration::from_secs(20),
            EconomicReleaseScheduleGateway::new().fetch(&request),
        )
        .await
        .expect("schedule gateway deadline")
        .expect("qualified schedule batch");
        assert_eq!(batch.records().len(), 2);
        assert_eq!(batch.records()[0].release_id, batch.records()[1].release_id);
        assert_ne!(
            batch.records()[0].release_date,
            batch.records()[1].release_date
        );
        assert_eq!(
            batch.records()[0].release_last_updated,
            "2026-08-01 09:30:00-05"
        );
        assert_eq!(batch.evidence().source_at, None);
        assert_eq!(batch.records()[0].evidence.source_at(), None);
        let observed = fixture.snapshot();
        assert_eq!(observed.capabilities_calls, 1);
        assert_eq!(observed.methods, ["economic_release_schedule"]);
        let wire = ExternalQueryRequest::decode(observed.requests[0].as_slice())
            .expect("captured generated RPC request");
        assert_eq!(wire.preferred_provider, "Fred");
        assert!(!wire.allow_unadmitted);
        let payload = wire.payload.expect("schedule payload");
        assert_eq!(
            payload.schema,
            "magic.market.economic_release_schedule.request"
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&payload.data).unwrap(),
            serde_json::json!({"start":"2026-09-13","end":"2026-10-13","limit":20})
        );
        crate::data_gateway::grpc_source::reset_bridge();
        fixture.finish().await.expect("schedule fixture cleanup");
    }
}
