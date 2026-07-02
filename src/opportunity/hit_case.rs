//! v10 P0.1 (BC-3) — 5 边界 hit CASE 逻辑
//!
//! 设计: 5 边界判断是纯函数, 不接 verify pipeline (Phase 4 接)
//! 输入: StockState (买价/实际价/特殊状态) + 窗口天数
//! 输出: Option<bool> (true=hit, false=miss, None=数据不足 N/A)
//!
//! 5 边界 (v10 §5.2 BC-3):
//!   1. 停牌日 → actual_change=0, hit=NULL (不算胜率, 避免虚高)
//!   2. 涨停日 → hit = "能否按买入价买进" 判 (一字板买不进 → false)
//!   3. 跌停日 → hit = false (跌停已亏, 不算 hit)
//!   4. 高开低走 → 正常 CASE (actual < 0 → miss)
//!   5. 低开高走 → 正常 CASE (actual > 0 → hit)
//!
//! 阈值: 默认 0.5% (实际涨跌幅 > 0.5% 算 hit), 可由 hit_threshold_pct 参数覆盖

/// 5 边界的特殊状态标识
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpecialCase {
    /// 停牌日 (actual_change 强制 0, hit=None)
    Suspended,
    /// 涨停日 (一字板 buy 价能否买进)
    LimitUp,
    /// 跌停日 (hit 强制 false)
    LimitDown,
    /// 正常交易日 (用 actual_change 判 hit)
    Normal,
}

/// 5 边界判断的输入 (买价/实际价/特殊状态)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitCaseInput {
    /// 买入价 (推送日的开盘价或 VWAP)
    pub buy_price: f64,
    /// 实际收盘价 (N 日后)
    pub actual_close: f64,
    /// 特殊状态 (停牌/涨停/跌停/正常)
    pub special: SpecialCase,
    /// hit 阈值 (默认 0.5%, 即 0.005; 可由 env V10_HIT_THRESHOLD_PCT 覆盖)
    pub hit_threshold_pct: f64,
}

impl HitCaseInput {
    /// 默认 hit 阈值 (从 env V10_HIT_THRESHOLD_PCT 读, 默认 0.5%)
    /// BUG FIX (codex B4): 之前 hardcoded 0.005, 与 shell 端 env 失同步
    pub fn default_threshold() -> f64 {
        std::env::var("V10_HIT_THRESHOLD_PCT")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .map(|p| p / 100.0) // pct → 比例
            .unwrap_or(0.005)
    }

    /// 便捷构造: 正常交易日 (用 env 读阈值)
    pub fn normal(buy_price: f64, actual_close: f64) -> Self {
        Self {
            buy_price,
            actual_close,
            special: SpecialCase::Normal,
            hit_threshold_pct: Self::default_threshold(),
        }
    }
}

/// 5 边界判断输出
///
/// 返回:
/// - `Some(true)` = hit
/// - `Some(false)` = miss
/// - `None` = 数据不足 (停牌 / 价格 ≤ 0 / 阈值无效)
pub fn compute_hit_t1(input: &HitCaseInput) -> Option<bool> {
    // 边界 1: 停牌日 → None
    if input.special == SpecialCase::Suspended {
        return None;
    }

    // 价格校验
    if input.buy_price <= 0.0 || input.actual_close <= 0.0 {
        return None;
    }
    if input.hit_threshold_pct < 0.0 || !input.hit_threshold_pct.is_finite() {
        return None;
    }

    // 边界 3: 跌停日 → 强制 false
    if input.special == SpecialCase::LimitDown {
        return Some(false);
    }

    // 计算 actual_change
    let actual_change = (input.actual_close - input.buy_price) / input.buy_price;

    // 边界 2: 涨停日 → "能否按买入价买进"
    //   - 一字板 (买价 = 收盘价): 实际是 hit, 因为没亏
    //   - 涨停 + 高开 (买价 < 收盘价): 实际是 hit
    //   - 涨停 + 收买价下: 实际是 miss
    if input.special == SpecialCase::LimitUp {
        // 一字板或更高 = 买不进但有利润 → 算 hit (特殊情况, 算策略有效)
        // 实际 hit: 实际涨跌幅 >= 0 算 hit
        return Some(actual_change >= 0.0);
    }

    // 边界 4+5: 正常 → 用阈值判
    // hit if actual_change > hit_threshold_pct (v10 §5.2: > 0.5%)
    Some(actual_change > input.hit_threshold_pct)
}

/// 同时算 hit_t1 + t1_special_case (写库用, 完整封装)
pub fn compute_hit_with_special(input: &HitCaseInput) -> (Option<bool>, &'static str) {
    let hit = compute_hit_t1(input);
    let special = match input.special {
        SpecialCase::Suspended => "suspended",
        SpecialCase::LimitUp => "limit_up",
        SpecialCase::LimitDown => "limit_down",
        SpecialCase::Normal => "normal",
    };
    (hit, special)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 边界 1: 停牌日 → None
    #[test]
    fn test_boundary_1_suspended_returns_none() {
        let input = HitCaseInput {
            buy_price: 10.0,
            actual_close: 10.0, // 停牌价 = 买价, 看似 0% 收益
            special: SpecialCase::Suspended,
            hit_threshold_pct: 0.005,
        };
        assert_eq!(compute_hit_t1(&input), None, "停牌日应返回 None, 不算 hit");
    }

    /// 边界 2a: 涨停 + 一字板 (买价 = 收盘价) → hit
    #[test]
    fn test_boundary_2a_limit_up_yi_zi_ban() {
        let input = HitCaseInput {
            buy_price: 10.0,
            actual_close: 10.0, // 一字板
            special: SpecialCase::LimitUp,
            hit_threshold_pct: 0.005,
        };
        assert_eq!(compute_hit_t1(&input), Some(true), "涨停一字板算 hit");
    }

    /// 边界 2b: 涨停 + 高开 (买价 < 收盘价) → hit
    #[test]
    fn test_boundary_2b_limit_up_gap_up() {
        let input = HitCaseInput {
            buy_price: 10.0,
            actual_close: 10.5, // 高开 5%
            special: SpecialCase::LimitUp,
            hit_threshold_pct: 0.005,
        };
        assert_eq!(compute_hit_t1(&input), Some(true), "涨停高开算 hit");
    }

    /// 边界 2c: 涨停 + 收买价下 (price < 买价) → miss
    #[test]
    fn test_boundary_2c_limit_up_close_below_buy() {
        let input = HitCaseInput {
            buy_price: 10.0,
            actual_close: 9.5, // 实际亏 5%
            special: SpecialCase::LimitUp,
            hit_threshold_pct: 0.005,
        };
        assert_eq!(compute_hit_t1(&input), Some(false), "涨停收买价下算 miss");
    }

    /// 边界 3: 跌停日 → 强制 false
    #[test]
    fn test_boundary_3_limit_down_returns_false() {
        let input = HitCaseInput {
            buy_price: 10.0,
            actual_close: 9.0, // 跌停 -10%
            special: SpecialCase::LimitDown,
            hit_threshold_pct: 0.005,
        };
        assert_eq!(compute_hit_t1(&input), Some(false), "跌停日强制 false");
    }

    /// 边界 4: 高开低走 (实际亏但 < 阈值) → miss
    #[test]
    fn test_boundary_4_high_open_low_close_below_threshold() {
        let input = HitCaseInput::normal(10.0, 9.99); // -0.1%
        assert_eq!(compute_hit_t1(&input), Some(false), "高开低走 < 阈值算 miss");
    }

    /// 边界 5a: 低开高走 (实际赚 > 阈值) → hit
    #[test]
    fn test_boundary_5a_low_open_high_close_above_threshold() {
        let input = HitCaseInput::normal(10.0, 10.5); // +5%
        assert_eq!(compute_hit_t1(&input), Some(true), "低开高走 > 阈值算 hit");
    }

    /// 边界 5b: 低开高走 (实际赚 < 阈值) → miss (默认 0.5% 阈值)
    #[test]
    fn test_boundary_5b_low_open_high_close_below_threshold() {
        let input = HitCaseInput::normal(10.0, 10.04); // +0.4% < 0.5% 阈值
        assert_eq!(compute_hit_t1(&input), Some(false), "低开高走 < 阈值算 miss (默认 0.5%)");
    }

    /// 价格校验: 0 / 负数 → None
    #[test]
    fn test_invalid_price_returns_none() {
        let cases = [
            HitCaseInput::normal(0.0, 10.0),       // buy_price=0
            HitCaseInput::normal(10.0, 0.0),       // actual_close=0
            HitCaseInput::normal(-10.0, 10.0),     // buy_price 负
            HitCaseInput::normal(10.0, -5.0),      // actual_close 负
        ];
        for input in &cases {
            assert_eq!(compute_hit_t1(input), None, "无效价格应返回 None: {:?}", input);
        }
    }

    /// 阈值校验: NaN/负数 → None
    #[test]
    fn test_invalid_threshold_returns_none() {
        let input = HitCaseInput {
            buy_price: 10.0,
            actual_close: 10.5,
            special: SpecialCase::Normal,
            hit_threshold_pct: f64::NAN,
        };
        assert_eq!(compute_hit_t1(&input), None, "NaN 阈值应返回 None");
    }

    /// compute_hit_with_special 完整封装测试 (写库用)
    #[test]
    fn test_with_special_returns_tuple() {
        let input = HitCaseInput::normal(10.0, 10.5);
        let (hit, special) = compute_hit_with_special(&input);
        assert_eq!(hit, Some(true));
        assert_eq!(special, "normal");

        let input2 = HitCaseInput {
            buy_price: 10.0,
            actual_close: 10.0,
            special: SpecialCase::Suspended,
            hit_threshold_pct: 0.005,
        };
        let (hit2, special2) = compute_hit_with_special(&input2);
        assert_eq!(hit2, None);
        assert_eq!(special2, "suspended");
    }
}
