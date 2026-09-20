use prost::Message as _;

use super::unary_attempt::project_response;
pub(crate) use super::unary_attempt::{
    UnaryAttemptCompletion as BoardAttemptCompletion, UnaryContinuation as BoardContinuation,
    UnaryTrailerMaterial as BoardTrailerMaterial,
};
use super::{ContractProfile, GrpcMarketClient};
use crate::grpc_client::envelope::QueryResult;
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::pb::magic::market::v1::{Operation, QueryRequest, QueryResponse};
use crate::grpc_client::retry::RetryPolicy;

pub(crate) struct BoardQuerySession {
    client: GrpcMarketClient,
    identity: BoardQueryIdentity,
    request: QueryRequest,
    request_id: String,
    next_attempt: u32,
}

pub(crate) struct AuthorizedBoardAttempt {
    client: GrpcMarketClient,
    identity: BoardQueryIdentity,
    request: tonic::Request<QueryRequest>,
    request_id: String,
    attempt_ordinal: u32,
}

#[derive(Clone)]
enum BoardQueryIdentity {
    Directory,
    Memberships { code: String },
    DragonTiger,
}

pub(crate) fn project_board_response(
    profile: ContractProfile,
    acquisition_authority: Option<&str>,
    request_id: &str,
    response: QueryResponse,
) -> Result<QueryResult, GrpcError> {
    project_response(
        profile,
        acquisition_authority,
        request_id,
        Operation::BoardDirectory,
        response,
    )
}

pub(crate) fn project_board_membership_response(
    profile: ContractProfile,
    acquisition_authority: Option<&str>,
    request_id: &str,
    response: QueryResponse,
) -> Result<QueryResult, GrpcError> {
    project_response(
        profile,
        acquisition_authority,
        request_id,
        Operation::BoardConstituents,
        response,
    )
}

pub(crate) fn project_dragon_tiger_response(
    profile: ContractProfile,
    acquisition_authority: Option<&str>,
    request_id: &str,
    response: QueryResponse,
) -> Result<QueryResult, GrpcError> {
    project_response(
        profile,
        acquisition_authority,
        request_id,
        Operation::DragonTiger,
        response,
    )
}

impl BoardQuerySession {
    pub(super) fn new(
        client: GrpcMarketClient,
        payload: serde_json::Value,
    ) -> Result<Self, GrpcError> {
        let request = client.build_profile_query_request(Operation::BoardDirectory, payload)?;
        let request_id = request
            .context
            .as_ref()
            .map(|context| context.request_id.clone())
            .unwrap_or_default();
        Ok(Self {
            client,
            identity: BoardQueryIdentity::Directory,
            request,
            request_id,
            next_attempt: 1,
        })
    }

    pub(super) fn new_memberships(
        client: GrpcMarketClient,
        code: String,
    ) -> Result<Self, GrpcError> {
        let request = client.build_profile_query_request(
            Operation::BoardConstituents,
            serde_json::json!({ "codes": [&code] }),
        )?;
        let request_id = request
            .context
            .as_ref()
            .map(|context| context.request_id.clone())
            .unwrap_or_default();
        Ok(Self {
            client,
            identity: BoardQueryIdentity::Memberships { code },
            request,
            request_id,
            next_attempt: 1,
        })
    }

    pub(super) fn new_dragon_tiger(
        client: GrpcMarketClient,
        date: String,
        disclosure_limit: u32,
        stock_limit: usize,
    ) -> Result<Self, GrpcError> {
        let request = client.build_profile_query_request(
            Operation::DragonTiger,
            serde_json::json!({
                "date": date,
                "disclosure_limit": disclosure_limit,
                "stock_limit": stock_limit,
            }),
        )?;
        let request_id = request
            .context
            .as_ref()
            .map(|context| context.request_id.clone())
            .unwrap_or_default();
        Ok(Self {
            client,
            identity: BoardQueryIdentity::DragonTiger,
            request,
            request_id,
            next_attempt: 1,
        })
    }

    pub(super) fn resume(
        client: GrpcMarketClient,
        request: QueryRequest,
        profile: ContractProfile,
        acquisition_authority: Option<&str>,
        retry_policy: (u32, u64, u64, u64),
        next_attempt: u32,
    ) -> Result<Self, GrpcError> {
        Self::resume_with_identity(
            client,
            BoardQueryIdentity::Directory,
            request,
            profile,
            acquisition_authority,
            retry_policy,
            next_attempt,
        )
    }

    pub(super) fn resume_memberships(
        client: GrpcMarketClient,
        code: String,
        expected_request_id: &str,
        request: QueryRequest,
        profile: ContractProfile,
        acquisition_authority: Option<&str>,
        retry_policy: (u32, u64, u64, u64),
        next_attempt: u32,
    ) -> Result<Self, GrpcError> {
        validate_membership_request(&request, &code, expected_request_id)?;
        if profile != ContractProfile::LocalBridgeV1 {
            return Err(board_session_mismatch());
        }
        Self::resume_with_identity(
            client,
            BoardQueryIdentity::Memberships { code },
            request,
            profile,
            acquisition_authority,
            retry_policy,
            next_attempt,
        )
    }

    pub(super) fn resume_dragon_tiger(
        client: GrpcMarketClient,
        date: &str,
        disclosure_limit: u32,
        stock_limit: usize,
        expected_request_id: &str,
        request: QueryRequest,
        profile: ContractProfile,
        acquisition_authority: Option<&str>,
        retry_policy: (u32, u64, u64, u64),
        next_attempt: u32,
    ) -> Result<Self, GrpcError> {
        validate_dragon_tiger_request(
            &request,
            date,
            disclosure_limit,
            stock_limit,
            expected_request_id,
        )?;
        if profile != ContractProfile::LocalBridgeV1 {
            return Err(board_session_mismatch());
        }
        Self::resume_with_identity(
            client,
            BoardQueryIdentity::DragonTiger,
            request,
            profile,
            acquisition_authority,
            retry_policy,
            next_attempt,
        )
    }

    fn resume_with_identity(
        mut client: GrpcMarketClient,
        identity: BoardQueryIdentity,
        request: QueryRequest,
        profile: ContractProfile,
        acquisition_authority: Option<&str>,
        retry_policy: (u32, u64, u64, u64),
        next_attempt: u32,
    ) -> Result<Self, GrpcError> {
        let request_id = request
            .context
            .as_ref()
            .map(|context| context.request_id.clone())
            .filter(|request_id| !request_id.is_empty())
            .ok_or_else(board_session_mismatch)?;
        if next_attempt == 0
            || next_attempt > retry_policy.0
            || client.profile != profile
            || client.acquisition_authority.as_deref() != acquisition_authority
        {
            return Err(board_session_mismatch());
        }
        client.retry = RetryPolicy {
            max_attempts: retry_policy.0,
            base_delay_ms: retry_policy.1,
            max_delay_ms: retry_policy.2,
            jitter_ms: retry_policy.3,
        };
        Ok(Self {
            client,
            identity,
            request,
            request_id,
            next_attempt,
        })
    }

    /// Authentication is attached here, before durable begin and before the network await.
    pub(crate) fn authorize_next(&mut self) -> Result<AuthorizedBoardAttempt, GrpcError> {
        let mut authorized = tonic::Request::new(self.request.clone());
        self.client.attach_request_auth(&mut authorized)?;
        let attempt_ordinal = self.next_attempt;
        self.next_attempt = self.next_attempt.saturating_add(1);
        Ok(AuthorizedBoardAttempt {
            client: self.client.clone(),
            identity: self.identity.clone(),
            request: authorized,
            request_id: self.request_id.clone(),
            attempt_ordinal,
        })
    }
}

fn board_session_mismatch() -> GrpcError {
    GrpcError::FailedPrecondition {
        details: Box::default(),
    }
}

fn validate_membership_request(
    request: &QueryRequest,
    expected_code: &str,
    expected_request_id: &str,
) -> Result<(), GrpcError> {
    let valid_context = request.context.as_ref().is_some_and(|context| {
        context.protocol_version == 1
            && !expected_request_id.is_empty()
            && context.request_id == expected_request_id
    });
    let valid_payload = request.payload.as_ref().is_some_and(|payload| {
        payload.schema == "board.constituents"
            && payload.schema_version == 1
            && payload.content_type == "application/json; charset=utf-8"
            && payload.data
                == serde_json::to_vec(&serde_json::json!({ "codes": [expected_code] }))
                    .expect("membership request JSON is serializable")
    });
    if !valid_context
        || !valid_payload
        || !request.preferred_provider.is_empty()
        || request.allow_unadmitted
    {
        return Err(board_session_mismatch());
    }
    Ok(())
}

fn validate_dragon_tiger_request(
    request: &QueryRequest,
    expected_date: &str,
    expected_disclosure_limit: u32,
    expected_stock_limit: usize,
    expected_request_id: &str,
) -> Result<(), GrpcError> {
    let valid_context = request.context.as_ref().is_some_and(|context| {
        context.protocol_version == 1
            && !expected_request_id.is_empty()
            && context.request_id == expected_request_id
    });
    let valid_payload = request.payload.as_ref().is_some_and(|payload| {
        payload.schema == "market.dragon_tiger"
            && payload.schema_version == 1
            && payload.content_type == "application/json; charset=utf-8"
            && payload.data
                == serde_json::to_vec(&serde_json::json!({
                    "date": expected_date,
                    "disclosure_limit": expected_disclosure_limit,
                    "stock_limit": expected_stock_limit,
                }))
                .expect("dragon-tiger request JSON is serializable")
    });
    if !valid_context
        || !valid_payload
        || !request.preferred_provider.is_empty()
        || request.allow_unadmitted
    {
        return Err(board_session_mismatch());
    }
    Ok(())
}

impl AuthorizedBoardAttempt {
    pub(crate) fn attempt_ordinal(&self) -> u32 {
        self.attempt_ordinal
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn request_bytes(&self) -> Vec<u8> {
        self.request.get_ref().encode_to_vec()
    }

    pub(crate) fn profile(&self) -> &'static str {
        match self.client.profile {
            super::ContractProfile::LocalBridgeV1 => "LocalBridgeV1",
            super::ContractProfile::ExternalV1 => "ExternalV1",
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

    pub(crate) async fn execute(self) -> BoardAttemptCompletion {
        let operation = match &self.identity {
            BoardQueryIdentity::Directory => Operation::BoardDirectory,
            BoardQueryIdentity::Memberships { code } => {
                debug_assert!(!code.is_empty());
                Operation::BoardConstituents
            }
            BoardQueryIdentity::DragonTiger => Operation::DragonTiger,
        };
        super::unary_attempt::execute(
            self.client,
            operation,
            super::ProfileAuthorizedRequest::Local(self.request),
            &self.request_id,
            self.attempt_ordinal,
        )
        .await
    }
}
