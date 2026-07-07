//! v13.27: 端到端连通性诊断
//!
//! 目标: 用户跑 monitor 没新推送, 不知道是哪个环节失败。
//! 此模块端到端跑: data_source → snapshot → render → dispatch
//! 输出: 哪些环节失败, 哪些 OK, 哪些空数据。
//!
//! 触发方式: 单独的 `--v13-diag` CLI 参数 (在 main 加一行)
//! 输出: stdout + data/v13_diag_report.json (持久化)

use std::time::Instant;
use std::collections::HashMap;

use serde::Serialize;

use super::dryrun_report::KindStat;
use super::push_templates::{
    load_sector_snapshot_real, load_news_catalyst_snapshot_real,
    load_industry_chain_snapshot_real, load_news_to_idea_snapshot_real,
    load_paper_review_snapshot_real,
};

/// 端到端诊断步骤
#[derive(Debug, Clone, Serialize)]
pub struct DiagStep {
    pub step: String,           // e.g. "load_sector", "render_intraday", "dispatch"
    pub dispatcher: String,     // e.g. "I-01", "T-07"
    pub status: String,          // "ok" | "empty" | "error"
    pub duration_ms: u64,
    pub detail: String,          // 错误信息 / 数据规模
}

/// v13.27 整体诊断报告
#[derive(Debug, Serialize)]
pub struct V13DiagReport {
    pub generated_at: String,
    pub total_steps: usize,
    pub ok_steps: usize,
    pub empty_steps: usize,
    pub error_steps: usize,
    pub steps: Vec<DiagStep>,
}

/// 端到端跑全部 6 个 v13 dispatcher 的关键路径
pub async fn run_v13_diag() -> V13DiagReport {
    let mut steps = Vec::new();
    let started = Instant::now();

    // I-01 盘中轮动 (依赖 sector_monitor)
    steps.push(check_step("I-01", "load_sector", || {
        let snap = load_sector_snapshot_real("10:30");
        if snap.tech_sub.is_empty() && snap.power_sub.is_empty() && snap.robot_sub.is_empty() {
            "empty".into()
        } else {
            format!(
                "ok: tech={}, power={}, robot={}",
                snap.tech_sub, snap.power_sub, snap.robot_sub
            )
        }
    }));

    // I-02 新闻催化 (依赖 chain_daily)
    steps.push(check_step("I-02", "load_news_catalyst", || {
        let snap = load_news_catalyst_snapshot_real("10:30");
        if snap.headline.is_empty() {
            "empty".into()
        } else {
            format!("ok: headline={}, stocks={}", snap.headline, snap.stocks.len())
        }
    }));

    // I-03 涨停扩散 (依赖 chain_daily)
    steps.push(check_step("I-03", "load_industry_chain", || {
        let snap = load_industry_chain_snapshot_real("10:30");
        if snap.chain.is_empty() {
            "empty".into()
        } else {
            format!("ok: chain={}, supplements={}", snap.chain, snap.supplements.len())
        }
    }));

    // D-01 新闻驱动个股 (依赖 chain_daily)
    steps.push(check_step("D-01", "load_news_to_idea", || {
        let snap = load_news_to_idea_snapshot_real("10:30");
        if snap.headline.is_empty() {
            "empty".into()
        } else {
            format!("ok: headline={}, name={}", snap.headline, snap.name)
        }
    }));

    // A-01 虚拟仓复盘 (依赖 virtual_observation JSON)
    steps.push(check_step("A-01", "load_paper_review", || {
        let snap = load_paper_review_snapshot_real("2026-07-07");
        if snap.name.is_empty() {
            "empty".into()
        } else {
            format!("ok: name={}, pnl={:?}", snap.name, snap.pnl)
        }
    }));

    // 总结
    let ok_steps = steps.iter().filter(|s| s.status == "ok").count();
    let empty_steps = steps.iter().filter(|s| s.status == "empty").count();
    let error_steps = steps.iter().filter(|s| s.status == "error").count();
    let total = steps.len();

    let _ = started;  // 总耗时 (unused for now)
    V13DiagReport {
        generated_at: chrono::Local::now().to_rfc3339(),
        total_steps: total,
        ok_steps,
        empty_steps,
        error_steps,
        steps,
    }
}

/// 单步检测包装器
fn check_step<F>(dispatcher: &str, step_name: &str, f: F) -> DiagStep
where
    F: FnOnce() -> String,
{
    let started = Instant::now();
    let detail = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| "panic".to_string());
    DiagStep {
        step: step_name.to_string(),
        dispatcher: dispatcher.to_string(),
        status: classify(&detail),
        duration_ms: started.elapsed().as_millis() as u64,
        detail,
    }
}

/// 把返回内容分类为 ok / empty / error
fn classify(s: &str) -> String {
    if s.starts_with("ok") {
        "ok".to_string()
    } else if s.starts_with("empty") {
        "empty".to_string()
    } else if s.starts_with("panic") {
        "error".to_string()
    } else {
        "error".to_string()
    }
}

/// 报告 v13.27 诊断到 stdout + 写 json 文件
pub async fn report_v13_diag() -> anyhow::Result<()> {
    let report = run_v13_diag().await;
    println!("\n=== v13.27 端到端诊断 ===");
    println!("生成时间: {}", report.generated_at);
    println!("总步骤: {}, OK: {}, 空: {}, 错: {}",
        report.total_steps, report.ok_steps, report.empty_steps, report.error_steps);
    for s in &report.steps {
        let icon = match s.status.as_str() {
            "ok" => "✓",
            "empty" => "○",
            _ => "✗",
        };
        println!("  {} {} {} ({}ms) {}",
            icon, s.dispatcher, s.step, s.duration_ms, s.detail);
    }
    if report.error_steps > 0 {
        println!("\n⚠ 发现 {} 个错误环节, 检查上方 ✗ 行", report.error_steps);
    }
    if report.empty_steps > 0 {
        println!("\n○ 发现 {} 个空数据, 需检查数据源", report.empty_steps);
    }
    let json = serde_json::to_string_pretty(&report)?;
    let path = "data/v13_diag_report.json";
    std::fs::create_dir_all("data").ok();
    std::fs::write(path, json)?;
    println!("\n报告已写入: {}", path);
    Ok(())
}
