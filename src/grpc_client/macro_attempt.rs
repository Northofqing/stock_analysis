use prost::Message as _;

pub(crate) use super::unary_attempt::{
    UnaryAttemptCompletion as MacroAttemptCompletion, UnaryContinuation as MacroContinuation,
    UnaryTrailerMaterial as MacroTrailerMaterial,
};
use super::{
    ClientAuthorization, ContractProfile, GrpcMarketClient, PreparedExternalEndpoint,
    ProfileAuthorizedRequest, ProfileQueryRequest,
};
use crate::data_gateway::{GeneralWebResearchProvider, GlobalNewsProvider};
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::pb::magic::market::v1::Operation;
use crate::grpc_client::retry::{RetryDecision, RetryPolicy};
use crate::grpc_contract::methods::MethodIdentity;

/// Closed data-operation identity, independent of Board/DragonTiger identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MacroQueryIdentity {
    GlobalNews {
        provider: GlobalNewsProvider,
        limit: u32,
    },
    EconomicCalendar,
    SemanticSearch {
        provider: GeneralWebResearchProvider,
        query: String,
        limit: usize,
    },
}

impl MacroQueryIdentity {
    pub(crate) fn operation(&self) -> Operation {
        match self {
            Self::GlobalNews { .. } => Operation::GlobalNews,
            Self::EconomicCalendar => Operation::EconomicCalendar,
            Self::SemanticSearch { .. } => Operation::SemanticSearch,
        }
    }

    pub(crate) fn method(&self, profile: ContractProfile) -> Option<MethodIdentity> {
        MethodIdentity::from_client_operation(profile, self.operation()).ok()
    }

    fn payload(&self) -> serde_json::Value {
        match self {
            Self::GlobalNews { provider, limit } => {
                serde_json::json!({ "provider": provider.wire_name(), "limit": limit })
            }
            Self::EconomicCalendar => serde_json::json!({}),
            Self::SemanticSearch {
                provider,
                query,
                limit,
            } => {
                serde_json::json!({ "provider": provider.wire_name(), "query": query, "limit": limit })
            }
        }
    }
}

/// Reproject original durable Macro bytes using the shared native parser.
pub(crate) fn project_macro_response(
    identity: &MacroQueryIdentity,
    profile: ContractProfile,
    authority: Option<&str>,
    request_id: &str,
    response: crate::grpc_client::pb::magic::market::v1::QueryResponse,
) -> Result<crate::grpc_client::envelope::QueryResult, GrpcError> {
    super::unary_attempt::project_response(
        profile,
        authority,
        request_id,
        identity.operation(),
        response,
    )
}

/// Credential-free material saved by the durable caller. Restoring an attempt
/// requires that caller to have confirmed the preceding result and its Retry.
pub(crate) struct RestoredMacroRequest {
    pub(crate) request_bytes: Vec<u8>,
    pub(crate) request_id: String,
    pub(crate) profile: ContractProfile,
    pub(crate) acquisition_authority: Option<String>,
    pub(crate) retry_policy: (u32, u64, u64, u64),
    pub(crate) next_attempt: u32,
}

/// Credential-free External route plus its canonical data request.
pub(crate) struct RestoredExternalMacroRequest {
    pub(crate) endpoint_uri: String,
    pub(crate) request: RestoredMacroRequest,
}

pub(crate) struct MacroQuerySession {
    client: GrpcMarketClient,
    state: MacroRequestState,
}

pub(super) struct MacroRequestState {
    identity: MacroQueryIdentity,
    request: ProfileQueryRequest,
    attempt_ordinal: u32,
    profile: ContractProfile,
    acquisition_authority: Option<String>,
    retry: RetryPolicy,
}

pub(crate) struct AuthorizedMacroRequest {
    identity: MacroQueryIdentity,
    request: ProfileAuthorizedRequest,
    request_id: String,
    attempt_ordinal: u32,
    profile: ContractProfile,
    acquisition_authority: Option<String>,
    retry: RetryPolicy,
}

/// An authorized data request whose External transport is not connected yet.
pub(crate) struct AuthorizedPreparedMacroRequest {
    prepared: PreparedExternalEndpoint,
    request: AuthorizedMacroRequest,
}

pub(crate) enum ExternalMacroAttemptCompletion {
    ConnectUnavailable {
        error: GrpcError,
        retry_decision: RetryDecision,
        continuation: MacroContinuation,
    },
    Unary(MacroAttemptCompletion),
}

pub(crate) struct AuthorizedMacroAttempt {
    client: GrpcMarketClient,
    identity: MacroQueryIdentity,
    request: ProfileAuthorizedRequest,
    request_id: String,
    attempt_ordinal: u32,
}

impl MacroQuerySession {
    pub(super) fn new(
        client: GrpcMarketClient,
        identity: MacroQueryIdentity,
    ) -> Result<Self, GrpcError> {
        let state = MacroRequestState::new(
            client.profile,
            client.acquisition_authority.clone(),
            client.retry.clone(),
            identity,
        )?;
        Ok(Self { client, state })
    }

    pub(super) fn resume(
        client: GrpcMarketClient,
        identity: MacroQueryIdentity,
        restored: RestoredMacroRequest,
    ) -> Result<Self, GrpcError> {
        let state = MacroRequestState::resume(
            client.profile,
            client.acquisition_authority.as_deref(),
            identity,
            restored,
        )?;
        Ok(Self { client, state })
    }

    /// Consuming the session prevents implicit advance after any completion.
    /// Authorization completes before durable begin and before the network await.
    pub(crate) fn authorize_next(self) -> Result<AuthorizedMacroAttempt, GrpcError> {
        self.state
            .authorize(&self.client.authorization)?
            .bind_connected(self.client)
    }
}

impl MacroRequestState {
    pub(super) fn new(
        profile: ContractProfile,
        acquisition_authority: Option<String>,
        retry: RetryPolicy,
        identity: MacroQueryIdentity,
    ) -> Result<Self, GrpcError> {
        let request =
            super::build_native_profile_query_request(
                profile,
                identity.operation(),
                identity.payload(),
            )?;
        Ok(Self {
            identity,
            request,
            attempt_ordinal: 1,
            profile,
            acquisition_authority,
            retry,
        })
    }

    pub(super) fn resume(
        profile: ContractProfile,
        acquisition_authority: Option<&str>,
        identity: MacroQueryIdentity,
        restored: RestoredMacroRequest,
    ) -> Result<Self, GrpcError> {
        if restored.next_attempt == 0
            || restored.next_attempt > restored.retry_policy.0
            || restored.profile != profile
            || restored.acquisition_authority.as_deref() != acquisition_authority
            || restored.request_id.is_empty()
        {
            return Err(session_mismatch());
        }
        let request = match profile {
            ContractProfile::LocalBridgeV1 => ProfileQueryRequest::Local(
                crate::grpc_client::pb::magic::market::v1::QueryRequest::decode(
                    restored.request_bytes.as_slice(),
                )
                .map_err(|_| session_mismatch())?,
            ),
            ContractProfile::ExternalV1 => ProfileQueryRequest::External(
                crate::grpc_client::external_pb::magic::market::v1::QueryRequest::decode(
                    restored.request_bytes.as_slice(),
                )
                .map_err(|_| session_mismatch())?,
            ),
        };
        if request.encode_to_vec() != restored.request_bytes {
            return Err(session_mismatch());
        }
        let mut expected =
            super::build_native_profile_query_request(
                profile,
                identity.operation(),
                identity.payload(),
            )?;
        match &mut expected {
            ProfileQueryRequest::Local(request) => {
                request
                    .context
                    .as_mut()
                    .ok_or_else(session_mismatch)?
                    .request_id = restored.request_id.clone();
            }
            ProfileQueryRequest::External(request) => {
                request
                    .context
                    .as_mut()
                    .ok_or_else(session_mismatch)?
                    .request_id = restored.request_id.clone();
            }
        }
        // Exact comparison includes protocol, schema, version, content type,
        // canonical JSON, provider routing, allow_unadmitted and request ID.
        if request != expected {
            return Err(session_mismatch());
        }
        let retry = RetryPolicy {
            max_attempts: restored.retry_policy.0,
            base_delay_ms: restored.retry_policy.1,
            max_delay_ms: restored.retry_policy.2,
            jitter_ms: restored.retry_policy.3,
        };
        Ok(Self {
            identity,
            request,
            attempt_ordinal: restored.next_attempt,
            profile,
            acquisition_authority: restored.acquisition_authority,
            retry,
        })
    }

    pub(super) fn authorize(
        self,
        authorization: &ClientAuthorization,
    ) -> Result<AuthorizedMacroRequest, GrpcError> {
        let request_id = self.request.request_id().to_owned();
        if request_id.is_empty() {
            return Err(session_mismatch());
        }
        let request = match self.request {
            ProfileQueryRequest::Local(request) => {
                let mut request = tonic::Request::new(request);
                super::attach_authorization(authorization, &mut request)?;
                ProfileAuthorizedRequest::Local(request)
            }
            ProfileQueryRequest::External(request) => {
                let mut request = tonic::Request::new(request);
                super::attach_authorization(authorization, &mut request)?;
                ProfileAuthorizedRequest::External(request)
            }
        };
        Ok(AuthorizedMacroRequest {
            identity: self.identity,
            request,
            request_id,
            attempt_ordinal: self.attempt_ordinal,
            profile: self.profile,
            acquisition_authority: self.acquisition_authority,
            retry: self.retry,
        })
    }
}

fn session_mismatch() -> GrpcError {
    GrpcError::FailedPrecondition {
        details: Box::default(),
    }
}

impl AuthorizedMacroRequest {
    pub(crate) fn attempt_ordinal(&self) -> u32 {
        self.attempt_ordinal
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn request_bytes(&self) -> Vec<u8> {
        match &self.request {
            ProfileAuthorizedRequest::Local(request) => request.get_ref().encode_to_vec(),
            ProfileAuthorizedRequest::External(request) => request.get_ref().encode_to_vec(),
        }
    }

    pub(crate) fn profile(&self) -> ContractProfile {
        self.profile
    }

    pub(crate) fn acquisition_authority(&self) -> &str {
        self.acquisition_authority.as_deref().unwrap_or_default()
    }

    pub(crate) fn retry_policy(&self) -> (u32, u64, u64, u64) {
        (
            self.retry.max_attempts,
            self.retry.base_delay_ms,
            self.retry.max_delay_ms,
            self.retry.jitter_ms,
        )
    }

    pub(crate) fn bind_connected(
        self,
        mut client: GrpcMarketClient,
    ) -> Result<AuthorizedMacroAttempt, GrpcError> {
        if client.profile != self.profile
            || client.acquisition_authority != self.acquisition_authority
        {
            return Err(session_mismatch());
        }
        client.retry = self.retry;
        Ok(AuthorizedMacroAttempt {
            client,
            identity: self.identity,
            request: self.request,
            request_id: self.request_id,
            attempt_ordinal: self.attempt_ordinal,
        })
    }
}

impl AuthorizedPreparedMacroRequest {
    pub(super) fn new(
        prepared: PreparedExternalEndpoint,
        identity: MacroQueryIdentity,
    ) -> Result<Self, GrpcError> {
        let request = MacroRequestState::new(
            ContractProfile::ExternalV1,
            Some(prepared.acquisition_authority.clone()),
            RetryPolicy::default(),
            identity,
        )?
        .authorize(&prepared.authorization)?;
        Ok(Self { prepared, request })
    }

    pub(super) fn resume(
        prepared: PreparedExternalEndpoint,
        identity: MacroQueryIdentity,
        restored: RestoredExternalMacroRequest,
    ) -> Result<Self, GrpcError> {
        if restored.endpoint_uri != prepared.endpoint_uri {
            return Err(session_mismatch());
        }
        let request = MacroRequestState::resume(
            ContractProfile::ExternalV1,
            Some(prepared.acquisition_authority.as_str()),
            identity,
            restored.request,
        )?
        .authorize(&prepared.authorization)?;
        Ok(Self { prepared, request })
    }

    pub(crate) fn endpoint_uri(&self) -> &str {
        &self.prepared.endpoint_uri
    }

    pub(crate) fn attempt_ordinal(&self) -> u32 {
        self.request.attempt_ordinal()
    }

    pub(crate) fn request_id(&self) -> &str {
        self.request.request_id()
    }

    pub(crate) fn request_bytes(&self) -> Vec<u8> {
        self.request.request_bytes()
    }

    pub(crate) fn profile(&self) -> ContractProfile {
        self.request.profile()
    }

    pub(crate) fn acquisition_authority(&self) -> &str {
        self.request.acquisition_authority()
    }

    pub(crate) fn retry_policy(&self) -> (u32, u64, u64, u64) {
        self.request.retry_policy()
    }

    pub(crate) fn bind_connected(
        self,
        client: GrpcMarketClient,
    ) -> Result<AuthorizedMacroAttempt, GrpcError> {
        if client.endpoint_uri.as_deref() != Some(self.prepared.endpoint_uri.as_str()) {
            return Err(session_mismatch());
        }
        self.request.bind_connected(client)
    }

    pub(crate) async fn execute(self) -> Result<ExternalMacroAttemptCompletion, GrpcError> {
        let client = match self.prepared.connect_once().await {
            Ok(client) => client,
            Err(error @ GrpcError::Unavailable { .. }) => {
                let (retry_decision, continuation) = super::unary_attempt::failure_retry(
                    &error,
                    &self.request.retry,
                    self.request.attempt_ordinal,
                );
                return Ok(ExternalMacroAttemptCompletion::ConnectUnavailable {
                    error,
                    retry_decision,
                    continuation,
                });
            }
            Err(error) => return Err(error),
        };
        let attempt = self.bind_connected(client)?;
        Ok(ExternalMacroAttemptCompletion::Unary(
            attempt.execute().await,
        ))
    }
}

impl AuthorizedMacroAttempt {
    pub(crate) fn attempt_ordinal(&self) -> u32 {
        self.attempt_ordinal
    }
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }
    pub(crate) fn request_bytes(&self) -> Vec<u8> {
        match &self.request {
            ProfileAuthorizedRequest::Local(request) => request.get_ref().encode_to_vec(),
            ProfileAuthorizedRequest::External(request) => request.get_ref().encode_to_vec(),
        }
    }
    pub(crate) fn profile(&self) -> &'static str {
        match self.client.profile {
            ContractProfile::LocalBridgeV1 => "LocalBridgeV1",
            ContractProfile::ExternalV1 => "ExternalV1",
        }
    }
    pub(crate) fn acquisition_authority(&self) -> Option<&str> {
        self.client.acquisition_authority.as_deref()
    }
    pub(crate) fn retry_policy(&self) -> (u32, u64, u64, u64) {
        (
            self.client.retry.max_attempts,
            self.client.retry.base_delay_ms,
            self.client.retry.max_delay_ms,
            self.client.retry.jitter_ms,
        )
    }
    pub(crate) async fn execute(self) -> MacroAttemptCompletion {
        super::unary_attempt::execute(
            self.client,
            self.identity.operation(),
            self.request,
            &self.request_id,
            self.attempt_ordinal,
        )
        .await
    }
}
