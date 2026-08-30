//! P4 M2: gRPC 响应 → 客户端类型化 GatewayBatch 转换。
//! BR-238 要求转换保留已认证 client-bundle 的原始批次证据。
//! BR-236 要求实时行情在 RPC 返回后的 consumer seam 重新执行精确五秒门。
//! 与服务端 delegate.rs fetch_xxx 的 JSON 视图逐字段镜像 (每条转换注明对应
//! fetch 行号, 视图字段名以 delegate 的 json! 键名为准 — 例如 change_pct 对应
//! 结构体字段 change_percent)。
//!
//! 缺字段/缺证据 → GatewayError::invalid_evidence (fail-closed, 绝不静默填充)。
//! 空 records → GatewayBatch::VerifiedEmpty (服务端 proven empty, 不 collapse
//! 成 unavailable)。
use crate::data_gateway::global_news::{
    parse_global_news_observed_at, parse_global_news_provider_time,
    validate_global_news_batch_evidence,
};
use crate::data_gateway::market_capabilities::MarketSecurityIdentity;
use crate::data_gateway::outcome_daily_bars::{OutcomeTransportFailure, RawOutcomeFetch};
use crate::data_gateway::{
    board_ranking::BoardRankingFact, BatchEvidence, BlockTradeReview, BoardDirectoryFact,
    BoardDirectoryRecordEvidence, BoardFlowFact, BoardKind, BoardMembershipRecord,
    DragonTigerSeatReview, DragonTigerSourceDisclosure, DragonTigerStockReview,
    EconomicReleaseFact, EventAnnouncement, ForeignExchangeFact, FuturesDeliveryFact, GatewayBatch,
    GatewayError, GeneralWebResearchBatch, GeneralWebResearchBatchEvidence,
    GeneralWebResearchProvider, GeneralWebResearchRecord, GlobalIndexFact, GlobalNewsProvider,
    GlobalNewsRecord, ImplementedCorporateAction, InstrumentFundFlowFact, IntradayShapeFact,
    MagicTdxT0Batch, MagicTdxT0DailyBar, MagicTdxT0Evidence, MagicTdxT0FiveMinuteBar,
    MagicTdxT0Quote, MagicTdxT0Rejection, MarketBookLevel, MarketMinutePoint, MarketMoneyFlow,
    MarketOrderBook, MarketSecurityMetadata, NorthboundDailyFact, NorthboundQuotaFact,
    NorthboundTopTurnoverFact, ProviderTopNFact, RealtimeIndexQuote, RealtimeMarketQuote,
    ResearchReportFact, ResearchUseScope, SecurityBoard, SinaInstrumentNewsRecord, T0BookLevel,
    UpperLimitRecord,
};
use crate::data_provider::{consensus::ConsensusData, news_item::NewsItem, AdjustType, KlineData};
use crate::grpc_client::envelope::QueryResult;
use crate::market_domain::SecurityBar;
use crate::market_domain::{
    AssetClass, Exchange, InstrumentId, NonEmptyText, ProviderId, SourceEvidence,
};
use crate::market_domain::{
    Bar, CorporateActionCategory, CorporateActionTerms, DataBatch, DragonTigerSide,
    FinancialStatement, FiniteNumber, FlowInterval, FxPair, GlobalIndexCode, IsoDate,
    LimitPoolEntry, LimitPoolKind, MarketRankingKind, MarketRankingUnit, MarketStatistics, Money,
    NorthboundChannel, PositiveU32, Price, Ratio,
};
use crate::selection::schema_v2::OutcomeTransportAttemptPreimage;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::Value;

/// bridge 缺证据时的 capability 标记 (audit_outcome=invalid_evidence)。
const BRIDGE_CAPABILITY: &str = "GrpcBridge";
const MAX_CLOCK_SKEW: chrono::TimeDelta = chrono::TimeDelta::seconds(2);
const LIVE_BATCH_MAX_AGE: chrono::TimeDelta = chrono::TimeDelta::seconds(5);

fn err(capability: &'static str, msg: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(capability, None, msg)
}

fn live_time_error(
    capability: &'static str,
    provider: ProviderId,
    message: impl Into<String>,
) -> GatewayError {
    GatewayError::classified(
        capability,
        Some(provider),
        "unavailable",
        "quote_stale",
        true,
        message,
    )
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
    if q.admission != crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted {
        return Err(err(capability, "响应未获 repository admission"));
    }
    if !q.diagnostic_blocker.is_empty() {
        return Err(err(
            capability,
            "响应携带 diagnostic_blocker, 不得进入生产证据转换",
        ));
    }
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
    if !q.complete {
        return Err(err(
            capability,
            "响应 complete=false, 完整数据转换不得接纳 partial batch",
        ));
    }
    if q.records.len() != 1 {
        return Err(err(
            capability,
            format!(
                "local canonical array contract requires exactly one payload, got {}",
                q.records.len()
            ),
        ));
    }
    let Some(payload) = q.records.first() else {
        return Err(err(capability, "records 空 (服务端无 canonical payload)"));
    };
    serde_json::from_slice(&payload.data)
        .map_err(|e| err(capability, format!("records 非 JSON 数组: {e}")))
}

fn parse_benchmark_wire(
    q: &QueryResult,
) -> Result<crate::data_gateway::benchmark::BenchmarkGrpcResponseWire, GatewayError> {
    const CAPABILITY: &str = "BenchmarkBars";
    let outer_evidence = evidence_of(q, CAPABILITY)?;
    if !q.complete {
        return Err(err(
            CAPABILITY,
            "BenchmarkBars response complete=false; partial batches are terminal",
        ));
    }
    let [payload] = q.records.as_slice() else {
        return Err(err(
            CAPABILITY,
            format!(
                "BenchmarkBars requires exactly one canonical payload, got {}",
                q.records.len()
            ),
        ));
    };
    if payload.schema != "market.benchmark_bars"
        || payload.schema_version != 1
        || payload.content_type != "application/json; charset=utf-8"
    {
        return Err(err(
            CAPABILITY,
            "BenchmarkBars payload schema/version/content_type is not the frozen v1 contract",
        ));
    }
    let wire: crate::data_gateway::benchmark::BenchmarkGrpcResponseWire =
        serde_json::from_slice(&payload.data).map_err(|error| {
            err(
                CAPABILITY,
                format!("BenchmarkBars wire is malformed: {error}"),
            )
        })?;
    if wire.evidence.provider != q.selected_provider
        || wire.evidence.source != outer_evidence.source
        || wire.evidence.source_at != outer_evidence.source_at
        || wire.evidence.observed_at != outer_evidence.observed_at
        || wire.evidence.batch_id != outer_evidence.batch_id
    {
        return Err(err(
            CAPABILITY,
            "BenchmarkBars inner evidence conflicts with the authenticated response envelope",
        ));
    }
    Ok(wire)
}

#[derive(Debug)]
pub(crate) struct BenchmarkGrpcConversionFailure {
    error: GatewayError,
    verified_receipt:
        Option<Box<crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt>>,
}

impl BenchmarkGrpcConversionFailure {
    fn unverified(error: GatewayError) -> Self {
        Self {
            error,
            verified_receipt: None,
        }
    }

    fn consumer_admission(
        error: GatewayError,
        receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt,
    ) -> Self {
        Self {
            error,
            verified_receipt: Some(Box::new(receipt)),
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        GatewayError,
        Option<crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt>,
    ) {
        (self.error, self.verified_receipt.map(|receipt| *receipt))
    }
}

pub(crate) fn benchmark_bars(
    request: &crate::data_gateway::BenchmarkRequest,
    q: &QueryResult,
) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, BenchmarkGrpcConversionFailure> {
    let wire = parse_benchmark_wire(q).map_err(BenchmarkGrpcConversionFailure::unverified)?;
    let receipt =
        crate::data_gateway::benchmark::verify_benchmark_grpc_server_receipt(request, &wire)
            .map_err(BenchmarkGrpcConversionFailure::unverified)?;
    crate::data_gateway::benchmark::admit_benchmark_grpc_wire(request, wire)
        .map_err(|error| BenchmarkGrpcConversionFailure::consumer_admission(error, receipt))
}

/// Decode and re-admit a gRPC response for a caller-owned audit database.
///
/// The remote receipt is checked only as wire evidence. The review gateway
/// replaces it with a new BR-159 receipt from the caller's explicit database
/// before exposing a successful batch to `BenchmarkCapture`.
pub(crate) fn benchmark_bars_for_local_readmission(
    request: &crate::data_gateway::BenchmarkRequest,
    q: &QueryResult,
) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, BenchmarkGrpcConversionFailure> {
    let wire = parse_benchmark_wire(q).map_err(BenchmarkGrpcConversionFailure::unverified)?;
    let receipt =
        crate::data_gateway::benchmark::decode_benchmark_grpc_server_receipt_for_local_readmission(
            request, &wire,
        )
        .map_err(BenchmarkGrpcConversionFailure::unverified)?;
    crate::data_gateway::benchmark::admit_benchmark_grpc_wire(request, wire)
        .map_err(|error| BenchmarkGrpcConversionFailure::consumer_admission(error, receipt))
}

#[cfg(test)]
pub(crate) fn benchmark_bars_for_test(
    request: &crate::data_gateway::BenchmarkRequest,
    q: &QueryResult,
    coverage: crate::data_gateway::BenchmarkAdmissionCoverage<'_>,
    consumer_now: DateTime<Utc>,
) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, GatewayError> {
    let wire = parse_benchmark_wire(q)?;
    crate::data_gateway::benchmark::admit_benchmark_grpc_wire_for_test(
        request,
        wire,
        coverage,
        consumer_now,
    )
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

fn as_positive_finite_f64(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<f64, GatewayError> {
    let value = as_f64(v, key, capability)?;
    if !value.is_finite() || value <= 0.0 {
        return Err(err(
            capability,
            format!("record 字段 {key} 必须为正有限数，实际为 {value}"),
        ));
    }
    Ok(value)
}

fn as_non_negative_finite_f64(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<f64, GatewayError> {
    let value = as_f64(v, key, capability)?;
    if !value.is_finite() || value < 0.0 {
        return Err(err(
            capability,
            format!("record 字段 {key} 必须为非负有限数，实际为 {value}"),
        ));
    }
    Ok(value)
}

fn as_bool(v: &Value, key: &str, capability: &'static str) -> Result<bool, GatewayError> {
    v.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| err(capability, format!("record 缺布尔字段 {key}")))
}

fn as_rfc3339(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<DateTime<Utc>, GatewayError> {
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
fn record_source_at(
    q: &QueryResult,
    capability: &'static str,
) -> Result<DateTime<Utc>, GatewayError> {
    if q.source_at.is_empty() {
        return Err(err(capability, "source_at 空 (服务端未回填证据链)"));
    }
    DateTime::parse_from_rfc3339(&q.source_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            err(
                capability,
                format!("source_at 非 RFC3339: {} ({e})", q.source_at),
            )
        })
}

fn record_observed_at(
    q: &QueryResult,
    capability: &'static str,
) -> Result<DateTime<Utc>, GatewayError> {
    crate::data_gateway::parse_evidence_instant(
        capability,
        parse_provider(&q.selected_provider)?,
        "observed_at",
        &q.observed_at,
    )
}

fn live_evidence_times(
    q: &QueryResult,
    capability: &'static str,
    now: DateTime<Utc>,
) -> Result<(BatchEvidence, DateTime<Utc>, DateTime<Utc>), GatewayError> {
    let (evidence, source_at, observed_at, _) = live_evidence_times_lenient(q, capability, now)?;
    // 非 T0 能力保持严格 age 门: lenient 变体仅对 age 放宽, 未来/倒挂仍硬错,
    // 因此这里 _ 恒 false → 直接沿用严格语义。
    validate_live_times(
        capability,
        evidence.provider,
        source_at,
        observed_at,
        now,
        "batch",
    )?;
    Ok((evidence, source_at, observed_at))
}

/// 2026-08-21 用户指令「拿不到时间先跳过, 标注时间不可信」: T0Evidence 专用
/// 宽松时间门。生产根因 (BR-243 follow-up): 桥往返 + server 侧整批组装
/// (7 codes × quote+daily+5min+minute-time 顺序 TDX 调用) 实测 11-55s,
/// 消费者按自己 now 复查 age 必然超 5s → 做T 自 P4 桥迁移 (08-16) 起全拒。
///
/// 放宽范围**仅 age 门** (source_at 比 now 老): 置 time_untrustworthy=true 由
/// 推送方标注「时间不可信」。未来时间 / source_at 晚于 observed_at / source_at
/// 缺失仍是硬错误 (损坏 ≠ 延迟, 绝不静默放行)。
fn live_evidence_times_lenient(
    q: &QueryResult,
    capability: &'static str,
    now: DateTime<Utc>,
) -> Result<(BatchEvidence, DateTime<Utc>, DateTime<Utc>, bool), GatewayError> {
    let evidence = evidence_of(q, capability)?;
    let source_at_raw = evidence
        .source_at
        .as_deref()
        .ok_or_else(|| err(capability, "source_at 空，实时批次不得进入 consumer"))?;
    let source_at = crate::data_gateway::parse_evidence_instant(
        capability,
        evidence.provider,
        "source_at",
        source_at_raw,
    )?;
    let observed_at = crate::data_gateway::parse_evidence_instant(
        capability,
        evidence.provider,
        "observed_at",
        &evidence.observed_at,
    )?;
    let mut time_untrustworthy = false;
    if source_at > now {
        return Err(live_time_error(
            capability,
            evidence.provider,
            format!("{capability}.source_at 晚于 consumer now: source_at={source_at} now={now}"),
        ));
    }
    if observed_at > now {
        return Err(live_time_error(
            capability,
            evidence.provider,
            format!(
                "{capability}.observed_at 晚于 consumer now: observed_at={observed_at} now={now}"
            ),
        ));
    }
    if source_at > observed_at {
        return Err(err(
            capability,
            format!(
                "{capability}.source_at 晚于 observed_at: source_at={source_at} observed_at={observed_at}"
            ),
        ));
    }
    let age = now.signed_duration_since(source_at);
    if age > LIVE_BATCH_MAX_AGE {
        // 2026-08-21 用户指令: 拿不到可信时间 → 先跳过 age 门, 标注时间不可信。
        time_untrustworthy = true;
        log::warn!(
            "[{capability}][时间不可信] batch age_ns={} max_ns={} source_at={source_at} \
             observed_at={observed_at} now={now} (桥组装延迟, 放宽 age 门, 推送须标注)",
            age.num_nanoseconds().unwrap_or(i64::MAX),
            LIVE_BATCH_MAX_AGE.num_nanoseconds().unwrap_or(i64::MAX),
        );
    }
    Ok((evidence, source_at, observed_at, time_untrustworthy))
}

fn validate_live_times(
    capability: &'static str,
    provider: ProviderId,
    source_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
    now: DateTime<Utc>,
    scope: &str,
) -> Result<(), GatewayError> {
    if source_at > now {
        return Err(live_time_error(
            capability,
            provider,
            format!("{scope}.source_at 晚于 consumer now: source_at={source_at} now={now}"),
        ));
    }
    if observed_at > now {
        return Err(live_time_error(
            capability,
            provider,
            format!("{scope}.observed_at 晚于 consumer now: observed_at={observed_at} now={now}"),
        ));
    }
    if source_at > observed_at {
        return Err(err(
            capability,
            format!(
                "{scope}.source_at 晚于 observed_at: source_at={source_at} observed_at={observed_at}"
            ),
        ));
    }
    let age = now.signed_duration_since(source_at);
    if age > LIVE_BATCH_MAX_AGE {
        return Err(live_time_error(
            capability,
            provider,
            format!(
                "consumer 收到过期实时 {scope}: age_ns={} max_ns={}",
                age.num_nanoseconds().unwrap_or(i64::MAX),
                LIVE_BATCH_MAX_AGE.num_nanoseconds().unwrap_or(i64::MAX)
            ),
        ));
    }
    Ok(())
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
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// RPC 完成后的 consumer-side 实时行情门。
///
/// `now` 是 consumer 实际收到并转换响应的时刻；provider `source_at` 到该时刻
/// 必须精确处于 `0..=5s`，且 `source_at <= observed_at <= now`。
pub fn realtime_quotes_at(
    q: &QueryResult,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<RealtimeMarketQuote>, GatewayError> {
    let capability = "RealtimeMarketQuotes";
    let (evidence, source_at, observed_at) = live_evidence_times(q, capability, now)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }
    let records = parsed
        .iter()
        .map(|value| {
            Ok(RealtimeMarketQuote {
                code: as_str(value, "code", capability)?,
                name: as_str(value, "name", capability)?,
                price: as_positive_finite_f64(value, "price", capability)?,
                change_percent: as_f64(value, "change_pct", capability)?,
                previous_close: as_positive_finite_f64(value, "previous_close", capability)?,
                source_at,
                observed_at,
                provider: evidence.provider,
                batch_id: evidence.batch_id.clone(),
            })
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    Ok(GatewayBatch::Available { records, evidence })
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
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
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
    let mut levels = [MarketBookLevel {
        price: 0.0,
        quantity: 0.0,
    }; 5];
    for (i, item) in arr.iter().enumerate() {
        levels[i] = MarketBookLevel {
            price: as_f64(item, "price", capability)?,
            quantity: as_f64(item, "quantity", capability)?,
        };
    }
    Ok(levels)
}

fn live_book_levels(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<[MarketBookLevel; 5], GatewayError> {
    let arr = v
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| err(capability, format!("record 缺数组字段 {key}")))?;
    if arr.len() != 5 {
        return Err(err(
            capability,
            format!(
                "live {key} 档位数必须精确为 5，实际为 {}；禁止以零值补缺档",
                arr.len()
            ),
        ));
    }
    let levels = book_levels(v, key, capability)?;
    for (index, item) in arr.iter().enumerate() {
        as_positive_finite_f64(item, "price", capability).map_err(|error| {
            err(
                capability,
                format!("{key}[{index}] price 非正有限数: {error}"),
            )
        })?;
        as_non_negative_finite_f64(item, "quantity", capability).map_err(|error| {
            err(
                capability,
                format!("{key}[{index}] quantity 非非负有限数: {error}"),
            )
        })?;
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
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// RPC 完成后的 consumer-side 五档盘口门。
pub fn order_books_at(
    q: &QueryResult,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<MarketOrderBook>, GatewayError> {
    let capability = "MarketOrderBooks";
    let (evidence, source_at, observed_at) = live_evidence_times(q, capability, now)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }
    let records = parsed
        .iter()
        .map(|value| {
            let record_source_at_raw = as_str(value, "source_at", capability)?;
            let record_source_at = crate::data_gateway::parse_evidence_instant(
                capability,
                evidence.provider,
                "record.source_at",
                &record_source_at_raw,
            )?;
            if record_source_at != source_at {
                return Err(err(
                    capability,
                    format!(
                        "record.source_at 与批次不一致: record={record_source_at} batch={source_at}"
                    ),
                ));
            }
            Ok(MarketOrderBook {
                code: as_str(value, "code", capability)?,
                bids: live_book_levels(value, "bids", capability)?,
                asks: live_book_levels(value, "asks", capability)?,
                total_bid_quantity: as_non_negative_finite_f64(
                    value,
                    "total_bid_quantity",
                    capability,
                )?,
                total_ask_quantity: as_non_negative_finite_f64(
                    value,
                    "total_ask_quantity",
                    capability,
                )?,
                source_at,
                observed_at,
                provider: evidence.provider,
                batch_id: evidence.batch_id.clone(),
            })
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    Ok(GatewayBatch::Available { records, evidence })
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
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// 证券元数据。视图: delegate.rs fetch_security_metadata (:282-291)
/// {"code","name","board"(Debug),"is_st","listed_on","price_limit_percent","source_at"}。
pub fn security_metadata(
    q: &QueryResult,
) -> Result<GatewayBatch<MarketSecurityMetadata>, GatewayError> {
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
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

const LOCAL_SECURITY_METADATA_SCHEMA: &str = "market.security_metadata";
const EXTERNAL_SECURITY_METADATA_SCHEMA: &str = "magic.market.security_metadata";
const CANONICAL_JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";

/// Security identity is the only SecurityMetadata projection allowed to
/// consume an admitted partial ExternalV1 response. It retains the immutable
/// record/envelope evidence and does not inspect or synthesize listing/limit
/// fields that the provider did not prove.
fn parse_security_identities(
    q: &QueryResult,
) -> Result<GatewayBatch<MarketSecurityIdentity>, GatewayError> {
    let capability = "SecurityIdentity";
    let ev = evidence_of(q, capability)?;
    let observed_at = crate::data_gateway::parse_evidence_instant(
        capability,
        ev.provider,
        "observed_at",
        &q.observed_at,
    )?;
    if q.source_at.is_empty() {
        return Err(err(capability, "source_at 空, 不得以 observed_at 补值"));
    }
    let source_at = crate::data_gateway::parse_evidence_instant(
        capability,
        ev.provider,
        "source_at",
        &q.source_at,
    )?;
    if source_at > observed_at {
        return Err(err(capability, "source_at 晚于 observed_at"));
    }

    let Some(first) = q.records.first() else {
        return Err(err(
            capability,
            "records 空 (无 canonical identity payload)",
        ));
    };
    let records = match first.schema.as_str() {
        LOCAL_SECURITY_METADATA_SCHEMA => {
            if !q.complete {
                return Err(err(
                    capability,
                    "local security metadata complete=false, 不得接纳 partial array",
                ));
            }
            if q.records.len() != 1 {
                return Err(err(
                    capability,
                    "local security metadata 必须是单 payload JSON 数组",
                ));
            }
            validate_identity_payload(first, LOCAL_SECURITY_METADATA_SCHEMA, capability)?;
            let values: Vec<Value> = serde_json::from_slice(&first.data).map_err(|e| {
                err(
                    capability,
                    format!("local identity payload 非 JSON 数组: {e}"),
                )
            })?;
            values
                .iter()
                .map(|record| {
                    let record_source_at =
                        required_evidence_time(record, "source_at", capability, ev.provider)?;
                    if record_source_at != source_at {
                        return Err(err(capability, "local record source_at 与 envelope 冲突"));
                    }
                    Ok(MarketSecurityIdentity {
                        code: non_empty_identity_text(
                            as_str(record, "code", capability)?,
                            "code",
                            capability,
                        )?,
                        name: non_empty_identity_text(
                            as_str(record, "name", capability)?,
                            "name",
                            capability,
                        )?,
                        is_st: as_bool(record, "is_st", capability)?,
                        source_at,
                        observed_at,
                        provider: ev.provider,
                        batch_id: ev.batch_id.clone(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        EXTERNAL_SECURITY_METADATA_SCHEMA => q
            .records
            .iter()
            .map(|payload| {
                validate_identity_payload(payload, EXTERNAL_SECURITY_METADATA_SCHEMA, capability)?;
                let record: Value = serde_json::from_slice(&payload.data).map_err(|e| {
                    err(
                        capability,
                        format!("external identity payload 非 JSON object: {e}"),
                    )
                })?;
                if !record.is_object() {
                    return Err(err(
                        capability,
                        "external identity payload 必须一 payload 一 object",
                    ));
                }
                let instrument: InstrumentId = serde_json::from_value(
                    record
                        .get("instrument")
                        .cloned()
                        .ok_or_else(|| err(capability, "record 缺 instrument"))?,
                )
                .map_err(|e| err(capability, format!("instrument 无效: {e}")))?;
                if instrument.asset_class() != AssetClass::Equity {
                    return Err(err(capability, "security identity 非 Equity"));
                }
                let raw_code = record
                    .get("instrument")
                    .and_then(|value| value.get("code"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| err(capability, "instrument 缺 code"))?;
                if raw_code != instrument.code() {
                    return Err(err(capability, "instrument code 非 canonical 原值"));
                }

                let record_provider = parse_provider(&as_str(&record, "provider", capability)?)?;
                if record_provider != ev.provider {
                    return Err(err(capability, "record provider 与 envelope 冲突"));
                }
                if as_str(&record, "batch_id", capability)? != ev.batch_id {
                    return Err(err(capability, "record batch_id 与 envelope 冲突"));
                }
                let record_observed_at =
                    required_evidence_time(&record, "observed_at", capability, ev.provider)?;
                if record_observed_at != observed_at {
                    return Err(err(capability, "record observed_at 与 envelope 冲突"));
                }
                let record_source_at =
                    required_evidence_time(&record, "source_at", capability, ev.provider)?;
                if record_source_at < source_at {
                    return Err(err(
                        capability,
                        "record source_at 早于 envelope oldest source_at",
                    ));
                }
                if record_source_at > record_observed_at {
                    return Err(err(capability, "record source_at 晚于 record observed_at"));
                }
                match as_str(&record, "status", capability)?.as_str() {
                    "Available" | "Unavailable" => {}
                    _ => return Err(err(capability, "record status 不可用于 identity 投影")),
                }

                Ok(MarketSecurityIdentity {
                    code: non_empty_identity_text(
                        instrument.code().to_string(),
                        "code",
                        capability,
                    )?,
                    name: non_empty_identity_text(
                        as_str(&record, "name", capability)?,
                        "name",
                        capability,
                    )?,
                    is_st: as_bool(&record, "is_st", capability)?,
                    source_at: record_source_at,
                    observed_at: record_observed_at,
                    provider: record_provider,
                    batch_id: ev.batch_id.clone(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(err(capability, "未知 SecurityMetadata schema")),
    };

    if records.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    if first.schema == EXTERNAL_SECURITY_METADATA_SCHEMA {
        let oldest_record_source = records
            .iter()
            .map(|record| record.source_at)
            .min()
            .ok_or_else(|| err(capability, "external identity records 空"))?;
        if oldest_record_source != source_at {
            return Err(err(
                capability,
                "envelope source_at 不是 record source_at 最小值",
            ));
        }
    }
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// Binds a SecurityMetadata identity projection to the exact request that
/// authorized it. ExternalV1 does not promise response order, so a complete
/// exact set is returned in request order without changing any record evidence.
pub fn security_identities(
    requested_codes: &[String],
    q: &QueryResult,
    now: DateTime<Utc>,
) -> Result<GatewayBatch<MarketSecurityIdentity>, GatewayError> {
    let capability = "SecurityIdentity";
    if !(1..=50).contains(&requested_codes.len()) {
        return Err(err(
            capability,
            "security identity request size must be within 1..=50",
        ));
    }
    let mut requested = std::collections::HashSet::with_capacity(requested_codes.len());
    if requested_codes
        .iter()
        .any(|code| !requested.insert(code.as_str()))
    {
        return Err(err(
            capability,
            "security identity request contains duplicate codes",
        ));
    }

    let batch = parse_security_identities(q)?;
    let (records, evidence) = match batch {
        GatewayBatch::Available { records, evidence } => (records, evidence),
        GatewayBatch::VerifiedEmpty(_) => {
            return Err(err(
                capability,
                "security identity response is empty for a non-empty request",
            ));
        }
    };
    validate_identity_observed_freshness(&evidence, now)?;
    validate_identity_source_freshness(&evidence, now)?;
    let mut by_code = std::collections::HashMap::with_capacity(records.len());
    for record in records {
        let code = record.code.clone();
        if by_code.insert(code, record).is_some() {
            return Err(err(
                capability,
                "security identity response contains duplicate codes",
            ));
        }
    }
    let mut ordered = Vec::with_capacity(requested_codes.len());
    for code in requested_codes {
        let record = by_code.remove(code).ok_or_else(|| {
            err(
                capability,
                format!("security identity response is missing requested code {code:?}"),
            )
        })?;
        ordered.push(record);
    }
    if !by_code.is_empty() {
        return Err(err(
            capability,
            "security identity response contains unrequested codes",
        ));
    }
    Ok(GatewayBatch::Available {
        records: ordered,
        evidence,
    })
}

fn validate_identity_observed_freshness(
    evidence: &BatchEvidence,
    now: DateTime<Utc>,
) -> Result<(), GatewayError> {
    const MAX_AGE: chrono::TimeDelta = chrono::TimeDelta::seconds(30);
    let observed_at = crate::data_gateway::parse_evidence_instant(
        "SecurityIdentity",
        evidence.provider,
        "observed_at",
        &evidence.observed_at,
    )?;
    let age = now.signed_duration_since(observed_at);
    if age < -MAX_CLOCK_SKEW || age > MAX_AGE {
        let age_millis = age.num_milliseconds();
        return Err(GatewayError::classified(
            "SecurityIdentity",
            Some(evidence.provider),
            "stale",
            "observation_stale",
            true,
            format!(
                "security identity observation failed freshness gate age_ms={age_millis} max_age_ms={} max_clock_skew_ms={}",
                MAX_AGE.num_milliseconds(),
                MAX_CLOCK_SKEW.num_milliseconds()
            ),
        ));
    }
    Ok(())
}

fn validate_identity_source_freshness(
    evidence: &BatchEvidence,
    now: DateTime<Utc>,
) -> Result<(), GatewayError> {
    let source_at = evidence.source_at.as_deref().ok_or_else(|| {
        err(
            "SecurityIdentity",
            "security identity batch has no source timestamp",
        )
    })?;
    let source_at = crate::data_gateway::parse_evidence_instant(
        "SecurityIdentity",
        evidence.provider,
        "source_at",
        source_at,
    )?;
    if source_at > now + MAX_CLOCK_SKEW {
        return Err(GatewayError::invalid_evidence(
            "SecurityIdentity",
            Some(evidence.provider),
            "security identity source timestamp exceeds maximum clock skew",
        ));
    }

    let shanghai = chrono::FixedOffset::east_opt(8 * 60 * 60)
        .expect("Shanghai UTC offset is a compile-time valid constant");
    let source_date = source_at.with_timezone(&shanghai).date_naive();
    let today = now.with_timezone(&shanghai).date_naive();
    let oldest_allowed = crate::calendar::prev_trading_day(today);
    if source_date < oldest_allowed {
        return Err(GatewayError::classified(
            "SecurityIdentity",
            Some(evidence.provider),
            "stale",
            "daily_source_stale",
            true,
            format!(
                "security identity source date {source_date} is older than one trading day; oldest_allowed={oldest_allowed}"
            ),
        ));
    }
    Ok(())
}

fn validate_identity_payload(
    payload: &crate::grpc_client::pb::magic::market::v1::CanonicalPayload,
    schema: &str,
    capability: &'static str,
) -> Result<(), GatewayError> {
    if payload.schema != schema
        || payload.schema_version != 1
        || payload.content_type != CANONICAL_JSON_CONTENT_TYPE
    {
        return Err(err(
            capability,
            "SecurityMetadata schema/version/content-type 冲突",
        ));
    }
    Ok(())
}

fn required_evidence_time(
    record: &Value,
    field: &'static str,
    capability: &'static str,
    provider: ProviderId,
) -> Result<DateTime<Utc>, GatewayError> {
    let value = record
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| err(capability, format!("record 缺 {field}, 不得补值")))?;
    crate::data_gateway::parse_evidence_instant(capability, provider, field, value)
}

fn non_empty_identity_text(
    value: String,
    field: &'static str,
    capability: &'static str,
) -> Result<String, GatewayError> {
    if value.trim().is_empty() {
        return Err(err(capability, format!("identity {field} 空")));
    }
    Ok(value)
}

/// 日线 K 线。视图: delegate.rs fetch_historical_bars (:538-550)
/// {"code","date","open","high","low","close","volume","amount","pct_chg","settled"}。
/// 视图只含 KlineData 的 10 个字段子集 → 其余 Option 字段 = None、bool = false、
/// adjust = None (视图冻结, 消费者需要的字段由 M3+ 扩展服务端视图, 不在客户端
/// 发明数据)。
pub fn historical_bars(
    code: &str,
    q: &QueryResult,
) -> Result<GatewayBatch<KlineData>, GatewayError> {
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
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

// ---------- M3 批次 1: 全球市场/日历/公告/新闻/交割 ----------

/// 全球指数。视图: delegate.rs fetch_global_indices (:339-361)
/// {"code"(Debug),"name","value","change","change_percent","source_at"}。
pub fn global_indices(q: &QueryResult) -> Result<GatewayBatch<GlobalIndexFact>, GatewayError> {
    let capability = "GlobalIndices";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(GlobalIndexFact {
                code: parse_global_index_code(&as_str(v, "code", capability)?)?,
                name: as_str(v, "name", capability)?,
                value: as_f64(v, "value", capability)?,
                change: as_f64(v, "change", capability)?,
                change_percent: as_f64(v, "change_percent", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// 外汇。视图: delegate.rs fetch_foreign_exchange (:900-921)
/// {"pair"(Debug),"name","rate","change","change_percent","source_at"}。
/// change/change_percent 是可空数值 (JSON null → None, 不补零)。
pub fn foreign_exchange(
    q: &QueryResult,
) -> Result<GatewayBatch<ForeignExchangeFact>, GatewayError> {
    let capability = "ForeignExchange";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(ForeignExchangeFact {
                pair: parse_fx_pair(&as_str(v, "pair", capability)?)?,
                name: as_str(v, "name", capability)?,
                rate: as_f64(v, "rate", capability)?,
                change: as_optional_f64(v, "change", capability)?,
                change_percent: as_optional_f64(v, "change_percent", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// 公告。视图: delegate.rs fetch_announcements (:363-385)
/// {"announcement_id","code","category","title","published_at","url"}。
/// category 可空 (JSON null → None)。
pub fn announcements(q: &QueryResult) -> Result<GatewayBatch<EventAnnouncement>, GatewayError> {
    let capability = "Announcements";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(EventAnnouncement {
                announcement_id: as_str(v, "announcement_id", capability)?,
                code: as_str(v, "code", capability)?,
                category: as_optional_str(v, "category", capability)?,
                title: as_str(v, "title", capability)?,
                published_at: as_str(v, "published_at", capability)?,
                canonical_url: as_str(v, "url", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// 全球新闻。视图: delegate.rs fetch_global_news (:387-411)
/// {"item_id","title","summary","publisher","url","published_at",
///  "instruments","topics","content","language"}。
pub fn global_news(q: &QueryResult) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
    let capability = "GlobalNews";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(GlobalNewsRecord {
                item_id: as_str(v, "item_id", capability)?,
                title: as_str(v, "title", capability)?,
                summary: as_optional_str(v, "summary", capability)?,
                content: as_optional_str(v, "content", capability)?,
                publisher: as_str(v, "publisher", capability)?,
                canonical_url: as_str(v, "url", capability)?,
                published_at: as_rfc3339(v, "published_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                instruments: as_str_array(v, "instruments", capability)?,
                topics: as_str_array(v, "topics", capability)?,
                language: as_str(v, "language", capability)?,
                evidence: record_evidence(&ev, q)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// ExternalV1 GlobalNews v2 uses one canonical payload per record. This is a
/// separate contract from LocalBridgeV1's single JSON-array payload above.
pub fn external_global_news(
    provider: GlobalNewsProvider,
    q: &QueryResult,
) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
    let capability = "GlobalNews";
    let transport_evidence = evidence_of(q, capability)?;
    if !q.complete {
        return Err(err(capability, "ExternalV1 GlobalNews complete=false"));
    }
    if transport_evidence.provider != provider.provider_id() {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider.provider_id()),
            "ExternalV1 GlobalNews selected provider differs from exact request",
        ));
    }
    if !transport_evidence.source.starts_with("grpc-mtls:") {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider.provider_id()),
            "ExternalV1 GlobalNews acquisition authority is missing",
        ));
    }

    let evidence = BatchEvidence {
        provider: provider.provider_id(),
        source: provider.source().to_owned(),
        source_at: transport_evidence.source_at,
        observed_at: transport_evidence.observed_at,
        batch_id: transport_evidence.batch_id,
    };
    if q.records.is_empty() {
        parse_global_news_observed_at(provider, &evidence.observed_at)?;
        if let Some(source_at) = evidence.source_at.as_deref() {
            parse_global_news_provider_time(provider, source_at)?;
        }
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }

    let (batch_source_at, batch_observed_at) =
        validate_global_news_batch_evidence(provider, &evidence)?;
    let mut item_ids = std::collections::HashSet::with_capacity(q.records.len());
    let mut urls = std::collections::HashSet::with_capacity(q.records.len());
    let mut previous_published_at: Option<DateTime<Utc>> = None;
    let mut records = Vec::with_capacity(q.records.len());
    for payload in &q.records {
        validate_external_news_payload(payload, capability)?;
        let value: Value = serde_json::from_slice(&payload.data).map_err(|error| {
            err(
                capability,
                format!("GlobalNews v2 record is not JSON: {error}"),
            )
        })?;
        validate_external_news_fields(&value, capability)?;
        let wire: ExternalNewsItemWire = serde_json::from_value(value).map_err(|error| {
            err(
                capability,
                format!("GlobalNews v2 fields are invalid: {error}"),
            )
        })?;

        validate_external_news_text(&wire.item_id, "item_id", capability)?;
        validate_external_news_text(&wire.title, "title", capability)?;
        validate_external_news_text(&wire.publisher, "publisher", capability)?;
        validate_external_news_text(&wire.url, "url", capability)?;
        validate_external_news_text(&wire.published_at, "published_at", capability)?;
        validate_external_news_text(&wire.language, "language", capability)?;
        if wire.evidence.provider() != evidence.provider
            || wire.evidence.batch_id() != evidence.batch_id
            || wire.evidence.observed_at() != evidence.observed_at
        {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider.provider_id()),
                "GlobalNews v2 record evidence differs from its envelope",
            ));
        }
        let record_source_at = wire.evidence.source_at().ok_or_else(|| {
            GatewayError::invalid_evidence(
                capability,
                Some(provider.provider_id()),
                "GlobalNews v2 record source_at is missing",
            )
        })?;
        let parsed_source_at = parse_global_news_provider_time(provider, record_source_at)?;
        let published_at = DateTime::parse_from_rfc3339(&wire.published_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|_| {
                GatewayError::invalid_evidence(
                    capability,
                    Some(provider.provider_id()),
                    "GlobalNews v2 published_at is not RFC3339",
                )
            })?;
        let observed_at = parse_global_news_observed_at(provider, wire.evidence.observed_at())?;
        if parsed_source_at != published_at
            || published_at > observed_at
            || observed_at > batch_observed_at
        {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider.provider_id()),
                "GlobalNews v2 record time evidence conflicts",
            ));
        }
        if previous_published_at.is_some_and(|previous| previous < published_at) {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider.provider_id()),
                "GlobalNews v2 records are not ordered newest first",
            ));
        }
        previous_published_at = Some(published_at);
        if !item_ids.insert(wire.item_id.clone()) || !urls.insert(wire.url.clone()) {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider.provider_id()),
                "GlobalNews v2 contains duplicate record identity",
            ));
        }
        let parsed_url = url::Url::parse(&wire.url).map_err(|_| {
            GatewayError::invalid_evidence(
                capability,
                Some(provider.provider_id()),
                "GlobalNews v2 url is invalid",
            )
        })?;
        if parsed_url.scheme() != "https" || parsed_url.host_str().is_none() {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(provider.provider_id()),
                "GlobalNews v2 url must be absolute HTTPS",
            ));
        }
        records.push(GlobalNewsRecord {
            item_id: wire.item_id,
            title: wire.title,
            summary: wire.summary,
            content: wire.content,
            publisher: wire.publisher,
            canonical_url: wire.url,
            published_at,
            observed_at,
            instruments: wire
                .instruments
                .into_iter()
                .map(|instrument| instrument.code().to_owned())
                .collect(),
            topics: wire.topics,
            language: wire.language,
            evidence: wire.evidence,
        });
    }
    if records
        .first()
        .is_none_or(|record| record.published_at != batch_source_at)
    {
        return Err(GatewayError::invalid_evidence(
            capability,
            Some(provider.provider_id()),
            "GlobalNews v2 batch source_at differs from newest record",
        ));
    }
    Ok(GatewayBatch::Available { records, evidence })
}

/// 财经日历。视图: delegate.rs fetch_economic_calendar (:413-439)
/// {"event_id","country","name","period","scheduled_at","previous","consensus",
///  "actual","unit","importance","released_at","revised","impact","indicator_id"}。
pub fn economic_calendar(
    q: &QueryResult,
) -> Result<GatewayBatch<EconomicReleaseFact>, GatewayError> {
    let capability = "EconomicCalendar";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(EconomicReleaseFact {
                event_id: as_str(v, "event_id", capability)?,
                indicator_id: as_u64(v, "indicator_id", capability)? as u32,
                country: as_str(v, "country", capability)?,
                name: as_str(v, "name", capability)?,
                period: as_optional_str(v, "period", capability)?,
                scheduled_at: as_rfc3339(v, "scheduled_at", capability)?,
                released_at: as_rfc3339(v, "released_at", capability)?,
                previous: as_optional_str(v, "previous", capability)?,
                consensus: as_optional_str(v, "consensus", capability)?,
                actual: as_optional_str(v, "actual", capability)?,
                revised: as_optional_str(v, "revised", capability)?,
                unit: as_optional_str(v, "unit", capability)?,
                importance: as_u64(v, "importance", capability)? as u32,
                impact: as_optional_str(v, "impact", capability)?,
                evidence: record_evidence(&ev, q)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// 交割日历。视图: delegate.rs fetch_futures_delivery (:441-463)
/// {"contract_code","product_code","last_trading_date","delivery_date","notice_url"}。
/// last_trading_date 可空 (JSON null → None)。
pub fn futures_delivery(
    q: &QueryResult,
) -> Result<GatewayBatch<FuturesDeliveryFact>, GatewayError> {
    let capability = "FuturesDelivery";
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records = parsed
        .iter()
        .map(|v| {
            Ok(FuturesDeliveryFact {
                contract_code: as_str(v, "contract_code", capability)?,
                product_code: as_str(v, "product_code", capability)?,
                last_trading_date: as_optional_date(v, "last_trading_date", capability)?,
                delivery_date: as_date(v, "delivery_date", capability)?,
                notice_url: as_str(v, "notice_url", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// Debug 名 → GlobalIndexCode (服务端 format!("{:?}", code))。未知 → Err。
fn parse_global_index_code(s: &str) -> Result<GlobalIndexCode, GatewayError> {
    Ok(match s {
        "DowJones" => GlobalIndexCode::DowJones,
        "NasdaqComposite" => GlobalIndexCode::NasdaqComposite,
        "Sp500" => GlobalIndexCode::Sp500,
        "Nikkei225" => GlobalIndexCode::Nikkei225,
        "HangSeng" => GlobalIndexCode::HangSeng,
        "Ftse100" => GlobalIndexCode::Ftse100,
        _ => return Err(err("GlobalIndices", format!("未知 GlobalIndexCode: {s}"))),
    })
}

/// Debug 名 → FxPair。未知 → Err。
fn parse_fx_pair(s: &str) -> Result<FxPair, GatewayError> {
    Ok(match s {
        "UsdCny" => FxPair::UsdCny,
        "EurUsd" => FxPair::EurUsd,
        "UsdJpy" => FxPair::UsdJpy,
        "GbpUsd" => FxPair::GbpUsd,
        "AudUsd" => FxPair::AudUsd,
        "UsdChf" => FxPair::UsdChf,
        "UsdCad" => FxPair::UsdCad,
        "NzdUsd" => FxPair::NzdUsd,
        _ => return Err(err("ForeignExchange", format!("未知 FxPair: {s}"))),
    })
}

/// record 级证据: 用批级 evidence 构造 (视图无逐条证据字段)。
fn record_evidence(ev: &BatchEvidence, q: &QueryResult) -> Result<SourceEvidence, GatewayError> {
    let mut evidence =
        SourceEvidence::new(ev.provider, ev.observed_at.clone(), ev.batch_id.clone())
            .map_err(|e| err(BRIDGE_CAPABILITY, format!("record evidence 构造失败: {e}")))?;
    if let Some(source_at) = &ev.source_at {
        evidence = evidence.with_source_at(source_at.clone()).map_err(|e| {
            err(
                BRIDGE_CAPABILITY,
                format!("record evidence source_at 失败: {e}"),
            )
        })?;
    }
    let _ = q;
    Ok(evidence)
}

/// 视图可空数值字段 (JSON null → None; 缺失 → Err fail-closed)。
fn as_optional_f64(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<Option<f64>, GatewayError> {
    let value = v
        .get(key)
        .ok_or_else(|| err(capability, format!("record 缺数值字段 {key}")))?;
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_f64()
        .map(Some)
        .ok_or_else(|| err(capability, format!("字段 {key} 非数值")))
}

/// 视图可空字符串字段 (JSON null → None; 缺失 → Err fail-closed)。
fn as_optional_str(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<Option<String>, GatewayError> {
    let value = v
        .get(key)
        .ok_or_else(|| err(capability, format!("record 缺字符串字段 {key}")))?;
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(|s| s.to_string())
        .map(Some)
        .ok_or_else(|| err(capability, format!("字段 {key} 非字符串")))
}

/// 视图可空日期字段 ("YYYY-MM-DD" 或 null)。
fn as_optional_date(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<Option<NaiveDate>, GatewayError> {
    let value = v
        .get(key)
        .ok_or_else(|| err(capability, format!("record 缺日期字段 {key}")))?;
    if value.is_null() {
        return Ok(None);
    }
    let s = value
        .as_str()
        .ok_or_else(|| err(capability, format!("字段 {key} 非字符串")))?;
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(Some)
        .map_err(|e| err(capability, format!("字段 {key} 非 YYYY-MM-DD: {s} ({e})")))
}

/// 视图整数字段。
fn as_u64(v: &Value, key: &str, capability: &'static str) -> Result<u64, GatewayError> {
    let value = v
        .get(key)
        .ok_or_else(|| err(capability, format!("record 缺数值字段 {key}")))?;
    value
        .as_u64()
        .ok_or_else(|| err(capability, format!("字段 {key} 非整数")))
}

/// 视图字符串数组字段。
fn as_str_array(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<Vec<String>, GatewayError> {
    let value = v
        .get(key)
        .ok_or_else(|| err(capability, format!("record 缺数组字段 {key}")))?;
    let arr = value
        .as_array()
        .ok_or_else(|| err(capability, format!("字段 {key} 非数组")))?;
    arr.iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| err(capability, format!("字段 {key} 元素非字符串")))
        })
        .collect()
}

// ---------- M3 批次 2: 龙虎榜/大宗/一致预期/板块/研报/北向/财务/技术/资金流/排行/指数/个股新闻/形态/涨停复盘/T0 ----------

/// core 验证类型构造错误 → GatewayError (fail-closed)。
fn core_err(capability: &'static str, e: impl std::fmt::Display) -> GatewayError {
    err(capability, format!("core 验证失败: {e}"))
}

/// 枚举 Debug 名反解析 (视图 format!("{:?}", ...) 契约)。
fn parse_exchange(s: &str, capability: &'static str) -> Result<Exchange, GatewayError> {
    match s {
        "Shanghai" => Ok(Exchange::Shanghai),
        "Shenzhen" => Ok(Exchange::Shenzhen),
        "Beijing" => Ok(Exchange::Beijing),
        _ => Err(err(capability, format!("未知 Exchange: {s}"))),
    }
}

fn parse_board_kind(s: &str, capability: &'static str) -> Result<BoardKind, GatewayError> {
    match s {
        "Industry" => Ok(BoardKind::Industry),
        "Concept" => Ok(BoardKind::Concept),
        "Region" => Ok(BoardKind::Region),
        _ => Err(err(capability, format!("未知 BoardKind: {s}"))),
    }
}

fn parse_flow_interval(s: &str, capability: &'static str) -> Result<FlowInterval, GatewayError> {
    match s {
        "Minute1" => Ok(FlowInterval::Minute1),
        "Day1" => Ok(FlowInterval::Day1),
        "Day5" => Ok(FlowInterval::Day5),
        "Day10" => Ok(FlowInterval::Day10),
        "Day120" => Ok(FlowInterval::Day120),
        _ => Err(err(capability, format!("未知 FlowInterval: {s}"))),
    }
}

fn parse_northbound_channel(
    s: &str,
    capability: &'static str,
) -> Result<NorthboundChannel, GatewayError> {
    match s {
        "Shanghai" => Ok(NorthboundChannel::Shanghai),
        "Shenzhen" => Ok(NorthboundChannel::Shenzhen),
        _ => Err(err(capability, format!("未知 NorthboundChannel: {s}"))),
    }
}

fn parse_dragon_tiger_side(
    s: &str,
    capability: &'static str,
) -> Result<DragonTigerSide, GatewayError> {
    match s {
        "Buy" => Ok(DragonTigerSide::Buy),
        "Sell" => Ok(DragonTigerSide::Sell),
        _ => Err(err(capability, format!("未知 DragonTigerSide: {s}"))),
    }
}

/// Custom("xxx") → 内嵌 NonEmptyText (视图 Debug 名契约)。
fn parse_custom_string(s: &str, capability: &'static str) -> Result<NonEmptyText, GatewayError> {
    let inner = s
        .strip_prefix("Custom(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| err(capability, format!("非 Custom Debug 名: {s}")))?;
    let value: String = serde_json::from_str(inner)
        .map_err(|e| err(capability, format!("Custom 内嵌值解析失败 ({e}): {s}")))?;
    NonEmptyText::new(value).map_err(|e| core_err(capability, e))
}

fn parse_market_ranking_kind(
    s: &str,
    capability: &'static str,
) -> Result<MarketRankingKind, GatewayError> {
    match s {
        "VolumeRatio" => Ok(MarketRankingKind::VolumeRatio),
        "MainNetInflow" => Ok(MarketRankingKind::MainNetInflow),
        "Industry" => Ok(MarketRankingKind::Industry),
        "Concept" => Ok(MarketRankingKind::Concept),
        "Region" => Ok(MarketRankingKind::Region),
        "Popularity" => Ok(MarketRankingKind::Popularity),
        _ if s.starts_with("Custom(") => Ok(MarketRankingKind::Custom(parse_custom_string(
            s, capability,
        )?)),
        _ => Err(err(capability, format!("未知 MarketRankingKind: {s}"))),
    }
}

fn parse_market_ranking_unit(
    s: &str,
    capability: &'static str,
) -> Result<MarketRankingUnit, GatewayError> {
    match s {
        "Multiple" => Ok(MarketRankingUnit::Multiple),
        "Yuan" => Ok(MarketRankingUnit::Yuan),
        "Percent" => Ok(MarketRankingUnit::Percent),
        "Score" => Ok(MarketRankingUnit::Score),
        _ if s.starts_with("Custom(") => Ok(MarketRankingUnit::Custom(parse_custom_string(
            s, capability,
        )?)),
        _ => Err(err(capability, format!("未知 MarketRankingUnit: {s}"))),
    }
}

fn instrument_for(code: &str, capability: &'static str) -> Result<InstrumentId, GatewayError> {
    #[cfg(test)]
    let exchange_code = match code.strip_prefix("TEST_CODE_") {
        Some(suffix) if suffix.len() == 6 && suffix.as_bytes().iter().all(u8::is_ascii_digit) => {
            suffix
        }
        Some(_) => return Err(err(capability, "TEST_CODE suffix 必须是六位 ASCII 数字")),
        None => code,
    };
    #[cfg(not(test))]
    let exchange_code = code;

    let exchange = match crate::grpc_contract::params::exchange_of(exchange_code) {
        "Shanghai" => Exchange::Shanghai,
        "Shenzhen" => Exchange::Shenzhen,
        "Beijing" => Exchange::Beijing,
        other => return Err(err(capability, format!("未知 exchange 前缀: {other}"))),
    };
    InstrumentId::new(exchange, code, AssetClass::Equity).map_err(|e| core_err(capability, e))
}

fn parse_disclosures(
    v: &Value,
    capability: &'static str,
) -> Result<Vec<DragonTigerSourceDisclosure>, GatewayError> {
    let arr = v
        .get("disclosures")
        .and_then(Value::as_array)
        .ok_or_else(|| err(capability, "字段 disclosures 非数组"))?;
    arr.iter()
        .map(|d| {
            let seats = d
                .get("seats")
                .and_then(Value::as_array)
                .ok_or_else(|| err(capability, "字段 seats 非数组"))?
                .iter()
                .map(|s| {
                    Ok(DragonTigerSeatReview {
                        side: parse_dragon_tiger_side(&as_str(s, "side", capability)?, capability)?,
                        rank: as_u64(s, "rank", capability)? as u32,
                        seat_name: as_str(s, "seat_name", capability)?,
                        amount_yuan: as_f64(s, "amount_yuan", capability)?,
                        buy_amount_yuan: as_optional_f64(s, "buy_amount_yuan", capability)?,
                        sell_amount_yuan: as_optional_f64(s, "sell_amount_yuan", capability)?,
                        net_amount_yuan: as_optional_f64(s, "net_amount_yuan", capability)?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DragonTigerSourceDisclosure {
                entry_id: as_str(d, "entry_id", capability)?,
                trade_id: as_str(d, "trade_id", capability)?,
                reason: as_optional_str(d, "reason", capability)?,
                buy_amount_yuan: as_optional_f64(d, "buy_amount_yuan", capability)?,
                sell_amount_yuan: as_optional_f64(d, "sell_amount_yuan", capability)?,
                net_amount_yuan: as_optional_f64(d, "net_amount_yuan", capability)?,
                turnover_rate_pct: as_optional_f64(d, "turnover_rate_pct", capability)?,
                seats,
            })
        })
        .collect()
}

pub fn dragon_tiger(q: &QueryResult) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
    let capability = "DragonTiger";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<DragonTigerStockReview> = parsed
        .iter()
        .map(|v| {
            Ok(DragonTigerStockReview {
                exchange: parse_exchange(&as_str(v, "exchange", capability)?, capability)?,
                code: as_str(v, "code", capability)?,
                ranking_net_amount_yuan: as_f64(v, "ranking_net_amount_yuan", capability)?,
                disclosures: parse_disclosures(v, capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn market_dragon_tiger(
    q: &QueryResult,
) -> Result<GatewayBatch<DragonTigerStockReview>, GatewayError> {
    let capability = "MarketDragonTiger";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<DragonTigerStockReview> = parsed
        .iter()
        .map(|v| {
            Ok(DragonTigerStockReview {
                exchange: parse_exchange(&as_str(v, "exchange", capability)?, capability)?,
                code: as_str(v, "code", capability)?,
                ranking_net_amount_yuan: as_f64(v, "ranking_net_amount_yuan", capability)?,
                disclosures: parse_disclosures(v, capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn block_trades(q: &QueryResult) -> Result<GatewayBatch<BlockTradeReview>, GatewayError> {
    let capability = "BlockTrades";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BlockTradeReview> = parsed
        .iter()
        .map(|v| {
            Ok(BlockTradeReview {
                code: as_str(v, "code", capability)?,
                traded_at: as_optional_str(v, "traded_at", capability)?,
                price: as_f64(v, "price", capability)?,
                close_price: as_optional_f64(v, "close_price", capability)?,
                premium_ratio: as_optional_f64(v, "premium_ratio", capability)?,
                volume: as_f64(v, "volume", capability)?,
                amount: as_optional_f64(v, "amount", capability)?,
                buyer: as_optional_str(v, "buyer", capability)?,
                seller: as_optional_str(v, "seller", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn consensus(q: &QueryResult) -> Result<GatewayBatch<ConsensusData>, GatewayError> {
    let capability = "Consensus";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<ConsensusData> = parsed
        .iter()
        .map(|v| {
            let rating_distribution = v
                .get("rating_distribution")
                .and_then(Value::as_object)
                .ok_or_else(|| err(capability, "字段 rating_distribution 非对象"))?
                .iter()
                .map(|(k, val)| {
                    Ok((
                        k.clone(),
                        val.as_u64().ok_or_else(|| {
                            err(capability, format!("rating_distribution[{k}] 非整数"))
                        })? as u32,
                    ))
                })
                .collect::<Result<std::collections::HashMap<String, u32>, _>>()?;
            Ok(ConsensusData {
                report_count: as_u64(v, "report_count", capability)? as usize,
                broker_count: as_u64(v, "broker_count", capability)? as usize,
                eps_this_year_avg: as_optional_f64(v, "eps_this_year_avg", capability)?,
                eps_next_year_avg: as_optional_f64(v, "eps_next_year_avg", capability)?,
                eps_next2_year_avg: as_optional_f64(v, "eps_next2_year_avg", capability)?,
                rating_distribution,
                target_price_high_avg: None,
                target_price_low_avg: None,
                latest_report_date: None,
                recent_reports: Vec::new(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn board_directory(q: &QueryResult) -> Result<GatewayBatch<BoardDirectoryFact>, GatewayError> {
    let capability = "BoardDirectory";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BoardDirectoryFact> = parsed
        .iter()
        .map(|v| {
            Ok(BoardDirectoryFact {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                kind: parse_board_kind(&as_str(v, "kind", capability)?, capability)?,
                member_count: as_u64(v, "member_count", capability)? as u32,
                evidence: directory_evidence(&ev, capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn board_constituents(
    q: &QueryResult,
) -> Result<GatewayBatch<BoardMembershipRecord>, GatewayError> {
    let capability = "BoardConstituents";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BoardMembershipRecord> = parsed
        .iter()
        .map(|v| {
            Ok(BoardMembershipRecord {
                instrument_code: as_str(v, "instrument_code", capability)?,
                board_code: as_str(v, "board_code", capability)?,
                board_name: as_str(v, "board_name", capability)?,
                kind: parse_board_kind(&as_str(v, "kind", capability)?, capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn board_flows(q: &QueryResult) -> Result<GatewayBatch<BoardFlowFact>, GatewayError> {
    let capability = "BoardFlows";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BoardFlowFact> = parsed
        .iter()
        .map(|v| {
            Ok(BoardFlowFact {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                kind: parse_board_kind(&as_str(v, "kind", capability)?, capability)?,
                rank: as_u64(v, "rank", capability)? as u32,
                return_pct: as_optional_f64(v, "return_pct", capability)?,
                main_net_yuan: as_optional_f64(v, "main_net_yuan", capability)?,
                leader_code: as_optional_str(v, "leader_code", capability)?,
                leader_name: as_optional_str(v, "leader_name", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn board_ranking(q: &QueryResult) -> Result<GatewayBatch<BoardRankingFact>, GatewayError> {
    let capability = "MarketRankings";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<BoardRankingFact> = parsed
        .iter()
        .map(|v| {
            Ok(BoardRankingFact {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                change_pct: as_f64(v, "change_pct", capability)?,
                main_inflow: as_f64(v, "main_inflow", capability)?,
                leader_name: as_str(v, "leader_name", capability)?,
                vol_ratio: as_f64(v, "vol_ratio", capability)?,
                turnover: as_f64(v, "turnover", capability)?,
                day1_ratio: as_f64(v, "day1_ratio", capability)?,
                day5_ratio: as_f64(v, "day5_ratio", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn research_reports(q: &QueryResult) -> Result<GatewayBatch<ResearchReportFact>, GatewayError> {
    let capability = "ResearchReports";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<ResearchReportFact> = parsed
        .iter()
        .map(|v| {
            Ok(ResearchReportFact {
                report_id: as_str(v, "report_id", capability)?,
                title: as_str(v, "title", capability)?,
                organization: as_str(v, "organization", capability)?,
                organization_id: None,
                author: None,
                rating: as_optional_str(v, "rating", capability)?,
                industry_code: None,
                industry_name: None,
                published_at: as_str(v, "published_at", capability)?,
                canonical_url: as_str(v, "canonical_url", capability)?,
                pdf_url: None,
                source_target_price_upper: as_optional_f64(v, "target_price_upper", capability)?,
                source_target_price_lower: as_optional_f64(v, "target_price_lower", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// BoardDirectoryRecordEvidence 构造 (批级 evidence 映射, 视图无逐条证据)。
fn directory_evidence(
    ev: &BatchEvidence,
    capability: &'static str,
) -> Result<BoardDirectoryRecordEvidence, GatewayError> {
    let _ = capability;
    Ok(BoardDirectoryRecordEvidence {
        provider: ev.provider,
        source: ev.source.clone(),
        source_at: ev.source_at.clone(),
        observed_at: ev.observed_at.clone(),
        batch_id: ev.batch_id.clone(),
    })
}

/// 视图级证据 parse (evidence_of + parse_records 合并)。
fn parse_records_parts(
    q: &QueryResult,
    capability: &'static str,
) -> Result<(Vec<Value>, BatchEvidence), GatewayError> {
    let ev = evidence_of(q, capability)?;
    let parsed = parse_records(q, capability)?;
    Ok((parsed, ev))
}

pub fn northbound_daily(
    q: &QueryResult,
) -> Result<GatewayBatch<NorthboundDailyFact>, GatewayError> {
    let capability = "NorthboundDaily";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<NorthboundDailyFact> = parsed
        .iter()
        .map(|v| {
            let quota_balance = match v.get("quota_balance") {
                Some(Value::Number(n)) => NorthboundQuotaFact::Amount(
                    n.as_f64()
                        .ok_or_else(|| err(capability, "quota_balance 非有限数字"))?,
                ),
                Some(Value::String(s)) if s == "unavailable" => NorthboundQuotaFact::Unavailable,
                _ => return Err(err(capability, "quota_balance 必须是数字或 unavailable")),
            };
            let top_turnover = v
                .get("top_turnover")
                .and_then(Value::as_array)
                .ok_or_else(|| err(capability, "字段 top_turnover 非数组"))?
                .iter()
                .map(|t| {
                    Ok(NorthboundTopTurnoverFact {
                        rank: as_u64(t, "rank", capability)? as u32,
                        code: as_str(t, "code", capability)?,
                        name: as_str(t, "name", capability)?,
                        total_turnover: as_f64(t, "total_turnover", capability)?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(NorthboundDailyFact {
                trading_date: as_date(v, "trading_date", capability)?,
                channel: parse_northbound_channel(&as_str(v, "channel", capability)?, capability)?,
                total_turnover: as_f64(v, "total_turnover", capability)?,
                total_trade_count: as_f64(v, "total_trade_count", capability)?,
                quota_balance,
                etf_turnover: as_f64(v, "etf_turnover", capability)?,
                top_turnover,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn financial_statements(
    q: &QueryResult,
) -> Result<GatewayBatch<FinancialStatement>, GatewayError> {
    let capability = "FinancialStatements";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<FinancialStatement> = parsed
        .iter()
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|e| err(capability, format!("FinancialStatement 反序列化失败: {e}")))
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn market_statistics(q: &QueryResult) -> Result<GatewayBatch<MarketStatistics>, GatewayError> {
    let capability = "MarketStatistics";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<MarketStatistics> = parsed
        .iter()
        .map(|v| {
            let code = as_str(v, "code", capability)?;
            let instrument = instrument_for(&code, capability)?;
            MarketStatistics::new(
                instrument,
                as_optional_f64(v, "turnover_rate", capability)?
                    .map(|x| Ratio::decimal(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "trailing_pe", capability)?
                    .map(|x| FiniteNumber::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "static_pe", capability)?
                    .map(|x| FiniteNumber::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "pb", capability)?
                    .map(|x| FiniteNumber::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "total_market_cap", capability)?
                    .map(|x| Money::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "floating_market_cap", capability)?
                    .map(|x| Money::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "upper_limit", capability)?
                    .map(|x| Price::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "lower_limit", capability)?
                    .map(|x| Price::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                as_optional_f64(v, "volume_ratio", capability)?
                    .map(|x| FiniteNumber::new(x).map_err(|e| core_err(capability, e)))
                    .transpose()?,
                record_evidence(&ev, q)?,
            )
            .map_err(|e| core_err(capability, e))
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn technical_bars(q: &QueryResult) -> Result<GatewayBatch<SecurityBar>, GatewayError> {
    let capability = "TechnicalBars";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<SecurityBar> = parsed
        .iter()
        .map(|v| {
            Ok(SecurityBar {
                open: as_f64(v, "open", capability)?,
                close: as_f64(v, "close", capability)?,
                high: as_f64(v, "high", capability)?,
                low: as_f64(v, "low", capability)?,
                vol: as_f64(v, "vol", capability)?,
                amount: as_f64(v, "amount", capability)?,
                year: 0,
                month: 0,
                day: 0,
                hour: 0,
                minute: 0,
                datetime: as_str(v, "at", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn fund_flow_series(
    q: &QueryResult,
) -> Result<GatewayBatch<InstrumentFundFlowFact>, GatewayError> {
    let capability = "FundFlowSeries";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<InstrumentFundFlowFact> = parsed
        .iter()
        .map(|v| {
            Ok(InstrumentFundFlowFact {
                code: as_str(v, "code", capability)?,
                interval: parse_flow_interval(&as_str(v, "interval", capability)?, capability)?,
                period_at: as_str(v, "period_at", capability)?,
                main_net: as_f64(v, "main_net", capability)?,
                main_ratio_percent: as_f64(v, "main_ratio_percent", capability)?,
                super_large_net: as_f64(v, "super_large_net", capability)?,
                large_net: as_f64(v, "large_net", capability)?,
                medium_net: as_f64(v, "medium_net", capability)?,
                small_net: as_f64(v, "small_net", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// 单条头部排行记录解析 (双路 provider_top_n_pair 与全量视图共用)。
fn parse_provider_top_n_record(
    v: &Value,
    capability: &'static str,
) -> Result<ProviderTopNFact, GatewayError> {
    Ok(ProviderTopNFact {
        metric: parse_market_ranking_kind(&as_str(v, "metric", capability)?, capability)?,
        source_order_ordinal: PositiveU32::new(as_u64(v, "ordinal", capability)? as u32)
            .map_err(|e| core_err(capability, e))?,
        instrument: instrument_for(&as_str(v, "code", capability)?, capability)?,
        label: NonEmptyText::new(as_str(v, "label", capability)?)
            .map_err(|e| core_err(capability, e))?,
        value: FiniteNumber::new(as_f64(v, "value", capability)?)
            .map_err(|e| core_err(capability, e))?,
        unit: parse_market_ranking_unit(&as_str(v, "unit", capability)?, capability)?,
        trading_date: IsoDate::new(as_str(v, "trading_date", capability)?)
            .map_err(|e| core_err(capability, e))?,
        filter_identity: NonEmptyText::new(as_str(v, "filter_identity", capability)?)
            .map_err(|e| core_err(capability, e))?,
        provider_declared_total: PositiveU32::new(
            as_u64(v, "provider_declared_total", capability)? as u32,
        )
        .map_err(|e| core_err(capability, e))?,
        // 服务端视图含真实 inspected_row_count (delegate 原样传递, 本地路径语义对等)。
        inspected_row_count: PositiveU32::new(as_u64(v, "inspected_row_count", capability)? as u32)
            .map_err(|e| core_err(capability, e))?,
    })
}

pub fn provider_top_n_rankings(
    q: &QueryResult,
) -> Result<GatewayBatch<ProviderTopNFact>, GatewayError> {
    let capability = "ProviderTopNRankings";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<ProviderTopNFact> = parsed
        .iter()
        .map(|v| parse_provider_top_n_record(v, capability))
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

fn parse_provider_top_n_row_evidence(
    value: &Value,
    capability: &'static str,
) -> Result<BatchEvidence, GatewayError> {
    let evidence = value
        .get("evidence")
        .filter(|value| value.is_object())
        .ok_or_else(|| err(capability, "头部排行 record 缺 evidence 对象"))?;
    let provider_name = as_str(evidence, "provider", capability)?;
    let provider = parse_provider(&provider_name).map_err(|error| {
        err(
            capability,
            format!("record evidence.provider 无法解析: {error}"),
        )
    })?;
    let source = as_str(evidence, "source", capability)?;
    let observed_at = as_str(evidence, "observed_at", capability)?;
    let batch_id = as_str(evidence, "batch_id", capability)?;
    if source.trim().is_empty() || observed_at.trim().is_empty() || batch_id.trim().is_empty() {
        return Err(err(
            capability,
            "头部排行 record evidence 的 source/observed_at/batch_id 不得为空",
        ));
    }
    let observed_instant = crate::data_gateway::parse_evidence_instant(
        capability,
        provider,
        "record.evidence.observed_at",
        &observed_at,
    )?;
    let source_at = match evidence.get("source_at") {
        Some(Value::Null) => None,
        Some(Value::String(value)) if !value.trim().is_empty() => {
            let source_instant = crate::data_gateway::parse_evidence_instant(
                capability,
                provider,
                "record.evidence.source_at",
                value,
            )?;
            if source_instant > observed_instant {
                return Err(err(
                    capability,
                    "头部排行 record evidence.source_at 晚于 observed_at",
                ));
            }
            Some(value.clone())
        }
        _ => {
            return Err(err(
                capability,
                "头部排行 record evidence.source_at 必须显式为时间或 null",
            ))
        }
    };
    Ok(BatchEvidence {
        provider,
        source,
        source_at,
        observed_at,
        batch_id,
    })
}

fn retain_provider_top_n_metric_evidence(
    retained: &mut Option<BatchEvidence>,
    evidence: BatchEvidence,
    capability: &'static str,
) -> Result<(), GatewayError> {
    if retained
        .as_ref()
        .is_some_and(|existing| existing != &evidence)
    {
        return Err(err(
            capability,
            "同一头部排行 metric 混入不同 record evidence",
        ));
    }
    if retained.is_none() {
        *retained = Some(evidence);
    }
    Ok(())
}

fn validate_provider_top_n_source_date(
    record: &ProviderTopNFact,
    evidence: &BatchEvidence,
    capability: &'static str,
) -> Result<NaiveDate, GatewayError> {
    let trading_date = NaiveDate::parse_from_str(record.trading_date.as_str(), "%Y-%m-%d")
        .map_err(|error| {
            err(
                capability,
                format!("头部排行 trading_date 无法解析: {error}"),
            )
        })?;
    let observed_at = crate::data_gateway::parse_evidence_instant(
        capability,
        evidence.provider,
        "record.evidence.observed_at",
        &evidence.observed_at,
    )?;
    let shanghai = chrono::FixedOffset::east_opt(8 * 60 * 60).expect("valid Shanghai fixed offset");
    if trading_date > observed_at.with_timezone(&shanghai).date_naive() {
        return Err(err(
            capability,
            "头部排行 trading_date 晚于 evidence 上海观察日",
        ));
    }
    if let Some(source_at) = evidence.source_at.as_deref() {
        let source_date = crate::data_gateway::parse_evidence_instant(
            capability,
            evidence.provider,
            "record.evidence.source_at",
            source_at,
        )?
        .with_timezone(&shanghai)
        .date_naive();
        if source_date != trading_date {
            return Err(err(
                capability,
                "头部排行 trading_date 与 evidence.source_at 上海日期冲突",
            ));
        }
    }
    Ok(trading_date)
}

/// 头部排行双路 (ProviderTopNPair 视角): 服务端视图合流输出 (无分路顺序
/// 保证), 客户端按 metric 分组重建 volume_ratio / main_net_inflow 两个
/// GatewayBatch — 与本地 CapitalDataGateway::provider_top_n_pair 的
/// `GatewayBatch<ProviderTopNFact> × 2` 结构对齐。
pub fn provider_top_n_pair(
    q: &QueryResult,
) -> Result<
    (
        GatewayBatch<ProviderTopNFact>,
        GatewayBatch<ProviderTopNFact>,
    ),
    GatewayError,
> {
    let capability = "ProviderTopNRankings";
    let (parsed, envelope_evidence) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Err(err(
            capability,
            "BR-240 头部排行原子双路不得缺少任一 metric evidence",
        ));
    }
    let mut volume: Vec<ProviderTopNFact> = Vec::new();
    let mut inflow: Vec<ProviderTopNFact> = Vec::new();
    let mut volume_evidence: Option<BatchEvidence> = None;
    let mut inflow_evidence: Option<BatchEvidence> = None;
    let mut pair_trading_date: Option<NaiveDate> = None;
    for v in &parsed {
        let row_evidence = parse_provider_top_n_row_evidence(v, capability)?;
        let record = parse_provider_top_n_record(v, capability)?;
        let record_trading_date =
            validate_provider_top_n_source_date(&record, &row_evidence, capability)?;
        if pair_trading_date
            .as_ref()
            .is_some_and(|expected| expected != &record_trading_date)
        {
            return Err(err(capability, "BR-240 双路头部排行 trading_date 不一致"));
        }
        if pair_trading_date.is_none() {
            pair_trading_date = Some(record_trading_date);
        }
        match record.metric {
            MarketRankingKind::VolumeRatio => {
                retain_provider_top_n_metric_evidence(
                    &mut volume_evidence,
                    row_evidence,
                    capability,
                )?;
                volume.push(record);
            }
            MarketRankingKind::MainNetInflow => {
                retain_provider_top_n_metric_evidence(
                    &mut inflow_evidence,
                    row_evidence,
                    capability,
                )?;
                inflow.push(record);
            }
            other => return Err(err(capability, format!("头部排行未知 metric: {other:?}"))),
        }
    }
    let volume_evidence = volume_evidence
        .ok_or_else(|| err(capability, "BR-240 volume-ratio metric/evidence 缺失"))?;
    let inflow_evidence = inflow_evidence
        .ok_or_else(|| err(capability, "BR-240 main-net-inflow metric/evidence 缺失"))?;
    if volume_evidence != envelope_evidence {
        return Err(err(
            capability,
            "BR-240 volume-ratio record evidence 与公共 envelope 冲突",
        ));
    }
    if volume_evidence.provider != inflow_evidence.provider
        || volume_evidence.source != inflow_evidence.source
    {
        return Err(err(
            capability,
            "BR-240 双路头部排行 provider/source 不一致",
        ));
    }
    if volume_evidence.batch_id == inflow_evidence.batch_id {
        return Err(err(
            capability,
            "BR-240 两路头部排行不得共享同一上游 batch_id",
        ));
    }
    Ok((
        GatewayBatch::Available {
            records: volume,
            evidence: volume_evidence,
        },
        GatewayBatch::Available {
            records: inflow,
            evidence: inflow_evidence,
        },
    ))
}

pub fn index_quotes(q: &QueryResult) -> Result<GatewayBatch<RealtimeIndexQuote>, GatewayError> {
    let capability = "IndexQuotes";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<RealtimeIndexQuote> = parsed
        .iter()
        .map(|v| {
            Ok(RealtimeIndexQuote {
                code: as_str(v, "code", capability)?,
                name: as_str(v, "name", capability)?,
                current: as_f64(v, "current", capability)?,
                change: as_f64(v, "change", capability)?,
                change_percent: as_f64(v, "change_percent", capability)?,
                open: as_f64(v, "open", capability)?,
                high: as_f64(v, "high", capability)?,
                low: as_f64(v, "low", capability)?,
                previous_close: as_f64(v, "previous_close", capability)?,
                volume: as_f64(v, "volume", capability)?,
                amount: as_f64(v, "amount", capability)?,
                source_at: as_rfc3339(v, "source_at", capability)?,
                observed_at: record_observed_at(q, capability)?,
                provider: ev.provider,
                batch_id: ev.batch_id.clone(),
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn instrument_news(
    q: &QueryResult,
) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
    let capability = "InstrumentNews";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<SinaInstrumentNewsRecord> = parsed
        .iter()
        .map(|v| {
            let item = NewsItem {
                source: as_str(v, "source", capability)?,
                external_id: as_str(v, "external_id", capability)?,
                category: as_str(v, "category", capability)?,
                code: as_optional_str(v, "code", capability)?,
                title: as_str(v, "title", capability)?,
                summary: as_str(v, "summary", capability)?,
                url: as_str(v, "url", capability)?,
                source_name: as_str(v, "source_name", capability)?,
                published_at: as_rfc3339(v, "published_at", capability)?,
                fetched_at: as_rfc3339(v, "fetched_at", capability)?,
                content_hash: as_str(v, "content_hash", capability)?,
            };
            Ok(SinaInstrumentNewsRecord::new(
                item,
                record_evidence(&ev, q)?,
            ))
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

const EXTERNAL_INSTRUMENT_NEWS_SCHEMA: &str = "magic.market.news_item";
const EXTERNAL_INSTRUMENT_NEWS_FIELDS: &[&str] = &[
    "item_id",
    "title",
    "summary",
    "content",
    "publisher",
    "url",
    "published_at",
    "instruments",
    "topics",
    "language",
    "evidence",
];

#[derive(serde::Deserialize)]
struct ExternalNewsItemWire {
    item_id: String,
    title: String,
    summary: Option<String>,
    content: Option<String>,
    publisher: String,
    url: String,
    published_at: String,
    instruments: Vec<InstrumentId>,
    topics: Vec<String>,
    language: String,
    evidence: SourceEvidence,
}

/// Normalize the delivered ExternalV1 `magic.market.news_item@2` contract.
///
/// The wire record stays bound to the exact canonical instrument and immutable
/// batch evidence that authorized the read. Missing provider source time is
/// never replaced by the acquisition time; the legacy persistence projection
/// uses the envelope observation only for `fetched_at`.
pub fn external_instrument_news(
    storage_code: &str,
    requested_instrument: &InstrumentId,
    request_limit: usize,
    q: &QueryResult,
) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
    external_instrument_news_with_range(
        storage_code,
        requested_instrument,
        None,
        request_limit,
        q,
        None,
        None,
    )
}

pub fn external_instrument_news_at(
    storage_code: &str,
    requested_instrument: &InstrumentId,
    request_limit: usize,
    q: &QueryResult,
    consumer_now: DateTime<Utc>,
) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
    external_instrument_news_with_range(
        storage_code,
        requested_instrument,
        None,
        request_limit,
        q,
        None,
        Some(consumer_now),
    )
}

pub fn external_instrument_news_in_range(
    storage_code: &str,
    requested_instrument: &InstrumentId,
    request_start: NaiveDate,
    request_end: NaiveDate,
    request_limit: usize,
    q: &QueryResult,
) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
    if request_start > request_end {
        return Err(GatewayError::invalid_request(
            "InstrumentNews",
            "ExternalV1 InstrumentNews request start must not follow end",
        ));
    }
    external_instrument_news_with_range(
        storage_code,
        requested_instrument,
        Some((request_start, request_end)),
        request_limit,
        q,
        None,
        None,
    )
}

/// Immutable request and observation bounds for one ExternalV1 InstrumentNews
/// projection. Keeping these values together prevents callers from mixing the
/// RPC's captured upper bound with a later consumer clock.
#[derive(Debug, Clone, Copy)]
pub struct ExternalInstrumentNewsRequestContext {
    request_start: NaiveDate,
    request_end: NaiveDate,
    request_limit: usize,
    captured_through: DateTime<Utc>,
    consumer_now: DateTime<Utc>,
}

impl ExternalInstrumentNewsRequestContext {
    pub const fn new(
        request_start: NaiveDate,
        request_end: NaiveDate,
        request_limit: usize,
        captured_through: DateTime<Utc>,
        consumer_now: DateTime<Utc>,
    ) -> Self {
        Self {
            request_start,
            request_end,
            request_limit,
            captured_through,
            consumer_now,
        }
    }
}

pub fn external_instrument_news_in_range_at(
    storage_code: &str,
    requested_instrument: &InstrumentId,
    q: &QueryResult,
    context: ExternalInstrumentNewsRequestContext,
) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
    if context.request_start > context.request_end {
        return Err(GatewayError::invalid_request(
            "InstrumentNews",
            "ExternalV1 InstrumentNews request start must not follow end",
        ));
    }
    external_instrument_news_with_range(
        storage_code,
        requested_instrument,
        Some((context.request_start, context.request_end)),
        context.request_limit,
        q,
        Some(context.captured_through),
        Some(context.consumer_now),
    )
}

fn external_instrument_news_with_range(
    storage_code: &str,
    requested_instrument: &InstrumentId,
    requested_range: Option<(NaiveDate, NaiveDate)>,
    request_limit: usize,
    q: &QueryResult,
    captured_through: Option<DateTime<Utc>>,
    consumer_now: Option<DateTime<Utc>>,
) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
    let capability = "InstrumentNews";
    let evidence = evidence_of(q, capability)?;
    if !q.complete {
        return Err(err(capability, "ExternalV1 InstrumentNews complete=false"));
    }
    if requested_instrument.asset_class() != AssetClass::Equity
        || requested_instrument.code() != storage_code
    {
        return Err(err(
            capability,
            "requested canonical instrument 与 storage code 冲突",
        ));
    }
    if !(1..=10_000).contains(&request_limit) || q.records.len() > request_limit {
        return Err(err(
            capability,
            "external news record count 超过 exact request limit",
        ));
    }
    let fetched_at = crate::data_gateway::parse_evidence_instant(
        capability,
        evidence.provider,
        "observed_at",
        &q.observed_at,
    )?;
    if let Some(now) = consumer_now {
        let max_age = chrono::TimeDelta::seconds(30);
        let age = now.signed_duration_since(fetched_at);
        if age < -MAX_CLOCK_SKEW || age > max_age {
            return Err(GatewayError::classified(
                capability,
                Some(evidence.provider),
                "stale",
                "observation_stale",
                true,
                format!(
                    "instrument news observation failed freshness gate age_ms={} max_age_ms={} max_clock_skew_ms={}",
                    age.num_milliseconds(),
                    max_age.num_milliseconds(),
                    MAX_CLOCK_SKEW.num_milliseconds()
                ),
            ));
        }
    }
    let envelope_source_at = evidence
        .source_at
        .as_deref()
        .map(|value| {
            crate::data_gateway::parse_evidence_instant(
                capability,
                evidence.provider,
                "source_at",
                value,
            )
        })
        .transpose()?;
    if envelope_source_at.is_some_and(|source_at| source_at > fetched_at) {
        return Err(err(capability, "source_at 晚于 observed_at"));
    }
    if let (Some(now), Some(source_at)) = (consumer_now, envelope_source_at) {
        if source_at > now + MAX_CLOCK_SKEW {
            return Err(GatewayError::invalid_evidence(
                capability,
                Some(evidence.provider),
                "instrument news source timestamp exceeds maximum clock skew",
            ));
        }
        let shanghai =
            chrono::FixedOffset::east_opt(8 * 60 * 60).expect("Shanghai fixed UTC offset is valid");
        let source_date = source_at.with_timezone(&shanghai).date_naive();
        let today = now.with_timezone(&shanghai).date_naive();
        let oldest_allowed = crate::calendar::prev_trading_day(today);
        if source_date < oldest_allowed {
            return Err(GatewayError::classified(
                capability,
                Some(evidence.provider),
                "stale",
                "daily_source_stale",
                true,
                format!(
                    "instrument news source date {source_date} is older than one trading day; oldest_allowed={oldest_allowed}"
                ),
            ));
        }
    }
    if q.records.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(evidence));
    }

    let mut records = Vec::with_capacity(q.records.len());
    for payload in &q.records {
        validate_external_news_payload(payload, capability)?;
        let value: Value = serde_json::from_slice(&payload.data)
            .map_err(|error| err(capability, format!("external news 非 JSON object: {error}")))?;
        validate_external_news_fields(&value, capability)?;
        let wire: ExternalNewsItemWire = serde_json::from_value(value)
            .map_err(|error| err(capability, format!("external news 字段无效: {error}")))?;

        validate_external_news_text(&wire.item_id, "item_id", capability)?;
        validate_external_news_text(&wire.title, "title", capability)?;
        validate_external_news_text(&wire.publisher, "publisher", capability)?;
        validate_external_news_text(&wire.url, "url", capability)?;
        validate_external_news_text(&wire.published_at, "published_at", capability)?;
        validate_external_news_text(&wire.language, "language", capability)?;
        if let Some(summary) = wire.summary.as_deref() {
            validate_external_news_text(summary, "summary", capability)?;
        }
        if let Some(content) = wire.content.as_deref() {
            validate_external_news_text(content, "content", capability)?;
        }
        for topic in &wire.topics {
            validate_external_news_text(topic, "topics[]", capability)?;
        }
        if !wire
            .instruments
            .iter()
            .any(|instrument| instrument == requested_instrument)
        {
            return Err(err(
                capability,
                "external news instruments 不含 exact request instrument",
            ));
        }
        let parsed_url = url::Url::parse(&wire.url)
            .map_err(|error| err(capability, format!("url 无效: {error}")))?;
        if parsed_url.scheme() != "https" || parsed_url.host_str().is_none() {
            return Err(err(capability, "url 必须是绝对 HTTPS URL"));
        }
        if wire.evidence.provider() != evidence.provider
            || wire.evidence.batch_id() != evidence.batch_id
        {
            return Err(err(
                capability,
                "external news record provider/batch 与 envelope 冲突",
            ));
        }
        let record_observed_at = crate::data_gateway::parse_evidence_instant(
            capability,
            evidence.provider,
            "record.evidence.observed_at",
            wire.evidence.observed_at(),
        )?;
        if record_observed_at > fetched_at {
            return Err(err(
                capability,
                "external news record observed_at 晚于 envelope final observed_at",
            ));
        }
        let published_at = DateTime::parse_from_rfc3339(&wire.published_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|error| {
                err(
                    capability,
                    format!("external news published_at 非 RFC3339: {error}"),
                )
            })?;
        if captured_through.is_some_and(|captured| published_at > captured) {
            return Err(err(
                capability,
                "external news published_at exceeds caller-captured upper bound",
            ));
        }
        if let Some((request_start, request_end)) = requested_range {
            let shanghai = chrono::FixedOffset::east_opt(8 * 60 * 60)
                .expect("Shanghai fixed UTC offset is valid");
            let published_date = published_at.with_timezone(&shanghai).date_naive();
            if published_date < request_start || published_date > request_end {
                return Err(err(
                    capability,
                    format!(
                        "external news published_at outside exact request range: date={published_date} start={request_start} end={request_end}"
                    ),
                ));
            }
        }
        let record_source_at = wire
            .evidence
            .source_at()
            .map(|value| {
                crate::data_gateway::parse_evidence_instant(
                    capability,
                    evidence.provider,
                    "record.evidence.source_at",
                    value,
                )
            })
            .transpose()?;
        match (record_source_at, envelope_source_at) {
            (Some(record_source_at), Some(envelope_source_at)) => {
                if record_source_at > envelope_source_at {
                    return Err(err(
                        capability,
                        "external news record source_at 晚于 envelope newest source_at",
                    ));
                }
                if record_source_at != published_at {
                    return Err(err(
                        capability,
                        "external news record source_at 与 published_at 冲突",
                    ));
                }
            }
            (None, None) => {}
            _ => {
                return Err(err(
                    capability,
                    "external news record/envelope source_at presence 冲突",
                ));
            }
        }
        if record_source_at.is_some_and(|source_at| source_at > record_observed_at) {
            return Err(err(
                capability,
                "external news record source_at 晚于 observed_at",
            ));
        }
        if published_at > record_observed_at {
            return Err(err(
                capability,
                "external news observed_at 早于 published_at",
            ));
        }

        // The canonical contract represents a missing summary as JSON null.
        // `NewsItem` is a legacy non-optional projection, so blank preserves
        // that explicit missing value without inventing provider content.
        let summary = wire.summary.unwrap_or_default();
        let persistence_item = NewsItem {
            source: q.selected_provider.clone(),
            external_id: wire.item_id,
            category: "个股新闻".to_string(),
            code: Some(storage_code.to_string()),
            title: wire.title.clone(),
            summary: summary.clone(),
            url: wire.url,
            source_name: wire.publisher,
            published_at,
            fetched_at,
            content_hash: crate::data_provider::news_item::content_hash(&wire.title, &summary),
        };
        records.push(SinaInstrumentNewsRecord::new(
            persistence_item,
            wire.evidence,
        ));
    }
    Ok(GatewayBatch::Available { records, evidence })
}

fn validate_external_news_payload(
    payload: &crate::grpc_client::pb::magic::market::v1::CanonicalPayload,
    capability: &'static str,
) -> Result<(), GatewayError> {
    if payload.schema != EXTERNAL_INSTRUMENT_NEWS_SCHEMA
        || payload.schema_version != 2
        || payload.content_type != CANONICAL_JSON_CONTENT_TYPE
    {
        return Err(err(
            capability,
            "InstrumentNews schema/version/content-type 冲突",
        ));
    }
    Ok(())
}

fn validate_external_news_fields(
    value: &Value,
    capability: &'static str,
) -> Result<(), GatewayError> {
    let object = value
        .as_object()
        .ok_or_else(|| err(capability, "external news payload 必须是 JSON object"))?;
    if object.len() != EXTERNAL_INSTRUMENT_NEWS_FIELDS.len()
        || EXTERNAL_INSTRUMENT_NEWS_FIELDS
            .iter()
            .any(|field| !object.contains_key(*field))
    {
        return Err(err(
            capability,
            "external news payload fields 与 schema@2 冲突",
        ));
    }
    Ok(())
}

fn validate_external_news_text(
    value: &str,
    field: &'static str,
    capability: &'static str,
) -> Result<(), GatewayError> {
    NonEmptyText::new(value)
        .map(|_| ())
        .map_err(|error| err(capability, format!("external news {field} 无效: {error}")))
}

pub fn intraday_shape(q: &QueryResult) -> Result<GatewayBatch<IntradayShapeFact>, GatewayError> {
    let capability = "IntradayShape";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<IntradayShapeFact> = parsed
        .iter()
        .map(|v| {
            // shape_label 是 &'static str (视图字符串) → Box::leak 保生命周期
            // (每批 record 数有限, 形状标签来自服务端已验证值, 非用户输入)。
            let label: &'static str =
                Box::leak(as_str(v, "shape_label", capability)?.into_boxed_str());
            Ok(IntradayShapeFact {
                date: as_str(v, "date", capability)?,
                pre_close: as_f64(v, "pre_close", capability)?,
                open_pct: as_f64(v, "open_pct", capability)?,
                high_pct: as_f64(v, "high_pct", capability)?,
                low_pct: as_f64(v, "low_pct", capability)?,
                close_pct: as_f64(v, "close_pct", capability)?,
                amplitude: as_f64(v, "amplitude", capability)?,
                tail_30m_pct: as_optional_f64(v, "tail_30m_pct", capability)?,
                shape_label: label,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

pub fn upper_limit_pool_review(
    q: &QueryResult,
) -> Result<GatewayBatch<UpperLimitRecord>, GatewayError> {
    let capability = "UpperLimitPoolReview";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<UpperLimitRecord> = parsed
        .iter()
        .map(|v| {
            Ok(UpperLimitRecord {
                code: as_str(v, "code", capability)?,
                trading_date: as_date(v, "trading_date", capability)?,
                theme: as_optional_str(v, "theme", capability)?,
                streak: as_optional_u32(v, "streak", capability)?,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

const LIMIT_POOL_SCHEMA: &str = "market.limit_pools";
const LIMIT_POOL_MAX_RECORDS: usize = 200;
const LIMIT_POOL_FIELDS: &[&str] = &[
    "kind",
    "instrument",
    "trading_date",
    "price",
    "change",
    "volume",
    "turnover",
    "sealed_amount",
    "first_seal_at",
    "last_seal_at",
    "break_count",
    "streak",
    "industry",
    "board_name",
    "seal_state",
    "reseal_count",
    "reason",
    "evidence",
];

/// Complete LocalBridge LimitPools contract. Optional provider fields must be
/// present as explicit nulls; serde performs the typed positive-price and
/// finite-ratio validation after the closed field-set check.
pub fn limit_pools(
    q: &QueryResult,
    trading_date: NaiveDate,
) -> Result<GatewayBatch<LimitPoolEntry>, GatewayError> {
    let capability = "LimitPools";
    let evidence = evidence_of(q, capability)?;
    if !q.complete {
        return Err(err(capability, "响应 complete=false，不接纳截断涨停池"));
    }
    if !matches!(
        evidence.provider,
        ProviderId::Eastmoney | ProviderId::Tonghuashun
    ) {
        return Err(err(
            capability,
            "batch provider 不是已登记 exact-date 涨停池来源",
        ));
    }
    let expected_date = trading_date.format("%Y-%m-%d").to_string();
    if evidence.source_at.as_deref() != Some(expected_date.as_str()) {
        return Err(err(capability, "batch source_at 与请求交易日冲突"));
    }
    crate::data_gateway::parse_evidence_instant(
        capability,
        evidence.provider,
        "observed_at",
        &evidence.observed_at,
    )?;
    if q.records.len() != 1 {
        return Err(err(
            capability,
            format!(
                "canonical LimitPools contract requires exactly one payload, got {}",
                q.records.len()
            ),
        ));
    }
    let payload = &q.records[0];
    if payload.schema != LIMIT_POOL_SCHEMA
        || payload.schema_version != 1
        || payload.content_type != CANONICAL_JSON_CONTENT_TYPE
    {
        return Err(err(
            capability,
            "LimitPools schema/version/content-type 冲突",
        ));
    }
    let values: Vec<Value> = serde_json::from_slice(&payload.data).map_err(|error| {
        err(
            capability,
            format!("LimitPools payload 非 JSON 数组: {error}"),
        )
    })?;
    if values.len() > LIMIT_POOL_MAX_RECORDS {
        return Err(err(
            capability,
            "LimitPools records 超过 P-01 whole-pool 上限 200",
        ));
    }

    let mut seen_codes = std::collections::HashSet::with_capacity(values.len());
    let mut records = Vec::with_capacity(values.len());
    for value in values {
        let object = value
            .as_object()
            .ok_or_else(|| err(capability, "LimitPoolEntry 必须是 JSON object"))?;
        if object.len() != LIMIT_POOL_FIELDS.len()
            || LIMIT_POOL_FIELDS
                .iter()
                .any(|field| !object.contains_key(*field))
        {
            return Err(err(capability, "LimitPoolEntry fields 与 schema@1 冲突"));
        }
        let record: LimitPoolEntry = serde_json::from_value(value)
            .map_err(|error| err(capability, format!("LimitPoolEntry 类型校验失败: {error}")))?;
        if record.kind != LimitPoolKind::Upper
            || record.trading_date.as_str() != expected_date.as_str()
        {
            return Err(err(
                capability,
                "LimitPoolEntry kind/trading_date 与请求冲突",
            ));
        }
        if record.evidence.provider() != evidence.provider
            || record.evidence.source_at() != evidence.source_at.as_deref()
            || record.evidence.observed_at() != evidence.observed_at.as_str()
            || record.evidence.batch_id() != evidence.batch_id.as_str()
        {
            return Err(err(
                capability,
                "LimitPoolEntry record evidence 与 envelope 冲突",
            ));
        }
        if !seen_codes.insert(record.instrument.code().to_owned()) {
            return Err(err(capability, "LimitPools 包含重复 instrument code"));
        }
        records.push(record);
    }

    if records.is_empty() {
        Ok(GatewayBatch::VerifiedEmpty(evidence))
    } else {
        Ok(GatewayBatch::Available { records, evidence })
    }
}

const T0_SCHEMA: &str = "market.t0_evidence";
const T0_SCHEMA_VERSION: u32 = 2;
const CHINA_OFFSET_SECONDS: i32 = 8 * 60 * 60;

fn t0_china_session_at(value: &Value) -> Result<NaiveDateTime, GatewayError> {
    let capability = "T0Evidence";
    let raw = as_str(value, "at", capability)?;
    let parsed = DateTime::parse_from_rfc3339(&raw)
        .map_err(|_| err(capability, "T0 five-minute at must be RFC3339"))?;
    if parsed.offset().local_minus_utc() != CHINA_OFFSET_SECONDS {
        return Err(err(
            capability,
            "T0 five-minute at must use explicit +08:00",
        ));
    }
    Ok(parsed.naive_local())
}

/// T0 证据批: 视图是含批级证据、records 与 rejections 的 v2 对象
/// (grpc_server::t0_wire 契约)。
/// 返回 MagicTdxT0Batch (records + rejections 全量) — 与本地
/// MagicTdxGateway::get_t0_evidence_batch 对齐, rejections 绝不丢弃。
/// requested_at 只从批级字段恢复；空 records 也绝不以 observed_at 或 consumer now 代填。
pub fn t0_evidence_batch(q: &QueryResult) -> Result<MagicTdxT0Batch, GatewayError> {
    let capability = "T0Evidence";
    let ev = evidence_of(q, capability)?;
    if !q.complete {
        return Err(err(
            capability,
            "响应 complete=false，T0 consumer 不接纳 partial batch",
        ));
    }
    if q.records.len() != 1 {
        return Err(err(
            capability,
            format!(
                "T0 canonical contract requires exactly one payload, got {}",
                q.records.len()
            ),
        ));
    }
    let payload = &q.records[0];
    if payload.schema != T0_SCHEMA
        || payload.schema_version != T0_SCHEMA_VERSION
        || payload.content_type != CANONICAL_JSON_CONTENT_TYPE
    {
        return Err(err(
            capability,
            "T0Evidence schema/version/content-type 冲突",
        ));
    }
    // 合同 v2: 视图必须是含批级证据的对象，不接纳 v1 或数组兼容形状。
    let value: Value = serde_json::from_slice(&payload.data)
        .map_err(|e| err(capability, format!("T0Evidence 视图非 JSON: {e}")))?;
    let view = value
        .as_object()
        .ok_or_else(|| err(capability, "T0Evidence 视图必须是 JSON 对象"))?;
    let requested_at_raw = as_str(&value, "requested_at", capability)?;
    let source_at_raw = as_str(&value, "source_at", capability)?;
    let observed_at_raw = as_str(&value, "observed_at", capability)?;
    let batch_id = as_str(&value, "batch_id", capability)?;
    let time_untrustworthy = as_bool(&value, "time_untrustworthy", capability)?;
    let requested_at = crate::data_gateway::parse_evidence_instant(
        capability,
        ev.provider,
        "batch.requested_at",
        &requested_at_raw,
    )?;
    let source_at = crate::data_gateway::parse_evidence_instant(
        capability,
        ev.provider,
        "batch.source_at",
        &source_at_raw,
    )?;
    let observed_at = crate::data_gateway::parse_evidence_instant(
        capability,
        ev.provider,
        "batch.observed_at",
        &observed_at_raw,
    )?;
    let envelope_source_at_raw = ev
        .source_at
        .as_deref()
        .ok_or_else(|| err(capability, "T0Evidence envelope source_at 缺失"))?;
    let envelope_source_at = crate::data_gateway::parse_evidence_instant(
        capability,
        ev.provider,
        "envelope.source_at",
        envelope_source_at_raw,
    )?;
    let envelope_observed_at = crate::data_gateway::parse_evidence_instant(
        capability,
        ev.provider,
        "envelope.observed_at",
        &ev.observed_at,
    )?;
    if source_at != envelope_source_at
        || observed_at != envelope_observed_at
        || batch_id != ev.batch_id
    {
        return Err(err(
            capability,
            "T0Evidence batch identity conflicts with gRPC envelope",
        ));
    }
    let records = view
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| err(capability, "T0Evidence records 非数组"))?;
    let mut out = Vec::new();
    for v in records {
        let quote_obj = v
            .get("quote")
            .and_then(Value::as_object)
            .ok_or_else(|| err(capability, "T0Evidence quote 非对象"))?;
        let book =
            |key: &str| -> Result<[T0BookLevel; 5], GatewayError> {
                let arr = quote_obj
                    .get(key)
                    .and_then(Value::as_array)
                    .ok_or_else(|| err(capability, format!("T0 quote.{key} 非数组")))?;
                let mut levels: Vec<T0BookLevel> = Vec::new();
                for item in arr {
                    let obj = item
                        .as_object()
                        .ok_or_else(|| err(capability, format!("T0 quote.{key} 元素非对象")))?;
                    levels.push(T0BookLevel {
                        price: obj.get("price").and_then(Value::as_f64).ok_or_else(|| {
                            err(capability, format!("T0 quote.{key}[].price 非法"))
                        })?,
                        volume: obj.get("volume").and_then(Value::as_f64).ok_or_else(|| {
                            err(capability, format!("T0 quote.{key}[].volume 非法"))
                        })?,
                    });
                }
                <[T0BookLevel; 5]>::try_from(levels)
                    .map_err(|_| err(capability, format!("T0 quote.{key} 长度必须为 5")))
            };
        let settled_daily = v
            .get("settled_daily")
            .and_then(Value::as_array)
            .ok_or_else(|| err(capability, "T0Evidence settled_daily 非数组"))?
            .iter()
            .map(|b| {
                Ok(MagicTdxT0DailyBar {
                    date: as_date(b, "date", capability)?,
                    open: as_f64(b, "open", capability)?,
                    high: as_f64(b, "high", capability)?,
                    low: as_f64(b, "low", capability)?,
                    close: as_f64(b, "close", capability)?,
                    volume: as_f64(b, "volume", capability)?,
                    amount: as_f64(b, "amount", capability)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let completed_five_minute = v
            .get("completed_five_minute")
            .and_then(Value::as_array)
            .ok_or_else(|| err(capability, "T0Evidence completed_five_minute 非数组"))?
            .iter()
            .map(|b| {
                Ok(MagicTdxT0FiveMinuteBar {
                    at: t0_china_session_at(b)?,
                    open: as_f64(b, "open", capability)?,
                    high: as_f64(b, "high", capability)?,
                    low: as_f64(b, "low", capability)?,
                    close: as_f64(b, "close", capability)?,
                    volume: as_f64(b, "volume", capability)?,
                    amount: as_f64(b, "amount", capability)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let code = as_str(v, "code", capability)?;
        let requested_at_raw = as_str(v, "requested_at", capability)?;
        let source_at_raw = as_str(v, "source_at", capability)?;
        let observed_at_raw = as_str(v, "observed_at", capability)?;
        let record_requested_at = crate::data_gateway::parse_evidence_instant(
            capability,
            ev.provider,
            "record.requested_at",
            &requested_at_raw,
        )?;
        let record_source_at = crate::data_gateway::parse_evidence_instant(
            capability,
            ev.provider,
            "record.source_at",
            &source_at_raw,
        )?;
        let record_observed_at = crate::data_gateway::parse_evidence_instant(
            capability,
            ev.provider,
            "record.observed_at",
            &observed_at_raw,
        )?;
        let record_batch_id = as_str(v, "batch_id", capability)?;
        if record_requested_at != requested_at
            || record_observed_at != observed_at
            || record_batch_id != batch_id
        {
            return Err(err(
                capability,
                format!("T0 record evidence differs from batch: code={code}"),
            ));
        }
        out.push(MagicTdxT0Evidence {
            instrument: instrument_for(&code, capability)?,
            code,
            requested_at: record_requested_at,
            source_at: record_source_at,
            observed_at: record_observed_at,
            batch_id: record_batch_id,
            quote: MagicTdxT0Quote {
                price: quote_obj
                    .get("price")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.price 非法"))?,
                last_close: quote_obj
                    .get("last_close")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.last_close 非法"))?,
                open: quote_obj
                    .get("open")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.open 非法"))?,
                high: quote_obj
                    .get("high")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.high 非法"))?,
                low: quote_obj
                    .get("low")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.low 非法"))?,
                volume: quote_obj
                    .get("volume")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.volume 非法"))?,
                amount: quote_obj
                    .get("amount")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| err(capability, "T0 quote.amount 非法"))?,
                bids: book("bids")?,
                asks: book("asks")?,
            },
            settled_daily,
            completed_five_minute,
            intraday_average_price: as_f64(v, "intraday_average_price", capability)?,
        });
    }
    let rejections = view
        .get("rejections")
        .and_then(Value::as_array)
        .ok_or_else(|| err(capability, "T0Evidence rejections 非数组"))?
        .iter()
        .map(|r| {
            let code = as_str(r, "code", capability)?;
            Ok(MagicTdxT0Rejection {
                code,
                reason_code: Box::leak(as_str(r, "reason_code", capability)?.into_boxed_str()),
                detail: as_str(r, "detail", capability)?,
                retryable: as_bool(r, "retryable", capability)?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MagicTdxT0Batch {
        provider: ev.provider,
        source: ev.source.clone(),
        requested_at,
        source_at,
        observed_at,
        batch_id,
        records: out,
        rejections,
        time_untrustworthy,
    })
}

fn require_positive_live_value(
    capability: &'static str,
    field: &str,
    value: f64,
) -> Result<(), GatewayError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(err(
            capability,
            format!("{field} 必须为正有限数，实际为 {value}"),
        ))
    }
}

fn require_non_negative_live_value(
    capability: &'static str,
    field: &str,
    value: f64,
) -> Result<(), GatewayError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(err(
            capability,
            format!("{field} 必须为非负有限数，实际为 {value}"),
        ))
    }
}

fn validate_t0_live_quote(
    quote: &MagicTdxT0Quote,
    capability: &'static str,
) -> Result<(), GatewayError> {
    for (field, value) in [
        ("quote.price", quote.price),
        ("quote.last_close", quote.last_close),
        ("quote.open", quote.open),
        ("quote.high", quote.high),
        ("quote.low", quote.low),
    ] {
        require_positive_live_value(capability, field, value)?;
    }
    for (field, value) in [
        ("quote.volume", quote.volume),
        ("quote.amount", quote.amount),
    ] {
        require_non_negative_live_value(capability, field, value)?;
    }
    for (side, levels) in [("bids", &quote.bids), ("asks", &quote.asks)] {
        for (index, level) in levels.iter().enumerate() {
            require_positive_live_value(
                capability,
                &format!("quote.{side}[{index}].price"),
                level.price,
            )?;
            require_non_negative_live_value(
                capability,
                &format!("quote.{side}[{index}].volume"),
                level.volume,
            )?;
        }
    }
    Ok(())
}

/// RPC 完成后的 consumer-side T0 完整批次门。
pub fn t0_evidence_batch_at(
    q: &QueryResult,
    now: DateTime<Utc>,
) -> Result<MagicTdxT0Batch, GatewayError> {
    let capability = "T0Evidence";
    let (envelope, source_at, observed_at, batch_time_untrustworthy) =
        live_evidence_times_lenient(q, capability, now)?;
    let mut batch = t0_evidence_batch(q)?;
    if batch.provider != envelope.provider
        || batch.source != envelope.source
        || batch.batch_id != envelope.batch_id
        || batch.source_at != source_at
        || batch.observed_at != observed_at
    {
        return Err(err(
            capability,
            "T0 reconstructed batch evidence differs from gRPC envelope",
        ));
    }
    if batch.requested_at > batch.observed_at {
        return Err(err(
            capability,
            format!(
                "batch.requested_at 晚于 observed_at: requested_at={} observed_at={}",
                batch.requested_at, batch.observed_at
            ),
        ));
    }

    let mut minimum_record_source_at: Option<DateTime<Utc>> = None;
    let mut record_time_untrustworthy = false;
    for record in &batch.records {
        if record.code != record.instrument.code() {
            return Err(err(
                capability,
                format!(
                    "T0 record identity mismatch: code={} instrument={}",
                    record.code,
                    record.instrument.code()
                ),
            ));
        }
        if record.batch_id != batch.batch_id
            || record.requested_at != batch.requested_at
            || record.observed_at != batch.observed_at
        {
            return Err(err(
                capability,
                format!(
                    "T0 record evidence differs from batch: code={}",
                    record.code
                ),
            ));
        }
        // 2026-08-21 用户指令「拿不到时间先跳过, 标注时间不可信」: 记录级 age 门
        // 同样放宽 (server 整批组装 11-55s 的延迟会落在这个记录上), 未来/倒挂仍硬错。
        let record_age = now.signed_duration_since(record.source_at);
        if record.source_at > now || record.observed_at > now {
            return Err(live_time_error(
                capability,
                batch.provider,
                format!(
                    "record 时间晚于 consumer now: source_at={} observed_at={} now={}",
                    record.source_at, record.observed_at, now
                ),
            ));
        }
        if record.source_at > record.observed_at {
            return Err(err(
                capability,
                format!(
                    "T0 record.source_at 晚于 observed_at: source_at={} observed_at={}",
                    record.source_at, record.observed_at
                ),
            ));
        }
        if record_age > LIVE_BATCH_MAX_AGE {
            record_time_untrustworthy = true;
            log::warn!(
                "[{capability}][时间不可信] record code={} age_ns={} max_ns={} \
                 source_at={} observed_at={} now={} (桥组装延迟, 放宽 age 门, 推送须标注)",
                record.code,
                record_age.num_nanoseconds().unwrap_or(i64::MAX),
                LIVE_BATCH_MAX_AGE.num_nanoseconds().unwrap_or(i64::MAX),
                record.source_at,
                record.observed_at,
                now,
            );
        }
        validate_t0_live_quote(&record.quote, capability)?;
        require_positive_live_value(
            capability,
            "intraday_average_price",
            record.intraday_average_price,
        )?;
        minimum_record_source_at = Some(
            minimum_record_source_at
                .map(|current| current.min(record.source_at))
                .unwrap_or(record.source_at),
        );
    }
    if !batch.records.is_empty() && minimum_record_source_at != Some(batch.source_at) {
        return Err(err(
            capability,
            format!(
                "batch.source_at 必须等于最早 record.source_at: batch={} minimum={:?}",
                batch.source_at, minimum_record_source_at
            ),
        ));
    }
    // 2026-08-21 用户指令: 拿不到可信时间 → 跳过 age 门但必须标注, 推送方据
    // time_untrustworthy 打印「时间不可信」, 禁止静默放行。
    batch.time_untrustworthy =
        batch.time_untrustworthy || batch_time_untrustworthy || record_time_untrustworthy;
    if batch.time_untrustworthy {
        log::warn!(
            "[{capability}][时间不可信] 整批已放宽 age 门: source_at={source_at} \
             observed_at={observed_at} now={now} records={} — 推送必须带标注",
            batch.records.len(),
        );
    }
    Ok(batch)
}

/// as_optional_u32: JSON null → None; 缺失 → Err fail-closed。
fn as_optional_u32(
    v: &Value,
    key: &str,
    capability: &'static str,
) -> Result<Option<u32>, GatewayError> {
    match v.get(key) {
        None => Err(err(capability, format!("字段 {key} 缺失"))),
        Some(Value::Null) => Ok(None),
        Some(x) => Ok(Some(
            x.as_u64()
                .ok_or_else(|| err(capability, format!("字段 {key} 非整数")))? as u32,
        )),
    }
}

/// Debug 名 → GeneralWebResearchProvider。服务端 pack_ev 用 format!("{:?}", provider)
/// 写 JSON ("Bocha"/"Tavily"/"SerpApi" — 不在 ProviderId 解析表内, 不能复用 parse_provider)。
/// 未知/空 → Err (fail-closed)。
fn parse_general_web_provider(s: &str) -> Result<GeneralWebResearchProvider, GatewayError> {
    GeneralWebResearchProvider::from_wire_name(s).ok_or_else(|| {
        err(
            "SemanticSearch",
            format!("selected_provider 无法解析为 GeneralWebResearchProvider: {s}"),
        )
    })
}

/// BR-242 语义检索桥。视图: delegate.rs fetch_semantic_search — 服务端
/// records = serde_json::to_value(GeneralWebResearchRecord) 直出 (snake_case serde,
/// 含 record 级 evidence 子对象), selected_provider = Debug 名。批级 evidence
/// 客户端重建 (query 客户端已知; use_scope 恒 ResearchOnly — 本地 admit_records 语义)。
/// record 级 wire 完整性: serde round-trip + evidence 归属
/// (batch_id/provider/observed_at) 与批级一致。
pub fn semantic_search(
    q: &QueryResult,
    query: &str,
    requested_provider: GeneralWebResearchProvider,
    requested_limit: usize,
) -> Result<GeneralWebResearchBatch, GatewayError> {
    let capability = "SemanticSearch";
    if !(1..=50).contains(&requested_limit) {
        return Err(GatewayError::invalid_request(
            capability,
            format!("requested limit must be within 1..=50, got {requested_limit}"),
        ));
    }
    if q.admission != crate::grpc_client::pb::magic::market::v1::AdmissionState::Admitted {
        return Err(err(capability, "响应未获 repository admission"));
    }
    if !q.diagnostic_blocker.is_empty() {
        return Err(err(
            capability,
            "响应携带 diagnostic_blocker, 不得进入生产证据转换",
        ));
    }
    let provider = parse_general_web_provider(&q.selected_provider)?;
    if provider != requested_provider {
        return Err(err(
            capability,
            format!(
                "response provider {} differs from requested provider {}",
                provider.label(),
                requested_provider.label()
            ),
        ));
    }
    if q.source != requested_provider.source() {
        return Err(err(
            capability,
            format!(
                "response source {:?} differs from requested provider source {:?}",
                q.source,
                requested_provider.source()
            ),
        ));
    }
    if q.source.is_empty() {
        return Err(err(capability, "source 空 (服务端未回填证据链)"));
    }
    if q.batch_id.is_empty() {
        return Err(err(capability, "batch_id 空 (服务端未回填证据链)"));
    }
    let evidence = GeneralWebResearchBatchEvidence {
        provider,
        source: q.source.clone(),
        query: query.to_string(),
        // SemanticSearch is not a financial ProviderId route. Its delegate
        // serializes GeneralWebResearchEvidence.observed_at with to_rfc3339(),
        // so parse that frozen wire field directly instead of routing Bocha
        // through the unrelated ProviderId parser.
        observed_at: DateTime::parse_from_rfc3339(&q.observed_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|error| {
                err(
                    capability,
                    format!("observed_at 非 RFC3339: {} ({error})", q.observed_at),
                )
            })?,
        batch_id: q.batch_id.clone(),
        use_scope: ResearchUseScope::ResearchOnly,
    };
    let parsed = parse_records(q, capability)?;
    if parsed.len() > requested_limit {
        return Err(err(
            capability,
            format!(
                "response record count {} exceeds requested limit {requested_limit}",
                parsed.len()
            ),
        ));
    }
    if parsed.is_empty() {
        return Ok(GeneralWebResearchBatch::VerifiedEmpty(evidence));
    }
    let records: Vec<GeneralWebResearchRecord> = parsed
        .iter()
        .map(|v| {
            let record: GeneralWebResearchRecord = serde_json::from_value(v.clone())
                .map_err(|e| err(capability, format!("记录非 GeneralWebResearchRecord: {e}")))?;
            if record.evidence.batch_id != evidence.batch_id {
                return Err(err(capability, "记录 evidence.batch_id 与批级不一致"));
            }
            if record.evidence.provider != evidence.provider {
                return Err(err(capability, "记录 evidence.provider 与批级不一致"));
            }
            if record.evidence.observed_at != evidence.observed_at {
                return Err(err(capability, "记录 evidence.observed_at 与批级不一致"));
            }
            Ok(record)
        })
        .collect::<Result<_, _>>()?;
    Ok(GeneralWebResearchBatch::Available { records, evidence })
}

/// 公司行动桥。视图: delegate.rs fetch_corporate_actions (:1132)
/// {"code","category"(Debug 名),"effective_on","record_on","ex_on","payable_on",
/// "terms"(serde_json::to_value(CorporateActionTerms))}。
/// 注意: 视图无法区分 Available-空 与 VerifiedEmpty (服务端只回 state.records());
/// 消费方 (historical_bars.rs:1156-1173) 对两者同等对待 — 仅 Unavailable 失败,
/// 与 evidence_of 的 evidence 存在性语义一致 → 空批统一映射 VerifiedEmpty。
pub fn corporate_actions(
    q: &QueryResult,
) -> Result<GatewayBatch<ImplementedCorporateAction>, GatewayError> {
    let capability = "CorporateActions";
    let (parsed, ev) = parse_records_parts(q, capability)?;
    if parsed.is_empty() {
        return Ok(GatewayBatch::VerifiedEmpty(ev));
    }
    let records: Vec<ImplementedCorporateAction> = parsed
        .iter()
        .map(|v| {
            let category: CorporateActionCategory = serde_json::from_value(
                v.get("category")
                    .cloned()
                    .ok_or_else(|| err(capability, "字段 category 缺失"))?,
            )
            .map_err(|e| err(capability, format!("category 非合法行动类别: {e}")))?;
            let terms: CorporateActionTerms = serde_json::from_value(
                v.get("terms")
                    .cloned()
                    .ok_or_else(|| err(capability, "字段 terms 缺失"))?,
            )
            .map_err(|e| err(capability, format!("terms 非合法条款: {e}")))?;
            Ok(ImplementedCorporateAction {
                code: as_str(v, "code", capability)?,
                category,
                effective_on: as_date(v, "effective_on", capability)?,
                record_on: as_optional_date(v, "record_on", capability)?,
                ex_on: as_optional_date(v, "ex_on", capability)?,
                payable_on: as_optional_date(v, "payable_on", capability)?,
                terms,
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(GatewayBatch::Available {
        records,
        evidence: ev,
    })
}

/// outcome 错误 wire 分类 → 静态常量对。服务端只发 review.rs GatewayError 构造器
/// 集合 (unavailable/partial/invalid_request/audit_failure + map_tdx_error 的
/// provider_* 对); 未知对 = 版本偏移 → 兜底 ("unavailable", "no_verified_batch")
/// 保持 fail-closed 重试语义 (retryable 由 wire 原样重建, 不受影响)。
fn rebuild_outcome_classification(
    audit_outcome: &str,
    reason_code: &str,
) -> (&'static str, &'static str) {
    match (audit_outcome, reason_code) {
        ("partial", "invalid_evidence") => ("partial", "invalid_evidence"),
        ("invalid_request", "invalid_request") => ("invalid_request", "invalid_request"),
        ("invalid_request", "unsupported_window") => ("invalid_request", "unsupported_window"),
        ("partial", "provider_invalid_data") => ("partial", "provider_invalid_data"),
        ("unavailable", "provider_unavailable") => ("unavailable", "provider_unavailable"),
        ("unavailable", "acquisition_audit_unavailable") => {
            ("unavailable", "acquisition_audit_unavailable")
        }
        _ => ("unavailable", "no_verified_batch"),
    }
}

/// outcome 复盘日线 (P4 M3): 服务端 adaptive 抓取完整视图
/// {"batch": DataBatch<Bar> 直出, "attempts": [Preimage], "error": {...}|null}。
/// batch/attempts 双向 serde 保真重建 (Bar 有 manual Deserialize, provider.rs:1469);
/// error 视图 → GatewayError::classified 重建 (capability 恒为本网关静态
/// "OutcomeDailyBarsV2", provider 经 parse_provider 回映, 分类经
/// rebuild_outcome_classification)。
pub fn outcome_daily_bars(q: &QueryResult) -> Result<RawOutcomeFetch, OutcomeTransportFailure> {
    let capability = "OutcomeDailyBarsV2";
    let payload = q.records.first().ok_or_else(|| {
        OutcomeTransportFailure::new(
            err(capability, "records 空 (服务端无 canonical payload)"),
            Vec::new(),
        )
    })?;
    let parsed: Value = serde_json::from_slice(&payload.data).map_err(|e| {
        OutcomeTransportFailure::new(err(capability, format!("视图非 JSON: {e}")), Vec::new())
    })?;
    let view = parsed.as_object().ok_or_else(|| {
        OutcomeTransportFailure::new(err(capability, "outcome 视图不是对象"), Vec::new())
    })?;
    let attempts = serde_json::from_value::<Vec<OutcomeTransportAttemptPreimage>>(
        view.get("attempts").cloned().unwrap_or(Value::Null),
    )
    .map_err(|e| {
        OutcomeTransportFailure::new(
            err(capability, format!("attempts 重建失败: {e}")),
            Vec::new(),
        )
    })?;
    if let Some(error_view) = view.get("error") {
        if !error_view.is_null() {
            let provider = error_view
                .get("provider")
                .and_then(Value::as_str)
                .and_then(|s| parse_provider(s).ok());
            let (audit_outcome, reason_code) = rebuild_outcome_classification(
                error_view
                    .get("audit_outcome")
                    .and_then(Value::as_str)
                    .unwrap_or("unavailable"),
                error_view
                    .get("reason_code")
                    .and_then(Value::as_str)
                    .unwrap_or("no_verified_batch"),
            );
            let retryable = error_view
                .get("retryable")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let message = error_view
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("(no message)")
                .to_string();
            return Err(OutcomeTransportFailure::new(
                GatewayError::classified(
                    capability,
                    provider,
                    audit_outcome,
                    reason_code,
                    retryable,
                    message,
                ),
                attempts,
            ));
        }
    }
    let batch =
        serde_json::from_value::<DataBatch<Bar>>(view.get("batch").cloned().ok_or_else(|| {
            OutcomeTransportFailure::new(err(capability, "outcome 视图缺 batch"), Vec::new())
        })?)
        .map_err(|e| {
            OutcomeTransportFailure::new(
                err(capability, format!("batch 重建失败: {e}")),
                Vec::new(),
            )
        })?;
    Ok(RawOutcomeFetch { batch, attempts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc_client::pb::magic::market::v1::{AdmissionState, CanonicalPayload};
    use chrono::TimeZone;

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
            diagnostic_blocker: String::new(),
        }
    }

    fn benchmark_request() -> crate::data_gateway::BenchmarkRequest {
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        crate::data_gateway::BenchmarkRequest {
            instrument: "TEST_CODE_000300".to_owned(),
            range: crate::data_gateway::BenchmarkRange::Daily { from: day, to: day },
        }
    }

    fn benchmark_query() -> QueryResult {
        let request = benchmark_request();
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        let audited = crate::data_gateway::review::AuditedBenchmarkBatch {
            batch: crate::data_gateway::GatewayBatch::Available {
                records: vec![crate::data_gateway::BenchmarkBar {
                    at: crate::data_gateway::BenchmarkBarTime::Daily(day),
                    open: 3_500.0,
                    high: 3_510.0,
                    low: 3_490.0,
                    close: 3_505.0,
                    volume: None,
                    amount: Some(8_000.0),
                }],
                evidence: crate::data_gateway::BatchEvidence {
                    provider: ProviderId::Tdx,
                    source: "TEST_CODE_magic-tdx-index-bars".to_owned(),
                    source_at: None,
                    observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
                    batch_id: "TEST_CODE_benchmark_batch".to_owned(),
                },
            },
            receipt: crate::database::data_acquisition_audit::DataAcquisitionAuditReceipt {
                audit_id: 17,
                record_hash: "a".repeat(64),
                previous_outcome: None,
                current_outcome: "available".to_owned(),
            },
            request_hash: crate::data_gateway::benchmark::canonical_base_request_hash(&request),
        };
        let wire = crate::data_gateway::grpc_source::BenchmarkGrpcResponseWire::from_audited(
            &request, &audited,
        )
        .expect("TEST_CODE benchmark wire");
        QueryResult {
            admission: AdmissionState::Admitted,
            selected_provider: "Tdx".to_owned(),
            batch_id: "TEST_CODE_benchmark_batch".to_owned(),
            complete: true,
            observed_at: "2026-08-21T15:01:00+08:00".to_owned(),
            source_at: String::new(),
            records: vec![CanonicalPayload {
                schema: "market.benchmark_bars".to_owned(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_owned(),
                data: serde_json::to_vec(&wire).expect("TEST_CODE benchmark JSON"),
            }],
            source: "TEST_CODE_magic-tdx-index-bars".to_owned(),
            diagnostic_blocker: String::new(),
        }
    }

    fn mutate_benchmark_query(
        query: &mut QueryResult,
        mutate: impl FnOnce(&mut serde_json::Value),
    ) {
        let mut wire: serde_json::Value =
            serde_json::from_slice(&query.records[0].data).expect("TEST_CODE benchmark JSON");
        mutate(&mut wire);
        query.records[0].data =
            serde_json::to_vec(&wire).expect("mutated TEST_CODE benchmark JSON");
    }

    fn admit_test_benchmark(
        query: &QueryResult,
    ) -> Result<crate::data_gateway::review::AuditedBenchmarkBatch, GatewayError> {
        let day = NaiveDate::from_ymd_opt(2026, 8, 21).expect("TEST_CODE date");
        benchmark_bars_for_test(
            &benchmark_request(),
            query,
            crate::data_gateway::BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
            DateTime::parse_from_rfc3339("2026-08-21T15:01:01+08:00")
                .expect("TEST_CODE consumer time")
                .with_timezone(&Utc),
        )
    }

    #[test]
    fn benchmark_wire_rejects_missing_nullable_fields() {
        for (section, field) in [
            ("bar", "volume"),
            ("bar", "amount"),
            ("evidence", "source_at"),
            ("receipt", "previous_outcome"),
        ] {
            let mut query = benchmark_query();
            mutate_benchmark_query(&mut query, |wire| {
                let object = match section {
                    "bar" => wire["bars"][0].as_object_mut().expect("bar object"),
                    "evidence" => wire["evidence"].as_object_mut().expect("evidence object"),
                    "receipt" => wire["receipt"].as_object_mut().expect("receipt object"),
                    _ => unreachable!(),
                };
                object.remove(field);
            });
            let error = admit_test_benchmark(&query)
                .expect_err("missing nullable wire field must fail closed");
            assert_eq!(error.reason_code(), "invalid_evidence", "{section}.{field}");
        }
    }

    #[test]
    fn benchmark_wire_rejects_unknown_provider_and_inner_evidence_conflict() {
        let mut unknown = benchmark_query();
        unknown.selected_provider = "TEST_CODE_unknown_provider".to_owned();
        mutate_benchmark_query(&mut unknown, |wire| {
            wire["evidence"]["provider"] =
                serde_json::Value::String("TEST_CODE_unknown_provider".to_owned());
        });
        assert_eq!(
            admit_test_benchmark(&unknown)
                .expect_err("unknown provider must fail closed")
                .reason_code(),
            "invalid_evidence"
        );

        let mut conflict = benchmark_query();
        mutate_benchmark_query(&mut conflict, |wire| {
            wire["evidence"]["batch_id"] =
                serde_json::Value::String("TEST_CODE_conflicting_batch".to_owned());
        });
        assert_eq!(
            admit_test_benchmark(&conflict)
                .expect_err("inner evidence conflict must fail closed")
                .reason_code(),
            "invalid_evidence"
        );
    }

    #[test]
    fn benchmark_wire_rejects_impossible_or_future_evidence_times() {
        let mut inverted = benchmark_query();
        inverted.source_at = "2026-08-21T15:02:00+08:00".to_owned();
        mutate_benchmark_query(&mut inverted, |wire| {
            wire["evidence"]["source_at"] = serde_json::json!("2026-08-21T15:02:00+08:00");
        });
        assert_eq!(
            admit_test_benchmark(&inverted)
                .expect_err("source_at after observed_at must fail closed")
                .reason_code(),
            "invalid_evidence"
        );

        let mut future = benchmark_query();
        future.observed_at = "2099-08-21T15:01:00+08:00".to_owned();
        mutate_benchmark_query(&mut future, |wire| {
            wire["evidence"]["observed_at"] = serde_json::json!("2099-08-21T15:01:00+08:00");
        });
        assert_eq!(
            admit_test_benchmark(&future)
                .expect_err("future observed_at beyond clock skew must fail closed")
                .reason_code(),
            "invalid_evidence"
        );
    }

    #[test]
    fn benchmark_wire_rejects_malformed_receipt_and_request_echo() {
        for (field, value) in [
            ("audit_id", serde_json::json!(0)),
            ("record_hash", serde_json::json!("TEST_CODE_short_hash")),
            ("current_outcome", serde_json::json!("unavailable")),
            (
                "previous_outcome",
                serde_json::json!("TEST_CODE_unknown_outcome"),
            ),
        ] {
            let mut query = benchmark_query();
            mutate_benchmark_query(&mut query, |wire| wire["receipt"][field] = value);
            assert_eq!(
                admit_test_benchmark(&query)
                    .expect_err("malformed receipt must fail closed")
                    .reason_code(),
                "invalid_evidence",
                "receipt.{field}"
            );
        }

        for (field, value) in [
            ("instrument", serde_json::json!("TEST_CODE_OTHER_INDEX")),
            (
                "from",
                serde_json::json!({"kind":"daily","year":2026,"month":8,"day":20}),
            ),
        ] {
            let mut query = benchmark_query();
            mutate_benchmark_query(&mut query, |wire| wire["request"][field] = value);
            assert_eq!(
                admit_test_benchmark(&query)
                    .expect_err("request echo mismatch must fail closed")
                    .reason_code(),
                "invalid_evidence",
                "request.{field}"
            );
        }
    }

    #[test]
    fn benchmark_wire_reexecutes_whole_batch_admission() {
        let mut invalid_ohlc = benchmark_query();
        mutate_benchmark_query(&mut invalid_ohlc, |wire| {
            wire["bars"][0]["close"] = serde_json::json!(0.0);
        });
        assert_eq!(
            admit_test_benchmark(&invalid_ohlc)
                .expect_err("client admission must reject invalid OHLC")
                .reason_code(),
            "benchmark_ohlc_not_positive_finite"
        );

        let mut empty = benchmark_query();
        mutate_benchmark_query(&mut empty, |wire| wire["bars"] = serde_json::json!([]));
        assert_eq!(
            admit_test_benchmark(&empty)
                .expect_err("client admission must reject an empty benchmark payload")
                .reason_code(),
            "benchmark_batch_empty"
        );

        let mut granularity_conflict = benchmark_query();
        mutate_benchmark_query(&mut granularity_conflict, |wire| {
            wire["request"]["granularity"] = serde_json::json!("Minute1");
        });
        assert_eq!(
            admit_test_benchmark(&granularity_conflict)
                .expect_err("request granularity and time shape must agree")
                .reason_code(),
            "invalid_evidence"
        );
    }

    fn p01_limit_pools_q() -> QueryResult {
        let mut query = mk_q("[]", "Eastmoney", "TEST_CODE_eastmoney_limit_pool");
        query.batch_id = "TEST_CODE_LIMIT_POOL_BATCH".to_string();
        query.source_at = "2026-08-18".to_string();
        query.observed_at = "2026-08-18T10:03:00+08:00".to_string();
        query.records[0].schema = "market.limit_pools".to_string();
        query.records[0].data = serde_json::to_vec(&serde_json::json!([{
            "kind": "Upper",
            "instrument": {
                "exchange": "Shanghai",
                "code": "TEST_CODE_600001",
                "asset_class": "Equity"
            },
            "trading_date": "2026-08-18",
            "price": 10.25,
            "change": {"value": 10.0, "unit": "Percent"},
            "volume": 12300.0,
            "turnover": {"value": 3.5, "unit": "Percent"},
            "sealed_amount": 8800000.0,
            "first_seal_at": "2026-08-18T09:31:00+08:00",
            "last_seal_at": "2026-08-18T10:02:00+08:00",
            "break_count": 1,
            "streak": 2,
            "industry": "TEST_CODE industry",
            "board_name": "TEST_CODE board",
            "seal_state": "TEST_CODE sealed",
            "reseal_count": 1,
            "reason": "TEST_CODE reason",
            "evidence": {
                "provider": "Eastmoney",
                "source_at": "2026-08-18",
                "observed_at": "2026-08-18T10:03:00+08:00",
                "batch_id": "TEST_CODE_LIMIT_POOL_BATCH"
            }
        }]))
        .expect("TEST_CODE canonical LimitPools JSON");
        query
    }

    #[test]
    fn p01_limit_pools_roundtrips_complete_entry_and_original_evidence() {
        let query = p01_limit_pools_q();
        let date = NaiveDate::from_ymd_opt(2026, 8, 18).expect("TEST_CODE date");
        let batch = limit_pools(&query, date).expect("complete exact-date LimitPools batch");
        let record = &batch.records()[0];

        assert_eq!(record.kind, LimitPoolKind::Upper);
        assert_eq!(record.instrument.code(), "TEST_CODE_600001");
        assert_eq!(record.trading_date.as_str(), "2026-08-18");
        assert_eq!(record.price.get(), 10.25);
        assert_eq!(record.change.get(), 10.0);
        assert_eq!(record.volume.map(|value| value.get()), Some(12_300.0));
        assert_eq!(record.break_count, Some(1));
        assert_eq!(record.streak.map(|value| value.get()), Some(2));
        assert_eq!(record.evidence.provider(), ProviderId::Eastmoney);
        assert_eq!(record.evidence.source_at(), Some("2026-08-18"));
        assert_eq!(record.evidence.observed_at(), "2026-08-18T10:03:00+08:00");
        assert_eq!(record.evidence.batch_id(), "TEST_CODE_LIMIT_POOL_BATCH");
        assert_eq!(batch.evidence().source, "TEST_CODE_eastmoney_limit_pool");
    }

    #[test]
    fn p01_limit_pools_rejects_contract_value_and_evidence_conflicts() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 18).expect("TEST_CODE date");
        let mut cases = Vec::new();

        let mut partial = p01_limit_pools_q();
        partial.complete = false;
        cases.push(("partial", partial));

        let mut wrong_schema = p01_limit_pools_q();
        wrong_schema.records[0].schema = "market.limit_pools.v2".to_string();
        cases.push(("schema", wrong_schema));

        let mut wrong_version = p01_limit_pools_q();
        wrong_version.records[0].schema_version = 2;
        cases.push(("version", wrong_version));

        let mut wrong_content_type = p01_limit_pools_q();
        wrong_content_type.records[0].content_type = "application/json".to_string();
        cases.push(("content_type", wrong_content_type));

        let mut missing_field = p01_limit_pools_q();
        let mut missing_value: Value =
            serde_json::from_slice(&missing_field.records[0].data).expect("TEST_CODE JSON");
        missing_value[0]
            .as_object_mut()
            .expect("TEST_CODE entry")
            .remove("reason");
        missing_field.records[0].data = serde_json::to_vec(&missing_value).expect("TEST_CODE JSON");
        cases.push(("missing_field", missing_field));

        let mut extra_field = p01_limit_pools_q();
        let mut extra_value: Value =
            serde_json::from_slice(&extra_field.records[0].data).expect("TEST_CODE JSON");
        extra_value[0]["unexpected"] = serde_json::json!(true);
        extra_field.records[0].data = serde_json::to_vec(&extra_value).expect("TEST_CODE JSON");
        cases.push(("extra_field", extra_field));

        for (label, field, value) in [
            ("kind", "kind", serde_json::json!("Broken")),
            (
                "trading_date",
                "trading_date",
                serde_json::json!("2026-08-17"),
            ),
            ("price", "price", serde_json::json!(0.0)),
            ("change", "change", serde_json::json!("not-a-ratio")),
        ] {
            let mut query = p01_limit_pools_q();
            let mut payload: Value =
                serde_json::from_slice(&query.records[0].data).expect("TEST_CODE JSON");
            payload[0][field] = value;
            query.records[0].data = serde_json::to_vec(&payload).expect("TEST_CODE JSON");
            cases.push((label, query));
        }

        for (label, field, value) in [
            (
                "record_provider",
                "provider",
                serde_json::json!("Tonghuashun"),
            ),
            (
                "record_source_at",
                "source_at",
                serde_json::json!("2026-08-17"),
            ),
            (
                "record_observed_at",
                "observed_at",
                serde_json::json!("2026-08-18T10:04:00+08:00"),
            ),
            (
                "record_batch_id",
                "batch_id",
                serde_json::json!("TEST_CODE_OTHER_BATCH"),
            ),
        ] {
            let mut query = p01_limit_pools_q();
            let mut payload: Value =
                serde_json::from_slice(&query.records[0].data).expect("TEST_CODE JSON");
            payload[0]["evidence"][field] = value;
            query.records[0].data = serde_json::to_vec(&payload).expect("TEST_CODE JSON");
            cases.push((label, query));
        }

        let mut duplicate = p01_limit_pools_q();
        let mut duplicate_value: Value =
            serde_json::from_slice(&duplicate.records[0].data).expect("TEST_CODE JSON");
        let duplicate_record = duplicate_value[0].clone();
        duplicate_value
            .as_array_mut()
            .expect("TEST_CODE array")
            .push(duplicate_record);
        duplicate.records[0].data = serde_json::to_vec(&duplicate_value).expect("TEST_CODE JSON");
        cases.push(("duplicate", duplicate));

        let mut over_limit = p01_limit_pools_q();
        let record: Value = serde_json::from_slice::<Value>(&over_limit.records[0].data)
            .expect("TEST_CODE JSON")[0]
            .clone();
        over_limit.records[0].data =
            serde_json::to_vec(&vec![record; 201]).expect("TEST_CODE JSON");
        cases.push(("count", over_limit));

        let mut wrong_envelope_source_at = p01_limit_pools_q();
        wrong_envelope_source_at.source_at = "2026-08-17".to_string();
        cases.push(("envelope_source_at", wrong_envelope_source_at));

        let mut invalid_envelope_observed_at = p01_limit_pools_q();
        invalid_envelope_observed_at.observed_at = "not-an-instant".to_string();
        cases.push(("envelope_observed_at", invalid_envelope_observed_at));

        let mut unregistered_provider = p01_limit_pools_q();
        unregistered_provider.selected_provider = "Tdx".to_string();
        cases.push(("envelope_provider", unregistered_provider));

        let mut multiple_payloads = p01_limit_pools_q();
        multiple_payloads
            .records
            .push(multiple_payloads.records[0].clone());
        cases.push(("payload_count", multiple_payloads));

        for (label, query) in cases {
            let error = limit_pools(&query, date)
                .expect_err("invalid LimitPools contract must fail closed");
            assert_eq!(error.reason_code(), "invalid_evidence", "case={label}");
        }
    }

    #[test]
    fn p01_limit_pools_accepts_only_proven_complete_empty() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 18).expect("TEST_CODE date");
        let mut empty = p01_limit_pools_q();
        empty.records[0].data = b"[]".to_vec();
        assert!(matches!(
            limit_pools(&empty, date).expect("complete empty is provider-verified"),
            GatewayBatch::VerifiedEmpty(_)
        ));

        empty.complete = false;
        assert!(limit_pools(&empty, date).is_err());
    }

    fn provider_top_n_row(
        metric: &str,
        ordinal: u64,
        code: &str,
        unit: &str,
        batch_id: &str,
        observed_at: &str,
    ) -> Value {
        serde_json::json!({
            "metric": metric,
            "ordinal": ordinal,
            "code": code,
            "label": format!("TEST_CODE_{metric}_{ordinal}"),
            "value": 1.0,
            "unit": unit,
            "trading_date": "2026-08-17",
            "filter_identity": "TEST_CODE_A_SHARE_FILTER",
            "provider_declared_total": 2,
            "inspected_row_count": 2,
            "evidence": {
                "provider": "Eastmoney",
                "source": "eastmoney-web",
                "source_at": null,
                "observed_at": observed_at,
                "batch_id": batch_id,
            },
        })
    }

    fn provider_top_n_pair_q() -> QueryResult {
        let volume_observed_at = "2026-08-17T15:36:00+08:00";
        let rows = serde_json::json!([
            provider_top_n_row(
                "VolumeRatio",
                1,
                "600001",
                "Multiple",
                "TEST_CODE_REAL_VOLUME_BATCH",
                volume_observed_at,
            ),
            provider_top_n_row(
                "MainNetInflow",
                1,
                "600002",
                "Yuan",
                "TEST_CODE_REAL_INFLOW_BATCH",
                "2026-08-17T15:36:01+08:00",
            ),
        ]);
        let mut query = mk_q(&rows.to_string(), "Eastmoney", "eastmoney-web");
        query.batch_id = "TEST_CODE_REAL_VOLUME_BATCH".to_string();
        query.source_at.clear();
        query.observed_at = volume_observed_at.to_string();
        query
    }

    #[test]
    fn br240_provider_top_n_pair_preserves_two_real_batch_evidences() {
        let (volume, inflow) =
            provider_top_n_pair(&provider_top_n_pair_q()).expect("two real metric batches");

        assert_eq!(volume.evidence().batch_id, "TEST_CODE_REAL_VOLUME_BATCH");
        assert_eq!(inflow.evidence().batch_id, "TEST_CODE_REAL_INFLOW_BATCH");
        assert_eq!(volume.evidence().source, "eastmoney-web");
        assert_eq!(inflow.evidence().source, "eastmoney-web");
        assert_eq!(volume.evidence().source_at, None);
        assert_eq!(inflow.evidence().source_at, None);
        assert_eq!(volume.evidence().observed_at, "2026-08-17T15:36:00+08:00");
        assert_eq!(inflow.evidence().observed_at, "2026-08-17T15:36:01+08:00");
        assert_ne!(volume.evidence().batch_id, inflow.evidence().batch_id);
    }

    #[test]
    fn br240_provider_top_n_pair_rejects_invalid_evidence_matrix() {
        let mut missing = provider_top_n_pair_q();
        let mut missing_rows: Value = serde_json::from_slice(&missing.records[0].data).unwrap();
        missing_rows[0]
            .as_object_mut()
            .expect("record object")
            .remove("evidence");
        missing.records[0].data = serde_json::to_vec(&missing_rows).unwrap();

        let mut mixed = provider_top_n_pair_q();
        let mut mixed_rows: Value = serde_json::from_slice(&mixed.records[0].data).unwrap();
        mixed_rows
            .as_array_mut()
            .expect("rows array")
            .push(provider_top_n_row(
                "VolumeRatio",
                2,
                "600003",
                "Multiple",
                "TEST_CODE_MIXED_VOLUME_BATCH",
                "2026-08-17T15:36:00+08:00",
            ));
        mixed.records[0].data = serde_json::to_vec(&mixed_rows).unwrap();

        let mut same = provider_top_n_pair_q();
        let mut same_rows: Value = serde_json::from_slice(&same.records[0].data).unwrap();
        same_rows[1]["evidence"]["batch_id"] =
            Value::String("TEST_CODE_REAL_VOLUME_BATCH".to_string());
        same.records[0].data = serde_json::to_vec(&same_rows).unwrap();

        let mut envelope_conflict = provider_top_n_pair_q();
        envelope_conflict.batch_id = "TEST_CODE_CONFLICTING_ENVELOPE_BATCH".to_string();

        let mut provider_conflict = provider_top_n_pair_q();
        let mut provider_rows: Value =
            serde_json::from_slice(&provider_conflict.records[0].data).unwrap();
        provider_rows[1]["evidence"]["provider"] = Value::String("Sina".to_string());
        provider_conflict.records[0].data = serde_json::to_vec(&provider_rows).unwrap();

        let mut source_conflict = provider_top_n_pair_q();
        let mut source_rows: Value =
            serde_json::from_slice(&source_conflict.records[0].data).unwrap();
        source_rows[1]["evidence"]["source"] = Value::String("TEST_CODE_OTHER_SOURCE".to_string());
        source_conflict.records[0].data = serde_json::to_vec(&source_rows).unwrap();

        let mut trading_date_conflict = provider_top_n_pair_q();
        let mut trading_date_rows: Value =
            serde_json::from_slice(&trading_date_conflict.records[0].data).unwrap();
        trading_date_rows[1]["trading_date"] = Value::String("2026-08-14".to_string());
        trading_date_conflict.records[0].data = serde_json::to_vec(&trading_date_rows).unwrap();

        let mut future_source_date = provider_top_n_pair_q();
        let mut future_rows: Value =
            serde_json::from_slice(&future_source_date.records[0].data).unwrap();
        future_rows[0]["trading_date"] = Value::String("2026-08-18".to_string());
        future_rows[1]["trading_date"] = Value::String("2026-08-18".to_string());
        future_source_date.records[0].data = serde_json::to_vec(&future_rows).unwrap();

        let mut source_at_date_conflict = provider_top_n_pair_q();
        let mut source_at_rows: Value =
            serde_json::from_slice(&source_at_date_conflict.records[0].data).unwrap();
        source_at_rows[1]["evidence"]["source_at"] =
            Value::String("2026-08-16T15:36:00+08:00".to_string());
        source_at_date_conflict.records[0].data = serde_json::to_vec(&source_at_rows).unwrap();

        for (case, query) in [
            ("missing", missing),
            ("mixed", mixed),
            ("same", same),
            ("envelope_conflict", envelope_conflict),
            ("provider_conflict", provider_conflict),
            ("source_conflict", source_conflict),
            ("trading_date_conflict", trading_date_conflict),
            ("future_source_date", future_source_date),
            ("source_at_date_conflict", source_at_date_conflict),
        ] {
            let error = provider_top_n_pair(&query)
                .expect_err("invalid per-metric evidence must fail closed");
            assert_eq!(error.reason_code(), "invalid_evidence", "case={case}");
            assert!(!error.retryable(), "case={case}");
        }
    }

    #[test]
    fn br238_local_array_contract_rejects_multiple_payloads() {
        let mut query = mk_q("[]", "Tdx", "TEST_CODE_source");
        query.records.push(query.records[0].clone());
        let error = parse_records(&query, "TEST_CODE_capability")
            .expect_err("additional local payloads must not be silently ignored");
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    fn external_identity_record(code: &str, listed_and_limit_fields: bool) -> CanonicalPayload {
        let mut data = serde_json::json!({
            "instrument": {
                "exchange": "Shanghai",
                "code": code,
                "asset_class": "Equity"
            },
            "name": format!("TEST_CODE_name_{code}"),
            "board": "Main",
            "is_st": false,
            "status": "Unavailable",
            "source_at": "1786931999.125000000",
            "observed_at": "1786932000.250000000",
            "provider": "Tencent",
            "batch_id": "TEST_CODE_external_security_batch"
        });
        if listed_and_limit_fields {
            let object = data.as_object_mut().expect("fixture object");
            object.insert("listed_on".to_string(), Value::Null);
            object.insert(
                "price_limit".to_string(),
                serde_json::json!({"percent": null, "version": null}),
            );
        }
        CanonicalPayload {
            schema: "magic.market.security_metadata".to_string(),
            schema_version: 1,
            content_type: "application/json; charset=utf-8".to_string(),
            data: serde_json::to_vec(&data).expect("fixture JSON"),
        }
    }

    fn external_identity_q() -> QueryResult {
        QueryResult {
            admission: AdmissionState::Admitted,
            selected_provider: "Tencent".to_string(),
            batch_id: "TEST_CODE_external_security_batch".to_string(),
            complete: false,
            observed_at: "1786932000.250000000".to_string(),
            source_at: "1786931999.125000000".to_string(),
            records: vec![
                external_identity_record("TEST_CODE_SECURITY_001", true),
                external_identity_record("TEST_CODE_SECURITY_002", false),
            ],
            source: "grpc-mtls:TEST_CODE_market.local".to_string(),
            diagnostic_blocker: String::new(),
        }
    }

    fn external_identity_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-17T10:00:20+08:00")
            .expect("fixed external identity clock")
            .with_timezone(&Utc)
    }

    fn external_identity_requested() -> Vec<String> {
        vec![
            "TEST_CODE_SECURITY_001".to_string(),
            "TEST_CODE_SECURITY_002".to_string(),
        ]
    }

    fn external_news_q() -> QueryResult {
        QueryResult {
            admission: AdmissionState::Admitted,
            selected_provider: "Sina".to_string(),
            batch_id: "TEST_CODE_external_news_batch".to_string(),
            complete: true,
            observed_at: "2026-08-17T10:00:01+08:00".to_string(),
            source_at: "2026-08-17T09:59:00+08:00".to_string(),
            records: vec![CanonicalPayload {
                schema: "magic.market.news_item".to_string(),
                schema_version: 2,
                content_type: "application/json; charset=utf-8".to_string(),
                data: serde_json::to_vec(&serde_json::json!({
                    "item_id": "https://example.com/TEST_CODE_news_1",
                    "title": "TEST_CODE 产业链新闻",
                    "summary": null,
                    "content": null,
                    "publisher": "TEST_CODE publisher",
                    "url": "https://example.com/TEST_CODE_news_1",
                    "published_at": "2026-08-17T09:59:00+08:00",
                    "instruments": [{
                        "exchange": "Shanghai",
                        "code": "TEST_CODE_600396",
                        "asset_class": "Equity"
                    }],
                    "topics": [],
                    "language": "zh-CN",
                    "evidence": {
                        "provider": "Sina",
                        "source_at": "2026-08-17T09:59:00+08:00",
                        "observed_at": "2026-08-17T10:00:01+08:00",
                        "batch_id": "TEST_CODE_external_news_batch"
                    }
                }))
                .expect("external news fixture JSON"),
            }],
            source: "grpc-mtls:TEST_CODE_market.local".to_string(),
            diagnostic_blocker: String::new(),
        }
    }

    fn external_global_news_v2_q() -> QueryResult {
        QueryResult {
            admission: AdmissionState::Admitted,
            selected_provider: "Jin10".to_string(),
            batch_id: "TEST_CODE_external_global_news_batch".to_string(),
            complete: true,
            observed_at: "1787127606.533354000".to_string(),
            source_at: "2026-08-19 16:15:37".to_string(),
            records: vec![CanonicalPayload {
                schema: "magic.market.news_item".to_string(),
                schema_version: 2,
                content_type: "application/json; charset=utf-8".to_string(),
                data: serde_json::to_vec(&serde_json::json!({
                    "item_id": "TEST_CODE_GLOBAL_NEWS_001",
                    "title": "TEST_CODE global news",
                    "summary": null,
                    "content": null,
                    "publisher": "Jin10",
                    "url": "https://example.com/TEST_CODE_GLOBAL_NEWS_001",
                    "published_at": "2026-08-19T16:15:37+08:00",
                    "instruments": [],
                    "topics": ["TEST_CODE_topic"],
                    "language": "zh-CN",
                    "evidence": {
                        "provider": "Jin10",
                        "source_at": "2026-08-19 16:15:37",
                        "observed_at": "1787127606.533354000",
                        "batch_id": "TEST_CODE_external_global_news_batch"
                    }
                }))
                .expect("TEST_CODE external GlobalNews v2 JSON"),
            }],
            source: "grpc-mtls:TEST_CODE_market.local".to_string(),
            diagnostic_blocker: String::new(),
        }
    }

    #[test]
    fn br238_external_global_news_v2_preserves_provider_times_and_record_evidence() {
        let batch = external_global_news(GlobalNewsProvider::Jin10, &external_global_news_v2_q())
            .expect("complete admitted ExternalV1 GlobalNews v2 batch");

        assert_eq!(batch.evidence().provider, ProviderId::Jin10);
        assert_eq!(batch.evidence().source, GlobalNewsProvider::Jin10.source());
        assert_eq!(
            batch.evidence().source_at.as_deref(),
            Some("2026-08-19 16:15:37")
        );
        let record = &batch.records()[0];
        assert_eq!(record.item_id, "TEST_CODE_GLOBAL_NEWS_001");
        assert_eq!(record.evidence.source_at(), Some("2026-08-19 16:15:37"));
        assert_eq!(
            record.published_at,
            DateTime::parse_from_rfc3339("2026-08-19T16:15:37+08:00")
                .expect("TEST_CODE publication")
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn br238_external_global_news_v2_rejects_provider_and_record_evidence_conflicts() {
        let mut response = external_global_news_v2_q();
        response.selected_provider = "Eastmoney".to_string();
        let error = external_global_news(GlobalNewsProvider::Jin10, &response)
            .expect_err("transport provider must match the exact requested route");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());

        let mut response = external_global_news_v2_q();
        replace_external_news_evidence_field(
            &mut response,
            0,
            "provider",
            Value::String("Eastmoney".to_string()),
        );
        let error = external_global_news(GlobalNewsProvider::Jin10, &response)
            .expect_err("record provider must match its batch envelope");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br238_external_global_news_v2_rejects_records_out_of_provider_order() {
        let mut response = external_global_news_v2_q();
        let mut newer: Value = serde_json::from_slice(&response.records[0].data)
            .expect("TEST_CODE external GlobalNews v2 JSON");
        newer["item_id"] = Value::String("TEST_CODE_GLOBAL_NEWS_002".to_string());
        newer["url"] = Value::String("https://example.com/TEST_CODE_GLOBAL_NEWS_002".to_string());
        newer["published_at"] = Value::String("2026-08-19T16:15:38+08:00".to_string());
        newer["evidence"]["source_at"] = Value::String("2026-08-19 16:15:38".to_string());
        response.records.push(CanonicalPayload {
            data: serde_json::to_vec(&newer).expect("TEST_CODE external GlobalNews v2 JSON"),
            ..response.records[0].clone()
        });

        let error = external_global_news(GlobalNewsProvider::Jin10, &response)
            .expect_err("provider records must remain newest-first");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    fn replace_external_news_evidence_field(
        q: &mut QueryResult,
        index: usize,
        key: &str,
        value: Value,
    ) {
        let mut data: Value =
            serde_json::from_slice(&q.records[index].data).expect("external news fixture JSON");
        data["evidence"][key] = value;
        q.records[index].data = serde_json::to_vec(&data).expect("external news fixture JSON");
    }

    #[test]
    fn external_instrument_news_preserves_request_identity_and_observation_time() {
        let requested =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
                .expect("canonical test instrument");
        let batch =
            external_instrument_news("TEST_CODE_600396", &requested, 100, &external_news_q())
                .expect("complete admitted external news");
        let record = &batch.records()[0];
        let item = record.persistence_item();

        assert_eq!(item.code.as_deref(), Some("TEST_CODE_600396"));
        assert_eq!(item.title, "TEST_CODE 产业链新闻");
        assert_eq!(item.summary, "");
        assert_eq!(item.url, "https://example.com/TEST_CODE_news_1");
        assert_eq!(item.source_name, "TEST_CODE publisher");
        assert_eq!(
            item.published_at,
            DateTime::parse_from_rfc3339("2026-08-17T09:59:00+08:00")
                .unwrap()
                .with_timezone(&Utc)
        );
        assert_eq!(
            item.fetched_at,
            DateTime::parse_from_rfc3339("2026-08-17T10:00:01+08:00")
                .unwrap()
                .with_timezone(&Utc),
            "fetched_at must preserve envelope observed_at"
        );
        assert_eq!(
            record.evidence().source_at(),
            Some("2026-08-17T09:59:00+08:00")
        );
    }

    #[test]
    fn external_instrument_news_rejects_publication_after_captured_upper_bound() {
        let requested =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
                .expect("canonical test instrument");
        let date = NaiveDate::from_ymd_opt(2026, 8, 17).expect("TEST_CODE date");
        let captured_through = DateTime::parse_from_rfc3339("2026-08-17T09:58:59+08:00")
            .expect("TEST_CODE captured upper bound")
            .with_timezone(&Utc);
        let consumer_now = DateTime::parse_from_rfc3339("2026-08-17T10:00:01+08:00")
            .expect("TEST_CODE consumer clock")
            .with_timezone(&Utc);

        let error = external_instrument_news_in_range_at(
            "TEST_CODE_600396",
            &requested,
            &external_news_q(),
            ExternalInstrumentNewsRequestContext::new(
                date,
                date,
                100,
                captured_through,
                consumer_now,
            ),
        )
        .expect_err("records after the exact request upper bound must fail closed");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn external_instrument_news_accepts_per_record_times_within_batch_aggregates() {
        let requested =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
                .expect("canonical test instrument");
        let mut response = external_news_q();
        let mut older = response.records[0].clone();
        let mut value: Value = serde_json::from_slice(&older.data).unwrap();
        value["item_id"] = Value::String("https://example.com/TEST_CODE_news_2".to_string());
        value["url"] = Value::String("https://example.com/TEST_CODE_news_2".to_string());
        value["title"] = Value::String("TEST_CODE 较早产业链新闻".to_string());
        value["published_at"] = Value::String("2026-08-17T09:58:00+08:00".to_string());
        value["evidence"]["source_at"] = Value::String("2026-08-17T01:58:00Z".to_string());
        value["evidence"]["observed_at"] = Value::String("2026-08-17T09:59:59+08:00".to_string());
        older.data = serde_json::to_vec(&value).unwrap();
        response.records.push(older);

        let batch = external_instrument_news("TEST_CODE_600396", &requested, 100, &response)
            .expect("record times may precede batch newest/final aggregate times");

        assert_eq!(batch.records().len(), 2);
        assert_eq!(
            batch.records()[1].evidence().source_at(),
            Some("2026-08-17T01:58:00Z"),
            "semantic comparison must not rewrite raw record evidence"
        );
        assert_eq!(
            batch.records()[1].evidence().observed_at(),
            "2026-08-17T09:59:59+08:00"
        );
        assert_eq!(
            batch.evidence().source_at.as_deref(),
            Some("2026-08-17T09:59:00+08:00")
        );
        assert_eq!(batch.evidence().observed_at, "2026-08-17T10:00:01+08:00");
    }

    #[test]
    fn external_instrument_news_rejects_records_without_requested_instrument() {
        let requested =
            InstrumentId::new(Exchange::Shenzhen, "TEST_CODE_000001", AssetClass::Equity)
                .expect("canonical test instrument");

        assert!(
            external_instrument_news("TEST_CODE_000001", &requested, 100, &external_news_q(),)
                .is_err()
        );
    }

    #[test]
    fn external_instrument_news_rejects_record_envelope_evidence_conflict() {
        let requested =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
                .expect("canonical test instrument");
        for (field, value) in [
            ("provider", Value::String("Tencent".to_string())),
            (
                "batch_id",
                Value::String("TEST_CODE_other_batch".to_string()),
            ),
        ] {
            let mut response = external_news_q();
            replace_external_news_evidence_field(&mut response, 0, field, value);

            assert!(
                external_instrument_news("TEST_CODE_600396", &requested, 100, &response).is_err(),
                "record {field} conflict must fail closed"
            );
        }
    }

    #[test]
    fn external_instrument_news_rejects_times_outside_batch_aggregate_contract() {
        let requested =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
                .expect("canonical test instrument");

        let mut source_after_envelope = external_news_q();
        let mut value: Value =
            serde_json::from_slice(&source_after_envelope.records[0].data).unwrap();
        value["published_at"] = Value::String("2026-08-17T10:00:00+08:00".to_string());
        value["evidence"]["source_at"] = Value::String("2026-08-17T10:00:00+08:00".to_string());
        source_after_envelope.records[0].data = serde_json::to_vec(&value).unwrap();
        assert!(external_instrument_news(
            "TEST_CODE_600396",
            &requested,
            100,
            &source_after_envelope,
        )
        .is_err());

        let mut source_not_publication = external_news_q();
        replace_external_news_evidence_field(
            &mut source_not_publication,
            0,
            "source_at",
            Value::String("2026-08-17T09:58:00+08:00".to_string()),
        );
        assert!(external_instrument_news(
            "TEST_CODE_600396",
            &requested,
            100,
            &source_not_publication,
        )
        .is_err());

        let mut observation_after_envelope = external_news_q();
        replace_external_news_evidence_field(
            &mut observation_after_envelope,
            0,
            "observed_at",
            Value::String("2026-08-17T10:00:02+08:00".to_string()),
        );
        assert!(external_instrument_news(
            "TEST_CODE_600396",
            &requested,
            100,
            &observation_after_envelope,
        )
        .is_err());

        let mut missing_envelope_source = external_news_q();
        missing_envelope_source.source_at.clear();
        assert!(external_instrument_news(
            "TEST_CODE_600396",
            &requested,
            100,
            &missing_envelope_source,
        )
        .is_err());
    }

    #[test]
    fn external_instrument_news_rejects_mixed_payload_schemas() {
        let requested =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
                .expect("canonical test instrument");
        let mut response = external_news_q();
        let mut unexpected = response.records[0].clone();
        unexpected.schema = "magic.market.unfrozen_news".to_string();
        response.records.push(unexpected);

        assert!(external_instrument_news("TEST_CODE_600396", &requested, 100, &response).is_err());
    }

    #[test]
    fn external_instrument_news_preserves_missing_source_time() {
        let requested =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
                .expect("canonical test instrument");
        let mut response = external_news_q();
        response.source_at.clear();
        let mut value: Value = serde_json::from_slice(&response.records[0].data).unwrap();
        value["evidence"]["source_at"] = Value::Null;
        response.records[0].data = serde_json::to_vec(&value).unwrap();

        let batch = external_instrument_news("TEST_CODE_600396", &requested, 100, &response)
            .expect("missing source_at remains an admitted absence");
        assert!(batch.evidence().source_at.is_none());
        assert!(batch.records()[0].evidence().source_at().is_none());
    }

    #[test]
    fn external_instrument_news_rejects_items_outside_exact_requested_date_range() {
        let requested =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
                .expect("canonical test instrument");
        let error = external_instrument_news_in_range(
            "TEST_CODE_600396",
            &requested,
            NaiveDate::from_ymd_opt(2026, 8, 18).unwrap(),
            NaiveDate::from_ymd_opt(2026, 8, 18).unwrap(),
            100,
            &external_news_q(),
        )
        .expect_err("a response outside the exact request date range must fail closed");
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn external_instrument_news_rejects_stale_observation_at_consumer_clock() {
        let requested =
            InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
                .expect("canonical test instrument");
        let now = DateTime::parse_from_rfc3339("2026-08-17T10:00:31.000000001+08:00")
            .unwrap()
            .with_timezone(&Utc);
        let error = external_instrument_news_at(
            "TEST_CODE_600396",
            &requested,
            100,
            &external_news_q(),
            now,
        )
        .expect_err("news observation older than thirty seconds must fail closed");
        assert_eq!(error.reason_code(), "observation_stale");
        assert!(error.retryable());
    }

    fn local_identity_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-15T10:00:20+08:00")
            .expect("fixed local identity clock")
            .with_timezone(&Utc)
    }

    fn replace_external_record_field(q: &mut QueryResult, index: usize, key: &str, value: Value) {
        let mut data: Value =
            serde_json::from_slice(&q.records[index].data).expect("fixture record JSON");
        data.as_object_mut()
            .expect("fixture record object")
            .insert(key.to_string(), value);
        q.records[index].data = serde_json::to_vec(&data).expect("fixture record JSON");
    }

    #[test]
    fn provider_debug_names_roundtrip() {
        assert_eq!(parse_provider("Tdx").unwrap(), ProviderId::Tdx);
        assert!(parse_provider("tdx-dev").is_err());
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
    fn br238_global_news_accepts_magic_evidence_timestamp_encodings() {
        let record = r#"[{"item_id":"TEST_CODE_GLOBAL_NEWS_001","title":"TEST_CODE industry-chain news","summary":null,"content":null,"publisher":"TEST_CODE publisher","url":"https://example.com/TEST_CODE_GLOBAL_NEWS_001","published_at":"2026-08-17T11:50:00+08:00","instruments":[],"topics":["TEST_CODE_topic"],"language":"zh-CN"}]"#;

        for (encoded, expected) in [
            ("unix-ms:1786967511935", "2026-08-17T11:51:51.935+00:00"),
            ("1786967511.935000000", "2026-08-17T11:51:51.935+00:00"),
        ] {
            let mut response = mk_q(record, "Eastmoney", "eastmoney-web");
            response.observed_at = encoded.to_string();

            let batch = global_news(&response)
                .expect("admitted global news must accept every validated Magic timestamp form");
            assert_eq!(
                batch.records()[0].observed_at.to_rfc3339(),
                expected,
                "encoding={encoded}"
            );
            assert_eq!(
                batch.evidence().observed_at,
                encoded,
                "raw evidence must remain traceable"
            );
        }
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
    fn br238_realtime_consumer_rejects_five_seconds_and_one_nanosecond_old() {
        let now =
            Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap() + chrono::Duration::nanoseconds(1);
        let mut q = mk_q(
            r#"[{"code":"TEST_CODE_QUOTE_001","name":"TEST_CODE_name","price":10.0,"change_pct":0.0,"previous_close":9.9}]"#,
            "Tdx",
            "TEST_CODE_tdx",
        );
        q.source_at = "2026-08-17T01:30:00Z".to_string();
        q.observed_at = "2026-08-17T01:30:00Z".to_string();

        let error = realtime_quotes_at(&q, now)
            .expect_err("consumer age 5s+1ns must fail the realtime freshness red line");
        assert_eq!(error.reason_code(), "quote_stale");
        assert!(error.retryable());
    }

    #[test]
    fn br238_realtime_consumer_accepts_exact_five_second_fractional_unix_evidence() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = mk_q(
            r#"[{"code":"TEST_CODE_QUOTE_003","name":"TEST_CODE_name","price":10.0,"change_pct":0.0,"previous_close":9.9}]"#,
            "Tdx",
            "TEST_CODE_tdx",
        );
        q.source_at = "1786930200.000000000".to_string();
        q.observed_at = "1786930200.250000000".to_string();

        let batch = realtime_quotes_at(&q, now)
            .expect("exactly five-second-old source evidence remains live");
        assert_eq!(batch.records().len(), 1);
        assert_eq!(
            batch.records()[0].observed_at.timestamp_subsec_millis(),
            250
        );
    }

    #[test]
    fn br238_realtime_consumer_rejects_one_nanosecond_future_source_time() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 0).unwrap();
        let mut q = mk_q(
            r#"[{"code":"TEST_CODE_QUOTE_004","name":"TEST_CODE_name","price":10.0,"change_pct":0.0,"previous_close":9.9}]"#,
            "Tdx",
            "TEST_CODE_tdx",
        );
        q.source_at = "1786930200.000000001".to_string();
        q.observed_at = "1786930200.000000001".to_string();

        let error = realtime_quotes_at(&q, now)
            .expect_err("even 1ns future provider evidence must fail closed");
        assert_eq!(error.reason_code(), "quote_stale");
        assert!(error.retryable());
    }

    #[test]
    fn br238_realtime_consumer_rejects_non_positive_quote_prices() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 0).unwrap();
        let mut q = mk_q(
            r#"[{"code":"TEST_CODE_QUOTE_002","name":"TEST_CODE_name","price":0.0,"change_pct":0.0,"previous_close":9.9}]"#,
            "Tdx",
            "TEST_CODE_tdx",
        );
        q.source_at = "1786930200.000000000".to_string();
        q.observed_at = "1786930200.000000000".to_string();

        let error = realtime_quotes_at(&q, now)
            .expect_err("zero quote price must fail before reaching a consumer");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn empty_records_is_verified_empty() {
        let q = mk_q("[]", "Tdx", "tdx");
        let batch = realtime_quotes(&q).unwrap();
        assert!(batch.is_verified_empty());
    }

    #[test]
    fn offline_fixture_converters_cover_dormant_available_empty_and_invalid_contracts() {
        macro_rules! assert_converter_contract {
            (
                $case:literal,
                $converter:path,
                $provider_name:literal,
                $provider:expr,
                $source:literal,
                $available_value:expr,
                |$records:ident| $identity_assertion:block,
                |$invalid:ident| $invalid_mutation:block
            ) => {{
                let available_value: Value = $available_value;
                let available_json = serde_json::to_string(&available_value)
                    .expect("TEST_CODE available converter JSON");
                let available_query = mk_q(&available_json, $provider_name, $source);
                let available = $converter(&available_query)
                    .unwrap_or_else(|error| panic!("{} available failed: {error}", $case));
                assert_eq!(available.records().len(), 1, "{} record count", $case);
                assert_eq!(available.evidence().provider, $provider, "{} provider", $case);
                assert_eq!(available.evidence().source, $source, "{} source", $case);
                assert_eq!(available.evidence().batch_id, "b-1", "{} batch_id", $case);
                let $records = available.records();
                $identity_assertion

                let empty_query = mk_q("[]", $provider_name, $source);
                let empty = $converter(&empty_query)
                    .unwrap_or_else(|error| panic!("{} verified-empty failed: {error}", $case));
                assert!(empty.is_verified_empty(), "{} empty state", $case);
                assert_eq!(empty.evidence().provider, $provider, "{} empty provider", $case);
                assert_eq!(empty.evidence().batch_id, "b-1", "{} empty batch_id", $case);

                let mut $invalid = available_value;
                $invalid_mutation
                let invalid_json = serde_json::to_string(&$invalid)
                    .expect("TEST_CODE invalid converter JSON");
                let invalid_query = mk_q(&invalid_json, $provider_name, $source);
                let error = $converter(&invalid_query)
                    .expect_err(concat!($case, " invalid evidence must fail closed"));
                assert_eq!(error.reason_code(), "invalid_evidence", "{} invalid reason", $case);
            }};
        }

        assert_converter_contract!(
            "global_indices",
            global_indices,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "code": "DowJones",
                "name": "TEST_CODE_DJ",
                "value": 41000.0,
                "change": 12.0,
                "change_percent": 0.03,
                "source_at": "2026-08-15T09:35:00+08:00"
            }]),
            |records| {
                assert_eq!(records[0].name, "TEST_CODE_DJ");
            },
            |invalid| {
                invalid[0]["code"] = serde_json::json!("TEST_CODE_UNKNOWN");
            }
        );

        assert_converter_contract!(
            "foreign_exchange",
            foreign_exchange,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "pair": "UsdCny",
                "name": "TEST_CODE_USD_CNY",
                "rate": 7.15,
                "change": null,
                "change_percent": null,
                "source_at": "2026-08-15T09:35:00+08:00"
            }]),
            |records| {
                assert_eq!(records[0].name, "TEST_CODE_USD_CNY");
            },
            |invalid| {
                invalid[0]["pair"] = serde_json::json!("TEST_CODE_USDCNY");
            }
        );

        assert_converter_contract!(
            "announcements",
            announcements,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "announcement_id": "TEST_CODE_A1",
                "code": "TEST_CODE_600519",
                "category": null,
                "title": "TEST_CODE 公告",
                "published_at": "2026-08-15T09:00:00+08:00",
                "url": "https://example.com/TEST_CODE_A1"
            }]),
            |records| {
                assert_eq!(records[0].announcement_id, "TEST_CODE_A1");
                assert_eq!(records[0].code, "TEST_CODE_600519");
            },
            |invalid| {
                invalid[0]
                    .as_object_mut()
                    .expect("TEST_CODE announcement object")
                    .remove("title");
            }
        );

        assert_converter_contract!(
            "economic_calendar",
            economic_calendar,
            "Jin10",
            ProviderId::Jin10,
            "TEST_CODE_source",
            serde_json::json!([{
                "event_id": "TEST_CODE_E1",
                "indicator_id": 123,
                "country": "CN",
                "name": "TEST_CODE CPI",
                "period": "2026-07",
                "scheduled_at": "2026-08-15T09:30:00+08:00",
                "released_at": "2026-08-15T09:30:00+08:00",
                "previous": "0.6",
                "consensus": "0.7",
                "actual": "0.8",
                "revised": null,
                "unit": "%",
                "importance": 3,
                "impact": "positive"
            }]),
            |records| {
                assert_eq!(records[0].event_id, "TEST_CODE_E1");
            },
            |invalid| {
                invalid[0]["importance"] = serde_json::json!("bad");
            }
        );

        assert_converter_contract!(
            "futures_delivery",
            futures_delivery,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "contract_code": "TEST_CODE_IF2608",
                "product_code": "TEST_CODE_IF",
                "last_trading_date": null,
                "delivery_date": "2026-08-21",
                "notice_url": "https://example.com/TEST_CODE_FD"
            }]),
            |records| {
                assert_eq!(records[0].contract_code, "TEST_CODE_IF2608");
            },
            |invalid| {
                invalid[0]["delivery_date"] = serde_json::json!("bad-date");
            }
        );

        assert_converter_contract!(
            "market_statistics",
            market_statistics,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "code": "TEST_CODE_600519",
                "turnover_rate": 0.42,
                "trailing_pe": 28.5,
                "static_pe": 26.0,
                "pb": 9.2,
                "total_market_cap": 1880000000000.0,
                "floating_market_cap": 1880000000000.0,
                "upper_limit": 1650.0,
                "lower_limit": 1350.0,
                "volume_ratio": 1.1
            }]),
            |records| {
                assert_eq!(records[0].instrument().code(), "TEST_CODE_600519");
            },
            |invalid| {
                invalid[0]["upper_limit"] = serde_json::json!(0.0);
            }
        );

        assert_converter_contract!(
            "technical_bars",
            technical_bars,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "open": 10.0,
                "close": 10.1,
                "high": 10.2,
                "low": 9.9,
                "vol": 1000.0,
                "amount": 10000.0,
                "at": "2026-08-15T10:30:00+08:00"
            }]),
            |records| {
                assert_eq!(records[0].datetime, "2026-08-15T10:30:00+08:00");
            },
            |invalid| {
                invalid[0]
                    .as_object_mut()
                    .expect("TEST_CODE technical bar object")
                    .remove("at");
            }
        );

        assert_converter_contract!(
            "intraday_shape",
            intraday_shape,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "date": "2026-08-15",
                "pre_close": 1500.0,
                "open_pct": 0.2,
                "high_pct": 2.1,
                "low_pct": -0.8,
                "close_pct": 1.3,
                "amplitude": 2.9,
                "tail_30m_pct": 0.5,
                "shape_label": "TEST_CODE_SHAPE"
            }]),
            |records| {
                assert_eq!(records[0].shape_label, "TEST_CODE_SHAPE");
            },
            |invalid| {
                invalid[0]
                    .as_object_mut()
                    .expect("TEST_CODE intraday shape object")
                    .remove("shape_label");
            }
        );

        assert_converter_contract!(
            "market_dragon_tiger",
            market_dragon_tiger,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "exchange": "Shanghai",
                "code": "TEST_CODE_600519",
                "ranking_net_amount_yuan": 150000000.0,
                "disclosures": []
            }]),
            |records| {
                assert_eq!(records[0].code, "TEST_CODE_600519");
            },
            |invalid| {
                invalid[0]
                    .as_object_mut()
                    .expect("TEST_CODE dragon-tiger object")
                    .remove("exchange");
            }
        );

        assert_converter_contract!(
            "board_constituents",
            board_constituents,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "instrument_code": "TEST_CODE_600519",
                "board_code": "TEST_CODE_BK0475",
                "board_name": "TEST_CODE_BOARD",
                "kind": "Concept"
            }]),
            |records| {
                assert_eq!(records[0].instrument_code, "TEST_CODE_600519");
            },
            |invalid| {
                invalid[0]["kind"] = serde_json::json!("TEST_CODE_UNKNOWN");
            }
        );

        assert_converter_contract!(
            "research_reports",
            research_reports,
            "Tdx",
            ProviderId::Tdx,
            "TEST_CODE_source",
            serde_json::json!([{
                "code": "TEST_CODE_600519",
                "report_id": "TEST_CODE_R1",
                "title": "TEST_CODE REPORT",
                "organization": "TEST_CODE_ORG",
                "rating": "Buy",
                "published_at": "2026-08-15T09:00:00+08:00",
                "canonical_url": "https://example.com/TEST_CODE_R1",
                "target_price_upper": 12.0,
                "target_price_lower": 10.0
            }]),
            |records| {
                assert_eq!(records[0].report_id, "TEST_CODE_R1");
            },
            |invalid| {
                invalid[0]
                    .as_object_mut()
                    .expect("TEST_CODE research report object")
                    .remove("report_id");
            }
        );

        assert_converter_contract!(
            "provider_top_n_rankings",
            provider_top_n_rankings,
            "Eastmoney",
            ProviderId::Eastmoney,
            "eastmoney-web",
            serde_json::json!([{
                "metric": "VolumeRatio",
                "ordinal": 1,
                "code": "TEST_CODE_600519",
                "label": "TEST_CODE_SECURITY",
                "value": 3.2,
                "unit": "Multiple",
                "trading_date": "2026-08-15",
                "filter_identity": "TEST_CODE_FILTER",
                "provider_declared_total": 20,
                "inspected_row_count": 20
            }]),
            |records| {
                assert_eq!(records[0].instrument.code(), "TEST_CODE_600519");
            },
            |invalid| {
                invalid[0]["ordinal"] = serde_json::json!(0);
            }
        );

        assert_converter_contract!(
            "instrument_news",
            instrument_news,
            "Sina",
            ProviderId::Sina,
            "TEST_CODE_source",
            serde_json::json!([{
                "code": "TEST_CODE_600519",
                "title": "TEST_CODE NEWS",
                "summary": "TEST_CODE SUMMARY",
                "url": "https://example.com/TEST_CODE_NEWS",
                "source": "Sina",
                "source_name": "TEST_CODE SINA",
                "category": "TEST_CODE CATEGORY",
                "external_id": "TEST_CODE_N1",
                "published_at": "2026-08-15T09:00:00+08:00",
                "fetched_at": "2026-08-15T09:00:01+08:00",
                "content_hash": "TEST_CODE_HASH"
            }]),
            |records| {
                assert_eq!(
                    records[0].persistence_item().code.as_deref(),
                    Some("TEST_CODE_600519")
                );
            },
            |invalid| {
                invalid[0]
                    .as_object_mut()
                    .expect("TEST_CODE instrument news object")
                    .remove("content_hash");
            }
        );
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
    fn common_converters_reject_partial_unadmitted_and_diagnostic_responses() {
        let data = r#"[{"code":"TEST_CODE_QUOTE_001","name":"TEST_CODE_name","price":1.0,"change_pct":0.0,"previous_close":1.0}]"#;

        let mut partial = mk_q(data, "Tdx", "tdx");
        partial.complete = false;
        assert!(realtime_quotes(&partial).is_err());

        let mut unadmitted = mk_q(data, "Tdx", "tdx");
        unadmitted.admission = AdmissionState::Unadmitted;
        assert!(realtime_quotes(&unadmitted).is_err());

        let mut diagnostic = mk_q(data, "Tdx", "tdx");
        diagnostic.diagnostic_blocker = "TEST_CODE_runtime_blocker".to_string();
        assert!(realtime_quotes(&diagnostic).is_err());
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
    fn br238_order_book_consumer_accepts_exact_five_second_fractional_unix_evidence() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = mk_q(
            r#"[{"code":"TEST_CODE_BOOK_001","bids":[{"price":9.90,"quantity":100.0},{"price":9.89,"quantity":100.0},{"price":9.88,"quantity":100.0},{"price":9.87,"quantity":100.0},{"price":9.86,"quantity":100.0}],"asks":[{"price":10.10,"quantity":200.0},{"price":10.11,"quantity":200.0},{"price":10.12,"quantity":200.0},{"price":10.13,"quantity":200.0},{"price":10.14,"quantity":200.0}],"total_bid_quantity":500.0,"total_ask_quantity":1000.0,"source_at":"1786930200.000000000"}]"#,
            "Tdx",
            "TEST_CODE_tdx",
        );
        q.source_at = "1786930200.000000000".to_string();
        q.observed_at = "1786930200.250000000".to_string();

        let batch =
            order_books_at(&q, now).expect("exactly five-second-old book evidence remains live");
        assert_eq!(batch.records().len(), 1);
        assert_eq!(batch.records()[0].source_at.timestamp(), 1_786_930_200);
    }

    #[test]
    fn br238_order_book_consumer_rejects_record_envelope_time_conflict() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = mk_q(
            r#"[{"code":"TEST_CODE_BOOK_002","bids":[{"price":9.9,"quantity":100.0}],"asks":[{"price":10.1,"quantity":200.0}],"total_bid_quantity":100.0,"total_ask_quantity":200.0,"source_at":"1786930200.000000001"}]"#,
            "Tdx",
            "TEST_CODE_tdx",
        );
        q.source_at = "1786930200.000000000".to_string();
        q.observed_at = "1786930200.250000000".to_string();

        let error = order_books_at(&q, now)
            .expect_err("record time must bind exactly to the gRPC envelope instant");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br238_order_book_consumer_rejects_missing_levels_instead_of_zero_filling() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = mk_q(
            r#"[{"code":"TEST_CODE_BOOK_003","bids":[{"price":9.9,"quantity":100.0}],"asks":[{"price":10.1,"quantity":200.0}],"total_bid_quantity":100.0,"total_ask_quantity":200.0,"source_at":"1786930200.000000000"}]"#,
            "Tdx",
            "TEST_CODE_tdx",
        );
        q.source_at = "1786930200.000000000".to_string();
        q.observed_at = "1786930200.250000000".to_string();

        let error = order_books_at(&q, now)
            .expect_err("a live five-level book cannot synthesize absent levels as zero");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    fn live_t0_q() -> QueryResult {
        let mut q = mk_q(
            r#"{"requested_at":"2026-08-17T01:29:59Z","source_at":"2026-08-17T01:30:00Z","observed_at":"2026-08-17T01:30:00.250Z","batch_id":"TEST_CODE_T0_BATCH_001","time_untrustworthy":false,"records":[{"instrument":{"exchange":"Shanghai","code":"TEST_CODE_600519","asset_class":"Equity"},"code":"TEST_CODE_600519","requested_at":"2026-08-17T01:29:59Z","source_at":"2026-08-17T01:30:00Z","observed_at":"2026-08-17T01:30:00.250Z","batch_id":"TEST_CODE_T0_BATCH_001","quote":{"price":10.0,"last_close":9.9,"open":9.95,"high":10.1,"low":9.8,"volume":1000.0,"amount":10000.0,"bids":[{"price":9.99,"volume":100.0},{"price":9.98,"volume":100.0},{"price":9.97,"volume":100.0},{"price":9.96,"volume":100.0},{"price":9.95,"volume":100.0}],"asks":[{"price":10.01,"volume":100.0},{"price":10.02,"volume":100.0},{"price":10.03,"volume":100.0},{"price":10.04,"volume":100.0},{"price":10.05,"volume":100.0}]},"settled_daily":[],"completed_five_minute":[{"at":"2026-08-17T09:35:00+08:00","open":10.0,"high":10.1,"low":9.9,"close":10.0,"volume":1000.0,"amount":10000.0}],"intraday_average_price":9.98}],"rejections":[]}"#,
            "Tdx",
            "TEST_CODE_magic_tdx_t0",
        );
        q.records[0].schema = "market.t0_evidence".to_string();
        q.records[0].schema_version = 2;
        q.batch_id = "TEST_CODE_T0_BATCH_001".to_string();
        q.source_at = "1786930200.000000000".to_string();
        q.observed_at = "1786930200.250000000".to_string();
        q
    }

    #[test]
    fn br253_t0_v2_preserves_china_session_civil_label() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let batch = t0_evidence_batch_at(&live_t0_q(), now).unwrap();
        assert_eq!(batch.records[0].code, "TEST_CODE_600519");
        assert_eq!(batch.records[0].instrument.code(), "TEST_CODE_600519");
        assert_eq!(
            batch.records[0].completed_five_minute[0].at,
            NaiveDate::from_ymd_opt(2026, 8, 17)
                .unwrap()
                .and_hms_opt(9, 35, 0)
                .unwrap()
        );
    }

    #[test]
    fn br253_t0_v2_preserves_alternate_test_code_identity() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = live_t0_q();
        let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
        view["records"][0]["instrument"]["code"] = serde_json::json!("TEST_CODE_600520");
        view["records"][0]["code"] = serde_json::json!("TEST_CODE_600520");
        q.records[0].data = serde_json::to_vec(&view).unwrap();

        let batch = t0_evidence_batch_at(&q, now).unwrap();
        assert_eq!(batch.records[0].code, "TEST_CODE_600520");
        assert_eq!(batch.records[0].instrument.code(), "TEST_CODE_600520");
    }

    #[test]
    fn br253_t0_v2_rejects_malformed_test_code_suffix() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        for malformed in [
            "TEST_CODE_",
            "TEST_CODE_60051",
            "TEST_CODE_600519X",
            "TEST_CODE_NOT_CODE",
        ] {
            let mut q = live_t0_q();
            let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
            view["records"][0]["instrument"]["code"] = serde_json::json!(malformed);
            view["records"][0]["code"] = serde_json::json!(malformed);
            q.records[0].data = serde_json::to_vec(&view).unwrap();

            let error = t0_evidence_batch_at(&q, now)
                .expect_err("malformed TEST_CODE suffix must fail closed");
            assert_eq!(error.reason_code(), "invalid_evidence", "code={malformed}");
        }
    }

    #[test]
    fn br253_t0_v2_rebuilds_empty_record_batch_from_batch_requested_at() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = live_t0_q();
        let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
        view["records"] = serde_json::json!([]);
        view["rejections"] = serde_json::json!([{
            "code": "TEST_CODE_600519",
            "reason_code": "quote_stale",
            "detail": "TEST_CODE source time unavailable",
            "retryable": true
        }]);
        q.records[0].data = serde_json::to_vec(&view).unwrap();

        let batch = t0_evidence_batch_at(&q, now).unwrap();
        assert!(batch.records.is_empty());
        assert_eq!(batch.rejections.len(), 1);
        assert_eq!(
            batch.requested_at,
            Utc.with_ymd_and_hms(2026, 8, 17, 1, 29, 59).unwrap()
        );
    }

    #[test]
    fn br253_t0_v2_rejects_missing_or_wrong_session_offset() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        for raw_at in [
            "2026-08-17T09:35:00",
            "2026-08-17T01:35:00Z",
            "2026-08-17T10:35:00+09:00",
        ] {
            let mut q = live_t0_q();
            let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
            view["records"][0]["completed_five_minute"][0]["at"] = serde_json::json!(raw_at);
            q.records[0].data = serde_json::to_vec(&view).unwrap();

            let error = t0_evidence_batch_at(&q, now)
                .expect_err("T0 session bars must carry an explicit +08:00 offset");
            assert_eq!(error.reason_code(), "invalid_evidence", "at={raw_at}");
        }
    }

    #[test]
    fn br253_t0_v2_rejects_noncanonical_payload_identity() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut cases = Vec::new();

        let mut wrong_schema = live_t0_q();
        wrong_schema.records[0].schema = "TEST_CODE_market.t0_evidence".to_string();
        cases.push(("schema", wrong_schema));

        let mut wrong_version = live_t0_q();
        wrong_version.records[0].schema_version = 1;
        cases.push(("schema_version", wrong_version));

        let mut wrong_content_type = live_t0_q();
        wrong_content_type.records[0].content_type = "application/json".to_string();
        cases.push(("content_type", wrong_content_type));

        for (field, q) in cases {
            let error = t0_evidence_batch_at(&q, now)
                .expect_err("T0 consumer must accept only the frozen canonical v2 payload");
            assert_eq!(error.reason_code(), "invalid_evidence", "field={field}");
        }
    }

    #[test]
    fn br253_t0_v2_rejects_batch_envelope_conflicts() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        for (field, conflicting_value) in [
            ("batch_id", serde_json::json!("TEST_CODE_T0_BATCH_CONFLICT")),
            ("source_at", serde_json::json!("2026-08-17T01:29:59Z")),
            ("observed_at", serde_json::json!("2026-08-17T01:30:00.249Z")),
        ] {
            let mut q = live_t0_q();
            let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
            view[field] = conflicting_value;
            q.records[0].data = serde_json::to_vec(&view).unwrap();

            let error = t0_evidence_batch_at(&q, now)
                .expect_err("canonical T0 batch identity must equal the gRPC envelope");
            assert_eq!(error.reason_code(), "invalid_evidence", "field={field}");
        }
    }

    #[test]
    fn br253_t0_v2_rejects_record_batch_identity_conflicts() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        for (field, conflicting_value) in [
            ("requested_at", serde_json::json!("2026-08-17T01:29:58Z")),
            ("observed_at", serde_json::json!("2026-08-17T01:30:00.249Z")),
            (
                "batch_id",
                serde_json::json!("TEST_CODE_T0_RECORD_CONFLICT"),
            ),
        ] {
            let mut q = live_t0_q();
            let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
            view["records"][0][field] = conflicting_value;
            q.records[0].data = serde_json::to_vec(&view).unwrap();

            let error = t0_evidence_batch_at(&q, now)
                .expect_err("record T0 batch identity must equal the batch-level identity");
            assert_eq!(error.reason_code(), "invalid_evidence", "field={field}");
        }
    }

    #[test]
    fn br253_t0_v2_preserves_server_time_untrustworthy() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = live_t0_q();
        let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
        view["time_untrustworthy"] = serde_json::json!(true);
        q.records[0].data = serde_json::to_vec(&view).unwrap();

        let batch = t0_evidence_batch_at(&q, now).unwrap();
        assert!(
            batch.time_untrustworthy,
            "server-side time distrust must survive consumer freshness checks"
        );
    }

    #[test]
    fn br238_t0_consumer_preserves_live_envelope_provenance() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let q = live_t0_q();

        let batch = t0_evidence_batch_at(&q, now)
            .expect("complete exact-boundary T0 evidence must remain available");
        assert_eq!(batch.provider, ProviderId::Tdx);
        assert_eq!(batch.source, "TEST_CODE_magic_tdx_t0");
        assert_eq!(batch.batch_id, "TEST_CODE_T0_BATCH_001");
        assert_eq!(batch.records.len(), 1);
    }

    #[test]
    fn br243_t0_consumer_flags_record_source_older_than_five_seconds_instead_of_rejecting() {
        // 2026-08-21 用户指令「拿不到时间先跳过, 标注时间不可信」:
        // T0 age 门从硬拒改为 time_untrustworthy 标注 (桥组装延迟 11-55s 生产实测)。
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = live_t0_q();
        q.source_at = "1786930199.999999999".to_string();
        let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
        view["source_at"] = Value::String("1786930199.999999999".to_string());
        view["records"][0]["source_at"] = Value::String("1786930199.999999999".to_string());
        q.records[0].data = serde_json::to_vec(&view).unwrap();

        let batch = t0_evidence_batch_at(&q, now)
            .expect("T0 source age 5s+1ns must be flagged, not rejected (2026-08-21 指令)");
        assert!(batch.time_untrustworthy, "age 超门批次必须标注 时间不可信");
        assert_eq!(batch.records.len(), 1);
    }

    #[test]
    fn br243_t0_consumer_keeps_strict_time_corruption_rejections() {
        // 未来时间 / source_at 晚于 observed_at = 损坏而非延迟, age 放宽不覆盖。
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();

        let mut future = live_t0_q();
        future.source_at = "1786930205.000000001".to_string();
        let mut view: Value = serde_json::from_slice(&future.records[0].data).unwrap();
        view["records"][0]["source_at"] = Value::String("1786930205.000000001".to_string());
        future.records[0].data = serde_json::to_vec(&view).unwrap();
        let error = t0_evidence_batch_at(&future, now)
            .expect_err("future record source time must remain a hard rejection");
        assert_eq!(error.reason_code(), "quote_stale");

        let mut inverted = live_t0_q();
        inverted.source_at = "1786930200.500000000".to_string();
        let mut view: Value = serde_json::from_slice(&inverted.records[0].data).unwrap();
        view["records"][0]["source_at"] = Value::String("1786930200.500000000".to_string());
        inverted.records[0].data = serde_json::to_vec(&view).unwrap();
        let error = t0_evidence_batch_at(&inverted, now)
            .expect_err("record.source_at later than observed_at must stay a hard rejection");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br238_t0_consumer_rejects_multiple_payloads() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = live_t0_q();
        q.records.push(q.records[0].clone());

        let error = t0_evidence_batch_at(&q, now)
            .expect_err("T0 canonical contract is exactly one payload");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br238_t0_consumer_rejects_non_positive_quote_price() {
        let now = Utc.with_ymd_and_hms(2026, 8, 17, 1, 30, 5).unwrap();
        let mut q = live_t0_q();
        let mut view: Value = serde_json::from_slice(&q.records[0].data).unwrap();
        view["records"][0]["quote"]["price"] = serde_json::json!(0.0);
        q.records[0].data = serde_json::to_vec(&view).unwrap();

        let error = t0_evidence_batch_at(&q, now)
            .expect_err("non-positive T0 quote price cannot reach live consumers");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
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
    fn external_partial_security_metadata_projects_identity_only() {
        let q = external_identity_q();
        let batch =
            security_identities(&external_identity_requested(), &q, external_identity_now())
                .expect("identity subset remains complete");
        assert!(
            !q.complete,
            "fixture must exercise partial metadata envelope"
        );
        assert_eq!(batch.records().len(), 2);
        assert_eq!(batch.records()[0].code, "TEST_CODE_SECURITY_001");
        assert_eq!(batch.records()[1].code, "TEST_CODE_SECURITY_002");
        assert_eq!(batch.records()[0].provider, ProviderId::Tencent);
        assert_eq!(
            batch.records()[0].batch_id,
            "TEST_CODE_external_security_batch"
        );
        assert_eq!(
            batch.records()[0].source_at.timestamp_subsec_nanos(),
            125_000_000
        );
        assert_eq!(
            batch.records()[0].observed_at.timestamp_subsec_nanos(),
            250_000_000
        );
        assert_eq!(batch.evidence().source, "grpc-mtls:TEST_CODE_market.local");
        assert_eq!(
            batch.evidence().source_at.as_deref(),
            Some("1786931999.125000000")
        );
    }

    #[test]
    fn external_identity_accepts_oldest_record_source_as_batch_provenance() {
        let mut q = external_identity_q();
        replace_external_record_field(
            &mut q,
            1,
            "source_at",
            Value::String("1786931999.500000000".to_string()),
        );

        let batch =
            security_identities(&external_identity_requested(), &q, external_identity_now())
                .expect("batch source is the oldest record source, not every record source");

        assert_eq!(
            batch.records()[0].source_at.timestamp_subsec_nanos(),
            125_000_000
        );
        assert_eq!(
            batch.records()[1].source_at.timestamp_subsec_nanos(),
            500_000_000
        );
        assert_eq!(
            batch.evidence().source_at.as_deref(),
            Some("1786931999.125000000")
        );
    }

    #[test]
    fn external_identity_rejects_envelope_source_that_is_not_the_record_minimum() {
        let mut q = external_identity_q();
        q.source_at = "1786931999.000000000".to_string();

        let error =
            security_identities(&external_identity_requested(), &q, external_identity_now())
                .expect_err("batch source must equal the oldest record source");

        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn external_identity_rejects_record_source_after_record_observation() {
        let mut q = external_identity_q();
        replace_external_record_field(
            &mut q,
            1,
            "source_at",
            Value::String("1786932001.000000000".to_string()),
        );

        let error =
            security_identities(&external_identity_requested(), &q, external_identity_now())
                .expect_err("record source cannot be later than its observation");

        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn external_identity_accepts_reordered_exact_requested_set_and_returns_request_order() {
        let mut q = external_identity_q();
        q.records.reverse();
        let requested = vec![
            "TEST_CODE_SECURITY_001".to_string(),
            "TEST_CODE_SECURITY_002".to_string(),
        ];

        let batch = security_identities(&requested, &q, external_identity_now())
            .expect("wire order is not part of the external contract");

        assert_eq!(batch.records()[0].code, "TEST_CODE_SECURITY_001");
        assert_eq!(batch.records()[1].code, "TEST_CODE_SECURITY_002");
    }

    #[test]
    fn external_identity_rejects_non_exact_or_duplicate_code_sets() {
        let requested = external_identity_requested();

        let mut missing = external_identity_q();
        missing.records.pop();
        assert!(security_identities(&requested, &missing, external_identity_now()).is_err());

        let mut extra = external_identity_q();
        extra
            .records
            .push(external_identity_record("TEST_CODE_SECURITY_003", true));
        assert!(security_identities(&requested, &extra, external_identity_now()).is_err());

        let mut duplicate = external_identity_q();
        duplicate.records.push(duplicate.records[0].clone());
        assert!(security_identities(&requested, &duplicate, external_identity_now()).is_err());
    }

    #[test]
    fn external_identity_rejects_request_outside_unique_one_to_fifty_contract() {
        let q = external_identity_q();
        assert!(security_identities(&[], &q, external_identity_now()).is_err());

        let duplicate = vec![
            "TEST_CODE_SECURITY_001".to_string(),
            "TEST_CODE_SECURITY_001".to_string(),
        ];
        assert!(security_identities(&duplicate, &q, external_identity_now()).is_err());

        let oversized = (0..51)
            .map(|index| format!("TEST_CODE_SECURITY_{index:03}"))
            .collect::<Vec<_>>();
        assert!(security_identities(&oversized, &q, external_identity_now()).is_err());
    }

    #[test]
    fn external_identity_rejects_observation_older_than_thirty_seconds() {
        let mut q = external_identity_q();
        q.observed_at = "1786931900.000000000".to_string();
        q.source_at = "1786931800.000000000".to_string();
        for index in 0..q.records.len() {
            replace_external_record_field(
                &mut q,
                index,
                "observed_at",
                Value::String("1786931900.000000000".to_string()),
            );
            replace_external_record_field(
                &mut q,
                index,
                "source_at",
                Value::String("1786931800.000000000".to_string()),
            );
        }
        let requested = vec![
            "TEST_CODE_SECURITY_001".to_string(),
            "TEST_CODE_SECURITY_002".to_string(),
        ];

        let error = security_identities(&requested, &q, external_identity_now()).unwrap_err();
        assert_eq!(error.reason_code(), "observation_stale");
    }

    #[test]
    fn external_identity_rejects_source_older_than_one_trading_day() {
        let mut q = external_identity_q();
        q.source_at = "2026-08-13T15:00:00+08:00".to_string();
        for index in 0..q.records.len() {
            replace_external_record_field(
                &mut q,
                index,
                "source_at",
                Value::String("2026-08-13T15:00:00+08:00".to_string()),
            );
        }
        let requested = vec![
            "TEST_CODE_SECURITY_001".to_string(),
            "TEST_CODE_SECURITY_002".to_string(),
        ];

        let error = security_identities(&requested, &q, external_identity_now()).unwrap_err();
        assert_eq!(error.reason_code(), "daily_source_stale");
    }

    #[test]
    fn external_identity_accepts_exact_freshness_boundaries() {
        let q = external_identity_q();
        let exactly_thirty_seconds =
            DateTime::parse_from_rfc3339("2026-08-17T10:00:30.250000000+08:00")
                .expect("fixed 30-second boundary")
                .with_timezone(&Utc);
        security_identities(&external_identity_requested(), &q, exactly_thirty_seconds)
            .expect("exactly 30 seconds remains fresh");

        let mut previous_trading_day = external_identity_q();
        previous_trading_day.source_at = "2026-08-14T15:00:00+08:00".to_string();
        for index in 0..previous_trading_day.records.len() {
            replace_external_record_field(
                &mut previous_trading_day,
                index,
                "source_at",
                Value::String("2026-08-14T15:00:00+08:00".to_string()),
            );
        }
        security_identities(
            &external_identity_requested(),
            &previous_trading_day,
            external_identity_now(),
        )
        .expect("the immediately previous trading day remains fresh");
    }

    #[test]
    fn external_identity_accepts_one_second_future_evidence_within_clock_skew() {
        let mut q = external_identity_q();
        q.observed_at = "2026-08-17T10:00:21+08:00".to_string();
        q.source_at = "2026-08-17T10:00:21+08:00".to_string();
        for index in 0..q.records.len() {
            replace_external_record_field(
                &mut q,
                index,
                "observed_at",
                Value::String("2026-08-17T10:00:21+08:00".to_string()),
            );
            replace_external_record_field(
                &mut q,
                index,
                "source_at",
                Value::String("2026-08-17T10:00:21+08:00".to_string()),
            );
        }

        security_identities(&external_identity_requested(), &q, external_identity_now())
            .expect("one second of positive clock skew remains admissible");
    }

    #[test]
    fn external_identity_rejects_future_evidence_time() {
        let mut future_observation = external_identity_q();
        future_observation.observed_at = "2026-08-17T10:00:23+08:00".to_string();
        for index in 0..future_observation.records.len() {
            replace_external_record_field(
                &mut future_observation,
                index,
                "observed_at",
                Value::String("2026-08-17T10:00:23+08:00".to_string()),
            );
        }
        assert!(security_identities(
            &external_identity_requested(),
            &future_observation,
            external_identity_now()
        )
        .is_err());

        let mut future_source = external_identity_q();
        future_source.source_at = "2026-08-17T10:00:23+08:00".to_string();
        for index in 0..future_source.records.len() {
            replace_external_record_field(
                &mut future_source,
                index,
                "source_at",
                Value::String("2026-08-17T10:00:23+08:00".to_string()),
            );
        }
        assert!(security_identities(
            &external_identity_requested(),
            &future_source,
            external_identity_now()
        )
        .is_err());
    }

    #[test]
    fn full_security_metadata_rejects_external_partial_identity_payloads() {
        assert!(security_metadata(&external_identity_q()).is_err());
    }

    #[test]
    fn security_identities_accepts_existing_local_array_shape() {
        let mut q = mk_q(
            r#"[{"code":"TEST_CODE_LOCAL_001","name":"TEST_CODE_local_name","board":"Main","is_st":true,"listed_on":"2026-08-01","price_limit_percent":10.0,"source_at":"2026-08-15T09:35:00+08:00"}]"#,
            "Tdx",
            "tdx",
        );
        q.records[0].schema = "market.security_metadata".to_string();
        let requested = vec!["TEST_CODE_LOCAL_001".to_string()];
        let batch = security_identities(&requested, &q, local_identity_now())
            .expect("existing local identity shape");
        assert_eq!(batch.records()[0].code, "TEST_CODE_LOCAL_001");
        assert_eq!(batch.records()[0].name, "TEST_CODE_local_name");
        assert!(batch.records()[0].is_st);

        q.complete = false;
        assert!(
            security_identities(&requested, &q, local_identity_now()).is_err(),
            "partial exception is ExternalV1-only"
        );
    }

    #[test]
    fn security_identities_reject_unknown_mixed_schema_and_version() {
        let mut unknown = external_identity_q();
        for payload in &mut unknown.records {
            payload.schema = "TEST_CODE.unknown.security_metadata".to_string();
        }
        assert!(security_identities(
            &external_identity_requested(),
            &unknown,
            external_identity_now()
        )
        .is_err());

        let mut mixed = external_identity_q();
        mixed.records[1].schema = "market.security_metadata".to_string();
        assert!(security_identities(
            &external_identity_requested(),
            &mixed,
            external_identity_now()
        )
        .is_err());

        let mut wrong_version = external_identity_q();
        wrong_version.records[0].schema_version = 2;
        assert!(security_identities(
            &external_identity_requested(),
            &wrong_version,
            external_identity_now()
        )
        .is_err());
    }

    #[test]
    fn security_identities_reject_record_evidence_conflicts_and_missing_source_time() {
        for (field, value) in [
            ("provider", Value::String("Sina".to_string())),
            (
                "batch_id",
                Value::String("TEST_CODE_conflicting_batch".to_string()),
            ),
            (
                "observed_at",
                Value::String("1786932001.250000000".to_string()),
            ),
            (
                "source_at",
                Value::String("1786931998.125000000".to_string()),
            ),
        ] {
            let mut q = external_identity_q();
            replace_external_record_field(&mut q, 0, field, value);
            assert!(
                security_identities(&external_identity_requested(), &q, external_identity_now())
                    .is_err(),
                "record {field} conflict must fail closed"
            );
        }

        let mut missing_record_source_at = external_identity_q();
        replace_external_record_field(&mut missing_record_source_at, 0, "source_at", Value::Null);
        assert!(security_identities(
            &external_identity_requested(),
            &missing_record_source_at,
            external_identity_now()
        )
        .is_err());

        let mut missing_envelope_source_at = external_identity_q();
        missing_envelope_source_at.source_at.clear();
        assert!(security_identities(
            &external_identity_requested(),
            &missing_envelope_source_at,
            external_identity_now()
        )
        .is_err());
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

    #[test]
    fn semantic_search_canned_roundtrip() {
        // 视图: delegate.rs fetch_semantic_search — to_value(GeneralWebResearchRecord)
        // 直出 (snake_case serde, 含 record 级 evidence 子对象)。
        let q = mk_q(
            r#"[{"title":"白酒行业景气度跟踪","snippet":"2026年中报白酒板块营收同比增长 8.2%","url":"https://example.com/ws1","publisher":"国泰君安证券","published_at_raw":"2026-08-15T09:00:00+08:00","published_at":"2026-08-15T09:00:00+08:00","evidence":{"provider":"bocha","observed_at":"2026-08-15T10:00:00+08:00","batch_id":"b-1","item_id":"ws-1","publication_quality":"exact_provider_time","use_scope":"research_only"}}]"#,
            "Bocha",
            "bocha-general-web",
        );
        let batch =
            semantic_search(&q, "白酒 景气", GeneralWebResearchProvider::Bocha, 10).unwrap();
        let (records, evidence) = match batch {
            GeneralWebResearchBatch::Available { records, evidence } => (records, evidence),
            GeneralWebResearchBatch::VerifiedEmpty(_) => panic!("fixture 不应为空"),
        };
        assert_eq!(evidence.provider, GeneralWebResearchProvider::Bocha);
        assert_eq!(evidence.query, "白酒 景气");
        assert_eq!(evidence.use_scope, ResearchUseScope::ResearchOnly);
        assert_eq!(records[0].title, "白酒行业景气度跟踪");
        assert_eq!(records[0].evidence.batch_id, "b-1");
    }

    #[test]
    fn br242_semantic_search_rejects_unadmitted_local_bridge_envelope() {
        let mut q = mk_q("[]", "Bocha", "bocha-general-web");
        q.admission = AdmissionState::Unadmitted;

        let error = semantic_search(&q, "TEST_CODE query", GeneralWebResearchProvider::Bocha, 10)
            .expect_err("SemanticSearch must reject a repository-unadmitted response");

        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br242_semantic_search_rejects_diagnostic_local_bridge_envelope() {
        let mut q = mk_q("[]", "Bocha", "bocha-general-web");
        q.diagnostic_blocker = "TEST_CODE_operator_diagnostic".to_string();

        let error = semantic_search(&q, "TEST_CODE query", GeneralWebResearchProvider::Bocha, 10)
            .expect_err("SemanticSearch must reject diagnostic evidence in production");

        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br242_semantic_search_rejects_a_different_requested_provider() {
        let q = mk_q(
            r#"[{"title":"TEST_CODE result","snippet":"TEST_CODE context","url":"https://example.com/ws1","publisher":"TEST_CODE publisher","published_at_raw":null,"published_at":null,"evidence":{"provider":"bocha","observed_at":"2026-08-15T09:00:00+08:00","batch_id":"b-1","item_id":"ws-1","publication_quality":"missing","use_scope":"research_only"}}]"#,
            "Bocha",
            "bocha-general-web",
        );
        let error = semantic_search(
            &q,
            "TEST_CODE query",
            GeneralWebResearchProvider::Tavily,
            10,
        )
        .expect_err("response provider must equal the requested provider");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br242_semantic_search_rejects_a_different_requested_source() {
        let q = mk_q(
            r#"[{"title":"TEST_CODE result","snippet":"TEST_CODE context","url":"https://example.com/ws1","publisher":"TEST_CODE publisher","published_at_raw":null,"published_at":null,"evidence":{"provider":"bocha","observed_at":"2026-08-15T09:00:00+08:00","batch_id":"b-1","item_id":"ws-1","publication_quality":"missing","use_scope":"research_only"}}]"#,
            "Bocha",
            "TEST_CODE_wrong-source",
        );
        let error = semantic_search(&q, "TEST_CODE query", GeneralWebResearchProvider::Bocha, 10)
            .expect_err("response source must equal the requested provider source");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br242_semantic_search_rejects_records_over_requested_limit() {
        let q = mk_q(
            r#"[{"title":"TEST_CODE one","snippet":"TEST_CODE context","url":"https://example.com/ws1","publisher":"TEST_CODE publisher","published_at_raw":null,"published_at":null,"evidence":{"provider":"bocha","observed_at":"2026-08-15T09:00:00+08:00","batch_id":"b-1","item_id":"ws-1","publication_quality":"missing","use_scope":"research_only"}},{"title":"TEST_CODE two","snippet":"TEST_CODE context","url":"https://example.com/ws2","publisher":"TEST_CODE publisher","published_at_raw":null,"published_at":null,"evidence":{"provider":"bocha","observed_at":"2026-08-15T09:00:00+08:00","batch_id":"b-1","item_id":"ws-2","publication_quality":"missing","use_scope":"research_only"}}]"#,
            "Bocha",
            "bocha-general-web",
        );
        let error = semantic_search(&q, "TEST_CODE query", GeneralWebResearchProvider::Bocha, 1)
            .expect_err("response records must not exceed the requested limit");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn br242_semantic_search_rejects_record_observed_at_different_from_batch() {
        let q = mk_q(
            r#"[{"title":"TEST_CODE result","snippet":"TEST_CODE context","url":"https://example.com/ws1","publisher":"TEST_CODE publisher","published_at_raw":null,"published_at":null,"evidence":{"provider":"bocha","observed_at":"2026-08-15T09:59:59+08:00","batch_id":"b-1","item_id":"ws-1","publication_quality":"missing","use_scope":"research_only"}}]"#,
            "Bocha",
            "bocha-general-web",
        );
        let error = semantic_search(&q, "TEST_CODE query", GeneralWebResearchProvider::Bocha, 10)
            .expect_err("record observed_at must equal the admitted batch evidence");
        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(!error.retryable());
    }

    #[test]
    fn corporate_actions_canned_roundtrip() {
        // 视图: delegate.rs fetch_corporate_actions — category = Debug 名,
        // terms = to_value(CorporateActionTerms) (externally-tagged)。
        let q = mk_q(
            r#"[{"code":"600519","category":"Distribution","effective_on":"2026-08-20","record_on":"2026-08-13","ex_on":"2026-08-19","payable_on":"2026-08-21","terms":{"Distribution":{"cash_per_share":0.15,"bonus_per_share":null,"rights_per_share":null,"rights_price":null}}}]"#,
            "Tdx",
            "tdx",
        );
        let batch = corporate_actions(&q).unwrap();
        let r = &batch.records()[0];
        assert_eq!(r.code, "600519");
        assert_eq!(r.category, CorporateActionCategory::Distribution);
        assert_eq!(
            r.effective_on,
            NaiveDate::from_ymd_opt(2026, 8, 20).unwrap()
        );
        assert_eq!(r.ex_on, Some(NaiveDate::from_ymd_opt(2026, 8, 19).unwrap()));
        assert_eq!(
            r.payable_on,
            Some(NaiveDate::from_ymd_opt(2026, 8, 21).unwrap())
        );
        match &r.terms {
            CorporateActionTerms::Distribution { cash_per_share, .. } => {
                assert_eq!(cash_per_share.map(|c| c.get()), Some(0.15));
            }
            _ => panic!("terms 变体不符"),
        }
    }

    #[test]
    fn outcome_daily_bars_canned_roundtrip() {
        // 视图: delegate.rs fetch_outcome_daily_bars — batch = to_value(DataBatch<Bar>)
        // (Bar serde 字段 = provider.rs Repr), attempts = to_value(Preimage), error null。
        let q = mk_q(
            r#"{"batch":{"records":[{"instrument":{"exchange":"Shanghai","code":"600519","asset_class":"Equity"},"interval":"Day","bar_start":"2026-08-14","bar_end":"2026-08-14","open":1480.0,"high":1510.0,"low":1475.0,"close":1500.0,"volume":123456.0,"amount":1.85e9,"adjustment":"Unadjusted","source_at":null,"provider":"Tdx","batch_id":"fixture-ob"}],"provenance":{"source":"tdx","source_at":null,"fetched_at":"2026-08-15T10:00:00+08:00","batch_id":"fixture-ob"},"quality":{"complete":true,"issues":[]}},"attempts":[],"error":null}"#,
            "Tdx",
            "tdx",
        );
        let raw = outcome_daily_bars(&q).unwrap();
        assert_eq!(raw.batch.records().len(), 1, "batch 重建保真");
        let bar = &raw.batch.records()[0];
        assert_eq!(bar.instrument().code(), "600519", "bar instrument 保真");
        assert_eq!(bar.bar_start(), "2026-08-14", "bar_start 保真");
        assert_eq!(bar.close().get(), 1500.0, "bar close 保真");
        assert!(raw.batch.quality().is_complete(), "quality 保真");
        assert!(raw.attempts.is_empty(), "attempts 保真");
    }

    #[test]
    fn outcome_daily_bars_error_view_rebuild() {
        // error 视图 → OutcomeTransportFailure { error: classified 重建, attempts }。
        let q = mk_q(
            r#"{"batch":null,"attempts":[],"error":{"capability":"OutcomeDailyBarsV2","provider":"Tdx","audit_outcome":"unavailable","reason_code":"provider_unavailable","retryable":true,"message":"Magic TDX outcome bars failed: boom"}}"#,
            "Tdx",
            "tdx",
        );
        let failure = outcome_daily_bars(&q).unwrap_err();
        assert_eq!(
            failure.error.reason_code(),
            "provider_unavailable",
            "reason_code 重建"
        );
        assert_eq!(
            failure.error.audit_outcome(),
            "unavailable",
            "audit_outcome 重建"
        );
        assert!(failure.error.retryable(), "retryable 重建");
        assert_eq!(
            failure.error.provider(),
            Some(ProviderId::Tdx),
            "provider 回映"
        );
        assert_eq!(
            failure.error.capability(),
            "OutcomeDailyBarsV2",
            "capability 静态"
        );
        assert!(failure.attempts.is_empty(), "attempts 保真");
    }

    #[test]
    fn outcome_daily_bars_unknown_classification_falls_back_fail_closed() {
        // 未知 audit_outcome/reason_code 对 (版本偏移) → 兜底 no_verified_batch,
        // retryable 仍按 wire 原样 (fail-closed 语义不因版本偏移丢失)。
        let q = mk_q(
            r#"{"batch":null,"attempts":[],"error":{"capability":"OutcomeDailyBarsV9","provider":"Tdx","audit_outcome":"mystery","reason_code":"mystery","retryable":true,"message":"skew"}}"#,
            "Tdx",
            "tdx",
        );
        let failure = outcome_daily_bars(&q).unwrap_err();
        assert_eq!(failure.error.reason_code(), "no_verified_batch");
        assert!(failure.error.retryable());
    }
}
