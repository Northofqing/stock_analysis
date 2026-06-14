//! 风控叠加层。
//!
//! 核心功能：
//! - 持仓类型区分（可用 / T+1 冻结）
//! - 市场状态门控（普涨/结构性/普跌/崩盘）
//! - 动态仓位上限（波动率 × 集中度 × 锁仓折扣）
//! - 三级止损体系（技术/结构/硬止损）

// ============================================================================
// 持仓类型
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionType {
    /// T+0 可用（可当日卖出）
    Available,
    /// T+1 冻结（当日买入，次日解禁）
    Locked { unlock_date: chrono::NaiveDate },
}

impl PositionType {
    pub fn can_sell_today(&self) -> bool {
        matches!(self, PositionType::Available)
    }

    pub fn is_locked(&self) -> bool {
        matches!(self, PositionType::Locked { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            PositionType::Available => "可用",
            PositionType::Locked { .. } => "冻结",
        }
    }
}

// ============================================================================
// 市场状态
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketRegime {
    BullRally,    // 普涨（上涨 > 70%）
    Structural,   // 结构性（30%-70%）
    BearDecline,  // 普跌（< 30%）
    Crash,        // 崩盘（沪指 < -3%）
}

impl MarketRegime {
    /// 市场状态系数（影响仓位上限）
    pub fn position_multiplier(&self) -> f64 {
        match self {
            MarketRegime::BullRally => 1.2,
            MarketRegime::Structural => 1.0,
            MarketRegime::BearDecline => 0.5,
            MarketRegime::Crash => 0.0,
        }
    }

    /// 是否允许新买入
    pub fn allow_new_position(&self) -> bool {
        self.position_multiplier() > 0.0
    }
}

/// 从上涨家数占比判定市场状态
pub fn classify_market(up_ratio: f64, index_change_pct: f64) -> MarketRegime {
    if index_change_pct <= -3.0 {
        MarketRegime::Crash
    } else if up_ratio >= 0.7 {
        MarketRegime::BullRally
    } else if up_ratio >= 0.3 {
        MarketRegime::Structural
    } else {
        MarketRegime::BearDecline
    }
}

// ============================================================================
// 止损三级体系
// ============================================================================

#[derive(Debug, Clone)]
pub struct StopLoss {
    pub technical: f64,   // 一级：买入价 × (1 - 2×ATR%)
    pub structural: f64,  // 二级：最近支撑位 × 0.98
    pub hard: f64,        // 三级：买入价 × 0.92（硬止损）
    pub atr: f64,
    pub buy_price: f64,
    pub support_level: Option<f64>,
}

impl StopLoss {
    pub fn new(buy_price: f64, atr: f64, support: Option<f64>) -> Self {
        let technical = buy_price * (1.0 - 2.0 * atr / 100.0);
        let hard = buy_price * 0.92;
        let structural = support.map(|s| s * 0.98).unwrap_or(hard);
        StopLoss { technical, structural, hard, atr, buy_price, support_level: support }
    }

    /// 有效止损价：取最紧的（更早保护本金）
    pub fn effective(&self) -> f64 {
        self.technical.max(self.structural).max(self.hard)
    }

    /// 距止损的距离（%）
    pub fn distance_pct(&self, current_price: f64) -> f64 {
        (current_price - self.effective()) / current_price * 100.0
    }

    /// 是否已触发止损
    pub fn triggered(&self, current_price: f64) -> bool {
        current_price <= self.effective()
    }

    /// 止损建议文本
    pub fn advice(&self, current_price: f64, pos_type: PositionType) -> String {
        let eff = self.effective();
        let dist = self.distance_pct(current_price);
        let base = format!(
            "止损价 {:.2}（技术{:.2}/结构{:.2}/硬{:.2}）当前 {:.2} 距止损 {:.1}%",
            eff, self.technical, self.structural, self.hard, current_price, dist
        );
        if pos_type.is_locked() {
            format!("{} | T+1锁仓，无法当日卖出，建议次日竞价挂单", base)
        } else {
            format!("{} | 建议立即减仓", base)
        }
    }
}

// ============================================================================
// 动态仓位计算
// ============================================================================

#[derive(Debug, Clone)]
pub struct PositionSizer {
    pub total_capital: f64,
    pub max_positions: usize,
    pub single_stock_cap_pct: f64,
    pub chain_concentration_limit: f64,
    pub t1_frozen_warn_ratio: f64,
}

impl Default for PositionSizer {
    fn default() -> Self {
        Self {
            total_capital: 100_000.0,
            max_positions: 5,
            single_stock_cap_pct: 20.0,
            chain_concentration_limit: 40.0,
            t1_frozen_warn_ratio: 30.0,
        }
    }
}

impl PositionSizer {
    pub fn from_env() -> Self {
        let total = std::env::var("TOTAL_CAPITAL").ok().and_then(|s| s.parse().ok()).unwrap_or(100_000.0);
        let max_pos = std::env::var("MAX_POSITIONS").ok().and_then(|s| s.parse().ok()).unwrap_or(5);
        let cap = std::env::var("RISK_SINGLE_STOCK_CAP_PCT").ok().and_then(|s| s.parse().ok()).unwrap_or(20.0);
        let chain = std::env::var("RISK_CHAIN_CONCENTRATION_PCT").ok().and_then(|s| s.parse().ok()).unwrap_or(40.0);
        let t1 = std::env::var("RISK_T1_FROZEN_RATIO").ok().and_then(|s| s.parse().ok()).unwrap_or(30.0);
        Self { total_capital: total, max_positions: max_pos, single_stock_cap_pct: cap, chain_concentration_limit: chain, t1_frozen_warn_ratio: t1 }
    }

    /// 基准仓位（单只股票）
    pub fn base_position(&self) -> f64 {
        self.total_capital / self.max_positions as f64
    }

    /// 动态仓位上限
    pub fn max_position(
        &self,
        regime: MarketRegime,
        volatility_pct: f64,
        chain_positions: usize,      // 同产业链已有持仓数
        chain_frozen: usize,         // 同产业链冻结持仓数
        already_held: bool,           // 该股当前是否已持有
    ) -> f64 {
        if already_held {
            return 0.0; // 禁止当日重复买入同一只
        }

        let base = self.base_position();
        let regime_m = regime.position_multiplier();
        if regime_m == 0.0 {
            return 0.0;
        }

        // 波动率系数：越波动，仓位越小
        let vol_m = (2.0 / volatility_pct.max(1.0)).min(1.0);

        // 集中度折扣：同产业链越多 → 折扣越大，冻结持仓惩罚更重
        let chain_penalty = chain_positions as f64 + chain_frozen as f64 * 1.5;
        let chain_m = (1.0 - 0.2 * chain_penalty).max(0.2);

        let cap = self.total_capital * self.single_stock_cap_pct / 100.0;
        (base * regime_m * vol_m * chain_m).min(cap)
    }

    /// 检查 T+1 冻结仓位风险
    pub fn check_t1_risk(&self, frozen_value: f64) -> Option<String> {
        let frozen_ratio = frozen_value / self.total_capital * 100.0;
        if frozen_ratio >= self.t1_frozen_warn_ratio {
            Some(format!(
                "T+1冻结仓位占比 {:.0}% ≥ {:.0}%，次日集中解禁风险",
                frozen_ratio, self.t1_frozen_warn_ratio
            ))
        } else {
            None
        }
    }

    /// 检查产业链集中度
    pub fn check_chain_concentration(&self, chain_value: f64) -> Option<String> {
        let ratio = chain_value / self.total_capital * 100.0;
        if ratio >= self.chain_concentration_limit {
            Some(format!(
                "产业链集中度 {:.0}% ≥ {:.0}%，建议分散",
                ratio, self.chain_concentration_limit
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_position_type_labels() {
        assert!(PositionType::Available.can_sell_today());
        let locked = PositionType::Locked { unlock_date: NaiveDate::from_ymd_opt(2026, 6, 14).unwrap() };
        assert!(!locked.can_sell_today());
        assert!(locked.is_locked());
    }

    #[test]
    fn test_market_classify_bull() {
        assert_eq!(classify_market(0.8, 1.5), MarketRegime::BullRally);
    }

    #[test]
    fn test_market_classify_crash() {
        assert_eq!(classify_market(0.1, -3.5), MarketRegime::Crash);
    }

    #[test]
    fn test_market_classify_structural() {
        assert_eq!(classify_market(0.5, 0.2), MarketRegime::Structural);
    }

    #[test]
    fn test_market_classify_bear() {
        assert_eq!(classify_market(0.2, -1.0), MarketRegime::BearDecline);
    }

    #[test]
    fn test_stop_loss_effective() {
        let sl = StopLoss::new(10.0, 3.0, Some(9.5));
        // technical = 10 * (1 - 2*3/100) = 9.4
        // hard = 10 * 0.92 = 9.2
        // structural = 9.5 * 0.98 = 9.31
        // effective = max(9.4, 9.31, 9.2) = 9.4
        assert!((sl.effective() - 9.4).abs() < 0.01);
    }

    #[test]
    fn test_stop_loss_triggered() {
        let sl = StopLoss::new(10.0, 3.0, None);
        assert!(sl.triggered(9.0));
        assert!(!sl.triggered(9.5));
    }

    #[test]
    fn test_stop_loss_advice_locked() {
        let sl = StopLoss::new(10.0, 3.0, None);
        let locked = PositionType::Locked { unlock_date: NaiveDate::from_ymd_opt(2026, 6, 14).unwrap() };
        let advice = sl.advice(9.5, locked);
        assert!(advice.contains("T+1锁仓"));
    }

    #[test]
    fn test_position_sizer_base() {
        let sizer = PositionSizer { total_capital: 100_000.0, max_positions: 5, ..Default::default() };
        assert!((sizer.base_position() - 20_000.0).abs() < 0.01);
    }

    #[test]
    fn test_position_sizer_zero_in_crash() {
        let sizer = PositionSizer::default();
        let max = sizer.max_position(MarketRegime::Crash, 3.0, 0, 0, false);
        assert!((max - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_position_sizer_no_double_buy() {
        let sizer = PositionSizer::default();
        let max = sizer.max_position(MarketRegime::Structural, 3.0, 0, 0, true);
        assert!((max - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_position_sizer_chain_penalty() {
        let sizer = PositionSizer::default();
        let max_no_chain = sizer.max_position(MarketRegime::Structural, 3.0, 0, 0, false);
        let max_with_chain = sizer.max_position(MarketRegime::Structural, 3.0, 2, 1, false);
        assert!(max_with_chain < max_no_chain);
    }

    #[test]
    fn test_regime_multipliers() {
        assert!((MarketRegime::BullRally.position_multiplier() - 1.2).abs() < 0.01);
        assert!((MarketRegime::Structural.position_multiplier() - 1.0).abs() < 0.01);
        assert!((MarketRegime::BearDecline.position_multiplier() - 0.5).abs() < 0.01);
        assert!((MarketRegime::Crash.position_multiplier() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_t1_risk_warning() {
        let sizer = PositionSizer { total_capital: 100_000.0, t1_frozen_warn_ratio: 30.0, ..Default::default() };
        assert!(sizer.check_t1_risk(35_000.0).is_some());
        assert!(sizer.check_t1_risk(10_000.0).is_none());
    }

    #[test]
    fn test_chain_concentration_warning() {
        let sizer = PositionSizer { total_capital: 100_000.0, chain_concentration_limit: 40.0, ..Default::default() };
        assert!(sizer.check_chain_concentration(45_000.0).is_some());
        assert!(sizer.check_chain_concentration(20_000.0).is_none());
    }
}
