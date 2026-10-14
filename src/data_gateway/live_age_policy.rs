//! 实时行情 age 门的**唯一口径**。
//!
//! 2026-10-14 用户指令「暂时不要五秒的限制」：全系统的行情 age 门由「5s 硬拒」
//! 改为「5s 仅标注 + 大阈值兜底拒绝」。
//!
//! 背景（生产实测，连续交易时段 13:00–14:11）：行情批次年龄 min 5.04s /
//! 中位 26–28s / p90 42–45s / max 102.7s，**≤5s 占比 0%**。根因是桥往返 +
//! server 侧整批组装（`convert.rs` 注释实测 11–55s），因此任何 5s 硬拒的消费路径
//! 在真实链路上都不会成立（2026-08-21 已对 T0Evidence 做过同样放宽）。
//!
//! 统一语义：
//! - `LIVE_QUOTE_ANNOTATE_AGE_SECONDS`（5s）＝**标注**阈值，超限逐次 `warn`，不拒绝；
//! - `LIVE_QUOTE_REJECT_AGE_SECONDS`（300s）＝**拒绝**阈值，只兜底「明显停摆/损坏」；
//! - **未来时间 / 时序倒挂不是延迟，仍各处硬拒**（本模块只管 age）。
//!
//! **回退**：把 `LIVE_QUOTE_REJECT_AGE_SECONDS` 改回 `5` 即恢复全系统原严格语义
//! （各处引用同一常量，不需要逐文件改）。

use chrono::TimeDelta;

/// 标注阈值：超过只 `warn`，不拒绝。
pub const LIVE_QUOTE_ANNOTATE_AGE_SECONDS: i64 = 5;

/// 拒绝阈值：只兜底「明显停摆/损坏」。**回退改这一处即可。**
pub const LIVE_QUOTE_REJECT_AGE_SECONDS: i64 = 300;

/// 同上，毫秒口径（供以 ms 比较的调用方使用）。
pub const LIVE_QUOTE_REJECT_AGE_MILLIS: i64 = LIVE_QUOTE_REJECT_AGE_SECONDS * 1000;

/// chrono 口径的拒绝阈值。
pub const LIVE_QUOTE_REJECT_AGE: TimeDelta = TimeDelta::seconds(LIVE_QUOTE_REJECT_AGE_SECONDS);

/// 超标注阈值但未达拒绝阈值 → 记一次标注日志（**非静默放行**）。
///
/// `scope` 用调用方能区分的短名（如 `ScannerQuotes` / `SelectionQuality`）。
pub fn annotate_superseded_age(scope: &str, age: TimeDelta) {
    log::warn!(
        "[{scope}][时间不可信] 行情 age={:?} 超标注阈值 {}s；临时拒绝阈值 {}s（2026-10-14 用户指令，见 live_age_policy）",
        age,
        LIVE_QUOTE_ANNOTATE_AGE_SECONDS,
        LIVE_QUOTE_REJECT_AGE_SECONDS
    );
}

/// age 是否只构成「标注」（既非新鲜、也未到拒绝阈值）。
pub fn is_annotation_only(age: TimeDelta) -> bool {
    age > TimeDelta::seconds(LIVE_QUOTE_ANNOTATE_AGE_SECONDS) && age <= LIVE_QUOTE_REJECT_AGE
}

/// age 是否已达拒绝阈值。
pub fn exceeds_reject_threshold(age: TimeDelta) -> bool {
    age > LIVE_QUOTE_REJECT_AGE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_boundaries_are_annotate_at_five_and_reject_at_three_hundred() {
        assert!(!is_annotation_only(TimeDelta::seconds(5)));
        assert!(is_annotation_only(
            TimeDelta::seconds(5) + TimeDelta::nanoseconds(1)
        ));
        assert!(is_annotation_only(TimeDelta::seconds(300)));
        assert!(!exceeds_reject_threshold(TimeDelta::seconds(300)));
        assert!(exceeds_reject_threshold(
            TimeDelta::seconds(300) + TimeDelta::nanoseconds(1)
        ));
        assert_eq!(LIVE_QUOTE_REJECT_AGE_MILLIS, 300_000);
    }
}
