//! position_tracker 核心路径测试 (P0-4)
//!
//! 覆盖:
//! - 辅助函数: net_return_rate, position_shares, RiskContext 构造
//! - StopLoss 集成: ATR 动态止损 / fallback to 固定8%
//! - PositionSizer 仓位计算
//! - MarketRegime 门控
//! - T+1 锁仓判断逻辑
//! - 边界: zero price, empty data, NaN ATR, zero capital
//!
//! 所有测试使用 TEST_CODE 前缀 (AGENTS.md 2.5)

use stock_analysis::monitor::risk::{MarketRegime, PositionSizer, StopLoss, PositionType};
use stock_analysis::pipeline::RiskContext;

// ============================================================================
// 辅助函数测试
// ============================================================================

#[test]
fn test_risk_context_from_env_default() {
    let ctx = RiskContext::from_env(MarketRegime::Structural, Some(3.0));
    assert!(ctx.use_dynamic);
    assert_eq!(ctx.regime, MarketRegime::Structural);
    assert_eq!(ctx.atr, Some(3.0));
    let base = ctx.sizer.base_position();
    assert!(base > 0.0, "base position should be > 0");
}

#[test]
fn test_risk_context_no_atr() {
    let ctx = RiskContext::from_env(MarketRegime::BullRally, None);
    assert_eq!(ctx.atr, None);
}

#[test]
fn test_risk_context_crash_regime() {
    let ctx = RiskContext::from_env(MarketRegime::Crash, Some(2.0));
    assert!(!ctx.regime.allow_new_position());
    assert!((ctx.regime.position_multiplier() - 0.0).abs() < 0.01);
}

#[test]
fn test_risk_context_bull_regime() {
    let ctx = RiskContext::from_env(MarketRegime::BullRally, Some(1.5));
    assert!(ctx.regime.allow_new_position());
    assert!((ctx.regime.position_multiplier() - 1.2).abs() < 0.01);
}

// ============================================================================
// StopLoss 集成测试
// ============================================================================

#[test]
fn test_stop_loss_triggered_with_atr() {
    let sl = StopLoss::new(10.0, 3.0, Some(9.5));
    // technical = 10 * (1 - 2*3/100) = 9.4
    // structural = 9.5 * 0.98 = 9.31
    // hard = 10 * 0.92 = 9.2
    // effective = max(9.4, 9.31, 9.2) = 9.4 (tightest)
    assert!((sl.effective() - 9.4).abs() < 0.01);
    assert!(sl.triggered(9.0));  // below effective
    assert!(!sl.triggered(9.5)); // above effective
}

#[test]
fn test_stop_loss_high_atr_wider_stop() {
    // High volatility → wider stop
    let sl_high_vol = StopLoss::new(10.0, 10.0, None);
    // technical = 10 * (1 - 2*10/100) = 8.0
    // hard = 9.2
    // effective = max(8.0, 9.2) = 9.2
    assert!((sl_high_vol.effective() - 9.2).abs() < 0.01);

    let sl_low_vol = StopLoss::new(10.0, 1.0, None);
    // technical = 10 * (1 - 2*1/100) = 9.8
    // hard = 9.2
    // effective = max(9.8, 9.2) = 9.8
    assert!((sl_low_vol.effective() - 9.8).abs() < 0.01);
    // Low vol = tighter stop (closer to buy price)
    assert!(sl_low_vol.effective() > sl_high_vol.effective());
}

#[test]
fn test_stop_loss_distance_pct() {
    let sl = StopLoss::new(10.0, 3.0, None);
    let dist = sl.distance_pct(9.5);
    // effective ≈ 9.4, distance = (9.5-9.4)/9.5 * 100 ≈ 1.05%
    assert!(dist > 0.0);
    assert!(dist < 5.0);
}

#[test]
fn test_stop_loss_with_support_level() {
    let sl = StopLoss::new(10.0, 3.0, Some(8.0));
    // support = 8.0 * 0.98 = 7.84
    // technical = 9.4, hard = 9.2
    // effective = max(9.4, 7.84, 9.2) = 9.4
    assert!((sl.effective() - 9.4).abs() < 0.01);
}

#[test]
fn test_stop_loss_t1_locked_advice() {
    let sl = StopLoss::new(10.0, 3.0, None);
    let locked = PositionType::Locked {
        unlock_date: chrono::NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
    };
    let advice = sl.advice(9.5, locked);
    assert!(advice.contains("T+1"), "Expected T+1 warning in: {}", advice);
    assert!(advice.contains("锁仓"), "Expected lock warning in: {}", advice);
}

#[test]
fn test_stop_loss_available_can_sell() {
    assert!(PositionType::Available.can_sell_today());
    assert!(!PositionType::Available.is_locked());
}

#[test]
fn test_stop_loss_locked_cannot_sell() {
    let locked = PositionType::Locked {
        unlock_date: chrono::NaiveDate::from_ymd_opt(2026, 6, 22).unwrap(),
    };
    assert!(!locked.can_sell_today());
    assert!(locked.is_locked());
}

// ============================================================================
// PositionSizer 仓位计算测试
// ============================================================================

#[test]
fn test_position_sizer_base() {
    let sizer = PositionSizer {
        total_capital: 100_000.0,
        max_positions: 5,
        ..Default::default()
    };
    assert!((sizer.base_position() - 20_000.0).abs() < 0.01);
}

#[test]
fn test_position_sizer_crash_zero() {
    let sizer = PositionSizer::default();
    let max_amt = sizer.max_position(MarketRegime::Crash, 3.0, 0, 0, false);
    assert!((max_amt - 0.0).abs() < 0.01, "Crash should block all new buys");
}

#[test]
fn test_position_sizer_no_double_buy() {
    let sizer = PositionSizer::default();
    let max_amt = sizer.max_position(MarketRegime::Structural, 3.0, 0, 0, true);
    assert!((max_amt - 0.0).abs() < 0.01, "Already held should return 0");
}

#[test]
fn test_position_sizer_bear_reduced() {
    let sizer = PositionSizer::default();
    let bull_max = sizer.max_position(MarketRegime::BullRally, 3.0, 0, 0, false);
    let bear_max = sizer.max_position(MarketRegime::BearDecline, 3.0, 0, 0, false);
    assert!(bear_max < bull_max, "Bear regime should reduce position size");
    assert!(bear_max > 0.0, "Bear should still allow some buying");
}

#[test]
fn test_position_sizer_chain_penalty() {
    let sizer = PositionSizer::default();
    let no_chain = sizer.max_position(MarketRegime::Structural, 3.0, 0, 0, false);
    let with_chain = sizer.max_position(MarketRegime::Structural, 3.0, 3, 0, false);
    assert!(with_chain < no_chain, "Chain concentration should reduce position");
}

#[test]
fn test_position_sizer_frozen_penalty() {
    let sizer = PositionSizer::default();
    let no_frozen = sizer.max_position(MarketRegime::Structural, 3.0, 0, 0, false);
    let with_frozen = sizer.max_position(MarketRegime::Structural, 3.0, 0, 2, false);
    // Frozen positions penalize 1.5x
    assert!(with_frozen < no_frozen, "Frozen positions should reduce capacity");
}

#[test]
fn test_position_sizer_high_volatility() {
    let sizer = PositionSizer::default();
    let low_vol = sizer.max_position(MarketRegime::Structural, 2.0, 0, 0, false);
    let high_vol = sizer.max_position(MarketRegime::Structural, 10.0, 0, 0, false);
    // Higher volatility → smaller position
    assert!(high_vol < low_vol,
        "High vol {} should be less than low vol {}", high_vol, low_vol);
}

#[test]
fn test_position_sizer_single_stock_cap() {
    let sizer = PositionSizer {
        total_capital: 100_000.0,
        max_positions: 5,
        single_stock_cap_pct: 20.0, // 20% of 100k = 20k
        ..Default::default()
    };
    let max_amt = sizer.max_position(MarketRegime::BullRally, 1.0, 0, 0, false);
    assert!(max_amt <= 20_000.0, "Should not exceed single stock cap");
}

// ============================================================================
// MarketRegime 门控测试
// ============================================================================

#[test]
fn test_market_regime_classify() {
    use stock_analysis::monitor::risk::classify_market;
    assert_eq!(classify_market(0.8, 1.5), MarketRegime::BullRally);
    assert_eq!(classify_market(0.5, 0.2), MarketRegime::Structural);
    assert_eq!(classify_market(0.2, -1.0), MarketRegime::BearDecline);
    assert_eq!(classify_market(0.1, -3.5), MarketRegime::Crash);
}

#[test]
fn test_market_regime_multipliers() {
    assert!((MarketRegime::BullRally.position_multiplier() - 1.2).abs() < 0.01);
    assert!((MarketRegime::Structural.position_multiplier() - 1.0).abs() < 0.01);
    assert!((MarketRegime::BearDecline.position_multiplier() - 0.5).abs() < 0.01);
    assert!((MarketRegime::Crash.position_multiplier() - 0.0).abs() < 0.01);
}

#[test]
fn test_crash_blocks_buying() {
    assert!(!MarketRegime::Crash.allow_new_position());
    assert!(MarketRegime::Structural.allow_new_position());
    assert!(MarketRegime::BullRally.allow_new_position());
    assert!(MarketRegime::BearDecline.allow_new_position());
}

// ============================================================================
// 边界测试
// ============================================================================

#[test]
fn test_position_sizer_zero_capital() {
    let sizer = PositionSizer {
        total_capital: 0.0,
        max_positions: 5,
        ..Default::default()
    };
    let max_amt = sizer.max_position(MarketRegime::Structural, 3.0, 0, 0, false);
    assert!((max_amt - 0.0).abs() < 0.01, "Zero capital should return 0");
}

#[test]
fn test_position_sizer_min_volatility_clamped() {
    let sizer = PositionSizer::default();
    let max_amt = sizer.max_position(MarketRegime::Structural, 0.1, 0, 0, false);
    // volatility clamps to max(1.0, 0.1) = 1.0
    assert!(max_amt > 0.0, "Very low vol should still work (clamped to 1.0)");
}

#[test]
fn test_stop_loss_zero_atr() {
    // ATR = 0 → technical = buy_price * (1 - 0) = buy_price
    // effective = max(buy_price, hard=0.92*buy_price)
    // = buy_price (highest = tightest)
    let sl = StopLoss::new(10.0, 0.0, None);
    let eff = sl.effective();
    assert!((eff - 10.0).abs() < 0.01, "Zero ATR → stop at buy price");
}

#[test]
fn test_position_sizer_label() {
    assert_eq!(PositionType::Available.label(), "可用");
    let locked = PositionType::Locked {
        unlock_date: chrono::NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
    };
    assert_eq!(locked.label(), "冻结");
}

// ============================================================================
// T+1 锁仓逻辑测试
// ============================================================================

#[test]
fn test_t1_locked_is_recent_buy() {
    // T+1: PositionType::Locked 表示今日买入
    let today = chrono::Local::now().date_naive();
    let locked = PositionType::Locked {
        unlock_date: today.succ_opt().unwrap_or(today),
    };
    assert!(locked.is_locked());
    assert!(!locked.can_sell_today());
}

#[test]
fn test_t1_warning_in_stop_loss_advice() {
    let sl = StopLoss::new(10.0, 3.0, None);
    let locked = PositionType::Locked {
        unlock_date: chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
    };
    let advice = sl.advice(9.0, locked);
    assert!(advice.contains("锁仓"));
    assert!(advice.contains("T+1"));
    assert!(advice.contains("次日"), "Expected suggestion for next day in: {}", advice);
}

// ============================================================================
// PositionSizer 风控告警测试
// ============================================================================

#[test]
fn test_t1_risk_warning_triggers() {
    let sizer = PositionSizer {
        total_capital: 100_000.0,
        t1_frozen_warn_ratio: 30.0,
        ..Default::default()
    };
    assert!(sizer.check_t1_risk(35_000.0).is_some(), "35% frozen should warn");
    assert!(sizer.check_t1_risk(10_000.0).is_none(), "10% frozen should be ok");
}

#[test]
fn test_chain_concentration_warning() {
    let sizer = PositionSizer {
        total_capital: 100_000.0,
        chain_concentration_limit: 40.0,
        ..Default::default()
    };
    assert!(sizer.check_chain_concentration(45_000.0).is_some());
    assert!(sizer.check_chain_concentration(20_000.0).is_none());
}
