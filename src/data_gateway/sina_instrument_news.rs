//! BR-066/BR-164/BR-172 evidence-preserving Sina instrument-news gateway.

use chrono::{DateTime, Utc};
use magic_market_core::{
    DataBatch, Exchange, InstrumentDateRangeRequest, IsoDate, NewsItem as CoreNewsItem,
    NewsProvider, PositiveU32, ProviderId, SourceEvidence,
};
use magic_market_router::{
    AcceptancePolicy, AttemptStatus, FailureKind, InstrumentNewsRouter, RouterError, SourceError,
    SourceFn,
};
use magic_sina_rs::{SinaClient, SinaError};

use super::review::{
    acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result, BatchEvidence,
    GatewayBatch, GatewayError,
};
use crate::data_provider::news_item::{content_hash, NewsItem};

const CAPABILITY: &str = "SinaInstrumentNews";
const SOURCE: &str = "sina-company-news";
const REQUEST_LIMIT: u32 = 100;

/// One admitted Sina company-news row with the legacy persistence projection
/// and its immutable upstream evidence.
#[derive(Debug, Clone)]
pub struct SinaInstrumentNewsRecord {
    persistence_item: NewsItem,
    evidence: SourceEvidence,
}

impl SinaInstrumentNewsRecord {
    pub fn persistence_item(&self) -> &NewsItem {
        &self.persistence_item
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

/// BR-163 production seam for bounded Sina company-news history.
#[derive(Debug, Clone, Copy, Default)]
pub struct SinaInstrumentNewsGateway;

impl SinaInstrumentNewsGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn instrument_news_in_range(
        &self,
        code: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
        let code = code.to_owned();
        let request_hash = acquisition_request_hash(
            CAPABILITY,
            &format!(
                "{code}:{}:{}:{REQUEST_LIMIT}",
                from.to_rfc3339(),
                to.to_rfc3339()
            ),
        );
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = build_request(&code, &from, &to).and_then(|(request, storage_code)| {
                fetch_and_admit_sina_batch(&request, &storage_code, from, to)
            });
            audit_gateway_result(CAPABILITY, ProviderId::Sina, &worker_hash, result)
        })
        .await;

        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    CAPABILITY,
                    ProviderId::Sina,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }
}

fn build_request(
    code: &str,
    from: &DateTime<Utc>,
    to: &DateTime<Utc>,
) -> Result<(InstrumentDateRangeRequest, String), GatewayError> {
    if from > to {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            "UTC start must not exceed end",
        ));
    }
    #[cfg(test)]
    let resolved = super::instrument_identity::resolve_test_equity(code, None);
    #[cfg(not(test))]
    let resolved = super::instrument_identity::resolve_production_equity(code, None);
    let identity =
        resolved.map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    identity
        .require_a_share()
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    if identity.instrument().exchange() == Exchange::Beijing {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            "Sina company-news is not verified for Beijing A-shares",
        ));
    }
    let instrument = identity.instrument().clone();
    let limit = PositiveU32::new(REQUEST_LIMIT).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid request limit: {error}"))
    })?;
    let start = IsoDate::new(from.format("%Y-%m-%d").to_string()).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid UTC start date: {error}"))
    })?;
    let end = IsoDate::new(to.format("%Y-%m-%d").to_string()).map_err(|error| {
        GatewayError::invalid_request(CAPABILITY, format!("invalid UTC end date: {error}"))
    })?;
    let request = InstrumentDateRangeRequest::new(instrument, limit)
        .and_then(|request| request.with_range(start, end))
        .map_err(|error| {
            GatewayError::invalid_request(
                CAPABILITY,
                format!("invalid date-range request: {error}"),
            )
        })?;
    Ok((request, code.to_owned()))
}

fn fetch_and_admit_sina_batch(
    request: &InstrumentDateRangeRequest,
    storage_code: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
    let client = SinaClient::new().map_err(sina_gateway_error)?;
    let batch = client
        .instrument_news(request)
        .map_err(sina_gateway_error)?;
    admit_sina_batch(batch, request, storage_code, from, to)
}

fn admit_sina_batch(
    batch: DataBatch<CoreNewsItem>,
    request: &InstrumentDateRangeRequest,
    storage_code: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
    validate_batch_provenance(&batch)?;
    let evidence = BatchEvidence::from_provenance(ProviderId::Sina, batch.provenance())?;
    if batch.records().is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }

    let captured = batch.clone();
    let source = SourceFn::new(
        ProviderId::Sina,
        move |_request: &InstrumentDateRangeRequest| {
            Ok::<DataBatch<CoreNewsItem>, SourceError>(captured.clone())
        },
    );
    let mut router = InstrumentNewsRouter::new(
        AcceptancePolicy::new()
            .with_require_complete(true)
            .with_require_source_at(true),
    );
    router
        .register(source)
        .map_err(|error| gateway_router_error(Some(ProviderId::Sina), error))?;
    let outcome = router
        .route(request)
        .map_err(|error| gateway_router_error(Some(ProviderId::Sina), error))?;
    if outcome.selected_provider() != ProviderId::Sina {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news Router selected a different provider",
        ));
    }
    let routed = outcome.into_batch();
    let routed_evidence = BatchEvidence::from_provenance(ProviderId::Sina, routed.provenance())?;
    if routed_evidence != evidence {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news Router changed provider provenance",
        ));
    }

    let mut records = Vec::with_capacity(routed.records().len());
    for record in routed.into_records() {
        let published_at = validate_record(&record, request, &evidence)?;
        if published_at < from || published_at > to {
            continue;
        }
        let observed_at = parse_observed_at(record.evidence.observed_at())?;
        let summary = record
            .summary
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or("")
            .to_owned();
        let title = record.title.as_str().to_owned();
        let persistence_item = NewsItem {
            source: "sina_stock".to_owned(),
            external_id: record.item_id.as_str().to_owned(),
            category: "个股新闻".to_owned(),
            code: Some(storage_code.to_owned()),
            title: title.clone(),
            summary: summary.clone(),
            url: record.canonical_url.as_str().to_owned(),
            source_name: record.publisher.as_str().to_owned(),
            published_at,
            fetched_at: observed_at,
            content_hash: content_hash(&title, &summary),
        };
        records.push(SinaInstrumentNewsRecord {
            persistence_item,
            evidence: record.evidence,
        });
    }

    if records.is_empty() {
        Ok(GatewayBatch::VerifiedEmpty(evidence))
    } else {
        Ok(GatewayBatch::Available { records, evidence })
    }
}

fn validate_batch_provenance(batch: &DataBatch<CoreNewsItem>) -> Result<(), GatewayError> {
    if batch.provenance().source() != SOURCE {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            format!(
                "unexpected instrument-news source {:?}",
                batch.provenance().source()
            ),
        ));
    }
    if !batch.quality().is_complete() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            format!(
                "incomplete instrument-news batch: {}",
                batch.quality().issues().join("; ")
            ),
        ));
    }
    let source_at = batch.provenance().source_at().ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news batch source time is missing",
        )
    })?;
    parse_published_at(source_at)?;
    parse_observed_at(batch.provenance().fetched_at())?;
    Ok(())
}

fn validate_record(
    record: &CoreNewsItem,
    request: &InstrumentDateRangeRequest,
    batch_evidence: &BatchEvidence,
) -> Result<DateTime<Utc>, GatewayError> {
    if record.evidence.provider() != ProviderId::Sina {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news record provider is not Sina",
        ));
    }
    if record.evidence.batch_id() != batch_evidence.batch_id {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news record batch ID differs from batch provenance",
        ));
    }
    if record.instruments.as_slice() != std::slice::from_ref(request.instrument()) {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news record instrument differs from request",
        ));
    }
    if record.item_id.as_str() != record.canonical_url.as_str() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news item identity differs from canonical URL",
        ));
    }
    let source_at = record.evidence.source_at().ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news record source time is missing",
        )
    })?;
    if source_at != record.published_at.as_str() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news record source time differs from published time",
        ));
    }
    let published_at = parse_published_at(record.published_at.as_str())?;
    let observed_at = parse_observed_at(record.evidence.observed_at())?;
    if observed_at < published_at {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            "instrument-news observation precedes publication",
        ));
    }
    Ok(published_at)
}

fn parse_published_at(value: &str) -> Result<DateTime<Utc>, GatewayError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Sina),
                format!("invalid instrument-news provider time {value:?}: {error}"),
            )
        })
}

fn parse_observed_at(value: &str) -> Result<DateTime<Utc>, GatewayError> {
    let (seconds, nanos) = value.split_once('.').ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            format!("invalid instrument-news observation time {value:?}"),
        )
    })?;
    if nanos.len() != 9 || !nanos.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            format!("invalid instrument-news observation precision {value:?}"),
        ));
    }
    let seconds = seconds.parse::<i64>().map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            format!("invalid instrument-news observation seconds {value:?}: {error}"),
        )
    })?;
    let nanos = nanos.parse::<u32>().map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            format!("invalid instrument-news observation nanos {value:?}: {error}"),
        )
    })?;
    DateTime::from_timestamp(seconds, nanos).ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Sina),
            format!("instrument-news observation time is out of range {value:?}"),
        )
    })
}

fn sina_gateway_error(error: SinaError) -> GatewayError {
    let message = error.to_string();
    match error {
        SinaError::InvalidRequest(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Sina),
            "invalid_request",
            "provider_invalid_request",
            false,
            message,
        ),
        SinaError::Transport(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Sina),
            "unavailable",
            "provider_transport",
            true,
            message,
        ),
        SinaError::Unsupported(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Sina),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        SinaError::Decode(_) | SinaError::Protocol(_) | SinaError::Core(_) => {
            GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Sina),
                "partial",
                "provider_batch_rejected",
                false,
                message,
            )
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use magic_market_core::{
        AssetClass, DataBatch, Exchange, HttpsUrl, InstrumentId, IsoDate, NewsItem as CoreNewsItem,
        NonEmptyText, Provenance, ProviderId, SourceEvidence,
    };

    fn instant(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("valid test instant")
            .with_timezone(&Utc)
    }

    fn test_provenance(published_at: &str, batch_id: &str) -> Provenance {
        let observed_at = format!(
            "{}.000000000",
            instant("2026-07-25T10:30:00+08:00").timestamp()
        );
        Provenance::new("sina-company-news", &observed_at)
            .unwrap()
            .with_source_at(published_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap()
    }

    fn test_batch_with_ids(
        published_at: &str,
        record_batch_id: &str,
        provenance_batch_id: &str,
    ) -> DataBatch<CoreNewsItem> {
        let observed_at = format!(
            "{}.000000000",
            instant("2026-07-25T10:30:00+08:00").timestamp()
        );
        let instrument =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600519", AssetClass::Equity).unwrap();
        let evidence = SourceEvidence::new(ProviderId::Sina, &observed_at, record_batch_id)
            .unwrap()
            .with_source_at(published_at)
            .unwrap();
        let item = CoreNewsItem {
            item_id: NonEmptyText::new("https://finance.sina.com.cn/test-news").unwrap(),
            title: NonEmptyText::new("TEST_CODE company news").unwrap(),
            summary: None,
            content: None,
            publisher: NonEmptyText::new("新浪财经").unwrap(),
            canonical_url: HttpsUrl::new("https://finance.sina.com.cn/test-news").unwrap(),
            published_at: NonEmptyText::new(published_at).unwrap(),
            instruments: vec![instrument],
            topics: Vec::new(),
            language: NonEmptyText::new("zh-CN").unwrap(),
            evidence,
        };
        DataBatch::strict(
            vec![item],
            test_provenance(published_at, provenance_batch_id),
        )
    }

    fn test_batch(published_at: &str) -> DataBatch<CoreNewsItem> {
        test_batch_with_ids(
            published_at,
            "TEST_CODE_sina-news-batch",
            "TEST_CODE_sina-news-batch",
        )
    }

    #[test]
    fn available_preserves_persistence_fields_and_immutable_evidence() {
        let from = instant("2026-07-25T09:00:00+08:00");
        let to = instant("2026-07-25T11:00:00+08:00");
        let (request, storage_code) =
            build_request("TEST_CODE_600519", &from, &to).expect("valid request");

        let result = admit_sina_batch(
            test_batch("2026-07-25T10:00:00+08:00"),
            &request,
            &storage_code,
            from,
            to,
        )
        .expect("available batch");

        let GatewayBatch::Available { records, evidence } = result else {
            panic!("expected available batch");
        };
        assert_eq!(records.len(), 1);
        assert_eq!(evidence.provider, ProviderId::Sina);
        assert_eq!(evidence.source, "sina-company-news");
        assert_eq!(evidence.batch_id, "TEST_CODE_sina-news-batch");
        assert_eq!(
            records[0].persistence_item().code.as_deref(),
            Some("TEST_CODE_600519")
        );
        assert_eq!(records[0].persistence_item().summary, "");
        assert_eq!(records[0].evidence().provider(), ProviderId::Sina);
        assert_eq!(
            records[0].evidence().batch_id(),
            "TEST_CODE_sina-news-batch"
        );
        assert_eq!(
            records[0].evidence().source_at(),
            Some("2026-07-25T10:00:00+08:00")
        );
    }

    #[test]
    fn provider_proven_empty_preserves_batch_evidence() {
        let from = instant("2026-07-25T09:00:00+08:00");
        let to = instant("2026-07-25T11:00:00+08:00");
        let (request, storage_code) =
            build_request("TEST_CODE_600519", &from, &to).expect("valid request");
        let empty = DataBatch::strict(
            Vec::<CoreNewsItem>::new(),
            test_provenance("2026-07-25T10:00:00+08:00", "TEST_CODE_verified-empty"),
        );

        let result =
            admit_sina_batch(empty, &request, &storage_code, from, to).expect("verified empty");
        let GatewayBatch::VerifiedEmpty(evidence) = result else {
            panic!("expected verified empty batch");
        };
        assert_eq!(evidence.provider, ProviderId::Sina);
        assert_eq!(evidence.batch_id, "TEST_CODE_verified-empty");
    }

    #[test]
    fn exact_utc_filter_returns_verified_empty_without_changing_evidence() {
        let from = instant("2026-07-25T09:00:00+08:00");
        let to = instant("2026-07-25T11:00:00+08:00");
        let (request, storage_code) =
            build_request("TEST_CODE_600519", &from, &to).expect("valid request");

        let result = admit_sina_batch(
            test_batch("2026-07-25T08:59:00+08:00"),
            &request,
            &storage_code,
            from,
            to,
        )
        .expect("filtered verified empty");

        let GatewayBatch::VerifiedEmpty(evidence) = result else {
            panic!("expected verified empty batch");
        };
        assert_eq!(evidence.batch_id, "TEST_CODE_sina-news-batch");
        assert_eq!(
            evidence.source_at.as_deref(),
            Some("2026-07-25T08:59:00+08:00")
        );
    }

    #[test]
    fn router_rejects_record_batch_identity_mismatch() {
        let from = instant("2026-07-25T09:00:00+08:00");
        let to = instant("2026-07-25T11:00:00+08:00");
        let (request, storage_code) =
            build_request("TEST_CODE_600519", &from, &to).expect("valid request");
        let batch = test_batch_with_ids(
            "2026-07-25T10:00:00+08:00",
            "TEST_CODE_wrong-record-batch",
            "TEST_CODE_sina-news-batch",
        );

        let error = admit_sina_batch(batch, &request, &storage_code, from, to)
            .expect_err("mismatched batch identity must fail");
        assert_eq!(error.audit_outcome(), "partial");
        assert_eq!(error.reason_code(), "router_batch_rejected");
        assert!(!error.retryable());
    }

    #[test]
    fn unsupported_and_transport_failures_remain_distinct() {
        let unsupported =
            sina_gateway_error(SinaError::Unsupported("TEST_CODE exchange".to_owned()));
        assert_eq!(unsupported.audit_outcome(), "unsupported");
        assert_eq!(unsupported.reason_code(), "provider_unsupported");
        assert!(!unsupported.retryable());

        let unavailable = sina_gateway_error(SinaError::Transport("TEST_CODE timeout".to_owned()));
        assert_eq!(unavailable.audit_outcome(), "unavailable");
        assert_eq!(unavailable.reason_code(), "provider_transport");
        assert!(unavailable.retryable());
    }

    #[test]
    fn request_builder_maps_all_a_share_exchanges_and_rejects_invalid_ranges() {
        let from = instant("2026-07-25T09:00:00+08:00");
        let to = instant("2026-07-25T11:00:00+08:00");
        for (code, exchange) in [
            ("TEST_CODE_600519", Exchange::Shanghai),
            ("TEST_CODE_000001", Exchange::Shenzhen),
        ] {
            let (request, storage_code) = build_request(code, &from, &to).unwrap();
            assert_eq!(request.instrument().exchange(), exchange);
            assert_eq!(request.instrument().code(), code);
            assert_eq!(storage_code, code);
            assert_eq!(request.start().map(IsoDate::as_str), Some("2026-07-25"));
            assert_eq!(request.end().map(IsoDate::as_str), Some("2026-07-25"));
        }
        assert!(build_request("TEST_CODE_920047", &from, &to).is_err());
        for code in [
            "TEST_CODE_430047",
            "TEST_CODE_830047",
            "TEST_CODE_200001",
            "TEST_CODE_900901",
        ] {
            assert!(build_request(code, &from, &to).is_err(), "{code}");
        }
        assert!(build_request("TEST_CODE_100001", &from, &to).is_err());
        assert!(build_request("TEST_CODE_60051A", &from, &to).is_err());
        assert!(build_request("TEST_CODE_600519", &to, &from).is_err());
    }

    #[test]
    fn provider_and_observation_time_parsers_reject_precision_loss() {
        assert_eq!(
            parse_published_at("2026-07-25T10:00:00+08:00")
                .unwrap()
                .timestamp(),
            instant("2026-07-25T10:00:00+08:00").timestamp()
        );
        assert!(parse_published_at("2026/07/25 10:00:00").is_err());

        let observed = format!(
            "{}.000000000",
            instant("2026-07-25T10:30:00+08:00").timestamp()
        );
        assert_eq!(
            parse_observed_at(&observed).unwrap(),
            instant("2026-07-25T10:30:00+08:00")
        );
        for invalid in [
            "TEST_CODE",
            "1784946600",
            "1784946600.000",
            "1784946600.00000000A",
        ] {
            assert!(parse_observed_at(invalid).is_err());
        }
    }

    #[test]
    fn record_validation_rejects_identity_and_temporal_mismatches() {
        let from = instant("2026-07-25T09:00:00+08:00");
        let to = instant("2026-07-25T11:00:00+08:00");
        let (request, _) = build_request("TEST_CODE_600519", &from, &to).unwrap();
        let batch = test_batch("2026-07-25T10:00:00+08:00");
        let evidence =
            BatchEvidence::from_provenance(ProviderId::Sina, batch.provenance()).unwrap();
        assert!(validate_record(&batch.records()[0], &request, &evidence).is_ok());

        let mut wrong_instrument = batch.records()[0].clone();
        wrong_instrument.instruments =
            vec![
                InstrumentId::new(Exchange::Shenzhen, "TEST_CODE_000001", AssetClass::Equity)
                    .unwrap(),
            ];
        assert!(validate_record(&wrong_instrument, &request, &evidence).is_err());

        let mut wrong_identity = batch.records()[0].clone();
        wrong_identity.item_id =
            NonEmptyText::new("https://finance.sina.com.cn/TEST_CODE-other").unwrap();
        assert!(validate_record(&wrong_identity, &request, &evidence).is_err());

        let future = test_batch("2026-07-25T11:00:00+08:00");
        let future_evidence =
            BatchEvidence::from_provenance(ProviderId::Sina, future.provenance()).unwrap();
        assert!(validate_record(&future.records()[0], &request, &future_evidence).is_err());
    }

    #[test]
    fn batch_provenance_and_provider_protocol_failures_are_explicit() {
        let wrong_source = Provenance::new("TEST_CODE_wrong-source", "1784946600.000000000")
            .unwrap()
            .with_source_at("2026-07-25T10:00:00+08:00")
            .unwrap()
            .with_batch_id("TEST_CODE_wrong-source")
            .unwrap();
        assert!(
            validate_batch_provenance(
                &DataBatch::<CoreNewsItem>::strict(Vec::new(), wrong_source,)
            )
            .is_err()
        );

        let missing_source_at = Provenance::new("sina-company-news", "1784946600.000000000")
            .unwrap()
            .with_batch_id("TEST_CODE_missing-source-time")
            .unwrap();
        assert!(
            validate_batch_provenance(&DataBatch::<CoreNewsItem>::strict(
                Vec::new(),
                missing_source_at,
            ))
            .is_err()
        );

        let invalid_request =
            sina_gateway_error(SinaError::InvalidRequest("TEST_CODE bad".to_owned()));
        assert_eq!(invalid_request.reason_code(), "provider_invalid_request");
        assert!(!invalid_request.retryable());
        let protocol = sina_gateway_error(SinaError::Protocol("TEST_CODE schema".to_owned()));
        assert_eq!(protocol.reason_code(), "provider_batch_rejected");
        assert_eq!(protocol.audit_outcome(), "partial");
        assert!(!protocol.retryable());
    }
}
