//! BR-164 evidence-preserving company and quote-statistics gateway.
//!
//! Financial statements retain the upstream normalized facts without flattening
//! line-item keys, units or explicit missing values. Market statistics retain
//! every optional field as `Option`; absence is never converted to zero.

use crate::market_domain::ProviderId;

pub use crate::market_domain::{
    FinancialLine, FinancialStatement, MarketStatistics, StatementKind,
};

use super::review::{acquisition_request_hash, audit_gateway_result, GatewayBatch, GatewayError};

const FINANCIAL_CAPABILITY: &str = "CompanyFinancialStatements";
const STATISTICS_CAPABILITY: &str = "CompanyMarketStatistics";
const REALTIME_MAX_AGE_MILLIS: i64 = 5_000;
const ACQUISITION_MAX_AGE_MILLIS: i64 = 30_000;
const SHANGHAI_OFFSET_SECONDS: i32 = 8 * 60 * 60;

/// Pinned upstream provider order for all three normalized statements.
pub const FINANCIAL_STATEMENT_PROVIDER_ORDER: &[ProviderId] = &[ProviderId::Sina];
/// Pinned upstream provider order for PE/PB/capitalization/trading statistics.
pub const MARKET_STATISTICS_PROVIDER_ORDER: &[ProviderId] = &[ProviderId::Tencent];

/// BR-164 gateway for normalized company facts.
#[derive(Debug, Clone, Copy, Default)]
pub struct CompanyDataGateway;

impl CompanyDataGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Fetches one statement family for every requested Shanghai/Shenzhen
    /// equity. Returned records are the original upstream strong types.
    pub async fn financial_statements(
        &self,
        codes: &[String],
        kind: StatementKind,
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash = acquisition_request_hash(
            FINANCIAL_CAPABILITY,
            format!("{kind:?}:{}", storage_codes.join(",")),
        );
        // P4 M4b: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("FinancialStatements") {
            Ok(bridge) => {
                let result = bridge
                    .financial_statements_async(&storage_codes, kind)
                    .await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Sina);
                return audit_gateway_result(
                    FINANCIAL_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    FINANCIAL_CAPABILITY,
                    ProviderId::Sina,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }

    pub async fn balance_sheets(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        self.financial_statements(codes, StatementKind::Balance)
            .await
    }

    pub async fn income_statements(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        self.financial_statements(codes, StatementKind::Income)
            .await
    }

    pub async fn cash_flow_statements(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
        self.financial_statements(codes, StatementKind::CashFlow)
            .await
    }

    /// Fetches Tencent quote-adjacent statistics. Optional source fields remain
    /// optional in the returned `MarketStatistics`.
    ///
    /// BR-205 is Gate-A evidence only: this request has no exact trading-session
    /// input, so its output is not yet authority for dynamic order-price limits.
    pub async fn market_statistics(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketStatistics>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash = acquisition_request_hash(STATISTICS_CAPABILITY, storage_codes.join(","));
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        match super::grpc_source::bridge_for("MarketStatistics") {
            Ok(bridge) => {
                let result = bridge.market_statistics_async(&storage_codes).await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tencent);
                return audit_gateway_result(
                    STATISTICS_CAPABILITY,
                    audit_provider,
                    &request_hash,
                    result,
                );
            }
            Err(error) => {
                return audit_gateway_result(
                    STATISTICS_CAPABILITY,
                    ProviderId::Tencent,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }
}
