//! BR-171 evidence-preserving security lifecycle acquisition.
//!
//! This module is the only downstream owner of the pinned Magic TDX security
//! metadata and corporate-action contracts. Listing date is admitted as an
//! independent field from a best-effort metadata batch. Corporate actions are
//! admitted only with exact request coverage and complete, ordered evidence;
//! only source-published `Implemented` actions are projected.

use crate::market_domain::{
    AssetClass, CorporateActionCategory, CorporateActionTerms, InstrumentId, ProviderId,
};

use chrono::NaiveDate;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::instrument_identity::{resolve_production_equity, EquitySegment};

use super::review::{
    acquisition_request_hash, audit_gateway_result, BatchEvidence, GatewayBatch, GatewayError,
};
use super::MarketSecurityMetadata;

const LIFECYCLE_CAPABILITY: &str = "SecurityLifecycle";
const LISTING_CAPABILITY: &str = "SecurityLifecycleListing";
const ACTIONS_CAPABILITY: &str = "SecurityLifecycleCorporateActions";

/// A listing date whose record identity and batch evidence were admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedListingDate {
    pub listed_on: NaiveDate,
    pub evidence: BatchEvidence,
}

/// Listing date remains independently unavailable when Magic TDX omitted the
/// field or its metadata batch could not be admitted.
#[derive(Debug, Clone)]
pub enum ListingDateState {
    Available(AdmittedListingDate),
    Unavailable {
        evidence: Option<BatchEvidence>,
        error: GatewayError,
    },
}

/// Consumer projection of one implemented corporate action.
#[derive(Debug, Clone, PartialEq)]
pub struct ImplementedCorporateAction {
    pub code: String,
    pub category: CorporateActionCategory,
    pub effective_on: NaiveDate,
    pub record_on: Option<NaiveDate>,
    pub ex_on: Option<NaiveDate>,
    pub payable_on: Option<NaiveDate>,
    pub terms: CorporateActionTerms,
}

/// Exact corporate-action coverage is distinct from provider unavailability.
///
/// `Available` may contain no projected records when the source batch was
/// non-empty but every source record was proposed, cancelled or unknown.
/// Only an explicitly empty, complete source response becomes `VerifiedEmpty`.
#[derive(Debug, Clone)]
pub enum CorporateActionState {
    Available {
        records: Vec<ImplementedCorporateAction>,
        evidence: BatchEvidence,
    },
    VerifiedEmpty(BatchEvidence),
    Unavailable(GatewayError),
}

impl CorporateActionState {
    pub fn evidence(&self) -> Option<&BatchEvidence> {
        match self {
            Self::Available { evidence, .. } | Self::VerifiedEmpty(evidence) => Some(evidence),
            Self::Unavailable(_) => None,
        }
    }

    pub fn records(&self) -> &[ImplementedCorporateAction] {
        match self {
            Self::Available { records, .. } => records,
            Self::VerifiedEmpty(_) | Self::Unavailable(_) => &[],
        }
    }

    pub fn is_verified_empty(&self) -> bool {
        matches!(self, Self::VerifiedEmpty(_))
    }
}

/// Lifecycle facts for the exact daily-bar window requested by a consumer.
#[derive(Debug, Clone)]
pub struct SecurityLifecycleContext {
    pub instrument: InstrumentId,
    pub window_start: NaiveDate,
    pub window_end: NaiveDate,
    pub listing: ListingDateState,
    pub corporate_actions: CorporateActionState,
}

/// Exact lifecycle evidence bound into one BR-171 operator confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleConfirmationEvidence {
    pub provider: String,
    pub batch_identity: String,
    pub listing_date: Option<NaiveDate>,
    pub corporate_action_identity: Option<String>,
}

impl SecurityLifecycleContext {
    /// Project the exact lifecycle facts relevant to one adjacent daily-close
    /// move. Lifecycle evidence explains context but never confirms the move.
    pub fn confirmation_evidence_for(
        &self,
        previous_date: NaiveDate,
        current_date: NaiveDate,
    ) -> Result<LifecycleConfirmationEvidence, GatewayError> {
        if previous_date >= current_date
            || previous_date < self.window_start
            || current_date > self.window_end
        {
            return Err(GatewayError::invalid_request(
                LIFECYCLE_CAPABILITY,
                "confirmation dates must be ordered and covered by the lifecycle window",
            ));
        }

        let (listing_date, listing_batch_id) = match &self.listing {
            ListingDateState::Available(listing) => (
                Some(listing.listed_on),
                Some(listing.evidence.batch_id.as_str()),
            ),
            ListingDateState::Unavailable { evidence, .. } => (
                None,
                evidence.as_ref().map(|evidence| evidence.batch_id.as_str()),
            ),
        };
        let (actions, actions_evidence) = match &self.corporate_actions {
            CorporateActionState::Available { records, evidence } => (records.as_slice(), evidence),
            CorporateActionState::VerifiedEmpty(evidence) => (&[][..], evidence),
            CorporateActionState::Unavailable(error) => {
                return Err(GatewayError::classified(
                    LIFECYCLE_CAPABILITY,
                    Some(ProviderId::Tdx),
                    error.audit_outcome(),
                    "corporate_action_context_unavailable",
                    error.retryable(),
                    format!(
                        "BR-171 cannot prepare manual confirmation without exact \
                         corporate-action coverage: {error}"
                    ),
                ));
            }
        };

        let corporate_action_identity =
            relevant_action_identity(actions, previous_date, current_date)?;
        let batch_identity = format!(
            "window={}:{}|listing={}|actions={}",
            self.window_start,
            self.window_end,
            listing_batch_id.unwrap_or("unavailable"),
            actions_evidence.batch_id
        );
        Ok(LifecycleConfirmationEvidence {
            provider: "magic_tdx".to_string(),
            batch_identity,
            listing_date,
            corporate_action_identity,
        })
    }
}

#[derive(Serialize)]
struct ConfirmationActionIdentity<'a> {
    code: &'a str,
    category: CorporateActionCategory,
    effective_on: String,
    record_on: Option<String>,
    ex_on: Option<String>,
    payable_on: Option<String>,
    terms: &'a CorporateActionTerms,
}

fn relevant_action_identity(
    actions: &[ImplementedCorporateAction],
    previous_date: NaiveDate,
    current_date: NaiveDate,
) -> Result<Option<String>, GatewayError> {
    let relevant = actions
        .iter()
        .filter(|action| {
            [
                Some(action.effective_on),
                action.record_on,
                action.ex_on,
                action.payable_on,
            ]
            .into_iter()
            .flatten()
            .any(|date| date > previous_date && date <= current_date)
        })
        .map(|action| ConfirmationActionIdentity {
            code: &action.code,
            category: action.category,
            effective_on: action.effective_on.to_string(),
            record_on: action.record_on.map(|date| date.to_string()),
            ex_on: action.ex_on.map(|date| date.to_string()),
            payable_on: action.payable_on.map(|date| date.to_string()),
            terms: &action.terms,
        })
        .collect::<Vec<_>>();
    if relevant.is_empty() {
        return Ok(None);
    }
    let encoded = serde_json::to_vec(&relevant).map_err(|error| {
        GatewayError::invalid_evidence(
            LIFECYCLE_CAPABILITY,
            Some(ProviderId::Tdx),
            format!("cannot serialize corporate-action identity: {error}"),
        )
    })?;
    let mut hasher = Sha256::new();
    hasher.update(b"BR171_CORPORATE_ACTION_IDENTITY_V1\0");
    hasher.update(encoded);
    Ok(Some(format!("sha256:{}", hex::encode(hasher.finalize()))))
}

/// Production lifecycle seam. The blocking Magic TDX client is constructed,
/// used and dropped wholly inside one `spawn_blocking` worker.
#[derive(Debug, Clone, Copy, Default)]
pub struct SecurityLifecycleGateway;

impl SecurityLifecycleGateway {
    pub const fn new() -> Self {
        Self
    }

    /// Acquires metadata and corporate actions once for one instrument/window.
    ///
    /// Component unavailability is preserved in the returned context so a
    /// metadata failure cannot be mistaken for a verified-empty action batch
    /// (or vice versa). Invalid requests and blocking-worker failures remain
    /// top-level errors.
    pub async fn acquire(
        &self,
        code: &str,
        window_start: NaiveDate,
        window_end: NaiveDate,
    ) -> Result<SecurityLifecycleContext, GatewayError> {
        let code = code.to_owned();
        let canonical = format!("{code}:{window_start}:{window_end}");
        let listing_hash = acquisition_request_hash(LISTING_CAPABILITY, &canonical);
        let actions_hash = acquisition_request_hash(ACTIONS_CAPABILITY, &canonical);

        // P4 M4b 批次 1B: grpc 模式走桥 (服务端 SecurityLifecycleGateway 直连)。
        // 本地验证先行复制 (fail-fast, 与 acquire_blocking 语义一致):
        // window 顺序 + build_instrument (含 Beijing 拒绝)。审计留客户端
        // (audit_gateway_result), 与服务端审计双写 — ProviderTopNRankings 先例。
        match super::grpc_source::bridge_for("CorporateActions") {
            Ok(bridge) => {
                if window_start > window_end {
                    return Err(GatewayError::invalid_request(
                        LIFECYCLE_CAPABILITY,
                        "lifecycle window start must not exceed end",
                    ));
                }
                let instrument = build_instrument(&code)?;
                let metadata = bridge
                    .security_metadata_async(std::slice::from_ref(&code))
                    .await
                    .map(bridge_listing_projection);
                let metadata = audit_gateway_result(
                    LISTING_CAPABILITY,
                    ProviderId::Tdx,
                    &listing_hash,
                    metadata,
                );
                let actions = bridge
                    .corporate_actions_async(&code, window_start, window_end)
                    .await;
                let actions = audit_gateway_result(
                    ACTIONS_CAPABILITY,
                    ProviderId::Tdx,
                    &actions_hash,
                    actions,
                );
                return Ok(SecurityLifecycleContext {
                    instrument,
                    window_start,
                    window_end,
                    listing: listing_state(metadata),
                    corporate_actions: action_state(actions),
                });
            }
            Err(error) => {
                return Err(GatewayError::unavailable(
                    LIFECYCLE_CAPABILITY,
                    Some(ProviderId::Tdx),
                    true,
                    format!("CorporateActions 桥初始化失败: {error}"),
                ));
            }
        }

        // no-feature (monitor 零 magic 构建): library transport 编译期不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListingProjection {
    listed_on: Option<NaiveDate>,
}

/// 桥路径 listing 映射: 服务端 SecurityMetadata 视图 listed_on 恒为 to_string()
/// (非 Option) → 记录恒携带日期; 空批 (VerifiedEmpty) 原样传递, 由 listing_state
/// 统一映射为 Unavailable-with-evidence (fail-closed, 与本地 "omitted" 语义等效)。
fn bridge_listing_projection(
    batch: GatewayBatch<MarketSecurityMetadata>,
) -> GatewayBatch<ListingProjection> {
    match batch {
        GatewayBatch::Available { records, evidence } => {
            let listed_on = records.first().map(|r| r.listed_on);
            GatewayBatch::Available {
                records: vec![ListingProjection { listed_on }],
                evidence,
            }
        }
        GatewayBatch::VerifiedEmpty(evidence) => GatewayBatch::VerifiedEmpty(evidence),
    }
}

fn listing_state(
    result: Result<GatewayBatch<ListingProjection>, GatewayError>,
) -> ListingDateState {
    match result {
        Ok(GatewayBatch::Available { records, evidence }) => match records.as_slice() {
            [ListingProjection {
                listed_on: Some(listed_on),
            }] => ListingDateState::Available(AdmittedListingDate {
                listed_on: *listed_on,
                evidence,
            }),
            [ListingProjection { listed_on: None }] => ListingDateState::Unavailable {
                evidence: Some(evidence),
                error: GatewayError::classified(
                    LISTING_CAPABILITY,
                    Some(ProviderId::Tdx),
                    "partial",
                    "listing_date_unavailable",
                    false,
                    "Magic TDX metadata batch omitted the listing date",
                ),
            },
            _ => ListingDateState::Unavailable {
                evidence: Some(evidence),
                error: GatewayError::invalid_evidence(
                    LISTING_CAPABILITY,
                    Some(ProviderId::Tdx),
                    "admitted listing projection cardinality changed",
                ),
            },
        },
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => ListingDateState::Unavailable {
            evidence: Some(evidence),
            error: GatewayError::invalid_evidence(
                LISTING_CAPABILITY,
                Some(ProviderId::Tdx),
                "security metadata cannot be represented as verified empty",
            ),
        },
        Err(error) => ListingDateState::Unavailable {
            evidence: None,
            error,
        },
    }
}

fn action_state(
    result: Result<GatewayBatch<ImplementedCorporateAction>, GatewayError>,
) -> CorporateActionState {
    match result {
        Ok(GatewayBatch::Available { records, evidence }) => {
            CorporateActionState::Available { records, evidence }
        }
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => CorporateActionState::VerifiedEmpty(evidence),
        Err(error) => CorporateActionState::Unavailable(error),
    }
}

fn build_instrument(code: &str) -> Result<InstrumentId, GatewayError> {
    #[cfg(test)]
    let identity = if code.starts_with("TEST_CODE_") {
        super::instrument_identity::resolve_test_equity(code, None)
    } else {
        resolve_production_equity(code, None)
    };
    #[cfg(not(test))]
    let identity = resolve_production_equity(code, None);
    let identity = identity
        .and_then(|identity| {
            identity.require_a_share()?;
            Ok(identity)
        })
        .map_err(|error| {
            GatewayError::invalid_request(
                LIFECYCLE_CAPABILITY,
                format!("invalid lifecycle equity identity {code:?}: {error}"),
            )
        })?;
    if identity.segment() == EquitySegment::BeijingA {
        return Err(GatewayError::invalid_request(
            LIFECYCLE_CAPABILITY,
            format!("Magic TDX lifecycle has no verified Beijing capability for {code:?}"),
        ));
    }
    InstrumentId::new(
        identity.exchange(),
        identity.canonical_code(),
        AssetClass::Equity,
    )
    .map_err(|error| {
        GatewayError::invalid_request(
            LIFECYCLE_CAPABILITY,
            format!("validated instrument {code:?} failed core invariant: {error}"),
        )
    })
}
