//! v10 P0.3 (BC-1) — real_alpha 计算
//!
//! BR-020: 胜率必对标 benchmark, real_alpha = push_winrate - 同日同窗市场涨幅
//! 落地 (v10 §5 BC-1 + Q2=B):
//! - real_alpha = push_winrate - 同日同窗市场涨幅基准
//! - 市场涨幅基准 = **全市场上涨家数占比** (Q2=B 决策, AQR academic 标准)
//! - 输入: 推送胜率 (0.0-1.0), 同日同窗市场上涨家数占比 (0.0-1.0)
//! - 输出: real_alpha (正 = 跑赢市场, 负 = 跑输, 0 = 与市场持平)
//! - 防御: 数值越界/NaN → 返回 None
//!
//! 例: 推送胜率 60% (0.6), 当日市场 50% 上涨 (0.5) → real_alpha = 0.1 (10% 真 alpha)

/// real_alpha = push_winrate - market_up_rate
/// 返回 None: 任一输入 NaN, 或越界 [0.0, 1.0]
pub fn compute_real_alpha(push_winrate: f64, market_up_rate: f64) -> Option<f64> {
    // 防御: NaN
    if !push_winrate.is_finite() || !market_up_rate.is_finite() {
        return None;
    }
    // 防御: 越界
    if !(0.0..=1.0).contains(&push_winrate) || !(0.0..=1.0).contains(&market_up_rate) {
        return None;
    }
    Some(push_winrate - market_up_rate)
}

/// 当日全市场上涨家数占比 → 0.0-1.0
/// 输入: 上涨家数, 总家数
/// 返回: 上涨占比, 0.0 if 总家数=0
pub fn compute_market_up_rate(up_count: usize, total_count: usize) -> f64 {
    if total_count == 0 {
        return 0.0;
    }
    (up_count as f64) / (total_count as f64)
}

/// 置信度 (v10 §6.1.1)
///
/// A: ≥2 独立源交叉证实, 关键数值误差 < 1%
/// B: 单源, 或多源但误差 1%~5%
/// C: 推算/估值 (盘中资金流日级估算, MISSING 堆断)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    /// 多源交叉证实, 高置信
    A,
    /// 单源或误差 1-5%, 中置信
    B,
    /// 推算/估算/MISSING, 低置信
    C,
}

impl Confidence {
    /// 从 (源数量, 误差百分比) 推导置信度
    /// - 2+ 源 AND 误差 < 1% → A
    /// - 1 源 OR 误差 1-5% → B
    /// - 0 源 OR 误差 > 5% OR NaN → C
    pub fn from_sources(source_count: usize, error_pct: f64) -> Self {
        if !error_pct.is_finite() || source_count == 0 || error_pct > 5.0 {
            return Confidence::C;
        }
        if source_count >= 2 && error_pct < 1.0 {
            return Confidence::A;
        }
        Confidence::B
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Confidence::A => "A",
            Confidence::B => "B",
            Confidence::C => "C",
        }
    }
}

/// 推送信封 5 要素 (v10 §6.1)
/// 这是个数据容器, 用于 push notification 拼接
#[derive(Debug, Clone)]
pub struct PushEnvelope {
    /// 推送类型: 竞价机会 / 开盘买入 / 虚拟仓卖出 / 持仓操作 / 资金主线 / 盘后复盘
    pub push_type: String,
    /// 标的: 名称(代码)
    pub symbol: String,
    /// 现价
    pub price: f64,
    /// 涨幅 (%)
    pub change_pct: f64,
    /// 主因 (1 句话 ≤ 25 字, 说不清=不推)
    pub main_reason: String,
    /// 置信度 A/B/C
    pub confidence: Confidence,
    /// 4 视角速览: (公司/资金/技术/情绪) 各 1 行
    pub four_views: FourViews,
    /// 分层建议: 激进/稳健/保守
    pub tier_advice: TierAdvice,
    /// 留白: 数据不足项, MISSING 不臆造
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FourViews {
    pub company: String,
    pub fund: String,
    pub technical: String,
    pub sentiment: String,
}

#[derive(Debug, Clone, Default)]
pub struct TierAdvice {
    pub aggressive: String,
    pub steady: String,
    pub conservative: String,
}

impl PushEnvelope {
    /// 渲染 5 要素为 markdown 文本 (供 wechat / feishu 推送)
    /// 严格按 v10 §6.1 顺序: 主因 / 置信度 / 4 视角 / 分层建议 / 留白
    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("【{}】\n", self.push_type));
        out.push_str(&format!(
            "标的: {} 现价 {:.2} 涨幅 {:+.1}%\n",
            self.symbol, self.price, self.change_pct
        ));
        out.push_str("────────────────────\n");
        out.push_str(&format!("① 主因 (1 句话): {}\n", self.main_reason));
        out.push_str(&format!(
            "② 置信度: {} (A=多源交叉, B=单源, C=推算)\n",
            self.confidence.as_str()
        ));
        out.push_str("③ 4 视角速览:\n");
        out.push_str(&format!("   · 公司/事件: {}\n", self.four_views.company));
        out.push_str(&format!("   · 资金:       {}\n", self.four_views.fund));
        out.push_str(&format!("   · 技术:       {}\n", self.four_views.technical));
        out.push_str(&format!("   · 情绪:       {}\n", self.four_views.sentiment));
        out.push_str("④ 分层建议:\n");
        out.push_str(&format!("   · 激进: {}\n", self.tier_advice.aggressive));
        out.push_str(&format!("   · 稳健: {}\n", self.tier_advice.steady));
        out.push_str(&format!("   · 保守: {}\n", self.tier_advice.conservative));
        if self.missing.is_empty() {
            out.push_str("⑤ 留白: (无)\n");
        } else {
            out.push_str(&format!("⑤ 留白: {}\n", self.missing.join(", ")));
        }
        out
    }

    /// 硬门禁: 开盘买入必须 A 置信度 (BC-7)
    /// 返回: Some(reason) 如果被拦截, None 如果通过
    pub fn check_open_buy_must_be_a(&self) -> Option<String> {
        if self.push_type == "开盘买入" && self.confidence != Confidence::A {
            Some(format!(
                "BC-7: 开盘买入必须 A 置信度, 当前 {} (单源/推算不入虚拟仓, §2.1/§2.4 张力)",
                self.confidence.as_str()
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== real_alpha =====

    #[test]
    fn test_real_alpha_basic() {
        // f64 精度: 0.6 - 0.5 ≈ 0.09999999999999998, 用 epsilon 比较
        let eps = 1e-9;
        let r1 = compute_real_alpha(0.6, 0.5).unwrap();
        assert!((r1 - 0.1).abs() < eps, "60%-50%=0.1, got {}", r1);
        let r2 = compute_real_alpha(0.5, 0.5).unwrap();
        assert!((r2 - 0.0).abs() < eps, "50%-50%=0, got {}", r2);
        let r3 = compute_real_alpha(0.4, 0.5).unwrap();
        assert!((r3 - (-0.1)).abs() < eps, "40%-50%=-0.1, got {}", r3);
    }

    #[test]
    fn test_real_alpha_boundary() {
        assert_eq!(compute_real_alpha(0.0, 0.0), Some(0.0));   // 0% 边界
        assert_eq!(compute_real_alpha(1.0, 1.0), Some(0.0));   // 100% 边界
        assert_eq!(compute_real_alpha(0.0, 1.0), Some(-1.0));  // 最大负
        assert_eq!(compute_real_alpha(1.0, 0.0), Some(1.0));   // 最大正
    }

    #[test]
    fn test_real_alpha_nan_returns_none() {
        assert_eq!(compute_real_alpha(f64::NAN, 0.5), None);
        assert_eq!(compute_real_alpha(0.5, f64::NAN), None);
        assert_eq!(compute_real_alpha(f64::INFINITY, 0.5), None);
    }

    #[test]
    fn test_real_alpha_out_of_range_returns_none() {
        assert_eq!(compute_real_alpha(-0.1, 0.5), None);  // push < 0
        assert_eq!(compute_real_alpha(0.5, 1.1), None);   // market > 1
        assert_eq!(compute_real_alpha(1.5, 0.5), None);   // push > 1
    }

    // ===== market_up_rate =====

    #[test]
    fn test_market_up_rate_basic() {
        assert_eq!(compute_market_up_rate(2700, 5400), 0.5);
        assert_eq!(compute_market_up_rate(3000, 5000), 0.6);
        assert_eq!(compute_market_up_rate(0, 100), 0.0);
    }

    #[test]
    fn test_market_up_rate_zero_total() {
        assert_eq!(compute_market_up_rate(0, 0), 0.0);
        assert_eq!(compute_market_up_rate(100, 0), 0.0); // 异常: 上涨 > 总数 → 0
    }

    // ===== Confidence =====

    #[test]
    fn test_confidence_a_high_quality() {
        assert_eq!(Confidence::from_sources(2, 0.5), Confidence::A);
        assert_eq!(Confidence::from_sources(3, 0.0), Confidence::A);
    }

    #[test]
    fn test_confidence_b_medium_quality() {
        assert_eq!(Confidence::from_sources(1, 2.0), Confidence::B);  // 单源
        assert_eq!(Confidence::from_sources(2, 3.0), Confidence::B); // 2 源但误差 3%
    }

    #[test]
    fn test_confidence_c_low_quality() {
        assert_eq!(Confidence::from_sources(0, 0.0), Confidence::C);   // 0 源
        assert_eq!(Confidence::from_sources(2, 6.0), Confidence::C);   // 误差 > 5%
        assert_eq!(Confidence::from_sources(1, 10.0), Confidence::C);  // 单源 + 误差大
        assert_eq!(Confidence::from_sources(2, f64::NAN), Confidence::C); // NaN
    }

    // ===== PushEnvelope + 5 要素渲染 =====

    #[test]
    fn test_envelope_render_basic() {
        let env = PushEnvelope {
            push_type: "开盘买入".to_string(),
            symbol: "腾讯(00700)".to_string(),
            price: 350.0,
            change_pct: 2.5,
            main_reason: "竞价高开 + 量比放大".to_string(),
            confidence: Confidence::A,
            four_views: FourViews {
                company: "回购公告".to_string(),
                fund: "主力净流入 1.2 亿".to_string(),
                technical: "MA5/10/20 收敛, 量价齐升".to_string(),
                sentiment: "板块联动, 龙头股带动".to_string(),
            },
            tier_advice: TierAdvice {
                aggressive: "买入 @ 348-352".to_string(),
                steady: "突破 355 加仓".to_string(),
                conservative: "观望 跌 340 出".to_string(),
            },
            missing: vec![],
        };
        let md = env.render_markdown();
        assert!(md.contains("【开盘买入】"));
        assert!(md.contains("① 主因"));
        assert!(md.contains("② 置信度: A"));
        assert!(md.contains("③ 4 视角速览"));
        assert!(md.contains("④ 分层建议"));
        assert!(md.contains("⑤ 留白: (无)"));
    }

    #[test]
    fn test_envelope_render_with_missing() {
        let env = PushEnvelope {
            push_type: "持仓操作".to_string(),
            symbol: "X".to_string(),
            price: 10.0,
            change_pct: 0.0,
            main_reason: "硬止损".to_string(),
            confidence: Confidence::B,
            four_views: FourViews::default(),
            tier_advice: TierAdvice::default(),
            missing: vec!["资金流向".to_string(), "板块联动".to_string()],
        };
        let md = env.render_markdown();
        assert!(md.contains("⑤ 留白: 资金流向, 板块联动"));
    }

    // ===== BC-7: 开盘买入必须 A =====

    #[test]
    fn test_bc7_open_buy_must_be_a_blocks_b() {
        let env = PushEnvelope {
            push_type: "开盘买入".to_string(),
            confidence: Confidence::B,
            ..make_empty_envelope()
        };
        let block = env.check_open_buy_must_be_a();
        assert!(block.is_some(), "BC-7: 开盘买入 + B 应被拦截");
        assert!(block.unwrap().contains("BC-7"));
    }

    #[test]
    fn test_bc7_open_buy_must_be_a_blocks_c() {
        let env = PushEnvelope {
            push_type: "开盘买入".to_string(),
            confidence: Confidence::C,
            ..make_empty_envelope()
        };
        assert!(env.check_open_buy_must_be_a().is_some());
    }

    #[test]
    fn test_bc7_open_buy_must_be_a_passes_a() {
        let env = PushEnvelope {
            push_type: "开盘买入".to_string(),
            confidence: Confidence::A,
            ..make_empty_envelope()
        };
        assert!(env.check_open_buy_must_be_a().is_none(), "A 置信度应通过");
    }

    #[test]
    fn test_bc7_other_push_types_pass_any_confidence() {
        // 非开盘买入不受 BC-7 限制
        for ct in [Confidence::A, Confidence::B, Confidence::C] {
            let env = PushEnvelope {
                push_type: "持仓操作".to_string(),
                confidence: ct,
                ..make_empty_envelope()
            };
            assert!(env.check_open_buy_must_be_a().is_none());
        }
    }

    fn make_empty_envelope() -> PushEnvelope {
        PushEnvelope {
            push_type: String::new(),
            symbol: String::new(),
            price: 0.0,
            change_pct: 0.0,
            main_reason: String::new(),
            confidence: Confidence::C,
            four_views: FourViews::default(),
            tier_advice: TierAdvice::default(),
            missing: vec![],
        }
    }
}
