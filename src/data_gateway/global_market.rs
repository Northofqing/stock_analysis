//! BR-161 evidence-preserving global-index and foreign-exchange acquisition.

use super::review::{acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result};
use super::{BatchEvidence, GatewayBatch, GatewayError};
use chrono::{DateTime, Utc};
use magic_market_core::{
    DataBatch, ForeignExchangeProvider, FxPair, FxQuote, FxRequest, GlobalIndexCode,
    GlobalIndexProvider, GlobalIndexQuote, GlobalIndexRequest, ProviderId, RatioUnit,
};
use magic_sina_rs::{SinaClient, SinaError};
use std::collections::HashSet;

const INDEX_CAPABILITY: &str = "R-08-global-indices";
const FX_CAPABILITY: &str = "R-08-global-fx";
const SOURCE: &str = "sina-web";
const REALTIME_MAX_AGE_MILLIS: i64 = 5_000;

/// One admitted global-index quote with exact provider timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalIndexFact {
    pub code: GlobalIndexCode,
    pub name: String,
    pub value: f64,
    pub change: f64,
    pub change_percent: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// One admitted foreign-exchange quote with exact provider timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignExchangeFact {
    pub pair: FxPair,
    pub name: String,
    pub rate: f64,
    pub change: Option<f64>,
    pub change_percent: Option<f64>,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Production seam for the typed Sina global-market providers.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalMarketGateway;

impl GlobalMarketGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Acquires exactly Dow Jones, Nasdaq Composite and S&P 500.
    pub async fn us_indices(&self) -> Result<GatewayBatch<GlobalIndexFact>, GatewayError> {
        let request = GlobalIndexRequest::new(vec![
            GlobalIndexCode::DowJones,
            GlobalIndexCode::NasdaqComposite,
            GlobalIndexCode::Sp500,
        ])
        .map_err(|error| {
            GatewayError::invalid_request(
                INDEX_CAPABILITY,
                format!("invalid US index request: {error}"),
            )
        })?;
        let request_hash =
            acquisition_request_hash(INDEX_CAPABILITY, "DowJones,NasdaqComposite,Sp500");
        let worker_request_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = fetch_and_admit_indices(request);
            audit_gateway_result(
                INDEX_CAPABILITY,
                ProviderId::Sina,
                &worker_request_hash,
                result,
            )
        })
        .await;

        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    INDEX_CAPABILITY,
                    ProviderId::Sina,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }

    /// Acquires exactly the USD/CNY quote.
    pub async fn usd_cny(&self) -> Result<GatewayBatch<ForeignExchangeFact>, GatewayError> {
        let request = FxRequest::new(vec![FxPair::UsdCny]).map_err(|error| {
            GatewayError::invalid_request(
                FX_CAPABILITY,
                format!("invalid USD/CNY request: {error}"),
            )
        })?;
        let request_hash = acquisition_request_hash(FX_CAPABILITY, "UsdCny");
        // P4 M3 钩子: DATA_GATEWAY_GRPC=1 → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("ForeignExchange") {
            Ok(Some(bridge)) => {
                let result = bridge.foreign_exchange_async().await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Sina);
                return audit_gateway_result(FX_CAPABILITY, audit_provider, &request_hash, result);
            }
            Ok(None) => {}
            Err(error) => {
                return audit_gateway_result(FX_CAPABILITY, ProviderId::Sina, &request_hash, Err(error));
            }
        }
        let worker_request_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = fetch_and_admit_fx(request);
            audit_gateway_result(
                FX_CAPABILITY,
                ProviderId::Sina,
                &worker_request_hash,
                result,
            )
        })
        .await;

        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    FX_CAPABILITY,
                    ProviderId::Sina,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }
}

fn fetch_and_admit_indices(
    request: GlobalIndexRequest,
) -> Result<GatewayBatch<GlobalIndexFact>, GatewayError> {
    if !SinaClient::global_market_capabilities().indices {
        return Err(GatewayError::classified(
            INDEX_CAPABILITY,
            Some(ProviderId::Sina),
            "unsupported",
            "provider_unsupported",
            false,
            "Sina global-index capability is not admitted",
        ));
    }
    let client = SinaClient::new().map_err(|error| sina_gateway_error(INDEX_CAPABILITY, error))?;
    let batch = client
        .global_indices(&request)
        .map_err(|error| sina_gateway_error(INDEX_CAPABILITY, error))?;
    admit_indices(batch, &request, Utc::now())
}

fn fetch_and_admit_fx(
    request: FxRequest,
) -> Result<GatewayBatch<ForeignExchangeFact>, GatewayError> {
    if !SinaClient::global_market_capabilities().foreign_exchange {
        return Err(GatewayError::classified(
            FX_CAPABILITY,
            Some(ProviderId::Sina),
            "unsupported",
            "provider_unsupported",
            false,
            "Sina foreign-exchange capability is not admitted",
        ));
    }
    let client = SinaClient::new().map_err(|error| sina_gateway_error(FX_CAPABILITY, error))?;
    let batch = client
        .foreign_exchange(&request)
        .map_err(|error| sina_gateway_error(FX_CAPABILITY, error))?;
    admit_fx(batch, &request, Utc::now())
}

fn admit_indices(
    batch: DataBatch<GlobalIndexQuote>,
    request: &GlobalIndexRequest,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<GlobalIndexFact>, GatewayError> {
    let evidence = validate_batch(INDEX_CAPABILITY, &batch, now)?;
    if batch.records().len() != request.indices().len() {
        return Err(GatewayError::invalid_evidence(
            INDEX_CAPABILITY,
            Some(ProviderId::Sina),
            format!(
                "US index batch cardinality differs from request: expected={} actual={}",
                request.indices().len(),
                batch.records().len()
            ),
        ));
    }

    let expected: HashSet<_> = request.indices().iter().copied().collect();
    let mut seen = HashSet::with_capacity(expected.len());
    let mut records = Vec::with_capacity(expected.len());
    for record in batch.into_records() {
        if !expected.contains(&record.index) || !seen.insert(record.index) {
            return Err(GatewayError::invalid_evidence(
                INDEX_CAPABILITY,
                Some(ProviderId::Sina),
                format!(
                    "unexpected or duplicate US index identity {:?}",
                    record.index
                ),
            ));
        }
        validate_record_evidence(INDEX_CAPABILITY, &evidence, &record.evidence)?;
        if record.change_percent.unit() != RatioUnit::Percent {
            return Err(GatewayError::invalid_evidence(
                INDEX_CAPABILITY,
                Some(ProviderId::Sina),
                format!("US index {:?} change ratio is not percent", record.index),
            ));
        }
        let source_at = parse_source_at(
            INDEX_CAPABILITY,
            record.evidence.source_at(),
            now,
            "global index",
        )?;
        let observed_at = parse_observed_at(INDEX_CAPABILITY, record.evidence.observed_at(), now)?;
        records.push(GlobalIndexFact {
            code: record.index,
            name: record.name.as_str().to_string(),
            value: record.value.get(),
            change: record.change.get(),
            change_percent: record.change_percent.get(),
            source_at,
            observed_at,
            provider: ProviderId::Sina,
            batch_id: record.evidence.batch_id().to_string(),
        });
    }
    records.sort_by_key(|record| index_order(record.code));
    Ok(GatewayBatch::Available { records, evidence })
}

fn admit_fx(
    batch: DataBatch<FxQuote>,
    request: &FxRequest,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<ForeignExchangeFact>, GatewayError> {
    let evidence = validate_batch(FX_CAPABILITY, &batch, now)?;
    if batch.records().len() != request.pairs().len() {
        return Err(GatewayError::invalid_evidence(
            FX_CAPABILITY,
            Some(ProviderId::Sina),
            format!(
                "FX batch cardinality differs from request: expected={} actual={}",
                request.pairs().len(),
                batch.records().len()
            ),
        ));
    }

    let expected: HashSet<_> = request.pairs().iter().copied().collect();
    let mut seen = HashSet::with_capacity(expected.len());
    let mut records = Vec::with_capacity(expected.len());
    for record in batch.into_records() {
        if !expected.contains(&record.pair) || !seen.insert(record.pair) {
            return Err(GatewayError::invalid_evidence(
                FX_CAPABILITY,
                Some(ProviderId::Sina),
                format!("unexpected or duplicate FX identity {:?}", record.pair),
            ));
        }
        validate_record_evidence(FX_CAPABILITY, &evidence, &record.evidence)?;
        if record
            .change_percent
            .is_some_and(|ratio| ratio.unit() != RatioUnit::Percent)
        {
            return Err(GatewayError::invalid_evidence(
                FX_CAPABILITY,
                Some(ProviderId::Sina),
                format!("FX pair {:?} change ratio is not percent", record.pair),
            ));
        }
        let source_at =
            parse_source_at(FX_CAPABILITY, record.evidence.source_at(), now, "FX quote")?;
        let observed_at = parse_observed_at(FX_CAPABILITY, record.evidence.observed_at(), now)?;
        records.push(ForeignExchangeFact {
            pair: record.pair,
            name: record.name.as_str().to_string(),
            rate: record.rate.get(),
            change: record.change.map(|value| value.get()),
            change_percent: record.change_percent.map(|value| value.get()),
            source_at,
            observed_at,
            provider: ProviderId::Sina,
            batch_id: record.evidence.batch_id().to_string(),
        });
    }
    Ok(GatewayBatch::Available { records, evidence })
}

fn validate_batch<T>(
    capability: &'static str,
    batch: &DataBatch<T>,
    now: DateTime<Utc>,
) -> Result<BatchEvidence, GatewayError> {
    if batch.provenance().source() != SOURCE || !batch.quality().is_complete() {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(ProviderId::Sina),
            "global-market batch is not a complete Sina Web batch",
        ));
    }
    let evidence = BatchEvidence::from_provenance(ProviderId::Sina, batch.provenance())?;
    let source_at = evidence.source_at.as_deref().ok_or_else(|| {
        GatewayError::invalid_evidence(
            capability,
            Some(ProviderId::Sina),
            "global-market batch source_at is missing",
        )
    })?;
    parse_source_at(capability, Some(source_at), now, "batch")?;
    parse_observed_at(capability, &evidence.observed_at, now)?;
    Ok(evidence)
}

fn validate_record_evidence(
    capability: &'static str,
    batch: &BatchEvidence,
    record: &crate::magic_compat::SourceEvidence,
) -> Result<(), GatewayError> {
    if record.provider() != ProviderId::Sina
        || record.batch_id() != batch.batch_id
        || record.observed_at() != batch.observed_at
        || record.source_at().is_none()
    {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(ProviderId::Sina),
            "record provider/batch/observation/source evidence differs from batch",
        ));
    }
    Ok(())
}

fn parse_source_at(
    capability: &'static str,
    value: Option<&str>,
    now: DateTime<Utc>,
    field: &str,
) -> Result<DateTime<Utc>, GatewayError> {
    let value = value.ok_or_else(|| {
        GatewayError::invalid_evidence(
            capability,
            Some(ProviderId::Sina),
            format!("{field} source_at is missing"),
        )
    })?;
    let timestamp = DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| {
            GatewayError::invalid_evidence(
                capability,
                Some(ProviderId::Sina),
                format!("invalid {field} source_at {value:?}: {error}"),
            )
        })?;
    validate_realtime_age(capability, timestamp, now, field)?;
    Ok(timestamp)
}

fn parse_observed_at(
    capability: &'static str,
    value: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, GatewayError> {
    let timestamp = if let Ok(seconds) = value.parse::<f64>() {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(ProviderId::Sina),
                format!("invalid observation epoch {value:?}"),
            ));
        }
        let millis = (seconds * 1_000.0).round() as i64;
        DateTime::<Utc>::from_timestamp_millis(millis).ok_or_else(|| {
            GatewayError::invalid_evidence(
                capability,
                Some(ProviderId::Sina),
                format!("observation epoch is out of range: {value:?}"),
            )
        })?
    } else {
        DateTime::parse_from_rfc3339(value)
            .map(|parsed| parsed.with_timezone(&Utc))
            .map_err(|error| {
                GatewayError::invalid_evidence(
                    capability,
                    Some(ProviderId::Sina),
                    format!("invalid observation timestamp {value:?}: {error}"),
                )
            })?
    };
    validate_realtime_age(capability, timestamp, now, "observation")?;
    Ok(timestamp)
}

fn validate_realtime_age(
    capability: &'static str,
    timestamp: DateTime<Utc>,
    now: DateTime<Utc>,
    field: &str,
) -> Result<(), GatewayError> {
    let age_ms = now.signed_duration_since(timestamp).num_milliseconds();
    if !(0..=REALTIME_MAX_AGE_MILLIS).contains(&age_ms) {
        return Err(GatewayError::classified(
            capability,
            Some(ProviderId::Sina),
            "stale",
            "global_market_stale",
            true,
            format!("{field} failed five-second freshness gate age_ms={age_ms}"),
        ));
    }
    Ok(())
}

fn index_order(code: GlobalIndexCode) -> u8 {
    match code {
        GlobalIndexCode::DowJones => 0,
        GlobalIndexCode::NasdaqComposite => 1,
        GlobalIndexCode::Sp500 => 2,
        GlobalIndexCode::Nikkei225 => 3,
        GlobalIndexCode::HangSeng => 4,
        GlobalIndexCode::Ftse100 => 5,
    }
}

fn sina_gateway_error(capability: &'static str, error: SinaError) -> GatewayError {
    let message = error.to_string();
    match error {
        SinaError::InvalidRequest(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Sina),
            "invalid_request",
            "provider_invalid_request",
            false,
            message,
        ),
        SinaError::Transport(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Sina),
            "unavailable",
            "provider_transport",
            true,
            message,
        ),
        SinaError::Unsupported(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Sina),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        SinaError::Decode(_) | SinaError::Protocol(_) | SinaError::Core(_) => {
            GatewayError::classified(
                capability,
                Some(ProviderId::Sina),
                "partial",
                "provider_batch_rejected",
                false,
                message,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magic_market_core::{FiniteNumber, NonEmptyText, Price, Provenance, Ratio, SourceEvidence};

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("TEST_CODE timestamp")
            .with_timezone(&Utc)
    }

    fn fx_batch(source_at: &str) -> DataBatch<FxQuote> {
        let observed_at = "2026-07-27T01:30:02Z";
        let batch_id = "TEST_CODE_fx_batch";
        let record = FxQuote {
            pair: FxPair::UsdCny,
            name: NonEmptyText::new("美元人民币").expect("name"),
            rate: Price::new(7.2).expect("rate"),
            change: Some(FiniteNumber::new(0.01).expect("change")),
            change_percent: Some(Ratio::new(0.14, RatioUnit::Percent).expect("change percent")),
            evidence: SourceEvidence::new(ProviderId::Sina, observed_at, batch_id)
                .expect("evidence")
                .with_source_at(source_at)
                .expect("source_at"),
        };
        let provenance = Provenance::new(SOURCE, observed_at)
            .expect("provenance")
            .with_source_at(source_at)
            .expect("source_at")
            .with_batch_id(batch_id)
            .expect("batch");
        DataBatch::strict(vec![record], provenance)
    }

    fn source_evidence(
        provider: ProviderId,
        observed_at: &str,
        source_at: Option<&str>,
        batch_id: &str,
    ) -> SourceEvidence {
        let evidence = SourceEvidence::new(provider, observed_at, batch_id)
            .expect("TEST_CODE source evidence");
        match source_at {
            Some(source_at) => evidence
                .with_source_at(source_at)
                .expect("TEST_CODE provider source time"),
            None => evidence,
        }
    }

    fn index_quote(
        index: GlobalIndexCode,
        observed_at: &str,
        source_at: Option<&str>,
        batch_id: &str,
        provider: ProviderId,
        ratio_unit: RatioUnit,
    ) -> GlobalIndexQuote {
        GlobalIndexQuote {
            index,
            name: NonEmptyText::new(format!("TEST_CODE_{index:?}")).expect("TEST_CODE index name"),
            value: Price::new(45_000.0).expect("TEST_CODE index value"),
            change: FiniteNumber::new(10.0).expect("TEST_CODE index change"),
            change_percent: Ratio::new(0.02, ratio_unit).expect("TEST_CODE index change percent"),
            evidence: source_evidence(provider, observed_at, source_at, batch_id),
        }
    }

    fn index_batch(
        records: Vec<GlobalIndexQuote>,
        source: &str,
        observed_at: &str,
        source_at: Option<&str>,
        batch_id: &str,
    ) -> DataBatch<GlobalIndexQuote> {
        let provenance = Provenance::new(source, observed_at)
            .expect("TEST_CODE provenance")
            .with_batch_id(batch_id)
            .expect("TEST_CODE batch id");
        let provenance = match source_at {
            Some(source_at) => provenance
                .with_source_at(source_at)
                .expect("TEST_CODE batch source time"),
            None => provenance,
        };
        DataBatch::strict(records, provenance)
    }

    fn fx_quote(
        pair: FxPair,
        observed_at: &str,
        source_at: Option<&str>,
        batch_id: &str,
        provider: ProviderId,
        ratio_unit: Option<RatioUnit>,
    ) -> FxQuote {
        FxQuote {
            pair,
            name: NonEmptyText::new(format!("TEST_CODE_{pair:?}")).expect("TEST_CODE FX name"),
            rate: Price::new(7.2).expect("TEST_CODE FX rate"),
            change: Some(FiniteNumber::new(0.01).expect("TEST_CODE FX change")),
            change_percent: ratio_unit
                .map(|unit| Ratio::new(0.14, unit).expect("TEST_CODE FX change percent")),
            evidence: source_evidence(provider, observed_at, source_at, batch_id),
        }
    }

    #[test]
    fn admits_fresh_usd_cny_with_complete_evidence() {
        let request = FxRequest::new(vec![FxPair::UsdCny]).expect("request");
        let admitted = admit_fx(
            fx_batch("2026-07-27T09:30:00+08:00"),
            &request,
            at("2026-07-27T01:30:03Z"),
        )
        .expect("fresh quote");

        assert_eq!(admitted.records().len(), 1);
        assert_eq!(admitted.records()[0].pair, FxPair::UsdCny);
        assert_eq!(admitted.records()[0].rate, 7.2);
        assert_eq!(admitted.evidence().source, SOURCE);
    }

    #[test]
    fn rejects_stale_usd_cny() {
        let request = FxRequest::new(vec![FxPair::UsdCny]).expect("request");
        let error = admit_fx(
            fx_batch("2026-07-27T09:29:50+08:00"),
            &request,
            at("2026-07-27T01:30:03Z"),
        )
        .expect_err("stale quote must fail");

        assert_eq!(error.reason_code(), "global_market_stale");
    }

    #[test]
    fn rejects_index_batch_without_provider_source_time() {
        let observed_at = "2026-07-27T01:30:02Z";
        let batch_id = "TEST_CODE_index_batch";
        let quote = GlobalIndexQuote {
            index: GlobalIndexCode::DowJones,
            name: NonEmptyText::new("道琼斯").expect("name"),
            value: Price::new(45_000.0).expect("value"),
            change: FiniteNumber::new(10.0).expect("change"),
            change_percent: Ratio::new(0.02, RatioUnit::Percent).expect("change percent"),
            evidence: SourceEvidence::new(ProviderId::Sina, observed_at, batch_id)
                .expect("evidence"),
        };
        let provenance = Provenance::new(SOURCE, observed_at)
            .expect("provenance")
            .with_batch_id(batch_id)
            .expect("batch");
        let request = GlobalIndexRequest::new(vec![GlobalIndexCode::DowJones]).expect("request");
        let error = admit_indices(
            DataBatch::strict(vec![quote], provenance),
            &request,
            at("2026-07-27T01:30:03Z"),
        )
        .expect_err("missing provider source time must fail");

        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn admits_and_orders_complete_us_index_batch() {
        let observed_at = "2026-07-27T01:30:02Z";
        let source_at = "2026-07-27T09:30:00+08:00";
        let batch_id = "TEST_CODE_index_order_batch";
        let request = GlobalIndexRequest::new(vec![
            GlobalIndexCode::DowJones,
            GlobalIndexCode::NasdaqComposite,
            GlobalIndexCode::Sp500,
        ])
        .expect("TEST_CODE US index request");
        let records = [
            GlobalIndexCode::Sp500,
            GlobalIndexCode::NasdaqComposite,
            GlobalIndexCode::DowJones,
        ]
        .into_iter()
        .map(|index| {
            index_quote(
                index,
                observed_at,
                Some(source_at),
                batch_id,
                ProviderId::Sina,
                RatioUnit::Percent,
            )
        })
        .collect();

        let admitted = admit_indices(
            index_batch(records, SOURCE, observed_at, Some(source_at), batch_id),
            &request,
            at("2026-07-27T01:30:03Z"),
        )
        .expect("TEST_CODE complete US index evidence");

        assert_eq!(
            admitted
                .records()
                .iter()
                .map(|record| record.code)
                .collect::<Vec<_>>(),
            vec![
                GlobalIndexCode::DowJones,
                GlobalIndexCode::NasdaqComposite,
                GlobalIndexCode::Sp500,
            ]
        );
        for record in admitted.records() {
            assert_eq!(record.provider, ProviderId::Sina);
            assert_eq!(record.batch_id, batch_id);
            assert_eq!(record.value, 45_000.0);
            assert_eq!(record.change, 10.0);
            assert_eq!(record.change_percent, 0.02);
            assert_eq!(record.source_at, at("2026-07-27T01:30:00Z"));
            assert_eq!(record.observed_at, at(observed_at));
        }
    }

    #[test]
    fn rejects_incomplete_or_wrong_source_batches_before_projection() {
        let observed_at = "2026-07-27T01:30:02Z";
        let source_at = "2026-07-27T09:30:00+08:00";
        let batch_id = "TEST_CODE_batch_contract";
        let record = index_quote(
            GlobalIndexCode::DowJones,
            observed_at,
            Some(source_at),
            batch_id,
            ProviderId::Sina,
            RatioUnit::Percent,
        );
        let wrong_source = index_batch(
            vec![record.clone()],
            "TEST_CODE_wrong_source",
            observed_at,
            Some(source_at),
            batch_id,
        );
        let wrong_source_error =
            validate_batch(INDEX_CAPABILITY, &wrong_source, at("2026-07-27T01:30:03Z"))
                .expect_err("TEST_CODE wrong source must fail");
        assert_eq!(wrong_source_error.reason_code(), "invalid_evidence");
        assert!(!wrong_source_error.retryable());

        let provenance = Provenance::new(SOURCE, observed_at)
            .expect("TEST_CODE incomplete provenance")
            .with_source_at(source_at)
            .expect("TEST_CODE incomplete source time")
            .with_batch_id(batch_id)
            .expect("TEST_CODE incomplete batch id");
        let incomplete = DataBatch::best_effort(
            vec![record],
            provenance,
            vec!["TEST_CODE missing provider row".into()],
        )
        .expect("TEST_CODE explicit incomplete batch");
        let incomplete_error =
            validate_batch(INDEX_CAPABILITY, &incomplete, at("2026-07-27T01:30:03Z"))
                .expect_err("TEST_CODE incomplete batch must fail");
        assert_eq!(incomplete_error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn rejects_cardinality_identity_and_lineage_conflicts() {
        let observed_at = "2026-07-27T01:30:02Z";
        let source_at = "2026-07-27T09:30:00+08:00";
        let batch_id = "TEST_CODE_identity_batch";
        let now = at("2026-07-27T01:30:03Z");
        let single_request =
            GlobalIndexRequest::new(vec![GlobalIndexCode::DowJones]).expect("TEST_CODE request");
        let cardinality_error = admit_indices(
            index_batch(vec![], SOURCE, observed_at, Some(source_at), batch_id),
            &single_request,
            now,
        )
        .expect_err("TEST_CODE cardinality mismatch must fail");
        assert_eq!(cardinality_error.reason_code(), "invalid_evidence");

        let pair_request = GlobalIndexRequest::new(vec![
            GlobalIndexCode::DowJones,
            GlobalIndexCode::NasdaqComposite,
        ])
        .expect("TEST_CODE pair request");
        for records in [
            vec![
                index_quote(
                    GlobalIndexCode::DowJones,
                    observed_at,
                    Some(source_at),
                    batch_id,
                    ProviderId::Sina,
                    RatioUnit::Percent,
                ),
                index_quote(
                    GlobalIndexCode::Sp500,
                    observed_at,
                    Some(source_at),
                    batch_id,
                    ProviderId::Sina,
                    RatioUnit::Percent,
                ),
            ],
            vec![
                index_quote(
                    GlobalIndexCode::DowJones,
                    observed_at,
                    Some(source_at),
                    batch_id,
                    ProviderId::Sina,
                    RatioUnit::Percent,
                ),
                index_quote(
                    GlobalIndexCode::DowJones,
                    observed_at,
                    Some(source_at),
                    batch_id,
                    ProviderId::Sina,
                    RatioUnit::Percent,
                ),
            ],
        ] {
            let error = admit_indices(
                index_batch(records, SOURCE, observed_at, Some(source_at), batch_id),
                &pair_request,
                now,
            )
            .expect_err("TEST_CODE unexpected or duplicate identity must fail");
            assert_eq!(error.reason_code(), "invalid_evidence");
        }

        let wrong_lineage = index_quote(
            GlobalIndexCode::DowJones,
            observed_at,
            Some(source_at),
            batch_id,
            ProviderId::Tencent,
            RatioUnit::Percent,
        );
        let lineage_error = admit_indices(
            index_batch(
                vec![wrong_lineage],
                SOURCE,
                observed_at,
                Some(source_at),
                batch_id,
            ),
            &single_request,
            now,
        )
        .expect_err("TEST_CODE record provider mismatch must fail");
        assert_eq!(lineage_error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn rejects_non_percent_index_and_fx_ratios() {
        let observed_at = "2026-07-27T01:30:02Z";
        let source_at = "2026-07-27T09:30:00+08:00";
        let now = at("2026-07-27T01:30:03Z");
        let index_batch_id = "TEST_CODE_decimal_index_ratio";
        let index_request =
            GlobalIndexRequest::new(vec![GlobalIndexCode::DowJones]).expect("TEST_CODE request");
        let index_error = admit_indices(
            index_batch(
                vec![index_quote(
                    GlobalIndexCode::DowJones,
                    observed_at,
                    Some(source_at),
                    index_batch_id,
                    ProviderId::Sina,
                    RatioUnit::Decimal,
                )],
                SOURCE,
                observed_at,
                Some(source_at),
                index_batch_id,
            ),
            &index_request,
            now,
        )
        .expect_err("TEST_CODE decimal index ratio must fail");
        assert_eq!(index_error.reason_code(), "invalid_evidence");

        let fx_batch_id = "TEST_CODE_decimal_fx_ratio";
        let provenance = Provenance::new(SOURCE, observed_at)
            .expect("TEST_CODE FX provenance")
            .with_source_at(source_at)
            .expect("TEST_CODE FX source time")
            .with_batch_id(fx_batch_id)
            .expect("TEST_CODE FX batch id");
        let fx_request = FxRequest::new(vec![FxPair::UsdCny]).expect("TEST_CODE FX request");
        let fx_error = admit_fx(
            DataBatch::strict(
                vec![fx_quote(
                    FxPair::UsdCny,
                    observed_at,
                    Some(source_at),
                    fx_batch_id,
                    ProviderId::Sina,
                    Some(RatioUnit::Decimal),
                )],
                provenance,
            ),
            &fx_request,
            now,
        )
        .expect_err("TEST_CODE decimal FX ratio must fail");
        assert_eq!(fx_error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn parses_both_observation_encodings_and_enforces_exact_freshness_bounds() {
        let now = at("2026-07-27T01:30:05Z");
        assert_eq!(
            parse_observed_at(INDEX_CAPABILITY, "1785115804.250", now)
                .expect("TEST_CODE finite epoch observation"),
            at("2026-07-27T01:30:04.250Z")
        );
        assert_eq!(
            parse_observed_at(INDEX_CAPABILITY, "2026-07-27T09:30:00+08:00", now)
                .expect("TEST_CODE RFC3339 observation"),
            at("2026-07-27T01:30:00Z")
        );
        assert!(validate_realtime_age(
            INDEX_CAPABILITY,
            at("2026-07-27T01:30:00Z"),
            now,
            "TEST_CODE boundary",
        )
        .is_ok());
        for timestamp in [
            at("2026-07-27T01:29:59.999Z"),
            at("2026-07-27T01:30:05.001Z"),
        ] {
            let error = validate_realtime_age(
                INDEX_CAPABILITY,
                timestamp,
                now,
                "TEST_CODE outside boundary",
            )
            .expect_err("TEST_CODE stale or future evidence must fail");
            assert_eq!(error.reason_code(), "global_market_stale");
            assert!(error.retryable());
        }
        for invalid in ["-1", "NaN", "TEST_CODE_not_a_timestamp"] {
            let error = parse_observed_at(INDEX_CAPABILITY, invalid, now)
                .expect_err("TEST_CODE invalid observation must fail");
            assert_eq!(error.reason_code(), "invalid_evidence");
        }
        assert_eq!(
            parse_source_at(INDEX_CAPABILITY, None, now, "TEST_CODE source")
                .expect_err("TEST_CODE missing source time must fail")
                .reason_code(),
            "invalid_evidence"
        );
        assert_eq!(
            parse_source_at(
                INDEX_CAPABILITY,
                Some("TEST_CODE_not_a_timestamp"),
                now,
                "TEST_CODE source",
            )
            .expect_err("TEST_CODE invalid source time must fail")
            .reason_code(),
            "invalid_evidence"
        );
    }

    #[test]
    fn maps_sina_failures_without_losing_retry_semantics() {
        let cases = [
            (
                SinaError::InvalidRequest("TEST_CODE invalid request".into()),
                "provider_invalid_request",
                "invalid_request",
                false,
            ),
            (
                SinaError::Transport("TEST_CODE unavailable".into()),
                "provider_transport",
                "unavailable",
                true,
            ),
            (
                SinaError::Unsupported("TEST_CODE unsupported".into()),
                "provider_unsupported",
                "unsupported",
                false,
            ),
            (
                SinaError::Decode("TEST_CODE malformed".into()),
                "provider_batch_rejected",
                "partial",
                false,
            ),
            (
                SinaError::Protocol("TEST_CODE protocol".into()),
                "provider_batch_rejected",
                "partial",
                false,
            ),
        ];
        for (source_error, reason_code, audit_outcome, retryable) in cases {
            let error = sina_gateway_error(INDEX_CAPABILITY, source_error);
            assert_eq!(error.reason_code(), reason_code);
            assert_eq!(error.audit_outcome(), audit_outcome);
            assert_eq!(error.retryable(), retryable);
        }
    }

    #[test]
    fn all_declared_index_identities_have_stable_order() {
        assert_eq!(index_order(GlobalIndexCode::DowJones), 0);
        assert_eq!(index_order(GlobalIndexCode::NasdaqComposite), 1);
        assert_eq!(index_order(GlobalIndexCode::Sp500), 2);
        assert_eq!(index_order(GlobalIndexCode::Nikkei225), 3);
        assert_eq!(index_order(GlobalIndexCode::HangSeng), 4);
        assert_eq!(index_order(GlobalIndexCode::Ftse100), 5);
    }
}
