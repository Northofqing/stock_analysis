//! Deterministic integration-test event hub (合同 §8: price/volume/amount/status/reset 事件;
//! cursor generation+sequence 单调递增; UNADMITTED 影子事件必须显式隔离)。
//!
//! 数据来源: 轮询快照 diff (纯函数 diff_snapshots) → EventHub 广播 + ring 重放。
//! fixture 模式不启动轮询, 集成测试直接注入 DetectedEvent。
use super::ServerState;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use stock_analysis::grpc_client::pb::magic::market::v1::{
    market_event_service_server::MarketEventService, AdmissionState, CanonicalPayload, EventCursor,
    EventFilter, ListenerStatusRequest, ListenerStatusResponse, MarketEventEnvelope, ReplayRequest,
    SetWatchlistRequest, SetWatchlistResponse, SubscribeRequest,
};
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
            event_id: stock_analysis::grpc_client::envelope::new_request_id(),
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
    pub fn new(state: Arc<ServerState>) -> Self {
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

pub use stock_analysis::grpc_client::pb::magic::market::v1::market_event_service_server;
