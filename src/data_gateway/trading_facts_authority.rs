//! Qualified trading facts **authority adapter** (甲-1, 2026-10-14 用户决策).
//!
//! `src/data_gateway/qualified_trading_facts.rs` 早已实现校验层
//! （`QualifiedTradingFacts::admit`：字段级 reason_code、覆盖窗口、证据一致性），
//! 但 `QualifiedTradingFactsGateway::acquire` 一直无条件返回
//! `ContractNotDelivered`——因为**没有任何来源**能供出 lifecycle / 价格带 / 停牌这三件事实。
//!
//! 取证结论（2026-10-14）：lifecycle 与价格带的数据已在证券资料视图里
//! （board / is_st / listed_on / price_limit_percent / price_limit_version + 证据），
//! 但**停牌在全系统没有任何来源**——两份合同无字段、正式停牌缓存无写入方、
//! K 线缺口推断被模块头明文禁止用作停牌授权。用户据此选择「由对端交付该事实」（甲-1）。
//!
//! 本模块就是对端交付物的 Mac 侧适配器：
//! - [`parse_authority_record`] / [`publish_authority_batch`]：把对端 payload 严格解析成
//!   `AuthorityTradingFactsRecord`（校验失败即拒绝，绝不部分采纳）；
//! - 进程内存储：monitor 取到后 [`publish_authority_batch`]，`acquire` 同步读取；
//! - 未交付 / 未发布时 `acquire` 行为**完全不变**（`ContractNotDelivered`，fail-closed）。
//!
//! 对端交付规格（schema/字段/语义）见
//! `client-bundle/WINDOWS_TRADING_FACTS_AUTHORITY_DELIVERY_REQUEST_20261014_V1.md`。

use std::collections::HashMap;
use std::sync::RwLock;

use chrono::NaiveDate;
use once_cell::sync::Lazy;
use serde_json::Value;

use crate::market_domain::{AssetClass, Exchange, InstrumentId, ProviderId};

use super::qualified_trading_facts::{
    AuthorityLifecycle, AuthoritySuspensionCoverage, AuthorityTradingFactsRecord,
    QualifiedPriceBand, SuspensionWindow,
};
use super::review::BatchEvidence;
use super::SecurityBoard;

/// 对端 facts payload 的 schema 名（独立于 legacy SecurityMetadata 视图）。
pub const TRADING_FACTS_AUTHORITY_SCHEMA: &str = "magic.market.qualified_trading_facts";
/// 本适配器接受的 schema 版本。
pub const TRADING_FACTS_AUTHORITY_SCHEMA_VERSION: u64 = 1;
/// 记录内的合同版本号（对端须原样回填）。
pub const TRADING_FACTS_AUTHORITY_CONTRACT_VERSION: &str = "qualified-trading-facts-authority-v1";

/// 解析拒绝：每条都带稳定 reason_code（进审计/日志，不靠文案判断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradingFactsAuthorityRejection {
    reason_code: &'static str,
    detail: String,
}

impl TradingFactsAuthorityRejection {
    fn new(reason_code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason_code,
            detail: detail.into(),
        }
    }

    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl std::fmt::Display for TradingFactsAuthorityRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.reason_code, self.detail)
    }
}

impl std::error::Error for TradingFactsAuthorityRejection {}

fn malformed(field: &'static str, detail: impl Into<String>) -> TradingFactsAuthorityRejection {
    TradingFactsAuthorityRejection::new(
        "trading_facts_authority_malformed",
        format!("{field}: {}", detail.into()),
    )
}

fn required<'a>(
    value: &'a Value,
    field: &'static str,
) -> Result<&'a Value, TradingFactsAuthorityRejection> {
    value
        .get(field)
        .filter(|value| !value.is_null())
        .ok_or_else(|| malformed(field, "缺失或为 null"))
}

fn required_str(
    value: &Value,
    field: &'static str,
) -> Result<String, TradingFactsAuthorityRejection> {
    let text = required(value, field)?
        .as_str()
        .ok_or_else(|| malformed(field, "必须是字符串"))?
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(malformed(field, "不得为空串"));
    }
    Ok(text)
}

fn required_bool(
    value: &Value,
    field: &'static str,
) -> Result<bool, TradingFactsAuthorityRejection> {
    required(value, field)?
        .as_bool()
        .ok_or_else(|| malformed(field, "必须是布尔"))
}

fn required_i64(value: &Value, field: &'static str) -> Result<i64, TradingFactsAuthorityRejection> {
    required(value, field)?
        .as_i64()
        .ok_or_else(|| malformed(field, "必须是整数"))
}

fn required_date(
    value: &Value,
    field: &'static str,
) -> Result<NaiveDate, TradingFactsAuthorityRejection> {
    let text = required_str(value, field)?;
    NaiveDate::parse_from_str(&text, "%Y-%m-%d")
        .map_err(|error| malformed(field, format!("日期须 YYYY-MM-DD: {error}")))
}

fn optional_date(
    value: &Value,
    field: &'static str,
) -> Result<Option<NaiveDate>, TradingFactsAuthorityRejection> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => Ok(Some(required_date(value, field)?)),
    }
}

fn board_from_name(name: &str) -> Result<SecurityBoard, TradingFactsAuthorityRejection> {
    Ok(match name {
        "Main" => SecurityBoard::Main,
        "Star" => SecurityBoard::Star,
        "ChiNext" => SecurityBoard::ChiNext,
        "Beijing" => SecurityBoard::Beijing,
        other => {
            return Err(malformed(
                "price_regime.board",
                format!("未知板块名: {other}"),
            ))
        }
    })
}

fn exchange_from_name(name: &str) -> Result<Exchange, TradingFactsAuthorityRejection> {
    Ok(match name {
        "Shanghai" => Exchange::Shanghai,
        "Shenzhen" => Exchange::Shenzhen,
        "Beijing" => Exchange::Beijing,
        other => {
            return Err(malformed(
                "instrument.exchange",
                format!("未知交易所: {other}"),
            ))
        }
    })
}

fn asset_class_from_name(name: &str) -> Result<AssetClass, TradingFactsAuthorityRejection> {
    Ok(match name {
        "Equity" => AssetClass::Equity,
        other => {
            return Err(malformed(
                "instrument.asset_class",
                format!("本适配器只接受 Equity，收到: {other}"),
            ))
        }
    })
}

fn instrument_from_record(record: &Value) -> Result<InstrumentId, TradingFactsAuthorityRejection> {
    let instrument = required(record, "instrument")?;
    if !instrument.is_object() {
        return Err(malformed("instrument", "必须是对象"));
    }
    let exchange = exchange_from_name(&required_str(instrument, "exchange")?)?;
    let code = required_str(instrument, "code")?;
    let asset_class = asset_class_from_name(&required_str(instrument, "asset_class")?)?;
    InstrumentId::new(exchange, &code, asset_class)
        .map_err(|error| malformed("instrument", format!("身份非法: {error}")))
}

fn evidence_from_record(record: &Value) -> Result<BatchEvidence, TradingFactsAuthorityRejection> {
    let evidence = required(record, "evidence")?;
    if !evidence.is_object() {
        return Err(malformed("evidence", "必须是对象"));
    }
    let provider_name = required_str(evidence, "provider")?;
    let provider: ProviderId = serde_json::from_value(Value::String(provider_name))
        .map_err(|_| malformed("evidence.provider", "不在 ProviderId 枚举内"))?;
    let source = required_str(evidence, "source")?;
    let source_at = required_str(evidence, "source_at")?;
    let observed_at = required_str(evidence, "observed_at")?;
    let batch_id = required_str(evidence, "batch_id")?;
    let source_instant = chrono::DateTime::parse_from_rfc3339(&source_at)
        .map_err(|error| malformed("evidence.source_at", format!("须 RFC3339: {error}")))?;
    let observed_instant = chrono::DateTime::parse_from_rfc3339(&observed_at)
        .map_err(|error| malformed("evidence.observed_at", format!("须 RFC3339: {error}")))?;
    if source_instant > observed_instant {
        return Err(malformed("evidence", "source_at 晚于 observed_at"));
    }
    Ok(BatchEvidence {
        provider,
        source,
        source_at: Some(source_at),
        observed_at,
        batch_id,
    })
}

fn lifecycle_from_record(
    record: &Value,
) -> Result<AuthorityLifecycle, TradingFactsAuthorityRejection> {
    let lifecycle = required(record, "lifecycle")?;
    let listed_on = required_date(lifecycle, "listed_on")?;
    let delisted_on = optional_date(lifecycle, "delisted_on")?;
    let covered_through = required_date(lifecycle, "covered_through")?;
    if covered_through < listed_on {
        return Err(malformed("lifecycle.covered_through", "覆盖终点早于上市日"));
    }
    if delisted_on.is_some_and(|delisted_on| delisted_on <= listed_on) {
        return Err(malformed("lifecycle.delisted_on", "退市日必须晚于上市日"));
    }
    if delisted_on.is_some_and(|delisted_on| delisted_on > covered_through) {
        return Err(malformed(
            "lifecycle.delisted_on",
            "退市日晚于覆盖终点（覆盖不完整）",
        ));
    }
    Ok(AuthorityLifecycle {
        listed_on,
        delisted_on,
        covered_through,
    })
}

fn price_regime_from_record(
    record: &Value,
) -> Result<QualifiedPriceBand, TradingFactsAuthorityRejection> {
    let regime = required(record, "price_regime")?;
    let board = board_from_name(&required_str(regime, "board")?)?;
    let is_st = required_bool(regime, "is_st")?;
    let tick_micros = required_i64(regime, "tick_micros")?;
    let lower_price_micros = required_i64(regime, "lower_price_micros")?;
    let upper_price_micros = required_i64(regime, "upper_price_micros")?;
    let effective_from = required_date(regime, "effective_from")?;
    let effective_through = required_date(regime, "effective_through")?;
    let version = required_str(regime, "version")?;
    QualifiedPriceBand::new(
        board,
        is_st,
        tick_micros,
        lower_price_micros,
        upper_price_micros,
        effective_from,
        effective_through,
        version,
    )
    .map_err(|error| malformed("price_regime", format!("结构非法: {error}")))
}

fn suspension_from_record(
    record: &Value,
) -> Result<AuthoritySuspensionCoverage, TradingFactsAuthorityRejection> {
    let suspension = required(record, "suspension")?;
    let covered_from = required_date(suspension, "covered_from")?;
    let covered_through = required_date(suspension, "covered_through")?;
    if covered_through < covered_from {
        return Err(malformed("suspension", "覆盖区间反向"));
    }
    let windows_value = suspension
        .get("windows")
        .ok_or_else(|| malformed("suspension.windows", "缺失（可为空数组，但必须显式给出）"))?;
    let windows_array = windows_value
        .as_array()
        .ok_or_else(|| malformed("suspension.windows", "必须是数组"))?;
    let mut windows = Vec::with_capacity(windows_array.len());
    for (index, window) in windows_array.iter().enumerate() {
        let halted_from = required_date(window, "halted_from")?;
        let halted_through = required_date(window, "halted_through")?;
        if halted_through < halted_from {
            return Err(malformed(
                "suspension.windows",
                format!("第 {index} 段区间反向"),
            ));
        }
        if halted_from < covered_from || halted_through > covered_through {
            return Err(malformed(
                "suspension.windows",
                format!("第 {index} 段超出覆盖区间"),
            ));
        }
        if let Some(previous) = windows.last() {
            let previous: &SuspensionWindow = previous;
            if halted_from <= previous.halted_through {
                return Err(malformed(
                    "suspension.windows",
                    format!("第 {index} 段与前一段重叠或未按时间升序"),
                ));
            }
        }
        windows.push(SuspensionWindow {
            halted_from,
            halted_through,
        });
    }
    Ok(AuthoritySuspensionCoverage::with_windows(
        covered_from,
        covered_through,
        windows,
    ))
}

/// 解析一条对端 facts 记录。任何字段缺失/越界/自相矛盾 → 拒绝整条（不部分采纳）。
pub fn parse_authority_record(
    record: &Value,
) -> Result<AuthorityTradingFactsRecord, TradingFactsAuthorityRejection> {
    if !record.is_object() {
        return Err(malformed("record", "必须是对象"));
    }
    let instrument = instrument_from_record(record)?;
    let lifecycle = lifecycle_from_record(record)?;
    let price_regime = price_regime_from_record(record)?;
    let suspension = suspension_from_record(record)?;
    let evidence = evidence_from_record(record)?;
    let contract_version = required_str(record, "contract_version")?;
    if contract_version != TRADING_FACTS_AUTHORITY_CONTRACT_VERSION {
        return Err(TradingFactsAuthorityRejection::new(
            "trading_facts_authority_contract_version_mismatch",
            format!("期望 {TRADING_FACTS_AUTHORITY_CONTRACT_VERSION}，收到 {contract_version}"),
        ));
    }
    let fresh_through = required_date(record, "fresh_through")?;
    Ok(AuthorityTradingFactsRecord {
        instrument,
        lifecycle: Some(lifecycle),
        price_regime: Some(price_regime),
        suspension: Some(suspension),
        evidence,
        contract_version,
        fresh_through,
    })
}

type AuthorityRecordsByCode = HashMap<String, AuthorityTradingFactsRecord>;

static AUTHORITY_RECORDS: Lazy<RwLock<AuthorityRecordsByCode>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// 解析并发布一批对端记录（monitor 取到 payload 后调用）。
///
/// 整批校验：**任一条非法即整批拒绝**，不写入任何记录——避免"部分采纳"导致
/// 同一批不同标的拿到不同等级的证据。
pub fn publish_authority_batch(
    schema: &str,
    schema_version: u64,
    records: &[Value],
) -> Result<usize, TradingFactsAuthorityRejection> {
    if schema != TRADING_FACTS_AUTHORITY_SCHEMA {
        return Err(TradingFactsAuthorityRejection::new(
            "trading_facts_authority_schema_mismatch",
            format!("期望 {TRADING_FACTS_AUTHORITY_SCHEMA}，收到 {schema}"),
        ));
    }
    if schema_version != TRADING_FACTS_AUTHORITY_SCHEMA_VERSION {
        return Err(TradingFactsAuthorityRejection::new(
            "trading_facts_authority_schema_version_unsupported",
            format!("期望 {TRADING_FACTS_AUTHORITY_SCHEMA_VERSION}，收到 {schema_version}"),
        ));
    }
    let mut parsed = Vec::with_capacity(records.len());
    for record in records {
        parsed.push(parse_authority_record(record)?);
    }
    let mut guard = AUTHORITY_RECORDS
        .write()
        .expect("trading facts authority store poisoned");
    let mut published = 0usize;
    for record in parsed {
        guard.insert(record.instrument.code().to_owned(), record);
        published += 1;
    }
    Ok(published)
}

/// 同步查表（`QualifiedTradingFactsGateway::acquire` 用）。
pub(crate) fn lookup_authority_record(
    instrument: &InstrumentId,
) -> Option<AuthorityTradingFactsRecord> {
    AUTHORITY_RECORDS
        .read()
        .ok()
        .and_then(|guard| guard.get(instrument.code()).cloned())
}

/// 当前已发布的记录数（观测/日志用；不暴露记录内容）。
pub fn published_authority_record_count() -> usize {
    AUTHORITY_RECORDS
        .read()
        .map(|guard| guard.len())
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn clear_authority_records() {
    if let Ok(mut guard) = AUTHORITY_RECORDS.write() {
        guard.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 发布存储是**进程级全局**，而 cargo 默认并行跑测试——必须串行化，
    /// 否则一个测试的 `clear` 会把另一个测试刚发布的记录清掉（计数断言随机失败）。
    static STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_store() -> std::sync::MutexGuard<'static, ()> {
        STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn fixture(code: &str) -> Value {
        serde_json::json!({
            "instrument": {"exchange": "Shanghai", "code": code, "asset_class": "Equity"},
            "lifecycle": {
                "listed_on": "2001-08-27",
                "delisted_on": null,
                "covered_through": "2026-12-31"
            },
            "price_regime": {
                "board": "Main",
                "is_st": false,
                "tick_micros": 10000,
                "lower_price_micros": 9900000,
                "upper_price_micros": 12100000,
                "effective_from": "2026-10-14",
                "effective_through": "2026-10-14",
                "version": "TEST_CODE_SSE_BAND_20261014"
            },
            "suspension": {
                "covered_from": "2026-01-01",
                "covered_through": "2026-12-31",
                "windows": []
            },
            "evidence": {
                "provider": "Sse",
                "source": "TEST_CODE_EXCHANGE_FACTS",
                "source_at": "2026-10-14T01:00:00Z",
                "observed_at": "2026-10-14T01:00:01Z",
                "batch_id": "TEST_CODE_FACTS_BATCH"
            },
            "contract_version": TRADING_FACTS_AUTHORITY_CONTRACT_VERSION,
            "fresh_through": "2026-10-14"
        })
    }

    fn publish_one(code: &str) -> usize {
        publish_authority_batch(
            TRADING_FACTS_AUTHORITY_SCHEMA,
            TRADING_FACTS_AUTHORITY_SCHEMA_VERSION,
            &[fixture(code)],
        )
        .expect("TEST_CODE fixture must parse")
    }

    #[test]
    fn authority_record_parses_and_publishes_with_full_field_coverage() {
        let _guard = lock_store();
        clear_authority_records();
        let code = "TEST_CODE_AUTHORITY_001";
        let record = parse_authority_record(&fixture(code)).unwrap();
        assert_eq!(record.instrument.code(), code);
        assert_eq!(
            record.contract_version,
            TRADING_FACTS_AUTHORITY_CONTRACT_VERSION
        );
        assert_eq!(
            record.lifecycle.as_ref().unwrap().listed_on,
            NaiveDate::from_ymd_opt(2001, 8, 27).unwrap()
        );
        let regime = record.price_regime.as_ref().unwrap();
        assert_eq!(regime.board(), SecurityBoard::Main);
        assert_eq!(regime.tick_micros(), 10_000);
        assert_eq!(publish_one(code), 1);
        assert_eq!(published_authority_record_count(), 1);
        clear_authority_records();
    }

    #[test]
    fn acquire_serves_published_facts_and_stays_closed_without_them() {
        let _guard = lock_store();
        use crate::data_gateway::qualified_trading_facts::{
            QualifiedListingStatus, QualifiedSuspensionStatus,
            QualifiedTradingFactsGateway as FactsGateway, QualifiedTradingFactsRequest,
            TradingFactUnavailableReason,
        };
        clear_authority_records();
        let instrument = InstrumentId::new(
            Exchange::Shanghai,
            "TEST_CODE_AUTHORITY_002",
            AssetClass::Equity,
        )
        .unwrap();
        let effective_on = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        let gateway = FactsGateway::new();

        // 未交付：行为与改造前完全一致（ContractNotDelivered，fail-closed）。
        let closed = gateway.acquire(QualifiedTradingFactsRequest::new(
            instrument.clone(),
            effective_on,
        ));
        assert_eq!(
            closed.lifecycle().require().unwrap_err().reason(),
            TradingFactUnavailableReason::ContractNotDelivered
        );

        // 已发布：三件事实全部可用。
        publish_one("TEST_CODE_AUTHORITY_002");
        let open = gateway.acquire(QualifiedTradingFactsRequest::new(instrument, effective_on));
        assert_eq!(
            open.lifecycle().require().unwrap(),
            &QualifiedListingStatus::Listed
        );
        assert_eq!(
            open.suspension().require().unwrap(),
            &QualifiedSuspensionStatus::Trading
        );
        assert!(open.price_regime().require().is_ok());
        clear_authority_records();
    }

    #[test]
    fn authority_batch_rejects_schema_version_and_contract_version_mismatches() {
        let _guard = lock_store();
        clear_authority_records();
        let error = publish_authority_batch(
            TRADING_FACTS_AUTHORITY_SCHEMA,
            2,
            &[fixture("TEST_CODE_AUTHORITY_003")],
        )
        .unwrap_err();
        assert_eq!(
            error.reason_code(),
            "trading_facts_authority_schema_version_unsupported"
        );
        let error = publish_authority_batch(
            "magic.market.security_metadata",
            TRADING_FACTS_AUTHORITY_SCHEMA_VERSION,
            &[fixture("TEST_CODE_AUTHORITY_003")],
        )
        .unwrap_err();
        assert_eq!(
            error.reason_code(),
            "trading_facts_authority_schema_mismatch"
        );
        let mut wrong_contract = fixture("TEST_CODE_AUTHORITY_003");
        wrong_contract["contract_version"] = Value::String("TEST_CODE_OTHER".to_owned());
        let error = parse_authority_record(&wrong_contract).unwrap_err();
        assert_eq!(
            error.reason_code(),
            "trading_facts_authority_contract_version_mismatch"
        );
        assert_eq!(published_authority_record_count(), 0);
    }

    #[test]
    fn authority_record_is_all_or_nothing_on_any_malformed_field() {
        let _guard = lock_store();
        clear_authority_records();
        let mutations: [(&str, fn(&mut Value)); 9] = [
            ("缺 lifecycle", |value| {
                value.as_object_mut().unwrap().remove("lifecycle");
            }),
            ("清单上市日非法", |value| {
                value["lifecycle"]["listed_on"] = Value::String("2026-13-40".to_owned());
            }),
            ("覆盖终点早于上市日", |value| {
                value["lifecycle"]["covered_through"] = Value::String("1999-01-01".to_owned());
            }),
            ("未知板块", |value| {
                value["price_regime"]["board"] = Value::String("TEST_CODE_BOARD".to_owned());
            }),
            ("tick 与限价不对齐", |value| {
                value["price_regime"]["upper_price_micros"] = serde_json::json!(12_100_001_i64);
            }),
            ("停牌段超出覆盖区间", |value| {
                value["suspension"]["windows"] = serde_json::json!([
                    {"halted_from": "2025-12-31", "halted_through": "2026-01-02"}
                ]);
            }),
            ("证据时间倒挂", |value| {
                value["evidence"]["source_at"] = Value::String("2026-10-14T02:00:00Z".to_owned());
            }),
            ("provider 未知", |value| {
                value["evidence"]["provider"] = Value::String("TEST_CODE_PROVIDER".to_owned());
            }),
            ("缺 fresh_through", |value| {
                value.as_object_mut().unwrap().remove("fresh_through");
            }),
        ];
        for (label, mutate) in mutations {
            let mut record = fixture("TEST_CODE_AUTHORITY_004");
            mutate(&mut record);
            assert!(
                parse_authority_record(&record).is_err(),
                "case={label} 必须被拒绝"
            );
        }
        // 整批拒绝：一条坏 → 一条都不发布。
        let mut bad = fixture("TEST_CODE_AUTHORITY_005");
        bad["price_regime"]["tick_micros"] = serde_json::json!(0_i64);
        let error = publish_authority_batch(
            TRADING_FACTS_AUTHORITY_SCHEMA,
            TRADING_FACTS_AUTHORITY_SCHEMA_VERSION,
            &[fixture("TEST_CODE_AUTHORITY_006"), bad],
        )
        .unwrap_err();
        assert_eq!(error.reason_code(), "trading_facts_authority_malformed");
        assert_eq!(
            published_authority_record_count(),
            0,
            "整批拒绝时不得留下任何已发布记录"
        );
    }
}
