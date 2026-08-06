//! Registered business rules: BR-213, BR-220, BR-221.
//! Evidence-preserving upper-limit market projection.

use anyhow::{bail, Result};
use magic_market_core::{LimitPoolEntry, LimitPoolKind, RatioUnit};

use crate::data_gateway::market_capabilities::{MarketCapabilitiesGateway, MarketSecurityIdentity};
use crate::data_gateway::{
    parse_evidence_instant, BatchEvidence, GatewayBatch, GatewayError, ReviewDataGateway,
};
use crate::market_data::TopStock;

use super::MarketAnalyzer;

/// A verified-empty limit-pool response cannot carry name evidence because no
/// identity request is permitted for that state.
#[derive(Debug)]
pub(crate) enum LimitUpStockBatch {
    Available {
        stocks: Vec<TopStock>,
        limit_pool_evidence: BatchEvidence,
        /// BR-220/BR-221: display names come from the daily `SecurityIdentity`
        /// capability, never from a five-second-gated realtime quote batch.
        /// One entry per acquisition shard; shard evidence is never merged.
        name_evidence: Vec<BatchEvidence>,
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

fn compose_limit_up_batch<LoadNames>(
    limit_pool: GatewayBatch<LimitPoolEntry>,
    load_names: LoadNames,
) -> Result<LimitUpStockBatch>
where
    LoadNames:
        FnOnce(
            &[String],
        )
            -> std::result::Result<Vec<GatewayBatch<MarketSecurityIdentity>>, GatewayError>,
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

    // BR-220: names are reference data on a daily freshness budget. Binding a
    // pure display field to the §2.4 five-second quote gate made the entire
    // authoritative limit-pool projection fail whenever any single member's
    // tick lagged, which is a capability mismatch, not a safety property.
    // BR-221: providers cap one request at 50 instruments, so a larger pool is
    // acquired as ordered shards whose evidence stays separate per shard.
    let shards = load_names(&requested_codes)?;
    if shards.is_empty() {
        bail!("BR-221 security identity acquisition produced no shard");
    }
    let mut name_by_code = std::collections::BTreeMap::new();
    let mut name_evidence = Vec::with_capacity(shards.len());
    for shard in shards {
        let (identities, evidence) = match shard {
            GatewayBatch::Available { records, evidence } if !records.is_empty() => {
                (records, evidence)
            }
            GatewayBatch::Available { evidence, .. } | GatewayBatch::VerifiedEmpty(evidence) => {
                bail!(
                    "BR-221 security identity shard carries no display names: source={} batch_id={}",
                    evidence.source,
                    evidence.batch_id
                );
            }
        };
        let shard_observed_at = parse_evidence_instant(
            "BR-220-UpperLimitNames",
            evidence.provider,
            "observed_at",
            &evidence.observed_at,
        )?;
        let _shard_source_at = evidence
            .source_at
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("BR-220 security identity shard has no source time"))
            .and_then(|value| {
                parse_evidence_instant(
                    "BR-220-UpperLimitNames",
                    evidence.provider,
                    "source_at",
                    value,
                )
                .map_err(anyhow::Error::from)
            })?;
        for identity in identities {
            // BR-221: a record is only ever validated against the evidence of
            // the shard it actually came from; shard evidence is never merged
            // or represented by a synthesised batch identity.
            // 归属校验 = provider + batch_id + observed_at (批次身份);
            // source_at 是逐记录时间戳, 与批次级 source_at 天然可差数秒,
            // 相等比较会产生误报 (实证: 002180 于 2026-08-06 09:45 被误拒)。
            if identity.provider != evidence.provider
                || identity.batch_id != evidence.batch_id
                || identity.observed_at != shard_observed_at
            {
                bail!(
                    "BR-221 security identity evidence mismatch for {}",
                    identity.code
                );
            }
            if identity.name.trim().is_empty() {
                bail!(
                    "BR-220 security identity carries no name for {}",
                    identity.code
                );
            }
            if name_by_code
                .insert(identity.code.clone(), identity.name.clone())
                .is_some()
            {
                bail!("BR-221 duplicate security identity {}", identity.code);
            }
        }
        name_evidence.push(evidence);
    }
    let name_codes = name_by_code
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if name_codes != limit_codes {
        bail!(
            "BR-221 exact-code join mismatch limit_codes={limit_codes:?} name_codes={name_codes:?}"
        );
    }

    let mut stocks = Vec::with_capacity(records.len());
    for record in records {
        let code = record.instrument.code().to_owned();
        if record.change.unit() != RatioUnit::Percent {
            bail!("BR-213 upper-limit change unit mismatch for {code}");
        }
        let name = name_by_code
            .get(&code)
            .ok_or_else(|| anyhow::anyhow!("BR-220 missing security identity for {code}"))?
            .clone();
        stocks.push(TopStock {
            code,
            name,
            change_pct: record.change.get(),
            price: record.price.get(),
            volume_ratio: None,
            main_net_yi: None,
        });
    }

    Ok(LimitUpStockBatch::Available {
        stocks,
        limit_pool_evidence,
        name_evidence,
    })
}

/// BR-221: providers accept at most 50 instruments per identity request.
const IDENTITY_REQUEST_SHARD_SIZE: usize = 50;

/// BR-213/BR-220/BR-221: the identity gateway is async and owns a blocking
/// client, so its creation, use and destruction all happen inside one
/// dedicated thread. Requests larger than the provider bound are acquired as
/// ordered shards; every shard keeps its own immutable batch evidence.
fn load_upper_limit_names(
    codes: &[String],
) -> std::result::Result<Vec<GatewayBatch<MarketSecurityIdentity>>, GatewayError> {
    let shards: Vec<Vec<String>> = codes
        .chunks(IDENTITY_REQUEST_SHARD_SIZE)
        .map(<[String]>::to_vec)
        .collect();
    std::thread::Builder::new()
        .name("upper-limit-security-identity".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    GatewayError::unavailable(
                        "BR-220-UpperLimitNames",
                        None,
                        true,
                        format!("create security identity runtime: {error}"),
                    )
                })?;
            let gateway = MarketCapabilitiesGateway::new();
            runtime.block_on(async move {
                let mut batches = Vec::with_capacity(shards.len());
                for shard in shards {
                    // Any shard failure fails the whole enrichment: a partial
                    // name set would silently drop authoritative pool members.
                    batches.push(gateway.security_identities(&shard).await?);
                }
                Ok(batches)
            })
        })
        .map_err(|error| {
            GatewayError::unavailable(
                "BR-220-UpperLimitNames",
                None,
                true,
                format!("spawn security identity thread: {error}"),
            )
        })?
        .join()
        .map_err(|_| {
            GatewayError::unavailable(
                "BR-220-UpperLimitNames",
                None,
                true,
                "security identity thread panicked".to_owned(),
            )
        })?
}

impl MarketAnalyzer {
    /// Return current-session upper-limit membership. Limit-pool facts remain
    /// authoritative; the quote batch contributes the display name only.
    pub(super) fn get_limit_up_from_gateway(
        &self,
        trading_date: chrono::NaiveDate,
    ) -> Result<Vec<TopStock>> {
        let limit_pool = ReviewDataGateway::new().current_upper_limit_pool(trading_date)?;
        let batch = compose_limit_up_batch(limit_pool, load_upper_limit_names)?;
        match &batch {
            LimitUpStockBatch::Available {
                stocks,
                limit_pool_evidence,
                name_evidence,
            } => {
                let receipt = crate::data_gateway::review::audit_limit_up_projection(
                    trading_date,
                    limit_pool_evidence,
                    name_evidence,
                    stocks.len(),
                )?;
                let name_batches = name_evidence
                    .iter()
                    .map(|evidence| format!("{:?}:{}", evidence.provider, evidence.batch_id))
                    .collect::<Vec<_>>()
                    .join(",");
                log::info!(
                    "[DataGateway][BR-213][BR-220][BR-221] status=available date={} records={} limit_provider={:?} limit_batch={} name_shards={} name_batches=[{}] composition_audit_id={} composition_record_hash={}",
                    trading_date,
                    stocks.len(),
                    limit_pool_evidence.provider,
                    limit_pool_evidence.batch_id,
                    name_evidence.len(),
                    name_batches,
                    receipt.audit_id,
                    receipt.record_hash
                );
            }
            LimitUpStockBatch::VerifiedEmpty {
                limit_pool_evidence,
            } => log::info!(
                "[DataGateway][BR-213][BR-220] status=verified_empty date={} records=0 limit_provider={:?} limit_batch={} name_request=not_called",
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

    use chrono::{DateTime, Utc};
    use magic_market_core::{
        AssetClass, Exchange, InstrumentId, IsoDate, Price, ProviderId, Ratio, SourceEvidence,
    };

    const TEST_DATE: &str = "2099-01-02";
    const TEST_DATE_TIME: &str = "2099-01-02T10:00:00+08:00";
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

    /// BR-220: display names now arrive as a daily `SecurityIdentity` batch.
    fn identity_batch(rows: &[(&str, &str)]) -> GatewayBatch<MarketSecurityIdentity> {
        let source_at = DateTime::parse_from_rfc3339(TEST_DATE_TIME)
            .unwrap()
            .with_timezone(&Utc);
        let observed_at = DateTime::parse_from_rfc3339(TEST_OBSERVED_AT)
            .unwrap()
            .with_timezone(&Utc);
        let batch_evidence = BatchEvidence {
            provider: ProviderId::Tencent,
            source: "TEST_CODE_identity".to_owned(),
            source_at: Some(TEST_DATE_TIME.to_owned()),
            observed_at: TEST_OBSERVED_AT.to_owned(),
            batch_id: "TEST_CODE_identity_batch".to_owned(),
        };
        let records = rows
            .iter()
            .map(|(code, name)| MarketSecurityIdentity {
                code: (*code).to_owned(),
                name: (*name).to_owned(),
                is_st: false,
                source_at,
                observed_at,
                provider: ProviderId::Tencent,
                batch_id: "TEST_CODE_identity_batch".to_owned(),
            })
            .collect();
        GatewayBatch::Available {
            records,
            evidence: batch_evidence,
        }
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
    fn br213_verified_empty_does_not_load_display_names() {
        let quote_load_calls = Cell::new(0_u32);
        let limit_pool_evidence = evidence(
            ProviderId::Eastmoney,
            "TEST_CODE_limit_pool",
            "TEST_CODE_limit_empty",
        );
        let batch = compose_limit_up_batch(
            GatewayBatch::VerifiedEmpty(limit_pool_evidence.clone()),
            |_| -> std::result::Result<GatewayBatch<MarketSecurityIdentity>, GatewayError> {
                quote_load_calls.set(quote_load_calls.get() + 1);
                unreachable!("verified-empty limit pool must not request display names")
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
    fn br220_available_batch_uses_pool_facts_and_identity_name_only() {
        let batch_id = "TEST_CODE_limit_available";
        let batch = compose_limit_up_batch(
            available_limit_pool(
                vec![limit_entry("TEST_CODE_600001", batch_id, 12.34, 10.0)],
                batch_id,
            ),
            |_| Ok(identity_batch(&[("TEST_CODE_600001", "TEST_CODE Name")])),
        )
        .unwrap();
        let LimitUpStockBatch::Available {
            stocks,
            limit_pool_evidence,
            name_evidence,
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
        assert_eq!(name_evidence.batch_id, "TEST_CODE_identity_batch");
    }

    #[test]
    fn br220_exact_code_join_rejects_missing_extra_and_duplicate_names() {
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
            assert!(compose_limit_up_batch(pool(), |_| Ok(identity_batch(&rows))).is_err());
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
            Ok(identity_batch(&[("TEST_CODE_600001", "TEST_CODE Name")]))
        })
        .expect_err("conflicting record observed_at must reject the projection");

        assert!(error.to_string().contains("record evidence mismatch"));
    }
}
