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
    acquisition_request_hash, audit_gateway_result, BatchEvidence, GatewayBatch, GatewayError,
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
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Tdx);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Err(error) => {
                return audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Tdx,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // P4 M5: no-feature 构建不携带 library transport, 无桥时显式失败
        // (fail-closed), 绝不静默回退。
    }

    /// Acquire a non-empty batch whose quote projections cannot be detached
    /// from their audited provider evidence.
    pub fn required_realtime_quotes(
        &self,
        codes: &[String],
    ) -> Result<AdmittedRealtimeQuotes, GatewayError> {
        AdmittedRealtimeQuotes::from_audited_batch(self.realtime_quotes(codes)?)
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
