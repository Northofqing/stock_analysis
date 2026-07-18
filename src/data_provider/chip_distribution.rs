//! 筹码分布分析（CYQ 成本分布模型）
//!
//! 基于历史 K 线 + 换手率，采用经典"衰减-叠加"模型重建筹码成本分布：
//!   1. 价格区间切成 N 个桶（默认 100）；
//!   2. 按时间升序遍历每根 K 线，
//!        - 存量筹码 × (1 - 换手率)   — 衰减（旧筹码随换手被抛售）
//!        - 今日新增筹码按 [low, high] 均匀分布加入 — 今日成交带来的新筹码
//!   3. 最终归一化得到概率密度。
//!
//! 产出关键指标供 AI prompt 使用：
//!   - 平均成本 / 主力成本（峰值成本）
//!   - 获利盘比例 (profit_ratio)
//!   - 90% / 70% 成本区间与集中度
//!   - 当前价相对筹码峰的位置（上方=突破 / 下方=套牢）
//!
//! 数据源：仅依赖 `KlineData`（已有的日线 + 换手率）。
//! 换手率缺失时以 3% 兜底（A 股中位数），并在输出中标记"估算"。

use super::KlineData;

/// 筹码分布核心指标（面向 AI Prompt）
#[derive(Debug, Clone)]
pub struct ChipDistribution {
    /// 是否成功计算
    pub present: bool,
    /// 参与计算的 K 线天数
    pub days_used: usize,
    /// 是否存在换手率估算（数据缺失时标记）
    pub turnover_estimated: bool,
    /// 当前价（最新收盘）
    pub current_price: f64,
    /// 加权平均成本
    pub avg_cost: f64,
    /// 主力成本（筹码峰价格，密度最大桶中价）
    pub main_cost: f64,
    /// 获利盘比例 (0-1)：成本低于当前价的筹码占比
    pub profit_ratio: f64,
    /// 90% 成本区间下沿
    pub p90_low: f64,
    /// 90% 成本区间上沿
    pub p90_high: f64,
    /// 70% 成本区间下沿
    pub p70_low: f64,
    /// 70% 成本区间上沿
    pub p70_high: f64,
    /// 筹码集中度（90% 区间宽度 / 平均成本，越小越集中），%
    pub concentration_90: f64,
    /// 筹码集中度（70% 区间宽度 / 平均成本），%
    pub concentration_70: f64,
    /// 当前价相对主力成本偏离 (%)
    pub price_vs_main_pct: f64,
}

impl Default for ChipDistribution {
    fn default() -> Self {
        Self {
            present: false,
            days_used: 0,
            turnover_estimated: false,
            current_price: 0.0,
            avg_cost: 0.0,
            main_cost: 0.0,
            profit_ratio: 0.0,
            p90_low: 0.0,
            p90_high: 0.0,
            p70_low: 0.0,
            p70_high: 0.0,
            concentration_90: 0.0,
            concentration_70: 0.0,
            price_vs_main_pct: 0.0,
        }
    }
}

/// 使用的桶数（价格分辨率）
const BUCKETS: usize = 120;
/// 最多回溯天数（超过此值往往对当前筹码影响已可忽略）
const MAX_DAYS: usize = 120;
/// 换手率缺失时的兜底值 (%)
const TURNOVER_FALLBACK: f64 = 3.0;

/// 计算筹码分布
///
/// `kline_data`：按照工程约定，**最新在前**（`[0]` 为最新交易日）。
/// 内部会翻转为时间升序后再迭代。
pub fn compute_chip_distribution(kline_data: &[KlineData]) -> ChipDistribution {
    if kline_data.is_empty() {
        return ChipDistribution::default();
    }

    // 取最近 MAX_DAYS 天，时间升序（旧→新）
    let slice_len = kline_data.len().min(MAX_DAYS);
    // kline_data[0] 最新 → 反向迭代得到时间升序
    let chron: Vec<&KlineData> = kline_data[..slice_len].iter().rev().collect();

    if chron.len() < 5 {
        return ChipDistribution::default();
    }

    // 价格区间
    let min_p = chron.iter().map(|k| k.low).fold(f64::INFINITY, f64::min);
    let max_p = chron
        .iter()
        .map(|k| k.high)
        .fold(f64::NEG_INFINITY, f64::max);
    if !(min_p.is_finite() && max_p.is_finite()) || max_p <= min_p {
        return ChipDistribution::default();
    }

    // 适度外扩 0.5%，避免边界价格落到桶外
    let pad = (max_p - min_p) * 0.005;
    let lo = (min_p - pad).max(0.01);
    let hi = max_p + pad;
    let step = (hi - lo) / BUCKETS as f64;
    if step <= 0.0 {
        return ChipDistribution::default();
    }

    let mut chips = vec![0f64; BUCKETS];
    let mut estimated = false;

    for k in &chron {
        let turnover_pct = match k.turnover_rate {
            Some(t) if t > 0.0 && t < 100.0 => t,
            _ => {
                estimated = true;
                TURNOVER_FALLBACK
            }
        };
        let turnover = (turnover_pct / 100.0).clamp(0.001, 0.95);

        // 1) 衰减
        for c in chips.iter_mut() {
            *c *= 1.0 - turnover;
        }

        // 2) 今日新增筹码：在 [low, high] 均匀分布
        let low_idx =
            (((k.low - lo) / step).floor() as isize).clamp(0, BUCKETS as isize - 1) as usize;
        let high_idx =
            (((k.high - lo) / step).floor() as isize).clamp(0, BUCKETS as isize - 1) as usize;
        let band = high_idx.saturating_sub(low_idx) + 1;
        let per_bucket = turnover / band as f64;
        for chip in &mut chips[low_idx..=high_idx] {
            *chip += per_bucket;
        }
    }

    // 归一化
    let total: f64 = chips.iter().sum();
    if total <= 0.0 {
        return ChipDistribution::default();
    }
    for c in chips.iter_mut() {
        *c /= total;
    }

    let price_mid = |i: usize| lo + (i as f64 + 0.5) * step;

    // 平均成本
    let avg_cost: f64 = chips
        .iter()
        .enumerate()
        .map(|(i, w)| w * price_mid(i))
        .sum();

    // 主力成本（峰值桶）
    let (peak_idx, _) =
        chips.iter().enumerate().fold(
            (0usize, 0f64),
            |(pi, pv), (i, &v)| {
                if v > pv {
                    (i, v)
                } else {
                    (pi, pv)
                }
            },
        );
    let main_cost = price_mid(peak_idx);

    // 当前价（最新收盘）
    let current_price = kline_data[0].close;

    // 获利盘：成本 < 当前价的桶累加
    let profit_ratio: f64 = chips
        .iter()
        .enumerate()
        .filter(|(i, _)| price_mid(*i) <= current_price)
        .map(|(_, w)| *w)
        .sum();

    // 成本区间（对称去掉两端 (1-p)/2 分位）
    let percentile_band = |p: f64| -> (f64, f64) {
        let tail = (1.0 - p) / 2.0;
        let mut cum = 0.0;
        let mut low_px = price_mid(0);
        let mut high_px = price_mid(BUCKETS - 1);
        let mut low_done = false;
        for (i, chip) in chips.iter().enumerate() {
            cum += chip;
            if !low_done && cum >= tail {
                low_px = price_mid(i);
                low_done = true;
            }
            if cum >= 1.0 - tail {
                high_px = price_mid(i);
                break;
            }
        }
        (low_px, high_px)
    };

    let (p90_low, p90_high) = percentile_band(0.90);
    let (p70_low, p70_high) = percentile_band(0.70);

    let concentration_90 = if avg_cost > 0.0 {
        (p90_high - p90_low) / avg_cost * 100.0
    } else {
        0.0
    };
    let concentration_70 = if avg_cost > 0.0 {
        (p70_high - p70_low) / avg_cost * 100.0
    } else {
        0.0
    };

    let price_vs_main_pct = if main_cost > 0.0 {
        (current_price / main_cost - 1.0) * 100.0
    } else {
        0.0
    };

    ChipDistribution {
        present: true,
        days_used: chron.len(),
        turnover_estimated: estimated,
        current_price,
        avg_cost,
        main_cost,
        profit_ratio,
        p90_low,
        p90_high,
        p70_low,
        p70_high,
        concentration_90,
        concentration_70,
        price_vs_main_pct,
    }
}

/// 将筹码分布格式化为 AI Prompt 片段
pub fn format_for_prompt(chip: &ChipDistribution) -> String {
    if !chip.present {
        return String::new();
    }

    // 获利盘定性
    let profit_label = if chip.profit_ratio >= 0.85 {
        "🔥 高位获利盘密集，警惕回吐压力"
    } else if chip.profit_ratio >= 0.60 {
        "多数持有者获利，趋势健康"
    } else if chip.profit_ratio >= 0.30 {
        "套牢盘仍占多数，上方压力较大"
    } else if chip.profit_ratio >= 0.10 {
        "⚠️ 深度套牢，反弹易遇抛压"
    } else {
        "⚠️ 几乎全线套牢，弱势特征明显"
    };

    // 集中度定性（90% 区间宽度 / 平均成本）
    let conc_label = if chip.concentration_90 < 15.0 {
        "筹码高度集中（主力锁仓/长期磨底）"
    } else if chip.concentration_90 < 25.0 {
        "筹码较为集中"
    } else if chip.concentration_90 < 40.0 {
        "筹码分散度中等"
    } else {
        "筹码高度分散（多空分歧大）"
    };

    // 价格相对主力成本
    let pos_label = if chip.price_vs_main_pct > 5.0 {
        "当前价显著高于主力成本，主力浮盈丰厚"
    } else if chip.price_vs_main_pct > -2.0 {
        "当前价贴近主力成本，关键支撑/压力位"
    } else if chip.price_vs_main_pct > -8.0 {
        "当前价低于主力成本，主力浅套"
    } else {
        "⚠️ 当前价远低于主力成本，主力深套"
    };

    let est_tag = if chip.turnover_estimated {
        "（换手率部分估算）"
    } else {
        ""
    };

    let mut s = String::new();
    s.push_str(&format!(
        "\n【筹码分布（近{}日，CYQ 衰减模型）{}】\n",
        chip.days_used, est_tag
    ));
    // 以表格形式输出，方便 Markdown 渲染时统一为表格
    s.push_str("指标 | 数值 | 解读\n");
    s.push_str(&format!("平均成本 | ¥{:.2} | -\n", chip.avg_cost));
    s.push_str(&format!("主力成本(峰值) | ¥{:.2} | -\n", chip.main_cost));
    s.push_str(&format!("当前价 | ¥{:.2} | -\n", chip.current_price));
    s.push_str(&format!(
        "获利盘比例 | {:.1}% | {}\n",
        chip.profit_ratio * 100.0,
        profit_label
    ));
    s.push_str(&format!(
        "90%成本区间 | ¥{:.2} ~ ¥{:.2} | 宽度 {:.1}%\n",
        chip.p90_low, chip.p90_high, chip.concentration_90
    ));
    s.push_str(&format!(
        "70%成本区间 | ¥{:.2} ~ ¥{:.2} | 宽度 {:.1}%，{}\n",
        chip.p70_low, chip.p70_high, chip.concentration_70, conc_label
    ));
    s.push_str(&format!(
        "现价/主力成本偏离 | {:+.2}% | {}\n",
        chip.price_vs_main_pct, pos_label
    ));

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_provider::AdjustType;
    use chrono::{Duration, NaiveDate};

    fn kline(index: i64, low: f64, high: f64, close: f64, turnover: Option<f64>) -> KlineData {
        KlineData {
            date: NaiveDate::from_ymd_opt(2026, 7, 18).expect("fixture date")
                - Duration::days(index),
            open: (low + high) / 2.0,
            high,
            low,
            close,
            volume: 1_000.0,
            amount: close * 1_000.0,
            pct_chg: 0.0,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: turnover,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            adjust: AdjustType::None,
        }
    }

    #[test]
    fn compute_rejects_empty_short_flat_and_nonfinite_histories() {
        assert!(!compute_chip_distribution(&[]).present);
        let short = vec![kline(0, 9.0, 11.0, 10.0, Some(3.0)); 4];
        assert!(!compute_chip_distribution(&short).present);
        let flat = vec![kline(0, 10.0, 10.0, 10.0, Some(3.0)); 5];
        assert!(!compute_chip_distribution(&flat).present);
        let nonfinite = vec![kline(0, f64::NAN, 11.0, 10.0, Some(3.0)); 5];
        assert!(!compute_chip_distribution(&nonfinite).present);
    }

    #[test]
    fn compute_uses_latest_120_days_and_marks_turnover_estimation() {
        let history: Vec<KlineData> = (0..130)
            .map(|index| {
                let center = 10.0 + index as f64 * 0.01;
                let turnover = match index % 3 {
                    0 => None,
                    1 => Some(0.0),
                    _ => Some(2.5),
                };
                kline(index, center - 0.4, center + 0.4, center, turnover)
            })
            .collect();

        let chip = compute_chip_distribution(&history);

        assert!(chip.present);
        assert_eq!(chip.days_used, 120);
        assert!(chip.turnover_estimated);
        assert_eq!(chip.current_price, 10.0);
        assert!(chip.avg_cost > 0.0);
        assert!(chip.main_cost > 0.0);
        assert!((0.0..=1.0).contains(&chip.profit_ratio));
        assert!(chip.p90_low <= chip.p70_low);
        assert!(chip.p70_high <= chip.p90_high);
        assert!(chip.concentration_90 >= chip.concentration_70);
        assert!(chip.price_vs_main_pct.is_finite());
    }

    fn rendered(profit: f64, concentration: f64, price_vs_main: f64, estimated: bool) -> String {
        format_for_prompt(&ChipDistribution {
            present: true,
            days_used: 20,
            turnover_estimated: estimated,
            current_price: 10.0,
            avg_cost: 9.5,
            main_cost: 9.0,
            profit_ratio: profit,
            p90_low: 8.0,
            p90_high: 11.0,
            p70_low: 8.5,
            p70_high: 10.5,
            concentration_90: concentration,
            concentration_70: concentration / 2.0,
            price_vs_main_pct: price_vs_main,
        })
    }

    #[test]
    fn prompt_covers_profit_concentration_and_cost_position_bands() {
        assert!(format_for_prompt(&ChipDistribution::default()).is_empty());

        let profit_cases = [
            (0.85, "高位获利盘密集"),
            (0.60, "趋势健康"),
            (0.30, "套牢盘仍占多数"),
            (0.10, "深度套牢"),
            (0.09, "几乎全线套牢"),
        ];
        for (value, expected) in profit_cases {
            assert!(rendered(value, 14.0, 6.0, false).contains(expected));
        }

        let concentration_cases = [
            (14.9, "筹码高度集中"),
            (24.9, "筹码较为集中"),
            (39.9, "筹码分散度中等"),
            (40.0, "筹码高度分散"),
        ];
        for (value, expected) in concentration_cases {
            assert!(rendered(0.5, value, 0.0, false).contains(expected));
        }

        let position_cases = [
            (5.1, "显著高于主力成本"),
            (-1.9, "贴近主力成本"),
            (-7.9, "主力浅套"),
            (-8.0, "主力深套"),
        ];
        for (value, expected) in position_cases {
            assert!(rendered(0.5, 20.0, value, false).contains(expected));
        }

        let estimated = rendered(0.5, 20.0, 0.0, true);
        assert!(estimated.contains("换手率部分估算"));
        assert!(estimated.contains("90%成本区间"));
        assert!(estimated.contains("70%成本区间"));
    }
}
