//! gRPC 客户端网络层 (合同: grpc/grpc-external-api.md)。
pub mod auth;
pub mod bundle;
pub mod client;
pub mod envelope;
pub mod errors;
pub mod external_pb;
pub(crate) mod external_query_transport;
pub mod external_v1;
pub mod pb;
pub mod provider_attempts;
pub mod retry;
