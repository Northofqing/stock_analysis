//! v12 MVP-5 §8.4: 失败样本归因 (R-06 推送).
//!
//! 设计: FailureReason 10 变体 + 周分布统计 + 仅审计输出 (零自动调参).

use chrono::Local;
use std::collections::HashMap;

/// 失败归因 10 变体 (BR-016 模式枚举)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureReason {
    BuyTooLate,       // 买点过晚
    SectorFade,       // 板块退潮
    NotTradable,      // 不可成交 (涨停)
    HumanNotExecuted, // 人未执行
    StopLossHit,      // 止损触发
    MacdBearish,      // MACD 死叉
    VolumeDivergence, // 量能背离
    NewsPositive,     // 新闻利好但未涨 (数据问题)
    IndexDrag,        // 大盘拖累
    Unknown,
}

impl FailureReason {
    pub fn label(self) -> &'static str {
        match self {
            FailureReason::BuyTooLate => "买点过晚",
            FailureReason::SectorFade => "板块退潮",
            FailureReason::NotTradable => "不可成交",
            FailureReason::HumanNotExecuted => "人未执行",
            FailureReason::StopLossHit => "止损触发",
            FailureReason::MacdBearish => "MACD死叉",
            FailureReason::VolumeDivergence => "量能背离",
            FailureReason::NewsPositive => "新闻利好但未涨",
            FailureReason::IndexDrag => "大盘拖累",
            FailureReason::Unknown => "未知",
        }
    }
}

/// 单条失败归因
#[derive(Debug, Clone)]
pub struct FailureItem {
    pub code: String,
    pub name: String,
    pub signal_level: String, // 原始信号级别 (A/B/C)
    pub reason: FailureReason,
    pub pnl_pct: f64,       // 实际盈亏
    pub suggestion: String, // 处理建议
}

/// 周分布统计
#[derive(Debug, Clone, Default)]
pub struct WeeklyDistribution {
    pub distribution: HashMap<FailureReason, usize>,
}

impl WeeklyDistribution {
    pub fn add(&mut self, r: FailureReason) {
        *self.distribution.entry(r).or_insert(0) += 1;
    }
    pub fn to_string_zh(&self) -> String {
        let mut parts: Vec<String> = self
            .distribution
            .iter()
            .map(|(k, v)| format!("{}{}{}", k.label(), v, ""))
            .collect();
        parts.sort();
        parts.join(" ")
    }
}

/// R-06 渲染
pub fn render_r06(items: &[FailureItem], weekly: &WeeklyDistribution) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "❌ 失败归因（{}）\n",
        Local::now().format("%Y-%m-%d")
    ));
    for it in items {
        s.push_str(&format!(
            "{}({}) 原信号: {} {}\n结果: pnl{:+.1}%\n归因: {}\n处理建议: {}\n─────\n",
            it.name,
            it.code,
            it.signal_level,
            it.reason.label(),
            it.pnl_pct,
            it.reason.label(),
            it.suggestion,
        ));
    }
    s.push_str(&format!("本周归因分布: {}\n", weekly.to_string_zh()));
    s.push_str("仅建议, 不自动改规则\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_basic() {
        let items = vec![FailureItem {
            code: "TEST_CODE_600519".into(),
            name: "XX".into(),
            signal_level: "A".into(),
            reason: FailureReason::BuyTooLate,
            pnl_pct: -2.5,
            suggestion: "下次提前 30min 介入".into(),
        }];
        let mut wk = WeeklyDistribution::default();
        wk.add(FailureReason::BuyTooLate);
        wk.add(FailureReason::SectorFade);
        let s = render_r06(&items, &wk);
        assert!(s.contains("❌ 失败归因"));
        assert!(s.contains("买点过晚"));
        assert!(s.contains("本周归因分布"));
        assert!(s.contains("仅建议"));
    }

    #[test]
    fn empty_items() {
        let wk = WeeklyDistribution::default();
        let s = render_r06(&[], &wk);
        assert!(s.contains("本周归因分布:"));
    }

    #[test]
    fn distribution_count() {
        let mut wk = WeeklyDistribution::default();
        wk.add(FailureReason::BuyTooLate);
        wk.add(FailureReason::BuyTooLate);
        wk.add(FailureReason::SectorFade);
        assert_eq!(wk.distribution.get(&FailureReason::BuyTooLate), Some(&2));
        assert_eq!(wk.distribution.get(&FailureReason::SectorFade), Some(&1));
    }
}
