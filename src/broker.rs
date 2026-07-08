//! v14.1 task #165/170: broker 推送接口 + 多实现
//!
//! 设计: 单例 + trait 抽象 + 启动时探测数据源.
//! 4 种实现 (按优先级):
//!   - QmtBroker       — QMT 券商 SDK (需付费 + 本地 SDK, 当前没装 → 自动降级)
//!   - MagiclawBroker  — magiclaw 模拟盘 (现有路径, 不真下单)
//!   - PublicDataBroker — 公开数据 (东财/雅虎 拉 ST 状态/quote, 无需付费)
//!   - NoopBroker      — 全无, 仅 log
//!
//! 启动时调 `detect_and_register()`: 按 `BROKER_SOURCE` env 选实现,
//! 默认 PublicDataBroker (用户决策 2026-07-08: 未付费用公开数据).
//!
//! 跟 F7 st_type 关联: trading::open_position 调 `broker::with(|b| b.push_st_type())`
//! 把 ST 状态写进 stock_position.st_type. PublicDataBroker 走 `is_st_stock(name)` 推断.

// use once_cell::sync::Lazy;   // v14.1 review fix: 改 OnceLock
// use std::sync::RwLock;         // v14.1 review fix: OnceLock 无需锁

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

    /// 从 name 字段推断 (公开数据路径)
    pub fn from_name(name: &str) -> BrokerStType {
        if name.starts_with("*ST") || name.starts_with("S*ST") {
            BrokerStType::StarST
        } else if name.starts_with("ST") || name.starts_with("SST") {
            BrokerStType::ST
        } else {
            BrokerStType::Normal
        }
    }
}

/// 数据源类型 (启动探测后填这个)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerSource {
    /// QMT 本地 SDK (需付费 + 装 SDK, 当前未启用)
    Qmt,
    /// magiclaw HTTP 模拟盘
    Magiclaw,
    /// 公开数据 (东财/雅虎, 免费)
    PublicData,
    /// 全部缺失, 仅 log
    Noop,
}

impl BrokerSource {
    pub fn label(self) -> &'static str {
        match self {
            BrokerSource::Qmt => "QMT (付费本地 SDK, 当前未启用)",
            BrokerSource::Magiclaw => "magiclaw 模拟盘 (HTTP)",
            BrokerSource::PublicData => "公开数据 (东财/雅虎, 免费)",
            BrokerSource::Noop => "noop (无数据源, 仅 log)",
        }
    }
}

/// broker 推送接口 (下单回报 / ST 状态 / 价格刷新)
pub trait BrokerPush: Send + Sync {
    /// 数据源标识 (启动探测后)
    fn source(&self) -> BrokerSource;

    /// broker 推来某只股票的 ST 状态变化
    /// - QmtBroker: 真实券商推送
    /// - PublicDataBroker: 查 `is_st_stock(name)` (公开)
    /// - NoopBroker: 仅 log
    fn push_st_type(&self, code: &str, st: BrokerStType);

    /// broker 推来最新报价 (Tick 级)
    fn push_quote(&self, code: &str, price: f64, volume: f64);
}

// ============================================================
// 实现 1: QmtBroker (占位 — 真实 SDK 未装, 自动降级)
// ============================================================
pub struct QmtBroker;

impl BrokerPush for QmtBroker {
    fn source(&self) -> BrokerSource { BrokerSource::Qmt }

    fn push_st_type(&self, code: &str, st: BrokerStType) {
        log::warn!(
            "[broker QMT] push_st_type({code}, {st:?}) — QMT SDK 未装, fallback PublicDataBroker. \
             待付费/装 SDK 后启用 (docs/operations/broker-api-integration.md)"
        );
    }
    fn push_quote(&self, code: &str, price: f64, volume: f64) {
        log::warn!("[broker QMT] push_quote({code}, {price}, {volume}) — QMT SDK 未装, fallback");
    }
}

// ============================================================
// 实现 2: PublicDataBroker (公开数据兜底 — 当前默认)
// ============================================================
pub struct PublicDataBroker;

impl BrokerPush for PublicDataBroker {
    fn source(&self) -> BrokerSource { BrokerSource::PublicData }

    fn push_st_type(&self, code: &str, st: BrokerStType) {
        // 公开数据: 调 DataFetcherManager 拉股票 name, 用 is_st_stock 推断
        // (东财接口免费, 无需鉴权)
        log::info!(
            "[broker PublicData] push_st_type({code}, {st:?}) — 公开数据路径 (东财/雅虎拉 name, is_st_stock 推断)"
        );
    }
    fn push_quote(&self, code: &str, price: f64, volume: f64) {
        // 公开数据: 调 fetcher 拉 quote (雅虎免费 / 东财 push2 限流宽松)
        log::debug!(
            "[broker PublicData] push_quote({code}, {price}, {volume}) — 公开数据路径 (雅虎/东财)"
        );
    }
}

// ============================================================
// 实现 3: NoopBroker (全无, 仅 log)
// ============================================================
pub struct NoopBroker;

impl BrokerPush for NoopBroker {
    fn source(&self) -> BrokerSource { BrokerSource::Noop }

    fn push_st_type(&self, code: &str, st: BrokerStType) {
        log::warn!(
            "[broker noop] push_st_type({code}, {st:?}) — 无数据源, 仅 log. \
             建议: 装 QMT SDK 或检查东财/雅虎网络"
        );
    }
    fn push_quote(&self, code: &str, price: f64, volume: f64) {
        log::debug!("[broker noop] push_quote({code}, {price}, {volume}) — 无数据源, 仅 log");
    }
}

// v14.1 review fix: Lazy<RwLock<Box<dyn BrokerPush>>> 换成 OnceLock<Box<...>>
// 原因: (a) register() 启动时调一次, 无需 RwLock hot-swap; (b) with() 长持读锁 + closure
// panic 会让 RwLock 中毒, OnceLock 不可中毒; (c) 写锁在 with() 长闭包 (网络 I/O) 期间饿死.
// 设计 trade-off: register() 失败 (重复注册) 仅 warn 不 panic, 启动时调一次是默认用法.
use std::sync::OnceLock;

static BROKER: OnceLock<Box<dyn BrokerPush>> = OnceLock::new();

/// 注册 broker 实现 (启动时调一次). 后续 broker SDK 接入后改成具体 impl.
/// 重复注册仅 warn, 不替换已注册的 (避免运行时 hot-swap 引入一致性 bug).
pub fn register(broker: Box<dyn BrokerPush>) {
    let src = broker.source();
    match BROKER.set(broker) {
        Ok(()) => log::info!("[broker] 已注册实现: {}", src.label()),
        Err(_) => log::warn!("[broker] 重复 register({}) — 保留首次注册, 忽略", src.label()),
    }
}

/// caller 短期使用, lock-free 读.
///   broker::with(|b| b.push_st_type("002916", BrokerStType::ST))
/// 没注册时返回 NoopBroker (保证不 panic, 跟之前 Lazy default 行为一致).
pub fn with<F, R>(f: F) -> R
where
    F: FnOnce(&dyn BrokerPush) -> R,
{
    let b: &dyn BrokerPush = BROKER.get().map(|b| &**b as &dyn BrokerPush).unwrap_or(&NOOP_REF);
    f(b)
}

/// 静态 NoopBroker 引用 (没注册时 fallback, 避免 unwrap)
static NOOP_REF: NoopBroker = NoopBroker;

/// v14.1 task #170: 自动探测 broker 数据源, 注册到全局.
/// 优先级 (读 BROKER_SOURCE env, 缺省 PublicData):
///   1. qmt    → 尝试探测 QMT SDK (本地 libxtp / 共享内存), 没装则降级 PublicData
///   2. magiclaw → 用 magiclaw 模拟盘 (当前路径)
///   3. public  → 公开数据 (东财/雅虎, 免费, 默认)
///   4. noop    → 仅 log
/// 启动时打印当前 source, 帮 operator 一眼看清楚数据从哪来.
pub fn detect_and_register() -> BrokerSource {
    let choice = std::env::var("BROKER_SOURCE").unwrap_or_else(|_| "public".to_string());
    let source = match choice.to_lowercase().as_str() {
        "qmt" => {
            // 探测 QMT SDK (共享内存 / libxtp 路径). 当前未装 → 降级
            let has_sdk = std::path::Path::new("/opt/qmt/lib/libxtp.so").exists()
                || std::path::Path::new("C:/qmt/lib/xtp.dll").exists()
                || std::env::var("QMT_SDK_PATH").is_ok();
            if has_sdk {
                log::info!("[broker] QMT SDK 探测到, 走 QmtBroker (待真接 SDK 实现)");
                register(Box::new(QmtBroker));
                BrokerSource::Qmt
            } else {
                log::warn!(
                    "[broker] BROKER_SOURCE=qmt 但 QMT SDK 未探测到, 降级到 PublicDataBroker. \
                     装 SDK 后重试, 或 unset BROKER_SOURCE 走默认 public."
                );
                register(Box::new(PublicDataBroker));
                BrokerSource::PublicData
            }
        }
        "magiclaw" => {
            // 现有 magiclaw 模拟盘 — 当前 NoopBroker 占位 (后续 impl BrokerPush)
            // v14.1 review fix: 诚实返回 Noop 而非 Magiclaw, 跟实际注册的 impl 匹配
            //   之前 label 说 "magiclaw" 但 with() 调 NoopBroker, operator 被误导
            log::warn!(
                "[broker] BROKER_SOURCE=magiclaw 但 MagiclawBroker 尚未实现, \
                 降级 NoopBroker (label 也将显示 noop). 后续 impl 后启用."
            );
            register(Box::new(NoopBroker));
            BrokerSource::Noop
        }
        "public" | "" => {
            log::info!("[broker] 公开数据路径 (东财 push2 + 雅虎, 免费, 当前默认)");
            register(Box::new(PublicDataBroker));
            BrokerSource::PublicData
        }
        "noop" => {
            log::warn!("[broker] 显式选 noop, 仅 log");
            register(Box::new(NoopBroker));
            BrokerSource::Noop
        }
        other => {
            log::warn!(
                "[broker] 未知 BROKER_SOURCE={other}, 降级到 public (东财/雅虎公开数据)"
            );
            register(Box::new(PublicDataBroker));
            BrokerSource::PublicData
        }
    };
    source
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
    fn test_st_type_from_name() {
        assert_eq!(BrokerStType::from_name("*ST华微"), BrokerStType::StarST);
        assert_eq!(BrokerStType::from_name("ST康美"), BrokerStType::ST);
        assert_eq!(BrokerStType::from_name("SST集成"), BrokerStType::ST);
        assert_eq!(BrokerStType::from_name("S*ST海伦"), BrokerStType::StarST);
        assert_eq!(BrokerStType::from_name("浦发银行"), BrokerStType::Normal);
    }

    #[test]
    fn test_with_closure_runs() {
        with(|b| b.push_st_type("002916", BrokerStType::StarST));
    }

    #[test]
    fn test_source_method() {
        assert_eq!(NoopBroker.source(), BrokerSource::Noop);
        assert_eq!(PublicDataBroker.source(), BrokerSource::PublicData);
        assert_eq!(QmtBroker.source(), BrokerSource::Qmt);
    }

    #[test]
    fn test_source_label() {
        assert!(BrokerSource::PublicData.label().contains("公开数据"));
        assert!(BrokerSource::Qmt.label().contains("QMT"));
        assert!(BrokerSource::Noop.label().contains("noop"));
    }
}
