//! 请求 params 契约 (P4 M1): 请求方向 payload.data = params JSON 对象。
//! `{}` = 全默认 = 与今天 (library 模式) 行为完全一致; 显式字段才改变行为。
//! 响应方向 = records 数组 (既有合同, 不变)。
//!
//! 默认值表 (op → 字段 → 默认) 与 monitor 生产调用惯例对齐 (P4 探索验证):
//! | op | 字段 | 默认 |
//! |----|------|------|
//! | RealtimeQuotes/HistoricalBars/... 全部逐代码 op | codes | STOCK_LIST env (watchlist) |
//! | HistoricalBars | days | 120 (delegate fetch_historical_bars 现用值) |
//! | TechnicalBars | count | 48 (15min K 线, 3 个交易日) |
//! | IndexQuotes | codes | 6 大指数 (MAIN_INDICES, 来源 market_analyzer/mod.rs:45 私有常量) |
//! | InstrumentNews | from_days | 30 (monitor post_close_news_review 现用值) |
//! | FundFlowSeries | interval/limit | "day1" / 20 |
//! | FinancialStatements | kind | 必填 ("balance"/"income"/"cash_flow") |
//! | CorporateActions | code/window_start/window_end | 必填 |
//! | SemanticSearch | query | 必填 |
//! | UpperLimitPoolReview/ProviderTopNRankings | date | 今天 |
//! | T0Evidence | codes/observed_at | watchlist / Utc::now() |
//! | OutcomeDailyBars | - | M1 不直连 (claim 台账留客户端, M3 transport seam) |
//!
//! 约定: 任何解析失败 → ParamsError::InvalidArgument(含字段名), 服务端映射
//! Status::invalid_argument。缺省规则全部"出声" (v15.x): 显式字段才改变行为,
//! 服务端不发明请求方没有声明的默认 (默认值仅来自本表)。

use chrono::{Local, NaiveDate};
use serde_json::Value;

/// params 解析失败 (请求方参数错误, 服务端 400 语义)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamsError {
    /// 字段缺失但必填, 或格式非法 (含字段名与期望)。
    InvalidArgument(String),
}

impl std::fmt::Display for ParamsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamsError::InvalidArgument(msg) => write!(f, "params 无效: {msg}"),
        }
    }
}

/// 6 大指数默认 (IndexQuotes 缺省 codes)。
/// 来源: market_analyzer/mod.rs:45 MarketAnalyzer::MAIN_INDICES_LIST (私有常量,
/// delegate 无法引用 → 合同冻结在 params 默认值表)。
pub const MAIN_INDICES: [(&str, &str); 6] = [
    ("sh000001", "上证指数"),
    ("sz399001", "深证成指"),
    ("sz399006", "创业板指"),
    ("sh000688", "科创50"),
    ("sh000016", "上证50"),
    ("sh000300", "沪深300"),
];

/// 逐代码 op 的默认 codes: STOCK_LIST env (watchlist)。
/// 与 delegate.rs 现 watchlist_codes() 语义一致 (M1 起以本模块为唯一来源)。
pub fn watchlist_codes() -> Vec<String> {
    std::env::var("STOCK_LIST")
        .map(|s| {
            s.split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// params["codes"]: 字符串数组; 缺省 → watchlist_codes()。
pub fn resolve_codes(p: &Value) -> Result<Vec<String>, ParamsError> {
    let Some(value) = p.get("codes") else {
        return Ok(watchlist_codes());
    };
    let arr = value
        .as_array()
        .ok_or_else(|| ParamsError::InvalidArgument("codes 必须是字符串数组".into()))?;
    let mut codes = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item.as_str().ok_or_else(|| {
            ParamsError::InvalidArgument("codes 元素必须是字符串".into())
        })?;
        if !s.is_empty() {
            codes.push(s.to_string());
        }
    }
    Ok(codes)
}

/// 文档 §8 证券资料请求 (SecurityMetadata/SecurityProfiles 契约):
/// `{"instruments":[{"exchange":"Shanghai","code":"600396","asset_class":"Equity"}]}`
/// (grpc/grpc-external-api.md §8「已接入的证券资料请求」)。
/// 解析 → codes; 缺省 → watchlist_codes()。exchange 必须是文档枚举
/// (Shanghai/Shenzhen/Beijing), 未知值 fail-closed 拒绝 (不静默猜市场)。
pub fn resolve_instruments(p: &Value) -> Result<Vec<String>, ParamsError> {
    let Some(value) = p.get("instruments") else {
        return Ok(watchlist_codes());
    };
    let arr = value.as_array().ok_or_else(|| {
        ParamsError::InvalidArgument("instruments 必须是对象数组".into())
    })?;
    let mut codes = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item.as_object().ok_or_else(|| {
            ParamsError::InvalidArgument("instruments 元素必须是对象".into())
        })?;
        let exchange = obj
            .get("exchange")
            .and_then(Value::as_str)
            .ok_or_else(|| ParamsError::InvalidArgument("instrument.exchange 缺失".into()))?;
        if !matches!(exchange, "Shanghai" | "Shenzhen" | "Beijing") {
            return Err(ParamsError::InvalidArgument(format!(
                "instrument.exchange 未知: {exchange}"
            )));
        }
        let code = obj
            .get("code")
            .and_then(Value::as_str)
            .ok_or_else(|| ParamsError::InvalidArgument("instrument.code 缺失".into()))?;
        let asset_class = obj
            .get("asset_class")
            .and_then(Value::as_str)
            .ok_or_else(|| ParamsError::InvalidArgument("instrument.asset_class 缺失".into()))?;
        if asset_class != "Equity" {
            return Err(ParamsError::InvalidArgument(format!(
                "instrument.asset_class 未知: {asset_class}"
            )));
        }
        if !code.is_empty() {
            codes.push(code.to_string());
        }
    }
    Ok(codes)
}

/// 请求构造方向 (桥用): codes → 文档 §8 instruments 数组。
/// exchange 由 code 前缀推导 (6→Shanghai, 0/3→Shenzhen, 4/8/9→Beijing)。
pub fn instruments_for(codes: &[String]) -> Value {
    serde_json::json!({
        "instruments": codes.iter().map(|c| {
            serde_json::json!({
                "exchange": exchange_of(c),
                "code": c,
                "asset_class": "Equity",
            })
        }).collect::<Vec<_>>()
    })
}

fn exchange_of(code: &str) -> &'static str {
    match code.as_bytes().first() {
        Some(b'6') => "Shanghai",
        Some(b'0' | b'3') => "Shenzhen",
        Some(b'4' | b'8' | b'9') => "Beijing",
        _ => "Unknown",
    }
}

/// params["date"]: "YYYY-MM-DD"; 缺省 → 今天 (Local 时区)。
pub fn resolve_date(p: &Value) -> Result<NaiveDate, ParamsError> {
    match p.get("date") {
        None => Ok(Local::now().date_naive()),
        Some(v) => {
            let s = v.as_str().ok_or_else(|| {
                ParamsError::InvalidArgument("date 必须是字符串 YYYY-MM-DD".into())
            })?;
            NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
                ParamsError::InvalidArgument(format!("date 格式非法 ({e}): {s}"))
            })
        }
    }
}

/// params[key]: 必填字符串。缺失/非字符串 → Err。
pub fn resolve_required_string(p: &Value, key: &str) -> Result<String, ParamsError> {
    let v = p.get(key).ok_or_else(|| {
        ParamsError::InvalidArgument(format!("{key} 必填"))
    })?;
    let s = v.as_str().ok_or_else(|| {
        ParamsError::InvalidArgument(format!("{key} 必须是字符串"))
    })?;
    if s.is_empty() {
        return Err(ParamsError::InvalidArgument(format!("{key} 不能为空")));
    }
    Ok(s.to_string())
}

/// params[key]: 必填日期 "YYYY-MM-DD"。缺失/格式非法 → Err。
pub fn resolve_required_date(p: &Value, key: &str) -> Result<NaiveDate, ParamsError> {
    let s = resolve_required_string(p, key)?;
    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
        .map_err(|e| ParamsError::InvalidArgument(format!("{key} 格式非法 ({e}): {s}")))
}

/// params[key]: 无符号整数字段; 缺省 → default。
pub fn resolve_u32(p: &Value, key: &str, default: u32) -> Result<u32, ParamsError> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| {
                ParamsError::InvalidArgument(format!("{key} 必须是无符号整数"))
            })?;
            u32::try_from(n)
                .map_err(|_| ParamsError::InvalidArgument(format!("{key} 超出 u32 范围")))
        }
    }
}

/// params[key]: 字符串枚举字段; 缺省 → default。非法值 → Err。
pub fn resolve_enum_str<'a>(
    p: &'a Value,
    key: &str,
    allowed: &'a [&'a str],
    default: &'a str,
) -> Result<&'a str, ParamsError> {
    let value = match p.get(key) {
        None => default,
        Some(v) => v.as_str().ok_or_else(|| {
            ParamsError::InvalidArgument(format!("{key} 必须是字符串"))
        })?,
    };
    if allowed.iter().any(|a| *a == value) {
        Ok(value)
    } else {
        Err(ParamsError::InvalidArgument(format!(
            "{key} 非法值 {value:?} (允许: {allowed:?})"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codes_defaults_to_watchlist() {
        // 不设 STOCK_LIST 时 → 空 (不静默填充)。
        std::env::remove_var("STOCK_LIST");
        assert_eq!(resolve_codes(&json!({})).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn codes_explicit_overrides_watchlist() {
        std::env::set_var("STOCK_LIST", "600519,000001");
        let codes = resolve_codes(&json!({"codes": ["600519"]})).unwrap();
        assert_eq!(codes, vec!["600519"]);
        std::env::remove_var("STOCK_LIST");
    }

    #[test]
    fn codes_rejects_non_array() {
        assert!(matches!(
            resolve_codes(&json!({"codes": 3})),
            Err(ParamsError::InvalidArgument(_))
        ));
        assert!(matches!(
            resolve_codes(&json!({"codes": ["600519", 3]})),
            Err(ParamsError::InvalidArgument(_))
        ));
    }

    #[test]
    fn date_defaults_to_today_and_parses_explicit() {
        let today = Local::now().date_naive();
        assert_eq!(resolve_date(&json!({})).unwrap(), today);
        assert_eq!(
            resolve_date(&json!({"date": "2026-08-13"})).unwrap(),
            NaiveDate::from_ymd_opt(2026, 8, 13).unwrap()
        );
        assert!(matches!(
            resolve_date(&json!({"date": "13/08/2026"})),
            Err(ParamsError::InvalidArgument(_))
        ));
    }

    #[test]
    fn required_date_parses_or_rejects() {
        assert!(matches!(
            resolve_required_date(&json!({}), "window_start"),
            Err(ParamsError::InvalidArgument(_))
        ));
        assert_eq!(
            resolve_required_date(&json!({"window_start": "2026-06-01"}), "window_start").unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
        assert!(matches!(
            resolve_required_date(&json!({"window_start": "2026/06/01"}), "window_start"),
            Err(ParamsError::InvalidArgument(_))
        ));
    }

    #[test]
    fn required_string_missing_or_non_string_rejected() {
        assert!(matches!(
            resolve_required_string(&json!({}), "query"),
            Err(ParamsError::InvalidArgument(_))
        ));
        assert!(matches!(
            resolve_required_string(&json!({"query": 42}), "query"),
            Err(ParamsError::InvalidArgument(_))
        ));
        assert!(matches!(
            resolve_required_string(&json!({"query": ""}), "query"),
            Err(ParamsError::InvalidArgument(_))
        ));
        assert_eq!(
            resolve_required_string(&json!({"query": "白酒"}), "query").unwrap(),
            "白酒"
        );
    }

    #[test]
    fn u32_default_and_range() {
        assert_eq!(resolve_u32(&json!({}), "days", 120).unwrap(), 120);
        assert_eq!(resolve_u32(&json!({"days": 30}), "days", 120).unwrap(), 30);
        assert!(matches!(
            resolve_u32(&json!({"days": -1}), "days", 120),
            Err(ParamsError::InvalidArgument(_))
        ));
    }

    #[test]
    fn enum_str_validates_against_allowed() {
        assert_eq!(
            resolve_enum_str(&json!({}), "interval", &["day1", "minute1"], "day1").unwrap(),
            "day1"
        );
        assert_eq!(
            resolve_enum_str(&json!({"interval": "minute1"}), "interval", &["day1", "minute1"], "day1")
                .unwrap(),
            "minute1"
        );
        assert!(matches!(
            resolve_enum_str(&json!({"interval": "week1"}), "interval", &["day1", "minute1"], "day1"),
            Err(ParamsError::InvalidArgument(_))
        ));
    }

    #[test]
    fn main_indices_has_exactly_six() {
        assert_eq!(MAIN_INDICES.len(), 6);
        assert_eq!(MAIN_INDICES[0], ("sh000001", "上证指数"));
    }

    #[test]
    fn instruments_defaults_to_watchlist_and_parses_doc_format() {
        // 缺省 → watchlist (与 codes 语义一致; watchlist 值由 codes_defaults 测试覆盖)。
        assert!(resolve_instruments(&json!({})).is_ok());
        // 文档 §8 例子。
        assert_eq!(
            resolve_instruments(&json!({
                "instruments": [
                    {"exchange": "Shanghai", "code": "600396", "asset_class": "Equity"},
                    {"exchange": "Shenzhen", "code": "000001", "asset_class": "Equity"}
                ]
            }))
            .unwrap(),
            vec!["600396".to_string(), "000001".to_string()]
        );
        // 未知 exchange fail-closed。
        assert!(matches!(
            resolve_instruments(&json!({
                "instruments": [{"exchange": "Tokyo", "code": "600396", "asset_class": "Equity"}]
            })),
            Err(ParamsError::InvalidArgument(_))
        ));
        // 非 Equity 拒绝。
        assert!(matches!(
            resolve_instruments(&json!({
                "instruments": [{"exchange": "Shanghai", "code": "600396", "asset_class": "Bond"}]
            })),
            Err(ParamsError::InvalidArgument(_))
        ));
        // 缺字段拒绝。
        assert!(matches!(
            resolve_instruments(&json!({
                "instruments": [{"exchange": "Shanghai", "asset_class": "Equity"}]
            })),
            Err(ParamsError::InvalidArgument(_))
        ));
    }

    #[test]
    fn instruments_for_derives_exchange_by_prefix() {
        let p = instruments_for(&["600396".into(), "000001".into(), "430001".into()]);
        let arr = p["instruments"].as_array().unwrap();
        assert_eq!(arr[0]["exchange"], "Shanghai");
        assert_eq!(arr[0]["code"], "600396");
        assert_eq!(arr[0]["asset_class"], "Equity");
        assert_eq!(arr[1]["exchange"], "Shenzhen");
        assert_eq!(arr[2]["exchange"], "Beijing");
    }
}
