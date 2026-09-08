//! Bounded local protocol. The wire can select only a pre-bound initial-intent effect.
#![cfg_attr(not(test), allow(dead_code))]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use super::activation_authorization::observe_unix_peer;
use super::activation_fence::{
    EffectBroker, EffectRequest, FenceError, OperationFact, Scope, ScopeStatus, WorkClass,
};

const MAX_FRAME: usize = 16 * 1024;
const IO_BOUND: Duration = Duration::from_secs(3);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClientIdentity {
    pub(super) client: String,
    pub(super) incarnation: String,
    pub(super) credential: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) enum Command {
    ExecuteCurrent {
        request: EffectRequest,
        wait: bool,
    },
    QueryOperation {
        request: EffectRequest,
        wait: bool,
    },
    Quiesce {
        scope: Scope,
        broker_epoch: String,
        class: WorkClass,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Envelope {
    pub(super) identity: ClientIdentity,
    pub(super) command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Reply {
    Operation(Option<OperationFact>),
    Scope(ScopeStatus),
    Refused(FenceError),
}

async fn read_frame(stream: &mut UnixStream) -> Result<Vec<u8>, FenceError> {
    tokio::time::timeout(IO_BOUND, async {
        let size = stream.read_u32().await.map_err(|_| FenceError::Protocol)? as usize;
        if size == 0 || size > MAX_FRAME {
            return Err(FenceError::Protocol);
        }
        let mut bytes = vec![0; size];
        stream
            .read_exact(&mut bytes)
            .await
            .map_err(|_| FenceError::Protocol)?;
        Ok(bytes)
    })
    .await
    .map_err(|_| FenceError::Protocol)?
}

async fn write_frame(stream: &mut UnixStream, bytes: &[u8]) -> Result<(), FenceError> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err(FenceError::Protocol);
    }
    tokio::time::timeout(IO_BOUND, async {
        stream
            .write_u32(bytes.len() as u32)
            .await
            .map_err(|_| FenceError::Protocol)?;
        stream
            .write_all(bytes)
            .await
            .map_err(|_| FenceError::Protocol)
    })
    .await
    .map_err(|_| FenceError::Protocol)?
}

impl EffectBroker {
    async fn serve_connection(&self, mut stream: UnixStream) -> Result<(), FenceError> {
        let peer = observe_unix_peer(&stream).map_err(|_| FenceError::Unauthorized)?;
        let (uid, gid) = (peer.uid(), peer.gid());
        let bytes = read_frame(&mut stream).await?;
        let envelope: Envelope =
            serde_json::from_slice(&bytes).map_err(|_| FenceError::Protocol)?;
        let result = self.dispatch(envelope, uid, gid).await;
        let reply = result.unwrap_or_else(Reply::Refused);
        write_frame(
            &mut stream,
            &serde_json::to_vec(&reply).map_err(|_| FenceError::Protocol)?,
        )
        .await
        // Failure/EOF here never changes operation state, gate state or worker lifetime.
    }

    async fn dispatch(&self, envelope: Envelope, uid: u32, gid: u32) -> Result<Reply, FenceError> {
        let supervisor = matches!(&envelope.command, Command::Quiesce { .. });
        let identity = envelope.identity;
        self.authenticate(
            &identity.client,
            &identity.incarnation,
            &identity.credential,
            uid,
            gid,
            supervisor,
        )?;
        match envelope.command {
            Command::ExecuteCurrent { request, wait } => {
                if request.client != identity.client
                    || request.client_incarnation != identity.incarnation
                {
                    return Err(FenceError::Unauthorized);
                }
                let fact = self.execute_current(request.clone())?;
                if wait {
                    self.query_bounded(&request).await.map(Reply::Operation)
                } else {
                    Ok(Reply::Operation(Some(fact)))
                }
            }
            Command::QueryOperation { request, wait } => {
                if request.client != identity.client
                    || request.client_incarnation != identity.incarnation
                {
                    return Err(FenceError::Unauthorized);
                }
                if wait {
                    self.query_bounded(&request).await.map(Reply::Operation)
                } else {
                    self.query_operation(&request).map(Reply::Operation)
                }
            }
            Command::Quiesce {
                scope,
                broker_epoch,
                class,
            } => self.quiesce(&scope, &broker_epoch, class).map(Reply::Scope),
        }
    }

    pub(super) async fn serve(self: Arc<Self>, listener: UnixListener) -> Result<(), FenceError> {
        let capacity = Arc::new(tokio::sync::Semaphore::new(32));
        loop {
            let (stream, _) = listener.accept().await.map_err(|_| FenceError::Protocol)?;
            let Ok(permit) = Arc::clone(&capacity).try_acquire_owned() else {
                drop(stream);
                continue;
            };
            let broker = Arc::clone(&self);
            tokio::spawn(async move {
                let _permit = permit;
                let _ = broker.serve_connection(stream).await;
            });
        }
    }
}

pub(super) struct EffectClient;

impl EffectClient {
    pub(super) async fn request(socket: &Path, envelope: &Envelope) -> Result<Reply, FenceError> {
        let mut stream = tokio::time::timeout(IO_BOUND, UnixStream::connect(socket))
            .await
            .map_err(|_| FenceError::Protocol)?
            .map_err(|_| FenceError::Protocol)?;
        write_frame(
            &mut stream,
            &serde_json::to_vec(envelope).map_err(|_| FenceError::Protocol)?,
        )
        .await?;
        let bytes = read_frame(&mut stream).await?;
        serde_json::from_slice(&bytes).map_err(|_| FenceError::Protocol)
    }
}
