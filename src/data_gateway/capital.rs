//! BR-164 typed capital-data boundary.
//!
//! The gateway exposes only capabilities admitted from the remote contract:
//!
//! - Eastmoney instrument fund-flow series (`Minute1` and `Day1`);
//! - Eastmoney source-limited post-close provider Top-N rankings;
//! - official HKEX northbound daily statistics.
//!
//! Every admitted batch is complete, identity-consistent, ordered, fresh for
//! its contract, and carries provider/batch/source/observation evidence.
//! Missing monetary components are rejected instead of being relabelled as
//! zero. This repository does not construct provider clients.

use crate::market_domain::{
    FiniteNumber, FlowInterval, InstrumentId, IsoDate, MarketRankingKind, MarketRankingUnit,
    NonEmptyText, NorthboundChannel, PositiveU32, ProviderId,
};
use chrono::NaiveDate;

use super::review::{acquisition_request_hash, audit_gateway_result, GatewayBatch, GatewayError};

const FUND_FLOW_CAPABILITY: &str = "CapitalInstrumentFundFlow";
const PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY: &str = "CapitalProviderTopNVolumeRatio";
const PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY: &str = "CapitalProviderTopNMainNetInflow";
const NORTHBOUND_CAPABILITY: &str = "CapitalNorthboundDaily";
const EASTMONEY_SOURCE: &str = "eastmoney-web";
const HKEX_SOURCE: &str = "hkex-official";
const PROVIDER_TOP_N_LIMIT: u32 = 20;
// Remote ProviderTopNRankings contract 的固定 A-share filter identity。客户端
// 构造 request evidence 时必须使用同一 filter，才能保证 canonical request_hash 一致。
const PROVIDER_TOP_N_A_SHARE_FILTER: &str =
    "m:0+t:6+f:!2,m:0+t:13+f:!2,m:0+t:80+f:!2,m:1+t:2+f:!2,m:1+t:23+f:!2,m:0+t:81+s:262144+f:!2";
const OBSERVATION_MAX_AGE_MILLIS: i64 = 30_000;
// Minute1 source timestamps have minute precision. Two minutes is the
// strictest useful gate that does not reject a just-published minute solely
// because its source omits seconds.
const MINUTE_FLOW_MAX_AGE_MILLIS: i64 = 120_000;
const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;

/// One complete instrument fund-flow observation in base CNY and percent.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentFundFlowFact {
    pub code: String,
    pub interval: FlowInterval,
    pub period_at: String,
    pub main_net: f64,
    pub main_ratio_percent: f64,
    pub super_large_net: f64,
    pub large_net: f64,
    pub medium_net: f64,
    pub small_net: f64,
}

/// One evidence-preserving row from an Eastmoney single-response Top-N page.
///
/// This source-limited fact is not a complete-market ranking and must not be
/// used to infer market breadth.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderTopNFact {
    pub metric: MarketRankingKind,
    pub source_order_ordinal: PositiveU32,
    pub instrument: InstrumentId,
    pub label: NonEmptyText,
    pub value: FiniteNumber,
    pub unit: MarketRankingUnit,
    pub trading_date: IsoDate,
    pub filter_identity: NonEmptyText,
    pub provider_declared_total: PositiveU32,
    pub inspected_row_count: PositiveU32,
}

/// Canonical request evidence retained for the R-09 delivery binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTopNRequestEvidence {
    pub metric: MarketRankingKind,
    pub trading_date: IsoDate,
    pub limit: PositiveU32,
    pub filter_identity: NonEmptyText,
    pub request_hash: String,
}

/// Atomic pair required by BR-192's R-09 report.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderTopNPair {
    pub volume_ratio_request: ProviderTopNRequestEvidence,
    pub volume_ratio: GatewayBatch<ProviderTopNFact>,
    pub main_net_inflow_request: ProviderTopNRequestEvidence,
    pub main_net_inflow: GatewayBatch<ProviderTopNFact>,
}

/// Preserves the official HKEX distinction between an amount and an
/// explicitly unavailable quota value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NorthboundQuotaFact {
    Amount(f64),
    Unavailable,
}

/// One official Top-10 northbound turnover entry.
#[derive(Debug, Clone, PartialEq)]
pub struct NorthboundTopTurnoverFact {
    pub rank: u32,
    pub code: String,
    pub name: String,
    pub total_turnover: f64,
}

/// One complete official HKEX northbound channel/day observation.
#[derive(Debug, Clone, PartialEq)]
pub struct NorthboundDailyFact {
    pub trading_date: NaiveDate,
    pub channel: NorthboundChannel,
    pub total_turnover: f64,
    pub total_trade_count: f64,
    pub quota_balance: NorthboundQuotaFact,
    pub etf_turnover: f64,
    pub top_turnover: Vec<NorthboundTopTurnoverFact>,
}

/// Evidence-preserving capital-data entry point.
#[derive(Debug, Clone, Copy, Default)]
pub struct CapitalDataGateway;

impl CapitalDataGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Fetches one complete Eastmoney instrument fund-flow series.
    ///
    /// Only the upstream-proved `Minute1` and `Day1` intervals are accepted.
    pub async fn instrument_fund_flow(
        &self,
        code: &str,
        interval: FlowInterval,
        limit: u32,
    ) -> Result<GatewayBatch<InstrumentFundFlowFact>, GatewayError> {
        let storage_code = code.to_owned();
        let canonical = format!("{storage_code}:{interval:?}:{limit}");
        let request_hash = acquisition_request_hash(FUND_FLOW_CAPABILITY, &canonical);
        // P4 M4b: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("FundFlowSeries") {
            Ok(bridge) => {
                let result = bridge
                    .fund_flow_series_async(&storage_code, interval, limit)
                    .await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Eastmoney);
                return audit_gateway_result(
                    FUND_FLOW_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    FUND_FLOW_CAPABILITY,
                    ProviderId::Eastmoney,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // P4 M5: no-feature 构建不携带 library transport, 无桥时显式失败
        // (fail-closed), 绝不静默回退。
    }

    /// Fetches BR-192's two source-limited Eastmoney Top-N pages atomically.
    ///
    /// The caller owns BR-198's typed R-09 date/window decision. Same-day
    /// requests are eligible only at or after 15:35; on a later calendar date,
    /// the concrete upstream route admits only rows whose provider settlement
    /// date exactly matches `trading_date`. Both metric acquisitions are
    /// audited independently, while this method returns no partial pair.
    pub async fn provider_top_n_pair(
        &self,
        trading_date: NaiveDate,
    ) -> Result<ProviderTopNPair, GatewayError> {
        // P4 M5: request evidence 本地构造 (无 transport 类型依赖), 与 library
        // transport 的 `provider_top_n_request_evidence` 保持同 canonical
        // request_hash (桥只换 transport 数据)。
        let volume_request_evidence = provider_top_n_a_share_evidence(
            MarketRankingKind::VolumeRatio,
            trading_date,
            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
        )?;
        let inflow_request_evidence = provider_top_n_a_share_evidence(
            MarketRankingKind::MainNetInflow,
            trading_date,
            PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
        )?;
        // 仅 library transport 路径需要独立的 hash 变量 (桥路径直接读
        // evidence.request_hash), no-feature 构建不产生未使用变量。

        // P4 M4b: gRPC 桥 (remote gRPC 时替换 transport; 双路 audit 留
        // 客户端, request evidence 是本地构造的 (桥只换 transport 数据)。
        match super::grpc_source::bridge_for("ProviderTopNRankings") {
            Ok(bridge) => {
                return match bridge.provider_top_n_pair_async(trading_date).await {
                    Ok((volume, inflow)) => {
                        let volume_audited = audit_gateway_result(
                            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
                            ProviderId::Eastmoney,
                            &volume_request_evidence.request_hash,
                            Ok(volume),
                        );
                        let inflow_audited = audit_gateway_result(
                            PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
                            ProviderId::Eastmoney,
                            &inflow_request_evidence.request_hash,
                            Ok(inflow),
                        );
                        match (volume_audited, inflow_audited) {
                            (Ok(volume), Ok(inflow)) => {
                                validate_provider_top_n_pair(&volume, &inflow, trading_date)?;
                                Ok(ProviderTopNPair {
                                    volume_ratio_request: volume_request_evidence,
                                    volume_ratio: volume,
                                    main_net_inflow_request: inflow_request_evidence,
                                    main_net_inflow: inflow,
                                })
                            }
                            (Err(error), _) | (_, Err(error)) => Err(error),
                        }
                    }
                    Err(error) => {
                        let volume_audited = audit_gateway_result::<ProviderTopNFact>(
                            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
                            ProviderId::Eastmoney,
                            &volume_request_evidence.request_hash,
                            Err(error.clone()),
                        );
                        let inflow_audited = audit_gateway_result::<ProviderTopNFact>(
                            PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
                            ProviderId::Eastmoney,
                            &inflow_request_evidence.request_hash,
                            Err(error),
                        );
                        match (volume_audited, inflow_audited) {
                            (Err(error), _) | (_, Err(error)) => Err(error),
                            (Ok(_), Ok(_)) => Err(GatewayError::unavailable(
                                PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
                                Some(ProviderId::Eastmoney),
                                true,
                                "头部排行 gRPC 桥失败且双路 audit 落库 (原始错误见上一条 audit 行)",
                            )),
                        }
                    }
                };
            }
            Err(error) => {
                let volume_audited = audit_gateway_result::<ProviderTopNFact>(
                    PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
                    ProviderId::Eastmoney,
                    &volume_request_evidence.request_hash,
                    Err(error.clone()),
                );
                let inflow_audited = audit_gateway_result::<ProviderTopNFact>(
                    PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
                    ProviderId::Eastmoney,
                    &inflow_request_evidence.request_hash,
                    Err(error),
                );
                return match (volume_audited, inflow_audited) {
                    (Err(error), _) | (_, Err(error)) => Err(error),
                    (Ok(_), Ok(_)) => Err(GatewayError::unavailable(
                        PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
                        Some(ProviderId::Eastmoney),
                        true,
                        "头部排行 gRPC 桥不可用且双路 audit 落库 (原始错误见上一条 audit 行)",
                    )),
                };
            }
        }
        // P4 M5: no-feature 构建不携带 library transport, 无桥时显式失败
        // (fail-closed), 绝不静默回退。
    }

    /// Fetches one official HKEX northbound channel/day statistic.
    pub async fn northbound_daily(
        &self,
        trading_date: NaiveDate,
        channel: NorthboundChannel,
    ) -> Result<GatewayBatch<NorthboundDailyFact>, GatewayError> {
        let canonical = format!("{trading_date}:{channel:?}");
        let request_hash = acquisition_request_hash(NORTHBOUND_CAPABILITY, &canonical);
        // P4 M4b: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("NorthboundDaily") {
            Ok(bridge) => {
                let result = bridge.northbound_daily_async(trading_date, channel).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Hkex);
                return audit_gateway_result(
                    NORTHBOUND_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    NORTHBOUND_CAPABILITY,
                    ProviderId::Hkex,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // P4 M5: no-feature 构建不携带 library transport, 无桥时显式失败
        // (fail-closed), 绝不静默回退。
    }
}

/// P4 M5: 桥模式本地构造 BR-192 A-share Top-N request evidence (无 transport
/// 类型依赖, magic_compat 即可)。canonical 格式与
/// `provider_top_n_request_evidence` 完全一致, 保证两模式 request_hash 相同。
fn provider_top_n_a_share_evidence(
    metric: MarketRankingKind,
    trading_date: NaiveDate,
    capability: &'static str,
) -> Result<ProviderTopNRequestEvidence, GatewayError> {
    let date = iso_date(trading_date, capability)?;
    let limit = PositiveU32::new(PROVIDER_TOP_N_LIMIT)
        .map_err(|error| GatewayError::invalid_request(capability, error.to_string()))?;
    let filter_identity = NonEmptyText::new(PROVIDER_TOP_N_A_SHARE_FILTER)
        .map_err(|error| GatewayError::invalid_request(capability, error.to_string()))?;
    let canonical = format!(
        "metric={:?};trading_date={};limit={};filter={}",
        metric,
        date.as_str(),
        limit.get(),
        filter_identity.as_str()
    );
    Ok(ProviderTopNRequestEvidence {
        metric,
        trading_date: date,
        limit,
        filter_identity,
        request_hash: acquisition_request_hash(capability, &canonical),
    })
}

fn validate_provider_top_n_pair(
    volume_ratio: &GatewayBatch<ProviderTopNFact>,
    main_net_inflow: &GatewayBatch<ProviderTopNFact>,
    trading_date: NaiveDate,
) -> Result<(), GatewayError> {
    let expected_date = iso_date(trading_date, PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY)?;
    validate_provider_top_n_side(
        volume_ratio,
        &MarketRankingKind::VolumeRatio,
        &MarketRankingUnit::Multiple,
        &expected_date,
        PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
    )?;
    validate_provider_top_n_side(
        main_net_inflow,
        &MarketRankingKind::MainNetInflow,
        &MarketRankingUnit::Yuan,
        &expected_date,
        PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
    )?;
    if volume_ratio.evidence().batch_id == main_net_inflow.evidence().batch_id {
        return Err(GatewayError::invalid_evidence(
            PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
            Some(main_net_inflow.evidence().provider),
            "atomic provider Top-N sides must retain distinct upstream batch identities",
        ));
    }
    Ok(())
}

fn validate_provider_top_n_side(
    batch: &GatewayBatch<ProviderTopNFact>,
    expected_metric: &MarketRankingKind,
    expected_unit: &MarketRankingUnit,
    expected_date: &IsoDate,
    capability: &'static str,
) -> Result<(), GatewayError> {
    if batch.is_verified_empty() || batch.records().is_empty() {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(ProviderId::Eastmoney),
            "atomic provider Top-N side is empty",
        ));
    }
    let evidence = batch.evidence();
    if evidence.provider != ProviderId::Eastmoney
        || evidence.source != EASTMONEY_SOURCE
        || evidence.source_at.is_some()
    {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(evidence.provider),
            "atomic provider Top-N side has inconsistent provider/source evidence",
        ));
    }
    for (index, record) in batch.records().iter().enumerate() {
        let expected_ordinal = u32::try_from(index + 1).map_err(|_| {
            GatewayError::invalid_evidence(
                capability,
                Some(ProviderId::Eastmoney),
                "provider Top-N ordinal overflow",
            )
        })?;
        if &record.metric != expected_metric
            || &record.unit != expected_unit
            || &record.trading_date != expected_date
            || record.source_order_ordinal.get() != expected_ordinal
        {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(ProviderId::Eastmoney),
                "atomic provider Top-N metric/unit/date/order mismatch",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod br240_transport_neutral_tests {
    use super::*;
    use crate::data_gateway::BatchEvidence;
    use crate::market_domain::{AssetClass, Exchange};

    fn evidence(batch_id: &str) -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: EASTMONEY_SOURCE.to_owned(),
            source_at: None,
            observed_at: "2026-07-24T15:36:00+08:00".to_owned(),
            batch_id: batch_id.to_owned(),
        }
    }

    fn record(
        metric: MarketRankingKind,
        unit: MarketRankingUnit,
        ordinal: u32,
    ) -> ProviderTopNFact {
        ProviderTopNFact {
            metric,
            source_order_ordinal: PositiveU32::new(ordinal).unwrap(),
            instrument: InstrumentId::new(
                Exchange::Shanghai,
                format!("TEST_CODE_600{ordinal:03}"),
                AssetClass::Equity,
            )
            .unwrap(),
            label: NonEmptyText::new(format!("TEST_CODE_TOP_N_{ordinal}")).unwrap(),
            value: FiniteNumber::new(f64::from(3 - ordinal)).unwrap(),
            unit,
            trading_date: IsoDate::new("2026-07-24").unwrap(),
            filter_identity: NonEmptyText::new(PROVIDER_TOP_N_A_SHARE_FILTER).unwrap(),
            provider_declared_total: PositiveU32::new(100).unwrap(),
            inspected_row_count: PositiveU32::new(2).unwrap(),
        }
    }

    fn side(
        metric: MarketRankingKind,
        unit: MarketRankingUnit,
        batch_id: &str,
    ) -> GatewayBatch<ProviderTopNFact> {
        GatewayBatch::Available {
            records: vec![
                record(metric.clone(), unit.clone(), 1),
                record(metric, unit, 2),
            ],
            evidence: evidence(batch_id),
        }
    }

    #[test]
    fn br240_transport_neutral_pair_rejects_wrong_source_metric_unit_date_or_order() {
        let volume = side(
            MarketRankingKind::VolumeRatio,
            MarketRankingUnit::Multiple,
            "TEST_CODE_BR240_VOLUME",
        );
        let inflow = side(
            MarketRankingKind::MainNetInflow,
            MarketRankingUnit::Yuan,
            "TEST_CODE_BR240_INFLOW",
        );
        let date = NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
        validate_provider_top_n_pair(&volume, &inflow, date).unwrap();

        let mutate_volume = |mutate: fn(&mut Vec<ProviderTopNFact>, &mut BatchEvidence)| {
            let mut records = volume.records().to_vec();
            let mut evidence = volume.evidence().clone();
            mutate(&mut records, &mut evidence);
            GatewayBatch::Available { records, evidence }
        };
        let cases = [
            (
                "wrong_source",
                mutate_volume(|_, evidence| evidence.source = "TEST_CODE_wrong-source".to_owned()),
            ),
            (
                "wrong_metric",
                mutate_volume(|records, _| records[0].metric = MarketRankingKind::MainNetInflow),
            ),
            (
                "wrong_unit",
                mutate_volume(|records, _| records[0].unit = MarketRankingUnit::Yuan),
            ),
            (
                "wrong_date",
                mutate_volume(|records, _| {
                    records[0].trading_date = IsoDate::new("2026-07-23").unwrap()
                }),
            ),
            (
                "wrong_order",
                mutate_volume(|records, _| {
                    records[0].source_order_ordinal = PositiveU32::new(2).unwrap()
                }),
            ),
        ];

        for (case, invalid_volume) in cases {
            let error = validate_provider_top_n_pair(&invalid_volume, &inflow, date)
                .expect_err("BR-240 invalid pair evidence must fail closed");
            assert_eq!(error.reason_code(), "invalid_evidence", "case={case}");
            assert!(!error.retryable(), "case={case}");
        }
    }

    #[test]
    fn br240_transport_neutral_pair_rejects_shared_upstream_batch_identity() {
        let shared_batch_id = "TEST_CODE_BR240_SHARED_BATCH";
        let volume = side(
            MarketRankingKind::VolumeRatio,
            MarketRankingUnit::Multiple,
            shared_batch_id,
        );
        let inflow = side(
            MarketRankingKind::MainNetInflow,
            MarketRankingUnit::Yuan,
            shared_batch_id,
        );

        let error = validate_provider_top_n_pair(
            &volume,
            &inflow,
            NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
        )
        .expect_err("BR-240 requires two independently attributable upstream batches");

        assert_eq!(
            error.capability(),
            PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY
        );
        assert_eq!(error.provider(), Some(ProviderId::Eastmoney));
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }
}

fn iso_date(value: NaiveDate, capability: &'static str) -> Result<IsoDate, GatewayError> {
    IsoDate::new(value.to_string())
        .map_err(|error| GatewayError::invalid_request(capability, error.to_string()))
}
