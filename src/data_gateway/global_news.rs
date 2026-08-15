//! BR-066/BR-133/BR-137/BR-166/BR-172 evidence-preserving global financial-news acquisition.

use super::review::{acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result};
use super::{BatchEvidence, GatewayBatch, GatewayError};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use magic_cls_rs::{ClsClient, ClsError};
use magic_eastmoney_rs::{EastmoneyClient, EastmoneyError};
use magic_jin10_rs::{Jin10Client, Jin10Error};
use magic_market_core::{
    DataBatch, NewsItem, NewsProvider, PositiveU32, ProviderId, SourceEvidence,
};
use magic_thepaper_rs::{ThePaperClient, ThePaperError};
use std::collections::HashSet;

const MAX_LIMIT: u32 = 20;

/// One released global-news provider and its immutable source contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalNewsProvider {
    Eastmoney,
    Cailianpress,
    Jin10,
    ThePaper,
}

impl GlobalNewsProvider {
    pub const fn provider_id(self) -> ProviderId {
        match self {
            Self::Eastmoney => ProviderId::Eastmoney,
            Self::Cailianpress => ProviderId::Cailianpress,
            Self::Jin10 => ProviderId::Jin10,
            Self::ThePaper => ProviderId::ThePaper,
        }
    }

    pub const fn source(self) -> &'static str {
        match self {
            Self::Eastmoney => "eastmoney-web",
            Self::Cailianpress => "cls-v1",
            Self::Jin10 => "jin10-flash-v1",
            Self::ThePaper => "thepaper-finance-v1",
        }
    }

    pub const fn feed_name(self) -> &'static str {
        match self {
            Self::Eastmoney => "eastmoney_global_news",
            Self::Cailianpress => "cls_global_news",
            Self::Jin10 => "jin10_global_news",
            Self::ThePaper => "thepaper_global_news",
        }
    }

    const fn capability(self) -> &'static str {
        match self {
            Self::Eastmoney => "GlobalNews-Eastmoney",
            Self::Cailianpress => "GlobalNews-CLS",
            Self::Jin10 => "GlobalNews-Jin10",
            Self::ThePaper => "GlobalNews-ThePaper",
        }
    }
}

/// One admitted global-news fact retaining its upstream record evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalNewsRecord {
    pub item_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub publisher: String,
    pub canonical_url: String,
    pub published_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub instruments: Vec<String>,
    pub topics: Vec<String>,
    pub language: String,
    pub evidence: SourceEvidence,
}

/// Production seam for all released typed global financial-news clients.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalNewsGateway;

impl GlobalNewsGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn global_news(
        &self,
        provider: GlobalNewsProvider,
        limit: u32,
    ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
        let capability = provider.capability();
        let provider_id = provider.provider_id();
        let request_hash =
            acquisition_request_hash(capability, &format!("{}:{limit}", provider.source()));
        // P4 M3 钩子: DATA_GATEWAY_GRPC=1 → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("GlobalNews") {
            Ok(Some(bridge)) => {
                let result = bridge.global_news_async().await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(provider_id);
                return audit_gateway_result(capability, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(capability, provider_id, &request_hash, Err(error));
            }
        }
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result =
                build_limit(provider, limit).and_then(|limit| fetch_and_admit(provider, limit));
            audit_gateway_result(capability, provider_id, &worker_hash, result)
        })
        .await;

        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    capability,
                    provider_id,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }
}

fn build_limit(provider: GlobalNewsProvider, limit: u32) -> Result<PositiveU32, GatewayError> {
    if limit > MAX_LIMIT {
        return Err(GatewayError::invalid_request(
            provider.capability(),
            format!("global-news limit {limit} exceeds {MAX_LIMIT}"),
        ));
    }
    PositiveU32::new(limit).map_err(|error| {
        GatewayError::invalid_request(
            provider.capability(),
            format!("invalid global-news limit: {error}"),
        )
    })
}

fn fetch_and_admit(
    provider: GlobalNewsProvider,
    limit: PositiveU32,
) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
    let batch = match provider {
        GlobalNewsProvider::Eastmoney => EastmoneyClient::new()
            .map_err(|error| eastmoney_gateway_error(provider, error))?
            .global_news(limit)
            .map_err(|error| eastmoney_gateway_error(provider, error))?,
        GlobalNewsProvider::Cailianpress => ClsClient::new()
            .map_err(|error| cls_gateway_error(provider, error))?
            .global_news(limit)
            .map_err(|error| cls_gateway_error(provider, error))?,
        GlobalNewsProvider::Jin10 => Jin10Client::new()
            .map_err(|error| jin10_gateway_error(provider, error))?
            .global_news(limit)
            .map_err(|error| jin10_gateway_error(provider, error))?,
        GlobalNewsProvider::ThePaper => ThePaperClient::new()
            .map_err(|error| thepaper_gateway_error(provider, error))?
            .global_news(limit)
            .map_err(|error| thepaper_gateway_error(provider, error))?,
    };
    admit_batch(provider, batch)
}

fn admit_batch(
    provider: GlobalNewsProvider,
    batch: DataBatch<NewsItem>,
) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
    let capability = provider.capability();
    let provider_id = provider.provider_id();
    if batch.provenance().source() != provider.source() {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider_id),
            format!(
                "unexpected global-news source {:?}",
                batch.provenance().source()
            ),
        ));
    }
    if !batch.quality().is_complete() {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider_id),
            format!(
                "incomplete global-news batch: {}",
                batch.quality().issues().join("; ")
            ),
        ));
    }
    let evidence = BatchEvidence::from_provenance(provider_id, batch.provenance())?;
    let observed_at = parse_observed_at(provider, &evidence.observed_at)?;
    let batch_source_at = evidence.source_at.as_deref().ok_or_else(|| {
        GatewayError::invalid_evidence(
            capability,
            Some(provider_id),
            "global-news batch source time is missing",
        )
    })?;
    let parsed_batch_source_at = parse_provider_time(provider, batch_source_at)?;
    if parsed_batch_source_at > observed_at {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider_id),
            "global-news batch source time is after observation time",
        ));
    }
    if batch.records().is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }

    let mut item_ids = HashSet::with_capacity(batch.records().len());
    let mut urls = HashSet::with_capacity(batch.records().len());
    let mut previous_published_at: Option<DateTime<Utc>> = None;
    let mut records = Vec::with_capacity(batch.records().len());

    for item in batch.into_records() {
        validate_record_evidence(provider, &item, &evidence)?;
        let published_at = parse_provider_time(provider, item.published_at.as_str())?;
        if published_at > observed_at {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider_id),
                format!(
                    "global-news item {} was published after observation",
                    item.item_id.as_str()
                ),
            ));
        }
        if previous_published_at.is_some_and(|previous| previous < published_at) {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider_id),
                "global-news records are not ordered newest first",
            ));
        }
        previous_published_at = Some(published_at);

        let item_id = item.item_id.as_str().to_owned();
        let canonical_url = item.canonical_url.as_str().to_owned();
        if !item_ids.insert(item_id.clone()) {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider_id),
                format!("duplicate global-news item ID {item_id}"),
            ));
        }
        if !urls.insert(canonical_url.clone()) {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider_id),
                format!("duplicate global-news canonical URL {canonical_url}"),
            ));
        }

        records.push(GlobalNewsRecord {
            item_id,
            title: item.title.as_str().to_owned(),
            summary: item.summary.map(|value| value.as_str().to_owned()),
            content: item.content.map(|value| value.as_str().to_owned()),
            publisher: item.publisher.as_str().to_owned(),
            canonical_url,
            published_at,
            observed_at,
            instruments: item
                .instruments
                .into_iter()
                .map(|instrument| instrument.code().to_owned())
                .collect(),
            topics: item
                .topics
                .into_iter()
                .map(|topic| topic.as_str().to_owned())
                .collect(),
            language: item.language.as_str().to_owned(),
            evidence: item.evidence,
        });
    }

    if records
        .first()
        .is_none_or(|record| record.published_at != parsed_batch_source_at)
    {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider_id),
            "global-news batch source time differs from its newest record",
        ));
    }

    Ok(GatewayBatch::Available { records, evidence })
}

fn validate_record_evidence(
    provider: GlobalNewsProvider,
    item: &NewsItem,
    batch: &BatchEvidence,
) -> Result<(), GatewayError> {
    let capability = provider.capability();
    let provider_id = provider.provider_id();
    if item.evidence.provider() != provider_id
        || item.evidence.batch_id() != batch.batch_id
        || item.evidence.observed_at() != batch.observed_at
        || item.evidence.source_at() != Some(item.published_at.as_str())
    {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider_id),
            format!(
                "global-news item {} evidence differs from its batch",
                item.item_id.as_str()
            ),
        ));
    }
    Ok(())
}

fn parse_provider_time(
    provider: GlobalNewsProvider,
    value: &str,
) -> Result<DateTime<Utc>, GatewayError> {
    let parsed = if provider == GlobalNewsProvider::Eastmoney {
        let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").map_err(|error| {
            GatewayError::invalid_evidence(
                provider.capability(),
                Some(provider.provider_id()),
                format!("invalid Eastmoney provider time {value:?}: {error}"),
            )
        })?;
        let china = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
            GatewayError::invalid_evidence(
                provider.capability(),
                Some(provider.provider_id()),
                "UTC+08:00 offset is unavailable",
            )
        })?;
        china.from_local_datetime(&naive).single()
    } else {
        return DateTime::parse_from_rfc3339(value)
            .map(|timestamp| timestamp.with_timezone(&Utc))
            .map_err(|error| {
                GatewayError::invalid_evidence(
                    provider.capability(),
                    Some(provider.provider_id()),
                    format!("invalid provider time {value:?}: {error}"),
                )
            });
    };
    parsed
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .ok_or_else(|| {
            GatewayError::invalid_evidence(
                provider.capability(),
                Some(provider.provider_id()),
                format!("ambiguous provider time {value:?}"),
            )
        })
}

fn parse_observed_at(
    provider: GlobalNewsProvider,
    value: &str,
) -> Result<DateTime<Utc>, GatewayError> {
    let (seconds, nanos) = value.split_once('.').ok_or_else(|| {
        GatewayError::invalid_evidence(
            provider.capability(),
            Some(provider.provider_id()),
            format!("invalid global-news observation time {value:?}"),
        )
    })?;
    if nanos.len() != 9 || !nanos.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GatewayError::invalid_evidence(
            provider.capability(),
            Some(provider.provider_id()),
            format!("invalid global-news observation nanoseconds {value:?}"),
        ));
    }
    let seconds = seconds.parse::<i64>().map_err(|error| {
        GatewayError::invalid_evidence(
            provider.capability(),
            Some(provider.provider_id()),
            format!("invalid global-news observation seconds {value:?}: {error}"),
        )
    })?;
    let nanos = nanos.parse::<u32>().map_err(|error| {
        GatewayError::invalid_evidence(
            provider.capability(),
            Some(provider.provider_id()),
            format!("invalid global-news observation nanoseconds {value:?}: {error}"),
        )
    })?;
    DateTime::from_timestamp(seconds, nanos).ok_or_else(|| {
        GatewayError::invalid_evidence(
            provider.capability(),
            Some(provider.provider_id()),
            format!("global-news observation time is out of range {value:?}"),
        )
    })
}

fn provider_error(
    provider: GlobalNewsProvider,
    category: &'static str,
    message: impl Into<String>,
) -> GatewayError {
    let message = message.into();
    match category {
        "invalid_request" => GatewayError::invalid_request(provider.capability(), message),
        "transport" => GatewayError::unavailable(
            provider.capability(),
            Some(provider.provider_id()),
            true,
            message,
        ),
        "unsupported" => GatewayError::classified(
            provider.capability(),
            Some(provider.provider_id()),
            "unsupported",
            "unsupported",
            false,
            message,
        ),
        _ => GatewayError::invalid_evidence(
            provider.capability(),
            Some(provider.provider_id()),
            message,
        ),
    }
}

fn eastmoney_gateway_error(provider: GlobalNewsProvider, error: EastmoneyError) -> GatewayError {
    provider_error(provider, error.category(), error.to_string())
}

fn cls_gateway_error(provider: GlobalNewsProvider, error: ClsError) -> GatewayError {
    let category = match error {
        ClsError::InvalidRequest(_) => "invalid_request",
        ClsError::Transport(_) => "transport",
        ClsError::Unsupported(_) => "unsupported",
        ClsError::Decode(_) | ClsError::Protocol(_) | ClsError::Core(_) => "protocol",
    };
    provider_error(provider, category, error.to_string())
}

fn jin10_gateway_error(provider: GlobalNewsProvider, error: Jin10Error) -> GatewayError {
    let category = match error {
        Jin10Error::InvalidRequest(_) => "invalid_request",
        Jin10Error::Transport(_) => "transport",
        Jin10Error::Unsupported(_) => "unsupported",
        Jin10Error::Decode(_) | Jin10Error::Protocol(_) | Jin10Error::Core(_) => "protocol",
    };
    provider_error(provider, category, error.to_string())
}

fn thepaper_gateway_error(provider: GlobalNewsProvider, error: ThePaperError) -> GatewayError {
    let category = match error {
        ThePaperError::InvalidRequest(_) => "invalid_request",
        ThePaperError::Transport(_) => "transport",
        ThePaperError::Unsupported(_) => "unsupported",
        ThePaperError::Decode(_) | ThePaperError::Protocol(_) | ThePaperError::Core(_) => {
            "protocol"
        }
    };
    provider_error(provider, category, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use magic_market_core::{HttpsUrl, NonEmptyText, Provenance};

    fn observed_at(after: &str) -> String {
        let timestamp = DateTime::parse_from_rfc3339(after)
            .expect("TEST_CODE observation timestamp")
            .with_timezone(&Utc);
        format!(
            "{}.{:09}",
            timestamp.timestamp(),
            timestamp.timestamp_subsec_nanos()
        )
    }

    fn item(
        provider: GlobalNewsProvider,
        batch_id: &str,
        observed_at: &str,
        id: &str,
        published_at: &str,
    ) -> NewsItem {
        NewsItem {
            item_id: NonEmptyText::new(id).expect("TEST_CODE item ID"),
            title: NonEmptyText::new(format!("TEST_CODE title {id}")).expect("TEST_CODE title"),
            summary: None,
            content: None,
            publisher: NonEmptyText::new("TEST_CODE publisher").expect("TEST_CODE publisher"),
            canonical_url: HttpsUrl::new(format!("https://example.com/{id}"))
                .expect("TEST_CODE URL"),
            published_at: NonEmptyText::new(published_at).expect("TEST_CODE published_at"),
            instruments: Vec::new(),
            topics: Vec::new(),
            language: NonEmptyText::new("zh-CN").expect("TEST_CODE language"),
            evidence: SourceEvidence::new(provider.provider_id(), observed_at, batch_id)
                .and_then(|evidence| evidence.with_source_at(published_at))
                .expect("TEST_CODE evidence"),
        }
    }

    fn batch(provider: GlobalNewsProvider, times: &[&str]) -> DataBatch<NewsItem> {
        let batch_id = format!("TEST_CODE_{}_batch", provider.feed_name());
        let observed_at = observed_at("2026-07-25T12:00:00+08:00");
        let records = times
            .iter()
            .enumerate()
            .map(|(index, time)| {
                item(
                    provider,
                    &batch_id,
                    &observed_at,
                    &format!("TEST_CODE_{index}"),
                    time,
                )
            })
            .collect();
        let provenance = Provenance::new(provider.source(), observed_at)
            .and_then(|provenance| provenance.with_source_at(times[0]))
            .and_then(|provenance| provenance.with_batch_id(batch_id))
            .expect("TEST_CODE provenance");
        DataBatch::strict(records, provenance)
    }

    fn provenance(
        provider: GlobalNewsProvider,
        observed_at: &str,
        source_at: Option<&str>,
        batch_id: &str,
    ) -> Provenance {
        let mut provenance =
            Provenance::new(provider.source(), observed_at).expect("TEST_CODE provenance source");
        if let Some(source_at) = source_at {
            provenance = provenance
                .with_source_at(source_at)
                .expect("TEST_CODE provenance source_at");
        }
        provenance
            .with_batch_id(batch_id)
            .expect("TEST_CODE provenance batch_id")
    }

    #[test]
    fn admits_every_released_provider_contract() {
        let cases = [
            (
                GlobalNewsProvider::Eastmoney,
                vec!["2026-07-25 11:00", "2026-07-25 10:30"],
            ),
            (
                GlobalNewsProvider::Cailianpress,
                vec!["2026-07-25T11:00:00+08:00", "2026-07-25T10:30:00+08:00"],
            ),
            (
                GlobalNewsProvider::Jin10,
                vec!["2026-07-25T11:00:00+08:00", "2026-07-25T10:30:00+08:00"],
            ),
            (
                GlobalNewsProvider::ThePaper,
                vec!["2026-07-25T11:00:00+08:00", "2026-07-25T10:30:00+08:00"],
            ),
        ];

        for (provider, times) in cases {
            let admitted = admit_batch(provider, batch(provider, &times))
                .expect("TEST_CODE admitted provider batch");
            assert_eq!(admitted.evidence().provider, provider.provider_id());
            assert_eq!(admitted.records().len(), 2);
            assert_eq!(admitted.records()[0].item_id, "TEST_CODE_0");
        }
    }

    #[test]
    fn rejects_future_and_out_of_order_records() {
        let provider = GlobalNewsProvider::Jin10;
        let future = admit_batch(provider, batch(provider, &["2026-07-25T12:01:00+08:00"]))
            .expect_err("TEST_CODE future record must fail");
        assert_eq!(future.reason_code(), "invalid_evidence");

        let unordered = admit_batch(
            provider,
            batch(
                provider,
                &["2026-07-25T10:00:00+08:00", "2026-07-25T11:00:00+08:00"],
            ),
        )
        .expect_err("TEST_CODE unordered records must fail");
        assert_eq!(unordered.reason_code(), "invalid_evidence");
    }

    #[test]
    fn rejects_zero_and_oversized_limits() {
        for limit in [0, 21] {
            let error = build_limit(GlobalNewsProvider::Eastmoney, limit)
                .expect_err("TEST_CODE invalid limit");
            assert_eq!(error.reason_code(), "invalid_request");
        }
    }

    #[test]
    fn provider_metadata_and_time_contracts_are_explicit() {
        let providers = [
            GlobalNewsProvider::Eastmoney,
            GlobalNewsProvider::Cailianpress,
            GlobalNewsProvider::Jin10,
            GlobalNewsProvider::ThePaper,
        ];
        for provider in providers {
            assert!(!provider.source().is_empty());
            assert!(!provider.feed_name().is_empty());
            assert!(!provider.capability().is_empty());
        }

        assert_eq!(
            parse_provider_time(GlobalNewsProvider::Eastmoney, "2026-07-25 11:00")
                .unwrap()
                .to_rfc3339(),
            "2026-07-25T03:00:00+00:00"
        );
        assert!(parse_provider_time(GlobalNewsProvider::Eastmoney, "bad").is_err());
        assert!(parse_provider_time(GlobalNewsProvider::Jin10, "bad").is_err());
        assert!(parse_observed_at(GlobalNewsProvider::Jin10, "bad").is_err());
        assert!(parse_observed_at(GlobalNewsProvider::Jin10, "1.bad").is_err());
        assert!(parse_observed_at(GlobalNewsProvider::Jin10, "bad.000000000").is_err());
        assert!(
            parse_observed_at(GlobalNewsProvider::Jin10, "999999999999999999.000000000").is_err()
        );
    }

    #[test]
    fn verified_empty_and_batch_level_evidence_failures_are_distinct() {
        let provider = GlobalNewsProvider::Jin10;
        let observed = observed_at("2026-07-25T12:00:00+08:00");
        let empty = DataBatch::strict(
            Vec::<NewsItem>::new(),
            provenance(
                provider,
                &observed,
                Some("2026-07-25T11:00:00+08:00"),
                "TEST_CODE_empty",
            ),
        );
        assert!(matches!(
            admit_batch(provider, empty).unwrap(),
            GatewayBatch::VerifiedEmpty(_)
        ));

        let wrong_source = Provenance::new("TEST_CODE_wrong-source", observed.clone())
            .unwrap()
            .with_source_at("2026-07-25T11:00:00+08:00")
            .unwrap()
            .with_batch_id("TEST_CODE_wrong")
            .unwrap();
        assert!(admit_batch(
            provider,
            DataBatch::strict(Vec::<NewsItem>::new(), wrong_source)
        )
        .is_err());

        let missing_source_at = DataBatch::strict(
            Vec::<NewsItem>::new(),
            provenance(provider, &observed, None, "TEST_CODE_missing_source_at"),
        );
        assert!(admit_batch(provider, missing_source_at).is_err());

        let future_source_at = DataBatch::strict(
            Vec::<NewsItem>::new(),
            provenance(
                provider,
                &observed,
                Some("2026-07-25T12:01:00+08:00"),
                "TEST_CODE_future_source_at",
            ),
        );
        assert!(admit_batch(provider, future_source_at).is_err());

        let partial = DataBatch::best_effort(
            Vec::<NewsItem>::new(),
            provenance(
                provider,
                &observed,
                Some("2026-07-25T11:00:00+08:00"),
                "TEST_CODE_partial",
            ),
            vec!["TEST_CODE missing provider field".to_string()],
        )
        .unwrap();
        assert!(admit_batch(provider, partial).is_err());
    }

    #[test]
    fn duplicate_identity_url_and_record_evidence_are_rejected() {
        let provider = GlobalNewsProvider::Jin10;
        let batch_id = "TEST_CODE_identity_batch";
        let observed = observed_at("2026-07-25T12:00:00+08:00");
        let newest = "2026-07-25T11:00:00+08:00";
        let older = "2026-07-25T10:00:00+08:00";
        let first = item(provider, batch_id, &observed, "TEST_CODE_same", newest);

        let duplicate_id = vec![
            first.clone(),
            item(provider, batch_id, &observed, "TEST_CODE_same", older),
        ];
        assert!(admit_batch(
            provider,
            DataBatch::strict(
                duplicate_id,
                provenance(provider, &observed, Some(newest), batch_id)
            )
        )
        .is_err());

        let mut duplicate_url = item(provider, batch_id, &observed, "TEST_CODE_other", older);
        duplicate_url.canonical_url = first.canonical_url.clone();
        assert!(admit_batch(
            provider,
            DataBatch::strict(
                vec![first.clone(), duplicate_url],
                provenance(provider, &observed, Some(newest), batch_id)
            )
        )
        .is_err());

        let mut wrong_evidence = first;
        wrong_evidence.evidence = SourceEvidence::new(ProviderId::Eastmoney, &observed, batch_id)
            .and_then(|evidence| evidence.with_source_at(newest))
            .unwrap();
        assert!(admit_batch(
            provider,
            DataBatch::strict(
                vec![wrong_evidence],
                provenance(provider, &observed, Some(newest), batch_id)
            )
        )
        .is_err());

        let source_mismatch = item(provider, batch_id, &observed, "TEST_CODE_one", newest);
        assert!(admit_batch(
            provider,
            DataBatch::strict(
                vec![source_mismatch],
                provenance(provider, &observed, Some(older), batch_id)
            )
        )
        .is_err());
    }

    #[test]
    fn provider_failures_keep_stable_operational_classification() {
        let provider = GlobalNewsProvider::Cailianpress;
        let cases = [
            provider_error(provider, "invalid_request", "TEST_CODE invalid"),
            provider_error(provider, "transport", "TEST_CODE transport"),
            provider_error(provider, "unsupported", "TEST_CODE unsupported"),
            provider_error(provider, "protocol", "TEST_CODE protocol"),
        ];
        assert_eq!(cases[0].audit_outcome(), "invalid_request");
        assert_eq!(cases[1].audit_outcome(), "unavailable");
        assert!(cases[1].retryable());
        assert_eq!(cases[2].audit_outcome(), "unsupported");
        assert_eq!(cases[3].audit_outcome(), "partial");

        for error in [
            cls_gateway_error(provider, ClsError::InvalidRequest("TEST_CODE".into())),
            cls_gateway_error(provider, ClsError::Transport("TEST_CODE".into())),
            cls_gateway_error(provider, ClsError::Unsupported("TEST_CODE".into())),
            cls_gateway_error(provider, ClsError::Decode("TEST_CODE".into())),
            cls_gateway_error(provider, ClsError::Protocol("TEST_CODE".into())),
            jin10_gateway_error(
                GlobalNewsProvider::Jin10,
                Jin10Error::InvalidRequest("TEST_CODE".into()),
            ),
            jin10_gateway_error(
                GlobalNewsProvider::Jin10,
                Jin10Error::Transport("TEST_CODE".into()),
            ),
            jin10_gateway_error(
                GlobalNewsProvider::Jin10,
                Jin10Error::Unsupported("TEST_CODE".into()),
            ),
            jin10_gateway_error(
                GlobalNewsProvider::Jin10,
                Jin10Error::Protocol("TEST_CODE".into()),
            ),
            thepaper_gateway_error(
                GlobalNewsProvider::ThePaper,
                ThePaperError::InvalidRequest("TEST_CODE".into()),
            ),
            thepaper_gateway_error(
                GlobalNewsProvider::ThePaper,
                ThePaperError::Transport("TEST_CODE".into()),
            ),
            thepaper_gateway_error(
                GlobalNewsProvider::ThePaper,
                ThePaperError::Unsupported("TEST_CODE".into()),
            ),
            thepaper_gateway_error(
                GlobalNewsProvider::ThePaper,
                ThePaperError::Decode("TEST_CODE".into()),
            ),
            eastmoney_gateway_error(
                GlobalNewsProvider::Eastmoney,
                EastmoneyError::ResponseTooLarge { limit: 1 },
            ),
        ] {
            assert!(!error.reason_code().is_empty());
        }
    }
}
