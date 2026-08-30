//! BR-164 typed capital-data boundary.
//!
//! The gateway only exposes capabilities proved by the pinned
//! `magic-market-data-rs` revision:
//!
//! - Eastmoney instrument fund-flow series (`Minute1` and `Day1`);
//! - Eastmoney source-limited post-close provider Top-N rankings;
//! - official HKEX northbound daily statistics.
//!
//! Every admitted batch is complete, identity-consistent, ordered, fresh for
//! its contract, and carries provider/batch/source/observation evidence.
//! Missing monetary components are rejected instead of being relabelled as
//! zero. Blocking provider clients are created, used and dropped inside
//! `spawn_blocking`, so they cannot drop a blocking runtime on a Tokio worker.

#[cfg(feature = "magic-gateway")]
use crate::market_domain::{DataBatch, Exchange, RatioUnit, SourceEvidence};
use crate::market_domain::{
    FiniteNumber, FlowInterval, InstrumentId, IsoDate, MarketRankingKind, MarketRankingUnit,
    NonEmptyText, NorthboundChannel, PositiveU32, ProviderId,
};
use chrono::NaiveDate;
#[cfg(feature = "magic-gateway")]
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
#[cfg(feature = "magic-gateway")]
use magic_eastmoney_rs::{EastmoneyClient, EastmoneyError};
#[cfg(feature = "magic-gateway")]
use magic_exchange_rs::{ExchangeError, HkexClient};
#[cfg(feature = "magic-gateway")]
use magic_market_composition::{
    EastmoneyProviderTopNRankingRouter, EastmoneyProviderTopNRouterError,
};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{
    validate_provider_top_n_ranking_batch, FlowScope, FundFlowPoint, FundFlowRequest,
    NorthboundDailyRequest, NorthboundDailyStat, NorthboundQuotaBalance, ProviderTopNRankingEntry,
    ProviderTopNRankingRequest,
};
#[cfg(feature = "magic-gateway")]
use magic_market_router::{
    fund_flow_series_source, northbound_daily_source, AcceptancePolicy, AttemptStatus, FailureKind,
    FundFlowSeriesRouter, NorthboundDailyRouter, RouterError, SourceError,
};
#[cfg(feature = "magic-gateway")]
use std::collections::HashSet;
#[cfg(feature = "magic-gateway")]
use std::sync::Arc;

use super::review::{acquisition_request_hash, audit_gateway_result, GatewayBatch, GatewayError};
#[cfg(feature = "magic-gateway")]
use super::review::{audit_blocking_join_failure, BatchEvidence};

const FUND_FLOW_CAPABILITY: &str = "CapitalInstrumentFundFlow";
const PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY: &str = "CapitalProviderTopNVolumeRatio";
const PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY: &str = "CapitalProviderTopNMainNetInflow";
const NORTHBOUND_CAPABILITY: &str = "CapitalNorthboundDaily";
const EASTMONEY_SOURCE: &str = "eastmoney-web";
const HKEX_SOURCE: &str = "hkex-official";
const PROVIDER_TOP_N_LIMIT: u32 = 20;
// Upstream `EastmoneyClient::provider_top_n_a_share_request` 的固定 A-share
// filter identity。桥模式本地构造 request evidence 时必须与 library transport
// 使用同一 filter, 才能保证 canonical request_hash 一致。
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
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                FUND_FLOW_CAPABILITY,
                Some(ProviderId::Eastmoney),
                "unavailable",
                "provider_transport",
                true,
                "remote market-data transport required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let worker_hash = request_hash.clone();
            let joined = tokio::task::spawn_blocking(move || {
                let result = build_fund_flow_request(&storage_code, interval, limit).and_then(
                    |(instrument, request)| {
                        route_fund_flow(&storage_code, &instrument, &request, Utc::now())
                    },
                );
                audit_gateway_result(
                    FUND_FLOW_CAPABILITY,
                    ProviderId::Eastmoney,
                    &worker_hash,
                    result,
                )
            })
            .await;
            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_blocking_join_failure(
                        FUND_FLOW_CAPABILITY,
                        ProviderId::Eastmoney,
                        request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
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
        #[cfg(feature = "magic-gateway")]
        let volume_request_hash = volume_request_evidence.request_hash.clone();
        #[cfg(feature = "magic-gateway")]
        let inflow_request_hash = inflow_request_evidence.request_hash.clone();
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
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
                Some(ProviderId::Eastmoney),
                "unavailable",
                "provider_transport",
                true,
                "remote market-data transport required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let volume_request = build_provider_top_n_request(
                MarketRankingKind::VolumeRatio,
                trading_date,
                PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
            )?;
            let inflow_request = build_provider_top_n_request(
                MarketRankingKind::MainNetInflow,
                trading_date,
                PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
            )?;
            let volume_request_evidence = provider_top_n_request_evidence(
                &volume_request,
                PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
            );
            let inflow_request_evidence = provider_top_n_request_evidence(
                &inflow_request,
                PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
            );
            let worker_volume_request_hash = volume_request_hash.clone();
            let worker_inflow_request_hash = inflow_request_hash.clone();
            let joined = tokio::task::spawn_blocking(move || {
                route_provider_top_n_pair(
                    volume_request,
                    volume_request_evidence,
                    inflow_request,
                    inflow_request_evidence,
                    trading_date,
                    &worker_volume_request_hash,
                    &worker_inflow_request_hash,
                )
            })
            .await;
            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_provider_top_n_join_failure(
                        volume_request_hash,
                        inflow_request_hash,
                        error.to_string(),
                    )
                    .await
                }
            }
        }
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
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(GatewayError::classified(
                NORTHBOUND_CAPABILITY,
                Some(ProviderId::Hkex),
                "unavailable",
                "provider_transport",
                true,
                "remote market-data transport required",
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            let worker_hash = request_hash.clone();
            let joined = tokio::task::spawn_blocking(move || {
                let result = iso_date(trading_date, NORTHBOUND_CAPABILITY)
                    .map(|date| NorthboundDailyRequest::new(date, channel))
                    .and_then(|request| route_northbound(&request, Utc::now()));
                audit_gateway_result(
                    NORTHBOUND_CAPABILITY,
                    ProviderId::Hkex,
                    &worker_hash,
                    result,
                )
            })
            .await;
            match joined {
                Ok(result) => result,
                Err(error) => {
                    audit_blocking_join_failure(
                        NORTHBOUND_CAPABILITY,
                        ProviderId::Hkex,
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
fn build_fund_flow_request(
    storage_code: &str,
    interval: FlowInterval,
    limit: u32,
) -> Result<(InstrumentId, FundFlowRequest), GatewayError> {
    if !matches!(interval, FlowInterval::Minute1 | FlowInterval::Day1) {
        return Err(GatewayError::invalid_request(
            FUND_FLOW_CAPABILITY,
            format!("unsupported instrument fund-flow interval {interval:?}"),
        ));
    }
    let instrument = a_share_instrument(storage_code, FUND_FLOW_CAPABILITY)?;
    let limit = positive_limit(limit, 10_000, FUND_FLOW_CAPABILITY)?;
    let request = FundFlowRequest::new(FlowScope::Instrument(instrument.clone()), interval, limit)
        .map_err(|error| GatewayError::invalid_request(FUND_FLOW_CAPABILITY, error.to_string()))?;
    Ok((instrument, request))
}

#[cfg(feature = "magic-gateway")]
fn build_provider_top_n_request(
    metric: MarketRankingKind,
    trading_date: NaiveDate,
    capability: &'static str,
) -> Result<ProviderTopNRankingRequest, GatewayError> {
    let date = iso_date(trading_date, capability)?;
    let limit = PositiveU32::new(PROVIDER_TOP_N_LIMIT)
        .map_err(|error| GatewayError::invalid_request(capability, error.to_string()))?;
    EastmoneyClient::provider_top_n_a_share_request(metric, date, limit)
        .map_err(|error| eastmoney_gateway_error(capability, error))
}

#[cfg(feature = "magic-gateway")]
fn provider_top_n_request_evidence(
    request: &ProviderTopNRankingRequest,
    capability: &'static str,
) -> ProviderTopNRequestEvidence {
    let canonical = format!(
        "metric={:?};trading_date={};limit={};filter={}",
        request.kind(),
        request.trading_date().as_str(),
        request.limit().get(),
        request.filter_identity().as_str()
    );
    ProviderTopNRequestEvidence {
        metric: request.kind().clone(),
        trading_date: request.trading_date().clone(),
        limit: request.limit(),
        filter_identity: request.filter_identity().clone(),
        request_hash: acquisition_request_hash(capability, &canonical),
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

#[cfg(feature = "magic-gateway")]
fn route_fund_flow(
    storage_code: &str,
    instrument: &InstrumentId,
    request: &FundFlowRequest,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<InstrumentFundFlowFact>, GatewayError> {
    let client = EastmoneyClient::new()
        .map_err(|error| eastmoney_gateway_error(FUND_FLOW_CAPABILITY, error))?;
    let mut router = FundFlowSeriesRouter::new(strict_policy());
    router
        .register(fund_flow_series_source(
            ProviderId::Eastmoney,
            Arc::new(client),
            classify_eastmoney_error,
        ))
        .map_err(|error| {
            router_gateway_error(FUND_FLOW_CAPABILITY, ProviderId::Eastmoney, error)
        })?;
    let outcome = router.route(request).map_err(|error| {
        router_gateway_error(FUND_FLOW_CAPABILITY, ProviderId::Eastmoney, error)
    })?;
    if outcome.selected_provider() != ProviderId::Eastmoney {
        return Err(GatewayError::invalid_evidence(
            FUND_FLOW_CAPABILITY,
            Some(outcome.selected_provider()),
            "unexpected provider selected for instrument fund flow",
        ));
    }
    admit_fund_flow_batch(storage_code, instrument, request, outcome.into_batch(), now)
}

#[cfg(feature = "magic-gateway")]
fn route_provider_top_n_pair(
    volume_request: ProviderTopNRankingRequest,
    volume_request_evidence: ProviderTopNRequestEvidence,
    inflow_request: ProviderTopNRankingRequest,
    inflow_request_evidence: ProviderTopNRequestEvidence,
    trading_date: NaiveDate,
    volume_request_hash: &str,
    inflow_request_hash: &str,
) -> Result<ProviderTopNPair, GatewayError> {
    let router = match EastmoneyProviderTopNRankingRouter::new() {
        Ok(router) => router,
        Err(error) => {
            let message = error.to_string();
            let volume = audit_gateway_result::<ProviderTopNFact>(
                PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
                ProviderId::Eastmoney,
                volume_request_hash,
                Err(GatewayError::classified(
                    PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
                    Some(ProviderId::Eastmoney),
                    "unsupported",
                    "provider_top_n_router_configuration",
                    false,
                    message.clone(),
                )),
            );
            let inflow = audit_gateway_result::<ProviderTopNFact>(
                PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
                ProviderId::Eastmoney,
                inflow_request_hash,
                Err(GatewayError::classified(
                    PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
                    Some(ProviderId::Eastmoney),
                    "unsupported",
                    "provider_top_n_router_configuration",
                    false,
                    message,
                )),
            );
            return match (volume, inflow) {
                (Err(error), _) | (_, Err(error)) => Err(error),
                (Ok(_), Ok(_)) => Err(GatewayError::classified(
                    PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
                    Some(ProviderId::Eastmoney),
                    "unsupported",
                    "provider_top_n_router_configuration",
                    false,
                    "provider Top-N Router initialization failed without an audited error",
                )),
            };
        }
    };

    let volume_ratio = route_provider_top_n(
        &router,
        &volume_request,
        PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
    );
    let volume_ratio = audit_gateway_result(
        PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
        ProviderId::Eastmoney,
        volume_request_hash,
        volume_ratio,
    )?;

    let main_net_inflow = route_provider_top_n(
        &router,
        &inflow_request,
        PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
    );
    let main_net_inflow = audit_gateway_result(
        PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
        ProviderId::Eastmoney,
        inflow_request_hash,
        main_net_inflow,
    )?;

    validate_provider_top_n_pair(&volume_ratio, &main_net_inflow, trading_date)?;
    Ok(ProviderTopNPair {
        volume_ratio_request: volume_request_evidence,
        volume_ratio,
        main_net_inflow_request: inflow_request_evidence,
        main_net_inflow,
    })
}

#[cfg(feature = "magic-gateway")]
fn route_provider_top_n(
    router: &EastmoneyProviderTopNRankingRouter,
    request: &ProviderTopNRankingRequest,
    capability: &'static str,
) -> Result<GatewayBatch<ProviderTopNFact>, GatewayError> {
    let outcome = router
        .route(request)
        .map_err(|error| provider_top_n_router_error(capability, error))?;
    if outcome.selected_provider() != ProviderId::Eastmoney {
        return Err(provider_top_n_invalid_evidence(
            capability,
            Some(outcome.selected_provider()),
            "unexpected provider selected for provider Top-N",
        ));
    }
    admit_provider_top_n_batch(
        request,
        outcome.into_batch(),
        router.capabilities(),
        router.expected_source(),
        capability,
    )
}

#[cfg(feature = "magic-gateway")]
fn route_northbound(
    request: &NorthboundDailyRequest,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<NorthboundDailyFact>, GatewayError> {
    let client =
        HkexClient::new().map_err(|error| exchange_gateway_error(NORTHBOUND_CAPABILITY, error))?;
    let mut router = NorthboundDailyRouter::new(strict_policy());
    router
        .register(northbound_daily_source(
            ProviderId::Hkex,
            Arc::new(client),
            classify_exchange_error,
        ))
        .map_err(|error| router_gateway_error(NORTHBOUND_CAPABILITY, ProviderId::Hkex, error))?;
    let outcome = router
        .route(request)
        .map_err(|error| router_gateway_error(NORTHBOUND_CAPABILITY, ProviderId::Hkex, error))?;
    if outcome.selected_provider() != ProviderId::Hkex {
        return Err(GatewayError::invalid_evidence(
            NORTHBOUND_CAPABILITY,
            Some(outcome.selected_provider()),
            "unexpected provider selected for northbound daily statistics",
        ));
    }
    admit_northbound_batch(request, outcome.into_batch(), now)
}

#[cfg(feature = "magic-gateway")]
fn strict_policy() -> AcceptancePolicy {
    AcceptancePolicy::new()
        .with_require_complete(true)
        .with_require_source_at(true)
}

#[cfg(feature = "magic-gateway")]
fn admit_fund_flow_batch(
    storage_code: &str,
    instrument: &InstrumentId,
    request: &FundFlowRequest,
    batch: DataBatch<FundFlowPoint>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<InstrumentFundFlowFact>, GatewayError> {
    let evidence = validate_batch(
        FUND_FLOW_CAPABILITY,
        ProviderId::Eastmoney,
        EASTMONEY_SOURCE,
        &batch,
        now,
    )?;
    if batch.records().len() != request.limit().get() as usize {
        return Err(GatewayError::invalid_evidence(
            FUND_FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!(
                "fund-flow cardinality mismatch requested={} actual={}",
                request.limit().get(),
                batch.records().len()
            ),
        ));
    }

    let expected_scope = FlowScope::Instrument(instrument.clone());
    let mut previous_period: Option<&str> = None;
    let mut periods = HashSet::with_capacity(batch.records().len());
    let mut projected = Vec::with_capacity(batch.records().len());
    for record in batch.records() {
        if record.scope != expected_scope || record.interval != request.interval() {
            return Err(GatewayError::invalid_evidence(
                FUND_FLOW_CAPABILITY,
                Some(ProviderId::Eastmoney),
                format!("fund-flow identity/interval mismatch for {storage_code}"),
            ));
        }
        validate_record_evidence(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            &evidence,
            &record.evidence,
        )?;
        let period = record.period_at.as_str();
        if record.evidence.source_at() != Some(period) {
            return Err(GatewayError::invalid_evidence(
                FUND_FLOW_CAPABILITY,
                Some(ProviderId::Eastmoney),
                format!("fund-flow record source_at differs from period_at {period}"),
            ));
        }
        if !periods.insert(period) || previous_period.is_some_and(|previous| previous >= period) {
            return Err(GatewayError::invalid_evidence(
                FUND_FLOW_CAPABILITY,
                Some(ProviderId::Eastmoney),
                "fund-flow periods must be unique and strictly increasing",
            ));
        }
        previous_period = Some(period);

        let main_net = required_money(record.main_net, "main_net", storage_code)?;
        let main_ratio = required_percent(record.main_ratio, "main_ratio", storage_code)?;
        let super_large_net =
            required_money(record.super_large_net, "super_large_net", storage_code)?;
        let large_net = required_money(record.large_net, "large_net", storage_code)?;
        let medium_net = required_money(record.medium_net, "medium_net", storage_code)?;
        let small_net = required_money(record.small_net, "small_net", storage_code)?;
        let composed_main = super_large_net + large_net;
        let tolerance = composed_main.abs().max(1.0) * 1.0e-8 + 0.01;
        if (main_net - composed_main).abs() > tolerance {
            return Err(GatewayError::invalid_evidence(
                FUND_FLOW_CAPABILITY,
                Some(ProviderId::Eastmoney),
                format!(
                    "fund-flow main_net contradicts super_large_net + large_net for {storage_code}"
                ),
            ));
        }
        projected.push(InstrumentFundFlowFact {
            code: storage_code.to_owned(),
            interval: record.interval,
            period_at: period.to_owned(),
            main_net,
            main_ratio_percent: main_ratio,
            super_large_net,
            large_net,
            medium_net,
            small_net,
        });
    }

    let latest = projected.last().ok_or_else(|| {
        GatewayError::invalid_evidence(
            FUND_FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            "fund-flow batch is empty",
        )
    })?;
    if evidence.source_at.as_deref() != Some(latest.period_at.as_str()) {
        return Err(GatewayError::invalid_evidence(
            FUND_FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            "fund-flow batch source_at does not equal latest period",
        ));
    }
    validate_fund_flow_freshness(request.interval(), &latest.period_at, now)?;
    Ok(GatewayBatch::Available {
        records: projected,
        evidence,
    })
}

#[cfg(feature = "magic-gateway")]
fn admit_provider_top_n_batch(
    request: &ProviderTopNRankingRequest,
    batch: DataBatch<ProviderTopNRankingEntry>,
    capabilities: magic_market_core::ProviderTopNRankingCapabilities,
    expected_source: &NonEmptyText,
    capability: &'static str,
) -> Result<GatewayBatch<ProviderTopNFact>, GatewayError> {
    validate_provider_top_n_ranking_batch(
        &batch,
        request,
        capabilities,
        ProviderId::Eastmoney,
        expected_source,
    )
    .map_err(|error| {
        provider_top_n_invalid_evidence(capability, Some(ProviderId::Eastmoney), error.to_string())
    })?;
    let evidence = BatchEvidence::from_provenance(ProviderId::Eastmoney, batch.provenance())
        .map_err(|error| {
            provider_top_n_invalid_evidence(
                capability,
                Some(ProviderId::Eastmoney),
                error.to_string(),
            )
        })?;
    if evidence.source_at.is_some() {
        return Err(provider_top_n_invalid_evidence(
            capability,
            Some(ProviderId::Eastmoney),
            "provider Top-N batch source_at must remain absent",
        ));
    }

    let records = batch
        .records()
        .iter()
        .map(|record| ProviderTopNFact {
            metric: record.kind().clone(),
            source_order_ordinal: record.source_order_ordinal(),
            instrument: record.instrument().clone(),
            label: record.label().clone(),
            value: record.value(),
            unit: record.unit().clone(),
            trading_date: record.latest_trading_date().clone(),
            filter_identity: record.filter_identity().clone(),
            provider_declared_total: record.provider_declared_total(),
            inspected_row_count: record.inspected_row_count(),
        })
        .collect::<Vec<_>>();
    if records.is_empty() {
        return Err(provider_top_n_invalid_evidence(
            capability,
            Some(ProviderId::Eastmoney),
            "provider Top-N batch must not be empty",
        ));
    }
    Ok(GatewayBatch::Available { records, evidence })
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

    #[cfg(feature = "magic-gateway")]
    #[test]
    fn br240_invalid_provider_evidence_is_non_retryable_through_delegate() {
        let error = provider_top_n_invalid_evidence(
            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
            Some(ProviderId::Eastmoney),
            "TEST_CODE deterministic invalid evidence",
        );
        let crate::grpc_server::delegate::DelegateError::Fetch(failure) =
            crate::grpc_server::delegate::provider_top_n_gateway_failure(error)
        else {
            panic!("provider Top-N gateway failures must cross the fetch boundary");
        };

        assert_eq!(failure.provider, Some(ProviderId::Eastmoney));
        assert_eq!(failure.reason_code, "invalid_evidence");
        assert!(!failure.retryable);
    }
}

#[cfg(feature = "magic-gateway")]
fn admit_northbound_batch(
    request: &NorthboundDailyRequest,
    batch: DataBatch<NorthboundDailyStat>,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<NorthboundDailyFact>, GatewayError> {
    let evidence = validate_batch(
        NORTHBOUND_CAPABILITY,
        ProviderId::Hkex,
        HKEX_SOURCE,
        &batch,
        now,
    )?;
    if batch.records().len() != 1 {
        return Err(GatewayError::invalid_evidence(
            NORTHBOUND_CAPABILITY,
            Some(ProviderId::Hkex),
            "northbound batch must contain exactly one channel",
        ));
    }
    let requested_date = parse_iso_date(request.trading_date(), NORTHBOUND_CAPABILITY)?;
    validate_daily_source_freshness(NORTHBOUND_CAPABILITY, ProviderId::Hkex, requested_date, now)?;
    if source_date(
        evidence.source_at.as_deref(),
        NORTHBOUND_CAPABILITY,
        ProviderId::Hkex,
    )? != requested_date
    {
        return Err(GatewayError::invalid_evidence(
            NORTHBOUND_CAPABILITY,
            Some(ProviderId::Hkex),
            "northbound batch source date differs from requested date",
        ));
    }

    let record = &batch.records()[0];
    validate_record_evidence(
        NORTHBOUND_CAPABILITY,
        ProviderId::Hkex,
        &evidence,
        record.evidence(),
    )?;
    if record.evidence().source_at() != evidence.source_at.as_deref()
        || record.trading_date() != request.trading_date()
        || record.channel() != request.channel()
    {
        return Err(GatewayError::invalid_evidence(
            NORTHBOUND_CAPABILITY,
            Some(ProviderId::Hkex),
            "northbound record identity/evidence differs from request",
        ));
    }

    let mut instruments = HashSet::with_capacity(10);
    let expected_exchange = match request.channel() {
        NorthboundChannel::Shanghai => Exchange::Shanghai,
        NorthboundChannel::Shenzhen => Exchange::Shenzhen,
    };
    let mut top_turnover = Vec::with_capacity(10);
    for (index, entry) in record.top_turnover().iter().enumerate() {
        if entry.rank().get() != index as u32 + 1
            || entry.instrument().exchange() != expected_exchange
            || !instruments.insert(entry.instrument().clone())
        {
            return Err(GatewayError::invalid_evidence(
                NORTHBOUND_CAPABILITY,
                Some(ProviderId::Hkex),
                "northbound Top-10 rank/exchange/identity is inconsistent",
            ));
        }
        top_turnover.push(NorthboundTopTurnoverFact {
            rank: entry.rank().get(),
            code: entry.instrument().code().to_owned(),
            name: entry.name().as_str().to_owned(),
            total_turnover: entry.total_turnover().get(),
        });
    }
    if top_turnover.len() != 10 {
        return Err(GatewayError::invalid_evidence(
            NORTHBOUND_CAPABILITY,
            Some(ProviderId::Hkex),
            "northbound Top-10 must contain exactly ten entries",
        ));
    }
    let quota_balance = match record.quota_balance() {
        NorthboundQuotaBalance::Amount(value) => NorthboundQuotaFact::Amount(value.get()),
        NorthboundQuotaBalance::Unavailable => NorthboundQuotaFact::Unavailable,
    };
    let projected = NorthboundDailyFact {
        trading_date: requested_date,
        channel: record.channel(),
        total_turnover: record.total_turnover().get(),
        total_trade_count: record.total_trade_count().get(),
        quota_balance,
        etf_turnover: record.etf_turnover().get(),
        top_turnover,
    };
    Ok(GatewayBatch::Available {
        records: vec![projected],
        evidence,
    })
}

#[cfg(feature = "magic-gateway")]
fn validate_batch<T>(
    capability: &'static str,
    provider: ProviderId,
    expected_source: &str,
    batch: &DataBatch<T>,
    now: DateTime<Utc>,
) -> Result<BatchEvidence, GatewayError> {
    if !batch.quality().is_complete() {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            format!(
                "provider returned partial batch: {:?}",
                batch.quality().issues()
            ),
        ));
    }
    let evidence = BatchEvidence::from_provenance(provider, batch.provenance())?;
    if evidence.source != expected_source {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            format!(
                "unexpected provenance source {:?}, expected {expected_source:?}",
                evidence.source
            ),
        ));
    }
    if evidence.source_at.is_none() {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "batch source_at is missing",
        ));
    }
    validate_observation_freshness(capability, provider, &evidence.observed_at, now)?;
    Ok(evidence)
}

#[cfg(feature = "magic-gateway")]
fn validate_record_evidence(
    capability: &'static str,
    provider: ProviderId,
    batch: &BatchEvidence,
    record: &SourceEvidence,
) -> Result<(), GatewayError> {
    if record.provider() != provider
        || record.batch_id() != batch.batch_id
        || record.observed_at() != batch.observed_at
        || record.source_at().is_none()
    {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            "record provider/batch/observation/source evidence differs from batch",
        ));
    }
    Ok(())
}

#[cfg(feature = "magic-gateway")]
fn validate_observation_freshness(
    capability: &'static str,
    provider: ProviderId,
    value: &str,
    now: DateTime<Utc>,
) -> Result<(), GatewayError> {
    let observed_at = parse_observed_at(value, capability, provider)?;
    let age = now.signed_duration_since(observed_at).num_milliseconds();
    if !(0..=OBSERVATION_MAX_AGE_MILLIS).contains(&age) {
        return Err(GatewayError::classified(
            capability,
            Some(provider),
            "stale",
            "capital_observation_stale",
            true,
            format!("provider observation failed 30-second freshness gate age_ms={age}"),
        ));
    }
    Ok(())
}

#[cfg(feature = "magic-gateway")]
fn validate_fund_flow_freshness(
    interval: FlowInterval,
    period_at: &str,
    now: DateTime<Utc>,
) -> Result<(), GatewayError> {
    match interval {
        FlowInterval::Day1 => {
            let date = NaiveDate::parse_from_str(period_at, "%Y-%m-%d").map_err(|error| {
                GatewayError::invalid_evidence(
                    FUND_FLOW_CAPABILITY,
                    Some(ProviderId::Eastmoney),
                    format!("invalid daily fund-flow period {period_at:?}: {error}"),
                )
            })?;
            validate_daily_source_freshness(FUND_FLOW_CAPABILITY, ProviderId::Eastmoney, date, now)
        }
        FlowInterval::Minute1 => {
            let local =
                NaiveDateTime::parse_from_str(period_at, "%Y-%m-%d %H:%M").map_err(|error| {
                    GatewayError::invalid_evidence(
                        FUND_FLOW_CAPABILITY,
                        Some(ProviderId::Eastmoney),
                        format!("invalid minute fund-flow period {period_at:?}: {error}"),
                    )
                })?;
            let source_at = shanghai_offset()
                .from_local_datetime(&local)
                .single()
                .ok_or_else(|| {
                    GatewayError::invalid_evidence(
                        FUND_FLOW_CAPABILITY,
                        Some(ProviderId::Eastmoney),
                        format!("ambiguous minute fund-flow period {period_at:?}"),
                    )
                })?
                .with_timezone(&Utc);
            let age = now.signed_duration_since(source_at).num_milliseconds();
            if !(0..=MINUTE_FLOW_MAX_AGE_MILLIS).contains(&age) {
                return Err(GatewayError::classified(
                    FUND_FLOW_CAPABILITY,
                    Some(ProviderId::Eastmoney),
                    "stale",
                    "minute_fund_flow_stale",
                    true,
                    format!(
                        "Minute1 fund flow failed two-minute source freshness gate age_ms={age}"
                    ),
                ));
            }
            Ok(())
        }
        _ => Err(GatewayError::invalid_request(
            FUND_FLOW_CAPABILITY,
            format!("unsupported fund-flow interval {interval:?}"),
        )),
    }
}

#[cfg(feature = "magic-gateway")]
fn validate_daily_source_freshness(
    capability: &'static str,
    provider: ProviderId,
    source_date: NaiveDate,
    now: DateTime<Utc>,
) -> Result<(), GatewayError> {
    let today = now.with_timezone(&shanghai_offset()).date_naive();
    if source_date > today {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            format!("daily source date {source_date} is in the future"),
        ));
    }
    let oldest_allowed = crate::calendar::prev_trading_day(today);
    if source_date < oldest_allowed {
        return Err(GatewayError::classified(
            capability,
            Some(provider),
            "stale",
            "daily_capital_source_stale",
            true,
            format!(
                "daily source date {source_date} is older than one trading day; \
                 oldest_allowed={oldest_allowed}"
            ),
        ));
    }
    Ok(())
}

#[cfg(feature = "magic-gateway")]
fn parse_observed_at(
    value: &str,
    capability: &'static str,
    provider: ProviderId,
) -> Result<DateTime<Utc>, GatewayError> {
    if let Some(raw) = value
        .strip_prefix("unix-ms:")
        .or_else(|| value.strip_prefix("unix:"))
    {
        let millis = raw.parse::<i64>().map_err(|error| {
            GatewayError::invalid_evidence(
                capability,
                Some(provider),
                format!("invalid observation epoch {value:?}: {error}"),
            )
        })?;
        return DateTime::<Utc>::from_timestamp_millis(millis).ok_or_else(|| {
            GatewayError::invalid_evidence(
                capability,
                Some(provider),
                format!("observation epoch is out of range: {value:?}"),
            )
        });
    }
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| {
            GatewayError::invalid_evidence(
                capability,
                Some(provider),
                format!("invalid observation timestamp {value:?}: {error}"),
            )
        })
}

#[cfg(feature = "magic-gateway")]
fn source_date(
    value: Option<&str>,
    capability: &'static str,
    provider: ProviderId,
) -> Result<NaiveDate, GatewayError> {
    let value = value.ok_or_else(|| {
        GatewayError::invalid_evidence(capability, Some(provider), "batch source_at is missing")
    })?;
    let date = value.get(..10).ok_or_else(|| {
        GatewayError::invalid_evidence(
            capability,
            Some(provider),
            format!("source_at must begin with YYYY-MM-DD: {value:?}"),
        )
    })?;
    if !matches!(value.as_bytes().get(10), None | Some(b' ') | Some(b'T')) {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider),
            format!("source_at date has an invalid separator: {value:?}"),
        ));
    }
    NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|error| {
        GatewayError::invalid_evidence(
            capability,
            Some(provider),
            format!("invalid source date {value:?}: {error}"),
        )
    })
}

#[cfg(feature = "magic-gateway")]
fn required_money(
    value: Option<crate::market_domain::Money>,
    field: &str,
    code: &str,
) -> Result<f64, GatewayError> {
    value.map(|money| money.get()).ok_or_else(|| {
        GatewayError::invalid_evidence(
            FUND_FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!("fund-flow {field} missing for {code}"),
        )
    })
}

#[cfg(feature = "magic-gateway")]
fn required_percent(
    value: Option<crate::market_domain::Ratio>,
    field: &str,
    code: &str,
) -> Result<f64, GatewayError> {
    let value = value.ok_or_else(|| {
        GatewayError::invalid_evidence(
            FUND_FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!("fund-flow {field} missing for {code}"),
        )
    })?;
    if value.unit() != RatioUnit::Percent {
        return Err(GatewayError::invalid_evidence(
            FUND_FLOW_CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!("fund-flow {field} is not percent for {code}"),
        ));
    }
    Ok(value.get())
}

#[cfg(feature = "magic-gateway")]
fn a_share_instrument(
    storage_code: &str,
    capability: &'static str,
) -> Result<InstrumentId, GatewayError> {
    #[cfg(test)]
    let resolved = super::instrument_identity::resolve_test_equity(storage_code, None);
    #[cfg(not(test))]
    let resolved = super::instrument_identity::resolve_production_equity(storage_code, None);
    let identity =
        resolved.map_err(|error| GatewayError::invalid_request(capability, error.to_string()))?;
    identity
        .require_a_share()
        .map_err(|error| GatewayError::invalid_request(capability, error.to_string()))?;
    if identity.instrument().exchange() == Exchange::Beijing {
        return Err(GatewayError::invalid_request(
            capability,
            "Eastmoney instrument fund-flow secid is not verified for Beijing A-shares",
        ));
    }
    Ok(identity.instrument().clone())
}

#[cfg(feature = "magic-gateway")]
fn positive_limit(
    value: u32,
    maximum: u32,
    capability: &'static str,
) -> Result<PositiveU32, GatewayError> {
    if value == 0 || value > maximum {
        return Err(GatewayError::invalid_request(
            capability,
            format!("limit must be in 1..={maximum}, got {value}"),
        ));
    }
    PositiveU32::new(value)
        .map_err(|error| GatewayError::invalid_request(capability, error.to_string()))
}

fn iso_date(value: NaiveDate, capability: &'static str) -> Result<IsoDate, GatewayError> {
    IsoDate::new(value.to_string())
        .map_err(|error| GatewayError::invalid_request(capability, error.to_string()))
}

#[cfg(feature = "magic-gateway")]
fn parse_iso_date(value: &IsoDate, capability: &'static str) -> Result<NaiveDate, GatewayError> {
    NaiveDate::parse_from_str(value.as_str(), "%Y-%m-%d")
        .map_err(|error| GatewayError::invalid_evidence(capability, None, error.to_string()))
}

#[cfg(feature = "magic-gateway")]
fn shanghai_offset() -> FixedOffset {
    FixedOffset::east_opt(SHANGHAI_OFFSET_SECONDS).expect("fixed Shanghai offset is valid")
}

#[cfg(feature = "magic-gateway")]
fn classify_eastmoney_error(error: EastmoneyError) -> SourceError {
    let message = error.to_string();
    match error {
        EastmoneyError::InvalidRequest(_) => {
            SourceError::stop(FailureKind::InvalidRequest, message)
        }
        EastmoneyError::Unsupported(_) => SourceError::stop(FailureKind::Unsupported, message),
        EastmoneyError::Transport(_) => SourceError::try_next(FailureKind::Transport, message),
        EastmoneyError::VerifiedEmpty(_) => SourceError::try_next(FailureKind::NoData, message),
        EastmoneyError::ResponseTooLarge { .. }
        | EastmoneyError::Decode(_)
        | EastmoneyError::Protocol(_) => SourceError::try_next(FailureKind::Protocol, message),
        EastmoneyError::Core(_) => SourceError::try_next(FailureKind::Quality, message),
    }
}

#[cfg(feature = "magic-gateway")]
fn classify_exchange_error(error: ExchangeError) -> SourceError {
    let message = error.to_string();
    match error {
        ExchangeError::InvalidRequest(_) => SourceError::stop(FailureKind::InvalidRequest, message),
        ExchangeError::Unsupported(_) => SourceError::stop(FailureKind::Unsupported, message),
        ExchangeError::Authentication(_) | ExchangeError::HttpStatus(_) => {
            SourceError::stop(FailureKind::Provider, message)
        }
        ExchangeError::RateLimited => SourceError::try_next(FailureKind::RateLimited, message),
        ExchangeError::Transport(_) | ExchangeError::Tls { .. } => {
            SourceError::try_next(FailureKind::Transport, message)
        }
        ExchangeError::Decode(_) | ExchangeError::Schema(_) => {
            SourceError::try_next(FailureKind::Protocol, message)
        }
        ExchangeError::Incomplete(_) | ExchangeError::Core(_) => {
            SourceError::try_next(FailureKind::Quality, message)
        }
    }
}

#[cfg(feature = "magic-gateway")]
fn eastmoney_gateway_error(capability: &'static str, error: EastmoneyError) -> GatewayError {
    let reason_code = error.category();
    let message = error.to_string();
    match error {
        EastmoneyError::InvalidRequest(_) => GatewayError::invalid_request(capability, message),
        EastmoneyError::Unsupported(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Eastmoney),
            "unsupported",
            reason_code,
            false,
            message,
        ),
        EastmoneyError::Transport(_) | EastmoneyError::ResponseTooLarge { .. } => {
            GatewayError::classified(
                capability,
                Some(ProviderId::Eastmoney),
                "unavailable",
                reason_code,
                true,
                message,
            )
        }
        EastmoneyError::VerifiedEmpty(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Eastmoney),
            "verified_empty",
            reason_code,
            false,
            message,
        ),
        EastmoneyError::Decode(_) | EastmoneyError::Protocol(_) | EastmoneyError::Core(_) => {
            GatewayError::classified(
                capability,
                Some(ProviderId::Eastmoney),
                "partial",
                reason_code,
                false,
                message,
            )
        }
    }
}

#[cfg(feature = "magic-gateway")]
fn provider_top_n_invalid_evidence(
    capability: &'static str,
    provider: Option<ProviderId>,
    message: impl Into<String>,
) -> GatewayError {
    GatewayError::invalid_evidence(capability, provider, message)
}

#[cfg(feature = "magic-gateway")]
fn provider_top_n_router_error(
    capability: &'static str,
    error: EastmoneyProviderTopNRouterError,
) -> GatewayError {
    match error {
        EastmoneyProviderTopNRouterError::RejectedRequest(message) => GatewayError::classified(
            capability,
            Some(ProviderId::Eastmoney),
            "invalid_request",
            "provider_top_n_future_date",
            false,
            message,
        ),
        EastmoneyProviderTopNRouterError::Clock(message) => GatewayError::classified(
            capability,
            Some(ProviderId::Eastmoney),
            "unavailable",
            "provider_top_n_clock_unavailable",
            true,
            message,
        ),
        EastmoneyProviderTopNRouterError::Routing(error) => {
            let terminal =
                error
                    .attempts()
                    .iter()
                    .rev()
                    .find_map(|attempt| match attempt.status() {
                        AttemptStatus::Failed { kind, .. }
                        | AttemptStatus::Rejected { kind, .. } => Some(*kind),
                        AttemptStatus::Selected => None,
                    });
            match terminal {
                Some(FailureKind::InvalidRequest) | None => GatewayError::classified(
                    capability,
                    Some(ProviderId::Eastmoney),
                    "invalid_request",
                    "provider_top_n_invalid_request",
                    false,
                    error.to_string(),
                ),
                Some(FailureKind::Unsupported) => GatewayError::classified(
                    capability,
                    Some(ProviderId::Eastmoney),
                    "unsupported",
                    "provider_top_n_unsupported",
                    false,
                    error.to_string(),
                ),
                Some(FailureKind::Transport)
                | Some(FailureKind::Timeout)
                | Some(FailureKind::RateLimited)
                | Some(FailureKind::NoData) => GatewayError::classified(
                    capability,
                    Some(ProviderId::Eastmoney),
                    "unavailable",
                    "provider_top_n_unavailable",
                    true,
                    error.to_string(),
                ),
                Some(FailureKind::Protocol)
                | Some(FailureKind::Quality)
                | Some(FailureKind::Evidence)
                | Some(FailureKind::Provider) => provider_top_n_invalid_evidence(
                    capability,
                    Some(ProviderId::Eastmoney),
                    error.to_string(),
                ),
            }
        }
    }
}

#[cfg(feature = "magic-gateway")]
async fn audit_provider_top_n_join_failure(
    volume_request_hash: String,
    inflow_request_hash: String,
    message: String,
) -> Result<ProviderTopNPair, GatewayError> {
    let volume = audit_blocking_join_failure::<ProviderTopNFact>(
        PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
        ProviderId::Eastmoney,
        volume_request_hash,
        message.clone(),
    )
    .await;
    let inflow = audit_blocking_join_failure::<ProviderTopNFact>(
        PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
        ProviderId::Eastmoney,
        inflow_request_hash,
        message,
    )
    .await;
    match (volume, inflow) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(_), Ok(_)) => Err(GatewayError::classified(
            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unavailable",
            "blocking_task_failed",
            true,
            "blocking provider task failed without an audited error",
        )),
    }
}

#[cfg(feature = "magic-gateway")]
fn exchange_gateway_error(capability: &'static str, error: ExchangeError) -> GatewayError {
    let message = error.to_string();
    match error {
        ExchangeError::InvalidRequest(_) => GatewayError::invalid_request(capability, message),
        ExchangeError::Unsupported(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Hkex),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        ExchangeError::RateLimited | ExchangeError::Transport(_) | ExchangeError::Tls { .. } => {
            GatewayError::classified(
                capability,
                Some(ProviderId::Hkex),
                "unavailable",
                "provider_transport",
                true,
                message,
            )
        }
        ExchangeError::Authentication(_) | ExchangeError::HttpStatus(_) => {
            GatewayError::classified(
                capability,
                Some(ProviderId::Hkex),
                "unavailable",
                "provider_rejected",
                false,
                message,
            )
        }
        ExchangeError::Decode(_)
        | ExchangeError::Schema(_)
        | ExchangeError::Incomplete(_)
        | ExchangeError::Core(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Hkex),
            "partial",
            "provider_invalid_batch",
            false,
            message,
        ),
    }
}

#[cfg(feature = "magic-gateway")]
fn router_gateway_error(
    capability: &'static str,
    provider: ProviderId,
    error: RouterError,
) -> GatewayError {
    let terminal = error
        .attempts()
        .iter()
        .rev()
        .find_map(|attempt| match attempt.status() {
            AttemptStatus::Failed { kind, .. } | AttemptStatus::Rejected { kind, .. } => {
                Some(*kind)
            }
            AttemptStatus::Selected => None,
        });
    let (outcome, reason, retryable) = match terminal {
        Some(FailureKind::InvalidRequest) | None => {
            ("invalid_request", "router_invalid_request", false)
        }
        Some(FailureKind::Unsupported) => ("unsupported", "provider_unsupported", false),
        Some(FailureKind::Transport)
        | Some(FailureKind::Timeout)
        | Some(FailureKind::RateLimited)
        | Some(FailureKind::NoData) => ("unavailable", "provider_unavailable", true),
        Some(FailureKind::Protocol)
        | Some(FailureKind::Quality)
        | Some(FailureKind::Evidence)
        | Some(FailureKind::Provider) => ("partial", "provider_batch_rejected", false),
    };
    GatewayError::classified(
        capability,
        Some(provider),
        outcome,
        reason,
        retryable,
        error.to_string(),
    )
}

#[cfg(all(test, feature = "magic-gateway"))]
mod tests {
    use super::{
        a_share_instrument, admit_fund_flow_batch, admit_northbound_batch,
        admit_provider_top_n_batch, build_fund_flow_request, build_provider_top_n_request,
        classify_eastmoney_error, classify_exchange_error, eastmoney_gateway_error,
        exchange_gateway_error, parse_observed_at, positive_limit, provider_top_n_request_evidence,
        required_money, required_percent, router_gateway_error, source_date, validate_batch,
        validate_daily_source_freshness, validate_fund_flow_freshness,
        validate_observation_freshness, validate_provider_top_n_pair, validate_record_evidence,
        InstrumentFundFlowFact, NorthboundQuotaFact, ProviderTopNPair, FUND_FLOW_CAPABILITY,
        NORTHBOUND_CAPABILITY, PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
        PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
    };
    use crate::market_domain::{
        AssetClass, DataBatch, Exchange, FiniteNumber, FlowInterval, InstrumentId, IsoDate,
        MarketRankingKind, MarketRankingUnit, Money, NonEmptyText, NorthboundChannel, PositiveU32,
        Provenance, ProviderId, Quantity, Ratio, RatioUnit, SourceEvidence,
    };
    use chrono::{DateTime, NaiveDate, Utc};
    #[cfg(feature = "magic-gateway")]
    use magic_eastmoney_rs::{EastmoneyClient, EastmoneyError};
    #[cfg(feature = "magic-gateway")]
    use magic_exchange_rs::{ExchangeError, TlsBackend};
    #[cfg(feature = "magic-gateway")]
    use magic_market_core::{
        FlowScope, FundFlowPoint, FundFlowRequest, NorthboundDailyRequest, NorthboundDailyStat,
        NorthboundQuotaBalance, NorthboundTopTurnover, ProviderTopNRankingCapabilities,
        ProviderTopNRankingEntry, VerifiedEmpty,
    };
    use magic_market_router::{
        AcceptancePolicy, FailoverChain, FailureAction, FailureKind, RouterError, SourceError,
        SourceFn,
    };

    fn verified_empty_error() -> EastmoneyError {
        let observed_at = "2099-07-24T08:00:00Z";
        let batch_id = "TEST_CODE_verified_empty";
        let evidence =
            SourceEvidence::new(ProviderId::Eastmoney, observed_at, batch_id).expect("evidence");
        let provenance = Provenance::new("eastmoney", observed_at)
            .expect("provenance")
            .with_batch_id(batch_id)
            .expect("batch ID");
        let empty = VerifiedEmpty::new(
            "TEST_CODE_capital",
            "TEST_CODE_request",
            "TEST_CODE source proved empty",
            evidence,
            provenance,
        )
        .expect("verified-empty contract");
        EastmoneyError::VerifiedEmpty(Box::new(empty))
    }

    fn now(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn provenance(source: &str, observed_at: &str, source_at: &str, batch_id: &str) -> Provenance {
        Provenance::new(source, observed_at)
            .unwrap()
            .with_source_at(source_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap()
    }

    fn evidence(
        provider: ProviderId,
        observed_at: &str,
        source_at: &str,
        batch_id: &str,
    ) -> SourceEvidence {
        SourceEvidence::new(provider, observed_at, batch_id)
            .unwrap()
            .with_source_at(source_at)
            .unwrap()
    }

    fn fund_flow_point(
        instrument: &InstrumentId,
        interval: FlowInterval,
        period_at: &str,
        observed_at: &str,
        batch_id: &str,
    ) -> FundFlowPoint {
        FundFlowPoint {
            scope: FlowScope::Instrument(instrument.clone()),
            interval,
            period_at: NonEmptyText::new(period_at).unwrap(),
            main_net: Some(Money::new(30.0).unwrap()),
            main_ratio: Some(Ratio::new(3.0, RatioUnit::Percent).unwrap()),
            super_large_net: Some(Money::new(10.0).unwrap()),
            large_net: Some(Money::new(20.0).unwrap()),
            medium_net: Some(Money::new(-5.0).unwrap()),
            small_net: Some(Money::new(-25.0).unwrap()),
            evidence: evidence(ProviderId::Eastmoney, observed_at, period_at, batch_id),
        }
    }

    fn provider_top_n_fixture(
        kind: MarketRankingKind,
        date: &str,
        limit: u32,
        observed_at: &str,
        batch_id: &str,
    ) -> (
        magic_market_core::ProviderTopNRankingRequest,
        DataBatch<ProviderTopNRankingEntry>,
    ) {
        let date = IsoDate::new(date).unwrap();
        let request = EastmoneyClient::provider_top_n_a_share_request(
            kind.clone(),
            date.clone(),
            PositiveU32::new(limit).unwrap(),
        )
        .unwrap();
        let unit = match kind {
            MarketRankingKind::VolumeRatio => MarketRankingUnit::Multiple,
            MarketRankingKind::MainNetInflow => MarketRankingUnit::Yuan,
            _ => unreachable!("fixture only builds the two admitted Top-N metrics"),
        };
        let records = (1..=limit)
            .map(|ordinal| {
                ProviderTopNRankingEntry::new(
                    kind.clone(),
                    PositiveU32::new(ordinal).unwrap(),
                    InstrumentId::new(
                        Exchange::Shanghai,
                        format!("600{ordinal:03}"),
                        AssetClass::Equity,
                    )
                    .unwrap(),
                    NonEmptyText::new(format!("TEST_CODE_TOP_N_{ordinal}")).unwrap(),
                    FiniteNumber::new(f64::from(limit + 1 - ordinal)).unwrap(),
                    unit.clone(),
                    date.clone(),
                    request.filter_identity().clone(),
                    PositiveU32::new(100).unwrap(),
                    PositiveU32::new(limit).unwrap(),
                    SourceEvidence::new(ProviderId::Eastmoney, observed_at, batch_id).unwrap(),
                )
                .unwrap()
            })
            .collect();
        let provenance = Provenance::new("eastmoney-web", observed_at)
            .unwrap()
            .with_batch_id(batch_id)
            .unwrap();
        (request, DataBatch::strict(records, provenance))
    }

    fn northbound_record(
        channel: NorthboundChannel,
        quota_balance: NorthboundQuotaBalance,
        provider: ProviderId,
        observed_at: &str,
        source_at: &str,
        batch_id: &str,
    ) -> NorthboundDailyStat {
        let exchange = match channel {
            NorthboundChannel::Shanghai => Exchange::Shanghai,
            NorthboundChannel::Shenzhen => Exchange::Shenzhen,
        };
        let prefix = match channel {
            NorthboundChannel::Shanghai => "600",
            NorthboundChannel::Shenzhen => "000",
        };
        let top = (1..=10)
            .map(|rank| {
                NorthboundTopTurnover::new(
                    PositiveU32::new(rank).unwrap(),
                    InstrumentId::new(exchange, format!("{prefix}{rank:03}"), AssetClass::Equity)
                        .unwrap(),
                    NonEmptyText::new(format!("TEST_CODE_STOCK_{rank}")).unwrap(),
                    Money::new(f64::from(11 - rank) * 100.0).unwrap(),
                )
                .unwrap()
            })
            .collect();
        NorthboundDailyStat::new(
            IsoDate::new("2026-07-24").unwrap(),
            channel,
            Money::new(1000.0).unwrap(),
            Quantity::new(500.0).unwrap(),
            quota_balance,
            Money::new(100.0).unwrap(),
            top,
            evidence(provider, observed_at, source_at, batch_id),
        )
        .unwrap()
    }

    fn routed_error(kind: FailureKind, action: FailureAction) -> RouterError {
        let mut router = FailoverChain::<(), FundFlowPoint>::new(AcceptancePolicy::new());
        router
            .register(SourceFn::new(
                ProviderId::Eastmoney,
                move |_: &()| -> Result<DataBatch<FundFlowPoint>, SourceError> {
                    Err(SourceError::new(kind, action, "TEST_CODE_ROUTER_ERROR"))
                },
            ))
            .unwrap();
        router.route(&()).unwrap_err()
    }

    #[test]
    fn observed_epoch_markers_are_parsed_without_guessing_units() {
        assert_eq!(
            parse_observed_at(
                "unix-ms:1784879995000",
                NORTHBOUND_CAPABILITY,
                ProviderId::Hkex
            )
            .unwrap(),
            DateTime::<Utc>::from_timestamp_millis(1_784_879_995_000).unwrap()
        );
        assert!(
            parse_observed_at("unix:not-a-number", NORTHBOUND_CAPABILITY, ProviderId::Hkex)
                .is_err()
        );
    }

    #[test]
    fn fund_flow_seam_rejects_missing_components() {
        let instrument =
            a_share_instrument("TEST_CODE_600000", super::FUND_FLOW_CAPABILITY).unwrap();
        let request = FundFlowRequest::new(
            FlowScope::Instrument(instrument.clone()),
            FlowInterval::Day1,
            PositiveU32::new(1).unwrap(),
        )
        .unwrap();
        let observed_at = "2026-07-24T07:59:55Z";
        let source_at = "2026-07-24";
        let batch_id = "test-fund";
        let record = FundFlowPoint {
            scope: FlowScope::Instrument(instrument.clone()),
            interval: FlowInterval::Day1,
            period_at: NonEmptyText::new(source_at).unwrap(),
            main_net: Some(Money::new(30.0).unwrap()),
            main_ratio: None,
            super_large_net: Some(Money::new(10.0).unwrap()),
            large_net: Some(Money::new(20.0).unwrap()),
            medium_net: Some(Money::new(-5.0).unwrap()),
            small_net: Some(Money::new(-25.0).unwrap()),
            evidence: evidence(ProviderId::Eastmoney, observed_at, source_at, batch_id),
        };
        let batch = DataBatch::strict(
            vec![record],
            provenance("eastmoney-web", observed_at, source_at, batch_id),
        );
        assert!(admit_fund_flow_batch(
            "TEST_CODE_600000",
            &instrument,
            &request,
            batch,
            now("2026-07-24T08:00:00Z")
        )
        .is_err());
    }

    #[test]
    fn complete_fund_flow_seam_is_admitted() {
        let instrument =
            a_share_instrument("TEST_CODE_600000", super::FUND_FLOW_CAPABILITY).unwrap();
        let request = FundFlowRequest::new(
            FlowScope::Instrument(instrument.clone()),
            FlowInterval::Day1,
            PositiveU32::new(1).unwrap(),
        )
        .unwrap();
        let observed_at = "2026-07-24T07:59:55Z";
        let source_at = "2026-07-24";
        let batch_id = "test-fund";
        let record = FundFlowPoint {
            scope: FlowScope::Instrument(instrument.clone()),
            interval: FlowInterval::Day1,
            period_at: NonEmptyText::new(source_at).unwrap(),
            main_net: Some(Money::new(30.0).unwrap()),
            main_ratio: Some(Ratio::new(3.0, RatioUnit::Percent).unwrap()),
            super_large_net: Some(Money::new(10.0).unwrap()),
            large_net: Some(Money::new(20.0).unwrap()),
            medium_net: Some(Money::new(-5.0).unwrap()),
            small_net: Some(Money::new(-25.0).unwrap()),
            evidence: evidence(ProviderId::Eastmoney, observed_at, source_at, batch_id),
        };
        let batch = DataBatch::strict(
            vec![record],
            provenance("eastmoney-web", observed_at, source_at, batch_id),
        );
        let admitted = admit_fund_flow_batch(
            "TEST_CODE_600000",
            &instrument,
            &request,
            batch,
            now("2026-07-24T08:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            admitted.records(),
            &[InstrumentFundFlowFact {
                code: "TEST_CODE_600000".into(),
                interval: FlowInterval::Day1,
                period_at: "2026-07-24".into(),
                main_net: 30.0,
                main_ratio_percent: 3.0,
                super_large_net: 10.0,
                large_net: 20.0,
                medium_net: -5.0,
                small_net: -25.0,
            }]
        );
    }

    #[test]
    fn provider_top_n_seam_preserves_typed_rows_order_and_absent_source_at() {
        let observed_at = "2026-07-24T15:36:00+08:00";
        let (request, batch) = provider_top_n_fixture(
            MarketRankingKind::VolumeRatio,
            "2026-07-24",
            2,
            observed_at,
            "TEST_CODE_TOP_N_VOLUME",
        );
        let source = EastmoneyClient::provider_top_n_source_identity().unwrap();
        let admitted = admit_provider_top_n_batch(
            &request,
            batch,
            ProviderTopNRankingCapabilities {
                volume_ratio: true,
                main_net_inflow: true,
            },
            &source,
            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
        )
        .unwrap();
        assert_eq!(admitted.records().len(), 2);
        assert_eq!(admitted.records()[0].source_order_ordinal.get(), 1);
        assert_eq!(admitted.records()[0].instrument.code(), "600001");
        assert_eq!(admitted.records()[0].metric, MarketRankingKind::VolumeRatio);
        assert_eq!(admitted.records()[0].unit, MarketRankingUnit::Multiple);
        assert_eq!(admitted.records()[0].value.get(), 2.0);
        assert_eq!(admitted.records()[0].provider_declared_total.get(), 100);
        assert_eq!(admitted.records()[0].inspected_row_count.get(), 2);
        assert_eq!(admitted.evidence().batch_id, "TEST_CODE_TOP_N_VOLUME");
        assert_eq!(admitted.evidence().observed_at, observed_at);
        assert!(admitted.evidence().source_at.is_none());
    }

    #[test]
    fn br198_provider_top_n_admits_later_capture_for_exact_settled_date() {
        let observed_at = "2026-08-01T08:15:00+08:00";
        let (request, batch) = provider_top_n_fixture(
            MarketRankingKind::VolumeRatio,
            "2026-07-31",
            2,
            observed_at,
            "TEST_CODE_BR198_SETTLED",
        );
        let source = EastmoneyClient::provider_top_n_source_identity().unwrap();

        let admitted = admit_provider_top_n_batch(
            &request,
            batch,
            ProviderTopNRankingCapabilities {
                volume_ratio: true,
                main_net_inflow: true,
            },
            &source,
            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
        )
        .expect("later closed-day capture must preserve the exact settled trading date");

        let evidence = admitted.evidence();
        assert_eq!(evidence.observed_at, observed_at);
        assert_eq!(admitted.records().len(), 2);
        assert!(admitted
            .records()
            .iter()
            .all(|record| record.trading_date.as_str() == "2026-07-31"));
    }

    #[test]
    fn northbound_seam_requires_official_ten_entry_shape() {
        let date = IsoDate::new("2026-07-24").unwrap();
        let request = NorthboundDailyRequest::new(date.clone(), NorthboundChannel::Shanghai);
        let observed_at = "2026-07-24T07:59:55Z";
        let source_at = "2026-07-24";
        let batch_id = "test-hkex";
        let top = (1..=10)
            .map(|rank| {
                NorthboundTopTurnover::new(
                    PositiveU32::new(rank).unwrap(),
                    InstrumentId::new(
                        Exchange::Shanghai,
                        format!("600{rank:03}"),
                        AssetClass::Equity,
                    )
                    .unwrap(),
                    NonEmptyText::new(format!("股票{rank}")).unwrap(),
                    Money::new(f64::from(11 - rank) * 100.0).unwrap(),
                )
                .unwrap()
            })
            .collect();
        let record = NorthboundDailyStat::new(
            date,
            NorthboundChannel::Shanghai,
            Money::new(1000.0).unwrap(),
            Quantity::new(500.0).unwrap(),
            NorthboundQuotaBalance::Unavailable,
            Money::new(100.0).unwrap(),
            top,
            evidence(ProviderId::Hkex, observed_at, source_at, batch_id),
        )
        .unwrap();
        let batch = DataBatch::strict(
            vec![record],
            provenance("hkex-official", observed_at, source_at, batch_id),
        );
        let admitted =
            admit_northbound_batch(&request, batch, now("2026-07-24T08:00:00Z")).unwrap();
        assert_eq!(admitted.records()[0].top_turnover.len(), 10);
    }

    #[test]
    fn capital_requests_enforce_supported_intervals_limits_and_a_share_identity() {
        let (instrument, request) =
            build_fund_flow_request("TEST_CODE_600000", FlowInterval::Day1, 120).unwrap();
        assert_eq!(instrument.exchange(), Exchange::Shanghai);
        assert_eq!(request.limit().get(), 120);
        assert!(build_fund_flow_request("TEST_CODE_600000", FlowInterval::Day5, 1).is_err());
        assert!(build_fund_flow_request("TEST_CODE_600000", FlowInterval::Day1, 0).is_err());
        assert!(build_fund_flow_request("TEST_CODE_600000", FlowInterval::Day1, 10_001).is_err());

        assert_eq!(
            a_share_instrument("TEST_CODE_000001", FUND_FLOW_CAPABILITY)
                .unwrap()
                .exchange(),
            Exchange::Shenzhen
        );
        assert!(a_share_instrument("TEST_CODE_920047", FUND_FLOW_CAPABILITY).is_err());
        for code in [
            "TEST_CODE_430047",
            "TEST_CODE_830047",
            "TEST_CODE_200001",
            "TEST_CODE_900901",
        ] {
            assert!(
                a_share_instrument(code, FUND_FLOW_CAPABILITY).is_err(),
                "{code}"
            );
        }
        assert!(a_share_instrument("TEST_CODE_100001", FUND_FLOW_CAPABILITY).is_err());
        assert!(a_share_instrument("TEST_CODE_60000A", FUND_FLOW_CAPABILITY).is_err());

        let date = NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
        let request = build_provider_top_n_request(
            MarketRankingKind::VolumeRatio,
            date,
            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
        )
        .unwrap();
        assert_eq!(request.limit().get(), 20);
        assert_eq!(request.kind(), &MarketRankingKind::VolumeRatio);
        assert_eq!(
            positive_limit(1, 1, PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY)
                .unwrap()
                .get(),
            1
        );
        let request_evidence =
            provider_top_n_request_evidence(&request, PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY);
        assert_eq!(request_evidence.metric, MarketRankingKind::VolumeRatio);
        assert_eq!(request_evidence.limit.get(), 20);
        assert_eq!(
            request_evidence.filter_identity,
            request.filter_identity().clone()
        );
        assert_eq!(request_evidence.request_hash.len(), 64);
    }

    #[test]
    fn capital_value_and_source_date_gates_never_fill_missing_fields() {
        assert_eq!(
            required_money(
                Some(Money::new(12.5).unwrap()),
                "main_net",
                "TEST_CODE_600000"
            )
            .unwrap(),
            12.5
        );
        assert!(required_money(None, "main_net", "TEST_CODE_600000").is_err());
        assert_eq!(
            required_percent(
                Some(Ratio::new(3.5, RatioUnit::Percent).unwrap()),
                "main_ratio",
                "TEST_CODE_600000"
            )
            .unwrap(),
            3.5
        );
        assert!(required_percent(None, "main_ratio", "TEST_CODE_600000").is_err());
        assert!(required_percent(
            Some(Ratio::new(0.035, RatioUnit::Decimal).unwrap()),
            "main_ratio",
            "TEST_CODE_600000"
        )
        .is_err());

        assert_eq!(
            source_date(
                Some("2026-07-24T15:35:00+08:00"),
                NORTHBOUND_CAPABILITY,
                ProviderId::Hkex,
            )
            .unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 24).unwrap()
        );
        assert!(source_date(None, NORTHBOUND_CAPABILITY, ProviderId::Hkex).is_err());
        assert!(source_date(
            Some("2026-07-24X15:35:00"),
            NORTHBOUND_CAPABILITY,
            ProviderId::Hkex,
        )
        .is_err());
        assert!(source_date(Some("TEST_CODE"), NORTHBOUND_CAPABILITY, ProviderId::Hkex,).is_err());
    }

    #[test]
    fn capital_freshness_gates_reject_future_stale_and_malformed_source_times() {
        let current = now("2026-07-24T08:00:00Z");
        assert!(validate_daily_source_freshness(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
            current,
        )
        .is_ok());
        assert!(validate_daily_source_freshness(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            NaiveDate::from_ymd_opt(2026, 7, 25).unwrap(),
            current,
        )
        .is_err());
        let stale = validate_daily_source_freshness(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            NaiveDate::from_ymd_opt(2026, 7, 22).unwrap(),
            current,
        )
        .unwrap_err();
        assert_eq!(stale.reason_code(), "daily_capital_source_stale");
        assert!(stale.retryable());

        assert!(
            validate_fund_flow_freshness(FlowInterval::Minute1, "2026-07-24 16:00", current,)
                .is_ok()
        );
        let stale_minute =
            validate_fund_flow_freshness(FlowInterval::Minute1, "2026-07-24 15:57", current)
                .unwrap_err();
        assert_eq!(stale_minute.reason_code(), "minute_fund_flow_stale");
        assert!(
            validate_fund_flow_freshness(FlowInterval::Minute1, "TEST_CODE invalid", current,)
                .is_err()
        );
    }

    #[test]
    fn batch_admission_rejects_partial_wrong_source_missing_source_and_stale_observations() {
        let current = now("2026-07-24T08:00:00Z");
        let complete = DataBatch::<FundFlowPoint>::strict(
            Vec::new(),
            provenance(
                "eastmoney-web",
                "2026-07-24T07:59:55Z",
                "2026-07-24",
                "TEST_CODE_BATCH",
            ),
        );
        assert!(validate_batch(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            "eastmoney-web",
            &complete,
            current,
        )
        .is_ok());

        let partial = DataBatch::<FundFlowPoint>::best_effort(
            Vec::new(),
            provenance(
                "eastmoney-web",
                "2026-07-24T07:59:55Z",
                "2026-07-24",
                "TEST_CODE_BATCH",
            ),
            vec!["TEST_CODE_PARTIAL".to_owned()],
        )
        .unwrap();
        let error = validate_batch(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            "eastmoney-web",
            &partial,
            current,
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());

        let wrong_source = DataBatch::<FundFlowPoint>::strict(
            Vec::new(),
            provenance(
                "TEST_CODE_WRONG_SOURCE",
                "2026-07-24T07:59:55Z",
                "2026-07-24",
                "TEST_CODE_BATCH",
            ),
        );
        assert!(validate_batch(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            "eastmoney-web",
            &wrong_source,
            current,
        )
        .unwrap_err()
        .to_string()
        .contains("unexpected provenance source"));

        let missing_source_at = DataBatch::<FundFlowPoint>::strict(
            Vec::new(),
            Provenance::new("eastmoney-web", "2026-07-24T07:59:55Z")
                .unwrap()
                .with_batch_id("TEST_CODE_BATCH")
                .unwrap(),
        );
        assert!(validate_batch(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            "eastmoney-web",
            &missing_source_at,
            current,
        )
        .unwrap_err()
        .to_string()
        .contains("source_at is missing"));

        for observed_at in ["2026-07-24T07:59:29.999Z", "2026-07-24T08:00:00.001Z"] {
            let batch = DataBatch::<FundFlowPoint>::strict(
                Vec::new(),
                provenance(
                    "eastmoney-web",
                    observed_at,
                    "2026-07-24",
                    "TEST_CODE_BATCH",
                ),
            );
            let error = validate_batch(
                FUND_FLOW_CAPABILITY,
                ProviderId::Eastmoney,
                "eastmoney-web",
                &batch,
                current,
            )
            .unwrap_err();
            assert_eq!(error.audit_outcome(), "stale");
            assert_eq!(error.reason_code(), "capital_observation_stale");
            assert!(error.retryable());
        }
    }

    #[test]
    fn record_admission_requires_provider_batch_observation_and_source_evidence() {
        let observed_at = "2026-07-24T07:59:55Z";
        let batch_id = "TEST_CODE_BATCH";
        let batch = DataBatch::<FundFlowPoint>::strict(
            Vec::new(),
            provenance("eastmoney-web", observed_at, "2026-07-24", batch_id),
        );
        let batch_evidence = validate_batch(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            "eastmoney-web",
            &batch,
            now("2026-07-24T08:00:00Z"),
        )
        .unwrap();
        assert!(validate_record_evidence(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            &batch_evidence,
            &evidence(ProviderId::Eastmoney, observed_at, "2026-07-24", batch_id,),
        )
        .is_ok());

        let invalid = [
            evidence(ProviderId::Hkex, observed_at, "2026-07-24", batch_id),
            evidence(
                ProviderId::Eastmoney,
                observed_at,
                "2026-07-24",
                "TEST_CODE_OTHER_BATCH",
            ),
            evidence(
                ProviderId::Eastmoney,
                "2026-07-24T07:59:54Z",
                "2026-07-24",
                batch_id,
            ),
            SourceEvidence::new(ProviderId::Eastmoney, observed_at, batch_id).unwrap(),
        ];
        for record in invalid {
            let error = validate_record_evidence(
                FUND_FLOW_CAPABILITY,
                ProviderId::Eastmoney,
                &batch_evidence,
                &record,
            )
            .unwrap_err();
            assert_eq!(error.reason_code(), "invalid_evidence");
            assert!(!error.retryable());
        }
    }

    #[test]
    fn fund_flow_admission_rejects_cardinality_identity_order_math_and_latest_source_drift() {
        let instrument = a_share_instrument("TEST_CODE_600000", FUND_FLOW_CAPABILITY).unwrap();
        let request = FundFlowRequest::new(
            FlowScope::Instrument(instrument.clone()),
            FlowInterval::Day1,
            PositiveU32::new(2).unwrap(),
        )
        .unwrap();
        let observed_at = "2026-07-24T07:59:55Z";
        let batch_id = "TEST_CODE_FUND";
        let first = fund_flow_point(
            &instrument,
            FlowInterval::Day1,
            "2026-07-23",
            observed_at,
            batch_id,
        );
        let second = fund_flow_point(
            &instrument,
            FlowInterval::Day1,
            "2026-07-24",
            observed_at,
            batch_id,
        );
        let make_batch = |records, source_at| {
            DataBatch::strict(
                records,
                provenance("eastmoney-web", observed_at, source_at, batch_id),
            )
        };
        let current = now("2026-07-24T08:00:00Z");
        assert!(admit_fund_flow_batch(
            "TEST_CODE_600000",
            &instrument,
            &request,
            make_batch(vec![first.clone(), second.clone()], "2026-07-24"),
            current,
        )
        .is_ok());

        let cases = [
            (
                make_batch(vec![first.clone()], "2026-07-23"),
                "cardinality mismatch",
            ),
            (
                make_batch(
                    vec![
                        first.clone(),
                        fund_flow_point(
                            &instrument,
                            FlowInterval::Minute1,
                            "2026-07-24",
                            observed_at,
                            batch_id,
                        ),
                    ],
                    "2026-07-24",
                ),
                "identity/interval mismatch",
            ),
            (
                {
                    let mut record = second.clone();
                    record.evidence =
                        evidence(ProviderId::Eastmoney, observed_at, "2026-07-23", batch_id);
                    make_batch(vec![first.clone(), record], "2026-07-24")
                },
                "source_at differs from period_at",
            ),
            (
                make_batch(vec![second.clone(), second.clone()], "2026-07-24"),
                "unique and strictly increasing",
            ),
            (
                {
                    let mut record = second.clone();
                    record.main_net = Some(Money::new(31.0).unwrap());
                    make_batch(vec![first.clone(), record], "2026-07-24")
                },
                "contradicts super_large_net + large_net",
            ),
            (
                make_batch(vec![first.clone(), second.clone()], "2026-07-23"),
                "batch source_at does not equal latest period",
            ),
        ];
        for (batch, message) in cases {
            let error =
                admit_fund_flow_batch("TEST_CODE_600000", &instrument, &request, batch, current)
                    .unwrap_err();
            assert_eq!(error.reason_code(), "invalid_evidence");
            assert!(error.to_string().contains(message));
        }
    }

    #[test]
    fn provider_top_n_admission_rejects_partial_order_date_identity_and_source_at() {
        let observed_at = "2026-07-24T15:36:00+08:00";
        let batch_id = "TEST_CODE_PROVIDER_TOP_N";
        let (request, valid) = provider_top_n_fixture(
            MarketRankingKind::VolumeRatio,
            "2026-07-24",
            2,
            observed_at,
            batch_id,
        );
        let source = EastmoneyClient::provider_top_n_source_identity().unwrap();
        assert!(admit_provider_top_n_batch(
            &request,
            valid.clone(),
            ProviderTopNRankingCapabilities {
                volume_ratio: true,
                main_net_inflow: true,
            },
            &source,
            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
        )
        .is_ok());

        let partial = DataBatch::best_effort(
            valid.records().to_vec(),
            valid.provenance().clone(),
            vec!["TEST_CODE_PARTIAL".to_string()],
        )
        .unwrap();
        let mut reversed_records = valid.records().to_vec();
        reversed_records.reverse();
        let reversed = DataBatch::strict(reversed_records, valid.provenance().clone());
        let wrong_date_request = EastmoneyClient::provider_top_n_a_share_request(
            MarketRankingKind::VolumeRatio,
            IsoDate::new("2026-07-25").unwrap(),
            PositiveU32::new(2).unwrap(),
        )
        .unwrap();
        let mut duplicate_records = valid.records().to_vec();
        duplicate_records[1] = duplicate_records[0].clone();
        let duplicate = DataBatch::strict(duplicate_records, valid.provenance().clone());
        let with_source_at = DataBatch::strict(
            valid.records().to_vec(),
            Provenance::new("eastmoney-web", observed_at)
                .unwrap()
                .with_source_at("2026-07-24T15:35:00+08:00")
                .unwrap()
                .with_batch_id(batch_id)
                .unwrap(),
        );

        for (candidate_request, batch) in [
            (&request, partial),
            (&request, reversed),
            (&wrong_date_request, valid.clone()),
            (&request, duplicate),
            (&request, with_source_at),
        ] {
            let error = admit_provider_top_n_batch(
                candidate_request,
                batch,
                ProviderTopNRankingCapabilities {
                    volume_ratio: true,
                    main_net_inflow: true,
                },
                &source,
                PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
            )
            .unwrap_err();
            assert_eq!(error.reason_code(), "invalid_evidence");
            assert!(!error.retryable());
        }
    }

    #[test]
    fn provider_top_n_pair_is_atomic_and_rejects_empty_or_metric_drift() {
        let observed_at = "2026-07-24T15:36:00+08:00";
        let (volume_request, volume_batch) = provider_top_n_fixture(
            MarketRankingKind::VolumeRatio,
            "2026-07-24",
            2,
            observed_at,
            "TEST_CODE_VOLUME_PAIR",
        );
        let (inflow_request, inflow_batch) = provider_top_n_fixture(
            MarketRankingKind::MainNetInflow,
            "2026-07-24",
            2,
            observed_at,
            "TEST_CODE_INFLOW_PAIR",
        );
        let source = EastmoneyClient::provider_top_n_source_identity().unwrap();
        let capabilities = ProviderTopNRankingCapabilities {
            volume_ratio: true,
            main_net_inflow: true,
        };
        let volume = admit_provider_top_n_batch(
            &volume_request,
            volume_batch,
            capabilities,
            &source,
            PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
        )
        .unwrap();
        let inflow = admit_provider_top_n_batch(
            &inflow_request,
            inflow_batch,
            capabilities,
            &source,
            PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
        )
        .unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 7, 24).unwrap();
        validate_provider_top_n_pair(&volume, &inflow, date).unwrap();

        let empty = super::BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: "eastmoney-web".to_string(),
            source_at: None,
            observed_at: observed_at.to_string(),
            batch_id: "TEST_CODE_EMPTY".to_string(),
        };
        let error =
            validate_provider_top_n_pair(&volume, &super::GatewayBatch::VerifiedEmpty(empty), date)
                .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());

        let metric_drift = ProviderTopNPair {
            volume_ratio_request: provider_top_n_request_evidence(
                &volume_request,
                PROVIDER_TOP_N_VOLUME_RATIO_CAPABILITY,
            ),
            volume_ratio: inflow.clone(),
            main_net_inflow_request: provider_top_n_request_evidence(
                &inflow_request,
                PROVIDER_TOP_N_MAIN_NET_INFLOW_CAPABILITY,
            ),
            main_net_inflow: inflow,
        };
        assert!(validate_provider_top_n_pair(
            &metric_drift.volume_ratio,
            &metric_drift.main_net_inflow,
            date,
        )
        .is_err());
    }

    #[test]
    fn northbound_admission_rejects_cardinality_request_and_record_evidence_drift() {
        let date = IsoDate::new("2026-07-24").unwrap();
        let request = NorthboundDailyRequest::new(date, NorthboundChannel::Shanghai);
        let observed_at = "2026-07-24T07:59:55Z";
        let source_at = "2026-07-24";
        let batch_id = "TEST_CODE_NORTHBOUND";
        let valid = northbound_record(
            NorthboundChannel::Shanghai,
            NorthboundQuotaBalance::Amount(Money::new(88.0).unwrap()),
            ProviderId::Hkex,
            observed_at,
            source_at,
            batch_id,
        );
        let make_batch = |records| {
            DataBatch::strict(
                records,
                provenance("hkex-official", observed_at, source_at, batch_id),
            )
        };
        let current = now("2026-07-24T08:00:00Z");
        let admitted =
            admit_northbound_batch(&request, make_batch(vec![valid.clone()]), current).unwrap();
        assert_eq!(
            admitted.records()[0].quota_balance,
            NorthboundQuotaFact::Amount(88.0)
        );

        let cases = [
            (make_batch(Vec::new()), "exactly one channel"),
            (
                make_batch(vec![valid.clone(), valid.clone()]),
                "exactly one channel",
            ),
            (
                make_batch(vec![northbound_record(
                    NorthboundChannel::Shenzhen,
                    NorthboundQuotaBalance::Unavailable,
                    ProviderId::Hkex,
                    observed_at,
                    source_at,
                    batch_id,
                )]),
                "identity/evidence differs from request",
            ),
            (
                make_batch(vec![northbound_record(
                    NorthboundChannel::Shanghai,
                    NorthboundQuotaBalance::Unavailable,
                    ProviderId::Eastmoney,
                    observed_at,
                    source_at,
                    batch_id,
                )]),
                "record provider/batch/observation/source evidence differs from batch",
            ),
            (
                make_batch(vec![northbound_record(
                    NorthboundChannel::Shanghai,
                    NorthboundQuotaBalance::Unavailable,
                    ProviderId::Hkex,
                    observed_at,
                    source_at,
                    "TEST_CODE_OTHER_BATCH",
                )]),
                "record provider/batch/observation/source evidence differs from batch",
            ),
        ];
        for (batch, message) in cases {
            let error = admit_northbound_batch(&request, batch, current).unwrap_err();
            assert_eq!(error.reason_code(), "invalid_evidence");
            assert!(error.to_string().contains(message));
        }
    }

    #[test]
    fn capital_freshness_boundaries_are_inclusive_and_future_values_are_rejected() {
        let current = now("2026-07-24T08:00:00Z");
        assert!(validate_observation_freshness(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            "2026-07-24T07:59:30Z",
            current,
        )
        .is_ok());
        let stale = validate_observation_freshness(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            "2026-07-24T07:59:29.999Z",
            current,
        )
        .unwrap_err();
        assert_eq!(stale.reason_code(), "capital_observation_stale");
        assert!(stale.retryable());
        let future = validate_observation_freshness(
            FUND_FLOW_CAPABILITY,
            ProviderId::Eastmoney,
            "2026-07-24T08:00:00.001Z",
            current,
        )
        .unwrap_err();
        assert_eq!(future.reason_code(), "capital_observation_stale");
        assert!(future.retryable());

        assert!(validate_fund_flow_freshness(
            FlowInterval::Minute1,
            "2026-07-24 16:00",
            now("2026-07-24T08:02:00Z"),
        )
        .is_ok());
        let stale_minute = validate_fund_flow_freshness(
            FlowInterval::Minute1,
            "2026-07-24 16:00",
            now("2026-07-24T08:02:00.001Z"),
        )
        .unwrap_err();
        assert_eq!(stale_minute.reason_code(), "minute_fund_flow_stale");
        assert!(stale_minute.retryable());
        assert!(
            validate_fund_flow_freshness(FlowInterval::Minute1, "2026-07-24 16:01", current,)
                .is_err()
        );
        assert!(validate_fund_flow_freshness(FlowInterval::Day5, "2026-07-24", current).is_err());
    }

    #[test]
    fn capital_provider_classifiers_cover_every_failure_category_and_action() {
        let eastmoney = [
            (
                EastmoneyError::InvalidRequest("TEST_CODE".to_owned()),
                FailureKind::InvalidRequest,
                FailureAction::Stop,
            ),
            (
                EastmoneyError::Unsupported("TEST_CODE".to_owned()),
                FailureKind::Unsupported,
                FailureAction::Stop,
            ),
            (
                EastmoneyError::Transport("TEST_CODE".to_owned()),
                FailureKind::Transport,
                FailureAction::TryNext,
            ),
            (
                verified_empty_error(),
                FailureKind::NoData,
                FailureAction::TryNext,
            ),
            (
                EastmoneyError::ResponseTooLarge { limit: 1 },
                FailureKind::Protocol,
                FailureAction::TryNext,
            ),
            (
                EastmoneyError::Decode("TEST_CODE".to_owned()),
                FailureKind::Protocol,
                FailureAction::TryNext,
            ),
            (
                EastmoneyError::Protocol("TEST_CODE".to_owned()),
                FailureKind::Protocol,
                FailureAction::TryNext,
            ),
            (
                EastmoneyError::Core(crate::market_domain::CoreError::InvalidRequest(
                    "TEST_CODE".to_owned(),
                )),
                FailureKind::Quality,
                FailureAction::TryNext,
            ),
        ];
        for (error, expected_kind, expected_action) in eastmoney {
            let classified = classify_eastmoney_error(error);
            assert_eq!(classified.kind(), expected_kind);
            assert_eq!(classified.action(), expected_action);
        }

        let exchange = [
            (
                ExchangeError::InvalidRequest("TEST_CODE".to_owned()),
                FailureKind::InvalidRequest,
                FailureAction::Stop,
            ),
            (
                ExchangeError::Unsupported("TEST_CODE".to_owned()),
                FailureKind::Unsupported,
                FailureAction::Stop,
            ),
            (
                ExchangeError::Authentication(403),
                FailureKind::Provider,
                FailureAction::Stop,
            ),
            (
                ExchangeError::HttpStatus(503),
                FailureKind::Provider,
                FailureAction::Stop,
            ),
            (
                ExchangeError::RateLimited,
                FailureKind::RateLimited,
                FailureAction::TryNext,
            ),
            (
                ExchangeError::Transport("TEST_CODE".to_owned()),
                FailureKind::Transport,
                FailureAction::TryNext,
            ),
            (
                ExchangeError::Tls {
                    backend: TlsBackend::Rustls,
                    message: "TEST_CODE".to_owned(),
                },
                FailureKind::Transport,
                FailureAction::TryNext,
            ),
            (
                ExchangeError::Decode("TEST_CODE".to_owned()),
                FailureKind::Protocol,
                FailureAction::TryNext,
            ),
            (
                ExchangeError::Schema("TEST_CODE".to_owned()),
                FailureKind::Protocol,
                FailureAction::TryNext,
            ),
            (
                ExchangeError::Incomplete("TEST_CODE".to_owned()),
                FailureKind::Quality,
                FailureAction::TryNext,
            ),
            (
                ExchangeError::Core(crate::market_domain::CoreError::InvalidRequest(
                    "TEST_CODE".to_owned(),
                )),
                FailureKind::Quality,
                FailureAction::TryNext,
            ),
        ];
        for (error, expected_kind, expected_action) in exchange {
            let classified = classify_exchange_error(error);
            assert_eq!(classified.kind(), expected_kind);
            assert_eq!(classified.action(), expected_action);
        }
    }

    #[test]
    fn gateway_error_mapping_preserves_outcome_reason_and_retryability() {
        let eastmoney = [
            (
                EastmoneyError::InvalidRequest("TEST_CODE".to_owned()),
                "invalid_request",
                "invalid_request",
                false,
            ),
            (
                EastmoneyError::Unsupported("TEST_CODE".to_owned()),
                "unsupported",
                "unsupported",
                false,
            ),
            (
                EastmoneyError::Transport("TEST_CODE".to_owned()),
                "unavailable",
                "transport",
                true,
            ),
            (
                EastmoneyError::ResponseTooLarge { limit: 1 },
                "unavailable",
                "response_too_large",
                true,
            ),
            (
                verified_empty_error(),
                "verified_empty",
                "verified_empty",
                false,
            ),
            (
                EastmoneyError::Decode("TEST_CODE".to_owned()),
                "partial",
                "decode",
                false,
            ),
            (
                EastmoneyError::Protocol("TEST_CODE".to_owned()),
                "partial",
                "protocol",
                false,
            ),
            (
                EastmoneyError::Core(crate::market_domain::CoreError::InvalidRequest(
                    "TEST_CODE".to_owned(),
                )),
                "partial",
                "core",
                false,
            ),
        ];
        for (source, outcome, reason, retryable) in eastmoney {
            let error = eastmoney_gateway_error(FUND_FLOW_CAPABILITY, source);
            assert_eq!(error.audit_outcome(), outcome);
            assert_eq!(error.reason_code(), reason);
            assert_eq!(error.retryable(), retryable);
        }

        let exchange = [
            (
                ExchangeError::InvalidRequest("TEST_CODE".to_owned()),
                "invalid_request",
                "invalid_request",
                false,
            ),
            (
                ExchangeError::Unsupported("TEST_CODE".to_owned()),
                "unsupported",
                "provider_unsupported",
                false,
            ),
            (
                ExchangeError::RateLimited,
                "unavailable",
                "provider_transport",
                true,
            ),
            (
                ExchangeError::Transport("TEST_CODE".to_owned()),
                "unavailable",
                "provider_transport",
                true,
            ),
            (
                ExchangeError::Tls {
                    backend: TlsBackend::NativeTls,
                    message: "TEST_CODE".to_owned(),
                },
                "unavailable",
                "provider_transport",
                true,
            ),
            (
                ExchangeError::Authentication(403),
                "unavailable",
                "provider_rejected",
                false,
            ),
            (
                ExchangeError::HttpStatus(503),
                "unavailable",
                "provider_rejected",
                false,
            ),
            (
                ExchangeError::Decode("TEST_CODE".to_owned()),
                "partial",
                "provider_invalid_batch",
                false,
            ),
            (
                ExchangeError::Schema("TEST_CODE".to_owned()),
                "partial",
                "provider_invalid_batch",
                false,
            ),
            (
                ExchangeError::Incomplete("TEST_CODE".to_owned()),
                "partial",
                "provider_invalid_batch",
                false,
            ),
            (
                ExchangeError::Core(crate::market_domain::CoreError::InvalidRequest(
                    "TEST_CODE".to_owned(),
                )),
                "partial",
                "provider_invalid_batch",
                false,
            ),
        ];
        for (source, outcome, reason, retryable) in exchange {
            let error = exchange_gateway_error(NORTHBOUND_CAPABILITY, source);
            assert_eq!(error.audit_outcome(), outcome);
            assert_eq!(error.reason_code(), reason);
            assert_eq!(error.retryable(), retryable);
        }

        let router = [
            (
                RouterError::InvalidConfiguration("TEST_CODE".to_owned()),
                "invalid_request",
                "router_invalid_request",
                false,
            ),
            (
                routed_error(FailureKind::InvalidRequest, FailureAction::Stop),
                "invalid_request",
                "router_invalid_request",
                false,
            ),
            (
                routed_error(FailureKind::Unsupported, FailureAction::Stop),
                "unsupported",
                "provider_unsupported",
                false,
            ),
            (
                routed_error(FailureKind::Transport, FailureAction::TryNext),
                "unavailable",
                "provider_unavailable",
                true,
            ),
            (
                routed_error(FailureKind::Timeout, FailureAction::TryNext),
                "unavailable",
                "provider_unavailable",
                true,
            ),
            (
                routed_error(FailureKind::RateLimited, FailureAction::TryNext),
                "unavailable",
                "provider_unavailable",
                true,
            ),
            (
                routed_error(FailureKind::NoData, FailureAction::TryNext),
                "unavailable",
                "provider_unavailable",
                true,
            ),
            (
                routed_error(FailureKind::Protocol, FailureAction::TryNext),
                "partial",
                "provider_batch_rejected",
                false,
            ),
            (
                routed_error(FailureKind::Quality, FailureAction::TryNext),
                "partial",
                "provider_batch_rejected",
                false,
            ),
            (
                routed_error(FailureKind::Evidence, FailureAction::TryNext),
                "partial",
                "provider_batch_rejected",
                false,
            ),
            (
                routed_error(FailureKind::Provider, FailureAction::TryNext),
                "partial",
                "provider_batch_rejected",
                false,
            ),
        ];
        for (source, outcome, reason, retryable) in router {
            let error = router_gateway_error(FUND_FLOW_CAPABILITY, ProviderId::Eastmoney, source);
            assert_eq!(error.audit_outcome(), outcome);
            assert_eq!(error.reason_code(), reason);
            assert_eq!(error.retryable(), retryable);
        }
    }
}
