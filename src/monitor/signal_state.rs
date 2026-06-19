//! 信号状态机：去重、幂等、静默、状态变化才告警。
//!
//! 核心规则：同一 (股票, 信号类型) 在冷却期内不重复告警。
//! 紧急级别告警无视冷却。状态变化（消失→出现）重新触发。

use crate::monitor::detector::{AlertCategory, AlertEvent, AlertLevel};
use chrono::{DateTime, Duration, Local};
use diesel::prelude::*;
use std::collections::HashMap;

// ============================================================================
// 状态定义
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignalState {
    #[allow(dead_code)]
    Idle,
    Firing,
    Cooldown,
}

struct SignalEntry {
    state: SignalState,
    last_alert: DateTime<Local>,
    last_change: DateTime<Local>,
}

// ============================================================================
// 状态机
// ============================================================================

pub struct SignalStateMachine {
    entries: HashMap<String, SignalEntry>,
    cooldown_important: Duration,
    cooldown_info: Duration,
    daily_important_max: usize,
    daily_info_max: usize,
    daily_important_count: usize,
    daily_info_count: usize,
}

impl SignalStateMachine {
    pub fn new(
        cooldown_important_secs: i64,
        cooldown_info_secs: i64,
        daily_important_max: usize,
        daily_info_max: usize,
    ) -> Self {
        Self {
            entries: HashMap::new(),
            cooldown_important: Duration::seconds(cooldown_important_secs),
            cooldown_info: Duration::seconds(cooldown_info_secs),
            daily_important_max,
            daily_info_max,
            daily_important_count: 0,
            daily_info_count: 0,
        }
    }

    /// 核心方法：接收原始告警事件，返回应该推送的事件（去重后）。
    /// 返回 None 表示被静默（冷却中 / 超预算 / 状态未变化）。
    pub fn process(&mut self, event: AlertEvent) -> Option<AlertEvent> {
        let key = make_key(&event.code, event.category);
        let now = Local::now();

        // 预算检查
        let budget_ok = match event.level {
            AlertLevel::Emergency => true, // 紧急无限制
            AlertLevel::Important => {
                if self.daily_important_count >= self.daily_important_max {
                    return None;
                }
                self.daily_important_count += 1;
                true
            }
            AlertLevel::Info => {
                if self.daily_info_count >= self.daily_info_max {
                    return None;
                }
                self.daily_info_count += 1;
                true
            }
        };

        if !budget_ok {
            return None;
        }

        // 紧急级别：1分钟内不重复，首次直接放行
        if event.level == AlertLevel::Emergency {
            let is_new = !self.entries.contains_key(&key);
            if is_new {
                self.entries.insert(key.clone(), SignalEntry {
                    state: SignalState::Firing,
                    last_alert: now,
                    last_change: now,
                });
                return Some(event);
            }
            let entry = self.entries.get_mut(&key).unwrap();
            if now - entry.last_alert < Duration::seconds(60) {
                return None;
            }
            entry.last_alert = now;
            entry.state = SignalState::Firing;
            return Some(event);
        }

        // 非紧急：冷却检查
        let cooldown = match event.level {
            AlertLevel::Important => self.cooldown_important,
            AlertLevel::Info => self.cooldown_info,
            AlertLevel::Emergency => unreachable!(),
        };

        match self.entries.get_mut(&key) {
            Some(entry) => {
                let elapsed = now - entry.last_alert;
                match entry.state {
                    SignalState::Cooldown => {
                        if elapsed < cooldown {
                            return None; // 冷却中，静默
                        }
                        // 冷却期满，信号仍在 → 重新触发
                        entry.state = SignalState::Firing;
                        entry.last_alert = now;
                        entry.last_change = now;
                        Some(event)
                    }
                    SignalState::Firing => {
                        // 仍在触发，但刚发过 → 静默
                        if elapsed < cooldown {
                            return None;
                        }
                        entry.last_alert = now;
                        Some(event)
                    }
                    SignalState::Idle => {
                        entry.state = SignalState::Firing;
                        entry.last_alert = now;
                        entry.last_change = now;
                        Some(event)
                    }
                }
            }
            None => {
                self.entries.insert(
                    key,
                    SignalEntry {
                        state: SignalState::Firing,
                        last_alert: now,
                        last_change: now,
                    },
                );
                Some(event)
            }
        }
    }

    /// 标记信号消失（例如，股票不再涨停）。下次再出现时会重新触发。
    pub fn mark_resolved(&mut self, code: &str, cat: AlertCategory) {
        let key = make_key(code, cat);
        if let Some(entry) = self.entries.get_mut(&key) {
            if entry.state == SignalState::Firing {
                entry.state = SignalState::Cooldown;
                entry.last_change = Local::now();
            }
        }
    }

    /// 每日重置（盘前调用）
    pub fn daily_reset(&mut self) {
        self.entries.clear();
        self.daily_important_count = 0;
        self.daily_info_count = 0;
    }

    /// 当前活跃信号数
    pub fn active_count(&self) -> usize {
        self.entries
            .values()
            .filter(|e| e.state == SignalState::Firing)
            .count()
    }

    pub fn budget_remaining(&self) -> (usize, usize) {
        (
            self.daily_important_max.saturating_sub(self.daily_important_count),
            self.daily_info_max.saturating_sub(self.daily_info_count),
        )
    }

    /// 每 5 分钟将当前状态写入 signal_state 表
    pub fn flush_state(&self) {
        let db = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::database::DatabaseManager::get()
        })) {
            Ok(db) => db,
            Err(_) => return,
        };
        let mut conn = match db.get_conn() {
            Ok(c) => c,
            Err(_) => return,
        };
        for (key, entry) in &self.entries {
            let sql = format!(
                "INSERT OR REPLACE INTO signal_state(key, state, last_alert, last_change, daily_important_count, daily_info_count) \
                 VALUES ('{}', '{}', '{}', '{}', {}, {})",
                key.replace('\'', "''"),
                match entry.state { SignalState::Idle => "idle", SignalState::Firing => "firing", SignalState::Cooldown => "cooldown" },
                entry.last_alert.format("%Y-%m-%d %H:%M:%S"),
                entry.last_change.format("%Y-%m-%d %H:%M:%S"),
                self.daily_important_count, self.daily_info_count,
            );
            let _ = diesel::sql_query(&sql).execute(&mut *conn);
        }
    }

    /// 启动时从 signal_state 恢复状态，清理过期数据
    pub fn restore_state(&mut self) {
        let db = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::database::DatabaseManager::get()
        })) {
            Ok(db) => db,
            Err(_) => return,
        };
        let mut conn = match db.get_conn() {
            Ok(c) => c,
            Err(_) => return,
        };
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        #[derive(QueryableByName, Debug)]
        struct StateRow {
            #[diesel(sql_type = diesel::sql_types::Text)] key: String,
            #[diesel(sql_type = diesel::sql_types::Text)] state: String,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)] last_alert: Option<String>,
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)] last_change: Option<String>,
            #[diesel(sql_type = diesel::sql_types::Integer)] daily_important_count: i32,
            #[diesel(sql_type = diesel::sql_types::Integer)] daily_info_count: i32,
        }
        let sql = format!("SELECT key, state, last_alert, last_change, daily_important_count, daily_info_count FROM signal_state");
        if let Ok(rows) = diesel::sql_query(&sql).load::<StateRow>(&mut *conn) {
            for r in rows {
                // 只恢复有 last_change 且是今天的
                if let Some(ref lc) = r.last_change {
                    if lc.starts_with(&today) {
                        let state = match r.state.as_str() {
                            "firing" => SignalState::Firing,
                            "cooldown" => SignalState::Cooldown,
                            _ => SignalState::Idle,
                        };
                        let now = chrono::Local::now();
                        let entry = SignalEntry {
                            state,
                            last_alert: r.last_alert.as_ref().and_then(|s| {
                                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
                                    .and_then(|t| t.and_local_timezone(chrono::Local).latest())
                            }).unwrap_or(now),
                            last_change: chrono::NaiveDateTime::parse_from_str(lc, "%Y-%m-%d %H:%M:%S").ok()
                                .and_then(|t| t.and_local_timezone(chrono::Local).latest())
                                .unwrap_or(now),
                        };
                        self.entries.insert(r.key, entry);
                        self.daily_important_count = self.daily_important_count.max(r.daily_important_count as usize);
                        self.daily_info_count = self.daily_info_count.max(r.daily_info_count as usize);
                    }
                }
            }
        }
        // 清理非今天的
        let _ = diesel::sql_query(&format!("DELETE FROM signal_state WHERE last_change IS NULL OR last_change < '{}'", today))
            .execute(&mut *conn);
    }
}

fn make_key(code: &str, cat: AlertCategory) -> String {
    format!("{}:{}", code, cat.key())
}

impl Default for SignalStateMachine {
    fn default() -> Self {
        Self::new(300, 900, 30, 15) // 重要5分钟冷却, 参考15分钟
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::detector::AlertDetail;

    fn event(code: &str, level: AlertLevel, cat: AlertCategory) -> AlertEvent {
        AlertEvent {
            level,
            category: cat,
            code: code.into(),
            name: "测试".into(),
            message: "test".into(),
            detail: AlertDetail {
                price: None, change_pct: None, volume_ratio: None,
                main_flow_yi: None, threshold: None, news_title: None,
                news_summary: None, ai_decision: None,
                t1_locked: false, extra: None,
            },
            triggered_at: Local::now(),
        }
    }

    #[test]
    fn test_first_event_passes() {
        let mut sm = SignalStateMachine::default();
        assert!(sm.process(event("000001", AlertLevel::Important, AlertCategory::MainOutflow)).is_some());
    }

    #[test]
    fn test_duplicate_within_cooldown_blocked() {
        let mut sm = SignalStateMachine::new(300, 900, 30, 15);
        let e = event("000001", AlertLevel::Important, AlertCategory::MainOutflow);
        assert!(sm.process(e.clone()).is_some());
        assert!(sm.process(e.clone()).is_none()); // 立即重复 → 静默
    }

    #[test]
    fn test_emergency_passes_then_cooldown() {
        let mut sm = SignalStateMachine::new(300, 900, 30, 15);
        let e = event("000001", AlertLevel::Emergency, AlertCategory::LimitDown);
        assert!(sm.process(e.clone()).is_some());
        // 紧急级别60秒内不重复
        assert!(sm.process(e.clone()).is_none());
    }

    #[test]
    fn test_mark_resolved_then_re_trigger() {
        let mut sm = SignalStateMachine::new(1, 1, 30, 15); // 1秒冷却用于测试
        let e = event("000001", AlertLevel::Important, AlertCategory::MainOutflow);
        assert!(sm.process(e.clone()).is_some());
        sm.mark_resolved("000001", AlertCategory::MainOutflow);
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(sm.process(e).is_some()); // 冷却期满，重新触发
    }

    #[test]
    fn test_daily_budget_enforced() {
        let mut sm = SignalStateMachine::new(1, 1, 1, 1); // 预算各1条
        let e1 = event("000001", AlertLevel::Important, AlertCategory::MainInflow);
        let e2 = event("000002", AlertLevel::Important, AlertCategory::VolBurst);
        assert!(sm.process(e1).is_some());
        assert!(sm.process(e2).is_none()); // 超出预算
    }

    #[test]
    fn test_daily_reset_restores_budget() {
        let mut sm = SignalStateMachine::new(1, 1, 1, 1);
        sm.process(event("000001", AlertLevel::Important, AlertCategory::MainInflow));
        sm.daily_reset();
        assert!(sm.process(event("000002", AlertLevel::Important, AlertCategory::MainInflow)).is_some());
    }

    #[test]
    fn test_different_categories_independent() {
        let mut sm = SignalStateMachine::default();
        assert!(sm.process(event("000001", AlertLevel::Important, AlertCategory::MainOutflow)).is_some());
        assert!(sm.process(event("000001", AlertLevel::Important, AlertCategory::VolBurst)).is_some());
    }

    #[test]
    fn test_different_codes_independent() {
        let mut sm = SignalStateMachine::default();
        assert!(sm.process(event("000001", AlertLevel::Important, AlertCategory::MainOutflow)).is_some());
        assert!(sm.process(event("000002", AlertLevel::Important, AlertCategory::MainOutflow)).is_some());
    }
}
