//! 多周期入场点分析（Multi-Timeframe Entry Assessment）。
//!
//! 当日线产生买入信号（评分≥60 / BB+MACD BottomBuy/UptrendStart / RSI Buy）时，
//! 在 60min / 15min K 线上进一步确认精准入场点，目的：减少日线追高风险。
//!
//! ## 5 条入场规则（命中越多，confidence 越高）
//! 1. 60min MACD 金叉 + 收在 60min MA20 上方
//! 2. 15min RSI 从 <30 反弹到 30-50 区间（超卖回升，未到超买）
//! 3. 60min 量价配合：放量上涨 + 站上 60min MA10
//! 4. 15min 缩量回踩 60min MA20 不破（健康洗盘）
//! 5. 60min SKDJ 金叉
//!
//! 输入：60min / 15min K 线，按时间**升序**排列（最新在末尾）。

use crate::data_provider::intraday_kline::MinuteBar;
use crate::indicators::{
    calc_macd, calc_rsi, calc_skdj, MACD_FAST, MACD_SIGNAL, MACD_SLOW, SKDJ_M, SKDJ_N,
};

/// 多周期入场评估结果
#[derive(Debug, Clone, Default)]
pub struct EntryAssessment {
    /// 命中的入场信号数（0-5）
    pub hit_count: usize,
    /// 命中信号清单（中文描述）
    pub signals: Vec<String>,
    /// 风险/未达条件清单
    pub risks: Vec<String>,
    /// 建议入场参考价（命中规则 1 或 3 时取最近一根 60min 收盘价；
    /// 命中规则 4 时取 60min MA20 价；其余 None）
    pub suggested_entry: Option<f64>,
    /// 数据是否可用
    pub present: bool,
}

impl EntryAssessment {
    /// 是否给出"精准入场点"建议（命中 ≥2 条规则）
    pub fn ready(&self) -> bool {
        self.present && self.hit_count >= 2
    }

    /// 渲染为 Markdown 片段，注入到 AI prompt 的 extra_context。
    pub fn to_prompt_section(&self) -> String {
        if !self.present {
            return String::new();
        }
        let mut s = String::new();
        s.push_str("\n【多周期入场点（60min/15min）】\n");
        s.push_str(&format!("命中入场规则: {}/5\n", self.hit_count));
        if !self.signals.is_empty() {
            s.push_str("命中明细:\n");
            for sig in &self.signals {
                s.push_str(&format!("  · {}\n", sig));
            }
        }
        if !self.risks.is_empty() {
            s.push_str("待确认/风险:\n");
            for r in &self.risks {
                s.push_str(&format!("  · {}\n", r));
            }
        }
        if let Some(p) = self.suggested_entry {
            s.push_str(&format!("建议入场参考价: ¥{:.2}\n", p));
        }
        if self.ready() {
            s.push_str("结论: ✅ 精准入场点已确认（命中 ≥2 条），可按系统建议执行\n");
        } else {
            s.push_str("结论: ⚠️ 入场点尚未确认，建议等待更多 60min/15min 信号共振\n");
        }
        s
    }
}

/// 计算简单移动均线（输入升序，返回长度等于 closes，前 N-1 个为 NaN）
fn sma(values: &[f64], period: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; values.len()];
    if values.len() < period || period == 0 {
        return out;
    }
    let mut sum: f64 = values[..period].iter().sum();
    out[period - 1] = sum / period as f64;
    for i in period..values.len() {
        sum += values[i] - values[i - period];
        out[i] = sum / period as f64;
    }
    out
}

/// 评估精准入场点。
pub fn assess_entry(h1: &[MinuteBar], m15: &[MinuteBar]) -> EntryAssessment {
    // 至少需要 30 根 60min 才能算 MA20+MACD，15 根 15min 才能算 RSI
    if h1.len() < 30 || m15.len() < 20 {
        return EntryAssessment {
            present: false,
            ..Default::default()
        };
    }

    let h1_closes: Vec<f64> = h1.iter().map(|b| b.close).collect();
    let h1_highs: Vec<f64> = h1.iter().map(|b| b.high).collect();
    let h1_lows: Vec<f64> = h1.iter().map(|b| b.low).collect();
    let h1_vols: Vec<f64> = h1.iter().map(|b| b.volume).collect();

    let m15_closes: Vec<f64> = m15.iter().map(|b| b.close).collect();
    let m15_vols: Vec<f64> = m15.iter().map(|b| b.volume).collect();

    let h1_ma10 = sma(&h1_closes, 10);
    let h1_ma20 = sma(&h1_closes, 20);
    let h1_vol_ma5 = sma(&h1_vols, 5);
    let m15_vol_ma5 = sma(&m15_vols, 5);

    let h1_macd = calc_macd(&h1_closes, MACD_FAST, MACD_SLOW, MACD_SIGNAL);
    let h1_kdj = calc_skdj(&h1_highs, &h1_lows, &h1_closes, SKDJ_N, SKDJ_M);
    let m15_rsi = calc_rsi(&m15_closes);

    let last_h1 = h1.last().unwrap();
    let last_h1_ma10 = *h1_ma10.last().unwrap_or(&f64::NAN);
    let last_h1_ma20 = *h1_ma20.last().unwrap_or(&f64::NAN);
    let last_h1_vol = *h1_vols.last().unwrap_or(&0.0);
    let last_h1_vol_ma5 = *h1_vol_ma5.last().unwrap_or(&f64::NAN);

    let mut signals: Vec<String> = Vec::new();
    let mut risks: Vec<String> = Vec::new();
    let mut suggested_entry: Option<f64> = None;

    // ── 规则 1：60min MACD 金叉 + 收在 MA20 上方 ──────────────────────
    {
        // MACD 金叉：DIF 在最近 3 根内上穿 DEA
        let mut macd_golden = false;
        if h1_macd.len() >= 4 {
            for i in (h1_macd.len() - 3)..h1_macd.len() {
                let prev = &h1_macd[i - 1];
                let curr = &h1_macd[i];
                if prev.dif <= prev.dea && curr.dif > curr.dea {
                    macd_golden = true;
                    break;
                }
            }
        }
        let above_ma20 = last_h1_ma20.is_finite() && last_h1.close > last_h1_ma20;
        if macd_golden && above_ma20 {
            signals.push(format!(
                "✅ 60min MACD 金叉 + 收 ¥{:.2} 在 MA20 ¥{:.2} 上方",
                last_h1.close, last_h1_ma20
            ));
            suggested_entry = Some(last_h1.close);
        } else if !macd_golden {
            risks.push("60min MACD 未金叉".to_string());
        } else {
            risks.push(format!(
                "60min 收 ¥{:.2} 仍在 MA20 ¥{:.2} 下方",
                last_h1.close, last_h1_ma20
            ));
        }
    }

    // ── 规则 2：15min RSI 从 <30 反弹到 30-50 ────────────────────────
    {
        let recent_n = m15_rsi.len().min(8);
        let recent: Vec<f64> = m15_rsi
            .iter()
            .rev()
            .take(recent_n)
            .map(|p| p.rsi6)
            .collect();
        let last_rsi = recent.first().copied().unwrap_or(f64::NAN);
        let had_oversold = recent.iter().any(|v| *v < 30.0);
        let in_rebound_zone = last_rsi >= 30.0 && last_rsi <= 50.0;
        if had_oversold && in_rebound_zone {
            signals.push(format!(
                "✅ 15min RSI6 近期触及超卖（<30）后回升至 {:.1}",
                last_rsi
            ));
        } else if last_rsi > 70.0 {
            risks.push(format!(
                "15min RSI6={:.1} 已进入超买区，警惕短线追高",
                last_rsi
            ));
        } else if !had_oversold {
            risks.push("15min RSI6 近期未出现超卖反弹".to_string());
        } else {
            risks.push(format!(
                "15min RSI6={:.1} 不在 30-50 反弹理想区间",
                last_rsi
            ));
        }
    }

    // ── 规则 3：60min 放量上涨 + 站上 60min MA10 ────────────────────
    {
        let prev_close = h1_closes
            .get(h1_closes.len() - 2)
            .copied()
            .unwrap_or(f64::NAN);
        let up = last_h1.close > prev_close;
        let vol_expand = last_h1_vol_ma5.is_finite()
            && last_h1_vol_ma5 > 0.0
            && last_h1_vol > last_h1_vol_ma5 * 1.3;
        let above_ma10 = last_h1_ma10.is_finite() && last_h1.close > last_h1_ma10;
        if up && vol_expand && above_ma10 {
            let ratio = last_h1_vol / last_h1_vol_ma5;
            signals.push(format!(
                "✅ 60min 放量上涨（量比{:.2}）+ 站上 MA10 ¥{:.2}",
                ratio, last_h1_ma10
            ));
            if suggested_entry.is_none() {
                suggested_entry = Some(last_h1.close);
            }
        } else {
            let mut why: Vec<&str> = Vec::new();
            if !up {
                why.push("未上涨");
            }
            if !vol_expand {
                why.push("未放量");
            }
            if !above_ma10 {
                why.push("未站上 MA10");
            }
            risks.push(format!("60min 量价配合不足：{}", why.join(" / ")));
        }
    }

    // ── 规则 4：15min 缩量回踩 60min MA20 不破 ──────────────────────
    {
        let last_m15 = m15.last().unwrap();
        let last_m15_vol = *m15_vols.last().unwrap_or(&0.0);
        let last_m15_vol_ma5 = *m15_vol_ma5.last().unwrap_or(&f64::NAN);
        let pulled_back = last_h1_ma20.is_finite()
            && last_m15.low <= last_h1_ma20 * 1.005   // 触及 MA20（容差 0.5%）
            && last_m15.close >= last_h1_ma20; // 收回 MA20 上方
        let vol_shrink = last_m15_vol_ma5.is_finite()
            && last_m15_vol_ma5 > 0.0
            && last_m15_vol < last_m15_vol_ma5 * 0.8;
        if pulled_back && vol_shrink {
            signals.push(format!(
                "✅ 15min 缩量回踩 60min MA20 ¥{:.2} 不破（健康洗盘）",
                last_h1_ma20
            ));
            if suggested_entry.is_none() {
                suggested_entry = Some(last_h1_ma20);
            }
        } else if last_h1_ma20.is_finite() && last_m15.close < last_h1_ma20 {
            risks.push(format!(
                "15min 收 ¥{:.2} 跌破 60min MA20 ¥{:.2}，需警惕",
                last_m15.close, last_h1_ma20
            ));
        }
    }

    // ── 规则 5：60min SKDJ 金叉 ──────────────────────────────────────
    {
        let n = h1_kdj.len();
        let golden = if n >= 2 {
            let prev = &h1_kdj[n - 2];
            let curr = &h1_kdj[n - 1];
            // K 上穿 D，且金叉点 K < 80（避免高位钝化金叉）
            prev.k <= prev.d && curr.k > curr.d && curr.k < 80.0
        } else {
            false
        };
        if golden {
            let last = h1_kdj.last().unwrap();
            signals.push(format!(
                "✅ 60min SKDJ 金叉（K={:.1} D={:.1} J={:.1}）",
                last.k, last.d, last.j
            ));
        } else {
            risks.push("60min SKDJ 未金叉".to_string());
        }
    }

    let hit_count = signals.len();
    EntryAssessment {
        hit_count,
        signals,
        risks,
        suggested_entry,
        present: true,
    }
}
