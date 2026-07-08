//! 数据获取服务（进程级缓存，单飞抓取，带 TTL）
//!
//! 目标：消除"快速分析流水线"与"ReAct Agent 工具"之间对同一只股票同一份数据的重复抓取。
//! 例如：pipeline 已抓 250 日 K 线计算筹码分布，Agent `fetch_chip_distribution` 工具
//! 又会再抓一次；财务/资金流/日内分时同理。
//!
//! 设计原则（遵循 AGENTS.md "Simplicity First"）：
//! - 进程级单例（`Lazy`），**带 TTL**（修复 2026-06-30 P1）: 盘内 5min / 盘后 1day。
//!   跨日期 process 重启前老缓存不会无限期有效，过期后下次调用重抓。
//! - 每个 cache key 一个 `tokio::sync::RwLock<Option<(Instant, Arc<V>)>>`，
//!   读时检查 TTL，过期则 invalidate。
//! - 多源回落：东方财富 → 腾讯 → RustDX。**P2 ban 检测**：empty reply / 4xx / 持续超时
//!   立即跳下一个源，不重试已 ban 域。
//! - 只缓存确实出现跨模块复用的字段；新增字段时再扩展，不预先抽象。
//! - 缓存值 `Arc<T>`，避免重复克隆大数据（K 线 250 行）。

use crate::data_provider::financials::{fetch_with_fallback_async, Financials};
use crate::data_provider::money_flow::{
    fetch_flow_history_async, fetch_intraday_shape_async, IntradayShape, MoneyFlowSummary,
};
use crate::data_provider::{is_ban_error, KlineData};
use anyhow::Result;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// 测试代码会引用 brief/is_ban_error. lib build 不需要 (#[allow] 给 test build).
#[allow(unused_imports)]
use crate::data_provider::brief;

/// 缓存条目：value + 写入时间。读时检查 TTL，过期则 invalidate。
type CachedSlot<T> = Arc<RwLock<Option<(Instant, Arc<T>)>>>;

/// 盘内 TTL = 5 分钟。盘后 / 午休 / 隔夜 → TTL = 1 day。
/// 让已收盘的数据活到次日盘前，盘后跑 --review 不需要重抓。
fn ttl_for_now() -> Duration {
    use crate::calendar::{session_at, MarketSession};
    let session = session_at(chrono::Local::now().naive_local());
    match session {
        MarketSession::Morning | MarketSession::Afternoon | MarketSession::Auction => {
            Duration::from_secs(5 * 60)
        }
        MarketSession::Closed
            | MarketSession::LunchBreak
            | MarketSession::AfterHours => Duration::from_secs(24 * 60 * 60),
    }
}

pub struct DataFetchService {
    client: reqwest::Client,
    // review #14: 原 Mutex<HashMap<...>> 串行化所有缓存访问, 100 并发请求全排队.
    // 改 DashMap (分片锁): 4 个字段独立分片, 同 key 串行 + 跨 key 并行.
    klines: DashMap<(String, usize), CachedSlot<Vec<KlineData>>>,
    financials: DashMap<String, CachedSlot<Financials>>,
    money_flow: DashMap<(String, usize), CachedSlot<MoneyFlowSummary>>,
    intraday: DashMap<String, CachedSlot<IntradayShape>>,
}

impl DataFetchService {
    fn new() -> Self {
        // review #15: 复用 SHARED_HTTP_CLIENT (30s timeout + Arc 内核),
        // 替代每次 new Client. 多 DataFetchService 实例 + 频繁 new 会浪费 TLS handshake.
        let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
        Self {
            client,
            klines: DashMap::new(),
            financials: DashMap::new(),
            money_flow: DashMap::new(),
            intraday: DashMap::new(),
        }
    }

    /// 获取或创建 key 对应的 CachedSlot. review #14: DashMap.entry() lock-free fast path.
    async fn slot<K, V>(map: &DashMap<K, CachedSlot<V>>, key: K) -> CachedSlot<V>
    where
        K: std::hash::Hash + Eq + Clone,
    {
        if let Some(cell) = map.get(&key) {
            return cell.clone();
        }
        // entry() 在多线程下可能 race, 但 entry().or_insert_with() 原子 — 谁先到谁 insert.
        map.entry(key)
            .or_insert_with(|| Arc::new(RwLock::new(None)))
            .clone()
    }

    /// 读缓存 + TTL 检查 + 过期失效.
    /// - 命中 + 未过期 → Some(value)
    /// - 命中 + 已过期 → invalidate + None (让上层重抓)
    /// - miss → None
    async fn read_cache<T: Clone>(cell: &CachedSlot<T>) -> Option<T> {
        let snapshot = {
            let g = cell.read().await;
            g.as_ref().map(|(t, v)| (*t, v.clone()))
        };
        let (written_at, value) = snapshot?;
        if written_at.elapsed() < ttl_for_now() {
            Some(value.as_ref().clone())
        } else {
            *cell.write().await = None;
            None
        }
    }

    /// 写缓存. 覆盖已有值 (即使未过期, TTL 重置).
    async fn write_cache<T>(cell: &CachedSlot<T>, value: Arc<T>) {
        *cell.write().await = Some((Instant::now(), value));
    }

    /// 获取 K 线数据（缓存 by `(code, days)`，带 TTL).
    ///
    /// P1: 盘内 5min / 盘后 1day, 过期自动 invalidate 重抓.
    /// P2: 多源回落统一走 `fallback::fetch_kline_with_fallback` (v11 commit 2 抽取共享函数)
    pub async fn get_kline(&self, code: &str, days: usize) -> Result<Arc<Vec<KlineData>>> {
        let cell = Self::slot(&self.klines, (code.to_string(), days)).await;
        // 1. TTL 读缓存
        if let Some(cached) = Self::read_cache(&cell).await {
            return Ok(Arc::new(cached));
        }
        // 2. cache miss / 过期 → 抓 (共享 fallback: 腾讯 → 东财 → RustDX)
        let cell_for_write = cell.clone();
        let (data, source) = crate::data_provider::fallback::fetch_kline_with_fallback(code, days).await?;
        log::info!("[DataFetch] {} OK (source={}), {} 条", code, source, data.len());
        // 3. 写缓存 (仅成功结果)
        let arc = Arc::new(data);
        Self::write_cache(&cell_for_write, arc.clone()).await;
        Ok(arc)
    }

    /// 获取最新一期核心财务指标（缓存 by `code`，带 TTL).
    pub async fn get_financials(&self, code: &str) -> Arc<Financials> {
        let cell = Self::slot(&self.financials, code.to_string()).await;
        if let Some(cached) = Self::read_cache(&cell).await {
            return Arc::new(cached);
        }
        let code_owned = code.to_string();
        let client = self.client.clone();
        let cell_for_write = cell.clone();
        // fetch_with_fallback_async 内部已 swallow error，返回空 Financials
        let fin = fetch_with_fallback_async(&client, &code_owned).await;
        let fin_arc = Arc::new(fin);
        Self::write_cache(&cell_for_write, fin_arc.clone()).await;
        fin_arc
    }

    /// 获取近 `lmt` 日资金流（缓存 by `(code, lmt)`，带 TTL).
    pub async fn get_money_flow(&self, code: &str, lmt: usize) -> Arc<MoneyFlowSummary> {
        let cell = Self::slot(&self.money_flow, (code.to_string(), lmt)).await;
        if let Some(cached) = Self::read_cache(&cell).await {
            return Arc::new(cached);
        }
        let code_owned = code.to_string();
        let client = self.client.clone();
        let cell_for_write = cell.clone();
        let flow = fetch_flow_history_async(&client, &code_owned, lmt)
            .await
            .unwrap_or_default();
        let flow_arc = Arc::new(flow);
        Self::write_cache(&cell_for_write, flow_arc.clone()).await;
        flow_arc
    }

    /// 获取今日日内分时形态（缓存 by `code`，带 TTL).
    pub async fn get_intraday_shape(&self, code: &str) -> Arc<IntradayShape> {
        let cell = Self::slot(&self.intraday, code.to_string()).await;
        if let Some(cached) = Self::read_cache(&cell).await {
            return Arc::new(cached);
        }
        let code_owned = code.to_string();
        let client = self.client.clone();
        let cell_for_write = cell.clone();
        let shape = fetch_intraday_shape_async(&client, &code_owned)
            .await
            .unwrap_or_default();
        let shape_arc = Arc::new(shape);
        Self::write_cache(&cell_for_write, shape_arc.clone()).await;
        shape_arc
    }
}

static SERVICE: Lazy<DataFetchService> = Lazy::new(DataFetchService::new);

/// 全局单例访问点。
pub fn service() -> &'static DataFetchService {
    &SERVICE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ttl_for_now_outside_trading_hours_is_long() {
        // 周日中午 → 隔夜/盘后 → 24h TTL
        let sunday_noon = chrono::NaiveDateTime::parse_from_str(
            "2026-06-21 12:00:00",
            "%Y-%m-%d %H:%M:%S",
        )
        .unwrap();
        assert_eq!(ttl_for_now_at(sunday_noon), Duration::from_secs(24 * 60 * 60));
    }

    #[test]
    fn test_ttl_for_now_during_trading_is_short() {
        // 周三 10:30 → 盘内 → 5min TTL
        let wed_morning = chrono::NaiveDateTime::parse_from_str(
            "2026-06-24 10:30:00",
            "%Y-%m-%d %H:%M:%S",
        )
        .unwrap();
        assert_eq!(ttl_for_now_at(wed_morning), Duration::from_secs(5 * 60));
    }

    #[test]
    fn test_is_ban_error_detects_known_patterns() {
        assert!(is_ban_error("HTTP 429 Too Many Requests"));
        assert!(is_ban_error("Empty reply from server"));
        assert!(is_ban_error("connection timeout after 8s"));
        assert!(is_ban_error("HTTP 502 Bad Gateway"));
        assert!(is_ban_error("peer closed connection"));
        // 不是 ban 的情况
        assert!(!is_ban_error("parse error"));
        assert!(!is_ban_error("NotFound"));
    }

    #[test]
    fn test_brief_truncates_long_error() {
        let long = "a".repeat(200);
        let truncated = brief(&long);
        assert!(truncated.len() < 200);
        assert!(truncated.contains("截断"));
        let short = "short";
        assert_eq!(brief(short), "short");
    }

    /// 测试用的 ttl 决策函数 (接受时间参数避免依赖 chrono::Local::now()).
    fn ttl_for_now_at(now: chrono::NaiveDateTime) -> Duration {
        use crate::calendar::{session_at, MarketSession};
        match session_at(now) {
            MarketSession::Morning | MarketSession::Afternoon | MarketSession::Auction => {
                Duration::from_secs(5 * 60)
            }
            MarketSession::Closed
            | MarketSession::LunchBreak
            | MarketSession::AfterHours => Duration::from_secs(24 * 60 * 60),
    }
}
}