//! 由持仓快照驱动 `ticket_lifecycle.stage` 自动流转（2026-10-18）。
//!
//! ## 为什么需要
//!
//! `ticket_lifecycle.stage` 有 7 个取值，但此前**只能手工改**（`ticket_open --stage`）。
//! 持仓快照里出现一只票、或某只票被清仓，系统都不会自己推进——票的生命周期是一张
//! 静态表，看着有记录、实际不会走动。
//!
//! ## 驱动源：持仓快照（客观事实，不需要人判断）
//!
//! 买入/卖出的唯一可信证据是持仓变化，不猜、不推断：
//!
//! - 持仓里有、ticket 没有          → 开票，stage=holding，source=holding
//! - 持仓里有、stage 是 observing   → 推进到 holding（观察池的票被买了）
//! - 持仓里没有、stage 是 holding   → 关闭 ticket，stage=exited
//! - 持仓里没有、stage 是 observing → 不动（观察池的票本来就不在持仓里）
//!
//! 加仓/减仓**不在本模块处理**：`ticket_lifecycle` 不存股数，无法判断"数量变化"。
//! 那需要一张持仓历史表（`user_position_snapshot` 每次导入留存），属于后续工作。
//!
//! ## 边界
//!
//! - 只读 `portfolio::load_positions()`（当前持仓）+ `ticket_lifecycle`
//! - 写操作全部幂等（`find_active_ticket` 先查、`set_stage` 带 `close_date IS NULL` 条件）
//! - 任何一只票失败只记 error，不中断其余票的处理

use std::collections::HashSet;

use diesel::prelude::*;
use diesel::sql_types::Text;

use crate::database::{ticket_lifecycle, DatabaseManager};

#[derive(Debug, Default, Clone)]
pub struct StageReport {
    pub held: usize,
    pub active: usize,
    /// observing / entry_proposed → holding
    pub promoted: Vec<String>,
    /// 持仓里有但没有任何 ticket，按持仓开了新票
    pub opened: Vec<String>,
    /// holding → exited（已清仓）
    pub exited: Vec<String>,
    pub errors: Vec<String>,
}

impl StageReport {
    pub fn log(&self) {
        log::info!(
            "[票状态] held={} active={} promoted={} opened={} exited={}",
            self.held,
            self.active,
            self.promoted.len(),
            self.opened.len(),
            self.exited.len()
        );
        for code in &self.promoted {
            log::info!("[票状态] {code} 观察 → 持有（持仓快照出现）");
        }
        for code in &self.opened {
            log::info!("[票状态] {code} 无观察记录，按持仓开票");
        }
        for code in &self.exited {
            log::info!("[票状态] {code} 持有 → 退出（持仓快照已无）");
        }
        for error in &self.errors {
            log::warn!("[票状态] {error}");
        }
    }
}

pub fn sync_stages_from_positions(
    db: &DatabaseManager,
    as_of: &str,
) -> Result<StageReport, String> {
    let positions = crate::portfolio::load_positions().unwrap_or_default();
    let held: HashSet<String> = positions
        .iter()
        .map(|position| position.code.clone())
        .collect();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;

    #[derive(QueryableByName)]
    struct CodeRow {
        #[diesel(sql_type = Text)]
        code: String,
    }
    let active_codes: Vec<String> =
        diesel::sql_query("SELECT code FROM ticket_lifecycle WHERE close_date IS NULL")
            .load::<CodeRow>(&mut conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|row| row.code)
            .collect();

    let mut report = StageReport {
        held: held.len(),
        active: active_codes.len(),
        ..Default::default()
    };

    // 1. 持仓票：没有 ticket 就开，observing 就推进到 holding
    for position in &positions {
        let code = &position.code;
        match ticket_lifecycle::find_active_ticket(&mut conn, code) {
            Ok(Some(ticket)) => {
                if ticket.stage == "observing" || ticket.stage == "entry_proposed" {
                    match ticket_lifecycle::set_stage(&mut conn, ticket.id, "holding") {
                        Ok(_) => {
                            let _ = ticket_lifecycle::append_event(
                                &mut conn,
                                ticket.id,
                                as_of,
                                "promoted_to_holding",
                                None,
                                None,
                                Some("持仓快照出现该票"),
                            );
                            report.promoted.push(code.clone());
                        }
                        Err(error) => report.errors.push(format!("{code} 推进失败: {error}")),
                    }
                }
            }
            Ok(None) => {
                let open_date = position.added_at.to_string();
                match ticket_lifecycle::open_ticket(
                    &mut conn,
                    code,
                    &open_date,
                    "holding",
                    "holding",
                    Some("持仓快照出现但无观察记录"),
                ) {
                    Ok(_) => report.opened.push(code.clone()),
                    Err(error) => report.errors.push(format!("{code} 开票失败: {error}")),
                }
            }
            Err(error) => report.errors.push(format!("{code} 查询失败: {error}")),
        }
    }

    // 2. holding 但已不在持仓：退出
    for code in &active_codes {
        if held.contains(code) {
            continue;
        }
        let ticket = match ticket_lifecycle::find_active_ticket(&mut conn, code) {
            Ok(Some(ticket)) => ticket,
            Ok(None) => continue,
            Err(error) => {
                report.errors.push(format!("{code} 查询失败: {error}"));
                continue;
            }
        };
        if ticket.stage != "holding" {
            continue;
        }
        match ticket_lifecycle::close_ticket(&mut conn, ticket.id, as_of, "exited") {
            Ok(_) => {
                let _ = ticket_lifecycle::append_event(
                    &mut conn,
                    ticket.id,
                    as_of,
                    "exited",
                    None,
                    None,
                    Some("持仓快照已无该票"),
                );
                report.exited.push(code.clone());
            }
            Err(error) => report.errors.push(format!("{code} 退出失败: {error}")),
        }
    }

    // 2026-11-11 A 方案双读：不改变 stage，只把三源差异摊到日志。
    // 视图缺失/查询失败只告警，不影响持仓驱动的状态同步。
    match ticket_lifecycle::active_ticket_states(&mut conn) {
        Ok(states) => {
            let conflicts = states.iter().filter(|state| state.status_conflict).count();
            let stale = states
                .iter()
                .filter(|state| state.stale_days.unwrap_or(0) >= 5)
                .count();
            let open = states
                .iter()
                .filter(|state| state.proposal_status.as_deref() == Some("open"))
                .count();
            let blocked = states
                .iter()
                .filter(|state| {
                    matches!(
                        state.proposal_status.as_deref(),
                        Some("triggered" | "invalidated" | "expired" | "superseded")
                    )
                })
                .count();
            log::info!(
                "[票状态][state_v1] active={} status_conflict={} stale5={} open={} blocked={}",
                states.len(),
                conflicts,
                stale,
                open,
                blocked
            );
        }
        Err(error) => log::warn!("[票状态][state_v1] 双读失败: {error}"),
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_counts_are_independent() {
        let report = StageReport {
            held: 3,
            active: 2,
            promoted: vec!["600703".to_string()],
            ..Default::default()
        };
        assert_eq!(report.promoted.len(), 1);
        assert!(report.opened.is_empty());
        assert!(report.errors.is_empty());
        assert_eq!(report.held, 3);
    }
}
