//! 夏普比率计算模块
//!
//! 提供夏普比率（Sharpe Ratio）的计算功能
//! 夏普比率 = (平均收益率 - 无风险利率) / 收益率标准差

use crate::data_provider::KlineData;

/// 计算夏普比率
///
/// # 参数
/// - kline_data: K线数据，至少需要20个交易日数据
/// - risk_free_rate: 无风险利率（年化），默认使用3% (0.03)
/// - trading_days_per_year: 一年交易日数，默认252天
///
/// # 返回
/// - Some(sharpe_ratio): 夏普比率值
/// - None: 数据不足或计算失败
pub fn calculate_sharpe_ratio(
    kline_data: &[KlineData],
    risk_free_rate: Option<f64>,
    trading_days_per_year: Option<usize>,
) -> Option<f64> {
    // 至少需要20个交易日数据
    if kline_data.len() < 20 {
        return None;
    }

    let risk_free = risk_free_rate.unwrap_or(0.03); // 默认3%年化无风险利率
    let trading_days = trading_days_per_year.unwrap_or(252) as f64;

    // 提取每日收益率
    let mut returns = Vec::new();
    for i in 1..kline_data.len() {
        let prev_close = kline_data[i - 1].close;
        let curr_close = kline_data[i].close;
        if prev_close > 0.0 {
            let daily_return = (curr_close - prev_close) / prev_close;
            returns.push(daily_return);
        }
    }

    if returns.is_empty() {
        return None;
    }

    // 计算平均收益率
    let mean_return: f64 = returns.iter().sum::<f64>() / returns.len() as f64;

    // 计算收益率标准差
    let variance: f64 = returns
        .iter()
        .map(|r| {
            let diff = r - mean_return;
            diff * diff
        })
        .sum::<f64>()
        / returns.len() as f64;

    let std_dev = variance.sqrt();

    // 避免除以零
    if std_dev == 0.0 {
        return None;
    }

    // 计算年化收益率和年化标准差
    let annualized_return = mean_return * trading_days;
    let annualized_std_dev = std_dev * trading_days.sqrt();

    // 计算夏普比率
    let sharpe = (annualized_return - risk_free) / annualized_std_dev;

    Some(sharpe)
}

/// 计算滚动窗口夏普比率
///
/// # 参数
/// - kline_data: K线数据
/// - window_size: 窗口大小（交易日数），默认60天（约3个月）
/// - risk_free_rate: 无风险利率（年化）
///
/// # 返回
/// - Some(sharpe_ratio): 最新的滚动窗口夏普比率
/// - None: 数据不足或计算失败
pub fn calculate_rolling_sharpe(
    kline_data: &[KlineData],
    window_size: Option<usize>,
    risk_free_rate: Option<f64>,
) -> Option<f64> {
    let window = window_size.unwrap_or(60);
    
    if kline_data.len() < window {
        // 数据不足，使用全部数据计算
        return calculate_sharpe_ratio(kline_data, risk_free_rate, None);
    }

    // 使用最近的窗口数据
    let recent_data = &kline_data[kline_data.len() - window..];
    calculate_sharpe_ratio(recent_data, risk_free_rate, None)
}

/// 批量计算并更新K线数据的夏普比率
///
/// # 参数
/// - kline_data: 可变的K线数据引用
/// - window_size: 窗口大小，默认60天
/// - risk_free_rate: 无风险利率，默认3%
pub fn update_sharpe_ratios(
    kline_data: &mut [KlineData],
    window_size: Option<usize>,
    risk_free_rate: Option<f64>,
) {
    let window = window_size.unwrap_or(60);
    let risk_free = risk_free_rate.unwrap_or(0.03);

    // 为每个数据点计算夏普比率（使用向前滚动窗口）
    for i in 0..kline_data.len() {
        if i + 1 < 20 {
            // 数据不足，跳过
            kline_data[i].sharpe_ratio = None;
            continue;
        }

        let start_idx = if i + 1 >= window { i + 1 - window } else { 0 };
        let window_data = &kline_data[start_idx..=i];
        
        kline_data[i].sharpe_ratio = calculate_sharpe_ratio(
            window_data,
            Some(risk_free),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn create_test_data(prices: Vec<f64>) -> Vec<KlineData> {
        use chrono::Duration;
        let base_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        
        prices
            .iter()
            .enumerate()
            .map(|(i, &price)| {
                let pct_chg = if i > 0 {
                    (price - prices[i - 1]) / prices[i - 1] * 100.0
                } else {
                    0.0
                };

                KlineData {
                    date: base_date + Duration::days(i as i64),
                    open: price,
                    high: price * 1.02,
                    low: price * 0.98,
                    close: price,
                    volume: 1000000.0,
                    amount: price * 1000000.0,
                    pct_chg,
                    pe_ratio: None,
                    pb_ratio: None,
                    turnover_rate: None,
                    market_cap: None,
                    circulating_cap: None,
                    eps: None,
                    roe: None,
                    revenue_yoy: None,
                    net_profit_yoy: None,
                    gross_margin: None,
                    net_margin: None,
                    sharpe_ratio: None,
                }
            })
            .collect()
    }

    #[test]
    fn test_sharpe_ratio_uptrend() {
        // 创建上涨趋势的测试数据
        let prices: Vec<f64> = (0..30).map(|i| 10.0 + i as f64 * 0.1).collect();
        let data = create_test_data(prices);

        let sharpe = calculate_sharpe_ratio(&data, Some(0.03), Some(252));
        assert!(sharpe.is_some(), "应该能计算出夏普比率");

        if let Some(ratio) = sharpe {
            assert!(ratio > 0.0, "上涨趋势的夏普比率应该为正");
            println!("上涨趋势夏普比率: {:.4}", ratio);
        }
    }

    #[test]
    fn test_sharpe_ratio_downtrend() {
        // 创建下跌趋势的测试数据
        let prices: Vec<f64> = (0..30).map(|i| 15.0 - i as f64 * 0.1).collect();
        let data = create_test_data(prices);

        let sharpe = calculate_sharpe_ratio(&data, Some(0.03), Some(252));
        assert!(sharpe.is_some());

        if let Some(ratio) = sharpe {
            assert!(ratio < 0.0, "下跌趋势的夏普比率应该为负");
            println!("下跌趋势夏普比率: {:.4}", ratio);
        }
    }

    #[test]
    fn test_insufficient_data() {
        let prices: Vec<f64> = vec![10.0, 10.1, 10.2];
        let data = create_test_data(prices);

        let sharpe = calculate_sharpe_ratio(&data, Some(0.03), Some(252));
        assert!(sharpe.is_none(), "数据不足应该返回None");
    }

    #[test]
    fn test_rolling_sharpe() {
        let prices: Vec<f64> = (0..100).map(|i| 10.0 + (i as f64 * 0.05).sin()).collect();
        let data = create_test_data(prices);

        let sharpe = calculate_rolling_sharpe(&data, Some(60), Some(0.03));
        assert!(sharpe.is_some(), "应该能计算滚动夏普比率");
    }
}
