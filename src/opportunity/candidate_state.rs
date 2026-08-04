//! v12 PR3-3.4: 候选实时 promotion gate.
//!
//! BR-191: 旧本地影子排序、状态机和审计 JSONL 已退役。本模块只保留
//! push consumer 仍使用的人工开关与可审计性能证据门，不声明生产候选来源。

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

/// 候选转正的可审计性能证据。
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PromotionEvidence {
    pub sample_count: u32,
    pub win_rate_strong: f64,
    pub win_rate_weak: f64,
}

/// BR-100: 人工开关和分层样本门必须同时通过。
///
/// `None` 表示尚未从持久化样本库取得证据，必须显式拒绝，不得把环境变量
/// 当成胜率证明。
pub fn require_live_promotion(
    evidence: Option<PromotionEvidence>,
    explicit_override: Option<bool>,
) -> Result<PromotionEvidence, String> {
    if !is_candidate_live_enabled(explicit_override) {
        return Err("候选人工转正开关未开启".to_string());
    }
    let evidence = evidence.ok_or_else(|| "候选分层样本证据源未接入".to_string())?;
    if !evidence.win_rate_strong.is_finite()
        || !evidence.win_rate_weak.is_finite()
        || !(0.0..=1.0).contains(&evidence.win_rate_strong)
        || !(0.0..=1.0).contains(&evidence.win_rate_weak)
    {
        return Err(format!(
            "候选分层胜率证据非法: strong={} weak={}",
            evidence.win_rate_strong, evidence.win_rate_weak
        ));
    }
    if !should_promote_to_live(
        evidence.sample_count,
        evidence.win_rate_strong,
        evidence.win_rate_weak,
    ) {
        return Err(format!(
            "候选分层样本门未通过: samples={} strong={:.3} weak={:.3}",
            evidence.sample_count, evidence.win_rate_strong, evidence.win_rate_weak
        ));
    }
    Ok(evidence)
}

/// MVP3-3.1: 人工转正开关.
///
/// 三种开启方式:
///   1. env `ENABLE_CANDIDATE_LIVE=true` → 全局启用
///   2. 调用方传 `explicit_override = Some(true)` → 显式覆盖 (供 PR4 主循环按节奏启用)
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
    // 当前仅依赖 env/调用方覆盖；没有 TOML 输入。保守默认 false。
    false
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn manual_switch_cannot_bypass_missing_promotion_evidence() {
        let error = require_live_promotion(None, Some(true)).expect_err("missing evidence blocks");
        assert!(error.contains("证据源未接入"));
    }

    #[test]
    fn live_promotion_requires_switch_and_valid_thresholds() {
        let evidence = PromotionEvidence {
            sample_count: 30,
            win_rate_strong: 0.30,
            win_rate_weak: 0.50,
        };
        assert!(require_live_promotion(Some(evidence), Some(false)).is_err());
        assert_eq!(
            require_live_promotion(Some(evidence), Some(true)).expect("promotion passes"),
            evidence
        );
        assert!(require_live_promotion(
            Some(PromotionEvidence {
                win_rate_strong: f64::NAN,
                ..evidence
            }),
            Some(true)
        )
        .is_err());
    }

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
}
