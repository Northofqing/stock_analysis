//! 信号融合层。
//!
//! 核心：
//! - 多信号共振确认（IC/IR 加权替代线性打分）
//! - 信号时效衰减
//! - 共线性处理（高度相关的信号降权）

use std::collections::HashMap;

// ============================================================================
// 信号定义
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalSource {
    Technical,   // 技术面
    FundFlow,    // 资金面
    News,        // 消息面
    Chain,       // 产业链
    Valuation,   // 估值面
}

impl SignalSource {
    pub fn label(&self) -> &'static str {
        match self {
            SignalSource::Technical => "技术面",
            SignalSource::FundFlow => "资金面",
            SignalSource::News => "消息面",
            SignalSource::Chain => "产业链",
            SignalSource::Valuation => "估值面",
        }
    }
}

// ============================================================================
// 信号值
// ============================================================================

#[derive(Debug, Clone)]
pub struct Signal {
    pub source: SignalSource,
    /// 方向：+1 看多, -1 看空, 0 中性
    pub direction: f64,
    /// 强度 0-100
    pub strength: f64,
    /// 信号产生时间（分钟前）。0 = 刚产生。
    pub age_minutes: f64,
}

impl Signal {
    pub fn new(source: SignalSource, direction: f64, strength: f64, age_minutes: f64) -> Self {
        Signal { source, direction, strength, age_minutes }
    }

    /// 时效衰减后的有效强度
    pub fn decayed_strength(&self) -> f64 {
        let lambda = match self.source {
            SignalSource::Technical => 0.01,
            SignalSource::FundFlow => 0.02,
            SignalSource::News => 0.05,
            SignalSource::Chain => 0.005,
            SignalSource::Valuation => 0.002,
        };
        self.strength * (-lambda * self.age_minutes).exp()
    }
}

// ============================================================================
// 信号融合器
// ============================================================================

#[derive(Debug, Clone)]
pub struct SignalFusion {
    /// 各信号源权重（总和=1）
    weights: HashMap<SignalSource, f64>,
    /// 信号源之间的相关系数（用于共线性检测）
    correlations: HashMap<(SignalSource, SignalSource), f64>,
}

impl Default for SignalFusion {
    fn default() -> Self {
        let mut weights = HashMap::new();
        weights.insert(SignalSource::Technical, 0.25);
        weights.insert(SignalSource::FundFlow, 0.25);
        weights.insert(SignalSource::News, 0.15);
        weights.insert(SignalSource::Chain, 0.20);
        weights.insert(SignalSource::Valuation, 0.15);

        // 预设相关系数：技术面和资金面通常正相关
        let mut correlations = HashMap::new();
        correlations.insert((SignalSource::Technical, SignalSource::FundFlow), 0.6);
        correlations.insert((SignalSource::Technical, SignalSource::Chain), 0.3);
        correlations.insert((SignalSource::FundFlow, SignalSource::News), 0.4);
        Self { weights, correlations }
    }
}

impl SignalFusion {
    pub fn with_weights(weights: HashMap<SignalSource, f64>) -> Self {
        let total: f64 = weights.values().sum();
        let normalized: HashMap<_, _> = weights
            .into_iter()
            .map(|(k, v)| (k, v / total))
            .collect();
        Self { weights: normalized, ..Default::default() }
    }

    /// 计算多信号共振得分（-100 到 +100）
    pub fn resonance(&self, signals: &[Signal]) -> f64 {
        if signals.is_empty() {
            return 0.0;
        }

        // Step 1: 计算每个信号的衰减后有效值
        let effective: Vec<(SignalSource, f64, f64)> = signals
            .iter()
            .map(|s| {
                let d = s.decayed_strength();
                (s.source, s.direction * d, d)
            })
            .collect();

        // Step 2: 共线性降权
        let mut adjusted_weights = self.weights.clone();
        for i in 0..effective.len() {
            for j in (i + 1)..effective.len() {
                let (sa, _, _) = effective[i];
                let (sb, _, _) = effective[j];
                let corr = self.correlations
                    .get(&(sa, sb))
                    .or_else(|| self.correlations.get(&(sb, sa)))
                    .copied()
                    .unwrap_or(0.0);
                if corr > 0.5 {
                    // 高相关 → 降权较弱的信号
                    let (_, _, da) = effective[i];
                    let (_, _, db) = effective[j];
                    if da > db {
                        *adjusted_weights.get_mut(&sb).unwrap_or(&mut 0.0) *= 0.5;
                    } else {
                        *adjusted_weights.get_mut(&sa).unwrap_or(&mut 0.0) *= 0.5;
                    }
                }
            }
        }

        // Step 3: 加权求和 + 方向一致性
        let mut score = 0.0;
        let mut direction_consensus = 0.0;
        let mut total_weight = 0.0;

        for (source, dir_val, _) in &effective {
            let w = adjusted_weights.get(source).copied().unwrap_or(0.0);
            score += dir_val * w;
            direction_consensus += dir_val.signum() * w;
            total_weight += w;
        }

        if total_weight > 0.0 {
            score /= total_weight;
        }

        // 方向一致性加成/惩罚
        let consensus = direction_consensus.abs() / total_weight.max(0.01);
        if consensus > 0.8 {
            score *= 1.2; // 高度一致，置信度加成
        } else if consensus < 0.5 {
            score *= 0.7; // 矛盾信号，降权
        }

        score.clamp(-100.0, 100.0)
    }

    /// 根据共振得分给出操作建议
    pub fn recommend(&self, resonance: f64) -> &'static str {
        match resonance {
            r if r >= 60.0 => "强买入（多信号共振）",
            r if r >= 30.0 => "适度参与",
            r if r >= 15.0 => "轻仓试探",
            r if r >= -15.0 => "观望（信号不足）",
            r if r >= -30.0 => "减仓观察",
            _ => "回避/卖出",
        }
    }

    /// 调整某个信号源的权重
    pub fn adjust_weight(&mut self, source: SignalSource, delta: f64) {
        if let Some(w) = self.weights.get_mut(&source) {
            *w = (*w + delta).clamp(0.05, 0.50);
        }
        // 重归一化
        let total: f64 = self.weights.values().sum();
        for v in self.weights.values_mut() {
            *v /= total;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_signals() {
        let f = SignalFusion::default();
        assert!((f.resonance(&[]) - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_perfect_resonance() {
        let f = SignalFusion::default();
        let signals = vec![
            Signal::new(SignalSource::Technical, 1.0, 80.0, 0.0),
            Signal::new(SignalSource::FundFlow, 1.0, 75.0, 0.0),
            Signal::new(SignalSource::Chain, 1.0, 70.0, 0.0),
        ];
        let score = f.resonance(&signals);
        assert!(score > 50.0); // High score for all-positive signals
    }

    #[test]
    fn test_conflicting_signals() {
        let f = SignalFusion::default();
        let signals = vec![
            Signal::new(SignalSource::Technical, 1.0, 80.0, 0.0),  // 看多
            Signal::new(SignalSource::FundFlow, -1.0, 80.0, 0.0), // 看空
        ];
        let score = f.resonance(&signals);
        assert!(score.abs() < 20.0); // Conflicting → low absolute score
    }

    #[test]
    fn test_signal_decay() {
        let fresh = Signal::new(SignalSource::News, 1.0, 100.0, 0.0);
        let old = Signal::new(SignalSource::News, 1.0, 100.0, 30.0);
        assert!(old.decayed_strength() < fresh.decayed_strength());
    }

    #[test]
    fn test_recommend_levels() {
        let f = SignalFusion::default();
        assert_eq!(f.recommend(70.0), "强买入（多信号共振）");
        assert_eq!(f.recommend(40.0), "适度参与");
        assert_eq!(f.recommend(10.0), "观望（信号不足）");
        assert_eq!(f.recommend(-50.0), "回避/卖出");
    }

    #[test]
    fn test_adjust_weight_and_normalize() {
        let mut f = SignalFusion::default();
        let tech_before = f.weights[&SignalSource::Technical];
        f.adjust_weight(SignalSource::Technical, 0.1);
        let total: f64 = f.weights.values().sum();
        assert!((total - 1.0).abs() < 0.01);
        assert!(f.weights[&SignalSource::Technical] > tech_before);
    }
}
