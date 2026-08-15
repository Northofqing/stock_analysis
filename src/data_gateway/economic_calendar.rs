//! BR-133/BR-167 evidence-preserving macroeconomic release acquisition.

use super::review::{acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result};
use super::{BatchEvidence, GatewayBatch, GatewayError};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use magic_jin10_rs::{Jin10Client, Jin10Error};
use magic_market_core::{
    DataBatch, EconomicCalendarProvider, EconomicCalendarRequest, EconomicEvent, PositiveU32,
    ProviderId, SourceEvidence,
};
use std::collections::HashSet;

const CAPABILITY: &str = "EconomicCalendar-Jin10";
const SOURCE: &str = "jin10-flash-v1";
const MAX_LIMIT: u32 = 20;

/// One admitted public macroeconomic release with immutable upstream evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomicReleaseFact {
    pub event_id: String,
    pub indicator_id: u32,
    pub country: String,
    pub name: String,
    pub period: Option<String>,
    pub scheduled_at: DateTime<Utc>,
    pub released_at: DateTime<Utc>,
    pub previous: Option<String>,
    pub consensus: Option<String>,
    pub actual: Option<String>,
    pub revised: Option<String>,
    pub unit: Option<String>,
    pub importance: u32,
    pub impact: Option<String>,
    pub evidence: SourceEvidence,
}

/// Production seam for the released Jin10 economic-release provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct EconomicCalendarGateway;

impl EconomicCalendarGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn latest_releases(
        &self,
        limit: u32,
        country: Option<&str>,
    ) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
        let country = country.map(str::to_owned);
        let request_hash = acquisition_request_hash(
            CAPABILITY,
            &format!(
                "limit={limit}:country={}",
                country.as_deref().unwrap_or("*")
            ),
        );
        // P4 M3 钩子: DATA_GATEWAY_GRPC=1 → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("EconomicCalendar") {
            Ok(Some(bridge)) => {
                let result = bridge.economic_calendar_async().await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Jin10);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(CAPABILITY, ProviderId::Jin10, &request_hash, Err(error));
            }
        }
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = build_request(limit, country.as_deref()).and_then(fetch_and_admit);
            audit_gateway_result(CAPABILITY, ProviderId::Jin10, &worker_hash, result)
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    CAPABILITY,
                    ProviderId::Jin10,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }
}

fn build_request(
    limit: u32,
    country: Option<&str>,
) -> Result<EconomicCalendarRequest, GatewayError> {
    if limit > MAX_LIMIT {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            format!("economic-release limit {limit} exceeds {MAX_LIMIT}"),
        ));
    }
    let limit = PositiveU32::new(limit).map_err(|error| {
        GatewayError::invalid_request(
            CAPABILITY,
            format!("invalid economic-release limit: {error}"),
        )
    })?;
    let mut request = EconomicCalendarRequest::new(limit).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid economic request: {error}"))
    })?;
    if let Some(country) = country {
        request = request.with_country(country).map_err(|error| {
            GatewayError::invalid_request(CAPABILITY, format!("invalid country: {error}"))
        })?;
    }
    Ok(request)
}

fn fetch_and_admit(
    request: EconomicCalendarRequest,
) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
    let client = Jin10Client::new().map_err(jin10_gateway_error)?;
    let batch = client
        .economic_calendar(&request)
        .map_err(jin10_gateway_error)?;
    admit_jin10_batch(batch, &request)
}

fn admit_jin10_batch(
    batch: DataBatch<EconomicEvent>,
    request: &EconomicCalendarRequest,
) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
    if batch.provenance().source() != SOURCE {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            format!(
                "unexpected economic-release source {:?}",
                batch.provenance().source()
            ),
        ));
    }
    if !batch.quality().is_complete() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            format!(
                "incomplete economic-release batch: {}",
                batch.quality().issues().join("; ")
            ),
        ));
    }
    let evidence = BatchEvidence::from_provenance(ProviderId::Jin10, batch.provenance())?;
    let observed_at = parse_observed_at(&evidence.observed_at)?;
    if batch.records().is_empty() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            "Jin10 economic-release batch is unexpectedly empty",
        ));
    }
    if batch.records().len() > request.limit().get() as usize {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            "Jin10 economic-release batch exceeds the requested limit",
        ));
    }

    let mut event_ids = HashSet::with_capacity(batch.records().len());
    let mut previous_released_at: Option<DateTime<Utc>> = None;
    let mut records = Vec::with_capacity(batch.records().len());
    for event in batch.into_records() {
        validate_record_evidence(&event, &evidence)?;
        if request
            .country()
            .is_some_and(|country| country.as_str() != event.country.as_str())
        {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Jin10),
                format!(
                    "economic event {} does not match requested country",
                    event.event_id.as_str()
                ),
            ));
        }
        let event_id = event.event_id.as_str().to_owned();
        if !event_ids.insert(event_id.clone()) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Jin10),
                format!("duplicate economic event ID {event_id}"),
            ));
        }
        let released_at = parse_china_time(event.released_at.as_str(), "released_at")?;
        if released_at > observed_at {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Jin10),
                format!("economic event {event_id} was released after observation"),
            ));
        }
        if previous_released_at.is_some_and(|previous| previous < released_at) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Jin10),
                "economic-release records are not ordered newest first",
            ));
        }
        previous_released_at = Some(released_at);
        if !(1..=5).contains(&event.importance.get()) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Jin10),
                format!(
                    "economic event {event_id} importance {} is outside 1..=5",
                    event.importance.get()
                ),
            ));
        }
        let scheduled_at = parse_china_time(event.scheduled_at.as_str(), "scheduled_at")?;
        records.push(EconomicReleaseFact {
            event_id,
            indicator_id: event.indicator_id.get(),
            country: event.country.as_str().to_owned(),
            name: event.name.as_str().to_owned(),
            period: optional_text(event.period),
            scheduled_at,
            released_at,
            previous: optional_text(event.previous),
            consensus: optional_text(event.consensus),
            actual: optional_text(event.actual),
            revised: optional_text(event.revised),
            unit: optional_text(event.unit),
            importance: event.importance.get(),
            impact: optional_text(event.impact),
            evidence: event.evidence,
        });
    }

    let newest = records
        .first()
        .ok_or_else(|| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Jin10),
                "economic-release batch has no newest record",
            )
        })?
        .released_at;
    let source_at = evidence.source_at.as_deref().ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            "economic-release batch source time is missing",
        )
    })?;
    if parse_china_time(source_at, "batch source_at")? != newest {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            "economic-release batch source time differs from its newest record",
        ));
    }
    Ok(GatewayBatch::Available { records, evidence })
}

fn validate_record_evidence(
    event: &EconomicEvent,
    batch: &BatchEvidence,
) -> Result<(), GatewayError> {
    if event.evidence.provider() != ProviderId::Jin10
        || event.evidence.batch_id() != batch.batch_id
        || event.evidence.observed_at() != batch.observed_at
        || event.evidence.source_at() != Some(event.released_at.as_str())
    {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            format!(
                "economic event {} evidence differs from its batch",
                event.event_id.as_str()
            ),
        ));
    }
    Ok(())
}

fn parse_china_time(value: &str, field: &str) -> Result<DateTime<Utc>, GatewayError> {
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            format!("invalid economic {field} {value:?}: {error}"),
        )
    })?;
    let china = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            "UTC+08:00 offset is unavailable",
        )
    })?;
    china
        .from_local_datetime(&naive)
        .single()
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .ok_or_else(|| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Jin10),
                format!("ambiguous economic {field} {value:?}"),
            )
        })
}

fn parse_observed_at(value: &str) -> Result<DateTime<Utc>, GatewayError> {
    let (seconds, nanos) = value.split_once('.').ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            format!("invalid economic observation time {value:?}"),
        )
    })?;
    if nanos.len() != 9 || !nanos.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            format!("invalid economic observation nanoseconds {value:?}"),
        ));
    }
    let seconds = seconds.parse::<i64>().map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            format!("invalid economic observation seconds {value:?}: {error}"),
        )
    })?;
    let nanos = nanos.parse::<u32>().map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            format!("invalid economic observation nanoseconds {value:?}: {error}"),
        )
    })?;
    DateTime::from_timestamp(seconds, nanos).ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Jin10),
            format!("economic observation time is out of range {value:?}"),
        )
    })
}

fn optional_text(value: Option<magic_market_core::NonEmptyText>) -> Option<String> {
    value.map(|value| value.as_str().to_owned())
}

fn jin10_gateway_error(error: Jin10Error) -> GatewayError {
    let message = error.to_string();
    match error {
        Jin10Error::InvalidRequest(_) => GatewayError::invalid_request(CAPABILITY, message),
        Jin10Error::Transport(_) => {
            GatewayError::unavailable(CAPABILITY, Some(ProviderId::Jin10), true, message)
        }
        Jin10Error::Unsupported(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Jin10),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        Jin10Error::Decode(_) | Jin10Error::Protocol(_) | Jin10Error::Core(_) => {
            GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::Jin10), message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magic_market_core::{
        DataBatch, EconomicEvent, NonEmptyText, PositiveU32, Provenance, ProviderId, SourceEvidence,
    };

    fn event(
        batch_id: &str,
        observed_at: &str,
        event_id: &str,
        released_at: &str,
        importance: u32,
    ) -> EconomicEvent {
        EconomicEvent {
            event_id: NonEmptyText::new(event_id).expect("TEST_CODE event ID"),
            indicator_id: PositiveU32::new(100).expect("TEST_CODE indicator"),
            country: NonEmptyText::new("中国").expect("TEST_CODE country"),
            name: NonEmptyText::new(format!("TEST_CODE indicator {event_id}"))
                .expect("TEST_CODE name"),
            period: None,
            scheduled_at: NonEmptyText::new(released_at).expect("TEST_CODE scheduled time"),
            released_at: NonEmptyText::new(released_at).expect("TEST_CODE release time"),
            previous: None,
            consensus: None,
            actual: None,
            revised: None,
            unit: None,
            importance: PositiveU32::new(importance).expect("TEST_CODE importance"),
            impact: None,
            evidence: SourceEvidence::new(ProviderId::Jin10, observed_at, batch_id)
                .and_then(|evidence| evidence.with_source_at(released_at))
                .expect("TEST_CODE record evidence"),
        }
    }

    fn batch_with_importance(times: &[&str], importance: u32) -> DataBatch<EconomicEvent> {
        let batch_id = "TEST_CODE_economic_batch";
        let observed_at = "1784959200.000000000";
        let records = times
            .iter()
            .enumerate()
            .map(|(index, time)| {
                event(
                    batch_id,
                    observed_at,
                    &format!("TEST_CODE_{index}"),
                    time,
                    importance,
                )
            })
            .collect();
        let provenance = Provenance::new("jin10-flash-v1", observed_at)
            .and_then(|value| value.with_source_at(times[0]))
            .and_then(|value| value.with_batch_id(batch_id))
            .expect("TEST_CODE provenance");
        DataBatch::strict(records, provenance)
    }

    fn batch(times: &[&str]) -> DataBatch<EconomicEvent> {
        batch_with_importance(times, 3)
    }

    #[test]
    fn admits_complete_latest_release_batch_and_preserves_missing_values() {
        let request = build_request(2, None).expect("TEST_CODE request");
        let admitted = admit_jin10_batch(
            batch(&["2026-07-25 10:00:00", "2026-07-25 09:00:00"]),
            &request,
        )
        .expect("TEST_CODE admitted batch");

        let GatewayBatch::Available { records, evidence } = admitted else {
            panic!("TEST_CODE expected available batch");
        };
        assert_eq!(evidence.provider, ProviderId::Jin10);
        assert_eq!(evidence.source, "jin10-flash-v1");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].event_id, "TEST_CODE_0");
        assert_eq!(records[0].country, "中国");
        assert_eq!(
            records[0].released_at.to_rfc3339(),
            "2026-07-25T02:00:00+00:00"
        );
        assert_eq!(records[0].previous, None);
        assert_eq!(records[0].actual, None);
    }

    #[test]
    fn rejects_invalid_request_and_evidence_instead_of_returning_empty() {
        assert!(build_request(0, None).is_err());
        assert!(build_request(21, None).is_err());
        assert!(build_request(2, Some(" ")).is_err());

        let request = build_request(2, None).expect("TEST_CODE request");
        assert!(admit_jin10_batch(
            batch(&["2026-07-25 09:00:00", "2026-07-25 10:00:00"]),
            &request
        )
        .is_err());
        assert!(
            admit_jin10_batch(batch_with_importance(&["2026-07-25 10:00:00"], 6), &request)
                .is_err()
        );
    }

    #[test]
    fn batch_contract_rejects_empty_partial_wrong_source_and_excess_rows() {
        let request = build_request(1, None).expect("TEST_CODE request");
        let observed_at = "1784959200.000000000";
        let batch_id = "TEST_CODE_economic_contract";

        let empty_provenance = Provenance::new(SOURCE, observed_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        assert!(admit_jin10_batch(
            DataBatch::strict(Vec::<EconomicEvent>::new(), empty_provenance),
            &request
        )
        .is_err());

        let wrong_source = Provenance::new("TEST_CODE_wrong-source", observed_at)
            .unwrap()
            .with_source_at("2026-07-25 10:00:00")
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        assert!(admit_jin10_batch(
            DataBatch::strict(
                vec![event(
                    batch_id,
                    observed_at,
                    "TEST_CODE_wrong_source",
                    "2026-07-25 10:00:00",
                    3,
                )],
                wrong_source,
            ),
            &request,
        )
        .is_err());

        let partial_provenance = Provenance::new(SOURCE, observed_at)
            .unwrap()
            .with_source_at("2026-07-25 10:00:00")
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        let partial = DataBatch::best_effort(
            Vec::<EconomicEvent>::new(),
            partial_provenance,
            vec!["TEST_CODE incomplete release".to_string()],
        )
        .unwrap();
        assert!(admit_jin10_batch(partial, &request).is_err());

        let too_many = batch(&["2026-07-25 10:00:00", "2026-07-25 09:00:00"]);
        assert!(admit_jin10_batch(too_many, &request).is_err());
    }

    #[test]
    fn record_identity_country_time_and_batch_source_must_match() {
        let country_request = build_request(2, Some("中国")).expect("TEST_CODE country request");
        let observed_at = "1784959200.000000000";
        let batch_id = "TEST_CODE_economic_batch";

        let mut wrong_country = batch(&["2026-07-25 10:00:00"]);
        let mut rows = wrong_country.into_records();
        rows[0].country = NonEmptyText::new("美国").unwrap();
        let provenance = Provenance::new(SOURCE, observed_at)
            .unwrap()
            .with_source_at("2026-07-25 10:00:00")
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        wrong_country = DataBatch::strict(rows, provenance);
        assert!(admit_jin10_batch(wrong_country, &country_request).is_err());

        let duplicate = vec![
            event(
                batch_id,
                observed_at,
                "TEST_CODE_duplicate",
                "2026-07-25 10:00:00",
                3,
            ),
            event(
                batch_id,
                observed_at,
                "TEST_CODE_duplicate",
                "2026-07-25 09:00:00",
                3,
            ),
        ];
        let provenance = Provenance::new(SOURCE, observed_at)
            .unwrap()
            .with_source_at("2026-07-25 10:00:00")
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        assert!(
            admit_jin10_batch(DataBatch::strict(duplicate, provenance), &country_request).is_err()
        );

        let future = batch(&["2026-07-25 23:59:59"]);
        assert!(admit_jin10_batch(future, &country_request).is_err());

        let source_mismatch = vec![event(
            batch_id,
            observed_at,
            "TEST_CODE_source_mismatch",
            "2026-07-25 10:00:00",
            3,
        )];
        let provenance = Provenance::new(SOURCE, observed_at)
            .unwrap()
            .with_source_at("2026-07-25 09:00:00")
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        assert!(admit_jin10_batch(
            DataBatch::strict(source_mismatch, provenance),
            &country_request,
        )
        .is_err());

        let mut wrong_evidence = event(
            batch_id,
            observed_at,
            "TEST_CODE_wrong_evidence",
            "2026-07-25 10:00:00",
            3,
        );
        wrong_evidence.evidence =
            SourceEvidence::new(ProviderId::Jin10, observed_at, "TEST_CODE_other_batch")
                .unwrap()
                .with_source_at("2026-07-25 10:00:00")
                .unwrap();
        let provenance = Provenance::new(SOURCE, observed_at)
            .unwrap()
            .with_source_at("2026-07-25 10:00:00")
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        assert!(admit_jin10_batch(
            DataBatch::strict(vec![wrong_evidence], provenance),
            &country_request,
        )
        .is_err());
    }

    #[test]
    fn timestamp_and_provider_error_classification_are_explicit() {
        assert_eq!(
            parse_china_time("2026-07-25 10:00:00", "TEST_CODE")
                .unwrap()
                .to_rfc3339(),
            "2026-07-25T02:00:00+00:00"
        );
        assert!(parse_china_time("bad", "TEST_CODE").is_err());
        assert!(parse_observed_at("bad").is_err());
        assert!(parse_observed_at("1.bad").is_err());
        assert!(parse_observed_at("bad.000000000").is_err());
        assert!(parse_observed_at("999999999999999999.000000000").is_err());
        assert_eq!(
            optional_text(Some(NonEmptyText::new("TEST_CODE value").unwrap())).as_deref(),
            Some("TEST_CODE value")
        );
        assert_eq!(optional_text(None), None);

        let invalid = jin10_gateway_error(Jin10Error::InvalidRequest("TEST_CODE".into()));
        let transport = jin10_gateway_error(Jin10Error::Transport("TEST_CODE".into()));
        let unsupported = jin10_gateway_error(Jin10Error::Unsupported("TEST_CODE".into()));
        let decode = jin10_gateway_error(Jin10Error::Decode("TEST_CODE".into()));
        let protocol = jin10_gateway_error(Jin10Error::Protocol("TEST_CODE".into()));
        assert_eq!(invalid.audit_outcome(), "invalid_request");
        assert_eq!(transport.audit_outcome(), "unavailable");
        assert!(transport.retryable());
        assert_eq!(unsupported.audit_outcome(), "unsupported");
        assert_eq!(decode.audit_outcome(), "partial");
        assert_eq!(protocol.audit_outcome(), "partial");
    }
}
