//! v12 MVP4-4.3: 龙虎榜 (R-04) 模板接通.
//!
//! 设计: 读 lhb_daily 表, 渲染成 §14.2 R-04 模板文本.
//!       数据缺失降级 (LhbEntryInput 字段 Option 化, 任一缺失标 "数据缺失").

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// 单条龙虎榜记录 (R-04 模板入参)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LhbEntryInput {
    pub code: String,
    pub name: String,
    pub net_buy_yi: f64,
    pub reason: String,
    pub buy_inst_n: Option<u32>,
    pub buy_inst_amt_wan: Option<f64>,
    pub buy_other_n: Option<u32>,
    pub buy_other_amt_wan: Option<f64>,
    pub buy_conc_pct: Option<f64>,
    pub sell_desc: Option<String>,
    pub sell_conc_pct: Option<f64>,
    pub chain_match: Option<String>,
    pub next_day_risk: Option<String>,
}

/// MVP4-4.3 主查询: 取最近 N 天龙虎榜
///
/// 注: 实际查询由 main 循环在 21:00 调; 本函数仅定义接口.
///     data_source 不稳定时返回空 Vec, 标 degraded.
pub fn fetch_recent_lhb(_date: NaiveDate, _limit: usize) -> Vec<LhbEntryInput> {
    Vec::new()
}

/// 渲染为字符串 (供 bin/monitor 拼接 R-04 模板).
///
/// 数据缺失字段填 "数据缺失" 占位.
pub fn render_to_string(inp: &LhbEntryInput, index: usize) -> String {
    let buy_inst = inp
        .buy_inst_n
        .map(|n| n.to_string())
        .unwrap_or_else(|| "数据缺失".to_string());
    let buy_inst_amt = inp
        .buy_inst_amt_wan
        .map(|v| format!("{:.0}", v))
        .unwrap_or_else(|| "数据缺失".to_string());
    let buy_other = inp
        .buy_other_n
        .map(|n| n.to_string())
        .unwrap_or_else(|| "数据缺失".to_string());
    let buy_other_amt = inp
        .buy_other_amt_wan
        .map(|v| format!("{:.0}", v))
        .unwrap_or_else(|| "数据缺失".to_string());
    let buy_conc = inp
        .buy_conc_pct
        .map(|v| format!("{:.0}", v))
        .unwrap_or_else(|| "数据缺失".to_string());
    let sell = inp.sell_desc.as_deref().unwrap_or("数据缺失");
    let sell_conc = inp
        .sell_conc_pct
        .map(|v| format!("{:.0}", v))
        .unwrap_or_else(|| "数据缺失".to_string());
    let chain = inp
        .chain_match
        .as_deref()
        .map(|s| format!("是-{}", s))
        .unwrap_or_else(|| "否".to_string());
    let risk = inp.next_day_risk.as_deref().unwrap_or("数据缺失");

    format!(
        "{}. {}({}) 净买{:.1}亿 | {}\n   买: 机构{}席{}万 其他{}席{}万（集中度{}%）\n   卖: {}（集中度{}%）\n   主线一致: {}\n   次日风险: {}",
        index + 1,
        inp.name,
        inp.code,
        inp.net_buy_yi,
        if inp.reason.is_empty() { "数据缺失" } else { &inp.reason },
        buy_inst, buy_inst_amt, buy_other, buy_other_amt, buy_conc,
        sell, sell_conc, chain, risk,
    )
}

/// 数据完整性评估
pub fn assess_data_quality(entries: &[LhbEntryInput]) -> (u8, bool) {
    let total = entries.len() * 7;
    if total == 0 {
        return (0, true);
    }
    let mut filled = 0;
    for e in entries {
        if e.buy_inst_n.is_some() {
            filled += 1;
        }
        if e.buy_inst_amt_wan.is_some() {
            filled += 1;
        }
        if e.buy_other_n.is_some() {
            filled += 1;
        }
        if e.buy_other_amt_wan.is_some() {
            filled += 1;
        }
        if e.buy_conc_pct.is_some() {
            filled += 1;
        }
        if e.sell_conc_pct.is_some() {
            filled += 1;
        }
        if e.next_day_risk.is_some() {
            filled += 1;
        }
    }
    let pct = ((filled as f64 / total as f64) * 100.0) as u8;
    let degraded = pct < 70;
    (pct, degraded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_fetch_returns_empty() {
        let r = fetch_recent_lhb(NaiveDate::from_ymd_opt(2026, 7, 5).unwrap(), 5);
        assert!(r.is_empty());
    }

    #[test]
    fn full_entry_renders_clean() {
        let inp = LhbEntryInput {
            code: "688001".to_string(),
            name: "X".to_string(),
            net_buy_yi: 1.5,
            reason: "涨幅偏离值7%".to_string(),
            buy_inst_n: Some(2),
            buy_inst_amt_wan: Some(8000.0),
            buy_other_n: Some(3),
            buy_other_amt_wan: Some(4000.0),
            buy_conc_pct: Some(65.0),
            sell_desc: Some("游资席位".to_string()),
            sell_conc_pct: Some(45.0),
            chain_match: Some("AI算力".to_string()),
            next_day_risk: Some("高开震荡".to_string()),
        };
        let s = render_to_string(&inp, 0);
        assert!(s.contains("X(688001)"));
        assert!(s.contains("净买1.5亿"));
        assert!(s.contains("机构2席8000万"));
        assert!(s.contains("主线一致: 是-AI算力"));
    }

    #[test]
    fn missing_fields_filled_with_placeholders() {
        let inp = LhbEntryInput {
            code: "000001".to_string(),
            name: "Y".to_string(),
            net_buy_yi: 0.5,
            reason: String::new(),
            ..Default::default()
        };
        let s = render_to_string(&inp, 0);
        assert!(s.contains("数据缺失"));
    }

    #[test]
    fn data_quality_full() {
        let entries: Vec<LhbEntryInput> = (0..5)
            .map(|i| LhbEntryInput {
                code: format!("{:06}", i),
                name: format!("N{}", i),
                net_buy_yi: 1.0,
                reason: "R".to_string(),
                buy_inst_n: Some(2),
                buy_inst_amt_wan: Some(8000.0),
                buy_other_n: Some(3),
                buy_other_amt_wan: Some(4000.0),
                buy_conc_pct: Some(65.0),
                sell_desc: Some("X".to_string()),
                sell_conc_pct: Some(45.0),
                chain_match: Some("AI".to_string()),
                next_day_risk: Some("高开".to_string()),
            })
            .collect();
        let (pct, degraded) = assess_data_quality(&entries);
        assert_eq!(pct, 100);
        assert!(!degraded);
    }

    #[test]
    fn data_quality_degraded() {
        let entries = vec![LhbEntryInput {
            code: "X".to_string(),
            name: "X".to_string(),
            net_buy_yi: 1.0,
            reason: "R".to_string(),
            buy_inst_n: Some(2),
            ..Default::default()
        }];
        let (pct, degraded) = assess_data_quality(&entries);
        assert!(degraded);
        assert!(pct < 70);
    }

    #[test]
    fn data_quality_empty() {
        let (pct, degraded) = assess_data_quality(&[]);
        assert_eq!(pct, 0);
        assert!(degraded);
    }
}
