//! v11-P0-4 commit C: 持仓决策台 — 渲染层
//!
//! ## 背景
//!
//! Commit A 落了 `Action` / `Priority` / `FinalDecision` 类型, Commit B 落了 `decide()` 5 层规则.
//! Commit C 把 `FinalDecision` 列表渲染成飞书卡片, 替代 `bin/monitor/main.rs::build_holding_summary` 的
//! `contains("规避")` 字符串猜 (B9 推送, P0-4 文档 §四 Commit C).
//!
//! ## 设计
//!
//! - `format_single_decision(d) -> String`: 单只持仓卡片 (按 P0-4 §3.5 模板)
//! - `format_decision_board(decisions) -> String`: 多只持仓聚合卡片
//! - AI 卡片 (grill Q4 决策): 仅 1-2 句文字附注 (ai_card_summary), 不含 composite_score / 6 维分数
//!   (v11 IC 证伪 sentiment, 数字误导). 完整报告落盘, 用户想查看日志.
//!
//! ## 切换路径 (grill Q3 修订: PUSH_SHADOW)
//!
//! - Commit C 加新路径 (`format_decision_board`), **不替换** B9 调用点
//! - Commit D 加 `PUSH_SHADOW=true` env var, 让决策台和旧推送都推
//! - Commit E shadow 跑 3-5 次 monitor --review, 验证无误后切 default
//! - 切换后, B9 调用点 (main.rs:967) 改调 `format_decision_board`

use crate::decision::decision_panel::{FinalDecision, Priority};

/// 格式化单只持仓决策卡片
///
/// 输出格式 (按 P0-4 §3.5 模板):
/// ```text
/// 🔴 [P0] 立即减仓 XXX(000001)
///    现价¥12.30 | 止损¥12.10 | 今日-3.2%
///    · 硬止损触发 ¥12.10
///    · 单票仓位14.2% > 10%
///    💬 AI参考(不入决策):技术破位
/// ```
pub fn format_single_decision(d: &FinalDecision) -> String {
    let mut out = String::new();
    let action_label = d.action.label();
    let emoji = d.action.emoji();
    let priority = d.priority.label();

    // 标题行: emoji + 优先级 + 动作 + 名称(代码)
    out.push_str(&format!(
        "{} [{}] {} {}({})\n",
        emoji, priority, action_label, d.name, d.code
    ));

    // 缩进: 现价行
    let stop_str = match d.stop_loss {
        Some(s) => format!(" | 止损¥{:.2}", s),
        None => String::new(),
    };
    out.push_str(&format!(
        "   现价¥{:.2}{} | 今日{:+.2}%\n",
        d.current_price, stop_str, d.change_pct
    ));

    // 缩进: reasons
    for r in &d.reasons {
        out.push_str(&format!("   · {}\n", r.text));
    }

    // AI 卡片 (grill Q4: 1-2 句摘要, 无分数)
    if let Some(ai) = &d.ai_card_summary {
        out.push_str(&format!("   💬 AI参考(不入决策): {}\n", ai));
    }

    out
}

/// 格式化多只持仓决策聚合卡片
///
/// 顶部统计 (n P0 / m P1 / k P2), 然后按 P 排序输出每只卡片.
///
/// 输出格式:
/// ```text
/// 🎯 持仓决策台 · 共 7 只 (1 P0 / 5 P1 / 1 P2)
/// 依据:止损+风控+放量+消息面(AI仅附注)
/// ━━━━━━━━━━
/// 🔴 [P0] 立即减仓 XXX(000001)
///    ... 单只卡片 ...
/// 🟡 [P1] 逢高减仓 YYY(000002)
///    ...
/// ```
pub fn format_decision_board(decisions: &[FinalDecision]) -> String {
    let mut out = String::new();

    // 顶部: 统计
    let total = decisions.len();
    let p0 = decisions
        .iter()
        .filter(|d| d.priority == Priority::P0)
        .count();
    let p1 = decisions
        .iter()
        .filter(|d| d.priority == Priority::P1)
        .count();
    let p2 = decisions
        .iter()
        .filter(|d| d.priority == Priority::P2)
        .count();
    out.push_str(&format!(
        "🎯 持仓决策台 · 共 {} 只 ({} P0 / {} P1 / {} P2)\n",
        total, p0, p1, p2
    ));
    out.push_str("依据:止损+风控+放量+消息面(AI仅附注)\n");
    out.push_str("━━━━━━━━━━\n");

    // 按优先级排序 (P0 最严重, P1, P2, Hold)
    let mut sorted: Vec<&FinalDecision> = decisions.iter().collect();
    sorted.sort_by_key(|d| d.priority); // Priority 实现了 Ord, P0 < P1 < P2

    for d in sorted {
        out.push_str(&format_single_decision(d));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::decision_panel::{Action, DecisionReason, DecisionReasonKind, Priority};

    fn make_decision(
        code: &str,
        name: &str,
        action: Action,
        priority: Priority,
        current_price: f64,
        change_pct: f64,
        reasons: Vec<DecisionReason>,
        ai: Option<&str>,
    ) -> FinalDecision {
        let mut d = FinalDecision::new(
            code,
            name,
            current_price,
            change_pct,
            action,
            priority,
            reasons,
        );
        if let Some(a) = ai {
            d = d.with_ai_summary(a);
        }
        d
    }

    /// 单只卡片: 包含 emoji + 优先级 + 动作 + reasons + AI 摘要
    #[test]
    fn single_decision_full() {
        let d = make_decision(
            "000001",
            "平安银行",
            Action::ReduceNow,
            Priority::P0,
            12.30,
            -3.2,
            vec![
                DecisionReason::new(DecisionReasonKind::StopLoss, "硬止损触发 ¥12.10"),
                DecisionReason::new(DecisionReasonKind::PositionLimit, "单票仓位14.2% > 10%"),
            ],
            Some("技术破位"),
        )
        .with_stop_loss(12.10);

        let s = format_single_decision(&d);
        assert!(s.contains("🔴"), "P0 必有 🔴 emoji");
        assert!(s.contains("[P0]"), "P0 标签");
        assert!(s.contains("立即减仓"), "P0 必有 立即减仓");
        assert!(s.contains("平安银行(000001)"), "名称+代码");
        assert!(s.contains("¥12.30"), "现价");
        assert!(s.contains("止损¥12.10"), "止损价");
        assert!(s.contains("-3.20%"), "涨跌幅");
        assert!(s.contains("硬止损触发"), "reason 1");
        assert!(s.contains("单票仓位"), "reason 2");
        assert!(
            s.contains("💬 AI参考(不入决策): 技术破位"),
            "AI 摘要 grill Q4 格式"
        );
    }

    /// 单只卡片: 无 AI 摘要时省略 "💬 AI" 行
    #[test]
    fn single_decision_no_ai_summary() {
        let d = make_decision(
            "000002",
            "万科A",
            Action::Hold,
            Priority::P2,
            8.50,
            0.0,
            vec![DecisionReason::new(
                DecisionReasonKind::Health,
                "无风险信号, 持有观察 (默认)",
            )],
            None,
        );
        let s = format_single_decision(&d);
        assert!(!s.contains("💬"), "无 AI 摘要应省略 💬 行");
        assert!(s.contains("🟢"), "Hold 应有 🟢");
        assert!(s.contains("[P2]"), "Hold 是 P2");
    }

    /// 多只聚合: 顶部统计正确 (P0/P1/P2 计数)
    #[test]
    fn decision_board_stats() {
        let decisions = vec![
            make_decision(
                "000001",
                "P0票",
                Action::ReduceNow,
                Priority::P0,
                10.0,
                0.0,
                vec![],
                None,
            ),
            make_decision(
                "000002",
                "P0票2",
                Action::ReduceNow,
                Priority::P0,
                10.0,
                0.0,
                vec![],
                None,
            ),
            make_decision(
                "000003",
                "P1票",
                Action::Reduce,
                Priority::P1,
                10.0,
                0.0,
                vec![],
                None,
            ),
            make_decision(
                "000004",
                "P2票",
                Action::WatchAdd,
                Priority::P2,
                10.0,
                0.0,
                vec![],
                None,
            ),
            make_decision(
                "000005",
                "Hold票",
                Action::Hold,
                Priority::P2,
                10.0,
                0.0,
                vec![],
                None,
            ),
        ];
        let s = format_decision_board(&decisions);
        assert!(s.contains("共 5 只 (2 P0 / 1 P1 / 2 P2)"), "顶部统计");
        assert!(s.contains("依据:止损+风控+放量+消息面(AI仅附注)"), "依据行");
        assert!(s.contains("━━━━━━━━━━"), "分隔线");
    }

    /// 多只聚合: 按 P 排序 (P0 在前, Hold 在后)
    #[test]
    fn decision_board_sorted_by_priority() {
        let decisions = vec![
            make_decision(
                "000005",
                "Hold票",
                Action::Hold,
                Priority::P2,
                10.0,
                0.0,
                vec![],
                None,
            ),
            make_decision(
                "000001",
                "P0票",
                Action::ReduceNow,
                Priority::P0,
                10.0,
                0.0,
                vec![],
                None,
            ),
            make_decision(
                "000003",
                "P1票",
                Action::Reduce,
                Priority::P1,
                10.0,
                0.0,
                vec![],
                None,
            ),
        ];
        let s = format_decision_board(&decisions);
        // P0 应在 P1 之前
        let p0_pos = s.find("P0票").expect("P0票 should be present");
        let p1_pos = s.find("P1票").expect("P1票 should be present");
        let p2_pos = s.find("Hold票").expect("Hold票 should be present");
        assert!(p0_pos < p1_pos, "P0 在 P1 之前");
        assert!(p1_pos < p2_pos, "P1 在 P2 (Hold) 之前");
    }

    /// 多只聚合: 空列表返回空字符串 (不报错)
    #[test]
    fn decision_board_empty() {
        let s = format_decision_board(&[]);
        // 仍输出顶部统计 (0 只), 但单只卡片为空
        assert!(s.contains("共 0 只"));
        assert!(!s.contains("🔴"));
        assert!(!s.contains("🟡"));
    }
}
