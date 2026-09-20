//! Profile-bound gRPC method identities.
//!
//! LocalBridgeV1 and ExternalV1 intentionally share a protobuf package name but
//! not an operation catalog. Raw ordinals therefore become meaningful only
//! after the contract profile has been selected.

use crate::grpc_client::external_pb::magic::market::v1::Operation as ExternalOperation;
use crate::grpc_client::pb::magic::market::v1::Operation as LocalOperation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractProfile {
    LocalBridgeV1,
    ExternalV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownMethod;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalMethod(LocalOperation);

impl LocalMethod {
    pub fn try_from_raw(raw: i32) -> Result<Self, UnknownMethod> {
        let operation = LocalOperation::try_from(raw).map_err(|_| UnknownMethod)?;
        Self::try_from_operation(operation)
    }

    pub fn as_str_name(self) -> &'static str {
        self.0.as_str_name()
    }

    pub(crate) fn try_from_operation(operation: LocalOperation) -> Result<Self, UnknownMethod> {
        if operation == LocalOperation::Unspecified {
            Err(UnknownMethod)
        } else {
            Ok(Self(operation))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExternalMethod(ExternalOperation);

impl ExternalMethod {
    pub fn try_from_raw(raw: i32) -> Result<Self, UnknownMethod> {
        let operation = ExternalOperation::try_from(raw).map_err(|_| UnknownMethod)?;
        Self::try_from_operation(operation)
    }

    pub fn as_str_name(self) -> &'static str {
        self.0.as_str_name()
    }

    pub(crate) const fn native_operation(self) -> ExternalOperation {
        self.0
    }

    pub(crate) fn try_from_operation(operation: ExternalOperation) -> Result<Self, UnknownMethod> {
        if operation == ExternalOperation::Unspecified {
            Err(UnknownMethod)
        } else {
            Ok(Self(operation))
        }
    }

    fn try_from_local_compat(operation: LocalOperation) -> Result<Self, UnknownMethod> {
        let external = match operation {
            LocalOperation::SecurityMetadata => ExternalOperation::SecurityMetadata,
            LocalOperation::GlobalNews => ExternalOperation::GlobalNews,
            LocalOperation::InstrumentNews => ExternalOperation::InstrumentNews,
            _ => return Err(UnknownMethod),
        };
        Self::try_from_operation(external)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodIdentity {
    Local(LocalMethod),
    External(ExternalMethod),
}

impl MethodIdentity {
    pub const fn profile(self) -> ContractProfile {
        match self {
            Self::Local(_) => ContractProfile::LocalBridgeV1,
            Self::External(_) => ContractProfile::ExternalV1,
        }
    }

    pub fn as_str_name(self) -> &'static str {
        match self {
            Self::Local(method) => method.as_str_name(),
            Self::External(method) => method.as_str_name(),
        }
    }

    pub(crate) fn from_client_operation(
        profile: ContractProfile,
        operation: LocalOperation,
    ) -> Result<Self, UnknownMethod> {
        match profile {
            ContractProfile::LocalBridgeV1 => {
                LocalMethod::try_from_operation(operation).map(Self::Local)
            }
            ContractProfile::ExternalV1 => {
                ExternalMethod::try_from_local_compat(operation).map(Self::External)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_dual_contract_method_identity_keeps_profile_for_colliding_raw_values() {
        let local_61 = LocalMethod::try_from_raw(61).expect("Local raw 61");
        let local_62 = LocalMethod::try_from_raw(62).expect("Local raw 62");
        let external_61 = ExternalMethod::try_from_raw(61).expect("External raw 61");
        let external_62 = ExternalMethod::try_from_raw(62).expect("External raw 62");
        let external_63 = ExternalMethod::try_from_raw(63).expect("External raw 63");

        assert_eq!(local_61.as_str_name(), "OPERATION_CHAIN_BATCH");
        assert_eq!(local_62.as_str_name(), "OPERATION_BENCHMARK_BARS");
        assert_eq!(
            external_61.as_str_name(),
            "OPERATION_CURRENT_AUCTION_OBSERVATIONS"
        );
        assert_eq!(
            external_62.as_str_name(),
            "OPERATION_ECONOMIC_RELEASE_OBSERVATIONS"
        );
        assert_eq!(
            external_63.as_str_name(),
            "OPERATION_ECONOMIC_RELEASE_SCHEDULE"
        );
        assert_eq!(
            MethodIdentity::Local(local_61).profile(),
            ContractProfile::LocalBridgeV1
        );
        assert_eq!(
            MethodIdentity::External(external_61).profile(),
            ContractProfile::ExternalV1
        );
    }

    #[test]
    fn grpc_dual_contract_method_identity_rejects_zero_and_profile_unknown_values() {
        assert_eq!(LocalMethod::try_from_raw(0), Err(UnknownMethod));
        assert_eq!(ExternalMethod::try_from_raw(0), Err(UnknownMethod));
        assert_eq!(LocalMethod::try_from_raw(63), Err(UnknownMethod));
        assert_eq!(ExternalMethod::try_from_raw(64), Err(UnknownMethod));
    }

    #[test]
    fn grpc_dual_contract_external_method_compatibility_is_closed_before_io() {
        for operation in [
            LocalOperation::SecurityMetadata,
            LocalOperation::GlobalNews,
            LocalOperation::InstrumentNews,
        ] {
            assert!(
                MethodIdentity::from_client_operation(ContractProfile::ExternalV1, operation)
                    .is_ok()
            );
        }
        for operation in [
            LocalOperation::RealtimeQuotes,
            LocalOperation::BoardConstituents,
            LocalOperation::ChainBatch,
            LocalOperation::BenchmarkBars,
        ] {
            assert_eq!(
                MethodIdentity::from_client_operation(ContractProfile::ExternalV1, operation),
                Err(UnknownMethod)
            );
        }
    }
}
