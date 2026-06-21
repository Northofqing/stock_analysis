//! 实时风控否决链 (VetoChain)。
//!
//! 将原本 inline 注释的 3 段风控拦截（技术面/资金面/基本面）重构为
//! 可配置的策略链，支持 dry_run / live 双模式灰度上线。
//!
//! ## 架构
//!
//! ```text
//! VetoChain
//!   ├── BiasRateRule          (乖离率 + 空头排列拦截)
//!   ├── MainFlowRule          (主力资金流出拦截)
//!   └── FundamentalDeteriorationRule (基本面恶化拦截)
//! ```
//!
//! ## 与旧模块关系
//!
//! - `pipeline/veto_rules.rs` 侧重**基本面估值否决** (Phase 1/3)，互补不冲突
//! - VetoChain 先执行 → veto_rules::evaluate 后执行
//! - VetoContext 使用基础类型（不依赖 trend_analyzer），由 Pipeline 层负责转换

use log::{info, warn};

use crate::data_provider::money_flow::MoneyFlowDay;

// ============================================================================
// VetoContext — 跨规则共享的评估上下文
// ============================================================================

/// 否决评估上下文。
///
/// 所有字段使用基础类型，不依赖 trend_analyzer / strategy 等领域类型，
/// 由 Pipeline 层在调用前完成类型转换。
#[derive(Debug, Clone)]
pub struct VetoContext {
    pub code: String,
    pub current_price: f64,
    /// 当前信号评分 (0-100)，规则可下调
    pub signal_score: i32,
    /// 是否为强烈买入或买入信号
    pub is_buy_signal: bool,
    /// 乖离率 (现价距 MA5 的百分比)
    pub bias_ma5: f64,
    /// 是否处于空头排列 (StrongBear / Bear)
    pub is_bearish: bool,
    /// 资金流数据 (可选 — 规则自行判断)
    pub money_flow_days: Option<Vec<MoneyFlowDay>>,
    /// 当日涨跌幅 (%)
    pub pct_chg: Option<f64>,
    /// PE 比率 (可选)
    pub pe_ratio: Option<f64>,
    /// 净利润同比增速 (可选)
    pub net_profit_yoy: Option<f64>,
}

// ============================================================================
// VetoVerdict — 单条规则的裁决结果
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct VetoVerdict {
    /// 触发的风险标签 (用于日志/审计/DB 持久化)
    pub risk_flags: Vec<String>,
    /// 评分下调量 (累计加到 signal_score)
    pub score_penalty: i32,
    /// 是否强制将买入信号降级为 Hold
    pub force_hold: bool,
}

impl VetoVerdict {
    pub fn is_empty(&self) -> bool {
        self.risk_flags.is_empty()
    }
}

// ============================================================================
// VetoRule trait
// ============================================================================

/// 否决规则统一接口。
///
/// 实现此 trait 即可注册到 VetoChain。
/// evaluate 方法不应 panic —— 由 VetoChain 以 catch_unwind 包裹。
pub trait VetoRule: Send + Sync {
    /// 规则名称 (日志与审计用)
    fn name(&self) -> &'static str;

    /// 评估是否触发否决
    fn evaluate(&self, ctx: &VetoContext) -> VetoVerdict;

    /// 优先级 (数字越小越先执行，默认 50)
    fn priority(&self) -> u8 {
        50
    }
}

// ============================================================================
// VetoChain
// ============================================================================

/// 否决规则链。
///
/// 按优先级排序执行，每个规则独立裁决，结果汇聚到 VetoOutcome。
/// 单条规则 panic 不影响其他规则继续执行。
pub struct VetoChain {
    rules: Vec<Box<dyn VetoRule>>,
}

impl VetoChain {
    pub fn new(mut rules: Vec<Box<dyn VetoRule>>) -> Self {
        rules.sort_by_key(|r| r.priority());
        Self { rules }
    }

    /// 执行所有规则，返回汇总结果。
    pub fn evaluate_all(&self, ctx: &VetoContext) -> VetoOutcome {
        let mut outcome = VetoOutcome::default();

        for rule in &self.rules {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rule.evaluate(ctx)
            }));

            match result {
                Ok(verdict) => {
                    if !verdict.is_empty() {
                        info!(
                            "[{}] VetoChain 规则触发: {} — flags={:?} penalty={} force_hold={}",
                            ctx.code,
                            rule.name(),
                            verdict.risk_flags,
                            verdict.score_penalty,
                            verdict.force_hold
                        );
                        outcome.flags.extend(verdict.risk_flags);
                        outcome.total_penalty += verdict.score_penalty;
                        if verdict.force_hold {
                            outcome.force_hold = true;
                        }
                    }
                }
                Err(e) => {
                    // 规则 panic → 记录并继续
                    let msg = if let Some(s) = e.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = e.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "unknown panic".to_string()
                    };
                    warn!(
                        "[{}] VetoChain 规则 '{}' panic: {} — 跳过该规则",
                        ctx.code,
                        rule.name(),
                        msg
                    );
                }
            }
        }

        outcome
    }

    /// 规则数量 (测试用)
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

// ============================================================================
// VetoOutcome — VetoChain 汇总结果
// ============================================================================

#[derive(Debug, Clone, Default)]
pub struct VetoOutcome {
    /// 所有规则触发的风险标签
    pub flags: Vec<String>,
    /// 总评分下调量
    pub total_penalty: i32,
    /// 是否有规则要求强制 Hold
    pub force_hold: bool,
}

impl VetoOutcome {
    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }
}

// ============================================================================
// 配置
// ============================================================================

/// VetoChain 配置 (来自 config/monitor.toml [live_veto])
#[derive(Debug, Clone)]
pub struct VetoChainConfig {
    /// 总开关
    pub enabled: bool,
    /// 运行模式
    pub mode: VetoMode,
    /// 乖离率拦截开关
    pub bias_rate_enabled: bool,
    /// 空头排列拦截开关
    pub bearish_alignment_enabled: bool,
    /// 主力资金拦截开关
    pub main_flow_enabled: bool,
    /// 基本面恶化拦截开关
    pub fundamental_enabled: bool,
}

impl Default for VetoChainConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: VetoMode::DryRun, // 默认 dry_run，安全第一
            bias_rate_enabled: true,
            bearish_alignment_enabled: true,
            main_flow_enabled: true,
            fundamental_enabled: true,
        }
    }
}

/// VetoChain 运行模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VetoMode {
    /// 仅记录日志，不修改 signal_score / buy_signal
    DryRun,
    /// 实际拦截
    Live,
}

impl VetoMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "live" => VetoMode::Live,
            _ => VetoMode::DryRun, // 默认安全
        }
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个始终触发的测试规则
    struct AlwaysTriggerRule;

    impl VetoRule for AlwaysTriggerRule {
        fn name(&self) -> &'static str {
            "AlwaysTrigger"
        }
        fn evaluate(&self, _ctx: &VetoContext) -> VetoVerdict {
            VetoVerdict {
                risk_flags: vec!["TEST: always triggers".to_string()],
                score_penalty: 5,
                force_hold: true,
            }
        }
        fn priority(&self) -> u8 {
            10
        }
    }

    /// 一个永不触发的规则
    struct NeverTriggerRule;

    impl VetoRule for NeverTriggerRule {
        fn name(&self) -> &'static str {
            "NeverTrigger"
        }
        fn evaluate(&self, _ctx: &VetoContext) -> VetoVerdict {
            VetoVerdict::default()
        }
    }

    /// 一个会 panic 的规则
    struct PanicRule;

    impl VetoRule for PanicRule {
        fn name(&self) -> &'static str {
            "PanicRule"
        }
        fn evaluate(&self, _ctx: &VetoContext) -> VetoVerdict {
            panic!("intentional test panic")
        }
    }

    fn make_ctx() -> VetoContext {
        VetoContext {
            code: "TEST_CODE".to_string(),
            current_price: 10.0,
            signal_score: 65,
            is_buy_signal: true,
            bias_ma5: 2.0,
            is_bearish: false,
            money_flow_days: None,
            pct_chg: None,
            pe_ratio: None,
            net_profit_yoy: None,
        }
    }

    #[test]
    fn test_empty_chain_no_panic() {
        let chain = VetoChain::new(vec![]);
        let outcome = chain.evaluate_all(&make_ctx());
        assert!(outcome.is_empty());
        assert!(!outcome.force_hold);
    }

    #[test]
    fn test_single_rule_triggers() {
        let chain = VetoChain::new(vec![Box::new(AlwaysTriggerRule)]);
        let outcome = chain.evaluate_all(&make_ctx());
        assert_eq!(outcome.flags.len(), 1);
        assert_eq!(outcome.total_penalty, 5);
        assert!(outcome.force_hold);
    }

    #[test]
    fn test_multiple_rules_aggregate() {
        let chain = VetoChain::new(vec![
            Box::new(AlwaysTriggerRule),
            Box::new(NeverTriggerRule),
        ]);
        let outcome = chain.evaluate_all(&make_ctx());
        assert_eq!(outcome.flags.len(), 1);
    }

    #[test]
    fn test_panic_rule_does_not_propagate() {
        let chain = VetoChain::new(vec![
            Box::new(PanicRule),
            Box::new(AlwaysTriggerRule),
        ]);
        let outcome = chain.evaluate_all(&make_ctx());
        // PanicRule 被跳过，AlwaysTriggerRule 正常触发
        assert_eq!(outcome.flags.len(), 1);
        assert!(outcome.force_hold);
    }

    #[test]
    fn test_chain_len() {
        let chain = VetoChain::new(vec![
            Box::new(AlwaysTriggerRule),
            Box::new(NeverTriggerRule),
        ]);
        assert_eq!(chain.len(), 2);
        assert!(!chain.is_empty());
    }

    #[test]
    fn test_veto_mode_from_str() {
        assert_eq!(VetoMode::from_str("live"), VetoMode::Live);
        assert_eq!(VetoMode::from_str("dry_run"), VetoMode::DryRun);
        assert_eq!(VetoMode::from_str("unknown"), VetoMode::DryRun); // 默认安全
    }

    #[test]
    fn test_veto_context_with_test_code() {
        let ctx = make_ctx();
        assert!(ctx.code.starts_with("TEST_CODE"));
        assert!(ctx.current_price > 0.0);
    }
}
