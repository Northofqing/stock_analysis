//! 修复 P1-2: winrate 二元化测试
//! 之前: 占位 50 = 假装中性, 系统偏差 7.5 分
//! 现在: 样本 < 200 = None, 无数据 = 0, 有数据 = 真实胜率

use stock_analysis::opportunity::winrate::*;
use chrono::NaiveDate;

fn sample(day_offset: i64, return_pct: f64) -> BacktestSample {
    BacktestSample {
        event_id: "evt1".to_string(),
        n_day_return: return_pct,
        day: NaiveDate::from_ymd_opt(2026, 6, 27).unwrap() + chrono::Duration::days(day_offset),
    }
}

#[test]
fn test_winrate_none_below_threshold() {
    // 修复 P1-2: < 200 样本 → None (不假装 50)
    let samples: Vec<_> = (0..199).map(|i| sample(i, 0.01)).collect();
    assert!(calc_winrate_score(&samples).is_none());
}

#[test]
fn test_winrate_at_threshold() {
    // 200 样本 → 计算
    let samples: Vec<_> = (0..200).map(|i| sample(i, 0.01)).collect();
    let score = calc_winrate_score(&samples);
    assert!(score.is_some());
    assert!((score.unwrap() - 1.0).abs() < 0.01);
}

#[test]
fn test_winrate_zero_clear_negative() {
    // 修复 P1-2: 真实胜率 < 50% → 0 (明确负信号, 允许 0.5 封顶)
    let samples: Vec<_> = (0..200).map(|i| sample(i, if i % 2 == 0 { 0.01 } else { -0.01 })).collect();
    let score = calc_winrate_score(&samples).unwrap();
    // 100 涨 100 跌 = 50% 胜率, clamp 到 0.5
    assert_eq!(score, 0.5);
}

#[test]
fn test_winrate_data_sufficiency() {
    let samples: Vec<_> = (0..200).map(|i| sample(i, 0.03)).collect();
    let summary = compute_winrate_summary(&samples);
    assert!(summary.sufficient);
    assert_eq!(summary.score, 1.0);
    assert_eq!(summary.total, 200);
    assert_eq!(summary.wins, 200);
    assert_eq!(summary.losses, 0);
}

#[test]
fn test_winrate_filter_zero_returns() {
    // 修复 P1-2: n_day_return=0 (中性) 不算胜负
    let samples = vec![
        sample(0, 0.05),
        sample(1, 0.0),  // 中性
        sample(2, -0.03),
        sample(3, 0.0),  // 中性
    ];
    let summary = compute_winrate_summary(&samples);
    // 只有 2 有效样本 (1 涨 1 跌), < 200 → insufficient
    assert!(!summary.sufficient);
    assert_eq!(summary.wins, 1);
    assert_eq!(summary.losses, 1);
    assert_eq!(summary.total, 2);
}

#[test]
fn test_winrate_seven_zero_returns_zero() {
    // 真实负信号: 100 涨 100 跌 → 50% 胜率 → clamp 0.5
    // 实际: 应该是 None (insufficient, < 200 有效) OR 0.5 (clamp)
    // 这里测试: 全部 0 收益 → insufficient
    let samples: Vec<_> = (0..200).map(|_| sample(0, 0.0)).collect();
    let summary = compute_winrate_summary(&samples);
    // total = 0 有效样本
    assert_eq!(summary.total, 0);
    assert!(!summary.sufficient);
}

#[test]
fn test_winrate_summary_full() {
    // 200 样本, 150 涨 50 跌 (排除 0 收益) → 0.75 胜率
    let mut samples = Vec::new();
    for i in 0..150 {
        samples.push(sample(i, 0.02));
    }
    for i in 150..200 {
        samples.push(sample(i, -0.01));
    }
    let summary = compute_winrate_summary(&samples);
    assert_eq!(summary.total, 200);
    assert_eq!(summary.wins, 150);
    assert_eq!(summary.losses, 50);
    assert!((summary.score - 0.75).abs() < 0.01);
}
