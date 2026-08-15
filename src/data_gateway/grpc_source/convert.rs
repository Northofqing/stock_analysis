//! P4 M2: gRPC 响应 → 客户端类型化 GatewayBatch 转换。
//! 与服务端 delegate.rs fetch_xxx 的 JSON 视图逐字段镜像 (每条转换注明对应
//! fetch 行号, 视图字段名以 delegate 的 json! 键名为准 — 例如 change_pct 对应
//! 结构体字段 change_percent)。
//!
//! 缺字段/缺证据 → GatewayError::invalid_evidence (fail-closed, 绝不静默填充)。
//! 空 records → GatewayBatch::VerifiedEmpty (服务端 proven empty, 不 collapse
//! 成 unavailable)。
use crate::data_gateway::{
    BatchEvidence, GatewayBatch, GatewayError, MarketBookLevel, MarketMinutePoint,
    MarketMoneyFlow, MarketOrderBook, MarketSecurityMetadata, RealtimeMarketQuote,
    SecurityBoard,
};
use crate::data_provider::{AdjustType, KlineData};
use crate::grpc_client::envelope::QueryResult;
use chrono::{DateTime, NaiveDate, Utc};
use magic_market_core::ProviderId;
use serde_json::Value;

/// bridge 缺证据时的 capability 标记 (audit_outcome=invalid_evidence)。
const BRIDGE_CAPABILITY: &str = "GrpcBridge";

fn err(capability: &'static str, msg: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(capability, None, msg)
}

/// Debug 名 → ProviderId (服务端 pack_ev 用 format!("{:?}", provider) 写 JSON)。
/// 未知/空 → Err (fail-closed: 不静默猜 Tdx)。
pub fn parse_provider(s: &str) -> Result<ProviderId, GatewayError> {
    Ok(match s {
        "Tdx" => ProviderId::Tdx,
        "Tencent" => ProviderId::Tencent,
        "Eastmoney" => ProviderId::Eastmoney,
        "Sina" => ProviderId::Sina,
        "Baostock" => ProviderId::Baostock,
        "Baidu" => ProviderId::Baidu,
        "Tonghuashun" => ProviderId::Tonghuashun,
        "Iwencai" => ProviderId::Iwencai,
        "Cninfo" => ProviderId::Cninfo,
        "Cailianpress" => ProviderId::Cailianpress,
        "Jin10" => ProviderId::Jin10,
        "ThePaper" => ProviderId::ThePaper,
        "Yonhap" => ProviderId::Yonhap,
        "WallstreetCn" => ProviderId::WallstreetCn,
        "Sse" => ProviderId::Sse,
        "Szse" => ProviderId::Szse,
        "Hkex" => ProviderId::Hkex,
        "Cffex" => ProviderId::Cffex,
        "StateCouncil" => ProviderId::StateCouncil,
        "Nbs" => ProviderId::Nbs,
        "Pbc" => ProviderId::Pbc,
        "Cfets" => ProviderId::Cfets,
        "Fred" => ProviderId::Fred,
        "Imf" => ProviderId::Imf,
        "WorldBank" => ProviderId::WorldBank,
        "SecEdgar" => ProviderId::SecEdgar,
        "XinhuaFinance" => ProviderId::XinhuaFinance,
        "Yicai" => ProviderId::Yicai,
        "SecuritiesTimes" => ProviderId::SecuritiesTimes,
        "LocalAnalysis" => ProviderId::LocalAnalysis,
        "LocalTerminal" => ProviderId::LocalTerminal,
        "Custom" => ProviderId::Custom,
        "" => {
            return Err(err(
                BRIDGE_CAPABILITY,
                "provider 空 (服务端未回填证据链, 旧 op 尚在升级中)",
            ))
        }
        other => {
            return Err(err(
                BRIDGE_CAPABILITY,
                format!("未知 provider Debug 名: {other}"),
            ))
        }
    })
}

/// 服务端 Debug 名 → SecurityBoard (fetch_security_metadata 视图)。
fn parse_board(s: &str) -> Result<SecurityBoard, GatewayError> {
    Ok(match s {
        "Main" => SecurityBoard::Main,
        "Star" => SecurityBoard::Star,
        "ChiNext" => SecurityBoard::ChiNext,
        "Beijing" => SecurityBoard::Beijing,
        other => {
            return Err(err(
                "SecurityMetadata",
                format!("未知 board Debug 名: {other}"),
            ))
        }
    })
}

/// 从 QueryResult 信封构造 BatchEvidence。
/// source_at 空 → None (合同 §6 缺则不填充); provider/source/batch_id 空 → Err。
fn evidence_of(q: &QueryResult, capability: &'static str) -> Result<BatchEvidence, GatewayError> {
    let provider = parse_provider(&q.selected_provider)
        .map_err(|e| err(capability, format!("selected_provider 无法解析: {e}")))?;
    if q.source.is_empty() {
        return Err(err(capability, "source 空 (服务端未回填证据链)"));
    }
    if q.batch_id.is_empty() {
        return Err(err(capability, "batch_id 空 (服务端未回填证据链)"));
    }
    Ok(BatchEvidence {
        provider,
        source: q.source.clone(),
        source_at: if q.source_at.is_empty() {
            None
        } else {
            Some(q.source_at.clone())
        },
        observed_at: q.observed_at.clone(),
        batch_id: q.batch_id.clone(),
    })
}

/// records data 字节 → Value 数组。
fn parse_records(q: &QueryResult, capability: &'static str) -> Result<Vec<Value>, GatewayError> {
    let Some(payload) = q.records.first() else {
        return Err(err(
            capability,
            "records 空 (服务端无 canonical payload)",
        ));
    };
    serde_json::from_slice(&payload.data)
        .map_err(|e| err(capability, format!("records 非 JSON 数组: {e}")))
}

fn as_str(v: &Value, key: &str, capability: &'static str) -> Result<String, GatewayError> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| err(capability, format!("record 缺字符串字段 {key}")))
}

fn as_f64(v: &Value, key: &str, capability: &'static str) -> Result<f64, GatewayError> {
    v.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| err(capability, format!("record 缺数值字段 {key}")))
}

fn as_bool(v: &Value, key: &str, capability: &'static str) -> Result<bool, GatewayError> {
    v.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| err(capability, format!("record 缺布尔字段 {key}")))
}

fn as_rfc3339(v: &Value, key: &str, capability: &'static str) -> Result<DateTime<Utc>, GatewayError> {
    let s = as_str(v, key, capability)?;
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| err(capability, format!("字段 {key} 非 RFC3339: {s} ({e})")))
}

fn as_date(v: &Value, key: &str, capability: &'static str) -> Result<NaiveDate, GatewayError> {
    let s = as_str(v, key, capability)?;
    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
        .map_err(|e| err(capability, format!("字段 {key} 非 YYYY-MM-DD: {s} ({e})")))
}

/// 逐代码 op 共用: 记录必须全部属于目标 code (服务端按 watchlist 拼接时,
/// params 未生效的中间态会被这里 fail-closed 拒绝, 不把错误代码当数据)。
fn per_code_records<'a>(
    q: &'a QueryResult,
    capability: &'static str,
    code: &'a str,
) -> Result<(Vec<Value>, BatchEvidence), GatewayError> {
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok((Vec::new(), ev));
    }
    if parsed
        .iter()
        .any(|v| v.get("code").and_then(Value::as_str) != Some(code))
    {
        return Err(err(
            capability,
            format!("服务端返回了非目标代码 {code} 的记录 (params 未生效?)"),
        ));
    }
    Ok((parsed, ev))
}

/// record 时间戳: 视图无逐条 source_at 的 op → 用证据链 source_at (fail-closed: 空 = Err)。
fn record_source_at(q: &QueryResult, capability: &'static str) -> Result<DateTime<Utc>, GatewayError> {
    if q.source_at.is_empty() {
        return Err(err(capability, "source_at 空 (服务端未回填证据链)"));
    }
    DateTime::parse_from_rfc3339(&q.source_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| err(capability, format!("source_at 非 RFC3339: {} ({e})", q.source_at)))
}

fn record_observed_at(q: &QueryResult, capability: &'static str) -> Result<DateTime<Utc>, GatewayError> {
    DateTime::parse_from_rfc3339(&q.observed_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| err(capability, format!("observed_at 非 RFC3339: {} ({e})", q.observed_at)))
}

// ---------- 6 个首批 op (M2) ----------

/// 统一实时行情。视图: delegate.rs fetch_realtime_quotes (:176-184)
/// {"code","name","price","change_pct","previous_close"}。
pub fn realtime_quotes(q: &QueryResult) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
    let capability = "RealtimeMarketQuotes";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<RealtimeMarketQuote> = parsed
        .iter()
        .map(|v| {
            Ok(RealtimeMarketQuote {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                price: as_f64(v, "price", capability)?,
                change_percent: as_f64(v, "change_pct", capability)?,
                previous_close: as_f64(v, "previous_close", capability)?,
                source_at: record_source_at(q, capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 分钟线。视图: delegate.rs fetch_minute_data (:211-219)
/// {"code","minute_at","price","cumulative_quantity","cumulative_amount","source_at"}。
pub fn minute_data(q: &QueryResult) -> Result<GatewayBatch<MarketMinutePoint>, GatewayError> {
    let capability = "MarketMinuteData";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketMinutePoint> = parsed
        .iter()
        .map(|v| {
            Ok(MarketMinutePoint {
                code: as_str(v, "code", capability)?,
                minute_at: as_rfc3339(v, "minute_at", capability)?,
                price: as_f64(v, "price", capability)?,
                cumulative_quantity: as_f64(v, "cumulative_quantity", capability)?,
                cumulative_amount: v.get("cumulative_amount").and_then(Value::as_f64),
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 盘口 5 档。视图: delegate.rs fetch_order_books (:235-244)
/// {"code","bids":[{"price","quantity"}],"asks","total_bid_quantity",
/// "total_ask_quantity","source_at"}。档位不足 5 → 补 0.0 (空档 = 无挂单, 语义
/// 正确); 超过 5 → 报错 (服务端视图违约)。
fn book_levels(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<[MarketBookLevel; 5], GatewayError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| err(capability, format!("record 缺数组字段 {key}")))?;
    if arr.len() > 5 {
        return Err(err(
            capability,
            format!("{key} 档位数 {} 超过 5 (服务端视图违约)", arr.len()),
        ));
    }
    let mut levels = [MarketBookLevel { price: 0.0, quantity: 0.0 }; 5];
    for (i, item) in arr.iter().enumerate() {
        levels[i] = MarketBookLevel {
            price: as_f64(item, "price", capability)?,
            quantity: as_f64(item, "quantity", capability)?,
        };
    }
    Ok(levels)
}

pub fn order_books(q: &QueryResult) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
    let capability = "MarketOrderBooks";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketOrderBook> = parsed
        .iter()
        .map(|v| {
            Ok(MarketOrderBook {
                code: as_str(v, "code", capability)?,
                bids: book_levels(v, "bids", capability)?,
                asks: book_levels(v, "asks", capability)?,
                total_bid_quantity: as_f64(v, "total_bid_quantity", capability)?,
                total_ask_quantity: as_f64(v, "total_ask_quantity", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 资金流。视图: delegate.rs fetch_money_flows (:262-270)
/// {"code","main_net","super_large_net","large_net","medium_net","small_net","source_at"}。
pub fn money_flows(q: &QueryResult) -> Result<GatewayBatch<MarketMoneyFlow>, GatewayError> {
    let capability = "MarketMoneyFlows";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketMoneyFlow> = parsed
        .iter()
        .map(|v| {
            Ok(MarketMoneyFlow {
                code: as_str(v, "code", capability)?,
                main_net: as_f64(v, "main_net", capability)?,
                super_large_net: as_f64(v, "super_large_net", capability)?,
                large_net: as_f64(v, "large_net", capability)?,
                medium_net: as_f64(v, "medium_net", capability)?,
                small_net: as_f64(v, "small_net", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 证券元数据。视图: delegate.rs fetch_security_metadata (:282-291)
/// {"code","name","board"(Debug),"is_st","listed_on","price_limit_percent","source_at"}。
pub fn security_metadata(q: &QueryResult) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
    let capability = "SecurityMetadata";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketSecurityMetadata> = parsed
        .iter()
        .map(|v| {
            let board = as_str(v, "board", capability)?;
            let listed_on = as_date(v, "listed_on", capability)?;
            Ok(MarketSecurityMetadata {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                board: parse_board(&board)?,
                is_st: as_bool(v, "is_st", capability)?,
                listed_on,
                price_limit_percent: as_f64(v, "price_limit_percent", capability)?,
                // 视图无 price_limit_version (M1 冻结) → 留空, 消费方按缺证据感知。
                price_limit_version: String::new(),
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

/// 日线 K 线。视图: delegate.rs fetch_historical_bars (:538-550)
/// {"code","date","open","high","low","close","volume","amount","pct_chg","settled"}。
/// 视图只含 KlineData 的 10 个字段子集 → 其余 Option 字段 = None、bool = false、
/// adjust = None (视图冻结, 消费者需要的字段由 M3+ 扩展服务端视图, 不在客户端
/// 发明数据)。
pub fn historical_bars(code: &str, q: &QueryResult) -> Result<GatewayBatch<KlineData>, GatewayError> {
    let capability = "HistoricalDailyBars";
    let (parsed, ev) = per_code_records(q, capability, code)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<KlineData> = parsed
        .iter()
        .map(|v| {
            Ok(KlineData {
                date: as_date(v, "date", capability)?,
                open: as_f64(v, "open", capability)?,
                high: as_f64(v, "high", capability)?,
                low: as_f64(v, "low", capability)?,
                close: as_f64(v, "close", capability)?,
                volume: as_f64(v, "volume", capability)?,
                amount: as_f64(v, "amount", capability)?,
                pct_chg: as_f64(v, "pct_chg", capability)?,
                settled: as_bool(v, "settled", capability)?,
                intraday_price: None,
                pe_ratio: None,
                pb_ratio: None,
                turnover_rate: None,
                market_cap: None,
                circulating_cap: None,
                eps: None,
                roe: None,
                revenue_yoy: None,
                net_profit_yoy: None,
                gross_margin: None,
                net_margin: None,
                sharpe_ratio: None,
                financials_history: None,
                valuation_history: None,
                consensus: None,
                industry: None,
                is_limit_up: false,
                is_limit_down: false,
                is_suspended: false,
                adjust: AdjustType::None,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available { records, evidence: ev })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::{AdmissionState, CanonicalPayload};

    fn mk_q(data: &str, provider: &str, source: &str) -> QueryResult {
        QueryResult {
            admission: AdmissionState::Admitted,
            selected_provider: provider.to_string(),
            batch_id: "b-1".to_string(),
            complete: true,
            observed_at: "2026-08-15T10:00:00+08:00".to_string(),
            source_at: "2026-08-15T09:35:00+08:00".to_string(),
            records: vec![CanonicalPayload {
                schema: "x".to_string(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_string(),
                data: data.as_bytes().to_vec(),
            }],
            source: source.to_string(),
        }
    }

    #[test]
    fn provider_debug_names_roundtrip() {
        assert_eq!(parse_provider("Tdx").unwrap(), ProviderId::Tdx);
        assert_eq!(parse_provider("Eastmoney").unwrap(), ProviderId::Eastmoney);
        assert!(parse_provider("").is_err());
        assert!(parse_provider("Mystery").is_err());
    }

    #[test]
    fn realtime_quotes_canned_roundtrip() {
        // 与 delegate.rs fetch_realtime_quotes 视图字段一致 (交叉引用)。
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","price":1500.0,"change_pct":2.34,"previous_close":1465.7}]"#,
            "Tdx",
            "tdx",
        );
        let batch = realtime_quotes(&q).unwrap();
        let records = batch.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].code, "600519");
        assert_eq!(records[0].change_percent, 2.34);
        assert_eq!(records[0].provider, ProviderId::Tdx);
        assert_eq!(batch.evidence().source, "tdx");
        assert_eq!(
            batch.evidence().source_at.as_deref(),
            Some("2026-08-15T09:35:00+08:00")
        );
    }

    #[test]
    fn realtime_quotes_rejects_missing_change_pct() {
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","price":1500.0,"previous_close":1465.7}]"#,
            "Tdx",
            "tdx",
        );
        assert!(realtime_quotes(&q).is_err());
    }

    #[test]
    fn empty_records_is_verified_empty() {
        let q = mk_q("[]", "Tdx", "tdx");
        let batch = realtime_quotes(&q).unwrap();
        assert!(batch.is_verified_empty());
    }

    #[test]
    fn missing_evidence_is_fail_closed() {
        // provider 空 → Err, 不静默猜 Tdx。
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","price":1.0,"change_pct":0.0,"previous_close":1.0}]"#,
            "",
            "tdx",
        );
        assert!(realtime_quotes(&q).is_err());
        // source 空 → Err。
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","price":1.0,"change_pct":0.0,"previous_close":1.0}]"#,
            "Tdx",
            "",
        );
        assert!(realtime_quotes(&q).is_err());
    }

    #[test]
    fn order_books_five_levels_padded() {
        // 视图 bids/asks 各 2 档 → 补足 5 档 (空档 = 无挂单)。
        let q = mk_q(
            r#"[{"code":"600519","bids":[{"price":1500.0,"quantity":100.0},{"price":1499.0,"quantity":200.0}],"asks":[{"price":1501.0,"quantity":50.0}],"total_bid_quantity":300.0,"total_ask_quantity":50.0,"source_at":"2026-08-15T09:35:00+08:00"}]"#,
            "Tdx",
            "tdx",
        );
        let batch = order_books(&q).unwrap();
        let records = batch.records();
        assert_eq!(records[0].bids[0].price, 1500.0);
        assert_eq!(records[0].bids[2].price, 0.0, "缺档补 0.0");
        assert_eq!(records[0].bids[2].quantity, 0.0);
        assert_eq!(records[0].asks[0].price, 1501.0);
        assert_eq!(records[0].asks[4].price, 0.0);
    }

    #[test]
    fn security_metadata_board_and_date_parsed() {
        let q = mk_q(
            r#"[{"code":"600519","name":"贵州茅台","board":"Main","is_st":false,"listed_on":"2001-08-27","price_limit_percent":10.0,"source_at":"2026-08-15T09:35:00+08:00"}]"#,
            "Tdx",
            "tdx",
        );
        let batch = security_metadata(&q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.board, SecurityBoard::Main);
        assert_eq!(r.listed_on, NaiveDate::from_ymd_opt(2001, 8, 27).unwrap());
        assert!(!r.is_st);
        // 视图无 price_limit_version → 留空 (消费方按缺证据感知)。
        assert!(r.price_limit_version.is_empty());
    }

    #[test]
    fn historical_bars_filters_to_target_code() {
        let q = mk_q(
            r#"[{"code":"600519","date":"2026-08-14","open":1480.0,"high":1510.0,"low":1475.0,"close":1500.0,"volume":123456.0,"amount":1.85e9,"pct_chg":1.3,"settled":true}]"#,
            "Tdx",
            "tdx",
        );
        let batch = historical_bars("600519", &q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.date, NaiveDate::from_ymd_opt(2026, 8, 14).unwrap());
        assert_eq!(r.close, 1500.0);
        assert!(r.settled);
        assert_eq!(r.adjust, AdjustType::None);
        assert!(r.pe_ratio.is_none(), "视图子集字段 = None (不发明数据)");
    }

    #[test]
    fn historical_bars_rejects_foreign_code() {
        let q = mk_q(
            r#"[{"code":"000001","date":"2026-08-14","open":10.0,"high":11.0,"low":9.0,"close":10.5,"volume":1.0,"amount":2.0,"pct_chg":0.5,"settled":true}]"#,
            "Tdx",
            "tdx",
        );
        assert!(historical_bars("600519", &q).is_err());
    }

    #[test]
    fn minute_data_optional_cumulative_amount() {
        let q = mk_q(
            r#"[{"code":"600519","minute_at":"2026-08-15T09:35:00+08:00","price":1500.0,"cumulative_quantity":100.0,"cumulative_amount":null,"source_at":"2026-08-15T09:35:00+08:00"}]"#,
            "Tdx",
            "tdx",
        );
        let batch = minute_data(&q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.code, "600519");
        assert!(r.cumulative_amount.is_none());
    }

    #[test]
    fn money_flows_all_five_nets() {
        let q = mk_q(
            r#"[{"code":"600519","main_net":1.0,"super_large_net":2.0,"large_net":3.0,"medium_net":4.0,"small_net":5.0,"source_at":"2026-08-15T09:35:00+08:00"}]"#,
            "Tdx",
            "tdx",
        );
        let batch = money_flows(&q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.main_net, 1.0);
        assert_eq!(r.small_net, 5.0);
        assert_eq!(r.provider, ProviderId::Tdx);
    }
}
