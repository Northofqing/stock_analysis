//! v12 PR3-3.4: 影子候选状态机.
//!
//! 设计: candidate 整个生命周期在 Shadow 状态, **零推送**. 只有 PR4 人工开关接通才转正.
//!
//! 数据来源: news_ranker.rs:1062 (shadow_rank_hits) + audit JSONL.
//! 每条候选落审计含账户/数据模式快照 (PR3-3.4 DoD 标准).
//!
//! 状态机 (v12 §12 简化):
//!   Shadow(命中) --(人工开关)--> Watch/Armed/Triggered
//!   Triggered --(T+1 验证)--> Verified/Win/Loss

use chrono::Local;
use serde::{Deserialize, Serialize};

/// 候选状态
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    /// 影子期 (默认, 零推送)
    Shadow,
    /// 观察池 (PR4 人工接入)
    Watch,
    /// 待触发
    Armed,
    /// 已触发 (PushKind::CandidateTriggered, ⚡ 1次/票/日)
    Triggered,
    /// T+1 验证: 命中
    Verified,
    /// T+1 验证: 胜
    Win,
    /// T+1 验证: 负
    Loss,
}

impl CandidateState {
    pub fn label(self) -> &'static str {
        match self {
            CandidateState::Shadow => "Shadow",
            CandidateState::Watch => "Watch",
            CandidateState::Armed => "Armed",
            CandidateState::Triggered => "Triggered",
            CandidateState::Verified => "Verified",
            CandidateState::Win => "Win",
            CandidateState::Loss => "Loss",
        }
    }

    /// PR3-3.4 核心: 是否禁用推送
    pub fn push_disabled(self) -> bool {
        // Shadow 期永远零推送 (默认)
        matches!(self, CandidateState::Shadow)
    }
}

/// 候选记录 (用于 news_audit JSONL)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateRecord {
    pub ts: String,
    pub code: String,
    pub name: String,
    pub state: CandidateState,
    pub rank_hits: u32,
    pub virtual_reason: String,
    pub account_mode: String,
    pub data_mode: String,
    pub evidence_quality: String, // Strong/Mid/Weak/Missing
}

/// v12 §14.3 PR3-3.4: 是否允许转正 (人工开关 + 样本门槛)
///
/// 当前 PR3 实现: 永远 false (影子期零推送).
/// PR4 接入: 检查 sample_threshold + EvidenceQuality 分层胜率.
pub fn should_promote_to_live(sample_count: u32, win_rate_strong: f64, win_rate_weak: f64) -> bool {
    // v12 §15.2 门槛: 样本 ≥ 30 且 EvidenceQuality 分层胜率完整
    if sample_count < 30 {
        return false;
    }
    // Strong 胜率 ≥ 30% 且 Weak 胜率有数据 (≠ 0/0)
    win_rate_strong >= 0.30 && (win_rate_weak > 0.0 || sample_count >= 100)
}

/// MVP3-3.1: 人工转正开关.
///
/// 三种开启方式:
///   1. env `ENABLE_CANDIDATE_LIVE=true` → 全局启用
///   2. config/opportunity.toml `[candidate_live] enabled = true` → 配置启用 (留待后续 PR)
///   3. 调用方传 `explicit_override = Some(true)` → 显式覆盖 (供 PR4 主循环按节奏启用)
///
/// 默认 false (影子期零推送), 避免误开启后大规模推 T-07.
pub fn is_candidate_live_enabled(explicit_override: Option<bool>) -> bool {
    if let Some(v) = explicit_override {
        return v;
    }
    if let Ok(s) = std::env::var("ENABLE_CANDIDATE_LIVE") {
        if s.eq_ignore_ascii_case("true") || s == "1" {
            return true;
        }
    }
    // config 读取: MVP3-3.1 当前依赖 env (config 接入留待后续 PR)
    // 保守默认 false, 即使 toml 缺失也不开启
    false
}

/// MVP3-3.1: 候选状态机跃迁.
///
/// 状态变更顺序: Shadow → Watch → Armed → Triggered → Verified/Win/Loss.
/// 跃迁条件:
///   - Shadow → Watch: `is_candidate_live_enabled()` && 样本 ≥ 10 (试运行)
///   - Watch → Armed: 样本 ≥ 20
///   - Armed → Triggered: 触发条件命中 (由调用方传入)
///   - Triggered → Win/Loss: T+1 验证 (PR4)
pub fn next_state(
    current: CandidateState,
    sample_count: u32,
    triggered: bool,
    live_enabled: Option<bool>,
) -> CandidateState {
    if !is_candidate_live_enabled(live_enabled) {
        return current; // 关闭时状态不变
    }
    match (current, sample_count, triggered) {
        (CandidateState::Shadow, n, _) if n >= 10 => CandidateState::Watch,
        (CandidateState::Watch, n, _) if n >= 20 => CandidateState::Armed,
        (CandidateState::Armed, _, true) => CandidateState::Triggered,
        (CandidateState::Triggered, _, _) => CandidateState::Triggered, // 需 PR4 验证再变 Win/Loss
        (other, _, _) => other,
    }
}

/// Shadow 候选审计: 写入本地 JSONL (news_audit path).
///
/// 落盘路径: data/news_audit/{date}.jsonl
pub fn write_audit_jsonl(record: &CandidateRecord) -> Result<(), String> {
    use std::io::Write;
    let dir = std::path::PathBuf::from("./data/news_audit");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir news_audit: {}", e))?;
    let fname = format!("{}.jsonl", Local::now().format("%Y-%m-%d"));
    let path = dir.join(&fname);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open {:?}: {}", path, e))?;
    let line = serde_json::to_string(record).map_err(|e| format!("serialize: {}", e))?;
    writeln!(f, "{}", line).map_err(|e| format!("write: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_disables_push() {
        assert!(CandidateState::Shadow.push_disabled());
        assert!(!CandidateState::Triggered.push_disabled());
        assert!(!CandidateState::Armed.push_disabled());
        assert!(!CandidateState::Watch.push_disabled());
    }

    #[test]
    fn state_labels() {
        assert_eq!(CandidateState::Shadow.label(), "Shadow");
        assert_eq!(CandidateState::Triggered.label(), "Triggered");
    }

    #[test]
    fn promote_below_threshold_blocked() {
        assert!(!should_promote_to_live(0, 0.5, 0.5));
        assert!(!should_promote_to_live(29, 0.5, 0.5));
        assert!(
            !should_promote_to_live(30, 0.29, 0.5),
            "Strong 胜率 < 30% 不转正"
        );
    }

    #[test]
    fn promote_with_sufficient_samples_and_winrate() {
        assert!(should_promote_to_live(30, 0.30, 0.5));
        assert!(should_promote_to_live(100, 0.40, 0.3));
    }

    #[test]
    fn candidate_record_serializes() {
        let r = CandidateRecord {
            ts: "2026-07-05T10:00:00".to_string(),
            code: "688001".to_string(),
            name: "测试".to_string(),
            state: CandidateState::Shadow,
            rank_hits: 5,
            virtual_reason: "NewsCatalyst".to_string(),
            account_mode: "Normal".to_string(),
            data_mode: "Full".to_string(),
            evidence_quality: "Strong".to_string(),
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"state\":\"shadow\""));
        assert!(s.contains("\"code\":\"688001\""));
    }

    // ---- MVP3-3.1 人工开关 ----

    #[test]
    fn live_disabled_by_default() {
        std::env::remove_var("ENABLE_CANDIDATE_LIVE");
        assert!(!is_candidate_live_enabled(None));
    }

    #[test]
    fn live_enabled_via_override() {
        assert!(is_candidate_live_enabled(Some(true)));
        assert!(!is_candidate_live_enabled(Some(false)));
    }

    #[test]
    fn live_enabled_via_env() {
        std::env::set_var("ENABLE_CANDIDATE_LIVE", "true");
        assert!(is_candidate_live_enabled(None));
        std::env::remove_var("ENABLE_CANDIDATE_LIVE");

        std::env::set_var("ENABLE_CANDIDATE_LIVE", "1");
        assert!(is_candidate_live_enabled(None));
        std::env::remove_var("ENABLE_CANDIDATE_LIVE");
    }

    #[test]
    fn live_env_other_value_disables() {
        std::env::set_var("ENABLE_CANDIDATE_LIVE", "false");
        assert!(!is_candidate_live_enabled(None));
        std::env::remove_var("ENABLE_CANDIDATE_LIVE");
    }

    // ---- MVP3-3.1 状态机跃迁 ----

    #[test]
    fn state_machine_disabled_when_live_off() {
        // 关闭时, 任何样本数 + triggered 都跳变不到下一态
        assert_eq!(
            next_state(CandidateState::Shadow, 100, true, Some(false)),
            CandidateState::Shadow
        );
        assert_eq!(
            next_state(CandidateState::Armed, 100, true, Some(false)),
            CandidateState::Armed
        );
    }

    #[test]
    fn state_machine_shadow_to_watch() {
        assert_eq!(
            next_state(CandidateState::Shadow, 10, false, Some(true)),
            CandidateState::Watch
        );
        assert_eq!(
            next_state(CandidateState::Shadow, 9, false, Some(true)),
            CandidateState::Shadow
        );
    }

    #[test]
    fn state_machine_watch_to_armed() {
        assert_eq!(
            next_state(CandidateState::Watch, 20, false, Some(true)),
            CandidateState::Armed
        );
        assert_eq!(
            next_state(CandidateState::Watch, 19, false, Some(true)),
            CandidateState::Watch
        );
    }

    #[test]
    fn state_machine_armed_to_triggered() {
        assert_eq!(
            next_state(CandidateState::Armed, 25, true, Some(true)),
            CandidateState::Triggered
        );
        assert_eq!(
            next_state(CandidateState::Armed, 25, false, Some(true)),
            CandidateState::Armed
        );
    }
}
