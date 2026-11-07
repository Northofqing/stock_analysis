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
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use diesel::OptionalExtension;
use diesel::{QueryableByName, RunQueryDsl, SqliteConnection};

use crate::database::ticket_lifecycle;
use crate::database::DatabaseManager;
use crate::monitor::buy_zone;
use crate::monitor::selection_context::{self, ExtensionPolicy, SelectionContext};
use crate::monitor::ticket_decision::{
    self, DecisionContext, EntryMode, EvidenceGrade, ProposalStatus, SourceHealth, SourceStatus,
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
    /// 情绪型票命中超买门但按 D1 豁免（2026-11-07）：不再直接跳过。
    pub exempt_overheat: Vec<String>,
    pub skipped_at_support: Vec<String>,
    pub skipped_no_data: Vec<String>,
    /// 连板 ≥4 的情绪段票：不给 ma 回踩价（2026-10-24）
    pub skipped_emotion_phase: Vec<String>,
    pub skipped_duplicate: usize,
    pub superseded: usize,
    pub errors: Vec<String>,
}

impl ProposalReport {
    pub fn summary_line(&self) -> String {
        format!(
            "[入场参考] scanned={} proposed={} overheated={} overheat_exempt={} at_support={} no_data={} emotion_phase={} duplicate={} superseded={} errors={}",
            self.scanned,
            self.proposed.len(),
            self.skipped_overheat.len(),
            self.exempt_overheat.len(),
            self.skipped_at_support.len(),
            self.skipped_no_data.len(),
            self.skipped_emotion_phase.len(),
            self.skipped_duplicate,
            self.superseded,
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

/// 新提案落库后，把上一张仍为 Open 的提案收口为 Superseded。
/// 事件保留旧 decision_id，便于审计时看到替换链。
fn supersede_previous(
    conn: &mut SqliteConnection,
    ticket_id: i64,
    as_of: &str,
    previous: &DecisionContext,
    new_decision_id: &str,
) -> Result<(), String> {
    let summary = format!("被新提案 {new_decision_id} 取代");
    ticket_lifecycle::append_event(
        conn,
        ticket_id,
        as_of,
        "entry_superseded",
        None,
        Some(&previous.decision_id),
        Some(&summary),
    )
    .map_err(|error| format!("entry_superseded 落库失败: {error}"))?;
    let changed = ticket_decision::update_proposal_status(
        conn,
        &previous.decision_id,
        ProposalStatus::Superseded,
    )?;
    if !changed {
        return Err(format!(
            "entry_superseded 已记账但 ticket_decision 不存在: {}",
            previous.decision_id
        ));
    }
    Ok(())
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

/// 提案日的日线最高价：下一交易日触发时它就是「昨高」。
///
/// 取 `date <= as_of` 的最后一根：15:40 提案时当日日线已落库，21:30 后补跑同理。
/// 没有日线返回 None，不补值（BR-222）。
fn prev_high_for_proposal(
    conn: &mut SqliteConnection,
    code: &str,
    as_of: &str,
) -> Result<Option<f64>, String> {
    #[derive(QueryableByName)]
    struct HighRow {
        #[diesel(sql_type = Nullable<Double>)]
        high: Option<f64>,
    }
    let row = diesel::sql_query(
        "SELECT high FROM stock_daily \
          WHERE code = ?1 AND date <= ?2 AND high IS NOT NULL \
          ORDER BY date DESC LIMIT 1",
    )
    .bind::<Text, _>(code)
    .bind::<Text, _>(as_of)
    .get_result::<HighRow>(conn)
    .optional()
    .map_err(|error| format!("昨高查询失败 ({code}): {error}"))?;
    Ok(row.and_then(|row| row.high))
}

/// 入场参考正文：回踩区间 + 理由 + 放弃价 + 突破确认价（昨高）。
///
/// 两个价格是两种执行方式，不能混成一个：区间是回踩才买，突破确认是站上昨高才追。
/// 区间必须在最前面——周报按 `A-B｜…` 解析可成交率。
fn proposal_summary(
    low: f64,
    high: f64,
    trigger: &str,
    abandon: Option<f64>,
    prev_high: Option<f64>,
) -> String {
    let mut summary = format!(
        "{low:.2}-{high:.2}｜{trigger}｜放弃 {:.2}",
        abandon.unwrap_or(0.0)
    );
    match prev_high {
        Some(value) => summary.push_str(&format!("｜突破确认 {value:.2}（站上才追）")),
        None => summary.push_str("｜突破确认缺失（无日线）"),
    }
    summary
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

        // 分型要先于买点计算（2026-11-07，D1）：情绪型豁免超买门，需要把
        // allow_overheat 传给 buy_zone —— overheat 命中时它会清空 entry_low/high，
        // 只在提案层跳过门是拿不到价格的。resolve 只读，不写事件。
        let (regime, regime_label) =
            match ticket_regime::resolve(&mut conn, ticket.id, code, &ticket.source, as_of) {
                Ok(decision) => (Some(decision.regime), decision.regime.as_str().to_owned()),
                Err(error) => {
                    report.errors.push(format!("{code} 分型失败: {error}"));
                    (None, "unknown".to_owned())
                }
            };
        let allow_overheat = regime == Some(Regime::Emotion);

        let zone = match buy_zone::compute_with_options(&mut conn, code, as_of, allow_overheat) {
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
        // 情绪型按 D1 豁免（不再跳过），但单独计数，便于比较豁免前后的成交与收益。
        if zone.overheated {
            if allow_overheat {
                report.exempt_overheat.push(code.to_string());
            } else {
                report.skipped_overheat.push(code.to_string());
                continue;
            }
        }

        let entry_mode = match regime {
            Some(value) => ticket_decision::entry_mode_for(value, zone.trend, &ticket.source),
            None => EntryMode::Unknown,
        };
        let valid_until = regime.and_then(|value| valid_until_for(as_of, value));
        if zone.at_support {
            report.skipped_at_support.push(code.to_string());
            continue;
        }

        // 选股时点上下文：只记录“选之前走了多远”，不改变本函数的推进会/不推进会。
        // 任一窗口不足时保留 None，后续 ExtensionPolicy 标 Unknown。
        let (selection, selection_error) = match selection_context::load(&mut conn, code, as_of) {
            Ok(selection) => (selection, None),
            Err(error) => {
                log::warn!("[入场参考][selection] code={code} 读取失败: {error}");
                (SelectionContext::default(), Some(error))
            }
        };
        let extension = ExtensionPolicy::evaluate(&selection);

        let (low, high) = match (zone.entry_low, zone.entry_high) {
            (Some(low), Some(high)) if low > 0.0 && high >= low => (low, high),
            _ => {
                report.skipped_no_data.push(code.to_string());
                continue;
            }
        };

        let prev_high = match prev_high_for_proposal(&mut conn, code, as_of) {
            Ok(value) => value,
            Err(error) => {
                report.errors.push(format!("{code} {error}"));
                None
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

        let previous_context = if shadow {
            match ticket_decision::latest_open_proposal(&mut conn, ticket.id) {
                Ok(context) => context,
                Err(error) => {
                    report
                        .errors
                        .push(format!("{code} 旧提案查询失败: {error}"));
                    None
                }
            }
        } else {
            None
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
        context.prev_high = prev_high;
        context.abandon = zone.abandon;
        context.confirmation = "price_only".to_owned();
        context.valid_from = Some(as_of.to_owned());
        context.valid_until = valid_until;
        context.source_health = source_health.clone();
        context.evidence_grade = EvidenceGrade::L1;
        context.missing_fields.push("volume_ratio".to_owned());
        if prev_high.is_none() {
            context.missing_fields.push("prev_high".to_owned());
        }
        context.selection = selection;
        context.extension = extension;
        if selection_error.is_some() {
            context.missing_fields.push("selection_context".to_owned());
        }
        context.supersedes = previous_context
            .as_ref()
            .map(|previous| previous.decision_id.clone());
        context.seal();

        log::info!(
            "[入场参考][selection] code={} state={} pre5={:?} pre10={:?} pre20={:?} dist_hi60={:?} pos60={:?} streak={:?} reasons={}",
            code,
            context.extension.state.as_str(),
            context.selection.pre5_return,
            context.selection.pre10_return,
            context.selection.pre20_return,
            context.selection.dist_hi60,
            context.selection.position_60d,
            context.selection.limit_up_streak,
            context.extension.reasons.join(",")
        );

        if shadow {
            match ticket_decision::persist_proposal(&db, &context) {
                Ok(()) => {
                    log::info!(
                        "[入场参考][shadow] code={} decision_id={} mode={} low={low:.2} high={high:.2} valid_until={:?}",
                        code,
                        context.decision_id,
                        context.entry_mode.as_str(),
                        context.valid_until
                    );
                    if let Some(previous) = previous_context {
                        if let Err(error) = supersede_previous(
                            &mut conn,
                            ticket.id,
                            as_of,
                            &previous,
                            &context.decision_id,
                        ) {
                            report
                                .errors
                                .push(format!("{code} 旧提案收口失败: {error}"));
                        } else {
                            report.superseded += 1;
                        }
                    }
                }
                Err(error) => report
                    .errors
                    .push(format!("{code} shadow context 写入失败: {error}")),
            }
        }

        let summary = proposal_summary(low, high, &zone.trigger, zone.abandon, prev_high);

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

#[cfg(test)]
mod tests {
    use super::proposal_summary;

    #[test]
    fn proposal_summary_keeps_range_first_and_adds_breakout_price() {
        let text = proposal_summary(
            12.99,
            13.25,
            "回踩 ma10 13.12 附近可买；确认条件：量比>1.2",
            Some(11.90),
            Some(13.88),
        );
        // 周报按 `A-B｜…` 解析可成交率，区间必须在最前面
        assert!(text.starts_with("12.99-13.25｜"));
        assert!(text.contains("突破确认 13.88（站上才追）"));
        assert!(text.contains("放弃 11.90"));
    }

    #[test]
    fn proposal_summary_marks_missing_prev_high_instead_of_zero() {
        let text = proposal_summary(10.0, 10.2, "回踩 ma10", Some(9.5), None);
        assert!(text.contains("突破确认缺失（无日线）"));
        assert!(!text.contains("突破确认 0.00"));
    }
}
