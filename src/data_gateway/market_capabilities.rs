//! BR-164 evidence-preserving market capability gateways.
//!
//! Provider order is part of the versioned remote contract. A source can win
//! only with an identity-consistent batch carrying every field and evidence
//! item required by the consumer. Missing fields never become zeroes.

use crate::market_domain::ProviderId;

use chrono::{DateTime, NaiveDate, Utc};

use super::review::{acquisition_request_hash, audit_gateway_result, GatewayBatch, GatewayError};

const MINUTE_CAPABILITY: &str = "MarketMinuteData";
const ORDER_BOOK_CAPABILITY: &str = "MarketOrderBooks";
const MONEY_FLOW_CAPABILITY: &str = "MarketMoneyFlows";
const METADATA_CAPABILITY: &str = "SecurityMetadata";
const SECURITY_IDENTITY_CAPABILITY: &str = "SecurityIdentity";
const REALTIME_MAX_AGE_MILLIS: i64 = 5_000;
const ACQUISITION_MAX_AGE_MILLIS: i64 = 30_000;
const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;

/// Actual upstream source order for current and historical minute data.
pub const MINUTE_PROVIDER_ORDER: &[ProviderId] =
    &[ProviderId::Tdx, ProviderId::Tencent, ProviderId::Sina];
/// Actual upstream source order for five-level order books.
///
/// Pinned TDX currently lacks an auditable source timestamp, so strict routing
/// rejects its batch and continues instead of pretending it is current.
pub const ORDER_BOOK_PROVIDER_ORDER: &[ProviderId] =
    &[ProviderId::Tdx, ProviderId::Tencent, ProviderId::Sina];
/// The only implemented normalized money-flow provider in the upstream
/// workspace is the separately licensed EMQuant adapter (Eastmoney identity).
pub const MONEY_FLOW_PROVIDER_ORDER: &[ProviderId] = &[ProviderId::Eastmoney];
/// Actual upstream source order for the source-backed security identity
/// subset (name, ST label and source evidence).
///
/// TDX is intentionally absent because its list packet has no source
/// timestamp. None of these providers is advertised as complete
/// security-master data.
pub const METADATA_PROVIDER_ORDER: &[ProviderId] = &[ProviderId::Tencent, ProviderId::Sina];

/// One admitted minute point. `cumulative_amount` remains optional because the
/// TDX protocol does not provide it; absence is preserved rather than filled.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketMinutePoint {
    pub code: String,
    pub minute_at: DateTime<Utc>,
    pub price: f64,
    pub cumulative_quantity: f64,
    pub cumulative_amount: Option<f64>,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// One complete order-book level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketBookLevel {
    pub price: f64,
    pub quantity: f64,
}

/// One admitted five-level order book.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketOrderBook {
    pub code: String,
    pub bids: [MarketBookLevel; 5],
    pub asks: [MarketBookLevel; 5],
    pub total_bid_quantity: f64,
    pub total_ask_quantity: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// One admitted normalized money-flow snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketMoneyFlow {
    pub code: String,
    pub main_net: f64,
    pub super_large_net: f64,
    pub large_net: f64,
    pub medium_net: f64,
    pub small_net: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Stable consumer-side board vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityBoard {
    Main,
    Star,
    ChiNext,
    Beijing,
}

/// One admitted complete security-master record.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketSecurityMetadata {
    pub code: String,
    pub name: String,
    pub board: SecurityBoard,
    pub is_st: bool,
    pub listed_on: NaiveDate,
    pub price_limit_percent: f64,
    pub price_limit_version: String,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Source-backed identity subset used by watchlist admission and delisting
/// name checks. This intentionally does not claim that listing date, board or
/// price-limit metadata is available.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketSecurityIdentity {
    pub code: String,
    pub name: String,
    pub is_st: bool,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Unified entry point for BR-164 market capability acquisition.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarketCapabilitiesGateway;

impl MarketCapabilitiesGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Fetches the current session (`date=None`) or one explicit historical
    /// session (`date=Some`) without blocking an async runtime worker.
    pub async fn minute_data(
        &self,
        code: &str,
        date: Option<NaiveDate>,
    ) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
        let storage_code = code.to_owned();
        let canonical = format!(
            "{storage_code}:{}",
            date.map(|value| value.to_string())
                .unwrap_or_else(|| "current".to_owned())
        );
        let request_hash = acquisition_request_hash(MINUTE_CAPABILITY, &canonical);
        // P4 M2 钩子: remote gRPC → gRPC 通道 (fail-closed, 连接失败
        // 也走 audit 对等, 不绕过 DataAcquisitionAuditRecord)。
        match super::grpc_source::bridge_for("MinuteData") {
            Ok(bridge) => {
                let result = bridge.minute_data_async(&storage_code).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(
                    MINUTE_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    MINUTE_CAPABILITY,
                    ProviderId::Tdx,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // P4 M5: no-feature 构建不携带 library transport, 无桥时显式失败
        // (fail-closed), 绝不静默回退。
    }

    /// Fetches complete, current five-level books for every requested code.
    pub async fn order_books(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash = acquisition_request_hash(ORDER_BOOK_CAPABILITY, storage_codes.join(","));
        // P4 M2 钩子: gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("OrderBooks") {
            Ok(bridge) => {
                let result = bridge.order_books_async(&storage_codes).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(
                    ORDER_BOOK_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    ORDER_BOOK_CAPABILITY,
                    ProviderId::Tdx,
                    &request_hash,
                    Err(error),
                );
            }
        }
    }

    /// Returns an explicit contract error until the separately licensed
    /// `magic-emquant-rs` provider is wired at the dependency boundary.
    ///
    /// TDX is deliberately not treated as a money-flow source: upstream marks
    /// this capability unsupported because its packets do not prove the
    /// standardized main/net-flow methodology.
    pub async fn money_flows(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash = acquisition_request_hash(MONEY_FLOW_CAPABILITY, storage_codes.join(","));
        // P4 M2 钩子: gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("MoneyFlows") {
            Ok(bridge) => {
                let result = bridge.money_flows_async(&storage_codes).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Eastmoney);
                return audit_gateway_result(
                    MONEY_FLOW_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    MONEY_FLOW_CAPABILITY,
                    ProviderId::Eastmoney,
                    &request_hash,
                    Err(error),
                );
            }
        }
    }

    /// Returns a typed non-retryable unsupported error because the pinned
    /// providers do not prove the complete security-master contract.
    pub async fn security_metadata(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash = acquisition_request_hash(METADATA_CAPABILITY, storage_codes.join(","));
        // P4 M2 钩子: gRPC 通道 (fail-closed, audit 对等; library 路径仍是
        // unsupported_security_metadata 显式错误)。
        match super::grpc_source::bridge_for("SecurityMetadata") {
            Ok(bridge) => {
                let result = bridge.security_metadata_async(&storage_codes).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(
                    METADATA_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    METADATA_CAPABILITY,
                    ProviderId::Tdx,
                    &request_hash,
                    Err(error),
                );
            }
        }
    }

    /// Fetches only the source-backed security identity subset needed by
    /// watchlist admission and delisting-name checks.
    pub async fn security_identities(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityIdentity>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash =
            acquisition_request_hash(SECURITY_IDENTITY_CAPABILITY, storage_codes.join(","));
        // BR-238: identity is a narrow projection of the authenticated
        // ExternalV1 SecurityMetadata contract. A configured bridge failure is
        // audited and returned; it never falls back to a different provider.
        match super::grpc_source::bridge_for("SecurityMetadata") {
            Ok(bridge) => {
                let result = bridge.security_identities_async(&storage_codes).await;
                let audit_provider = match &result {
                    Ok(batch) => batch.evidence().provider,
                    Err(error) => error.provider().unwrap_or(ProviderId::Custom),
                };
                return audit_gateway_result(
                    SECURITY_IDENTITY_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                let audit_provider = error.provider().unwrap_or(ProviderId::Custom);
                return audit_gateway_result(
                    SECURITY_IDENTITY_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature builds have no library transport. Without the bridge,
        // fail explicitly rather than fabricating an identity.
    }
}
