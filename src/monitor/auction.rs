//! 集合竞价扫描器（09:15-09:25）。
//!
//! A股主力常在竞价阶段通过虚假申报或爆量低开表达意图。
//! 09:25 的竞价结果是对 08:45 Checklist 的最终修正。

use crate::calendar::is_auction_now;
use crate::monitor::detector::{AlertCategory, AlertEvent, AlertLevel};
use chrono::Local;

/// 竞价扫描结果
#[derive(Debug, Clone)]
pub struct AuctionResult {
    pub code: String,
    pub name: String,
    /// 竞价涨幅（%）
    pub gap_pct: f64,
    /// 竞价量比（相对近5日均量）
    pub vol_ratio: f64,
    /// 匹配量占比（%）
    pub match_ratio: f64,
    /// 是否疑似虚假申报（09:20 前大单封涨→09:20撤单）
    pub suspected_fake: bool,
}

impl AuctionResult {
    /// 是否为异常竞价（需要告警）
    pub fn is_abnormal(&self, gap_threshold: f64, vol_threshold: f64) -> bool {
        self.gap_pct.abs() >= gap_threshold && self.vol_ratio >= vol_threshold
    }

    /// 竞价方向
    pub fn direction(&self) -> &'static str {
        if self.gap_pct > 0.0 {
            "高开"
        } else if self.gap_pct < 0.0 {
            "低开"
        } else {
            "平开"
        }
    }
}

/// 从竞价结果生成告警事件
pub fn classify_auction(
    r: &AuctionResult,
    t1_locked: bool,
    gap_pct: f64,
    vol_ratio: f64,
) -> Option<AlertEvent> {
    if !r.is_abnormal(gap_pct, vol_ratio) {
        return None;
    }

    let (level, mut msg) = if r.gap_pct > gap_pct {
        // 高开抢筹
        (
            AlertLevel::Important,
            format!(
                "{} 竞价高开 {:.1}%，量比 {:.1}，疑似抢筹",
                r.name, r.gap_pct, r.vol_ratio
            ),
        )
    } else if r.gap_pct < -gap_pct {
        // 低开出逃
        let lvl = if r.gap_pct < -5.0 && t1_locked {
            AlertLevel::Emergency
        } else {
            AlertLevel::Important
        };
        let mut m = format!(
            "{} 竞价低开 {:.1}%，量比 {:.1}，疑似出逃",
            r.name, r.gap_pct, r.vol_ratio
        );
        if t1_locked {
            m.push_str(" ⚠️ T+1锁仓解禁日");
        }
        (lvl, m)
    } else {
        return None;
    };

    if r.suspected_fake {
        msg.push_str(" ⚠️ 疑似虚假申报（诱多）");
    }

    Some(AlertEvent {
        level,
        category: AlertCategory::AuctionGap,
        code: r.code.clone(),
        name: r.name.clone(),
        message: msg,
        detail: crate::monitor::detector::AlertDetail {
            price: None,
            change_pct: Some(r.gap_pct),
            volume_ratio: Some(r.vol_ratio),
            main_flow_yi: None,
            threshold: Some(gap_pct),
            news_title: None,
            news_summary: None,
            news_importance: None,
            ai_decision: None,
            t1_locked,
            extra: if r.suspected_fake {
                Some("疑似虚假申报（诱多），该股买入信号降级".into())
            } else {
                None
            },
        },
        triggered_at: Local::now(),
        routed_external_id: None,
    })
}

/// 检查当前是否应该执行竞价扫描
pub fn should_scan() -> bool {
    is_auction_now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auction_abnormal_high_open() {
        let r = AuctionResult {
            code: "TEST_CODE_000001".into(),
            name: "测试".into(),
            gap_pct: 5.0,
            vol_ratio: 6.0,
            match_ratio: 80.0,
            suspected_fake: false,
        };
        assert!(r.is_abnormal(3.0, 5.0));
        let event = classify_auction(&r, false, 3.0, 5.0).unwrap();
        assert_eq!(event.level, AlertLevel::Important);
        assert!(event.message.contains("高开"));
    }

    #[test]
    fn test_auction_normal() {
        let r = AuctionResult {
            code: "TEST_CODE_000001".into(),
            name: "测试".into(),
            gap_pct: 1.0,
            vol_ratio: 2.0,
            match_ratio: 70.0,
            suspected_fake: false,
        };
        assert!(!r.is_abnormal(3.0, 5.0));
    }

    #[test]
    fn test_auction_t1_locked_low_open() {
        let r = AuctionResult {
            code: "TEST_CODE_000002".into(),
            name: "锁仓股".into(),
            gap_pct: -6.0,
            vol_ratio: 8.0,
            match_ratio: 90.0,
            suspected_fake: false,
        };
        let event = classify_auction(&r, true, 3.0, 5.0).unwrap();
        assert_eq!(event.level, AlertLevel::Emergency);
        assert!(event.message.contains("T+1"));
    }

    #[test]
    fn test_auction_suspected_fake() {
        let r = AuctionResult {
            code: "TEST_CODE_000003".into(),
            name: "诱多股".into(),
            gap_pct: 8.0,
            vol_ratio: 10.0,
            match_ratio: 30.0,
            suspected_fake: true,
        };
        let event = classify_auction(&r, false, 3.0, 5.0).unwrap();
        assert!(event.message.contains("虚假申报"));
    }

    #[test]
    fn test_direction_labels() {
        let high = AuctionResult {
            code: "1".into(),
            name: "a".into(),
            gap_pct: 5.0,
            vol_ratio: 1.0,
            match_ratio: 50.0,
            suspected_fake: false,
        };
        assert_eq!(high.direction(), "高开");
        let low = AuctionResult {
            code: "2".into(),
            name: "b".into(),
            gap_pct: -2.0,
            vol_ratio: 1.0,
            match_ratio: 50.0,
            suspected_fake: false,
        };
        assert_eq!(low.direction(), "低开");
    }
}
