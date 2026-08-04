//! Registered business rule: BR-213.
//! Evidence-preserving upper-limit market projection.

use anyhow::{bail, Result};
use magic_market_core::{LimitPoolEntry, LimitPoolKind, RatioUnit};

use crate::data_gateway::market_data::{AdmittedRealtimeQuotes, MarketDataGateway};
use crate::data_gateway::{BatchEvidence, GatewayBatch, GatewayError, ReviewDataGateway};
use crate::market_data::TopStock;

use super::MarketAnalyzer;

/// A verified-empty limit-pool response cannot carry quote evidence because no
/// quote request is permitted for that state.
#[derive(Debug)]
pub(crate) enum LimitUpStockBatch {
    Available {
        stocks: Vec<TopStock>,
        limit_pool_evidence: BatchEvidence,
        quote_evidence: BatchEvidence,
    },
    VerifiedEmpty {
        limit_pool_evidence: BatchEvidence,
    },
}

impl LimitUpStockBatch {
    fn into_stocks(self) -> Vec<TopStock> {
        match self {
            Self::Available { stocks, .. } => stocks,
            Self::VerifiedEmpty { .. } => Vec::new(),
        }
    }
}

fn compose_limit_up_batch<LoadQuotes>(
    limit_pool: GatewayBatch<LimitPoolEntry>,
    load_quotes: LoadQuotes,
) -> Result<LimitUpStockBatch>
where
    LoadQuotes: FnOnce(&[String]) -> std::result::Result<AdmittedRealtimeQuotes, GatewayError>,
{
    let (records, limit_pool_evidence) = match limit_pool {
        GatewayBatch::VerifiedEmpty(limit_pool_evidence) => {
            return Ok(LimitUpStockBatch::VerifiedEmpty {
                limit_pool_evidence,
            });
        }
        GatewayBatch::Available { records, evidence } if records.is_empty() => {
            bail!(
                "BR-213 invalid available upper-limit batch: source={} batch_id={} records=0",
                evidence.source,
                evidence.batch_id
            );
        }
        GatewayBatch::Available { records, evidence } => (records, evidence),
    };

    let mut requested_codes = Vec::with_capacity(records.len());
    let mut limit_codes = std::collections::BTreeSet::new();
    for record in &records {
        let code = record.instrument.code().to_owned();
        if record.kind != LimitPoolKind::Upper
            || record.evidence.provider() != limit_pool_evidence.provider
            || record.evidence.batch_id() != limit_pool_evidence.batch_id
            || record.evidence.source_at() != limit_pool_evidence.source_at.as_deref()
            || record.evidence.observed_at() != limit_pool_evidence.observed_at
        {
            bail!("BR-213 limit-pool record evidence mismatch for {code}");
        }
        if !limit_codes.insert(code.clone()) {
            bail!("BR-213 duplicate limit-pool security {code}");
        }
        requested_codes.push(code);
    }

    let quotes = load_quotes(&requested_codes)?;
    let quote_evidence = quotes.evidence().clone();
    let mut quote_by_code = std::collections::BTreeMap::new();
    for quote in quotes.quotes() {
        if quote.evidence() != &quote_evidence {
            bail!("BR-213 quote record evidence mismatch for {}", quote.code());
        }
        if quote_by_code
            .insert(quote.code().to_owned(), quote)
            .is_some()
        {
            bail!("BR-213 duplicate realtime quote security {}", quote.code());
        }
    }
    let quote_codes = quote_by_code
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if quote_codes != limit_codes {
        bail!(
            "BR-213 exact-code join mismatch limit_codes={limit_codes:?} quote_codes={quote_codes:?}"
        );
    }

    let mut stocks = Vec::with_capacity(records.len());
    for record in records {
        let code = record.instrument.code().to_owned();
        if record.change.unit() != RatioUnit::Percent {
            bail!("BR-213 upper-limit change unit mismatch for {code}");
        }
        let quote = quote_by_code
            .get(&code)
            .ok_or_else(|| anyhow::anyhow!("BR-213 missing realtime quote for {code}"))?;
        stocks.push(TopStock {
            code,
            name: quote.name().to_owned(),
            change_pct: record.change.get(),
            price: record.price.get(),
            volume_ratio: None,
            main_net_yi: None,
        });
    }

    Ok(LimitUpStockBatch::Available {
        stocks,
        limit_pool_evidence,
        quote_evidence,
    })
}

impl MarketAnalyzer {
    /// Return current-session upper-limit membership. Limit-pool facts remain
    /// authoritative; the quote batch contributes the display name only.
    pub(super) fn get_limit_up_from_gateway(
        &self,
        trading_date: chrono::NaiveDate,
    ) -> Result<Vec<TopStock>> {
        let limit_pool = ReviewDataGateway::new().current_upper_limit_pool(trading_date)?;
        let batch = compose_limit_up_batch(limit_pool, |codes| {
            MarketDataGateway::new().required_realtime_quotes(codes)
        })?;
        match &batch {
            LimitUpStockBatch::Available {
                stocks,
                limit_pool_evidence,
                quote_evidence,
            } => {
                let receipt = crate::data_gateway::review::audit_limit_up_projection(
                    trading_date,
                    limit_pool_evidence,
                    quote_evidence,
                    stocks.len(),
                )?;
                log::info!(
                    "[DataGateway][BR-213] status=available date={} records={} limit_provider={:?} limit_batch={} quote_provider={:?} quote_batch={} composition_audit_id={} composition_record_hash={}",
                    trading_date,
                    stocks.len(),
                    limit_pool_evidence.provider,
                    limit_pool_evidence.batch_id,
                    quote_evidence.provider,
                    quote_evidence.batch_id,
                    receipt.audit_id,
                    receipt.record_hash
                );
            }
            LimitUpStockBatch::VerifiedEmpty {
                limit_pool_evidence,
            } => log::info!(
                "[DataGateway][BR-213] status=verified_empty date={} records=0 limit_provider={:?} limit_batch={} quote_request=not_called",
                trading_date,
                limit_pool_evidence.provider,
                limit_pool_evidence.batch_id
            ),
        }
        Ok(batch.into_stocks())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use chrono::Utc;
    use magic_market_core::{
        AssetClass, Exchange, InstrumentId, IsoDate, Price, ProviderId, Ratio, SourceEvidence,
    };

    use crate::data_gateway::market_data::{AdmittedRealtimeQuote, RealtimeMarketQuote};

    const TEST_DATE: &str = "2099-01-02";
    const TEST_OBSERVED_AT: &str = "2099-01-02T10:00:01+08:00";

    fn evidence(provider: ProviderId, source: &str, batch_id: &str) -> BatchEvidence {
        BatchEvidence {
            provider,
            source: source.to_owned(),
            source_at: Some(TEST_DATE.to_owned()),
            observed_at: TEST_OBSERVED_AT.to_owned(),
            batch_id: batch_id.to_owned(),
        }
    }

    fn limit_entry(code: &str, batch_id: &str, price: f64, change: f64) -> LimitPoolEntry {
        LimitPoolEntry {
            kind: LimitPoolKind::Upper,
            instrument: InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap(),
            trading_date: IsoDate::new(TEST_DATE).unwrap(),
            price: Price::new(price).unwrap(),
            change: Ratio::new(change, RatioUnit::Percent).unwrap(),
            volume: None,
            turnover: None,
            sealed_amount: None,
            first_seal_at: None,
            last_seal_at: None,
            break_count: None,
            streak: None,
            industry: None,
            board_name: None,
            seal_state: None,
            reseal_count: None,
            reason: None,
            evidence: SourceEvidence::new(ProviderId::Eastmoney, TEST_OBSERVED_AT, batch_id)
                .unwrap()
                .with_source_at(TEST_DATE)
                .unwrap(),
        }
    }

    fn quote_batch(rows: &[(&str, &str)]) -> AdmittedRealtimeQuotes {
        let now = Utc::now();
        let batch_evidence = BatchEvidence {
            provider: ProviderId::Tencent,
            source: "TEST_CODE_quote".to_owned(),
            source_at: Some(now.to_rfc3339()),
            observed_at: now.to_rfc3339(),
            batch_id: "TEST_CODE_quote_batch".to_owned(),
        };
        let quotes = rows
            .iter()
            .map(|(code, name)| {
                AdmittedRealtimeQuote::from_test_fixture(
                    RealtimeMarketQuote {
                        code: (*code).to_owned(),
                        name: (*name).to_owned(),
                        price: 10.0,
                        previous_close: 9.0,
                        change_percent: 11.11,
                        source_at: now,
                        observed_at: now,
                        provider: ProviderId::Tencent,
                        batch_id: "TEST_CODE_quote_batch".to_owned(),
                    },
                    batch_evidence.clone(),
                )
                .unwrap()
            })
            .collect();
        AdmittedRealtimeQuotes::from_test_fixtures(quotes).unwrap()
    }

    fn available_limit_pool(
        records: Vec<LimitPoolEntry>,
        batch_id: &str,
    ) -> GatewayBatch<LimitPoolEntry> {
        GatewayBatch::Available {
            records,
            evidence: evidence(ProviderId::Eastmoney, "TEST_CODE_limit_pool", batch_id),
        }
    }

    #[test]
    fn br213_verified_empty_does_not_load_realtime_quotes() {
        let quote_load_calls = Cell::new(0_u32);
        let limit_pool_evidence = evidence(
            ProviderId::Eastmoney,
            "TEST_CODE_limit_pool",
            "TEST_CODE_limit_empty",
        );
        let batch = compose_limit_up_batch(
            GatewayBatch::VerifiedEmpty(limit_pool_evidence.clone()),
            |_| -> std::result::Result<AdmittedRealtimeQuotes, GatewayError> {
                quote_load_calls.set(quote_load_calls.get() + 1);
                unreachable!("verified-empty limit pool must not request quotes")
            },
        )
        .unwrap();

        assert_eq!(quote_load_calls.get(), 0);
        assert!(matches!(
            batch,
            LimitUpStockBatch::VerifiedEmpty { limit_pool_evidence: actual }
                if actual == limit_pool_evidence
        ));
    }

    #[test]
    fn br213_available_batch_uses_pool_facts_and_quote_name_only() {
        let batch_id = "TEST_CODE_limit_available";
        let batch = compose_limit_up_batch(
            available_limit_pool(
                vec![limit_entry("TEST_CODE_600001", batch_id, 12.34, 10.0)],
                batch_id,
            ),
            |_| Ok(quote_batch(&[("TEST_CODE_600001", "TEST_CODE Name")])),
        )
        .unwrap();
        let LimitUpStockBatch::Available {
            stocks,
            limit_pool_evidence,
            quote_evidence,
        } = batch
        else {
            panic!("expected available batch")
        };

        assert_eq!(stocks.len(), 1);
        assert_eq!(stocks[0].code, "TEST_CODE_600001");
        assert_eq!(stocks[0].name, "TEST_CODE Name");
        assert_eq!(stocks[0].price, 12.34);
        assert_eq!(stocks[0].change_pct, 10.0);
        assert_eq!(stocks[0].volume_ratio, None);
        assert_eq!(stocks[0].main_net_yi, None);
        assert_eq!(limit_pool_evidence.batch_id, batch_id);
        assert_eq!(quote_evidence.batch_id, "TEST_CODE_quote_batch");
    }

    #[test]
    fn br213_exact_code_join_rejects_missing_extra_and_duplicate_quotes() {
        let batch_id = "TEST_CODE_limit_join";
        let pool = || {
            available_limit_pool(
                vec![
                    limit_entry("TEST_CODE_600001", batch_id, 10.0, 10.0),
                    limit_entry("TEST_CODE_600002", batch_id, 20.0, 10.0),
                ],
                batch_id,
            )
        };

        for rows in [
            vec![("TEST_CODE_600001", "TEST_CODE One")],
            vec![
                ("TEST_CODE_600001", "TEST_CODE One"),
                ("TEST_CODE_600002", "TEST_CODE Two"),
                ("TEST_CODE_600003", "TEST_CODE Extra"),
            ],
            vec![
                ("TEST_CODE_600001", "TEST_CODE One"),
                ("TEST_CODE_600001", "TEST_CODE Duplicate"),
            ],
        ] {
            assert!(compose_limit_up_batch(pool(), |_| Ok(quote_batch(&rows))).is_err());
        }
    }

    #[test]
    fn br213_rejects_limit_record_with_conflicting_observed_at() {
        let batch_id = "TEST_CODE_limit_observed_at";
        let mut record = limit_entry("TEST_CODE_600001", batch_id, 10.0, 10.0);
        record.evidence =
            SourceEvidence::new(ProviderId::Eastmoney, "2099-01-02T10:00:02+08:00", batch_id)
                .unwrap()
                .with_source_at(TEST_DATE)
                .unwrap();

        let error = compose_limit_up_batch(available_limit_pool(vec![record], batch_id), |_| {
            Ok(quote_batch(&[("TEST_CODE_600001", "TEST_CODE Name")]))
        })
        .expect_err("conflicting record observed_at must reject the projection");

        assert!(error.to_string().contains("record evidence mismatch"));
    }
}
