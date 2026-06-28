//! v9.1 调度器: 盘前 batch + 盘中 incremental
//!
//! 修复 v9.1 §1.3 调度空白: event_extractor 5 模块已完成, 但没人调用
//! 现在: 盘前 9:00 + 盘中每 5min 触发 extract_batch / extract_incremental

use std::time::Duration;
use chrono::{Local, NaiveTime, Timelike};

/// 修复 v9.1 §1.3: 触发时刻表
#[derive(Debug, Clone)]
pub struct OpportunitySchedule {
    /// 盘前 batch 时刻 (默认 09:00)
    pub batch_morning: NaiveTime,
    /// 盘后 batch 时刻 (默认 15:30)
    pub batch_evening: NaiveTime,
    /// 盘中 incremental 间隔
    pub incremental_interval: Duration,
}

impl Default for OpportunitySchedule {
    fn default() -> Self {
        Self {
            batch_morning: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            batch_evening: NaiveTime::from_hms_opt(15, 30, 0).unwrap(),
            incremental_interval: Duration::from_secs(5 * 60),
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
        let now_secs = now.num_seconds_from_midnight();

        let mut best = 86400u64; // max 1 day
        for t in triggers {
            let trigger_secs = t.num_seconds_from_midnight();
            let diff = if trigger_secs >= now_secs {
                trigger_secs - now_secs
            } else {
                trigger_secs + 86400 - now_secs // wrap to tomorrow
            };
            if diff < best { best = diff; }
        }
        best
    }

    /// 修复 v9.1 §1.3: 距下次 incremental 的秒数 (固定间隔)
    pub fn seconds_until_incremental(&self) -> u64 {
        self.incremental_interval.as_secs()
    }
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
        };
        let now = NaiveTime::from_hms_opt(15, 0, 0).unwrap();
        let secs = s.seconds_until_next_trigger_at(now);
        assert_eq!(secs, 3600, "15:00 距 16:00 必为 3600s");
        assert_eq!(s.seconds_until_incremental(), 600);
    }
}
