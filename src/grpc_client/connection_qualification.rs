//! An ExternalV1 authority belongs to one physical dial, not a tonic Channel.
//! The connector refuses every subsequent dial, including queued RPC reconnects.

use super::errors::{ErrorDetail, GrpcError};
use super::external_pb::magic::market::v1::HealthResponse;
use hyper_util::rt::TokioIo;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::net::TcpStream;
use tonic::codegen::{http::Uri, Service};

/// Auditable planned identity only. Deserializing this never creates a session.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnectionIdentity {
    pub(crate) version: u32,
    pub(crate) epoch: String,
    pub(crate) policy_sha256: String,
    pub(crate) descriptor_sha256: String,
}

impl ConnectionIdentity {
    pub(crate) fn validate_recorded(&self) -> bool {
        self.version == 1
            && !self.epoch.is_empty()
            && self.epoch.len() <= 512
            && super::build_identity::BuildIdentityTrust::bundled().is_ok_and(|trust| {
                trust.accepts_recorded_policy(&self.policy_sha256, &self.descriptor_sha256)
            })
    }
}

#[derive(Clone)]
pub(super) struct ConnectionGeneration(Arc<State>);

struct State {
    epoch: String,
    trust: super::build_identity::BuildIdentityTrust,
    dial_claimed: AtomicBool,
    revoked: AtomicBool,
    health: Mutex<Option<HealthResponse>>,
}

impl ConnectionGeneration {
    pub(super) fn new(trust: super::build_identity::BuildIdentityTrust) -> Self {
        Self(Arc::new(State {
            epoch: super::envelope::new_request_id(),
            trust,
            dial_claimed: AtomicBool::new(false),
            revoked: AtomicBool::new(false),
            health: Mutex::new(None),
        }))
    }

    pub(super) fn connector(&self) -> OneDialConnector {
        OneDialConnector(self.clone())
    }

    pub(super) fn observe_health(
        &self,
        request_id: &str,
        response: &HealthResponse,
    ) -> Result<(), GrpcError> {
        let mut health = self.0.health.lock().map_err(|_| unqualified())?;
        *health = None;
        if self.0.revoked.load(Ordering::SeqCst)
            || response.request_id != request_id
            || self.0.trust.current_health(response).is_err()
        {
            return Err(unqualified());
        }
        *health = Some(response.clone());
        Ok(())
    }

    /// Explicit probe Health starts by discarding any previous qualification.
    /// A status or transport error therefore cannot retain the old Health.
    pub(super) fn begin_health_observation(&self) -> Result<(), GrpcError> {
        let mut health = self.0.health.lock().map_err(|_| unqualified())?;
        *health = None;
        if self.0.revoked.load(Ordering::SeqCst) {
            return Err(unqualified());
        }
        Ok(())
    }

    /// A terminal candidate failure spends this physical generation even
    /// before tonic attempts another dial. Later Health cannot revive it.
    pub(super) fn revoke(&self) {
        self.0.revoked.store(true, Ordering::SeqCst);
        if let Ok(mut health) = self.0.health.lock() {
            *health = None;
        }
    }

    pub(super) fn require_qualified(&self) -> Result<(), GrpcError> {
        if self.0.revoked.load(Ordering::SeqCst)
            || self.0.health.lock().map_err(|_| unqualified())?.is_none()
        {
            return Err(unqualified());
        }
        Ok(())
    }

    pub(super) fn epoch(&self) -> &str {
        &self.0.epoch
    }

    pub(super) fn identity(&self) -> ConnectionIdentity {
        ConnectionIdentity {
            version: 1,
            epoch: self.epoch().to_owned(),
            policy_sha256: self.0.trust.current_policy_sha256(),
            descriptor_sha256: self.0.trust.current_descriptor().to_owned(),
        }
    }
}

pub(super) fn unqualified() -> GrpcError {
    GrpcError::FailedPrecondition {
        details: Box::new(ErrorDetail {
            code: "external_connection_unqualified".into(),
            reason_code: Some("external_connection_unqualified".into()),
            retryable: Some(false),
            ..ErrorDetail::default()
        }),
    }
}

#[derive(Clone)]
pub(super) struct OneDialConnector(ConnectionGeneration);

impl Service<Uri> for OneDialConnector {
    type Response = TokioIo<TcpStream>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        // Claim before awaiting DNS/TCP/TLS. A failed first dial is still spent.
        if self.0 .0.dial_claimed.swap(true, Ordering::SeqCst) {
            self.0 .0.revoked.store(true, Ordering::SeqCst);
            return Box::pin(async {
                Err(io::Error::other(
                    "external connection generation was revoked",
                ))
            });
        }
        Box::pin(async move {
            let host = uri
                .host()
                .ok_or_else(|| io::Error::other("missing endpoint host"))?;
            let port = uri
                .port_u16()
                .unwrap_or(if uri.scheme_str() == Some("https") {
                    443
                } else {
                    80
                });
            let stream = TcpStream::connect((host, port)).await?;
            stream.set_nodelay(true)?;
            Ok(TokioIo::new(stream))
        })
    }
}
