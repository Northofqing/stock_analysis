//! op 67 `QualifiedTradingFacts` 的 **provider-observed** 解析层 (2026-10-17).
//!
//! ## 为什么单独一套类型，不复用 `qualified_trading_facts.rs`
//!
//! 现有 `qualified_trading_facts` 模块是**权威 (Authority) 语义**：
//! `AuthorityLifecycle` 要求 `delisted_on` + `covered_through` 证明，
//! `QualifiedPriceBand` 要求 micro-CNY + tick grid + version，
//! `AuthoritySuspensionCoverage` 要求停牌覆盖窗口。
//!
//! 而对端 op 67 交付的是**观察值 (provider-sourced observed)**：
//! - `delisted` **永不产出**（无退市来源）
//! - `limit_up/limit_down` 是**裸 f64**（不是 micros、不是 tick grid）
//! - **没有 suspension**（显式请求 → typed `Unsupported`）
//!
//! 把观察值塞进权威模型 = 伪造权威证明。因此本模块独立定义类型，并用
//! `quality` 字段把"这不是权威"固化进类型，禁止下游误用。
//!
//! ## 依据
//! - `2026-10-17-qualified-trading-facts-mac-side-checklist.md` §1.1 / §4
//! - `2026-10-17-p01-provider-sourced-trading-facts-spec.md`
//! - 对端确认的封闭枚举：`quality` 恒 `provider_observed`；
//!   `source` ∈ {`tdx-security-metadata`, `tencent-market-statistics`}；
//!   `price_band_source` 同上或 null；`listing_state` ∈ {listed, not_yet_listed, unknown}
//!
//! ## 尚未覆盖
//! wire 层（`Operation` 枚举 / `build_external_query_request` 白名单 / proto）需要
//! 新 client bundle 才能接——当前 bundle 是 66 op，没有这个 RPC。本模块只做
//! **JSON 记录 → 强类型** 的一层，可用 SYNTHETIC 帧单测，不依赖部署。

use serde::Deserialize;
use serde_json::Value;
use std::fmt;

/// 事实质量。op 67 目前只交付 `ProviderObserved`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingFactQuality {
    ProviderObserved,
}

impl TradingFactQuality {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "provider_observed" => Some(Self::ProviderObserved),
            _ => None,
        }
    }
}

/// 事实来源（封闭枚举）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingFactSource {
    /// TDX 证券元数据（提供 `listed_on`）
    TdxSecurityMetadata,
    /// 腾讯 MarketStatistics（提供涨跌停价）
    TencentMarketStatistics,
}

impl TradingFactSource {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "tdx-security-metadata" => Some(Self::TdxSecurityMetadata),
            "tencent-market-statistics" => Some(Self::TencentMarketStatistics),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TdxSecurityMetadata => "tdx-security-metadata",
            Self::TencentMarketStatistics => "tencent-market-statistics",
        }
    }
}

/// 上市状态。**`Delisted` 不在此枚举内**——对端永不产出该值，
/// 出现即视为合同违约（见错误 `DelistedNotSupported`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingState {
    Listed,
    NotYetListed,
    Unknown,
}

impl ListingState {
    fn parse(raw: &str) -> Result<Self, ProviderObservedFactError> {
        match raw {
            "listed" => Ok(Self::Listed),
            "not_yet_listed" => Ok(Self::NotYetListed),
            "unknown" => Ok(Self::Unknown),
            "delisted" => Err(ProviderObservedFactError::DelistedNotSupported),
            other => Err(ProviderObservedFactError::InvalidEnum {
                field: "listing_state",
                value: other.to_owned(),
            }),
        }
    }

    /// `listed` **不构成"在交易"证明**（对端清单 §1.1-3）。
    /// 下游若要判"能否交易"，必须另取权威停牌来源；本模块不提供该结论。
    pub fn implies_tradable(self) -> bool {
        false
    }
}

/// 来源证据（与既有 `SourceEvidence` 同形，但本模块自持，便于单测构造）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactEvidence {
    pub provider: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
}

/// 一条 provider-observed 事实（一个 instrument × 一个 date）。
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderObservedTradingFact {
    pub exchange: String,
    pub code: String,
    pub date: String,
    pub quality: TradingFactQuality,
    pub source: TradingFactSource,
    pub observed_at: String,
    pub source_at: Option<String>,
    pub listed_on: Option<String>,
    pub listing_state: Option<ListingState>,
    /// 裸价格（元/股），**不是** micro-CNY。
    pub limit_up: Option<f64>,
    pub limit_down: Option<f64>,
    pub price_band_source: Option<TradingFactSource>,
    pub evidence: Option<FactEvidence>,
    pub price_band_evidence: Option<FactEvidence>,
}

impl ProviderObservedTradingFact {
    /// 该日价格带是否可用。`false` 时 `limit_up/limit_down` 必为 `None`，
    /// 下游**不得**用"昨收 × 涨跌幅上限"推导（BR-205 同源要求）。
    pub fn price_band_available(&self) -> bool {
        self.price_band_source.is_some()
    }
}

/// 解析错误。按对端清单 §2.2 的三类语义分组，便于映射到 gRPC status。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderObservedFactError {
    /// 记录里出现 `delisted` —— 合同禁止（对端无退市来源）。
    DelistedNotSupported,
    /// 封闭枚举越界。
    InvalidEnum {
        field: &'static str,
        value: String,
    },
    /// 字段间不自洽（价格带三元组不成对、listing_state 与 listed_on 冲突等）。
    Inconsistent {
        field: &'static str,
        detail: String,
    },
    MissingField(&'static str),
    /// 数值非有限或非正。
    NonFinite(&'static str),
    /// 记录里出现未在合同中定义的字段。
    UnknownField(String),
}

impl fmt::Display for ProviderObservedFactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DelistedNotSupported => {
                write!(f, "listing_state=delisted 不在 op 67 合同内（无退市来源）")
            }
            Self::InvalidEnum { field, value } => {
                write!(f, "{field} 取值不在封闭枚举内: {value}")
            }
            Self::Inconsistent { field, detail } => write!(f, "{field} 不自洽: {detail}"),
            Self::MissingField(name) => write!(f, "缺少必填字段 {name}"),
            Self::NonFinite(name) => write!(f, "{name} 非有限正数"),
            Self::UnknownField(name) => write!(f, "未在合同中定义的字段 {name}"),
        }
    }
}

/// 合同允许的字段全集（用于拒绝拼写错误/未来字段被静默接受）。
const ALLOWED_FIELDS: &[&str] = &[
    "instrument",
    "date",
    "quality",
    "source",
    "observed_at",
    "source_at",
    "listed_on",
    "listing_state",
    "limit_up",
    "limit_down",
    "price_band_source",
    "evidence",
    "price_band_evidence",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireInstrument {
    exchange: String,
    code: String,
    asset_class: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEvidence {
    provider: String,
    #[serde(default)]
    source_at: Option<String>,
    observed_at: String,
    batch_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFact {
    instrument: WireInstrument,
    date: String,
    quality: String,
    source: String,
    observed_at: String,
    #[serde(default)]
    source_at: Option<String>,
    #[serde(default)]
    listed_on: Option<String>,
    #[serde(default)]
    listing_state: Option<String>,
    #[serde(default)]
    limit_up: Option<f64>,
    #[serde(default)]
    limit_down: Option<f64>,
    #[serde(default)]
    price_band_source: Option<String>,
    #[serde(default)]
    evidence: Option<WireEvidence>,
    #[serde(default)]
    price_band_evidence: Option<WireEvidence>,
}

fn to_fact_evidence(wire: WireEvidence) -> FactEvidence {
    FactEvidence {
        provider: wire.provider,
        source_at: wire.source_at,
        observed_at: wire.observed_at,
        batch_id: wire.batch_id,
    }
}

fn validate_price(
    value: Option<f64>,
    field: &'static str,
) -> Result<(), ProviderObservedFactError> {
    match value {
        Some(v) if !v.is_finite() || v <= 0.0 => Err(ProviderObservedFactError::NonFinite(field)),
        _ => Ok(()),
    }
}

/// 解析一条记录。严格按对端清单 §1.1 / §4 的封闭枚举与一致性要求。
pub fn parse_fact(value: &Value) -> Result<ProviderObservedTradingFact, ProviderObservedFactError> {
    if let Some(object) = value.as_object() {
        for key in object.keys() {
            if !ALLOWED_FIELDS.contains(&key.as_str()) {
                return Err(ProviderObservedFactError::UnknownField(key.clone()));
            }
        }
    }

    let wire: WireFact = serde_json::from_value(value.clone()).map_err(|error| {
        ProviderObservedFactError::Inconsistent {
            field: "record",
            detail: format!("记录形状不符: {error}"),
        }
    })?;

    // instrument
    if wire.instrument.asset_class != "Equity" {
        return Err(ProviderObservedFactError::InvalidEnum {
            field: "asset_class",
            value: wire.instrument.asset_class,
        });
    }
    if wire.instrument.code.len() != 6 || !wire.instrument.code.chars().all(|c| c.is_ascii_digit())
    {
        return Err(ProviderObservedFactError::Inconsistent {
            field: "instrument.code",
            detail: "必须是 6 位数字".to_owned(),
        });
    }

    // 日期形状（不在这里做交易日校验；日历约束由服务端保证）
    if chrono::NaiveDate::parse_from_str(&wire.date, "%Y-%m-%d").is_err() {
        return Err(ProviderObservedFactError::Inconsistent {
            field: "date",
            detail: format!("非法日期 {}", wire.date),
        });
    }

    let quality = TradingFactQuality::parse(&wire.quality).ok_or_else(|| {
        ProviderObservedFactError::InvalidEnum {
            field: "quality",
            value: wire.quality.clone(),
        }
    })?;
    let source = TradingFactSource::parse(&wire.source).ok_or_else(|| {
        ProviderObservedFactError::InvalidEnum {
            field: "source",
            value: wire.source.clone(),
        }
    })?;

    let listing_state = match wire.listing_state.as_deref() {
        Some(raw) => Some(ListingState::parse(raw)?),
        None => None,
    };

    // listing_state 与 listed_on 的一致性
    match (listing_state, wire.listed_on.as_deref()) {
        (Some(ListingState::Listed | ListingState::NotYetListed), None) => {
            return Err(ProviderObservedFactError::Inconsistent {
                field: "listed_on",
                detail: "listing_state 为 listed/not_yet_listed 时必须给出 listed_on".to_owned(),
            });
        }
        (_, Some(listed_on))
            if chrono::NaiveDate::parse_from_str(listed_on, "%Y-%m-%d").is_err() =>
        {
            return Err(ProviderObservedFactError::Inconsistent {
                field: "listed_on",
                detail: format!("非法日期 {listed_on}"),
            });
        }
        _ => {}
    }

    validate_price(wire.limit_up, "limit_up")?;
    validate_price(wire.limit_down, "limit_down")?;

    let price_band_source = match wire.price_band_source.as_deref() {
        Some(raw) => Some(TradingFactSource::parse(raw).ok_or_else(|| {
            ProviderObservedFactError::InvalidEnum {
                field: "price_band_source",
                value: raw.to_owned(),
            }
        })?),
        None => None,
    };

    // 价格带三元组必须成对出现：limit_up/limit_down <-> price_band_source <-> price_band_evidence
    let has_limits = wire.limit_up.is_some() || wire.limit_down.is_some();
    if has_limits {
        if wire.limit_up.is_none() || wire.limit_down.is_none() {
            return Err(ProviderObservedFactError::Inconsistent {
                field: "limit_up/limit_down",
                detail: "必须成对出现".to_owned(),
            });
        }
        let up = wire.limit_up.unwrap_or_default();
        let down = wire.limit_down.unwrap_or_default();
        if down >= up {
            return Err(ProviderObservedFactError::Inconsistent {
                field: "limit_up/limit_down",
                detail: format!("跌停价 {down} 必须低于涨停价 {up}"),
            });
        }
        if price_band_source.is_none() {
            return Err(ProviderObservedFactError::Inconsistent {
                field: "price_band_source",
                detail: "有涨跌停价时必须给出 price_band_source".to_owned(),
            });
        }
        if wire.price_band_evidence.is_none() {
            return Err(ProviderObservedFactError::Inconsistent {
                field: "price_band_evidence",
                detail: "有涨跌停价时必须给出 price_band_evidence".to_owned(),
            });
        }
    } else {
        // 无价格带时，source 必须为 null（避免"声明有来源但无值"）
        if price_band_source.is_some() {
            return Err(ProviderObservedFactError::Inconsistent {
                field: "price_band_source",
                detail: "无涨跌停价时不得声明 price_band_source".to_owned(),
            });
        }
        if wire.price_band_evidence.is_some() {
            return Err(ProviderObservedFactError::Inconsistent {
                field: "price_band_evidence",
                detail: "无涨跌停价时不得给出 price_band_evidence".to_owned(),
            });
        }
    }

    Ok(ProviderObservedTradingFact {
        exchange: wire.instrument.exchange,
        code: wire.instrument.code,
        date: wire.date,
        quality,
        source,
        observed_at: wire.observed_at,
        source_at: wire.source_at,
        listed_on: wire.listed_on,
        listing_state,
        limit_up: wire.limit_up,
        limit_down: wire.limit_down,
        price_band_source,
        evidence: wire.evidence.map(to_fact_evidence),
        price_band_evidence: wire.price_band_evidence.map(to_fact_evidence),
    })
}

/// 请求侧错误分类（对端清单 §1.1-2 / §2.2）。
///
/// 三类语义不同，消费侧应分别记录，不要靠字符串匹配。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedFactsRequestError {
    /// 显式请求 `suspension` → gRPC `UNIMPLEMENTED`，
    /// `reason_code=capability_unadmitted`，**不要重试**。
    SuspensionUnsupported,
    /// 越界 / 日历外日期 / 未知字段 → `INVALID_ARGUMENT`。
    InvalidArgument(String),
    /// 批次缺标的 / 缺源时间 → `FAILED_PRECONDITION`（可重试）。
    FailedPrecondition(String),
}

impl QualifiedFactsRequestError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::SuspensionUnsupported => "capability_unadmitted",
            Self::InvalidArgument(_) => "invalid_argument",
            Self::FailedPrecondition(_) => "failed_precondition",
        }
    }

    /// 是否值得重试。`suspension` 是设计性不支持，永不重试。
    pub fn retryable(&self) -> bool {
        matches!(self, Self::FailedPrecondition(_))
    }
}

/// 请求侧校验（对端清单 §1.2 的边界）。
pub fn validate_request_shape(
    instruments: usize,
    dates: usize,
    include: &[String],
) -> Result<(), QualifiedFactsRequestError> {
    if !(1..=50).contains(&instruments) {
        return Err(QualifiedFactsRequestError::InvalidArgument(format!(
            "instruments 必须在 1..=50，实际 {instruments}"
        )));
    }
    if !(1..=250).contains(&dates) {
        return Err(QualifiedFactsRequestError::InvalidArgument(format!(
            "dates 必须在 1..=250，实际 {dates}"
        )));
    }
    if instruments.saturating_mul(dates) > 500 {
        return Err(QualifiedFactsRequestError::InvalidArgument(format!(
            "instruments × dates 必须 ≤ 500，实际 {} × {} = {}",
            instruments,
            dates,
            instruments * dates
        )));
    }
    for item in include {
        match item.as_str() {
            "lifecycle" | "price_band" => {}
            "suspension" => return Err(QualifiedFactsRequestError::SuspensionUnsupported),
            other => {
                return Err(QualifiedFactsRequestError::InvalidArgument(format!(
                    "include 不支持的子集: {other}"
                )))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 对端清单 §4 记录 1：非快照日，价格带为 null。
    fn synthetic_non_snapshot() -> Value {
        json!({
            "instrument": {"exchange": "Shanghai", "code": "600000", "asset_class": "Equity"},
            "date": "2026-10-15",
            "quality": "provider_observed",
            "source": "tdx-security-metadata",
            "observed_at": "2026-10-17T10:21:25+08:00",
            "source_at": null,
            "listed_on": "1999-11-10",
            "listing_state": "listed",
            "limit_up": null,
            "limit_down": null,
            "price_band_source": null,
            "evidence": {
                "provider": "Tdx",
                "source_at": null,
                "observed_at": "1792203685",
                "batch_id": "tdx:1792203685:security-metadata"
            },
            "price_band_evidence": null
        })
    }

    /// 对端清单 §4 记录 2：快照日，价格带来自腾讯 MarketStatistics。
    fn synthetic_snapshot() -> Value {
        json!({
            "instrument": {"exchange": "Shanghai", "code": "600000", "asset_class": "Equity"},
            "date": "2026-10-16",
            "quality": "provider_observed",
            "source": "tdx-security-metadata",
            "observed_at": "2026-10-17T10:21:25+08:00",
            "source_at": null,
            "listed_on": "1999-11-10",
            "listing_state": "listed",
            "limit_up": 10.55,
            "limit_down": 8.63,
            "price_band_source": "tencent-market-statistics",
            "evidence": {
                "provider": "Tdx",
                "source_at": null,
                "observed_at": "1792203685",
                "batch_id": "tdx:1792203685:security-metadata"
            },
            "price_band_evidence": {
                "provider": "Tencent",
                "source_at": "2026-10-16T16:14:47+08:00",
                "observed_at": "1792203713.191533300",
                "batch_id": "tencent-web:1792203713.191533300:market-statistics"
            }
        })
    }

    #[test]
    fn parses_synthetic_non_snapshot_record_without_price_band() {
        let fact = parse_fact(&synthetic_non_snapshot()).expect("记录 1 应可解析");
        assert_eq!(fact.code, "600000");
        assert_eq!(fact.listing_state, Some(ListingState::Listed));
        assert_eq!(fact.listed_on.as_deref(), Some("1999-11-10"));
        assert!(!fact.price_band_available(), "非快照日不应有价格带");
        assert_eq!(fact.limit_up, None);
        assert_eq!(fact.limit_down, None);
        assert!(
            !ListingState::Listed.implies_tradable(),
            "listed 不等于可交易"
        );
    }

    #[test]
    fn parses_synthetic_snapshot_record_with_price_band() {
        let fact = parse_fact(&synthetic_snapshot()).expect("记录 2 应可解析");
        assert!(fact.price_band_available());
        assert_eq!(fact.limit_up, Some(10.55));
        assert_eq!(fact.limit_down, Some(8.63));
        assert_eq!(
            fact.price_band_source,
            Some(TradingFactSource::TencentMarketStatistics)
        );
        let band_evidence = fact.price_band_evidence.expect("应有价格带证据");
        assert_eq!(band_evidence.provider, "Tencent");
        assert_eq!(
            band_evidence.batch_id,
            "tencent-web:1792203713.191533300:market-statistics"
        );
    }

    #[test]
    fn rejects_delisted_state_even_though_contract_has_no_source_for_it() {
        let mut value = synthetic_non_snapshot();
        value["listing_state"] = json!("delisted");
        assert_eq!(
            parse_fact(&value),
            Err(ProviderObservedFactError::DelistedNotSupported)
        );
    }

    #[test]
    fn rejects_unknown_quality_and_source() {
        let mut authoritative = synthetic_non_snapshot();
        authoritative["quality"] = json!("authoritative");
        assert!(matches!(
            parse_fact(&authoritative),
            Err(ProviderObservedFactError::InvalidEnum {
                field: "quality",
                ..
            })
        ));

        let mut wrong_band_source = synthetic_snapshot();
        wrong_band_source["price_band_source"] = json!("tencent-quote-batch");
        assert!(matches!(
            parse_fact(&wrong_band_source),
            Err(ProviderObservedFactError::InvalidEnum {
                field: "price_band_source",
                ..
            })
        ));
    }

    #[test]
    fn rejects_inconsistent_price_band_triplet() {
        // 有涨停价但缺 price_band_source
        let mut missing_source = synthetic_snapshot();
        missing_source["price_band_source"] = json!(null);
        assert!(matches!(
            parse_fact(&missing_source),
            Err(ProviderObservedFactError::Inconsistent {
                field: "price_band_source",
                ..
            })
        ));

        // 只有涨停没有跌停
        let mut missing_lower = synthetic_snapshot();
        missing_lower["limit_down"] = json!(null);
        assert!(matches!(
            parse_fact(&missing_lower),
            Err(ProviderObservedFactError::Inconsistent {
                field: "limit_up/limit_down",
                ..
            })
        ));

        // 跌停高于涨停
        let mut reversed = synthetic_snapshot();
        reversed["limit_up"] = json!(8.0);
        reversed["limit_down"] = json!(9.0);
        assert!(matches!(
            parse_fact(&reversed),
            Err(ProviderObservedFactError::Inconsistent {
                field: "limit_up/limit_down",
                ..
            })
        ));

        // 无价格带却声明来源
        let mut stray_source = synthetic_non_snapshot();
        stray_source["price_band_source"] = json!("tencent-market-statistics");
        assert!(matches!(
            parse_fact(&stray_source),
            Err(ProviderObservedFactError::Inconsistent {
                field: "price_band_source",
                ..
            })
        ));
    }

    #[test]
    fn accepts_price_band_only_record_with_null_lifecycle() {
        // 对端清单 §4 末尾: include=["price_band"] 时 listed_on/listing_state/evidence 不产出
        let value = json!({
            "instrument": {"exchange": "Shanghai", "code": "600000", "asset_class": "Equity"},
            "date": "2026-10-16",
            "quality": "provider_observed",
            "source": "tencent-market-statistics",
            "observed_at": "2026-10-17T10:21:25+08:00",
            "source_at": "2026-10-16T16:14:47+08:00",
            "listed_on": null,
            "listing_state": null,
            "limit_up": 10.55,
            "limit_down": 8.63,
            "price_band_source": "tencent-market-statistics",
            "evidence": null,
            "price_band_evidence": {
                "provider": "Tencent",
                "source_at": "2026-10-16T16:14:47+08:00",
                "observed_at": "1792203713.191533300",
                "batch_id": "tencent-web:1792203713.191533300:market-statistics"
            }
        });
        let fact = parse_fact(&value).expect("price_band-only 记录应可解析");
        assert_eq!(fact.listing_state, None);
        assert!(fact.price_band_available());
    }

    #[test]
    fn rejects_unknown_fields_to_avoid_silently_accepting_drift() {
        let mut value = synthetic_snapshot();
        value["suspended"] = json!(false);
        assert_eq!(
            parse_fact(&value),
            Err(ProviderObservedFactError::UnknownField(
                "suspended".to_owned()
            )),
            "suspension 不在本 op 合同内，出现即应拒绝"
        );
    }

    #[test]
    fn request_shape_validation_classifies_errors_by_semantics() {
        // suspension 是设计性不支持 → 不重试
        let err = validate_request_shape(1, 1, &["lifecycle".into(), "suspension".into()])
            .expect_err("suspension 应被拒");
        assert_eq!(err.reason_code(), "capability_unadmitted");
        assert!(!err.retryable());

        // 越界 → invalid_argument
        assert_eq!(
            validate_request_shape(0, 1, &[]).unwrap_err().reason_code(),
            "invalid_argument"
        );
        // 行数超限 → invalid_argument
        assert_eq!(
            validate_request_shape(50, 250, &[])
                .unwrap_err()
                .reason_code(),
            "invalid_argument"
        );
        // 正常组合
        assert!(validate_request_shape(2, 3, &["lifecycle".into(), "price_band".into()]).is_ok());
    }
}
