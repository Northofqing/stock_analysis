//! Registered business rules: BR-057, BR-115, BR-187.
//! 数据获取服务（进程级缓存，单飞抓取，带 TTL）
//!
//! 目标：消除"快速分析流水线"与"ReAct Agent 工具"之间对同一只股票同一份数据的重复抓取。
//! 例如：pipeline 已抓 250 日 K 线计算筹码分布，Agent `fetch_chip_distribution` 工具
//! 又会再抓一次；财务/资金流/日内分时同理。
//!
//! 设计原则（遵循 AGENTS.md "Simplicity First"）：
//! - 进程级单例（`Lazy`），**带 TTL**（修复 2026-06-30 P1）: 盘内 5min / 盘后 1day。
//!   跨日期 process 重启前老缓存不会无限期有效，过期后下次调用重抓。
//!   BR-187 盘中形态例外：缓存携带原始 `source_at`，命中时仍必须满足 5 秒实时门。
//! - 每个 cache key 一个 `tokio::sync::RwLock<Option<(Instant, Arc<V>)>>`，
//!   读时检查 TTL，过期则 invalidate。
//! - 数据获取只委托统一 Gateway；provider 路由、完整性和失败类型由 Gateway
//!   所有，缓存层不得实现第二套源顺序或把失败降级为空批次。
//! - 只缓存确实出现跨模块复用的字段；新增字段时再扩展，不预先抽象。
//! - 缓存值 `Arc<T>`，避免重复克隆大数据（K 线 250 行）。

use crate::capital_flow::{IntradayShape, MoneyFlowSummary};
use crate::company_financials::{project_income_statements, Financials};
use crate::data_gateway::{
    daily_bar_provider_label, CapitalDataGateway, CompanyDataGateway, GatewayBatch,
    HistoricalBarsGateway, IntradayShapeFact, IntradayShapeGateway,
};
use crate::data_provider::KlineData;
use anyhow::Result;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use crate::magic_compat::FlowInterval;
use once_cell::sync::Lazy;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// 缓存条目：value + 写入时间。读时检查 TTL，过期则 invalidate。
type CachedSlot<T> = Arc<RwLock<Option<(Instant, Arc<T>)>>>;

#[derive(Clone)]
struct CachedIntradayShape {
    shape: IntradayShape,
    source_at: DateTime<Utc>,
}

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
    // review #14: 原 Mutex<HashMap<...>> 串行化所有缓存访问, 100 并发请求全排队.
    // 改 DashMap (分片锁): 4 个字段独立分片, 同 key 串行 + 跨 key 并行.
    klines: DashMap<(String, usize), CachedSlot<Vec<KlineData>>>,
    financials: DashMap<String, CachedSlot<Financials>>,
    money_flow: DashMap<(String, usize), CachedSlot<MoneyFlowSummary>>,
    intraday: DashMap<String, CachedSlot<CachedIntradayShape>>,
}

impl DataFetchService {
    fn new() -> Self {
        Self {
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

    async fn cache_financial_result(
        cell: &CachedSlot<Financials>,
        result: Result<Financials>,
    ) -> Result<Arc<Financials>> {
        let value = result?;
        if !value.any() {
            anyhow::bail!("BR-115 financial projection has no supported metric");
        }
        value.require_projection_evidence()?;
        let value = Arc::new(value);
        Self::write_cache(cell, value.clone()).await;
        Ok(value)
    }

    async fn cache_money_flow_result(
        cell: &CachedSlot<MoneyFlowSummary>,
        code: &str,
        result: Result<MoneyFlowSummary>,
    ) -> Result<Arc<MoneyFlowSummary>> {
        let value = result?;
        if value.is_empty() {
            anyhow::bail!("[{code}] 资金流来源成功但返回空批次");
        }
        let value = Arc::new(value);
        Self::write_cache(cell, value.clone()).await;
        Ok(value)
    }

    async fn cache_intraday_result(
        cell: &CachedSlot<CachedIntradayShape>,
        code: &str,
        result: Result<(IntradayShape, DateTime<Utc>)>,
    ) -> Result<Arc<IntradayShape>> {
        let (shape, source_at) = result?;
        if !shape.present {
            anyhow::bail!("[{code}] 分时来源未提供有效形态");
        }
        validate_cached_intraday_freshness(code, source_at)?;
        let value = Arc::new(CachedIntradayShape { shape, source_at });
        Self::write_cache(cell, value.clone()).await;
        Ok(Arc::new(value.shape.clone()))
    }

    async fn read_intraday_cache(
        code: &str,
        cell: &CachedSlot<CachedIntradayShape>,
    ) -> Result<Option<IntradayShape>> {
        let snapshot = {
            let guard = cell.read().await;
            guard.as_ref().map(|(_, value)| value.clone())
        };
        let Some(value) = snapshot else {
            return Ok(None);
        };
        if validate_cached_intraday_freshness(code, value.source_at).is_ok() {
            return Ok(Some(value.shape.clone()));
        }
        *cell.write().await = None;
        Ok(None)
    }

    /// 获取 K 线数据（缓存 by `(code, days)`，带 TTL).
    ///
    /// P1: 盘内 5min / 盘后 1day, 过期自动 invalidate 重抓.
    /// Provider routing and validation are owned by `HistoricalBarsGateway`.
    pub async fn get_kline(&self, code: &str, days: usize) -> Result<Arc<Vec<KlineData>>> {
        let cell = Self::slot(&self.klines, (code.to_string(), days)).await;
        // 1. TTL 读缓存
        if let Some(cached) = Self::read_cache(&cell).await {
            return Ok(Arc::new(cached));
        }
        // 2. cache miss / 过期 → 统一 Gateway。
        let cell_for_write = cell.clone();
        let batch = HistoricalBarsGateway::new()
            .required_daily_bars_async(code, days)
            .await
            .map_err(anyhow::Error::from)?;
        let (data, evidence) = batch.into_parts();
        crate::monitor::data_mode::mark_capability_success(
            crate::monitor::data_mode::Capability::Kline,
        )
        .map_err(anyhow::Error::msg)?;
        let source = daily_bar_provider_label(evidence.provider);
        log::info!(
            "[DataFetch][BR-164] {} OK provider={:?} source={} batch_id={} records={}",
            code,
            evidence.provider,
            source,
            evidence.batch_id,
            data.len(),
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
        let cell_for_write = cell.clone();
        let result = CompanyDataGateway::new()
            .income_statements(&[code.to_string()])
            .await
            .map_err(anyhow::Error::from)
            .and_then(project_income_statements);
        Self::cache_financial_result(&cell_for_write, result).await
    }

    /// 获取近 `lmt` 日资金流（缓存 by `(code, lmt)`，带 TTL).
    pub async fn get_money_flow(&self, code: &str, lmt: usize) -> Result<Arc<MoneyFlowSummary>> {
        let cell = Self::slot(&self.money_flow, (code.to_string(), lmt)).await;
        if let Some(cached) = Self::read_cache(&cell).await {
            return Ok(Arc::new(cached));
        }
        let cell_for_write = cell.clone();
        let limit = u32::try_from(lmt)
            .map_err(|_| anyhow::anyhow!("[{code}] 资金流条数超出 u32 范围: {lmt}"))?;
        let result = CapitalDataGateway::new()
            .instrument_fund_flow(code, FlowInterval::Day1, limit)
            .await
            .map_err(anyhow::Error::from)
            .and_then(MoneyFlowSummary::from_gateway);
        Self::cache_money_flow_result(&cell_for_write, code, result).await
    }

    /// 获取今日日内分时形态（缓存 by `code`，带 TTL).
    pub async fn get_intraday_shape(&self, code: &str) -> Result<Arc<IntradayShape>> {
        let cell = Self::slot(&self.intraday, code.to_string()).await;
        if let Some(cached) = Self::read_intraday_cache(code, &cell).await? {
            return Ok(Arc::new(cached));
        }
        let cell_for_write = cell.clone();
        let result = IntradayShapeGateway::new()
            .current_shape(code)
            .await
            .map_err(anyhow::Error::from)
            .and_then(project_intraday_shape);
        Self::cache_intraday_result(&cell_for_write, code, result).await
    }
}

fn project_intraday_shape(
    batch: GatewayBatch<IntradayShapeFact>,
) -> Result<(IntradayShape, DateTime<Utc>)> {
    let (mut records, evidence) = match batch {
        GatewayBatch::Available { records, evidence } => (records, evidence),
        GatewayBatch::VerifiedEmpty(evidence) => {
            anyhow::bail!(
                "[BR-187] intraday shape verified empty provider={:?} batch_id={}",
                evidence.provider,
                evidence.batch_id
            )
        }
    };
    if records.len() != 1 {
        anyhow::bail!(
            "[BR-187] intraday shape requires exactly one admitted fact, actual={}",
            records.len()
        );
    }
    let fact = records
        .pop()
        .expect("intraday-shape cardinality checked above");
    let source_at = evidence
        .source_at
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("[BR-187] intraday shape source_at is absent"))
        .and_then(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|error| {
                    anyhow::anyhow!(
                        "[BR-187] intraday shape source_at is invalid value={value:?}: {error}"
                    )
                })
        })?;
    Ok((
        IntradayShape {
            date: fact.date,
            pre_close: fact.pre_close,
            open_pct: fact.open_pct,
            high_pct: fact.high_pct,
            low_pct: fact.low_pct,
            close_pct: fact.close_pct,
            amplitude: fact.amplitude,
            tail_30m_pct: fact.tail_30m_pct,
            shape_label: fact.shape_label,
            present: true,
        },
        source_at,
    ))
}

fn validate_cached_intraday_freshness(code: &str, source_at: DateTime<Utc>) -> Result<()> {
    let age_millis = Utc::now()
        .signed_duration_since(source_at)
        .num_milliseconds();
    if !(0..=5_000).contains(&age_millis) {
        anyhow::bail!(
            "[BR-187] cached intraday shape is stale code={code} age_ms={age_millis} max_ms=5000"
        );
    }
    Ok(())
}

static SERVICE: Lazy<DataFetchService> = Lazy::new(DataFetchService::new);

/// 全局单例访问点。
pub fn service() -> &'static DataFetchService {
    &SERVICE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn financial_evidence(source: &str) -> crate::company_financials::FinancialProjectionEvidence {
        crate::company_financials::FinancialProjectionEvidence {
            provider: crate::magic_compat::ProviderId::Sina,
            source: source.to_string(),
            source_at: Some("2026-06-30".to_string()),
            observed_at: "2026-07-18T10:00:00+08:00".to_string(),
            batch_id: "TEST_CODE_FINANCIAL_BATCH".to_string(),
            content_sha256: "a".repeat(64),
        }
    }

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
            source: Some("TEST_CODE_LOCAL_PROTOCOL".to_string()),
            evidence: Some(financial_evidence("TEST_CODE_LOCAL_PROTOCOL")),
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
            days: vec![crate::capital_flow::MoneyFlowDay {
                date: "2026-07-18".to_string(),
                main_net: 10.0,
                xl_net: 4.0,
                big_net: 6.0,
                main_pct: 1.0,
                pct_chg: Some(2.0),
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
        DataFetchService::write_cache(
            &intraday_cell,
            Arc::new(CachedIntradayShape {
                shape: intraday,
                source_at: Utc::now(),
            }),
        )
        .await;
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

    #[tokio::test]
    async fn resolved_provider_results_validate_before_cache_commit() {
        let financial_cell: CachedSlot<Financials> = Arc::new(RwLock::new(None));
        let financial = DataFetchService::cache_financial_result(
            &financial_cell,
            Ok(Financials {
                report_date: Some("2026-06-30".to_string()),
                eps: Some(1.0),
                source: Some("TEST_CODE_真实协议解析".to_string()),
                evidence: Some(financial_evidence("TEST_CODE_真实协议解析")),
                ..Financials::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(financial.eps, Some(1.0));
        assert!(financial_cell.read().await.is_some());

        let failed_financial: CachedSlot<Financials> = Arc::new(RwLock::new(None));
        assert!(DataFetchService::cache_financial_result(
            &failed_financial,
            Err(anyhow::anyhow!("TEST_CODE_财务来源失败")),
        )
        .await
        .is_err());
        assert!(failed_financial.read().await.is_none());
        let missing_evidence: CachedSlot<Financials> = Arc::new(RwLock::new(None));
        assert!(DataFetchService::cache_financial_result(
            &missing_evidence,
            Ok(Financials {
                report_date: Some("2026-06-30".to_string()),
                eps: Some(1.0),
                source: Some("TEST_CODE_真实协议解析".to_string()),
                ..Financials::default()
            }),
        )
        .await
        .is_err());
        assert!(missing_evidence.read().await.is_none());

        let flow_cell: CachedSlot<MoneyFlowSummary> = Arc::new(RwLock::new(None));
        assert!(DataFetchService::cache_money_flow_result(
            &flow_cell,
            "TEST_CODE_000001",
            Ok(MoneyFlowSummary::default()),
        )
        .await
        .is_err());
        assert!(flow_cell.read().await.is_none());
        let flow = MoneyFlowSummary {
            days: vec![crate::capital_flow::MoneyFlowDay {
                date: "2026-07-18".to_string(),
                main_net: 10.0,
                xl_net: 4.0,
                big_net: 6.0,
                main_pct: 1.0,
                pct_chg: Some(2.0),
            }],
        };
        assert_eq!(
            DataFetchService::cache_money_flow_result(&flow_cell, "TEST_CODE_000001", Ok(flow),)
                .await
                .unwrap()
                .days
                .len(),
            1
        );

        let intraday_cell: CachedSlot<CachedIntradayShape> = Arc::new(RwLock::new(None));
        assert!(DataFetchService::cache_intraday_result(
            &intraday_cell,
            "TEST_CODE_000001",
            Ok((IntradayShape::default(), Utc::now())),
        )
        .await
        .is_err());
        assert!(intraday_cell.read().await.is_none());
        let present = IntradayShape {
            date: "2026-07-18".to_string(),
            pre_close: 10.0,
            open_pct: 1.0,
            high_pct: 2.0,
            low_pct: -1.0,
            close_pct: 1.5,
            amplitude: 3.0,
            tail_30m_pct: Some(0.5),
            shape_label: "TEST_CODE_完整形态",
            present: true,
        };
        assert!(
            DataFetchService::cache_intraday_result(
                &intraday_cell,
                "TEST_CODE_000001",
                Ok((present, Utc::now())),
            )
            .await
            .unwrap()
            .present
        );

        let stale_intraday: CachedSlot<CachedIntradayShape> = Arc::new(RwLock::new(Some((
            Instant::now(),
            Arc::new(CachedIntradayShape {
                shape: IntradayShape {
                    present: true,
                    ..IntradayShape::default()
                },
                source_at: Utc::now() - chrono::Duration::milliseconds(5_001),
            }),
        ))));
        assert!(
            DataFetchService::read_intraday_cache("TEST_CODE_000001", &stale_intraday)
                .await
                .unwrap()
                .is_none()
        );
        assert!(stale_intraday.read().await.is_none());
    }

    #[tokio::test]
    async fn rejected_gateway_requests_do_not_populate_caches() {
        let fetch_service = DataFetchService::new();
        let rejected_code = "TEST_CODE_BAD";
        assert!(fetch_service.get_financials(rejected_code).await.is_err());
        assert!(fetch_service
            .get_money_flow(rejected_code, 1)
            .await
            .is_err());
        assert!(fetch_service
            .get_intraday_shape(rejected_code)
            .await
            .is_err());
        let financial_slot = fetch_service
            .financials
            .get(rejected_code)
            .expect("failed request still owns an empty single-flight slot")
            .clone();
        let flow_slot = fetch_service
            .money_flow
            .get(&(rejected_code.to_string(), 1))
            .expect("failed request still owns an empty single-flight slot")
            .clone();
        let intraday_slot = fetch_service
            .intraday
            .get(rejected_code)
            .expect("failed request still owns an empty single-flight slot")
            .clone();
        assert!(financial_slot.read().await.is_none());
        assert!(flow_slot.read().await.is_none());
        assert!(intraday_slot.read().await.is_none());
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
