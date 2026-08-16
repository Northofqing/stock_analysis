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

pub mod bars;
pub mod evidence;
pub mod instrument;
pub mod lifecycle;
pub mod market;
pub mod provider_id;
pub mod ranking;
pub mod record;
pub mod tdx;
pub mod value;

#[cfg(not(feature = "magic-gateway"))]
pub use bars::{Adjustment, Bar, BarInterval};
#[cfg(not(feature = "magic-gateway"))]
pub use evidence::{EvidenceTimestamp, NonEmptyText, SourceEvidence};
#[cfg(not(feature = "magic-gateway"))]
pub use instrument::{AssetClass, CoreError, Exchange, InstrumentId};
#[cfg(not(feature = "magic-gateway"))]
pub use lifecycle::{
    CorporateActionCategory, CorporateActionStatus, CorporateActionTerms, UnverifiedSourceUnit,
};
#[cfg(feature = "magic-gateway")]
pub use magic_market_core::{
    Adjustment, AssetClass, Bar, BarInterval, CoreError, CorporateActionCategory,
    CorporateActionStatus, CorporateActionTerms, DataBatch, DragonTigerSide, EvidenceTimestamp,
    Exchange, FinancialLine, FinancialStatement, FiniteNumber, FlowInterval, FxPair,
    GlobalIndexCode, InstrumentId, IsoDate, LimitPoolEntry, LimitPoolKind, MarketRankingKind,
    MarketRankingUnit, MarketStatistics, Money, NonEmptyText, NorthboundChannel, PositiveU32,
    Price, Provenance, ProviderId, QualityReport, Quantity, Ratio, RatioUnit, SourceEvidence,
    StatementKind, UnverifiedSourceUnit,
};
#[cfg(feature = "magic-gateway")]
pub use magic_tdx_rs::protocol::types::SecurityBar;
#[cfg(not(feature = "magic-gateway"))]
pub use market::{
    FinancialLine, FinancialStatement, LimitPoolEntry, LimitPoolKind, MarketStatistics,
    StatementKind,
};
#[cfg(not(feature = "magic-gateway"))]
pub use provider_id::ProviderId;
#[cfg(not(feature = "magic-gateway"))]
pub use ranking::{DragonTigerSide, FxPair, GlobalIndexCode, MarketRankingKind, MarketRankingUnit};
#[cfg(not(feature = "magic-gateway"))]
pub use record::{DataBatch, FlowInterval, IsoDate, NorthboundChannel, Provenance, QualityReport};
#[cfg(not(feature = "magic-gateway"))]
pub use tdx::SecurityBar;
#[cfg(not(feature = "magic-gateway"))]
pub use value::{FiniteNumber, Money, PositiveU32, Price, Quantity, Ratio, RatioUnit};

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
        assert_eq!(
            exchange_cases.len(),
            3,
            "Exchange 变体数必须与上游 75ee2a2 一致"
        );
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
        assert_eq!(
            asset_cases.len(),
            5,
            "AssetClass 变体数必须与上游 75ee2a2 一致"
        );
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

    /// Phase 3: unit-variant 枚举 Debug 名 + serde 字符串 (wire 契约) 与上游一致。
    #[test]
    fn enum_wire_names_match_upstream() {
        let limit_pool_kinds: &[(&str, LimitPoolKind)] = &[
            ("Upper", LimitPoolKind::Upper),
            ("Broken", LimitPoolKind::Broken),
            ("Lower", LimitPoolKind::Lower),
            ("PreviousUpper", LimitPoolKind::PreviousUpper),
        ];
        assert_eq!(
            limit_pool_kinds.len(),
            4,
            "LimitPoolKind 变体数 = 上游 75ee2a2"
        );
        for (name, kind) in limit_pool_kinds {
            assert_eq!(&format!("{kind:?}"), name);
            assert_eq!(
                serde_json::to_string(kind).unwrap(),
                serde_json::to_string(name).unwrap()
            );
        }

        let statement_kinds: &[(&str, StatementKind)] = &[
            ("Balance", StatementKind::Balance),
            ("Income", StatementKind::Income),
            ("CashFlow", StatementKind::CashFlow),
        ];
        assert_eq!(
            statement_kinds.len(),
            3,
            "StatementKind 变体数 = 上游 75ee2a2"
        );
        for (name, kind) in statement_kinds {
            assert_eq!(&format!("{kind:?}"), name);
            assert_eq!(
                serde_json::to_string(kind).unwrap(),
                serde_json::to_string(name).unwrap()
            );
        }

        let indices: &[(&str, GlobalIndexCode)] = &[
            ("DowJones", GlobalIndexCode::DowJones),
            ("NasdaqComposite", GlobalIndexCode::NasdaqComposite),
            ("Sp500", GlobalIndexCode::Sp500),
            ("Nikkei225", GlobalIndexCode::Nikkei225),
            ("HangSeng", GlobalIndexCode::HangSeng),
            ("Ftse100", GlobalIndexCode::Ftse100),
        ];
        assert_eq!(indices.len(), 6, "GlobalIndexCode 变体数 = 上游 75ee2a2");
        for (name, code) in indices {
            assert_eq!(&format!("{code:?}"), name);
            assert_eq!(
                serde_json::to_string(code).unwrap(),
                serde_json::to_string(name).unwrap()
            );
        }

        let pairs: &[(&str, FxPair)] = &[
            ("UsdCny", FxPair::UsdCny),
            ("EurUsd", FxPair::EurUsd),
            ("UsdJpy", FxPair::UsdJpy),
            ("GbpUsd", FxPair::GbpUsd),
            ("AudUsd", FxPair::AudUsd),
            ("UsdChf", FxPair::UsdChf),
            ("UsdCad", FxPair::UsdCad),
            ("NzdUsd", FxPair::NzdUsd),
        ];
        assert_eq!(pairs.len(), 8, "FxPair 变体数 = 上游 75ee2a2");
        for (name, pair) in pairs {
            assert_eq!(&format!("{pair:?}"), name);
            assert_eq!(
                serde_json::to_string(pair).unwrap(),
                serde_json::to_string(name).unwrap()
            );
        }
    }

    /// Phase 3: Custom(NonEmptyText) 变体的 wire 表示 (newtype 变体) 与上游一致。
    #[test]
    fn custom_variant_wire_representation_matches_upstream() {
        let kind = MarketRankingKind::Custom(NonEmptyText::new("region_heat").unwrap());
        // NonEmptyText 是 derive Debug 的 tuple struct → Debug 名含类型前缀
        assert_eq!(
            format!("{kind:?}"),
            r#"Custom(NonEmptyText("region_heat"))"#
        );
        assert_eq!(
            serde_json::to_string(&kind).unwrap(),
            r#"{"Custom":"region_heat"}"#
        );
        let parsed: MarketRankingKind =
            serde_json::from_str(r#"{"Custom":"region_heat"}"#).unwrap();
        assert_eq!(parsed, kind);

        let unit = MarketRankingUnit::Custom(NonEmptyText::new("score_100").unwrap());
        assert_eq!(format!("{unit:?}"), r#"Custom(NonEmptyText("score_100"))"#);
        assert_eq!(
            serde_json::to_string(&unit).unwrap(),
            r#"{"Custom":"score_100"}"#
        );
        let parsed: MarketRankingUnit = serde_json::from_str(r#"{"Custom":"score_100"}"#).unwrap();
        assert_eq!(parsed, unit);

        // 内部校验由 NonEmptyText 承担 (evidence.rs 已测), Custom 变体直接构造
        assert_eq!(
            serde_json::to_string(&MarketRankingKind::Custom(NonEmptyText::new("x").unwrap()))
                .unwrap(),
            r#"{"Custom":"x"}"#
        );
    }

    fn test_evidence() -> SourceEvidence {
        SourceEvidence::new(ProviderId::Eastmoney, "2099-01-02T10:00:01+08:00", "phase3").unwrap()
    }

    fn test_instrument() -> InstrumentId {
        InstrumentId::new(Exchange::Shanghai, "600396", AssetClass::Equity).unwrap()
    }

    /// Phase 3: LimitPoolEntry serde round-trip + 字段保真 (review.rs:535
    /// `DataBatch<LimitPoolEntry>` 反序列化依赖)。
    #[test]
    fn limit_pool_entry_json_roundtrip() {
        let entry = LimitPoolEntry {
            kind: LimitPoolKind::Upper,
            instrument: test_instrument(),
            trading_date: IsoDate::new("2099-01-02").unwrap(),
            price: Price::new(11.0).unwrap(),
            change: Ratio::decimal(0.1).unwrap(),
            volume: Some(Quantity::new(1_000_000.0).unwrap()),
            turnover: None,
            sealed_amount: Some(Money::new(123_456_789.0).unwrap()),
            first_seal_at: Some(NonEmptyText::new("09:30:00").unwrap()),
            last_seal_at: None,
            break_count: Some(2),
            streak: Some(PositiveU32::new(3).unwrap()),
            industry: Some(NonEmptyText::new("半导体").unwrap()),
            board_name: None,
            seal_state: Some(NonEmptyText::new("封板").unwrap()),
            reseal_count: None,
            reason: None,
            evidence: test_evidence(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains(r#""kind":"Upper""#));
        assert!(json.contains(r#""trading_date":"2099-01-02""#));
        assert!(json.contains(r#""first_seal_at":"09:30:00""#));
        assert!(json.contains(r#""industry":"半导体""#));
        assert!(json.contains(r#""provider":"Eastmoney""#));
        assert!(json.contains(r#""batch_id":"phase3""#));
        let parsed: LimitPoolEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(parsed.kind, LimitPoolKind::Upper);
        assert_eq!(parsed.streak.unwrap().get(), 3);
        assert_eq!(parsed.change.get(), 0.1);
    }

    /// Phase 3: FinancialStatement serde round-trip (company_financials.rs
    /// 消费其 lines/instrument/kind)。
    #[test]
    fn financial_statement_json_roundtrip() {
        let statement = FinancialStatement {
            instrument: test_instrument(),
            kind: StatementKind::Income,
            report_period: IsoDate::new("2026-03-31").unwrap(),
            announced_on: Some(IsoDate::new("2026-04-28").unwrap()),
            currency: Some(NonEmptyText::new("CNY").unwrap()),
            lines: vec![FinancialLine {
                key: NonEmptyText::new("revenue").unwrap(),
                source_label: NonEmptyText::new("营业收入").unwrap(),
                value: Some(FiniteNumber::new(1_234_567_890.0).unwrap()),
                unit: Some(NonEmptyText::new("元").unwrap()),
            }],
            evidence: test_evidence(),
        };
        let json = serde_json::to_string(&statement).unwrap();
        assert!(json.contains(r#""kind":"Income""#));
        assert!(json.contains(r#""report_period":"2026-03-31""#));
        assert!(json.contains(r#""key":"revenue""#));
        let parsed: FinancialStatement = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, statement);
        assert_eq!(parsed.lines[0].value.unwrap().get(), 1_234_567_890.0);
    }

    /// Phase 3: MarketStatistics 私有字段 + new 校验 + accessors + round-trip
    /// (push_templates.rs:3686 `MarketStatistics::new` 测试依赖)。
    #[test]
    fn market_statistics_validation_and_roundtrip() {
        let stats = MarketStatistics::new(
            test_instrument(),
            Some(Ratio::decimal(0.05).unwrap()),
            Some(FiniteNumber::new(15.0).unwrap()),
            None,
            Some(FiniteNumber::new(2.0).unwrap()),
            Some(Money::new(1e11).unwrap()),
            Some(Money::new(8e10).unwrap()),
            Some(Price::new(13.0).unwrap()),
            None,
            Some(FiniteNumber::new(2.5).unwrap()),
            test_evidence(),
        )
        .unwrap();
        assert_eq!(stats.turnover_rate().unwrap().get(), 0.05);
        assert_eq!(stats.total_market_cap().unwrap().get(), 1e11);
        assert_eq!(stats.instrument().code(), "600396");
        assert_eq!(stats.volume_ratio().unwrap().get(), 2.5);
        assert_eq!(stats.evidence().provider(), ProviderId::Eastmoney);

        // 校验: 市值不可为负 (ensure_nonnegative 错误字符串逐字一致)
        let error = MarketStatistics::new(
            test_instrument(),
            None,
            None,
            None,
            None,
            Some(Money::new(-1.0).unwrap()),
            None,
            None,
            None,
            None,
            test_evidence(),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("must be non-negative"),
            "{error}"
        );

        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains(r#""turnover_rate":{"value":0.05,"unit":"Decimal"}"#));
        let parsed: MarketStatistics = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, stats);
    }

    /// Phase 3: SecurityBar 字段名 (上游仅有 Serialize, wire 字段名=字段名)。
    #[test]
    fn security_bar_json_field_names_match_upstream() {
        let bar = SecurityBar {
            open: 10.0,
            close: 10.5,
            high: 10.6,
            low: 9.9,
            vol: 100_000.0,
            amount: 1_050_000.0,
            year: 2026,
            month: 8,
            day: 16,
            hour: 14,
            minute: 30,
            datetime: "2026-08-16 14:30:00".to_owned(),
        };
        let json = serde_json::to_string(&bar).unwrap();
        assert_eq!(
            json,
            r#"{"open":10.0,"close":10.5,"high":10.6,"low":9.9,"vol":100000.0,"amount":1050000.0,"year":2026,"month":8,"day":16,"hour":14,"minute":30,"datetime":"2026-08-16 14:30:00"}"#
        );
    }

    /// Phase 3: bars/lifecycle/signals 补充枚举 wire 名 (convert.rs 依赖 serde 表示)。
    #[test]
    fn bridge_enum_wire_names_match_upstream() {
        let bar_intervals: &[(&str, BarInterval)] = &[
            ("Minute1", BarInterval::Minute1),
            ("Minute5", BarInterval::Minute5),
            ("Minute15", BarInterval::Minute15),
            ("Minute30", BarInterval::Minute30),
            ("Hour1", BarInterval::Hour1),
            ("Day", BarInterval::Day),
            ("Week", BarInterval::Week),
            ("Month", BarInterval::Month),
            ("Year", BarInterval::Year),
        ];
        assert_eq!(bar_intervals.len(), 9, "BarInterval 变体数 = 上游 75ee2a2");
        for (name, v) in bar_intervals {
            assert_eq!(&format!("{v:?}"), name);
            assert_eq!(
                serde_json::to_string(v).unwrap(),
                serde_json::to_string(name).unwrap()
            );
        }

        let adjustments: &[(&str, Adjustment)] = &[
            ("Unadjusted", Adjustment::Unadjusted),
            ("Forward", Adjustment::Forward),
            ("Backward", Adjustment::Backward),
        ];
        assert_eq!(adjustments.len(), 3, "Adjustment 变体数 = 上游 75ee2a2");
        for (name, v) in adjustments {
            assert_eq!(&format!("{v:?}"), name);
            assert_eq!(
                serde_json::to_string(v).unwrap(),
                serde_json::to_string(name).unwrap()
            );
        }

        let sides: &[(&str, DragonTigerSide)] = &[
            ("Buy", DragonTigerSide::Buy),
            ("Sell", DragonTigerSide::Sell),
        ];
        assert_eq!(sides.len(), 2, "DragonTigerSide 变体数 = 上游 75ee2a2");
        for (name, v) in sides {
            assert_eq!(&format!("{v:?}"), name);
            assert_eq!(
                serde_json::to_string(v).unwrap(),
                serde_json::to_string(name).unwrap()
            );
        }

        let categories: &[(&str, CorporateActionCategory)] = &[
            ("Distribution", CorporateActionCategory::Distribution),
            (
                "BonusRightsListing",
                CorporateActionCategory::BonusRightsListing,
            ),
            (
                "NonTradableShareListing",
                CorporateActionCategory::NonTradableShareListing,
            ),
            (
                "UnknownCapitalChange",
                CorporateActionCategory::UnknownCapitalChange,
            ),
            ("CapitalChange", CorporateActionCategory::CapitalChange),
            (
                "AdditionalIssuance",
                CorporateActionCategory::AdditionalIssuance,
            ),
            ("ShareRepurchase", CorporateActionCategory::ShareRepurchase),
            (
                "AdditionalIssuanceListing",
                CorporateActionCategory::AdditionalIssuanceListing,
            ),
            (
                "TransferredAllotmentListing",
                CorporateActionCategory::TransferredAllotmentListing,
            ),
            (
                "ConvertibleBondListing",
                CorporateActionCategory::ConvertibleBondListing,
            ),
            (
                "CapitalRescaling",
                CorporateActionCategory::CapitalRescaling,
            ),
            (
                "NonTradableReverseSplit",
                CorporateActionCategory::NonTradableReverseSplit,
            ),
            (
                "SubscriptionWarrantGrant",
                CorporateActionCategory::SubscriptionWarrantGrant,
            ),
            ("PutWarrantGrant", CorporateActionCategory::PutWarrantGrant),
        ];
        assert_eq!(
            categories.len(),
            14,
            "CorporateActionCategory 变体数 = 上游 75ee2a2"
        );
        for (name, v) in categories {
            assert_eq!(&format!("{v:?}"), name);
            assert_eq!(
                serde_json::to_string(v).unwrap(),
                serde_json::to_string(name).unwrap()
            );
        }

        let statuses: &[(&str, CorporateActionStatus)] = &[
            ("Implemented", CorporateActionStatus::Implemented),
            ("Proposed", CorporateActionStatus::Proposed),
            ("Cancelled", CorporateActionStatus::Cancelled),
            ("Unknown", CorporateActionStatus::Unknown),
        ];
        assert_eq!(
            statuses.len(),
            4,
            "CorporateActionStatus 变体数 = 上游 75ee2a2"
        );
        for (name, v) in statuses {
            assert_eq!(&format!("{v:?}"), name);
            assert_eq!(
                serde_json::to_string(v).unwrap(),
                serde_json::to_string(name).unwrap()
            );
        }
    }

    /// Phase 3: Bar serde round-trip + 校验语义 (convert.rs:1886
    /// `DataBatch<Bar>` 反序列化依赖)。
    #[test]
    fn bar_json_roundtrip_and_validation() {
        let bar = Bar::new(
            test_instrument(),
            BarInterval::Minute15,
            "2099-01-02 09:45:00",
            "2099-01-02 10:00:00",
            Price::new(10.0).unwrap(),
            Price::new(10.8).unwrap(),
            Price::new(9.9).unwrap(),
            Price::new(10.5).unwrap(),
            Quantity::new(1_000_000.0).unwrap(),
            Some(Money::new(10_500_000.0).unwrap()),
            Adjustment::Forward,
            ProviderId::Tdx,
            "phase3-batch",
        )
        .unwrap();
        let json = serde_json::to_string(&bar).unwrap();
        assert!(json.contains(r#""interval":"Minute15""#));
        assert!(json.contains(r#""adjustment":"Forward""#));
        assert!(json.contains(r#""provider":"Tdx""#));
        assert!(json.contains(r#""bar_start":"2099-01-02 09:45:00""#));
        let parsed: Bar = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, bar);
        assert_eq!(parsed.interval(), BarInterval::Minute15);
        assert_eq!(parsed.close().get(), 10.5);
        assert_eq!(parsed.batch_id(), "phase3-batch");

        // 校验: 时间格式与 interval 匹配 + OHLC 一致性 (错误字符串逐字)
        let bad_time = Bar::new(
            test_instrument(),
            BarInterval::Minute15,
            "2099-01-02",
            "2099-01-02 10:00:00",
            Price::new(10.0).unwrap(),
            Price::new(10.8).unwrap(),
            Price::new(9.9).unwrap(),
            Price::new(10.5).unwrap(),
            Quantity::new(1.0).unwrap(),
            None,
            Adjustment::Unadjusted,
            ProviderId::Tdx,
            "b",
        );
        assert_eq!(
            bad_time.unwrap_err().to_string(),
            "invalid request: invalid bar time range"
        );

        let bad_ohlc = Bar::new(
            test_instrument(),
            BarInterval::Day,
            "2099-01-02",
            "2099-01-03",
            Price::new(10.0).unwrap(),
            Price::new(9.5).unwrap(),
            Price::new(9.9).unwrap(),
            Price::new(10.5).unwrap(),
            Quantity::new(1.0).unwrap(),
            None,
            Adjustment::Unadjusted,
            ProviderId::Tdx,
            "b",
        );
        assert_eq!(
            bad_ohlc.unwrap_err().to_string(),
            "invalid request: inconsistent OHLC range"
        );
    }

    /// Phase 3: CorporateActionTerms 反序列化 (convert.rs from_value 路径)。
    #[test]
    fn corporate_action_terms_json_roundtrip() {
        let terms = CorporateActionTerms::Distribution {
            cash_per_share: Some(FiniteNumber::new(0.5).unwrap()),
            bonus_per_share: None,
            rights_per_share: None,
            rights_price: None,
        };
        let json = serde_json::to_string(&terms).unwrap();
        assert_eq!(
            json,
            r#"{"Distribution":{"cash_per_share":0.5,"bonus_per_share":null,"rights_per_share":null,"rights_price":null}}"#
        );
        let parsed: CorporateActionTerms = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, terms);

        let native = CorporateActionTerms::ProviderNativeRatio {
            category: CorporateActionCategory::CapitalRescaling,
            source_ratio: FiniteNumber::new(3.0).unwrap(),
            source_ratio_unit: UnverifiedSourceUnit::ProviderNative,
        };
        let parsed: CorporateActionTerms =
            serde_json::from_str(&serde_json::to_string(&native).unwrap()).unwrap();
        assert_eq!(parsed, native);

        // 校验: 类别不匹配时反序列化必须拒绝 (上游错误字符串逐字)
        let invalid = serde_json::json!({
            "ProviderNativeRatio": {
                "category": "CapitalChange",
                "source_ratio": 3.0,
                "source_ratio_unit": "ProviderNative",
            }
        });
        let err = serde_json::from_value::<CorporateActionTerms>(invalid).unwrap_err();
        assert!(
            err.to_string()
                .contains("category does not use provider-native ratio terms"),
            "got: {err}"
        );
    }
}
