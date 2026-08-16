//! BR-161 evidence-preserving R-08 event-calendar acquisition.

#[cfg(feature = "magic-gateway")]
use super::review::audit_blocking_join_failure;
use super::review::{acquisition_request_hash, audit_gateway_result};
#[cfg(feature = "magic-gateway")]
use super::BatchEvidence;
use super::GatewayBatch;
use super::GatewayError;
use crate::magic_compat::ProviderId;
#[cfg(feature = "magic-gateway")]
use crate::magic_compat::{DataBatch, IsoDate, PositiveU32};
use chrono::{DateTime, NaiveDate, Utc};
#[cfg(feature = "magic-gateway")]
use magic_cninfo_rs::{CninfoClient, CninfoError};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{Announcement as CoreAnnouncement, MarketAnnouncementRequest};
#[cfg(feature = "magic-gateway")]
use magic_market_router::{
    market_announcement_source, AcceptancePolicy, AttemptStatus, FailureKind,
    MarketAnnouncementRouter, RouterError, SourceError,
};
#[cfg(feature = "magic-gateway")]
use std::collections::HashSet;
#[cfg(feature = "magic-gateway")]
use std::sync::Arc;

const CAPABILITY: &str = "R-08-announcements";
const SOURCE: &str = "cninfo-market";

/// One validated whole-market announcement fact for R-08 rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventAnnouncement {
    pub announcement_id: String,
    pub code: String,
    pub category: Option<String>,
    pub title: String,
    pub published_at: String,
    pub canonical_url: String,
}

/// BR-161 production seam for bounded, whole-market CNInfo announcements.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventCalendarGateway;

impl EventCalendarGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn market_announcements(
        &self,
        trading_date: NaiveDate,
        limit: u32,
    ) -> Result<GatewayBatch<EventAnnouncement>, GatewayError> {
        let request_hash = acquisition_request_hash(CAPABILITY, &format!("{trading_date}:{limit}"));
        // P4 M3 钩子: DATA_GATEWAY_GRPC=1 → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("Announcements") {
            Ok(Some(bridge)) => {
                let result = bridge.announcements_async().await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Cninfo);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Cninfo,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Cninfo),
                "unavailable",
                "provider_transport",
                true,
                "library transport disabled: DATA_GATEWAY_GRPC=1 required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let worker_request_hash = request_hash.clone();
            let joined = tokio::task::spawn_blocking(move || {
                let result = build_request(trading_date, limit)
                    .and_then(|request| fetch_and_admit_cninfo_batch(&request, trading_date));
                audit_gateway_result(CAPABILITY, ProviderId::Cninfo, &worker_request_hash, result)
            })
            .await;

            match joined {
                Ok(result) => {
                    let batch = result?;
                    // BR-216: a completed announcement poll proves the News source
                    // is alive. A legitimately empty batch still counts as success;
                    // only failed acquisitions skip the marker, so freshness is
                    // never fabricated.
                    crate::monitor::data_mode::mark_capability_success(
                        crate::monitor::data_mode::Capability::News,
                    )
                    .map_err(|error| GatewayError::unavailable(CAPABILITY, None, false, error))?;
                    Ok(batch)
                }
                Err(error) => {
                    audit_blocking_join_failure(
                        CAPABILITY,
                        ProviderId::Cninfo,
                        request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
    }
}

#[cfg(feature = "magic-gateway")]
fn build_request(
    trading_date: NaiveDate,
    limit: u32,
) -> Result<MarketAnnouncementRequest, GatewayError> {
    let date = IsoDate::new(trading_date.to_string()).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid announcement date: {error}"))
    })?;
    let limit = PositiveU32::new(limit).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid announcement limit: {error}"))
    })?;
    MarketAnnouncementRequest::new(date.clone(), date, limit).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid announcement request: {error}"))
    })
}

#[cfg(feature = "magic-gateway")]
fn fetch_and_admit_cninfo_batch(
    request: &MarketAnnouncementRequest,
    trading_date: NaiveDate,
) -> Result<GatewayBatch<EventAnnouncement>, GatewayError> {
    let client = Arc::new(CninfoClient::new().map_err(cninfo_gateway_error)?);
    let source =
        market_announcement_source(ProviderId::Cninfo, client, classify_cninfo_source_error);
    let mut router = MarketAnnouncementRouter::new(
        AcceptancePolicy::new()
            .with_require_complete(true)
            .with_require_source_at(false)
            .with_accept_complete_empty(true),
    );
    router
        .register(source)
        .map_err(|error| gateway_router_error(Some(ProviderId::Cninfo), error))?;
    let outcome = router
        .route(request)
        .map_err(|error| gateway_router_error(Some(ProviderId::Cninfo), error))?;
    if outcome.selected_provider() != ProviderId::Cninfo {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            "market-announcement Router selected a different provider",
        ));
    }
    admit_cninfo_market_batch(outcome.into_batch(), &trading_date.to_string())
}

#[cfg(feature = "magic-gateway")]
fn admit_cninfo_market_batch(
    batch: DataBatch<CoreAnnouncement>,
    requested_date: &str,
) -> Result<GatewayBatch<EventAnnouncement>, GatewayError> {
    let requested_date =
        NaiveDate::parse_from_str(requested_date, "%Y-%m-%d").map_err(|error| {
            GatewayError::invalid_request(
                CAPABILITY,
                format!("invalid R-08 announcement date {requested_date:?}: {error}"),
            )
        })?;
    if batch.provenance().source() != SOURCE || !batch.quality().is_complete() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            "CNInfo market announcement batch is not a complete cninfo-market batch",
        ));
    }
    let evidence = BatchEvidence::from_provenance(ProviderId::Cninfo, batch.provenance())?;
    if batch.records().is_empty() {
        if evidence.source_at.is_some() {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cninfo),
                "verified-empty CNInfo market batch must not claim source_at",
            ));
        }
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }

    let observed_at = parse_observed_at(&evidence.observed_at)?;
    let mut seen = HashSet::with_capacity(batch.records().len());
    let mut records = Vec::with_capacity(batch.records().len());
    let mut previous_published_at: Option<DateTime<Utc>> = None;
    for record in batch.into_records() {
        let provider_published_at = DateTime::parse_from_rfc3339(record.published_at.as_str())
            .map_err(|error| {
                GatewayError::invalid_evidence(
                    CAPABILITY,
                    Some(ProviderId::Cninfo),
                    format!(
                        "invalid CNInfo publication time {:?}: {error}",
                        record.published_at.as_str()
                    ),
                )
            })?;
        let published_at = provider_published_at.with_timezone(&Utc);
        if provider_published_at.date_naive() != requested_date || published_at > observed_at {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cninfo),
                "CNInfo publication time is outside the requested date or after observation",
            ));
        }
        if previous_published_at.is_some_and(|previous| published_at > previous) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cninfo),
                "CNInfo market announcements are not newest-first",
            ));
        }
        previous_published_at = Some(published_at);

        let record_evidence = &record.evidence;
        if record_evidence.provider() != ProviderId::Cninfo
            || record_evidence.batch_id() != evidence.batch_id
            || record_evidence.observed_at() != evidence.observed_at
            || record_evidence.source_at() != Some(record.published_at.as_str())
        {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cninfo),
                "CNInfo announcement evidence differs from batch provenance",
            ));
        }
        let announcement_id = record.announcement_id.as_str().to_owned();
        if !seen.insert(announcement_id.clone()) {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Cninfo),
                format!("duplicate CNInfo announcement identity {announcement_id}"),
            ));
        }
        records.push(EventAnnouncement {
            announcement_id,
            code: record.instrument.code().to_owned(),
            category: record.category.map(|value| value.as_str().to_owned()),
            title: record.title.as_str().to_owned(),
            published_at: record.published_at.as_str().to_owned(),
            canonical_url: record.canonical_url.as_str().to_owned(),
        });
    }
    if evidence.source_at.as_deref() != records.first().map(|row| row.published_at.as_str()) {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            "CNInfo batch source_at differs from newest announcement",
        ));
    }
    Ok(GatewayBatch::Available { records, evidence })
}

#[cfg(feature = "magic-gateway")]
fn cninfo_gateway_error(error: CninfoError) -> GatewayError {
    let message = error.to_string();
    match error {
        CninfoError::InvalidRequest(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            "invalid_request",
            "provider_invalid_request",
            false,
            message,
        ),
        CninfoError::Unsupported(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        CninfoError::Authentication(_)
        | CninfoError::RateLimited
        | CninfoError::Transport(_)
        | CninfoError::HttpStatus(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            "unavailable",
            "provider_transport",
            true,
            message,
        ),
        CninfoError::Decode(_)
        | CninfoError::Schema(_)
        | CninfoError::Incomplete(_)
        | CninfoError::Core(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            "partial",
            "provider_batch_rejected",
            false,
            message,
        ),
    }
}

#[cfg(feature = "magic-gateway")]
fn classify_cninfo_source_error(error: CninfoError) -> SourceError {
    let message = error.to_string();
    match error {
        CninfoError::InvalidRequest(_) => SourceError::stop(FailureKind::InvalidRequest, message),
        CninfoError::Unsupported(_) => SourceError::try_next(FailureKind::Unsupported, message),
        CninfoError::Authentication(_) => SourceError::try_next(FailureKind::Provider, message),
        CninfoError::RateLimited => SourceError::try_next(FailureKind::RateLimited, message),
        CninfoError::Transport(_) | CninfoError::HttpStatus(_) => {
            SourceError::try_next(FailureKind::Transport, message)
        }
        CninfoError::Decode(_) | CninfoError::Schema(_) => {
            SourceError::try_next(FailureKind::Protocol, message)
        }
        CninfoError::Incomplete(_) => SourceError::try_next(FailureKind::Quality, message),
        CninfoError::Core(_) => SourceError::try_next(FailureKind::Evidence, message),
    }
}

#[cfg(feature = "magic-gateway")]
fn gateway_router_error(provider: Option<ProviderId>, error: RouterError) -> GatewayError {
    let terminal_kind = error
        .attempts()
        .iter()
        .rev()
        .find_map(|attempt| match attempt.status() {
            AttemptStatus::Failed { kind, .. } | AttemptStatus::Rejected { kind, .. } => {
                Some(*kind)
            }
            AttemptStatus::Selected => None,
        });
    let (audit_outcome, reason_code, retryable) = match terminal_kind {
        Some(FailureKind::InvalidRequest) | None => {
            ("invalid_request", "router_invalid_request", false)
        }
        Some(FailureKind::Unsupported) => ("unsupported", "router_unsupported", false),
        Some(
            FailureKind::Transport
            | FailureKind::Timeout
            | FailureKind::RateLimited
            | FailureKind::Provider
            | FailureKind::NoData,
        ) => ("unavailable", "router_unavailable", true),
        Some(FailureKind::Protocol | FailureKind::Quality | FailureKind::Evidence) => {
            ("partial", "router_batch_rejected", false)
        }
    };
    GatewayError::classified(
        CAPABILITY,
        provider,
        audit_outcome,
        reason_code,
        retryable,
        error.to_string(),
    )
}

fn parse_observed_at(value: &str) -> Result<DateTime<Utc>, GatewayError> {
    let (seconds, nanos) = value.split_once('.').ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("invalid CNInfo observation time {value:?}"),
        )
    })?;
    if nanos.len() != 9 || !nanos.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("invalid CNInfo observation nanos {value:?}"),
        ));
    }
    let seconds = seconds.parse::<i64>().map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("invalid CNInfo observation seconds {value:?}: {error}"),
        )
    })?;
    let nanos = nanos.parse::<u32>().map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("invalid CNInfo observation nanos {value:?}: {error}"),
        )
    })?;
    DateTime::from_timestamp(seconds, nanos).ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("CNInfo observation time is out of range {value:?}"),
        )
    })
}

#[cfg(test)]
#[cfg(feature = "magic-gateway")]
mod tests {
    use super::*;
    use crate::magic_compat::{
        AssetClass, DataBatch, Exchange, InstrumentId, NonEmptyText, Provenance, ProviderId,
        SourceEvidence,
    };
    #[cfg(feature = "magic-gateway")]
    use magic_market_core::{Announcement, HttpsUrl};

    fn announcement(
        id: &str,
        published_at: &str,
        observed_at: &str,
        batch_id: &str,
    ) -> Announcement {
        let evidence = SourceEvidence::new(ProviderId::Cninfo, observed_at, batch_id)
            .unwrap()
            .with_source_at(published_at)
            .unwrap();
        Announcement {
            announcement_id: NonEmptyText::new(id).unwrap(),
            instrument: InstrumentId::new(
                Exchange::Shenzhen,
                "TEST_CODE_300457",
                AssetClass::Equity,
            )
            .unwrap(),
            instrument_name: None,
            category: Some(NonEmptyText::new("TEST_CODE category").unwrap()),
            title: NonEmptyText::new(format!("TEST_CODE announcement {id}")).unwrap(),
            published_at: NonEmptyText::new(published_at).unwrap(),
            canonical_url: HttpsUrl::new(format!("https://www.cninfo.com.cn/{id}")).unwrap(),
            pdf_url: None,
            evidence,
        }
    }

    fn provenance(
        source: &str,
        observed_at: &str,
        source_at: Option<&str>,
        batch_id: &str,
    ) -> Provenance {
        let mut provenance = Provenance::new(source, observed_at).unwrap();
        if let Some(source_at) = source_at {
            provenance = provenance.with_source_at(source_at).unwrap();
        }
        provenance.with_batch_id(batch_id).unwrap()
    }

    #[test]
    fn br161_admits_a_complete_cninfo_market_announcement_batch() {
        let observed_at = "1784908800.000000000";
        let source_at = "2026-07-24T23:54:08+08:00";
        let batch_id = "TEST_CODE_cninfo-market-batch";
        let evidence = SourceEvidence::new(ProviderId::Cninfo, observed_at, batch_id)
            .expect("source evidence")
            .with_source_at(source_at)
            .expect("source time");
        let record = Announcement {
            announcement_id: NonEmptyText::new("TEST_CODE_1225441868").unwrap(),
            instrument: InstrumentId::new(
                Exchange::Shenzhen,
                "TEST_CODE_300457",
                AssetClass::Equity,
            )
            .unwrap(),
            instrument_name: None,
            category: Some(NonEmptyText::new("股东大会").unwrap()),
            title: NonEmptyText::new("2026年第一次临时股东会决议公告").unwrap(),
            published_at: NonEmptyText::new(source_at).unwrap(),
            canonical_url: HttpsUrl::new(
                "https://www.cninfo.com.cn/new/disclosure/detail?stockCode=300457&announcementId=1225441868",
            )
            .unwrap(),
            pdf_url: None,
            evidence,
        };
        let provenance = Provenance::new("cninfo-market", observed_at)
            .unwrap()
            .with_source_at(source_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();

        let admitted =
            admit_cninfo_market_batch(DataBatch::strict(vec![record], provenance), "2026-07-24")
                .expect("complete market announcement batch");

        let super::super::GatewayBatch::Available { records, evidence } = admitted else {
            panic!("expected an available batch");
        };
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].announcement_id, "TEST_CODE_1225441868");
        assert_eq!(records[0].code, "TEST_CODE_300457");
        assert_eq!(records[0].published_at, source_at);
        assert_eq!(evidence.provider, ProviderId::Cninfo);
        assert_eq!(evidence.batch_id, batch_id);
    }

    #[test]
    fn br161_preserves_a_verified_empty_cninfo_market_batch() {
        let observed_at = "1784908800.000000000";
        let batch_id = "TEST_CODE_cninfo-market-empty";
        let provenance = Provenance::new("cninfo-market", observed_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();

        let admitted = admit_cninfo_market_batch(
            DataBatch::strict(Vec::<Announcement>::new(), provenance),
            "2026-07-24",
        )
        .expect("verified-empty market announcement batch");

        let super::super::GatewayBatch::VerifiedEmpty(evidence) = admitted else {
            panic!("expected a verified-empty batch");
        };
        assert_eq!(evidence.provider, ProviderId::Cninfo);
        assert_eq!(evidence.batch_id, batch_id);
        assert!(evidence.source_at.is_none());
    }

    #[test]
    fn br161_uses_the_provider_offset_for_the_announcement_date() {
        let observed_at = "1784908800.000000000";
        let source_at = "2026-07-24T00:00:00+08:00";
        let batch_id = "TEST_CODE_cninfo-market-midnight";
        let evidence = SourceEvidence::new(ProviderId::Cninfo, observed_at, batch_id)
            .unwrap()
            .with_source_at(source_at)
            .unwrap();
        let record = Announcement {
            announcement_id: NonEmptyText::new("TEST_CODE_midnight").unwrap(),
            instrument: InstrumentId::new(
                Exchange::Beijing,
                "TEST_CODE_920189",
                AssetClass::Equity,
            )
            .unwrap(),
            instrument_name: None,
            category: None,
            title: NonEmptyText::new("TEST_CODE midnight announcement").unwrap(),
            published_at: NonEmptyText::new(source_at).unwrap(),
            canonical_url: HttpsUrl::new("https://www.cninfo.com.cn/test-midnight").unwrap(),
            pdf_url: None,
            evidence,
        };
        let provenance = Provenance::new("cninfo-market", observed_at)
            .unwrap()
            .with_source_at(source_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();

        admit_cninfo_market_batch(DataBatch::strict(vec![record], provenance), "2026-07-24")
            .expect("provider-local midnight belongs to the requested provider date");
    }

    #[test]
    fn br161_request_and_verified_empty_contracts_reject_ambiguous_input() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
        assert!(build_request(date, 0).is_err());
        assert!(build_request(date, 1).is_ok());

        let observed_at = "1784908800.000000000";
        let source_at = "2026-07-24T23:54:08+08:00";
        let batch_id = "TEST_CODE_empty_contract";
        let claimed_source = DataBatch::strict(
            Vec::<Announcement>::new(),
            provenance(SOURCE, observed_at, Some(source_at), batch_id),
        );
        assert!(admit_cninfo_market_batch(claimed_source, "2026-07-24").is_err());

        let wrong_source = DataBatch::strict(
            Vec::<Announcement>::new(),
            provenance("TEST_CODE_wrong_source", observed_at, None, batch_id),
        );
        assert!(admit_cninfo_market_batch(wrong_source, "2026-07-24").is_err());

        let partial = DataBatch::best_effort(
            Vec::<Announcement>::new(),
            provenance(SOURCE, observed_at, None, batch_id),
            vec!["TEST_CODE incomplete page".to_string()],
        )
        .unwrap();
        assert!(admit_cninfo_market_batch(partial, "2026-07-24").is_err());
        assert!(admit_cninfo_market_batch(
            DataBatch::strict(
                Vec::<Announcement>::new(),
                provenance(SOURCE, observed_at, None, batch_id),
            ),
            "bad-date",
        )
        .is_err());
    }

    #[test]
    fn br161_record_time_order_identity_and_evidence_are_strict() {
        let observed_at = "1784908800.000000000";
        let batch_id = "TEST_CODE_strict_records";
        let newest = "2026-07-24T23:54:08+08:00";
        let older = "2026-07-24T22:54:08+08:00";

        let unordered = vec![
            announcement("TEST_CODE_older", older, observed_at, batch_id),
            announcement("TEST_CODE_newer", newest, observed_at, batch_id),
        ];
        assert!(admit_cninfo_market_batch(
            DataBatch::strict(
                unordered,
                provenance(SOURCE, observed_at, Some(older), batch_id),
            ),
            "2026-07-24",
        )
        .is_err());

        let duplicate = vec![
            announcement("TEST_CODE_duplicate", newest, observed_at, batch_id),
            announcement("TEST_CODE_duplicate", older, observed_at, batch_id),
        ];
        assert!(admit_cninfo_market_batch(
            DataBatch::strict(
                duplicate,
                provenance(SOURCE, observed_at, Some(newest), batch_id),
            ),
            "2026-07-24",
        )
        .is_err());

        let wrong_date = vec![announcement(
            "TEST_CODE_wrong_date",
            "2026-07-23T23:54:08+08:00",
            observed_at,
            batch_id,
        )];
        assert!(admit_cninfo_market_batch(
            DataBatch::strict(
                wrong_date,
                provenance(
                    SOURCE,
                    observed_at,
                    Some("2026-07-23T23:54:08+08:00"),
                    batch_id,
                ),
            ),
            "2026-07-24",
        )
        .is_err());

        let mut wrong_evidence = announcement("TEST_CODE_evidence", newest, observed_at, batch_id);
        wrong_evidence.evidence =
            SourceEvidence::new(ProviderId::Cninfo, observed_at, "TEST_CODE_other_batch")
                .unwrap()
                .with_source_at(newest)
                .unwrap();
        assert!(admit_cninfo_market_batch(
            DataBatch::strict(
                vec![wrong_evidence],
                provenance(SOURCE, observed_at, Some(newest), batch_id),
            ),
            "2026-07-24",
        )
        .is_err());

        let source_mismatch = vec![announcement(
            "TEST_CODE_source_mismatch",
            newest,
            observed_at,
            batch_id,
        )];
        assert!(admit_cninfo_market_batch(
            DataBatch::strict(
                source_mismatch,
                provenance(SOURCE, observed_at, Some(older), batch_id),
            ),
            "2026-07-24",
        )
        .is_err());
    }

    #[test]
    fn br161_timestamp_and_cninfo_errors_preserve_failure_classification() {
        assert!(parse_observed_at("bad").is_err());
        assert!(parse_observed_at("1.bad").is_err());
        assert!(parse_observed_at("bad.000000000").is_err());
        assert!(parse_observed_at("999999999999999999.000000000").is_err());

        let cases = [
            cninfo_gateway_error(CninfoError::InvalidRequest("TEST_CODE".into())),
            cninfo_gateway_error(CninfoError::Unsupported("TEST_CODE".into())),
            cninfo_gateway_error(CninfoError::Authentication(403)),
            cninfo_gateway_error(CninfoError::RateLimited),
            cninfo_gateway_error(CninfoError::Transport("TEST_CODE".into())),
            cninfo_gateway_error(CninfoError::HttpStatus(503)),
            cninfo_gateway_error(CninfoError::Decode("TEST_CODE".into())),
            cninfo_gateway_error(CninfoError::Schema("TEST_CODE".into())),
            cninfo_gateway_error(CninfoError::Incomplete("TEST_CODE".into())),
        ];
        assert_eq!(cases[0].audit_outcome(), "invalid_request");
        assert_eq!(cases[1].audit_outcome(), "unsupported");
        for error in &cases[2..6] {
            assert_eq!(error.audit_outcome(), "unavailable");
            assert!(error.retryable());
        }
        for error in &cases[6..] {
            assert_eq!(error.audit_outcome(), "partial");
            assert!(!error.retryable());
        }

        let source_errors = [
            classify_cninfo_source_error(CninfoError::InvalidRequest("TEST_CODE".into())),
            classify_cninfo_source_error(CninfoError::Unsupported("TEST_CODE".into())),
            classify_cninfo_source_error(CninfoError::Authentication(403)),
            classify_cninfo_source_error(CninfoError::RateLimited),
            classify_cninfo_source_error(CninfoError::Transport("TEST_CODE".into())),
            classify_cninfo_source_error(CninfoError::HttpStatus(503)),
            classify_cninfo_source_error(CninfoError::Decode("TEST_CODE".into())),
            classify_cninfo_source_error(CninfoError::Schema("TEST_CODE".into())),
            classify_cninfo_source_error(CninfoError::Incomplete("TEST_CODE".into())),
        ];
        assert_eq!(source_errors.len(), 9);
    }
}
