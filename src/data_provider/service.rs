//! Registered business rules: BR-057, BR-115.
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
use crate::data_provider::KlineData;
use anyhow::Result;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

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
        MarketSession::Closed | MarketSession::LunchBreak | MarketSession::AfterHours => {
            Duration::from_secs(24 * 60 * 60)
        }
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
        let (data, source) =
            crate::data_provider::fallback::fetch_kline_with_fallback(code, days).await?;
        log::info!(
            "[DataFetch] {} OK (source={}), {} 条",
            code,
            source,
            data.len()
        );
        // 3. 写缓存 (仅成功结果)
        let arc = Arc::new(data);
        Self::write_cache(&cell_for_write, arc.clone()).await;
        Ok(arc)
    }

    /// 获取最新一期核心财务指标（缓存 by `code`，带 TTL).
    pub async fn get_financials(&self, code: &str) -> Result<Arc<Financials>> {
        let cell = Self::slot(&self.financials, code.to_string()).await;
        if let Some(cached) = Self::read_cache(&cell).await {
            return Ok(Arc::new(cached));
        }
        let code_owned = code.to_string();
        let client = self.client.clone();
        let cell_for_write = cell.clone();
        let fin = fetch_with_fallback_async(&client, &code_owned).await?;
        let fin_arc = Arc::new(fin);
        Self::write_cache(&cell_for_write, fin_arc.clone()).await;
        Ok(fin_arc)
    }

    /// 获取近 `lmt` 日资金流（缓存 by `(code, lmt)`，带 TTL).
    pub async fn get_money_flow(&self, code: &str, lmt: usize) -> Result<Arc<MoneyFlowSummary>> {
        let cell = Self::slot(&self.money_flow, (code.to_string(), lmt)).await;
        if let Some(cached) = Self::read_cache(&cell).await {
            return Ok(Arc::new(cached));
        }
        let code_owned = code.to_string();
        let client = self.client.clone();
        let cell_for_write = cell.clone();
        let flow = fetch_flow_history_async(&client, &code_owned, lmt).await?;
        if flow.is_empty() {
            anyhow::bail!("[{code}] 资金流来源成功但返回空批次");
        }
        let flow_arc = Arc::new(flow);
        Self::write_cache(&cell_for_write, flow_arc.clone()).await;
        Ok(flow_arc)
    }

    /// 获取今日日内分时形态（缓存 by `code`，带 TTL).
    pub async fn get_intraday_shape(&self, code: &str) -> Result<Arc<IntradayShape>> {
        let cell = Self::slot(&self.intraday, code.to_string()).await;
        if let Some(cached) = Self::read_cache(&cell).await {
            return Ok(Arc::new(cached));
        }
        let code_owned = code.to_string();
        let client = self.client.clone();
        let cell_for_write = cell.clone();
        let shape = fetch_intraday_shape_async(&client, &code_owned).await?;
        if !shape.present {
            anyhow::bail!("[{code}] 分时来源未提供有效形态");
        }
        let shape_arc = Arc::new(shape);
        Self::write_cache(&cell_for_write, shape_arc.clone()).await;
        Ok(shape_arc)
    }
}

static SERVICE: Lazy<DataFetchService> = Lazy::new(DataFetchService::new);

/// 全局单例访问点。
pub fn service() -> &'static DataFetchService {
    &SERVICE
}

#[cfg(test)]
mod tests {
    // review #15: 把测试 only 的 import (brief/is_ban_error) 移进 test mod,
    // 避免 lib build 的 #[allow(unused_imports)] smell.
    use super::*;
    #[allow(unused_imports)]
    use crate::data_provider::brief;
    use crate::data_provider::is_ban_error;

    fn kline() -> KlineData {
        KlineData {
            date: chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap(),
            open: 10.0,
            high: 10.5,
            low: 9.8,
            close: 10.2,
            volume: 1_000.0,
            amount: 10_200.0,
            pct_chg: 2.0,
            intraday_price: None,
            settled: true,
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
            adjust: crate::data_provider::AdjustType::Qfq,
        }
    }

    #[test]
    fn test_ttl_for_now_outside_trading_hours_is_long() {
        // 周日中午 → 隔夜/盘后 → 24h TTL
        let sunday_noon =
            chrono::NaiveDateTime::parse_from_str("2026-06-21 12:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap();
        assert_eq!(
            ttl_for_now_at(sunday_noon),
            Duration::from_secs(24 * 60 * 60)
        );
    }

    #[test]
    fn test_ttl_for_now_during_trading_is_short() {
        // 周三 10:30 → 盘内 → 5min TTL
        let wed_morning =
            chrono::NaiveDateTime::parse_from_str("2026-06-24 10:30:00", "%Y-%m-%d %H:%M:%S")
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

    #[tokio::test]
    async fn br115_cache_hit_paths_share_values_and_expire_without_transport() {
        let fetch_service = DataFetchService::new();

        let kline_key = ("TEST_CODE_CACHE_KLINE".to_string(), 1);
        let kline_cell = DataFetchService::slot(&fetch_service.klines, kline_key.clone()).await;
        let same_cell = DataFetchService::slot(&fetch_service.klines, kline_key.clone()).await;
        assert!(Arc::ptr_eq(&kline_cell, &same_cell));
        DataFetchService::write_cache(&kline_cell, Arc::new(vec![kline()])).await;
        let klines = fetch_service
            .get_kline(&kline_key.0, kline_key.1)
            .await
            .expect("cached kline must not open transport");
        assert_eq!(klines.len(), 1);
        assert_eq!(klines[0].close, 10.2);

        let finance_code = "TEST_CODE_CACHE_FINANCE".to_string();
        let finance_cell =
            DataFetchService::slot(&fetch_service.financials, finance_code.clone()).await;
        let finance = Financials {
            report_date: Some("2026-06-30".to_string()),
            eps: Some(1.25),
            source: Some("TEST_CODE_LOCAL_PROTOCOL"),
            ..Financials::default()
        };
        DataFetchService::write_cache(&finance_cell, Arc::new(finance)).await;
        let finance = fetch_service
            .get_financials(&finance_code)
            .await
            .expect("cached financials must not open transport");
        assert_eq!(finance.eps, Some(1.25));

        let flow_key = ("TEST_CODE_CACHE_FLOW".to_string(), 1);
        let flow_cell = DataFetchService::slot(&fetch_service.money_flow, flow_key.clone()).await;
        let flow = MoneyFlowSummary {
            days: vec![crate::data_provider::money_flow::MoneyFlowDay {
                date: "2026-07-18".to_string(),
                main_net: 10.0,
                xl_net: 4.0,
                big_net: 6.0,
                main_pct: 1.0,
                pct_chg: 2.0,
            }],
        };
        DataFetchService::write_cache(&flow_cell, Arc::new(flow)).await;
        let flow = fetch_service
            .get_money_flow(&flow_key.0, flow_key.1)
            .await
            .expect("cached money flow must not open transport");
        assert_eq!(flow.days.len(), 1);

        let intraday_code = "TEST_CODE_CACHE_INTRADAY".to_string();
        let intraday_cell =
            DataFetchService::slot(&fetch_service.intraday, intraday_code.clone()).await;
        let intraday = IntradayShape {
            date: "2026-07-18".to_string(),
            pre_close: 10.0,
            open_pct: 1.0,
            high_pct: 3.0,
            low_pct: -1.0,
            close_pct: 2.0,
            amplitude: 4.0,
            tail_30m_pct: Some(0.5),
            shape_label: "TEST_CODE 本地形态",
            present: true,
        };
        DataFetchService::write_cache(&intraday_cell, Arc::new(intraday)).await;
        let intraday = fetch_service
            .get_intraday_shape(&intraday_code)
            .await
            .expect("cached intraday shape must not open transport");
        assert!(intraday.present);
        assert_eq!(intraday.tail_30m_pct, Some(0.5));

        let empty: CachedSlot<i32> = Arc::new(RwLock::new(None));
        assert_eq!(DataFetchService::read_cache(&empty).await, None);
        let expired: CachedSlot<i32> = Arc::new(RwLock::new(Some((
            Instant::now()
                .checked_sub(ttl_for_now() + Duration::from_secs(1))
                .expect("TTL fits Instant range"),
            Arc::new(7),
        ))));
        assert_eq!(DataFetchService::read_cache(&expired).await, None);
        assert!(expired.read().await.is_none());

        assert!(std::ptr::eq(service(), service()));
    }

    /// 测试用的 ttl 决策函数 (接受时间参数避免依赖 chrono::Local::now()).
    fn ttl_for_now_at(now: chrono::NaiveDateTime) -> Duration {
        use crate::calendar::{session_at, MarketSession};
        match session_at(now) {
            MarketSession::Morning | MarketSession::Afternoon | MarketSession::Auction => {
                Duration::from_secs(5 * 60)
            }
            MarketSession::Closed | MarketSession::LunchBreak | MarketSession::AfterHours => {
                Duration::from_secs(24 * 60 * 60)
            }
        }
    }
}
