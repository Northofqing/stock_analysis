//! 投递记账（P1.2, 2026-10-17）：把「已投递的推送」写进 ticket 的事件时间轴。
//!
//! ## 口径（operator 2026-10-17 定）
//!
//! 票**被观察池选出**（新闻催化 / 板块轮动）**或属于实际持仓** 才开票记账；
//! 其余推送的票不自动建档 —— 避免"没被观察过却突然出现"的票污染时间轴。
//!
//! ## 为什么是扫描式而不是推送钩子
//!
//! 推送链路是全系统最不能碰的一段（BR-201/205/211 四铁律 + 投递幂等）。
//! 用盘后扫描 `delivery_decisions` 的方式：零侵入、写失败不影响任何投递、
//! 不需要在每条 push 的 Delivered 分支插代码。代价是时间轴延迟到盘后补记。
//!
//! ## 幂等
//!
//! 去重键 = `(code, push_kind, occurrence_identity=business_date:push_kind)`：
//! 同一只票、同一类推送、同一业务日只记一次；重跑 sync 不会产生重复事件。

use std::collections::HashSet;
use std::path::Path;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};

use crate::database::ticket_lifecycle;
use crate::database::DatabaseManager;

const EVENT_KIND_DELIVERED: &str = "push_delivered";
const SOURCE_HOLDING: &str = "holding";
/// 盘后补记的统一事件时刻（真实投递时刻在 delivery_decisions 里，这里只做时间轴排序）。
const BACKFILL_EVENT_TIME_SUFFIX: &str = "T15:35:00+08:00";

#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    pub scanned: usize,
    pub recorded: usize,
    pub opened_for_holding: usize,
    pub skipped_no_ticket: usize,
    pub skipped_duplicate: usize,
    pub errors: Vec<String>,
}

impl SyncReport {
    pub fn log(&self) {
        log::info!(
            "[ticket记账] scanned={} recorded={} opened_for_holding={} skipped_no_ticket={} skipped_duplicate={}",
            self.scanned,
            self.recorded,
            self.opened_for_holding,
            self.skipped_no_ticket,
            self.skipped_duplicate
        );
        for error in &self.errors {
            log::warn!("[ticket记账] {error}");
        }
    }
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

/// 把 `business_date` 当天已投递的逐票推送记账到 ticket_event。
///
/// `delivery_db_path` 指向 `durable_delivery.sqlite3`（只读打开）。
pub fn sync_from_delivery(
    db: &DatabaseManager,
    delivery_db_path: &Path,
    business_date: &str,
) -> Result<SyncReport, String> {
    let mut report = SyncReport::default();

    let delivery = rusqlite::Connection::open_with_flags(
        delivery_db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| format!("open delivery db: {e}"))?;

    let pairs = {
        let mut stmt = delivery
            .prepare(
                "SELECT DISTINCT push_kind, scope_key FROM delivery_decisions \
                 WHERE business_date = ?1 AND state = 'Delivered' AND scope_key <> 'GLOBAL' \
                 ORDER BY push_kind, scope_key",
            )
            .map_err(|e| format!("prepare delivery query: {e}"))?;
        let rows = stmt
            .query_map([business_date], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("query delivery decisions: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read delivery decisions: {e}"))?;
        rows
    };

    report.scanned = pairs.len();
    if pairs.is_empty() {
        return Ok(report);
    }

    // 实际持仓（读不到就退化为"没有持仓可自动建档"，不阻断记账）
    let holding_codes: HashSet<String> = crate::portfolio::load_positions()
        .unwrap_or_default()
        .into_iter()
        .map(|position| position.code)
        .collect();

    for (push_kind, scope_key) in pairs {
        let Some(code) = extract_code(&scope_key) else {
            report
                .errors
                .push(format!("无法从 scope_key 提取 6 位代码: {scope_key}"));
            continue;
        };

        let mut conn = db.get_conn().map_err(|e| e.to_string())?;

        let identity = format!("{business_date}:{push_kind}");
        let already = diesel::sql_query(
            "SELECT COUNT(*) AS n FROM ticket_event \
             WHERE occurrence_identity = ?1 AND push_kind = ?2 \
               AND ticket_id IN (SELECT id FROM ticket_lifecycle WHERE code = ?3)",
        )
        .bind::<Text, _>(&identity)
        .bind::<Text, _>(&push_kind)
        .bind::<Text, _>(&code)
        .get_result::<CountRow>(&mut *conn)
        .map(|row| row.n)
        .unwrap_or(0);
        if already > 0 {
            report.skipped_duplicate += 1;
            continue;
        }

        let ticket = match ticket_lifecycle::find_active_ticket(&mut conn, &code) {
            Ok(Some(ticket)) => Some(ticket),
            Ok(None) => {
                if !holding_codes.contains(&code) {
                    // 既不在观察池、也不是持仓 → 不建档（operator 口径）
                    report.skipped_no_ticket += 1;
                    continue;
                }
                match ticket_lifecycle::open_ticket(
                    &mut conn,
                    &code,
                    business_date,
                    "holding",
                    SOURCE_HOLDING,
                    Some("实际持仓自动建档"),
                ) {
                    Ok(_id) => {
                        report.opened_for_holding += 1;
                        // open_ticket 幂等（INSERT OR IGNORE），实际 stage/source 以库内为准
                        ticket_lifecycle::find_active_ticket(&mut conn, &code)
                            .ok()
                            .flatten()
                    }
                    Err(error) => {
                        report.errors.push(format!("{code} 持仓开票失败: {error}"));
                        None
                    }
                }
            }
            Err(error) => {
                report.errors.push(format!("{code} 查询票据失败: {error}"));
                None
            }
        };

        let Some(ticket) = ticket else { continue };

        let event_at = format!("{business_date}{BACKFILL_EVENT_TIME_SUFFIX}");
        match ticket_lifecycle::append_event(
            &mut conn,
            ticket.id,
            &event_at,
            EVENT_KIND_DELIVERED,
            Some(&push_kind),
            Some(&identity),
            Some(&format!("推送已投递: {push_kind}")),
        ) {
            Ok(()) => report.recorded += 1,
            Err(error) => report.errors.push(format!("{code} 记账失败: {error}")),
        }
    }

    Ok(report)
}

/// `scope_key` 形如 `SHANGHAI:EQUITY:600396`；只接受尾部 6 位数字。
fn extract_code(scope_key: &str) -> Option<String> {
    let code = scope_key.rsplit(':').next()?;
    (code.len() == 6 && code.chars().all(|c| c.is_ascii_digit())).then(|| code.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_code_accepts_exchange_prefixed_scopes() {
        assert_eq!(
            extract_code("SHANGHAI:EQUITY:600396").as_deref(),
            Some("600396")
        );
        assert_eq!(
            extract_code("SHENZHEN:EQUITY:000813").as_deref(),
            Some("000813")
        );
    }

    #[test]
    fn extract_code_rejects_global_and_malformed() {
        assert_eq!(extract_code("GLOBAL"), None);
        assert_eq!(extract_code("SHANGHAI:EQUITY:60039"), None);
        assert_eq!(extract_code("SHANGHAI:EQUITY:60039A"), None);
        assert_eq!(extract_code(""), None);
    }
}
