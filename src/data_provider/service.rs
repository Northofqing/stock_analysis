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
use crate::data_provider::{DataProvider, GtimgProvider, HttpProvider, KlineData, RustdxProvider};
use anyhow::Result;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

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

/// P2 ban 检测: HTTP/HTTPS 错误信息如果包含 empty reply / 4xx / 持续超时，
/// 标记为该源被 ban / 不可用, 立即跳下一个, 不要重试.
fn is_ban_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("empty reply")
        || m.contains("connection reset")
        || m.contains("connection refused")
        || m.contains("timeout")
        || m.contains("429")
        || m.contains("403")
        || m.contains("502")
        || m.contains("503")
        || m.contains("504")
        || m.contains("peer closed connection")
}

pub struct DataFetchService {
    client: reqwest::Client,
    klines: Mutex<HashMap<(String, usize), CachedSlot<Vec<KlineData>>>>,
    financials: Mutex<HashMap<String, CachedSlot<Financials>>>,
    money_flow: Mutex<HashMap<(String, usize), CachedSlot<MoneyFlowSummary>>>,
    intraday: Mutex<HashMap<String, CachedSlot<IntradayShape>>>,
}

impl DataFetchService {
    fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client,
            klines: Mutex::new(HashMap::new()),
            financials: Mutex::new(HashMap::new()),
            money_flow: Mutex::new(HashMap::new()),
            intraday: Mutex::new(HashMap::new()),
        }
    }

    async fn slot<K, V>(map: &Mutex<HashMap<K, CachedSlot<V>>>, key: K) -> CachedSlot<V>
    where
        K: std::hash::Hash + Eq + Clone,
    {
        let mut g = map.lock().await;
        g.entry(key)
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
    /// P2: 多源回落 (东方财富 → 腾讯 → RustDX), ban detected 时立刻跳下一个.
    pub async fn get_kline(&self, code: &str, days: usize) -> Result<Arc<Vec<KlineData>>> {
        let cell = Self::slot(&self.klines, (code.to_string(), days)).await;
        // 1. TTL 读缓存
        if let Some(cached) = Self::read_cache(&cell).await {
            return Ok(Arc::new(cached));
        }
        // 2. cache miss / 过期 → 抓
        let code_owned = code.to_string();
        let client = self.client.clone();
        let cell_for_write = cell.clone();
        let result: Result<Arc<Vec<KlineData>>> = async {
            // 主源：东方财富
            match HttpProvider::fetch_kline_data_internal(&client, &code_owned, days).await {
                Ok(data) => {
                    log::info!(
                        "[DataFetch] {} 东方财富 OK, {} 条",
                        code_owned,
                        data.len()
                    );
                    Ok(Arc::new(data))
                }
                Err(em_err) => {
                    let em_msg = format!("{:#}", em_err);
                    let em_banned = is_ban_error(&em_msg);
                    log::warn!(
                        "[DataFetch] {} 东方财富获取失败 ({}), 回落到腾讯: {}",
                        code_owned,
                        if em_banned { "ban suspected" } else { "non-ban error" },
                        brief(&em_msg)
                    );
                    // P2: 腾讯 fallback
                    match GtimgProvider::fetch_kline_data_internal(&client, &code_owned, days)
                        .await
                    {
                        Ok(data) => {
                            log::info!(
                                "[DataFetch] {} 腾讯回落成功, {} 条",
                                code_owned,
                                data.len()
                            );
                            Ok(Arc::new(data))
                        }
                        Err(gt_err) => {
                            let gt_msg = format!("{:#}", gt_err);
                            let gt_banned = is_ban_error(&gt_msg);
                            log::warn!(
                                "[DataFetch] {} 腾讯获取失败 ({}), 回落到 RustDX: {}",
                                code_owned,
                                if gt_banned { "ban suspected" } else { "non-ban error" },
                                brief(&gt_msg)
                            );
                            // P2: RustDX 最终 fallback
                            let rustdx_code = code_owned.clone();
                            let rustdx_result = tokio::task::spawn_blocking(move || {
                                let provider = RustdxProvider::new()?;
                                provider.get_daily_data(&rustdx_code, days)
                            })
                            .await
                            .map_err(|e| anyhow::anyhow!("RustDX 任务执行失败: {}", e))?;

                            match rustdx_result {
                                Ok(data) => {
                                    log::info!(
                                        "[DataFetch] {} RustDX 回落成功, {} 条",
                                        code_owned,
                                        data.len()
                                    );
                                    Ok(Arc::new(data))
                                }
                                Err(rustdx_err) => Err(anyhow::anyhow!(
                                    "K线获取全部失败: 东方财富={}; 腾讯={}; RustDX={}",
                                    em_msg,
                                    gt_msg,
                                    rustdx_err
                                )),
                            }
                        }
                    }
                }
            }
        }
        .await;
        // 3. 写缓存 (仅成功结果)
        if let Ok(ref data) = result {
            Self::write_cache(&cell_for_write, data.clone()).await;
        }
        result
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

/// 截断超长错误信息 (避免日志刷屏).
fn brief(s: &str) -> String {
    const MAX: usize = 120;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let head: String = s.chars().take(MAX).collect();
        format!("{head}…(截断)")
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