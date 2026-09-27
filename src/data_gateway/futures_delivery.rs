//! BR-165/BR-199 evidence-preserving CFFEX futures-delivery acquisition.

use super::{GatewayBatch, GatewayError};

use chrono::NaiveDate;

const CAPABILITY: &str = "R-08-cffex-delivery";
pub const FUTURES_DELIVERY_CONTRACT_UNAVAILABLE_V1: &str =
    "futures_delivery_contract_unavailable_v1";

/// no-feature (monitor 零 magic): 进程内无 CffexClient, 契约无从读取。
/// 诚实声明 = false → 启动 banner 走 warn 分支 (出声, 与 remote gRPC
/// 下 gRPC 通道独立承载 R-08 交付不冲突)。

pub const fn cffex_futures_delivery_live_supported() -> bool {
    false
}

/// One admitted contract fact from an official CFFEX delivery notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuturesDeliveryFact {
    pub contract_code: String,
    pub product_code: String,
    pub last_trading_date: Option<NaiveDate>,
    pub delivery_date: NaiveDate,
    pub notice_url: String,
}

/// Production seam for the unified CFFEX official-notice provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct FuturesDeliveryGateway;

impl FuturesDeliveryGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn cffex_contract_month(
        &self,
        year: u32,
        month: u32,
    ) -> Result<GatewayBatch<FuturesDeliveryFact>, GatewayError> {
        if !(2000..=9999).contains(&year) || !(1..=12).contains(&month) {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                format!("invalid requested CFFEX contract month {year:04}-{month:02}"),
            ));
        }
        Err(GatewayError::classified(
            CAPABILITY,
            None,
            "unavailable",
            FUTURES_DELIVERY_CONTRACT_UNAVAILABLE_V1,
            false,
            format!(
                "CFFEX requested month {year:04}-{month:02} is blocked before business RPC: \
                 FuturesDeliveryRequest v1 scope, coverage and verified-empty semantics are not delivered"
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseManager;
    use diesel::{sql_types::BigInt, RunQueryDsl};

    #[derive(diesel::QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    fn audit_count() -> i64 {
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        diesel::sql_query(
            "SELECT COUNT(*) AS count FROM data_acquisition_audit WHERE capability = 'R-08-cffex-delivery'",
        )
        .get_result::<CountRow>(&mut *connection)
        .unwrap()
        .count
    }

    #[tokio::test]
    async fn task9_futures_delivery_contract_unavailable_has_zero_rpc_and_zero_success_audit() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).unwrap();
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();
        // Any attempted physical query panics because the queue is empty.
        super::super::grpc_source::set_test_query_responses(vec![]);
        let before = audit_count();

        let error = FuturesDeliveryGateway::new()
            .cffex_contract_month(2026, 9)
            .await
            .expect_err("missing request/coverage contract must fail closed");
        assert_eq!(
            error.reason_code(),
            "futures_delivery_contract_unavailable_v1"
        );
        assert!(error.message().contains("2026-09"));
        assert_eq!(
            audit_count(),
            before,
            "no success/failure RPC audit is legal"
        );

        for month in [0, 13] {
            let error = FuturesDeliveryGateway::new()
                .cffex_contract_month(2026, month)
                .await
                .expect_err("invalid month must fail before RPC");
            assert_eq!(error.reason_code(), "invalid_request");
        }
        assert_eq!(audit_count(), before);
    }
}
