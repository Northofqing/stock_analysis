//! 资金验证 — RS 相对强度 + 量能位置判断。

use crate::data_provider::KlineData;

#[derive(Debug, Clone)]
pub struct CapitalSignal {
    pub code: String,
    pub name: String,
    pub rs_vs_index: Option<f64>,
    pub volume_phase: VolumePhase,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumePhase {
    LaunchBreakout,  // 首次放量启动
    Normal,
    TopDistribution, // 高位持续放量 → 警惕
}

impl VolumePhase {
    pub fn label(&self) -> &'static str {
        match self {
            VolumePhase::LaunchBreakout => "启动放量",
            VolumePhase::Normal => "正常换手",
            VolumePhase::TopDistribution => "高位放量⚠️",
        }
    }
}

/// 计算 RS 相对强度：个股 N 日收益 vs 指数 N 日收益
/// 返回 -100~+100，正数 = 跑赢指数
pub fn compute_rs(stock_kline: &[KlineData], index_kline: &[KlineData], days: usize) -> Option<f64> {
    if stock_kline.len() < days || index_kline.len() < days { return None; }

    let stock_start = stock_kline[stock_kline.len() - days].close;
    let stock_end = stock_kline.last()?.close;
    let index_start = index_kline[index_kline.len() - days].close;
    let index_end = index_kline.last()?.close;

    if stock_start <= 0.0 || index_start <= 0.0 { return None; }

    let stock_ret = (stock_end - stock_start) / stock_start * 100.0;
    let index_ret = (index_end - index_start) / index_start * 100.0;
    Some(stock_ret - index_ret)
}

/// 判断量能所处阶段
pub fn classify_volume(kline: &[KlineData]) -> VolumePhase {
    if kline.len() < 20 { return VolumePhase::Normal; }

    let recent_vol: f64 = kline.iter().rev().take(5).map(|k| k.volume).sum::<f64>() / 5.0;
    let old_vol: f64 = kline.iter().rev().skip(5).take(15).map(|k| k.volume).sum::<f64>() / 15.0;

    if old_vol <= 0.0 { return VolumePhase::Normal; }
    let ratio = recent_vol / old_vol;

    // 最近价格走势
    let recent_price: f64 = kline.iter().rev().take(5).map(|k| k.close).sum::<f64>() / 5.0;
    let mid_price: f64 = kline.iter().rev().take(20).map(|k| k.close).sum::<f64>() / 20.0;
    let price_position = if mid_price > 0.0 { (recent_price - mid_price) / mid_price * 100.0 } else { 0.0 };

    if ratio > 2.0 && price_position < 10.0 {
        VolumePhase::LaunchBreakout  // 放量但价格未远离均线 → 启动
    } else if ratio > 2.0 && price_position > 20.0 {
        VolumePhase::TopDistribution // 放量且价格已大幅高于均线 → 警惕
    } else {
        VolumePhase::Normal
    }
}

/// 对持仓股做资金验证
pub fn verify_holdings(
    holdings: &[crate::portfolio::Position],
    stock_klines: &std::collections::HashMap<String, Vec<KlineData>>,
    index_kline: &[KlineData],
) -> Vec<CapitalSignal> {
    holdings.iter().filter_map(|p| {
        let kline = stock_klines.get(&p.code)?;
        Some(CapitalSignal {
            code: p.code.clone(),
            name: p.name.clone(),
            rs_vs_index: compute_rs(kline, index_kline, 10),
            volume_phase: classify_volume(kline),
            note: String::new(),
        })
    }).collect()
}

/// 格式化资金验证结果
pub fn format_capital_signals(signals: &[CapitalSignal]) -> String {
    if signals.is_empty() { return String::new(); }
    let mut lines = vec!["💰 资金验证".to_string()];
    for s in signals {
        let rs_str = match s.rs_vs_index {
            Some(v) if v > 0.0 => format!("RS+{:.1}", v),
            Some(v) => format!("RS{:.1}", v),
            None => "RS——".to_string(),
        };
        let warn = if s.volume_phase == VolumePhase::TopDistribution { " ⚠️追高风险" } else { "" };
        lines.push(format!("  {}({}) {} {}{}", s.name, s.code, rs_str, s.volume_phase.label(), warn));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn kline(close: f64, volume: f64) -> KlineData {
        KlineData {
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            open: close, high: close, low: close, close, volume,
            amount: 0.0, pct_chg: 0.0,
            pe_ratio: None, pb_ratio: None, turnover_rate: None,
            market_cap: None, circulating_cap: None,
            eps: None, roe: None, revenue_yoy: None, net_profit_yoy: None,
            gross_margin: None, net_margin: None, sharpe_ratio: None,
            financials_history: None, valuation_history: None,
            consensus: None, industry: None,
            is_limit_up: false, is_limit_down: false, is_suspended: false,
        }
    }

    #[test]
    fn test_rs_calculation() {
        // 股票涨 20%，指数涨 5% → RS = +15
        let stock = vec![kline(100.0, 1e6), kline(120.0, 1e6)];
        let index = vec![kline(3000.0, 1e9), kline(3150.0, 1e9)];
        let rs = compute_rs(&stock, &index, 2);
        assert!(rs.is_some());
        assert!((rs.unwrap() - 15.0).abs() < 0.1);
    }

    #[test]
    fn test_rs_insufficient_data() {
        let stock = vec![kline(100.0, 1e6)];
        let index = vec![kline(3000.0, 1e9)];
        assert!(compute_rs(&stock, &index, 5).is_none());
    }

    #[test]
    fn test_volume_normal_when_flat() {
        let klines: Vec<KlineData> = (0..20).map(|_| kline(100.0, 1e6)).collect();
        assert_eq!(classify_volume(&klines), VolumePhase::Normal);
    }
}
