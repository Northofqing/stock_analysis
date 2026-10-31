//! TDX 异动事件生成器 (合同 §8: price/volume/amount/status/reset 事件;
//! cursor generation+sequence 单调递增; UNADMITTED 影子事件必须显式隔离)。
//!
//! 数据来源: 轮询快照 diff (纯函数 diff_snapshots) → EventHub 广播 + ring 重放。
//! fixture 模式不启动轮询, 集成测试直接注入 DetectedEvent。
use crate::grpc_client::pb::magic::market::v1::{
    market_event_service_server::MarketEventService, AdmissionState, CanonicalPayload, EventCursor,
    EventFilter, ListenerStatusRequest, ListenerStatusResponse, MarketEventEnvelope, ReplayRequest,
    SetWatchlistRequest, SetWatchlistResponse, SubscribeRequest,
};
use crate::grpc_server::ServerState;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tonic::{Request, Response, Status};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventKind {
    Price,
    Volume,
    Amount,
    Status,
    Reset,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Price => "price",
            EventKind::Volume => "volume",
            EventKind::Amount => "amount",
            EventKind::Status => "status",
            EventKind::Reset => "reset",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub prev_close: f64,
    pub volume: u64,
    pub amount: f64,
}

impl Quote {
    pub fn change_pct(&self) -> f64 {
        if self.prev_close <= 0.0 {
            0.0
        } else {
            (self.price - self.prev_close) / self.prev_close * 100.0
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetectedEvent {
    pub kind: EventKind,
    pub code: String,
    pub name: String,
    pub price: f64,
    pub prev_close: f64,
    pub change_pct: f64,
    pub volume: u64,
    pub amount: f64,
    pub reason: String,
}

/// 快照 diff: 涨跌幅变化 ≥ threshold_pct 百分点 → Price;
/// 成交量/成交额相对上一快照突增 ≥ volume_x 倍 → Volume/Amount;
/// 停牌 (volume=0) / 复牌 转换 → Status。纯函数, 离线单测。
pub fn diff_snapshots(
    prev: &[Quote],
    next: &[Quote],
    threshold_pct: f64,
    volume_x: f64,
) -> Vec<DetectedEvent> {
    let mut events = Vec::new();
    for q in next {
        let Some(p) = prev.iter().find(|p| p.code == q.code) else {
            // 新出现标的: 只作为初始快照, 不产生事件 (避免启动刷屏)。
            continue;
        };
        let change = (q.change_pct() - p.change_pct()).abs();
        if change >= threshold_pct {
            events.push(DetectedEvent {
                kind: EventKind::Price,
                code: q.code.clone(),
                name: q.name.clone(),
                price: q.price,
                prev_close: q.prev_close,
                change_pct: q.change_pct(),
                volume: q.volume,
                amount: q.amount,
                reason: format!("涨跌幅变化 {change:.2}pp"),
            });
        }
        if p.volume > 0 && q.volume as f64 >= p.volume as f64 * volume_x {
            events.push(DetectedEvent {
                kind: EventKind::Volume,
                code: q.code.clone(),
                name: q.name.clone(),
                price: q.price,
                prev_close: q.prev_close,
                change_pct: q.change_pct(),
                volume: q.volume,
                amount: q.amount,
                reason: format!("成交量突增 {:.1}x", q.volume as f64 / p.volume as f64),
            });
        }
        if p.amount > 0.0 && q.amount >= p.amount * volume_x {
            events.push(DetectedEvent {
                kind: EventKind::Amount,
                code: q.code.clone(),
                name: q.name.clone(),
                price: q.price,
                prev_close: q.prev_close,
                change_pct: q.change_pct(),
                volume: q.volume,
                amount: q.amount,
                reason: format!("成交额突增 {:.1}x", q.amount / p.amount),
            });
        }
        let was_halted = p.volume == 0;
        let now_halted = q.volume == 0;
        if was_halted != now_halted {
            events.push(DetectedEvent {
                kind: EventKind::Status,
                code: q.code.clone(),
                name: q.name.clone(),
                price: q.price,
                prev_close: q.prev_close,
                change_pct: q.change_pct(),
                volume: q.volume,
                amount: q.amount,
                reason: if now_halted {
                    "停牌".to_string()
                } else {
                    "复牌".to_string()
                },
            });
        }
    }
    events
}

pub struct EventHub {
    generation: String,
    sequence: AtomicU64,
    tx: broadcast::Sender<MarketEventEnvelope>,
    ring: Mutex<VecDeque<MarketEventEnvelope>>,
    shadow_events: bool,
}

const RING_CAPACITY: usize = 10_000;

impl EventHub {
    pub fn new(generation: String, shadow_events: bool) -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self {
            generation,
            sequence: AtomicU64::new(0),
            tx,
            ring: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
            shadow_events,
        }
    }

    pub fn push_event(&self, event: &DetectedEvent) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let envelope = MarketEventEnvelope {
            protocol_version: 1,
            event_id: crate::grpc_client::envelope::new_request_id(),
            cursor: Some(EventCursor {
                generation: self.generation.clone(),
                sequence,
            }),
            event_kind: event.kind.as_str().to_string(),
            provider: "tdx-dev".to_string(),
            instrument: event.code.clone(),
            observed_at: chrono::Local::now().to_rfc3339(),
            source_at: String::new(), // 轮询行情无可信源时间 → 空 (合同 §6 不填充)
            admission: if self.shadow_events {
                AdmissionState::Unadmitted as i32
            } else {
                AdmissionState::Admitted as i32
            },
            payload: Some(CanonicalPayload {
                schema: "market_event".to_string(),
                schema_version: 1,
                content_type: "application/json; charset=utf-8".to_string(),
                data: serde_json::to_vec(&serde_json::json!({
                    "code": event.code, "name": event.name, "price": event.price,
                    "prev_close": event.prev_close, "change_pct": event.change_pct,
                    "volume": event.volume, "amount": event.amount, "reason": event.reason,
                }))
                .unwrap_or_default(),
            }),
        };
        let mut ring = self.ring.lock().unwrap();
        if ring.len() >= RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(envelope.clone());
        drop(ring);
        let _ = self.tx.send(envelope);
    }

    pub fn latest_cursor(&self) -> EventCursor {
        let ring = self.ring.lock().unwrap();
        let seq = ring
            .back()
            .and_then(|e| e.cursor.as_ref().map(|c| c.sequence))
            .unwrap_or(0);
        EventCursor {
            generation: self.generation.clone(),
            sequence: seq,
        }
    }

    /// Replay: 有界、同 generation、best-effort (合同 §8)。
    pub fn replay_after(
        &self,
        cursor: Option<EventCursor>,
    ) -> Result<Vec<MarketEventEnvelope>, Status> {
        let ring = self.ring.lock().unwrap();
        let Some(cursor) = cursor else {
            return Ok(ring.iter().cloned().collect());
        };
        if cursor.generation != self.generation {
            return Err(Status::failed_precondition(
                "generation 不匹配, 连续性已重置",
            ));
        }
        if cursor.sequence == 0 {
            // 序列从 1 开始; 0 = 无事件序号, 与 None 同义 → 从起点重放。
            return Ok(ring.iter().cloned().collect());
        }
        let latest = ring
            .back()
            .and_then(|e| e.cursor.as_ref().map(|c| c.sequence))
            .unwrap_or(0);
        if latest < cursor.sequence {
            // cursor 未来值 → 空重放 (可能服务端重启后 sequence 回退)。
            return Ok(vec![]);
        }
        let oldest = ring
            .front()
            .and_then(|e| e.cursor.as_ref().map(|c| c.sequence))
            .unwrap_or(0);
        if cursor.sequence < oldest {
            return Err(Status::out_of_range("cursor 早于重放窗口, 记录明确 gap"));
        }
        Ok(ring
            .iter()
            .filter(|e| {
                e.cursor
                    .as_ref()
                    .map(|c| c.sequence > cursor.sequence)
                    .unwrap_or(false)
            })
            .cloned()
            .collect())
    }
}

pub struct EventService {
    pub hub: Arc<EventHub>,
    /// SetWatchlist / ListenerStatus 的 watchlist 状态源 (ServerState.watchlist)。
    pub state: Arc<ServerState>,
}

impl EventService {
    /// fixture 模式: hub 只接受外部注入 (集成测试调 push_event), 不启动轮询。
    pub fn new(state: Arc<ServerState>, _fixture_mode: bool) -> Self {
        Self {
            hub: Arc::new(EventHub::new(state.generation.clone(), state.shadow_events)),
            state,
        }
    }
}

fn envelope_matches(e: &MarketEventEnvelope, f: &EventFilter) -> bool {
    let ok_instrument = f.instruments.is_empty() || f.instruments.contains(&e.instrument);
    let ok_kind = f.event_kinds.is_empty() || f.event_kinds.contains(&e.event_kind);
    ok_instrument && ok_kind
}

#[tonic::async_trait]
impl MarketEventService for EventService {
    // proto 两个流方法各有一个独立关联类型 (SubscribeStream/ReplayStream),
    // 即使返回类型相同也不能共用。
    type SubscribeStream =
        tokio_stream::wrappers::ReceiverStream<Result<MarketEventEnvelope, Status>>;
    type ReplayStream = tokio_stream::wrappers::ReceiverStream<Result<MarketEventEnvelope, Status>>;

    async fn subscribe(
        &self,
        req: Request<SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let inner = req.into_inner();
        let filter = inner.filter.unwrap_or(EventFilter {
            instruments: vec![],
            event_kinds: vec![],
        });
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let hub = self.hub.clone();
        let mut live_rx = hub.tx.subscribe();
        let replay = hub.replay_after(inner.after)?;
        tokio::spawn(async move {
            for envelope in replay {
                if envelope_matches(&envelope, &filter) && tx.send(Ok(envelope)).await.is_err() {
                    return;
                }
            }
            while let Ok(envelope) = live_rx.recv().await {
                if envelope_matches(&envelope, &filter) && tx.send(Ok(envelope)).await.is_err() {
                    return;
                }
            }
        });
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    async fn replay(
        &self,
        req: Request<ReplayRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let inner = req.into_inner();
        let envelopes = self.hub.replay_after(inner.after)?;
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        tokio::spawn(async move {
            for envelope in envelopes {
                if tx.send(Ok(envelope)).await.is_err() {
                    return;
                }
            }
        });
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    async fn get_listener_status(
        &self,
        _req: Request<ListenerStatusRequest>,
    ) -> Result<Response<ListenerStatusResponse>, Status> {
        let cursor = self.hub.latest_cursor();
        let watchlist = self.state.watchlist.lock().unwrap().clone();
        let revision = self.state.watchlist_revision.load(Ordering::Relaxed);
        // 合同 §8 watchlist 状态: 本地 server 无异步应用流程, desired==applied;
        // maximum=0 = 未声明上限 (本地 server 不限制); admitted_event_families =
        // 本地 EventKind 全集 (与 diff_snapshots 产出一致)。
        let families: Vec<String> = [
            EventKind::Price,
            EventKind::Volume,
            EventKind::Amount,
            EventKind::Status,
            EventKind::Reset,
        ]
        .into_iter()
        .map(|k| k.as_str().to_string())
        .collect();
        Ok(Response::new(ListenerStatusResponse {
            request_id: "status".to_string(),
            state: "RUNNING".to_string(),
            terminal_generation: cursor.generation.clone(),
            latest: Some(cursor),
            capabilities: vec![],
            desired_watchlist_revision: revision,
            desired_instruments: watchlist.clone(),
            applied_watchlist_revision: revision,
            applied_instruments: watchlist,
            maximum_watchlist_instruments: 0,
            admitted_event_families: families,
        }))
    }

    async fn set_watchlist(
        &self,
        req: Request<SetWatchlistRequest>,
    ) -> Result<Response<SetWatchlistResponse>, Status> {
        let inner = req.into_inner();
        // 本地 server 无异步应用流程: 请求立即应用 (desired==applied), 版本 +1。
        let mut watchlist = self.state.watchlist.lock().unwrap();
        watchlist.clear();
        watchlist.extend(inner.instruments.iter().cloned());
        let revision = self
            .state
            .watchlist_revision
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        Ok(Response::new(SetWatchlistResponse {
            request_id: "set-watchlist".to_string(),
            desired_revision: revision,
            state: "APPLIED".to_string(),
            instruments: inner.instruments,
        }))
    }
}

pub use crate::grpc_client::pb::magic::market::v1::market_event_service_server;

/// 轮询间隔 (EVENT_POLL_INTERVAL_MS, 默认 3000ms)。
/// v15.x: 默认值出声 — 调用方启动时必须打印实际生效值。
pub fn poll_interval_ms() -> u64 {
    std::env::var("EVENT_POLL_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000)
}

/// 异动阈值 (EVENT_PRICE_THRESHOLD_PCT 百分点 / EVENT_VOLUME_THRESHOLD_X 倍数,
/// 默认 0.5 / 1.5)。v15.x: 调用方启动时必须打印实际生效值。
pub fn thresholds() -> (f64, f64) {
    let pct = std::env::var("EVENT_PRICE_THRESHOLD_PCT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.5);
    let x = std::env::var("EVENT_VOLUME_THRESHOLD_X")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.5);
    (pct, x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(code: &str, price: f64, prev_close: f64, volume: u64, amount: f64) -> Quote {
        Quote {
            code: code.to_string(),
            name: format!("n-{code}"),
            price,
            prev_close,
            volume,
            amount,
        }
    }

    #[test]
    fn detects_price_movement() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let next = vec![q("600519", 1520.0, 1500.0, 100, 1e8)];
        let events = diff_snapshots(&prev, &next, 0.5, 1.5);
        assert!(events.iter().any(|e| e.kind == EventKind::Price));
    }

    #[test]
    fn ignores_small_movement() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let next = vec![q("600519", 1501.0, 1500.0, 100, 1e8)];
        let events = diff_snapshots(&prev, &next, 0.5, 1.5);
        assert!(events.is_empty());
    }

    #[test]
    fn detects_volume_spike() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let next = vec![q("600519", 1500.0, 1500.0, 400, 1e8)];
        let events = diff_snapshots(&prev, &next, 0.5, 1.5);
        assert!(events.iter().any(|e| e.kind == EventKind::Volume));
    }

    #[test]
    fn detects_halt_and_resume() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let halted = vec![q("600519", 1500.0, 1500.0, 0, 0.0)];
        let resumed = vec![q("600519", 1500.0, 1500.0, 50, 5e7)];
        let e1 = diff_snapshots(&prev, &halted, 0.5, 1.5);
        assert!(e1
            .iter()
            .any(|e| e.kind == EventKind::Status && e.reason == "停牌"));
        let e2 = diff_snapshots(&halted, &resumed, 0.5, 1.5);
        assert!(e2
            .iter()
            .any(|e| e.kind == EventKind::Status && e.reason == "复牌"));
    }

    #[test]
    fn new_code_in_snapshot_does_not_spam() {
        let prev = vec![q("600519", 1500.0, 1500.0, 100, 1e8)];
        let next = vec![
            q("600519", 1500.0, 1500.0, 100, 1e8),
            q("000001", 10.0, 10.0, 1000, 1e6),
        ];
        assert!(diff_snapshots(&prev, &next, 0.5, 1.5).is_empty());
    }
}
