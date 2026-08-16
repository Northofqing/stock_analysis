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

pub mod evidence;
pub mod instrument;
pub mod provider_id;

#[cfg(feature = "magic-gateway")]
pub use magic_market_core::{
    AssetClass, CoreError, EvidenceTimestamp, Exchange, InstrumentId, NonEmptyText, ProviderId,
    SourceEvidence,
};
#[cfg(not(feature = "magic-gateway"))]
pub use evidence::{EvidenceTimestamp, NonEmptyText, SourceEvidence};
#[cfg(not(feature = "magic-gateway"))]
pub use instrument::{AssetClass, CoreError, Exchange, InstrumentId};
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

    /// Phase 2: Exchange/AssetClass 变体名 (wire 契约) 与上游一致。
    #[test]
    fn instrument_variants_wire_names_match_upstream() {
        let exchange_cases: &[(&str, Exchange)] = &[
            ("Shanghai", Exchange::Shanghai),
            ("Shenzhen", Exchange::Shenzhen),
            ("Beijing", Exchange::Beijing),
        ];
        assert_eq!(exchange_cases.len(), 3, "Exchange 变体数必须与上游 75ee2a2 一致");
        for (name, exchange) in exchange_cases {
            assert_eq!(&format!("{exchange:?}"), name, "Debug 名 = wire 契约");
            assert_eq!(
                serde_json::to_string(exchange).unwrap(),
                serde_json::to_string(name).unwrap(),
                "serde unit-variant 字符串 = Debug 名"
            );
        }
        let asset_cases: &[(&str, AssetClass)] = &[
            ("Equity", AssetClass::Equity),
            ("Index", AssetClass::Index),
            ("Fund", AssetClass::Fund),
            ("Bond", AssetClass::Bond),
            ("Option", AssetClass::Option),
        ];
        assert_eq!(asset_cases.len(), 5, "AssetClass 变体数必须与上游 75ee2a2 一致");
        for (name, asset_class) in asset_cases {
            assert_eq!(&format!("{asset_class:?}"), name, "Debug 名 = wire 契约");
            assert_eq!(
                serde_json::to_string(asset_class).unwrap(),
                serde_json::to_string(name).unwrap(),
                "serde unit-variant 字符串 = Debug 名"
            );
        }
    }

    /// Phase 2: InstrumentId JSON 表示 (grpc delegate 视图 + serde 往返) 与上游一致。
    #[test]
    fn instrument_id_json_repr_matches_upstream() {
        let id = InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(
            json,
            r#"{"exchange":"Shanghai","code":"600396","asset_class":"Equity"}"#
        );
        let parsed: InstrumentId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.exchange(), Exchange::Shanghai);
        assert_eq!(parsed.code(), "600396");
        assert_eq!(parsed.asset_class(), AssetClass::Equity);
        assert!(InstrumentId::new(Exchange::Shanghai, " ", AssetClass::Equity).is_err());
    }

    /// Phase 2: SourceEvidence JSON 表示 (wire 契约) 与上游一致。
    #[test]
    fn source_evidence_json_repr_matches_upstream() {
        let evidence = SourceEvidence::new(
            ProviderId::Eastmoney,
            "2099-01-02T10:00:01+08:00",
            "batch-1",
        )
        .unwrap()
        .with_source_at("2099-01-02T10:00:00+08:00")
        .unwrap();
        let json = serde_json::to_string(&evidence).unwrap();
        assert_eq!(
            json,
            r#"{"provider":"Eastmoney","source_at":"2099-01-02T10:00:00+08:00","observed_at":"2099-01-02T10:00:01+08:00","batch_id":"batch-1"}"#
        );
        let parsed: SourceEvidence = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, evidence);
        assert_eq!(parsed.provider(), ProviderId::Eastmoney);
        assert_eq!(parsed.source_at(), Some("2099-01-02T10:00:00+08:00"));
        assert_eq!(parsed.observed_at(), "2099-01-02T10:00:01+08:00");
        assert_eq!(parsed.batch_id(), "batch-1");

        let minimal =
            SourceEvidence::new(ProviderId::Sina, "2099-01-02T10:00:01+08:00", "b2").unwrap();
        assert_eq!(
            serde_json::to_string(&minimal).unwrap(),
            r#"{"provider":"Sina","source_at":null,"observed_at":"2099-01-02T10:00:01+08:00","batch_id":"b2"}"#
        );
    }

    /// Phase 2: EvidenceTimestamp 接受集 (admission 契约) 与上游 probe.rs 一致。
    #[test]
    fn evidence_timestamp_parse_matches_upstream_format_set() {
        assert!(EvidenceTimestamp::parse("2026-08-16T10:00:00+08:00").is_ok());
        assert!(EvidenceTimestamp::parse("2026-08-16").is_ok());
        assert!(EvidenceTimestamp::parse("1770000000").is_ok());
        assert!(EvidenceTimestamp::parse("1770000000.123456789").is_ok());
        assert!(EvidenceTimestamp::parse("unix-ms:1770000000123").is_ok());
        assert!(EvidenceTimestamp::parse("2026-08-16T10:00:00Z").is_ok());
        assert!(EvidenceTimestamp::parse("2026-08-16 10:00:00+08:00").is_ok());
        assert!(EvidenceTimestamp::parse("not-a-time").is_err());
        assert!(EvidenceTimestamp::parse("2026-13-40T10:00:00+08:00").is_err());
        assert!(EvidenceTimestamp::parse("unix-ms:").is_err());

        // parse_instant: 拒绝无时区后缀的 wall-clock, 接受 epoch/unix-ms/带后缀 ISO
        assert!(EvidenceTimestamp::parse_instant("1770000000").is_ok());
        assert!(EvidenceTimestamp::parse_instant("unix-ms:1770000000123").is_ok());
        assert!(EvidenceTimestamp::parse_instant("2026-08-16T10:00:00+08:00").is_ok());
        assert!(EvidenceTimestamp::parse_instant("2026-08-16").is_err());
        assert!(EvidenceTimestamp::parse_instant("2026-08-16T10:00:00").is_err());
    }
}
