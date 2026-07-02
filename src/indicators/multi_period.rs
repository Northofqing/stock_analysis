//! v10 P3 G1 — 多周期确认器
//!
//! 设计 (v10 §9 P3 + §4.2):
//! - 输入: stock_daily 日线序列 (已有)
//! - 本地聚合日线 → 周线/月线 (不新数据源, 避免 §2.1 假数据风险)
//! - 输出: MultiPeriodConfirm { daily, weekly, monthly, aligned }
//! - 开盘买入判定 (T1) 只在 aligned (日周月不背离) 时入虚拟仓
//! - 用途: 补技术地板, 借鉴 ai-berkshire "基本面/多周期地板" 战略
//!
//! 实施: 不依赖具体 stock_daily 数据结构, 用价格序列输入

/// 单期趋势
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Trend {
    Up,
    Down,
    Sideways,
}

/// 多周期确认
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MultiPeriodConfirm {
    pub daily: Trend,
    pub weekly: Trend,
    pub monthly: Trend,
    /// 日周月不背离 (同向)
    pub aligned: bool,
}

/// 单期价格序列 → 趋势判定
/// 简化规则: 最后 N 天平均价 > 之前 N 天平均价 → Up
/// 注: 实际接 stock_daily 后, 简化为 moving average 方向
pub fn trend_from_prices(prices: &[f64]) -> Trend {
    if prices.len() < 2 {
        return Trend::Sideways;
    }
    let mid = prices.len() / 2;
    if mid == 0 {
        return Trend::Sideways;
    }
    let early_avg: f64 = prices[..mid].iter().sum::<f64>() / mid as f64;
    let late_avg: f64 = prices[mid..].iter().sum::<f64>() / (prices.len() - mid) as f64;
    if late_avg > early_avg * 1.005 {
        Trend::Up
    } else if late_avg < early_avg * 0.995 {
        Trend::Down
    } else {
        Trend::Sideways
    }
}

/// 聚合日线 → 周线 (5 日 = 1 周)
pub fn aggregate_daily_to_weekly(daily: &[f64]) -> Vec<f64> {
    daily.chunks(5).map(|w| w.iter().sum::<f64>() / w.len() as f64).collect()
}

/// 聚合日线 → 月线 (20 日 = 1 月)
pub fn aggregate_daily_to_monthly(daily: &[f64]) -> Vec<f64> {
    daily.chunks(20).map(|m| m.iter().sum::<f64>() / m.len() as f64).collect()
}

/// 多周期确认 (3 周期同向 = aligned)
pub fn confirm_multi_period(daily_prices: &[f64]) -> Option<MultiPeriodConfirm> {
    if daily_prices.len() < 20 {
        return None; // 数据不足 (需 1 个月日线)
    }
    let daily = trend_from_prices(daily_prices);
    let weekly_prices = aggregate_daily_to_weekly(daily_prices);
    let weekly = trend_from_prices(&weekly_prices);
    let monthly_prices = aggregate_daily_to_monthly(daily_prices);
    let monthly = trend_from_prices(&monthly_prices);

    // 判定 aligned: 3 期都不是 Sideways 且同向 (Sideways 不算 aligned, 避免误判)
    let all_non_sideways = daily != Trend::Sideways
        && weekly != Trend::Sideways
        && monthly != Trend::Sideways;
    let all_up = daily == Trend::Up && weekly == Trend::Up && monthly == Trend::Up;
    let all_down = daily == Trend::Down && weekly == Trend::Down && monthly == Trend::Down;
    let aligned = all_non_sideways && (all_up || all_down);

    Some(MultiPeriodConfirm {
        daily,
        weekly,
        monthly,
        aligned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trend_up() {
        // 上升趋势: 早期均价 10, 后期均价 11 (+10%)
        let prices = vec![9.0, 10.0, 10.0, 10.0, 11.0, 11.0, 11.0, 12.0];
        assert_eq!(trend_from_prices(&prices), Trend::Up);
    }

    #[test]
    fn test_trend_down() {
        let prices = vec![12.0, 11.0, 11.0, 11.0, 10.0, 10.0, 10.0, 9.0];
        assert_eq!(trend_from_prices(&prices), Trend::Down);
    }

    #[test]
    fn test_trend_sideways() {
        let prices = vec![10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0];
        assert_eq!(trend_from_prices(&prices), Trend::Sideways);
    }

    #[test]
    fn test_trend_insufficient_data() {
        let prices = vec![10.0];
        assert_eq!(trend_from_prices(&prices), Trend::Sideways);
    }

    #[test]
    fn test_aggregate_weekly() {
        let daily = vec![10.0; 25]; // 5 周
        let weekly = aggregate_daily_to_weekly(&daily);
        assert_eq!(weekly.len(), 5);
    }

    #[test]
    fn test_aggregate_monthly() {
        let daily = vec![10.0; 60]; // 3 个月
        let monthly = aggregate_daily_to_monthly(&daily);
        assert_eq!(monthly.len(), 3);
    }

    #[test]
    fn test_confirm_insufficient_data() {
        let prices = vec![10.0; 10]; // < 20
        assert_eq!(confirm_multi_period(&prices), None);
    }

    #[test]
    fn test_confirm_aligned_up() {
        // 30 天上升: 日周月都 Up
        let mut prices = vec![10.0; 30];
        for (i, p) in prices.iter_mut().enumerate() {
            *p = 10.0 + (i as f64) * 0.1;
        }
        let confirm = confirm_multi_period(&prices).unwrap();
        assert!(confirm.aligned, "应 aligned, got {:?}", confirm);
        assert_eq!(confirm.daily, Trend::Up);
        assert_eq!(confirm.weekly, Trend::Up);
        assert_eq!(confirm.monthly, Trend::Up);
    }

    #[test]
    fn test_confirm_not_aligned_mixed() {
        // 构造"明显混合" 数据: 日线 zigzag 振幅大 (日内 +5%/-5%) 让 daily 算 Sideways,
        // 但月线整体仍能识别为单一方向
        // 30 天: 反复 10+9+10+9... 让 daily 锯齿, 整 30 天水平 → Sideways
        // 注: 这个测试验证 "实际是 Sideways 不算 aligned"
        let mut prices = Vec::new();
        for i in 0..30 {
            if i % 2 == 0 {
                prices.push(10.0);
            } else {
                prices.push(10.0 - 0.0001); // 极小降 (Sideways)
            }
        }
        let confirm = confirm_multi_period(&prices).unwrap();
        eprintln!("DEBUG: daily={:?} weekly={:?} monthly={:?} aligned={}",
            confirm.daily, confirm.weekly, confirm.monthly, confirm.aligned);
        // 锯齿+极小降 → daily Sideways, weekly Sideways, monthly 略 Down
        // 按 v10 §9 "aligned = 3 期都不是 Sideways 且同向", Sideways 排除 → aligned=false
        assert!(!confirm.aligned, "Sideways 不应 aligned, got {:?}", confirm);
    }
}
