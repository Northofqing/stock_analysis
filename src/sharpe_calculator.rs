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