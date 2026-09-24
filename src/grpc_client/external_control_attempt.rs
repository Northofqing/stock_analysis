//! Closed native attempts for External system-service controls.

use prost::Message as _;

use super::unary_attempt::{capture_status_material, UnaryTrailerMaterial};
use super::{ContractProfile, ExternalSystemCall, GrpcMarketClient, PreparedExternalEndpoint};
use crate::grpc_client::errors::{ErrorDetail, GrpcError, StatusErrorContext};
use crate::grpc_client::external_pb::magic::market::v1::{
    CapabilitiesRequest, CapabilitiesResponse, HealthRequest, HealthResponse, RequestContext,
};
use crate::grpc_client::provider_attempts::ExternalProviderCatalog;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExternalControlKind {
    Health,
    Capabilities,
}

/// Credential-free request and route material. Durable callers may rebuild
/// this value from its fields, but only the typed resume methods validate and
/// authorize it for execution.
#[derive(Clone)]
pub(crate) struct ExternalControlRequestMaterial {
    pub(crate) kind: ExternalControlKind,
    pub(crate) request_bytes: Vec<u8>,
    pub(crate) request_id: String,
    pub(crate) profile: ContractProfile,
    pub(crate) endpoint_uri: String,
    pub(crate) acquisition_authority: String,
}

impl ExternalControlRequestMaterial {
    pub(crate) fn kind(&self) -> ExternalControlKind {
        self.kind
    }

    pub(crate) fn request_bytes(&self) -> &[u8] {
        &self.request_bytes
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn profile(&self) -> ContractProfile {
        self.profile
    }

    pub(crate) fn endpoint_uri(&self) -> &str {
        &self.endpoint_uri
    }

    pub(crate) fn acquisition_authority(&self) -> &str {
        &self.acquisition_authority
    }
}

trait ClosedExternalControlRequest: prost::Message + Default {
    const KIND: ExternalControlKind;

    fn from_context(context: RequestContext) -> Self;
    fn context(&self) -> Option<&RequestContext>;
}

impl ClosedExternalControlRequest for HealthRequest {
    const KIND: ExternalControlKind = ExternalControlKind::Health;

    fn from_context(context: RequestContext) -> Self {
        Self {
            context: Some(context),
        }
    }

    fn context(&self) -> Option<&RequestContext> {
        self.context.as_ref()
    }
}

impl ClosedExternalControlRequest for CapabilitiesRequest {
    const KIND: ExternalControlKind = ExternalControlKind::Capabilities;

    fn from_context(context: RequestContext) -> Self {
        Self {
            context: Some(context),
        }
    }

    fn context(&self) -> Option<&RequestContext> {
        self.context.as_ref()
    }
}

struct ExternalControlRequestCore<T> {
    request: tonic::Request<T>,
    request_id: String,
    kind: ExternalControlKind,
    profile: ContractProfile,
    endpoint_uri: String,
    acquisition_authority: String,
}

impl<T> ExternalControlRequestCore<T>
where
    T: ClosedExternalControlRequest,
{
    fn new(prepared: &PreparedExternalEndpoint) -> Result<Self, GrpcError> {
        let request_id = crate::grpc_client::envelope::new_request_id();
        let request = T::from_context(RequestContext {
            protocol_version: 1,
            request_id: request_id.clone(),
        });
        Self::authorize(prepared, request_id, request)
    }

    fn resume(
        prepared: &PreparedExternalEndpoint,
        material: ExternalControlRequestMaterial,
    ) -> Result<Self, GrpcError> {
        if material.kind != T::KIND
            || material.profile != ContractProfile::ExternalV1
            || material.endpoint_uri != prepared.endpoint_uri
            || material.acquisition_authority != prepared.acquisition_authority
            || material.request_id.is_empty()
        {
            return Err(request_mismatch());
        }
        let request =
            T::decode(material.request_bytes.as_slice()).map_err(|_| request_mismatch())?;
        if request.encode_to_vec() != material.request_bytes {
            return Err(request_mismatch());
        }
        let context = request.context().ok_or_else(request_mismatch)?;
        if context.protocol_version != 1
            || context.request_id.is_empty()
            || context.request_id != material.request_id
        {
            return Err(request_mismatch());
        }
        Self::authorize(prepared, material.request_id, request)
    }

    fn authorize(
        prepared: &PreparedExternalEndpoint,
        request_id: String,
        request: T,
    ) -> Result<Self, GrpcError> {
        let mut request = tonic::Request::new(request);
        prepared.attach_request_auth(&mut request)?;
        Ok(Self {
            request,
            request_id,
            kind: T::KIND,
            profile: ContractProfile::ExternalV1,
            endpoint_uri: prepared.endpoint_uri.clone(),
            acquisition_authority: prepared.acquisition_authority.clone(),
        })
    }

    fn request_material(&self) -> ExternalControlRequestMaterial {
        ExternalControlRequestMaterial {
            kind: self.kind,
            request_bytes: self.request.get_ref().encode_to_vec(),
            request_id: self.request_id.clone(),
            profile: self.profile,
            endpoint_uri: self.endpoint_uri.clone(),
            acquisition_authority: self.acquisition_authority.clone(),
        }
    }

    fn into_request(self) -> (String, tonic::Request<T>) {
        (self.request_id, self.request)
    }
}

enum ExternalControlTarget {
    Prepared(PreparedExternalEndpoint),
    Connected(GrpcMarketClient),
}

impl ExternalControlTarget {
    async fn connect(self) -> Result<GrpcMarketClient, GrpcError> {
        match self {
            Self::Prepared(prepared) => prepared.connect_once().await,
            Self::Connected(client) => Ok(client),
        }
    }
}

pub(crate) struct AuthorizedHealthAttempt {
    core: ExternalControlRequestCore<HealthRequest>,
    target: ExternalControlTarget,
}

pub(crate) struct AuthorizedCapabilitiesAttempt {
    core: ExternalControlRequestCore<CapabilitiesRequest>,
    target: ExternalControlTarget,
}

pub(crate) struct ExternalControlCompletion<T> {
    material: OwnedExternalControlMaterial<T>,
    processed: Result<(), GrpcError>,
    connected: Option<GrpcMarketClient>,
}

enum OwnedExternalControlMaterial<T> {
    ConnectUnavailable,
    Response {
        bytes: Vec<u8>,
        response: T,
    },
    Status {
        code: i32,
        details: Vec<u8>,
        error_detail_trailer: UnaryTrailerMaterial,
    },
}

pub(crate) enum ExternalControlResultMaterial<'a, T> {
    ConnectUnavailable {
        error: &'a GrpcError,
    },
    Response {
        bytes: &'a [u8],
        response: &'a T,
    },
    Status {
        code: i32,
        details: &'a [u8],
        error_detail_trailer: &'a UnaryTrailerMaterial,
        error: &'a GrpcError,
    },
}

impl AuthorizedHealthAttempt {
    pub(super) fn new(prepared: PreparedExternalEndpoint) -> Result<Self, GrpcError> {
        let core = ExternalControlRequestCore::new(&prepared)?;
        Ok(Self {
            core,
            target: ExternalControlTarget::Prepared(prepared),
        })
    }

    pub(super) fn resume(
        prepared: PreparedExternalEndpoint,
        material: ExternalControlRequestMaterial,
    ) -> Result<Self, GrpcError> {
        let core = ExternalControlRequestCore::resume(&prepared, material)?;
        Ok(Self {
            core,
            target: ExternalControlTarget::Prepared(prepared),
        })
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.core.request_id
    }

    pub(crate) fn request_bytes(&self) -> Vec<u8> {
        self.core.request.get_ref().encode_to_vec()
    }

    pub(crate) fn request_material(&self) -> ExternalControlRequestMaterial {
        self.core.request_material()
    }

    pub(crate) async fn execute(self) -> ExternalControlCompletion<HealthResponse> {
        let mut client = match self.target.connect().await {
            Ok(client) => client,
            Err(error) => {
                return ExternalControlCompletion::connect_unavailable(error);
            }
        };
        let (request_id, request) = self.core.into_request();
        match client.execute_external_health(request).await {
            ExternalSystemCall::Response(response) => {
                let bytes = response.encode_to_vec();
                let processed = validate_health_response_id(&request_id, &response);
                let connected = processed.is_ok().then_some(client);
                ExternalControlCompletion {
                    material: OwnedExternalControlMaterial::Response { bytes, response },
                    processed,
                    connected,
                }
            }
            ExternalSystemCall::UnaryStatus(status) => {
                ExternalControlCompletion::status(status, &request_id)
            }
        }
    }
}

impl AuthorizedCapabilitiesAttempt {
    pub(super) fn new(prepared: PreparedExternalEndpoint) -> Result<Self, GrpcError> {
        let core = ExternalControlRequestCore::new(&prepared)?;
        Ok(Self {
            core,
            target: ExternalControlTarget::Prepared(prepared),
        })
    }

    pub(super) fn resume(
        prepared: PreparedExternalEndpoint,
        material: ExternalControlRequestMaterial,
    ) -> Result<Self, GrpcError> {
        let core = ExternalControlRequestCore::resume(&prepared, material)?;
        Ok(Self {
            core,
            target: ExternalControlTarget::Prepared(prepared),
        })
    }

    pub(crate) fn request_id(&self) -> &str {
        &self.core.request_id
    }

    pub(crate) fn request_bytes(&self) -> Vec<u8> {
        self.core.request.get_ref().encode_to_vec()
    }

    pub(crate) fn request_material(&self) -> ExternalControlRequestMaterial {
        self.core.request_material()
    }

    pub(crate) fn bind_connected(mut self, client: GrpcMarketClient) -> Result<Self, GrpcError> {
        if !matches!(&self.target, ExternalControlTarget::Prepared(_))
            || client.profile != self.core.profile
            || client.endpoint_uri.as_deref() != Some(self.core.endpoint_uri.as_str())
            || client.acquisition_authority.as_deref()
                != Some(self.core.acquisition_authority.as_str())
        {
            return Err(request_mismatch());
        }
        self.target = ExternalControlTarget::Connected(client);
        Ok(self)
    }

    pub(crate) async fn execute(self) -> ExternalControlCompletion<CapabilitiesResponse> {
        let mut client = match self.target.connect().await {
            Ok(client) => client,
            Err(error) => {
                return ExternalControlCompletion::connect_unavailable(error);
            }
        };
        let (request_id, request) = self.core.into_request();
        match client.execute_external_capabilities(request).await {
            ExternalSystemCall::Response(response) => {
                let bytes = response.encode_to_vec();
                let processed = client.accept_external_capabilities(&request_id, &response);
                let connected = processed.is_ok().then_some(client);
                ExternalControlCompletion {
                    material: OwnedExternalControlMaterial::Response { bytes, response },
                    processed,
                    connected,
                }
            }
            ExternalSystemCall::UnaryStatus(status) => {
                ExternalControlCompletion::status(status, &request_id)
            }
        }
    }
}

impl<T> ExternalControlCompletion<T> {
    fn connect_unavailable(error: GrpcError) -> Self {
        Self {
            material: OwnedExternalControlMaterial::ConnectUnavailable,
            processed: Err(error),
            connected: None,
        }
    }

    fn status(status: tonic::Status, request_id: &str) -> Self {
        let (code, details, error_detail_trailer) = capture_status_material(&status);
        Self {
            material: OwnedExternalControlMaterial::Status {
                code,
                details,
                error_detail_trailer,
            },
            processed: Err(GrpcError::from_status(
                status,
                StatusErrorContext::control(ContractProfile::ExternalV1, request_id),
            )),
            connected: None,
        }
    }

    pub(crate) fn result_material(&self) -> ExternalControlResultMaterial<'_, T> {
        match &self.material {
            OwnedExternalControlMaterial::ConnectUnavailable => {
                ExternalControlResultMaterial::ConnectUnavailable {
                    error: self
                        .processed
                        .as_ref()
                        .expect_err("connect failure must retain its typed error"),
                }
            }
            OwnedExternalControlMaterial::Response { bytes, response } => {
                ExternalControlResultMaterial::Response { bytes, response }
            }
            OwnedExternalControlMaterial::Status {
                code,
                details,
                error_detail_trailer,
            } => ExternalControlResultMaterial::Status {
                code: *code,
                details,
                error_detail_trailer,
                error: self
                    .processed
                    .as_ref()
                    .expect_err("remote status must retain its typed error"),
            },
        }
    }

    pub(crate) fn processed(&self) -> Result<(), &GrpcError> {
        self.processed.as_ref().map(|_| ()).map_err(|error| error)
    }

    pub(crate) fn into_connected_client(self) -> Option<GrpcMarketClient> {
        self.connected
    }
}

pub(crate) fn validate_health_response_id(
    request_id: &str,
    response: &HealthResponse,
) -> Result<(), GrpcError> {
    if response.request_id == request_id {
        Ok(())
    } else {
        Err(GrpcError::FailedPrecondition {
            details: Box::new(ErrorDetail {
                code: "health_request_id_mismatch".to_owned(),
                request_id: None,
                reason_code: Some("health_request_id_mismatch".to_owned()),
                retryable: Some(false),
                ..ErrorDetail::default()
            }),
        })
    }
}

pub(crate) fn validate_capabilities_response_id(
    request_id: &str,
    response: &CapabilitiesResponse,
) -> Result<(), GrpcError> {
    if response.request_id == request_id {
        Ok(())
    } else {
        Err(GrpcError::FailedPrecondition {
            details: Box::new(ErrorDetail {
                code: "capabilities_request_id_mismatch".to_owned(),
                request_id: None,
                reason_code: Some("capabilities_request_id_mismatch".to_owned()),
                retryable: Some(false),
                ..ErrorDetail::default()
            }),
        })
    }
}

pub(crate) fn validated_external_provider_catalog(
    request_id: &str,
    response: &CapabilitiesResponse,
) -> Result<ExternalProviderCatalog, GrpcError> {
    validate_capabilities_response_id(request_id, response)?;
    Ok(ExternalProviderCatalog::from_request_id_validated_capabilities(response))
}

fn request_mismatch() -> GrpcError {
    GrpcError::FailedPrecondition {
        details: Box::new(ErrorDetail {
            code: "external_control_request_mismatch".to_owned(),
            reason_code: Some("external_control_request_mismatch".to_owned()),
            retryable: Some(false),
            ..ErrorDetail::default()
        }),
    }
}
