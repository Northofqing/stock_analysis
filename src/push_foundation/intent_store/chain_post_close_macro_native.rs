//! Canonical native facts for the full Macro journal. No rendering is persisted as source data.
use super::macro_codec::{self as codec, require, RawResult, RecoveredResult, Request, Result};
use super::ChainPostCloseError;
use crate::data_gateway::review::store_gateway_error;
use crate::data_gateway::GatewayBatch;
use crate::grpc_client::client::macro_attempt::MacroQueryIdentity;
use crate::grpc_client::provider_attempts::ExternalProviderCatalog;
use crate::monitor::push_job::raw_digest;
use crate::search_service::macro_news::runner::QueryKey;
use crate::search_service::macro_news::NativeOutcome;
use serde::{Deserialize, Serialize};

pub(super) fn native_bytes(outcome: &NativeOutcome) -> Result<Vec<u8>> {
    let value = match outcome {
        NativeOutcome::News(outcome) => return codec::native_bytes(outcome),
        NativeOutcome::Economic(Err(error)) => serde_json::json!({
            "version":2,"kind":"EconomicError","error":store_gateway_error(error)
        }),
        NativeOutcome::Economic(Ok(batch)) => {
            let evidence = batch.evidence();
            let records = batch.records().iter().map(|record| serde_json::json!({
                "event_id":record.event_id,"indicator_id":record.indicator_id,
                "country":record.country,"name":record.name,"period":record.period,
                "scheduled_at":record.scheduled_at.to_rfc3339(),
                "released_at":record.released_at.to_rfc3339(),
                "previous":record.previous,"consensus":record.consensus,"actual":record.actual,
                "revised":record.revised,"unit":record.unit,"importance":record.importance,
                "impact":record.impact,"evidence":record.evidence,
            })).collect::<Vec<_>>();
            serde_json::json!({
                "version":2,"kind":if matches!(batch, GatewayBatch::VerifiedEmpty(_)) {"EconomicVerifiedEmpty"} else {"EconomicAvailable"},
                "evidence":{"provider":evidence.provider,"source":evidence.source,
                    "source_at":evidence.source_at,"observed_at":evidence.observed_at,"batch_id":evidence.batch_id},
                "records":records,
            })
        }
        NativeOutcome::Web(Ok(batch)) => {
            serde_json::json!({"version":2,"kind":"Web","batch":batch})
        }
        NativeOutcome::Web(Err(error)) => serde_json::json!({
            "version":2,"kind":"WebError","provider":error.provider(),
            "reason_code":error.reason_code(),"retryable":error.retryable(),
            "stage":error.stage().as_str(),"message":error.message(),
        }),
    };
    codec::encode(&value)
}

/// The logical acquisition specification is shared by the transactional writer
/// and strict reader. Web research deliberately has no BR-159 acquisition row.
pub(super) fn gateway_audit(
    identity: &MacroQueryIdentity,
    outcome: &NativeOutcome,
    time: i64,
) -> Result<(
    &'static str,
    crate::data_gateway::review::OwnedGatewayAuditRecord,
)> {
    use crate::data_gateway::review::map_gateway_audit_record;
    use crate::data_gateway::{economic_calendar, global_news};
    let observed = super::macro_stage::audit_time(time)?;
    let (capability, audit) = match (identity, outcome) {
        (MacroQueryIdentity::GlobalNews { provider, limit }, NativeOutcome::News(result)) => {
            let actual = result
                .as_ref()
                .map(|batch| batch.evidence().provider)
                .unwrap_or(provider.provider_id());
            (
                provider.capability(),
                map_gateway_audit_record(
                    provider.capability(),
                    actual,
                    &global_news::macro_request_hash(*provider, *limit),
                    result,
                    &observed,
                ),
            )
        }
        (MacroQueryIdentity::EconomicCalendar, NativeOutcome::Economic(result)) => {
            let actual = result
                .as_ref()
                .map(|batch| batch.evidence().provider)
                .unwrap_or(crate::market_domain::ProviderId::Jin10);
            (
                economic_calendar::CAPABILITY,
                map_gateway_audit_record(
                    economic_calendar::CAPABILITY,
                    actual,
                    &economic_calendar::macro_request_hash(20, None),
                    result,
                    &observed,
                ),
            )
        }
        _ => return Err(ChainPostCloseError::SchemaRejected),
    };
    Ok((
        capability,
        audit.map_err(|_| ChainPostCloseError::SchemaRejected)?,
    ))
}

pub(super) fn local_unavailable_outcome(
    route: &super::macro_plan_v3::LocalRoute,
) -> Result<NativeOutcome> {
    use super::macro_plan_v3::{LocalRouteState, LocalUnavailableReason};
    require(route.state == LocalRouteState::ObservedUnavailable)?;
    let error = match route
        .reason
        .as_ref()
        .ok_or(ChainPostCloseError::SchemaRejected)?
    {
        LocalUnavailableReason::NotConnectedObserved => {
            crate::data_gateway::GatewayError::unavailable(
                "GrpcBridge",
                None,
                false,
                "explicit Macro Local transport was not connected at the original observation",
            )
        }
        LocalUnavailableReason::OrdinaryPreparationFailure(stored) => {
            crate::data_gateway::review::restore_gateway_error(stored)
                .map_err(|_| ChainPostCloseError::SchemaRejected)?
        }
    };
    Ok(NativeOutcome::Economic(Err(error)))
}

pub(super) fn request_rejected_outcome(
    definition: &crate::search_service::macro_news::runner::Definition,
    query: QueryKey,
) -> Result<NativeOutcome> {
    let MacroQueryIdentity::SemanticSearch {
        provider,
        query,
        limit,
    } = definition
        .identity(query)
        .map_err(|_| ChainPostCloseError::SchemaRejected)?
    else {
        return Err(ChainPostCloseError::SchemaRejected);
    };
    let error =
        crate::data_gateway::general_web_research::validate_request(provider, &query, limit)
            .err()
            .ok_or(ChainPostCloseError::SchemaRejected)?;
    Ok(NativeOutcome::Web(Err(error)))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DataResult {
    pub(super) version: u32,
    pub(super) query: QueryKey,
    pub(super) attempt: u32,
    pub(super) raw: RawResult,
    pub(super) native: Vec<u8>,
    pub(super) native_sha256: String,
}

impl DataResult {
    pub(super) fn capture_external(
        query: QueryKey,
        identity: &MacroQueryIdentity,
        request: &Request,
        attempt: u32,
        completion: &crate::grpc_client::client::macro_attempt::ExternalMacroAttemptCompletion,
        provider_catalog: Option<&ExternalProviderCatalog>,
    ) -> Result<Self> {
        let raw = RawResult::capture_external(completion);
        let (processed, _, _) = raw.project_for(identity, request, attempt, provider_catalog)?;
        let native = native_bytes(&NativeOutcome::project(
            identity,
            request.contract_profile(),
            &processed,
        ))?;
        let value = Self {
            version: 2,
            query,
            attempt,
            raw,
            native_sha256: raw_digest(&native).as_str().to_owned(),
            native,
        };
        value.project(identity, request, provider_catalog)?;
        Ok(value)
    }
    pub(super) fn project(
        &self,
        identity: &MacroQueryIdentity,
        request: &Request,
        provider_catalog: Option<&ExternalProviderCatalog>,
    ) -> Result<(NativeOutcome, RecoveredResult)> {
        require(
            self.version == 2
                && self.attempt > 0
                && raw_digest(&self.native).as_str() == self.native_sha256,
        )?;
        request.validate_for(identity)?;
        let (processed, decision, provider_attempts) =
            self.raw
                .project_for(identity, request, self.attempt, provider_catalog)?;
        let outcome = NativeOutcome::project(identity, request.contract_profile(), &processed);
        require(native_bytes(&outcome)? == self.native)?;
        Ok((
            outcome,
            self.raw
                .clone()
                .into_recovered(decision, provider_attempts)?,
        ))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DataBegin {
    pub(super) version: u32,
    pub(super) query: QueryKey,
    pub(super) attempt: u32,
    pub(super) request_plan_version: u64,
    pub(super) request_sha256: String,
    pub(super) readiness_result_version: Option<u64>,
    pub(super) previous_result_version: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) enum TerminalCause {
    DataResult { version: u64 },
    SharedControlRejected { version: u64 },
    HistoricalControlRejected {
        control_result_version: u64,
        control_result_sha256: String,
        source_final_version: u64,
        source_final_sha256: String,
    },
    RequestRejected,
    LocalRouteUnavailable,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryTerminal {
    pub(super) version: u32,
    pub(super) query: QueryKey,
    pub(super) plan_version: u64,
    pub(super) plan_sha256: String,
    pub(super) request_plan_version: Option<u64>,
    pub(super) request_sha256: Option<String>,
    pub(super) cause: TerminalCause,
    pub(super) native_sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DimensionTerminal {
    pub(super) version: u32,
    pub(super) dimension: u8,
    pub(super) plan_version: u64,
    pub(super) plan_sha256: String,
    pub(super) selected_candidate: Option<u32>,
    pub(super) last_query_terminal_version: Option<u64>,
    pub(super) eligible_candidates: Vec<u32>,
    pub(super) pace_due: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FinalKind {
    Complete,
    BudgetExpired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum ExpiryBasis {
    None,
    WallDeadline,
    MonotonicRemaining { opened_wall_at: i64, elapsed_us: i64 },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FinalizeBegin {
    pub(super) version: u32,
    pub(super) plan_version: u64,
    pub(super) plan_sha256: String,
    pub(super) kind: FinalKind,
    pub(super) started_at: i64,
    pub(super) deadline_at: i64,
    pub(super) facts_sha256: String,
    pub(super) output: Vec<u8>,
    pub(super) output_sha256: String,
    pub(super) pending: Vec<QueryKey>,
    pub(super) expiry: ExpiryBasis,
}
impl FinalizeBegin {
    pub(super) fn validate_bytes(&self) -> Result<()> {
        require(
            self.version == 3
                && self.plan_version > 0
                && self.plan_sha256.len() == 64
                && self.facts_sha256.len() == 64
                && self.started_at >= 0
                && self.deadline_at.checked_sub(self.started_at) == Some(15_000_000)
                && raw_digest(&self.output).as_str() == self.output_sha256
                && std::str::from_utf8(&self.output).is_ok(),
        )?;
        match self.kind {
            FinalKind::Complete => {
                require(self.pending.is_empty() && self.expiry == ExpiryBasis::None)
            }
            FinalKind::BudgetExpired => {
                require(self.output.is_empty())?;
                match self.expiry {
                    ExpiryBasis::None => Err(ChainPostCloseError::SchemaRejected),
                    ExpiryBasis::WallDeadline => Ok(()),
                    ExpiryBasis::MonotonicRemaining { opened_wall_at, elapsed_us } => {
                        let remaining = self.deadline_at.checked_sub(opened_wall_at)
                            .ok_or(ChainPostCloseError::SchemaRejected)?;
                        require(self.started_at <= opened_wall_at
                            && opened_wall_at < self.deadline_at && elapsed_us >= remaining)
                    }
                }
            }
        }
    }

    pub(super) fn validate_recorded_at(&self, recorded_at: i64) -> Result<()> {
        self.validate_bytes()?;
        require(recorded_at >= self.started_at)?;
        match self.expiry {
            ExpiryBasis::None | ExpiryBasis::MonotonicRemaining { .. } => {
                require(recorded_at < self.deadline_at)
            }
            ExpiryBasis::WallDeadline => require(recorded_at >= self.deadline_at),
        }
    }

    pub(super) fn expiry_columns(&self) -> (Option<i64>, Option<i64>) {
        match self.expiry {
            ExpiryBasis::None | ExpiryBasis::WallDeadline => (None, None),
            ExpiryBasis::MonotonicRemaining { opened_wall_at, elapsed_us } => {
                (Some(opened_wall_at), Some(elapsed_us))
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StageFinal {
    pub(super) version: u32,
    pub(super) finalize_begin_version: u64,
    pub(super) finalize_begin_sha256: String,
    pub(super) kind: FinalKind,
    pub(super) output_sha256: String,
}
impl StageFinal {
    pub(super) fn validate(
        &self,
        begin_version: u64,
        begin_bytes: &[u8],
        begin: &FinalizeBegin,
    ) -> Result<()> {
        begin.validate_bytes()?;
        require(
            self.version == 2
                && self.finalize_begin_version == begin_version
                && self.finalize_begin_sha256 == raw_digest(begin_bytes).as_str()
                && self.kind == begin.kind
                && self.output_sha256 == begin.output_sha256,
        )
    }
}

pub(super) fn query_columns(key: QueryKey) -> Result<(&'static str, u8, u32)> {
    match key {
        QueryKey::Gateway(ordinal @ 1..=5) => Ok(("Gateway", ordinal, 1)),
        QueryKey::Web {
            dimension: dimension @ 1..=6,
            candidate: candidate @ 1..,
        } => Ok(("WebDimension", dimension, candidate)),
        _ => Err(ChainPostCloseError::SchemaRejected),
    }
}
