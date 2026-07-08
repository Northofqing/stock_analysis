//! v14.1 task #165: broker 推送接口 stub
//!
//! broker 真实接入前, 任何依赖外部券商推送的能力 (st_type / 价格刷新 / 委托回报)
//! 走 NoopBroker 占位, 不阻塞业务接入. broker SDK 接入后, 换实现即可, 调用方零改动.
//!
//! 设计: 单例 + trait 抽象. 当前 NoopBroker, 后续 QmtBroker / 华泰Broker / ptradeBroker
//! 都按 Broker trait 实现, 启动时按 env / config 选实现.
//!
//! 跟 F7 st_type 关联: trading::open_position 调 `BrokerPush::push_st_type()`
//! 把 broker 推来的 ST 状态写进 stock_position.st_type.

use once_cell::sync::Lazy;
use std::sync::RwLock;

/// broker 推送过来的股票 ST 状态 (跟 push_templates::StType 区分: 这是 broker 数据源)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerStType {
    Normal,
    ST,
    StarST, // *ST
}

impl BrokerStType {
    /// 转 stock_position.st_type 列值 (跟 F7 schema 约定: "ST" / "*ST" / NULL)
    pub fn as_db_value(self) -> Option<&'static str> {
        match self {
            BrokerStType::Normal => None,
            BrokerStType::ST => Some("ST"),
            BrokerStType::StarST => Some("*ST"),
        }
    }
}

/// broker 推送接口 (下单回报 / ST 状态 / 价格刷新)
pub trait BrokerPush: Send + Sync {
    /// broker 推来某只股票的 ST 状态变化 (开盘前 / 状态变更时)
    /// 默认 NoopBroker 实现: log 警告, 不真推.
    fn push_st_type(&self, code: &str, st: BrokerStType);

    /// broker 推来最新报价 (Tick 级)
    fn push_quote(&self, code: &str, price: f64, volume: f64);
}

/// 默认 NoopBroker — broker 未接入时占位
pub struct NoopBroker;

impl BrokerPush for NoopBroker {
    fn push_st_type(&self, code: &str, st: BrokerStType) {
        log::warn!(
            "[broker stub] NoopBroker.push_st_type({code}, {st:?}) — broker 未接入, 仅 log"
        );
    }
    fn push_quote(&self, code: &str, price: f64, volume: f64) {
        log::debug!(
            "[broker stub] NoopBroker.push_quote({code}, {price}, {volume}) — broker 未接入, 仅 log"
        );
    }
}

static BROKER: Lazy<RwLock<Box<dyn BrokerPush>>> = Lazy::new(|| RwLock::new(Box::new(NoopBroker)));

/// 注册 broker 实现 (启动时调一次). 后续 broker SDK 接入后改成具体 impl.
pub fn register(broker: Box<dyn BrokerPush>) {
    let mut guard = BROKER.write().expect("broker lock poisoned");
    *guard = broker;
}

/// caller 短期使用, 用 read 拿 guard 期间数据稳定.
///   broker::with(|b| b.push_st_type("002916", BrokerStType::ST))
pub fn with<F, R>(f: F) -> R
where
    F: FnOnce(&dyn BrokerPush) -> R,
{
    let guard = BROKER.read().expect("broker lock poisoned");
    f(&**guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_broker_does_not_panic() {
        let b = NoopBroker;
        b.push_st_type("002916", BrokerStType::ST);
        b.push_quote("002916", 412.10, 1000.0);
    }

    #[test]
    fn test_st_type_db_value() {
        assert_eq!(BrokerStType::Normal.as_db_value(), None);
        assert_eq!(BrokerStType::ST.as_db_value(), Some("ST"));
        assert_eq!(BrokerStType::StarST.as_db_value(), Some("*ST"));
    }

    #[test]
    fn test_with_closure_runs() {
        with(|b| b.push_st_type("002916", BrokerStType::StarST));
    }
}
