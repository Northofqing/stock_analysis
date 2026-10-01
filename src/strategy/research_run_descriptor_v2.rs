//! Content identity for an already executed modeled research portfolio.
//!
//! Historical bars here are sealed Gateway observations, not exact persisted
//! replay, full-window coverage, PIT membership, or qualified instruments.
//! This descriptor cannot authorize a published v2 backtest or a live trade.

use std::collections::{BTreeMap, BTreeSet};

use chrono::NaiveDate;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::data_gateway::historical_bars::ObservedDailyBarsCapture;
use crate::data_gateway::{BenchmarkRange, BenchmarkRequest, VerifiedBenchmarkSnapshot};
use crate::market_domain::{AssetClass, Exchange, InstrumentId};
use crate::performance::fee_evidence::{
    shanghai_execution_date, FeeCoverageRequirement, FeeListingSegment, FillSide, StampTaxBracketV2,
};

use super::research_portfolio_v2::ResearchPortfolioV2;

const SCHEMA: &str = "observed-research-run-descriptor-v2";
const DOMAIN: &[u8] = b"stock_analysis.m4.observed-research-run-descriptor/v2\0";

/// Explicit, caller-declared strategy and code identity. The Git SHA shape is
/// checked, but this pure builder cannot attest which binary executed it.
pub struct ResearchStrategyDeclarationV2<'a> {
    pub strategy_id: &'a str,
    pub strategy_version: &'a str,
    pub config_canonical: &'a [u8],
    pub declared_git_commit: &'a str,
}

/// The complete requested symbol set must have one sealed observation each.
/// `benchmark` must come from `BenchmarkReader::read_verified_exact`.
/// `from/to` bound the evaluation and modeled-execution window, not a complete
/// historical-bars acquisition window. Earlier warmup bars may be observed;
/// gaps inside the window remain unqualified and later bars are rejected.
pub struct ObservedResearchRunRequestV2<'a> {
    pub strategy: ResearchStrategyDeclarationV2<'a>,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub requested_instruments: &'a [InstrumentId],
    pub observed_bars: &'a [ObservedDailyBarsCapture],
    pub expected_benchmark_request: &'a BenchmarkRequest,
    pub benchmark: &'a VerifiedBenchmarkSnapshot,
}

/// A deterministic observation identity, deliberately distinct from a
/// source-qualified or publishable backtest run identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedResearchRunDescriptorV2 {
    observed_run_id: String,
    canonical_bytes: Vec<u8>,
}

impl ObservedResearchRunDescriptorV2 {
    pub fn observed_run_id(&self) -> &str {
        &self.observed_run_id
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResearchRunDescriptorV2Error {
    #[error("strategy or version is not an explicit ASCII token")]
    InvalidStrategyIdentity,
    #[error("declared Git commit must be one full lowercase SHA-1")]
    InvalidDeclaredCodeRevision,
    #[error("strategy config must be a nonempty canonical JSON object")]
    InvalidStrategyConfig,
    #[error("research window is reversed")]
    InvalidWindow,
    #[error("complete trading-cost coverage is unavailable")]
    UnsupportedFeeCoverage,
    #[error("requested research universe is empty")]
    EmptyUniverse,
    #[error("requested instrument is outside modeled Shanghai equity scope: {0}")]
    UnsupportedInstrument(String),
    #[error("requested instrument identity is duplicated or ambiguous: {0}")]
    DuplicateInstrument(String),
    #[error("observed bars contain an unexpected instrument: {0}")]
    UnexpectedObservation(String),
    #[error("observed bars duplicate an instrument: {0}")]
    DuplicateObservation(String),
    #[error("observed bars are missing for requested instrument: {0}")]
    MissingObservation(String),
    #[error("observed bars have invalid identity or no in-window record: {0}")]
    InvalidObservation(String),
    #[error("exact verified benchmark does not match the expected daily request/window")]
    BenchmarkMismatch,
    #[error("portfolio effect has no matching requested instrument: {0}")]
    EffectOutsideUniverse(i64),
    #[error("portfolio effect is outside the research window: {0}")]
    EffectOutsideWindow(i64),
    #[error("portfolio effect differs from its explicit fee policy: {0}")]
    EffectPolicyMismatch(i64),
    #[error("canonical descriptor serialization failed")]
    Serialization,
}

#[derive(Serialize)]
struct StrategyWire<'a> {
    strategy_id: &'a str,
    strategy_version: &'a str,
    config: &'a serde_json::Value,
    declared_git_commit: &'a str,
}

#[derive(Serialize)]
struct ObservedBarsWire<'a> {
    instrument: &'a InstrumentId,
    requested_days: usize,
    projection_hash: &'a str,
}

#[derive(Serialize)]
struct HoldingWire<'a> {
    instrument: &'a InstrumentId,
    quantity: u64,
}

#[derive(Serialize)]
struct EffectWire<'a> {
    fill_id: i64,
    instrument: &'a InstrumentId,
    assumed_fee_segment: &'static str,
    side: &'static str,
    assumed_executed_at_utc: String,
    executed_price_micro_cny: i64,
    quantity: u64,
    notional_micro_cny: i64,
    fee_policy_instance_id: &'a str,
    fee_trade_date: NaiveDate,
    fee_stamp_tax_bracket: &'static str,
    fee_commission_micro_cny: i64,
    fee_stamp_tax_micro_cny: i64,
    fee_total_micro_cny: i64,
    cash_delta_micro_cny: i64,
    post_cash_micro_cny: i64,
    post_held_quantity: u64,
}

#[derive(Serialize)]
struct DescriptorWire<'a> {
    schema: &'static str,
    input_assurance: &'static str,
    strategy: StrategyWire<'a>,
    from: NaiveDate,
    to: NaiveDate,
    requested_instruments: Vec<&'a InstrumentId>,
    observed_bars: Vec<ObservedBarsWire<'a>>,
    benchmark_request: &'a BenchmarkRequest,
    benchmark_manifest_hash: &'a str,
    fee_policy_instance_id: String,
    fee_policy_canonical: String,
    required_fee_coverage: &'static str,
    initial_cash_micro_cny: i64,
    final_cash_micro_cny: i64,
    final_holdings: Vec<HoldingWire<'a>>,
    ordered_effects: Vec<EffectWire<'a>>,
}

/// Build a fixed ID from the portfolio owner's actual ordered effects and
/// explicit source references. No caller-supplied fill list can replace those
/// effects. Data observation IDs remain unverified for exact historical replay.
pub fn build_observed_research_run_descriptor_v2(
    portfolio: &ResearchPortfolioV2,
    request: &ObservedResearchRunRequestV2<'_>,
) -> Result<ObservedResearchRunDescriptorV2, ResearchRunDescriptorV2Error> {
    let strategy = &request.strategy;
    if !valid_token(strategy.strategy_id) || !valid_token(strategy.strategy_version) {
        return Err(ResearchRunDescriptorV2Error::InvalidStrategyIdentity);
    }
    if strategy.declared_git_commit.len() != 40
        || !valid_lower_hex(strategy.declared_git_commit)
        || strategy
            .declared_git_commit
            .bytes()
            .all(|byte| byte == b'0')
    {
        return Err(ResearchRunDescriptorV2Error::InvalidDeclaredCodeRevision);
    }
    let config: serde_json::Value = serde_json::from_slice(strategy.config_canonical)
        .map_err(|_| ResearchRunDescriptorV2Error::InvalidStrategyConfig)?;
    if !config.as_object().is_some_and(|object| !object.is_empty())
        || serde_json::to_vec(&config).map_err(|_| ResearchRunDescriptorV2Error::Serialization)?
            != strategy.config_canonical
    {
        return Err(ResearchRunDescriptorV2Error::InvalidStrategyConfig);
    }
    if request.from > request.to {
        return Err(ResearchRunDescriptorV2Error::InvalidWindow);
    }
    if portfolio.required_coverage() != FeeCoverageRequirement::ModeledComponentsOnly {
        return Err(ResearchRunDescriptorV2Error::UnsupportedFeeCoverage);
    }
    let benchmark = request.benchmark;
    if benchmark.request() != request.expected_benchmark_request
        || !matches!(
            &request.expected_benchmark_request.range,
            BenchmarkRange::Daily { from, to }
                if *from == request.from && *to == request.to
        )
        || benchmark.manifest_hash() != benchmark.snapshot().manifest.manifest_hash
        || benchmark.manifest_hash().len() != 64
        || !valid_lower_hex(benchmark.manifest_hash())
    {
        return Err(ResearchRunDescriptorV2Error::BenchmarkMismatch);
    }

    let mut universe = BTreeMap::new();
    for instrument in request.requested_instruments {
        let code = instrument.code();
        if instrument.exchange() != Exchange::Shanghai
            || instrument.asset_class() != AssetClass::Equity
        {
            return Err(ResearchRunDescriptorV2Error::UnsupportedInstrument(
                code.to_owned(),
            ));
        }
        if universe.insert(code, instrument).is_some() {
            return Err(ResearchRunDescriptorV2Error::DuplicateInstrument(
                code.to_owned(),
            ));
        }
    }
    if universe.is_empty() {
        return Err(ResearchRunDescriptorV2Error::EmptyUniverse);
    }
    let mut observations = BTreeMap::new();
    for capture in request.observed_bars {
        let code = capture.target_code();
        if !universe.contains_key(code) {
            return Err(ResearchRunDescriptorV2Error::UnexpectedObservation(
                code.to_owned(),
            ));
        }
        if capture.requested_days() == 0
            || capture.bars().is_empty()
            || capture.projection_hash().len() != 64
            || !valid_lower_hex(capture.projection_hash())
            || !capture
                .bars()
                .iter()
                .any(|bar| request.from <= bar.date() && bar.date() <= request.to)
            || capture.bars().iter().any(|bar| bar.date() > request.to)
        {
            return Err(ResearchRunDescriptorV2Error::InvalidObservation(
                code.to_owned(),
            ));
        }
        if observations.insert(code, capture).is_some() {
            return Err(ResearchRunDescriptorV2Error::DuplicateObservation(
                code.to_owned(),
            ));
        }
    }
    for code in universe.keys() {
        if !observations.contains_key(code) {
            return Err(ResearchRunDescriptorV2Error::MissingObservation(
                (*code).to_owned(),
            ));
        }
    }

    let policy = portfolio.policy();
    let policy_id = policy.instance_id();
    let mut seen_fill_ids = BTreeSet::new();
    let mut effects = Vec::with_capacity(portfolio.effects().len());
    for effect in portfolio.effects() {
        let input = &effect.input;
        let id = input.fill_id;
        if !universe
            .get(input.instrument_id.code())
            .is_some_and(|instrument| **instrument == input.instrument_id)
            || !seen_fill_ids.insert(id)
        {
            return Err(ResearchRunDescriptorV2Error::EffectOutsideUniverse(id));
        }
        let trade_date = shanghai_execution_date(input.assumed_executed_at_utc);
        if trade_date < request.from || trade_date > request.to {
            return Err(ResearchRunDescriptorV2Error::EffectOutsideWindow(id));
        }
        if effect.fee.policy_instance_id != policy_id
            || effect.fee.scope != policy.scope()
            || effect.fee.coverage != policy.coverage()
            || effect.fee.trade_date != trade_date
            || effect.fee.side != input.side
            || effect.fee.notional_micro_cny != effect.notional_micro_cny
            || effect.fee.total_micro_cny
                != effect
                    .fee
                    .commission_micro_cny
                    .checked_add(effect.fee.stamp_tax_micro_cny)
                    .ok_or(ResearchRunDescriptorV2Error::EffectPolicyMismatch(id))?
        {
            return Err(ResearchRunDescriptorV2Error::EffectPolicyMismatch(id));
        }
        effects.push(EffectWire {
            fill_id: id,
            instrument: &input.instrument_id,
            assumed_fee_segment: fee_segment(input.assumed_instrument.segment()),
            side: side(input.side),
            assumed_executed_at_utc: input.assumed_executed_at_utc.to_rfc3339(),
            executed_price_micro_cny: input.executed_price_micro_cny,
            quantity: input.quantity,
            notional_micro_cny: effect.notional_micro_cny,
            fee_policy_instance_id: &effect.fee.policy_instance_id,
            fee_trade_date: effect.fee.trade_date,
            fee_stamp_tax_bracket: tax_bracket(effect.fee.stamp_tax_bracket),
            fee_commission_micro_cny: effect.fee.commission_micro_cny,
            fee_stamp_tax_micro_cny: effect.fee.stamp_tax_micro_cny,
            fee_total_micro_cny: effect.fee.total_micro_cny,
            cash_delta_micro_cny: effect.cash_delta_micro_cny,
            post_cash_micro_cny: effect.poststate.cash_micro_cny,
            post_held_quantity: effect.poststate.held_quantity,
        });
    }
    let descriptor = DescriptorWire {
        schema: SCHEMA,
        input_assurance: "observed_bars_only_no_exact_persisted_read_or_pit",
        strategy: StrategyWire {
            strategy_id: strategy.strategy_id,
            strategy_version: strategy.strategy_version,
            config: &config,
            declared_git_commit: strategy.declared_git_commit,
        },
        from: request.from,
        to: request.to,
        requested_instruments: universe.values().copied().collect(),
        observed_bars: observations
            .into_iter()
            .map(|(code, capture)| ObservedBarsWire {
                instrument: universe[code],
                requested_days: capture.requested_days(),
                projection_hash: capture.projection_hash(),
            })
            .collect(),
        benchmark_request: request.expected_benchmark_request,
        benchmark_manifest_hash: benchmark.manifest_hash(),
        fee_policy_instance_id: policy_id,
        fee_policy_canonical: String::from_utf8(policy.canonical_bytes())
            .map_err(|_| ResearchRunDescriptorV2Error::Serialization)?,
        required_fee_coverage: "modeled_components_only",
        initial_cash_micro_cny: portfolio.initial_cash_micro_cny(),
        final_cash_micro_cny: portfolio.cash_micro_cny(),
        final_holdings: portfolio
            .holdings()
            .map(|holding| HoldingWire {
                instrument: &holding.instrument_id,
                quantity: holding.quantity,
            })
            .collect(),
        ordered_effects: effects,
    };
    let canonical_bytes =
        serde_json::to_vec(&descriptor).map_err(|_| ResearchRunDescriptorV2Error::Serialization)?;
    let mut digest = Sha256::new();
    digest.update(DOMAIN);
    digest.update((canonical_bytes.len() as u64).to_be_bytes());
    digest.update(&canonical_bytes);
    Ok(ObservedResearchRunDescriptorV2 {
        observed_run_id: format!("m4-observed-run-v2:sha256:{:x}", digest.finalize()),
        canonical_bytes,
    })
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

fn valid_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn fee_segment(value: FeeListingSegment) -> &'static str {
    match value {
        FeeListingSegment::ShanghaiMainA => "ShanghaiMainA",
        FeeListingSegment::ShanghaiStarA => "ShanghaiStarA",
        FeeListingSegment::Other => "Other",
    }
}

fn side(value: FillSide) -> &'static str {
    match value {
        FillSide::Buy => "buy",
        FillSide::Sell => "sell",
    }
}

fn tax_bracket(value: StampTaxBracketV2) -> &'static str {
    match value {
        StampTaxBracketV2::SellerOnePerThousand => "seller_one_per_thousand",
        StampTaxBracketV2::SellerHalfPerThousand => "seller_half_per_thousand",
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate, Utc};

    use super::*;
    use crate::data_gateway::historical_bars::AdmittedDailyBars;
    use crate::data_gateway::review::{AuditedBenchmarkBatch, BatchEvidence, GatewayBatch};
    use crate::data_gateway::{BenchmarkBar, BenchmarkBarTime, BenchmarkCapture, BenchmarkReader};
    use crate::data_provider::{AdjustType, KlineData};
    use crate::database::data_acquisition_audit::DataAcquisitionAuditRecord;
    use crate::database::DatabaseManager;
    use crate::market_domain::ProviderId;
    use crate::performance::fee_evidence::{
        AShareFeePolicyV2, FeeCoverage, FeeListingSegment, FeeMarket, FeeRate, FeeSecurityKind,
        QualifiedInstrument,
    };
    use crate::strategy::research_fill_v2::ResearchFillInputV2;

    const GIT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const GIT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 5).unwrap()
    }

    fn instrument(code: &str) -> InstrumentId {
        InstrumentId::new(Exchange::Shanghai, code, AssetClass::Equity).unwrap()
    }

    fn policy(commission_numerator: i64) -> AShareFeePolicyV2 {
        AShareFeePolicyV2::new(
            QualifiedInstrument::new(
                FeeMarket::Shanghai,
                FeeSecurityKind::AShareStock,
                FeeListingSegment::ShanghaiMainA,
            )
            .unwrap(),
            FeeRate::new(commission_numerator, 10_000).unwrap(),
            5_000_000,
            FeeCoverage::initial_model(),
            "TEST_CODE_m4_observed_run",
        )
        .unwrap()
    }

    fn portfolio_with_fill(
        commission_numerator: i64,
        fill_code: &str,
        quantity: u64,
    ) -> ResearchPortfolioV2 {
        let policy = policy(commission_numerator);
        let mut portfolio = ResearchPortfolioV2::new(
            policy.clone(),
            2_000_000_000,
            FeeCoverageRequirement::ModeledComponentsOnly,
        )
        .unwrap();
        portfolio
            .apply_fill(ResearchFillInputV2 {
                fill_id: 11,
                instrument_id: instrument(fill_code),
                assumed_instrument: policy.scope(),
                side: FillSide::Buy,
                assumed_executed_at_utc: DateTime::parse_from_rfc3339("2026-01-05T02:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                executed_price_micro_cny: 10_000_000,
                quantity,
            })
            .unwrap();
        portfolio
    }

    fn observed_record(bar_date: NaiveDate) -> KlineData {
        KlineData {
            date: bar_date,
            open: 10.0,
            high: 10.5,
            low: 9.5,
            close: 10.0,
            volume: 1000.0,
            amount: 10_000.0,
            pct_chg: 0.0,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            adjust: AdjustType::None,
        }
    }

    fn observed_dates(code: &str, batch_id: &str, dates: &[NaiveDate]) -> ObservedDailyBarsCapture {
        AdmittedDailyBars::from_test_fixture_with_days(
            code,
            dates.len(),
            dates.iter().copied().map(observed_record).collect(),
            BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_historical_bars".to_owned(),
                source_at: Some("2026-01-05T15:00:00+08:00".to_owned()),
                observed_at: "2026-01-05T16:00:00+08:00".to_owned(),
                batch_id: batch_id.to_owned(),
            },
        )
        .unwrap()
        .into_observed_projection()
        .unwrap()
    }

    fn observed(code: &str, batch_id: &str, bar_date: NaiveDate) -> ObservedDailyBarsCapture {
        observed_dates(code, batch_id, &[bar_date])
    }

    fn exact_benchmark() -> (
        tempfile::TempDir,
        BenchmarkRequest,
        VerifiedBenchmarkSnapshot,
    ) {
        exact_benchmark_dates(&[day()])
    }

    fn exact_benchmark_dates(
        dates: &[NaiveDate],
    ) -> (
        tempfile::TempDir,
        BenchmarkRequest,
        VerifiedBenchmarkSnapshot,
    ) {
        let from = *dates.first().unwrap();
        let to = *dates.last().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseManager::open_isolated_for_test(
            dir.path().join("TEST_CODE_m4_observed_run.db"),
        )
        .unwrap();
        let request = BenchmarkRequest {
            instrument: "TEST_CODE_sh000300".to_owned(),
            range: BenchmarkRange::Daily { from, to },
        };
        let source_at = format!("{to}T15:00:00+08:00");
        let observed_at = format!("{to}T16:00:00+08:00");
        let evidence = BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_benchmark_source".to_owned(),
            source_at: Some(source_at),
            observed_at,
            batch_id: "TEST_CODE_benchmark_batch".to_owned(),
        };
        let request_hash = request.canonical_request_hash();
        let provider = serde_json::to_value(evidence.provider)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        let receipt = db
            .record_data_acquisition(&DataAcquisitionAuditRecord {
                capability: "BenchmarkBars",
                provider: &provider,
                source: &evidence.source,
                request_hash: &request_hash,
                source_at: evidence.source_at.as_deref(),
                observed_at: &evidence.observed_at,
                batch_id: Some(&evidence.batch_id),
                outcome: "available",
                request_count: 1,
                accepted_count: dates.len() as i64,
                rejected_count: 0,
                reason_code: "accepted",
                retryable: false,
            })
            .unwrap();
        let audited = AuditedBenchmarkBatch {
            batch: GatewayBatch::Available {
                records: dates
                    .iter()
                    .enumerate()
                    .map(|(index, date)| BenchmarkBar {
                        at: BenchmarkBarTime::Daily(*date),
                        open: 100.0 + index as f64,
                        high: 102.0 + index as f64,
                        low: 99.0 + index as f64,
                        close: 101.0 + index as f64,
                        volume: Some(1000.0),
                        amount: None,
                    })
                    .collect(),
                evidence,
            },
            receipt,
            request_hash,
        };
        let capture = BenchmarkCapture::new(&db);
        let manifest = capture
            .preview_audited_for_test(request.clone(), audited)
            .and_then(|preview| capture.commit(preview))
            .unwrap();
        let verified = BenchmarkReader::new(&db)
            .read_verified_exact(&manifest.manifest_hash, &request)
            .unwrap();
        (dir, request, verified)
    }

    fn request<'a>(
        config_canonical: &'a [u8],
        git_commit: &'a str,
        instruments: &'a [InstrumentId],
        bars: &'a [ObservedDailyBarsCapture],
        benchmark_request: &'a BenchmarkRequest,
        benchmark: &'a VerifiedBenchmarkSnapshot,
    ) -> ObservedResearchRunRequestV2<'a> {
        ObservedResearchRunRequestV2 {
            strategy: ResearchStrategyDeclarationV2 {
                strategy_id: "TEST_CODE_strategy",
                strategy_version: "v2.1",
                config_canonical,
                declared_git_commit: git_commit,
            },
            from: day(),
            to: day(),
            requested_instruments: instruments,
            observed_bars: bars,
            expected_benchmark_request: benchmark_request,
            benchmark,
        }
    }

    #[test]
    fn identity_binds_executed_effect_policy_versions_and_exact_source_refs() {
        let (_dir, benchmark_request, benchmark) = exact_benchmark();
        let instruments = vec![instrument("TEST_CODE_600000")];
        let bars = vec![observed("TEST_CODE_600000", "TEST_CODE_batch_a", day())];
        let portfolio = portfolio_with_fill(3, "TEST_CODE_600000", 100);
        let base_request = request(
            br#"{"lookback":20}"#,
            GIT_A,
            &instruments,
            &bars,
            &benchmark_request,
            &benchmark,
        );
        let first = build_observed_research_run_descriptor_v2(&portfolio, &base_request).unwrap();
        assert_eq!(
            first,
            build_observed_research_run_descriptor_v2(&portfolio, &base_request).unwrap()
        );
        assert!(first
            .observed_run_id()
            .starts_with("m4-observed-run-v2:sha256:"));
        let canonical: serde_json::Value = serde_json::from_slice(first.canonical_bytes()).unwrap();
        assert_eq!(
            canonical["input_assurance"],
            "observed_bars_only_no_exact_persisted_read_or_pit"
        );
        assert_eq!(canonical["ordered_effects"].as_array().unwrap().len(), 1);
        assert_eq!(
            canonical["benchmark_manifest_hash"],
            benchmark.manifest_hash()
        );
        assert_eq!(
            canonical["observed_bars"][0]["projection_hash"],
            bars[0].projection_hash()
        );

        let changed_policy = portfolio_with_fill(4, "TEST_CODE_600000", 100);
        assert_ne!(
            first.observed_run_id(),
            build_observed_research_run_descriptor_v2(&changed_policy, &base_request)
                .unwrap()
                .observed_run_id()
        );
        let changed_effect = portfolio_with_fill(3, "TEST_CODE_600000", 101);
        assert_ne!(
            first.observed_run_id(),
            build_observed_research_run_descriptor_v2(&changed_effect, &base_request)
                .unwrap()
                .observed_run_id()
        );
        let changed_config = request(
            br#"{"lookback":21}"#,
            GIT_A,
            &instruments,
            &bars,
            &benchmark_request,
            &benchmark,
        );
        assert_ne!(
            first.observed_run_id(),
            build_observed_research_run_descriptor_v2(&portfolio, &changed_config)
                .unwrap()
                .observed_run_id()
        );
        let changed_code = request(
            br#"{"lookback":20}"#,
            GIT_B,
            &instruments,
            &bars,
            &benchmark_request,
            &benchmark,
        );
        assert_ne!(
            first.observed_run_id(),
            build_observed_research_run_descriptor_v2(&portfolio, &changed_code)
                .unwrap()
                .observed_run_id()
        );
        let changed_bars = vec![observed("TEST_CODE_600000", "TEST_CODE_batch_b", day())];
        let changed_data = request(
            br#"{"lookback":20}"#,
            GIT_A,
            &instruments,
            &changed_bars,
            &benchmark_request,
            &benchmark,
        );
        assert_ne!(
            first.observed_run_id(),
            build_observed_research_run_descriptor_v2(&portfolio, &changed_data)
                .unwrap()
                .observed_run_id()
        );
    }

    #[test]
    fn cross_day_lookback_is_observed_only_and_future_bars_are_rejected() {
        let jan_2 = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        let jan_6 = NaiveDate::from_ymd_opt(2026, 1, 6).unwrap();
        let jan_7 = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        let jan_8 = NaiveDate::from_ymd_opt(2026, 1, 8).unwrap();
        let (_dir, benchmark_request, benchmark) = exact_benchmark_dates(&[day(), jan_6, jan_7]);
        let instruments = vec![instrument("TEST_CODE_600000")];
        let portfolio = portfolio_with_fill(3, "TEST_CODE_600000", 100);

        // Jan 2 is explicit lookback; Jan 6 is absent from the observed equity
        // batch even though the exact benchmark has that day. This is still
        // only an observation identity, never full-window bar qualification.
        let sparse = vec![observed_dates(
            "TEST_CODE_600000",
            "TEST_CODE_sparse_batch",
            &[jan_2, day(), jan_7],
        )];
        let mut sparse_request = request(
            br#"{"lookback":20}"#,
            GIT_A,
            &instruments,
            &sparse,
            &benchmark_request,
            &benchmark,
        );
        sparse_request.to = jan_7;
        let sparse_id = build_observed_research_run_descriptor_v2(&portfolio, &sparse_request)
            .unwrap()
            .observed_run_id()
            .to_owned();
        let complete = vec![observed_dates(
            "TEST_CODE_600000",
            "TEST_CODE_complete_batch",
            &[jan_2, day(), jan_6, jan_7],
        )];
        let mut complete_request = request(
            br#"{"lookback":20}"#,
            GIT_A,
            &instruments,
            &complete,
            &benchmark_request,
            &benchmark,
        );
        complete_request.to = jan_7;
        assert_ne!(
            sparse_id,
            build_observed_research_run_descriptor_v2(&portfolio, &complete_request)
                .unwrap()
                .observed_run_id()
        );

        let future = vec![observed_dates(
            "TEST_CODE_600000",
            "TEST_CODE_future_batch",
            &[jan_2, day(), jan_8],
        )];
        let mut future_request = request(
            br#"{"lookback":20}"#,
            GIT_A,
            &instruments,
            &future,
            &benchmark_request,
            &benchmark,
        );
        future_request.to = jan_7;
        assert_eq!(
            build_observed_research_run_descriptor_v2(&portfolio, &future_request),
            Err(ResearchRunDescriptorV2Error::InvalidObservation(
                "TEST_CODE_600000".to_owned()
            ))
        );
    }

    #[test]
    fn rejects_missing_ambiguous_or_mismatched_inputs_without_creating_an_identity() {
        let (_dir, benchmark_request, benchmark) = exact_benchmark();
        let portfolio = portfolio_with_fill(3, "TEST_CODE_600000", 100);
        let instruments = vec![instrument("TEST_CODE_600000")];
        let bars = vec![observed("TEST_CODE_600000", "TEST_CODE_batch_a", day())];
        let make = |instruments: &[InstrumentId], bars: &[ObservedDailyBarsCapture]| {
            build_observed_research_run_descriptor_v2(
                &portfolio,
                &request(
                    br#"{"lookback":20}"#,
                    GIT_A,
                    instruments,
                    bars,
                    &benchmark_request,
                    &benchmark,
                ),
            )
        };
        assert_eq!(
            make(&instruments, &[]),
            Err(ResearchRunDescriptorV2Error::MissingObservation(
                "TEST_CODE_600000".to_owned()
            ))
        );
        let duplicate = vec![
            instrument("TEST_CODE_600000"),
            instrument("TEST_CODE_600000"),
        ];
        assert_eq!(
            make(&duplicate, &bars),
            Err(ResearchRunDescriptorV2Error::DuplicateInstrument(
                "TEST_CODE_600000".to_owned()
            ))
        );
        let duplicate_bars = vec![
            observed("TEST_CODE_600000", "TEST_CODE_batch_a", day()),
            observed("TEST_CODE_600000", "TEST_CODE_batch_b", day()),
        ];
        assert_eq!(
            make(&instruments, &duplicate_bars),
            Err(ResearchRunDescriptorV2Error::DuplicateObservation(
                "TEST_CODE_600000".to_owned()
            ))
        );
        let wrong_code = vec![observed("TEST_CODE_600001", "TEST_CODE_batch_b", day())];
        assert_eq!(
            make(&instruments, &wrong_code),
            Err(ResearchRunDescriptorV2Error::UnexpectedObservation(
                "TEST_CODE_600001".to_owned()
            ))
        );
        let old_bars = vec![observed(
            "TEST_CODE_600000",
            "TEST_CODE_batch_old",
            NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
        )];
        assert_eq!(
            make(&instruments, &old_bars),
            Err(ResearchRunDescriptorV2Error::InvalidObservation(
                "TEST_CODE_600000".to_owned()
            ))
        );
        let mismatched_benchmark = BenchmarkRequest {
            instrument: "TEST_CODE_wrong_benchmark".to_owned(),
            range: benchmark_request.range.clone(),
        };
        assert_eq!(
            build_observed_research_run_descriptor_v2(
                &portfolio,
                &request(
                    br#"{"lookback":20}"#,
                    GIT_A,
                    &instruments,
                    &bars,
                    &mismatched_benchmark,
                    &benchmark,
                )
            ),
            Err(ResearchRunDescriptorV2Error::BenchmarkMismatch)
        );
        assert_eq!(
            build_observed_research_run_descriptor_v2(
                &portfolio,
                &request(
                    br#"{ "lookback":20 }"#,
                    GIT_A,
                    &instruments,
                    &bars,
                    &benchmark_request,
                    &benchmark,
                )
            ),
            Err(ResearchRunDescriptorV2Error::InvalidStrategyConfig)
        );
        let outside_portfolio = portfolio_with_fill(3, "TEST_CODE_600001", 100);
        assert_eq!(
            build_observed_research_run_descriptor_v2(
                &outside_portfolio,
                &request(
                    br#"{"lookback":20}"#,
                    GIT_A,
                    &instruments,
                    &bars,
                    &benchmark_request,
                    &benchmark,
                )
            ),
            Err(ResearchRunDescriptorV2Error::EffectOutsideUniverse(11))
        );
        let empty_incomplete = ResearchPortfolioV2::new(
            policy(3),
            2_000_000_000,
            FeeCoverageRequirement::CompleteTradingCost,
        )
        .unwrap();
        assert_eq!(
            build_observed_research_run_descriptor_v2(
                &empty_incomplete,
                &request(
                    br#"{"lookback":20}"#,
                    GIT_A,
                    &instruments,
                    &bars,
                    &benchmark_request,
                    &benchmark,
                )
            ),
            Err(ResearchRunDescriptorV2Error::UnsupportedFeeCoverage)
        );
    }
}
