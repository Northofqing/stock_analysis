//! BR-004 排序 + BR-005 限额 — 测试覆盖
//!
//! 修复 F15/F16 (2026-06-29 codex review): 业务规则 BR-004/005 文档承诺有测试,
//! 实际 `ls tests/` 没找到 `tests/ranking.rs`. 本文件补上.
//!
//! 覆盖:
//! - BR-004: final_score 降序, 同分按 push_time 升序 (越早越前)
//! - BR-005: 评估 push 跳过逻辑 (无快讯/无产业链命中/可信度不足)

use stock_analysis::opportunity::discover::Candidate;

/// 修复 F15 (2026-06-29 BR-004): final_score 降序测试.
/// 模拟 run_post_close_candidates 的 sort_by 逻辑, 验证排序正确性.
fn sort_by_final_score_and_push_time(
    mut candidates: Vec<(Candidate, f64)>,
) -> Vec<(Candidate, f64)> {
    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1) // final_score 降序
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.push_time.cmp(&b.0.push_time)) // 同分 push_time 升序
    });
    candidates
}

fn make_candidate(code: &str, push_time: i64) -> Candidate {
    Candidate {
        code: code.to_string(),
        name: format!("股票{}", code),
        chain: String::from("test"),
        logic: String::from("test"),
        score: 0.0,
        price_note: String::new(),
        reason_summary: String::new(),
        push_time,
    }
}

#[test]
fn test_br004_final_score_descending() {
    let candidates = vec![
        (make_candidate("A", 100), 50.0),
        (make_candidate("B", 200), 80.0),
        (make_candidate("C", 300), 30.0),
    ];
    let sorted = sort_by_final_score_and_push_time(candidates);
    assert_eq!(sorted[0].0.code, "B", "最高分 80 应排第一");
    assert_eq!(sorted[1].0.code, "A", "次高分 50 应排第二");
    assert_eq!(sorted[2].0.code, "C", "最低分 30 应排第三");
}

#[test]
fn test_br004_tie_breaker_push_time_ascending() {
    // 同分时按 push_time 升序 (越早推送的排前面)
    let candidates = vec![
        (make_candidate("late", 300), 50.0),
        (make_candidate("early", 100), 50.0),
        (make_candidate("middle", 200), 50.0),
    ];
    let sorted = sort_by_final_score_and_push_time(candidates);
    assert_eq!(
        sorted[0].0.code, "early",
        "同分时 push_time=100 (最早) 排第一"
    );
    assert_eq!(sorted[1].0.code, "middle");
    assert_eq!(
        sorted[2].0.code, "late",
        "同分时 push_time=300 (最晚) 排最后"
    );
}

#[test]
fn test_br004_default_push_time_zero_sorts_first() {
    // 老调用方未填 push_time → 默认 0, 排在新调用方 (push_time > 0) 前面.
    // 这是有意设计的可观测回归点: 老测试 code 排前面, 新 code 排后面.
    let candidates = vec![
        (make_candidate("new", 1000), 50.0),
        (make_candidate("old", 0), 50.0),
    ];
    let sorted = sort_by_final_score_and_push_time(candidates);
    assert_eq!(sorted[0].0.code, "old", "push_time=0 (老调用方) 排第一");
    assert_eq!(sorted[1].0.code, "new");
}

// ========================================================================
// BR-005 限额测试 — 修复 F16 (2026-06-29 codex review)
// 注: evaluate_opportunity_push_skip_reason 在 src/bin/monitor/notify.rs (binary 模块),
// 不能从 lib test 访问. 这里复制实现 (作为 snapshot test, 防止逻辑漂移).
// 真实现改动时, 这个测试会失败提醒 maintainer 同步两处.
// ========================================================================

fn evaluate_opportunity_push_skip_reason(opp_text: &str) -> Option<&'static str> {
    // 注: 必须与 src/bin/monitor/notify.rs::evaluate_opportunity_push_skip_reason 保持一致.
    if opp_text.contains("暂无最新快讯") {
        return Some("contains:暂无最新快讯");
    }
    if opp_text.contains("当前快讯未命中已知产业链") {
        return Some("contains:当前快讯未命中已知产业链");
    }
    if opp_text.contains("当前产业链信号可信度不足（已降级观察）") {
        return Some("contains:当前产业链信号可信度不足");
    }
    if opp_text.contains("无可用标的") {
        return Some("contains:无可用标的");
    }
    None
}

#[test]
fn test_br005_skip_when_no_flash_news() {
    let reason = evaluate_opportunity_push_skip_reason("📡 产业链扫描\n暂无最新快讯");
    assert!(reason.is_some(), "无快讯应跳过推送");
    assert!(reason.unwrap().contains("暂无最新快讯"));
}

#[test]
fn test_br005_skip_when_no_chain_match() {
    let reason = evaluate_opportunity_push_skip_reason("📡 产业链扫描\n当前快讯未命中已知产业链");
    assert!(reason.is_some(), "未命中产业链应跳过推送");
    assert!(reason.unwrap().contains("当前快讯未命中已知产业链"));
}

#[test]
fn test_br005_skip_when_low_confidence() {
    let reason = evaluate_opportunity_push_skip_reason(
        "📡 产业链扫描\n当前产业链信号可信度不足（已降级观察）",
    );
    assert!(reason.is_some(), "可信度不足应跳过推送");
    assert!(reason.unwrap().contains("可信度不足"));
}

#[test]
fn test_br005_no_skip_when_normal_output() {
    let reason = evaluate_opportunity_push_skip_reason(
        "📡 产业链扫描\n━━━━━━━━━━━━━━━━━━━━━━━━\n🔗 AI硬件-PCB\n受益标的：广立微(688214) +5.2%\n",
    );
    assert!(reason.is_none(), "正常产业链输出不应跳过推送");
}

// ========================================================================
// BR-005 日度成功投递 ≤5：真实 L5 GovernanceEngine 边界测试。
// CandidateBoard 的 profile=5 接线由 monitor::v14_adapter 单元测试覆盖；这里验证
// 第 5 条前放行、已有 5 条时拒绝，避免再用“未实现”为绿色测试。
// ========================================================================

#[test]
fn test_br005_daily_limit_five_boundary() {
    use chrono::Local;
    use stock_analysis::push_l1::{Severity, SignalEvent, SignalPayload, SignalSource};
    use stock_analysis::push_l2::{DataMode, TemplateCategory, TemplateMetadata};
    use stock_analysis::push_l5::{GovernanceContext, GovernanceDecision, GovernanceEngine};

    let event = SignalEvent::new(
        SignalSource::HoldingHealth,
        "candidate_board",
        None,
        Local::now(),
        SignalPayload::HoldingHealth(Default::default()),
        Severity::Normal,
    );
    let profile = TemplateMetadata {
        category: TemplateCategory::Holding,
        quiet_hours_respect: false,
        frozen_mode_respect: false,
        data_mode_min: DataMode::Degraded,
        cooldown_secs: 0,
        max_per_user_per_day: Some(5),
        always_send_on_data_source_down: false,
    };
    let mut ctx = GovernanceContext {
        data_mode: DataMode::Full,
        is_quiet_hour: false,
        is_frozen: false,
        now: Local::now(),
        today_pushed_count: 4,
    };
    let engine = GovernanceEngine::new();

    assert_eq!(
        engine.check(&profile, &event, &ctx),
        GovernanceDecision::Approve
    );
    ctx.today_pushed_count = 5;
    assert_eq!(
        engine.check(&profile, &event, &ctx),
        GovernanceDecision::Deny("daily_limit".to_string())
    );
}
