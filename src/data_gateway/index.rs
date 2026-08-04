//! BR-164 evidence-preserving A-share index quote Gateway.
//!
//! Tencent's Magic provider is the only provider at the pinned upstream
//! revision that proves the normalized realtime quote contract for the six
//! domestic indices consumed by `MarketAnalyzer`.  The consumer therefore
//! receives one complete, ordered Tencent batch or an explicit failure; it no
//! longer owns a Tencent URL, HTTP client, retry loop, or wire parser.

use chrono::{DateTime, Utc};
use magic_market_core::{
    AssetClass, DataBatch, DataStatus, Exchange, InstrumentId, ProviderId, Quote, RatioUnit,
    RealtimeQuotes,
};
use magic_tencent_rs::{TencentClient, TencentError};
use std::collections::HashSet;

use super::review::{
    acquisition_request_hash, audit_gateway_result, BatchEvidence, GatewayBatch, GatewayError,
};

const CAPABILITY: &str = "RealtimeIndexQuotes";
const REALTIME_MAX_AGE_MILLISECONDS: i64 = 5_000;

/// One admitted domestic-index quote and its record-level evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeIndexQuote {
    pub code: String,
    pub name: String,
    pub current: f64,
    pub change: f64,
    pub change_percent: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub previous_close: f64,
    pub volume: f64,
    pub amount: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Single-owner boundary for domestic-index quotes.
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexDataGateway;

impl IndexDataGateway {
    pub const fn new() -> Self {
        Self
    }

    pub fn realtime_quotes(
        &self,
        storage_codes: &[String],
    ) -> Result<GatewayBatch<RealtimeIndexQuote>, GatewayError> {
        let request_hash = acquisition_request_hash(CAPABILITY, &storage_codes.join(","));
        let result = build_index_instruments(storage_codes).and_then(|instruments| {
            let client = TencentClient::new().map_err(tencent_gateway_error)?;
            let batch = client
                .realtime_quotes(&instruments)
                .map_err(tencent_gateway_error)?;
            admit_index_batch(storage_codes, batch, Utc::now())
        });
        audit_gateway_result(CAPABILITY, ProviderId::Tencent, &request_hash, result)
    }
}

fn build_index_instruments(storage_codes: &[String]) -> Result<Vec<InstrumentId>, GatewayError> {
    if storage_codes.is_empty() {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            "index quote request must contain at least one code",
        ));
    }

    let mut seen = HashSet::with_capacity(storage_codes.len());
    storage_codes
        .iter()
        .map(|storage_code| {
            if !seen.insert(storage_code.as_str()) {
                return Err(GatewayError::invalid_request(
                    CAPABILITY,
                    format!("duplicate index code {storage_code:?}"),
                ));
            }
            #[cfg(test)]
            let storage_code = storage_code
                .strip_prefix("TEST_CODE_")
                .unwrap_or(storage_code.as_str());
            #[cfg(not(test))]
            let storage_code = storage_code.as_str();

            let (exchange, code) = if let Some(code) = storage_code.strip_prefix("sh") {
                (Exchange::Shanghai, code)
            } else if let Some(code) = storage_code.strip_prefix("sz") {
                (Exchange::Shenzhen, code)
            } else {
                return Err(GatewayError::invalid_request(
                    CAPABILITY,
                    format!("index code must use sh/sz storage prefix: {storage_code:?}"),
                ));
            };
            if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(GatewayError::invalid_request(
                    CAPABILITY,
                    format!("invalid six-digit index code {storage_code:?}"),
                ));
            }
            InstrumentId::new(exchange, code, AssetClass::Index).map_err(|error| {
                GatewayError::invalid_request(
                    CAPABILITY,
                    format!("invalid index instrument {storage_code:?}: {error}"),
                )
            })
        })
        .collect()
}

fn admit_index_batch(
    storage_codes: &[String],
    batch: DataBatch<Quote>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<RealtimeIndexQuote>, GatewayError> {
    let provider = ProviderId::Tencent;
    let evidence = BatchEvidence::from_provenance(provider, batch.provenance())?;
    if !batch.quality().is_complete() {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(provider),
            "partial",
            "index_quote_batch_incomplete",
            false,
            format!(
                "Tencent index quote batch is incomplete: {}",
                batch.quality().issues().join("; ")
            ),
        ));
    }
    if batch.records().is_empty() || batch.records().len() != storage_codes.len() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(provider),
            format!(
                "index quote cardinality mismatch requested={} actual={}",
                storage_codes.len(),
                batch.records().len()
            ),
        ));
    }

    let observed_at = parse_timestamp(batch.provenance().fetched_at(), "observed_at")?;
    let mut records = Vec::with_capacity(batch.records().len());
    for (storage_code, quote) in storage_codes.iter().zip(batch.records()) {
        let canonical_storage_code = storage_code
            .strip_prefix("TEST_CODE_")
            .unwrap_or(storage_code.as_str());
        let expected_exchange = if canonical_storage_code.starts_with("sh") {
            Exchange::Shanghai
        } else {
            Exchange::Shenzhen
        };
        let expected_code = &canonical_storage_code[2..];
        if quote.instrument().exchange() != expected_exchange
            || quote.instrument().code() != expected_code
            || quote.instrument().asset_class() != AssetClass::Index
            || quote.provider() != provider
            || quote.batch_id() != evidence.batch_id
            || quote.observed_at() != evidence.observed_at
            || quote.status() != DataStatus::Available
        {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                format!("index quote evidence/identity mismatch for {storage_code}"),
            ));
        }

        let name = required_text(quote.name(), storage_code, "name")?.to_owned();
        let previous_close =
            required_value(quote.previous_close(), storage_code, "previous_close")?;
        let open = required_value(quote.open(), storage_code, "open")?;
        let high = required_value(quote.high(), storage_code, "high")?;
        let low = required_value(quote.low(), storage_code, "low")?;
        let current = quote.price().get();
        let amount = quote
            .amount()
            .ok_or_else(|| missing_field(storage_code, "amount"))?
            .get();
        let volume = quote.volume().get();
        let change_percent = quote
            .change_percent()
            .ok_or_else(|| missing_field(storage_code, "change_percent"))?;
        if change_percent.unit() != RatioUnit::Percent {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                format!("index {storage_code} change_percent unit is not Percent"),
            ));
        }
        let change_percent = change_percent.get();
        if change_percent.abs() > 20.0 {
            return Err(GatewayError::classified(
                CAPABILITY,
                Some(provider),
                "partial",
                "manual_confirmation_required",
                false,
                format!("index {storage_code} daily change {change_percent:.4}% exceeds ±20%"),
            ));
        }
        if low > open.min(current) || open.max(current) > high || amount < 0.0 || volume < 0.0 {
            return Err(GatewayError::invalid_evidence(
                CAPABILITY,
                Some(provider),
                format!("index {storage_code} OHLC/volume/amount relationship is invalid"),
            ));
        }

        let source_at = parse_timestamp(
            quote
                .source_at()
                .ok_or_else(|| missing_field(storage_code, "source_at"))?,
            "source_at",
        )?;
        let age_ms = now.signed_duration_since(source_at).num_milliseconds();
        if !(0..=REALTIME_MAX_AGE_MILLISECONDS).contains(&age_ms) {
            return Err(GatewayError::classified(
                CAPABILITY,
                Some(provider),
                "stale",
                "index_quote_stale",
                true,
                format!("index {storage_code} failed five-second freshness gate age_ms={age_ms}"),
            ));
        }

        records.push(RealtimeIndexQuote {
            code: storage_code.clone(),
            name,
            current,
            change: current - previous_close,
            change_percent,
            open,
            high,
            low,
            previous_close,
            volume,
            amount,
            source_at,
            observed_at,
            provider,
            batch_id: quote.batch_id().to_owned(),
        });
    }

    Ok(GatewayBatch::Available { records, evidence })
}

fn required_text<'a>(
    value: Option<&'a str>,
    code: &str,
    field: &str,
) -> Result<&'a str, GatewayError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| missing_field(code, field))
}

fn required_value(
    value: Option<magic_market_core::Price>,
    code: &str,
    field: &str,
) -> Result<f64, GatewayError> {
    value
        .map(|value| value.get())
        .ok_or_else(|| missing_field(code, field))
}

fn missing_field(code: &str, field: &str) -> GatewayError {
    GatewayError::invalid_evidence(
        CAPABILITY,
        Some(ProviderId::Tencent),
        format!("index {code} required field {field} is unavailable"),
    )
}

fn parse_timestamp(value: &str, field: &str) -> Result<DateTime<Utc>, GatewayError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(ProviderId::Tencent),
                format!("invalid {field} timestamp {value:?}: {error}"),
            )
        })
}

fn tencent_gateway_error(error: TencentError) -> GatewayError {
    match error {
        TencentError::InvalidRequest(message) => GatewayError::invalid_request(CAPABILITY, message),
        TencentError::Transport(message) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tencent),
            "unavailable",
            "tencent_transport_failure",
            true,
            message,
        ),
        TencentError::Decode(message) | TencentError::Protocol(message) => {
            GatewayError::classified(
                CAPABILITY,
                Some(ProviderId::Tencent),
                "partial",
                "tencent_protocol_failure",
                false,
                message,
            )
        }
        TencentError::Unsupported(message) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tencent),
            "unsupported",
            "tencent_index_quotes_unsupported",
            false,
            message,
        ),
        TencentError::Core(error) => {
            GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::Tencent), error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magic_market_core::{Money, Price, Provenance, Quantity, Ratio};

    struct QuoteFixture {
        exchange: Exchange,
        code: &'static str,
        asset_class: AssetClass,
        name: Option<&'static str>,
        price: f64,
        previous_close: Option<f64>,
        open: Option<f64>,
        high: Option<f64>,
        low: Option<f64>,
        change_percent: Option<(f64, RatioUnit)>,
        volume: f64,
        amount: Option<f64>,
        status: DataStatus,
        source_at: Option<String>,
        observed_at: String,
        provider: ProviderId,
        batch_id: &'static str,
    }

    impl QuoteFixture {
        fn valid(now: DateTime<Utc>) -> Self {
            let timestamp = now.to_rfc3339();
            Self {
                exchange: Exchange::Shanghai,
                code: "000001",
                asset_class: AssetClass::Index,
                name: Some("TEST_CODE index"),
                price: 3_500.0,
                previous_close: Some(3_480.0),
                open: Some(3_490.0),
                high: Some(3_510.0),
                low: Some(3_470.0),
                change_percent: Some((0.57, RatioUnit::Percent)),
                volume: 123_456.0,
                amount: Some(10_000_000_000.0),
                status: DataStatus::Available,
                source_at: Some(timestamp.clone()),
                observed_at: timestamp,
                provider: ProviderId::Tencent,
                batch_id: "TEST_CODE_index_fixture",
            }
        }

        fn build(&self) -> Quote {
            Quote::from_parts(
                InstrumentId::new(self.exchange, self.code, self.asset_class).unwrap(),
                self.name.map(str::to_owned),
                Price::new(self.price).unwrap(),
                self.previous_close.map(Price::new).transpose().unwrap(),
                self.open.map(Price::new).transpose().unwrap(),
                self.high.map(Price::new).transpose().unwrap(),
                self.low.map(Price::new).transpose().unwrap(),
                self.change_percent
                    .map(|(value, unit)| Ratio::new(value, unit))
                    .transpose()
                    .unwrap(),
                Quantity::new(self.volume).unwrap(),
                self.amount.map(Money::new).transpose().unwrap(),
                self.status,
                self.source_at.clone(),
                self.observed_at.clone(),
                self.provider,
                self.batch_id,
            )
            .unwrap()
        }

        fn provenance(&self) -> Provenance {
            let mut provenance =
                Provenance::new("TEST_CODE_tencent", self.observed_at.clone()).unwrap();
            if let Some(source_at) = &self.source_at {
                provenance = provenance.with_source_at(source_at.clone()).unwrap();
            }
            provenance.with_batch_id(self.batch_id).unwrap()
        }
    }

    #[test]
    fn br164_index_request_rejects_empty_duplicate_and_bad_prefix() {
        assert!(build_index_instruments(&[]).is_err());
        assert!(build_index_instruments(&[
            "TEST_CODE_sh000001".to_owned(),
            "TEST_CODE_sh000001".to_owned(),
        ])
        .is_err());
        assert!(build_index_instruments(&["TEST_CODE_xx000001".to_owned()]).is_err());
        let instruments = build_index_instruments(&["TEST_CODE_sh000001".to_owned()]).unwrap();
        assert_eq!(instruments[0].asset_class(), AssetClass::Index);
        assert_eq!(instruments[0].exchange(), Exchange::Shanghai);
    }

    #[test]
    fn br164_complete_index_batch_preserves_order_and_evidence() {
        let now = Utc::now();
        let timestamp = now.to_rfc3339();
        let batch_id = "TEST_CODE_tencent_index_batch";
        let instrument =
            InstrumentId::new(Exchange::Shanghai, "000001", AssetClass::Index).unwrap();
        let quote = Quote::from_parts(
            instrument,
            Some("协议测试指数".to_owned()),
            Price::new(3_500.0).unwrap(),
            Some(Price::new(3_480.0).unwrap()),
            Some(Price::new(3_490.0).unwrap()),
            Some(Price::new(3_510.0).unwrap()),
            Some(Price::new(3_470.0).unwrap()),
            Some(Ratio::new(0.574_712_643_7, RatioUnit::Percent).unwrap()),
            Quantity::new(123_456.0).unwrap(),
            Some(Money::new(10_000_000_000.0).unwrap()),
            DataStatus::Available,
            Some(timestamp.clone()),
            timestamp.clone(),
            ProviderId::Tencent,
            batch_id,
        )
        .unwrap();
        let provenance = Provenance::new("TEST_CODE_tencent", timestamp.clone())
            .unwrap()
            .with_source_at(timestamp)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        let batch = DataBatch::strict(vec![quote], provenance);

        let admitted = admit_index_batch(&["TEST_CODE_sh000001".to_owned()], batch, now).unwrap();
        let record = &admitted.records()[0];
        assert_eq!(record.code, "TEST_CODE_sh000001");
        assert_eq!(record.provider, ProviderId::Tencent);
        assert_eq!(record.change, 20.0);
        assert_eq!(record.amount, 10_000_000_000.0);
    }

    #[test]
    fn br164_stale_or_partial_index_batch_is_rejected() {
        let now = Utc::now();
        let old = now - chrono::Duration::seconds(6);
        let timestamp = old.to_rfc3339();
        let batch_id = "TEST_CODE_tencent_index_stale";
        let instrument =
            InstrumentId::new(Exchange::Shanghai, "000001", AssetClass::Index).unwrap();
        let quote = Quote::from_parts(
            instrument,
            Some("协议测试指数".to_owned()),
            Price::new(3_500.0).unwrap(),
            Some(Price::new(3_480.0).unwrap()),
            Some(Price::new(3_490.0).unwrap()),
            Some(Price::new(3_510.0).unwrap()),
            Some(Price::new(3_470.0).unwrap()),
            Some(Ratio::new(0.57, RatioUnit::Percent).unwrap()),
            Quantity::new(123_456.0).unwrap(),
            Some(Money::new(10_000_000_000.0).unwrap()),
            DataStatus::Available,
            Some(timestamp.clone()),
            timestamp.clone(),
            ProviderId::Tencent,
            batch_id,
        )
        .unwrap();
        let provenance = Provenance::new("TEST_CODE_tencent", timestamp.clone())
            .unwrap()
            .with_source_at(timestamp)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        let batch = DataBatch::strict(vec![quote], provenance);
        let error = admit_index_batch(&["TEST_CODE_sh000001".to_owned()], batch, now).unwrap_err();
        assert_eq!(error.reason_code(), "index_quote_stale");
    }

    #[test]
    fn br164_request_accepts_both_exchanges_and_rejects_bad_codes() {
        let instruments = build_index_instruments(&[
            "TEST_CODE_sh000001".to_owned(),
            "TEST_CODE_sz399001".to_owned(),
        ])
        .unwrap();
        assert_eq!(instruments[0].exchange(), Exchange::Shanghai);
        assert_eq!(instruments[1].exchange(), Exchange::Shenzhen);
        for code in [
            "TEST_CODE_sh12345",
            "TEST_CODE_sz1234567",
            "TEST_CODE_sh12A456",
        ] {
            assert!(build_index_instruments(&[code.to_owned()]).is_err());
        }
    }

    #[test]
    fn br164_cardinality_completeness_identity_and_required_fields_are_strict() {
        let now = Utc::now();
        let fixture = QuoteFixture::valid(now);
        assert!(admit_index_batch(
            &["TEST_CODE_sh000001".to_owned()],
            DataBatch::strict(Vec::<Quote>::new(), fixture.provenance()),
            now,
        )
        .is_err());

        let partial = DataBatch::best_effort(
            vec![fixture.build()],
            fixture.provenance(),
            vec!["TEST_CODE incomplete quote".to_string()],
        )
        .unwrap();
        assert!(admit_index_batch(&["TEST_CODE_sh000001".to_owned()], partial, now).is_err());

        let mut wrong_identity = QuoteFixture::valid(now);
        wrong_identity.code = "399001";
        wrong_identity.exchange = Exchange::Shenzhen;
        assert!(admit_index_batch(
            &["TEST_CODE_sh000001".to_owned()],
            DataBatch::strict(vec![wrong_identity.build()], wrong_identity.provenance()),
            now,
        )
        .is_err());

        assert!(required_text(None, "TEST_CODE_sh000001", "name").is_err());
        assert!(required_text(Some(" "), "TEST_CODE_sh000001", "name").is_err());
        assert!(required_value(None, "TEST_CODE_sh000001", "open").is_err());
        assert_eq!(
            required_value(Some(Price::new(1.0).unwrap()), "TEST_CODE_sh000001", "open").unwrap(),
            1.0
        );
    }

    #[test]
    fn br164_ratio_ohlc_and_freshness_quality_gates_fail_closed() {
        let now = Utc::now();

        let mut wrong_unit = QuoteFixture::valid(now);
        wrong_unit.change_percent = Some((0.57, RatioUnit::Decimal));
        assert!(admit_index_batch(
            &["TEST_CODE_sh000001".to_owned()],
            DataBatch::strict(vec![wrong_unit.build()], wrong_unit.provenance()),
            now,
        )
        .is_err());

        let mut extreme = QuoteFixture::valid(now);
        extreme.change_percent = Some((20.01, RatioUnit::Percent));
        let error = admit_index_batch(
            &["TEST_CODE_sh000001".to_owned()],
            DataBatch::strict(vec![extreme.build()], extreme.provenance()),
            now,
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "manual_confirmation_required");

        let mut bad_ohlc = QuoteFixture::valid(now);
        bad_ohlc.low = Some(3_495.0);
        assert!(admit_index_batch(
            &["TEST_CODE_sh000001".to_owned()],
            DataBatch::strict(vec![bad_ohlc.build()], bad_ohlc.provenance()),
            now,
        )
        .is_err());

        assert_eq!(
            missing_field("TEST_CODE_sh000001", "source_at").reason_code(),
            "invalid_evidence"
        );

        let mut future = QuoteFixture::valid(now);
        future.source_at = Some((now + chrono::Duration::seconds(1)).to_rfc3339());
        let error = admit_index_batch(
            &["TEST_CODE_sh000001".to_owned()],
            DataBatch::strict(vec![future.build()], future.provenance()),
            now,
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "index_quote_stale");

        let mut bad_observed = QuoteFixture::valid(now);
        bad_observed.observed_at = "TEST_CODE_bad_timestamp".to_string();
        assert!(admit_index_batch(
            &["TEST_CODE_sh000001".to_owned()],
            DataBatch::strict(vec![bad_observed.build()], bad_observed.provenance()),
            now,
        )
        .is_err());
    }

    #[test]
    fn br164_tencent_error_mapping_preserves_retry_policy() {
        let cases = [
            tencent_gateway_error(TencentError::InvalidRequest("TEST_CODE".into())),
            tencent_gateway_error(TencentError::Transport("TEST_CODE".into())),
            tencent_gateway_error(TencentError::Decode("TEST_CODE".into())),
            tencent_gateway_error(TencentError::Protocol("TEST_CODE".into())),
            tencent_gateway_error(TencentError::Unsupported("TEST_CODE".into())),
        ];
        assert_eq!(cases[0].audit_outcome(), "invalid_request");
        assert_eq!(cases[1].audit_outcome(), "unavailable");
        assert!(cases[1].retryable());
        assert_eq!(cases[2].audit_outcome(), "partial");
        assert_eq!(cases[3].audit_outcome(), "partial");
        assert_eq!(cases[4].audit_outcome(), "unsupported");
    }
}
