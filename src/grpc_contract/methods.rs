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
        // 本地专有 op：外部合同无对应名（外部 61/62/63 是拍卖/经济数据族）。必须显式拒绝——
        // 若按 raw 序号透传，本地 61 CHAIN_BATCH / 62 BENCHMARK_BARS 会撞成外部的
        // CURRENT_AUCTION_OBSERVATIONS / ECONOMIC_RELEASE_OBSERVATIONS。
        if matches!(
            operation,
            LocalOperation::ChainBatch | LocalOperation::BenchmarkBars
        ) {
            return Err(UnknownMethod);
        }
        // 其余 op 在两份合同中同名同号（由 external_shared_operations_agree_by_name_and_number 守卫），
        // 因此按序号对齐，并额外校验名字一致以拒绝未来单侧 proto 漂移。
        let external = ExternalOperation::try_from(operation as i32).map_err(|_| UnknownMethod)?;
        if external.as_str_name() != operation.as_str_name() {
            return Err(UnknownMethod);
        }
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
        // 本地枚举到 62（61/62 为本地专有扩展 CHAIN_BATCH / BENCHMARK_BARS）。
        assert_eq!(LocalMethod::try_from_raw(63), Err(UnknownMethod));
        // 外部枚举到 65（61..65 为拍卖 / 经济数据 / 官方出版物族）。
        // 原断言为 64，随外部 proto 扩到 65 后已陈旧——62 上验证过与本次改动无关。
        assert!(ExternalMethod::try_from_raw(64).is_ok());
        assert!(ExternalMethod::try_from_raw(65).is_ok());
        assert_eq!(ExternalMethod::try_from_raw(66), Err(UnknownMethod));
    }

    #[test]
    fn external_shared_operations_agree_by_name_and_number() {
        // 守卫：本地 op 若在外部合同中存在，必须同名同号——否则按序号对齐会静默错配。
        let mut shared = 0usize;
        for raw in 0..=64i32 {
            let Ok(local) = LocalOperation::try_from(raw) else {
                continue;
            };
            if matches!(
                local,
                LocalOperation::ChainBatch | LocalOperation::BenchmarkBars
            ) {
                // 仅本地专有：外部同号位必须是别的名字。
                if let Ok(external) = ExternalOperation::try_from(raw) {
                    assert_ne!(external.as_str_name(), local.as_str_name());
                }
                continue;
            }
            let external = ExternalOperation::try_from(raw)
                .expect("shared op must exist in external contract");
            assert_eq!(external.as_str_name(), local.as_str_name());
            shared += 1;
        }
        assert!(shared >= 60, "共有 op 数量异常: {shared}");
    }

    #[test]
    fn grpc_dual_contract_external_shared_operations_route_to_external() {
        // 扩展后：共有 op 在 ExternalV1 profile 下全部放行，且映射到同名方法。
        for operation in [
            LocalOperation::SecurityMetadata,
            LocalOperation::MarketAnnouncements,
            LocalOperation::GlobalNews,
            LocalOperation::InstrumentNews,
            // 本次新增路由（此前被白名单拒绝）
            LocalOperation::RealtimeQuotes,
            LocalOperation::HistoricalBars,
            LocalOperation::MinuteData,
            LocalOperation::MoneyFlows,
            LocalOperation::OrderBooks,
            LocalOperation::BoardConstituents,
            LocalOperation::BoardDirectory,
            LocalOperation::TechnicalBars,
            LocalOperation::FinancialStatements,
            LocalOperation::CorporateActions,
        ] {
            let identity =
                MethodIdentity::from_client_operation(ContractProfile::ExternalV1, operation)
                    .unwrap_or_else(|_| panic!("{operation:?} 应放行"));
            assert_eq!(identity.profile(), ContractProfile::ExternalV1);
            assert_eq!(identity.as_str_name(), operation.as_str_name());
        }
    }

    #[test]
    fn grpc_dual_contract_local_only_operations_are_rejected_before_io() {
        // 仅本地专有 op：外部目录无对应名，必须仍在 IO 之前拒绝。
        for operation in [LocalOperation::ChainBatch, LocalOperation::BenchmarkBars] {
            assert_eq!(
                MethodIdentity::from_client_operation(ContractProfile::ExternalV1, operation),
                Err(UnknownMethod)
            );
        }
    }
}
