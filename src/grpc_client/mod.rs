//! gRPC 客户端网络层 (合同: grpc/grpc-external-api.md)。
pub(crate) mod archived_external_20260928;
pub(crate) mod archived_external_20261001;
pub mod auth;
pub mod build_identity;
pub mod bundle;
pub mod client;
pub(crate) mod connection_qualification;
pub mod envelope;
pub mod errors;
pub(crate) mod external_decoder;
pub mod external_pb;
pub(crate) mod external_query_transport;
pub mod external_v1;
pub(crate) mod historical_external;
pub mod pb;
pub mod provider_attempts;
pub mod retry;
