//! 单票决策上下文与入口策略（11.10 / WP1-WP3）。
//!
//! 这一层只做三件事：
//! 1. 把一次入场决策需要的输入收敛成 `DecisionContext`（纯数据、可序列化）；
//! 2. 用纯函数 `EntryPolicy` 按 `EntryMode` 分派触发语义；
//! 3. 把提案上下文写入 `ticket_decision`，供 shadow、投影和证据包复用。
//!
//! 边界：
//! - 不查行情、不发推送、不调用 LLM；
//! - 不把缺失字段补成 0；没有证据时返回 `NoDecision`；
//! - 第一版实时行情没有量比字段，`confirmation` 只能标 `price_only`，并在
//!   `missing_fields` 里记录 `volume_ratio`。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::database::DatabaseManager;
use crate::monitor::buy_zone::Trend;
use crate::monitor::selection_context::{ExtensionAssessment, SelectionContext};
use crate::monitor::ticket_regime::Regime;

/// 规则版本必须和证据包、回放脚本一起固定。
pub const RULE_VERSION: &str = "ticket_decision_v1";
/// 突破型触发允许的最大当日涨幅；超过它不追。
pub const MAX_TRIGGER_CHANGE_PCT: f64 = 6.0;
/// Breakout 的追高上限（相对 trigger_level）。首版只用于 EntryPolicy/shadow，
/// 不改变 legacy；后续按 shadow 的区间外比例再调。
pub const MAX_BREAKOUT_CHASE_PCT: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryMode {
    /// 回踩型：价格进入 `price_low..price_high`。
    Pullback,
    /// 突破型：价格站上 `trigger_level` 且涨幅未超上限。
    Breakout,
    /// 竞价参考型：只给参考价，不伪造可执行触发。
    AuctionReference,
    /// 无法判定：不给可执行价，不猜触发方式。
    Unknown,
}

impl EntryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pullback => "pullback",
            Self::Breakout => "breakout",
            Self::AuctionReference => "auction_reference",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Open,
    Triggered,
    Invalidated,
    Expired,
    Superseded,
}

impl ProposalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Triggered => "triggered",
            Self::Invalidated => "invalidated",
            Self::Expired => "expired",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Triggered,
    Invalidated,
    Expired,
    NoDecision,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Triggered => "triggered",
            Self::Invalidated => "invalidated",
            Self::Expired => "expired",
            Self::NoDecision => "no_decision",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    ReferencePrice,
    ExecutablePrice,
    NoPrice,
}

impl OutputKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReferencePrice => "reference_price",
            Self::ExecutablePrice => "executable_price",
            Self::NoPrice => "no_price",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceHealth {
    Available,
    Partial,
    Unavailable,
    Stale,
    Conflict,
}

impl SourceHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Partial => "partial",
            Self::Unavailable => "unavailable",
            Self::Stale => "stale",
            Self::Conflict => "conflict",
        }
    }

    pub fn blocks_execution(self) -> bool {
        matches!(self, Self::Unavailable | Self::Stale | Self::Conflict)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceGrade {
    L0,
    L1,
    L2,
    L3,
    L4,
}

impl EvidenceGrade {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "l0",
            Self::L1 => "l1",
            Self::L2 => "l2",
            Self::L3 => "l3",
            Self::L4 => "l4",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStatus {
    pub source: String,
    pub health: SourceHealth,
    pub as_of: Option<String>,
    pub detail: Option<String>,
}

impl SourceStatus {
    pub fn new(source: impl Into<String>, health: SourceHealth) -> Self {
        Self {
            source: source.into(),
            health,
            as_of: None,
            detail: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub kind: String,
    pub reference: String,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionContext {
    pub held_qty: i64,
    pub available_qty: i64,
    pub cost: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityContext {
    pub suggested_pct: Option<String>,
    pub rank: Option<i32>,
    pub max_pct: Option<String>,
}

/// 一次入场决策的完整、可序列化上下文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionContext {
    pub decision_id: String,
    pub ticket_id: i64,
    pub code: String,
    pub as_of: String,
    pub source: String,
    pub regime: String,
    pub entry_mode: EntryMode,
    pub price_low: Option<f64>,
    pub price_high: Option<f64>,
    pub trigger_level: Option<f64>,
    pub abandon: Option<f64>,
    pub confirmation: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub source_health: Vec<SourceStatus>,
    pub data_mode: Option<String>,
    pub evidence_grade: EvidenceGrade,
    pub missing_fields: Vec<String>,
    pub position: PositionContext,
    pub capacity: CapacityContext,
    /// 选股时的历史价格位置；只记录，不参与 EntryPolicy 触发。
    #[serde(default)]
    pub selection: SelectionContext,
    /// 由 SelectionContext 派生出的观察状态；不代表交易闸门。
    #[serde(default)]
    pub extension: ExtensionAssessment,
    pub rule_version: String,
    pub evidence_refs: Vec<EvidenceRef>,
    pub supersedes: Option<String>,
}

impl DecisionContext {
    pub fn new(
        ticket_id: i64,
        code: impl Into<String>,
        as_of: impl Into<String>,
        source: impl Into<String>,
        regime: impl Into<String>,
        entry_mode: EntryMode,
    ) -> Self {
        Self {
            decision_id: String::new(),
            ticket_id,
            code: code.into(),
            as_of: as_of.into(),
            source: source.into(),
            regime: regime.into(),
            entry_mode,
            price_low: None,
            price_high: None,
            trigger_level: None,
            abandon: None,
            confirmation: "price_only".to_owned(),
            valid_from: None,
            valid_until: None,
            source_health: Vec::new(),
            data_mode: None,
            evidence_grade: EvidenceGrade::L1,
            missing_fields: Vec::new(),
            position: PositionContext::default(),
            capacity: CapacityContext::default(),
            selection: SelectionContext::default(),
            extension: ExtensionAssessment::default(),
            rule_version: RULE_VERSION.to_owned(),
            evidence_refs: Vec::new(),
            supersedes: None,
        }
    }

    /// 由上下文内容生成稳定 ID；`decision_id` 本身不参与哈希。
    pub fn compute_decision_id(&self) -> String {
        let mut copy = self.clone();
        copy.decision_id.clear();
        match serde_json::to_vec(&copy) {
            Ok(bytes) => {
                let digest = Sha256::digest(&bytes);
                format!("td-{}", hex::encode(digest))
            }
            Err(_) => format!(
                "td-unsealed-{}-{}-{}",
                self.ticket_id, self.as_of, self.rule_version
            ),
        }
    }

    pub fn seal(&mut self) {
        self.decision_id = self.compute_decision_id();
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| format!("DecisionContext 序列化失败: {error}"))
    }

    pub fn quote_health_allows_decision(&self) -> bool {
        if self.missing_fields.iter().any(|field| field == "quotes") {
            return false;
        }
        !self
            .source_health
            .iter()
            .any(|status| status.source == "quotes" && status.health.blocks_execution())
    }
}

/// 行情视图。只包含判定需要的两个字段，避免把整条行情记录带进内核。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuoteView {
    pub price: f64,
    pub change_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyOutcome {
    pub decision: Decision,
    pub reason: &'static str,
}

impl PolicyOutcome {
    fn new(decision: Decision, reason: &'static str) -> Self {
        Self { decision, reason }
    }
}

/// 纯函数入口策略。
pub struct EntryPolicy;

impl EntryPolicy {
    pub fn evaluate(context: &DecisionContext, quote: QuoteView) -> PolicyOutcome {
        if quote.price <= 0.0 {
            return PolicyOutcome::new(Decision::NoDecision, "invalid_quote");
        }
        if !context.quote_health_allows_decision() {
            return PolicyOutcome::new(Decision::NoDecision, "quote_source_blocked");
        }

        // 参考型与未知型没有可执行触发；不要因为 abandon 把它们写成失效。
        if matches!(
            context.entry_mode,
            EntryMode::AuctionReference | EntryMode::Unknown
        ) {
            return PolicyOutcome::new(Decision::NoDecision, "reference_only_or_unknown");
        }

        if let Some(abandon) = context.abandon {
            if quote.price < abandon {
                return PolicyOutcome::new(Decision::Invalidated, "below_abandon");
            }
        }

        match context.entry_mode {
            EntryMode::Pullback => match (context.price_low, context.price_high) {
                (Some(low), Some(high)) if high >= low => {
                    if quote.price >= low && quote.price <= high {
                        PolicyOutcome::new(Decision::Triggered, "inside_pullback_range")
                    } else {
                        PolicyOutcome::new(Decision::NoDecision, "outside_pullback_range")
                    }
                }
                _ => PolicyOutcome::new(Decision::NoDecision, "missing_price_window"),
            },
            EntryMode::Breakout => match context.trigger_level {
                Some(level) if quote.price >= level => {
                    if quote.price > level * (1.0 + MAX_BREAKOUT_CHASE_PCT / 100.0) {
                        PolicyOutcome::new(Decision::NoDecision, "above_breakout_chase_cap")
                    } else if quote.change_pct < MAX_TRIGGER_CHANGE_PCT {
                        PolicyOutcome::new(Decision::Triggered, "above_trigger_level")
                    } else {
                        PolicyOutcome::new(Decision::NoDecision, "above_change_cap")
                    }
                }
                Some(_) => PolicyOutcome::new(Decision::NoDecision, "below_trigger_level"),
                None => PolicyOutcome::new(Decision::NoDecision, "missing_trigger_level"),
            },
            EntryMode::AuctionReference => {
                PolicyOutcome::new(Decision::NoDecision, "auction_reference_only")
            }
            EntryMode::Unknown => PolicyOutcome::new(Decision::NoDecision, "unknown_entry_mode"),
        }
    }
}

/// 第一版只做无副作用的输出映射；可执行价需要成交证据，当前不产出。
pub fn output_kind_for(decision: Decision) -> OutputKind {
    match decision {
        Decision::Triggered => OutputKind::ReferencePrice,
        Decision::Invalidated | Decision::Expired | Decision::NoDecision => OutputKind::NoPrice,
    }
}

/// 分型映射：P01 情绪型走竞价参考，其余按均线形态分派。
pub fn entry_mode_for(regime: Regime, trend: Trend, source: &str) -> EntryMode {
    // 只有 P01 情绪票在 09:25 有竞价参考；其他情绪来源没有竞价证据。
    if regime == Regime::Emotion && source == "p01_preopen" {
        return EntryMode::AuctionReference;
    }
    match trend {
        Trend::Bullish => EntryMode::Pullback,
        Trend::Bearish | Trend::Tangled => EntryMode::Breakout,
        Trend::Unknown => EntryMode::Unknown,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub decision_id: String,
    pub ticket_id: i64,
    pub code: String,
    pub as_of: String,
    pub decision: Decision,
    pub output_kind: OutputKind,
    pub reason: String,
    pub rule_version: String,
}

/// 写入一条提案上下文。同一 context 幂等；不覆盖已有状态。
pub fn persist_proposal(db: &DatabaseManager, context: &DecisionContext) -> Result<(), String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    persist_proposal_on(&mut conn, context)
}

fn persist_proposal_on(
    conn: &mut SqliteConnection,
    context: &DecisionContext,
) -> Result<(), String> {
    let source_health = serde_json::to_string(&context.source_health)
        .map_err(|error| format!("source_health 序列化失败: {error}"))?;
    let context_json = context.to_json()?;
    diesel::sql_query(
        "INSERT OR IGNORE INTO ticket_decision (\
         decision_id, ticket_id, code, as_of, source, regime, entry_mode,\
         price_low, price_high, trigger_level, abandon, proposal_status,\
         valid_from, valid_until, source_health, evidence_grade,\
         context_json, rule_version, supersedes) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
    )
    .bind::<Text, _>(&context.decision_id)
    .bind::<BigInt, _>(context.ticket_id)
    .bind::<Text, _>(&context.code)
    .bind::<Text, _>(&context.as_of)
    .bind::<Text, _>(&context.source)
    .bind::<Text, _>(&context.regime)
    .bind::<Text, _>(context.entry_mode.as_str())
    .bind::<Nullable<Double>, _>(context.price_low)
    .bind::<Nullable<Double>, _>(context.price_high)
    .bind::<Nullable<Double>, _>(context.trigger_level)
    .bind::<Nullable<Double>, _>(context.abandon)
    .bind::<Text, _>(ProposalStatus::Open.as_str())
    .bind::<Nullable<Text>, _>(&context.valid_from)
    .bind::<Nullable<Text>, _>(&context.valid_until)
    .bind::<Text, _>(&source_health)
    .bind::<Text, _>(context.evidence_grade.as_str())
    .bind::<Text, _>(&context_json)
    .bind::<Text, _>(&context.rule_version)
    .bind::<Nullable<Text>, _>(&context.supersedes)
    .execute(conn)
    .map_err(|error| format!("ticket_decision 写入失败: {error}"))?;
    Ok(())
}

/// 读取某票最新的 Open 提案上下文。
pub fn latest_open_proposal(
    conn: &mut SqliteConnection,
    ticket_id: i64,
) -> Result<Option<DecisionContext>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        context_json: String,
    }

    let row: Option<Row> = diesel::sql_query(
        "SELECT context_json FROM ticket_decision \
         WHERE ticket_id = ?1 AND proposal_status = 'open' \
         ORDER BY as_of DESC, created_at DESC, decision_id DESC LIMIT 1",
    )
    .bind::<BigInt, _>(ticket_id)
    .get_result::<Row>(conn)
    .optional()
    .map_err(|error| format!("ticket_decision 查询失败: {error}"))?;

    match row {
        Some(row) => serde_json::from_str(&row.context_json)
            .map(Some)
            .map_err(|error| format!("ticket_decision context 解析失败: {error}")),
        None => Ok(None),
    }
}

/// 更新提案状态；只更新已存在的 decision，不创建新行。
pub fn update_proposal_status(
    conn: &mut SqliteConnection,
    decision_id: &str,
    status: ProposalStatus,
) -> Result<bool, String> {
    let changed = diesel::sql_query(
        "UPDATE ticket_decision SET proposal_status = ?, updated_at = CURRENT_TIMESTAMP \
         WHERE decision_id = ?",
    )
    .bind::<Text, _>(status.as_str())
    .bind::<Text, _>(decision_id)
    .execute(conn)
    .map_err(|error| format!("ticket_decision 状态更新失败: {error}"))?;
    Ok(changed > 0)
}

/// 由 `ticket_event` 事件推出的提案状态。事件是事实，`ticket_decision` 只是投影。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalProjection {
    pub ticket_id: i64,
    pub code: String,
    pub decision_id: Option<String>,
    pub status: Option<ProposalStatus>,
    pub latest_event_kind: Option<String>,
    pub latest_event_at: Option<String>,
}

/// 读取一张票最新一次提案生命周期事件，映射成状态；不回写历史。
pub fn latest_proposal_status(
    conn: &mut SqliteConnection,
    ticket_id: i64,
) -> Result<Option<ProposalStatus>, String> {
    #[derive(QueryableByName)]
    struct EventRow {
        #[diesel(sql_type = Text)]
        event_kind: String,
    }
    let event: Option<EventRow> = diesel::sql_query(
        "SELECT event_kind FROM ticket_event \
         WHERE ticket_id = ?1 \
           AND event_kind IN ('entry_proposed','entry_triggered','entry_invalidated',\
                              'entry_expired','entry_superseded') \
         ORDER BY event_at DESC, id DESC LIMIT 1",
    )
    .bind::<BigInt, _>(ticket_id)
    .get_result::<EventRow>(conn)
    .optional()
    .map_err(|error| format!("提案事件查询失败: {error}"))?;
    Ok(event.and_then(|row| match row.event_kind.as_str() {
        "entry_proposed" => Some(ProposalStatus::Open),
        "entry_triggered" => Some(ProposalStatus::Triggered),
        "entry_invalidated" => Some(ProposalStatus::Invalidated),
        "entry_expired" => Some(ProposalStatus::Expired),
        "entry_superseded" => Some(ProposalStatus::Superseded),
        _ => None,
    }))
}

/// 列出所有活跃 `entry_proposed` 票及其事件投影，供审计/验收使用。
pub fn list_active_proposals(
    conn: &mut SqliteConnection,
) -> Result<Vec<ProposalProjection>, String> {
    #[derive(QueryableByName)]
    struct TicketRow {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        code: String,
    }
    let tickets: Vec<TicketRow> = diesel::sql_query(
        "SELECT id, code FROM ticket_lifecycle \
         WHERE close_date IS NULL AND stage = 'entry_proposed' \
         ORDER BY code, id",
    )
    .load(conn)
    .map_err(|error| format!("活跃提案查询失败: {error}"))?;

    let mut out = Vec::with_capacity(tickets.len());
    for ticket in tickets {
        #[derive(QueryableByName)]
        struct EventRow {
            #[diesel(sql_type = Text)]
            event_kind: String,
            #[diesel(sql_type = Nullable<Text>)]
            event_at: Option<String>,
        }
        let event: Option<EventRow> = diesel::sql_query(
            "SELECT event_kind, event_at FROM ticket_event \
             WHERE ticket_id = ?1 \
               AND event_kind IN ('entry_proposed','entry_triggered','entry_invalidated',\
                                  'entry_expired','entry_superseded') \
             ORDER BY event_at DESC, id DESC LIMIT 1",
        )
        .bind::<BigInt, _>(ticket.id)
        .get_result::<EventRow>(conn)
        .optional()
        .map_err(|error| format!("提案事件查询失败: {error}"))?;
        let latest_event_kind = event.as_ref().map(|row| row.event_kind.clone());
        let status = latest_event_kind.as_deref().and_then(|kind| match kind {
            "entry_proposed" => Some(ProposalStatus::Open),
            "entry_triggered" => Some(ProposalStatus::Triggered),
            "entry_invalidated" => Some(ProposalStatus::Invalidated),
            "entry_expired" => Some(ProposalStatus::Expired),
            "entry_superseded" => Some(ProposalStatus::Superseded),
            _ => None,
        });
        let decision_id: Option<String> = {
            #[derive(QueryableByName)]
            struct DecisionRow {
                #[diesel(sql_type = Text)]
                decision_id: String,
            }
            diesel::sql_query(
                "SELECT decision_id FROM ticket_decision \
                 WHERE ticket_id = ?1 ORDER BY as_of DESC, updated_at DESC, decision_id DESC LIMIT 1",
            )
            .bind::<BigInt, _>(ticket.id)
            .get_result::<DecisionRow>(conn)
            .optional()
            .map_err(|error| format!("提案上下文查询失败: {error}"))?
            .map(|row| row.decision_id)
        };
        out.push(ProposalProjection {
            ticket_id: ticket.id,
            code: ticket.code,
            decision_id,
            status,
            latest_event_kind,
            latest_event_at: event.and_then(|row| row.event_at),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ticket_lifecycle;

    fn context(mode: EntryMode) -> DecisionContext {
        let mut context =
            DecisionContext::new(1, "001216", "2026-10-26", "board_rotation", "trend", mode);
        context.price_low = Some(10.0);
        context.price_high = Some(10.5);
        context.trigger_level = Some(10.8);
        context.abandon = Some(9.0);
        context.seal();
        context
    }

    #[test]
    fn pullback_triggers_inside_range_not_above_prev_high() {
        let context = context(EntryMode::Pullback);
        let inside = EntryPolicy::evaluate(
            &context,
            QuoteView {
                price: 10.2,
                change_pct: 3.0,
            },
        );
        assert_eq!(inside.decision, Decision::Triggered);
        let above = EntryPolicy::evaluate(
            &context,
            QuoteView {
                price: 10.9,
                change_pct: 3.0,
            },
        );
        assert_eq!(above.decision, Decision::NoDecision);
    }

    #[test]
    fn breakout_uses_trigger_level_and_change_cap() {
        let context = context(EntryMode::Breakout);
        let triggered = EntryPolicy::evaluate(
            &context,
            QuoteView {
                price: 10.9,
                change_pct: 3.0,
            },
        );
        assert_eq!(triggered.decision, Decision::Triggered);
        let too_hot = EntryPolicy::evaluate(
            &context,
            QuoteView {
                price: 10.9,
                change_pct: 6.0,
            },
        );
        assert_eq!(too_hot.decision, Decision::NoDecision);
    }

    #[test]
    fn breakout_rejects_chase_above_the_cap() {
        let context = context(EntryMode::Breakout);
        let inside = EntryPolicy::evaluate(
            &context,
            QuoteView {
                price: 11.30,
                change_pct: 3.0,
            },
        );
        assert_eq!(inside.decision, Decision::Triggered);
        let too_far = EntryPolicy::evaluate(
            &context,
            QuoteView {
                price: 11.50,
                change_pct: 3.0,
            },
        );
        assert_eq!(too_far.decision, Decision::NoDecision);
        assert_eq!(too_far.reason, "above_breakout_chase_cap");
    }

    #[test]
    fn abandon_invalidates_before_mode_specific_trigger() {
        let context = context(EntryMode::Pullback);
        let outcome = EntryPolicy::evaluate(
            &context,
            QuoteView {
                price: 8.9,
                change_pct: -4.0,
            },
        );
        assert_eq!(outcome.decision, Decision::Invalidated);
    }

    #[test]
    fn auction_reference_and_unknown_never_trigger() {
        for mode in [EntryMode::AuctionReference, EntryMode::Unknown] {
            let context = context(mode);
            let outcome = EntryPolicy::evaluate(
                &context,
                QuoteView {
                    price: 10.2,
                    change_pct: 2.0,
                },
            );
            assert_eq!(outcome.decision, Decision::NoDecision);
        }
    }

    #[test]
    fn missing_window_returns_no_decision_not_zero() {
        let mut context = context(EntryMode::Pullback);
        context.price_low = None;
        context.price_high = None;
        context.seal();
        let outcome = EntryPolicy::evaluate(
            &context,
            QuoteView {
                price: 10.2,
                change_pct: 2.0,
            },
        );
        assert_eq!(outcome.decision, Decision::NoDecision);
        assert_eq!(outcome.reason, "missing_price_window");
    }

    #[test]
    fn blocked_quote_health_blocks_execution() {
        let mut context = context(EntryMode::Pullback);
        context
            .source_health
            .push(SourceStatus::new("quotes", SourceHealth::Stale));
        context.seal();
        let outcome = EntryPolicy::evaluate(
            &context,
            QuoteView {
                price: 10.2,
                change_pct: 2.0,
            },
        );
        assert_eq!(outcome.decision, Decision::NoDecision);
        assert_eq!(outcome.reason, "quote_source_blocked");
    }

    #[test]
    fn decision_id_is_content_addressed_and_changes_with_input() {
        let mut context = context(EntryMode::Pullback);
        let first = context.decision_id.clone();
        assert!(first.starts_with("td-"));
        context.seal();
        assert_eq!(context.decision_id, first);
        context.price_low = Some(10.1);
        context.seal();
        assert_ne!(context.decision_id, first);
    }

    #[test]
    fn output_kind_does_not_claim_executable_price() {
        assert_eq!(
            output_kind_for(Decision::Triggered),
            OutputKind::ReferencePrice
        );
        assert_eq!(output_kind_for(Decision::NoDecision), OutputKind::NoPrice);
    }

    #[test]
    fn context_serde_roundtrip_preserves_policy_input() {
        let context = context(EntryMode::Breakout);
        let json = context.to_json().expect("serialize context");
        let decoded: DecisionContext = serde_json::from_str(&json).expect("deserialize context");
        assert_eq!(decoded, context);
    }

    #[test]
    fn legacy_context_without_selection_deserializes_as_unknown() {
        let context = context(EntryMode::Breakout);
        let mut value = serde_json::to_value(&context).expect("serialize context");
        let object = value.as_object_mut().expect("context object");
        object.remove("selection");
        object.remove("extension");

        let decoded: DecisionContext = serde_json::from_value(value).expect("legacy context");
        assert_eq!(decoded.selection, SelectionContext::default());
        assert_eq!(
            decoded.extension.state,
            crate::monitor::selection_context::ExtensionState::Unknown
        );
    }

    #[test]
    fn selection_context_is_part_of_decision_identity() {
        let mut base = context(EntryMode::Pullback);
        let original_id = base.decision_id.clone();

        base.selection.pre20_return = Some(0.35);
        base.selection.dist_hi60 = Some(-0.02);
        base.selection.position_60d = Some(0.80);
        base.selection.limit_up_streak = Some(2);
        base.extension.state = crate::monitor::selection_context::ExtensionState::Extended;
        base.extension.reasons = vec!["pre20>20%".to_owned()];
        base.seal();

        assert_ne!(base.decision_id, original_id);
        let json = base.to_json().expect("serialize selected context");
        let decoded: DecisionContext = serde_json::from_str(&json).expect("deserialize selected");
        assert_eq!(decoded.selection.pre20_return, Some(0.35));
        assert_eq!(
            decoded.extension.state,
            crate::monitor::selection_context::ExtensionState::Extended
        );
    }

    #[test]
    fn entry_mode_mapping_is_explicit() {
        assert_eq!(
            entry_mode_for(Regime::Emotion, Trend::Bullish, "p01_preopen"),
            EntryMode::AuctionReference
        );
        assert_eq!(
            entry_mode_for(Regime::Emotion, Trend::Bullish, "holding"),
            EntryMode::Pullback
        );
        assert_eq!(
            entry_mode_for(Regime::Trend, Trend::Bullish, "catalyst"),
            EntryMode::Pullback
        );
        assert_eq!(
            entry_mode_for(Regime::Trend, Trend::Tangled, "catalyst"),
            EntryMode::Breakout
        );
        assert_eq!(
            entry_mode_for(Regime::Trend, Trend::Unknown, "catalyst"),
            EntryMode::Unknown
        );
    }

    #[test]
    fn proposal_roundtrips_and_status_closes_open_window() {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        ticket_lifecycle::create_schema(&mut conn).expect("ticket schema");
        let context = context(EntryMode::Pullback);
        persist_proposal_on(&mut conn, &context).expect("persist proposal");
        let loaded = latest_open_proposal(&mut conn, 1)
            .expect("load proposal")
            .expect("open proposal");
        assert_eq!(loaded.decision_id, context.decision_id);
        assert_eq!(loaded.entry_mode, EntryMode::Pullback);
        assert_eq!(loaded.price_low, Some(10.0));

        assert!(
            update_proposal_status(&mut conn, &context.decision_id, ProposalStatus::Triggered)
                .expect("close proposal")
        );
        assert!(latest_open_proposal(&mut conn, 1)
            .expect("query after status")
            .is_none());
    }

    #[test]
    fn event_projection_distinguishes_expired_and_superseded() {
        let mut conn = SqliteConnection::establish(":memory:").expect("in-memory sqlite");
        ticket_lifecycle::create_schema(&mut conn).expect("ticket schema");
        diesel::sql_query(
            "INSERT INTO ticket_lifecycle (id, code, open_date, stage, source) \
             VALUES (1, '001216', '2026-10-26', 'entry_proposed', 'p01_preopen')",
        )
        .execute(&mut conn)
        .expect("ticket");
        ticket_lifecycle::append_event(
            &mut conn,
            1,
            "2026-10-26 15:40:00",
            "entry_proposed",
            None,
            Some("td-old"),
            Some("初始提案"),
        )
        .expect("proposal event");
        assert_eq!(
            latest_proposal_status(&mut conn, 1).expect("projection"),
            Some(ProposalStatus::Open)
        );

        ticket_lifecycle::append_event(
            &mut conn,
            1,
            "2026-10-27 15:42:00",
            "entry_expired",
            None,
            Some("td-old"),
            Some("到期"),
        )
        .expect("expired event");
        assert_eq!(
            latest_proposal_status(&mut conn, 1).expect("projection"),
            Some(ProposalStatus::Expired)
        );

        ticket_lifecycle::append_event(
            &mut conn,
            1,
            "2026-10-27 15:43:00",
            "entry_superseded",
            None,
            Some("td-old"),
            Some("被新提案取代"),
        )
        .expect("superseded event");
        let active = list_active_proposals(&mut conn).expect("active projections");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].ticket_id, 1);
        assert_eq!(active[0].status, Some(ProposalStatus::Superseded));
        assert_eq!(
            active[0].latest_event_kind.as_deref(),
            Some("entry_superseded")
        );
    }
}
