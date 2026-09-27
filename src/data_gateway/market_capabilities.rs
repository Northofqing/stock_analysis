//! BR-164 evidence-preserving market capability gateways.
//!
//! Provider order is part of the versioned remote contract. A source can win
//! only with an identity-consistent batch carrying every field and evidence
//! item required by the consumer. Missing fields never become zeroes.

use crate::market_domain::ProviderId;

use chrono::{DateTime, NaiveDate, Utc};

use super::review::{
    acquisition_request_hash, audit_gateway_result_with_receipt, audit_routed_gateway_result,
    AuditedGatewayBatch, GatewayBatch, GatewayError,
};

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
                return audit_routed_gateway_result(MINUTE_CAPABILITY, &request_hash, result);
            }
            Err(error) => {
                return audit_routed_gateway_result(MINUTE_CAPABILITY, &request_hash, Err(error));
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
                return audit_routed_gateway_result(ORDER_BOOK_CAPABILITY, &request_hash, result);
            }
            Err(error) => {
                return audit_routed_gateway_result(
                    ORDER_BOOK_CAPABILITY,
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
                return audit_routed_gateway_result(MONEY_FLOW_CAPABILITY, &request_hash, result);
            }
            Err(error) => {
                return audit_routed_gateway_result(
                    MONEY_FLOW_CAPABILITY,
                    &request_hash,
                    Err(error),
                );
            }
        }
    }

    /// Returns a request-bound legacy metadata batch when the configured
    /// bridge supports it. This view does not prove qualified trading facts.
    pub async fn security_metadata(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
        let storage_codes = codes.to_vec();
        let request_hash = acquisition_request_hash(METADATA_CAPABILITY, storage_codes.join(","));
        if let Err(error) = validate_security_metadata_request(&storage_codes) {
            return audit_routed_gateway_result(METADATA_CAPABILITY, &request_hash, Err(error));
        }
        // P4 M2 钩子: gRPC 通道 (fail-closed, audit 对等; library 路径仍是
        // unsupported_security_metadata 显式错误)。
        match super::grpc_source::bridge_for("SecurityMetadata") {
            Ok(bridge) => {
                let result = bridge
                    .security_metadata_async(&storage_codes)
                    .await
                    .and_then(|batch| admit_requested_security_metadata(&storage_codes, batch));
                return audit_routed_gateway_result(METADATA_CAPABILITY, &request_hash, result);
            }
            Err(error) => {
                return audit_routed_gateway_result(METADATA_CAPABILITY, &request_hash, Err(error));
            }
        }
    }

    /// Fetches only the source-backed security identity subset needed by
    /// watchlist admission and delisting-name checks.
    pub async fn security_identities(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<MarketSecurityIdentity>, GatewayError> {
        self.security_identities_observation(codes)
            .await
            .map(AuditedGatewayBatch::into_batch)
    }

    pub(crate) async fn security_identities_observation(
        &self,
        codes: &[String],
    ) -> Result<AuditedGatewayBatch<MarketSecurityIdentity>, GatewayError> {
        let storage_codes = codes.to_vec();
        // BR-238: identity is a narrow projection of the authenticated
        // ExternalV1 SecurityMetadata contract. A configured bridge failure is
        // audited and returned; it never falls back to a different provider.
        // No-feature builds have no library transport. Without the bridge,
        // fail explicitly rather than fabricating an identity.
        let result = match super::grpc_source::bridge_for("SecurityMetadata") {
            Ok(bridge) => bridge.security_identities_async(&storage_codes).await,
            Err(error) => Err(error),
        };
        retain_security_identities_observation(&storage_codes, result, |provider, hash, result| {
            audit_gateway_result_with_receipt(SECURITY_IDENTITY_CAPABILITY, provider, hash, result)
        })
    }
}

fn validate_security_metadata_request(codes: &[String]) -> Result<(), GatewayError> {
    if codes.is_empty() {
        return Err(GatewayError::invalid_request(
            METADATA_CAPABILITY,
            "security metadata request must contain at least one code",
        ));
    }
    if codes.iter().any(|code| code.trim().is_empty()) {
        return Err(GatewayError::invalid_request(
            METADATA_CAPABILITY,
            "security metadata request contains an empty code",
        ));
    }
    let mut unique = std::collections::HashSet::with_capacity(codes.len());
    if codes.iter().any(|code| !unique.insert(code.as_str())) {
        return Err(GatewayError::invalid_request(
            METADATA_CAPABILITY,
            "security metadata request contains duplicate codes",
        ));
    }
    Ok(())
}

fn admit_requested_security_metadata(
    codes: &[String],
    batch: GatewayBatch<MarketSecurityMetadata>,
) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
    let (records, evidence) = match &batch {
        GatewayBatch::Available { records, evidence } => (records, evidence),
        GatewayBatch::VerifiedEmpty(evidence) => {
            return Err(GatewayError::invalid_evidence(
                METADATA_CAPABILITY,
                Some(evidence.provider),
                "security metadata response is empty for a non-empty request",
            ));
        }
    };
    let mut remaining = codes
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    if records.len() != codes.len()
        || records
            .iter()
            .any(|record| !remaining.remove(record.code.as_str()))
        || !remaining.is_empty()
    {
        return Err(GatewayError::invalid_evidence(
            METADATA_CAPABILITY,
            Some(evidence.provider),
            "security metadata response must contain exactly one record for each requested code",
        ));
    }
    Ok(batch)
}

fn retain_security_identities_observation<Audit>(
    codes: &[String],
    result: Result<GatewayBatch<MarketSecurityIdentity>, GatewayError>,
    audit: Audit,
) -> Result<AuditedGatewayBatch<MarketSecurityIdentity>, GatewayError>
where
    Audit: FnOnce(
        ProviderId,
        &str,
        Result<GatewayBatch<MarketSecurityIdentity>, GatewayError>,
    ) -> Result<
        (
            GatewayBatch<MarketSecurityIdentity>,
            crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
        ),
        GatewayError,
    >,
{
    let request_hash = acquisition_request_hash(SECURITY_IDENTITY_CAPABILITY, codes.join(","));
    let audit_provider = security_identity_audit_provider(&result);
    let (batch, receipt) = audit(audit_provider, &request_hash, result)?;
    Ok(AuditedGatewayBatch::new(batch, request_hash, receipt))
}

fn security_identity_audit_provider(
    result: &Result<GatewayBatch<MarketSecurityIdentity>, GatewayError>,
) -> ProviderId {
    // D15 receipt-owner allowlist: identical evidence/error/Custom routing,
    // retaining its immutable observation receipt and original request hash.
    match result {
        Ok(batch) => batch.evidence().provider,
        Err(error) => error.provider().unwrap_or(ProviderId::Custom),
    }
}

#[cfg(test)]
pub(crate) fn security_identities_observation_in(
    database: &crate::database::DatabaseManager,
    codes: &[String],
    result: Result<GatewayBatch<MarketSecurityIdentity>, GatewayError>,
) -> Result<AuditedGatewayBatch<MarketSecurityIdentity>, GatewayError> {
    retain_security_identities_observation(codes, result, |provider, hash, result| {
        super::review::audit_gateway_result_with_receipt_in(
            database,
            SECURITY_IDENTITY_CAPABILITY,
            provider,
            hash,
            result,
        )
    })
}

#[cfg(test)]
mod observation_tests {
    use super::*;
    use crate::data_gateway::BatchEvidence;
    use crate::database::attribution_reports::{
        AttributionDatabaseAccess, AttributionDatabaseSession,
    };
    use crate::database::DatabaseManager;
    use diesel::prelude::*;
    use diesel::sql_types::{BigInt, Text};

    #[derive(QueryableByName)]
    struct IdentityAuditRow {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        provider: String,
        #[diesel(sql_type = Text)]
        request_hash: String,
        #[diesel(sql_type = Text)]
        record_hash: String,
    }

    #[derive(QueryableByName)]
    struct AuditProviderRow {
        #[diesel(sql_type = Text)]
        provider: String,
    }

    #[test]
    fn metadata_gateway_requires_exact_requested_identity_set() {
        let requested = vec!["600519".to_owned()];
        let timestamp = DateTime::<Utc>::from_timestamp(1_790_300_000, 0).unwrap();
        let evidence = BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE metadata source".to_owned(),
            source_at: Some(timestamp.to_rfc3339()),
            observed_at: timestamp.to_rfc3339(),
            batch_id: "TEST_CODE_metadata_batch".to_owned(),
        };
        let record = |code: &str| MarketSecurityMetadata {
            code: code.to_owned(),
            name: format!("TEST_CODE_{code}"),
            board: SecurityBoard::Main,
            is_st: false,
            listed_on: NaiveDate::from_ymd_opt(2001, 8, 27).unwrap(),
            price_limit_percent: 10.0,
            price_limit_version: String::new(),
            source_at: timestamp,
            observed_at: timestamp,
            provider: ProviderId::Tdx,
            batch_id: evidence.batch_id.clone(),
        };
        for (label, codes, admitted) in [
            ("A", vec!["600519"], true),
            ("B", vec!["600000"], false),
            ("A+B", vec!["600519", "600000"], false),
            ("duplicate A", vec!["600519", "600519"], false),
            ("empty available", vec![], false),
        ] {
            let batch = GatewayBatch::Available {
                records: codes.into_iter().map(&record).collect(),
                evidence: evidence.clone(),
            };
            assert_eq!(
                admit_requested_security_metadata(&requested, batch).is_ok(),
                admitted,
                "{label}"
            );
        }
        assert!(admit_requested_security_metadata(
            &requested,
            GatewayBatch::VerifiedEmpty(evidence.clone())
        )
        .is_err());
        assert!(validate_security_metadata_request(&[]).is_err());
        assert!(validate_security_metadata_request(&[" ".into()]).is_err());
        assert!(validate_security_metadata_request(&["600519".into(), "600519".into()]).is_err());

        let requested_pair = vec!["600519".to_owned(), "600000".to_owned()];
        let reversed = GatewayBatch::Available {
            records: vec![record("600000"), record("600519")],
            evidence,
        };
        assert!(admit_requested_security_metadata(&requested_pair, reversed).is_ok());
    }

    #[tokio::test]
    async fn metadata_bridge_failure_without_provider_is_audited_as_custom() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();

        let result = MarketCapabilitiesGateway::new()
            .security_metadata(&["399993".to_owned()])
            .await;

        std::env::remove_var("GRPC_MARKET_ADDR");
        super::super::grpc_source::reset_bridge();

        let error = result.expect_err("unreachable bridge must fail closed");
        assert_eq!(error.provider(), None);

        let request_hash = acquisition_request_hash(METADATA_CAPABILITY, "399993");
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT provider FROM data_acquisition_audit \
             WHERE capability = 'SecurityMetadata' AND request_hash = ? \
             ORDER BY id DESC LIMIT 1",
        )
        .bind::<Text, _>(request_hash)
        .get_result::<AuditProviderRow>(&mut *connection)
        .expect("bridge failure must be audited");
        assert_eq!(row.provider, "Custom");
    }

    #[test]
    fn br221_observation_identity_retains_exact_request_provider_and_compatibility_projection() {
        let file = tempfile::NamedTempFile::new().expect("TEST_CODE identity audit database");
        let session =
            AttributionDatabaseSession::open(file.path(), AttributionDatabaseAccess::AppendOnly)
                .expect("TEST_CODE identity append-only database");
        let database = session.database();
        let codes = vec!["TEST_CODE_600001".to_owned(), "TEST_CODE_600002".to_owned()];
        let source_at = DateTime::parse_from_rfc3339("2099-01-02T10:00:00+08:00")
            .unwrap()
            .with_timezone(&Utc);
        let observed_at = DateTime::parse_from_rfc3339("2099-01-02T10:00:01+08:00")
            .unwrap()
            .with_timezone(&Utc);
        let expected_batch = GatewayBatch::Available {
            records: codes
                .iter()
                .map(|code| MarketSecurityIdentity {
                    code: code.clone(),
                    name: format!("Name {code}"),
                    is_st: false,
                    source_at: source_at + chrono::Duration::seconds(3),
                    observed_at,
                    provider: ProviderId::Sina,
                    batch_id: "TEST_CODE_identity_retention".to_owned(),
                })
                .collect(),
            evidence: BatchEvidence {
                provider: ProviderId::Sina,
                source: "TEST_CODE identity provider".to_owned(),
                source_at: Some("2099-01-02T10:00:00+08:00".to_owned()),
                observed_at: "2099-01-02T10:00:01+08:00".to_owned(),
                batch_id: "TEST_CODE_identity_retention".to_owned(),
            },
        };

        let observed =
            security_identities_observation_in(database, &codes, Ok(expected_batch.clone()))
                .expect("TEST_CODE audited identity observation");
        let expected_request_hash =
            "cefec6bc3c228366e37eb3445afeaa17f60b56a0e5b2b795a4296ddea2e6400e";
        assert_eq!(observed.batch(), &expected_batch);
        assert_eq!(observed.request_hash(), expected_request_hash);

        let mut connection = database.get_conn().expect("TEST_CODE audit connection");
        let row = diesel::sql_query(
            "SELECT audit.id,audit.provider,audit.request_hash,chain.record_hash \
             FROM data_acquisition_audit AS audit \
             JOIN data_acquisition_audit_chain AS chain \
               ON chain.acquisition_audit_id=audit.id \
             WHERE audit.capability='SecurityIdentity'",
        )
        .get_result::<IdentityAuditRow>(&mut connection)
        .expect("TEST_CODE identity audit row");
        assert_eq!(row.provider, "Sina");
        assert_eq!(row.request_hash, expected_request_hash);
        assert_eq!(row.id, observed.receipt().audit_id);
        assert_eq!(row.record_hash, observed.receipt().record_hash);
        let before_projection = diesel::sql_query(
            "SELECT audit.id,audit.provider,audit.request_hash,chain.record_hash \
             FROM data_acquisition_audit AS audit \
             JOIN data_acquisition_audit_chain AS chain \
               ON chain.acquisition_audit_id=audit.id \
             WHERE audit.capability='SecurityIdentity'",
        )
        .load::<IdentityAuditRow>(&mut connection)
        .expect("TEST_CODE identity audit rows")
        .len();
        drop(connection);

        let compatible_batch = observed.into_batch();
        assert_eq!(compatible_batch, expected_batch);
        let mut connection = database.get_conn().expect("TEST_CODE audit connection");
        let after_projection = diesel::sql_query(
            "SELECT audit.id,audit.provider,audit.request_hash,chain.record_hash \
             FROM data_acquisition_audit AS audit \
             JOIN data_acquisition_audit_chain AS chain \
               ON chain.acquisition_audit_id=audit.id \
             WHERE audit.capability='SecurityIdentity'",
        )
        .load::<IdentityAuditRow>(&mut connection)
        .expect("TEST_CODE identity audit rows")
        .len();
        assert_eq!(after_projection, before_projection);
    }
}
