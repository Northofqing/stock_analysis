//! Registered business rules: BR-078, BR-082, BR-137, BR-174.
//! Unified global-news acquisition and receipt-gated notification projection.
//!
//! The old scheduler projected `MarketEvent` and advanced simhash before the
//! source facts had a durable ingress receipt. BR-174 splits that ownership:
//!
//! 1. [`fetch_raw_global_news_batch`] returns typed per-provider terminals and
//!    exact `GlobalNewsRecord + BatchEvidence` without notification effects.
//! 2. The selection ingress owner persists that batch and verifies a receipt.
//! 3. [`project_notifications_after_ingress`] accepts only the sealed
//!    `ReceiptedRawNewsBatch`, then projects BR-082/BR-137 `MarketEvent`s and
//!    advances notification simhash.
//!
//! ## 红线约束
//!
//! - AGENTS.md §§2.1/2.2: provider failure remains `Unavailable`; it is never
//!   converted to a successful empty notification batch.
//! - AGENTS.md §§2.4/2.7: publication, observation, provider and batch identity
//!   remain attached to raw source facts.

#![allow(
    dead_code,
    reason = "BR-174 receipt-gated projection is retained for the selection-v2 release; BR-183 forbids constructing a production receipt while that capability is disabled"
)]

use diesel::RunQueryDsl;
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use stock_analysis::news::aggregator::projection_v2::{
    NotificationProjectionError, NotificationProjectionState, ReceiptedRawNewsBatch,
};
use stock_analysis::news::aggregator::raw_v2::{
    self, registered_global_news_feeds, RawNewsAcquisitionError, RawNewsAggregationBatch,
};
use stock_analysis::signal::market_event::MarketEvent;

static NOTIFICATION_PROJECTION: Lazy<NotificationProjectionState> =
    Lazy::new(NotificationProjectionState::new);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalNewsPipelineRegistration {
    pub feed_count: usize,
    pub registered_feed_set_sha256: String,
}

pub fn uninitialized_global_news_pipeline_registration() -> GlobalNewsPipelineRegistration {
    GlobalNewsPipelineRegistration {
        feed_count: 0,
        registered_feed_set_sha256: sha256_domain(
            "stock_analysis.br196.registered_feed_set.v1",
            b"state=uninitialized\n",
        ),
    }
}

/// Initialize the notification-only state and report the immutable real
/// provider registry size.
pub fn init_global_news_pipeline() -> GlobalNewsPipelineRegistration {
    Lazy::force(&NOTIFICATION_PROJECTION);
    let registrations = registered_global_news_feeds();
    let canonical = registrations
        .iter()
        .map(|feed| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}",
                feed.feed_name,
                feed.gateway_provider,
                feed.provider_id,
                feed.source_contract,
                feed.capability_name,
                feed.max_limit,
                feed.upstream_revision
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let registration = GlobalNewsPipelineRegistration {
        feed_count: registrations.len(),
        registered_feed_set_sha256: sha256_domain(
            "stock_analysis.br196.registered_feed_set.v1",
            canonical.as_bytes(),
        ),
    };
    log::info!(
        "[NewsAggregator][BR-174] raw acquisition + receipted notification projection ready: {} unified Gateway feeds registered",
        registration.feed_count
    );
    registration
}

fn sha256_domain(domain: &str, payload: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(payload);
    format!("{:x}", hasher.finalize())
}

/// Fetch the complete registered real-provider set without constructing
/// notification events or mutating simhash.
pub async fn fetch_raw_global_news_batch(
    per_feed_limit: u32,
) -> Result<RawNewsAggregationBatch, RawNewsAcquisitionError> {
    let batch = raw_v2::fetch_raw_global_news_batch(per_feed_limit).await?;
    log::info!(
        "[NewsAggregator][BR-174] raw batch acquired attempts={} records={} sources_complete={} per_feed_limit={}",
        batch.attempts().len(),
        batch.source_record_count(),
        batch.sources_complete(),
        per_feed_limit
    );
    Ok(batch)
}

/// Project BR-082/BR-137 notification events only after the selection ingress
/// owner has returned a verified receipt capability.
pub fn project_notifications_after_ingress(
    batch: ReceiptedRawNewsBatch,
) -> Result<Vec<MarketEvent>, NotificationProjectionError> {
    let source_batch_content_hash = batch.source_batch_content_hash().to_owned();
    let ingress_receipt_hash = batch.ingress_receipt_hash().to_owned();
    let events = NOTIFICATION_PROJECTION.project_after_ingress(batch)?;
    log::info!(
        "[NewsAggregator][BR-174] receipted notification projection events={} source_batch_content_hash={} ingress_receipt_hash={}",
        events.len(),
        source_batch_content_hash,
        ingress_receipt_hash
    );
    Ok(events)
}

// ============================================================================
// v17.4 §5.1 能力1: NewsFlashGate — critical 即时推 + 4 时段聚合 Top3
// 业务规则登记: BR-082 (docs/business_rules.md, 红线 2.10)
// ============================================================================

/// 4 个聚合窗口 (开盘/午盘收/午盘开/收盘)
const AGG_WINDOWS: [(u32, u32); 4] = [(9, 30), (11, 30), (13, 0), (15, 0)];

/// 窗口触发容差: 窗口时刻起 5 分钟内首个 tick 触发 (news_monitor_loop 轮询默认
/// 120s, spec ±1min 会漏; 加宽到 5min + 当日一次门控, 偏差已在 spec 回填注明)
const AGG_WINDOW_TOLERANCE_SECS: i64 = 300;

/// 聚合决策 (纯数据, 供单测断言)
#[derive(Debug, PartialEq)]
pub enum FlashDecision {
    /// 即时推 (critical): 保留逐事件来源证据；event_id 仅作治理身份，
    /// 不得冒充证券代码。
    Critical {
        event_id: String,
        headline: String,
        source: String,
        observed_at: chrono::DateTime<chrono::Local>,
        source_published_on: chrono::NaiveDate,
        stale: bool,
        strength: u8,
        certainty: u8,
        text: String,
    },
    /// 时段聚合推: (窗口标签, 渲染文本)
    Aggregated(String, String),
}

/// v17.4 §5.1 门控状态机 (纯逻辑, 不做 IO — 推送由 caller 处理)
pub struct NewsFlashGate {
    day: chrono::NaiveDate,
    seen_today: std::collections::HashSet<String>,
    critical_pushed_today: u32,
    /// 每窗口当日是否已触发
    window_fired: [bool; 4],
    /// 当日事件缓冲 (strength, 标题行) — 聚合 Top3 用, 上限 200
    buffer: Vec<(u8, String)>,
}

impl NewsFlashGate {
    pub fn new(today: chrono::NaiveDate) -> Self {
        Self {
            day: today,
            seen_today: std::collections::HashSet::new(),
            critical_pushed_today: 0,
            window_fired: [false; 4],
            buffer: Vec::new(),
        }
    }

    /// 跨天重置 (BR-082: 日桶清零, 防内存增长)
    fn rollover(&mut self, today: chrono::NaiveDate) {
        if self.day != today {
            self.day = today;
            self.seen_today.clear();
            self.critical_pushed_today = 0;
            self.window_fired = [false; 4];
            self.buffer.clear();
            log::info!("[NewsFlashGate] day rollover → {} (buckets reset)", today);
        }
    }

    /// 每 tick 调用: 喂入 dedup 后事件 + 当前时间 → 产出推送决策 (BR-082)
    ///
    /// critical 判定: strength ≥ threshold 且 certainty ≥ 60 (官方性门槛);
    /// 每日上限 max_per_day, 超限 warn 出声 (v15.x 静默路径可见)。
    pub fn process(
        &mut self,
        events: &[MarketEvent],
        now: chrono::DateTime<chrono::Local>,
        critical_threshold: u8,
        max_critical_per_day: u32,
    ) -> Vec<FlashDecision> {
        self.rollover(now.date_naive());
        let mut out = Vec::new();

        // 1. 事件驱动: critical 即时推 (AC34)
        for e in events {
            let provenance = e.provenance.first();
            let validation_error = if e.event_id.trim().is_empty() {
                Some("missing_event_id")
            } else if e.full_title.trim().is_empty() {
                Some("missing_headline")
            } else if provenance.is_none_or(|item| item.provider.trim().is_empty()) {
                Some("missing_provenance")
            } else if e.strength > 100 {
                Some("strength_out_of_range")
            } else if e.certainty > 100 {
                Some("certainty_out_of_range")
            } else if e.stale {
                Some("stale")
            } else if e.occurred_at > now {
                Some("future_publication")
            } else if e.occurred_at.date_naive() != now.date_naive() {
                Some("publication_date_not_current")
            } else if provenance
                .is_some_and(|item| item.fetched_at > now || item.fetched_at < e.occurred_at)
            {
                Some("invalid_observation_time")
            } else {
                None
            };
            if let Some(reason) = validation_error {
                log::warn!(
                    "[NewsFlashGate][BR-137] source event rejected before critical and aggregate governance: {reason}"
                );
                continue;
            }
            if !self.seen_today.insert(e.event_id.clone()) {
                continue; // event_id 当日去重
            }
            // buffer 收集 (聚合用, 上限 200)
            if self.buffer.len() < 200 {
                self.buffer.push((
                    e.strength,
                    format!(
                        "[{}] {} (强度{} 确定性{})",
                        e.event_type.label(),
                        &e.full_title,
                        e.strength,
                        e.certainty
                    ),
                ));
            }
            if e.strength >= critical_threshold && e.certainty >= 60 {
                if self.critical_pushed_today >= max_critical_per_day {
                    log::warn!(
                        "[NewsFlashGate] critical 日上限已满 ({}/{}), 跳过: {}",
                        self.critical_pushed_today,
                        max_critical_per_day,
                        e.subject
                    );
                    continue;
                }
                self.critical_pushed_today += 1;
                let headline = e.full_title.clone();
                let source = provenance
                    .expect("BR-137 provenance validated above")
                    .provider
                    .clone();
                out.push(FlashDecision::Critical {
                    event_id: e.event_id.clone(),
                    headline,
                    source,
                    observed_at: provenance
                        .expect("BR-137 provenance validated above")
                        .fetched_at,
                    source_published_on: e.occurred_at.date_naive(),
                    stale: e.stale,
                    strength: e.strength,
                    certainty: e.certainty,
                    text: assemble_news_flash_critical(
                        &now.format("%H:%M").to_string(),
                        e.event_type.label(),
                        &e.full_title,
                        e.strength,
                        e.certainty,
                        self.critical_pushed_today,
                        max_critical_per_day,
                    ),
                });
            }
        }

        // 2. 4 时段聚合 Top3 (AC35): 窗口时刻起 5min 内首个 tick 触发, 当日一次
        for (i, (h, m)) in AGG_WINDOWS.iter().enumerate() {
            if self.window_fired[i] {
                continue;
            }
            let target = now
                .date_naive()
                .and_hms_opt(*h, *m, 0)
                .expect("valid window time")
                .and_local_timezone(chrono::Local)
                .single();
            let Some(target) = target else { continue };
            let delta = (now - target).num_seconds();
            if (0..AGG_WINDOW_TOLERANCE_SECS).contains(&delta) {
                self.window_fired[i] = true;
                let label = format!("{:02}:{:02}", h, m);
                if self.buffer.is_empty() {
                    // 红线 2.2: 无数据显式说明, 不臆造
                    log::info!("[NewsFlashGate] {} 窗口无事件, 跳过聚合推送", label);
                    continue;
                }
                let mut sorted: Vec<&(u8, String)> = self.buffer.iter().collect();
                sorted.sort_by_key(|item| std::cmp::Reverse(item.0));
                let lines = sorted
                    .iter()
                    .take(3)
                    .map(|(_, line)| line.clone())
                    .collect::<Vec<_>>();
                let text = assemble_news_flash_aggregated(&label, &lines)
                    .expect("nonempty NewsFlash buffer produces a card");
                out.push(FlashDecision::Aggregated(label, text));
            }
        }

        out
    }
}

pub(super) fn assemble_news_flash_critical(
    hhmm: &str,
    event_label: &str,
    headline: &str,
    strength: u8,
    certainty: u8,
    ordinal: u32,
    daily_limit: u32,
) -> String {
    format!(
        "🚨 高分新闻快讯 ({hhmm})\n[{event_label}] {headline}\n强度 {strength} | 确定性 {certainty} | 今日第 {ordinal}/{daily_limit} 条"
    )
}

pub(super) fn assemble_news_flash_aggregated(
    window: &str,
    lines: &[String],
) -> Result<String, String> {
    if window.trim().is_empty()
        || lines.is_empty()
        || lines.iter().any(|line| line.trim().is_empty())
    {
        return Err("BR-196 NewsFlash aggregate requires window and nonempty lines".to_string());
    }
    let mut text = format!("📰 新闻时段聚合 ({window}) Top3:\n");
    for (rank, line) in lines.iter().take(3).enumerate() {
        text.push_str(&format!("{}. {line}\n", rank + 1));
    }
    Ok(text)
}

/// 推送包装: 把 FlashDecision 走现有 push_governor_v3 (L4 dedup: critical 按
/// event_id, 聚合按窗口标签 — 见 BR-082)。返回 (critical 推送数, 聚合推送数)。
pub async fn push_flash_decisions(decisions: Vec<FlashDecision>) -> (usize, usize) {
    let mut n_critical = 0usize;
    let mut n_agg = 0usize;
    for d in decisions {
        match d {
            FlashDecision::Critical {
                event_id,
                headline,
                source,
                observed_at,
                source_published_on,
                stale,
                strength,
                certainty,
                text,
            } => {
                let presentation_token = match crate::presentation_registry::acquire_token(
                    "N-01-news-flash-critical",
                    crate::notify::PushKind::NewsFlashCritical,
                    "news_flash_critical_dispatcher",
                    "assemble_news_flash_critical",
                ) {
                    Ok(token) => token,
                    Err(error) => {
                        log::error!("[NewsFlashGate][BR-196] critical token rejected: {error}");
                        continue;
                    }
                };
                let outcome = match crate::v14_adapter::SourceFactEvidence::new(
                    crate::notify::PushKind::NewsFlashCritical,
                    event_id,
                    None,
                    headline,
                    source,
                    observed_at,
                    Some(source_published_on),
                    strength,
                    certainty,
                    stale,
                ) {
                    Ok(evidence) => {
                        crate::notify::push_presented_source_fact_v3(
                            presentation_token,
                            &text,
                            &evidence,
                        )
                        .await
                    }
                    Err(error) => {
                        log::error!(
                            "[NewsFlashGate][BR-137] critical source fact rejected: {error}"
                        );
                        crate::notify::PushOutcome::Denied(format!("source_fact_invalid:{error}"))
                    }
                };
                if outcome.is_pushed() {
                    n_critical += 1;
                } else {
                    log::info!("[NewsFlashGate] critical 未推 (治理): {:?}", outcome);
                }
            }
            FlashDecision::Aggregated(window, text) => {
                let presentation_token = match crate::presentation_registry::acquire_token(
                    "N-02-news-flash-aggregated",
                    crate::notify::PushKind::NewsFlashAggregated,
                    "news_flash_aggregate_dispatcher",
                    "assemble_news_flash_aggregated",
                ) {
                    Ok(token) => token,
                    Err(error) => {
                        log::error!("[NewsFlashGate][BR-196] aggregate token rejected: {error}");
                        continue;
                    }
                };
                let outcome =
                    crate::notify::push_presented_v3(presentation_token, &text, Some(&window))
                        .await;
                if outcome.is_pushed() {
                    n_agg += 1;
                } else {
                    log::info!("[NewsFlashGate] {} 聚合未推 (治理): {:?}", window, outcome);
                }
            }
        }
    }
    (n_critical, n_agg)
}

/// BR-183 Track A 候选入池: 新闻标题 → LLM 提取受益个股 → pushed_stocks
/// 候选池 (复用 D-01/NewsCatalyst 已注册评分路径, intraday_monitor 直接消费)。
///
/// 当日一票一入: 入池前查 pushed_stocks 当日该 code 是否已有 (DB 级去重,
/// 吸取 T-03 内存去重重启丢失教训)。同票跨 tick 重复新闻不重复入池。
///
/// 红线 2.2: 无真实实时报价 (执行报价新鲜度 ≤5s) 的票不入池, 不造价格。
/// LLM 不可用 / 提取失败 → 出声 warn, 本轮不入池 (v15.x 静默路径可见)。
/// 返回 (入池数, 因无报价/已入池跳过数)。
pub async fn candidate_ingest_from_news(titles: &[String]) -> (usize, usize) {
    if titles.is_empty() {
        return (0, 0);
    }
    let registry = stock_analysis::llm::LlmRegistry::from_env();
    let Some(provider) = registry.select("ticker") else {
        log::warn!("[候选入池][BR-183] LLM role=ticker 无可用 provider (env 未配置), 本轮不入池");
        return (0, 0);
    };
    // 2026-08-08 实测: 20 条标题 → 输出超 8192 tokens 仍可能截断; 10 条
    // 覆盖单轮 tick 的头部新闻, 输出稳定收敛。
    let batch: Vec<String> = titles.iter().take(10).cloned().collect();
    let hits = match stock_analysis::llm::extract_tickers(provider, batch).await {
        Ok(hits) => hits,
        Err(error) => {
            log::warn!("[候选入池][BR-183] LLM 提取失败, 本轮不入池: {error}");
            return (0, 0);
        }
    };
    let mut recorded = 0usize;
    let mut skipped = 0usize;
    for hit in hits {
        if already_pooled_today(&hit.code) {
            skipped += 1;
            log::debug!(
                "[候选入池][BR-183] {}({}) 当日已入池, 跳过 (DB 级去重)",
                hit.name,
                hit.code
            );
            continue;
        }
        match stock_analysis::broker::execution_quote(&hit.code) {
            Ok(quote) => {
                let theme = json_escape(&hit.chain);
                let headline = json_escape(&hit.reason);
                let metric_json = format!(
                    "{{\"push_subkind\":\"NewsCatalyst\",\"theme\":\"{theme}\",\
                     \"headline\":\"{headline}\",\"llm_importance\":{}}}",
                    hit.importance
                );
                let outcome = stock_analysis::signal::push_recorder::record(
                    &stock_analysis::signal::push_recorder::PushRecordMeta {
                        code: hit.code.clone(),
                        name: hit.name.clone(),
                        push_kind: "D-01".to_string(),
                        push_price: quote.price,
                        metric_json,
                        source: "news_flash".to_string(),
                    },
                );
                match outcome {
                    Ok(_) => {
                        recorded += 1;
                        log::info!(
                            "[候选入池][BR-183] {}({}) 入池 importance={} chain={}",
                            hit.name,
                            hit.code,
                            hit.importance,
                            hit.chain
                        );
                    }
                    Err(error) => {
                        log::error!(
                            "[候选入池][BR-183] {}({}) pushed_stocks 写入失败: {error}",
                            hit.name,
                            hit.code
                        );
                    }
                }
            }
            Err(error) => {
                skipped += 1;
                log::warn!(
                    "[候选入池][BR-183] {}({}) 无真实实时报价, 跳过入池 (红线 2.2): {error}",
                    hit.name,
                    hit.code
                );
            }
        }
    }
    log::info!(
        "[候选入池][BR-183] recorded={} skipped={} titles={}",
        recorded,
        skipped,
        titles.len()
    );
    (recorded, skipped)
}

/// pushed_stocks 当日该 code 是否已有 (DB 级当日去重)。
fn already_pooled_today(code: &str) -> bool {
    let Ok(mut conn) = stock_analysis::database::DatabaseManager::get().get_conn() else {
        return false; // 连接失败 → 不拦截, 由写入路径暴露错误
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    diesel::sql_query(
        "SELECT COUNT(*) AS count FROM pushed_stocks \
         WHERE code = ? AND substr(push_time, 1, 10) = ?",
    )
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(&today)
    .get_result::<PooledCountRow>(&mut conn)
    .map(|row| row.count > 0)
    .unwrap_or_else(|error| {
        log::error!("[候选入池][BR-183] 当日去重查询失败: {error}");
        false
    })
}

#[derive(diesel::QueryableByName)]
struct PooledCountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

/// JSON 字符串转义 (候选 metric_json 内联字段用)
fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .chars()
        .take(120)
        .collect()
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use stock_analysis::signal::market_event::{Direction, EventType};

    fn ev(id_seed: &str, strength: u8, certainty: u8) -> MarketEvent {
        let mut e = MarketEvent::new(
            EventType::Policy,
            format!("测试事件-{}", id_seed),
            None,
            Direction::Bull,
            strength,
            certainty,
        );
        e.event_id = format!("eid-{}", id_seed); // 固定 id 便于断言
        e.occurred_at = at(0, 0);
        e.provenance
            .push(stock_analysis::signal::market_event::SourceRef {
                provider: "TEST_CODE_NEWS_PROVIDER".to_string(),
                url: None,
                fetched_at: at(0, 0),
            });
        e
    }

    fn at(h: u32, m: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local::now()
            .date_naive()
            .and_hms_opt(h, m, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .single()
            .unwrap()
    }

    /// AC34 + AC46: 阈值默认 80/certainty 60 门; 低分不推
    #[test]
    fn gate_critical_threshold_and_certainty() {
        let mut g = NewsFlashGate::new(at(10, 0).date_naive());
        let d = g.process(
            &[ev("a", 85, 70), ev("b", 85, 30), ev("c", 60, 90)],
            at(10, 0),
            80,
            20,
        );
        assert_eq!(d.len(), 1, "仅 strength≥80 且 certainty≥60 推");
        assert!(matches!(
            &d[0],
            FlashDecision::Critical {
                event_id,
                source,
                ..
            } if event_id == "eid-a" && source == "TEST_CODE_NEWS_PROVIDER"
        ));
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn br137_critical_flash_pushes_at_data_mode_down_with_event_identity() {
        let _env_guard = crate::TestEnvGuard::dry_run_non_quiet();
        crate::v14_adapter::_reset_dedup_for_test();
        crate::LATEST_BANNER
            .lock()
            .expect("test banner lock")
            .as_mut()
            .expect("test banner")
            .data_mode = crate::push_templates::DataMode::Unsafe;
        let mut gate = NewsFlashGate::new(at(10, 0).date_naive());
        let decisions = gate.process(&[ev("source-fact", 90, 90)], at(10, 0), 80, 20);

        assert_eq!(push_flash_decisions(decisions).await, (1, 0));
    }

    #[test]
    fn br137_stale_flash_is_excluded_from_critical_and_aggregate_buffer() {
        let now = at(10, 0);
        let mut stale = ev("stale-source-fact", 90, 90);
        stale.stale = true;
        let mut gate = NewsFlashGate::new(now.date_naive());
        assert!(gate.process(&[stale], now, 80, 20).is_empty());
        assert!(gate.buffer.is_empty());
        assert!(gate.seen_today.is_empty());
    }

    #[test]
    fn br137_old_flash_is_rejected_even_when_upstream_stale_flag_is_false() {
        let now = at(10, 0);
        let old_time = now - chrono::Duration::days(1);
        let mut old = ev("old-source-fact", 70, 70);
        old.stale = false;
        old.occurred_at = old_time;
        old.provenance[0].fetched_at = old_time;
        let mut gate = NewsFlashGate::new(now.date_naive());
        assert!(gate.process(&[old], now, 80, 20).is_empty());
        assert!(gate.buffer.is_empty());
        assert!(gate.seen_today.is_empty());
    }

    #[test]
    fn br137_malformed_flash_is_excluded_from_critical_and_aggregate_buffer() {
        let now = at(10, 0);
        let mut malformed = ev("malformed-source-fact", 101, 90);
        malformed.event_id.clear();
        malformed.full_title.clear();
        malformed.subject.clear();
        malformed.provenance.clear();
        let mut gate = NewsFlashGate::new(now.date_naive());
        assert!(gate.process(&[malformed], now, 80, 20).is_empty());
        assert!(gate.buffer.is_empty());
        assert!(gate.seen_today.is_empty());
    }

    /// BR-082: event_id 当日去重
    #[test]
    fn gate_dedup_same_event_id() {
        let mut g = NewsFlashGate::new(at(10, 0).date_naive());
        let e = ev("dup", 90, 90);
        assert_eq!(
            g.process(std::slice::from_ref(&e), at(10, 0), 80, 20).len(),
            1
        );
        assert_eq!(
            g.process(&[e], at(10, 1), 80, 20).len(),
            0,
            "同 event_id 当日不重推"
        );
    }

    /// BR-082: 每日上限
    #[test]
    fn gate_daily_cap() {
        let mut g = NewsFlashGate::new(at(10, 0).date_naive());
        let events: Vec<MarketEvent> = (0..5).map(|i| ev(&format!("cap{}", i), 90, 90)).collect();
        let d = g.process(&events, at(10, 0), 80, 3);
        assert_eq!(d.len(), 3, "超 max_critical_per_day=3 截断");
    }

    /// AC35: 窗口触发一次/日 + Top3 按 strength 降序
    #[test]
    fn gate_window_fires_once_with_top3() {
        let mut g = NewsFlashGate::new(at(9, 0).date_naive());
        // 9:00 喂 4 条低分事件 (进 buffer, 不 critical)
        let events: Vec<MarketEvent> = [40u8, 70, 55, 60]
            .iter()
            .enumerate()
            .map(|(i, &s)| ev(&format!("w{}", i), s, 50))
            .collect();
        assert!(g.process(&events, at(9, 0), 80, 20).is_empty());
        // 9:31 → 触发 09:30 窗口
        let d1 = g.process(&[], at(9, 31), 80, 20);
        assert_eq!(d1.len(), 1);
        match &d1[0] {
            FlashDecision::Aggregated(w, text) => {
                assert_eq!(w, "09:30");
                assert!(text.contains("强度70"), "Top1 应是 strength=70: {}", text);
                assert_eq!(text.matches("测试事件").count(), 3, "只取 Top3");
            }
            other => panic!("应为 Aggregated, got {:?}", other),
        }
        // 9:33 再 tick → 同窗口不重复触发
        assert!(g.process(&[], at(9, 33), 80, 20).is_empty(), "窗口当日一次");
    }

    /// Track A: 空标题列表 → 不入池 (0, 0), 不触发 LLM
    #[tokio::test]
    async fn candidate_ingest_empty_titles_is_noop() {
        let (recorded, skipped) = candidate_ingest_from_news(&[]).await;
        assert_eq!((recorded, skipped), (0, 0));
    }

    /// Track A: JSON 转义不产生畸形 metric_json
    #[test]
    fn json_escape_handles_quotes_and_backslashes() {
        let escaped = json_escape("PCB \"涨价\" \\ 事件");
        assert_eq!(escaped, "PCB \\\"涨价\\\" \\\\ 事件");
    }

    /// 红线 2.2: 窗口无事件不臆造推送
    #[test]
    fn gate_window_empty_buffer_no_push() {
        let mut g = NewsFlashGate::new(at(11, 0).date_naive());
        assert!(g.process(&[], at(11, 30), 80, 20).is_empty());
    }

    /// AC46: config 默认值
    #[test]
    fn news_config_defaults() {
        let cfg = stock_analysis::config::MonitorConfig::default();
        assert_eq!(cfg.news_critical_score_threshold, 80);
        assert_eq!(cfg.news_max_critical_per_day, 20);
    }

    #[test]
    fn global_news_pipeline_reports_the_fixed_real_provider_registry() {
        let first = init_global_news_pipeline();
        let second = init_global_news_pipeline();
        assert_eq!(first.feed_count, 4);
        assert_eq!(first, second);
        assert_eq!(first.registered_feed_set_sha256.len(), 64);
    }

    #[tokio::test]
    async fn raw_fetch_rejects_zero_limit_before_provider_work() {
        let error = fetch_raw_global_news_batch(0)
            .await
            .expect_err("TEST_CODE zero limit must fail before provider work");
        assert!(matches!(error, RawNewsAcquisitionError::InvalidLimit(0)));
    }
}
