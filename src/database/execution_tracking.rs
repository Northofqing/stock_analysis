//! v12 MVP-5 §8.1: 执行率统计 (建议推送 vs 实际执行).
//!
//! 设计: execution_tracking 消费, 按周输出. HumanNotExecuted 归因.
//! v12.2 §8.1 设计: source ∈ {manual_confirm, position_adjustment, paper_trade}.

use chrono::Local;
use std::collections::HashMap;

/// 单条建议执行追踪
#[derive(Debug, Clone)]
pub struct ExecutionRecord {
    pub date: NaiveDateLite,           // 简化版日期 (避免 chrono 依赖)
    pub code: String,
    pub push_kind: String,              // e.g. HoldingPlan/T0Advice/CandidateTriggered
    pub executed: bool,                // 是否实际执行
    pub execution_source: Option<String>, // manual_confirm / paper_trade / None
    pub pnl_pct: Option<f64>,          // 执行后盈亏 (%)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NaiveDateLite {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl NaiveDateLite {
    pub fn today() -> Self {
        let now = Local::now();
        Self { year: now.format("%Y").to_string().parse().unwrap_or(2026), month: now.format("%m").to_string().parse().unwrap_or(7), day: now.format("%d").to_string().parse().unwrap_or(5) }
    }
    pub fn to_string_zh(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// 执行率统计
#[derive(Debug, Clone, Default)]
pub struct ExecutionStats {
    pub pushed: u32,
    pub executed: u32,
    pub by_kind: HashMap<String, (u32, u32)>,   // (pushed, executed)
    pub not_executed_reasons: HashMap<String, u32>, // HumanNotExecuted 等
}

impl ExecutionStats {
    pub fn add(&mut self, r: &ExecutionRecord) {
        self.pushed += 1;
        if r.executed {
            self.executed += 1;
        }
        let entry = self.by_kind.entry(r.push_kind.clone()).or_insert((0, 0));
        entry.0 += 1;
        if r.executed {
            entry.1 += 1;
        }
        if !r.executed {
            *self.not_executed_reasons
                .entry(r.execution_source.clone().unwrap_or_else(|| "HumanNotExecuted".into()))
                .or_insert(0) += 1;
        }
    }

    pub fn execution_rate(&self) -> f64 {
        if self.pushed == 0 { 0.0 } else { self.executed as f64 / self.pushed as f64 }
    }
}

/// 周报告渲染
pub fn render_weekly(stats: &ExecutionStats) -> String {
    let mut s = String::new();
    s.push_str(&format!("📊 周执行率报告（{}）\n", Local::now().format("%Y-%m-%d")));
    s.push_str(&format!(
        "本周推送: {}条, 执行: {}条 ({:.1}%)\n",
        stats.pushed, stats.executed, stats.execution_rate() * 100.0,
    ));
    if !stats.by_kind.is_empty() {
        s.push_str("\n按 PushKind:\n");
        for (kind, (p, e)) in &stats.by_kind {
            let rate = if *p > 0 { *e as f64 / *p as f64 * 100.0 } else { 0.0 };
            s.push_str(&format!("  {}: 推{}条 执{}条 ({:.1}%)\n", kind, p, e, rate));
        }
    }
    if !stats.not_executed_reasons.is_empty() {
        s.push_str("\n未执行归因:\n");
        for (reason, count) in &stats.not_executed_reasons {
            s.push_str(&format!("  {}: {}次\n", reason, count));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_record(executed: bool) -> ExecutionRecord {
        ExecutionRecord {
            date: NaiveDateLite::today(),
            code: "600519".into(),
            push_kind: "HoldingPlan".into(),
            executed,
            execution_source: if executed { Some("manual_confirm".into()) } else { None },
            pnl_pct: if executed { Some(1.5) } else { None },
        }
    }

    #[test]
    fn execution_rate_basic() {
        let mut s = ExecutionStats::default();
        s.add(&mock_record(true));
        s.add(&mock_record(true));
        s.add(&mock_record(false));
        s.add(&mock_record(false));
        assert_eq!(s.pushed, 4);
        assert_eq!(s.executed, 2);
        assert!((s.execution_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn render_weekly_basic() {
        let mut s = ExecutionStats::default();
        s.add(&mock_record(true));
        s.add(&mock_record(false));
        let r = render_weekly(&s);
        assert!(r.contains("周执行率报告"));
        assert!(r.contains("推2条"));
        assert!(r.contains("HoldingPlan"));
        assert!(r.contains("HumanNotExecuted"));
    }

    #[test]
    fn empty_stats() {
        let s = ExecutionStats::default();
        let r = render_weekly(&s);
        assert!(r.contains("0条"));
    }

    #[test]
    fn naive_date_format() {
        let d = NaiveDateLite { year: 2026, month: 7, day: 5 };
        assert_eq!(d.to_string_zh(), "2026-07-05");
    }
}