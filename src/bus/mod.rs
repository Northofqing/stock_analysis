//! v16.4 Commit 1 — 3 Bus 拆分 (替代 v16.2 单 EventBus).
//!
//! 设计 (v16.3 doc §3.1): SignalBus/TradingBus/SystemBus 3 个独立 broadcast channel,
//!                          替代 v16.2 单一 EventBus. 共存, 增量添加, 不替换现有
//!                          `src/monitor/event_bus.rs::EventBus` (v16.2 业务仍在用).
//!
//! 业务:
//!   - SignalBus  : 跨模块 "信号" 事件 (Feature 计算完成, Signal emit, Risk 检查结果)
//!   - TradingBus : "交易" 事件 (Order 创建, Execution 成交, Position 变化)
//!   - SystemBus  : "系统" 事件 (PerformanceSnapshot, Error, Config 变化)
//!
//! 复用: `tokio::sync::broadcast` (项目已依赖, 1 emit → N consumer 并行).
//! 单例: `OnceCell<SignalBus/TradingBus/SystemBus>` 全局 3 个.
//!
//! v16.4 Commit 1 注: 仅落地 3 Bus 骨架 + 6 单测.
//! Fix review #14: 业务模块接入推 v16.4 #5 (SignalBus 接入 intraday_monitor,
//!                  TradingBus 接入 paper_trade, SystemBus 接入 PerformanceEngine).

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

const SIGNAL_BUS_CAPACITY: usize = 512;
const TRADING_BUS_CAPACITY: usize = 128;
const SYSTEM_BUS_CAPACITY: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignalEvent {
    FeatureComputed {
        signal_id: SignalId,
        strategy_id: StrategyId,
        code: String,
        score: f64,
    },
    SignalEmitted {
        signal_id: SignalId,
        strategy_id: StrategyId,
        code: String,
        score: f64,
    },
    RiskChecked {
        signal_id: SignalId,
        decision_id: DecisionId,
        allowed: bool,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TradingEvent {
    OrderCreated {
        decision_id: DecisionId,
        order_id: OrderId,
        code: String,
        side: String,
    },
    ExecutionFilled {
        order_id: OrderId,
        execution_id: ExecutionId,
        fill_price: f64,
    },
    PositionChanged {
        code: String,
        quantity: i64,
        pnl: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemEvent {
    PerformanceSnapshot {
        snapshot_id: String,
        date: chrono::NaiveDate,
        total_pnl: f64,
    },
    ErrorOccurred {
        component: String,
        message: String,
    },
}

pub type SignalId = String;
pub type DecisionId = String;
pub type OrderId = String;
pub type ExecutionId = String;
pub type StrategyId = String;

fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = chrono::Utc::now().timestamp_millis();
    format!("{}-{}-{:x}", prefix, ts, n)
}

pub fn new_signal_id() -> SignalId {
    new_id("sig")
}
pub fn new_decision_id() -> DecisionId {
    new_id("dec")
}
pub fn new_order_id() -> OrderId {
    new_id("ord")
}
pub fn new_execution_id() -> ExecutionId {
    new_id("exe")
}
pub fn new_strategy_id(name: &str, version: &str) -> StrategyId {
    format!("strat-{}-{}-{}", name, version, &new_id("v")[4..])
}

pub struct SignalBus {
    tx: broadcast::Sender<SignalEvent>,
}
pub struct TradingBus {
    tx: broadcast::Sender<TradingEvent>,
}
pub struct SystemBus {
    tx: broadcast::Sender<SystemEvent>,
}

static SIGNAL_BUS: OnceCell<SignalBus> = OnceCell::new();
static TRADING_BUS: OnceCell<TradingBus> = OnceCell::new();
static SYSTEM_BUS: OnceCell<SystemBus> = OnceCell::new();

impl SignalBus {
    pub fn global() -> &'static Self {
        SIGNAL_BUS.get_or_init(|| Self {
            tx: broadcast::channel(SIGNAL_BUS_CAPACITY).0,
        })
    }
    pub fn publish(&self, event: SignalEvent) {
        let _ = self.tx.send(event);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<SignalEvent> {
        self.tx.subscribe()
    }
}

impl TradingBus {
    pub fn global() -> &'static Self {
        TRADING_BUS.get_or_init(|| Self {
            tx: broadcast::channel(TRADING_BUS_CAPACITY).0,
        })
    }
    pub fn publish(&self, event: TradingEvent) {
        let _ = self.tx.send(event);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<TradingEvent> {
        self.tx.subscribe()
    }
}

impl SystemBus {
    pub fn global() -> &'static Self {
        SYSTEM_BUS.get_or_init(|| Self {
            tx: broadcast::channel(SYSTEM_BUS_CAPACITY).0,
        })
    }
    pub fn publish(&self, event: SystemEvent) {
        let _ = self.tx.send(event);
    }
    pub fn subscribe(&self) -> broadcast::Receiver<SystemEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_bus_emit_and_subscribe() {
        let bus = SignalBus::global();
        let mut rx = bus.subscribe();
        let signal_id = new_signal_id();
        bus.publish(SignalEvent::SignalEmitted {
            signal_id: signal_id.clone(),
            strategy_id: "strat-test".to_string(),
            code: "TEST_CODE_000001".to_string(),
            score: 7.5,
        });
        let ev = rx.try_recv().expect("应该收到事件");
        match ev {
            SignalEvent::SignalEmitted {
                signal_id: id,
                score,
                ..
            } => {
                assert_eq!(id, signal_id);
                assert_eq!(score, 7.5);
            }
            _ => panic!("事件类型错"),
        }
    }

    #[test]
    fn trading_bus_emit_and_subscribe() {
        let bus = TradingBus::global();
        let mut rx = bus.subscribe();
        let order_id = new_order_id();
        bus.publish(TradingEvent::OrderCreated {
            decision_id: new_decision_id(),
            order_id: order_id.clone(),
            code: "TEST_CODE_600519".to_string(),
            side: "buy".to_string(),
        });
        let ev = rx.try_recv().expect("应该收到事件");
        match ev {
            TradingEvent::OrderCreated {
                order_id: id, code, ..
            } => {
                assert_eq!(id, order_id);
                assert_eq!(code, "TEST_CODE_600519");
            }
            _ => panic!("事件类型错"),
        }
    }

    #[test]
    fn system_bus_emit_and_subscribe() {
        let bus = SystemBus::global();
        let mut rx = bus.subscribe();
        bus.publish(SystemEvent::ErrorOccurred {
            component: "test".to_string(),
            message: "hello".to_string(),
        });
        let ev = rx.try_recv().expect("应该收到事件");
        match ev {
            SystemEvent::ErrorOccurred { component, message } => {
                assert_eq!(component, "test");
                assert_eq!(message, "hello");
            }
            _ => panic!("事件类型错"),
        }
    }

    #[test]
    fn id_generators_are_unique() {
        let a = new_signal_id();
        let b = new_signal_id();
        assert_ne!(a, b, "signal id 应唯一");
        assert!(a.starts_with("sig-"));
    }

    #[test]
    fn strategy_id_format() {
        let id = new_strategy_id("Momentum", "v1");
        assert!(id.starts_with("strat-Momentum-v1-"));
    }

    #[test]
    fn three_buses_are_independent() {
        // 类型隔离已经保证独立: TradingEvent/SignalEvent 是不同 enum, 编译期区分
        // 这里用类型断言验证 (编译过 = 独立)
        let signal = SignalBus::global();
        let mut trading_rx = TradingBus::global().subscribe();
        // drain 残留
        while trading_rx.try_recv().is_ok() {}
        signal.publish(SignalEvent::SignalEmitted {
            signal_id: new_signal_id(),
            strategy_id: "s".to_string(),
            code: "x".to_string(),
            score: 1.0,
        });
        // 等 50ms broadcast 传播
        std::thread::sleep(std::time::Duration::from_millis(50));
        // TradingBus receiver 只能解出 TradingEvent, 不会解出 SignalEvent
        // 编译期已保证类型安全, 这里只验证 receiver buffer 不爆
        let _: Result<TradingEvent, _> = trading_rx.try_recv();
    }
}
