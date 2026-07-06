//! v9.1 调度器: 盘前 batch + 盘中 incremental
//!
//! 修复 v9.1 §1.3 调度空白: event_extractor 5 模块已完成, 但没人调用
//! 现在: 盘前 9:00 + 盘中每 5min 触发 extract_batch / extract_incremental
//!
//! v22: 增加 push 调度字段 (替代 v17.6 写死的 09:00-19:00 窗口)

use std::time::Duration;
use chrono::{Local, NaiveTime, Timelike};

/// 修复 v9.1 §1.3 + v22: 触发时刻表
#[derive(Debug, Clone)]
pub struct OpportunitySchedule {
    /// 盘前 batch 时刻 (默认 09:00)
    pub batch_morning: NaiveTime,
    /// 盘后 batch 时刻 (默认 15:30)
    pub batch_evening: NaiveTime,
    /// 盘中 incremental 间隔
    pub incremental_interval: Duration,
    // v22: push 推送窗口 (替代 v17.6 写死的 09:00-19:00)
    /// 盘前 push 时刻 (默认 09:00, 触发 P-01)
    pub push_preopen: NaiveTime,
    /// 盘中 push 时刻列表 (默认 [10:30, 11:00, 14:30], 触发 4 个盘中 dispatcher)
    pub push_intraday: Vec<NaiveTime>,
    /// 盘后 push 时刻 (默认 19:00, 触发 A-01)
    pub push_evening: NaiveTime,
}

impl Default for OpportunitySchedule {
    fn default() -> Self {
        Self {
            batch_morning: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            batch_evening: NaiveTime::from_hms_opt(15, 30, 0).unwrap(),
            incremental_interval: Duration::from_secs(5 * 60),
            // v22: push 推送窗口配置
            push_preopen: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            push_intraday: vec![
                NaiveTime::from_hms_opt(10, 30, 0).unwrap(),
                NaiveTime::from_hms_opt(11, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(14, 30, 0).unwrap(),
            ],
            push_evening: NaiveTime::from_hms_opt(19, 0, 0).unwrap(),
        }
    }
}

impl OpportunitySchedule {
    /// 修复 v9.1 §1.3: 计算距下次触发的秒数
    /// 返回 0 = 当前时刻就是触发时刻 (立即触发)
    pub fn seconds_until_next_trigger(&self) -> u64 {
        self.seconds_until_next_trigger_at(Local::now().time())
    }

    pub fn seconds_until_next_trigger_at(&self, now: NaiveTime) -> u64 {
        let triggers = [self.batch_morning, self.batch_evening];
        let now_secs = now.num_seconds_from_midnight() as u64;

        let mut best = 86400u64;
        for t in triggers {
            let trigger_secs = t.num_seconds_from_midnight() as u64;
            let diff = if trigger_secs >= now_secs {
                trigger_secs - now_secs
            } else {
                trigger_secs + 86400 - now_secs
            };
            if diff < best { best = diff; }
        }
        best
    }

    /// 修复 v9.1 §1.3: 距下次 incremental 的秒数 (固定间隔)
    pub fn seconds_until_incremental(&self) -> u64 {
        self.incremental_interval.as_secs()
    }

    /// v22: 检查当前时刻是否在 push 窗口内
    /// 返回枚举区分窗口类型 (盘前/盘中/盘后)
    pub fn push_window(&self, now: NaiveTime) -> PushWindow {
        // 比较秒数 (NaiveTime -> num_seconds_from_midnight)
        let now_secs = now.num_seconds_from_midnight() as u64;
        let preopen_secs = self.push_preopen.num_seconds_from_midnight() as u64;
        let evening_secs = self.push_evening.num_seconds_from_midnight() as u64;

        if now_secs == preopen_secs {
            return PushWindow::Preopen;
        }
        if self.push_intraday.contains(&now) {
            return PushWindow::Intraday;
        }
        if now_secs == evening_secs {
            return PushWindow::Evening;
        }
        PushWindow::Outside
    }
}

/// v22: push 窗口枚举
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PushWindow {
    /// 盘前 (P-01 触发)
    Preopen,
    /// 盘中 (4 个 dispatcher: I-01/I-02/I-03/D-01)
    Intraday,
    /// 盘后 (A-01 触发, 无时间窗)
    Evening,
    /// 窗口外 (仅 A-01 兜底 + 提示)
    Outside,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seconds_until_next_trigger_morning() {
        let s = OpportunitySchedule::default();
        let now = NaiveTime::from_hms_opt(8, 0, 0).unwrap(); // 8:00
        let secs = s.seconds_until_next_trigger_at(now);
        assert_eq!(secs, 3600, "8:00 距 9:00 必为 3600s");
    }

    #[test]
    fn test_seconds_until_next_trigger_evening() {
        let s = OpportunitySchedule::default();
        let now = NaiveTime::from_hms_opt(10, 0, 0).unwrap();
        let secs = s.seconds_until_next_trigger_at(now);
        // 10:00 → 15:30 = 5.5h = 19800s
        assert_eq!(secs, 19800);
    }

    #[test]
    fn test_seconds_until_next_trigger_after_evening_wraps() {
        let s = OpportunitySchedule::default();
        let now = NaiveTime::from_hms_opt(20, 0, 0).unwrap(); // 20:00 (过 15:30, wrap 到明天 9:00)
        let secs = s.seconds_until_next_trigger_at(now);
        // 20:00 → 明天 9:00 = 13h = 46800s
        assert_eq!(secs, 13 * 3600, "20:00 距明天 9:00 必为 13h");
    }

    #[test]
    fn test_incremental_interval_seconds() {
        let s = OpportunitySchedule::default();
        assert_eq!(s.seconds_until_incremental(), 5 * 60);
    }

    #[test]
    fn test_custom_schedule() {
        let s = OpportunitySchedule {
            batch_morning: NaiveTime::from_hms_opt(8, 30, 0).unwrap(),
            batch_evening: NaiveTime::from_hms_opt(16, 0, 0).unwrap(),
            incremental_interval: Duration::from_secs(10 * 60),
            push_preopen: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            push_intraday: vec![NaiveTime::from_hms_opt(10, 0, 0).unwrap()],
            push_evening: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
        };
        let now = NaiveTime::from_hms_opt(15, 0, 0).unwrap();
        let secs = s.seconds_until_next_trigger_at(now);
        assert_eq!(secs, 3600, "15:00 距 16:00 必为 3600s");
        assert_eq!(s.seconds_until_incremental(), 600);
    }

    // v22: push_window 测试
    #[test]
    fn test_push_window_default() {
        let s = OpportunitySchedule::default();
        // 09:00 盘前
        assert_eq!(
            s.push_window(NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
            PushWindow::Preopen
        );
        // 10:30 盘中
        assert_eq!(
            s.push_window(NaiveTime::from_hms_opt(10, 30, 0).unwrap()),
            PushWindow::Intraday
        );
        // 11:00 盘中
        assert_eq!(
            s.push_window(NaiveTime::from_hms_opt(11, 0, 0).unwrap()),
            PushWindow::Intraday
        );
        // 14:30 盘中
        assert_eq!(
            s.push_window(NaiveTime::from_hms_opt(14, 30, 0).unwrap()),
            PushWindow::Intraday
        );
        // 19:00 盘后
        assert_eq!(
            s.push_window(NaiveTime::from_hms_opt(19, 0, 0).unwrap()),
            PushWindow::Evening
        );
        // 10:00 窗口外
        assert_eq!(
            s.push_window(NaiveTime::from_hms_opt(10, 0, 0).unwrap()),
            PushWindow::Outside
        );
        // 20:00 窗口外 (盘后后)
        assert_eq!(
            s.push_window(NaiveTime::from_hms_opt(20, 0, 0).unwrap()),
            PushWindow::Outside
        );
    }

    #[test]
    fn test_push_window_custom() {
        // 自定义时刻, 测试不依赖默认值
        let s = OpportunitySchedule {
            batch_morning: NaiveTime::from_hms_opt(8, 30, 0).unwrap(),
            batch_evening: NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            incremental_interval: Duration::from_secs(5 * 60),
            push_preopen: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            push_intraday: vec![NaiveTime::from_hms_opt(10, 0, 0).unwrap()],
            push_evening: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
        };
        assert_eq!(s.push_window(NaiveTime::from_hms_opt(8, 0, 0).unwrap()), PushWindow::Preopen);
        assert_eq!(s.push_window(NaiveTime::from_hms_opt(10, 0, 0).unwrap()), PushWindow::Intraday);
        assert_eq!(s.push_window(NaiveTime::from_hms_opt(20, 0, 0).unwrap()), PushWindow::Evening);
        assert_eq!(s.push_window(NaiveTime::from_hms_opt(8, 30, 0).unwrap()), PushWindow::Outside);
    }
}
