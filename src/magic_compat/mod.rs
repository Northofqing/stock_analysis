//! magic-* 类型兼容层 (M5, Task #76)。
//!
//! 目标: 删除 14 个 magic-* git 依赖后, monitor (`--no-default-features`)
//! 仍需编译 gateway 公共 API 泄漏的 magic-core 类型。策略:
//! - `magic-gateway` feature 开 (默认): 重导出 `magic_market_core` 真实类型
//!   → 与迁移前零行为差异 (monitor 与 server 都用真实类型)。
//! - feature 关: 本地镜像类型, 字段/变体/serde 表示与上游 pin
//!   (rev 75ee2a2) 一致。wire 是 JSON, provider 的 Debug 名是 wire 契约
//!   (grpc_server/delegate.rs `pack_ev` 写, convert.rs `parse_provider` 读)。
//!
//! 镜像保真由双向测试钉住: 同一测试在 feature 模式验证真实类型,
//! 在 no-feature 模式验证镜像 — 两者断言必须一致。
//!
//! 依赖全部删除后, 本模块可更名或并入各自归属模块 (见
//! docs/superpowers/plans/2026-08-15-p4-migration.md M5 节)。

pub mod provider_id;

#[cfg(feature = "magic-gateway")]
pub use magic_market_core::ProviderId;
#[cfg(not(feature = "magic-gateway"))]
pub use provider_id::ProviderId;

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire 契约钉住: Debug 名 (delegate pack_ev 写) 与 serde unit-variant
    /// 字符串必须与上游 magic_market_core 一致。feature 模式跑真实类型,
    /// no-feature 模式跑镜像 — 断言相同即证明双向一致。
    #[test]
    fn provider_id_wire_names_match_upstream() {
        let cases: &[(&str, ProviderId)] = &[
            ("Tdx", ProviderId::Tdx),
            ("Tencent", ProviderId::Tencent),
            ("Eastmoney", ProviderId::Eastmoney),
            ("Sina", ProviderId::Sina),
            ("Baostock", ProviderId::Baostock),
            ("Baidu", ProviderId::Baidu),
            ("Tonghuashun", ProviderId::Tonghuashun),
            ("Iwencai", ProviderId::Iwencai),
            ("Cninfo", ProviderId::Cninfo),
            ("Cailianpress", ProviderId::Cailianpress),
            ("Jin10", ProviderId::Jin10),
            ("ThePaper", ProviderId::ThePaper),
            ("Yonhap", ProviderId::Yonhap),
            ("WallstreetCn", ProviderId::WallstreetCn),
            ("Sse", ProviderId::Sse),
            ("Szse", ProviderId::Szse),
            ("Hkex", ProviderId::Hkex),
            ("Cffex", ProviderId::Cffex),
            ("StateCouncil", ProviderId::StateCouncil),
            ("Nbs", ProviderId::Nbs),
            ("Pbc", ProviderId::Pbc),
            ("Cfets", ProviderId::Cfets),
            ("Fred", ProviderId::Fred),
            ("Imf", ProviderId::Imf),
            ("WorldBank", ProviderId::WorldBank),
            ("SecEdgar", ProviderId::SecEdgar),
            ("XinhuaFinance", ProviderId::XinhuaFinance),
            ("Yicai", ProviderId::Yicai),
            ("SecuritiesTimes", ProviderId::SecuritiesTimes),
            ("LocalAnalysis", ProviderId::LocalAnalysis),
            ("LocalTerminal", ProviderId::LocalTerminal),
            ("Custom", ProviderId::Custom),
        ];
        assert_eq!(cases.len(), 32, "ProviderId 变体数必须与上游 75ee2a2 一致");
        for (name, provider) in cases {
            assert_eq!(&format!("{provider:?}"), name, "Debug 名 = wire 契约");
            assert_eq!(
                serde_json::to_string(provider).unwrap(),
                serde_json::to_string(name).unwrap(),
                "serde unit-variant 字符串 = Debug 名"
            );
        }
    }

    #[test]
    fn provider_id_copy_eq_hash_roundtrip() {
        let a = ProviderId::Eastmoney;
        let b = ProviderId::Eastmoney;
        assert_eq!(a, b);
        let mut set = std::collections::HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        let c = a; // Copy
        assert_eq!(c, a);
    }
}
