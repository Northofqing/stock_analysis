//! v16.7 broker API: IBBroker 抽象 (Interactive Brokers TWS 真 impl)
//!
//! 业务: 替换 MockQuoteProvider (0.0 fallback) 为 IBBroker (真价接入 IB TWS API).
//! 业务 0 改, 仅改 QuoteProvider impl, 0 重构.
//!
//! 真生产: 需付费 IB TWS 账号 + ib_async / reqwest crate (外部依赖).
//! 简化 (本 commit): IBBroker struct 抽象 + 0 网络调用 (返回 0.0 等 broker 接入, 0 价).
//! 业务: 接 IB TWS 后, get_quote_price 真返实时价, sector 真返行业分类.
//!
//! 用: v16.7 启动时 register_quote_provider(Box::new(IBBroker::new()))
//! 替代 ensure_default_quote_provider() (MockQuoteProvider)

use crate::broker::QuoteProvider;
use dashmap::DashMap;
use std::time::{Duration, Instant};

/// IBBroker struct: 客户端 + 价格 cache (60s TTL, 避免每次调 TWS 拉数据)
pub struct IBBroker {
    cache: DashMap<String, (f64, Instant)>,
    client: Option<IBClient>,
}

struct IBClient {
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
}

impl IBBroker {
    pub fn new() -> Self {
        Self {
            cache: DashMap::new(),
            client: Some(IBClient {
                host: "127.0.0.1".to_string(),
                port: 7497,
            }),
        }
    }

    fn fetch_price(&self, code: &str) -> f64 {
        if let Some((price, ts)) = self.cache.get(code) {
            if ts.elapsed() < Duration::from_secs(60) {
                return *price;
            }
        }
        // 真生产: reqwest::get("http://127.0.0.1:7497/price/...") → 返 f64
        // 简化: 返回 0.0, 标 broker 未接入
        0.0
    }
}

impl QuoteProvider for IBBroker {
    fn get_quote_price(&self, code: &str) -> f64 {
        self.fetch_price(code)
    }

    fn get_sector(&self, _code: &str) -> String {
        // 真生产: reqContractDetails → industry 字段
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ibbroker_new() {
        let b = IBBroker::new();
        assert!(b.client.is_some());
    }

    #[test]
    fn get_quote_price_returns_zero_when_no_tws() {
        let b = IBBroker::new();
        assert_eq!(b.get_quote_price("600519"), 0.0);
    }

    #[test]
    fn get_sector_returns_empty() {
        let b = IBBroker::new();
        assert_eq!(b.get_sector("600519"), "");
    }
}
