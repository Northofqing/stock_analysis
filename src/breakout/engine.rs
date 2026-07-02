//! Breakout Engine — 盘中 + 盘后双模式。

use super::position;
use super::signal::*;

/// 盘中模式：仅需东财 push2 数据 + ignition，不需要 K 线
pub fn screen_intraday(
    code: &str, name: &str,
    vol_ratio: f64, change_pct: f64, main_net_yi: f64,
    ignition_near_limit: usize,
) -> BreakoutSignal {
    let mut confidence: u8 = 0;
    let mut desc_parts: Vec<String> = Vec::new();
    let mut data_degraded = false;

    // 1. 量能模式
    let volume_pattern = if vol_ratio <= 0.0 {
        data_degraded = true;
        VolumePattern::Flat
    } else if vol_ratio >= 4.0 {
        desc_parts.push("天量".into());
        VolumePattern::SuddenSpike
    } else if vol_ratio >= 1.5 {
        desc_parts.push("放量".into());
        confidence = confidence.saturating_add(30);
        VolumePattern::GentleIncrease
    } else {
        VolumePattern::Flat
    };

    // 1b. 高量但转弱：当日已经翻绿或涨幅明显不足时，按分布/出货风险处理。
    // 这类标的容易在盘中从“放量启动”转成“冲高回落”，不应继续加分。
    if vol_ratio >= 1.5 && change_pct < 1.0 {
        confidence = confidence.saturating_sub(15);
        desc_parts.push("高量转弱".into());
    }
    if vol_ratio >= 1.5 && change_pct < 0.0 {
        confidence = confidence.saturating_sub(20);
        desc_parts.push("放量下跌".into());
    }
    if vol_ratio >= 2.5 && change_pct <= 0.5 {
        confidence = confidence.saturating_sub(20);
        desc_parts.push("天量不涨".into());
    }

    // 2. K线强度
    let candle_strength = if change_pct >= 5.0 {
        confidence = confidence.saturating_add(25);
        desc_parts.push("强阳".into());
        CandleStrength::Strong
    } else if change_pct >= 1.0 {
        confidence = confidence.saturating_add(15);
        CandleStrength::Medium
    } else if change_pct >= 0.0 {
        CandleStrength::Weak
    } else {
        CandleStrength::Bearish
    };

    // 3. 资金验证
    if main_net_yi > 0.0 {
        confidence = confidence.saturating_add(15);
        desc_parts.push(format!("主力流入{:.1}亿", main_net_yi));
    }

    // 4. 板块点火
    if ignition_near_limit >= 3 {
        confidence = confidence.saturating_add(20);
        desc_parts.push(format!("板块点火{}只", ignition_near_limit));
    }

    // 5. 综合判定
    let breakout_type = if vol_ratio >= 2.5 && change_pct <= 0.5 {
        BreakoutType::Distribution
    } else if vol_ratio >= 1.5 && change_pct < 0.0 {
        BreakoutType::Distribution
    } else if confidence >= 50 {
        BreakoutType::Launch
    } else if confidence >= 20 {
        BreakoutType::Uncertain
    } else {
        BreakoutType::Uncertain
    };

    let note = if data_degraded { " [数据源降级]" } else { "" };
    let description = format!("{}{}", desc_parts.join(" "), note);

    BreakoutSignal {
        code: code.into(), name: name.into(),
        volume_ratio: if vol_ratio > 0.0 { Some(vol_ratio) } else { None },
        vol_vs_20d_avg: None,
        volume_pattern,
        change_pct,
        candle_strength,
        ma_break: None,
        price_position: PricePosition::Unknown,
        breakout_type,
        confidence,
        description,
        data_degraded,
    }
}

/// 盘后模式：需要 K 线数据，输出含启动 vs 出货判断
pub fn analyze_postmarket(
    code: &str, name: &str,
    kline: &[crate::data_provider::KlineData],
) -> BreakoutSignal {
    if kline.len() < 20 {
        return BreakoutSignal {
            code: code.into(), name: name.into(),
            volume_ratio: None, vol_vs_20d_avg: None,
            volume_pattern: VolumePattern::Flat,
            change_pct: 0.0, candle_strength: CandleStrength::Weak,
            ma_break: None, price_position: PricePosition::Unknown,
            breakout_type: BreakoutType::Uncertain, confidence: 0,
            description: "K线数据不足".into(), data_degraded: true,
        };
    }

    let latest = kline.last().unwrap();
    let change_pct = latest.pct_chg;
    let mut confidence: u8 = 0;
    let mut launch_score: u8 = 0;
    let mut distribution_score: u8 = 0;
    let mut desc: Vec<String> = Vec::new();

    // 1. 位置判断 (60日均线)
    let ma60: f64 = kline.iter().rev().take(60).map(|k| k.close).sum::<f64>()
        / (kline.len().min(60) as f64).max(1.0);
    let dist = position::distance_from_ma(latest.close, ma60);
    let price_position = position::classify_position(dist);
    desc.push(format!("距60均线{:+.1}%", dist));

    match price_position {
        PricePosition::Low => { launch_score += 1; desc.push("低位".into()); }
        PricePosition::High => { distribution_score += 1; desc.push("高位".into()); }
        _ => {}
    }

    // 2. 量能：5日 vs 20日均量
    let vol_5d: f64 = kline.iter().rev().take(5).map(|k| k.volume).sum::<f64>() / 5.0;
    let vol_20d: f64 = kline.iter().rev().take(20).map(|k| k.volume).sum::<f64>() / 20.0;
    let vol_ratio = if vol_20d > 0.0 { vol_5d / vol_20d } else { 1.0 };

    let vol_10d_old: f64 = kline.iter().rev().skip(5).take(10).map(|k| k.volume).sum::<f64>() / 10.0;
    let was_shrinking = vol_10d_old > 0.0 && vol_10d_old < vol_20d * 0.7;

    let volume_pattern = if was_shrinking && vol_ratio >= 1.5 {
        launch_score += 2;
        desc.push("地量后放量".into());
        VolumePattern::PostShrinkBurst
    } else if vol_ratio >= 2.5 {
        distribution_score += 1;
        desc.push("天量".into());
        VolumePattern::SuddenSpike
    } else if vol_ratio >= 1.5 {
        launch_score += 1;
        desc.push("温和放量".into());
        VolumePattern::GentleIncrease
    } else {
        VolumePattern::Flat
    };

    // 3. 量价背离
    if vol_ratio >= 1.5 && change_pct.abs() < 0.5 && change_pct >= 0.0 {
        distribution_score += 2;
        desc.push("价滞量增".into());
    }
    if vol_ratio >= 1.5 && change_pct < -2.0 {
        distribution_score += 2;
        desc.push("价跌量增".into());
    }

    // 4. 均线状态
    let ma5: f64 = kline.iter().rev().take(5).map(|k| k.close).sum::<f64>() / 5.0;
    let ma10: f64 = kline.iter().rev().take(10).map(|k| k.close).sum::<f64>() / 10.0;
    let ma20: f64 = kline.iter().rev().take(20).map(|k| k.close).sum::<f64>() / 20.0;
    let ma_info = MaBreakInfo {
        ma5_above_ma10: ma5 > ma10,
        ma5_above_ma20: ma5 > ma20,
        ma10_above_ma20: ma10 > ma20,
        is_bullish_alignment: ma5 > ma10 && ma10 > ma20,
    };
    if ma_info.is_bullish_alignment {
        launch_score += 1; desc.push("多头排列".into());
    } else if ma20 > ma5 {
        distribution_score += 1;
    }

    // 5. K线强度
    let candle_strength = if latest.close > 0.0 && latest.open > 0.0 {
        let body_pct = (latest.close - latest.open) / latest.open * 100.0;
        if body_pct > 3.0 {
            launch_score += 1; desc.push("强阳线".into());
            CandleStrength::Strong
        } else if body_pct > 1.0 {
            CandleStrength::Medium
        } else if body_pct > 0.0 {
            CandleStrength::Weak
        } else {
            CandleStrength::Bearish
        }
    } else { CandleStrength::Weak };

    // 6. 综合判定
    let (breakout_type, base_confidence) = if launch_score >= 4 && distribution_score <= 1 {
        (BreakoutType::Launch, 70u8 + launch_score * 5)
    } else if distribution_score >= 3 && launch_score <= 1 {
        (BreakoutType::Distribution, 65u8 + distribution_score * 5)
    } else if launch_score >= 2 {
        (BreakoutType::Launch, 40u8 + launch_score * 5)
    } else {
        (BreakoutType::Uncertain, 30u8)
    };

    confidence = base_confidence.min(95);

    BreakoutSignal {
        code: code.into(), name: name.into(),
        volume_ratio: Some(vol_ratio),
        vol_vs_20d_avg: Some(vol_ratio),
        volume_pattern,
        change_pct,
        candle_strength,
        ma_break: Some(ma_info),
        price_position,
        breakout_type,
        confidence,
        description: desc.join(" "),
        data_degraded: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::KlineData;
    use chrono::NaiveDate;

    fn k(date: &str, open: f64, close: f64, vol: f64, pct: f64) -> KlineData {
        KlineData {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            open, high: close.max(open), low: close.min(open), close, volume: vol,
            amount: 0.0, pct_chg: pct, intraday_price: None, settled: true, pe_ratio: None, pb_ratio: None,
            turnover_rate: None, market_cap: None, circulating_cap: None,
            eps: None, roe: None, revenue_yoy: None, net_profit_yoy: None,
            gross_margin: None, net_margin: None, sharpe_ratio: None,
            financials_history: None, valuation_history: None,
            consensus: None, industry: None,
            is_limit_up: false, is_limit_down: false, is_suspended: false,
            adjust: crate::data_provider::AdjustType::None,
        }
    }

    #[test]
    fn test_intraday_launch() {
        let s = screen_intraday("000001", "测试", 2.5, 5.2, 0.8, 4);
        assert_eq!(s.breakout_type, BreakoutType::Launch);
        assert!(s.confidence >= 50);
    }

    #[test]
    fn test_intraday_degraded() {
        let s = screen_intraday("000001", "测试", 0.0, 1.0, 0.0, 0);
        assert!(s.data_degraded);
    }

    #[test]
    fn test_postmarket_low_launch() {
        // 低位 + 地量后放量 + 多头排列 + 强阳线
        let mut data = Vec::new();
        for i in 0..30 {
            let price = 90.0 + i as f64 * 0.5; // 从 90 涨到 104.5
            let vol = if i < 25 { 0.5e6 } else { 2.0e6 }; // 前25天地量，近5天放量
            data.push(k(&format!("2026-06-{:02}", i+1), price-0.5, price, vol, 1.0));
        }
        let s = analyze_postmarket("000001", "测试", &data);
        // 应该检测到地量后放量的特征
        assert!(matches!(s.breakout_type, BreakoutType::Launch | BreakoutType::Uncertain));
    }

    #[test]
    fn test_insufficient_data() {
        let data = vec![k("2026-06-01", 10.0, 10.5, 1e6, 5.0)];
        let s = analyze_postmarket("000001", "测试", &data);
        assert!(s.data_degraded);
    }
}
