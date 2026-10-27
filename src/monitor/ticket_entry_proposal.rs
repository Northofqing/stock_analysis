//! 观察池 → 入场参考区间（方案 P2）。
//!
//! 复用 [`crate::monitor::buy_zone::compute`] 的均线判型、支撑压力与拥挤度门，
//! 把「确实有买点」的观察票推进到 `stage='entry_proposed'`，并记一条
//! `ticket_event(kind='entry_proposed')`。这是 ticket 生命周期第二段：
//! 前面 P1 只负责把票选进池子（`observing`），这一段才给出「在哪买、放弃在哪」。
//!
//! 为什么不另写一套价位算法：`buy_zone` 已经处理了三件容易踩的事——
//! ① 不同趋势用不同上下沿（多头踩 ma10、空头不追高）；
//! ② 已在 20 日低附近时不给买点（否则买入价和放弃价交叉，等于买了就止损）；
//! ③ 连板/近 5 日涨幅/偏离 ma5 三道超买门。
//! 重写一遍只会引入分歧，所以这里只做「取结果 + 记账」。
//!
//! 约束：
//! - 只处理 `stage='observing'`。已经是 `entry_proposed` 的不重复推，`holding` 绝不动。
//! - 同一天同一票不重复写事件（幂等看 `ticket_event` 当天有没有 `entry_proposed`）。
//! - 数据不足 / 超买 / 已在支撑位：**不推进**，只记 skipped 原因，不编造区间（BR-222）。
//! - 单票失败不影响其余票：错误进 `errors`，继续下一只。

use chrono::NaiveDate;
use diesel::sql_types::{BigInt, Nullable, Text};
use diesel::OptionalExtension;
use diesel::{QueryableByName, RunQueryDsl, SqliteConnection};

use crate::database::ticket_lifecycle;
use crate::database::DatabaseManager;
use crate::monitor::buy_zone;
use crate::monitor::ticket_decision::{
    self, DecisionContext, EntryMode, EvidenceGrade, SourceHealth, SourceStatus,
};
use crate::monitor::ticket_regime::{self, Regime};

#[derive(QueryableByName)]
struct CodeRow {
    #[diesel(sql_type = Text)]
    code: String,
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

#[derive(Debug, Default)]
pub struct ProposalReport {
    pub scanned: usize,
    pub proposed: Vec<String>,
    pub skipped_overheat: Vec<String>,
    pub skipped_at_support: Vec<String>,
    pub skipped_no_data: Vec<String>,
    /// 连板 ≥4 的情绪段票：不给 ma 回踩价（2026-10-24）
    pub skipped_emotion_phase: Vec<String>,
    pub skipped_duplicate: usize,
    pub errors: Vec<String>,
}

impl ProposalReport {
    pub fn summary_line(&self) -> String {
        format!(
            "[入场参考] scanned={} proposed={} overheated={} at_support={} no_data={} emotion_phase={} duplicate={} errors={}",
            self.scanned,
            self.proposed.len(),
            self.skipped_overheat.len(),
            self.skipped_at_support.len(),
            self.skipped_no_data.len(),
            self.skipped_emotion_phase.len(),
            self.skipped_duplicate,
            self.errors.len()
        )
    }
}

/// 仅 shadow 写 `ticket_decision`，默认不改变 proposal 的旧行为。
fn decision_shadow_enabled() -> bool {
    std::env::var("TICKET_DECISION_SHADOW")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn latest_source_date(
    conn: &mut SqliteConnection,
    sql: &str,
    label: &str,
) -> Result<Option<String>, String> {
    #[derive(QueryableByName)]
    struct DateRow {
        #[diesel(sql_type = Nullable<Text>)]
        d: Option<String>,
    }
    diesel::sql_query(sql)
        .get_result::<DateRow>(conn)
        .optional()
        .map(|row| row.and_then(|row| row.d))
        .map_err(|error| format!("{label} 日期查询失败: {error}"))
}

fn health_for_date(label: &str, date: Option<String>, as_of: &str) -> SourceStatus {
    match date {
        Some(date) if date == as_of => SourceStatus {
            source: label.to_owned(),
            health: SourceHealth::Available,
            as_of: Some(date),
            detail: None,
        },
        Some(date) => SourceStatus {
            source: label.to_owned(),
            health: SourceHealth::Stale,
            as_of: Some(date),
            detail: Some(format!("最新日期早于 {as_of}")),
        },
        None => SourceStatus {
            source: label.to_owned(),
            health: SourceHealth::Unavailable,
            as_of: None,
            detail: Some("未找到可用数据".to_owned()),
        },
    }
}

fn source_health_snapshot(conn: &mut SqliteConnection, as_of: &str) -> Vec<SourceStatus> {
    let mut sources = Vec::with_capacity(4);
    let daily = latest_source_date(conn, "SELECT MAX(date) AS d FROM stock_daily", "daily_bars");
    sources.push(match daily {
        Ok(date) => health_for_date("daily_bars", date, as_of),
        Err(error) => SourceStatus {
            source: "daily_bars".to_owned(),
            health: SourceHealth::Unavailable,
            as_of: None,
            detail: Some(error),
        },
    });
    let news = latest_source_date(
        conn,
        "SELECT MAX(substr(created_at, 1, 10)) AS d FROM news_ai_assessment",
        "news",
    );
    sources.push(match news {
        Ok(date) => health_for_date("news", date, as_of),
        Err(error) => SourceStatus {
            source: "news".to_owned(),
            health: SourceHealth::Unavailable,
            as_of: None,
            detail: Some(error),
        },
    });
    let board = latest_source_date(
        conn,
        "SELECT MAX(trade_date) AS d FROM board_ranking_daily",
        "board",
    );
    sources.push(match board {
        Ok(date) => health_for_date("board", date, as_of),
        Err(error) => SourceStatus {
            source: "board".to_owned(),
            health: SourceHealth::Unavailable,
            as_of: None,
            detail: Some(error),
        },
    });
    // proposal 阶段还没有盘中报价；这是事实，不伪装成已确认。
    sources.push(SourceStatus {
        source: "quotes".to_owned(),
        health: SourceHealth::Partial,
        as_of: Some(as_of.to_owned()),
        detail: Some("proposal 阶段尚未评估实时报价".to_owned()),
    });
    sources
}

fn valid_until_for(as_of: &str, regime: Regime) -> Option<String> {
    let mut date = NaiveDate::parse_from_str(as_of, "%Y-%m-%d").ok()?;
    let limit = match regime {
        Regime::Emotion => 1,
        Regime::Trend => 3,
    };
    for _ in 0..limit {
        date = crate::calendar::next_trading_day(date);
    }
    Some(date.format("%Y-%m-%d").to_string())
}

/// 当天是否已经给这只票推过入场参考。
fn already_proposed_today(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    as_of: &str,
) -> Result<bool, String> {
    let row: CountRow = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM ticket_event \
         WHERE ticket_id = ? AND event_kind = 'entry_proposed' AND date(event_at) = date(?)",
    )
    .bind::<BigInt, _>(ticket_id)
    .bind::<Text, _>(as_of)
    .get_result(conn)
    .map_err(|e| format!("entry_proposed 幂等查询失败: {e}"))?;
    Ok(row.n > 0)
}

/// 给所有 `observing` 的票算一次入场参考。
///
/// `as_of` 必须是交易日（`YYYY-MM-DD`），盘后调度传当天日期。
pub fn propose_for_observing(db: &DatabaseManager, as_of: &str) -> Result<ProposalReport, String> {
    // 先校验日期格式，避免把非法值带进 SQL 的 date() 比较
    NaiveDate::parse_from_str(as_of, "%Y-%m-%d")
        .map_err(|error| format!("as_of 非法 ({as_of}): {error}"))?;
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;

    let observing: Vec<String> = diesel::sql_query(
        "SELECT code FROM ticket_lifecycle WHERE close_date IS NULL AND stage = 'observing'",
    )
    .load::<CodeRow>(&mut conn)
    .map_err(|e| e.to_string())?
    .into_iter()
    .map(|row| row.code)
    .collect();

    let source_health = source_health_snapshot(&mut conn, as_of);
    let shadow = decision_shadow_enabled();

    let mut report = ProposalReport {
        scanned: observing.len(),
        ..Default::default()
    };

    for code in &observing {
        let ticket = match ticket_lifecycle::find_active_ticket(&mut conn, code) {
            Ok(Some(ticket)) => ticket,
            Ok(None) => continue,
            Err(error) => {
                report.errors.push(format!("{code} 查询失败: {error}"));
                continue;
            }
        };

        // 2026-10-24（情绪—逻辑框架）：连板 ≥4 属情绪段，不给 ma 回踩价。
        // 依据：涨停次日触及 ma10 仅 15.4%，连板越高越不回踩 —— 给回踩价等于给不成交的价。
        let streak = crate::monitor::market_regime::streak_on(&mut conn, as_of, code);
        if streak.is_some_and(|value| value >= crate::monitor::market_regime::EMOTION_STREAK) {
            report
                .skipped_emotion_phase
                .push(format!("{code}（{} 板）", streak.unwrap_or(0)));
            continue;
        }

        let zone = match buy_zone::compute(&mut conn, code, as_of) {
            Ok(Some(zone)) => zone,
            Ok(None) => {
                report.skipped_no_data.push(code.to_string());
                continue;
            }
            Err(error) => {
                report.errors.push(format!("{code} 买点计算失败: {error}"));
                continue;
            }
        };

        // 超买：连板 / 近 5 日涨幅过大 / 偏离 ma5 过多。这类票不是「没数据」，
        // 是明确不建议追，分开计数便于盘后复盘。
        if zone.overheated {
            report.skipped_overheat.push(code.to_string());
            continue;
        }
        if zone.at_support {
            report.skipped_at_support.push(code.to_string());
            continue;
        }

        let (low, high) = match (zone.entry_low, zone.entry_high) {
            (Some(low), Some(high)) if low > 0.0 && high >= low => (low, high),
            _ => {
                report.skipped_no_data.push(code.to_string());
                continue;
            }
        };

        match already_proposed_today(&mut conn, ticket.id, as_of) {
            Ok(true) => {
                report.skipped_duplicate += 1;
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                report.errors.push(format!("{code} {error}"));
                continue;
            }
        }

        let (regime_label, entry_mode, valid_until) =
            match ticket_regime::resolve(&mut conn, ticket.id, code, &ticket.source, as_of) {
                Ok(decision) => (
                    decision.regime.as_str().to_owned(),
                    ticket_decision::entry_mode_for(decision.regime, zone.trend, &ticket.source),
                    valid_until_for(as_of, decision.regime),
                ),
                Err(error) => {
                    report.errors.push(format!("{code} 分型失败: {error}"));
                    ("unknown".to_owned(), EntryMode::Unknown, None)
                }
            };

        let mut context = DecisionContext::new(
            ticket.id,
            code.clone(),
            as_of,
            ticket.source.clone(),
            regime_label,
            entry_mode,
        );
        context.price_low = Some(low);
        context.price_high = Some(high);
        context.trigger_level = zone.ma5;
        context.abandon = zone.abandon;
        context.confirmation = "price_only".to_owned();
        context.valid_from = Some(as_of.to_owned());
        context.valid_until = valid_until;
        context.source_health = source_health.clone();
        context.evidence_grade = EvidenceGrade::L1;
        context.missing_fields.push("volume_ratio".to_owned());
        context.seal();

        if shadow {
            match ticket_decision::persist_proposal(&db, &context) {
                Ok(()) => log::info!(
                    "[入场参考][shadow] code={} decision_id={} mode={} low={low:.2} high={high:.2} valid_until={:?}",
                    code,
                    context.decision_id,
                    context.entry_mode.as_str(),
                    context.valid_until
                ),
                Err(error) => report
                    .errors
                    .push(format!("{code} shadow context 写入失败: {error}")),
            }
        }

        let summary = format!(
            "{low:.2}-{high:.2}｜{}｜放弃 {:.2}",
            zone.trigger,
            zone.abandon.unwrap_or(0.0)
        );

        if let Err(error) = ticket_lifecycle::append_event(
            &mut conn,
            ticket.id,
            as_of,
            "entry_proposed",
            None,
            None,
            Some(&summary),
        ) {
            report.errors.push(format!("{code} 记事件失败: {error}"));
            continue;
        }

        match ticket_lifecycle::set_stage(&mut conn, ticket.id, "entry_proposed") {
            Ok(changed) if changed > 0 => {
                report.proposed.push(code.clone());
                // 2026-10-24：归因扩到入场提案（原表只接 T0Advice）。
                // best-effort：失败只记 error，不影响提案本身。
                if let Err(error) =
                    crate::database::advice_outcome_tracking::record_advice_push_simple(
                        &mut conn,
                        "EntryProposal",
                        as_of,
                        &code,
                        &chrono::Utc::now().to_rfc3339(),
                    )
                {
                    report.errors.push(format!("{code} 归因落库失败: {error}"));
                }
            }
            Ok(_) => {
                // 事件写了但 stage 没动：票可能刚被别处推进/关闭。不算失败，
                // 但要能看到，否则会以为推成功了。
                report
                    .errors
                    .push(format!("{code} 事件已记但 stage 未变更"));
            }
            Err(error) => report.errors.push(format!("{code} 推进失败: {error}")),
        }
    }

    Ok(report)
}
