//! BR-171 evidence-preserving security lifecycle acquisition.
//!
//! This module is the only downstream owner of the pinned Magic TDX security
//! metadata and corporate-action contracts. Listing date is admitted as an
//! independent field from a best-effort metadata batch. Corporate actions are
//! admitted only with exact request coverage and complete, ordered evidence;
//! only source-published `Implemented` actions are projected.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
#[cfg(test)]
use crate::magic_compat::Exchange;
use crate::magic_compat::{AssetClass, DataBatch, InstrumentId, IsoDate, ProviderId, SourceEvidence};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{CorporateAction, CorporateActionCategory, CorporateActionRequest, CorporateActionResponse, CorporateActionStatus, CorporateActionTerms, CorporateActions, SecurityMetadata, SecurityMetadataProvider};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::{TdxError, TdxSmartClient};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::instrument_identity::{resolve_production_equity, EquitySegment};
use super::review::{
    acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result, BatchEvidence,
    GatewayBatch, GatewayError,
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
        let lifecycle_hash = acquisition_request_hash(LIFECYCLE_CAPABILITY, &canonical);
        let listing_hash = acquisition_request_hash(LISTING_CAPABILITY, &canonical);
        let actions_hash = acquisition_request_hash(ACTIONS_CAPABILITY, &canonical);
        let worker_lifecycle_hash = lifecycle_hash.clone();

        // P4 M4b 批次 1B: grpc 模式走桥 (服务端 SecurityLifecycleGateway 直连)。
        // 本地验证先行复制 (fail-fast, 与 acquire_blocking 语义一致):
        // window 顺序 + build_instrument (含 Beijing 拒绝)。审计留客户端
        // (audit_gateway_result), 与服务端审计双写 — ProviderTopNRankings 先例。
        match super::grpc_source::bridge_for("CorporateActions") {
            Ok(Some(bridge)) => {
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
                let actions =
                    audit_gateway_result(ACTIONS_CAPABILITY, ProviderId::Tdx, &actions_hash, actions);
                return Ok(SecurityLifecycleContext {
                    instrument,
                    window_start,
                    window_end,
                    listing: listing_state(metadata),
                    corporate_actions: action_state(actions),
                });
            }
            Ok(None) => {}
            Err(error) => {
                return Err(GatewayError::unavailable(
                    LIFECYCLE_CAPABILITY,
                    Some(ProviderId::Tdx),
                    true,
                    format!("CorporateActions 桥初始化失败: {error}"),
                ));
            }
        }

        let joined = tokio::task::spawn_blocking(move || {
            acquire_blocking(code, window_start, window_end, listing_hash, actions_hash)
        })
        .await;

        match joined {
            Ok(result) => result,
            Err(error) => {
                match audit_blocking_join_failure::<()>(
                    LIFECYCLE_CAPABILITY,
                    ProviderId::Tdx,
                    worker_lifecycle_hash,
                    error.to_string(),
                )
                .await
                {
                    Err(error) => Err(error),
                    Ok(_) => Err(GatewayError::invalid_evidence(
                        LIFECYCLE_CAPABILITY,
                        Some(ProviderId::Tdx),
                        "blocking join failure unexpectedly produced an available batch",
                    )),
                }
            }
        }
    }
}

fn acquire_blocking(
    code: String,
    window_start: NaiveDate,
    window_end: NaiveDate,
    listing_hash: String,
    actions_hash: String,
) -> Result<SecurityLifecycleContext, GatewayError> {
    if window_start > window_end {
        return Err(GatewayError::invalid_request(
            LIFECYCLE_CAPABILITY,
            "lifecycle window start must not exceed end",
        ));
    }
    let instrument = build_instrument(&code)?;
    let request = build_action_request(instrument.clone(), window_start, window_end)?;

    // Do not move this client outside the blocking worker. Some blocking
    // provider internals own runtimes whose destructor may block.
    let client = TdxSmartClient::new();
    let metadata = client
        .security_metadata(std::slice::from_ref(&instrument))
        .map_err(|error| tdx_gateway_error(LISTING_CAPABILITY, error))
        .and_then(|batch| admit_listing_metadata(&instrument, batch));
    let metadata =
        audit_gateway_result(LISTING_CAPABILITY, ProviderId::Tdx, &listing_hash, metadata);

    let actions = client
        .corporate_actions(&request)
        .map_err(|error| tdx_gateway_error(ACTIONS_CAPABILITY, error))
        .and_then(|response| {
            current_china_date()
                .and_then(|today| admit_corporate_actions(&request, &response, today))
        });
    let actions = audit_gateway_result(ACTIONS_CAPABILITY, ProviderId::Tdx, &actions_hash, actions);
    drop(client);

    Ok(SecurityLifecycleContext {
        instrument,
        window_start,
        window_end,
        listing: listing_state(metadata),
        corporate_actions: action_state(actions),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListingProjection {
    listed_on: Option<NaiveDate>,
}

fn admit_listing_metadata(
    instrument: &InstrumentId,
    batch: DataBatch<SecurityMetadata>,
) -> Result<GatewayBatch<ListingProjection>, GatewayError> {
    let evidence = BatchEvidence::from_provenance(ProviderId::Tdx, batch.provenance())?;
    validate_observation_timestamp(LISTING_CAPABILITY, &evidence.observed_at)?;

    let [record] = batch.records() else {
        return Err(GatewayError::invalid_evidence(
            LISTING_CAPABILITY,
            Some(ProviderId::Tdx),
            format!(
                "Magic TDX metadata cardinality must be exactly one, got {}",
                batch.records().len()
            ),
        ));
    };
    if record.instrument() != instrument {
        return Err(GatewayError::invalid_evidence(
            LISTING_CAPABILITY,
            Some(ProviderId::Tdx),
            "metadata instrument differs from the lifecycle request",
        ));
    }
    if record.provider() != ProviderId::Tdx {
        return Err(GatewayError::invalid_evidence(
            LISTING_CAPABILITY,
            Some(ProviderId::Tdx),
            "metadata record provider is not Magic TDX",
        ));
    }
    if record.batch_id() != evidence.batch_id.as_str() {
        return Err(GatewayError::invalid_evidence(
            LISTING_CAPABILITY,
            Some(ProviderId::Tdx),
            "metadata record batch ID differs from batch provenance",
        ));
    }
    if record.observed_at() != evidence.observed_at.as_str() {
        return Err(GatewayError::invalid_evidence(
            LISTING_CAPABILITY,
            Some(ProviderId::Tdx),
            "metadata observation time differs from batch provenance",
        ));
    }
    if record.source_at() != evidence.source_at.as_deref() {
        return Err(GatewayError::invalid_evidence(
            LISTING_CAPABILITY,
            Some(ProviderId::Tdx),
            "metadata source time differs from batch provenance",
        ));
    }

    let listed_on = record
        .listed_on()
        .map(parse_iso_date)
        .transpose()
        .map_err(|error| {
            GatewayError::invalid_evidence(
                LISTING_CAPABILITY,
                Some(ProviderId::Tdx),
                format!("invalid source listing date: {error}"),
            )
        })?;
    Ok(GatewayBatch::Available {
        records: vec![ListingProjection { listed_on }],
        evidence,
    })
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

fn admit_corporate_actions(
    request: &CorporateActionRequest,
    response: &CorporateActionResponse,
    admission_today: NaiveDate,
) -> Result<GatewayBatch<ImplementedCorporateAction>, GatewayError> {
    if response.coverage() != request {
        return Err(GatewayError::invalid_evidence(
            ACTIONS_CAPABILITY,
            Some(ProviderId::Tdx),
            "corporate-action response does not prove the exact requested coverage",
        ));
    }
    let admission_as_of = parse_iso_date(response.admission_as_of().as_str()).map_err(|error| {
        GatewayError::invalid_evidence(
            ACTIONS_CAPABILITY,
            Some(ProviderId::Tdx),
            format!("invalid corporate-action admission boundary: {error}"),
        )
    })?;
    if admission_as_of > admission_today {
        return Err(GatewayError::invalid_evidence(
            ACTIONS_CAPABILITY,
            Some(ProviderId::Tdx),
            "corporate-action admission boundary is in the future",
        ));
    }

    let evidence = validate_action_batch(request, response, admission_as_of)?;
    if response.batch().records().is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }

    let records = response
        .batch()
        .records()
        .iter()
        .filter(|record| record.status() == CorporateActionStatus::Implemented)
        .map(project_implemented_action)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GatewayBatch::Available { records, evidence })
}

fn validate_action_batch(
    request: &CorporateActionRequest,
    response: &CorporateActionResponse,
    admission_as_of: NaiveDate,
) -> Result<BatchEvidence, GatewayError> {
    validate_action_parts(
        request,
        response.evidence(),
        response.batch(),
        admission_as_of,
    )
}

fn validate_action_parts(
    request: &CorporateActionRequest,
    response_evidence: &SourceEvidence,
    batch: &DataBatch<CorporateAction>,
    admission_as_of: NaiveDate,
) -> Result<BatchEvidence, GatewayError> {
    if !batch.quality().is_complete() {
        return Err(GatewayError::invalid_evidence(
            ACTIONS_CAPABILITY,
            Some(ProviderId::Tdx),
            format!(
                "corporate-action batch is partial: {}",
                batch.quality().issues().join("; ")
            ),
        ));
    }
    let evidence = BatchEvidence::from_provenance(ProviderId::Tdx, batch.provenance())?;
    validate_observation_timestamp(ACTIONS_CAPABILITY, &evidence.observed_at)?;
    if response_evidence.provider() != ProviderId::Tdx
        || response_evidence.batch_id() != evidence.batch_id.as_str()
        || response_evidence.observed_at() != evidence.observed_at.as_str()
        || response_evidence.source_at() != evidence.source_at.as_deref()
    {
        return Err(GatewayError::invalid_evidence(
            ACTIONS_CAPABILITY,
            Some(ProviderId::Tdx),
            "corporate-action response evidence differs from batch provenance",
        ));
    }

    let start = request
        .start()
        .ok_or_else(|| {
            GatewayError::invalid_request(
                ACTIONS_CAPABILITY,
                "corporate-action request must have an exact start date",
            )
        })
        .and_then(|value| parse_request_date("start", value))?;
    let end = request
        .end()
        .ok_or_else(|| {
            GatewayError::invalid_request(
                ACTIONS_CAPABILITY,
                "corporate-action request must have an exact end date",
            )
        })
        .and_then(|value| parse_request_date("end", value))?;
    if end > admission_as_of {
        return Err(GatewayError::invalid_evidence(
            ACTIONS_CAPABILITY,
            Some(ProviderId::Tdx),
            "corporate-action coverage extends beyond its admission boundary",
        ));
    }

    let mut previous = None;
    for record in batch.records() {
        if record.instrument() != request.instrument() {
            return Err(GatewayError::invalid_evidence(
                ACTIONS_CAPABILITY,
                Some(ProviderId::Tdx),
                "corporate-action record instrument is outside request coverage",
            ));
        }
        if record.evidence() != response_evidence {
            return Err(GatewayError::invalid_evidence(
                ACTIONS_CAPABILITY,
                Some(ProviderId::Tdx),
                "corporate-action record evidence differs from response evidence",
            ));
        }
        let effective_on = parse_iso_date(record.effective_on().as_str()).map_err(|error| {
            GatewayError::invalid_evidence(
                ACTIONS_CAPABILITY,
                Some(ProviderId::Tdx),
                format!("invalid corporate-action effective date: {error}"),
            )
        })?;
        if effective_on < start || effective_on > end || effective_on > admission_as_of {
            return Err(GatewayError::invalid_evidence(
                ACTIONS_CAPABILITY,
                Some(ProviderId::Tdx),
                "corporate-action effective date is outside admitted coverage",
            ));
        }
        let identity = (effective_on, record.category());
        if previous.is_some_and(|previous| previous >= identity) {
            return Err(GatewayError::invalid_evidence(
                ACTIONS_CAPABILITY,
                Some(ProviderId::Tdx),
                "corporate-action identities are duplicate or not strictly ordered",
            ));
        }
        previous = Some(identity);
    }
    Ok(evidence)
}

fn project_implemented_action(
    record: &CorporateAction,
) -> Result<ImplementedCorporateAction, GatewayError> {
    if record.status() != CorporateActionStatus::Implemented {
        return Err(GatewayError::invalid_evidence(
            ACTIONS_CAPABILITY,
            Some(ProviderId::Tdx),
            "non-implemented corporate action reached the public projection",
        ));
    }
    Ok(ImplementedCorporateAction {
        code: record.instrument().code().to_owned(),
        category: record.category(),
        effective_on: parse_record_date("effective_on", record.effective_on())?,
        record_on: record
            .record_on()
            .map(|value| parse_record_date("record_on", value))
            .transpose()?,
        ex_on: record
            .ex_on()
            .map(|value| parse_record_date("ex_on", value))
            .transpose()?,
        payable_on: record
            .payable_on()
            .map(|value| parse_record_date("payable_on", value))
            .transpose()?,
        terms: record.terms().clone(),
    })
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

fn build_action_request(
    instrument: InstrumentId,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<CorporateActionRequest, GatewayError> {
    let start = IsoDate::new(start.format("%Y-%m-%d").to_string())
        .map_err(|error| GatewayError::invalid_request(ACTIONS_CAPABILITY, error.to_string()))?;
    let end = IsoDate::new(end.format("%Y-%m-%d").to_string())
        .map_err(|error| GatewayError::invalid_request(ACTIONS_CAPABILITY, error.to_string()))?;
    CorporateActionRequest::new(instrument)
        .with_range(start, end)
        .map_err(|error| GatewayError::invalid_request(ACTIONS_CAPABILITY, error.to_string()))
}

fn validate_observation_timestamp(
    capability: &'static str,
    observed_at: &str,
) -> Result<(), GatewayError> {
    if DateTime::parse_from_rfc3339(observed_at).is_ok() {
        return Ok(());
    }

    let invalid = || {
        GatewayError::invalid_evidence(
            capability,
            Some(ProviderId::Tdx),
            format!("invalid observation timestamp {observed_at:?}"),
        )
    };
    let (seconds, nanos) = match observed_at.split_once('.') {
        Some((seconds, fraction)) => {
            if fraction.is_empty()
                || fraction.len() > 9
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(invalid());
            }
            let seconds = seconds.parse::<i64>().map_err(|_| invalid())?;
            let nanos = format!("{fraction:0<9}")
                .parse::<u32>()
                .map_err(|_| invalid())?;
            (seconds, nanos)
        }
        None => (observed_at.parse::<i64>().map_err(|_| invalid())?, 0),
    };
    Utc.timestamp_opt(seconds, nanos)
        .single()
        .map(|_| ())
        .ok_or_else(invalid)
}

fn parse_request_date(field: &str, value: &IsoDate) -> Result<NaiveDate, GatewayError> {
    parse_iso_date(value.as_str()).map_err(|error| {
        GatewayError::invalid_request(
            ACTIONS_CAPABILITY,
            format!("invalid corporate-action request {field}: {error}"),
        )
    })
}

fn parse_record_date(field: &str, value: &IsoDate) -> Result<NaiveDate, GatewayError> {
    parse_iso_date(value.as_str()).map_err(|error| {
        GatewayError::invalid_evidence(
            ACTIONS_CAPABILITY,
            Some(ProviderId::Tdx),
            format!("invalid corporate-action {field}: {error}"),
        )
    })
}

fn parse_iso_date(value: &str) -> Result<NaiveDate, chrono::ParseError> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
}

fn current_china_date() -> Result<NaiveDate, GatewayError> {
    let offset = FixedOffset::east_opt(8 * 60 * 60).ok_or_else(|| {
        GatewayError::invalid_request(
            LIFECYCLE_CAPABILITY,
            "invalid fixed China-market timezone offset",
        )
    })?;
    Ok(Utc::now().with_timezone(&offset).date_naive())
}

fn tdx_gateway_error(capability: &'static str, error: TdxError) -> GatewayError {
    let message = error.to_string();
    match error {
        TdxError::Io(_)
        | TdxError::Connection(_)
        | TdxError::ConnectionTimeout
        | TdxError::SetupFailed(_)
        | TdxError::Disconnected
        | TdxError::RetryExhausted(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "unavailable",
            "provider_transport",
            true,
            message,
        ),
        TdxError::Unsupported(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        TdxError::HistoricalBarCardinality {
            offset,
            actual,
            expected_page,
            requested_total,
        } => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "partial",
            "provider_batch_rejected",
            false,
            format!(
                "Magic TDX historical-bar cardinality mismatch: offset={offset} actual={actual} \
                 expected_page={expected_page} requested_total={requested_total}"
            ),
        ),
        TdxError::Parse(_)
        | TdxError::InvalidData(_)
        | TdxError::ResponseParse(_)
        | TdxError::Core(_)
        | TdxError::Coded(_)
        | TdxError::FileNotFound(_) => GatewayError::classified(
            capability,
            Some(ProviderId::Tdx),
            "partial",
            "provider_batch_rejected",
            false,
            message,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::magic_compat::{FiniteNumber, Provenance, SourceEvidence};
#[cfg(feature = "magic-gateway")]
use magic_market_core::{DataStatus, PriceLimitRule};

    const OBSERVED_AT: &str = "2026-07-27T01:00:00Z";
    const BATCH_ID: &str = "TEST_CODE_lifecycle_batch";

    fn instrument() -> InstrumentId {
        InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600001", AssetClass::Equity)
            .expect("TEST_CODE instrument")
    }

    fn iso(value: &str) -> IsoDate {
        IsoDate::new(value).expect("TEST_CODE ISO date")
    }

    fn provenance(batch_id: &str) -> Provenance {
        Provenance::new("tdx", OBSERVED_AT)
            .expect("TEST_CODE provenance")
            .with_batch_id(batch_id)
            .expect("TEST_CODE batch ID")
    }

    #[test]
    fn br171_observation_timestamp_accepts_declared_tdx_epoch_and_rfc3339_forms() {
        assert!(validate_observation_timestamp(LISTING_CAPABILITY, OBSERVED_AT).is_ok());
        assert!(
            validate_observation_timestamp(ACTIONS_CAPABILITY, "1785169084").is_ok(),
            "Magic TDX emits whole epoch seconds"
        );
        assert!(
            validate_observation_timestamp(ACTIONS_CAPABILITY, "1785169084.123456789").is_ok(),
            "Magic Core may retain nanosecond epoch fractions"
        );
        for invalid in [
            "",
            "epoch",
            "1785169084.",
            "1785169084.bad",
            "1785169084.1234567890",
        ] {
            assert!(validate_observation_timestamp(ACTIONS_CAPABILITY, invalid).is_err());
        }
    }

    #[test]
    fn br171_tdx_cardinality_failure_preserves_typed_batch_mismatch() {
        let error = tdx_gateway_error(
            ACTIONS_CAPABILITY,
            TdxError::HistoricalBarCardinality {
                offset: 800,
                actual: 99,
                expected_page: 100,
                requested_total: 900,
            },
        );
        assert_eq!(error.audit_outcome(), "partial");
        assert_eq!(error.reason_code(), "provider_batch_rejected");
        assert!(!error.retryable());
        let message = error.to_string();
        for expected in [
            "offset=800",
            "actual=99",
            "expected_page=100",
            "requested_total=900",
        ] {
            assert!(message.contains(expected));
        }
    }

    fn evidence(batch_id: &str) -> SourceEvidence {
        SourceEvidence::new(ProviderId::Tdx, OBSERVED_AT, batch_id).expect("TEST_CODE evidence")
    }

    fn batch_evidence(batch_id: &str) -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_tdx".to_string(),
            source_at: Some("2026-07-27".to_string()),
            observed_at: OBSERVED_AT.to_string(),
            batch_id: batch_id.to_string(),
        }
    }

    fn metadata(listed_on: Option<&str>, batch_id: &str) -> SecurityMetadata {
        SecurityMetadata::new(
            instrument(),
            Some("TEST_CODE security".to_owned()),
            None,
            None,
            listed_on.map(str::to_owned),
            PriceLimitRule::new(None, None).expect("TEST_CODE price-limit absence"),
            DataStatus::Unavailable,
            None,
            OBSERVED_AT,
            ProviderId::Tdx,
            batch_id,
        )
        .expect("TEST_CODE best-effort metadata")
    }

    fn request(start: &str, end: &str) -> CorporateActionRequest {
        CorporateActionRequest::new(instrument())
            .with_range(iso(start), iso(end))
            .expect("TEST_CODE action request")
    }

    fn action(
        effective_on: &str,
        status: CorporateActionStatus,
        batch_id: &str,
    ) -> CorporateAction {
        let terms = CorporateActionTerms::distribution(
            Some(FiniteNumber::new(0.1).expect("TEST_CODE finite distribution")),
            None,
            None,
            None,
        )
        .expect("TEST_CODE distribution terms");
        CorporateAction::new(
            instrument(),
            CorporateActionCategory::Distribution,
            iso(effective_on),
            status,
            terms,
            evidence(batch_id),
        )
        .expect("TEST_CODE corporate action")
    }

    fn response(
        coverage: CorporateActionRequest,
        admission_as_of: &str,
        records: Vec<CorporateAction>,
        batch_id: &str,
    ) -> CorporateActionResponse {
        CorporateActionResponse::new(
            coverage,
            iso(admission_as_of),
            evidence(batch_id),
            DataBatch::strict(records, provenance(batch_id)),
        )
        .expect("TEST_CODE corporate-action response")
    }

    #[test]
    fn br171_listing_date_accepts_field_from_best_effort_metadata() {
        let batch = DataBatch::best_effort(
            vec![metadata(Some("2001-02-03"), BATCH_ID)],
            provenance(BATCH_ID),
            vec!["TEST_CODE unrelated fields unavailable".to_owned()],
        )
        .expect("TEST_CODE best-effort batch");

        let admitted =
            admit_listing_metadata(&instrument(), batch).expect("listing field must be admitted");
        let state = listing_state(Ok(admitted));
        let ListingDateState::Available(listing) = state else {
            panic!("TEST_CODE valid listing field was not admitted");
        };
        assert_eq!(
            listing.listed_on,
            NaiveDate::from_ymd_opt(2001, 2, 3).unwrap()
        );
        assert_eq!(listing.evidence.batch_id, BATCH_ID);
    }

    #[test]
    fn br171_missing_listing_date_preserves_unavailable_with_evidence() {
        let batch = DataBatch::best_effort(
            vec![metadata(None, BATCH_ID)],
            provenance(BATCH_ID),
            vec!["TEST_CODE listing date unavailable".to_owned()],
        )
        .expect("TEST_CODE best-effort batch");

        let state = listing_state(admit_listing_metadata(&instrument(), batch));
        let ListingDateState::Unavailable { evidence, error } = state else {
            panic!("TEST_CODE missing listing date must stay unavailable");
        };
        assert_eq!(evidence.expect("batch evidence").batch_id, BATCH_ID);
        assert_eq!(error.reason_code(), "listing_date_unavailable");
    }

    #[test]
    fn br171_listing_identity_mismatch_rejects_batch() {
        let wrong =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600002", AssetClass::Equity).unwrap();
        let batch = DataBatch::best_effort(
            vec![metadata(Some("2001-02-03"), BATCH_ID)],
            provenance(BATCH_ID),
            vec!["TEST_CODE unrelated fields unavailable".to_owned()],
        )
        .unwrap();

        let error = admit_listing_metadata(&wrong, batch).unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br173_lifecycle_identity_rejects_unverified_beijing_aliases_and_b_shares() {
        assert_eq!(
            build_instrument("TEST_CODE_600001")
                .expect("Shanghai A share")
                .exchange(),
            Exchange::Shanghai
        );
        assert_eq!(
            build_instrument("TEST_CODE_000001")
                .expect("Shenzhen A share")
                .exchange(),
            Exchange::Shenzhen
        );
        for code in [
            "TEST_CODE_920118",
            "TEST_CODE_921001",
            "TEST_CODE_929999",
            "TEST_CODE_430001",
            "TEST_CODE_830001",
            "TEST_CODE_900001",
            "TEST_CODE_200001",
        ] {
            assert!(build_instrument(code).is_err());
        }
    }

    #[test]
    fn br171_projects_only_implemented_actions_without_claiming_empty() {
        let request = request("2026-07-01", "2026-07-26");
        let response = response(
            request.clone(),
            "2026-07-27",
            vec![
                action("2026-07-10", CorporateActionStatus::Implemented, BATCH_ID),
                action("2026-07-11", CorporateActionStatus::Proposed, BATCH_ID),
            ],
            BATCH_ID,
        );

        let admitted = admit_corporate_actions(
            &request,
            &response,
            NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
        )
        .expect("TEST_CODE complete response");
        assert!(!admitted.is_verified_empty());
        assert_eq!(admitted.records().len(), 1);
        assert_eq!(
            admitted.records()[0].effective_on,
            NaiveDate::from_ymd_opt(2026, 7, 10).unwrap()
        );
    }

    #[test]
    fn br171_explicit_empty_response_is_verified_empty() {
        let request = request("2026-07-01", "2026-07-26");
        let response = response(request.clone(), "2026-07-27", vec![], BATCH_ID);

        let admitted = admit_corporate_actions(
            &request,
            &response,
            NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
        )
        .expect("TEST_CODE empty complete response");
        assert!(admitted.is_verified_empty());
        assert_eq!(admitted.evidence().batch_id, BATCH_ID);
    }

    #[test]
    fn br171_action_coverage_must_match_exact_request() {
        let expected = request("2026-07-01", "2026-07-26");
        let other = request("2026-07-02", "2026-07-26");
        let response = response(other, "2026-07-27", vec![], BATCH_ID);

        let error = admit_corporate_actions(
            &expected,
            &response,
            NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br171_future_admission_boundary_rejects_response() {
        let request = request("2027-07-01", "2027-07-26");
        let response = response(request.clone(), "2027-07-27", vec![], BATCH_ID);

        let error = admit_corporate_actions(
            &request,
            &response,
            NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br171_partial_action_batch_rejects() {
        let request = request("2026-07-01", "2026-07-26");
        let partial = DataBatch::best_effort(
            vec![action(
                "2026-07-10",
                CorporateActionStatus::Implemented,
                BATCH_ID,
            )],
            provenance(BATCH_ID),
            vec!["TEST_CODE incomplete action page".to_owned()],
        )
        .unwrap();
        let response = RawActionResponseForTest {
            coverage: request.clone(),
            admission_as_of: iso("2026-07-27"),
            evidence: evidence(BATCH_ID),
            batch: partial,
        };

        let error = validate_raw_action_batch_for_test(
            &request,
            &response,
            NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br171_duplicate_or_unordered_action_identity_rejects() {
        let request = request("2026-07-01", "2026-07-26");
        let records = vec![
            action("2026-07-10", CorporateActionStatus::Implemented, BATCH_ID),
            action("2026-07-10", CorporateActionStatus::Implemented, BATCH_ID),
        ];
        let response = RawActionResponseForTest {
            coverage: request.clone(),
            admission_as_of: iso("2026-07-27"),
            evidence: evidence(BATCH_ID),
            batch: DataBatch::strict(records, provenance(BATCH_ID)),
        };

        let error = validate_raw_action_batch_for_test(
            &request,
            &response,
            NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br171_unordered_action_identity_rejects() {
        let request = request("2026-07-01", "2026-07-26");
        let records = vec![
            action("2026-07-11", CorporateActionStatus::Implemented, BATCH_ID),
            action("2026-07-10", CorporateActionStatus::Implemented, BATCH_ID),
        ];
        let response = RawActionResponseForTest {
            coverage: request.clone(),
            admission_as_of: iso("2026-07-27"),
            evidence: evidence(BATCH_ID),
            batch: DataBatch::strict(records, provenance(BATCH_ID)),
        };

        let error = validate_raw_action_batch_for_test(
            &request,
            &response,
            NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br171_action_response_evidence_mismatch_rejects() {
        let request = request("2026-07-01", "2026-07-26");
        let response = RawActionResponseForTest {
            coverage: request.clone(),
            admission_as_of: iso("2026-07-27"),
            evidence: evidence("TEST_CODE_other_batch"),
            batch: DataBatch::strict(
                vec![action(
                    "2026-07-10",
                    CorporateActionStatus::Implemented,
                    BATCH_ID,
                )],
                provenance(BATCH_ID),
            ),
        };

        let error = validate_raw_action_batch_for_test(
            &request,
            &response,
            NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br171_confirmation_projection_binds_listing_action_and_exact_window_batches() {
        let context = SecurityLifecycleContext {
            instrument: instrument(),
            window_start: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            window_end: NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
            listing: ListingDateState::Available(AdmittedListingDate {
                listed_on: NaiveDate::from_ymd_opt(2001, 2, 3).unwrap(),
                evidence: batch_evidence("TEST_CODE_listing_batch"),
            }),
            corporate_actions: CorporateActionState::Available {
                records: vec![ImplementedCorporateAction {
                    code: "TEST_CODE_600001".to_string(),
                    category: CorporateActionCategory::Distribution,
                    effective_on: NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
                    record_on: Some(NaiveDate::from_ymd_opt(2026, 7, 23).unwrap()),
                    ex_on: Some(NaiveDate::from_ymd_opt(2026, 7, 24).unwrap()),
                    payable_on: None,
                    terms: CorporateActionTerms::distribution(
                        Some(FiniteNumber::new(0.1).unwrap()),
                        None,
                        None,
                        None,
                    )
                    .unwrap(),
                }],
                evidence: batch_evidence("TEST_CODE_actions_batch"),
            },
        };

        let projected = context
            .confirmation_evidence_for(
                NaiveDate::from_ymd_opt(2026, 7, 23).unwrap(),
                NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
            )
            .expect("TEST_CODE exact lifecycle projection");
        assert_eq!(projected.provider, "magic_tdx");
        assert!(projected
            .batch_identity
            .contains("listing=TEST_CODE_listing_batch"));
        assert!(projected
            .batch_identity
            .contains("actions=TEST_CODE_actions_batch"));
        assert_eq!(
            projected.listing_date,
            Some(NaiveDate::from_ymd_opt(2001, 2, 3).unwrap())
        );
        assert!(projected
            .corporate_action_identity
            .as_deref()
            .is_some_and(|identity| identity.starts_with("sha256:")));
    }

    #[test]
    fn br171_unavailable_action_context_cannot_prepare_manual_confirmation() {
        let context = SecurityLifecycleContext {
            instrument: instrument(),
            window_start: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            window_end: NaiveDate::from_ymd_opt(2026, 7, 27).unwrap(),
            listing: ListingDateState::Unavailable {
                evidence: None,
                error: GatewayError::unavailable(
                    LISTING_CAPABILITY,
                    Some(ProviderId::Tdx),
                    true,
                    "TEST_CODE listing unavailable",
                ),
            },
            corporate_actions: CorporateActionState::Unavailable(GatewayError::unavailable(
                ACTIONS_CAPABILITY,
                Some(ProviderId::Tdx),
                true,
                "TEST_CODE actions unavailable",
            )),
        };

        let error = context
            .confirmation_evidence_for(
                NaiveDate::from_ymd_opt(2026, 7, 23).unwrap(),
                NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
            )
            .expect_err("TEST_CODE missing action coverage must fail closed");
        assert_eq!(error.reason_code(), "corporate_action_context_unavailable");
    }

    #[derive(Debug)]
    struct RawActionResponseForTest {
        coverage: CorporateActionRequest,
        admission_as_of: IsoDate,
        evidence: SourceEvidence,
        batch: DataBatch<CorporateAction>,
    }

    fn validate_raw_action_batch_for_test(
        request: &CorporateActionRequest,
        raw: &RawActionResponseForTest,
        admission_today: NaiveDate,
    ) -> Result<BatchEvidence, GatewayError> {
        if &raw.coverage != request {
            return Err(GatewayError::invalid_evidence(
                ACTIONS_CAPABILITY,
                Some(ProviderId::Tdx),
                "TEST_CODE raw response coverage mismatch",
            ));
        }
        let admission =
            parse_iso_date(raw.admission_as_of.as_str()).expect("TEST_CODE admission date");
        if admission > admission_today {
            return Err(GatewayError::invalid_evidence(
                ACTIONS_CAPABILITY,
                Some(ProviderId::Tdx),
                "TEST_CODE raw response future admission",
            ));
        }
        validate_action_parts(request, &raw.evidence, &raw.batch, admission)
    }
}
