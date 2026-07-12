//! 监控事件总线 — 基于 `tokio::sync::broadcast` 的多消费者发布/订阅。
//!
//! 设计目标（对应 ARCHITECTURE_REVIEW.md F6 "无事件总线"）：
//! 生产者（如告警推送、机会扫描）只负责 `publish`，无需知道有哪些消费者；
//! 新增消费者（持久化、统计、二次风控等）只需 `subscribe`，无需改动生产者代码。
//!
//! 全局单例与项目既有风格一致（`OnceCell`），避免在大量自由函数间穿引用。

use once_cell::sync::OnceCell;
use tokio::sync::broadcast;

/// 监控域事件。新增事件类型不影响既有消费者（消费者按需匹配）。
///
/// 修复 P3.6: 之前只 3 种事件 (Alert/OpportunityScan/Info)
/// 量化分析师要求: 至少 6 种以支持模块解耦:
///   - Alert: 告警
///   - OpportunityScan: 机会扫描
///   - OrderUpdate: 订单状态变化 (持仓/下单)
///   - PriceUpdate: 价格异常变动 (涨跌停/异动)
///   - DataQuality: 数据陈旧/缺失/异常
///   - Info: 通用信息
#[derive(Debug, Clone)]
pub enum MonitorEvent {
    /// 一条告警被推送（含是否成功送达）
    Alert { title: String, success: bool },
    /// 机会扫描完成，附候选数量
    OpportunityScan { candidates: usize },
    /// 修复 P3.6: 订单状态变化 (持仓建立/平仓/止损触发)
    OrderUpdate {
        code: String,
        action: String,
        shares: u64,
    },
    /// 修复 P3.6: 价格异常变动 (涨跌停/异动/突破)
    PriceUpdate {
        code: String,
        change_pct: f64,
        reason: String,
    },
    /// 修复 P3.6: 数据质量事件 (陈旧/缺失/异常)
    /// 用于 P3.5 之后的"指数 ATR 缺失"等数据降级告警
    DataQuality {
        source: String,
        issue: String,
        severity: DataQualityLevel,
    },
    /// 通用信息事件
    Info(String),
}

/// 数据质量严重度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataQualityLevel {
    Warn,  // 数据可用但有偏差 (e.g. ATR 缺失用静态回退)
    Error, // 数据不可用, 功能降级
    Fatal, // 数据源全挂, 必须停机
}

impl MonitorEvent {
    /// 简短标签，用于日志/审计
    pub fn kind(&self) -> &'static str {
        match self {
            MonitorEvent::Alert { .. } => "alert",
            MonitorEvent::OpportunityScan { .. } => "opportunity_scan",
            MonitorEvent::OrderUpdate { .. } => "order_update",
            MonitorEvent::PriceUpdate { .. } => "price_update",
            MonitorEvent::DataQuality { .. } => "data_quality",
            MonitorEvent::Info(_) => "info",
        }
    }
}

/// 事件总线：内部持有一个 broadcast sender，可派生任意数量的订阅者。
pub struct EventBus {
    tx: broadcast::Sender<MonitorEvent>,
}

static BUS: OnceCell<EventBus> = OnceCell::new();

impl EventBus {
    /// 获取全局事件总线（首次访问惰性初始化）。
    pub fn global() -> &'static EventBus {
        BUS.get_or_init(|| {
            let (tx, _rx) = broadcast::channel(256);
            EventBus { tx }
        })
    }

    /// 订阅事件流。每个订阅者拥有独立游标，互不影响。
    pub fn subscribe(&self) -> broadcast::Receiver<MonitorEvent> {
        self.tx.subscribe()
    }

    /// 发布事件。无订阅者时 `send` 返回 `Err`，按设计忽略（fire-and-forget）。
    pub fn publish(&self, event: MonitorEvent) {
        let _ = self.tx.send(event);
    }
}

/// 便捷发布入口：`event_bus::publish(MonitorEvent::Alert { .. })`。
pub fn publish(event: MonitorEvent) {
    EventBus::global().publish(event);
}

/// 便捷订阅入口。
pub fn subscribe() -> broadcast::Receiver<MonitorEvent> {
    EventBus::global().subscribe()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 修复 并行测试隔离: 测试用本地 EventBus 实例, 不共享全局 singleton
    /// 之前: EventBus::global() 是 OnceCell<broadcast::Sender>, 所有测试共享
    /// → 并行测试时, 先跑的 publish 的消息会进同一个 broadcast channel,
    ///   后跑的测试 subscribe 时可能收到前序测试的"幽灵事件"
    fn local_bus() -> EventBus {
        let (tx, _rx) = broadcast::channel(256);
        EventBus { tx }
    }

    #[tokio::test]
    async fn publish_reaches_all_subscribers() {
        // 修复: 用本地 bus, 不与并行测试共享状态
        let bus = local_bus();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.publish(MonitorEvent::Alert {
            title: "测试告警".to_string(),
            success: true,
        });

        let e1 = rx1.recv().await.expect("rx1 应收到事件");
        let e2 = rx2.recv().await.expect("rx2 应收到事件");
        assert_eq!(e1.kind(), "alert");
        assert_eq!(e2.kind(), "alert");
    }

    #[tokio::test]
    async fn publish_without_subscriber_is_ok() {
        // 修复: 无订阅者时不应 panic, 用本地 bus 验证
        let bus = local_bus();
        bus.publish(MonitorEvent::Info("no subscriber".to_string()));
    }
}
