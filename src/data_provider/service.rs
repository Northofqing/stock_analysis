//! 数据获取服务（进程级缓存，单飞抓取）
//!
//! 目标：消除"快速分析流水线"与"ReAct Agent 工具"之间对同一只股票同一份数据的重复抓取。
//! 例如：pipeline 已抓 250 日 K 线计算筹码分布，Agent `fetch_chip_distribution` 工具
//! 又会再抓一次；财务/资金流/日内分时同理。
//!
//! 设计原则（遵循 AGENTS.md "Simplicity First"）：
//! - 进程级单例（`Lazy`），无 TTL：单次进程运行内不会发生数据"过期"的语义
//! - 每个 cache key 一个 `tokio::sync::OnceCell`，天然单飞（concurrent calls 只抓 1 次）
//! - 只缓存确实出现跨模块复用的字段；新增字段时再扩展，不预先抽象
//! - 缓存值 `Arc<T>`，避免重复克隆大数据（K 线 250 行）

use crate::data_provider::financials::{fetch_with_fallback_async, Financials};
use crate::data_provider::money_flow::{
    fetch_flow_history_async, fetch_intraday_shape_async, IntradayShape, MoneyFlowSummary,
};
use crate::data_provider::{GtimgProvider, HttpProvider, KlineData};
use anyhow::Result;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};

type Slot<T> = Arc<OnceCell<Arc<T>>>;

pub struct DataFetchService {
    client: reqwest::Client,
    klines: Mutex<HashMap<(String, usize), Slot<Vec<KlineData>>>>,
    financials: Mutex<HashMap<String, Slot<Financials>>>,
    money_flow: Mutex<HashMap<(String, usize), Slot<MoneyFlowSummary>>>,
    intraday: Mutex<HashMap<String, Slot<IntradayShape>>>,
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

    async fn slot<K, V>(map: &Mutex<HashMap<K, Slot<V>>>, key: K) -> Slot<V>
    where
        K: std::hash::Hash + Eq + Clone,
    {
        let mut g = map.lock().await;
        g.entry(key)
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone()
    }

    /// 获取 K 线数据（缓存 by `(code, days)`）。
    ///
    /// 多源回落：优先东方财富，失败后回落到腾讯财经。两者皆失败才返回 Err。
    /// 仅成功结果会被 `OnceCell` 缓存；失败时 cell 仍未初始化，下次调用会重新尝试。
    pub async fn get_kline(&self, code: &str, days: usize) -> Result<Arc<Vec<KlineData>>> {
        let cell = Self::slot(&self.klines, (code.to_string(), days)).await;
        let code_owned = code.to_string();
        let client = self.client.clone();
        cell.get_or_try_init(|| async move {
            // 主源：东方财富（内部已含 2 次网络重试）
            match HttpProvider::fetch_kline_data_internal(&client, &code_owned, days).await {
                Ok(data) => Ok::<Arc<Vec<KlineData>>, anyhow::Error>(Arc::new(data)),
                Err(em_err) => {
                    log::warn!(
                        "[DataFetch] {} 东方财富获取失败，回落至腾讯: {}",
                        code_owned, em_err
                    );
                    match GtimgProvider::fetch_kline_data_internal(&client, &code_owned, days).await {
                        Ok(data) => {
                            log::info!(
                                "[DataFetch] {} 腾讯回落成功，{} 条数据",
                                code_owned,
                                data.len()
                            );
                            Ok(Arc::new(data))
                        }
                        Err(gt_err) => Err(anyhow::anyhow!(
                            "K线获取全部失败: 东方财富={}; 腾讯={}",
                            em_err,
                            gt_err
                        )),
                    }
                }
            }
        })
        .await
        .cloned()
    }

    /// 获取最新一期核心财务指标（缓存 by `code`）。
    pub async fn get_financials(&self, code: &str) -> Arc<Financials> {
        let cell = Self::slot(&self.financials, code.to_string()).await;
        let code_owned = code.to_string();
        let client = self.client.clone();
        // fetch_with_fallback_async 内部已 swallow error，返回空 Financials
        cell.get_or_init(|| async move {
            let fin = fetch_with_fallback_async(&client, &code_owned).await;
            Arc::new(fin)
        })
        .await
        .clone()
    }

    /// 获取近 `lmt` 日资金流（缓存 by `(code, lmt)`）。
    pub async fn get_money_flow(&self, code: &str, lmt: usize) -> Arc<MoneyFlowSummary> {
        let cell = Self::slot(&self.money_flow, (code.to_string(), lmt)).await;
        let code_owned = code.to_string();
        let client = self.client.clone();
        cell.get_or_init(|| async move {
            let flow = fetch_flow_history_async(&client, &code_owned, lmt)
                .await
                .unwrap_or_default();
            Arc::new(flow)
        })
        .await
        .clone()
    }

    /// 获取今日日内分时形态（缓存 by `code`）。
    pub async fn get_intraday_shape(&self, code: &str) -> Arc<IntradayShape> {
        let cell = Self::slot(&self.intraday, code.to_string()).await;
        let code_owned = code.to_string();
        let client = self.client.clone();
        cell.get_or_init(|| async move {
            let shape = fetch_intraday_shape_async(&client, &code_owned)
                .await
                .unwrap_or_default();
            Arc::new(shape)
        })
        .await
        .clone()
    }
}

static SERVICE: Lazy<DataFetchService> = Lazy::new(DataFetchService::new);

/// 全局单例访问点。
pub fn service() -> &'static DataFetchService {
    &SERVICE
}
