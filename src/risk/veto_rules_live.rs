//! 实时否决规则实现。
//!
//! 3 条规则对应原 `pipeline/mod.rs:686-740` 被注释的拦截逻辑：
//!
//! | 规则 | 原注释行 | 触发条件 |
//! |------|---------|---------|
//! | BiasRateRule | 688-704 | 乖离率 > 5% 追高风险 / 空头排列接飞刀 |
//! | MainFlowRule | 706-728 | 主力单日净流出 > 5000 万 / 价涨+主力出逃诱多 |
//! | FundamentalDeteriorationRule | 730-740 | PE 异常 + 净利润大幅下滑 |

use log::warn;

use super::veto_chain::{VetoContext, VetoRule, VetoVerdict};

// ============================================================================
// BiasRateRule — 技术面极端危险形态拦截
// ============================================================================

/// 乖离率过高追高风险 + 空头排列接飞刀风险。
///
/// **触发条件**:
/// - bias_ma5 > 5%: 短期涨幅过大，追高回调风险高
/// - is_bearish (空头排列 StrongBear/Bear): 弱势股反弹可能是诱多
///
/// **优先级**: 10 (最早执行)
pub struct BiasRateRule {
    /// 乖离率阈值 (默认 5.0%)
    pub bias_threshold: f64,
}

impl Default for BiasRateRule {
    fn default() -> Self {
        Self {
            bias_threshold: 5.0,
        }
    }
}

impl VetoRule for BiasRateRule {
    fn name(&self) -> &'static str {
        "BiasRateRule"
    }

    fn priority(&self) -> u8 {
        10
    }

    fn evaluate(&self, ctx: &VetoContext) -> VetoVerdict {
        let mut verdict = VetoVerdict::default();

        // 仅对买入信号做拦截（Hold/Wait/Sell 无需拦截）
        if !ctx.is_buy_signal || ctx.signal_score < 60 {
            return verdict;
        }

        // 条件 1: 乖离率过高 → 追高风险
        if ctx.bias_ma5 > self.bias_threshold {
            verdict.risk_flags.push(format!(
                "❌ 乖离率超{:.0}%(当前{:.1}%)有大幅回调风险，严禁追高，强制降级至观望",
                self.bias_threshold, ctx.bias_ma5
            ));
            verdict.force_hold = true;
        }

        // 条件 2: 空头排列 → 反弹诱多风险
        if ctx.is_bearish {
            verdict.risk_flags.push(
                "❌ 整体处于空头排列，极其弱势，放弃短线博弈避开接飞刀，强制降级至观望".to_string(),
            );
            verdict.force_hold = true;
        }

        verdict
    }
}

// ============================================================================
// MainFlowRule — 资金面拦截
// ============================================================================

/// 主力资金大幅流出 / 诱多形态拦截。
///
/// **触发条件**:
/// - 单日主力净流出 > 5000 万
/// - 股价大涨 (>4%) 但主力净流出 (>1000 万) — 典型诱多/拉高出货
///
/// **数据缺失处理**: MoneyFlow 不可用时返回空 VetoVerdict (不否决)
///
/// **优先级**: 20
pub struct MainFlowRule {
    /// 单日净流出阈值 (元，默认 5000 万)
    pub outflow_threshold: f64,
    /// 诱多: 涨幅阈值 (%，默认 4.0)
    pub lure_pct_threshold: f64,
    /// 诱多: 主力流出阈值 (元，默认 1000 万)
    pub lure_outflow_threshold: f64,
}

impl Default for MainFlowRule {
    fn default() -> Self {
        Self {
            outflow_threshold: 50_000_000.0,
            lure_pct_threshold: 4.0,
            lure_outflow_threshold: 10_000_000.0,
        }
    }
}

impl VetoRule for MainFlowRule {
    fn name(&self) -> &'static str {
        "MainFlowRule"
    }

    fn priority(&self) -> u8 {
        20
    }

    fn evaluate(&self, ctx: &VetoContext) -> VetoVerdict {
        let mut verdict = VetoVerdict::default();

        if !ctx.is_buy_signal || ctx.signal_score < 60 {
            return verdict;
        }

        // 数据缺失 → 不否决（pass-through + warn）
        let days = match &ctx.money_flow_days {
            Some(d) if !d.is_empty() => d,
            _ => {
                warn!(
                    "[{}] MainFlowRule: 资金流数据缺失，跳过资金面拦截",
                    ctx.code
                );
                return verdict;
            }
        };

        let last_day = &days[days.len() - 1];

        // 条件 1: 单日主力大幅净流出
        if last_day.main_net < -self.outflow_threshold {
            verdict.risk_flags.push(format!(
                "❌ 主力资金单日大幅流出({:.2}亿)，风险极高，强制取消买入建议",
                last_day.main_net / 1_0000_0000.0
            ));
            verdict.force_hold = true;
        }

        // 条件 2: 价涨量增但资金大幅流出（诱多）
        if last_day.pct_chg > self.lure_pct_threshold
            && last_day.main_net < -self.lure_outflow_threshold
        {
            verdict.risk_flags.push(
                "❌ 股价大涨但主力净流出(典型诱多/拉高出货)，极其凶险，强制取消买入建议"
                    .to_string(),
            );
            verdict.force_hold = true;
        }

        verdict
    }
}

// ============================================================================
// FundamentalDeteriorationRule — 基本面恶化拦截
// ============================================================================

/// 基本面极度恶化拦截。
///
/// **触发条件**:
/// - PE < 0 (亏损) 或 PE > 300 (畸高估值)
/// - 且净利润同比下滑 > 30%
///
/// **数据缺失处理**: PE 或净利润数据缺失时返回空 VetoVerdict (不否决)
///
/// **优先级**: 30
pub struct FundamentalDeteriorationRule {
    /// PE 上限阈值 (超过视为畸高)
    pub pe_upper: f64,
    /// 净利润同比下滑阈值 (%，负数)
    pub profit_decline_threshold: f64,
}

impl Default for FundamentalDeteriorationRule {
    fn default() -> Self {
        Self {
            pe_upper: 300.0,
            profit_decline_threshold: -30.0,
        }
    }
}

impl VetoRule for FundamentalDeteriorationRule {
    fn name(&self) -> &'static str {
        "FundamentalDeteriorationRule"
    }

    fn priority(&self) -> u8 {
        30
    }

    fn evaluate(&self, ctx: &VetoContext) -> VetoVerdict {
        let mut verdict = VetoVerdict::default();

        if !ctx.is_buy_signal || ctx.signal_score < 60 {
            return verdict;
        }

        // 数据缺失 → 不否决
        let pe = match ctx.pe_ratio {
            Some(v) if v != 0.0 => v,
            _ => return verdict,
        };
        let np_yoy = match ctx.net_profit_yoy {
            Some(v) => v,
            None => return verdict,
        };

        let is_pe_abnormal = pe < 0.0 || pe > self.pe_upper;
        let is_profit_crashing = np_yoy < self.profit_decline_threshold;

        if is_pe_abnormal && is_profit_crashing {
            verdict.risk_flags.push(format!(
                "❌ 基本面极度恶化(PE={:.0} 业绩大幅下滑{:.0}% 且估值畸高/亏损)，底线拦截取消买入",
                pe, np_yoy
            ));
            verdict.force_hold = true;
        }

        verdict
    }
}

// ============================================================================
// 工厂函数
// ============================================================================

use super::veto_chain::{VetoChain, VetoChainConfig};

/// 根据配置构建 VetoChain。
///
/// 返回 None 表示 VetoChain 总开关关闭，调用方应完全跳过。
pub fn build_chain(config: &VetoChainConfig) -> Option<VetoChain> {
    if !config.enabled {
        return None;
    }

    let mut rules: Vec<Box<dyn VetoRule>> = Vec::new();

    if config.bias_rate_enabled || config.bearish_alignment_enabled {
        rules.push(Box::new(BiasRateRule::default()));
    }
    if config.main_flow_enabled {
        rules.push(Box::new(MainFlowRule::default()));
    }
    if config.fundamental_enabled {
        rules.push(Box::new(FundamentalDeteriorationRule::default()));
    }

    if rules.is_empty() {
        None
    } else {
        Some(VetoChain::new(rules))
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::money_flow::MoneyFlowDay;

    fn make_ctx(overrides: impl FnOnce(&mut VetoContext)) -> VetoContext {
        let mut ctx = VetoContext {
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
        };
        overrides(&mut ctx);
        ctx
    }

    // ── BiasRateRule ──

    #[test]
    fn test_bias_rate_triggers_on_high_bias() {
        let rule = BiasRateRule::default();
        let ctx = make_ctx(|c| {
            c.bias_ma5 = 6.0;
        });
        let v = rule.evaluate(&ctx);
        assert!(v.force_hold);
        assert!(!v.risk_flags.is_empty());
    }

    #[test]
    fn test_bias_rate_triggers_on_bearish() {
        let rule = BiasRateRule::default();
        let ctx = make_ctx(|c| {
            c.is_bearish = true;
        });
        let v = rule.evaluate(&ctx);
        assert!(v.force_hold);
    }

    #[test]
    fn test_bias_rate_no_trigger_normal() {
        let rule = BiasRateRule::default();
        let ctx = make_ctx(|_| {});
        let v = rule.evaluate(&ctx);
        assert!(!v.force_hold);
    }

    #[test]
    fn test_bias_rate_skips_non_buy() {
        let rule = BiasRateRule::default();
        let ctx = make_ctx(|c| {
            c.is_buy_signal = false;
            c.bias_ma5 = 10.0; // 即使乖离很高
        });
        let v = rule.evaluate(&ctx);
        assert!(!v.force_hold);
    }

    #[test]
    fn test_bias_rate_skips_low_score() {
        let rule = BiasRateRule::default();
        let ctx = make_ctx(|c| {
            c.signal_score = 55; // < 60
            c.bias_ma5 = 10.0;
        });
        let v = rule.evaluate(&ctx);
        assert!(!v.force_hold);
    }

    // ── MainFlowRule ──

    #[test]
    fn test_main_flow_triggers_on_heavy_outflow() {
        let rule = MainFlowRule::default();
        let ctx = make_ctx(|c| {
            c.money_flow_days = Some(vec![MoneyFlowDay {
                date: "2026-06-20".to_string(),
                main_net: -60_000_000.0, // -6000 万
                xl_net: -30_000_000.0,
                big_net: -30_000_000.0,
                main_pct: -10.0,
                pct_chg: -2.0,
            }]);
        });
        let v = rule.evaluate(&ctx);
        assert!(v.force_hold);
    }

    #[test]
    fn test_main_flow_triggers_on_lure() {
        let rule = MainFlowRule::default();
        let ctx = make_ctx(|c| {
            c.money_flow_days = Some(vec![MoneyFlowDay {
                date: "2026-06-20".to_string(),
                main_net: -15_000_000.0, // -1500 万
                xl_net: -10_000_000.0,
                big_net: -5_000_000.0,
                main_pct: -8.0,
                pct_chg: 5.0, // +5% 大涨
            }]);
        });
        let v = rule.evaluate(&ctx);
        assert!(v.force_hold);
        assert!(v.risk_flags[0].contains("诱多"));
    }

    #[test]
    fn test_main_flow_no_trigger_normal() {
        let rule = MainFlowRule::default();
        let ctx = make_ctx(|c| {
            c.money_flow_days = Some(vec![MoneyFlowDay {
                date: "2026-06-20".to_string(),
                main_net: -5_000_000.0, // -500万，正常范围
                xl_net: 0.0,
                big_net: -5_000_000.0,
                main_pct: -2.0,
                pct_chg: 0.5,
            }]);
        });
        let v = rule.evaluate(&ctx);
        assert!(!v.force_hold);
    }

    #[test]
    fn test_main_flow_missing_data_pass_through() {
        let rule = MainFlowRule::default();
        let ctx = make_ctx(|c| {
            c.money_flow_days = None; // 数据缺失
        });
        let v = rule.evaluate(&ctx);
        assert!(!v.force_hold);
    }

    // ── FundamentalDeteriorationRule ──

    #[test]
    fn test_fundamental_triggers_on_deterioration() {
        let rule = FundamentalDeteriorationRule::default();
        let ctx = make_ctx(|c| {
            c.pe_ratio = Some(-5.0); // 亏损
            c.net_profit_yoy = Some(-50.0); // 大幅下滑
        });
        let v = rule.evaluate(&ctx);
        assert!(v.force_hold);
    }

    #[test]
    fn test_fundamental_triggers_on_absurd_pe() {
        let rule = FundamentalDeteriorationRule::default();
        let ctx = make_ctx(|c| {
            c.pe_ratio = Some(500.0); // PE > 300
            c.net_profit_yoy = Some(-35.0);
        });
        let v = rule.evaluate(&ctx);
        assert!(v.force_hold);
    }

    #[test]
    fn test_fundamental_no_trigger_normal() {
        let rule = FundamentalDeteriorationRule::default();
        let ctx = make_ctx(|c| {
            c.pe_ratio = Some(15.0);
            c.net_profit_yoy = Some(10.0); // 增长
        });
        let v = rule.evaluate(&ctx);
        assert!(!v.force_hold);
    }

    #[test]
    fn test_fundamental_missing_data_pass_through() {
        let rule = FundamentalDeteriorationRule::default();
        let ctx = make_ctx(|c| {
            c.pe_ratio = None; // 数据缺失
            c.net_profit_yoy = Some(-50.0);
        });
        let v = rule.evaluate(&ctx);
        assert!(!v.force_hold);
    }

    // ── build_chain ──

    #[test]
    fn test_build_chain_disabled() {
        let config = VetoChainConfig {
            enabled: false,
            ..VetoChainConfig::default()
        };
        assert!(build_chain(&config).is_none());
    }

    #[test]
    fn test_build_chain_all_enabled() {
        let config = VetoChainConfig::default();
        let chain = build_chain(&config).unwrap();
        assert_eq!(chain.len(), 3);
    }

    #[test]
    fn test_build_chain_partial() {
        let config = VetoChainConfig {
            main_flow_enabled: false,
            fundamental_enabled: false,
            ..VetoChainConfig::default()
        };
        let chain = build_chain(&config).unwrap();
        assert_eq!(chain.len(), 1); // 仅 BiasRateRule
    }
}
