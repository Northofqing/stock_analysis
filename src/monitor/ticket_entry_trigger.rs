//! 入场价格的触发与失效（批 1.4 / 1.5，2026-10-23）。
//!
//! ## 两件事
//!
//! 1. **触发确认（1.4，Q5=昨日最高价）**：`entry_proposed` 的票，现价 ≥ 昨日最高价
//!    且当日涨幅 < 6% → 记 `ticket_event(kind='entry_triggered')` + `candidate_trigger_selection`，
//!    并推一条「突破确认」。1 票/日只推一次（`(trigger_date, code)` 主键天然幂等）。
//! 2. **价格失效（1.5）**：现价跌破放弃价 → `kind='entry_invalidated'`；
//!    时效到期（趋势型 > 3 个交易日、情绪型跨日未触发）→ `kind='entry_expired'`。
//!    **只作废价格，不淘汰票** —— stage 不动，淘汰仍归 `ticket_eviction`。
//!
//! ## 为什么缓存
//!
//! 扫描每 30 秒一轮，每轮都对每只票重算 `buy_zone`（含日线聚合）会把主库读爆。
//! 每日 09:26 / 13:01 预热一次内存 `HashMap`，扫描只比价。
//! 预热失败（日线缺失）的票不参与 —— 没有放弃价就没有失效判据，不编造（BR-222）。
//!
//! ## 边界
//!
//! - 实时行情合同**没有量比字段**（2026-08-31 注释），所以不加量能条件；
//! - 无分钟线，触发后的走势无法回测，上线后按方案观察两周；
//! - 「所属板块退出 top10」这一条失效条件**没有实现**：`board_member` 只覆盖采集成功的板块，
//!   「表里没有」≠「不属于」，用它作废价格会误杀；明日计划已有 C 规则做同样的过滤。

use std::collections::{HashMap, HashSet};

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use crate::data_gateway::grpc_source;
use crate::data_gateway::{GatewayBatch, RealtimeMarketQuote};
use crate::database::{ticket_lifecycle, DatabaseManager};
use crate::monitor::buy_zone;
use crate::monitor::ticket_decision::{
    self, Decision, DecisionContext, EntryMode, EntryPolicy, EvidenceGrade, ProposalStatus,
    QuoteView, SourceStatus,
};
use crate::monitor::ticket_regime::{self, Regime};

/// 触发确认的涨幅上限：涨幅超过它说明已经拉到高位，突破买点没有意义。
const MAX_TRIGGER_CHANGE_PCT: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerPolicyMode {
    Legacy,
    Shadow,
    New,
}

fn trigger_policy_mode() -> TriggerPolicyMode {
    match std::env::var("TICKET_ENTRY_POLICY")
        .unwrap_or_else(|_| "legacy".to_owned())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "new" => TriggerPolicyMode::New,
        "shadow" => TriggerPolicyMode::Shadow,
        _ => TriggerPolicyMode::Legacy,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitAction {
    /// 现价站上昨日最高价
    Triggered,
    /// 现价跌破放弃价
    Invalidated,
}

impl HitAction {
    pub fn as_str(self) -> &'static str {
        match self {
            HitAction::Triggered => "triggered",
            HitAction::Invalidated => "invalidated",
        }
    }
}

/// 一只票的盘中判据（预热得到，不随扫描变化）。
#[derive(Debug, Clone)]
pub struct TriggerTarget {
    pub ticket_id: i64,
    pub code: String,
    pub name: String,
    pub prev_high: f64,
    pub entry_low: f64,
    pub entry_high: f64,
    pub abandon: Option<f64>,
    pub regime: Regime,
    /// `None` = 旧提案还没有 context；只允许 legacy 路径使用。
    pub context: Option<DecisionContext>,
}

impl TriggerTarget {
    pub fn entry_mode(&self) -> EntryMode {
        self.context
            .as_ref()
            .map(|context| context.entry_mode)
            .unwrap_or(EntryMode::Unknown)
    }

    pub fn trigger_level(&self) -> Option<f64> {
        self.context
            .as_ref()
            .and_then(|context| context.trigger_level)
    }

    pub fn decision_id(&self) -> Option<&str> {
        self.context
            .as_ref()
            .map(|context| context.decision_id.as_str())
            .filter(|decision_id| !decision_id.is_empty())
    }

    pub fn source_health(&self) -> &[SourceStatus] {
        self.context
            .as_ref()
            .map(|context| context.source_health.as_slice())
            .unwrap_or(&[])
    }

    pub fn evidence_grade(&self) -> EvidenceGrade {
        self.context
            .as_ref()
            .map(|context| context.evidence_grade)
            .unwrap_or(EvidenceGrade::L1)
    }
}

#[derive(Debug, Default)]
pub struct TriggerCache {
    pub as_of: String,
    pub targets: Vec<TriggerTarget>,
    /// 预热时被跳过的票（无日线 / 无买点），只用于日志计数
    pub skipped: Vec<String>,
}

impl TriggerCache {
    /// 当日有没有可看的票（没有就不必拉行情）。
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    pub fn codes(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.targets
            .iter()
            .filter_map(|item| {
                let code = item.code.trim();
                if code.is_empty() || !seen.insert(code.to_owned()) {
                    None
                } else {
                    Some(code.to_owned())
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct TriggerHit {
    pub ticket_id: i64,
    pub code: String,
    pub name: String,
    pub price: f64,
    pub change_pct: f64,
    pub prev_high: f64,
    pub entry_low: f64,
    pub entry_high: f64,
    pub abandon: Option<f64>,
    pub regime: Regime,
    pub action: HitAction,
    pub decision_id: Option<String>,
    pub entry_mode: EntryMode,
    pub trigger_level: Option<f64>,
}

impl TriggerHit {
    pub fn basis(&self) -> String {
        match self.action {
            HitAction::Triggered => match self.entry_mode {
                EntryMode::Pullback => format!(
                    "进入参考区间 {:.2}-{:.2}（现价 {:.2}，{:+.2}%）",
                    self.entry_low, self.entry_high, self.price, self.change_pct
                ),
                EntryMode::Breakout => self.trigger_level.map_or_else(
                    || {
                        format!(
                            "突破昨高 {:.2}（现价 {:.2}，{:+.2}%）；区间 {:.2}-{:.2}",
                            self.prev_high,
                            self.price,
                            self.change_pct,
                            self.entry_low,
                            self.entry_high
                        )
                    },
                    |level| {
                        format!(
                            "站上触发位 {level:.2}（现价 {:.2}，{:+.2}%）；区间 {:.2}-{:.2}",
                            self.price, self.change_pct, self.entry_low, self.entry_high
                        )
                    },
                ),
                EntryMode::AuctionReference | EntryMode::Unknown => format!(
                    "突破昨高 {:.2}（现价 {:.2}，{:+.2}%）；区间 {:.2}-{:.2}",
                    self.prev_high, self.price, self.change_pct, self.entry_low, self.entry_high
                ),
            },
            HitAction::Invalidated => format!(
                "跌破放弃价 {:.2}（现价 {:.2}）",
                self.abandon.unwrap_or(0.0),
                self.price
            ),
        }
    }

    pub fn render(&self) -> String {
        let head = match self.action {
            HitAction::Triggered => "⚡ 突破确认",
            HitAction::Invalidated => "⚠ 价格失效",
        };
        format!(
            "{head} {} {} 现价 {:.2}（{:+.2}%）— {}｜{}",
            self.code,
            self.name,
            self.price,
            self.change_pct,
            self.regime.label(),
            self.basis()
        )
    }
}

fn prev_high_before(
    conn: &mut SqliteConnection,
    code: &str,
    as_of: &str,
) -> Result<Option<f64>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Nullable<Double>)]
        high: Option<f64>,
    }
    let row = diesel::sql_query(
        "SELECT high FROM stock_daily \
          WHERE code = ?1 AND date < ?2 AND high IS NOT NULL \
          ORDER BY date DESC LIMIT 1",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(as_of)
    .get_result::<Row>(conn)
    .optional()
    .map_err(|error| format!("昨高查询失败 ({code}): {error}"))?;
    Ok(row.and_then(|row| row.high))
}

/// 最近一次入场参考之后是否已经关闭（失效/到期/被新提案取代）。
///
/// `entry_invalidated` 只作废这一次价格窗口，不淘汰票；旧实现把它当成
/// 纯日志：warm 仍把票放进缓存，价格回升后还能再触发，expiry 也会每天重复写。
/// 新 proposal 事件会把窗口重新打开，所以比较的是「最近一次 proposal」。
fn invalidated_after_latest_proposal(
    conn: &mut SqliteConnection,
    ticket_id: i64,
) -> Result<bool, String> {
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    let row: CountRow = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM ticket_event i \
          WHERE i.ticket_id = ?1 \
            AND i.event_kind IN ('entry_invalidated','entry_expired','entry_superseded') \
            AND i.event_at >= COALESCE((\
                  SELECT MAX(p.event_at) FROM ticket_event p \
                   WHERE p.ticket_id = ?1 AND p.event_kind = 'entry_proposed'\
                ), '')",
    )
    .bind::<BigInt, _>(ticket_id)
    .get_result(conn)
    .map_err(|error| format!("入场失效事件查询失败: {error}"))?;
    Ok(row.n > 0)
}

/// Shadow 模式下给旧 proposal 做只读重建：只用于比较，不写库、不参与 new 模式。
fn legacy_shadow_context(
    ticket_id: i64,
    code: &str,
    source: &str,
    as_of: &str,
    regime: Regime,
    zone: &buy_zone::BuyZone,
) -> DecisionContext {
    let mut context = DecisionContext::new(
        ticket_id,
        code,
        as_of,
        source,
        regime.as_str(),
        ticket_decision::entry_mode_for(regime, zone.trend, source),
    );
    context.price_low = zone.entry_low;
    context.price_high = zone.entry_high;
    context.trigger_level = zone.ma5;
    context.abandon = zone.abandon;
    context.confirmation = "price_only".to_owned();
    context.valid_from = Some(as_of.to_owned());
    context.missing_fields.push("volume_ratio".to_owned());
    context.missing_fields.push("legacy_backfill".to_owned());
    context.seal();
    // 这是 shadow 比较用的临时 context，没有写入 ticket_decision；
    // 清空 decision_id，避免 record() 去更新一条不存在的决策行。
    context.decision_id.clear();
    context
}

/// 从 `entry_proposed` 事件摘要解析提案区间：
/// `"22.00-23.00｜回踩 ma10｜放弃 20.00"` → `(22.0, 23.0)`。
/// 只接受正数且 high >= low；解析失败返回 None，不猜。
fn parse_proposed_range(summary: &str) -> Option<(f64, f64)> {
    let first = summary.split('｜').next()?.trim();
    let (low, high) = first.split_once('-')?;
    let low: f64 = low.trim().parse().ok()?;
    let high: f64 = high.trim().parse().ok()?;
    (low > 0.0 && high >= low).then_some((low, high))
}

fn latest_proposed_range(
    conn: &mut SqliteConnection,
    ticket_id: i64,
) -> Result<Option<(f64, f64)>, String> {
    #[derive(QueryableByName)]
    struct SummaryRow {
        #[diesel(sql_type = Nullable<Text>)]
        summary: Option<String>,
    }
    let row: Option<SummaryRow> = diesel::sql_query(
        "SELECT summary FROM ticket_event \
         WHERE ticket_id = ?1 AND event_kind = 'entry_proposed' \
         ORDER BY event_at DESC, id DESC LIMIT 1",
    )
    .bind::<BigInt, _>(ticket_id)
    .get_result::<SummaryRow>(conn)
    .optional()
    .map_err(|error| format!("entry_proposed 摘要查询失败: {error}"))?;
    Ok(row
        .and_then(|row| row.summary)
        .as_deref()
        .and_then(parse_proposed_range))
}

/// 预热当日判据：`entry_proposed` 的票 → 昨高 + buy_zone 区间 + 分型。
pub fn warm(db: &DatabaseManager, as_of: &str) -> Result<TriggerCache, String> {
    chrono::NaiveDate::parse_from_str(as_of, "%Y-%m-%d")
        .map_err(|error| format!("as_of 非法 ({as_of}): {error}"))?;

    #[derive(QueryableByName)]
    struct TicketRow {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        source: String,
    }

    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let tickets: Vec<TicketRow> = diesel::sql_query(
        "SELECT id, code, source FROM ticket_lifecycle \
          WHERE close_date IS NULL AND stage = 'entry_proposed' \
          ORDER BY code, open_date DESC, id DESC",
    )
    .load(&mut conn)
    .map_err(|error| format!("entry_proposed 票查询失败: {error}"))?;

    let mut cache = TriggerCache {
        as_of: as_of.to_string(),
        ..Default::default()
    };
    let mut seen_codes = HashSet::new();
    let policy_mode = trigger_policy_mode();
    log::info!("[触发确认] policy_mode={policy_mode:?}");

    for ticket in tickets {
        let code = ticket.code.trim().to_owned();
        if code.is_empty() || !seen_codes.insert(code.clone()) {
            cache.skipped.push(format!("{} 重复/空活跃票", ticket.code));
            continue;
        }
        if invalidated_after_latest_proposal(&mut conn, ticket.id)? {
            cache.skipped.push(format!("{code} 已有失效价格窗口"));
            continue;
        }
        let Some(prev_high) = prev_high_before(&mut conn, &code, as_of)? else {
            cache.skipped.push(format!("{code} 无昨高"));
            continue;
        };
        let zone = match buy_zone::compute(&mut conn, &code, as_of) {
            Ok(Some(zone)) => zone,
            Ok(None) => {
                cache.skipped.push(format!("{code} 无日线"));
                continue;
            }
            Err(error) => {
                cache.skipped.push(format!("{code} 买点失败: {error}"));
                continue;
            }
        };
        let (entry_low, entry_high) = match (zone.entry_low, zone.entry_high) {
            (Some(low), Some(high)) if low > 0.0 && high >= low => (low, high),
            _ => match latest_proposed_range(&mut conn, ticket.id)? {
                Some((low, high)) => {
                    log::debug!(
                        "[触发确认] {code} 当日 buy_zone 无区间，回退提案原文区间 {low:.2}-{high:.2}"
                    );
                    (low, high)
                }
                None => {
                    cache.skipped.push(format!("{code} 无入场区间"));
                    continue;
                }
            },
        };
        let regime =
            match ticket_regime::resolve(&mut conn, ticket.id, &code, &ticket.source, as_of) {
                Ok(decision) => decision.regime,
                Err(error) => {
                    cache.skipped.push(format!("{code} 分型失败: {error}"));
                    continue;
                }
            };
        let loaded_context = if policy_mode == TriggerPolicyMode::Legacy {
            None
        } else {
            match ticket_decision::latest_open_proposal(&mut conn, ticket.id) {
                Ok(context) => context,
                Err(error) => {
                    cache
                        .skipped
                        .push(format!("{code} decision context 查询失败: {error}"));
                    None
                }
            }
        };
        let context = match loaded_context {
            Some(context) => Some(context),
            None if policy_mode == TriggerPolicyMode::Shadow => Some(legacy_shadow_context(
                ticket.id,
                &code,
                &ticket.source,
                as_of,
                regime,
                &zone,
            )),
            None => None,
        };
        if context.is_none() && policy_mode == TriggerPolicyMode::New {
            cache
                .skipped
                .push(format!("{code} legacy_context_missing（new 模式不猜触发）"));
            continue;
        }
        let (target_entry_low, target_entry_high, target_abandon) = match &context {
            Some(context) => (
                context.price_low.unwrap_or(entry_low),
                context.price_high.unwrap_or(entry_high),
                context.abandon.or(zone.abandon),
            ),
            None => (entry_low, entry_high, zone.abandon),
        };
        cache.targets.push(TriggerTarget {
            ticket_id: ticket.id,
            name: zone.name.clone().unwrap_or_else(|| code.clone()),
            code,
            prev_high,
            entry_low: target_entry_low,
            entry_high: target_entry_high,
            abandon: target_abandon,
            regime,
            context,
        });
    }

    Ok(cache)
}

/// 旧触发判据。保留为 legacy 开关的回归基线，不再作为默认新逻辑。
fn judge_legacy(target: &TriggerTarget, price: f64, change_pct: f64) -> Option<HitAction> {
    if price <= 0.0 {
        return None;
    }
    if let Some(abandon) = target.abandon {
        if price < abandon {
            return Some(HitAction::Invalidated);
        }
    }
    if price >= target.prev_high && change_pct < MAX_TRIGGER_CHANGE_PCT {
        return Some(HitAction::Triggered);
    }
    None
}

/// 旧测试和 legacy 路径使用的名字。
fn judge(target: &TriggerTarget, price: f64, change_pct: f64) -> Option<HitAction> {
    judge_legacy(target, price, change_pct)
}

/// 新策略结果 + 原因；没有 context 时返回 NoDecision 原因，不猜。
fn judge_policy(
    target: &TriggerTarget,
    price: f64,
    change_pct: f64,
) -> (Option<HitAction>, &'static str) {
    let Some(context) = target.context.as_ref() else {
        return (None, "legacy_context_missing");
    };
    let outcome = EntryPolicy::evaluate(context, QuoteView { price, change_pct });
    let action = match outcome.decision {
        Decision::Triggered => Some(HitAction::Triggered),
        Decision::Invalidated => Some(HitAction::Invalidated),
        Decision::Expired | Decision::NoDecision => None,
    };
    (action, outcome.reason)
}

/// 拉一次行情并判定。行情不可用时返回错误（调用方记 warn、下一轮再试）。
pub async fn evaluate(cache: &TriggerCache) -> Result<Vec<TriggerHit>, String> {
    if cache.is_empty() {
        return Ok(Vec::new());
    }
    let policy_mode = trigger_policy_mode();
    let codes = cache.codes();
    let bridge = grpc_source::bridge_for("RealtimeQuotes").map_err(|error| error.to_string())?;
    let quotes: Vec<RealtimeMarketQuote> = match bridge.realtime_quotes_async(&codes).await {
        Ok(GatewayBatch::Available { records, .. }) => records,
        Ok(GatewayBatch::VerifiedEmpty(_)) => Vec::new(),
        Err(error) => {
            // provider 对整批（含个别停牌/陈旧代码）可能回 no_verified_batch；
            // 触发确认不能因为一只坏票或批次上限整组不跑。二分隔离后继续；
            // 只有全部代码都被隔离时才报错。
            let reason = error.reason_code().to_owned();
            let isolated = bridge.realtime_quotes_with_isolation(&codes).await;
            log::warn!(
                "[触发确认] RealtimeQuotes 被 {} 拒绝, 二分隔离后取到 {} 只 / 隔离 {} 只 / 请求 {} 次",
                reason,
                isolated.quotes.len(),
                isolated.isolated.len(),
                isolated.requests
            );
            if isolated.quotes.is_empty() {
                return Err(format!("行情拉取失败: {error}"));
            }
            isolated.quotes
        }
    };
    let by_code: HashMap<&str, &RealtimeMarketQuote> = quotes
        .iter()
        .map(|quote| (quote.code.as_str(), quote))
        .collect();

    let mut hits = Vec::new();
    for target in &cache.targets {
        let Some(quote) = by_code.get(target.code.as_str()) else {
            continue;
        };
        let legacy = judge_legacy(target, quote.price, quote.change_percent);
        let (policy_action, policy_reason) =
            judge_policy(target, quote.price, quote.change_percent);
        let action = match policy_mode {
            TriggerPolicyMode::Legacy => legacy,
            TriggerPolicyMode::Shadow => {
                if legacy != policy_action {
                    log::info!(
                        "[触发确认][shadow] code={} policy={:?} old={:?} new={:?} reason={} mode={} price={:.2} change={:.2}",
                        target.code,
                        policy_mode,
                        legacy,
                        policy_action,
                        policy_reason,
                        target.entry_mode().as_str(),
                        quote.price,
                        quote.change_percent
                    );
                }
                legacy
            }
            TriggerPolicyMode::New => policy_action,
        };
        let Some(action) = action else {
            continue;
        };
        hits.push(TriggerHit {
            ticket_id: target.ticket_id,
            code: target.code.clone(),
            name: target.name.clone(),
            price: quote.price,
            change_pct: quote.change_percent,
            prev_high: target.prev_high,
            entry_low: target.entry_low,
            entry_high: target.entry_high,
            abandon: target.abandon,
            regime: target.regime,
            action,
            decision_id: target.decision_id().map(str::to_owned),
            entry_mode: target.entry_mode(),
            trigger_level: target.trigger_level(),
        });
    }
    Ok(hits)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordOutcome {
    /// 本次真的写入了（应当推送）
    Recorded,
    /// 当日已经记过（不推送）
    Duplicate,
}

#[derive(Debug)]
enum RecordTxError {
    Db(diesel::result::Error),
    Message(String),
}

impl From<diesel::result::Error> for RecordTxError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Db(error)
    }
}

impl From<String> for RecordTxError {
    fn from(error: String) -> Self {
        Self::Message(error)
    }
}

impl std::fmt::Display for RecordTxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db(error) => write!(formatter, "{error}"),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for RecordTxError {}

/// 某票当日是否已经记录过入场触发。事件是唯一事实，不能拿候选表的
/// 精确 basis 做幂等键 —— 现价每轮变化会让同一票重复追加事件。
fn entry_triggered_exists(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    as_of: &str,
) -> Result<bool, String> {
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    let row: CountRow = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM ticket_event \
         WHERE ticket_id = ?1 AND event_kind = 'entry_triggered' AND date(event_at) = date(?2)",
    )
    .bind::<BigInt, _>(ticket_id)
    .bind::<Text, _>(as_of)
    .get_result(conn)
    .map_err(|error| format!("entry_triggered 去重查询失败: {error}"))?;
    Ok(row.n > 0)
}

/// 把一次命中写进库。同日同票同动作只记一次。
pub fn record(
    db: &DatabaseManager,
    as_of: &str,
    hit: &TriggerHit,
) -> Result<RecordOutcome, String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    match hit.action {
        HitAction::Triggered => {
            let basis = hit.basis();
            if entry_triggered_exists(&mut conn, hit.ticket_id, as_of)? {
                return Ok(RecordOutcome::Duplicate);
            }
            conn.transaction::<(), RecordTxError, _>(|conn| {
                ticket_lifecycle::append_event(
                    conn,
                    hit.ticket_id,
                    as_of,
                    "entry_triggered",
                    None,
                    hit.decision_id.as_deref(),
                    Some(&basis),
                )?;
                if let Some(decision_id) = hit.decision_id.as_deref() {
                    let changed = ticket_decision::update_proposal_status(
                        conn,
                        decision_id,
                        ProposalStatus::Triggered,
                    )?;
                    if !changed {
                        return Err(format!(
                            "entry_triggered 已记账但 ticket_decision 不存在: {decision_id}"
                        )
                        .into());
                    }
                }
                Ok(())
            })
            .map_err(|error| error.to_string())?;
            // 事件提交成功后再写候选行；这样失败的事务不会留下孤儿候选。
            db.record_candidate_trigger(as_of, &hit.code, &hit.name, &basis)?;
            Ok(RecordOutcome::Recorded)
        }
        HitAction::Invalidated => {
            // 摘要不带现价：现价每轮变，带进去等于永远判不出重复。
            let summary = format!("价格失效：跌破放弃价 {:.2}", hit.abandon.unwrap_or(0.0));
            if ticket_lifecycle::event_exists_same_day(
                &mut conn,
                hit.ticket_id,
                "entry_invalidated",
                as_of,
                &summary,
            )? {
                return Ok(RecordOutcome::Duplicate);
            }
            conn.transaction::<(), RecordTxError, _>(|conn| {
                ticket_lifecycle::append_event(
                    conn,
                    hit.ticket_id,
                    as_of,
                    "entry_invalidated",
                    None,
                    hit.decision_id.as_deref(),
                    Some(&summary),
                )?;
                if let Some(decision_id) = hit.decision_id.as_deref() {
                    let changed = ticket_decision::update_proposal_status(
                        conn,
                        decision_id,
                        ProposalStatus::Invalidated,
                    )?;
                    if !changed {
                        return Err(format!(
                            "entry_invalidated 已记账但 ticket_decision 不存在: {decision_id}"
                        )
                        .into());
                    }
                }
                Ok(())
            })
            .map_err(|error| error.to_string())?;
            Ok(RecordOutcome::Recorded)
        }
    }
}

#[derive(Debug, Default)]
pub struct ExpiryReport {
    pub scanned: usize,
    pub invalidated: Vec<(String, String)>,
    pub errors: Vec<String>,
}

impl ExpiryReport {
    pub fn log(&self) {
        log::info!(
            "[价格失效] 时效检查 scanned={} invalidated={} errors={}",
            self.scanned,
            self.invalidated.len(),
            self.errors.len()
        );
        for (code, reason) in &self.invalidated {
            log::info!("[价格失效] {code} {reason}");
        }
        for error in &self.errors {
            log::warn!("[价格失效] {error}");
        }
    }
}

/// 交易日数：`entry_date` 之后、`as_of` 之前（含）有多少个有日线的交易日。
fn trading_days_between(
    conn: &mut SqliteConnection,
    entry_date: &str,
    as_of: &str,
) -> Result<i64, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    diesel::sql_query(
        "SELECT COUNT(DISTINCT date) AS n FROM stock_daily WHERE date > ?1 AND date <= ?2",
    )
    .bind::<Text, _>(entry_date)
    .bind::<Text, _>(as_of)
    .get_result::<Row>(conn)
    .map(|row| row.n)
    .map_err(|error| format!("交易日查询失败: {error}"))
}

/// 时效失效（1.5 的第三条）：趋势型超过 3 个交易日、情绪型跨日仍未触发。
pub fn invalidate_expired(db: &DatabaseManager, as_of: &str) -> Result<ExpiryReport, String> {
    #[derive(QueryableByName)]
    struct TicketRow {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        source: String,
    }
    #[derive(QueryableByName)]
    struct EventRow {
        #[diesel(sql_type = Nullable<Text>)]
        event_at: Option<String>,
    }

    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let tickets: Vec<TicketRow> = diesel::sql_query(
        "SELECT id, code, source FROM ticket_lifecycle \
          WHERE close_date IS NULL AND stage = 'entry_proposed' ORDER BY code",
    )
    .load(&mut conn)
    .map_err(|error| format!("entry_proposed 票查询失败: {error}"))?;

    let mut report = ExpiryReport {
        scanned: tickets.len(),
        ..Default::default()
    };

    for ticket in tickets {
        // 最近一次给出入场参考的日期。
        let proposed_at: Option<String> = diesel::sql_query(
            "SELECT event_at FROM ticket_event \
              WHERE ticket_id = ? AND event_kind = 'entry_proposed' \
              ORDER BY event_at DESC, id DESC LIMIT 1",
        )
        .bind::<BigInt, _>(ticket.id)
        .get_result::<EventRow>(&mut conn)
        .optional()
        .map_err(|error| format!("entry_proposed 事件查询失败: {error}"))?
        .and_then(|row| row.event_at);
        let Some(proposed_at) = proposed_at else {
            continue;
        };
        match invalidated_after_latest_proposal(&mut conn, ticket.id) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                report.errors.push(format!("{} {error}", ticket.code));
                continue;
            }
        }
        let proposed_date: String = proposed_at.chars().take(10).collect();
        if proposed_date >= as_of.to_string() {
            continue; // 今天刚给的参考，跨日才谈失效
        }
        match ticket_decision::latest_proposal_status(&mut conn, ticket.id) {
            Ok(Some(ProposalStatus::Open)) | Ok(None) => {}
            Ok(Some(_)) => continue, // 已触发/失效/到期/被取代，不重复写
            Err(error) => {
                report
                    .errors
                    .push(format!("{} 状态投影失败: {error}", ticket.code));
                continue;
            }
        }
        let regime =
            match ticket_regime::resolve(&mut conn, ticket.id, &ticket.code, &ticket.source, as_of)
            {
                Ok(decision) => decision.regime,
                Err(error) => {
                    report
                        .errors
                        .push(format!("{} 分型失败: {error}", ticket.code));
                    continue;
                }
            };
        let elapsed = match trading_days_between(&mut conn, &proposed_date, as_of) {
            Ok(days) => days,
            Err(error) => {
                report.errors.push(format!("{} {error}", ticket.code));
                continue;
            }
        };
        let limit = match regime {
            Regime::Emotion => 1,
            Regime::Trend => 3,
        };
        if elapsed < limit {
            continue;
        }

        let reason = format!(
            "{}入场参考 {proposed_date} 起已 {elapsed} 个交易日未触发（上限 {limit}）",
            regime.label()
        );
        // 同日重跑（重启补跑）不重复记。到期是独立事件，不能再伪装成
        // entry_invalidated；旧历史事件保留不动。
        match ticket_lifecycle::event_exists_same_day(
            &mut conn,
            ticket.id,
            "entry_expired",
            as_of,
            &reason,
        ) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                report.errors.push(format!("{} {error}", ticket.code));
                continue;
            }
        }
        let decision_id = match ticket_decision::latest_open_proposal(&mut conn, ticket.id) {
            Ok(Some(context)) => Some(context.decision_id),
            Ok(None) => None,
            Err(error) => {
                report
                    .errors
                    .push(format!("{} 提案上下文查询失败: {error}", ticket.code));
                continue;
            }
        };
        let result = conn.transaction::<(), RecordTxError, _>(|conn| {
            ticket_lifecycle::append_event(
                conn,
                ticket.id,
                as_of,
                "entry_expired",
                None,
                decision_id.as_deref(),
                Some(&reason),
            )?;
            if let Some(decision_id) = decision_id.as_deref() {
                let changed =
                    ticket_decision::update_proposal_status(conn, decision_id, ProposalStatus::Expired)?;
                if !changed {
                    return Err(format!(
                        "entry_expired 已记账但 ticket_decision 不存在: {decision_id}"
                    )
                    .into());
                }
            }
            Ok(())
        });
        match result {
            Ok(()) => report.invalidated.push((ticket.code.clone(), reason)),
            Err(error) => report
                .errors
                .push(format!("{} 记到期事件失败: {error}", ticket.code)),
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(prev_high: f64, abandon: Option<f64>) -> TriggerTarget {
        TriggerTarget {
            ticket_id: 1,
            code: "002696".to_string(),
            name: "测试".to_string(),
            prev_high,
            entry_low: 10.0,
            entry_high: 10.5,
            abandon,
            regime: Regime::Emotion,
            context: None,
        }
    }

    fn quote(price: f64, change: f64) -> (f64, f64) {
        (price, change)
    }

    fn policy_target(mode: EntryMode, prev_high: f64) -> TriggerTarget {
        let mut base = target(prev_high, Some(9.0));
        let mut context =
            DecisionContext::new(1, "002696", "2026-10-27", "board_rotation", "trend", mode);
        context.price_low = Some(10.0);
        context.price_high = Some(10.5);
        context.trigger_level = Some(10.8);
        context.abandon = Some(9.0);
        context.seal();
        base.context = Some(context);
        base
    }

    #[test]
    fn pullback_policy_uses_range_not_prev_high() {
        let target = policy_target(EntryMode::Pullback, 11.0);
        let (price, change) = quote(10.2, 3.0);
        assert_eq!(judge_legacy(&target, price, change), None);
        assert_eq!(
            judge_policy(&target, price, change).0,
            Some(HitAction::Triggered)
        );
    }

    #[test]
    fn breakout_policy_uses_trigger_level_not_prev_high() {
        let target = policy_target(EntryMode::Breakout, 11.0);
        let (price, change) = quote(10.9, 3.0);
        assert_eq!(judge_legacy(&target, price, change), None);
        assert_eq!(
            judge_policy(&target, price, change).0,
            Some(HitAction::Triggered)
        );
    }

    #[test]
    fn policy_without_context_is_no_decision() {
        let target = target(10.3, Some(9.0));
        let outcome = judge_policy(&target, 11.0, 3.0);
        assert_eq!(outcome.0, None);
        assert_eq!(outcome.1, "legacy_context_missing");
    }

    #[test]
    fn shadow_synthetic_context_does_not_claim_a_decision_row() {
        let mut context = DecisionContext::new(
            1,
            "002696",
            "2026-10-27",
            "board_rotation",
            "trend",
            EntryMode::Breakout,
        );
        context.seal();
        context.decision_id.clear();
        let mut target = target(10.3, Some(9.0));
        target.context = Some(context);
        assert!(target.decision_id().is_none());
        assert_eq!(target.entry_mode(), EntryMode::Breakout);
    }

    #[test]
    fn breaks_prev_high_triggers_but_high_change_is_skipped() {
        let t = target(10.30, Some(9.00));
        let (price, change) = quote(10.35, 3.0);
        assert_eq!(judge(&t, price, change), Some(HitAction::Triggered));
        // 涨幅过大不追（Q2 回测：高开 >6% 是唯一死亡区）
        let (price, change) = quote(10.35, 7.5);
        assert_eq!(judge(&t, price, change), None);
    }

    #[test]
    fn below_abandon_invalidates_before_trigger() {
        let t = target(10.30, Some(9.00));
        let (price, change) = quote(8.90, -5.0);
        assert_eq!(judge(&t, price, change), Some(HitAction::Invalidated));
    }

    #[test]
    fn missing_abandon_never_invalidates() {
        let t = target(10.30, None);
        let (price, change) = quote(8.00, -9.0);
        assert_eq!(judge(&t, price, change), None);
    }

    #[test]
    fn hit_render_carries_action_and_regime() {
        let hit = TriggerHit {
            ticket_id: 1,
            code: "002696".to_string(),
            name: "测试".to_string(),
            price: 10.4,
            change_pct: 3.2,
            prev_high: 10.3,
            entry_low: 10.0,
            entry_high: 10.5,
            abandon: Some(9.0),
            regime: Regime::Emotion,
            action: HitAction::Triggered,
            decision_id: None,
            entry_mode: EntryMode::Unknown,
            trigger_level: None,
        };
        let text = hit.render();
        assert!(text.contains("突破确认"));
        assert!(text.contains("情绪型"));
        assert!(text.contains("002696"));
    }

    #[test]
    fn trigger_codes_are_unique_and_trimmed() {
        let mut cache = TriggerCache::default();
        cache.targets.push(target(10.0, Some(9.0)));
        let mut duplicate = target(11.0, Some(9.5));
        duplicate.ticket_id = 2;
        duplicate.code = " 002696 ".to_string();
        cache.targets.push(duplicate);
        assert_eq!(cache.codes(), vec!["002696".to_string()]);
    }

    #[test]
    fn invalidation_only_closes_the_current_proposal() {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        diesel::sql_query(
            "CREATE TABLE ticket_event (\
               id INTEGER PRIMARY KEY AUTOINCREMENT,\
               ticket_id INTEGER NOT NULL,\
               event_kind TEXT NOT NULL,\
               event_at TEXT NOT NULL)",
        )
        .execute(&mut conn)
        .expect("ticket_event schema");
        for (kind, at) in [
            ("entry_proposed", "2026-10-20 15:40:00"),
            ("entry_invalidated", "2026-10-21 09:31:00"),
        ] {
            diesel::sql_query(
                "INSERT INTO ticket_event (ticket_id, event_kind, event_at) VALUES (1, ?1, ?2)",
            )
            .bind::<Text, _>(kind)
            .bind::<Text, _>(at)
            .execute(&mut conn)
            .expect("insert event");
        }
        assert_eq!(
            invalidated_after_latest_proposal(&mut conn, 1).unwrap(),
            true
        );

        diesel::sql_query(
            "INSERT INTO ticket_event (ticket_id, event_kind, event_at) VALUES (1, 'entry_proposed', '2026-10-22 15:40:00')",
        )
        .execute(&mut conn)
        .expect("insert refreshed proposal");
        assert_eq!(
            invalidated_after_latest_proposal(&mut conn, 1).unwrap(),
            false
        );
    }

    #[test]
    fn expiry_and_supersede_also_close_the_current_window() {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        diesel::sql_query(
            "CREATE TABLE ticket_event (\
               id INTEGER PRIMARY KEY AUTOINCREMENT,\
               ticket_id INTEGER NOT NULL,\
               event_kind TEXT NOT NULL,\
               event_at TEXT NOT NULL)",
        )
        .execute(&mut conn)
        .expect("ticket_event schema");
        for (kind, at) in [
            ("entry_proposed", "2026-10-20 15:40:00"),
            ("entry_expired", "2026-10-21 15:42:00"),
        ] {
            diesel::sql_query(
                "INSERT INTO ticket_event (ticket_id, event_kind, event_at) VALUES (1, ?1, ?2)",
            )
            .bind::<Text, _>(kind)
            .bind::<Text, _>(at)
            .execute(&mut conn)
            .expect("insert event");
        }
        assert!(invalidated_after_latest_proposal(&mut conn, 1).unwrap());
        diesel::sql_query(
            "INSERT INTO ticket_event (ticket_id, event_kind, event_at) \
             VALUES (1, 'entry_proposed', '2026-10-22 15:40:00')",
        )
        .execute(&mut conn)
        .expect("new proposal reopens window");
        diesel::sql_query(
            "INSERT INTO ticket_event (ticket_id, event_kind, event_at) \
             VALUES (1, 'entry_superseded', '2026-10-23 15:40:00')",
        )
        .execute(&mut conn)
        .expect("supersede closes window");
        assert!(invalidated_after_latest_proposal(&mut conn, 1).unwrap());
    }

    #[test]
    fn entry_triggered_dedupe_ignores_changing_price_basis() {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        diesel::sql_query(
            "CREATE TABLE ticket_event (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               ticket_id INTEGER NOT NULL,
               event_kind TEXT NOT NULL,
               event_at TEXT NOT NULL,
               summary TEXT)",
        )
        .execute(&mut conn)
        .expect("ticket_event schema");
        for summary in [
            "突破昨高 38.52（现价 40.16）",
            "突破昨高 38.52（现价 39.90）",
        ] {
            diesel::sql_query(
                "INSERT INTO ticket_event (ticket_id, event_kind, event_at, summary) \
                 VALUES (1, 'entry_triggered', '2026-10-28', ?1)",
            )
            .bind::<Text, _>(summary)
            .execute(&mut conn)
            .expect("insert trigger");
        }
        assert!(entry_triggered_exists(&mut conn, 1, "2026-10-28").unwrap());
        assert!(!entry_triggered_exists(&mut conn, 1, "2026-10-29").unwrap());
        assert!(!entry_triggered_exists(&mut conn, 2, "2026-10-28").unwrap());
    }

    #[test]
    fn proposal_range_parses_stored_summary_and_rejects_garbage() {
        assert_eq!(
            parse_proposed_range("22.00-23.50｜回踩 ma10｜放弃 20.00"),
            Some((22.0, 23.5))
        );
        assert_eq!(parse_proposed_range("not-a-range"), None);
        assert_eq!(parse_proposed_range("23.50-22.00"), None);
        assert_eq!(parse_proposed_range("0-1"), None);
    }
}
