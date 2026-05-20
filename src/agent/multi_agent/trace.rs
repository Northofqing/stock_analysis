//! Agent 流水线追踪日志：通过 `AI_AGENT_TRACE=true` 打印每个 Agent 的思考过程与结果。
//!
//! 使用：
//!   - 设置 `AI_AGENT_TRACE=true` 启用完整追踪（切片/分析师 JSON/辩论全文/终稿）
//!   - 默认（false）只通过 info!() 打印分析师评分概要 + 仲裁综合分

use log::info;

use super::analysts::AnalystView;
use super::debate::DebateOutput;
use super::slices::DomainSlices;
use crate::analyzer::GeminiAnalyzer;

pub(super) fn trace_enabled(analyzer: &GeminiAnalyzer) -> bool {
    analyzer.config.agent_trace
}

fn banner(title: &str) {
    info!("\n{}\n[Agent追踪] {}\n{}", "=".repeat(60), title, "=".repeat(60));
}

/// 打印各领域数据切片（仅 trace 模式）
pub(super) fn print_slices(s: &DomainSlices) {
    banner("📊 数据切片（每个 Agent 看到的输入）");
    info!("\n--- basics ---\n{}", s.basics);
    info!("\n--- technical ---\n{}", s.technical);
    info!("\n--- capital ---\n{}", s.capital);
    info!("\n--- fundamental ---\n{}", s.fundamental);
    info!("\n--- sector ---\n{}", s.sector);
    if let Some(n) = &s.news {
        info!("\n--- news ---\n{}", truncate(n, 1500));
    }
    if let Some(m) = &s.macro_ctx {
        info!("\n--- macro ---\n{}", truncate(m, 1500));
    }
}

/// 打印单个分析师结果（不论 trace 都打印精简版；trace 时再打印完整 signals）
pub(super) fn print_analyst(analyzer: &GeminiAnalyzer, role: &str, v: &AnalystView) {
    let signals = if v.key_signals.is_empty() {
        "（无）".to_string()
    } else {
        v.key_signals.join(" | ")
    };
    info!(
        "[Agent结果] {} score={} stance={} confidence={} | signals={} | summary={}",
        role,
        v.score,
        nz(&v.stance, "neutral"),
        nz(&v.confidence, "low"),
        truncate(&signals, 300),
        truncate(&v.summary, 200),
    );
    if trace_enabled(analyzer) {
        info!("[Agent追踪] {} 完整视角：\n{}", role, v.to_markdown(role));
    }
}

/// 打印多空辩论与风控结果
pub(super) fn print_debate(analyzer: &GeminiAnalyzer, d: &DebateOutput) {
    if trace_enabled(analyzer) {
        banner("🐂 多头研究员最终论点");
        info!("\n{}", if d.bull.is_empty() { "（无）" } else { d.bull.as_str() });
        banner("🐻 空头研究员最终论点");
        info!("\n{}", if d.bear.is_empty() { "（无）" } else { d.bear.as_str() });
        banner("🛡️ 风控官清单");
        info!("\n{}", if d.risk.is_empty() { "（无）" } else { d.risk.as_str() });
    } else {
        info!(
            "[Agent结果] 多空辩论 — 多头长度={} 空头长度={} 风控长度={}",
            d.bull.len(),
            d.bear.len(),
            d.risk.len()
        );
    }
}

/// 打印仲裁终稿（trace 模式打全文，否则只打前 600 字预览）
pub(super) fn print_final(analyzer: &GeminiAnalyzer, composite: i32, md: &str) {
    if trace_enabled(analyzer) {
        banner(&format!("🏛️ 仲裁终稿（composite_score={}）", composite));
        info!("\n{}", md);
    } else {
        info!(
            "[Agent结果] 仲裁终稿预览（composite={}, 共 {} 字）：\n{}",
            composite,
            md.chars().count(),
            truncate(md, 600)
        );
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{}…(已截断 {} 字)", head, count - max_chars)
    }
}

fn nz<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    if s.trim().is_empty() {
        fallback
    } else {
        s
    }
}
