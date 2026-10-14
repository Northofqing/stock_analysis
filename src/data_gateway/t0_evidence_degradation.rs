//! op 58 T0Evidence 证据缺省的降级事件（Windows 侧 3faa9d8 起在 Subscribe 通道推送）。
//!
//! 来源（对端 2026-10-14 交付单 §2）：每次证据缺省的 op 58 响应，服务端记结构化日志
//! `t0_evidence_degraded`，并在 **现有** Subscribe 通道推送
//! `event_kind = "t0_evidence_degraded"` 事件：`provider = LocalAnalysis`，
//! payload `schema = tdx.t0_evidence_degradation.v1`，`data` 为 JSON（含 batch_id 与缺失字段清单）。
//!
//! **本模块是纯观测**：只做严格解析、去重、计数与有界留存，不参与任何准入门控，
//! 也不改变行情或交易判定。对端字段名未在合同中冻结，故未知键**保留观察**（记录键名），
//! 既不臆造语义，也不因未知键丢弃整条事件。

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

/// 对端事件类型（区分大小写，逐字节比较）。
pub const EVENT_KIND: &str = "t0_evidence_degraded";
/// 对端声明的 provider。
pub const EXPECTED_PROVIDER: &str = "LocalAnalysis";
/// payload schema 名（版本由 `schema_version` 单独承载）。
pub const PAYLOAD_SCHEMA: &str = "tdx.t0_evidence_degradation.v1";
/// 目前已知的 schema 版本。
pub const PAYLOAD_SCHEMA_VERSION: u32 = 1;
/// 单条事件留存的原始 payload 上限（避免无界增长）。
const RAW_PAYLOAD_KEEP_BYTES: usize = 512;
/// 留存的最近事件条数上限。
const RECENT_CAPACITY: usize = 32;
/// 缺失字段清单的候选键（对端未冻结键名，按序取第一个命中的字符串数组）。
const MISSING_FIELD_KEYS: [&str; 4] = [
    "missing_fields",
    "missing",
    "missing_evidence_fields",
    "degraded_fields",
];

/// 与传输层解耦的事件视图：调用方把 `MarketEventEnvelope` 摊平后传入，
/// 便于纯函数测试。
#[derive(Debug, Clone, Copy)]
pub struct EventEnvelopeView<'a> {
    pub event_id: &'a str,
    pub event_kind: &'a str,
    pub provider: &'a str,
    pub instrument: &'a str,
    pub observed_at: &'a str,
    pub source_at: &'a str,
    pub schema: &'a str,
    pub schema_version: u32,
    pub content_type: &'a str,
    pub data: &'a [u8],
}

/// 解析后的一条降级事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct T0EvidenceDegradation {
    pub event_id: String,
    pub batch_id: String,
    pub instrument: String,
    pub observed_at: String,
    pub missing_fields: Vec<String>,
    /// 命中的缺失字段键名（对端未冻结；`None` 表示 payload 未给出可识别的字段清单）。
    pub missing_fields_key: Option<String>,
    /// payload 中出现、但本模块未消费的顶层键（保留观察，不认语义）。
    pub unknown_keys: Vec<String>,
    /// 原始 payload 前缀（有界，仅用于诊断；不含任何凭据）。
    pub raw_payload_prefix: String,
}

/// 观测动作的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// 首次记录（`degradation` 为解析结果）。
    Recorded(T0EvidenceDegradation),
    /// 同一 `event_id` 重复投递（对端重连重放时可能出现）。
    Duplicate { event_id: String, batch_id: String },
}

/// 进程内观测快照（诊断用）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DegradationSnapshot {
    pub recorded: u64,
    pub duplicates: u64,
    /// 每次出现缺失字段即计数（一条事件多个字段则计多次）。
    pub missing_field_counts: HashMap<String, u64>,
    /// 未知顶层键的出现次数（用于对端字段演进时对齐）。
    pub unknown_key_counts: HashMap<String, u64>,
    pub last_batch_id: Option<String>,
    pub last_observed_at: Option<String>,
}

#[derive(Default)]
struct Store {
    seen_event_ids: HashMap<String, String>,
    snapshot: DegradationSnapshot,
    recent: VecDeque<String>,
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Store::default()))
}

/// 观测存储是**进程级全局**：任何测试都必须串行访问（含 subscriber_tests）。
#[cfg(test)]
static TEST_STORE_LOCK: Mutex<()> = Mutex::new(());

fn json_content_type(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("application/json"))
}

/// 严格校验头部字段并解析 payload。
///
/// 头部字段（event_kind / provider / schema / schema_version / content_type）任一不符
/// 即整体拒绝；`batch_id` 缺失或为空亦拒绝。payload 中的未知键不拒绝，只记录键名。
pub fn parse_event(view: EventEnvelopeView<'_>) -> Result<T0EvidenceDegradation, String> {
    if view.event_kind != EVENT_KIND {
        return Err(format!("event_kind mismatch: {}", view.event_kind));
    }
    if view.provider != EXPECTED_PROVIDER {
        return Err(format!("provider mismatch: {}", view.provider));
    }
    if view.schema != PAYLOAD_SCHEMA {
        return Err(format!("payload schema mismatch: {}", view.schema));
    }
    if view.schema_version != PAYLOAD_SCHEMA_VERSION {
        return Err(format!(
            "payload schema_version mismatch: {}",
            view.schema_version
        ));
    }
    if !json_content_type(view.content_type) {
        return Err(format!(
            "payload content_type not JSON: {}",
            view.content_type
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(view.data)
        .map_err(|error| format!("payload is not valid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "payload JSON is not an object".to_string())?;
    let batch_id = object
        .get("batch_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "payload lacks a non-empty batch_id".to_string())?
        .to_string();

    let mut missing_fields_key = None;
    let mut missing_fields = Vec::new();
    for key in MISSING_FIELD_KEYS {
        if let Some(items) = object.get(key).and_then(|value| value.as_array()) {
            missing_fields = items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect();
            missing_fields_key = Some(key.to_string());
            break;
        }
    }

    const CONSUMED_KEYS: [&str; 5] = [
        "batch_id",
        "missing_fields",
        "missing",
        "missing_evidence_fields",
        "degraded_fields",
    ];
    let mut unknown_keys: Vec<String> = object
        .keys()
        .filter(|key| !CONSUMED_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    unknown_keys.sort();

    let mut raw_payload_prefix = String::from_utf8_lossy(view.data).into_owned();
    if raw_payload_prefix.len() > RAW_PAYLOAD_KEEP_BYTES {
        let mut end = RAW_PAYLOAD_KEEP_BYTES;
        while !raw_payload_prefix.is_char_boundary(end) {
            end -= 1;
        }
        raw_payload_prefix.truncate(end);
    }

    Ok(T0EvidenceDegradation {
        event_id: view.event_id.to_string(),
        batch_id,
        instrument: view.instrument.to_string(),
        observed_at: view.observed_at.to_string(),
        missing_fields,
        missing_fields_key,
        unknown_keys,
        raw_payload_prefix,
    })
}

/// 解析并记录一条事件（进程内、去重）。
pub fn observe(view: EventEnvelopeView<'_>) -> Result<Observed, String> {
    let parsed = parse_event(view)?;
    let mut store = store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let already_seen = store.seen_event_ids.get(&parsed.event_id).cloned();
    if let Some(batch_id) = already_seen {
        store.snapshot.duplicates += 1;
        return Ok(Observed::Duplicate {
            event_id: parsed.event_id.clone(),
            batch_id,
        });
    }
    store
        .seen_event_ids
        .insert(parsed.event_id.clone(), parsed.batch_id.clone());
    store.snapshot.recorded += 1;
    for field in &parsed.missing_fields {
        *store
            .snapshot
            .missing_field_counts
            .entry(field.clone())
            .or_insert(0) += 1;
    }
    for key in &parsed.unknown_keys {
        *store
            .snapshot
            .unknown_key_counts
            .entry(key.clone())
            .or_insert(0) += 1;
    }
    store.snapshot.last_batch_id = Some(parsed.batch_id.clone());
    store.snapshot.last_observed_at = Some(parsed.observed_at.clone());
    store.recent.push_back(format!(
        "batch_id={} missing=[{}]",
        parsed.batch_id,
        parsed.missing_fields.join(",")
    ));
    while store.recent.len() > RECENT_CAPACITY {
        store.recent.pop_front();
    }
    Ok(Observed::Recorded(parsed))
}

/// 当前观测快照。
pub fn snapshot() -> DegradationSnapshot {
    store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .snapshot
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 subscriber_tests 共用同一把锁（存储是进程级全局）。
    fn lock_store() -> std::sync::MutexGuard<'static, ()> {
        TEST_STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn reset_store() {
        let mut store = store()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *store = Store::default();
    }

    fn view<'a>(data: &'a [u8]) -> EventEnvelopeView<'a> {
        EventEnvelopeView {
            event_id: "evt-1",
            event_kind: EVENT_KIND,
            provider: EXPECTED_PROVIDER,
            instrument: "600519",
            observed_at: "2026-10-14T09:35:00Z",
            source_at: "",
            schema: PAYLOAD_SCHEMA,
            schema_version: PAYLOAD_SCHEMA_VERSION,
            content_type: "application/json; charset=utf-8",
            data,
        }
    }

    #[test]
    fn br255_degradation_event_parses_batch_and_missing_fields() {
        let _guard = lock_store();
        reset_store();
        let payload =
            br#"{"batch_id":"tdx-smart:1791963329","missing_fields":["name","source_at"]}"#;
        let observed = observe(view(payload)).expect("well-formed degradation event");
        let Observed::Recorded(recorded) = observed else {
            panic!("first delivery must be recorded");
        };
        assert_eq!(recorded.batch_id, "tdx-smart:1791963329");
        assert_eq!(recorded.missing_fields, vec!["name", "source_at"]);
        assert_eq!(
            recorded.missing_fields_key.as_deref(),
            Some("missing_fields")
        );
        assert!(recorded.unknown_keys.is_empty());

        let snapshot = snapshot();
        assert_eq!(snapshot.recorded, 1);
        assert_eq!(snapshot.duplicates, 0);
        assert_eq!(snapshot.missing_field_counts.get("name"), Some(&1));
        assert_eq!(snapshot.missing_field_counts.get("source_at"), Some(&1));
        assert_eq!(
            snapshot.last_batch_id.as_deref(),
            Some("tdx-smart:1791963329")
        );
    }

    #[test]
    fn br255_degradation_event_rejects_any_header_or_payload_mismatch() {
        let _guard = lock_store();
        reset_store();
        let payload = br#"{"batch_id":"b1","missing":["name"]}"#;
        assert!(parse_event(view(payload)).is_ok());

        let mut wrong_kind = view(payload);
        wrong_kind.event_kind = "something_else";
        assert!(parse_event(wrong_kind).is_err());

        let mut wrong_provider = view(payload);
        wrong_provider.provider = "Tdx";
        assert!(parse_event(wrong_provider).is_err());

        let mut wrong_schema = view(payload);
        wrong_schema.schema = "tdx.t0_evidence_degradation.v2";
        assert!(parse_event(wrong_schema).is_err());

        let mut wrong_version = view(payload);
        wrong_version.schema_version = 2;
        assert!(parse_event(wrong_version).is_err());

        let mut wrong_type = view(payload);
        wrong_type.content_type = "application/protobuf";
        assert!(parse_event(wrong_type).is_err());

        assert!(parse_event(view(b"not json")).is_err());
        assert!(parse_event(view(br#"[1,2,3]"#)).is_err());
        assert!(parse_event(view(br#"{"missing":["name"]}"#)).is_err());
        assert!(parse_event(view(br#"{"batch_id":"   "}"#)).is_err());
    }

    #[test]
    fn br255_degradation_event_dedupes_by_event_id_and_keeps_unknown_keys_visible() {
        let _guard = lock_store();
        reset_store();
        let payload =
            br#"{"batch_id":"b7","missing_evidence_fields":["name"],"severity":"degraded"}"#;
        assert!(matches!(observe(view(payload)), Ok(Observed::Recorded(_))));
        assert_eq!(
            snapshot().missing_field_counts.get("name"),
            Some(&1),
            "候选键 missing_evidence_fields 也应被识别"
        );
        assert_eq!(
            snapshot().unknown_key_counts.get("severity"),
            Some(&1),
            "未知键保留观察，不得静默丢弃"
        );

        let mut replay = view(payload);
        replay.event_id = "evt-1";
        assert!(matches!(observe(replay), Ok(Observed::Duplicate { .. })));
        let after_replay = snapshot();
        assert_eq!(after_replay.recorded, 1, "重复投递不得重复计数");
        assert_eq!(after_replay.duplicates, 1);

        // 不同 event_id、同 batch：视为新事件（对端可为同一批次多次投递）
        let mut second = view(payload);
        second.event_id = "evt-2";
        assert!(matches!(observe(second), Ok(Observed::Recorded(_))));
        assert_eq!(snapshot().recorded, 2);
    }
}

// ============================================================================
// 常驻订阅（纯观测；不参与任何准入判定）
// ============================================================================

use crate::grpc_client::client::GrpcMarketClient;
use crate::grpc_client::errors::GrpcError;
use crate::grpc_client::external_pb::magic::market::v1::{EventFilter, MarketEventEnvelope};
use std::path::Path;
use std::time::Duration;

/// 重连退避区间（防对端重连风暴）。
const RECONNECT_MIN: Duration = Duration::from_secs(5);
const RECONNECT_MAX: Duration = Duration::from_secs(60);

/// 把对端信封摊平为解析视图；payload 缺失时留空，交由 [`parse_event`] 拒绝。
fn view_from_envelope(envelope: &MarketEventEnvelope) -> EventEnvelopeView<'_> {
    let payload = envelope.payload.as_ref();
    EventEnvelopeView {
        event_id: envelope.event_id.as_str(),
        event_kind: envelope.event_kind.as_str(),
        provider: envelope.provider.as_str(),
        instrument: envelope.instrument.as_str(),
        observed_at: envelope.observed_at.as_str(),
        source_at: envelope.source_at.as_str(),
        schema: payload.map(|p| p.schema.as_str()).unwrap_or_default(),
        schema_version: payload.map(|p| p.schema_version).unwrap_or_default(),
        content_type: payload.map(|p| p.content_type.as_str()).unwrap_or_default(),
        data: payload.map(|p| p.data.as_slice()).unwrap_or_default(),
    }
}

/// 只暴露 reason_code/retryable，绝不回显对端报文（沿用本地探针的脱敏口径）。
fn safe_reason(error: &GrpcError) -> String {
    let details = error.details();
    format!(
        "reason_code={} retryable={}",
        details.reason_code.as_deref().unwrap_or("absent"),
        details
            .retryable
            .map(|value| value.to_string())
            .unwrap_or_else(|| "absent".to_string())
    )
}

/// 处理一条信封：非本类事件直接跳过；本类事件解析并记录。
fn consume_envelope(envelope: &MarketEventEnvelope) {
    if envelope.event_kind != EVENT_KIND {
        return;
    }
    match observe(view_from_envelope(envelope)) {
        Ok(Observed::Recorded(recorded)) => log::warn!(
            "[t0_evidence_degraded] batch_id={} instrument={} missing=[{}] unknown_keys=[{}] observed_at={}",
            recorded.batch_id,
            recorded.instrument,
            recorded.missing_fields.join(","),
            recorded.unknown_keys.join(","),
            recorded.observed_at
        ),
        Ok(Observed::Duplicate { batch_id, .. }) => log::debug!(
            "[t0_evidence_degraded] 重复投递 batch_id={batch_id}（已计入去重计数）"
        ),
        Err(reason) => log::warn!("[t0_evidence_degraded] 事件解析被拒: {reason}"),
    }
}

async fn open_stream(bundle: &Path) -> Result<tonic::Streaming<MarketEventEnvelope>, String> {
    let mut client = GrpcMarketClient::connect_client_bundle(bundle)
        .await
        .map_err(|error| format!("connect {}", safe_reason(&error)))?;
    // 与网关同序：先做一次 GetHealth 连接资格确认。`subscribe_external` 只检查资格
    // 状态（require_external_qualification），不会自行确认；跳过这一步会稳定得到
    // external_connection_unqualified（2026-10-14 实测）。
    client
        .get_external_health()
        .await
        .map_err(|error| format!("qualify {}", safe_reason(&error)))?;
    client
        .subscribe_external(
            EventFilter {
                instruments: Vec::new(),
                event_kinds: vec![EVENT_KIND.to_string()],
            },
            None,
        )
        .await
        .map_err(|error| format!("subscribe {}", safe_reason(&error)))
}

/// 常驻订阅对端降级事件的循环。失败按指数退避重连，永不向上传播错误，
/// 也绝不阻塞或改变任何数据/交易路径。
pub async fn run_degradation_subscriber(bundle: std::path::PathBuf) {
    let mut backoff = RECONNECT_MIN;
    loop {
        match open_stream(&bundle).await {
            Ok(mut stream) => {
                backoff = RECONNECT_MIN;
                log::info!("[t0_evidence_degraded] 观测订阅已建立");
                loop {
                    match stream.message().await {
                        Ok(Some(envelope)) => consume_envelope(&envelope),
                        Ok(None) => {
                            log::info!("[t0_evidence_degraded] 观测订阅流正常结束");
                            break;
                        }
                        Err(status) => {
                            log::warn!(
                                "[t0_evidence_degraded] 观测订阅流中断 grpc_code={}",
                                status.code() as i32
                            );
                            break;
                        }
                    }
                }
            }
            Err(reason) => log::warn!("[t0_evidence_degraded] 观测订阅未建立: {reason}"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_MAX);
    }
}

#[cfg(test)]
mod subscriber_tests {
    use super::*;

    fn envelope(kind: &str, data: &[u8]) -> MarketEventEnvelope {
        MarketEventEnvelope {
            protocol_version: 1,
            event_id: "evt-sub-1".to_string(),
            event_kind: kind.to_string(),
            provider: EXPECTED_PROVIDER.to_string(),
            instrument: "600519".to_string(),
            observed_at: "2026-10-14T09:35:00Z".to_string(),
            payload: Some(
                crate::grpc_client::external_pb::magic::market::v1::CanonicalPayload {
                    schema: PAYLOAD_SCHEMA.to_string(),
                    schema_version: PAYLOAD_SCHEMA_VERSION,
                    content_type: "application/json; charset=utf-8".to_string(),
                    data: data.to_vec(),
                },
            ),
            ..Default::default()
        }
    }

    #[test]
    fn br255_subscriber_records_our_kind_and_ignores_other_kinds() {
        let _guard = TEST_STORE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        {
            let mut store = store()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *store = Store::default();
        }

        // 其他事件类型：不算错误、不计数、不记录
        consume_envelope(&envelope("something_else", br#"{"batch_id":"b9"}"#));
        assert_eq!(snapshot().recorded, 0);
        assert_eq!(snapshot().duplicates, 0);

        // 本类事件：记录
        consume_envelope(&envelope(
            EVENT_KIND,
            br#"{"batch_id":"b9","missing_fields":["name"]}"#,
        ));
        assert_eq!(snapshot().recorded, 1);
        assert_eq!(snapshot().missing_field_counts.get("name"), Some(&1));

        // 同 event_id 重复投递：只计去重
        consume_envelope(&envelope(
            EVENT_KIND,
            br#"{"batch_id":"b9","missing_fields":["name"]}"#,
        ));
        assert_eq!(snapshot().recorded, 1);
        assert_eq!(snapshot().duplicates, 1);

        // 头部不符（provider 变了）：解析被拒，不计入
        let mut wrong = envelope(EVENT_KIND, br#"{"batch_id":"b9"}"#);
        wrong.provider = "Tdx".to_string();
        consume_envelope(&wrong);
        assert_eq!(snapshot().recorded, 1);
    }
}
