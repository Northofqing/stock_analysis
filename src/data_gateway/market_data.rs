//! BR-064/BR-164/BR-172/BR-210/BR-213 realtime market-data boundary.
//!
//! The ordered route is Magic TDX -> Magic Tencent -> Magic Sina. A provider
//! can only win with a complete batch carrying provider source time. The TDX
//! quote contract at the currently pinned upstream revision does not prove a
//! second-level source timestamp, so the router correctly rejects that batch
//! under the five-second freshness rule and continues to the next Magic
//! provider. No consumer-owned HTTP or legacy parser is retained.

use crate::market_domain::ProviderId;

use chrono::{DateTime, NaiveDate, Utc};

use super::parse_evidence_instant;
use super::review::{
    acquisition_request_hash, audit_routed_gateway_result, BatchEvidence, GatewayBatch,
    GatewayError,
};

const CAPABILITY: &str = "RealtimeMarketQuotes";

/// BR-233 (2026-08-10): 实时行情准入模式。
/// - `RealtimeFiveSecond`: BR-218 盘中 5s 红线 — 默认, 所有盘中消费者不变。
/// - `SettledClose { trading_date }`: 盘后收盘静态快照 — 仅收市后消费者
///   (R-07 晚间明日观察池) 使用。收市后最后成交时间必然超龄 (Tencent/Sina
///   source_at = 最后成交时间, TDX 缺高精度 source_at), 但价格=当日收盘价,
///   是合法盘后快照; 时段未收盘 (盘中误调) 仍 fail-closed。
#[derive(Debug, Clone, Copy)]
enum QuoteAdmissionMode {
    RealtimeFiveSecond,
    SettledClose { trading_date: NaiveDate },
}

/// One admitted quote projection used by monitor consumers.
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeMarketQuote {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub previous_close: f64,
    pub change_percent: f64,
    pub source_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub batch_id: String,
}

/// Consumer-visible completeness for one requested realtime-quote set.
///
/// This is intentionally quote-specific: generic [`GatewayBatch`] keeps its
/// existing provider-envelope meaning, while this result additionally records
/// per-instrument coverage and local admission failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteCoverageDisposition {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteRecordRejection {
    pub code: Option<String>,
    pub reason_code: &'static str,
    pub message: String,
}

/// Sealed quote projection retaining the exact requested set and raw batch
/// lineage. Only failures attributable to one uniquely identified record are
/// isolated; envelope, provider, batch, timestamp and identity conflicts make
/// the whole result unavailable.
#[derive(Debug, Clone)]
pub struct RealtimeQuoteCoverage {
    disposition: QuoteCoverageDisposition,
    requested: Vec<String>,
    accepted: Vec<RealtimeMarketQuote>,
    rejected: Vec<QuoteRecordRejection>,
    missing: Vec<String>,
    evidence: Option<BatchEvidence>,
    unavailable_error: Option<GatewayError>,
}

impl RealtimeQuoteCoverage {
    pub fn classify(
        requested: &[String],
        result: Result<GatewayBatch<RealtimeMarketQuote>, GatewayError>,
    ) -> Self {
        use std::collections::HashSet;

        let requested_owned = requested.to_vec();
        let requested_set = requested.iter().map(String::as_str).collect::<HashSet<_>>();
        if requested.is_empty() || requested_set.len() != requested.len() {
            return Self::unavailable(
                requested_owned,
                Vec::new(),
                requested.to_vec(),
                None,
                GatewayError::invalid_request(
                    CAPABILITY,
                    "realtime quote coverage requires a non-empty unique requested set",
                ),
            );
        }

        let batch = match result {
            Ok(batch) => batch,
            Err(error) => {
                return Self::unavailable(
                    requested_owned,
                    Vec::new(),
                    requested.to_vec(),
                    None,
                    error,
                );
            }
        };
        let (records, evidence) = match batch {
            GatewayBatch::Available { records, evidence } if !records.is_empty() => {
                (records, evidence)
            }
            GatewayBatch::Available { evidence, .. } | GatewayBatch::VerifiedEmpty(evidence) => {
                return Self::unavailable(
                    requested_owned,
                    Vec::new(),
                    requested.to_vec(),
                    Some(evidence.clone()),
                    GatewayError::classified(
                        CAPABILITY,
                        Some(evidence.provider),
                        "unavailable",
                        "quote_coverage_unavailable",
                        true,
                        "provider returned no independently admitted realtime quote records",
                    ),
                );
            }
        };

        let mut returned = HashSet::with_capacity(records.len());
        for record in &records {
            if record.code.trim().is_empty()
                || !requested_set.contains(record.code.as_str())
                || !returned.insert(record.code.as_str())
            {
                return Self::unavailable(
                    requested_owned,
                    Vec::new(),
                    requested.to_vec(),
                    Some(evidence.clone()),
                    GatewayError::invalid_evidence(
                        CAPABILITY,
                        Some(evidence.provider),
                        format!(
                            "realtime quote response has an empty, duplicate or unrequested identity {:?}",
                            record.code
                        ),
                    ),
                );
            }
            if let Err(error) = validate_admitted_projection(record, &evidence) {
                return Self::unavailable(
                    requested_owned,
                    Vec::new(),
                    requested.to_vec(),
                    Some(evidence.clone()),
                    error,
                );
            }
        }

        let missing = requested
            .iter()
            .filter(|code| !returned.contains(code.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let mut accepted = Vec::with_capacity(records.len());
        let mut rejected = Vec::new();
        for record in records {
            let rejection = if record.name.trim().is_empty() {
                Some((
                    "invalid_quote_name",
                    "quote name must be a non-empty string".to_owned(),
                ))
            } else if !record.price.is_finite() || record.price <= 0.0 {
                Some((
                    "invalid_quote_price",
                    format!(
                        "quote price must be positive and finite, got {:?}",
                        record.price
                    ),
                ))
            } else if !record.previous_close.is_finite() || record.previous_close <= 0.0 {
                Some((
                    "invalid_previous_close",
                    format!(
                        "quote previous_close must be positive and finite, got {:?}",
                        record.previous_close
                    ),
                ))
            } else if !record.change_percent.is_finite() {
                Some((
                    "invalid_change_percent",
                    format!(
                        "quote change_percent must be finite, got {:?}",
                        record.change_percent
                    ),
                ))
            } else {
                None
            };
            if let Some((reason_code, message)) = rejection {
                rejected.push(QuoteRecordRejection {
                    code: Some(record.code),
                    reason_code,
                    message,
                });
            } else {
                accepted.push(record);
            }
        }

        let disposition = if accepted.is_empty() {
            QuoteCoverageDisposition::Unavailable
        } else if rejected.is_empty() && missing.is_empty() {
            QuoteCoverageDisposition::Complete
        } else {
            QuoteCoverageDisposition::Partial
        };
        let unavailable_error = (disposition == QuoteCoverageDisposition::Unavailable).then(|| {
            GatewayError::classified(
                CAPABILITY,
                Some(evidence.provider),
                "unavailable",
                "quote_coverage_unavailable",
                false,
                "no requested realtime quote passed record-local admission",
            )
        });
        Self {
            disposition,
            requested: requested_owned,
            accepted,
            rejected,
            missing,
            evidence: Some(evidence),
            unavailable_error,
        }
    }

    fn unavailable(
        requested: Vec<String>,
        rejected: Vec<QuoteRecordRejection>,
        missing: Vec<String>,
        evidence: Option<BatchEvidence>,
        error: GatewayError,
    ) -> Self {
        Self {
            disposition: QuoteCoverageDisposition::Unavailable,
            requested,
            accepted: Vec::new(),
            rejected,
            missing,
            evidence,
            unavailable_error: Some(error),
        }
    }

    pub const fn disposition(&self) -> QuoteCoverageDisposition {
        self.disposition
    }

    pub fn requested(&self) -> &[String] {
        &self.requested
    }

    pub fn accepted(&self) -> &[RealtimeMarketQuote] {
        &self.accepted
    }

    pub fn rejected(&self) -> &[QuoteRecordRejection] {
        &self.rejected
    }

    pub fn missing(&self) -> &[String] {
        &self.missing
    }

    pub const fn evidence(&self) -> Option<&BatchEvidence> {
        self.evidence.as_ref()
    }

    pub const fn unavailable_error(&self) -> Option<&GatewayError> {
        self.unavailable_error.as_ref()
    }

    pub fn require_complete(self) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        if self.disposition == QuoteCoverageDisposition::Complete {
            return Ok(GatewayBatch::Available {
                records: self.accepted,
                evidence: self
                    .evidence
                    .expect("complete quote coverage always retains batch evidence"),
            });
        }
        if let Some(error) = self.unavailable_error {
            return Err(error);
        }
        Err(GatewayError::classified(
            CAPABILITY,
            self.evidence.as_ref().map(|evidence| evidence.provider),
            "partial",
            "quote_coverage_incomplete",
            true,
            format!(
                "strict quote consumer requires complete coverage: rejected={} missing={}",
                self.rejected.len(),
                self.missing.len()
            ),
        ))
    }

    pub fn into_accepted(self) -> Vec<RealtimeMarketQuote> {
        self.accepted
    }
}

/// One realtime quote that cannot be separated from the audited source batch
/// that admitted it.
///
/// All fields are private and production construction is restricted to
/// [`AdmittedRealtimeQuotes::from_audited_batch`]. This prevents consumers from
/// promoting a freely constructed [`RealtimeMarketQuote`] projection into
/// evidence that can drive a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedRealtimeQuote {
    record: RealtimeMarketQuote,
    evidence: BatchEvidence,
}

impl AdmittedRealtimeQuote {
    pub fn code(&self) -> &str {
        &self.record.code
    }

    pub fn name(&self) -> &str {
        &self.record.name
    }

    pub fn price(&self) -> f64 {
        self.record.price
    }

    pub fn source_at(&self) -> DateTime<Utc> {
        self.record.source_at
    }

    pub fn observed_at(&self) -> DateTime<Utc> {
        self.record.observed_at
    }

    pub const fn evidence(&self) -> &BatchEvidence {
        &self.evidence
    }

    /// Pure test seam. This symbol is absent from production builds and keeps
    /// test/live identities physically distinct.
    #[cfg(test)]
    pub(crate) fn from_test_fixture(
        record: RealtimeMarketQuote,
        evidence: BatchEvidence,
    ) -> Result<Self, GatewayError> {
        if !record.code.starts_with("TEST_CODE_")
            || !evidence.source.starts_with("TEST_CODE")
            || !evidence.batch_id.starts_with("TEST_CODE")
        {
            return Err(GatewayError::invalid_request(
                CAPABILITY,
                "realtime-quote fixtures must use TEST_CODE identities",
            ));
        }
        validate_admitted_projection(&record, &evidence)?;
        Ok(Self { record, evidence })
    }
}

/// A non-empty realtime quote batch whose records remain bound to the exact
/// provider evidence admitted by [`MarketDataGateway`].
#[derive(Debug)]
pub struct AdmittedRealtimeQuotes {
    quotes: Vec<AdmittedRealtimeQuote>,
}

impl AdmittedRealtimeQuotes {
    fn from_audited_batch(batch: GatewayBatch<RealtimeMarketQuote>) -> Result<Self, GatewayError> {
        match batch {
            GatewayBatch::Available { records, evidence } if !records.is_empty() => {
                let mut quotes = Vec::with_capacity(records.len());
                for record in records {
                    validate_admitted_projection(&record, &evidence)?;
                    quotes.push(AdmittedRealtimeQuote {
                        record,
                        evidence: evidence.clone(),
                    });
                }
                Ok(Self { quotes })
            }
            GatewayBatch::Available { evidence, .. } | GatewayBatch::VerifiedEmpty(evidence) => {
                Err(GatewayError::unavailable(
                    CAPABILITY,
                    Some(evidence.provider),
                    true,
                    format!(
                        "provider returned no admitted realtime quotes source={} batch_id={}",
                        evidence.source, evidence.batch_id
                    ),
                ))
            }
        }
    }

    pub fn len(&self) -> usize {
        self.quotes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.quotes.is_empty()
    }

    pub fn quotes(&self) -> &[AdmittedRealtimeQuote] {
        &self.quotes
    }

    /// Consume the sealed batch and return the exact requested quote. Absence
    /// is an identity/evidence failure, never a default quote.
    pub fn into_required_quote(
        self,
        required_code: &str,
    ) -> Result<AdmittedRealtimeQuote, GatewayError> {
        self.quotes
            .into_iter()
            .find(|quote| quote.code() == required_code)
            .ok_or_else(|| {
                GatewayError::invalid_evidence(
                    CAPABILITY,
                    None,
                    format!("admitted batch does not contain required quote {required_code}"),
                )
            })
    }
}

/// Evidence-preserving public quote route.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarketDataGateway;

impl MarketDataGateway {
    pub const fn new() -> Self {
        Self
    }

    pub fn realtime_quotes(
        &self,
        codes: &[String],
    ) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        let request_hash = acquisition_request_hash(CAPABILITY, codes.join(","));
        // P4 M2 钩子: remote gRPC → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("RealtimeQuotes") {
            Ok(bridge) => {
                let result = bridge.realtime_quotes(codes);
                return audit_routed_gateway_result(CAPABILITY, &request_hash, result);
            }
            Err(error) => {
                return audit_routed_gateway_result(CAPABILITY, &request_hash, Err(error));
            }
        }
        // P4 M5: no-feature 构建不携带 library transport, 无桥时显式失败
        // (fail-closed), 绝不静默回退。
    }

    /// Quote-specific request coverage. Callers that produce account-wide or
    /// atomic decisions must call [`RealtimeQuoteCoverage::require_complete`];
    /// scanners may explicitly consume `Partial` and surface rejected/missing
    /// instruments without marking whole-account quote readiness.
    pub fn realtime_quote_coverage(&self, codes: &[String]) -> RealtimeQuoteCoverage {
        let request_hash = acquisition_request_hash(CAPABILITY, codes.join(","));
        let result = match super::grpc_source::bridge_for("RealtimeQuotes") {
            Ok(bridge) => bridge.realtime_quote_candidates(codes),
            Err(error) => Err(error),
        };
        RealtimeQuoteCoverage::classify(
            codes,
            audit_routed_gateway_result(CAPABILITY, &request_hash, result),
        )
    }

    /// Acquire a non-empty batch whose quote projections cannot be detached
    /// from their audited provider evidence.
    pub fn required_realtime_quotes(
        &self,
        codes: &[String],
    ) -> Result<AdmittedRealtimeQuotes, GatewayError> {
        AdmittedRealtimeQuotes::from_audited_batch(
            self.realtime_quote_coverage(codes).require_complete()?,
        )
    }

    /// Acquire exactly one source-bound realtime quote.
    pub fn required_realtime_quote(
        &self,
        code: &str,
    ) -> Result<AdmittedRealtimeQuote, GatewayError> {
        self.required_realtime_quotes(&[code.to_owned()])?
            .into_required_quote(code)
    }

    /// BR-233 (2026-08-10): 盘后收盘静态快照 — 收市后消费者 (R-07 明日观察池
    /// 21:00 晚间装配) 获取最后交易时段 (trading_date) 的收盘价+中文名。
    /// 准入规则: source_at 日期 == trading_date 且 observed_at 已过该日
    /// 北京时间 15:00 (UTC 07:00) 收盘时刻; 盘中调用 fail-closed。
    /// 盘中路径仍走 [`Self::realtime_quotes`] 的 BR-218 五秒红线, 不受影响。
    pub fn settled_close_quotes(
        &self,
        codes: &[String],
        trading_date: NaiveDate,
    ) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
        let _ = (codes, trading_date);
        Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Tdx),
            "unavailable",
            "provider_transport",
            true,
            "settled-close quotes are unavailable over the remote transport",
        ))
    }
}

fn validate_admitted_projection(
    record: &RealtimeMarketQuote,
    evidence: &BatchEvidence,
) -> Result<(), GatewayError> {
    let evidence_observed_at = parse_evidence_instant(
        CAPABILITY,
        evidence.provider,
        "observed_at",
        &evidence.observed_at,
    )?;
    let evidence_source_at = evidence
        .source_at
        .as_deref()
        .ok_or_else(|| {
            GatewayError::invalid_evidence(
                CAPABILITY,
                Some(evidence.provider),
                "admitted realtime batch has no provider source time",
            )
        })
        .and_then(|value| {
            parse_evidence_instant(CAPABILITY, evidence.provider, "source_at", value)
        })?;
    if record.provider != evidence.provider
        || record.batch_id != evidence.batch_id
        || record.observed_at != evidence_observed_at
        || record.source_at != evidence_source_at
    {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(evidence.provider),
            format!(
                "realtime quote {} differs from admitted batch evidence",
                record.code
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseManager;
    use diesel::RunQueryDsl;

    fn task9_evidence() -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tencent,
            source: "TEST_CODE_quote_source".to_owned(),
            source_at: Some("2026-09-25T01:30:00Z".to_owned()),
            observed_at: "2026-09-25T01:30:01Z".to_owned(),
            batch_id: "TEST_CODE_quote_coverage".to_owned(),
        }
    }

    fn task9_quote(code: &str, price: f64) -> RealtimeMarketQuote {
        RealtimeMarketQuote {
            code: code.to_owned(),
            name: format!("TEST_CODE_{code}"),
            price,
            previous_close: 10.0,
            change_percent: 1.0,
            source_at: DateTime::parse_from_rfc3339("2026-09-25T01:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            observed_at: DateTime::parse_from_rfc3339("2026-09-25T01:30:01Z")
                .unwrap()
                .with_timezone(&Utc),
            provider: ProviderId::Tencent,
            batch_id: "TEST_CODE_quote_coverage".to_owned(),
        }
    }

    #[test]
    fn task9_quote_coverage_isolates_record_local_failure_and_strict_consumer_rejects_partial() {
        let requested = vec!["TEST_CODE_A".to_owned(), "TEST_CODE_B".to_owned()];
        let coverage = RealtimeQuoteCoverage::classify(
            &requested,
            Ok(GatewayBatch::Available {
                records: vec![
                    task9_quote("TEST_CODE_A", 10.0),
                    task9_quote("TEST_CODE_B", f64::NAN),
                ],
                evidence: task9_evidence(),
            }),
        );

        assert_eq!(coverage.disposition(), QuoteCoverageDisposition::Partial);
        assert_eq!(coverage.requested(), requested.as_slice());
        assert_eq!(coverage.accepted()[0].code, "TEST_CODE_A");
        assert_eq!(coverage.rejected()[0].code.as_deref(), Some("TEST_CODE_B"));
        assert_eq!(coverage.rejected()[0].reason_code, "invalid_quote_price");
        assert!(coverage.missing().is_empty());
        assert_eq!(
            coverage.evidence().expect("raw lineage").batch_id,
            "TEST_CODE_quote_coverage"
        );
        assert_eq!(
            coverage
                .clone()
                .require_complete()
                .expect_err("strict consumer must reject partial coverage")
                .reason_code(),
            "quote_coverage_incomplete"
        );
    }

    #[test]
    fn task9_quote_coverage_distinguishes_complete_missing_all_bad_and_shared_failure() {
        let requested = vec!["TEST_CODE_A".to_owned(), "TEST_CODE_B".to_owned()];
        let complete = RealtimeQuoteCoverage::classify(
            &requested,
            Ok(GatewayBatch::Available {
                records: vec![
                    task9_quote("TEST_CODE_A", 10.0),
                    task9_quote("TEST_CODE_B", 11.0),
                ],
                evidence: task9_evidence(),
            }),
        );
        assert_eq!(complete.disposition(), QuoteCoverageDisposition::Complete);
        assert_eq!(complete.require_complete().unwrap().records().len(), 2);

        let missing = RealtimeQuoteCoverage::classify(
            &requested,
            Ok(GatewayBatch::Available {
                records: vec![task9_quote("TEST_CODE_A", 10.0)],
                evidence: task9_evidence(),
            }),
        );
        assert_eq!(missing.disposition(), QuoteCoverageDisposition::Partial);
        assert_eq!(missing.missing(), &["TEST_CODE_B".to_owned()]);

        let all_bad = RealtimeQuoteCoverage::classify(
            &requested,
            Ok(GatewayBatch::Available {
                records: vec![
                    task9_quote("TEST_CODE_A", 0.0),
                    task9_quote("TEST_CODE_B", f64::INFINITY),
                ],
                evidence: task9_evidence(),
            }),
        );
        assert_eq!(all_bad.disposition(), QuoteCoverageDisposition::Unavailable);
        assert_eq!(all_bad.rejected().len(), 2);
        assert!(all_bad.accepted().is_empty());

        let mut shared_identity_failure = task9_quote("TEST_CODE_A", 10.0);
        shared_identity_failure.batch_id = "TEST_CODE_WRONG_BATCH".to_owned();
        let shared = RealtimeQuoteCoverage::classify(
            &requested,
            Ok(GatewayBatch::Available {
                records: vec![shared_identity_failure, task9_quote("TEST_CODE_B", 11.0)],
                evidence: task9_evidence(),
            }),
        );
        assert_eq!(shared.disposition(), QuoteCoverageDisposition::Unavailable);
        assert!(shared.accepted().is_empty());
        assert!(shared.rejected().is_empty());
    }

    #[derive(diesel::QueryableByName)]
    struct AuditProviderRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        provider: String,
    }

    #[test]
    fn realtime_bridge_failure_without_provider_is_audited_as_custom() {
        let _env = super::super::grpc_source::test_grpc_env_guard();
        DatabaseManager::init(None).expect("TEST_CODE audit database init");
        std::env::remove_var("GRPC_MARKET_CLIENT_BUNDLE");
        std::env::set_var("GRPC_MARKET_ADDR", "http://127.0.0.1:1");
        super::super::grpc_source::reset_bridge();

        let result = MarketDataGateway::new().realtime_quotes(&["399992".to_owned()]);

        std::env::remove_var("GRPC_MARKET_ADDR");
        super::super::grpc_source::reset_bridge();

        let error = result.expect_err("unreachable bridge must fail closed");
        assert_eq!(error.provider(), None);

        let request_hash = acquisition_request_hash(CAPABILITY, "399992");
        let mut connection = DatabaseManager::get().get_conn().unwrap();
        let row = diesel::sql_query(
            "SELECT provider FROM data_acquisition_audit \
             WHERE capability = 'RealtimeMarketQuotes' AND request_hash = ? \
             ORDER BY id DESC LIMIT 1",
        )
        .bind::<diesel::sql_types::Text, _>(request_hash)
        .get_result::<AuditProviderRow>(&mut *connection)
        .expect("bridge failure must be audited");
        assert_eq!(row.provider, "Custom");
    }
}
