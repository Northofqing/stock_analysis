//! Registered business rules: BR-213, BR-220, BR-221.
//! Evidence-preserving upper-limit market projection.

use crate::market_domain::{LimitPoolEntry, LimitPoolKind, RatioUnit};
use anyhow::{bail, Result};

use crate::data_gateway::market_capabilities::{MarketCapabilitiesGateway, MarketSecurityIdentity};
use crate::data_gateway::review::AuditedGatewayBatch;
use crate::data_gateway::{
    parse_evidence_instant, BatchEvidence, GatewayBatch, GatewayError, ReviewDataGateway,
};
use crate::market_data::TopStock;

use super::MarketAnalyzer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitUpObservationStatus {
    Available,
    VerifiedEmpty,
}

#[derive(Clone)]
pub struct LimitUpNameShardObservation {
    requested_codes: Vec<String>,
    acquisition: AuditedGatewayBatch<MarketSecurityIdentity>,
}

impl LimitUpNameShardObservation {
    fn new(
        requested_codes: Vec<String>,
        acquisition: AuditedGatewayBatch<MarketSecurityIdentity>,
    ) -> Self {
        Self {
            requested_codes,
            acquisition,
        }
    }

    pub fn requested_codes(&self) -> &[String] {
        &self.requested_codes
    }

    pub fn batch(&self) -> &GatewayBatch<MarketSecurityIdentity> {
        self.acquisition.batch()
    }

    pub fn request_hash(&self) -> &str {
        self.acquisition.request_hash()
    }

    pub fn receipt(&self) -> &crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt {
        self.acquisition.receipt()
    }
}

impl std::fmt::Debug for LimitUpNameShardObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LimitUpNameShardObservation")
            .field("requested_count", &self.requested_codes.len())
            .field("record_count", &self.acquisition.batch().records().len())
            .finish()
    }
}

#[derive(Clone)]
pub struct LimitUpObservation {
    trading_date: chrono::NaiveDate,
    limit_pool: AuditedGatewayBatch<LimitPoolEntry>,
    name_shards: Vec<LimitUpNameShardObservation>,
    stocks: Vec<TopStock>,
    composition_receipt:
        Option<crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt>,
}

impl LimitUpObservation {
    pub fn trading_date(&self) -> chrono::NaiveDate {
        self.trading_date
    }

    pub fn status(&self) -> LimitUpObservationStatus {
        if self.limit_pool.batch().is_verified_empty() {
            LimitUpObservationStatus::VerifiedEmpty
        } else {
            LimitUpObservationStatus::Available
        }
    }

    pub fn limit_pool_batch(&self) -> &GatewayBatch<LimitPoolEntry> {
        self.limit_pool.batch()
    }

    pub fn limit_pool_request_hash(&self) -> &str {
        self.limit_pool.request_hash()
    }

    pub fn limit_pool_receipt(
        &self,
    ) -> &crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt {
        self.limit_pool.receipt()
    }

    pub fn name_shards(&self) -> &[LimitUpNameShardObservation] {
        &self.name_shards
    }

    pub fn stocks(&self) -> &[TopStock] {
        &self.stocks
    }

    pub fn composition_receipt(
        &self,
    ) -> Option<&crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt> {
        self.composition_receipt.as_ref()
    }
}

impl std::fmt::Debug for LimitUpObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LimitUpObservation")
            .field("status", &self.status())
            .field(
                "limit_pool_records",
                &self.limit_pool.batch().records().len(),
            )
            .field("name_shards", &self.name_shards.len())
            .field("stocks", &self.stocks.len())
            .field(
                "has_composition_receipt",
                &self.composition_receipt.is_some(),
            )
            .finish()
    }
}

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

fn compose_limit_up_batch<LoadNames>(
    limit_pool: GatewayBatch<LimitPoolEntry>,
    load_names: LoadNames,
) -> Result<LimitUpStockBatch>
where
    LoadNames: FnOnce(&[String]) -> Result<Vec<GatewayBatch<MarketSecurityIdentity>>>,
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

fn validate_name_shard_observations(
    requested_codes: &[String],
    shards: &[LimitUpNameShardObservation],
) -> Result<()> {
    if shards.is_empty() {
        bail!("BR-221 security identity acquisition produced no shard");
    }
    let mut flattened_requests = Vec::with_capacity(requested_codes.len());
    for shard in shards {
        if shard.requested_codes.is_empty()
            || shard.requested_codes.len() > IDENTITY_REQUEST_SHARD_SIZE
        {
            bail!("BR-221 security identity request shard has invalid size");
        }
        let expected = shard
            .requested_codes
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        if expected.len() != shard.requested_codes.len() {
            bail!("BR-221 security identity request shard contains duplicate codes");
        }
        let actual = match shard.acquisition.batch() {
            GatewayBatch::Available { records, .. } if !records.is_empty() => records
                .iter()
                .map(|record| record.code.clone())
                .collect::<std::collections::BTreeSet<_>>(),
            GatewayBatch::Available { .. } | GatewayBatch::VerifiedEmpty(_) => {
                bail!("BR-221 security identity observation carries no records")
            }
        };
        if actual.len() != shard.acquisition.batch().records().len() || actual != expected {
            bail!("BR-221 security identity observation differs from its request shard");
        }
        flattened_requests.extend(shard.requested_codes.iter().cloned());
    }
    if flattened_requests != requested_codes {
        bail!("BR-221 security identity request shards differ from pool member order");
    }
    Ok(())
}

fn assemble_limit_up_observation<LoadNames, AuditComposition>(
    trading_date: chrono::NaiveDate,
    limit_pool: AuditedGatewayBatch<LimitPoolEntry>,
    load_names: LoadNames,
    audit_composition: AuditComposition,
) -> Result<LimitUpObservation>
where
    LoadNames: FnOnce(&[String]) -> Result<Vec<LimitUpNameShardObservation>>,
    AuditComposition: FnOnce(
        chrono::NaiveDate,
        &BatchEvidence,
        &[BatchEvidence],
        usize,
    ) -> Result<
        crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    >,
{
    let mut retained_name_shards = None;
    let projected = compose_limit_up_batch(limit_pool.batch().clone(), |requested_codes| {
        let shards = load_names(requested_codes)?;
        validate_name_shard_observations(requested_codes, &shards)?;
        let batches = shards
            .iter()
            .map(|shard| shard.acquisition.batch().clone())
            .collect();
        retained_name_shards = Some(shards);
        Ok(batches)
    })?;

    match projected {
        LimitUpStockBatch::VerifiedEmpty {
            limit_pool_evidence,
        } => {
            if &limit_pool_evidence != limit_pool.batch().evidence() {
                bail!(
                    "BR-213 verified-empty projection evidence differs from retained acquisition"
                );
            }
            Ok(LimitUpObservation {
                trading_date,
                limit_pool,
                name_shards: Vec::new(),
                stocks: Vec::new(),
                composition_receipt: None,
            })
        }
        LimitUpStockBatch::Available {
            stocks,
            limit_pool_evidence,
            name_evidence,
        } => {
            let name_shards = retained_name_shards.ok_or_else(|| {
                anyhow::anyhow!("BR-221 available projection lost name shard observations")
            })?;
            let composition_receipt = audit_composition(
                trading_date,
                &limit_pool_evidence,
                &name_evidence,
                stocks.len(),
            )?;
            Ok(LimitUpObservation {
                trading_date,
                limit_pool,
                name_shards,
                stocks,
                composition_receipt: Some(composition_receipt),
            })
        }
    }
}

/// BR-221: providers accept at most 50 instruments per identity request.
const IDENTITY_REQUEST_SHARD_SIZE: usize = 50;

fn collect_upper_limit_name_observations<Acquire>(
    codes: &[String],
    mut acquire: Acquire,
) -> std::result::Result<Vec<LimitUpNameShardObservation>, GatewayError>
where
    Acquire:
        FnMut(
            &[String],
        )
            -> std::result::Result<AuditedGatewayBatch<MarketSecurityIdentity>, GatewayError>,
{
    let mut observations = Vec::with_capacity(codes.len().div_ceil(IDENTITY_REQUEST_SHARD_SIZE));
    for requested_codes in codes.chunks(IDENTITY_REQUEST_SHARD_SIZE) {
        let requested_codes = requested_codes.to_vec();
        // Any shard failure stops the same production loop immediately: no
        // partial observation, retry or later-shard request is permitted.
        let acquisition = acquire(&requested_codes)?;
        observations.push(LimitUpNameShardObservation::new(
            requested_codes,
            acquisition,
        ));
    }
    Ok(observations)
}

/// BR-213/BR-220/BR-221: the identity gateway is async and owns a blocking
/// client, so its creation, use and destruction all happen inside one
/// dedicated thread. Requests larger than the provider bound are acquired as
/// ordered shards; every shard keeps its own immutable batch evidence.
fn load_upper_limit_name_observations(
    codes: &[String],
) -> std::result::Result<Vec<LimitUpNameShardObservation>, GatewayError> {
    let codes = codes.to_vec();
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
            collect_upper_limit_name_observations(&codes, |requested_codes| {
                runtime.block_on(gateway.security_identities_observation(requested_codes))
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
    /// Capture the exact-date upper-limit acquisition, name shards and
    /// composition audit from one admitted production path.
    pub fn get_limit_up_observation(
        &self,
        trading_date: chrono::NaiveDate,
    ) -> Result<LimitUpObservation> {
        let limit_pool =
            ReviewDataGateway::new().current_upper_limit_pool_observation(trading_date)?;
        let observation = assemble_limit_up_observation(
            trading_date,
            limit_pool,
            |codes| load_upper_limit_name_observations(codes).map_err(anyhow::Error::from),
            |date, limit_evidence, name_evidence, record_count| {
                crate::data_gateway::review::audit_limit_up_projection(
                    date,
                    limit_evidence,
                    name_evidence,
                    record_count,
                )
                .map_err(anyhow::Error::from)
            },
        )?;
        match observation.status() {
            LimitUpObservationStatus::Available => {
                let limit_pool_evidence = observation.limit_pool_batch().evidence();
                let name_batches = observation
                    .name_shards()
                    .iter()
                    .map(|shard| {
                        let evidence = shard.batch().evidence();
                        format!("{:?}:{}", evidence.provider, evidence.batch_id)
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let receipt = observation
                    .composition_receipt()
                    .expect("available observation must retain composition receipt");
                log::info!(
                    "[DataGateway][BR-213][BR-220][BR-221] status=available date={} records={} limit_provider={:?} limit_batch={} name_shards={} name_batches=[{}] composition_audit_id={} composition_record_hash={}",
                    trading_date,
                    observation.stocks().len(),
                    limit_pool_evidence.provider,
                    limit_pool_evidence.batch_id,
                    observation.name_shards().len(),
                    name_batches,
                    receipt.audit_id,
                    receipt.record_hash
                );
            }
            LimitUpObservationStatus::VerifiedEmpty => {
                let limit_pool_evidence = observation.limit_pool_batch().evidence();
                log::info!(
                    "[DataGateway][BR-213][BR-220] status=verified_empty date={} records=0 limit_provider={:?} limit_batch={} name_request=not_called",
                    trading_date,
                    limit_pool_evidence.provider,
                    limit_pool_evidence.batch_id
                )
            }
        }
        Ok(observation)
    }

    /// Compatibility projection used by the existing P-01/current-pool path.
    pub(super) fn get_limit_up_from_gateway(
        &self,
        trading_date: chrono::NaiveDate,
    ) -> Result<Vec<TopStock>> {
        self.get_limit_up_observation(trading_date)
            .map(|observation| observation.stocks().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use crate::market_domain::ProviderId;
    use crate::market_domain::{
        AssetClass, Exchange, InstrumentId, IsoDate, Money, NonEmptyText, PositiveU32, Price,
        Quantity, Ratio, SourceEvidence,
    };
    use chrono::{DateTime, Utc};
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Integer, Text};

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

    /// BR-220: display names now arrive as daily `SecurityIdentity` shards.
    /// BR-221: the projection consumes `Vec<GatewayBatch<...>>` (one entry per
    /// acquisition shard, retained separately), so tests build shards too.
    fn identity_batches(rows: &[(&str, &str)]) -> Vec<GatewayBatch<MarketSecurityIdentity>> {
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
        vec![GatewayBatch::Available {
            records,
            evidence: batch_evidence,
        }]
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

    #[derive(QueryableByName)]
    struct AuditFactRow {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        capability: String,
        #[diesel(sql_type = Text)]
        provider: String,
        #[diesel(sql_type = Text)]
        request_hash: String,
        #[diesel(sql_type = Text)]
        outcome: String,
        #[diesel(sql_type = Text)]
        reason_code: String,
        #[diesel(sql_type = Integer)]
        retryable: i32,
    }

    #[derive(QueryableByName)]
    struct AuditChainFactRow {
        #[diesel(sql_type = Text)]
        record_hash: String,
    }

    fn audit_session() -> (
        tempfile::NamedTempFile,
        crate::database::attribution_reports::AttributionDatabaseSession,
    ) {
        let file = tempfile::NamedTempFile::new().expect("TEST_CODE audit database file");
        let session = crate::database::attribution_reports::AttributionDatabaseSession::open(
            file.path(),
            crate::database::attribution_reports::AttributionDatabaseAccess::AppendOnly,
        )
        .expect("TEST_CODE append-only audit database");
        (file, session)
    }

    fn audit_rows(database: &crate::database::DatabaseManager) -> Vec<AuditFactRow> {
        let mut connection = database.get_conn().expect("TEST_CODE audit connection");
        diesel::sql_query(
            "SELECT id, capability, provider, request_hash, outcome, reason_code, retryable \
             FROM data_acquisition_audit ORDER BY id",
        )
        .load(&mut connection)
        .expect("TEST_CODE audit rows")
    }

    fn assert_receipt_persisted(
        database: &crate::database::DatabaseManager,
        receipt: &crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    ) {
        let mut connection = database.get_conn().expect("TEST_CODE audit connection");
        let chain = diesel::sql_query(
            "SELECT record_hash FROM data_acquisition_audit_chain \
             WHERE acquisition_audit_id = ?",
        )
        .bind::<BigInt, _>(receipt.audit_id)
        .get_result::<AuditChainFactRow>(&mut connection)
        .expect("TEST_CODE persisted receipt chain row");
        assert_eq!(chain.record_hash, receipt.record_hash);
    }

    fn identity_batch_for_codes(
        codes: &[String],
        provider: ProviderId,
        batch_id: &str,
    ) -> GatewayBatch<MarketSecurityIdentity> {
        let batch_source_at = DateTime::parse_from_rfc3339(TEST_DATE_TIME)
            .unwrap()
            .with_timezone(&Utc);
        let record_source_at = batch_source_at + chrono::Duration::seconds(5);
        let observed_at = DateTime::parse_from_rfc3339(TEST_OBSERVED_AT)
            .unwrap()
            .with_timezone(&Utc);
        GatewayBatch::Available {
            records: codes
                .iter()
                .map(|code| MarketSecurityIdentity {
                    code: code.clone(),
                    name: format!("Name {code}"),
                    is_st: false,
                    source_at: record_source_at,
                    observed_at,
                    provider,
                    batch_id: batch_id.to_owned(),
                })
                .collect(),
            evidence: BatchEvidence {
                provider,
                source: "TEST_CODE_identity".to_owned(),
                source_at: Some(TEST_DATE_TIME.to_owned()),
                observed_at: TEST_OBSERVED_AT.to_owned(),
                batch_id: batch_id.to_owned(),
            },
        }
    }

    fn limit_records(count: usize, batch_id: &str) -> Vec<LimitPoolEntry> {
        let mut records = (0..count)
            .map(|index| {
                limit_entry(
                    &format!("TEST_CODE_{:06}", 600_000 + index),
                    batch_id,
                    10.0 + index as f64,
                    10.0,
                )
            })
            .collect::<Vec<_>>();
        if let Some(first) = records.first_mut() {
            first.volume = Some(Quantity::new(123_456.0).unwrap());
            first.turnover = Some(Ratio::new(7.5, RatioUnit::Percent).unwrap());
            first.sealed_amount = Some(Money::new(88_000_000.0).unwrap());
            first.first_seal_at = Some(NonEmptyText::new("09:31:01").unwrap());
            first.last_seal_at = Some(NonEmptyText::new("14:55:02").unwrap());
            first.break_count = Some(2);
            first.streak = Some(PositiveU32::new(3).unwrap());
            first.industry = Some(NonEmptyText::new("TEST_INDUSTRY").unwrap());
            first.board_name = Some(NonEmptyText::new("TEST_BOARD").unwrap());
            first.seal_state = Some(NonEmptyText::new("TEST_SEALED").unwrap());
            first.reseal_count = Some(1);
            first.reason = Some(NonEmptyText::new("TEST_REASON").unwrap());
        }
        records
    }

    #[test]
    fn br213_observation_retains_full_pool_name_shards_and_real_audit_receipts() {
        let (_file, session) = audit_session();
        let database = session.database();
        let trading_date = chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let batch_id = "TEST_CODE_limit_observation";
        let raw_records = limit_records(51, batch_id);
        let expected_pool = available_limit_pool(raw_records.clone(), batch_id);
        let limit_pool = crate::data_gateway::review::current_upper_limit_pool_observation_in(
            database,
            trading_date,
            Ok(expected_pool.clone()),
        )
        .expect("TEST_CODE audited pool observation");

        let observation = assemble_limit_up_observation(
            trading_date,
            limit_pool,
            |requested_codes| {
                let shard_index = Cell::new(0);
                collect_upper_limit_name_observations(requested_codes, |requested_shard| {
                    let index = shard_index.get();
                    shard_index.set(index + 1);
                    let batch_id = format!("TEST_CODE_name_shard_{index}");
                    crate::data_gateway::market_capabilities::security_identities_observation_in(
                        database,
                        requested_shard,
                        Ok(identity_batch_for_codes(
                            requested_shard,
                            ProviderId::Tencent,
                            &batch_id,
                        )),
                    )
                })
                .map_err(anyhow::Error::from)
            },
            |date, pool_evidence, name_evidence, record_count| {
                crate::data_gateway::review::audit_limit_up_projection_in(
                    Some(database),
                    date,
                    pool_evidence,
                    name_evidence,
                    record_count,
                )
                .map_err(anyhow::Error::from)
            },
        )
        .expect("TEST_CODE complete source observation");

        assert_eq!(observation.trading_date(), trading_date);
        assert_eq!(observation.status(), LimitUpObservationStatus::Available);
        assert_eq!(observation.limit_pool_batch(), &expected_pool);
        assert_eq!(
            observation.limit_pool_batch().records(),
            raw_records.as_slice()
        );
        assert_eq!(observation.name_shards().len(), 2);
        let expected_codes = raw_records
            .iter()
            .map(|record| record.instrument.code().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            observation.name_shards()[0].requested_codes(),
            &expected_codes[..50]
        );
        assert_eq!(
            observation.name_shards()[1].requested_codes(),
            &expected_codes[50..]
        );
        for shard in observation.name_shards() {
            assert_eq!(
                shard
                    .batch()
                    .records()
                    .iter()
                    .map(|identity| identity.code.as_str())
                    .collect::<Vec<_>>(),
                shard
                    .requested_codes()
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(observation.stocks().len(), 51);
        assert_eq!(
            observation.stocks()[0].code,
            raw_records[0].instrument.code()
        );
        assert_eq!(observation.stocks()[0].volume_ratio, None);
        assert_eq!(observation.stocks()[0].main_net_yi, None);
        let identity = &observation.name_shards()[0].batch().records()[0];
        assert_ne!(
            identity.source_at.to_rfc3339(),
            observation.name_shards()[0]
                .batch()
                .evidence()
                .source_at
                .as_deref()
                .unwrap()
        );

        let rows = audit_rows(database);
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter()
                .map(|row| row.capability.as_str())
                .collect::<Vec<_>>(),
            vec![
                "BR-213-UpperLimitPool",
                "SecurityIdentity",
                "SecurityIdentity",
                "BR-213-UpperLimitProjection",
            ]
        );
        assert_eq!(rows[0].provider, "Eastmoney");
        assert_eq!(rows[1].provider, "Tencent");
        assert_eq!(rows[2].provider, "Tencent");
        assert_eq!(rows[3].provider, "Composite");
        assert_eq!(rows[0].request_hash, observation.limit_pool_request_hash());
        assert_eq!(
            rows[1].request_hash,
            observation.name_shards()[0].request_hash()
        );
        assert_eq!(
            rows[2].request_hash,
            observation.name_shards()[1].request_hash()
        );
        assert_eq!(rows[0].id, observation.limit_pool_receipt().audit_id);
        assert_eq!(
            rows[3].id,
            observation.composition_receipt().unwrap().audit_id
        );
        assert_receipt_persisted(database, observation.limit_pool_receipt());
        for shard in observation.name_shards() {
            assert_receipt_persisted(database, shard.receipt());
        }
        assert_receipt_persisted(database, observation.composition_receipt().unwrap());

        let audits_before_compatibility_projection = rows.len();
        let compatible_stocks = observation.stocks().to_vec();
        assert_eq!(compatible_stocks.len(), observation.stocks().len());
        for (compatible, retained) in compatible_stocks.iter().zip(observation.stocks()) {
            assert_eq!(compatible.code, retained.code);
            assert_eq!(compatible.name, retained.name);
            assert_eq!(
                compatible.change_pct.to_bits(),
                retained.change_pct.to_bits()
            );
            assert_eq!(compatible.price.to_bits(), retained.price.to_bits());
            assert_eq!(
                compatible.volume_ratio.map(f64::to_bits),
                retained.volume_ratio.map(f64::to_bits)
            );
            assert_eq!(
                compatible.main_net_yi.map(f64::to_bits),
                retained.main_net_yi.map(f64::to_bits)
            );
        }
        assert_eq!(
            audit_rows(database).len(),
            audits_before_compatibility_projection,
            "projecting the retained observation must not append another audit"
        );

        let debug = format!("{observation:?}");
        assert!(!debug.contains(raw_records[0].instrument.code()));
        assert!(!debug.contains(observation.limit_pool_request_hash()));
        assert!(!debug.contains(&observation.limit_pool_receipt().record_hash));
        let shard_debug = format!("{:?}", &observation.name_shards()[0]);
        assert!(!shard_debug.contains(observation.name_shards()[0].requested_codes()[0].as_str()));
        assert!(!shard_debug.contains(observation.name_shards()[0].request_hash()));
    }

    #[test]
    fn br213_observation_verified_empty_retains_only_real_pool_audit() {
        let (_file, session) = audit_session();
        let database = session.database();
        let trading_date = chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let empty_evidence = evidence(
            ProviderId::Eastmoney,
            "TEST_CODE_limit_pool",
            "TEST_CODE_empty_observation",
        );
        let limit_pool = crate::data_gateway::review::current_upper_limit_pool_observation_in(
            database,
            trading_date,
            Ok(GatewayBatch::VerifiedEmpty(empty_evidence.clone())),
        )
        .expect("TEST_CODE audited empty pool");
        let name_calls = Cell::new(0);
        let composition_calls = Cell::new(0);

        let observation = assemble_limit_up_observation(
            trading_date,
            limit_pool,
            |_| {
                name_calls.set(name_calls.get() + 1);
                unreachable!("verified empty must not load names")
            },
            |_, _, _, _| {
                composition_calls.set(composition_calls.get() + 1);
                unreachable!("verified empty must not audit composition")
            },
        )
        .expect("TEST_CODE verified empty observation");

        assert_eq!(
            observation.status(),
            LimitUpObservationStatus::VerifiedEmpty
        );
        assert!(observation.limit_pool_batch().is_verified_empty());
        assert_eq!(observation.limit_pool_batch().evidence(), &empty_evidence);
        assert!(observation.name_shards().is_empty());
        assert!(observation.stocks().is_empty());
        assert!(observation.composition_receipt().is_none());
        assert_eq!(name_calls.get(), 0);
        assert_eq!(composition_calls.get(), 0);
        assert_eq!(audit_rows(database).len(), 1);
        assert_receipt_persisted(database, observation.limit_pool_receipt());
    }

    #[test]
    fn br221_observation_rejects_wrong_duplicate_and_missing_request_shards() {
        #[derive(Clone, Copy)]
        enum InvalidShards {
            WrongShard,
            Duplicate,
            Missing,
        }

        for scenario in [
            InvalidShards::WrongShard,
            InvalidShards::Duplicate,
            InvalidShards::Missing,
        ] {
            let (_file, session) = audit_session();
            let database = session.database();
            let trading_date = chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
            let batch_id = "TEST_CODE_invalid_shards";
            let pool_records = limit_records(2, batch_id);
            let expected_codes = pool_records
                .iter()
                .map(|record| record.instrument.code().to_owned())
                .collect::<Vec<_>>();
            let limit_pool = crate::data_gateway::review::current_upper_limit_pool_observation_in(
                database,
                trading_date,
                Ok(available_limit_pool(pool_records, batch_id)),
            )
            .expect("TEST_CODE audited invalid-shard pool");
            let composition_calls = Cell::new(0);

            let result = assemble_limit_up_observation(
                trading_date,
                limit_pool,
                |requested_codes| {
                    assert_eq!(requested_codes, expected_codes.as_slice());
                    let shard_bindings = match scenario {
                        InvalidShards::WrongShard => vec![
                            (
                                vec![expected_codes[1].clone()],
                                vec![expected_codes[0].clone()],
                            ),
                            (
                                vec![expected_codes[0].clone()],
                                vec![expected_codes[1].clone()],
                            ),
                        ],
                        InvalidShards::Duplicate => vec![
                            (
                                vec![expected_codes[0].clone()],
                                vec![expected_codes[0].clone()],
                            ),
                            (
                                vec![expected_codes[0].clone()],
                                vec![expected_codes[0].clone()],
                            ),
                        ],
                        InvalidShards::Missing => vec![(
                            vec![expected_codes[0].clone()],
                            vec![expected_codes[0].clone()],
                        )],
                    };
                    shard_bindings
                        .into_iter()
                        .enumerate()
                        .map(|(index, (reported_codes, acquisition_codes))| {
                            let shard_batch_id = format!("TEST_CODE_invalid_name_shard_{index}");
                            let acquisition = crate::data_gateway::market_capabilities::security_identities_observation_in(
                                database,
                                &acquisition_codes,
                                Ok(identity_batch_for_codes(
                                    &acquisition_codes,
                                    ProviderId::Tencent,
                                    &shard_batch_id,
                                )),
                            )?;
                            Ok(LimitUpNameShardObservation::new(
                                reported_codes,
                                acquisition,
                            ))
                        })
                        .collect()
                },
                |_, _, _, _| {
                    composition_calls.set(composition_calls.get() + 1);
                    unreachable!("invalid request shards must reject before composition")
                },
            );

            assert!(result.is_err());
            assert_eq!(composition_calls.get(), 0);
            let expected_audits = match scenario {
                InvalidShards::WrongShard | InvalidShards::Duplicate => 3,
                InvalidShards::Missing => 2,
            };
            assert_eq!(audit_rows(database).len(), expected_audits);
        }
    }

    #[test]
    fn br221_observation_second_shard_failure_preserves_prior_audits_without_retry() {
        let (_file, session) = audit_session();
        let database = session.database();
        let trading_date = chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let batch_id = "TEST_CODE_second_shard_failure";
        let limit_pool = crate::data_gateway::review::current_upper_limit_pool_observation_in(
            database,
            trading_date,
            Ok(available_limit_pool(limit_records(51, batch_id), batch_id)),
        )
        .expect("TEST_CODE audited two-shard pool");
        let name_calls = Cell::new(0);
        let composition_calls = Cell::new(0);

        let result = assemble_limit_up_observation(
            trading_date,
            limit_pool,
            |requested_codes| {
                collect_upper_limit_name_observations(requested_codes, |shard| {
                    name_calls.set(name_calls.get() + 1);
                    if name_calls.get() == 2 {
                        let failure = GatewayError::unavailable(
                            "SecurityIdentity",
                            Some(ProviderId::Sina),
                            true,
                            "TEST_CODE second shard unavailable",
                        );
                        crate::data_gateway::market_capabilities::security_identities_observation_in(
                            database,
                            shard,
                            Err(failure),
                        )
                    } else {
                        crate::data_gateway::market_capabilities::security_identities_observation_in(
                            database,
                            shard,
                            Ok(identity_batch_for_codes(
                                shard,
                                ProviderId::Tencent,
                                "TEST_CODE_first_name_shard",
                            )),
                        )
                    }
                })
                .map_err(anyhow::Error::from)
            },
            |_, _, _, _| {
                composition_calls.set(composition_calls.get() + 1);
                unreachable!("name failure must reject before composition")
            },
        );

        assert!(result.is_err());
        assert_eq!(name_calls.get(), 2, "failed shard must not be retried");
        assert_eq!(composition_calls.get(), 0);
        let rows = audit_rows(database);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].capability, "BR-213-UpperLimitPool");
        assert_eq!(rows[1].capability, "SecurityIdentity");
        assert_eq!(rows[1].outcome, "available");
        assert_eq!(rows[2].capability, "SecurityIdentity");
        assert_eq!(rows[2].provider, "Sina");
        assert_eq!(rows[2].outcome, "unavailable");
        assert_eq!(rows[2].reason_code, "no_verified_batch");
        assert_eq!(rows[2].retryable, 1);
    }

    #[test]
    fn br213_observation_composition_append_failure_preserves_acquisition_audits() {
        let (_file, session) = audit_session();
        let database = session.database();
        let trading_date = chrono::NaiveDate::from_ymd_opt(2099, 1, 2).unwrap();
        let batch_id = "TEST_CODE_composition_failure";
        let limit_pool = crate::data_gateway::review::current_upper_limit_pool_observation_in(
            database,
            trading_date,
            Ok(available_limit_pool(limit_records(1, batch_id), batch_id)),
        )
        .expect("TEST_CODE audited composition-failure pool");
        let composition_calls = Cell::new(0);

        let result = assemble_limit_up_observation(
            trading_date,
            limit_pool,
            |requested_codes| {
                let requested_codes = requested_codes.to_vec();
                let acquisition =
                    crate::data_gateway::market_capabilities::security_identities_observation_in(
                        database,
                        &requested_codes,
                        Ok(identity_batch_for_codes(
                            &requested_codes,
                            ProviderId::Tencent,
                            "TEST_CODE_composition_failure_names",
                        )),
                    )?;
                Ok(vec![LimitUpNameShardObservation::new(
                    requested_codes,
                    acquisition,
                )])
            },
            |date, pool_evidence, name_evidence, record_count| {
                composition_calls.set(composition_calls.get() + 1);
                let mut connection = database.get_conn().expect("TEST_CODE audit connection");
                diesel::sql_query(
                    "CREATE TRIGGER TEST_CODE_reject_limit_up_composition \
                     BEFORE INSERT ON data_acquisition_audit \
                     WHEN NEW.capability = 'BR-213-UpperLimitProjection' \
                     BEGIN SELECT RAISE(ABORT, 'TEST_CODE reject composition'); END",
                )
                .execute(&mut connection)
                .expect("TEST_CODE composition rejection trigger");
                drop(connection);
                crate::data_gateway::review::audit_limit_up_projection_in(
                    Some(database),
                    date,
                    pool_evidence,
                    name_evidence,
                    record_count,
                )
                .map_err(anyhow::Error::from)
            },
        );

        assert!(
            result.is_err(),
            "append failure must not return an observation"
        );
        assert_eq!(composition_calls.get(), 1);
        let rows = audit_rows(database);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].capability, "BR-213-UpperLimitPool");
        assert_eq!(rows[1].capability, "SecurityIdentity");
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
            |_| -> Result<Vec<GatewayBatch<MarketSecurityIdentity>>> {
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
            |_| Ok(identity_batches(&[("TEST_CODE_600001", "TEST_CODE Name")])),
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
        assert_eq!(name_evidence.len(), 1);
        assert_eq!(name_evidence[0].batch_id, "TEST_CODE_identity_batch");
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
            assert!(compose_limit_up_batch(pool(), |_| Ok(identity_batches(&rows))).is_err());
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
            Ok(identity_batches(&[("TEST_CODE_600001", "TEST_CODE Name")]))
        })
        .expect_err("conflicting record observed_at must reject the projection");

        assert!(error.to_string().contains("record evidence mismatch"));
    }
}
