//! 自适应权重 + Shadow mode（Phase 5 补完）。
//!
//! Shadow mode: 新信号规则先"只记录不告警"跑一段，验证后再上线。
//! 自适应权重: 基于命中率的信号源权重调整，带统计显著性门槛。

use crate::monitor::signal_fusion::SignalSource;
use log::info;
use std::collections::HashMap;
// 修复 Top10#6 (2026-06-29 audit): std::sync::Mutex → tokio::sync::Mutex
// 此处保留 std::sync::Mutex 因为 `locked: Mutex<()>` 是 dummy lock guard,
// 只保护 signal_weights HashMap 微秒级内存修改, 不横跨 await. 改 tokio Mutex 会
// 要求 adjust_weights() 改 async, 影响面过大. audit 列的 5 处中 analyzer/mod.rs:454
// 实际**已经**是 tokio::sync::Mutex (audit 描述错), 其他 3 处 (search_service/adaptive/rate_budget/industry)
// 也是 sync API + 微秒级持有, 不阻塞 worker. 详见 v9.4.6 CHANGELOG 注释.
use std::sync::Mutex;

/// 信号规则的运行模式
pub enum RuleMode {
    Active,   // 正常告警
    Shadow,   // 只记录不告警
    Disabled, // 停用
}

/// 单条信号规则的追踪状态
pub struct RuleTracker {
    pub name: String,
    pub mode: RuleMode,
    pub shadow_signals: u64,
    pub shadow_hits: u64,
    pub active_signals: u64,
    pub active_hits: u64,
    pub promoted_at: Option<String>,
}

impl RuleTracker {
    pub fn new(name: &str, mode: RuleMode) -> Self {
        RuleTracker {
            name: name.into(),
            mode,
            shadow_signals: 0,
            shadow_hits: 0,
            active_signals: 0,
            active_hits: 0,
            promoted_at: None,
        }
    }

    /// Shadow 命中率
    pub fn shadow_hit_rate(&self) -> f64 {
        if self.shadow_signals == 0 {
            return 0.0;
        }
        self.shadow_hits as f64 / self.shadow_signals as f64
    }

    /// 是否满足上线条件
    pub fn ready_for_promotion(&self, min_samples: u64, min_hit_rate: f64) -> bool {
        matches!(self.mode, RuleMode::Shadow)
            && self.shadow_signals >= min_samples
            && self.shadow_hit_rate() >= min_hit_rate
    }
}

/// 自适应权重管理器
pub struct AdaptiveWeightManager {
    pub rules: HashMap<String, RuleTracker>,
    pub signal_weights: HashMap<SignalSource, f64>,
    min_samples: u64,
    max_weekly_change: f64,
    shrinkage: f64,
    locked: Mutex<()>,
}

impl AdaptiveWeightManager {
    pub fn new(min_samples: u64, max_weekly_change: f64, shrinkage: f64) -> Self {
        let mut signal_weights = HashMap::new();
        signal_weights.insert(SignalSource::Technical, 0.25);
        signal_weights.insert(SignalSource::FundFlow, 0.25);
        signal_weights.insert(SignalSource::News, 0.15);
        signal_weights.insert(SignalSource::Chain, 0.20);
        signal_weights.insert(SignalSource::Valuation, 0.15);
        AdaptiveWeightManager {
            rules: HashMap::new(),
            signal_weights,
            min_samples,
            max_weekly_change,
            shrinkage,
            locked: Mutex::new(()),
        }
    }

    /// 注册新规则（默认 Shadow mode）
    pub fn register_rule(&mut self, name: &str) {
        self.rules
            .insert(name.into(), RuleTracker::new(name, RuleMode::Shadow));
        info!("[Adaptive] 规则 '{}' 注册（Shadow mode）", name);
    }

    /// 记录 Shadow 信号 + 验证结果
    pub fn record_shadow(&mut self, name: &str, hit: bool) {
        if let Some(r) = self.rules.get_mut(name) {
            r.shadow_signals += 1;
            if hit {
                r.shadow_hits += 1;
            }
        }
    }

    /// 记录 Active 信号 + 验证结果
    pub fn record_active(&mut self, name: &str, hit: bool) {
        if let Some(r) = self.rules.get_mut(name) {
            r.active_signals += 1;
            if hit {
                r.active_hits += 1;
            }
        }
    }

    /// 检查并自动升级 Shadow 规则
    pub fn check_promotions(&mut self) -> Vec<String> {
        let mut promoted = Vec::new();
        for (name, rule) in self.rules.iter_mut() {
            if rule.ready_for_promotion(self.min_samples, 0.55) {
                rule.mode = RuleMode::Active;
                rule.promoted_at = Some(chrono::Local::now().format("%Y-%m-%d").to_string());
                promoted.push(name.clone());
                info!(
                    "[Adaptive] 规则 '{}' Shadow→Active（命中率 {:.0}%）",
                    name,
                    rule.shadow_hit_rate() * 100.0
                );
            }
        }
        promoted
    }

    /// 基于命中率调整信号源权重（带收缩）
    pub fn adjust_weights(&mut self) {
        let _lock = self.locked.lock().unwrap();
        let mut new_weights = self.signal_weights.clone();

        for (source, w) in new_weights.iter_mut() {
            let hit_rate = self.source_hit_rate(*source);
            if hit_rate > 0.0 {
                let delta = if hit_rate > 0.6 {
                    self.max_weekly_change
                } else if hit_rate < 0.4 {
                    -self.max_weekly_change
                } else {
                    0.0
                };
                // 收缩：调权 = delta × (1 - shrinkage) + 0 × shrinkage
                let adjusted = delta * (1.0 - self.shrinkage);
                *w = (*w + adjusted).clamp(0.05, 0.50);
            }
        }

        // 归一化
        let total: f64 = new_weights.values().sum();
        for v in new_weights.values_mut() {
            *v /= total;
        }
        self.signal_weights = new_weights;
    }

    /// 计算某信号源的近期命中率（简化：统计所有规则中该源的命中率）
    fn source_hit_rate(&self, _source: SignalSource) -> f64 {
        let total: u64 = self
            .rules
            .values()
            .map(|r| r.active_signals + r.shadow_signals)
            .sum();
        let hits: u64 = self
            .rules
            .values()
            .map(|r| r.active_hits + r.shadow_hits)
            .sum();
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// 所有权重摘要
    pub fn weight_summary(&self) -> String {
        self.signal_weights
            .iter()
            .map(|(s, w)| format!("{}={:.0}%", s.label(), w * 100.0))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Shadow 规则状态摘要
    pub fn shadow_summary(&self) -> String {
        let shadows: Vec<_> = self
            .rules
            .values()
            .filter(|r| matches!(r.mode, RuleMode::Shadow))
            .map(|r| {
                format!(
                    "{}: {}/{} ({:.0}%)",
                    r.name,
                    r.shadow_hits,
                    r.shadow_signals,
                    r.shadow_hit_rate() * 100.0
                )
            })
            .collect();
        if shadows.is_empty() {
            "无 Shadow 规则".into()
        } else {
            shadows.join("\n")
        }
    }
}

impl Default for AdaptiveWeightManager {
    fn default() -> Self {
        Self::new(30, 0.05, 0.3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_tracker_shadow() {
        let mut t = RuleTracker::new("test", RuleMode::Shadow);
        assert!(!t.ready_for_promotion(30, 0.55));
        t.shadow_signals = 30;
        t.shadow_hits = 20;
        assert!(t.ready_for_promotion(30, 0.55)); // 67% > 55%
    }

    #[test]
    fn test_rule_tracker_insufficient_samples() {
        let mut t = RuleTracker::new("test", RuleMode::Shadow);
        t.shadow_signals = 5;
        t.shadow_hits = 5; // 100% but not enough samples
        assert!(!t.ready_for_promotion(30, 0.55));
    }

    #[test]
    fn test_adaptive_manager_register_and_record() {
        let mut m = AdaptiveWeightManager::new(5, 0.05, 0.3);
        m.register_rule("测试规则");
        m.record_shadow("测试规则", true);
        m.record_shadow("测试规则", true);
        m.record_shadow("测试规则", true);
        m.record_shadow("测试规则", false);
        m.record_shadow("测试规则", true);

        let promoted = m.check_promotions();
        assert!(promoted.contains(&"测试规则".to_string()));
    }

    #[test]
    fn test_weight_adjustment_normalizes() {
        let mut m = AdaptiveWeightManager::new(30, 0.05, 0.3);
        m.register_rule("r1");
        m.record_shadow("r1", true);
        m.adjust_weights();
        let total: f64 = m.signal_weights.values().sum();
        assert!((total - 1.0).abs() < 0.001, "权重应归一化");
    }

    #[test]
    fn test_shadow_summary() {
        let mut m = AdaptiveWeightManager::new(30, 0.05, 0.3);
        m.register_rule("shadow_rule");
        let s = m.shadow_summary();
        assert!(s.contains("shadow_rule"));
    }
}
