//! 入场价格的触发与失效（批 1.4 / 1.5，2026-10-23）。
//!
//! ## 两件事
//!
//! 1. **触发确认（1.4，Q5=昨日最高价）**：`entry_proposed` 的票，现价 ≥ 昨日最高价
//!    且当日涨幅 < 6% → 记 `ticket_event(kind='entry_triggered')` + `candidate_trigger_selection`，
//!    并推一条「突破确认」。1 票/日只推一次（`(trigger_date, code)` 主键天然幂等）。
//! 2. **价格失效（1.5）**：现价跌破放弃价 → `kind='entry_invalidated'`；
//!    时效到期（趋势型 > 3 个交易日、情绪型跨日未触发）同样失效。
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

use std::collections::HashMap;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use crate::data_gateway::grpc_source;
use crate::data_gateway::{GatewayBatch, RealtimeMarketQuote};
use crate::database::{ticket_lifecycle, DatabaseManager};
use crate::monitor::buy_zone;
use crate::monitor::ticket_regime::{self, Regime};

/// 触发确认的涨幅上限：涨幅超过它说明已经拉到高位，突破买点没有意义。
const MAX_TRIGGER_CHANGE_PCT: f64 = 6.0;

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
        self.targets.iter().map(|item| item.code.clone()).collect()
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
}

impl TriggerHit {
    pub fn basis(&self) -> String {
        match self.action {
            HitAction::Triggered => format!(
                "突破昨高 {:.2}（现价 {:.2}，{:+.2}%）；区间 {:.2}-{:.2}",
                self.prev_high, self.price, self.change_pct, self.entry_low, self.entry_high
            ),
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
          WHERE close_date IS NULL AND stage = 'entry_proposed' ORDER BY code",
    )
    .load(&mut conn)
    .map_err(|error| format!("entry_proposed 票查询失败: {error}"))?;

    let mut cache = TriggerCache {
        as_of: as_of.to_string(),
        ..Default::default()
    };

    for ticket in tickets {
        let Some(prev_high) = prev_high_before(&mut conn, &ticket.code, as_of)? else {
            cache.skipped.push(format!("{} 无昨高", ticket.code));
            continue;
        };
        let zone = match buy_zone::compute(&mut conn, &ticket.code, as_of) {
            Ok(Some(zone)) => zone,
            Ok(None) => {
                cache.skipped.push(format!("{} 无日线", ticket.code));
                continue;
            }
            Err(error) => {
                cache
                    .skipped
                    .push(format!("{} 买点失败: {error}", ticket.code));
                continue;
            }
        };
        let (Some(entry_low), Some(entry_high)) = (zone.entry_low, zone.entry_high) else {
            cache.skipped.push(format!("{} 无入场区间", ticket.code));
            continue;
        };
        let regime =
            match ticket_regime::resolve(&mut conn, ticket.id, &ticket.code, &ticket.source, as_of)
            {
                Ok(decision) => decision.regime,
                Err(error) => {
                    cache
                        .skipped
                        .push(format!("{} 分型失败: {error}", ticket.code));
                    continue;
                }
            };
        cache.targets.push(TriggerTarget {
            ticket_id: ticket.id,
            name: zone.name.clone().unwrap_or_else(|| ticket.code.clone()),
            code: ticket.code,
            prev_high,
            entry_low,
            entry_high,
            abandon: zone.abandon,
            regime,
        });
    }

    Ok(cache)
}

/// 判定一只票当前该触发还是失效（纯函数，便于回归）。
fn judge(target: &TriggerTarget, price: f64, change_pct: f64) -> Option<HitAction> {
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

/// 拉一次行情并判定。行情不可用时返回错误（调用方记 warn、下一轮再试）。
pub async fn evaluate(cache: &TriggerCache) -> Result<Vec<TriggerHit>, String> {
    if cache.is_empty() {
        return Ok(Vec::new());
    }
    let codes = cache.codes();
    let bridge = grpc_source::bridge_for("RealtimeQuotes").map_err(|error| error.to_string())?;
    let quotes: Vec<RealtimeMarketQuote> = match bridge.realtime_quotes_async(&codes).await {
        Ok(GatewayBatch::Available { records, .. }) => records,
        Ok(GatewayBatch::VerifiedEmpty(_)) => Vec::new(),
        Err(error) if error.reason_code() == "no_last_traded_price" => {
            bridge.realtime_quotes_with_isolation(&codes).await.quotes
        }
        Err(error) => return Err(format!("行情拉取失败: {error}")),
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
        let Some(action) = judge(target, quote.price, quote.change_percent) else {
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

/// 把一次命中写进库。同日同票同动作只记一次。
pub fn record(
    db: &DatabaseManager,
    as_of: &str,
    hit: &TriggerHit,
) -> Result<RecordOutcome, String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    match hit.action {
        HitAction::Triggered => {
            if db.latest_candidate_trigger(as_of, &hit.code)?.is_some() {
                return Ok(RecordOutcome::Duplicate);
            }
            let basis = hit.basis();
            db.record_candidate_trigger(as_of, &hit.code, &hit.name, &basis)?;
            ticket_lifecycle::append_event(
                &mut conn,
                hit.ticket_id,
                as_of,
                "entry_triggered",
                None,
                None,
                Some(&basis),
            )?;
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
            ticket_lifecycle::append_event(
                &mut conn,
                hit.ticket_id,
                as_of,
                "entry_invalidated",
                None,
                None,
                Some(&summary),
            )?;
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
        let proposed_date: String = proposed_at.chars().take(10).collect();
        if proposed_date >= as_of.to_string() {
            continue; // 今天刚给的参考，跨日才谈失效
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
        // 同日重跑（重启补跑）不重复记
        match ticket_lifecycle::event_exists_same_day(
            &mut conn,
            ticket.id,
            "entry_invalidated",
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
        match ticket_lifecycle::append_event(
            &mut conn,
            ticket.id,
            as_of,
            "entry_invalidated",
            None,
            None,
            Some(&reason),
        ) {
            Ok(()) => report.invalidated.push((ticket.code.clone(), reason)),
            Err(error) => report
                .errors
                .push(format!("{} 记事件失败: {error}", ticket.code)),
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
        }
    }

    fn quote(price: f64, change: f64) -> (f64, f64) {
        (price, change)
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
        };
        let text = hit.render();
        assert!(text.contains("突破确认"));
        assert!(text.contains("情绪型"));
        assert!(text.contains("002696"));
    }
}
